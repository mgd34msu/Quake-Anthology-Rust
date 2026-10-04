//! Shared simulation vocabulary: options, travel, presentation, and events.
//!
//! Port of donor `src/app/bootstrap/simulation/types.ts`.
//!
//! Conventions used throughout this hub:
//!
//! * Donor `T | null` becomes `Option<T>` and donor `field?: T | null`
//!   collapses absent and null into `None`; no in-lane consumer distinguishes
//!   the two spellings.
//! * Donor `number` becomes `i32` for counts, ordinals, frames, and
//!   millisecond clocks, `u32` for slots and seeds, `u64` for monotonic
//!   sequences, and `f64` for measurements.
//! * Wave 2 unified the in-scope opaque mirrors (`PreparedClassicGuest`,
//!   `PreparedRereleaseGuest`, `PreparedQ3Game`, `PreparedQuakeCSource`,
//!   `NativeQ2Travel`, `ApplicationMonsterNavigation`) into imports of their
//!   real homes, and added `prepareRereleaseNavigation` now that
//!   `super::runtime` exists. Remaining opaque mirrors below stand in for
//!   out-of-scope subtrees nobody in the lane reads (`QvmWeaponProfile`,
//!   `NativeWeaponBehaviorDeclaration`; `WeaponBehaviorComponent` is real,
//!   see below).

use std::collections::HashMap;
use std::rc::Rc;

use qa_bots::behavior::q3::navigation_types::BotMovementPrediction;
use qa_bots::behavior::q3::travel::types::BotTravelPredictionResult;
use qa_bots::runtime::NavigationRuntime;
use qa_bots::scene::Q3WorldGeometry;
use qa_client::materials::fog::Q1FogTransition;
use qa_client::render::scene::models::types::IndexedModelSkin;
use qa_client::text::ui_world::WorldText;
use qa_compat::q2::classic::records::RawEntityView;
use qa_compat::q2::native_primary_inventory::NativeInventoryReadout;
use qa_compat::q2::rerelease::module::RereleaseGuestModule;
use qa_content::bsp::Q1Map;
use qa_content::bsp2::Q2DecodedMap;
use qa_content::contract::{
    ArmorState, ContentId, ExecutableRecipe, GameFamily, HeldWeaponDeclaration, InventoryEntry, ItemId, ModCommands,
    ModDescription, ModIdentity, ModSelection, ModTravelCheckpoint, ModUserFiles, ModuleIdentity,
    PoweredProtectionState, PresentationOwner, ProviderReference, QvmGrappleDefinition, QvmGrappleViewAnchor,
    RegularArmorState, ResolvedResourceReference, ResolvedWeaponBehaviorSelection, ResourceRequest,
};
use qa_content::mounts::MountedContent;
use qa_content::q1::base::rules::Q1IntermissionResult;
use qa_content::q1::base::travel::Q1TravelState;
use qa_content::q1::composition::types::Q1CompositionEvent;
use qa_content::q1::foundation::gameplay::TransitionIntent;
use qa_content::q1::foundation::types::{Q1Event, Q1Weapon};
use qa_content::q2::base::player::types::{Q2PlayerCarry, Q2PlayerEvent};
use qa_content::q2::composition::types::Q2CompositionEvent;
use qa_content::q2::equipment::HandGrenadeEquipmentState;
use qa_content::q2::foundation::host::Q2PresentationEvent;
use qa_content::q2::foundation::weapons::types::{Q2WeaponDefinition, Q2WeaponEvent};
use qa_content::q2::multiplayer::lmctf::types::LmctfTravel;
use qa_content::q2::rerelease::campaign::Q2RereleaseCampaignState;
use qa_content::q2::rerelease::types::Q2RereleaseEvent;
use qa_content::q2::support::contracts::SceneFlare;
use qa_content::q3::foundation::arsenal::Q3ArsenalRuntimeState;
use qa_content::q3::foundation::presentation::Q3CharacterView;
use qa_core::cvar::{CvarArchiveEntry, CvarRegistry};
use qa_core::identity::{ActorId, ClientId, IdentityOwner};
use qa_core::math::{Bounds, Vec3, Vec4};
use qa_guest::core::contracts::GuestAddress;
use qa_guest::qc::program::QcProgram;
use qa_guest::qvm::artifacts::ResolvedQvmArtifact;
use qa_guest::runtime::windows::contracts::WindowsCapabilities;
use qa_net::q2_adapters::{Q2PlayerView, Q2Vec3, Q2Vec4};
use qa_platform::files::writable::UserFileStore;
use qa_world::movement::types::ArsenalState;

use super::arsenal::selected::{ArsenalAmmoWarning, WeaponHudStatus};
use super::arsenal::weapon_status::q2_weapon_status;
use super::powerup_timers::ActivePowerupTimer;
use super::q3::host::Q3SourceEvent;
use super::q3::types::Q3SourceSessionCarry;
use super::q3_ballistics::Q3SharedBallisticEvent;
use super::weapon_slot::WeaponReference;
use crate::debug::DebugLine;
use crate::settings::server::ServerProfile;

// ---------------------------------------------------------------------------
// In-scope mirrors (transient): real homes land in wave 1, wave 2 unifies.
// ---------------------------------------------------------------------------

/// Wave-2 unification: real prepared guests (see the module docs).
pub use super::classic_guest_source::PreparedClassicGuest;
/// Wave-2 unification: real prepared guests (see the module docs).
pub use super::rerelease_guest_source::PreparedRereleaseGuest;

/// Wave-2 unification: real monster navigation (see the module docs).
pub use super::monster_navigation::ApplicationMonsterNavigation;
/// Wave-2 unification: real native Q2 travel (see the module docs).
pub use super::native_q2_travel::NativeQ2Travel;
/// Wave-2 unification: real prepared Q3 game (see the module docs).
pub use super::q3::guest_artifact::PreparedQ3Game;
/// Wave-2 unification: real prepared QuakeC source (see the module docs).
pub use super::quakec_source::PreparedQuakeCSource;

/// Mirror of `HandGrenadeTravel` from donor
/// `src/app/bootstrap/simulation/equipment-runtime.ts` (canonical home:
/// `crate::bootstrap::simulation::equipment_runtime`); unify when it merges.
#[derive(Debug, Clone, PartialEq)]
pub struct HandGrenadeTravel {
    /// Equipment state.
    pub state: HandGrenadeEquipmentState,
    /// Grenade ammo entry.
    pub ammo: InventoryEntry,
    /// Travel time in seconds.
    pub seconds: f64,
}

/// Mirror of `Q1SelectedArsenalTravel` from donor
/// `src/app/bootstrap/simulation/arsenal/q1.ts` (canonical home:
/// `crate::bootstrap::simulation::arsenal::q1`); unify when it merges.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1SelectedArsenalTravel {
    /// Selected weapon.
    pub weapon: Q1Weapon,
    /// Carried inventory.
    pub inventory: Vec<InventoryEntry>,
    /// Opaque mod extensions.
    pub extensions: Option<Vec<ArsenalTravelExtension>>,
}

