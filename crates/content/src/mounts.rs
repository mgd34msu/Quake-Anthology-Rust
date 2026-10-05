//! Mount plans: resource resolution, opening, and reading over archives.
//!
//! Donors: `src/content/mounts/index.ts` and `src/content/mounts/paths.ts`.
//! Path normalization, root containment, and case resolution already live in
//! [`crate::paths`]; only the [`is_missing_file`] gap is ported here. The
//! donor is async; this port is synchronous. Quake III archive checksums
//! come from `src/network/q3/pure.ts` (`q3ArchiveChecksums`) and are
//! implemented over [`crate::archive::OpenArchive`] since content sits below
//! networking in the layering.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, OnceLock};
use std::time::SystemTime;

use thiserror::Error;

use crate::archive::{
    file_identity_at, open_archive, read_loose_entry, ArchiveEntry, ArchiveError, ArchivePathComparison, EntryRef,
    OpenArchive,
};
use crate::contract::{
    create_resource_id, is_content_digest, ArchiveFormat, ArchiveMount, ContentDigest, ContentMount, ContractError,
    LooseMount, MountId, MountIdentity, MountPlanId, PrefixMountOrder, ResolvedMountPlan, ResolvedResourceReference,
    ResourceProvenance, ResourceResolution, UnresolvedResourceReference,
};
use crate::hash::{
    hex_lower, md4_block_checksum, md4_block_checksum_key, record_file_digest, sha256_hex, stored_file_digest, Sha256,
};
use crate::paths::{find_content_path, normalize_resource_path, PathComparison, PathError};

/// Mount failure (donor `RangeError`/`Error` throws plus wrapped sources).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MountError {
    /// Invalid plan, order, link, or checksum input (donor `RangeError`).
    #[error("{0}")]
    Invalid(String),
    /// Use after close (donor `Error` throws).
    #[error("{0}")]
    Closed(String),
    /// Mount, resolution, or verification failure (donor `Error` throws).
    #[error("{0}")]
    Failed(String),
    /// Archive failure.
    #[error(transparent)]
    Archive(#[from] ArchiveError),
    /// Path failure.
    #[error(transparent)]
    Path(#[from] PathError),
    /// Contract failure.
    #[error(transparent)]
    Contract(#[from] ContractError),
    /// Filesystem access failed.
    #[error("Mount I/O error for {path}: {message}")]
    Io {
        /// Requested path.
        path: String,
        /// Underlying error.
        message: String,
    },
}

/// Whether an I/O error means a missing file or directory (`isMissingFile`).
#[must_use]
pub fn is_missing_file(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
    )
}

#[must_use]
fn content_digest_hex(hex: String) -> ContentDigest {
    let digest = ContentDigest(format!("sha256:{hex}"));
    debug_assert!(is_content_digest(digest.as_str()));
    digest
}

/// SHA-256 digest of bytes (`digestBytes`).
///
/// Only real consumers call this: declaration checks hash opened bytes
/// at their own site, and [`archive_digest_or_compute`] hashes archive
/// files when a lazy digest is forced. Opening resources never hashes.
#[must_use]
pub fn digest_bytes(bytes: &[u8]) -> ContentDigest {
    content_digest_hex(sha256_hex(bytes))
}

/// Mint a resource identity token without hashing.
///
/// The token carries the payload length, the archive central-directory
/// CRC (or zero), and a per-source tag (loose mtime, else zero). It
/// identifies bytes for resource IDs and manifests; it is not a content
/// hash, and nothing verifies bytes against it.
fn resource_identity_token(byte_length: u64, crc: u32, tag: u64) -> ContentDigest {
    ContentDigest(format!("sha256:{crc:08x}{byte_length:016x}{tag:016x}{:024x}", 0))
}

/// Archive content digest, forcing computation when nothing is cached.
///
/// Resolution order: the mount's lazy cell, the on-disk store keyed by
/// filesystem identity, then a fresh 1 MiB-streamed hash (recorded
/// back). Only real consumers call this (download verification, save
/// provenance, pure-server checks); discovery and opening never do.
pub fn archive_digest_or_compute(mount: &ArchiveMount) -> Result<ContentDigest, MountError> {
    if let Some(digest) = mount.archive_digest.get() {
        return Ok(digest);
    }
    let path = Path::new(&mount.archive_path);
    let identity = file_identity_at(path)?;
    if let Some(stored) = stored_file_digest(&mount.archive_path, &identity) {
        if is_content_digest(&stored) {
            let digest = ContentDigest(stored);
            mount.archive_digest.set(digest.clone());
            return Ok(digest);
        }
    }
    let mut file = File::open(path).map_err(|error| MountError::Io {
        path: path.to_string_lossy().into_owned(),
        message: error.to_string(),
    })?;
    let mut hasher = Sha256::new();
    let mut chunk = vec![0u8; 1024 * 1024];
    loop {
        let count = file.read(&mut chunk).map_err(|error| MountError::Io {
            path: path.to_string_lossy().into_owned(),
            message: error.to_string(),
        })?;
        if count == 0 {
            break;
        }
        hasher.update(&chunk[..count]);
    }
    let digest = content_digest_hex(hex_lower(&hasher.finish()));
    mount.archive_digest.set(digest.clone());
    if file_identity_at(path).is_ok_and(|after| after == identity) {
        record_file_digest(&mount.archive_path, &identity, digest.as_str());
    }
    Ok(digest)
}

/// Resource prefix link into a loose mount (`ResourceLink`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ResourceLink {
    /// Requested-path prefix to rewrite.
    pub source_prefix: String,
    /// Loose mount holding the targets.
    pub mount: MountId,
    /// Replacement prefix.
    pub target_prefix: String,
}

/// Pure-server archive policy (`PureMountPolicy`).
///
/// The network adapter resolves native pure checksums to verified archive
/// digests.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PureMountPolicy {
    /// Pure archives, highest priority first.
    pub archives: Vec<ContentDigest>,
}

/// Quake III content restriction (`q3Restriction`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q3Restriction {
    /// Restricted demo content (`"demo"`).
    Demo,
}

/// Mount open options (`OpenMountOptions`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OpenMountOptions {
    /// Pure-server archive policy.
    pub pure: Option<PureMountPolicy>,
    /// Quake III content restriction.
    pub q3_restriction: Option<Q3Restriction>,
    /// Resource prefix links.
    pub links: Vec<ResourceLink>,
    /// Loose path comparison. Reads default to case-insensitive when absent
    /// (via [`find_content_path`]); borrow comparison treats absence as
    /// exact, mirroring the donor.
    pub loose_comparison: Option<PathComparison>,
}

/// Opened resource bytes plus their resolved reference (`OpenedResource`).
#[derive(Debug, Clone)]
pub struct OpenedResource {
    /// Resolved reference.
    pub reference: ResolvedResourceReference,
    /// Resource bytes.
    pub bytes: Vec<u8>,
    /// True content hash, computed only when a real consumer asks.
    cached_content_digest: Arc<OnceLock<ContentDigest>>,
}

impl OpenedResource {
    /// Wrap opened bytes with their resolved reference.
    pub fn new(reference: ResolvedResourceReference, bytes: Vec<u8>) -> Self {
        Self {
            reference,
            bytes,
            cached_content_digest: Arc::new(OnceLock::new()),
        }
    }

    /// True SHA-256 over the bytes, hashed at most once per instance.
    ///
    /// Declaration and definition checks call this at their own site;
    /// opening and reading never do. Clones share the cached value.
    pub fn content_digest(&self) -> &ContentDigest {
        self.cached_content_digest.get_or_init(|| digest_bytes(&self.bytes))
    }

    /// Whether the content hash was computed.
    #[must_use]
    pub fn is_content_digest_computed(&self) -> bool {
        self.cached_content_digest.get().is_some()
    }
}

impl PartialEq for OpenedResource {
    fn eq(&self, other: &Self) -> bool {
        self.reference == other.reference && self.bytes == other.bytes
    }
}

impl Eq for OpenedResource {}

/// Resource read selector: path or previously resolved reference.
#[derive(Debug, Clone, Copy)]
pub enum ResourceRef<'a> {
    /// Resolve and read by path.
    Path(&'a str),
    /// Re-read a previously resolved reference.
    Resolved(&'a ResolvedResourceReference),
}

/// Quake III archive checksums (`q3ArchiveChecksums`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q3ArchiveChecksums {
    /// Checksum over nonempty entry CRC words.
    pub checksum: u32,
    /// Checksum keyed by the checksum feed.
    pub pure_checksum: u32,
}

/// Expected `pak0.pk3` checksum for restricted Q3 demo content.
pub const Q3_DEMO_PAK0_CHECKSUM: u32 = 437_558_517;

