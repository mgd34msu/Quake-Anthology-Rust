//! Seat-owned Quake III client game (cgame) presentation.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/q3-client.ts`
//! (`ApplicationQ3ClientSource`, `ApplicationQ3ClientOptions`, `QvmVideoReopenOptions`,
//! `ApplicationQ3LocalRound`, `ApplicationQ3Client`). The client owns cgame state:
//! kind dispatch, cvar wiring, input mapping, round management, system-info parsing,
//! submission collection, backend dispatch, and the render-frame orchestration.
//!
//! The `q3_client` module name was taken by the `q3-client/` directory port, so this
//! donor lives in `q3_client_app`.
//!
//! Documented folds:
//! - Async work resolves synchronously: `create`, `prepare`, `command`,
//!   `handlesCommand`, key/mouse/event/input handling, and `shutdown` are sync;
//!   server-command reads settle immediately.
//! - Backend construction is host-owned. The donor builds the TypeScript presentation
//!   and QVM client inline, but those builders live in missing siblings and the
//!   ported presentation API was redesigned without a hook surface, so the host
//!   supplies `create_native`/`create_qvm` factories. The port keeps every piece of
//!   donor logic the factories need as callbacks: the session surface, movement
//!   proxy, render gates, body capture, and the `q3:` provider wrapper contract is
//!   documented on [`Q3NativeParams`] for the wiring lane.
//! - Services, media, audio, and the render pipeline are host seams
//!   ([`Q3ClientServices`], [`Q3ClientMedia`], [`Q3ClientAudio`],
//!   [`Q3RenderPipeline`]). Frame-time sync and the cvar mirror bind the
//!   canonical `frame-time`/`SharedCvarMirror` items directly.
//!   Frame inputs (light merge, area-mask expansion, clear colors, portal loop,
//!   effect merge) are computed in the port; the pipeline adapts them to the
//!   redesigned render API.
//! - `additionalEffects` takes only the camera. The donor `SourceSceneOrder`
//!   argument manages entity reservation for effect operations; order management
//!   is pipeline-internal now and effect operations convert at the seam.
//! - `options.cvars`/`timeCvars` are shared registries (`Rc<RefCell<..>>`) so the
//!   session, mirror, and backends observe one object like the donor. `adoptCvars`
//!   swaps contents in place, matching the donor dynamic `cvars` getter.
//! - The local snapshot source is the real [`ApplicationQ3Source`]; entity
//!   selection runs the ported donor selector
//!   (`super::q3_client::visibility::select_application_q3_snapshot`) over
//!   `options.queries`, with app-to-visibility type conversion at the
//!   snapshot read. Remote sources are host trait objects. Local sources
//!   expose no system info, pings, or sequences (the donor leaves them
//!   undefined too).
//! - There is no donor `q3-client/session.ts` file; the donor session surface
//!   (`Q3PresentationSession`) comes from `content/q3/presentation/client.ts`
//!   (ported as a trait) and is mirrored locally as [`Q3ClientSession`].
//! - `adoptCvars` swaps registry contents in place so the session, mirror, and
//!   backends keep observing one object. The identity check folds to the
//!   dialect: the ported registry carries no session token.
//! - The donor dynamic `cheatsAllowed` callback folds into the registry, which
//!   reads `sv_cheats` from the same owner the system-info refresh fills.
//! - `MaterialTextDraw` borrows its compiled shader, so submissions carry the
//!   owned [`Q3TextDraw`] and rebuild the borrow at frame time.
//! - Local snapshots convert from the network shape to the presentation shape
//!   at the session read; the field maps are 1:1.
//! - Video reopen rebuilds owned options from a retained clone (settings and
//!   server settings dropped, current cvars adopted), matching the donor
//!   spread; the reopen closure is single-shot.
//! - `ActorId` hides its session token, so the seat session and the
//!   same-session probe arrive explicitly (`Q3ClientLocal.session`,
//!   `Q3LocalOptions.same_session`); sessionless cvar registries adopt the
//!   seat session when the time-cvar mirror binds.
//!
//! Missing siblings (host seams, referenced but NOT ported here): `assets.ts`,
//! `content/q3/presentation/client.ts` (native backend), and the QVM guest
//! inputs (`Q3BrowserView`, guest cvars/input/client-state, connection,
//! scalars) which the host factory captures directly.
//!
//! Canonical siblings (ported; not host seams): `keys.ts`
//! (`crate::bootstrap::keys::ApplicationKeys` over
//! `qa_core::q3_cd_key::Q3CdKeyState`), `input.ts`
//! (`crate::bootstrap::input::ApplicationInput`; the mouse-button mapping stays
//! inlined), `audio.ts` (`crate::bootstrap::audio::application`),
//! `q3-client/services.ts`
//! (`crate::bootstrap::q3_client::services::ApplicationQ3Services`),
//! `q3-client/cinematics.ts`
//! (`crate::bootstrap::q3_client::cinematics::ApplicationQ3Cinematics`), and
//! `server-administration.ts`
//! (`crate::bootstrap::server_administration::SourceServerAdministration`,
//! whose operator state the donor client never consumes).
//!
//! Bound canonically here: `component-bodies.ts`
//! (`prepare` bodies; primary-body capture still mirrors locally because the
//! client entity handle allocates host-side), `effects.ts` (effect frames),
//! `frame-time.ts` plus `world/collision/q3/settings.ts` (frame-time sync),
//! `q3-client/visibility.ts` (entity selection), and `core/cvars/mirror.ts`
//! (time-cvar mirror).

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use qa_client::input::{KeyCode, KEY_CHAR_FLAG};
use qa_client::materials::compile::CompiledMaterial;
use qa_client::materials::geometry::MaterialGeometry;
use qa_client::materials::lighting::SurfaceDynamicLight;
use qa_client::materials::q3_lighting::{light_for_point, LightingScales};
use qa_client::render::material2d::MaterialTextDraw;
use qa_client::render::q3_hardware::{q3_hardware, Q3Hardware};
use qa_client::render::scene::models::light_sampler::{ModelLightSampler, ModelLightViewInput};
use qa_client::render::scene::portal::{portal_camera, portal_surface_offscreen, PortalEntity};
use qa_client::render::scene::submissions::SceneOperation;
use qa_client::render::types::{Rect as RenderRect, RenderCommand, RenderFrame};
use qa_client::text::draw2d::TextureRect;
use qa_client::ui::types::{SeatInputEvent, SeatInputEventKind};
use qa_client::view::{perspective_projection, CameraClip, Rect, SceneCamera};
use qa_content::contract::ContentId;
use qa_content::q3::base::shared::definitions::{MoveType, Product, Team};
use qa_content::q3::base::shared::entity_state::EntityState;
use qa_content::q3::base::shared::player_state::{PlayerState, PlayerStateSlots, UserCommand as PredictionCommand};
use qa_content::q3::base::shared::trajectory::{Trajectory, TrajectoryType};
use qa_content::q3::presentation::config::cvar_table;
use qa_content::q3::presentation::movement_host::PresentationMovementHost;
use qa_content::q3::presentation::player_state::WeaponHudReader;
use qa_content::q3::presentation::prediction::CommandSource;
use qa_content::q3::presentation::ref_entity::{copy_ref_entity, RefEntity, RefModelEntity, SceneModel};
use qa_content::q3::presentation::refdef::{Refdef, RenderText, RDF_NOWORLDMODEL};
use qa_content::q3::presentation::retail_snapshot::Snapshot as PresentSnapshot;
use qa_content::q3::presentation::scene::{prepare_q3_model, ModelSourceOptions, PresentSceneEntity};
use qa_content::q3::presentation::snapshots::{SnapshotCurrent, SnapshotSource};
use qa_content::q3::presentation::state::PresentResult as PresentClientResult;
use qa_core::cmd::Dialect;
use qa_core::cmd_buffer::{CommandBuffer, CommandContext, CommandOrigin};
use qa_core::cvar::{CvarRegistry, CvarSnapshot, SharedCvarMirror};
use qa_core::identity::{ActorId, ClientId, SeatId, SessionId};
use qa_core::math::{angles_to_axis, vec3, Axis, Bounds, Plane, Vec2, Vec3, Vec4};
use qa_net::common::commands::{ActorCommand, UserCommand as NetUserCommand};
use qa_net::q3_net::{
    Q3EntityState, Q3PlayerSlots, Q3PlayerState as NetPlayerState, Q3Trajectory, Snapshot as NetSnapshot,
};
use qa_world::collision::q3::settings::COLLISION_MAP_CVAR_DEFINITIONS;
use thiserror::Error;

use super::component_bodies::ComponentBody;
use super::effects::application::ApplicationEffectFrame;
use super::frame_time::{frame_time_cvar_names, refresh_frame_time_cvars, FrameTimeError};
use super::q3_client::qvm::{QvmBodyPart, QvmHeldWeapon, QvmPresentationArtifacts};
use super::q3_client::qvm_display::QvmDisplayRenderer;
use super::q3_client::source::{
    ApplicationQ3Source, ApplicationQ3SourceOptions, Q3SourceError, Q3SourcePresentationEvent,
};
use super::q3_client::view::q3_weapon_camera;
use super::q3_client::visibility::{
    select_application_q3_snapshot, ApplicationQ3SceneQueries, ApplicationQ3SourceEntity,
};
use super::simulation::q3::types::Q3SourcePresentationState;
use super::weapon_view::weapon_view_camera;
use crate::bootstrap::audio::q3::Q3SeatAudioOperation;

/// Failure of a Q3 client operation.
#[derive(Debug, Error)]
pub enum Q3ClientError {
    /// Cvar failure.
    #[error(transparent)]
    Cvar(#[from] qa_core::cvar::CvarError),
    /// Buffer failure.
    #[error(transparent)]
    Buffer(#[from] qa_core::cmd_buffer::BufferError),
    /// Frame-time failure.
    #[error(transparent)]
    FrameTime(#[from] FrameTimeError),
    /// Source failure.
    #[error(transparent)]
    Source(#[from] Q3SourceError),
    /// Render failure.
    #[error(transparent)]
    Render(#[from] qa_client::ClientError),
    /// Host factory failure.
    #[error("Q3 backend factory failed: {0}")]
    Factory(String),
    /// Invariant violation (donor `throw new Error`).
    #[error("{0}")]
    Message(String),
}

/// Snapshot source mode (donor `sourceMode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3SourceMode {
    /// Live game.
    Live,
    /// Demo playback.
    Demo,
}

/// Client kind (donor `options.kind`, defaulting to local).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3ClientKind {
    /// Native local client.
    Local,
    /// Native remote client.
    Remote,
    /// QVM guest client.
    Qvm,
}

/// Active-frame draw input (donor `drawActiveFrame` argument).
#[derive(Debug, Clone, Copy)]
pub struct Q3ActiveFrame {
    /// Server time in milliseconds.
    pub server_time_ms: i32,
    /// Demo playback.
    pub demo_playback: bool,
    /// Engine frame number.
    pub engine_frame_number: u64,
}

/// HUD visibility input (donor `weaponHudView` reads).
#[derive(Debug, Clone, Copy)]
pub struct Q3HudPlayer {
    /// Health.
    pub health: i32,
    /// Movement type.
    pub movement_type: MoveType,
    /// Team.
    pub team: Team,
}

/// HUD state read from the native backend.
#[derive(Debug, Clone, Copy)]
pub struct Q3HudState {
    /// Level-shot rendering.
    pub level_shot: bool,
    /// Scoreboard visible.
    pub show_scores: bool,
    /// Player state, if any.
    pub player: Option<Q3HudPlayer>,
}

/// QVM event-handling mode (donor string mapping in `eventHandling`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3EventHandling {
    /// No handling.
    None,
    /// Team menu.
    TeamMenu,
    /// Scoreboard.
    Scoreboard,
    /// Edit HUD.
    EditHud,
}

/// Sampled light (donor `light` return).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q3LightSample {
    /// Ambient light.
    pub ambient: Vec3,
    /// Directed light.
    pub directed: Vec3,
    /// Light direction.
    pub direction: Vec3,
}

/// Q3 dynamic light (donor scene light rows).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q3SceneLight {
    /// Origin.
    pub origin: Vec3,
    /// Radius.
    pub radius: f32,
    /// Color.
    pub color: Vec3,
    /// Additive blending.
    pub additive: bool,
}

/// Presented scene view (donor `Q3PresentedScene` fields consumed here).
#[derive(Debug, Clone)]
pub struct Q3PresentedSceneView {
    /// Scene camera.
    pub camera: SceneCamera,
    /// Scene viewport.
    pub viewport: Rect,
    /// Source refdef (flags, area mask, text).
    pub source: Refdef,
    /// Scene lights.
    pub lights: Vec<Q3SceneLight>,
    /// Portal entities.
    pub portals: Vec<PortalEntity>,
    /// Admitted entity count for range reservation.
    pub admission_entity_count: usize,
}

/// Owned text submission (donor `MaterialTextDraw` without the borrow).
#[derive(Debug, Clone)]
pub struct Q3TextDraw {
    /// Destination rectangle in viewport coordinates.
    pub rect: RenderRect,
    /// Source texture coordinates.
    pub uv: TextureRect,
    /// Draw color (normalized).
    pub color: Vec4,
    /// Compiled shader the text layer selected.
    pub compiled: CompiledMaterial,
}

/// Frame submission (donor `Submission`).
#[derive(Debug, Clone)]
pub enum Q3Submission {
    /// Rendered scene.
    Scene(Box<Q3PresentedSceneView>),
    /// Owned text draw.
    Text(Box<Q3TextDraw>),
    /// Render command (never swap-buffers; the backend contract).
    Command(RenderCommand),
}

/// Portal surface candidate (donor `world.surfaces` reads).
#[derive(Debug, Clone)]
pub struct Q3PortalSurface {
    /// Surface plane.
    pub plane: Plane,
    /// Surface geometry.
    pub geometry: MaterialGeometry,
    /// Finished shader sort.
    pub shader_sort: i32,
    /// Portal range.
    pub portal_range: f32,
}

/// View clear (donor `clear` rows).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q3ViewClear {
    /// Depth clear.
    pub depth: f32,
    /// Color clear, if any.
    pub color: Option<Vec3>,
    /// Stencil clear.
    pub stencil: bool,
}

/// World view for the pipeline (donor `WorldViewInput` fields consumed here).
#[derive(Debug, Clone)]
pub struct Q3WorldView {
    /// View camera.
    pub camera: SceneCamera,
    /// Presentation time in milliseconds.
    pub time_ms: i32,
    /// Presenting seat.
    pub seat: SeatId,
    /// Surface lights.
    pub lights: Vec<SurfaceDynamicLight>,
    /// Q3 dynamic lights.
    pub q3_lights: Vec<Q3SceneLight>,
    /// Visible areas.
    pub visible_areas: HashSet<i32>,
    /// Render text.
    pub render_text: RenderText,
    /// Clear.
    pub clear: Q3ViewClear,
    /// Q1 fog override.
    pub q1_fog: Option<qa_client::render::scene::world::Q1FogParams>,
    /// Source sky override.
    pub source_sky: Option<qa_client::render::scene::q2_sky::Q2SkyView>,
    /// No world model.
    pub no_world_model: bool,
    /// PVS sampling origin override.
    pub pvs_origin: Option<Vec3>,
}

/// Text view for the pipeline (donor text-branch `WorldViewInput`).
#[derive(Debug, Clone)]
pub struct Q3TextView {
    /// View camera.
    pub camera: SceneCamera,
    /// Presentation time in milliseconds.
    pub time_ms: i32,
    /// Presenting seat.
    pub seat: SeatId,
}

/// Scene flags for the pipeline (donor `operations` options).
#[derive(Debug, Clone, Copy)]
pub struct Q3SceneFlags {
    /// No world model.
    pub no_world_model: bool,
    /// Split screen.
    pub split_screen: bool,
    /// Supplemental view weapon.
    pub supplemental_view_weapon: bool,
}

/// Frame environment overrides (donor `frame` `environment`).
#[derive(Debug, Clone, Default)]
pub struct Q3FrameEnvironment {
    /// Q1 fog override.
    pub q1_fog: Option<qa_client::render::scene::world::Q1FogParams>,
    /// Source sky override.
    pub source_sky: Option<qa_client::render::scene::q2_sky::Q2SkyView>,
    /// No world model.
    pub no_world_model: bool,
}

/// Client model presentation input (donor `SimulationPresentation` fields consumed here).
#[derive(Debug, Clone)]
pub struct Q3ClientModel {
    /// Acting actor.
    pub actor: ActorId,
    /// Whether the source client owns rendering.
    pub render_owner_source_client: bool,
    /// View weapon presentation.
    pub view_weapon: bool,
    /// Model path.
    pub path: String,
    /// Held weapon, if any.
    pub held_weapon: Option<i32>,
    /// Whether the model replaces the body.
    pub replaces_body: bool,
    /// Whether the model is visible.
    pub visible: bool,
}

/// Prepared primary body (donor `PreparedPrimaryBody`).
#[derive(Debug, Clone)]
pub struct Q3PrimaryBody {
    /// Acting actor.
    pub actor: ActorId,
    /// Body part.
    pub part: QvmBodyPart,
    /// Geometry owner content.
    pub content: ContentId,
    /// Prepared entity.
    pub entity: PresentSceneEntity,
    /// Base pose.
    pub base: bool,
    /// Model source options.
    pub options: ModelSourceOptions,
    /// Shader texture coordinate override.
    pub shader_tex_coord: Vec2,
    /// Shader content.
    pub shader_content: ContentId,
    /// Presentation time in milliseconds.
    pub time_ms: i32,
}

/// Body pose capture (donor `bodyPose` callback reads).
#[derive(Debug, Clone, Copy)]
pub struct Q3BodyPoseCapture {
    /// Entity number.
    pub number: i32,
    /// Interpolated origin.
    pub origin: Vec3,
    /// Interpolated angles.
    pub angles: Vec3,
}

/// Local seat binding (donor `options.local`).
#[derive(Clone)]
pub struct Q3ClientLocal {
    /// Seat actor.
    pub actor: ActorId,
    /// Seat handle.
    pub seat: SeatId,
    /// Client handle.
    pub client: ClientId,
    /// Seat session (donor `player.actor.session`; the token is hidden).
    pub session: SessionId,
    /// Console close hook.
    pub console_close: Rc<dyn Fn()>,
}

/// Command sinks (donor `options.commands`).
#[derive(Clone)]
pub struct Q3ClientCommands {
    /// Reliable command sink.
    pub reliable: Rc<dyn Fn(&str)>,
    /// Console command sink.
    pub console: Rc<dyn Fn(&str)>,
    /// Print sink.
    pub print: Rc<dyn Fn(&str)>,
}

/// Client command registration (donor `ClientCommandRegistration`, unported).
///
/// Interior mutability: implementations hold their table behind a lock so the
/// session callbacks can share one registration.
pub trait Q3CommandRegistration {
    /// Register a cgame command.
    fn register(&self, name: &str);
    /// Remove a cgame command.
    fn remove(&self, name: &str);
    /// Close the registration.
    fn close(&self);
}

/// Services output callbacks (donor `createApplicationQ3Services` `output`).
#[derive(Clone)]
pub struct Q3ServiceOutput {
    /// Scene submission sink.
    pub scene: Rc<dyn Fn(Q3PresentedSceneView)>,
    /// Command submission sink.
    pub command: Rc<dyn Fn(RenderCommand)>,
    /// Text submission sink.
    pub text: Rc<dyn for<'a> Fn(MaterialTextDraw<'a>)>,
    /// Audio operation sink.
    pub audio: Rc<dyn Fn(Q3SeatAudioOperation)>,
    /// Listener update sink.
    pub listener: Rc<dyn Fn(Vec3, Axis)>,
}

/// Services parameters (donor `createApplicationQ3Services` options consumed here).
pub struct Q3ServicesParams {
    /// Output callbacks.
    pub output: Q3ServiceOutput,
    /// Actor lookup by entity number.
    pub actor_at: Rc<dyn Fn(i32) -> ActorId>,
    /// Clock milliseconds.
    pub now: Rc<dyn Fn() -> f64>,
    /// Engine frame number.
    pub frame_number: Rc<dyn Fn() -> u64>,
    /// Seat viewport probe (live: `prepare` retargets it).
    pub viewport: Rc<dyn Fn() -> Rect>,
    /// Seat handle.
    pub seat: SeatId,
    /// Shared cvar registry.
    pub cvars: Rc<RefCell<CvarRegistry>>,
    /// Shared time cvars, if any.
    pub time_cvars: Option<Rc<RefCell<CvarRegistry>>>,
    /// Shader remap wrapper, if any.
    pub remap_shader: Option<Q3ServiceRemap>,
}

/// Client services (host-built scene/sound/draw/cinematic bundle).
pub trait Q3ClientServices {
    /// Close cinematics.
    fn close_cinematics(&mut self);
    /// Registered model diagnostics.
    fn registered_models(&self) -> Vec<String>;
    /// Registered skin diagnostics.
    fn registered_skins(&self) -> Vec<String>;
}

/// Client session surface (donor `Q3PresentationSession` fields consumed here).
#[derive(Clone)]
pub struct Q3ClientSession {
    /// Product.
    pub product: Product,
    /// Client number.
    pub client_number: i32,
    /// Server message sequence.
    pub server_message_sequence: i32,
    /// Last executed server command.
    pub last_executed_server_command: i32,
    /// Snapshot source.
    pub snapshots: SharedSnapshots,
    /// Command source.
    pub commands: SharedCommands,
    /// Shared cvar registry.
    pub cvars: Rc<RefCell<CvarRegistry>>,
    /// Status visibility probe.
    pub status_visible: Rc<dyn Fn() -> bool>,
    /// Game state probe.
    pub game_state: Rc<dyn Fn() -> Vec<String>>,
    /// Server command probe (sync fold).
    pub server_command: Rc<dyn Fn(i32) -> Option<Vec<String>>>,
    /// Snapshot ping probe.
    pub snapshot_ping: Rc<dyn Fn(i32) -> i32>,
    /// Server time probe.
    pub server_time: Rc<dyn Fn() -> i32>,
    /// Reliable command sink.
    pub add_reliable_command: Rc<dyn Fn(&str)>,
    /// Console command sink.
    pub append_console_command: Rc<dyn Fn(&str)>,
    /// Cgame command registration.
    pub register_cgame_command: Rc<dyn Fn(&str)>,
    /// User command value sink (weapon, sensitivity).
    pub set_user_command_value: Rc<dyn Fn(i32, f32)>,
    /// Current-world assertion.
    pub assert_current: Rc<dyn Fn()>,
    /// Print sink.
    pub print: Rc<dyn Fn(&str)>,
}

/// Render gates for the native backend (donor presentation-hook wrapper contract).
///
/// The ported presentation has no hook surface, so the host factory replicates the
/// donor wrappers (`q3-client.ts` `character`/`event`/`predictItem`/`viewWeapon`/
/// `playerWeapon`): hidden bodies always render through; otherwise the host hooks run
/// when present, else `q3:` providers render through. View weapons additionally
/// require visibility without the supplemental flag. Events bump the event count.
#[derive(Clone)]
pub struct Q3RenderGates {
    /// Whether an entity number is body-hidden.
    pub body_hidden: Rc<dyn Fn(i32) -> bool>,
    /// Whether the view weapon renders (visible without the supplemental flag).
    pub view_weapon: Rc<dyn Fn() -> bool>,
    /// Whether the supplemental view weapon renders.
    pub supplemental_view_weapon: Rc<dyn Fn() -> bool>,
    /// Note a rendered event.
    pub note_event: Rc<dyn Fn()>,
}

