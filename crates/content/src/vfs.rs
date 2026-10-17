use qa_core::{
    names::{MAX_PATH_BYTES, NameTable},
    primitives::IndexSpan,
};
mod list;
pub use list::{FileListName, FileListQuery, FileListRequest, FileListing};

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
/// Immutable caller-selected mount order over this VFS's one file index.
/// New mounts stay outside a retained view until its caller explicitly rebuilds it.
#[derive(Clone)]
pub struct SearchView {
    owner: Arc<()>,
    ranks: Arc<[u32]>,
}
impl SearchView {
    fn rank(&self, mount: MountId) -> Option<u32> {
        self.ranks
            .get(mount.0 as usize)
            .copied()
            .filter(|rank| *rank != u32::MAX)
    }
}

/// Borrow the shared file table through one caller-selected search view.
#[derive(Clone, Copy)]
pub struct ReadView<'a> {
    vfs: &'a Vfs,
    search: Option<&'a SearchView>,
}
impl ReadView<'_> {
    pub fn open(self, path: &[u8]) -> Option<FileRef> {
        self.vfs.open_with(path, self.search)
    }
    pub fn read_file(
        self,
        path: &[u8],
        limit: usize,
        reader: &mut qa_formats::archive::ArchiveReader,
    ) -> Result<Vec<u8>, VfsError> {
        let file = self.open(path).ok_or(VfsError::Missing)?;
        self.vfs.read_to_vec(&file, limit, reader)
    }
}

/// One parsed package with VFS member paths admitted before publication.
/// The retained reader can still name an unpublished staging inode.
pub struct PreparedArchive {
    archive: Arc<Archive>,
    members: Box<[(usize, Box<[u8]>)]>,
    directories: Arc<[DirectoryName]>,
}
impl PreparedArchive {
    pub fn read(file: Arc<ReadFile>) -> Result<Self, VfsError> {
        Self::new(Arc::new(Archive::parse(file)?))
    }
    fn new(archive: Arc<Archive>) -> Result<Self, VfsError> {
        let mut members = Vec::new();
        let mut directories = Vec::new();
        for (ordinal, entry) in archive.entries.iter().enumerate() {
            if entry.directory {
                // Unused root/traversal directory markers were never file
                // admissions. Keep only valid virtual directory metadata.
                if let Ok(name) = normalize(entry.name.strip_suffix(b"/").unwrap_or(&entry.name)) {
                    directories.push(DirectoryName {
                        ordinal,
                        name: name.into_boxed_slice(),
                        original_name: entry.name.clone(),
                    });
                }
            } else {
                members.push((ordinal, normalize(&entry.name)?.into_boxed_slice()));
            }
        }
        Ok(Self {
            archive,
            members: members.into_boxed_slice(),
            directories: directories.into(),
        })
    }
    pub fn archive(&self) -> &Archive {
        &self.archive
    }
}

/// Retained opened source and metadata. Rename/unlink replacements preserve
/// this opened file; external writes to the same inode remain observable.
#[derive(Clone)]
pub struct FileRef {
    owner: Arc<()>,
    entry: Arc<Entry>,
}
impl std::fmt::Debug for FileRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileRef")
            .field("entry", &self.entry.slot)
            .field("generation", &self.entry.generation)
            .finish()
    }
}
impl PartialEq for FileRef {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == CmpOrdering::Equal
    }
}
impl Eq for FileRef {}
impl PartialOrd for FileRef {
    fn partial_cmp(&self, other: &Self) -> Option<CmpOrdering> {
        Some(self.cmp(other))
    }
}
impl Ord for FileRef {
    fn cmp(&self, other: &Self) -> CmpOrdering {
        Arc::as_ptr(&self.owner)
            .cmp(&Arc::as_ptr(&other.owner))
            .then(self.entry.slot.cmp(&other.entry.slot))
            .then(self.entry.generation.cmp(&other.entry.generation))
    }
}

