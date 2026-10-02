//! Local-seat input orchestration: seats, consoles, bindings, and command builders.
//!
//! Port of donor `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/input.ts`
//! ([`LocalPlayer`], [`LocalInput`], [`ApplicationInputCommands`],
//! [`ApplicationInputUi`], [`Q3CommandSelection`], [`movement_dialect`],
//! [`ApplicationInputCommandOwner`], [`ApplicationInput`]).
//!
//! Ownership adaptations (behavior-preserving; forced by Rust aliasing rules):
//!
//! * The port is synchronous. Async settings reads, haptic loads, and controller
//!   settling fold into the synchronous canonical stores, matching the sibling
//!   bootstrap ports.
//! * Seats live inside the owned [`InputRouter`]; [`LocalInput`] carries the seat id
//!   plus its owned console, builder, and haptics, and seat operations resolve the
//!   seat through the router.
//! * Cvar registries are shared cells ([`SharedRegistry`]). Console routing
//!   ([`ApplicationConsoleRouting`]) borrows registries, so it is constructed per
//!   call from live borrows instead of stored.
//! * Staged candidate commands are an input-owned [`StagedClientCommands`] built on
//!   the canonical buffer/binding primitives. The canonical
//!   [`PreparedClientCommands`](crate::bootstrap::prepared_startup::PreparedClientCommands)
//!   borrows its seats mutably, so it cannot be held alongside router-owned seats,
//!   and the startup method stages the startup seats rather than the input seats.
//! * Consumers the input lane does not own (startup, profile configuration, client,
//!   window, controllers, scripts, actions, UI) arrive as trait objects behind the
//!   narrow contracts in this module, following the sibling-lane idiom.

use std::cell::{RefCell, RefMut};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::rc::Rc;

use qa_client::audio::types::AudioAudience;
use qa_client::input::bindings::{
    default_bindings, register_binding_commands, register_input_commands, register_wheel_commands, BindingLookup,
    ButtonSeat, ButtonSeatLookup, ClientScoresFn, PrintFn, WheelMode,
};
use qa_client::input::commands::{
    ClientCommandBindings, ClientCommandHandler, ClientCommandOwner, CommandHandler as ClientRegistryHandler,
    CommandInvocation, CommandOrigin as ClientOrigin, CommandRegistry, CommandsError,
};
use qa_client::input::gamepad::GamepadTuning;
use qa_client::input::haptics::{HapticError, SeatHaptics};
use qa_client::input::midi::MidiInputBoundary;
use qa_client::input::mouse_settings::{read_mouse_tuning, register_mouse_settings, write_mouse_tuning};
use qa_client::input::router::{
    InputRouter, RouterControllers, RouterError, RouterWindow, Seat, SeatError, SeatFrame,
    SeatInputEvent as RouterSeatEvent, SeatRoute, UiCallback, UnhandledEvent,
};
use qa_client::input::source::JoystickOpener;
use qa_client::input::weapons::WeaponBindingItem;
use qa_client::input::{
    BuiltCommand, FrameContext, InputAction, InputBinding, InputBindingTarget, InputButton, InputCommandBuilder,
    MouseTuning, PhysicalInput, PitchDriftState, SeatFocus, SeatSample, SourceAction,
};
use qa_client::ui::settings::action_catalog::{BindingCapabilities, ScoreCommand};
use qa_client::ui::types::{
    ContentId, ResourceRequest, SeatInputEvent, SeatInputEventKind, SeatInputFocus, UiControlId, UiMenuId,
};
use qa_client::ClientError;
use qa_content::contract::{ExecutableRecipe, ExecutionModule, ModuleRole, NativeModuleApi};
use qa_core::cmd::Dialect;
use qa_core::cmd_buffer::{
    BufferError, BufferOptions, BufferServices, CommandBuffer, CommandContext, CommandHandler, CommandOrigin,
    ForwardedCommand, Invocation, ScriptCompletion, ScriptRead,
};
use qa_core::cvar::{CvarArchiveEntry, CvarError, CvarRegistry};
use qa_core::identity::{ActorId, ClientId, ProviderId, SeatId, SessionId};
use qa_core::math::{Vec2, Vec3};
use qa_net::common::commands::{ActorCommand, ArsenalIntent, CommandSource, UserCommand};
use qa_platform::controller::{
    ControllerDevice, ControllerEvent, ControllerOperationResult, ControllerSelection, ControllerSensor,
    ControllerState, SdlControllers,
};
use qa_platform::sdl::{read_sdl_clipboard, SdlEvent, SdlWindow};
use qa_world::client::ClientFamily;
use qa_world::session::SessionSeat;
use qa_world::WorldError;
use thiserror::Error;

use super::audio::commands::APPLICATION_AUDIO_COMMANDS;
use super::config_scripts::{
    console_config_root, ConfigScriptError, ConsoleScriptFiles, ConsoleScriptInputs, ConsoleScriptMounts,
};
use super::configuration::ConfigurationCommandRequest;
use super::console::{
    ApplicationConsoleRouting, ApplicationConsoleRoutingOptions, ApplicationConsoleServer, ConsoleCvarRouting,
    ConsoleError, ConsoleRegistry,
};
use super::controller_settings::{ControllerDeviceInfo, ControllerRouter, ControllerSettings, GyroEnableResult};
use super::cvar_archives::{load_cvar_archive, save_cvar_archive, CvarArchiveError, CvarArchiveHead, CvarArchiveOwner};
use super::input_devices::{input_device_store, DeviceRouter, InputDeviceError, InputDevices};
use super::prepared_startup::{
    AdoptedOwners, ForwardFn, PreparedMouse, PreparedScriptFiles, PreparedSeatDevice, PreparedStartup,
    PreparedStartupConfig, PreparedStartupError, PublishedSeat, RegistrySlot, ScopedReadFn, SeatRegistries,
    StartupCvarRouting, StartupScriptScope as PreparedScope,
};
use super::presentation::SeatPlayerView;
use super::q1_client_commands::{register_q1_client_commands, Q1ClientCommandExecute, Q1ClientCommandGuard};
use super::q1_client_settings::{register_q1_view_commands, Q1ViewCommandGuard};
use super::q2_client_commands::{register_q2_client_commands, Q2ClientCommandExecute, Q2ClientCommandGuard};
use super::q3_client::cvars::{Q3ClientCvarError, Q3ClientCvarOwners, Q3ClientCvars};
use super::q3_client::qvm_scalars::QvmClientInput;
use super::shared_setting_cvars::bind_run_cvar;
use super::startup_config::{StartupScriptRead, StartupScriptScope};
use crate::console::commands::{ConsoleCommandServices, ConsoleCommands};
use crate::console::discovery::register_discovery_commands;
use crate::console::llm::{register_llm_commands, SharedRequester};
use crate::console::llm_batch::dialect_name;
use crate::console::session::{ConsoleFocus, ConsoleInputEvent, SeatConsole, SeatConsoleServices};
use crate::console::ConsoleError as SeatConsoleError;
use crate::options::{ApplicationOptions, GameFamily, Network};
use crate::settings::config::{
    ConfigStore, ControllerSelection as SavedControllerSelection, GamepadTuning as SavedGamepadTuning, GyroYawAxis,
    InputRouting, MouseTuning as SavedMouseTuning, SeatSettings, StickCurve as SavedStickCurve,
};
use crate::settings::SettingsError;
use qa_client::input::gamepad::StickCurve as ClientStickCurve;
use qa_content::mounts::MountError;

/// Shared cvar registry cell.
pub type SharedRegistry = Rc<RefCell<CvarRegistry>>;

/// Shared command buffer cell.
pub type SharedCommands = Rc<RefCell<CommandBuffer>>;

/// Millisecond clock (`now`).
pub type InputClock = Rc<dyn Fn() -> f64>;

/// Window close event id (`SDL_WINDOWEVENT_CLOSE`).
const WINDOW_CLOSE: u8 = 14;
/// Window focus-lost event id (`SDL_WINDOWEVENT_FOCUS_LOST`).
const WINDOW_FOCUS_LOST: u8 = 13;
/// Console toggle key codes (backtick and tilde).
const CONSOLE_KEYS: [i32; 2] = [96, 126];
/// Cgame menu id the client captures for its own input.
const Q3_CGAME_MENU: &str = "menu:q3:cgame";
/// Maximum input command sequence (`Number.MAX_SAFE_INTEGER`).
const MAX_SEQUENCE: u64 = (1 << 53) - 1;

