//! Local-seat presentation: cameras, layout, messages, and frame assembly.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/presentation.ts`
//! (`seatViewport`, `cameraWithKick`, `cameraWithClientOffset`,
//! `cameraWithCharacterDeath`, `WorldSeatPresentation`). Directly reused ports:
//! the world scene (behind [`SeatScene`], which the host forwards to
//! [`ApplicationWorldScene`](super::presentation_scene::ApplicationWorldScene)),
//! [`Q1MapFog`](super::q1_fog::Q1MapFog),
//! [`Q1ServicePresentation`](super::q1_service_presentation::Q1ServicePresentation),
//! [`Q1MessageLocalization`](super::q1_localization::Q1MessageLocalization),
//! [`ApplicationQ2NativeHud`](super::q2_native_hud::ApplicationQ2NativeHud), the Q1
//! view helpers, [`weapon_view_camera`](super::weapon_view::weapon_view_camera), and
//! [`prepare_q2_damage_blend`](super::q2_damage_blend::prepare_q2_damage_blend).
//! Everything else (simulation, local input, native renderer, assets beyond the Q1
//! reads, effects, seat UI, finale, component drawings, fonts, the frame builder,
//! 2D drawing, debug shapes, and the console) arrives through [`SeatHost`]; the Q3
//! client through [`SeatQ3Client`]; content reads through [`SeatContent`].
//! Documented folds: async preparation is sync through the host; the Q3 client's
//! frame callbacks are `FnMut` (the donor closures capture `this` mutably); the
//! component-effect unbind closure is a numeric handle; overlay draws receive draw
//! context instead of the live frame builder (the host submits through its own
//! captured frame handle); the UI snapshot and debug lines are host-opaque; the
//! chase numeric-profile lookup is a host query; the view-size `fov` math keeps the
//! donor's `atan` form; unparseable server commands fall back to whitespace
//! splitting; the scene observes owner and lightstyle events only, so the remaining
//! seat events map to its foreign case; `renderOwner` is a boolean (its only donor
//! value is `"source-client"`); the message language is sampled once per prepare.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use qa_client::materials::fog::Q1FogTransition;
use qa_client::materials::lighting::SurfaceDynamicLight;
use qa_client::materials::q3_lighting::DynamicLight;
use qa_client::render::scene::material_registrations::ShaderRegistration;
use qa_client::render::scene::submissions::{SceneGroupOrder, SceneOperation, SourceEntityOrder, SourceSceneOrder};
use qa_client::render::scene::visibility::WorldVisibilityOptions;
use qa_client::render::scene::world::{Q1FogParams, WorldSurfaceAdmission, WorldViewInput};
use qa_client::render::types::SceneFog;
use qa_client::render::types::{
    DrawBatch, RenderCommand, RenderFrame, RendererImage, SourceTime as RenderSourceTime, ViewClear, ViewTarget,
};
use qa_client::render::SceneLight;
use qa_client::ui::hud::q2_native::NativeQ2HudFrame;
use qa_client::view::{perspective_projection, Rect, SceneCamera};
use qa_content::contract::{ContentId, GameFamily, PresentationOwner, PresentationSelection};
use qa_core::cmd::{tokenize_command, Dialect, TextMode};
use qa_core::identity::{ActorId, ClientId, SeatId};
use qa_core::math::{angles_to_axis, vec3, Vec3, Vec4};
use qa_core::time::SourceTime;
use qa_world::session::WorldSnapshot;
use thiserror::Error;

use super::presentation_scene::{
    ApplicationWorldScene, SceneBody, SceneError, SceneHeldWeapon, ScenePrepareInput, ScenePresentation,
    ScenePrimaryBody,
};
use super::presentation_state::{
    OwnerLifecycle, Q1PresentationEvent, Q2PresentationEvent, SimulationPresentationEvent,
};
use super::q1_client_settings::{
    q1_chase_camera, q1_view_camera, q1_view_rectangle, Q1ChaseScene, Q1ChaseSettings, Q1ViewSettings,
};
use super::q1_fog::{Q1MapFog, Q1MapFogEvent};
use super::q1_localization::{Q1MessageArg, Q1MessageAssets, Q1MessageLocalization, Q1MessagePart};
use super::q1_service_presentation::{
    Q1ClientMetadataEvent, Q1ServiceAssets, Q1ServiceClosed, Q1ServiceEvent, Q1ServicePresentation, Q1SkyRefresh,
};
use super::q2_damage_blend::{prepare_q2_damage_blend, DAMAGE_BLEND_BORDER};
use super::q2_native_hud::ApplicationQ2NativeHud;
use super::weapon_view::weapon_view_camera;

/// Seat presentation failure, with donor messages.
#[derive(Debug, Error)]
pub enum SeatError {
    /// Invalid local seat layout.
    #[error("Invalid local seat layout")]
    Layout,
    /// Chase camera requires the selected numeric profile.
    #[error("Chase camera requires the selected numeric profile")]
    ChaseTiming,
    /// Component client presentation is retired.
    #[error("Component client presentation is retired")]
    ComponentRetired,
    /// Replacement presentation belongs to another local player.
    #[error("Replacement presentation belongs to another local player")]
    ReplacementPlayer,
    /// Shared view lost its prepared effects.
    #[error("Shared view lost its prepared effects")]
    MissingEffects,
    /// Cgame cannot present the shared framebuffer.
    #[error("Cgame cannot present the shared framebuffer")]
    SwapBuffers,
    /// World text font was not prepared.
    #[error("World text font was not prepared")]
    WorldTextFont,
    /// Seat presentation uses another renderer.
    #[error("Seat presentation uses another renderer")]
    ForeignRenderer,
    /// Failed to close seat presentation.
    #[error("Failed to close seat presentation: {0:?}")]
    Close(Vec<String>),
    /// Q1 service presentation is closed.
    #[error("Q1 service presentation is closed")]
    ServicesClosed,
    /// World scene failure.
    #[error(transparent)]
    Scene(#[from] SceneError),
    /// Camera projection failure.
    #[error("Seat camera projection failed: {0}")]
    Camera(String),
    /// Fog parse failure.
    #[error("Seat fog failed: {0}")]
    Fog(String),
    /// Host failure.
    #[error("Seat host failed: {0}")]
    Host(String),
}

impl From<Q1ServiceClosed> for SeatError {
    fn from(_: Q1ServiceClosed) -> Self {
        SeatError::ServicesClosed
    }
}

/// Viewport for one local seat of a split screen (donor `seatViewport`).
pub fn seat_viewport(index: u32, count: u32, width: i32, height: i32) -> Result<Rect, SeatError> {
    if !(1..=4).contains(&count) || index >= count {
        return Err(SeatError::Layout);
    }
    if count == 1 {
        return Ok(Rect {
            x: 0,
            y: 0,
            width,
            height,
        });
    }
    let columns = if count == 2 { 1 } else { 2 };
    let column = index % columns;
    let row = index / columns;
    let x = (column * width as u32 / columns) as i32;
    let y = (row * height as u32 / 2) as i32;
    Ok(Rect {
        x,
        y,
        width: ((column + 1) * width as u32 / columns) as i32 - x,
        height: ((row + 1) * height as u32 / 2) as i32 - y,
    })
}

/// Rotate a camera by kick angles (donor `cameraWithKick`).
#[must_use]
pub fn camera_with_kick(camera: &SceneCamera, kick: Vec3) -> SceneCamera {
    if kick.x == 0.0 && kick.y == 0.0 && kick.z == 0.0 {
        return *camera;
    }
    let local = angles_to_axis(kick);
    let axis = camera.axis;
    let rotate = |v: Vec3| Vec3 {
        x: axis[0].x * v.x + axis[1].x * v.y + axis[2].x * v.z,
        y: axis[0].y * v.x + axis[1].y * v.y + axis[2].y * v.z,
        z: axis[0].z * v.x + axis[1].z * v.y + axis[2].z * v.z,
    };
    SceneCamera {
        axis: [rotate(local[0]), rotate(local[1]), rotate(local[2])],
        ..*camera
    }
}

/// Shift a camera by the client view offset (donor `cameraWithClientOffset`).
#[must_use]
pub fn camera_with_client_offset(camera: &SceneCamera, player: &SeatPlayerView) -> SceneCamera {
    let Some(delta) = player.client_view_offset_delta else {
        return *camera;
    };
    if !matches!(camera.clip, qa_client::view::CameraClip::None) {
        return *camera;
    }
    SceneCamera {
        origin: vec3(
            camera.origin.x + delta.x,
            camera.origin.y + delta.y,
            camera.origin.z + delta.z,
        ),
        ..*camera
    }
}

/// Snap a camera to a dead character's eyes (donor `cameraWithCharacterDeath`).
#[must_use]
pub fn camera_with_character_death(camera: &SceneCamera, player: &SeatPlayerView) -> SceneCamera {
    if !player.foreign_character_death || !matches!(camera.clip, qa_client::view::CameraClip::None) {
        return *camera;
    }
    SceneCamera {
        origin: vec3(player.origin.x, player.origin.y, player.origin.z + player.view_height),
        axis: angles_to_axis(player.angles),
        ..*camera
    }
}

/// Vertical field of view for a horizontal one (donor `fovY` form).
fn vertical_fov(fov_x: f32, viewport: Rect) -> f32 {
    ((f64::from(viewport.height) / f64::from(viewport.width) * (f64::from(fov_x) * std::f64::consts::PI / 360.0).tan())
        .atan()
        * 360.0
        / std::f64::consts::PI) as f32
}

/// Player view reads (donor `PlayerView` subset).
#[derive(Debug, Clone, PartialEq)]
pub struct SeatPlayerView {
    /// World origin.
    pub origin: Vec3,
    /// World angles.
    pub angles: Vec3,
    /// View height above the origin.
    pub view_height: f32,
    /// Horizontal field of view override.
    pub field_of_view: Option<f32>,
    /// Kick angles.
    pub kick_angles: Option<Vec3>,
    /// Client view offset delta.
    pub client_view_offset_delta: Option<Vec3>,
    /// Whether a foreign character died.
    pub foreign_character_death: bool,
    /// Full-screen blend color.
    pub blend: Option<Vec4>,
    /// Damage blend color.
    pub damage_blend: Option<Vec4>,
}

/// World text reads (donor `WorldText` subset: only content is read here).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeatWorldText {
    /// Text content.
    pub content: ContentId,
}

/// Seat event (donor `SimulationPresentationEvent` subset).
#[derive(Debug, Clone, PartialEq)]
pub struct SeatSourceEvent {
    /// Event payload.
    pub event: SeatEvent,
    /// Presenting owner.
    pub owner: Option<PresentationOwner>,
    /// Per-actor recipient.
    pub recipient: Option<ActorId>,
    /// Presentation sequence.
    pub sequence: i64,
    /// Content.
    pub content: ContentId,
    /// Seconds.
    pub seconds: f64,
    /// Source entity slot.
    pub source_entity: Option<i32>,
}

/// Seat event payload (donor event kinds read by this module).
#[derive(Debug, Clone, PartialEq)]
pub enum SeatEvent {
    /// Owner lifecycle.
    Owner {
        /// Lifecycle.
        lifecycle: OwnerLifecycle,
    },
    /// Lightstyle (both families; the scene treats them alike).
    Lightstyle {
        /// Game family.
        family: GameFamily,
        /// Style number.
        style: i32,
        /// Pattern.
        pattern: String,
    },
    /// Quake message.
    Q1Message {
        /// Target player.
        player: ActorId,
        /// Message text.
        text: String,
        /// Message arguments.
        args: Vec<Q1MessageArg>,
        /// Message parts.
        parts: Vec<Q1MessagePart>,
        /// Whether center-printed.
        center: bool,
    },
    /// Quake player teleport.
    Q1Teleport {
        /// Teleported player.
        player: ActorId,
        /// View angles.
        angles: Vec3,
    },
    /// Quake sky selection.
    Q1Sky {
        /// Skybox name.
        name: String,
    },
    /// Quake client metadata.
    Q1Client(Q1ClientMetadataEvent),
    /// Quake fog transition.
    Q1Fog {
        /// Target player.
        player: Option<ActorId>,
        /// Installed transition.
        transition: Q1FogTransition,
        /// Sky blend factor.
        sky_factor: f32,
    },
    /// Any other Quake event (passes through to the seat UI).
    Q1Other {
        /// Event kind.
        kind: String,
    },
    /// Quake II help text.
    Q2Help {
        /// Help text.
        text: String,
    },
    /// Quake II pickup.
    Q2Pickup {
        /// Picking player.
        player: ActorId,
        /// Item name.
        name: String,
    },
    /// Quake II print.
    Q2Print {
        /// Owning actor.
        actor: Option<ActorId>,
        /// Print text.
        text: String,
    },
    /// Quake II center print.
    Q2Centerprint {
        /// Owning actor.
        actor: Option<ActorId>,
        /// Print text.
        text: String,
    },
    /// Any other Quake II event (passes through to the seat UI).
    Q2Other {
        /// Event kind.
        kind: String,
    },
    /// Quake II player print.
    Q2PlayerPrint {
        /// Target actor.
        target: Option<ActorId>,
        /// Print text.
        text: String,
    },
    /// Any other Quake II player event (passes through to the seat UI).
    Q2PlayerOther {
        /// Event kind.
        kind: String,
    },
    /// Quake III server command.
    Q3ServerCommand {
        /// Target client, negative for broadcast.
        client: i32,
        /// Command text.
        text: String,
    },
    /// Any other Quake III source event.
    Q3Other {
        /// Event kind.
        kind: String,
    },
    /// View reset.
    ViewReset {
        /// Reset actor.
        actor: ActorId,
        /// View angles.
        angles: Vec3,
    },
    /// Any other family.
    Foreign {
        /// Family kind.
        kind: String,
    },
}

impl SeatEvent {
    /// Donor family kind.
    fn family_kind(&self) -> &'static str {
        match self {
            SeatEvent::Owner { .. } => "presentation-owner",
            SeatEvent::Lightstyle { .. } => "lightstyle",
            SeatEvent::Q1Message { .. }
            | SeatEvent::Q1Teleport { .. }
            | SeatEvent::Q1Sky { .. }
            | SeatEvent::Q1Client(_)
            | SeatEvent::Q1Fog { .. }
            | SeatEvent::Q1Other { .. } => "q1",
            SeatEvent::Q2Help { .. }
            | SeatEvent::Q2Pickup { .. }
            | SeatEvent::Q2Print { .. }
            | SeatEvent::Q2Centerprint { .. }
            | SeatEvent::Q2Other { .. } => "q2",
            SeatEvent::Q2PlayerPrint { .. } | SeatEvent::Q2PlayerOther { .. } => "q2-player",
            SeatEvent::Q3ServerCommand { .. } | SeatEvent::Q3Other { .. } => "q3-source",
            SeatEvent::ViewReset { .. } => "view-reset",
            SeatEvent::Foreign { .. } => "foreign",
        }
    }
}

/// Simulation message event (donor `SimulationEvent` message subset).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeatSimulationEvent {
    /// Message text, when the payload carries a string.
    pub message_text: Option<String>,
    /// Source presentation sequence.
    pub source_sequence: Option<i64>,
}
/// Seat presentation with its render owner (donor `SimulationPresentation` subset).
#[derive(Debug, Clone, PartialEq)]
pub struct SeatPresentation {
    /// Scene presentation.
    pub scene: ScenePresentation,
    /// Whether the source client owns rendering.
    pub render_owner_source_client: bool,
}

/// Effect frame (donor `ApplicationEffectFrame`).
#[derive(Debug, Clone, PartialEq)]
pub struct SeatEffectFrame {
    /// Q3 admissions.
    pub q3_admissions: Vec<WorldSurfaceAdmission>,
    /// Operations.
    pub operations: Vec<SceneOperation>,
    /// Surface lights.
    pub lights: Vec<SurfaceDynamicLight>,
    /// Q3 dynamic lights.
    pub q3_lights: Vec<DynamicLight>,
}

impl SeatEffectFrame {
    /// Empty frame.
    pub fn empty() -> Self {
        Self {
            q3_admissions: Vec::new(),
            operations: Vec::new(),
            lights: Vec::new(),
            q3_lights: Vec::new(),
        }
    }
}

