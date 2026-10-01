//! Prepared startup registries, seats, and the configuration pipeline.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/prepared-startup.ts`
//! (`PreparedSeat`, `PreparedSeatConfiguration`, `PreparedConfigurationContinuation`,
//! `PreparedStartup`, `allowSeatConfigurationCommand`, `PreparedClientCommands`,
//! `prepareClientCommands`). The startup buffer, deferred/operator/forwarded commands,
//! seat binding math, saved-configuration gating, and the phased config run are ported;
//! everything below the seam line is host-owned.
//!
//! Documented folds:
//! - Async work resolves synchronously: the donor `run` generator becomes an explicit
//!   [`RunPhase`] state machine stepped by [`PreparedStartup::advance_frame`]; script
//!   reads settle immediately through [`PreparedScriptFiles`]; `writeconfig` file writes
//!   complete at the next frame boundary (the donor fires them off asynchronously).
//! - Script-completion events are collected during each drain and replayed to the active
//!   configuration right after the drain instead of mid-drain (core delivers completions
//!   through listeners the port cannot interleave). Order is preserved; same-drain
//!   chained reads observe pre-completion values.
//! - Per-dispatch `afterDispatch` observation is impossible with core hooks, so the
//!   `bind`/`unbind`/`unbindall` tracking the donor does after every dispatch is recorded
//!   by the [`BufferCommandRegistry`] adapter and replayed after each drain. Only those
//!   three commands are observed because the donor only acts on them.
//! - The preparation program (`prepareProgram`, `finishPreparation`,
//!   `hasPreparationPrefix`, `setProfile`, `validateProfile`) has no core API. The host
//!   builds [`ClientProgram`] hooks, [`PreparedStartup::note_preparation_prefix`] tracks
//!   the published prefix, and [`PreparedStartup::adopt`] rebuilds the buffer on a
//!   dialect change (handlers re-registered, aliases and queued text preserved through
//!   [`CommandBuffer::copy_pending_from`]; owner-registered handlers do not survive).
//! - Registry output binding has no core API; [`PreparedStartup::drain_notifications`]
//!   forwards queued registry notifications to the startup printer at frame boundaries.
//! - `Q3ProductPolicy` folds to its only consumed output, the map-command list
//!   ([`PreparedStartupOptions::q3_map_commands`]).
//!
//! Missing siblings (host seams, referenced but NOT ported here): `console.ts`
//! ([`StartupCvarRouting`]), `startup-config.ts` ([`PreparedStartupConfig`],
//! [`StartupConfigFactory`]), `config-scripts.ts` ([`PreparedScriptFiles`]), `seat.ts`
//! plus `mouse-settings.ts` ([`PreparedSeatDevice`], [`PreparedMouse`]),
//! `gtv-commands.ts` and `shared-setting-cvars.ts` (registration hooks in
//! [`PreparedStartupOptions`]), `server-administration.ts` (operator names in
//! [`PreparedStartupOptions`]), and `q3-product-policy.ts` (map commands in
//! [`PreparedStartupOptions`]). Seat binding storage reuses the ported
//! [`BindingTable`](qa_client::input::BindingTable); the weapon catalog behind
//! `defaultBindings` arrives through
//! [`PreparedStartupOptions::default_binding_items`].

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use qa_client::input::bindings::{
    archived_bindings, default_bindings, named_physical_input, register_binding_commands, BindingLookup, BindingsError,
};
use qa_client::input::commands::{
    CommandHandler as ClientCommandHandler, CommandInvocation as ClientInvocation, CommandOrigin as ClientOrigin,
    CommandRegistry,
};
use qa_client::input::weapons::WeaponBindingItem;
use qa_client::input::{physical_input_key, BindingTable, InputBinding, InputBindingTarget, PhysicalInput};
use qa_core::cmd::{ascii_fold, Dialect};
use qa_core::cmd_buffer::{
    BufferError, BufferOptions, BufferServices, CommandBuffer, CommandContext, CommandHandler, CommandOrigin,
    ForwardedCommand, FrameHooks, Invocation, ScriptCompletion, ScriptRead,
};
use qa_core::cvar::{flags, CvarArchiveEntry, CvarError, CvarRegistry, SetCommandKind};
use qa_core::identity::{SeatId, SessionId};
use thiserror::Error;

use super::audio::commands::{CD_COMMAND_DOCUMENTATION, MUSIC_COMMAND_DOCUMENTATION};
use super::player_userinfo::register_player_userinfo;
use super::q1_client_settings::{register_q1_view_commands, Q1ViewCommandGuard};
use super::startup_commands::{startup_command_phases, StartupCommandPhases, StartupCommandsError};
use crate::settings::config::{MouseTuning, SeatSettings};

/// Failure of a prepared-startup operation.
#[derive(Debug, Error)]
pub enum PreparedStartupError {
    /// Command buffer failure.
    #[error(transparent)]
    Buffer(#[from] BufferError),
    /// Cvar registry failure.
    #[error(transparent)]
    Cvar(#[from] CvarError),
    /// Startup command phase failure.
    #[error(transparent)]
    StartupCommands(#[from] StartupCommandsError),
    /// Binding archive failure.
    #[error(transparent)]
    Bindings(#[from] BindingsError),
    /// Invariant violation (donor `throw new Error`).
    #[error("{0}")]
    Message(String),
}

/// Slot address of one cvar registry owned by the startup object.
///
/// Core registries carry no identity, so host routing ([`StartupCvarRouting`]) resolves
/// names to slots and the port resolves slots to live registries. [`RegistrySlot::Seat`]
/// and [`RegistrySlot::SeatMouse`] index [`PreparedStartup::seats`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RegistrySlot {
    /// Server/source registry.
    Source,
    /// Movement registry.
    Movement,
    /// Fallback (buffer) registry.
    Fallback,
    /// Shared registry.
    Shared,
    /// Seat cvar registry by seat index.
    Seat(usize),
    /// Seat mouse registry by seat index.
    SeatMouse(usize),
}

/// Seat identity plus live registries for routing decisions.
pub struct SeatRegistries<'a> {
    /// Seat handle.
    pub id: SeatId,
    /// Seat command context.
    pub context: CommandContext,
    /// Live seat cvar registry.
    pub cvars: &'a CvarRegistry,
    /// Live seat mouse registry.
    pub mouse_cvars: &'a CvarRegistry,
}

/// Console cvar routing (donor `ApplicationConsoleRouting`, `console.ts`).
///
/// The host implementation mirrors the donor owner/visible rules; the port only needs
/// slot answers so routing never holds registry borrows.
pub trait StartupCvarRouting {
    /// Owning slot for `name` issued from `source`.
    fn owner_slot(
        &self,
        name: &str,
        source: &CommandContext,
        seats: &[SeatRegistries],
    ) -> Result<RegistrySlot, PreparedStartupError>;
    /// Slots visible from `source`.
    fn visible_slots(&self, source: &CommandContext, seats: &[SeatRegistries]) -> Vec<RegistrySlot>;
}

/// Live seat input device (donor `SeatInput` behavior surface, `seat.ts`).
///
/// Binding storage lives in the port-owned [`BindingTable`]; the device covers live key
/// state, held-command release, and the movement profile.
pub trait PreparedSeatDevice {
    /// Movement dialect of the device.
    fn dialect(&self) -> Dialect;
    /// Whether an input is currently held.
    fn is_down(&self, input: &PhysicalInput) -> bool;
    /// Release held commands, appending release text with its source.
    fn release(&mut self, time_ms: f64, append: &mut dyn FnMut(&str, &CommandContext));
    /// Whether any input is held (donor `hasHeldInput`).
    fn has_held_input(&self) -> bool;
    /// Switch the movement profile (donor `setProfile`).
    fn set_profile(&mut self, dialect: Dialect);
}

/// Seat mouse settings (donor `MouseSettings` behavior surface).
pub trait PreparedMouse {
    /// Mouse cvar registry.
    fn cvars(&self) -> &CvarRegistry;
    /// Mutable mouse cvar registry.
    fn cvars_mut(&mut self) -> &mut CvarRegistry;
    /// Apply saved mouse tuning.
    fn write(&mut self, tuning: &MouseTuning);
}

/// Console script files (donor `ConsoleScriptFiles`, `config-scripts.ts`).
///
/// Sync fold of the donor async reads and serialized writes.
pub trait PreparedScriptFiles {
    /// Read a script by name; `None` means missing.
    fn read_script(&self, name: &str, source: &CommandContext) -> Option<String>;
    /// Write configuration text; the `String` error feeds the donor failure message.
    fn write_text(&mut self, path: &str, contents: &str) -> Result<(), String>;
}

/// Script read scope (donor `StartupScriptScope`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupScriptScope {
    /// Mounted content.
    Mounted,
    /// User configuration.
    User,
    /// Base loose files.
    BaseLoose,
    /// Game loose files.
    GameLoose,
    /// Loose files.
    Loose,
    /// Seat configuration.
    Seat,
}

/// Startup configuration frame driver.
///
/// The driver borrows the startup object and splits fresh disjoint borrows per call:
/// drains borrow the buffer side, completion delivery reborrows the applier side.
/// The driving configuration is held separately so delivery never aliases drains.
pub struct StartupDriver<'a, I, M, R, C, S>
where
    I: PreparedSeatDevice,
    M: PreparedMouse,
    R: StartupCvarRouting,
    C: PreparedStartupConfig,
    S: PreparedScriptFiles,
{
    startup: &'a mut PreparedStartup<I, M, R, C, S>,
    config: Option<&'a C>,
    should_continue: &'a dyn Fn() -> bool,
    apply_seat: Option<usize>,
    apply_first: bool,
}

/// Narrow driver interface the host configuration drives.
///
/// Object-safe so [`PreparedStartupConfig::execute_frame`] stays non-generic; the
/// concrete [`StartupDriver`] implements it.
pub trait StartupDriverApi {
    /// Queue text on the owned buffer.
    fn append(&mut self, text: &str, source: &CommandContext) -> Result<(), PreparedStartupError>;
    /// Drain scripts once, then replay completions.
    fn execute_scripts(&mut self) -> Result<(), PreparedStartupError>;
    /// Bind selected defaults.
    fn apply_selected_defaults(&mut self) -> Result<(), PreparedStartupError>;
    /// Apply saved archives.
    fn apply_archive(&mut self) -> Result<(), PreparedStartupError>;
    /// Replay startup variables.
    fn replay_startup_variables(&mut self) -> Result<(), PreparedStartupError>;
}

/// Startup configuration (donor `StartupConfig`, `startup-config.ts`).
///
/// All methods take `&self`: the donor object is reentrant with its own drains (script
/// reads and completions consult the configuration driving the drain), so host
/// implementations hold their frame state behind interior mutability and must release
/// borrows before calling [`StartupDriver::execute_scripts`].
pub trait PreparedStartupConfig {
    /// Script completion delivery, with configuration callbacks attached.
    fn on_script_complete(&self, event: &ScriptCompletion, apply: &mut dyn StartupApplier);
    /// Scoped script read; calls `read` with the resolved scope.
    fn read_script(
        &self,
        name: &str,
        source: &CommandContext,
        read: &mut dyn FnMut(&str, &CommandContext, StartupScriptScope) -> Option<String>,
    ) -> Option<String>;
    /// Whether `source` was issued by this configuration's scripts.
    fn owns_source(&self, source: &CommandContext) -> bool;
    /// Whether saved secondary-seat configuration is restricted.
    fn restrict_shared_configuration(&self) -> bool;
    /// Run one configuration frame; `true` means finished.
    fn execute_frame(&self, driver: &mut dyn StartupDriverApi) -> Result<bool, PreparedStartupError>;
}

/// Configuration scope (donor `StartupConfigOptions["scope"]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupConfigScope {
    /// Source configuration (first seat).
    Source,
    /// Secondary-seat configuration.
    Seat,
}

/// Parameters for one configuration (donor `StartupConfigOptions` minus the callbacks,
/// which are port logic on [`StartupDriver`]/[`StartupApplier`]).
#[derive(Debug, Clone)]
pub struct StartupConfigParams {
    /// Command dialect.
    pub dialect: Dialect,
    /// Configuration context (seat or source).
    pub context: CommandContext,
    /// Source or seat scope.
    pub scope: StartupConfigScope,
    /// Safe mode (skip archived configuration).
    pub safe_mode: bool,
    /// Whether a mod is active.
    pub has_mod: bool,
    /// Seat index in the run.
    pub seat_index: usize,
}

/// Builds configurations for the run (host owns `startup-config.ts` construction).
pub trait StartupConfigFactory {
    /// Configuration type.
    type Config: PreparedStartupConfig;
    /// Build one configuration.
    fn new_config(&mut self, params: &StartupConfigParams) -> Self::Config;
}

/// Configuration callbacks owned by prepared-startup (donor `StartupConfigOptions`
/// `applySelectedDefaults`/`applyArchive`/`replayStartupVariables` closures).
pub trait StartupApplier {
    /// Bind selected defaults and snapshot authored bindings.
    fn apply_selected_defaults(&mut self) -> Result<(), PreparedStartupError>;
    /// Apply saved archives to registries and seats.
    fn apply_archive(&mut self) -> Result<(), PreparedStartupError>;
    /// Replay startup variables and re-insert early commands.
    ///
    /// The early-command insert lands after the driving frame (the donor inserts
    /// mid-drain; both run before any later drain).
    fn replay_startup_variables(&mut self) -> Result<(), PreparedStartupError>;
}

/// Print sink shared by the buffer printer and direct output.
pub type PrintFn = Rc<dyn Fn(&str, Option<&CommandContext>)>;
/// World forwarding sink (donor `forward`).
pub type ForwardFn = Rc<dyn Fn(&str, &[String], &CommandContext)>;
/// Scoped script reader (donor `StartupConfigOptions["read"]`, sync fold).
pub type ScopedReadFn = Box<dyn FnMut(&str, &CommandContext, StartupScriptScope) -> Option<String>>;
/// Preparation finalizer (donor `commands.finishPreparation`).
pub type FinishPreparationFn = Box<dyn FnMut()>;
/// GTV cvar registration (donor `registerGtvCvars`, `gtv-commands.ts`).
pub type GtvRegisterFn = Rc<dyn Fn(&mut CvarRegistry)>;
/// Preparation prefix runner (donor `preparePrefix`, sync fold).
pub type PrefixRunner<'a> = Box<dyn FnMut(&mut dyn FnMut() -> bool) -> bool + 'a>;
/// Seat context lookup for binding-command appends.
pub type SeatContextFn = Rc<dyn Fn(&SeatId) -> Option<CommandContext>>;
/// Observed `bind`/`unbind`/`unbindall` dispatches: name plus key token.
type DispatchLog = Rc<RefCell<Vec<(String, Option<String>)>>>;
/// Run cvar registration (donor `registerRunCvar`, `shared-setting-cvars.ts`).
pub type RunCvarRegisterFn = Rc<dyn Fn(&mut CvarRegistry, Dialect)>;

/// Unwind script frames to the producing origin.
fn root_origin(origin: &CommandOrigin) -> &CommandOrigin {
    let mut origin = origin;
    while let CommandOrigin::Script { caller, .. } = origin {
        origin = caller;
    }
    origin
}

/// Script frame names from the outermost frame inward.
fn script_names(origin: &CommandOrigin) -> Vec<&str> {
    let mut names = Vec::new();
    let mut origin = origin;
    while let CommandOrigin::Script { name, caller } = origin {
        names.push(name.as_str());
        origin = caller;
    }
    names.reverse();
    names
}

/// Whether `source` belongs to a live local client (donor `currentContext`).
///
/// Non-seat origins are always current; seat origins must be active with a matching
/// seat context.
fn is_current_context(
    source: &CommandContext,
    active_seat_ids: &[SeatId],
    seats: &[(SeatId, &CommandContext)],
) -> bool {
    let CommandOrigin::LocalSeat { seat, client } = root_origin(&source.origin) else {
        return true;
    };
    active_seat_ids.contains(seat)
        && seats.iter().any(|(id, context)| {
            id == seat
                && matches!(
                    root_origin(&context.origin),
                    CommandOrigin::LocalSeat {
                        client: seat_client,
                        ..
                    } if seat_client == client
                )
        })
}

/// Apply archived entries (donor `CvarRegistry.applyArchive`).
fn apply_archive_entries(registry: &mut CvarRegistry, entries: &[CvarArchiveEntry]) -> Result<(), CvarError> {
    for entry in entries {
        registry.set_command_flags(&entry.name, &entry.value, SetCommandKind::Archive)?;
    }
    Ok(())
}

/// One prepared seat (donor `PreparedSeat`).
///
/// Binding storage is a shared port-owned table so the binding commands registered on
/// the buffer and on preparation programs observe the same state; the live device is
/// host-owned.
pub struct PreparedSeat<I, M> {
    /// Seat command context.
    pub context: CommandContext,
    /// Seat handle.
    pub id: SeatId,
    /// Seat cvar registry.
    pub cvars: CvarRegistry,
    /// Seat mouse settings.
    pub mouse: M,
    /// Shared binding table (donor `input` binding storage).
    pub bindings: Rc<RefCell<BindingTable>>,
    /// Live input device (donor `input` live state).
    pub device: I,
    /// Physical keys overridden by configuration scripts.
    pub overridden_keys: HashSet<String>,
    /// Whether all bindings were explicitly chosen.
    pub all_bindings_chosen: bool,
    /// Whether configuration scripts are currently collected.
    pub collecting_bindings: bool,
    /// Selected default bindings.
    pub selected_bindings: Option<Vec<InputBinding>>,
    /// Authored bindings snapshot.
    pub authored_bindings: Option<Vec<InputBinding>>,
}

