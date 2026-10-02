//! Port of Quake-Anthology-TS `src/app/bootstrap/mod-presentation.ts`
//!
//! One original component cgame instance for one live source generation and
//! viewing seat. The donor builds guest pieces directly
//! (`ApplicationQ3Assets`, `QvmFiles`, `QvmClientScripts`, `QvmModPresentation`
//! ...) and drives them with `async` calls; the host here cannot lend the
//! owner into guest callbacks, so guest construction, driving, checkpoints,
//! and teardown live behind [`ModPresentationBackend`] (implemented by the
//! host next to this owner), the live world side behind
//! [`ModPresentationSource`], and the donor `host(call)` syscall chain behind
//! [`ModSyscalls`], which the owner implements. The host runs the donor chain
//! order inside guest traps (see [`ModPresentationBackend`); the owner keeps
//! the dispatch order consequences: HUD gating, command registration
//! tracking, the collision-map gate, and the cvar-gated developer print).
//!
//! Sync collapses: the donor `await`s core calls, `nextFrame`, and media
//! delivery. The guest ports are synchronous, so every drive is a plain call;
//! `current` stays lazy (a closure) while media delivery itself is sync.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use qa_client::render::scene::submissions::SceneOperation;
use qa_client::render::scene::world::WorldViewInput;
use qa_client::render::types::RenderCommand;
use qa_content::contract::{ComponentPresentationMediaRequest, ContentId, ModuleIdentity, PresentationOwner};
use qa_content::mounts::MountedContent;
use qa_content::q3::presentation::refdef::RDF_NOWORLDMODEL;
use qa_content::q3::presentation::resources::ResourceHandleOwner;
use qa_content::q3::presentation::scene::{Q3PresentedScene, Q3SceneContent, Rect};
use qa_core::cmd::Dialect;
use qa_core::cvar::{CvarError, CvarRegistry, CvarSaveState, SavedCvarState};
use qa_core::identity::{ActorId, SavedActorId, SeatId};
use qa_core::math::{Axis, Vec3, Vec4};
use qa_guest::qvm::mod_presentation::QvmModPresentationDeclaration;
use qa_guest::qvm::mod_presentation_checkpoint::SourceGameState;
use qa_guest::qvm::mod_provider::{ModuleId, QvmAbi, QvmArtifact};
use qa_net::q3::{MessageMode, Q3MsgError, Q3MsgReader, Q3MsgWriter, MAX_MESSAGE_LENGTH};
use qa_net::q3_net::{
    read_delta_entity, read_delta_player_state, write_delta_entity, write_delta_player_state, Q3EntityState,
    Q3NetError, Q3Product, ENTITY_NUMBER_BITS,
};
use qa_world::collision::q3::settings::CollisionMapSettings;
use qa_world::save::value::str as save_str;
use qa_world::save::value::{arr, boolean, encode_checkpoint_value, int, num, obj, SaveJson, SaveReader};
use qa_world::WorldError;

use crate::bootstrap::audio::q3::Q3SeatAudioOperation;
use crate::bootstrap::component_bodies::ComponentBody;
use crate::bootstrap::component_scene::{
    select_component_scene, ComponentSceneActor, ComponentSceneCommandView, ComponentSceneContext, ComponentSceneError,
    ComponentScenePublication, ComponentSceneSnapshot,
};
use crate::bootstrap::q3_client::overlay::Q3OverlayCommand;
use crate::bootstrap::q3_client::qvm_display::QvmDisplayRenderer;
use crate::bootstrap::q3_client::scene::Q3SceneRenderOptions;
use crate::bootstrap::q3_client::services::Q3ServiceTextDraw;
use crate::bootstrap::q3_client::visibility::ApplicationQ3SceneQueries;
use crate::bootstrap::simulation::q3::types::Q3SourcePlayerEvent;

/// Failure of a component presentation operation.
#[derive(Debug)]
pub enum ModPresentationError<E> {
    /// Donor `Error` with the donor message.
    Presentation(String),
    /// Guest backend failure.
    Backend(E),
    /// Cvar failure.
    Cvar(CvarError),
    /// Checkpoint value failure.
    World(WorldError),
    /// Snapshot codec failure.
    Net(Q3NetError),
    /// Scene selection failure.
    Scene(ComponentSceneError),
}

impl<E: std::fmt::Display> std::fmt::Display for ModPresentationError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Presentation(message) => write!(f, "{message}"),
            Self::Backend(error) => write!(f, "{error}"),
            Self::Cvar(error) => write!(f, "{error}"),
            Self::World(error) => write!(f, "{error}"),
            Self::Net(error) => write!(f, "{error}"),
            Self::Scene(error) => write!(f, "{error}"),
        }
    }
}

impl<E: std::error::Error + 'static> std::error::Error for ModPresentationError<E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Presentation(_) => None,
            Self::Backend(error) => Some(error),
            Self::Cvar(error) => Some(error),
            Self::World(error) => Some(error),
            Self::Net(error) => Some(error),
            Self::Scene(error) => Some(error),
        }
    }
}

impl<E> From<CvarError> for ModPresentationError<E> {
    fn from(error: CvarError) -> Self {
        Self::Cvar(error)
    }
}

impl<E> From<WorldError> for ModPresentationError<E> {
    fn from(error: WorldError) -> Self {
        Self::World(error)
    }
}

impl<E> From<Q3NetError> for ModPresentationError<E> {
    fn from(error: Q3NetError) -> Self {
        Self::Net(error)
    }
}

impl<E> From<Q3MsgError> for ModPresentationError<E> {
    fn from(error: Q3MsgError) -> Self {
        Self::Net(Q3NetError::from(error))
    }
}

impl<E> From<ComponentSceneError> for ModPresentationError<E> {
    fn from(error: ComponentSceneError) -> Self {
        Self::Scene(error)
    }
}

/// Print sink (donor `print(text)`).
pub type PrintCallback = Rc<dyn Fn(&str)>;
/// Next-frame pump (donor `nextFrame()`; sync collapse of the promise).
pub type NextFrameCallback = Rc<dyn Fn()>;
/// View origin sampler (donor `viewOrigin()`).
pub type ViewOriginCallback = Rc<dyn Fn() -> Vec3>;
/// View axis sampler (donor `viewAxis()`).
pub type ViewAxisCallback = Rc<dyn Fn() -> Axis>;
/// Free-memory sampler in bytes (donor `freemem`).
pub type FreeMemoryCallback = Rc<dyn Fn() -> u64>;
/// Media delivery (donor `presentationMedia(request, initializing, current)`;
/// `current` stays a lazy closure; the promise collapses to a result).
pub type ModPresentationMedia =
    Rc<dyn Fn(&ComponentPresentationMediaRequest, bool, &dyn Fn() -> bool) -> Result<(), String>>;
/// Scalar fallback (donor `scalar(call, owner)` over the host guest-call type).
pub type ModScalarCallback<B> =
    Rc<dyn Fn(&<B as ModPresentationBackend>::HostCall, &ApplicationModPresentation<B>) -> Option<i32>>;

/// Presentation clock (donor `ApplicationQ3ServiceOptions["clock"]`).
pub trait ModPresentationClock {
    /// Wall time in milliseconds (donor `clock.now()`).
    fn now(&self) -> f64;
    /// Frame ordinal (donor `clock.frameNumber()`).
    fn frame_number(&self) -> i64;
}

/// Asset inputs (donor `ApplicationAssets` surface used here).
pub trait ModPresentationAssets {
    /// Requested collision-map geometry path
    /// (donor `assets.content.recipe.map.geometry.requestedPath`).
    fn map_geometry_path(&self) -> &str;
    /// World-map leaf count (donor `assets.world.map.leaves.length`).
    fn world_leaf_count(&self) -> i32;
}

/// Scene queries (donor `SharedSceneQueries` surface used here).
pub trait ModSceneQueries: ApplicationQ3SceneQueries {
    /// Whether native Q3 clip models back collision
    /// (donor `queries.nativeQ3ClipModels() !== null`).
    fn has_native_collision(&self) -> bool;
}

/// Cgame audio frame (donor `receiveCgameFrame` argument).
#[derive(Debug, Clone)]
pub struct ModCgameAudioFrame {
    /// Presenting content.
    pub content: ContentId,
    /// Owning provider.
    pub owner: ActorId,
    /// Owning seat.
    pub seat: SeatId,
    /// Audio operations.
    pub operations: Vec<Q3SeatAudioOperation>,
}

/// Cgame audio sink (donor `options.audio.receiveCgameFrame`).
pub trait ModAudioSink {
    /// Receive one cgame audio frame.
    fn receive_cgame_frame(&self, frame: ModCgameAudioFrame);
}

/// Presentation output without audio
/// (donor `Omit<ApplicationQ3ServiceOptions["output"], "audio">`; outputs can
/// throw, so submissions are fallible).
pub trait ModPresentationOutput {
    /// Publish a presented scene.
    fn submit_scene(&self, scene: Q3PresentedScene) -> Result<(), String>;
    /// Emit a render command.
    fn submit_command(&self, command: RenderCommand) -> Result<(), String>;
    /// Emit a text draw.
    fn submit_text(&self, draw: Q3ServiceTextDraw) -> Result<(), String>;
    /// Move the listener.
    fn submit_listener(&self, origin: Vec3, axis: Axis) -> Result<(), String>;
}

/// Cgame command host
/// (donor `Extract<QvmCommonServices, { readonly role: "cgame" }>["commands"]`;
/// hosts throw out of every command, so all four are fallible).
pub trait ModCommandHost {
    /// Append console text.
    fn append_command(&self, text: &str) -> Result<(), String>;
    /// Register a console command.
    fn register_command(&self, name: &str) -> Result<(), String>;
    /// Remove a console command.
    fn remove_command(&self, name: &str) -> Result<(), String>;
    /// Send a reliable client command.
    fn reliable_command(&self, text: &str) -> Result<(), String>;
}

/// System cinematics (donor `systemCinematics` value or factory over
/// `Pick<ApplicationModPresentation, "cvars" | "fileMounts">`).
pub enum ModPresentationCinematics<B: ModPresentationBackend> {
    /// Ready host value.
    Host(B::Cinematics),
    /// Factory over the presentation cvars and file mounts.
    Factory(ModCinematicsFactory<B>),
}

/// System cinematics factory type (fallible: the donor factory throws
/// through its scope append).
pub type ModCinematicsFactory<B> =
    Rc<dyn Fn(&CvarRegistry, &MountedContent) -> Result<<B as ModPresentationBackend>::Cinematics, String>>;

/// Presentation options (donor `ApplicationModPresentationOptions`; the guest
/// backend is a separate `&mut B` on every driving call because the host owns
/// it next to this owner).
pub struct ApplicationModPresentationOptions<B: ModPresentationBackend> {
    /// Asset inputs.
    pub assets: Rc<dyn ModPresentationAssets>,
    /// Cgame audio sink.
    pub audio: Rc<dyn ModAudioSink>,
    /// Scene queries.
    pub queries: Rc<dyn ModSceneQueries>,
    /// Viewing seat.
    pub seat: SeatId,
    /// Viewing actor.
    pub viewer: ActorId,
    /// 2D viewport.
    pub viewport: Rect,
    /// Display renderer, when display traps are served.
    pub renderer: Option<Rc<QvmDisplayRenderer>>,
    /// Live source, prepared module, and owner.
    pub source: Rc<dyn ModPresentationSource>,
    /// Presentation clock.
    pub clock: Rc<dyn ModPresentationClock>,
    /// Presentation output.
    pub output: Rc<dyn ModPresentationOutput>,
    /// System cinematics value or factory.
    pub system_cinematics: Option<ModPresentationCinematics<B>>,
    /// Command host (absent registries make command traps silent no-ops).
    pub commands: Option<Rc<dyn ModCommandHost>>,
    /// Media delivery sink (absent sinks reject music/remap traps).
    pub presentation_media: Option<ModPresentationMedia>,
    /// Print sink.
    pub print: PrintCallback,
    /// View origin sampler.
    pub view_origin: ViewOriginCallback,
    /// View axis sampler.
    pub view_axis: Option<ViewAxisCallback>,
    /// Next-frame pump.
    pub next_frame: NextFrameCallback,
    /// Scalar fallback.
    pub scalar: Option<ModScalarCallback<B>>,
    /// Free-memory sampler.
    pub free_memory: FreeMemoryCallback,
}

/// Admitted scene publication pair (donor `source.scene()` result).
#[derive(Debug, Clone)]
pub struct ModScenePublication {
    /// Current publication.
    pub current: ComponentScenePublication<SourceGameState>,
    /// Baseline publication, when admitted.
    pub baseline: Option<ComponentScenePublication<SourceGameState>>,
}

/// Live component source, prepared module, and owner in one handle (donor
/// `ActiveModPresentation`): the host must use a single object per
/// presentation so identity comparison covers its source, prepared module,
/// and owner together.
pub trait ModPresentationSource {
    /// Source generation (donor `source.generation`).
    fn generation(&self) -> u64;
    /// Whether a viewer is live (donor `source.live(viewer)`).
    fn live(&self, viewer: &ActorId) -> bool;
    /// Assert the source is current (donor `source.assertCurrent()`).
    fn assert_current(&self) -> Result<(), String>;
    /// Live module identity (donor `source.module`).
    fn live_module(&self) -> ModuleId;
    /// Prepared module identity (donor `prepared.source`).
    fn prepared_module(&self) -> ModuleId;
    /// Live module ABI (donor `source.abiProfile`).
    fn source_abi(&self) -> QvmAbi;
    /// Prepared declaration (donor `prepared.declaration`).
    fn declaration(&self) -> QvmModPresentationDeclaration;
    /// Prepared artifact (donor `prepared.artifact.module`).
    fn core_artifact(&self) -> QvmArtifact;
    /// Prepared declaration checkpoint image, compared byte-wise across
    /// save/load (donor `encodeCheckpointValue(prepared.declaration)`).
    fn declaration_checkpoint(&self) -> SaveJson;
    /// Presenting content (donor `identity.source.content`).
    fn identity_content(&self) -> ContentId;
    /// Source identity checkpoint image, compared byte-wise across save/load
    /// (donor `source.identity`).
    fn identity_checkpoint(&self) -> SaveJson;
    /// Prepared contract module (donor `prepared.artifact.module`).
    fn prepared_contract_module(&self) -> ModuleIdentity;
    /// Deliver an admitted client command (donor `source.clientCommand`):
    /// `Ok(false)` means the source admits no receiver.
    fn client_command(&self, viewer: &ActorId, arguments: &[String]) -> Result<bool, String>;
    /// Prepared source owner (donor `prepared.source.id`).
    fn prepared_source_id(&self) -> ActorId;
    /// Presentation owner (donor `source.owner`).
    fn presentation_owner(&self) -> PresentationOwner;
    /// Source context for a viewer, when admitted
    /// (donor `source.context(viewer)`).
    fn scene_context(&self, viewer: &ActorId) -> Option<ComponentSceneContext<SourceGameState>>;
    /// Admitted scene publication (donor `source.scene()`).
    fn scene_publication(&self) -> Option<ModScenePublication>;
    /// Actor owning a source slot (donor `source.actor(slot)`).
    fn actor_for_slot(&self, slot: i32) -> Option<ActorId>;
    /// Source file mounts (donor `source.files?.mounts`).
    fn source_mounts(&self) -> Option<Rc<MountedContent>>;
    /// Source files writability (donor `source.files?.writable`).
    fn source_files_writable(&self) -> Option<bool>;
}

