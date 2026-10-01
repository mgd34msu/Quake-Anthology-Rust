//! QVM presentation client: guest modules over shared host services.
//!
//! Port of `src/app/bootstrap/q3-client/qvm.ts`
//! (`ApplicationQvmClient`, `qvmClientCommands`). The port owns the
//! trap dispatch order, cgame status masking, held-weapon pass
//! interception, equipment/body policy state, command fan-out, and the
//! init/draw/input/shutdown lifecycle. Module mechanics (bytecode
//! execution, hook installation, file/script owners, render/audio
//! emission) stay behind injected seams: the Rust guest modules do not
//! expose execution yet, so [`QvmPresentationModules`] runs frames and
//! input while [`QvmClientBridges`] answers the engine-owned trap
//! stages in donor order.

use qa_content::q3::presentation::ref_entity::{RefModelEntity, SceneModel, SceneShader, SceneSkin, ShadedFields};
use qa_core::cmd_buffer::{CommandContext, CommandOrigin};
use qa_core::identity::ActorId;
use qa_core::math::{Vec3, Vec4};
use qa_guest::qvm::abi::QvmCgameImport;
use qa_guest::qvm::client_state::{AbiProfile, CallKind, HostCall, QvmRole as CallRole, SyscallMemory};
use qa_guest::qvm::cvar_syscalls::CvarHost;
use qa_guest::qvm::game_data::{QvmArtifact, QvmRole};
use qa_guest::qvm::legacy_client_abi::legacy_client_command;
use qa_guest::qvm::render_record::{read_qvm_ref_entity, QvmRefEntity, QvmRefEntityKind, QVM_REF_ENTITY_BYTES};
use qa_guest::GuestError;
use qa_net::q3::WireUserCommand as NetUserCommand;
use thiserror::Error;

use super::client_state::{LocalQ3ClientBindings, LocalQ3ClientState, Q3ClientStateError};
use super::equipment::{q3_equipment_command, Q3EquipmentPresentation};
use super::qvm_scalars::{QvmScalarResult, QvmScalarStage};
use super::status::CgameStatusView;

/// QVM client failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Q3QvmError {
    /// QVM presentation belongs to a retired gamestate.
    #[error("QVM presentation belongs to a retired gamestate")]
    Retired,
    /// QVM cgame has not initialized.
    #[error("QVM cgame has not initialized")]
    NotInitialized,
    /// Presentation modules are not initialized.
    #[error("Presentation modules are not initialized")]
    PresentationNotInitialized,
    /// Retained presentation artifact has the wrong role.
    #[error("Retained presentation artifact has the wrong role")]
    WrongArtifact,
    /// Loaded presentation artifact has the wrong role.
    #[error("Loaded presentation artifact has the wrong role")]
    WrongLoadedArtifact,
    /// Body replacements need a matched declaration.
    #[error("This cgame needs an artifact-matched cgame-presentation.json declaration for body replacements")]
    BodyDeclarations,
    /// Body capture needs matched player mesh call sites.
    #[error("This cgame needs artifact-matched player mesh call sites for component body materials")]
    BodyMeshes,
    /// Component camera control needs a view-weapon boundary.
    #[error("Component camera control requires a qualified original view-weapon boundary")]
    ViewBoundary,
    /// Weapon presentation needs a HUD boundary.
    #[error("Selected weapon presentation requires a qualified original cgame HUD boundary")]
    HudBoundary,
    /// Original weapon attachments no longer belong to this presentation.
    #[error("Original weapon attachments no longer belong to this QVM presentation")]
    StaleAttachment,
    /// Original weapon attachment scopes unwound out of order.
    #[error("Original weapon attachment scopes unwound out of order")]
    ScopeOrder,
    /// Original weapon parent is not a model.
    #[error("Original weapon parent is not a model")]
    ParentNotModel,
    /// Original weapon parent has no decoded model.
    #[error("Original weapon parent has no decoded model")]
    ParentNoModel,
    /// QVM body source is not a model.
    #[error("QVM body source is not a model")]
    BodyNotModel,
    /// QVM body model handle has no scene model.
    #[error("QVM body model handle has no scene model")]
    BodyNoModel,
    /// QVM presentation cleanup failed.
    #[error("QVM presentation cleanup failed: {0}")]
    Cleanup(String),
    /// QVM initialization and cleanup failed.
    #[error("QVM initialization and cleanup failed: {error}; cleanup: {cleanup}")]
    InitCleanup {
        /// Initialization failure.
        error: Box<Q3QvmError>,
        /// Cleanup failure.
        cleanup: Box<Q3QvmError>,
    },
    /// Local client failure.
    #[error("{0}")]
    Connection(String),
    /// Guest failure.
    #[error(transparent)]
    Guest(#[from] GuestError),
}

impl From<Q3ClientStateError> for Q3QvmError {
    fn from(error: Q3ClientStateError) -> Self {
        Self::Connection(error.to_string())
    }
}

/// Engine command buffer behind the QVM script fan-out.
pub trait QvmCommandBuffer {
    /// Execute text immediately.
    fn execute_now(&mut self, text: &str, context: &CommandContext);
    /// Queue text ahead of the pending program.
    fn insert(&mut self, text: &str, context: &CommandContext);
    /// Queue text behind the pending program.
    fn append(&mut self, text: &str, context: &CommandContext);
}

/// QVM script command fan-out (donor `qvmClientCommands`).
pub struct QvmClientCommands<'a, B: QvmCommandBuffer + ?Sized> {
    buffer: &'a mut B,
    context: CommandContext,
}

impl<B: QvmCommandBuffer + ?Sized> QvmClientCommands<'_, B> {
    /// Execute text immediately as a QVM script.
    pub fn execute_now(&mut self, text: &str) {
        self.buffer.execute_now(text, &self.context);
    }

    /// Queue text ahead of the pending program as a QVM script.
    pub fn insert(&mut self, text: &str) {
        self.buffer.insert(text, &self.context);
    }

    /// Queue text behind the pending program as a QVM script.
    pub fn append(&mut self, text: &str) {
        self.buffer.append(text, &self.context);
    }
}

/// Wrap a command buffer with the QVM script origin.
pub fn qvm_client_commands<'a, B: QvmCommandBuffer + ?Sized>(
    buffer: &'a mut B,
    context: &CommandContext,
    role: QvmRole,
) -> QvmClientCommands<'a, B> {
    let name = match role {
        QvmRole::Ui => "q3-ui",
        QvmRole::Cgame => "q3-cgame",
        QvmRole::Qagame => "q3-qagame",
    };
    QvmClientCommands {
        buffer,
        context: CommandContext {
            session: context.session.clone(),
            origin: CommandOrigin::Script {
                name: name.to_string(),
                caller: Box::new(context.origin.clone()),
            },
        },
    }
}

/// Local connection behind the cgame state bridge.
pub trait QvmClientConnection {
    /// Connection generation.
    fn generation(&self) -> i32;
    /// Latest server message number.
    fn server_message_sequence(&self) -> i32;
    /// Last executed server command.
    fn last_executed_server_command(&self) -> i32;
    /// Seat's client slot.
    fn client_number(&self) -> i32;
    /// Current user-command number.
    fn commands_current_number(&mut self) -> Result<i32, Q3QvmError>;
    /// Read a retained user command.
    fn commands_read(&mut self, number: i32) -> Result<Option<NetUserCommand>, Q3QvmError>;
    /// Ping recorded for a snapshot.
    fn snapshot_ping(&mut self, number: i32) -> Result<Option<i32>, Q3QvmError>;
    /// Execute a reliable server command.
    fn get_server_command(&mut self, number: i32) -> Result<Option<Vec<String>>, Q3QvmError>;
}

impl<B: LocalQ3ClientBindings> QvmClientConnection for LocalQ3ClientState<B> {
    fn generation(&self) -> i32 {
        self.generation
    }

    fn server_message_sequence(&self) -> i32 {
        self.server_message_sequence
    }

    fn last_executed_server_command(&self) -> i32 {
        self.last_executed_server_command
    }