/// Native backend parameters.
pub struct Q3NativeParams {
    /// Client session.
    pub session: Q3ClientSession,
    /// Movement host (round proxy for local clients).
    pub movement: Rc<RefCell<dyn PresentationMovementHost>>,
    /// Weapon HUD reader, if any.
    pub weapon_hud: Option<Rc<RefCell<dyn WeaponHudReader>>>,
    /// Rage Pro hardware path.
    pub hardware_ragepro: bool,
    /// Body capture submission.
    pub submit_body: Rc<dyn Fn(i32, QvmBodyPart, RefModelEntity, bool) -> bool>,
    /// Body pose capture sink.
    pub record_body_pose: Rc<dyn Fn(Q3BodyPoseCapture)>,
    /// Render gates.
    pub gates: Q3RenderGates,
    /// Weapon selection sink.
    pub weapon_selection: Rc<dyn Fn(i32)>,
    /// Point light sampler.
    pub light_for_point: Rc<dyn Fn(Vec3) -> Q3LightSample>,
    /// Remaining memory probe (donor `freemem`).
    pub memory_remaining: Rc<dyn Fn() -> u64>,
}

/// Key catcher cell (donor scalar `keyCatcher` accessors).
#[derive(Clone)]
pub struct Q3KeyCatcher {
    /// Read the mask.
    pub get: Rc<dyn Fn() -> i32>,
    /// Write the mask.
    pub set: Rc<dyn Fn(i32)>,
}

/// QVM backend parameters.
pub struct Q3QvmParams {
    /// Client session.
    pub session: Q3ClientSession,
    /// Retained presentation artifacts, if any.
    pub artifacts: Option<QvmPresentationArtifacts>,
    /// Command context.
    pub command_context: CommandContext,
    /// Shared command buffer.
    pub commands: Rc<RefCell<CommandBuffer>>,
    /// Key catcher cell.
    pub key_catcher: Q3KeyCatcher,
    /// View weapon visibility probe.
    pub view_weapon_visible: Rc<dyn Fn() -> bool>,
    /// Held-weapon actor probe.
    pub held_weapon_actor: Rc<dyn Fn(i32) -> Option<ActorId>>,
    /// Body capture active probe.
    pub body_capture_active: Rc<dyn Fn() -> bool>,
    /// Body selected probe.
    pub body_selected: Rc<dyn Fn(i32) -> bool>,
    /// Body capture submission.
    pub submit_body: Rc<dyn Fn(i32, QvmBodyPart, RefModelEntity, bool) -> bool>,
    /// Body overrides active probe.
    pub body_overrides_active: Rc<dyn Fn() -> bool>,
    /// Body hidden probe.
    pub body_hidden: Rc<dyn Fn(i32) -> bool>,
    /// Command removal sink.
    pub remove_command: Rc<dyn Fn(&str)>,
    /// Point light sampler.
    pub light_for_point: Rc<dyn Fn(Vec3) -> Q3LightSample>,
}

/// Native (TypeScript) game backend.
pub trait Q3NativeGame {
    /// Draw the active frame.
    fn draw_active_frame(&mut self, frame: Q3ActiveFrame) -> Result<(), Q3ClientError>;
    /// Execute a console command; `true` means handled.
    fn console_execute(&mut self, argv: &[String]) -> Result<bool, Q3ClientError>;
    /// Whether the console handles a command.
    fn console_handles(&self, name: &str) -> bool;
    /// Key event.
    fn key_event(&mut self, key: i32, down: bool) -> Result<(), Q3ClientError>;
    /// Mouse motion event.
    fn mouse_event(&mut self, x: f64, y: f64) -> Result<(), Q3ClientError>;
    /// Event-handling mode (0-3, validated by the caller).
    fn event_handling(&mut self, mode: u8) -> Result<(), Q3ClientError>;
    /// HUD state.
    fn hud_state(&self) -> Q3HudState;
    /// Close the backend.
    fn close(&mut self);
}

/// QVM guest game backend.
pub trait Q3QvmGame {
    /// Draw a frame.
    fn draw(&mut self, time_ms: i32, demo_playback: bool) -> Result<(), Q3ClientError>;
    /// Refresh status after a frame.
    fn refresh_status(&mut self) -> Result<(), Q3ClientError>;
    /// Execute a command; `true` means handled.
    fn command(&mut self, argv: &[String]) -> Result<bool, Q3ClientError>;
    /// Key event.
    fn key_event(&mut self, key: i32, down: bool) -> Result<(), Q3ClientError>;
    /// Mouse motion event.
    fn mouse_event(&mut self, x: f64, y: f64) -> Result<(), Q3ClientError>;
    /// Event-handling mode.
    fn event_handling(&mut self, mode: Q3EventHandling) -> Result<(), Q3ClientError>;
    /// Whether the guest captures input.
    fn captures_input(&self) -> bool;
    /// Shared held weapons.
    fn shared_held_weapons(&self) -> Vec<QvmHeldWeapon>;
    /// Shared equipment view visibility.
    fn shared_equipment_view_visible(&self) -> bool;
    /// Shared equipment HUD flag.
    fn shared_equipment_hud(&self) -> bool;
    /// Retained presentation artifacts for video reopen.
    fn presentation_artifacts(&self) -> Result<QvmPresentationArtifacts, Q3ClientError>;
    /// Shut down the guest.
    fn shutdown(&mut self) -> Result<(), Q3ClientError>;
    /// Close the backend.
    fn close(&mut self);
}

/// Client backend (donor `backend` union).
pub enum Q3ClientBackend {
    /// Native presentation backend.
    Native(Box<dyn Q3NativeGame>),
    /// QVM guest backend.
    Qvm(Box<dyn Q3QvmGame>),
}

/// Remote snapshot source (donor network `ApplicationQ3ClientSource`).
pub trait Q3RemoteSource {
    /// Actor by entity number.
    fn actor_at(&mut self, number: i32) -> Result<ActorId, Q3ClientError>;
    /// Game state configstrings.
    fn game_state(&self) -> Vec<String>;
    /// Server command by sequence (sync fold).
    fn server_command(&mut self, sequence: i32) -> Result<Option<Vec<String>>, Q3ClientError>;
    /// Snapshot ping, if tracked.
    fn snapshot_ping(&self, _number: i32) -> Option<i32> {
        None
    }
    /// Presentation time in milliseconds.
    fn time(&self) -> i32;
    /// Client number.
    fn client_number(&self) -> i32;
    /// Server message sequence.
    fn server_message_sequence(&self) -> i32 {
        0
    }
    /// Last executed server command.
    fn last_executed_server_command(&self) -> i32 {
        0
    }
    /// System info, if any.
    fn system_info(&self) -> Option<String>;
    /// Source mode.
    fn source_mode(&self) -> Q3SourceMode;
}

/// Client media (host-built asset surface consumed here).
pub trait Q3ClientMedia {
    /// Media content id.
    fn content(&self) -> ContentId;
    /// Provider slot for a scene model.
    fn model_provider(&self, model: &SceneModel) -> Option<usize>;
    /// Content for a provider slot.
    fn provider_content(&self, slot: usize) -> Option<ContentId>;
    /// Build the model light sampler.
    fn build_light_sampler(&self) -> ModelLightSampler;
    /// Close media.
    fn close(&self);
}

/// Client audio (donor `ApplicationAudio` surface consumed here).
pub trait Q3ClientAudio {
    /// Receive a cgame audio frame.
    fn receive_cgame_frame(&self, content: &ContentId, seat: &SeatId, operations: Vec<Q3SeatAudioOperation>);
}

/// Render pipeline (host adapter over the redesigned render API).
///
/// Each method replicates the donor `frame()` call sequence for its branch; the port
/// computes every input.
pub trait Q3RenderPipeline {
    /// Begin a frame.
    fn begin_frame(&mut self);
    /// Submit a render command.
    fn submit_command(&mut self, command: RenderCommand);
    /// Submit a text view.
    fn submit_text(&mut self, draw: &MaterialTextDraw, viewport: Rect, view: &Q3TextView);
    /// Submit a no-world-model scene view.
    fn submit_direct_scene(&mut self, scene: &Q3PresentedSceneView, view: &Q3WorldView, flags: Q3SceneFlags);
    /// Submit a world scene view with merged effects.
    fn submit_world_scene(
        &mut self,
        scene: &Q3PresentedSceneView,
        view: &Q3WorldView,
        flags: Q3SceneFlags,
        operations: &[SceneOperation],
        view_offset: Option<Vec3>,
    );
    /// Preload scenes.
    fn preload_scenes(&mut self, scenes: &[Q3PresentedSceneView]);
    /// Visible surface indices for a portal pass.
    fn visible_surfaces(&mut self, camera: &SceneCamera, view: &Q3WorldView) -> Vec<usize>;
    /// Portal surface candidate by index.
    fn portal_surface(&self, index: usize) -> Option<Q3PortalSurface>;
    /// Finish the frame.
    fn finish_frame(&mut self, present: bool) -> RenderFrame;
    /// Close the pipeline.
    fn close(&mut self);
}

/// Shared registries alias.
pub type SharedCvars = Rc<RefCell<CvarRegistry>>;
/// Host shader remap (sync fold of the donor async remap).
pub type RemapShaderHost = Rc<dyn Fn(&str, &str, &str, bool, &dyn Fn() -> bool) -> Result<(), String>>;
/// Services shader-remap wrapper.
pub type Q3ServiceRemap = Rc<dyn Fn(&str, &str, &str) -> Result<(), String>>;
/// Services factory.
pub type Q3CreateServices = Rc<dyn Fn(Q3ServicesParams) -> Result<Box<dyn Q3ClientServices>, String>>;
/// Native backend factory.
pub type Q3CreateNative = Rc<dyn Fn(Q3NativeParams) -> Result<Box<dyn Q3NativeGame>, String>>;
/// QVM backend factory.
pub type Q3CreateQvm = Rc<dyn Fn(Q3QvmParams) -> Result<Box<dyn Q3QvmGame>, String>>;
/// Prediction-command adapter.
pub type Q3PredictionAdapter = Rc<dyn Fn(&ActorCommand, i32) -> PredictionCommand>;
/// Shared snapshot views.
pub type SharedSnapshots = Rc<RefCell<dyn SnapshotSource>>;
/// Shared command views.
pub type SharedCommands = Rc<RefCell<dyn CommandSource>>;
/// Single-shot video reopen.
pub type QvmVideoReopen = Box<dyn FnMut(QvmVideoReopenOptions) -> Result<ApplicationQ3Client, Q3ClientError>>;

/// Common client options (donor `ApplicationQ3ClientCommonOptions`).
#[derive(Clone)]
pub struct Q3ClientCommonOptions {
    /// Shared cvar owner, if adopted.
    pub cvars: Option<SharedCvars>,
    /// Shared time cvars, if any.
    pub time_cvars: Option<SharedCvars>,
    /// Weapon HUD reader, if any.
    pub weapon_hud: Option<Rc<RefCell<dyn WeaponHudReader>>>,
    /// Local seat binding.
    pub local: Q3ClientLocal,
    /// Client audio.
    pub audio: Rc<dyn Q3ClientAudio>,
    /// Initial settings applied to the owner.
    pub settings: Vec<CvarSnapshot>,
    /// Split screen.
    pub split_screen: bool,
    /// Viewport probe.
    pub viewport: Rc<dyn Fn() -> Rect>,
    /// Clock milliseconds probe.
    pub now: Rc<dyn Fn() -> f64>,
    /// Command sinks.
    pub commands: Q3ClientCommands,
    /// Command registration.
    pub command_registration: Rc<dyn Q3CommandRegistration>,
    /// Server settings probe, if any.
    pub server_settings: Option<Rc<dyn Fn() -> Vec<CvarSnapshot>>>,
    /// Current-world assertion, if any.
    pub assert_current: Option<Rc<dyn Fn()>>,
    /// Client media.
    pub media: Rc<dyn Q3ClientMedia>,
    /// Render pipeline.
    pub pipeline: Rc<RefCell<dyn Q3RenderPipeline>>,
    /// Services factory.
    pub create_services: Q3CreateServices,
    /// Native backend factory.
    pub create_native: Q3CreateNative,
    /// QVM backend factory.
    pub create_qvm: Q3CreateQvm,
    /// Shared command buffer for the QVM backend.
    pub command_buffer: Rc<RefCell<CommandBuffer>>,
    /// Remaining memory probe (donor `freemem`).
    pub memory_remaining: Rc<dyn Fn() -> u64>,
    /// Host shader remap, if any.
    pub remap_shader: Option<RemapShaderHost>,
    /// Display renderer for hardware detection.
    pub renderer: QvmDisplayRenderer,
}

/// Local client options (donor `{ kind?: "local", ... }`).
#[derive(Clone)]
pub struct Q3LocalOptions {
    /// Movement host.
    pub movement: Rc<RefCell<dyn PresentationMovementHost>>,
    /// Initial source state.
    pub initial: Q3SourcePresentationState,
    /// Link bounds by entity number.
    pub link_bounds: Rc<dyn Fn(i32) -> Option<Bounds>>,
    /// Source actor by entity number.
    pub source_actor: Rc<dyn Fn(i32) -> Option<ActorId>>,
    /// Prediction command adapter, if any (donor default otherwise).
    pub prediction_command: Option<Q3PredictionAdapter>,
    /// Scene queries for the ported visibility selector.
    pub queries: Rc<dyn ApplicationQ3SceneQueries>,
    /// World leaf count for the ported visibility selector (donor
    /// `assets.world.map.leaves.length`; assets stay host-owned).
    pub leaf_count: i32,
    /// Seat-session membership probe (donor `actor.session` comparison).
    pub same_session: Rc<dyn Fn(&ActorId) -> bool>,
    /// Round server settings probe, if any.
    pub server_settings: Option<Rc<dyn Fn() -> Vec<CvarSnapshot>>>,
}

/// Remote client options (donor `{ kind: "remote", ... }`).
#[derive(Clone)]
pub struct Q3RemoteOptions {
    /// Movement host.
    pub movement: Rc<RefCell<dyn PresentationMovementHost>>,
    /// Snapshot source.
    pub snapshots: SharedSnapshots,
    /// Command source.
    pub commands: SharedCommands,
    /// Remote surface.
    pub surface: Rc<RefCell<dyn Q3RemoteSource>>,
    /// Initial player state.
    pub initial_player: NetPlayerState,
}

/// QVM client options (donor `{ kind: "qvm", ... }`).
///
/// Guest inputs (browser, keys, guest cvars/input/client-state, connection)
/// are captured by the host factory directly.
#[derive(Clone)]
pub struct Q3QvmOptions {
    /// Local server flag.
    pub local_server: bool,
    /// Equipment primary-weapon probe, if any (donor `equipmentWeapon`).
    pub equipment_primary_weapon: Option<Rc<dyn Fn() -> Option<i32>>>,
    /// Snapshot source.
    pub snapshots: SharedSnapshots,
    /// Command source.
    pub commands: SharedCommands,
    /// Remote surface.
    pub surface: Rc<RefCell<dyn Q3RemoteSource>>,
}

/// Client kind options (donor kind union).
#[derive(Clone)]
pub enum Q3ClientKindOptions {
    /// Native local client.
    Local(Box<Q3LocalOptions>),
    /// Native remote client.
    Remote(Box<Q3RemoteOptions>),
    /// QVM guest client.
    Qvm(Q3QvmOptions),
}

/// Client options (donor `ApplicationQ3ClientOptions`).
#[derive(Clone)]
pub struct ApplicationQ3ClientOptions {
    /// Common options.
    pub common: Q3ClientCommonOptions,
    /// Kind options.
    pub kind: Q3ClientKindOptions,
}

/// Video reopen overrides (donor `QvmVideoReopenOptions`).
#[derive(Clone)]
pub struct QvmVideoReopenOptions {
    /// Display renderer.
    pub renderer: QvmDisplayRenderer,
    /// Viewport probe.
    pub viewport: Rc<dyn Fn() -> Rect>,
    /// Command registration.
    pub command_registration: Rc<dyn Q3CommandRegistration>,
}

/// Local round (donor `ApplicationQ3LocalRound`).
pub struct ApplicationQ3LocalRound {
    /// Round actor.
    pub actor: ActorId,
    /// Initial source state.
    pub initial: Q3SourcePresentationState,
    /// Movement host.
    pub movement: Rc<RefCell<dyn PresentationMovementHost>>,
    /// Link bounds by entity number.
    pub link_bounds: Rc<dyn Fn(i32) -> Option<Bounds>>,
    /// Source actor by entity number.
    pub source_actor: Rc<dyn Fn(i32) -> Option<ActorId>>,
    /// Prediction command adapter, if any.
    pub prediction_command: Option<Q3PredictionAdapter>,
    /// Round server settings probe, if any.
    pub server_settings: Option<Rc<dyn Fn() -> Vec<CvarSnapshot>>>,
}

/// Callback-shared mutable client state.
struct Q3ClientShared {
    camera: SceneCamera,
    viewport: Rect,
    command_names: HashSet<String>,
    selection: (i32, f32),
    event_count: u64,
    key_catcher: i32,
    view_weapon_visible: bool,
    supplemental_view_weapon: bool,
    closed: bool,
    initializing: bool,
    status_visible: bool,
    frame_number: u64,
    weapon_selection: Option<i32>,
    submissions: Vec<Q3Submission>,
    audio_operations: Vec<Q3SeatAudioOperation>,
    primary_bodies: Vec<Q3PrimaryBody>,
    body_poses: HashMap<ActorId, (Vec3, Vec3)>,
    pose_actors: HashMap<u32, ActorId>,
    hidden_bodies: HashMap<u32, ActorId>,
    selected_bodies: HashMap<u32, ActorId>,
    selected_held_actors: HashMap<u32, ActorId>,
}

/// Snapshot source (donor `source` field).
pub enum Q3ClientSource {
    /// Native local source.
    Local(Box<ApplicationQ3Source>),
    /// Remote surface.
    Remote(Rc<RefCell<dyn Q3RemoteSource>>),
}

/// Snapshot/command views over a shared local source.
#[derive(Clone)]
struct LocalSourceViews {
    source: Rc<RefCell<Q3ClientSource>>,
    product: Product,
}

impl SnapshotSource for LocalSourceViews {
    fn current(&self) -> SnapshotCurrent {
        match &*self.source.borrow() {
            Q3ClientSource::Local(source) => source.current(),
            Q3ClientSource::Remote(_) => panic!("remote source needs host snapshot views"),
        }
    }

    fn read(&mut self, number: i32) -> PresentClientResult<Option<PresentSnapshot>> {
        match &mut *self.source.borrow_mut() {
            Q3ClientSource::Local(source) => Ok(source
                .read(number)
                .map(|snapshot| retail_snapshot(self.product, &snapshot))),
            Q3ClientSource::Remote(_) => panic!("remote source needs host snapshot views"),
        }
    }
}

impl CommandSource for LocalSourceViews {
    fn current_number(&self) -> i32 {
        match &*self.source.borrow() {
            Q3ClientSource::Local(source) => source.commands.current_number(),
            Q3ClientSource::Remote(_) => panic!("remote source needs host command views"),
        }
    }

    fn read(&self, number: i32) -> PresentClientResult<Option<PredictionCommand>> {
        match &*self.source.borrow() {
            Q3ClientSource::Local(source) => source.commands.read(number),
            Q3ClientSource::Remote(_) => panic!("remote source needs host command views"),
        }
    }
}

/// Round movement proxy (donor `initialize` movement wrapper).
struct RoundMovement {
    round: Rc<RefCell<Option<ApplicationQ3LocalRound>>>,
}

impl PresentationMovementHost for RoundMovement {
    fn command_timing(&self) -> qa_content::q3::presentation::movement_host::CommandTiming {
        self.round
            .borrow()
            .as_ref()
            .expect("Q3 client has no native local round")
            .movement
            .borrow()
            .command_timing()
    }

    fn move_player(
        &mut self,
        state: &mut qa_content::q3::base::shared::player_state::PlayerState,
        command: &PredictionCommand,
        options: &dyn qa_content::q3::presentation::movement_host::PresentationMovementOptions,
    ) -> qa_content::q3::presentation::movement_host::MoveBounds {
        self.round
            .borrow_mut()
            .as_mut()
            .expect("Q3 client has no native local round")
            .movement
            .borrow_mut()
            .move_player(state, command, options)
    }

    fn update_view_angles(
        &mut self,
        state: &mut qa_content::q3::base::shared::player_state::PlayerState,
        command: &PredictionCommand,
    ) {
        self.round
            .borrow_mut()
            .as_mut()
            .expect("Q3 client has no native local round")
            .movement
            .borrow_mut()
            .update_view_angles(state, command);
    }
}

/// Seat-owned Q3 client (donor `ApplicationQ3Client`).
pub struct ApplicationQ3Client {
    product: Product,
    local: Q3ClientLocal,
    audio: Rc<dyn Q3ClientAudio>,
    split_screen: bool,
    now_fn: Rc<dyn Fn() -> f64>,
    commands: Q3ClientCommands,
    registration: Rc<dyn Q3CommandRegistration>,
    server_settings_fn: Option<Rc<dyn Fn() -> Vec<CvarSnapshot>>>,
    assert_current: Option<Rc<dyn Fn()>>,
    media: Rc<dyn Q3ClientMedia>,
    pipeline: Rc<RefCell<dyn Q3RenderPipeline>>,
    command_buffer: Rc<RefCell<CommandBuffer>>,
    memory_remaining: Rc<dyn Fn() -> u64>,
    time_cvars: Option<SharedCvars>,
    cvars: SharedCvars,
    source: Rc<RefCell<Q3ClientSource>>,
    round: Rc<RefCell<Option<ApplicationQ3LocalRound>>>,
    shared: Rc<RefCell<Q3ClientShared>>,
    sampler: Rc<ModelLightSampler>,
    backend: Option<Q3ClientBackend>,
    services: Option<Box<dyn Q3ClientServices>>,
    time_mirror: Option<SharedCvarMirror>,
    applied_time_system_info: Option<String>,
    shared_cvar_names: HashSet<String>,
    reopen_options: Option<ApplicationQ3ClientOptions>,
    equipment_primary_weapon: Option<Rc<dyn Fn() -> Option<i32>>>,
}

/// Physical mouse button to Quake button (donor `input/mouse-buttons.ts`
/// `quakeMouseButton`: middle and right swap; unported sibling, inlined).
fn quake_mouse_button(physical: i32) -> i32 {
    if physical == 2 {
        3
    } else if physical == 3 {
        2
    } else {
        physical
    }
}