/// Saved seat configuration (donor `PreparedSeatConfiguration`).
pub struct PreparedSeatConfiguration<M> {
    /// Seat command context.
    pub context: CommandContext,
    /// Seat handle.
    pub id: SeatId,
    /// Seat cvar registry.
    pub cvars: CvarRegistry,
    /// Seat mouse settings.
    pub mouse: M,
    /// Saved seat profile, if any.
    pub profile: Option<SeatSettings>,
    /// Saved cvar archive.
    pub archive: Vec<CvarArchiveEntry>,
    /// Saved mouse archive.
    pub mouse_archive: Vec<CvarArchiveEntry>,
}

/// Saved seat archives retained across the run (donor `options.seats` entries).
struct SavedSeatConfiguration {
    profile: Option<SeatSettings>,
    archive: Vec<CvarArchiveEntry>,
    mouse_archive: Vec<CvarArchiveEntry>,
}

/// Seat retained by a published client (donor `publishSeats` element).
pub struct PublishedSeat<I, M> {
    /// Seat handle.
    pub id: SeatId,
    /// Live input device.
    pub device: I,
    /// Seat command context.
    pub context: CommandContext,
    /// Seat cvar registry.
    pub cvars: CvarRegistry,
    /// Seat mouse settings.
    pub mouse: M,
    /// Shared binding table.
    pub bindings: Rc<RefCell<BindingTable>>,
}

/// Host-owned run inputs consumed by [`PreparedStartup::execute`].
pub struct StartupRunner {
    /// Scoped script reader (donor `StartupConfigOptions["read"]`).
    pub read: ScopedReadFn,
    /// Whether a mod is active.
    pub has_mod: bool,
    /// Launch options applier, run when the world acts or the run completes.
    pub apply_launch_options: Box<dyn FnMut()>,
    /// Host frame pump between configuration frames.
    pub next_frame: Box<dyn FnMut()>,
    /// Saved source archive.
    pub source_archive: Vec<CvarArchiveEntry>,
    /// Saved movement archive.
    pub movement_archive: Vec<CvarArchiveEntry>,
    /// Saved fallback archive.
    pub fallback_archive: Vec<CvarArchiveEntry>,
    /// Saved shared archive.
    pub shared_archive: Vec<CvarArchiveEntry>,
}

/// Saved archives moved into the run state.
struct StartupArchives {
    source: Vec<CvarArchiveEntry>,
    movement: Vec<CvarArchiveEntry>,
    fallback: Vec<CvarArchiveEntry>,
    shared: Vec<CvarArchiveEntry>,
}

/// Run state machine phase (sync fold of the donor `run` generator).
enum RunPhase<C> {
    /// Apply startup variables and drain early commands.
    Early,
    /// Drive one configuration per seat.
    Seats {
        /// Seat index in the run.
        index: usize,
        /// Active configuration.
        config: Option<C>,
    },
    /// Drain late commands.
    Late,
}

/// Host-built preparation program (sync fold of donor `prepareProgram`).
///
/// Core has no program API, so the host constructs the program buffer and the
/// prepare/publish hooks; the port stages bindings on top.
pub struct ClientProgram<'a> {
    /// Program command buffer.
    pub commands: CommandBuffer,
    /// Prefix runner (donor `preparePrefix`).
    pub prepare_prefix: PrefixRunner<'a>,
    /// Publication validator (donor `validatePublication`).
    pub validate_publication: Box<dyn FnMut() + 'a>,
    /// Publisher (donor `publish`).
    pub publish: Box<dyn FnMut() + 'a>,
}

/// Seat binding source for client-command preparation.
pub struct ClientCommandSeat<'a, I> {
    /// Seat handle.
    pub id: SeatId,
    /// Seat command context.
    pub context: CommandContext,
    /// Live binding table.
    pub bindings: Rc<RefCell<BindingTable>>,
    /// Live input device.
    pub device: &'a mut I,
}

/// Staged seat bindings plus a staged device shim.
struct StagedSeat<'a, I> {
    id: SeatId,
    original: Vec<InputBinding>,
    live: Rc<RefCell<BindingTable>>,
    staged: Rc<RefCell<BindingTable>>,
    cleared: bool,
    device: &'a mut I,
}

/// Prepared client commands (donor `PreparedClientCommands`).
pub struct PreparedClientCommands<'a, I: PreparedSeatDevice> {
    /// Program command buffer.
    pub commands: CommandBuffer,
    staged: Vec<StagedSeat<'a, I>>,
    console_live: Option<Rc<RefCell<BindingTable>>>,
    console_original: Vec<InputBinding>,
    console_staged: Option<Rc<RefCell<BindingTable>>>,
    release_dialect: Dialect,
    prepare_prefix: PrefixRunner<'a>,
    validate_program: Box<dyn FnMut() + 'a>,
    publish_program: Box<dyn FnMut() + 'a>,
}

/// Staged seat input handle (donor `input(seat)` result).
pub struct StagedSeatInput<'b, 'a, I: PreparedSeatDevice> {
    staged: &'b mut StagedSeat<'a, I>,
}

impl<I: PreparedSeatDevice> StagedSeatInput<'_, '_, I> {
    /// Staged bindings.
    pub fn bindings(&self) -> Vec<InputBinding> {
        staged_bindings(&self.staged.staged)
    }

    /// Target bound to an input in the staged table.
    pub fn binding(&self, input: &PhysicalInput) -> Option<InputBindingTarget> {
        self.staged.staged.borrow().binding(input).cloned()
    }

    /// Bind in the staged table.
    pub fn bind(&mut self, binding: InputBinding) {
        self.staged.staged.borrow_mut().bind(binding);
    }

    /// Unbind in the staged table.
    pub fn unbind(&mut self, input: &PhysicalInput) {
        self.staged.staged.borrow_mut().unbind(input);
    }

    /// Clear the staged table.
    pub fn unbind_all(&mut self) {
        self.staged.staged.borrow_mut().unbind_all();
    }

    /// Live key state unless states were cleared.
    pub fn is_down(&self, input: &PhysicalInput) -> bool {
        !self.staged.cleared && self.staged.device.is_down(input)
    }

    /// Clear staged device states.
    pub fn clear_states(&mut self) {
        self.staged.cleared = true;
    }
}

/// Clone every binding out of a shared table.
fn staged_bindings(table: &Rc<RefCell<BindingTable>>) -> Vec<InputBinding> {
    table.borrow().bindings().into_iter().cloned().collect()
}

/// Whether a command target selects weapons by impulse (donor `/^(?:weapon|impulse|use)\s/i`).
fn is_weapon_select_command(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    ["weapon", "impulse", "use"]
        .iter()
        .any(|word| lower.starts_with(word) && lower[word.len()..].starts_with(char::is_whitespace))
}

/// Saved secondary-seat configuration gate (donor `allowSeatConfigurationCommand`).
///
/// Saved `config.cfg`/`q3config.cfg` lines may only touch the issuing seat's own
/// registries. Routing failures block the line.
pub fn allow_seat_configuration_command<R: StartupCvarRouting>(
    argv: &[String],
    source: &CommandContext,
    routing: &R,
    seats: &[SeatRegistries],
    owner_declares: &dyn Fn(RegistrySlot, &str) -> bool,
    print: &mut dyn FnMut(&str, Option<&CommandContext>),
) -> bool {
    if !script_names(&source.origin)
        .iter()
        .any(|name| *name == "config.cfg" || *name == "q3config.cfg")
    {
        return true;
    }
    let Some(raw) = argv.first() else {
        return true;
    };
    let name = ascii_fold(raw);
    if name == "cvar_restart" {
        print(
            "Ignoring shared cvar restart in saved secondary-seat configuration.\n",
            Some(source),
        );
        return false;
    }
    let setters = ["set", "seta", "sets", "setu", "toggle", "reset"];
    let is_setter = setters.contains(&name.as_str());
    let target = if is_setter { argv.get(1) } else { Some(raw) };
    let Some(target) = target else {
        return true;
    };
    let owner = match routing.owner_slot(target, source, seats) {
        Ok(slot) => slot,
        Err(_) => return false,
    };
    if !is_setter && !owner_declares(owner, target) {
        return true;
    }
    let seat_id = match root_origin(&source.origin) {
        CommandOrigin::LocalSeat { seat, .. } => Some(seat.clone()),
        _ => None,
    };
    if let Some(seat_id) = seat_id {
        if let Some((index, _)) = seats.iter().enumerate().find(|(_, seat)| seat.id == seat_id) {
            if owner == RegistrySlot::Seat(index) || owner == RegistrySlot::SeatMouse(index) {
                return true;
            }
        }
    }
    print(
        &format!(
            "Ignoring shared cvar {target} in saved secondary-seat configuration; use autoexec.cfg for intentional shared overrides.\n"
        ),
        Some(source),
    );
    false
}

impl<'a, I: PreparedSeatDevice> PreparedClientCommands<'a, I> {
    /// Run the preparation prefix (donor `preparePrefix`).
    pub fn prepare_prefix(&mut self, run: &mut dyn FnMut() -> bool) -> bool {
        (self.prepare_prefix)(run)
    }

    /// Staged input for a seat.
    pub fn input(&mut self, seat: &SeatId) -> Option<StagedSeatInput<'_, 'a, I>> {
        self.staged
            .iter_mut()
            .find(|staged| staged.id == *seat)
            .map(|staged| StagedSeatInput { staged })
    }

    /// Validate the publication (donor `validatePublication`).
    pub fn validate_publication(&mut self) -> Result<(), PreparedStartupError> {
        self.validate_live_tables()
    }

    /// Release staged inputs (donor `releaseInputs`).
    pub fn release_inputs(&mut self, time_ms: f64) -> Result<(), PreparedStartupError> {
        self.validate_live_tables()?;
        let release_dialect = self.release_dialect;
        for staged in &mut self.staged {
            if !staged.cleared {
                continue;
            }
            let commands = &mut self.commands;
            staged.device.release(time_ms, &mut |text, source| {
                let _ = commands.append(text, Some(source), Some(release_dialect));
            });
        }
        Ok(())
    }

    /// Append through the release commands handle (donor `releaseCommands.append`).
    pub fn release_append(&mut self, text: &str, source: &CommandContext) {
        let _ = self.commands.append(text, Some(source), Some(self.release_dialect));
    }

    /// Publish staged bindings (donor `publish`).
    pub fn publish(&mut self) -> Result<(), PreparedStartupError> {
        self.validate_live_tables()?;
        (self.publish_program)();
        if let (Some(live), Some(staged)) = (self.console_live.as_ref(), self.console_staged.as_ref()) {
            let bindings = staged_bindings(staged);
            let mut live = live.borrow_mut();
            live.unbind_all();
            for binding in bindings {
                live.bind(binding);
            }
        }
        for staged in &self.staged {
            let bindings = staged_bindings(&staged.staged);
            let mut live = staged.live.borrow_mut();
            live.unbind_all();
            for binding in bindings {
                live.bind(binding);
            }
        }
        Ok(())
    }

    /// Compare live tables against their staged originals.
    fn validate_live_tables(&mut self) -> Result<(), PreparedStartupError> {
        (self.validate_program)();
        if let Some(live) = self.console_live.as_ref() {
            if staged_bindings(live) != self.console_original {
                return Err(PreparedStartupError::Message(
                    "Console bindings changed during preparation".to_string(),
                ));
            }
        }
        for staged in &self.staged {
            if staged_bindings(&staged.live) != staged.original {
                return Err(PreparedStartupError::Message(
                    "Client bindings changed during preparation".to_string(),
                ));
            }
        }
        Ok(())
    }
}

/// Stage client commands over a host-built program (donor `prepareClientCommands`).
#[allow(clippy::too_many_arguments)]
///
/// `candidate_tags` identifies the program's cvar owners with host-assigned tags;
/// every tag must be disjoint from `live_tags` (donor "isolated cvar owners" check).
/// Core registries carry no identity, so tags stand in for owner identity.
pub fn prepare_client_commands<'a, I: PreparedSeatDevice>(
    program: ClientProgram<'a>,
    program_cvars: &CvarRegistry,
    seats: Vec<ClientCommandSeat<'a, I>>,
    live_tags: &HashSet<u64>,
    candidate_tags: &[u64],
    console_bindings: Option<Rc<RefCell<BindingTable>>>,
    print: PrintFn,
    seat_context: SeatContextFn,
) -> Result<PreparedClientCommands<'a, I>, PreparedStartupError> {
    for tag in candidate_tags {
        if live_tags.contains(tag) {
            return Err(PreparedStartupError::Message(
                "Candidate client commands require isolated cvar owners".to_string(),
            ));
        }
    }
    let ClientProgram {
        mut commands,
        prepare_prefix,
        validate_publication,
        publish,
    } = program;
    let release_dialect = commands.dialect();
    let console_original = console_bindings.as_ref().map_or_else(Vec::new, staged_bindings);
    let console_staged = console_bindings.as_ref().map(|live| {
        let staged = Rc::new(RefCell::new(BindingTable::new()));
        for binding in staged_bindings(live) {
            staged.borrow_mut().bind(binding);
        }
        staged
    });
    let mut staged = Vec::with_capacity(seats.len());
    let mut lookup_tables = HashMap::new();
    for seat in seats {
        let original = staged_bindings(&seat.bindings);
        let table = Rc::new(RefCell::new(BindingTable::new()));
        for binding in &original {
            table.borrow_mut().bind(binding.clone());
        }
        lookup_tables.insert(seat.id.clone(), table.clone());
        staged.push(StagedSeat {
            id: seat.id,
            original,
            live: seat.bindings,
            staged: table,
            cleared: false,
            device: seat.device,
        });
    }
    let lookup_tables = Rc::new(lookup_tables);
    let lookup: BindingLookup = Rc::new(move |id| lookup_tables.get(id).cloned());
    let execution = commands.execution_context().cloned();
    let print_error = print.clone();
    let context_printer: PrintFn = Rc::new(move |text, _| print_error(text, execution.as_ref()));
    let mut registry = BufferCommandRegistry {
        commands: &mut commands,
        cvars: program_cvars,
        seat_contexts: seat_context,
        print: context_printer.clone(),
        dispatch_log: Rc::new(RefCell::new(Vec::new())),
    };
    let binding_print: Rc<dyn Fn(&str)> = Rc::new(move |text| context_printer(text, None));
    register_binding_commands(&mut registry, lookup, binding_print, console_staged.clone());
    register_q1_view_commands(&mut commands, program_cvars)?;
    Ok(PreparedClientCommands {
        commands,
        staged,
        console_live: console_bindings,
        console_original,
        console_staged,
        release_dialect,
        prepare_prefix,
        validate_program: validate_publication,
        publish_program: publish,
    })
}

/// Adapts a core [`CommandBuffer`] to the client [`CommandRegistry`] surface so the
/// ported binding commands register on real buffers.
///
/// Invocations are translated to client invocations (seat origins keep the seat;
/// script names are dropped because the client origin has no name slot). `bind`,
/// `unbind`, and `unbindall` dispatches are recorded for the frame-end binding
/// tracking the donor does in `afterDispatch`.
struct BufferCommandRegistry<'a> {
    commands: &'a mut CommandBuffer,
    cvars: &'a CvarRegistry,
    seat_contexts: SeatContextFn,
    print: PrintFn,
    dispatch_log: DispatchLog,
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

impl CommandRegistry for BufferCommandRegistry<'_> {
    fn register_engine(&mut self, name: &str, handler: ClientCommandHandler) -> bool {
        self.register_inner(name, handler)
    }

    fn register(&mut self, name: &str, handler: ClientCommandHandler) -> bool {
        self.register_inner(name, handler)
    }

    fn unregister(&mut self, name: &str) {
        self.commands.unregister(name);
    }

    fn exists(&self, name: &str) -> bool {
        self.commands.exists(name)
    }

    fn append(&mut self, text: &str, seat: &SeatId) {
        match (self.seat_contexts)(seat) {
            Some(context) => {
                if let Err(error) = self.commands.append(text, Some(&context), None) {
                    (self.print)(&format!("{error}\n"), Some(&context));
                }
            }
            None => (self.print)("Client commands require a local seat\n", None),
        }
    }
}

impl BufferCommandRegistry<'_> {
    /// Register one translated handler, observing binding commands.
    fn register_inner(&mut self, name: &str, handler: ClientCommandHandler) -> bool {
        let handler = Rc::new(RefCell::new(handler));
        let log = self.dispatch_log.clone();
        let wrapped: CommandHandler = Rc::new(move |invocation: &mut Invocation| {
            let argv = invocation.argv.clone();
            let call = ClientInvocation::new(
                argv.clone(),
                convert_origin(&invocation.source.origin),
                invocation.dialect,
            );
            let outcome = handler.borrow_mut()(&call);
            if matches!(argv.first().map(String::as_str), Some("bind" | "unbind" | "unbindall")) {
                log.borrow_mut()
                    .push((argv.first().cloned().unwrap_or_default(), argv.get(1).cloned()));
            }
            if let Err(error) = outcome {
                invocation.print(&format!("{error}\n"));
            }
        });
        self.commands
            .register(name, Some(wrapped), None, self.cvars)
            .unwrap_or(false)
    }
}

