use std::{
    fs::File,
    io,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MountId(pub u32);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileRef {
    entry: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MountKind {
    Directory,
    Pak,
    Pk3,
}

pub struct MountInfo {
    pub id: MountId,
    pub path: PathBuf,
    pub kind: MountKind,
    /// Larger priorities win; equally ranked mounts prefer the last mounted.
    pub priority: i32,
    active: bool,
}

pub struct Origin<'a> {
    pub mount: MountId,
    pub path: &'a Path,
    pub member: &'a [u8],
    pub kind: MountKind,
}

/// A boundary reader supplies admitted member ranges, in original order.
pub struct FileRange {
    pub name: Vec<u8>,
    pub offset: u64,
    pub length: u64,
}

struct Entry {
    mount: usize,
    name: Box<[u8]>,
    original_name: Box<[u8]>,
    file: Arc<File>,
    offset: u64,
    length: u64,
}

#[derive(Debug)]
pub enum VfsError {
    Path,
    Capacity,
    Range,
    Handle,
    Io(io::Error),
}
impl From<io::Error> for VfsError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

#[derive(Default)]
pub struct Vfs {
    mounts: Vec<MountInfo>,
    entries: Vec<Entry>,
    index: Vec<usize>,
    lookups: AtomicU64,
}

pub fn normalize(path: &[u8]) -> Result<Vec<u8>, VfsError> {
    if path.is_empty() || path.len() > 4096 || path.iter().any(|&c| c == 0 || c == b':') {
        return Err(VfsError::Path);
    }
    for component in path.split(|&c| c == b'/' || c == b'\\') {
        if component.is_empty() || component == b"." || component == b".." {
            return Err(VfsError::Path);
        }
    }
    Ok(path
        .iter()
        .map(|&c| {
            if c == b'\\' {
                b'/'
            } else {
                c.to_ascii_lowercase()
            }
        })
        .collect())
}

impl Vfs {
    pub fn mount_directory(&mut self, path: &Path, priority: i32) -> Result<MountId, VfsError> {
        let root = path.canonicalize()?;
        let mut pending = vec![root.clone()];
        let mut entries = Vec::new();
        while let Some(directory) = pending.pop() {
            let mut children = std::fs::read_dir(directory)?.collect::<Result<Vec<_>, _>>()?;
            children.sort_unstable_by_key(|child| child.file_name());
            for child in children {
                let kind = child.file_type()?;
                if kind.is_dir() {
                    pending.push(child.path());
                    continue;
                }
                if !kind.is_file() {
                    continue;
                }
                let full = child.path();
                let relative = full.strip_prefix(&root).map_err(|_| VfsError::Path)?;
                let name = relative
                    .to_str()
                    .ok_or(VfsError::Path)?
                    .replace(std::path::MAIN_SEPARATOR, "/")
                    .into_bytes();
                let normalized = normalize(&name)?;
                let file = Arc::new(File::open(full)?);
                let length = file.metadata()?.len();
                entries.push(Entry {
                    mount: self.mounts.len(),
                    name: normalized.into_boxed_slice(),
                    original_name: name.into_boxed_slice(),
                    file,
                    offset: 0,
                    length,
                });
            }
        }
        self.publish(root, MountKind::Directory, priority, entries)
    }

    pub fn mount_ranges(
        &mut self,
        path: &Path,
        kind: MountKind,
        priority: i32,
        file: Arc<File>,
        ranges: impl IntoIterator<Item = FileRange>,
    ) -> Result<MountId, VfsError> {
        let size = file.metadata()?.len();
        let mut entries = Vec::new();
        for range in ranges {
            if range
                .offset
                .checked_add(range.length)
                .is_none_or(|end| end > size)
            {
                return Err(VfsError::Range);
            }
            entries.push(Entry {
                mount: self.mounts.len(),
                name: normalize(&range.name)?.into_boxed_slice(),
                original_name: range.name.into_boxed_slice(),
                file: Arc::clone(&file),
                offset: range.offset,
                length: range.length,
            });
        }
        self.publish(path.to_path_buf(), kind, priority, entries)
    }

    fn publish(
        &mut self,
        path: PathBuf,
        kind: MountKind,
        priority: i32,
        entries: Vec<Entry>,
    ) -> Result<MountId, VfsError> {
        if self.mounts.len() >= 65536
            || self
                .entries
                .len()
                .checked_add(entries.len())
                .is_none_or(|len| len > u32::MAX as usize)
        {
            return Err(VfsError::Capacity);
        }
        let id = MountId(self.mounts.len() as u32);
        self.mounts.push(MountInfo {
            id,
            path,
            kind,
            priority,
            active: true,
        });
        self.entries.extend(entries);
        self.reindex();
        Ok(id)
    }

    fn reindex(&mut self) {
        self.index.clear();
        self.index.extend(
            self.entries
                .iter()
                .enumerate()
                .filter(|(_, entry)| self.mounts[entry.mount].active)
                .map(|(index, _)| index),
        );
        self.index.sort_unstable_by(|&a, &b| {
            let x = &self.entries[a];
            let y = &self.entries[b];
            x.name
                .cmp(&y.name)
                .then_with(|| {
                    self.mounts[y.mount]
                        .priority
                        .cmp(&self.mounts[x.mount].priority)
                })
                .then(y.mount.cmp(&x.mount))
                .then(a.cmp(&b))
        });
    }

    /// Resolve names only while loading. Simulation/render/audio retain FileRefs.
    pub fn open(&self, path: &[u8]) -> Option<FileRef> {
        self.lookups.fetch_add(1, Ordering::Relaxed);
        let name = normalize(path).ok()?;
        let position = self
            .index
            .partition_point(|&index| self.entries[index].name.as_ref() < name.as_slice());
        let &entry = self.index.get(position)?;
        (self.entries[entry].name.as_ref() == name).then_some(FileRef {
            entry: entry as u32,
        })
    }

    fn entry(&self, reference: FileRef) -> Result<&Entry, VfsError> {
        self.entries
            .get(reference.entry as usize)
            .filter(|entry| self.mounts[entry.mount].active)
            .ok_or(VfsError::Handle)
    }

    pub fn length(&self, reference: FileRef) -> Result<u64, VfsError> {
        Ok(self.entry(reference)?.length)
    }

    pub fn read_at(
        &self,
        reference: FileRef,
        offset: u64,
        destination: &mut [u8],
    ) -> Result<usize, VfsError> {
        let entry = self.entry(reference)?;
        let remaining = entry.length.checked_sub(offset).ok_or(VfsError::Range)?;
        let count = destination
            .len()
            .min(usize::try_from(remaining).unwrap_or(usize::MAX));
        let mut written = 0;
        while written < count {
            let read = native_read(
                &entry.file,
                &mut destination[written..count],
                entry.offset + offset + written as u64,
            )?;
            if read == 0 {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            written += read;
        }
        Ok(written)
    }

    pub fn origin(&self, reference: FileRef) -> Option<Origin<'_>> {
        let entry = self.entry(reference).ok()?;
        let mount = &self.mounts[entry.mount];
        Some(Origin {
            mount: mount.id,
            path: &mount.path,
            member: &entry.original_name,
            kind: mount.kind,
        })
    }

    pub fn mounts(&self) -> impl Iterator<Item = &MountInfo> {
        self.mounts.iter().filter(|mount| mount.active)
    }
    pub fn files(&self) -> impl Iterator<Item = (FileRef, &[u8])> {
        self.index.iter().map(|&entry| {
            (
                FileRef {
                    entry: entry as u32,
                },
                self.entries[entry].name.as_ref(),
            )
        })
    }
    pub fn take_lookup_count(&self) -> u64 {
        self.lookups.swap(0, Ordering::Relaxed)
    }

    pub fn unmount(&mut self, id: MountId) -> bool {
        let Some(mount) = self
            .mounts
            .get_mut(id.0 as usize)
            .filter(|mount| mount.active)
        else {
            return false;
        };
        mount.active = false;
        self.reindex();
        true
    }
}

#[cfg(unix)]
fn native_read(file: &File, destination: &mut [u8], offset: u64) -> io::Result<usize> {
    use std::os::unix::fs::FileExt;
    file.read_at(destination, offset)
}
#[cfg(windows)]
fn native_read(file: &File, destination: &mut [u8], offset: u64) -> io::Result<usize> {
    use std::os::windows::fs::FileExt;
    file.seek_read(destination, offset)
}

impl From<io::ErrorKind> for VfsError {
    fn from(kind: io::ErrorKind) -> Self {
        Self::Io(kind.into())
    }
}
