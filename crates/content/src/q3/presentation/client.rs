//! Quake III presentation: client.
//!
//! Donor provenance: `src/content/q3/presentation/client.ts`.

use qa_core::math::{Axis, Vec3};

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::presentation::events::*;
use crate::q3::presentation::frame::*;
use crate::q3::presentation::mirrors_present_client::*;
use crate::q3::presentation::player_state::*;
use crate::q3::presentation::players::*;
use crate::q3::presentation::prediction::*;
use crate::q3::presentation::resources::*;
use crate::q3::presentation::retail_snapshot::*;
use crate::q3::presentation::server_commands::*;
use crate::q3::presentation::snapshots::*;
use crate::q3::presentation::state::*;

// ---------------------------------------------------------------------------
// Client assembly (client.ts)
// ---------------------------------------------------------------------------

/// Presentation session mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresentationSessionMode {
    /// Demo playback.
    Demo,
    /// Live session.
    Live,
}

/// Supplemental scene session.
///
/// Scene seats share effects but never own input, UI, or the destination
/// view, so the primary-only command methods are absent here.
pub trait Q3SceneSession {
    /// Whether the status display is visible.
    fn status_visible(&self) -> bool {
        true
    }
    /// Presentation product.
    fn product(&self) -> Q3Product;
    /// Client number.
    fn client_number(&self) -> i32;
    /// Server message sequence.
    fn server_message_sequence(&self) -> i32;
    /// Last executed server command.
    fn last_executed_server_command(&self) -> i32;
    /// Session mode.
    fn session_mode(&self) -> PresentationSessionMode;
    /// Current game-state configstrings.
    fn get_game_state(&mut self) -> Vec<String>;
    /// Server command by sequence.
    fn get_server_command(&mut self, sequence: i32) -> PresentResult<Option<Vec<String>>>;
    /// Snapshot ping by message number.
    fn snapshot_ping(&mut self, number: i32) -> Option<i32>;
    /// Assert this seat is current.
    fn assert_current(&self);
    /// Diagnostic print.
    fn print(&mut self, text: &str);
}

/// Presentation session (`Q3PresentationSession`).
pub trait Q3PresentationSession: Q3SceneSession {
    /// Add a reliable client command.
    fn add_reliable_command(&mut self, text: &str);
    /// Append a console command.
    fn append_console_command(&mut self, text: &str);
    /// Register a cgame console command.
    fn register_cgame_command(&mut self, name: &str);
    /// Set the user command value.
    fn set_user_command_value(&mut self, weapon: i32, sensitivity: f32);
}

/// Source sound origin (`VoiceOrigin`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SourceSoundOrigin {
    /// Local voice.
    Local,
    /// Fixed world point.
    Fixed(Vec3),
    /// Entity voice.
    Entity(i32),
}

/// Source sound options (`StartSoundOptions`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SourceSoundOptions {
    /// Entity number.
    pub entity: i32,
    /// Channel.
    pub channel: i32,
    /// Voice origin.
    pub origin: SourceSoundOrigin,
    /// Channel volume units (`S_StartSound` uses 127).
    pub volume: i32,
}

/// Client sound (`Q3ClientSound`).
///
/// The donor's sound bank folds into the caller's registration hosts.
pub trait Q3ClientSound {
    /// Start a positional sound.
    fn start_sound(&mut self, origin: Option<Vec3>, entity: i32, channel: i32, pcm: Option<PcmSound>);
    /// Start a source sound.
    fn start_source_sound(&mut self, pcm: Option<PcmSound>, options: &SourceSoundOptions);
    /// Start a local sound.
    fn start_local_sound(&mut self, pcm: Option<PcmSound>, channel: i32);
    /// Add a looping sound.
    fn add_loop_sound(&mut self, entity: i32, origin: Vec3, velocity: Vec3, pcm: Option<PcmSound>, real: bool);
    /// Update a looping sound's position.
    fn update_sound_position(&mut self, entity: i32, position: Vec3);
    /// Stop a looping sound.
    fn stop_looping_sound(&mut self, entity: i32);
    /// Clear looping sounds.
    fn clear_looping_sounds(&mut self, kill_all: bool);
    /// Set the audio listener.
    fn set_listener(&mut self, client: i32, origin: Vec3, axis: Axis);
    /// Start the background track.
    fn start_background_track(&mut self, intro: &str, loop_track: &str) -> PresentResult<()>;
}