/// Local input failure.
#[derive(Debug, Error)]
pub enum InputError {
    /// At least one local player is required.
    #[error("Native input requires at least one local player")]
    NoPlayers,
    /// Seat count must be one to four.
    #[error("A graphical world needs one to four local players")]
    BadSeatCount,
    /// Command owner dialects do not match.
    #[error("Input command owner does not match its movement and console dialects")]
    OwnerDialect,
    /// Adopted client source registries do not match.
    #[error("Client input requires matching movement and source registries")]
    AdoptDialect,
    /// Mouse settings belong elsewhere.
    #[error("Mouse settings belong to another seat or source dialect")]
    ForeignMouse,
    /// Mouse publication lost its seat.
    #[error("Mouse publication lost its seat")]
    MouseSeat,
    /// Shared settings lost their owner.
    #[error("Shared settings lost their owner")]
    SharedOwner,
    /// Guest cvars need a routed local seat.
    #[error("Guest cvars require a routed local seat")]
    GuestSeat,
    /// Guest input needs a local seat.
    #[error("Guest input requires a local seat")]
    GuestInput,
    /// Command input seat is not active.
    #[error("Command input seat is not active")]
    InactiveSeat,
    /// Command selection has no local seat.
    #[error("Command selection has no local seat")]
    SelectionSeat,
    /// Arsenal selection has no local seat.
    #[error("Arsenal selection has no local seat")]
    ArsenalSeat,
    /// UI seat has no local input.
    #[error("UI seat has no local input")]
    UiSeat,
    /// Seat UI is already attached.
    #[error("Seat UI is already attached")]
    UiAttached,
    /// Retired seats differ from active owners.
    #[error("Retired local seats differ from their active input owners")]
    RetireMismatch,
    /// Local input retirement failed.
    #[error("Local input retirement failed: {0}")]
    RetireFailed(String),
    /// Local input preparation is stale.
    #[error("Local input preparation is stale")]
    StalePreparation,
    /// Admission changed prepared identities.
    #[error("Local input admission changed prepared identities")]
    AdmissionChanged,
    /// Local seat has no registry owners.
    #[error("Local seat has no registry owners")]
    SeatOwners,
    /// Local input has no settings registry.
    #[error("Local input has no settings registry")]
    SettingsRegistry,
    /// Published client seat has no registry owners.
    #[error("Published client seat has no registry owners")]
    PublishedOwners,
    /// Published configuration seat is missing.
    #[error("Published configuration seat is missing")]
    PublishedConfiguration,
    /// Published input seat is missing.
    #[error("Published input seat is missing")]
    PublishedSeat,
    /// Retained client replacement attempt.
    #[error("Source input attempted to replace a retained client")]
    RetainedReplace,
    /// Publication changed retained owners.
    #[error("Client input publication changed retained owners")]
    RetainedOwners,
    /// Client input preparation changed retained owners.
    #[error("Client input preparation changed retained owners")]
    PreparedOwners,
    /// Startup script reader adoption is missing.
    #[error("Startup script reader adoption is missing")]
    StartupReader,
    /// Startup adoption has no cvar routing.
    #[error("Startup adoption has no cvar routing")]
    StartupRouting,
    /// Authored binding defaults are unavailable.
    #[error("Authored binding defaults are unavailable")]
    BindingDefaults,
    /// World travel changed the local seat count.
    #[error("World travel changed the local seat count")]
    TravelSeats,
    /// World travel has no player for a local seat.
    #[error("World travel has no player for a local seat")]
    TravelPlayer,
    /// Input command sequence must be a nonnegative safe integer.
    #[error("Input command sequence must be a nonnegative safe integer")]
    BadSequence,
    /// Candidate defaults lost their retained seat.
    #[error("Candidate defaults lost their retained seat")]
    CandidateSeat,
    /// Configuration commands must publish before their continuation.
    #[error("Configuration commands must publish before their continuation")]
    ConfigurationOrder,
    /// Client commands require a local seat.
    #[error("Client commands require a local seat")]
    CommandSeat,
    /// Cvar failure.
    #[error(transparent)]
    Cvar(#[from] CvarError),
    /// Command buffer failure.
    #[error(transparent)]
    Buffer(#[from] BufferError),
    /// Console routing failure.
    #[error(transparent)]
    Routing(#[from] ConsoleError),
    /// Seat failure.
    #[error(transparent)]
    Seat(#[from] SeatError),
    /// Router failure.
    #[error(transparent)]
    Router(#[from] RouterError),
    /// Client input failure.
    #[error(transparent)]
    Client(#[from] ClientError),
    /// Client command failure.
    #[error(transparent)]
    Commands(#[from] CommandsError),
    /// Settings failure.
    #[error(transparent)]
    Settings(#[from] SettingsError),
    /// Cvar archive failure.
    #[error(transparent)]
    Archive(#[from] CvarArchiveError),
    /// Config script failure.
    #[error(transparent)]
    Scripts(#[from] ConfigScriptError),
    /// Seat console failure.
    #[error(transparent)]
    Console(#[from] SeatConsoleError),
    /// Input device failure.
    #[error(transparent)]
    Devices(#[from] InputDeviceError),
    /// Haptic failure.
    #[error(transparent)]
    Haptics(#[from] HapticError),
    /// World failure.
    #[error(transparent)]
    World(#[from] WorldError),
    /// Q3 client cvar failure.
    #[error(transparent)]
    Q3Cvars(#[from] Q3ClientCvarError),
    /// Platform failure.
    #[error("Platform input failed: {0}")]
    Platform(String),
    /// Prepared startup failure.
    #[error("Prepared startup failed: {0}")]
    Startup(String),
    /// Profile configuration was already consumed.
    #[error("Profile configuration was already consumed")]
    MissingConfiguration,
}

/// Quake III per-seat command selection (donor `Q3CommandSelection`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q3CommandSelection {
    /// Selected weapon number.
    pub weapon: u32,
    /// Command-time mouse sensitivity.
    pub sensitivity: f64,
}

/// [`movement_dialect`] failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MovementDialectError {
    /// The recipe has no timing profile for its movement provider.
    MissingTiming(ProviderId),
}

impl std::fmt::Display for MovementDialectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MovementDialectError::MissingTiming(provider) => {
                write!(f, "Recipe has no timing for {provider:?}")
            }
        }
    }
}

impl std::error::Error for MovementDialectError {}

/// Clock profile to movement dialect (donor `timing.clock.kind`).
#[must_use]
pub fn clock_dialect(clock: &qa_core::time::ClockProfile) -> Dialect {
    use qa_core::time::ClockProfile;
    match clock {
        ClockProfile::Q1Netquake { .. } => Dialect::Q1Netquake,
        ClockProfile::Q1Quakeworld { .. } => Dialect::Q1Quakeworld,
        ClockProfile::Q2Classic => Dialect::Q2Classic,
        ClockProfile::Q2Rerelease { .. } => Dialect::Q2Rerelease,
        ClockProfile::Q3 { .. } => Dialect::Q3,
    }
}

/// Resolve the movement command dialect (donor `movementDialect`).
///
/// A recipe with a native server-game module speaking a Quake II game API
/// pins the dialect; otherwise the movement provider's timing clock wins.
/// Without a recipe, QuakeWorld clients speak `q1-quakeworld` and the
/// movement family picks the fallback.
pub fn movement_dialect(
    options: &ApplicationOptions,
    recipe: Option<&ExecutableRecipe>,
) -> Result<Dialect, MovementDialectError> {
    if let Some(recipe) = recipe {
        for module in &recipe.execution {
            let ExecutionModule::Native { role, api, .. } = module else {
                continue;
            };
            if *role != ModuleRole::ServerGame {
                continue;
            }
            match api {
                NativeModuleApi::Q2RereleaseGame => return Ok(Dialect::Q2Rerelease),
                NativeModuleApi::Q2ClassicGame => return Ok(Dialect::Q2Classic),
                _ => {}
            }
        }
        let timing = recipe
            .timing
            .iter()
            .find(|profile| profile.provider == recipe.movement.provider)
            .ok_or_else(|| MovementDialectError::MissingTiming(recipe.movement.provider.clone()))?;
        return Ok(clock_dialect(&timing.clock));
    }
    if matches!(options.network, Network::QwClient { .. }) {
        return Ok(Dialect::Q1Quakeworld);
    }
    Ok(match options.movement {
        GameFamily::Q1 => Dialect::Q1Netquake,
        GameFamily::Q2 => Dialect::Q2Classic,
        GameFamily::Q3 => Dialect::Q3,
    })
}

/// Default binding capabilities before caller overrides (donor
/// `ApplicationInput.bindingCapabilities` fallback).
#[must_use]
pub fn default_binding_capabilities(network: &Network) -> BindingCapabilities {
    let chat = matches!(
        network,
        Network::Q1Client { .. }
            | Network::QwClient { .. }
            | Network::Q2Client { .. }
            | Network::Q3Client { .. }
            | Network::UnifiedClient { .. }
    );
    let score_command = if matches!(network, Network::Q2Client { .. }) {
        ScoreCommand::Score
    } else {
        ScoreCommand::Scores
    };
    BindingCapabilities {
        chat,
        score_command: Some(score_command),
        offhand_grapple: false,
        offhand_grenades: false,
    }
}

/// Unwrap script origins to the commanding origin.
fn root_origin(mut origin: &CommandOrigin) -> &CommandOrigin {
    while let CommandOrigin::Script { caller, .. } = origin {
        origin = caller;
    }
    origin
}

/// Local-seat command context.
fn seat_context(session: &SessionId, seat: &SeatId, client: &ClientId) -> CommandContext {
    CommandContext::new(
        session.clone(),
        CommandOrigin::LocalSeat {
            seat: seat.clone(),
            client: client.clone(),
        },
    )
}

/// Dialect to client family.
fn dialect_family(dialect: Dialect) -> ClientFamily {
    match dialect {
        Dialect::Q1Netquake => ClientFamily::Q1Netquake,
        Dialect::Q1Quakeworld => ClientFamily::Q1Quakeworld,
        Dialect::Q2Classic => ClientFamily::Q2Classic,
        Dialect::Q2Rerelease => ClientFamily::Q2Rerelease,
        Dialect::Q3 => ClientFamily::Q3,
    }
}

/// Wrap a registry in a shared cell.
fn shared_registry(registry: CvarRegistry) -> SharedRegistry {
    Rc::new(RefCell::new(registry))
}

/// Registry identity tag (donor object identity for candidate isolation).
fn registry_tag(cell: &SharedRegistry) -> u64 {
    Rc::as_ptr(cell) as u64
}

/// Seat settings path for a seat index.
fn seat_settings_path(index: u32) -> String {
    format!("input/seat-{}.json", index + 1)
}

/// Local player: session seat plus controlled actor (donor `LocalPlayer`).
pub struct LocalPlayer {
    /// Session seat.
    pub seat: SessionSeat,
    /// Controlled actor.
    pub actor: ActorId,
}

impl LocalPlayer {
    /// Seat id.
    #[must_use]
    pub fn seat_id(&self) -> &SeatId {
        self.seat.id()
    }
}

/// Local seat input (donor `LocalInput`).
///
/// The live [`Seat`] lives inside the input router; the seat id reaches it.
/// Everything else is owned here (consoles and haptics are shared with the
/// router callbacks that drive them).
pub struct LocalInput {
    /// Local player.
    pub player: LocalPlayer,
    /// Seat console.
    pub console: Rc<RefCell<SeatConsole>>,
    /// Command builder.
    pub builder: InputCommandBuilder,
    /// Seat haptics.
    pub haptics: Rc<RefCell<SeatHaptics>>,
    /// Seat command context.
    pub context: CommandContext,
}

impl LocalInput {
    /// Seat id.
    #[must_use]
    pub fn seat_id(&self) -> &SeatId {
        self.player.seat_id()
    }
}

/// Application callbacks (donor `ApplicationInputCommands`).
///
/// Async donor reads fold to synchronous results, matching the sibling ports.
pub trait ApplicationInputCommands {
    /// Quit the application.
    fn quit(&mut self);
    /// Execute a server-bound command.
    fn execute(&mut self, name: &str, args: &[String], seat: Option<&SeatId>, source: &CommandContext);
    /// Print global text.
    fn print(&mut self, text: &str);
    /// Prepared profile configuration, if any.
    fn configuration(&self) -> Option<&dyn InputProfileConfiguration> {
        None
    }
    /// Prepared profile configuration, mutably.
    fn configuration_mut(&mut self) -> Option<&mut dyn InputProfileConfiguration> {
        None
    }
    /// Shared console scripts, if any.
    fn scripts(&self) -> Option<ScriptsCell> {
        None
    }
    /// Read a script through the mounted fallback.
    fn read_script(&self, _name: &str) -> Option<Vec<u8>> {
        None
    }
    /// Read a mounted script override.
    fn read_mounted_script(&self, _name: &str) -> Option<Vec<u8>> {
        None
    }
    /// Whether a mounted script override is installed.
    fn has_mounted_script(&self) -> bool {
        false
    }
    /// Startup script reader for adoption.
    fn startup_reader(&self, _scripts: &dyn InputScripts, _options: &ApplicationOptions) -> Option<StartupScriptRead> {
        None
    }
    /// LLM requester for console commands.
    fn llm(&self) -> Option<SharedRequester> {
        None
    }
    /// Local player capacity.
    fn local_player_capacity(&self) -> usize {
        4
    }
    /// Whether a local player may be removed.
    fn can_remove_local_player(&self, _seat: &SeatId) -> bool {
        true
    }
    /// Binding capability overrides.
    fn binding_capabilities(&self) -> Option<BindingCapabilities> {
        None
    }
    /// Arsenal binding items for a seat.
    fn binding_items(&self, _seat: &SeatId) -> Vec<WeaponBindingItem> {
        Vec::new()
    }
    /// Arsenal impulse provider for a seat.
    fn arsenal_impulse_provider(&self, _seat: &SeatId) -> Option<ProviderId> {
        None
    }
    /// Shared settings registry, if any.
    fn shared_cvars(&self) -> Option<SharedRegistry> {
        None
    }
    /// Application console, if any.
    fn console(&self) -> Option<&dyn InputConsole> {
        None
    }
    /// Client input hook; true consumes the event.
    fn client_input(&mut self, _event: &SeatInputEvent) -> bool {
        false
    }
    /// Whether the client captures a seat's game input.
    fn client_captures_input(&self, _seat: &SeatId) -> bool {
        false
    }
}

/// Shared application callbacks.
pub type ActionsCell = Rc<RefCell<dyn ApplicationInputCommands>>;

/// Per-seat UI (donor `ApplicationInputUi`).
pub trait ApplicationInputUi {
    /// Handle an input event; true consumes it.
    fn input(&mut self, event: &SeatInputEvent, focus: &SeatInputFocus) -> bool;
    /// Close menus.
    fn close_menus(&mut self);
    /// Clear the prompt, if any.
    fn clear_prompt(&mut self) {}
    /// Transform a seat sample.
    fn sample(&mut self, sample: SeatSample) -> SeatSample {
        sample
    }
    /// Roll the wheel.
    fn wheel(&mut self, mode: WheelMode, down: bool);
    /// Cycle the weapon; true consumes the command.
    fn cycle_weapon(&mut self, _direction: i32) -> bool {
        false
    }
    /// Switch weapons; true consumes the command.
    fn switch_weapon(&mut self, _first: i32, _second: i32) -> bool {
        false
    }
}

/// Shared per-seat UI.
pub type UiCell = Rc<RefCell<dyn ApplicationInputUi>>;
/// Shared haptic sample loader (indirected so `bind_haptics` swaps it live).
/// Haptic sample loader.
pub type HapticLoader = Rc<dyn Fn(&ResourceRequest) -> Option<Vec<u8>>>;
/// Shared haptic sample loader.
pub type LoaderCell = Rc<RefCell<HapticLoader>>;
/// Release-text sink for profile changes.
pub type ReleaseSink = Rc<RefCell<dyn FnMut(&str, &CommandContext)>>;
/// Startup print sink.
pub type StartupPrintFn = Rc<dyn Fn(&str, Option<&CommandContext>)>;

/// Application console (donor `ApplicationInputCommands["console"]`).
pub trait InputConsole {
    /// Source game dialect.
    fn dialect(&self) -> Dialect;
    /// Server console plus client-mirrorable names.
    fn server(&self) -> Option<(SharedRegistry, Vec<String>)>;
    /// Seat registry.
    fn seat(&self, seat: &SeatId) -> Option<SharedRegistry>;
}

/// Console script files (donor `ConsoleScriptFiles` surface used here).
pub trait InputScripts {
    /// Read a script by name; `None` means missing.
    fn read(&self, name: &str, source: &CommandContext) -> Result<Option<String>, InputError>;
    /// Read through the mounted fallback.
    fn read_mounted(&self, name: &str) -> Result<Option<Vec<u8>>, InputError>;
    /// Write configuration text; the `String` error feeds the donor failure message.
    fn write_text(&self, path: &str, contents: &str) -> Result<(), String>;
}

/// Shared console scripts.
pub type ScriptsCell = Rc<RefCell<dyn InputScripts>>;

impl<M: ConsoleScriptMounts> InputScripts for ConsoleScriptFiles<M> {
    fn read(&self, name: &str, source: &CommandContext) -> Result<Option<String>, InputError> {
        Ok(ConsoleScriptFiles::read(self, name, source)?)
    }

    fn read_mounted(&self, name: &str) -> Result<Option<Vec<u8>>, InputError> {
        Ok(ConsoleScriptFiles::read_mounted(self, name)?)
    }

    fn write_text(&self, path: &str, contents: &str) -> Result<(), String> {
        let root = ConsoleScriptFiles::root(self).to_path_buf();
        ConsoleScriptFiles::write(self, || {
            std::fs::write(root.join(path), contents).map_err(|error| ConfigScriptError::Io {
                path: path.to_string(),
                message: error.to_string(),
            })
        })
        .map_err(|error| error.to_string())
    }
}

/// External cvar routing (donor `CommandCvarRouting` from an owner).
///
/// Slots are indexes into the routing table; [`Self::registry`] vends the
/// live cell for a slot.
pub trait InputCvarRouting {
    /// Owning slot for `name` issued from `source`.
    fn owner(&self, name: &str, source: &CommandContext) -> Result<usize, InputError>;
    /// Slots visible from `source`.
    fn visible(&self, source: &CommandContext) -> Result<Vec<usize>, InputError>;
    /// Live cell for a slot.
    fn registry(&self, index: usize) -> Option<SharedRegistry>;
}

/// Shared input router.
pub type RouterCell = Rc<RefCell<InputRouter>>;
/// Shared application window.
pub type WindowCell = Rc<RefCell<dyn InputWindow>>;
/// Shared controllers.
pub type ControllersCell = Rc<RefCell<dyn InputControllers>>;
/// Shared prepared startup.
pub type StartupCell = Rc<RefCell<dyn InputStartup>>;

/// Owned registry plus its declaring session.
pub type OwnedRegistry = (CvarRegistry, SessionId);

/// Registry image: dialect, session, and a world-transfer save image.
pub struct RegistryImage {
    /// Registry dialect.
    pub dialect: Dialect,
    /// Declaring session.
    pub session: SessionId,
    /// Save image.
    pub state: qa_core::cvar::CvarSaveState,
}

/// Startup registry images.
pub struct StartupRegistryImages {
    /// Source registry image.
    pub source: RegistryImage,
    /// Movement registry image.
    pub movement: RegistryImage,
    /// Fallback registry image.
    pub fallback: RegistryImage,
}

/// Live startup seat view.
pub struct StartupSeatView {
    /// Seat handle.
    pub id: SeatId,
    /// Seat command context.
    pub context: CommandContext,
    /// Seat cvar image.
    pub cvars: RegistryImage,
    /// Seat mouse image.
    pub mouse: RegistryImage,
    /// Device movement dialect.
    pub device_dialect: Dialect,
    /// Whether the device holds input.
    pub has_held_input: bool,
    /// Whether authored bindings exist.
    pub authored_bindings: bool,
    /// Shared live binding table.
    pub bindings: Rc<RefCell<qa_client::input::BindingTable>>,
}

/// Seat published into a startup.
pub struct InputPublishedSeat {
    /// Seat handle.
    pub id: SeatId,
    /// Live input device.
    pub device: InputSeatDevice,
    /// Seat command context.
    pub context: CommandContext,
    /// Seat cvar registry.
    pub cvars: CvarRegistry,
    /// Seat mouse registry.
    pub mouse: CvarRegistry,
    /// Shared binding table.
    pub bindings: Rc<RefCell<qa_client::input::BindingTable>>,
}

/// Owners adopted into a startup (donor `adopt` owners).
pub struct AdoptedOwnersView {
    /// Source registry plus session.
    pub source: OwnedRegistry,
    /// Movement registry plus session.
    pub movement: OwnedRegistry,
    /// Fallback registry plus session.
    pub fallback: OwnedRegistry,
    /// Console scripts.
    pub scripts: ScriptsCell,
    /// Scoped script reader.
    pub read: StartupScriptRead,
}

/// Prepared startup surface consumed here (donor `PreparedStartup`).
///
/// Input-compatible startups are
/// [`PreparedStartup`](super::prepared_startup::PreparedStartup)s built over
/// [`InputSeatDevice`], [`CvarRegistry`] mice, [`AdoptedRouting`], and
/// [`ScriptsCell`] scripts; see the trait implementation for the canonical
/// type.
pub trait InputStartup {
    /// Whether a configuration run is pending.
    fn pending(&self) -> bool;
    /// Source, movement, and fallback images.
    fn registries(&self) -> StartupRegistryImages;
    /// Command buffer dialect.
    fn commands_dialect(&self) -> Dialect;
    /// Command buffer context.
    fn commands_context(&self) -> CommandContext;
    /// Command buffer program revision.
    fn commands_revision(&self) -> u64;
    /// Movement variable value.
    fn movement_variable(&self, name: &str) -> f64;
    /// Copy pending program state into `target`.
    fn copy_commands_pending(&self, target: &mut CommandBuffer) -> Result<(), InputError>;
    /// Live seat views.
    fn seats(&self) -> Vec<StartupSeatView>;
    /// Console binding table, when the startup stages console bindings.
    fn console_bindings(&self) -> Option<Rc<RefCell<qa_client::input::BindingTable>>> {
        None
    }
    /// Console scripts cell.
    fn scripts_cell(&self) -> ScriptsCell;
    /// Registry identity tags for candidate isolation.
    fn live_tags(&self) -> HashSet<u64>;
    /// Adopt routing, forwarding, and owners.
    fn adopt(
        &mut self,
        routing: AdoptedRouting,
        forward: super::prepared_startup::ForwardFn,
        owners: AdoptedOwnersView,
    ) -> Result<(), InputError>;
    /// Validate adopted owners.
    fn validate_owners(&self, owners: &AdoptedOwnersView) -> Result<(), InputError>;
    /// Adopt seat registries.
    fn adopt_seat(&mut self, id: &SeatId, cvars: Option<CvarRegistry>, mouse: Option<CvarRegistry>);
    /// Publish seats, retaining unlisted previous seats on `Retain`.
    fn publish_seats(
        &mut self,
        seats: Vec<InputPublishedSeat>,
        active: Vec<SeatId>,
        retention: SeatPublication,
    ) -> Result<(), InputError>;
    /// Apply configuration binding choices to a published seat.
    fn apply_binding_choices(&mut self, seat: &SeatId, choices: &BindingChoices) -> Result<(), InputError>;
    /// Set the active seats.
    fn set_active_seats(&mut self, ids: Vec<SeatId>) -> Result<(), InputError>;
    /// Bind print output; returns the release.
    fn bind_output(&mut self, print: StartupPrintFn) -> Box<dyn FnOnce()>;
    /// Gate one invocation.
    fn allow_command(&mut self, argv: &[String], source: &CommandContext) -> bool;
    /// Note a world action.
    fn note_world_action(&mut self);
    /// Read a script.
    fn read_script(&mut self, name: &str, source: &CommandContext) -> Option<String>;
    /// Deliver a script completion.
    fn on_script_complete(&mut self, event: &ScriptCompletion);
    /// Resolve bindings, adopting defaults.
    fn bindings(&mut self, id: &SeatId, defaults: Vec<InputBinding>) -> Vec<InputBinding>;
    /// Adopt binding defaults.
    fn adopt_binding_defaults(&mut self, id: &SeatId, defaults: Vec<InputBinding>);
    /// Preview bindings without adopting.
    fn preview_bindings(&self, id: &SeatId, defaults: &[InputBinding]) -> Vec<InputBinding>;
    /// Reset bindings to authored defaults.
    fn reset_bindings(&mut self, id: &SeatId) -> Result<(), InputError>;
    /// Advance one startup frame.
    ///
    /// The canonical advance needs its configuration factory, which only the
    /// startup owner holds; this reports `Ok(false)` when nothing is pending
    /// and errors loudly when a pending run needs its owner-driven advance.
    fn advance_frame(&mut self) -> Result<bool, InputError>;
}

/// Prepared profile seat (donor `PreparedSeatConfiguration` plus live input).
pub struct ProfileSeat {
    /// Seat handle.
    pub id: SeatId,
    /// Seat command context.
    pub context: CommandContext,
    /// Seat cvar registry.
    pub cvars: CvarRegistry,
    /// Seat mouse registry.
    pub mouse: CvarRegistry,
    /// Live seat built by the configuration, moved into the router.
    pub seat: Option<Seat>,
    /// Saved seat profile, if any.
    pub profile: Option<SeatSettings>,
}

/// Binding choices for one seat (donor `bindingChoices` element).
#[derive(Debug, Clone)]
pub struct BindingChoices {
    /// Seat handle.
    pub id: SeatId,
    /// Overridden physical keys.
    pub overridden_keys: Vec<String>,
    /// Whether all bindings were explicitly chosen.
    pub all_bindings_chosen: bool,
    /// Selected bindings.
    pub selected_bindings: Vec<InputBinding>,
    /// Authored bindings, if any.
    pub authored_bindings: Option<Vec<InputBinding>>,
}

/// Prepared profile configuration surface consumed here (donor
/// `PreparedProfileConfiguration`).
pub trait InputProfileConfiguration {
    /// Move the movement registry out.
    fn take_movement(&mut self) -> Option<CvarRegistry>;
    /// Move the fallback registry out.
    fn take_fallback(&mut self) -> Option<CvarRegistry>;
    /// Move the prepared seats out.
    fn take_seats(&mut self) -> Vec<ProfileSeat>;
    /// Move the prebuilt staged program out.
    fn take_program(&mut self) -> Option<StagedClientCommands>;
    /// Shared console scripts.
    fn scripts(&self) -> Option<ScriptsCell>;
    /// Binding choices by seat.
    fn binding_choices(&self) -> Vec<BindingChoices>;
    /// Apply binding defaults for a seat.
    fn apply_binding_defaults(&mut self, seat: &SeatId, defaults: &[InputBinding]);
    /// Forward configuration command requests.
    fn forward_commands(&mut self, forward: &mut dyn FnMut(ConfigurationCommandRequest));
    /// Publish the continuation into a startup.
    fn publish_continuation(&mut self, startup: &StartupCell) -> Result<(), InputError>;
    /// Retire the configuration routing.
    fn close_routing(&mut self);
}

/// Client local seat (donor `ClientBootstrapSeat`).
pub struct ClientLocal {
    /// Session client.
    pub client: ClientId,
    /// Session seat.
    pub seat: SessionSeat,
    /// Prepared seat handle.
    pub prepared: ClientLocalPrepared,
}

/// Live prepared-seat handle (seat id plus its startup cell).
#[derive(Clone)]
pub struct ClientLocalPrepared {
    /// Seat handle.
    pub id: SeatId,
    /// Startup cell.
    pub startup: StartupCell,
}

/// Client input publication (donor `ClientBootstrap["platform"]["current"]`).
pub enum InputClientPlatform {
    /// Menu input with a command retirer.
    Menu {
        /// Input router.
        router: RouterCell,
        /// Controller settings.
        settings: Box<ControllerSettings<RouterHandle, SharedDevicesFn>>,
        /// Retire menu commands.
        retire_commands: Box<dyn FnOnce()>,
    },
    /// World input.
    World {
        /// Application input.
        input: Box<ApplicationInput>,
    },
}

/// Shared client input publication.
pub type PlatformCell = Rc<RefCell<Option<InputClientPlatform>>>;

/// Client bootstrap surface consumed here (donor `ClientBootstrap`).
pub trait InputClient {
    /// Renderer window cell.
    fn renderer_window(&self) -> WindowCell;
    /// Controllers cell.
    fn controllers_cell(&self) -> ControllersCell;
    /// Input devices cell.
    fn devices_cell(&self) -> DeviceCell;
    /// Prepared startup cell.
    fn prepared_cell(&self) -> StartupCell;
    /// Local seats, mutably.
    fn locals_mut(&mut self) -> &mut Vec<ClientLocal>;
    /// Seat consoles by seat id, mutably.
    fn consoles_mut(&mut self) -> &mut HashMap<SeatId, ConsoleCell>;
    /// Input publication cell.
    fn platform(&self) -> PlatformCell;
}

/// Shared seat console.
pub type ConsoleCell = Rc<RefCell<SeatConsole>>;
/// Shared seat haptics.
pub type HapticsCell = Rc<RefCell<SeatHaptics>>;

/// Application window surface consumed here (donor `SdlWindow`).
pub trait InputWindow {
    /// Pump platform events.
    fn poll_events(&mut self) -> Result<Vec<SdlEvent>, InputError>;
    /// Millisecond ticks.
    fn ticks(&self) -> Result<u32, InputError>;
    /// Logical window size.
    fn logical_size(&mut self) -> Result<(i32, i32), InputError>;
    /// Drawable size in pixels.
    fn drawable_size(&mut self) -> Result<(i32, i32), InputError>;
    /// Capture relative mouse motion.
    fn set_relative_mouse(&mut self, enabled: bool) -> Result<(), InputError>;
}

impl InputWindow for SdlWindow {
    fn poll_events(&mut self) -> Result<Vec<SdlEvent>, InputError> {
        SdlWindow::poll_events(self).map_err(|error| InputError::Platform(error.to_string()))
    }

    fn ticks(&self) -> Result<u32, InputError> {
        SdlWindow::ticks(self).map_err(|error| InputError::Platform(error.to_string()))
    }

    fn logical_size(&mut self) -> Result<(i32, i32), InputError> {
        SdlWindow::logical_size(self).map_err(|error| InputError::Platform(error.to_string()))
    }

    fn drawable_size(&mut self) -> Result<(i32, i32), InputError> {
        SdlWindow::drawable_size_signed(self).map_err(|error| InputError::Platform(error.to_string()))
    }

    fn set_relative_mouse(&mut self, enabled: bool) -> Result<(), InputError> {
        // Relative mode lives on the window's input lease; a window that
        // never leased input has nothing to set.
        if let Some(lease) = self.input_lease() {
            lease
                .set_relative_mouse(enabled)
                .map_err(|error| InputError::Platform(error.to_string()))?;
        }
        Ok(())
    }
}

/// Controller surface consumed here (donor `SdlControllers`).
pub trait InputControllers {
    /// Pump controller events.
    fn poll_events(&mut self) -> Result<Vec<ControllerEvent>, InputError>;
    /// Rumble a controller.
    fn rumble(
        &self,
        instance: i32,
        low: f64,
        high: f64,
        duration_ms: u32,
    ) -> Result<ControllerOperationResult, InputError>;
    /// Attached devices.
    fn devices(&self) -> Result<Vec<ControllerDevice>, InputError>;
    /// Route assignment slots.
    fn set_assignments(&mut self, selections: &[ControllerSelection]) -> Result<(), InputError>;
    /// Snapshot live axes and buttons.
    fn snapshot(&self, instance: i32) -> Result<Option<ControllerState>, InputError>;
    /// Current slot assignments.
    fn assignments(&self) -> Result<Vec<Option<i32>>, InputError>;
    /// Enable a controller sensor.
    fn set_sensor_enabled(
        &mut self,
        instance: i32,
        sensor: ControllerSensor,
        enabled: bool,
    ) -> Result<ControllerOperationResult, InputError>;
    /// Close the controllers.
    fn close(&mut self);
}

impl InputControllers for SdlControllers {
    fn poll_events(&mut self) -> Result<Vec<ControllerEvent>, InputError> {
        SdlControllers::poll_events(self).map_err(|error| InputError::Platform(error.to_string()))
    }

    fn rumble(
        &self,
        instance: i32,
        low: f64,
        high: f64,
        duration_ms: u32,
    ) -> Result<ControllerOperationResult, InputError> {
        SdlControllers::rumble(self, instance, low, high, duration_ms)
            .map_err(|error| InputError::Platform(error.to_string()))
    }

    fn devices(&self) -> Result<Vec<ControllerDevice>, InputError> {
        SdlControllers::devices(self).map_err(|error| InputError::Platform(error.to_string()))
    }

    fn set_assignments(&mut self, selections: &[ControllerSelection]) -> Result<(), InputError> {
        SdlControllers::set_assignments(self, selections).map_err(|error| InputError::Platform(error.to_string()))
    }

    fn snapshot(&self, instance: i32) -> Result<Option<ControllerState>, InputError> {
        SdlControllers::snapshot(self, instance).map_err(|error| InputError::Platform(error.to_string()))
    }

    fn assignments(&self) -> Result<Vec<Option<i32>>, InputError> {
        SdlControllers::assignments(self).map_err(|error| InputError::Platform(error.to_string()))
    }

    fn set_sensor_enabled(
        &mut self,
        instance: i32,
        sensor: ControllerSensor,
        enabled: bool,
    ) -> Result<ControllerOperationResult, InputError> {
        SdlControllers::set_sensor_enabled(self, instance, sensor, enabled)
            .map_err(|error| InputError::Platform(error.to_string()))
    }

    fn close(&mut self) {
        SdlControllers::close(self);
    }
}

/// Player-view source (donor `SimulationPresentationAccess["playerView"]`).
pub trait InputPlayerView {
    /// Player view for an actor.
    fn player_view(&self, actor: &ActorId) -> SeatPlayerView;
    /// Pitch drift for Quake I frames, when the simulation tracks it.
    fn pitch_drift(&self, _actor: &ActorId) -> Option<PitchDriftState> {
        None
    }
}

impl<T: super::presentation::SeatHost> InputPlayerView for T {
    fn player_view(&self, actor: &ActorId) -> SeatPlayerView {
        super::presentation::SeatHost::player_view(self, actor)
    }
}

/// Adopted command owners (donor `ApplicationInputCommandOwner`).
pub struct ApplicationInputCommandOwner {
    /// Movement registry override.
    pub movement: Option<SharedRegistry>,
    /// Fallback registry override.
    pub fallback: Option<SharedRegistry>,
    /// Console scripts override.
    pub scripts: Option<ScriptsCell>,
    /// Command cvar registry.
    pub cvars: SharedRegistry,
    /// Command buffer, moved in.
    pub commands: CommandBuffer,
    /// External cvar routing.
    pub routing: Box<dyn InputCvarRouting>,
    /// Input settings registry override.
    pub input_settings: Option<SharedRegistry>,
}

/// Shared device list for controller settings.
pub type SharedDevicesFn = Box<dyn Fn() -> Vec<ControllerDeviceInfo>>;
/// Shared input devices.
pub type DeviceCell = Rc<RefCell<InputDevices<RouterHandle>>>;

/// Router handle shared with device and controller settings.
#[derive(Clone)]
pub struct RouterHandle {
    /// Router cell.
    pub router: RouterCell,
    /// Command buffer cell.
    pub commands: SharedCommands,
    /// Console cvar cell (buffer registry for command registration).
    pub cvars: SharedRegistry,
    /// Seat contexts by seat id.
    pub contexts: Rc<RefCell<HashMap<SeatId, CommandContext>>>,
    /// Print router.
    pub print: PrintRouter,
}

impl RouterHandle {
    /// Refresh seat contexts from the locals.
    pub fn refresh_contexts(&self, locals: &[LocalInput]) {
        let mut contexts = self.contexts.borrow_mut();
        contexts.clear();
        for local in locals {
            contexts.insert(local.seat_id().clone(), local.context.clone());
        }
    }

    /// Command registry adapter over the shared buffer.
    pub fn registry(&self) -> BufferRegistry {
        BufferRegistry {
            commands: Rc::clone(&self.commands),
            cvars: Rc::clone(&self.cvars),
            contexts: Rc::clone(&self.contexts),
            print: self.print.clone(),
        }
    }
}

/// Seat-routed printing (donor `ApplicationInput.print`).
#[derive(Clone)]
pub struct PrintRouter {
    /// Application callbacks.
    pub actions: ActionsCell,
    /// Seat consoles in seat order.
    pub consoles: Rc<RefCell<Vec<(SeatId, ConsoleCell)>>>,
    /// Command buffer cell.
    pub commands: SharedCommands,
    /// Millisecond clock.
    pub now: InputClock,
}

impl PrintRouter {
    /// Print through the execution context, falling back to global output.
    pub fn print(&self, text: &str, source: Option<&CommandContext>) {
        let context = source.cloned().or_else(|| {
            self.commands
                .try_borrow()
                .ok()
                .and_then(|commands| commands.execution_context().cloned())
        });
        self.print_direct(text, context.as_ref());
    }

    /// Print for an explicit context without consulting the buffer.
    pub fn print_direct(&self, text: &str, source: Option<&CommandContext>) {
        let Some(context) = source else {
            self.actions.borrow_mut().print(text);
            for (_, console) in self.consoles.borrow().iter() {
                let mut services = NullSeatServices {
                    now: Rc::clone(&self.now),
                };
                let _ = console.borrow_mut().print(text, &mut services);
            }
            return;
        };
        match root_origin(&context.origin) {
            CommandOrigin::LocalSeat { seat, .. } => {
                let consoles = self.consoles.borrow();
                if let Some((_, console)) = consoles.iter().find(|(id, _)| id == seat) {
                    let mut services = NullSeatServices {
                        now: Rc::clone(&self.now),
                    };
                    let _ = console.borrow_mut().print(text, &mut services);
                }
            }
            _ => {
                self.actions.borrow_mut().print(text);
                if matches!(root_origin(&context.origin), CommandOrigin::LocalConsole) {
                    let consoles = self.consoles.borrow();
                    if let Some((_, console)) = consoles.first() {
                        let mut services = NullSeatServices {
                            now: Rc::clone(&self.now),
                        };
                        let _ = console.borrow_mut().print(text, &mut services);
                    }
                }
            }
        }
    }
}

/// Minimal seat services for global printing.
struct NullSeatServices {
    now: InputClock,
}

impl SeatConsoleServices for NullSeatServices {
    fn now_ms(&mut self) -> i64 {
        (self.now)() as i64
    }

    fn connected(&self) -> bool {
        false
    }

    fn clipboard(&mut self) -> Option<String> {
        None
    }

    fn set_focus(&mut self, _focus: ConsoleFocus) {}

    fn chat(&mut self, _text: &str, _team: bool, _target: Option<i32>) {}
}

/// Translate a core origin to a client origin.
fn convert_origin(origin: &CommandOrigin) -> ClientOrigin {
    match origin {
        CommandOrigin::LocalConsole => ClientOrigin::LocalConsole,
        CommandOrigin::ServerConsole => ClientOrigin::ServerConsole,
        CommandOrigin::LocalSeat { seat, .. } => ClientOrigin::LocalSeat(seat.clone()),
        CommandOrigin::RemoteClient { .. } => ClientOrigin::Other,
        CommandOrigin::Script { caller, .. } => ClientOrigin::Script {
            caller: Box::new(convert_origin(caller)),
        },
    }
}

/// Adapts a core [`CommandBuffer`] to the client [`CommandRegistry`] surface.
pub struct BufferRegistry {
    /// Command buffer cell.
    pub commands: SharedCommands,
    /// Registry for the registration collision check.
    pub cvars: SharedRegistry,
    /// Seat contexts by seat id.
    pub contexts: Rc<RefCell<HashMap<SeatId, CommandContext>>>,
    /// Print router.
    pub print: PrintRouter,
}

impl BufferRegistry {
    /// Register one translated handler.
    fn register_inner(&mut self, name: &str, handler: ClientRegistryHandler) -> bool {
        let handler = Rc::new(RefCell::new(handler));
        let wrapped: CommandHandler = Rc::new(move |invocation: &mut Invocation| {
            let argv = invocation.argv.clone();
            let call = CommandInvocation::new(argv, convert_origin(&invocation.source.origin), invocation.dialect);
            let outcome = (*handler.borrow_mut())(&call);
            if let Err(error) = outcome {
                invocation.print(&format!("{error}\n"));
            }
        });
        let cvars = self.cvars.borrow();
        self.commands
            .borrow_mut()
            .register(name, Some(wrapped), None, &cvars)
            .unwrap_or(false)
    }
}

impl CommandRegistry for BufferRegistry {
    fn register_engine(&mut self, name: &str, handler: ClientRegistryHandler) -> bool {
        self.register_inner(name, handler)
    }

    fn register(&mut self, name: &str, handler: ClientRegistryHandler) -> bool {
        self.register_inner(name, handler)
    }

    fn unregister(&mut self, name: &str) {
        self.commands.borrow_mut().unregister(name);
    }

    fn exists(&self, name: &str) -> bool {
        self.commands.borrow().exists(name)
    }

    fn append(&mut self, text: &str, seat: &SeatId) {
        let context = self.contexts.borrow().get(seat).cloned();
        match context {
            Some(context) => {
                if let Err(error) = self.commands.borrow_mut().append(text, Some(&context), None) {
                    self.print.print(&format!("{error}\n"), Some(&context));
                }
            }
            None => self.print.print_direct("Client commands require a local seat\n", None),
        }
    }
}

/// Discarding command registry for releases without a sink.
pub struct NullRegistry;

impl CommandRegistry for NullRegistry {
    fn register_engine(&mut self, _name: &str, _handler: ClientRegistryHandler) -> bool {
        false
    }

    fn register(&mut self, _name: &str, _handler: ClientRegistryHandler) -> bool {
        false
    }

    fn unregister(&mut self, _name: &str) {}

    fn exists(&self, _name: &str) -> bool {
        false
    }

    fn append(&mut self, _text: &str, _seat: &SeatId) {}
}

/// Command registry shim routing appends into a release sink.
pub struct SinkRegistry<'a> {
    /// Seat contexts by seat id.
    pub contexts: Rc<RefCell<HashMap<SeatId, CommandContext>>>,
    /// Fallback context.
    pub fallback: CommandContext,
    /// Release sink.
    pub sink: &'a mut dyn FnMut(&str, &CommandContext),
}

impl CommandRegistry for SinkRegistry<'_> {
    fn register_engine(&mut self, _name: &str, _handler: ClientRegistryHandler) -> bool {
        false
    }

    fn register(&mut self, _name: &str, _handler: ClientRegistryHandler) -> bool {
        false
    }

    fn unregister(&mut self, _name: &str) {}

    fn exists(&self, _name: &str) -> bool {
        false
    }

    fn append(&mut self, text: &str, seat: &SeatId) {
        let context = self
            .contexts
            .borrow()
            .get(seat)
            .cloned()
            .unwrap_or_else(|| self.fallback.clone());
        (self.sink)(text, &context);
    }
}

impl DeviceRouter for RouterHandle {
    fn seat_id(&self, index: u32) -> Option<SeatId> {
        self.router
            .borrow()
            .seats()
            .get(index as usize)
            .map(|seat| seat.seat().clone())
    }

    fn seat_count(&self) -> u32 {
        self.router.borrow().seats().len() as u32
    }

    fn key(&mut self, index: u32, code: i32, down: bool, time_ms: i64) {
        let Some(seat) = self.seat_id(index) else {
            return;
        };
        let event = RouterSeatEvent::Key {
            seat: seat.clone(),
            time_ms: time_ms as f64,
            code,
            down,
        };
        let mut registry = self.registry();
        if let Some(seat) = self.router.borrow_mut().seat_mut(&seat) {
            let _ = seat.input(&event, &mut registry);
        }
    }

    fn mouse_motion(&mut self, index: u32, dx: i32, dy: i32, time_ms: i64) {
        let Some(seat) = self.seat_id(index) else {
            return;
        };
        let event = RouterSeatEvent::MouseMotion {
            seat: seat.clone(),
            time_ms: time_ms as f64,
            position: (0.0, 0.0),
            delta: (f64::from(dx), f64::from(dy)),
        };
        let mut registry = self.registry();
        if let Some(seat) = self.router.borrow_mut().seat_mut(&seat) {
            let _ = seat.input(&event, &mut registry);
        }
    }

    fn release_seat(&mut self, index: u32, time_ms: i64, append: Option<&mut dyn FnMut(&str)>) {
        let Some(seat) = self.seat_id(index) else {
            return;
        };
        let mut null = NullRegistry;
        let mut forwarded;
        let registry: &mut dyn CommandRegistry = match append {
            Some(append) => {
                forwarded = ForwardRegistry {
                    seat: seat.clone(),
                    sink: append,
                };
                &mut forwarded
            }
            None => &mut null,
        };
        if let Some(seat) = self.router.borrow_mut().seat_mut(&seat) {
            seat.release(time_ms as f64, registry);
        }
    }

    fn any_focused(&self) -> bool {
        self.router.borrow().seats().iter().any(|seat| seat.focused())
    }

    fn set_source_joystick(&mut self, instance: Option<i32>, seat: Option<SeatId>) {
        let _ = self.router.borrow_mut().set_source_joystick(instance, seat);
    }

    fn player_choices(&self) -> Vec<(String, String)> {
        self.router
            .borrow()
            .seats()
            .iter()
            .enumerate()
            .map(|(index, _)| (index.to_string(), format!("Player {}", index + 1)))
            .collect()
    }
}

/// Registry forwarding appends for one seat into a text sink.
struct ForwardRegistry<'a> {
    seat: SeatId,
    sink: &'a mut dyn FnMut(&str),
}

impl CommandRegistry for ForwardRegistry<'_> {
    fn register_engine(&mut self, _name: &str, _handler: ClientRegistryHandler) -> bool {
        false
    }

    fn register(&mut self, _name: &str, _handler: ClientRegistryHandler) -> bool {
        false
    }

    fn unregister(&mut self, _name: &str) {}

    fn exists(&self, _name: &str) -> bool {
        false
    }

    fn append(&mut self, text: &str, seat: &SeatId) {
        if seat == &self.seat {
            (self.sink)(text);
        }
    }
}

impl ControllerRouter for RouterHandle {
    fn seat_tuning(&self, seat: &SeatId) -> Option<GamepadTuning> {
        self.router.borrow().seat(seat).map(|seat| seat.gamepad.tuning)
    }

    fn set_seat_tuning(&mut self, seat: &SeatId, tuning: GamepadTuning) {
        if let Some(seat) = self.router.borrow_mut().seat_mut(seat) {
            seat.gamepad.tuning = tuning;
        }
    }

    fn controller_for(&self, seat: &SeatId) -> Option<i32> {
        self.router.borrow().controller_for(seat)
    }

    fn set_gyro_enabled(&mut self, seat: &SeatId, enabled: bool) -> GyroEnableResult {
        match self.router.borrow_mut().set_gyro_enabled(seat, enabled) {
            Ok(ControllerOperationResult::Accepted) => GyroEnableResult::Accepted,
            Ok(other) => GyroEnableResult::Rejected {
                reason: format!("{other:?}"),
            },
            Err(error) => GyroEnableResult::Rejected {
                reason: error.to_string(),
            },
        }
    }
}

/// Button seat over a routed seat.
pub struct ButtonSeatHandle {
    /// Router cell.
    pub router: RouterCell,
    /// Seat id.
    pub seat: SeatId,
}

impl ButtonSeat for ButtonSeatHandle {
    fn command_button(&mut self, action: SourceAction, key: &str, down: bool, time_ms: i64) {
        if let Some(seat) = self.router.borrow_mut().seat_mut(&self.seat) {
            seat.command_button(action, key, down, time_ms);
        }
    }

    fn release_button(&mut self, action: SourceAction, time_ms: i64) {
        if let Some(seat) = self.router.borrow_mut().seat_mut(&self.seat) {
            seat.button_mut(action).release(time_ms);
        }
    }

    fn set_impulse_byte(&mut self, value: u8) {
        if let Some(seat) = self.router.borrow_mut().seat_mut(&self.seat) {
            seat.set_impulse(value);
        }
    }
}

/// Queued focus changes drained by the input pump.
pub type FocusQueue = Rc<RefCell<HashMap<SeatId, SeatFocus>>>;

/// Seat console services for one seat (donor `SeatConsole` closures).
#[derive(Clone)]
pub struct ConsoleSeatServices {
    /// Seat id.
    pub seat: SeatId,
    /// Seat command context.
    pub context: CommandContext,
    /// Haptics cell.
    pub haptics: HapticsCell,
    /// Router handle.
    pub handle: RouterHandle,
    /// Queued focus changes (console input runs inside a router borrow).
    pub pending_focus: FocusQueue,
    /// Millisecond clock.
    pub now: InputClock,
}

impl SeatConsoleServices for ConsoleSeatServices {
    fn now_ms(&mut self) -> i64 {
        (self.now)() as i64
    }

    fn connected(&self) -> bool {
        true
    }

    fn clipboard(&mut self) -> Option<String> {
        read_sdl_clipboard()
            .ok()
            .flatten()
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
    }

    fn set_focus(&mut self, focus: ConsoleFocus) {
        let seat_focus = match focus {
            ConsoleFocus::Game => SeatFocus::Game,
            ConsoleFocus::Console => SeatFocus::Console,
            ConsoleFocus::Chat { team } => SeatFocus::Chat {
                team,
                text: String::new(),
            },
        };
        self.pending_focus
            .borrow_mut()
            .insert(self.seat.clone(), seat_focus.clone());
        let active = matches!(seat_focus, SeatFocus::Game);
        self.haptics.borrow_mut().set_active(active);
    }

    fn chat(&mut self, text: &str, team: bool, target: Option<i32>) {
        let mut args = vec![text.to_string()];
        if let Some(target) = target {
            args.push(target.to_string());
        }
        let context = self.context.clone();
        self.handle.print.actions.borrow_mut().execute(
            if team { "say_team" } else { "say" },
            &args,
            Some(&self.seat),
            &context,
        );
    }
}

/// Console command services for one seat console.
pub struct ConsoleServices {
    /// Seat id.
    pub seat: SeatId,
    /// Seat command context.
    pub context: CommandContext,
    /// Seat console cell.
    pub console: ConsoleCell,
    /// Console command registry cell.
    pub commands: Rc<RefCell<ConsoleCommands>>,
    /// Console cvar cell.
    pub cvars: SharedRegistry,
    /// Movement cvar cell.
    pub movement: SharedRegistry,
    /// Mouse cvar cell.
    pub mouse: SharedRegistry,
    /// Binding table for `bind` archival.
    pub bindings: Rc<RefCell<qa_client::input::BindingTable>>,
    /// Seat services.
    pub seat_services: ConsoleSeatServices,
    /// Batches queued by `llm_exec`, drained by the caller after execution.
    pub pending_batches: Vec<String>,
    /// Settings store for file writes.
    pub settings: ConfigStore,
    /// Source dialect.
    pub dialect: Dialect,
    /// Current map name.
    pub map: String,
}

impl ConsoleCommandServices for ConsoleServices {
    fn print(&mut self, text: &str) {
        let context = self.context.clone();
        self.seat_services.handle.print.print(text, Some(&context));
    }

    fn forward_to_server(&mut self, line: &str) {
        let mut words = line.split_whitespace();
        let Some(name) = words.next() else {
            return;
        };
        let args: Vec<String> = words.map(str::to_string).collect();
        let context = self.context.clone();
        self.seat_services.handle.print.actions.borrow_mut().execute(
            name,
            &args,
            Some(&self.seat_services.seat),
            &context,
        );
    }

    fn toggle_console(&mut self) {
        let mut services = self.seat_services.clone();
        self.console.borrow_mut().toggle(&mut services);
    }

    fn clear_console(&mut self) {
        self.console.borrow_mut().buffer.clear();
    }

    fn message_mode(&mut self, team: bool) {
        let mut services = self.seat_services.clone();
        self.console.borrow_mut().message(team, None, &mut services);
    }

    fn can_chat(&self) -> bool {
        true
    }

    fn console_dump(&self) -> String {
        self.console.borrow().buffer.dump()
    }

    fn write_file(&mut self, path: &str, contents: &str) -> Result<(), String> {
        self.settings.dump(path, contents).map_err(|error| error.to_string())
    }

    fn configuration_text(&mut self, _argv: &[String]) -> String {
        let mut lines = Vec::new();
        let movement = self.movement.borrow();
        let cvars = self.cvars.borrow();
        lines.extend(movement.archive_commands(&|_| true));
        if !Rc::ptr_eq(&self.movement, &self.cvars) {
            lines.extend(cvars.archive_commands(&|_| true));
        }
        lines.extend(self.mouse.borrow().archive_commands(&|_| true));
        let bindings: Vec<InputBinding> = self.bindings.borrow().bindings().into_iter().cloned().collect();
        match qa_client::input::bindings::archived_bindings(&bindings, true) {
            Ok(archived) => lines.extend(archived),
            Err(error) => lines.push(format!("// binding archival failed: {error}")),
        }
        lines.join("\n")
    }

    fn map_name(&self) -> String {
        self.map.clone()
    }

    fn discovery_entries(&self) -> Vec<super::super::console::discovery::ConsoleDiscoveryEntry> {
        use super::super::console::discovery::{ConsoleDiscoveryEntry, DiscoveryKind};
        let mut entries = Vec::new();
        let commands = self.commands.borrow();
        for name in commands.registered_names() {
            let documentation = commands.command_documentation(&name);
            entries.push(ConsoleDiscoveryEntry {
                name,
                kind: DiscoveryKind::Command,
                summary: documentation.and_then(|documentation| documentation.summary.clone()),
                usage: documentation.and_then(|documentation| documentation.usage.clone()),
                examples: documentation
                    .map(|documentation| documentation.examples.clone())
                    .unwrap_or_default(),
                allowed_values: None,
                alias_value: None,
                value: None,
                reset_value: None,
                latched_value: None,
            });
        }
        for name in commands.alias_names() {
            let alias_value = commands.alias_value(&name).map(str::to_string);
            entries.push(ConsoleDiscoveryEntry {
                name,
                kind: DiscoveryKind::Alias,
                summary: None,
                usage: None,
                examples: Vec::new(),
                allowed_values: None,
                alias_value,
                value: None,
                reset_value: None,
                latched_value: None,
            });
        }
        for snapshot in self.cvars.borrow().snapshots(0) {
            entries.push(ConsoleDiscoveryEntry {
                name: snapshot.name,
                kind: DiscoveryKind::Cvar,
                summary: None,
                usage: None,
                examples: Vec::new(),
                allowed_values: None,
                alias_value: None,
                value: Some(snapshot.value),
                reset_value: Some(snapshot.reset_value),
                latched_value: snapshot.latched_value,
            });
        }
        entries
    }

    fn llm_batch_registry(&self) -> super::super::console::llm_batch::LlmBatchRegistry {
        super::super::console::llm_batch::LlmBatchRegistry::for_dialect(self.dialect)
    }

    fn execute_llm_batch(&mut self, batch: &str) {
        self.pending_batches.push(batch.to_string());
    }
}

/// Script mounts over the application callbacks.
#[derive(Clone)]
pub struct ScriptMounts {
    /// Application callbacks.
    pub actions: ActionsCell,
    /// Whether a mounted script override is installed.
    pub has_override: bool,
}

impl super::config_scripts::ConsoleScriptMounts for ScriptMounts {
    fn read_mounted(&self, name: &str) -> Result<Option<Vec<u8>>, MountError> {
        Ok(self.actions.borrow().read_script(name))
    }

    fn read_mounted_script(&self, name: &str) -> Result<Option<Vec<u8>>, MountError> {
        Ok(self.actions.borrow().read_mounted_script(name))
    }

    fn has_mounted_script(&self) -> bool {
        self.has_override
    }
}

/// Router window over a shared application window.
pub struct WindowAdapter {
    /// Window cell.
    pub window: WindowCell,
}

impl RouterWindow for WindowAdapter {
    fn logical_size(&mut self) -> Result<(i32, i32), RouterError> {
        self.window
            .borrow_mut()
            .logical_size()
            .map_err(|error| RouterError::Platform(error.to_string()))
    }

    fn drawable_size(&mut self) -> Result<(i32, i32), RouterError> {
        self.window
            .borrow_mut()
            .drawable_size()
            .map_err(|error| RouterError::Platform(error.to_string()))
    }

    fn poll_events(&mut self) -> Result<Vec<SdlEvent>, RouterError> {
        self.window
            .borrow_mut()
            .poll_events()
            .map_err(|error| RouterError::Platform(error.to_string()))
    }

    fn set_relative_mouse(&mut self, enabled: bool) -> Result<(), RouterError> {
        self.window
            .borrow_mut()
            .set_relative_mouse(enabled)
            .map_err(|error| RouterError::Platform(error.to_string()))
    }
}

/// Router controllers over shared controllers.
pub struct ControllersAdapter {
    /// Controllers cell.
    pub controllers: ControllersCell,
}

impl RouterControllers for ControllersAdapter {
    fn set_assignments(&mut self, selections: &[ControllerSelection]) -> Result<(), RouterError> {
        self.controllers
            .borrow_mut()
            .set_assignments(selections)
            .map_err(|error| RouterError::Platform(error.to_string()))
    }

    fn set_sensor_enabled(
        &mut self,
        instance: i32,
        sensor: ControllerSensor,
        enabled: bool,
    ) -> Result<ControllerOperationResult, RouterError> {
        self.controllers
            .borrow_mut()
            .set_sensor_enabled(instance, sensor, enabled)
            .map_err(|error| RouterError::Platform(error.to_string()))
    }

    fn poll_events(&mut self) -> Result<Vec<ControllerEvent>, RouterError> {
        self.controllers
            .borrow_mut()
            .poll_events()
            .map_err(|error| RouterError::Platform(error.to_string()))
    }

    fn snapshot(&mut self, instance: i32) -> Result<Option<ControllerState>, RouterError> {
        self.controllers
            .borrow()
            .snapshot(instance)
            .map_err(|error| RouterError::Platform(error.to_string()))
    }

    fn assignments(&mut self) -> Result<Vec<Option<i32>>, RouterError> {
        self.controllers
            .borrow()
            .assignments()
            .map_err(|error| RouterError::Platform(error.to_string()))
    }
}

/// Adopted console routing answers over live cells (donor routing adopted
/// into a startup).
///
/// Answers are computed through the canonical [`ApplicationConsoleRouting`]
/// on every call, so no routing rule is duplicated here.
#[derive(Clone)]
pub struct AdoptedRouting {
    /// Declaring session.
    pub session: SessionId,
    /// Source game dialect.
    pub source_dialect: Dialect,
    /// Fallback registry.
    pub fallback: SharedRegistry,
    /// Movement registry.
    pub movement: Option<SharedRegistry>,
    /// Shared registry.
    pub shared: Option<SharedRegistry>,
    /// Server console plus mirrorable names.
    pub server: Option<(SharedRegistry, Vec<String>)>,
    /// Seat registries by seat and client.
    pub seats: Vec<(SeatId, ClientId, SharedRegistry)>,
    /// Mouse registries by seat and client.
    pub mice: Vec<(SeatId, ClientId, SharedRegistry)>,
}

/// Pointer-to-slot map for routing answers.
struct SlotMapper {
    slots: Vec<(usize, RegistrySlot)>,
}

impl SlotMapper {
    /// Slot for a resolved registry, if it is a known cell.
    fn slot(&self, registry: &CvarRegistry) -> Option<RegistrySlot> {
        let address = std::ptr::from_ref(registry) as usize;
        self.slots
            .iter()
            .find(|(slot, _)| *slot == address)
            .map(|(_, slot)| *slot)
    }
}

fn startup_message(message: impl Into<String>) -> PreparedStartupError {
    PreparedStartupError::Message(message.into())
}

impl AdoptedRouting {
    /// Answer through canonical routing built from live borrows.
    fn with_routing<R>(
        &self,
        _source: &CommandContext,
        seats: &[SeatRegistries],
        answer: impl FnOnce(&ApplicationConsoleRouting<'_>, &SlotMapper) -> Result<R, PreparedStartupError>,
    ) -> Result<R, PreparedStartupError> {
        let fallback = self.fallback.borrow();
        let movement = self.movement.as_ref().map(|cell| cell.borrow());
        let shared = self.shared.as_ref().map(|cell| cell.borrow());
        let server = self.server.as_ref().map(|(cell, _)| cell.borrow());
        let seat_cells: Vec<_> = self.seats.iter().map(|(_, _, cell)| cell.borrow()).collect();
        let mouse_cells: Vec<_> = self.mice.iter().map(|(_, _, cell)| cell.borrow()).collect();
        let session = self.session.clone();
        let fallback_entry = ConsoleRegistry {
            registry: &fallback,
            session: session.clone(),
            origin: CommandOrigin::LocalConsole,
        };
        let movement_entry = movement.as_ref().map(|registry| ConsoleRegistry {
            registry,
            session: session.clone(),
            origin: CommandOrigin::LocalConsole,
        });
        let server_entry = server.as_ref().map(|registry| {
            let names = self.server.as_ref().map(|(_, names)| names.clone()).unwrap_or_default();
            ApplicationConsoleServer {
                cvars: ConsoleRegistry {
                    registry,
                    session: session.clone(),
                    origin: CommandOrigin::LocalConsole,
                },
                shared_names: names,
            }
        });
        let seat_entries: Vec<(SeatId, ConsoleRegistry<'_>)> = self
            .seats
            .iter()
            .zip(seat_cells.iter())
            .map(|((seat, client, _), registry)| {
                (
                    seat.clone(),
                    ConsoleRegistry {
                        registry,
                        session: session.clone(),
                        origin: CommandOrigin::LocalSeat {
                            seat: seat.clone(),
                            client: client.clone(),
                        },
                    },
                )
            })
            .collect();
        let mouse_entries: Vec<(SeatId, ConsoleRegistry<'_>)> = self
            .mice
            .iter()
            .zip(mouse_cells.iter())
            .map(|((seat, client, _), registry)| {
                (
                    seat.clone(),
                    ConsoleRegistry {
                        registry,
                        session: session.clone(),
                        origin: CommandOrigin::LocalSeat {
                            seat: seat.clone(),
                            client: client.clone(),
                        },
                    },
                )
            })
            .collect();
        let routing = ApplicationConsoleRouting::new(ApplicationConsoleRoutingOptions {
            fallback: fallback_entry,
            source_dialect: self.source_dialect,
            server: Box::new({
                let server_entry = server_entry.clone();
                move || server_entry.clone()
            }),
            seat: Box::new({
                let seat_entries = seat_entries.clone();
                move |seat: &SeatId| {
                    seat_entries
                        .iter()
                        .find(|(id, _)| id == seat)
                        .map(|(_, entry)| entry.clone())
                }
            }),
            input: Some(Box::new({
                let mouse_entries = mouse_entries.clone();
                move |seat: Option<&SeatId>| match seat {
                    Some(seat) => mouse_entries
                        .iter()
                        .find(|(id, _)| id == seat)
                        .map(|(_, entry)| entry.clone()),
                    None => mouse_entries.first().map(|(_, entry)| entry.clone()),
                }
            })),
            movement: Some(Box::new({
                let movement_entry = movement_entry.clone();
                move || movement_entry.clone()
            })),
            shared: shared.as_ref().map(|guard| {
                let registry: &CvarRegistry = guard;
                Box::new(move || Some(registry)) as super::console::SharedLookup<'_>
            }),
        })
        .map_err(|error| startup_message(error.to_string()))?;
        let mut slots = Vec::new();
        slots.push((std::ptr::from_ref(&*fallback) as usize, RegistrySlot::Fallback));
        if let Some(movement) = movement.as_ref() {
            slots.push((std::ptr::from_ref(&**movement) as usize, RegistrySlot::Movement));
        }
        if let Some(shared) = shared.as_ref() {
            slots.push((std::ptr::from_ref(&**shared) as usize, RegistrySlot::Shared));
        }
        if let Some(server) = server.as_ref() {
            slots.push((std::ptr::from_ref(&**server) as usize, RegistrySlot::Source));
        }
        for (index, seat) in seats.iter().enumerate() {
            slots.push((std::ptr::from_ref(seat.cvars) as usize, RegistrySlot::Seat(index)));
            slots.push((
                std::ptr::from_ref(seat.mouse_cvars) as usize,
                RegistrySlot::SeatMouse(index),
            ));
        }
        for ((seat, _, _), cell) in self.seats.iter().zip(seat_cells.iter()) {
            if let Some((index, _)) = seats.iter().enumerate().find(|(_, candidate)| candidate.id == *seat) {
                slots.push((std::ptr::from_ref(&**cell) as usize, RegistrySlot::Seat(index)));
            }
        }
        for ((seat, _, _), cell) in self.mice.iter().zip(mouse_cells.iter()) {
            if let Some((index, _)) = seats.iter().enumerate().find(|(_, candidate)| candidate.id == *seat) {
                slots.push((std::ptr::from_ref(&**cell) as usize, RegistrySlot::SeatMouse(index)));
            }
        }
        let mapper = SlotMapper { slots };
        answer(&routing, &mapper)
    }
}

impl StartupCvarRouting for AdoptedRouting {
    fn owner_slot(
        &self,
        name: &str,
        source: &CommandContext,
        seats: &[SeatRegistries],
    ) -> Result<RegistrySlot, PreparedStartupError> {
        self.with_routing(source, seats, |routing, mapper| {
            let owner = routing
                .owner(name, source)
                .map_err(|error| startup_message(error.to_string()))?;
            mapper
                .slot(owner)
                .ok_or_else(|| startup_message("Console routing resolved an unknown registry"))
        })
    }

    fn visible_slots(&self, source: &CommandContext, seats: &[SeatRegistries]) -> Vec<RegistrySlot> {
        self.with_routing(source, seats, |routing, mapper| {
            let visible = routing
                .visible(source)
                .map_err(|error| startup_message(error.to_string()))?;
            let mut slots = Vec::new();
            for registry in visible {
                let Some(slot) = mapper.slot(registry) else {
                    return Err(startup_message("Console routing resolved an unknown registry"));
                };
                if !slots.contains(&slot) {
                    slots.push(slot);
                }
            }
            Ok(slots)
        })
        .unwrap_or_default()
    }
}

/// Live seat device over a routed seat (donor `SeatInput` live state).
#[derive(Clone)]
pub struct InputSeatDevice {
    /// Seat id.
    pub seat: SeatId,
    /// Seat command context.
    pub context: CommandContext,
    /// Movement dialect.
    pub dialect: Dialect,
    /// Router cell.
    pub router: RouterCell,
    /// Seat contexts by seat id.
    pub contexts: Rc<RefCell<HashMap<SeatId, CommandContext>>>,
}

impl PreparedSeatDevice for InputSeatDevice {
    fn dialect(&self) -> Dialect {
        self.dialect
    }

    fn is_down(&self, input: &PhysicalInput) -> bool {
        self.router
            .borrow()
            .seat(&self.seat)
            .is_some_and(|seat| seat.is_down(input))
    }

    fn release(&mut self, time_ms: f64, append: &mut dyn FnMut(&str, &CommandContext)) {
        let mut registry = SinkRegistry {
            contexts: Rc::clone(&self.contexts),
            fallback: self.context.clone(),
            sink: append,
        };
        if let Some(seat) = self.router.borrow_mut().seat_mut(&self.seat) {
            seat.release(time_ms, &mut registry);
        }
    }

    fn has_held_input(&self) -> bool {
        self.router
            .borrow()
            .seat(&self.seat)
            .is_some_and(|seat| seat.has_held_input())
    }

    fn set_profile(&mut self, dialect: Dialect) {
        let mut router = self.router.borrow_mut();
        let seat = router.seat_mut(&self.seat).expect("seat device lost its routed seat");
        seat.set_profile(dialect)
            .expect("seat profile change requires released input");
        self.dialect = dialect;
    }
}

/// Saved stick curve as live client tuning.
fn client_stick_curve(value: &SavedStickCurve) -> ClientStickCurve {
    match *value {
        SavedStickCurve::Radial {
            deadzone,
            exponent,
            outer_threshold,
        } => ClientStickCurve::Radial {
            deadzone,
            outer_threshold,
            exponent,
        },
        SavedStickCurve::Axial { deadzone, exponent } => ClientStickCurve::Axial { deadzone, exponent },
    }
}

/// Saved gamepad tuning as live client tuning.
fn client_gamepad_tuning(value: &SavedGamepadTuning) -> GamepadTuning {
    GamepadTuning {
        stick_move: client_stick_curve(&value.move_curve),
        stick_look: client_stick_curve(&value.look_curve),
        swap_sticks: value.swap_sticks,
        yaw_degrees_per_second: value.yaw_degrees_per_second,
        pitch_degrees_per_second: value.pitch_degrees_per_second,
        invert_pitch: value.invert_pitch,
        forward_sensitivity: value.forward_sensitivity,
        side_sensitivity: value.side_sensitivity,
        trigger_threshold: value.trigger_threshold,
        gyro: qa_client::input::gamepad::GyroTuning {
            enabled: value.gyro.enabled,
            yaw_sensitivity: value.gyro.yaw_sensitivity,
            pitch_sensitivity: value.gyro.pitch_sensitivity,
            yaw_axis_y: matches!(value.gyro.yaw_axis, GyroYawAxis::Y),
        },
    }
}

/// Saved mouse tuning as live client tuning.
fn client_mouse_tuning(value: &SavedMouseTuning) -> MouseTuning {
    MouseTuning {
        sensitivity: value.sensitivity,
        acceleration: value.acceleration,
        filter: value.filter,
        yaw: value.yaw,
        pitch: value.pitch,
        side: value.side,
        forward: value.forward,
        free_look: value.free_look,
        look_spring: value.look_spring,
        look_strafe: value.look_strafe,
        invert_pitch: value.invert_pitch,
    }
}

/// Live controller selection as saved selection.
fn saved_controller_selection(value: &ControllerSelection) -> SavedControllerSelection {
    match value.clone() {
        ControllerSelection::Automatic => SavedControllerSelection::Automatic,
        ControllerSelection::None => SavedControllerSelection::None,
        ControllerSelection::Device { guid, ordinal } => SavedControllerSelection::Device { guid, ordinal },
        ControllerSelection::Serial { guid, serial } => SavedControllerSelection::Serial { guid, serial },
    }
}

/// Saved controller selection as live selection.
fn live_controller_selection(value: &SavedControllerSelection) -> ControllerSelection {
    match value.clone() {
        SavedControllerSelection::Automatic => ControllerSelection::Automatic,
        SavedControllerSelection::None => ControllerSelection::None,
        SavedControllerSelection::Device { guid, ordinal } => ControllerSelection::Device { guid, ordinal },
        SavedControllerSelection::Serial { guid, serial } => ControllerSelection::Serial { guid, serial },
    }
}

impl PreparedMouse for CvarRegistry {
    fn cvars(&self) -> &CvarRegistry {
        self
    }

    fn cvars_mut(&mut self) -> &mut CvarRegistry {
        self
    }

    fn write(&mut self, tuning: &SavedMouseTuning) {
        // Donor mouse tuning assignment never fails; keep current values when
        // a saved value does not apply.
        let _ = write_mouse_tuning(self, &client_mouse_tuning(tuning));
    }
}

impl PreparedScriptFiles for ScriptsCell {
    fn read_script(&self, name: &str, source: &CommandContext) -> Option<String> {
        self.borrow().read(name, source).unwrap_or(None)
    }

    fn write_text(&mut self, path: &str, contents: &str) -> Result<(), String> {
        self.borrow().write_text(path, contents)
    }
}

/// Map a startup error into an input error.
fn startup_error(error: PreparedStartupError) -> InputError {
    match error {
        PreparedStartupError::Buffer(error) => InputError::Buffer(error),
        PreparedStartupError::Cvar(error) => InputError::Cvar(error),
        other => InputError::Startup(other.to_string()),
    }
}

/// Bridge a startup-config read scope into a prepared-startup read.
fn bridge_scopes(mut read: StartupScriptRead) -> ScopedReadFn {
    Box::new(move |name: &str, source: &CommandContext, scope: PreparedScope| {
        let scope = match scope {
            PreparedScope::Mounted => StartupScriptScope::Mounted,
            PreparedScope::User => StartupScriptScope::User,
            PreparedScope::BaseLoose => StartupScriptScope::BaseLoose,
            PreparedScope::GameLoose => StartupScriptScope::GameLoose,
            PreparedScope::Loose => StartupScriptScope::Loose,
            PreparedScope::Seat => StartupScriptScope::Seat,
        };
        read(name, source, scope)
    })
}

/// Registry image from a live registry.
fn registry_image(registry: &CvarRegistry, fallback: &SessionId) -> RegistryImage {
    RegistryImage {
        dialect: registry.dialect(),
        session: registry.session().cloned().unwrap_or_else(|| fallback.clone()),
        state: registry.capture_world_transfer_state(),
    }
}

impl<C: PreparedStartupConfig + 'static> InputStartup
    for PreparedStartup<InputSeatDevice, CvarRegistry, AdoptedRouting, C, ScriptsCell>
{
    fn pending(&self) -> bool {
        self.pending()
    }

    fn registries(&self) -> StartupRegistryImages {
        let session = self.commands.context().session.clone();
        StartupRegistryImages {
            source: registry_image(&self.source, &session),
            movement: registry_image(self.movement(), &session),
            fallback: registry_image(self.fallback(), &session),
        }
    }

    fn commands_dialect(&self) -> Dialect {
        self.commands.dialect()
    }

    fn commands_context(&self) -> CommandContext {
        self.commands.context().clone()
    }

    fn commands_revision(&self) -> u64 {
        self.commands.program_revision()
    }

    fn movement_variable(&self, name: &str) -> f64 {
        f64::from(self.movement().variable_value(name))
    }

    fn copy_commands_pending(&self, target: &mut CommandBuffer) -> Result<(), InputError> {
        Ok(target.copy_pending_from(&self.commands)?)
    }

    fn seats(&self) -> Vec<StartupSeatView> {
        let session = self.commands.context().session.clone();
        self.seats()
            .iter()
            .map(|seat| StartupSeatView {
                id: seat.id.clone(),
                context: seat.context.clone(),
                cvars: registry_image(&seat.cvars, &session),
                mouse: registry_image(seat.mouse.cvars(), &session),
                device_dialect: seat.device.dialect(),
                has_held_input: seat.device.has_held_input(),
                authored_bindings: seat.authored_bindings.is_some(),
                bindings: Rc::clone(&seat.bindings),
            })
            .collect()
    }

    fn console_bindings(&self) -> Option<Rc<RefCell<qa_client::input::BindingTable>>> {
        self.console_bindings.clone()
    }

    fn scripts_cell(&self) -> ScriptsCell {
        Rc::clone(&self.scripts)
    }

    fn live_tags(&self) -> HashSet<u64> {
        let mut tags = HashSet::new();
        tags.insert(std::ptr::from_ref(&self.source) as u64);
        tags.insert(std::ptr::from_ref(self.movement()) as u64);
        tags.insert(std::ptr::from_ref(self.fallback()) as u64);
        for seat in self.seats() {
            tags.insert(std::ptr::from_ref(&seat.cvars) as u64);
            tags.insert(std::ptr::from_ref(seat.mouse.cvars()) as u64);
        }
        tags
    }

    fn adopt(
        &mut self,
        routing: AdoptedRouting,
        forward: ForwardFn,
        owners: AdoptedOwnersView,
    ) -> Result<(), InputError> {
        let adopted = AdoptedOwners {
            source: owners.source,
            movement: owners.movement,
            fallback: owners.fallback,
            scripts: owners.scripts,
            read: bridge_scopes(owners.read),
        };
        self.adopt(routing, forward, Some(adopted)).map_err(startup_error)
    }

    fn validate_owners(&self, owners: &AdoptedOwnersView) -> Result<(), InputError> {
        self.validate_owners(&owners.source, &owners.movement, &owners.fallback)
            .map_err(startup_error)
    }

    fn adopt_seat(&mut self, id: &SeatId, cvars: Option<CvarRegistry>, mouse: Option<CvarRegistry>) {
        self.adopt_seat(id, cvars, mouse);
    }

    fn publish_seats(
        &mut self,
        seats: Vec<InputPublishedSeat>,
        active: Vec<SeatId>,
        retention: SeatPublication,
    ) -> Result<(), InputError> {
        let seats = seats
            .into_iter()
            .map(|seat| PublishedSeat {
                id: seat.id,
                device: seat.device,
                context: seat.context,
                cvars: seat.cvars,
                mouse: seat.mouse,
                bindings: seat.bindings,
            })
            .collect();
        match retention {
            SeatPublication::Replace => self.publish_seats(seats, Some(active)),
            SeatPublication::Retain => self.publish_seats_retaining(seats, active),
        }
        .map_err(startup_error)
    }

    fn apply_binding_choices(&mut self, seat: &SeatId, choices: &BindingChoices) -> Result<(), InputError> {
        self.apply_binding_choices(
            seat,
            choices.overridden_keys.clone(),
            choices.all_bindings_chosen,
            choices.selected_bindings.clone(),
            choices.authored_bindings.clone(),
        )
        .map_err(startup_error)
    }

    fn set_active_seats(&mut self, ids: Vec<SeatId>) -> Result<(), InputError> {
        self.set_active_seats(ids).map_err(startup_error)
    }

    fn bind_output(&mut self, print: Rc<dyn Fn(&str, Option<&CommandContext>)>) -> Box<dyn FnOnce()> {
        let release = self.bind_output(move |text, source| print(text, source));
        Box::new(release)
    }

    fn allow_command(&mut self, argv: &[String], source: &CommandContext) -> bool {
        self.allow_command(argv, source)
    }

    fn note_world_action(&mut self) {
        self.note_world_action();
    }

    fn read_script(&mut self, name: &str, source: &CommandContext) -> Option<String> {
        self.read_script(name, source)
    }

    fn on_script_complete(&mut self, event: &ScriptCompletion) {
        self.on_script_complete(event);
    }

    fn bindings(&mut self, id: &SeatId, defaults: Vec<InputBinding>) -> Vec<InputBinding> {
        self.bindings(id, defaults)
    }

    fn adopt_binding_defaults(&mut self, id: &SeatId, defaults: Vec<InputBinding>) {
        self.adopt_binding_defaults(id, defaults);
    }

    fn preview_bindings(&self, id: &SeatId, defaults: &[InputBinding]) -> Vec<InputBinding> {
        self.preview_bindings(id, defaults)
    }

    fn reset_bindings(&mut self, id: &SeatId) -> Result<(), InputError> {
        self.reset_bindings(id).map_err(startup_error)
    }

    fn advance_frame(&mut self) -> Result<bool, InputError> {
        if !self.pending() {
            return Ok(false);
        }
        Err(InputError::Startup(
            "Prepared startup has pending work; drive the concrete startup with its factory".to_string(),
        ))
    }
}

/// Staging phase for candidate commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StagingPhase {
    /// Staging accepts commands and bindings.
    Ready,
    /// Staged state was published.
    Published,
}

/// Staged candidate client commands (donor `PreparedClientCommands` for the
/// input seats).
///
/// The canonical staged program borrows its seats mutably, so the input keeps
/// equivalent staging over its own seats: a program buffer plus per-seat
/// staged binding tables, published back into the live seats and buffer.
pub struct StagedClientCommands {
    /// Program command buffer.
    pub buffer: CommandBuffer,
    /// Live buffer revision when staging began.
    live_revision: u64,
    /// Staging phase.
    phase: StagingPhase,
    /// Staged binding tables by seat.
    staged: HashMap<SeatId, Rc<RefCell<qa_client::input::BindingTable>>>,
    /// Live bindings when staging began.
    originals: HashMap<SeatId, Vec<InputBinding>>,
    /// Cleared states by seat.
    cleared: HashMap<SeatId, Rc<RefCell<bool>>>,
    /// Live console bindings.
    console_live: Option<Rc<RefCell<qa_client::input::BindingTable>>>,
    /// Console bindings when staging began.
    console_original: Vec<InputBinding>,
    /// Staged console bindings.
    console_staged: Option<Rc<RefCell<qa_client::input::BindingTable>>>,
    /// Release dialect for staged releases.
    release_dialect: Dialect,
}

/// Staged program construction.
pub struct StagedProgram {
    /// Program command buffer.
    pub buffer: CommandBuffer,
    /// Live buffer revision when staging began.
    pub live_revision: u64,
    /// Live bindings by seat.
    pub live: Vec<(SeatId, Vec<InputBinding>)>,
    /// Live console bindings.
    pub console_live: Option<Rc<RefCell<qa_client::input::BindingTable>>>,
    /// Release dialect for staged releases.
    pub release_dialect: Dialect,
    /// Binding-command print sink.
    pub binding_print: Rc<dyn Fn(&str)>,
}

impl StagedClientCommands {
    /// Stage a program over live seat bindings.
    pub fn stage(program: StagedProgram) -> Result<Self, InputError> {
        let StagedProgram {
            mut buffer,
            live_revision,
            live,
            console_live,
            release_dialect,
            binding_print,
        } = program;
        let console_original = console_live.as_ref().map_or_else(Vec::new, table_bindings);
        let console_staged = console_live.as_ref().map(|live| {
            let staged = Rc::new(RefCell::new(qa_client::input::BindingTable::new()));
            for binding in table_bindings(live) {
                staged.borrow_mut().bind(binding);
            }
            staged
        });
        let mut staged = HashMap::new();
        let mut originals = HashMap::new();
        let mut cleared = HashMap::new();
        let mut tables = HashMap::new();
        for (seat, bindings) in live {
            let table = Rc::new(RefCell::new(qa_client::input::BindingTable::new()));
            for binding in &bindings {
                table.borrow_mut().bind(binding.clone());
            }
            tables.insert(seat.clone(), Rc::clone(&table));
            staged.insert(seat.clone(), table);
            originals.insert(seat.clone(), bindings);
            cleared.insert(seat, Rc::new(RefCell::new(false)));
        }
        let tables = Rc::new(tables);
        let lookup: BindingLookup = Rc::new(move |seat| tables.get(seat).cloned());
        let mut program_registry = ProgramRegistry { buffer: &mut buffer };
        register_binding_commands(&mut program_registry, lookup, binding_print, console_staged.clone());
        Ok(Self {
            buffer,
            live_revision,
            phase: StagingPhase::Ready,
            staged,
            originals,
            cleared,
            console_live,
            console_original,
            console_staged,
            release_dialect,
        })
    }

    /// Whether staging is still accepting work.
    fn check_ready(&self) -> Result<(), InputError> {
        if self.phase != StagingPhase::Ready {
            return Err(InputError::Startup(
                "Prepared command program already published".to_string(),
            ));
        }
        Ok(())
    }

    /// Staged table for a seat.
    pub fn staged_table(&self, seat: &SeatId) -> Option<Rc<RefCell<qa_client::input::BindingTable>>> {
        self.staged.get(seat).cloned()
    }

    /// Cleared flag for a seat.
    pub fn cleared_flag(&self, seat: &SeatId) -> Option<Rc<RefCell<bool>>> {
        self.cleared.get(seat).cloned()
    }

    /// Validate the publication against live seats and the live revision.
    pub fn validate_publication(
        &self,
        live_revision: u64,
        live: &dyn Fn(&SeatId) -> Vec<InputBinding>,
    ) -> Result<(), InputError> {
        self.check_ready()?;
        if live_revision != self.live_revision {
            return Err(InputError::Startup(
                "Authority command program changed during preparation".to_string(),
            ));
        }
        if let Some(live_table) = self.console_live.as_ref() {
            if table_bindings(live_table) != self.console_original {
                return Err(InputError::Startup(
                    "Console bindings changed during preparation".to_string(),
                ));
            }
        }
        for (seat, original) in &self.originals {
            if live(seat) != *original {
                return Err(InputError::Startup(
                    "Client bindings changed during preparation".to_string(),
                ));
            }
        }
        Ok(())
    }

    /// Release dialect for staged releases.
    #[must_use]
    pub fn release_dialect(&self) -> Dialect {
        self.release_dialect
    }

    /// Append through the release handle.
    pub fn release_append(&mut self, text: &str, source: &CommandContext) {
        let _ = self.buffer.append(text, Some(source), Some(self.release_dialect));
    }

    /// Publish staged bindings into live tables.
    pub fn publish_bindings(&mut self, live: &mut dyn FnMut(&SeatId, Vec<InputBinding>)) -> Result<(), InputError> {
        self.check_ready()?;
        if let (Some(live_table), Some(staged)) = (self.console_live.as_ref(), self.console_staged.as_ref()) {
            let bindings = table_bindings(staged);
            let mut live_table = live_table.borrow_mut();
            live_table.unbind_all();
            for binding in bindings {
                live_table.bind(binding);
            }
        }
        for (seat, staged) in &self.staged {
            live(seat, table_bindings(staged));
        }
        self.phase = StagingPhase::Published;
        Ok(())
    }
}

/// Clone every binding out of a shared table.
fn table_bindings(table: &Rc<RefCell<qa_client::input::BindingTable>>) -> Vec<InputBinding> {
    table.borrow().bindings().into_iter().cloned().collect()
}

/// Command registry borrowing a program buffer mutably for registration.
struct ProgramRegistry<'a> {
    buffer: &'a mut CommandBuffer,
}

impl CommandRegistry for ProgramRegistry<'_> {
    fn register_engine(&mut self, name: &str, handler: ClientRegistryHandler) -> bool {
        self.register_inner(name, handler)
    }

    fn register(&mut self, name: &str, handler: ClientRegistryHandler) -> bool {
        self.register_inner(name, handler)
    }

    fn unregister(&mut self, name: &str) {
        self.buffer.unregister(name);
    }

    fn exists(&self, name: &str) -> bool {
        self.buffer.exists(name)
    }

    fn append(&mut self, text: &str, _seat: &SeatId) {
        let _ = self.buffer.append(text, None, None);
    }
}

impl ProgramRegistry<'_> {
    fn register_inner(&mut self, name: &str, handler: ClientRegistryHandler) -> bool {
        if self.buffer.exists(name) {
            return false;
        }
        let handler = Rc::new(RefCell::new(handler));
        let wrapped: CommandHandler = Rc::new(move |invocation: &mut Invocation| {
            let argv = invocation.argv.clone();
            let call = CommandInvocation::new(argv, convert_origin(&invocation.source.origin), invocation.dialect);
            let outcome = (*handler.borrow_mut())(&call);
            if let Err(error) = outcome {
                invocation.print(&format!("{error}\n"));
            }
        });
        // Staged program buffers start empty and the caller owns the names,
        // so the collision check runs against an empty registry.
        let empty = CvarRegistry::new(self.buffer.dialect());
        self.buffer.register(name, Some(wrapped), None, &empty).unwrap_or(false)
    }
}

/// Slot kind for guest cvar owners.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SlotKind {
    /// Fallback (console) registry.
    Fallback,
    /// Movement registry.
    Movement,
    /// Shared registry.
    Shared,
    /// Server registry.
    Server,
    /// Seat registry.
    Seat,
    /// Mouse registry.
    Mouse,
    /// Redirect target outside the routing inputs.
    Redirect,
}

/// Q3 guest cvar owners over borrowed live cells.
pub struct InputQ3Owners<'a> {
    /// Live cells, slot order plus redirect targets.
    guards: Vec<RefMut<'a, CvarRegistry>>,
    /// Slot kinds parallel to `guards`.
    layout: Vec<SlotKind>,
    /// Cells parallel to `guards` for identity mapping.
    order: Vec<SharedRegistry>,
    /// Candidate-to-retained redirects.
    published: Rc<RefCell<HashMap<usize, SharedRegistry>>>,
    /// External routing, when the input was adopted from an owner.
    external: Option<&'a dyn InputCvarRouting>,
    /// Print router.
    print: PrintRouter,
    /// Seat command context.
    context: CommandContext,
    /// Server mirrorable names.
    server_names: Vec<String>,
    /// Declaring session.
    session: SessionId,
    /// Source game dialect.
    source_dialect: Dialect,
}

impl InputQ3Owners<'_> {
    /// Routing slot count (redirect targets excluded).
    fn slot_count(&self) -> usize {
        self.layout.iter().filter(|kind| **kind != SlotKind::Redirect).count()
    }

    /// Guard index for a routing slot kind.
    fn slot_guard(&self, kind: SlotKind) -> Option<usize> {
        self.layout.iter().position(|slot| *slot == kind)
    }

    /// Resolve a routed registry to its guard index.
    fn guard_index(&self, registry: &CvarRegistry) -> Option<usize> {
        let address = std::ptr::from_ref(registry) as usize;
        self.guards
            .iter()
            .position(|guard| std::ptr::from_ref(&**guard) as usize == address)
    }

    /// Answer through canonical routing built from the guards.
    fn route<T>(&self, answer: impl FnOnce(&ApplicationConsoleRouting<'_>) -> Result<T, ConsoleError>) -> T {
        let fallback = self.slot_guard(SlotKind::Fallback).map(|index| &*self.guards[index]);
        let Some(fallback) = fallback else {
            panic!("guest cvars lost their fallback registry");
        };
        let movement = self.slot_guard(SlotKind::Movement).map(|index| &*self.guards[index]);
        let shared = self.slot_guard(SlotKind::Shared).map(|index| &*self.guards[index]);
        let server = self.slot_guard(SlotKind::Server).map(|index| &*self.guards[index]);
        let seat = self.slot_guard(SlotKind::Seat).map(|index| &*self.guards[index]);
        let mouse = self.slot_guard(SlotKind::Mouse).map(|index| &*self.guards[index]);
        let (seat_id, client_id) = match root_origin(&self.context.origin) {
            CommandOrigin::LocalSeat { seat, client } => (seat.clone(), client.clone()),
            _ => panic!("guest cvars require a local-seat context"),
        };
        let session = self.session.clone();
        let fallback_entry = ConsoleRegistry {
            registry: fallback,
            session: session.clone(),
            origin: CommandOrigin::LocalConsole,
        };
        let movement_entry = movement.map(|registry| ConsoleRegistry {
            registry,
            session: session.clone(),
            origin: CommandOrigin::LocalConsole,
        });
        let server_entry = server.map(|registry| ApplicationConsoleServer {
            cvars: ConsoleRegistry {
                registry,
                session: session.clone(),
                origin: CommandOrigin::LocalConsole,
            },
            shared_names: self.server_names.clone(),
        });
        let seat_entry = seat.map(|registry| ConsoleRegistry {
            registry,
            session: session.clone(),
            origin: CommandOrigin::LocalSeat {
                seat: seat_id.clone(),
                client: client_id.clone(),
            },
        });
        let mouse_entry = mouse.map(|registry| ConsoleRegistry {
            registry,
            session: session.clone(),
            origin: CommandOrigin::LocalSeat {
                seat: seat_id,
                client: client_id,
            },
        });
        let routing = ApplicationConsoleRouting::new(ApplicationConsoleRoutingOptions {
            fallback: fallback_entry,
            source_dialect: self.source_dialect,
            server: Box::new({
                let server_entry = server_entry.clone();
                move || server_entry.clone()
            }),
            seat: Box::new({
                let seat_entry = seat_entry.clone();
                move |_| seat_entry.clone()
            }),
            input: Some(Box::new({
                let mouse_entry = mouse_entry.clone();
                move |_| mouse_entry.clone()
            })),
            movement: Some(Box::new({
                let movement_entry = movement_entry.clone();
                move || movement_entry.clone()
            })),
            shared: shared.map(|registry| Box::new(move || Some(registry)) as super::console::SharedLookup<'_>),
        })
        .expect("guest cvars lost their console dialect");
        answer(&routing).expect("guest cvar routing failed")
    }
}

impl Q3ClientCvarOwners for InputQ3Owners<'_> {
    fn owner(&mut self, name: &str) -> usize {
        if let Some(external) = self.external {
            let index = external.owner(name, &self.context).expect("external routing failed");
            assert!(
                index < self.slot_count(),
                "external routing owner outside its visible set"
            );
            return self.current(index);
        }
        let position = self.route(|routing| {
            let registry = routing.owner(name, &self.context)?;
            self.guard_index(registry).ok_or(ConsoleError::ForeignSeatRegistry)
        });
        self.current(position)
    }

    fn visible(&self) -> Vec<usize> {
        if let Some(external) = self.external {
            let visible = external.visible(&self.context).expect("external routing failed");
            assert!(
                visible.iter().all(|index| *index < self.slot_count()),
                "external routing visible set outside its table"
            );
            return visible
                .into_iter()
                .map(|index| {
                    let candidate = Rc::as_ptr(&self.order[index]) as usize;
                    self.published
                        .borrow()
                        .get(&candidate)
                        .and_then(|retained| self.order.iter().position(|cell| Rc::ptr_eq(cell, retained)))
                        .unwrap_or(index)
                })
                .collect();
        }
        self.route(|routing| {
            let visible = routing.visible(&self.context)?;
            visible
                .into_iter()
                .map(|registry| self.guard_index(registry).ok_or(ConsoleError::ForeignSeatRegistry))
                .collect::<Result<Vec<_>, _>>()
        })
    }

    fn current(&mut self, owner: usize) -> usize {
        let candidate = Rc::as_ptr(&self.order[owner]) as usize;
        self.published
            .borrow()
            .get(&candidate)
            .and_then(|retained| self.order.iter().position(|cell| Rc::ptr_eq(cell, retained)))
            .filter(|index| *index < self.guards.len())
            .unwrap_or(owner)
    }

    fn registry(&self, index: usize) -> &CvarRegistry {
        &self.guards[index]
    }

    fn registry_mut(&mut self, index: usize) -> &mut CvarRegistry {
        &mut self.guards[index]
    }

    fn print(&mut self, text: &str) {
        let context = self.context.clone();
        self.print.print(text, Some(&context));
    }
}

/// Live Q3 guest cvar session (donor `guestCvars` result).
///
/// The session holds exclusive guards over the routed cells; keep sessions
/// short (one trap batch) and re-acquire after publication events.
pub struct GuestCvarSession<'a> {
    /// Guest cvars.
    cvars: Q3ClientCvars<InputQ3Owners<'a>>,
}

impl<'a> GuestCvarSession<'a> {
    /// Borrow the guest cvars.
    pub fn cvars(&mut self) -> &mut Q3ClientCvars<InputQ3Owners<'a>> {
        &mut self.cvars
    }
}

/// Live guest seat input (donor `guestInput` result).
pub struct GuestSeatInput {
    /// Router cell.
    pub router: RouterCell,
    /// Seat id.
    pub seat: SeatId,
    /// Staged table while a candidate is prepared.
    pub staged: Option<Rc<RefCell<qa_client::input::BindingTable>>>,
    /// Cleared flag while a candidate is prepared.
    pub cleared: Option<Rc<RefCell<bool>>>,
    /// Seat contexts by seat id.
    pub contexts: Rc<RefCell<HashMap<SeatId, CommandContext>>>,
    /// Seat command context.
    pub context: CommandContext,
    /// Millisecond clock.
    pub now: InputClock,
}

impl QvmClientInput for GuestSeatInput {
    fn is_down(&mut self, input: &PhysicalInput) -> bool {
        if self.cleared.as_ref().is_some_and(|cleared| *cleared.borrow()) {
            return false;
        }
        self.router
            .borrow()
            .seat(&self.seat)
            .is_some_and(|seat| seat.is_down(input))
    }