/// Opaque selected-arsenal travel extension.
#[derive(Debug, Clone, PartialEq)]
pub struct ArsenalTravelExtension {
    /// Extension identity.
    pub id: String,
    /// Extension bytes.
    pub bytes: Vec<u8>,
}

/// Mirror of `Q3SelectedArsenalCheckpoint` from donor
/// `src/app/bootstrap/simulation/arsenal/q3.ts` (canonical home:
/// `crate::bootstrap::simulation::arsenal::q3`); unify when it merges.
#[derive(Debug, Clone, PartialEq)]
pub struct Q3SelectedArsenalCheckpoint {
    /// Active supply profile.
    pub supply_profile: Option<ItemId>,
    /// Arsenal snapshot.
    pub arsenal: ArsenalState,
    /// Q3 runtime state.
    pub runtime: Q3ArsenalRuntimeState,
    /// Torso animation.
    pub torso_animation: f64,
    /// Last fire time in milliseconds.
    pub last_fire_milliseconds: Option<i32>,
    /// Pending weapon use.
    pub pending_use: Option<ItemId>,
}

/// Mirror of `ApplicationBotNavigation` from donor
/// `src/app/bootstrap/simulation/navigation.ts` (canonical home:
/// `crate::bootstrap::simulation::navigation`); unify when it merges.
///
/// Only the `SelectedBotNavigation` fields are mirrored: wave 1 spreads them
/// into the Q3 guest bot host, while the navigation methods are called solely
/// by wave-2 modules that will share a worktree with the real home.
pub struct ApplicationBotNavigation<'w> {
    /// Live navigation runtime.
    pub runtime: NavigationRuntime<'w>,
    /// Per-client runtime.
    pub for_client: Rc<dyn Fn(i32) -> NavigationRuntime<'w> + 'w>,
    /// Crouched hull.
    pub crouched_bounds: Bounds,
    /// Detached client movement prediction.
    pub predict_client_movement: Rc<dyn Fn(BotMovementPrediction) -> BotTravelPredictionResult>,
    /// Jump-pad weapon selection.
    pub travel_weapon: Rc<dyn Fn(i32, BotTravelWeaponMode) -> Option<i32>>,
}

/// Jump-pad weapon mode for bot travel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BotTravelWeaponMode {
    /// Rocket jump.
    RocketJump,
    /// BFG jump.
    BfgJump,
    /// Grapple.
    Grapple,
}

// ---------------------------------------------------------------------------
// Out-of-scope mirrors (canonical): homes live outside the sim lane.
// ---------------------------------------------------------------------------

/// QVM calendar, mirroring donor `QvmCalendar`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QvmCalendar {
    /// Second.
    pub second: i32,
    /// Minute.
    pub minute: i32,
    /// Hour.
    pub hour: i32,
    /// Day of month.
    pub day: i32,
    /// Month.
    pub month: i32,
    /// Year.
    pub year: i32,
    /// Weekday.
    pub weekday: i32,
    /// Day of year.
    pub year_day: i32,
    /// Daylight-saving flag.
    pub is_dst: i32,
}

/// QVM `realTime` trap, mirroring donor `QvmCommonServices["qagame"]["realTime"]`.
pub type QvmRealTime = Rc<dyn Fn(Option<Rc<dyn Fn(&QvmCalendar)>>) -> i32>;

/// QVM immediate command execution.
pub type QvmExecuteNow = Rc<dyn Fn(Option<&str>)>;

/// QVM console commands, mirroring donor `ConsoleCommands`. The donor
/// `executeNow` may complete asynchronously; callers manage completion and
/// this mirror reports dispatch only.
pub struct QvmCommonCommands {
    /// Execute a command now.
    pub execute_now: QvmExecuteNow,
    /// Insert a command.
    pub insert: Rc<dyn Fn(&str)>,
    /// Append a command.
    pub append: Rc<dyn Fn(&str)>,
}

/// Q3 guest common services (`writable`/`common` projection of donor
/// `Q3GuestRuntimeOptions`).
pub struct Q3GuestCommonServices {
    /// Millisecond clock.
    pub milliseconds: Rc<dyn Fn() -> i32>,
    /// Calendar trap.
    pub real_time: QvmRealTime,
    /// Console commands.
    pub commands: QvmCommonCommands,
}

/// Rerelease clipboard selection, mirroring donor `RereleaseClipboard`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RereleaseClipboard {
    /// System clipboard.
    System,
    /// Internal clipboard.
    Internal,
}

/// Rerelease actor bindings, mirroring donor `RereleaseActorBindings` from
/// `src/compat/q2/rerelease/host.ts`. Nobody in the lane reads past the
/// handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct RereleaseActorBindings;

/// Entity-view world projection.
pub type SemanticProject = Rc<dyn Fn(&RawEntityView) -> Option<Vec3>>;
/// Entity-view generation tag.
pub type SemanticGeneration = Rc<dyn Fn(&RawEntityView) -> Option<String>>;
/// Foreign engine module adoption.
pub type SemanticBound = Rc<dyn Fn(&RereleaseGuestModule) -> Option<RereleaseGuestModule>>;
/// Entity-view engine binding.
pub type SemanticBind = Rc<dyn Fn(&RereleaseGuestModule, &RawEntityView) -> RereleaseActorBindings>;
/// Engine module foreign address.
pub type SemanticForeignAddress = Rc<dyn Fn(&RereleaseGuestModule) -> GuestAddress>;

/// Rerelease semantic bindings, mirroring donor `RereleaseSemanticBindings`
/// from `src/compat/q2/rerelease/host.ts`.
#[derive(Clone)]
pub struct RereleaseSemanticBindings {
    /// Project an entity view to world space.
    pub project: Option<SemanticProject>,
    /// Entity generation tag.
    pub generation: Option<SemanticGeneration>,
    /// Adopt a foreign engine module.
    pub bound: Option<SemanticBound>,
    /// Bind an entity view to the engine.
    pub bind: SemanticBind,
    /// Foreign address of an engine module.
    pub foreign_address: SemanticForeignAddress,
}

/// Guest string localization with substitutions.
pub type GuestLocalize = Rc<dyn Fn(&str, &[String]) -> String>;

/// Local services projection of donor `RereleaseGuestServicesOptions`
/// (`localize`/`clipboard`/`semanticBindings`).
pub struct RereleaseGuestLocalServices {
    /// Localize a string with substitutions.
    pub localize: GuestLocalize,
    /// Clipboard selection.
    pub clipboard: RereleaseClipboard,
    /// Semantic bindings.
    pub semantic_bindings: Option<RereleaseSemanticBindings>,
}

/// Grapple inventory binding, mirroring donor `QvmGrappleSelection["binding"]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GrappleBinding {
    /// Weapon slot.
    Slot,
    /// Offhand.
    Offhand,
}

/// QVM grapple selection, mirroring donor `QvmGrappleSelection` from
/// `src/app/bootstrap/qvm-grapple-selection.ts`.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmGrappleSelection {
    /// Defining provider.
    pub source: ProviderReference,
    /// Inventory binding.
    pub binding: GrappleBinding,
    /// Grapple definition.
    pub profile: QvmGrappleDefinition,
}

