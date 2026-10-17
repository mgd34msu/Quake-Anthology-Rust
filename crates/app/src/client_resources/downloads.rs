use super::*;
use qa_content::vfs::{MountId, PreparedArchive, Vfs};
use qa_core::{sys_events::SeatId, text::FixedText};
use qa_network::{channel::TransmitError, downloads::Chunk};
use qa_platform::files::StagedFile;
use std::{fmt::Write, sync::Arc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DownloadConfig {
    game: MountId,
    skins: MountId,
    maximum_bytes: u64,
    buffer_bytes: usize,
}

#[derive(Clone)]
struct Request {
    remote: FixedText<{ qa_network::downloads::REQUEST_NAME_BYTES }>,
    local: Arc<FixedText<{ qa_core::names::MAX_PATH_BYTES + 1 }>>,
    mount: MountId,
    purpose: Purpose,
}
impl Request {
    fn name<const N: usize>(
        raw: &[u8],
        role: qa_network::downloads::PathRole,
    ) -> Result<FixedText<N>, String> {
        qa_network::downloads::validate_path(raw, role)
            .map_err(|_| "invalid native download path")?;
        let raw = std::str::from_utf8(raw).map_err(|_| "download path is not UTF-8")?;
        let canonical = qa_core::names::canonical_path(raw);
        let mut name = FixedText::default();
        name.set(&canonical).map_err(|_| "download path capacity")?;
        Ok(name)
    }
    fn prepare_package(
        &self,
        file: &mut StagedFile,
    ) -> Result<Option<Arc<PreparedArchive>>, String> {
        let Some(kind) = qa_formats::archive::ArchiveKind::from_path(self.local.as_bytes()) else {
            return Ok(None);
        };
        let reader = file
            .reader()
            .map_err(|error| format!("download inspection: {error}"))?;
        let package = PreparedArchive::read(Arc::new(reader))
            .map_err(|error| format!("download package: {error}"))?;
        if package.archive().kind != kind {
            return Err("download package format differs from its suffix".into());
        }
        if let Purpose::Package(expected) = self.purpose
            && package
                .archive()
                .q3_checksums()
                .is_none_or(|checksums| checksums.checksum() != expected)
        {
            return Err("download package differs from the server reference".into());
        }
        Ok(Some(Arc::new(package)))
    }
}
impl std::fmt::Display for Request {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        out.write_str(self.remote.as_str())?;
        if self.remote.as_bytes() != self.local.as_bytes() {
            write!(out, " -> {}", self.local.as_str())?;
        }
        Ok(())
    }
}
#[derive(Clone, Copy)]
enum Purpose {
    Manual,
    Skin,
    Startup,
    Package(u32),
}

struct PackagePass {
    requests: Box<[Request]>,
    next: Option<usize>,
    paused: bool,
}

struct SkinPass {
    stamp: Option<[u64; 4]>,
    cursor: usize,
    attempted: qa_core::bits::BitSet,
    paused: bool,
    complete: bool,
}

#[derive(Clone, Copy)]
enum Sending {
    Request,
    Next,
    Awaiting,
}

enum Transfer {
    Idle,
    Active {
        request: Request,
        file: StagedFile,
        percent: u8,
        sending: Sending,
    },
    Paused {
        request: Request,
        draining: bool,
        retry: bool,
    },
    Published {
        request: Request,
        bytes: u64,
        package: Option<Arc<PreparedArchive>>,
    },
    RefreshFailed {
        request: Request,
        bytes: u64,
        package: Option<Arc<PreparedArchive>>,
    },
    Finished {
        request: Request,
        bytes: u64,
    },
    Failed(Request),
}

pub(crate) struct ClientDownload {
    config: Option<DownloadConfig>,
    transfer: Transfer,
    permissions: Option<DownloadViews>,
    skin_pass: Option<SkinPass>,
    packages: Option<PackagePass>,
    blocks: Option<BlockProgress>,
    progress_dirty: bool,
}

#[derive(Default)]
struct BlockProgress {
    received: i32,
    queued: i32,
    total: Option<u32>,
    stop: bool,
    done: bool,
}

#[derive(Clone, Copy)]
enum Pending {
    Request,
    Next,
    Block(i32),
    Stop,
    Done,
}
impl Pending {
    fn text(self, transfer: &Transfer) -> Option<FixedText<80>> {
        let mut text = FixedText::default();
        match self {
            Self::Request => {
                let Transfer::Active { request, .. } = transfer else {
                    return None;
                };
                write!(text, "download {}", request.remote.as_str()).ok()?;
            }
            Self::Next => text.set("nextdl").ok()?,
            Self::Block(number) => write!(text, "nextdl {number}").ok()?,
            Self::Stop => text.set("stopdl").ok()?,
            Self::Done => text.set("donedl").ok()?,
        }
        Some(text)
    }
}

struct DownloadViews {
    protocol: Protocol,
    allow: qa_console::cvars::View,
    categories: Option<[qa_console::cvars::View; 4]>,
    progress: Option<ProgressViews>,
}

#[derive(Clone, Copy)]
pub(crate) struct ProgressViews {
    name: qa_console::cvars::View,
    size: qa_console::cvars::View,
    count: qa_console::cvars::View,
    time: qa_console::cvars::View,
}
impl ProgressViews {
    pub(crate) fn bind(cvars: &qa_console::cvars::Cvars, seat: SeatId) -> Result<Self, String> {
        let context = qa_console::views::Context {
            source: RuleSetId::Quake3,
            seat,
            ..Default::default()
        };
        let [name, size, count, time] = [
            "cl_downloadName",
            "cl_downloadSize",
            "cl_downloadCount",
            "cl_downloadTime",
        ]
        .map(|name| {
            cvars
                .bind(name, context)
                .ok_or_else(|| format!("missing native progress {name}"))
        });
        Ok(Self {
            name: name?,
            size: size?,
            count: count?,
            time: time?,
        })
    }
    fn number(
        view: qa_console::cvars::View,
        value: f32,
        cvars: &mut qa_console::cvars::Cvars,
    ) -> Result<(), String> {
        let mut text = FixedText::<64>::default();
        qa_core::text::native_number(&mut text, f64::from(value), 0, 6)
            .map_err(|_| "download number capacity")?;
        cvars
            .force_write(view, text.as_str())
            .map_err(|error| format!("download progress: {error:?}"))
    }
    pub(crate) fn rejected(
        self,
        size: i32,
        cvars: &mut qa_console::cvars::Cvars,
    ) -> Result<(), String> {
        Self::number(self.size, size as f32, cvars)
    }
}

impl ClientResources {
    pub(crate) fn receive_download(
        &mut self,
        protocol: Protocol,
        chunk: Result<Chunk<'_>, qa_network::downloads::Error>,
        vfs: &Vfs,
        cvars: &qa_console::cvars::Cvars,
    ) -> Result<(), String> {
        if matches!(chunk, Ok(Chunk::Block { .. })) && self.downloads.is_none() {
            // A control-only owner needs no writable root. Its retained stopdl
            // is flushed after ingress has restored the connection.
            self.downloads = Some(ClientDownload::new(None, None, protocol));
        }
        let downloads = self
            .downloads
            .as_mut()
            .ok_or("unsolicited download with no configured receiver")?;
        match chunk {
            Ok(chunk) => downloads.receive(protocol, chunk, vfs, cvars),
            Err(error) => {
                downloads.reject();
                Err(format!("malformed native download: {error:?}"))
            }
        }
    }