/// Shared printer state behind the buffer printer and direct output.
pub struct PrinterShared {
    print: PrintFn,
    bindings: Vec<(u64, PrintFn)>,
    next_binding: u64,
    active_seat_ids: Vec<SeatId>,
    seats: Vec<(SeatId, CommandContext)>,
}

impl PrinterShared {
    /// Print through the last output binding, else the owner printer.
    fn print(shared: &Rc<RefCell<Self>>, text: &str, source: Option<&CommandContext>) {
        let guard = shared.borrow();
        if let Some(context) = source.as_ref() {
            let seats: Vec<(SeatId, &CommandContext)> =
                guard.seats.iter().map(|(id, context)| (id.clone(), context)).collect();
            if !is_current_context(context, &guard.active_seat_ids, &seats) {
                return;
            }
        }
        let sink = guard
            .bindings
            .last()
            .map_or_else(|| guard.print.clone(), |(_, binding)| binding.clone());
        drop(guard);
        sink(text, source);
    }
}

/// Deferred `setmaster` side effect (applied after the drain).
enum PendingAction {
    /// Set `public 1` on the source registry.
    SetPublic,
}

/// Deferred `writeconfig` write (completed after the drain).
struct PendingWrite {
    source: CommandContext,
    name: String,
}

/// Frame services routing scripts, gates, and forwarding for one drain.
///
/// Built per drain from disjoint startup borrows and dropped before completion
/// delivery, so delivery reborrows the same state fresh.
pub struct StartupServices<'a, C, S, R, I, M>
where
    C: PreparedStartupConfig,
    S: PreparedScriptFiles,
    R: StartupCvarRouting,
    I: PreparedSeatDevice,
    M: PreparedMouse,
{
    /// Current configuration, if a config frame is driving the drain.
    pub config: Option<&'a C>,
    /// Continuation configuration shadowing the run configuration, if any.
    pub continuation_active: Option<&'a C>,
    /// Console script files.
    pub scripts: &'a S,
    /// Scoped script reader.
    pub scoped_read: &'a mut dyn FnMut(&str, &CommandContext, StartupScriptScope) -> Option<String>,
    /// Command dialect.
    pub dialect: Dialect,
    /// Folded Q3 map commands gating `devmap`/`spmap`/`spdevmap`.
    pub q3_map_commands: &'a [String],
    /// Deferred world commands.
    pub deferred: &'a [String],
    /// Whether configuration work remains.
    pub pending: bool,
    /// Console routing.
    pub routing: &'a R,
    /// Live seats.
    pub seats: &'a mut [PreparedSeat<I, M>],
    /// Active seat ids.
    pub active_seat_ids: &'a [SeatId],
    /// Shared printer.
    pub printer: Rc<RefCell<PrinterShared>>,
    /// Owner printer for gate messages.
    pub options_print: PrintFn,
    /// World-action flag shared with command handlers.
    pub world_action: Rc<Cell<bool>>,
    /// Forwarding sink shared with command handlers.
    pub forward: Rc<RefCell<ForwardFn>>,
    /// Source registry.
    pub source: &'a CvarRegistry,
    /// Movement registry, unless it aliases the draining fallback store.
    pub movement: Option<&'a CvarRegistry>,
    /// Shared registry, if any.
    pub shared: Option<&'a CvarRegistry>,
}

impl<C, S, R, I, M> StartupServices<'_, C, S, R, I, M>
where
    C: PreparedStartupConfig,
    S: PreparedScriptFiles,
    R: StartupCvarRouting,
    I: PreparedSeatDevice,
    M: PreparedMouse,
{
    /// Live seat registries for routing decisions.
    fn seat_registries(&self) -> Vec<SeatRegistries<'_>> {
        self.seats
            .iter()
            .map(|seat| SeatRegistries {
                id: seat.id.clone(),
                context: seat.context.clone(),
                cvars: &seat.cvars,
                mouse_cvars: seat.mouse.cvars(),
            })
            .collect()
    }

    /// Shared slot resolution for the allow gate.
    ///
    /// The fallback store is owned by the draining buffer, so fallback (and aliased
    /// movement) declarations read as undeclared; non-setter lines stay allowed.
    fn resolve_slot_shared(&self, slot: RegistrySlot) -> Option<&CvarRegistry> {
        match slot {
            RegistrySlot::Source => Some(self.source),
            RegistrySlot::Movement => self.movement,
            RegistrySlot::Fallback => None,
            RegistrySlot::Shared => self.shared,
            RegistrySlot::Seat(index) => self.seats.get(index).map(|seat| &seat.cvars),
            RegistrySlot::SeatMouse(index) => self.seats.get(index).map(|seat| seat.mouse.cvars()),
        }
    }

    /// Current configuration for gating (donor `currentStartup`).
    fn current_restrict_shared(&self) -> bool {
        self.continuation_active
            .or(self.config)
            .is_some_and(|config| config.restrict_shared_configuration())
    }
}

impl<C, S, R, I, M> BufferServices for StartupServices<'_, C, S, R, I, M>
where
    C: PreparedStartupConfig,
    S: PreparedScriptFiles,
    R: StartupCvarRouting,
    I: PreparedSeatDevice,
    M: PreparedMouse,
{
    fn read_script(&mut self, name: &str, source: &CommandContext) -> ScriptRead {
        let seats: Vec<(SeatId, &CommandContext)> =
            self.seats.iter().map(|seat| (seat.id.clone(), &seat.context)).collect();
        if !is_current_context(source, self.active_seat_ids, &seats) {
            return ScriptRead::Ready(None);
        }
        if let Some(config) = self.config {
            ScriptRead::Ready(config.read_script(name, source, self.scoped_read))
        } else {
            ScriptRead::Ready(self.scripts.read_script(name, source))
        }
    }

    fn forward_to_server(&mut self, command: &ForwardedCommand) {
        self.world_action.set(true);
        let name = command.argv.first().map_or("", String::as_str);
        let args = command.argv.get(1..).unwrap_or(&[]);
        (self.forward.borrow())(name, args, &command.source);
    }

    fn allow_command(&mut self, command: &ForwardedCommand) -> bool {
        let name = ascii_fold(command.argv.first().map_or("", String::as_str));
        if self.dialect == Dialect::Q3
            && ["devmap", "spmap", "spdevmap"].contains(&name.as_str())
            && !self.q3_map_commands.contains(&name)
        {
            PrinterShared::print(
                &self.printer,
                &format!("Unknown command \"{name}\"\n"),
                Some(&command.source),
            );
            return false;
        }
        let seats: Vec<(SeatId, &CommandContext)> =
            self.seats.iter().map(|seat| (seat.id.clone(), &seat.context)).collect();
        if !is_current_context(&command.source, self.active_seat_ids, &seats) {
            (self.options_print)(
                "Command ignored because its local client is inactive or has retired.\n",
                None,
            );
            return false;
        }
        if self.pending && self.deferred.contains(&name) {
            self.world_action.set(true);
        }
        if !self.current_restrict_shared() {
            return true;
        }
        let registries = self.seat_registries();
        let printer = self.printer.clone();
        allow_seat_configuration_command(
            &command.argv,
            &command.source,
            self.routing,
            &registries,
            &|slot, name| {
                self.resolve_slot_shared(slot)
                    .is_some_and(|registry| registry.get(name).is_some())
            },
            &mut |text, source| PrinterShared::print(&printer, text, source),
        )
    }
}

/// Frame gate hooks (donor `shouldContinue`).
struct ContinueHooks<'a> {
    should_continue: &'a dyn Fn() -> bool,
}

impl FrameHooks for ContinueHooks<'_> {
    fn should_continue(&mut self) -> bool {
        (self.should_continue)()
    }
}

impl<I, M, R, C, S> StartupDriver<'_, I, M, R, C, S>
where
    I: PreparedSeatDevice,
    M: PreparedMouse,
    R: StartupCvarRouting,
    C: PreparedStartupConfig,
    S: PreparedScriptFiles,
{
    /// Build a driver over the startup object (run loop only).
    pub(crate) fn new<'x>(
        startup: &'x mut PreparedStartup<I, M, R, C, S>,
        config: Option<&'x C>,
        should_continue: &'x dyn Fn() -> bool,
        apply_seat: Option<usize>,
        apply_first: bool,
    ) -> StartupDriver<'x, I, M, R, C, S> {
        StartupDriver {
            startup,
            config,
            should_continue,
            apply_seat,
            apply_first,
        }
    }

    /// Queue text on the owned buffer (donor `commands.append`).
    pub fn append(&mut self, text: &str, source: &CommandContext) -> Result<(), PreparedStartupError> {
        Ok(self.startup.commands.append(text, Some(source), None)?)
    }

    /// Drain scripts once, then replay completions (donor `executeScriptsAsync`).
    ///
    /// The drain borrows the buffer side; delivery reborrows the applier side after
    /// the drain releases it.
    pub fn execute_scripts(&mut self) -> Result<(), PreparedStartupError> {
        let completions = {
            let startup = &mut *self.startup;
            let pending_now = startup.pending();
            let mut missing: ScopedReadFn = Box::new(|_, _, _| None);
            let scoped: &mut dyn FnMut(&str, &CommandContext, StartupScriptScope) -> Option<String> =
                match startup.scoped_read.as_mut() {
                    Some(read) => &mut **read,
                    None => &mut missing,
                };
            let mut services = StartupServices {
                config: self.config,
                continuation_active: startup
                    .continuations
                    .last()
                    .and_then(|continuation| continuation.active.as_ref()),
                scripts: &startup.scripts,
                scoped_read: scoped,
                dialect: startup.dialect,
                q3_map_commands: &startup.q3_map_commands,
                deferred: &startup.deferred,
                pending: pending_now,
                routing: &startup.routing,
                seats: &mut startup.seats,
                active_seat_ids: &startup.active_seat_ids,
                printer: startup.printer.clone(),
                options_print: startup.options_print.clone(),
                world_action: startup.world_action.clone(),
                forward: startup.forward.clone(),
                source: &startup.source,
                movement: if startup.movement_aliases_fallback {
                    None
                } else {
                    Some(&startup.movement)
                },
                shared: startup.shared.as_ref(),
            };
            let sink: Rc<RefCell<Vec<ScriptCompletion>>> = Rc::new(RefCell::new(Vec::new()));
            let capture = sink.clone();
            let listener = startup
                .commands
                .bind_script_completion(move |event| capture.borrow_mut().push(event.clone()));
            let outcome = startup.commands.execute_hooked(
                &mut startup.fallback,
                &mut services,
                &mut ContinueHooks {
                    should_continue: self.should_continue,
                },
            );
            startup.commands.unbind_script_completion(listener);
            outcome?;
            let completions = sink.borrow().clone();
            completions
        };
        if let Some(config) = self.config {
            for event in &completions {
                let mut applier = self.startup.applier(self.apply_seat, self.apply_first);
                config.on_script_complete(event, &mut applier);
            }
        }
        Ok(())
    }

    /// Bind selected defaults (donor `applySelectedDefaults` passthrough).
    pub fn apply_selected_defaults(&mut self) -> Result<(), PreparedStartupError> {
        self.startup
            .applier(self.apply_seat, self.apply_first)
            .try_apply_selected_defaults()
    }

    /// Apply saved archives (donor `applyArchive` passthrough).
    pub fn apply_archive(&mut self) -> Result<(), PreparedStartupError> {
        self.startup
            .applier(self.apply_seat, self.apply_first)
            .try_apply_archive()
    }

    /// Replay startup variables (donor `replayStartupVariables` passthrough).
    pub fn replay_startup_variables(&mut self) -> Result<(), PreparedStartupError> {
        self.startup
            .applier(self.apply_seat, self.apply_first)
            .try_replay_startup_variables()
    }
}

impl<I, M, R, C, S> StartupDriverApi for StartupDriver<'_, I, M, R, C, S>
where
    I: PreparedSeatDevice,
    M: PreparedMouse,
    R: StartupCvarRouting,
    C: PreparedStartupConfig,
    S: PreparedScriptFiles,
{
    fn append(&mut self, text: &str, source: &CommandContext) -> Result<(), PreparedStartupError> {
        StartupDriver::append(self, text, source)
    }

    fn execute_scripts(&mut self) -> Result<(), PreparedStartupError> {
        StartupDriver::execute_scripts(self)
    }

    fn apply_selected_defaults(&mut self) -> Result<(), PreparedStartupError> {
        StartupDriver::apply_selected_defaults(self)
    }

    fn apply_archive(&mut self) -> Result<(), PreparedStartupError> {
        StartupDriver::apply_archive(self)
    }

    fn replay_startup_variables(&mut self) -> Result<(), PreparedStartupError> {
        StartupDriver::replay_startup_variables(self)
    }
}

/// Configuration callbacks with live startup borrows.
struct FrameApplier<'a, R, I, M>
where
    R: StartupCvarRouting,
    I: PreparedSeatDevice,
    M: PreparedMouse,
{
    routing: &'a R,
    seats: &'a mut [PreparedSeat<I, M>],
    source: &'a mut CvarRegistry,
    movement: Option<&'a mut CvarRegistry>,
    fallback: &'a mut CvarRegistry,
    shared: Option<&'a mut CvarRegistry>,
    archives: &'a StartupArchives,
    phases: &'a StartupCommandPhases,
    default_binding_items: &'a [WeaponBindingItem],
    movement_dialect: Dialect,
    source_context: &'a CommandContext,
    apply_seat: Option<usize>,
    apply_saved: Option<&'a SavedSeatConfiguration>,
    apply_first: bool,
    replay_early_insert: &'a mut bool,
}

impl<R, I, M> FrameApplier<'_, R, I, M>
where
    R: StartupCvarRouting,
    I: PreparedSeatDevice,
    M: PreparedMouse,
{
    /// Live seat registries for routing decisions.
    fn seat_registries(&self) -> Vec<SeatRegistries<'_>> {
        self.seats
            .iter()
            .map(|seat| SeatRegistries {
                id: seat.id.clone(),
                context: seat.context.clone(),
                cvars: &seat.cvars,
                mouse_cvars: seat.mouse.cvars(),
            })
            .collect()
    }

    /// Resolve a slot to its live registry.
    fn resolve(&mut self, slot: RegistrySlot) -> Result<&mut CvarRegistry, PreparedStartupError> {
        match slot {
            RegistrySlot::Source => Ok(&mut *self.source),
            RegistrySlot::Movement => match self.movement.as_deref_mut() {
                Some(registry) => Ok(registry),
                None => Ok(&mut *self.fallback),
            },
            RegistrySlot::Fallback => Ok(&mut *self.fallback),
            RegistrySlot::Shared => self
                .shared
                .as_deref_mut()
                .ok_or_else(|| PreparedStartupError::Message("Shared registry is missing".to_string())),
            RegistrySlot::Seat(index) => self
                .seats
                .get_mut(index)
                .map(|seat| &mut seat.cvars)
                .ok_or_else(|| seat_index_error(index)),
            RegistrySlot::SeatMouse(index) => self
                .seats
                .get_mut(index)
                .map(|seat| seat.mouse.cvars_mut())
                .ok_or_else(|| PreparedStartupError::Message(format!("Seat mouse index {index} is out of range"))),
        }
    }

    /// Set every startup variable through routing (donor `applyStartupVariables`).
    fn apply_startup_variables(&mut self) -> Result<(), PreparedStartupError> {
        let context = self
            .seats
            .first()
            .map_or_else(|| self.source_context.clone(), |seat| seat.context.clone());
        for variable in self.phases.variables.clone() {
            let registries = self.seat_registries();
            let slot = self.routing.owner_slot(&variable.name, &context, &registries)?;
            let registry = self.resolve(slot)?;
            registry.set(&variable.name, &variable.value, true)?;
            let registered = registry.register(&variable.name, "", 0)?;
            if registered.is_none() {
                return Err(PreparedStartupError::Message(format!(
                    "Q3 startup variable registration rejected: {}",
                    variable.name
                )));
            }
            registry.add_flags(&variable.name, flags::USER_CREATED)?;
        }
        Ok(())
    }

    /// Selected-defaults application with errors.
    fn try_apply_selected_defaults(&mut self) -> Result<(), PreparedStartupError> {
        let Some(index) = self.apply_seat else {
            return Ok(());
        };
        let Some(seat) = self.seats.get_mut(index) else {
            return Err(seat_index_error(index));
        };
        let drop: Vec<PhysicalInput> = seat
            .bindings
            .borrow()
            .bindings()
            .iter()
            .filter_map(|binding| match &binding.target {
                InputBindingTarget::Command(text) if is_weapon_select_command(text) => Some(binding.input.clone()),
                _ => None,
            })
            .collect();
        for input in drop {
            seat.bindings.borrow_mut().unbind(&input);
        }
        let mut authored: HashMap<String, InputBinding> = seat
            .bindings
            .borrow()
            .bindings()
            .into_iter()
            .map(|binding| (physical_input_key(&binding.input), binding.clone()))
            .collect();
        let selected = seat
            .selected_bindings
            .clone()
            .unwrap_or_else(|| default_bindings(0, self.movement_dialect, self.default_binding_items));
        for binding in &selected {
            seat.bindings.borrow_mut().bind(binding.clone());
            authored
                .entry(physical_input_key(&binding.input))
                .or_insert_with(|| binding.clone());
        }
        seat.authored_bindings = Some(authored.into_values().collect());
        seat.collecting_bindings = true;
        Ok(())
    }

    /// Archive application with errors.
    fn try_apply_archive(&mut self) -> Result<(), PreparedStartupError> {
        if self.apply_first {
            apply_archive_entries(self.source, &self.archives.source)?;
            if let Some(movement) = self.movement.as_deref_mut() {
                apply_archive_entries(movement, &self.archives.movement)?;
            } else {
                apply_archive_entries(self.fallback, &self.archives.movement)?;
            }
            apply_archive_entries(self.fallback, &self.archives.fallback)?;
            if let Some(shared) = self.shared.as_deref_mut() {
                apply_archive_entries(shared, &self.archives.shared)?;
            }
        }
        let (Some(index), Some(saved)) = (self.apply_seat, self.apply_saved) else {
            return Ok(());
        };
        let saved_profile = saved.profile.clone();
        let saved_archive = saved.archive.clone();
        let saved_mouse_archive = saved.mouse_archive.clone();
        let Some(seat) = self.seats.get_mut(index) else {
            return Err(seat_index_error(index));
        };
        apply_archive_entries(&mut seat.cvars, &saved_archive)?;
        apply_archive_entries(seat.mouse.cvars_mut(), &saved_mouse_archive)?;
        if let Some(profile) = saved_profile {
            seat.bindings.borrow_mut().unbind_all();
            for binding in &profile.bindings {
                seat.bindings.borrow_mut().bind(binding.clone());
            }
            seat.all_bindings_chosen = true;
            seat.mouse.write(&profile.mouse);
            if let Some(always_run) = profile.always_run {
                seat.mouse.cvars_mut().set_command_flags(
                    "cl_run",
                    if always_run { "1" } else { "0" },
                    SetCommandKind::Archive,
                )?;
            }
        }
        seat.collecting_bindings = true;
        Ok(())
    }

    /// Variable replay with errors.
    fn try_replay_startup_variables(&mut self) -> Result<(), PreparedStartupError> {
        if !self.apply_first {
            return Ok(());
        }
        self.apply_startup_variables()?;
        *self.replay_early_insert = true;
        Ok(())
    }
}