/// Checkpoint value plus actor resolver for restoration.
type CheckpointRestore<'a, 'b> = Option<(&'a SaveJson, &'b dyn Fn(&SavedActorId) -> Option<ActorId>)>;

/// Scene context with its nested baseline.
type SceneContextPair = (
    ComponentSceneContext<SourceGameState>,
    Option<ComponentSceneContext<SourceGameState>>,
);

/// Presentation context for one frame (donor `context()` result).
#[derive(Debug, Clone)]
pub struct ModPresentationContext {
    /// Source context, or the selected scene context for scene runtimes.
    pub scene: ComponentSceneContext<SourceGameState>,
    /// Scene baseline for scene runtimes (donor `scene.baseline`).
    pub baseline: Option<ComponentSceneContext<SourceGameState>>,
    /// Frame time in milliseconds.
    pub frame_time_ms: f64,
    /// Presentation time in milliseconds (scene runtimes only).
    pub time_ms: Option<f64>,
    /// View origin.
    pub view_origin: Vec3,
    /// View axis, when sampled.
    pub view_axis: Option<Axis>,
}

/// Whether a declaration selects the scene runtime
/// (donor `prepared.declaration.runtime === "qvm-scene"`).
fn declaration_is_scene(declaration: &QvmModPresentationDeclaration) -> bool {
    matches!(declaration, QvmModPresentationDeclaration::Scene(_))
}

/// Whether a declaration carries HUD calls (donor `declaration.hud`).
fn declaration_hud(declaration: &QvmModPresentationDeclaration) -> bool {
    match declaration {
        QvmModPresentationDeclaration::Scene(presentation) => presentation.hud.is_some(),
        QvmModPresentationDeclaration::PlayerEvents(presentation) => presentation.hud.is_some(),
    }
}

/// Declaration cvars (scene runtimes only; donor `declaration.cvars`).
fn declaration_cvars(declaration: &QvmModPresentationDeclaration) -> &[(String, String)] {
    match declaration {
        QvmModPresentationDeclaration::Scene(presentation) => &presentation.cvars,
        QvmModPresentationDeclaration::PlayerEvents(_) => &[],
    }
}

/// Gameplay ABI (donor `prepared.declaration.gameplay.abiProfile`).
fn declaration_gameplay_abi(declaration: &QvmModPresentationDeclaration) -> QvmAbi {
    match declaration {
        QvmModPresentationDeclaration::Scene(presentation) => presentation.gameplay.abi,
        QvmModPresentationDeclaration::PlayerEvents(presentation) => presentation.gameplay.abi,
    }
}

/// Services construction parameters (donor `createApplicationQ3Services`
/// argument; output callbacks become [`ModSyscalls`] submissions, the actor
/// lookup becomes `actor_for_slot`, and the clock stays live behind
/// [`ModSyscalls::milliseconds`]).
pub struct ModServicesParams<B: ModPresentationBackend> {
    /// Viewing seat.
    pub seat: SeatId,
    /// 2D viewport.
    pub viewport: Rect,
    /// Owning provider (donor `prepared.source.id`).
    pub owner: ActorId,
    /// Resource handle owner (donor `"client"`).
    pub resource_handles: ResourceHandleOwner,
    /// System cinematics, when configured.
    pub system_cinematics: Option<B::Cinematics>,
    /// Collision settings (donor `collisionSettings`).
    pub collision_settings: CollisionMapSettings,
    /// Print sink.
    pub print: PrintCallback,
}

/// Core construction parameters (donor `new QvmModPresentation` prepared
/// triple; the host closure, context, actor, live, and assert callbacks
/// become [`ModSyscalls`]).
#[derive(Debug, Clone)]
pub struct ModCoreParams {
    /// Prepared artifact (donor `prepared.artifact.module`).
    pub artifact: QvmArtifact,
    /// Prepared module identity (donor `prepared.source`).
    pub source: ModuleId,
    /// Prepared declaration (donor `prepared.declaration`).
    pub declaration: QvmModPresentationDeclaration,
}

/// Guest backend (donor constructors and `async` drives): the host owns the
/// backend next to the owner and passes `&mut B` into every driving call.
/// Construction order and close order match the donor; the host wires the same
/// queries, audio, and asset handles the options reference when it builds the
/// backend (shared clip models read the queries; services read media, audio,
/// and the live clock through [`ModSyscalls`]).
///
/// Driving calls hand the owner to the guest as `&mut dyn ModSyscalls<Self>`.
/// Inside guest traps the host runs the donor `host(call)` order: assert
/// current, then reject unless the role is cgame and files, scripts,
/// collision, and marks are ready; music start/stop and shader remap through
/// [`ModSyscalls::deliver_media`]; `CG_UPDATESCREEN` through
/// [`ModSyscalls::next_frame`]; `CG_GETGAMESTATE` through
/// [`ModSyscalls::game_state`]; then the common syscalls over the owner cvars,
/// print, milliseconds, core arguments, and the [`ModSyscalls`] command
/// methods; then display (when [`ModSyscalls::display_renderer`] is set),
/// cinematic, file, script, render, audio, collision (gated by
/// [`ModSyscalls::check_collision_map`]), and mark syscalls; then
/// [`ModSyscalls::scalar`]; then `CG_MEMORY_REMAINING` through
/// [`ModSyscalls::memory_remaining`]; anything else rejects. Output callbacks
/// become [`ModSyscalls`] submissions (`submit_scene`, `submit_overlay`,
/// `submit_text`, `submit_listener`, `emit_audio`), so HUD gating,
/// color tracking, and audio queueing stay in the owner.
pub trait ModPresentationBackend {
    /// Backend failure.
    type Error: std::error::Error + 'static;
    /// Q3 media (donor `ApplicationQ3Assets`).
    type Media;
    /// Q3 services (donor `ApplicationQ3Services`).
    type Services;
    /// Presentation core (donor `QvmModPresentation`).
    type Core;
    /// Scene renderer (donor `ApplicationQ3SceneRenderer`).
    type Renderer;
    /// Guest files (donor `QvmFiles`).
    type Files;
    /// Client scripts (donor `QvmClientScripts`).
    type Scripts;
    /// Collision models (donor `SourceClipModels | SharedQvmClientClipModels`).
    type Collision;
    /// Mark projector (donor `worldMarkProjector` result).
    type Marks;
    /// System cinematics value.
    type Cinematics;
    /// Guest host-call record (donor `QvmHostCall`).
    type HostCall;

    /// Create media (donor `ApplicationQ3Assets.create`).
    fn create_media(&mut self, content: &ContentId, print: PrintCallback) -> Result<Self::Media, Self::Error>;
    /// Media file mounts (donor `media.provider.mounts`).
    fn media_mounts(&self, media: &Self::Media) -> Rc<MountedContent>;
    /// Create collision models: native source models when `native`, else
    /// shared client models over the host queries and cvars (donor
    /// `SourceClipModels | SharedQvmClientClipModels` branch).
    fn create_collision(
        &mut self,
        native: bool,
        map: &str,
        cvars: Rc<RefCell<CvarRegistry>>,
    ) -> Result<Self::Collision, Self::Error>;
    /// Create the mark projector (donor `worldMarkProjector`).
    fn create_marks(&mut self) -> Result<Self::Marks, Self::Error>;
    /// Create services (donor `createApplicationQ3Services`).
    fn create_services(&mut self, params: ModServicesParams<Self>) -> Result<Self::Services, Self::Error>
    where
        Self: Sized;
    /// Create the scene renderer (donor `new ApplicationQ3SceneRenderer`).
    fn create_renderer(
        &mut self,
        media: &Self::Media,
        services: &Self::Services,
    ) -> Result<Self::Renderer, Self::Error>;
    /// Create guest files (donor `new QvmFiles`).
    fn create_files(
        &mut self,
        mounts: Rc<MountedContent>,
        writable: Option<bool>,
        print: PrintCallback,
    ) -> Result<Self::Files, Self::Error>;
    /// Create client scripts (donor `new QvmClientScripts`; the donor script
    /// globals fold into the backend scripts lifecycle).
    fn create_scripts(
        &mut self,
        mounts: Rc<MountedContent>,
        writable: Option<bool>,
        print: PrintCallback,
    ) -> Result<Self::Scripts, Self::Error>;
    /// Create the presentation core (donor `new QvmModPresentation`).
    fn create_core(&mut self, params: ModCoreParams) -> Result<Self::Core, Self::Error>;
    /// Initialize the core (donor `core.initialize`).
    fn initialize_core(
        &mut self,
        core: &mut Self::Core,
        baseline_sequence: i64,
        syscalls: &mut dyn ModSyscalls<Self>,
    ) -> Result<(), Self::Error>
    where
        Self: Sized;
    /// Advance one frame (donor `core.advance`).
    fn advance_core(
        &mut self,
        core: &mut Self::Core,
        sequence: i64,
        syscalls: &mut dyn ModSyscalls<Self>,
    ) -> Result<(), Self::Error>
    where
        Self: Sized;
    /// Consume a player event (donor `core.consume`; the host converts the
    /// source event to the guest record).
    fn consume_event(
        &mut self,
        core: &mut Self::Core,
        event: &Q3SourcePlayerEvent,
        sequence: i64,
        syscalls: &mut dyn ModSyscalls<Self>,
    ) -> Result<(), Self::Error>
    where
        Self: Sized;
    /// Run a console command (donor `core.consoleCommand`).
    fn console_command(
        &mut self,
        core: &mut Self::Core,
        arguments: &[String],
        syscalls: &mut dyn ModSyscalls<Self>,
    ) -> Result<bool, Self::Error>
    where
        Self: Sized;
    /// Draw the HUD (donor `core.drawHud`).
    fn draw_hud(
        &mut self,
        core: &mut Self::Core,
        sequence: i64,
        syscalls: &mut dyn ModSyscalls<Self>,
    ) -> Result<(), Self::Error>
    where
        Self: Sized;
    /// Capture bodies (donor `frame` body mapping): one [`ComponentBody`] per
    /// core body with the owner, content, and frame time attached, numeric
    /// custom shader/skin handles resolved through services, and ref-entity
    /// pass bytes decoded into passes.
    fn capture_bodies(
        &mut self,
        core: &Self::Core,
        services: &Self::Services,
        owner: &PresentationOwner,
        content: &ContentId,
        time: f64,
    ) -> Result<Vec<ComponentBody>, Self::Error>;
    /// Capture the scene (donor `services.scene.capture`).
    fn capture_scene(&mut self, services: &mut Self::Services) -> Result<Q3SceneContent, Self::Error>;
    /// Clear the scene (donor `services.scene.clearScene`).
    fn clear_scene(&mut self, services: &mut Self::Services);
    /// Preload the frame scene plus HUD scenes (donor
    /// `renderer.preload([scene, ...hudScenes])`).
    fn preload(
        &mut self,
        renderer: &mut Self::Renderer,
        scene: &Q3SceneContent,
        hud_scenes: &[Q3PresentedScene],
    ) -> Result<(), Self::Error>;
    /// Capture the core checkpoint (donor `core.captureCheckpoint`).
    fn capture_core(&self, core: &Self::Core) -> Result<SaveJson, Self::Error>;
    /// Restore the core checkpoint (donor `core.restoreCheckpoint`).
    fn restore_core(
        &mut self,
        core: &mut Self::Core,
        value: &SaveJson,
        resolve: &dyn Fn(&SavedActorId) -> Option<ActorId>,
    ) -> Result<(), Self::Error>;
    /// Capture the files checkpoint (donor `files.captureCheckpoint`).
    fn capture_files(&self, files: &Self::Files) -> Result<SaveJson, Self::Error>;
    /// Restore the files checkpoint (donor `files.restoreCheckpoint`).
    fn restore_files(&mut self, files: &mut Self::Files, value: &SaveJson) -> Result<(), Self::Error>;
    /// Capture the scripts checkpoint (donor `scripts.captureCheckpoint`).
    fn capture_scripts(&self, scripts: &Self::Scripts) -> Result<SaveJson, Self::Error>;
    /// Restore the scripts checkpoint (donor `scripts.restoreCheckpoint`).
    fn restore_scripts(&mut self, scripts: &mut Self::Scripts, value: &SaveJson) -> Result<(), Self::Error>;
    /// Capture the resources checkpoint (donor `services.resources.captureCheckpoint`).
    fn capture_resources(&self, services: &Self::Services) -> Result<SaveJson, Self::Error>;
    /// Restore the resources checkpoint (donor `services.resources.restoreCheckpoint`).
    fn restore_resources(&mut self, services: &mut Self::Services, value: &SaveJson) -> Result<(), Self::Error>;
    /// Capture the sound bank checkpoint (donor `media.bank.captureCheckpoint`).
    fn capture_sounds(&self, media: &Self::Media) -> Result<SaveJson, Self::Error>;
    /// Restore the sound bank checkpoint (donor `media.bank.restoreCheckpoint`).
    fn restore_sounds(&mut self, media: &mut Self::Media, value: &SaveJson) -> Result<(), Self::Error>;
    /// Capture the fonts checkpoint (donor `media.fonts.captureCheckpoint`).
    fn capture_fonts(&self, media: &Self::Media) -> Result<SaveJson, Self::Error>;
    /// Restore the fonts checkpoint (donor `media.fonts.restoreCheckpoint`).
    fn restore_fonts(&mut self, media: &mut Self::Media, value: &SaveJson) -> Result<(), Self::Error>;
    /// Capture the temporary collision checkpoint
    /// (donor `collision.captureTemporaryCheckpoint`).
    fn capture_collision(&self, collision: &Self::Collision) -> Result<SaveJson, Self::Error>;
    /// Restore the temporary collision checkpoint
    /// (donor `collision.restoreTemporaryCheckpoint`).
    fn restore_collision(&mut self, collision: &mut Self::Collision, value: &SaveJson) -> Result<(), Self::Error>;
    /// Capture the cinematics checkpoint
    /// (donor `services.cinematics.captureCheckpoint`).
    fn capture_cinematics(&self, services: &Self::Services) -> Result<SaveJson, Self::Error>;
    /// Restore the cinematics checkpoint
    /// (donor `services.cinematics.restoreCheckpoint`).
    fn restore_cinematics(&mut self, services: &mut Self::Services, value: &SaveJson) -> Result<(), Self::Error>;
    /// Publish restored cinematics (donor `services.cinematics.publishRestored`).
    fn publish_restored_cinematics(&mut self, services: &mut Self::Services) -> Result<(), Self::Error>;
    /// Close the core (donor `core.close`).
    fn close_core(&mut self, core: &mut Self::Core) -> Result<(), Self::Error>;
    /// Close guest files (donor `files.closeAll`).
    fn close_files(&mut self, files: &mut Self::Files) -> Result<(), Self::Error>;
    /// Close client scripts (donor `scripts.closeAll`, including globals).
    fn close_scripts(&mut self, scripts: &mut Self::Scripts) -> Result<(), Self::Error>;
    /// Close cinematics (donor `services.cinematics.close`).
    fn close_cinematics(&mut self, services: &mut Self::Services) -> Result<(), Self::Error>;
    /// Close the renderer (donor `renderer.close`).
    fn close_renderer(&mut self, renderer: &mut Self::Renderer) -> Result<(), Self::Error>;
    /// Clear the sound bank and close media
    /// (donor `media.bank.bank.clear` then `media.close`).
    fn close_media(&mut self, media: &mut Self::Media) -> Result<(), Self::Error>;
}