/// Checksum nonempty entries in physical central-directory order.
///
/// Donor: `q3ArchiveChecksums` in `src/network/q3/pure.ts` (FS_LoadZipFile
/// checksums, files.c; Copyright (C) 1999-2005 Id Software, Inc.,
/// GPL-2.0-or-later). Implemented here over [`OpenArchive`] because content
/// cannot depend on the network crate.
pub fn q3_archive_checksums(archive: &OpenArchive, checksum_feed: u32) -> Result<Q3ArchiveChecksums, MountError> {
    if archive.format == ArchiveFormat::Pak {
        return Err(MountError::Invalid(
            "Q3 pak checksums require a ZIP/PK3 central directory".to_string(),
        ));
    }
    let mut crcs = Vec::new();
    for entry in &archive.entries {
        if entry.byte_length() == 0 {
            continue;
        }
        let ArchiveEntry::Zip(zip) = entry else {
            return Err(MountError::Invalid("Q3 ZIP directory contains a PAK entry".to_string()));
        };
        crcs.extend_from_slice(&zip.crc32.to_le_bytes());
    }
    Ok(Q3ArchiveChecksums {
        checksum: md4_block_checksum(&crcs),
        pure_checksum: md4_block_checksum_key(&crcs, checksum_feed),
    })
}

fn verify_demo_format(archive: &OpenArchive, archive_path: &str) -> Result<(), MountError> {
    if archive.format != ArchiveFormat::Pk3 {
        return Err(MountError::Failed(format!(
            "Restricted Q3 content requires PK3 archives: {archive_path}"
        )));
    }
    Ok(())
}

/// Open mount backing store.
#[derive(Debug, Clone)]
enum MountedSource {
    Archive {
        mount: ArchiveMount,
        archive: Rc<OpenArchive>,
    },
    Loose {
        mount: LooseMount,
    },
}

impl MountedSource {
    fn content_mount(&self) -> ContentMount {
        match self {
            MountedSource::Archive { mount, .. } => ContentMount::Archive(mount.clone()),
            MountedSource::Loose { mount } => ContentMount::Loose(mount.clone()),
        }
    }
}

fn provenance_identity(provenance: &ResourceProvenance) -> &MountIdentity {
    match provenance {
        ResourceProvenance::Archive { mount, .. } => &mount.identity,
        ResourceProvenance::Loose { mount, .. } => &mount.identity,
    }
}

fn source_identity(source: &MountedSource) -> &MountIdentity {
    match source {
        MountedSource::Archive { mount, .. } => &mount.identity,
        MountedSource::Loose { mount } => &mount.identity,
    }
}

fn validate_order(order: &[MountId], mounts: &HashMap<MountId, ContentMount>) -> Result<(), MountError> {
    let mut seen = HashSet::new();
    for id in order {
        if !mounts.contains_key(id) {
            return Err(MountError::Invalid(format!(
                "Mount order refers to unknown mount: {id}"
            )));
        }
        if !seen.insert(id) {
            return Err(MountError::Invalid(format!("Mount order repeats mount: {id}")));
        }
    }
    if seen.len() != mounts.len() {
        return Err(MountError::Invalid(
            "Mount order must include every fallback mount".to_string(),
        ));
    }
    Ok(())
}

fn pure_order(
    order: &[MountId],
    mounts: &HashMap<MountId, ContentMount>,
    pure: Option<&PureMountPolicy>,
) -> Result<Vec<MountId>, MountError> {
    let Some(pure) = pure else {
        return Ok(order.to_vec());
    };
    if pure.archives.is_empty() {
        return Ok(order.to_vec());
    }
    // The pure check is a real consumer: force the candidate digests
    // once here, at the plan-open boundary, then match plain values.
    let mut forced: HashMap<&MountId, ContentDigest> = HashMap::new();
    for id in order {
        if let Some(ContentMount::Archive(mount)) = mounts.get(id) {
            forced.insert(id, archive_digest_or_compute(mount)?);
        }
    }
    let mut remaining: Vec<MountId> = order.to_vec();
    let mut first = Vec::new();
    for digest in &pure.archives {
        if let Some(position) = remaining
            .iter()
            .position(|id| forced.get(id).is_some_and(|forced| forced == digest))
        {
            first.push(remaining.remove(position));
        }
    }
    first.extend(remaining);
    Ok(first)
}

#[must_use]
fn pure_loose_path(path: &str) -> bool {
    let folded = path.to_lowercase();
    [".cfg", ".menu", ".game", ".dm_68", ".dat"]
        .iter()
        .any(|suffix| folded.ends_with(suffix))
}

#[must_use]
fn strip_one_trailing_slash(prefix: &str) -> &str {
    prefix.strip_suffix('/').unwrap_or(prefix)
}

fn resolve_order(
    plan: &ResolvedMountPlan,
    mounts: &HashMap<MountId, ContentMount>,
    pure: Option<&PureMountPolicy>,
) -> Result<ResolvedMountPlan, MountError> {
    validate_order(&plan.default_order, mounts)?;
    for order in &plan.prefix_orders {
        normalize_resource_path(strip_one_trailing_slash(&order.prefix))?;
    }
    let mut prefix_orders = Vec::new();
    for order in &plan.prefix_orders {
        prefix_orders.push(PrefixMountOrder {
            prefix: order.prefix.clone(),
            mounts: pure_order(&order.mounts, mounts, pure)?,
        });
    }
    Ok(ResolvedMountPlan {
        id: plan.id.clone(),
        mounts: plan.mounts.clone(),
        default_order: pure_order(&plan.default_order, mounts, pure)?,
        prefix_orders,
    })
}

fn resolve_mount_plan(plan: &ResolvedMountPlan, options: &OpenMountOptions) -> Result<ResolvedMountPlan, MountError> {
    let mut mounts = HashMap::new();
    for mount in &plan.mounts {
        if mounts.insert(mount.identity().id.clone(), mount.clone()).is_some() {
            return Err(MountError::Invalid(format!(
                "Repeated mount identity: {}",
                mount.identity().id
            )));
        }
    }
    let resolved = resolve_order(plan, &mounts, options.pure.as_ref())?;
    for link in &options.links {
        if !matches!(mounts.get(&link.mount), Some(ContentMount::Loose(_))) {
            return Err(MountError::Invalid(format!(
                "Link must target a mounted loose root: {}",
                link.mount
            )));
        }
        normalize_resource_path(strip_one_trailing_slash(&link.source_prefix))?;
        if !link.target_prefix.is_empty() {
            normalize_resource_path(strip_one_trailing_slash(&link.target_prefix))?;
        }
    }
    if let Some(pure) = &options.pure {
        if !pure.archives.is_empty() {
            let mut available: HashSet<ContentDigest> = HashSet::new();
            for mount in &plan.mounts {
                if let ContentMount::Archive(mount) = mount {
                    available.insert(archive_digest_or_compute(mount)?);
                }
            }
            for digest in &pure.archives {
                if !available.contains(digest) {
                    return Err(MountError::Failed(format!(
                        "Required pure archive is missing: {digest}"
                    )));
                }
            }
        }
    }
    Ok(resolved)
}

#[must_use]
fn same_mount(left: &ContentMount, right: &ContentMount) -> bool {
    if left.identity().id != right.identity().id
        || left.identity().content != right.identity().content
        || left.identity().generation != right.identity().generation
    {
        return false;
    }
    match (left, right) {
        (ContentMount::Archive(left), ContentMount::Archive(right)) => {
            left.archive_path == right.archive_path && left.format == right.format
        }
        (ContentMount::Loose(left), ContentMount::Loose(right)) => left.root_path == right.root_path,
        _ => false,
    }
}

/// Selected mount plan owning its open archives (`MountedContent`).
///
/// A selected plan owns its open archives; no process-global search path is
/// mutated. Borrowed plans share sources with their owner and fail once any
/// owner in the chain closes. Dropping a plan closes it.
#[derive(Debug)]
pub struct MountedContent {
    /// Resolved plan.
    pub plan: ResolvedMountPlan,
    /// Open options.
    pub options: OpenMountOptions,
    sources: HashMap<MountId, MountedSource>,
    user_mounts: HashSet<MountId>,
    referenced: RefCell<HashMap<MountId, ArchiveMount>>,
    opened_resources: RefCell<HashMap<String, ResolvedResourceReference>>,
    ancestors: Vec<Rc<Cell<bool>>>,
    live: Rc<Cell<bool>>,
    owns_archives: bool,
}

impl MountedContent {
    fn assemble(
        plan: ResolvedMountPlan,
        sources: Vec<MountedSource>,
        options: OpenMountOptions,
        ancestors: Vec<Rc<Cell<bool>>>,
        owns_archives: bool,
    ) -> Self {
        let mut map = HashMap::new();
        for source in sources {
            map.insert(source_identity(&source).id.clone(), source);
        }
        Self {
            plan,
            options,
            sources: map,
            user_mounts: HashSet::new(),
            referenced: RefCell::new(HashMap::new()),
            opened_resources: RefCell::new(HashMap::new()),
            ancestors,
            live: Rc::new(Cell::new(true)),
            owns_archives,
        }
    }

