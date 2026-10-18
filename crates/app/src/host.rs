//! Com_Frame: events/commands, server, events/commands, client.
use crate::Runtime;
use qa_console::commands::Console;
use qa_core::{
    primitives::{ClientId, CvarHandle, EffectEvent, ModuleId, RuleSetId, SoundEvent, UserCmd},
    sys_events::{EventKind, EventTime, SeatId, SysEventQueue},
};
use qa_input::Target;
use qa_platform::{EventPump, Stopwatch, Window};
use qa_session::timing::{Tick, TickRate, TickTarget, Timeline};
use std::time::Duration;

/// CLIENT camera values are copied after snapshot application and prediction.
/// This is a fixed per-seat submission, not a second player or command store.
#[derive(Clone, Copy, Debug)]
pub struct ClientView {
    pub world: qa_render::world::WorldId,
    pub world_presentation: qa_render::world::WorldPresentationId,
    pub presentation: qa_render::material::world_load::PresentationDefaults,
    pub presentation_rules: RuleSetId,
    pub origin: qa_core::primitives::Vec3,
    pub angles: qa_core::primitives::Vec3,
    pub fov_x: f32,
    pub screen_blend: [f32; 4],
    pub damage_blend: [f32; 4],
    pub flags: qa_core::primitives::PlayerViewFlags,
    pub time_ms: u64,
    pub model_time: Option<qa_core::primitives::ThinkTime>,
    pub lighting_scales: [f32; 2],
    pub curve_error: f32,
    pub model_lod: qa_render::scene::ModelLodSettings,
    pub model_replacements: [qa_render::scene::ModelReplacementSettings; 5],
    pub policy: qa_render::scene::ViewPolicy,
    pub near: f32,
}

/// The live adapter delegates every OS operation to platform. The same host
/// function can be checked headlessly with supplied event times.
pub trait FrameSource {
    fn video_kind(&self) -> Option<crate::renderer::Kind> {
        None
    }
    fn video_window_request(&self) -> Option<qa_core::presentation::WindowRequest> {
        None
    }
    fn prepare_video(
        &mut self,
        _policy: crate::renderer::VideoPolicy,
        _assets: &qa_render::Assets,
        _limits: qa_render::Limits,
    ) -> Result<crate::renderer::PreparedVideo, String> {
        Err("video restart is unavailable".into())
    }
    fn publish_video(
        &mut self,
        _candidate: crate::renderer::PreparedVideo,
    ) -> Result<std::sync::Arc<qa_core::presentation::RendererConfiguration>, String> {
        Err("video restart is unavailable".into())
    }
    fn discard_video(&mut self, candidate: crate::renderer::PreparedVideo) -> Result<(), String> {
        drop(candidate);
        Ok(())
    }
    fn asset_preparer(&mut self) -> Option<&mut (dyn crate::renderer::AssetPreparer + '_)> {
        None
    }
    fn driver(&mut self) -> crate::modules::Driver<'_> {
        crate::modules::Driver::new(self.asset_preparer())
    }

    /// Starts measurement and samples platform time without collecting events.
    fn begin_frame(&mut self) -> Duration;
    fn poll_events(&mut self, queue: &mut SysEventQueue);
    fn set_socket_submission_enabled(&mut self, _enabled: bool) {}
    /// Selection only; SDL applies it during an existing physical intake.
    fn request_mouse_capture(&mut self, _requested: bool) {}
    /// Waits on time only. Physical intake occurs in the two poll_events calls.
    fn wait_time(&mut self, remaining: Duration) -> Duration;
    fn elapsed(&self) -> Duration;
    fn render(
        &mut self,
        _composition: Option<&mut crate::renderer::SceneComposer>,
        _resources: Option<crate::model_assets::ModelView<'_>>,
        _native: &dyn qa_render::shader::reference::NativeShaders,
        _views: &[Option<ClientView>; SeatId::COUNT],
        _scenes: &[Option<crate::client_scene::ClientSceneInput<'_>>; SeatId::COUNT],
        _huds: &[Option<crate::renderer::hud::HudView<'_>>; SeatId::COUNT],
        _console: qa_console::presentation::ConsoleView<'_>,
    ) {
    }
    fn present(&mut self);
    /// Cold registration has no consumer backend yet. Retain its presentation
    /// records while independent native channel consumers continue delivery.
    fn presentation_ready(&self) -> bool {
        true
    }
    /// False reports an unloaded backend, not successful audio/particle proof.
    fn reset_presentation(
        &mut self,
        _client: ClientId,
        _lifetime: qa_core::primitives::EntityId,
    ) -> bool {
        true
    }
    fn sound(&mut self, _bank: Option<&qa_audio::Bank>, _event: SoundEvent) -> bool {
        false
    }
    fn music(
        &mut self,
        _vfs: &qa_content::vfs::Vfs,
        _control: qa_core::primitives::MusicControl,
        _intro: Option<&[u8]>,
        _loop_track: Option<&[u8]>,
        _search: Option<&qa_content::vfs::SearchView>,
    ) -> bool {
        false
    }
    fn update_music(&mut self, _vfs: &qa_content::vfs::Vfs, _volume: f32) {}
    fn music_report(
        &self,
        _vfs: &qa_content::vfs::Vfs,
        _query: qa_core::primitives::MusicQuery,
        texts: &mut qa_core::events::TextStore,
    ) -> Option<qa_core::primitives::TextLease> {
        texts.insert_formatted(format_args!("No soundtrack source selected.\n"))
    }
    fn listeners(
        &mut self,
        _listeners: &[Option<qa_audio::Listener>; SeatId::COUNT],
        _volume: f32,
    ) {
    }
    fn mix_audio(
        &mut self,
        _bank: Option<&qa_audio::Bank>,
        _entities: &[Option<crate::client_scene::ClientEntities<'_>>; SeatId::COUNT],
    ) {
    }
    fn effect(&mut self, _palette: Option<qa_render::PaletteId>, _event: EffectEvent) -> bool {
        false
    }
    fn effect_clients(
        &mut self,
        _clients: &[Option<qa_render::effects::EffectClient>; SeatId::COUNT],
        _time: EventTime,
        _gravity: f32,
    ) {
    }
    /// Native encoding, sequencing and ACKs belong to the shared Channel.
    /// This primitive transport reports only datagram admission, never an ACK.
    fn send_packet(&mut self, _socket: u16, _to: std::net::SocketAddr, _bytes: &[u8]) -> bool {
        false
    }
}
pub struct LiveFrame<'a> {
    pub pump: &'a mut EventPump,
    pub window: &'a mut Window,
    pub renderer: Option<&'a mut crate::renderer::Renderer>,
    pub audio: Option<&'a mut crate::audio::Output>,
}
impl FrameSource for LiveFrame<'_> {
    fn video_window_request(&self) -> Option<qa_core::presentation::WindowRequest> {
        Some(self.window.request())
    }
    fn video_kind(&self) -> Option<crate::renderer::Kind> {
        self.renderer
            .as_deref()
            .map(crate::renderer::Renderer::kind)
    }
    fn prepare_video(
        &mut self,
        policy: crate::renderer::VideoPolicy,
        assets: &qa_render::Assets,
        limits: qa_render::Limits,
    ) -> Result<crate::renderer::PreparedVideo, String> {
        crate::renderer::prepare_live_video(self, policy, assets, limits)
    }
    fn publish_video(
        &mut self,
        candidate: crate::renderer::PreparedVideo,
    ) -> Result<std::sync::Arc<qa_core::presentation::RendererConfiguration>, String> {
        crate::renderer::publish_live_video(self, candidate)
    }
    fn discard_video(&mut self, candidate: crate::renderer::PreparedVideo) -> Result<(), String> {
        crate::renderer::discard_live_video(self, candidate)
    }
    fn driver(&mut self) -> crate::modules::Driver<'_> {
        let preparer = self
            .renderer
            .as_deref_mut()
            .map(|renderer| renderer as &mut dyn crate::renderer::AssetPreparer);
        crate::modules::Driver::with_connector(preparer, self.pump)
    }
    fn asset_preparer(&mut self) -> Option<&mut (dyn crate::renderer::AssetPreparer + '_)> {
        match self.renderer.as_deref_mut() {
            Some(renderer) => Some(renderer),
            None => None,
        }
    }

    fn set_socket_submission_enabled(&mut self, enabled: bool) {
        self.pump.set_socket_submission_enabled(enabled);
    }
    fn effect(&mut self, palette: Option<qa_render::PaletteId>, event: EffectEvent) -> bool {
        self.renderer
            .as_deref_mut()
            .is_some_and(|renderer| renderer.effect(palette, event))
    }
    fn effect_clients(
        &mut self,
        clients: &[Option<qa_render::effects::EffectClient>; SeatId::COUNT],
        time: EventTime,
        gravity: f32,
    ) {
        if let Some(renderer) = self.renderer.as_deref_mut() {
            renderer.set_effect_clients(*clients, time, gravity);
        }
    }
    fn reset_presentation(
        &mut self,
        client: ClientId,
        lifetime: qa_core::primitives::EntityId,
    ) -> bool {
        if let Some(audio) = self.audio.as_deref_mut() {
            audio.mixer.reset_client(client, lifetime, false);
            audio.music.reset_client(&mut audio.mixer, client, lifetime);
        }
        if let Some(renderer) = self.renderer.as_deref_mut() {
            renderer.reset_client_effects(client, lifetime);
        }
        true
    }
    fn sound(&mut self, bank: Option<&qa_audio::Bank>, event: SoundEvent) -> bool {
        self.audio.as_deref_mut().is_some_and(|audio| {
            audio.music.sound_control(&mut audio.mixer, event);
            bank.is_some_and(|bank| audio.mixer.sound(bank, event))
        })
    }
    fn music(
        &mut self,
        vfs: &qa_content::vfs::Vfs,
        control: qa_core::primitives::MusicControl,
        intro: Option<&[u8]>,
        loop_track: Option<&[u8]>,
        search: Option<&qa_content::vfs::SearchView>,
    ) -> bool {
        self.audio.as_deref_mut().is_some_and(|audio| {
            audio
                .music
                .control(vfs, &mut audio.mixer, control, intro, loop_track, search)
        })
    }
    fn update_music(&mut self, vfs: &qa_content::vfs::Vfs, volume: f32) {
        if let Some(audio) = self.audio.as_deref_mut() {
            audio.music.update(vfs, &mut audio.mixer, volume);
        }
    }
    fn music_report(
        &self,
        vfs: &qa_content::vfs::Vfs,
        query: qa_core::primitives::MusicQuery,
        texts: &mut qa_core::events::TextStore,
    ) -> Option<qa_core::primitives::TextLease> {
        match self.audio.as_deref() {
            Some(audio) => {
                texts.insert_formatted(format_args!("{}", audio.music.report(vfs, query)))
            }
            None => texts.insert_formatted(format_args!("No soundtrack source selected.\n")),
        }
    }
    fn listeners(&mut self, listeners: &[Option<qa_audio::Listener>; SeatId::COUNT], volume: f32) {
        if let Some(audio) = self.audio.as_deref_mut() {
            audio.mixer.listen(listeners, volume);
        }
    }
    fn mix_audio(
        &mut self,
        bank: Option<&qa_audio::Bank>,
        entities: &[Option<crate::client_scene::ClientEntities<'_>>; SeatId::COUNT],
    ) {
        if let Some(audio) = self.audio.as_deref_mut() {
            if let Some(bank) = bank {
                audio.submit(bank, entities);
            }
        }
    }
    fn send_packet(&mut self, socket: u16, to: std::net::SocketAddr, bytes: &[u8]) -> bool {
        self.pump.send_udp(socket, to, bytes).is_ok()
    }
    fn begin_frame(&mut self) -> Duration {
        self.pump.begin_frame()
    }
    fn poll_events(&mut self, queue: &mut SysEventQueue) {
        self.pump.poll_events(self.window, queue);
    }
    fn request_mouse_capture(&mut self, requested: bool) {
        self.window.request_mouse_capture(requested);
    }
    fn wait_time(&mut self, remaining: Duration) -> Duration {
        self.pump.wait_time(remaining)
    }
    fn elapsed(&self) -> Duration {
        self.pump.elapsed()
    }
    fn present(&mut self) {
        if let Some(renderer) = self.renderer.as_deref_mut() {
            renderer.present(self.window);
        }
    }
    fn presentation_ready(&self) -> bool {
        self.renderer.is_some() && self.audio.is_some()
    }
    fn render(
        &mut self,
        composition: Option<&mut crate::renderer::SceneComposer>,
        resources: Option<crate::model_assets::ModelView<'_>>,
        native: &dyn qa_render::shader::reference::NativeShaders,
        views: &[Option<ClientView>; SeatId::COUNT],
        scenes: &[Option<crate::client_scene::ClientSceneInput<'_>>; SeatId::COUNT],
        huds: &[Option<crate::renderer::hud::HudView<'_>>; SeatId::COUNT],
        console: qa_console::presentation::ConsoleView<'_>,
    ) {
        if let (Some(renderer), Some(composition), Some(resources)) =
            (self.renderer.as_deref_mut(), composition, resources)
        {
            renderer.frame(composition, resources, native, views, scenes, huds, console);
        }
    }
}

#[derive(Clone, Copy)]
pub struct Provider {
    pub module: ModuleId,
    pub rate: TickRate,
    pub frame: fn(&mut FrameHost, Tick, &mut crate::modules::Driver<'_>),
    pub output: Option<
        fn(
            &mut [qa_session::clients::Client],
            Tick,
            qa_core::events::OutputRecord,
            &mut qa_core::events::TextStore,
        ) -> qa_core::events::OutputSubmission,
    >,
}
#[derive(Default)]
struct ClientUserinfo {
    entity: Option<qa_core::primitives::EntityId>,
    revision: Option<u64>,
    queued: qa_core::text::FixedText<{ qa_session::clients::USERINFO_BYTES }>,
}
impl ClientUserinfo {
    fn serialize(
        cvars: &qa_console::cvars::Cvars,
        protocol: qa_network::commands::packet::Protocol,
        seat: SeatId,
        peer: qa_core::sys_events::Peer,
        output: &mut [u8],
    ) -> Result<usize, qa_console::cvars::WriteError> {
        let context = qa_console::views::Context {
            source: protocol.rule_set(),
            seat,
            ..Default::default()
        };
        let rules = protocol
            .userinfo_rules()
            .ok_or(qa_console::cvars::WriteError::InvalidInfo)?;
        let output = output
            .get_mut(..rules.bytes)
            .ok_or(qa_console::cvars::WriteError::Capacity)?;
        let mut length = cvars.info_string(context, 2, rules, output)?;
        if protocol == qa_network::commands::packet::Protocol::QuakeWorld28
            && let qa_core::sys_events::Peer::Socket(address) = peer
        {
            use std::fmt::Write;
            let mut text = qa_core::text::FixedText::<64>::default();
            if write!(text, "{address}").is_ok()
                && let Ok(update) = qa_core::text::InfoUpdate::new(b"*ip", text.as_bytes(), rules)
            {
                // Native Info_SetValueForStarKey leaves usable userinfo when
                // this optional address cannot fit its total native budget.
                let _ = update.first_only().apply(output, &mut length);
            }
        }
        Ok(length)
    }
    fn queue_qw(
        &mut self,
        channel: &mut qa_network::channel::Channel,
        text: &[u8],
    ) -> Result<(), qa_network::channel::TransmitError> {
        use qa_core::text::{FixedText, InfoUpdate, info_changes, info_value};
        use qa_network::{channel::TransmitError, commands::packet::Protocol};
        let previous = self.queued.clone();
        let rules = Protocol::QuakeWorld28
            .userinfo_rules()
            .ok_or(TransmitError::MessageTooLarge)?;
        // Shrink first so individually valid replacements cannot overflow the
        // native total while exchanging a long value for a short one.
        for growing in [false, true] {
            for (key, value) in info_changes(previous.as_bytes(), text) {
                let before = info_value(self.queued.as_bytes(), key, false)
                    .map_or(&b""[..], |range| &self.queued.as_bytes()[range]);
                if key.starts_with(b"*")
                    || value == before
                    || (value.len() > before.len()) != growing
                {
                    continue;
                }
                let update = InfoUpdate::new(key, value, rules)
                    .map_err(|_| TransmitError::MessageTooLarge)?;
                let mut queued = self.queued.clone();
                queued
                    .update_info(update)
                    .map_err(|_| TransmitError::MessageTooLarge)?;
                let mut command = FixedText::<160>::default();
                for part in [&b"setinfo \""[..], key, b"\" \"", value, b"\""] {
                    command
                        .append_bytes(part)
                        .map_err(|_| TransmitError::MessageTooLarge)?;
                }
                channel.queue_client_command(Protocol::QuakeWorld28, command.as_bytes())?;
                // A full reliable queue retains the first unsent field for the
                // next frame; already queued fields are not published again.
                self.queued = queued;
            }
        }
        self.queued
            .set_bytes(text)
            .map_err(|_| TransmitError::MessageTooLarge)
    }
}

#[derive(Clone, Copy)]
struct VideoRequest {
    kind: Option<crate::renderer::Kind>,
}

pub struct FrameHost {
    pub(crate) modules: Option<crate::modules::Modules>,
    pub(crate) renderer_contexts: crate::modules::renderer_contexts::Contexts,
    pub(crate) cue_random: qa_core::random::Random,
    pub console: Console<Runtime>,
    pub runtime: Runtime,
    pub queue: SysEventQueue,
    pub journal: Option<qa_content::journal::Mode>,
    pub journal_error: Option<qa_formats::FormatError>,
    pub time: EventTime,
    pub key_downs: u64,
    pub key_repeats: u64,
    pub close_acknowledged: u64,
    pub close_timed_out: u64,
    pub close_aborted: u64,
    pub close_without_transport: u64,
    pub developer: CvarHandle,
    pub(crate) renderer_smp: Option<CvarHandle>,
    renderer_ui_fullscreen: Option<CvarHandle>,
    cpu_band_view: qa_console::cvars::View,
    cpu_band_override: Option<qa_render::cpu::RasterBands>,
    pub(crate) video_command: qa_core::primitives::NameId,
    pub(crate) video_serial: u64,
    video_request: Option<VideoRequest>,
    video_active: bool,
    pub local_clients: [Option<ClientId>; SeatId::COUNT],
    pub(crate) connection_commands: [qa_core::primitives::NameId; 3],
    /// A seat's world/presentation choice is independent of its physics rules.
    pub local_worlds: [Option<(
        qa_render::world::WorldId,
        qa_render::world::WorldPresentationId,
    )>; SeatId::COUNT],
    pub local_presentations: [qa_render::material::world_load::PresentationDefaults; SeatId::COUNT],
    pub local_presentation_rules: [Option<qa_core::primitives::RuleSetId>; SeatId::COUNT],
    maxfps: CvarHandle,
    zombie_time: CvarHandle,
    fov: CvarHandle,
    q2_fov: (qa_console::cvars::View, qa_console::cvars::View),
    pub(crate) qw_highchars: qa_console::cvars::View,
    pub(crate) qport: qa_console::cvars::View,
    connect_packet_length: qa_console::cvars::View,
    pub(crate) player_model_options: (qa_console::cvars::View, qa_console::cvars::View),
    pub(crate) player_skin_views: crate::client_scene::PlayerSkinViews,
    pub(crate) allskins_command: qa_core::primitives::NameId,
    pub(crate) skins_command: qa_core::primitives::NameId,
    pub(crate) download_commands: [qa_core::primitives::NameId; 4],
    pub(crate) download_defaults: Option<crate::client_resources::downloads::DownloadConfig>,
    software_waterwarp: qa_console::cvars::View,
    curve_error: qa_console::cvars::View,
    model_lod: [qa_console::cvars::View; 2],
    pub(crate) model_controls: qa_console::model::ModelHandles,
    view_policy: [crate::renderer::ViewPolicyViews; 5],
    pub(crate) image_search: qa_console::cvars::View,
    image_search_generation: Option<u64>,
    userinfo: Box<[ClientUserinfo]>,
    nq_identity: [(qa_console::cvars::View, qa_console::cvars::View); SeatId::COUNT],
    lighting_scales: [CvarHandle; 2],
    style_interpolation: CvarHandle,
    roll_angle: CvarHandle,
    roll_speed: CvarHandle,
    kick_angles: CvarHandle,
    smooth_clients: CvarHandle,
    time_nudge: CvarHandle,
    push_latency: qa_console::cvars::View,
    timescale: CvarHandle,
    input_handles: qa_console::input_settings::InputHandles,
    pub(crate) movement_handles: qa_console::movement::MovementHandles,
    pub(crate) collision_handles: qa_console::collision::CollisionHandles,
    notify_time: CvarHandle,
    center_time: CvarHandle,
    pub(crate) team_chats_only: CvarHandle,
    packet_dup: CvarHandle,
    volume: CvarHandle,
    music_volume: CvarHandle,
    particle_gravity: CvarHandle,
    mouse_enabled: CvarHandle,
    previous: Option<EventTime>,
    loop_client_cursor: usize,
    timeline: Timeline,
    providers: Box<[Provider]>,
    module_outputs: Box<[Option<qa_core::events::OutputConsumerId>]>,
    pub(crate) command_tokens: qa_console::command_text::Tokens,
    pub(crate) execution: Box<[crate::modules::ExecutionSlot]>,
}

#[derive(Default)]
pub struct FrameResult {
    pub events: u64,
    pub drains: u64,
    pub server_ticks: u64,
    pub commands: [UserCmd; SeatId::COUNT],
    pub input_ns: u64,
    pub simulation_ns: u64,
    pub client_ns: u64,
    pub total_ns: u64,
    pub output: crate::output::OutputCounts,
    pub output_drains: u64,
}

pub(crate) fn local_seat<'a>(
    runtime: &'a Runtime,
    local_clients: &[Option<qa_core::primitives::ClientId>; SeatId::COUNT],
    seat: usize,
) -> Option<(
    qa_core::primitives::ClientId,
    &'a qa_session::clients::Client,
    &'a qa_core::primitives::PlayerState,
)> {
    let client = local_clients.get(seat).copied().flatten()?;
    let row = runtime.server.clients.get(client.0 as usize)?;
    row.connection?;
    if !runtime
        .network
        .get(client, qa_core::loopback::Endpoint::Client)?
        .phase
        .is_connected()
    {
        return None;
    }
    if runtime
        .snapshots
        .get(client.0 as usize)?
        .as_ref()?
        .presentation_closed()
    {
        return None;
    }
    Some((client, row, &runtime.prediction.get(seat)?.player))
}

pub struct EngineGraphicsAppend<'a> {
    appends: Vec<crate::modules::renderer_contexts::Append<'a>>,
}
impl EngineGraphicsAppend<'_> {
    pub fn commit(self) {
        for append in self.appends {
            append.commit();
        }
    }
}
impl FrameHost {
    pub fn renderer_fullscreen_ui(&self) -> bool {
        self.renderer_ui_fullscreen
            .and_then(|handle| self.console.cvars.native_integer(handle, RuleSetId::Quake3))
            .is_some_and(|value| value != 0)
    }