type PackagePrecedence = ((u8, u64), Arc<[u8]>);

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct MountPrecedence {
    priority: i32,
    product: u32,
    package: Option<PackagePrecedence>,
    mount: u32,
}

/// Retained mount precedence from an accepted resource's VFS owner.
#[derive(Clone, Debug)]
pub struct SourcePrecedence {
    owner: Arc<()>,
    mount: MountPrecedence,
}
impl SourcePrecedence {
    /// Product priority, native package order, then mount order. Different VFS owners
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

#[derive(Clone)]
pub struct MountInfo {
    pub id: MountId,
    pub path: PathBuf,
    pub kind: MountKind,
    /// Larger product priorities win. Native package order applies within a
    /// product; equal-priority products prefer the later mounted root.
    pub priority: i32,
    active: bool,
    settings_only: bool,
    product: Option<MountId>,
    package: Option<PackagePrecedence>,
    contents: Option<MountContents>,
}
#[derive(Clone)]
struct MountContents {
    files: Arc<[u32]>,
    archive: Option<Arc<Archive>>,
    directories: Arc<[DirectoryName]>,
}
struct DirectoryName {
    ordinal: usize,
    name: Box<[u8]>,
    original_name: Box<[u8]>,
}
fn loose_directories(mut directories: Vec<DirectoryName>) -> Arc<[DirectoryName]> {
    directories.sort_unstable_by(|a, b| a.original_name.cmp(&b.original_name));
    directories.into()
}
impl MountInfo {
    fn retire(&mut self) {
        self.active = false;
        self.contents = None;
    }
    fn precedence(&self) -> MountPrecedence {
        MountPrecedence {
            priority: self.priority,
            product: self.product.unwrap_or(self.id).0,
            package: self.package.clone(),
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
    slot: u32,
    generation: u64,
    ordinal: usize,
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
    entries: Vec<Option<Arc<Entry>>>,
    names: Option<NameTable>,
    candidates: Box<[IndexSpan]>,
    listing: Vec<u32>,
    generation: u64,
    lookups: AtomicU64,
}

pub fn normalize(path: &[u8]) -> Result<Vec<u8>, VfsError> {
    let mut bytes = [0; MAX_PATH_BYTES];
    let length = normalize_into(path, &mut bytes)?;
    Ok(bytes[..length].to_vec())
}

fn normalize_into(path: &[u8], normalized: &mut [u8; MAX_PATH_BYTES]) -> Result<usize, VfsError> {
    if path.is_empty() || path.len() > MAX_PATH_BYTES || path.iter().any(|&c| c == 0 || c == b':') {
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
    pub fn read_view<'a>(&'a self, search: Option<&'a SearchView>) -> ReadView<'a> {
        ReadView { vfs: self, search }
    }
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
                .filter_map(|(slot, entry)| entry.as_ref().map(|entry| (slot, entry)))
                .filter(|(_, entry)| {
                    entry.mount == root.0 as usize
                        && !entry.name.contains(&b'/')
                        && ArchiveKind::from_path(&entry.name).is_some()
                })
                .filter_map(|(entry, value)| {
                    Some((
                        self.reference(entry as u32)?,
                        value.name.to_vec(),
                        value.original_name.to_vec(),
                    ))
                })
                .collect();
            packages.sort_unstable_by(|a, b| compare_packages(&a.1, &b.1));
            let mut mounts = vec![root];
            for (reference, _, name) in packages {
                let file = self.source_file(&reference).ok_or(VfsError::Handle)?;
                let archive = PreparedArchive::read(file)?;
                let name = std::str::from_utf8(&name).map_err(|_| VfsError::Path)?;
                let native_path = self.mounts[root.0 as usize].path.join(name);
                mounts.push(self.mount_archive_at(&native_path, priority, &archive, Some(root))?);
            }
            Ok(mounts)
        })();
        if result.is_err() {
            for mount in &mut self.mounts[first_mount..] {
                mount.retire();
            }
            for entry in &mut self.entries {
                if entry
                    .as_ref()
                    .is_some_and(|entry| entry.mount >= first_mount)
                {
                    *entry = None;
                }
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
        let scanned = scan_directory(&root, self.mounts.len(), settings_only, &[])?;
        self.publish(
            root,
            priority,
            settings_only,
            None,
            scanned.files,
            MountContents {
                files: Arc::from([]),
                archive: None,
                directories: loose_directories(scanned.directories),
            },
        )
    }

    /// Refresh only loose files below this canonical directory prefix. Empty
    /// prefixes refresh the entire mount. Existing FileRefs retain their opened
    /// sources; all new lookups observe the staged replacement atomically.
    pub fn refresh_directory_prefix(&mut self, id: MountId, prefix: &[u8]) -> Result<(), VfsError> {
        let mount = self
            .mounts
            .get(id.0 as usize)
            .filter(|mount| mount.active && mount.kind == MountKind::Directory)
            .ok_or(VfsError::Handle)?;
        let prefix = if prefix.is_empty() {
            Vec::new()
        } else {
            normalize(prefix)?
        };
        let scanned = scan_directory(&mount.path, id.0 as usize, mount.settings_only, &prefix)?;
        let previous = mount.contents.as_ref().ok_or(VfsError::Handle)?;
        let directories = previous
            .directories
            .iter()
            .filter(|directory| {
                !below_prefix(&directory.name, &prefix) && !below_prefix(&prefix, &directory.name)
            })
            .map(|directory| DirectoryName {
                ordinal: directory.ordinal,
                name: directory.name.clone(),
                original_name: directory.original_name.clone(),
            })
            .chain(scanned.directories)
            .collect::<Vec<_>>();
        let (entries, generation) = self.stage_entries(
            |entry| entry.mount == id.0 as usize && below_prefix(&entry.name, &prefix),
            scanned.files,
        )?;
        let mut mounts = self.mounts.clone();
        mounts[id.0 as usize]
            .contents
            .as_mut()
            .ok_or(VfsError::Handle)?
            .directories = loose_directories(directories);
        let index = FileIndex::build(&entries, &mut mounts)?;
        self.mounts = mounts;
        self.entries = entries;
        self.generation = generation;
        self.install_index(index);
        Ok(())
    }

    /// Index a completed owned download, then admit package members through
    /// the same archive loader as startup. Repeated resource-refresh attempts
    /// reuse the package mount; network publication never replaces its file.
    pub fn refresh_published_file(
        &mut self,
        id: MountId,
        name: &[u8],
        prepared: Option<&PreparedArchive>,
    ) -> Result<Option<MountId>, VfsError> {
        let name = normalize(name)?;
        let root = self
            .directory_path(id)
            .ok_or(VfsError::Handle)?
            .to_path_buf();
        let prefix = name
            .iter()
            .rposition(|&byte| byte == b'/')
            .map_or(&b""[..], |end| &name[..end]);
        self.refresh_directory_prefix(id, prefix)?;
        if ArchiveKind::from_path(&name).is_none() {
            return Ok(None);
        }
        // This authority-bound cold lookup deliberately ignores another
        // product's higher-priority loose file with the same virtual name.
        let reference = self
            .files_in_mount(id)
            .find(|(_, candidate)| *candidate == name)
            .map(|(file, _)| file)
            .ok_or(VfsError::Missing)?;
        let member = self.origin(&reference).ok_or(VfsError::Handle)?.member;
        let path = root.join(std::str::from_utf8(member).map_err(|_| VfsError::Path)?);
        if let Some(existing) = self
            .mounts
            .iter()
            .find(|mount| mount.active && mount.product == Some(id) && mount.path == path)
        {
            return Ok(Some(existing.id));
        }
        let parsed;
        let archive = if let Some(prepared) = prepared {
            prepared
        } else {
            let file = self.source_file(&reference).ok_or(VfsError::Handle)?;
            parsed = PreparedArchive::read(file)?;
            &parsed
        };
        let priority = self.mounts[id.0 as usize].priority;
        self.mount_archive_at(&path, priority, archive, Some(id))
            .map(Some)
    }

    pub fn mount_archive(
        &mut self,
        path: &Path,
        priority: i32,
        archive: Arc<Archive>,
    ) -> Result<MountId, VfsError> {
        self.mount_archive_at(path, priority, &PreparedArchive::new(archive)?, None)
    }
    fn mount_archive_at(
        &mut self,
        path: &Path,
        priority: i32,
        prepared: &PreparedArchive,
        product: Option<MountId>,
    ) -> Result<MountId, VfsError> {
        let mut entries = Vec::new();
        let archive = &prepared.archive;
        for (ordinal, name) in &prepared.members {
            let ordinal = *ordinal;
            let entry = &archive.entries[ordinal];
            entries.push(Entry {
                slot: 0,
                generation: 0,
                ordinal,
                mount: self.mounts.len(),
                name: name.clone(),
                original_name: entry.name.clone(),
                source: EntrySource::Archive {
                    archive: Arc::clone(archive),
                    entry: ordinal,
                },
                length: entry.length,
            });
        }
        self.publish(
            path.to_path_buf(),
            priority,
            false,
            product,
            entries,
            MountContents {
                files: Arc::from([]),
                archive: Some(Arc::clone(archive)),
                directories: Arc::clone(&prepared.directories),
            },
        )
    }

    fn publish(
        &mut self,
        path: PathBuf,
        priority: i32,
        settings_only: bool,
        product: Option<MountId>,
        entries: Vec<Entry>,
        contents: MountContents,
    ) -> Result<MountId, VfsError> {
        if self.mounts.len() >= 65536 {
            return Err(VfsError::Capacity);
        }
        let kind = match contents.archive.as_ref().map(|archive| archive.kind) {
            None => MountKind::Directory,
            Some(ArchiveKind::Pak) => MountKind::Pak,
            Some(ArchiveKind::Zip) => MountKind::Pk3,
        };
        let id = MountId(self.mounts.len() as u32);
        // Package names, not an insertion rank, keep retained asset precedence
        // valid when a later download sorts before an already loaded package.
        let package = if let Some(product) = product {
            let root = self
                .mounts
                .get(product.0 as usize)
                .filter(|mount| mount.active && mount.kind == MountKind::Directory)
                .ok_or(VfsError::Handle)?;
            let relative = path.strip_prefix(&root.path).map_err(|_| VfsError::Path)?;
            let name = normalize(relative.as_os_str().as_encoded_bytes())?;
            Some((package_order(&name), Arc::from(name)))
        } else {
            None
        };
        let mut mounts = self.mounts.clone();
        mounts.push(MountInfo {
            id,
            path,
            kind,
            priority,
            active: true,
            settings_only,
            product,
            package,
            contents: Some(contents),
        });
        let (entries, generation) = self.stage_entries(|_| false, entries)?;
        let index = FileIndex::build(&entries, &mut mounts)?;
        self.mounts = mounts;
        self.entries = entries;
        self.generation = generation;
        self.install_index(index);
        Ok(id)
    }

    fn stage_entries(
        &self,
        remove: impl Fn(&Entry) -> bool,
        added: Vec<Entry>,
    ) -> Result<(Vec<Option<Arc<Entry>>>, u64), VfsError> {
        let mut entries: Vec<_> = self
            .entries
            .iter()
            .map(|entry| entry.as_ref().filter(|entry| !remove(entry)).cloned())
            .collect();
        let mut free = 0;
        let mut generation = self.generation;
        for mut entry in added {
            while free < entries.len() && entries[free].is_some() {
                free += 1;
            }
            if free >= u32::MAX as usize {
                return Err(VfsError::Capacity);
            }
            generation = generation.checked_add(1).ok_or(VfsError::Capacity)?;
            entry.slot = free as u32;
            entry.generation = generation;
            let entry = Some(Arc::new(entry));
            if free == entries.len() {
                entries.push(entry);
            } else {
                entries[free] = entry;
            }
            free += 1;
        }
        while entries.last().is_some_and(Option::is_none) {
            entries.pop();
        }
        Ok((entries, generation))
    }

    fn install_index(&mut self, index: FileIndex) {
        self.names = Some(index.names);
        self.candidates = index.candidates;
        self.listing = index.listing;
    }

    fn reindex(&mut self) {
        let Some(names) = &self.names else {
            return;
        };
        (self.candidates, self.listing) = index_entries(&self.entries, &self.mounts, names);
    }

    fn reference(&self, slot: u32) -> Option<FileRef> {
        let entry = self.entries.get(slot as usize)?.as_ref()?;
        self.mounts.get(entry.mount).filter(|mount| mount.active)?;
        Some(FileRef {
            owner: Arc::clone(&self.owner),
            entry: Arc::clone(entry),
        })
    }

    /// Resolve names only while loading. Simulation/render/audio retain FileRefs.
    pub fn open(&self, path: &[u8]) -> Option<FileRef> {
        self.open_with(path, None)
    }
    pub fn search_view(
        &self,
        order: impl IntoIterator<Item = MountId>,
    ) -> Result<SearchView, VfsError> {
        let mut ranks = vec![u32::MAX; self.mounts.len()];
        let mut next = 0;
        for id in order {
            self.mounts
                .get(id.0 as usize)
                .filter(|mount| mount.active)
                .ok_or(VfsError::Handle)?;
            let rank = &mut ranks[id.0 as usize];
            if *rank == u32::MAX {
                *rank = next;
                next += 1;
            }
        }
        Ok(SearchView {
            owner: Arc::clone(&self.owner),
            ranks: ranks.into(),
        })
    }
    pub fn accepts_search(&self, view: &SearchView) -> bool {
        Arc::ptr_eq(&self.owner, &view.owner)
    }
    /// Name conversion and candidate traversal are shared by default and scoped
    /// reads. Selection does not mutate another caller's mount order or handles.
    pub fn open_with(&self, path: &[u8], view: Option<&SearchView>) -> Option<FileRef> {
        if view.is_some_and(|view| !self.accepts_search(view)) {
            return None;
        }
        let candidates = self.find_candidates(path)?;
        self.reference(self.select_candidate(candidates, view)?)
    }

    fn select_candidate(&self, candidates: &[u32], view: Option<&SearchView>) -> Option<u32> {
        if let Some(view) = view {
            candidates
                .iter()
                .filter_map(|&slot| {
                    let entry = self.entries.get(slot as usize)?.as_ref()?;
                    Some((slot, view.rank(MountId(entry.mount as u32))?))
                })
                .min_by_key(|(_, rank)| *rank)
                .map(|(slot, _)| slot)
        } else {
            candidates.first().copied()
        }
    }

    fn find_candidates(&self, path: &[u8]) -> Option<&[u32]> {
        self.lookups.fetch_add(1, Ordering::Relaxed);
        let mut normalized = [0; MAX_PATH_BYTES];
        let length = normalize_into(path, &mut normalized).ok()?;
        let id = self.names.as_ref()?.find(&normalized[..length])?;
        self.listing
            .get(self.candidates.get(id.0 as usize)?.range())
    }

    /// Native SERVER package requests include the product directory. Resolve
    /// only loose files inside explicitly mounted content roots, never settings
    /// or an archive member whose name happens to match a distributable package.
    pub fn open_product_file(&self, path: &[u8]) -> Option<FileRef> {
        let slash = path.iter().position(|byte| *byte == b'/')?;
        let product = &path[..slash];
        if product.is_empty()
            || product == b"."
            || product == b".."
            || product.iter().any(|byte| matches!(byte, b':' | b'\\' | 0))
        {
            return None;
        }
        let candidates = self.find_candidates(&path[slash + 1..])?;
        let slot = candidates.iter().copied().find(|&slot| {
            let Some(entry) = self.entries.get(slot as usize).and_then(Option::as_ref) else {
                return false;
            };
            let mount = &self.mounts[entry.mount];
            mount.active
                && mount.kind == MountKind::Directory
                && !mount.settings_only
                && mount
                    .path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| {
                        name.bytes()
                            .map(qa_core::names::path_byte)
                            .eq(product.iter().copied().map(qa_core::names::path_byte))
                    })
        })?;
        self.reference(slot)
    }

    fn entry<'a>(&self, reference: &'a FileRef) -> Result<&'a Entry, VfsError> {
        if !Arc::ptr_eq(&self.owner, &reference.owner)
            || !self
                .mounts
                .get(reference.entry.mount)
                .is_some_and(|mount| mount.active)
        {
            return Err(VfsError::Handle);
        }
        Ok(&reference.entry)
    }