    fn binding(&mut self, input: &PhysicalInput) -> Option<InputBindingTarget> {
        if let Some(staged) = self.staged.as_ref() {
            return staged.borrow().binding(input).cloned();
        }
        self.router
            .borrow()
            .seat(&self.seat)
            .and_then(|seat| seat.binding(input).cloned())
    }

    fn bindings(&self) -> Vec<InputBinding> {
        if let Some(staged) = self.staged.as_ref() {
            return table_bindings(staged);
        }
        self.router
            .borrow()
            .seat(&self.seat)
            .map(|seat| seat.bindings().into_iter().cloned().collect())
            .unwrap_or_default()
    }

    fn bind(&mut self, input: PhysicalInput, command: &str) {
        let binding = InputBinding {
            input,
            target: InputBindingTarget::Command(command.to_string()),
        };
        if let Some(staged) = self.staged.as_ref() {
            staged.borrow_mut().bind(binding);
        } else if let Some(seat) = self.router.borrow_mut().seat_mut(&self.seat) {
            seat.bind(binding);
        }
    }

    fn unbind(&mut self, input: &PhysicalInput) {
        if let Some(staged) = self.staged.as_ref() {
            staged.borrow_mut().unbind(input);
        } else if let Some(seat) = self.router.borrow_mut().seat_mut(&self.seat) {
            seat.unbind(input);
        }
    }