/// Owner side of guest traps (donor `host(call)` state): the host resolves
/// guest addresses and runs the syscall chain from
/// [`ModPresentationBackend`], and the owner answers with live state, applies
/// the donor output routing, and tracks registrations.
pub trait ModSyscalls<B: ModPresentationBackend> {
    /// Assert the presentation is current (donor `assertCurrent`).
    fn assert_current(&mut self) -> Result<(), ModPresentationError<B::Error>>;
    /// Whether files, scripts, collision, and marks are ready (donor
    /// readiness gate before dispatch).
    fn ready(&self) -> bool;
    /// Presentation context (donor `context()`).
    fn context(&mut self) -> Result<ModPresentationContext, ModPresentationError<B::Error>>;
    /// Source game state for `CG_GETGAMESTATE` (donor `context().gameState`).
    fn game_state(&mut self) -> Result<SourceGameState, ModPresentationError<B::Error>>;
    /// Component cvars.
    fn cvars(&self) -> Rc<RefCell<CvarRegistry>>;
    /// Print a message.
    fn print(&self, text: &str);
    /// Presentation milliseconds (donor `clock.now() + millisecondsOffset`).
    fn milliseconds(&self) -> f64;
    /// Remaining guest memory, capped at `0x7fffffff` (donor
    /// `CG_MEMORY_REMAINING`).
    fn memory_remaining(&self) -> i32;
    /// Whether a command host is configured.
    fn has_commands(&self) -> bool;
    /// Append console text (silent no-op without a command host, donor
    /// rejecting stub whose value the wrapper discards). Host common-service
    /// adapters record failures and fail the trap after the void call.
    fn command_append(&mut self, text: &str) -> Result<(), ModPresentationError<B::Error>>;
    /// Register a console command and track it (silent no-op without a host).
    fn command_register(&mut self, name: &str) -> Result<(), ModPresentationError<B::Error>>;
    /// Remove a console command and untrack it (silent no-op without a host).
    fn command_remove(&mut self, name: &str) -> Result<(), ModPresentationError<B::Error>>;
    /// Send a reliable client command (silent no-op without a host; host
    /// adapters record failures like `command_append`).
    fn command_reliable(&mut self, text: &str) -> Result<(), ModPresentationError<B::Error>>;
    /// Actor owning a source slot (donor `actorAt`; throws when no live actor
    /// owns the slot).
    fn actor_for_slot(&mut self, slot: i32) -> Result<ActorId, ModPresentationError<B::Error>>;
    /// Whether an actor is live.
    fn live(&self, actor: &ActorId) -> bool;
    /// Q3 media.
    fn media_mut(&mut self) -> Result<&mut B::Media, ModPresentationError<B::Error>>;
    /// Q3 services.
    fn services_mut(&mut self) -> Result<&mut B::Services, ModPresentationError<B::Error>>;
    /// Guest files.
    fn files_mut(&mut self) -> Result<&mut B::Files, ModPresentationError<B::Error>>;
    /// Client scripts.
    fn scripts_mut(&mut self) -> Result<&mut B::Scripts, ModPresentationError<B::Error>>;
    /// Collision models.
    fn collision_mut(&mut self) -> Result<&mut B::Collision, ModPresentationError<B::Error>>;
    /// Mark projector.
    fn marks_mut(&mut self) -> Result<&mut B::Marks, ModPresentationError<B::Error>>;
    /// Display renderer, when display traps are served.
    fn display_renderer(&self) -> Option<&QvmDisplayRenderer>;
    /// 2D viewport.
    fn viewport(&self) -> Rect;
    /// Submit a presented scene (donor `output.scene` with HUD routing).
    fn submit_scene(&mut self, scene: Q3PresentedScene) -> Result<(), ModPresentationError<B::Error>>;
    /// Submit a 2D overlay command (donor `output.command` with HUD gating
    /// and color tracking).
    fn submit_overlay(&mut self, command: Q3OverlayCommand) -> Result<(), ModPresentationError<B::Error>>;
    /// Submit a text draw (donor `output.text` with HUD routing).
    fn submit_text(&mut self, draw: Q3ServiceTextDraw) -> Result<(), ModPresentationError<B::Error>>;
    /// Move the listener (donor `output.listener`, always routed).
    fn submit_listener(&mut self, origin: Vec3, axis: Axis) -> Result<(), ModPresentationError<B::Error>>;
    /// Queue a seat audio operation (donor `output.audio`).
    fn emit_audio(&mut self, operation: Q3SeatAudioOperation) -> Result<(), ModPresentationError<B::Error>>;
    /// Print when the `developer` cvar is nonzero (donor `developerPrint`).
    fn developer_print(&mut self, text: &str);
    /// Assert a collision-map request names the presentation map (donor
    /// `loadMap` gate).
    fn check_collision_map(&mut self, path: &str) -> Result<(), ModPresentationError<B::Error>>;
    /// Deliver a media request (donor music/remap arms); `Ok(true)` means
    /// delivered (the host answers `0`), `Ok(false)` means no sink (the host
    /// rejects the trap).
    fn deliver_media(
        &mut self,
        request: &ComponentPresentationMediaRequest,
    ) -> Result<bool, ModPresentationError<B::Error>>;
    /// Pump the next frame (donor `CG_UPDATESCREEN`).
    fn next_frame(&mut self) -> Result<(), ModPresentationError<B::Error>>;
    /// Scalar fallback (donor `options.scalar`); `None` continues the chain.
    fn scalar(&mut self, call: &B::HostCall) -> Option<i32>;
}

/// Scene renderer operations (donor `renderer.operations`): the host renderer
/// implements this so seat effect frames can build scene operations without
/// the backend handle.
pub trait ModSceneRendererOps<E> {
    /// Build scene operations for a captured scene.
    fn scene_operations(
        &mut self,
        scene: &Q3SceneContent,
        input: &WorldViewInput,
        first_entity: u32,
        options: &Q3SceneRenderOptions,
    ) -> Result<Vec<SceneOperation>, E>;
}

/// Owned HUD submission (donor `Q3OverlaySubmission`).
#[derive(Debug, Clone)]
pub enum ModPresentationHudSubmission {
    /// HUD scene without a world model.
    Scene(Box<Q3PresentedScene>),
    /// 2D command.
    Command(Q3OverlayCommand),
    /// HUD text draw.
    Text(Q3ServiceTextDraw),
}

/// One original component cgame instance for one live source generation and
/// viewing seat (donor `ApplicationModPresentation`).
pub struct ApplicationModPresentation<B: ModPresentationBackend> {
    /// Presentation options.
    pub options: ApplicationModPresentationOptions<B>,
    /// Component cvars.
    pub cvars: Rc<RefCell<CvarRegistry>>,
    /// Bound source generation.
    generation: u64,
    /// Queued seat audio operations.
    audio_operations: Vec<Q3SeatAudioOperation>,
    /// Q3 media.
    media: Option<B::Media>,
    /// Q3 services.
    services: Option<B::Services>,
    /// Presentation core.
    core: Option<B::Core>,
    /// Scene renderer.
    renderer: Option<B::Renderer>,
    /// Captured scene.
    captured: Option<Q3SceneContent>,
    /// Pending HUD submissions.
    pending_hud: Vec<ModPresentationHudSubmission>,
    /// Current HUD color.
    hud_color: Vec4,
    /// Captured HUD submissions.
    captured_hud: Vec<ModPresentationHudSubmission>,
    /// Captured bodies.
    captured_bodies: Vec<ComponentBody>,
    /// Published flag.
    published: bool,
    /// In-flight core operations.
    operations: u32,
    /// Commands-published flag.
    commands_published: bool,
    /// Last frame sequence.
    frame_sequence: i64,
    /// Frame ordinal offset.
    frame_offset: i64,
    /// Clock milliseconds offset.
    milliseconds_offset: f64,
    /// Registered command names.
    registered_commands: HashSet<String>,
    /// Previous frame time in milliseconds.
    previous_frame_time: Option<f64>,
    /// Closed flag.
    closed: bool,
    /// Initializing flag.
    initializing: bool,
    /// Scene context for scene runtimes.
    scene_context: Option<ComponentSceneContext<SourceGameState>>,
    /// Scene baseline for scene runtimes.
    scene_baseline: Option<ComponentSceneContext<SourceGameState>>,
    /// Scene time offset in milliseconds.
    scene_time_offset: Option<f64>,
    /// File mounts captured at creation.
    mounts: Option<Rc<MountedContent>>,
    /// Guest files.
    files: Option<B::Files>,
    /// Client scripts.
    scripts: Option<B::Scripts>,
    /// Collision models.
    collision: Option<B::Collision>,
    /// Mark projector.
    marks: Option<B::Marks>,
}

impl<B: ModPresentationBackend> ApplicationModPresentation<B> {
    /// Create a presentation (donor `create`).
    pub fn create(
        options: ApplicationModPresentationOptions<B>,
        backend: &mut B,
    ) -> Result<Self, ModPresentationError<B::Error>> {
        Self::create_owner(options, backend, -1, None)
    }

    /// Create a presentation over a baseline sequence (donor `create` with
    /// `baselineSequence`).
    pub fn create_with_baseline(
        options: ApplicationModPresentationOptions<B>,
        backend: &mut B,
        baseline_sequence: i64,
    ) -> Result<Self, ModPresentationError<B::Error>> {
        Self::create_owner(options, backend, baseline_sequence, None)
    }

    /// Restore a presentation from a checkpoint (donor `restore`).
    pub fn restore(
        options: ApplicationModPresentationOptions<B>,
        backend: &mut B,
        checkpoint: &SaveJson,
        resolve_actor: &dyn Fn(&SavedActorId) -> Option<ActorId>,
    ) -> Result<Self, ModPresentationError<B::Error>> {
        Self::create_owner(options, backend, -1, Some((checkpoint, resolve_actor)))
    }

    /// Create or restore an owner (donor `createOwner`).
    fn create_owner(
        options: ApplicationModPresentationOptions<B>,
        backend: &mut B,
        baseline_sequence: i64,
        checkpoint: CheckpointRestore<'_, '_>,
    ) -> Result<Self, ModPresentationError<B::Error>> {
        let source = Rc::clone(&options.source);
        if !source.live_module().same_module(&source.prepared_module())
            || source.source_abi() != declaration_gameplay_abi(&source.declaration())
        {
            return Err(ModPresentationError::Presentation(
                "Component presentation source differs from its prepared gameplay module".to_string(),
            ));
        }
        let mut owner = Self {
            options,
            cvars: Rc::new(RefCell::new(CvarRegistry::new(Dialect::Q3))),
            generation: source.generation(),
            audio_operations: Vec::new(),
            media: None,
            services: None,
            core: None,
            renderer: None,
            captured: None,
            pending_hud: Vec::new(),
            hud_color: Vec4 {
                x: 1.0,
                y: 1.0,
                z: 1.0,
                w: 1.0,
            },
            captured_hud: Vec::new(),
            captured_bodies: Vec::new(),
            published: checkpoint.is_none(),
            operations: 0,
            commands_published: true,
            frame_sequence: -1,
            frame_offset: 0,
            milliseconds_offset: 0.0,
            registered_commands: HashSet::new(),
            previous_frame_time: None,
            closed: false,
            initializing: true,
            scene_context: None,
            scene_baseline: None,
            scene_time_offset: None,
            mounts: None,
            files: None,
            scripts: None,
            collision: None,
            marks: None,
        };
        owner.assert_current()?;
        if let Err(error) = owner.create_owned(backend, baseline_sequence, checkpoint) {
            if let Err(cleanup) = owner.close(backend) {
                return Err(ModPresentationError::Presentation(format!(
                    "Component presentation initialization and cleanup failed: {error}; {cleanup}"
                )));
            }
            return Err(error);
        }
        Ok(owner)
    }

    /// Run creation or restoration (donor `createOwner` body).
    fn create_owned(
        &mut self,
        backend: &mut B,
        baseline_sequence: i64,
        checkpoint: CheckpointRestore<'_, '_>,
    ) -> Result<(), ModPresentationError<B::Error>> {
        let declaration = self.options.source.declaration();
        if declaration_is_scene(&declaration) {
            for (name, value) in declaration_cvars(&declaration) {
                self.cvars.borrow_mut().set(name, value, false)?;
            }
        }
        let content = self.options.source.identity_content();
        let print = Rc::clone(&self.options.print);
        let media = backend
            .create_media(&content, print)
            .map_err(ModPresentationError::Backend)?;
        self.media = Some(media);
        self.assert_current()?;
        let settings = CollisionMapSettings::new(Rc::clone(&self.cvars));
        settings.register_map()?;
        let native = self.options.queries.has_native_collision();
        let map = self.options.assets.map_geometry_path().to_string();
        let collision = backend
            .create_collision(native, &map, Rc::clone(&self.cvars))
            .map_err(ModPresentationError::Backend)?;
        self.collision = Some(collision);
        let marks = backend.create_marks().map_err(ModPresentationError::Backend)?;
        self.marks = Some(marks);
        let mounts = match self.options.source.source_mounts() {
            Some(mounts) => mounts,
            None => {
                let media = self.media_ref()?;
                backend.media_mounts(media)
            }
        };
        self.mounts = Some(Rc::clone(&mounts));
        let system_cinematics = match self.options.system_cinematics.take() {
            None => None,
            Some(ModPresentationCinematics::Host(host)) => Some(host),
            Some(ModPresentationCinematics::Factory(factory)) => {
                let cvars = self.cvars.borrow();
                Some(factory(&cvars, &mounts).map_err(ModPresentationError::Presentation)?)
            }
        };
        let services = backend
            .create_services(ModServicesParams {
                seat: self.options.seat.clone(),
                viewport: self.options.viewport,
                owner: self.options.source.prepared_source_id(),
                resource_handles: ResourceHandleOwner::Client,
                system_cinematics,
                collision_settings: settings,
                print: Rc::clone(&self.options.print),
            })
            .map_err(ModPresentationError::Backend)?;
        self.services = Some(services);
        self.assert_current()?;
        let renderer = backend
            .create_renderer(self.media_ref()?, self.services_ref()?)
            .map_err(ModPresentationError::Backend)?;
        self.renderer = Some(renderer);
        let writable = self.options.source.source_files_writable();
        let files = backend
            .create_files(Rc::clone(&mounts), writable, Rc::clone(&self.options.print))
            .map_err(ModPresentationError::Backend)?;
        self.files = Some(files);
        let scripts = backend
            .create_scripts(mounts, writable, Rc::clone(&self.options.print))
            .map_err(ModPresentationError::Backend)?;
        self.scripts = Some(scripts);
        let core = backend
            .create_core(ModCoreParams {
                artifact: self.options.source.core_artifact(),
                source: self.options.source.prepared_module(),
                declaration: self.options.source.declaration(),
            })
            .map_err(ModPresentationError::Backend)?;
        self.core = Some(core);
        match checkpoint {
            None => {
                let mut core = self.core.take().ok_or_else(|| {
                    ModPresentationError::Presentation("Component presentation has not initialized".to_string())
                })?;
                let initialized = backend
                    .initialize_core(&mut core, baseline_sequence, self)
                    .map_err(ModPresentationError::Backend);
                self.core = Some(core);
                initialized?;
            }
            Some((value, resolve)) => self.restore_owned(backend, value, resolve)?,
        }
        self.assert_current()?;
        self.initializing = false;
        let services = self.services_value_mut()?;
        backend.clear_scene(services);
        self.pending_hud.clear();
        self.flush_audio()?;
        Ok(())
    }