    /// References opened through this plan.
    #[must_use]
    pub fn opened_resources(&self) -> Vec<ResolvedResourceReference> {
        self.opened_resources.borrow().values().cloned().collect()
    }

    /// Archive mounts that produced bytes through this plan.
    #[must_use]
    pub fn referenced_archives(&self) -> Vec<ArchiveMount> {
        self.referenced.borrow().values().cloned().collect()
    }

    /// Re-check every archive source against its mount-time identity.
    ///
    /// Map loads call this at their boundary instead of on every read.
    pub fn verify_archives(&self) -> Result<(), MountError> {
        self.assert_open()?;
        for source in self.sources.values() {
            if let MountedSource::Archive { archive, .. } = source {
                archive.verify_storage()?;
            }
        }
        Ok(())
    }

    /// Fail once this plan or any owner in the chain is closed.
    pub fn assert_open(&self) -> Result<(), MountError> {
        if !self.live.get() || self.ancestors.iter().any(|owner| !owner.get()) {
            return Err(MountError::Closed("Content mount plan is closed".to_string()));
        }
        Ok(())
    }

    fn effective_loose_comparison(&self) -> PathComparison {
        self.options.loose_comparison.unwrap_or(PathComparison::CaseInsensitive)
    }

    /// A locally closable scope over identical sources; `None` requests an
    /// independently opened plan.
    pub fn borrow_mount_plan(
        &self,
        plan: &ResolvedMountPlan,
        mut options: OpenMountOptions,
    ) -> Result<Option<MountedContent>, MountError> {
        self.assert_open()?;
        if self.options.q3_restriction.is_some() {
            options.q3_restriction = self.options.q3_restriction;
        }
        if options.q3_restriction != self.options.q3_restriction {
            return Ok(None);
        }
        let resolved = resolve_mount_plan(plan, &options)?;
        let mut sources = Vec::new();
        for mount in &plan.mounts {
            let Some(source) = self.sources.get(&mount.identity().id) else {
                return Ok(None);
            };
            if !same_mount(&source.content_mount(), mount) {
                return Ok(None);
            }
            sources.push(source.clone());
        }
        if options.loose_comparison.unwrap_or(PathComparison::Exact)
            != self.options.loose_comparison.unwrap_or(PathComparison::Exact)
        {
            return Ok(None);
        }
        if options.links.len() != self.options.links.len()
            || options
                .links
                .iter()
                .zip(self.options.links.iter())
                .any(|(link, parent)| {
                    link.mount != parent.mount
                        || link.source_prefix != parent.source_prefix
                        || link.target_prefix != parent.target_prefix
                })
        {
            return Ok(None);
        }
        let mut forced = Vec::new();
        for mount in &plan.mounts {
            if let ContentMount::Archive(mount) = mount {
                forced.push(archive_digest_or_compute(mount)?);
            }
        }
        let available: HashSet<&ContentDigest> = forced.iter().collect();
        let parent_pure: &[ContentDigest] = self
            .options
            .pure
            .as_ref()
            .map(|pure| pure.archives.as_slice())
            .unwrap_or(&[]);
        let pure: &[ContentDigest] = options
            .pure
            .as_ref()
            .map(|pure| pure.archives.as_slice())
            .unwrap_or(&[]);
        let expected: Vec<&ContentDigest> = parent_pure
            .iter()
            .filter(|digest| available.contains(*digest))
            .collect();
        if !parent_pure.is_empty() && expected.is_empty()
            || pure.len() != expected.len()
            || pure
                .iter()
                .zip(expected.iter())
                .any(|(digest, expected)| digest != *expected)
        {
            return Ok(None);
        }
        let mut ancestors = self.ancestors.clone();
        ancestors.push(self.live.clone());
        let mut borrowed = Self::assemble(resolved, sources, options, ancestors, false);
        for id in &self.user_mounts {
            if borrowed.sources.contains_key(id) {
                borrowed.user_mounts.insert(id.clone());
            }
        }
        Ok(Some(borrowed))
    }

    /// A scoped user overlay borrowing installed archives without taking
    /// ownership of them.
    pub fn borrow_with_loose_mount(&self, mount: LooseMount, id: MountPlanId) -> Result<MountedContent, MountError> {
        self.assert_open()?;
        if self.sources.contains_key(&mount.identity.id) {
            return Err(MountError::Failed(format!(
                "Duplicate user mount: {}",
                mount.identity.id
            )));
        }
        let plan = ResolvedMountPlan {
            id,
            mounts: std::iter::once(ContentMount::Loose(mount.clone()))
                .chain(self.plan.mounts.iter().cloned())
                .collect(),
            default_order: std::iter::once(mount.identity.id.clone())
                .chain(self.plan.default_order.iter().cloned())
                .collect(),
            prefix_orders: self
                .plan
                .prefix_orders
                .iter()
                .map(|order| PrefixMountOrder {
                    prefix: order.prefix.clone(),
                    mounts: std::iter::once(mount.identity.id.clone())
                        .chain(order.mounts.iter().cloned())
                        .collect(),
                })
                .collect(),
        };
        let mut sources = vec![MountedSource::Loose { mount: mount.clone() }];
        sources.extend(self.sources.values().cloned());
        let mut ancestors = self.ancestors.clone();
        ancestors.push(self.live.clone());
        let mut borrowed = Self::assemble(plan, sources, self.options.clone(), ancestors, false);
        borrowed.user_mounts = self.user_mounts.clone();
        borrowed.user_mounts.insert(mount.identity.id.clone());
        Ok(borrowed)
    }

