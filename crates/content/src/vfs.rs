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

struct Entry {
    mount: usize,
    name: Box<[u8]>,
    original_name: Box<[u8]>,
    source: EntrySource,
    length: u64,
}

enum EntrySource {
    Loose(Arc<File>),
    Archive { archive: Arc<Archive>, entry: usize },
}

#[derive(Debug)]
pub enum VfsError {
    Path,
    Capacity,
    Range,
    Handle,
    Io(io::Error),
    Format(FormatError),
}
impl From<FormatError> for VfsError {
    fn from(error: FormatError) -> Self {
        Self::Format(error)
    }
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
    let mut bytes = [0; 4096];
    let length = normalize_into(path, &mut bytes)?;
    Ok(bytes[..length].to_vec())
}

fn normalize_into(path: &[u8], normalized: &mut [u8; 4096]) -> Result<usize, VfsError> {
    if path.is_empty() || path.len() > 4096 || path.iter().any(|&c| c == 0 || c == b':') {
        return Err(VfsError::Path);
    }
    let mut starts = [0usize; 2048];
    let mut count = 0;
    let mut length = 0;
    for component in path.split(|&c| c == b'/' || c == b'\\') {
        if component.is_empty() || component == b"." {
            return Err(VfsError::Path);
        }
        if component == b".." {
            if count == 0 {
                return Err(VfsError::Path);
            }
            count -= 1;
            length = starts[count];
        } else {
            starts[count] = length;
            count += 1;
            if length != 0 {
                normalized[length] = b'/';
                length += 1;
            }
            for byte in component {
                normalized[length] = qa_core::names::path_byte(*byte);
                length += 1;
            }
        }
    }
    if count == 0 {
        return Err(VfsError::Path);
    }
    Ok(length)
}