/// Client hardware (`"generic" | "ragepro"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientHardware {
    /// Generic hardware.
    Generic,
    /// Rage Pro.
    RagePro,
}

/// Client presentation options (`Q3ClientPresentationOptions`).
///
/// Each runtime arrives with its caller-built host; recipe hooks
/// (`character`, `event`, `predictItem`, `viewWeapon`, `playerWeapon`,
/// `bodyHidden`, `bodySubmission`, `bodyPose`, `weaponHud`) fold into those
/// hosts' pre/post methods. Configuration, media, HUD, view, weapons, and the
/// remaining subsystems live behind the same hosts.
pub struct Q3ClientPresentationOptions<SH, CW, PH, PS, EV, SC, CI, PL, RS, RW, FR> {
    /// Snapshot host.
    pub snapshot_host: SH,
    /// Collision world.
    pub collision: CW,
    /// Prediction host.
    pub prediction_host: PH,
    /// Player-state host.
    pub player_state_host: PS,
    /// Event host.
    pub event_host: EV,
    /// Server-command host.
    pub server_command_host: SC,
    /// Client-info host.
    pub client_info_host: CI,
    /// Player host.
    pub player_host: PL,
    /// Resource host.
    pub resource_host: RS,
    /// Resource world, if the seat owns collision geometry.
    pub resource_world: Option<RW>,
    /// Resource handle owner.
    pub resource_handles: ResourceHandleOwner,
    /// Frame host.
    pub frame_host: FR,
    /// Client hardware.
    pub hardware: ClientHardware,
}

/// Scene presentation options (`Q3ScenePresentationOptions`).
pub struct Q3ScenePresentationOptions<SH, EV, SC, CI, RS, RW, FR> {
    /// Snapshot host.
    pub snapshot_host: SH,
    /// Event host.
    pub event_host: EV,
    /// Server-command host.
    pub server_command_host: SC,
    /// Client-info host.
    pub client_info_host: CI,
    /// Resource host.
    pub resource_host: RS,
    /// Resource world, if the seat owns collision geometry.
    pub resource_world: Option<RW>,
    /// Resource handle owner.
    pub resource_handles: ResourceHandleOwner,
    /// Scene frame host.
    pub frame_host: FR,
    /// Client hardware.
    pub hardware: ClientHardware,
}

/// Client presentation (`Q3ClientPresentation`).
pub struct Q3ClientPresentation<SH, CW, PH, PS, EV, SC, CI, PL, RS, RW, FR> {
    /// Client state.
    pub state: ClientGameState,
    /// Static client state.
    pub static_state: ClientGameStaticState,
    /// Snapshot runtime.
    pub snapshots: SnapshotRuntime<SH>,
    /// Prediction runtime.
    pub prediction: PredictionRuntime<CW, PH>,
    /// Player-state runtime.
    pub player_state: PlayerStateRuntime<PS>,
    /// Event runtime.
    pub events: ClientEventRuntime<EV>,
    /// Server-command runtime.
    pub server_commands: ClientServerCommandRuntime<SC>,
    /// Client-info store.
    pub clients: ClientInfoStore<CI>,
    /// Player presenter.
    pub players: PlayerPresenter<PL>,
    /// Renderer resources.
    pub resources: Q3RendererResources<RS, RW>,
    /// Frame runtime.
    pub frames: Q3PresentationFrameRuntime<FR>,
    /// Client hardware.
    pub hardware: ClientHardware,
}