    fn unbind_all(&mut self) {
        if let Some(staged) = self.staged.as_ref() {
            staged.borrow_mut().unbind_all();
        } else if let Some(seat) = self.router.borrow_mut().seat_mut(&self.seat) {
            seat.unbind_all();
        }
    }

    fn clear_states(&mut self) {
        if let Some(cleared) = self.cleared.as_ref() {
            *cleared.borrow_mut() = true;
        } else if let Some(seat) = self.router.borrow_mut().seat_mut(&self.seat) {
            let mut null = NullRegistry;
            seat.release((self.now)(), &mut null);
        }
    }
}

/// Translate a seat focus to a UI focus.
fn seat_focus_to_input(focus: &SeatFocus) -> SeatInputFocus {
    match focus {
        SeatFocus::Game => SeatInputFocus::Game,
        SeatFocus::Menu { menu, control } => SeatInputFocus::Menu {
            menu: UiMenuId::new(menu).expect("menu focus carries a menu id"),
            control: control
                .as_ref()
                .map(|control| UiControlId::new(control).expect("menu focus carries a control id")),
        },
        SeatFocus::Console => SeatInputFocus::Console,
        SeatFocus::Chat { team, text } => SeatInputFocus::Chat {
            team: *team,
            text: text.clone(),
        },
    }
}

/// Translate a seat focus to a console focus.
fn seat_focus_to_console(focus: &SeatFocus) -> ConsoleFocus {
    match focus {
        SeatFocus::Game | SeatFocus::Menu { .. } => ConsoleFocus::Game,
        SeatFocus::Console => ConsoleFocus::Console,
        SeatFocus::Chat { team, .. } => ConsoleFocus::Chat { team: *team },
    }
}

/// Translate a router event to a console event.
fn router_event_to_console(event: &RouterSeatEvent, repeat: bool) -> Option<ConsoleInputEvent> {
    match event {
        RouterSeatEvent::Focus { focused, .. } => Some(ConsoleInputEvent::Focus { focused: *focused }),
        RouterSeatEvent::Key { code, down, .. } => Some(ConsoleInputEvent::Key {
            code: *code,
            down: *down,
            repeat,
        }),
        RouterSeatEvent::Text { text, .. } => Some(ConsoleInputEvent::Text(text.clone())),
        RouterSeatEvent::MouseWheel { delta, .. } => Some(ConsoleInputEvent::MouseWheel { delta: delta.1 }),
        _ => None,
    }
}

/// Event time for a router event.
fn router_event_time(event: &RouterSeatEvent) -> f64 {
    match event {
        RouterSeatEvent::Focus { time_ms, .. }
        | RouterSeatEvent::Key { time_ms, .. }
        | RouterSeatEvent::Text { time_ms, .. }
        | RouterSeatEvent::MouseMotion { time_ms, .. }
        | RouterSeatEvent::MouseButton { time_ms, .. }
        | RouterSeatEvent::MouseWheel { time_ms, .. }
        | RouterSeatEvent::ControllerButton { time_ms, .. }
        | RouterSeatEvent::ControllerAxis { time_ms, .. } => *time_ms,
    }
}

/// Translate a router event to a UI event.
fn router_event_to_ui(seat: &SeatId, time_ms: i64, repeat: bool, event: &RouterSeatEvent) -> Option<SeatInputEvent> {
    let kind = match event {
        RouterSeatEvent::Focus { focused, .. } => SeatInputEventKind::Focus { focused: *focused },
        RouterSeatEvent::Key { code, down, .. } => SeatInputEventKind::Key {
            code: *code,
            down: *down,
            repeat,
        },
        RouterSeatEvent::Text { text, .. } => SeatInputEventKind::Text { text: text.clone() },
        RouterSeatEvent::MouseMotion { position, delta, .. } => SeatInputEventKind::MouseMotion {
            position: Vec2 {
                x: position.0 as f32,
                y: position.1 as f32,
            },
            delta: Vec2 {
                x: delta.0 as f32,
                y: delta.1 as f32,
            },
        },
        RouterSeatEvent::MouseButton { button, down, .. } => SeatInputEventKind::MouseButton {
            button: i32::from(*button),
            down: *down,
        },
        RouterSeatEvent::MouseWheel { delta, .. } => SeatInputEventKind::MouseWheel {
            delta: Vec2 {
                x: delta.0 as f32,
                y: delta.1 as f32,
            },
        },
        RouterSeatEvent::ControllerButton {
            device, button, down, ..
        } => SeatInputEventKind::ControllerButton {
            device: *device,
            button: i32::from(*button),
            down: *down,
        },
        RouterSeatEvent::ControllerAxis {
            device, axis, value, ..
        } => SeatInputEventKind::ControllerAxis {
            device: *device,
            axis: *axis,
            value: *value as f32,
        },
    };
    Some(SeatInputEvent {
        seat: seat.clone(),
        time_ms,
        kind,
    })
}

/// Translate a UI event to a router event.
fn ui_event_to_router(event: &SeatInputEvent) -> RouterSeatEvent {
    let time_ms = event.time_ms as f64;
    match &event.kind {
        SeatInputEventKind::Focus { focused } => RouterSeatEvent::Focus {
            seat: event.seat.clone(),
            time_ms,
            focused: *focused,
        },
        SeatInputEventKind::Key { code, down, .. } => RouterSeatEvent::Key {
            seat: event.seat.clone(),
            time_ms,
            code: *code,
            down: *down,
        },
        SeatInputEventKind::Text { text } => RouterSeatEvent::Text {
            seat: event.seat.clone(),
            time_ms,
            text: text.clone(),
        },
        SeatInputEventKind::MouseMotion { position, delta } => RouterSeatEvent::MouseMotion {
            seat: event.seat.clone(),
            time_ms,
            position: (f64::from(position.x), f64::from(position.y)),
            delta: (f64::from(delta.x), f64::from(delta.y)),
        },
        SeatInputEventKind::MouseButton { button, down } => RouterSeatEvent::MouseButton {
            seat: event.seat.clone(),
            time_ms,
            button: *button as u8,
            down: *down,
        },
        SeatInputEventKind::MouseWheel { delta } => RouterSeatEvent::MouseWheel {
            seat: event.seat.clone(),
            time_ms,
            delta: (f64::from(delta.x), f64::from(delta.y)),
        },
        SeatInputEventKind::ControllerButton { device, button, down } => RouterSeatEvent::ControllerButton {
            seat: event.seat.clone(),
            time_ms,
            device: *device,
            button: *button as u8,
            down: *down,
        },
        SeatInputEventKind::ControllerAxis { device, axis, value } => RouterSeatEvent::ControllerAxis {
            seat: event.seat.clone(),
            time_ms,
            device: *device,
            axis: *axis,
            value: f64::from(*value),
        },
    }
}

/// Execute a UI command through weapon hooks or the application.
///
/// Shared by the Quake II execute callback and the UI command handlers
/// (donor `executeUiCommand`).
fn execute_ui_command(
    seat_ui: &Rc<RefCell<HashMap<SeatId, UiCell>>>,
    actions: &ActionsCell,
    name: &str,
    args: &[String],
    seat: Option<&SeatId>,
    source: &CommandContext,
) {
    if let Some(seat) = seat {
        if (name == "weapnext" || name == "weapprev")
            && seat_ui
                .borrow()
                .get(seat)
                .is_some_and(|ui| ui.borrow_mut().cycle_weapon(if name == "weapnext" { 1 } else { -1 }))
        {
            return;
        }
        if name == "switchweapon" && args.len() == 2 {
            let first = args[0].parse::<i32>();
            let second = args[1].parse::<i32>();
            if let (Ok(first), Ok(second)) = (first, second) {
                if seat_ui
                    .borrow()
                    .get(seat)
                    .is_some_and(|ui| ui.borrow_mut().switch_weapon(first, second))
                {
                    return;
                }
            }
        }
    }
    actions.borrow_mut().execute(name, args, seat, source);
}

/// Staged command queued before activation.
enum StagedCommand {
    /// Console text with its source.
    Console {
        /// Command text.
        text: String,
        /// Command source.
        source: CommandContext,
    },
    /// Reliable command with its dispatch.
    Reliable {
        /// Command text.
        text: String,
        /// Command source.
        source: CommandContext,
        /// Dispatch sink.
        dispatch: Box<dyn FnOnce(String, CommandContext)>,
    },
}

/// Registered command release.
enum Unregister {
    /// Buffer command name.
    Command(String),
    /// Console command name.
    Console(String),
    /// Quake I client commands.
    Q1(Q1ClientCommandGuard),
    /// Quake II client commands.
    Q2(Q2ClientCommandGuard),
    /// Quake I view commands.
    View(Q1ViewCommandGuard),
    /// Startup output binding.
    Output(Box<dyn FnOnce()>),
}

/// Arsenal selection for one seat (donor `Pick<ArsenalIntent, "provider" | "weapon">`).
#[derive(Debug, Clone)]
pub struct ArsenalSelection {
    /// Arsenal provider.
    pub provider: ProviderId,
    /// Selected weapon.
    pub weapon: Option<String>,
}

/// Offhand buttons for one seat.
#[derive(Debug, Clone, Default)]
struct OffhandButtons {
    /// Grapple button.
    grapple: InputButton,
    /// Grenade button.
    grenade: InputButton,
}

/// Loaded seat settings (donor `readSettings` result).
struct LoadedSettings {
    /// Saved seat settings by seat order.
    saved: Vec<Option<SeatSettings>>,
    /// Saved input routing.
    routing: Option<InputRouting>,
    /// Saved movement archive.
    movement: Vec<CvarArchiveEntry>,
    /// Saved fallback archive.
    fallback: Vec<CvarArchiveEntry>,
    /// Saved mouse archives by seat order.
    input: Vec<Vec<CvarArchiveEntry>>,
}

/// Application input construction parameters.
pub struct ApplicationInputOpen<'a> {
    /// Application window cell.
    pub window: WindowCell,
    /// Local players.
    pub players: Vec<LocalPlayer>,
    /// Application options.
    pub options: ApplicationOptions,
    /// Movement dialect.
    pub dialect: Dialect,
    /// Declaring session.
    pub session: SessionId,
    /// Player-view source.
    pub simulation: Rc<RefCell<dyn InputPlayerView>>,
    /// Application callbacks.
    pub actions: ActionsCell,
    /// Millisecond clock.
    pub now: InputClock,
    /// Settings store.
    pub settings: ConfigStore,
    /// Adopted command owners.
    pub owner: Option<ApplicationInputCommandOwner>,
    /// Previous input, if preparing a replacement.
    pub previous: Option<&'a ApplicationInput>,
    /// Prepared startup.
    pub prepared: Option<StartupCell>,
    /// Controllers cell override.
    pub controllers: Option<ControllersCell>,
    /// Input devices override.
    pub devices: Option<DeviceCell>,
    /// Menu controller settings for profile inheritance.
    pub menu_settings: Option<&'a ControllerSettings<RouterHandle, SharedDevicesFn>>,
    /// MIDI input boundary for fresh devices.
    pub midi_boundary: Box<dyn MidiInputBoundary>,
    /// Joystick opener for fresh devices.
    pub joystick_opener: JoystickOpener,
}

/// Local input orchestration (donor `ApplicationInput`).
pub struct ApplicationInput {
    /// Application options.
    pub options: ApplicationOptions,
    /// Movement dialect.
    pub dialect: Dialect,
    /// Declaring session.
    session: SessionId,
    /// Command buffer.
    commands: SharedCommands,
    /// Console command registry.
    console_commands: Rc<RefCell<ConsoleCommands>>,
    /// Movement cvars.
    cvars: SharedRegistry,
    /// Console (fallback) cvars.
    console_cvars: SharedRegistry,
    /// Shared settings registry.
    shared: Option<SharedRegistry>,
    /// Candidate-to-retained registry redirects.
    published: Rc<RefCell<HashMap<usize, SharedRegistry>>>,
    /// Mouse registries by seat.
    mouse: HashMap<SeatId, SharedRegistry>,
    /// Seat cvar cells by seat (console-shared or input-owned).
    ///
    /// Guest sessions borrow these through `&self`, so console cells are
    /// cached here by `Rc` at wire-up time instead of borrowed per call.
    seat_cvars: HashMap<SeatId, SharedRegistry>,
    /// Server console plus mirrorable names (console-shared).
    server_cvars: Option<(SharedRegistry, Vec<String>)>,
    /// Console scripts.
    scripts: ScriptsCell,
    /// Local seats.
    locals: Vec<LocalInput>,
    /// Input router.
    router: RouterCell,
    /// Router handle.
    handle: RouterHandle,
    /// Binding-command tables by seat.
    shadows: HashMap<SeatId, Rc<RefCell<qa_client::input::BindingTable>>>,
    /// Controllers.
    controllers: ControllersCell,
    /// Whether this input closes the controllers.
    owns_controllers: bool,
    /// Input devices.
    devices: DeviceCell,
    /// Whether this input closes the devices.
    owns_devices: bool,
    /// Controller settings.
    settings_ctrl: ControllerSettings<RouterHandle, SharedDevicesFn>,
    /// Application window.
    window: WindowCell,
    /// Pending window events from a previous input.
    pending_window: Vec<SdlEvent>,
    /// Pending controller events from a previous input.
    pending_controller: Vec<ControllerEvent>,
    /// Client command bindings.
    client_commands: Rc<RefCell<ClientCommandBindings>>,
    /// Whether commands are active.
    commands_active: bool,
    /// Staged pre-activation commands.
    staged: Vec<StagedCommand>,
    /// Registered command releases.
    unregister: Vec<Unregister>,
    /// UI callback ids by seat.
    ui_ids: HashMap<SeatId, u64>,
    /// Seat UI by seat.
    seat_ui: Rc<RefCell<HashMap<SeatId, UiCell>>>,
    /// Quake III selections by seat.
    q3_selections: HashMap<SeatId, Q3CommandSelection>,
    /// Arsenal selections by seat.
    arsenals: HashMap<SeatId, ArsenalSelection>,
    /// Offhand buttons by seat.
    offhand: Rc<RefCell<HashMap<SeatId, OffhandButtons>>>,
    /// Staged candidate commands.
    candidate: Option<StagedClientCommands>,
    /// Candidate binding defaults by seat.
    candidate_defaults: HashMap<SeatId, Vec<InputBinding>>,
    /// Candidate gamepad tunings by seat.
    candidate_gamepads: HashMap<SeatId, GamepadTuning>,
    /// Prepared startup.
    startup: Option<StartupCell>,
    /// Whether the configuration published its continuation.
    configuration_published: bool,
    /// Whether archives persist.
    archive_persistence: bool,
    /// Input command sequence.
    sequence: u64,
    /// Application callbacks.
    actions: ActionsCell,
    /// Print router.
    print_router: PrintRouter,
    /// Millisecond clock.
    now: InputClock,
    /// Settings store.
    settings: ConfigStore,
    /// Player-view source.
    simulation: Rc<RefCell<dyn InputPlayerView>>,
    /// External cvar routing.
    external_routing: Option<Box<dyn InputCvarRouting>>,
    /// Haptic sample loader.
    haptic_loader: LoaderCell,
    /// Queued focus changes from router callbacks.
    pending_focus: Rc<RefCell<HashMap<SeatId, SeatFocus>>>,
}

impl ApplicationInput {
    /// Open fresh input.
    pub fn open(params: ApplicationInputOpen<'_>) -> Result<Self, InputError> {
        Self::create(params, None, false)
    }

    /// Prepare staged input over live cvar owners.
    pub fn prepare(params: ApplicationInputOpen<'_>, live_tags: HashSet<u64>) -> Result<Self, InputError> {
        Self::create(params, Some(live_tags), false)
    }

    /// Prepare staged input for a client.
    pub fn prepare_for_client(
        mut params: ApplicationInputOpen<'_>,
        client: &mut dyn InputClient,
        live_tags: HashSet<u64>,
    ) -> Result<Self, InputError> {
        if !Rc::ptr_eq(&params.window, &client.renderer_window()) {
            return Err(InputError::PreparedOwners);
        }
        let prepared = client.prepared_cell();
        if let Some(params_prepared) = params.prepared.as_ref() {
            if !Rc::ptr_eq(params_prepared, &prepared) {
                return Err(InputError::PreparedOwners);
            }
        }
        params.prepared = Some(prepared);
        if params.controllers.is_none() {
            params.controllers = Some(client.controllers_cell());
        }
        if params.devices.is_none() {
            params.devices = Some(client.devices_cell());
        }
        let platform = client.platform();
        let guard = platform.borrow();
        // The guard lives across `create`; struct update shrinks the bundle
        // lifetime so the platform borrows stay local.
        match guard.as_ref() {
            Some(InputClientPlatform::World { input }) => {
                if input.locals.len() != params.players.len() {
                    return Err(InputError::TravelSeats);
                }
                let params = ApplicationInputOpen {
                    previous: Some(input),
                    ..params
                };
                Self::create(params, Some(live_tags), true)
            }
            Some(InputClientPlatform::Menu { settings, .. }) => {
                let params = ApplicationInputOpen {
                    menu_settings: Some(settings),
                    ..params
                };
                Self::create(params, Some(live_tags), true)
            }
            None => Self::create(params, Some(live_tags), true),
        }
    }
}

/// Load archives and seat settings (donor `readSettings`).
#[allow(clippy::too_many_arguments)]
fn read_settings(
    seats: &[SeatId],
    dialect: Dialect,
    source_dialect: Dialect,
    session: &SessionId,
    settings: &ConfigStore,
    owner: Option<&ApplicationInputCommandOwner>,
    previous: Option<&ApplicationInput>,
    startup: Option<&StartupCell>,
) -> Result<LoadedSettings, InputError> {
    let mut saved = Vec::new();
    let mut input = Vec::new();
    for seat in seats {
        saved.push(settings.load_seat(&seat_settings_path(seat.index()))?);
        let skip = owner
            .and_then(|owner| owner.input_settings.as_ref())
            .is_some_and(|cell| {
                let cell = cell.borrow();
                cell.dialect() == source_dialect && cell.session() == Some(session)
            })
            || previous.is_some_and(|previous| previous.mouse.contains_key(seat))
            || startup.is_some_and(|startup| startup.borrow().seats().iter().any(|view| view.id == *seat));
        if skip {
            input.push(Vec::new());
            continue;
        }
        let archive_owner = CvarArchiveOwner::new(
            CvarArchiveHead::Input,
            vec![dialect_name(source_dialect).to_string(), seat.index().to_string()],
        );
        input.push(load_cvar_archive(settings, &archive_owner, source_dialect)?);
    }
    let has_owners = owner.is_some() || previous.is_some() || startup.is_some();
    let movement = if has_owners {
        Vec::new()
    } else {
        let archive_owner = CvarArchiveOwner::new(CvarArchiveHead::Movement, vec![dialect_name(dialect).to_string()]);
        load_cvar_archive(settings, &archive_owner, dialect)?
    };
    let fallback = if has_owners {
        Vec::new()
    } else {
        let archive_owner = CvarArchiveOwner::new(
            CvarArchiveHead::Fallback,
            vec![dialect_name(source_dialect).to_string()],
        );
        load_cvar_archive(settings, &archive_owner, source_dialect)?
    };
    Ok(LoadedSettings {
        saved,
        routing: settings.load_input_routing("input/routing.json")?,
        movement,
        fallback,
        input,
    })
}

/// Fresh registry with a session.
fn seed_registry(dialect: Dialect, session: &SessionId) -> CvarRegistry {
    CvarRegistry::with_session(dialect, Some(session.clone()))
}

/// Restore a world-transfer image into a registry.
fn apply_state(registry: &mut CvarRegistry, state: &qa_core::cvar::CvarSaveState) -> Result<(), InputError> {
    Ok(registry.restore_save_state(state)?)
}

/// Buffer services routing through startup, scripts, and actions.
struct InputBufferServices {
    /// Prepared startup.
    startup: Option<StartupCell>,
    /// Console scripts.
    scripts: ScriptsCell,
    /// Application callbacks.
    actions: ActionsCell,
}

impl BufferServices for InputBufferServices {
    fn forward_to_server(&mut self, command: &ForwardedCommand) {
        let Some(name) = command.argv.first().cloned() else {
            return;
        };
        if let Some(startup) = self.startup.as_ref() {
            startup.borrow_mut().note_world_action();
        }
        let seat = match root_origin(&command.source.origin) {
            CommandOrigin::LocalSeat { seat, .. } => Some(seat.clone()),
            _ => None,
        };
        let args = command.argv[1..].to_vec();
        self.actions
            .borrow_mut()
            .execute(&name, &args, seat.as_ref(), &command.source);
    }

    fn read_script(&mut self, name: &str, source: &CommandContext) -> ScriptRead {
        if let Some(startup) = self.startup.as_ref() {
            if let Some(text) = startup.borrow_mut().read_script(name, source) {
                return ScriptRead::Ready(Some(text));
            }
        }
        match self.scripts.borrow().read(name, source) {
            Ok(text) => ScriptRead::Ready(text),
            Err(error) => ScriptRead::Failed(error.to_string()),
        }
    }

    fn allow_command(&mut self, command: &ForwardedCommand) -> bool {
        match self.startup.as_ref() {
            Some(startup) => startup.borrow_mut().allow_command(&command.argv, &command.source),
            None => true,
        }
    }
}

impl ApplicationInput {
    /// Shared constructor (donor `create`).
    fn create(
        params: ApplicationInputOpen<'_>,
        staging: Option<HashSet<u64>>,
        for_client: bool,
    ) -> Result<Self, InputError> {
        let ApplicationInputOpen {
            window,
            players,
            options,
            dialect,
            session,
            simulation,
            actions,
            now,
            settings,
            owner,
            previous,
            prepared,
            controllers,
            devices,
            menu_settings,
            midi_boundary,
            joystick_opener,
        } = params;
        if players.is_empty() {
            return Err(InputError::NoPlayers);
        }
        let startup = prepared
            .clone()
            .or_else(|| previous.and_then(|previous| previous.startup.clone()));
        let has_configuration = actions.borrow().configuration().is_some();
        let mut configuration_movement = None;
        let mut configuration_fallback = None;
        let mut configuration_seats = Vec::new();
        let mut configuration_program = None;
        let mut configuration_scripts = None;
        if has_configuration {
            let mut actions = actions.borrow_mut();
            let Some(configuration) = actions.configuration_mut() else {
                return Err(InputError::MissingConfiguration);
            };
            configuration_movement = configuration.take_movement();
            configuration_fallback = configuration.take_fallback();
            configuration_seats = configuration.take_seats();
            configuration_program = configuration.take_program();
            configuration_scripts = configuration.scripts();
        }
        let context = CommandContext::new(session.clone(), CommandOrigin::LocalConsole);
        if let Some(owner) = owner.as_ref() {
            let movement = owner.movement.as_ref().unwrap_or(&owner.cvars).borrow().dialect();
            let fallback = owner.fallback.as_ref().unwrap_or(&owner.cvars).borrow().dialect();
            let cvars = owner.cvars.borrow().dialect();
            let has_console = actions.borrow().console().is_some();
            if movement != dialect || fallback != cvars || has_console {
                return Err(InputError::OwnerDialect);
            }
        }
        let source_dialect = actions
            .borrow()
            .console()
            .map(|console| console.dialect())
            .or_else(|| owner.as_ref().map(|owner| owner.cvars.borrow().dialect()))
            .unwrap_or(dialect);
        let seat_ids: Vec<SeatId> = players.iter().map(|player| player.seat_id().clone()).collect();
        let loaded = read_settings(
            &seat_ids,
            dialect,
            source_dialect,
            &session,
            &settings,
            owner.as_ref(),
            previous,
            startup.as_ref(),
        )?;
        // Movement registry.
        let cvars = match configuration_movement {
            Some(registry) => shared_registry(registry),
            None => match owner.as_ref().and_then(|owner| owner.movement.clone()) {
                Some(cell) => cell,
                None => match owner.as_ref().map(|owner| Rc::clone(&owner.cvars)) {
                    Some(cell) => cell,
                    None => {
                        let mut fresh = seed_registry(dialect, &session);
                        Self::restore_movement(&mut fresh, dialect, previous, startup.as_ref(), &loaded.movement)?;
                        shared_registry(fresh)
                    }
                },
            },
        };
        // Console (fallback) registry.
        let console_cvars = match configuration_fallback {
            Some(registry) => shared_registry(registry),
            None => match owner.as_ref().and_then(|owner| owner.fallback.clone()) {
                Some(cell) => cell,
                None => match owner.as_ref().map(|owner| Rc::clone(&owner.cvars)) {
                    Some(cell) => cell,
                    None => {
                        if source_dialect == dialect {
                            let shared = Rc::clone(&cvars);
                            shared.borrow_mut().apply_archive(&loaded.fallback)?;
                            shared
                        } else {
                            let mut fresh = seed_registry(source_dialect, &session);
                            Self::restore_fallback(
                                &mut fresh,
                                source_dialect,
                                previous,
                                startup.as_ref(),
                                &loaded.fallback,
                            )?;
                            shared_registry(fresh)
                        }
                    }
                },
            },
        };
        if console_cvars.borrow().dialect() != source_dialect {
            return Err(InputError::AdoptDialect);
        }
        // Console scripts.
        let scripts = configuration_scripts
            .or_else(|| actions.borrow().scripts())
            .or_else(|| startup.as_ref().map(|startup| startup.borrow().scripts_cell()))
            .or_else(|| owner.as_ref().and_then(|owner| owner.scripts.clone()))
            .unwrap_or_else(|| {
                let root = options.user_content_root.as_deref().map(Path::new);
                let mounts = ScriptMounts {
                    actions: Rc::clone(&actions),
                    has_override: actions.borrow().has_mounted_script(),
                };
                Rc::new(RefCell::new(ConsoleScriptFiles::new(
                    ConsoleScriptInputs {
                        console_root: console_config_root(root),
                        settings: settings.clone(),
                        mounts,
                        legacy_config: None,
                    },
                    None,
                )))
            });
        // Shared settings registry.
        let shared = actions.borrow().shared_cvars();
        // Command buffer.
        let (owner_commands, external_routing, owner_input_settings) = match owner {
            Some(owner) => (Some(owner.commands), Some(owner.routing), owner.input_settings),
            None => (None, None, None),
        };
        let commands = match owner_commands {
            Some(commands) => Rc::new(RefCell::new(commands)),
            None => {
                let buffer = CommandBuffer::new(source_dialect, context.clone(), BufferOptions::new())?;
                let commands = Rc::new(RefCell::new(buffer));
                if let Some(startup) = startup.as_ref() {
                    startup.borrow().copy_commands_pending(&mut commands.borrow_mut())?;
                }
                commands
            }
        };
        if let Some(previous) = previous {
            if !Rc::ptr_eq(&commands, &previous.commands) {
                commands.borrow_mut().copy_pending_from(&previous.commands.borrow())?;
            }
        }
        let print_router = PrintRouter {
            actions: Rc::clone(&actions),
            consoles: Rc::new(RefCell::new(Vec::new())),
            commands: Rc::clone(&commands),
            now: Rc::clone(&now),
        };
        {
            let printer = print_router.clone();
            commands
                .borrow_mut()
                .set_printer(move |text, source| printer.print_direct(text, source));
            let startup_cell = startup.clone();
            commands
                .borrow_mut()
                .bind_script_completion(move |event: &ScriptCompletion| {
                    if let Some(startup) = startup_cell.as_ref() {
                        startup.borrow_mut().on_script_complete(event);
                    }
                });
        }
        Self::create_seats(CreateSeats {
            window,
            players,
            options,
            dialect,
            session,
            simulation,
            actions,
            now,
            settings,
            previous,
            startup,
            staging,
            controllers,
            devices,
            menu_settings,
            midi_boundary,
            joystick_opener,
            source_dialect,
            loaded,
            cvars,
            console_cvars,
            shared,
            scripts,
            commands,
            print_router,
            configuration_seats,
            configuration_program,
            external_routing,
            owner_input_settings,
            for_client,
        })
    }

    /// Restore a fresh movement registry from previous, startup, or archive.
    fn restore_movement(
        fresh: &mut CvarRegistry,
        dialect: Dialect,
        previous: Option<&ApplicationInput>,
        startup: Option<&StartupCell>,
        archive: &[CvarArchiveEntry],
    ) -> Result<(), InputError> {
        if let Some(previous) = previous {
            if previous.cvars.borrow().dialect() == dialect {
                let state = previous.cvars.borrow().capture_world_transfer_state();
                return apply_state(fresh, &state);
            }
        }
        if let Some(startup) = startup {
            let images = startup.borrow().registries();
            if images.movement.dialect == dialect {
                return apply_state(fresh, &images.movement.state);
            }
        }
        fresh.apply_archive(archive)?;
        Ok(())
    }