    fn client_number(&self) -> i32 {
        self.client_number
    }

    fn commands_current_number(&mut self) -> Result<i32, Q3QvmError> {
        Ok(LocalQ3ClientState::commands_current_number(self)?)
    }

    fn commands_read(&mut self, number: i32) -> Result<Option<NetUserCommand>, Q3QvmError> {
        Ok(LocalQ3ClientState::commands_read(self, number)?)
    }

    fn snapshot_ping(&mut self, number: i32) -> Result<Option<i32>, Q3QvmError> {
        Ok(LocalQ3ClientState::snapshot_ping(self, number)?)
    }

    fn get_server_command(&mut self, number: i32) -> Result<Option<Vec<String>>, Q3QvmError> {
        Ok(LocalQ3ClientState::get_server_command(self, number)?)
    }
}

/// Engine-owned trap stage in donor dispatch order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QvmBridgeStage {
    /// Common traps.
    Common,
    /// Filesystem traps.
    File,
    /// Client script traps.
    Script,
    /// Render traps.
    Render,
    /// Audio traps.
    Audio,
    /// Client-state traps.
    State,
    /// Collision traps.
    Collision,
    /// Mark traps.
    Mark,
    /// Cinematic traps.
    Cinematic,
    /// Server-browser traps.
    Browser,
    /// UI key traps.
    Key,
}

/// Engine-owned trap stages and their file/script owners.
pub trait QvmClientBridges {
    /// Dispatch one stage, or `None` when the stage declines.
    fn dispatch_bridge(
        &mut self,
        stage: QvmBridgeStage,
        call: &HostCall,
        memory: &mut SyscallMemory,
    ) -> Result<Option<i32>, GuestError>;
    /// Close file/script owners.
    fn close_bridges(&mut self) -> Result<(), String>;
}

/// Cgame event-handling mode (donor `Q3CgameEventHandling`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QvmEventHandling {
    /// No menu.
    None,
    /// Team menu.
    TeamMenu,
    /// Scoreboard.
    Scoreboard,
    /// Edit HUD.
    EditHud,
}

/// Equipment observations recorded while a cgame frame runs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QvmEquipmentNotes {
    /// The frame requested the equipment HUD.
    pub hud_requested: bool,
    /// The frame kept the equipment view visible.
    pub view_visible: bool,
}

/// Guest module mechanics: execution, hooks, and lifecycle.
pub trait QvmPresentationModules {
    /// Initialize the ui module.
    fn init_ui(&mut self, connecting: bool) -> Result<(), GuestError>;
    /// Initialize the cgame module with its handshake.
    fn init_cgame(&mut self, server_message: i32, last_command: i32, client_number: i32) -> Result<(), GuestError>;
    /// Draw a cgame frame, recording equipment observations.
    fn draw_cgame_frame(
        &mut self,
        time_ms: i32,
        demo_playback: bool,
        notes: &mut QvmEquipmentNotes,
    ) -> Result<(), GuestError>;
    /// Refresh the ui module.
    fn refresh_ui(&mut self, time_ms: i32) -> Result<(), GuestError>;
    /// Re-enter the ui connect-screen export.
    fn invoke_connect_screen(&mut self) -> Result<(), GuestError>;
    /// Draw the connect screen.
    fn draw_connect_screen(&mut self, overlay: bool) -> Result<(), GuestError>;
    /// Run a console command; `time_ms` is `None` for cgame.
    fn console_command(&mut self, role: QvmRole, time_ms: Option<i32>, argv: &[String]) -> Result<bool, GuestError>;
    /// Deliver a key event.
    fn key_event(&mut self, role: QvmRole, key: i32, down: bool) -> Result<(), GuestError>;
    /// Deliver a mouse event.
    fn mouse_event(&mut self, role: QvmRole, x: i32, y: i32) -> Result<(), GuestError>;
    /// Switch event handling.
    fn event_handling(&mut self, mode: QvmEventHandling) -> Result<(), GuestError>;
    /// Shut the cgame module down.
    fn shutdown_cgame(&mut self) -> Result<(), GuestError>;
    /// Shut the ui module down.
    fn shutdown_ui(&mut self) -> Result<(), GuestError>;
    /// Retire both modules.
    fn retire_modules(&mut self);
    /// Whether cgame accepts input events.
    fn supports_input_events(&self) -> bool;
}

/// Equipment boundary presence read from the artifact profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QvmEquipmentBoundary {
    /// Qualified HUD boundary.
    pub hud: bool,
    /// Qualified view-weapon boundary.
    pub view: bool,
    /// Qualified status boundary.
    pub status: bool,
    /// Qualified warning boundary.
    pub warning: bool,
    /// Qualified held-weapon call sites.
    pub held: bool,
}

/// Installed hook token minted by the execution seam.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QvmHookToken(pub u64);

/// Equipment hook mechanics owned by module execution.
pub trait QvmEquipmentHooks {
    /// Install the HUD selector hook.
    fn install_selector(&mut self) -> QvmHookToken;
    /// Remove the HUD selector hook.
    fn remove_selector(&mut self, token: QvmHookToken);
}

/// Body boundary presence read from the artifact profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QvmBodyBoundaries {
    /// Artifact-matched body declarations.
    pub declarations: bool,
    /// Artifact-matched player mesh call sites.
    pub captures_player_meshes: bool,
}

/// Body part (donor `QvmBodyPart`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QvmBodyPart {
    /// Full body.
    Body,
    /// Lower.
    Lower,
    /// Upper.
    Upper,
    /// Head.
    Head,
}

/// Body capture and override policy with suppression mechanics.
pub trait QvmBodyPolicy {
    /// Enable body replacement mechanics.
    fn enable_bodies(&mut self, active: bool) -> Result<(), GuestError>;
    /// Close body submissions.
    fn close_bodies(&mut self);
    /// Suppress a trap for body capture.
    fn suppress_body(&mut self, call: &HostCall) -> bool;
    /// Whether body capture is armed.
    fn body_capture_active(&mut self) -> bool;
    /// Whether an entity is selected for capture.
    fn body_capture_selected(&mut self, entity: i32) -> bool;
    /// Offer a resolved body source for capture.
    fn body_capture_submit(&mut self, entity: i32, part: QvmBodyPart, source: RefModelEntity, base: bool) -> bool;
    /// Whether body overrides hide entities.
    fn body_overrides_active(&mut self) -> bool;
    /// Whether an entity stays hidden.
    fn body_hidden(&mut self, entity: i32) -> bool;
}

/// Resolved held-weapon model identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmHeldModelRef {
    /// Model path.
    pub path: String,
    /// Resource identity.
    pub resource: String,
}

/// One held-weapon shader pass.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmHeldPass {
    /// Shader name, if any.
    pub shader: Option<String>,
    /// Pass color.
    pub color: Vec4,
    /// Shader time in seconds.
    pub shader_time: f32,
}

/// Held-weapon parent entity.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmHeldParent {
    /// Owning actor.
    pub actor: ActorId,
    /// Entity number.
    pub entity_number: i32,
    /// Model path.
    pub model: String,
    /// Model resource identity.
    pub resource: String,
    /// Origin.
    pub origin: Vec3,
    /// Axis.
    pub axis: qa_core::math::Axis,
    /// Previous origin.
    pub old_origin: Vec3,
    /// Frame.
    pub frame: i32,
    /// Previous frame.
    pub old_frame: i32,
    /// Back lerp.
    pub back_lerp: f32,
    /// Skin number.
    pub skin: i32,
    /// Shader time in seconds.
    pub shader_time: f32,
    /// Lighting origin.
    pub lighting_origin: Vec3,
    /// Shadow plane.
    pub shadow_plane: f32,
    /// Render flags.
    pub render_flags: i32,
}

/// Captured held weapon (donor `QvmHeldWeapon`).
#[derive(Debug, Clone, PartialEq)]
pub struct QvmHeldWeapon {
    /// Content identity.
    pub content: String,
    /// Parent entity.
    pub parent: QvmHeldParent,
    /// Shader passes in trap order.
    pub passes: Vec<QvmHeldPass>,
}

