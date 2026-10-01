//! Quake III application packages and downloads.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/network/q3-downloads.ts`
//! (`Q3ApplicationPackages`). Native checksums and download paths refer
//! only to the application's selected mounts. The donor is asynchronous
//! over `node:fs` promises; this port uses synchronous file reads. Pak
//! registration, pure checksums, reference tracking, and download-name
//! validation reuse `qa-net` (`registerQ3Pak`, `Q3ContentReferences`,
//! `checkQ3DownloadName`); archives reuse
//! [`open_archive`](qa_content::archive::open_archive); the catalog and
//! mount plan reuse the real [`InstalledCatalog`](qa_content::catalog::InstalledCatalog)
//! and [`ResolvedMountPlan`](qa_content::mounts::ResolvedMountPlan), so no
//! content-lane shim is needed. [`collect`](Q3ApplicationPackages::collect)
//! takes the opened mount sets explicitly (donor
//! `LoadedApplicationContent.openedMounts()`), letting callers with several
//! sets aggregate them.
//!
//! [`Q3DownloadReadFile`](qa_net::q3_net::Q3DownloadReadFile) has no error
//! channel, so post-open violations (read after close, archive changed on
//! disk) surface as terminal zero reads; the `qa-net` download pump turns
//! those into its loud `Source download file changed or returned an
//! invalid read` error, matching the donor's throws.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io;
use std::os::unix::fs::{FileExt, MetadataExt};
use std::path::Path;
use std::time::SystemTime;

use qa_content::archive::{open_archive, ArchiveEntry};
use qa_content::catalog::{CatalogError, InstalledCatalog};
use qa_content::contract::{
    create_content_digest, ArchiveFormat, ContentId, ContentMount, ContractError, GameFamily, MountId,
    ResolvedMountPlan, ResourceId,
};
use qa_content::hash::{hex_lower, Sha256};
use qa_content::mounts::{MountError, MountedContent};
use qa_net::common::session::SessionError;
use qa_net::q3_content::{register_q3_pak, Q3ContentError, Q3ContentReferences, Q3MountedPak};
use qa_net::q3_net::{check_q3_download_name, Q3ArchiveEntry, Q3ArchiveHandle, Q3NetError};
use thiserror::Error;

use qa_net::q3_net::Q3DownloadReadFile;