/// Component effect frame (donor `ComponentEffectFrame`).
pub type SeatComponentEffect = Rc<dyn Fn(&SceneCamera, &SourceSceneOrder, Option<&SceneFog>) -> SeatEffectFrame>;

/// Component overlay draw context (the host submits through its own frame handle).
#[derive(Debug, Clone, PartialEq)]
pub struct SeatOverlayContext {
    /// View camera.
    pub camera: SceneCamera,
    /// Seat viewport.
    pub viewport: Rect,
    /// Presenting seat.
    pub seat: SeatId,
    /// Prepared time in milliseconds.
    pub time_ms: f64,
}

/// Component overlay (donor `bindComponentEffects` overlay).
#[derive(Clone)]
pub struct SeatComponentOverlay {
    /// Owning activation.
    pub owner: PresentationOwner,
    /// Component bodies.
    pub bodies: Option<Rc<dyn Fn() -> Vec<SceneBody>>>,
    /// Overlay draw.
    pub draw: Rc<dyn Fn(&SeatOverlayContext)>,
}

/// Effect player view (donor `effects.playerView` result subset).
#[derive(Debug, Clone, PartialEq)]
pub struct SeatEffectPlayerView {
    /// Adjusted camera.
    pub camera: SceneCamera,
    /// Whether infrared is active.
    pub infrared: bool,
    /// Full-screen blend color.
    pub blend: Option<Vec4>,
}

/// Mod client source (donor `modClientPresentationSources` entry).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeatModClientSource {
    /// Host source handle.
    pub id: u64,
    /// Owning activation.
    pub owner: PresentationOwner,
    /// Source generation.
    pub generation: u64,
    /// Identity content.
    pub identity_content: ContentId,
}

/// Mod client kind (donor frame `kind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeatModKind {
    /// QVM client.
    Qvm,
    /// QuakeC client.
    Quakec,
    /// Native client.
    Native,
}

/// Mod HUD mode (donor `mode`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SeatHudMode {
    /// Overlay HUD.
    Overlay,
    /// Layout overlay HUD.
    LayoutOverlay,
    /// Status replacement HUD.
    ReplaceStatus,
}

/// Mod client HUD (donor frame `hud`).
#[derive(Debug, Clone, PartialEq)]
pub enum SeatModHud {
    /// QVM HUD mode.
    Qvm {
        /// HUD mode.
        mode: SeatHudMode,
    },
    /// QuakeC vitals.
    Quakec {
        /// Health.
        health: i32,
        /// Armor.
        armor: i32,
    },
    /// Native HUD frame.
    Native {
        /// HUD mode.
        mode: SeatHudMode,
        /// HUD frame.
        frame: NativeQ2HudFrame,
    },
}

impl SeatModHud {
    /// Whether this HUD replaces the status bar.
    fn replaces_status(&self) -> bool {
        match self {
            SeatModHud::Qvm { mode } | SeatModHud::Native { mode, .. } => *mode == SeatHudMode::ReplaceStatus,
            SeatModHud::Quakec { .. } => false,
        }
    }
}

/// Mod client frame (donor `ModClientPresentationFrame`).
#[derive(Debug, Clone, PartialEq)]
pub struct SeatModClientFrame {
    /// Client kind.
    pub kind: SeatModKind,
    /// Whether the frame carries a camera view.
    pub has_view: bool,
    /// Native flags, when the frame is native.
    pub native: Option<SeatNativeFlags>,
    /// HUD, when present.
    pub hud: Option<SeatModHud>,
}

/// Native camera flags (donor `NativeModCameraView["native"]` subset).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeatNativeFlags {
    /// Whether the weapon stays visible.
    pub weapon_visible: bool,
    /// Render flags.
    pub render_flags: i32,
}

/// Seat presentation binding (donor `SeatPresentationBinding`).
#[derive(Debug, Clone, PartialEq)]
pub struct SeatPresentationBinding {
    /// Presenting seat.
    pub seat: SeatId,
    /// Presenting client.
    pub client: ClientId,
    /// Seat viewport.
    pub viewport: Rect,
    /// Safe area (always the viewport here).
    pub safe_area: Rect,
    /// HUD scale.
    pub hud_scale: f32,
    /// Presentation selection.
    pub presentation: PresentationSelection,
}

/// Seat UI data (donor `state.ui`: the host merges controller and message state).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeatUiData<U> {
    /// Host UI snapshot.
    pub snapshot: U,
    /// Whether the console holds focus.
    pub focus_console: bool,
    /// Whether scores are requested.
    pub show_scores: bool,
}

/// Seat client state (donor `SeatClientState`).
#[derive(Debug, Clone, PartialEq)]
pub struct SeatClientState<U> {
    /// Presenting seat.
    pub seat: SeatId,
    /// Presenting client.
    pub client: ClientId,
    /// Player actor.
    pub actor: ActorId,
    /// UI state.
    pub ui: SeatUiData<U>,
    /// Presentation binding.
    pub presentation: SeatPresentationBinding,
}

/// UI draw arguments (donor `ui.draw` parameters).
#[derive(Debug, Clone, PartialEq)]
pub struct SeatUiDrawArgs {
    /// Presentation binding.
    pub binding: SeatPresentationBinding,
    /// Prepared time in milliseconds.
    pub time_ms: f64,
    /// View camera.
    pub camera: SceneCamera,
    /// Whether the weapon HUD is visible.
    pub weapon_hud_visible: bool,
    /// Whether the story overlay is suppressed.
    pub story_suppressed: bool,
    /// Whether native HUD rules apply.
    pub native_hud: bool,
    /// Whether aggregate warnings show.
    pub aggregate_warning: bool,
    /// Whether a Q3 client drives the seat.
    pub is_q3: bool,
    /// QuakeC status HUD, when one drives vitals.
    pub qc_status: Option<SeatQcHud>,
}

/// QuakeC status HUD (donor `{ kind: "vitals", ...hud }`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeatQcHud {
    /// Health.
    pub health: i32,
    /// Armor.
    pub armor: i32,
}

/// Console draw context (the host draws through its own console handles).
#[derive(Debug, Clone, PartialEq)]
pub struct SeatConsoleContext {
    /// Viewport width.
    pub width: i32,
    /// Viewport height.
    pub height: i32,
    /// Console height in pixels.
    pub console_height: i32,
    /// Prepared time in milliseconds.
    pub time_ms: f64,
}

/// Component drawings snapshot (donor `snapshot` result).
#[derive(Debug, Clone, PartialEq)]
pub struct SeatDrawingSnapshot<D> {
    /// Debug lines.
    pub lines: Vec<D>,
    /// World text.
    pub text: Vec<SeatWorldText>,
}

/// Staged image refresh (donor `prepareImageRefresh` result).
#[derive(Debug, Clone, PartialEq)]
pub struct SeatStagedRefresh {
    /// Replacement font handles by content.
    pub fonts: Vec<(ContentId, u64)>,
    /// UI staging handle.
    pub ui: u64,
    /// Sky replacements.
    pub sky: Q1SkyRefresh,
}

/// Q3 body pose (donor `bodyPose` result).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SeatBodyPose {
    /// Body origin.
    pub origin: Vec3,
    /// Body angles.
    pub angles: Vec3,
}

/// Q3 weapon HUD view (donor `weaponHudView` result).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SeatWeaponHudView {
    /// Whether visible.
    pub visible: Option<bool>,
    /// Whether aggregate warnings show.
    pub aggregate_warning: Option<bool>,
}

/// Q3 frame view options (donor `frame` view spread).
#[derive(Debug, Clone, PartialEq)]
pub struct SeatQ3View {
    /// Whether the world model is skipped.
    pub no_world_model: bool,
    /// Q2 sky override.
    pub q2_sky: Option<qa_client::render::scene::q2_sky::Q2SkyView>,
    /// Q1 fog override.
    pub q1_fog: Option<SceneFog>,
}

/// Seat host: simulation, input, assets, effects, UI, and frame sinks.
///
/// Query methods take `&self` so [`WorldSeatPresentation::camera`] stays
/// side-effect free like the donor; commands take `&mut self`.
pub trait SeatHost {
    /// Host UI snapshot (donor merged controller and message state).
    type UiSnapshot: Clone + std::fmt::Debug + PartialEq + Eq;
    /// Host debug line.
    type DebugLine: Clone + std::fmt::Debug + PartialEq;

    /// Presenting seat.
    fn seat_id(&self) -> SeatId;
    /// Presenting client.
    fn client_id(&self) -> ClientId;
    /// Player actor.
    fn actor(&self) -> ActorId;
    /// Whether the console holds input focus.
    fn focus_is_console(&self) -> bool;
    /// Whether a named button is active.
    fn button_active(&self, name: &str) -> bool;
    /// Whether the input builder speaks Q3.
    fn builder_is_q3(&self) -> bool;
    /// Set builder view angles.
    fn set_view_angles(&mut self, angles: Vec3);
    /// Window drawable size.
    fn window_drawable_size(&self) -> (i32, i32);
    /// Default horizontal field of view.
    fn field_of_view(&self) -> f32;
    /// Q1 view settings, when active.
    fn view_size(&self) -> Option<Q1ViewSettings>;
    /// Override a computed camera.
    fn camera_override(&self, camera: &SceneCamera) -> SceneCamera;

    /// Print console text.
    fn console_print(&mut self, text: &str);
    /// Draw the console.
    fn draw_console(&mut self, ctx: &SeatConsoleContext);

    /// Player view for an actor.
    fn player_view(&self, actor: &ActorId) -> SeatPlayerView;
    /// Simulation world text.
    fn world_texts(&self) -> Vec<SeatWorldText>;
    /// Mod client presentation sources.
    fn mod_client_sources(&mut self) -> Vec<SeatModClientSource>;
    /// Current frame for a source, or [`None`] when retired.
    fn mod_client_frame(&mut self, source: u64) -> Option<SeatModClientFrame>;
    /// Assert a source is current.
    fn mod_client_assert_current(&self, source: u64) -> Result<(), SeatError>;
    /// Live generation for a source.
    fn mod_client_generation(&self, source: u64) -> u64;
    /// Resolve a source's camera view.
    fn mod_client_view(&self, source: u64, movement_origin: Option<Vec3>, angles: Vec3) -> SeatPlayerView;

    /// Bind renderer hardware.
    fn bind_renderer_hardware(&mut self);
    /// Effect-adjusted player view.
    fn effect_player_view(&self, actor: &ActorId, camera: &SceneCamera) -> SeatEffectPlayerView;
    /// Effect frame for a camera and order.
    fn effect_frame(
        &self,
        camera: &SceneCamera,
        source: &SourceSceneOrder,
        viewer: Option<&ActorId>,
        fog: Option<&SceneFog>,
    ) -> SeatEffectFrame;
    /// Shadow scene lights.
    fn shadow_lights(
        &self,
        camera: &SceneCamera,
        style: &dyn Fn(i32) -> f64,
        viewer: Option<&ActorId>,
    ) -> Vec<SceneLight>;
    /// Chase obstruction queries, or [`None`] without the numeric profile.
    fn chase_queries(&self) -> Option<&dyn Q1ChaseScene>;
    /// Close effects.
    fn close_effects(&mut self);

    /// UI snapshot for a timestamp.
    fn ui_snapshot(&self, time_ms: f64) -> Self::UiSnapshot;
    /// Receive seat events.
    fn ui_receive(&mut self, events: &[SeatSourceEvent]);
    /// Prepare the seat UI.
    fn ui_prepare(&mut self);
    /// Prepare the native Q2 HUD.
    fn ui_prepare_native_hud(
        &mut self,
        frame: &NativeQ2HudFrame,
        content: &ContentId,
        binding: &SeatPresentationBinding,
        time_ms: f64,
    );
    /// Prepare a component HUD.
    #[allow(clippy::too_many_arguments)]
    fn ui_prepare_component_hud(
        &mut self,
        frame: &NativeQ2HudFrame,
        content: &ContentId,
        binding: &SeatPresentationBinding,
        time_ms: f64,
        hud: &mut ApplicationQ2NativeHud,
        mode: SeatHudMode,
        assert_current: &dyn Fn() -> Result<(), SeatError>,
    );
    /// Weapon occlusion rectangles.
    fn ui_weapon_occlusion(&self, binding: &SeatPresentationBinding, time_ms: f64, active: bool) -> Vec<Rect>;
    /// Draw the seat UI.
    fn ui_draw(&mut self, args: &SeatUiDrawArgs);
    /// Draw the native Q2 HUD.
    fn ui_draw_native_hud(&mut self, frame: &NativeQ2HudFrame, binding: &SeatPresentationBinding, time_ms: f64);
    /// Draw a component HUD.
    fn ui_draw_component_hud(
        &mut self,
        frame: &NativeQ2HudFrame,
        binding: &SeatPresentationBinding,
        time_ms: f64,
        hud: &mut ApplicationQ2NativeHud,
        mode: SeatHudMode,
    );
    /// Close the seat UI.
    fn ui_close(&mut self);
    /// Stage a UI image refresh, returning a handle.
    fn stage_ui_image_refresh(&mut self) -> u64;
    /// Commit a staged UI image refresh.
    fn commit_ui_image_refresh(&mut self, handle: u64);
    /// Discard a staged UI image refresh.
    fn discard_ui_image_refresh(&mut self, handle: u64);
    /// Refresh UI images.
    fn refresh_ui_images(&mut self);

    /// Selected message language, or [`None`] without rerelease.
    fn rerelease_language(&self, seat: &SeatId) -> Option<String>;
    /// Whether a presentation item stays visible.
    fn rerelease_item_visible(&self, actor: &ActorId, presentation: &ActorId) -> bool;
    /// Localize a message.
    fn rerelease_localize(&mut self, seat: &SeatId, content: &ContentId, text: &str) -> String;
    /// Apply the rerelease view spread to an input.
    fn apply_rerelease_view(&self, input: &mut WorldViewInput, actor: &ActorId, time: f64);
    /// Draw the rerelease story overlay.
    fn draw_rerelease_story(&mut self, actor: &ActorId, height_scale: f64);
    /// Whether the rerelease story is active.
    fn rerelease_story_active(&self, actor: &ActorId) -> bool;

    /// Whether the finale is active.
    fn finale_active(&self) -> bool;
    /// Receive seat events.
    fn finale_receive(&mut self, events: &[SeatSourceEvent]);
    /// Prepare the finale.
    fn finale_prepare(&mut self);
    /// Draw the finale.
    fn finale_draw(&mut self, time: f64);

    /// Receive seat events for component drawings.
    fn drawings_receive(&mut self, events: &[SeatSourceEvent]);
    /// Prepare component debug graphs.
    fn drawings_prepare_graphs(&mut self) -> Result<(), SeatError>;
    /// Snapshot component drawings.
    fn drawings_snapshot(&self, time: f64, frame: u64) -> SeatDrawingSnapshot<Self::DebugLine>;
    /// Clear component drawings.
    fn drawings_clear(&mut self);

    /// Load a world-text font, returning a handle.
    fn load_font(&mut self, content: &ContentId) -> Result<u64, SeatError>;
    /// Close a font handle.
    fn close_font(&mut self, handle: u64) -> Result<(), SeatError>;
    /// Commit image fonts.
    fn commit_image_fonts(&mut self);

    /// Begin a frame.
    fn begin_frame(&mut self);
    /// Clear a mismatched viewport.
    fn clear_viewport_mismatch(&mut self, viewport: Rect, time: RenderSourceTime);
    /// Submit a prepared world view.
    fn submit_world(&mut self, view: qa_client::render::scene::world::PreparedWorldView);
    /// Submit native frame commands.
    fn submit_native_commands(&mut self, commands: Vec<RenderCommand>);
    /// Host debug lines.
    fn debug_lines(&self) -> Vec<Self::DebugLine>;
    /// Debug line width.
    fn debug_line_width(&self) -> f32;
    /// Draw debug lines.
    fn draw_debug_lines(&mut self, lines: &[Self::DebugLine], camera: &SceneCamera, line_width: f32);
    /// Draw world text.
    fn draw_world_text(&mut self, texts: &[SeatWorldText], camera: &SceneCamera);
    /// Fill a fullscreen blend.
    fn fill_blend(&mut self, viewport: Rect, blend: Vec4);
    /// Submit damage-blend batches.
    fn submit_damage(&mut self, batches: Vec<DrawBatch>, viewport: Rect);
    /// Draw the graph overlay.
    fn draw_graph_overlay(&mut self, viewport: Rect);
    /// Finish the frame.
    fn finish_frame(&mut self) -> RenderFrame;
    /// Execute a frame on the native backend.
    fn execute_frame(&mut self, frame: RenderFrame);
}