    pub(crate) fn retry_published_file(&mut self, path: &[u8]) {
        let matches = |candidate: &[u8]| {
            candidate
                .iter()
                .copied()
                .map(qa_core::names::path_byte)
                .eq(path.iter().copied().map(qa_core::names::path_byte))
        };
        for index in self.state.layout.resources[0].range() {
            if self
                .state
                .configs
                .config(index)
                .is_some_and(|row| matches(row.text))
            {
                self.state.models.config_received(index);
            }
        }
        for index in self.state.layout.players.range() {
            let Some(row) = self.state.configs.config(index) else {
                continue;
            };
            if (0..5).any(|part| {
                self.state
                    .models
                    .player_download_path(row.text, part)
                    .is_some_and(|name| matches(name.as_bytes()))
            }) {
                self.state.models.config_received(index);
            }
        }
    }

    fn next_startup_file(
        &mut self,
        vfs: &Vfs,
        mut models: Option<&mut crate::model_assets::ModelAssets>,
    ) -> Result<Option<FixedText<{ qa_network::downloads::REQUEST_NAME_BYTES }>>, String> {
        use super::startup::{Q2FileCursor, ResourceKind};
        loop {
            let mut name = FixedText::<64>::default();
            if let Some((kind, ordinal, end)) = self.startup.file_cursor() {
                if ordinal > end {
                    self.startup.files_complete();
                    return Ok(None);
                }
                let path = kind
                    .range(self.state.layout)
                    .get(ordinal as usize)
                    .and_then(|row| self.state.configs.config(row))
                    .ok_or("QW resource list row missing")?
                    .text;
                if kind == ResourceKind::Sound {
                    if let Some(path) = qa_audio::sound_path(path, RuleSetId::QuakeWorld)
                        .map_err(|_| "invalid QW sound path")?
                    {
                        name.set(path.as_str())
                            .map_err(|_| "QW resource path capacity")?;
                    }
                } else if !path.starts_with(b"*") {
                    name.set_bytes(path)
                        .map_err(|_| "QW resource path capacity")?;
                }
                self.startup.advance_file();
            } else if let Some(cursor) = self.startup.q2().and_then(|state| state.file_cursor()) {
                let layout = self.state.layout;
                let config = |range: qa_core::primitives::IndexSpan, ordinal: u32| {
                    range
                        .get(ordinal as usize)
                        .and_then(|row| self.state.configs.config(row))
                        .map(|row| row.text)
                        .filter(|text| !text.is_empty())
                };
                let next = match cursor {
                    Q2FileCursor::Models { ordinal, image } => {
                        if ordinal >= layout.resources[0].count {
                            Some(Q2FileCursor::Sounds(1))
                        } else {
                            let next_model = Q2FileCursor::Models {
                                ordinal: ordinal + 1,
                                image: None,
                            };
                            match config(layout.resources[0], ordinal)
                                .filter(|path| !matches!(path.first(), Some(b'*' | b'#')))
                            {
                                Some(path) => {
                                    if let Some(image) = image {
                                        let Some(owner) = models.as_deref_mut() else {
                                            return Ok(None);
                                        };
                                        match owner.source_image(vfs, path, image)? {
                                            Some(path) if !path.is_empty() => {
                                                name.set_bytes(path)
                                                    .map_err(|_| "Q2 model image path capacity")?;
                                                Some(Q2FileCursor::Models {
                                                    ordinal,
                                                    image: Some(image + 1),
                                                })
                                            }
                                            _ => Some(next_model),
                                        }
                                    } else {
                                        name.set_bytes(path)
                                            .map_err(|_| "Q2 model path capacity")?;
                                        Some(if ordinal == 1 {
                                            next_model
                                        } else {
                                            Q2FileCursor::Models {
                                                ordinal,
                                                image: Some(0),
                                            }
                                        })
                                    }
                                }
                                None => Some(next_model),
                            }
                        }
                    }
                    Q2FileCursor::Sounds(ordinal) => {
                        if ordinal >= layout.resources[1].count {
                            Some(Q2FileCursor::Images(1))
                        } else {
                            if let Some(raw) = config(layout.resources[1], ordinal)
                                && let Some(path) = qa_audio::sound_path(raw, layout.rules)
                                    .map_err(|_| "invalid Q2 sound path")?
                            {
                                name.set(path.as_str())
                                    .map_err(|_| "Q2 sound path capacity")?;
                            }
                            Some(Q2FileCursor::Sounds(ordinal + 1))
                        }
                    }
                    Q2FileCursor::Images(ordinal) => {
                        if ordinal >= layout.resources[2].count {
                            Some(Q2FileCursor::Players { slot: 0, part: 0 })
                        } else {
                            if let Some(raw) = config(layout.resources[2], ordinal) {
                                if let Some(raw) =
                                    raw.strip_prefix(b"/").or_else(|| raw.strip_prefix(b"\\"))
                                {
                                    name.set_bytes(raw).map_err(|_| "Q2 image path capacity")?;
                                } else {
                                    name.set("pics/")
                                        .and_then(|()| name.append_bytes(raw))
                                        .and_then(|()| name.append_bytes(b".pcx"))
                                        .map_err(|_| "Q2 image path capacity")?;
                                }
                            }
                            Some(Q2FileCursor::Images(ordinal + 1))
                        }
                    }
                    Q2FileCursor::Players { slot, part } => {
                        if slot >= layout.players.count {
                            Some(Q2FileCursor::Sky(0))
                        } else {
                            if let Some(raw) = config(layout.players, slot)
                                && let Some(path) =
                                    self.state.models.player_download_path(raw, part)
                            {
                                name = path;
                            }
                            Some(if part == 4 {
                                Q2FileCursor::Players {
                                    slot: slot + 1,
                                    part: 0,
                                }
                            } else {
                                Q2FileCursor::Players {
                                    slot,
                                    part: part + 1,
                                }
                            })
                        }
                    }
                    Q2FileCursor::Sky(index) => {
                        if index >= 12 {
                            Some(Q2FileCursor::Textures(0))
                        } else {
                            if let Some(sky) = self
                                .state
                                .configs
                                .config(2)
                                .filter(|row| !row.text.is_empty())
                            {
                                let face =
                                    qa_render::sky::CubeFace::ALL[usize::from(index / 2)].suffix();
                                let mut stem = FixedText::<64>::default();
                                stem.set("env/")
                                    .and_then(|()| stem.append_bytes(sky.text))
                                    .and_then(|()| stem.append_bytes(face.as_bytes()))
                                    .map_err(|_| "Q2 sky path capacity")?;
                                let mut tga = stem.clone();
                                tga.append_bytes(b".tga")
                                    .map_err(|_| "Q2 sky path capacity")?;
                                let mut pcx = stem.clone();
                                pcx.append_bytes(b".pcx")
                                    .map_err(|_| "Q2 sky path capacity")?;
                                if vfs.open(tga.as_bytes()).is_none()
                                    && vfs.open(pcx.as_bytes()).is_none()
                                {
                                    name = if index % 2 == 0 { tga } else { pcx };
                                }
                            }
                            Some(Q2FileCursor::Sky(index + 1))
                        }
                    }
                    Q2FileCursor::Textures(index) => {
                        let mut path = FixedText::<64>::default();
                        let raw = config(layout.resources[0], 1)
                            .ok_or("Q2 server supplied no world model")?;
                        path.set_bytes(raw).map_err(|_| "Q2 world path capacity")?;
                        let state = self.startup.q2_mut().ok_or("Q2 startup owner changed")?;
                        if state.world_input.is_none() {
                            state.world_input = Some(crate::map::read(vfs, path.as_str())?);
                        }
                        let input = state.world_input.as_ref().ok_or("Q2 world input missing")?;
                        if let Some(texture) = input.texture_names()?.nth(index as usize) {
                            let texture = texture
                                .map_err(|error| format!("Q2 texture dependency: {error:?}"))?;
                            if !texture.is_empty() {
                                name.set("textures/")
                                    .and_then(|()| name.append_bytes(texture))
                                    .and_then(|()| name.append_bytes(b".wal"))
                                    .map_err(|_| "Q2 texture path capacity")?;
                            }
                            Some(Q2FileCursor::Textures(index + 1))
                        } else {
                            None
                        }
                    }
                };
                if let Some(state) = self.startup.q2_mut() {
                    state.advance_file(next);
                }
                if next.is_none() {
                    return Ok(None);
                }
            } else {
                return Ok(None);
            }
            if !name.as_bytes().is_empty() {
                return Ok(Some(name));
            }
        }
    }
}
impl DownloadViews {
    fn bind(
        cvars: &qa_console::cvars::Cvars,
        protocol: Protocol,
        seat: SeatId,
    ) -> Result<Option<Self>, String> {
        if !matches!(
            protocol,
            Protocol::Quake2_34 | Protocol::Quake2Repro1038 | Protocol::Quake3_68
        ) {
            return Ok(None);
        }
        let context = qa_console::views::Context {
            source: protocol.resource_layout().rules,
            side: qa_console::catalog::Scope::Client,
            seat,
            ..Default::default()
        };
        let allow = cvars
            .bind("cl_allowDownload", context)
            .ok_or("missing CLIENT download permission")?;
        if protocol == Protocol::Quake3_68 {
            return Ok(Some(Self {
                protocol,
                allow,
                categories: None,
                progress: Some(ProgressViews::bind(cvars, seat)?),
            }));
        }
        let [maps, models, sounds, players] = [
            "allow_download_maps",
            "allow_download_models",
            "allow_download_sounds",
            "allow_download_players",
        ]
        .map(|name| {
            cvars
                .bind(name, context)
                .ok_or("missing CLIENT download category")
        });
        Ok(Some(Self {
            protocol,
            allow,
            categories: Some([maps?, models?, sounds?, players?]),
            progress: None,
        }))
    }
    fn allows(&self, name: &[u8], cvars: &qa_console::cvars::Cvars) -> bool {
        if cvars.numeric(self.allow).unwrap_or(0.0) <= 0.0 {
            return false;
        }
        let Some(categories) = self.categories else {
            return true;
        };
        let allowed = |index| cvars.numeric(categories[index]).unwrap_or(0.0) != 0.0;
        use qa_network::downloads::AssetCategory;
        match qa_network::downloads::category(self.protocol, name) {
            AssetCategory::Package => (0..4).all(allowed),
            AssetCategory::Map | AssetCategory::Texture => allowed(0),
            AssetCategory::Model => allowed(1),
            AssetCategory::Sound => allowed(2),
            AssetCategory::Skin | AssetCategory::Player => allowed(3),
            AssetCategory::Picture | AssetCategory::Other => true,
        }
    }
}