/// Held-weapon capture scope decoded by module execution.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmHeldScope {
    /// Invocation frame base.
    pub frame: usize,
    /// Invocation frame end.
    pub end: usize,
    /// Gun record pointer.
    pub gun: usize,
    /// Capture state word (0 captures).
    pub state: i32,
    /// Entity number.
    pub entity_number: i32,
    /// Parent `refEntity_t` record bytes.
    pub parent_record: Vec<u8>,
}

/// Open capture scope.
#[derive(Debug)]
struct HeldWeaponInvocation {
    id: u64,
    frame: usize,
    end: usize,
    gun: usize,
    held: QvmHeldWeapon,
}

/// Retained presentation artifacts by role.
#[derive(Debug, Clone)]
pub struct QvmPresentationArtifacts {
    /// Ui artifact.
    pub ui: QvmArtifact,
    /// Cgame artifact.
    pub cgame: QvmArtifact,
}

/// Boxed cvar services forwarding to the shared owner.
struct CvarServiceBox(Box<dyn CvarHost>);

impl CvarHost for CvarServiceBox {
    fn bind_vm(&mut self, name: &str, default: &str, flags: i32) -> i32 {
        self.0.bind_vm(name, default, flags)
    }

    fn read_vm(&mut self, handle: i32) -> Option<qa_guest::qvm::cvar_syscalls::CvarVmBinding> {
        self.0.read_vm(handle)
    }

    fn get(&mut self, name: &str) -> Option<qa_guest::qvm::cvar_syscalls::CvarValue> {
        self.0.get(name)
    }

    fn set(&mut self, name: &str, value: &str) {
        self.0.set(name, value);
    }

    fn set_value(&mut self, name: &str, value: f32) {
        self.0.set_value(name, value);
    }

    fn reset(&mut self, name: &str) {
        self.0.reset(name);
    }

    fn register(&mut self, name: &str, default: &str, flags: i32) {
        self.0.register(name, default, flags);
    }

    fn info_string(&mut self, flags: i32) -> String {
        self.0.info_string(flags)
    }
}

/// Options for [`ApplicationQvmClient`].
pub struct ApplicationQvmClientOptions {
    /// Local connection.
    pub connection: Box<dyn QvmClientConnection>,
    /// Cvar services shared with the status view.
    pub cvars: Box<dyn CvarHost>,
    /// Status visibility probe.
    pub status_visible: Box<dyn FnMut() -> bool>,
    /// Session currency guard (panics when the seat moved on).
    pub assert_session: Box<dyn FnMut()>,
    /// Engine-owned trap stages.
    pub bridges: Box<dyn QvmClientBridges>,
    /// Guest module mechanics.
    pub modules: Box<dyn QvmPresentationModules>,
    /// Body policy and suppression.
    pub bodies: Box<dyn QvmBodyPolicy>,
    /// Equipment hook mechanics.
    pub equipment_hooks: Box<dyn QvmEquipmentHooks>,
    /// Scalar traps.
    pub scalar: Box<dyn QvmScalarStage>,
    /// Module loader by role.
    pub loader: Box<dyn FnMut(QvmRole) -> Result<QvmArtifact, GuestError>>,
    /// Retained artifacts, if any.
    pub artifacts: Option<QvmPresentationArtifacts>,
    /// Milliseconds clock.
    pub now: Box<dyn FnMut() -> i32>,
    /// Key-catcher word.
    pub key_catcher: Box<dyn Fn() -> i32>,
    /// Content identity for held weapons.
    pub content: String,
    /// Component view-weapon visibility probe, if any.
    pub view_weapon_visible: Option<Box<dyn FnMut() -> bool>>,
    /// Selected weapon presentation probe, if any.
    pub equipment_weapon: Option<Box<dyn FnMut() -> Option<Q3EquipmentPresentation>>>,
    /// Equipment boundary presence, if any.
    pub equipment_boundary: Option<QvmEquipmentBoundary>,
    /// Body boundary presence, if any.
    pub body_boundaries: Option<QvmBodyBoundaries>,
    /// Actor behind an entity number, if any.
    pub held_weapon_actor: Option<Box<dyn FnMut(i32) -> Option<ActorId>>>,
    /// Resolve a guest model handle.
    pub resolve_held_model: Box<dyn FnMut(i32) -> Option<QvmHeldModelRef>>,
    /// Resolve a guest shader handle.
    pub resolve_held_shader: Box<dyn FnMut(i32) -> Option<String>>,
    /// Resolve a guest body model handle.
    pub resolve_body_model: Box<dyn FnMut(i32) -> Option<SceneModel>>,
    /// Resolve a guest body shader handle.
    pub resolve_body_shader: Box<dyn FnMut(i32) -> Option<SceneShader>>,
    /// Resolve a guest body skin handle.
    pub resolve_body_skin: Box<dyn FnMut(i32) -> Option<SceneSkin>>,
}

/// QVM presentation client over injected module mechanics.
pub struct ApplicationQvmClient {
    connection: Box<dyn QvmClientConnection>,
    status: CgameStatusView<CvarServiceBox, Box<dyn FnMut() -> bool>>,
    assert_session: Box<dyn FnMut()>,
    bridges: Box<dyn QvmClientBridges>,
    modules: Box<dyn QvmPresentationModules>,
    bodies: Box<dyn QvmBodyPolicy>,
    equipment_hooks: Box<dyn QvmEquipmentHooks>,
    scalar: Box<dyn QvmScalarStage>,
    now: Box<dyn FnMut() -> i32>,
    key_catcher: Box<dyn Fn() -> i32>,
    content: String,
    view_weapon_visible: Option<Box<dyn FnMut() -> bool>>,
    equipment_weapon: Option<Box<dyn FnMut() -> Option<Q3EquipmentPresentation>>>,
    equipment_boundary: Option<QvmEquipmentBoundary>,
    body_boundaries: Option<QvmBodyBoundaries>,
    held_weapon_actor: Option<Box<dyn FnMut(i32) -> Option<ActorId>>>,
    resolve_held_model: Box<dyn FnMut(i32) -> Option<QvmHeldModelRef>>,
    resolve_held_shader: Box<dyn FnMut(i32) -> Option<String>>,
    resolve_body_model: Box<dyn FnMut(i32) -> Option<SceneModel>>,
    resolve_body_shader: Box<dyn FnMut(i32) -> Option<SceneShader>>,
    resolve_body_skin: Box<dyn FnMut(i32) -> Option<SceneSkin>>,
    arguments: Vec<String>,
    retired: bool,
    ready: bool,
    generation: i32,
    cgame_initialized: bool,
    artifacts: Option<QvmPresentationArtifacts>,
    equipment: Option<Q3EquipmentPresentation>,
    equipment_selector: Option<QvmHookToken>,
    equipment_hud_requested: bool,
    equipment_view_visible: bool,
    held_invocations: Vec<HeldWeaponInvocation>,
    held_weapons: Vec<QvmHeldWeapon>,
    held_scope_ids: u64,
}

fn raw_color(rgba: [u8; 4]) -> Vec4 {
    Vec4 {
        x: f32::from(rgba[0]),
        y: f32::from(rgba[1]),
        z: f32::from(rgba[2]),
        w: f32::from(rgba[3]),
    }
}

/// Missing-module loader failure (donor `module()`).
#[must_use]
pub fn missing_native_module(path: &str) -> GuestError {
    GuestError::invalid(format!("Missing native module: {path}"))
}

/// Unsupported-module loader failure (donor `module()`).
#[must_use]
pub fn unsupported_native_module(path: &str) -> GuestError {
    GuestError::invalid(format!("Unsupported native baseq3 module: {path}"))
}

const BRIDGE_STAGES: [QvmBridgeStage; 11] = [
    QvmBridgeStage::Common,
    QvmBridgeStage::File,
    QvmBridgeStage::Script,
    QvmBridgeStage::Render,
    QvmBridgeStage::Audio,
    QvmBridgeStage::State,
    QvmBridgeStage::Collision,
    QvmBridgeStage::Mark,
    QvmBridgeStage::Cinematic,
    QvmBridgeStage::Browser,
    QvmBridgeStage::Key,
];

