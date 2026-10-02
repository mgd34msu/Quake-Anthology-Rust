//! Quake II application downloads.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/network/q2-downloads.ts`
//! (`createQ2ApplicationDownloads`, `Q2PeerDownload`, `Q2DownloadReceiver`).
//! The donor is asynchronous (promises, `AbortController`, an async HTTP
//! queue, an async resource generator); this port resolves every step
//! inline: mounted opens are already synchronous, the HTTP queue pumps to
//! completion per call, and the resource walk is an explicit state machine
//! ([`Q2ResourcePaths`]) preserving the donor's lazy error timing. Q2
//! `sv_user.c` download semantics, permission checks, validation patterns,
//! and the 1024-byte sender block size are unchanged.
//!
//! `RemoteContentMounts` (donor `../content.ts`) is out of scope; the
//! content lane owns its canonical port, so this module shims the exact
//! surface the receiver reads ([`RemoteContentRoots`] plus a borrowed
//! [`MountedContent`](qa_content::mounts::MountedContent)). The donor's
//! global fetch becomes an explicit
//! [`HttpClient`](qa_net::services::http_downloads::HttpClient) factory.
//! The queue's `resolved`/`refreshPackage` callbacks cannot borrow the
//! receiver (`'static` bounds), so this port pre-checks resolution at
//! enqueue time and refreshes packages explicitly after they settle, with
//! identical observable behavior.

use std::cell::Cell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use qa_client::materials::sky::SKY_FACE_SUFFIXES;
use qa_content::archive::open_archive;
use qa_content::bsp2::read_q2_bsp;
use qa_content::catalog::{remote_content_selection, CatalogError, RemoteContentBase, RemoteContentSelection};
use qa_content::contract::ArchiveFormat;
use qa_content::md2::parse_md2;
use qa_content::mounts::{can_download_resource, MountError, MountedContent};
use qa_content::spr::parse_sp2;
use qa_core::binary::BinaryError;
use qa_core::cvar::{q2_flags, CvarError, CvarRegistry};
use qa_core::numeric::native_atoi;
use qa_net::common::hash::md4_block_checksum;
use qa_net::protocol::ProtocolIdentity;
use qa_net::q2_net::{read_q2_download_server, Q2NetError, Q2ServerEvent};
use qa_net::q2_svc::Q2DownloadSender;
use qa_net::services::downloads::{
    download_path, DownloadError, DownloadSink, DownloadSource, ProtocolDownloadExpectation, SinkExpectation,
};
use qa_net::services::http_downloads::{
    fetch_http_download_metadata, HttpClient, HttpDownloadError, HttpDownloadKind, HttpDownloadQueue,
    HttpDownloadRequest, HttpDownloadResult, HttpQueueCallbacks,
};
use thiserror::Error;

use super::client_download_policy::{
    client_download_category, ClientDownloadCategory, ClientDownloadPhase, ClientDownloadProgress,
    ClientDownloadRequest, ClientDownloadTransport,
};
use super::q2_layout::{q2_application_layout, Q2ApplicationLayout, Q2LayoutError};
use super::types::Q2ApplicationGameState;