/// Quake III download failure.
#[derive(Debug, Error)]
pub enum Q3DownloadError {
    /// Policy, validation, or protocol failure with donor text.
    #[error("{0}")]
    Message(String),
    /// Filesystem failure.
    #[error("Q3 download filesystem failure: {0}")]
    Filesystem(String),
    /// Mount failure.
    #[error(transparent)]
    Mount(#[from] MountError),
    /// Catalog failure.
    #[error(transparent)]
    Catalog(#[from] CatalogError),
    /// Archive failure.
    #[error(transparent)]
    Archive(#[from] qa_content::archive::ArchiveError),
    /// Content reference failure.
    #[error(transparent)]
    Content(#[from] Q3ContentError),
    /// Wire failure.
    #[error(transparent)]
    Net(#[from] Q3NetError),
    /// Contract failure.
    #[error(transparent)]
    Contract(#[from] ContractError),
    /// Session failure.
    #[error(transparent)]
    Session(#[from] SessionError),
}

/// Maximum retained file bytes (donor `0x7fffffff`).
const MAX_DOWNLOAD_BYTES: u64 = 0x7FFF_FFFF;

/// Hash read chunk (donor 64 KiB).
const HASH_CHUNK_BYTES: usize = 65536;

/// Selected catalog and mount plan (donor `Pick<LoadedApplicationContent,
/// 'catalog' | 'mounts'>`, projected to the plan the packages walk reads).
pub struct Q3CatalogMounts<'a> {
    /// Installed catalog.
    pub catalog: &'a InstalledCatalog,
    /// Resolved mount plan.
    pub plan: &'a ResolvedMountPlan,
}

/// Quake III application packages (`Q3ApplicationPackages`).
pub struct Q3ApplicationPackages {
    /// Mounted paks in plan order.
    pub packs: Vec<Q3MountedPak>,
    /// Content reference tracker.
    pub references: Q3ContentReferences,
    members: HashMap<MountId, HashSet<String>>,
    processed: HashSet<ResourceId>,
}

impl Q3ApplicationPackages {
    /// Register the selected pk3 mounts and seed references (`open`).
    pub fn open(content: &Q3CatalogMounts<'_>, checksum_feed: i32) -> Result<Self, Q3DownloadError> {
        let mounts: HashMap<&MountId, &ContentMount> = content
            .plan
            .mounts
            .iter()
            .map(|mount| (&mount.identity().id, mount))
            .collect();
        let mut packs = Vec::new();
        let mut members = HashMap::new();
        for id in &content.plan.default_order {
            let Some(mount) = mounts.get(id) else {
                continue;
            };
            let ContentMount::Archive(archive) = mount else {
                continue;
            };
            if archive.format != ArchiveFormat::Pk3 {
                continue;
            }
            let product = content.catalog.product(archive.identity.content.as_str())?;
            if product.expectation.family != GameFamily::Q3 {
                continue;
            }
            let opened = open_archive(Path::new(&archive.archive_path), Some(archive.format))?;
            let game = product
                .expectation
                .content_directory
                .split('/')
                .next_back()
                .unwrap_or("");
            if game.is_empty() {
                return Err(Q3DownloadError::Message("Q3 archive has no game directory".to_string()));
            }
            let basename = Path::new(&archive.archive_path)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            let basename = strip_pk3_extension(&basename);
            let handle = archive_handle(&opened);
            let registered = register_q3_pak(archive.clone(), &handle, game, &basename, checksum_feed)?;
            members.insert(
                (*id).clone(),
                opened
                    .entries
                    .iter()
                    .filter(|entry| !is_directory(entry))
                    .map(|entry| entry.path().to_lowercase())
                    .collect(),
            );
            opened.close();
            packs.push(registered);
        }
        let references = Q3ContentReferences::new(packs.clone(), checksum_feed as u32, || 1.0)?;
        Ok(Self {
            packs,
            references,
            members,
            processed: HashSet::new(),
        })
    }

    /// Record opened resources from the caller's mount sets (`collect`).
    pub fn collect(&mut self, mounts: &[&MountedContent], catalog: &InstalledCatalog) -> Result<(), Q3DownloadError> {
        for mounts in mounts {
            for reference in mounts.opened_resources() {
                if self.processed.contains(&reference.id) {
                    continue;
                }
                let content = provenance_content(&reference.provenance);
                let product = catalog.product(content.as_str())?;
                if product.expectation.family != GameFamily::Q3 {
                    continue;
                }
                self.references.opened(&reference)?;
                self.processed.insert(reference.id.clone());
            }
        }
        Ok(())
    }

    /// Pure checksum for a member path, when a selected pak carries it.
    #[must_use]
    pub fn pure_checksum(&self, path: &str) -> Option<u32> {
        let lowered = path.to_lowercase();
        self.packs
            .iter()
            .find(|pack| {
                self.members
                    .get(&pack.mount.identity.id)
                    .is_some_and(|members| members.contains(&lowered))
            })
            .map(|pack| pack.pack.pure_checksum)
    }

    /// Open a pak download by `game/basename.pk3` name (`openDownload`).
    ///
    /// Invalid names fail; unmounted names return `None`. Mount identity
    /// guards the retained bytes: the digest must match the mount, and the
    /// size/mtime/ctime fingerprint must survive hashing.
    pub fn open_download(&self, name: &str) -> Result<Option<Box<dyn Q3DownloadReadFile>>, Q3DownloadError> {
        check_q3_download_name(name)?;
        let mounted = self.packs.iter().find(|pack| {
            format!("{}/{}.pk3", pack.pack.game, pack.pack.basename).to_lowercase() == name.to_lowercase()
        });
        let Some(mounted) = mounted else {
            return Ok(None);
        };
        let file =
            File::open(&mounted.mount.archive_path).map_err(|error| Q3DownloadError::Filesystem(error.to_string()))?;
        let original = file
            .metadata()
            .map_err(|error| Q3DownloadError::Filesystem(error.to_string()))?;
        let size = original.len();
        if !original.is_file() || size == 0 || size > MAX_DOWNLOAD_BYTES {
            return Ok(None);
        }
        let fingerprint = FileFingerprint::of(&original);
        let mut hash = Sha256::new();
        let mut buffer = vec![0u8; HASH_CHUNK_BYTES];
        let mut offset = 0;
        while offset < size {
            let end = (offset + HASH_CHUNK_BYTES as u64).min(size);
            let count = read_at(&file, &mut buffer[..(end - offset) as usize], offset)?;
            if count == 0 {
                return Err(Q3DownloadError::Message("Truncated mounted download".to_string()));
            }
            hash.update(&buffer[..count]);
            offset += count as u64;
        }
        fingerprint
            .check(&file)
            .map_err(|_| Q3DownloadError::Message("Download archive changed after opening".to_string()))?;
        let digest = hex_lower(&hash.finish());
        if create_content_digest(&digest)? != mounted.mount.archive_digest {
            return Err(Q3DownloadError::Message(
                "Download archive changed after mount".to_string(),
            ));
        }
        Ok(Some(Box::new(Q3DownloadFile {
            file: Some(file),
            size: size as i64,
            position: 0,
            fingerprint,
        })))
    }
}

/// Strip a trailing `.pk3` (case-insensitive, donor `/\.pk3$/i`).
fn strip_pk3_extension(basename: &str) -> String {
    basename
        .strip_suffix(".pk3")
        .or_else(|| basename.strip_suffix(".PK3"))
        .or_else(|| {
            basename
                .to_lowercase()
                .strip_suffix(".pk3")
                .map(|_| &basename[..basename.len() - 4])
        })
        .unwrap_or(basename)
        .to_string()
}

/// Whether an entry is a directory.
fn is_directory(entry: &ArchiveEntry) -> bool {
    match entry {
        ArchiveEntry::Pak(entry) => entry.is_directory,
        ArchiveEntry::Zip(entry) => entry.is_directory,
    }
}

/// Build a checksum handle over an opened archive.
pub(crate) fn archive_handle(opened: &qa_content::archive::OpenArchive) -> Q3ArchiveHandle {
    Q3ArchiveHandle {
        pak_format: false,
        entries: opened
            .entries
            .iter()
            .map(|entry| match entry {
                ArchiveEntry::Zip(entry) => Q3ArchiveEntry {
                    byte_length: entry.compressed_size as usize,
                    crc32: entry.crc32,
                    pak_entry: false,
                },
                ArchiveEntry::Pak(entry) => Q3ArchiveEntry {
                    byte_length: entry.compressed_size as usize,
                    crc32: 0,
                    pak_entry: true,
                },
            })
            .collect(),
    }
}

/// Content identity behind a resource provenance.
fn provenance_content(provenance: &qa_content::contract::ResourceProvenance) -> &ContentId {
    match provenance {
        qa_content::contract::ResourceProvenance::Archive { mount, .. } => &mount.identity.content,
        qa_content::contract::ResourceProvenance::Loose { mount, .. } => &mount.identity.content,
    }
}

/// Size/mtime/ctime fingerprint (donor `unchanged`).
#[derive(Debug, Clone, PartialEq, Eq)]
struct FileFingerprint {
    size: u64,
    mtime: SystemTime,
    ctime: (i64, i64),
}

impl FileFingerprint {
    fn of(metadata: &std::fs::Metadata) -> Self {
        Self {
            size: metadata.len(),
            mtime: metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            ctime: (metadata.ctime(), metadata.ctime_nsec()),
        }
    }

    fn check(&self, file: &File) -> io::Result<()> {
        let current = file.metadata()?;
        if current.len() != self.size
            || current.modified().unwrap_or(SystemTime::UNIX_EPOCH) != self.mtime
            || (current.ctime(), current.ctime_nsec()) != self.ctime
        {
            return Err(io::Error::other("Download archive changed after opening"));
        }
        Ok(())
    }
}

/// Positional read, mapping I/O failures to download errors.
fn read_at(file: &File, target: &mut [u8], offset: u64) -> Result<usize, Q3DownloadError> {
    file.read_at(target, offset)
        .map_err(|error| Q3DownloadError::Filesystem(error.to_string()))
}

/// Open pak download (donor `openDownload` result).
struct Q3DownloadFile {
    file: Option<File>,
    size: i64,
    position: u64,
    fingerprint: FileFingerprint,
}

impl Q3DownloadReadFile for Q3DownloadFile {
    fn size(&self) -> i64 {
        self.size
    }

    fn read(&mut self, target: &mut [u8]) -> usize {
        let Some(file) = self.file.as_ref() else {
            return 0;
        };
        if self.fingerprint.check(file).is_err() {
            return 0;
        }
        let end = (target.len() as u64).min(self.size as u64 - self.position.min(self.size as u64));
        let Ok(count) = file.read_at(&mut target[..end as usize], self.position) else {
            return 0;
        };
        self.position += count as u64;
        count
    }

    fn close(&mut self) {
        self.file = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_content::contract::MountPlanId;
    use qa_content::mounts::{open_mount_plan, OpenMountOptions};

    fn empty_plan() -> ResolvedMountPlan {
        ResolvedMountPlan {
            id: MountPlanId("mount-plan:test:q3dl".to_string()),
            mounts: Vec::new(),
            default_order: Vec::new(),
            prefix_orders: Vec::new(),
        }
    }

    fn empty_catalog() -> InstalledCatalog {
        InstalledCatalog::new(String::new(), Vec::new(), Vec::new(), 1, None).expect("catalog")
    }

    fn empty_mounts() -> MountedContent {
        let plan = empty_plan();
        open_mount_plan(
            &plan,
            OpenMountOptions {
                pure: None,
                q3_restriction: None,
                links: Vec::new(),
                loose_comparison: None,
            },
        )
        .expect("mounts")
    }

    #[test]
    fn opens_empty_selection() {
        let plan = empty_plan();
        let catalog = empty_catalog();
        let packages = Q3ApplicationPackages::open(
            &Q3CatalogMounts {
                catalog: &catalog,
                plan: &plan,
            },
            1,
        )
        .expect("open");
        assert!(packages.packs.is_empty());
        assert!(packages.pure_checksum("maps/q3dm1.bsp").is_none());
    }

    #[test]
    fn collect_ignores_unopened_resources() {
        let plan = empty_plan();
        let catalog = empty_catalog();
        let mut packages = Q3ApplicationPackages::open(
            &Q3CatalogMounts {
                catalog: &catalog,
                plan: &plan,
            },
            1,
        )
        .expect("open");
        let mounts = empty_mounts();
        packages.collect(&[&mounts], &catalog).expect("collect");
    }

    #[test]
    fn download_names_validate_before_lookup() {
        let plan = empty_plan();
        let catalog = empty_catalog();
        let packages = Q3ApplicationPackages::open(
            &Q3CatalogMounts {
                catalog: &catalog,
                plan: &plan,
            },
            1,
        )
        .expect("open");
        assert!(packages.open_download("../evil.pk3").is_err());
        assert!(packages.open_download("baseq3/missing.pk3").expect("lookup").is_none());
    }

    #[test]
    fn strips_pk3_suffix_case_insensitively() {
        assert_eq!(strip_pk3_extension("maps.pk3"), "maps");
        assert_eq!(strip_pk3_extension("maps.PK3"), "maps");
        assert_eq!(strip_pk3_extension("maps.Pk3"), "maps");
        assert_eq!(strip_pk3_extension("maps.zip"), "maps.zip");
    }
}