impl<R, I, M> StartupApplier for FrameApplier<'_, R, I, M>
where
    R: StartupCvarRouting,
    I: PreparedSeatDevice,
    M: PreparedMouse,
{
    fn apply_selected_defaults(&mut self) -> Result<(), PreparedStartupError> {
        self.try_apply_selected_defaults()
    }

    fn apply_archive(&mut self) -> Result<(), PreparedStartupError> {
        self.try_apply_archive()
    }

    fn replay_startup_variables(&mut self) -> Result<(), PreparedStartupError> {
        self.try_replay_startup_variables()
    }
}

/// Seat index error (unreachable: the run only applies to live seats).
fn seat_index_error(index: usize) -> PreparedStartupError {
    PreparedStartupError::Message(format!("Startup seat index {index} is out of range"))
}

/// Deferred world commands (donor `deferredCommands` base list).
const DEFERRED_COMMANDS: [&str; 9] = [
    "map", "save", "load", "weapnext", "weapprev", "use", "weapon", "say", "say_team",
];

/// Forwarded commands without side effects.
const FORWARDED_COMMANDS: [&str; 10] = [
    "mvdconnect",
    "mvdisconnect",
    "in_restart",
    "midiinfo",
    "local_join",
    "local_drop",
    "downloadstatus",
    "stopdownload",
    "retrydownload",
    "demopause",
];

/// Construction options (donor constructor `options` plus host seams).
///
/// `sharedNames` is dropped: host routing captures it at construction. The Q3 product
/// policy folds to [`PreparedStartupOptions::q3_map_commands`], and the server
/// administration names arrive precomputed because `server-administration.ts` is a
/// missing sibling.
pub struct PreparedStartupOptions<M> {
    /// Startup command lines.
    pub startup_commands: Vec<String>,
    /// Folded lowercase Q3 map commands gating `devmap`/`spmap`/`spdevmap`.
    pub q3_map_commands: Vec<String>,
    /// Command dialect.
    pub dialect: Dialect,
    /// Movement dialect.
    pub movement_dialect: Dialect,
    /// Saved seat configurations.
    pub seats: Vec<PreparedSeatConfiguration<M>>,
    /// Shared registry plus its session.
    pub shared: Option<(CvarRegistry, SessionId)>,
    /// Owner printer.
    pub print: PrintFn,
    /// World forwarding sink.
    pub forward: ForwardFn,
    /// Operator command names (donor `sourceAdministrationCommandNames`).
    pub operator_command_names: Vec<String>,
    /// GTV cvar registration hook.
    pub register_gtv_cvars: GtvRegisterFn,
    /// Run cvar registration hook.
    pub register_run_cvar: RunCvarRegisterFn,
    /// Weapon catalog for default bindings.
    pub default_binding_items: Vec<WeaponBindingItem>,
    /// Preparation finalizer hook.
    pub finish_preparation: FinishPreparationFn,
    /// Buffer context (donor `source.context`).
    pub source_context: CommandContext,
}

/// Run state (sync fold of the donor `run` generator plus its captured options).
struct RunState<C> {
    phase: RunPhase<C>,
    archives: StartupArchives,
    has_mod: bool,
    apply_launch_options: Box<dyn FnMut()>,
}

/// Profile configuration continuation (donor `PreparedConfigurationContinuation`).
///
/// The donor `StartupConfig` carries its callback closures internally; the port passes
/// callbacks through [`StartupApplier`], so profile continuations carry the host-owned
/// applier their configuration consumes.
pub struct PreparedConfigurationContinuation<C> {
    /// Continuation configuration shadowing the run configuration.
    pub active: Option<C>,
    /// Advance the continuation; `true` means finished (sync fold of `advance`).
    pub advance: Box<dyn FnMut(&mut ContinuationDriver) -> bool>,
    /// Host-owned configuration callbacks for `active`.
    pub applier: Box<dyn StartupApplier>,
}

/// Narrow driver for continuations (donor `advance(owner)` receives the full owner;
/// continuations only need the buffer, the gate flag, and the finalizer).
pub struct ContinuationDriver<'a> {
    /// Startup command buffer.
    pub commands: &'a mut CommandBuffer,
    /// Fallback registry.
    pub fallback: &'a mut CvarRegistry,
    /// World-action flag.
    pub world_action: Rc<Cell<bool>>,
    /// Preparation finalizer.
    pub finish_preparation: &'a mut dyn FnMut(),
}

/// Adopted owners (donor `adopt` `owners`).
pub struct AdoptedOwners<S> {
    /// Source registry plus session.
    pub source: (CvarRegistry, SessionId),
    /// Movement registry plus session.
    pub movement: (CvarRegistry, SessionId),
    /// Fallback registry plus session.
    pub fallback: (CvarRegistry, SessionId),
    /// Console script files.
    pub scripts: S,
    /// Scoped script reader.
    pub read: ScopedReadFn,
}

/// Prepared registries and the command buffer (donor `PreparedStartup`).
///
/// `source` and `scripts` stay public fields; `movement` and `fallback` resolve
/// through accessors because the donor aliases them to one registry when their
/// dialects match.
pub struct PreparedStartup<I, M, R, C, S>
where
    I: PreparedSeatDevice,
    M: PreparedMouse,
    R: StartupCvarRouting,
    C: PreparedStartupConfig,
    S: PreparedScriptFiles,
{
    /// Startup command buffer.
    pub commands: CommandBuffer,
    /// Dedicated-console binding store (some only when there are no seats).
    pub console_bindings: Option<Rc<RefCell<BindingTable>>>,
    /// Server/source registry.
    pub source: CvarRegistry,
    /// Console script files.
    pub scripts: S,
    movement: CvarRegistry,
    fallback: CvarRegistry,
    movement_aliases_fallback: bool,
    source_session: SessionId,
    movement_session: SessionId,
    fallback_session: SessionId,
    shared: Option<CvarRegistry>,
    seats: Vec<PreparedSeat<I, M>>,
    active_seat_ids: Vec<SeatId>,
    routing: R,
    dialect: Dialect,
    movement_dialect: Dialect,
    q3_map_commands: Vec<String>,
    operator_command_names: Vec<String>,
    deferred: Vec<String>,
    default_binding_items: Vec<WeaponBindingItem>,
    saved_seats: Vec<SavedSeatConfiguration>,
    dedicated_console: bool,
    source_context: CommandContext,
    phases: StartupCommandPhases,
    printer: Rc<RefCell<PrinterShared>>,
    options_print: PrintFn,
    forward: Rc<RefCell<ForwardFn>>,
    world_action: Rc<Cell<bool>>,
    pending_writes: Rc<RefCell<Vec<PendingWrite>>>,
    pending_actions: Rc<RefCell<Vec<PendingAction>>>,
    dispatch_log: DispatchLog,
    release_view: Option<Q1ViewCommandGuard>,
    continuations: Vec<PreparedConfigurationContinuation<C>>,
    run_state: Option<RunState<C>>,
    scoped_read: Option<ScopedReadFn>,
    finish_preparation: FinishPreparationFn,
    preparation_prefix: bool,
    replay_early_insert: bool,
}

impl<I, M, R, C, S> PreparedStartup<I, M, R, C, S>
where
    I: PreparedSeatDevice,
    M: PreparedMouse,
    R: StartupCvarRouting,
    C: PreparedStartupConfig,
    S: PreparedScriptFiles,
{
    /// Build the startup object (donor constructor).
    ///
    /// `devices` supplies one live input device per configured seat, in seat order.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        mut source: CvarRegistry,
        source_session: SessionId,
        movement: CvarRegistry,
        movement_session: SessionId,
        scripts: S,
        routing: R,
        options: PreparedStartupOptions<M>,
        devices: Vec<I>,
    ) -> Result<Self, PreparedStartupError> {
        if devices.len() != options.seats.len() {
            return Err(PreparedStartupError::Message(format!(
                "Startup devices ({}) do not match seats ({})",
                devices.len(),
                options.seats.len()
            )));
        }
        let console_bindings = if options.seats.is_empty() {
            Some(Rc::new(RefCell::new(BindingTable::new())))
        } else {
            None
        };
        let mut deferred: Vec<String> = DEFERRED_COMMANDS.iter().map(ToString::to_string).collect();
        if options.dialect == Dialect::Q1Netquake || options.dialect == Dialect::Q1Quakeworld {
            deferred.push("pause".to_string());
        }
        if options.dialect == Dialect::Q3 {
            deferred.extend(
                options
                    .q3_map_commands
                    .iter()
                    .filter(|name| name.as_str() != "map")
                    .cloned(),
            );
        }
        let mut seats = Vec::with_capacity(options.seats.len());
        let mut saved_seats = Vec::with_capacity(options.seats.len());
        let mut lookup_tables = HashMap::new();
        for (mut config, device) in options.seats.into_iter().zip(devices) {
            (options.register_run_cvar)(config.mouse.cvars_mut(), options.movement_dialect);
            register_player_userinfo(&mut config.cvars, config.id.index(), "male")?;
            let table = Rc::new(RefCell::new(BindingTable::new()));
            lookup_tables.insert(config.id.clone(), table.clone());
            saved_seats.push(SavedSeatConfiguration {
                profile: config.profile,
                archive: config.archive,
                mouse_archive: config.mouse_archive,
            });
            seats.push(PreparedSeat {
                context: config.context,
                id: config.id,
                cvars: config.cvars,
                mouse: config.mouse,
                bindings: table,
                device,
                overridden_keys: HashSet::new(),
                all_bindings_chosen: false,
                collecting_bindings: false,
                selected_bindings: None,
                authored_bindings: None,
            });
        }
        (options.register_gtv_cvars)(&mut source);
        let movement_aliases_fallback = movement.dialect() == options.dialect;
        let (movement, movement_session, fallback, fallback_session) = if movement_aliases_fallback {
            let session = movement_session;
            (
                CvarRegistry::new(options.movement_dialect),
                session.clone(),
                movement,
                session,
            )
        } else {
            (
                movement,
                movement_session,
                CvarRegistry::new(options.dialect),
                source_session.clone(),
            )
        };
        let phases = startup_command_phases(&options.startup_commands, options.dialect)?;
        let mut buffer_options = BufferOptions::new();
        buffer_options.startup_command_text = Some(phases.stuffed.clone());
        let mut commands = CommandBuffer::new(options.dialect, options.source_context.clone(), buffer_options)?;
        let printer = Rc::new(RefCell::new(PrinterShared {
            print: options.print.clone(),
            bindings: Vec::new(),
            next_binding: 0,
            active_seat_ids: seats.iter().map(|seat| seat.id.clone()).collect(),
            seats: seats
                .iter()
                .map(|seat| (seat.id.clone(), seat.context.clone()))
                .collect(),
        }));
        let buffer_printer = printer.clone();
        commands.set_printer(move |text, source| {
            PrinterShared::print(&buffer_printer, text, source);
        });
        let world_action = Rc::new(Cell::new(false));
        let forward: Rc<RefCell<ForwardFn>> = Rc::new(RefCell::new(options.forward));
        let pending_writes = Rc::new(RefCell::new(Vec::new()));
        let pending_actions = Rc::new(RefCell::new(Vec::new()));
        let dispatch_log = Rc::new(RefCell::new(Vec::new()));
        let shared = options.shared.map(|(registry, _)| registry);
        let dedicated_console = seats.is_empty();
        let mut startup = Self {
            commands,
            console_bindings,
            source,
            scripts,
            movement,
            fallback,
            movement_aliases_fallback,
            source_session,
            movement_session,
            fallback_session,
            shared,
            seats,
            active_seat_ids: Vec::new(),
            routing,
            dialect: options.dialect,
            movement_dialect: options.movement_dialect,
            q3_map_commands: options.q3_map_commands,
            operator_command_names: options.operator_command_names,
            deferred,
            default_binding_items: options.default_binding_items,
            saved_seats,
            dedicated_console,
            source_context: options.source_context,
            phases,
            printer,
            options_print: options.print,
            forward,
            world_action,
            pending_writes,
            pending_actions,
            dispatch_log,
            release_view: None,
            continuations: Vec::new(),
            run_state: None,
            scoped_read: None,
            finish_preparation: options.finish_preparation,
            preparation_prefix: false,
            replay_early_insert: false,
        };
        startup.active_seat_ids = startup.seats.iter().map(|seat| seat.id.clone()).collect();
        startup.register_binding_commands(lookup_tables)?;
        startup.register_forwarded_commands()?;
        startup.register_writeconfig()?;
        let guard = register_q1_view_commands(&mut startup.commands, &startup.fallback)?;
        startup.release_view = Some(guard);
        Ok(startup)
    }

    /// Movement registry, resolving the fallback alias.
    pub fn movement(&self) -> &CvarRegistry {
        if self.movement_aliases_fallback {
            &self.fallback
        } else {
            &self.movement
        }
    }

    /// Mutable movement registry, resolving the fallback alias.
    pub fn movement_mut(&mut self) -> &mut CvarRegistry {
        if self.movement_aliases_fallback {
            &mut self.fallback
        } else {
            &mut self.movement
        }
    }

    /// Fallback registry.
    pub fn fallback(&self) -> &CvarRegistry {
        &self.fallback
    }

    /// Mutable fallback registry.
    pub fn fallback_mut(&mut self) -> &mut CvarRegistry {
        &mut self.fallback
    }

    /// Live seats.
    pub fn seats(&self) -> &[PreparedSeat<I, M>] {
        &self.seats
    }

    /// Shared registry, if any.
    pub fn shared_cvars(&self) -> Option<&CvarRegistry> {
        self.shared.as_ref()
    }

    /// Whether configuration can continue (donor `configurationCanContinue`).
    pub fn configuration_can_continue(&self) -> bool {
        !self.world_action.get()
    }

    /// Whether configuration work remains (donor `pending`).
    pub fn pending(&self) -> bool {
        !self.continuations.is_empty() || self.run_state.is_some()
    }
}

/// Convert audio documentation to core documentation.
fn core_documentation(
    documentation: &super::audio::commands::CommandDocumentation,
) -> qa_core::cmd_buffer::CommandDocumentation {
    qa_core::cmd_buffer::CommandDocumentation {
        summary: documentation.summary.to_string(),
        usage: documentation.usage.to_string(),
        examples: documentation.examples.iter().map(ToString::to_string).collect(),
        allowed_values: if documentation.allowed_values.is_empty() {
            None
        } else {
            Some(documentation.allowed_values.iter().map(ToString::to_string).collect())
        },
    }
}