/// Q3 client surface (donor `ApplicationQ3Client` subset).
pub trait SeatQ3Client {
    /// Whether the client runs a QVM.
    fn q3_kind_is_qvm(&self) -> bool;
    /// Client camera.
    fn q3_camera(&self) -> SceneCamera;
    /// Integer cvar, when set.
    fn q3_cvar_int(&self, name: &str) -> Option<i32>;
    /// Shared equipment view visibility override.
    fn q3_shared_equipment_view_visible(&self) -> Option<bool>;
    /// Prepare the client for a frame.
    #[allow(clippy::too_many_arguments)]
    fn q3_prepare(
        &mut self,
        frame: i32,
        viewport: Rect,
        presentations: &[ScenePresentation],
        hud_visible: bool,
        bodies: Vec<SceneBody>,
        weapon_visible: bool,
        native_view: bool,
    );
    /// Body pose for an actor.
    fn q3_body_pose(&self, actor: &ActorId) -> Option<SeatBodyPose>;
    /// Shared held weapons.
    fn q3_shared_held_weapons(&self) -> Vec<SceneHeldWeapon>;
    /// Shared bodies.
    fn q3_shared_bodies(&self) -> Vec<ScenePrimaryBody>;
    /// Render a native frame, invoking the seat callbacks.
    ///
    /// The host resolves the supplemental weapon camera before invoking
    /// `effects` (the donor reads it from the client inside the closure).
    fn q3_frame(
        &mut self,
        effects: &mut dyn FnMut(&SceneCamera, &SourceSceneOrder, SceneCamera) -> Result<SeatEffectFrame, SeatError>,
        camera: &mut dyn FnMut(&SceneCamera) -> Result<SceneCamera, SeatError>,
        view: &SeatQ3View,
        offset: Option<Vec3>,
    ) -> Result<Option<Vec<RenderCommand>>, SeatError>;
    /// Supplemental weapon camera.
    fn q3_supplemental_weapon_camera(&self, camera: &SceneCamera) -> SceneCamera;
    /// Weapon HUD view.
    fn q3_weapon_hud_view(&self) -> SeatWeaponHudView;
    /// Close the client.
    fn q3_close(&mut self);
}

/// World scene surface (forwarded to [`ApplicationWorldScene`]).
pub trait SeatScene {
    /// Receive presentation events.
    fn scene_receive(&mut self, events: &[SimulationPresentationEvent<()>]);
    /// Sample a lightstyle pattern.
    fn scene_style(&self, index: i32, absent: f64) -> f64;
    /// Quake and Quake II style tables.
    fn scene_styles(&self) -> (Vec<i32>, Vec<qa_client::materials::lighting::Q2LightStyle>);
    /// Assemble the scene for one frame.
    fn scene_prepare(&mut self, input: &ScenePrepareInput) -> Result<(), SceneError>;
    /// Prepare the full view.
    fn scene_view(
        &mut self,
        input: WorldViewInput,
        operations: &[SceneOperation],
        shadow_lights: &[SceneLight],
        infrared: bool,
        weapon_camera: SceneCamera,
    ) -> Result<qa_client::render::scene::world::PreparedWorldView, SceneError>;
    /// Inline-model and model operations.
    fn scene_supplemental(
        &mut self,
        input: &WorldViewInput,
        weapon_camera: SceneCamera,
        infrared: bool,
    ) -> Result<Vec<SceneOperation>, SceneError>;
    /// Release scene caches.
    fn scene_close(&mut self);
}

/// Forward the seat scene surface to the ported world scene.
impl<A, R, W, H, B> SeatScene for ApplicationWorldScene<A, R, W, H, B>
where
    A: super::presentation_scene::SceneModelAssets,
    R: super::presentation_scene::SceneRendererFactory,
    W: super::presentation_scene::SceneWorld,
    H: super::presentation_scene::SceneHeldWeapons,
    B: super::presentation_scene::SceneBodies,
{
    fn scene_receive(&mut self, events: &[SimulationPresentationEvent<()>]) {
        self.receive(events);
    }

    fn scene_style(&self, index: i32, absent: f64) -> f64 {
        self.style(index, absent)
    }

    fn scene_styles(&self) -> (Vec<i32>, Vec<qa_client::materials::lighting::Q2LightStyle>) {
        self.styles()
    }

    fn scene_prepare(&mut self, input: &ScenePrepareInput) -> Result<(), SceneError> {
        self.prepare(input)
    }

    fn scene_view(
        &mut self,
        input: WorldViewInput,
        operations: &[SceneOperation],
        shadow_lights: &[SceneLight],
        infrared: bool,
        weapon_camera: SceneCamera,
    ) -> Result<qa_client::render::scene::world::PreparedWorldView, SceneError> {
        self.view(input, operations, shadow_lights, infrared, Some(weapon_camera))
    }

    fn scene_supplemental(
        &mut self,
        input: &WorldViewInput,
        weapon_camera: SceneCamera,
        infrared: bool,
    ) -> Result<Vec<SceneOperation>, SceneError> {
        self.supplemental(input, Some(weapon_camera), infrared)
    }

    fn scene_close(&mut self) {
        self.close();
    }
}

/// Content reads (donor `ApplicationAssets["content"]` subset beyond the Q1 reads).
pub trait SeatContent {
    /// Worldspawn entities text.
    fn worldspawn_entities(&self) -> String;
    /// Whether the world is a Quake map.
    fn world_is_q1_bsp(&self) -> bool;
    /// Mount contents.
    fn mount_contents(&self) -> HashSet<ContentId>;
    /// Whether the numeric timing profile is present.
    fn numeric_timing_present(&self) -> bool;
    /// Presentation selection.
    fn presentation_selection(&self) -> PresentationSelection;
    /// White image for blends.
    fn white_image(&self) -> RendererImage;
    /// Material registrations for admission.
    fn material_registrations(&mut self) -> Vec<ShaderRegistration>;
    /// Whether a native Q2 HUD is attached.
    fn has_native_q2(&self) -> bool;
    /// Current native Q2 HUD frame.
    fn native_q2_frame(&mut self) -> Option<NativeQ2HudFrame>;
    /// Whether the native Q2 HUD owns effects.
    fn native_q2_owns_effects(&self) -> bool;
}
/// Pending console message (donor `pendingMessages` entry).
#[derive(Debug, Clone, PartialEq, Eq)]
struct SeatPendingMessage {
    text: String,
    source_sequence: Option<i64>,
}

/// Component client frame (donor `ComponentClientFrame`).
struct SeatComponentClient {
    source: u64,
    owner: PresentationOwner,
    generation: u64,
    identity_content: ContentId,
    frame: SeatModClientFrame,
    hud: ApplicationQ2NativeHud,
}

/// Map a seat event into a scene event (the scene observes owner and lightstyle
/// events; the rest arrive as foreign).
fn scene_event(event: &SeatSourceEvent) -> SimulationPresentationEvent<()> {
    let source = match &event.event {
        SeatEvent::Owner { lifecycle } => super::presentation_state::SourcePresentationEvent::PresentationOwner {
            event: lifecycle.clone(),
        },
        SeatEvent::Lightstyle { family, style, pattern } => match family {
            GameFamily::Q1 => super::presentation_state::SourcePresentationEvent::Q1(Q1PresentationEvent::Lightstyle {
                style: *style,
                pattern: pattern.clone(),
            }),
            GameFamily::Q2 => super::presentation_state::SourcePresentationEvent::Q2(Q2PresentationEvent::Lightstyle {
                style: *style,
                pattern: pattern.clone(),
            }),
            GameFamily::Q3 => super::presentation_state::SourcePresentationEvent::Foreign {
                kind: "q3-lightstyle".to_string(),
                actor: None,
                payload: (),
            },
        },
        other => super::presentation_state::SourcePresentationEvent::Foreign {
            kind: other.family_kind().to_string(),
            actor: None,
            payload: (),
        },
    };
    SimulationPresentationEvent {
        source,
        owner: event.owner.clone(),
        recipient: event.recipient.clone(),
        sequence: event.sequence,
        content: event.content.clone(),
        seconds: event.seconds,
        source_entity: event.source_entity,
    }
}

/// Map a seat event into a fog event.
fn fog_event(event: &SeatSourceEvent) -> Q1MapFogEvent {
    match &event.event {
        SeatEvent::Owner {
            lifecycle: OwnerLifecycle::Retired { owner },
        } => Q1MapFogEvent::OwnerRetired { owner: owner.clone() },
        SeatEvent::Q1Fog {
            player,
            transition,
            sky_factor,
        } => Q1MapFogEvent::Transition(super::q1_fog::Q1FogTransitionEvent {
            owner: event.owner.clone(),
            content: event.content.clone(),
            player: player.clone(),
            transition: *transition,
            sky_factor: *sky_factor,
        }),
        _ => Q1MapFogEvent::Other,
    }
}

/// Map a seat event into a service event.
fn service_event(event: &SeatSourceEvent) -> Q1ServiceEvent {
    match &event.event {
        SeatEvent::Owner { lifecycle } => match lifecycle {
            OwnerLifecycle::Refreshed { owner } => Q1ServiceEvent::OwnerRefreshed { owner: owner.clone() },
            OwnerLifecycle::Retired { owner } => Q1ServiceEvent::OwnerRetired { owner: owner.clone() },
        },
        SeatEvent::Q1Sky { name } => Q1ServiceEvent::Sky {
            owner: event.owner.clone(),
            content: event.content.clone(),
            recipient: event.recipient.clone(),
            name: name.clone(),
            sequence: event.sequence as u64,
        },
        SeatEvent::Q1Client(metadata) => Q1ServiceEvent::Client {
            owner: event.owner.clone(),
            content: event.content.clone(),
            recipient: event.recipient.clone(),
            event: metadata.clone(),
            sequence: event.sequence as u64,
        },
        _ => Q1ServiceEvent::Other,
    }
}

/// Whether an operation draws world polygons (donor `polygon`).
fn is_world_polygon(operation: &SceneOperation) -> bool {
    matches!(operation, SceneOperation::Group(group)
        if matches!(&group.order, SceneGroupOrder::Source { source, .. }
            if source.entity == SourceEntityOrder::World))
}

/// Tokenize a server command, falling back to whitespace splitting.
fn server_command_argv(text: &str) -> Vec<String> {
    tokenize_command(text, Dialect::Q3, TextMode::Source)
        .map(|tokens| tokens.argv)
        .unwrap_or_else(|_| text.split_whitespace().map(str::to_string).collect())
}

/// Sized chase-query wrapper (the ported chase camera takes `impl`).
struct ChaseQueries<'a>(&'a dyn Q1ChaseScene);

impl Q1ChaseScene for ChaseQueries<'_> {
    fn trace_box(&self, start: Vec3, end: Vec3, actor: &ActorId) -> super::q1_client_settings::Q1ChaseTrace {
        self.0.trace_box(start, end, actor)
    }

    fn trace_point(&self, start: Vec3, end: Vec3, actor: &ActorId) -> super::q1_client_settings::Q1ChaseTrace {
        self.0.trace_point(start, end, actor)
    }
}

/// Q1 fog parameters for an optional scene fog.
fn q1_fog_params(fog: Option<&SceneFog>) -> Option<Q1FogParams> {
    match fog {
        Some(SceneFog::Q1 {
            color,
            density,
            sky_factor,
        }) => Some(Q1FogParams {
            color: *color,
            density: *density,
            sky_factor: *sky_factor,
        }),
        _ => None,
    }
}

/// Render clock for a source time.
fn render_time(time: &SourceTime) -> RenderSourceTime {
    match *time {
        SourceTime::Seconds(value) => RenderSourceTime::Seconds(f64::from(value)),
        SourceTime::Milliseconds(value) => RenderSourceTime::Milliseconds(f64::from(value)),
    }
}

/// World seat presentation (donor `WorldSeatPresentation`).
pub struct WorldSeatPresentation<H: SeatHost, Q: SeatQ3Client, S: SeatScene, A> {
    host: H,
    q3client: Option<Q>,
    scene: S,
    assets: A,
    localization: Q1MessageLocalization<A>,
    language: String,
    fog: Q1MapFog,
    services: Q1ServicePresentation,
    component_effects: HashMap<u64, (SeatComponentEffect, Option<SeatComponentOverlay>)>,
    next_effect: u64,
    pending_messages: Vec<SeatPendingMessage>,
    pending_q1: Vec<SeatSourceEvent>,
    pending_q2: Vec<SeatSourceEvent>,
    prepared_time: f64,
    world_texts: Vec<SeatWorldText>,
    drawing_frame: u64,
    component_lines: Vec<H::DebugLine>,
    world_fonts: HashMap<ContentId, u64>,
    layout_index: u32,
    layout_count: u32,
    native_q2_frame: Option<NativeQ2HudFrame>,
    component_clients: Vec<SeatComponentClient>,
    component_movement_origin: Option<Vec3>,
    backend_id: u64,
    closed: bool,
}