    pub fn set_renderer_fullscreen_ui(&mut self, fullscreen: bool) {
        if let Some(handle) = self.renderer_ui_fullscreen
            && let Err(error) = self
                .console
                .cvars
                .set_text(handle, if fullscreen { "1" } else { "0" })
        {
            let _ = self
                .runtime
                .server
                .events
                .console_formatted(format_args!("Renderer UI state: {error:?}\n"));
        }
    }
    fn graphics_owners(
        &self,
    ) -> [Option<qa_render::shader::registration::ShaderOwner>; SeatId::COUNT] {
        std::array::from_fn(|seat| {
            let (client, row, _) = self.local_seat(seat)?;
            Some(qa_render::shader::registration::ShaderOwner::Client(
                client, row.entity,
            ))
        })
    }
    pub fn capture_graphics_owners(&mut self) {
        let owners = self.graphics_owners();
        if let Some(scene) = self.runtime.scene.as_mut() {
            scene.graphics_owners(owners);
        }
    }
    pub fn stage_engine_graphics<'a>(
        &'a mut self,
        console: &crate::renderer::console::ConsoleAssets,
        huds: &[Option<crate::renderer::hud::HudAssets>; 5],
    ) -> Result<(&'a mut qa_render::Assets, EngineGraphicsAppend<'a>), String> {
        let owner = self
            .runtime
            .model_assets
            .as_mut()
            .ok_or("missing graphics assets")?;
        let registrations = console
            .registrations()
            .map(|registration| console.font().registered(registration))
            .chain(huds.iter().flatten().flat_map(|hud| {
                hud.registrations()
                    .map(move |registration| hud.font().registered(registration))
            }));
        for registration in registrations.clone() {
            let profile = registration.profile;
            self.renderer_contexts.ensure_owner(
                qa_render::shader::registration::ShaderOwner::Engine,
                profile,
                registration.files,
                owner.assets(),
                owner.shader_occurrence_budget(),
                None,
            )?;
        }
        let appends = self.renderer_contexts.stage_graphics(registrations)?;
        let (assets, _) = owner.unpublished_assets()?;
        Ok((assets, EngineGraphicsAppend { appends }))
    }
    pub(crate) fn prepare_graphics_contexts(&mut self) -> Result<(), String> {
        let owners = self.graphics_owners();
        let Some(scene) = self.runtime.scene.as_ref() else {
            return Ok(());
        };
        let Some(owner) = self.runtime.model_assets.as_ref() else {
            return Ok(());
        };
        for (profile, files) in scene.graphic_sources() {
            let handle = self.renderer_contexts.ensure_owner(
                qa_render::shader::registration::ShaderOwner::Engine,
                profile,
                files,
                owner.assets(),
                owner.shader_occurrence_budget(),
                None,
            )?;
            scene.replay_graphics(&mut self.renderer_contexts, handle)?;
            for authority in owners.iter().flatten().copied() {
                let qa_render::shader::registration::ShaderOwner::Client(client, _) = authority
                else {
                    continue;
                };
                if self
                    .runtime
                    .client_resources
                    .get(client.0 as usize)
                    .and_then(Option::as_ref)
                    .is_none_or(|resources| {
                        !resources.state.file_search().same_selection(files.view())
                    })
                {
                    continue;
                }
                let selected = self
                    .runtime
                    .client_resources
                    .get(client.0 as usize)
                    .and_then(Option::as_ref)
                    .and_then(|resources| resources.state.models.profile());
                if selected != Some(profile)
                    && qa_render::shader::reference::NativeShaders::context(
                        &self.renderer_contexts,
                        authority,
                        profile,
                        files.id(),
                    )
                    .is_none()
                {
                    continue;
                }
                let reset = self
                    .runtime
                    .client_resources
                    .get(client.0 as usize)
                    .and_then(Option::as_ref)
                    .map(|resources| resources.state.completed_presentation_reset());
                if reset == Some(None) {
                    continue;
                }
                let handle = self.renderer_contexts.ensure_owner(
                    authority,
                    profile,
                    files,
                    owner.assets(),
                    owner.shader_occurrence_budget(),
                    reset.flatten().flatten(),
                )?;
                scene.replay_graphics(&mut self.renderer_contexts, handle)?;
            }
        }
        Ok(())
    }

    pub fn set_cpu_band_override(&mut self, bands: Option<qa_render::cpu::RasterBands>) {
        self.cpu_band_override = bands;
    }
    pub(crate) fn request_video_restart(&mut self, kind: Option<crate::renderer::Kind>) {
        self.video_request = Some(VideoRequest { kind });
    }
    pub(crate) fn video_restart_pending(&self) -> bool {
        self.video_request.is_some() || self.video_active
    }
    fn video_resource_selections(
        &self,
    ) -> Result<
        (
            Vec<crate::model_assets::ModelProfile>,
            Vec<(
                qa_render::world::WorldId,
                qa_render::world::WorldPresentationId,
            )>,
        ),
        String,
    > {
        let owner = self
            .runtime
            .model_assets
            .as_ref()
            .ok_or("missing model assets")?;
        let mut profiles = Vec::new();
        let mut presentations = Vec::new();
        for resources in self.runtime.client_resources.iter().flatten() {
            resources.state.image_refresh_selections(
                owner.catalog(),
                &mut profiles,
                &mut presentations,
            );
        }
        if let Some(scene) = &self.runtime.scene {
            for profile in scene.graphic_profiles() {
                if !profiles.contains(&profile) {
                    profiles.push(profile);
                }
            }
        }
        for &world in self.local_worlds.iter().flatten() {
            if !presentations.contains(&world) {
                presentations.push(world);
            }
        }
        self.video_image_selections(&mut profiles, &mut presentations);
        for &(_, presentation) in &presentations {
            let selected = owner
                .assets()
                .world_presentation_registration(presentation)
                .ok_or("video world presentation is unavailable")?;
            for profile in owner.catalog().profiles().filter(|profile| {
                profile.rules == selected.profile.rules
                    && profile.palette == selected.profile.indexed_palette
                    && profile.image_settings == selected.profile.image_settings
                    && profile.finish == selected.profile.finish
            }) {
                if !profiles.contains(&profile) {
                    profiles.push(profile);
                }
            }
        }
        Ok((profiles, presentations))
    }

    fn restart_video(&mut self, source: &mut impl FrameSource) {
        let Some(request) = self.video_request else {
            return;
        };
        let ready = self.video_restart_ready();
        if matches!(ready, Ok(false)) {
            return;
        }
        let limits = match self.runtime.scene.as_mut() {
            Some(scene) => {
                if !scene.asset_stage_ready() {
                    return;
                }
                Some(scene.limits())
            }
            None => None,
        };
        self.video_request = None;
        let candidate = (|| {
            ready?;
            let serial = self.video_serial.checked_add(1).ok_or("video generation")?;
            let limits = limits.ok_or("video restart has no scene")?;
            let (profiles, presentations) = self.video_resource_selections()?;
            let resource_plan = crate::render_settings::ResourceSettingsPlan::prepare(
                &self.console.cvars,
                profiles.iter().map(|profile| profile.rules),
            )?;
            let owner = self
                .runtime
                .model_assets
                .as_ref()
                .ok_or("missing model assets")?;
            let kind = request
                .kind
                .or_else(|| source.video_kind())
                .ok_or("missing active video backend")?;
            let window_plan = crate::window_settings::WindowSettingsPlan::prepare(
                &self.console.cvars,
                kind,
                source.video_window_request(),
            )?;
            let (cpu_bands, band_settings) = if matches!(kind, crate::renderer::Kind::Cpu) {
                (
                    crate::renderer::pending_cpu_band_setting(
                        &self.console.cvars,
                        self.cpu_band_view,
                        self.cpu_band_override,
                    )?,
                    Some(qa_console::pending::PendingSettings::capture(
                        &self.console.cvars,
                        [self.cpu_band_view],
                    )),
                )
            } else {
                (None, None)
            };
            let candidate = source.prepare_video(
                crate::renderer::VideoPolicy {
                    kind: Some(kind),
                    cpu_bands,
                    window: window_plan.request(),
                },
                owner.assets(),
                limits,
            )?;
            let world_plan = match crate::render_settings::WorldSettingsPlan::prepare(
                &self.console.cvars,
                Some(&candidate.configuration()),
            ) {
                Ok(plan) => plan,
                Err(error) => {
                    return Err(match source.discard_video(candidate) {
                        Ok(()) => error.to_owned(),
                        Err(cleanup) => format!("{error}; video candidate cleanup: {cleanup}"),
                    });
                }
            };
            Ok((
                candidate,
                serial,
                kind,
                band_settings,
                window_plan,
                resource_plan,
                world_plan,
                profiles,
                presentations,
            ))
        })();
        let (
            candidate,
            serial,
            kind,
            band_settings,
            window_plan,
            resource_plan,
            world_plan,
            profiles,
            presentations,
        ) = match candidate {
            Ok(candidate) => candidate,
            Err(error) => {
                let _ = self
                    .runtime
                    .server
                    .events
                    .console_formatted(format_args!("Video restart rejected: {error}\n"));
                return;
            }
        };
        self.video_active = true;
        let mut candidate = Some(candidate);
        let stopped = self
            .restart_client_modules(crate::modules::RendererRestart::Video, &mut source.driver());
        let published = stopped.and_then(|()| {
            resource_plan.validate(&self.console.cvars)?;
            world_plan.validate(&self.console.cvars)?;
            window_plan.validate(&self.console.cvars)?;
            if band_settings
                .as_ref()
                .is_some_and(|pending| !pending.unchanged(&self.console.cvars))
            {
                return Err("CPU band setting changed during restart".into());
            }
            self.refresh_resource_settings()?;
            // The sole asset owner is moved only across this cold, callback-free
            // transaction. Restore it before any CLIENT can execute again.
            let mut owner = self
                .runtime
                .model_assets
                .take()
                .ok_or("missing model assets")?;
            let ui_fullscreen = self.renderer_fullscreen_ui();
            let vfs = std::mem::take(&mut self.runtime.vfs);
            let result = owner.image_transaction(
                &vfs,
                Some(world_plan.settings()),
                crate::model_assets::ImageRefreshMode::Reload,
                |owner, changed| {
                    let limits = self
                        .runtime
                        .scene
                        .as_ref()
                        .ok_or("missing scene composer")?
                        .limits();
                    if changed.iter().any(|&world| {
                        self.runtime
                            .scene
                            .as_ref()
                            .is_none_or(|scene| !scene.has_world(world))
                    }) {
                        return Err("rebuilt world has no scene scratch".into());
                    }
                    let refresh = owner.refresh_images(
                        &vfs,
                        &profiles,
                        &presentations,
                        resource_plan.selections(),
                        resource_plan.finish(),
                        ui_fullscreen,
                        |assets| {
                            candidate
                                .as_mut()
                                .ok_or("replacement renderer was consumed")?
                                .prepare_assets(assets, limits)
                        },
                    )?;
                    let configuration = candidate
                        .as_ref()
                        .ok_or("replacement renderer was consumed")?
                        .configuration();
                    let (width, height) =
                        (configuration.display.width, configuration.display.height);
                    let scene = self
                        .runtime
                        .scene
                        .as_mut()
                        .ok_or("missing scene composer")?;
                    let graphics = scene.load_video_graphics(
                        &vfs,
                        owner,
                        &refresh,
                        ui_fullscreen,
                        width,
                        height,
                    )?;
                    // Graphics use the same registration owner, so admit their final
                    // suffix before staging numeric CLIENT and shader-bank selectors.
                    let counts = owner.assets().counts();
                    candidate
                        .as_mut()
                        .ok_or("replacement renderer was consumed")?
                        .prepare_assets(owner.assets(), limits)?;
                    owner
                        .publish_admission(counts)
                        .map_err(|error| format!("video resource publication: {error:?}"))?;
                    let mut clients = Vec::new();
                    for (index, resources) in self.runtime.client_resources.iter().enumerate() {
                        if let Some(resources) = resources {
                            clients.push((
                                index,
                                resources
                                    .state
                                    .stage_image_refresh(&refresh, owner.catalog())?,
                            ));
                        }
                    }
                    let roles = self.stage_video_image_roles(&refresh, width, height)?;
                    let scene = self
                        .runtime
                        .scene
                        .as_mut()
                        .ok_or("missing scene composer")?;
                    let staged = scene.stage_video_graphics(
                        owner.assets(),
                        graphics,
                        &refresh,
                        owner.catalog().max_choice_nodes(),
                    )?;
                    if staged.limits() != limits {
                        candidate
                            .as_mut()
                            .ok_or("replacement renderer was consumed")?
                            .prepare_assets(owner.assets(), staged.limits())?;
                    }
                    if candidate
                        .as_ref()
                        .and_then(|candidate| candidate.admitted_assets())
                        != Some(counts)
                    {
                        return Err("video restart resource prefix".into());
                    }
                    for registration in staged.graphic_registrations() {
                        self.renderer_contexts.ensure_owner(
                            qa_render::shader::registration::ShaderOwner::Engine,
                            registration.profile,
                            registration.files,
                            owner.assets(),
                            owner.shader_occurrence_budget(),
                            None,
                        )?;
                    }
                    let appends = self
                        .renderer_contexts
                        .stage_graphics(staged.graphic_registrations())?;
                    resource_plan.validate(&self.console.cvars)?;
                    world_plan.validate(&self.console.cvars)?;
                    window_plan.validate(&self.console.cvars)?;
                    let configuration = source.publish_video(
                        candidate
                            .take()
                            .ok_or("replacement renderer was consumed")?,
                    )?;
                    for append in appends {
                        append.commit();
                    }
                    staged.commit(kind);
                    for (index, staged) in clients {
                        if let Some(resources) = self.runtime.client_resources[index].as_mut() {
                            resources.state.commit_image_refresh(staged);
                        }
                    }
                    for seat in 0..SeatId::COUNT {
                        if let Some((world, previous)) = self.local_worlds[seat]
                            && let Some(next) = refresh.presentation(world, previous)
                        {
                            self.local_worlds[seat] = Some((world, next.id));
                            self.local_presentations[seat] = next.defaults;
                        }
                    }
                    self.commit_video_image_roles(roles);
                    Ok(configuration)
                },
            );
            self.runtime.vfs = vfs;
            self.runtime.model_assets = Some(owner);
            if result.is_err() {
                let catalog = self
                    .runtime
                    .model_assets
                    .as_ref()
                    .map(|owner| owner.catalog());
                self.renderer_contexts.retain_profiles(|profile| {
                    catalog.is_some_and(|catalog| catalog.profiles().any(|entry| entry == profile))
                });
            }
            result
        });
        if let Some(candidate) = candidate {
            if let Err(error) = source.discard_video(candidate) {
                let _ = self
                    .runtime
                    .server
                    .events
                    .console_formatted(format_args!("Video candidate cleanup: {error}\n"));
            }
        }
        match published {
            Ok(configuration) => {
                let display = configuration.display;
                self.runtime.renderer_configuration = Some(configuration);
                self.video_serial = serial;
                if let Err(error) = window_plan.apply(&mut self.console.cvars, &display) {
                    let _ =
                        self.runtime.server.events.console_formatted(format_args!(
                            "Window latch publication: {error:?}\n"
                        ));
                }
                window_plan.report_outcome(
                    source.video_window_request(),
                    &mut self.runtime.server.events,
                );
                if let Err(error) = resource_plan.apply(&mut self.console.cvars) {
                    let _ = self.runtime.server.events.console_formatted(format_args!(
                        "Render resource latch publication: {error:?}\n"
                    ));
                }
                if let Err(error) = world_plan.apply(&mut self.console.cvars) {
                    let _ = self.runtime.server.events.console_formatted(format_args!(
                        "World rendering latch publication: {error:?}\n"
                    ));
                }
                match kind {
                    crate::renderer::Kind::Cpu => {
                        if let Some(pending) = &band_settings
                            && let Err(error) = pending.apply(&mut self.console.cvars)
                        {
                            let _ = self.runtime.server.events.console_formatted(format_args!(
                                "CPU band latch publication: {error:?}\n"
                            ));
                        }
                    }
                    crate::renderer::Kind::Gl => {
                        self.console
                            .cvars
                            .set_latch_active(self.cpu_band_view.canonical(), false);
                    }
                }
                let _ = self
                    .runtime
                    .server
                    .events
                    .console_bytes(b"Renderer restarted.\n");
            }
            Err(error) => {
                let _ = self.runtime.server.events.console_formatted(format_args!(
                    "Video restart rejected after guest shutdown: {error}\n"
                ));
            }
        }
        self.video_active = false;
    }

    pub fn native_shaders(&self) -> &dyn qa_render::shader::reference::NativeShaders {
        &self.renderer_contexts
    }
    pub(crate) fn retain_command_producers(
        &mut self,
        retained: impl FnMut(qa_core::primitives::CommandProducer) -> bool,
    ) {
        if let Err(error) = self.console.retain_producers(&mut self.runtime, retained) {
            let _ = self
                .runtime
                .server
                .events
                .console_formatted(format_args!("Module command retirement: {error:?}\n"));
        }
    }

    pub(crate) fn update_console_policy(&mut self, initialize: bool) {
        let native = self
            .modules
            .as_ref()
            .is_some_and(|modules| modules.has_running_server());
        let providers = self.providers.iter().any(|provider| {
            self.modules
                .as_ref()
                .is_none_or(|modules| !modules.has_module(provider.module))
        });
        let builtin =
            self.runtime.collision.is_some()
                && self.runtime.server.clients.iter().any(|client| {
                    client.is_server_active() && client.module == ModuleId::default()
                });
        self.console
            .cvars
            .set_runtime_policy(qa_console::cvars::RuntimePolicy {
                initialized: initialize || self.console.cvars.runtime_policy().initialized,
                server_active: native || providers || builtin,
            });
    }

    fn intake(&mut self, source: &mut impl FrameSource) {
        // Keep the two physical intakes, including window state changes. During
        // replay, only a real window quit interrupts the recorded input.
        source.poll_events(&mut self.queue);
        let result = match self.journal.as_mut() {
            Some(qa_content::journal::Mode::Recording(recorder)) => {
                // Journal physical events before Com_EventLoop generates local
                // loopback traffic. Replay regenerates that traffic normally.
                self.queue.visit(|event| recorder.append(event))
            }
            Some(qa_content::journal::Mode::Replay(replay)) => {
                let mut quit = false;
                while let Some(event) = self.queue.pop() {
                    quit |= matches!(event.kind, EventKind::Quit);
                }
                if quit {
                    let _ = self.queue.push(qa_core::sys_events::SysEvent {
                        time: self.time,
                        kind: EventKind::Quit,
                    });
                }
                replay.enqueue_drain(&mut self.queue).map(|more| {
                    if !more {
                        let _ = self.queue.push(qa_core::sys_events::SysEvent {
                            time: self.time,
                            kind: EventKind::Quit,
                        });
                    }
                })
            }
            None => Ok(()),
        };
        if let Err(error) = result {
            qa_console::logger::console(format_args!("event journal: {error:?}\n"));
            self.journal_error = Some(error);
            self.runtime.quit = true;
        }
    }
    pub fn record_journal(
        &mut self,
        path: &std::path::Path,
    ) -> Result<(), qa_formats::FormatError> {
        self.flush_journal()?;
        self.journal = Some(qa_content::journal::Mode::Recording(
            qa_content::journal::Recorder::create(path, 64 * 1024)?,
        ));
        Ok(())
    }
    pub fn replay_journal(
        &mut self,
        path: &std::path::Path,
    ) -> Result<(), qa_formats::FormatError> {
        self.flush_journal()?;
        self.journal = Some(qa_content::journal::Mode::Replay(
            qa_content::journal::Replay::open(path, self.queue.payload_capacity())?,
        ));
        Ok(())
    }
    pub fn flush_journal(&mut self) -> Result<(), qa_formats::FormatError> {
        if let Some(qa_content::journal::Mode::Recording(recorder)) = self.journal.as_mut() {
            recorder.flush()?;
        }
        Ok(())
    }

    /// The current global search list is parsed once per cvar change. Retained
    /// image bodies keep their first resolution until an explicit reload.
    pub fn refresh_resource_settings(&mut self) -> Result<(), &'static str> {
        let Some(owner) = self.runtime.model_assets.as_mut() else {
            return Ok(());
        };
        let (q1, q2) = self
            .model_controls
            .replacement_load(&self.console.cvars)
            .map_err(|_| "model load setting conversion")?;
        owner.set_model_replacement_load(q1, q2);
        let generation = self.console.cvars.view_generation(self.image_search);
        if self.image_search_generation == Some(generation) {
            return Ok(());
        }
        self.image_search_generation = Some(generation);
        let order = crate::render_settings::image_search(&self.console.cvars, self.image_search)?;
        owner.set_image_search_order(order);
        Ok(())
    }

    pub fn load(
        mut console: Console<Runtime>,
        mut runtime: Runtime,
        world_rate: TickRate,
        mut providers: Vec<Provider>,
    ) -> Result<Self, String> {
        let history_view = console
            .cvars
            .bind("cl_commandHistory", console.cvars.context())
            .ok_or("missing cl_commandHistory")?;
        let history = history_view.canonical();
        console
            .cvars
            .apply_latch(history_view)
            .map_err(|e| format!("command history setting: {e:?}"))?;
        let capacity = console.cvars.integer(history);
        if capacity < 64 {
            return Err("cl_commandHistory requires at least 64 command slots".into());
        }
        runtime
            .network
            .set_history_capacity(capacity as usize)
            .map_err(|e| format!("command history: {e:?}"))?;
        console.cvars.set_latch_active(history, true);
        runtime.prepare_traces(std::iter::empty())?;
        let localization = console
            .cvars
            .bind("loc_file", console.cvars.context())
            .ok_or("missing loc_file")?;
        let path = console
            .cvars
            .read(localization)
            .map_err(|_| "invalid loc_file")?;
        let path = if path.as_str().is_empty() {
            "localization/loc_english.txt"
        } else {
            path.as_str()
        };
        if let Some(file) = runtime.vfs.open(path.as_bytes()) {
            let bytes = runtime
                .vfs
                .read_to_vec(&file, 16 * 1024 * 1024, &mut runtime.script_reader)
                .map_err(|e| format!("localization: {e}"))?;
            runtime.localization = qa_content::localization::Catalog::load(&bytes)
                .map_err(|e| format!("localization: {e:?}"))?;
        }
        console.register("play", crate::audio::play);
        console.register("music", crate::audio::music);
        console.register("cd", crate::audio::cd);
        let connection_commands =
            ["connect", "disconnect", "reconnect"].map(|name| console.register_external(name));
        let [Some(connect), Some(disconnect), Some(reconnect)] = connection_commands else {
            return Err("connection command registration".into());
        };
        let connection_commands = [connect, disconnect, reconnect];
        let video_command = console
            .register_external("vid_restart")
            .ok_or("video restart command registration")?;
        let volume = console.cvars.find("s_volume").ok_or("missing s_volume")?;
        let music_volume = console
            .cvars
            .find("s_musicvolume")
            .ok_or("missing s_musicvolume")?;
        let particle_gravity = console.cvars.find("g_gravity").ok_or("missing g_gravity")?;
        let mouse_enabled = console.cvars.find("in_mouse").ok_or("missing in_mouse")?;
        let developer = console.cvars.find("developer").ok_or("missing developer")?;
        let renderer_smp = console.cvars.find("r_smp");
        let renderer_ui_fullscreen = console.cvars.find("r_uifullscreen");
        if let Some(handle) = renderer_ui_fullscreen {
            console
                .cvars
                .set_text(handle, "1")
                .map_err(|error| format!("renderer UI startup: {error:?}"))?;
        }
        let cpu_band_view = console
            .cvars
            .bind("r_cpuBands", qa_console::views::Context::default())
            .ok_or("missing CPU band setting")?;
        let maxfps = console
            .cvars
            .find("com_maxfps")
            .ok_or("missing com_maxfps")?;
        let zombie_time = console
            .cvars
            .find("sv_zombietime")
            .ok_or("missing sv_zombietime")?;
        let fov = console.cvars.find("cg_fov").ok_or("missing cg_fov")?;
        let q2_context = qa_console::views::Context {
            source: qa_core::primitives::RuleSetId::Quake2,
            ..Default::default()
        };
        let q1_context = qa_console::views::Context {
            source: qa_core::primitives::RuleSetId::Quake,
            seat: SeatId::FIRST,
            ..Default::default()
        };
        let q1_identity = (
            console
                .cvars
                .bind("_cl_name", q1_context)
                .ok_or("missing _cl_name")?,
            console
                .cvars
                .bind("_cl_color", q1_context)
                .ok_or("missing _cl_color")?,
        );
        let mut nq_identity = [q1_identity; SeatId::COUNT];
        for seat in SeatId::ALL.into_iter().skip(1) {
            let context = qa_console::views::Context { seat, ..q1_context };
            nq_identity[seat.index()] = (
                console
                    .cvars
                    .bind("_cl_name", context)
                    .ok_or("missing _cl_name")?,
                console
                    .cvars
                    .bind("_cl_color", context)
                    .ok_or("missing _cl_color")?,
            );
        }
        let q2_fov = console
            .cvars
            .bind("deathmatch", q2_context)
            .zip(console.cvars.bind("dmflags", q2_context))
            .ok_or("missing Q2 FOV policy cvars")?;
        let player_model_options = console
            .cvars
            .bind("cl_noskins", q2_context)
            .zip(console.cvars.bind("cl_vwep", q2_context))
            .ok_or("missing player model cvars")?;
        let player_skin_views = crate::client_scene::PlayerSkinViews::bind(&console.cvars)?;
        let qw_highchars = console
            .cvars
            .bind(
                "sv_highchars",
                qa_console::views::Context {
                    source: RuleSetId::QuakeWorld,
                    side: qa_console::catalog::Scope::Server,
                    ..Default::default()
                },
            )
            .ok_or("missing sv_highchars")?;
        let qport = console
            .cvars
            .bind(
                "net_qport",
                qa_console::views::Context {
                    source: RuleSetId::QuakeWorld,
                    side: qa_console::catalog::Scope::Client,
                    ..Default::default()
                },
            )
            .ok_or("missing native qport")?;
        let connect_packet_length = console
            .cvars
            .bind("net_maxmsglen", q2_context)
            .ok_or("missing native packet length")?;
        let allskins_command = console
            .register_external("allskins")
            .ok_or("player skin command registration")?;
        let skins_command = console
            .register_external("skins")
            .ok_or("player skin command registration")?;
        let [download, downloadstatus, stopdownload, retrydownload] = [
            "download",
            "downloadstatus",
            "stopdownload",
            "retrydownload",
        ]
        .map(|name| {
            console
                .register_external(name)
                .ok_or("download command registration")
        });
        let download_commands = [download?, downloadstatus?, stopdownload?, retrydownload?];
        let software_waterwarp = console
            .cvars
            .bind("sw_waterwarp", q2_context)
            .ok_or("missing software water warp cvar")?;
        let curve_error = crate::render_settings::curve_error_view(&console.cvars)?;
        let model_lod = crate::render_settings::model_lod_views(&console.cvars)?;
        let model_controls = qa_console::model::ModelHandles::bind(&console.cvars)?;
        let [q1, qw, q2, rr, q3] = RuleSetId::ALL
            .map(|source| crate::renderer::ViewPolicyViews::bind(&console.cvars, source));
        let view_policy = [q1?, qw?, q2?, rr?, q3?];
        let image_search = crate::render_settings::image_search_view(&console.cvars)?;
        let lighting_scales =
            ["r_ambientScale", "r_directedScale"].map(|name| console.cvars.find(name));
        let [Some(ambient), Some(directed)] = lighting_scales else {
            return Err("missing entity lighting scales".into());
        };
        let lighting_scales = [ambient, directed];
        let style_interpolation = console
            .cvars
            .find("cl_lerp_lightstyles")
            .ok_or("missing cl_lerp_lightstyles")?;
        let roll_angle = console
            .cvars
            .find("cl_rollangle")
            .ok_or("missing cl_rollangle")?;
        let roll_speed = console
            .cvars
            .find("cl_rollspeed")
            .ok_or("missing cl_rollspeed")?;
        let kick_angles = console
            .cvars
            .find("cl_kickangles")
            .ok_or("missing cl_kickangles")?;
        let smooth_clients = console
            .cvars
            .find("cg_smoothClients")
            .ok_or("missing cg_smoothClients")?;
        let time_nudge = console
            .cvars
            .find("cl_timeNudge")
            .ok_or("missing cl_timeNudge")?;
        let push_latency = console
            .cvars
            .bind(
                "pushlatency",
                qa_console::views::Context {
                    source: RuleSetId::QuakeWorld,
                    ..Default::default()
                },
            )
            .ok_or("missing pushlatency")?;
        let timescale = console.cvars.find("timescale").ok_or("missing timescale")?;
        let input_handles = qa_console::input_settings::InputHandles::load(&console.cvars)?;
        let movement_handles = qa_console::movement::MovementHandles::bind(&console.cvars)?;
        let collision_handles = qa_console::collision::CollisionHandles::bind(&console.cvars)?;
        let notify_time = console
            .cvars
            .find("con_notifytime")
            .ok_or("missing con_notifytime")?;
        let center_time = console
            .cvars
            .find("cg_centertime")
            .ok_or("missing cg_centertime")?;
        let team_chats_only = console
            .cvars
            .find("cg_teamChatsOnly")
            .ok_or("missing cg_teamChatsOnly")?;
        let packet_dup = console
            .cvars
            .find("cl_packetdup")
            .ok_or("missing cl_packetdup")?;
        let timeline = Timeline::load(world_rate, providers.iter().map(|p| (p.module, p.rate)))
            .map_err(|e| format!("provider clocks: {e:?}"))?;
        providers.sort_unstable_by_key(|p| p.module.0);
        let module_outputs = providers
            .iter()
            .map(|p| {
                if p.output.is_some() {
                    runtime
                        .server
                        .events
                        .bind(qa_core::events::OutputTarget::Module(p.module))
                        .ok_or("module output capacity")
                        .map(Some)
                } else {
                    Ok(None)
                }
            })
            .collect::<Result<Box<[_]>, _>>()?;
        Ok(Self {
            modules: None,
            renderer_contexts: crate::modules::renderer_contexts::Contexts::load(
                runtime.vfs.default_view(),
                0,
            ),
            execution: crate::modules::ExecutionSlot::reserve(1)?,
            cue_random: qa_core::random::Random::new(5489),
            console,
            runtime,
            queue: SysEventQueue::load(1024, 256 * 1024).map_err(|e| format!("{e:?}"))?,
            journal: None,
            journal_error: None,
            time: EventTime::default(),
            key_downs: 0,
            key_repeats: 0,
            close_acknowledged: 0,
            close_timed_out: 0,
            close_aborted: 0,
            close_without_transport: 0,
            developer,
            renderer_smp,
            renderer_ui_fullscreen,
            cpu_band_view,
            cpu_band_override: None,
            video_command,
            video_serial: 0,
            video_request: None,
            video_active: false,
            local_clients: [None; SeatId::COUNT],
            connection_commands,
            local_worlds: [None; SeatId::COUNT],
            local_presentations: [Default::default(); SeatId::COUNT],
            local_presentation_rules: [None; SeatId::COUNT],
            maxfps,
            zombie_time,
            fov,
            q2_fov,
            qw_highchars,
            qport,
            connect_packet_length,
            nq_identity,
            player_model_options,
            player_skin_views,
            allskins_command,
            skins_command,
            download_commands,
            download_defaults: None,
            software_waterwarp,
            curve_error,
            model_lod,
            model_controls,
            view_policy,
            image_search,
            image_search_generation: None,
            userinfo: (0..SeatId::COUNT)
                .map(|_| ClientUserinfo::default())
                .collect(),
            lighting_scales,
            style_interpolation,
            roll_angle,
            roll_speed,
            kick_angles,
            smooth_clients,
            time_nudge,
            push_latency,
            timescale,
            input_handles,
            movement_handles,
            collision_handles,
            notify_time,
            center_time,
            team_chats_only,
            packet_dup,
            volume,
            music_volume,
            particle_gravity,
            mouse_enabled,
            previous: None,
            loop_client_cursor: 0,
            command_tokens: Default::default(),
            timeline,
            providers: providers.into_boxed_slice(),
            module_outputs,
        })
    }

    pub fn frame(&mut self, source: &mut impl FrameSource, uncapped: bool) -> FrameResult {
        self.console.begin_frame();
        source.set_socket_submission_enabled(!matches!(
            self.journal,
            Some(qa_content::journal::Mode::Replay(_))
        ));
        let mut result = FrameResult::default();
        let mut now = source.begin_frame();
        // Native lastTime starts at zero and is reset if the clock goes back.
        let previous_ms = self
            .previous
            .unwrap_or_default()
            .milliseconds()
            .min(now.as_millis().min(u128::from(u64::MAX)) as u64);
        let fps = self.console.cvars.integer(self.maxfps);
        // Preserve Com_Frame's integer-millisecond cap, including uncapped
        // values <= 0 and its zero period above 1000 fps.
        let period_ms = if uncapped
            || fps <= 0
            || matches!(self.journal, Some(qa_content::journal::Mode::Replay(_)))
        {
            0
        } else {
            (1000 / fps) as u64
        };
        let deadline_ns = EventTime::from_milliseconds(previous_ms + period_ms).0;
        let deadline = Duration::from_nanos(deadline_ns);
        while now < deadline && !self.runtime.quit {
            now = source.wait_time(deadline - now);
        }
        // Owner ruling: both intake points physically poll SDL/stdin/UDP, and
        // neither the cap wait nor any other frame phase performs intake.
        source.request_mouse_capture(self.mouse_capture_requested());
        self.intake(source);
        self.drain_with_driver(&mut result, &mut source.driver());
        self.retire_closing_clients();
        self.retire_stale_modules(&mut source.driver());
        self.update_console_policy(true);
        self.execute_commands_with_driver(&mut source.driver());
        if self.runtime.quit {
            source.request_mouse_capture(false);
            self.shutdown_modules_with_driver(&mut source.driver());
            self.dispatch_output(source, &mut result);
            return result;
        }
        self.initialize_modules_with_driver(crate::modules::Phase::Server, &mut source.driver());
        let server_time = self.time;
        let frame_span = EventTime(server_time.since(self.previous.unwrap_or_default()));
        let simulation = Stopwatch::start();
        while let Some(tick) = self.timeline.next_tick(server_time) {
            result.server_ticks += 1;
            match tick.target {
                TickTarget::World => {
                    self.runtime.server.world_time = tick.end;
                    self.runtime.server.world_frame = tick.index;
                    self.runtime
                        .server
                        .build_bot_commands(tick.start, tick.end, |rules| {
                            self.input_handles.policy(&self.console.cvars, rules)
                        });
                    self.run_pending_commands(frame_span, &mut source.driver());
                }
                TickTarget::Provider(_) => {
                    let slot = tick.source_slot - 1;
                    let provider = self.providers[slot];
                    (provider.frame)(self, tick, &mut source.driver());
                    if let (Some(consume), Some(id)) =
                        (provider.output, self.module_outputs[slot].as_mut())
                    {
                        let server = &mut self.runtime.server;
                        if !server.events.resume_if_needed(id) {
                            continue;
                        }
                        let clients = &mut server.clients;
                        qa_core::events::dispatch_consumer(
                            &mut server.events,
                            *id,
                            &mut |record, texts: &mut qa_core::events::TextStore| {
                                consume(clients, tick, record, texts)
                            },
                        );
                    }
                }
            }
        }
        self.run_pending_commands(frame_span, &mut source.driver());
        self.publish_module_client_state();
        result.simulation_ns = simulation.elapsed().as_nanos() as u64;
        self.publish_module_game_states();
        self.publish_module_configs();
        self.publish_module_signon();
        for index in 0..self.runtime.snapshots.len() {
            if self.runtime.snapshots[index].is_none() {
                continue;
            }
            if self.runtime.config_publications[index].game_state_waiting() {
                continue;
            }
            let id = ClientId(index as u32);
            let observation = self.snapshot_observation(id, server_time);
            self.runtime.send_snapshot(
                id,
                server_time,
                observation,
                source,
                |module, ordinal| self.modules.as_ref()?.bound_sound(module, ordinal),
                |module, ordinal, world, usage| {
                    self.modules
                        .as_ref()?
                        .model_name(module, ordinal, world, Some(usage))
                },
            );
        }
        // Immediate server->client packets take this path in the same frame.
        source.request_mouse_capture(self.mouse_capture_requested());
        self.intake(source);
        self.drain_with_driver(&mut result, &mut source.driver());
        self.retire_closing_clients();
        self.previous = Some(server_time);
        self.retire_stale_modules(&mut source.driver());
        self.update_console_policy(true);
        self.execute_commands_with_driver(&mut source.driver());
        if self.runtime.quit {
            source.request_mouse_capture(false);
            self.shutdown_modules_with_driver(&mut source.driver());
            self.dispatch_output(source, &mut result);
            return result;
        }
        self.flush_client_connect_requests(source);
        self.restart_video(source);
        if let Err(error) = self.restart_client_modules(
            crate::modules::RendererRestart::ChangedClient,
            &mut source.driver(),
        ) {
            let _ = self
                .runtime
                .server
                .events
                .console_formatted(format_args!("CLIENT renderer reset: {error}\n"));
        }
        self.rebind_client_view_modules();
        self.client_configurations(&mut source.driver());
        self.register_client_model_assets(source.asset_preparer());
        self.flush_startup_replies();
        let client = Stopwatch::start();
        let command_times = std::array::from_fn(|seat| {
            self.local_clients[seat]
                .and_then(|id| {
                    self.runtime.snapshots[id.0 as usize]
                        .as_ref()?
                        .interpolation()
                })
                .map(|sample| sample.time)
        });
        self.advance_client_presentation(&mut source.driver());
        self.flush_closing_client_controls(source);
        self.queue_client_userinfo();
        result.commands = self.client_frame(command_times, source);
        let native_owners = std::array::from_fn(|seat| {
            self.local_clients[seat]
                .and_then(|id| self.runtime.snapshots.get(id.0 as usize)?.as_ref())
                .is_some_and(|snapshot| snapshot.owns_client_view())
        });
        if let Some(scene) = &mut self.runtime.scene {
            scene.begin_frame();
            scene.native_owners(native_owners);
        }
        self.initialize_modules_with_driver(crate::modules::Phase::Client, &mut source.driver());
        self.prime_q3_clients();
        self.client_module_frames_with_driver(&mut source.driver());
        self.publish_client_feedback();
        self.publish_client_loops();
        result.client_ns = client.elapsed().as_nanos() as u64;
        self.dispatch_output(source, &mut result);
        result.input_ns = source.elapsed().as_nanos() as u64;
        let views = self.client_views();
        for (seat, view) in views.iter().enumerate() {
            if let (Some(view), Some(client)) = (view, self.local_clients[seat])
                && let Some(snapshot) = self.runtime.snapshots[client.0 as usize].as_mut()
            {
                let axes = if native_owners[seat] {
                    self.runtime
                        .scene
                        .as_ref()
                        .and_then(|scene| scene.submitted_axes(seat))
                } else {
                    Some(qa_core::math::angle_vectors(view.angles).axes())
                };
                if let Some(axes) = axes {
                    snapshot.submitted_view(axes);
                }
            }
        }
        self.capture_graphics_owners();
        let mut composition = self.runtime.scene.take();
        source.render(
            composition.as_mut(),
            self.runtime
                .model_assets
                .as_ref()
                .map(crate::model_assets::ModelAssets::view),
            &self.renderer_contexts,
            &views,
            &self.client_scene_inputs(),
            &self.client_hud_inputs(),
            qa_console::presentation::ConsoleView {
                editor: &self.console.editor,
                scrollback: &self.console.scrollback,
                texts: &self.runtime.server.events.texts,
                rules: self.console.cvars.context().source,
                time: self.time,
            },
        );
        if let Some(scene) = &mut composition {
            scene.discard_frame();
        }
        self.runtime.scene = composition;
        source.present();
        result.total_ns = source.elapsed().as_nanos() as u64;
        result
    }

    #[expect(
        clippy::question_mark,
        reason = "Preserve the explicit disconnected-client filter before constructing each seat view"
    )]
    pub fn client_views(&self) -> [Option<ClientView>; SeatId::COUNT] {
        let curve_error =
            crate::render_settings::curve_error(&self.console.cvars, self.curve_error);
        let model_lod = crate::render_settings::model_lod(&self.console.cvars, self.model_lod);
        let model_replacements =
            crate::render_settings::model_replacements(&self.console.cvars, self.model_controls)
                .ok();
        std::array::from_fn(|seat| {
            let (client, row, player) = self.local_seat(seat)?;
            let (world, world_presentation) = self.local_worlds[seat]?;
            let rules = self
                .runtime
                .model_assets
                .as_ref()
                .and_then(|owner| {
                    owner
                        .assets()
                        .world_presentation_registration(world_presentation)
                })
                .map(|entry| entry.profile.rules)
                .or(self.local_presentation_rules[seat])
                .or_else(|| {
                    self.runtime.client_resources[client.0 as usize]
                        .as_ref()?
                        .state
                        .models
                        .profile()
                        .map(|profile| profile.rules)
                })
                .unwrap_or(row.client_rules);
            let (policy, near) = self.view_policy[rules as usize]
                .read(&self.console.cvars)
                .ok()?;
            let camera = self.client_camera(client, player);
            let mut presentation = self.local_presentations[seat];
            presentation.software_waterwarp &= self
                .console
                .cvars
                .numeric(self.software_waterwarp)
                .unwrap_or(0.0)
                != 0.0;
            Some(ClientView {
                world,
                world_presentation,
                presentation,
                presentation_rules: rules,
                origin: camera.origin,
                angles: self.client_angles(client, row, player),
                fov_x: camera
                    .view
                    .fov_x
                    .unwrap_or_else(|| self.console.cvars.value(self.fov)),
                screen_blend: camera.view.screen_blend,
                damage_blend: camera.view.damage_blend,
                flags: camera.view.flags,
                time_ms: self.time.milliseconds(),
                model_time: self
                    .runtime
                    .snapshots
                    .get(client.0 as usize)
                    .and_then(Option::as_ref)
                    .and_then(|snapshot| snapshot.interpolation())
                    .map(|interpolation| interpolation.time),
                lighting_scales: self
                    .lighting_scales
                    .map(|handle| self.console.cvars.value(handle)),
                curve_error,
                model_lod,
                model_replacements: model_replacements?,
                policy,
                near,
            })
        })
    }

    fn client_camera(
        &self,
        client: ClientId,
        player: &qa_core::primitives::PlayerState,
    ) -> qa_network::projection::PlayerViewSample {
        self.runtime.snapshots[client.0 as usize]
            .as_ref()
            .and_then(|snapshot| {
                self.runtime
                    .network
                    .get(client, qa_core::loopback::Endpoint::Client)
                    .map(|connection| {
                        snapshot.view_presentation(&connection.channel, player, |number| {
                            self.client_entities(client, player, self.alias_roll())?
                                .get(number)
                                .map(|pose| pose.origin)
                        })
                    })
            })
            .unwrap_or(qa_network::projection::PlayerViewSample {
                origin: player.eye(),
                view: player.view,
            })
    }

    fn client_angles(
        &self,
        client: ClientId,
        row: &qa_session::clients::Client,
        player: &qa_core::primitives::PlayerState,
    ) -> qa_core::primitives::Vec3 {
        let Some(snapshot) = self.runtime.snapshots[client.0 as usize].as_ref() else {
            return player.view_angles;
        };
        if snapshot.owns_client_view() {
            return player.view_angles;
        }
        let Some(interpolation) = snapshot.interpolation() else {
            return player.view_angles;
        };
        let angles = self
            .runtime
            .network
            .get(client, qa_core::loopback::Endpoint::Client)
            .map_or(player.view_angles, |connection| {
                snapshot.view_angles(
                    &connection.channel,
                    player,
                    snapshot.view_source().is_none_or(|source| {
                        self.console.cvars.integer_in(self.kick_angles, source) != 0
                    }),
                )
            });
        qa_ui::hud::view_angles(&row.hud, angles, player.health, interpolation.time)
    }

    /// Transition due frames from the connection ring without replacing an
    /// active envelope with a snapshot ahead of the presentation clock.
    pub fn publish_client_feedback(&mut self) {
        for snapshot in self.runtime.snapshots.iter_mut().flatten() {
            let Some(_seat) = snapshot.seat() else {
                continue;
            };
            let Some(client) = self
                .runtime
                .server
                .clients
                .get_mut(snapshot.client.0 as usize)
            else {
                continue;
            };
            if client.connection.is_some()
                && let Some(connection) = self
                    .runtime
                    .network
                    .get(snapshot.client, qa_core::loopback::Endpoint::Client)
            {
                if let Some(frame @ qa_network::snapshots::ReceivedFrame::NetQuake(_)) =
                    connection.channel.current_snapshot()
                    && let Some(resources) = self
                        .runtime
                        .client_resources
                        .get_mut(snapshot.client.0 as usize)
                        .and_then(Option::as_mut)
                {
                    resources.state.animation.retire_missing(|number| {
                        crate::client_scene::ClientEntities::row(frame, number).is_some()
                    });
                }
                snapshot.publish_feedback(
                    client.entity,
                    &connection.channel,
                    self.runtime
                        .client_resources
                        .get(snapshot.client.0 as usize)
                        .and_then(Option::as_ref)
                        .map(|row| &row.state),
                    self.runtime.server.presentation,
                    &mut client.hud,
                    &mut self.runtime.server.events,
                    self.time,
                );
            }
        }
    }

    pub(crate) fn publish_client_values(
        &mut self,
        id: ClientId,
        rules: qa_core::primitives::RuleSetId,
        module: ModuleId,
    ) {
        use qa_core::{
            events::FrameEvent,
            primitives::{NativeDelivery, NativeTarget},
        };
        let bindings = &self.runtime.catalog.hud.values.native(rules).stats;
        let server = &mut self.runtime.server;
        let client = &mut server.clients[id.0 as usize];
        let delivery = NativeDelivery {
            module,
            source: rules,
            target: NativeTarget::Client(id),
            origin: client.player.body.position,
            reliable: true,
            dupe_key: 0,
            component: 0,
            emission: None,
            check_portals: false,
        };
        client.value_publication.publish_changes(
            bindings,
            &client.player.values,
            |value_id, value| {
                server
                    .events
                    .push_delivered(
                        delivery,
                        FrameEvent::PlayerValue {
                            id: value_id,
                            value,
                        },
                    )
                    .is_ok()
            },
        );
    }

    pub(crate) fn smooth_clients(&self) -> bool {
        self.console.cvars.integer(self.smooth_clients) != 0
    }

    pub(crate) fn alias_roll(&self) -> qa_gameplay::rules::appearance::AliasRoll {
        qa_gameplay::rules::appearance::AliasRoll {
            angle: self
                .console
                .cvars
                .value_in(self.roll_angle, qa_core::primitives::RuleSetId::QuakeWorld),
            speed: self
                .console
                .cvars
                .value_in(self.roll_speed, qa_core::primitives::RuleSetId::QuakeWorld),
        }
    }

    /// Pose readers do not require a loaded renderer or model catalog.
    pub fn client_entities_inputs(
        &self,
    ) -> [Option<crate::client_scene::ClientEntities<'_>>; SeatId::COUNT] {
        let alias_roll = self.alias_roll();
        std::array::from_fn(|seat| {
            let (client, _, player) = self.local_seat(seat)?;
            self.client_entities(client, player, alias_roll)
        })
    }

    fn client_entities<'a>(
        &'a self,
        client: ClientId,
        player: &'a qa_core::primitives::PlayerState,
        alias_roll: qa_gameplay::rules::appearance::AliasRoll,
    ) -> Option<crate::client_scene::ClientEntities<'a>> {
        let snapshot = self.runtime.snapshots.get(client.0 as usize)?.as_ref()?;
        let channel = &self
            .runtime
            .network
            .get(client, qa_core::loopback::Endpoint::Client)?
            .channel;
        Some(crate::client_scene::ClientEntities {
            client,
            player: Some(player),
            current: channel.current_snapshot(),
            previous: channel.previous_snapshot(),
            projection: snapshot.entity_projection(),
            appearance: snapshot
                .appearance_policy()?
                .with_smooth_clients(self.smooth_clients()),
            alias_roll,
            interpolation: snapshot.interpolation()?,
        })
    }

    pub fn client_scene_inputs(
        &self,
    ) -> [Option<crate::client_scene::ClientSceneInput<'_>>; SeatId::COUNT] {
        let entities = self.client_entities_inputs();
        std::array::from_fn(|seat| {
            let (_, row, player) = self.local_seat(seat)?;
            let entities = entities[seat]?;
            let client = entities.client;
            let snapshot = self.runtime.snapshots.get(client.0 as usize)?.as_ref()?;
            let resources = self
                .runtime
                .client_resources
                .get(client.0 as usize)?
                .as_ref()?;
            let channel = &self
                .runtime
                .network
                .get(client, qa_core::loopback::Endpoint::Client)?
                .channel;
            Some(crate::client_scene::ClientSceneInput {
                client,
                current: entities.current,
                previous: entities.previous,
                projection: entities.projection,
                appearance: entities.appearance,
                player_projection: snapshot.player_projection(),
                bindings: &resources.state.models,
                synchronization: Some(&resources.state.animation),
                static_entities: resources.state.static_entities(),
                models: self.runtime.model_assets.as_ref()?.catalog(),
                player,
                weapon_view_angles: snapshot.weapon_view_angles(
                    player.view_angles,
                    self.client_angles(client, row, player),
                ),
                alias_roll: entities.alias_roll,
                interpolation: entities.interpolation,
                gun_animation: snapshot.gun_animation(),
                gun_previous: snapshot
                    .gun_animation()
                    .and_then(|animation| animation.previous)
                    .and_then(|sequence| channel.snapshot(sequence)),
                view_entity: snapshot.view_entity(channel.current_snapshot()),
            })
        })
    }

    pub fn client_hud_inputs(&self) -> [Option<crate::renderer::hud::HudView<'_>>; SeatId::COUNT] {
        std::array::from_fn(|seat| {
            let (client, row, _) = self.local_seat(seat)?;
            let resources = self
                .runtime
                .client_resources
                .get(client.0 as usize)?
                .as_ref()?;
            let snapshot = self.runtime.snapshots.get(client.0 as usize)?.as_ref()?;
            Some(crate::renderer::hud::HudView {
                seat: SeatId::new(seat as u8),
                file_source: resources.state.file_source(),
                hud: &row.hud,
                texts: &self.runtime.server.events.texts,
                configs: &resources.state.configs,
                layout: resources.state.layout,
                values: &self.runtime.catalog.hud.values,
                localization: &self.runtime.localization,
                client,
                client_lifetime: row.entity,
                style_interpolation: self
                    .console
                    .cvars
                    .native_integer(self.style_interpolation, resources.state.layout.rules)
                    .unwrap_or(0),
                rules: row.client_rules,
                time: self.time,
                style_time: if snapshot.protocol
                    == qa_network::commands::packet::Protocol::QuakeWorld28
                {
                    qa_core::primitives::ThinkTime::Seconds(0.0).at(self.time)
                } else {
                    snapshot.interpolation()?.time
                },
                server_frame: self
                    .runtime
                    .network
                    .get(client, qa_core::loopback::Endpoint::Client)
                    .and_then(|connection| connection.channel.current_snapshot())
                    .map_or(0, |snapshot| snapshot.sequence()),
                show_text: self.console.editor.seat().is_none(),
            })
        })
    }

    fn advance_client_presentation(&mut self, driver: &mut crate::modules::Driver<'_>) {
        let mut push_latency_ms = self.console.cvars.numeric(self.push_latency).unwrap_or(0.0);
        if push_latency_ms > 0.0
            && self
                .runtime
                .snapshots
                .iter()
                .flatten()
                .any(crate::snapshots::SnapshotBinding::uses_realtime_prediction)
        {
            if let Err(error) = self.console.cvars.force_write(self.push_latency, "0") {
                let _ = self
                    .runtime
                    .server
                    .events
                    .console_formatted(format_args!("CLIENT pushlatency: {error:?}\n"));
            }
            push_latency_ms = 0.0;
        }
        let options = crate::snapshots::ClockOptions {
            time_nudge_ms: self.console.cvars.integer(self.time_nudge),
            timescale: self.console.cvars.value(self.timescale),
            // Native local-server pause and modified frame duration remain a
            // separate capability; queued time is not silently rescaled here.
            paused: false,
            push_latency_ms,
        };
        for slot in 0..self.runtime.snapshots.len() {
            let failed = self.runtime.snapshots[slot].as_mut().and_then(|snapshot| {
                if snapshot.presentation_closed() {
                    return None;
                }
                let connection = self
                    .runtime
                    .network
                    .get(snapshot.client, qa_core::loopback::Endpoint::Client)?;
                snapshot
                    .advance_presentation(&connection.channel, self.time, options)
                    .err()
                    .map(|error| (snapshot.client, error))
            });
            if let Some((client, error)) = failed {
                let _ = self
                    .runtime
                    .server
                    .events
                    .console_formatted(format_args!("CLIENT {} clock: {error:?}\n", client.0));
                self.disconnect_client_with_driver(client, driver);
            }
        }
    }

    /// Accepted CLIENT rows publish to Presentation once per connection, not
    /// once per seat/view. No native guest or SERVER entity pose is observed.
    pub fn publish_client_loops(&mut self) {
        let alias_roll = self.alias_roll();
        let smooth_clients = self.smooth_clients();
        let runtime = &mut self.runtime;
        let server = &mut runtime.server;
        let mut budget = (server.events.capacity() / 4).max(1);
        let count = runtime.snapshots.len();
        for step in 0..count {
            let index = (self.loop_client_cursor + step) % count;
            let Some(snapshot) = runtime.snapshots[index].as_mut() else {
                continue;
            };
            let Some(client) = server.clients.get(snapshot.client.0 as usize) else {
                continue;
            };
            if client.connection.is_none()
                || snapshot.presentation_closed()
                || server.entities.resolve(client.entity).is_none()
            {
                continue;
            }
            if snapshot.owns_client_audio()
                || runtime.client_resources[snapshot.client.0 as usize]
                    .as_ref()
                    .is_some_and(|resources| resources.awaiting_startup())
            {
                snapshot.maintain_client_audio(
                    client.entity,
                    server.presentation,
                    &mut server.events,
                    &mut budget,
                );
                if budget == 0 {
                    self.loop_client_cursor = (index + 1) % count;
                    break;
                }
                continue;
            }
            let Some(resources) = runtime.client_resources[snapshot.client.0 as usize].as_ref()
            else {
                continue;
            };
            let Some(connection) = runtime
                .network
                .get(snapshot.client, qa_core::loopback::Endpoint::Client)
            else {
                continue;
            };
            snapshot.publish_loops(
                snapshot
                    .seat()
                    .and_then(|seat| runtime.prediction.get(seat.index()))
                    .map(|prediction| &prediction.player),
                client.entity,
                server.presentation,
                &mut server.events,
                &connection.channel,
                &resources.state,
                alias_roll,
                smooth_clients,
                &mut budget,
            );
            if budget == 0 {
                self.loop_client_cursor = (index + 1) % count;
                break;
            }
        }
    }

    pub(crate) fn local_seat(
        &self,
        seat: usize,
    ) -> Option<(
        qa_core::primitives::ClientId,
        &qa_session::clients::Client,
        &qa_core::primitives::PlayerState,
    )> {
        local_seat(&self.runtime, &self.local_clients, seat)
    }

    fn mouse_capture_requested(&self) -> bool {
        !self.runtime.quit
            && self.console.cvars.integer(self.mouse_enabled) != 0
            && (self.native_input_wants_mouse()
                || self.local_worlds.iter().enumerate().any(|(seat, world)| {
                    self.runtime.input.key_catchers(SeatId::ALL[seat])
                        & qa_core::sys_events::key_catchers::CONSOLE
                        == 0
                        && world.is_some()
                        && self.local_seat(seat).is_some()
                }))
    }

    pub(crate) fn reject_client_startup(
        &mut self,
        client: ClientId,
        reason: std::fmt::Arguments<'_>,
    ) {
        if let Some(resources) = self.runtime.client_resources[client.0 as usize].as_mut() {
            resources.state.rejected += 1;
        }
        let _ = self
            .runtime
            .server
            .events
            .console_formatted(format_args!("Native CLIENT startup: {reason}\n"));
        self.received_client_disconnect(client);
    }

    fn prime_q3_clients(&mut self) {
        use crate::client_resources::startup::Q3Phase;
        for slot in 0..self.runtime.client_resources.len() {
            let client = ClientId(slot as u32);
            if self.runtime.snapshots[slot]
                .as_ref()
                .is_none_or(|binding| binding.presentation_closed())
            {
                continue;
            }
            let Some(resources) = self.runtime.client_resources[slot]
                .as_ref()
                .filter(|resources| resources.startup.q3() == Some(Q3Phase::Loading))
            else {
                continue;
            };
            if let Some(reason) = resources.state.q3_requirement() {
                self.reject_client_startup(client, format_args!("{reason}"));
                continue;
            }
            if !resources
                .state
                .q3_media_ready(self.runtime.scene.as_ref().is_some_and(|scene| {
                    scene.has_hud(
                        self.runtime.server.clients[slot].client_rules,
                        self.runtime.snapshots[slot]
                            .as_ref()
                            .and_then(|binding| binding.seat()),
                        resources.state.file_source(),
                    )
                }))
                || self
                    .runtime
                    .model_assets
                    .as_ref()
                    .is_none_or(|assets| assets.initial_registration())
                || self.runtime.scene.is_none()
                || self
                    .runtime
                    .network
                    .get(client, qa_core::loopback::Endpoint::Client)
                    .is_none_or(|connection| connection.channel.q3_startup().is_none())
            {
                continue;
            }
            let Some(seat) = self.runtime.snapshots[slot]
                .as_ref()
                .and_then(|binding| binding.seat())
            else {
                continue;
            };
            match self.client_view_ready(client, seat) {
                Ok(true) => {
                    if let Some(resources) = self.runtime.client_resources[slot].as_mut() {
                        resources.startup.q3_primed();
                    }
                }
                Ok(false) => {}
                Err(reason) => {
                    self.reject_client_startup(client, format_args!("{reason}"));
                }
            }
        }
    }

    fn flush_startup_replies(&mut self) {
        for slot in 0..self.runtime.client_resources.len() {
            self.flush_startup_reply(ClientId(slot as u32));
        }
    }

    fn report_connect_rejection(events: &mut qa_core::events::EventRing, reason: &[u8]) {
        let _ = events.console_parts(&[b"Connection rejected: ", reason, b"\n"]);
    }

    fn retire_unconnected_clients(&mut self, source: &mut impl FrameSource) {
        for slot in 0..self.runtime.server.clients.len() {
            let id = ClientId(slot as u32);
            let Some(connection) = self
                .runtime
                .network
                .get(id, qa_core::loopback::Endpoint::Client)
            else {
                continue;
            };
            if connection.phase.is_connected() {
                continue;
            }
            let rejected = matches!(
                connection.phase,
                qa_network::ingress::ConnectionPhase::Rejected
            );
            let closed = self.runtime.snapshots[slot]
                .as_ref()
                .is_some_and(|binding| binding.presentation_closed())
                || !self.runtime.server.clients[slot].is_active();
            if rejected || closed {
                self.disconnect_client_with_driver(id, &mut source.driver());
            }
        }
    }

    fn flush_client_connect_requests(&mut self, source: &mut impl FrameSource) {
        self.retire_unconnected_clients(source);
        let runtime = &mut self.runtime;
        let cvars = &self.console.cvars;
        let packet_length = self.connect_packet_length;
        runtime.network.send_connect_requests(
            self.time,
            |id, protocol, route, output| {
                let seat = runtime.snapshots.get(id.0 as usize)?.as_ref()?.seat()?;
                let mut parameters = qa_network::ingress::ConnectParameters::new(
                    ClientUserinfo::serialize(cvars, protocol, seat, route.peer, output).ok()?,
                );
                if protocol == qa_network::commands::packet::Protocol::Quake2Repro1038 {
                    parameters.packet_length = cvars.integer_view(packet_length).ok()?;
                }
                Some(parameters)
            },
            |route, bytes| {
                crate::transport::admit_output(
                    &mut runtime.loopback,
                    &mut |socket, to, bytes| source.send_packet(socket, to, bytes),
                    qa_core::loopback::Endpoint::Client,
                    route,
                    bytes,
                )
            },
            |_, _, incoming| {
                if let qa_network::ingress::Incoming::ConnectRejected { reason } = incoming {
                    Self::report_connect_rejection(&mut runtime.server.events, reason);
                }
                std::ops::ControlFlow::Continue(())
            },
        );
        self.retire_unconnected_clients(source);
    }

    fn flush_startup_reply(&mut self, client: ClientId) {
        use crate::client_resources::startup::{ResourceKind, StartupReply};
        use qa_network::{channel::TransmitError, commands::packet::Protocol};
        use std::fmt::Write;
        let slot = client.0 as usize;
        let Some(binding) = self
            .runtime
            .snapshots
            .get(slot)
            .and_then(Option::as_ref)
            .filter(|binding| {
                matches!(
                    binding.protocol,
                    Protocol::NetQuake15
                        | Protocol::QuakeWorld28
                        | Protocol::Quake2_34
                        | Protocol::Quake2Repro1038
                )
            })
        else {
            return;
        };
        let Some(seat) = binding.seat() else {
            return;
        };
        let protocol = binding.protocol;
        if self
            .runtime
            .network
            .get(client, qa_core::loopback::Endpoint::Client)
            .is_none_or(|connection| !connection.phase.is_connected())
        {
            return;
        }
        if self.runtime.server.clients.get(slot).is_none_or(|row| {
            row.connection != Some(qa_session::clients::Connection::Local)
                || self.runtime.server.entities.resolve(row.entity).is_none()
        }) {
            return;
        }
        let Some(resources) = self.runtime.client_resources[slot].as_ref() else {
            return;
        };
        if resources.startup.resources_needed() {
            if resources.state.presentation_reset_pending() {
                return;
            }
            if self.local_worlds[seat.index()].is_none() {
                if matches!(
                    protocol,
                    Protocol::QuakeWorld28 | Protocol::Quake2_34 | Protocol::Quake2Repro1038
                ) && !resources.state.world_managed()
                {
                    self.reject_client_startup(
                        client,
                        format_args!("required world model unavailable"),
                    );
                }
                return;
            }
            match resources
                .state
                .models
                .required_models(&resources.state.configs)
            {
                crate::client_scene::RequiredModels::Waiting => return,
                crate::client_scene::RequiredModels::Missing(ordinal) => {
                    self.reject_client_startup(
                        client,
                        format_args!("required model {ordinal} unavailable"),
                    );
                    return;
                }
                crate::client_scene::RequiredModels::Ready => {}
            }
            let checksums = resources.state.layout.resources[0]
                .get(1)
                .and_then(|row| resources.state.configs.config(row))
                .and_then(|row| {
                    self.runtime
                        .model_assets
                        .as_ref()?
                        .catalog()
                        .world_native_checksums(row.text, Some(resources.state.file_search()))
                });
            let checksum = checksums.and_then(|checksums| match protocol {
                Protocol::QuakeWorld28 => checksums.quakeworld_lumps,
                Protocol::Quake2_34 | Protocol::Quake2Repro1038 => Some(checksums.whole_file),
                _ => None,
            });
            if matches!(protocol, Protocol::Quake2_34 | Protocol::Quake2Repro1038) {
                let expected = resources
                    .state
                    .layout
                    .map_checksum()
                    .and_then(|index| resources.state.configs.config(index))
                    .filter(|row| !row.text.is_empty())
                    .map(|row| row.integer as u32);
                if checksum.is_none() || checksum != expected {
                    self.reject_client_startup(
                        client,
                        format_args!("Q2 local map checksum differs from server"),
                    );
                    return;
                }
            }
            if let Some(resources) = self.runtime.client_resources[slot].as_mut()
                && let Err(error) = resources.startup.resources_admitted(checksum)
            {
                self.reject_client_startup(client, format_args!("{error}"));
                return;
            }
        }
        loop {
            let Some(resources) = self.runtime.client_resources[slot].as_ref() else {
                return;
            };
            let skins_done = !self.player_skin_views.refresh_pending()
                && resources
                    .downloads
                    .as_ref()
                    .is_none_or(|downloads| downloads.skins_done());
            let Some(reply) = resources.startup.reply(skins_done) else {
                return;
            };
            let mut text =
                qa_core::text::FixedText::<{ qa_network::message::LEGACY_STRING_BYTES }>::default();
            let (name, color) = self.nq_identity[seat.index()];
            let formatted = match reply {
                StartupReply::Prespawn => text.set("prespawn"),
                StartupReply::Name => self
                    .console
                    .cvars
                    .read(name)
                    .map_err(|_| std::fmt::Error)
                    .and_then(|name| writeln!(text, "name \"{}\"", name.as_str())),
                StartupReply::Color => self
                    .console
                    .cvars
                    .numeric(color)
                    .map_err(|_| std::fmt::Error)
                    .and_then(|color| {
                        let color = color as i32;
                        writeln!(text, "color {} {}", color >> 4, color & 15)
                    }),
                StartupReply::Spawn => text
                    .set("spawn ")
                    .and_then(|()| text.append_bytes(resources.nq_spawn_parameters())),
                StartupReply::Begin => text.set("begin"),
                StartupReply::List {
                    kind,
                    server,
                    start,
                } => write!(
                    text,
                    "{} {server} {start}",
                    if kind == ResourceKind::Sound {
                        "soundlist"
                    } else {
                        "modellist"
                    }
                ),
                StartupReply::QwPrespawn {
                    server,
                    buffer,
                    checksum: Some(checksum),
                } => write!(text, "prespawn {server} {buffer} {}", checksum as i32),
                StartupReply::QwPrespawn {
                    server,
                    buffer,
                    checksum: None,
                } => write!(text, "prespawn {server} {buffer}"),
                StartupReply::QwSpawn { server, start } => write!(text, "spawn {server} {start}"),
                StartupReply::QwBegin { server } | StartupReply::Q2Begin { server } => {
                    write!(text, "begin {server}")
                }
                StartupReply::Q2List {
                    kind,
                    server,
                    start,
                } => write!(
                    text,
                    "{} {server} {start}",
                    match kind {
                        qa_network::commands::startup::ResourceStream::ConfigStrings =>
                            "configstrings",
                        qa_network::commands::startup::ResourceStream::Baselines => "baselines",
                    }
                ),
                StartupReply::New => text.set("new"),
            };
            if formatted.is_err()
                || text.as_bytes().iter().any(|&byte| byte == 0 || byte == 255)
                || protocol.client_string_policy().truncates(text.as_bytes())
            {
                self.reject_client_startup(
                    client,
                    format_args!("reply exceeds native string policy"),
                );
                return;
            }
            let Some(connection) = self
                .runtime
                .network
                .get_mut(client, qa_core::loopback::Endpoint::Client)
            else {
                return;
            };
            match connection
                .channel
                .queue_client_command(protocol, text.as_bytes())
            {
                Ok(_) => {
                    if let Some(resources) = self.runtime.client_resources[slot].as_mut() {
                        resources.startup.queued(reply);
                    }
                }
                Err(TransmitError::Full) => return,
                Err(error) => {
                    self.reject_client_startup(client, format_args!("{error}"));
                    return;
                }
            }
        }
    }

    fn queue_client_userinfo(&mut self) {
        use qa_network::commands::packet::Protocol;
        let revision = self.console.cvars.revision();
        let mut bytes = [0; qa_session::clients::USERINFO_BYTES];
        for seat in SeatId::ALL {
            let index = seat.index();
            let Some(id) = self.local_clients[index] else {
                let cache = &mut self.userinfo[index];
                cache.entity = None;
                cache.revision = None;
                cache.queued.clear();
                continue;
            };
            let Some(row) = self
                .runtime
                .server
                .clients
                .get(id.0 as usize)
                .filter(|row| row.is_active())
            else {
                continue;
            };
            let cache = &mut self.userinfo[index];
            if cache.entity != Some(row.entity) {
                cache.entity = Some(row.entity);
                cache.revision = None;
                cache.queued.clear();
            }
            if cache.revision == Some(revision) {
                continue;
            }
            let Some(connection) = self
                .runtime
                .network
                .get_mut(id, qa_core::loopback::Endpoint::Client)
            else {
                continue;
            };
            if !connection.phase.is_connected() {
                continue;
            }
            let Some(
                protocol @ (Protocol::QuakeWorld28
                | Protocol::Quake2_34
                | Protocol::Quake2Repro1038
                | Protocol::Quake3_68),
            ) = connection
                .commands
                .as_ref()
                .map(|commands| commands.protocol)
            else {
                continue;
            };
            let Ok(length) = ClientUserinfo::serialize(
                &self.console.cvars,
                protocol,
                seat,
                connection.route.peer,
                &mut bytes,
            ) else {
                continue;
            };
            let cache = &mut self.userinfo[index];
            if cache.queued.as_bytes() == &bytes[..length] {
                cache.revision = Some(revision);
                continue;
            }
            let queued = if protocol == Protocol::QuakeWorld28 {
                cache
                    .queue_qw(&mut connection.channel, &bytes[..length])
                    .is_ok()
            } else {
                connection
                    .channel
                    .queue_userinfo(protocol, &bytes[..length])
                    .is_ok()
                    && cache.queued.set_bytes(&bytes[..length]).is_ok()
            };
            if queued {
                cache.revision = Some(revision);
            }
        }
    }

    fn receive_client_userinfo(
        &mut self,
        id: ClientId,
        protocol: qa_network::commands::packet::Protocol,
        text: &[u8],
        driver: &mut crate::modules::Driver<'_>,
    ) -> Result<(), crate::modules::ClientCommandError> {
        use crate::modules::ClientCommandError;
        use qa_network::commands::packet::Protocol;
        let rules = protocol
            .userinfo_rules()
            .ok_or(ClientCommandError::Usercmd(
                qa_compat::services::CallError::Unsupported,
            ))?;
        self.runtime
            .server
            .replace_userinfo(id, text, rules)
            .map_err(|_| ClientCommandError::Usercmd(qa_compat::services::CallError::Text))?;
        if self.runtime.server.module_moves(id).is_some() {
            return self
                .notify_userinfo_module_client_with_driver(id, driver)
                .map(|_| ());
        }
        let policy = match protocol {
            Protocol::Quake2_34 => Some((90, Some(0x8000))),
            Protocol::Quake2Repro1038 => Some((1, None)),
            _ => None,
        };
        if let Some((below_minimum, fixed_mask)) = policy {
            let raw = qa_core::text::info_value(text, b"fov", false)
                .map_or(&b""[..], |range| &text[range]);
            let value = qa_core::text::atoi_prefix(raw);
            let (deathmatch, flags) = self.q2_fov;
            let fixed = fixed_mask.is_some_and(|mask| {
                self.console.cvars.numeric(deathmatch).unwrap_or(0.0) != 0.0
                    && self.console.cvars.integer_view(flags).unwrap_or(0) & mask != 0
            });
            let fov = if fixed {
                90
            } else if value < 1 {
                below_minimum
            } else {
                value.min(160)
            };
            self.runtime.server.clients[id.0 as usize].player.view.fov_x = Some(fov as f32);
        }
        Ok(())
    }

    fn admit_received_presentation(&mut self, client: ClientId) -> bool {
        if self.runtime.ensure_client_presentation_reset(client) {
            true
        } else {
            if let Some(resources) = self.runtime.client_resources[client.0 as usize].as_mut() {
                resources.state.rejected += 1;
            }
            false
        }
    }

    fn reset_received_client(&mut self, client: ClientId, seat: SeatId) {
        let lifetime = self.runtime.server.clients[client.0 as usize].entity;
        if let Some(resources) = self.runtime.client_resources[client.0 as usize].as_mut() {
            resources.state.request_presentation_reset(lifetime);
        }
        let _ = self.runtime.ensure_client_presentation_reset(client);
        let prediction = &mut self.runtime.prediction[seat.index()];
        prediction.clear_history();
        prediction.player.clear_received();
        if let Some(collision) = self.runtime.client_collision[client.0 as usize].as_mut() {
            collision.reset();
        }
    }

    pub(crate) fn begin_client_map_change(&mut self, client: ClientId) {
        let lifetime = self.runtime.server.clients[client.0 as usize].entity;
        if let Some(resources) = self.runtime.client_resources[client.0 as usize].as_mut() {
            resources
                .startup
                .changing(self.runtime.snapshots[client.0 as usize].as_ref().map_or(
                    qa_network::commands::packet::Protocol::QuakeWorld28,
                    |binding| binding.protocol,
                ));
            resources.state.request_presentation_reset(lifetime);
        }
        if let Some(binding) = self.runtime.snapshots[client.0 as usize].as_mut() {
            binding.begin_map_change();
            if let Some(seat) = binding.seat() {
                let mode = &mut self.runtime.prediction[seat.index()].player.movement.mode;
                if matches!(
                    mode,
                    qa_core::primitives::MovementMode::Intermission
                        | qa_core::primitives::MovementMode::SinglePlayerIntermission
                ) {
                    *mode = if binding.spectator() {
                        qa_core::primitives::MovementMode::Spectator
                    } else {
                        qa_core::primitives::MovementMode::Walk
                    };
                }
            }
        }
        let _ = self.runtime.ensure_client_presentation_reset(client);
    }

    fn receive_server_commands(
        &mut self,
        client: ClientId,
        seat: SeatId,
        text: &[u8],
        time: EventTime,
        protocol: qa_network::commands::packet::Protocol,
    ) {
        use crate::client_resources::startup::ServerControl;
        let mut remaining = text;
        while !remaining.is_empty() {
            let end = qa_core::text::command_separator(
                remaining,
                true,
                protocol.rule_set() == RuleSetId::Quake3,
            );
            let line = &remaining[..end];
            remaining = &remaining[end + usize::from(end < remaining.len())..];
            let result = self.runtime.client_resources[client.0 as usize]
                .as_mut()
                .ok_or("missing QW CLIENT resource owner")
                .and_then(|resources| resources.receive_server_command(line, protocol));
            match result {
                Ok(Some(control)) => {
                    if protocol != qa_network::commands::packet::Protocol::Quake2Repro1038
                        && matches!(control, ServerControl::Changing | ServerControl::Reconnect)
                        && self.runtime.client_resources[client.0 as usize]
                            .as_ref()
                            .is_some_and(|resources| resources.receiving_download())
                    {
                        continue;
                    }
                    match control {
                        ServerControl::Changing => self.begin_client_map_change(client),
                        ServerControl::Reconnect => {
                            if protocol == qa_network::commands::packet::Protocol::Quake2Repro1038
                                && self.runtime.client_resources[client.0 as usize]
                                    .as_ref()
                                    .is_some_and(|resources| resources.startup.reconnect_in_place())
                            {
                                if self.runtime.client_resources[client.0 as usize]
                                    .as_ref()
                                    .is_some_and(|resources| resources.receiving_download())
                                {
                                    continue;
                                }
                                self.begin_client_map_change(client);
                                if let Some(resources) =
                                    self.runtime.client_resources[client.0 as usize].as_mut()
                                {
                                    resources.startup.reconnect(protocol);
                                }
                                continue;
                            }
                            let context = qa_console::views::Context {
                                source: protocol.rule_set(),
                                seat,
                                event_time: Some(time),
                                file_search: self.runtime.client_resources[client.0 as usize]
                                    .as_ref()
                                    .and_then(|resources| resources.state.file_source()),
                                client_lifetime: Some((
                                    client,
                                    self.runtime.server.clients[client.0 as usize].entity,
                                )),
                                ..Default::default()
                            };
                            if let Err(error) = self
                                .console
                                .append_registered(self.connection_commands[2], context)
                            {
                                let _ = self.runtime.server.events.console_formatted(format_args!(
                                    "Native reconnect rejected: {error:?}\n"
                                ));
                            }
                        }
                        ServerControl::Skins => self.player_skin_views.request_refresh(),
                    }
                }
                Ok(None) => {}
                Err(error) => {
                    let _ = self.runtime.server.events.console_formatted(format_args!(
                        "Native server command rejected: {error}\n"
                    ));
                }
            }
        }
    }

    pub fn dispatch_output(&mut self, source: &mut impl FrameSource, result: &mut FrameResult) {
        self.runtime
            .flush_server_downloads(self.time, &mut self.console.cvars);
        let effect_clients = std::array::from_fn(|seat| {
            let (client, row, _) = self.local_seat(seat)?;
            Some(qa_render::effects::EffectClient {
                client,
                lifetime: row.entity,
            })
        });
        source.effect_clients(
            &effect_clients,
            self.time,
            self.console
                .cvars
                .value_in(self.particle_gravity, qa_core::primitives::RuleSetId::Quake),
        );
        // Channel replacement/protection observes this frame's listener even
        // before newly received camera feedback is consumed.
        source.listeners(
            &self.client_listeners(),
            self.console.cvars.value(self.volume),
        );
        let counts = crate::output::retire_resync_clients(self, &mut source.driver());
        result.output = crate::output::dispatch_frame(
            &mut self.runtime,
            source,
            &mut self.console.scrollback,
            &self.local_clients,
            self.time,
            f64::from(self.console.cvars.value(self.notify_time)),
            f64::from(self.console.cvars.value(self.center_time)),
            counts,
            |module, ordinal, world, usage| {
                self.modules
                    .as_ref()?
                    .model_name(module, ordinal, world, Some(usage))
            },
        );
        result.output_drains += 1;
        self.retire_closing_clients();
        // Feedback has reached the HUD cursor before camera/listener sampling.
        if result.output.view_feedback != 0 {
            source.listeners(
                &self.client_listeners(),
                self.console.cvars.value(self.volume),
            );
        }
        source.update_music(
            &self.runtime.vfs,
            self.console.cvars.value(self.music_volume),
        );
        source.mix_audio(self.runtime.sound_bank(), &self.client_entities_inputs());
    }

    fn client_listeners(&self) -> [Option<qa_audio::Listener>; SeatId::COUNT] {
        std::array::from_fn(|seat| {
            let (client, row, player) = self.local_seat(seat)?;
            if self.runtime.snapshots[client.0 as usize]
                .as_ref()
                .is_some_and(crate::snapshots::SnapshotBinding::owns_client_audio)
            {
                return None;
            }
            Some(qa_audio::Listener {
                negate_dot: false,
                client,
                entity: Some(
                    self.runtime.snapshots[client.0 as usize]
                        .as_ref()
                        .and_then(|snapshot| {
                            snapshot.view_entity(
                                self.runtime
                                    .network
                                    .get(client, qa_core::loopback::Endpoint::Client)
                                    .and_then(|connection| connection.channel.current_snapshot()),
                            )
                        })
                        .map_or(
                            qa_core::primitives::SoundEmitter::Entity(row.entity),
                            |slot| qa_core::primitives::SoundEmitter::Native {
                                namespace: qa_core::primitives::ResourceNamespace::Client(client),
                                slot: slot as i32,
                            },
                        ),
                ),
                origin: self.client_camera(client, player).origin,
                right: qa_core::math::angle_vectors(self.client_angles(client, row, player)).right,
            })
        })
    }

    pub(crate) fn received_client_disconnect(&mut self, id: ClientId) {
        self.close_client_downloads(id);
        // The network may be temporarily moved into receive_packet. Retire
        // presentation here; transport completion is observed after restoration.
        self.runtime.close_client_presentation(id);
        if self.local_clients.contains(&Some(id))
            && (0..SeatId::COUNT).all(|seat| self.local_seat(seat).is_none())
        {
            self.set_renderer_fullscreen_ui(true);
        }
    }

    fn retire_closing_clients(&mut self) {
        use qa_session::clients::ClientPhase;
        let seconds = self.console.cvars.integer(self.zombie_time);
        for slot in 0..self.runtime.server.clients.len() {
            let client = &self.runtime.server.clients[slot];
            let ClientPhase::Closing(closing) = client.phase else {
                continue;
            };
            if client.connection.is_none() {
                continue;
            }
            let id = ClientId(slot as u32);
            if closing.acknowledged {
                self.close_acknowledged += 1;
            } else if client.output.is_none() {
                self.close_without_transport += 1;
            } else if self
                .runtime
                .network
                .get(id, qa_core::loopback::Endpoint::Server)
                .is_none()
                || client
                    .output
                    .is_some_and(|output| self.runtime.server.events.needs_resync(output))
                || closing.final_output.is_some_and(|(output, sequence)| {
                    client.output != Some(output)
                        || !self.runtime.server.events.retained_for(output, sequence)
                })
            {
                self.close_aborted += 1;
            } else if qa_session::clients::close_timeout_expired(
                closing.started,
                self.time,
                seconds,
            ) {
                self.close_timed_out += 1;
            } else {
                continue;
            }
            self.retire_client(id, crate::modules::TraceAccess::empty());
        }
    }

    fn flush_closing_client_controls(&mut self, source: &mut impl FrameSource) {
        use qa_core::loopback::Endpoint;
        use qa_session::clients::ClientPhase;
        for slot in 0..self.runtime.server.clients.len() {
            let id = ClientId(slot as u32);
            let closing = matches!(
                self.runtime.server.clients[slot].phase,
                ClientPhase::Closing(_)
            );
            let closed = self.runtime.snapshots[slot]
                .as_ref()
                .is_some_and(|binding| binding.presentation_closed());
            if !closing && !closed {
                continue;
            }
            let Some(connection) = self.runtime.network.get_mut(id, Endpoint::Client) else {
                if closed && !closing {
                    self.disconnect_client_with_driver(id, &mut source.driver());
                }
                continue;
            };
            if !connection.phase.is_connected() {
                self.disconnect_client_with_driver(id, &mut source.driver());
                continue;
            }
            let Some(binding) = self.runtime.snapshots[slot].as_mut() else {
                continue;
            };
            let route = connection.route;
            let mut complete = false;
            if connection.channel.pending_packet().is_some()
                || connection.channel.pending_fragments()
            {
                let submitted = crate::transport::send(
                    &mut connection.channel,
                    self.time,
                    &mut (),
                    |channel, ()| {
                        channel
                            .prepare(None, self.time)
                            .is_ok_and(|packet| packet.is_some())
                    },
                    |bytes| {
                        crate::transport::admit_output(
                            &mut self.runtime.loopback,
                            &mut |socket, to, bytes| source.send_packet(socket, to, bytes),
                            Endpoint::Client,
                            route,
                            bytes,
                        )
                    },
                    |channel, (), _| !channel.pending_fragments(),
                );
                if !submitted.complete {
                    continue;
                }
                complete = binding.close_control_pending;
                binding.close_control_pending = false;
            }
            if !complete {
                let Some(commands) = connection.commands.as_ref() else {
                    continue;
                };
                let Ok(maximum) = connection.channel.native_payload_capacity() else {
                    continue;
                };
                let Some(bytes) = self.runtime.output_native_bytes.get_mut(..maximum) else {
                    continue;
                };
                let Ok(length) = commands.encode_control(&connection.channel, bytes) else {
                    continue;
                };
                let mut included = length == 0;
                let submitted = crate::transport::send(
                    &mut connection.channel,
                    self.time,
                    &mut included,
                    |channel, _| {
                        channel
                            .prepare_move(&bytes[..length], self.time, None, None)
                            .is_ok_and(|packet| packet.is_some())
                    },
                    |bytes| {
                        crate::transport::admit_output(
                            &mut self.runtime.loopback,
                            &mut |socket, to, bytes| source.send_packet(socket, to, bytes),
                            Endpoint::Client,
                            route,
                            bytes,
                        )
                    },
                    |channel, included, disposition| {
                        *included |= disposition == qa_network::channel::Unreliable::Included;
                        *included && !channel.pending_fragments()
                    },
                );
                complete = submitted.complete;
                binding.close_control_pending = !complete
                    && (connection.channel.pending_packet().is_some()
                        || connection.channel.pending_fragments());
            }
            if complete && closed && !closing {
                self.disconnect_client_with_driver(id, &mut source.driver());
            }
        }
    }

    fn receive_packet(
        &mut self,
        socket: u16,
        from: qa_core::sys_events::Peer,
        bytes: &[u8],
        time: EventTime,
        driver: &mut crate::modules::Driver<'_>,
    ) {
        // Ownership moves temporarily; the callback retains neither the packet
        // nor an extra usercmd queue, and the same native channels are restored.
        let mut network = std::mem::replace(
            &mut self.runtime.network,
            qa_network::ingress::Connections::load(0),
        );
        let mut disconnected = None;
        let mut completed_close = None;
        let mut signon_client = None;
        network.receive(socket, from, bytes, time, |id, endpoint, incoming| {
            use qa_network::ingress::Incoming;
            match endpoint {
                qa_core::loopback::Endpoint::Client => {
                    if let Incoming::ConnectRejected { reason } = &incoming {
                        Self::report_connect_rejection(&mut self.runtime.server.events, reason);
                        self.disconnect_client_with_driver(id, driver);
                        disconnected = Some(id);
                        return std::ops::ControlFlow::Break(());
                    }
                    if matches!(incoming, Incoming::Disconnect { .. }) {
                        self.received_client_disconnect(id);
                        return std::ops::ControlFlow::Break(());
                    }
                    if self.runtime.snapshots[id.0 as usize].as_ref().is_some_and(|binding| binding.presentation_closed()) {
                        return std::ops::ControlFlow::Continue(());
                    }
                    if let Incoming::Snapshot(qa_network::snapshots::ReceivedFrame::Quake3(frame)) = &incoming
                        && self.runtime.client_resources[id.0 as usize].as_ref().is_some_and(|resources| {
                            resources.startup.q3().is_some_and(|phase| {
                                match phase {
                                    crate::client_resources::startup::Q3Phase::Active => false,
                                    crate::client_resources::startup::Q3Phase::Primed => frame.flags & 2 != 0,
                                    _ => true,
                                }
                            })
                        })
                    {
                        return std::ops::ControlFlow::Continue(());
                    }
                    if let Incoming::Snapshot(frame) = incoming {
                        if let Some(resources) = self.runtime.client_resources[id.0 as usize].as_mut() {
                            if let qa_network::snapshots::ReceivedFrame::Quake3(frame) = frame {
                                resources.startup.q3_snapshot(frame.flags);
                            } else if let Some(binding) = self.runtime.snapshots[id.0 as usize].as_ref() { resources.startup.receive_entity(binding.protocol); }
                        }
                        self.runtime.synchronize_client_collision(id, frame);
                    }
                    let server = &self.runtime.server;
                    if let Some(binding) = self.runtime.snapshots[id.0 as usize].as_ref()
                        && let Some(seat) = binding.seat()
                        && server.clients[id.0 as usize].connection
                            == Some(qa_session::clients::Connection::Local)
                        && server.clients[id.0 as usize].output == Some(binding.output)
                    {
                        let download_protocol = binding.protocol;
                        let awaiting_player = binding.native_client().is_none();
                        if awaiting_player
                            && let Incoming::SetView { entity } = &incoming
                            && let Some(max_clients) = self.runtime.client_resources[id.0 as usize]
                                .as_ref()
                                .and_then(crate::client_resources::ClientResources::nq_assignment_max_clients)
                            && !self.runtime.snapshots[id.0 as usize].as_mut()
                                .is_some_and(|binding| binding.assign_nq_player(*entity, max_clients))
                        {
                            self.reject_client_startup(id, format_args!("invalid initial player edict {entity}"));
                            return std::ops::ControlFlow::Break(());
                        }
                        match incoming {
                            Incoming::ConnectAccepted { userinfo, .. } => {
                                signon_client = Some(id);
                                if let Some(text) = userinfo {
                                    let cache = &mut self.userinfo[seat.index()];
                                    cache.entity = Some(self.runtime.server.clients[id.0 as usize].entity);
                                    cache.revision = None;
                                    if cache.queued.set_bytes(text).is_err() {
                                        self.reject_client_startup(id, format_args!("accepted userinfo capacity"));
                                        return std::ops::ControlFlow::Break(());
                                    }
                                }
                                if let Some(resources) = self.runtime.client_resources[id.0 as usize].as_mut() {
                                    resources.startup.connected(download_protocol);
                                }
                            }
                            Incoming::MaxSpeed(value) => {
                                self.runtime.prediction[seat.index()].player.movement.tuning.max_speed = Some(qa_core::primitives::MovementMagnitude::Base(value));
                            }
                            Incoming::EntityGravity(value) => {
                                self.runtime.prediction[seat.index()].player.movement.tuning.gravity_multiplier = value;
                            }
                            Incoming::QwServerData(data) => {
                                signon_client = Some(id);
                                self.local_worlds[seat.index()] = None;
                                self.reset_received_client(id, seat);
                                let result = self.runtime.client_resources[id.0 as usize].as_mut()
                                    .ok_or("missing QW CLIENT resource owner")
                                    .and_then(|resources| resources.apply_qw_server_data(&data));
                                let assigned = self.runtime.snapshots[id.0 as usize].as_mut()
                                    .is_some_and(|binding| binding.receive_qw_server_data(&data));
                                if let Err(error) = result {
                                    self.reject_client_startup(id, format_args!("{error}"));
                                    return std::ops::ControlFlow::Break(());
                                }
                                if !assigned {
                                    self.reject_client_startup(id, format_args!("invalid QW native player assignment"));
                                    return std::ops::ControlFlow::Break(());
                                }
                                let player = &mut self.runtime.prediction[seat.index()].player;
                                player.movement.tuning = data.tuning;
                                player.movement.mode = if data.spectator { qa_core::primitives::MovementMode::Spectator } else { qa_core::primitives::MovementMode::Walk };
                            }
                            Incoming::Q2ServerData(data) => {
                                signon_client = Some(id);
                                self.local_worlds[seat.index()] = None;
                                self.reset_received_client(id, seat);
                                let result = self.runtime.client_resources[id.0 as usize].as_mut()
                                    .ok_or_else(|| "missing Q2 CLIENT resource owner".to_owned())
                                    .and_then(|resources| resources.apply_q2_server_data(id, &data));
                                let assigned = self.runtime.snapshots[id.0 as usize].as_mut()
                                    .is_some_and(|binding| binding.receive_q2_server_data(&data));
                                if let Err(error) = result {
                                    self.reject_client_startup(id, format_args!("{error}"));
                                    return std::ops::ControlFlow::Break(());
                                }
                                if !assigned {
                                    self.reject_client_startup(id, format_args!("invalid Q2 native player assignment"));
                                    return std::ops::ControlFlow::Break(());
                                }
                            }
                            Incoming::ResourceStream(kind) => {
                                signon_client = Some(id);
                                let result = self.runtime.client_resources[id.0 as usize].as_mut()
                                    .ok_or("missing Q2 CLIENT resource owner")
                                    .and_then(|resources| resources.startup.resource_stream(kind));
                                if let Err(error) = result {
                                    self.reject_client_startup(id, format_args!("{error}"));
                                    return std::ops::ControlFlow::Break(());
                                }
                            }
                            Incoming::Reconnect => {
                                if let Some(resources) = self.runtime.client_resources[id.0 as usize].as_mut() {
                                    if let Some(downloads) = resources.downloads.as_mut() { downloads.close(); }
                                }
                                self.begin_client_map_change(id);
                                if let Some(resources) = self.runtime.client_resources[id.0 as usize].as_mut() {
                                    resources.startup.restart_connection();
                                }
                                self.receive_server_commands(id, seat, b"reconnect", time, download_protocol);
                            }
                            Incoming::ResourceList(progress) => {
                                signon_client = Some(id);
                                let result = self.runtime.client_resources[id.0 as usize].as_mut()
                                    .ok_or("missing QW CLIENT resource owner")
                                    .and_then(|resources| resources.startup.resource_list(progress, resources.state.layout));
                                if let Err(error) = result {
                                    self.reject_client_startup(id, format_args!("{error}"));
                                    return std::ops::ControlFlow::Break(());
                                }
                            }
                            Incoming::StuffText(text) => {
                                signon_client = Some(id);
                                self.receive_server_commands(id, seat, text, time, download_protocol);
                            }
                            Incoming::Download(chunk) => {
                                if let Ok(qa_network::downloads::Chunk::Rejected { size, message }) = chunk {
                                    let progress = crate::client_resources::downloads::ProgressViews::bind(&self.console.cvars, seat);
                                    if let Err(error) = progress.and_then(|views| views.rejected(size, &mut self.console.cvars)) {
                                        let _ = self.runtime.server.events.console_formatted(format_args!("{error}\n"));
                                    }
                                    self.reject_client_startup(id, format_args!("{}", String::from_utf8_lossy(message)));
                                    return std::ops::ControlFlow::Break(());
                                }
                                let result = self.runtime.client_resources[id.0 as usize].as_mut()
                                    .ok_or_else(|| "download has no CLIENT resource owner".to_owned())
                                    .and_then(|resources| resources.receive_download(download_protocol, chunk, &self.runtime.vfs, &self.console.cvars));
                                if let Err(error) = result {
                                    let _ = self.runtime.server.events.console_formatted(format_args!("Download rejected: {error}\n"));
                                }
                            }
                            Incoming::Peer(update) => {
                                if let Some(resources) = self.runtime.client_resources[id.0 as usize].as_mut() {
                                    resources.state.receive_peer(update);
                                }
                            }
                            Incoming::SignonStage { stage } => {
                                if awaiting_player {
                                    self.reject_client_startup(id, format_args!("player assignment missing before native stage {stage}"));
                                    return std::ops::ControlFlow::Break(());
                                }
                                let accepted = self.runtime.client_resources[id.0 as usize].as_mut()
                                    .is_some_and(|resources| resources.startup.receive_nq_stage(stage));
                                if !accepted {
                                    self.reject_client_startup(id, format_args!("non-increasing native stage {stage}"));
                                    return std::ops::ControlFlow::Break(());
                                }
                                signon_client = Some(id);
                            }
                            Incoming::NqEntityUpdate { number, model } => {
                                let policy = self.runtime.snapshots[id.0 as usize].as_ref().and_then(|binding| binding.appearance_policy());
                                if let Some(resources) = self.runtime.client_resources[id.0 as usize].as_mut() {
                                    resources.startup.receive_entity(qa_network::commands::packet::Protocol::NetQuake15);
                                    if let Some(policy) = policy {
                                        resources.state.observe_models(u32::from(number), [model, None, None, None], policy, self.runtime.model_assets.as_ref().map(|owner| owner.catalog()));
                                    }
                                }
                                signon_client = Some(id);
                            }
                            Incoming::CommandProgress(number) => self.runtime.prediction[seat.index()].observe_command_progress(number),
                            Incoming::PlayerValue { index, value } => {
                                let protocol = self.runtime.snapshots[id.0 as usize]
                                    .as_ref()
                                    .map(|binding| binding.protocol);
                                if let Some(protocol) = protocol {
                                    self.runtime.catalog.hud.import_native_stat(
                                        protocol.rule_set(),
                                        id,
                                        index as usize,
                                        value as u32,
                                        &mut self.runtime.prediction[seat.index()].player,
                                        time,
                                    );
                                }
                            }
                            Incoming::SetAngles(angles) => {
                                self.runtime.prediction[seat.index()].player.view_angles = angles;
                                self.runtime.input.set_view_angles(seat, angles);
                            }
                            Incoming::Print(print) => {
                                let _ = self.runtime.server.events.print_level_bytes(
                                    Some(qa_core::events::OutputTarget::Local(id)),
                                    Some(id),
                                    print.kind,
                                    print.level.unwrap_or(print.kind.default_level()),
                                    print.text,
                                );
                            }
                            Incoming::Cue { cue, current } => {
                                if self.admit_received_presentation(id) {
                                    self.received_cue(id, cue, current);
                                }
                            }
                            Incoming::CdTrack(track) => {
                                // Native Q1/QW clients loop the primary track;
                                // NetQuake's second byte remains a wire field.
                                if self.admit_received_presentation(id) {
                                    crate::audio::cd_track(&mut self.runtime, id, u32::from(track.track));
                                }
                            }
                            Incoming::Particle(particle) => {
                                if !self.admit_received_presentation(id) {
                                    return std::ops::ControlFlow::Continue(());
                                }
                                if let Some(mut event) = self.runtime.snapshots[id.0 as usize]
                                    .as_ref()
                                    .and_then(|binding| binding.particle_event(particle))
                                {
                                    event.client = Some(id);
                                    event.client_lifetime =
                                        Some(self.runtime.server.clients[id.0 as usize].entity);
                                    event.time = time;
                                    let _ = self.runtime.server.events.push_to(
                                        qa_core::events::OutputTarget::Presentation,
                                        qa_core::events::FrameEvent::Effect(event),
                                    );
                                }
                            }
                            Incoming::StaticEntity(entity) => {
                                if let Some(resources) =
                                    self.runtime.client_resources[id.0 as usize].as_mut()
                                {
                                    let lookup=self.runtime.model_assets.as_ref().map(|owner|owner.catalog() as &dyn qa_compat::services::ModelLookup);
                                    let world=self.runtime.snapshots[id.0 as usize].as_ref().and_then(|binding|binding.seat()).and_then(|seat|self.local_worlds[seat.index()]);
                                    resources.state.receive_static_entity(entity,lookup,world);
                                }
                            }
                            Incoming::ConfigString(config) => {
                                let mut music_changed = false;
                                if let Some(resources) =
                                    self.runtime.client_resources[id.0 as usize].as_mut()
                                {
                                    let music = resources.state.layout.cd_track == Some(config.index as usize)
                                        && resources.state.configs.config(config.index as usize)
                                            .is_some_and(|row| row.text != config.text);
                                    music_changed = resources.apply(
                                        config.index as usize,
                                        config.text,
                                        self.runtime.model_assets.as_ref().filter(|owner| !owner.initial_registration()).map(crate::model_assets::ModelAssets::catalog),
                                        self.runtime.sound_assets.as_deref().map(crate::audio::SoundAssets::bank),
                                    ) && music;
                                    if let Some(assets) = self.runtime.sound_assets.as_deref_mut() {
                                        resources.prepare_sounds(assets, &self.runtime.vfs);
                                    }
                                }
                                if music_changed {
                                    crate::audio::config_music(&mut self.runtime, id, config.text);
                                }
                            }
                            Incoming::ServerInfo(info) => {
                                signon_client = Some(id);
                                let old_world = self.local_worlds[seat.index()];
                                let owner = self.runtime.model_assets.as_ref();
                                let mut world = None;
                                let _ = info.for_each_config(|config| {
                                    if config.index == 1
                                        && let Some(owner) = owner
                                        && let Some(binding) = qa_compat::services::ModelLookup::loaded_world(owner.catalog(), config.text, None)
                                        && binding.collision.is_some()
                                        && let Some((render, presentation)) = old_world
                                        && render == binding.render
                                        && owner.assets().world_presentation(presentation)
                                            .is_some_and(|entry| entry.world() == render)
                                    {
                                        world = Some((binding, presentation));
                                    }
                                });
                                self.local_worlds[seat.index()] = world.map(|(world, presentation)| (world.render, presentation));
                                self.reset_received_client(id, seat);
                                if let Some(resources) = self.runtime.client_resources[id.0 as usize].as_mut() {
                                    resources.apply_server_info(
                                        &info,
                                        world,
                                        self.runtime.model_assets.as_ref().filter(|owner| !owner.initial_registration()).map(crate::model_assets::ModelAssets::catalog),
                                        self.runtime.sound_assets.as_deref().map(crate::audio::SoundAssets::bank),
                                    );
                                    if let Some(assets) = self.runtime.sound_assets.as_deref_mut() {
                                        resources.prepare_sounds(assets, &self.runtime.vfs);
                                    }
                                }
                                if let Some(binding) = self.runtime.snapshots[id.0 as usize].as_mut() {
                                    binding.receive_server_info();
                                }
                            }
                            Incoming::Gamestate(gamestate) => {
                                self.reset_received_client(id, seat);
                                if let Some(resources) =
                                    self.runtime.client_resources[id.0 as usize].as_mut()
                                {
                                    resources.apply_gamestate(
                                        &gamestate,
                                        self.runtime.model_assets.as_ref().filter(|owner| !owner.initial_registration()).map(crate::model_assets::ModelAssets::catalog),
                                        self.runtime.sound_assets.as_deref().map(crate::audio::SoundAssets::bank),
                                    );
                                    if let Some(assets) = self.runtime.sound_assets.as_deref_mut() {
                                        resources.prepare_sounds(assets, &self.runtime.vfs);
                                    }
                                }
                                if let Some(binding) =
                                    self.runtime.snapshots[id.0 as usize].as_mut()
                                {
                                    binding.receive_gamestate(&gamestate);
                                }
                                if let Some(reason) = self.runtime.client_resources[id.0 as usize].as_ref().filter(|resources| resources.startup.q3().is_some()).and_then(|resources| resources.state.q3_requirement()) {
                                    self.reject_client_startup(id, format_args!("{reason}"));
                                    return std::ops::ControlFlow::Break(());
                                }
                            }
                            // Q3 cgame selects retained commands through the
                            // shared CLIENT service after packet intake.
                            Incoming::ReliableCommand { .. } => {}
                            incoming => {
                                if let Some(binding) =
                                    self.runtime.snapshots[id.0 as usize].as_mut()
                                {
                                    let prediction = &mut self.runtime.prediction[seat.index()];
                                    if binding.apply(
                                        incoming,
                                        &mut prediction.player,
                                        |number| self.runtime.client_collision[id.0 as usize].as_ref().and_then(|view| view.entity(number)),
                                        &self.runtime.catalog.hud,
                                        &self.runtime.catalog.registry,
                                        time,
                                    ) {
                                        let acknowledged = if binding.protocol == qa_network::commands::packet::Protocol::Quake3_68 {
                                            Some(qa_core::command_history::Acknowledgement::ServerTime(prediction.player.movement.command_time_ms))
                                        } else {
                                            prediction.packet_acknowledged()
                                        };
                                        prediction.remember_snapshot(acknowledged);
                                    }
                                }
                            }
                        }
                    }
                }
                qa_core::loopback::Endpoint::Server => {
                    if !self.runtime.server.clients[id.0 as usize].is_active()
                        && !matches!(&incoming, Incoming::Acknowledged { .. })
                    {
                        return std::ops::ControlFlow::Continue(());
                    }
                    match incoming {
                    Incoming::MoveBegin => self.begin_command_batch(id),
                    Incoming::MoveEnd => self.end_command_batch(id, driver),
                    Incoming::CommandString {
                        protocol,
                        text,
                        output,
                        ..
                    } => {
                        if output.is_some()
                            && self.runtime.server.clients[id.0 as usize].output == output
                        {
                            match self.client_string_command_with_driver(id, protocol.rule_set(), text, driver) {
                                Ok(std::ops::ControlFlow::Break(())) => {
                                    disconnected = Some(id);
                                    return std::ops::ControlFlow::Break(());
                                }
                                Ok(std::ops::ControlFlow::Continue(())) => {}
                                Err(error) => {
                                    let _ = self.runtime.server.events.console_formatted(
                                        format_args!("ClientCommand: {error}\n"),
                                    );
                                }
                            }
                        }
                    }
                    Incoming::Userinfo { protocol, text, output } => {
                        if output.is_some()
                            && self.runtime.server.clients[id.0 as usize].output == output
                        {
                            let result = self.receive_client_userinfo(id, protocol, text, driver);
                            if let Err(error) = result {
                                let _ = self.runtime.server.events.console_formatted(format_args!(
                                    "ClientUserinfoChanged: {error}\n"
                                ));
                            }
                        }
                    }
                    Incoming::Disconnect { output } => {
                        if output.is_some()
                            && self.runtime.server.clients[id.0 as usize].output == output
                        {
                            self.disconnect_client_with_driver(id, driver);
                            disconnected = Some(id);
                            return std::ops::ControlFlow::Break(());
                        }
                    }
                    Incoming::Command {
                        mut command,
                        native_weapon,
                        observation,
                        execution,
                        output,
                    } => {
                        if output.is_some()
                            && self.runtime.server.clients[id.0 as usize].output == output
                        {
                            if let Some(number) = native_weapon {
                                command.weapon = self.runtime.catalog.registry.native_weapon(
                                    qa_core::primitives::RuleSetId::Quake3,
                                    u16::from(number),
                                );
                            }
                            self.received_command(id, command, execution, observation, driver);
                        }
                    }
                    Incoming::Acknowledged {
                        receipt,
                        output: Some(output),
                    } => {
                        if self.runtime.server.clients[id.0 as usize].output == Some(output) {
                            let final_sequence = match self.runtime.server.clients[id.0 as usize].phase {
                                qa_session::clients::ClientPhase::Closing(closing) => closing.final_output.filter(|(owner, _)| *owner == output).map(|(_, sequence)| sequence),
                                _ => None,
                            };
                            let mut acknowledged_close = None;
                            let publication = &mut self.runtime.config_publications[id.0 as usize];
                            let duplicates = &mut self.runtime.output_duplicates;
                            duplicates.acknowledged(output, receipt);
                            self.runtime.server.events.acknowledge_with(
                                output,
                                receipt,
                                |record| {
                                    if final_sequence == Some(record.sequence) {
                                        acknowledged_close = Some(record.sequence);
                                    }
                                    match record.event {
                                    qa_core::events::FrameEvent::ConfigString {
                                        destination_index,
                                        ..
                                    } => publication
                                        .acknowledged(destination_index as usize, record.sequence),
                                    qa_core::events::FrameEvent::GameState { .. } => {
                                        publication.acknowledge_game_state(record.sequence);
                                    }
                                    _ => {}
                                    }
                                },
                            );
                            if let Some(sequence) = acknowledged_close
                                && self.runtime.server.acknowledge_close(id, output, sequence)
                            {
                                completed_close = Some(id);
                            }
                        }
                    }
                    _ => {}
                    }
                },
            }
            if self.runtime.server.clients[id.0 as usize]
                .connection
                .is_none()
            {
                disconnected = Some(id);
                std::ops::ControlFlow::Break(())
            } else {
                std::ops::ControlFlow::Continue(())
            }
        });
        if let Some(id) = disconnected {
            network.unbind(id);
        }
        self.runtime.network = network;
        self.flush_client_downloads();
        if let Some(id) = signon_client {
            self.flush_startup_reply(id);
        }
        if let Some(id) = completed_close {
            self.close_acknowledged += 1;
            self.retire_client(id, crate::modules::TraceAccess::empty());
        }
    }

    pub fn drain(&mut self, result: &mut FrameResult) {
        self.drain_with_driver(result, &mut crate::modules::Driver::cache_only())
    }
    pub fn drain_with_driver(
        &mut self,
        result: &mut FrameResult,
        driver: &mut crate::modules::Driver<'_>,
    ) {
        result.drains += 1;
        // Move the sole queue while callbacks borrow the composed host. Its
        // closed placeholder owns no payload storage and admits no intake.
        let mut queue = std::mem::take(&mut self.queue);
        loop {
            while let Some(event) = queue.pop() {
                self.runtime.input_time = event.time;
                result.events += 1;
                if let EventKind::Key {
                    down: true, repeat, ..
                } = event.kind
                {
                    self.key_downs += 1;
                    self.key_repeats += u64::from(repeat);
                }
                if !matches!(event.kind, EventKind::Time | EventKind::Packet { .. }) {
                    qa_console::logger::dev_print(
                        &self.console.cvars,
                        self.developer,
                        1,
                        format_args!(
                            "{{\"event\":\"input_diagnostic\",\"time_ns\":{},\"input\":\"{}\"}}",
                            event.time.0,
                            event_name(event.kind)
                        ),
                    );
                }
                match event.kind {
                    EventKind::Time => {
                        self.time = event.time;
                        self.runtime.input.seed(event.time);
                    }
                    EventKind::Quit => self.runtime.quit = true,
                    EventKind::ConsoleLine(text) => {
                        let seat = self.console.cvars.context().seat;
                        let mut target = ConsoleInput {
                            files: self.client_file_sources(),
                            console: &mut self.console,
                            events: &mut self.runtime.server.events,
                            time: event.time,
                        };
                        target.command(seat, event.time, None, None, text);
                    }
                    EventKind::Packet {
                        socket,
                        from,
                        bytes,
                    } => {
                        self.receive_packet(socket, from, bytes, event.time, driver);
                    }
                    _ => self.dispatch_input(event, driver),
                }
                self.console
                    .sync_editor_input(&mut self.runtime.input, event.time);
            }
            // Com_EventLoop drains local client packets, then server packets when
            // system events run out. Both use the same packet consumer as UDP.
            if self.runtime.loopback.enqueue(&mut queue, self.time) == 0 {
                break;
            }
        }
        self.queue = queue;
    }

    fn client_file_sources(&self) -> [Option<qa_core::primitives::FileSearchId>; SeatId::COUNT] {
        std::array::from_fn(|seat| {
            let (client, _, _) = self.local_seat(seat)?;
            self.runtime.client_resources[client.0 as usize]
                .as_ref()?
                .state
                .file_source()
        })
    }

    fn dispatch_input(
        &mut self,
        event: qa_core::sys_events::SysEvent<'_>,
        driver: &mut crate::modules::Driver<'_>,
    ) {
        use qa_input::{DispatchCursor, DispatchStep, InputAtom, InputCapture};
        let files = self.client_file_sources();
        let mut cursor = DispatchCursor::new(event);
        loop {
            let step = self.runtime.input.advance(
                cursor,
                &mut ConsoleInput {
                    files,
                    console: &mut self.console,
                    events: &mut self.runtime.server.events,
                    time: event.time,
                },
            );
            let DispatchStep::Atom { atom, resume } = step else {
                break;
            };
            let console_caught = match atom {
                InputAtom::Key {
                    seat,
                    source,
                    down,
                    repeat,
                    time,
                } => ConsoleInput {
                    files,
                    console: &mut self.console,
                    events: &mut self.runtime.server.events,
                    time,
                }
                .key(seat, source.control, down, repeat),
                InputAtom::Character {
                    seat, value, time, ..
                } => {
                    let caught = self.console.editor.seat() == Some(seat);
                    ConsoleInput {
                        files,
                        console: &mut self.console,
                        events: &mut self.runtime.server.events,
                        time,
                    }
                    .character(seat, value);
                    caught
                }
                InputAtom::Mouse { .. } => false,
            };
            let capture = if console_caught {
                if matches!(atom, InputAtom::Key { down: false, .. }) {
                    let _ = self.module_input_with_driver(atom, driver);
                }
                InputCapture::Caught
            } else {
                self.module_input_with_driver(atom, driver)
            };
            self.console
                .sync_editor_input(&mut self.runtime.input, event.time);
            cursor = self.runtime.input.resume(
                resume,
                capture,
                &mut ConsoleInput {
                    files,
                    console: &mut self.console,
                    events: &mut self.runtime.server.events,
                    time: event.time,
                },
            );
        }
    }

    fn client_frame(
        &mut self,
        command_times: [Option<qa_core::primitives::ThinkTime>; SeatId::COUNT],
        source: &mut impl FrameSource,
    ) -> [UserCmd; SeatId::COUNT] {
        let rules: [_; SeatId::COUNT] = std::array::from_fn(|seat| {
            self.local_clients[seat].map_or(qa_core::primitives::RuleSetId::default(), |_| {
                self.runtime.prediction[seat].player.movement_rules
            })
        });
        let policies = {
            std::array::from_fn(|seat| {
                let input_rules = self.local_clients[seat]
                    .map_or(qa_core::primitives::RuleSetId::default(), |id| {
                        self.runtime.server.clients[id.0 as usize].input_rules
                    });
                let mut policy = self.input_handles.policy(&self.console.cvars, input_rules);
                if self.local_clients[seat].is_some() {
                    policy.delta_pitch =
                        self.runtime.prediction[seat].player.movement.delta_angles.0[0].degrees();
                }
                policy
            })
        };
        let mut commands = self
            .runtime
            .input
            .build_frame_with_policy(self.time, &policies);
        for (seat, command) in commands.iter_mut().enumerate() {
            if self.local_clients[seat].is_some_and(|id| {
                self.runtime
                    .network
                    .get(id, qa_core::loopback::Endpoint::Client)
                    .is_none_or(|connection| !connection.phase.is_connected())
                    || self.runtime.client_resources[id.0 as usize]
                        .as_ref()
                        .is_some_and(|resources| resources.awaiting_startup())
                    || !self.runtime.server.clients[id.0 as usize].is_active()
                    || self.runtime.snapshots[id.0 as usize]
                        .as_ref()
                        .is_some_and(|binding| {
                            binding.presentation_closed() || binding.native_client().is_none()
                        })
            }) {
                *command = UserCmd::default();
                continue;
            }
            *command = qa_movement::prepare_command(rules[seat], *command);
            if self.local_clients[seat]
                .and_then(|id| self.runtime.snapshots[id.0 as usize].as_ref())
                .is_some_and(|binding| {
                    binding.protocol == qa_network::commands::packet::Protocol::Quake3_68
                })
            {
                command.server_time_ms =
                    command_times[seat].map_or(0, |time| time.milliseconds() as i32);
            }
        }
        for (seat, id) in self.local_clients.iter().enumerate() {
            if let Some(id) = id {
                if !self.runtime.server.clients[id.0 as usize].is_active()
                    || self
                        .runtime
                        .network
                        .get(*id, qa_core::loopback::Endpoint::Client)
                        .is_none_or(|connection| !connection.phase.is_connected())
                    || self.runtime.snapshots[id.0 as usize]
                        .as_ref()
                        .is_some_and(|binding| binding.presentation_closed())
                {
                    continue;
                }
                if let Some(commands) = self
                    .runtime
                    .network
                    .get_mut(*id, qa_core::loopback::Endpoint::Client)
                    .and_then(|connection| connection.commands.as_mut())
                {
                    commands.set_packet_dup(self.console.cvars.integer(self.packet_dup));
                }
                self.runtime
                    .send_client_command(*id, &commands[seat], self.time, source);
                if self.runtime.client_resources[id.0 as usize]
                    .as_ref()
                    .is_some_and(|resources| resources.awaiting_startup())
                    || self.runtime.snapshots[id.0 as usize]
                        .as_ref()
                        .is_some_and(|binding| binding.native_client().is_none())
                {
                    continue;
                }
                if let Some(frame) = self
                    .runtime
                    .network
                    .get(*id, qa_core::loopback::Endpoint::Client)
                    .and_then(|connection| connection.channel.current_snapshot())
                {
                    if let (Some(view), Some(binding), Some(resources)) = (
                        self.runtime.client_collision[id.0 as usize].as_mut(),
                        self.runtime.snapshots[id.0 as usize].as_ref(),
                        self.runtime.client_resources[id.0 as usize].as_ref(),
                    ) {
                        let _ = view.synchronize(
                            frame,
                            binding.entity_projection(),
                            &resources.state.models,
                            &self.runtime.geometry,
                            binding.player_entity(Some(frame)),
                            resources.state.max_clients(),
                        );
                    }
                }
                if self
                    .runtime
                    .network
                    .get(*id, qa_core::loopback::Endpoint::Client)
                    .and_then(|connection| connection.channel.current_snapshot())
                    .is_none()
                    && let Some(view) = self.runtime.client_collision[id.0 as usize].as_mut()
                {
                    view.invalidate();
                }
                if let Some(view) = self.runtime.client_collision[id.0 as usize].as_mut() {
                    let prediction = &mut self.runtime.prediction[seat];
                    let result = if let Ok(mut trace) = view.trace(
                        &self.runtime.geometry,
                        self.collision_handles.resolve(&self.console.cvars),
                    ) {
                        let base = prediction.player.movement.tuning;
                        prediction.player.movement.tuning = self.movement_handles.resolve(
                            &self.console.cvars,
                            prediction.player.movement_rules,
                            base,
                        );
                        let result = if let Some(history) = self
                            .runtime
                            .network
                            .get(*id, qa_core::loopback::Endpoint::Client)
                            .and_then(|connection| connection.commands.as_ref())
                            .and_then(|commands| commands.history())
                        {
                            prediction.replay(history, &mut trace)
                        } else {
                            prediction.advance(commands[seat], &mut trace).map(|_| 1)
                        };
                        prediction.player.movement.tuning = base;
                        result
                    } else {
                        prediction.advance_angles(commands[seat]);
                        Ok(0)
                    };
                    if let Err(error) = result {
                        let _ = self.runtime.server.events.console_formatted(format_args!(
                            "Prediction client {}: {error:?}\n",
                            id.0
                        ));
                        continue;
                    }
                    if let Some(seat_id) = SeatId::new(seat as u8)
                        && let Some((previous, pitch)) = self.runtime.input.drift_view(
                            seat_id,
                            self.time,
                            policies[seat],
                            &prediction.player,
                            commands[seat].movement[0],
                        )
                    {
                        prediction.drift_pitch(commands[seat], previous, pitch);
                    }
                }
            }
        }
        self.runtime.targets.refresh(
            &mut self.runtime.server.entities,
            &self.runtime.catalog.names,
        );
        for (seat, &id) in self.local_clients.iter().enumerate() {
            let Some(id) = id else {
                continue;
            };
            if self.local_clients[..seat].contains(&Some(id)) {
                continue;
            }
            let Some(client) = self.runtime.server.clients.get_mut(id.0 as usize) else {
                continue;
            };
            if !client.is_active()
                || client.connection != Some(qa_session::clients::Connection::Local)
            {
                continue;
            }
            let player = &self.runtime.prediction[seat].player;
            self.runtime
                .catalog
                .hud
                .project_stats(player, &mut client.hud);
            client
                .hud_publication
                .publish_changes(player, &mut self.runtime.server.events, id);
        }
        commands
    }
}