/// Scene presentation (`Q3ScenePresentation`).
pub struct Q3ScenePresentation<SH, EV, SC, CI, RS, RW, FR> {
    /// Client state.
    pub state: ClientGameState,
    /// Static client state.
    pub static_state: ClientGameStaticState,
    /// Snapshot runtime.
    pub snapshots: SnapshotRuntime<SH>,
    /// Event runtime.
    pub events: ClientEventRuntime<EV>,
    /// Server-command runtime.
    pub server_commands: ClientServerCommandRuntime<SC>,
    /// Client-info store.
    pub clients: ClientInfoStore<CI>,
    /// Renderer resources.
    pub resources: Q3RendererResources<RS, RW>,
    /// Scene frame runtime.
    pub frames: Q3PresentationFrameRuntime<FR>,
    /// Client hardware.
    pub hardware: ClientHardware,
}

/// Verify the server game and read the level start time.
pub fn check_game_match(game_state: &[String]) -> PresentResult<i32> {
    let mismatch = game_state.get(20).map(String::as_str).unwrap_or_default();
    if mismatch != "baseq3-1" {
        return Err(drop_msg(format!("Client/Server game mismatch: baseq3-1/{mismatch}")));
    }
    Ok(game_atoi(game_state.get(21).map(String::as_str).unwrap_or_default()))
}

/// Create a client presentation (`createQ3ClientPresentation`).
#[allow(clippy::type_complexity)]
pub fn create_q3_client_presentation<S, SH, CW, PH, PS, EV, SC, CI, PL, RS, RW, FR>(
    session: &mut S,
    options: Q3ClientPresentationOptions<SH, CW, PH, PS, EV, SC, CI, PL, RS, RW, FR>,
) -> PresentResult<Q3ClientPresentation<SH, CW, PH, PS, EV, SC, CI, PL, RS, RW, FR>>
where
    S: Q3PresentationSession,
    SH: SnapshotHost,
    CW: CollisionWorld,
    PH: PredictionHost,
    PS: PlayerStateHost,
    EV: ClientEventHost,
    SC: ClientServerCommandHost,
    CI: ClientInfoHost,
    PL: PlayerPresentationHost,
    RS: Q3ResourceHost,
    RW: ResourceWorld,
    FR: Q3PresentationFrameHost,
{
    session.assert_current();
    let product = session.product();
    let mut state = ClientGameState::new(product, session.client_number(), session.server_message_sequence())?;
    let mut static_state = ClientGameStaticState::new(product);
    static_state.server_command_sequence = session.last_executed_server_command();
    state.weapon_select = Weapon::Machinegun as i32;
    static_state.redflag = -1;
    static_state.blueflag = -1;
    static_state.flag_status = -1;
    let strings = session.get_game_state();
    static_state.level_start_time = check_game_match(&strings)?;
    let mut server_commands = ClientServerCommandRuntime::new(options.server_command_host);
    server_commands.parse_server_info(&mut static_state)?;
    let mut presentation = Q3ClientPresentation {
        state,
        static_state,
        snapshots: SnapshotRuntime::new(options.snapshot_host),
        prediction: PredictionRuntime::new(options.collision, options.prediction_host),
        player_state: PlayerStateRuntime::new(options.player_state_host),
        events: ClientEventRuntime::new(options.event_host),
        server_commands,
        clients: ClientInfoStore::new(options.client_info_host),
        players: PlayerPresenter::new(options.player_host, product)?,
        resources: Q3RendererResources::with_world(
            options.resource_host,
            options.resource_world,
            options.resource_handles,
        ),
        frames: Q3PresentationFrameRuntime::new(options.frame_host),
        hardware: options.hardware,
    };
    presentation.state.info_screen_text.clear();
    presentation
        .server_commands
        .set_config_values(&mut presentation.state, &mut presentation.static_state)?;
    presentation.server_commands.start_music()?;
    presentation.server_commands.shader_state_changed()?;
    presentation.server_commands.host.clear_looping_sounds(true);
    session.assert_current();
    Ok(presentation)
}