    /// Borrows verified sources until this owner closes; the reader cannot
    /// close them.
    pub fn borrow_ordered_reader(&self, order: &OrderedPlan) -> Result<OrderedReader<'_>, MountError> {
        self.assert_open()?;
        let mounts: HashMap<MountId, ContentMount> = self
            .plan
            .mounts
            .iter()
            .map(|mount| (mount.identity().id.clone(), mount.clone()))
            .collect();
        let plan = resolve_order(
            &ResolvedMountPlan {
                id: order.id.clone(),
                mounts: self.plan.mounts.clone(),
                default_order: order.default_order.clone(),
                prefix_orders: order.prefix_orders.clone(),
            },
            &mounts,
            self.options.pure.as_ref(),
        )?;
        Ok(OrderedReader {
            plan,
            owner: self,
            opened: RefCell::new(HashMap::new()),
        })
    }

    fn allowed(&self, source: &MountedSource, path: &str) -> Result<bool, MountError> {
        if let MountedSource::Loose { mount } = source {
            if self.user_mounts.contains(&mount.identity.id) {
                return Ok(true);
            }
        }
        if matches!(source, MountedSource::Loose { .. })
            && self.options.q3_restriction == Some(Q3Restriction::Demo)
            && !pure_loose_path(path)
        {
            return Ok(false);
        }
        let Some(pure) = &self.options.pure else {
            return Ok(true);
        };
        if pure.archives.is_empty() {
            return Ok(true);
        }
        match source {
            MountedSource::Archive { mount, .. } => Ok(pure.archives.contains(&archive_digest_or_compute(mount)?)),
            MountedSource::Loose { .. } => Ok(pure_loose_path(path)),
        }
    }

    fn read_source(
        &self,
        source: &MountedSource,
        member_path: &str,
    ) -> Result<Option<(Vec<u8>, ResourceProvenance, ContentDigest)>, MountError> {
        if !self.allowed(source, member_path)? {
            return Ok(None);
        }
        match source {
            MountedSource::Archive { mount, archive } => {
                let entries: Vec<ArchiveEntry> = archive
                    .find_entries(member_path, ArchivePathComparison::CaseInsensitive)?
                    .into_iter()
                    .filter(|entry| !entry.is_directory())
                    .collect();
                // PACK walks directory records forward; the Q3 ZIP index replaces duplicate names.
                let entry = if mount.format == ArchiveFormat::Pak {
                    entries.first()
                } else {
                    entries.last()
                };
                let Some(entry) = entry else {
                    return Ok(None);
                };
                let crc = match entry {
                    ArchiveEntry::Zip(zip) => zip.crc32,
                    ArchiveEntry::Pak(_) => 0,
                };
                let bytes = archive.read_entry(EntryRef::Ordinal(entry.ordinal()))?;
                let digest = resource_identity_token(bytes.len() as u64, crc, 0);
                Ok(Some((
                    bytes,
                    ResourceProvenance::Archive {
                        mount: mount.clone(),
                        member_path: entry.path().to_string(),
                        member_index: entry.ordinal() as u64,
                    },
                    digest,
                )))
            }
            MountedSource::Loose { mount } => {
                let root = Path::new(&mount.root_path);
                let Some(path) = find_content_path(root, member_path, self.effective_loose_comparison())? else {
                    return Ok(None);
                };
                let metadata = std::fs::metadata(&path).map_err(|error| MountError::Io {
                    path: path.to_string_lossy().into_owned(),
                    message: error.to_string(),
                })?;
                if !metadata.is_file() {
                    return Ok(None);
                }
                let modified = metadata
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(SystemTime::UNIX_EPOCH).ok())
                    .map(|elapsed| elapsed.as_nanos() as u64)
                    .unwrap_or(0);
                let actual = path
                    .strip_prefix(root)
                    .map_err(|_| MountError::Failed(format!("Loose path escapes root: {member_path}")))?;
                let actual = actual
                    .components()
                    .map(|component| component.as_os_str().to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("/");
                let bytes = read_loose_entry(root, &actual)?;
                let digest = resource_identity_token(bytes.len() as u64, 0, modified);
                Ok(Some((
                    bytes,
                    ResourceProvenance::Loose {
                        mount: mount.clone(),
                        member_path: actual,
                    },
                    digest,
                )))
            }
        }
    }

    fn opened(
        &self,
        unresolved: UnresolvedResourceReference,
        bytes: Vec<u8>,
        opened: &RefCell<HashMap<String, ResolvedResourceReference>>,
    ) -> Result<OpenedResource, MountError> {
        let id = create_resource_id(&unresolved)?;
        let UnresolvedResourceReference {
            requested_path,
            provenance,
            digest,
            byte_length,
            resolution,
        } = unresolved;
        if let ResourceProvenance::Archive { mount, .. } = &provenance {
            self.referenced
                .borrow_mut()
                .insert(mount.identity.id.clone(), mount.clone());
        }
        let reference = ResolvedResourceReference {
            id,
            requested_path: requested_path.clone(),
            provenance,
            digest,
            byte_length,
            resolution,
        };
        opened.borrow_mut().insert(
            format!("{}:{requested_path}", provenance_identity(&reference.provenance).id),
            reference.clone(),
        );
        Ok(OpenedResource::new(reference, bytes))
    }

    fn open_in(
        &self,
        plan: &ResolvedMountPlan,
        path: &str,
        accept_mount: &dyn Fn(&ContentMount) -> bool,
        opened: &RefCell<HashMap<String, ResolvedResourceReference>>,
    ) -> Result<Option<OpenedResource>, MountError> {
        self.assert_open()?;
        let requested_path = normalize_resource_path(path)?;
        if !self.user_mounts.is_empty() {
            for (rank, id) in plan.default_order.iter().enumerate() {
                if !self.user_mounts.contains(id) {
                    continue;
                }
                let Some(source) = self.sources.get(id) else {
                    continue;
                };
                if !accept_mount(&source.content_mount()) {
                    continue;
                }
                let read = self.read_source(source, &requested_path)?;
                self.assert_open()?;
                if let Some((bytes, provenance, digest)) = read {
                    return self
                        .opened(
                            UnresolvedResourceReference {
                                requested_path,
                                provenance,
                                digest,
                                byte_length: bytes.len() as u64,
                                resolution: ResourceResolution::DefaultOrder {
                                    plan: plan.id.clone(),
                                    rank: rank as u64,
                                },
                            },
                            bytes,
                            opened,
                        )
                        .map(Some);
                }
            }
        }
        for link in &self.options.links {
            if !requested_path.starts_with(&link.source_prefix) {
                continue;
            }
            let Some(source) = self.sources.get(&link.mount) else {
                return Err(MountError::Failed(format!("Unknown link mount: {}", link.mount)));
            };
            if !accept_mount(&source.content_mount()) {
                return Ok(None);
            }
            let target_path = normalize_resource_path(&format!(
                "{}{}",
                link.target_prefix,
                &requested_path[link.source_prefix.len()..]
            ))?;
            let read = self.read_source(source, &target_path)?;
            self.assert_open()?;
            return match read {
                None => Ok(None),
                Some((bytes, provenance, digest)) => self
                    .opened(
                        UnresolvedResourceReference {
                            requested_path,
                            provenance,
                            digest,
                            byte_length: bytes.len() as u64,
                            resolution: ResourceResolution::Link {
                                plan: plan.id.clone(),
                                source_prefix: link.source_prefix.clone(),
                                target_path,
                            },
                        },
                        bytes,
                        opened,
                    )
                    .map(Some),
            };
        }
        let prefix = plan
            .prefix_orders
            .iter()
            .find(|order| requested_path.to_lowercase().starts_with(&order.prefix.to_lowercase()));
        let order = prefix.map(|order| &order.mounts).unwrap_or(&plan.default_order);
        for (rank, id) in order.iter().enumerate() {
            let Some(source) = self.sources.get(id) else {
                return Err(MountError::Failed(format!("Unknown mounted source: {id}")));
            };
            if self.user_mounts.contains(id) {
                continue;
            }
            if !accept_mount(&source.content_mount()) {
                continue;
            }
            let read = self.read_source(source, &requested_path)?;
            self.assert_open()?;
            if let Some((bytes, provenance, digest)) = read {
                let resolution = match prefix {
                    None => ResourceResolution::DefaultOrder {
                        plan: plan.id.clone(),
                        rank: rank as u64,
                    },
                    Some(prefix) => ResourceResolution::PrefixOrder {
                        plan: plan.id.clone(),
                        prefix: prefix.prefix.clone(),
                        rank: rank as u64,
                    },
                };
                return self
                    .opened(
                        UnresolvedResourceReference {
                            requested_path,
                            provenance,
                            digest,
                            byte_length: bytes.len() as u64,
                            resolution,
                        },
                        bytes,
                        opened,
                    )
                    .map(Some);
            }
        }
        Ok(None)
    }

    /// Open a resource by path, skipping mounts the filter rejects.
    pub fn open(
        &self,
        path: &str,
        accept_mount: impl Fn(&ContentMount) -> bool,
    ) -> Result<Option<OpenedResource>, MountError> {
        self.open_in(&self.plan, path, &accept_mount, &self.opened_resources)
    }

    /// Resolve a resource reference without copying its bytes twice.
    pub fn resolve(&self, path: &str) -> Result<Option<ResolvedResourceReference>, MountError> {
        Ok(self.open(path, |_| true)?.map(|found| found.reference))
    }

    /// Q3 FS_ListFilteredFiles without a filter, in selected mount and
    /// archive-directory order.
    pub fn list_files(&self, path: &str, extension: &str) -> Result<Vec<String>, MountError> {
        self.assert_open()?;
        if path.encode_utf16().count() >= 256
            || path
                .chars()
                .chain(extension.chars())
                .any(|character| character == '\0' || character as u32 > 255)
        {
            return Err(MountError::Invalid(
                "Q3 file listing exceeds source path representation".to_string(),
            ));
        }
        let directory = path.trim_end_matches(['/', '\\']);
        if !directory.is_empty() {
            normalize_resource_path(directory)?;
        }
        let prefix = self.plan.prefix_orders.iter().find(|order| {
            format!("{directory}/")
                .to_lowercase()
                .starts_with(&order.prefix.to_lowercase())
        });
        let directory_folded = directory.to_lowercase();
        let directory_chars = directory.chars().count();
        let extension_folded = extension.to_lowercase();
        let mut names: Vec<String> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        let order = prefix.map(|order| &order.mounts).unwrap_or(&self.plan.default_order);
        for id in order {
            let Some(source) = self.sources.get(id) else {
                return Err(MountError::Failed(format!("Unknown mounted source: {id}")));
            };
            match source {
                MountedSource::Archive { archive, .. } => {
                    for entry in &archive.entries {
                        let name = entry.path();
                        let depth = |value: &str| {
                            value
                                .chars()
                                .filter(|character| *character == '/' || *character == '\\')
                                .count() as i64
                        };
                        let name_chars: Vec<char> = name.chars().collect();
                        let last_separator = name_chars
                            .iter()
                            .rposition(|character| *character == '/' || *character == '\\')
                            .map(|index| index as i64)
                            .unwrap_or(-1);
                        if !self.allowed(source, name)?
                            || depth(name) - depth(path) > 2
                            || directory_chars as i64 > last_separator.max(0)
                            || !name.to_lowercase().starts_with(&directory_folded)
                            || !name.to_lowercase().ends_with(&extension_folded)
                        {
                            continue;
                        }
                        let tail: String = if directory.is_empty() {
                            name.to_string()
                        } else {
                            name.chars().skip(directory_chars + 1).collect()
                        };
                        if names.len() < 4095 && seen.insert(tail.to_lowercase()) {
                            names.push(tail);
                        }
                    }
                }
                MountedSource::Loose { mount } => {
                    if !self.user_mounts.contains(&mount.identity.id)
                        && (self.options.q3_restriction == Some(Q3Restriction::Demo)
                            || self.options.pure.as_ref().is_some_and(|pure| !pure.archives.is_empty()))
                    {
                        continue;
                    }
                    let root = Path::new(&mount.root_path);
                    let location = if directory.is_empty() {
                        PathBuf::from(&mount.root_path)
                    } else {
                        match find_content_path(root, directory, self.effective_loose_comparison())? {
                            Some(location) => location,
                            None => continue,
                        }
                    };
                    let listing: Vec<std::fs::DirEntry> = match std::fs::read_dir(&location) {
                        Ok(entries) => match entries.collect::<Result<Vec<_>, _>>() {
                            Ok(entries) => entries,
                            Err(error) if is_missing_file(&error) => continue,
                            Err(error) => {
                                return Err(MountError::Io {
                                    path: location.to_string_lossy().into_owned(),
                                    message: error.to_string(),
                                });
                            }
                        },
                        Err(error) if is_missing_file(&error) => continue,
                        Err(error) => {
                            return Err(MountError::Io {
                                path: location.to_string_lossy().into_owned(),
                                message: error.to_string(),
                            });
                        }
                    };
                    self.assert_open()?;
                    for entry in listing {
                        let file_type = entry.file_type().map_err(|error| MountError::Io {
                            path: location.to_string_lossy().into_owned(),
                            message: error.to_string(),
                        })?;
                        if file_type.is_symlink() {
                            continue;
                        }
                        if (extension == "/") != file_type.is_dir() {
                            continue;
                        }
                        let name = entry.file_name().to_string_lossy().into_owned();
                        if extension != "/" && !name.to_lowercase().ends_with(&extension_folded) {
                            continue;
                        }
                        if names.len() < 4095 && seen.insert(name.to_lowercase()) {
                            names.push(name);
                        }
                    }
                }
            }
        }
        self.assert_open()?;
        Ok(names)
    }

    /// Read a resource by path or previously resolved reference.
    pub fn read(&self, resource: ResourceRef<'_>) -> Result<Vec<u8>, MountError> {
        self.read_in(resource, &self.opened_resources)
    }

    fn read_in(
        &self,
        resource: ResourceRef<'_>,
        opened: &RefCell<HashMap<String, ResolvedResourceReference>>,
    ) -> Result<Vec<u8>, MountError> {
        self.assert_open()?;
        match resource {
            ResourceRef::Path(path) => match self.open_in(&self.plan, path, &|_| true, opened)? {
                Some(found) => Ok(found.bytes),
                None => Err(MountError::Failed(format!("Resource not found: {path}"))),
            },
            ResourceRef::Resolved(resource) => {
                let identity = provenance_identity(&resource.provenance);
                let stale = || MountError::Failed(format!("Stale resource mount: {}", resource.id));
                let Some(source) = self.sources.get(&identity.id) else {
                    return Err(stale());
                };
                let current = source_identity(source);
                if current.content != identity.content || current.generation != identity.generation {
                    return Err(stale());
                }
                let bytes = match &resource.provenance {
                    ResourceProvenance::Archive {
                        mount,
                        member_path,
                        member_index,
                    } => {
                        let MountedSource::Archive {
                            mount: current,
                            archive,
                        } = source
                        else {
                            return Err(MountError::Failed(format!("Archive identity changed: {}", resource.id)));
                        };
                        if current.archive_path != mount.archive_path {
                            return Err(MountError::Failed(format!("Archive identity changed: {}", resource.id)));
                        }
                        let member_changed =
                            || MountError::Failed(format!("Archive member identity changed: {}", resource.id));
                        let Some(entry) = archive
                            .entries
                            .iter()
                            .find(|entry| entry.ordinal() as u64 == *member_index)
                        else {
                            return Err(member_changed());
                        };
                        if entry.path() != member_path {
                            return Err(member_changed());
                        }
                        if !self.allowed(source, entry.path())? {
                            return Err(MountError::Failed(format!(
                                "Resource excluded by pure policy: {}",
                                resource.requested_path
                            )));
                        }
                        let bytes = archive.read_entry(EntryRef::Ordinal(entry.ordinal()))?;
                        self.referenced
                            .borrow_mut()
                            .insert(current.identity.id.clone(), current.clone());
                        bytes
                    }
                    ResourceProvenance::Loose { mount, member_path } => {
                        let MountedSource::Loose { mount: current } = source else {
                            return Err(MountError::Failed(format!(
                                "Loose root identity changed: {}",
                                resource.id
                            )));
                        };
                        if current.root_path != mount.root_path {
                            return Err(MountError::Failed(format!(
                                "Loose root identity changed: {}",
                                resource.id
                            )));
                        }
                        match self.read_source(source, member_path)? {
                            Some((bytes, _, _)) => bytes,
                            None => {
                                return Err(MountError::Failed(format!(
                                    "Resource is no longer available: {}",
                                    resource.requested_path
                                )));
                            }
                        }
                    }
                };
                self.assert_open()?;
                if bytes.len() as u64 != resource.byte_length {
                    return Err(MountError::Failed(format!(
                        "Resource bytes changed since resolution: {}",
                        resource.requested_path
                    )));
                }
                opened
                    .borrow_mut()
                    .insert(format!("{}:{}", identity.id, resource.requested_path), resource.clone());
                Ok(bytes)
            }
        }
    }

    /// Close the plan, releasing owned archives. Borrowed sources stay open.
    pub fn close(&self) {
        if !self.live.get() {
            return;
        }
        self.live.set(false);
        self.opened_resources.borrow_mut().clear();
        if self.owns_archives {
            for source in self.sources.values() {
                if let MountedSource::Archive { archive, .. } = source {
                    archive.close();
                }
            }
        }
    }
}