/// Quake II download failure.
#[derive(Debug, Error)]
pub enum Q2DownloadError {
    /// Policy, validation, or protocol failure with donor text.
    #[error("{0}")]
    Message(String),
    /// Filesystem failure.
    #[error("Q2 download filesystem failure: {0}")]
    Filesystem(String),
    /// Cvar failure.
    #[error(transparent)]
    Cvar(#[from] CvarError),
    /// Download service failure.
    #[error(transparent)]
    Download(#[from] DownloadError),
    /// HTTP download failure.
    #[error(transparent)]
    Http(#[from] HttpDownloadError),
    /// Mount failure.
    #[error(transparent)]
    Mount(#[from] MountError),
    /// Catalog failure.
    #[error(transparent)]
    Catalog(#[from] CatalogError),
    /// Layout failure.
    #[error(transparent)]
    Layout(#[from] Q2LayoutError),
    /// Wire failure.
    #[error(transparent)]
    Net(#[from] Q2NetError),
    /// Format parse failure.
    #[error(transparent)]
    Binary(#[from] BinaryError),
    /// Archive failure.
    #[error(transparent)]
    Archive(#[from] qa_content::archive::ArchiveError),
}

/// Maximum retained file bytes (donor `0x7fffffff`).
const MAX_DOWNLOAD_BYTES: u64 = 0x7FFF_FFFF;

/// Native sender block size (donor `Q2DownloadSender` default).
const SENDER_BLOCK_BYTES: usize = 1024;

/// File-list metadata cap (donor `1 << 20`).
const FILELIST_MAX_BYTES: u64 = 1 << 20;

/// Classic world-model configstring slot (the donor hardcodes `33` in
/// `httpInitial`; the classic layout also places `models + 1` there).
const CLASSIC_WORLD_MODEL_SLOT: u32 = 33;

/// HTTP queue concurrency (donor `HttpDownloadQueue` default).
const HTTP_CONCURRENCY: usize = 2;

/// HTTP queue range streams (donor `HttpDownloadQueue` default).
const HTTP_RANGE_STREAMS: usize = 4;

// ---------------------------------------------------------------------------
// Server side
// ---------------------------------------------------------------------------

/// Server-side download policy (`Q2ApplicationDownloads`).
pub trait Q2ApplicationDownloads {
    /// HTTP download server, when configured.
    fn http_server(&self) -> Option<String> {
        None
    }
    /// Whether a name may be downloaded.
    fn allowed(&self, name: &str) -> bool;
    /// Open a name for transfer.
    fn open(&mut self, name: &str) -> Option<Box<dyn DownloadSource>>;
}

/// Server edition selecting the policy defaults (`'classic' | 'rerelease'`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q2DownloadEdition {
    /// Classic defaults (automatic transfers off).
    Classic,
    /// Rerelease defaults (automatic transfers on).
    Rerelease,
}

/// Mount-backed download policy (`createQ2ApplicationDownloads` result).
///
/// `FS_LoadFile` semantics: selected mounted bytes are retained for the
/// lifetime of one native transfer.
pub struct Q2MountedDownloads<'a> {
    mounts: &'a MountedContent,
    cvars: &'a CvarRegistry,
}

/// Register the server policy cvars and bind mounted downloads
/// (`createQ2ApplicationDownloads`).
pub fn create_q2_application_downloads<'a>(
    mounts: &'a MountedContent,
    cvars: &'a mut CvarRegistry,
    edition: Q2DownloadEdition,
) -> Result<Q2MountedDownloads<'a>, CvarError> {
    cvars.register("sv_downloadserver", "", 0)?;
    let master = match edition {
        Q2DownloadEdition::Classic => "0",
        Q2DownloadEdition::Rerelease => "1",
    };
    cvars.register("allow_download", master, q2_flags::ARCHIVE)?;
    cvars.register("allow_download_players", master, q2_flags::ARCHIVE)?;
    for name in ["models", "sounds", "maps"] {
        cvars.register(&format!("allow_download_{name}"), "1", q2_flags::ARCHIVE)?;
    }
    Ok(Q2MountedDownloads { mounts, cvars })
}

impl Q2ApplicationDownloads for Q2MountedDownloads<'_> {
    fn http_server(&self) -> Option<String> {
        read_q2_download_server(&[format!("dlserver={}", self.cvars.variable_string("sv_downloadserver"))])
    }

    fn allowed(&self, name: &str) -> bool {
        if name.contains("..")
            || name.starts_with('.')
            || !name.contains('/')
            || self.cvars.variable_value("allow_download") == 0.0
        {
            return false;
        }
        if download_path(name).is_err() {
            return false;
        }
        // Match complete categories so the original six-byte `maps/`
        // comparison cannot bypass policy.
        let path = name.to_lowercase();
        for (prefix, setting) in [
            ("players/", "players"),
            ("models/", "models"),
            ("sound/", "sounds"),
            ("maps/", "maps"),
        ] {
            if path.starts_with(prefix) && self.cvars.variable_value(&format!("allow_download_{setting}")) == 0.0 {
                return false;
            }
        }
        true
    }

    fn open(&mut self, name: &str) -> Option<Box<dyn DownloadSource>> {
        let opened = self.mounts.open(name, |_| true).ok()??;
        if opened.bytes.len() as u64 > MAX_DOWNLOAD_BYTES {
            return None;
        }
        if !can_download_resource(&opened.reference) {
            return None;
        }
        if let qa_content::contract::ResourceProvenance::Archive { member_path, .. } = &opened.reference.provenance {
            if member_path.to_lowercase().starts_with("maps/") {
                return None;
            }
        }
        Some(Box::new(MemoryDownloadSource {
            bytes: Some(opened.bytes),
        }))
    }
}

/// Retained-bytes download source for one native transfer.
struct MemoryDownloadSource {
    bytes: Option<Vec<u8>>,
}

impl DownloadSource for MemoryDownloadSource {
    fn byte_length(&self) -> u64 {
        self.bytes.as_ref().map_or(0, |bytes| bytes.len() as u64)
    }

    fn read(&mut self, offset: u64, max_bytes: usize) -> Result<Vec<u8>, DownloadError> {
        let Some(bytes) = self.bytes.as_ref() else {
            return Err(DownloadError::Filesystem("Q2 mounted download is closed".to_string()));
        };
        let start = (offset as usize).min(bytes.len());
        let end = start.saturating_add(max_bytes).min(bytes.len());
        Ok(bytes[start..end].to_vec())
    }

    fn close(&mut self) {
        self.bytes = None;
    }
}

/// Refused download event (donor `refused`).
fn refused_download() -> Q2ServerEvent {
    Q2ServerEvent::Download {
        bytes: None,
        percent: 0,
    }
}

/// One peer's source file (`Q2PeerDownload`).
///
/// Reconnect and world replacement cancel pending opens too.
pub struct Q2PeerDownload {
    sender: Option<Q2DownloadSender>,
    generation: u64,
}

impl Q2PeerDownload {
    /// Idle peer download.
    #[must_use]
    pub fn new() -> Self {
        Self {
            sender: None,
            generation: 0,
        }
    }

    /// Revision counter (donor `revision`).
    #[must_use]
    pub fn revision(&self) -> u64 {
        self.generation
    }

    /// Begin a transfer, answering the first block (`begin`).
    ///
    /// Returns `Ok(None)` when a newer `close` retired the open; the sync
    /// port has no await points, so the generation guard only observes
    /// reentrant closes.
    pub fn begin(
        &mut self,
        downloads: &mut dyn Q2ApplicationDownloads,
        name: &str,
        offset_text: Option<&str>,
    ) -> Result<Option<Q2ServerEvent>, Q2NetError> {
        let offset = native_atoi(offset_text.unwrap_or("0"));
        let Ok(offset) = offset else {
            return Ok(Some(refused_download()));
        };
        if offset < 0 || !downloads.allowed(name) {
            return Ok(Some(refused_download()));
        }
        self.close();
        let generation = self.generation;
        let source = downloads.open(name);
        if generation != self.generation {
            if let Some(mut source) = source {
                source.close();
            }
            return Ok(None);
        }
        let Some(source) = source else {
            return Ok(Some(refused_download()));
        };
        let start = (offset as u64).min(source.byte_length());
        let Ok(sender) = Q2DownloadSender::new(source, start, SENDER_BLOCK_BYTES) else {
            return Ok(Some(refused_download()));
        };
        self.sender = Some(sender);
        self.next()
    }

    /// Next download chunk, or `None` once the terminal marker passed.
    ///
    /// Named after the donor pump; it cannot implement `Iterator` because
    /// fallible reads return `Result`.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Result<Option<Q2ServerEvent>, Q2NetError> {
        match self.sender.as_mut() {
            Some(sender) => sender.next(),
            None => Ok(None),
        }
    }

    /// Cancel any pending transfer.
    pub fn close(&mut self) {
        self.generation += 1;
        if let Some(sender) = self.sender.as_mut() {
            sender.close();
        }
        self.sender = None;
    }
}

impl Default for Q2PeerDownload {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Client side
// ---------------------------------------------------------------------------

/// Content roots read from the out-of-scope `RemoteContentMounts` (donor
/// `../content.ts`, owned by the content lane).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteContentRoots {
    /// Player-model write root.
    pub base_write_root: PathBuf,
    /// General write root.
    pub write_root: PathBuf,
    /// Selected remote content.
    pub selection: RemoteContentSelection,
    /// Selected product's content directory.
    pub content_directory: String,
    /// Selected product's loose root, when it has one.
    pub loose_root: Option<PathBuf>,
    /// Catalog corpus root.
    pub corpus_root: PathBuf,
}

/// Client preparation verdict (`Q2DownloadPreparation`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q2DownloadPreparation {
    /// All resources installed.
    Ready,
    /// Transfers still running.
    Waiting,
    /// Cancelled or retired.
    Canceled,
}

/// Download block (`Q2DownloadBlock`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2DownloadBlock {
    /// Completion percent.
    pub percent: u8,
    /// Chunk bytes (`None` refuses the file).
    pub bytes: Option<Vec<u8>>,
}

/// Block outcome (`receive` result).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q2DownloadOutcome {
    /// File settled (completed, refused, or permission-revoked).
    Complete,
    /// More blocks expected.
    Waiting,
    /// No pending file wants this block.
    Unsolicited,
}

/// Protocol 34 advertises a completion percent but neither a size nor a
/// digest (`Q2ApplicationClientDownloads`).
pub trait Q2ApplicationClientDownloads {
    /// In-flight transfers.
    fn progress(&self) -> Vec<ClientDownloadProgress> {
        Vec::new()
    }
    /// Cancel all transfers.
    fn cancel(&mut self) {}
    /// Retry after a cancel.
    fn retry(&mut self) {}
    /// Set the HTTP download server.
    fn set_http_server(&mut self, server: Option<String>);
    /// Ensure the next resource, issuing at most one native request.
    fn prepare(&mut self, state: &Q2ApplicationGameState) -> Result<Q2DownloadPreparation, Q2DownloadError>;
    /// Receive one native block.
    fn receive(&mut self, block: &Q2DownloadBlock) -> Result<Q2DownloadOutcome, Q2DownloadError>;
    /// Cancel everything and forget the game state.
    fn close(&mut self);
}

/// HTTP asset scope (donor `HttpAssetScope`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HttpAssetScope {
    SearchPath,
    GameLocal,
}

/// Lazy resource walk for [`Q2DownloadReceiver::prepare`] (donor
/// `resources` generator).
///
/// The walk stays lazy so errors surface on the same `prepare` call as the
/// donor: the map checksum check runs only after the map yield is consumed,
/// and sky faces re-resolve between yields.
struct Q2ResourcePaths {
    stage: Q2ResourceStage,
    extra: VecDeque<String>,
    model_index: u32,
    sound_index: u32,
    image_index: u32,
    skin_slot: u32,
    texture_index: usize,
    sky_face: usize,
    sky_tga_yielded: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Q2ResourceStage {
    Map,
    MapCheck,
    Models,
    Sounds,
    Images,
    Skins,
    Sky,
    Textures,
    Done,
}

impl Q2ResourcePaths {
    fn new() -> Self {
        Self {
            stage: Q2ResourceStage::Map,
            extra: VecDeque::new(),
            model_index: 1,
            sound_index: 1,
            image_index: 1,
            skin_slot: 0,
            texture_index: 0,
            sky_face: 0,
            sky_tga_yielded: false,
        }
    }