    /// Restore a fresh fallback registry from previous, startup, or archive.
    fn restore_fallback(
        fresh: &mut CvarRegistry,
        source_dialect: Dialect,
        previous: Option<&ApplicationInput>,
        startup: Option<&StartupCell>,
        archive: &[CvarArchiveEntry],
    ) -> Result<(), InputError> {
        if let Some(previous) = previous {
            if previous.console_cvars.borrow().dialect() == source_dialect {
                let state = previous.console_cvars.borrow().capture_world_transfer_state();
                return apply_state(fresh, &state);
            }
        }
        if let Some(startup) = startup {
            let images = startup.borrow().registries();
            if images.fallback.dialect == source_dialect {
                return apply_state(fresh, &images.fallback.state);
            }
        }
        fresh.apply_archive(archive)?;
        Ok(())
    }
}

/// Seat construction state carried from `create`.
#[allow(clippy::too_many_arguments)]
struct CreateSeats<'a> {
    window: WindowCell,
    players: Vec<LocalPlayer>,
    options: ApplicationOptions,
    dialect: Dialect,
    session: SessionId,
    simulation: Rc<RefCell<dyn InputPlayerView>>,
    actions: ActionsCell,
    now: InputClock,
    settings: ConfigStore,
    previous: Option<&'a ApplicationInput>,
    startup: Option<StartupCell>,
    staging: Option<HashSet<u64>>,
    controllers: Option<ControllersCell>,
    devices: Option<DeviceCell>,
    menu_settings: Option<&'a ControllerSettings<RouterHandle, SharedDevicesFn>>,
    midi_boundary: Box<dyn MidiInputBoundary>,
    joystick_opener: JoystickOpener,
    source_dialect: Dialect,
    loaded: LoadedSettings,
    cvars: SharedRegistry,
    console_cvars: SharedRegistry,
    shared: Option<SharedRegistry>,
    scripts: ScriptsCell,
    commands: SharedCommands,
    print_router: PrintRouter,
    configuration_seats: Vec<ProfileSeat>,
    configuration_program: Option<StagedClientCommands>,
    external_routing: Option<Box<dyn InputCvarRouting>>,
    owner_input_settings: Option<SharedRegistry>,
    for_client: bool,
}

impl ApplicationInput {
    /// Build seats, router, settings, and candidates (donor `create`, seats half).
    fn create_seats(bundle: CreateSeats<'_>) -> Result<Self, InputError> {
        let CreateSeats {
            window,
            players,
            options,
            dialect,
            session,
            simulation,
            actions,
            now,
            settings,
            previous,
            startup,
            staging,
            controllers,
            devices,
            menu_settings,
            midi_boundary,
            joystick_opener,
            source_dialect,
            loaded,
            cvars,
            console_cvars,
            shared,
            scripts,
            commands,
            print_router,
            mut configuration_seats,
            configuration_program,
            external_routing,
            owner_input_settings,
            for_client,
        } = bundle;
        let fresh = previous.is_none() && staging.is_none();
        let player_count = players.len();
        let haptic_loader: LoaderCell = Rc::new(RefCell::new(Rc::new(|_| None)));
        let has_configuration = actions.borrow().configuration().is_some();
        let startup_views = startup.as_ref().map(|startup| startup.borrow().seats());
        let mut locals = Vec::new();
        let mut seats = Vec::new();
        let mut mice = HashMap::new();
        let mut shadows: HashMap<SeatId, Rc<RefCell<qa_client::input::BindingTable>>> = HashMap::new();
        let mut selections = Vec::new();
        let mut fresh_inputs: HashSet<SeatId> = HashSet::new();
        let mut candidate_defaults = HashMap::new();
        let mut candidate_gamepads = HashMap::new();
        for (index, player) in players.into_iter().enumerate() {
            let seat = player.seat_id().clone();
            let client = player.seat.client_id().clone();
            let seat_context = seat_context(&session, &seat, &client);
            let startup_view = startup_views.as_ref().and_then(|views| {
                views.iter().find(|view| {
                    view.id == seat
                        && match root_origin(&view.context.origin) {
                            CommandOrigin::LocalSeat {
                                client: seat_client, ..
                            } => *seat_client == client,
                            _ => false,
                        }
                })
            });
            let configured = configuration_seats
                .iter()
                .position(|candidate| {
                    candidate.id == seat
                        && match root_origin(&candidate.context.origin) {
                            CommandOrigin::LocalSeat {
                                client: seat_client, ..
                            } => *seat_client == client,
                            _ => false,
                        }
                })
                .map(|position| configuration_seats.remove(position));
            let has_configured = configured.is_some();
            let mut configured_seat_input = None;
            let mut configured_mouse = None;
            if let Some(configured) = configured {
                configured_mouse = Some(configured.mouse);
                configured_seat_input = configured.seat;
            }
            // Mouse registry: configured, prepared (fresh with a matching
            // dialect), owner-shared, or restored fresh.
            let mouse = match configured_mouse {
                Some(registry) => shared_registry(registry),
                None => {
                    let prepared = fresh && startup_view.is_some_and(|view| view.mouse.dialect == source_dialect);
                    if prepared {
                        let view = startup_view.expect("prepared mouse lost its startup seat");
                        let mut seeded = seed_registry(source_dialect, &session);
                        apply_state(&mut seeded, &view.mouse.state)?;
                        shared_registry(seeded)
                    } else if let Some(cell) = owner_input_settings.clone() {
                        cell
                    } else {
                        let mut fresh_registry = seed_registry(source_dialect, &session);
                        register_mouse_settings(&mut fresh_registry)?;
                        Self::restore_mouse(
                            &mut fresh_registry,
                            &seat,
                            source_dialect,
                            previous,
                            startup_view,
                            loaded.input.get(index).map(Vec::as_slice).unwrap_or(&[]),
                        )?;
                        shared_registry(fresh_registry)
                    }
                }
            };
            {
                let mouse_ref = mouse.borrow();
                if mouse_ref.dialect() != source_dialect {
                    return Err(InputError::ForeignMouse);
                }
                if mouse_ref.session().is_some_and(|declared| declared != &session) {
                    return Err(InputError::ForeignMouse);
                }
            }
            mice.insert(seat.clone(), Rc::clone(&mouse));
            let shadow = Rc::new(RefCell::new(qa_client::input::BindingTable::new()));
            shadows.insert(seat.clone(), Rc::clone(&shadow));
            // Live seat: configured seats move in; staged seats without a
            // configured seat copy the previous router seat and transfer the
            // live seat at publication; fresh seats start empty.
            let previous_has_seat = previous.is_some_and(|previous| previous.router.borrow().seat(&seat).is_some());
            let retained = configured_seat_input.is_some() || previous_has_seat || startup_view.is_some();
            let mut seat_input = match configured_seat_input {
                Some(seat_input) => seat_input,
                None => {
                    let mut seat_input = Seat::new(seat.clone(), dialect, Box::new(|_, _| false));
                    if !fresh && previous_has_seat {
                        if let Some(previous) = previous {
                            if let Some(live) = previous.router.borrow().seat(&seat) {
                                for binding in live.bindings() {
                                    seat_input.bind((*binding).clone());
                                }
                            }
                        }
                    }
                    seat_input
                }
            };
            if !retained || (has_configured && startup_view.is_none()) {
                fresh_inputs.insert(seat.clone());
            }
            // Console.
            let console = Rc::new(RefCell::new(SeatConsole::new(source_dialect, true)));
            // Builder.
            let mut builder = InputCommandBuilder::new(dialect_family(dialect));
            let view = simulation.borrow().player_view(&player.actor);
            builder.set_view_angles(view.angles)?;
            // Bindings.
            let defaults = default_bindings(0, dialect, &actions.borrow().binding_items(&seat));
            if has_configuration {
                if let Some(configuration) = actions.borrow_mut().configuration_mut() {
                    configuration.apply_binding_defaults(&seat, &defaults);
                }
            }
            if !has_configuration && (previous.is_none() || !retained) {
                if !retained {
                    for binding in defaults {
                        seat_input.bind(binding.clone());
                        shadow.borrow_mut().bind(binding);
                    }
                } else if staging.is_some() && startup_view.is_some() {
                    candidate_defaults.insert(seat.clone(), defaults);
                } else {
                    let resolved = match startup.as_ref() {
                        Some(startup) => startup.borrow_mut().bindings(&seat, defaults),
                        None => defaults,
                    };
                    for binding in resolved {
                        seat_input.bind(binding.clone());
                        shadow.borrow_mut().bind(binding);
                    }
                }
            }
            // Temporary haptics; the swap below replaces it with live router,
            // loader, clock, and controller wiring once those cells exist.
            let haptics = Rc::new(RefCell::new(SeatHaptics::new(
                seat.clone(),
                Box::new(|_| None),
                Box::new(|_, _| None),
                Box::new({
                    let now = Rc::clone(&now);
                    move || now()
                }),
                Box::new(|_, _, _, _| ControllerOperationResult::Accepted),
            )));
            let selection = if previous_has_seat {
                previous
                    .map(|previous| {
                        previous
                            .router
                            .borrow()
                            .controller_selection(&seat)
                            .unwrap_or(ControllerSelection::Automatic)
                    })
                    .unwrap_or(ControllerSelection::Automatic)
            } else {
                loaded
                    .saved
                    .get(index)
                    .and_then(|saved| saved.as_ref())
                    .map(|saved| live_controller_selection(&saved.controller))
                    .unwrap_or(if player_count > 1 && index == 0 {
                        ControllerSelection::None
                    } else {
                        ControllerSelection::Automatic
                    })
            };
            selections.push(selection);
            seats.push(seat_input);
            locals.push(LocalInput {
                player,
                console,
                builder,
                haptics,
                context: seat_context,
            });
        }
        // Saved profiles apply after every local exists.
        for (index, local) in locals.iter_mut().enumerate() {
            let saved = loaded.saved.get(index).and_then(|saved| saved.clone());
            let Some(saved) = saved else {
                continue;
            };
            let seat = local.seat_id().clone();
            let fresh_input = fresh_inputs.contains(&seat);
            if !has_configuration && ((startup.is_none() && previous.is_none()) || fresh_input) {
                seats[index].unbind_all();
                if let Some(shadow) = shadows.get(&seat) {
                    shadow.borrow_mut().unbind_all();
                }
                for binding in &saved.bindings {
                    seats[index].bind(binding.clone());
                    if let Some(shadow) = shadows.get(&seat) {
                        shadow.borrow_mut().bind(binding.clone());
                    }
                }
            }
            if previous.is_none() || fresh_input {
                if staging.is_some() && startup.is_some() && !fresh_input {
                    if !for_client {
                        candidate_gamepads.insert(seat.clone(), client_gamepad_tuning(&saved.gamepad));
                    }
                } else {
                    seats[index].gamepad.tuning = client_gamepad_tuning(&saved.gamepad);
                }
            }
            if startup.is_none() {
                if let Some(always_run) = saved.always_run {
                    let mut tuning = local.builder.tuning();
                    tuning.always_run = always_run;
                    local.builder.set_tuning(tuning);
                }
            }
            if !has_configuration && owner_input_settings.is_none() && (startup.is_none() || fresh_input) {
                local.builder.mouse.tuning = client_mouse_tuning(&saved.mouse);
            }
            local.console.borrow_mut().history.replace(&saved.history);
            local.haptics.borrow_mut().set_enabled(saved.rumble);
            local.haptics.borrow_mut().set_strength(saved.rumble_strength)?;
        }
        // A pending previous startup copies current locals.
        if previous.is_some_and(|previous| {
            previous
                .startup
                .as_ref()
                .is_some_and(|startup| startup.borrow().pending())
        }) {
            if let Some(previous) = previous {
                for (index, local) in locals.iter_mut().enumerate() {
                    let prior = previous.locals.iter().find(|prior| prior.seat_id() == local.seat_id());
                    let Some(prior) = prior else {
                        continue;
                    };
                    let prior_seat = prior.seat_id().clone();
                    let bindings: Vec<InputBinding> = previous
                        .router
                        .borrow()
                        .seat(&prior_seat)
                        .map(|seat| seat.bindings().into_iter().cloned().collect())
                        .unwrap_or_default();
                    seats[index].unbind_all();
                    if let Some(shadow) = shadows.get(local.seat_id()) {
                        shadow.borrow_mut().unbind_all();
                    }
                    for binding in bindings {
                        seats[index].bind(binding.clone());
                        if let Some(shadow) = shadows.get(local.seat_id()) {
                            shadow.borrow_mut().bind(binding);
                        }
                    }
                    local.builder.mouse.tuning = prior.builder.mouse.tuning;
                    let mut tuning = local.builder.tuning();
                    tuning.always_run = prior.builder.tuning().always_run;
                    local.builder.set_tuning(tuning);
                }
            }
        }
        Self::create_router(CreateRouter {
            window,
            options,
            dialect,
            session,
            simulation,
            actions,
            now,
            settings,
            previous,
            startup,
            staging,
            controllers,
            devices,
            menu_settings,
            midi_boundary,
            joystick_opener,
            source_dialect,
            loaded,
            cvars,
            console_cvars,
            shared,
            scripts,
            commands,
            print_router,
            configuration_program,
            external_routing,
            for_client,
            locals,
            seats,
            mice,
            shadows,
            selections,
            candidate_defaults,
            candidate_gamepads,
            haptic_loader,
        })
    }
}

/// Router construction state carried from `create_seats`.
#[allow(clippy::too_many_arguments)]
struct CreateRouter<'a> {
    window: WindowCell,
    options: ApplicationOptions,
    dialect: Dialect,
    session: SessionId,
    simulation: Rc<RefCell<dyn InputPlayerView>>,
    actions: ActionsCell,
    now: InputClock,
    settings: ConfigStore,
    previous: Option<&'a ApplicationInput>,
    startup: Option<StartupCell>,
    staging: Option<HashSet<u64>>,
    controllers: Option<ControllersCell>,
    devices: Option<DeviceCell>,
    menu_settings: Option<&'a ControllerSettings<RouterHandle, SharedDevicesFn>>,
    midi_boundary: Box<dyn MidiInputBoundary>,
    joystick_opener: JoystickOpener,
    source_dialect: Dialect,
    loaded: LoadedSettings,
    cvars: SharedRegistry,
    console_cvars: SharedRegistry,
    shared: Option<SharedRegistry>,
    scripts: ScriptsCell,
    commands: SharedCommands,
    print_router: PrintRouter,
    configuration_program: Option<StagedClientCommands>,
    external_routing: Option<Box<dyn InputCvarRouting>>,
    for_client: bool,
    locals: Vec<LocalInput>,
    seats: Vec<Seat>,
    mice: HashMap<SeatId, SharedRegistry>,
    shadows: HashMap<SeatId, Rc<RefCell<qa_client::input::BindingTable>>>,
    selections: Vec<ControllerSelection>,
    candidate_defaults: HashMap<SeatId, Vec<InputBinding>>,
    candidate_gamepads: HashMap<SeatId, GamepadTuning>,
    haptic_loader: LoaderCell,
}

impl ApplicationInput {
    /// Restore a fresh mouse registry from previous, startup, or archive.
    fn restore_mouse(
        fresh: &mut CvarRegistry,
        seat: &SeatId,
        source_dialect: Dialect,
        previous: Option<&ApplicationInput>,
        startup_view: Option<&StartupSeatView>,
        archive: &[CvarArchiveEntry],
    ) -> Result<(), InputError> {
        if let Some(previous) = previous {
            if let Some(mouse) = previous.mouse.get(seat) {
                if mouse.borrow().dialect() == source_dialect {
                    let state = mouse.borrow().capture_world_transfer_state();
                    return apply_state(fresh, &state);
                }
            }
        }
        if startup_view.is_some_and(|view| view.mouse.dialect == source_dialect) {
            let view = startup_view.expect("startup mouse lost its seat");
            return apply_state(fresh, &view.mouse.state);
        }
        fresh.apply_archive(archive)?;
        Ok(())
    }

    /// Build the router, settings, and candidates (donor `create`, router half).
    #[allow(clippy::too_many_lines)]
    fn create_router(bundle: CreateRouter<'_>) -> Result<Self, InputError> {
        let CreateRouter {
            window,
            options,
            dialect,
            session,
            simulation,
            actions,
            now,
            settings,
            previous,
            startup,
            staging,
            controllers,
            devices,
            menu_settings,
            midi_boundary,
            joystick_opener,
            source_dialect,
            loaded,
            cvars,
            console_cvars,
            shared,
            scripts,
            commands,
            print_router,
            configuration_program,
            external_routing,
            for_client,
            locals,
            seats,
            mice,
            shadows,
            selections,
            candidate_defaults,
            candidate_gamepads,
            haptic_loader,
        } = bundle;
        let fresh = previous.is_none() && staging.is_none();
        // Controllers.
        let owns_controllers = !for_client && previous.is_none();
        let controllers = match controllers.or_else(|| previous.map(|previous| Rc::clone(&previous.controllers))) {
            Some(controllers) => controllers,
            None => {
                let opened: SdlControllers =
                    SdlControllers::open().map_err(|error| InputError::Platform(error.to_string()))?;
                Rc::new(RefCell::new(opened))
            }
        };
        // Devices.
        let owns_devices = !for_client && previous.is_none();
        let devices = match devices.or_else(|| previous.map(|previous| Rc::clone(&previous.devices))) {
            Some(devices) => devices,
            None => {
                let source = shared.as_ref().unwrap_or(&cvars);
                let mut device_cvars = seed_registry(source.borrow().dialect(), &session);
                let state = source.borrow().capture_world_transfer_state();
                apply_state(&mut device_cvars, &state)?;
                let print_actions = Rc::clone(&actions);
                let print = Rc::new(RefCell::new(move |text: &str| {
                    print_actions.borrow_mut().print(text);
                }));
                Rc::new(RefCell::new(InputDevices::new(
                    device_cvars,
                    input_device_store(options.user_content_root.as_deref()),
                    print,
                    midi_boundary,
                    joystick_opener,
                )?))
            }
        };
        // Router.
        let contexts: Rc<RefCell<HashMap<SeatId, CommandContext>>> = Rc::new(RefCell::new(
            locals
                .iter()
                .map(|local| (local.seat_id().clone(), local.context.clone()))
                .collect(),
        ));
        let routes: Vec<SeatRoute> = seats
            .into_iter()
            .zip(selections)
            .map(|(seat, controller)| SeatRoute { seat, controller })
            .collect();
        let keyboard = match previous {
            Some(previous) => match previous.router.borrow().keyboard_seat() {
                None => None,
                Some(id) => locals
                    .iter()
                    .find(|local| local.seat_id() == &id)
                    .map(|local| local.seat_id().clone())
                    .or_else(|| locals.first().map(|local| local.seat_id().clone())),
            },
            None => match loaded.routing {
                None => locals.first().map(|local| local.seat_id().clone()),
                Some(routing) => match routing.keyboard_seat {
                    None => None,
                    Some(index) => locals
                        .iter()
                        .find(|local| local.seat_id().index() == index)
                        .map(|local| local.seat_id().clone())
                        .or_else(|| locals.first().map(|local| local.seat_id().clone())),
                },
            },
        };
        let registry = BufferRegistry {
            commands: Rc::clone(&commands),
            cvars: Rc::clone(&console_cvars),
            contexts: Rc::clone(&contexts),
            print: print_router.clone(),
        };
        let router = Rc::new(RefCell::new(InputRouter::new(
            routes,
            keyboard,
            Some(Box::new(ControllersAdapter {
                controllers: Rc::clone(&controllers),
            })),
            Box::new(registry),
            Box::new({
                let now = Rc::clone(&now);
                move || now()
            }),
            Box::new({
                let window = Rc::clone(&window);
                move || window.borrow().ticks().unwrap_or(0)
            }),
            true,
            Box::new({
                let actions = Rc::clone(&actions);
                move |event: UnhandledEvent| {
                    if matches!(event, UnhandledEvent::Platform(SdlEvent::Quit { .. }))
                        || matches!(
                            event,
                            UnhandledEvent::Platform(SdlEvent::Window {
                                event: WINDOW_CLOSE,
                                ..
                            })
                        )
                    {
                        actions.borrow_mut().quit();
                    }
                }
            }),
            previous.is_some() || staging.is_some(),
            None,
        )?));
        let handle = RouterHandle {
            router: Rc::clone(&router),
            commands: Rc::clone(&commands),
            cvars: Rc::clone(&console_cvars),
            contexts: Rc::clone(&contexts),
            print: print_router.clone(),
        };
        // Swap real haptics into the locals.
        for local in &locals {
            let seat = local.seat_id().clone();
            let enabled = local.haptics.borrow().enabled();
            let strength = local.haptics.borrow().strength();
            let real = SeatHaptics::new(
                seat.clone(),
                Box::new({
                    let router = Rc::clone(&router);
                    move |seat: &SeatId| router.borrow().controller_for(seat)
                }),
                Box::new({
                    let loader = Rc::clone(&haptic_loader);
                    move |content: &str, sound: &str| {
                        let content = ContentId::new(content);
                        let request = ResourceRequest {
                            content,
                            path: sound.to_string(),
                        };
                        Rc::clone(&loader.borrow())(&request)
                    }
                }),
                Box::new({
                    let now = Rc::clone(&now);
                    move || now()
                }),
                Box::new({
                    let controllers = Rc::clone(&controllers);
                    move |instance, low, high, duration| {
                        controllers
                            .borrow()
                            .rumble(instance, low, high, duration)
                            .unwrap_or_else(|error| ControllerOperationResult::Disconnected {
                                reason: error.to_string(),
                            })
                    }
                }),
            );
            *local.haptics.borrow_mut() = real;
            local.haptics.borrow_mut().set_enabled(enabled);
            local.haptics.borrow_mut().set_strength(strength)?;
        }
        // Controller settings.
        let devices_fn: SharedDevicesFn = Box::new({
            let controllers = Rc::clone(&controllers);
            move || {
                controllers
                    .borrow()
                    .devices()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|device| ControllerDeviceInfo {
                        instance: device.instance,
                        name: device.name,
                        guid: device.guid,
                        serial: device.serial,
                    })
                    .collect()
            }
        });
        let seats: Vec<SeatId> = locals.iter().map(|local| local.seat_id().clone()).collect();
        let mut settings_ctrl = ControllerSettings::new(handle.clone(), seats, devices_fn, settings.clone(), {
            let actions = Rc::clone(&actions);
            move |text: &str| actions.borrow_mut().print(text)
        });
        let attach = (|| -> Result<(), InputError> {
            if fresh {
                router.borrow_mut().attach_window(Box::new(WindowAdapter {
                    window: Rc::clone(&window),
                }))?;
            }
            router.borrow_mut().restart()?;
            let previous_settings = previous.map(|previous| &previous.settings_ctrl).or(menu_settings);
            match previous_settings {
                None => settings_ctrl.update(),
                Some(previous_settings) => {
                    settings_ctrl.copy_settled_profiles_from(previous_settings);
                }
            }
            Ok(())
        })();
        if let Err(error) = attach {
            if owns_controllers {
                controllers.borrow_mut().close();
            }
            return Err(error);
        }
        // Client command bindings.
        let client_commands = Rc::new(RefCell::new(ClientCommandBindings::new(
            locals.iter().map(|local| local.seat_id().clone()).collect(),
        )));
        // Candidate program.
        let candidate = if previous.is_some() || staging.is_some() {
            if let Some(program) = configuration_program {
                Some(program)
            } else {
                let live_tags = staging
                    .clone()
                    .unwrap_or_else(|| previous.map(|previous| previous.owned_tags()).unwrap_or_default());
                let mut candidate_tags = HashSet::new();
                candidate_tags.insert(registry_tag(&console_cvars));
                candidate_tags.insert(registry_tag(&cvars));
                if let Some(shared) = shared.as_ref() {
                    candidate_tags.insert(registry_tag(shared));
                }
                for cell in mice.values() {
                    candidate_tags.insert(registry_tag(cell));
                }
                if !candidate_tags.is_disjoint(&live_tags) {
                    return Err(InputError::Startup(
                        "Candidate client commands require isolated cvar owners".to_string(),
                    ));
                }
                let mut buffer = CommandBuffer::new(
                    source_dialect,
                    commands.borrow().context().clone(),
                    BufferOptions::new(),
                )?;
                {
                    let printer = print_router.clone();
                    buffer.set_printer(move |text, source| printer.print_direct(text, source));
                }
                let live: Vec<(SeatId, Vec<InputBinding>)> = locals
                    .iter()
                    .map(|local| {
                        let bindings = router
                            .borrow()
                            .seat(local.seat_id())
                            .map(|seat| seat.bindings().into_iter().cloned().collect())
                            .unwrap_or_default();
                        (local.seat_id().clone(), bindings)
                    })
                    .collect();
                let console_live = startup.as_ref().and_then(|startup| startup.borrow().console_bindings());
                let printer = print_router.clone();
                let binding_print: Rc<dyn Fn(&str)> = Rc::new(move |text| printer.print_direct(text, None));
                let staged = StagedClientCommands::stage(StagedProgram {
                    buffer,
                    live_revision: commands.borrow().program_revision(),
                    live,
                    console_live,
                    release_dialect: source_dialect,
                    binding_print,
                })?;
                if actions.borrow().configuration().is_some() {
                    let mut requests = Vec::new();
                    actions
                        .borrow_mut()
                        .configuration_mut()
                        .expect("configuration lost during candidate staging")
                        .forward_commands(&mut |request| requests.push(request));
                    for request in requests {
                        actions.borrow_mut().execute(
                            &request.name,
                            &request.arguments,
                            request.seat.as_ref(),
                            &request.source,
                        );
                    }
                }
                for (seat, defaults) in &candidate_defaults {
                    let Some(table) = staged.staged_table(seat) else {
                        return Err(InputError::CandidateSeat);
                    };
                    let preview = match startup.as_ref() {
                        Some(startup) => startup.borrow().preview_bindings(seat, defaults),
                        None => defaults.clone(),
                    };
                    for binding in preview {
                        table.borrow_mut().bind(binding);
                    }
                }
                Some(staged)
            }
        } else {
            None
        };
        let mut this = Self {
            options,
            dialect,
            session,
            commands,
            console_commands: Rc::new(RefCell::new(ConsoleCommands::new())),
            cvars,
            console_cvars,
            shared,
            published: Rc::new(RefCell::new(HashMap::new())),
            mouse: mice,
            seat_cvars: HashMap::new(),
            server_cvars: None,
            scripts,
            locals,
            router,
            handle,
            shadows,
            controllers,
            owns_controllers,
            devices,
            owns_devices,
            settings_ctrl,
            window,
            pending_window: Vec::new(),
            pending_controller: Vec::new(),
            client_commands,
            commands_active: false,
            staged: Vec::new(),
            unregister: Vec::new(),
            ui_ids: HashMap::new(),
            seat_ui: Rc::new(RefCell::new(HashMap::new())),
            q3_selections: HashMap::new(),
            arsenals: HashMap::new(),
            offhand: Rc::new(RefCell::new(HashMap::new())),
            candidate,
            candidate_defaults,
            candidate_gamepads,
            startup,
            configuration_published: false,
            archive_persistence: false,
            sequence: 0,
            actions,
            print_router,
            now,
            settings,
            simulation,
            external_routing,
            haptic_loader,
            pending_focus: Rc::new(RefCell::new(HashMap::new())),
        };
        *this.print_router.consoles.borrow_mut() = this
            .locals
            .iter()
            .map(|local| (local.seat_id().clone(), Rc::clone(&local.console)))
            .collect();
        this.wire_console_cells()?;
        if fresh {
            let activated = (|| -> Result<(), InputError> {
                this.activate_commands(false)?;
                this.adopt_startup()?;
                Ok(())
            })();
            if let Err(error) = activated {
                this.close();
                return Err(error);
            }
        }
        Ok(this)
    }
}

impl ApplicationInput {
    /// Command buffer cell.
    #[must_use]
    pub fn commands(&self) -> SharedCommands {
        Rc::clone(&self.commands)
    }

    /// Movement cvars.
    #[must_use]
    pub fn cvars(&self) -> SharedRegistry {
        Rc::clone(&self.cvars)
    }

    /// Console cvars.
    #[must_use]
    pub fn console_cvars(&self) -> SharedRegistry {
        Rc::clone(&self.console_cvars)
    }

    /// Shared settings registry.
    #[must_use]
    pub fn shared_cvars(&self) -> Option<SharedRegistry> {
        self.shared.clone()
    }

    /// Local seats.
    #[must_use]
    pub fn locals(&self) -> &[LocalInput] {
        &self.locals
    }

    /// Input router cell.
    #[must_use]
    pub fn router(&self) -> RouterCell {
        Rc::clone(&self.router)
    }

    /// Controllers cell.
    #[must_use]
    pub fn controllers(&self) -> ControllersCell {
        Rc::clone(&self.controllers)
    }

    /// Input devices cell.
    #[must_use]
    pub fn devices(&self) -> DeviceCell {
        Rc::clone(&self.devices)
    }

    /// Controller settings.
    #[must_use]
    pub fn controller_settings(&self) -> &ControllerSettings<RouterHandle, SharedDevicesFn> {
        &self.settings_ctrl
    }

    /// Console scripts.
    #[must_use]
    pub fn scripts(&self) -> ScriptsCell {
        Rc::clone(&self.scripts)
    }

    /// Application window cell.
    #[must_use]
    pub fn window(&self) -> WindowCell {
        Rc::clone(&self.window)
    }

    /// Millisecond clock.
    #[must_use]
    pub fn now(&self) -> InputClock {
        Rc::clone(&self.now)
    }

    /// Local player capacity.
    #[must_use]
    pub fn local_player_capacity(&self) -> usize {
        self.actions.borrow().local_player_capacity()
    }

    /// Next input command sequence.
    #[must_use]
    pub fn next_command_sequence(&self) -> u64 {
        self.sequence
    }

    /// Mouse registry for a seat.
    #[must_use]
    pub fn input_cvars(&self, seat: &SeatId) -> Option<SharedRegistry> {
        self.mouse.get(seat).cloned()
    }

    /// Registry identity tags for every owned and reachable cell.
    fn owned_tags(&self) -> HashSet<u64> {
        let mut tags = HashSet::new();
        tags.insert(registry_tag(&self.console_cvars));
        tags.insert(registry_tag(&self.cvars));
        if let Some(shared) = self.shared.as_ref() {
            tags.insert(registry_tag(shared));
        }
        for cell in self.mouse.values() {
            tags.insert(registry_tag(cell));
        }
        if let Some(console) = self.actions.borrow().console() {
            if let Some((server, _)) = console.server() {
                tags.insert(registry_tag(&server));
            }
            for local in &self.locals {
                if let Some(cell) = console.seat(local.seat_id()) {
                    tags.insert(registry_tag(&cell));
                }
            }
        }
        if let Some(external) = self.external_routing.as_ref() {
            let mut index = 0;
            while let Some(cell) = external.registry(index) {
                tags.insert(registry_tag(&cell));
                index += 1;
            }
        }
        tags
    }

    /// Binding capabilities with caller overrides.
    fn binding_capabilities(&self) -> BindingCapabilities {
        let mut capabilities = default_binding_capabilities(&self.options.network);
        if let Some(actions) = self.actions.borrow().binding_capabilities() {
            capabilities.chat = actions.chat;
            if actions.score_command.is_some() {
                capabilities.score_command = actions.score_command;
            }
            capabilities.offhand_grapple = actions.offhand_grapple;
            capabilities.offhand_grenades = actions.offhand_grenades;
        }
        capabilities
    }

    /// Register a world command unless taken.
    fn register_command(&mut self, name: &str, handler: CommandHandler) {
        if self.commands.borrow().exists(name) {
            return;
        }
        let cvars = self.console_cvars.borrow();
        let registered = self
            .commands
            .borrow_mut()
            .register(name, Some(handler), None, &cvars)
            .unwrap_or(false);
        if registered {
            self.unregister.push(Unregister::Command(name.to_string()));
        }
    }