impl Drop for MountedContent {
    fn drop(&mut self) {
        self.close();
    }
}

/// Order subset for [`MountedContent::borrow_ordered_reader`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderedPlan {
    /// Plan identity.
    pub id: MountPlanId,
    /// Default order.
    pub default_order: Vec<MountId>,
    /// Prefix orders.
    pub prefix_orders: Vec<PrefixMountOrder>,
}

/// Order-only view over verified sources (`MountedContentReader`).
#[derive(Debug)]
pub struct OrderedReader<'a> {
    /// Resolved order-only plan.
    pub plan: ResolvedMountPlan,
    owner: &'a MountedContent,
    opened: RefCell<HashMap<String, ResolvedResourceReference>>,
}

impl OrderedReader<'_> {
    /// Open a resource through the borrowed order.
    pub fn open(
        &self,
        path: &str,
        accept_mount: impl Fn(&ContentMount) -> bool,
    ) -> Result<Option<OpenedResource>, MountError> {
        self.owner.open_in(&self.plan, path, &accept_mount, &self.opened)
    }

    /// Resolve a resource reference through the borrowed order.
    pub fn resolve(&self, path: &str) -> Result<Option<ResolvedResourceReference>, MountError> {
        Ok(self.open(path, |_| true)?.map(|found| found.reference))
    }

    /// Read a resource through the borrowed order.
    pub fn read(&self, resource: ResourceRef<'_>) -> Result<Vec<u8>, MountError> {
        match resource {
            ResourceRef::Path(path) => match self.open(path, |_| true)? {
                Some(found) => Ok(found.bytes),
                None => Err(MountError::Failed(format!("Resource not found: {path}"))),
            },
            ResourceRef::Resolved(_) => self.owner.read_in(resource, &self.opened),
        }
    }
}

fn close_sources(sources: &[MountedSource]) {
    for source in sources {
        if let MountedSource::Archive { archive, .. } = source {
            archive.close();
        }
    }
}

/// Open and verify a mount plan (`openMountPlan`).
pub fn open_mount_plan(plan: &ResolvedMountPlan, options: OpenMountOptions) -> Result<MountedContent, MountError> {
    let resolved = resolve_mount_plan(plan, &options)?;
    let mut sources: Vec<MountedSource> = Vec::new();
    let built = (|| -> Result<(), MountError> {
        for mount in &plan.mounts {
            match mount {
                ContentMount::Loose(mount) => sources.push(MountedSource::Loose { mount: mount.clone() }),
                ContentMount::Archive(mount) => {
                    let archive = Rc::new(open_archive(Path::new(&mount.archive_path), Some(mount.format))?);
                    archive.verify_storage()?;
                    if options.q3_restriction == Some(Q3Restriction::Demo) {
                        verify_demo_format(&archive, &mount.archive_path)?;
                        let checksum = q3_archive_checksums(&archive, 0)?.checksum;
                        if checksum != Q3_DEMO_PAK0_CHECKSUM {
                            return Err(MountError::Failed(format!("Corrupted demo pak0.pk3: {checksum}")));
                        }
                    }
                    sources.push(MountedSource::Archive {
                        mount: mount.clone(),
                        archive,
                    });
                }
            }
        }
        Ok(())
    })();
    if let Err(error) = built {
        close_sources(&sources);
        return Err(error);
    }
    Ok(MountedContent::assemble(resolved, sources, options, Vec::new(), true))
}

/// Whether a resource may be downloaded (`canDownloadResource`).
#[must_use]
pub fn can_download_resource(resource: &ResolvedResourceReference) -> bool {
    if !matches!(&resource.provenance, ResourceProvenance::Archive { .. }) {
        return true;
    }
    let identity = provenance_identity(&resource.provenance);
    !(identity.content.as_str().starts_with("q2:") && resource.requested_path.to_lowercase().starts_with("maps/"))
}