    fn config_names(state: &Q2ApplicationGameState, first: u32, count: u32) -> Vec<String> {
        (1..count)
            .filter_map(|offset| state.config_strings.get(&first.saturating_add(offset)))
            .filter(|name| !name.is_empty())
            .cloned()
            .collect()
    }

    /// Pull the next path, opening/checking content exactly when the donor
    /// generator would resume.
    fn next(
        &mut self,
        mounts: &MountedContent,
        state: &Q2ApplicationGameState,
        layout: &Q2ApplicationLayout,
        map_bytes: &mut Option<Vec<u8>>,
    ) -> Result<Option<String>, Q2DownloadError> {
        if let Some(path) = self.extra.pop_front() {
            return Ok(Some(path));
        }
        loop {
            match self.stage {
                Q2ResourceStage::Map => {
                    let map = state
                        .config_strings
                        .get(&layout.models.saturating_add(1))
                        .cloned()
                        .ok_or_else(|| Q2DownloadError::Message("Q2 server supplied no world model".to_string()))?;
                    self.stage = Q2ResourceStage::MapCheck;
                    return Ok(Some(map));
                }
                Q2ResourceStage::MapCheck => {
                    let map = state
                        .config_strings
                        .get(&layout.models.saturating_add(1))
                        .cloned()
                        .unwrap_or_default();
                    let world = mounts
                        .open(&map, |_| true)?
                        .ok_or_else(|| Q2DownloadError::Message(format!("Q2 server map is unavailable: {map}")))?;
                    let checksum = state.config_strings.get(&layout.map_checksum);
                    let matches = checksum.is_some_and(|text| {
                        text.parse::<u32>()
                            .is_ok_and(|value| value == md4_block_checksum(&world.bytes))
                    });
                    if !matches {
                        return Err(Q2DownloadError::Message(
                            "Q2 server map checksum differs from mounted content".to_string(),
                        ));
                    }
                    *map_bytes = Some(world.bytes);
                    self.stage = Q2ResourceStage::Models;
                }
                Q2ResourceStage::Models => {
                    let map = state
                        .config_strings
                        .get(&layout.models.saturating_add(1))
                        .cloned()
                        .unwrap_or_default();
                    let models = Self::config_names(state, layout.models, layout.max_models);
                    for model in models.iter().skip(self.model_index.saturating_sub(1) as usize) {
                        self.model_index += 1;
                        if *model == map || model.starts_with('*') || model.starts_with('#') {
                            continue;
                        }
                        let lowered = model.to_lowercase();
                        if let Some(opened) = mounts.open(model, |_| true)? {
                            if lowered.ends_with(".md2") {
                                for skin in &parse_md2(&opened.bytes, model)?.skins {
                                    if !skin.is_empty() {
                                        self.extra.push_back(skin.clone());
                                    }
                                }
                            } else if lowered.ends_with(".sp2") {
                                for frame in &parse_sp2(&opened.bytes, model)?.frames {
                                    self.extra.push_back(frame.image.clone());
                                }
                            }
                        }
                        return Ok(Some(model.clone()));
                    }
                    self.stage = Q2ResourceStage::Sounds;
                }
                Q2ResourceStage::Sounds => {
                    let sounds = Self::config_names(state, layout.sounds, layout.max_sounds);
                    for sound in sounds.iter().skip(self.sound_index.saturating_sub(1) as usize) {
                        self.sound_index += 1;
                        if sound.starts_with('*') {
                            continue;
                        }
                        let path = sound
                            .strip_prefix('#')
                            .map_or_else(|| format!("sound/{sound}"), ToString::to_string);
                        return Ok(Some(path));
                    }
                    self.stage = Q2ResourceStage::Images;
                }
                Q2ResourceStage::Images => {
                    let images = Self::config_names(state, layout.images, layout.max_images);
                    let image = images.get(self.image_index.saturating_sub(1) as usize).cloned();
                    match image {
                        Some(image) => {
                            self.image_index += 1;
                            return Ok(Some(
                                image
                                    .strip_prefix('/')
                                    .map_or_else(|| format!("pics/{image}.pcx"), ToString::to_string),
                            ));
                        }
                        None => self.stage = Q2ResourceStage::Skins,
                    }
                }
                Q2ResourceStage::Skins => {
                    while self.skin_slot < 256 {
                        let value = state
                            .config_strings
                            .get(&layout.player_skins.saturating_add(self.skin_slot))
                            .cloned()
                            .unwrap_or_default();
                        self.skin_slot += 1;
                        if value.is_empty() {
                            continue;
                        }
                        let skin = value.split('\\').nth(1).unwrap_or("");
                        let Some(slash) = skin.find('/') else { continue };
                        if slash < 1 {
                            continue;
                        }
                        let model = &skin[..slash];
                        let name = &skin[slash + 1..];
                        self.extra.push_back(format!("players/{model}/weapon.md2"));
                        self.extra.push_back(format!("players/{model}/weapon.pcx"));
                        self.extra.push_back(format!("players/{model}/{name}.pcx"));
                        self.extra.push_back(format!("players/{model}/{name}_i.pcx"));
                        return Ok(Some(format!("players/{model}/tris.md2")));
                    }
                    self.stage = Q2ResourceStage::Sky;
                }
                Q2ResourceStage::Sky => {
                    let sky = state.config_strings.get(&2).cloned().unwrap_or_default();
                    if sky.is_empty() || self.sky_face >= SKY_FACE_SUFFIXES.len() {
                        self.stage = Q2ResourceStage::Textures;
                        continue;
                    }
                    let suffix = SKY_FACE_SUFFIXES[self.sky_face];
                    let tga = format!("env/{sky}{suffix}.tga");
                    let pcx = format!("env/{sky}{suffix}.pcx");
                    if mounts.resolve(&tga)?.is_some() || mounts.resolve(&pcx)?.is_some() {
                        self.sky_face += 1;
                        self.sky_tga_yielded = false;
                        continue;
                    }
                    if !self.sky_tga_yielded {
                        self.sky_tga_yielded = true;
                        return Ok(Some(tga));
                    }
                    self.sky_tga_yielded = false;
                    self.sky_face += 1;
                    if mounts.resolve(&tga)?.is_none() {
                        return Ok(Some(pcx));
                    }
                }
                Q2ResourceStage::Textures => {
                    let Some(bytes) = map_bytes.as_ref() else {
                        self.stage = Q2ResourceStage::Done;
                        continue;
                    };
                    let map = state
                        .config_strings
                        .get(&layout.models.saturating_add(1))
                        .cloned()
                        .unwrap_or_default();
                    let world = read_q2_bsp(bytes, &map)?;
                    if self.texture_index < world.texture_info.len() {
                        let name = world.texture_info[self.texture_index].name.clone();
                        self.texture_index += 1;
                        return Ok(Some(format!("textures/{name}.wal")));
                    }
                    self.stage = Q2ResourceStage::Done;
                }
                Q2ResourceStage::Done => return Ok(None),
            }
        }
    }
}

/// Whether a path uses only file-list characters (donor
/// `/^[a-zA-Z0-9_+./-]+$/`).
fn is_path_chars(path: &str) -> bool {
    !path.is_empty()
        && path
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'+' | b'.' | b'/' | b'-'))
}

/// Whether a name is a flat package file (donor
/// `/^[a-zA-Z0-9_+.-]+\.(?:pak|pkz)$/i`).
fn is_flat_package(name: &str) -> bool {
    let lowered = name.to_lowercase();
    let stem = lowered.strip_suffix(".pak").or_else(|| lowered.strip_suffix(".pkz"));
    stem.is_some_and(|stem| {
        !stem.is_empty()
            && stem
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'+' | b'.' | b'-'))
    })
}

