use super::*;
use qa_content::vfs::{MountId, Vfs};
use qa_core::{sys_events::SeatId, text::FixedText};
use qa_network::{channel::TransmitError, downloads::Chunk};
use qa_platform::files::StagedFile;
use std::fmt::Write;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DownloadConfig {
    game: MountId,
    skins: MountId,
    maximum_bytes: u64,
    buffer_bytes: usize,
}

#[derive(Clone)]
struct Request {
    name: FixedText<64>,
    mount: MountId,
    purpose: Purpose,
}
#[derive(Clone, Copy)]
enum Purpose {
    Manual,
    Skin,
    Startup,
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
    },
    RefreshFailed {
        request: Request,
        bytes: u64,
    },
    Finished {
        request: Request,
        bytes: u64,
    },
    Failed(Request),
}

pub(crate) struct ClientDownload {
    config: DownloadConfig,
    transfer: Transfer,
    permissions: Option<DownloadViews>,
    skin_pass: Option<SkinPass>,
}

struct DownloadViews {
    protocol: Protocol,
    allow: qa_console::cvars::View,
    categories: [qa_console::cvars::View; 4],
}
impl DownloadViews {
    fn bind(
        cvars: &qa_console::cvars::Cvars,
        protocol: Protocol,
        seat: SeatId,
    ) -> Result<Option<Self>, String> {
        if !matches!(protocol, Protocol::Quake2_34 | Protocol::Quake2Repro1038) {
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
            categories: [maps?, models?, sounds?, players?],
        }))
    }
    fn allows(&self, name: &[u8], cvars: &qa_console::cvars::Cvars) -> bool {
        if cvars.numeric(self.allow).unwrap_or(0.0) <= 0.0 {
            return false;
        }
        let allowed = |index| cvars.numeric(self.categories[index]).unwrap_or(0.0) != 0.0;
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
    fn new(config: DownloadConfig, permissions: Option<DownloadViews>, protocol: Protocol) -> Self {
        Self {
            config,
            transfer: Transfer::Idle,
            permissions,
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
        self.transfer = Transfer::Idle;
        if let Some(pass) = &mut self.skin_pass {
            pass.stamp = None;
            pass.cursor = 0;
            pass.attempted.clear();
            pass.paused = false;
            pass.complete = false;
        }
    }

    pub(crate) fn reject(&mut self) {
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
        name: &[u8],
        vfs: &Vfs,
        cvars: &qa_console::cvars::Cvars,
        purpose: Purpose,
    ) -> Result<(), String> {
        if matches!(
            self.transfer,
            Transfer::Active { .. }
                | Transfer::Paused { draining: true, .. }
                | Transfer::Published { .. }
        ) {
            return Err("a download or its outstanding reply is still active".into());
        }
        qa_network::downloads::validate_path(name).map_err(|_| "invalid native download path")?;
        let text = std::str::from_utf8(name).map_err(|_| "download path is not UTF-8")?;
        let canonical = qa_core::names::canonical_path(text);
        let mut name = FixedText::default();
        name.set(&canonical).map_err(|_| "download path capacity")?;
        if vfs.open(name.as_bytes()).is_some() {
            return Err("download file is already available in mounted content".into());
        }
        if !self.allowed(name.as_bytes(), cvars) {
            return Err("download disabled by local CLIENT policy".into());
        }
        let mount = if name.as_bytes().starts_with(b"skins/") {
            self.config.skins
        } else {
            self.config.game
        };
        self.open(
            Request {
                name,
                mount,
                purpose,
            },
            vfs,
        )
    }

    fn allowed(&self, name: &[u8], cvars: &qa_console::cvars::Cvars) -> bool {
        self.permissions
            .as_ref()
            .is_none_or(|views| views.allows(name, cvars))
    }

    fn open(&mut self, request: Request, vfs: &Vfs) -> Result<(), String> {
        let result = (|| {
            let root = vfs
                .directory_path(request.mount)
                .ok_or("download directory is no longer mounted")?;
            let file = StagedFile::create(
                root,
                request.name.as_str(),
                self.config.maximum_bytes,
                self.config.buffer_bytes,
            )
            .map_err(|error| format!("download staging: {error}"))?;
            Ok::<_, String>(file)
        })();
        match result {
            Ok(file) => {
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

    fn stop(&mut self) {
        if let Some(pass) = &mut self.skin_pass {
            pass.paused = true;
        }
        let previous = std::mem::replace(&mut self.transfer, Transfer::Idle);
        self.transfer = match previous {
            Transfer::Active {
                request, sending, ..
            } => Transfer::Paused {
                request,
                draining: matches!(sending, Sending::Awaiting),
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
                match self.start(request.name.as_bytes(), vfs, cvars, request.purpose) {
                    Ok(()) => Ok(()),
                    Err(error) => {
                        self.transfer = Transfer::Failed(request);
                        Err(error)
                    }
                }
            }
            Transfer::RefreshFailed { request, bytes } => {
                self.transfer = Transfer::Published { request, bytes };
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
        mut paths: impl FnMut(usize) -> Option<(FixedText<64>, Option<FixedText<64>>)>,
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
            match self.start(name.as_bytes(), vfs, cvars, Purpose::Skin) {
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
        matches!(
            self.transfer,
            Transfer::Idle
                | Transfer::Finished { .. }
                | Transfer::Failed(Request {
                    purpose: Purpose::Skin | Purpose::Startup,
                    ..
                })
        )
    }

    fn pending(&self) -> Option<FixedText<80>> {
        let Transfer::Active {
            request, sending, ..
        } = &self.transfer
        else {
            return None;
        };
        let mut command = FixedText::default();
        match sending {
            Sending::Request => write!(command, "download {}", request.name.as_str()).ok()?,
            Sending::Next => command.set("nextdl").ok()?,
            Sending::Awaiting => return None,
        }
        Some(command)
    }

    fn queued(&mut self) {
        if let Transfer::Active { sending, .. } = &mut self.transfer {
            *sending = Sending::Awaiting;
        }
    }

    pub(crate) fn receive(
        &mut self,
        protocol: Protocol,
        chunk: Chunk<'_>,
        vfs: &Vfs,
        cvars: &qa_console::cvars::Cvars,
    ) -> Result<(), String> {
        let previous = std::mem::replace(&mut self.transfer, Transfer::Idle);
        match previous {
            Transfer::Paused {
                request,
                draining: true,
                retry,
            } => {
                if retry && self.allowed(request.name.as_bytes(), cvars) {
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
                    if !self.allowed(request.name.as_bytes(), cvars) {
                        return Err("download disabled by local CLIENT policy".into());
                    }
                    if vfs.directory_path(request.mount).is_none() {
                        return Err("download directory retired".into());
                    }
                    let bytes = chunk
                        .bytes
                        .ok_or_else(|| format!("server has no {}", request.name.as_str()))?;
                    if chunk.percent < percent || chunk.percent > 100 {
                        return Err("download progress regressed".into());
                    }
                    file.append(bytes)
                        .map_err(|error| format!("download write: {error}"))?;
                    let empty = qa_network::downloads::sequential(protocol)
                        .is_some_and(|policy| policy.empty_completes())
                        && file.is_empty()
                        && bytes.is_empty();
                    if chunk.percent == 100 || empty {
                        let bytes = file
                            .publish()
                            .map_err(|error| format!("download publication: {error}"))?;
                        return Ok(Transfer::Published {
                            request: request.clone(),
                            bytes,
                        });
                    }
                    if bytes.is_empty() {
                        return Err("download made no progress".into());
                    }
                    Ok(Transfer::Active {
                        request: request.clone(),
                        file,
                        percent: chunk.percent,
                        sending: Sending::Next,
                    })
                })();
                match result {
                    Ok(state) => {
                        self.transfer = state;
                        Ok(())
                    }
                    Err(error) => {
                        self.transfer = Transfer::Failed(request);
                        Err(error)
                    }
                }
            }
            other => {
                self.transfer = other;
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
            } => writeln!(
                out,
                "{}: {} bytes, {}%",
                request.name.as_str(),
                file.len(),
                percent
            ),
            Transfer::Paused {
                request,
                draining,
                retry,
            } => writeln!(
                out,
                "{}: paused, pending reply={}, retry={}",
                request.name.as_str(),
                draining,
                retry
            ),
            Transfer::Published { request, bytes } => writeln!(
                out,
                "{}: {} bytes published, content refresh pending",
                request.name.as_str(),
                bytes
            ),
            Transfer::RefreshFailed { request, bytes } => writeln!(
                out,
                "{}: {} bytes published, refresh failed; retrydownload retries refresh",
                request.name.as_str(),
                bytes
            ),
            Transfer::Finished { request, bytes } => {
                writeln!(out, "{}: {} bytes complete", request.name.as_str(), bytes)
            }
            Transfer::Failed(request) => writeln!(
                out,
                "{}: failed; retrydownload retries",
                request.name.as_str()
            ),
        }
    }
}

impl crate::host::FrameHost {
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
        qa_network::downloads::sequential(protocol)
            .ok_or("protocol has no sequential download service")?;
        let resources = self
            .runtime
            .client_resources
            .get_mut(client.0 as usize)
            .and_then(Option::as_mut)
            .ok_or("no CLIENT resources")?;
        if let Some(existing) = &resources.downloads {
            if existing.config == config {
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
        resources.downloads = Some(ClientDownload::new(config, permissions, protocol));
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
        qa_network::downloads::sequential(protocol)
            .ok_or("this protocol has no sequential downloads")?;
        if self.runtime.client_resources[client.0 as usize]
            .as_ref()
            .is_some_and(|resources| resources.downloads.is_none())
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
                if args.len() != 2 {
                    return Err("usage: download <datafile>".into());
                }
                downloads.start(
                    args.get_bytes(1),
                    &self.runtime.vfs,
                    &self.console.cvars,
                    Purpose::Manual,
                )?;
            }
            1 => {
                let mut text = FixedText::<192>::default();
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
                continue;
            }
            let Some(command) = downloads.pending() else {
                continue;
            };
            if let Transfer::Active { request, .. } = &downloads.transfer
                && !downloads.allowed(request.name.as_bytes(), &self.console.cvars)
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
            let Some(commands) = &connection.commands else {
                downloads.close();
                continue;
            };
            match connection
                .channel
                .queue_client_command(commands.protocol, command.as_bytes())
            {
                Ok(_) => downloads.queued(),
                Err(TransmitError::Full) => {}
                Err(_) => {
                    downloads.reject();
                    let _ = self
                        .runtime
                        .server
                        .events
                        .console_bytes(b"Download command rejected by native channel.\n");
                }
            }
        }
    }

    pub(crate) fn advance_startup_files(&mut self) {
        use super::startup::ResourceKind;
        for index in 0..self.runtime.client_resources.len() {
            loop {
                let Some(resources) = self.runtime.client_resources[index].as_ref() else {
                    break;
                };
                let Some((kind, ordinal, end)) = resources.startup.file_cursor() else {
                    break;
                };
                if resources
                    .downloads
                    .as_ref()
                    .is_some_and(|downloads| !downloads.resource_idle())
                {
                    break;
                }
                if ordinal > end {
                    if let Some(resources) = self.runtime.client_resources[index].as_mut() {
                        resources.startup.files_complete();
                        if kind == ResourceKind::Sound
                            && let Some(assets) = self.runtime.sound_assets.as_deref_mut()
                        {
                            resources.prepare_sounds(assets, &self.runtime.vfs);
                        }
                    }
                    break;
                }
                let mut name = FixedText::<64>::default();
                let path = kind
                    .range(resources.state.layout)
                    .get(ordinal as usize)
                    .and_then(|row| resources.state.configs.config(row))
                    .map(|row| row.text);
                let path = match path {
                    Some(path) if kind == ResourceKind::Sound => {
                        qa_audio::sound_path(path, qa_core::primitives::RuleSetId::QuakeWorld)
                            .map_err(|_| "invalid QW sound path")
                            .and_then(|path| match path {
                                Some(path) => name
                                    .set(path.as_str())
                                    .map_err(|_| "QW resource path capacity"),
                                None => Ok(()),
                            })
                    }
                    Some(path) if path.starts_with(b"*") => Ok(()),
                    Some(path) => name
                        .set_bytes(path)
                        .map_err(|_| "QW resource path capacity"),
                    None => Err("QW resource list row missing"),
                };
                if let Some(resources) = self.runtime.client_resources[index].as_mut() {
                    resources.startup.advance_file();
                }
                if let Err(error) = path {
                    self.reject_client_startup(ClientId(index as u32), format_args!("{error}"));
                    break;
                }
                if name.as_bytes().is_empty() || self.runtime.vfs.open(name.as_bytes()).is_some() {
                    continue;
                }
                if self.runtime.client_resources[index]
                    .as_ref()
                    .is_some_and(|resources| resources.downloads.is_none())
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
                        downloads.start(
                            name.as_bytes(),
                            &self.runtime.vfs,
                            &self.console.cvars,
                            Purpose::Startup,
                        )
                    });
                match result {
                    Some(Ok(())) => break,
                    Some(Err(error)) => {
                        let _ = self
                            .runtime
                            .server
                            .events
                            .console_formatted(format_args!("QW resource download: {error}\n"));
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
                .is_some_and(|resources| resources.downloads.is_none())
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

    pub(crate) fn finish_client_downloads(&mut self) {
        for resources in self.runtime.client_resources.iter_mut().flatten() {
            let Some(downloads) = &mut resources.downloads else {
                continue;
            };
            let Transfer::Published { request, .. } = &downloads.transfer else {
                continue;
            };
            let name = request.name.as_bytes();
            let prefix = name
                .iter()
                .rposition(|&byte| byte == b'/')
                .map_or(&b""[..], |end| &name[..end]);
            let result = self
                .runtime
                .vfs
                .refresh_directory_prefix(request.mount, prefix);
            let previous = std::mem::replace(&mut downloads.transfer, Transfer::Idle);
            let Transfer::Published { request, bytes } = previous else {
                continue;
            };
            match result {
                Ok(()) => {
                    if request.name.as_bytes().starts_with(b"skins/") {
                        self.player_skin_views.request_refresh();
                        resources.startup.invalidate_skins();
                    }
                    let _ = self.runtime.server.events.console_formatted(format_args!(
                        "Downloaded {} ({} bytes).\n",
                        request.name.as_str(),
                        bytes
                    ));
                    downloads.transfer = Transfer::Finished { request, bytes };
                }
                Err(error) => {
                    let _ = self.runtime.server.events.console_formatted(format_args!(
                        "Downloaded file content refresh rejected: {error}\n"
                    ));
                    downloads.transfer = Transfer::RefreshFailed { request, bytes };
                }
            }
        }
    }
}