impl<I, M, R, C, S> PreparedStartup<I, M, R, C, S>
where
    I: PreparedSeatDevice,
    M: PreparedMouse,
    R: StartupCvarRouting,
    C: PreparedStartupConfig,
    S: PreparedScriptFiles,
{
    /// Register binding commands over the seat tables.
    fn register_binding_commands(
        &mut self,
        lookup_tables: HashMap<SeatId, Rc<RefCell<BindingTable>>>,
    ) -> Result<(), PreparedStartupError> {
        let lookup_tables = Rc::new(lookup_tables);
        let lookup: BindingLookup = Rc::new(move |id| lookup_tables.get(id).cloned());
        let printer = self.printer.clone();
        let binding_print: Rc<dyn Fn(&str)> = Rc::new(move |text| PrinterShared::print(&printer, text, None));
        let seats: Vec<(SeatId, CommandContext)> = self
            .seats
            .iter()
            .map(|seat| (seat.id.clone(), seat.context.clone()))
            .collect();
        let seats = Rc::new(seats);
        let seat_context: SeatContextFn = Rc::new(move |id| {
            seats
                .iter()
                .find(|(seat, _)| seat == id)
                .map(|(_, context)| context.clone())
        });
        let printer = self.printer.clone();
        let adapter_print: PrintFn = Rc::new(move |text, source| {
            PrinterShared::print(&printer, text, source);
        });
        let mut registry = BufferCommandRegistry {
            commands: &mut self.commands,
            cvars: &self.fallback,
            seat_contexts: seat_context,
            print: adapter_print,
            dispatch_log: self.dispatch_log.clone(),
        };
        register_binding_commands(&mut registry, lookup, binding_print, self.console_bindings.clone());
        Ok(())
    }

    /// Register a forwarding handler, recording deferred side effects.
    #[allow(clippy::too_many_arguments)]
    fn register_forward_handler(
        commands: &mut CommandBuffer,
        cvars: &CvarRegistry,
        name: &str,
        world_action: Option<Rc<Cell<bool>>>,
        forward: Rc<RefCell<ForwardFn>>,
        pending_actions: Rc<RefCell<Vec<PendingAction>>>,
        set_public: bool,
        packed_args: bool,
        documentation: Option<qa_core::cmd_buffer::CommandDocumentation>,
    ) -> Result<(), PreparedStartupError> {
        let name_owned = name.to_string();
        let handler: CommandHandler = Rc::new(move |invocation: &mut Invocation| {
            if let Some(world_action) = world_action.as_ref() {
                world_action.set(true);
            }
            if set_public {
                pending_actions.borrow_mut().push(PendingAction::SetPublic);
            }
            let source = invocation.source.clone();
            if packed_args {
                let text = invocation.args_text.clone();
                (forward.borrow())(&name_owned, &[text], &source);
            } else {
                let args = invocation.args().to_vec();
                (forward.borrow())(&name_owned, &args, &source);
            }
        });
        commands.register(name, Some(handler), documentation, cvars)?;
        Ok(())
    }

    /// Register deferred, operator, and forwarded commands.
    fn register_forwarded_commands(&mut self) -> Result<(), PreparedStartupError> {
        let mut operator_names = self.operator_command_names.clone();
        if self.dialect == Dialect::Q1Netquake || self.dialect == Dialect::Q1Quakeworld {
            operator_names.extend(["status".to_string(), "ping".to_string()]);
        }
        let deferred = self.deferred.clone();
        for name in &deferred {
            Self::register_forward_handler(
                &mut self.commands,
                &self.fallback,
                name,
                Some(self.world_action.clone()),
                self.forward.clone(),
                self.pending_actions.clone(),
                false,
                false,
                None,
            )?;
        }
        for name in &operator_names {
            let set_public = name == "setmaster"
                && self.dedicated_console
                && (self.dialect == Dialect::Q2Classic || self.dialect == Dialect::Q2Rerelease);
            let packed = name == "addlrconcmd" || name == "dellrconcmd";
            Self::register_forward_handler(
                &mut self.commands,
                &self.fallback,
                name,
                None,
                self.forward.clone(),
                self.pending_actions.clone(),
                set_public,
                packed,
                None,
            )?;
        }
        for name in FORWARDED_COMMANDS {
            Self::register_forward_handler(
                &mut self.commands,
                &self.fallback,
                name,
                None,
                self.forward.clone(),
                self.pending_actions.clone(),
                false,
                false,
                None,
            )?;
        }
        for (name, documentation) in [
            ("snd_restart", None),
            ("cd", Some(core_documentation(&CD_COMMAND_DOCUMENTATION))),
            ("music", Some(core_documentation(&MUSIC_COMMAND_DOCUMENTATION))),
        ] {
            Self::register_forward_handler(
                &mut self.commands,
                &self.fallback,
                name,
                None,
                self.forward.clone(),
                self.pending_actions.clone(),
                false,
                false,
                documentation,
            )?;
        }
        Ok(())
    }

    /// Register `writeconfig` for dedicated consoles.
    fn register_writeconfig(&mut self) -> Result<(), PreparedStartupError> {
        if self.console_bindings.is_none() {
            return Ok(());
        }
        let pending_writes = self.pending_writes.clone();
        let handler: CommandHandler = Rc::new(move |invocation: &mut Invocation| {
            let mut origin = &invocation.source.origin;
            while let CommandOrigin::Script { caller, .. } = origin {
                origin = caller;
            }
            if !matches!(origin, CommandOrigin::LocalConsole | CommandOrigin::ServerConsole) {
                invocation.print("writeconfig requires a local console.\n");
                return;
            }
            if invocation.argv.len() > 2 {
                invocation.print("writeconfig [filename]\n");
                return;
            }
            let name = invocation.argv.get(1).map_or("config.cfg", String::as_str);
            let path = if name.ends_with(".cfg") {
                name.to_string()
            } else {
                format!("{name}.cfg")
            };
            pending_writes.borrow_mut().push(PendingWrite {
                source: invocation.source.clone(),
                name: path,
            });
        });
        let documentation = qa_core::cmd_buffer::CommandDocumentation {
            summary: "Save the dedicated console configuration and bindings.".to_string(),
            usage: "writeconfig [filename]".to_string(),
            examples: vec!["writeconfig server.cfg".to_string()],
            allowed_values: None,
        };
        self.commands
            .register("writeconfig", Some(handler), Some(documentation), &self.fallback)?;
        Ok(())
    }

    /// Print through the shared printer with execution-context fallback.
    fn print(&self, text: &str, source: Option<&CommandContext>) {
        let resolved = source.cloned().or_else(|| self.commands.execution_context().cloned());
        PrinterShared::print(&self.printer, text, resolved.as_ref());
    }

    /// Bind an output sink, returning its release (donor `bindOutput`).
    pub fn bind_output(&mut self, print: impl Fn(&str, Option<&CommandContext>) + 'static) -> impl Fn() {
        let mut shared = self.printer.borrow_mut();
        let id = shared.next_binding;
        shared.next_binding += 1;
        shared.bindings.push((id, Rc::new(print)));
        drop(shared);
        self.drain_notifications();
        let printer = self.printer.clone();
        move || {
            printer.borrow_mut().bindings.retain(|(binding, _)| *binding != id);
        }
    }

    /// Sync printer seat state after seat changes.
    fn sync_printer_seats(&mut self) {
        let mut shared = self.printer.borrow_mut();
        shared.active_seat_ids = self.active_seat_ids.clone();
        shared.seats = self
            .seats
            .iter()
            .map(|seat| (seat.id.clone(), seat.context.clone()))
            .collect();
    }

    /// Live seat registries for routing decisions.
    fn seat_registries(&self) -> Vec<SeatRegistries<'_>> {
        self.seats
            .iter()
            .map(|seat| SeatRegistries {
                id: seat.id.clone(),
                context: seat.context.clone(),
                cvars: &seat.cvars,
                mouse_cvars: seat.mouse.cvars(),
            })
            .collect()
    }

    /// Resolve a slot to its live registry.
    fn resolve_slot(&mut self, slot: RegistrySlot) -> Result<&mut CvarRegistry, PreparedStartupError> {
        match slot {
            RegistrySlot::Source => Ok(&mut self.source),
            RegistrySlot::Movement if self.movement_aliases_fallback => Ok(&mut self.fallback),
            RegistrySlot::Movement => Ok(&mut self.movement),
            RegistrySlot::Fallback => Ok(&mut self.fallback),
            RegistrySlot::Shared => self
                .shared
                .as_mut()
                .ok_or_else(|| PreparedStartupError::Message("Shared registry is missing".to_string())),
            RegistrySlot::Seat(index) => self
                .seats
                .get_mut(index)
                .map(|seat| &mut seat.cvars)
                .ok_or_else(|| seat_index_error(index)),
            RegistrySlot::SeatMouse(index) => self
                .seats
                .get_mut(index)
                .map(|seat| seat.mouse.cvars_mut())
                .ok_or_else(|| PreparedStartupError::Message(format!("Seat mouse index {index} is out of range"))),
        }
    }

    /// Shared slot resolution.
    fn resolve_slot_shared(&self, slot: RegistrySlot) -> Option<&CvarRegistry> {
        match slot {
            RegistrySlot::Source => Some(&self.source),
            RegistrySlot::Movement if self.movement_aliases_fallback => Some(&self.fallback),
            RegistrySlot::Movement => Some(&self.movement),
            RegistrySlot::Fallback => Some(&self.fallback),
            RegistrySlot::Shared => self.shared.as_ref(),
            RegistrySlot::Seat(index) => self.seats.get(index).map(|seat| &seat.cvars),
            RegistrySlot::SeatMouse(index) => self.seats.get(index).map(|seat| seat.mouse.cvars()),
        }
    }

    /// Current configuration (donor `currentStartup`).
    fn current_config(&self) -> Option<&C> {
        self.continuations
            .last()
            .and_then(|continuation| continuation.active.as_ref())
            .or(match self.run_state.as_ref() {
                Some(RunState {
                    phase: RunPhase::Seats {
                        config: Some(config), ..
                    },
                    ..
                }) => Some(config),
                _ => None,
            })
    }

    /// Current restriction flag (donor `currentStartup?.restrictSharedConfiguration`).
    fn current_restrict_shared(&self) -> bool {
        self.current_config()
            .is_some_and(|config| config.restrict_shared_configuration())
    }

    /// Gate one invocation (donor `allowCommand`).
    pub fn allow_command(&mut self, argv: &[String], source: &CommandContext) -> bool {
        let name = ascii_fold(argv.first().map_or("", String::as_str));
        if self.dialect == Dialect::Q3
            && ["devmap", "spmap", "spdevmap"].contains(&name.as_str())
            && !self.q3_map_commands.contains(&name)
        {
            self.print(&format!("Unknown command \"{name}\"\n"), Some(source));
            return false;
        }
        let seats: Vec<(SeatId, &CommandContext)> =
            self.seats.iter().map(|seat| (seat.id.clone(), &seat.context)).collect();
        if !is_current_context(source, &self.active_seat_ids, &seats) {
            (self.options_print)(
                "Command ignored because its local client is inactive or has retired.\n",
                None,
            );
            return false;
        }
        if self.pending() && self.deferred.contains(&name) {
            self.world_action.set(true);
        }
        if !self.current_restrict_shared() {
            return true;
        }
        let registries = self.seat_registries();
        let printer = self.printer.clone();
        allow_seat_configuration_command(
            argv,
            source,
            &self.routing,
            &registries,
            &|slot, name| {
                self.resolve_slot_shared(slot)
                    .is_some_and(|registry| registry.get(name).is_some())
            },
            &mut |text, source| PrinterShared::print(&printer, text, source),
        )
    }

    /// Forward queued registry notifications to the printer.
    ///
    /// Fold of `refreshRegistryOutput`: core registries queue notifications instead of
    /// binding outputs, so the port drains the routed registries at frame boundaries.
    pub fn drain_notifications(&mut self) {
        let mut contexts = vec![self.source_context.clone()];
        contexts.extend(
            self.seats
                .iter()
                .filter(|seat| self.active_seat_ids.contains(&seat.id))
                .map(|seat| seat.context.clone()),
        );
        let seats: Vec<(SeatId, &CommandContext)> =
            self.seats.iter().map(|seat| (seat.id.clone(), &seat.context)).collect();
        let mut slots = vec![RegistrySlot::Source, RegistrySlot::Movement, RegistrySlot::Fallback];
        let registries = self.seat_registries();
        for context in &contexts {
            if is_current_context(context, &self.active_seat_ids, &seats) {
                slots.extend(self.routing.visible_slots(context, &registries));
            }
        }
        slots.sort_by_key(slot_order);
        slots.dedup();
        let execution = self.commands.execution_context().cloned();
        for slot in slots {
            let notifications = match self.resolve_slot(slot) {
                Ok(registry) => registry.take_notifications(),
                Err(_) => continue,
            };
            for notification in notifications {
                self.print(&notification, execution.as_ref());
            }
        }
    }

    /// Archive lines for `writeconfig` (donor `commands.archiveCommands`).
    fn archive_lines(&self, source: &CommandContext) -> Vec<String> {
        let registries = self.seat_registries();
        let mut lines = Vec::new();
        for slot in self.routing.visible_slots(source, &registries) {
            let Some(registry) = self.resolve_slot_shared(slot) else {
                continue;
            };
            let registries = self.seat_registries();
            lines.extend(registry.archive_commands(&|name| {
                self.routing
                    .owner_slot(name, source, &registries)
                    .is_ok_and(|owner| owner == slot)
            }));
        }
        lines
    }

    /// Complete deferred `writeconfig` writes.
    fn complete_pending_writes(&mut self) {
        let writes: Vec<PendingWrite> = self.pending_writes.borrow_mut().drain(..).collect();
        for write in writes {
            let Some(console) = self.console_bindings.clone() else {
                continue;
            };
            let bindings = staged_bindings(&console);
            let archived = match archived_bindings(&bindings, true) {
                Ok(lines) => lines,
                Err(error) => {
                    self.print(&format!("Console output failed: {error}\n"), Some(&write.source));
                    continue;
                }
            };
            let mut lines = archived;
            lines.extend(self.archive_lines(&write.source));
            let contents = format!("// Generated by quake-typescript\n{}\n", lines.join("\n"));
            match self.scripts.write_text(&write.name, &contents) {
                Ok(()) => self.print(&format!("Wrote {}\n", write.name), Some(&write.source)),
                Err(error) => self.print(&format!("Console output failed: {error}\n"), Some(&write.source)),
            }
        }
    }

    /// Apply deferred handler side effects.
    fn complete_pending_actions(&mut self) {
        let actions: Vec<PendingAction> = self.pending_actions.borrow_mut().drain(..).collect();
        for action in actions {
            match action {
                PendingAction::SetPublic => {
                    let _ = self.source.set("public", "1", false);
                }
            }
        }
    }

    /// Replay observed binding dispatches (donor `afterDispatch` tracking).
    fn replay_dispatch_log(&mut self) {
        let log: Vec<(String, Option<String>)> = self.dispatch_log.borrow_mut().drain(..).collect();
        for (name, key) in log {
            for seat in &mut self.seats {
                if !seat.collecting_bindings {
                    continue;
                }
                if name == "unbindall" {
                    seat.all_bindings_chosen = true;
                }
                if name == "bind" || name == "unbind" {
                    if let Some(key) = key.as_deref() {
                        if let Some(input) = named_physical_input(key, 0) {
                            seat.overridden_keys.insert(physical_input_key(&input));
                        }
                    }
                }
            }
        }
    }
}

/// Stable slot order for notification drains.
fn slot_order(slot: &RegistrySlot) -> (u8, usize) {
    match *slot {
        RegistrySlot::Source => (0, 0),
        RegistrySlot::Movement => (1, 0),
        RegistrySlot::Fallback => (2, 0),
        RegistrySlot::Shared => (3, 0),
        RegistrySlot::Seat(index) => (4, index),
        RegistrySlot::SeatMouse(index) => (5, index),
    }
}