/// Whether a path is an HTTP file-list asset (donor whitelist plus the
/// JPEG/BMP/GIF extensions the shared image reader supports).
fn is_filelist_asset(path: &str) -> bool {
    if !is_path_chars(path) || !path.contains('/') {
        return false;
    }
    let lowered = path.to_lowercase();
    [
        ".bsp", ".dm2", ".ent", ".jpg", ".loc", ".md2", ".md3", ".ogg", ".pcx", ".png", ".sp2", ".tga", ".txt", ".wal",
        ".wav", ".jpeg", ".bmp", ".gif",
    ]
    .iter()
    .any(|extension| lowered.ends_with(extension))
}

/// Whether a server-requested path is a downloadable asset (donor
/// `validate` pattern).
fn is_server_asset(path: &str) -> bool {
    if !is_path_chars(path) {
        return false;
    }
    let lowered = path.to_lowercase();
    for (prefix, extensions) in [
        ("maps/", [".bsp"].as_slice()),
        ("models/", [".md2", ".sp2", ".pcx", ".wav"].as_slice()),
        ("players/", [".md2", ".sp2", ".pcx", ".wav"].as_slice()),
        ("sound/", [".wav"].as_slice()),
        ("pics/", [".pcx"].as_slice()),
        ("env/", [".tga", ".pcx"].as_slice()),
        ("textures/", [".wal"].as_slice()),
    ] {
        if let Some(rest) = lowered.strip_prefix(prefix) {
            return extensions.iter().any(|extension| rest.ends_with(extension));
        }
    }
    false
}

/// Percent-encode one URL segment (donor `encodeURIComponent`).
fn encode_segment(segment: &str) -> String {
    let mut encoded = String::with_capacity(segment.len());
    for byte in segment.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

/// Join a base URL and a relative reference (donor `new URL(relative, base)`
/// for the path-only references this module builds).
fn join_url(base: &str, relative: &str) -> String {
    format!("{}/{}", base.trim_end_matches('/'), relative.trim_start_matches('/'))
}

/// Pending native transfer.
struct PendingNativeDownload {
    path: String,
    sink: DownloadSink,
    percent: u8,
}

/// Download permission callback.
pub type Q2DownloadPermissionFn<'a> = Box<dyn Fn(&ClientDownloadRequest) -> bool + 'a>;

/// [`Q2DownloadReceiver`] constructor options.
pub struct Q2DownloadReceiverOptions<'a> {
    /// Mounted content.
    pub mounts: &'a MountedContent,
    /// Fresh content roots (selection may change after a game switch).
    pub content: Box<dyn FnMut() -> RemoteContentRoots + 'a>,
    /// Client command sink.
    pub command: Box<dyn FnMut(&str) + 'a>,
    /// Print sink.
    pub print: Box<dyn FnMut(&str) + 'a>,
    /// Package refresh hook.
    pub refresh_packages: Option<Box<dyn FnMut() -> Result<(), Q2DownloadError> + 'a>>,
    /// Download permission (`None` allows everything).
    pub permission: Option<Q2DownloadPermissionFn<'a>>,
    /// HTTP client factory (the donor's global fetch).
    pub make_http_client: Box<dyn FnMut() -> Box<dyn HttpClient> + 'a>,
}

/// Quake II download receiver (`Q2DownloadReceiver`).
///
/// One peer owns its HTTP queue, native sink, and resource walk; `close`
/// retires all three. Native Q2 cannot download flat packages, so package
/// publication settles before assets.
pub struct Q2DownloadReceiver<'a> {
    mounts: &'a MountedContent,
    content: Box<dyn FnMut() -> RemoteContentRoots + 'a>,
    command: Box<dyn FnMut(&str) + 'a>,
    print: Box<dyn FnMut(&str) + 'a>,
    refresh_packages: Option<Box<dyn FnMut() -> Result<(), Q2DownloadError> + 'a>>,
    permission: Q2DownloadPermissionFn<'a>,
    make_http_client: Box<dyn FnMut() -> Box<dyn HttpClient> + 'a>,
    cancelled: bool,
    retired_block: bool,
    retry_requested: bool,
    server: Option<String>,
    http: Option<HttpDownloadQueue>,
    attempted: HashSet<String>,
    http_paths: HashMap<String, String>,
    retry_path: Option<String>,
    generation: Rc<Cell<u64>>,
    state: Option<Q2ApplicationGameState>,
    paths: Option<Q2ResourcePaths>,
    map_bytes: Option<Vec<u8>>,
    pending: Option<PendingNativeDownload>,
    refused: HashSet<String>,
}

impl<'a> Q2DownloadReceiver<'a> {
    /// Build a receiver over its sinks and content.
    pub fn new(options: Q2DownloadReceiverOptions<'a>) -> Self {
        Self {
            mounts: options.mounts,
            content: options.content,
            command: options.command,
            print: options.print,
            refresh_packages: options.refresh_packages,
            permission: options.permission.unwrap_or_else(|| Box::new(|_| true)),
            make_http_client: options.make_http_client,
            cancelled: false,
            retired_block: false,
            retry_requested: false,
            server: None,
            http: None,
            attempted: HashSet::new(),
            http_paths: HashMap::new(),
            retry_path: None,
            generation: Rc::new(Cell::new(0)),
            state: None,
            paths: None,
            map_bytes: None,
            pending: None,
            refused: HashSet::new(),
        }
    }

    /// Revision counter (donor `revision`).
    #[must_use]
    pub fn revision(&self) -> u64 {
        self.generation.get()
    }

    /// Whether a request is permitted.
    fn permitted(&self, transport: ClientDownloadTransport, path: &str) -> bool {
        (self.permission)(&ClientDownloadRequest {
            transport,
            category: client_download_category(path),
        })
    }

    /// Write root for a path (player models use the base root).
    fn root(&mut self, path: &str) -> PathBuf {
        let roots = (self.content)();
        if path.to_lowercase().starts_with("players/") {
            roots.base_write_root
        } else {
            roots.write_root
        }
    }

    /// HTTP destination for a path.
    fn http_destination(&mut self, path: &str) -> String {
        let root = self.root(path);
        let base = root
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        format!("{base}/{path}")
    }

    /// Resolve a destination back to its path.
    fn http_resource(&self, destination: &str) -> Result<&str, Q2DownloadError> {
        self.http_paths
            .get(destination)
            .map(String::as_str)
            .ok_or_else(|| Q2DownloadError::Message("Unknown Q2 HTTP download destination".to_string()))
    }

    /// Assert the connection epoch is still current (donor `current`).
    ///
    /// The sync port runs every step inline, so generation is the only
    /// live check; permission is rechecked at each entry point instead of
    /// mid-flight.
    fn assert_current(&self, generation: u64) -> Result<(), Q2DownloadError> {
        if generation != self.generation.get() {
            return Err(Q2DownloadError::Message("Q2 download generation retired".to_string()));
        }
        Ok(())
    }

    /// Whether a path is already installed (donor queue `resolved`).
    ///
    /// Runs at enqueue time because the queue callback cannot borrow the
    /// receiver; game-local assets only match mounts under the game roots.
    fn http_resolved(&mut self, destination: &str, scope: HttpAssetScope) -> bool {
        let path = match self.http_resource(destination) {
            Ok(path) => path.to_string(),
            Err(_) => return false,
        };
        if !self.permitted(ClientDownloadTransport::Http, &path) {
            return true;
        }
        let roots = (self.content)();
        let mut game_roots = HashSet::new();
        game_roots.insert(roots.corpus_root.join(&roots.content_directory));
        game_roots.insert(roots.write_root.clone());
        if let Some(loose) = &roots.loose_root {
            game_roots.insert(loose.clone());
        }
        self.mounts
            .open(&path, |mount| {
                if scope != HttpAssetScope::GameLocal {
                    return true;
                }
                let root = match mount {
                    qa_content::contract::ContentMount::Archive(mount) => Path::new(&mount.archive_path)
                        .parent()
                        .map(Path::to_path_buf)
                        .unwrap_or_default(),
                    qa_content::contract::ContentMount::Loose(mount) => PathBuf::from(&mount.root_path),
                };
                game_roots.contains(&root)
            })
            .is_ok_and(|found| found.is_some())
    }