    /// Build the UI callback for one seat.
    fn ui_callback(&self, seat: &SeatId) -> UiCallback {
        let seat = seat.clone();
        let console = self
            .locals
            .iter()
            .find(|local| local.seat_id() == &seat)
            .map(|local| Rc::clone(&local.console));
        let Some(console) = console else {
            return Box::new(|_, _| false);
        };
        let console_commands = Rc::clone(&self.console_commands);
        let console_cvars = Rc::clone(&self.console_cvars);
        let movement = Rc::clone(&self.cvars);
        let mouse = self.mouse.get(&seat).cloned().unwrap_or_else(|| {
            let mut fresh = seed_registry(self.console_cvars.borrow().dialect(), &self.session);
            let _ = register_mouse_settings(&mut fresh);
            Rc::new(RefCell::new(fresh))
        });
        let shadow = self
            .shadows
            .get(&seat)
            .cloned()
            .unwrap_or_else(|| Rc::new(RefCell::new(qa_client::input::BindingTable::new())));
        let context = self
            .locals
            .iter()
            .find(|local| local.seat_id() == &seat)
            .map(|local| local.context.clone())
            .unwrap_or_else(|| self.commands.borrow().context().clone());
        let seat_services = ConsoleSeatServices {
            seat: seat.clone(),
            context: context.clone(),
            haptics: self
                .locals
                .iter()
                .find(|local| local.seat_id() == &seat)
                .map(|local| Rc::clone(&local.haptics))
                .unwrap_or_else(|| {
                    let router = Rc::clone(&self.router);
                    let loader = Rc::clone(&self.haptic_loader);
                    let now = Rc::clone(&self.now);
                    let controllers = Rc::clone(&self.controllers);
                    Rc::new(RefCell::new(SeatHaptics::new(
                        seat.clone(),
                        Box::new(move |seat: &SeatId| router.borrow().controller_for(seat)),
                        Box::new(move |content: &str, sound: &str| {
                            let content = ContentId::new(content);
                            let request = ResourceRequest {
                                content,
                                path: sound.to_string(),
                            };
                            Rc::clone(&loader.borrow())(&request)
                        }),
                        Box::new(move || now()),
                        Box::new(move |instance, low, high, duration| {
                            controllers
                                .borrow()
                                .rumble(instance, low, high, duration)
                                .unwrap_or_else(|error| ControllerOperationResult::Disconnected {
                                    reason: error.to_string(),
                                })
                        }),
                    )))
                }),
            handle: self.handle.clone(),
            pending_focus: Rc::clone(&self.pending_focus),
            now: Rc::clone(&self.now),
        };
        // NOTE: the callback runs inside a router borrow, so it must never
        // borrow the router cell. Focus changes queue for the caller to drain.
        let pending_focus = Rc::clone(&self.pending_focus);
        let seat_ui = Rc::clone(&self.seat_ui);
        let actions = Rc::clone(&self.actions);
        let settings = self.settings.clone();
        let dialect = self.console_cvars.borrow().dialect();
        let map = self.options.map.clone();
        let mut held: HashSet<i32> = HashSet::new();
        Box::new(move |event: &RouterSeatEvent, focus: &SeatFocus| {
            if let RouterSeatEvent::Focus { focused, .. } = event {
                if !focused {
                    held.clear();
                }
            }
            let repeat = match event {
                RouterSeatEvent::Key { code, down, .. } => {
                    if *down {
                        let repeat = held.contains(code);
                        held.insert(*code);
                        repeat
                    } else {
                        held.remove(code);
                        false
                    }
                }
                _ => false,
            };
            if let RouterSeatEvent::Key { code, down, .. } = event {
                if CONSOLE_KEYS.contains(code) {
                    if *down {
                        if !repeat {
                            if let Some(ui) = seat_ui.borrow().get(&seat) {
                                ui.borrow_mut().close_menus();
                            }
                        }
                        {
                            let mut seat_svc = seat_services.clone();
                            console.borrow_mut().toggle_from_key(repeat, &mut seat_svc);
                        }
                    }
                    return true;
                }
            }
            if let Some(console_event) = router_event_to_console(event, repeat) {
                let consumed = {
                    let mut console_ref = console.borrow_mut();
                    let mut commands = console_commands.borrow_mut();
                    let mut cvars = console_cvars.borrow_mut();
                    let mut services = ConsoleServices {
                        seat: seat.clone(),
                        context: context.clone(),
                        console: Rc::clone(&console),
                        commands: Rc::clone(&console_commands),
                        cvars: Rc::clone(&console_cvars),
                        movement: Rc::clone(&movement),
                        mouse: Rc::clone(&mouse),
                        bindings: Rc::clone(&shadow),
                        seat_services: seat_services.clone(),
                        pending_batches: Vec::new(),
                        settings: settings.clone(),
                        dialect,
                        map: map.clone(),
                    };
                    // A cell-shared clone feeds the seat-services slot; the
                    // borrow checker cannot split the two mutable borrows.
                    let mut seat_services = services.seat_services.clone();
                    match console_ref.input(
                        &console_event,
                        seat_focus_to_console(focus),
                        &mut commands,
                        &mut cvars,
                        &mut services,
                        &mut seat_services,
                    ) {
                        Ok(consumed) => {
                            let batches = std::mem::take(&mut services.pending_batches);
                            (consumed, batches)
                        }
                        Err(error) => {
                            let _ = console_ref.print(&format!("{error}\n"), &mut seat_services);
                            (true, Vec::new())
                        }
                    }
                };
                // Drain LLM batches with sequential executes.
                let mut batches = consumed.1;
                while let Some(batch) = batches.pop() {
                    let mut commands = console_commands.borrow_mut();
                    let mut cvars = console_cvars.borrow_mut();
                    let mut services = ConsoleServices {
                        seat: seat.clone(),
                        context: context.clone(),
                        console: Rc::clone(&console),
                        commands: Rc::clone(&console_commands),
                        cvars: Rc::clone(&console_cvars),
                        movement: Rc::clone(&movement),
                        mouse: Rc::clone(&mouse),
                        bindings: Rc::clone(&shadow),
                        seat_services: seat_services.clone(),
                        pending_batches: Vec::new(),
                        settings: settings.clone(),
                        dialect,
                        map: map.clone(),
                    };
                    services.seat_services.pending_focus = Rc::clone(&pending_focus);
                    let _ = commands.execute(
                        &batch,
                        dialect,
                        qa_core::cmd::TextMode::Console,
                        &mut cvars,
                        &mut services,
                    );
                    batches.extend(std::mem::take(&mut services.pending_batches));
                }
                if consumed.0 {
                    return true;
                }
            }
            let time_ms = router_event_time(event) as i64;
            let Some(ui_event) = router_event_to_ui(&seat, time_ms, repeat, event) else {
                return false;
            };
            if actions.borrow_mut().client_input(&ui_event) {
                return true;
            }
            let focus = seat_focus_to_input(focus);
            seat_ui
                .borrow()
                .get(&seat)
                .is_some_and(|ui| ui.borrow_mut().input(&ui_event, &focus))
        })
    }

    /// Activate commands (donor `activateCommands`).
    fn activate_commands(&mut self, preserve_focus: bool) -> Result<(), InputError> {
        if self.commands_active {
            return Ok(());
        }
        self.commands_active = true;
        let now_ms = (self.now)() as i64;
        self.devices.borrow_mut().activate(self.handle.clone(), now_ms)?;
        {
            let router = Rc::clone(&self.router);
            let devices = Rc::clone(&self.devices);
            let contexts = Rc::clone(&self.handle.contexts);
            let now = Rc::clone(&self.now);
            self.register_command(
                "in_restart",
                Rc::new(move |_| {
                    let time = now();
                    Self::release_cells(&router, &devices, &contexts, time, None);
                    let _ = devices.borrow_mut().restart(time as i64);
                    let _ = router.borrow_mut().restart();
                }),
            );
        }
        {
            let devices = Rc::clone(&self.devices);
            self.register_command(
                "midiinfo",
                Rc::new(move |_| {
                    let _ = devices.borrow_mut().info();
                }),
            );
        }
        if let Some(startup) = self.startup.as_ref() {
            let print = self.print_router.clone();
            let release = startup
                .borrow_mut()
                .bind_output(Rc::new(move |text, source| print.print(text, source)));
            self.unregister.push(Unregister::Output(release));
        }
        for index in 0..self.locals.len() {
            let seat = self.locals[index].seat_id().clone();
            let Some(mouse) = self.mouse.get(&seat).cloned() else {
                return Err(InputError::SettingsRegistry);
            };
            // Seed a missing mouse `cl_run` from the console registries. The
            // donor searches through the buffer; the ported buffer has no
            // cvar search surface, and the console registries carry `cl_run`.
            let previous_run = self
                .console_cvars
                .borrow()
                .get("cl_run")
                .or_else(|| self.cvars.borrow().get("cl_run"));
            let mut mouse_ref = mouse.borrow_mut();
            if mouse_ref.get("cl_run").is_none() {
                if let Some(previous_run) = previous_run {
                    mouse_ref.apply_archive(&[CvarArchiveEntry {
                        name: "cl_run".to_string(),
                        value: previous_run.value.clone(),
                    }])?;
                }
            }
            let always_run = self.locals[index].builder.tuning().always_run;
            let effective = bind_run_cvar(&mut mouse_ref, self.dialect, always_run)?;
            drop(mouse_ref);
            let mut tuning = self.locals[index].builder.tuning();
            tuning.always_run = effective;
            self.locals[index].builder.set_tuning(tuning);
        }
        let source_dialect = self
            .actions
            .borrow()
            .console()
            .map(|console| console.dialect())
            .unwrap_or_else(|| self.console_cvars.borrow().dialect());
        // Wheel commands.
        {
            let seat_ui = Rc::clone(&self.seat_ui);
            let mut registry = self.handle.registry();
            let names = register_wheel_commands(
                &mut registry,
                Rc::new(move |seat: SeatId, mode: WheelMode, down: bool| {
                    if let Some(ui) = seat_ui.borrow().get(&seat) {
                        ui.borrow_mut().wheel(mode, down);
                    }
                }),
            );
            for name in names {
                self.unregister.push(Unregister::Command(name));
            }
        }
        // Button commands with client-game score interception.
        {
            let router = Rc::clone(&self.router);
            let lookup: ButtonSeatLookup = Rc::new(move |seat| {
                Some(Rc::new(RefCell::new(ButtonSeatHandle {
                    router: Rc::clone(&router),
                    seat: seat.clone(),
                })) as Rc<RefCell<dyn ButtonSeat>>)
            });
            let bindings = Rc::clone(&self.client_commands);
            let actions = Rc::clone(&self.actions);
            let contexts = Rc::clone(&self.handle.contexts);
            let commands = Rc::clone(&self.commands);
            let scores: ClientScoresFn = Rc::new(move |invocation: &CommandInvocation| {
                let mut execute = {
                    let actions = Rc::clone(&actions);
                    let contexts = Rc::clone(&contexts);
                    let commands = Rc::clone(&commands);
                    move |command: &CommandInvocation, seat: &SeatId| {
                        let name = command.argv.first().cloned().unwrap_or_default();
                        let args = command.argv.get(1..).unwrap_or(&[]).to_vec();
                        let source = contexts
                            .borrow()
                            .get(seat)
                            .cloned()
                            .unwrap_or_else(|| commands.borrow().context().clone());
                        actions.borrow_mut().execute(&name, &args, Some(seat), &source);
                    }
                };
                bindings
                    .borrow_mut()
                    .dispatch(invocation, &mut execute)
                    .unwrap_or(false)
            });
            let mut registry = self.handle.registry();
            let names = register_input_commands(&mut registry, lookup, Some(scores));
            for name in names {
                self.unregister.push(Unregister::Command(name));
            }
        }
        // Binding commands only without a startup.
        if self.startup.is_none() {
            let shadows = Rc::new(self.shadows.clone());
            let lookup: BindingLookup = Rc::new(move |seat| shadows.get(seat).cloned());
            let print = self.print_router.clone();
            let binding_print: PrintFn = Rc::new(move |text| print.print_direct(text, None));
            let mut registry = self.handle.registry();
            let names = register_binding_commands(&mut registry, lookup, binding_print, None);
            for name in names {
                self.unregister.push(Unregister::Command(name));
            }
        }
        // Discovery and LLM console commands.
        {
            let names = register_discovery_commands(&mut self.console_commands.borrow_mut());
            for name in names {
                self.unregister.push(Unregister::Console(name));
            }
            let llm = self.actions.borrow().llm();
            let names = register_llm_commands(&mut self.console_commands.borrow_mut(), llm);
            for name in names {
                self.unregister.push(Unregister::Console(name));
            }
        }
        // Quake II client commands through UI execution.
        {
            let seat_ui = Rc::clone(&self.seat_ui);
            let actions = Rc::clone(&self.actions);
            let execute: Q2ClientCommandExecute = Rc::new(
                move |name: &str, args: &[String], seat: Option<SeatId>, source: &CommandContext| {
                    execute_ui_command(&seat_ui, &actions, name, args, seat.as_ref(), source);
                },
            );
            let cvars = self.console_cvars.borrow();
            let guard = register_q2_client_commands(&mut self.commands.borrow_mut(), source_dialect, &cvars, execute)?;
            self.unregister.push(Unregister::Q2(guard));
        }
        // Quake I client commands straight through the application.
        {
            let actions = Rc::clone(&self.actions);
            let execute: Q1ClientCommandExecute = Rc::new(
                move |name: &str, args: &[String], seat: Option<SeatId>, source: &CommandContext| {
                    actions.borrow_mut().execute(name, args, seat.as_ref(), source);
                },
            );
            let cvars = self.console_cvars.borrow();
            let guard = register_q1_client_commands(&mut self.commands.borrow_mut(), source_dialect, &cvars, execute)?;
            self.unregister.push(Unregister::Q1(guard));
        }
        // Quake I view-size commands.
        {
            let cvars = self.console_cvars.borrow();
            let guard = register_q1_view_commands(&mut self.commands.borrow_mut(), &cvars)?;
            self.unregister.push(Unregister::View(guard));
        }
        self.register_command(
            "quit",
            Rc::new({
                let actions = Rc::clone(&self.actions);
                move |_| {
                    actions.borrow_mut().quit();
                }
            }),
        );
        // Offhand buttons.
        let offhand_capabilities = self.binding_capabilities();
        for name in ["+grapple", "-grapple", "+grenade", "-grenade"] {
            let offhand = Rc::clone(&self.offhand);
            let actions = Rc::clone(&self.actions);
            let print = self.print_router.clone();
            let now = Rc::clone(&self.now);
            let capabilities = offhand_capabilities.clone();
            let handler: CommandHandler = Rc::new(move |invocation: &mut Invocation| {
                let origin = root_origin(&invocation.source.origin).clone();
                let CommandOrigin::LocalSeat { seat, .. } = origin else {
                    return;
                };
                let grapple = name.ends_with("grapple");
                if !(if grapple {
                    capabilities.offhand_grapple
                } else {
                    capabilities.offhand_grenades
                }) {
                    if name.starts_with('+') {
                        print.print(
                            "This session has no selected offhand action.\n",
                            Some(&invocation.source),
                        );
                    }
                    return;
                }
                let mut offhand = offhand.borrow_mut();
                let buttons = offhand.entry(seat.clone()).or_default();
                let button = if grapple {
                    &mut buttons.grapple
                } else {
                    &mut buttons.grenade
                };
                let active = button.active();
                let key = invocation.argv.get(1).cloned().unwrap_or_else(|| "console".to_string());
                let time = now() as i64;
                if name.starts_with('+') {
                    button.down(&key, time);
                } else if invocation.argv.get(1).is_none() {
                    button.release(time);
                } else {
                    button.up(&key, time);
                }
                if active != button.active() {
                    let source = invocation.source.clone();
                    actions.borrow_mut().execute(name, &[], Some(&seat), &source);
                }
            });
            self.register_command(name, handler);
        }
        // UI commands (Quake II skips names the client game owns).
        let audio: Vec<String> = APPLICATION_AUDIO_COMMANDS.iter().map(|name| name.to_string()).collect();
        let mut ui_commands = vec![
            "local_join",
            "local_drop",
            "weapnext",
            "weapprev",
            "switchweapon",
            "use",
            "weapon",
            "save",
            "load",
            "map",
            "say",
            "say_team",
            "centerview",
        ]
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
        ui_commands.extend(audio);
        for name in ui_commands {
            if matches!(source_dialect, Dialect::Q2Classic | Dialect::Q2Rerelease)
                && self.commands.borrow().exists(&name)
            {
                continue;
            }
            let seat_ui = Rc::clone(&self.seat_ui);
            let actions = Rc::clone(&self.actions);
            let owned = name.clone();
            let handler: CommandHandler = Rc::new(move |invocation: &mut Invocation| {
                let origin = root_origin(&invocation.source.origin).clone();
                let seat = match &origin {
                    CommandOrigin::LocalSeat { seat, .. } => Some(seat.clone()),
                    _ => None,
                };
                let args = invocation.argv.get(1..).unwrap_or(&[]).to_vec();
                let source = invocation.source.clone();
                execute_ui_command(&seat_ui, &actions, &owned, &args, seat.as_ref(), &source);
            });
            self.register_command(&name, handler);
        }
        // Bind UI callbacks and publish console focus.
        for local in &self.locals {
            let seat = local.seat_id().clone();
            if self.ui_ids.contains_key(&seat) {
                continue;
            }
            let callback = self.ui_callback(&seat);
            let now = (self.now)();
            let mut registry = self.handle.registry();
            let mut router = self.router.borrow_mut();
            let Some(seat_input) = router.seat_mut(&seat) else {
                continue;
            };
            seat_input.unbind_ui_event(0, now, &mut registry);
            let id = seat_input.bind_ui_event(callback, now, &mut registry);
            if !preserve_focus {
                seat_input.set_focus(SeatFocus::Game, now, &mut registry);
            }
            self.ui_ids.insert(seat.clone(), id);
        }
        for local in &self.locals {
            let focus = self
                .router
                .borrow()
                .seat(local.seat_id())
                .map(|seat| seat.focus().clone())
                .unwrap_or(SeatFocus::Game);
            local.console.borrow_mut().publish(seat_focus_to_console(&focus));
        }
        // Activate client commands.
        {
            let mut registry = self.handle.registry();
            self.client_commands.borrow_mut().activate(&mut registry);
        }
        // Drain staged commands.
        for staged in std::mem::take(&mut self.staged) {
            match staged {
                StagedCommand::Console { text, source } => {
                    self.commands.borrow_mut().append(&text, Some(&source), None)?;
                }
                StagedCommand::Reliable { text, source, dispatch } => dispatch(text, source),
            }
        }
        Ok(())
    }

    /// Retire commands (donor `retireCommands`).
    fn retire_commands(&mut self) {
        {
            let mut registry = self.handle.registry();
            self.client_commands.borrow_mut().deactivate(&mut registry);
        }
        let retire_now = (self.now)();
        let mut retire_registry = self.handle.registry();
        for local in &self.locals {
            let seat = local.seat_id().clone();
            if let Some(id) = self.ui_ids.remove(&seat) {
                if let Some(seat_input) = self.router.borrow_mut().seat_mut(&seat) {
                    seat_input.unbind_ui_event(id, retire_now, &mut retire_registry);
                }
            }
        }
        for unregister in std::mem::take(&mut self.unregister) {
            match unregister {
                Unregister::Command(name) => {
                    self.commands.borrow_mut().unregister(&name);
                }
                Unregister::Console(name) => {
                    self.console_commands.borrow_mut().unregister(&name);
                }
                Unregister::Q1(guard) => {
                    guard.release(&mut self.commands.borrow_mut());
                }
                Unregister::Q2(guard) => {
                    guard.release(&mut self.commands.borrow_mut());
                }
                Unregister::View(guard) => {
                    guard.release(&mut self.commands.borrow_mut());
                }
                Unregister::Output(release) => release(),
            }
        }
        self.commands_active = false;
    }
}

/// Owned registry plus its session (donor registry object adoption).
///
/// `CvarRegistry` is not `Clone`; the content round-trips through the
/// world-transfer save image instead.
fn owned_registry(registry: &CvarRegistry, fallback: &SessionId) -> Result<OwnedRegistry, InputError> {
    let session = registry.session().cloned().unwrap_or_else(|| fallback.clone());
    let mut owned = seed_registry(registry.dialect(), &session);
    apply_state(&mut owned, &registry.capture_world_transfer_state())?;
    Ok((owned, session))
}

/// Owned registry plus its session from a shared cell.
fn owned_cell(cell: &SharedRegistry, fallback: &SessionId) -> Result<OwnedRegistry, InputError> {
    owned_registry(&cell.borrow(), fallback)
}

impl ApplicationInput {
    /// Wire console-shared seat and server cells (donor console routing half).
    ///
    /// Cells are cached by `Rc` so guest sessions can borrow them through
    /// `&self`; missing seats rebuild from the startup image or seed fresh.
    /// Existing entries are kept, so adopt flows refresh explicitly.
    fn wire_console_cells(&mut self) -> Result<(), InputError> {
        let actions = self.actions.borrow();
        let console = actions.console();
        let server = console.and_then(InputConsole::server);
        let seats: Vec<(SeatId, Option<SharedRegistry>)> = self
            .locals
            .iter()
            .map(|local| {
                let seat = local.seat_id().clone();
                (seat.clone(), console.and_then(|console| console.seat(&seat)))
            })
            .collect();
        drop(actions);
        if server.is_some() {
            self.server_cvars = server;
        }
        let views = self
            .startup
            .as_ref()
            .map(|startup| startup.borrow().seats())
            .unwrap_or_default();
        for (seat, cell) in seats {
            if self.seat_cvars.contains_key(&seat) {
                continue;
            }
            let cell = match cell {
                Some(cell) => cell,
                None => {
                    let image = views.iter().find(|view| view.id == seat);
                    let (dialect, session) = match image {
                        Some(view) => (view.cvars.dialect, view.cvars.session.clone()),
                        None => (self.console_cvars.borrow().dialect(), self.session.clone()),
                    };
                    let mut registry = seed_registry(dialect, &session);
                    if let Some(view) = image {
                        apply_state(&mut registry, &view.cvars.state)?;
                    }
                    shared_registry(registry)
                }
            };
            self.seat_cvars.insert(seat, cell);
        }
        Ok(())
    }

    /// Adopted routing from live state (donor `adoptStartup` routing half).
    fn adopted_routing(&self) -> AdoptedRouting {
        let seats: Vec<(SeatId, ClientId, SharedRegistry)> = self
            .locals
            .iter()
            .filter_map(|local| {
                let seat = local.seat_id().clone();
                self.seat_cvars
                    .get(&seat)
                    .map(|cell| (seat, local.player.seat.client_id().clone(), Rc::clone(cell)))
            })
            .collect();
        let mice: Vec<(SeatId, ClientId, SharedRegistry)> = self
            .locals
            .iter()
            .filter_map(|local| {
                let seat = local.seat_id().clone();
                self.mouse
                    .get(&seat)
                    .map(|cell| (seat, local.player.seat.client_id().clone(), Rc::clone(cell)))
            })
            .collect();
        AdoptedRouting {
            session: self.session.clone(),
            source_dialect: self.console_cvars.borrow().dialect(),
            fallback: Rc::clone(&self.console_cvars),
            movement: Some(Rc::clone(&self.cvars)),
            shared: self.shared.clone(),
            server: self.server_cvars.clone(),
            seats,
            mice,
        }
    }

    /// Adopted owners view (donor `adoptStartup` owners).
    fn adopted_owners(&self, read: StartupScriptRead) -> Result<AdoptedOwnersView, InputError> {
        let session = self.session.clone();
        let server_cvars = self.server_cvars.as_ref().map(|(cell, _)| cell);
        let source = match server_cvars {
            Some(cell) => owned_cell(cell, &session)?,
            None if self.external_routing.is_none() => {
                let startup = self.startup.as_ref().expect("startup adoption lost its startup");
                let images = startup.borrow().registries();
                let mut registry = seed_registry(images.source.dialect, &images.source.session);
                apply_state(&mut registry, &images.source.state)?;
                (registry, images.source.session)
            }
            None => owned_cell(&self.console_cvars, &session)?,
        };
        Ok(AdoptedOwnersView {
            source,
            movement: owned_cell(&self.cvars, &session)?,
            fallback: owned_cell(&self.console_cvars, &session)?,
            scripts: Rc::clone(&self.scripts),
            read,
        })
    }

    /// Validate startup adoption (donor `validateStartupAdoption`).
    pub fn validate_startup_adoption(&self) -> Result<(), InputError> {
        let startup = match self.startup.as_ref() {
            Some(cell) => cell,
            None => return Ok(()),
        };
        let read = {
            let actions = self.actions.borrow();
            let scripts = self.scripts.borrow();
            actions.startup_reader(&*scripts, &self.options)
        };
        let Some(read) = read else {
            return Err(InputError::StartupReader);
        };
        let owners = self.adopted_owners(read)?;
        startup.borrow().validate_owners(&owners)
    }

    /// Adopt the prepared startup (donor `adoptStartup`).
    pub fn adopt_startup(&mut self) -> Result<(), InputError> {
        let startup = match self.startup.clone() {
            Some(cell) => cell,
            None => return Ok(()),
        };
        let read = {
            let actions = self.actions.borrow();
            let scripts = self.scripts.borrow();
            actions.startup_reader(&*scripts, &self.options)
        };
        let Some(read) = read else {
            return Err(InputError::StartupReader);
        };
        let routing = self.adopted_routing();
        let forward: ForwardFn = {
            let actions = Rc::clone(&self.actions);
            Rc::new(move |name: &str, args: &[String], source: &CommandContext| {
                let seat = match root_origin(&source.origin) {
                    CommandOrigin::LocalSeat { seat, .. } => Some(seat),
                    _ => None,
                };
                actions.borrow_mut().execute(name, args, seat, source);
            })
        };
        let owners = self.adopted_owners(read)?;
        startup.borrow_mut().adopt(routing, forward, owners)?;
        for local in &self.locals {
            let seat = local.seat_id().clone();
            let wired = self.seat_cvars.get(&seat);
            let cvars = match wired {
                Some(cell) => Some(owned_cell(cell, &self.session)?.0),
                None if self.external_routing.is_some() => Some(owned_cell(&self.console_cvars, &self.session)?.0),
                None => None,
            };
            let mouse = self
                .mouse
                .get(&seat)
                .map(|cell| owned_cell(cell, &self.session).map(|owned| owned.0))
                .transpose()?;
            startup.borrow_mut().adopt_seat(&seat, cvars, mouse);
        }
        if !self.configuration_published && self.actions.borrow().configuration().is_some() {
            if self.candidate.is_some() {
                return Err(InputError::ConfigurationOrder);
            }
            self.actions
                .borrow_mut()
                .configuration_mut()
                .expect("configuration vanished during adoption")
                .publish_continuation(&startup)?;
            self.configuration_published = true;
        }
        Ok(())
    }

    /// Release routed seats and device state (donor `releaseForProfileChange` core).
    fn release_cells(
        router: &RouterCell,
        devices: &DeviceCell,
        contexts: &Rc<RefCell<HashMap<SeatId, CommandContext>>>,
        time: f64,
        append: Option<ReleaseSink>,
    ) {
        let ids: Vec<SeatId> = router.borrow().seats().iter().map(|seat| seat.seat().clone()).collect();
        let fallback = contexts.borrow().values().next().cloned();
        let Some(fallback) = fallback else {
            return;
        };
        let mut owned = append.as_ref().map(|cell| cell.borrow_mut());
        {
            let mut discard = |_: &str, _: &CommandContext| {};
            let sink: &mut dyn FnMut(&str, &CommandContext) = match owned.as_mut() {
                Some(guard) => &mut **guard,
                None => &mut discard,
            };
            let mut registry = SinkRegistry {
                contexts: Rc::clone(contexts),
                fallback: fallback.clone(),
                sink,
            };
            let mut router_ref = router.borrow_mut();
            for id in &ids {
                if let Some(seat) = router_ref.seat_mut(id) {
                    seat.release(time, &mut registry);
                }
            }
        }
        match owned.as_mut() {
            Some(guard) => {
                let sink: &mut dyn FnMut(&str, &CommandContext) = &mut **guard;
                let mut text_sink = |text: &str| sink(text, &fallback);
                let _ = devices.borrow_mut().release(time as i64, Some(&mut text_sink));
            }
            None => {
                let _ = devices.borrow_mut().release(time as i64, None);
            }
        }
    }

    /// Release offhand buttons (donor `releaseOffhand`).
    fn release_offhand(&mut self, all: bool) {
        let now = (self.now)();
        for local in &self.locals {
            let seat = local.seat_id().clone();
            if !all {
                let focused_game = self
                    .router
                    .borrow()
                    .seat(&seat)
                    .is_some_and(|seat| seat.focused() && matches!(seat.focus(), SeatFocus::Game));
                if focused_game {
                    continue;
                }
            }
            let mut offhand = self.offhand.borrow_mut();
            let Some(buttons) = offhand.get_mut(&seat) else {
                continue;
            };
            for name in ["grapple", "grenade"] {
                let button = if name == "grapple" {
                    &mut buttons.grapple
                } else {
                    &mut buttons.grenade
                };
                if !button.active() {
                    continue;
                }
                button.release(now as i64);
                self.actions
                    .borrow_mut()
                    .execute(&format!("-{name}"), &[], Some(&seat), &local.context);
            }
        }
    }

    /// Close the input owner (donor `close`).
    pub fn close(&mut self) {
        self.release_offhand(true);
        if self.owns_devices {
            let _ = self.devices.borrow_mut().close((self.now)() as i64);
        }
        self.settings_ctrl.close();
        for local in &self.locals {
            local.haptics.borrow_mut().close();
        }
        let _ = self.router.borrow_mut().close();
        if self.owns_controllers {
            self.controllers.borrow_mut().close();
        }
        self.staged.clear();
        self.seat_ui.borrow_mut().clear();
        self.q3_selections.clear();
        self.arsenals.clear();
        self.retire_commands();
        if let Some(configuration) = self.actions.borrow_mut().configuration_mut() {
            configuration.close_routing();
        }
    }
}

/// Guest command buffer view (donor `guestCommands`).
pub struct GuestCommands<'a> {
    /// Live buffer guard or staged program buffer.
    buffer: GuestBuffer<'a>,
}

/// Borrowed guest command buffer.
enum GuestBuffer<'a> {
    /// Live command buffer.
    Live(RefMut<'a, CommandBuffer>),
    /// Staged candidate program buffer.
    Candidate(&'a mut CommandBuffer),
}

impl GuestCommands<'_> {
    /// Dispatch text immediately.
    pub fn execute_now(
        &mut self,
        text: Option<&str>,
        source: Option<&CommandContext>,
        dialect: Option<Dialect>,
        cvars: &mut CvarRegistry,
        services: &mut dyn BufferServices,
    ) -> Result<usize, InputError> {
        let buffer = match &mut self.buffer {
            GuestBuffer::Live(guard) => &mut **guard,
            GuestBuffer::Candidate(buffer) => *buffer,
        };
        Ok(buffer.execute_now(text, source, dialect, cvars, services)?)
    }

    /// Queue text ahead of the pending program.
    pub fn insert(
        &mut self,
        text: &str,
        source: Option<&CommandContext>,
        dialect: Option<Dialect>,
    ) -> Result<(), InputError> {
        let buffer = match &mut self.buffer {
            GuestBuffer::Live(guard) => &mut **guard,
            GuestBuffer::Candidate(buffer) => *buffer,
        };
        Ok(buffer.insert(text, source, dialect)?)
    }

    /// Queue text behind the pending program.
    pub fn append(
        &mut self,
        text: &str,
        source: Option<&CommandContext>,
        dialect: Option<Dialect>,
    ) -> Result<(), InputError> {
        let buffer = match &mut self.buffer {
            GuestBuffer::Live(guard) => &mut **guard,
            GuestBuffer::Candidate(buffer) => *buffer,
        };
        Ok(buffer.append(text, source, dialect)?)
    }
}

impl StagedClientCommands {
    /// Publish the staged program and bindings (donor `publish`).
    pub fn publish(
        &mut self,
        live: &mut CommandBuffer,
        bindings: &mut dyn FnMut(&SeatId, Vec<InputBinding>),
    ) -> Result<(), InputError> {
        live.copy_pending_from(&self.buffer)?;
        self.publish_bindings(bindings)
    }

    /// Release cleared staged inputs into the program buffer (donor `releaseInputs`).
    pub fn release_inputs(&mut self, release: &mut dyn FnMut(&SeatId, &mut dyn CommandRegistry)) {
        let mut registry = ProgramRegistry {
            buffer: &mut self.buffer,
        };
        for (seat, cleared) in &self.cleared {
            if *cleared.borrow() {
                release(seat, &mut registry);
            }
        }
    }
}

impl InputQ3Owners<'_> {
    /// Push a cell unless an identical cell is already queued.
    ///
    /// Cells that alias (movement over fallback in one-dialect games) borrow
    /// once under the first kind; canonical resolution falls through to the
    /// surviving guard.
    fn push_unique<'a>(queued: &mut Vec<(SlotKind, &'a SharedRegistry)>, kind: SlotKind, cell: &'a SharedRegistry) {
        if queued.iter().any(|(_, known)| Rc::ptr_eq(known, cell)) {
            return;
        }
        queued.push((kind, cell));
    }
}

impl ApplicationInput {
    /// Whether a local player may leave (donor `canRemoveLocalPlayer`).
    #[must_use]
    pub fn can_remove_local_player(&self, seat: &SeatId) -> bool {
        self.locals.len() > 1 && self.actions.borrow().can_remove_local_player(seat)
    }

    /// Persist cvar archives on save (donor `enableArchivePersistence`).
    pub fn enable_archive_persistence(&mut self) {
        self.archive_persistence = true;
    }

    /// Print through the execution context (donor `print`).
    pub fn print(&self, text: &str, source: Option<&CommandContext>) {
        self.print_router.print(text, source);
    }