impl ApplicationQ3Client {
    /// Create a client (donor static `create`, sync fold).
    ///
    /// `artifacts` carries retained QVM presentation artifacts for video reopen.
    pub fn create(
        options: ApplicationQ3ClientOptions,
        artifacts: Option<QvmPresentationArtifacts>,
    ) -> Result<Self, Q3ClientError> {
        if let Some(assert) = options.common.assert_current.as_ref() {
            assert();
        }
        let kind = match options.kind {
            Q3ClientKindOptions::Local(_) => Q3ClientKind::Local,
            Q3ClientKindOptions::Remote(_) => Q3ClientKind::Remote,
            Q3ClientKindOptions::Qvm(_) => Q3ClientKind::Qvm,
        };
        let product = match &options.kind {
            Q3ClientKindOptions::Local(local) => local.initial.product,
            Q3ClientKindOptions::Remote(_) | Q3ClientKindOptions::Qvm(_) => Product::Baseq3,
        };
        if let Q3ClientKindOptions::Local(local) = &options.kind {
            let seat = local
                .initial
                .clients
                .iter()
                .any(|client| client.actor == options.common.local.actor);
            if !seat {
                return Err(Q3ClientError::Message(
                    "Q3 cgame seat lacks a source player".to_string(),
                ));
            }
        }
        // Round and local source first: the source constructor needs round callbacks.
        let round: Rc<RefCell<Option<ApplicationQ3LocalRound>>> = Rc::new(RefCell::new(None));
        let source = match &options.kind {
            Q3ClientKindOptions::Local(local) => {
                *round.borrow_mut() = Some(ApplicationQ3LocalRound {
                    actor: options.common.local.actor.clone(),
                    initial: local.initial.clone(),
                    movement: local.movement.clone(),
                    link_bounds: local.link_bounds.clone(),
                    source_actor: local.source_actor.clone(),
                    prediction_command: local.prediction_command.clone(),
                    server_settings: local.server_settings.clone(),
                });
                let round_ref = round.clone();
                let round_bounds = round.clone();
                let prediction = local.prediction_command.clone();
                let round_prediction = round.clone();
                let console_close = options.common.local.console_close.clone();
                let console_sink = options.common.commands.console.clone();
                let select_queries = local.queries.clone();
                let select_leaf_count = local.leaf_count;
                let select_print = options.common.commands.print.clone();
                let local_source = ApplicationQ3Source::new(ApplicationQ3SourceOptions {
                    actor: options.common.local.actor.clone(),
                    initial: local.initial.clone(),
                    select: Box::new(move |player, state| {
                        let rows = visibility_source_rows(state);
                        let print = select_print.clone();
                        let mut emit = move |text: &str| print(text);
                        select_application_q3_snapshot(
                            player,
                            &rows,
                            select_queries.as_ref(),
                            &|number| {
                                round_bounds
                                    .borrow()
                                    .as_ref()
                                    .and_then(|round| (round.link_bounds)(number))
                            },
                            select_leaf_count,
                            &mut emit,
                        )
                        .unwrap_or_else(|error| panic!("Q3 snapshot selection failed: {error}"))
                    }),
                    source_actor: Box::new(move |number| {
                        round_ref
                            .borrow()
                            .as_ref()
                            .and_then(|round| (round.source_actor)(number))
                    }),
                    same_session: {
                        let same = local.same_session.clone();
                        Box::new(move |actor| same(actor))
                    },
                    prediction_command: Some(Box::new(move |command, time| {
                        if let Some(custom) = prediction.as_ref() {
                            return custom(command, time);
                        }
                        if let Some(round) = round_prediction.borrow().as_ref() {
                            if let Some(custom) = round.prediction_command.as_ref() {
                                return custom(command, time);
                            }
                        }
                        default_prediction_command(command)
                    })),
                    level_shot: Some(Box::new(move || {
                        console_close();
                        console_sink("wait; wait; wait; wait; screenshot levelshot\n");
                    })),
                })?;
                Q3ClientSource::Local(Box::new(local_source))
            }
            Q3ClientKindOptions::Remote(remote) => Q3ClientSource::Remote(remote.surface.clone()),
            Q3ClientKindOptions::Qvm(qvm) => Q3ClientSource::Remote(qvm.surface.clone()),
        };
        let source = Rc::new(RefCell::new(source));
        let viewport_value = (options.common.viewport)();
        let player = match &options.kind {
            Q3ClientKindOptions::Qvm(_) => None,
            Q3ClientKindOptions::Remote(remote) => Some((
                to_vec3(remote.initial_player.origin),
                remote.initial_player.viewheight,
                to_vec3(remote.initial_player.viewangles),
            )),
            Q3ClientKindOptions::Local(local) => local
                .initial
                .clients
                .iter()
                .find(|client| client.actor == options.common.local.actor)
                .map(|client| (client.state.origin, client.state.view_height, client.state.view_angles)),
        };
        let Some((origin, view_height, view_angles)) = player else {
            if !matches!(options.kind, Q3ClientKindOptions::Qvm(_)) {
                return Err(Q3ClientError::Message(
                    "Q3 cgame seat lacks a source player".to_string(),
                ));
            }
            return Self::finish_create(options, artifacts, kind, product, round, source, viewport_value, None);
        };
        Self::finish_create(
            options,
            artifacts,
            kind,
            product,
            round,
            source,
            viewport_value,
            Some((origin, view_height, view_angles)),
        )
    }

    /// Complete creation once the camera player resolves.
    #[allow(clippy::too_many_arguments)]
    fn finish_create(
        options: ApplicationQ3ClientOptions,
        artifacts: Option<QvmPresentationArtifacts>,
        kind: Q3ClientKind,
        product: Product,
        round: Rc<RefCell<Option<ApplicationQ3LocalRound>>>,
        source: Rc<RefCell<Q3ClientSource>>,
        viewport_value: Rect,
        player: Option<(Vec3, i32, Vec3)>,
    ) -> Result<Self, Q3ClientError> {
        let (origin, view_height, view_angles) =
            player.unwrap_or((Vec3 { x: 0.0, y: 0.0, z: 0.0 }, 0, Vec3 { x: 0.0, y: 0.0, z: 0.0 }));
        let camera = SceneCamera {
            origin: Vec3 {
                x: origin.x,
                y: origin.y,
                z: origin.z + view_height as f32,
            },
            axis: angles_to_axis(view_angles),
            viewport: viewport_value,
            projection: perspective_projection(90.0, 73.739_79, 16384.0, 4.0)?,
            clip: CameraClip::None,
        };
        let cvars = match options.common.cvars.clone() {
            Some(shared) => shared,
            None => Rc::new(RefCell::new(CvarRegistry::new(Dialect::Q3))),
        };
        let shared = Rc::new(RefCell::new(Q3ClientShared {
            camera,
            viewport: viewport_value,
            command_names: HashSet::new(),
            selection: (2, 1.0),
            event_count: 0,
            key_catcher: 0,
            view_weapon_visible: true,
            supplemental_view_weapon: false,
            closed: false,
            initializing: true,
            status_visible: true,
            frame_number: 0,
            weapon_selection: None,
            submissions: Vec::new(),
            audio_operations: Vec::new(),
            primary_bodies: Vec::new(),
            body_poses: HashMap::new(),
            pose_actors: HashMap::new(),
            hidden_bodies: HashMap::new(),
            selected_bodies: HashMap::new(),
            selected_held_actors: HashMap::new(),
        }));
        let mut client = Self {
            product,
            local: options.common.local.clone(),
            audio: options.common.audio.clone(),
            split_screen: options.common.split_screen,
            now_fn: options.common.now.clone(),
            commands: options.common.commands.clone(),
            registration: options.common.command_registration.clone(),
            server_settings_fn: options.common.server_settings.clone(),
            assert_current: options.common.assert_current.clone(),
            media: options.common.media.clone(),
            pipeline: options.common.pipeline.clone(),
            command_buffer: options.common.command_buffer.clone(),
            memory_remaining: options.common.memory_remaining.clone(),
            time_cvars: options.common.time_cvars.clone(),
            cvars,
            source,
            round,
            shared,
            sampler: Rc::new(options.common.media.build_light_sampler()),
            backend: None,
            services: None,
            time_mirror: None,
            applied_time_system_info: None,
            shared_cvar_names: cvar_table(product)
                .iter()
                .map(|definition| definition.name.to_lowercase())
                .collect(),
            reopen_options: None,
            equipment_primary_weapon: None,
        };
        for setting in &options.common.settings {
            client.cvars.borrow_mut().set(&setting.name, &setting.value, true)?;
        }
        for setting in client.server_settings() {
            if client.shared_cvar_names.contains(&setting.name.to_lowercase()) {
                client.cvars.borrow_mut().set(&setting.name, &setting.value, true)?;
            }
        }
        client.refresh_system_info()?;
        let remote = matches!(kind, Q3ClientKind::Remote);
        let qvm_remote = match &options.kind {
            Q3ClientKindOptions::Qvm(qvm) => !qvm.local_server,
            _ => false,
        };
        client
            .cvars
            .borrow_mut()
            .set("sv_running", if remote || qvm_remote { "0" } else { "1" }, true)?;
        let result = client.initialize(&options, artifacts);
        if result.is_err() {
            client.close();
        }
        result?;
        if let Some(assert) = client.assert_current.as_ref() {
            assert();
        }
        client.shared.borrow_mut().initializing = false;
        client.bind_frame_time()?;
        if let Q3ClientKindOptions::Qvm(qvm) = &options.kind {
            client.equipment_primary_weapon = qvm.equipment_primary_weapon.clone();
            client.reopen_options = Some(options);
        }
        Ok(client)
    }
}

/// User-command selection (donor `userCommandSelection`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q3UserCommandSelection {
    /// Selected weapon.
    pub weapon: i32,
    /// Sensitivity scale.
    pub sensitivity: f32,
}

/// Weapon HUD visibility (donor `weaponHudView`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q3WeaponHudView {
    /// HUD visible.
    pub visible: bool,
    /// Aggregate ammo warning.
    pub aggregate_warning: bool,
}

/// Registered resource diagnostics (donor `resourceDiagnostics`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3ResourceDiagnostics {
    /// Registered models.
    pub models: Vec<String>,
    /// Registered skins.
    pub skins: Vec<String>,
}

/// Copy a network triple into a vector.
fn to_vec3(value: [f32; 3]) -> Vec3 {
    Vec3 {
        x: value[0],
        y: value[1],
        z: value[2],
    }
}

/// Convert a network trajectory to the presentation shape.
fn retail_trajectory(source: &Q3Trajectory) -> Trajectory {
    let kind = TrajectoryType::from_i32(source.trajectory_type)
        .unwrap_or_else(|_| panic!("Q3 snapshot has unknown trajectory type {}", source.trajectory_type));
    Trajectory {
        trajectory_type: kind,
        time: source.time,
        duration: source.duration,
        base: to_vec3(source.base),
        delta: to_vec3(source.delta),
    }
}

/// Convert a network entity state to the presentation shape.
fn retail_entity(source: &Q3EntityState) -> EntityState {
    EntityState {
        number: source.number,
        e_type: source.e_type,
        e_flags: source.e_flags,
        pos: retail_trajectory(&source.pos),
        apos: retail_trajectory(&source.apos),
        time: source.time,
        time2: source.time2,
        origin: to_vec3(source.origin),
        origin2: to_vec3(source.origin2),
        angles: to_vec3(source.angles),
        angles2: to_vec3(source.angles2),
        other_entity_num: source.other_entity_num,
        other_entity_num2: source.other_entity_num2,
        ground_entity_num: source.ground_entity_num,
        constant_light: source.constant_light,
        loop_sound: source.loop_sound,
        modelindex: source.modelindex,
        modelindex2: source.modelindex2,
        client_num: source.client_num,
        frame: source.frame,
        solid: source.solid,
        event: source.event,
        event_parm: source.event_parm,
        powerups: source.powerups,
        weapon: source.weapon,
        legs_anim: source.legs_anim,
        torso_anim: source.torso_anim,
        generic1: source.generic1,
    }
}

/// Copy a vector into a network triple.
fn to_triple(value: Vec3) -> [f32; 3] {
    [value.x, value.y, value.z]
}

/// Convert a presentation trajectory to the network shape.
fn transport_trajectory(source: &Trajectory) -> Q3Trajectory {
    Q3Trajectory {
        trajectory_type: source.trajectory_type as i32,
        time: source.time,
        duration: source.duration,
        base: to_triple(source.base),
        delta: to_triple(source.delta),
    }
}

/// Convert a presentation entity state to the network shape.
fn transport_entity(source: &EntityState) -> Q3EntityState {
    Q3EntityState {
        number: source.number,
        e_type: source.e_type,
        e_flags: source.e_flags,
        pos: transport_trajectory(&source.pos),
        apos: transport_trajectory(&source.apos),
        time: source.time,
        time2: source.time2,
        origin: to_triple(source.origin),
        origin2: to_triple(source.origin2),
        angles: to_triple(source.angles),
        angles2: to_triple(source.angles2),
        other_entity_num: source.other_entity_num,
        other_entity_num2: source.other_entity_num2,
        ground_entity_num: source.ground_entity_num,
        constant_light: source.constant_light,
        loop_sound: source.loop_sound,
        modelindex: source.modelindex,
        modelindex2: source.modelindex2,
        client_num: source.client_num,
        frame: source.frame,
        solid: source.solid,
        event: source.event,
        event_parm: source.event_parm,
        powerups: source.powerups,
        weapon: source.weapon,
        legs_anim: source.legs_anim,
        torso_anim: source.torso_anim,
        generic1: source.generic1,
    }
}

/// Convert copied source rows to visibility-selector rows (app-to-visibility
/// conversion at the snapshot read).
fn visibility_source_rows(state: &Q3SourcePresentationState) -> Vec<ApplicationQ3SourceEntity> {
    state
        .entities
        .iter()
        .map(|row| ApplicationQ3SourceEntity {
            state: transport_entity(&row.state),
            linked: row.linked,
            server_flags: row.server_flags,
            single_client: row.single_client,
        })
        .collect()
}

/// Copy sixteen network slots into presentation slots.
fn retail_slots(slots: &Q3PlayerSlots) -> PlayerStateSlots {
    let mut values = Vec::with_capacity(16);
    for index in 0..16 {
        values.push(slots.get(index).expect("player slot index is in range"));
    }
    PlayerStateSlots::new(16, Some(values), None, None)
}

/// Convert a network player state to the presentation shape.
fn retail_player(product: Product, source: &NetPlayerState) -> PlayerState {
    let mut state = PlayerState::new(product, None);
    state.set_origin(to_vec3(source.origin));
    state.set_velocity(to_vec3(source.velocity));
    state.command_time = source.command_time;
    state.pm_type = source.pm_type;
    state.bob_cycle = source.bob_cycle;
    state.pm_flags = source.pm_flags;
    state.pm_time = source.pm_time;
    state.weapon_time = source.weapon_time;
    state.gravity = source.gravity;
    state.speed = source.speed;
    state.delta_angles = to_vec3(source.delta_angles);
    state.ground_entity_num = source.ground_entity_num;
    state.legs_timer = source.legs_timer;
    state.legs_anim = source.legs_anim;
    state.torso_timer = source.torso_timer;
    state.torso_anim = source.torso_anim;
    state.movement_dir = source.movement_dir;
    state.grapple_point = to_vec3(source.grapple_point);
    state.e_flags = source.e_flags;
    state.event_sequence = source.event_sequence;
    state.events = PlayerStateSlots::new(2, Some(vec![source.events[0], source.events[1]]), None, None);
    state.event_parms = PlayerStateSlots::new(2, Some(vec![source.event_parms[0], source.event_parms[1]]), None, None);
    state.external_event = source.external_event;
    state.external_event_parm = source.external_event_parm;
    state.external_event_time = source.external_event_time;
    state.client_num = source.client_num;
    state.weapon = source.weapon;
    state.weapon_state = source.weapon_state;
    state.viewangles = to_vec3(source.viewangles);
    state.viewheight = source.viewheight;
    state.damage_event = source.damage_event;
    state.damage_yaw = source.damage_yaw;
    state.damage_pitch = source.damage_pitch;
    state.damage_count = source.damage_count;
    state.stats = retail_slots(&source.stats);
    state.persistant = retail_slots(&source.persistant);
    state.powerups = retail_slots(&source.powerups);
    state.ammo = retail_slots(&source.ammo);
    state.generic1 = source.generic1;
    state.loop_sound = source.loop_sound;
    state.jumppad_ent = source.jumppad_ent;
    state.ping = source.ping;
    state.pmove_framecount = source.pmove_framecount;
    state.jumppad_frame = source.jumppad_frame;
    state.entity_event_sequence = source.entity_event_sequence;
    state
}

/// Convert a local network snapshot to the presentation shape.
fn retail_snapshot(product: Product, source: &NetSnapshot) -> PresentSnapshot {
    let mut area_mask = [0u8; 32];
    let length = source.area_mask.len().min(32);
    area_mask[..length].copy_from_slice(&source.area_mask[..length]);
    PresentSnapshot {
        message_number: source.message_number,
        server_time: source.server_time,
        delta_number: source.delta_number,
        flags: source.flags,
        server_command_number: source.server_command_number,
        parse_entities_number: source.parse_entities_number,
        area_mask,
        player_state: retail_player(product, &source.player_state),
        entities: source.entities.iter().map(retail_entity).collect(),
    }
}

/// Default prediction-command adapter (donor inline mapping; mirrors the
/// `ApplicationQ3Source` fallback because the round consult stays dynamic).
fn default_prediction_command(command: &ActorCommand) -> PredictionCommand {
    match &command.command {
        NetUserCommand::Q3 {
            server_time_milliseconds,
            angle_words,
            buttons,
            weapon,
            forward_move,
            right_move,
            up_move,
        } => PredictionCommand {
            server_time: *server_time_milliseconds as i32,
            angles: Vec3 {
                x: angle_words[0] as f32,
                y: angle_words[1] as f32,
                z: angle_words[2] as f32,
            },
            buttons: *buttons as i32,
            weapon: *weapon as i32,
            forwardmove: *forward_move as i32,
            rightmove: *right_move as i32,
            upmove: *up_move as i32,
        },
        _ => panic!("Foreign movement requires prediction command binding"),
    }
}

impl ApplicationQ3Client {
    /// Shared cvar owner (donor `cvars` getter).
    pub fn cvars(&self) -> SharedCvars {
        self.cvars.clone()
    }

    /// Shared time cvars, if any (donor `timeCvars` getter).
    pub fn time_cvars(&self) -> Option<SharedCvars> {
        self.time_cvars.clone()
    }

    /// Adopt a replacement cvar owner (donor `adoptCvars`).
    ///
    /// Contents swap in place so the session, mirror, and backends keep
    /// observing one object; the identity check folds to the dialect.
    pub fn adopt_cvars(&mut self, cvars: SharedCvars) -> Result<(), Q3ClientError> {
        if cvars.borrow().dialect() != self.cvars.borrow().dialect() {
            return Err(Q3ClientError::Message(
                "Q3 client cvar owner changed identity".to_string(),
            ));
        }
        if !Rc::ptr_eq(&cvars, &self.cvars) {
            std::mem::swap(&mut *self.cvars.borrow_mut(), &mut *cvars.borrow_mut());
        }
        self.bind_frame_time()
    }

    /// Registered cgame command names (donor `commandNames`).
    pub fn command_names(&self) -> Vec<String> {
        self.shared.borrow().command_names.iter().cloned().collect()
    }

    /// Shared primary bodies captured this frame (donor `sharedBodies`).
    pub fn shared_bodies(&self) -> Vec<Q3PrimaryBody> {
        self.shared.borrow().primary_bodies.clone()
    }

    /// Round or client server settings (donor `serverSettings`).
    fn server_settings(&self) -> Vec<CvarSnapshot> {
        if let Some(round) = self.round.borrow().as_ref() {
            if let Some(server) = round.server_settings.as_ref() {
                return server();
            }
            return Vec::new();
        }
        if let Some(server) = self.server_settings_fn.as_ref() {
            return server();
        }
        Vec::new()
    }

    /// Run the current-world assertion, if any.
    fn assert_current(&self) {
        if let Some(assert) = self.assert_current.as_ref() {
            assert();
        }
    }

    /// Command context for the seat (donor `commandContext`).
    fn command_context(&self) -> CommandContext {
        CommandContext::new(
            self.local.session.clone(),
            CommandOrigin::LocalSeat {
                seat: self.local.seat.clone(),
                client: self.local.client.clone(),
            },
        )
    }

    /// Reject work while closed or uninitialized (donor `requireBackend`).
    fn require_backend(&self) -> Result<(), Q3ClientError> {
        if self.shared.borrow().closed || self.backend.is_none() {
            return Err(Q3ClientError::Message(
                "Q3 cgame seat is closed or uninitialized".to_string(),
            ));
        }
        Ok(())
    }

    /// Mutable backend or the closed/uninitialized error.
    fn backend_mut(&mut self) -> Result<&mut Q3ClientBackend, Q3ClientError> {
        if self.shared.borrow().closed {
            return Err(Q3ClientError::Message(
                "Q3 cgame seat is closed or uninitialized".to_string(),
            ));
        }
        self.backend
            .as_mut()
            .ok_or_else(|| Q3ClientError::Message("Q3 cgame seat is closed or uninitialized".to_string()))
    }

    /// Shared backend or the closed/uninitialized error.
    fn backend_ref(&self) -> Result<&Q3ClientBackend, Q3ClientError> {
        if self.shared.borrow().closed {
            return Err(Q3ClientError::Message(
                "Q3 cgame seat is closed or uninitialized".to_string(),
            ));
        }
        self.backend
            .as_ref()
            .ok_or_else(|| Q3ClientError::Message("Q3 cgame seat is closed or uninitialized".to_string()))
    }

    /// Presentation time in milliseconds.
    fn source_time(&self) -> i32 {
        match &*self.source.borrow() {
            Q3ClientSource::Local(source) => source.time,
            Q3ClientSource::Remote(surface) => surface.borrow().time(),
        }
    }

    /// Snapshot source mode.
    fn source_mode(&self) -> Q3SourceMode {
        match &*self.source.borrow() {
            Q3ClientSource::Local(_) => Q3SourceMode::Live,
            Q3ClientSource::Remote(surface) => surface.borrow().source_mode(),
        }
    }

    /// Resolve the actor behind an entity number (donor `source.actorAt`).
    fn actor_at(source: &Rc<RefCell<Q3ClientSource>>, number: i32) -> Result<ActorId, Q3ClientError> {
        match &mut *source.borrow_mut() {
            Q3ClientSource::Local(local) => Ok(local.actor_at(number)?),
            Q3ClientSource::Remote(surface) => Ok(surface.borrow_mut().actor_at(number)?),
        }
    }

    /// Whether an entity number is body-hidden (donor `bodyHidden`).
    fn body_hidden(shared: &Rc<RefCell<Q3ClientShared>>, source: &Rc<RefCell<Q3ClientSource>>, number: i32) -> bool {
        if shared.borrow().hidden_bodies.is_empty() {
            return false;
        }
        let actor =
            Self::actor_at(source, number).unwrap_or_else(|error| panic!("Q3 body actor lookup failed: {error}"));
        shared.borrow().hidden_bodies.get(&actor.slot()) == Some(&actor)
    }

    /// Whether an entity number is body-selected (donor `bodySelected`).
    fn body_selected(shared: &Rc<RefCell<Q3ClientShared>>, source: &Rc<RefCell<Q3ClientSource>>, number: i32) -> bool {
        if shared.borrow().selected_bodies.is_empty() || Self::body_hidden(shared, source, number) {
            return false;
        }
        let actor =
            Self::actor_at(source, number).unwrap_or_else(|error| panic!("Q3 body actor lookup failed: {error}"));
        shared.borrow().selected_bodies.get(&actor.slot()) == Some(&actor)
    }