impl ApplicationQvmClient {
    /// Create and initialize the presentation client.
    pub fn create(mut options: ApplicationQvmClientOptions) -> Result<Self, Q3QvmError> {
        let artifacts = match options.artifacts.take() {
            Some(artifacts) => {
                if artifacts.ui.role != QvmRole::Ui || artifacts.cgame.role != QvmRole::Cgame {
                    return Err(Q3QvmError::WrongArtifact);
                }
                artifacts
            }
            None => {
                let ui = (options.loader)(QvmRole::Ui)?;
                let cgame = (options.loader)(QvmRole::Cgame)?;
                if ui.role != QvmRole::Ui || cgame.role != QvmRole::Cgame {
                    return Err(Q3QvmError::WrongLoadedArtifact);
                }
                QvmPresentationArtifacts { ui, cgame }
            }
        };
        let server_message = options.connection.server_message_sequence();
        let last_command = options.connection.last_executed_server_command();
        let client_number = options.connection.client_number();
        let generation = options.connection.generation();
        let status = CgameStatusView::new(CvarServiceBox(options.cvars), options.status_visible);
        let mut client = Self {
            connection: options.connection,
            status,
            assert_session: options.assert_session,
            bridges: options.bridges,
            modules: options.modules,
            bodies: options.bodies,
            equipment_hooks: options.equipment_hooks,
            scalar: options.scalar,
            now: options.now,
            key_catcher: options.key_catcher,
            content: options.content,
            view_weapon_visible: options.view_weapon_visible,
            equipment_weapon: options.equipment_weapon,
            equipment_boundary: options.equipment_boundary,
            body_boundaries: options.body_boundaries,
            held_weapon_actor: options.held_weapon_actor,
            resolve_held_model: options.resolve_held_model,
            resolve_held_shader: options.resolve_held_shader,
            resolve_body_model: options.resolve_body_model,
            resolve_body_shader: options.resolve_body_shader,
            resolve_body_skin: options.resolve_body_skin,
            arguments: Vec::new(),
            retired: false,
            ready: true,
            generation,
            cgame_initialized: true,
            artifacts: Some(artifacts),
            equipment: None,
            equipment_selector: None,
            equipment_hud_requested: false,
            equipment_view_visible: false,
            held_invocations: Vec::new(),
            held_weapons: Vec::new(),
            held_scope_ids: 1,
        };
        let initialized = client
            .modules
            .init_ui(true)
            .and_then(|()| client.modules.init_cgame(server_message, last_command, client_number));
        if let Err(error) = initialized {
            let error = Q3QvmError::Guest(error);
            return match client.close() {
                Ok(()) => Err(error),
                Err(cleanup) => Err(Q3QvmError::InitCleanup {
                    error: Box::new(error),
                    cleanup: Box::new(cleanup),
                }),
            };
        }
        Ok(client)
    }

    /// Retained presentation artifacts.
    pub fn presentation_artifacts(&self) -> Result<&QvmPresentationArtifacts, Q3QvmError> {
        self.artifacts.as_ref().ok_or(Q3QvmError::PresentationNotInitialized)
    }

    /// Latest server-command argv installed for the guest.
    #[must_use]
    pub fn arguments(&self) -> &[String] {
        &self.arguments
    }

    /// Shared held weapons.
    #[must_use]
    pub fn shared_held_weapons(&self) -> &[QvmHeldWeapon] {
        &self.held_weapons
    }

    /// Whether the last frame requested the equipment HUD.
    #[must_use]
    pub fn shared_equipment_hud(&self) -> bool {
        self.equipment_hud_requested
    }

    /// Whether the last frame kept the equipment view visible.
    #[must_use]
    pub fn shared_equipment_view_visible(&self) -> bool {
        self.equipment_view_visible
    }

    fn assert_current(&mut self) -> Result<(), Q3QvmError> {
        (self.assert_session)();
        if self.retired || self.connection.generation() != self.generation {
            return Err(Q3QvmError::Retired);
        }
        Ok(())
    }

    fn assert_guest(&mut self) -> Result<(), GuestError> {
        self.assert_current()
            .map_err(|error| GuestError::runtime(error.to_string()))
    }

    /// Adapt a user command to the selected weapon presentation.
    #[must_use]
    pub fn adapt_user_command(&self, command: &NetUserCommand) -> NetUserCommand {
        q3_equipment_command(command, self.equipment.as_ref())
    }

    /// Install server-command argv for the guest, returning it.
    pub fn install_server_command(
        &mut self,
        argv: Option<Vec<String>>,
        profile: AbiProfile,
    ) -> Result<Option<Vec<String>>, GuestError> {
        self.assert_guest()?;
        if let Some(argv) = argv {
            self.arguments = legacy_client_command(&argv, profile)?;
            Ok(Some(argv))
        } else {
            Ok(None)
        }
    }

    /// Dispatch a guest trap.
    pub fn host(&mut self, call: &HostCall, memory: &mut SyscallMemory) -> Result<i32, GuestError> {
        self.assert_guest()?;
        if call.kind == CallKind::Engine
            && call.role == CallRole::Cgame
            && call.code == QvmCgameImport::CgRAddrefentitytoscene as i32
            && !self.held_invocations.is_empty()
        {
            let invocation = self.held_invocations.last().expect("held scope");
            let pointer = call.int(1)?;
            let frame = invocation.frame;
            let end = invocation.end;
            let gun = invocation.gun;
            if pointer >= 0 && (pointer as usize) >= frame && (pointer as usize) + QVM_REF_ENTITY_BYTES <= end {
                if (pointer as usize) == gun {
                    let base = memory
                        .pointer(pointer)
                        .ok_or_else(|| GuestError::invalid("refentity record requires a pointer"))?;
                    memory.span(pointer, QVM_REF_ENTITY_BYTES, 0)?;
                    let bytes = memory.read_bytes(base, QVM_REF_ENTITY_BYTES)?.to_vec();
                    let reference = read_qvm_ref_entity(&bytes)?;
                    let shader = (self.resolve_held_shader)(reference.custom_shader);
                    let scope = self.held_invocations.last_mut().expect("held scope");
                    scope.held.passes.push(QvmHeldPass {
                        shader,
                        color: raw_color(reference.shader_rgba),
                        shader_time: reference.shader_time,
                    });
                }
                return Ok(0);
            }
        }
        if self.bodies.suppress_body(call) {
            return Ok(0);
        }
        if call.role != CallRole::Cgame && call.role != CallRole::Ui {
            return Err(GuestError::invalid(format!(
                "QVM presentation client rejects {:?} syscalls",
                call.role
            )));
        }
        if call.role == CallRole::Cgame {
            if let Some(handled) = self.status.syscall(call, memory)? {
                return Ok(handled);
            }
        }
        for stage in BRIDGE_STAGES {
            if let Some(handled) = self.bridges.dispatch_bridge(stage, call, memory)? {
                return Ok(handled);
            }
        }
        match self.scalar.dispatch_scalar(call, memory)? {
            QvmScalarResult::Handled(value) => Ok(value),
            QvmScalarResult::UpdateScreen => {
                self.update_screen(call)?;
                self.assert_guest()?;
                Ok(0)
            }
            QvmScalarResult::Unhandled => Err(GuestError::invalid(format!(
                "QVM presentation client rejects {:?} syscalls",
                call.role
            ))),
        }
    }