impl ClientDownload {
    fn begin_packages(
        &mut self,
        info: &[u8],
        vfs: &Vfs,
        cvars: &qa_console::cvars::Cvars,
    ) -> Result<bool, String> {
        let loaded: Vec<_> = vfs
            .mounts()
            .filter_map(|mount| vfs.q3_package_checksums(mount.id))
            .map(|checksums| checksums.checksum())
            .collect();
        let mut references = qa_network::downloads::ReferencedPackages::new(info);
        let mut requests: Vec<Request> = Vec::new();
        while let Some(reference) = references
            .next()
            .map_err(|error| format!("server package references: {error:?}"))?
        {
            let Some(name) = reference.name.filter(|name| !name.is_empty()) else {
                continue;
            };
            if loaded.contains(&reference.checksum)
                || qa_network::downloads::q3_stock_package(name).is_some()
            {
                continue;
            }
            let mut remote = FixedText::<{ qa_network::downloads::REQUEST_NAME_BYTES }>::default();
            remote
                .append_bytes(name)
                .and_then(|()| remote.append_bytes(b".pk3"))
                .map_err(|_| "server package exceeds native request capacity")?;
            let remote = Request::name::<{ qa_network::downloads::REQUEST_NAME_BYTES }>(
                remote.as_bytes(),
                qa_network::downloads::PathRole::Request,
            )?;
            if let Some(previous) = requests
                .iter()
                .find(|request| request.remote.as_bytes() == remote.as_bytes())
            {
                if !matches!(previous.purpose, Purpose::Package(sum) if sum == reference.checksum) {
                    return Err("server references conflicting versions of one package".into());
                }
                continue;
            }
            if !self.allowed(remote.as_bytes(), cvars) {
                continue;
            }
            let config = self.config.ok_or("package download has no writable root")?;
            let root_name = vfs
                .directory_path(config.game)
                .and_then(|path| path.file_name())
                .and_then(|name| name.to_str())
                .ok_or("package download root retired")?;
            let prefix = format!("{}/", qa_core::names::canonical_path(root_name));
            let local = remote
                .as_str()
                .strip_prefix(&prefix)
                .unwrap_or(remote.as_str());
            let mut destination = FixedText::<{ qa_core::names::MAX_PATH_BYTES + 1 }>::default();
            if vfs.open(local.as_bytes()).is_some() {
                // FS_ComparePaks' native wire-checksum suffix protects an
                // existing package. It is never a content/cache identity.
                write!(
                    destination,
                    "{}.{:08x}.pk3",
                    &local[..local.len() - 4],
                    reference.checksum
                )
                .map_err(|_| "package destination capacity")?;
            } else {
                destination
                    .set(local)
                    .map_err(|_| "package destination capacity")?;
            }
            requests.push(self.request(
                remote.as_bytes(),
                Some(destination.as_bytes()),
                vfs,
                Purpose::Package(reference.checksum),
            )?);
        }
        let needed = !requests.is_empty();
        self.packages = Some(PackagePass {
            requests: requests.into_boxed_slice(),
            next: needed.then_some(0),
            paused: false,
        });
        Ok(needed)
    }

    fn advance_packages(
        &mut self,
        vfs: &Vfs,
        cvars: &qa_console::cvars::Cvars,
    ) -> Result<bool, String> {
        if !self.resource_idle() {
            return Ok(false);
        }
        let Some(pass) = self.packages.as_mut().filter(|pass| !pass.paused) else {
            return Ok(false);
        };
        let Some(next) = pass.next else {
            return Ok(false);
        };
        let Some(request) = pass.requests.get(next).cloned() else {
            pass.next = None;
            if let Some(blocks) = &mut self.blocks {
                blocks.done = true;
            }
            return Ok(true);
        };
        pass.next = Some(next + 1);
        if !self.allowed_request(&request, cvars) {
            if let Some(pass) = &mut self.packages {
                pass.paused = true;
                pass.next = Some(next);
            }
            return Err("package download disabled by local CLIENT policy".into());
        }
        if let Err(error) = self.open(request, vfs) {
            if let Some(pass) = &mut self.packages {
                pass.paused = true;
            }
            return Err(error);
        }
        Ok(false)
    }