    /// Borrow media without asserting (creation paths assert separately).
    fn media_ref(&self) -> Result<&B::Media, ModPresentationError<B::Error>> {
        self.media.as_ref().ok_or_else(|| {
            ModPresentationError::Presentation("Component presentation media are not initialized".to_string())
        })
    }

    /// Borrow services without asserting (creation paths assert separately).
    fn services_ref(&self) -> Result<&B::Services, ModPresentationError<B::Error>> {
        self.services.as_ref().ok_or_else(|| {
            ModPresentationError::Presentation("Component presentation services are not initialized".to_string())
        })
    }

    /// Mutably borrow services without asserting.
    fn services_value_mut(&mut self) -> Result<&mut B::Services, ModPresentationError<B::Error>> {
        self.services.as_mut().ok_or_else(|| {
            ModPresentationError::Presentation("Component presentation services are not initialized".to_string())
        })
    }

    /// File mounts for reads (donor `fileMounts`, captured at creation).
    pub fn file_mounts(&self) -> Result<Rc<MountedContent>, ModPresentationError<B::Error>> {
        self.assert_current()?;
        self.mounts.as_ref().map(Rc::clone).ok_or_else(|| {
            ModPresentationError::Presentation("Component presentation media are not initialized".to_string())
        })
    }

    /// Q3 media (donor `media`).
    pub fn media(&self) -> Result<&B::Media, ModPresentationError<B::Error>> {
        self.assert_current()?;
        self.media_ref()
    }

    /// Q3 services (donor `services`).
    pub fn services(&self) -> Result<&B::Services, ModPresentationError<B::Error>> {
        self.assert_current()?;
        self.services_ref()
    }

    /// Scene renderer (donor `renderer`).
    pub fn renderer(&self) -> Result<&B::Renderer, ModPresentationError<B::Error>> {
        self.assert_current()?;
        self.renderer.as_ref().ok_or_else(|| {
            ModPresentationError::Presentation("Component scene renderer is not initialized".to_string())
        })
    }

    /// Build scene operations for a captured scene (donor
    /// `renderer.operations`).
    pub fn scene_operations(
        &mut self,
        scene: &Q3SceneContent,
        input: &WorldViewInput,
        first_entity: u32,
        options: &Q3SceneRenderOptions,
    ) -> Result<Vec<SceneOperation>, ModPresentationError<B::Error>>
    where
        B::Renderer: ModSceneRendererOps<B::Error>,
    {
        self.assert_current()?;
        let renderer = self.renderer.as_mut().ok_or_else(|| {
            ModPresentationError::Presentation("Component scene renderer is not initialized".to_string())
        })?;
        renderer
            .scene_operations(scene, input, first_entity, options)
            .map_err(ModPresentationError::Backend)
    }

    /// Presentation time in milliseconds (donor `time`).
    pub fn time(&mut self) -> Result<f64, ModPresentationError<B::Error>> {
        if let Some(previous) = self.previous_frame_time {
            return Ok(previous);
        }
        Ok(self.build_context()?.scene.snapshot.server_time as f64)
    }

    /// Captured bodies (donor `bodies`).
    pub fn bodies(&self) -> Result<&[ComponentBody], ModPresentationError<B::Error>> {
        self.assert_current()?;
        Ok(&self.captured_bodies)
    }

    /// Captured HUD submissions (donor `hud`).
    pub fn hud(&self) -> Result<&[ModPresentationHudSubmission], ModPresentationError<B::Error>> {
        self.assert_current()?;
        Ok(&self.captured_hud)
    }

    /// Whether this presentation owns a source and viewer (donor `owns`): the
    /// single-handle comparison covers the donor source, prepared module, and
    /// owner identity together.
    pub fn owns(&self, candidate: &Rc<dyn ModPresentationSource>, viewer: &ActorId) -> bool {
        !self.closed
            && Rc::ptr_eq(candidate, &self.options.source)
            && candidate.generation() == self.generation
            && viewer == &self.options.viewer
            && candidate.live(viewer)
    }

    /// Assert the presentation is current (donor `assertCurrent`).
    fn assert_current(&self) -> Result<(), ModPresentationError<B::Error>> {
        let source = &self.options.source;
        if self.closed || source.generation() != self.generation || !source.live(&self.options.viewer) {
            return Err(ModPresentationError::Presentation(
                "Component presentation belongs to a retired source or viewer".to_string(),
            ));
        }
        source.assert_current().map_err(ModPresentationError::Presentation)?;
        Ok(())
    }

    /// Build the presentation context (donor `context()`).
    fn build_context(&mut self) -> Result<ModPresentationContext, ModPresentationError<B::Error>> {
        self.assert_current()?;
        let source = Rc::clone(&self.options.source);
        let context = source.scene_context(&self.options.viewer).ok_or_else(|| {
            ModPresentationError::Presentation(
                "Component presentation viewer has no original source context".to_string(),
            )
        })?;
        let frame_time_ms = match self.previous_frame_time {
            None => 0.0,
            Some(previous) => context.snapshot.server_time as f64 - previous,
        };
        let declaration = source.declaration();
        let scene_runtime = declaration_is_scene(&declaration);
        if frame_time_ms < 0.0 && !scene_runtime {
            return Err(ModPresentationError::Presentation(
                "Component presentation source time moved backward".to_string(),
            ));
        }
        let view_origin = (self.options.view_origin)();
        let view_axis = self.options.view_axis.as_ref().map(|axis| axis());
        if !scene_runtime {
            return Ok(ModPresentationContext {
                scene: context,
                baseline: None,
                frame_time_ms,
                time_ms: None,
                view_origin,
                view_axis,
            });
        }
        // Scene runtimes always select from the admitted publication: a
        // pre-selected source scene has no Rust counterpart.
        let publication = source.scene_publication().ok_or_else(|| {
            ModPresentationError::Presentation(
                "Scene cgame requires an admitted gameplay entity snapshot source".to_string(),
            )
        })?;
        let stale = self
            .scene_context
            .as_ref()
            .is_none_or(|scene| scene.revision != publication.current.revision);
        if stale {
            if self.scene_baseline.is_none() {
                if let Some(baseline) = &publication.baseline {
                    let print = Rc::clone(&self.options.print);
                    let mut emit = |text: &str| print(text);
                    self.scene_baseline = Some(select_component_scene(
                        baseline,
                        &self.options.viewer,
                        &*self.options.queries,
                        self.options.assets.world_leaf_count(),
                        &mut emit,
                    )?);
                }
            }
            let print = Rc::clone(&self.options.print);
            let mut emit = |text: &str| print(text);
            self.scene_context = Some(select_component_scene(
                &publication.current,
                &self.options.viewer,
                &*self.options.queries,
                self.options.assets.world_leaf_count(),
                &mut emit,
            )?);
        }
        let scene = self
            .scene_context
            .as_ref()
            .ok_or_else(|| ModPresentationError::Presentation("Component scene context is unavailable".to_string()))?
            .clone();
        let server_time = scene.snapshot.server_time as f64;
        let offset = match self.scene_time_offset {
            Some(offset) => offset,
            None => {
                let offset = server_time - self.options.clock.now();
                self.scene_time_offset = Some(offset);
                offset
            }
        };
        let floor = self.previous_frame_time.unwrap_or(server_time);
        let time_ms = server_time.max(floor).max(self.options.clock.now() + offset).trunc();
        let frame_time_ms = match self.previous_frame_time {
            None => 0.0,
            Some(previous) => time_ms - previous,
        };
        Ok(ModPresentationContext {
            scene,
            baseline: self.scene_baseline.clone(),
            frame_time_ms,
            time_ms: Some(time_ms),
            view_origin,
            view_axis,
        })
    }

    /// Flush queued audio into the sink (donor `flushAudio`).
    fn flush_audio(&mut self) -> Result<(), ModPresentationError<B::Error>> {
        if self.audio_operations.is_empty() {
            return Ok(());
        }
        self.assert_current()?;
        let frame = ModCgameAudioFrame {
            content: self.options.source.identity_content(),
            seat: self.options.seat.clone(),
            owner: self.options.source.prepared_source_id(),
            operations: std::mem::take(&mut self.audio_operations),
        };
        self.options.audio.receive_cgame_frame(frame);
        Ok(())
    }

    /// Capture the checkpoint (donor `captureCheckpoint`).
    pub fn capture_checkpoint(&self, backend: &B) -> Result<SaveJson, ModPresentationError<B::Error>> {
        self.assert_current()?;
        if self.initializing
            || self.operations != 0
            || self.core.is_none()
            || self.files.is_none()
            || self.scripts.is_none()
            || self.collision.is_none()
            || !self.audio_operations.is_empty()
            || !self.pending_hud.is_empty()
        {
            return Err(ModPresentationError::Presentation(
                "Component checkpoint requires an idle completed frame".to_string(),
            ));
        }
        let idle =
            || ModPresentationError::Presentation("Component checkpoint requires an idle completed frame".to_string());
        let core = self.core.as_ref().ok_or_else(idle)?;
        let files = self.files.as_ref().ok_or_else(idle)?;
        let scripts = self.scripts.as_ref().ok_or_else(idle)?;
        let services = self.services.as_ref().ok_or_else(idle)?;
        let media = self.media.as_ref().ok_or_else(idle)?;
        let collision = self.collision.as_ref().ok_or_else(idle)?;
        let scene_context = match &self.scene_context {
            None => SaveJson::Null,
            Some(scene) => write_scene_context(scene, self.scene_baseline.as_ref())?,
        };
        let scene_baseline = match &self.scene_baseline {
            None => SaveJson::Null,
            Some(baseline) => write_scene_context(baseline, None)?,
        };
        Ok(obj(vec![
            ("version", int(1)),
            ("declaration", self.options.source.declaration_checkpoint()),
            (
                "core",
                backend.capture_core(core).map_err(ModPresentationError::Backend)?,
            ),
            ("cvars", write_cvars(&self.cvars.borrow().capture_save_state()?)),
            (
                "files",
                backend.capture_files(files).map_err(ModPresentationError::Backend)?,
            ),
            (
                "scripts",
                backend
                    .capture_scripts(scripts)
                    .map_err(ModPresentationError::Backend)?,
            ),
            (
                "resources",
                backend
                    .capture_resources(services)
                    .map_err(ModPresentationError::Backend)?,
            ),
            (
                "sounds",
                backend.capture_sounds(media).map_err(ModPresentationError::Backend)?,
            ),
            (
                "fonts",
                backend.capture_fonts(media).map_err(ModPresentationError::Backend)?,
            ),
            (
                "collision",
                backend
                    .capture_collision(collision)
                    .map_err(ModPresentationError::Backend)?,
            ),
            (
                "cinematics",
                backend
                    .capture_cinematics(services)
                    .map_err(ModPresentationError::Backend)?,
            ),
            (
                "commands",
                arr(self.registered_commands.iter().map(|name| save_str(name)).collect()),
            ),
            ("frameSequence", int(self.frame_sequence)),
            (
                "frameOrdinal",
                int(self.options.clock.frame_number() + self.frame_offset),
            ),
            (
                "previousFrameTime",
                self.previous_frame_time.map_or(SaveJson::Null, num),
            ),
            ("milliseconds", num(self.options.clock.now() + self.milliseconds_offset)),
            (
                "sceneTime",
                self.scene_time_offset
                    .map_or(SaveJson::Null, |offset| num(self.options.clock.now() + offset)),
            ),
            (
                "hudColor",
                obj(vec![
                    ("x", num(self.hud_color.x as f64)),
                    ("y", num(self.hud_color.y as f64)),
                    ("z", num(self.hud_color.z as f64)),
                    ("w", num(self.hud_color.w as f64)),
                ]),
            ),
            ("sceneContext", scene_context),
            ("sceneBaseline", scene_baseline),
        ]))
    }