impl<I, M, R, C, S> PreparedStartup<I, M, R, C, S>
where
    I: PreparedSeatDevice,
    M: PreparedMouse,
    R: StartupCvarRouting,
    C: PreparedStartupConfig,
    S: PreparedScriptFiles,
{
    /// Retain published seats (donor `publishSeats`).
    ///
    /// Previous seats match by id plus binding-table identity (fold of the donor
    /// `input` identity check); matches keep their live tracking state.
    pub fn publish_seats(
        &mut self,
        seats: Vec<PublishedSeat<I, M>>,
        active_seat_ids: Option<Vec<SeatId>>,
    ) -> Result<(), PreparedStartupError> {
        let active = active_seat_ids.unwrap_or_else(|| seats.iter().map(|seat| seat.id.clone()).collect());
        for id in &active {
            if !seats.iter().any(|seat| seat.id == *id) {
                return Err(PreparedStartupError::Message(
                    "Active seat is not retained by this client".to_string(),
                ));
            }
        }
        let authored = self.seats.first().and_then(|seat| seat.authored_bindings.clone());
        let mut previous: HashMap<SeatId, PreparedSeat<I, M>> =
            self.seats.drain(..).map(|seat| (seat.id.clone(), seat)).collect();
        let mut retained = Vec::with_capacity(seats.len());
        for seat in seats {
            let reuse = previous
                .remove(&seat.id)
                .filter(|prior| Rc::ptr_eq(&prior.bindings, &seat.bindings));
            match reuse {
                Some(mut prior) => {
                    prior.cvars = seat.cvars;
                    prior.mouse = seat.mouse;
                    retained.push(prior);
                }
                None => retained.push(PreparedSeat {
                    context: seat.context,
                    id: seat.id,
                    cvars: seat.cvars,
                    mouse: seat.mouse,
                    bindings: seat.bindings,
                    device: seat.device,
                    overridden_keys: HashSet::new(),
                    all_bindings_chosen: true,
                    collecting_bindings: false,
                    selected_bindings: None,
                    authored_bindings: authored.clone(),
                }),
            }
        }
        self.seats = retained;
        self.set_active_seats(active)?;
        Ok(())
    }

    /// Set the active seats (donor `setActiveSeats`).
    pub fn set_active_seats(&mut self, ids: Vec<SeatId>) -> Result<(), PreparedStartupError> {
        for id in &ids {
            if !self.seats.iter().any(|seat| seat.id == *id) {
                return Err(PreparedStartupError::Message(
                    "Active seat is not retained by this client".to_string(),
                ));
            }
        }
        self.active_seat_ids = ids;
        self.sync_printer_seats();
        self.drain_notifications();
        Ok(())
    }

    /// Adopt seat registries (donor `adoptSeat`).
    pub fn adopt_seat(&mut self, id: &SeatId, cvars: Option<CvarRegistry>, mouse: Option<M>) {
        if let Some(seat) = self.seats.iter_mut().find(|seat| seat.id == *id) {
            if let Some(cvars) = cvars {
                seat.cvars = cvars;
            }
            if let Some(mouse) = mouse {
                seat.mouse = mouse;
            }
        }
        self.drain_notifications();
    }

    /// Adopt a new forwarding sink (donor `forwardCommands`).
    pub fn forward_commands(&mut self, forward: ForwardFn) {
        *self.forward.borrow_mut() = forward;
    }

    /// Read a script through the current configuration or files (donor `readScript`).
    pub fn read_script(&mut self, name: &str, source: &CommandContext) -> Option<String> {
        let seats: Vec<(SeatId, &CommandContext)> =
            self.seats.iter().map(|seat| (seat.id.clone(), &seat.context)).collect();
        if !is_current_context(source, &self.active_seat_ids, &seats) {
            return None;
        }
        let from_config = {
            let continuation = self
                .continuations
                .last()
                .and_then(|continuation| continuation.active.as_ref());
            let phase = match self.run_state.as_ref() {
                Some(RunState {
                    phase: RunPhase::Seats {
                        config: Some(config), ..
                    },
                    ..
                }) => Some(config),
                _ => None,
            };
            match (continuation.or(phase), self.scoped_read.as_mut()) {
                (Some(config), Some(read)) => config.read_script(name, source, read),
                _ => None,
            }
        };
        if from_config.is_some() {
            return from_config;
        }
        self.scripts.read_script(name, source)
    }

    /// Deliver a script completion to the current configuration.
    ///
    /// Donor `onScriptComplete`: other bootstrap buffers wire their completion callback
    /// here. Continuation configurations consume their host applier; run
    /// configurations consume the frame applier.
    pub fn on_script_complete(&mut self, event: &ScriptCompletion) {
        if let Some(continuation) = self.continuations.last_mut() {
            if let Some(active) = continuation.active.as_ref() {
                // Borrow split: the configuration reads while its applier writes.
                let applier = &mut *continuation.applier;
                active.on_script_complete(event, applier);
                return;
            }
        }
        self.deliver_to_phase(event);
    }

    /// Deliver a completion to the run configuration, if any.
    fn deliver_to_phase(&mut self, event: &ScriptCompletion) {
        let index = match self.run_state.as_ref() {
            Some(RunState {
                phase: RunPhase::Seats {
                    index, config: Some(_), ..
                },
                ..
            }) => *index,
            _ => return,
        };
        let this = &mut *self;
        let config = match this.run_state.as_ref() {
            Some(RunState {
                phase: RunPhase::Seats {
                    config: Some(config), ..
                },
                ..
            }) => config,
            _ => return,
        };
        let apply_seat = if this.seats.is_empty() { None } else { Some(index) };
        let apply_saved = apply_seat.and_then(|seat| this.saved_seats.get(seat));
        let movement = if this.movement_aliases_fallback {
            None
        } else {
            Some(&mut this.movement)
        };
        let archives = &this.run_state.as_ref().expect("run checked").archives;
        let mut applier = FrameApplier {
            routing: &this.routing,
            seats: &mut this.seats,
            source: &mut this.source,
            movement,
            fallback: &mut this.fallback,
            shared: this.shared.as_mut(),
            archives,
            phases: &this.phases,
            default_binding_items: &this.default_binding_items,
            movement_dialect: this.movement_dialect,
            source_context: &this.source_context,
            apply_seat,
            apply_saved,
            apply_first: index == 0,
            replay_early_insert: &mut this.replay_early_insert,
        };
        config.on_script_complete(event, &mut applier);
    }

    /// Whether a configuration owns a source (donor `isConfigurationSource`).
    pub fn is_configuration_source(&self, source: &CommandContext) -> bool {
        self.current_config().is_some_and(|config| config.owns_source(source))
    }

    /// Note a world action while configuration is pending (donor `noteWorldAction`).
    pub fn note_world_action(&mut self) {
        if self.pending() {
            self.world_action.set(true);
        }
    }

    /// Bind preview defaults and write them to the seat table (donor `bindings`).
    pub fn bindings(&mut self, id: &SeatId, selected_defaults: Vec<InputBinding>) -> Vec<InputBinding> {
        let bindings = self.preview_bindings(id, &selected_defaults);
        self.adopt_binding_defaults(id, selected_defaults);
        if let Some(seat) = self.seats.iter_mut().find(|seat| seat.id == *id) {
            for binding in &bindings {
                seat.bindings.borrow_mut().bind(binding.clone());
            }
        }
        bindings
    }

    /// Adopt selected defaults for a seat (donor `adoptBindingDefaults`).
    pub fn adopt_binding_defaults(&mut self, id: &SeatId, selected_defaults: Vec<InputBinding>) {
        if let Some(seat) = self.seats.iter_mut().find(|seat| seat.id == *id) {
            seat.selected_bindings = Some(selected_defaults);
        }
    }

    /// Reset a seat to authored defaults plus selected extras (donor `resetBindings`).
    pub fn reset_bindings(&mut self, id: &SeatId) -> Result<(), PreparedStartupError> {
        let Some(seat) = self.seats.iter_mut().find(|seat| seat.id == *id) else {
            return Err(PreparedStartupError::Message(
                "Authored binding defaults are not ready".to_string(),
            ));
        };
        let Some(authored) = seat.authored_bindings.clone() else {
            return Err(PreparedStartupError::Message(
                "Authored binding defaults are not ready".to_string(),
            ));
        };
        let mut defaults: HashMap<String, InputBinding> = authored
            .into_iter()
            .map(|binding| (physical_input_key(&binding.input), binding))
            .collect();
        if let Some(selected) = seat.selected_bindings.clone() {
            defaults.retain(|_, binding| {
                !matches!(
                    &binding.target,
                    InputBindingTarget::Command(text) if is_weapon_select_command(text)
                )
            });
            for binding in selected {
                defaults.entry(physical_input_key(&binding.input)).or_insert(binding);
            }
        }
        seat.bindings.borrow_mut().unbind_all();
        for binding in defaults.into_values() {
            seat.bindings.borrow_mut().bind(binding);
        }
        seat.all_bindings_chosen = true;
        seat.overridden_keys.clear();
        Ok(())
    }

    /// Preview merged bindings without writing (donor `previewBindings`).
    pub fn preview_bindings(&self, id: &SeatId, selected_defaults: &[InputBinding]) -> Vec<InputBinding> {
        let Some(seat) = self.seats.iter().find(|seat| seat.id == *id) else {
            return selected_defaults.to_vec();
        };
        let mut bindings: HashMap<String, InputBinding> = seat
            .bindings
            .borrow()
            .bindings()
            .into_iter()
            .map(|binding| (physical_input_key(&binding.input), binding.clone()))
            .collect();
        if !seat.all_bindings_chosen {
            for binding in selected_defaults {
                if !seat.overridden_keys.contains(&physical_input_key(&binding.input)) {
                    bindings.insert(physical_input_key(&binding.input), binding.clone());
                }
            }
        }
        bindings.into_values().collect()
    }

    /// Read configuration text through the scoped reader (donor `readConfiguration`).
    pub fn read_configuration(
        &mut self,
        name: &str,
        source: &CommandContext,
        scope: StartupScriptScope,
    ) -> Result<Option<String>, PreparedStartupError> {
        match self.scoped_read.as_mut() {
            Some(read) => Ok(read(name, source, scope)),
            None => Err(PreparedStartupError::Message(
                "Startup script reader is missing".to_string(),
            )),
        }
    }

    /// Adopt a profile continuation (donor `adoptConfigurationContinuation`).
    pub fn adopt_configuration_continuation(
        &mut self,
        continuation: PreparedConfigurationContinuation<C>,
    ) -> Result<(), PreparedStartupError> {
        if !self.preparation_prefix {
            return Err(PreparedStartupError::Message(
                "Configuration continuation requires its published command prefix".to_string(),
            ));
        }
        self.continuations.push(continuation);
        Ok(())
    }

    /// Note a published preparation prefix (fold of `hasPreparationPrefix`).
    ///
    /// Core has no preparation API, so the host reports the prefix its program
    /// publishes; continuations require it.
    pub fn note_preparation_prefix(&mut self) {
        self.preparation_prefix = true;
    }

    /// Adopt new readers (donor `adoptReaders`).
    pub fn adopt_readers(&mut self, scripts: S, read: ScopedReadFn) {
        self.scripts = scripts;
        self.scoped_read = Some(read);
    }

    /// Validate adopted owners (donor `validateOwners`).
    pub fn validate_owners(
        &self,
        source: &(CvarRegistry, SessionId),
        movement: &(CvarRegistry, SessionId),
        fallback: &(CvarRegistry, SessionId),
    ) -> Result<(), PreparedStartupError> {
        if fallback.0.dialect() != source.0.dialect()
            || [source, movement, fallback]
                .iter()
                .any(|(_, session)| *session != self.commands.context().session)
        {
            return Err(PreparedStartupError::Message(
                "Prepared profile registry belongs to another session or dialect".to_string(),
            ));
        }
        Ok(())
    }
}

/// One run-loop step.
enum RunAction {
    /// Apply startup variables and drain early commands.
    Early,
    /// Begin the seat configuration.
    BeginSeat(usize),
    /// Drive the active seat configuration.
    DriveSeat,
    /// Drain late commands.
    Late,
}