    fn configured(&self) -> bool {
        self.config.is_some()
    }

    pub(crate) fn receiving(&self) -> bool {
        matches!(self.transfer, Transfer::Active { .. })
    }

    fn new(
        config: Option<DownloadConfig>,
        permissions: Option<DownloadViews>,
        protocol: Protocol,
    ) -> Self {
        Self {
            config,
            transfer: Transfer::Idle,
            permissions,
            packages: None,
            blocks: (protocol == Protocol::Quake3_68).then(BlockProgress::default),
            progress_dirty: true,
            skin_pass: (protocol == Protocol::QuakeWorld28).then(|| SkinPass {
                stamp: None,
                cursor: 0,
                attempted: qa_core::bits::BitSet::empty(
                    protocol.resource_layout().players.count as usize * 2,
                ),
                paused: false,
                complete: false,
            }),
        }
    }

    pub(crate) fn close(&mut self) {
        self.progress_dirty |= !matches!(self.transfer, Transfer::Idle);
        self.transfer = Transfer::Idle;
        self.packages = None;
        if let Some(blocks) = &mut self.blocks {
            *blocks = BlockProgress::default();
        }
        if let Some(pass) = &mut self.skin_pass {
            pass.stamp = None;
            pass.cursor = 0;
            pass.attempted.clear();
            pass.paused = false;
            pass.complete = false;
        }
    }

    pub(crate) fn reject(&mut self) {
        self.progress_dirty = true;
        self.stop_blocks();
        let previous = std::mem::replace(&mut self.transfer, Transfer::Idle);
        self.transfer = match previous {
            Transfer::Active { request, .. } | Transfer::Paused { request, .. } => {
                Transfer::Failed(request)
            }
            other => other,
        };
    }

    fn start(
        &mut self,
        remote: &[u8],
        local: Option<&[u8]>,
        vfs: &Vfs,
        cvars: &qa_console::cvars::Cvars,
        purpose: Purpose,
    ) -> Result<(), String> {
        if !matches!(purpose, Purpose::Package(_))
            && self
                .packages
                .as_ref()
                .is_some_and(|pass| pass.next.is_some())
        {
            return Err("automatic package group is still pending".into());
        }
        if matches!(
            self.transfer,
            Transfer::Active { .. }
                | Transfer::Paused { draining: true, .. }
                | Transfer::Published { .. }
        ) {
            return Err("a download or its outstanding reply is still active".into());
        }
        if self.pending().is_some() {
            return Err("download acknowledgement is still pending".into());
        }
        let request = self.request(remote, local, vfs, purpose)?;
        if !self.allowed_request(&request, cvars) {
            return Err("download disabled by local CLIENT policy".into());
        }
        self.open(request, vfs)
    }

    fn request(
        &self,
        remote: &[u8],
        local: Option<&[u8]>,
        vfs: &Vfs,
        purpose: Purpose,
    ) -> Result<Request, String> {
        let remote = Request::name::<{ qa_network::downloads::REQUEST_NAME_BYTES }>(
            remote,
            qa_network::downloads::PathRole::Request,
        )?;
        let local = Arc::new(Request::name::<{ qa_core::names::MAX_PATH_BYTES + 1 }>(
            local.unwrap_or_else(|| remote.as_bytes()),
            qa_network::downloads::PathRole::Destination,
        )?);
        if vfs.open(local.as_bytes()).is_some() {
            return Err("download file is already available in mounted content".into());
        }
        let config = self
            .config
            .ok_or("downloads have no configured writable roots")?;
        let mount = if local.as_bytes().starts_with(b"skins/") {
            config.skins
        } else {
            config.game
        };
        Ok(Request {
            remote,
            local,
            mount,
            purpose,
        })
    }

    fn allowed_request(&self, request: &Request, cvars: &qa_console::cvars::Cvars) -> bool {
        self.allowed(request.remote.as_bytes(), cvars)
            && self.allowed(request.local.as_bytes(), cvars)
    }

    fn allowed(&self, name: &[u8], cvars: &qa_console::cvars::Cvars) -> bool {
        self.permissions
            .as_ref()
            .is_none_or(|views| views.allows(name, cvars))
    }

    fn open(&mut self, request: Request, vfs: &Vfs) -> Result<(), String> {
        let config = self
            .config
            .ok_or("downloads have no configured writable roots")?;
        let result = (|| {
            let root = vfs
                .directory_path(request.mount)
                .ok_or("download directory is no longer mounted")?;
            let file = StagedFile::create(
                root,
                request.local.as_str(),
                config.maximum_bytes,
                config.buffer_bytes,
            )
            .map_err(|error| format!("download staging: {error}"))?;
            Ok::<_, String>(file)
        })();
        match result {
            Ok(file) => {
                self.progress_dirty = true;
                if let Some(blocks) = &mut self.blocks {
                    *blocks = BlockProgress::default();
                }
                self.transfer = Transfer::Active {
                    request,
                    file,
                    percent: 0,
                    sending: Sending::Request,
                };
                Ok(())
            }
            Err(error) => {
                self.transfer = Transfer::Failed(request);
                Err(error)
            }
        }
    }

    fn stop_blocks(&mut self) {
        if let Some(blocks) = &mut self.blocks {
            blocks.stop = true;
            blocks.done = false;
            blocks.queued = blocks.received;
        }
    }

    fn stop(&mut self) {
        self.progress_dirty = true;
        self.stop_blocks();
        if let Some(pass) = &mut self.packages {
            pass.paused = true;
        }
        if let Some(pass) = &mut self.skin_pass {
            pass.paused = true;
        }
        let previous = std::mem::replace(&mut self.transfer, Transfer::Idle);
        self.transfer = match previous {
            Transfer::Active {
                request, sending, ..
            } => Transfer::Paused {
                request,
                draining: self.blocks.is_none() && matches!(sending, Sending::Awaiting),
                retry: false,
            },
            Transfer::Paused {
                request, draining, ..
            } => Transfer::Paused {
                request,
                draining,
                retry: false,
            },
            other => other,
        };
    }

    fn retry(&mut self, vfs: &Vfs, cvars: &qa_console::cvars::Cvars) -> Result<(), String> {
        if let Some(pass) = &mut self.packages {
            pass.paused = false;
        }
        if let Some(pass) = &mut self.skin_pass {
            pass.paused = false;
        }
        let previous = std::mem::replace(&mut self.transfer, Transfer::Idle);
        match previous {
            Transfer::Paused {
                request,
                draining: true,
                ..
            } => {
                self.transfer = Transfer::Paused {
                    request,
                    draining: true,
                    retry: true,
                };
                Ok(())
            }
            Transfer::Paused {
                request,
                draining: false,
                ..
            }
            | Transfer::Failed(request) => {
                match self.start(
                    request.remote.as_bytes(),
                    Some(request.local.as_bytes()),
                    vfs,
                    cvars,
                    request.purpose,
                ) {
                    Ok(()) => Ok(()),
                    Err(error) => {
                        self.transfer = Transfer::Failed(request);
                        Err(error)
                    }
                }
            }
            Transfer::RefreshFailed {
                request,
                bytes,
                package,
            } => {
                self.transfer = Transfer::Published {
                    request,
                    bytes,
                    package,
                };
                Ok(())
            }
            other => {
                self.transfer = other;
                Ok(())
            }
        }
    }