    /// Guest command buffer (donor `guestCommands`).
    pub fn guest_commands(&mut self) -> GuestCommands<'_> {
        if let Some(candidate) = self.candidate.as_mut() {
            GuestCommands {
                buffer: GuestBuffer::Candidate(&mut candidate.buffer),
            }
        } else {
            GuestCommands {
                buffer: GuestBuffer::Live(self.commands.borrow_mut()),
            }
        }
    }

    /// Guest cvar owners for a seat (donor `guestCvars`).
    pub fn guest_cvars(&self, seat: &SeatId) -> Result<Q3ClientCvars<InputQ3Owners<'_>>, InputError> {
        let local = self
            .locals
            .iter()
            .find(|local| local.seat_id() == seat)
            .ok_or(InputError::GuestSeat)?;
        // Every guard borrows through `&self`: seat and server cells were
        // cached by `Rc` at wire-up time, so no per-call console borrow
        // escapes its frame.
        let mut queued: Vec<(SlotKind, &SharedRegistry)> = Vec::new();
        InputQ3Owners::push_unique(&mut queued, SlotKind::Fallback, &self.console_cvars);
        InputQ3Owners::push_unique(&mut queued, SlotKind::Movement, &self.cvars);
        if let Some(shared) = self.shared.as_ref() {
            InputQ3Owners::push_unique(&mut queued, SlotKind::Shared, shared);
        }
        if let Some((server, _)) = self.server_cvars.as_ref() {
            InputQ3Owners::push_unique(&mut queued, SlotKind::Server, server);
        }
        if let Some(seat_cell) = self.seat_cvars.get(seat) {
            InputQ3Owners::push_unique(&mut queued, SlotKind::Seat, seat_cell);
        }
        if let Some(mouse) = self.mouse.get(seat) {
            InputQ3Owners::push_unique(&mut queued, SlotKind::Mouse, mouse);
        }
        let mut guards = Vec::with_capacity(queued.len());
        let mut layout = Vec::with_capacity(queued.len());
        let mut order = Vec::with_capacity(queued.len());
        for (kind, cell) in queued {
            guards.push(cell.borrow_mut());
            layout.push(kind);
            order.push(Rc::clone(cell));
        }
        let server_names = self
            .server_cvars
            .as_ref()
            .map(|(_, names)| names.clone())
            .unwrap_or_default();
        Ok(Q3ClientCvars::new(InputQ3Owners {
            guards,
            layout,
            order,
            published: Rc::clone(&self.published),
            external: self.external_routing.as_deref(),
            print: self.print_router.clone(),
            context: local.context.clone(),
            server_names,
            session: self.session.clone(),
            source_dialect: self.console_cvars.borrow().dialect(),
        }))
    }

    /// Guest seat input for a seat (donor `guestInput`).
    pub fn guest_input(&self, seat: &SeatId) -> Result<GuestSeatInput, InputError> {
        let local = self
            .locals
            .iter()
            .find(|local| local.seat_id() == seat)
            .ok_or(InputError::GuestInput)?;
        let (staged, cleared) = match self.candidate.as_ref() {
            Some(candidate) => (candidate.staged_table(seat), candidate.cleared_flag(seat)),
            None => (None, None),
        };
        Ok(GuestSeatInput {
            router: Rc::clone(&self.router),
            seat: seat.clone(),
            staged,
            cleared,
            contexts: Rc::clone(&self.handle.contexts),
            context: local.context.clone(),
            now: Rc::clone(&self.now),
        })
    }

    /// Publish a retained shared registry (donor `publishSharedCvars`).
    pub fn publish_shared_cvars(&mut self, registry: SharedRegistry) {
        if let Some(shared) = self.shared.as_ref() {
            self.published
                .borrow_mut()
                .insert(Rc::as_ptr(shared) as usize, Rc::clone(&registry));
        }
        self.shared = Some(registry);
    }

    /// Redirect a candidate registry at a retained one (donor `adoptCvarOwner`).
    pub fn adopt_cvar_owner(&mut self, candidate: &SharedRegistry, retained: SharedRegistry) {
        self.published
            .borrow_mut()
            .insert(Rc::as_ptr(candidate) as usize, Rc::clone(&retained));
        if Rc::ptr_eq(&self.cvars, candidate) {
            self.cvars = Rc::clone(&retained);
        }
        if Rc::ptr_eq(&self.console_cvars, candidate) {
            self.console_cvars = Rc::clone(&retained);
        }
    }

    /// Adopt client source registries (donor `adoptClientSource`).
    pub fn adopt_client_source(&mut self, owner: ApplicationInputCommandOwner) -> Result<(), InputError> {
        if self.external_routing.is_none() {
            return Err(InputError::AdoptDialect);
        }
        let movement = owner.movement.clone().unwrap_or_else(|| Rc::clone(&owner.cvars));
        let fallback = owner.fallback.clone().unwrap_or_else(|| Rc::clone(&owner.cvars));
        if movement.borrow().dialect() != self.dialect
            || fallback.borrow().dialect() != owner.cvars.borrow().dialect()
            || owner.cvars.borrow().dialect() != self.console_cvars.borrow().dialect()
        {
            return Err(InputError::AdoptDialect);
        }
        self.cvars = movement;
        self.console_cvars = fallback;
        self.external_routing = Some(owner.routing);
        if let Some(scripts) = owner.scripts {
            self.scripts = scripts;
        }
        let seats: Vec<SeatId> = self
            .startup
            .as_ref()
            .map(|startup| startup.borrow().seats().iter().map(|seat| seat.id.clone()).collect())
            .unwrap_or_default();
        for seat in seats {
            if self.locals.iter().any(|local| local.seat_id() == &seat) {
                let Some(view) = self
                    .startup
                    .as_ref()
                    .and_then(|startup| startup.borrow().seats().into_iter().find(|view| view.id == seat))
                else {
                    continue;
                };
                let mut mouse = seed_registry(view.mouse.dialect, &view.mouse.session);
                apply_state(&mut mouse, &view.mouse.state)?;
                self.adopt_mouse_owner(&seat, shared_registry(mouse))?;
            }
        }
        Ok(())
    }

    /// Adopt a retained mouse registry (donor `adoptMouseOwner`).
    pub fn adopt_mouse_owner(&mut self, seat: &SeatId, retained: SharedRegistry) -> Result<(), InputError> {
        let candidate = self.mouse.get(seat).ok_or(InputError::MouseSeat)?;
        self.published
            .borrow_mut()
            .insert(Rc::as_ptr(candidate) as usize, Rc::clone(&retained));
        self.mouse.insert(seat.clone(), Rc::clone(&retained));
        let tuning = read_mouse_tuning(&retained.borrow());
        if let Some(local) = self.locals.iter_mut().find(|local| local.seat_id() == seat) {
            local.builder.mouse.tuning = tuning;
        } else {
            return Err(InputError::MouseSeat);
        }
        Ok(())
    }

    /// Validate the candidate program (donor `validateCandidateCommands`).
    pub fn validate_candidate_commands(&self) -> Result<(), InputError> {
        let Some(candidate) = self.candidate.as_ref() else {
            return Ok(());
        };
        let live_revision = self.commands.borrow().program_revision();
        candidate.validate_publication(live_revision, &|seat| {
            self.router
                .borrow()
                .seat(seat)
                .map(|seat| seat.bindings().into_iter().cloned().collect())
                .unwrap_or_default()
        })
    }

    /// Release into the candidate program (donor `releaseIntoCandidate`).
    pub fn release_into_candidate(
        &mut self,
        previous: &mut ApplicationInput,
        world_changed: bool,
    ) -> Result<(), InputError> {
        if world_changed || self.profile_changed() {
            let released: Rc<RefCell<Vec<(String, CommandContext)>>> = Rc::new(RefCell::new(Vec::new()));
            let append = {
                let released = Rc::clone(&released);
                Rc::new(RefCell::new(move |text: &str, source: &CommandContext| {
                    released.borrow_mut().push((text.to_string(), source.clone()));
                })) as Rc<RefCell<dyn FnMut(&str, &CommandContext)>>
            };
            previous.release_for_profile_change(Some(append));
            if let Some(candidate) = self.candidate.as_mut() {
                for (text, source) in released.borrow().iter() {
                    candidate.release_append(text, source);
                }
            }
        } else if let Some(candidate) = self.candidate.as_mut() {
            let now = (self.now)();
            let router = Rc::clone(&self.router);
            candidate.release_inputs(&mut |seat, registry| {
                if let Some(seat_input) = router.borrow_mut().seat_mut(seat) {
                    seat_input.release(now, registry);
                }
            });
        }
        Ok(())
    }

    /// Publish the candidate program (donor `publishCandidateCommands`).
    pub fn publish_candidate_commands(&mut self) -> Result<(), InputError> {
        if let Some(mut candidate) = self.candidate.take() {
            let router = Rc::clone(&self.router);
            let mut commands = self.commands.borrow_mut();
            candidate.publish(&mut commands, &mut |seat, bindings| {
                if let Some(seat_input) = router.borrow_mut().seat_mut(seat) {
                    seat_input.unbind_all();
                    for binding in bindings {
                        seat_input.bind(binding);
                    }
                }
            })?;
        }
        if let Some(startup) = self.startup.as_ref() {
            for (seat, defaults) in &self.candidate_defaults {
                startup.borrow_mut().adopt_binding_defaults(seat, defaults.clone());
            }
        }
        for (seat, tuning) in &self.candidate_gamepads {
            if let Some(seat_input) = self.router.borrow_mut().seat_mut(seat) {
                seat_input.gamepad.tuning = *tuning;
            }
        }
        self.candidate_defaults.clear();
        self.candidate_gamepads.clear();
        Ok(())
    }
}

impl ApplicationInput {
    /// Whether the profile changed under the input (donor `profileChanged`).
    #[must_use]
    pub fn profile_changed(&self) -> bool {
        let expected = self
            .actions
            .borrow()
            .console()
            .map(InputConsole::dialect)
            .unwrap_or(self.dialect);
        if self.commands.borrow().dialect() != expected {
            return true;
        }
        self.locals.iter().any(|local| {
            self.router
                .borrow()
                .seat(local.seat_id())
                .is_some_and(|seat| seat.dialect() != self.dialect)
        })
    }

    /// Release held input for a profile change (donor `releaseForProfileChange`).
    pub fn release_for_profile_change(&mut self, append: Option<ReleaseSink>) {
        self.release_offhand(true);
        let now = (self.now)();
        Self::release_cells(&self.router, &self.devices, &self.handle.contexts, now, append);
    }

    /// Poll window and controller events during loading (donor `pollLoadingEvents`).
    pub fn poll_loading_events(&mut self) -> Result<(), InputError> {
        let events = self.window.borrow_mut().poll_events()?;
        let quit = events.iter().any(|event| {
            matches!(
                event,
                SdlEvent::Quit { .. }
                    | SdlEvent::Window {
                        event: WINDOW_CLOSE,
                        ..
                    }
            )
        });
        self.pending_window.extend(events);
        self.pending_controller
            .extend(self.controllers.borrow_mut().poll_events()?);
        if quit {
            self.actions.borrow_mut().quit();
        }
        Ok(())
    }

    /// Pump one input frame (donor `pump`).
    pub fn pump(&mut self, execute_commands: bool) -> Result<(), InputError> {
        self.release_offhand(false);
        self.synchronize_client_focus();
        let mut window_events = std::mem::take(&mut self.pending_window);
        window_events.extend(self.window.borrow_mut().poll_events()?);
        for event in window_events {
            if matches!(
                event,
                SdlEvent::Window {
                    event: WINDOW_FOCUS_LOST,
                    ..
                }
            ) {
                self.stop_haptics();
            }
            self.router.borrow_mut().handle_platform(event)?;
        }
        self.devices.borrow_mut().frame((self.now)() as i64)?;
        let mut controller_events = std::mem::take(&mut self.pending_controller);
        controller_events.extend(self.controllers.borrow_mut().poll_events()?);
        for event in controller_events {
            self.router.borrow_mut().handle_controller(event)?;
        }
        self.settings_ctrl.update();
        if execute_commands {
            let mut services = InputBufferServices {
                startup: self.startup.clone(),
                scripts: Rc::clone(&self.scripts),
                actions: Rc::clone(&self.actions),
            };
            let mut commands = self.commands.borrow_mut();
            let mut cvars = self.console_cvars.borrow_mut();
            commands.execute(&mut cvars, &mut services)?;
        }
        self.router.borrow_mut().update_capture()?;
        for local in &self.locals {
            let seat = local.seat_id().clone();
            let active = self
                .router
                .borrow()
                .seat(&seat)
                .is_some_and(|seat| seat.focused() && matches!(seat.focus(), SeatFocus::Game));
            let mut haptics = local.haptics.borrow_mut();
            haptics.set_active(active);
            let _ = haptics.update();
        }
        Ok(())
    }

    /// Mirror client input capture into seat focus (donor `synchronizeClientFocus`).
    fn synchronize_client_focus(&mut self) {
        let now = (self.now)();
        for local in &self.locals {
            let seat = local.seat_id().clone();
            let captured = self.actions.borrow().client_captures_input(&seat);
            let mut router = self.router.borrow_mut();
            let Some(seat_input) = router.seat_mut(&seat) else {
                continue;
            };
            let focus = seat_input.focus().clone();
            match (&focus, captured) {
                (SeatFocus::Game, true) => {
                    let mut registry = self.handle.registry();
                    seat_input.set_focus(
                        SeatFocus::Menu {
                            menu: Q3_CGAME_MENU.to_string(),
                            control: None,
                        },
                        now,
                        &mut registry,
                    );
                }
                (SeatFocus::Menu { menu, .. }, false) if menu == Q3_CGAME_MENU => {
                    let mut registry = self.handle.registry();
                    seat_input.set_focus(SeatFocus::Game, now, &mut registry);
                }
                _ => {}
            }
        }
    }

    /// Publish a replacement window (donor `publishWindow`).
    pub fn publish_window(&mut self, next: WindowCell) -> Result<(), InputError> {
        if Rc::ptr_eq(&next, &self.window) {
            return Ok(());
        }
        let previous = Rc::clone(&self.window);
        self.release_for_profile_change(None);
        self.stop_haptics();
        self.pending_window.clear();
        self.window = next;
        if let Err(error) = self.router.borrow_mut().attach_window(Box::new(WindowAdapter {
            window: Rc::clone(&self.window),
        })) {
            self.window = previous;
            self.router.borrow_mut().attach_window(Box::new(WindowAdapter {
                window: Rc::clone(&self.window),
            }))?;
            return Err(error.into());
        }
        Ok(())
    }

    /// Deliver an input event to a seat (donor `input`).
    pub fn input(&mut self, event: &SeatInputEvent) -> Result<bool, InputError> {
        if let SeatInputEventKind::Focus { focused, .. } = &event.kind {
            if !focused {
                if let Some(local) = self.locals.iter().find(|local| local.seat_id() == &event.seat) {
                    local.haptics.borrow_mut().cancel();
                }
            }
        }
        self.synchronize_client_focus();
        let mut router = self.router.borrow_mut();
        let Some(seat) = router.seat_mut(&event.seat) else {
            return Ok(false);
        };
        let mut registry = self.handle.registry();
        Ok(seat.input(&ui_event_to_router(event), &mut registry)?)
    }

    /// Set the Quake III command selection (donor `setQ3CommandSelection`).
    pub fn set_q3_command_selection(&mut self, seat: &SeatId, selection: Q3CommandSelection) -> Result<(), InputError> {
        if !self.locals.iter().any(|local| local.seat_id() == seat) {
            return Err(InputError::CommandSeat);
        }
        self.q3_selections.insert(seat.clone(), selection);
        Ok(())
    }

    /// Set the arsenal selection (donor `setArsenalSelection`).
    pub fn set_arsenal_selection(
        &mut self,
        seat: &SeatId,
        selection: Option<ArsenalSelection>,
    ) -> Result<(), InputError> {
        if !self.locals.iter().any(|local| local.seat_id() == seat) {
            return Err(InputError::ArsenalSeat);
        }
        match selection {
            Some(selection) => {
                self.arsenals.insert(seat.clone(), selection);
            }
            None => {
                self.arsenals.remove(seat);
            }
        }
        Ok(())
    }

    /// Bind an arsenal provider (donor `bindArsenalProvider`).
    pub fn bind_arsenal_provider(&mut self, seat: &SeatId, provider: ProviderId) -> Result<(), InputError> {
        if self
            .arsenals
            .get(seat)
            .is_some_and(|selection| selection.provider == provider)
        {
            return Ok(());
        }
        self.set_arsenal_selection(seat, Some(ArsenalSelection { provider, weapon: None }))
    }

    /// Register a client-command owner (donor `clientCommandRegistration`).
    pub fn client_command_registration(
        &mut self,
        seat: &SeatId,
        handler: Option<ClientCommandHandler>,
    ) -> Result<ClientCommandOwner, InputError> {
        Ok(self.client_commands.borrow_mut().create_owner(seat.clone(), handler)?)
    }

    /// Attach seat UI (donor `attachUi`); returns the detach handle.
    pub fn attach_ui(&mut self, seat: &SeatId, ui: UiCell) -> Result<Box<dyn FnOnce()>, InputError> {
        if !self.locals.iter().any(|local| local.seat_id() == seat) {
            return Err(InputError::UiSeat);
        }
        if self.seat_ui.borrow().contains_key(seat) {
            return Err(InputError::UiAttached);
        }
        self.seat_ui.borrow_mut().insert(seat.clone(), ui);
        let seat_ui = Rc::clone(&self.seat_ui);
        let seat = seat.clone();
        Ok(Box::new(move || {
            seat_ui.borrow_mut().remove(&seat);
        }))
    }

    /// Enqueue a client command (donor `enqueueClientCommand`).
    pub fn enqueue_client_command(&mut self, text: &str, source: CommandContext) {
        if let Some(candidate) = self.candidate.as_mut() {
            let _ = candidate.buffer.append(text, Some(&source), None);
        } else if self.commands_active {
            let _ = self.commands.borrow_mut().append(text, Some(&source), None);
        } else {
            self.staged.push(StagedCommand::Console {
                text: text.to_string(),
                source,
            });
        }
    }

    /// Enqueue a reliable client command (donor `enqueueClientReliable`).
    pub fn enqueue_client_reliable(
        &mut self,
        text: &str,
        source: CommandContext,
        dispatch: Box<dyn FnOnce(String, CommandContext)>,
    ) {
        if self.commands_active {
            dispatch(text.to_string(), source);
        } else {
            self.staged.push(StagedCommand::Reliable {
                text: text.to_string(),
                source,
                dispatch,
            });
        }
    }

    /// Whether bindings can reset to authored defaults (donor `canResetBindings`).
    #[must_use]
    pub fn can_reset_bindings(&self, seat: &SeatId) -> bool {
        self.startup.as_ref().is_some_and(|startup| {
            startup
                .borrow()
                .seats()
                .iter()
                .any(|view| view.id == *seat && view.authored_bindings)
        })
    }

    /// Reset bindings to authored defaults (donor `resetBindings`).
    pub fn reset_bindings(&mut self, seat: &SeatId) -> Result<(), InputError> {
        let Some(startup) = self.startup.as_ref() else {
            return Err(InputError::BindingDefaults);
        };
        startup.borrow_mut().reset_bindings(seat)
    }

    /// Advance the pending startup one frame (donor `advanceStartup`).
    pub fn advance_startup(&mut self) -> Result<bool, InputError> {
        match self.startup.as_ref() {
            Some(startup) => startup.borrow_mut().advance_frame(),
            None => Ok(false),
        }
    }

    /// Resume command sequencing (donor `resumeCommands`).
    ///
    /// The donor takes an arbitrary number; values above the safe-integer
    /// range are rejected.
    pub fn resume_commands(&mut self, sequence: u64) -> Result<(), InputError> {
        if sequence > MAX_SEQUENCE {
            return Err(InputError::BadSequence);
        }
        self.sequence = self.sequence.max(sequence);
        Ok(())
    }

    /// Bind the haptic sample loader (donor `bindHaptics`).
    pub fn bind_haptics(&mut self, load: HapticLoader) {
        for local in &self.locals {
            local.haptics.borrow_mut().invalidate_assets();
        }
        *self.haptic_loader.borrow_mut() = load;
    }

    /// Stop all haptics (donor `stopHaptics`).
    pub fn stop_haptics(&mut self) {
        for local in &self.locals {
            local.haptics.borrow_mut().cancel();
        }
    }

    /// Play sound haptics (donor `soundHaptics`).
    pub fn sound_haptics(&mut self, content: &str, sound: &str, actor: Option<&ActorId>, audience: &AudioAudience) {
        let Some(actor) = actor else {
            return;
        };
        for local in &self.locals {
            if &local.player.actor != actor {
                continue;
            }
            if let AudioAudience::Seat { seat } = audience {
                if local.seat_id() != seat {
                    continue;
                }
            }
            let seat = local.seat_id().clone();
            let active = self
                .router
                .borrow()
                .seat(&seat)
                .is_some_and(|seat| seat.focused() && matches!(seat.focus(), SeatFocus::Game));
            let mut haptics = local.haptics.borrow_mut();
            haptics.set_active(active);
            let _ = haptics.sound(content, sound);
        }
    }
}

/// Sample a seat frame as builder input.
fn seat_sample(frame: &SeatFrame) -> SeatSample {
    SeatSample {
        buttons: frame.buttons.clone(),
        mouse: frame.mouse,
        gamepad_move: frame.gamepad_move,
        gamepad_look_degrees: frame.gamepad_look_degrees,
        impulse: frame.impulse,
        any_key_down: frame.any_key_down as i32,
        focus_is_game: matches!(frame.focus, SeatFocus::Game),
        frame_ms: frame.frame_ms,
    }
}

/// Built command as a net user command.
fn user_command(built: &BuiltCommand) -> UserCommand {
    match *built {
        BuiltCommand::Q1Netquake {
            ack_time_s,
            view_angles,
            forward,
            side,
            up,
            buttons,
            impulse,
        } => UserCommand::Q1Netquake {
            acknowledged_server_time_seconds: ack_time_s,
            view_angles: [
                f64::from(view_angles.x),
                f64::from(view_angles.y),
                f64::from(view_angles.z),
            ],
            forward_move: f64::from(forward),
            side_move: f64::from(side),
            up_move: f64::from(up),
            buttons: f64::from(buttons),
            impulse: f64::from(impulse),
        },
        BuiltCommand::Q1Quakeworld {
            msec,
            angles,
            forward,
            side,
            up,
            buttons,
            impulse,
        } => UserCommand::Q1Quakeworld {
            milliseconds: f64::from(msec),
            angles: [f64::from(angles.x), f64::from(angles.y), f64::from(angles.z)],
            forward_move: f64::from(forward),
            side_move: f64::from(side),
            up_move: f64::from(up),
            buttons: f64::from(buttons),
            impulse: f64::from(impulse),
        },
        BuiltCommand::Q2Classic {
            msec,
            angle_shorts,
            forward,
            side,
            up,
            buttons,
            impulse,
            light_level,
        } => UserCommand::Q2Classic {
            milliseconds: f64::from(msec),
            angle_shorts: [
                f64::from(angle_shorts[0]),
                f64::from(angle_shorts[1]),
                f64::from(angle_shorts[2]),
            ],
            forward_move: f64::from(forward),
            side_move: f64::from(side),
            up_move: f64::from(up),
            buttons: f64::from(buttons),
            impulse: f64::from(impulse),
            light_level: f64::from(light_level),
        },
        BuiltCommand::Q2Rerelease {
            msec,
            angles,
            forward,
            side,
            buttons,
            server_frame,
        } => UserCommand::Q2Rerelease {
            milliseconds: f64::from(msec),
            angles: [f64::from(angles.x), f64::from(angles.y), f64::from(angles.z)],
            forward_move: forward,
            side_move: side,
            buttons: f64::from(buttons),
            server_frame: f64::from(server_frame),
        },
        BuiltCommand::Q3 {
            server_time_ms,
            angle_words,
            forward,
            right,
            up,
            buttons,
            weapon,
        } => UserCommand::Q3 {
            server_time_milliseconds: f64::from(server_time_ms),
            angle_words: [
                f64::from(angle_words[0]),
                f64::from(angle_words[1]),
                f64::from(angle_words[2]),
            ],
            buttons: f64::from(buttons),
            weapon: f64::from(weapon),
            forward_move: f64::from(forward),
            right_move: f64::from(right),
            up_move: f64::from(up),
        },
    }
}

/// Provider id as an arsenal provider string.
fn arsenal_provider(provider: &ProviderId) -> String {
    format!("{}:{}", provider.namespace, provider.name)
}

impl ApplicationInput {
    /// Build actor commands for every local (donor `build`).
    pub fn build(
        &mut self,
        elapsed_ms: f64,
        server_ms: f64,
        server_frame: i32,
        wall_elapsed_ms: f64,
    ) -> Result<Vec<ActorCommand>, InputError> {
        let seats: Vec<SeatId> = self.locals.iter().map(|local| local.seat_id().clone()).collect();
        let mut commands = Vec::with_capacity(seats.len());
        for seat in seats {
            commands.push(self.build_for_seat(seat, elapsed_ms, server_ms, server_frame, wall_elapsed_ms)?);
        }
        Ok(commands)
    }

    /// Build the actor command for one seat (donor `buildForSeat`).
    ///
    /// The canonical builder derives its timing from the sample, so the
    /// server-frame `elapsed_ms` only documents the donor signature.
    pub fn build_for_seat(
        &mut self,
        seat: SeatId,
        elapsed_ms: f64,
        server_ms: f64,
        server_frame: i32,
        wall_elapsed_ms: f64,
    ) -> Result<ActorCommand, InputError> {
        let index = self
            .locals
            .iter()
            .position(|local| local.seat_id() == &seat)
            .ok_or(InputError::InactiveSeat)?;
        let drift = self.simulation.borrow().pitch_drift(&self.locals[index].player.actor);
        let frame = match self.dialect {
            Dialect::Q1Netquake => FrameContext::Q1Netquake {
                ack_time_s: server_ms / 1000.0,
                pitch_drift: drift,
            },
            Dialect::Q1Quakeworld => FrameContext::Q1Quakeworld { pitch_drift: drift },
            Dialect::Q2Classic => FrameContext::Q2Classic {
                delta_angles: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                light_level: 128,
                attack_allowed: true,
            },
            Dialect::Q2Rerelease => FrameContext::Q2Rerelease {
                delta_angles: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                server_frame,
                attack_allowed: true,
            },
            _ => {
                let selection = self.q3_selections.get(&seat);
                FrameContext::Q3 {
                    server_time_ms: server_ms.trunc() as i32,
                    weapon: selection.map(|selection| selection.weapon as i32).unwrap_or(2),
                    sensitivity: selection.map(|selection| selection.sensitivity).unwrap_or(1.0),
                }
            }
        };
        let now = (self.now)();
        let sample = local_sample(&self.router, &seat, now, wall_elapsed_ms)?;
        let mut sample = seat_sample(&sample);
        if let Some(ui) = self.seat_ui.borrow().get(&seat).cloned() {
            sample = ui.borrow_mut().sample(sample);
        }
        let _ = elapsed_ms;
        let built = self.locals[index].builder.build(&sample, &frame)?;
        let local = &self.locals[index];
        let impulse_provider = match self.dialect {
            Dialect::Q3 | Dialect::Q2Rerelease => self.actions.borrow().arsenal_impulse_provider(&seat),
            _ => None,
        };
        let arsenal = match impulse_provider {
            Some(provider) => Some(ArsenalIntent {
                provider: arsenal_provider(&provider),
                weapon: None,
                // The canonical intent carries no impulse word; the impulse
                // already rides in the built command.
                use_holdable: sample_use_holdable(&sample),
            }),
            None => self.arsenals.get(&seat).map(|selection| ArsenalIntent {
                provider: arsenal_provider(&selection.provider),
                weapon: selection.weapon.clone(),
                use_holdable: sample_use_holdable(&sample),
            }),
        };
        let sequence = self.sequence;
        self.sequence += 1;
        Ok(ActorCommand {
            actor: local.player.actor.clone(),
            source: CommandSource::LocalSeat { seat: seat.clone() },
            sequence,
            command: user_command(&built),
            arsenal,
        })
    }
}

/// Sample one routed seat.
fn local_sample(router: &RouterCell, seat: &SeatId, now: f64, wall_elapsed_ms: f64) -> Result<SeatFrame, InputError> {
    let mut router = router.borrow_mut();
    let seat_input = router.seat_mut(seat).ok_or(InputError::InactiveSeat)?;
    Ok(seat_input.sample(now, wall_elapsed_ms)?)
}

/// Whether the sample holds a use holdable.
fn sample_use_holdable(sample: &SeatSample) -> bool {
    sample.focus_is_game
        && sample.buttons.iter().any(|button| {
            matches!(
                button.action,
                SourceAction::Action(InputAction::Use) | SourceAction::Button(2)
            ) && (button.active || button.pressed)
        })
}

/// Seat publication mode (donor `"replace" | "retain"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeatPublication {
    /// Replace the client seats.
    Replace,
    /// Retain other client seats.
    Retain,
}

/// Prepared local seats awaiting publication (donor `prepareLocalSeats` result).
pub struct PreparedLocalSeats {
    /// Prepared seat ids in admission order.
    seats: Vec<SeatId>,
    /// Local seat ids before preparation, in local order.
    before: Vec<SeatId>,
    /// Loaded settings and archives in admission order.
    loaded: LoadedSettings,
}

impl PreparedLocalSeats {
    /// Discard the preparation (donor `discard`).
    pub fn discard(self) {}
}

impl ApplicationInput {
    /// Retire removed local seats (donor `retireLocalSeats`).
    pub fn retire_local_seats(
        &mut self,
        seats: &[SessionSeat],
        client: Option<&mut dyn InputClient>,
    ) -> Result<(), InputError> {
        let ids: Vec<SeatId> = seats.iter().map(|seat| seat.id().clone()).collect();
        // Donor identity folds to seat-id equality.
        let doomed: Vec<SeatId> = self
            .locals
            .iter()
            .filter(|local| ids.contains(local.seat_id()))
            .map(|local| local.seat_id().clone())
            .collect();
        if doomed.len() != seats.len() {
            return Err(InputError::RetireMismatch);
        }
        if doomed.is_empty() {
            return Ok(());
        }
        let keyboard = self.router.borrow().keyboard_seat();
        let mut failures: Vec<String> = Vec::new();
        self.retire_commands();
        let mut next = Vec::with_capacity(self.locals.len() - doomed.len());
        let mut removed: Vec<(SeatId, ClientId)> = Vec::new();
        for local in std::mem::take(&mut self.locals) {
            if doomed.contains(local.seat_id()) {
                let id = local.seat_id().clone();
                self.seat_ui.borrow_mut().remove(&id);
                self.mouse.remove(&id);
                self.seat_cvars.remove(&id);
                self.shadows.remove(&id);
                self.q3_selections.remove(&id);
                self.arsenals.remove(&id);
                self.offhand.borrow_mut().remove(&id);
                self.ui_ids.remove(&id);
                local.haptics.borrow_mut().close();
                removed.push((id, local.player.seat.client_id().clone()));
            } else {
                next.push(local);
            }
        }
        self.locals = next;
        let active: Vec<SeatId> = self.locals.iter().map(|local| local.seat_id().clone()).collect();
        {
            let mut registry = self.handle.registry();
            self.client_commands
                .borrow_mut()
                .publish_seats(&mut registry, active.clone());
        }
        let keyboard = match keyboard {
            Some(id) if active.contains(&id) => Some(id),
            _ => active.first().cloned(),
        };
        if let Err(error) = self.router.borrow_mut().retain_seats(&active, keyboard) {
            failures.push(error.to_string());
        }
        self.settings_ctrl.publish_seats(active.clone());
        for (_, client_id) in &removed {
            self.commands.borrow_mut().discard_client(client_id);
            self.staged.retain(|staged| {
                let source = match staged {
                    StagedCommand::Console { source, .. } | StagedCommand::Reliable { source, .. } => source,
                };
                !matches!(
                    root_origin(&source.origin),
                    CommandOrigin::LocalSeat { client, .. }
                    | CommandOrigin::RemoteClient { client }
                    if client == client_id
                )
            });
        }
        if let Some(client) = client {
            client.locals_mut().retain(|local| !ids.contains(local.seat.id()));
            for seat in seats {
                client.consoles_mut().remove(seat.id());
            }
            let prepared = client.prepared_cell();
            let activated = prepared.borrow_mut().set_active_seats(active);
            if let Err(error) = activated {
                failures.push(error.to_string());
            }
        } else if let Some(startup) = self.startup.as_ref() {
            if let Err(error) = startup.borrow_mut().set_active_seats(active) {
                failures.push(error.to_string());
            }
        }
        if let Err(error) = self.activate_commands(true) {
            failures.push(error.to_string());
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(InputError::RetireFailed(failures.join("; ")))
        }
    }

    /// Load fallible profile resources before admission (donor `prepareLocalSeats`).
    pub fn prepare_local_seats(&mut self, seats: &[SessionSeat]) -> Result<PreparedLocalSeats, InputError> {
        if seats.is_empty() || seats.len() > 4 {
            return Err(InputError::BadSeatCount);
        }
        self.settings_ctrl.settle();
        let before: Vec<SeatId> = self.locals.iter().map(|local| local.seat_id().clone()).collect();
        let ids: Vec<SeatId> = seats.iter().map(|seat| seat.id().clone()).collect();
        let source_dialect = self
            .actions
            .borrow()
            .console()
            .map(InputConsole::dialect)
            .unwrap_or_else(|| self.console_cvars.borrow().dialect());
        let loaded = read_settings(
            &ids,
            self.dialect,
            source_dialect,
            &self.session,
            &self.settings,
            None,
            Some(self),
            self.startup.as_ref(),
        )?;
        Ok(PreparedLocalSeats {
            seats: ids,
            before,
            loaded,
        })
    }