    /// Restore owned state from a checkpoint (donor `restoreOwned`).
    fn restore_owned(
        &mut self,
        backend: &mut B,
        value: &SaveJson,
        resolve_actor: &dyn Fn(&SavedActorId) -> Option<ActorId>,
    ) -> Result<(), ModPresentationError<B::Error>> {
        if self.core.is_none() || self.files.is_none() || self.scripts.is_none() || self.collision.is_none() {
            return Err(ModPresentationError::Presentation(
                "Component host is not allocated".to_string(),
            ));
        }
        self.commands_published = false;
        let reader = SaveReader::at(value, "component-presentation");
        reader.field("version").literal_i64(1)?;
        let stored = checkpoint_field(&reader, "declaration")?;
        if encode_checkpoint_value(stored) != encode_checkpoint_value(&self.options.source.declaration_checkpoint()) {
            return Err(reader.field("declaration").fail("component declaration changed").into());
        }
        self.frame_sequence = reader.field("frameSequence").integer(-1)?;
        self.frame_offset = reader.field("frameOrdinal").integer(0)? - self.options.clock.frame_number();
        self.previous_frame_time = reader
            .field("previousFrameTime")
            .nullable(|field| field.integer(0))?
            .map(|time| time as f64);
        self.milliseconds_offset = reader.field("milliseconds").finite()? - self.options.clock.now();
        self.scene_time_offset =
            reader
                .field("sceneTime")
                .nullable(|field| -> Result<f64, ModPresentationError<B::Error>> {
                    Ok(field.finite()? - self.options.clock.now())
                })?;
        let color = reader.field("hudColor");
        self.hud_color = Vec4 {
            x: color.field("x").number()? as f32,
            y: color.field("y").number()? as f32,
            z: color.field("z").number()? as f32,
            w: color.field("w").number()? as f32,
        };
        let stored_context = reader
            .field("sceneContext")
            .nullable(|field| read_scene_context(field, resolve_actor, 0))?;
        let nested_baseline = stored_context.as_ref().and_then(|(_, baseline)| baseline.clone());
        self.scene_context = stored_context.map(|(scene, _)| scene);
        let stored_baseline = reader
            .field("sceneBaseline")
            .nullable(|field| read_scene_context(field, resolve_actor, 0))?;
        // The top-level baseline wins; the nested copy rides along for layout
        // compatibility with the donor capture.
        self.scene_baseline = stored_baseline.map(|(scene, _)| scene).or(nested_baseline);
        let cvars = read_cvars(reader.field("cvars"))?;
        self.cvars.borrow_mut().restore_save_state(&cvars)?;
        let fonts = checkpoint_field(&reader, "fonts")?;
        let media = self.media.as_mut().ok_or_else(|| {
            ModPresentationError::Presentation("Component presentation media are not initialized".to_string())
        })?;
        backend
            .restore_fonts(media, fonts)
            .map_err(ModPresentationError::Backend)?;
        self.assert_current()?;
        let resources = checkpoint_field(&reader, "resources")?;
        let services = self.services_value_mut()?;
        backend
            .restore_resources(services, resources)
            .map_err(ModPresentationError::Backend)?;
        self.assert_current()?;
        let sounds = checkpoint_field(&reader, "sounds")?;
        let media = self.media.as_mut().ok_or_else(|| {
            ModPresentationError::Presentation("Component presentation media are not initialized".to_string())
        })?;
        backend
            .restore_sounds(media, sounds)
            .map_err(ModPresentationError::Backend)?;
        self.assert_current()?;
        let files_value = checkpoint_field(&reader, "files")?;
        let files = self
            .files
            .as_mut()
            .ok_or_else(|| ModPresentationError::Presentation("Component host is not allocated".to_string()))?;
        backend
            .restore_files(files, files_value)
            .map_err(ModPresentationError::Backend)?;
        let scripts_value = checkpoint_field(&reader, "scripts")?;
        let scripts = self
            .scripts
            .as_mut()
            .ok_or_else(|| ModPresentationError::Presentation("Component host is not allocated".to_string()))?;
        backend
            .restore_scripts(scripts, scripts_value)
            .map_err(ModPresentationError::Backend)?;
        let collision_value = checkpoint_field(&reader, "collision")?;
        let collision = self
            .collision
            .as_mut()
            .ok_or_else(|| ModPresentationError::Presentation("Component host is not allocated".to_string()))?;
        backend
            .restore_collision(collision, collision_value)
            .map_err(ModPresentationError::Backend)?;
        let cinematics = checkpoint_field(&reader, "cinematics")?;
        let services = self.services_value_mut()?;
        backend
            .restore_cinematics(services, cinematics)
            .map_err(ModPresentationError::Backend)?;
        self.assert_current()?;
        let core_value = checkpoint_field(&reader, "core")?;
        let mut core = self.core.take().ok_or_else(|| {
            ModPresentationError::Presentation("Component presentation has not initialized".to_string())
        })?;
        let restored = backend
            .restore_core(&mut core, core_value, resolve_actor)
            .map_err(ModPresentationError::Backend);
        self.core = Some(core);
        restored?;
        let commands = reader.field("commands").list(|field| field.string())?;
        let unique: HashSet<&String> = commands.iter().collect();
        if commands.len() != unique.len() {
            return Err(reader
                .field("commands")
                .fail("duplicate component command registration")
                .into());
        }
        for name in commands {
            self.registered_commands.insert(name);
        }
        Ok(())
    }

    /// Bind saved command registrations when the prepared consumer is
    /// published (donor `publishCommands`).
    pub fn publish_commands(&mut self, backend: &mut B) -> Result<(), ModPresentationError<B::Error>> {
        self.assert_current()?;
        if self.initializing {
            return Err(ModPresentationError::Presentation(
                "Cannot publish an initializing component".to_string(),
            ));
        }
        if self.commands_published {
            return Ok(());
        }
        if self.options.commands.is_none() && !self.registered_commands.is_empty() {
            return Err(ModPresentationError::Presentation(
                "Restored component commands have no destination registry".to_string(),
            ));
        }
        if let Some(commands) = &self.options.commands {
            for name in &self.registered_commands {
                commands
                    .register_command(name)
                    .map_err(ModPresentationError::Presentation)?;
            }
        }
        self.commands_published = true;
        self.published = true;
        let services = self.services_value_mut()?;
        backend
            .publish_restored_cinematics(services)
            .map_err(ModPresentationError::Backend)?;
        Ok(())
    }

    /// Run a console command (donor `command`).
    pub fn command(&mut self, backend: &mut B, arguments: &[String]) -> Result<bool, ModPresentationError<B::Error>> {
        if !self.published {
            return Err(ModPresentationError::Presentation(
                "Component presentation has not been published".to_string(),
            ));
        }
        self.assert_current()?;
        if self.core.is_none() {
            return Err(ModPresentationError::Presentation(
                "Component presentation has not initialized".to_string(),
            ));
        }
        self.operations += 1;
        let mut core = self.core.take().ok_or_else(|| {
            ModPresentationError::Presentation("Component presentation has not initialized".to_string())
        })?;
        let result = backend
            .console_command(&mut core, arguments, self)
            .map_err(ModPresentationError::Backend);
        self.core = Some(core);
        self.operations -= 1;
        match result {
            Ok(value) => {
                self.assert_current()?;
                self.flush_audio()?;
                Ok(value)
            }
            Err(error) => {
                if let Err(cleanup) = self.close(backend) {
                    return Err(ModPresentationError::Presentation(format!(
                        "Component command and cleanup failed: {error}; {cleanup}"
                    )));
                }
                Err(error)
            }
        }
    }

    /// Consume a player event (donor `consume`).
    pub fn consume(
        &mut self,
        backend: &mut B,
        event: &Q3SourcePlayerEvent,
        sequence: i64,
    ) -> Result<(), ModPresentationError<B::Error>> {
        if !self.published {
            return Err(ModPresentationError::Presentation(
                "Component presentation has not been published".to_string(),
            ));
        }
        self.assert_current()?;
        if self.core.is_none() {
            return Err(ModPresentationError::Presentation(
                "Component presentation has not initialized".to_string(),
            ));
        }
        self.operations += 1;
        let mut core = self.core.take().ok_or_else(|| {
            ModPresentationError::Presentation("Component presentation has not initialized".to_string())
        })?;
        let result = backend
            .consume_event(&mut core, event, sequence, self)
            .map_err(ModPresentationError::Backend);
        self.core = Some(core);
        self.operations -= 1;
        match result {
            Ok(()) => {
                self.assert_current()?;
                if self.options.source.live(&event.actor) {
                    self.flush_audio()?;
                } else {
                    self.audio_operations.clear();
                }
                Ok(())
            }
            Err(error) => {
                self.audio_operations.clear();
                if let Err(cleanup) = self.close(backend) {
                    return Err(ModPresentationError::Presentation(format!(
                        "Component presentation execution and cleanup failed: {error}; {cleanup}"
                    )));
                }
                Err(error)
            }
        }
    }

    /// Present one frame (donor `frame`; the `i64` sequence is always a safe
    /// integer, so only negativity rejects).
    pub fn frame(&mut self, backend: &mut B, sequence: i64) -> Result<Q3SceneContent, ModPresentationError<B::Error>> {
        if !self.published {
            return Err(ModPresentationError::Presentation(
                "Component presentation has not been published".to_string(),
            ));
        }
        self.assert_current()?;
        if sequence < 0 {
            return Err(ModPresentationError::Presentation(
                "Invalid component presentation frame sequence".to_string(),
            ));
        }
        let sequence = sequence + self.frame_offset;
        if sequence <= self.frame_sequence {
            if let Some(captured) = &self.captured {
                return Ok(captured.clone());
            }
        }
        if self.core.is_none() {
            return Err(ModPresentationError::Presentation(
                "Component presentation has not initialized".to_string(),
            ));
        }
        self.operations += 1;
        let result = self.frame_inner(backend, sequence);
        self.operations -= 1;
        match result {
            Ok(scene) => Ok(scene),
            Err(error) => {
                if let Err(cleanup) = self.close(backend) {
                    return Err(ModPresentationError::Presentation(format!(
                        "Component presentation frame and cleanup failed: {error}; {cleanup}"
                    )));
                }
                Err(error)
            }
        }
    }

    /// Present one uncached frame (donor `frame` body).
    fn frame_inner(
        &mut self,
        backend: &mut B,
        sequence: i64,
    ) -> Result<Q3SceneContent, ModPresentationError<B::Error>> {
        let context = self.build_context()?;
        let time = context.time_ms.unwrap_or(context.scene.snapshot.server_time as f64);
        let mut core = self.core.take().ok_or_else(|| {
            ModPresentationError::Presentation("Component presentation has not initialized".to_string())
        })?;
        let advanced = backend
            .advance_core(&mut core, sequence, self)
            .map_err(ModPresentationError::Backend);
        self.core = Some(core);
        advanced?;
        self.assert_current()?;
        let owner = self.options.source.presentation_owner();
        let content = self.options.source.identity_content();
        let bodies = {
            let core = self.core.as_ref().ok_or_else(|| {
                ModPresentationError::Presentation("Component presentation has not initialized".to_string())
            })?;
            let services = self.services_ref()?;
            backend
                .capture_bodies(core, services, &owner, &content, time)
                .map_err(ModPresentationError::Backend)?
        };
        let scene = {
            let services = self.services_value_mut()?;
            backend.capture_scene(services).map_err(ModPresentationError::Backend)?
        };
        backend.clear_scene(self.services_value_mut()?);
        if declaration_hud(&self.options.source.declaration()) {
            let mut core = self.core.take().ok_or_else(|| {
                ModPresentationError::Presentation("Component presentation has not initialized".to_string())
            })?;
            let drawn = backend
                .draw_hud(&mut core, sequence, self)
                .map_err(ModPresentationError::Backend);
            self.core = Some(core);
            drawn?;
        }
        self.assert_current()?;
        let hud = std::mem::take(&mut self.pending_hud);
        backend.clear_scene(self.services_value_mut()?);
        let mut hud_scenes = Vec::new();
        for submission in &hud {
            if let ModPresentationHudSubmission::Scene(scene) = submission {
                hud_scenes.push((**scene).clone());
            }
        }
        {
            let renderer = self.renderer.as_mut().ok_or_else(|| {
                ModPresentationError::Presentation("Component scene renderer is not initialized".to_string())
            })?;
            backend
                .preload(renderer, &scene, &hud_scenes)
                .map_err(ModPresentationError::Backend)?;
        }
        self.assert_current()?;
        self.flush_audio()?;
        self.previous_frame_time = Some(time);
        self.frame_sequence = sequence;
        self.captured = Some(scene.clone());
        self.captured_hud = hud;
        self.captured_bodies = bodies;
        Ok(scene)
    }

    /// Close the presentation, collecting cleanup failures (donor `close`).
    pub fn close(&mut self, backend: &mut B) -> Result<(), ModPresentationError<B::Error>> {
        if self.closed {
            return Ok(());
        }
        self.closed = true;
        self.audio_operations.clear();
        self.pending_hud.clear();
        self.captured_hud.clear();
        self.captured_bodies.clear();
        let mut failures: Vec<String> = Vec::new();
        if let Some(core) = self.core.as_mut() {
            if let Err(error) = backend.close_core(core) {
                failures.push(error.to_string());
            }
        }
        if let Some(files) = self.files.as_mut() {
            if let Err(error) = backend.close_files(files) {
                failures.push(error.to_string());
            }
        }
        if let Some(scripts) = self.scripts.as_mut() {
            if let Err(error) = backend.close_scripts(scripts) {
                failures.push(error.to_string());
            }
        }
        if let Some(services) = self.services.as_mut() {
            if let Err(error) = backend.close_cinematics(services) {
                failures.push(error.to_string());
            }
        }
        if let Some(renderer) = self.renderer.as_mut() {
            if let Err(error) = backend.close_renderer(renderer) {
                failures.push(error.to_string());
            }
        }
        if let Some(media) = self.media.as_mut() {
            if let Err(error) = backend.close_media(media) {
                failures.push(error.to_string());
            }
        }
        if self.published {
            self.options.audio.receive_cgame_frame(ModCgameAudioFrame {
                content: self.options.source.identity_content(),
                seat: self.options.seat.clone(),
                owner: self.options.source.prepared_source_id(),
                operations: vec![Q3SeatAudioOperation::ReleaseOwner],
            });
        }
        self.core = None;
        self.files = None;
        self.scripts = None;
        self.services = None;
        self.media = None;
        self.collision = None;
        self.marks = None;
        self.renderer = None;
        self.mounts = None;
        self.captured = None;
        if failures.is_empty() {
            Ok(())
        } else {
            Err(ModPresentationError::Presentation(format!(
                "Component presentation cleanup failed: {}",
                failures.join("; ")
            )))
        }
    }
}

impl<B: ModPresentationBackend> ModSyscalls<B> for ApplicationModPresentation<B> {
    fn assert_current(&mut self) -> Result<(), ModPresentationError<B::Error>> {
        Self::assert_current(self)
    }

    fn ready(&self) -> bool {
        self.files.is_some() && self.scripts.is_some() && self.collision.is_some() && self.marks.is_some()
    }

    fn context(&mut self) -> Result<ModPresentationContext, ModPresentationError<B::Error>> {
        self.build_context()
    }

    fn game_state(&mut self) -> Result<SourceGameState, ModPresentationError<B::Error>> {
        self.assert_current()?;
        self.options
            .source
            .scene_context(&self.options.viewer)
            .map(|context| context.game_state)
            .ok_or_else(|| {
                ModPresentationError::Presentation(
                    "Component presentation viewer has no original source context".to_string(),
                )
            })
    }

    fn cvars(&self) -> Rc<RefCell<CvarRegistry>> {
        Rc::clone(&self.cvars)
    }

    fn print(&self, text: &str) {
        (self.options.print)(text);
    }

    fn milliseconds(&self) -> f64 {
        self.options.clock.now() + self.milliseconds_offset
    }

    fn memory_remaining(&self) -> i32 {
        (self.options.free_memory)().min(0x7fff_ffff) as i32
    }

    fn has_commands(&self) -> bool {
        self.options.commands.is_some()
    }

    fn command_append(&mut self, text: &str) -> Result<(), ModPresentationError<B::Error>> {
        if let Some(commands) = &self.options.commands {
            commands
                .append_command(text)
                .map_err(ModPresentationError::Presentation)?;
        }
        Ok(())
    }

    fn command_register(&mut self, name: &str) -> Result<(), ModPresentationError<B::Error>> {
        if let Some(commands) = &self.options.commands {
            commands
                .register_command(name)
                .map_err(ModPresentationError::Presentation)?;
            self.registered_commands.insert(name.to_string());
        }
        Ok(())
    }

    fn command_remove(&mut self, name: &str) -> Result<(), ModPresentationError<B::Error>> {
        if let Some(commands) = &self.options.commands {
            commands
                .remove_command(name)
                .map_err(ModPresentationError::Presentation)?;
            self.registered_commands.remove(name);
        }
        Ok(())
    }

    fn command_reliable(&mut self, text: &str) -> Result<(), ModPresentationError<B::Error>> {
        if let Some(commands) = &self.options.commands {
            commands
                .reliable_command(text)
                .map_err(ModPresentationError::Presentation)?;
        }
        Ok(())
    }

    fn actor_for_slot(&mut self, slot: i32) -> Result<ActorId, ModPresentationError<B::Error>> {
        self.assert_current()?;
        self.options.source.actor_for_slot(slot).ok_or_else(|| {
            ModPresentationError::Presentation(format!("Component sound source slot {slot} has no live actor"))
        })
    }