/// Prepared QVM grapple, mirroring donor `PreparedQvmGrapple` from
/// `src/app/bootstrap/qvm-grapple-selection.ts`.
#[derive(Debug)]
pub struct PreparedQvmGrapple {
    /// Grapple selection.
    pub selection: QvmGrappleSelection,
    /// Resolved bytecode artifact.
    pub artifact: ResolvedQvmArtifact,
    /// Content mounts.
    pub mounts: MountedContent,
}

/// QVM mod presentation declaration, mirroring donor
/// `QvmModPresentationDeclaration`. Nobody in the lane reads past the handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct QvmModPresentationDeclaration;

/// Mod client presentation admission, mirroring donor
/// `ModClientPresentationAdmission`. Nobody in the lane reads past the handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ModClientPresentationAdmission;

/// Mod module checkpoint, mirroring donor `ModModuleCheckpoint`. Nobody in
/// the lane reads past the handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ModModuleCheckpoint;

/// Mod travel mode, mirroring donor `ModTravelMode`. Nobody in the lane reads
/// past the handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ModTravelMode;

/// QVM mod presentation, mirroring the `presentation` member of donor
/// `PreparedMod`.
#[derive(Debug)]
pub struct ModQvmPresentation {
    /// Resolved bytecode artifact.
    pub artifact: ResolvedQvmArtifact,
    /// Owning module.
    pub source: ModuleIdentity,
    /// Presentation declaration.
    pub declaration: QvmModPresentationDeclaration,
}

/// Prepared gameplay mod, mirroring donor `PreparedMod` from
/// `src/world/session/mods.ts`. The behavior callbacks (`validateState`,
/// `initialize`) live with the world/session mods lane; this mirror covers
/// the data the simulation reads.
#[derive(Debug)]
pub struct PreparedMod {
    /// Mod description.
    pub description: ModDescription,
    /// Mod identity.
    pub identity: ModIdentity,
    /// QVM presentation.
    pub presentation: Option<ModQvmPresentation>,
    /// Module checkpoint.
    pub module_checkpoint: Option<ModModuleCheckpoint>,
    /// Travel mode.
    pub travel: Option<ModTravelMode>,
    /// Client presentation admission.
    pub client_presentation: Option<ModClientPresentationAdmission>,
}

/// QVM weapon profile, mirroring donor `QvmWeaponProfile` from
/// `src/compat/qvm/weapon-behavior-profile.ts` (canonical home:
/// `crate::bootstrap::simulation::qvm_weapon_behavior`); unify when it merges.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct QvmWeaponProfile;

/// Native weapon behavior declaration, mirroring donor
/// `NativeWeaponBehaviorDeclaration` from `src/contracts/native-weapon-behavior.ts`
/// (canonical home:
/// `crate::bootstrap::simulation::rerelease_weapon_behavior`); unify when it merges.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct NativeWeaponBehaviorDeclaration;

/// QuakeC weapon resource entry: `PreparedQuakeCSource["resources"]` values.
#[derive(Debug, Clone, PartialEq)]
pub struct QuakeCWeaponResource {
    /// Resolved resource.
    pub resource: ResolvedResourceReference,
    /// Model bounds.
    pub model_bounds: Option<Bounds>,
}

/// Prepared weapon behavior, mirroring donor `PreparedWeaponBehavior` from
/// `src/app/bootstrap/weapon-behavior-selection.ts`.
#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub enum PreparedWeaponBehavior {
    /// QVM behavior.
    Qvm {
        /// Resolved selection.
        selection: ResolvedWeaponBehaviorSelection,
        /// Resolved bytecode artifact.
        artifact: ResolvedQvmArtifact,
        /// Weapon profile.
        profile: QvmWeaponProfile,
        /// Content mounts.
        mounts: MountedContent,
    },
    /// QuakeC behavior.
    QuakeC {
        /// Resolved selection.
        selection: ResolvedWeaponBehaviorSelection,
        /// QuakeC program.
        program: QcProgram,
        /// Weapon resources by name.
        resources: HashMap<String, QuakeCWeaponResource>,
        /// Content mounts.
        mounts: MountedContent,
    },
    /// Rerelease-native behavior.
    RereleaseNative {
        /// Resolved selection.
        selection: ResolvedWeaponBehaviorSelection,
        /// Prepared rerelease guest.
        prepared: PreparedRereleaseGuest,
        /// Native declaration.
        declaration: NativeWeaponBehaviorDeclaration,
        /// Content mounts.
        mounts: MountedContent,
    },
}

/// Active mod client presentation, mirroring donor
/// `ActiveModClientPresentation` from
/// `src/world/session/mod-client-presentation.ts`. Nobody in the lane reads
/// past the handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ActiveModClientPresentation;

/// Decoded application world: donor `ApplicationWorld`.
#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum ApplicationWorld<'w> {
    /// Quake map.
    Q1(Q1Map<'w>),
    /// Quake II map.
    Q2(Q2DecodedMap<'w>),
    /// Quake III geometry.
    Q3(Q3WorldGeometry),
}

impl ApplicationWorld<'_> {
    /// Donor map kind discriminator.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            ApplicationWorld::Q1(_) => "q1-bsp",
            ApplicationWorld::Q2(_) => "q2-bsp",
            ApplicationWorld::Q3(_) => "q3-bsp",
        }
    }

    /// Entity text.
    #[must_use]
    pub fn entities(&self) -> &str {
        match self {
            ApplicationWorld::Q1(map) => &map.entities,
            ApplicationWorld::Q2(map) => &map.map.entities,
            ApplicationWorld::Q3(geometry) => &geometry.entities,
        }
    }
}

// ---------------------------------------------------------------------------
// Simulation options.
// ---------------------------------------------------------------------------

/// Native Quake II guest callbacks shared by both editions.
pub struct NativeQ2GuestCallbacks {
    /// Native capabilities.
    pub capabilities: WindowsCapabilities,
    /// Print a line.
    pub print: Rc<dyn Fn(&str)>,
    /// Register a console command.
    pub add_command: Rc<dyn Fn(&str)>,
    /// Sample a debug graph.
    pub debug_graph: Rc<dyn Fn(f64, i32)>,
}

/// Native Quake II guest options, mirroring donor `NativeQ2GuestOptions`.
pub enum NativeQ2GuestOptions {
    /// Classic guest.
    Classic {
        /// Shared callbacks.
        callbacks: NativeQ2GuestCallbacks,
        /// Prepared guest.
        prepared: PreparedClassicGuest,
    },
    /// Rerelease guest.
    Rerelease {
        /// Shared callbacks.
        callbacks: NativeQ2GuestCallbacks,
        /// Prepared guest.
        prepared: PreparedRereleaseGuest,
        /// Local services.
        local: RereleaseGuestLocalServices,
    },
}

