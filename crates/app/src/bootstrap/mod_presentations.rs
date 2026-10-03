//! Port of Quake-Anthology-TS `src/app/bootstrap/mod-presentations.ts`
//!
//! The local seats share source events; each cgame keeps its own viewer,
//! media, and transient state. The collection owns one
//! [`ApplicationModPresentation`](super::mod_presentation::ApplicationModPresentation)
//! per seat and source, wires each consumer's command host to the client
//! command services, binds each consumer's scene and HUD into its seat, and
//! checkpoints the whole collection.
//!
//! Documented folds: the donor's async create/restore/consume/frame/dispatch
//! calls are sync through the host backend; producer instances are `u64`
//! tokens (donor `symbol`s); the collection session arrives in the options
//! because the seat token is hidden; effect and command callbacks are shared
//! `Rc` handles because the seat and the command services retain them past
//! any single call; the seat overlay pass carries no frame builder, so HUD
//! submissions route through the host [`ModHudDraw`] sink; the seat camera
//! and viewport reads are fallible while the guest view callbacks are not,
//! so a failing read records its error and the next driven frame reports it;
//! entity-range reservation failure yields an empty effect frame (the donor
//! reserve cannot fail); q3-source player events arrive as
//! `Foreign { kind: "q3-source" }` payloads projected through
//! [`ModQ3EventPayload`] because the source-event enum carries no Q3 variant.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;

use qa_client::materials::lighting::SurfaceDynamicLight;
use qa_client::materials::q3_lighting::DynamicLight;
use qa_client::render::scene::models::types::CustomSkinEntry;
use qa_client::render::scene::submissions::reserve_source_entity_range;
use qa_client::render::scene::world::{create_world_surface_admission, WorldViewInput};
use qa_client::render::types::{SceneFog, SourceTime, ViewTarget};
use qa_client::view::{Rect as ViewRect, SceneCamera};
use qa_content::contract::{
    ComponentPresentationMediaRequest, ContractError, ModCommandCvars, ModCommandInvocation, ModCommandProducer,
    ModCommandSource, ModuleIdentity, PresentationOwner, QvmAbiProfile,
};
use qa_content::mounts::{MountError, MountedContent};
use qa_content::q3::presentation::scene::{Q3SceneContent, Rect as ContentRect};
use qa_core::cmd::{tokenize_command, CmdError, Dialect, TextMode};
use qa_core::cmd_buffer::CommandOrigin;
use qa_core::cvar::{CvarError, CvarRegistry};
use qa_core::identity::{ActorId, ClientId, ProviderId, SavedActorId, SeatId, SessionId};
use qa_core::math::Vec3;
use qa_guest::qvm::mod_presentation::QvmModPresentationDeclaration;
use qa_guest::qvm::mod_provider::QvmAbi;
use qa_net::q3_net::Q3NetError;
use qa_world::save::records::{read_saved_actor, write_saved_actor};
use qa_world::save::value::{arr, int, obj, str as save_str, SaveJson, SaveReader};
use qa_world::WorldError;

use super::component_bodies::ComponentBody;
use super::component_scene::ComponentSceneError;
use super::mod_presentation::{
    ApplicationModPresentation, ApplicationModPresentationOptions, FreeMemoryCallback, ModAudioSink, ModCommandHost,
    ModPresentationAssets, ModPresentationBackend, ModPresentationCinematics, ModPresentationClock,
    ModPresentationError, ModPresentationMedia, ModPresentationOutput, ModPresentationSource, ModSceneQueries,
    ModSceneRendererOps, NextFrameCallback, PrintCallback, ViewAxisCallback, ViewOriginCallback,
};
use super::network::unified_components::write_component_owner;
use super::presentation::{SeatComponentEffect, SeatComponentOverlay, SeatEffectFrame, SeatError, SeatOverlayContext};
use super::presentation_scene::{
    BodyCustomShader, BodyCustomSkin, SceneBody, SceneBodyMaterial, SceneBodyPart, SceneBodyPartEntry,
};
use super::presentation_state::{SimulationPresentationEvent, SourcePresentationEvent};
use super::q3_client::qvm_display::QvmDisplayRenderer;
use super::q3_client::scene::Q3SceneRenderOptions;
use super::q3_client_app::Q3CommandRegistration;
use super::simulation::q3::types::Q3SourcePlayerEvent;

/// Failure of a component presentation collection operation.
#[derive(Debug)]
pub enum ModPresentationsError<E> {
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
    /// Command contract failure.
    Contract(ContractError),
    /// Command tokenize failure.
    Cmd(CmdError),
    /// Seat read failure.
    Seat(SeatError),
    /// Script mount failure.
    Mount(MountError),
}

impl<E: std::fmt::Display> std::fmt::Display for ModPresentationsError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Presentation(message) => write!(f, "{message}"),
            Self::Backend(error) => write!(f, "{error}"),
            Self::Cvar(error) => write!(f, "{error}"),
            Self::World(error) => write!(f, "{error}"),
            Self::Net(error) => write!(f, "{error}"),
            Self::Scene(error) => write!(f, "{error}"),
            Self::Contract(error) => write!(f, "{error}"),
            Self::Cmd(error) => write!(f, "{error}"),
            Self::Seat(error) => write!(f, "{error}"),
            Self::Mount(error) => write!(f, "{error}"),
        }
    }
}

impl<E: std::error::Error + 'static> std::error::Error for ModPresentationsError<E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Presentation(_) => None,
            Self::Backend(error) => Some(error),
            Self::Cvar(error) => Some(error),
            Self::World(error) => Some(error),
            Self::Net(error) => Some(error),
            Self::Scene(error) => Some(error),
            Self::Contract(error) => Some(error),
            Self::Cmd(error) => Some(error),
            Self::Seat(error) => Some(error),
            Self::Mount(error) => Some(error),
        }
    }
}

impl<E> From<ModPresentationError<E>> for ModPresentationsError<E> {
    fn from(error: ModPresentationError<E>) -> Self {
        match error {
            ModPresentationError::Presentation(message) => Self::Presentation(message),
            ModPresentationError::Backend(error) => Self::Backend(error),
            ModPresentationError::Cvar(error) => Self::Cvar(error),
            ModPresentationError::World(error) => Self::World(error),
            ModPresentationError::Net(error) => Self::Net(error),
            ModPresentationError::Scene(error) => Self::Scene(error),
        }
    }
}

impl<E> From<WorldError> for ModPresentationsError<E> {
    fn from(error: WorldError) -> Self {
        Self::World(error)
    }
}

impl<E> From<ContractError> for ModPresentationsError<E> {
    fn from(error: ContractError) -> Self {
        Self::Contract(error)
    }
}

impl<E> From<CmdError> for ModPresentationsError<E> {
    fn from(error: CmdError) -> Self {
        Self::Cmd(error)
    }
}

impl<E> From<SeatError> for ModPresentationsError<E> {
    fn from(error: SeatError) -> Self {
        Self::Seat(error)
    }
}

impl<E> From<MountError> for ModPresentationsError<E> {
    fn from(error: MountError) -> Self {
        Self::Mount(error)
    }
}

/// Component client command target (donor `ComponentClientCommandTarget`).
/// The instance is a `u64` token (donor `symbol`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentClientCommandTarget {
    /// Presenting owner.
    pub owner: PresentationOwner,
    /// Owning instance token.
    pub instance: u64,
    /// Viewing seat.
    pub seat: SeatId,
    /// Viewing actor.
    pub viewer: ActorId,
}

/// Component client command mode (donor `ComponentClientCommandRequest["mode"]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComponentClientCommandMode {
    /// Console command.
    Console,
    /// Reliable client command.
    Reliable,
}

/// Queued component client command (donor `ComponentClientCommandRequest`).
#[derive(Debug, Clone)]
pub struct ComponentClientCommandRequest {
    /// Owning consumer.
    pub consumer: ComponentClientCommandTarget,
    /// Delivery mode.
    pub mode: ComponentClientCommandMode,
    /// Command name.
    pub name: String,
    /// Command arguments.
    pub arguments: Vec<String>,
    /// Viewing seat.
    pub seat: SeatId,
    /// Invocation source.
    pub source: ModCommandSource,
}

/// Component command dispatch outcome (donor `"handled" | "retired"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComponentCommandOutcome {
    /// A live consumer handled the command.
    Handled,
    /// The consumer retired before dispatch.
    Retired,
}

/// Viewing seat surface the collection mounts consumers into (donor
/// `ViewingSeat`): identity and camera reads plus effect binding. The host
/// implements this over its seat presentation; binding returns the donor
/// unbind closure directly.
pub trait ModCollectionSeat {
    /// Presenting seat.
    fn seat_id(&self) -> SeatId;
    /// Presenting client.
    fn client_id(&self) -> ClientId;
    /// Player actor.
    fn actor(&self) -> ActorId;
    /// Seat camera (donor `camera()`).
    fn camera(&self) -> Result<SceneCamera, SeatError>;
    /// Seat viewport (donor `viewport`).
    fn viewport(&self) -> Result<ViewRect, SeatError>;
    /// Whether the seat shares the screen (donor `splitScreen`).
    fn split_screen(&self) -> bool;
    /// Bind a component effect frame, returning its unbind closure (donor
    /// `bindComponentEffects`).
    fn bind_component_effects(
        &self,
        frame: SeatComponentEffect,
        overlay: Option<SeatComponentOverlay>,
    ) -> Box<dyn FnOnce()>;
}

/// Entry liveness check (donor `current`).
type CheckCurrent = Rc<dyn Fn() -> Result<(), String>>;
/// Command services access (donor `input`).
type CheckInput = Rc<dyn Fn() -> Result<ModCollectionInput, String>>;
/// Component command queue (donor `queue`).
type QueueRequest = Rc<dyn Fn(ComponentClientCommandMode, &[String], &ModCommandSource) -> Result<(), String>>;
/// Console queue (donor `queue("console", ...)`).
type QueueConsole = Rc<dyn Fn(&[String], &ModCommandSource) -> Result<(), String>>;
/// Script append (donor `append`).
type AppendCommand = Rc<dyn Fn(&str) -> Result<(), String>>;
/// Producer cvars callback (donor `cvars`).
pub type ModBindingCvars = Rc<dyn Fn(&ModCommandSource) -> Result<ModCommandCvars, String>>;
/// Producer script callback (donor `readScript`).
pub type ModBindingReadScript = Rc<dyn Fn(&str, &ModCommandSource) -> Result<Option<String>, String>>;
/// Producer command callback (donor `command`).
pub type ModBindingCommand = Rc<dyn Fn(&ModCommandInvocation) -> Result<(), String>>;
/// Engine command probe (donor `engineCommand`).
pub type ModBindingEngineCommand = Rc<dyn Fn(&str) -> bool>;
/// Entry checkpoint (donor `create` checkpoint argument).
type EntryCheckpoint<'a> = Option<(&'a SaveJson, &'a dyn Fn(&SavedActorId) -> Option<ActorId>)>;

/// Producer binding installed on the client command services (donor
/// `bindProducer` payload).
#[derive(Clone)]
pub struct ModProducerBinding {
    /// Component cvars for a caller (donor `cvars`).
    pub cvars: ModBindingCvars,
    /// Component script read (donor `readScript`; sync collapse).
    pub read_script: ModBindingReadScript,
    /// Component command handler (donor `command`).
    pub command: ModBindingCommand,
    /// Engine command probe (donor `engineCommand`).
    pub engine_command: ModBindingEngineCommand,
}

/// Client command services surface (donor `input.commands`).
pub trait ModCollectionCommands {
    /// Queue text behind the pending program in the Q3 dialect (donor
    /// `append(text, source, "q3")`).
    fn append(&self, text: &str, source: &ModCommandSource) -> Result<(), ContractError>;
    /// Bind a producer, returning its release closure (donor
    /// `bindProducer`).
    fn bind_producer(&self, instance: u64, binding: ModProducerBinding) -> Box<dyn FnOnce()>;
    /// Discard one producer's queued text (donor `discardProducer`).
    fn discard_producer(&self, instance: u64);
    /// Whether a producer holds queued commands (donor `producerPending`).
    fn producer_pending(&self, instance: u64) -> bool;
}

/// Client command registration request (donor
/// `clientCommandRegistration` payload).
#[derive(Clone)]
pub struct ModClientRegistrationParams {
    /// Owning instance token.
    pub instance: u64,
    /// Diagnostic label (donor `prepared.source.id`).
    pub label: String,
    /// Registered command handler (donor `execute`).
    pub execute: ModBindingCommand,
}

/// Client command services (donor `ApplicationModPresentationsOptions["input"]`).
#[derive(Clone)]
pub struct ModCollectionInput {
    /// Client command buffer.
    pub commands: Rc<dyn ModCollectionCommands>,
    /// Client command registration factory.
    pub client_command_registration: Rc<dyn Fn(SeatId, ModClientRegistrationParams) -> Rc<dyn Q3CommandRegistration>>,
    /// Engine command names a component may emit. The donor computes this
    /// from the command dialect and the asset content
    /// (`componentEngineCommands`); the host precomputes it because the
    /// dialect and content live outside the collection.
    pub engine_commands: HashSet<String>,
}

/// Queued command sink (donor `queueCommand`).
pub type ModQueueCommand = Rc<dyn Fn(ComponentClientCommandRequest)>;

/// Component media delivery (donor `presentationMedia(source, request,
/// initializing, current)`; sync collapse).
pub type ModCollectionMedia = Rc<
    dyn Fn(
        Rc<dyn ModPresentationSource>,
        &ComponentPresentationMediaRequest,
        bool,
        &dyn Fn() -> bool,
    ) -> Result<(), String>,