    fn advance_skins(
        &mut self,
        stamp: [u64; 4],
        count: usize,
        enabled: bool,
        vfs: &Vfs,
        cvars: &qa_console::cvars::Cvars,
        mut paths: impl FnMut(
            usize,
        ) -> Option<(
            FixedText<{ qa_network::downloads::REQUEST_NAME_BYTES }>,
            Option<FixedText<{ qa_network::downloads::REQUEST_NAME_BYTES }>>,
        )>,
    ) -> Result<(), String> {
        let Some(pass) = &mut self.skin_pass else {
            return Ok(());
        };
        let explicit_request = pass.stamp.is_some_and(|previous| previous[1] != stamp[1]);
        if pass.stamp != Some(stamp) {
            pass.stamp = Some(stamp);
            pass.cursor = 0;
            pass.attempted.clear();
            pass.complete = false;
        }
        if !enabled {
            pass.complete = true;
        }
        if explicit_request
            && matches!(
                self.transfer,
                Transfer::Failed(Request {
                    purpose: Purpose::Manual,
                    ..
                })
            )
        {
            self.transfer = Transfer::Idle;
        }
        if !enabled
            || pass.paused
            || !matches!(
                self.transfer,
                Transfer::Idle
                    | Transfer::Finished { .. }
                    | Transfer::Failed(Request {
                        purpose: Purpose::Skin | Purpose::Startup,
                        ..
                    })
            )
        {
            return Ok(());
        }
        let mut rejected = None;
        while self
            .skin_pass
            .as_ref()
            .is_some_and(|pass| pass.cursor < count)
        {
            let Some(pass) = &mut self.skin_pass else {
                break;
            };
            let cursor = pass.cursor;
            pass.cursor += 1;
            let Some((selected, fallback)) = paths(cursor / 2) else {
                continue;
            };
            let name = if cursor % 2 == 0 {
                selected
            } else {
                if vfs.open(selected.as_bytes()).is_some() {
                    continue;
                }
                let Some(fallback) = fallback else {
                    continue;
                };
                fallback
            };
            let duplicate = (0..cursor)
                .filter(|&prior| pass.attempted.contains(prior))
                .any(|prior| {
                    let Some((selected, fallback)) = paths(prior / 2) else {
                        return false;
                    };
                    let previous = if prior % 2 == 0 {
                        Some(selected)
                    } else {
                        fallback
                    };
                    previous.is_some_and(|previous| previous.as_bytes() == name.as_bytes())
                });
            if duplicate || vfs.open(name.as_bytes()).is_some() {
                continue;
            }
            pass.attempted.insert(cursor);
            match self.start(name.as_bytes(), None, vfs, cvars, Purpose::Skin) {
                Ok(()) => return rejected.map_or(Ok(()), Err),
                Err(error) => rejected = Some(error),
            }
        }
        if let Some(pass) = &mut self.skin_pass {
            pass.complete = true;
        }
        rejected.map_or(Ok(()), Err)
    }

    pub(crate) fn skins_done(&self) -> bool {
        self.skin_pass
            .as_ref()
            .is_none_or(|pass| pass.complete && !pass.paused)
            && self.resource_idle()
    }
    fn resource_idle(&self) -> bool {
        self.pending().is_none()
            && matches!(
                self.transfer,
                Transfer::Idle
                    | Transfer::Finished { .. }
                    | Transfer::Failed(Request {
                        purpose: Purpose::Skin | Purpose::Startup,
                        ..
                    })
            )
    }

    fn progress_views(&self) -> Option<ProgressViews> {
        self.permissions.as_ref()?.progress
    }
    fn flush_progress(
        &mut self,
        cvars: &mut qa_console::cvars::Cvars,
        time: qa_core::sys_events::EventTime,
    ) -> Result<(), String> {
        if !self.progress_dirty {
            return Ok(());
        }
        if let Some(views) = self.progress_views() {
            if matches!(
                self.transfer,
                Transfer::Active {
                    sending: Sending::Request,
                    ..
                }
            ) {
                ProgressViews::number(views.size, 0.0, cvars)?;
                ProgressViews::number(views.count, 0.0, cvars)?;
                ProgressViews::number(views.time, time.milliseconds() as i32 as f32, cvars)?;
            }
            let (name, count) = match &self.transfer {
                Transfer::Active { request, file, .. } => {
                    (request.remote.as_str(), Some(file.len()))
                }
                Transfer::Published { bytes, .. }
                | Transfer::Finished { bytes, .. }
                | Transfer::RefreshFailed { bytes, .. } => ("", Some(*bytes)),
                _ => ("", None),
            };
            cvars
                .force_write(views.name, name)
                .map_err(|error| format!("download progress: {error:?}"))?;
            if let Some(total) = self.blocks.as_ref().and_then(|blocks| blocks.total) {
                ProgressViews::number(views.size, total as f32, cvars)?;
            }
            if let Some(count) = count {
                ProgressViews::number(views.count, count as f32, cvars)?;
            }
        }
        self.progress_dirty = false;
        Ok(())
    }

    fn pending(&self) -> Option<Pending> {
        if let Some(blocks) = &self.blocks {
            if blocks.stop {
                return Some(Pending::Stop);
            }
            if blocks.queued < blocks.received {
                return Some(Pending::Block(blocks.queued));
            }
            if blocks.done {
                return Some(Pending::Done);
            }
        }
        let Transfer::Active { sending, .. } = self.transfer else {
            return None;
        };
        match sending {
            Sending::Request => Some(Pending::Request),
            Sending::Next => Some(Pending::Next),
            Sending::Awaiting => None,
        }
    }

    fn queued(&mut self, pending: Pending) {
        match pending {
            Pending::Block(number) => {
                if let Some(blocks) = &mut self.blocks {
                    blocks.queued = number + 1;
                }
            }
            Pending::Stop => {
                if let Some(blocks) = &mut self.blocks {
                    blocks.stop = false;
                }
            }
            Pending::Done => {
                if let Some(blocks) = &mut self.blocks {
                    blocks.done = false;
                }
            }
            Pending::Request | Pending::Next => {
                if let Transfer::Active { sending, .. } = &mut self.transfer {
                    *sending = Sending::Awaiting;
                }
            }
        }
    }