impl NativeQ2GuestOptions {
    /// Donor `edition` discriminant.
    #[must_use]
    pub fn edition(&self) -> qa_content::q2::foundation::host::Q2Edition {
        use qa_content::q2::foundation::host::Q2Edition;
        match self {
            NativeQ2GuestOptions::Classic { .. } => Q2Edition::Classic,
            NativeQ2GuestOptions::Rerelease { .. } => Q2Edition::Rerelease,
        }
    }

    /// Shared native callbacks.
    #[must_use]
    pub fn callbacks(&self) -> &NativeQ2GuestCallbacks {
        match self {
            NativeQ2GuestOptions::Classic { callbacks, .. } => callbacks,
            NativeQ2GuestOptions::Rerelease { callbacks, .. } => callbacks,
        }
    }

    /// Prepared classic guest, when classic.
    #[must_use]
    pub fn prepared_classic(&self) -> Option<&PreparedClassicGuest> {
        match self {
            NativeQ2GuestOptions::Classic { prepared, .. } => Some(prepared),
            NativeQ2GuestOptions::Rerelease { .. } => None,
        }
    }

    /// Prepared rerelease guest, when rerelease.
    #[must_use]
    pub fn prepared_rerelease(&self) -> Option<&PreparedRereleaseGuest> {
        match self {
            NativeQ2GuestOptions::Classic { .. } => None,
            NativeQ2GuestOptions::Rerelease { prepared, .. } => Some(prepared),
        }
    }

    /// Rerelease local services, when rerelease.
    #[must_use]
    pub fn local(&self) -> Option<&RereleaseGuestLocalServices> {
        match self {
            NativeQ2GuestOptions::Classic { .. } => None,
            NativeQ2GuestOptions::Rerelease { local, .. } => Some(local),
        }
    }
}

/// Q3 guest options: the `q3Guest` member of donor `SimulationOptions`.
pub struct Q3GuestOptions {
    /// Writable file store.
    pub writable: UserFileStore,
    /// Common services.
    pub common: Q3GuestCommonServices,
    /// Prepared game.
    pub prepared: PreparedQ3Game,
    /// Game directory.
    pub game_directory: String,
    /// Print a line.
    pub print: Rc<dyn Fn(&str)>,
}

/// Weapon behavior clock: the required `WindowsCapabilities` clock fields.
pub struct WeaponBehaviorClock {
    /// Millisecond clock.
    pub now_milliseconds: Rc<dyn Fn() -> i64>,
    /// Performance counter.
    pub performance_counter: Rc<dyn Fn() -> u64>,
    /// Performance frequency.
    pub performance_frequency: u64,
}

/// Simulation mode, mirroring donor `SimulationOptions["mode"]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SimulationMode {
    /// Singleplayer.
    Singleplayer,
    /// Cooperative.
    Coop,
    /// Deathmatch.
    Deathmatch,
}

/// Player identity admission, mirroring donor `SimulationOptions["playerIdentity"]` results.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PlayerIdentity {
    /// Server seat.
    pub seat: usize,
    /// Social identity.
    pub social_id: String,
}

/// Name/value console variable pair.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CvarNameValue {
    /// Variable name.
    pub name: String,
    /// Variable value.
    pub value: String,
}

/// Client prompt-support probe.
pub type PromptSupported = Rc<dyn Fn(&ClientId) -> bool>;

/// Player identity lookup.
pub type PlayerIdentityLookup = Rc<dyn Fn(&ClientId) -> PlayerIdentity>;

/// Rerelease navigation factory (donor `prepareRereleaseNavigation`).
///
/// Wave-2 addition: the runtime home (`super::runtime`) and the navigation
/// home (`super::navigation`) both exist now, so the callback names them.
pub type PrepareRereleaseNavigation = Rc<
    dyn Fn(
        &super::runtime::SharedSimulation,
    ) -> Result<super::navigation::ApplicationBotNavigation, super::navigation::NavigationError>,
>;

/// Simulation options, mirroring donor `SimulationOptions`.
pub struct SimulationOptions<'w> {
    /// Rerelease navigation factory.
    pub prepare_rerelease_navigation: Option<PrepareRereleaseNavigation>,
    /// Prepared QVM grapple.
    pub prepared_qvm_grapple: Option<PreparedQvmGrapple>,
    /// Prepared gameplay mods.
    pub prepared_mods: Option<Vec<PreparedMod>>,
    /// Mod commands.
    pub mod_commands: Option<ModCommands>,
    /// Mod user files.
    pub mod_files: Option<ModUserFiles>,
    /// Enabled mods.
    pub enabled_mods: Option<Vec<ModSelection>>,
    /// Mod travel checkpoint.
    pub mod_travel: Option<ModTravelCheckpoint>,
    /// Weapon behavior real-time trap.
    pub weapon_behavior_real_time: Option<QvmRealTime>,
    /// Weapon behavior clock.
    pub weapon_behavior_clock: Option<WeaponBehaviorClock>,
    /// Prepared weapon behaviors.
    pub weapon_behaviors: Option<Vec<PreparedWeaponBehavior>>,
    /// Dedicated server without local rendering.
    pub dedicated: Option<bool>,
    /// Whether a client supports prompts.
    pub prompt_supported: Option<PromptSupported>,
    /// Prepared QuakeC source.
    pub prepared_quakec: Option<PreparedQuakeCSource>,
    /// Native Quake II guest.
    pub q2_guest: Option<NativeQ2GuestOptions>,
    /// Native Quake II travel.
    pub native_q2_travel: Option<NativeQ2Travel>,
    /// Whether an original save may be offered.
    pub original_save_candidate: Option<bool>,
    /// Starting items.
    pub start_items: Option<String>,
    /// Initial spawn point.
    pub initial_spawn_point: Option<String>,
    /// Next Quake II server.
    pub q2_next_server: Option<String>,
    /// Quake III guest.
    pub q3_guest: Option<Q3GuestOptions>,
    /// Session identity owner.
    pub identity: IdentityOwner,
    /// Executable recipe.
    pub recipe: ExecutableRecipe,
    /// Decoded world.
    pub world: ApplicationWorld<'w>,
    /// Content mounts.
    pub mounts: MountedContent,
    /// Skill level (0-3).
    pub skill: u8,
    /// Simulation mode.
    pub mode: SimulationMode,
    /// Random seed.
    pub seed: u32,
    /// Maximum clients.
    pub max_clients: usize,
    /// Travel to admit.
    pub travel: Option<SimulationTravel>,
    /// Player identity lookup.
    pub player_identity: Option<PlayerIdentityLookup>,
    /// Quake III session carry.
    pub q3_session: Option<Q3SourceSessionCarry>,
    /// Server profile.
    pub profile: Option<ServerProfile>,
    /// Source console registry.
    pub source_registry: Option<CvarRegistry>,
    /// Source console archive.
    pub source_archive: Option<Vec<CvarArchiveEntry>>,
    /// Quake II console variables.
    pub q2_cvars: Option<Vec<CvarNameValue>>,
    /// Quake III console variables.
    pub q3_cvars: Option<Vec<CvarNameValue>>,
    /// Initial source time in milliseconds.
    pub initial_source_milliseconds: Option<i32>,
    /// Monster navigation.
    pub monster_navigation: Option<ApplicationMonsterNavigation>,
    /// Save image to restore (rich simulation image; the `qa_world` session
    /// image does not carry providers, guests, or recipes).
    pub restore: Option<super::save::SimulationSaveImage>,
    /// Restored clients.
    pub restored_clients: Option<Vec<ClientId>>,
}