>;

/// Cinematics scope append (donor `scope.append`).
#[derive(Clone)]
pub struct ModScopeAppend {
    /// Command services access (donor `input`, checked lazily per call).
    input: CheckInput,
    /// Base command context.
    context: ModCommandSource,
    /// Producer module artifact path (donor script name).
    module_path: String,
    /// Live entry command source.
    command_source: Rc<RefCell<ModCommandSource>>,
}

impl ModScopeAppend {
    /// Queue script text on behalf of the current caller.
    pub fn append(&self, text: &str) -> Result<(), String> {
        let host = (self.input)()?;
        let caller = self.command_source.borrow().clone();
        host.commands
            .append(
                text,
                &ModCommandSource {
                    session: self.context.session.clone(),
                    origin: CommandOrigin::Script {
                        name: self.module_path.clone(),
                        caller: Box::new(caller.origin),
                    },
                    producer: self.context.producer.clone(),
                },
            )
            .map_err(|error| error.to_string())
    }
}

/// System cinematics factory (donor `systemCinematics(source, presentation,
/// scope)`; the scope `cvars`/`mounts` arrive as the factory arguments and
/// the fallible result preserves the donor throw).
pub type ModCollectionCinematics<B, S> = Rc<
    dyn Fn(
        Rc<dyn ModPresentationSource>,
        Rc<RefCell<S>>,
        &CvarRegistry,
        &MountedContent,
        &ModScopeAppend,
    ) -> Result<<B as ModPresentationBackend>::Cinematics, String>,
>;

/// Host HUD submission sink (donor `drawQ3Overlay` call). The seat overlay
/// pass carries no frame builder, so the host draws through its own frame
/// handle.
pub trait ModHudDraw {
    /// Draw owned HUD submissions for an overlay context.
    fn draw_hud(&self, submissions: &[super::mod_presentation::ModPresentationHudSubmission], ctx: &SeatOverlayContext);
}

/// Host foreign-payload projection for q3-source player events (donor
/// `event.event` with `kind: "player-event"`). The source-event enum carries
/// no Q3 variant, so q3-source events arrive as `Foreign` payloads and the
/// host projects the player events out; anything else reports `None`.
pub trait ModQ3EventPayload {
    /// Borrow the q3-source player event, if this payload carries one.
    fn q3_player_event(&self) -> Option<&Q3SourcePlayerEvent>;
}

/// Creation options (donor `ApplicationModPresentationsOptions`).
pub struct ApplicationModPresentationsOptions<B: ModPresentationBackend, S> {
    /// Asset inputs.
    pub assets: Rc<dyn ModPresentationAssets>,
    /// Cgame audio sink.
    pub audio: Rc<dyn ModAudioSink>,
    /// Scene queries.
    pub queries: Rc<dyn ModSceneQueries>,
    /// Print sink.
    pub print: PrintCallback,
    /// Next-frame pump.
    pub next_frame: NextFrameCallback,
    /// Presentation clock.
    pub clock: Rc<dyn ModPresentationClock>,
    /// Display renderer, when display traps are served.
    pub renderer: Option<Rc<QvmDisplayRenderer>>,
    /// Collection session (donor `seat.session`; the seat token is hidden
    /// so the host supplies it).
    pub session: SessionId,
    /// Client command services, when commands are served.
    pub input: Option<ModCollectionInput>,
    /// Component media delivery, when media is served.
    pub presentation_media: Option<ModCollectionMedia>,
    /// Queued command sink.
    pub queue_command: Option<ModQueueCommand>,
    /// System cinematics factory, when cinematics are served.
    pub system_cinematics: Option<ModCollectionCinematics<B, S>>,
    /// Host HUD submission sink.
    pub hud_sink: Rc<dyn ModHudDraw>,
    /// Free-memory sampler in bytes (donor `freemem`, forwarded to consumers).
    pub free_memory: FreeMemoryCallback,
}

/// One tracked consumer (donor `Entry`).
struct CollectionEntry<B: ModPresentationBackend> {
    /// Live cgame presentation, shared with the seat effect callbacks.
    consumer: Rc<RefCell<ApplicationModPresentation<B>>>,
    /// Component client command target.
    target: ComponentClientCommandTarget,
    /// Live invocation source, swapped during dispatch.
    command_source: Rc<RefCell<ModCommandSource>>,
    /// Producer release closure, while bound.
    release_producer: Option<Box<dyn FnOnce()>>,
    /// Client command registration, once created.
    registration: Rc<RefCell<Option<Rc<dyn Q3CommandRegistration>>>>,
    /// Whether the command services are closed.
    commands_closed: Rc<Cell<bool>>,
    /// Bound source generation.
    generation: u64,
    /// Effect unbind closure, once published.
    unbind: Option<Box<dyn FnOnce()>>,
    /// Whether the entry published.
    published: bool,
    /// Captured scene, shared with the seat effect callback.
    scene: Rc<RefCell<Option<Q3SceneContent>>>,
    /// Captured time, shared with the seat effect callback.
    time: Rc<Cell<f64>>,
    /// Seat camera failure recorded by the guest view callbacks, reported by
    /// the next driven frame.
    view_error: Rc<RefCell<Option<SeatError>>>,
}

/// Entries mounted into one seat (donor `entries` row).
struct SeatEntries<B: ModPresentationBackend, S> {
    /// Viewing seat handle (donor map key; identity by handle).
    presentation: Rc<RefCell<S>>,
    /// Consumers by prepared module provider.
    providers: Vec<(ProviderId, CollectionEntry<B>)>,
}

/// Collection of component presentations (donor
/// `ApplicationModPresentations`).
pub struct ApplicationModPresentations<B: ModPresentationBackend, S> {
    /// Entries by seat.
    seats: Vec<SeatEntries<B, S>>,
    /// Whether the collection closed.
    closed: Rc<Cell<bool>>,
    /// Whether a preparation or restore is running.
    busy: bool,
    /// Whether restored entries still need publishing.
    unpublished: bool,
    /// Next producer instance token.
    next_instance: Cell<u64>,
    /// Creation options.
    options: ApplicationModPresentationsOptions<B, S>,
    /// Guest backend handle shared with every consumer.
    backend: Rc<RefCell<B>>,
}

impl<B: ModPresentationBackend, S> ApplicationModPresentations<B, S> {
    /// Create the collection (donor `new ApplicationModPresentations(options)`).
    /// The guest backend arrives separately because the host owns it next
    /// to the collection.
    pub fn new(options: ApplicationModPresentationsOptions<B, S>, backend: Rc<RefCell<B>>) -> Self {
        Self {
            seats: Vec::new(),
            closed: Rc::new(Cell::new(false)),
            busy: false,
            unpublished: false,
            next_instance: Cell::new(0),
            options,
            backend,
        }
    }

    /// Fail once the collection closes (donor `assertOpen`).
    fn assert_open(&self) -> Result<(), ModPresentationsError<B::Error>> {
        if self.closed.get() {
            return Err(ModPresentationsError::Presentation(
                "Component presentation collection is closed".to_string(),
            ));
        }
        Ok(())
    }

    /// Mint a producer instance token.
    fn mint_instance(&self) -> u64 {
        let instance = self.next_instance.get();
        self.next_instance.set(instance + 1);
        instance
    }
}

/// Write a module identity checkpoint value.
fn write_module_identity(module: &ModuleIdentity) -> SaveJson {
    obj(vec![
        ("id", save_str(&format!("{}:{}", module.id.namespace, module.id.name))),
        ("artifactPath", save_str(&module.artifact_path)),
        ("digest", save_str(module.digest.as_str())),
        ("revision", save_str(&module.revision)),
    ])
}

/// Parse a checkpoint provider reference (donor `owner.provider`).
fn parse_provider(reader: SaveReader, text: &str) -> Result<ProviderId, WorldError> {
    let (namespace, name) = text.split_once(':').unwrap_or(("", ""));
    if namespace.is_empty() || name.is_empty() {
        return Err(reader.fail("invalid provider reference"));
    }
    Ok(ProviderId::new(namespace, name))
}

/// Whether a guest ABI matches an event ABI profile.
fn abi_matches(abi: QvmAbi, profile: QvmAbiProfile) -> bool {
    matches!(
        (abi, profile),
        (QvmAbi::Modern, QvmAbiProfile::Modern) | (QvmAbi::Legacy, QvmAbiProfile::Legacy116n)
    )
}

/// Q1 fog parameters for an optional scene fog.
fn q1_fog_params(fog: Option<&SceneFog>) -> Option<qa_client::render::scene::world::Q1FogParams> {
    match fog {
        Some(SceneFog::Q1 {
            color,
            density,
            sky_factor,
        }) => Some(qa_client::render::scene::world::Q1FogParams {
            color: *color,
            density: *density,
            sky_factor: *sky_factor,
        }),
        _ => None,
    }
}

/// Convert captured component bodies to seat scene bodies.
fn scene_bodies(bodies: &[ComponentBody]) -> Vec<SceneBody> {
    bodies
        .iter()
        .map(|body| SceneBody {
            owner: body.owner.clone(),
            actor: body.actor.clone(),
            content: body.content.clone(),
            time_ms: body.time.trunc() as i32,
            parts: body
                .parts
                .iter()
                .map(|part| SceneBodyPartEntry {
                    part: match part.part {
                        qa_content::contract::QvmBodyPart::Body => SceneBodyPart::Body,
                        qa_content::contract::QvmBodyPart::Lower => SceneBodyPart::Lower,
                        qa_content::contract::QvmBodyPart::Upper => SceneBodyPart::Upper,
                        qa_content::contract::QvmBodyPart::Head => SceneBodyPart::Head,
                    },
                    base: part.base,
                    passes: part
                        .passes
                        .iter()
                        .map(|pass| SceneBodyMaterial {
                            custom_shader: pass.custom_shader.as_ref().map(|shader| BodyCustomShader {
                                name: shader.name.clone(),
                            }),
                            custom_skin: pass.custom_skin.as_ref().map(|skin| BodyCustomSkin {
                                surfaces: skin
                                    .surfaces
                                    .iter()
                                    .map(|surface| CustomSkinEntry {
                                        name: surface.name.clone(),
                                        shader: surface.shader.clone(),
                                    })
                                    .collect(),
                            }),
                            shader_rgba: pass.shader_rgba,
                            shader_tex_coord: pass.shader_tex_coord,
                            shader_time: f64::from(pass.shader_time),
                            render_flags: pass.render_flags,
                            lighting_origin: pass.lighting_origin,
                            shadow_plane: pass.shadow_plane,
                            non_normalized_axes: pass.non_normalized_axes,
                        })
                        .collect(),
                })
                .collect(),
        })
        .collect()
}

/// Consumer command host wiring one entry to the client command services
/// (donor `commands` in `create`).
struct EntryCommandHost {
    /// Entry liveness check (donor `current`).
    current: CheckCurrent,
    /// Command services access (donor `input`).
    input: CheckInput,
    /// Client command registration, once created.
    registration: Rc<RefCell<Option<Rc<dyn Q3CommandRegistration>>>>,
    /// Viewing seat.
    seat: SeatId,
    /// Owning instance token.
    instance: u64,
    /// Registration label.
    label: String,
    /// Console queue (donor `queue("console", ...)`).
    queue_console: QueueConsole,
    /// Script append (donor `append`).
    append: AppendCommand,
    /// Reliable queue (donor `reliable`).
    reliable: AppendCommand,
}

impl ModCommandHost for EntryCommandHost {
    fn append_command(&self, text: &str) -> Result<(), String> {
        (self.append)(text)
    }

    fn register_command(&self, name: &str) -> Result<(), String> {
        let host = (self.input)()?;
        let mut registration = self.registration.borrow_mut();
        if registration.is_none() {
            let queue_console = Rc::clone(&self.queue_console);
            *registration = Some((host.client_command_registration)(
                self.seat.clone(),
                ModClientRegistrationParams {
                    instance: self.instance,
                    label: self.label.clone(),
                    execute: Rc::new(move |command| queue_console(&command.argv, &command.source)),
                },
            ));
        }
        if let Some(registration) = registration.as_ref() {
            registration.register(name);
        }
        Ok(())
    }

    fn remove_command(&self, name: &str) -> Result<(), String> {
        (self.current)()?;
        if let Some(registration) = self.registration.borrow().as_ref() {
            registration.remove(name);
        }
        Ok(())
    }

    fn reliable_command(&self, text: &str) -> Result<(), String> {
        (self.reliable)(text)
    }
}

/// Presentation output rejecting every destination (donor `output` in
/// `create`: the collection serves no destination view or overlay takeover).
struct RejectOutput;

impl ModPresentationOutput for RejectOutput {
    fn submit_scene(&self, _: qa_content::q3::presentation::scene::Q3PresentedScene) -> Result<(), String> {
        Err("Component cgame requires an unsupported destination view or overlay takeover".to_string())
    }

    fn submit_command(&self, _: qa_client::render::types::RenderCommand) -> Result<(), String> {
        Err("Component cgame requires an unsupported destination view or overlay takeover".to_string())
    }

    fn submit_text(&self, _: super::q3_client::services::Q3ServiceTextDraw) -> Result<(), String> {
        Err("Component cgame requires an unsupported destination view or overlay takeover".to_string())
    }

    fn submit_listener(&self, _: Vec3, _: qa_core::math::Axis) -> Result<(), String> {
        Err("Component cgame requires an unsupported destination view or overlay takeover".to_string())
    }
}

