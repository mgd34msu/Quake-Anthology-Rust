use qa_core::names::NameTable;
use qa_platform::files::{self, EntryKind, ReadFile};
use std::{
    cmp::Ordering as CmpOrdering,
    io,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MountId(pub u32);
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct FileRef {
    entry: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct MountPrecedence {
    priority: i32,
    mount: u32,
}

/// Retained mount precedence from an accepted resource's VFS owner.
#[derive(Clone, Debug)]
pub struct SourcePrecedence {
    owner: Arc<()>,
    mount: MountPrecedence,
}
impl SourcePrecedence {
    /// Greater means higher priority, then later mounted. Different VFS owners
    /// have no comparable search order, even if their numeric ranks coincide.
    pub fn compare(&self, other: &Self) -> Option<CmpOrdering> {
        Arc::ptr_eq(&self.owner, &other.owner).then(|| self.mount.cmp(&other.mount))
    }
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
impl MountInfo {
    fn precedence(&self) -> MountPrecedence {
        MountPrecedence {
            priority: self.priority,
            mount: self.id.0,
        }
    }
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
    Loose(Arc<ReadFile>),
    Archive { archive: Arc<Archive>, entry: usize },
}

#[derive(Debug)]
pub enum VfsError {
    Path,
    Capacity,
    Range,
    Handle,
    Missing,
    Size,
    Limit,
    Allocation,
    ShortRead,
    Io(io::Error),
    Format(FormatError),
}
impl std::fmt::Display for VfsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing => f.write_str("not found"),
            Self::Size => f.write_str("too large for this platform"),
            Self::Limit => f.write_str("size limit exceeded"),
            Self::Allocation => f.write_str("allocation failed"),
            Self::ShortRead => f.write_str("short read"),
            Self::Path => f.write_str("invalid path"),
            Self::Capacity => f.write_str("capacity exceeded"),
            Self::Range => f.write_str("invalid range"),
            Self::Handle => f.write_str("invalid handle"),
            Self::Io(error) => error.fmt(f),
            Self::Format(error) => write!(f, "{error:?}"),
        }
    }
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
    owner: Arc<()>,
    mounts: Vec<MountInfo>,
    entries: Vec<Entry>,
    names: Option<NameTable>,
    winners: Box<[Option<FileRef>]>,
    listing: Vec<FileRef>,
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
                        && [b".pak".as_slice(), b".pk3", b".pkz", b".kpf", b".zip"]
                            .iter()
                            .any(|extension| entry.name.ends_with(extension))
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
        let root = files::canonicalize(path)?;
        let mut pending = vec![root.clone()];
        let mut entries = Vec::new();
        while let Some(directory) = pending.pop() {
            for child in files::directory_entries(directory)? {
                if child.kind == EntryKind::Directory {
                    if !settings_only
                        || !(child.name.eq_ignore_ascii_case("assets")
                            || child.name.eq_ignore_ascii_case("saves"))
                    {
                        pending.push(child.path);
                    }
                    continue;
                }
                if child.kind != EntryKind::File {
                    continue;
                }
                let full = child.path;
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
                let file = Arc::new(ReadFile::open(full)?);
                let length = file.length()?;
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
        let names = NameTable::load(
            self.entries
                .iter()
                .chain(&entries)
                .map(|entry| entry.name.as_ref()),
        )
        .map_err(|_| VfsError::Capacity)?;
        let winners = vec![None; names.len()].into_boxed_slice();
        let id = MountId(self.mounts.len() as u32);
        self.mounts.push(MountInfo {
            id,
            path,
            kind,
            priority,
            active: true,
        });
        self.entries.extend(entries);
        self.names = Some(names);
        self.winners = winners;
        self.reindex();
        Ok(id)
    }

    fn reindex(&mut self) {
        self.listing.clear();
        self.listing.extend(
            self.entries
                .iter()
                .enumerate()
                .filter(|(_, entry)| self.mounts[entry.mount].active)
                .map(|(index, _)| FileRef {
                    entry: index as u32,
                }),
        );
        self.listing.sort_unstable_by(|a, b| {
            let x = &self.entries[a.entry as usize];
            let y = &self.entries[b.entry as usize];
            x.name
                .cmp(&y.name)
                .then_with(|| {
                    self.mounts[y.mount]
                        .precedence()
                        .cmp(&self.mounts[x.mount].precedence())
                })
                .then_with(|| match self.mounts[x.mount].kind {
                    MountKind::Pk3 => b.entry.cmp(&a.entry),
                    MountKind::Directory | MountKind::Pak => a.entry.cmp(&b.entry),
                })
        });
        self.winners.fill(None);
        if let Some(names) = &self.names {
            for &reference in &self.listing {
                if let Some(id) = names.find(&self.entries[reference.entry as usize].name) {
                    self.winners[id.0 as usize].get_or_insert(reference);
                }
            }
        }
    }

    /// Resolve names only while loading. Simulation/render/audio retain FileRefs.
    pub fn open(&self, path: &[u8]) -> Option<FileRef> {
        self.lookups.fetch_add(1, Ordering::Relaxed);
        let mut normalized = [0; 4096];
        let length = normalize_into(path, &mut normalized).ok()?;
        let id = self.names.as_ref()?.find(&normalized[..length])?;
        self.winners[id.0 as usize]
    }

    fn entry(&self, reference: FileRef) -> Result<&Entry, VfsError> {
        self.entries
            .get(reference.entry as usize)
            .filter(|entry| self.mounts[entry.mount].active)
            .ok_or(VfsError::Handle)
    }

    /// Snapshot the source used by a cold read. The value remains comparable
    /// after its file is removed, its mount is retired, or this VFS is dropped.
    pub fn source_precedence(&self, reference: FileRef) -> Result<SourcePrecedence, VfsError> {
        let entry = self.entry(reference)?;
        Ok(SourcePrecedence {
            owner: Arc::clone(&self.owner),
            mount: self.mounts[entry.mount].precedence(),
        })
    }

    pub fn length(&self, reference: FileRef) -> Result<u64, VfsError> {
        Ok(self.entry(reference)?.length)
    }
    pub fn name(&self, reference: FileRef) -> Result<&[u8], VfsError> {
        Ok(&self.entry(reference)?.name)
    }

    /// Cold whole-file read through the same loose/archive range reader.
    pub fn read_to_vec(
        &self,
        reference: FileRef,
        limit: usize,
        reader: &mut qa_formats::archive::ArchiveReader,
    ) -> Result<Vec<u8>, VfsError> {
        let length = usize::try_from(self.length(reference)?).map_err(|_| VfsError::Size)?;
        if length > limit {
            return Err(VfsError::Limit);
        }
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(length)
            .map_err(|_| VfsError::Allocation)?;
        bytes.resize(length, 0);
        self.read_whole_into(reference, &mut bytes, reader)?;
        Ok(bytes)
    }
    pub fn read_whole_into(
        &self,
        reference: FileRef,
        destination: &mut [u8],
        reader: &mut qa_formats::archive::ArchiveReader,
    ) -> Result<usize, VfsError> {
        let length = usize::try_from(self.length(reference)?).map_err(|_| VfsError::Size)?;
        if length > destination.len() {
            return Err(VfsError::Limit);
        }
        if self.read_into_reusing(reference, &mut destination[..length], reader)? != length {
            return Err(VfsError::ShortRead);
        }
        Ok(length)
    }
    pub fn read_file(
        &self,
        path: &[u8],
        limit: usize,
        reader: &mut qa_formats::archive::ArchiveReader,
    ) -> Result<Vec<u8>, VfsError> {
        let file = self.open(path).ok_or(VfsError::Missing)?;
        self.read_to_vec(file, limit, reader)
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
        self.read_loose(reference, 0, destination)
    }

    fn read_loose(
        &self,
        reference: FileRef,
        offset: u64,
        destination: &mut [u8],
    ) -> Result<usize, VfsError> {
        let entry = self.entry(reference)?;
        let EntrySource::Loose(file) = &entry.source else {
            return Err(VfsError::Handle);
        };
        let remaining = entry.length.checked_sub(offset).ok_or(VfsError::Range)?;
        let count = destination
            .len()
            .min(usize::try_from(remaining).unwrap_or(usize::MAX));
        file.read_exact_at(offset, &mut destination[..count])
            .map_err(FormatError::from_io)?;
        Ok(count)
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
        self.read_loose(reference, offset, destination)
    }

    /// Streamed consumers retain their own load-allocated archive cursor.
    /// Loose files use the same owned file range path; packages verify CRC at EOF.
    pub fn read_stream_reusing(
        &self,
        reference: FileRef,
        offset: u64,
        destination: &mut [u8],
        reader: &mut qa_formats::archive::ArchiveReader,
    ) -> Result<usize, VfsError> {
        let entry = self.entry(reference)?;
        if let EntrySource::Archive { archive, entry } = &entry.source {
            return Ok(archive.read_stream_reusing(*entry, offset, destination, reader)?);
        }
        self.read_loose(reference, offset, destination)
    }

    /// The product mount loader reuses loose archive handles already opened by
    /// mount_directory. This is a cold operation; it never reopens a package.
    pub fn source_file(&self, reference: FileRef) -> Option<Arc<ReadFile>> {
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
        self.listing.iter().map(|&reference| {
            (
                reference,
                self.entries[reference.entry as usize].name.as_ref(),
            )
        })
    }
    /// Cold authority-bound reads can select saved settings even when a product
    /// has a same-named file. The existing winner bindings remain unchanged.
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

impl From<io::ErrorKind> for VfsError {
    fn from(kind: io::ErrorKind) -> Self {
        Self::Io(kind.into())
    }
}
use qa_formats::{
    FormatError,
    archive::{Archive, ArchiveKind},
};