// ---------------------------------------------------------------------------
// Travel.
// ---------------------------------------------------------------------------

/// QuakeC source kind, mirroring donor `QuakeCSourceTravel["kind"]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QuakeCSourceKind {
    /// NetQuake.
    Netquake,
    /// QuakeWorld.
    Quakeworld,
}

/// QuakeC client role, mirroring donor `QuakeCSourceTravel["clients"]` roles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QuakeCClientRole {
    /// Player.
    Player,
    /// Spectator.
    Spectator,
}

/// QuakeC source client travel entry.
#[derive(Debug, Clone, PartialEq)]
pub struct QuakeCSourceClient {
    /// Client.
    pub client: ClientId,
    /// Client parameters.
    pub parameters: Vec<f64>,
    /// User info.
    pub user_info: HashMap<String, String>,
    /// Client role.
    pub role: Option<QuakeCClientRole>,
}

/// QuakeC source travel, mirroring donor `QuakeCSourceTravel`.
#[derive(Debug, Clone, PartialEq)]
pub struct QuakeCSourceTravel {
    /// Source kind.
    pub kind: QuakeCSourceKind,
    /// Server flags.
    pub server_flags: i32,
    /// Console variables.
    pub cvars: Vec<CvarNameValue>,
    /// Traveling clients.
    pub clients: Vec<QuakeCSourceClient>,
}

/// Quake II landmark travel entry.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2Landmark {
    /// Client slot.
    pub client_slot: u32,
    /// Landmark name.
    pub name: String,
    /// Origin relative to the landmark.
    pub relative_origin: Vec3,
    /// Velocity relative to the landmark.
    pub relative_velocity: Vec3,
    /// View angles relative to the landmark.
    pub relative_view_angles: Vec3,
}

/// Simulation travel source, mirroring donor `SimulationTravel["source"]`.
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum SimulationTravelSource {
    /// Quake travel.
    Q1 {
        /// Server flags.
        flags: i32,
        /// Skill level.
        skill: u8,
    },
    /// QuakeC source travel.
    QuakeC(QuakeCSourceTravel),
    /// Quake II travel.
    Q2 {
        /// Server flags.
        server_flags: i32,
        /// Capture-the-flag travel.
        lmctf: Option<LmctfTravel>,
        /// Rerelease campaign state.
        rerelease: Option<Q2RereleaseCampaignState>,
        /// Coop landmark.
        landmark: Option<Q2Landmark>,
    },
}

impl SimulationTravelSource {
    /// Quake campaign carry (donor `source.kind === "q1"` flags/skill).
    #[must_use]
    pub fn q1_campaign(&self) -> Option<(i32, u8)> {
        match self {
            SimulationTravelSource::Q1 { flags, skill } => Some((*flags, *skill)),
            _ => None,
        }
    }

    /// Quake II server flags (donor `source.kind === "q2"` carry).
    #[must_use]
    pub fn q2_server_flags(&self) -> Option<i32> {
        match self {
            SimulationTravelSource::Q2 { server_flags, .. } => Some(*server_flags),
            _ => None,
        }
    }

    /// QuakeC source travel (donor `source.kind` netquake/quakeworld carry).
    #[must_use]
    pub fn quakec(&self) -> Option<&QuakeCSourceTravel> {
        match self {
            SimulationTravelSource::QuakeC(travel) => Some(travel),
            _ => None,
        }
    }
}

/// Selected-arsenal travel by family.
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum SelectedArsenalTravel {
    /// Quake travel.
    Q1(Q1SelectedArsenalTravel),
    /// Quake II travel.
    Q2 {
        /// Selected weapon.
        weapon: Option<ItemId>,
        /// Carried inventory.
        inventory: Vec<InventoryEntry>,
    },
    /// Quake III travel.
    Q3 {
        /// Arsenal checkpoint.
        state: Q3SelectedArsenalCheckpoint,
        /// Checkpoint time in milliseconds.
        milliseconds: i32,
    },
}

/// Per-player travel state by family.
#[derive(Debug, Clone, PartialEq)]
pub enum TravelPlayerState {
    /// Quake carry.
    Q1 {
        /// Travel state.
        carry: Q1TravelState,
    },
    /// Quake II carry.
    Q2 {
        /// Player carry.
        carry: Q2PlayerCarry,
    },
    /// QuakeC carry.
    QuakeC,
}

/// Per-player simulation travel entry.
#[derive(Debug, Clone, PartialEq)]
pub struct SimulationTravelPlayer {
    /// Client.
    pub client: ClientId,
    /// Weapon slot reference.
    pub weapon_slot: Option<WeaponReference>,
    /// Hand grenade travel.
    pub hand_grenades: Option<HandGrenadeTravel>,
    /// Selected-arsenal travel.
    pub selected_arsenal: Option<SelectedArsenalTravel>,
    /// Family travel state.
    pub state: TravelPlayerState,
}

/// Simulation travel snapshot, mirroring donor `SimulationTravel`.
#[derive(Debug, Clone, PartialEq)]
pub struct SimulationTravel {
    /// Spawn point.
    pub spawn_point: String,
    /// Travel source.
    pub source: SimulationTravelSource,
    /// Traveling players.
    pub players: Vec<SimulationTravelPlayer>,
}

// ---------------------------------------------------------------------------
// Players and presentation.
// ---------------------------------------------------------------------------

/// Player admission receipt, mirroring donor `PlayerAdmission`.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerAdmission {
    /// Admitted actor.
    pub actor: ActorId,
    /// View height.
    pub view_height: f64,
}

/// View pitch drift state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PitchDrift {
    /// Grounded flag.
    pub grounded: bool,
    /// Ideal pitch.
    pub ideal_pitch: f64,
    /// Disabled flag.
    pub disabled: bool,
}

/// Player view, mirroring donor `PlayerView`.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerView {
    /// Client view-offset delta.
    pub client_view_offset_delta: Option<Vec3>,
    /// Screen blend.
    pub blend: Option<Vec4>,
    /// Damage blend.
    pub damage_blend: Option<Vec4>,
    /// View origin.
    pub origin: Vec3,
    /// View angles.
    pub angles: Vec3,
    /// View height.
    pub view_height: f64,
    /// Kick angles.
    pub kick_angles: Option<Vec3>,
    /// Field of view.
    pub field_of_view: Option<f64>,
    /// Foreign character death presentation.
    pub foreign_character_death: bool,
    /// Pitch drift.
    pub pitch_drift: Option<PitchDrift>,
}

/// Player UI item kind, mirroring donor `PlayerUiItem["kind"]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlayerUiItemKind {
    /// Weapon.
    Weapon,
    /// Powerup.
    Powerup,
}