    /// Begin a held-weapon capture scope, or `None` to proceed plainly.
    pub fn begin_held_weapon(&mut self, scope: QvmHeldScope) -> Result<Option<u64>, Q3QvmError> {
        if self.retired {
            return Err(Q3QvmError::StaleAttachment);
        }
        if scope.state != 0 {
            return Ok(None);
        }
        let actor = self
            .held_weapon_actor
            .as_mut()
            .and_then(|resolve| resolve(scope.entity_number));
        let Some(actor) = actor else {
            return Ok(None);
        };
        let reference = read_qvm_ref_entity(&scope.parent_record)?;
        if reference.kind != QvmRefEntityKind::Model {
            return Err(Q3QvmError::ParentNotModel);
        }
        let Some(model) = (self.resolve_held_model)(reference.model) else {
            return Err(Q3QvmError::ParentNoModel);
        };
        let id = self.held_scope_ids;
        self.held_scope_ids += 1;
        self.held_invocations.push(HeldWeaponInvocation {
            id,
            frame: scope.frame,
            end: scope.end,
            gun: scope.gun,
            held: QvmHeldWeapon {
                content: self.content.clone(),
                parent: QvmHeldParent {
                    actor,
                    entity_number: scope.entity_number,
                    model: model.path,
                    resource: model.resource,
                    origin: reference.origin,
                    axis: reference.axis,
                    old_origin: reference.old_origin,
                    frame: reference.frame,
                    old_frame: reference.old_frame,
                    back_lerp: reference.back_lerp,
                    skin: reference.skin_num,
                    shader_time: reference.shader_time,
                    lighting_origin: reference.lighting_origin,
                    shadow_plane: reference.shadow_plane,
                    render_flags: reference.render_flags,
                },
                passes: Vec::new(),
            },
        });
        Ok(Some(id))
    }

    /// End a capture scope, publishing weapons with passes.
    pub fn end_held_weapon(&mut self, token: u64) -> Result<(), Q3QvmError> {
        if self.retired {
            return Err(Q3QvmError::StaleAttachment);
        }
        match self.held_invocations.pop() {
            Some(invocation) if invocation.id == token => {
                if !invocation.held.passes.is_empty() {
                    self.held_weapons.push(invocation.held);
                }
                Ok(())
            }
            Some(invocation) => {
                self.held_invocations.push(invocation);
                Err(Q3QvmError::ScopeOrder)
            }
            None => Err(Q3QvmError::ScopeOrder),
        }
    }

    /// Resolve a body source and offer it for capture.
    pub fn submit_body(
        &mut self,
        entity: i32,
        part: QvmBodyPart,
        source: &QvmRefEntity,
        base: bool,
    ) -> Result<bool, Q3QvmError> {
        self.assert_current()?;
        if source.kind != QvmRefEntityKind::Model {
            return Err(Q3QvmError::BodyNotModel);
        }
        let Some(model) = (self.resolve_body_model)(source.model) else {
            return Err(Q3QvmError::BodyNoModel);
        };
        let custom_shader = (self.resolve_body_shader)(source.custom_shader);
        let custom_skin = (self.resolve_body_skin)(source.custom_skin);
        let resolved = RefModelEntity {
            shading: ShadedFields {
                render_flags: source.render_flags,
                custom_shader,
                shader_rgba: raw_color(source.shader_rgba),
                shader_tex_coord: source.shader_tex_coord,
                shader_time: source.shader_time,
            },
            model,
            origin: source.origin,
            old_origin: source.old_origin,
            axis: source.axis,
            non_normalized_axes: source.non_normalized_axes,
            lighting_origin: source.lighting_origin,
            shadow_plane: source.shadow_plane,
            frame: source.frame,
            old_frame: source.old_frame,
            back_lerp: source.back_lerp,
            skin_num: source.skin_num,
            custom_skin,
        };
        if !self.bodies.body_capture_selected(entity) {
            return Ok(false);
        }
        Ok(self.bodies.body_capture_submit(entity, part, resolved, base))
    }

    /// Refresh tracked status registrations.
    pub fn refresh_status(&mut self, memory: &mut SyscallMemory) -> Result<(), GuestError> {
        if self.retired {
            return Ok(());
        }
        self.assert_guest()?;
        self.status.refresh(memory)
    }

    /// Run the screen update for an update-screen trap.
    pub fn update_screen(&mut self, call: &HostCall) -> Result<(), GuestError> {
        self.assert_guest()?;
        if call.role == CallRole::Ui {
            self.modules.invoke_connect_screen()
        } else {
            self.modules.draw_connect_screen(true)
        }
    }

    /// Draw one frame.
    pub fn draw(&mut self, time_ms: i32, demo_playback: bool) -> Result<(), Q3QvmError> {
        self.assert_current()?;
        if !self.ready || !self.cgame_initialized {
            return Err(Q3QvmError::NotInitialized);
        }
        let hide = self.bodies.body_overrides_active();
        let capture = self.bodies.body_capture_active();
        if (hide || capture) && self.body_boundaries.is_none_or(|body| !body.declarations) {
            return Err(Q3QvmError::BodyDeclarations);
        }
        if capture && self.body_boundaries.is_none_or(|body| !body.captures_player_meshes) {
            return Err(Q3QvmError::BodyMeshes);
        }
        self.bodies.enable_bodies(hide || capture)?;
        if self.view_weapon_visible.as_mut().is_some_and(|visible| !visible())
            && self.equipment_boundary.is_none_or(|boundary| !boundary.view)
        {
            return Err(Q3QvmError::ViewBoundary);
        }
        self.equipment = self.equipment_weapon.as_mut().and_then(|weapon| weapon());
        if self.equipment.is_some() && self.equipment_boundary.is_none_or(|boundary| !boundary.hud) {
            return Err(Q3QvmError::HudBoundary);
        }
        if self.equipment.is_some() {
            if self.equipment_selector.is_none() {
                self.equipment_selector = Some(self.equipment_hooks.install_selector());
            }
        } else if let Some(token) = self.equipment_selector.take() {
            self.equipment_hooks.remove_selector(token);
        }
        self.equipment_hud_requested = false;
        self.equipment_view_visible = false;
        self.held_weapons.clear();
        let mut notes = QvmEquipmentNotes::default();
        self.modules.draw_cgame_frame(time_ms, demo_playback, &mut notes)?;
        self.equipment_hud_requested = notes.hud_requested;
        self.equipment_view_visible = notes.view_visible;
        if (self.key_catcher)() & 2 != 0 {
            let now = (self.now)();
            self.modules.refresh_ui(now)?;
        }
        Ok(())
    }

    /// Run a console command through cgame, then ui.
    pub fn command(&mut self, argv: &[String]) -> Result<bool, Q3QvmError> {
        self.assert_current()?;
        if self.modules.console_command(QvmRole::Cgame, None, argv)? {
            return Ok(true);
        }
        let now = (self.now)();
        Ok(self.modules.console_command(QvmRole::Ui, Some(now), argv)?)
    }

    /// Deliver a key event to the catching module.
    pub fn key_event(&mut self, key: i32, down: bool) -> Result<(), Q3QvmError> {
        self.assert_current()?;
        if (self.key_catcher)() & 2 != 0 {
            self.modules.key_event(QvmRole::Ui, key, down)?;
        } else if self.modules.supports_input_events() {
            self.modules.key_event(QvmRole::Cgame, key, down)?;
        }
        Ok(())
    }

    /// Deliver a mouse event to the catching module.
    pub fn mouse_event(&mut self, x: i32, y: i32) -> Result<(), Q3QvmError> {
        self.assert_current()?;
        if (self.key_catcher)() & 2 != 0 {
            self.modules.mouse_event(QvmRole::Ui, x, y)?;
        } else if self.modules.supports_input_events() {
            self.modules.mouse_event(QvmRole::Cgame, x, y)?;
        }
        Ok(())
    }

    /// Whether a module captures input.
    #[must_use]
    pub fn captures_input(&self) -> bool {
        let catcher = (self.key_catcher)();
        if self.modules.supports_input_events() {
            catcher != 0
        } else {
            catcher & 2 != 0
        }
    }

    /// Switch cgame event handling.
    pub fn event_handling(&mut self, mode: QvmEventHandling) -> Result<(), Q3QvmError> {
        self.assert_current()?;
        if mode == QvmEventHandling::None && !self.modules.supports_input_events() {
            return Ok(());
        }
        self.modules.event_handling(mode)?;
        Ok(())
    }