    /// Fetch one asset over HTTP, pumping the queue to completion
    /// (donor `httpAsset`).
    fn http_asset(&mut self, path: &str, kind: HttpDownloadKind, scope: HttpAssetScope) -> Result<(), Q2DownloadError> {
        if self.http.is_none() || self.server.is_none() || self.state.is_none() {
            return Ok(());
        }
        if self.attempted.contains(path) {
            return Ok(());
        }
        if !self.permitted(ClientDownloadTransport::Http, path) {
            return Ok(());
        }
        self.attempted.insert(path.to_string());
        let destination = self.http_destination(path);
        self.http_paths.insert(destination.clone(), path.to_string());
        if self.http_resolved(&destination, scope) {
            return Ok(());
        }
        let game = self
            .state
            .as_ref()
            .map(|state| state.data.gamedir().to_string())
            .filter(|gamedir| !gamedir.is_empty())
            .unwrap_or_else(|| "baseq2".to_string());
        let encoded = path.split('/').map(encode_segment).collect::<Vec<_>>().join("/");
        let Some(server) = self.server.clone() else {
            return Ok(());
        };
        let url = join_url(&server, &format!("{}/{encoded}", encode_segment(&game)));
        let staged_path = path.to_string();
        let package = kind == HttpDownloadKind::Package;
        let validate_tag = {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            let mut hasher = DefaultHasher::new();
            destination.hash(&mut hasher);
            hasher.finish()
        };
        let Some(queue) = self.http.as_mut() else {
            return Ok(());
        };
        queue.enqueue(HttpDownloadRequest {
            path: destination.clone(),
            url,
            kind,
            expected: SinkExpectation::Protocol(ProtocolDownloadExpectation {
                maximum_bytes: MAX_DOWNLOAD_BYTES,
            }),
            validate: Some((
                validate_tag,
                Box::new(move |staged: &Path| {
                    if package {
                        let format = if staged_path.to_lowercase().ends_with(".pak") {
                            ArchiveFormat::Pak
                        } else {
                            ArchiveFormat::Zip
                        };
                        open_archive(staged, Some(format))
                            .map_err(|error| HttpDownloadError::ValidationFailed(error.to_string()))?;
                    }
                    Ok(())
                }),
            )),
        })?;
        let settled = self.pump_http(&destination)?;
        if let HttpDownloadResult::Failed { reason } = &settled {
            return Err(Q2DownloadError::Message(reason.clone()));
        }
        if matches!(settled, HttpDownloadResult::Fallback { .. }) {
            let hint = if package {
                "continuing with individual files"
            } else {
                "using native download"
            };
            (self.print)(&format!("HTTP unavailable: {path}; {hint}\n"));
        }
        if package && matches!(settled, HttpDownloadResult::Downloaded { .. }) {
            self.http_resource(&destination)?;
            match self.refresh_packages.as_mut() {
                Some(refresh) => refresh()?,
                None => {
                    return Err(Q2DownloadError::Message(
                        "Q2 package refresh is unavailable".to_string(),
                    ));
                }
            }
        }
        Ok(())
    }

    /// Pump the HTTP queue until a destination settles.
    fn pump_http(&mut self, destination: &str) -> Result<HttpDownloadResult, Q2DownloadError> {
        let cap = self.http_paths.len().saturating_add(2);
        for _ in 0..cap {
            if let Some(queue) = self.http.as_mut() {
                queue.process();
                if let Some(result) = queue.result(destination) {
                    return Ok(result.clone());
                }
            }
        }
        Err(Q2DownloadError::Message("Q2 HTTP download stalled".to_string()))
    }