/// Player UI item row, mirroring donor `PlayerUiItem`.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerUiItem {
    /// Item identity.
    pub id: ItemId,
    /// Display label.
    pub label: String,
    /// Item kind.
    pub kind: PlayerUiItemKind,
    /// Source ordinal.
    pub source_ordinal: f64,
    /// Owned flag.
    pub owned: bool,
    /// Has-ammo flag.
    pub has_ammo: bool,
    /// Count.
    pub count: Option<f64>,
    /// Warning count.
    pub warning_count: f64,
}

/// Player UI snapshot, mirroring donor `PlayerUi`.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerUi {
    /// Selected-arsenal presentation.
    pub selected_arsenal: bool,
    /// Native inventory readout.
    pub native_inventory: Option<NativeInventoryReadout>,
    /// Active powerup timers.
    pub powerups: Vec<ActivePowerupTimer>,
    /// Weapon status.
    pub weapon_status: Option<WeaponHudStatus>,
    /// Arsenal warning.
    pub arsenal_warning: ArsenalAmmoWarning,
    /// Health.
    pub health: f64,
    /// Armor state.
    pub armor: ArmorState,
    /// Active weapon.
    pub active_weapon: Option<ItemId>,
    /// Active ammo.
    pub ammo: Option<UiAmmo>,
    /// Carried inventory.
    pub inventory: Vec<InventoryEntry>,
    /// Item rows.
    pub items: Vec<PlayerUiItem>,
}

/// Active ammo readout.
#[derive(Debug, Clone, PartialEq)]
pub struct UiAmmo {
    /// Ammo item.
    pub item: ItemId,
    /// Count.
    pub count: f64,
}

/// Remote armor item reported for Q2 guest players (classic and rerelease share it).
pub(crate) const REMOTE_ARMOR_ITEM: &str = "q2:remote-armor";

/// Narrow a Q2 triple into a scene vector (shared guest-player helper).
pub(crate) fn guest_to_vec3(value: &Q2Vec3) -> Vec3 {
    Vec3 {
        x: value.x as f32,
        y: value.y as f32,
        z: value.z as f32,
    }
}

/// Narrow a Q2 quad into a scene vector (shared guest-player helper).
pub(crate) fn guest_to_vec4(value: &Q2Vec4) -> Vec4 {
    Vec4 {
        x: value.x as f32,
        y: value.y as f32,
        z: value.z as f32,
        w: value.w as f32,
    }
}

/// Read a HUD stat slot, defaulting to zero (shared guest-player helper).
pub(crate) fn guest_stat(stats: &[i16], index: usize) -> i16 {
    stats.get(index).copied().unwrap_or(0)
}

/// Build the guest player HUD from the shared view half of a Q2 player state.
///
/// Classic and rerelease states carry the same [`Q2PlayerView`]; only the
/// configstring model base differs per API, so callers pass it in. Only the
/// selected weapon's ammo is public here; full inventory comes from
/// svc_inventory.
pub(crate) fn guest_player_ui(
    view: &Q2PlayerView,
    model_base: u32,
    configstrings: &HashMap<u32, String>,
    source: ProviderReference,
    definitions: &[Q2WeaponDefinition],
) -> PlayerUi {
    let model = if view.gun_index == 0 {
        None
    } else {
        configstrings.get(&(model_base + view.gun_index as u32))
    };
    let weapon = model.and_then(|model| definitions.iter().find(|definition| definition.view_model == *model));
    let armor = guest_stat(&view.stats, 5);
    let ammo = guest_stat(&view.stats, 3);
    PlayerUi {
        selected_arsenal: false,
        native_inventory: None,
        powerups: Vec::new(),
        weapon_status: q2_weapon_status(weapon, |_: &ItemId| i32::from(ammo), source),
        arsenal_warning: ArsenalAmmoWarning::None,
        health: f64::from(guest_stat(&view.stats, 1)),
        armor: ArmorState {
            powered: PoweredProtectionState::None,
            regular: if armor == 0 {
                RegularArmorState::None
            } else {
                RegularArmorState::Q2 {
                    points: f64::from(armor),
                    normal_protection: 0.0,
                    energy_protection: 0.0,
                    item: REMOTE_ARMOR_ITEM.to_string(),
                }
            },
        },
        active_weapon: weapon.map(|weapon| weapon.item.clone()),
        ammo: match weapon {
            None => None,
            Some(weapon) => weapon.ammo.as_ref().map(|item| UiAmmo {
                item: item.clone(),
                count: f64::from(ammo),
            }),
        },
        inventory: Vec::new(),
        items: Vec::new(),
    }
}

/// Player colors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PlayerColors {
    /// Top color.
    pub top: i32,
    /// Bottom color.
    pub bottom: i32,
}

/// Model beam presentation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelBeam {
    /// Segment length.
    pub segment_length: f64,
}

/// Shader beam presentation.
#[derive(Debug, Clone, PartialEq)]
pub struct ShaderBeam {
    /// Shader path.
    pub path: String,
    /// Beam end.
    pub end: Vec3,
    /// Beam width.
    pub width: f64,
}

/// Model attachment.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelAttachment {
    /// Model path.
    pub path: String,
    /// Attachment tag.
    pub tag: String,
}

/// Quake III grapple cable presentation.
#[derive(Debug, Clone, PartialEq)]
pub struct Q3GrappleCable {
    /// Owning actor.
    pub owner: ActorId,
    /// Owner origin.
    pub owner_origin: Vec3,
    /// Owner angles.
    pub owner_angles: Vec3,
    /// View height.
    pub view_height: f64,
    /// Offhand flag.
    pub offhand: bool,
    /// Attached flag.
    pub attached: bool,
    /// Flight curve.
    pub flight: String,
    /// Pull curve.
    pub pull: String,
    /// Hold curve.
    pub hold: String,
    /// Segment length.
    pub segment_length: f64,
}

/// Quake III weapon presentation.
#[derive(Debug, Clone, PartialEq)]
pub struct Q3WeaponPresentation {
    /// Time in milliseconds.
    pub time_milliseconds: i32,
    /// Torso animation.
    pub torso_animation: i32,
    /// Last fire time in milliseconds.
    pub last_fire_milliseconds: Option<i32>,
    /// Firing flag.
    pub firing: bool,
    /// Horizontal speed.
    pub horizontal_speed: f64,
    /// Bob cycle.
    pub bob_cycle: f64,
    /// Source weapon number.
    pub weapon: i32,
}