    /// Publish prepared local seats (donor `prepareLocalSeats` publish).
    pub fn publish_prepared_seats(
        &mut self,
        prepared: PreparedLocalSeats,
        players: &[LocalPlayer],
        client: Option<&mut dyn InputClient>,
        simulation: Option<Rc<RefCell<dyn InputPlayerView>>>,
        retention: SeatPublication,
    ) -> Result<(), InputError> {
        if prepared.before.len() != self.locals.len()
            || self
                .locals
                .iter()
                .enumerate()
                .any(|(index, local)| prepared.before.get(index) != Some(local.seat_id()))
        {
            return Err(InputError::StalePreparation);
        }
        if players.len() != prepared.seats.len()
            || players
                .iter()
                .enumerate()
                .any(|(index, player)| prepared.seats.get(index) != Some(player.seat_id()))
        {
            return Err(InputError::AdmissionChanged);
        }
        if let Some(simulation) = simulation {
            self.simulation = simulation;
        }
        let keyboard = self.router.borrow().keyboard_seat();
        let mut selections = Vec::with_capacity(players.len());
        for (index, player) in players.iter().enumerate() {
            let seat = player.seat_id();
            if prepared.before.contains(seat) {
                selections.push(
                    self.router
                        .borrow()
                        .controller_selection(seat)
                        .unwrap_or(ControllerSelection::Automatic),
                );
            } else {
                selections.push(
                    prepared
                        .loaded
                        .saved
                        .get(index)
                        .and_then(|saved| saved.as_ref())
                        .map(|saved| live_controller_selection(&saved.controller))
                        .unwrap_or(ControllerSelection::Automatic),
                );
            }
        }
        self.release_for_profile_change(None);
        self.retire_commands();
        let mut next = Vec::with_capacity(players.len());
        let mut routes = Vec::with_capacity(players.len());
        for (index, player) in players.iter().enumerate() {
            let seat = player.seat_id().clone();
            if let Some(position) = self.locals.iter().position(|local| local.seat_id() == &seat) {
                let mut local = self.locals.remove(position);
                if local.player.actor != player.actor {
                    local.player.actor = player.actor.clone();
                    if dialect_family(self.dialect) != ClientFamily::Q3 {
                        let view = self.simulation.borrow().player_view(&player.actor);
                        local.builder.set_view_angles(view.angles)?;
                    }
                }
                let (seat_input, _) = self
                    .router
                    .borrow_mut()
                    .remove_seat(&seat)
                    .ok_or(InputError::InactiveSeat)?;
                routes.push(SeatRoute {
                    seat: seat_input,
                    controller: selections[index].clone(),
                });
                next.push(local);
            } else {
                let saved = prepared.loaded.saved.get(index).and_then(|saved| saved.as_ref());
                let archive = prepared.loaded.input.get(index).map(Vec::as_slice).unwrap_or(&[]);
                let (local, seat_input) = self.create_joined_local(player, saved, archive)?;
                routes.push(SeatRoute {
                    seat: seat_input,
                    controller: selections[index].clone(),
                });
                next.push(local);
            }
        }
        for local in std::mem::take(&mut self.locals) {
            let id = local.seat_id().clone();
            self.seat_ui.borrow_mut().remove(&id);
            self.mouse.remove(&id);
            self.seat_cvars.remove(&id);
            self.shadows.remove(&id);
            self.q3_selections.remove(&id);
            self.arsenals.remove(&id);
            self.offhand.borrow_mut().remove(&id);
            self.ui_ids.remove(&id);
            local.haptics.borrow_mut().close();
        }
        self.locals = next;
        let ids: Vec<SeatId> = self.locals.iter().map(|local| local.seat_id().clone()).collect();
        {
            let mut registry = self.handle.registry();
            self.client_commands
                .borrow_mut()
                .publish_seats(&mut registry, ids.clone());
        }
        self.handle.refresh_contexts(&self.locals);
        *self.print_router.consoles.borrow_mut() = self
            .locals
            .iter()
            .map(|local| (local.seat_id().clone(), Rc::clone(&local.console)))
            .collect();
        let keyboard = match keyboard {
            Some(id) if ids.contains(&id) => Some(id),
            _ => ids.first().cloned(),
        };
        self.router.borrow_mut().publish_seats(routes, keyboard)?;
        self.settings_ctrl.publish_seats(ids);
        self.wire_console_cells()?;
        if let Some(client) = client {
            self.publish_client_seats(client, retention)?;
        } else if self.startup.is_some() {
            let seats = self.published_seats()?;
            let ids: Vec<SeatId> = seats.iter().map(|seat| seat.id.clone()).collect();
            if let Some(startup) = self.startup.as_ref() {
                startup.borrow_mut().publish_seats(seats, ids, retention)?;
            }
        }
        self.adopt_startup()?;
        self.activate_commands(true)?;
        Ok(())
    }

    /// Build a joined local plus its seat (donor `createJoinedLocal`).
    fn create_joined_local(
        &mut self,
        player: &LocalPlayer,
        saved: Option<&SeatSettings>,
        archive: &[CvarArchiveEntry],
    ) -> Result<(LocalInput, Seat), InputError> {
        let seat = player.seat_id().clone();
        let client = player.seat.client_id().clone();
        let source_dialect = self
            .actions
            .borrow()
            .console()
            .map(InputConsole::dialect)
            .unwrap_or_else(|| self.console_cvars.borrow().dialect());
        let context = seat_context(&self.session, &seat, &client);
        let prepared = self
            .startup
            .as_ref()
            .and_then(|startup| startup.borrow().seats().into_iter().find(|view| view.id == seat));
        let mouse = match prepared.as_ref() {
            Some(view) => {
                let mut registry = seed_registry(view.mouse.dialect, &view.mouse.session);
                apply_state(&mut registry, &view.mouse.state)?;
                shared_registry(registry)
            }
            None => {
                let mut registry = seed_registry(source_dialect, &self.session);
                register_mouse_settings(&mut registry)?;
                registry.apply_archive(archive)?;
                shared_registry(registry)
            }
        };
        self.mouse.insert(seat.clone(), Rc::clone(&mouse));
        let shadow = Rc::new(RefCell::new(qa_client::input::BindingTable::new()));
        self.shadows.insert(seat.clone(), Rc::clone(&shadow));
        let mut seat_input = Seat::new(seat.clone(), self.dialect, Box::new(|_, _| false));
        let mut builder = InputCommandBuilder::new(dialect_family(self.dialect));
        let view = self.simulation.borrow().player_view(&player.actor);
        builder.set_view_angles(view.angles)?;
        let defaults = default_bindings(0, self.dialect, &self.actions.borrow().binding_items(&seat));
        match prepared.as_ref() {
            Some(view) => {
                for binding in view.bindings.borrow().bindings() {
                    seat_input.bind((*binding).clone());
                    shadow.borrow_mut().bind((*binding).clone());
                }
            }
            None => {
                let bindings = saved.map(|saved| saved.bindings.clone()).unwrap_or(defaults);
                for binding in bindings {
                    seat_input.bind(binding.clone());
                    shadow.borrow_mut().bind(binding);
                }
            }
        }
        if let Some(saved) = saved {
            if prepared.is_none() {
                seat_input.gamepad.tuning = client_gamepad_tuning(&saved.gamepad);
                builder.mouse.tuning = client_mouse_tuning(&saved.mouse);
                if let Some(always_run) = saved.always_run {
                    let mut tuning = builder.tuning();
                    tuning.always_run = always_run;
                    builder.set_tuning(tuning);
                }
            }
        }
        let haptics = Rc::new(RefCell::new(SeatHaptics::new(
            seat.clone(),
            Box::new({
                let router = Rc::clone(&self.router);
                move |seat: &SeatId| router.borrow().controller_for(seat)
            }),
            Box::new({
                let loader = Rc::clone(&self.haptic_loader);
                move |content: &str, sound: &str| {
                    let content = ContentId::new(content);
                    let request = ResourceRequest {
                        content,
                        path: sound.to_string(),
                    };
                    Rc::clone(&loader.borrow())(&request)
                }
            }),
            Box::new({
                let now = Rc::clone(&self.now);
                move || now()
            }),
            Box::new({
                let controllers = Rc::clone(&self.controllers);
                move |instance, low, high, duration| {
                    controllers
                        .borrow()
                        .rumble(instance, low, high, duration)
                        .unwrap_or(ControllerOperationResult::Accepted)
                }
            }),
        )));
        if let Some(saved) = saved {
            haptics.borrow_mut().set_enabled(saved.rumble);
            haptics.borrow_mut().set_strength(saved.rumble_strength)?;
        }
        let console = Rc::new(RefCell::new(SeatConsole::new(source_dialect, true)));
        if let Some(saved) = saved {
            console.borrow_mut().history.replace(&saved.history);
        }
        Ok((
            LocalInput {
                player: LocalPlayer {
                    seat: SessionSeat::new(seat.clone(), client.clone()),
                    actor: player.actor.clone(),
                },
                console,
                builder,
                haptics,
                context,
            },
            seat_input,
        ))
    }

    /// Published seats for a startup (donor seat assembly).
    fn published_seats(&self) -> Result<Vec<InputPublishedSeat>, InputError> {
        let mut seats = Vec::with_capacity(self.locals.len());
        for local in &self.locals {
            let seat = local.seat_id().clone();
            let mouse = self.mouse.get(&seat).ok_or(InputError::SeatOwners)?;
            let cvars = self.seat_cvars.get(&seat).ok_or(InputError::SeatOwners)?;
            let bindings = self
                .shadows
                .get(&seat)
                .cloned()
                .unwrap_or_else(|| Rc::new(RefCell::new(qa_client::input::BindingTable::new())));
            seats.push(InputPublishedSeat {
                id: seat.clone(),
                device: InputSeatDevice {
                    seat: seat.clone(),
                    context: local.context.clone(),
                    dialect: self.dialect,
                    router: Rc::clone(&self.router),
                    contexts: Rc::clone(&self.handle.contexts),
                },
                context: local.context.clone(),
                cvars: owned_cell(cvars, &self.session)?.0,
                mouse: owned_cell(mouse, &self.session)?.0,
                bindings,
            });
        }
        Ok(seats)
    }

    /// Publish seats into a client (donor `publishClientSeats`).
    pub fn publish_client_seats(
        &mut self,
        client: &mut dyn InputClient,
        retention: SeatPublication,
    ) -> Result<(), InputError> {
        let consoles = client.consoles_mut();
        for local in &mut self.locals {
            let seat = local.seat_id().clone();
            let focus = self
                .router
                .borrow()
                .seat(&seat)
                .map(|seat| seat_focus_to_console(seat.focus()))
                .unwrap_or(ConsoleFocus::Game);
            match consoles.get(&seat) {
                Some(retained) if !Rc::ptr_eq(retained, &local.console) => {
                    retained.borrow_mut().adopt(&mut local.console.borrow_mut(), focus)?;
                    local.console = Rc::clone(retained);
                }
                None => {
                    local.console.borrow_mut().publish(focus);
                    consoles.insert(seat, Rc::clone(&local.console));
                }
                _ => {}
            }
        }
        let seats = self.published_seats().map_err(|_| InputError::PublishedOwners)?;
        let ids: Vec<SeatId> = seats.iter().map(|seat| seat.id.clone()).collect();
        let prepared = client.prepared_cell();
        prepared.borrow_mut().publish_seats(seats, ids.clone(), retention)?;
        let choices = self
            .actions
            .borrow()
            .configuration()
            .map(|configuration| configuration.binding_choices())
            .unwrap_or_default();
        for choice in &choices {
            prepared
                .borrow_mut()
                .apply_binding_choices(&choice.id, choice)
                .map_err(|_| InputError::PublishedConfiguration)?;
        }
        let mut locals = Vec::with_capacity(self.locals.len());
        for local in &self.locals {
            let prepared = prepared.clone();
            let found = prepared.borrow().seats().iter().any(|view| view.id == *local.seat_id());
            if !found {
                return Err(InputError::PublishedSeat);
            }
            locals.push(ClientLocal {
                client: local.player.seat.client_id().clone(),
                seat: SessionSeat::new(local.seat_id().clone(), local.player.seat.client_id().clone()),
                prepared: ClientLocalPrepared {
                    id: local.seat_id().clone(),
                    startup: prepared,
                },
            });
        }
        match retention {
            SeatPublication::Replace => {
                client.locals_mut().clear();
                client.locals_mut().extend(locals);
                client
                    .consoles_mut()
                    .retain(|seat, _| self.locals.iter().any(|local| local.seat_id() == seat));
            }
            SeatPublication::Retain => {
                for local in locals {
                    let position = client
                        .locals_mut()
                        .iter()
                        .position(|previous| previous.seat.id() == local.seat.id() && previous.client == local.client);
                    match position {
                        Some(index) => client.locals_mut()[index] = local,
                        None => return Err(InputError::RetainedReplace),
                    }
                }
            }
        }
        Ok(())
    }

    /// Publish this platform into a client (donor `publishClientPlatform`).
    ///
    /// The input moves into the client platform, matching the donor
    /// assignment.
    pub fn publish_client_platform(
        mut self,
        client: &mut dyn InputClient,
        retention: SeatPublication,
    ) -> Result<(), InputError> {
        // Donor buffer identity folds to shared-owner identity: the input
        // shares its controllers and startup cells with its client.
        let same_controllers = Rc::ptr_eq(&self.controllers, &client.controllers_cell());
        let same_prepared = self
            .startup
            .as_ref()
            .is_some_and(|startup| Rc::ptr_eq(startup, &client.prepared_cell()));
        if !same_controllers || !same_prepared {
            return Err(InputError::RetainedOwners);
        }
        self.validate_startup_adoption()?;
        self.validate_candidate_commands()?;
        let platform = client.platform();
        let previous = platform.borrow_mut().take();
        if let Some(InputClientPlatform::World { mut input }) = previous {
            self.release_into_candidate(&mut input, true)?;
            self.publish_candidate_commands()?;
            self.publish_client_seats(client, retention)?;
            input.transfer_platform_to(&mut self)?;
            *platform.borrow_mut() = Some(InputClientPlatform::World { input: Box::new(self) });
            return Ok(());
        }
        if let Some(candidate) = self.candidate.as_mut() {
            let now = (self.now)();
            let seats: Vec<SeatId> = self.locals.iter().map(|local| local.seat_id().clone()).collect();
            let buffer = &mut candidate.buffer;
            let mut registry = ProgramRegistry { buffer };
            let mut router = self.router.borrow_mut();
            for seat in &seats {
                if let Some(seat_input) = router.seat_mut(seat) {
                    seat_input.release(now, &mut registry);
                }
            }
        }
        self.publish_candidate_commands()?;
        self.publish_client_seats(client, retention)?;
        if previous.is_none() {
            self.router.borrow_mut().attach_window(Box::new(WindowAdapter {
                window: Rc::clone(&self.window),
            }))?;
        }
        // A menu previous retires its commands; a missing previous attached
        // above. The world case returned early.
        if let Some(InputClientPlatform::Menu { retire_commands, .. }) = previous {
            retire_commands();
        }
        self.adopt_startup()?;
        self.activate_commands(false)?;
        self.router.borrow_mut().update_capture()?;
        self.owns_controllers = false;
        *platform.borrow_mut() = Some(InputClientPlatform::World { input: Box::new(self) });
        Ok(())
    }

    /// Move platform state into a successor (donor `transferPlatformTo`).
    pub fn transfer_platform_to(&mut self, next: &mut ApplicationInput) -> Result<(), InputError> {
        next.pending_window = std::mem::take(&mut self.pending_window);
        next.pending_controller = std::mem::take(&mut self.pending_controller);
        self.retire_commands();
        next.adopt_startup()?;
        self.router
            .borrow_mut()
            .transfer_window_to(&mut next.router.borrow_mut());
        next.activate_commands(false)?;
        next.router.borrow_mut().update_capture()?;
        next.owns_controllers = self.owns_controllers;
        self.owns_controllers = false;
        next.owns_devices = self.owns_devices;
        self.owns_devices = false;
        Ok(())
    }

    /// Move platform state into a frontend router (donor `transferPlatformToFrontend`).
    pub fn transfer_platform_to_frontend(
        &mut self,
        next: &mut InputRouter,
        append: Option<ReleaseSink>,
    ) -> Result<(), InputError> {
        self.release_for_profile_change(append);
        self.retire_commands();
        self.router.borrow_mut().transfer_window_to(next);
        for event in std::mem::take(&mut self.pending_window) {
            next.handle_platform(event)?;
        }
        for event in std::mem::take(&mut self.pending_controller) {
            next.handle_controller(event)?;
        }
        self.owns_controllers = false;
        Ok(())
    }

    /// Activate a prepared platform (donor `activatePreparedPlatform`).
    pub fn activate_prepared_platform(&mut self) -> Result<(), InputError> {
        self.router.borrow_mut().attach_window(Box::new(WindowAdapter {
            window: Rc::clone(&self.window),
        }))?;
        self.activate_commands(false)?;
        self.adopt_startup()?;
        Ok(())
    }
}

/// Live stick curve as saved tuning.
fn saved_stick_curve(value: &ClientStickCurve) -> SavedStickCurve {
    match *value {
        ClientStickCurve::Radial {
            deadzone,
            outer_threshold,
            exponent,
        } => SavedStickCurve::Radial {
            deadzone,
            exponent,
            outer_threshold,
        },
        ClientStickCurve::Axial { deadzone, exponent } => SavedStickCurve::Axial { deadzone, exponent },
    }
}

/// Live gamepad tuning as saved tuning.
fn saved_gamepad_tuning(value: &GamepadTuning) -> SavedGamepadTuning {
    SavedGamepadTuning {
        move_curve: saved_stick_curve(&value.stick_move),
        look_curve: saved_stick_curve(&value.stick_look),
        swap_sticks: value.swap_sticks,
        yaw_degrees_per_second: value.yaw_degrees_per_second,
        pitch_degrees_per_second: value.pitch_degrees_per_second,
        invert_pitch: value.invert_pitch,
        forward_sensitivity: value.forward_sensitivity,
        side_sensitivity: value.side_sensitivity,
        trigger_threshold: value.trigger_threshold,
        gyro: crate::settings::config::GyroTuning {
            enabled: value.gyro.enabled,
            yaw_axis: if value.gyro.yaw_axis_y {
                GyroYawAxis::Y
            } else {
                GyroYawAxis::Z
            },
            yaw_sensitivity: value.gyro.yaw_sensitivity,
            pitch_sensitivity: value.gyro.pitch_sensitivity,
        },
    }
}

/// Live mouse tuning as saved tuning.
fn saved_mouse_tuning(value: &MouseTuning) -> SavedMouseTuning {
    SavedMouseTuning {
        sensitivity: value.sensitivity,
        acceleration: value.acceleration,
        filter: value.filter,
        yaw: value.yaw,
        pitch: value.pitch,
        side: value.side,
        forward: value.forward,
        look_spring: value.look_spring,
        look_strafe: value.look_strafe,
        free_look: value.free_look,
        invert_pitch: value.invert_pitch,
    }
}

/// Controller binding with its device normalized to the first device.
fn normalized_device_binding(binding: &InputBinding) -> InputBinding {
    let input = match &binding.input {
        PhysicalInput::ControllerButton { button, .. } => PhysicalInput::ControllerButton {
            device: 0,
            button: *button,
        },
        PhysicalInput::ControllerAxis { axis, direction, .. } => PhysicalInput::ControllerAxis {
            device: 0,
            axis: *axis,
            direction: *direction,
        },
        other => other.clone(),
    };
    InputBinding {
        input,
        target: binding.target.clone(),
    }
}

/// Player rebind mode (donor `"world" | "source-round"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RebindMode {
    /// World travel.
    World,
    /// Source-round travel.
    SourceRound,
}

impl ApplicationInput {
    /// Rebind players after travel (donor `rebindPlayers`).
    pub fn rebind_players(
        &mut self,
        players: &[LocalPlayer],
        simulation: Rc<RefCell<dyn InputPlayerView>>,
        mode: RebindMode,
    ) -> Result<(), InputError> {
        self.simulation = simulation;
        if mode == RebindMode::World {
            self.release_offhand(true);
        }
        if players.len() != self.locals.len() {
            return Err(InputError::TravelSeats);
        }
        for local in &mut self.locals {
            let seat = local.seat_id().clone();
            let player = players
                .iter()
                .find(|player| player.seat_id() == &seat)
                .ok_or(InputError::TravelPlayer)?;
            if let Some(ui) = self.seat_ui.borrow().get(&seat).cloned() {
                ui.borrow_mut().clear_prompt();
            }
            local.haptics.borrow_mut().invalidate_assets();
            if mode == RebindMode::World {
                let now = (self.now)();
                let mut registry = self.handle.registry();
                if let Some(seat_input) = self.router.borrow_mut().seat_mut(&seat) {
                    seat_input.release(now, &mut registry);
                }
            }
            local.player.actor = player.actor.clone();
            if mode == RebindMode::World || dialect_family(self.dialect) != ClientFamily::Q3 {
                let view = self.simulation.borrow().player_view(&player.actor);
                local.builder.set_view_angles(view.angles)?;
            }
        }
        self.q3_selections.clear();
        self.arsenals.clear();
        Ok(())
    }

    /// Persist settings and archives (donor `saveSettings`).
    pub fn save_settings(&mut self) -> Result<(), InputError> {
        if self.startup.as_ref().is_some_and(|startup| startup.borrow().pending()) {
            return Ok(());
        }
        self.devices.borrow_mut().save()?;
        self.settings_ctrl.settle();
        for local in &self.locals {
            let seat = local.seat_id().clone();
            let router = self.router.borrow();
            let seat_input = router.seat(&seat).ok_or(InputError::InactiveSeat)?;
            let bindings: Vec<InputBinding> = seat_input
                .bindings()
                .into_iter()
                .map(normalized_device_binding)
                .collect();
            let gamepad = saved_gamepad_tuning(&seat_input.gamepad.tuning);
            let selection = router
                .controller_selection(&seat)
                .map(|selection| saved_controller_selection(&selection))
                .unwrap_or(SavedControllerSelection::Automatic);
            drop(router);
            let settings = SeatSettings {
                version: 1,
                bindings,
                gamepad,
                mouse: saved_mouse_tuning(&local.builder.mouse.tuning),
                always_run: Some(local.builder.tuning().always_run),
                history: local.console.borrow().history.lines(),
                rumble: local.haptics.borrow().enabled(),
                rumble_strength: local.haptics.borrow().strength(),
                controller: selection,
            };
            self.settings.save_seat(&seat_settings_path(seat.index()), &settings)?;
            self.settings_ctrl.save(&seat);
        }
        let keyboard = self.router.borrow().keyboard_seat().map(|seat| seat.index());
        self.settings.save_input_routing("input/routing.json", keyboard)?;
        if self.archive_persistence && !self.startup.as_ref().is_some_and(|startup| startup.borrow().pending()) {
            let movement_owner =
                CvarArchiveOwner::new(CvarArchiveHead::Movement, vec![dialect_name(self.dialect).to_string()]);
            save_cvar_archive(&self.settings, &movement_owner, &self.cvars.borrow(), None)?;
            if !Rc::ptr_eq(&self.console_cvars, &self.cvars) {
                let fallback_owner = CvarArchiveOwner::new(
                    CvarArchiveHead::Fallback,
                    vec![dialect_name(self.console_cvars.borrow().dialect()).to_string()],
                );
                save_cvar_archive(&self.settings, &fallback_owner, &self.console_cvars.borrow(), None)?;
            }
            for (seat, mouse) in &self.mouse {
                let mouse_ref = mouse.borrow();
                let owner = CvarArchiveOwner::new(
                    CvarArchiveHead::Input,
                    vec![dialect_name(mouse_ref.dialect()).to_string(), seat.index().to_string()],
                );
                save_cvar_archive(&self.settings, &owner, &mouse_ref, None)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;

    fn owner() -> IdentityOwner {
        IdentityOwner::create("input-tests").expect("owner")
    }

    fn test_context(owner: &IdentityOwner) -> CommandContext {
        CommandContext::new(
            owner.session().clone(),
            CommandOrigin::LocalSeat {
                seat: owner.seat(0),
                client: owner.client(0, 0),
            },
        )
    }

    fn test_router(seat: SeatId) -> RouterCell {
        let route = SeatRoute {
            seat: Seat::new(seat.clone(), Dialect::Q3, Box::new(|_, _| false)),
            controller: ControllerSelection::Automatic,
        };
        Rc::new(RefCell::new(
            InputRouter::new(
                vec![route],
                Some(seat),
                None,
                Box::new(NullRegistry),
                Box::new(|| 0.0),
                Box::new(|| 0),
                false,
                Box::new(|_| {}),
                false,
                None,
            )
            .expect("router"),
        ))
    }

    #[test]
    fn tuning_conversions_round_trip() {
        let gamepad = GamepadTuning {
            stick_move: ClientStickCurve::Radial {
                deadzone: 0.2,
                outer_threshold: 0.03,
                exponent: 2.5,
            },
            stick_look: ClientStickCurve::Axial {
                deadzone: 0.1,
                exponent: 1.5,
            },
            swap_sticks: true,
            yaw_degrees_per_second: 240.0,
            pitch_degrees_per_second: 130.0,
            invert_pitch: true,
            forward_sensitivity: 1.2,
            side_sensitivity: 0.8,
            trigger_threshold: 0.4,
            gyro: qa_client::input::gamepad::GyroTuning {
                enabled: true,
                yaw_sensitivity: 2.0,
                pitch_sensitivity: 1.5,
                yaw_axis_y: false,
            },
        };
        let saved = saved_gamepad_tuning(&gamepad);
        assert_eq!(client_gamepad_tuning(&saved), gamepad);

        let mouse = MouseTuning {
            sensitivity: 3.0,
            acceleration: 0.5,
            filter: true,
            yaw: 0.022,
            pitch: 0.022,
            side: 0.8,
            forward: 1.0,
            free_look: true,
            look_spring: false,
            look_strafe: true,
            invert_pitch: true,
        };
        let saved_mouse = saved_mouse_tuning(&mouse);
        assert_eq!(client_mouse_tuning(&saved_mouse), mouse);

        for selection in [
            ControllerSelection::Automatic,
            ControllerSelection::None,
            ControllerSelection::Device {
                guid: "0".repeat(32),
                ordinal: 2,
            },
            ControllerSelection::Serial {
                guid: "1".repeat(32),
                serial: "abc".to_string(),
            },
        ] {
            assert_eq!(
                live_controller_selection(&saved_controller_selection(&selection)),
                selection
            );
        }
    }

    #[test]
    fn user_command_maps_built_commands() {
        let angles = Vec3 { x: 1.0, y: 2.0, z: 3.0 };
        match user_command(&BuiltCommand::Q1Netquake {
            ack_time_s: 4.0,
            view_angles: angles,
            forward: 10,
            side: -5,
            up: 0,
            buttons: 3,
            impulse: 7,
        }) {
            UserCommand::Q1Netquake {
                acknowledged_server_time_seconds,
                forward_move,
                impulse,
                ..
            } => {
                assert_eq!(acknowledged_server_time_seconds, 4.0);
                assert_eq!(forward_move, 10.0);
                assert_eq!(impulse, 7.0);
            }
            other => panic!("wrong variant: {other:?}"),
        }
        match user_command(&BuiltCommand::Q3 {
            server_time_ms: 100,
            angle_words: [1, 2, 3],
            forward: 4,
            right: 5,
            up: 6,
            buttons: 7,
            weapon: 8,
        }) {
            UserCommand::Q3 {
                server_time_milliseconds,
                weapon,
                right_move,
                ..
            } => {
                assert_eq!(server_time_milliseconds, 100.0);
                assert_eq!(weapon, 8.0);
                assert_eq!(right_move, 5.0);
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn normalized_device_binding_zeroes_device() {
        let button = InputBinding {
            input: PhysicalInput::ControllerButton { device: 3, button: 1 },
            target: InputBindingTarget::Command("+attack".to_string()),
        };
        match normalized_device_binding(&button).input {
            PhysicalInput::ControllerButton { device, button } => {
                assert_eq!((device, button), (0, 1));
            }
            other => panic!("wrong input: {other:?}"),
        }
        let key = InputBinding {
            input: PhysicalInput::Key(42),
            target: InputBindingTarget::Command("+forward".to_string()),
        };
        assert_eq!(normalized_device_binding(&key), key);
    }

    #[test]
    fn staged_commands_validate_and_publish() {
        let owner = owner();
        let seat = owner.seat(0);
        let context = test_context(&owner);
        let live = vec![InputBinding {
            input: PhysicalInput::Key(65),
            target: InputBindingTarget::Command("+attack".to_string()),
        }];
        let buffer = CommandBuffer::new(Dialect::Q3, context.clone(), BufferOptions::default()).expect("buffer");
        let revision = buffer.program_revision();
        let mut staged = StagedClientCommands::stage(StagedProgram {
            buffer,
            live_revision: revision,
            live: vec![(seat.clone(), live.clone())],
            console_live: None,
            release_dialect: Dialect::Q3,
            binding_print: Rc::new(|_| {}),
        })
        .expect("stage");
        staged.validate_publication(revision, &|_| live.clone()).expect("valid");
        assert!(staged.validate_publication(revision + 1, &|_| live.clone()).is_err());
        assert!(staged.validate_publication(revision, &|_| Vec::new()).is_err());

        let table = staged.staged_table(&seat).expect("table");
        table.borrow_mut().bind(InputBinding {
            input: PhysicalInput::Key(66),
            target: InputBindingTarget::Command("+jump".to_string()),
        });
        let mut published: Vec<(SeatId, Vec<InputBinding>)> = Vec::new();
        staged
            .publish_bindings(&mut |seat, bindings| published.push((seat.clone(), bindings)))
            .expect("publish");
        assert_eq!(published.len(), 1);
        assert!(published[0]
            .1
            .iter()
            .any(|binding| binding.input == PhysicalInput::Key(66)));
    }

    #[test]
    fn guest_seat_input_drives_live_bindings() {
        let owner = owner();
        let seat = owner.seat(0);
        let router = test_router(seat.clone());
        let contexts = Rc::new(RefCell::new(HashMap::new()));
        let mut guest = GuestSeatInput {
            router,
            seat: seat.clone(),
            staged: None,
            cleared: None,
            contexts,
            context: test_context(&owner),
            now: Rc::new(|| 100.0),
        };
        guest.bind(PhysicalInput::Key(65), "+attack");
        assert_eq!(
            guest.binding(&PhysicalInput::Key(65)),
            Some(InputBindingTarget::Command("+attack".to_string()))
        );
        assert_eq!(guest.bindings().len(), 1);
        assert!(!guest.is_down(&PhysicalInput::Key(65)));
        guest.unbind(&PhysicalInput::Key(65));
        assert_eq!(guest.binding(&PhysicalInput::Key(65)), None);
        guest.bind(PhysicalInput::Key(65), "+attack");
        guest.unbind_all();
        assert!(guest.bindings().is_empty());
        guest.clear_states();
    }

    #[test]
    fn adopted_routing_resolves_movement_and_fallback() {
        let owner = owner();
        let session = owner.session().clone();
        let context = test_context(&owner);
        let mut movement = CvarRegistry::with_session(Dialect::Q3, Some(session.clone()));
        movement.set("sensitivity", "5", false).expect("set");
        let fallback = CvarRegistry::with_session(Dialect::Q3, Some(session.clone()));
        let routing = AdoptedRouting {
            session: session.clone(),
            source_dialect: Dialect::Q3,
            fallback: shared_registry(fallback),
            movement: Some(shared_registry(movement)),
            shared: None,
            server: None,
            seats: Vec::new(),
            mice: Vec::new(),
        };
        let seats: Vec<SeatRegistries> = Vec::new();
        let resolved = routing.owner_slot("sensitivity", &context, &seats).expect("owner");
        assert_eq!(resolved, RegistrySlot::Movement);
        let visible = routing.visible_slots(&context, &seats);
        assert!(visible.contains(&RegistrySlot::Movement));
        assert!(visible.contains(&RegistrySlot::Fallback));
    }

    struct RecordUi {
        cycled: Vec<i32>,
        switched: Vec<(i32, i32)>,
    }

    impl ApplicationInputUi for RecordUi {
        fn input(&mut self, _event: &SeatInputEvent, _focus: &SeatInputFocus) -> bool {
            false
        }
        fn cycle_weapon(&mut self, direction: i32) -> bool {
            self.cycled.push(direction);
            true
        }
        fn switch_weapon(&mut self, first: i32, second: i32) -> bool {
            self.switched.push((first, second));
            true
        }
        fn wheel(&mut self, _mode: WheelMode, _down: bool) {}
        fn close_menus(&mut self) {}
    }

    struct RecordActions {
        executed: Vec<String>,
    }

    impl ApplicationInputCommands for RecordActions {
        fn execute(&mut self, name: &str, _args: &[String], _seat: Option<&SeatId>, _source: &CommandContext) {
            self.executed.push(name.to_string());
        }
        fn quit(&mut self) {}
        fn print(&mut self, _text: &str) {}
    }

    #[test]
    fn ui_command_prefers_weapon_hooks() {
        let owner = owner();
        let seat = owner.seat(0);
        let context = test_context(&owner);
        let ui = Rc::new(RefCell::new(RecordUi {
            cycled: Vec::new(),
            switched: Vec::new(),
        }));
        let mut map = HashMap::new();
        let ui_cell: UiCell = ui.clone();
        map.insert(seat.clone(), ui_cell);
        let seat_ui = Rc::new(RefCell::new(map));
        let actions = Rc::new(RefCell::new(RecordActions { executed: Vec::new() }));
        let actions_cell: ActionsCell = actions.clone();
        execute_ui_command(&seat_ui, &actions_cell, "weapnext", &[], Some(&seat), &context);
        execute_ui_command(
            &seat_ui,
            &actions_cell,
            "switchweapon",
            &["1".to_string(), "2".to_string()],
            Some(&seat),
            &context,
        );
        execute_ui_command(&seat_ui, &actions_cell, "centerview", &[], Some(&seat), &context);
        let ui_ref = ui.borrow();
        assert_eq!(ui_ref.cycled, vec![1]);
        assert_eq!(ui_ref.switched, vec![(1, 2)]);
        assert_eq!(actions.borrow().executed, vec!["centerview".to_string()]);
    }
}