    fn live(&self, actor: &ActorId) -> bool {
        self.options.source.live(actor)
    }

    fn media_mut(&mut self) -> Result<&mut B::Media, ModPresentationError<B::Error>> {
        self.assert_current()?;
        self.media.as_mut().ok_or_else(|| {
            ModPresentationError::Presentation("Component presentation media are not initialized".to_string())
        })
    }

    fn services_mut(&mut self) -> Result<&mut B::Services, ModPresentationError<B::Error>> {
        self.assert_current()?;
        self.services.as_mut().ok_or_else(|| {
            ModPresentationError::Presentation("Component presentation services are not initialized".to_string())
        })
    }

    fn files_mut(&mut self) -> Result<&mut B::Files, ModPresentationError<B::Error>> {
        self.assert_current()?;
        self.files
            .as_mut()
            .ok_or_else(|| ModPresentationError::Presentation("Component host is not allocated".to_string()))
    }

    fn scripts_mut(&mut self) -> Result<&mut B::Scripts, ModPresentationError<B::Error>> {
        self.assert_current()?;
        self.scripts
            .as_mut()
            .ok_or_else(|| ModPresentationError::Presentation("Component host is not allocated".to_string()))
    }

    fn collision_mut(&mut self) -> Result<&mut B::Collision, ModPresentationError<B::Error>> {
        self.assert_current()?;
        self.collision
            .as_mut()
            .ok_or_else(|| ModPresentationError::Presentation("Component collision owner is closed".to_string()))
    }

    fn marks_mut(&mut self) -> Result<&mut B::Marks, ModPresentationError<B::Error>> {
        self.assert_current()?;
        self.marks
            .as_mut()
            .ok_or_else(|| ModPresentationError::Presentation("Component host is not allocated".to_string()))
    }

    fn display_renderer(&self) -> Option<&QvmDisplayRenderer> {
        self.options.renderer.as_deref()
    }

    fn viewport(&self) -> Rect {
        self.options.viewport
    }

    fn submit_scene(&mut self, scene: Q3PresentedScene) -> Result<(), ModPresentationError<B::Error>> {
        self.assert_current()?;
        if !declaration_hud(&self.options.source.declaration()) {
            self.options
                .output
                .submit_scene(scene)
                .map_err(ModPresentationError::Presentation)?;
            return Ok(());
        }
        if scene.source.render_flags & RDF_NOWORLDMODEL == 0 {
            return Err(ModPresentationError::Presentation(
                "Component HUD cannot replace the world view".to_string(),
            ));
        }
        self.pending_hud
            .push(ModPresentationHudSubmission::Scene(Box::new(scene)));
        Ok(())
    }

    fn submit_overlay(&mut self, command: Q3OverlayCommand) -> Result<(), ModPresentationError<B::Error>> {
        self.assert_current()?;
        if !declaration_hud(&self.options.source.declaration()) {
            let rendered = match command {
                Q3OverlayCommand::SetColor(color) => RenderCommand::SetColor(color),
                Q3OverlayCommand::StretchPic { rect, uv, image } => RenderCommand::StretchPic { rect, uv, image },
            };
            self.options
                .output
                .submit_command(rendered)
                .map_err(ModPresentationError::Presentation)?;
            return Ok(());
        }
        match command {
            Q3OverlayCommand::SetColor(color) => {
                self.hud_color = color;
                self.pending_hud
                    .push(ModPresentationHudSubmission::Command(Q3OverlayCommand::SetColor(color)));
            }
            Q3OverlayCommand::StretchPic { .. } => {
                self.pending_hud
                    .push(ModPresentationHudSubmission::Command(Q3OverlayCommand::SetColor(
                        self.hud_color,
                    )));
                self.pending_hud.push(ModPresentationHudSubmission::Command(command));
            }
        }
        Ok(())
    }

    fn submit_text(&mut self, draw: Q3ServiceTextDraw) -> Result<(), ModPresentationError<B::Error>> {
        self.assert_current()?;
        if !declaration_hud(&self.options.source.declaration()) {
            self.options
                .output
                .submit_text(draw)
                .map_err(ModPresentationError::Presentation)?;
            return Ok(());
        }
        self.pending_hud.push(ModPresentationHudSubmission::Text(draw));
        Ok(())
    }

    fn submit_listener(&mut self, origin: Vec3, axis: Axis) -> Result<(), ModPresentationError<B::Error>> {
        self.assert_current()?;
        self.options
            .output
            .submit_listener(origin, axis)
            .map_err(ModPresentationError::Presentation)?;
        Ok(())
    }

    fn emit_audio(&mut self, operation: Q3SeatAudioOperation) -> Result<(), ModPresentationError<B::Error>> {
        self.assert_current()?;
        self.audio_operations.push(operation);
        Ok(())
    }

    fn developer_print(&mut self, text: &str) {
        let enabled = self
            .cvars
            .borrow()
            .get("developer")
            .map_or(0, |variable| variable.integer_value)
            != 0;
        if enabled {
            (self.options.print)(text);
        }
    }

    fn check_collision_map(&mut self, path: &str) -> Result<(), ModPresentationError<B::Error>> {
        self.assert_current()?;
        if path != self.options.assets.map_geometry_path() {
            return Err(ModPresentationError::Presentation(format!(
                "Component requested a different collision map: {path}"
            )));
        }
        Ok(())
    }

    fn deliver_media(
        &mut self,
        request: &ComponentPresentationMediaRequest,
    ) -> Result<bool, ModPresentationError<B::Error>> {
        let Some(deliver) = &self.options.presentation_media else {
            return Ok(false);
        };
        let initializing = self.initializing;
        let current = || self.owns(&self.options.source, &self.options.viewer);
        deliver(request, initializing, &current).map_err(ModPresentationError::Presentation)?;
        self.assert_current()?;
        Ok(true)
    }

    fn next_frame(&mut self) -> Result<(), ModPresentationError<B::Error>> {
        (self.options.next_frame)();
        self.assert_current()
    }

    fn scalar(&mut self, call: &B::HostCall) -> Option<i32> {
        self.options.scalar.as_ref().and_then(|scalar| scalar(call, self))
    }
}

/// Read a required checkpoint field (donor `r.field(name).value`).
fn checkpoint_field<'a, E>(reader: &SaveReader<'a>, name: &str) -> Result<&'a SaveJson, ModPresentationError<E>> {
    let field = reader.field(name);
    let value = field.value;
    value.ok_or_else(|| field.fail("missing checkpoint field").into())
}

/// Write a scene context (donor `captureSceneContext`).
fn write_scene_context<E>(
    scene: &ComponentSceneContext<SourceGameState>,
    baseline: Option<&ComponentSceneContext<SourceGameState>>,
) -> Result<SaveJson, ModPresentationError<E>> {
    Ok(obj(vec![
        ("revision", int(scene.revision as i64)),
        ("gameState", write_game_state(&scene.game_state)),
        ("gameStateRevision", int(scene.game_state_revision as i64)),
        ("snapshot", write_snapshot(&scene.snapshot)?),
        (
            "actors",
            arr(scene
                .actors
                .iter()
                .map(|row| {
                    let saved = SavedActorId::from(&row.actor);
                    obj(vec![
                        ("slot", int(row.slot as i64)),
                        ("owned", boolean(row.owned)),
                        (
                            "actor",
                            obj(vec![
                                ("slot", int(saved.slot as i64)),
                                ("generation", int(saved.generation as i64)),
                            ]),
                        ),
                    ])
                })
                .collect()),
        ),
        (
            "commands",
            arr(scene
                .commands
                .iter()
                .map(|row| {
                    obj(vec![
                        ("sequence", int(row.sequence as i64)),
                        (
                            "arguments",
                            arr(row.arguments.iter().map(|text| save_str(text)).collect()),
                        ),
                    ])
                })
                .collect()),
        ),
        (
            "baseline",
            baseline.map_or(Ok(SaveJson::Null), |scene| write_scene_context(scene, None))?,
        ),
    ]))
}

/// Read a scene context with its nested baseline (donor `readSceneContext`).
fn read_scene_context<E>(
    reader: SaveReader,
    resolve_actor: &dyn Fn(&SavedActorId) -> Option<ActorId>,
    depth: u32,
) -> Result<SceneContextPair, ModPresentationError<E>> {
    if depth > 1 {
        return Err(reader.fail("nested scene baseline").into());
    }
    let baseline = reader
        .field("baseline")
        .nullable(|field| read_scene_context(field, resolve_actor, depth + 1))?
        .map(|(scene, _)| scene);
    Ok((
        ComponentSceneContext {
            revision: reader.field("revision").integer(0)? as u64,
            game_state: read_game_state(reader.field("gameState"))?,
            game_state_revision: reader.field("gameStateRevision").integer(0)? as u64,
            snapshot: read_snapshot(reader.field("snapshot"))?,
            actors: reader
                .field("actors")
                .list(|row| -> Result<ComponentSceneActor, ModPresentationError<E>> {
                    let actor = row.field("actor");
                    let saved = SavedActorId {
                        slot: actor.field("slot").integer(0)? as u32,
                        generation: actor.field("generation").integer(0)? as u32,
                    };
                    let resolved = resolve_actor(&saved).ok_or_else(|| row.fail("unresolvable saved actor"))?;
                    Ok(ComponentSceneActor {
                        actor: resolved,
                        slot: row.field("slot").integer(i64::from(i32::MIN))? as i32,
                        owned: row.field("owned").boolean()?,
                    })
                })?,
            commands: reader.field("commands").list(
                |row| -> Result<ComponentSceneCommandView, ModPresentationError<E>> {
                    Ok(ComponentSceneCommandView {
                        sequence: row.field("sequence").integer(0)? as u64,
                        arguments: row.field("arguments").list(|field| field.string())?,
                    })
                },
            )?,
        },
        baseline,
    ))
}

/// Write a game-state record.
fn write_game_state(state: &SourceGameState) -> SaveJson {
    obj(vec![
        (
            "offsets",
            arr(state.offsets.iter().map(|offset| int(*offset as i64)).collect()),
        ),
        ("data", SaveJson::Bytes(state.data.clone())),
        ("count", int(state.count as i64)),
    ])
}

/// Read a game-state record.
fn read_game_state<E>(reader: SaveReader) -> Result<SourceGameState, ModPresentationError<E>> {
    Ok(SourceGameState {
        offsets: reader
            .field("offsets")
            .list(|field| field.integer(i64::from(i32::MIN)))?
            .into_iter()
            .map(|offset| offset as i32)
            .collect(),
        data: reader.field("data").bytes()?,
        count: reader.field("count").integer(0)? as usize,
    })
}

/// Write a scene snapshot: states ride the net delta codec against default
/// baselines (the donor uses guest record bytes; the app states have no guest
/// record form, so the product-aware delta codec carries them instead).
fn write_snapshot<E>(snapshot: &ComponentSceneSnapshot) -> Result<SaveJson, ModPresentationError<E>> {
    let mut player = Q3MsgWriter::new(MessageMode::Bitstream, MAX_MESSAGE_LENGTH)?;
    write_delta_player_state(&mut player, None, &snapshot.player_state)?;
    let mut entities = Vec::with_capacity(snapshot.entities.len());
    for entity in &snapshot.entities {
        let mut writer = Q3MsgWriter::new(MessageMode::Bitstream, MAX_MESSAGE_LENGTH)?;
        write_delta_entity(&mut writer, None, Some(entity), true)?;
        entities.push(obj(vec![
            ("number", int(entity.number as i64)),
            ("state", SaveJson::Bytes(writer.to_bytes().to_vec())),
        ]));
    }
    Ok(obj(vec![
        ("serverTime", int(snapshot.server_time as i64)),
        ("flags", int(snapshot.flags as i64)),
        ("areaMask", SaveJson::Bytes(snapshot.area_mask.to_vec())),
        ("playerState", SaveJson::Bytes(player.to_bytes().to_vec())),
        (
            "product",
            int(match snapshot.player_state.product {
                Q3Product::Base => 0,
                Q3Product::MissionPack => 1,
            }),
        ),
        ("entities", arr(entities)),
        ("serverCommandSequence", int(snapshot.server_command_sequence as i64)),
    ]))
}

/// Read a scene snapshot.
fn read_snapshot<E>(reader: SaveReader) -> Result<ComponentSceneSnapshot, ModPresentationError<E>> {
    let area_mask = reader.field("areaMask").bytes()?;
    if area_mask.len() != 32 {
        return Err(reader.field("areaMask").fail("invalid scene area mask").into());
    }
    let mut mask = [0u8; 32];
    mask.copy_from_slice(&area_mask);
    let product = match reader.field("product").integer(0)? {
        0 => Q3Product::Base,
        1 => Q3Product::MissionPack,
        _ => return Err(reader.field("product").fail("unknown player state product").into()),
    };
    let player_bytes = reader.field("playerState").bytes()?;
    let mut player_reader = Q3MsgReader::new(&player_bytes, MessageMode::Bitstream)?;
    let player_state = read_delta_player_state(&mut player_reader, None, product, None)?;
    let entities = reader
        .field("entities")
        .list(|row| -> Result<Q3EntityState, ModPresentationError<E>> {
            let number = row.field("number").integer(i64::from(i32::MIN))? as i32;
            let bytes = row.field("state").bytes()?;
            let mut entity_reader = Q3MsgReader::new(&bytes, MessageMode::Bitstream)?;
            // The decoder expects the snapshot loop to have consumed the
            // number; standalone reads discard it (the stored key wins).
            let _stream_number = entity_reader.read_bits(ENTITY_NUMBER_BITS)?;
            let mut entity = read_delta_entity(&mut entity_reader, &Q3EntityState::default(), number, None)?;
            entity.number = number;
            Ok(entity)
        })?;
    Ok(ComponentSceneSnapshot {
        server_time: reader.field("serverTime").integer(i64::from(i32::MIN))? as i32,
        flags: reader.field("flags").integer(i64::from(i32::MIN))? as i32,
        area_mask: mask,
        player_state,
        entities,
        server_command_sequence: reader.field("serverCommandSequence").integer(0)? as u64,
    })
}

/// Write a cvar save state.
fn write_cvars(state: &CvarSaveState) -> SaveJson {
    obj(vec![
        ("dialect", save_str(&format!("{:?}", state.dialect))),
        (
            "variables",
            arr(state
                .variables
                .iter()
                .map(|row| {
                    row.as_ref().map_or(SaveJson::Null, |variable| {
                        obj(vec![
                            ("name", save_str(&variable.name)),
                            ("value", save_str(&variable.value)),
                            ("resetValue", save_str(&variable.reset_value)),
                            (
                                "latchedValue",
                                variable
                                    .latched_value
                                    .as_ref()
                                    .map_or(SaveJson::Null, |latched| save_str(latched)),
                            ),
                            ("flags", int(variable.flags as i64)),
                            ("modified", boolean(variable.modified)),
                            ("modificationCount", int(variable.modification_count as i64)),
                            ("numericValue", num(variable.numeric_value as f64)),
                            ("integerValue", int(variable.integer_value as i64)),
                        ])
                    })
                })
                .collect()),
        ),
        ("order", arr(state.order.iter().map(|name| save_str(name)).collect())),
        ("changedFlags", int(state.changed_flags as i64)),
        ("cheatsEnabled", boolean(state.cheats_enabled)),
        ("serverActive", boolean(state.server_active)),
        ("clientConnected", boolean(state.client_connected)),
        ("highCharacters", boolean(state.high_characters)),
        ("clientInfo", save_str(&state.client_info)),
        ("serverInfo", save_str(&state.server_info)),
        ("userinfoDirty", boolean(state.userinfo_dirty)),
        (
            "consoleVariables",
            arr(state.console_variables.iter().map(|name| save_str(name)).collect()),
        ),
        (
            "aliasHandles",
            arr(state
                .alias_handles
                .iter()
                .map(|(handle, alias)| obj(vec![("handle", int(*handle as i64)), ("alias", save_str(alias))]))
                .collect()),
        ),
    ])
}