/// Simulation presentation snapshot, mirroring donor `SimulationPresentation`.
#[derive(Debug, Clone, PartialEq)]
pub struct SimulationPresentation {
    /// Held weapon declaration.
    pub held_weapon: Option<HeldWeaponDeclaration>,
    /// Native held weapon presentation.
    pub native_held_weapon: bool,
    /// Weapon item.
    pub weapon_item: Option<ItemId>,
    /// Whether the weapon replaces the body model.
    pub replaces_body: bool,
    /// Source-client render owner.
    pub render_source_client: bool,
    /// Flare.
    pub flare: Option<SceneFlare>,
    /// Actor.
    pub actor: ActorId,
    /// Content.
    pub content: ContentId,
    /// Game family.
    pub family: GameFamily,
    /// Model path.
    pub path: String,
    /// Frame.
    pub frame: i32,
    /// Previous frame.
    pub old_frame: i32,
    /// Frame blend.
    pub back_lerp: Option<f64>,
    /// Skin.
    pub skin: i32,
    /// Skin path.
    pub skin_path: Option<String>,
    /// Indexed skin.
    pub indexed_skin: Option<IndexedModelSkin>,
    /// Player colors.
    pub player_colors: Option<PlayerColors>,
    /// Effects word.
    pub effects: i32,
    /// Render flags.
    pub render_flags: i32,
    /// Origin.
    pub origin: Vec3,
    /// Previous origin.
    pub previous_origin: Option<Vec3>,
    /// Model beam.
    pub model_beam: Option<ModelBeam>,
    /// Shader beam.
    pub shader_beam: Option<ShaderBeam>,
    /// Model attachments.
    pub model_attachments: Option<Vec<ModelAttachment>>,
    /// Grapple model anchor.
    pub model_anchor: Option<QvmGrappleViewAnchor>,
    /// Grapple cable.
    pub q3_grapple_cable: Option<Q3GrappleCable>,
    /// Angles.
    pub angles: Vec3,
    /// Scale.
    pub scale: f64,
    /// Alpha.
    pub alpha: Option<f64>,
    /// Visible flag.
    pub visible: bool,
    /// View weapon flag.
    pub view_weapon: bool,
    /// Quake III weapon presentation.
    pub q3_weapon: Option<Q3WeaponPresentation>,
}

// ---------------------------------------------------------------------------
// Presentation events.
// ---------------------------------------------------------------------------

/// Quake III character presentation event: donor `Q3CharacterEvent` with the
/// owned actor projected to its handle.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Q3CharacterPresentationEvent {
    /// Actor.
    pub actor: ActorId,
    /// Sequence.
    pub sequence: i32,
    /// Time in milliseconds.
    pub time_ms: i32,
    /// Event.
    pub event: i32,
    /// Parameter.
    pub parameter: i32,
}

/// String-valued Quake I client metadata kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q1ClientStringKind {
    /// Name.
    Name,
    /// Social identity.
    Social,
    /// Player info.
    PlayerInfo,
}

/// Numeric Quake I client metadata kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q1ClientNumericKind {
    /// Colors.
    Colors,
    /// Frags.
    Frags,
    /// Ping.
    Ping,
}

/// Quake I client metadata event, mirroring donor `Q1ClientMetadataEvent`.
#[derive(Debug, Clone, PartialEq)]
pub enum Q1ClientMetadataEvent {
    /// String metadata.
    String {
        /// Metadata kind.
        kind: Q1ClientStringKind,
        /// Client slot.
        slot: u32,
        /// Value.
        value: String,
    },
    /// Numeric metadata.
    Numeric {
        /// Metadata kind.
        kind: Q1ClientNumericKind,
        /// Client slot.
        slot: u32,
        /// Value.
        value: i32,
    },
}

/// View-reset reason, mirroring donor `SourcePresentationEvent` view reasons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ViewResetReason {
    /// Spawn.
    Spawn,
    /// Teleport.
    Teleport,
    /// Freeze.
    Freeze,
    /// Source reset.
    Source,
}

/// Source presentation event, mirroring donor `SourcePresentationEvent`.
#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum SourcePresentationEvent {
    /// Presentation owner retired.
    OwnerRetired {
        /// Owner.
        owner: PresentationOwner,
    },
    /// Presentation owner refreshed.
    OwnerRefreshed {
        /// Owner.
        owner: PresentationOwner,
    },
    /// Debug graph sample.
    DebugGraph {
        /// Sample value.
        value: f64,
        /// Sample color.
        color: i32,
    },
    /// Quake I event.
    Q1(Q1Event),
    /// Quake I skybox.
    Q1Skybox {
        /// Skybox name.
        name: String,
    },
    /// Quake I client metadata.
    Q1Client(Q1ClientMetadataEvent),
    /// Quake I level completed.
    Q1LevelCompleted,
    /// Quake I back to lobby.
    Q1BackToLobby,
    /// Quake I fog transition.
    Q1Fog {
        /// Player, when targeted.
        player: Option<ActorId>,
        /// Fog transition.
        transition: Q1FogTransition,
        /// Sky factor.
        sky_factor: f64,
    },
    /// Music track.
    CdTrack {
        /// Track.
        track: i32,
    },
    /// Music pause.
    MusicPause {
        /// Paused flag.
        paused: bool,
    },
    /// Quake I composition event.
    Q1Composition(Q1CompositionEvent),
    /// Quake I level event.
    Q1Level(Q1IntermissionResult),
    /// Quake II event.
    Q2(Q2PresentationEvent),
    /// Quake II weapon event.
    Q2Weapon(Q2WeaponEvent),
    /// View reset.
    ViewReset {
        /// Reason.
        reason: ViewResetReason,
        /// Actor.
        actor: ActorId,
        /// Angles.
        angles: Vec3,
    },
    /// Quake II composition event.
    Q2Composition(Q2CompositionEvent),
    /// Quake II rerelease event.
    Q2Rerelease(Q2RereleaseEvent),
    /// Quake II player event.
    Q2Player(Q2PlayerEvent),
    /// Quake III character event.
    Q3Character(Q3CharacterPresentationEvent),
    /// Shared Q3 ballistic event.
    Q3Ballistics(Q3SharedBallisticEvent),
    /// Quake III source event.
    Q3Source(Q3SourceEvent),
}

/// Simulation presentation event envelope, mirroring donor
/// `SimulationPresentationEvent`.
#[derive(Debug, Clone)]
pub struct SimulationPresentationEvent {
    /// Event.
    pub event: SourcePresentationEvent,
    /// Owner.
    pub owner: Option<PresentationOwner>,
    /// Recipient.
    pub recipient: Option<ActorId>,
    /// Sequence.
    pub sequence: u64,
    /// Content.
    pub content: ContentId,
    /// Time in seconds.
    pub seconds: f64,
    /// Source entity.
    pub source_entity: Option<i32>,
}

// ---------------------------------------------------------------------------
// Presentation access.
// ---------------------------------------------------------------------------

/// Debug shape access, mirroring donor `DebugShapePresentationAccess`.
pub trait DebugShapePresentationAccess {
    /// Debug lines.
    fn lines(&self) -> Vec<DebugLine>;
    /// Line width.
    fn line_width(&self) -> f64;
}

/// Simulation presentation access, mirroring donor
/// `SimulationPresentationAccess`.
pub trait SimulationPresentationAccess {
    /// Active mod client presentations.
    fn mod_client_presentation_sources(&self) -> Vec<ActiveModClientPresentation> {
        Vec::new()
    }

    /// World text.
    fn world_text(&self) -> WorldText;

    /// Player UI.
    fn player_ui(&self, client: &ClientId) -> PlayerUi;

    /// Capture travel from a spawn point.
    fn capture_travel(&self, spawn_point: Option<&str>) -> SimulationTravel;

    /// Admit travel for a client.
    fn admit_travel(&mut self, client: &ClientId, travel: &SimulationTravel) -> PlayerAdmission;