    /// Capture a primary body (donor `submitBody`).
    fn submit_body(
        shared: &Rc<RefCell<Q3ClientShared>>,
        media: &Rc<dyn Q3ClientMedia>,
        source: &Rc<RefCell<Q3ClientSource>>,
        number: i32,
        part: QvmBodyPart,
        entity: RefModelEntity,
        base: bool,
    ) -> bool {
        if !Self::body_selected(shared, source, number) {
            return false;
        }
        let copied = copy_ref_entity(&RefEntity::Model(entity));
        let RefEntity::Model(model) = copied else {
            panic!("Source body changed reference kind");
        };
        let actor =
            Self::actor_at(source, number).unwrap_or_else(|error| panic!("Q3 body actor lookup failed: {error}"));
        let Some(prepared) = prepare_q3_model(&model, Some(actor.clone()), number.max(0) as usize) else {
            return false;
        };
        let content = media
            .model_provider(&model.model)
            .and_then(|slot| media.provider_content(slot))
            .unwrap_or_else(|| panic!("Source body lost its registered geometry owner"));
        let time_ms = match &*source.borrow() {
            Q3ClientSource::Local(local) => local.time,
            Q3ClientSource::Remote(surface) => surface.borrow().time(),
        };
        shared.borrow_mut().primary_bodies.push(Q3PrimaryBody {
            actor,
            part,
            content,
            entity: prepared.entity,
            base,
            options: prepared.options,
            shader_tex_coord: model.shading.shader_tex_coord,
            shader_content: media.content(),
            time_ms,
        });
        true
    }

    /// Record a body pose capture (donor `bodyPose` callback).
    fn record_body_pose(
        shared: &Rc<RefCell<Q3ClientShared>>,
        source: &Rc<RefCell<Q3ClientSource>>,
        capture: Q3BodyPoseCapture,
    ) {
        if shared.borrow().pose_actors.is_empty() {
            return;
        }
        let actor = Self::actor_at(source, capture.number)
            .unwrap_or_else(|error| panic!("Q3 body actor lookup failed: {error}"));
        if shared.borrow().pose_actors.get(&actor.slot()) == Some(&actor) {
            shared
                .borrow_mut()
                .body_poses
                .insert(actor, (capture.origin, capture.angles));
        }
    }

    /// Sample a point light (donor `light`).
    ///
    /// The fallback sample drops the donor camera/time/target input: the
    /// ported sampler takes dynamic-light input instead, and the donor call
    /// carries no dynamic lights.
    fn sample_light(sampler: &ModelLightSampler, point: Vec3) -> Q3LightSample {
        let scales = LightingScales {
            ambient_scale: 1.0,
            directed_scale: 1.0,
        };
        if let Some(grid) = sampler.grid.as_ref() {
            match light_for_point(Some(grid), &point, &scales) {
                Ok(Some(sample)) => {
                    return Q3LightSample {
                        ambient: sample.ambient_light,
                        directed: sample.directed_light,
                        direction: sample.light_dir,
                    };
                }
                Ok(None) => {}
                Err(error) => panic!("Q3 light grid read failed: {error}"),
            }
        }
        let sample = sampler
            .sample(point, &ModelLightViewInput::default(), false)
            .expect("Q3 light fallback sample failed");
        Q3LightSample {
            ambient: Vec3 {
                x: sample.color.x * 255.0,
                y: sample.color.y * 255.0,
                z: sample.color.z * 255.0,
            },
            directed: vec3(0.0, 0.0, 0.0),
            direction: Vec3 { x: 0.0, y: 0.0, z: 1.0 },
        }
    }
}

impl ApplicationQ3Client {
    /// Build services, the session, and the backend (donor `initialize`, sync fold).
    fn initialize(
        &mut self,
        options: &ApplicationQ3ClientOptions,
        artifacts: Option<QvmPresentationArtifacts>,
    ) -> Result<(), Q3ClientError> {
        let shared = self.shared.clone();
        let source = self.source.clone();
        let seat = self.local.seat.clone();
        let scene_shared = shared.clone();
        let command_shared = shared.clone();
        let text_shared = shared.clone();
        let audio_shared = shared.clone();
        let listener_shared = shared.clone();
        let output = Q3ServiceOutput {
            scene: Rc::new(move |scene| {
                if (scene.source.render_flags & RDF_NOWORLDMODEL) == 0 {
                    scene_shared.borrow_mut().camera = scene.camera;
                }
                scene_shared
                    .borrow_mut()
                    .submissions
                    .push(Q3Submission::Scene(Box::new(scene)));
            }),
            command: Rc::new(move |command| {
                command_shared
                    .borrow_mut()
                    .submissions
                    .push(Q3Submission::Command(command));
            }),
            text: Rc::new(move |draw| {
                text_shared
                    .borrow_mut()
                    .submissions
                    .push(Q3Submission::Text(Box::new(Q3TextDraw {
                        rect: draw.rect,
                        uv: draw.uv,
                        color: draw.color,
                        compiled: draw.compiled.clone(),
                    })));
            }),
            audio: Rc::new(move |operation| {
                audio_shared.borrow_mut().audio_operations.push(operation);
            }),
            listener: Rc::new(move |origin, axis| {
                let mut guard = listener_shared.borrow_mut();
                guard.camera.origin = origin;
                guard.camera.axis = axis;
            }),
        };
        let actor_source = source.clone();
        let viewport_shared = shared.clone();
        let frame_shared = shared.clone();
        let remap_shader: Option<Q3ServiceRemap> = options.common.remap_shader.clone().map(|host| {
            let assert = self.assert_current.clone();
            let state = shared.clone();
            Rc::new(move |original: &str, replacement: &str, offset: &str| {
                if let Some(assert) = assert.as_ref() {
                    assert();
                }
                let initializing = state.borrow().initializing;
                let current = state.clone();
                let result = host(original, replacement, offset, initializing, &|| {
                    !current.borrow().closed
                });
                if let Some(assert) = assert.as_ref() {
                    assert();
                }
                result
            }) as Q3ServiceRemap
        });
        let services = (options.common.create_services)(Q3ServicesParams {
            output,
            actor_at: Rc::new(move |number| {
                Self::actor_at(&actor_source, number)
                    .unwrap_or_else(|error| panic!("Q3 service actor lookup failed: {error}"))
            }),
            now: self.now_fn.clone(),
            frame_number: Rc::new(move || frame_shared.borrow().frame_number),
            viewport: Rc::new(move || viewport_shared.borrow().viewport),
            seat: seat.clone(),
            cvars: self.cvars.clone(),
            time_cvars: self.time_cvars.clone(),
            remap_shader,
        })
        .map_err(Q3ClientError::Factory)?;
        self.services = Some(services);
        let (client_number, server_message_sequence, last_executed_server_command) = match &*source.borrow() {
            Q3ClientSource::Local(local) => (local.client_number, 0, 0),
            Q3ClientSource::Remote(surface) => {
                let surface = surface.borrow();
                (
                    surface.client_number(),
                    surface.server_message_sequence(),
                    surface.last_executed_server_command(),
                )
            }
        };
        let (snapshots, commands): (SharedSnapshots, SharedCommands) = match &options.kind {
            Q3ClientKindOptions::Local(_) => {
                let views = LocalSourceViews {
                    source: source.clone(),
                    product: self.product,
                };
                (Rc::new(RefCell::new(views.clone())), Rc::new(RefCell::new(views)))
            }
            Q3ClientKindOptions::Remote(remote) => (remote.snapshots.clone(), remote.commands.clone()),
            Q3ClientKindOptions::Qvm(qvm) => (qvm.snapshots.clone(), qvm.commands.clone()),
        };
        let game_source = source.clone();
        let command_source = source.clone();
        let ping_source = source.clone();
        let time_source = source.clone();
        let assert_session = self.assert_current.clone();
        let closed_session = shared.clone();
        let status_session = shared.clone();
        let selection_session = shared.clone();
        let names_session = shared.clone();
        let registration = self.registration.clone();
        let session = Q3ClientSession {
            product: self.product,
            client_number,
            server_message_sequence,
            last_executed_server_command,
            snapshots,
            commands,
            cvars: self.cvars.clone(),
            status_visible: Rc::new(move || status_session.borrow().status_visible),
            game_state: Rc::new(move || match &*game_source.borrow() {
                Q3ClientSource::Local(local) => local.game_state(),
                Q3ClientSource::Remote(surface) => surface.borrow().game_state(),
            }),
            server_command: Rc::new(move |sequence| match &mut *command_source.borrow_mut() {
                Q3ClientSource::Local(local) => local
                    .get_server_command(sequence)
                    .unwrap_or_else(|error| panic!("Q3 server command read failed: {error}")),
                Q3ClientSource::Remote(surface) => surface
                    .borrow_mut()
                    .server_command(sequence)
                    .unwrap_or_else(|error| panic!("Q3 server command read failed: {error}")),
            }),
            snapshot_ping: Rc::new(move |number| match &*ping_source.borrow() {
                Q3ClientSource::Local(_) => 0,
                Q3ClientSource::Remote(surface) => surface.borrow().snapshot_ping(number).unwrap_or(0),
            }),
            server_time: Rc::new(move || match &*time_source.borrow() {
                Q3ClientSource::Local(local) => local.time,
                Q3ClientSource::Remote(surface) => surface.borrow().time(),
            }),
            add_reliable_command: self.commands.reliable.clone(),
            append_console_command: self.commands.console.clone(),
            register_cgame_command: Rc::new(move |name: &str| {
                names_session.borrow_mut().command_names.insert(name.to_string());
                registration.register(name);
            }),
            set_user_command_value: Rc::new(move |weapon, sensitivity| {
                selection_session.borrow_mut().selection = (weapon, sensitivity);
            }),
            assert_current: Rc::new(move || {
                if let Some(assert) = assert_session.as_ref() {
                    assert();
                }
                if closed_session.borrow().closed {
                    panic!("Q3 cgame belongs to a retired world");
                }
            }),
            print: self.commands.print.clone(),
        };
        if let Q3ClientKindOptions::Qvm(_) = &options.kind {
            let key_get = shared.clone();
            let key_set = shared.clone();
            let weapon_shared = shared.clone();
            let held_shared = shared.clone();
            let held_source = source.clone();
            let capture_shared = shared.clone();
            let selected_shared = shared.clone();
            let selected_source = source.clone();
            let submit_shared = shared.clone();
            let submit_media = self.media.clone();
            let submit_source = source.clone();
            let overrides_shared = shared.clone();
            let hidden_shared = shared.clone();
            let hidden_source = source.clone();
            let remove_shared = shared.clone();
            let remove_registration = self.registration.clone();
            let sampler = self.sampler.clone();
            let game = (options.common.create_qvm)(Q3QvmParams {
                session,
                artifacts,
                command_context: self.command_context(),
                commands: self.command_buffer.clone(),
                key_catcher: Q3KeyCatcher {
                    get: Rc::new(move || key_get.borrow().key_catcher),
                    set: Rc::new(move |value| key_set.borrow_mut().key_catcher = value),
                },
                view_weapon_visible: Rc::new(move || weapon_shared.borrow().view_weapon_visible),
                held_weapon_actor: Rc::new(move |number| {
                    let actor = Self::actor_at(&held_source, number)
                        .unwrap_or_else(|error| panic!("Q3 held-weapon actor lookup failed: {error}"));
                    if Self::body_hidden(&held_shared, &held_source, number) {
                        return None;
                    }
                    if held_shared.borrow().selected_held_actors.get(&actor.slot()) == Some(&actor) {
                        Some(actor)
                    } else {
                        None
                    }
                }),
                body_capture_active: Rc::new(move || !capture_shared.borrow().selected_bodies.is_empty()),
                body_selected: Rc::new(move |number| Self::body_selected(&selected_shared, &selected_source, number)),
                submit_body: Rc::new(move |number, part, entity, base| {
                    Self::submit_body(
                        &submit_shared,
                        &submit_media,
                        &submit_source,
                        number,
                        part,
                        entity,
                        base,
                    )
                }),
                body_overrides_active: Rc::new(move || !overrides_shared.borrow().hidden_bodies.is_empty()),
                body_hidden: Rc::new(move |number| Self::body_hidden(&hidden_shared, &hidden_source, number)),
                remove_command: Rc::new(move |name: &str| {
                    remove_shared.borrow_mut().command_names.remove(name);
                    remove_registration.remove(name);
                }),
                light_for_point: Rc::new(move |point| Self::sample_light(&sampler, point)),
            })
            .map_err(Q3ClientError::Factory)?;
            self.backend = Some(Q3ClientBackend::Qvm(game));
            self.shared.borrow_mut().submissions.clear();
            return Ok(());
        }
        let movement: Rc<RefCell<dyn PresentationMovementHost>> = match &options.kind {
            Q3ClientKindOptions::Local(_) => Rc::new(RefCell::new(RoundMovement {
                round: self.round.clone(),
            })),
            Q3ClientKindOptions::Remote(remote) => remote.movement.clone(),
            Q3ClientKindOptions::Qvm(_) => {
                unreachable!("QVM clients return before native backend creation")
            }
        };
        let renderer_name = options
            .common
            .renderer
            .driver
            .as_ref()
            .map_or("", |driver| driver.renderer.as_str());
        let hardware_ragepro = q3_hardware(renderer_name) == Q3Hardware::RagePro;
        let memory = self.memory_remaining.clone();
        let body_shared = shared.clone();
        let body_media = self.media.clone();
        let body_source = source.clone();
        let pose_shared = shared.clone();
        let pose_source = source.clone();
        let gate_hidden = shared.clone();
        let gate_hidden_source = source.clone();
        let gate_weapon = shared.clone();
        let gate_supplemental = shared.clone();
        let gate_events = shared.clone();
        let selection = shared.clone();
        let sampler = self.sampler.clone();
        let game = (options.common.create_native)(Q3NativeParams {
            session,
            movement,
            weapon_hud: options.common.weapon_hud.clone(),
            hardware_ragepro,
            submit_body: Rc::new(move |number, part, entity, base| {
                Self::submit_body(&body_shared, &body_media, &body_source, number, part, entity, base)
            }),
            record_body_pose: Rc::new(move |capture: Q3BodyPoseCapture| {
                Self::record_body_pose(&pose_shared, &pose_source, capture);
            }),
            gates: Q3RenderGates {
                body_hidden: Rc::new(move |number| Self::body_hidden(&gate_hidden, &gate_hidden_source, number)),
                view_weapon: Rc::new(move || gate_weapon.borrow().view_weapon_visible),
                supplemental_view_weapon: Rc::new(move || gate_supplemental.borrow().supplemental_view_weapon),
                note_event: Rc::new(move || gate_events.borrow_mut().event_count += 1),
            },
            weapon_selection: Rc::new(move |weapon| {
                selection.borrow_mut().weapon_selection = Some(weapon);
            }),
            light_for_point: Rc::new(move |point| Self::sample_light(&sampler, point)),
            memory_remaining: Rc::new(move || (*memory)().min(0x7FFF_FFFF)),
        })
        .map_err(Q3ClientError::Factory)?;
        self.backend = Some(Q3ClientBackend::Native(game));
        self.shared.borrow_mut().submissions.clear();
        Ok(())
    }

    /// Whether a round restart can proceed (donor restart guard).
    fn native_local_ready(&self) -> bool {
        !self.shared.borrow().closed
            && matches!(&*self.source.borrow(), Q3ClientSource::Local(_))
            && self.round.borrow().is_some()
            && matches!(self.backend, Some(Q3ClientBackend::Native(_)))
    }

    /// Reject round restarts without an active native local cgame (donor `assertCanRestartRound`).
    pub fn assert_can_restart_round(&self) -> Result<(), Q3ClientError> {
        self.assert_current();
        if !self.native_local_ready() {
            return Err(Q3ClientError::Message(
                "Q3 fast restart requires an active native local cgame".to_string(),
            ));
        }
        match &*self.source.borrow() {
            Q3ClientSource::Local(source) => Ok(source.assert_can_restart_round()?),
            Q3ClientSource::Remote(_) => Err(Q3ClientError::Message(
                "Q3 fast restart requires an active native local cgame".to_string(),
            )),
        }
    }

    /// Begin a round restart (donor `beginRoundRestart`).
    pub fn begin_round_restart(&mut self, bit: i32, source: &Q3SourcePresentationState) -> Result<(), Q3ClientError> {
        self.assert_can_restart_round()?;
        match &mut *self.source.borrow_mut() {
            Q3ClientSource::Local(local) => local.begin_round_restart(bit, source)?,
            Q3ClientSource::Remote(_) => {
                return Err(Q3ClientError::Message("Missing native local source".to_string()));
            }
        }
        let mut shared = self.shared.borrow_mut();
        shared.submissions.clear();
        shared.audio_operations.clear();
        Ok(())
    }

    /// Rebind the round after a restart (donor `rebindRound`).
    pub fn rebind_round(&mut self, binding: ApplicationQ3LocalRound) -> Result<(), Q3ClientError> {
        self.assert_current();
        if !self.native_local_ready() {
            return Err(Q3ClientError::Message(
                "Q3 fast restart requires an active native local cgame".to_string(),
            ));
        }
        match &mut *self.source.borrow_mut() {
            Q3ClientSource::Local(local) => {
                local.validate_round_actor(&binding.actor, &binding.initial)?;
            }
            Q3ClientSource::Remote(_) => {
                return Err(Q3ClientError::Message(
                    "Q3 fast restart requires an active native local cgame".to_string(),
                ));
            }
        }
        let current_timing = self
            .round
            .borrow()
            .as_ref()
            .map(|round| round.movement.borrow().command_timing());
        if Some(binding.movement.borrow().command_timing()) != current_timing {
            return Err(Q3ClientError::Message("Q3 restart changed movement timing".to_string()));
        }
        match &mut *self.source.borrow_mut() {
            Q3ClientSource::Local(local) => {
                local.rebind_round(binding.actor.clone(), &binding.initial)?;
            }
            Q3ClientSource::Remote(_) => {
                return Err(Q3ClientError::Message("Missing native local source".to_string()));
            }
        }
        *self.round.borrow_mut() = Some(binding);
        let mut shared = self.shared.borrow_mut();
        shared.body_poses.clear();
        shared.pose_actors.clear();
        shared.weapon_selection = None;
        Ok(())
    }

    /// Refresh cvars from the remote system info (donor `refreshSystemInfo`).
    ///
    /// Local sources expose no system info; the time-mirror refresh falls
    /// back to the frame-time sync when no mirror is bound.
    pub fn refresh_system_info(&mut self) -> Result<(), Q3ClientError> {
        if self.shared.borrow().closed {
            return Err(Q3ClientError::Message("Q3 presentation is closed".to_string()));
        }
        self.assert_current();
        let info = match &*self.source.borrow() {
            Q3ClientSource::Local(_) => None,
            Q3ClientSource::Remote(surface) => surface.borrow().system_info(),
        };
        if let Some(info) = info {
            let skip: HashSet<String> = match self.time_cvars.as_ref() {
                Some(time) => frame_time_cvar_names(time.borrow().dialect())
                    .iter()
                    .map(ToString::to_string)
                    .collect(),
                None => HashSet::new(),
            };
            let fields: Vec<&str> = info.split('\\').collect();
            let mut index = if fields.first() == Some(&"") { 1 } else { 0 };
            while index + 1 < fields.len() {
                let name = fields[index];
                let value = fields[index + 1];
                index += 2;
                if skip.contains(&name.to_lowercase()) {
                    continue;
                }
                if name.to_lowercase() == "timescale" && Some(&info) == self.applied_time_system_info.as_ref() {
                    continue;
                }
                if !name.is_empty() && name.to_lowercase() != "cl_allowdownload" {
                    self.cvars.borrow_mut().set(name, value, true)?;
                }
            }
            self.applied_time_system_info = Some(info);
        }
        if let Some(mirror) = self.time_mirror.as_ref() {
            mirror.pump()?;
            mirror.refresh()?;
        } else if let Some(time) = self.time_cvars.as_ref() {
            if !Rc::ptr_eq(time, &self.cvars) {
                refresh_frame_time_cvars(&time.borrow(), &mut self.cvars.borrow_mut())?;
            }
        }
        Ok(())
    }

    /// Bind the shared time-cvar mirror (donor `bindFrameTime`).
    ///
    /// Registries without a session adopt the seat session (or the peer
    /// session when one side already has one): the canonical mirror requires
    /// distinct registries in the same session, while this port otherwise
    /// folds session identity away.
    fn bind_frame_time(&mut self) -> Result<(), Q3ClientError> {
        if let Some(mut mirror) = self.time_mirror.take() {
            mirror.close();
        }
        let Some(time) = self.time_cvars.clone() else {
            return Ok(());
        };
        if Rc::ptr_eq(&time, &self.cvars) {
            return Ok(());
        }
        self.refresh_system_info()?;
        {
            let seat = self.local.session.clone();
            let mut cvars = self.cvars.borrow_mut();
            let mut owner = time.borrow_mut();
            match (cvars.session().cloned(), owner.session().cloned()) {
                (None, None) => {
                    cvars.set_session(seat.clone());
                    owner.set_session(seat);
                }
                (None, Some(peer)) => cvars.set_session(peer),
                (Some(peer), None) => owner.set_session(peer),
                (Some(_), Some(_)) => {}
            }
        }
        let names: Vec<String> = frame_time_cvar_names(time.borrow().dialect())
            .iter()
            .map(ToString::to_string)
            .chain(
                COLLISION_MAP_CVAR_DEFINITIONS
                    .iter()
                    .map(|(name, _, _)| name.to_string()),
            )
            .filter(|name| time.borrow().get(name).is_some())
            .collect();
        let closed = self.shared.clone();
        let assert = self.assert_current.clone();
        let assert_current: Rc<dyn Fn()> = Rc::new(move || {
            if closed.borrow().closed {
                panic!("Q3 presentation is closed");
            }
            if let Some(assert) = assert.as_ref() {
                assert();
            }
        });
        let mirror = SharedCvarMirror::attach(time, self.cvars.clone(), &names, assert_current)?;
        self.time_mirror = Some(mirror);
        Ok(())
    }

    /// Capture a single-shot video reopen (donor `captureVideoReopen`, sync fold).
    pub fn capture_video_reopen(&self) -> Result<QvmVideoReopen, Q3ClientError> {
        let Some(options) = self.reopen_options.clone() else {
            return Err(Q3ClientError::Message(
                "Video guest reopen requires QVM presentation".to_string(),
            ));
        };
        let artifacts = match self.backend.as_ref() {
            Some(Q3ClientBackend::Qvm(game)) => game.presentation_artifacts()?,
            _ => {
                return Err(Q3ClientError::Message(
                    "Video guest reopen requires QVM presentation".to_string(),
                ));
            }
        };
        let cvars = self.cvars.clone();
        let time_cvars = self.time_cvars.clone();
        let shared = self.shared.clone();
        let mut opened = false;
        Ok(Box::new(move |overrides| {
            if !shared.borrow().closed {
                return Err(Q3ClientError::Message(
                    "Previous guest presentation must shut down before reopening".to_string(),
                ));
            }
            if opened {
                return Err(Q3ClientError::Message(
                    "Video guest reopen was already attempted".to_string(),
                ));
            }
            opened = true;
            let mut common = options.common.clone();
            common.settings = Vec::new();
            common.server_settings = None;
            common.cvars = Some(cvars.clone());
            common.time_cvars = time_cvars.clone();
            common.renderer = overrides.renderer;
            common.viewport = overrides.viewport;
            common.command_registration = overrides.command_registration;
            ApplicationQ3Client::create(
                ApplicationQ3ClientOptions {
                    common,
                    kind: options.kind.clone(),
                },
                Some(artifacts.clone()),
            )
        }))
    }