/// Create a scene presentation (`createQ3ScenePresentation`).
pub fn create_q3_scene_presentation<S, SH, EV, SC, CI, RS, RW, FR>(
    session: &mut S,
    options: Q3ScenePresentationOptions<SH, EV, SC, CI, RS, RW, FR>,
) -> PresentResult<Q3ScenePresentation<SH, EV, SC, CI, RS, RW, FR>>
where
    S: Q3SceneSession,
    SH: SnapshotHost,
    EV: ClientEventHost,
    SC: ClientServerCommandHost,
    CI: ClientInfoHost,
    RS: Q3ResourceHost,
    RW: ResourceWorld,
    FR: Q3PresentationSceneHost,
{
    session.assert_current();
    let product = session.product();
    let mut state = ClientGameState::new(product, session.client_number(), session.server_message_sequence())?;
    let mut static_state = ClientGameStaticState::new(product);
    static_state.server_command_sequence = session.last_executed_server_command();
    state.weapon_select = Weapon::Machinegun as i32;
    static_state.redflag = -1;
    static_state.blueflag = -1;
    static_state.flag_status = -1;
    let strings = session.get_game_state();
    static_state.level_start_time = check_game_match(&strings)?;
    let mut server_commands = ClientServerCommandRuntime::new(options.server_command_host);
    server_commands.parse_server_info(&mut static_state)?;
    let mut presentation = Q3ScenePresentation {
        state,
        static_state,
        snapshots: SnapshotRuntime::new(options.snapshot_host),
        events: ClientEventRuntime::new(options.event_host),
        server_commands,
        clients: ClientInfoStore::new(options.client_info_host),
        resources: Q3RendererResources::with_world(
            options.resource_host,
            options.resource_world,
            options.resource_handles,
        ),
        frames: Q3PresentationFrameRuntime::new_scene(options.frame_host),
        hardware: options.hardware,
    };
    presentation.state.info_screen_text.clear();
    presentation
        .server_commands
        .set_config_values(&mut presentation.state, &mut presentation.static_state)?;
    presentation.server_commands.host.clear_looping_sounds(true);
    session.assert_current();
    Ok(presentation)
}

impl<SH, CW, PH, PS, EV, SC, CI, PL, RS, RW, FR> Q3ClientPresentation<SH, CW, PH, PS, EV, SC, CI, PL, RS, RW, FR>
where
    SH: SnapshotHost,
    CW: CollisionWorld,
    PH: PredictionHost,
    PS: PlayerStateHost,
    EV: ClientEventHost,
    SC: ClientServerCommandHost,
    CI: ClientInfoHost,
    PL: PlayerPresentationHost,
    RS: Q3ResourceHost,
    RW: ResourceWorld,
    FR: Q3PresentationFrameHost,
{
    /// Retire the presentation.
    pub fn close(&mut self) -> PresentResult<()> {
        self.frames.close()?;
        self.server_commands.dispose(&mut self.static_state);
        self.server_commands.host.clear_looping_sounds(true);
        self.resources.host.clear_scene();
        Ok(())
    }
}

impl<SH, EV, SC, CI, RS, RW, FR> Q3ScenePresentation<SH, EV, SC, CI, RS, RW, FR>
where
    SH: SnapshotHost,
    EV: ClientEventHost,
    SC: ClientServerCommandHost,
    CI: ClientInfoHost,
    RS: Q3ResourceHost,
    RW: ResourceWorld,
    FR: Q3PresentationSceneHost,
{
    /// Retire the presentation.
    pub fn close(&mut self) -> PresentResult<()> {
        self.frames.close()?;
        self.server_commands.dispose(&mut self.static_state);
        self.server_commands.host.clear_looping_sounds(true);
        self.resources.host.clear_scene();
        Ok(())
    }
}