    /// Character views.
    fn character_views(&self) -> Vec<(ActorId, Q3CharacterView)>;

    /// Submit a player command.
    fn player_command(&mut self, client: &ClientId, command: &str);

    /// Take pending transition intents.
    fn take_transitions(&mut self) -> Vec<TransitionIntent>;

    /// Current presentations.
    fn presentations(&self) -> Vec<SimulationPresentation>;

    /// Drain presentation events for a client.
    fn drain_presentation_events(&mut self, client: Option<&ClientId>) -> Vec<SimulationPresentationEvent>;

    /// Register a resource request.
    fn register_resource(&mut self, request: &ResourceRequest) -> ResolvedResourceReference;

    /// Player view.
    fn player_view(&self, client: &ClientId) -> Option<PlayerView>;

    /// Admit a player.
    fn admit_player(&mut self, client: &ClientId, actor: &ActorId, view_height: f64) -> PlayerAdmission;
}

#[cfg(test)]
mod tests {
    use qa_content::contract::ContentId;
    use qa_core::identity::ProviderId;
    use qa_core::math::vec3;

    use super::*;

    fn client() -> (IdentityOwner, ClientId) {
        let owner = IdentityOwner::create("types-test").unwrap();
        let client = owner.client(1, 0);
        (owner, client)
    }

    #[test]
    fn travel_round_trip() {
        let (_owner, client) = client();
        let travel = SimulationTravel {
            spawn_point: "start".to_string(),
            source: SimulationTravelSource::Q2 {
                server_flags: 3,
                lmctf: None,
                rerelease: None,
                landmark: Some(Q2Landmark {
                    client_slot: 1,
                    name: "lm".to_string(),
                    relative_origin: vec3(1.0, 2.0, 3.0),
                    relative_velocity: vec3(0.0, 0.0, 0.0),
                    relative_view_angles: vec3(0.0, 90.0, 0.0),
                }),
            },
            players: vec![SimulationTravelPlayer {
                client,
                weapon_slot: None,
                hand_grenades: None,
                selected_arsenal: Some(SelectedArsenalTravel::Q2 {
                    weapon: Some("q2:weapon/blaster".to_string()),
                    inventory: vec![InventoryEntry {
                        item: "q2:ammo/cells".to_string(),
                        count: 50.0,
                        capacity: 100.0,
                        count_policy: None,
                    }],
                }),
                state: TravelPlayerState::QuakeC,
            }],
        };
        assert_eq!(travel.spawn_point, "start");
        assert!(matches!(
            travel.source,
            SimulationTravelSource::Q2 { server_flags: 3, .. }
        ));
        assert_eq!(travel.players.len(), 1);
    }

    #[test]
    fn presentation_event_envelope() {
        let event = SimulationPresentationEvent {
            event: SourcePresentationEvent::ViewReset {
                reason: ViewResetReason::Teleport,
                actor: IdentityOwner::create("evt").unwrap().actor(2, 0),
                angles: vec3(0.0, 0.0, 0.0),
            },
            owner: None,
            recipient: None,
            sequence: 7,
            content: ContentId("q1:id1:quake:1".to_string()),
            seconds: 1.5,
            source_entity: Some(3),
        };
        assert_eq!(event.sequence, 7);
        assert!(matches!(
            event.event,
            SourcePresentationEvent::ViewReset {
                reason: ViewResetReason::Teleport,
                ..
            }
        ));
    }

    #[test]
    fn q1_client_metadata_kinds() {
        let name = Q1ClientMetadataEvent::String {
            kind: Q1ClientStringKind::Name,
            slot: 1,
            value: "player".to_string(),
        };
        let frags = Q1ClientMetadataEvent::Numeric {
            kind: Q1ClientNumericKind::Frags,
            slot: 1,
            value: 10,
        };
        assert!(matches!(name, Q1ClientMetadataEvent::String { .. }));
        assert!(matches!(frags, Q1ClientMetadataEvent::Numeric { value: 10, .. }));
    }

    #[test]
    fn ui_item_rows() {
        let item = PlayerUiItem {
            id: "q1:weapon/shotgun".to_string(),
            label: "Shotgun".to_string(),
            kind: PlayerUiItemKind::Weapon,
            source_ordinal: 2.0,
            owned: true,
            has_ammo: true,
            count: Some(12.0),
            warning_count: 5.0,
        };
        assert_eq!(item.kind, PlayerUiItemKind::Weapon);
        assert_eq!(item.count, Some(12.0));
    }

    #[test]
    fn mode_and_world_kinds() {
        assert_ne!(SimulationMode::Coop, SimulationMode::Deathmatch);
        let _ = ProviderId::new("sim", "test");
    }

    #[test]
    fn guest_helpers_narrow_vectors_and_stats() {
        assert_eq!(
            guest_to_vec3(&Q2Vec3 { x: 1.0, y: 2.0, z: 3.0 }),
            Vec3 { x: 1.0, y: 2.0, z: 3.0 }
        );
        assert_eq!(
            guest_to_vec4(&Q2Vec4 {
                x: 1.0,
                y: 2.0,
                z: 3.0,
                w: 4.0
            }),
            Vec4 {
                x: 1.0,
                y: 2.0,
                z: 3.0,
                w: 4.0
            }
        );
        assert_eq!(guest_stat(&[10, 20], 1), 20);
        assert_eq!(guest_stat(&[10, 20], 7), 0);
        assert_eq!(REMOTE_ARMOR_ITEM, "q2:remote-armor");
    }

    #[test]
    fn guest_player_ui_reports_health_armor_and_no_weapon() {
        let view = Q2PlayerView {
            view_angles: Q2Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            view_offset: Q2Vec3 {
                x: 0.0,
                y: 0.0,
                z: 22.0,
            },
            kick_angles: Q2Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            gun_angles: Q2Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            gun_offset: Q2Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            gun_index: 0,
            gun_frame: 0,
            fov: 90,
            render_flags: 0,
            stats: vec![0, 100, 0, 12, 0, 50],
        };
        let source = ProviderReference {
            provider: ProviderId::new("test", "native"),
            content: ContentId("q2:baseq2:baseq2:1".to_string()),
        };
        let ui = guest_player_ui(&view, 0, &HashMap::new(), source, &[]);
        assert_eq!(ui.health, 100.0);
        assert!(matches!(ui.armor.regular, RegularArmorState::Q2 { .. }));
        assert!(ui.active_weapon.is_none());
        assert!(ui.ammo.is_none());
        assert!(ui.weapon_status.is_none());
    }
}

/// Scene light style, mirroring donor `SceneLightStyle` from
/// `src/contracts/scene.ts` (C6: needed by the presentation seam).
#[derive(Debug, Clone, PartialEq)]
pub enum SceneLightStyle {
    /// Quake light style.
    Q1 {
        /// Style index.
        style: i32,
        /// Light value.
        value: i32,
    },
    /// Quake II light style.
    Q2 {
        /// Style index.
        style: i32,
        /// RGB scale.
        rgb: Vec3,
        /// White scale.
        white: f64,
    },
}