    /// Registered resource diagnostics (donor `resourceDiagnostics`).
    pub fn resource_diagnostics(&self) -> Result<Q3ResourceDiagnostics, Q3ClientError> {
        if self.shared.borrow().closed {
            return Err(Q3ClientError::Message(
                "Q3 presentation services are closed or uninitialized".to_string(),
            ));
        }
        let Some(services) = self.services.as_ref() else {
            return Err(Q3ClientError::Message(
                "Q3 presentation services are closed or uninitialized".to_string(),
            ));
        };
        Ok(Q3ResourceDiagnostics {
            models: services.registered_models(),
            skins: services.registered_skins(),
        })
    }

    /// Native game backend (donor `cgame` getter).
    pub fn cgame(&mut self) -> Result<&mut dyn Q3NativeGame, Q3ClientError> {
        match self.backend_mut()? {
            Q3ClientBackend::Native(game) => Ok(&mut **game),
            Q3ClientBackend::Qvm(_) => Err(Q3ClientError::Message("This seat runs native guest cgame".to_string())),
        }
    }

    /// Take a pending weapon selection (donor `consumeWeaponSelection`).
    pub fn consume_weapon_selection(&mut self) -> Option<i32> {
        self.shared.borrow_mut().weapon_selection.take()
    }

    /// Current user-command selection (donor `userCommandSelection`).
    pub fn user_command_selection(&self) -> Q3UserCommandSelection {
        let shared = self.shared.borrow();
        let mut weapon = shared.selection.0;
        if let Some(equipment) = self.equipment_primary_weapon.as_ref() {
            if let Some(primary) = equipment() {
                weapon = primary;
            }
        }
        Q3UserCommandSelection {
            weapon,
            sensitivity: shared.selection.1,
        }
    }

    /// Presented event count (donor `presentedEvents`).
    pub fn presented_events(&self) -> u64 {
        self.shared.borrow().event_count
    }

    /// Shared QVM held weapons (donor `sharedHeldWeapons`).
    pub fn shared_held_weapons(&self) -> Result<Vec<QvmHeldWeapon>, Q3ClientError> {
        match self.backend_ref()? {
            Q3ClientBackend::Qvm(game) => Ok(game.shared_held_weapons()),
            Q3ClientBackend::Native(_) => Ok(Vec::new()),
        }
    }

    /// Shared equipment view visibility (donor `sharedEquipmentViewVisible`).
    pub fn shared_equipment_view_visible(&self) -> Result<Option<bool>, Q3ClientError> {
        match self.backend_ref()? {
            Q3ClientBackend::Qvm(game) => Ok(Some(game.shared_equipment_view_visible())),
            Q3ClientBackend::Native(_) => Ok(None),
        }
    }

    /// Weapon HUD visibility (donor `weaponHudView`).
    pub fn weapon_hud_view(&self) -> Result<Q3WeaponHudView, Q3ClientError> {
        match self.backend_ref()? {
            Q3ClientBackend::Qvm(game) => Ok(Q3WeaponHudView {
                visible: game.shared_equipment_hud(),
                aggregate_warning: false,
            }),
            Q3ClientBackend::Native(game) => {
                let hud = game.hud_state();
                let cvars = self.cvars.borrow();
                let player = hud.player.is_some_and(|player| {
                    player.health > 0
                        && player.movement_type != MoveType::PmIntermission
                        && player.team != Team::TeamSpectator
                });
                let visible = player
                    && !hud.level_shot
                    && !hud.show_scores
                    && cvars.get("cg_draw2D").map_or(1, |snapshot| snapshot.integer_value) != 0
                    && cvars.get("cg_drawStatus").map_or(1, |snapshot| snapshot.integer_value) != 0;
                Ok(Q3WeaponHudView {
                    visible,
                    aggregate_warning: cvars
                        .get("cg_drawAmmoWarning")
                        .map_or(1, |snapshot| snapshot.integer_value)
                        != 0,
                })
            }
        }
    }

    /// Latest camera (donor `camera`).
    pub fn camera(&self) -> SceneCamera {
        self.shared.borrow().camera
    }

    /// Captured body pose for an actor (donor `bodyPose`).
    pub fn body_pose(&self, actor: &ActorId) -> Option<(Vec3, Vec3)> {
        self.shared.borrow().body_poses.get(actor).copied()
    }

    /// Supplemental weapon camera (donor `supplementalWeaponCamera`).
    pub fn supplemental_weapon_camera(&self, camera: &SceneCamera) -> Result<SceneCamera, Q3ClientError> {
        match self.backend_ref()? {
            Q3ClientBackend::Qvm(_) => Ok(*camera),
            Q3ClientBackend::Native(_) => {
                let occupied: Vec<Rect> = self
                    .shared
                    .borrow()
                    .submissions
                    .iter()
                    .filter_map(|submission| match submission {
                        Q3Submission::Scene(scene) if (scene.source.render_flags & RDF_NOWORLDMODEL) != 0 => {
                            Some(scene.viewport)
                        }
                        _ => None,
                    })
                    .collect();
                Ok(weapon_view_camera(
                    &q3_weapon_camera(camera, self.split_screen),
                    &occupied,
                ))
            }
        }
    }

    /// Fold presentation events into the local source (donor `receiveEvents`).
    pub fn receive_events(&mut self, events: &[Q3SourcePresentationEvent]) -> Result<(), Q3ClientError> {
        self.assert_current();
        match self.backend_mut()? {
            Q3ClientBackend::Native(_) => {}
            Q3ClientBackend::Qvm(_) => {
                return Err(Q3ClientError::Message("This seat runs native guest cgame".to_string()));
            }
        }
        match &mut *self.source.borrow_mut() {
            Q3ClientSource::Local(local) => Ok(local.receive_events(events)?),
            Q3ClientSource::Remote(_) => Err(Q3ClientError::Message(
                "Remote Q3 cgame receives events through its network connection".to_string(),
            )),
        }
    }

    /// Fold a source state into the local source (donor `receive`).
    pub fn receive(
        &mut self,
        state: &Q3SourcePresentationState,
        events: &[Q3SourcePresentationEvent],
        commands: &[ActorCommand],
    ) -> Result<(), Q3ClientError> {
        match self.backend_mut()? {
            Q3ClientBackend::Native(_) => {}
            Q3ClientBackend::Qvm(_) => {
                return Err(Q3ClientError::Message("This seat runs native guest cgame".to_string()));
            }
        }
        match &mut *self.source.borrow_mut() {
            Q3ClientSource::Local(local) => Ok(local.receive(state, events, commands)?),
            Q3ClientSource::Remote(_) => Err(Q3ClientError::Message(
                "Remote Q3 cgame receives snapshots through its network connection".to_string(),
            )),
        }
    }
}