struct ConsoleInput<'a> {
    files: [Option<qa_core::primitives::FileSearchId>; SeatId::COUNT],
    console: &'a mut Console<Runtime>,
    events: &'a mut qa_core::events::EventRing,
    time: EventTime,
}
impl ConsoleInput<'_> {
    fn context(
        &self,
        seat: SeatId,
        time: EventTime,
        key_source: Option<qa_core::sys_events::KeySource>,
        producer: Option<qa_core::primitives::CommandProducer>,
    ) -> qa_console::views::Context {
        qa_console::views::Context {
            seat,
            event_time: Some(time),
            key_source,
            producer,
            file_search: self.files[seat.index()],
            ..self.console.cvars.context()
        }
    }
}
impl Target for ConsoleInput<'_> {
    fn key(&mut self, seat: SeatId, control: u16, down: bool, repeat: bool) -> bool {
        let context = self.context(seat, self.time, None, None);
        match self
            .console
            .edit_key(control, down, repeat, context, |text| {
                let _ = self.events.console_formatted(text);
            }) {
            Ok(caught) => caught,
            Err(error) => {
                let _ = self
                    .events
                    .console_formatted(format_args!("Console input: {error:?}\n"));
                true
            }
        }
    }
    fn character(&mut self, seat: SeatId, value: char) {
        if self.console.editor.character(seat, value).is_err() {
            let _ = self.events.console_bytes(b"Console input line is full\n");
        }
    }
    fn command(
        &mut self,
        seat: SeatId,
        time: EventTime,
        source: Option<qa_core::sys_events::KeySource>,
        producer: Option<qa_core::primitives::CommandProducer>,
        text: &str,
    ) -> bool {
        self.command_bytes(seat, time, source, producer, text.as_bytes())
    }
    fn command_bytes(
        &mut self,
        seat: SeatId,
        time: EventTime,
        source: Option<qa_core::sys_events::KeySource>,
        producer: Option<qa_core::primitives::CommandProducer>,
        text: &[u8],
    ) -> bool {
        let context = self.context(seat, time, source, producer);
        if let Err(error) = self.console.append_line_bytes(text, context) {
            self.events.reject_publication();
            let _ = self
                .events
                .console_formatted(format_args!("Input command: {error:?}\n"));
            false
        } else {
            true
        }
    }
    fn cancel_commands(
        &mut self,
        source: qa_core::sys_events::KeySource,
        producer: Option<qa_core::primitives::CommandProducer>,
    ) {
        self.console.discard_acquisition(source, producer);
    }
}

fn event_name(kind: EventKind<'_>) -> &'static str {
    match kind {
        EventKind::Time => "time",
        EventKind::Key { .. } => "key",
        EventKind::Text { .. } => "text",
        EventKind::Mouse { .. } => "mouse",
        EventKind::MouseButton { .. } => "mouse_button",
        EventKind::MouseWheel { .. } => "mouse_wheel",
        EventKind::ControllerAxis { .. } => "controller_axis",
        EventKind::ControllerButton { .. } => "controller_button",
        EventKind::DeviceRemoved(_) => "device_removed",
        EventKind::Focus(_) => "focus",
        EventKind::MouseCapture(_) => "mouse_capture",
        EventKind::Quit => "quit",
        EventKind::ConsoleLine(_) => "console_line",
        EventKind::Packet { .. } => "packet",
    }
}

#[cfg(test)]
#[path = "../tests/unit/qw_userinfo.rs"]
mod userinfo_tests;