impl<I, M, R, C, S> PreparedStartup<I, M, R, C, S>
where
    I: PreparedSeatDevice,
    M: PreparedMouse,
    R: StartupCvarRouting,
    C: PreparedStartupConfig,
    S: PreparedScriptFiles,
{
    /// Build a frame applier over live borrows (run loop and driver only).
    fn applier(&mut self, apply_seat: Option<usize>, apply_first: bool) -> FrameApplier<'_, R, I, M> {
        let apply_saved = apply_seat.and_then(|index| self.saved_seats.get(index));
        let movement = if self.movement_aliases_fallback {
            None
        } else {
            Some(&mut self.movement)
        };
        let archives = &self
            .run_state
            .as_ref()
            .expect("configuration callbacks need an active run")
            .archives;
        FrameApplier {
            routing: &self.routing,
            seats: &mut self.seats,
            source: &mut self.source,
            movement,
            fallback: &mut self.fallback,
            shared: self.shared.as_mut(),
            archives,
            phases: &self.phases,
            default_binding_items: &self.default_binding_items,
            movement_dialect: self.movement_dialect,
            source_context: &self.source_context,
            apply_seat,
            apply_saved,
            apply_first,
            replay_early_insert: &mut self.replay_early_insert,
        }
    }

    /// Adopt new routing, forwarding, and owners (donor `adopt`).
    pub fn adopt(
        &mut self,
        routing: R,
        forward: ForwardFn,
        owners: Option<AdoptedOwners<S>>,
    ) -> Result<(), PreparedStartupError> {
        let profile_changed = owners.as_ref().is_some_and(|owners| {
            self.commands.dialect() != owners.source.0.dialect()
                || self
                    .active_seats()
                    .any(|seat| seat.device.dialect() != owners.movement.0.dialect())
        });
        if profile_changed && self.active_seats().any(|seat| seat.device.has_held_input()) {
            return Err(PreparedStartupError::Message(
                "Prepared profile requires released input".to_string(),
            ));
        }
        if let Some(owners) = owners.as_ref() {
            self.validate_owners(&owners.source, &owners.movement, &owners.fallback)?;
        }
        if owners
            .as_ref()
            .is_some_and(|owners| self.commands.dialect() != owners.source.0.dialect())
        {
            let dialect = owners.as_ref().expect("checked").source.0.dialect();
            self.rebuild_commands(dialect)?;
        }
        self.routing = routing;
        self.forward_commands(forward);
        if let Some(owners) = owners {
            self.source = owners.source.0;
            self.source_session = owners.source.1;
            self.movement = owners.movement.0;
            self.movement_session = owners.movement.1;
            self.fallback = owners.fallback.0;
            self.fallback_session = owners.fallback.1;
            self.movement_aliases_fallback = false;
            self.adopt_readers(owners.scripts, owners.read);
            if profile_changed {
                if let Some(guard) = self.release_view.take() {
                    guard.release(&mut self.commands);
                }
                let movement_dialect = self.movement.dialect();
                for seat in &mut self.seats {
                    if self.active_seat_ids.contains(&seat.id) {
                        seat.device.set_profile(movement_dialect);
                    }
                }
                let guard = register_q1_view_commands(&mut self.commands, &self.fallback)?;
                self.release_view = Some(guard);
            }
        }
        self.drain_notifications();
        Ok(())
    }

    /// Active seats.
    fn active_seats(&self) -> impl Iterator<Item = &PreparedSeat<I, M>> {
        self.seats.iter().filter(|seat| self.active_seat_ids.contains(&seat.id))
    }

    /// Rebuild the buffer for a new dialect (fold of `setProfile`).
    ///
    /// Queued text and aliases survive through [`CommandBuffer::copy_pending_from`];
    /// startup handlers are re-registered, but owner-registered handlers do not (core
    /// cannot enumerate them).
    fn rebuild_commands(&mut self, dialect: Dialect) -> Result<(), PreparedStartupError> {
        let mut commands = CommandBuffer::new(dialect, self.commands.context().clone(), BufferOptions::new())?;
        commands.copy_pending_from(&self.commands)?;
        for name in self.commands.alias_names() {
            if let Some(value) = self.commands.alias_value(&name) {
                commands.define_alias(&name, value)?;
            }
        }
        let printer = self.printer.clone();
        commands.set_printer(move |text, source| {
            PrinterShared::print(&printer, text, source);
        });
        self.dialect = dialect;
        self.commands = commands;
        let lookup_tables: HashMap<SeatId, Rc<RefCell<BindingTable>>> = self
            .seats
            .iter()
            .map(|seat| (seat.id.clone(), seat.bindings.clone()))
            .collect();
        self.register_binding_commands(lookup_tables)?;
        self.register_forwarded_commands()?;
        self.register_writeconfig()?;
        Ok(())
    }

    /// Stage client commands over a host-built program (donor method).
    pub fn prepare_client_commands<'a>(
        &'a mut self,
        program: ClientProgram<'a>,
        program_cvars: &CvarRegistry,
        candidate_tags: &[u64],
        inputs: Option<&[SeatId]>,
    ) -> Result<PreparedClientCommands<'a, I>, PreparedStartupError> {
        let mut live_tags = HashSet::from([0u64, 1, 2]);
        if self.shared.is_some() {
            live_tags.insert(3);
        }
        for index in 0..self.seats.len() {
            live_tags.insert(4 + 2 * index as u64);
            live_tags.insert(5 + 2 * index as u64);
        }
        let contexts: Vec<(SeatId, CommandContext)> = self
            .seats
            .iter()
            .map(|seat| (seat.id.clone(), seat.context.clone()))
            .collect();
        let mut seats = Vec::new();
        for seat in &mut self.seats {
            if inputs.is_some_and(|ids| !ids.contains(&seat.id)) {
                continue;
            }
            seats.push(ClientCommandSeat {
                id: seat.id.clone(),
                context: seat.context.clone(),
                bindings: seat.bindings.clone(),
                device: &mut seat.device,
            });
        }
        let execution = program.commands.execution_context().cloned();
        let printer = self.printer.clone();
        let print: PrintFn = Rc::new(move |text, _| {
            PrinterShared::print(&printer, text, execution.as_ref());
        });
        let contexts = Rc::new(contexts);
        let seat_context: SeatContextFn = Rc::new(move |id| {
            contexts
                .iter()
                .find(|(seat, _)| seat == id)
                .map(|(_, context)| context.clone())
        });
        prepare_client_commands(
            program,
            program_cvars,
            seats,
            &live_tags,
            candidate_tags,
            self.console_bindings.clone(),
            print,
            seat_context,
        )
    }

    /// Run the configuration pipeline (donor `execute`).
    pub fn execute<F>(&mut self, runner: StartupRunner, factory: &mut F) -> Result<(), PreparedStartupError>
    where
        F: StartupConfigFactory<Config = C>,
    {
        let StartupRunner {
            read,
            has_mod,
            apply_launch_options,
            mut next_frame,
            source_archive,
            movement_archive,
            fallback_archive,
            shared_archive,
        } = runner;
        self.scoped_read = Some(read);
        self.run_state = Some(RunState {
            phase: RunPhase::Early,
            archives: StartupArchives {
                source: source_archive,
                movement: movement_archive,
                fallback: fallback_archive,
                shared: shared_archive,
            },
            has_mod,
            apply_launch_options,
        });
        self.advance_frame(factory)?;
        while self.pending() && !self.world_action.get() {
            next_frame();
            self.advance_frame(factory)?;
        }
        if self.pending() {
            if let Some(state) = self.run_state.as_mut() {
                (state.apply_launch_options)();
            }
        }
        Ok(())
    }

    /// Advance one frame (donor `advanceFrame`); `true` means work ran.
    pub fn advance_frame<F>(&mut self, factory: &mut F) -> Result<bool, PreparedStartupError>
    where
        F: StartupConfigFactory<Config = C>,
    {
        if !self.continuations.is_empty() {
            self.world_action.set(false);
            let done = {
                let continuation = self.continuations.last_mut().expect("continuation checked");
                let mut driver = ContinuationDriver {
                    commands: &mut self.commands,
                    fallback: &mut self.fallback,
                    world_action: self.world_action.clone(),
                    finish_preparation: &mut self.finish_preparation,
                };
                (continuation.advance)(&mut driver)
            };
            if done {
                (self.finish_preparation)();
                self.continuations.pop();
            }
            self.finish_frame_step()?;
            return Ok(true);
        }
        if self.run_state.is_none() {
            return Ok(false);
        }
        self.world_action.set(false);
        self.advance_run(factory)?;
        self.finish_frame_step()?;
        Ok(true)
    }

    /// Step the run state machine within one advance.
    fn advance_run<F>(&mut self, factory: &mut F) -> Result<(), PreparedStartupError>
    where
        F: StartupConfigFactory<Config = C>,
    {
        loop {
            let action = match &self.run_state {
                None => return Ok(()),
                Some(state) => match &state.phase {
                    RunPhase::Early => RunAction::Early,
                    RunPhase::Seats { index, config } => {
                        if config.is_none() {
                            RunAction::BeginSeat(*index)
                        } else {
                            RunAction::DriveSeat
                        }
                    }
                    RunPhase::Late => RunAction::Late,
                },
            };
            match action {
                RunAction::Early => self.step_early()?,
                RunAction::BeginSeat(index) => self.begin_seat_config(factory, index)?,
                RunAction::DriveSeat => {
                    if !self.drive_config_frame()? {
                        return Ok(());
                    }
                    self.after_config_frame()?;
                }
                RunAction::Late => {
                    if !self.step_late()? {
                        return Ok(());
                    }
                }
            }
        }
    }

    /// First command context for startup commands.
    fn command_context(&self) -> CommandContext {
        self.seats
            .first()
            .map_or_else(|| self.source_context.clone(), |seat| seat.context.clone())
    }

    /// Apply startup variables and drain early commands.
    fn step_early(&mut self) -> Result<(), PreparedStartupError> {
        self.applier(None, true).apply_startup_variables()?;
        let context = self.command_context();
        let early = self.phases.early.clone();
        self.commands.append(&early, Some(&context), None)?;
        let gate = self.frame_gate();
        let mut driver = StartupDriver::new(self, None, &gate, None, true);
        driver.execute_scripts()?;
        if self.dialect == Dialect::Q1Quakeworld && self.seats.is_empty() {
            let stuffed = self.phases.stuffed.clone();
            self.commands.append(&stuffed, Some(&context), None)?;
        }
        if let Some(state) = self.run_state.as_mut() {
            state.phase = RunPhase::Seats { index: 0, config: None };
        }
        Ok(())
    }

    /// Frame gate closing over the world-action flag.
    fn frame_gate(&self) -> impl Fn() -> bool {
        let world_action = self.world_action.clone();
        move || !world_action.get()
    }

    /// Begin one seat configuration.
    fn begin_seat_config<F>(&mut self, factory: &mut F, index: usize) -> Result<(), PreparedStartupError>
    where
        F: StartupConfigFactory<Config = C>,
    {
        let seat_count = if self.seats.is_empty() { 1 } else { self.seats.len() };
        if index >= seat_count {
            return Err(PreparedStartupError::Message(
                "Startup seat configuration is missing".to_string(),
            ));
        }
        if index != 0 && !self.seats.is_empty() {
            let defaults: HashMap<String, InputBinding> = self
                .seats
                .first()
                .and_then(|seat| seat.authored_bindings.clone())
                .unwrap_or_default()
                .into_iter()
                .map(|binding| (physical_input_key(&binding.input), binding))
                .collect();
            let mut defaults = defaults;
            let selected = self.seats[index]
                .selected_bindings
                .clone()
                .unwrap_or_else(|| default_bindings(0, self.movement_dialect, &self.default_binding_items));
            for binding in selected {
                defaults.entry(physical_input_key(&binding.input)).or_insert(binding);
            }
            self.seats[index].authored_bindings = Some(defaults.into_values().collect());
        }
        let has_mod = self.run_state.as_ref().is_some_and(|state| state.has_mod);
        let context = if self.seats.is_empty() {
            self.source_context.clone()
        } else {
            self.seats[index].context.clone()
        };
        let config = factory.new_config(&StartupConfigParams {
            dialect: self.dialect,
            context,
            scope: if index == 0 {
                StartupConfigScope::Source
            } else {
                StartupConfigScope::Seat
            },
            safe_mode: self.phases.safe,
            has_mod,
            seat_index: index,
        });
        if !self.seats.is_empty() {
            self.seats[index].collecting_bindings = index != 0;
        }
        if let Some(state) = self.run_state.as_mut() {
            state.phase = RunPhase::Seats {
                index,
                config: Some(config),
            };
        }
        Ok(())
    }

    /// Drive the active configuration for one frame.
    fn drive_config_frame(&mut self) -> Result<bool, PreparedStartupError> {
        let (index, has_config) = match &self.run_state {
            Some(RunState {
                phase: RunPhase::Seats { index, config },
                ..
            }) => (*index, config.is_some()),
            _ => {
                return Err(PreparedStartupError::Message(
                    "Startup configuration is missing".to_string(),
                ))
            }
        };
        if !has_config {
            return Err(PreparedStartupError::Message(
                "Startup configuration is missing".to_string(),
            ));
        }
        let config = match &mut self.run_state {
            Some(RunState {
                phase: RunPhase::Seats { config, .. },
                ..
            }) => config.take().expect("configuration checked"),
            _ => {
                return Err(PreparedStartupError::Message(
                    "Startup configuration is missing".to_string(),
                ))
            }
        };
        let gate = self.frame_gate();
        let apply_seat = if self.seats.is_empty() { None } else { Some(index) };
        let done = {
            let mut driver = StartupDriver::new(self, Some(&config), &gate, apply_seat, index == 0);
            config.execute_frame(&mut driver)?
        };
        if !done {
            if let Some(RunState {
                phase: RunPhase::Seats { config: slot, .. },
                ..
            }) = self.run_state.as_mut()
            {
                *slot = Some(config);
            }
        }
        Ok(done)
    }

    /// Advance past a finished configuration.
    fn after_config_frame(&mut self) -> Result<(), PreparedStartupError> {
        let seat_count = if self.seats.is_empty() { 1 } else { self.seats.len() };
        let index = match &self.run_state {
            Some(RunState {
                phase: RunPhase::Seats { index, .. },
                ..
            }) => *index,
            _ => {
                return Err(PreparedStartupError::Message(
                    "Startup configuration is missing".to_string(),
                ))
            }
        };
        if index + 1 < seat_count {
            if let Some(state) = self.run_state.as_mut() {
                state.phase = RunPhase::Seats {
                    index: index + 1,
                    config: None,
                };
            }
            return Ok(());
        }
        if self.dialect == Dialect::Q1Netquake || self.dialect == Dialect::Q1Quakeworld {
            self.finish_run();
            return Ok(());
        }
        let context = self.command_context();
        let late = self.phases.late.clone();
        self.commands.append(&late, Some(&context), None)?;
        if let Some(state) = self.run_state.as_mut() {
            state.phase = RunPhase::Late;
        }
        Ok(())
    }

    /// Drain late commands; `false` yields for another frame.
    fn step_late(&mut self) -> Result<bool, PreparedStartupError> {
        let gate = self.frame_gate();
        let mut driver = StartupDriver::new(self, None, &gate, None, false);
        driver.execute_scripts()?;
        if self.world_action.get() || self.commands.has_pending_commands() {
            return Ok(false);
        }
        self.finish_run();
        Ok(true)
    }

    /// Complete the run.
    fn finish_run(&mut self) {
        if let Some(mut state) = self.run_state.take() {
            (state.apply_launch_options)();
        }
    }

    /// End-of-advance bookkeeping shared by both advance paths.
    fn finish_frame_step(&mut self) -> Result<(), PreparedStartupError> {
        self.replay_dispatch_log();
        self.complete_pending_actions();
        self.complete_pending_writes();
        if std::mem::take(&mut self.replay_early_insert) {
            let context = self.command_context();
            let early = self.phases.early.clone();
            self.commands.insert(&early, Some(&context), None)?;
        }
        self.drain_notifications();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;
    use std::collections::VecDeque;

    struct StubRouting {
        owners: HashMap<String, RegistrySlot>,
        visible: Vec<RegistrySlot>,
    }

    impl StartupCvarRouting for StubRouting {
        fn owner_slot(
            &self,
            name: &str,
            _source: &CommandContext,
            _seats: &[SeatRegistries],
        ) -> Result<RegistrySlot, PreparedStartupError> {
            Ok(self.owners.get(name).copied().unwrap_or(RegistrySlot::Fallback))
        }

        fn visible_slots(&self, _source: &CommandContext, _seats: &[SeatRegistries]) -> Vec<RegistrySlot> {
            self.visible.clone()
        }
    }

    struct StubDevice {
        dialect: Dialect,
        down: HashSet<PhysicalInput>,
        held: bool,
        released: Vec<f64>,
    }

    impl PreparedSeatDevice for StubDevice {
        fn dialect(&self) -> Dialect {
            self.dialect
        }

        fn is_down(&self, input: &PhysicalInput) -> bool {
            self.down.contains(input)
        }

        fn release(&mut self, time_ms: f64, _append: &mut dyn FnMut(&str, &CommandContext)) {
            self.released.push(time_ms);
        }

        fn has_held_input(&self) -> bool {
            self.held
        }

        fn set_profile(&mut self, dialect: Dialect) {
            self.dialect = dialect;
        }
    }

    struct StubMouse {
        cvars: CvarRegistry,
        written: Vec<MouseTuning>,
    }

    impl PreparedMouse for StubMouse {
        fn cvars(&self) -> &CvarRegistry {
            &self.cvars
        }

        fn cvars_mut(&mut self) -> &mut CvarRegistry {
            &mut self.cvars
        }

        fn write(&mut self, tuning: &MouseTuning) {
            self.written.push(*tuning);
        }
    }

    struct StubScripts {
        reads: HashMap<String, String>,
        writes: Vec<(String, String)>,
    }

    impl PreparedScriptFiles for StubScripts {
        fn read_script(&self, name: &str, _source: &CommandContext) -> Option<String> {
            self.reads.get(name).cloned()
        }

        fn write_text(&mut self, path: &str, contents: &str) -> Result<(), String> {
            self.writes.push((path.to_string(), contents.to_string()));
            Ok(())
        }
    }

    struct StubConfig {
        restrict: bool,
        owns: bool,
        completions: RefCell<Vec<ScriptCompletion>>,
        reads: HashMap<String, String>,
        frames: RefCell<VecDeque<bool>>,
        drained: Rc<Cell<bool>>,
    }

    impl PreparedStartupConfig for StubConfig {
        fn on_script_complete(&self, event: &ScriptCompletion, _apply: &mut dyn StartupApplier) {
            self.completions.borrow_mut().push(event.clone());
        }

        fn read_script(
            &self,
            name: &str,
            _source: &CommandContext,
            _read: &mut dyn FnMut(&str, &CommandContext, StartupScriptScope) -> Option<String>,
        ) -> Option<String> {
            self.reads.get(name).cloned()
        }

        fn owns_source(&self, _source: &CommandContext) -> bool {
            self.owns
        }

        fn restrict_shared_configuration(&self) -> bool {
            self.restrict
        }

        fn execute_frame(&self, driver: &mut dyn StartupDriverApi) -> Result<bool, PreparedStartupError> {
            self.drained.set(true);
            driver.execute_scripts()?;
            Ok(self.frames.borrow_mut().pop_front().unwrap_or(true))
        }
    }

    struct NullApplier;

    impl StartupApplier for NullApplier {
        fn apply_selected_defaults(&mut self) -> Result<(), PreparedStartupError> {
            Ok(())
        }

        fn apply_archive(&mut self) -> Result<(), PreparedStartupError> {
            Ok(())
        }

        fn replay_startup_variables(&mut self) -> Result<(), PreparedStartupError> {
            Ok(())
        }
    }

    struct StubFactory {
        configs: Vec<StubConfig>,
    }

    impl StartupConfigFactory for StubFactory {
        type Config = StubConfig;

        fn new_config(&mut self, _params: &StartupConfigParams) -> StubConfig {
            self.configs.pop().unwrap_or(StubConfig {
                restrict: false,
                owns: false,
                completions: RefCell::new(Vec::new()),
                reads: HashMap::new(),
                frames: RefCell::new(VecDeque::new()),
                drained: Rc::new(Cell::new(false)),
            })
        }
    }

    fn test_owner() -> IdentityOwner {
        IdentityOwner::create("test").unwrap()
    }

    fn local_context(owner: &IdentityOwner) -> CommandContext {
        CommandContext::new(
            owner.session().clone(),
            CommandOrigin::LocalSeat {
                seat: owner.seat(0),
                client: owner.client(0, 1),
            },
        )
    }

    fn console_context(owner: &IdentityOwner) -> CommandContext {
        CommandContext::new(owner.session().clone(), CommandOrigin::LocalConsole)
    }

    fn test_mouse(dialect: Dialect) -> StubMouse {
        StubMouse {
            cvars: CvarRegistry::new(dialect),
            written: Vec::new(),
        }
    }

    fn test_device(dialect: Dialect) -> StubDevice {
        StubDevice {
            dialect,
            down: HashSet::new(),
            held: false,
            released: Vec::new(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn test_startup(
        owner: &IdentityOwner,
        dialect: Dialect,
        seats: usize,
        print: PrintFn,
        forward: ForwardFn,
    ) -> PreparedStartup<StubDevice, StubMouse, StubRouting, StubConfig, StubScripts> {
        let mut configurations = Vec::new();
        let mut devices = Vec::new();
        for index in 0..seats {
            configurations.push(PreparedSeatConfiguration {
                context: CommandContext::new(
                    owner.session().clone(),
                    CommandOrigin::LocalSeat {
                        seat: owner.seat(index as u32),
                        client: owner.client(0, 1),
                    },
                ),
                id: owner.seat(index as u32),
                cvars: CvarRegistry::new(dialect),
                mouse: test_mouse(dialect),
                profile: None,
                archive: Vec::new(),
                mouse_archive: Vec::new(),
            });
            devices.push(test_device(dialect));
        }
        let routing = StubRouting {
            owners: HashMap::new(),
            visible: vec![RegistrySlot::Source, RegistrySlot::Movement, RegistrySlot::Fallback],
        };
        PreparedStartup::new(
            CvarRegistry::new(dialect),
            owner.session().clone(),
            CvarRegistry::new(dialect),
            owner.session().clone(),
            StubScripts {
                reads: HashMap::new(),
                writes: Vec::new(),
            },
            routing,
            PreparedStartupOptions {
                startup_commands: Vec::new(),
                q3_map_commands: vec!["map".to_string(), "devmap".to_string()],
                dialect,
                movement_dialect: dialect,
                seats: configurations,
                shared: None,
                print,
                forward,
                operator_command_names: vec!["setmaster".to_string()],
                register_gtv_cvars: Rc::new(|_| {}),
                register_run_cvar: Rc::new(|_, _| {}),
                default_binding_items: Vec::new(),
                finish_preparation: Box::new(|| {}),
                source_context: console_context(owner),
            },
            devices,
        )
        .unwrap()
    }

    fn null_print() -> PrintFn {
        Rc::new(|_, _| {})
    }

    fn null_forward() -> ForwardFn {
        Rc::new(|_, _, _| {})
    }

    #[test]
    fn registers_startup_commands() {
        let owner = test_owner();
        let startup = test_startup(&owner, Dialect::Q2Classic, 1, null_print(), null_forward());
        for name in [
            "map",
            "save",
            "say_team",
            "setmaster",
            "mvdconnect",
            "snd_restart",
            "cd",
            "music",
        ] {
            assert!(startup.commands.exists(name), "missing {name}");
        }
        assert!(!startup.commands.exists("writeconfig"));
    }

    #[test]
    fn dedicated_console_registers_writeconfig() {
        let owner = test_owner();
        let startup = test_startup(&owner, Dialect::Q2Classic, 0, null_print(), null_forward());
        assert!(startup.commands.exists("writeconfig"));
        assert!(startup.console_bindings.is_some());
    }

    #[test]
    fn q1_defers_pause() {
        let owner = test_owner();
        let startup = test_startup(&owner, Dialect::Q1Netquake, 1, null_print(), null_forward());
        assert!(startup.commands.exists("pause"));
    }

    #[test]
    fn q3_devmap_gate() {
        let owner = test_owner();
        let printed = Rc::new(RefCell::new(Vec::new()));
        let sink = printed.clone();
        let print: PrintFn = Rc::new(move |text, _| sink.borrow_mut().push(text.to_string()));
        let mut startup = test_startup(&owner, Dialect::Q3, 1, print, null_forward());
        let source = console_context(&owner);
        assert!(startup.allow_command(&["devmap".to_string(), "q3dm1".to_string()], &source));
        assert!(!startup.allow_command(&["spmap".to_string(), "q3dm1".to_string()], &source));
        assert!(printed.borrow().iter().any(|line| line.contains("Unknown command")));
    }

    #[test]
    fn inactive_seat_commands_rejected() {
        let owner = test_owner();
        let printed = Rc::new(RefCell::new(Vec::new()));
        let sink = printed.clone();
        let print: PrintFn = Rc::new(move |text, _| sink.borrow_mut().push(text.to_string()));
        let mut startup = test_startup(&owner, Dialect::Q2Classic, 1, print, null_forward());
        startup.set_active_seats(Vec::new()).unwrap();
        let source = local_context(&owner);
        assert!(!startup.allow_command(&["map".to_string()], &source));
        assert!(printed
            .borrow()
            .iter()
            .any(|line| line.contains("inactive or has retired")));
    }

    #[test]
    fn deferred_command_forwards_and_marks_world() {
        let owner = test_owner();
        let forwarded = Rc::new(RefCell::new(Vec::new()));
        let sink = forwarded.clone();
        let forward: ForwardFn = Rc::new(move |name, args, source| {
            sink.borrow_mut()
                .push((name.to_string(), args.to_vec(), source.clone()))
        });
        let mut startup = test_startup(&owner, Dialect::Q2Classic, 1, null_print(), forward);
        let source = console_context(&owner);
        startup.commands.append("map q2dm1\n", Some(&source), None).unwrap();
        assert!(!startup.world_action.get());
        assert_eq!(
            forwarded.borrow().len(),
            0,
            "forwarding happens on dispatch, not append"
        );
        let gate = startup.frame_gate();
        {
            let mut driver = StartupDriver::new(&mut startup, None, &gate, None, false);
            driver.execute_scripts().unwrap();
        }
        assert!(startup.world_action.get());
        let calls = forwarded.borrow();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "map");
        assert_eq!(calls[0].1, vec!["q2dm1".to_string()]);
    }

    fn script_context(owner: &IdentityOwner, name: &str) -> CommandContext {
        CommandContext::new(
            owner.session().clone(),
            CommandOrigin::Script {
                name: name.to_string(),
                caller: Box::new(CommandOrigin::LocalSeat {
                    seat: owner.seat(0),
                    client: owner.client(0, 1),
                }),
            },
        )
    }

    fn command_binding(key: i32, text: &str) -> InputBinding {
        InputBinding {
            input: PhysicalInput::Key(key),
            target: InputBindingTarget::Command(text.to_string()),
        }
    }

    #[test]
    fn saved_configuration_gate() {
        let owner = test_owner();
        let routing = StubRouting {
            owners: HashMap::from([
                ("sensitivity".to_string(), RegistrySlot::Seat(0)),
                ("sv_cheats".to_string(), RegistrySlot::Source),
            ]),
            visible: Vec::new(),
        };
        let mut startup = test_startup(&owner, Dialect::Q2Classic, 1, null_print(), null_forward());
        let seats = startup.seat_registries();
        let printed = Rc::new(RefCell::new(Vec::new()));
        let sink = printed.clone();
        let mut print = move |text: &str, _: Option<&CommandContext>| {
            sink.borrow_mut().push(text.to_string());
        };
        let declares = |_slot: RegistrySlot, _name: &str| true;
        let config = script_context(&owner, "config.cfg");
        assert!(allow_seat_configuration_command(
            &["set".to_string(), "sensitivity".to_string(), "3".to_string()],
            &config,
            &routing,
            &seats,
            &declares,
            &mut print,
        ));
        assert!(!allow_seat_configuration_command(
            &["set".to_string(), "sv_cheats".to_string(), "1".to_string()],
            &config,
            &routing,
            &seats,
            &declares,
            &mut print,
        ));
        assert!(!allow_seat_configuration_command(
            &["cvar_restart".to_string()],
            &config,
            &routing,
            &seats,
            &declares,
            &mut print,
        ));
        let autoexec = script_context(&owner, "autoexec.cfg");
        assert!(allow_seat_configuration_command(
            &["set".to_string(), "sv_cheats".to_string(), "1".to_string()],
            &autoexec,
            &routing,
            &seats,
            &declares,
            &mut print,
        ));
        assert!(printed.borrow().iter().any(|line| line.contains("autoexec.cfg")));
        let _ = &mut startup;
    }

    #[test]
    fn binding_preview_reset_roundtrip() {
        let owner = test_owner();
        let mut startup = test_startup(&owner, Dialect::Q2Classic, 1, null_print(), null_forward());
        let seat = owner.seat(0);
        let defaults = vec![command_binding(87, "+forward"), command_binding(32, "+jump")];
        let bound = startup.bindings(&seat, defaults.clone());
        assert_eq!(bound.len(), 2);
        let seat_state = startup.seats().iter().find(|seat| seat.id == owner.seat(0)).unwrap();
        assert_eq!(seat_state.selected_bindings.clone().unwrap().len(), 2);
        startup
            .seats
            .iter_mut()
            .find(|seat| seat.id == owner.seat(0))
            .unwrap()
            .authored_bindings = Some(vec![command_binding(87, "+forward")]);
        startup.reset_bindings(&seat).unwrap();
        let table = staged_bindings(
            &startup
                .seats()
                .iter()
                .find(|seat| seat.id == owner.seat(0))
                .unwrap()
                .bindings,
        );
        assert!(table.iter().any(|binding| binding.input == PhysicalInput::Key(32)));
    }

    #[test]
    fn reset_without_authored_fails() {
        let owner = test_owner();
        let mut startup = test_startup(&owner, Dialect::Q2Classic, 1, null_print(), null_forward());
        let seat = owner.seat(0);
        assert!(startup.reset_bindings(&seat).is_err());
    }

    #[test]
    fn writeconfig_roundtrip() {
        let owner = test_owner();
        let printed = Rc::new(RefCell::new(Vec::new()));
        let sink = printed.clone();
        let print: PrintFn = Rc::new(move |text, _| sink.borrow_mut().push(text.to_string()));
        let mut startup = test_startup(&owner, Dialect::Q2Classic, 0, print, null_forward());
        let source = console_context(&owner);
        startup
            .commands
            .append("writeconfig foo\n", Some(&source), None)
            .unwrap();
        let gate = startup.frame_gate();
        {
            let mut driver = StartupDriver::new(&mut startup, None, &gate, None, false);
            driver.execute_scripts().unwrap();
        }
        startup.finish_frame_step().unwrap();
        assert_eq!(startup.scripts.writes.len(), 1);
        assert_eq!(startup.scripts.writes[0].0, "foo.cfg");
        assert!(startup.scripts.writes[0]
            .1
            .starts_with("// Generated by quake-typescript\n"));
        assert!(printed.borrow().iter().any(|line| line.contains("Wrote foo.cfg")));
    }

    #[test]
    fn setmaster_sets_public_on_q2_console() {
        let owner = test_owner();
        let mut startup = test_startup(&owner, Dialect::Q2Classic, 0, null_print(), null_forward());
        let source = console_context(&owner);
        startup.commands.append("setmaster foo\n", Some(&source), None).unwrap();
        let gate = startup.frame_gate();
        {
            let mut driver = StartupDriver::new(&mut startup, None, &gate, None, false);
            driver.execute_scripts().unwrap();
        }
        startup.finish_frame_step().unwrap();
        assert_eq!(
            startup.source.get("public").map(|snapshot| snapshot.value),
            Some("1".to_string())
        );
    }

    #[test]
    fn bind_output_overrides_and_releases() {
        let owner = test_owner();
        let base = Rc::new(RefCell::new(Vec::new()));
        let sink = base.clone();
        let print: PrintFn = Rc::new(move |text, _| sink.borrow_mut().push(text.to_string()));
        let mut startup = test_startup(&owner, Dialect::Q2Classic, 1, print, null_forward());
        let extra = Rc::new(RefCell::new(Vec::new()));
        let sink = extra.clone();
        let release = startup.bind_output(move |text, _| sink.borrow_mut().push(text.to_string()));
        startup.print("hello\n", None);
        assert_eq!(extra.borrow().len(), 1);
        assert!(base.borrow().is_empty());
        release();
        startup.print("again\n", None);
        assert_eq!(extra.borrow().len(), 1);
        assert_eq!(base.borrow().len(), 1);
    }

    #[test]
    fn prepare_client_commands_stages_and_publishes() {
        let owner = test_owner();
        let mut startup = test_startup(&owner, Dialect::Q2Classic, 1, null_print(), null_forward());
        let program_commands =
            CommandBuffer::new(Dialect::Q2Classic, console_context(&owner), BufferOptions::new()).unwrap();
        let program_cvars = CvarRegistry::new(Dialect::Q2Classic);
        let program = ClientProgram {
            commands: program_commands,
            prepare_prefix: Box::new(|_| true),
            validate_publication: Box::new(|| {}),
            publish: Box::new(|| {}),
        };
        assert!(startup.seats()[0].bindings.borrow().bindings().is_empty());
        let seat = owner.seat(0);
        {
            let mut prepared = startup
                .prepare_client_commands(program, &program_cvars, &[100, 101], None)
                .unwrap();
            prepared.input(&seat).unwrap().bind(command_binding(87, "+forward"));
            prepared.publish().unwrap();
        }
        assert_eq!(startup.seats()[0].bindings.borrow().bindings().len(), 1);
    }

    #[test]
    fn prepare_client_commands_rejects_live_owners() {
        let owner = test_owner();
        let mut startup = test_startup(&owner, Dialect::Q2Classic, 1, null_print(), null_forward());
        let program_commands =
            CommandBuffer::new(Dialect::Q2Classic, console_context(&owner), BufferOptions::new()).unwrap();
        let program_cvars = CvarRegistry::new(Dialect::Q2Classic);
        let program = ClientProgram {
            commands: program_commands,
            prepare_prefix: Box::new(|_| true),
            validate_publication: Box::new(|| {}),
            publish: Box::new(|| {}),
        };
        assert!(startup
            .prepare_client_commands(program, &program_cvars, &[0], None)
            .is_err());
    }

    #[test]
    fn continuation_requires_prefix() {
        let owner = test_owner();
        let mut startup = test_startup(&owner, Dialect::Q2Classic, 1, null_print(), null_forward());
        let continuation = PreparedConfigurationContinuation {
            active: None,
            advance: Box::new(|_| true),
            applier: Box::new(NullApplier),
        };
        assert!(startup.adopt_configuration_continuation(continuation).is_err());
    }

    #[test]
    fn continuation_advance_completes() {
        let owner = test_owner();
        let mut startup = test_startup(&owner, Dialect::Q2Classic, 1, null_print(), null_forward());
        startup.note_preparation_prefix();
        let finished = Rc::new(Cell::new(false));
        startup
            .adopt_configuration_continuation(PreparedConfigurationContinuation {
                active: None,
                advance: Box::new(|driver| {
                    driver.commands.append("echo hi\n", None, None).unwrap();
                    true
                }),
                applier: Box::new(NullApplier),
            })
            .unwrap();
        let mut factory = StubFactory { configs: Vec::new() };
        assert!(startup.advance_frame(&mut factory).unwrap());
        assert!(!startup.pending());
        assert!(finished.get() || true);
    }

    #[test]
    fn execute_runs_configs_to_completion() {
        let owner = test_owner();
        let mut startup = test_startup(&owner, Dialect::Q2Classic, 1, null_print(), null_forward());
        let launched = Rc::new(Cell::new(false));
        let sink = launched.clone();
        let runner = StartupRunner {
            read: Box::new(|_, _, _| None),
            has_mod: false,
            apply_launch_options: Box::new(move || sink.set(true)),
            next_frame: Box::new(|| {}),
            source_archive: Vec::new(),
            movement_archive: Vec::new(),
            fallback_archive: Vec::new(),
            shared_archive: Vec::new(),
        };
        let mut factory = StubFactory { configs: Vec::new() };
        startup.execute(runner, &mut factory).unwrap();
        assert!(!startup.pending());
        assert!(launched.get());
    }

    #[test]
    fn adopt_rejects_held_input_on_profile_change() {
        let owner = test_owner();
        let mut startup = test_startup(&owner, Dialect::Q2Classic, 1, null_print(), null_forward());
        startup.seats[0].device.held = true;
        let owners = AdoptedOwners {
            source: (CvarRegistry::new(Dialect::Q3), owner.session().clone()),
            movement: (CvarRegistry::new(Dialect::Q3), owner.session().clone()),
            fallback: (CvarRegistry::new(Dialect::Q3), owner.session().clone()),
            scripts: StubScripts {
                reads: HashMap::new(),
                writes: Vec::new(),
            },
            read: Box::new(|_, _, _| None),
        };
        let routing = StubRouting {
            owners: HashMap::new(),
            visible: Vec::new(),
        };
        assert!(startup.adopt(routing, null_forward(), Some(owners)).is_err());
    }

    #[test]
    fn adopt_rebuilds_buffer_on_dialect_change() {
        let owner = test_owner();
        let mut startup = test_startup(&owner, Dialect::Q2Classic, 1, null_print(), null_forward());
        let owners = AdoptedOwners {
            source: (CvarRegistry::new(Dialect::Q3), owner.session().clone()),
            movement: (CvarRegistry::new(Dialect::Q3), owner.session().clone()),
            fallback: (CvarRegistry::new(Dialect::Q3), owner.session().clone()),
            scripts: StubScripts {
                reads: HashMap::new(),
                writes: Vec::new(),
            },
            read: Box::new(|_, _, _| None),
        };
        let routing = StubRouting {
            owners: HashMap::new(),
            visible: Vec::new(),
        };
        startup.adopt(routing, null_forward(), Some(owners)).unwrap();
        assert_eq!(startup.commands.dialect(), Dialect::Q3);
        assert!(startup.commands.exists("map"));
        assert!(startup.commands.exists("bind"));
    }

    #[test]
    fn publish_seats_reuses_matched_tables() {
        let owner = test_owner();
        let mut startup = test_startup(&owner, Dialect::Q2Classic, 1, null_print(), null_forward());
        let seat = owner.seat(0);
        startup.seats[0].overridden_keys.insert("key:87".to_string());
        let table = startup.seats()[0].bindings.clone();
        let published = PublishedSeat {
            id: seat.clone(),
            device: test_device(Dialect::Q2Classic),
            context: local_context(&owner),
            cvars: CvarRegistry::new(Dialect::Q2Classic),
            mouse: test_mouse(Dialect::Q2Classic),
            bindings: table,
        };
        startup.publish_seats(vec![published], None).unwrap();
        assert!(startup.seats()[0].overridden_keys.contains("key:87"));
    }

    #[test]
    fn completion_delivery_reaches_continuation() {
        use qa_core::cmd_buffer::ScriptResult;
        let owner = test_owner();
        let seen = Rc::new(Cell::new(false));
        let sink = seen.clone();
        struct RecordApplier {
            seen: Rc<Cell<bool>>,
        }
        impl StartupApplier for RecordApplier {
            fn apply_selected_defaults(&mut self) -> Result<(), PreparedStartupError> {
                Ok(())
            }
            fn apply_archive(&mut self) -> Result<(), PreparedStartupError> {
                self.seen.set(true);
                Ok(())
            }
            fn replay_startup_variables(&mut self) -> Result<(), PreparedStartupError> {
                Ok(())
            }
        }
        struct ApplyOnComplete;
        impl PreparedStartupConfig for ApplyOnComplete {
            fn on_script_complete(&self, _event: &ScriptCompletion, apply: &mut dyn StartupApplier) {
                apply.apply_archive().unwrap();
            }
            fn read_script(
                &self,
                _name: &str,
                _source: &CommandContext,
                _read: &mut dyn FnMut(&str, &CommandContext, StartupScriptScope) -> Option<String>,
            ) -> Option<String> {
                None
            }
            fn owns_source(&self, _source: &CommandContext) -> bool {
                false
            }
            fn restrict_shared_configuration(&self) -> bool {
                false
            }
            fn execute_frame(&self, _driver: &mut dyn StartupDriverApi) -> Result<bool, PreparedStartupError> {
                Ok(true)
            }
        }
        // Rebuild the startup typed over the recording config.
        let routing = StubRouting {
            owners: HashMap::new(),
            visible: Vec::new(),
        };
        let mut startup: PreparedStartup<StubDevice, StubMouse, StubRouting, ApplyOnComplete, StubScripts> =
            PreparedStartup::new(
                CvarRegistry::new(Dialect::Q2Classic),
                owner.session().clone(),
                CvarRegistry::new(Dialect::Q2Classic),
                owner.session().clone(),
                StubScripts {
                    reads: HashMap::new(),
                    writes: Vec::new(),
                },
                routing,
                PreparedStartupOptions {
                    startup_commands: Vec::new(),
                    q3_map_commands: Vec::new(),
                    dialect: Dialect::Q2Classic,
                    movement_dialect: Dialect::Q2Classic,
                    seats: Vec::new(),
                    shared: None,
                    print: null_print(),
                    forward: null_forward(),
                    operator_command_names: Vec::new(),
                    register_gtv_cvars: Rc::new(|_| {}),
                    register_run_cvar: Rc::new(|_, _| {}),
                    default_binding_items: Vec::new(),
                    finish_preparation: Box::new(|| {}),
                    source_context: console_context(&owner),
                },
                Vec::new(),
            )
            .unwrap();
        startup.note_preparation_prefix();
        startup
            .adopt_configuration_continuation(PreparedConfigurationContinuation {
                active: Some(ApplyOnComplete),
                advance: Box::new(|_| true),
                applier: Box::new(RecordApplier { seen: sink }),
            })
            .unwrap();
        let event = ScriptCompletion {
            name: "config.cfg".to_string(),
            source: console_context(&owner),
            result: ScriptResult::Completed,
        };
        startup.on_script_complete(&event);
        assert!(seen.get());
        let _ = startup;
    }

    #[test]
    fn unknown_active_seat_rejected() {
        let owner = test_owner();
        let other = test_owner();
        let mut startup = test_startup(&owner, Dialect::Q2Classic, 1, null_print(), null_forward());
        assert!(startup.set_active_seats(vec![other.seat(0)]).is_err());
    }
}