    /// Fetch file lists and referenced assets (donor `httpInitial`).
    fn http_initial(&mut self, state: &Q2ApplicationGameState) -> Result<(), Q2DownloadError> {
        let Some(server) = self.server.clone() else {
            return Ok(());
        };
        if !(self.permission)(&ClientDownloadRequest {
            transport: ClientDownloadTransport::Http,
            category: ClientDownloadCategory::Metadata,
        }) {
            return Ok(());
        }
        let generation = self.generation.get();
        let roots = (self.content)();
        let parent = roots
            .write_root
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| roots.write_root.clone());
        std::fs::create_dir_all(&parent).map_err(|error| Q2DownloadError::Filesystem(error.to_string()))?;
        self.assert_current(generation)?;
        if !(self.permission)(&ClientDownloadRequest {
            transport: ClientDownloadTransport::Http,
            category: ClientDownloadCategory::Metadata,
        }) {
            return Err(Q2DownloadError::Message(
                "Q2 HTTP download permission changed".to_string(),
            ));
        }
        let queue_generation = Rc::clone(&self.generation);
        let client = (self.make_http_client)();
        self.http = Some(HttpDownloadQueue::new(
            parent,
            client,
            HttpQueueCallbacks {
                assert_current: Box::new(move || {
                    if queue_generation.get() == generation {
                        Ok(())
                    } else {
                        Err(HttpDownloadError::StaleEpoch)
                    }
                }),
                // Resolution is pre-checked at enqueue time (see
                // `http_resolved`); the queue never skips on its own.
                resolved: Box::new(|_| false),
                // Packages refresh explicitly after they settle (see
                // `http_asset`).
                refresh_package: Box::new(|_| Ok(())),
                progress: Box::new(|_, _, _| {}),
            },
            HTTP_CONCURRENCY,
            HTTP_RANGE_STREAMS,
        )?);
        let game = if state.data.gamedir().is_empty() {
            "baseq2".to_string()
        } else {
            state.data.gamedir().to_string()
        };
        let mut assets: HashMap<String, HttpAssetScope> = HashMap::new();
        let mut add_asset = |path: String, scope: HttpAssetScope| {
            if assets.get(&path) != Some(&HttpAssetScope::GameLocal) {
                assets.insert(path, scope);
            }
        };
        let mut lists = vec![format!("{}.filelist", encode_segment(&game))];
        if let Some(map) = state.config_strings.get(&CLASSIC_WORLD_MODEL_SLOT) {
            Self::validate_server_path(map)?;
            lists.push(format!(
                "{}/{}.filelist",
                encode_segment(&game),
                map.strip_suffix(".bsp")
                    .unwrap_or(map)
                    .split('/')
                    .map(encode_segment)
                    .collect::<Vec<_>>()
                    .join("/")
            ));
            add_asset(map.clone(), HttpAssetScope::SearchPath);
        }
        let mut packages: Vec<String> = Vec::new();
        for list in &lists {
            if !(self.permission)(&ClientDownloadRequest {
                transport: ClientDownloadTransport::Http,
                category: ClientDownloadCategory::Metadata,
            }) {
                return Ok(());
            }
            let url = join_url(&server, list);
            let mut client = (self.make_http_client)();
            let bytes = fetch_http_download_metadata(&mut *client, &url, FILELIST_MAX_BYTES);
            self.assert_current(generation)?;
            let Some(bytes) = bytes else { continue };
            for raw in String::from_utf8_lossy(&bytes).split('\n') {
                let line = raw.strip_suffix('\r').unwrap_or(raw);
                if line.is_empty() {
                    continue;
                }
                if line.to_lowercase().ends_with(".pak") || line.to_lowercase().ends_with(".pkz") {
                    let valid = download_path(line).is_ok() && is_flat_package(line);
                    if !valid {
                        (self.print)(&format!(
                            "Ignoring invalid Q2 filelist entry: {}\n",
                            line.chars().take(128).collect::<String>()
                        ));
                        continue;
                    }
                    if self.refresh_packages.is_some() && !packages.contains(&line.to_string()) {
                        packages.push(line.to_string());
                    }
                    continue;
                }
                let (path, scope) = match line.strip_prefix('@') {
                    Some(rest) => (rest.to_string(), HttpAssetScope::GameLocal),
                    None => (line.to_string(), HttpAssetScope::SearchPath),
                };
                if download_path(&path).is_err() || !is_filelist_asset(&path) {
                    (self.print)(&format!(
                        "Ignoring invalid Q2 filelist entry: {}\n",
                        line.chars().take(128).collect::<String>()
                    ));
                    continue;
                }
                add_asset(path, scope);
            }
        }
        // Native Q2 cannot download flat packages. Settle package
        // publication/reload before assets.
        for package in packages {
            self.http_asset(&package, HttpDownloadKind::Package, HttpAssetScope::SearchPath)?;
            self.assert_current(generation)?;
        }
        let layout = q2_application_layout(ProtocolIdentity::Q2Classic)?;
        for index in 1..layout.max_models {
            let Some(path) = state.config_strings.get(&layout.models.saturating_add(index)) else {
                continue;
            };
            if path.is_empty() || path.starts_with('*') || path.starts_with('#') {
                continue;
            }
            Self::validate_server_path(path)?;
            add_asset(path.clone(), HttpAssetScope::SearchPath);
        }
        for index in 1..layout.max_sounds {
            let Some(name) = state.config_strings.get(&layout.sounds.saturating_add(index)) else {
                continue;
            };
            if name.is_empty() || name.starts_with('*') {
                continue;
            }
            let path = name
                .strip_prefix('#')
                .map_or_else(|| format!("sound/{name}"), ToString::to_string);
            Self::validate_server_path(&path)?;
            add_asset(path, HttpAssetScope::SearchPath);
        }
        let mut ordered: Vec<(String, HttpAssetScope)> = assets.into_iter().collect();
        ordered.sort_by(|left, right| left.0.cmp(&right.0));
        for (path, scope) in ordered {
            self.http_asset(&path, HttpDownloadKind::Asset, scope)?;
        }
        self.assert_current(generation)?;
        Ok(())
    }

    /// Validate a server-requested path (donor `validate`).
    fn validate_server_path(path: &str) -> Result<(), Q2DownloadError> {
        download_path(path)?;
        if !is_server_asset(path) {
            return Err(Q2DownloadError::Message(format!(
                "Q2 server requested a non-asset download: {path}"
            )));
        }
        Ok(())
    }

    /// Run one preparation step, retiring the epoch on reentrant closes.
    fn prepare_inner(&mut self, state: &Q2ApplicationGameState) -> Result<Q2DownloadPreparation, Q2DownloadError> {
        if self.cancelled {
            return Ok(Q2DownloadPreparation::Canceled);
        }
        let selected = remote_content_selection(RemoteContentBase::Q2ClassicBaseq2, state.data.gamedir())?;
        let owner = (self.content)();
        if owner.selection != selected {
            return Err(Q2DownloadError::Message(
                "Q2 server game differs from prepared content".to_string(),
            ));
        }
        if self.state.as_ref() != Some(state) {
            self.close();
            self.state = Some(state.clone());
            if self.server.is_some()
                && (self.permission)(&ClientDownloadRequest {
                    transport: ClientDownloadTransport::Http,
                    category: ClientDownloadCategory::Metadata,
                })
            {
                self.http_initial(state)?;
            }
        }
        if self.pending.is_some() {
            return Ok(Q2DownloadPreparation::Waiting);
        }
        if self.paths.is_none() {
            self.paths = Some(Q2ResourcePaths::new());
        }
        let generation = self.generation.get();
        let layout = q2_application_layout(ProtocolIdentity::Q2Classic)?;
        loop {
            let next = match self.retry_path.take() {
                Some(path) => Some(path),
                None => {
                    let Some(paths) = self.paths.as_mut() else {
                        return Ok(Q2DownloadPreparation::Canceled);
                    };
                    let mut map_bytes = self.map_bytes.take();
                    let pulled = paths.next(self.mounts, state, &layout, &mut map_bytes);
                    self.map_bytes = map_bytes;
                    pulled?
                }
            };
            if generation != self.generation.get() {
                return Ok(Q2DownloadPreparation::Canceled);
            }
            let Some(path) = next else {
                return Ok(Q2DownloadPreparation::Ready);
            };
            Self::validate_server_path(&path)?;
            let installed = self.mounts.resolve(&path)?;
            if generation != self.generation.get() {
                return Ok(Q2DownloadPreparation::Canceled);
            }
            if installed.is_some() || self.refused.contains(&path) {
                continue;
            }
            if self.http.is_some()
                && !self.attempted.contains(&path)
                && self.permitted(ClientDownloadTransport::Http, &path)
            {
                self.retry_path = Some(path.clone());
                self.http_asset(&path, HttpDownloadKind::Asset, HttpAssetScope::SearchPath)?;
                return Ok(Q2DownloadPreparation::Waiting);
            }
            if !self.permitted(ClientDownloadTransport::Native, &path) {
                continue;
            }
            let root = self.root(&path);
            std::fs::create_dir_all(&root).map_err(|error| Q2DownloadError::Filesystem(error.to_string()))?;
            if generation != self.generation.get() {
                return Ok(Q2DownloadPreparation::Canceled);
            }
            if !self.permitted(ClientDownloadTransport::Native, &path) {
                continue;
            }
            let sink = DownloadSink::create(
                &root,
                &path,
                SinkExpectation::Protocol(ProtocolDownloadExpectation {
                    maximum_bytes: MAX_DOWNLOAD_BYTES,
                }),
            )?;
            self.pending = Some(PendingNativeDownload {
                path: path.clone(),
                sink,
                percent: 0,
            });
            (self.command)(&format!("download {path}"));
            return Ok(Q2DownloadPreparation::Waiting);
        }
    }
}