    /// Snapshot the source used by a cold read. The value remains comparable
    /// after its file is removed, its mount is retired, or this VFS is dropped.
    pub fn source_precedence(&self, reference: &FileRef) -> Result<SourcePrecedence, VfsError> {
        let entry = self.entry(reference)?;
        Ok(SourcePrecedence {
            owner: Arc::clone(&self.owner),
            mount: self.mounts[entry.mount].precedence(),
        })
    }

    pub fn length(&self, reference: &FileRef) -> Result<u64, VfsError> {
        Ok(self.entry(reference)?.length)
    }
    pub fn name<'a>(&self, reference: &'a FileRef) -> Result<&'a [u8], VfsError> {
        Ok(&self.entry(reference)?.name)
    }

    /// Cold whole-file read through the same loose/archive range reader.
    pub fn read_to_vec(
        &self,
        reference: &FileRef,
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
        reference: &FileRef,
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
        self.read_view(None).read_file(path, limit, reader)
    }

    pub fn read_into_reusing(
        &self,
        reference: &FileRef,
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
        reference: &FileRef,
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
        reference: &FileRef,
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
        reference: &FileRef,
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
    pub fn source_file(&self, reference: &FileRef) -> Option<Arc<ReadFile>> {
        match &self.entry(reference).ok()?.source {
            EntrySource::Loose(file) => Some(Arc::clone(file)),
            EntrySource::Archive { .. } => None,
        }
    }

    pub fn origin<'a>(&'a self, reference: &'a FileRef) -> Option<Origin<'a>> {
        let entry = self.entry(reference).ok()?;
        let mount = &self.mounts[entry.mount];
        Some(Origin {
            mount: mount.id,
            path: &mount.path,
            member: &entry.original_name,
            kind: mount.kind,
        })
    }

    /// Native protocol metadata belongs to the retained mounted archive, not
    /// to a winning member name or a general-purpose content identity.
    pub fn q3_package_checksums(
        &self,
        mount: MountId,
    ) -> Option<&qa_formats::archive::Q3PackageChecksums> {
        self.mounts
            .get(mount.0 as usize)
            .filter(|mount| mount.active)?
            .contents
            .as_ref()?
            .archive
            .as_ref()?
            .q3_checksums()
    }
    pub fn mounts(&self) -> impl Iterator<Item = &MountInfo> {
        self.mounts.iter().filter(|mount| mount.active)
    }
    /// A caller explicitly selects this content directory before granting writes.
    /// Saved settings mounts and archives never become download destinations.
    pub fn directory_path(&self, id: MountId) -> Option<&Path> {
        self.mounts
            .get(id.0 as usize)
            .filter(|mount| {
                mount.active && mount.kind == MountKind::Directory && !mount.settings_only
            })
            .map(|mount| mount.path.as_path())
    }
    pub fn files(&self) -> impl Iterator<Item = (FileRef, &[u8])> {
        self.listing.iter().filter_map(|&slot| {
            let entry = self.entries.get(slot as usize)?.as_ref()?;
            Some((self.reference(slot)?, entry.name.as_ref()))
        })
    }

    /// Enumerate one selected source per name, using the same cached candidate
    /// spans as open_with. A caller view never admits a newly mounted source.
    pub fn files_with<'a>(
        &'a self,
        view: Option<&'a SearchView>,
    ) -> impl Iterator<Item = (FileRef, &'a [u8])> + 'a {
        let spans = if view.is_none_or(|view| self.accepts_search(view)) {
            self.candidates.as_ref()
        } else {
            &[]
        };
        // FileIndex owns a freshly sorted NameTable, so span order is lexical.
        spans.iter().filter_map(move |span| {
            let slot = self.select_candidate(self.listing.get(span.range())?, view)?;
            let entry = self.entries.get(slot as usize)?.as_ref()?;
            Some((self.reference(slot)?, entry.name.as_ref()))
        })
    }
    /// Cold authority-bound reads can select saved settings even when a product
    /// has a same-named file. The existing winner bindings remain unchanged.
    pub fn files_in_mount(&self, id: MountId) -> impl Iterator<Item = (FileRef, &[u8])> {
        self.mounts
            .get(id.0 as usize)
            .filter(|mount| mount.active)
            .and_then(|mount| mount.contents.as_ref())
            .into_iter()
            .flat_map(|contents| contents.files.iter())
            .filter_map(move |&slot| {
                let entry = self.entries.get(slot as usize)?.as_ref()?;
                Some((self.reference(slot)?, entry.name.as_ref()))
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
        mount.retire();
        for entry in &mut self.entries {
            if entry
                .as_ref()
                .is_some_and(|entry| entry.mount == id.0 as usize)
            {
                *entry = None;
            }
        }
        self.reindex();
        true
    }
}

struct FileIndex {
    names: NameTable,
    candidates: Box<[IndexSpan]>,
    listing: Vec<u32>,
}
impl FileIndex {
    fn build(entries: &[Option<Arc<Entry>>], mounts: &mut [MountInfo]) -> Result<Self, VfsError> {
        for mount in mounts.iter_mut().filter(|mount| mount.active) {
            let mut slots: Vec<_> = entries
                .iter()
                .flatten()
                .filter(|entry| entry.mount == mount.id.0 as usize)
                .map(|entry| entry.slot)
                .collect();
            slots.sort_unstable_by(|&a, &b| {
                let a = entries[a as usize].as_deref();
                let b = entries[b as usize].as_deref();
                match (a, b) {
                    (Some(a), Some(b)) if mount.kind == MountKind::Directory => {
                        a.original_name.cmp(&b.original_name)
                    }
                    (Some(a), Some(b)) => a.ordinal.cmp(&b.ordinal),
                    _ => CmpOrdering::Equal,
                }
            });
            if let Some(contents) = &mut mount.contents {
                contents.files = slots.into();
            }
        }
        let names = NameTable::load(
            entries
                .iter()
                .flatten()
                .filter(|entry| mounts[entry.mount].active)
                .map(|entry| entry.name.as_ref())
                .chain(
                    mounts
                        .iter()
                        .filter(|mount| mount.active)
                        .flat_map(|mount| {
                            mount.contents.as_ref().into_iter().flat_map(|contents| {
                                contents
                                    .directories
                                    .iter()
                                    .map(|directory| directory.name.as_ref())
                            })
                        }),
                )
                .chain(
                    mounts
                        .iter()
                        .filter(|mount| {
                            mount.active
                                && !mount.settings_only
                                && mount.kind == MountKind::Directory
                        })
                        .filter_map(|mount| {
                            mount.path.file_name().map(|name| name.as_encoded_bytes())
                        }),
                ),
        )
        .map_err(|_| VfsError::Capacity)?;
        let (candidates, listing) = index_entries(entries, mounts, &names);
        Ok(Self {
            names,
            candidates,
            listing,
        })
    }
}

fn index_entries(
    entries: &[Option<Arc<Entry>>],
    mounts: &[MountInfo],
    names: &NameTable,
) -> (Box<[IndexSpan]>, Vec<u32>) {
    let mut indexed: Vec<_> = entries
        .iter()
        .enumerate()
        .filter_map(|(slot, entry)| {
            let entry = entry.as_deref()?;
            let mount = mounts.get(entry.mount).filter(|mount| mount.active)?;
            let name = names.find(&entry.name)?;
            Some((slot as u32, entry, mount, name))
        })
        .collect();
    indexed.sort_unstable_by(|(_, x, x_mount, _), (_, y, y_mount, _)| {
        x.name
            .cmp(&y.name)
            .then_with(|| y_mount.precedence().cmp(&x_mount.precedence()))
            .then_with(|| match x_mount.kind {
                MountKind::Pk3 => y.ordinal.cmp(&x.ordinal),
                MountKind::Directory | MountKind::Pak => x.ordinal.cmp(&y.ordinal),
            })
    });
    let mut candidates = vec![IndexSpan::default(); names.len()].into_boxed_slice();
    let mut listing = Vec::with_capacity(indexed.len());
    for (slot, _, _, name) in indexed {
        let range = &mut candidates[name.0 as usize];
        if range.count == 0 {
            range.first = listing.len() as u32;
        }
        range.count += 1;
        listing.push(slot);
    }
    (candidates, listing)
}

fn below_prefix(name: &[u8], prefix: &[u8]) -> bool {
    prefix.is_empty()
        || name == prefix
        || name
            .strip_prefix(prefix)
            .is_some_and(|tail| tail.starts_with(b"/"))
}

struct ScannedEntries {
    files: Vec<Entry>,
    directories: Vec<DirectoryName>,
}
fn scan_directory(
    root: &Path,
    mount: usize,
    settings_only: bool,
    prefix: &[u8],
) -> Result<ScannedEntries, VfsError> {
    let mut pending = vec![root.to_path_buf()];
    let mut entries = Vec::new();
    let mut directories = Vec::new();
    while let Some(directory) = pending.pop() {
        for child in files::directory_entries(directory)? {
            if child.kind == EntryKind::Other
                || (settings_only
                    && match child.kind {
                        EntryKind::Directory => {
                            child.name.eq_ignore_ascii_case("assets")
                                || child.name.eq_ignore_ascii_case("saves")
                        }
                        EntryKind::File => !child.path.extension().is_some_and(|extension| {
                            extension.eq_ignore_ascii_case("cfg")
                                || extension.eq_ignore_ascii_case("json")
                        }),
                        EntryKind::Other => true,
                    })
            {
                continue;
            }
            let relative = child.path.strip_prefix(root).map_err(|_| VfsError::Path)?;
            let name = relative
                .to_str()
                .ok_or(VfsError::Path)?
                .replace(std::path::MAIN_SEPARATOR, "/")
                .into_bytes();
            let normalized = normalize(&name)?;
            if child.kind == EntryKind::Directory {
                if below_prefix(&normalized, prefix) || below_prefix(prefix, &normalized) {
                    directories.push(DirectoryName {
                        ordinal: directories.len(),
                        name: normalized.into_boxed_slice(),
                        original_name: name.into_boxed_slice(),
                    });
                    pending.push(child.path);
                }
                continue;
            }
            if !below_prefix(&normalized, prefix) {
                continue;
            }
            let file = Arc::new(ReadFile::open(child.path)?);
            let length = file.length()?;
            entries.push(Entry {
                slot: 0,
                generation: 0,
                ordinal: entries.len(),
                mount,
                name: normalized.into_boxed_slice(),
                original_name: name.into_boxed_slice(),
                source: EntrySource::Loose(file),
                length,
            });
        }
    }
    Ok(ScannedEntries {
        files: entries,
        directories,
    })
}

fn compare_packages(a: &[u8], b: &[u8]) -> CmpOrdering {
    package_order(a).cmp(&package_order(b)).then(a.cmp(b))
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