    /// Shut both modules down, then close.
    pub fn shutdown(&mut self) -> Result<(), Q3QvmError> {
        if self.retired {
            return Ok(());
        }
        let body = self.modules.shutdown_cgame().and_then(|()| self.modules.shutdown_ui());
        let closed = self.close();
        closed?;
        body?;
        Ok(())
    }

    /// Retire the client, releasing hooks, files, and modules.
    pub fn close(&mut self) -> Result<(), Q3QvmError> {
        if self.retired {
            return Ok(());
        }
        self.retired = true;
        self.ready = false;
        self.bodies.close_bodies();
        if let Some(token) = self.equipment_selector.take() {
            self.equipment_hooks.remove_selector(token);
        }
        self.modules.retire_modules();
        let closed = self.bridges.close_bridges().map_err(Q3QvmError::Cleanup);
        self.held_invocations.clear();
        self.held_weapons.clear();
        self.equipment = None;
        self.equipment_hud_requested = false;
        self.equipment_view_visible = false;
        closed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_client::ui::types::ArsenalAmmoWarning;
    use qa_core::identity::IdentityOwner;
    use qa_guest::qvm::cvar_syscalls::{CvarValue, CvarVmBinding};
    use qa_guest::qvm::game_data::ModuleIdentity;
    use qa_guest::qvm::game_data::QvmImage;

    struct StubConnection {
        generation: i32,
        commands: Vec<String>,
    }

    impl StubConnection {
        fn new() -> Self {
            Self {
                generation: 1,
                commands: vec!["print".to_string(), "hello".to_string()],
            }
        }
    }

    impl QvmClientConnection for StubConnection {
        fn generation(&self) -> i32 {
            self.generation
        }
        fn server_message_sequence(&self) -> i32 {
            10
        }
        fn last_executed_server_command(&self) -> i32 {
            4
        }
        fn client_number(&self) -> i32 {
            2
        }
        fn commands_current_number(&mut self) -> Result<i32, Q3QvmError> {
            Ok(1)
        }
        fn commands_read(&mut self, _number: i32) -> Result<Option<NetUserCommand>, Q3QvmError> {
            Ok(Some(NetUserCommand::default()))
        }
        fn snapshot_ping(&mut self, _number: i32) -> Result<Option<i32>, Q3QvmError> {
            Ok(Some(20))
        }
        fn get_server_command(&mut self, _number: i32) -> Result<Option<Vec<String>>, Q3QvmError> {
            Ok(Some(self.commands.clone()))
        }
    }

    struct StubCvars;

    impl CvarHost for StubCvars {
        fn bind_vm(&mut self, _name: &str, _default: &str, _flags: i32) -> i32 {
            0
        }
        fn read_vm(&mut self, _handle: i32) -> Option<CvarVmBinding> {
            None
        }
        fn get(&mut self, _name: &str) -> Option<CvarValue> {
            None
        }
        fn set(&mut self, _name: &str, _value: &str) {}
        fn set_value(&mut self, _name: &str, _value: f32) {}
        fn reset(&mut self, _name: &str) {}
        fn register(&mut self, _name: &str, _default: &str, _flags: i32) {}
        fn info_string(&mut self, _flags: i32) -> String {
            String::new()
        }
    }

    struct StubBridges {
        stages: Vec<QvmBridgeStage>,
    }

    impl StubBridges {
        fn new() -> Self {
            Self { stages: Vec::new() }
        }
    }

    impl QvmClientBridges for StubBridges {
        fn dispatch_bridge(
            &mut self,
            stage: QvmBridgeStage,
            _call: &HostCall,
            _memory: &mut SyscallMemory,
        ) -> Result<Option<i32>, GuestError> {
            self.stages.push(stage);
            Ok(None)
        }

        fn close_bridges(&mut self) -> Result<(), String> {
            Ok(())
        }
    }

    struct StubModules {
        supports_input: bool,
        frames: usize,
        refreshed: Vec<i32>,
        connect_invocations: usize,
        connect_screens: Vec<bool>,
        commands: Vec<(QvmRole, Vec<String>)>,
        keys: Vec<(QvmRole, i32, bool)>,
        mice: Vec<(QvmRole, i32, i32)>,
        modes: Vec<QvmEventHandling>,
        shutdowns: Vec<QvmRole>,
        retired: bool,
        hud_notes: bool,
        fail_init: bool,
    }

    impl StubModules {
        fn new() -> Self {
            Self {
                supports_input: true,
                frames: 0,
                refreshed: Vec::new(),
                connect_invocations: 0,
                connect_screens: Vec::new(),
                commands: Vec::new(),
                keys: Vec::new(),
                mice: Vec::new(),
                modes: Vec::new(),
                shutdowns: Vec::new(),
                retired: false,
                hud_notes: false,
                fail_init: false,
            }
        }
    }

    impl QvmPresentationModules for StubModules {
        fn init_ui(&mut self, connecting: bool) -> Result<(), GuestError> {
            assert!(connecting);
            if self.fail_init {
                return Err(GuestError::invalid("init failed"));
            }
            Ok(())
        }

        fn init_cgame(&mut self, server_message: i32, last_command: i32, client_number: i32) -> Result<(), GuestError> {
            assert_eq!((server_message, last_command, client_number), (10, 4, 2));
            Ok(())
        }

        fn draw_cgame_frame(
            &mut self,
            _time_ms: i32,
            _demo_playback: bool,
            notes: &mut QvmEquipmentNotes,
        ) -> Result<(), GuestError> {
            self.frames += 1;
            notes.hud_requested = self.hud_notes;
            notes.view_visible = self.hud_notes;
            Ok(())
        }

        fn refresh_ui(&mut self, time_ms: i32) -> Result<(), GuestError> {
            self.refreshed.push(time_ms);
            Ok(())
        }

        fn invoke_connect_screen(&mut self) -> Result<(), GuestError> {
            self.connect_invocations += 1;
            Ok(())
        }

        fn draw_connect_screen(&mut self, overlay: bool) -> Result<(), GuestError> {
            self.connect_screens.push(overlay);
            Ok(())
        }

        fn console_command(
            &mut self,
            role: QvmRole,
            time_ms: Option<i32>,
            argv: &[String],
        ) -> Result<bool, GuestError> {
            self.commands.push((role, argv.to_vec()));
            Ok(role == QvmRole::Ui && time_ms.is_some())
        }

        fn key_event(&mut self, role: QvmRole, key: i32, down: bool) -> Result<(), GuestError> {
            self.keys.push((role, key, down));
            Ok(())
        }

        fn mouse_event(&mut self, role: QvmRole, x: i32, y: i32) -> Result<(), GuestError> {
            self.mice.push((role, x, y));
            Ok(())
        }

        fn event_handling(&mut self, mode: QvmEventHandling) -> Result<(), GuestError> {
            self.modes.push(mode);
            Ok(())
        }

        fn shutdown_cgame(&mut self) -> Result<(), GuestError> {
            self.shutdowns.push(QvmRole::Cgame);
            Ok(())
        }

        fn shutdown_ui(&mut self) -> Result<(), GuestError> {
            self.shutdowns.push(QvmRole::Ui);
            Ok(())
        }

        fn retire_modules(&mut self) {
            self.retired = true;
        }

        fn supports_input_events(&self) -> bool {
            self.supports_input
        }
    }

    struct StubBodies {
        enabled: Vec<bool>,
        suppressions: usize,
        capture: bool,
        overrides: bool,
        submitted: Vec<(i32, QvmBodyPart)>,
    }

    impl StubBodies {
        fn new() -> Self {
            Self {
                enabled: Vec::new(),
                suppressions: 0,
                capture: false,
                overrides: false,
                submitted: Vec::new(),
            }
        }
    }

    impl QvmBodyPolicy for StubBodies {
        fn enable_bodies(&mut self, active: bool) -> Result<(), GuestError> {
            self.enabled.push(active);
            Ok(())
        }

        fn close_bodies(&mut self) {}

        fn suppress_body(&mut self, _call: &HostCall) -> bool {
            self.suppressions += 1;
            false
        }

        fn body_capture_active(&mut self) -> bool {
            self.capture
        }

        fn body_capture_selected(&mut self, _entity: i32) -> bool {
            true
        }

        fn body_capture_submit(
            &mut self,
            entity: i32,
            part: QvmBodyPart,
            _source: RefModelEntity,
            _base: bool,
        ) -> bool {
            self.submitted.push((entity, part));
            true
        }

        fn body_overrides_active(&mut self) -> bool {
            self.overrides
        }

        fn body_hidden(&mut self, _entity: i32) -> bool {
            false
        }
    }

    struct StubHooks {
        installed: usize,
        removed: Vec<QvmHookToken>,
        next: u64,
    }

    impl StubHooks {
        fn new() -> Self {
            Self {
                installed: 0,
                removed: Vec::new(),
                next: 1,
            }
        }
    }

    impl QvmEquipmentHooks for StubHooks {
        fn install_selector(&mut self) -> QvmHookToken {
            self.installed += 1;
            let token = QvmHookToken(self.next);
            self.next += 1;
            token
        }

        fn remove_selector(&mut self, token: QvmHookToken) {
            self.removed.push(token);
        }
    }

    struct StubScalar {
        result: QvmScalarResult,
    }

    impl QvmScalarStage for StubScalar {
        fn dispatch_scalar(
            &mut self,
            _call: &HostCall,
            _memory: &mut SyscallMemory,
        ) -> Result<QvmScalarResult, GuestError> {
            Ok(self.result)
        }
    }

    fn artifact(role: QvmRole) -> QvmArtifact {
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

    fn options() -> ApplicationQvmClientOptions {
        let registry = IdentityOwner::create("qvm-test").expect("owner");
        let actor = registry.actor(9, 1);
        ApplicationQvmClientOptions {
            connection: Box::new(StubConnection::new()),
            cvars: Box::new(StubCvars),
            status_visible: Box::new(|| true),
            assert_session: Box::new(|| {}),
            bridges: Box::new(StubBridges::new()),
            modules: Box::new(StubModules::new()),
            bodies: Box::new(StubBodies::new()),
            equipment_hooks: Box::new(StubHooks::new()),
            scalar: Box::new(StubScalar {
                result: QvmScalarResult::Handled(7),
            }),
            loader: Box::new(|role| Ok(artifact(role))),
            artifacts: None,
            now: Box::new(|| 100),
            key_catcher: Box::new(|| 0),
            content: "q3-test".to_string(),
            view_weapon_visible: None,
            equipment_weapon: None,
            equipment_boundary: None,
            body_boundaries: None,
            held_weapon_actor: Some(Box::new(move |_| Some(actor.clone()))),
            resolve_held_model: Box::new(|_| {
                Some(QvmHeldModelRef {
                    path: "models/gun.md3".to_string(),
                    resource: "pak0:gun".to_string(),
                })
            }),
            resolve_held_shader: Box::new(|handle| (handle != 0).then(|| format!("shader{handle}"))),
            resolve_body_model: Box::new(|_| Some(SceneModel::default_model())),
            resolve_body_shader: Box::new(|handle| (handle != 0).then(|| SceneShader::new(format!("s{handle}")))),
            resolve_body_skin: Box::new(|_| None),
        }
    }

    fn memory() -> SyscallMemory {
        SyscallMemory::new(65536).expect("memory")
    }

    fn parent_record() -> Vec<u8> {
        let mut record = vec![0u8; QVM_REF_ENTITY_BYTES];
        // kind 0 (model), model handle 3, frame 5, shader time 250ms.
        record[8..12].copy_from_slice(&3i32.to_le_bytes());
        record[80..84].copy_from_slice(&5i32.to_le_bytes());
        record[128..132].copy_from_slice(&250f32.to_le_bytes());
        record
    }

    #[test]
    fn create_initializes_modules_and_artifacts() {
        let client = ApplicationQvmClient::create(options()).expect("create");
        let artifacts = client.presentation_artifacts().expect("artifacts");
        assert_eq!(artifacts.ui.role, QvmRole::Ui);
        assert_eq!(artifacts.cgame.role, QvmRole::Cgame);
        assert!(client.arguments().is_empty());
    }

    #[test]
    fn create_rejects_mismatched_retained_role() {
        let mut opts = options();
        opts.artifacts = Some(QvmPresentationArtifacts {
            ui: artifact(QvmRole::Cgame),
            cgame: artifact(QvmRole::Cgame),
        });
        assert_eq!(
            ApplicationQvmClient::create(opts).err(),
            Some(Q3QvmError::WrongArtifact)
        );
    }

    #[test]
    fn server_command_installs_legacy_arguments() {
        let mut client = ApplicationQvmClient::create(options()).expect("create");
        let argv = client
            .install_server_command(
                Some(vec!["cs".to_string(), "1".to_string(), "x".to_string()]),
                AbiProfile::Legacy,
            )
            .expect("install")
            .expect("argv");
        assert_eq!(argv[0], "cs");
        assert!(!client.arguments().is_empty());
        assert!(client
            .install_server_command(None, AbiProfile::Modern)
            .expect("install")
            .is_none());
    }

    #[test]
    fn host_walks_bridge_stages_then_scalar() {
        let mut client = ApplicationQvmClient::create(options()).expect("create");
        let mut memory = memory();
        let call = HostCall::engine(CallRole::Ui, 999, &[], AbiProfile::Modern);
        assert_eq!(client.host(&call, &mut memory).expect("host"), 7);
        let game = HostCall::engine(CallRole::Qagame, 999, &[], AbiProfile::Modern);
        assert!(client.host(&game, &mut memory).is_err());
    }

    #[test]
    fn update_screen_result_runs_connect_paths() {
        let mut opts = options();
        opts.scalar = Box::new(StubScalar {
            result: QvmScalarResult::UpdateScreen,
        });
        let mut client = ApplicationQvmClient::create(opts).expect("create");
        let mut memory = memory();
        let ui = HostCall::engine(CallRole::Ui, 999, &[], AbiProfile::Modern);
        assert_eq!(client.host(&ui, &mut memory).expect("host"), 0);
        let cgame = HostCall::engine(CallRole::Cgame, 999, &[], AbiProfile::Modern);
        assert_eq!(client.host(&cgame, &mut memory).expect("host"), 0);
    }

    #[test]
    fn held_weapon_capture_records_gun_passes() {
        let mut client = ApplicationQvmClient::create(options()).expect("create");
        let mut memory = memory();
        // Gun record at 1024 with a red pass and custom shader 9.
        let base = memory.pointer(1024).expect("gun");
        let mut gun = vec![0u8; QVM_REF_ENTITY_BYTES];
        gun[8..12].copy_from_slice(&3i32.to_le_bytes());
        gun[112..116].copy_from_slice(&9i32.to_le_bytes());
        gun[116..120].copy_from_slice(&[255, 0, 0, 255]);
        gun[128..132].copy_from_slice(&500f32.to_le_bytes());
        memory.write_bytes(base, &gun).expect("gun");
        let token = client
            .begin_held_weapon(QvmHeldScope {
                frame: 1024,
                end: 1024 + 2 * QVM_REF_ENTITY_BYTES,
                gun: 1024,
                state: 0,
                entity_number: 9,
                parent_record: parent_record(),
            })
            .expect("begin")
            .expect("scope");
        // A record outside the scope passes through to the stages.
        let other = HostCall::engine(
            CallRole::Cgame,
            QvmCgameImport::CgRAddrefentitytoscene as i32,
            &[2048],
            AbiProfile::Modern,
        );
        assert_eq!(client.host(&other, &mut memory).expect("host"), 7);
        // A non-gun record inside the scope is swallowed without a pass.
        let sibling = HostCall::engine(
            CallRole::Cgame,
            QvmCgameImport::CgRAddrefentitytoscene as i32,
            &[1024 + QVM_REF_ENTITY_BYTES as i32],
            AbiProfile::Modern,
        );
        assert_eq!(client.host(&sibling, &mut memory).expect("host"), 0);
        let gun_call = HostCall::engine(
            CallRole::Cgame,
            QvmCgameImport::CgRAddrefentitytoscene as i32,
            &[1024],
            AbiProfile::Modern,
        );
        assert_eq!(client.host(&gun_call, &mut memory).expect("host"), 0);
        client.end_held_weapon(token).expect("end");
        assert_eq!(client.shared_held_weapons().len(), 1);
        let held = &client.shared_held_weapons()[0];
        assert_eq!(held.parent.entity_number, 9);
        assert_eq!(held.passes.len(), 1);
        assert_eq!(held.passes[0].shader, Some("shader9".to_string()));
        assert_eq!(held.passes[0].shader_time, 500.0);
        assert_eq!(client.end_held_weapon(token).unwrap_err(), Q3QvmError::ScopeOrder);
    }

    #[test]
    fn held_weapon_skips_without_state_or_actor() {
        let mut opts = options();
        opts.held_weapon_actor = None;
        let mut client = ApplicationQvmClient::create(opts).expect("create");
        let skipped = client
            .begin_held_weapon(QvmHeldScope {
                frame: 0,
                end: 140,
                gun: 0,
                state: 1,
                entity_number: 9,
                parent_record: parent_record(),
            })
            .expect("begin");
        assert_eq!(skipped, None);
        let skipped = client
            .begin_held_weapon(QvmHeldScope {
                frame: 0,
                end: 140,
                gun: 0,
                state: 0,
                entity_number: 9,
                parent_record: parent_record(),
            })
            .expect("begin");
        assert_eq!(skipped, None);
    }

    #[test]
    fn body_submit_resolves_and_forwards() {
        let mut client = ApplicationQvmClient::create(options()).expect("create");
        let source = read_qvm_ref_entity(&parent_record()).expect("record");
        assert!(client.submit_body(9, QvmBodyPart::Head, &source, true).expect("submit"));
        let mut sprite = vec![0u8; QVM_REF_ENTITY_BYTES];
        sprite[0..4].copy_from_slice(&2i32.to_le_bytes());
        let sprite = read_qvm_ref_entity(&sprite).expect("record");
        assert_eq!(
            client.submit_body(9, QvmBodyPart::Body, &sprite, false).unwrap_err(),
            Q3QvmError::BodyNotModel
        );
    }

    #[test]
    fn draw_gates_boundaries_and_routes_ui_refresh() {
        let mut opts = options();
        opts.key_catcher = Box::new(|| 2);
        let mut client = ApplicationQvmClient::create(opts).expect("create");
        client.draw(50, false).expect("draw");
        assert!(!client.shared_equipment_hud());
        assert!(client.command(&["say".to_string(), "hi".to_string()]).expect("command"));
        client.key_event(65, true).expect("key");
        client.mouse_event(3, 4).expect("mouse");
        assert!(client.captures_input());
        client.event_handling(QvmEventHandling::Scoreboard).expect("event");
        client.event_handling(QvmEventHandling::None).expect("event");
    }

    #[test]
    fn draw_rejects_undeclared_overrides() {
        let mut client = ApplicationQvmClient::create(options()).expect("create");
        client.bodies = Box::new(StubBodies {
            enabled: Vec::new(),
            suppressions: 0,
            capture: true,
            overrides: false,
            submitted: Vec::new(),
        });
        assert_eq!(client.draw(50, false).unwrap_err(), Q3QvmError::BodyDeclarations);
    }

    #[test]
    fn draw_installs_equipment_selector() {
        let mut opts = options();
        opts.equipment_boundary = Some(QvmEquipmentBoundary {
            hud: true,
            view: true,
            status: false,
            warning: false,
            held: true,
        });
        opts.equipment_weapon = Some(Box::new(|| {
            Some(Q3EquipmentPresentation {
                primary_weapon: 2,
                warning: ArsenalAmmoWarning::None,
            })
        }));
        let mut client = ApplicationQvmClient::create(opts).expect("create");
        client.draw(50, false).expect("draw");
        client.close().expect("close");
        assert_eq!(client.draw(60, false).unwrap_err(), Q3QvmError::Retired);
    }

    #[test]
    fn shutdown_runs_both_modules_then_closes() {
        let mut client = ApplicationQvmClient::create(options()).expect("create");
        client.shutdown().expect("shutdown");
        client.shutdown().expect("shutdown");
        assert!(client
            .host(
                &HostCall::engine(CallRole::Ui, 1, &[], AbiProfile::Modern),
                &mut memory()
            )
            .is_err());
    }

    #[test]
    fn create_cleans_up_failed_init() {
        struct FailBridges;
        impl QvmClientBridges for FailBridges {
            fn dispatch_bridge(
                &mut self,
                _stage: QvmBridgeStage,
                _call: &HostCall,
                _memory: &mut SyscallMemory,
            ) -> Result<Option<i32>, GuestError> {
                Ok(None)
            }

            fn close_bridges(&mut self) -> Result<(), String> {
                Err("files busy".to_string())
            }
        }

        let mut opts = options();
        let mut modules = StubModules::new();
        modules.fail_init = true;
        opts.modules = Box::new(modules);
        assert!(matches!(
            ApplicationQvmClient::create(opts).err(),
            Some(Q3QvmError::Guest(_))
        ));

        let mut opts = options();
        let mut modules = StubModules::new();
        modules.fail_init = true;
        opts.modules = Box::new(modules);
        opts.bridges = Box::new(FailBridges);
        assert!(matches!(
            ApplicationQvmClient::create(opts).err(),
            Some(Q3QvmError::InitCleanup { .. })
        ));
    }

    #[test]
    fn refresh_status_skips_retired_clients() {
        let mut client = ApplicationQvmClient::create(options()).expect("create");
        client.refresh_status(&mut memory()).expect("refresh");
        client.close().expect("close");
        client.refresh_status(&mut memory()).expect("refresh");
    }

    #[test]
    fn loader_failures_use_donor_messages() {
        assert_eq!(
            missing_native_module("vm/cgame.qvm").to_string(),
            "Missing native module: vm/cgame.qvm"
        );
        assert_eq!(
            unsupported_native_module("vm/ui.qvm").to_string(),
            "Unsupported native baseq3 module: vm/ui.qvm"
        );
    }

    #[test]
    fn client_commands_wrap_script_origin() {
        struct Buffer {
            calls: Vec<(String, String, String)>,
        }

        impl QvmCommandBuffer for Buffer {
            fn execute_now(&mut self, text: &str, context: &CommandContext) {
                self.calls
                    .push(("now".to_string(), text.to_string(), script_name(context)));
            }

            fn insert(&mut self, text: &str, context: &CommandContext) {
                self.calls
                    .push(("insert".to_string(), text.to_string(), script_name(context)));
            }

            fn append(&mut self, text: &str, context: &CommandContext) {
                self.calls
                    .push(("append".to_string(), text.to_string(), script_name(context)));
            }
        }

        fn script_name(context: &CommandContext) -> String {
            match &context.origin {
                CommandOrigin::Script { name, .. } => name.clone(),
                _ => "other".to_string(),
            }
        }

        let registry = IdentityOwner::create("qvm-command-test").expect("owner");
        let context = CommandContext::new(registry.session().clone(), CommandOrigin::LocalConsole);
        let mut buffer = Buffer { calls: Vec::new() };
        let mut commands = qvm_client_commands(&mut buffer, &context, QvmRole::Cgame);
        commands.execute_now("a");
        commands.insert("b");
        commands.append("c");
        drop(commands);
        assert_eq!(
            buffer.calls,
            vec![
                ("now".to_string(), "a".to_string(), "q3-cgame".to_string()),
                ("insert".to_string(), "b".to_string(), "q3-cgame".to_string()),
                ("append".to_string(), "c".to_string(), "q3-cgame".to_string()),
            ]
        );
    }

    #[test]
    fn equipment_adapts_user_commands() {
        let client = ApplicationQvmClient::create(options()).expect("create");
        let adapted = client.adapt_user_command(&NetUserCommand::default());
        assert_eq!(adapted, NetUserCommand::default());
    }
}