impl Q2ApplicationClientDownloads for Q2DownloadReceiver<'_> {
    fn progress(&self) -> Vec<ClientDownloadProgress> {
        let mut items = Vec::new();
        if let Some(queue) = self.http.as_ref() {
            for item in queue.progress() {
                let Some(path) = self.http_paths.get(&item.path) else {
                    continue;
                };
                items.push(ClientDownloadProgress {
                    path: path.clone(),
                    transport: ClientDownloadTransport::Http,
                    received: item.received,
                    total: item.total,
                    percent: item
                        .total
                        .filter(|total| *total != 0)
                        .map(|total| item.received as f64 * 100.0 / total as f64),
                    phase: match item.phase {
                        "running" => ClientDownloadPhase::Running,
                        "done" => ClientDownloadPhase::Done,
                        _ => ClientDownloadPhase::Pending,
                    },
                });
            }
        }
        if let Some(pending) = self.pending.as_ref() {
            items.push(ClientDownloadProgress {
                path: pending.path.clone(),
                transport: ClientDownloadTransport::Native,
                received: pending.sink.byte_length(),
                total: None,
                percent: Some(f64::from(pending.percent)),
                phase: ClientDownloadPhase::Running,
            });
        }
        items
    }

    fn cancel(&mut self) {
        if self.cancelled {
            return;
        }
        let waiting = self.pending.is_some();
        self.close();
        self.cancelled = true;
        self.retired_block = waiting;
    }

    fn retry(&mut self) {
        if self.retired_block {
            self.retry_requested = true;
            return;
        }
        self.close();
        self.cancelled = false;
    }

    fn set_http_server(&mut self, server: Option<String>) {
        self.close();
        self.cancelled = false;
        self.server = server;
    }

    fn prepare(&mut self, state: &Q2ApplicationGameState) -> Result<Q2DownloadPreparation, Q2DownloadError> {
        let generation = self.generation.get();
        match self.prepare_inner(state) {
            Ok(preparation) => Ok(preparation),
            Err(error) => {
                if generation != self.generation.get() {
                    return Ok(Q2DownloadPreparation::Canceled);
                }
                self.close();
                Err(error)
            }
        }
    }

    fn receive(&mut self, block: &Q2DownloadBlock) -> Result<Q2DownloadOutcome, Q2DownloadError> {
        if self.retired_block {
            self.retired_block = false;
            if self.retry_requested {
                self.retry_requested = false;
                self.cancelled = false;
            }
            return Ok(Q2DownloadOutcome::Complete);
        }
        let pending_path = match self.pending.as_ref() {
            Some(pending) => pending.path.clone(),
            None => return Ok(Q2DownloadOutcome::Unsolicited),
        };
        if !self.permitted(ClientDownloadTransport::Native, &pending_path) {
            if let Some(mut pending) = self.pending.take() {
                pending.sink.close();
            }
            return Ok(Q2DownloadOutcome::Complete);
        }
        let outcome = (|| -> Result<Q2DownloadOutcome, Q2DownloadError> {
            let Some(pending) = self.pending.as_mut() else {
                return Ok(Q2DownloadOutcome::Unsolicited);
            };
            let Some(bytes) = block.bytes.as_ref() else {
                let mut pending = self.pending.take().expect("pending checked");
                self.refused.insert(pending.path.clone());
                (self.print)(&format!("Server download unavailable: {}\n", pending.path));
                pending.sink.close();
                return Ok(Q2DownloadOutcome::Complete);
            };
            if block.percent < pending.percent || block.percent > 100 {
                return Err(Q2DownloadError::Message("Invalid Q2 download progress".to_string()));
            }
            pending.sink.append(bytes)?;
            pending.percent = block.percent;
            // Native zero-length files end with size=0, percent=0.
            if block.percent == 100 || (bytes.is_empty() && pending.sink.byte_length() == 0) {
                let mut pending = self.pending.take().expect("pending checked");
                pending.sink.finish()?;
                return Ok(Q2DownloadOutcome::Complete);
            }
            if bytes.is_empty() {
                return Err(Q2DownloadError::Message("Q2 download made no progress".to_string()));
            }
            (self.command)("nextdl");
            Ok(Q2DownloadOutcome::Waiting)
        })();
        match outcome {
            Ok(outcome) => Ok(outcome),
            Err(error) => {
                self.close();
                Err(error)
            }
        }
    }

    fn close(&mut self) {
        self.retired_block = false;
        self.retry_requested = false;
        self.generation.set(self.generation.get().wrapping_add(1));
        if let Some(queue) = self.http.as_mut() {
            queue.cancel();
        }
        self.http = None;
        self.attempted.clear();
        self.http_paths.clear();
        self.retry_path = None;
        if let Some(mut pending) = self.pending.take() {
            pending.sink.close();
        }
        self.paths = None;
        self.map_bytes = None;
        self.state = None;
        self.refused.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_content::contract::{MountPlanId, ResolvedMountPlan};
    use qa_content::mounts::{open_mount_plan, OpenMountOptions};
    use qa_core::cmd::Dialect;
    use qa_net::q2_net::Q2ServerData;
    use qa_net::services::http_downloads::{HttpMethod, HttpResponse};
    use std::cell::RefCell;

    fn empty_mounts() -> MountedContent {
        let plan = ResolvedMountPlan {
            id: MountPlanId("mount-plan:test:q2dl".to_string()),
            mounts: Vec::new(),
            default_order: Vec::new(),
            prefix_orders: Vec::new(),
        };
        open_mount_plan(
            &plan,
            OpenMountOptions {
                pure: None,
                q3_restriction: None,
                links: Vec::new(),
                loose_comparison: None,
            },
        )
        .expect("empty plan opens")
    }

    fn scratch_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("qa-q2dl-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");
        dir
    }

    fn game_state(gamedir: &str, configs: Vec<(u32, String)>) -> Q2ApplicationGameState {
        Q2ApplicationGameState {
            data: Q2ServerData::Vanilla(qa_net::q2::ServerData {
                servercount: 1,
                attractloop: false,
                gamedir: gamedir.to_string(),
                clientnum: 0,
                levelname: "test".to_string(),
            }),
            config_strings: configs.into_iter().collect(),
            baselines: HashMap::new(),
        }
    }

    struct NullHttpClient;

    impl HttpClient for NullHttpClient {
        fn request(
            &mut self,
            _method: HttpMethod,
            _url: &str,
            _headers: &[(String, String)],
        ) -> Result<HttpResponse, HttpDownloadError> {
            Ok(HttpResponse {
                status: 404,
                headers: Vec::new(),
                body: Vec::new(),
            })
        }
    }

    #[test]
    fn validates_server_and_filelist_paths() {
        assert!(is_server_asset("maps/q2dm1.bsp"));
        assert!(is_server_asset("players/male/tris.md2"));
        assert!(is_server_asset("models/weapons/v_shot.md2"));
        assert!(is_server_asset("sound/world/wind.wav"));
        assert!(is_server_asset("pics/health.pcx"));
        assert!(is_server_asset("env/skyrt.tga"));
        assert!(is_server_asset("textures/wall.wal"));
        assert!(!is_server_asset("maps/q2dm1.bsp "));
        assert!(!is_server_asset("gamex86/q2.dll"));
        assert!(!is_server_asset("maps/../autoexec.cfg"));
        assert!(is_filelist_asset("maps/q2dm1.bsp"));
        assert!(is_filelist_asset("pics/health.jpeg"));
        assert!(!is_filelist_asset("flatname.bsp"));
        assert!(!is_filelist_asset("gamex86/q2.dll"));
        assert!(is_flat_package("maps.pkz"));
        assert!(is_flat_package("extra.PAK"));
        assert!(!is_flat_package("subdir/maps.pak"));
        assert_eq!(encode_segment("a b/c"), "a%20b%2Fc");
        assert_eq!(
            join_url("https://dl.example/q2/", "/baseq2/a.filelist"),
            "https://dl.example/q2/baseq2/a.filelist"
        );
    }

    #[test]
    fn server_policy_gates_categories() {
        let mounts = empty_mounts();
        let mut cvars = CvarRegistry::new(Dialect::Q2Classic);
        let mut classic =
            create_q2_application_downloads(&mounts, &mut cvars, Q2DownloadEdition::Classic).expect("register");
        assert!(!classic.allowed("maps/q2dm1.bsp"));
        assert!(classic.open("maps/q2dm1.bsp").is_none());
        let mut cvars = CvarRegistry::new(Dialect::Q2Classic);
        // Pre-stage the gate: registration never replaces a live value, so
        // the policy observes the staged `0` afterwards.
        cvars.set("allow_download_sounds", "0", true).expect("set");
        let rerelease =
            create_q2_application_downloads(&mounts, &mut cvars, Q2DownloadEdition::Rerelease).expect("register");
        assert!(rerelease.allowed("maps/q2dm1.bsp"));
        assert!(!rerelease.allowed("sound/world/wind.wav"));
        assert!(!rerelease.allowed("maps/../evil"));
        assert!(!rerelease.allowed(".hidden/file"));
        assert!(!rerelease.allowed("flatname"));
    }

    struct ScriptDownloads {
        allowed: bool,
        bytes: Vec<u8>,
    }

    impl Q2ApplicationDownloads for ScriptDownloads {
        fn allowed(&self, _name: &str) -> bool {
            self.allowed
        }

        fn open(&mut self, _name: &str) -> Option<Box<dyn DownloadSource>> {
            Some(Box::new(MemoryDownloadSource {
                bytes: Some(self.bytes.clone()),
            }))
        }
    }

    fn is_refused(event: &Q2ServerEvent) -> bool {
        matches!(
            event,
            Q2ServerEvent::Download {
                bytes: None,
                percent: 0
            }
        )
    }

    #[test]
    fn peer_refuses_bad_requests_and_pumps_blocks() {
        let mut peer = Q2PeerDownload::new();
        let mut denied = ScriptDownloads {
            allowed: false,
            bytes: vec![1, 2, 3],
        };
        let event = peer
            .begin(&mut denied, "maps/x.bsp", None)
            .expect("begin")
            .expect("event");
        assert!(is_refused(&event));
        let mut allowed = ScriptDownloads {
            allowed: true,
            bytes: vec![7; 2500],
        };
        // `atoi` yields 0 for non-numeric text (donor behavior), so only a
        // negative offset refuses here.
        let event = peer
            .begin(&mut allowed, "maps/x.bsp", Some("-5"))
            .expect("begin")
            .expect("event");
        assert!(is_refused(&event));
        let mut allowed = ScriptDownloads {
            allowed: true,
            bytes: vec![7; 2500],
        };
        let mut event = peer
            .begin(&mut allowed, "maps/x.bsp", Some("100"))
            .expect("begin")
            .expect("first block");
        let mut blocks = 0;
        loop {
            blocks += 1;
            match event {
                Q2ServerEvent::Download {
                    bytes: Some(_),
                    percent,
                } => assert!(percent <= 100),
                other => panic!("unexpected event: {other:?}"),
            }
            match peer.next().expect("next") {
                Some(next) => event = next,
                None => break,
            }
        }
        assert!(blocks >= 2);
        assert!(peer.revision() > 0);
    }

    type ReceiverFixture = (
        MountedContent,
        RemoteContentRoots,
        Rc<RefCell<Vec<String>>>,
        Rc<RefCell<Vec<String>>>,
        PathBuf,
    );

    fn receiver_fixture(name: &str) -> ReceiverFixture {
        let dir = scratch_dir(name);
        let roots = RemoteContentRoots {
            base_write_root: dir.join("base"),
            write_root: dir.join("game"),
            selection: remote_content_selection(RemoteContentBase::Q2ClassicBaseq2, "").expect("selection"),
            content_directory: "baseq2".to_string(),
            loose_root: None,
            corpus_root: dir.join("corpus"),
        };
        (
            empty_mounts(),
            roots,
            Rc::new(RefCell::new(Vec::new())),
            Rc::new(RefCell::new(Vec::new())),
            dir,
        )
    }

    #[test]
    fn receiver_prepares_native_download_and_pumps_it() {
        let (mounts, roots, commands, prints, dir) = receiver_fixture("native");
        let commands_out = Rc::clone(&commands);
        let prints_out = Rc::clone(&prints);
        let roots_out = roots.clone();
        let mut receiver = Q2DownloadReceiver::new(Q2DownloadReceiverOptions {
            mounts: &mounts,
            content: Box::new(move || roots_out.clone()),
            command: Box::new(move |text| commands_out.borrow_mut().push(text.to_string())),
            print: Box::new(move |text| prints_out.borrow_mut().push(text.to_string())),
            refresh_packages: None,
            permission: None,
            make_http_client: Box::new(|| Box::new(NullHttpClient)),
        });
        let layout = q2_application_layout(ProtocolIdentity::Q2Classic).expect("layout");
        let state = game_state("", vec![(layout.models + 1, "maps/test.bsp".to_string())]);
        let preparation = receiver.prepare(&state).expect("prepare");
        assert_eq!(preparation, Q2DownloadPreparation::Waiting);
        assert_eq!(commands.borrow().as_slice(), ["download maps/test.bsp"]);
        let progress = receiver.progress();
        assert_eq!(progress.len(), 1);
        assert_eq!(progress[0].transport, ClientDownloadTransport::Native);
        assert_eq!(progress[0].phase, ClientDownloadPhase::Running);
        let outcome = receiver
            .receive(&Q2DownloadBlock {
                percent: 50,
                bytes: Some(vec![1, 2, 3]),
            })
            .expect("receive");
        assert_eq!(outcome, Q2DownloadOutcome::Waiting);
        assert_eq!(commands.borrow().as_slice(), ["download maps/test.bsp", "nextdl"]);
        let outcome = receiver
            .receive(&Q2DownloadBlock {
                percent: 100,
                bytes: Some(vec![4, 5]),
            })
            .expect("receive");
        assert_eq!(outcome, Q2DownloadOutcome::Complete);
        let outcome = receiver
            .receive(&Q2DownloadBlock {
                percent: 100,
                bytes: Some(vec![9]),
            })
            .expect("receive");
        assert_eq!(outcome, Q2DownloadOutcome::Unsolicited);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn receiver_rejects_foreign_game_and_retries_cancel() {
        let (mounts, mut roots, commands, prints, dir) = receiver_fixture("cancel");
        roots.selection = remote_content_selection(RemoteContentBase::Q2ClassicBaseq2, "ctf").expect("selection");
        let commands_out = Rc::clone(&commands);
        let prints_out = Rc::clone(&prints);
        let roots_out = roots.clone();
        let mut receiver = Q2DownloadReceiver::new(Q2DownloadReceiverOptions {
            mounts: &mounts,
            content: Box::new(move || roots_out.clone()),
            command: Box::new(move |text| commands_out.borrow_mut().push(text.to_string())),
            print: Box::new(move |text| prints_out.borrow_mut().push(text.to_string())),
            refresh_packages: None,
            permission: None,
            make_http_client: Box::new(|| Box::new(NullHttpClient)),
        });
        let layout = q2_application_layout(ProtocolIdentity::Q2Classic).expect("layout");
        let state = game_state("", vec![(layout.models + 1, "maps/test.bsp".to_string())]);
        let error = receiver.prepare(&state).expect_err("foreign game");
        assert_eq!(error.to_string(), "Q2 server game differs from prepared content");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn receiver_cancel_and_retry_cycle() {
        let (mounts, roots, commands, prints, dir) = receiver_fixture("retry");
        let commands_out = Rc::clone(&commands);
        let prints_out = Rc::clone(&prints);
        let roots_out = roots.clone();
        let mut receiver = Q2DownloadReceiver::new(Q2DownloadReceiverOptions {
            mounts: &mounts,
            content: Box::new(move || roots_out.clone()),
            command: Box::new(move |text| commands_out.borrow_mut().push(text.to_string())),
            print: Box::new(move |text| prints_out.borrow_mut().push(text.to_string())),
            refresh_packages: None,
            permission: None,
            make_http_client: Box::new(|| Box::new(NullHttpClient)),
        });
        let layout = q2_application_layout(ProtocolIdentity::Q2Classic).expect("layout");
        let state = game_state("", vec![(layout.models + 1, "maps/test.bsp".to_string())]);
        assert_eq!(
            receiver.prepare(&state).expect("prepare"),
            Q2DownloadPreparation::Waiting
        );
        receiver.cancel();
        assert_eq!(
            receiver.prepare(&state).expect("prepare"),
            Q2DownloadPreparation::Canceled
        );
        receiver.retry();
        let outcome = receiver
            .receive(&Q2DownloadBlock {
                percent: 0,
                bytes: None,
            })
            .expect("receive");
        assert_eq!(outcome, Q2DownloadOutcome::Complete);
        assert_eq!(
            receiver.prepare(&state).expect("prepare"),
            Q2DownloadPreparation::Waiting
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