    pub(crate) fn receive(
        &mut self,
        protocol: Protocol,
        chunk: Chunk<'_>,
        vfs: &Vfs,
        cvars: &qa_console::cvars::Cvars,
    ) -> Result<(), String> {
        // CL_ParseDownload consumes native fields before comparing the block.
        // This includes repeated EOF after publication when its ACK was lost.
        if let (Chunk::Block { number, .. }, Some(blocks)) = (chunk, &self.blocks)
            && i32::from(number) != blocks.received
        {
            return Ok(());
        }
        let previous = std::mem::replace(&mut self.transfer, Transfer::Idle);
        match previous {
            Transfer::Paused {
                request,
                draining: true,
                retry,
            } => {
                if retry && self.allowed_request(&request, cvars) {
                    self.open(request, vfs)
                } else {
                    self.transfer = Transfer::Paused {
                        request,
                        draining: false,
                        retry: false,
                    };
                    Ok(())
                }
            }
            Transfer::Active {
                request,
                mut file,
                percent,
                sending: Sending::Awaiting,
            } => {
                let result = (|| {
                    if !self.allowed_request(&request, cvars) {
                        return Err("download disabled by local CLIENT policy".into());
                    }
                    if vfs.directory_path(request.mount).is_none() {
                        return Err("download directory retired".into());
                    }
                    let (bytes, received_percent, compressed, complete, sending) = match chunk {
                        Chunk::Sequential {
                            percent: received_percent,
                            bytes,
                            compressed,
                        } if self.blocks.is_none() => {
                            let bytes = bytes.ok_or_else(|| {
                                format!("server has no {}", request.remote.as_str())
                            })?;
                            if received_percent < percent || received_percent > 100 {
                                return Err("download progress regressed".into());
                            }
                            let empty = qa_network::downloads::sequential(protocol)
                                .is_some_and(|policy| policy.empty_completes())
                                && !compressed
                                && file.is_empty()
                                && bytes.is_empty();
                            (
                                bytes,
                                received_percent,
                                compressed,
                                received_percent == 100 || empty,
                                Sending::Next,
                            )
                        }
                        Chunk::Block { total, bytes, .. } => {
                            let blocks =
                                self.blocks.as_mut().ok_or("unexpected numbered download")?;
                            if let Some(total) = total {
                                blocks.total = Some(total);
                            }
                            let total = u64::from(blocks.total.ok_or("missing download size")?);
                            if self
                                .config
                                .is_none_or(|config| total > config.maximum_bytes)
                                || file.len() + bytes.len() as u64 > total
                            {
                                return Err("download exceeds declared or configured size".into());
                            }
                            let complete = bytes.is_empty();
                            if complete && file.len() != total {
                                return Err("incomplete download at EOF".into());
                            }
                            let percent = if complete {
                                100
                            } else {
                                ((file.len() + bytes.len() as u64) * 100 / total.max(1)).min(99)
                                    as u8
                            };
                            (bytes, percent, false, complete, Sending::Awaiting)
                        }
                        _ => return Err("unexpected download encoding".into()),
                    };
                    file.append(bytes)
                        .map_err(|error| format!("download write: {error}"))?;
                    if complete {
                        let package = request.prepare_package(&mut file)?;
                        let bytes = file
                            .publish()
                            .map_err(|error| format!("download publication: {error}"))?;
                        if let Some(blocks) = &mut self.blocks {
                            blocks.received += 1;
                        }
                        return Ok(Transfer::Published {
                            request: request.clone(),
                            bytes,
                            package,
                        });
                    }
                    if bytes.is_empty() && !compressed {
                        return Err("download made no progress".into());
                    }
                    if let Some(blocks) = &mut self.blocks {
                        blocks.received += 1;
                    }
                    Ok(Transfer::Active {
                        request: request.clone(),
                        file,
                        percent: received_percent,
                        sending,
                    })
                })();
                match result {
                    Ok(state) => {
                        self.progress_dirty = true;
                        self.transfer = state;
                        Ok(())
                    }
                    Err(error) => {
                        self.progress_dirty = true;
                        self.transfer = Transfer::Failed(request);
                        self.stop_blocks();
                        Err(error)
                    }
                }
            }
            other => {
                self.transfer = other;
                self.stop_blocks();
                Err("unsolicited download block".into())
            }
        }
    }

    fn status(&self, out: &mut impl Write) -> std::fmt::Result {
        match &self.transfer {
            Transfer::Idle => writeln!(out, "No active download."),
            Transfer::Active {
                request,
                file,
                percent,
                ..
            } => writeln!(out, "{}: {} bytes, {}%", request, file.len(), percent),
            Transfer::Paused {
                request,
                draining,
                retry,
            } => writeln!(
                out,
                "{}: paused, pending reply={}, retry={}",
                request, draining, retry
            ),
            Transfer::Published { request, bytes, .. } => writeln!(
                out,
                "{}: {} bytes published, content refresh pending",
                request, bytes
            ),
            Transfer::RefreshFailed { request, bytes, .. } => writeln!(
                out,
                "{}: {} bytes published, refresh failed; retrydownload retries refresh",
                request, bytes
            ),
            Transfer::Finished { request, bytes } => {
                writeln!(out, "{}: {} bytes complete", request, bytes)
            }
            Transfer::Failed(request) => {
                writeln!(out, "{}: failed; retrydownload retries", request)
            }
        }
    }
}

impl crate::host::FrameHost {
    pub(crate) fn close_client_downloads(&mut self, id: ClientId) {
        if let Some(downloads) = self
            .runtime
            .client_resources
            .get_mut(id.0 as usize)
            .and_then(Option::as_mut)
            .and_then(|resources| resources.downloads.as_mut())
        {
            downloads.close();
            if let Err(error) = downloads.flush_progress(&mut self.console.cvars, self.time) {
                let _ = self
                    .runtime
                    .server
                    .events
                    .console_formatted(format_args!("{error}\n"));
            }
        }
    }
    pub fn configure_client_downloads(
        &mut self,
        client: ClientId,
        game: MountId,
        skins: MountId,
        maximum_bytes: u64,
        buffer_bytes: usize,
    ) -> Result<(), String> {
        let config = DownloadConfig {
            game,
            skins,
            maximum_bytes,
            buffer_bytes,
        };
        self.validate_download_config(config)?;
        let connection = self
            .runtime
            .network
            .get_mut(client, qa_core::loopback::Endpoint::Client)
            .ok_or("no CLIENT download connection")?;
        let protocol = connection
            .commands
            .as_ref()
            .ok_or("missing native CLIENT commands")?
            .protocol;
        if protocol == Protocol::NetQuake15 {
            return Err("protocol has no native download service".into());
        }
        let resources = self
            .runtime
            .client_resources
            .get_mut(client.0 as usize)
            .and_then(Option::as_mut)
            .ok_or("no CLIENT resources")?;
        if let Some(existing) = &resources.downloads {
            if existing.config == Some(config) {
                return Ok(());
            }
            if matches!(
                existing.transfer,
                Transfer::Active { .. }
                    | Transfer::Paused { draining: true, .. }
                    | Transfer::Published { .. }
            ) {
                return Err("download roots cannot change during a transfer".into());
            }
        }
        let seat = self
            .runtime
            .snapshots
            .get(client.0 as usize)
            .and_then(Option::as_ref)
            .and_then(|binding| binding.seat())
            .ok_or("downloads require a local CLIENT seat")?;
        let permissions = DownloadViews::bind(&self.console.cvars, protocol, seat)?;
        resources.downloads = Some(ClientDownload::new(Some(config), permissions, protocol));
        Ok(())
    }

    pub fn configure_download_roots(
        &mut self,
        game: MountId,
        skins: MountId,
        maximum_bytes: u64,
        buffer_bytes: usize,
    ) -> Result<(), String> {
        let config = DownloadConfig {
            game,
            skins,
            maximum_bytes,
            buffer_bytes,
        };
        self.validate_download_config(config)?;
        self.download_defaults = Some(config);
        Ok(())
    }

    fn validate_download_config(&self, config: DownloadConfig) -> Result<(), String> {
        if self.runtime.vfs.directory_path(config.game).is_none()
            || self.runtime.vfs.directory_path(config.skins).is_none()
        {
            return Err(
                "download roots must be explicitly selected mounted content directories".into(),
            );
        }
        Ok(())
    }