impl<H: SeatHost, Q: SeatQ3Client, S: SeatScene, A> WorldSeatPresentation<H, Q, S, A>
where
    A: SeatContent + Q1MessageAssets + Q1ServiceAssets + Clone,
{
    /// Build the presentation over a host, scene, and content reads.
    ///
    /// `seat_count` is the initial layout count (validated on use, like the
    /// donor); `backend_id` is the host-minted native backend identity.
    pub fn new(
        mut host: H,
        q3client: Option<Q>,
        scene: S,
        assets: A,
        seat_count: u32,
        backend_id: u64,
    ) -> Result<Self, SeatError> {
        host.bind_renderer_hardware();
        let fog = Q1MapFog::new(
            &assets.worldspawn_entities(),
            assets.mount_contents(),
            host.actor(),
            assets.world_is_q1_bsp(),
        )
        .map_err(|error| SeatError::Fog(error.to_string()))?;
        let seat = host.seat_id();
        let layout_index = seat.index();
        Ok(Self {
            host,
            q3client,
            scene,
            assets: assets.clone(),
            localization: Q1MessageLocalization::new(seat, assets),
            language: "english".to_string(),
            fog,
            services: Q1ServicePresentation::new(),
            component_effects: HashMap::new(),
            next_effect: 0,
            pending_messages: Vec::new(),
            pending_q1: Vec::new(),
            pending_q2: Vec::new(),
            prepared_time: 0.0,
            world_texts: Vec::new(),
            drawing_frame: 0,
            component_lines: Vec::new(),
            world_fonts: HashMap::new(),
            layout_index,
            layout_count: seat_count,
            native_q2_frame: None,
            component_clients: Vec::new(),
            component_movement_origin: None,
            backend_id,
            closed: false,
        })
    }

    /// Seat viewport (donor `viewport`).
    pub fn viewport(&self) -> Result<Rect, SeatError> {
        let (width, height) = self.host.window_drawable_size();
        seat_viewport(self.layout_index, self.layout_count, width, height)
    }

    /// Publish a split-screen layout (donor `publishLayout`).
    pub fn publish_layout(&mut self, index: u32, count: u32) -> Result<(), SeatError> {
        if !(1..=4).contains(&count) || index >= count {
            return Err(SeatError::Layout);
        }
        self.layout_index = index;
        self.layout_count = count;
        Ok(())
    }

    /// Presentation binding for host calls.
    fn binding(&self) -> Result<SeatPresentationBinding, SeatError> {
        let viewport = self.viewport()?;
        Ok(SeatPresentationBinding {
            seat: self.host.seat_id(),
            client: self.host.client_id(),
            viewport,
            safe_area: viewport,
            hud_scale: 1.0,
            presentation: self.assets.presentation_selection(),
        })
    }

    /// Seat client state (donor `state`).
    pub fn state(&self) -> Result<SeatClientState<H::UiSnapshot>, SeatError> {
        Ok(SeatClientState {
            seat: self.host.seat_id(),
            client: self.host.client_id(),
            actor: self.host.actor(),
            ui: SeatUiData {
                snapshot: self.host.ui_snapshot(self.prepared_time * 1000.0),
                focus_console: self.host.focus_is_console(),
                show_scores: self.host.button_active("scores"),
            },
            presentation: self.binding()?,
        })
    }

    /// Seat camera (donor `camera`).
    pub fn camera(&self) -> Result<SceneCamera, SeatError> {
        Ok(self.host.camera_override(&self.source_camera()?))
    }

    /// Un-overridden seat camera (donor `sourceCamera`).
    fn source_camera(&self) -> Result<SceneCamera, SeatError> {
        if let Some(controlled) = self.component_view()? {
            return self.client_camera(&controlled);
        }
        let player = self.host.player_view(&self.host.actor());
        if let Some(client) = self.q3client.as_ref().filter(|client| client.q3_kind_is_qvm()) {
            let kick = player.kick_angles.unwrap_or_default();
            return Ok(camera_with_kick(
                &camera_with_client_offset(&client.q3_camera(), &player),
                kick,
            ));
        }
        let viewport = match self.host.view_size() {
            None => self.viewport()?,
            Some(size) => q1_view_rectangle(
                self.viewport()?,
                size.size,
                self.host.finale_active(),
                size.overlay_status,
            ),
        };
        if let Some(client) = &self.q3client {
            let third_person = client.q3_cvar_int("cg_thirdPerson").unwrap_or(0) != 0;
            let base = client.q3_camera();
            let offset = camera_with_client_offset(&base, &player);
            let posed = if third_person {
                offset
            } else {
                camera_with_character_death(&offset, &player)
            };
            let kick = player.kick_angles.unwrap_or_default();
            return self.apply_view_size(&camera_with_kick(&posed, kick));
        }
        let fov_x = player.field_of_view.unwrap_or_else(|| self.host.field_of_view());
        let fov_y = vertical_fov(fov_x, viewport);
        let camera = SceneCamera {
            origin: vec3(player.origin.x, player.origin.y, player.origin.z + player.view_height),
            axis: angles_to_axis(player.angles),
            viewport,
            projection: perspective_projection(fov_x, fov_y, 16384.0, 4.0)
                .map_err(|error| SeatError::Camera(error.to_string()))?,
            clip: qa_client::view::CameraClip::None,
        };
        let kick = player.kick_angles.unwrap_or_default();
        let first_person = self
            .host
            .effect_player_view(&self.host.actor(), &camera_with_kick(&camera, kick))
            .camera;
        let Some(chase) = self.chase_settings() else {
            return Ok(first_person);
        };
        if !self.assets.numeric_timing_present() {
            return Err(SeatError::ChaseTiming);
        }
        let Some(queries) = self.host.chase_queries() else {
            return Err(SeatError::ChaseTiming);
        };
        let queries = ChaseQueries(queries);
        Ok(q1_chase_camera(
            &first_person,
            player.angles,
            &chase,
            &queries,
            &self.host.actor(),
        ))
    }

    /// Component-controlled view, when one drives the camera (donor `componentView`).
    fn component_view(&self) -> Result<Option<SeatPlayerView>, SeatError> {
        let Some(client) = self.component_clients.iter().find(|client| client.frame.has_view) else {
            return Ok(None);
        };
        Ok(Some(self.host.mod_client_view(
            client.source,
            self.component_movement_origin,
            self.host.player_view(&self.host.actor()).angles,
        )))
    }

    /// Native camera view, when one drives the camera (donor `nativeView`).
    fn native_view(&self) -> Option<SeatNativeFlags> {
        self.component_clients
            .iter()
            .find(|client| client.frame.has_view)
            .filter(|client| client.frame.kind == SeatModKind::Native)
            .and_then(|client| client.frame.native)
    }

    /// Whether component views keep the weapon visible (donor `componentWeaponVisible`).
    fn component_weapon_visible(&self) -> bool {
        let viewed = self.component_clients.iter().find(|client| client.frame.has_view);
        match viewed {
            None => true,
            Some(client) if client.frame.kind != SeatModKind::Native => true,
            Some(client) => client.frame.native.is_some_and(|native| native.weapon_visible),
        }
    }

    /// Camera for a component-controlled view (donor `clientCamera`).
    fn client_camera(&self, view: &SeatPlayerView) -> Result<SceneCamera, SeatError> {
        let viewport = self.viewport()?;
        let fov_x = view.field_of_view.unwrap_or_else(|| self.host.field_of_view());
        let fov_y = vertical_fov(fov_x, viewport);
        let camera = SceneCamera {
            origin: vec3(view.origin.x, view.origin.y, view.origin.z + view.view_height),
            axis: angles_to_axis(view.angles),
            viewport,
            projection: perspective_projection(fov_x, fov_y, 16384.0, 4.0)
                .map_err(|error| SeatError::Camera(error.to_string()))?,
            clip: qa_client::view::CameraClip::None,
        };
        Ok(camera_with_kick(&camera, view.kick_angles.unwrap_or_default()))
    }

    /// Refresh component client frames, retaining HUDs (donor `prepareComponentClients`).
    fn prepare_component_clients(&mut self) -> Result<(), SeatError> {
        let sources = self.host.mod_client_sources();
        let actor = self.host.actor();
        let mut previous = HashMap::new();
        for client in self.component_clients.drain(..) {
            previous.insert((client.owner, client.source, client.generation), client.hud);
        }
        let mut next = Vec::new();
        for source in &sources {
            self.host.mod_client_assert_current(source.id)?;
            let Some(frame) = self.host.mod_client_frame(source.id) else {
                continue;
            };
            let key = (source.owner.clone(), source.id, source.generation);
            let hud = previous.remove(&key).unwrap_or_else(ApplicationQ2NativeHud::new);
            next.push(SeatComponentClient {
                source: source.id,
                owner: source.owner.clone(),
                generation: source.generation,
                identity_content: source.identity_content.clone(),
                frame,
                hud,
            });
        }
        for (_, mut hud) in previous {
            hud.clear();
        }
        self.component_clients = next;
        for index in 0..self.component_clients.len() {
            let native_hud = match &self.component_clients[index].frame {
                SeatModClientFrame {
                    kind: SeatModKind::Native,
                    hud: Some(SeatModHud::Native { mode, frame }),
                    ..
                } => Some((mode.clone(), frame.clone())),
                _ => None,
            };
            let Some((mode, frame)) = native_hud else {
                continue;
            };
            let (source, content) = (
                self.component_clients[index].source,
                self.component_clients[index].identity_content.clone(),
            );
            let binding = self.binding()?;
            let time_ms = self.prepared_time * 1000.0;
            let closed = &self.closed;
            let assert_light = || {
                if *closed {
                    return Err(SeatError::ComponentRetired);
                }
                Ok(())
            };
            let hud = &mut self.component_clients[index].hud;
            self.host
                .ui_prepare_component_hud(&frame, &content, &binding, time_ms, hud, mode, &assert_light);
            self.assert_component_current(source, &actor)?;
        }
        Ok(())
    }

    /// Assert a component client is current (donor `assertCurrent`).
    fn assert_component_current(&mut self, source: u64, actor: &ActorId) -> Result<(), SeatError> {
        let Some(client) = self.component_clients.iter().find(|client| client.source == source) else {
            return Err(SeatError::ComponentRetired);
        };
        let generation = client.generation;
        self.host.mod_client_assert_current(source)?;
        if self.closed
            || self.host.actor() != *actor
            || self.host.mod_client_generation(source) != generation
            || self.host.mod_client_frame(source).is_none()
        {
            return Err(SeatError::ComponentRetired);
        }
        Ok(())
    }

    /// Current Q3 client (donor `q3Client`).
    pub fn q3client(&self) -> Option<&Q> {
        self.q3client.as_ref()
    }

    /// Replace the Q3 client, returning the previous one (donor `replaceQ3Client`).
    pub fn replace_q3client(&mut self, candidate: Q, seat: &SeatId, actor: &ActorId) -> Result<Option<Q>, SeatError> {
        if *seat != self.host.seat_id() || *actor != self.host.actor() {
            return Err(SeatError::ReplacementPlayer);
        }
        let previous = self.q3client.replace(candidate);
        Ok(previous)
    }

    /// Chase settings, when the chase camera applies (donor `chaseSettings`).
    fn chase_settings(&self) -> Option<Q1ChaseSettings> {
        if self.q3client.is_some() || self.host.finale_active() {
            return None;
        }
        self.host.view_size()?.chase
    }

    /// Apply the view size to a camera (donor `applyViewSize`).
    fn apply_view_size(&self, camera: &SceneCamera) -> Result<SceneCamera, SeatError> {
        let viewport = self.viewport()?;
        match self.host.view_size() {
            None => Ok(*camera),
            Some(settings) => Ok(q1_view_camera(camera, viewport, &settings, self.host.finale_active())),
        }
    }

    /// Receive simulation events (donor `receive`).
    pub fn receive(&mut self, events: &[SeatSimulationEvent]) {
        for event in events {
            if let Some(text) = &event.message_text {
                self.pending_messages.push(SeatPendingMessage {
                    text: text.clone(),
                    source_sequence: event.source_sequence,
                });
            }
        }
    }

    /// Receive presentation events (donor `sourceEvents`).
    pub fn source_events(&mut self, incoming: &[SeatSourceEvent]) -> Result<(), SeatError> {
        let actor = self.host.actor();
        let events: Vec<SeatSourceEvent> = incoming
            .iter()
            .filter(|event| event.recipient.as_ref().is_none_or(|recipient| *recipient == actor))
            .cloned()
            .collect();
        for event in &events {
            if let SeatEvent::Owner { lifecycle } = &event.event {
                let owner = match lifecycle {
                    OwnerLifecycle::Refreshed { owner } | OwnerLifecycle::Retired { owner } => owner,
                };
                for client in &mut self.component_clients {
                    if client.owner == *owner {
                        client.hud.clear();
                    }
                }
                self.component_clients.retain(|client| client.owner != *owner);
            }
        }
        self.host.drawings_receive(&events);
        self.fog.receive(&events.iter().map(fog_event).collect::<Vec<_>>());
        self.services
            .receive(&events.iter().map(service_event).collect::<Vec<_>>())?;
        for source in &events {
            if let SeatEvent::Q1Message { player, .. } = &source.event {
                if *player == actor {
                    self.pending_q1.push(source.clone());
                }
            }
        }
        if self.q3client.is_some() {
            if self.host.builder_is_q3() {
                return Ok(());
            }
            for source in &events {
                if let SeatEvent::ViewReset { actor: reset, angles } = &source.event {
                    if *reset == actor {
                        self.host.set_view_angles(*angles);
                    }
                }
            }
            return Ok(());
        }
        let mapped: Vec<SimulationPresentationEvent<()>> = events.iter().map(scene_event).collect();
        self.scene.scene_receive(&mapped);
        let ui_events: Vec<SeatSourceEvent> = events
            .iter()
            .filter(|source| !matches!(source.event, SeatEvent::Q1Message { .. }))
            .filter(|source| {
                !matches!(
                    source.event,
                    SeatEvent::Q2Help { .. }
                        | SeatEvent::Q2Pickup { .. }
                        | SeatEvent::Q2Print { .. }
                        | SeatEvent::Q2Centerprint { .. }
                )
            })
            .filter(|source| !matches!(source.event, SeatEvent::Q2PlayerPrint { .. }))
            .cloned()
            .collect();
        self.host.ui_receive(&ui_events);
        self.host.finale_receive(&events);
        for source in &events {
            match &source.event {
                SeatEvent::ViewReset { actor: reset, angles } => {
                    if *reset == actor {
                        self.host.set_view_angles(*angles);
                    }
                }
                SeatEvent::Q1Teleport { player, angles } => {
                    if *player == actor {
                        self.host.set_view_angles(*angles);
                    }
                }
                SeatEvent::Q2Help { .. }
                | SeatEvent::Q2Pickup { .. }
                | SeatEvent::Q2Print { .. }
                | SeatEvent::Q2Centerprint { .. } => {
                    self.pending_q2.push(source.clone());
                }
                SeatEvent::Q2PlayerPrint { .. } => {
                    self.pending_q2.push(source.clone());
                }
                SeatEvent::Q3ServerCommand { client, text }
                    if *client < 0 || *client as u32 == self.host.client_id().slot() =>
                {
                    let argv = server_command_argv(text);
                    let print = match argv.as_slice() {
                        [command, text, ..]
                            if (command == "print" || command == "chat" || command == "tchat") && !text.is_empty() =>
                        {
                            Some(text)
                        }
                        _ => None,
                    };
                    if let Some(text) = print {
                        self.host.console_print(text);
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Localization for the current language, rebuilding on change.
    fn localization_for(&mut self, language: &str) -> &mut Q1MessageLocalization<A> {
        if self.language != language {
            let owned = language.to_string();
            self.localization = Q1MessageLocalization::with_language(
                self.host.seat_id(),
                self.assets.clone(),
                Rc::new(move || owned.clone()),
            );
            self.language = language.to_string();
        }
        &mut self.localization
    }

    /// Stage an image refresh (donor `prepareImageRefresh`).
    pub fn stage_image_refresh(&mut self) -> Result<SeatStagedRefresh, SeatError> {
        let mut fonts = Vec::new();
        for content in self.world_fonts.keys().cloned().collect::<Vec<_>>() {
            match self.host.load_font(&content) {
                Ok(handle) => fonts.push((content, handle)),
                Err(error) => {
                    for (_, handle) in &fonts {
                        let _ = self.host.close_font(*handle);
                    }
                    return Err(error);
                }
            }
        }
        let ui = self.host.stage_ui_image_refresh();
        let sky = match self.services.prepare_image_refresh(&mut self.assets) {
            Ok(sky) => sky,
            Err(error) => {
                for (_, handle) in &fonts {
                    let _ = self.host.close_font(*handle);
                }
                self.host.discard_ui_image_refresh(ui);
                return Err(error.into());
            }
        };
        Ok(SeatStagedRefresh { fonts, ui, sky })
    }

    /// Commit a staged image refresh (donor `commit`).
    pub fn commit_image_refresh(&mut self, staged: SeatStagedRefresh) {
        self.host.commit_image_fonts();
        self.host.refresh_ui_images();
        self.host.commit_ui_image_refresh(staged.ui);
        for client in &mut self.component_clients {
            client.hud.clear();
        }
        for (_, handle) in self.world_fonts.drain() {
            let _ = self.host.close_font(handle);
        }
        self.world_fonts.extend(staged.fonts);
        self.services.commit_refresh(staged.sky);
    }

    /// Discard a staged image refresh (donor `discard`).
    pub fn discard_image_refresh(&mut self, staged: SeatStagedRefresh) {
        for (_, handle) in staged.fonts {
            let _ = self.host.close_font(handle);
        }
        self.host.discard_ui_image_refresh(staged.ui);
    }

    /// Prepare the seat for a snapshot (donor `prepare`).
    pub fn prepare(
        &mut self,
        snapshot: &WorldSnapshot,
        presentations: &[SeatPresentation],
        characters: &[qa_content::q3::foundation::presentation::Q3CharacterView],
    ) -> Result<(), SeatError> {
        self.prepared_time = snapshot.frame.time.as_seconds_f64();
        let actor = self.host.actor();
        self.component_movement_origin = snapshot
            .bodies
            .iter()
            .find(|body| qa_core::identity::SavedActorId::from(&actor) == body.id)
            .map(|body| body.state.origin);
        self.prepare_component_clients()?;
        self.host.drawings_prepare_graphs()?;
        let mut bodies = Vec::new();
        for (_, overlay) in self.component_effects.values() {
            if let Some(overlay) = overlay {
                if let Some(collect) = &overlay.bodies {
                    bodies.extend(collect());
                }
            }
        }
        let visible: Vec<SeatPresentation> = presentations
            .iter()
            .filter(|presentation| {
                !presentation.render_owner_source_client
                    && self.host.rerelease_item_visible(&actor, &presentation.scene.actor)
            })
            .cloned()
            .collect();
        let q1_messages = std::mem::take(&mut self.pending_q1);
        let q2_messages = std::mem::take(&mut self.pending_q2);
        let mut mirrored = HashSet::new();
        for source in &q1_messages {
            mirrored.insert(source.sequence);
        }
        for source in &q2_messages {
            match &source.event {
                SeatEvent::Q2Help { .. } => {
                    mirrored.insert(source.sequence);
                }
                SeatEvent::Q2Centerprint { actor: owner, .. } if owner.as_ref() == Some(&actor) => {
                    mirrored.insert(source.sequence);
                }
                _ => {}
            }
        }
        for message in std::mem::take(&mut self.pending_messages) {
            if message
                .source_sequence
                .is_none_or(|sequence| !mirrored.contains(&sequence))
            {
                self.host.console_print(&format!("{}\n", message.text));
            }
        }
        let language = self
            .host
            .rerelease_language(&self.host.seat_id())
            .unwrap_or_else(|| "english".to_string());
        for source in &q1_messages {
            let SeatEvent::Q1Message {
                text,
                args,
                parts,
                center,
                ..
            } = &source.event
            else {
                continue;
            };
            let resolved = self
                .localization_for(&language)
                .resolve(&source.content, text, args, parts);
            if !center {
                self.host.console_print(&format!("{resolved}\n"));
            }
            let mut forwarded = source.clone();
            forwarded.event = SeatEvent::Q1Message {
                player: actor.clone(),
                text: resolved,
                args: args.clone(),
                parts: parts.clone(),
                center: *center,
            };
            self.host.ui_receive(std::slice::from_ref(&forwarded));
        }
        for source in &q2_messages {
            let seat = self.host.seat_id();
            if let SeatEvent::Q2Pickup { player, name } = &source.event {
                if *player == actor {
                    let text = self.host.rerelease_localize(&seat, &source.content, name);
                    self.host.console_print(&format!("{text}\n"));
                }
                continue;
            }
            let (text, print_console) = match &source.event {
                SeatEvent::Q2Help { text } => (text.clone(), true),
                SeatEvent::Q2Print { actor: owner, text } => {
                    if owner.as_ref().is_some_and(|owner| *owner != actor) {
                        continue;
                    }
                    (text.clone(), true)
                }
                SeatEvent::Q2Centerprint { actor: owner, text } => {
                    if owner.as_ref() != Some(&actor) {
                        continue;
                    }
                    (text.clone(), false)
                }
                SeatEvent::Q2PlayerPrint { target, text } => {
                    if target.as_ref().is_some_and(|target| *target != actor) {
                        continue;
                    }
                    (text.clone(), true)
                }
                _ => continue,
            };
            let resolved = self.host.rerelease_localize(&seat, &source.content, &text);
            if print_console {
                if matches!(source.event, SeatEvent::Q2Help { .. }) {
                    self.host.console_print(&format!("{resolved}\n"));
                } else {
                    self.host.console_print(&resolved);
                }
            }
            let mut forwarded = source.clone();
            forwarded.event = match &source.event {
                SeatEvent::Q2Help { .. } => SeatEvent::Q2Help { text: resolved },
                SeatEvent::Q2Print { actor: owner, .. } => SeatEvent::Q2Print {
                    actor: owner.clone(),
                    text: resolved,
                },
                SeatEvent::Q2Centerprint { actor: owner, .. } => SeatEvent::Q2Centerprint {
                    actor: owner.clone(),
                    text: resolved,
                },
                SeatEvent::Q2PlayerPrint { target, .. } => SeatEvent::Q2PlayerPrint {
                    target: target.clone(),
                    text: resolved,
                },
                _ => continue,
            };
            self.host.ui_receive(std::slice::from_ref(&forwarded));
        }
        self.services.prepare(&mut self.assets)?;
        self.host.ui_prepare();
        self.drawing_frame += 1;
        let drawings = self.host.drawings_snapshot(self.prepared_time, self.drawing_frame);
        self.component_lines = drawings.lines;
        let mut world_texts = self.host.world_texts();
        world_texts.extend(drawings.text);
        self.world_texts = world_texts;
        for text in self.world_texts.clone() {
            if !self.world_fonts.contains_key(&text.content) {
                let handle = self.host.load_font(&text.content)?;
                self.world_fonts.insert(text.content, handle);
            }
        }
        self.native_q2_frame = self.assets.native_q2_frame();
        if let Some(frame) = self.native_q2_frame.clone() {
            let binding = self.binding()?;
            let assets = self.assets.presentation_selection().assets;
            self.host
                .ui_prepare_native_hud(&frame, &assets, &binding, self.prepared_time * 1000.0);
        }
        if self.q3client.is_some() {
            let viewport = match self.host.view_size() {
                None => self.viewport()?,
                Some(size) => q1_view_rectangle(
                    self.viewport()?,
                    size.size,
                    self.host.finale_active(),
                    size.overlay_status,
                ),
            };
            let hud_visible = !self.component_clients.iter().any(|client| {
                client
                    .frame
                    .hud
                    .as_ref()
                    .is_some_and(|hud| client.frame.kind == SeatModKind::Quakec || hud.replaces_status())
            });
            let bodies_snapshot = bodies.clone();
            let weapon_visible = self.component_weapon_visible();
            let native_view = self.native_view().is_some();
            let scene_presentations: Vec<ScenePresentation> =
                visible.iter().map(|presentation| presentation.scene.clone()).collect();
            if let Some(client) = self.q3client.as_mut() {
                client.q3_prepare(
                    snapshot.frame.frame,
                    viewport,
                    &scene_presentations,
                    hud_visible,
                    bodies_snapshot,
                    weapon_visible,
                    native_view,
                );
            }
            let third_person = self.native_view().is_none()
                && self
                    .q3client
                    .as_ref()
                    .and_then(|client| client.q3_cvar_int("cg_thirdPerson"))
                    .unwrap_or(0)
                    != 0;
            let draw_weapon = self.component_weapon_visible()
                && self
                    .q3client
                    .as_ref()
                    .and_then(|client| {
                        client
                            .q3_shared_equipment_view_visible()
                            .or_else(|| client.q3_cvar_int("cg_drawGun").map(|gun| gun != 0))
                    })
                    .unwrap_or(true);
            let mut supplemental = visible;
            for presentation in &mut supplemental {
                if presentation.scene.view_weapon {
                    continue;
                }
                if let Some(pose) = self
                    .q3client
                    .as_ref()
                    .and_then(|client| client.q3_body_pose(&presentation.scene.actor))
                {
                    presentation.scene.origin = pose.origin;
                    presentation.scene.angles = pose.angles;
                }
            }
            let scenes: Vec<ScenePresentation> = supplemental
                .iter()
                .map(|presentation| presentation.scene.clone())
                .collect();
            let empty: Vec<qa_content::q3::foundation::presentation::Q3CharacterView> = Vec::new();
            let held = self
                .q3client
                .as_ref()
                .map(|client| client.q3_shared_held_weapons())
                .unwrap_or_default();
            let shared = self
                .q3client
                .as_ref()
                .map(|client| client.q3_shared_bodies())
                .unwrap_or_default();
            let fov = self
                .q3client
                .as_ref()
                .and_then(|client| client.q3_cvar_int("cg_fov"))
                .unwrap_or(90) as f32;
            let viewer = if third_person { None } else { Some(actor) };
            let input = ScenePrepareInput {
                viewer: viewer.as_ref(),
                snapshot,
                presentations: &scenes,
                characters: &empty,
                field_of_view: fov,
                held_weapons: &held,
                view_weapon_visible: draw_weapon,
                bodies: &bodies,
                primary_bodies: &shared,
            };
            self.scene.scene_prepare(&input)?;
            return Ok(());
        }
        self.host.finale_prepare();
        let viewer = if self.native_view().is_some() || self.chase_settings().is_none() {
            Some(actor)
        } else {
            None
        };
        let fov = (1.0 / f64::from(self.camera()?.projection[0])).atan() * 360.0 / std::f64::consts::PI;
        let scenes: Vec<ScenePresentation> = visible.iter().map(|presentation| presentation.scene.clone()).collect();
        let input = ScenePrepareInput {
            viewer: viewer.as_ref(),
            snapshot,
            presentations: &scenes,
            characters,
            field_of_view: fov as f32,
            held_weapons: &[],
            view_weapon_visible: self.component_weapon_visible(),
            bodies: &bodies,
            primary_bodies: &[],
        };
        self.scene.scene_prepare(&input)?;
        Ok(())
    }

    /// Whether the seat shares the screen (donor `splitScreen`).
    pub fn split_screen(&self) -> bool {
        self.layout_count > 1
    }

    /// Bind a component effect frame, returning its handle (donor `bindComponentEffects`).
    pub fn bind_component_effects(&mut self, frame: SeatComponentEffect, overlay: Option<SeatComponentOverlay>) -> u64 {
        let handle = self.next_effect;
        self.next_effect += 1;
        self.component_effects.insert(handle, (frame, overlay));
        handle
    }

    /// Release a bound component effect frame.
    pub fn unbind_component_effects(&mut self, handle: u64) {
        self.component_effects.remove(&handle);
    }

    /// Merge base and component effect frames (donor `effectFrame`).
    fn effect_frame(
        &self,
        camera: &SceneCamera,
        source: &SourceSceneOrder,
        viewer: Option<&ActorId>,
        fog: Option<&SceneFog>,
    ) -> SeatEffectFrame {
        let base = self.host.effect_frame(camera, source, viewer, fog);
        if self.component_effects.is_empty() {
            return base;
        }
        let mut frames = Vec::with_capacity(self.component_effects.len() + 1);
        frames.push(base);
        let mut handles: Vec<u64> = self.component_effects.keys().copied().collect();
        handles.sort_unstable();
        for handle in handles {
            frames.push(self.component_effects[&handle].0(camera, source, fog));
        }
        let operations: Vec<SceneOperation> = frames
            .iter()
            .flat_map(|frame| frame.operations.iter().cloned())
            .collect();
        let (mut polygon, mut rest): (Vec<SceneOperation>, Vec<SceneOperation>) =
            operations.into_iter().partition(is_world_polygon);
        polygon.append(&mut rest);
        SeatEffectFrame {
            q3_admissions: frames
                .iter()
                .flat_map(|frame| frame.q3_admissions.iter().cloned())
                .collect(),
            operations: polygon,
            lights: frames.iter().flat_map(|frame| frame.lights.iter().copied()).collect(),
            q3_lights: frames
                .iter()
                .flat_map(|frame| frame.q3_lights.iter().copied())
                .take(32)
                .collect(),
        }
    }

    /// Assemble one frame (donor `frame`).
    pub fn frame(&mut self, snapshot: &WorldSnapshot) -> Result<RenderFrame, SeatError> {
        let actor = self.host.actor();
        let viewer = if self.native_view().is_some() || self.chase_settings().is_none() {
            Some(actor.clone())
        } else {
            None
        };
        let time = snapshot.frame.time;
        let camera = self.camera()?;
        let source = if self.q3client.is_none() {
            Some(qa_client::render::scene::submissions::create_source_scene_order(
                self.assets.material_registrations(),
            ))
        } else {
            None
        };
        let fog = if self.fog.active() {
            Some(self.fog.current(self.prepared_time as f32))
        } else {
            None
        };
        let effects = source
            .as_ref()
            .map(|source| self.effect_frame(&camera, source, viewer.as_ref(), fog.as_ref()));
        let player_view = self.host.effect_player_view(&actor, &camera);
        let native_view = self.native_view();
        let no_world_model = native_view.is_some_and(|view| view.render_flags & 2 != 0);
        let infrared = match native_view {
            None => player_view.infrared,
            Some(view) => view.render_flags & 4 != 0,
        };
        let mut input = WorldViewInput::new(camera, ViewTarget::Seat(self.host.seat_id()), render_time(&time));
        input.no_world_model = no_world_model;
        if let Some(source) = &source {
            input.source = Some(qa_client::render::scene::world::create_world_surface_admission(
                source.clone(),
            ));
        }
        self.host.apply_rerelease_view(&mut input, &actor, self.prepared_time);
        input.q2_sky = self.services.view(&actor).cloned();
        input.q1_fog = q1_fog_params(fog.as_ref());
        input.clear = Some(ViewClear {
            depth: 1.0,
            color: Some(if no_world_model {
                qa_core::math::vec4(0.3, 0.3, 0.3, 1.0)
            } else {
                qa_core::math::vec4(0.0, 0.0, 0.0, 1.0)
            }),
            stencil: false,
        });
        input.lights = effects
            .as_ref()
            .map(|effects| effects.lights.clone())
            .unwrap_or_default();
        input.visibility = WorldVisibilityOptions {
            q3_lights: effects
                .as_ref()
                .map(|effects| effects.q3_lights.clone())
                .unwrap_or_default(),
            ..Default::default()
        };
        let (q1_styles, q2_styles) = self.scene.scene_styles();
        input.q1_styles = q1_styles;
        input.q2_styles = q2_styles;
        let native_frame = if self.q3client.is_none() {
            None
        } else {
            let view = SeatQ3View {
                no_world_model,
                q2_sky: self.services.view(&actor).cloned(),
                q1_fog: fog.clone(),
            };
            let offset = if self.component_view()?.is_none() {
                self.host.player_view(&actor).client_view_offset_delta
            } else {
                None
            };
            let viewport = self.viewport()?;
            let field_of_view = self.host.field_of_view();
            let qvm = self.q3client.as_ref().is_some_and(|client| client.q3_kind_is_qvm());
            let third_person = self
                .q3client
                .as_ref()
                .and_then(|client| client.q3_cvar_int("cg_thirdPerson"))
                .unwrap_or(0)
                != 0;
            let host = &self.host;
            let scene = &mut self.scene;
            let clients = &self.component_clients;
            let movement = self.component_movement_origin;
            let mut effects_fn = |camera: &SceneCamera, source: &SourceSceneOrder, weapon: SceneCamera| {
                let frame = host.effect_frame(camera, source, Some(&actor), fog.as_ref());
                let supplemental = scene.scene_supplemental(
                    &WorldViewInput::new(*camera, ViewTarget::Seat(host.seat_id()), render_time(&time)),
                    weapon,
                    infrared,
                )?;
                let mut operations = frame.operations.clone();
                operations.extend(supplemental);
                Ok(SeatEffectFrame { operations, ..frame })
            };
            let mut camera_fn = |camera: &SceneCamera| {
                q3_camera_override(
                    host,
                    clients,
                    movement,
                    viewport,
                    field_of_view,
                    camera,
                    &actor,
                    qvm,
                    third_person,
                )
            };
            if let Some(client) = self.q3client.as_mut() {
                client.q3_frame(&mut effects_fn, &mut camera_fn, &view, offset)?
            } else {
                None
            }
        };
        self.host.begin_frame();
        let area = self.viewport()?;
        if camera.viewport.x != area.x
            || camera.viewport.y != area.y
            || camera.viewport.width != area.width
            || camera.viewport.height != area.height
        {
            self.host.clear_viewport_mismatch(area, render_time(&time));
        }
        match native_frame {
            None => {
                let effects = effects.ok_or(SeatError::MissingEffects)?;
                let style = |index: i32| self.scene.scene_style(index, 12.0) / 12.0;
                let shadow_lights = self.host.shadow_lights(&camera, &style, viewer.as_ref());
                let binding = self.binding()?;
                let occlusions =
                    self.host
                        .ui_weapon_occlusion(&binding, self.prepared_time * 1000.0, !self.host.finale_active());
                let view = self.scene.scene_view(
                    input,
                    &effects.operations,
                    &shadow_lights,
                    infrared,
                    weapon_view_camera(&camera, &occlusions),
                )?;
                self.host.submit_world(view);
            }
            Some(commands) => {
                if commands
                    .iter()
                    .any(|command| matches!(command, RenderCommand::SwapBuffers))
                {
                    return Err(SeatError::SwapBuffers);
                }
                self.host.submit_native_commands(commands);
            }
        }
        let mut debug_lines = self.host.debug_lines();
        debug_lines.extend(self.component_lines.iter().cloned());
        if !debug_lines.is_empty() {
            self.host
                .draw_debug_lines(&debug_lines, &camera, self.host.debug_line_width());
        }
        if !self.world_texts.is_empty() {
            for text in &self.world_texts {
                if !self.world_fonts.contains_key(&text.content) {
                    return Err(SeatError::WorldTextFont);
                }
            }
            let texts = self.world_texts.clone();
            self.host.draw_world_text(&texts, &camera);
        }
        let controlled = self.component_view()?;
        let source_view = match &controlled {
            Some(view) => view.clone(),
            None => self.host.player_view(&actor),
        };
        let blend = source_view.blend.or(player_view.blend);
        if controlled.is_some() || self.q3client.is_none() {
            if let Some(blend) = blend {
                self.host.fill_blend(self.viewport()?, blend);
            }
        }
        if controlled.is_some() || self.q3client.is_none() {
            if let Some(damage) = &source_view.damage_blend {
                let viewport = qa_client::render::types::Rect {
                    x: camera.viewport.x as f32,
                    y: camera.viewport.y as f32,
                    width: camera.viewport.width as f32,
                    height: camera.viewport.height as f32,
                };
                let batches =
                    prepare_q2_damage_blend(damage, &viewport, self.assets.white_image(), DAMAGE_BLEND_BORDER);
                if !batches.is_empty() {
                    self.host.submit_damage(batches, camera.viewport);
                }
            }
        }
        self.host.finale_draw(self.prepared_time);
        let height_scale = (self.viewport()?.height as f64 / 480.0).max(1.0);
        self.host.draw_rerelease_story(&actor, height_scale);
        let native_replacement = self.component_clients.iter().any(|client| {
            client.frame.kind != SeatModKind::Quakec
                && client.frame.hud.as_ref().is_some_and(|hud| hud.replaces_status())
        });
        let qc_status = self
            .component_clients
            .iter()
            .find(|client| {
                client.frame.kind == SeatModKind::Quakec && matches!(client.frame.hud, Some(SeatModHud::Quakec { .. }))
            })
            .and_then(|client| match &client.frame.hud {
                Some(SeatModHud::Quakec { health, armor }) => Some(SeatQcHud {
                    health: *health,
                    armor: *armor,
                }),
                _ => None,
            });
        let weapon_hud = self.q3client.as_ref().map(|client| client.q3_weapon_hud_view());
        let weapon_hud_visible = !self.host.finale_active()
            && self.host.view_size().map(|size| size.size).unwrap_or(100.0) < 120.0
            && weapon_hud.and_then(|view| view.visible).unwrap_or(true);
        self.host.ui_draw(&SeatUiDrawArgs {
            binding: self.binding()?,
            time_ms: self.prepared_time * 1000.0,
            camera,
            weapon_hud_visible,
            story_suppressed: !self.host.rerelease_story_active(&actor),
            native_hud: native_replacement || self.q3client.is_some() || self.assets.has_native_q2(),
            aggregate_warning: weapon_hud.and_then(|view| view.aggregate_warning).unwrap_or(true),
            is_q3: self.q3client.is_some(),
            qc_status: if native_replacement { None } else { qc_status.clone() },
        });
        if self.native_q2_frame.is_some() && !native_replacement && qc_status.is_none() {
            let frame = self.native_q2_frame.clone().expect("native frame present");
            let binding = self.binding()?;
            self.host
                .ui_draw_native_hud(&frame, &binding, self.prepared_time * 1000.0);
        }
        for index in 0..self.component_clients.len() {
            let (kind, hud) = (
                self.component_clients[index].frame.kind,
                self.component_clients[index].frame.hud.clone(),
            );
            match (kind, hud) {
                (SeatModKind::Native, Some(SeatModHud::Native { mode, frame })) => {
                    let binding = self.binding()?;
                    let hud_state = &mut self.component_clients[index].hud;
                    self.host
                        .ui_draw_component_hud(&frame, &binding, self.prepared_time * 1000.0, hud_state, mode);
                }
                (SeatModKind::Qvm, _) => {
                    let owner = self.component_clients[index].owner.clone();
                    for (_, overlay) in self.component_effects.values() {
                        if let Some(overlay) = overlay {
                            if overlay.owner == owner {
                                (overlay.draw)(&SeatOverlayContext {
                                    camera,
                                    viewport: self.viewport()?,
                                    seat: self.host.seat_id(),
                                    time_ms: self.prepared_time * 1000.0,
                                });
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        let graph_viewport = Rect {
            x: camera.viewport.x - area.x,
            y: camera.viewport.y - area.y,
            width: camera.viewport.width,
            height: camera.viewport.height,
        };
        self.host.draw_graph_overlay(graph_viewport);
        if self.host.focus_is_console() {
            let viewport = self.viewport()?;
            let height = viewport.height * 50 / 100;
            self.host.draw_console(&SeatConsoleContext {
                width: viewport.width,
                height: viewport.height,
                console_height: height,
                time_ms: self.prepared_time * 1000.0,
            });
        }
        Ok(self.host.finish_frame())
    }

    /// Render a frame on a backend (donor `render`).
    pub fn render(&mut self, frame: RenderFrame, backend_id: u64) -> Result<(), SeatError> {
        if backend_id != self.backend_id {
            return Err(SeatError::ForeignRenderer);
        }
        self.host.execute_frame(frame);
        Ok(())
    }

    /// Close the presentation, aggregating failures (donor `close`).
    pub fn close(&mut self) -> Result<(), SeatError> {
        self.closed = true;
        for client in &mut self.component_clients {
            client.hud.clear();
        }
        self.component_clients.clear();
        self.component_effects.clear();
        let mut errors = Vec::new();
        for (_, handle) in self.world_fonts.drain() {
            if let Err(error) = self.host.close_font(handle) {
                errors.push(error.to_string());
            }
        }
        self.world_texts.clear();
        self.native_q2_frame = None;
        self.host.drawings_clear();
        self.component_lines.clear();
        if let Some(client) = self.q3client.as_mut() {
            client.q3_close();
        }
        self.host.ui_close();
        self.scene.scene_close();
        self.services.close();
        if self.assets.native_q2_owns_effects() {
            self.host.close_effects();
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(SeatError::Close(errors))
        }
    }
}

/// Q3 frame camera override without a seat borrow (donor `frame` camera callback).
#[allow(clippy::too_many_arguments)]
fn q3_camera_override<H: SeatHost>(
    host: &H,
    clients: &[SeatComponentClient],
    movement: Option<Vec3>,
    viewport: Rect,
    field_of_view: f32,
    camera: &SceneCamera,
    actor: &ActorId,
    qvm: bool,
    third_person: bool,
) -> Result<SceneCamera, SeatError> {
    if let Some(client) = clients.iter().find(|client| client.frame.has_view) {
        let controlled = host.mod_client_view(client.source, movement, host.player_view(actor).angles);
        return Ok(host.camera_override(&client_camera_for(&controlled, viewport, field_of_view)?));
    }
    let player = host.player_view(actor);
    if qvm {
        let kick = player.kick_angles.unwrap_or_default();
        return Ok(host.camera_override(&camera_with_kick(&camera_with_client_offset(camera, &player), kick)));
    }
    let offset = camera_with_client_offset(camera, &player);
    let posed = if third_person {
        offset
    } else {
        camera_with_character_death(&offset, &player)
    };
    let kick = player.kick_angles.unwrap_or_default();
    let sized = match host.view_size() {
        None => camera_with_kick(&posed, kick),
        Some(settings) => q1_view_camera(
            &camera_with_kick(&posed, kick),
            viewport,
            &settings,
            host.finale_active(),
        ),
    };
    Ok(host.camera_override(&sized))
}

/// Camera for a component-controlled view without a seat borrow.
fn client_camera_for(view: &SeatPlayerView, viewport: Rect, field_of_view: f32) -> Result<SceneCamera, SeatError> {
    let fov_x = view.field_of_view.unwrap_or(field_of_view);
    let fov_y = vertical_fov(fov_x, viewport);
    let camera = SceneCamera {
        origin: vec3(view.origin.x, view.origin.y, view.origin.z + view.view_height),
        axis: angles_to_axis(view.angles),
        viewport,
        projection: perspective_projection(fov_x, fov_y, 16384.0, 4.0)
            .map_err(|error| SeatError::Camera(error.to_string()))?,
        clip: qa_client::view::CameraClip::None,
    };
    Ok(camera_with_kick(&camera, view.kick_angles.unwrap_or_default()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_client::materials::state::CullFace as MaterialCullFace;
    use qa_client::render::types::{ImageSource, ResourceOwner};
    use qa_content::contract::{DopplerSelection, EnvironmentSelection, ProviderReference};
    use qa_core::identity::{IdentityOwner, ProviderId};
    use qa_core::math::{identity_mat4, vec4};
    use qa_core::time::{FrameContext, FramePhase};
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    #[derive(Debug, Default)]
    struct StubState {
        prints: Vec<String>,
        view_angles: Vec<Vec3>,
        ui_events: Vec<SeatSourceEvent>,
        fonts_closed: Vec<u64>,
        worlds_submitted: usize,
        native_submitted: Vec<Vec<RenderCommand>>,
        damages: Vec<usize>,
        blends: Vec<Vec4>,
        draws: Vec<SeatUiDrawArgs>,
        consoles: Vec<SeatConsoleContext>,
        executed: usize,
        begins: usize,
        clears: Vec<Rect>,
        graphs: Vec<Rect>,
        stories: Vec<f64>,
        finales: Vec<f64>,
        world_text_draws: Vec<usize>,
        debug_draws: Vec<usize>,
        refreshes_committed: Vec<u64>,
        refreshes_discarded: Vec<u64>,
        images_refreshed: usize,
        fonts_committed: usize,
        effects_closed: usize,
        ui_closed: bool,
        native_huds_prepared: usize,
        native_huds_drawn: usize,
        component_huds_prepared: Vec<SeatHudMode>,
        component_huds_drawn: Vec<SeatHudMode>,
    }

    #[derive(Clone)]
    struct StubHost {
        state: Rc<RefCell<StubState>>,
        seat: SeatId,
        client: ClientId,
        actor: ActorId,
        player: SeatPlayerView,
        window: (i32, i32),
        fov: f32,
        view_size: Option<Q1ViewSettings>,
        finale: bool,
        q3_builder: bool,
        focus_console: bool,
        scores: bool,
        sources: Vec<SeatModClientSource>,
        frames: HashMap<u64, SeatModClientFrame>,
        views: HashMap<u64, SeatPlayerView>,
        languages: HashMap<String, String>,
        story_active: bool,
        texts: Vec<SeatWorldText>,
        snapshot_lines: Vec<String>,
        snapshot_text: Vec<SeatWorldText>,
        debug_host_lines: Vec<String>,
        next_font: u64,
        fail_fonts: bool,
        fail_graphs: bool,
    }

    impl StubHost {
        fn new(owner: &IdentityOwner) -> Self {
            Self {
                state: Rc::new(RefCell::new(StubState::default())),
                seat: owner.seat(0),
                client: owner.client(1, 1),
                actor: owner.actor(1, 1),
                player: SeatPlayerView {
                    origin: vec3(10.0, 20.0, 30.0),
                    angles: vec3(0.0, 45.0, 0.0),
                    view_height: 22.0,
                    field_of_view: None,
                    kick_angles: None,
                    client_view_offset_delta: None,
                    foreign_character_death: false,
                    blend: None,
                    damage_blend: None,
                },
                window: (640, 480),
                fov: 90.0,
                view_size: None,
                finale: false,
                q3_builder: false,
                focus_console: false,
                scores: false,
                sources: Vec::new(),
                frames: HashMap::new(),
                views: HashMap::new(),
                languages: HashMap::new(),
                story_active: false,
                texts: Vec::new(),
                snapshot_lines: Vec::new(),
                snapshot_text: Vec::new(),
                debug_host_lines: Vec::new(),
                next_font: 1,
                fail_fonts: false,
                fail_graphs: false,
            }
        }
    }

    fn test_image() -> RendererImage {
        let authority = IdentityOwner::create("presentation-test").unwrap();
        RendererImage {
            owner: ResourceOwner::new(1, authority.session().clone(), 0),
            ordinal: 1,
            source: ImageSource::Generated {
                name: "white".to_string(),
            },
            width: 4,
            height: 4,
        }
    }

    fn test_frame() -> RenderFrame {
        RenderFrame {
            owner: test_image().owner,
            sequence: 1,
            commands: Vec::new(),
        }
    }

    impl SeatHost for StubHost {
        type UiSnapshot = String;
        type DebugLine = String;

        fn seat_id(&self) -> SeatId {
            self.seat.clone()
        }

        fn client_id(&self) -> ClientId {
            self.client.clone()
        }

        fn actor(&self) -> ActorId {
            self.actor.clone()
        }

        fn focus_is_console(&self) -> bool {
            self.focus_console
        }

        fn button_active(&self, name: &str) -> bool {
            name == "scores" && self.scores
        }

        fn builder_is_q3(&self) -> bool {
            self.q3_builder
        }

        fn set_view_angles(&mut self, angles: Vec3) {
            self.state.borrow_mut().view_angles.push(angles);
        }

        fn window_drawable_size(&self) -> (i32, i32) {
            self.window
        }

        fn field_of_view(&self) -> f32 {
            self.fov
        }

        fn view_size(&self) -> Option<Q1ViewSettings> {
            self.view_size
        }

        fn camera_override(&self, camera: &SceneCamera) -> SceneCamera {
            *camera
        }

        fn console_print(&mut self, text: &str) {
            self.state.borrow_mut().prints.push(text.to_string());
        }

        fn draw_console(&mut self, ctx: &SeatConsoleContext) {
            self.state.borrow_mut().consoles.push(ctx.clone());
        }

        fn player_view(&self, _actor: &ActorId) -> SeatPlayerView {
            self.player.clone()
        }

        fn world_texts(&self) -> Vec<SeatWorldText> {
            self.texts.clone()
        }

        fn mod_client_sources(&mut self) -> Vec<SeatModClientSource> {
            self.sources.clone()
        }

        fn mod_client_frame(&mut self, source: u64) -> Option<SeatModClientFrame> {
            self.frames.get(&source).cloned()
        }

        fn mod_client_assert_current(&self, _source: u64) -> Result<(), SeatError> {
            Ok(())
        }

        fn mod_client_generation(&self, source: u64) -> u64 {
            self.sources
                .iter()
                .find(|entry| entry.id == source)
                .map(|entry| entry.generation)
                .unwrap_or(0)
        }

        fn mod_client_view(&self, source: u64, _movement_origin: Option<Vec3>, _angles: Vec3) -> SeatPlayerView {
            self.views.get(&source).cloned().unwrap_or_else(|| self.player.clone())
        }

        fn bind_renderer_hardware(&mut self) {}

        fn effect_player_view(&self, _actor: &ActorId, camera: &SceneCamera) -> SeatEffectPlayerView {
            SeatEffectPlayerView {
                camera: *camera,
                infrared: false,
                blend: None,
            }
        }

        fn effect_frame(
            &self,
            _camera: &SceneCamera,
            _source: &SourceSceneOrder,
            _viewer: Option<&ActorId>,
            _fog: Option<&SceneFog>,
        ) -> SeatEffectFrame {
            SeatEffectFrame::empty()
        }

        fn shadow_lights(
            &self,
            _camera: &SceneCamera,
            _style: &dyn Fn(i32) -> f64,
            _viewer: Option<&ActorId>,
        ) -> Vec<SceneLight> {
            Vec::new()
        }

        fn chase_queries(&self) -> Option<&dyn Q1ChaseScene> {
            None
        }

        fn close_effects(&mut self) {
            self.state.borrow_mut().effects_closed += 1;
        }

        fn ui_snapshot(&self, time_ms: f64) -> String {
            format!("ui@{time_ms}")
        }

        fn ui_receive(&mut self, events: &[SeatSourceEvent]) {
            self.state.borrow_mut().ui_events.extend(events.iter().cloned());
        }

        fn ui_prepare(&mut self) {}

        fn ui_prepare_native_hud(
            &mut self,
            _frame: &NativeQ2HudFrame,
            _content: &ContentId,
            _binding: &SeatPresentationBinding,
            _time_ms: f64,
        ) {
            self.state.borrow_mut().native_huds_prepared += 1;
        }

        fn ui_prepare_component_hud(
            &mut self,
            _frame: &NativeQ2HudFrame,
            _content: &ContentId,
            _binding: &SeatPresentationBinding,
            _time_ms: f64,
            _hud: &mut ApplicationQ2NativeHud,
            mode: SeatHudMode,
            assert_current: &dyn Fn() -> Result<(), SeatError>,
        ) {
            assert_current().unwrap();
            self.state.borrow_mut().component_huds_prepared.push(mode);
        }

        fn ui_weapon_occlusion(&self, _binding: &SeatPresentationBinding, _time_ms: f64, _active: bool) -> Vec<Rect> {
            Vec::new()
        }

        fn ui_draw(&mut self, args: &SeatUiDrawArgs) {
            self.state.borrow_mut().draws.push(args.clone());
        }

        fn ui_draw_native_hud(&mut self, _frame: &NativeQ2HudFrame, _binding: &SeatPresentationBinding, _time_ms: f64) {
            self.state.borrow_mut().native_huds_drawn += 1;
        }

        fn ui_draw_component_hud(
            &mut self,
            _frame: &NativeQ2HudFrame,
            _binding: &SeatPresentationBinding,
            _time_ms: f64,
            _hud: &mut ApplicationQ2NativeHud,
            mode: SeatHudMode,
        ) {
            self.state.borrow_mut().component_huds_drawn.push(mode);
        }

        fn ui_close(&mut self) {
            self.state.borrow_mut().ui_closed = true;
        }

        fn stage_ui_image_refresh(&mut self) -> u64 {
            7
        }

        fn commit_ui_image_refresh(&mut self, handle: u64) {
            self.state.borrow_mut().refreshes_committed.push(handle);
        }

        fn discard_ui_image_refresh(&mut self, handle: u64) {
            self.state.borrow_mut().refreshes_discarded.push(handle);
        }

        fn refresh_ui_images(&mut self) {
            self.state.borrow_mut().images_refreshed += 1;
        }

        fn rerelease_language(&self, _seat: &SeatId) -> Option<String> {
            self.languages.get("language").cloned()
        }

        fn rerelease_item_visible(&self, _actor: &ActorId, _presentation: &ActorId) -> bool {
            true
        }

        fn rerelease_localize(&mut self, _seat: &SeatId, _content: &ContentId, text: &str) -> String {
            format!("L:{text}")
        }

        fn apply_rerelease_view(&self, _input: &mut WorldViewInput, _actor: &ActorId, _time: f64) {}

        fn draw_rerelease_story(&mut self, _actor: &ActorId, height_scale: f64) {
            self.state.borrow_mut().stories.push(height_scale);
        }

        fn rerelease_story_active(&self, _actor: &ActorId) -> bool {
            self.story_active
        }

        fn finale_active(&self) -> bool {
            self.finale
        }

        fn finale_receive(&mut self, _events: &[SeatSourceEvent]) {}

        fn finale_prepare(&mut self) {}

        fn finale_draw(&mut self, time: f64) {
            self.state.borrow_mut().finales.push(time);
        }

        fn drawings_receive(&mut self, _events: &[SeatSourceEvent]) {}

        fn drawings_prepare_graphs(&mut self) -> Result<(), SeatError> {
            if self.fail_graphs {
                return Err(SeatError::Host("graphs".to_string()));
            }
            Ok(())
        }

        fn drawings_snapshot(&self, _time: f64, _frame: u64) -> SeatDrawingSnapshot<String> {
            SeatDrawingSnapshot {
                lines: self.snapshot_lines.clone(),
                text: self.snapshot_text.clone(),
            }
        }

        fn drawings_clear(&mut self) {}

        fn load_font(&mut self, content: &ContentId) -> Result<u64, SeatError> {
            if self.fail_fonts {
                return Err(SeatError::Host("font".to_string()));
            }
            let handle = self.next_font;
            self.next_font += 1;
            let _ = content;
            Ok(handle)
        }

        fn close_font(&mut self, handle: u64) -> Result<(), SeatError> {
            self.state.borrow_mut().fonts_closed.push(handle);
            if self.fail_fonts {
                return Err(SeatError::Host(format!("close:{handle}")));
            }
            Ok(())
        }

        fn commit_image_fonts(&mut self) {
            self.state.borrow_mut().fonts_committed += 1;
        }

        fn begin_frame(&mut self) {
            self.state.borrow_mut().begins += 1;
        }

        fn clear_viewport_mismatch(&mut self, viewport: Rect, _time: RenderSourceTime) {
            self.state.borrow_mut().clears.push(viewport);
        }

        fn submit_world(&mut self, _view: qa_client::render::scene::world::PreparedWorldView) {
            self.state.borrow_mut().worlds_submitted += 1;
        }

        fn submit_native_commands(&mut self, commands: Vec<RenderCommand>) {
            self.state.borrow_mut().native_submitted.push(commands);
        }

        fn debug_lines(&self) -> Vec<String> {
            self.debug_host_lines.clone()
        }

        fn debug_line_width(&self) -> f32 {
            1.0
        }

        fn draw_debug_lines(&mut self, lines: &[String], _camera: &SceneCamera, _line_width: f32) {
            self.state.borrow_mut().debug_draws.push(lines.len());
        }

        fn draw_world_text(&mut self, texts: &[SeatWorldText], _camera: &SceneCamera) {
            self.state.borrow_mut().world_text_draws.push(texts.len());
        }

        fn fill_blend(&mut self, _viewport: Rect, blend: Vec4) {
            self.state.borrow_mut().blends.push(blend);
        }

        fn submit_damage(&mut self, batches: Vec<DrawBatch>, _viewport: Rect) {
            self.state.borrow_mut().damages.push(batches.len());
        }

        fn draw_graph_overlay(&mut self, viewport: Rect) {
            self.state.borrow_mut().graphs.push(viewport);
        }

        fn finish_frame(&mut self) -> RenderFrame {
            test_frame()
        }

        fn execute_frame(&mut self, _frame: RenderFrame) {
            self.state.borrow_mut().executed += 1;
        }
    }

    #[derive(Clone, Default)]
    struct StubQ3 {
        qvm: bool,
        camera: Option<SceneCamera>,
        cvars: HashMap<String, i32>,
        equipment: Option<bool>,
        poses: HashMap<ActorId, SeatBodyPose>,
        held: Vec<SceneHeldWeapon>,
        bodies: Vec<ScenePrimaryBody>,
        native: Option<Option<Vec<RenderCommand>>>,
        invoke_effects: bool,
        hud_view: SeatWeaponHudView,
    }

    impl SeatQ3Client for StubQ3 {
        fn q3_kind_is_qvm(&self) -> bool {
            self.qvm
        }

        fn q3_camera(&self) -> SceneCamera {
            self.camera.unwrap_or_else(test_camera)
        }

        fn q3_cvar_int(&self, name: &str) -> Option<i32> {
            self.cvars.get(name).copied()
        }

        fn q3_shared_equipment_view_visible(&self) -> Option<bool> {
            self.equipment
        }

        fn q3_prepare(
            &mut self,
            _frame: i32,
            _viewport: Rect,
            _presentations: &[ScenePresentation],
            _hud_visible: bool,
            _bodies: Vec<SceneBody>,
            _weapon_visible: bool,
            _native_view: bool,
        ) {
        }

        fn q3_body_pose(&self, actor: &ActorId) -> Option<SeatBodyPose> {
            self.poses.get(actor).copied()
        }

        fn q3_shared_held_weapons(&self) -> Vec<SceneHeldWeapon> {
            self.held.clone()
        }

        fn q3_shared_bodies(&self) -> Vec<ScenePrimaryBody> {
            self.bodies.clone()
        }

        fn q3_frame(
            &mut self,
            effects: &mut dyn FnMut(&SceneCamera, &SourceSceneOrder, SceneCamera) -> Result<SeatEffectFrame, SeatError>,
            camera: &mut dyn FnMut(&SceneCamera) -> Result<SceneCamera, SeatError>,
            _view: &SeatQ3View,
            _offset: Option<Vec3>,
        ) -> Result<Option<Vec<RenderCommand>>, SeatError> {
            if self.invoke_effects {
                let order = qa_client::render::scene::submissions::create_source_scene_order(Vec::new());
                let base = test_camera();
                effects(&base, &order, base)?;
                camera(&base)?;
            }
            Ok(self.native.clone().unwrap_or(None))
        }

        fn q3_supplemental_weapon_camera(&self, camera: &SceneCamera) -> SceneCamera {
            *camera
        }

        fn q3_weapon_hud_view(&self) -> SeatWeaponHudView {
            self.hud_view
        }

        fn q3_close(&mut self) {}
    }

    #[derive(Clone, Default)]
    struct StubScene {
        received: Vec<usize>,
        prepares: Vec<(Option<ActorId>, usize, f32, bool)>,
        views: usize,
        supplementals: usize,
    }

    impl SeatScene for StubScene {
        fn scene_receive(&mut self, events: &[SimulationPresentationEvent<()>]) {
            self.received.push(events.len());
        }

        fn scene_style(&self, _index: i32, absent: f64) -> f64 {
            absent
        }

        fn scene_styles(&self) -> (Vec<i32>, Vec<qa_client::materials::lighting::Q2LightStyle>) {
            (
                vec![256; 256],
                vec![
                    qa_client::materials::lighting::Q2LightStyle {
                        rgb: vec3(1.0, 1.0, 1.0),
                        white: 3.0,
                    };
                    256
                ],
            )
        }

        fn scene_prepare(&mut self, input: &ScenePrepareInput) -> Result<(), SceneError> {
            self.prepares.push((
                input.viewer.cloned(),
                input.presentations.len(),
                input.field_of_view,
                input.view_weapon_visible,
            ));
            Ok(())
        }

        fn scene_view(
            &mut self,
            _input: WorldViewInput,
            _operations: &[SceneOperation],
            _shadow_lights: &[SceneLight],
            _infrared: bool,
            _weapon_camera: SceneCamera,
        ) -> Result<qa_client::render::scene::world::PreparedWorldView, SceneError> {
            self.views += 1;
            Ok(qa_client::render::scene::world::PreparedWorldView {
                image_operations: Vec::new(),
                view: qa_client::render::types::RenderView {
                    state: qa_client::render::types::RenderViewState {
                        viewport: qa_client::render::types::Rect {
                            x: 0.0,
                            y: 0.0,
                            width: 1.0,
                            height: 1.0,
                        },
                        clear: None,
                        clip_plane: None,
                    },
                    target: qa_client::render::types::ViewTarget::Preview("test".to_string()),
                    time: RenderSourceTime::Seconds(0.0),
                    before_view: Vec::new(),
                    operations: Vec::new(),
                },
            })
        }

        fn scene_supplemental(
            &mut self,
            _input: &WorldViewInput,
            _weapon_camera: SceneCamera,
            _infrared: bool,
        ) -> Result<Vec<SceneOperation>, SceneError> {
            self.supplementals += 1;
            Ok(Vec::new())
        }

        fn scene_close(&mut self) {}
    }

    #[derive(Clone)]
    struct StubAssets {
        q1: bool,
        selection: PresentationSelection,
        native_frame: Option<NativeQ2HudFrame>,
        native_q2: bool,
        owns_effects: bool,
    }

    impl StubAssets {
        fn new() -> Self {
            Self {
                q1: false,
                selection: PresentationSelection {
                    doppler: DopplerSelection::Disabled,
                    environment: EnvironmentSelection::Disabled,
                    assets: ContentId("q1:test:assets:1".to_string()),
                    hud: ProviderReference {
                        provider: ProviderId::new("test", "hud"),
                        content: ContentId("q1:test:hud:1".to_string()),
                    },
                    effects: ProviderReference {
                        provider: ProviderId::new("test", "fx"),
                        content: ContentId("q1:test:fx:1".to_string()),
                    },
                    audio: ProviderReference {
                        provider: ProviderId::new("test", "audio"),
                        content: ContentId("q1:test:audio:1".to_string()),
                    },
                },
                native_frame: None,
                native_q2: false,
                owns_effects: false,
            }
        }
    }

    impl SeatContent for StubAssets {
        fn worldspawn_entities(&self) -> String {
            String::new()
        }

        fn world_is_q1_bsp(&self) -> bool {
            self.q1
        }

        fn mount_contents(&self) -> HashSet<ContentId> {
            HashSet::new()
        }

        fn numeric_timing_present(&self) -> bool {
            true
        }

        fn presentation_selection(&self) -> PresentationSelection {
            self.selection.clone()
        }

        fn white_image(&self) -> RendererImage {
            test_image()
        }

        fn material_registrations(&mut self) -> Vec<ShaderRegistration> {
            Vec::new()
        }

        fn has_native_q2(&self) -> bool {
            self.native_q2
        }

        fn native_q2_frame(&mut self) -> Option<NativeQ2HudFrame> {
            self.native_frame.clone()
        }

        fn native_q2_owns_effects(&self) -> bool {
            self.owns_effects
        }
    }

    impl Q1MessageAssets for StubAssets {
        fn product(&self, _content: &ContentId) -> crate::bootstrap::q1_localization::Q1MessageProduct {
            crate::bootstrap::q1_localization::Q1MessageProduct {
                q1_family: self.q1,
                rerelease: false,
            }
        }

        fn open(&mut self, _content: &ContentId, _path: &str) -> Option<Vec<u8>> {
            None
        }
    }

    impl Q1ServiceAssets for StubAssets {
        fn sky_face(&mut self, _content: &ContentId, _path: &str) -> Option<RendererImage> {
            None
        }

        fn missing_image(&self) -> RendererImage {
            test_image()
        }
    }

    type TestSeat = WorldSeatPresentation<StubHost, StubQ3, StubScene, StubAssets>;

    fn content() -> ContentId {
        ContentId("q1:test:scene:1".to_string())
    }

    fn test_camera() -> SceneCamera {
        SceneCamera {
            origin: vec3(0.0, 0.0, 0.0),
            axis: angles_to_axis(vec3(0.0, 0.0, 0.0)),
            projection: identity_mat4(),
            viewport: Rect {
                x: 0,
                y: 0,
                width: 640,
                height: 480,
            },
            clip: qa_client::view::CameraClip::None,
        }
    }

    fn seat_event(event: SeatEvent, _actor: &ActorId) -> SeatSourceEvent {
        SeatSourceEvent {
            event,
            owner: None,
            recipient: None,
            sequence: 1,
            content: content(),
            seconds: 0.0,
            source_entity: None,
        }
    }

    fn snapshot() -> WorldSnapshot {
        WorldSnapshot {
            frame: FrameContext {
                frame: 7,
                time: SourceTime::Seconds(2.0),
                elapsed: SourceTime::Milliseconds(16),
                phase: FramePhase::FrameEntry,
            },
            actors: Vec::new(),
            bodies: Vec::new(),
            inventories: Vec::new(),
        }
    }

    fn seat() -> (TestSeat, Rc<RefCell<StubState>>, IdentityOwner) {
        let owner = IdentityOwner::create("presentation-seat").unwrap();
        let host = StubHost::new(&owner);
        let state = Rc::clone(&host.state);
        let seat =
            WorldSeatPresentation::new(host, None, StubScene::default(), StubAssets::new(), 1, 11).expect("seat");
        (seat, state, owner)
    }

    fn hud_frame() -> NativeQ2HudFrame {
        NativeQ2HudFrame {
            protocol: qa_client::ui::types::Q2ProtocolFamily::Classic,
            stats: vec![0; 32],
            configstrings: BTreeMap::new(),
            layout: String::new(),
            inventory: Vec::new(),
            player_number: 0,
            server_frame: 0,
            time_ms: 0,
            frame_time_ms: None,
        }
    }

    #[test]
    fn seat_viewport_splits() {
        assert_eq!(
            seat_viewport(0, 1, 640, 480).unwrap(),
            Rect {
                x: 0,
                y: 0,
                width: 640,
                height: 480
            }
        );
        assert_eq!(
            seat_viewport(1, 2, 640, 480).unwrap(),
            Rect {
                x: 0,
                y: 240,
                width: 640,
                height: 240
            }
        );
        assert_eq!(
            seat_viewport(3, 4, 640, 480).unwrap(),
            Rect {
                x: 320,
                y: 240,
                width: 320,
                height: 240
            }
        );
        assert!(matches!(seat_viewport(0, 0, 640, 480), Err(SeatError::Layout)));
        assert!(matches!(seat_viewport(0, 5, 640, 480), Err(SeatError::Layout)));
        assert!(matches!(seat_viewport(2, 2, 640, 480), Err(SeatError::Layout)));
    }

    #[test]
    fn camera_helpers_cover_guards() {
        let camera = test_camera();
        assert_eq!(camera_with_kick(&camera, vec3(0.0, 0.0, 0.0)), camera);
        let kicked = camera_with_kick(&camera, vec3(0.0, 90.0, 0.0));
        assert_ne!(kicked.axis, camera.axis);
        let (seat, _, owner) = seat();
        let plain = seat.host.player_view(&owner.actor(1, 1));
        assert_eq!(camera_with_client_offset(&camera, &plain), camera);
        assert_eq!(camera_with_character_death(&camera, &plain), camera);
        let mut offset = plain.clone();
        offset.client_view_offset_delta = Some(vec3(1.0, 2.0, 3.0));
        assert_eq!(camera_with_client_offset(&camera, &offset).origin, vec3(1.0, 2.0, 3.0));
        let mut dead = plain.clone();
        dead.foreign_character_death = true;
        let snapped = camera_with_character_death(&camera, &dead);
        assert_eq!(snapped.origin, vec3(10.0, 20.0, 52.0));
    }

    #[test]
    fn publish_layout_validates() {
        let (mut seat, _, _) = seat();
        assert!(seat.publish_layout(0, 2).is_ok());
        assert!(seat.split_screen());
        assert!(matches!(seat.publish_layout(2, 2), Err(SeatError::Layout)));
        assert!(matches!(seat.publish_layout(0, 5), Err(SeatError::Layout)));
    }

    #[test]
    fn source_events_route_and_filter() {
        let (mut seat, state, owner) = seat();
        let actor = owner.actor(1, 1);
        let other = owner.actor(2, 1);
        let snap = snapshot();
        seat.prepare(&snap, &[], &[]).unwrap();
        let mine = seat_event(
            SeatEvent::Q1Message {
                player: actor.clone(),
                text: "hi".to_string(),
                args: Vec::new(),
                parts: Vec::new(),
                center: false,
            },
            &actor,
        );
        let mut foreign = mine.clone();
        foreign.event = SeatEvent::Q1Message {
            player: other.clone(),
            text: "yo".to_string(),
            args: Vec::new(),
            parts: Vec::new(),
            center: false,
        };
        let mut addressed = mine.clone();
        addressed.recipient = Some(other.clone());
        let reset = seat_event(
            SeatEvent::ViewReset {
                actor: actor.clone(),
                angles: vec3(1.0, 2.0, 3.0),
            },
            &actor,
        );
        let command = seat_event(
            SeatEvent::Q3ServerCommand {
                client: -1,
                text: "print hello".to_string(),
            },
            &actor,
        );
        seat.source_events(&[mine, foreign, addressed, reset, command]).unwrap();
        assert_eq!(seat.pending_q1.len(), 1);
        assert_eq!(state.borrow().view_angles, vec![vec3(1.0, 2.0, 3.0)]);
        assert_eq!(state.borrow().prints, vec!["hello".to_string()]);
        let ui = &state.borrow().ui_events;
        assert!(ui
            .iter()
            .all(|event| !matches!(event.event, SeatEvent::Q1Message { .. })));
        assert_eq!(ui.len(), 2);
    }

    #[test]
    fn source_events_match_server_command_clients() {
        let (mut seat, state, owner) = seat();
        let actor = owner.actor(1, 1);
        let snap = snapshot();
        seat.prepare(&snap, &[], &[]).unwrap();
        let slot = seat.host.client_id().slot() as i32;
        let addressed = seat_event(
            SeatEvent::Q3ServerCommand {
                client: slot,
                text: "chat hey".to_string(),
            },
            &actor,
        );
        let missed = seat_event(
            SeatEvent::Q3ServerCommand {
                client: slot + 50,
                text: "print no".to_string(),
            },
            &actor,
        );
        let other = seat_event(
            SeatEvent::Q3ServerCommand {
                client: -1,
                text: "cp ignore".to_string(),
            },
            &actor,
        );
        seat.source_events(&[addressed, missed, other]).unwrap();
        assert_eq!(state.borrow().prints, vec!["hey".to_string()]);
    }

    #[test]
    fn prepare_prints_unmirrored_messages() {
        let (mut seat, state, owner) = seat();
        let actor = owner.actor(1, 1);
        seat.receive(&[
            SeatSimulationEvent {
                message_text: Some("a".to_string()),
                source_sequence: Some(9),
            },
            SeatSimulationEvent {
                message_text: Some("b".to_string()),
                source_sequence: None,
            },
            SeatSimulationEvent {
                message_text: None,
                source_sequence: Some(9),
            },
        ]);
        let mut mirrored = seat_event(
            SeatEvent::Q2Help {
                text: "help".to_string(),
            },
            &actor,
        );
        mirrored.sequence = 9;
        seat.pending_q2.push(mirrored);
        let snap = snapshot();
        seat.prepare(&snap, &[], &[]).unwrap();
        assert_eq!(state.borrow().prints, vec!["b\n".to_string(), "L:help\n".to_string()]);
    }

    #[test]
    fn prepare_q1_resolves_and_forwards() {
        let (mut seat, state, owner) = seat();
        let actor = owner.actor(1, 1);
        let snap = snapshot();
        seat.prepare(&snap, &[], &[]).unwrap();
        let message = seat_event(
            SeatEvent::Q1Message {
                player: actor.clone(),
                text: "hello".to_string(),
                args: Vec::new(),
                parts: Vec::new(),
                center: false,
            },
            &actor,
        );
        seat.source_events(std::slice::from_ref(&message)).unwrap();
        assert!(state.borrow().ui_events.is_empty());
        seat.prepare(&snap, &[], &[]).unwrap();
        assert_eq!(state.borrow().prints, vec!["hello\n".to_string()]);
        assert_eq!(state.borrow().ui_events.len(), 1);
    }

    #[test]
    fn prepare_q2_prints_without_newline() {
        let (mut seat, state, owner) = seat();
        let actor = owner.actor(1, 1);
        let snap = snapshot();
        seat.prepare(&snap, &[], &[]).unwrap();
        let print = seat_event(
            SeatEvent::Q2Print {
                actor: Some(actor.clone()),
                text: "p".to_string(),
            },
            &actor,
        );
        let center = seat_event(
            SeatEvent::Q2Centerprint {
                actor: Some(actor.clone()),
                text: "c".to_string(),
            },
            &actor,
        );
        seat.source_events(&[print, center]).unwrap();
        seat.prepare(&snap, &[], &[]).unwrap();
        assert_eq!(state.borrow().prints, vec!["L:p".to_string()]);
        assert_eq!(state.borrow().ui_events.len(), 2);
    }

    #[test]
    fn prepare_component_clients_retains_huds() {
        let (mut seat, state, owner) = seat();
        let actor = owner.actor(1, 1);
        let token = PresentationOwner {
            provider: ProviderId::new("test", "seat"),
            generation: 1,
        };
        seat.host.sources = vec![SeatModClientSource {
            id: 3,
            owner: token.clone(),
            generation: 1,
            identity_content: content(),
        }];
        seat.host.frames.insert(
            3,
            SeatModClientFrame {
                kind: SeatModKind::Native,
                has_view: true,
                native: Some(SeatNativeFlags {
                    weapon_visible: false,
                    render_flags: 0,
                }),
                hud: Some(SeatModHud::Native {
                    mode: SeatHudMode::Overlay,
                    frame: hud_frame(),
                }),
            },
        );
        let snap = snapshot();
        seat.prepare(&snap, &[], &[]).unwrap();
        assert_eq!(seat.component_clients.len(), 1);
        assert_eq!(state.borrow().component_huds_prepared, vec![SeatHudMode::Overlay]);
        assert!(!seat.component_weapon_visible());
        seat.host.sources.clear();
        seat.host.frames.clear();
        seat.prepare(&snap, &[], &[]).unwrap();
        assert!(seat.component_clients.is_empty());
        assert!(seat.component_weapon_visible());
        let _ = actor;
    }

    #[test]
    fn replace_q3client_validates_owner() {
        let (mut seat, _, owner) = seat();
        let actor = owner.actor(1, 1);
        let seat_id = owner.seat(0);
        assert!(seat
            .replace_q3client(StubQ3::default(), &seat_id, &actor)
            .unwrap()
            .is_none());
        assert!(seat.q3client().is_some());
        let wrong = owner.actor(9, 1);
        assert!(matches!(
            seat.replace_q3client(StubQ3::default(), &seat_id, &wrong),
            Err(SeatError::ReplacementPlayer)
        ));
    }

    #[test]
    fn effect_merge_orders_polygons_and_caps_lights() {
        let (mut seat, _, owner) = seat();
        let actor = owner.actor(1, 1);
        let material = {
            let definition = qa_client::materials::material::ShaderDefinition {
                name: "x".to_string(),
                stages: Vec::new(),
                surface_parms: Vec::new(),
                cull: MaterialCullFace::Back,
                sort: None,
                sky: None,
                fog: None,
                sun: None,
                deforms: Vec::new(),
                polygon_offset: false,
                no_mipmaps: false,
                no_picmip: false,
                entity_mergable: false,
                portal_range: 0.0,
                clamp_time: 0.0,
                warnings: Vec::new(),
                compiler_directives: Vec::new(),
            };
            let finished = qa_client::materials::finish::finish_implicit_shader(
                &qa_client::materials::finish::FinishImplicitShaderInput {
                    name: "x".to_string(),
                    base_image: qa_client::materials::compile::RegisteredStage::Missing,
                    profile: qa_client::materials::compile::default_shader_profile(),
                    kind: qa_client::materials::finish::ImplicitShaderKind::Default,
                },
            )
            .unwrap();
            let mut table = qa_client::render::scene::material_registrations::MaterialRegistrationTable::default();
            table.admit(qa_client::materials::compile::CompiledMaterial {
                registered: qa_client::materials::compile::RegisteredExplicitShader {
                    definition: definition.clone(),
                    stages: Vec::new(),
                    sky: None,
                    outcome: qa_client::materials::compile::RegistrationOutcome::Defined,
                },
                finished,
                material: qa_client::materials::compile::shader_render_material(&definition).unwrap(),
            })
        };
        let polygon = SceneOperation::Group(qa_client::render::scene::submissions::SceneGroup {
            order: SceneGroupOrder::Source {
                material: material.clone(),
                source: qa_client::render::scene::submissions::SourceSurfaceOrder {
                    view: qa_client::render::scene::submissions::create_source_scene_order(Vec::new()),
                    entity: SourceEntityOrder::World,
                    surface: 0,
                    fog: 0,
                    dlight: 0,
                },
            },
            operations: Vec::new(),
        });
        let plain = SceneOperation::Group(qa_client::render::scene::submissions::SceneGroup {
            order: SceneGroupOrder::Sequence {
                phase: qa_client::render::scene::submissions::SequencePhase::Opaque,
            },
            operations: Vec::new(),
        });
        let lights = vec![
            DynamicLight {
                origin: vec3(0.0, 0.0, 0.0),
                radius: 1.0,
                color: vec3(1.0, 1.0, 1.0),
                additive: false,
            };
            40
        ];
        let effect: SeatComponentEffect = Rc::new(move |_, _, _| SeatEffectFrame {
            q3_admissions: Vec::new(),
            operations: vec![plain.clone(), polygon.clone()],
            lights: Vec::new(),
            q3_lights: lights.clone(),
        });
        let handle = seat.bind_component_effects(effect, None);
        let order = qa_client::render::scene::submissions::create_source_scene_order(Vec::new());
        let merged = seat.effect_frame(&test_camera(), &order, Some(&actor), None);
        assert_eq!(merged.operations.len(), 2);
        assert!(is_world_polygon(&merged.operations[0]));
        assert_eq!(merged.q3_lights.len(), 32);
        seat.unbind_component_effects(handle);
        let merged = seat.effect_frame(&test_camera(), &order, Some(&actor), None);
        assert!(merged.operations.is_empty());
    }

    #[test]
    fn frame_world_path_submits_and_draws() {
        let (mut seat, state, owner) = seat();
        let actor = owner.actor(1, 1);
        seat.host.focus_console = true;
        seat.host.player.damage_blend = Some(vec4(1.0, 0.0, 0.0, 0.5));
        let snap = snapshot();
        seat.prepare(&snap, &[], &[]).unwrap();
        let frame = seat.frame(&snap).unwrap();
        assert_eq!(frame.sequence, 1);
        let state = state.borrow();
        assert_eq!(state.begins, 1);
        assert_eq!(state.worlds_submitted, 1);
        assert_eq!(state.damages.len(), 1);
        assert_eq!(state.consoles.len(), 1);
        assert_eq!(state.consoles[0].console_height, 240);
        assert_eq!(state.draws.len(), 1);
        assert!(!state.draws[0].is_q3);
        let _ = actor;
    }

    #[test]
    fn frame_rejects_swap_buffers() {
        let (mut seat, _, owner) = seat();
        let actor = owner.actor(1, 1);
        let seat_id = owner.seat(0);
        let q3 = StubQ3 {
            native: Some(Some(vec![RenderCommand::SwapBuffers])),
            ..Default::default()
        };
        seat.replace_q3client(q3, &seat_id, &actor).unwrap();
        let snap = snapshot();
        seat.prepare(&snap, &[], &[]).unwrap();
        assert!(matches!(seat.frame(&snap), Err(SeatError::SwapBuffers)));
    }

    #[test]
    fn render_checks_backend() {
        let (mut seat, state, _) = seat();
        assert!(matches!(seat.render(test_frame(), 99), Err(SeatError::ForeignRenderer)));
        seat.render(test_frame(), 11).unwrap();
        assert_eq!(state.borrow().executed, 1);
    }

    #[test]
    fn close_aggregates_font_failures() {
        let (mut seat, state, _) = seat();
        seat.world_fonts.insert(content(), 4);
        seat.host.fail_fonts = true;
        assert!(matches!(seat.close(), Err(SeatError::Close(_))));
        assert_eq!(state.borrow().fonts_closed, vec![4]);
        assert!(state.borrow().ui_closed);
    }

    #[test]
    fn image_refresh_stages_and_commits() {
        let (mut seat, state, _) = seat();
        seat.world_fonts.insert(content(), 4);
        let staged = seat.stage_image_refresh().unwrap();
        assert_eq!(staged.ui, 7);
        assert_eq!(staged.fonts.len(), 1);
        seat.commit_image_refresh(staged);
        assert_eq!(state.borrow().refreshes_committed, vec![7]);
        assert_eq!(state.borrow().fonts_closed, vec![4]);
        assert_eq!(state.borrow().fonts_committed, 1);
    }

    #[test]
    fn camera_requires_numeric_timing_for_chase() {
        let owner = IdentityOwner::create("presentation-chase").unwrap();
        let mut host = StubHost::new(&owner);
        host.view_size = Some(Q1ViewSettings {
            size: 100.0,
            chase: Some(Q1ChaseSettings {
                back: 100.0,
                up: 4.0,
                right: 0.0,
            }),
            overlay_status: false,
        });
        struct NoTiming(StubAssets);
        impl Clone for NoTiming {
            fn clone(&self) -> Self {
                Self(self.0.clone())
            }
        }
        impl SeatContent for NoTiming {
            fn worldspawn_entities(&self) -> String {
                String::new()
            }

            fn world_is_q1_bsp(&self) -> bool {
                false
            }

            fn mount_contents(&self) -> HashSet<ContentId> {
                HashSet::new()
            }

            fn numeric_timing_present(&self) -> bool {
                false
            }

            fn presentation_selection(&self) -> PresentationSelection {
                self.0.presentation_selection()
            }

            fn white_image(&self) -> RendererImage {
                test_image()
            }

            fn material_registrations(&mut self) -> Vec<ShaderRegistration> {
                Vec::new()
            }

            fn has_native_q2(&self) -> bool {
                false
            }

            fn native_q2_frame(&mut self) -> Option<NativeQ2HudFrame> {
                None
            }

            fn native_q2_owns_effects(&self) -> bool {
                false
            }
        }
        impl Q1MessageAssets for NoTiming {
            fn product(&self, _content: &ContentId) -> crate::bootstrap::q1_localization::Q1MessageProduct {
                crate::bootstrap::q1_localization::Q1MessageProduct {
                    q1_family: false,
                    rerelease: false,
                }
            }

            fn open(&mut self, _content: &ContentId, _path: &str) -> Option<Vec<u8>> {
                None
            }
        }
        impl Q1ServiceAssets for NoTiming {
            fn sky_face(&mut self, _content: &ContentId, _path: &str) -> Option<RendererImage> {
                None
            }

            fn missing_image(&self) -> RendererImage {
                test_image()
            }
        }
        let seat: WorldSeatPresentation<StubHost, StubQ3, StubScene, NoTiming> =
            WorldSeatPresentation::new(host, None, StubScene::default(), NoTiming(StubAssets::new()), 1, 11)
                .expect("seat");
        assert!(matches!(seat.camera(), Err(SeatError::ChaseTiming)));
    }
}