/// Read a cvar save state.
fn read_cvars<E>(reader: SaveReader) -> Result<CvarSaveState, ModPresentationError<E>> {
    let dialect = match reader.field("dialect").string()?.as_str() {
        "Q1Netquake" => Dialect::Q1Netquake,
        "Q1Quakeworld" => Dialect::Q1Quakeworld,
        "Q2Classic" => Dialect::Q2Classic,
        "Q2Rerelease" => Dialect::Q2Rerelease,
        "Q3" => Dialect::Q3,
        _ => return Err(reader.field("dialect").fail("unknown cvar dialect").into()),
    };
    Ok(CvarSaveState {
        dialect,
        variables: reader.field("variables").list(|row| {
            row.nullable(|field| -> Result<SavedCvarState, ModPresentationError<E>> {
                Ok(SavedCvarState {
                    name: field.field("name").string()?,
                    value: field.field("value").string()?,
                    reset_value: field.field("resetValue").string()?,
                    latched_value: field.field("latchedValue").nullable(|latched| latched.string())?,
                    flags: field.field("flags").integer(0)? as u32,
                    modified: field.field("modified").boolean()?,
                    modification_count: field.field("modificationCount").integer(0)? as u32,
                    numeric_value: field.field("numericValue").number()? as f32,
                    integer_value: field.field("integerValue").integer(i64::from(i32::MIN))? as i32,
                })
            })
        })?,
        order: reader.field("order").list(|field| field.string())?,
        changed_flags: reader.field("changedFlags").integer(0)? as u32,
        cheats_enabled: reader.field("cheatsEnabled").boolean()?,
        server_active: reader.field("serverActive").boolean()?,
        client_connected: reader.field("clientConnected").boolean()?,
        high_characters: reader.field("highCharacters").boolean()?,
        client_info: reader.field("clientInfo").string()?,
        server_info: reader.field("serverInfo").string()?,
        userinfo_dirty: reader.field("userinfoDirty").boolean()?,
        console_variables: reader.field("consoleVariables").list(|field| field.string())?,
        alias_handles: reader.field("aliasHandles").list(
            |row| -> Result<(usize, String), ModPresentationError<E>> {
                Ok((row.field("handle").integer(0)? as usize, row.field("alias").string()?))
            },
        )?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_content::contract::{ContentDigest, MountPlanId, ResolvedMountPlan};
    use qa_content::mounts::{open_mount_plan, OpenMountOptions};
    use qa_content::q3::presentation::scene::{snapshot_q3_scene_admission, SceneAdmissionOrigin};
    use qa_core::identity::{IdentityOwner, ProviderId};
    use qa_core::math::Bounds;
    use qa_guest::qvm::mod_presentation::{
        HudMode, PlayerEventCentities, PlayerEventStorage, PresentationHud, QvmPlayerEventPresentation,
        QvmPresentationCall, QvmPresentationProgram,
    };
    use qa_guest::qvm::mod_provider::{QvmImage, QvmRole};
    use qa_net::q3_net::Q3PlayerState;

    #[derive(Debug)]
    struct MockError(String);

    impl std::fmt::Display for MockError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "{}", self.0)
        }
    }

    impl std::error::Error for MockError {}

    struct MockBackend {
        mounts: Rc<MountedContent>,
        scene: Q3SceneContent,
    }

    impl MockBackend {
        fn new() -> Self {
            let plan = ResolvedMountPlan {
                id: MountPlanId("mock:test:1".to_string()),
                mounts: Vec::new(),
                default_order: Vec::new(),
                prefix_orders: Vec::new(),
            };
            let mounts = Rc::new(open_mount_plan(&plan, OpenMountOptions::default()).unwrap());
            let scene = Q3SceneContent {
                admission: snapshot_q3_scene_admission(SceneAdmissionOrigin::Native, Vec::new(), Vec::new()),
                models: Vec::new(),
                effects: Vec::new(),
                special_entities: Vec::new(),
                portals: Vec::new(),
                lights: Vec::new(),
            };
            Self { mounts, scene }
        }
    }

    impl ModPresentationBackend for MockBackend {
        type Error = MockError;
        type Media = ();
        type Services = ();
        type Core = ();
        type Renderer = ();
        type Files = ();
        type Scripts = ();
        type Collision = ();
        type Marks = ();
        type Cinematics = ();
        type HostCall = ();

        fn create_media(&mut self, _: &ContentId, _: PrintCallback) -> Result<(), MockError> {
            Ok(())
        }

        fn media_mounts(&self, _: &()) -> Rc<MountedContent> {
            Rc::clone(&self.mounts)
        }

        fn create_collision(&mut self, _: bool, _: &str, _: Rc<RefCell<CvarRegistry>>) -> Result<(), MockError> {
            Ok(())
        }

        fn create_marks(&mut self) -> Result<(), MockError> {
            Ok(())
        }

        fn create_services(&mut self, _: ModServicesParams<Self>) -> Result<(), MockError> {
            Ok(())
        }

        fn create_renderer(&mut self, _: &(), _: &()) -> Result<(), MockError> {
            Ok(())
        }

        fn create_files(&mut self, _: Rc<MountedContent>, _: Option<bool>, _: PrintCallback) -> Result<(), MockError> {
            Ok(())
        }

        fn create_scripts(
            &mut self,
            _: Rc<MountedContent>,
            _: Option<bool>,
            _: PrintCallback,
        ) -> Result<(), MockError> {
            Ok(())
        }

        fn create_core(&mut self, _: ModCoreParams) -> Result<(), MockError> {
            Ok(())
        }

        fn initialize_core(&mut self, _: &mut (), _: i64, _: &mut dyn ModSyscalls<Self>) -> Result<(), MockError> {
            Ok(())
        }

        fn advance_core(&mut self, _: &mut (), _: i64, _: &mut dyn ModSyscalls<Self>) -> Result<(), MockError> {
            Ok(())
        }

        fn consume_event(
            &mut self,
            _: &mut (),
            _: &Q3SourcePlayerEvent,
            _: i64,
            _: &mut dyn ModSyscalls<Self>,
        ) -> Result<(), MockError> {
            Ok(())
        }

        fn console_command(
            &mut self,
            _: &mut (),
            _: &[String],
            _: &mut dyn ModSyscalls<Self>,
        ) -> Result<bool, MockError> {
            Ok(true)
        }

        fn draw_hud(&mut self, _: &mut (), _: i64, _: &mut dyn ModSyscalls<Self>) -> Result<(), MockError> {
            Ok(())
        }

        fn capture_bodies(
            &mut self,
            _: &(),
            _: &(),
            _: &PresentationOwner,
            _: &ContentId,
            _: f64,
        ) -> Result<Vec<ComponentBody>, MockError> {
            Ok(Vec::new())
        }

        fn capture_scene(&mut self, _: &mut ()) -> Result<Q3SceneContent, MockError> {
            Ok(self.scene.clone())
        }

        fn clear_scene(&mut self, _: &mut ()) {}

        fn preload(&mut self, _: &mut (), _: &Q3SceneContent, _: &[Q3PresentedScene]) -> Result<(), MockError> {
            Ok(())
        }

        fn capture_core(&self, _: &()) -> Result<SaveJson, MockError> {
            Ok(SaveJson::Null)
        }

        fn restore_core(
            &mut self,
            _: &mut (),
            _: &SaveJson,
            _: &dyn Fn(&SavedActorId) -> Option<ActorId>,
        ) -> Result<(), MockError> {
            Ok(())
        }

        fn capture_files(&self, _: &()) -> Result<SaveJson, MockError> {
            Ok(SaveJson::Null)
        }

        fn restore_files(&mut self, _: &mut (), _: &SaveJson) -> Result<(), MockError> {
            Ok(())
        }

        fn capture_scripts(&self, _: &()) -> Result<SaveJson, MockError> {
            Ok(SaveJson::Null)
        }

        fn restore_scripts(&mut self, _: &mut (), _: &SaveJson) -> Result<(), MockError> {
            Ok(())
        }

        fn capture_resources(&self, _: &()) -> Result<SaveJson, MockError> {
            Ok(SaveJson::Null)
        }

        fn restore_resources(&mut self, _: &mut (), _: &SaveJson) -> Result<(), MockError> {
            Ok(())
        }

        fn capture_sounds(&self, _: &()) -> Result<SaveJson, MockError> {
            Ok(SaveJson::Null)
        }

        fn restore_sounds(&mut self, _: &mut (), _: &SaveJson) -> Result<(), MockError> {
            Ok(())
        }

        fn capture_fonts(&self, _: &()) -> Result<SaveJson, MockError> {
            Ok(SaveJson::Null)
        }

        fn restore_fonts(&mut self, _: &mut (), _: &SaveJson) -> Result<(), MockError> {
            Ok(())
        }

        fn capture_collision(&self, _: &()) -> Result<SaveJson, MockError> {
            Ok(SaveJson::Null)
        }

        fn restore_collision(&mut self, _: &mut (), _: &SaveJson) -> Result<(), MockError> {
            Ok(())
        }

        fn capture_cinematics(&self, _: &()) -> Result<SaveJson, MockError> {
            Ok(SaveJson::Null)
        }

        fn restore_cinematics(&mut self, _: &mut (), _: &SaveJson) -> Result<(), MockError> {
            Ok(())
        }

        fn publish_restored_cinematics(&mut self, _: &mut ()) -> Result<(), MockError> {
            Ok(())
        }

        fn close_core(&mut self, _: &mut ()) -> Result<(), MockError> {
            Ok(())
        }

        fn close_files(&mut self, _: &mut ()) -> Result<(), MockError> {
            Ok(())
        }

        fn close_scripts(&mut self, _: &mut ()) -> Result<(), MockError> {
            Ok(())
        }

        fn close_cinematics(&mut self, _: &mut ()) -> Result<(), MockError> {
            Ok(())
        }

        fn close_renderer(&mut self, _: &mut ()) -> Result<(), MockError> {
            Ok(())
        }

        fn close_media(&mut self, _: &mut ()) -> Result<(), MockError> {
            Ok(())
        }
    }

    struct MockSource {
        generation: u64,
        live: bool,
        module: ModuleId,
        prepared: ModuleId,
        declaration: QvmModPresentationDeclaration,
        content: ContentId,
        source_id: ActorId,
        owner: PresentationOwner,
        context: Option<ComponentSceneContext<SourceGameState>>,
        actor: ActorId,
    }

    impl ModPresentationSource for MockSource {
        fn generation(&self) -> u64 {
            self.generation
        }

        fn live(&self, viewer: &ActorId) -> bool {
            self.live && viewer == &self.actor
        }

        fn assert_current(&self) -> Result<(), String> {
            Ok(())
        }

        fn live_module(&self) -> ModuleId {
            self.module.clone()
        }

        fn prepared_module(&self) -> ModuleId {
            self.prepared.clone()
        }

        fn source_abi(&self) -> QvmAbi {
            QvmAbi::Modern
        }

        fn declaration(&self) -> QvmModPresentationDeclaration {
            self.declaration.clone()
        }

        fn core_artifact(&self) -> QvmArtifact {
            QvmArtifact {
                module: self.prepared.clone(),
                role: QvmRole::Cgame,
                abi_profile: None,
                image: QvmImage {
                    instructions: Vec::new(),
                    data_length: 0,
                    literal_length: 0,
                    bss_length: 0,
                    allocated_data_length: 0,
                    initialized_length: 0,
                },
            }
        }

        fn declaration_checkpoint(&self) -> SaveJson {
            obj(vec![("revision", save_str("mock"))])
        }

        fn identity_content(&self) -> ContentId {
            self.content.clone()
        }

        fn identity_checkpoint(&self) -> SaveJson {
            obj(vec![("content", save_str(self.content.as_str()))])
        }

        fn prepared_contract_module(&self) -> ModuleIdentity {
            ModuleIdentity {
                id: ProviderId {
                    namespace: "mock".to_string(),
                    name: self.prepared.id.clone(),
                },
                artifact_path: self.prepared.artifact_path.clone(),
                digest: ContentDigest("sha256:00".to_string()),
                revision: self.prepared.revision.clone(),
            }
        }

        fn client_command(&self, _: &ActorId, _: &[String]) -> Result<bool, String> {
            Ok(false)
        }

        fn prepared_source_id(&self) -> ActorId {
            self.source_id.clone()
        }

        fn presentation_owner(&self) -> PresentationOwner {
            self.owner.clone()
        }

        fn scene_context(&self, _: &ActorId) -> Option<ComponentSceneContext<SourceGameState>> {
            self.context.clone()
        }

        fn scene_publication(&self) -> Option<ModScenePublication> {
            None
        }

        fn actor_for_slot(&self, _: i32) -> Option<ActorId> {
            Some(self.actor.clone())
        }

        fn source_mounts(&self) -> Option<Rc<MountedContent>> {
            None
        }

        fn source_files_writable(&self) -> Option<bool> {
            None
        }
    }

    struct MockAssets;

    impl ModPresentationAssets for MockAssets {
        fn map_geometry_path(&self) -> &str {
            "maps/test.bsp"
        }

        fn world_leaf_count(&self) -> i32 {
            1
        }
    }

    struct MockAudio {
        frames: RefCell<Vec<ModCgameAudioFrame>>,
    }

    impl ModAudioSink for MockAudio {
        fn receive_cgame_frame(&self, frame: ModCgameAudioFrame) {
            self.frames.borrow_mut().push(frame);
        }
    }

    struct MockQueries;

    impl ApplicationQ3SceneQueries for MockQueries {
        fn point_leaf(&self, _: Vec3) -> i32 {
            0
        }

        fn leaf_cluster(&self, _: i32) -> i32 {
            0
        }

        fn leaf_area(&self, _: i32) -> i32 {
            0
        }

        fn area_bits(&self, _: i32) -> Vec<u8> {
            Vec::new()
        }

        fn box_leaves(&self, _: Bounds, _: i32) -> Vec<i32> {
            Vec::new()
        }

        fn cluster_visible(&self, _: i32, _: i32) -> bool {
            true
        }

        fn areas_connected(&self, _: i32, _: i32) -> bool {
            true
        }
    }

    impl ModSceneQueries for MockQueries {
        fn has_native_collision(&self) -> bool {
            false
        }
    }

    struct MockClock {
        now: f64,
        frame: i64,
    }

    impl ModPresentationClock for MockClock {
        fn now(&self) -> f64 {
            self.now
        }

        fn frame_number(&self) -> i64 {
            self.frame
        }
    }

    struct MockOutput {
        scenes: RefCell<usize>,
        commands: RefCell<Vec<RenderCommand>>,
    }

    impl ModPresentationOutput for MockOutput {
        fn submit_scene(&self, _: Q3PresentedScene) -> Result<(), String> {
            *self.scenes.borrow_mut() += 1;
            Ok(())
        }

        fn submit_command(&self, command: RenderCommand) -> Result<(), String> {
            self.commands.borrow_mut().push(command);
            Ok(())
        }

        fn submit_text(&self, _: Q3ServiceTextDraw) -> Result<(), String> {
            Ok(())
        }

        fn submit_listener(&self, _: Vec3, _: Axis) -> Result<(), String> {
            Ok(())
        }
    }

    struct MockCommands {
        registered: RefCell<HashSet<String>>,
    }

    impl ModCommandHost for MockCommands {
        fn append_command(&self, _: &str) -> Result<(), String> {
            Ok(())
        }

        fn register_command(&self, name: &str) -> Result<(), String> {
            self.registered.borrow_mut().insert(name.to_string());
            Ok(())
        }

        fn remove_command(&self, name: &str) -> Result<(), String> {
            self.registered.borrow_mut().remove(name);
            Ok(())
        }

        fn reliable_command(&self, _: &str) -> Result<(), String> {
            Ok(())
        }
    }

    fn module(id: &str) -> ModuleId {
        ModuleId {
            id: id.to_string(),
            artifact_path: "mock/test".to_string(),
            digest: "00".to_string(),
            revision: "1".to_string(),
        }
    }

    fn player_declaration(hud: bool) -> QvmModPresentationDeclaration {
        let program = || QvmPresentationProgram {
            path: "mock".to_string(),
            digest: "00".to_string(),
            abi: QvmAbi::Modern,
        };
        QvmModPresentationDeclaration::PlayerEvents(QvmPlayerEventPresentation {
            gameplay: program(),
            cgame: program(),
            initialize: Vec::new(),
            refresh: Vec::new(),
            frame: Vec::new(),
            hud: hud.then(|| PresentationHud {
                mode: HudMode::Overlay,
                frame: Vec::new(),
            }),
            storage: PlayerEventStorage {
                game_state: 0,
                player_state: 0,
                snapshot_address: 0,
                snapshot_pointers: Vec::new(),
                centities: PlayerEventCentities {
                    address: 0,
                    stride: 0,
                    capacity: 0,
                    state: 0,
                    origin: 0,
                },
                time: Vec::new(),
                frame_time: Vec::new(),
                view_origin: Vec::new(),
                view_angles: Vec::new(),
                view_axis: Vec::new(),
            },
            project: Vec::new(),
            event: QvmPresentationCall {
                entry: 0,
                when_weapon_presented: false,
                arguments: Vec::new(),
            },
        })
    }

    fn test_context(actor: ActorId) -> ComponentSceneContext<SourceGameState> {
        ComponentSceneContext {
            revision: 3,
            game_state: SourceGameState::empty(),
            game_state_revision: 1,
            snapshot: ComponentSceneSnapshot {
                server_time: 1000,
                flags: 0,
                area_mask: [7u8; 32],
                player_state: Q3PlayerState::new(Q3Product::Base),
                entities: vec![Q3EntityState {
                    number: 5,
                    e_type: 3,
                    ..Q3EntityState::default()
                }],
                server_command_sequence: 9,
            },
            actors: vec![ComponentSceneActor {
                actor,
                slot: 2,
                owned: true,
            }],
            commands: vec![ComponentSceneCommandView {
                sequence: 4,
                arguments: vec!["say".to_string(), "hi".to_string()],
            }],
        }
    }

    struct Fixture {
        authority: IdentityOwner,
        viewer: ActorId,
        seat: SeatId,
        assets: Rc<MockAssets>,
        audio: Rc<MockAudio>,
        queries: Rc<MockQueries>,
        clock: Rc<MockClock>,
        output: Rc<MockOutput>,
        commands: Rc<MockCommands>,
    }

    impl Fixture {
        fn new() -> Self {
            let authority = IdentityOwner::create("mod-presentation-test").unwrap();
            let viewer = authority.actor(0, 1);
            let seat = authority.seat(0);
            Self {
                authority,
                viewer,
                seat,
                assets: Rc::new(MockAssets),
                audio: Rc::new(MockAudio {
                    frames: RefCell::new(Vec::new()),
                }),
                queries: Rc::new(MockQueries),
                clock: Rc::new(MockClock { now: 5000.0, frame: 40 }),
                output: Rc::new(MockOutput {
                    scenes: RefCell::new(0),
                    commands: RefCell::new(Vec::new()),
                }),
                commands: Rc::new(MockCommands {
                    registered: RefCell::new(HashSet::new()),
                }),
            }
        }

        fn source(&self, hud: bool) -> Rc<MockSource> {
            let identical = module("mock");
            Rc::new(MockSource {
                generation: 7,
                live: true,
                module: identical.clone(),
                prepared: identical,
                declaration: player_declaration(hud),
                content: ContentId("mock:test:1".to_string()),
                source_id: self.authority.actor(1, 1),
                owner: PresentationOwner {
                    provider: ProviderId {
                        namespace: "mock".to_string(),
                        name: "provider".to_string(),
                    },
                    generation: 7,
                },
                context: Some(test_context(self.viewer.clone())),
                actor: self.viewer.clone(),
            })
        }

        fn options(&self, source: Rc<MockSource>) -> ApplicationModPresentationOptions<MockBackend> {
            ApplicationModPresentationOptions {
                assets: self.assets.clone(),
                audio: self.audio.clone(),
                queries: self.queries.clone(),
                seat: self.seat.clone(),
                viewer: self.viewer.clone(),
                viewport: Rect {
                    x: 0,
                    y: 0,
                    width: 640,
                    height: 480,
                },
                renderer: None,
                source,
                clock: self.clock.clone(),
                output: self.output.clone(),
                system_cinematics: None,
                commands: Some(self.commands.clone()),
                presentation_media: None,
                print: Rc::new(|_| {}),
                view_origin: Rc::new(|| Vec3 { x: 0.0, y: 0.0, z: 0.0 }),
                view_axis: None,
                next_frame: Rc::new(|| {}),
                scalar: None,
                free_memory: Rc::new(|| 1 << 30),
            }
        }
    }

    #[test]
    fn rejects_module_mismatch() {
        let fixture = Fixture::new();
        let mut source = fixture.source(false);
        Rc::get_mut(&mut source).unwrap().prepared = module("other");
        let mut backend = MockBackend::new();
        let error = ApplicationModPresentation::create(fixture.options(source), &mut backend)
            .err()
            .expect("mismatched modules must fail");
        assert!(matches!(error, ModPresentationError::Presentation(message)
                if message == "Component presentation source differs from its prepared gameplay module"));
    }

    #[test]
    fn scene_context_checkpoint_roundtrips() {
        let authority = IdentityOwner::create("mod-presentation-roundtrip").unwrap();
        let actor = authority.actor(2, 3);
        let scene = test_context(actor.clone());
        let mut baseline = test_context(actor.clone());
        baseline.revision = 2;
        let json = write_scene_context::<MockError>(&scene, Some(&baseline)).unwrap();
        let resolve = |saved: &SavedActorId| (*saved == SavedActorId::from(&actor)).then(|| actor.clone());
        let (back, base) = read_scene_context::<MockError>(SaveReader::at(&json, "test"), &resolve, 0).unwrap();
        assert_eq!(back.revision, scene.revision);
        assert_eq!(back.game_state, scene.game_state);
        assert_eq!(back.game_state_revision, scene.game_state_revision);
        assert_eq!(back.snapshot.server_time, scene.snapshot.server_time);
        assert_eq!(back.snapshot.flags, scene.snapshot.flags);
        assert_eq!(back.snapshot.area_mask, scene.snapshot.area_mask);
        assert_eq!(back.snapshot.player_state, scene.snapshot.player_state);
        assert_eq!(back.snapshot.entities, scene.snapshot.entities);
        assert_eq!(
            back.snapshot.server_command_sequence,
            scene.snapshot.server_command_sequence
        );
        assert_eq!(back.actors, scene.actors);
        assert_eq!(back.commands, scene.commands);
        assert_eq!(base.as_ref(), Some(&baseline));
    }

    #[test]
    fn scene_context_rejects_nested_baseline() {
        let authority = IdentityOwner::create("mod-presentation-depth").unwrap();
        let actor = authority.actor(2, 3);
        let scene = test_context(actor.clone());
        let inner = write_scene_context::<MockError>(&scene, None).unwrap();
        let mut middle = write_scene_context::<MockError>(&scene, None).unwrap();
        replace_baseline(&mut middle, inner);
        let mut outer = write_scene_context::<MockError>(&scene, None).unwrap();
        replace_baseline(&mut outer, middle);
        let resolve = |saved: &SavedActorId| (*saved == SavedActorId::from(&actor)).then(|| actor.clone());
        let error = read_scene_context::<MockError>(SaveReader::at(&outer, "test"), &resolve, 0)
            .expect_err("depth-two baselines must fail");
        assert!(matches!(error, ModPresentationError::World(_)));
    }

    fn replace_baseline(json: &mut SaveJson, baseline: SaveJson) {
        let SaveJson::Object(members) = json else {
            panic!("scene context must be an object");
        };
        let slot = members
            .iter_mut()
            .find(|(name, _)| name == "baseline")
            .expect("scene context must carry a baseline member");
        slot.1 = baseline;
    }

    #[test]
    fn cvar_checkpoint_roundtrips() {
        let mut registry = CvarRegistry::new(Dialect::Q3);
        registry.set("developer", "1", false).unwrap();
        let state = registry.capture_save_state().unwrap();
        let json = write_cvars(&state);
        let back = read_cvars::<MockError>(SaveReader::at(&json, "test")).unwrap();
        assert_eq!(back, state);
    }

    #[test]
    fn owns_tracks_source_generation_and_viewer() {
        let fixture = Fixture::new();
        let source = fixture.source(false);
        let candidate: Rc<dyn ModPresentationSource> = source.clone();
        let mut backend = MockBackend::new();
        let mut owner = ApplicationModPresentation::create(fixture.options(source), &mut backend).unwrap();
        assert!(owner.owns(&candidate, &fixture.viewer));
        let stranger = fixture.authority.actor(9, 9);
        assert!(!owner.owns(&candidate, &stranger));
        let other: Rc<dyn ModPresentationSource> = fixture.source(false);
        assert!(!owner.owns(&other, &fixture.viewer));
        owner.close(&mut backend).unwrap();
        assert!(!owner.owns(&candidate, &fixture.viewer));
    }

    #[test]
    fn frame_caches_completed_sequence() {
        let fixture = Fixture::new();
        let mut backend = MockBackend::new();
        let mut owner =
            ApplicationModPresentation::create(fixture.options(fixture.source(false)), &mut backend).unwrap();
        let first = owner.frame(&mut backend, 0).unwrap();
        let second = owner.frame(&mut backend, 0).unwrap();
        assert_eq!(first, second);
        assert!(owner.frame(&mut backend, -1).is_err());
        owner.capture_checkpoint(&backend).unwrap();
        assert!(owner.bodies().unwrap().is_empty());
        assert!(owner.hud().unwrap().is_empty());
    }

    #[test]
    fn submit_overlay_tracks_hud_color() {
        let fixture = Fixture::new();
        let mut backend = MockBackend::new();
        let mut owner =
            ApplicationModPresentation::create(fixture.options(fixture.source(true)), &mut backend).unwrap();
        let red = Vec4 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
            w: 1.0,
        };
        owner.submit_overlay(Q3OverlayCommand::SetColor(red)).unwrap();
        assert_eq!(owner.pending_hud.len(), 1);
        assert!(owner.capture_checkpoint(&backend).is_err());
        let scene = owner.frame(&mut backend, 0).unwrap();
        assert_eq!(owner.hud().unwrap().len(), 1);
        assert_eq!(owner.captured, Some(scene));
        owner.capture_checkpoint(&backend).unwrap();
    }

    #[test]
    fn capture_restore_roundtrips() {
        let fixture = Fixture::new();
        let mut backend = MockBackend::new();
        let mut owner =
            ApplicationModPresentation::create(fixture.options(fixture.source(false)), &mut backend).unwrap();
        owner.command_register("mock-command").unwrap();
        owner.frame(&mut backend, 0).unwrap();
        let checkpoint = owner.capture_checkpoint(&backend).unwrap();

        let restore = Fixture::new();
        let mut restore_backend = MockBackend::new();
        let mut restored = ApplicationModPresentation::restore(
            restore.options(restore.source(false)),
            &mut restore_backend,
            &checkpoint,
            &|saved: &SavedActorId| (*saved == SavedActorId::from(&fixture.viewer)).then(|| fixture.viewer.clone()),
        )
        .unwrap();
        assert_eq!(restored.frame_sequence, 0);
        assert_eq!(restored.previous_frame_time, Some(1000.0));
        assert!(restored.registered_commands.contains("mock-command"));
        assert!(!restored.published);
        restored.publish_commands(&mut restore_backend).unwrap();
        assert!(restored.published);
        assert!(restore.commands.registered.borrow().contains("mock-command"));
    }

    impl ModSceneRendererOps<MockError> for () {
        fn scene_operations(
            &mut self,
            _: &Q3SceneContent,
            _: &WorldViewInput,
            first_entity: u32,
            _: &Q3SceneRenderOptions,
        ) -> Result<Vec<SceneOperation>, MockError> {
            assert_eq!(first_entity, 7);
            Ok(Vec::new())
        }
    }

    #[test]
    fn scene_operations_delegate_to_renderer() {
        use qa_client::render::types::{SourceTime, ViewTarget};
        use qa_client::view::{CameraClip, Rect as ViewRect, SceneCamera};
        use qa_core::math::identity_mat4;

        let fixture = Fixture::new();
        let mut backend = MockBackend::new();
        let mut owner =
            ApplicationModPresentation::create(fixture.options(fixture.source(false)), &mut backend).unwrap();
        let scene = owner.frame(&mut backend, 0).unwrap();
        let zero = Vec3 { x: 0.0, y: 0.0, z: 0.0 };
        let camera = SceneCamera {
            origin: zero,
            axis: [zero, zero, zero],
            projection: identity_mat4(),
            viewport: ViewRect {
                x: 0,
                y: 0,
                width: 640,
                height: 480,
            },
            clip: CameraClip::None,
        };
        let input = WorldViewInput::new(
            camera,
            ViewTarget::Seat(fixture.seat.clone()),
            SourceTime::Milliseconds(100.0),
        );
        let options = Q3SceneRenderOptions {
            view_offset: None,
            no_world_model: false,
            split_screen: false,
            supplemental_view_weapon: false,
        };
        let operations = owner.scene_operations(&scene, &input, 7, &options).unwrap();
        assert!(operations.is_empty());
    }
}
