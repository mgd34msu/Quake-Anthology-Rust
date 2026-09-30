//! Quake III presentation: client.
//!
//! Donor provenance: `src/content/q3/presentation/client.ts`.

use qa_core::math::{Axis, Vec3};

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::shared::definitions::*;
use crate::q3::presentation::collision_host::*;
use crate::q3::presentation::events::*;
use crate::q3::presentation::frame::*;
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
    fn product(&self) -> Product;
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
    game_atoi(game_state.get(21).map(String::as_str).unwrap_or_default())
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
    state.weapon_select = Weapon::WpMachinegun as i32;
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
    state.weapon_select = Weapon::WpMachinegun as i32;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q3::base::shared::player_state::PlayerState as CanonicalPlayerState;
    use crate::q3::base::shared::player_state::UserCommand;
    use crate::q3::base::world::{TraceContact, TraceSolidity};
    use crate::q3::presentation::client_info::ClientInfo;
    use crate::q3::presentation::frame::tests::TestFrameHost;
    use crate::q3::presentation::movement_host::{CommandTiming, MoveBounds};
    use crate::q3::presentation::players::tests::{TestClientInfoHost, TestPlayerHost};
    use crate::q3::presentation::resources::tests::{TestResourceHost, TestWorld};
    use qa_core::math::{vec3, Bounds, Vec3};
    use std::rc::Rc;

    struct TestSession {
        product: Product,
        client_number: i32,
        sequence: i32,
        last_command: i32,
        mode: PresentationSessionMode,
        game_state: Vec<String>,
        prints: Vec<String>,
    }

    impl TestSession {
        fn new(game_state: Vec<String>) -> Self {
            Self {
                product: Product::Baseq3,
                client_number: 0,
                sequence: 5,
                last_command: 4,
                mode: PresentationSessionMode::Live,
                game_state,
                prints: Vec::new(),
            }
        }
    }

    impl Q3SceneSession for TestSession {
        fn product(&self) -> Product {
            self.product
        }
        fn client_number(&self) -> i32 {
            self.client_number
        }
        fn server_message_sequence(&self) -> i32 {
            self.sequence
        }
        fn last_executed_server_command(&self) -> i32 {
            self.last_command
        }
        fn session_mode(&self) -> PresentationSessionMode {
            self.mode
        }
        fn get_game_state(&mut self) -> Vec<String> {
            self.game_state.clone()
        }
        fn get_server_command(&mut self, _sequence: i32) -> PresentResult<Option<Vec<String>>> {
            Ok(None)
        }
        fn snapshot_ping(&mut self, _number: i32) -> Option<i32> {
            Some(50)
        }
        fn assert_current(&self) {}
        fn print(&mut self, text: &str) {
            self.prints.push(text.to_string());
        }
    }

    impl Q3PresentationSession for TestSession {
        fn add_reliable_command(&mut self, _text: &str) {}
        fn append_console_command(&mut self, _text: &str) {}
        fn register_cgame_command(&mut self, _name: &str) {}
        fn set_user_command_value(&mut self, _weapon: i32, _sensitivity: f32) {}
    }
    struct TestServerHost {
        config: Vec<String>,
        cvars_set: Vec<(String, String)>,
        background: Option<(String, String)>,
        remaps: Vec<(String, String, String)>,
        looping: Vec<bool>,
        refreshed: bool,
    }

    impl TestServerHost {
        fn new() -> Self {
            let mut config = vec![String::new(); 32];
            config[2] = "intro loop".to_string();
            config[5] = "0".to_string();
            config[6] = "3".to_string();
            config[7] = "4".to_string();
            config[21] = "222".to_string();
            Self {
                config,
                cvars_set: Vec::new(),
                background: None,
                remaps: Vec::new(),
                looping: Vec::new(),
                refreshed: false,
            }
        }
    }

    impl ClientServerCommandHost for TestServerHost {
        fn client_info<'a>(
            &self,
            static_state: &'a ClientGameStaticState,
            index: usize,
        ) -> PresentResult<&'a ClientInfo> {
            static_state
                .client_info
                .get(index)
                .ok_or_else(|| range_msg(format!("Source array index {index} outside 64")))
        }
        fn new_client_info(
            &mut self,
            _state: &mut ClientGameState,
            static_state: &mut ClientGameStaticState,
            index: usize,
            configstring: &str,
        ) -> PresentResult<()> {
            let slot = static_state
                .client_info
                .get_mut(index)
                .ok_or_else(|| range_msg(format!("Source array index {index} outside 64")))?;
            slot.info_valid = true;
            slot.name = info_value_for_key(configstring, "n", 8192).unwrap_or_default();
            Ok(())
        }
        fn load_deferred_players(
            &mut self,
            _state: &mut ClientGameState,
            _static_state: &mut ClientGameStaticState,
        ) -> PresentResult<()> {
            Ok(())
        }
        fn reset_clients(&mut self, static_state: &mut ClientGameStaticState) {
            for slot in static_state.client_info.iter_mut() {
                *slot = ClientInfo::default();
            }
        }
        fn register_model(&mut self, path: &str) -> PresentResult<SceneModel> {
            Ok(SceneModel::Loaded {
                id: path.len() as u32 + 1,
            })
        }
        fn assets_has(&self, _path: &str) -> bool {
            false
        }
        fn assets_read(&mut self, path: &str) -> PresentResult<Vec<u8>> {
            Err(state_msg(format!("no test assets: {path}")))
        }
        fn assets_read_sync(&self, path: &str) -> PresentResult<Vec<u8>> {
            Err(state_msg(format!("no test assets: {path}")))
        }
        fn get_server_command(&mut self, _sequence: i32) -> PresentResult<Option<Vec<String>>> {
            Ok(None)
        }
        fn refresh_game_state(&mut self) {
            self.refreshed = true;
        }
        fn config_string(&self, index: i32) -> PresentResult<String> {
            if index < 0 {
                return Err(range_msg(format!("CG_ConfigString: bad index: {index}")));
            }
            self.config
                .get(index as usize)
                .cloned()
                .ok_or_else(|| range_msg(format!("CG_ConfigString: bad index: {index}")))
        }
        fn read_vm_cvar(&self, name: ClientServerCommandCvar) -> CvarSnapshot {
            CvarSnapshot {
                name: format!("{name:?}"),
                value: "0".to_string(),
                numeric_value: 0.0,
                integer_value: 0,
            }
        }
        fn set_cvar(&mut self, name: &str, value: &str) {
            self.cvars_set.push((name.to_string(), value.to_string()));
        }
        fn print(&mut self, _text: &str) {}
        fn center_print(&mut self, _text: &str, _y: i32, _char_width: i32) {}
        fn send_console_command(&mut self, _text: &str) {}
        fn sound(&self, _name: ClientServerCommandSound) -> Option<PcmSound> {
            None
        }
        fn register_sound(&mut self, _path: &str, _compressed: bool) -> PresentResult<Option<PcmSound>> {
            Ok(None)
        }
        fn start_local_sound(&mut self, _sound: Option<PcmSound>, _channel: i32) {}
        fn start_background_track(&mut self, intro: &str, loop_track: &str) -> PresentResult<()> {
            self.background = Some((intro.to_string(), loop_track.to_string()));
            Ok(())
        }
        fn remap_shader(&mut self, original: &str, replacement: &str, time_offset: &str) -> PresentResult<()> {
            self.remaps
                .push((original.to_string(), replacement.to_string(), time_offset.to_string()));
            Ok(())
        }
        fn clear_local_entities(&mut self) {}
        fn clear_marks(&mut self) {}
        fn clear_particles(&mut self) -> PresentResult<()> {
            Ok(())
        }
        fn clear_looping_sounds(&mut self, kill_all: bool) {
            self.looping.push(kill_all);
        }
        fn set_score_selection(&mut self) {}
        fn show_response_head(&mut self) -> PresentResult<()> {
            Ok(())
        }
        fn memory_remaining(&self) -> i64 {
            i64::MAX
        }
        fn random_float(&mut self) -> f32 {
            0.5
        }
    }

    // ---------- filler doubles for assembly ----------
    struct TestSnapHost;

    impl SnapshotHost for TestSnapHost {
        fn source_current(&mut self) -> SnapshotCurrent {
            SnapshotCurrent {
                number: 0,
                server_time: 0,
            }
        }
        fn source_read(&mut self, _number: i32) -> PresentResult<Option<Snapshot>> {
            Ok(None)
        }
        fn demo_playback(&self) -> bool {
            false
        }
        fn no_predict(&self) -> bool {
            false
        }
        fn synchronous_clients(&self) -> bool {
            false
        }
        fn execute_server_commands(
            &mut self,
            _state: &mut ClientGameState,
            _static_state: &mut ClientGameStaticState,
            _sequence: i32,
        ) -> PresentResult<()> {
            Ok(())
        }
        fn respawn(&mut self, _state: &mut ClientGameState) -> PresentResult<()> {
            Ok(())
        }
        fn reset_player_entity(
            &mut self,
            _state: &mut ClientGameState,
            _static_state: &mut ClientGameStaticState,
            _entity_number: usize,
        ) {
        }
        fn check_events(
            &mut self,
            _state: &mut ClientGameState,
            _static_state: &mut ClientGameStaticState,
            _entity_number: usize,
        ) -> PresentResult<()> {
            Ok(())
        }
        fn transition_player_state(
            &mut self,
            _state: &mut ClientGameState,
            _static_state: &mut ClientGameStaticState,
            _current: &CanonicalPlayerState,
            _previous: &mut CanonicalPlayerState,
        ) -> PresentResult<()> {
            Ok(())
        }
        fn lagometer_snapshot(&mut self, _snapshot: Option<&Snapshot>) {}
        fn warn(&mut self, _message: &str) {}
    }
    struct TestCollision;

    impl CollisionWorld for TestCollision {
        fn trace(&mut self, query: &TraceQuery) -> TraceResult {
            TraceResult {
                fraction: 1.0,
                end: query.end,
                solidity: TraceSolidity::Clear,
                contact: TraceContact::None,
                contents: 0,
                surface_flags: 0,
            }
        }
        fn point_contents(&mut self, _point: Vec3) -> i32 {
            0
        }
        fn transformed_trace(
            &mut self,
            query: &TraceQuery,
            _model_index: i32,
            _origin: Vec3,
            _angles: Vec3,
        ) -> TraceResult {
            self.trace(query)
        }
        fn transformed_point_contents(&mut self, _point: Vec3, _model_index: i32, _origin: Vec3, _angles: Vec3) -> i32 {
            0
        }
        fn box_trace(&mut self, _mins: Vec3, _maxs: Vec3, query: &TraceQuery, _origin: Vec3) -> TraceResult {
            self.trace(query)
        }
    }
    struct TestPredHost;

    impl PredictionHost for TestPredHost {
        fn command_timing(&self) -> CommandTiming {
            CommandTiming::Q3
        }
        fn move_player(
            &mut self,
            _ps: &mut CanonicalPlayerState,
            _command: &UserCommand,
            _options: &mut PmoveOptions,
        ) -> MoveBounds {
            MoveBounds {
                bounds: Bounds {
                    min: vec3(0.0, 0.0, 0.0),
                    max: vec3(0.0, 0.0, 0.0),
                },
            }
        }
        fn update_view_angles(&mut self, _ps: &mut CanonicalPlayerState, _command: &UserCommand) {}
        fn current_command_number(&mut self) -> i32 {
            0
        }
        fn read_command(&mut self, _number: i32) -> PresentResult<Option<UserCommand>> {
            Ok(None)
        }
        fn settings(&self) -> PredictionSettings {
            PredictionSettings {
                game_type: crate::q3::base::shared::definitions::GameType::GtFfa,
                dm_flags: 0,
                demo_playback: false,
                no_predict: false,
                synchronous_clients: false,
                predict_items: false,
                pmove_fixed: false,
                pmove_msec: 8,
                error_decay_integer: 0,
                error_decay_value: 0.0,
                show_miss: 0,
            }
        }
        fn set_pmove_msec(&mut self, _value: i32) {}
        fn transition_player_state(
            &mut self,
            _state: &mut ClientGameState,
            _static_state: &mut ClientGameStaticState,
            _current: &CanonicalPlayerState,
            _previous: &mut CanonicalPlayerState,
        ) -> PresentResult<()> {
            Ok(())
        }
        fn event_debug(&self) -> Option<Rc<dyn crate::q3::base::shared::player_state::PredictableEventDebug>> {
            None
        }
        fn pre_predict_item(&mut self, _state: &mut ClientGameState, _entity_number: usize) -> bool {
            false
        }
        fn post_predict_item(&mut self, _state: &mut ClientGameState, _entity_number: usize) {}
        fn warn(&mut self, _message: &str) {}
    }
    struct TestPsHost {
        sounds: PlayerStateSounds,
        medals: RewardMedals,
    }

    impl TestPsHost {
        fn new() -> Self {
            Self {
                sounds: PlayerStateSounds {
                    no_ammo_sound: None,
                    hit_sound: None,
                    hit_team_sound: None,
                    capture_award_sound: None,
                    impressive_sound: None,
                    excellent_sound: None,
                    humiliation_sound: None,
                    defend_sound: None,
                    assist_sound: None,
                    denied_sound: None,
                    holy_shit_sound: None,
                    you_have_flag_sound: None,
                    taken_lead_sound: None,
                    tied_lead_sound: None,
                    lost_lead_sound: None,
                    sudden_death_sound: None,
                    one_minute_sound: None,
                    five_minute_sound: None,
                    one_frag_sound: None,
                    two_frag_sound: None,
                    three_frag_sound: None,
                },
                medals: RewardMedals {
                    medal_capture: None,
                    medal_impressive: None,
                    medal_excellent: None,
                    medal_gauntlet: None,
                    medal_defend: None,
                    medal_assist: None,
                },
            }
        }
    }

    impl PlayerStateHost for TestPsHost {
        fn product(&self) -> Product {
            Product::Baseq3
        }
        fn show_miss(&self) -> bool {
            false
        }
        fn weapon_hud(&mut self) -> Option<(Option<WeaponHudStatus>, ArsenalAmmoWarning)> {
            None
        }
        fn sounds(&self) -> &PlayerStateSounds {
            &self.sounds
        }
        fn medals(&self) -> &RewardMedals {
            &self.medals
        }
        fn mission_sounds(&self) -> Option<&MissionPlayerStateSounds> {
            None
        }
        fn entity_event(
            &mut self,
            _state: &mut ClientGameState,
            _static_state: &mut ClientGameStaticState,
            _entity_ref: EventEntityRef,
            _position: Vec3,
        ) -> PresentResult<()> {
            Ok(())
        }
        fn pain_event(
            &mut self,
            _state: &mut ClientGameState,
            _static_state: &mut ClientGameStaticState,
            _entity_ref: EventEntityRef,
            _health: i32,
        ) -> PresentResult<()> {
            Ok(())
        }
        fn start_local_sound(&mut self, _sound: Option<PcmSound>, _channel: i32) {}
        fn add_buffered_sound(&mut self, _sound: Option<PcmSound>) {}
        fn print(&mut self, _message: &str) {}
    }
    struct TestEventHost {
        media: ClientEventMedia,
    }

    impl TestEventHost {
        fn new() -> Self {
            Self {
                media: ClientEventMedia {
                    sounds: ClientEventSounds {
                        use_nothing_sound: None,
                        medkit_sound: None,
                        land_sound: None,
                        jump_pad_sound: None,
                        watr_in_sound: None,
                        watr_out_sound: None,
                        watr_un_sound: None,
                        n_health_sound: None,
                        select_sound: None,
                        tele_in_sound: None,
                        tele_out_sound: None,
                        respawn_sound: None,
                        hgrenb1a_sound: None,
                        hgrenb2a_sound: None,
                        capture_your_team_sound: None,
                        capture_opponent_sound: None,
                        return_your_team_sound: None,
                        return_opponent_sound: None,
                        blue_flag_returned_sound: None,
                        red_flag_returned_sound: None,
                        enemy_took_your_flag_sound: None,
                        your_team_took_enemy_flag_sound: None,
                        your_base_is_under_attack_sound: None,
                        red_scored_sound: None,
                        blue_scored_sound: None,
                        red_leads_sound: None,
                        blue_leads_sound: None,
                        teams_tied_sound: None,
                        quad_sound: None,
                        protect_sound: None,
                        regen_sound: None,
                        gib_sound: None,
                    },
                    footsteps: FootstepBank {
                        normal: [None; 4],
                        boot: [None; 4],
                        flesh: [None; 4],
                        mech: [None; 4],
                        energy: [None; 4],
                        metal: [None; 4],
                        splash: [None; 4],
                    },
                    game_sounds: Vec::new(),
                    smoke_puff_shader: None,
                },
            }
        }
    }

    impl ClientEventHost for TestEventHost {
        fn product(&self) -> Product {
            Product::Baseq3
        }
        fn media(&self) -> &ClientEventMedia {
            &self.media
        }
        fn options(&self) -> ClientEventOptions {
            ClientEventOptions {
                game_type: GameType::GtFfa,
                debug_events: false,
                footsteps: false,
                autoswitch: false,
                demo_playback: false,
                no_predict: false,
                synchronous_clients: false,
                single_player_active: false,
                camera_orbit: false,
            }
        }
        fn mission_sounds(&self) -> Option<&MissionEventSounds> {
            None
        }
        fn rand_int(&mut self) -> i32 {
            0
        }
        fn pre_present_event(
            &mut self,
            _state: &mut ClientGameState,
            _entity_ref: EventEntityRef,
            _position: Vec3,
        ) -> bool {
            false
        }
        fn post_present_event(&mut self, _state: &mut ClientGameState, _entity_ref: EventEntityRef, _position: Vec3) {}
        fn set_entity_sound_position(&mut self, _state: &ClientGameState, _entity_number: usize) {}
        fn beam(&mut self, _state: &mut ClientGameState, _entity_ref: EventEntityRef) {}
        fn out_of_ammo_change(&mut self, _state: &mut ClientGameState) {}
        fn fire_weapon(&mut self, _state: &mut ClientGameState, _entity_ref: EventEntityRef) {}
        fn missile_hit_player(
            &mut self,
            _state: &mut ClientGameState,
            _weapon: i32,
            _position: Vec3,
            _direction: Vec3,
            _other_entity_num: i32,
        ) {
        }
        fn missile_hit_wall(
            &mut self,
            _state: &mut ClientGameState,
            _weapon: i32,
            _client_num: i32,
            _position: Vec3,
            _direction: Vec3,
            _impact: ImpactSound,
        ) {
        }
        fn rail_trail(&mut self, _state: &mut ClientGameState, _client_num: i32, _origin2: Vec3, _base: Vec3) {}
        fn bullet(&mut self, _state: &mut ClientGameState, _base: Vec3, _other_entity_num: i32, _target: BulletTarget) {
        }
        fn shotgun_fire(&mut self, _state: &mut ClientGameState, _entity_ref: EventEntityRef) {}
        fn smoke_puff(
            &mut self,
            _state: &mut ClientGameState,
            _desc: &SmokePuffDesc,
            _le_type_override: Option<&str>,
        ) -> SpawnedPuff {
            SpawnedPuff { le_type: None }
        }
        fn spawn_effect(&mut self, _state: &mut ClientGameState, _position: Vec3) {}
        fn gib_player(&mut self, _state: &mut ClientGameState, _position: Vec3) {}
        fn score_plum(&mut self, _state: &mut ClientGameState, _other_entity_num: i32, _position: Vec3, _time: i32) {}
        fn kamikaze_effect(&mut self, _state: &mut ClientGameState, _position: Vec3) {}
        fn obelisk_explode(&mut self, _state: &mut ClientGameState, _position: Vec3) {}
        fn obelisk_pain(&mut self, _state: &mut ClientGameState, _position: Vec3) {}
        fn invulnerability_impact(&mut self, _state: &mut ClientGameState, _position: Vec3, _angles: Vec3) {}
        fn invulnerability_juiced(&mut self, _state: &mut ClientGameState, _position: Vec3) {}
        fn lightning_bolt_beam(&mut self, _state: &mut ClientGameState, _origin2: Vec3, _base: Vec3) {}
        fn client_info_brief(
            &self,
            static_state: &ClientGameStaticState,
            number: i32,
        ) -> PresentResult<EventClientInfo> {
            if number < 0 {
                return Err(range_msg(format!("Player index {number} outside clients")));
            }
            let slot = static_state
                .client_info
                .get(number as usize)
                .ok_or_else(|| range_msg(format!("Player index {number} outside clients")))?;
            Ok(EventClientInfo {
                gender: slot.gender,
                footsteps: slot.footsteps,
                team: slot.team,
            })
        }
        fn set_medkit_usage_time(
            &mut self,
            static_state: &mut ClientGameStaticState,
            client_num: i32,
            time: i32,
        ) -> PresentResult<()> {
            if client_num < 0 {
                return Err(range_msg(format!("Player index {client_num} outside clients")));
            }
            let slot = static_state
                .client_info
                .get_mut(client_num as usize)
                .ok_or_else(|| range_msg(format!("Player index {client_num} outside clients")))?;
            slot.medkit_usage_time = time;
            Ok(())
        }
        fn player_name(&self, _number: i32) -> Option<String> {
            None
        }
        fn sound_config_string(&self, _index: i32) -> String {
            String::new()
        }
        fn custom_sound(&mut self, _client_num: i32, _name: &str) -> Option<PcmSound> {
            None
        }
        fn register_sound(&mut self, _path: Option<&str>, _compressed: bool) -> Option<PcmSound> {
            None
        }
        fn start_sound(&mut self, _origin: Option<Vec3>, _entity_num: i32, _channel: i32, _sound: Option<PcmSound>) {}
        fn start_local_sound(&mut self, _sound: Option<PcmSound>, _channel: i32) {}
        fn stop_looping_sound(&mut self, _entity_num: i32) {}
        fn add_buffered_sound(&mut self, _sound: Option<PcmSound>) {}
        fn print(&mut self, _message: &str) {}
        fn center_print(&mut self, _message: &str, _y: i32, _char_width: i32) {}
        fn voice_chat_local(
            &mut self,
            _state: &mut ClientGameState,
            _static_state: &mut ClientGameStaticState,
            _mode: i32,
            _voice_only: bool,
            _client_num: i32,
            _color: i32,
            _command: &str,
        ) -> PresentResult<()> {
            Ok(())
        }
    }

    // ---------- resources tests ----------
    fn session_game_state() -> Vec<String> {
        let mut strings = vec![String::new(); 22];
        strings[20] = "baseq3-1".to_string();
        strings[21] = "111".to_string();
        strings
    }
    #[test]
    fn game_match_reports_level_time() {
        assert_eq!(check_game_match(&session_game_state()).unwrap(), 111);
        let mut bad = session_game_state();
        bad[20] = "mission-1".to_string();
        assert!(matches!(check_game_match(&bad), Err(PresentClientError::Drop(_))));
        assert!(check_game_match(&[]).is_err());
    }

    #[test]
    fn create_client_presentation_initializes_and_closes() {
        let mut session = TestSession::new(session_game_state());
        let options = Q3ClientPresentationOptions {
            snapshot_host: TestSnapHost,
            collision: TestCollision,
            prediction_host: TestPredHost,
            player_state_host: TestPsHost::new(),
            event_host: TestEventHost::new(),
            server_command_host: TestServerHost::new(),
            client_info_host: TestClientInfoHost::new(),
            player_host: TestPlayerHost::new(),
            resource_host: TestResourceHost::new(),
            resource_world: None::<TestWorld>,
            resource_handles: ResourceHandleOwner::Renderer,
            frame_host: TestFrameHost::new(),
            hardware: ClientHardware::Generic,
        };
        let mut presentation = create_q3_client_presentation(&mut session, options).unwrap();
        assert_eq!(presentation.state.weapon_select, Weapon::WpMachinegun as i32);
        assert_eq!(presentation.static_state.redflag, -1);
        assert_eq!(presentation.static_state.blueflag, -1);
        assert_eq!(presentation.static_state.flag_status, -1);
        assert_eq!(presentation.static_state.server_command_sequence, 4);
        assert_eq!(presentation.static_state.level_start_time, 222);
        assert_eq!(presentation.static_state.game_type, GameType::GtFfa);
        assert!(presentation.state.info_screen_text.is_empty());
        assert_eq!(
            presentation.server_commands.host.background,
            Some(("intro".to_string(), "loop".to_string()))
        );
        assert_eq!(presentation.server_commands.host.looping, vec![true]);
        presentation.close().unwrap();
        assert!(!presentation.static_state.client_info[0].info_valid);
        assert_eq!(presentation.resources.host.cleared, 1);
    }

    #[test]
    fn create_client_presentation_rejects_mismatch() {
        let mut strings = session_game_state();
        strings[20] = "other-1".to_string();
        let mut session = TestSession::new(strings);
        let options = Q3ClientPresentationOptions {
            snapshot_host: TestSnapHost,
            collision: TestCollision,
            prediction_host: TestPredHost,
            player_state_host: TestPsHost::new(),
            event_host: TestEventHost::new(),
            server_command_host: TestServerHost::new(),
            client_info_host: TestClientInfoHost::new(),
            player_host: TestPlayerHost::new(),
            resource_host: TestResourceHost::new(),
            resource_world: None::<TestWorld>,
            resource_handles: ResourceHandleOwner::Renderer,
            frame_host: TestFrameHost::new(),
            hardware: ClientHardware::Generic,
        };
        assert!(matches!(
            create_q3_client_presentation(&mut session, options),
            Err(PresentClientError::Drop(_))
        ));
    }

    #[test]
    fn create_scene_presentation_skips_music_and_closes() {
        let mut session = TestSession::new(session_game_state());
        let options = Q3ScenePresentationOptions {
            snapshot_host: TestSnapHost,
            event_host: TestEventHost::new(),
            server_command_host: TestServerHost::new(),
            client_info_host: TestClientInfoHost::new(),
            resource_host: TestResourceHost::new(),
            resource_world: None::<TestWorld>,
            resource_handles: ResourceHandleOwner::Renderer,
            frame_host: TestFrameHost::new(),
            hardware: ClientHardware::RagePro,
        };
        let mut presentation = create_q3_scene_presentation(&mut session, options).unwrap();
        assert_eq!(presentation.state.weapon_select, Weapon::WpMachinegun as i32);
        assert_eq!(presentation.static_state.level_start_time, 222);
        assert_eq!(presentation.server_commands.host.background, None);
        assert_eq!(presentation.hardware, ClientHardware::RagePro);
        presentation.close().unwrap();
    }

    #[test]
    fn scene_session_defaults_status_visible() {
        let session = TestSession::new(session_game_state());
        assert!(session.status_visible());
    }
}