impl<B: ModPresentationBackend + 'static, S: ModCollectionSeat + 'static> ApplicationModPresentations<B, S>
where
    B::Renderer: ModSceneRendererOps<B::Error>,
{
    /// Assert one entry still owns its source and viewer (donor `current`
    /// in `create`).
    fn check_current(
        closed: &Cell<bool>,
        source: &dyn ModPresentationSource,
        generation: u64,
        viewer: &ActorId,
        commands_closed: &Cell<bool>,
        presentation: &RefCell<S>,
    ) -> Result<(), String> {
        if closed.get() {
            return Err("Component presentation collection is closed".to_string());
        }
        source.assert_current()?;
        if commands_closed.get()
            || source.generation() != generation
            || !source.live(viewer)
            || presentation.borrow().actor() != *viewer
        {
            return Err("Component client command owner is retired".to_string());
        }
        Ok(())
    }

    /// Access one entry's command services (donor `input` in `create`).
    #[allow(clippy::too_many_arguments)]
    fn check_input(
        input: &Option<ModCollectionInput>,
        has_queue: bool,
        closed: &Cell<bool>,
        source: &dyn ModPresentationSource,
        generation: u64,
        viewer: &ActorId,
        commands_closed: &Cell<bool>,
        presentation: &RefCell<S>,
    ) -> Result<ModCollectionInput, String> {
        Self::check_current(closed, source, generation, viewer, commands_closed, presentation)?;
        match input.clone() {
            Some(input) if has_queue => Ok(input),
            _ => Err("Component cgame requires actual client command services".to_string()),
        }
    }

    /// Close one entry's command services (donor `closeCommands`). The
    /// release, registration, and discard calls are all infallible in Rust,
    /// so unlike the donor this cannot fail.
    fn close_entry_commands(
        input: &Option<ModCollectionInput>,
        registration: &Rc<RefCell<Option<Rc<dyn Q3CommandRegistration>>>>,
        release_producer: &mut Option<Box<dyn FnOnce()>>,
        commands_closed: &Rc<Cell<bool>>,
        instance: u64,
    ) {
        if commands_closed.get() {
            return;
        }
        commands_closed.set(true);
        if let Some(release) = release_producer.take() {
            release();
        }
        if let Some(registration) = registration.borrow_mut().take() {
            registration.close();
        }
        if let Some(input) = input.as_ref() {
            input.commands.discard_producer(instance);
        }
    }

    /// Create one consumer entry (donor `create`), returning its prepared
    /// module provider key. The caller inserts the entry; when no
    /// checkpoint restores, the entry publishes before returning.
    fn create_entry(
        &self,
        presentation: &Rc<RefCell<S>>,
        source: &Rc<dyn ModPresentationSource>,
        checkpoint: EntryCheckpoint<'_>,
    ) -> Result<(ProviderId, CollectionEntry<B>), ModPresentationsError<B::Error>> {
        let seat = presentation.borrow().seat_id();
        let client = presentation.borrow().client_id();
        let viewer = presentation.borrow().actor();
        let module = source.prepared_contract_module();
        let target = ComponentClientCommandTarget {
            owner: source.presentation_owner(),
            instance: self.mint_instance(),
            seat: seat.clone(),
            viewer: viewer.clone(),
        };
        let producer = ModCommandProducer {
            module: module.clone(),
            instance: target.instance,
        };
        let context = ModCommandSource {
            session: self.options.session.clone(),
            origin: CommandOrigin::LocalSeat {
                seat: seat.clone(),
                client,
            },
            producer: Some(producer.clone()),
        };
        let generation = source.generation();
        let commands_closed = Rc::new(Cell::new(false));
        let registration: Rc<RefCell<Option<Rc<dyn Q3CommandRegistration>>>> = Rc::new(RefCell::new(None));
        let command_source = Rc::new(RefCell::new(context.clone()));
        let view_error: Rc<RefCell<Option<SeatError>>> = Rc::new(RefCell::new(None));

        let current_closed = Rc::clone(&self.closed);
        let current_source = Rc::clone(source);
        let current_seat = Rc::clone(presentation);
        let current_viewer = viewer.clone();
        let current_commands_closed = Rc::clone(&commands_closed);
        let current: CheckCurrent = Rc::new(move || {
            Self::check_current(
                &current_closed,
                current_source.as_ref(),
                generation,
                &current_viewer,
                &current_commands_closed,
                &current_seat,
            )
        });

        let input_options = self.options.input.clone();
        let has_queue = self.options.queue_command.is_some();
        let input_closed = Rc::clone(&self.closed);
        let input_source = Rc::clone(source);
        let input_viewer = viewer.clone();
        let input_commands_closed = Rc::clone(&commands_closed);
        let input_seat = Rc::clone(presentation);
        let input: CheckInput = Rc::new(move || {
            Self::check_input(
                &input_options,
                has_queue,
                &input_closed,
                input_source.as_ref(),
                generation,
                &input_viewer,
                &input_commands_closed,
                &input_seat,
            )
        });

        let queue_command = self.options.queue_command.clone();
        let queue_target = target.clone();
        let queue_producer = producer.clone();
        let queue_input = Rc::clone(&input);
        let queue: QueueRequest = Rc::new(move |mode, arguments, caller| {
            queue_input()?;
            let Some(name) = arguments.first() else {
                return Ok(());
            };
            if let Some(queue_command) = queue_command.as_ref() {
                queue_command(ComponentClientCommandRequest {
                    consumer: queue_target.clone(),
                    mode,
                    name: name.clone(),
                    arguments: arguments[1..].to_vec(),
                    seat: queue_target.seat.clone(),
                    source: ModCommandSource {
                        session: caller.session.clone(),
                        origin: caller.origin.clone(),
                        producer: Some(queue_producer.clone()),
                    },
                });
            }
            Ok(())
        });

        let append_input = Rc::clone(&input);
        let append_context = context.clone();
        let append_module = module.artifact_path.clone();
        let append_command_source = Rc::clone(&command_source);
        let append: AppendCommand = Rc::new(move |text| {
            let host = append_input()?;
            let caller = append_command_source.borrow().clone();
            host.commands
                .append(
                    text,
                    &ModCommandSource {
                        session: append_context.session.clone(),
                        origin: CommandOrigin::Script {
                            name: append_module.clone(),
                            caller: Box::new(caller.origin),
                        },
                        producer: append_context.producer.clone(),
                    },
                )
                .map_err(|error| error.to_string())
        });

        let view_seat = Rc::clone(presentation);
        let view_origin_error = Rc::clone(&view_error);
        let view_origin: ViewOriginCallback = Rc::new(move || match view_seat.borrow().camera() {
            Ok(camera) => camera.origin,
            Err(error) => {
                *view_origin_error.borrow_mut() = Some(error);
                Vec3 { x: 0.0, y: 0.0, z: 0.0 }
            }
        });
        let axis_seat = Rc::clone(presentation);
        let axis_error = Rc::clone(&view_error);
        let view_axis: ViewAxisCallback = Rc::new(move || match axis_seat.borrow().camera() {
            Ok(camera) => camera.axis,
            Err(error) => {
                *axis_error.borrow_mut() = Some(error);
                [
                    Vec3 { x: 1.0, y: 0.0, z: 0.0 },
                    Vec3 { x: 0.0, y: 1.0, z: 0.0 },
                    Vec3 { x: 0.0, y: 0.0, z: 1.0 },
                ]
            }
        });

        let viewport = presentation.borrow().viewport()?;
        let system_cinematics = self.options.system_cinematics.clone().map(|factory| {
            let factory_source = Rc::clone(source);
            let factory_seat = Rc::clone(presentation);
            let scope = ModScopeAppend {
                input: Rc::clone(&input),
                context: context.clone(),
                module_path: module.artifact_path.clone(),
                command_source: Rc::clone(&command_source),
            };
            ModPresentationCinematics::Factory(Rc::new(move |cvars, mounts| {
                factory(
                    Rc::clone(&factory_source),
                    Rc::clone(&factory_seat),
                    cvars,
                    mounts,
                    &scope,
                )
            }))
        });
        let presentation_media = self.options.presentation_media.clone().map(|media| {
            let media_source = Rc::clone(source);
            let media_current = Rc::clone(&current);
            let media_closed = Rc::clone(&self.closed);
            let media_commands_closed = Rc::clone(&commands_closed);
            let media_seat = Rc::clone(presentation);
            let media_viewer = viewer.clone();
            let wrapped: ModPresentationMedia = Rc::new(
                move |request: &ComponentPresentationMediaRequest,
                      initializing: bool,
                      consumer_current: &dyn Fn() -> bool| {
                    media_current()?;
                    media(Rc::clone(&media_source), request, initializing, &|| {
                        !media_closed.get()
                            && !media_commands_closed.get()
                            && media_seat.borrow().actor() == media_viewer
                            && consumer_current()
                    })?;
                    media_current()
                },
            );
            wrapped
        });

        let queue_console_inner = Rc::clone(&queue);
        let queue_console: QueueConsole = Rc::new(move |arguments, caller| {
            queue_console_inner(ComponentClientCommandMode::Console, arguments, caller)
        });
        let reliable_queue = Rc::clone(&queue);
        let reliable_source = Rc::clone(&command_source);
        let reliable: AppendCommand = Rc::new(move |text| {
            let tokens = tokenize_command(text, Dialect::Q3, TextMode::Console).map_err(|error| error.to_string())?;
            let caller = reliable_source.borrow().clone();
            reliable_queue(ComponentClientCommandMode::Reliable, &tokens.argv, &caller)
        });
        let commands = Rc::new(EntryCommandHost {
            current: Rc::clone(&current),
            input: Rc::clone(&input),
            registration: Rc::clone(&registration),
            seat,
            instance: target.instance,
            label: format!("{}:{}", module.id.namespace, module.id.name),
            queue_console,
            append,
            reliable,
        });

        let owner_options = ApplicationModPresentationOptions {
            assets: Rc::clone(&self.options.assets),
            audio: Rc::clone(&self.options.audio),
            queries: Rc::clone(&self.options.queries),
            seat: target.seat.clone(),
            viewer: target.viewer.clone(),
            viewport: ContentRect {
                x: viewport.x,
                y: viewport.y,
                width: viewport.width,
                height: viewport.height,
            },
            renderer: self.options.renderer.clone(),
            source: Rc::clone(source),
            clock: Rc::clone(&self.options.clock),
            output: Rc::new(RejectOutput),
            system_cinematics,
            commands: Some(commands),
            presentation_media,
            print: Rc::clone(&self.options.print),
            view_origin,
            view_axis: Some(view_axis),
            next_frame: Rc::clone(&self.options.next_frame),
            scalar: None,
            free_memory: Rc::clone(&self.options.free_memory),
        };
        let mut release_producer: Option<Box<dyn FnOnce()>> = None;
        let mut backend = self.backend.borrow_mut();
        let consumer = match checkpoint {
            None => ApplicationModPresentation::create(owner_options, &mut *backend),
            Some((value, resolve)) => ApplicationModPresentation::restore(owner_options, &mut *backend, value, resolve),
        };
        drop(backend);
        let consumer = match consumer {
            Ok(consumer) => consumer,
            Err(error) => {
                Self::close_entry_commands(
                    &self.options.input,
                    &registration,
                    &mut release_producer,
                    &commands_closed,
                    target.instance,
                );
                return Err(error.into());
            }
        };
        self.assert_open()?;
        let mut entry = CollectionEntry {
            consumer: Rc::new(RefCell::new(consumer)),
            target: target.clone(),
            command_source,
            release_producer,
            registration,
            commands_closed,
            generation,
            unbind: None,
            published: false,
            scene: Rc::new(RefCell::new(None)),
            time: Rc::new(Cell::new(0.0)),
            view_error,
        };
        if checkpoint.is_none() {
            if let Err(error) = Self::publish_entry(
                &self.options,
                &self.backend,
                &self.closed,
                presentation,
                source,
                &module,
                &mut entry,
            ) {
                if let Some(unbind) = entry.unbind.take() {
                    unbind();
                }
                Self::close_entry_commands(
                    &self.options.input,
                    &entry.registration,
                    &mut entry.release_producer,
                    &entry.commands_closed,
                    entry.target.instance,
                );
                let mut backend = self.backend.borrow_mut();
                match entry.consumer.borrow_mut().close(&mut backend) {
                    Ok(()) => return Err(error),
                    Err(cleanup) => {
                        return Err(ModPresentationsError::Presentation(format!(
                            "Component presentation creation and cleanup failed: {error}; {cleanup}"
                        )));
                    }
                }
            }
        }
        Ok((module.id.clone(), entry))
    }

    /// Publish one entry (donor `entry.publish`): bind its scene and HUD
    /// into the seat, bind its command producer, and publish its commands.
    /// Retained closures borrow the consumer through `try_borrow` so a host
    /// reentering the collection from inside a callback degrades to an
    /// empty frame instead of panicking.
    fn publish_entry(
        options: &ApplicationModPresentationsOptions<B, S>,
        backend: &Rc<RefCell<B>>,
        closed: &Rc<Cell<bool>>,
        presentation: &Rc<RefCell<S>>,
        source: &Rc<dyn ModPresentationSource>,
        module: &ModuleIdentity,
        entry: &mut CollectionEntry<B>,
    ) -> Result<(), ModPresentationsError<B::Error>> {
        if entry.published {
            return Ok(());
        }
        Self::check_current(
            closed,
            source.as_ref(),
            entry.generation,
            &entry.target.viewer,
            &entry.commands_closed,
            presentation,
        )
        .map_err(ModPresentationsError::Presentation)?;

        let frame_consumer = Rc::clone(&entry.consumer);
        let frame_source = Rc::clone(source);
        let frame_scene = Rc::clone(&entry.scene);
        let frame_time = Rc::clone(&entry.time);
        let frame_seat = Rc::clone(presentation);
        let frame_viewer = entry.target.viewer.clone();
        let frame: SeatComponentEffect = Rc::new(move |camera, order, fog| {
            let scene_guard = frame_scene.borrow();
            let Some(scene) = scene_guard.as_ref() else {
                return SeatEffectFrame::empty();
            };
            let Ok(mut consumer) = frame_consumer.try_borrow_mut() else {
                return SeatEffectFrame::empty();
            };
            if !consumer.owns(&frame_source, &frame_viewer) {
                return SeatEffectFrame::empty();
            }
            let Ok(first_entity) = reserve_source_entity_range(order, scene.admission.entities.len() as u32) else {
                return SeatEffectFrame::empty();
            };
            let lights: Vec<SurfaceDynamicLight> = scene
                .lights
                .iter()
                .map(|light| SurfaceDynamicLight {
                    origin: light.origin,
                    radius: light.radius,
                    color: light.color,
                    minimum: 0.0,
                })
                .collect();
            let q3_lights: Vec<DynamicLight> = scene
                .lights
                .iter()
                .take(32)
                .map(|light| DynamicLight {
                    origin: light.origin,
                    radius: light.radius,
                    color: light.color,
                    additive: light.additive,
                })
                .collect();
            let seat = frame_seat.borrow();
            let mut input = WorldViewInput::new(
                *camera,
                ViewTarget::Seat(seat.seat_id()),
                SourceTime::Milliseconds(frame_time.get()),
            );
            let admission = create_world_surface_admission(order.clone());
            input.source = Some(admission.clone());
            input.lights.clone_from(&lights);
            input.visibility.q3_lights.clone_from(&q3_lights);
            input.q1_fog = q1_fog_params(fog);
            let split_screen = seat.split_screen();
            drop(seat);
            let operations = match consumer.scene_operations(
                scene,
                &input,
                first_entity,
                &Q3SceneRenderOptions {
                    view_offset: None,
                    no_world_model: false,
                    split_screen,
                    supplemental_view_weapon: false,
                },
            ) {
                Ok(operations) => operations,
                Err(_) => return SeatEffectFrame::empty(),
            };
            SeatEffectFrame {
                q3_admissions: vec![admission],
                operations,
                lights,
                q3_lights,
            }
        });

        let bodies_consumer = Rc::clone(&entry.consumer);
        let bodies_source = Rc::clone(source);
        let bodies_viewer = entry.target.viewer.clone();
        let bodies = Rc::new(move || {
            let Ok(consumer) = bodies_consumer.try_borrow() else {
                return Vec::new();
            };
            if !consumer.owns(&bodies_source, &bodies_viewer) {
                return Vec::new();
            }
            match consumer.bodies() {
                Ok(bodies) => scene_bodies(bodies),
                Err(_) => Vec::new(),
            }
        });
        let draw_consumer = Rc::clone(&entry.consumer);
        let draw_source = Rc::clone(source);
        let draw_viewer = entry.target.viewer.clone();
        let draw_sink = Rc::clone(&options.hud_sink);
        let draw = Rc::new(move |ctx: &SeatOverlayContext| {
            let Ok(consumer) = draw_consumer.try_borrow() else {
                return;
            };
            if !consumer.owns(&draw_source, &draw_viewer) {
                return;
            }
            if let Ok(hud) = consumer.hud() {
                draw_sink.draw_hud(hud, ctx);
            }
        });
        let unbind = presentation.borrow().bind_component_effects(
            frame,
            Some(SeatComponentOverlay {
                owner: source.presentation_owner(),
                bodies: Some(bodies),
                draw,
            }),
        );
        entry.unbind = Some(unbind);

        if let Some(input) = options.input.as_ref() {
            let cvars_collection_closed = Rc::clone(closed);
            let script_closed = Rc::clone(closed);
            let command_closed = Rc::clone(closed);
            let cvars_source = Rc::clone(source);
            let cvars_seat = Rc::clone(presentation);
            let cvars_viewer = entry.target.viewer.clone();
            let cvars_generation = entry.generation;
            let cvars_closed = Rc::clone(&entry.commands_closed);
            let cvars_module = module.clone();
            let cvars_registry = entry.consumer.borrow().cvars.clone();
            let cvars_session = options.session.clone();
            let binding_cvars = Rc::new(move |caller: &ModCommandSource| {
                Self::check_current(
                    &cvars_collection_closed,
                    cvars_source.as_ref(),
                    cvars_generation,
                    &cvars_viewer,
                    &cvars_closed,
                    &cvars_seat,
                )?;
                let actual = caller.producer.as_ref().map(|producer| &producer.module);
                if actual == Some(&cvars_module) {
                    Ok(ModCommandCvars {
                        session: cvars_session.clone(),
                        registry: Rc::clone(&cvars_registry),
                    })
                } else {
                    Err("Component command producer differs from its original client module".to_string())
                }
            });

            let script_source = Rc::clone(source);
            let script_seat = Rc::clone(presentation);
            let script_viewer = entry.target.viewer.clone();
            let script_generation = entry.generation;
            let script_commands_closed = Rc::clone(&entry.commands_closed);
            let script_consumer = Rc::clone(&entry.consumer);
            let script_instance = entry.target.instance;
            let read_script = Rc::new(move |name: &str, caller: &ModCommandSource| {
                Self::check_current(
                    &script_closed,
                    script_source.as_ref(),
                    script_generation,
                    &script_viewer,
                    &script_commands_closed,
                    &script_seat,
                )?;
                let consumer = match script_consumer.try_borrow() {
                    Ok(consumer) => consumer,
                    Err(_) => return Err("Component client command owner is retired".to_string()),
                };
                let mounts = consumer.file_mounts().map_err(|error| error.to_string())?;
                let resource = mounts.open(name, |_| true).map_err(|error| error.to_string())?;
                Self::check_current(
                    &script_closed,
                    script_source.as_ref(),
                    script_generation,
                    &script_viewer,
                    &script_commands_closed,
                    &script_seat,
                )?;
                if caller
                    .producer
                    .as_ref()
                    .is_none_or(|producer| producer.instance != script_instance)
                {
                    return Err("Component script changed its producer".to_string());
                }
                Ok(resource.map(|resource| String::from_utf8_lossy(&resource.bytes).into_owned()))
            });

            let command_input = options.input.clone();
            let command_has_queue = options.queue_command.is_some();
            let command_check_source = Rc::clone(source);
            let command_generation = entry.generation;
            let command_viewer = entry.target.viewer.clone();
            let command_commands_closed = Rc::clone(&entry.commands_closed);
            let command_seat = Rc::clone(presentation);
            let command_queue = options.queue_command.clone();
            let command_target = entry.target.clone();
            let command_producer = ModCommandProducer {
                module: module.clone(),
                instance: entry.target.instance,
            };
            let command = Rc::new(move |command: &ModCommandInvocation| {
                command.assert_active().map_err(|error| error.to_string())?;
                Self::check_input(
                    &command_input,
                    command_has_queue,
                    &command_closed,
                    command_check_source.as_ref(),
                    command_generation,
                    &command_viewer,
                    &command_commands_closed,
                    &command_seat,
                )?;
                let Some(name) = command.argv.first() else {
                    return Ok(());
                };
                if let Some(queue_command) = command_queue.as_ref() {
                    queue_command(ComponentClientCommandRequest {
                        consumer: command_target.clone(),
                        mode: ComponentClientCommandMode::Console,
                        name: name.clone(),
                        arguments: command.argv[1..].to_vec(),
                        seat: command_target.seat.clone(),
                        source: ModCommandSource {
                            session: command.source.session.clone(),
                            origin: command.source.origin.clone(),
                            producer: Some(command_producer.clone()),
                        },
                    });
                }
                Ok(())
            });

            let engine_commands = input.engine_commands.clone();
            let engine_command = Rc::new(move |name: &str| engine_commands.contains(&name.to_lowercase()));
            let release = input.commands.bind_producer(
                entry.target.instance,
                ModProducerBinding {
                    cvars: binding_cvars,
                    read_script,
                    command,
                    engine_command,
                },
            );
            entry.release_producer = Some(release);
        }

        entry
            .consumer
            .borrow_mut()
            .publish_commands(&mut backend.borrow_mut())?;
        entry.published = true;
        Ok(())
    }

    /// Remove one entry (donor `remove`): unbind its effects, close its
    /// command services, and close its consumer. The unbind and command
    /// closes are infallible, so only the consumer close can fail.
    fn remove_entry(
        options: &ApplicationModPresentationsOptions<B, S>,
        backend: &Rc<RefCell<B>>,
        entry: &mut CollectionEntry<B>,
    ) -> Result<(), ModPresentationsError<B::Error>> {
        if let Some(unbind) = entry.unbind.take() {
            unbind();
        }
        Self::close_entry_commands(
            &options.input,
            &entry.registration,
            &mut entry.release_producer,
            &entry.commands_closed,
            entry.target.instance,
        );
        entry.consumer.borrow_mut().close(&mut backend.borrow_mut())?;
        Ok(())
    }

    /// Capture the collection checkpoint (donor `captureCheckpoint`).
    pub fn capture_checkpoint(
        &self,
        presentations: &[Rc<RefCell<S>>],
        sources: &[Rc<dyn ModPresentationSource>],
    ) -> Result<SaveJson, ModPresentationsError<B::Error>> {
        self.assert_open()?;
        if self.busy || self.unpublished || self.pending_commands() {
            return Err(ModPresentationsError::Presentation(
                "Component client checkpoint requires an idle published collection without queued commands".to_string(),
            ));
        }
        let mut available: Vec<(ProviderId, Rc<dyn ModPresentationSource>)> = Vec::new();
        for source in sources {
            available.push((source.prepared_contract_module().id.clone(), Rc::clone(source)));
        }
        for (index, (provider, _)) in available.iter().enumerate() {
            if available[..index].iter().any(|(other, _)| other == provider) {
                return Err(ModPresentationsError::Presentation(
                    "Duplicate component presentation source identity".to_string(),
                ));
            }
        }
        for row in &self.seats {
            if !presentations
                .iter()
                .any(|presentation| Rc::ptr_eq(presentation, &row.presentation))
            {
                return Err(ModPresentationsError::Presentation(
                    "Component checkpoint contains a retired seat".to_string(),
                ));
            }
            let viewer = row.presentation.borrow().actor();
            for (provider, entry) in &row.providers {
                let Some((_, source)) = available.iter().find(|(candidate, _)| candidate == provider) else {
                    return Err(ModPresentationsError::Presentation(
                        "Component checkpoint contains a retired source consumer".to_string(),
                    ));
                };
                if !entry.consumer.borrow().owns(source, &viewer) {
                    return Err(ModPresentationsError::Presentation(
                        "Component checkpoint contains a retired source consumer".to_string(),
                    ));
                }
            }
        }
        let backend = self.backend.borrow();
        let mut seats = Vec::new();
        for presentation in presentations {
            let seat = presentation.borrow();
            let mut rows = Vec::new();
            for source in sources {
                let state = self
                    .seats
                    .iter()
                    .find(|row| Rc::ptr_eq(&row.presentation, presentation))
                    .and_then(|row| {
                        row.providers
                            .iter()
                            .find(|(provider, _)| *provider == source.prepared_contract_module().id)
                    });
                let state = match state {
                    None => obj(vec![("kind", save_str("lazy"))]),
                    Some((_, entry)) => obj(vec![
                        ("kind", save_str("initialized")),
                        ("checkpoint", entry.consumer.borrow().capture_checkpoint(&backend)?),
                    ]),
                };
                rows.push(obj(vec![
                    ("owner", write_component_owner(&source.presentation_owner())),
                    ("identity", source.identity_checkpoint()),
                    ("module", write_module_identity(&source.prepared_contract_module())),
                    ("declaration", source.declaration_checkpoint()),
                    ("state", state),
                ]));
            }
            seats.push(obj(vec![
                ("seat", int(seat.seat_id().index() as i64)),
                (
                    "client",
                    obj(vec![
                        ("slot", int(seat.client_id().slot() as i64)),
                        ("generation", int(seat.client_id().generation() as i64)),
                    ]),
                ),
                ("viewer", write_saved_actor(SavedActorId::from(&seat.actor()))),
                ("sources", arr(rows)),
            ]));
        }
        Ok(obj(vec![("version", int(1)), ("seats", arr(seats))]))
    }

    /// Restore the collection checkpoint (donor `restoreCheckpoint`). On
    /// failure the collection closes, like the donor.
    pub fn restore_checkpoint(
        &mut self,
        value: &SaveJson,
        presentations: &[Rc<RefCell<S>>],
        sources: &[Rc<dyn ModPresentationSource>],
        resolve_actor: &dyn Fn(&SavedActorId) -> Option<ActorId>,
    ) -> Result<(), ModPresentationsError<B::Error>> {
        self.assert_open()?;
        if self.busy || self.unpublished || !self.seats.is_empty() {
            return Err(ModPresentationsError::Presentation(
                "Component client restore requires an unpublished empty collection".to_string(),
            ));
        }
        let reader = SaveReader::at(value, "component-clients");
        reader.field("version").literal_i64(1)?;
        let mut seats = HashSet::new();
        let rows: Vec<(Rc<RefCell<S>>, Vec<RestoredClient>)> = reader
            .field("seats")
            .list(|row| {
                let seat = row.field("seat").integer(0)?;
                let presentation = presentations
                    .iter()
                    .find(|presentation| presentation.borrow().seat_id().index() as i64 == seat)
                    .cloned();
                let Some(presentation) = presentation else {
                    return Err(row.fail("duplicate or missing component seat"));
                };
                if !seats.insert(seat) {
                    return Err(row.fail("duplicate or missing component seat"));
                }
                let live = presentation.borrow();
                let client = row.field("client");
                let viewer = read_saved_actor(row.field("viewer"))
                    .ok()
                    .and_then(|saved| resolve_actor(&saved));
                let live_slot = live.client_id().slot();
                let live_generation = live.client_id().generation();
                let live_actor = live.actor();
                drop(live);
                if client.field("slot").integer(0)? != live_slot as i64
                    || client.field("generation").integer(0)? != live_generation as i64
                    || viewer.as_ref() != Some(&live_actor)
                {
                    return Err(row.fail("component viewing client changed"));
                }
                let mut owners = HashSet::new();
                let clients = row.field("sources").list(|saved| {
                    let provider = saved.field("owner").field("provider").string()?;
                    let provider = parse_provider(saved.field("owner").field("provider"), &provider)?;
                    let source = sources
                        .iter()
                        .find(|source| source.presentation_owner().provider == provider)
                        .cloned();
                    let Some(source) = source else {
                        return Err(saved.fail("duplicate or missing component source"));
                    };
                    if !owners.insert(provider) {
                        return Err(saved.fail("duplicate or missing component source"));
                    }
                    let owner = write_component_owner(&source.presentation_owner());
                    let module = write_module_identity(&source.prepared_contract_module());
                    let identity = source.identity_checkpoint();
                    let declaration = source.declaration_checkpoint();
                    let owner_field = saved.field("owner").value;
                    let identity_field = saved.field("identity").value;
                    let module_field = saved.field("module").value;
                    let declaration_field = saved.field("declaration").value;
                    if owner_field != Some(&owner)
                        || identity_field != Some(&identity)
                        || module_field != Some(&module)
                        || declaration_field != Some(&declaration)
                    {
                        return Err(saved.fail("component activation or original module identity changed"));
                    }
                    let state = saved.field("state");
                    let kind = state.field("kind").choice_str(&["lazy", "initialized"])?;
                    let checkpoint = if kind == "lazy" {
                        None
                    } else {
                        match state.field("checkpoint").value {
                            Some(checkpoint) => Some(checkpoint.clone()),
                            None => return Err(state.fail("missing checkpoint field")),
                        }
                    };
                    Ok(RestoredClient { source, checkpoint })
                })?;
                if clients.len() != sources.len() {
                    return Err(row.fail("required component client state is missing"));
                }
                Ok((presentation, clients))
            })
            .map_err(ModPresentationsError::World)?;
        if rows.len() != presentations.len() {
            return Err(ModPresentationsError::World(
                reader.fail("required viewing seat state is missing"),
            ));
        }
        self.busy = true;
        self.unpublished = true;
        let result = self.restore_rows(&rows, resolve_actor);
        self.busy = false;
        if let Err(error) = result {
            let cleanup = self.close().err().map(|error| error.to_string());
            if let Some(cleanup) = cleanup {
                return Err(ModPresentationsError::Presentation(format!(
                    "Component client restore and cleanup failed: {error}; {cleanup}"
                )));
            }
            return Err(error);
        }
        Ok(())
    }

    /// Insert restored rows (donor restore loop in `restoreCheckpoint`).
    fn restore_rows(
        &mut self,
        rows: &[(Rc<RefCell<S>>, Vec<RestoredClient>)],
        resolve_actor: &dyn Fn(&SavedActorId) -> Option<ActorId>,
    ) -> Result<(), ModPresentationsError<B::Error>> {
        for (presentation, clients) in rows {
            for client in clients {
                let Some(checkpoint) = client.checkpoint.as_ref() else {
                    continue;
                };
                let (provider, entry) =
                    self.create_entry(presentation, &client.source, Some((checkpoint, resolve_actor)))?;
                self.assert_open()?;
                let found = self
                    .seats
                    .iter()
                    .position(|row| Rc::ptr_eq(&row.presentation, presentation));
                let index = match found {
                    Some(index) => index,
                    None => {
                        self.seats.push(SeatEntries {
                            presentation: Rc::clone(presentation),
                            providers: Vec::new(),
                        });
                        self.seats.len() - 1
                    }
                };
                self.seats[index].providers.push((provider, entry));
            }
        }
        Ok(())
    }

    /// Publish restored entries (donor `publishRestored`).
    pub fn publish_restored(&mut self) -> Result<(), ModPresentationsError<B::Error>> {
        self.assert_open()?;
        if self.busy {
            return Err(ModPresentationsError::Presentation(
                "Component client restore is still running".to_string(),
            ));
        }
        if !self.unpublished {
            return Ok(());
        }
        for index in 0..self.seats.len() {
            let mut providers = std::mem::take(&mut self.seats[index].providers);
            let mut failed: Option<ModPresentationsError<B::Error>> = None;
            for (_, entry) in providers.iter_mut() {
                let source = entry.consumer.borrow().options.source.clone();
                let module = source.prepared_contract_module();
                if let Err(error) = Self::publish_entry(
                    &self.options,
                    &self.backend,
                    &self.closed,
                    &self.seats[index].presentation.clone(),
                    &source,
                    &module,
                    entry,
                ) {
                    failed = Some(error);
                    break;
                }
            }
            self.seats[index].providers = providers;
            if let Some(error) = failed {
                return Err(error);
            }
        }
        self.unpublished = false;
        Ok(())
    }

    /// Whether any entry holds queued commands (donor `pendingCommands`).
    pub fn pending_commands(&self) -> bool {
        self.seats.iter().any(|row| {
            row.providers.iter().any(|(_, entry)| {
                self.options
                    .input
                    .as_ref()
                    .is_some_and(|input| input.commands.producer_pending(entry.target.instance))
            })
        })
    }

    /// Resolve the entry owning a command source (donor `commandEntry`):
    /// `None` without a component producer, the entry's target, cvars, and
    /// mounts otherwise. Entry identity compares instance tokens (donor
    /// object identity).
    fn command_entry(
        &self,
        source: &ModCommandSource,
    ) -> Result<Option<CommandEntryRef>, ModPresentationsError<B::Error>> {
        let Some(producer) = source.producer.as_ref() else {
            return Ok(None);
        };
        self.assert_open()?;
        for row in &self.seats {
            for (_, entry) in &row.providers {
                if entry.target.instance != producer.instance {
                    continue;
                }
                let consumer = entry.consumer.borrow();
                let expected = consumer.options.source.prepared_contract_module();
                if producer.module != expected || !consumer.owns(&consumer.options.source, &entry.target.viewer) {
                    return Err(ModPresentationsError::Presentation(
                        "Component client command producer is retired or differs from its source".to_string(),
                    ));
                }
                return Ok(Some(CommandEntryRef {
                    target: entry.target.clone(),
                    cvars: consumer.cvars.clone(),
                    mounts: consumer.file_mounts()?,
                }));
            }
        }
        Err(ModPresentationsError::Presentation(
            "Component client command producer is retired".to_string(),
        ))
    }

    /// Component cvars for a command source (donor `commandCvars`).
    pub fn command_cvars(
        &self,
        source: &ModCommandSource,
    ) -> Result<Option<Rc<RefCell<CvarRegistry>>>, ModPresentationsError<B::Error>> {
        Ok(self.command_entry(source)?.map(|entry| entry.cvars))
    }

    /// Read a component command script (donor `readCommandScript`).
    pub fn read_command_script(
        &self,
        name: &str,
        source: &ModCommandSource,
    ) -> Result<Option<String>, ModPresentationsError<B::Error>> {
        let Some(entry) = self.command_entry(source)? else {
            return Ok(None);
        };
        let resource = entry.mounts.open(name, |_| true)?;
        if self
            .command_entry(source)?
            .as_ref()
            .is_none_or(|later| later.target.instance != entry.target.instance)
        {
            return Err(ModPresentationsError::Presentation(
                "Component script owner changed while reading".to_string(),
            ));
        }
        Ok(resource.map(|resource| String::from_utf8_lossy(&resource.bytes).into_owned()))
    }

    /// Queue a console command for its owning entry (donor `queueConsole`).
    pub fn queue_console(&self, command: &ModCommandInvocation) -> Result<bool, ModPresentationsError<B::Error>> {
        let Some(entry) = self.command_entry(&command.source)? else {
            return Ok(false);
        };
        command.assert_active()?;
        let Some(name) = command.argv.first() else {
            return Ok(true);
        };
        let Some(queue_command) = self.options.queue_command.as_ref() else {
            return Err(ModPresentationsError::Presentation(
                "Component command queue is unavailable".to_string(),
            ));
        };
        queue_command(ComponentClientCommandRequest {
            consumer: entry.target.clone(),
            mode: ComponentClientCommandMode::Console,
            name: name.clone(),
            arguments: command.argv[1..].to_vec(),
            source: command.source.clone(),
            seat: entry.target.seat.clone(),
        });
        Ok(true)
    }

    /// Dispatch one queued command (donor `dispatchCommand`).
    pub fn dispatch_command(
        &mut self,
        request: &ComponentClientCommandRequest,
    ) -> Result<ComponentCommandOutcome, ModPresentationsError<B::Error>> {
        self.assert_open()?;
        let mut found: Option<(usize, usize)> = None;
        for (seat_index, row) in self.seats.iter().enumerate() {
            for (provider_index, (_, entry)) in row.providers.iter().enumerate() {
                if entry.target == request.consumer {
                    found = Some((seat_index, provider_index));
                    break;
                }
            }
            if found.is_some() {
                break;
            }
        }
        let Some((seat_index, provider_index)) = found else {
            return Ok(ComponentCommandOutcome::Retired);
        };
        let owned = {
            let entry = &self.seats[seat_index].providers[provider_index].1;
            let consumer = entry.consumer.borrow();
            consumer.owns(&consumer.options.source, &request.consumer.viewer)
        };
        if !owned {
            return Ok(ComponentCommandOutcome::Retired);
        }
        let previous = self.seats[seat_index].providers[provider_index]
            .1
            .command_source
            .borrow()
            .clone();
        *self.seats[seat_index].providers[provider_index]
            .1
            .command_source
            .borrow_mut() = request.source.clone();
        let result = self.dispatch_inner(seat_index, provider_index, request);
        *self.seats[seat_index].providers[provider_index]
            .1
            .command_source
            .borrow_mut() = previous;
        result
    }

    /// Run one dispatch with the request source installed (donor
    /// `dispatchCommand` body).
    fn dispatch_inner(
        &mut self,
        seat_index: usize,
        provider_index: usize,
        request: &ComponentClientCommandRequest,
    ) -> Result<ComponentCommandOutcome, ModPresentationsError<B::Error>> {
        let mut arguments = vec![request.name.clone()];
        arguments.extend(request.arguments.iter().cloned());
        if request.mode == ComponentClientCommandMode::Console {
            let handled = {
                let entry = &self.seats[seat_index].providers[provider_index].1;
                entry
                    .consumer
                    .borrow_mut()
                    .command(&mut self.backend.borrow_mut(), &arguments)?
            };
            if handled {
                return Ok(ComponentCommandOutcome::Handled);
            }
        }
        let (owned, source, viewer) = {
            let entry = &self.seats[seat_index].providers[provider_index].1;
            let consumer = entry.consumer.borrow();
            (
                consumer.owns(&consumer.options.source, &request.consumer.viewer),
                consumer.options.source.clone(),
                request.consumer.viewer.clone(),
            )
        };
        if !owned {
            return Ok(ComponentCommandOutcome::Retired);
        }
        if !source
            .client_command(&viewer, &arguments)
            .map_err(ModPresentationsError::Presentation)?
        {
            return Err(ModPresentationsError::Presentation(
                "Component source has no admitted client command receiver".to_string(),
            ));
        }
        Ok(ComponentCommandOutcome::Handled)
    }

    /// Retain entries for live seats, retiring the rest (donor
    /// `retainPresentations`).
    pub fn retain_presentations(
        &mut self,
        presentations: &[Rc<RefCell<S>>],
    ) -> Result<(), ModPresentationsError<B::Error>> {
        self.assert_open()?;
        let mut failures = Vec::new();
        let mut kept = Vec::new();
        for mut row in std::mem::take(&mut self.seats) {
            if presentations
                .iter()
                .any(|presentation| Rc::ptr_eq(presentation, &row.presentation))
            {
                kept.push(row);
                continue;
            }
            for (_, entry) in row.providers.iter_mut() {
                if let Err(error) = Self::remove_entry(&self.options, &self.backend, entry) {
                    failures.push(error.to_string());
                }
            }
        }
        self.seats = kept;
        if !failures.is_empty() {
            return Err(ModPresentationsError::Presentation(format!(
                "Component seat retirement failed: {}",
                failures.join("; ")
            )));
        }
        Ok(())
    }

    /// Prepare the collection over live seats and sources (donor `prepare`).
    pub fn prepare<F: ModQ3EventPayload>(
        &mut self,
        presentations: &[Rc<RefCell<S>>],
        sources: &[Rc<dyn ModPresentationSource>],
        events: &[SimulationPresentationEvent<F>],
        frame_sequence: i64,
    ) -> Result<(), ModPresentationsError<B::Error>> {
        self.assert_open()?;
        if self.unpublished {
            return Err(ModPresentationsError::Presentation(
                "Restored component presentations are not published".to_string(),
            ));
        }
        if self.busy {
            return Err(ModPresentationsError::Presentation(
                "Component presentation preparation is already running".to_string(),
            ));
        }
        self.busy = true;
        let result = self.prepare_inner(presentations, sources, events, frame_sequence);
        self.busy = false;
        result
    }

    /// Run one preparation (donor `prepare` body).
    fn prepare_inner<F: ModQ3EventPayload>(
        &mut self,
        presentations: &[Rc<RefCell<S>>],
        sources: &[Rc<dyn ModPresentationSource>],
        events: &[SimulationPresentationEvent<F>],
        frame_sequence: i64,
    ) -> Result<(), ModPresentationsError<B::Error>> {
        self.retain_presentations(presentations)?;
        let mut available: Vec<(ProviderId, Rc<dyn ModPresentationSource>)> = Vec::new();
        for source in sources {
            available.push((source.prepared_contract_module().id.clone(), Rc::clone(source)));
        }
        for (index, (provider, _)) in available.iter().enumerate() {
            if available[..index].iter().any(|(other, _)| other == provider) {
                return Err(ModPresentationsError::Presentation(
                    "Duplicate component presentation source identity".to_string(),
                ));
            }
        }
        let mut seat_index = 0;
        while seat_index < self.seats.len() {
            let mut provider_index = 0;
            while provider_index < self.seats[seat_index].providers.len() {
                let stale = {
                    let row = &self.seats[seat_index];
                    let (provider, entry) = &row.providers[provider_index];
                    let viewer = row.presentation.borrow().actor();
                    match available.iter().find(|(candidate, _)| candidate == provider) {
                        None => true,
                        Some((_, source)) => !entry.consumer.borrow().owns(source, &viewer),
                    }
                };
                if stale {
                    let mut entry = self.seats[seat_index].providers.remove(provider_index).1;
                    Self::remove_entry(&self.options, &self.backend, &mut entry)?;
                } else {
                    provider_index += 1;
                }
            }
            if self.seats[seat_index].providers.is_empty() {
                self.seats.remove(seat_index);
            } else {
                seat_index += 1;
            }
        }
        for presentation in presentations {
            for source in sources {
                self.assert_open()?;
                let viewer = presentation.borrow().actor();
                let provider = source.prepared_contract_module().id.clone();
                let found = self
                    .seats
                    .iter()
                    .position(|row| Rc::ptr_eq(&row.presentation, presentation))
                    .and_then(|seat_index| {
                        self.seats[seat_index]
                            .providers
                            .iter()
                            .position(|(candidate, _)| *candidate == provider)
                            .map(|provider_index| (seat_index, provider_index))
                    });
                if source.scene_context(&viewer).is_none() {
                    if let Some((seat_index, provider_index)) = found {
                        let mut entry = self.seats[seat_index].providers.remove(provider_index).1;
                        Self::remove_entry(&self.options, &self.backend, &mut entry)?;
                    }
                    continue;
                }
                let (seat_index, provider_index) = match found {
                    Some(found) => found,
                    None => {
                        let (provider, entry) = self.create_entry(presentation, source, None)?;
                        let seat_row = self
                            .seats
                            .iter()
                            .position(|row| Rc::ptr_eq(&row.presentation, presentation));
                        let seat_index = match seat_row {
                            Some(seat_index) => seat_index,
                            None => {
                                self.seats.push(SeatEntries {
                                    presentation: Rc::clone(presentation),
                                    providers: Vec::new(),
                                });
                                self.seats.len() - 1
                            }
                        };
                        self.seats[seat_index].providers.push((provider, entry));
                        (seat_index, self.seats[seat_index].providers.len() - 1)
                    }
                };
                if let Err(error) = self.prepare_entry(seat_index, provider_index, source, events, frame_sequence) {
                    let mut entry = self.seats[seat_index].providers.remove(provider_index).1;
                    match Self::remove_entry(&self.options, &self.backend, &mut entry) {
                        Ok(()) => return Err(error),
                        Err(cleanup) => {
                            return Err(ModPresentationsError::Presentation(format!(
                                "Component presentation preparation and cleanup failed: {error}; {cleanup}"
                            )));
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Consume admitted events and frame one entry (donor per-entry block
    /// in `prepare`).
    fn prepare_entry<F: ModQ3EventPayload>(
        &mut self,
        seat_index: usize,
        provider_index: usize,
        source: &Rc<dyn ModPresentationSource>,
        events: &[SimulationPresentationEvent<F>],
        frame_sequence: i64,
    ) -> Result<(), ModPresentationsError<B::Error>> {
        let viewer = self.seats[seat_index].presentation.borrow().actor();
        if matches!(source.declaration(), QvmModPresentationDeclaration::PlayerEvents(_)) {
            for event in events {
                let SourcePresentationEvent::Foreign { kind, payload, .. } = &event.source else {
                    continue;
                };
                if kind != "q3-source" {
                    continue;
                }
                if event.recipient.as_ref().is_some_and(|recipient| *recipient != viewer) {
                    continue;
                }
                let Some(player_event) = payload.q3_player_event() else {
                    continue;
                };
                if player_event.source.module != source.prepared_contract_module()
                    || !abi_matches(source.source_abi(), player_event.source.abi_profile)
                {
                    continue;
                }
                self.seats[seat_index].providers[provider_index]
                    .1
                    .consumer
                    .borrow_mut()
                    .consume(&mut self.backend.borrow_mut(), player_event, event.sequence)?;
                self.assert_open()?;
                self.check_view_error(seat_index, provider_index)?;
            }
        }
        let scene = self.seats[seat_index].providers[provider_index]
            .1
            .consumer
            .borrow_mut()
            .frame(&mut self.backend.borrow_mut(), frame_sequence)?;
        self.assert_open()?;
        let time = self.seats[seat_index].providers[provider_index]
            .1
            .consumer
            .borrow_mut()
            .time()?;
        *self.seats[seat_index].providers[provider_index].1.scene.borrow_mut() = Some(scene);
        self.seats[seat_index].providers[provider_index].1.time.set(time);
        self.check_view_error(seat_index, provider_index)?;
        Ok(())
    }

    /// Report a seat camera failure recorded while driving an entry.
    fn check_view_error(
        &self,
        seat_index: usize,
        provider_index: usize,
    ) -> Result<(), ModPresentationsError<B::Error>> {
        if let Some(error) = self.seats[seat_index].providers[provider_index]
            .1
            .view_error
            .borrow_mut()
            .take()
        {
            return Err(ModPresentationsError::Seat(error));
        }
        Ok(())
    }

    /// Close the collection (donor `close`).
    pub fn close(&mut self) -> Result<(), ModPresentationsError<B::Error>> {
        if self.closed.get() {
            return Ok(());
        }
        self.closed.set(true);
        let mut failures = Vec::new();
        for mut row in std::mem::take(&mut self.seats) {
            for (_, entry) in row.providers.iter_mut() {
                if let Err(error) = Self::remove_entry(&self.options, &self.backend, entry) {
                    failures.push(error.to_string());
                }
            }
        }
        if !failures.is_empty() {
            return Err(ModPresentationsError::Presentation(format!(
                "Component presentation collection cleanup failed: {}",
                failures.join("; ")
            )));
        }
        Ok(())
    }
}

/// Entry data resolved for a command source (donor `commandEntry` result).
struct CommandEntryRef {
    /// Owning target.
    target: ComponentClientCommandTarget,
    /// Component cvars.
    cvars: Rc<RefCell<CvarRegistry>>,
    /// Component file mounts.
    mounts: Rc<MountedContent>,
}

/// One restored source client (donor restore row client).
struct RestoredClient {
    /// Live source.
    source: Rc<dyn ModPresentationSource>,
    /// Checkpoint value, unless lazy.
    checkpoint: Option<SaveJson>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    use qa_client::render::scene::submissions::create_source_scene_order;
    use qa_client::view::{CameraClip, Rect as ViewRect};
    use qa_content::contract::{ContentDigest, ContentId, MountPlanId, QvmBodyPart, ResolvedMountPlan};
    use qa_content::mounts::{open_mount_plan, OpenMountOptions};
    use qa_content::q3::presentation::ref_entity::{SceneShader, SceneSkin, SkinMapping};
    use qa_content::q3::presentation::scene::{snapshot_q3_scene_admission, SceneAdmissionOrigin};
    use qa_core::identity::IdentityOwner;
    use qa_core::math::{identity_mat4, Bounds, Vec2, Vec4};
    use qa_guest::qvm::mod_presentation::{
        HudMode, PlayerEventCentities, PlayerEventStorage, PresentationHud, QvmPlayerEventPresentation,
        QvmPresentationCall, QvmPresentationProgram,
    };
    use qa_guest::qvm::mod_provider::{ModuleId, QvmArtifact, QvmImage, QvmRole};
    use qa_guest::qvm::player_record::QvmPlayerState;
    use qa_net::q3_net::{Q3EntityState, Q3PlayerState, Q3Product};

    use super::super::component_scene::{
        ComponentSceneActor, ComponentSceneCommandView, ComponentSceneContext, ComponentSceneSnapshot,
    };
    use super::super::mod_presentation::{
        ModCgameAudioFrame, ModCoreParams, ModScenePublication, ModServicesParams, ModSyscalls,
    };
    use super::super::q3_client::visibility::ApplicationQ3SceneQueries;
    use super::super::simulation::q3::types::{Q3EventSource, Q3PlayerEventSequence};
    use qa_guest::qvm::mod_presentation_checkpoint::SourceGameState;

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
        consumed: RefCell<Vec<(i32, i64)>>,
        consoled: RefCell<Vec<Vec<String>>>,
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
            Self {
                mounts,
                scene,
                consumed: RefCell::new(Vec::new()),
                consoled: RefCell::new(Vec::new()),
            }
        }
    }

    impl ModSceneRendererOps<MockError> for () {
        fn scene_operations(
            &mut self,
            _: &Q3SceneContent,
            _: &WorldViewInput,
            _: u32,
            _: &Q3SceneRenderOptions,
        ) -> Result<Vec<qa_client::render::scene::submissions::SceneOperation>, MockError> {
            Ok(Vec::new())
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
            event: &Q3SourcePlayerEvent,
            sequence: i64,
            _: &mut dyn ModSyscalls<Self>,
        ) -> Result<(), MockError> {
            self.consumed.borrow_mut().push((event.event, sequence));
            Ok(())
        }

        fn console_command(
            &mut self,
            _: &mut (),
            arguments: &[String],
            _: &mut dyn ModSyscalls<Self>,
        ) -> Result<bool, MockError> {
            self.consoled.borrow_mut().push(arguments.to_vec());
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

        fn preload(
            &mut self,
            _: &mut (),
            _: &Q3SceneContent,
            _: &[qa_content::q3::presentation::scene::Q3PresentedScene],
        ) -> Result<(), MockError> {
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
        generation: Cell<u64>,
        live: Cell<bool>,
        module: ModuleId,
        declaration: QvmModPresentationDeclaration,
        content: ContentId,
        source_id: ActorId,
        owner: PresentationOwner,
        context: RefCell<Option<ComponentSceneContext<SourceGameState>>>,
        actor: ActorId,
        received: RefCell<Vec<Vec<String>>>,
        admit_commands: Cell<bool>,
    }

    impl ModPresentationSource for MockSource {
        fn generation(&self) -> u64 {
            self.generation.get()
        }

        fn live(&self, viewer: &ActorId) -> bool {
            self.live.get() && viewer == &self.actor
        }

        fn assert_current(&self) -> Result<(), String> {
            Ok(())
        }

        fn live_module(&self) -> ModuleId {
            self.module.clone()
        }

        fn prepared_module(&self) -> ModuleId {
            self.module.clone()
        }

        fn source_abi(&self) -> QvmAbi {
            QvmAbi::Modern
        }

        fn declaration(&self) -> QvmModPresentationDeclaration {
            self.declaration.clone()
        }

        fn core_artifact(&self) -> QvmArtifact {
            QvmArtifact {
                module: self.module.clone(),
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
                    name: self.module.id.clone(),
                },
                artifact_path: self.module.artifact_path.clone(),
                digest: ContentDigest("sha256:00".to_string()),
                revision: self.module.revision.clone(),
            }
        }

        fn client_command(&self, _: &ActorId, arguments: &[String]) -> Result<bool, String> {
            self.received.borrow_mut().push(arguments.to_vec());
            Ok(self.admit_commands.get())
        }

        fn prepared_source_id(&self) -> ActorId {
            self.source_id.clone()
        }

        fn presentation_owner(&self) -> PresentationOwner {
            self.owner.clone()
        }

        fn scene_context(&self, _: &ActorId) -> Option<ComponentSceneContext<SourceGameState>> {
            self.context.borrow().clone()
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

    struct MockAudio;

    impl ModAudioSink for MockAudio {
        fn receive_cgame_frame(&self, _: ModCgameAudioFrame) {}
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

    struct MockClock;

    impl ModPresentationClock for MockClock {
        fn now(&self) -> f64 {
            5000.0
        }

        fn frame_number(&self) -> i64 {
            40
        }
    }

    fn module(id: &str) -> ModuleId {
        ModuleId {
            id: id.to_string(),
            artifact_path: format!("mock/{id}"),
            digest: "00".to_string(),
            revision: "1".to_string(),
        }
    }

    fn player_declaration() -> QvmModPresentationDeclaration {
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
            hud: Some(PresentationHud {
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

    type MockEffects = Rc<RefCell<HashMap<u64, (SeatComponentEffect, Option<SeatComponentOverlay>)>>>;

    struct MockSeat {
        seat: SeatId,
        client: ClientId,
        actor: ActorId,
        camera: SceneCamera,
        effects: MockEffects,
        next_effect: Cell<u64>,
        unbound: Rc<RefCell<Vec<u64>>>,
    }

    impl MockSeat {
        fn new(seat: SeatId, client: ClientId, actor: ActorId) -> Self {
            let zero = Vec3 { x: 0.0, y: 0.0, z: 0.0 };
            Self {
                seat,
                client,
                actor,
                camera: SceneCamera {
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
                },
                effects: Rc::new(RefCell::new(HashMap::new())),
                next_effect: Cell::new(0),
                unbound: Rc::new(RefCell::new(Vec::new())),
            }
        }
    }

    impl ModCollectionSeat for MockSeat {
        fn seat_id(&self) -> SeatId {
            self.seat.clone()
        }

        fn client_id(&self) -> ClientId {
            self.client.clone()
        }

        fn actor(&self) -> ActorId {
            self.actor.clone()
        }

        fn camera(&self) -> Result<SceneCamera, SeatError> {
            Ok(self.camera)
        }

        fn viewport(&self) -> Result<ViewRect, SeatError> {
            Ok(self.camera.viewport)
        }

        fn split_screen(&self) -> bool {
            false
        }

        fn bind_component_effects(
            &self,
            frame: SeatComponentEffect,
            overlay: Option<SeatComponentOverlay>,
        ) -> Box<dyn FnOnce()> {
            let handle = self.next_effect.get();
            self.next_effect.set(handle + 1);
            self.effects.borrow_mut().insert(handle, (frame, overlay));
            let effects = Rc::clone(&self.effects);
            let unbound = Rc::clone(&self.unbound);
            Box::new(move || {
                effects.borrow_mut().remove(&handle);
                unbound.borrow_mut().push(handle);
            })
        }
    }

    struct MockCommands {
        appended: RefCell<Vec<(String, ModCommandSource)>>,
        bindings: Rc<RefCell<HashMap<u64, ModProducerBinding>>>,
        discarded: RefCell<Vec<u64>>,
        pending: RefCell<HashMap<u64, bool>>,
    }

    impl MockCommands {
        fn new() -> Self {
            Self {
                appended: RefCell::new(Vec::new()),
                bindings: Rc::new(RefCell::new(HashMap::new())),
                discarded: RefCell::new(Vec::new()),
                pending: RefCell::new(HashMap::new()),
            }
        }
    }

    impl ModCollectionCommands for MockCommands {
        fn append(&self, text: &str, source: &ModCommandSource) -> Result<(), ContractError> {
            self.appended.borrow_mut().push((text.to_string(), source.clone()));
            Ok(())
        }

        fn bind_producer(&self, instance: u64, binding: ModProducerBinding) -> Box<dyn FnOnce()> {
            self.bindings.borrow_mut().insert(instance, binding);
            let bindings = Rc::clone(&self.bindings);
            Box::new(move || {
                bindings.borrow_mut().remove(&instance);
            })
        }

        fn discard_producer(&self, instance: u64) {
            self.discarded.borrow_mut().push(instance);
        }

        fn producer_pending(&self, instance: u64) -> bool {
            self.pending.borrow().get(&instance).copied().unwrap_or(false)
        }
    }

    struct MockHud {
        draws: RefCell<Vec<usize>>,
    }

    impl ModHudDraw for MockHud {
        fn draw_hud(
            &self,
            submissions: &[super::super::mod_presentation::ModPresentationHudSubmission],
            _: &SeatOverlayContext,
        ) {
            self.draws.borrow_mut().push(submissions.len());
        }
    }

    struct MockRegistration {
        registered: RefCell<Vec<String>>,
        removed: RefCell<Vec<String>>,
        closes: Cell<usize>,
    }

    impl Q3CommandRegistration for MockRegistration {
        fn register(&self, name: &str) {
            self.registered.borrow_mut().push(name.to_string());
        }

        fn remove(&self, name: &str) {
            self.removed.borrow_mut().push(name.to_string());
        }

        fn close(&self) {
            self.closes.set(self.closes.get() + 1);
        }
    }

    #[derive(Clone)]
    struct TestPayload(Option<Q3SourcePlayerEvent>);

    impl ModQ3EventPayload for TestPayload {
        fn q3_player_event(&self) -> Option<&Q3SourcePlayerEvent> {
            self.0.as_ref()
        }
    }

    struct Fixture {
        authority: IdentityOwner,
        viewer: ActorId,
        seat: Rc<RefCell<MockSeat>>,
        source: Rc<MockSource>,
        backend: Rc<RefCell<MockBackend>>,
        commands: Rc<MockCommands>,
        hud: Rc<MockHud>,
        registration: Rc<MockRegistration>,
        queued: Rc<RefCell<Vec<ComponentClientCommandRequest>>>,
    }

    impl Fixture {
        fn new() -> Self {
            let authority = IdentityOwner::create("mod-presentations-test").unwrap();
            let viewer = authority.actor(0, 1);
            let seat = Rc::new(RefCell::new(MockSeat::new(
                authority.seat(0),
                authority.client(0, 1),
                viewer.clone(),
            )));
            let identical = module("mock");
            let source = Rc::new(MockSource {
                generation: Cell::new(7),
                live: Cell::new(true),
                module: identical,
                declaration: player_declaration(),
                content: ContentId("mock:test:1".to_string()),
                source_id: authority.actor(1, 1),
                owner: PresentationOwner {
                    provider: ProviderId {
                        namespace: "mock".to_string(),
                        name: "provider".to_string(),
                    },
                    generation: 7,
                },
                context: RefCell::new(Some(test_context(viewer.clone()))),
                actor: viewer.clone(),
                received: RefCell::new(Vec::new()),
                admit_commands: Cell::new(false),
            });
            Self {
                authority,
                viewer,
                seat,
                source,
                backend: Rc::new(RefCell::new(MockBackend::new())),
                commands: Rc::new(MockCommands::new()),
                hud: Rc::new(MockHud {
                    draws: RefCell::new(Vec::new()),
                }),
                registration: Rc::new(MockRegistration {
                    registered: RefCell::new(Vec::new()),
                    removed: RefCell::new(Vec::new()),
                    closes: Cell::new(0),
                }),
                queued: Rc::new(RefCell::new(Vec::new())),
            }
        }

        fn options(&self) -> ApplicationModPresentationsOptions<MockBackend, MockSeat> {
            let registration = Rc::clone(&self.registration);
            let queued = Rc::clone(&self.queued);
            ApplicationModPresentationsOptions {
                assets: Rc::new(MockAssets),
                audio: Rc::new(MockAudio),
                queries: Rc::new(MockQueries),
                print: Rc::new(|_| {}),
                next_frame: Rc::new(|| {}),
                clock: Rc::new(MockClock),
                renderer: None,
                session: self.authority.session().clone(),
                input: Some(ModCollectionInput {
                    commands: self.commands.clone(),
                    client_command_registration: Rc::new(move |_, _| registration.clone()),
                    engine_commands: HashSet::from(["quit".to_string()]),
                }),
                presentation_media: None,
                queue_command: Some(Rc::new(move |request| {
                    queued.borrow_mut().push(request);
                })),
                system_cinematics: None,
                hud_sink: self.hud.clone(),
                free_memory: Rc::new(|| 1 << 30),
            }
        }

        fn collection(&self) -> ApplicationModPresentations<MockBackend, MockSeat> {
            ApplicationModPresentations::new(self.options(), Rc::clone(&self.backend))
        }

        fn presentations(&self) -> Vec<Rc<RefCell<MockSeat>>> {
            vec![Rc::clone(&self.seat)]
        }

        fn sources(&self) -> Vec<Rc<dyn ModPresentationSource>> {
            vec![self.source.clone()]
        }

        fn player_event(&self, id: i32) -> Q3SourcePlayerEvent {
            Q3SourcePlayerEvent {
                actor: self.viewer.clone(),
                source: Q3EventSource {
                    module: self.source.prepared_contract_module(),
                    abi_profile: QvmAbiProfile::Modern,
                },
                player_state: QvmPlayerState::default(),
                event: id,
                parameter: 0,
                sequence: Q3PlayerEventSequence::External { time: 100 },
                origin: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                time: 100,
            }
        }

        fn foreign_event(&self, id: i32, sequence: i64) -> SimulationPresentationEvent<TestPayload> {
            SimulationPresentationEvent {
                source: SourcePresentationEvent::Foreign {
                    kind: "q3-source".to_string(),
                    actor: Some(self.viewer.clone()),
                    payload: TestPayload(Some(self.player_event(id))),
                },
                owner: None,
                recipient: None,
                sequence,
                content: ContentId("mock:test:1".to_string()),
                seconds: 0.0,
                source_entity: None,
            }
        }

        fn instance(&self) -> u64 {
            let bindings = self.commands.bindings.borrow();
            assert_eq!(bindings.len(), 1);
            *bindings.keys().next().unwrap()
        }

        fn entry_source(&self, instance: u64) -> ModCommandSource {
            ModCommandSource {
                session: self.authority.session().clone(),
                origin: CommandOrigin::LocalSeat {
                    seat: self.seat.borrow().seat_id(),
                    client: self.seat.borrow().client_id(),
                },
                producer: Some(ModCommandProducer {
                    module: self.source.prepared_contract_module(),
                    instance,
                }),
            }
        }
    }

    #[test]
    fn prepare_creates_entry_and_binds_effects() {
        let fixture = Fixture::new();
        let mut collection = fixture.collection();
        collection
            .prepare(
                &fixture.presentations(),
                &fixture.sources(),
                &[] as &[SimulationPresentationEvent<TestPayload>],
                0,
            )
            .unwrap();
        assert_eq!(fixture.seat.borrow().effects.borrow().len(), 1);
        assert_eq!(fixture.commands.bindings.borrow().len(), 1);
        assert!(!collection.pending_commands());
        collection
            .prepare(
                &fixture.presentations(),
                &fixture.sources(),
                &[] as &[SimulationPresentationEvent<TestPayload>],
                1,
            )
            .unwrap();
        assert_eq!(fixture.seat.borrow().effects.borrow().len(), 1);
        assert_eq!(fixture.commands.bindings.borrow().len(), 1);
    }

    #[test]
    fn effect_frame_renders_scene_and_hud() {
        let fixture = Fixture::new();
        let mut collection = fixture.collection();
        collection
            .prepare(
                &fixture.presentations(),
                &fixture.sources(),
                &[] as &[SimulationPresentationEvent<TestPayload>],
                0,
            )
            .unwrap();
        let seat = fixture.seat.borrow();
        let (frame, overlay) = seat.effects.borrow().get(&0).unwrap().clone();
        let order = create_source_scene_order(Vec::new());
        let rendered = frame(&seat.camera, &order, None);
        assert_eq!(rendered.q3_admissions.len(), 1);
        assert!(rendered.operations.is_empty());
        let overlay = overlay.unwrap();
        assert!(overlay.bodies.unwrap()().is_empty());
        (overlay.draw)(&SeatOverlayContext {
            camera: seat.camera,
            viewport: seat.camera.viewport,
            seat: seat.seat_id(),
            time_ms: 1000.0,
        });
        assert_eq!(fixture.hud.draws.borrow().as_slice(), &[0]);
    }

    #[test]
    fn prepare_consumes_matching_player_events() {
        let fixture = Fixture::new();
        let mut collection = fixture.collection();
        let mut other = fixture.foreign_event(2, 11);
        if let SourcePresentationEvent::Foreign { payload, .. } = &mut other.source {
            payload.0.as_mut().unwrap().source.module = ModuleIdentity {
                id: ProviderId::new("mock", "other"),
                artifact_path: "mock/other".to_string(),
                digest: ContentDigest("sha256:00".to_string()),
                revision: "1".to_string(),
            };
        }
        let mut stranger = fixture.foreign_event(3, 12);
        stranger.recipient = Some(fixture.authority.actor(9, 9));
        collection
            .prepare(
                &fixture.presentations(),
                &fixture.sources(),
                &[fixture.foreign_event(1, 10), other, stranger],
                0,
            )
            .unwrap();
        assert_eq!(fixture.backend.borrow().consumed.borrow().as_slice(), &[(1, 10)]);
    }

    #[test]
    fn prepare_prunes_retired_source() {
        let fixture = Fixture::new();
        let mut collection = fixture.collection();
        collection
            .prepare(
                &fixture.presentations(),
                &fixture.sources(),
                &[] as &[SimulationPresentationEvent<TestPayload>],
                0,
            )
            .unwrap();
        assert_eq!(fixture.seat.borrow().effects.borrow().len(), 1);
        fixture.source.generation.set(8);
        collection
            .prepare(
                &fixture.presentations(),
                &fixture.sources(),
                &[] as &[SimulationPresentationEvent<TestPayload>],
                1,
            )
            .unwrap();
        assert_eq!(fixture.seat.borrow().effects.borrow().len(), 1);
        assert_eq!(fixture.seat.borrow().unbound.borrow().len(), 1);
        assert_eq!(fixture.commands.discarded.borrow().len(), 1);
        assert_eq!(fixture.commands.bindings.borrow().len(), 1);
    }

    #[test]
    fn prepare_skips_contextless_source() {
        let fixture = Fixture::new();
        *fixture.source.context.borrow_mut() = None;
        let mut collection = fixture.collection();
        collection
            .prepare(
                &fixture.presentations(),
                &fixture.sources(),
                &[] as &[SimulationPresentationEvent<TestPayload>],
                0,
            )
            .unwrap();
        assert!(fixture.seat.borrow().effects.borrow().is_empty());
        assert!(fixture.commands.bindings.borrow().is_empty());
    }

    #[test]
    fn queue_and_dispatch_console_command() {
        let fixture = Fixture::new();
        let mut collection = fixture.collection();
        collection
            .prepare(
                &fixture.presentations(),
                &fixture.sources(),
                &[] as &[SimulationPresentationEvent<TestPayload>],
                0,
            )
            .unwrap();
        let source = fixture.entry_source(fixture.instance());
        let command = ModCommandInvocation::new(
            vec!["say".to_string(), "hi".to_string()],
            "hi".to_string(),
            source,
            Dialect::Q3,
        );
        assert!(collection.queue_console(&command).unwrap());
        assert_eq!(fixture.queued.borrow().len(), 1);
        let request = fixture.queued.borrow().first().unwrap().clone();
        assert_eq!(request.name, "say");
        assert_eq!(request.arguments, vec!["hi".to_string()]);
        assert_eq!(
            collection.dispatch_command(&request).unwrap(),
            ComponentCommandOutcome::Handled
        );
        assert_eq!(
            fixture.backend.borrow().consoled.borrow().as_slice(),
            &[vec!["say".to_string(), "hi".to_string()]]
        );
    }

    #[test]
    fn dispatch_retires_unknown_target() {
        let fixture = Fixture::new();
        let mut collection = fixture.collection();
        collection
            .prepare(
                &fixture.presentations(),
                &fixture.sources(),
                &[] as &[SimulationPresentationEvent<TestPayload>],
                0,
            )
            .unwrap();
        let request = ComponentClientCommandRequest {
            consumer: ComponentClientCommandTarget {
                owner: fixture.source.presentation_owner(),
                instance: 99,
                seat: fixture.seat.borrow().seat_id(),
                viewer: fixture.viewer.clone(),
            },
            mode: ComponentClientCommandMode::Console,
            name: "say".to_string(),
            arguments: Vec::new(),
            seat: fixture.seat.borrow().seat_id(),
            source: fixture.entry_source(99),
        };
        assert_eq!(
            collection.dispatch_command(&request).unwrap(),
            ComponentCommandOutcome::Retired
        );
    }

    #[test]
    fn dispatch_reliable_falls_through_to_source() {
        let fixture = Fixture::new();
        fixture.source.admit_commands.set(true);
        let mut collection = fixture.collection();
        collection
            .prepare(
                &fixture.presentations(),
                &fixture.sources(),
                &[] as &[SimulationPresentationEvent<TestPayload>],
                0,
            )
            .unwrap();
        let source = fixture.entry_source(fixture.instance());
        let command = ModCommandInvocation::new(vec!["kill".to_string()], String::new(), source, Dialect::Q3);
        assert!(collection.queue_console(&command).unwrap());
        let mut request = fixture.queued.borrow().first().unwrap().clone();
        request.mode = ComponentClientCommandMode::Reliable;
        assert_eq!(
            collection.dispatch_command(&request).unwrap(),
            ComponentCommandOutcome::Handled
        );
        assert!(fixture.backend.borrow().consoled.borrow().is_empty());
        assert_eq!(fixture.source.received.borrow().as_slice(), &[vec!["kill".to_string()]]);
    }

    #[test]
    fn command_cvars_and_script_cover_entry() {
        let fixture = Fixture::new();
        let collection = fixture.collection();
        let missing = ModCommandSource {
            session: fixture.authority.session().clone(),
            origin: CommandOrigin::LocalSeat {
                seat: fixture.seat.borrow().seat_id(),
                client: fixture.seat.borrow().client_id(),
            },
            producer: None,
        };
        assert!(collection.command_cvars(&missing).unwrap().is_none());
        assert!(collection
            .read_command_script("autoexec.cfg", &missing)
            .unwrap()
            .is_none());
        let mut collection = collection;
        collection
            .prepare(
                &fixture.presentations(),
                &fixture.sources(),
                &[] as &[SimulationPresentationEvent<TestPayload>],
                0,
            )
            .unwrap();
        let source = fixture.entry_source(fixture.instance());
        assert!(collection.command_cvars(&source).unwrap().is_some());
        assert!(collection
            .read_command_script("autoexec.cfg", &source)
            .unwrap()
            .is_none());
    }

    #[test]
    fn capture_restore_roundtrips() {
        let fixture = Fixture::new();
        let mut collection = fixture.collection();
        collection
            .prepare(
                &fixture.presentations(),
                &fixture.sources(),
                &[] as &[SimulationPresentationEvent<TestPayload>],
                0,
            )
            .unwrap();
        let checkpoint = collection
            .capture_checkpoint(&fixture.presentations(), &fixture.sources())
            .unwrap();
        let SaveJson::Object(members) = &checkpoint else {
            panic!("checkpoint must be an object");
        };
        assert!(members.iter().any(|(name, _)| name == "version"));
        let mut restored = fixture.collection();
        let viewer = fixture.viewer.clone();
        restored
            .restore_checkpoint(
                &checkpoint,
                &fixture.presentations(),
                &fixture.sources(),
                &|saved: &SavedActorId| (*saved == SavedActorId::from(&viewer)).then(|| viewer.clone()),
            )
            .unwrap();
        restored.publish_restored().unwrap();
        assert_eq!(fixture.seat.borrow().effects.borrow().len(), 2);
        let again = restored
            .capture_checkpoint(&fixture.presentations(), &fixture.sources())
            .unwrap();
        let SaveJson::Object(members) = &again else {
            panic!("checkpoint must be an object");
        };
        let seats = members.iter().find(|(name, _)| name == "seats").unwrap().1.clone();
        let SaveJson::Array(seats) = seats else {
            panic!("seats must be an array");
        };
        let SaveJson::Object(seat) = &seats[0] else {
            panic!("seat must be an object");
        };
        let sources = seat.iter().find(|(name, _)| name == "sources").unwrap().1.clone();
        let SaveJson::Array(sources) = sources else {
            panic!("sources must be an array");
        };
        let SaveJson::Object(row) = &sources[0] else {
            panic!("source row must be an object");
        };
        let state = row.iter().find(|(name, _)| name == "state").unwrap().1.clone();
        let SaveJson::Object(state) = state else {
            panic!("state must be an object");
        };
        let kind = state.iter().find(|(name, _)| name == "kind").unwrap().1.clone();
        assert_eq!(kind, save_str("initialized"));
    }

    #[test]
    fn close_cleans_up_and_rejects_further_work() {
        let fixture = Fixture::new();
        let mut collection = fixture.collection();
        collection
            .prepare(
                &fixture.presentations(),
                &fixture.sources(),
                &[] as &[SimulationPresentationEvent<TestPayload>],
                0,
            )
            .unwrap();
        collection.close().unwrap();
        assert!(fixture.seat.borrow().effects.borrow().is_empty());
        assert_eq!(fixture.commands.discarded.borrow().len(), 1);
        assert!(fixture.commands.bindings.borrow().is_empty());
        assert!(collection.close().is_ok());
        let error = collection
            .prepare(
                &fixture.presentations(),
                &fixture.sources(),
                &[] as &[SimulationPresentationEvent<TestPayload>],
                1,
            )
            .expect_err("closed collection must reject prepare");
        assert!(matches!(error, ModPresentationsError::Presentation(message)
            if message == "Component presentation collection is closed"));
    }

    #[test]
    fn pending_commands_reflects_buffer() {
        let fixture = Fixture::new();
        let mut collection = fixture.collection();
        collection
            .prepare(
                &fixture.presentations(),
                &fixture.sources(),
                &[] as &[SimulationPresentationEvent<TestPayload>],
                0,
            )
            .unwrap();
        assert!(!collection.pending_commands());
        fixture.commands.pending.borrow_mut().insert(fixture.instance(), true);
        assert!(collection.pending_commands());
    }

    struct StubMedia {
        sources: Vec<Rc<dyn ModPresentationSource>>,
        audio_batches: RefCell<Vec<usize>>,
        shaders: Cell<usize>,
    }

    impl super::super::remote_components::RemoteComponentMedia<TestPayload> for StubMedia {
        type Error = MockError;
        type Source = Rc<dyn ModPresentationSource>;

        fn prepare_audio(&mut self, events: &[SimulationPresentationEvent<TestPayload>]) -> Result<(), MockError> {
            self.audio_batches.borrow_mut().push(events.len());
            Ok(())
        }

        fn prepare_shaders(&mut self) -> Result<(), MockError> {
            self.shaders.set(self.shaders.get() + 1);
            Ok(())
        }

        fn mod_presentation_sources(&self) -> Vec<Rc<dyn ModPresentationSource>> {
            self.sources.clone()
        }
    }

    #[test]
    fn remote_view_drives_collection_through_seam() {
        use super::super::remote_components::{RemoteComponentQueue, RemoteComponentView};

        let fixture = Fixture::new();
        let view_queue = RemoteComponentQueue::new();
        let queue_sink = view_queue.clone();
        let mut options = fixture.options();
        options.queue_command = Some(Rc::new(move |request| {
            queue_sink.queue_command(request).unwrap();
        }));
        let collection = ApplicationModPresentations::new(options, Rc::clone(&fixture.backend));
        let media = StubMedia {
            sources: fixture.sources(),
            audio_batches: RefCell::new(Vec::new()),
            shaders: Cell::new(0),
        };
        let mut view = RemoteComponentView::new(collection, media, view_queue.clone());
        assert!(!view.pending_commands());
        view.prepare(&fixture.seat, &[], 41).unwrap();
        assert_eq!(fixture.seat.borrow().effects.borrow().len(), 1);
        let instance = fixture.instance();
        view_queue
            .queue_command(ComponentClientCommandRequest {
                consumer: ComponentClientCommandTarget {
                    owner: fixture.source.presentation_owner(),
                    instance,
                    seat: fixture.seat.borrow().seat_id(),
                    viewer: fixture.viewer.clone(),
                },
                mode: ComponentClientCommandMode::Console,
                name: "say".to_string(),
                arguments: vec!["hi".to_string()],
                seat: fixture.seat.borrow().seat_id(),
                source: fixture.entry_source(instance),
            })
            .unwrap();
        assert!(view.pending_commands());
        view.prepare_media(&[], true).unwrap();
        assert_eq!(
            fixture.backend.borrow().consoled.borrow().as_slice(),
            &[vec!["say".to_string(), "hi".to_string()]]
        );
        assert!(!view.pending_commands());
        view.close();
        assert!(fixture.seat.borrow().effects.borrow().is_empty());
    }

    #[test]
    fn helpers_cover_abi_identity_bodies_and_fog() {
        assert!(abi_matches(QvmAbi::Modern, QvmAbiProfile::Modern));
        assert!(abi_matches(QvmAbi::Legacy, QvmAbiProfile::Legacy116n));
        assert!(!abi_matches(QvmAbi::Modern, QvmAbiProfile::Legacy116n));
        let module = ModuleIdentity {
            id: ProviderId::new("mock", "unit"),
            artifact_path: "mock/unit".to_string(),
            digest: ContentDigest("sha256:00".to_string()),
            revision: "1".to_string(),
        };
        let written = write_module_identity(&module);
        let provider = SaveReader::at(&written, "test").field("id").string().unwrap();
        let reader = SaveReader::at(&written, "test");
        assert_eq!(
            parse_provider(reader.field("id"), &provider).unwrap(),
            ProviderId::new("mock", "unit")
        );
        let body = ComponentBody {
            owner: PresentationOwner {
                provider: ProviderId::new("mock", "provider"),
                generation: 7,
            },
            actor: IdentityOwner::create("bodies").unwrap().actor(0, 1),
            content: ContentId("mock:test:1".to_string()),
            time: 1500.9,
            parts: vec![super::super::component_bodies::ComponentBodyPart {
                part: QvmBodyPart::Head,
                base: true,
                passes: vec![super::super::component_bodies::BodyMaterial {
                    custom_shader: Some(SceneShader::new("shader")),
                    custom_skin: Some(SceneSkin {
                        path: "skin".to_string(),
                        surfaces: vec![SkinMapping {
                            name: "surface".to_string(),
                            shader: "replacement".to_string(),
                        }],
                    }),
                    shader_rgba: Vec4 {
                        x: 1.0,
                        y: 0.0,
                        z: 0.0,
                        w: 1.0,
                    },
                    shader_tex_coord: Vec2 { x: 0.0, y: 0.0 },
                    shader_time: 0.5,
                    render_flags: 3,
                    lighting_origin: Vec3 { x: 1.0, y: 2.0, z: 3.0 },
                    shadow_plane: 0.25,
                    non_normalized_axes: true,
                }],
            }],
        };
        let converted = scene_bodies(&[body]);
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0].time_ms, 1500);
        assert_eq!(converted[0].parts[0].part, SceneBodyPart::Head);
        assert_eq!(converted[0].parts[0].passes[0].shader_time, 0.5);
        assert!(converted[0].parts[0].passes[0].non_normalized_axes);
        let fog = q1_fog_params(Some(&SceneFog::Q1 {
            color: Vec3 { x: 0.5, y: 0.5, z: 0.5 },
            density: 0.1,
            sky_factor: 0.2,
        }));
        assert!(fog.is_some());
        assert!(q1_fog_params(None).is_none());
        assert!(q1_fog_params(Some(&SceneFog::None)).is_none());
    }
}