impl Vfs {
    /// Higher-ranked products/mods choose a larger base priority. Inside each
    /// product, loose files are below numerically ordered PAKs and alphabetic PK3s.
    pub fn mount_product(&mut self, path: &Path, priority: i32) -> Result<Vec<MountId>, VfsError> {
        let first_mount = self.mounts.len();
        let result = (|| {
            let root = self.mount_directory(path, priority)?;
            let mut packages: Vec<_> = self
                .entries
                .iter()
                .enumerate()
                .filter(|(_, entry)| {
                    entry.mount == root.0 as usize
                        && !entry.name.contains(&b'/')
                        && (entry.name.ends_with(b".pak") || entry.name.ends_with(b".pk3"))
                })
                .map(|(entry, value)| {
                    (
                        FileRef {
                            entry: entry as u32,
                        },
                        value.name.to_vec(),
                        value.original_name.to_vec(),
                    )
                })
                .collect();
            packages.sort_unstable_by(|a, b| {
                package_order(&a.1)
                    .cmp(&package_order(&b.1))
                    .then(a.1.cmp(&b.1))
            });
            let mut mounts = vec![root];
            for (rank, (reference, _, name)) in packages.into_iter().enumerate() {
                let file = self.source_file(reference).ok_or(VfsError::Handle)?;
                let archive = Arc::new(Archive::parse(file)?);
                let name = std::str::from_utf8(&name).map_err(|_| VfsError::Path)?;
                let native_path = self.mounts[root.0 as usize].path.join(name);
                let rank = i32::try_from(rank + 1).map_err(|_| VfsError::Capacity)?;
                let priority = priority.checked_add(rank).ok_or(VfsError::Capacity)?;
                mounts.push(self.mount_archive(&native_path, priority, archive)?);
            }
            Ok(mounts)
        })();
        if result.is_err() {
            for mount in &mut self.mounts[first_mount..] {
                mount.active = false;
            }
            self.reindex();
        }
        result
    }
    pub fn mount_directory(&mut self, path: &Path, priority: i32) -> Result<MountId, VfsError> {
        self.mount_directory_filtered(path, priority, false)
    }
    /// The saved profile shares this VFS without indexing binary assets/saves
    /// or allowing profile files to override higher-priority product content.
    pub fn mount_settings_directory(
        &mut self,
        path: &Path,
        priority: i32,
    ) -> Result<MountId, VfsError> {
        self.mount_directory_filtered(path, priority, true)
    }
    fn mount_directory_filtered(
        &mut self,
        path: &Path,
        priority: i32,
        settings_only: bool,
    ) -> Result<MountId, VfsError> {
        let root = path.canonicalize()?;
        let mut pending = vec![root.clone()];
        let mut entries = Vec::new();
        while let Some(directory) = pending.pop() {
            let mut children = std::fs::read_dir(directory)?.collect::<Result<Vec<_>, _>>()?;
            children.sort_unstable_by_key(|child| child.file_name());
            for child in children {
                let kind = child.file_type()?;
                if kind.is_dir() {
                    if !settings_only
                        || !(child.file_name().eq_ignore_ascii_case("assets")
                            || child.file_name().eq_ignore_ascii_case("saves"))
                    {
                        pending.push(child.path());
                    }
                    continue;
                }
                if !kind.is_file() {
                    continue;
                }
                let full = child.path();
                if settings_only
                    && !full.extension().is_some_and(|extension| {
                        extension.eq_ignore_ascii_case("cfg")
                            || extension.eq_ignore_ascii_case("json")
                    })
                {
                    continue;
                }
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
                    source: EntrySource::Loose(file),
                    length,
                });
            }
        }
        self.publish(root, MountKind::Directory, priority, entries)
    }

    pub fn mount_archive(
        &mut self,
        path: &Path,
        priority: i32,
        archive: Arc<Archive>,
    ) -> Result<MountId, VfsError> {
        let kind = match archive.kind {
            ArchiveKind::Pak => MountKind::Pak,
            ArchiveKind::Zip => MountKind::Pk3,
        };
        let mut entries = Vec::new();
        for (ordinal, entry) in archive.entries.iter().enumerate() {
            if entry.directory {
                continue;
            }
            entries.push(Entry {
                mount: self.mounts.len(),
                name: normalize(&entry.name)?.into_boxed_slice(),
                original_name: entry.name.clone(),
                source: EntrySource::Archive {
                    archive: Arc::clone(&archive),
                    entry: ordinal,
                },
                length: entry.length,
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
        let mut normalized = [0; 4096];
        let length = normalize_into(path, &mut normalized).ok()?;
        let name = &normalized[..length];
        let position = self
            .index
            .partition_point(|&index| self.entries[index].name.as_ref() < name);
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

    pub fn read_into_reusing(
        &self,
        reference: FileRef,
        destination: &mut [u8],
        reader: &mut qa_formats::archive::ArchiveReader,
    ) -> Result<usize, VfsError> {
        let entry = self.entry(reference)?;
        if let EntrySource::Archive { archive, entry } = &entry.source {
            return Ok(archive.read_into_reusing(*entry, destination, reader)?);
        }
        self.read_at(reference, 0, destination)
    }

    pub fn read_at(
        &self,
        reference: FileRef,
        offset: u64,
        destination: &mut [u8],
    ) -> Result<usize, VfsError> {
        let entry = self.entry(reference)?;
        if let EntrySource::Archive { archive, entry } = &entry.source {
            return Ok(archive.read_at(*entry, offset, destination)?);
        }
        let EntrySource::Loose(file) = &entry.source else {
            return Err(VfsError::Handle);
        };
        let remaining = entry.length.checked_sub(offset).ok_or(VfsError::Range)?;
        let count = destination
            .len()
            .min(usize::try_from(remaining).unwrap_or(usize::MAX));
        let mut written = 0;
        while written < count {
            let read = native_read(
                file,
                &mut destination[written..count],
                offset + written as u64,
            )?;
            if read == 0 {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            written += read;
        }
        Ok(written)
    }

    /// Module reads reuse the same load-owned inflate state for partial ZIP
    /// members as well as ordinary file ranges.
    pub fn read_range_reusing(
        &self,
        reference: FileRef,
        offset: u64,
        destination: &mut [u8],
        reader: &mut qa_formats::archive::ArchiveReader,
    ) -> Result<usize, VfsError> {
        let entry = self.entry(reference)?;
        if let EntrySource::Archive { archive, entry } = &entry.source {
            return Ok(archive.read_range_reusing(*entry, offset, destination, reader)?);
        }
        self.read_at(reference, offset, destination)
    }

    /// The product mount loader reuses loose archive handles already opened by
    /// mount_directory. This is a cold operation; it never reopens a package.
    pub fn source_file(&self, reference: FileRef) -> Option<Arc<File>> {
        match &self.entry(reference).ok()?.source {
            EntrySource::Loose(file) => Some(Arc::clone(file)),
            EntrySource::Archive { .. } => None,
        }
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
    /// Cold authority-bound reads can select saved settings even when a product
    /// has a same-named file. The existing winning index remains unchanged.
    pub fn files_in_mount(&self, id: MountId) -> impl Iterator<Item = (FileRef, &[u8])> {
        self.entries
            .iter()
            .enumerate()
            .filter_map(move |(index, entry)| {
                let mount = &self.mounts[entry.mount];
                (mount.active && mount.id == id).then_some((
                    FileRef {
                        entry: index as u32,
                    },
                    entry.name.as_ref(),
                ))
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

fn package_order(name: &[u8]) -> (u8, u64) {
    if let Some(stem) = name.strip_suffix(b".pak") {
        let number = stem
            .strip_prefix(b"pak")
            .filter(|digits| !digits.is_empty())
            .and_then(|digits| {
                digits.iter().try_fold(0u64, |number, &digit| {
                    if !digit.is_ascii_digit() {
                        return None;
                    }
                    number.checked_mul(10)?.checked_add(u64::from(digit - b'0'))
                })
            })
            .unwrap_or(u64::MAX);
        (0, number)
    } else {
        (1, 0)
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
use qa_formats::{
    FormatError,
    archive::{Archive, ArchiveKind},
};