/// Plan opener (donor `MountPlanOpener`).
pub trait MountPlanOpener {
    /// Open a verified plan.
    fn open_plan(&self, plan: &ResolvedMountPlan, options: OpenMountOptions) -> Result<MountedContent, MountError>;
}

/// Shared verified-descriptor scope (`MountPreparationScope`).
///
/// A single preparation operation owns verified descriptors shared by
/// product views. Dropping the scope closes it.
#[derive(Debug)]
pub struct MountPreparationScope {
    archives: RefCell<HashMap<String, Rc<OpenArchive>>>,
    live: Rc<Cell<bool>>,
}

impl MountPreparationScope {
    /// Create an empty scope.
    #[must_use]
    pub fn new() -> Self {
        Self {
            archives: RefCell::new(HashMap::new()),
            live: Rc::new(Cell::new(true)),
        }
    }

    /// Fail once the scope is closed.
    pub fn assert_open(&self) -> Result<(), MountError> {
        if !self.live.get() {
            return Err(MountError::Closed("Mount preparation scope is closed".to_string()));
        }
        Ok(())
    }

    fn archive(&self, mount: &ArchiveMount) -> Result<Rc<OpenArchive>, MountError> {
        self.assert_open()?;
        let key = format!(
            "{}\0{}\0{}\0{:?}",
            mount.archive_path, mount.identity.id, mount.identity.generation, mount.format
        );
        if let Some(archive) = self.archives.borrow().get(&key) {
            return Ok(archive.clone());
        }
        let archive = self.open_verified(mount)?;
        self.archives.borrow_mut().insert(key, archive.clone());
        Ok(archive)
    }

    fn open_verified(&self, mount: &ArchiveMount) -> Result<Rc<OpenArchive>, MountError> {
        let archive = Rc::new(open_archive(Path::new(&mount.archive_path), Some(mount.format))?);
        if let Err(error) = archive.verify_storage() {
            archive.close();
            return Err(error.into());
        }
        if let Err(error) = self.assert_open() {
            archive.close();
            return Err(error);
        }
        Ok(archive)
    }

    /// Open a plan over the scope's verified descriptors.
    pub fn open(&self, plan: &ResolvedMountPlan, options: OpenMountOptions) -> Result<MountedContent, MountError> {
        self.assert_open()?;
        let resolved = resolve_mount_plan(plan, &options)?;
        let mut sources = Vec::new();
        for mount in &plan.mounts {
            match mount {
                ContentMount::Loose(mount) => sources.push(MountedSource::Loose { mount: mount.clone() }),
                ContentMount::Archive(mount) => {
                    let archive = self.archive(mount)?;
                    self.assert_open()?;
                    if options.q3_restriction == Some(Q3Restriction::Demo) {
                        verify_demo_format(&archive, &mount.archive_path)?;
                        if q3_archive_checksums(&archive, 0)?.checksum != Q3_DEMO_PAK0_CHECKSUM {
                            return Err(MountError::Failed(format!(
                                "Corrupted demo pak0.pk3: {}",
                                mount.archive_path
                            )));
                        }
                    }
                    sources.push(MountedSource::Archive {
                        mount: mount.clone(),
                        archive,
                    });
                }
            }
        }
        self.assert_open()?;
        Ok(MountedContent::assemble(
            resolved,
            sources,
            options,
            vec![self.live.clone()],
            false,
        ))
    }

    /// Close the scope, releasing every verified descriptor.
    pub fn close(&self) {
        self.live.set(false);
        for archive in self.archives.borrow().values() {
            archive.close();
        }
        self.archives.borrow_mut().clear();
    }
}