    pub(crate) fn client_download_command(
        &mut self,
        seat: SeatId,
        command: usize,
        args: qa_console::command_text::Arguments<'_>,
    ) -> Result<(), String> {
        let client = self.local_clients[seat.index()].ok_or("no CLIENT for this seat")?;
        let connection = self
            .runtime
            .network
            .get_mut(client, qa_core::loopback::Endpoint::Client)
            .ok_or("no native CLIENT channel")?;
        if !connection.phase.is_connected() {
            return Err("download requires a connected CLIENT".into());
        }
        let protocol = connection
            .commands
            .as_ref()
            .ok_or("missing CLIENT commands")?
            .protocol;
        if protocol == Protocol::NetQuake15 {
            return Err("protocol has no native download service".into());
        }
        if self.runtime.client_resources[client.0 as usize]
            .as_ref()
            .is_some_and(|resources| {
                resources
                    .downloads
                    .as_ref()
                    .is_none_or(|downloads| !downloads.configured())
            })
        {
            let defaults = self
                .download_defaults
                .ok_or("downloads have no explicitly configured writable roots")?;
            self.configure_client_downloads(
                client,
                defaults.game,
                defaults.skins,
                defaults.maximum_bytes,
                defaults.buffer_bytes,
            )?;
        }
        let resources = self.runtime.client_resources[client.0 as usize]
            .as_mut()
            .ok_or("missing CLIENT resources")?;
        let downloads = resources
            .downloads
            .as_mut()
            .ok_or("downloads have no configured writable roots")?;
        match command {
            0 => {
                if !(2..=3).contains(&args.len()) {
                    return Err("usage: download <remote> [local]".into());
                }
                downloads.start(
                    args.get_bytes(1),
                    (args.len() == 3).then(|| args.get_bytes(2)),
                    &self.runtime.vfs,
                    &self.console.cvars,
                    Purpose::Manual,
                )?;
            }
            1 => {
                let mut text = FixedText::<{ qa_console::text::MAX_TEXT }>::default();
                downloads
                    .status(&mut text)
                    .map_err(|_| "download status capacity")?;
                let _ = self.runtime.server.events.print_level_bytes(
                    Some(qa_core::events::OutputTarget::Local(client)),
                    Some(client),
                    qa_core::primitives::PrintKind::Console,
                    qa_core::primitives::PrintKind::Console.default_level(),
                    text.as_bytes(),
                );
            }
            2 => downloads.stop(),
            3 => downloads.retry(&self.runtime.vfs, &self.console.cvars)?,
            _ => return Err("unknown download operation".into()),
        }
        self.flush_client_downloads();
        Ok(())
    }

    pub(crate) fn flush_client_downloads(&mut self) {
        for index in 0..self.runtime.client_resources.len() {
            let Some(resources) = &mut self.runtime.client_resources[index] else {
                continue;
            };
            let Some(downloads) = &mut resources.downloads else {
                continue;
            };
            if self.runtime.snapshots[index]
                .as_ref()
                .is_none_or(|binding| binding.presentation_closed())
            {
                downloads.close();
            }
            if let Err(error) = downloads.flush_progress(&mut self.console.cvars, self.time) {
                let _ = self
                    .runtime
                    .server
                    .events
                    .console_formatted(format_args!("{error}\n"));
            }
            if downloads.pending().is_none() {
                continue;
            }
            if let Transfer::Active { request, .. } = &downloads.transfer
                && !downloads.allowed_request(request, &self.console.cvars)
            {
                downloads.stop();
                continue;
            }
            let client = ClientId(index as u32);
            let Some(connection) = self
                .runtime
                .network
                .get_mut(client, qa_core::loopback::Endpoint::Client)
            else {
                downloads.close();
                continue;
            };
            let Some(commands) = connection.commands.as_mut() else {
                downloads.close();
                continue;
            };
            while let Some(pending) = downloads.pending() {
                let Some(command) = pending.text(&downloads.transfer) else {
                    break;
                };
                match connection
                    .channel
                    .queue_client_command(commands.protocol, command.as_bytes())
                {
                    Ok(_) => {
                        if matches!(pending, Pending::Request) {
                            commands.reset_download_stream();
                        }
                        downloads.queued(pending);
                    }
                    Err(TransmitError::Full) => break,
                    Err(_) => {
                        downloads.reject();
                        let _ = self
                            .runtime
                            .server
                            .events
                            .console_bytes(b"Download command rejected by native channel.\n");
                        break;
                    }
                }
            }
        }
    }

    pub(crate) fn advance_q3_package_downloads(&mut self) {
        use super::startup::Q3Phase;
        for index in 0..self.runtime.client_resources.len() {
            let client = ClientId(index as u32);
            let Some(resources) = self.runtime.client_resources[index].as_ref() else {
                continue;
            };
            if resources.startup.q3() != Some(Q3Phase::Downloading) {
                continue;
            }
            let configured = resources
                .downloads
                .as_ref()
                .is_some_and(ClientDownload::configured);
            if !configured && let Some(config) = self.download_defaults {
                if let Err(error) = self.configure_client_downloads(
                    client,
                    config.game,
                    config.skins,
                    config.maximum_bytes,
                    config.buffer_bytes,
                ) {
                    self.reject_client_startup(client, format_args!("{error}"));
                    continue;
                }
            }
            let Some(resources) = self.runtime.client_resources[index].as_mut() else {
                continue;
            };
            if resources.downloads.is_none() {
                let Some(seat) = self.runtime.snapshots[index]
                    .as_ref()
                    .and_then(|binding| binding.seat())
                else {
                    continue;
                };
                let permissions =
                    match DownloadViews::bind(&self.console.cvars, Protocol::Quake3_68, seat) {
                        Ok(permissions) => permissions,
                        Err(error) => {
                            self.reject_client_startup(client, format_args!("{error}"));
                            continue;
                        }
                    };
                resources.downloads =
                    Some(ClientDownload::new(None, permissions, Protocol::Quake3_68));
            }
            let Some(downloads) = resources.downloads.as_mut() else {
                continue;
            };
            if downloads.packages.is_none() {
                let info = resources
                    .state
                    .configs
                    .config(qa_core::configstrings::ConfigLayout::Q3_SYSTEM_INFO)
                    .map_or(&b""[..], |config| config.text);
                match downloads.begin_packages(info, &self.runtime.vfs, &self.console.cvars) {
                    Ok(false) => {
                        resources.startup.q3_packages_ready();
                        if let Some(assets) = self.runtime.sound_assets.as_deref_mut() {
                            resources.prepare_sounds(assets, &self.runtime.vfs);
                        }
                        continue;
                    }
                    Ok(true) => {}
                    Err(error) => {
                        self.reject_client_startup(client, format_args!("{error}"));
                        continue;
                    }
                }
            }
            match downloads.advance_packages(&self.runtime.vfs, &self.console.cvars) {
                Ok(true) => resources.startup.q3_wait_gamestate(),
                Ok(false) => {}
                Err(error) => {
                    let _ = self
                        .runtime
                        .server
                        .events
                        .console_formatted(format_args!("Native package download: {error}\n"));
                }
            }
        }
        self.flush_client_downloads();
    }