impl ApplicationQ3Client {
    /// Prepare a cgame frame (donor `prepare`, sync fold).
    ///
    /// All donor default arguments are explicit here.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare(
        &mut self,
        frame_number: u64,
        viewport: Rect,
        presentations: &[Q3ClientModel],
        status_visible: bool,
        bodies: &[ComponentBody],
        view_weapon_visible: bool,
        camera_controlled: bool,
    ) -> Result<(), Q3ClientError> {
        {
            let mut shared = self.shared.borrow_mut();
            shared.view_weapon_visible = view_weapon_visible;
            shared.hidden_bodies.clear();
            if camera_controlled {
                shared
                    .hidden_bodies
                    .insert(self.local.actor.slot(), self.local.actor.clone());
            }
            shared.selected_bodies.clear();
            shared.primary_bodies.clear();
            for body in bodies {
                shared.selected_bodies.insert(body.actor.slot(), body.actor.clone());
            }
            shared.selected_held_actors.clear();
            shared.body_poses.clear();
            shared.pose_actors.clear();
            for model in presentations {
                if !model.render_owner_source_client
                    && model.view_weapon
                    && (!model.path.is_empty() || model.held_weapon.is_some())
                {
                    shared
                        .selected_held_actors
                        .insert(model.actor.slot(), model.actor.clone());
                }
                if model.replaces_body {
                    shared.hidden_bodies.insert(model.actor.slot(), model.actor.clone());
                }
                if !model.render_owner_source_client && model.visible && !model.view_weapon {
                    shared.pose_actors.insert(model.actor.slot(), model.actor.clone());
                }
            }
        }
        self.refresh_system_info()?;
        self.require_backend()?;
        let supplemental = presentations.iter().any(|model| {
            !model.render_owner_source_client && model.visible && model.view_weapon && model.actor == self.local.actor
        });
        {
            let mut shared = self.shared.borrow_mut();
            shared.frame_number = frame_number;
            shared.viewport = viewport;
            shared.submissions.clear();
            shared.supplemental_view_weapon = supplemental;
        }
        for setting in self.server_settings() {
            if self.shared_cvar_names.contains(&setting.name.to_lowercase()) {
                self.cvars.borrow_mut().set(&setting.name, &setting.value, true)?;
            }
        }
        if let Some(mirror) = self.time_mirror.as_ref() {
            mirror.pump()?;
        }
        let previous_status = std::mem::replace(&mut self.shared.borrow_mut().status_visible, status_visible);
        let time = self.source_time();
        let demo = self.source_mode() == Q3SourceMode::Demo;
        let is_qvm = matches!(self.backend, Some(Q3ClientBackend::Qvm(_)));
        let result = match self.backend_mut()? {
            Q3ClientBackend::Native(game) => game.draw_active_frame(Q3ActiveFrame {
                server_time_ms: time,
                demo_playback: demo,
                engine_frame_number: frame_number,
            }),
            Q3ClientBackend::Qvm(game) => game.draw(time, demo),
        };
        self.shared.borrow_mut().status_visible = previous_status;
        let failure = result.as_ref().err().map(ToString::to_string);
        if is_qvm {
            if let Some(Q3ClientBackend::Qvm(game)) = self.backend.as_mut() {
                if let Err(status_error) = game.refresh_status() {
                    if let Some(frame_error) = failure {
                        return Err(Q3ClientError::Message(format!(
                            "Cgame frame and status restoration failed: {frame_error}; {status_error}"
                        )));
                    }
                    return Err(status_error);
                }
            }
        }
        result?;
        let scenes: Vec<Q3PresentedSceneView> = self
            .shared
            .borrow()
            .submissions
            .iter()
            .filter_map(|submission| match submission {
                Q3Submission::Scene(scene) => Some(scene.as_ref().clone()),
                _ => None,
            })
            .collect();
        self.pipeline.borrow_mut().preload_scenes(&scenes);
        let operations = std::mem::take(&mut self.shared.borrow_mut().audio_operations);
        let content = self.media.content();
        self.audio.receive_cgame_frame(&content, &self.local.seat, operations);
        Ok(())
    }

    /// Execute a console command (donor `command`, sync fold).
    pub fn command(&mut self, argv: &[String]) -> Result<bool, Q3ClientError> {
        match self.backend_mut()? {
            Q3ClientBackend::Native(game) => Ok(game.console_execute(argv)?),
            Q3ClientBackend::Qvm(game) => Ok(game.command(argv)?),
        }
    }

    /// Whether a console command is handled (donor `handlesCommand`).
    pub fn handles_command(&self, name: &str) -> Result<bool, Q3ClientError> {
        match self.backend_ref()? {
            Q3ClientBackend::Native(game) => Ok(game.console_handles(name)),
            Q3ClientBackend::Qvm(_) => Ok(self.shared.borrow().command_names.contains(name)),
        }
    }

    /// Key event (donor `keyEvent`, sync fold).
    pub fn key_event(&mut self, key: i32, down: bool) -> Result<(), Q3ClientError> {
        match self.backend_mut()? {
            Q3ClientBackend::Native(game) => Ok(game.key_event(key, down)?),
            Q3ClientBackend::Qvm(game) => Ok(game.key_event(key, down)?),
        }
    }

    /// Mouse motion event (donor `mouseEvent`, sync fold).
    pub fn mouse_event(&mut self, x: f64, y: f64) -> Result<(), Q3ClientError> {
        match self.backend_mut()? {
            Q3ClientBackend::Native(game) => Ok(game.mouse_event(x, y)?),
            Q3ClientBackend::Qvm(game) => Ok(game.mouse_event(x, y)?),
        }
    }

    /// Set the event-handling mode (donor `eventHandling`, sync fold).
    pub fn event_handling(&mut self, mode: u8) -> Result<(), Q3ClientError> {
        if mode > 3 {
            return Err(Q3ClientError::Message("Invalid Q3 event handling mode".to_string()));
        }
        match self.backend_mut()? {
            Q3ClientBackend::Native(game) => Ok(game.event_handling(mode)?),
            Q3ClientBackend::Qvm(game) => {
                let mapped = match mode {
                    0 => Q3EventHandling::None,
                    1 => Q3EventHandling::TeamMenu,
                    2 => Q3EventHandling::Scoreboard,
                    _ => Q3EventHandling::EditHud,
                };
                Ok(game.event_handling(mapped)?)
            }
        }
    }

    /// Route a seat input event (donor `input`, sync fold).
    pub fn input(&mut self, event: &SeatInputEvent) -> Result<(), Q3ClientError> {
        self.require_backend()?;
        if event.seat != self.local.seat {
            return Err(Q3ClientError::Message("Q3 input belongs to another seat".to_string()));
        }
        match &event.kind {
            SeatInputEventKind::Key { code, down, .. } => self.key_event(*code, *down),
            SeatInputEventKind::Text { text } => {
                for character in text.chars() {
                    self.key_event(character as i32 | KEY_CHAR_FLAG, true)?;
                }
                Ok(())
            }
            SeatInputEventKind::MouseButton { button, down } => {
                self.key_event(KeyCode::Mouse1 as i32 + quake_mouse_button(*button) - 1, *down)
            }
            SeatInputEventKind::MouseMotion { delta, .. } => self.mouse_event(f64::from(delta.x), f64::from(delta.y)),
            SeatInputEventKind::MouseWheel { delta } => {
                let presses = delta.y.abs().ceil() as i32;
                let key = if delta.y > 0.0 {
                    KeyCode::MouseWheelUp as i32
                } else {
                    KeyCode::MouseWheelDown as i32
                };
                for _ in 0..presses {
                    self.key_event(key, true)?;
                    self.key_event(key, false)?;
                }
                Ok(())
            }
            SeatInputEventKind::ControllerButton { button, down, .. } => {
                if (0..32).contains(button) {
                    self.key_event(KeyCode::Joy1 as i32 + *button, *down)?;
                }
                Ok(())
            }
            SeatInputEventKind::ControllerAxis { .. } | SeatInputEventKind::Focus { .. } => Ok(()),
        }
    }

    /// Whether the client captures input (donor `capturesInput`).
    pub fn captures_input(&self) -> bool {
        match self.backend.as_ref() {
            Some(Q3ClientBackend::Qvm(game)) => game.captures_input(),
            _ => self.shared.borrow().key_catcher != 0,
        }
    }

    /// Find a portal child view (donor `portal`).
    ///
    /// Portal entities are stored pre-projected; the sort and offscreen
    /// tests match the donor.
    fn portal(
        &mut self,
        scene: &Q3PresentedSceneView,
        input: &Q3WorldView,
    ) -> Result<Option<Q3WorldView>, Q3ClientError> {
        if scene.portals.is_empty() || !matches!(input.camera.clip, CameraClip::None) {
            return Ok(None);
        }
        let visible = self.pipeline.borrow_mut().visible_surfaces(&input.camera, input);
        for index in visible {
            let Some(surface) = self.pipeline.borrow().portal_surface(index) else {
                continue;
            };
            if surface.shader_sort != 1 {
                continue;
            }
            let child = portal_camera(
                &surface.plane,
                &scene.portals,
                &input.camera,
                self.source_time() as f32,
                None,
            )
            .map_err(|error| Q3ClientError::Message(error.to_string()))?;
            let Some(child) = child else {
                continue;
            };
            if portal_surface_offscreen(&surface.geometry, &input.camera, surface.portal_range, child.mirror)
                .map_err(|error| Q3ClientError::Message(error.to_string()))?
            {
                continue;
            }
            return Ok(Some(Q3WorldView {
                camera: child.camera,
                pvs_origin: Some(child.pvs_origin),
                ..input.clone()
            }));
        }
        Ok(None)
    }

    /// Publish one world view with merged effects (donor `frame` `publish`).
    fn publish_world_scene(
        &mut self,
        scene: &Q3PresentedSceneView,
        view: &Q3WorldView,
        additional_effects: Option<&dyn Fn(&SceneCamera) -> ApplicationEffectFrame>,
        view_offset: Option<Vec3>,
    ) {
        let effects = additional_effects.map(|run| run(&view.camera));
        let mut combined = view.clone();
        if let Some(effects) = effects.as_ref() {
            combined.lights.clone_from(&effects.lights);
            combined
                .q3_lights
                .extend(effects.q3_lights.iter().map(|light| Q3SceneLight {
                    origin: light.origin,
                    radius: light.radius,
                    color: light.color,
                    additive: light.additive,
                }));
            combined.q3_lights.truncate(32);
        }
        let operations: &[SceneOperation] = effects.as_ref().map_or(&[], |effects| &effects.operations);
        let flags = Q3SceneFlags {
            no_world_model: view.no_world_model,
            split_screen: self.split_screen,
            supplemental_view_weapon: self.shared.borrow().supplemental_view_weapon,
        };
        self.pipeline
            .borrow_mut()
            .submit_world_scene(scene, &combined, flags, operations, view_offset);
    }

    /// Render the prepared submissions (donor `frame`).
    ///
    /// Effect callbacks take only the camera: entity-range reservation is
    /// pipeline-internal now.
    pub fn frame(
        &mut self,
        additional_effects: Option<&dyn Fn(&SceneCamera) -> ApplicationEffectFrame>,
        transform_camera: Option<&dyn Fn(&SceneCamera) -> SceneCamera>,
        environment: &Q3FrameEnvironment,
        view_offset: Option<Vec3>,
    ) -> Result<RenderFrame, Q3ClientError> {
        self.require_backend()?;
        self.pipeline.borrow_mut().begin_frame();
        let seat = self.local.seat.clone();
        let time_ms = self.source_time();
        let submissions = self.shared.borrow().submissions.clone();
        for submission in &submissions {
            match submission {
                Q3Submission::Command(command) => {
                    self.pipeline.borrow_mut().submit_command(command.clone());
                }
                Q3Submission::Text(draw) => {
                    let view = Q3TextView {
                        camera: self.shared.borrow().camera,
                        time_ms,
                        seat: seat.clone(),
                    };
                    let viewport = self.shared.borrow().viewport;
                    let borrowed = MaterialTextDraw {
                        rect: draw.rect,
                        uv: draw.uv,
                        color: draw.color,
                        compiled: &draw.compiled,
                    };
                    self.pipeline.borrow_mut().submit_text(&borrowed, viewport, &view);
                }
                Q3Submission::Scene(scene) => {
                    let direct = (scene.source.render_flags & RDF_NOWORLDMODEL) != 0;
                    let camera = if direct {
                        scene.camera
                    } else if let Some(transform) = transform_camera {
                        transform(&scene.camera)
                    } else {
                        scene.camera
                    };
                    let mut q3_lights = scene.lights.clone();
                    q3_lights.truncate(32);
                    let mut visible_areas = HashSet::new();
                    for (byte, mask) in scene.source.area_mask.iter().enumerate() {
                        for bit in 0..8 {
                            if (mask >> bit) & 1 == 0 {
                                visible_areas.insert((byte * 8 + bit) as i32);
                            }
                        }
                    }
                    let input = Q3WorldView {
                        camera,
                        time_ms,
                        seat: seat.clone(),
                        lights: Vec::new(),
                        q3_lights,
                        visible_areas,
                        render_text: scene.source.text.clone(),
                        clear: if direct {
                            Q3ViewClear {
                                depth: 1.0,
                                color: None,
                                stencil: false,
                            }
                        } else {
                            Q3ViewClear {
                                depth: 1.0,
                                color: Some(if environment.no_world_model {
                                    Vec3 { x: 0.3, y: 0.3, z: 0.3 }
                                } else {
                                    Vec3 { x: 0.0, y: 0.0, z: 0.0 }
                                }),
                                stencil: false,
                            }
                        },
                        q1_fog: None,
                        source_sky: None,
                        no_world_model: false,
                        pvs_origin: None,
                    };
                    if direct {
                        let flags = Q3SceneFlags {
                            no_world_model: true,
                            split_screen: self.split_screen,
                            supplemental_view_weapon: self.shared.borrow().supplemental_view_weapon,
                        };
                        self.pipeline.borrow_mut().submit_direct_scene(scene, &input, flags);
                    } else {
                        let world_input = Q3WorldView {
                            q1_fog: environment.q1_fog,
                            source_sky: environment.source_sky.clone(),
                            no_world_model: environment.no_world_model,
                            ..input
                        };
                        let child = if world_input.no_world_model {
                            None
                        } else {
                            self.portal(scene, &world_input)?
                        };
                        if let Some(child) = child {
                            self.publish_world_scene(scene, &child, additional_effects, view_offset);
                        }
                        self.publish_world_scene(scene, &world_input, additional_effects, view_offset);
                    }
                }
            }
        }
        Ok(self.pipeline.borrow_mut().finish_frame(false))
    }

    /// Shut down the guest and close (donor `shutdown`, sync fold).
    pub fn shutdown(&mut self) -> Result<(), Q3ClientError> {
        if let Some(Q3ClientBackend::Qvm(game)) = self.backend.as_mut() {
            game.shutdown()?;
        }
        self.close();
        Ok(())
    }

    /// Close the client (donor `close`).
    pub fn close(&mut self) {
        if self.shared.borrow().closed {
            return;
        }
        self.registration.close();
        if let Some(mut mirror) = self.time_mirror.take() {
            mirror.close();
        }
        if let Some(backend) = self.backend.take() {
            match backend {
                Q3ClientBackend::Native(mut game) => game.close(),
                Q3ClientBackend::Qvm(mut game) => game.close(),
            }
        }
        self.shared
            .borrow_mut()
            .audio_operations
            .push(Q3SeatAudioOperation::ClearLoops { kill_all: true });
        let operations = std::mem::take(&mut self.shared.borrow_mut().audio_operations);
        let content = self.media.content();
        self.audio.receive_cgame_frame(&content, &self.local.seat, operations);
        self.shared.borrow_mut().closed = true;
        if let Some(mut services) = self.services.take() {
            services.close_cinematics();
        }
        self.media.close();
        {
            let mut shared = self.shared.borrow_mut();
            shared.body_poses.clear();
            shared.pose_actors.clear();
            shared.selected_bodies.clear();
            shared.primary_bodies.clear();
            shared.submissions.clear();
        }
        self.pipeline.borrow_mut().close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bootstrap::q3_client::qvm_display::QvmDisplayDriver;
    use crate::bootstrap::simulation::q3::types::{Q3SourcePresentationClient, Q3SourcePresentationEntity};
    use qa_client::input::ControllerAxis;
    use qa_client::materials::q3_lighting::DynamicLight;
    use qa_client::render::types::ResourceOwner;
    use qa_content::contract::PresentationOwner;
    use qa_content::q3::presentation::movement_host::{CommandTiming, MoveBounds, PresentationMovementOptions};
    use qa_content::q3::presentation::ref_entity::ShadedFields;
    use qa_core::cmd_buffer::BufferOptions;
    use qa_core::identity::{IdentityOwner, ProviderId};
    use qa_guest::qvm::game_data::{ModuleIdentity, QvmArtifact, QvmImage, QvmRole};
    use qa_guest::qvm::player_record::QvmPlayerState;
    use qa_net::q3_net::Q3Product;

    #[derive(Debug, Default)]
    struct NativeCalls {
        frames: Vec<Q3ActiveFrame>,
        commands: Vec<Vec<String>>,
        keys: Vec<(i32, bool)>,
        mouse: Vec<(f64, f64)>,
        modes: Vec<u8>,
        closed: bool,
    }

    struct FakeNative {
        calls: Rc<RefCell<NativeCalls>>,
        hud: Q3HudState,
        handles: Vec<String>,
        execute: bool,
    }

    impl Q3NativeGame for FakeNative {
        fn draw_active_frame(&mut self, frame: Q3ActiveFrame) -> Result<(), Q3ClientError> {
            self.calls.borrow_mut().frames.push(frame);
            Ok(())
        }

        fn console_execute(&mut self, argv: &[String]) -> Result<bool, Q3ClientError> {
            self.calls.borrow_mut().commands.push(argv.to_vec());
            Ok(self.execute)
        }

        fn console_handles(&self, name: &str) -> bool {
            self.handles.iter().any(|handled| handled == name)
        }

        fn key_event(&mut self, key: i32, down: bool) -> Result<(), Q3ClientError> {
            self.calls.borrow_mut().keys.push((key, down));
            Ok(())
        }

        fn mouse_event(&mut self, x: f64, y: f64) -> Result<(), Q3ClientError> {
            self.calls.borrow_mut().mouse.push((x, y));
            Ok(())
        }

        fn event_handling(&mut self, mode: u8) -> Result<(), Q3ClientError> {
            self.calls.borrow_mut().modes.push(mode);
            Ok(())
        }

        fn hud_state(&self) -> Q3HudState {
            self.hud
        }

        fn close(&mut self) {
            self.calls.borrow_mut().closed = true;
        }
    }

    #[derive(Debug, Default)]
    struct QvmCalls {
        draws: Vec<(i32, bool)>,
        status_refreshes: u32,
        commands: Vec<Vec<String>>,
        keys: Vec<(i32, bool)>,
        mouse: Vec<(f64, f64)>,
        modes: Vec<Q3EventHandling>,
        shutdowns: u32,
        closed: bool,
    }

    struct FakeQvm {
        calls: Rc<RefCell<QvmCalls>>,
        captures: bool,
        equipment_visible: bool,
        equipment_hud: bool,
        execute: bool,
        draw_error: Option<String>,
        status_error: Option<String>,
    }

    impl Q3QvmGame for FakeQvm {
        fn draw(&mut self, time_ms: i32, demo_playback: bool) -> Result<(), Q3ClientError> {
            self.calls.borrow_mut().draws.push((time_ms, demo_playback));
            if let Some(error) = self.draw_error.as_ref() {
                return Err(Q3ClientError::Message(error.clone()));
            }
            Ok(())
        }

        fn refresh_status(&mut self) -> Result<(), Q3ClientError> {
            self.calls.borrow_mut().status_refreshes += 1;
            if let Some(error) = self.status_error.as_ref() {
                return Err(Q3ClientError::Message(error.clone()));
            }
            Ok(())
        }

        fn command(&mut self, argv: &[String]) -> Result<bool, Q3ClientError> {
            self.calls.borrow_mut().commands.push(argv.to_vec());
            Ok(self.execute)
        }

        fn key_event(&mut self, key: i32, down: bool) -> Result<(), Q3ClientError> {
            self.calls.borrow_mut().keys.push((key, down));
            Ok(())
        }

        fn mouse_event(&mut self, x: f64, y: f64) -> Result<(), Q3ClientError> {
            self.calls.borrow_mut().mouse.push((x, y));
            Ok(())
        }

        fn event_handling(&mut self, mode: Q3EventHandling) -> Result<(), Q3ClientError> {
            self.calls.borrow_mut().modes.push(mode);
            Ok(())
        }

        fn captures_input(&self) -> bool {
            self.captures
        }

        fn shared_held_weapons(&self) -> Vec<QvmHeldWeapon> {
            Vec::new()
        }

        fn shared_equipment_view_visible(&self) -> bool {
            self.equipment_visible
        }

        fn shared_equipment_hud(&self) -> bool {
            self.equipment_hud
        }

        fn presentation_artifacts(&self) -> Result<QvmPresentationArtifacts, Q3ClientError> {
            Ok(QvmPresentationArtifacts {
                ui: test_artifact(QvmRole::Ui),
                cgame: test_artifact(QvmRole::Cgame),
            })
        }

        fn shutdown(&mut self) -> Result<(), Q3ClientError> {
            self.calls.borrow_mut().shutdowns += 1;
            Ok(())
        }

        fn close(&mut self) {
            self.calls.borrow_mut().closed = true;
        }
    }

    fn test_artifact(role: QvmRole) -> QvmArtifact {
        QvmArtifact {
            module: ModuleIdentity {
                id: "test:qvm".to_string(),
                artifact_path: format!("vm/{role:?}.qvm").to_lowercase(),
                digest: "ab".repeat(32),
                revision: "1".to_string(),
            },
            role,
            abi_profile: None,
            image: QvmImage {
                source: "test".to_string(),
                instructions: Vec::new(),
                code_offset: 0,
                code_length: 0,
                data_length: 0,
                literal_length: 0,
                bss_length: 0,
                allocated_data_length: 0,
                initialized_data: Vec::new(),
                data_mask: 0,
            },
        }
    }

    struct FakeRemote {
        actor: ActorId,
        game_state: Vec<String>,
        commands: HashMap<i32, Vec<String>>,
        ping: Option<i32>,
        time: i32,
        client_number: i32,
        message_sequence: i32,
        last_command: i32,
        system_info: Option<String>,
        mode: Q3SourceMode,
    }

    impl Q3RemoteSource for FakeRemote {
        fn actor_at(&mut self, _number: i32) -> Result<ActorId, Q3ClientError> {
            Ok(self.actor.clone())
        }

        fn game_state(&self) -> Vec<String> {
            self.game_state.clone()
        }

        fn server_command(&mut self, sequence: i32) -> Result<Option<Vec<String>>, Q3ClientError> {
            Ok(self.commands.get(&sequence).cloned())
        }

        fn snapshot_ping(&self, _number: i32) -> Option<i32> {
            self.ping
        }

        fn time(&self) -> i32 {
            self.time
        }

        fn client_number(&self) -> i32 {
            self.client_number
        }

        fn server_message_sequence(&self) -> i32 {
            self.message_sequence
        }

        fn last_executed_server_command(&self) -> i32 {
            self.last_command
        }

        fn system_info(&self) -> Option<String> {
            self.system_info.clone()
        }

        fn source_mode(&self) -> Q3SourceMode {
            self.mode
        }
    }

    struct FakeMedia {
        content: ContentId,
        closed: RefCell<bool>,
    }

    impl Q3ClientMedia for FakeMedia {
        fn content(&self) -> ContentId {
            self.content.clone()
        }

        fn model_provider(&self, _model: &SceneModel) -> Option<usize> {
            None
        }

        fn provider_content(&self, _slot: usize) -> Option<ContentId> {
            None
        }

        fn build_light_sampler(&self) -> ModelLightSampler {
            ModelLightSampler::fullbright()
        }

        fn close(&self) {
            *self.closed.borrow_mut() = true;
        }
    }

    #[derive(Default)]
    struct FakeAudio {
        frames: RefCell<Vec<Vec<Q3SeatAudioOperation>>>,
    }

    impl Q3ClientAudio for FakeAudio {
        fn receive_cgame_frame(&self, _content: &ContentId, _seat: &SeatId, operations: Vec<Q3SeatAudioOperation>) {
            self.frames.borrow_mut().push(operations);
        }
    }

    #[derive(Default)]
    struct FakeRegistration {
        names: RefCell<HashSet<String>>,
        closed: RefCell<bool>,
    }

    impl Q3CommandRegistration for FakeRegistration {
        fn register(&self, name: &str) {
            self.names.borrow_mut().insert(name.to_string());
        }

        fn remove(&self, name: &str) {
            self.names.borrow_mut().remove(name);
        }

        fn close(&self) {
            *self.closed.borrow_mut() = true;
        }
    }

    struct FakeServices {
        closed: RefCell<bool>,
    }

    impl Q3ClientServices for FakeServices {
        fn close_cinematics(&mut self) {
            *self.closed.borrow_mut() = true;
        }

        fn registered_models(&self) -> Vec<String> {
            vec!["models/test".to_string()]
        }

        fn registered_skins(&self) -> Vec<String> {
            vec!["skins/test".to_string()]
        }
    }

    struct FakePipeline {
        began: RefCell<u32>,
        commands: RefCell<Vec<RenderCommand>>,
        texts: RefCell<u32>,
        direct: RefCell<u32>,
        direct_views: RefCell<Vec<Q3WorldView>>,
        worlds: RefCell<u32>,
        world_views: RefCell<Vec<Q3WorldView>>,
        preloaded: RefCell<usize>,
        finished: RefCell<Vec<bool>>,
        closed: RefCell<bool>,
        session: SessionId,
    }

    impl Q3RenderPipeline for FakePipeline {
        fn begin_frame(&mut self) {
            *self.began.borrow_mut() += 1;
        }

        fn submit_command(&mut self, command: RenderCommand) {
            self.commands.borrow_mut().push(command);
        }

        fn submit_text(&mut self, _draw: &MaterialTextDraw, _viewport: Rect, _view: &Q3TextView) {
            *self.texts.borrow_mut() += 1;
        }

        fn submit_direct_scene(&mut self, _scene: &Q3PresentedSceneView, view: &Q3WorldView, _flags: Q3SceneFlags) {
            *self.direct.borrow_mut() += 1;
            self.direct_views.borrow_mut().push(view.clone());
        }

        fn submit_world_scene(
            &mut self,
            _scene: &Q3PresentedSceneView,
            view: &Q3WorldView,
            _flags: Q3SceneFlags,
            _operations: &[SceneOperation],
            _view_offset: Option<Vec3>,
        ) {
            *self.worlds.borrow_mut() += 1;
            self.world_views.borrow_mut().push(view.clone());
        }

        fn preload_scenes(&mut self, scenes: &[Q3PresentedSceneView]) {
            *self.preloaded.borrow_mut() += scenes.len();
        }

        fn visible_surfaces(&mut self, _camera: &SceneCamera, _view: &Q3WorldView) -> Vec<usize> {
            Vec::new()
        }

        fn portal_surface(&self, _index: usize) -> Option<Q3PortalSurface> {
            None
        }

        fn finish_frame(&mut self, present: bool) -> RenderFrame {
            self.finished.borrow_mut().push(present);
            RenderFrame {
                owner: ResourceOwner::new(1, self.session.clone(), 0),
                sequence: 7,
                commands: Vec::new(),
            }
        }

        fn close(&mut self) {
            *self.closed.borrow_mut() = true;
        }
    }

    struct FakeQueries;

    impl ApplicationQ3SceneQueries for FakeQueries {
        fn point_leaf(&self, _point: Vec3) -> i32 {
            0
        }

        fn leaf_cluster(&self, _leaf: i32) -> i32 {
            0
        }

        fn leaf_area(&self, _leaf: i32) -> i32 {
            0
        }

        fn area_bits(&self, _area: i32) -> Vec<u8> {
            vec![0x01]
        }

        fn box_leaves(&self, _bounds: Bounds, _limit: i32) -> Vec<i32> {
            vec![0]
        }

        fn cluster_visible(&self, _from: i32, _cluster: i32) -> bool {
            true
        }

        fn areas_connected(&self, _first: i32, _second: i32) -> bool {
            true
        }
    }

    struct FakeMovement {
        timing: CommandTiming,
    }

    impl PresentationMovementHost for FakeMovement {
        fn command_timing(&self) -> CommandTiming {
            self.timing
        }

        fn move_player(
            &mut self,
            _state: &mut PlayerState,
            _command: &PredictionCommand,
            _options: &dyn PresentationMovementOptions,
        ) -> MoveBounds {
            MoveBounds {
                bounds: Bounds {
                    min: vec3(0.0, 0.0, 0.0),
                    max: vec3(0.0, 0.0, 0.0),
                },
            }
        }

        fn update_view_angles(&mut self, _state: &mut PlayerState, _command: &PredictionCommand) {}
    }

    struct FakeSnapshots {
        current: SnapshotCurrent,
    }

    impl SnapshotSource for FakeSnapshots {
        fn current(&self) -> SnapshotCurrent {
            self.current
        }

        fn read(&mut self, _number: i32) -> PresentClientResult<Option<PresentSnapshot>> {
            Ok(None)
        }
    }

    struct FakeCommands {
        number: i32,
    }

    impl CommandSource for FakeCommands {
        fn current_number(&self) -> i32 {
            self.number
        }

        fn read(&self, _number: i32) -> PresentClientResult<Option<PredictionCommand>> {
            Ok(None)
        }
    }

    type SubmitBodyFn = Rc<dyn Fn(i32, QvmBodyPart, RefModelEntity, bool) -> bool>;
    type BodySelectedFn = Rc<dyn Fn(i32) -> bool>;
    type ViewportProbeFn = Rc<dyn Fn() -> Rect>;
    type ServiceRemapFn = Rc<dyn Fn(&str, &str, &str) -> Result<(), String>>;
    type RemapRecord = (String, String, String, bool, bool);
    type RemoveCommandFn = Rc<dyn Fn(&str)>;

    #[derive(Default)]
    struct Captured {
        session: Option<Q3ClientSession>,
        submit_body: Option<SubmitBodyFn>,
        record_pose: Option<Rc<dyn Fn(Q3BodyPoseCapture)>>,
        gates: Option<Q3RenderGates>,
        weapon_selection: Option<Rc<dyn Fn(i32)>>,
        light: Option<Rc<dyn Fn(Vec3) -> Q3LightSample>>,
        memory: Option<Rc<dyn Fn() -> u64>>,
        ragepro: Option<bool>,
        movement: Option<Rc<RefCell<dyn PresentationMovementHost>>>,
    }

    #[derive(Default)]
    struct CapturedQvm {
        session: Option<Q3ClientSession>,
        key_catcher: Option<Q3KeyCatcher>,
        view_weapon_visible: Option<Rc<dyn Fn() -> bool>>,
        held_weapon_actor: Option<Rc<dyn Fn(i32) -> Option<ActorId>>>,
        body_capture_active: Option<Rc<dyn Fn() -> bool>>,
        body_selected: Option<BodySelectedFn>,
        submit_body: Option<SubmitBodyFn>,
        body_overrides_active: Option<Rc<dyn Fn() -> bool>>,
        body_hidden: Option<BodySelectedFn>,
        remove_command: Option<RemoveCommandFn>,
        light: Option<Rc<dyn Fn(Vec3) -> Q3LightSample>>,
    }

    struct Ids {
        actor: ActorId,
        seat: SeatId,
        client: ClientId,
        session: SessionId,
    }

    fn ids() -> (IdentityOwner, Ids) {
        let owner = IdentityOwner::create("q3-client-app-test").expect("owner");
        let ids = Ids {
            actor: owner.actor(3, 1),
            seat: owner.seat(0),
            client: owner.client(5, 1),
            session: owner.session().clone(),
        };
        (owner, ids)
    }

    fn test_viewport() -> Rect {
        Rect {
            x: 0,
            y: 0,
            width: 640,
            height: 480,
        }
    }

    fn healthy_hud() -> Q3HudState {
        Q3HudState {
            level_shot: false,
            show_scores: false,
            player: Some(Q3HudPlayer {
                health: 100,
                movement_type: MoveType::PmNormal,
                team: Team::TeamRed,
            }),
        }
    }

    struct Behavior {
        hud: Q3HudState,
        native_execute: bool,
        native_handles: Vec<String>,
        qvm_captures: bool,
        qvm_equipment_visible: bool,
        qvm_equipment_hud: bool,
        qvm_execute: bool,
        qvm_draw_error: Option<String>,
        qvm_status_error: Option<String>,
        equipment: Option<i32>,
        qvm_local_server: bool,
        renderer_name: Option<String>,
        settings: Vec<(String, String)>,
        server_settings: Vec<(String, String)>,
    }

    impl Default for Behavior {
        fn default() -> Self {
            Self {
                hud: healthy_hud(),
                native_execute: false,
                native_handles: Vec::new(),
                qvm_captures: false,
                qvm_equipment_visible: false,
                qvm_equipment_hud: false,
                qvm_execute: false,
                qvm_draw_error: None,
                qvm_status_error: None,
                equipment: None,
                qvm_local_server: false,
                renderer_name: None,
                settings: Vec::new(),
                server_settings: Vec::new(),
            }
        }
    }

    struct Handles {
        audio: Rc<FakeAudio>,
        pipeline: Rc<RefCell<FakePipeline>>,
        registration: Rc<FakeRegistration>,
        media: Rc<FakeMedia>,
        native_calls: Rc<RefCell<NativeCalls>>,
        qvm_calls: Rc<RefCell<QvmCalls>>,
        captured: Rc<RefCell<Captured>>,
        captured_qvm: Rc<RefCell<CapturedQvm>>,
        services_output: Rc<RefCell<Option<Q3ServiceOutput>>>,
        services_viewport: Rc<RefCell<Option<ViewportProbeFn>>>,
        services_remap: Rc<RefCell<Option<ServiceRemapFn>>>,
        actor_map: Rc<RefCell<HashMap<i32, ActorId>>>,
        link_bounds: Rc<RefCell<HashMap<i32, Bounds>>>,
        reliable: Rc<RefCell<Vec<String>>>,
        console: Rc<RefCell<Vec<String>>>,
        prints: Rc<RefCell<Vec<String>>>,
        asserts: Rc<RefCell<u32>>,
        remaps: Rc<RefCell<Vec<RemapRecord>>>,
        time_cvars: Option<SharedCvars>,
    }

    fn make_handles(ids: &Ids) -> Handles {
        Handles {
            audio: Rc::new(FakeAudio::default()),
            pipeline: Rc::new(RefCell::new(FakePipeline {
                began: RefCell::new(0),
                commands: RefCell::new(Vec::new()),
                texts: RefCell::new(0),
                direct: RefCell::new(0),
                direct_views: RefCell::new(Vec::new()),
                worlds: RefCell::new(0),
                world_views: RefCell::new(Vec::new()),
                preloaded: RefCell::new(0),
                finished: RefCell::new(Vec::new()),
                closed: RefCell::new(false),
                session: ids.session.clone(),
            })),
            registration: Rc::new(FakeRegistration::default()),
            media: Rc::new(FakeMedia {
                content: ContentId("test:content".to_string()),
                closed: RefCell::new(false),
            }),
            native_calls: Rc::new(RefCell::new(NativeCalls::default())),
            qvm_calls: Rc::new(RefCell::new(QvmCalls::default())),
            captured: Rc::new(RefCell::new(Captured::default())),
            captured_qvm: Rc::new(RefCell::new(CapturedQvm::default())),
            services_output: Rc::new(RefCell::new(None)),
            services_viewport: Rc::new(RefCell::new(None)),
            services_remap: Rc::new(RefCell::new(None)),
            actor_map: Rc::new(RefCell::new(HashMap::new())),
            link_bounds: Rc::new(RefCell::new(HashMap::new())),
            reliable: Rc::new(RefCell::new(Vec::new())),
            console: Rc::new(RefCell::new(Vec::new())),
            prints: Rc::new(RefCell::new(Vec::new())),
            asserts: Rc::new(RefCell::new(0)),
            remaps: Rc::new(RefCell::new(Vec::new())),
            time_cvars: None,
        }
    }

    fn seed_snapshots(settings: &[(String, String)]) -> Vec<CvarSnapshot> {
        let mut seed = CvarRegistry::new(Dialect::Q3);
        let mut out = Vec::new();
        for (name, value) in settings {
            seed.set(name, value, true).expect("seed");
            out.push(seed.get(name).expect("seed get"));
        }
        out
    }

    fn make_common(ids: &Ids, handles: &Handles, behavior: &Behavior) -> Q3ClientCommonOptions {
        let output_slot = handles.services_output.clone();
        let viewport_slot = handles.services_viewport.clone();
        let remap_slot = handles.services_remap.clone();
        let create_services: Q3CreateServices = Rc::new(move |params| {
            *output_slot.borrow_mut() = Some(params.output.clone());
            *viewport_slot.borrow_mut() = Some(params.viewport.clone());
            *remap_slot.borrow_mut() = params.remap_shader.clone();
            Ok(Box::new(FakeServices {
                closed: RefCell::new(false),
            }) as Box<dyn Q3ClientServices>)
        });
        let native_calls = handles.native_calls.clone();
        let captured = handles.captured.clone();
        let hud = behavior.hud;
        let execute = behavior.native_execute;
        let owns = behavior.native_handles.clone();
        let create_native: Q3CreateNative = Rc::new(move |params| {
            let mut slot = captured.borrow_mut();
            slot.session = Some(params.session.clone());
            slot.submit_body = Some(params.submit_body.clone());
            slot.record_pose = Some(params.record_body_pose.clone());
            slot.gates = Some(params.gates.clone());
            slot.weapon_selection = Some(params.weapon_selection.clone());
            slot.light = Some(params.light_for_point.clone());
            slot.memory = Some(params.memory_remaining.clone());
            slot.ragepro = Some(params.hardware_ragepro);
            slot.movement = Some(params.movement.clone());
            Ok(Box::new(FakeNative {
                calls: native_calls.clone(),
                hud,
                handles: owns.clone(),
                execute,
            }) as Box<dyn Q3NativeGame>)
        });
        let qvm_calls = handles.qvm_calls.clone();
        let captured_qvm = handles.captured_qvm.clone();
        let captures = behavior.qvm_captures;
        let equipment_visible = behavior.qvm_equipment_visible;
        let equipment_hud = behavior.qvm_equipment_hud;
        let qvm_execute = behavior.qvm_execute;
        let draw_error = behavior.qvm_draw_error.clone();
        let status_error = behavior.qvm_status_error.clone();
        let create_qvm: Q3CreateQvm = Rc::new(move |params| {
            let mut slot = captured_qvm.borrow_mut();
            slot.session = Some(params.session.clone());
            slot.key_catcher = Some(params.key_catcher.clone());
            slot.view_weapon_visible = Some(params.view_weapon_visible.clone());
            slot.held_weapon_actor = Some(params.held_weapon_actor.clone());
            slot.body_capture_active = Some(params.body_capture_active.clone());
            slot.body_selected = Some(params.body_selected.clone());
            slot.submit_body = Some(params.submit_body.clone());
            slot.body_overrides_active = Some(params.body_overrides_active.clone());
            slot.body_hidden = Some(params.body_hidden.clone());
            slot.remove_command = Some(params.remove_command.clone());
            slot.light = Some(params.light_for_point.clone());
            Ok(Box::new(FakeQvm {
                calls: qvm_calls.clone(),
                captures,
                equipment_visible,
                equipment_hud,
                execute: qvm_execute,
                draw_error: draw_error.clone(),
                status_error: status_error.clone(),
            }) as Box<dyn Q3QvmGame>)
        });
        let reliable = handles.reliable.clone();
        let console = handles.console.clone();
        let prints = handles.prints.clone();
        let asserts = handles.asserts.clone();
        let remaps = handles.remaps.clone();
        let context = CommandContext::new(
            ids.session.clone(),
            CommandOrigin::LocalSeat {
                seat: ids.seat.clone(),
                client: ids.client.clone(),
            },
        );
        let viewport = test_viewport();
        let driver = behavior.renderer_name.clone().map(|renderer| QvmDisplayDriver {
            renderer,
            vendor: "test".to_string(),
            version: "1".to_string(),
        });
        Q3ClientCommonOptions {
            cvars: None,
            time_cvars: handles.time_cvars.clone(),
            weapon_hud: None,
            local: Q3ClientLocal {
                actor: ids.actor.clone(),
                seat: ids.seat.clone(),
                client: ids.client.clone(),
                session: ids.session.clone(),
                console_close: Rc::new(|| {}),
            },
            audio: handles.audio.clone(),
            settings: seed_snapshots(&behavior.settings),
            split_screen: false,
            viewport: Rc::new(move || viewport),
            now: Rc::new(|| 1000.0),
            commands: Q3ClientCommands {
                reliable: Rc::new(move |text| reliable.borrow_mut().push(text.to_string())),
                console: Rc::new(move |text| console.borrow_mut().push(text.to_string())),
                print: Rc::new(move |text| prints.borrow_mut().push(text.to_string())),
            },
            command_registration: handles.registration.clone(),
            server_settings: if behavior.server_settings.is_empty() {
                None
            } else {
                let snapshots = seed_snapshots(&behavior.server_settings);
                Some(Rc::new(move || snapshots.clone()) as Rc<dyn Fn() -> Vec<CvarSnapshot>>)
            },
            assert_current: Some(Rc::new(move || {
                *asserts.borrow_mut() += 1;
            })),
            media: handles.media.clone(),
            pipeline: handles.pipeline.clone(),
            create_services,
            create_native,
            create_qvm,
            command_buffer: Rc::new(RefCell::new(
                CommandBuffer::new(Dialect::Q3, context, BufferOptions::default()).expect("buffer"),
            )),
            memory_remaining: Rc::new(|| u64::MAX),
            remap_shader: Some(Rc::new(move |original, replacement, offset, initializing, current| {
                remaps.borrow_mut().push((
                    original.to_string(),
                    replacement.to_string(),
                    offset.to_string(),
                    initializing,
                    current(),
                ));
                Ok(())
            })),
            renderer: QvmDisplayRenderer {
                stencil_bits: 8,
                driver,
                gl_config: None,
            },
        }
    }

    fn local_initial(actor: &ActorId, slot: i32, origin: Vec3, height: i32, angles: Vec3) -> Q3SourcePresentationState {
        let player = QvmPlayerState {
            origin,
            view_height: height,
            view_angles: angles,
            ..QvmPlayerState::default()
        };
        Q3SourcePresentationState {
            product: Product::Baseq3,
            time: 0,
            entities: Vec::new(),
            clients: vec![Q3SourcePresentationClient {
                actor: actor.clone(),
                slot,
                state: player,
            }],
            configstrings: Vec::new(),
        }
    }

    fn make_local(
        ids: &Ids,
        handles: &Handles,
        behavior: &Behavior,
        initial: Q3SourcePresentationState,
    ) -> ApplicationQ3ClientOptions {
        let actor_map = handles.actor_map.clone();
        let bounds_map = handles.link_bounds.clone();
        ApplicationQ3ClientOptions {
            common: make_common(ids, handles, behavior),
            kind: Q3ClientKindOptions::Local(Box::new(Q3LocalOptions {
                movement: Rc::new(RefCell::new(FakeMovement {
                    timing: CommandTiming::Q3,
                })),
                initial,
                link_bounds: Rc::new(move |number| bounds_map.borrow().get(&number).cloned()),
                source_actor: Rc::new(move |number| actor_map.borrow().get(&number).cloned()),
                prediction_command: None,
                queries: Rc::new(FakeQueries),
                leaf_count: 64,
                same_session: Rc::new(|_| true),
                server_settings: None,
            })),
        }
    }

    fn remote_surface(ids: &Ids) -> Rc<RefCell<FakeRemote>> {
        Rc::new(RefCell::new(FakeRemote {
            actor: ids.actor.clone(),
            game_state: Vec::new(),
            commands: HashMap::new(),
            ping: None,
            time: 500,
            client_number: 2,
            message_sequence: 11,
            last_command: 9,
            system_info: None,
            mode: Q3SourceMode::Live,
        }))
    }

    fn make_remote(
        ids: &Ids,
        handles: &Handles,
        behavior: &Behavior,
        surface: Rc<RefCell<FakeRemote>>,
        player: NetPlayerState,
    ) -> ApplicationQ3ClientOptions {
        let snapshots: SharedSnapshots = Rc::new(RefCell::new(FakeSnapshots {
            current: SnapshotCurrent {
                number: 3,
                server_time: 900,
            },
        }));
        let commands: SharedCommands = Rc::new(RefCell::new(FakeCommands { number: 7 }));
        let erased: Rc<RefCell<dyn Q3RemoteSource>> = surface;
        ApplicationQ3ClientOptions {
            common: make_common(ids, handles, behavior),
            kind: Q3ClientKindOptions::Remote(Box::new(Q3RemoteOptions {
                movement: Rc::new(RefCell::new(FakeMovement {
                    timing: CommandTiming::Q3,
                })),
                snapshots,
                commands,
                surface: erased,
                initial_player: player,
            })),
        }
    }

    fn make_qvm(
        ids: &Ids,
        handles: &Handles,
        behavior: &Behavior,
        surface: Rc<RefCell<FakeRemote>>,
    ) -> ApplicationQ3ClientOptions {
        let snapshots: SharedSnapshots = Rc::new(RefCell::new(FakeSnapshots {
            current: SnapshotCurrent {
                number: 3,
                server_time: 900,
            },
        }));
        let commands: SharedCommands = Rc::new(RefCell::new(FakeCommands { number: 7 }));
        let erased: Rc<RefCell<dyn Q3RemoteSource>> = surface;
        let equipment = behavior.equipment;
        ApplicationQ3ClientOptions {
            common: make_common(ids, handles, behavior),
            kind: Q3ClientKindOptions::Qvm(Q3QvmOptions {
                local_server: behavior.qvm_local_server,
                snapshots,
                commands,
                surface: erased,
                equipment_primary_weapon: equipment
                    .map(|weapon| Rc::new(move || Some(weapon)) as Rc<dyn Fn() -> Option<i32>>),
            }),
        }
    }

    fn test_player() -> NetPlayerState {
        let mut player = NetPlayerState::new(Q3Product::Base);
        player.origin = [4.0, 5.0, 6.0];
        player.viewheight = 26;
        player.viewangles = [0.0, 90.0, 0.0];
        player
    }

    fn test_body_entity() -> RefModelEntity {
        RefModelEntity {
            shading: ShadedFields::default(),
            model: SceneModel::default_model(),
            origin: vec3(1.0, 2.0, 3.0),
            old_origin: vec3(0.0, 0.0, 0.0),
            axis: [vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)],
            non_normalized_axes: false,
            lighting_origin: vec3(0.0, 0.0, 0.0),
            shadow_plane: 0.0,
            frame: 0,
            old_frame: 0,
            back_lerp: 0.0,
            skin_num: 0,
            custom_skin: None,
        }
    }

    fn test_scene(camera: SceneCamera, flags: i32) -> Q3PresentedSceneView {
        Q3PresentedSceneView {
            camera,
            viewport: test_viewport(),
            source: Refdef {
                render_flags: flags,
                ..Refdef::default()
            },
            lights: Vec::new(),
            portals: Vec::new(),
            admission_entity_count: 0,
        }
    }

    fn test_model(actor: &ActorId) -> Q3ClientModel {
        Q3ClientModel {
            actor: actor.clone(),
            render_owner_source_client: false,
            view_weapon: false,
            path: String::new(),
            held_weapon: None,
            replaces_body: false,
            visible: true,
        }
    }

    fn local_client(ids: &Ids, handles: &Handles, behavior: &Behavior) -> ApplicationQ3Client {
        let initial = local_initial(&ids.actor, 0, vec3(10.0, 20.0, 30.0), 26, vec3(0.0, 90.0, 0.0));
        ApplicationQ3Client::create(make_local(ids, handles, behavior, initial), None).expect("create local client")
    }

    #[test]
    fn local_create_builds_camera_and_marks_server_running() {
        let (_owner, ids) = ids();
        let handles = make_handles(&ids);
        let client = local_client(&ids, &handles, &Behavior::default());
        let camera = client.camera();
        assert_eq!(camera.origin, vec3(10.0, 20.0, 56.0));
        assert_eq!(camera.axis, angles_to_axis(vec3(0.0, 90.0, 0.0)));
        assert_eq!(camera.viewport, test_viewport());
        assert_eq!(
            client.cvars().borrow().get("sv_running").expect("sv_running").value,
            "1"
        );
        assert!(*handles.asserts.borrow() > 0);
    }

    #[test]
    fn local_create_reads_initial_snapshot_through_session() {
        let (_owner, ids) = ids();
        let handles = make_handles(&ids);
        let client = local_client(&ids, &handles, &Behavior::default());
        let session = handles.captured.borrow().session.clone().expect("session");
        assert_eq!(session.client_number, 0);
        assert_eq!(session.product, Product::Baseq3);
        assert_eq!(session.snapshots.borrow().current().number, 1);
        let snapshot = session.snapshots.borrow_mut().read(1).expect("read").expect("snapshot");
        assert_eq!(snapshot.player_state.origin(), vec3(10.0, 20.0, 30.0));
        assert_eq!(snapshot.player_state.viewheight, 26);
        assert!(session.snapshots.borrow_mut().read(99).expect("read").is_none());
        assert_eq!(session.commands.borrow().current_number(), 0);
        assert!((session.server_command)(1).is_none());
        assert_eq!((session.snapshot_ping)(1), 0);
        assert_eq!((session.server_time)(), 0);
        assert!((session.status_visible)());
        drop(client);
    }

    #[test]
    fn remote_create_applies_system_info_and_clears_server_flag() {
        let (_owner, ids) = ids();
        let handles = make_handles(&ids);
        let surface = remote_surface(&ids);
        surface.borrow_mut().system_info = Some("\\sv_cheats\\1\\mapname\\q3dm1\\cl_allowdownload\\0".to_string());
        let client = ApplicationQ3Client::create(
            make_remote(&ids, &handles, &Behavior::default(), surface, test_player()),
            None,
        )
        .expect("create remote client");
        let cvars = client.cvars();
        let cvars = cvars.borrow();
        assert_eq!(cvars.get("sv_cheats").expect("sv_cheats").value, "1");
        assert_eq!(cvars.get("mapname").expect("mapname").value, "q3dm1");
        assert!(cvars.get("cl_allowdownload").is_none());
        assert_eq!(cvars.get("sv_running").expect("sv_running").value, "0");
        let camera = client.camera();
        assert_eq!(camera.origin, vec3(4.0, 5.0, 32.0));
    }

    #[test]
    fn refresh_system_info_skips_canonical_time_names() {
        let (_owner, ids) = ids();
        let mut handles = make_handles(&ids);
        handles.time_cvars = Some(Rc::new(RefCell::new(CvarRegistry::new(Dialect::Q3))));
        let surface = remote_surface(&ids);
        surface.borrow_mut().system_info = Some("\\timescale\\5\\fixedtime\\3\\mapname\\q3dm1".to_string());
        let client = ApplicationQ3Client::create(
            make_remote(&ids, &handles, &Behavior::default(), surface, test_player()),
            None,
        )
        .expect("create");
        assert!(client.cvars().borrow().get("timescale").is_none());
        assert!(client.cvars().borrow().get("fixedtime").is_none());
        assert_eq!(client.cvars().borrow().get("mapname").expect("mapname").value, "q3dm1");
    }

    #[test]
    fn refresh_system_info_reapplies_timescale_once_without_time_cvars() {
        let (_owner, ids) = ids();
        let handles = make_handles(&ids);
        let surface = remote_surface(&ids);
        surface.borrow_mut().system_info = Some("\\timescale\\5".to_string());
        let mut client = ApplicationQ3Client::create(
            make_remote(&ids, &handles, &Behavior::default(), surface.clone(), test_player()),
            None,
        )
        .expect("create");
        assert_eq!(client.cvars().borrow().get("timescale").expect("timescale").value, "5");
        client.cvars().borrow_mut().set("timescale", "9", true).expect("set");
        client.refresh_system_info().expect("refresh");
        assert_eq!(client.cvars().borrow().get("timescale").expect("timescale").value, "9");
        surface.borrow_mut().system_info = Some("\\timescale\\7".to_string());
        client.refresh_system_info().expect("refresh");
        assert_eq!(client.cvars().borrow().get("timescale").expect("timescale").value, "7");
    }

    #[test]
    fn time_mirror_binds_canonical_names_and_refreshes() {
        let (_owner, ids) = ids();
        let mut handles = make_handles(&ids);
        let time = Rc::new(RefCell::new(CvarRegistry::new(Dialect::Q3)));
        time.borrow_mut().set("timescale", "2", true).expect("set");
        time.borrow_mut().set("cm_noCurves", "1", true).expect("set");
        handles.time_cvars = Some(time.clone());
        let mut client = local_client(&ids, &handles, &Behavior::default());
        assert_eq!(client.cvars().borrow().get("timescale").expect("timescale").value, "2");
        assert_eq!(
            client.cvars().borrow().get("cm_noCurves").expect("cm_noCurves").value,
            "1"
        );
        assert!(client.cvars().borrow().get("fixedtime").is_none());
        time.borrow_mut().set("timescale", "4", true).expect("set");
        client.refresh_system_info().expect("refresh");
        assert_eq!(client.cvars().borrow().get("timescale").expect("timescale").value, "4");
        client.cvars().borrow_mut().set("timescale", "6", true).expect("set");
        client.refresh_system_info().expect("refresh");
        assert_eq!(time.borrow().get("timescale").expect("owner timescale").value, "6");
    }

    #[test]
    fn adopt_cvars_swaps_contents_and_validates_dialect() {
        let (_owner, ids) = ids();
        let handles = make_handles(&ids);
        let mut client = local_client(&ids, &handles, &Behavior::default());
        let adopted = Rc::new(RefCell::new(CvarRegistry::new(Dialect::Q3)));
        adopted.borrow_mut().set("adopted_marker", "yes", true).expect("set");
        client.adopt_cvars(adopted.clone()).expect("adopt");
        assert_eq!(
            client.cvars().borrow().get("adopted_marker").expect("marker").value,
            "yes"
        );
        assert!(adopted.borrow().get("sv_running").is_some());
        let session = handles.captured.borrow().session.clone().expect("session");
        assert_eq!(
            session.cvars.borrow().get("adopted_marker").expect("marker").value,
            "yes"
        );
        let foreign = Rc::new(RefCell::new(CvarRegistry::new(Dialect::Q2Classic)));
        assert!(client.adopt_cvars(foreign).is_err());
    }

    #[test]
    fn round_restart_rebinds_actor_and_checks_timing() {
        let (owner, ids) = ids();
        let handles = make_handles(&ids);
        let mut client = local_client(&ids, &handles, &Behavior::default());
        client.assert_can_restart_round().expect("assert");
        let next = owner.actor(4, 1);
        let mut state = local_initial(&next, 0, vec3(1.0, 2.0, 3.0), 26, vec3(0.0, 0.0, 0.0));
        state.time = 100;
        client.begin_round_restart(4, &state).expect("begin");
        assert!(client.begin_round_restart(0, &state).is_err());
        let bad = ApplicationQ3LocalRound {
            actor: next.clone(),
            initial: state.clone(),
            movement: Rc::new(RefCell::new(FakeMovement {
                timing: CommandTiming::Provider,
            })),
            link_bounds: Rc::new(|_| None),
            source_actor: Rc::new(|_| None),
            prediction_command: None,
            server_settings: None,
        };
        assert!(client.rebind_round(bad).is_err());
        let good = ApplicationQ3LocalRound {
            actor: next.clone(),
            initial: state.clone(),
            movement: Rc::new(RefCell::new(FakeMovement {
                timing: CommandTiming::Q3,
            })),
            link_bounds: Rc::new(|_| None),
            source_actor: Rc::new(|_| None),
            prediction_command: None,
            server_settings: None,
        };
        client.rebind_round(good).expect("rebind");
        client.assert_can_restart_round().expect("assert again");
    }

    #[test]
    fn remote_rejects_round_restart() {
        let (_owner, ids) = ids();
        let handles = make_handles(&ids);
        let surface = remote_surface(&ids);
        let mut client = ApplicationQ3Client::create(
            make_remote(&ids, &handles, &Behavior::default(), surface, test_player()),
            None,
        )
        .expect("create");
        assert!(client.assert_can_restart_round().is_err());
        let state = local_initial(&ids.actor, 0, vec3(0.0, 0.0, 0.0), 0, vec3(0.0, 0.0, 0.0));
        assert!(client.begin_round_restart(0, &state).is_err());
    }

    #[test]
    fn prepare_drives_native_frame_and_drains_audio() {
        let (_owner, ids) = ids();
        let handles = make_handles(&ids);
        let mut client = local_client(&ids, &handles, &Behavior::default());
        let output = handles.services_output.borrow().clone().expect("output");
        (output.audio)(Q3SeatAudioOperation::ClearLoops { kill_all: false });
        let viewport = Rect {
            x: 8,
            y: 8,
            width: 320,
            height: 200,
        };
        client
            .prepare(11, viewport, &[], false, &[], true, false)
            .expect("prepare");
        let frames = handles.native_calls.borrow();
        assert_eq!(frames.frames.len(), 1);
        assert_eq!(frames.frames[0].server_time_ms, 0);
        assert!(!frames.frames[0].demo_playback);
        assert_eq!(frames.frames[0].engine_frame_number, 11);
        assert_eq!(*handles.pipeline.borrow().preloaded.borrow(), 0);
        let audio_frames = handles.audio.frames.borrow();
        assert_eq!(audio_frames.len(), 1);
        assert_eq!(audio_frames[0].len(), 1);
        let session = handles.captured.borrow().session.clone().expect("session");
        assert!((session.status_visible)());
        let probe = handles.services_viewport.borrow().clone().expect("viewport probe");
        assert_eq!(probe(), viewport);
    }

    #[test]
    fn submit_body_gates_on_selection_and_preparation() {
        let (_owner, ids) = ids();
        let handles = make_handles(&ids);
        handles.actor_map.borrow_mut().insert(5, ids.actor.clone());
        let mut client = local_client(&ids, &handles, &Behavior::default());
        client
            .prepare(1, test_viewport(), &[], true, &[], true, false)
            .expect("prepare");
        let submit = handles.captured.borrow().submit_body.clone().expect("submit");
        assert!(!submit(5, QvmBodyPart::Body, test_body_entity(), false));
        let bodies = vec![ComponentBody {
            owner: PresentationOwner {
                provider: ProviderId::new("test", "bodies"),
                generation: 1,
            },
            actor: ids.actor.clone(),
            content: ContentId("test:content".to_string()),
            time: 0.0,
            parts: Vec::new(),
        }];
        client
            .prepare(2, test_viewport(), &[], true, &bodies, true, false)
            .expect("prepare");
        assert!(!submit(5, QvmBodyPart::Head, test_body_entity(), true));
        assert!(client.shared_bodies().is_empty());
        let mut hidden = test_model(&ids.actor);
        hidden.replaces_body = true;
        client
            .prepare(3, test_viewport(), &[hidden], true, &bodies, true, false)
            .expect("prepare");
        assert!(!submit(5, QvmBodyPart::Body, test_body_entity(), false));
    }

    #[test]
    fn prepare_records_poses_gates_and_selections() {
        let (_owner, ids) = ids();
        let handles = make_handles(&ids);
        handles.actor_map.borrow_mut().insert(5, ids.actor.clone());
        let mut client = local_client(&ids, &handles, &Behavior::default());
        let mut weapon = test_model(&ids.actor);
        weapon.view_weapon = true;
        weapon.path = "models/gun".to_string();
        let pose = test_model(&ids.actor);
        client
            .prepare(4, test_viewport(), &[weapon, pose], true, &[], false, false)
            .expect("prepare");
        let captured = handles.captured.borrow();
        let record = captured.record_pose.clone().expect("record");
        record(Q3BodyPoseCapture {
            number: 5,
            origin: vec3(7.0, 8.0, 9.0),
            angles: vec3(0.0, 45.0, 0.0),
        });
        assert_eq!(
            client.body_pose(&ids.actor),
            Some((vec3(7.0, 8.0, 9.0), vec3(0.0, 45.0, 0.0)))
        );
        let gates = captured.gates.clone().expect("gates");
        assert!(!(gates.view_weapon)());
        assert!((gates.supplemental_view_weapon)());
        (gates.note_event)();
        (gates.note_event)();
        assert_eq!(client.presented_events(), 2);
        let select = captured.weapon_selection.clone().expect("selection");
        assert_eq!(client.consume_weapon_selection(), None);
        select(4);
        assert_eq!(client.consume_weapon_selection(), Some(4));
        assert_eq!(client.consume_weapon_selection(), None);
    }

    #[test]
    fn qvm_prepare_refreshes_status_and_combines_failures() {
        let (_owner, ids) = ids();
        let handles = make_handles(&ids);
        let surface = remote_surface(&ids);
        let mut client =
            ApplicationQ3Client::create(make_qvm(&ids, &handles, &Behavior::default(), surface), None).expect("create");
        client
            .prepare(6, test_viewport(), &[], false, &[], true, false)
            .expect("prepare");
        assert_eq!(handles.qvm_calls.borrow().draws, vec![(500, false)]);
        assert_eq!(handles.qvm_calls.borrow().status_refreshes, 1);
        let session = handles.captured_qvm.borrow().session.clone().expect("session");
        assert!((session.status_visible)());

        let handles = make_handles(&ids);
        let surface = remote_surface(&ids);
        let behavior = Behavior {
            qvm_draw_error: Some("draw boom".to_string()),
            qvm_status_error: Some("status boom".to_string()),
            ..Behavior::default()
        };
        let mut client =
            ApplicationQ3Client::create(make_qvm(&ids, &handles, &behavior, surface), None).expect("create");
        let error = client
            .prepare(7, test_viewport(), &[], true, &[], true, false)
            .expect_err("prepare");
        let message = error.to_string();
        assert!(
            message.contains("Cgame frame and status restoration failed"),
            "{message}"
        );
        assert!(message.contains("draw boom"), "{message}");
        assert!(message.contains("status boom"), "{message}");

        let handles = make_handles(&ids);
        let surface = remote_surface(&ids);
        let behavior = Behavior {
            qvm_status_error: Some("status only".to_string()),
            ..Behavior::default()
        };
        let mut client =
            ApplicationQ3Client::create(make_qvm(&ids, &handles, &behavior, surface), None).expect("create");
        let error = client
            .prepare(8, test_viewport(), &[], true, &[], true, false)
            .expect_err("prepare");
        assert_eq!(error.to_string(), "status only");
    }

    #[test]
    fn command_and_handles_dispatch_by_backend() {
        let (_owner, ids) = ids();
        let handles = make_handles(&ids);
        let behavior = Behavior {
            native_execute: true,
            native_handles: vec!["+attack".to_string()],
            ..Behavior::default()
        };
        let mut client = local_client(&ids, &handles, &behavior);
        assert!(client.command(&["+attack".to_string()]).expect("command"));
        assert_eq!(
            handles.native_calls.borrow().commands,
            vec![vec!["+attack".to_string()]]
        );
        assert!(client.handles_command("+attack").expect("handles"));
        assert!(!client.handles_command("nope").expect("handles"));

        let handles = make_handles(&ids);
        let surface = remote_surface(&ids);
        let behavior = Behavior {
            qvm_execute: true,
            ..Behavior::default()
        };
        let mut client =
            ApplicationQ3Client::create(make_qvm(&ids, &handles, &behavior, surface), None).expect("create");
        assert!(client.command(&["hook".to_string()]).expect("command"));
        assert!(!client.handles_command("hook").expect("handles"));
        let session = handles.captured_qvm.borrow().session.clone().expect("session");
        (session.register_cgame_command)("hook");
        assert!(client.handles_command("hook").expect("handles"));
        assert_eq!(client.command_names(), vec!["hook".to_string()]);
        let remove = handles.captured_qvm.borrow().remove_command.clone().expect("remove");
        remove("hook");
        assert!(!client.handles_command("hook").expect("handles"));
    }

    #[test]
    fn key_mouse_and_event_handling_forward() {
        let (_owner, ids) = ids();
        let handles = make_handles(&ids);
        let mut client = local_client(&ids, &handles, &Behavior::default());
        client.key_event(42, true).expect("key");
        client.mouse_event(1.5, -2.5).expect("mouse");
        client.event_handling(2).expect("mode");
        assert_eq!(handles.native_calls.borrow().keys, vec![(42, true)]);
        assert_eq!(handles.native_calls.borrow().mouse, vec![(1.5, -2.5)]);
        assert_eq!(handles.native_calls.borrow().modes, vec![2]);
        assert!(client.event_handling(4).is_err());

        let handles = make_handles(&ids);
        let surface = remote_surface(&ids);
        let mut client =
            ApplicationQ3Client::create(make_qvm(&ids, &handles, &Behavior::default(), surface), None).expect("create");
        for mode in 0..=3 {
            client.event_handling(mode).expect("mode");
        }
        assert_eq!(
            handles.qvm_calls.borrow().modes,
            vec![
                Q3EventHandling::None,
                Q3EventHandling::TeamMenu,
                Q3EventHandling::Scoreboard,
                Q3EventHandling::EditHud,
            ]
        );
    }

    #[test]
    fn input_routes_seat_events() {
        let (_owner, ids) = ids();
        let handles = make_handles(&ids);
        let mut client = local_client(&ids, &handles, &Behavior::default());
        let event = |kind| SeatInputEvent {
            seat: ids.seat.clone(),
            time_ms: 0,
            kind,
        };
        client
            .input(&event(SeatInputEventKind::Key {
                code: 65,
                down: true,
                repeat: false,
            }))
            .expect("key");
        client
            .input(&event(SeatInputEventKind::Text { text: "A".to_string() }))
            .expect("text");
        client
            .input(&event(SeatInputEventKind::MouseButton { button: 2, down: true }))
            .expect("button");
        client
            .input(&event(SeatInputEventKind::MouseMotion {
                position: Vec2 { x: 0.0, y: 0.0 },
                delta: Vec2 { x: 1.0, y: 2.0 },
            }))
            .expect("motion");
        client
            .input(&event(SeatInputEventKind::MouseWheel {
                delta: Vec2 { x: 0.0, y: 2.0 },
            }))
            .expect("wheel");
        client
            .input(&event(SeatInputEventKind::ControllerButton {
                device: 0,
                button: 1,
                down: true,
            }))
            .expect("pad");
        client
            .input(&event(SeatInputEventKind::ControllerAxis {
                device: 0,
                axis: ControllerAxis::LeftX,
                value: 0.5,
            }))
            .expect("axis");
        client
            .input(&event(SeatInputEventKind::Focus { focused: true }))
            .expect("focus");
        assert_eq!(
            handles.native_calls.borrow().keys,
            vec![
                (65, true),
                (65 | KEY_CHAR_FLAG, true),
                (KeyCode::Mouse1 as i32 + 2, true),
                (KeyCode::MouseWheelUp as i32, true),
                (KeyCode::MouseWheelUp as i32, false),
                (KeyCode::MouseWheelUp as i32, true),
                (KeyCode::MouseWheelUp as i32, false),
                (KeyCode::Joy1 as i32 + 1, true),
            ]
        );
        assert_eq!(handles.native_calls.borrow().mouse, vec![(1.0, 2.0)]);
        let foreign = SeatInputEvent {
            seat: ids.seat.clone(),
            time_ms: 0,
            kind: SeatInputEventKind::Focus { focused: true },
        };
        let mut foreign = foreign;
        foreign.seat = IdentityOwner::create("other").expect("owner").seat(9);
        assert!(client.input(&foreign).is_err());
    }

    #[test]
    fn captures_input_reads_backend_or_catcher() {
        let (_owner, ids) = ids();
        let handles = make_handles(&ids);
        let client = local_client(&ids, &handles, &Behavior::default());
        assert!(!client.captures_input());

        let handles = make_handles(&ids);
        let surface = remote_surface(&ids);
        let behavior = Behavior {
            qvm_captures: true,
            ..Behavior::default()
        };
        let client = ApplicationQ3Client::create(make_qvm(&ids, &handles, &behavior, surface), None).expect("create");
        assert!(client.captures_input());
        let catcher = handles.captured_qvm.borrow().key_catcher.clone().expect("catcher");
        assert_eq!((catcher.get)(), 0);
        (catcher.set)(3);
        assert_eq!((catcher.get)(), 3);
    }

    #[test]
    fn weapon_hud_view_combines_state_and_cvars() {
        let (_owner, ids) = ids();
        let handles = make_handles(&ids);
        let client = local_client(&ids, &handles, &Behavior::default());
        assert_eq!(
            client.weapon_hud_view().expect("hud"),
            Q3WeaponHudView {
                visible: true,
                aggregate_warning: true,
            }
        );
        client.cvars().borrow_mut().set("cg_draw2D", "0", true).expect("set");
        assert!(!client.weapon_hud_view().expect("hud").visible);

        let handles = make_handles(&ids);
        let behavior = Behavior {
            hud: Q3HudState {
                level_shot: true,
                show_scores: false,
                player: Some(Q3HudPlayer {
                    health: 100,
                    movement_type: MoveType::PmNormal,
                    team: Team::TeamRed,
                }),
            },
            ..Behavior::default()
        };
        let client = local_client(&ids, &handles, &behavior);
        assert!(!client.weapon_hud_view().expect("hud").visible);

        let handles = make_handles(&ids);
        let surface = remote_surface(&ids);
        let behavior = Behavior {
            qvm_equipment_hud: true,
            qvm_equipment_visible: true,
            ..Behavior::default()
        };
        let client = ApplicationQ3Client::create(make_qvm(&ids, &handles, &behavior, surface), None).expect("create");
        assert_eq!(
            client.weapon_hud_view().expect("hud"),
            Q3WeaponHudView {
                visible: true,
                aggregate_warning: false,
            }
        );
        assert_eq!(client.shared_equipment_view_visible().expect("view"), Some(true));
        assert!(client.shared_held_weapons().expect("held").is_empty());
    }

    #[test]
    fn user_command_selection_prefers_equipment() {
        let (_owner, ids) = ids();
        let handles = make_handles(&ids);
        let surface = remote_surface(&ids);
        let behavior = Behavior {
            equipment: Some(7),
            ..Behavior::default()
        };
        let client = ApplicationQ3Client::create(make_qvm(&ids, &handles, &behavior, surface), None).expect("create");
        let session = handles.captured_qvm.borrow().session.clone().expect("session");
        (session.set_user_command_value)(3, 0.5);
        assert_eq!(
            client.user_command_selection(),
            Q3UserCommandSelection {
                weapon: 7,
                sensitivity: 0.5,
            }
        );

        let handles = make_handles(&ids);
        let client = local_client(&ids, &handles, &Behavior::default());
        assert_eq!(
            client.user_command_selection(),
            Q3UserCommandSelection {
                weapon: 2,
                sensitivity: 1.0,
            }
        );
    }

    #[test]
    fn remote_session_probes_hit_the_surface() {
        let (_owner, ids) = ids();
        let handles = make_handles(&ids);
        let surface = remote_surface(&ids);
        surface.borrow_mut().game_state = vec!["a".to_string(), "b".to_string()];
        surface.borrow_mut().commands.insert(9, vec!["cs".to_string()]);
        surface.borrow_mut().ping = Some(42);
        let client = ApplicationQ3Client::create(
            make_remote(&ids, &handles, &Behavior::default(), surface.clone(), test_player()),
            None,
        )
        .expect("create");
        let session = handles.captured.borrow().session.clone().expect("session");
        assert_eq!(session.client_number, 2);
        assert_eq!(session.server_message_sequence, 11);
        assert_eq!(session.last_executed_server_command, 9);
        assert_eq!((session.game_state)(), vec!["a".to_string(), "b".to_string()]);
        assert_eq!((session.server_command)(9), Some(vec!["cs".to_string()]));
        assert_eq!((session.snapshot_ping)(4), 42);
        assert_eq!((session.server_time)(), 500);
        assert_eq!(session.snapshots.borrow().current().number, 3);
        assert_eq!(session.commands.borrow().current_number(), 7);
        surface.borrow_mut().ping = None;
        assert_eq!((session.snapshot_ping)(4), 0);
        drop(client);
    }

    #[test]
    fn supplemental_weapon_camera_skips_qvm() {
        let (_owner, ids) = ids();
        let handles = make_handles(&ids);
        let surface = remote_surface(&ids);
        let client =
            ApplicationQ3Client::create(make_qvm(&ids, &handles, &Behavior::default(), surface), None).expect("create");
        let camera = client.camera();
        assert_eq!(client.supplemental_weapon_camera(&camera).expect("camera"), camera);
    }

    #[test]
    fn frame_submits_scenes_commands_and_effects() {
        let (_owner, ids) = ids();
        let handles = make_handles(&ids);
        let mut client = local_client(&ids, &handles, &Behavior::default());
        let output = handles.services_output.borrow().clone().expect("output");
        let camera = client.camera();
        let mut world = test_scene(camera, 0);
        world.lights = vec![
            Q3SceneLight {
                origin: vec3(0.0, 0.0, 0.0),
                radius: 300.0,
                color: vec3(1.0, 1.0, 1.0),
                additive: false,
            };
            40
        ];
        world.source.area_mask[0] = 0xFF;
        let direct = test_scene(camera, RDF_NOWORLDMODEL);
        (output.scene)(world);
        (output.scene)(direct);
        (output.command)(RenderCommand::SetColor(Vec4 {
            x: 1.0,
            y: 1.0,
            z: 1.0,
            w: 1.0,
        }));
        let shifted = camera;
        let frame = client
            .frame(
                Some(&|_| ApplicationEffectFrame {
                    q3_admissions: Vec::new(),
                    operations: Vec::new(),
                    lights: Vec::new(),
                    q3_lights: Vec::new(),
                }),
                Some(&|camera| {
                    let mut moved = *camera;
                    moved.origin.x += 5.0;
                    moved
                }),
                &Q3FrameEnvironment {
                    no_world_model: true,
                    ..Q3FrameEnvironment::default()
                },
                Some(vec3(0.0, 0.0, 8.0)),
            )
            .expect("frame");
        assert_eq!(frame.sequence, 7);
        let pipeline = handles.pipeline.borrow();
        assert_eq!(*pipeline.began.borrow(), 1);
        assert_eq!(pipeline.commands.borrow().len(), 1);
        assert_eq!(*pipeline.direct.borrow(), 1);
        assert_eq!(*pipeline.worlds.borrow(), 1);
        assert_eq!(*pipeline.finished.borrow(), vec![false]);
        let direct_view = &pipeline.direct_views.borrow()[0];
        assert_eq!(direct_view.camera.origin, shifted.origin);
        assert!(direct_view.clear.color.is_none());
        let world_view = &pipeline.world_views.borrow()[0];
        assert_eq!(world_view.camera.origin.x, shifted.origin.x + 5.0);
        assert_eq!(world_view.q3_lights.len(), 32);
        assert!(!world_view.visible_areas.contains(&0));
        assert!(world_view.visible_areas.contains(&8));
        assert_eq!(world_view.clear.color, Some(vec3(0.3, 0.3, 0.3)));
        assert!(world_view.no_world_model);
    }

    #[test]
    fn close_drains_clear_loops_and_is_idempotent() {
        let (_owner, ids) = ids();
        let handles = make_handles(&ids);
        let mut client = local_client(&ids, &handles, &Behavior::default());
        assert!(client.resource_diagnostics().is_ok());
        client.close();
        client.close();
        let frames = handles.audio.frames.borrow();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0], vec![Q3SeatAudioOperation::ClearLoops { kill_all: true }]);
        assert!(*handles.registration.closed.borrow());
        assert!(*handles.media.closed.borrow());
        assert!(*handles.pipeline.borrow().closed.borrow());
        assert!(handles.native_calls.borrow().closed);
        assert!(client.command(&["x".to_string()]).is_err());
        assert!(client.refresh_system_info().is_err());
        assert!(client.resource_diagnostics().is_err());
        assert!(client.frame(None, None, &Q3FrameEnvironment::default(), None).is_err());
    }

    #[test]
    fn shutdown_runs_qvm_shutdown() {
        let (_owner, ids) = ids();
        let handles = make_handles(&ids);
        let surface = remote_surface(&ids);
        let mut client =
            ApplicationQ3Client::create(make_qvm(&ids, &handles, &Behavior::default(), surface), None).expect("create");
        client.shutdown().expect("shutdown");
        assert_eq!(handles.qvm_calls.borrow().shutdowns, 1);
        assert!(handles.qvm_calls.borrow().closed);

        let handles = make_handles(&ids);
        let mut client = local_client(&ids, &handles, &Behavior::default());
        client.shutdown().expect("shutdown");
    }

    #[test]
    fn capture_video_reopen_round_trips_qvm() {
        let (_owner, ids) = ids();
        let handles = make_handles(&ids);
        let surface = remote_surface(&ids);
        let mut client =
            ApplicationQ3Client::create(make_qvm(&ids, &handles, &Behavior::default(), surface), None).expect("create");
        client
            .cvars()
            .borrow_mut()
            .set("reopen_marker", "kept", true)
            .expect("set");
        let mut reopen = client.capture_video_reopen().expect("capture");
        let overrides = || QvmVideoReopenOptions {
            renderer: QvmDisplayRenderer {
                stencil_bits: 8,
                driver: None,
                gl_config: None,
            },
            viewport: Rc::new(test_viewport),
            command_registration: handles.registration.clone(),
        };
        assert!(reopen(overrides()).is_err());
        client.close();
        drop(client);
        let reopened = reopen(overrides()).expect("reopen");
        assert_eq!(
            reopened.cvars().borrow().get("reopen_marker").expect("marker").value,
            "kept"
        );
        assert!(reopen(overrides()).is_err());

        let handles = make_handles(&ids);
        let client = local_client(&ids, &handles, &Behavior::default());
        assert!(client.capture_video_reopen().is_err());
    }

    #[test]
    fn quake_mouse_button_mapping() {
        assert_eq!(quake_mouse_button(1), 1);
        assert_eq!(quake_mouse_button(2), 3);
        assert_eq!(quake_mouse_button(3), 2);
        assert_eq!(quake_mouse_button(4), 4);
    }

    #[test]
    fn create_rejects_missing_local_player() {
        let (owner, ids) = ids();
        let handles = make_handles(&ids);
        let stranger = owner.actor(9, 1);
        let initial = local_initial(&stranger, 0, vec3(0.0, 0.0, 0.0), 0, vec3(0.0, 0.0, 0.0));
        let error = ApplicationQ3Client::create(make_local(&ids, &handles, &Behavior::default(), initial), None)
            .err()
            .expect("create");
        assert_eq!(error.to_string(), "Q3 cgame seat lacks a source player");
    }

    #[test]
    fn cgame_rejects_qvm_backend() {
        let (_owner, ids) = ids();
        let handles = make_handles(&ids);
        let surface = remote_surface(&ids);
        let mut client =
            ApplicationQ3Client::create(make_qvm(&ids, &handles, &Behavior::default(), surface), None).expect("create");
        assert!(client.cgame().is_err());

        let handles = make_handles(&ids);
        let mut client = local_client(&ids, &handles, &Behavior::default());
        assert!(client.cgame().is_ok());
    }

    #[test]
    fn native_params_carry_light_memory_and_hardware() {
        let (_owner, ids) = ids();
        let handles = make_handles(&ids);
        let behavior = Behavior {
            renderer_name: Some("ATI Rage Pro".to_string()),
            ..Behavior::default()
        };
        let client = local_client(&ids, &handles, &behavior);
        let captured = handles.captured.borrow();
        assert_eq!(captured.ragepro, Some(true));
        let memory = captured.memory.clone().expect("memory");
        assert_eq!(memory(), 0x7FFF_FFFF);
        let light = captured.light.clone().expect("light");
        let sample = light(vec3(0.0, 0.0, 0.0));
        assert_eq!(sample.ambient, vec3(255.0, 255.0, 255.0));
        assert_eq!(sample.directed, vec3(0.0, 0.0, 0.0));
        assert_eq!(sample.direction, vec3(0.0, 0.0, 1.0));
        let movement = captured.movement.clone().expect("movement");
        assert_eq!(movement.borrow().command_timing(), CommandTiming::Q3);
        drop(client);
    }

    #[test]
    fn services_viewport_and_remap_follow_lifecycle() {
        let (_owner, ids) = ids();
        let handles = make_handles(&ids);
        let mut client = local_client(&ids, &handles, &Behavior::default());
        let viewport = handles.services_viewport.borrow().clone().expect("viewport");
        assert_eq!(viewport(), test_viewport());
        let retarget = Rect {
            x: 1,
            y: 2,
            width: 3,
            height: 4,
        };
        client
            .prepare(1, retarget, &[], true, &[], true, false)
            .expect("prepare");
        assert_eq!(viewport(), retarget);
        let remap = handles.services_remap.borrow().clone().expect("remap");
        remap("a", "b", "c").expect("remap");
        assert_eq!(
            *handles.remaps.borrow(),
            vec![("a".to_string(), "b".to_string(), "c".to_string(), false, true)]
        );
    }

    #[test]
    fn settings_and_server_settings_seed_shared_cvars() {
        let (_owner, ids) = ids();
        let handles = make_handles(&ids);
        let behavior = Behavior {
            settings: vec![("seeded_setting".to_string(), "yes".to_string())],
            server_settings: vec![
                ("cg_draw2D".to_string(), "0".to_string()),
                ("unshared_var".to_string(), "1".to_string()),
            ],
            ..Behavior::default()
        };
        let surface = remote_surface(&ids);
        let client = ApplicationQ3Client::create(make_remote(&ids, &handles, &behavior, surface, test_player()), None)
            .expect("create");
        let cvars = client.cvars();
        let cvars = cvars.borrow();
        assert_eq!(cvars.get("seeded_setting").expect("seeded").value, "yes");
        assert_eq!(cvars.get("cg_draw2D").expect("shared").value, "0");
        assert!(cvars.get("unshared_var").is_none());
    }

    #[test]
    fn transport_entity_round_trips_presentation_fields() {
        let source = EntityState {
            number: 9,
            e_type: 2,
            e_flags: 4,
            pos: Trajectory {
                trajectory_type: TrajectoryType::TrLinear,
                time: 100,
                duration: 50,
                base: vec3(1.0, 2.0, 3.0),
                delta: vec3(4.0, 5.0, 6.0),
            },
            apos: Trajectory {
                trajectory_type: TrajectoryType::TrStationary,
                time: 0,
                duration: 0,
                base: vec3(0.0, 0.0, 0.0),
                delta: vec3(0.0, 0.0, 0.0),
            },
            time: 7,
            time2: 8,
            origin: vec3(10.0, 20.0, 30.0),
            origin2: vec3(11.0, 21.0, 31.0),
            angles: vec3(0.0, 90.0, 0.0),
            angles2: vec3(0.0, 0.0, 0.0),
            other_entity_num: 3,
            other_entity_num2: 4,
            ground_entity_num: 5,
            constant_light: 6,
            loop_sound: 7,
            modelindex: 8,
            modelindex2: 9,
            client_num: 1,
            frame: 12,
            solid: 13,
            event: 14,
            event_parm: 15,
            powerups: 16,
            weapon: 17,
            legs_anim: 18,
            torso_anim: 19,
            generic1: 20,
        };
        let network = transport_entity(&source);
        assert_eq!(network.number, 9);
        assert_eq!(network.pos.trajectory_type, TrajectoryType::TrLinear as i32);
        assert_eq!(network.origin, [10.0, 20.0, 30.0]);
        assert_eq!(network.angles, [0.0, 90.0, 0.0]);
        let back = retail_entity(&network);
        assert_eq!(back.number, source.number);
        assert_eq!(back.e_type, source.e_type);
        assert_eq!(back.pos.trajectory_type, source.pos.trajectory_type);
        assert_eq!(back.origin, source.origin);
        assert_eq!(back.angles, source.angles);
        assert_eq!(back.weapon, source.weapon);
        assert_eq!(back.generic1, source.generic1);
    }

    #[test]
    fn local_source_selects_through_ported_visibility() {
        let (_owner, ids) = ids();
        let handles = make_handles(&ids);
        handles.actor_map.borrow_mut().insert(5, ids.actor.clone());
        handles.link_bounds.borrow_mut().insert(
            5,
            Bounds {
                min: vec3(-8.0, -8.0, -8.0),
                max: vec3(8.0, 8.0, 8.0),
            },
        );
        let mut client = local_client(&ids, &handles, &Behavior::default());
        let mut state = local_initial(&ids.actor, 0, vec3(10.0, 20.0, 30.0), 26, vec3(0.0, 90.0, 0.0));
        state.time = 100;
        state.entities.push(Q3SourcePresentationEntity {
            actor: ids.actor.clone(),
            state: EntityState {
                number: 5,
                ..Default::default()
            },
            origin: vec3(0.0, 0.0, 0.0),
            linked: true,
            server_flags: 0,
            single_client: 0,
        });
        client.receive(&state, &[], &[]).expect("receive");
        let session = handles.captured.borrow().session.clone().expect("session");
        let snapshot = session.snapshots.borrow_mut().read(2).expect("read").expect("snapshot");
        assert_eq!(snapshot.entities.len(), 1);
        assert_eq!(snapshot.entities[0].number, 5);
    }

    #[test]
    fn frame_merges_canonical_effect_frame() {
        let (_owner, ids) = ids();
        let handles = make_handles(&ids);
        let mut client = local_client(&ids, &handles, &Behavior::default());
        let output = handles.services_output.borrow().clone().expect("output");
        (output.scene)(test_scene(client.camera(), 0));
        client
            .frame(
                Some(&|_| ApplicationEffectFrame {
                    q3_admissions: Vec::new(),
                    operations: Vec::new(),
                    lights: Vec::new(),
                    q3_lights: vec![DynamicLight {
                        origin: vec3(1.0, 2.0, 3.0),
                        radius: 100.0,
                        color: vec3(1.0, 0.5, 0.0),
                        additive: true,
                    }],
                }),
                None,
                &Q3FrameEnvironment::default(),
                None,
            )
            .expect("frame");
        let pipeline = handles.pipeline.borrow();
        let views = pipeline.world_views.borrow();
        assert_eq!(views.len(), 1);
        assert_eq!(
            views[0].q3_lights,
            vec![Q3SceneLight {
                origin: vec3(1.0, 2.0, 3.0),
                radius: 100.0,
                color: vec3(1.0, 0.5, 0.0),
                additive: true,
            }]
        );
    }
}