impl Default for MountPreparationScope {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for MountPreparationScope {
    fn drop(&mut self) {
        self.close();
    }
}

impl MountPlanOpener for MountPreparationScope {
    fn open_plan(&self, plan: &ResolvedMountPlan, options: OpenMountOptions) -> Result<MountedContent, MountError> {
        self.open(plan, options)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{create_content_digest, ContentId, LazyArchiveDigest};

    fn pak_bytes(members: &[(&str, &[u8])]) -> Vec<u8> {
        let mut bytes = b"PACK".to_vec();
        let dir_offset = 12 + members.iter().map(|(_, data)| data.len()).sum::<usize>();
        bytes.extend_from_slice(&(dir_offset as i32).to_le_bytes());
        bytes.extend_from_slice(&(members.len() as i32 * 64).to_le_bytes());
        for (_, data) in members {
            bytes.extend_from_slice(data);
        }
        let mut data_offset = 12;
        for (name, data) in members {
            let mut raw = [0u8; 56];
            raw[..name.len()].copy_from_slice(name.as_bytes());
            bytes.extend_from_slice(&raw);
            bytes.extend_from_slice(&(data_offset as i32).to_le_bytes());
            bytes.extend_from_slice(&(data.len() as i32).to_le_bytes());
            data_offset += data.len();
        }
        bytes
    }

    fn mount_id(name: &str) -> MountId {
        MountId(format!("mount:test:{name}"))
    }

    fn content_id() -> ContentId {
        ContentId("q1:test:base:1".to_string())
    }

    fn identity(name: &str) -> MountIdentity {
        MountIdentity {
            id: mount_id(name),
            content: content_id(),
            generation: 1,
        }
    }

    fn archive_mount(name: &str, path: &Path, _bytes: &[u8]) -> ArchiveMount {
        ArchiveMount {
            identity: identity(name),
            format: ArchiveFormat::Pak,
            archive_path: path.to_string_lossy().into_owned(),
            archive_digest: LazyArchiveDigest::uncomputed(),
        }
    }

    fn loose_mount(name: &str, root: &Path) -> LooseMount {
        LooseMount {
            identity: identity(name),
            root_path: root.to_string_lossy().into_owned(),
        }
    }

    fn plan(mounts: Vec<ContentMount>, order: Vec<MountId>) -> ResolvedMountPlan {
        ResolvedMountPlan {
            id: MountPlanId("mount-plan:test:1".to_string()),
            mounts,
            default_order: order,
            prefix_orders: Vec::new(),
        }
    }

    fn scratch_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("qa-mounts-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn digests_tokens_and_missing_files() {
        assert_eq!(
            digest_bytes(b"abc").as_str(),
            "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        // Identity tokens are deterministic, shape-valid digests minted
        // without hashing.
        let token = resource_identity_token(9, 0x12345678, 0);
        assert_eq!(
            token.as_str(),
            "sha256:1234567800000000000000090000000000000000000000000000000000000000"
        );
        assert!(is_content_digest(token.as_str()));
        assert_eq!(resource_identity_token(9, 0x12345678, 0), token);
        assert_ne!(resource_identity_token(10, 0x12345678, 0), token);
        assert_ne!(resource_identity_token(9, 0x12345678, 1), token);
        assert!(is_missing_file(&std::io::Error::new(std::io::ErrorKind::NotFound, "x")));
        assert!(is_missing_file(&std::io::Error::new(
            std::io::ErrorKind::NotADirectory,
            "x"
        )));
        assert!(!is_missing_file(&std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "x"
        )));
    }

    #[test]
    fn loose_plan_opens_resolves_and_reads() {
        let root = scratch_dir("loose");
        std::fs::create_dir_all(root.join("maps")).unwrap();
        std::fs::write(root.join("maps").join("a.bsp"), b"loose-map").unwrap();
        let mount = loose_mount("loose", &root);
        let content = open_mount_plan(
            &plan(
                vec![ContentMount::Loose(mount.clone())],
                vec![mount.identity.id.clone()],
            ),
            OpenMountOptions::default(),
        )
        .unwrap();
        // Case-insensitive read by default.
        let found = content.open("MAPS/A.BSP", |_| true).unwrap().unwrap();
        assert_eq!(found.bytes, b"loose-map".to_vec());
        assert_eq!(found.reference.requested_path, "MAPS/A.BSP");
        assert_eq!(
            content.resolve("maps/a.bsp").unwrap().unwrap().digest,
            found.reference.digest
        );
        assert_eq!(
            content.read(ResourceRef::Path("maps/a.bsp")).unwrap(),
            b"loose-map".to_vec()
        );
        assert_eq!(
            content.read(ResourceRef::Resolved(&found.reference)).unwrap(),
            b"loose-map".to_vec()
        );
        // One record per (mount, requested path) spelling.
        assert_eq!(content.opened_resources().len(), 2);
        assert!(content.referenced_archives().is_empty());
        assert!(content.open("maps/missing.bsp", |_| true).unwrap().is_none());
        assert!(content.read(ResourceRef::Path("maps/missing.bsp")).is_err());
        // Mount filter rejects everything.
        assert!(content.open("maps/a.bsp", |_| false).unwrap().is_none());
        content.close();
        assert!(content.open("maps/a.bsp", |_| true).is_err());
        assert!(content.resolve("maps/a.bsp").is_err());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn archive_plan_prefers_first_mount_and_tracks_references() {
        let root = scratch_dir("archive");
        let first = pak_bytes(&[("maps/shared.bsp", b"first"), ("maps/only-first.bsp", b"1")]);
        let second = pak_bytes(&[("maps/shared.bsp", b"second")]);
        std::fs::write(root.join("first.pak"), &first).unwrap();
        std::fs::write(root.join("second.pak"), &second).unwrap();
        let a = archive_mount("a", &root.join("first.pak"), &first);
        let b = archive_mount("b", &root.join("second.pak"), &second);
        let content = open_mount_plan(
            &plan(
                vec![ContentMount::Archive(a.clone()), ContentMount::Archive(b.clone())],
                vec![a.identity.id.clone(), b.identity.id.clone()],
            ),
            OpenMountOptions::default(),
        )
        .unwrap();
        let found = content.open("maps/shared.bsp", |_| true).unwrap().unwrap();
        assert_eq!(found.bytes, b"first".to_vec());
        assert!(matches!(
            found.reference.resolution,
            ResourceResolution::DefaultOrder { rank: 0, .. }
        ));
        let only = content.open("maps/only-first.bsp", |_| true).unwrap().unwrap();
        assert_eq!(only.bytes, b"1".to_vec());
        assert_eq!(content.referenced_archives().len(), 1);
        // Resource references carry identity tokens, not content hashes.
        assert_eq!(found.reference.digest, resource_identity_token(5, 0, 0));
        // Tampered lengths fail re-reads; tampered digests are inert
        // tokens, so only provenance and length are checked per read.
        let mut stale = found.reference.clone();
        stale.byte_length = 999;
        let error = content.read(ResourceRef::Resolved(&stale)).unwrap_err();
        assert!(error.to_string().contains("bytes changed since resolution"), "{error}");
        let mut stale_mount = found.reference.clone();
        if let ResourceProvenance::Archive { mount, .. } = &mut stale_mount.provenance {
            mount.identity.generation = 2;
        }
        let error = content.read(ResourceRef::Resolved(&stale_mount)).unwrap_err();
        assert!(error.to_string().contains("Stale resource mount"), "{error}");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn opening_resources_performs_zero_hashes() {
        let root = scratch_dir("zero-hash");
        let bytes = pak_bytes(&[
            ("maps/a.bsp", b"aaa"),
            ("maps/b.bsp", b"bbbb"),
            ("sound/shot.wav", b"ccccc"),
        ]);
        std::fs::write(root.join("mod.pak"), &bytes).unwrap();
        std::fs::create_dir_all(root.join("loose")).unwrap();
        std::fs::write(root.join("loose").join("extra.cfg"), b"loose-bytes").unwrap();
        let mount = archive_mount("a", &root.join("mod.pak"), &bytes);
        let loose = loose_mount("loose", &root.join("loose"));
        let content = open_mount_plan(
            &plan(
                vec![ContentMount::Archive(mount), ContentMount::Loose(loose)],
                vec![mount_id("a"), mount_id("loose")],
            ),
            OpenMountOptions::default(),
        )
        .unwrap();
        // Open every resource twice and re-read each resolved reference.
        let mut opened = Vec::new();
        for path in ["maps/a.bsp", "maps/b.bsp", "sound/shot.wav", "extra.cfg"] {
            let first = content.open(path, |_| true).unwrap().unwrap();
            let second = content.open(path, |_| true).unwrap().unwrap();
            assert_eq!(first.reference.digest, second.reference.digest);
            assert_eq!(
                content.read(ResourceRef::Resolved(&first.reference)).unwrap(),
                first.bytes
            );
            opened.push(first);
        }
        // No lazy digest was forced anywhere: uncomputed archive cells,
        // uncomputed content cells, and deterministic identity tokens
        // prove no hashing happened on the open path.
        for mount in &content.plan.mounts {
            if let ContentMount::Archive(mount) = mount {
                assert!(!mount.archive_digest.is_computed());
            }
        }
        for resource in &opened {
            assert!(!resource.is_content_digest_computed());
        }
        assert_eq!(opened[0].reference.digest, resource_identity_token(3, 0, 0));
        assert_eq!(opened[1].reference.digest, resource_identity_token(4, 0, 0));
        assert_eq!(opened[2].reference.digest, resource_identity_token(5, 0, 0));
        // Forcing one resource hashes its bytes exactly once per instance.
        assert_eq!(opened[3].content_digest(), &digest_bytes(b"loose-bytes"));
        assert!(opened[3].is_content_digest_computed());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn mount_boundary_detects_changed_archives() {
        let root = scratch_dir("verify");
        let bytes = pak_bytes(&[("a.txt", b"a")]);
        std::fs::write(root.join("mod.pak"), &bytes).unwrap();
        let mount = archive_mount("a", &root.join("mod.pak"), &bytes);
        let content = open_mount_plan(
            &plan(vec![ContentMount::Archive(mount)], vec![mount_id("a")]),
            OpenMountOptions::default(),
        )
        .unwrap();
        // Mounting never hashes: the lazy cell stays uncomputed.
        assert!(!content.plan.mounts.iter().any(|mount| match mount {
            ContentMount::Archive(mount) => mount.archive_digest.is_computed(),
            ContentMount::Loose(_) => false,
        }));
        assert!(content.verify_archives().is_ok());
        // Replacing the archive trips the boundary check, not reads.
        std::fs::write(root.join("mod.pak"), pak_bytes(&[("a.txt", b"bb")])).unwrap();
        let error = content.verify_archives().unwrap_err();
        assert!(
            error.to_string().contains("source changed after it was opened"),
            "{error}"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn plan_validation_rejects_bad_orders_links_and_pure() {
        let root = scratch_dir("plan");
        let mount = loose_mount("loose", &root);
        let id = mount.identity.id.clone();
        // Unknown mount in order.
        let error = open_mount_plan(
            &plan(vec![ContentMount::Loose(mount.clone())], vec![mount_id("nope")]),
            OpenMountOptions::default(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("unknown mount"), "{error}");
        // Repeated mount in order.
        let error = open_mount_plan(
            &plan(vec![ContentMount::Loose(mount.clone())], vec![id.clone(), id.clone()]),
            OpenMountOptions::default(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("repeats mount"), "{error}");
        // Missing mount in order.
        let error = open_mount_plan(
            &plan(vec![ContentMount::Loose(mount.clone())], vec![]),
            OpenMountOptions::default(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("every fallback mount"), "{error}");
        // Repeated mount identity.
        let error = open_mount_plan(
            &plan(
                vec![ContentMount::Loose(mount.clone()), ContentMount::Loose(mount.clone())],
                vec![id.clone()],
            ),
            OpenMountOptions::default(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("Repeated mount identity"), "{error}");
        // Link to a missing mount.
        let error = open_mount_plan(
            &plan(vec![ContentMount::Loose(mount.clone())], vec![id.clone()]),
            OpenMountOptions {
                links: vec![ResourceLink {
                    source_prefix: "v/".to_string(),
                    mount: mount_id("nope"),
                    target_prefix: String::new(),
                }],
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("Link must target"), "{error}");
        // Required pure archive missing.
        let error = open_mount_plan(
            &plan(vec![ContentMount::Loose(mount.clone())], vec![id.clone()]),
            OpenMountOptions {
                pure: Some(PureMountPolicy {
                    archives: vec![digest_bytes(b"x")],
                }),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(
            error.to_string().contains("Required pure archive is missing"),
            "{error}"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn pure_policy_reorders_and_restricts() {
        let root = scratch_dir("pure");
        let first = pak_bytes(&[("maps/shared.bsp", b"first")]);
        let second = pak_bytes(&[("maps/shared.bsp", b"second")]);
        std::fs::write(root.join("first.pak"), &first).unwrap();
        std::fs::write(root.join("second.pak"), &second).unwrap();
        let a = archive_mount("a", &root.join("first.pak"), &first);
        let b = archive_mount("b", &root.join("second.pak"), &second);
        let forced = archive_digest_or_compute(&b).unwrap();
        assert_eq!(forced, digest_bytes(&second));
        let loose_root = root.join("loose");
        std::fs::create_dir_all(loose_root.join("maps")).unwrap();
        std::fs::write(loose_root.join("maps").join("loose.bsp"), b"loose").unwrap();
        std::fs::write(loose_root.join("autoexec.cfg"), b"cfg").unwrap();
        let loose = loose_mount("loose", &loose_root);
        let plan = plan(
            vec![
                ContentMount::Archive(a.clone()),
                ContentMount::Archive(b.clone()),
                ContentMount::Loose(loose.clone()),
            ],
            vec![a.identity.id.clone(), b.identity.id.clone(), loose.identity.id.clone()],
        );
        let content = open_mount_plan(
            &plan,
            OpenMountOptions {
                pure: Some(PureMountPolicy { archives: vec![forced] }),
                ..Default::default()
            },
        )
        .unwrap();
        // Pure archive moves first even though it was second.
        assert_eq!(content.plan.default_order[0], b.identity.id);
        let found = content.open("maps/shared.bsp", |_| true).unwrap().unwrap();
        assert_eq!(found.bytes, b"second".to_vec());
        // Non-config loose files are excluded under pure; configs still resolve.
        assert!(content.open("maps/loose.bsp", |_| true).unwrap().is_none());
        assert!(content.open("autoexec.cfg", |_| true).unwrap().is_some());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn links_rewrite_into_loose_mounts() {
        let root = scratch_dir("links");
        std::fs::create_dir_all(root.join("real")).unwrap();
        std::fs::write(root.join("real").join("file.txt"), b"linked").unwrap();
        let mount = loose_mount("loose", &root);
        let content = open_mount_plan(
            &plan(
                vec![ContentMount::Loose(mount.clone())],
                vec![mount.identity.id.clone()],
            ),
            OpenMountOptions {
                links: vec![ResourceLink {
                    source_prefix: "virtual/".to_string(),
                    mount: mount.identity.id.clone(),
                    target_prefix: "real/".to_string(),
                }],
                ..Default::default()
            },
        )
        .unwrap();
        let found = content.open("virtual/file.txt", |_| true).unwrap().unwrap();
        assert_eq!(found.bytes, b"linked".to_vec());
        assert!(matches!(found.reference.resolution, ResourceResolution::Link { .. }));
        assert!(content.open("virtual/missing.txt", |_| true).unwrap().is_none());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn borrows_share_sources_and_close_locally() {
        let root = scratch_dir("borrow");
        let bytes = pak_bytes(&[("a.txt", b"a")]);
        std::fs::write(root.join("mod.pak"), &bytes).unwrap();
        let mount = archive_mount("a", &root.join("mod.pak"), &bytes);
        let plan = plan(vec![ContentMount::Archive(mount)], vec![mount_id("a")]);
        let content = open_mount_plan(&plan, OpenMountOptions::default()).unwrap();
        let borrowed = content
            .borrow_mount_plan(&plan, OpenMountOptions::default())
            .unwrap()
            .unwrap();
        assert_eq!(borrowed.read(ResourceRef::Path("a.txt")).unwrap(), b"a".to_vec());
        // Mismatched options refuse the borrow.
        let refused = content
            .borrow_mount_plan(
                &plan,
                OpenMountOptions {
                    loose_comparison: Some(PathComparison::CaseInsensitive),
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(refused.is_none());
        // Closing the borrow leaves the owner usable.
        borrowed.close();
        assert!(borrowed.read(ResourceRef::Path("a.txt")).is_err());
        assert_eq!(content.read(ResourceRef::Path("a.txt")).unwrap(), b"a".to_vec());
        // Closing the owner fails outstanding borrows.
        let borrowed = content
            .borrow_mount_plan(&plan, OpenMountOptions::default())
            .unwrap()
            .unwrap();
        content.close();
        assert!(borrowed.read(ResourceRef::Path("a.txt")).is_err());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn user_overlay_wins_and_reader_reorders() {
        let root = scratch_dir("overlay");
        let bytes = pak_bytes(&[("a.txt", b"pak")]);
        std::fs::write(root.join("mod.pak"), &bytes).unwrap();
        let mount = archive_mount("a", &root.join("mod.pak"), &bytes);
        let plan = plan(vec![ContentMount::Archive(mount)], vec![mount_id("a")]);
        let content = open_mount_plan(&plan, OpenMountOptions::default()).unwrap();
        let overlay_root = root.join("overlay");
        std::fs::create_dir_all(&overlay_root).unwrap();
        std::fs::write(overlay_root.join("a.txt"), b"user").unwrap();
        let overlay = LooseMount {
            identity: MountIdentity {
                id: mount_id("user"),
                content: content_id(),
                generation: 7,
            },
            root_path: overlay_root.to_string_lossy().into_owned(),
        };
        let scoped = content
            .borrow_with_loose_mount(overlay, MountPlanId("mount-plan:test:user".to_string()))
            .unwrap();
        assert_eq!(scoped.read(ResourceRef::Path("a.txt")).unwrap(), b"user".to_vec());
        assert_eq!(content.read(ResourceRef::Path("a.txt")).unwrap(), b"pak".to_vec());
        let reader = scoped
            .borrow_ordered_reader(&OrderedPlan {
                id: MountPlanId("mount-plan:test:order".to_string()),
                default_order: vec![mount_id("user"), mount_id("a")],
                prefix_orders: Vec::new(),
            })
            .unwrap();
        assert_eq!(reader.read(ResourceRef::Path("a.txt")).unwrap(), b"user".to_vec());
        assert!(reader.resolve("a.txt").unwrap().is_some());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn list_files_covers_archives_and_loose() {
        let root = scratch_dir("list");
        let bytes = pak_bytes(&[("maps/a.bsp", b"a"), ("maps/b.bsp", b"b"), ("maps/a.txt", b"t")]);
        std::fs::write(root.join("mod.pak"), &bytes).unwrap();
        let mount = archive_mount("a", &root.join("mod.pak"), &bytes);
        let loose_root = root.join("loose");
        std::fs::create_dir_all(loose_root.join("maps")).unwrap();
        std::fs::write(loose_root.join("maps").join("c.bsp"), b"c").unwrap();
        std::fs::create_dir_all(loose_root.join("maps").join("sub")).unwrap();
        let loose = loose_mount("loose", &loose_root);
        let content = open_mount_plan(
            &plan(
                vec![ContentMount::Archive(mount.clone()), ContentMount::Loose(loose)],
                vec![mount.identity.id.clone(), mount_id("loose")],
            ),
            OpenMountOptions::default(),
        )
        .unwrap();
        let mut files = content.list_files("maps", ".bsp").unwrap();
        files.sort();
        assert_eq!(
            files,
            vec!["a.bsp".to_string(), "b.bsp".to_string(), "c.bsp".to_string()]
        );
        assert_eq!(content.list_files("maps", "/").unwrap(), vec!["sub".to_string()]);
        assert!(content.list_files("maps", ".bsp").unwrap().len() <= 4095);
        assert!(content.list_files(&"p".repeat(256), ".bsp").is_err());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn preparation_scope_shares_verified_archives() {
        let root = scratch_dir("scope");
        let bytes = pak_bytes(&[("a.txt", b"a")]);
        std::fs::write(root.join("mod.pak"), &bytes).unwrap();
        let mount = archive_mount("a", &root.join("mod.pak"), &bytes);
        let plan = plan(vec![ContentMount::Archive(mount)], vec![mount_id("a")]);
        let scope = MountPreparationScope::new();
        let first = scope.open(&plan, OpenMountOptions::default()).unwrap();
        let second: MountedContent = scope.open_plan(&plan, OpenMountOptions::default()).unwrap();
        assert_eq!(first.read(ResourceRef::Path("a.txt")).unwrap(), b"a".to_vec());
        assert_eq!(second.read(ResourceRef::Path("a.txt")).unwrap(), b"a".to_vec());
        scope.close();
        assert!(first.read(ResourceRef::Path("a.txt")).is_err());
        assert!(scope.open(&plan, OpenMountOptions::default()).is_err());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn q3_checksums_reject_pak_and_key_feed() {
        use crate::archive::decode_archive;
        let pak = pak_bytes(&[("a.txt", b"a")]);
        let archive = decode_archive(&pak, None, "x.pak").unwrap();
        assert!(q3_archive_checksums(&archive, 0).is_err());
    }

    #[test]
    fn downloads_exclude_q2_maps() {
        let digest = digest_bytes(b"x");
        let resource = |content: &str, requested: &str| ResolvedResourceReference {
            id: crate::contract::ResourceId("resource:test".to_string()),
            requested_path: requested.to_string(),
            provenance: ResourceProvenance::Archive {
                mount: ArchiveMount {
                    identity: MountIdentity {
                        id: mount_id("a"),
                        content: ContentId(content.to_string()),
                        generation: 1,
                    },
                    format: ArchiveFormat::Pak,
                    archive_path: "mod.pak".to_string(),
                    archive_digest: LazyArchiveDigest::computed(digest.clone()),
                },
                member_path: requested.to_string(),
                member_index: 0,
            },
            digest: digest.clone(),
            byte_length: 1,
            resolution: ResourceResolution::DefaultOrder {
                plan: MountPlanId("mount-plan:test:1".to_string()),
                rank: 0,
            },
        };
        assert!(!can_download_resource(&resource("q2:base:pak:0", "maps/q2.bsp")));
        assert!(can_download_resource(&resource("q2:base:pak:0", "sound/shot.wav")));
        assert!(can_download_resource(&resource("q1:base:pak:0", "maps/q1.bsp")));
        let loose = ResolvedResourceReference {
            provenance: ResourceProvenance::Loose {
                mount: loose_mount("loose", Path::new("/tmp")),
                member_path: "maps/q2.bsp".to_string(),
            },
            ..resource("q2:base:pak:0", "maps/q2.bsp")
        };
        assert!(can_download_resource(&loose));
        let _ = create_content_digest(&"ab".repeat(32)).unwrap();
    }
}