    pub(crate) fn advance_startup_files(&mut self) {
        for index in 0..self.runtime.client_resources.len() {
            loop {
                let Some(resources) = self.runtime.client_resources[index].as_ref() else {
                    break;
                };
                if resources
                    .downloads
                    .as_ref()
                    .is_some_and(|downloads| !downloads.resource_idle())
                {
                    break;
                }
                let result = self.runtime.client_resources[index]
                    .as_mut()
                    .map(|resources| {
                        resources.next_startup_file(
                            &self.runtime.vfs,
                            self.runtime.model_assets.as_mut(),
                        )
                    });
                let name = match result {
                    Some(Ok(Some(name))) => name,
                    Some(Ok(None)) => {
                        if let Some(resources) = self.runtime.client_resources[index].as_mut()
                            && let Some(assets) = self.runtime.sound_assets.as_deref_mut()
                        {
                            resources.prepare_sounds(assets, &self.runtime.vfs);
                        }
                        break;
                    }
                    Some(Err(error)) => {
                        self.reject_client_startup(ClientId(index as u32), format_args!("{error}"));
                        break;
                    }
                    None => break,
                };
                if name.as_bytes().is_empty() || self.runtime.vfs.open(name.as_bytes()).is_some() {
                    continue;
                }
                if self.runtime.client_resources[index]
                    .as_ref()
                    .is_some_and(|resources| {
                        resources
                            .downloads
                            .as_ref()
                            .is_none_or(|downloads| !downloads.configured())
                    })
                {
                    if let Some(config) = self.download_defaults {
                        if let Err(error) = self.configure_client_downloads(
                            ClientId(index as u32),
                            config.game,
                            config.skins,
                            config.maximum_bytes,
                            config.buffer_bytes,
                        ) {
                            self.reject_client_startup(
                                ClientId(index as u32),
                                format_args!("{error}"),
                            );
                            break;
                        }
                    } else {
                        continue;
                    }
                }
                let result = self.runtime.client_resources[index]
                    .as_mut()
                    .and_then(|resources| resources.downloads.as_mut())
                    .map(|downloads| {
                        let name = std::str::from_utf8(name.as_bytes())
                            .map_err(|_| "native resource path encoding".to_owned())?;
                        let name = qa_core::names::canonical_path(name);
                        downloads.start(
                            name.as_bytes(),
                            None,
                            &self.runtime.vfs,
                            &self.console.cvars,
                            Purpose::Startup,
                        )
                    });
                match result {
                    Some(Ok(())) => break,
                    Some(Err(error)) => {
                        let _ =
                            self.runtime.server.events.console_formatted(format_args!(
                                "Native resource download: {error}\n"
                            ));
                    }
                    None => {}
                }
            }
        }
    }

    pub(crate) fn advance_qw_skin_downloads(&mut self) {
        for index in 0..self.runtime.client_resources.len() {
            let Some(binding) = self.runtime.snapshots[index].as_ref() else {
                continue;
            };
            if binding.protocol != Protocol::QuakeWorld28
                || binding.presentation_closed()
                || self.runtime.client_resources[index]
                    .as_ref()
                    .is_some_and(|resources| !resources.startup.allow_skins())
            {
                continue;
            }
            let Some(seat) = binding.seat() else {
                continue;
            };
            if self.runtime.client_resources[index]
                .as_ref()
                .is_some_and(|resources| {
                    resources
                        .downloads
                        .as_ref()
                        .is_none_or(|downloads| !downloads.configured())
                })
            {
                let Some(config) = self.download_defaults else {
                    continue;
                };
                if let Err(error) = self.configure_client_downloads(
                    ClientId(index as u32),
                    config.game,
                    config.skins,
                    config.maximum_bytes,
                    config.buffer_bytes,
                ) {
                    let _ = self.runtime.server.events.console_formatted(format_args!(
                        "Automatic skin download unavailable: {error}\n"
                    ));
                    continue;
                }
            }
            let Some(resources) = self.runtime.client_resources[index].as_mut() else {
                continue;
            };
            let Some(downloads) = resources.downloads.as_mut() else {
                continue;
            };
            let Some((stamp, count, enabled)) = self.player_skin_views.download_stamp(
                &self.console.cvars,
                seat,
                &resources.state.models,
            ) else {
                continue;
            };
            let result = downloads.advance_skins(
                stamp,
                count,
                enabled,
                &self.runtime.vfs,
                &self.console.cvars,
                |slot| {
                    self.player_skin_views.download_paths(
                        &self.console.cvars,
                        seat,
                        &resources.state.models,
                        &resources.state.configs,
                        slot,
                    )
                },
            );
            if let Err(error) = result {
                let _ =
                    self.runtime.server.events.console_formatted(format_args!(
                        "Automatic skin download rejected: {error}\n"
                    ));
            }
        }
        self.flush_client_downloads();
    }

    pub(crate) fn finish_client_downloads(
        &mut self,
        mut publish: impl FnMut(
            &mut crate::model_assets::ModelAssets,
            &Vfs,
            &[u8],
            &mut Option<crate::renderer::SceneComposer>,
            &mut crate::modules::renderer_contexts::Contexts,
        ) -> Result<bool, String>,
    ) {
        let mut native_complete = false;
        for index in 0..self.runtime.client_resources.len() {
            let Some((request, package)) = self.runtime.client_resources[index]
                .as_ref()
                .and_then(|resources| resources.downloads.as_ref())
                .and_then(|downloads| match &downloads.transfer {
                    Transfer::Published {
                        request, package, ..
                    } => Some((request.clone(), package.clone())),
                    _ => None,
                })
            else {
                continue;
            };
            let name = request.local.as_bytes();
            let result = self
                .runtime
                .vfs
                .refresh_published_file(request.mount, name, package.as_deref())
                .map_err(|error| error.to_string())
                .and_then(|package| {
                    let vfs = &self.runtime.vfs;
                    for (_, path) in vfs
                        .files_in_mount(package.unwrap_or(request.mount))
                        .filter(|(_, path)| package.is_some() || *path == name)
                    {
                        let changed =
                            self.runtime
                                .model_assets
                                .as_mut()
                                .map_or(Ok(false), |owner| {
                                    publish(
                                        owner,
                                        vfs,
                                        path,
                                        &mut self.runtime.scene,
                                        &mut self.renderer_contexts,
                                    )
                                })?;
                        let skin = path.starts_with(b"skins/");
                        if skin {
                            self.player_skin_views.request_refresh();
                        }
                        if changed || skin {
                            for resources in self.runtime.client_resources.iter_mut().flatten() {
                                if changed {
                                    resources.retry_published_file(path);
                                }
                                if skin {
                                    resources.startup.invalidate_skins();
                                }
                            }
                        }
                    }
                    Ok(package.is_some())
                });
            let Some(resources) = self.runtime.client_resources[index].as_mut() else {
                continue;
            };
            let Some(downloads) = resources.downloads.as_mut() else {
                continue;
            };
            let previous = std::mem::replace(&mut downloads.transfer, Transfer::Idle);
            let Transfer::Published {
                request,
                bytes,
                package,
            } = previous
            else {
                continue;
            };
            match result {
                Ok(package) => {
                    if package
                        && !matches!(request.purpose, Purpose::Package(_))
                        && let Some(blocks) = &mut downloads.blocks
                    {
                        blocks.done = true;
                        resources.startup.q3_wait_gamestate();
                        native_complete = true;
                    }
                    let _ = self.runtime.server.events.console_formatted(format_args!(
                        "Downloaded {} ({} bytes).\n",
                        request, bytes
                    ));
                    downloads.transfer = Transfer::Finished { request, bytes };
                }
                Err(error) => {
                    let _ = self.runtime.server.events.console_formatted(format_args!(
                        "Downloaded file content refresh rejected: {error}\n"
                    ));
                    downloads.transfer = Transfer::RefreshFailed {
                        request,
                        bytes,
                        package,
                    };
                }
            }
        }
        if native_complete {
            self.flush_client_downloads();
        }
    }
}
