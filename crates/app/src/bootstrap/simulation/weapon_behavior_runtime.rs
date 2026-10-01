//! Simulation weapon behavior runtime.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/simulation/weapon-behavior-runtime.ts`.
//!
//! The runtime orchestrates trajectory sources (QuakeC, QVM, rerelease
//! native) selected in [`PreparedWeaponBehavior`](super::types::PreparedWeaponBehavior)
//! and delegates attachment bookkeeping to the world-partition driver. Every
//! source, reader, and the driver itself are injected seams: the donor
//! imports them from the qc, guest, and world partitions, and this file must
//! not duplicate their logic. Donor `async` loading boundaries become sync
//! methods taking `next_frame: &mut dyn FnMut()`, matching the
//! `bootstrap::loading` precedent.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use qa_content::contract::{ItemId, NativeWeaponBehaviorDeclaration, ProjectileRole, WeaponBehaviorDefinition};
use qa_core::identity::{ActorId, OwnedActor, ProviderId, SavedActorId};
use qa_core::math::{Bounds, Vec3};
use qa_guest::qc::program::QcProgram;
use qa_world::body::BodyState;
use qa_world::save::value::{SaveJson, SaveReader};
use thiserror::Error;

use super::native_q2_rerelease_save::RereleaseSourceSave;
use super::random::{RandomCheckpoint, RandomError, SourceRandom};
use super::types::{PreparedWeaponBehavior, SimulationMode};

/// Runtime checkpoint schema version (donor `version: 1`).
pub const WEAPON_BEHAVIOR_RUNTIME_CHECKPOINT_VERSION: u32 = 1;

/// Donor spelling of a projectile role for diagnostics.
fn projectile_role_name(role: ProjectileRole) -> &'static str {
    match role {
        ProjectileRole::Rocket => "rocket",
        ProjectileRole::Grenade => "grenade",
        ProjectileRole::Nail => "nail",
        ProjectileRole::Bolt => "bolt",
        ProjectileRole::Plasma => "plasma",
        ProjectileRole::Energy => "energy",
        ProjectileRole::Grapple => "grapple",
    }
}

/// Simulation weapon behavior failure.
#[derive(Debug, Error)]
pub enum WeaponBehaviorRuntimeError {
    /// Two selected behaviors share a projectile role.
    #[error("Multiple selected trajectory behaviors for {0}")]
    DuplicateRole(String),
    /// Launch before loading completed.
    #[error("Weapon components require completed asynchronous loading")]
    LoadingRequired,
    /// Sync checkpoint with native components selected.
    #[error("Native weapon components require asynchronous checkpoint capture")]
    NativeCheckpointRequiresLoading,
    /// Checkpoint before loading completed.
    #[error("Weapon components have not finished loading")]
    ComponentsNotLoaded,
    /// Sync restore with native components selected.
    #[error("Native weapon components require asynchronous restoration")]
    NativeRestoreRequiresLoading,
    /// Restore queued twice.
    #[error("Weapon component restore is already queued")]
    RestoreAlreadyQueued,
    /// Save references a retired actor.
    #[error("Weapon component references a retired actor")]
    RetiredActor,
    /// Seam-reported failure from an injected source, reader, hook, or the
    /// attachments driver (donor logic owned by another partition).
    #[error("Weapon behavior seam failure: {0}")]
    Seam(String),
    /// Save decode failure.
    #[error(transparent)]
    Save(#[from] qa_world::WorldError),
    /// Trajectory random restore failure.
    #[error(transparent)]
    Random(#[from] RandomError),
    /// Cleanup collected failures (donor `AggregateError`).
    #[error("Weapon component cleanup failed: {0:?}")]
    CloseFailed(Vec<String>),
}

/// Mirror of `WeaponBehaviorLaunch` from donor
/// `src/contracts/weapon-behavior.ts` (canonical home:
/// `qa_content::contract`); unify post-merge.
#[derive(Debug, Clone, PartialEq)]
pub struct WeaponBehaviorLaunch {
    /// Launched projectile.
    pub projectile: OwnedActor,
    /// Shooter.
    pub shooter: ActorId,
    /// Fired weapon.
    pub weapon: ItemId,
    /// Projectile role.
    pub role: ProjectileRole,
    /// Launch time in seconds.
    pub time_seconds: f64,
    /// Launch body.
    pub body: BodyState,
}

/// Mirror of `WeaponTrajectoryUpdate` from donor
/// `src/contracts/weapon-behavior.ts` (canonical home:
/// `qa_content::contract`); unify post-merge.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WeaponTrajectoryUpdate {
    /// Forced origin.
    pub origin: Vec3,
    /// Forced velocity.
    pub velocity: Vec3,
    /// Forced angles.
    pub angles: Vec3,
}

/// One retained attachment, mirroring donor
/// `WeaponBehaviorAttachmentCheckpoint["attachments"][number]`.
#[derive(Debug, Clone, PartialEq)]
pub struct WeaponBehaviorAttachmentEntry {
    /// Attached projectile.
    pub projectile: SavedActorId,
    /// Source owner.
    pub owner: ProviderId,
    /// Behavior definition.
    pub definition: WeaponBehaviorDefinition,
}

/// Mirror of `WeaponBehaviorAttachmentCheckpoint` from donor
/// `src/contracts/weapon-behavior.ts` (canonical home:
/// `qa_content::contract`); unify post-merge.
#[derive(Debug, Clone, PartialEq)]
pub struct WeaponBehaviorAttachmentCheckpoint {
    /// Schema version (donor `version: 1`).
    pub version: u32,
    /// Retained attachments.
    pub attachments: Vec<WeaponBehaviorAttachmentEntry>,
}

/// Mirror of `WeaponBehaviorProjectilePort` from donor
/// `src/contracts/weapon-behavior.ts` (canonical home:
/// `qa_content::contract`); unify post-merge.
///
/// Donor throws become [`WeaponBehaviorRuntimeError`].
pub trait WeaponBehaviorProjectilePort {
    /// Whether the runtime steers a projectile.
    fn controls_trajectory(&self, projectile: &ActorId) -> bool;
    /// Launch a projectile, or [`None`] when no behavior matches the role.
    fn launch(
        &mut self,
        input: &WeaponBehaviorLaunch,
    ) -> Result<Option<WeaponTrajectoryUpdate>, WeaponBehaviorRuntimeError>;
    /// Step a projectile trajectory.
    fn step(
        &mut self,
        projectile: &OwnedActor,
        body: &BodyState,
        time_seconds: f64,
    ) -> Result<Option<WeaponTrajectoryUpdate>, WeaponBehaviorRuntimeError>;
}

/// Mirror of `WeaponBehaviorInstance` from donor
/// `src/contracts/weapon-behavior.ts` (canonical home:
/// `qa_content::contract`); unify post-merge.
pub trait WeaponBehaviorInstance {
    /// Launch update.
    fn initial(&self) -> &WeaponTrajectoryUpdate;
    /// Behavior definition.
    fn definition(&self) -> &WeaponBehaviorDefinition;
    /// Step the instance.
    fn step(&mut self, body: &BodyState, time_seconds: f64) -> Option<WeaponTrajectoryUpdate>;
    /// Release the instance.
    fn close(&mut self);
}

/// Mirror of `WeaponBehaviorSource` from donor
/// `src/contracts/weapon-behavior.ts` (canonical home:
/// `qa_content::contract`); unify post-merge.
pub trait WeaponBehaviorSource {
    /// Behavior definition.
    fn definition(&self) -> &WeaponBehaviorDefinition;
    /// Attach to a launch, or [`None`] when declined.
    fn attach(&mut self, launch: &WeaponBehaviorLaunch) -> Option<Box<dyn WeaponBehaviorInstance>>;
    /// Resume a restored projectile.
    fn resume(&mut self, projectile: &ActorId) -> Box<dyn WeaponBehaviorInstance>;
}

/// Mirror of `QcWeaponBehaviorTarget` from donor
/// `src/app/bootstrap/simulation/quakec-weapon-behavior.ts` (canonical home:
/// `crate::bootstrap::simulation::quakec_weapon_behavior`); unify post-merge.
#[derive(Debug, Clone, PartialEq)]
pub struct QcWeaponBehaviorTarget {
    /// Target actor.
    pub actor: ActorId,
    /// Target body.
    pub body: BodyState,
    /// Target health.
    pub health: f64,
    /// Target classname.
    pub classname: String,
    /// Target name, when named.
    pub name: Option<String>,
    /// Whether the target is solid.
    pub solid: bool,
}

/// Model reference behind the QuakeC `model` resolver (donor inline
/// `{ index, bounds }` in `weapon-behavior-runtime.ts`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WeaponModelRef {
    /// One-based model index.
    pub index: u32,
    /// Model bounds.
    pub bounds: Bounds,
}

/// Mirror of `ClassicGuestMap` from donor
/// `src/app/bootstrap/simulation/classic-guest-world.ts` (canonical home:
/// `crate::bootstrap::simulation::classic_guest_world`); unify post-merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassicGuestMap {
    /// Map path.
    pub map: String,
    /// Entity text.
    pub entities: String,
    /// Spawn point.
    pub spawn_point: String,
}

/// Mirror of `BindingKind` from donor
/// `src/app/bootstrap/simulation/rerelease-weapon-behavior.ts` (canonical
/// home: `crate::bootstrap::simulation::rerelease_weapon_behavior`); unify
/// post-merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RereleaseWeaponBindingKind {
    /// Client record binding.
    Client,
    /// Target binding.
    Target,
    /// Projectile binding.
    Projectile,
}

/// One saved configstring, mirroring donor
/// `RereleaseWeaponBehaviorCheckpoint["configstrings"][number]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RereleaseWeaponConfigString {
    /// Configstring index.
    pub index: u32,
    /// Configstring value.
    pub value: String,
}

/// One retired trajectory, mirroring donor
/// `RereleaseWeaponBehaviorCheckpoint["retired"][number]`.
#[derive(Debug, Clone, PartialEq)]
pub struct RereleaseWeaponRetired {
    /// Retired projectile.
    pub actor: SavedActorId,
    /// Final trajectory.
    pub trajectory: WeaponTrajectoryUpdate,
}

/// One actor binding, mirroring donor
/// `RereleaseWeaponBehaviorCheckpoint["bindings"][number]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RereleaseWeaponBinding {
    /// Entity slot.
    pub slot: u32,
    /// Bound actor.
    pub actor: SavedActorId,
    /// Binding kind.
    pub kind: RereleaseWeaponBindingKind,
    /// Profile generation.
    pub generation: u32,
}

/// Mirror of `RereleaseWeaponBehaviorCheckpoint` from donor
/// `src/app/bootstrap/simulation/rerelease-weapon-behavior.ts` (canonical
/// home: `crate::bootstrap::simulation::rerelease_weapon_behavior`); unify
/// post-merge.
#[derive(Debug, Clone, PartialEq)]
pub struct RereleaseWeaponBehaviorCheckpoint {
    /// Schema version (donor `version: 1`).
    pub version: u32,
    /// Behavior declaration.
    pub declaration: NativeWeaponBehaviorDeclaration,
    /// Behavior definition.
    pub definition: WeaponBehaviorDefinition,
    /// Guest map.
    pub map: ClassicGuestMap,
    /// Source time in seconds.
    pub time: f64,
    /// Saved game state.
    pub game: RereleaseSourceSave,
    /// Saved level state.
    pub level: RereleaseSourceSave,
    /// Saved cvar state.
    pub cvars: Vec<u8>,
    /// Saved configstrings.
    pub configstrings: Vec<RereleaseWeaponConfigString>,
    /// Retired trajectories.
    pub retired: Vec<RereleaseWeaponRetired>,
    /// Actor bindings.
    pub bindings: Vec<RereleaseWeaponBinding>,
}

/// Mirror of `RereleaseWeaponBehaviorSource` from donor
/// `src/app/bootstrap/simulation/rerelease-weapon-behavior.ts` (canonical
/// home: `crate::bootstrap::simulation::rerelease_weapon_behavior`); unify
/// post-merge.
///
/// The donor class becomes a trait because the guest partition owns every
/// method; the runtime only orchestrates it.
pub trait RereleaseWeaponBehaviorSource: WeaponBehaviorSource {
    /// Behavior declaration.
    fn declaration(&self) -> &NativeWeaponBehaviorDeclaration;
    /// Capture a checkpoint across loading frames.
    fn checkpoint(
        &self,
        next_frame: &mut dyn FnMut(),
    ) -> Result<RereleaseWeaponBehaviorCheckpoint, WeaponBehaviorRuntimeError>;
    /// Restore a checkpoint across loading frames.
    fn restore(
        &mut self,
        checkpoint: &RereleaseWeaponBehaviorCheckpoint,
        resolve: &mut dyn FnMut(SavedActorId) -> Result<OwnedActor, WeaponBehaviorRuntimeError>,
        next_frame: &mut dyn FnMut(),
    ) -> Result<(), WeaponBehaviorRuntimeError>;
    /// Release the source.
    fn close(&mut self) -> Result<(), WeaponBehaviorRuntimeError>;
}

/// QuakeC weapon behavior source (seam).
///
/// Donor `QuakeCWeaponBehaviorSource` (donor
/// `src/app/bootstrap/simulation/quakec-weapon-behavior.ts`, qc partition)
/// owns construction, checkpointing, restoration, and trajectory instances;
/// the runtime only orchestrates it. QuakeC sources have no `close` in the
/// donor: cleanup flows through the attachments driver. Fallible resolve
/// closures adapt donor throws across the callback boundary.
pub trait QuakeCWeaponBehaviorSource: WeaponBehaviorSource {
    /// Checkpoint type owned by the qc partition.
    type Checkpoint;
    /// Capture a checkpoint.
    fn checkpoint(&self) -> Self::Checkpoint;
    /// Trajectory random stream behind a checkpoint.
    fn random_checkpoint(checkpoint: &Self::Checkpoint) -> &RandomCheckpoint;
    /// Restore a checkpoint with resolved actors and a restored stream.
    fn restore(
        &mut self,
        checkpoint: &Self::Checkpoint,
        resolve: &mut dyn FnMut(SavedActorId) -> Result<OwnedActor, WeaponBehaviorRuntimeError>,
        random: SourceRandom,
    ) -> Result<(), WeaponBehaviorRuntimeError>;
}

/// QVM weapon behavior source (seam).
///
/// Donor `QvmWeaponBehaviorSource` (donor
/// `src/app/bootstrap/simulation/qvm-weapon-behavior.ts`, qc partition)
/// owns construction, checkpointing, restoration, and trajectory instances;
/// the runtime only orchestrates it.
pub trait QvmWeaponBehaviorSource: WeaponBehaviorSource {
    /// Checkpoint type owned by the qc partition.
    type Checkpoint;
    /// Capture a checkpoint.
    fn checkpoint(&self) -> Self::Checkpoint;
    /// Restore a checkpoint with resolved actors.
    fn restore(
        &mut self,
        checkpoint: &Self::Checkpoint,
        resolve: &mut dyn FnMut(SavedActorId) -> Result<ActorId, WeaponBehaviorRuntimeError>,
    ) -> Result<(), WeaponBehaviorRuntimeError>;
    /// Release the source.
    fn close(&mut self) -> Result<(), WeaponBehaviorRuntimeError>;
}

/// Weapon behavior attachments driver (seam).
///
/// Donor `WeaponBehaviorAttachments` (donor
/// `src/world/gameplay/weapon-behaviors.ts`, world partition) owns launch
/// attachment, trajectory queries, stepping, attachment checkpoints, and
/// instance cleanup. Sources are shared with the runtime through
/// `Rc<RefCell<..>>`, matching the donor aliasing (one source object in the
/// runtime map and the attachments map).
pub trait WeaponBehaviorAttachmentDriver {
    /// Register a source under its definition id.
    fn register(
        &mut self,
        id: &str,
        source: Rc<RefCell<dyn WeaponBehaviorSource>>,
    ) -> Result<(), WeaponBehaviorRuntimeError>;
    /// Launch through one registered source.
    fn launch(
        &mut self,
        selection: &str,
        input: &WeaponBehaviorLaunch,
    ) -> Result<Option<WeaponTrajectoryUpdate>, WeaponBehaviorRuntimeError>;
    /// Whether the driver steers a projectile.
    fn controls_trajectory(&self, projectile: &ActorId) -> bool;
    /// Step a projectile trajectory.
    fn step(
        &mut self,
        projectile: &OwnedActor,
        body: &BodyState,
        time_seconds: f64,
    ) -> Result<Option<WeaponTrajectoryUpdate>, WeaponBehaviorRuntimeError>;
    /// Capture the attachment checkpoint.
    fn checkpoint(&self) -> WeaponBehaviorAttachmentCheckpoint;
    /// Restore the attachment checkpoint.
    fn restore(&mut self, checkpoint: &WeaponBehaviorAttachmentCheckpoint) -> Result<(), WeaponBehaviorRuntimeError>;
    /// Release every instance.
    fn close(&mut self) -> Result<(), WeaponBehaviorRuntimeError>;
}

/// Saved-actor resolution for restores (seam).
///
/// Donor `SessionActorRegistry` (donor `src/world/actors/index.ts`, world
/// partition) owns these mappings; the runtime maps misses to
/// [`WeaponBehaviorRuntimeError::RetiredActor`], matching donor throw sites.
pub trait WeaponBehaviorActors {
    /// Reference a saved actor, or [`None`] when retired.
    fn reference_saved(&self, saved: SavedActorId) -> Option<OwnedActor>;
    /// Resolve a saved actor, or [`None`] when retired.
    fn resolve_saved(&self, saved: SavedActorId) -> Option<OwnedActor>;
}

/// Target enumeration for QuakeC construction.
pub type WeaponBehaviorTargets = Rc<dyn Fn() -> Vec<QcWeaponBehaviorTarget>>;
/// Aim probe for QuakeC construction.
pub type WeaponBehaviorAim = Rc<dyn Fn(&ActorId, f64) -> Vec3>;
/// Client print for QuakeC construction.
pub type WeaponBehaviorPrint = Rc<dyn Fn(Option<&ActorId>, &str)>;
/// Model resolver for QuakeC construction (owned: sources retain it past
/// the construction call, so the runtime hands over an `Rc`, not a borrow).
pub type WeaponBehaviorModel = Rc<dyn Fn(&str) -> Option<WeaponModelRef>>;
/// QuakeC source constructor.
pub type WeaponBehaviorQuakeC<Q, S> = Box<dyn FnMut(QuakeCWeaponContext<S>) -> Result<Q, WeaponBehaviorRuntimeError>>;
/// QVM source constructor.
pub type WeaponBehaviorQvm<V> = Box<dyn FnMut(&PreparedWeaponBehavior) -> Result<V, WeaponBehaviorRuntimeError>>;
/// Rerelease-native source constructor.
pub type WeaponBehaviorNative<N> =
    Box<dyn FnMut(&PreparedWeaponBehavior, &mut dyn FnMut()) -> Result<N, WeaponBehaviorRuntimeError>>;
/// QuakeC checkpoint reader.
pub type WeaponBehaviorReadQuakeC<Q> = Box<
    dyn for<'r> FnMut(
        SaveReader<'r>,
        &WeaponBehaviorDefinition,
    ) -> Result<<Q as QuakeCWeaponBehaviorSource>::Checkpoint, WeaponBehaviorRuntimeError>,
>;
/// QVM checkpoint reader.
pub type WeaponBehaviorReadQvm<V> = Box<
    dyn for<'r> FnMut(
        SaveReader<'r>,
        &WeaponBehaviorDefinition,
    ) -> Result<<V as QvmWeaponBehaviorSource>::Checkpoint, WeaponBehaviorRuntimeError>,
>;
/// Rerelease-native checkpoint reader.
pub type WeaponBehaviorReadRerelease = Box<
    dyn for<'r> FnMut(
        SaveReader<'r>,
        &WeaponBehaviorDefinition,
        &NativeWeaponBehaviorDeclaration,
    ) -> Result<RereleaseWeaponBehaviorCheckpoint, WeaponBehaviorRuntimeError>,
>;
/// Saved definition validator.
pub type WeaponBehaviorReadDefinition = Box<
    dyn for<'r> FnMut(
        SaveReader<'r>,
        &WeaponBehaviorDefinition,
    ) -> Result<WeaponBehaviorDefinition, WeaponBehaviorRuntimeError>,
>;
/// Attachments checkpoint reader.
pub type WeaponBehaviorReadAttachments = Box<
    dyn for<'r> FnMut(
        SaveReader<'r>,
        &HashMap<String, WeaponBehaviorDefinition>,
    ) -> Result<WeaponBehaviorAttachmentCheckpoint, WeaponBehaviorRuntimeError>,
>;

/// QuakeC source construction context: donor
/// `QuakeCWeaponBehaviorOptions` with owned snapshots.
///
/// The donor shares references into the prepared entry; the runtime hands
/// over owned definition/program snapshots instead so sources never borrow
/// caller entries.
pub struct QuakeCWeaponContext<S> {
    /// Behavior definition.
    pub definition: WeaponBehaviorDefinition,
    /// Behavior program.
    pub program: QcProgram,
    /// Trajectory random stream.
    pub random: SourceRandom,
    /// Shared scene queries.
    pub scene: Rc<S>,
    /// Simulation mode.
    pub mode: SimulationMode,
    /// Target enumeration.
    pub targets: WeaponBehaviorTargets,
    /// Aim probe.
    pub aim: WeaponBehaviorAim,
    /// Client print.
    pub print: WeaponBehaviorPrint,
    /// Model resolver.
    pub model: WeaponBehaviorModel,
}

/// Simulation weapon behavior host: construction hooks plus checkpoint readers.
///
/// Donor `WeaponBehaviorRuntimeHost` plus the QuakeC constructor and the save
/// readers, which the donor imports directly from the qc, guest, and world
/// partitions. Every hook takes the same [`super::types`] entries the donor
/// passes; hooks must clone retained entry data during the call because
/// entries are borrowed. `S` is the opaque scene queries object forwarded to
/// QuakeC construction (donor `Pick<SceneQueries, "trace" |
/// "pointContents">`); the `Rc` shares one scene object exactly like the
/// donor.
pub struct WeaponBehaviorRuntimeHost<
    Q: QuakeCWeaponBehaviorSource,
    V: QvmWeaponBehaviorSource,
    N: RereleaseWeaponBehaviorSource,
    S,
> {
    /// Simulation mode.
    pub mode: SimulationMode,
    /// Trajectory random seed.
    pub seed: u32,
    /// Target enumeration.
    pub targets: WeaponBehaviorTargets,
    /// Aim probe.
    pub aim: WeaponBehaviorAim,
    /// Client print.
    pub print: WeaponBehaviorPrint,
    /// Shared scene queries.
    pub scene: Rc<S>,
    /// Construct a QuakeC source (seam for donor
    /// `src/app/bootstrap/simulation/quakec-weapon-behavior.ts`).
    pub quakec: WeaponBehaviorQuakeC<Q, S>,
    /// Construct a QVM source (seam for donor
    /// `src/app/bootstrap/simulation/qvm-weapon-behavior.ts`).
    pub qvm: WeaponBehaviorQvm<V>,
    /// Construct a rerelease-native source (seam for donor
    /// `src/app/bootstrap/simulation/rerelease-weapon-behavior.ts`).
    pub native: WeaponBehaviorNative<N>,
    /// Read a QuakeC checkpoint (seam for donor
    /// `readQuakeCWeaponBehaviorCheckpoint`).
    pub read_quakec: WeaponBehaviorReadQuakeC<Q>,
    /// Read a QVM checkpoint (seam for donor
    /// `readQvmWeaponBehaviorCheckpoint`).
    pub read_qvm: WeaponBehaviorReadQvm<V>,
    /// Read a rerelease-native checkpoint (seam for donor
    /// `readRereleaseWeaponBehaviorCheckpoint`).
    pub read_rerelease: WeaponBehaviorReadRerelease,
    /// Validate a saved definition against the selection (seam for donor
    /// `readWeaponBehaviorDefinition`).
    pub read_definition: WeaponBehaviorReadDefinition,
    /// Read the attachments checkpoint (seam for donor
    /// `readWeaponBehaviorAttachmentCheckpoint`).
    pub read_attachments: WeaponBehaviorReadAttachments,
}

/// One saved source checkpoint by kind.
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum WeaponBehaviorSourceCheckpoint<Q, V> {
    /// QuakeC checkpoint.
    QuakeC(Q),
    /// Rerelease-native checkpoint.
    RereleaseNative(RereleaseWeaponBehaviorCheckpoint),
    /// QVM checkpoint.
    Qvm(V),
}

/// Sources read from a save in save order, paired with definition ids.
pub type WeaponBehaviorRestoredSources<Q, V> = Vec<(String, WeaponBehaviorSourceCheckpoint<Q, V>)>;

/// Runtime checkpoint, mirroring donor `WeaponBehaviorRuntimeCheckpoint`.
#[derive(Debug, Clone, PartialEq)]
pub struct WeaponBehaviorRuntimeCheckpoint<Q, V> {
    /// Schema version.
    pub version: u32,
    /// Source checkpoints in selection order.
    pub sources: Vec<WeaponBehaviorSourceCheckpoint<Q, V>>,
    /// Attachments checkpoint.
    pub attachments: WeaponBehaviorAttachmentCheckpoint,
}

/// One live source by kind.
enum WeaponBehaviorEntry<Q, V, N> {
    QuakeC(Rc<RefCell<Q>>),
    Qvm(Rc<RefCell<V>>),
    RereleaseNative(Rc<RefCell<N>>),
}

/// Selection order plus kind for checkpoint/restore dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WeaponBehaviorEntryKind {
    QuakeC,
    Qvm,
    RereleaseNative,
}

impl<Q, V, N> WeaponBehaviorEntry<Q, V, N> {
    fn kind(&self) -> WeaponBehaviorEntryKind {
        match self {
            WeaponBehaviorEntry::QuakeC(_) => WeaponBehaviorEntryKind::QuakeC,
            WeaponBehaviorEntry::Qvm(_) => WeaponBehaviorEntryKind::Qvm,
            WeaponBehaviorEntry::RereleaseNative(_) => WeaponBehaviorEntryKind::RereleaseNative,
        }
    }
}

/// Simulation weapon behaviors over injected sources and driver.
pub struct SimulationWeaponBehaviors<
    'p,
    Q: QuakeCWeaponBehaviorSource + 'static,
    V: QvmWeaponBehaviorSource + 'static,
    N: RereleaseWeaponBehaviorSource + 'static,
    D: WeaponBehaviorAttachmentDriver,
    A: WeaponBehaviorActors,
    S,
> {
    host: WeaponBehaviorRuntimeHost<Q, V, N, S>,
    attachments: D,
    actors: A,
    sources: HashMap<String, WeaponBehaviorEntry<Q, V, N>>,
    definitions: HashMap<ProjectileRole, WeaponBehaviorDefinition>,
    /// Selection order for deterministic checkpoints (donor `Map`
    /// insertion order; `HashMap` alone would serialize randomly).
    order: Vec<String>,
    deferred: Vec<&'p PreparedWeaponBehavior>,
    /// Queued restore as an owned snapshot: cloning frees callers from
    /// lending save data across the loading boundary.
    pending_restore: Option<(Option<SaveJson>, String)>,
    ready: bool,
}

impl<
        'p,
        Q: QuakeCWeaponBehaviorSource + 'static,
        V: QvmWeaponBehaviorSource + 'static,
        N: RereleaseWeaponBehaviorSource + 'static,
        D: WeaponBehaviorAttachmentDriver,
        A: WeaponBehaviorActors,
        S,
    > SimulationWeaponBehaviors<'p, Q, V, N, D, A, S>
{
    /// Build the runtime, constructing QuakeC sources and deferring QVM and
    /// native sources to [`initialize_loading`](Self::initialize_loading).
    pub fn new(
        prepared: &'p [PreparedWeaponBehavior],
        mut host: WeaponBehaviorRuntimeHost<Q, V, N, S>,
        actors: A,
        mut attachments: D,
    ) -> Result<Self, WeaponBehaviorRuntimeError> {
        let mut sources = HashMap::new();
        let mut definitions = HashMap::new();
        let mut order = Vec::new();
        let mut deferred = Vec::new();
        for entry in prepared {
            let definition = match entry {
                PreparedWeaponBehavior::QuakeC { selection, .. }
                | PreparedWeaponBehavior::Qvm { selection, .. }
                | PreparedWeaponBehavior::RereleaseNative { selection, .. } => &selection.definition,
            };
            if definitions.contains_key(&definition.role) {
                return Err(WeaponBehaviorRuntimeError::DuplicateRole(
                    projectile_role_name(definition.role).to_string(),
                ));
            }
            definitions.insert(definition.role, definition.clone());
            match entry {
                PreparedWeaponBehavior::QuakeC { program, resources, .. } => {
                    let models: Vec<(String, Bounds)> = resources
                        .iter()
                        .filter_map(|(path, resource)| resource.model_bounds.map(|bounds| (path.clone(), bounds)))
                        .collect();
                    let model: WeaponBehaviorModel = Rc::new(move |path| {
                        models
                            .iter()
                            .position(|(name, _)| name == path)
                            .map(|index| WeaponModelRef {
                                index: (index + 1) as u32,
                                bounds: models[index].1,
                            })
                    });
                    let context = QuakeCWeaponContext {
                        definition: definition.clone(),
                        program: program.clone(),
                        random: SourceRandom::new(host.seed),
                        scene: Rc::clone(&host.scene),
                        mode: host.mode,
                        targets: Rc::clone(&host.targets),
                        aim: Rc::clone(&host.aim),
                        print: Rc::clone(&host.print),
                        model,
                    };
                    let source = (host.quakec)(context)?;
                    let shared = Rc::new(RefCell::new(source));
                    attachments.register(&definition.id, shared.clone() as Rc<RefCell<dyn WeaponBehaviorSource>>)?;
                    order.push(definition.id.clone());
                    sources.insert(definition.id.clone(), WeaponBehaviorEntry::QuakeC(shared));
                }
                PreparedWeaponBehavior::Qvm { .. } | PreparedWeaponBehavior::RereleaseNative { .. } => {
                    deferred.push(entry)
                }
            }
        }
        let ready = deferred.is_empty();
        Ok(Self {
            host,
            attachments,
            actors,
            sources,
            definitions,
            order,
            deferred,
            pending_restore: None,
            ready,
        })
    }

    /// Whether deferred sources finished loading.
    #[must_use]
    pub fn ready(&self) -> bool {
        self.ready
    }

    /// Construct deferred QVM and native sources, then drain a queued restore.
    pub fn initialize_loading(&mut self, next_frame: &mut dyn FnMut()) -> Result<(), WeaponBehaviorRuntimeError> {
        if self.ready {
            return Ok(());
        }
        for index in 0..self.deferred.len() {
            let entry = self.deferred[index];
            match entry {
                PreparedWeaponBehavior::Qvm { .. } => {
                    let source = (self.host.qvm)(entry)?;
                    let id = source.definition().id.clone();
                    let shared = Rc::new(RefCell::new(source));
                    self.attachments
                        .register(&id, shared.clone() as Rc<RefCell<dyn WeaponBehaviorSource>>)?;
                    self.order.push(id.clone());
                    self.sources.insert(id, WeaponBehaviorEntry::Qvm(shared));
                }
                PreparedWeaponBehavior::RereleaseNative { .. } => {
                    let source = (self.host.native)(entry, next_frame)?;
                    let id = source.definition().id.clone();
                    let shared = Rc::new(RefCell::new(source));
                    self.attachments
                        .register(&id, shared.clone() as Rc<RefCell<dyn WeaponBehaviorSource>>)?;
                    self.order.push(id.clone());
                    self.sources.insert(id, WeaponBehaviorEntry::RereleaseNative(shared));
                }
                PreparedWeaponBehavior::QuakeC { .. } => {
                    unreachable!("quakec sources are never deferred")
                }
            }
        }
        self.ready = true;
        if let Some((value, path)) = self.pending_restore.take() {
            match value {
                Some(json) => self.restore_loading(SaveReader::at(&json, &path), next_frame)?,
                None => self.restore_loading(SaveReader::at(&SaveJson::Null, &path).field("queued"), next_frame)?,
            }
        }
        Ok(())
    }

    /// Launch a projectile, or [`None`] when no behavior matches the role.
    pub fn launch(
        &mut self,
        input: &WeaponBehaviorLaunch,
    ) -> Result<Option<WeaponTrajectoryUpdate>, WeaponBehaviorRuntimeError> {
        if !self.ready {
            return Err(WeaponBehaviorRuntimeError::LoadingRequired);
        }
        let Some(definition) = self.definitions.get(&input.role) else {
            return Ok(None);
        };
        let id = definition.id.clone();
        self.attachments.launch(&id, input)
    }

    /// Whether the runtime steers a projectile.
    #[must_use]
    pub fn controls_trajectory(&self, projectile: &ActorId) -> bool {
        self.attachments.controls_trajectory(projectile)
    }

    /// Step a projectile trajectory.
    pub fn step(
        &mut self,
        projectile: &OwnedActor,
        body: &BodyState,
        time_seconds: f64,
    ) -> Result<Option<WeaponTrajectoryUpdate>, WeaponBehaviorRuntimeError> {
        self.attachments.step(projectile, body, time_seconds)
    }

    /// Capture a checkpoint; native sources require
    /// [`checkpoint_loading`](Self::checkpoint_loading).
    pub fn checkpoint(
        &self,
    ) -> Result<WeaponBehaviorRuntimeCheckpoint<Q::Checkpoint, V::Checkpoint>, WeaponBehaviorRuntimeError> {
        if !self.ready {
            return Err(WeaponBehaviorRuntimeError::ComponentsNotLoaded);
        }
        // The deferred list permanently records native selection (the donor
        // never clears it), so native always needs the loading capture.
        if self
            .deferred
            .iter()
            .any(|entry| matches!(entry, PreparedWeaponBehavior::RereleaseNative { .. }))
        {
            return Err(WeaponBehaviorRuntimeError::NativeCheckpointRequiresLoading);
        }
        let mut sources = Vec::new();
        for id in &self.order {
            match &self.sources[id] {
                WeaponBehaviorEntry::QuakeC(source) => {
                    sources.push(WeaponBehaviorSourceCheckpoint::QuakeC(source.borrow().checkpoint()))
                }
                WeaponBehaviorEntry::Qvm(source) => {
                    sources.push(WeaponBehaviorSourceCheckpoint::Qvm(source.borrow().checkpoint()))
                }
                WeaponBehaviorEntry::RereleaseNative(_) => {}
            }
        }
        Ok(WeaponBehaviorRuntimeCheckpoint {
            version: WEAPON_BEHAVIOR_RUNTIME_CHECKPOINT_VERSION,
            sources,
            attachments: self.attachments.checkpoint(),
        })
    }

    /// Capture a checkpoint across loading frames.
    pub fn checkpoint_loading(
        &self,
        next_frame: &mut dyn FnMut(),
    ) -> Result<WeaponBehaviorRuntimeCheckpoint<Q::Checkpoint, V::Checkpoint>, WeaponBehaviorRuntimeError> {
        if !self.ready {
            return Err(WeaponBehaviorRuntimeError::ComponentsNotLoaded);
        }
        let mut sources = Vec::new();
        for id in &self.order {
            match &self.sources[id] {
                WeaponBehaviorEntry::QuakeC(source) => {
                    sources.push(WeaponBehaviorSourceCheckpoint::QuakeC(source.borrow().checkpoint()))
                }
                WeaponBehaviorEntry::Qvm(source) => {
                    sources.push(WeaponBehaviorSourceCheckpoint::Qvm(source.borrow().checkpoint()))
                }
                WeaponBehaviorEntry::RereleaseNative(source) => {
                    sources.push(WeaponBehaviorSourceCheckpoint::RereleaseNative(
                        source.borrow().checkpoint(next_frame)?,
                    ));
                }
            }
        }
        Ok(WeaponBehaviorRuntimeCheckpoint {
            version: WEAPON_BEHAVIOR_RUNTIME_CHECKPOINT_VERSION,
            sources,
            attachments: self.attachments.checkpoint(),
        })
    }

    /// Read and validate saved source checkpoints in save order.
    fn read_sources(
        &mut self,
        reader: &SaveReader<'_>,
    ) -> Result<WeaponBehaviorRestoredSources<Q::Checkpoint, V::Checkpoint>, WeaponBehaviorRuntimeError> {
        reader
            .field("version")
            .literal_i64(WEAPON_BEHAVIOR_RUNTIME_CHECKPOINT_VERSION.into())?;
        let mut seen = HashSet::new();
        let mut read = Vec::new();
        for value in reader.field("sources").list(Ok::<_, WeaponBehaviorRuntimeError>)? {
            let id = value.field("definition").field("id").string()?;
            let kind = match self.sources.get(&id) {
                Some(entry) => entry.kind(),
                None => {
                    return Err(value.fail("Saved weapon behavior differs from selection").into());
                }
            };
            if !seen.insert(id.clone()) {
                return Err(value.fail("Saved weapon behavior differs from selection").into());
            }
            let (definition, declaration) = match &self.sources[&id] {
                WeaponBehaviorEntry::QuakeC(source) => (source.borrow().definition().clone(), None),
                WeaponBehaviorEntry::Qvm(source) => (source.borrow().definition().clone(), None),
                WeaponBehaviorEntry::RereleaseNative(source) => {
                    let borrowed = source.borrow();
                    (borrowed.definition().clone(), Some(borrowed.declaration().clone()))
                }
            };
            let saved = (self.host.read_definition)(value.field("definition"), &definition)?;
            if self.definitions.get(&saved.role).map(|current| &current.id) != Some(&id) {
                return Err(value.fail("Saved weapon behavior differs from selection").into());
            }
            let checkpoint = match kind {
                WeaponBehaviorEntryKind::QuakeC => {
                    WeaponBehaviorSourceCheckpoint::QuakeC((self.host.read_quakec)(value, &definition)?)
                }
                WeaponBehaviorEntryKind::Qvm => {
                    WeaponBehaviorSourceCheckpoint::Qvm((self.host.read_qvm)(value, &definition)?)
                }
                WeaponBehaviorEntryKind::RereleaseNative => {
                    let declaration = declaration.expect("rerelease entry carries a declaration");
                    WeaponBehaviorSourceCheckpoint::RereleaseNative((self.host.read_rerelease)(
                        value,
                        &definition,
                        &declaration,
                    )?)
                }
            };
            read.push((id, checkpoint));
        }
        if seen.len() != self.sources.len() {
            return Err(reader.fail("Saved weapon behavior source is missing").into());
        }
        Ok(read)
    }

    /// Restore a checkpoint; native sources require
    /// [`restore_loading`](Self::restore_loading).
    pub fn restore(&mut self, reader: SaveReader<'_>) -> Result<(), WeaponBehaviorRuntimeError> {
        if reader.is_missing() && self.definitions.is_empty() {
            return Ok(());
        }
        if !self.ready {
            if self.pending_restore.is_some() {
                return Err(WeaponBehaviorRuntimeError::RestoreAlreadyQueued);
            }
            self.pending_restore = Some((reader.value.cloned(), reader.path().to_string()));
            return Ok(());
        }
        let read = self.read_sources(&reader)?;
        for (id, checkpoint) in &read {
            let entry = self
                .sources
                .get(id)
                .expect("saved weapon behavior resolved during read");
            match (entry, checkpoint) {
                (WeaponBehaviorEntry::QuakeC(source), WeaponBehaviorSourceCheckpoint::QuakeC(saved)) => {
                    if !matches!(Q::random_checkpoint(saved), RandomCheckpoint::Glibc(_)) {
                        return Err(reader.fail("QuakeC trajectory requires its source stream").into());
                    }
                    let mut random = SourceRandom::new(self.host.seed);
                    random.restore(Q::random_checkpoint(saved))?;
                    let mut resolve = |saved: SavedActorId| {
                        self.actors
                            .reference_saved(saved)
                            .ok_or(WeaponBehaviorRuntimeError::RetiredActor)
                    };
                    source.borrow_mut().restore(saved, &mut resolve, random)?;
                }
                (WeaponBehaviorEntry::Qvm(source), WeaponBehaviorSourceCheckpoint::Qvm(saved)) => {
                    let mut resolve = |saved: SavedActorId| {
                        self.actors
                            .resolve_saved(saved)
                            .map(|owned| owned.id().clone())
                            .ok_or(WeaponBehaviorRuntimeError::RetiredActor)
                    };
                    source.borrow_mut().restore(saved, &mut resolve)?;
                }
                (WeaponBehaviorEntry::RereleaseNative(_), _) => {
                    return Err(WeaponBehaviorRuntimeError::NativeRestoreRequiresLoading);
                }
                _ => unreachable!("checkpoint kind matches its source kind"),
            }
        }
        let by_id: HashMap<String, WeaponBehaviorDefinition> = self
            .definitions
            .values()
            .map(|definition| (definition.id.clone(), definition.clone()))
            .collect();
        let attachments = (self.host.read_attachments)(reader.field("attachments"), &by_id)?;
        self.attachments.restore(&attachments)
    }

    /// Restore a checkpoint across loading frames.
    pub fn restore_loading(
        &mut self,
        reader: SaveReader<'_>,
        next_frame: &mut dyn FnMut(),
    ) -> Result<(), WeaponBehaviorRuntimeError> {
        if reader.is_missing() && self.definitions.is_empty() {
            return Ok(());
        }
        let read = self.read_sources(&reader)?;
        for (id, checkpoint) in &read {
            let entry = self
                .sources
                .get(id)
                .expect("saved weapon behavior resolved during read");
            match (entry, checkpoint) {
                (WeaponBehaviorEntry::QuakeC(source), WeaponBehaviorSourceCheckpoint::QuakeC(saved)) => {
                    if !matches!(Q::random_checkpoint(saved), RandomCheckpoint::Glibc(_)) {
                        return Err(reader.fail("QuakeC trajectory requires its source stream").into());
                    }
                    let mut random = SourceRandom::new(self.host.seed);
                    random.restore(Q::random_checkpoint(saved))?;
                    let mut resolve = |saved: SavedActorId| {
                        self.actors
                            .reference_saved(saved)
                            .ok_or(WeaponBehaviorRuntimeError::RetiredActor)
                    };
                    source.borrow_mut().restore(saved, &mut resolve, random)?;
                }
                (WeaponBehaviorEntry::Qvm(source), WeaponBehaviorSourceCheckpoint::Qvm(saved)) => {
                    let mut resolve = |saved: SavedActorId| {
                        self.actors
                            .resolve_saved(saved)
                            .map(|owned| owned.id().clone())
                            .ok_or(WeaponBehaviorRuntimeError::RetiredActor)
                    };
                    source.borrow_mut().restore(saved, &mut resolve)?;
                }
                (
                    WeaponBehaviorEntry::RereleaseNative(source),
                    WeaponBehaviorSourceCheckpoint::RereleaseNative(saved),
                ) => {
                    let mut resolve = |saved: SavedActorId| {
                        self.actors
                            .resolve_saved(saved)
                            .ok_or(WeaponBehaviorRuntimeError::RetiredActor)
                    };
                    source.borrow_mut().restore(saved, &mut resolve, next_frame)?;
                }
                _ => unreachable!("checkpoint kind matches its source kind"),
            }
        }
        let by_id: HashMap<String, WeaponBehaviorDefinition> = self
            .definitions
            .values()
            .map(|definition| (definition.id.clone(), definition.clone()))
            .collect();
        let attachments = (self.host.read_attachments)(reader.field("attachments"), &by_id)?;
        self.attachments.restore(&attachments)
    }

    /// Release every instance and close QVM and native sources.
    pub fn close(&mut self) -> Result<(), WeaponBehaviorRuntimeError> {
        let mut failures = Vec::new();
        if let Err(error) = self.attachments.close() {
            failures.push(error.to_string());
        }
        for entry in self.sources.values() {
            let result = match entry {
                WeaponBehaviorEntry::QuakeC(_) => continue,
                WeaponBehaviorEntry::Qvm(source) => source.borrow_mut().close(),
                WeaponBehaviorEntry::RereleaseNative(source) => source.borrow_mut().close(),
            };
            if let Err(error) = result {
                failures.push(error.to_string());
            }
        }
        self.sources.clear();
        self.definitions.clear();
        self.order.clear();
        if failures.is_empty() {
            Ok(())
        } else {
            Err(WeaponBehaviorRuntimeError::CloseFailed(failures))
        }
    }
}

impl<
        'p,
        Q: QuakeCWeaponBehaviorSource + 'static,
        V: QvmWeaponBehaviorSource + 'static,
        N: RereleaseWeaponBehaviorSource + 'static,
        D: WeaponBehaviorAttachmentDriver,
        A: WeaponBehaviorActors,
        S,
    > WeaponBehaviorProjectilePort for SimulationWeaponBehaviors<'p, Q, V, N, D, A, S>
{
    fn controls_trajectory(&self, projectile: &ActorId) -> bool {
        self.controls_trajectory(projectile)
    }

    fn launch(
        &mut self,
        input: &WeaponBehaviorLaunch,
    ) -> Result<Option<WeaponTrajectoryUpdate>, WeaponBehaviorRuntimeError> {
        self.launch(input)
    }

    fn step(
        &mut self,
        projectile: &OwnedActor,
        body: &BodyState,
        time_seconds: f64,
    ) -> Result<Option<WeaponTrajectoryUpdate>, WeaponBehaviorRuntimeError> {
        self.step(projectile, body, time_seconds)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use qa_content::catalog::{
        resolve_qc_weapon_behavior, QcWeaponFunction, QcWeaponProgramSnapshot, SourceWeaponBehaviorMetadata,
        WeaponBehaviorCompatibility,
    };
    use qa_content::contract::{
        create_content_digest, ContentId, ContentMount, LooseMount, MountId, MountIdentity, MountPlanId,
        NativeWeaponAllocate, NativeWeaponCalls, NativeWeaponClient, NativeWeaponCommand, NativeWeaponEntity,
        NativeWeaponEntry, NativeWeaponEquipped, NativeWeaponFree, NativeWeaponRegistrationLayout, NativeWeaponThink,
        NativeWeaponTime, ProviderReference, ResolvedMountPlan, ResolvedWeaponBehaviorSelection,
    };
    use qa_content::mounts::{open_mount_plan, OpenMountOptions};
    use qa_core::identity::IdentityOwner;
    use qa_core::math::vec3;
    use qa_guest::core::contracts::{ContentDigest as GuestDigest, ModuleIdentity as GuestModule};
    use qa_guest::qc::program::{load_qc_program, QuakeCApi};
    use qa_guest::qvm::artifacts::{resolve_qvm_artifact, sha256_hex, KnownQvmArtifact, QvmProduct, QvmReplacement};
    use qa_guest::qvm::syscalls::{QvmAbiProfile, QvmRole};
    use qa_world::save::value::{arr, int, obj, str, SaveJson};

    use super::super::types::{
        NativeWeaponBehaviorDeclaration as OpaqueDeclaration, PreparedRereleaseGuest, QuakeCWeaponResource,
        QvmWeaponProfile,
    };
    use super::*;
    use crate::bootstrap::simulation::random::RandomProfile;

    // ---- stub sources ----

    #[derive(Debug, Clone, PartialEq)]
    struct StubQcCheckpoint {
        random: RandomCheckpoint,
    }

    #[derive(Debug, Clone, PartialEq)]
    struct StubQvmCheckpoint {
        marker: u32,
    }

    #[derive(Debug)]
    struct StubInstance {
        initial: WeaponTrajectoryUpdate,
        definition: WeaponBehaviorDefinition,
        closed: bool,
    }

    impl WeaponBehaviorInstance for StubInstance {
        fn initial(&self) -> &WeaponTrajectoryUpdate {
            &self.initial
        }

        fn definition(&self) -> &WeaponBehaviorDefinition {
            &self.definition
        }

        fn step(&mut self, _body: &BodyState, _time_seconds: f64) -> Option<WeaponTrajectoryUpdate> {
            Some(self.initial)
        }

        fn close(&mut self) {
            self.closed = true;
        }
    }

    fn stub_update() -> WeaponTrajectoryUpdate {
        WeaponTrajectoryUpdate {
            origin: vec3(1.0, 2.0, 3.0),
            velocity: vec3(4.0, 5.0, 6.0),
            angles: vec3(0.0, 90.0, 0.0),
        }
    }

    #[derive(Debug)]
    struct StubQc {
        definition: WeaponBehaviorDefinition,
        checkpoint: StubQcCheckpoint,
        restores: usize,
    }

    impl WeaponBehaviorSource for StubQc {
        fn definition(&self) -> &WeaponBehaviorDefinition {
            &self.definition
        }

        fn attach(&mut self, _launch: &WeaponBehaviorLaunch) -> Option<Box<dyn WeaponBehaviorInstance>> {
            Some(Box::new(StubInstance {
                initial: stub_update(),
                definition: self.definition.clone(),
                closed: false,
            }))
        }

        fn resume(&mut self, _projectile: &ActorId) -> Box<dyn WeaponBehaviorInstance> {
            Box::new(StubInstance {
                initial: stub_update(),
                definition: self.definition.clone(),
                closed: false,
            })
        }
    }

    impl QuakeCWeaponBehaviorSource for StubQc {
        type Checkpoint = StubQcCheckpoint;

        fn checkpoint(&self) -> StubQcCheckpoint {
            self.checkpoint.clone()
        }

        fn random_checkpoint(checkpoint: &StubQcCheckpoint) -> &RandomCheckpoint {
            &checkpoint.random
        }

        fn restore(
            &mut self,
            checkpoint: &StubQcCheckpoint,
            _resolve: &mut dyn FnMut(SavedActorId) -> Result<OwnedActor, WeaponBehaviorRuntimeError>,
            _random: SourceRandom,
        ) -> Result<(), WeaponBehaviorRuntimeError> {
            self.checkpoint = checkpoint.clone();
            self.restores += 1;
            Ok(())
        }
    }

    #[derive(Debug)]
    struct StubQvm {
        definition: WeaponBehaviorDefinition,
        checkpoint: StubQvmCheckpoint,
        restores: usize,
        closes: usize,
        fail_close: bool,
    }

    impl WeaponBehaviorSource for StubQvm {
        fn definition(&self) -> &WeaponBehaviorDefinition {
            &self.definition
        }

        fn attach(&mut self, _launch: &WeaponBehaviorLaunch) -> Option<Box<dyn WeaponBehaviorInstance>> {
            None
        }

        fn resume(&mut self, _projectile: &ActorId) -> Box<dyn WeaponBehaviorInstance> {
            Box::new(StubInstance {
                initial: stub_update(),
                definition: self.definition.clone(),
                closed: false,
            })
        }
    }

    impl QvmWeaponBehaviorSource for StubQvm {
        type Checkpoint = StubQvmCheckpoint;

        fn checkpoint(&self) -> StubQvmCheckpoint {
            self.checkpoint.clone()
        }

        fn restore(
            &mut self,
            checkpoint: &StubQvmCheckpoint,
            _resolve: &mut dyn FnMut(SavedActorId) -> Result<ActorId, WeaponBehaviorRuntimeError>,
        ) -> Result<(), WeaponBehaviorRuntimeError> {
            self.checkpoint = checkpoint.clone();
            self.restores += 1;
            Ok(())
        }

        fn close(&mut self) -> Result<(), WeaponBehaviorRuntimeError> {
            self.closes += 1;
            if self.fail_close {
                return Err(WeaponBehaviorRuntimeError::Seam("qvm close failed".to_string()));
            }
            Ok(())
        }
    }

    #[derive(Debug)]
    struct StubNative {
        definition: WeaponBehaviorDefinition,
        declaration: NativeWeaponBehaviorDeclaration,
        checkpoint: RereleaseWeaponBehaviorCheckpoint,
        restores: usize,
        closes: usize,
    }

    impl WeaponBehaviorSource for StubNative {
        fn definition(&self) -> &WeaponBehaviorDefinition {
            &self.definition
        }

        fn attach(&mut self, _launch: &WeaponBehaviorLaunch) -> Option<Box<dyn WeaponBehaviorInstance>> {
            None
        }

        fn resume(&mut self, _projectile: &ActorId) -> Box<dyn WeaponBehaviorInstance> {
            Box::new(StubInstance {
                initial: stub_update(),
                definition: self.definition.clone(),
                closed: false,
            })
        }
    }

    impl RereleaseWeaponBehaviorSource for StubNative {
        fn declaration(&self) -> &NativeWeaponBehaviorDeclaration {
            &self.declaration
        }

        fn checkpoint(
            &self,
            _next_frame: &mut dyn FnMut(),
        ) -> Result<RereleaseWeaponBehaviorCheckpoint, WeaponBehaviorRuntimeError> {
            Ok(self.checkpoint.clone())
        }

        fn restore(
            &mut self,
            checkpoint: &RereleaseWeaponBehaviorCheckpoint,
            _resolve: &mut dyn FnMut(SavedActorId) -> Result<OwnedActor, WeaponBehaviorRuntimeError>,
            next_frame: &mut dyn FnMut(),
        ) -> Result<(), WeaponBehaviorRuntimeError> {
            next_frame();
            self.checkpoint = checkpoint.clone();
            self.restores += 1;
            Ok(())
        }

        fn close(&mut self) -> Result<(), WeaponBehaviorRuntimeError> {
            self.closes += 1;
            Ok(())
        }
    }

    // ---- stub driver and actors ----

    #[derive(Debug, Default)]
    struct StubDriver {
        registered: Vec<String>,
        launched: Vec<String>,
        trajectory: HashSet<ActorId>,
        restored: Option<WeaponBehaviorAttachmentCheckpoint>,
        closed: bool,
        fail_close: bool,
    }

    impl WeaponBehaviorAttachmentDriver for StubDriver {
        fn register(
            &mut self,
            id: &str,
            _source: Rc<RefCell<dyn WeaponBehaviorSource>>,
        ) -> Result<(), WeaponBehaviorRuntimeError> {
            self.registered.push(id.to_string());
            Ok(())
        }

        fn launch(
            &mut self,
            selection: &str,
            _input: &WeaponBehaviorLaunch,
        ) -> Result<Option<WeaponTrajectoryUpdate>, WeaponBehaviorRuntimeError> {
            self.launched.push(selection.to_string());
            Ok(Some(stub_update()))
        }

        fn controls_trajectory(&self, projectile: &ActorId) -> bool {
            self.trajectory.contains(projectile)
        }

        fn step(
            &mut self,
            projectile: &OwnedActor,
            _body: &BodyState,
            _time_seconds: f64,
        ) -> Result<Option<WeaponTrajectoryUpdate>, WeaponBehaviorRuntimeError> {
            Ok(self.trajectory.contains(projectile.id()).then(stub_update))
        }

        fn checkpoint(&self) -> WeaponBehaviorAttachmentCheckpoint {
            WeaponBehaviorAttachmentCheckpoint {
                version: 1,
                attachments: Vec::new(),
            }
        }

        fn restore(
            &mut self,
            checkpoint: &WeaponBehaviorAttachmentCheckpoint,
        ) -> Result<(), WeaponBehaviorRuntimeError> {
            self.restored = Some(checkpoint.clone());
            Ok(())
        }

        fn close(&mut self) -> Result<(), WeaponBehaviorRuntimeError> {
            self.closed = true;
            if self.fail_close {
                return Err(WeaponBehaviorRuntimeError::Seam("driver close failed".to_string()));
            }
            Ok(())
        }
    }

    #[derive(Debug, Default)]
    struct StubActors {
        live: HashMap<SavedActorId, OwnedActor>,
    }

    impl WeaponBehaviorActors for StubActors {
        fn reference_saved(&self, saved: SavedActorId) -> Option<OwnedActor> {
            self.live.get(&saved).cloned()
        }

        fn resolve_saved(&self, saved: SavedActorId) -> Option<OwnedActor> {
            self.live.get(&saved).cloned()
        }
    }

    // ---- entry builders through public production paths ----

    fn test_digest() -> qa_content::contract::ContentDigest {
        create_content_digest(&"ab".repeat(32)).unwrap()
    }

    fn test_module() -> qa_content::contract::ModuleIdentity {
        qa_content::contract::ModuleIdentity {
            id: ProviderId::new("test", "behaviors"),
            artifact_path: "behaviors/mod".to_string(),
            digest: test_digest(),
            revision: "1".to_string(),
        }
    }

    fn definition(id: &str, role: ProjectileRole) -> WeaponBehaviorDefinition {
        let module = test_module();
        let program = QcWeaponProgramSnapshot {
            digest: module.digest.clone(),
            capability_error: None,
            functions: vec![QcWeaponFunction {
                index: 1,
                name: "fire".to_string(),
                first_statement: 0,
                parameter_words: 0,
            }],
            statements: Vec::new(),
            think_field_offset: None,
            initial_global_words: Vec::new(),
            function_globals: Vec::new(),
        };
        let metadata = SourceWeaponBehaviorMetadata {
            id: id.to_string(),
            title: format!("{id} title"),
            artifact_digest: module.digest.clone(),
            role,
            fire_function: "fire".to_string(),
            activation_function: None,
        };
        match resolve_qc_weapon_behavior(&module, &program, &metadata) {
            WeaponBehaviorCompatibility::Supported { definition } => definition,
            unsupported => panic!("expected supported behavior: {unsupported:?}"),
        }
    }

    fn artifact_mounts() -> (
        qa_content::mounts::MountedContent,
        qa_content::contract::ResolvedResourceReference,
    ) {
        static FIXTURE_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let seq = FIXTURE_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("qa-sim-net-weapon-runtime-{}-{seq}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("behavior"), b"stub").unwrap();
        let mount_id = MountId("mount:test:loose".to_string());
        let mount = ContentMount::Loose(LooseMount {
            identity: MountIdentity {
                id: mount_id.clone(),
                content: ContentId("test:content".to_string()),
                generation: 1,
            },
            root_path: dir.to_string_lossy().into_owned(),
        });
        let plan = ResolvedMountPlan {
            id: MountPlanId("mount-plan:test:1".to_string()),
            mounts: vec![mount],
            default_order: vec![mount_id],
            prefix_orders: Vec::new(),
        };
        let mounts = open_mount_plan(
            &plan,
            OpenMountOptions {
                pure: None,
                q3_restriction: None,
                links: Vec::new(),
                loose_comparison: None,
            },
        )
        .unwrap();
        let reference = mounts.resolve("behavior").unwrap().unwrap();
        (mounts, reference)
    }

    fn selection(definition: WeaponBehaviorDefinition) -> ResolvedWeaponBehaviorSelection {
        let (_, artifact) = artifact_mounts();
        ResolvedWeaponBehaviorSelection {
            component: None,
            source: ProviderReference {
                provider: ProviderId::new("test", "behaviors"),
                content: ContentId("test:content".to_string()),
            },
            artifact,
            definition,
        }
    }

    fn test_bounds() -> Bounds {
        Bounds {
            min: vec3(0.0, 0.0, 0.0),
            max: vec3(1.0, 1.0, 1.0),
        }
    }

    /// Minimal valid `progs.dat`: version 6, NetQuake CRC, one `Done`
    /// statement, the null function, one zero string byte, and 28 reserved
    /// global words.
    fn minimal_progs() -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&6i32.to_le_bytes());
        bytes.extend_from_slice(&5927i32.to_le_bytes());
        // statements, globals, fields, functions, strings, values.
        for (offset, count) in [(60, 1), (60, 0), (60, 0), (68, 1), (104, 1), (105, 28)] {
            bytes.extend_from_slice(&i32::to_le_bytes(offset));
            bytes.extend_from_slice(&i32::to_le_bytes(count));
        }
        bytes.extend_from_slice(&8i32.to_le_bytes());
        bytes.extend_from_slice(&[0u8; 8]);
        bytes.extend_from_slice(&[0u8; 36]);
        bytes.push(0);
        bytes.extend_from_slice(&[0u8; 112]);
        bytes
    }

    fn quakec_entry(id: &str, role: ProjectileRole) -> PreparedWeaponBehavior {
        let (mounts, resource) = artifact_mounts();
        PreparedWeaponBehavior::QuakeC {
            selection: selection(definition(id, role)),
            program: load_qc_program(&minimal_progs(), Some(QuakeCApi::Netquake), "test.dat").unwrap(),
            resources: HashMap::from([(
                "weapons/rocket.mdl".to_string(),
                QuakeCWeaponResource {
                    resource,
                    model_bounds: Some(test_bounds()),
                },
            )]),
            mounts,
        }
    }

    fn qvm_entry(id: &str, role: ProjectileRole) -> PreparedWeaponBehavior {
        let bytes = b"stub-qvm-image";
        let hex = sha256_hex(bytes);
        let module = GuestModule::new(
            ProviderId::new("qvm", "test"),
            "vm/test.qvm",
            GuestDigest::new("sha256", &hex),
            "1",
        );
        let replacement = QvmReplacement {
            artifact: KnownQvmArtifact {
                role: QvmRole::Qagame,
                product: QvmProduct::Baseq3,
                reference_package: "test".to_string(),
                related_game_build_date: "test".to_string(),
                byte_length: bytes.len(),
                digest: format!("sha256:{hex}"),
            },
            implementation: ProviderId::new("test", "qvm"),
            create: Box::new(|_, _| panic!("stub replacement never instantiates")),
        };
        let virtual_machine = resolve_qvm_artifact(
            &module,
            QvmRole::Qagame,
            bytes,
            vec![replacement],
            QvmAbiProfile::Modern,
        )
        .unwrap();
        let (mounts, _) = artifact_mounts();
        PreparedWeaponBehavior::Qvm {
            selection: selection(definition(id, role)),
            artifact: virtual_machine,
            profile: QvmWeaponProfile,
            mounts,
        }
    }

    fn declaration(id: &str, role: ProjectileRole) -> NativeWeaponBehaviorDeclaration {
        let entry = NativeWeaponEntry {
            rva: 0x100,
            registration: None,
        };
        NativeWeaponBehaviorDeclaration {
            version: 1,
            id: id.to_string(),
            title: format!("{id} title"),
            role,
            artifact_path: "game.dll".to_string(),
            artifact_digest: test_digest(),
            entity: NativeWeaponEntity {
                byte_length: 256,
                origin: 0,
                angles: 12,
                velocity: 24,
                client: 36,
                owner: 40,
                view_height: 44,
                generation: 48,
                next_think: 52,
                think_callback: 56,
                think_registration: 60,
                touch_callback: 64,
            },
            client: NativeWeaponClient {
                byte_length: 128,
                weapon: 0,
                view_angles: 8,
                forward: 20,
            },
            equipped_weapon: NativeWeaponEquipped {
                byte_length: 32,
                callback: 0,
                expected: entry.clone(),
            },
            time: NativeWeaponTime { rva: 0x300 },
            think: NativeWeaponThink {
                tag: 7,
                registration: NativeWeaponRegistrationLayout {
                    byte_length: 32,
                    name: 0,
                    tag: 8,
                    callback: 16,
                },
            },
            allocate: NativeWeaponAllocate { entry: entry.clone() },
            free: NativeWeaponFree { entry: entry.clone() },
            projectile_touch: entry.clone(),
            equip: NativeWeaponCalls { calls: Vec::new() },
            launch: NativeWeaponCalls { calls: Vec::new() },
            activate_rva: None,
            fire_rva: 0x500,
            initialization_classes: Vec::new(),
            equipment: Vec::new(),
            ammunition: NativeWeaponCommand {
                arguments: Vec::new(),
                tail: String::new(),
            },
            initial_cvars: Vec::new(),
            provisioning_cvars: Vec::new(),
        }
    }

    fn native_entry(id: &str, role: ProjectileRole) -> PreparedWeaponBehavior {
        let definition = definition(id, role);
        let (mounts, _) = artifact_mounts();
        PreparedWeaponBehavior::RereleaseNative {
            selection: selection(definition),
            prepared: PreparedRereleaseGuest,
            declaration: OpaqueDeclaration,
            mounts,
        }
    }

    struct HostProbes {
        quakec_contexts: usize,
        model_index: Option<u32>,
        pumps: Rc<Cell<usize>>,
    }

    fn host(probes: Rc<RefCell<HostProbes>>) -> WeaponBehaviorRuntimeHost<StubQc, StubQvm, StubNative, u32> {
        let pumps = Rc::clone(&probes.borrow().pumps);
        WeaponBehaviorRuntimeHost {
            mode: SimulationMode::Deathmatch,
            seed: 11,
            targets: Rc::new(Vec::new),
            aim: Rc::new(|_, _| vec3(0.0, 0.0, 0.0)),
            print: Rc::new(|_, _| {}),
            scene: Rc::new(5),
            quakec: Box::new(move |context: QuakeCWeaponContext<u32>| {
                let mut probes = probes.borrow_mut();
                probes.quakec_contexts += 1;
                probes.model_index = (context.model)("weapons/rocket.mdl").map(|found| found.index);
                assert_eq!(context.mode, SimulationMode::Deathmatch);
                assert_eq!(*context.scene, 5);
                let random = context.random.checkpoint();
                Ok(StubQc {
                    definition: context.definition,
                    checkpoint: StubQcCheckpoint { random },
                    restores: 0,
                })
            }),
            qvm: Box::new(|entry| match entry {
                PreparedWeaponBehavior::Qvm { selection, .. } => Ok(StubQvm {
                    definition: selection.definition.clone(),
                    checkpoint: StubQvmCheckpoint { marker: 7 },
                    restores: 0,
                    closes: 0,
                    fail_close: false,
                }),
                _ => panic!("qvm hook takes qvm entries"),
            }),
            native: Box::new(move |entry, next_frame| {
                next_frame();
                pumps.set(pumps.get() + 1);
                match entry {
                    PreparedWeaponBehavior::RereleaseNative { selection, .. } => {
                        // The entry carries the opaque hub declaration; the
                        // stub resolves the content declaration the way the
                        // guest partition will.
                        let resolved = declaration(&selection.definition.id, selection.definition.role);
                        Ok(StubNative {
                            definition: selection.definition.clone(),
                            declaration: resolved.clone(),
                            checkpoint: RereleaseWeaponBehaviorCheckpoint {
                                version: 1,
                                declaration: resolved,
                                definition: selection.definition.clone(),
                                map: ClassicGuestMap {
                                    map: "maps/base1.bsp".to_string(),
                                    entities: String::new(),
                                    spawn_point: "start".to_string(),
                                },
                                time: 0.0,
                                game: RereleaseSourceSave {
                                    native: Vec::new(),
                                    deferred_damage: Vec::new(),
                                    projections: Vec::new(),
                                },
                                level: RereleaseSourceSave {
                                    native: Vec::new(),
                                    deferred_damage: Vec::new(),
                                    projections: Vec::new(),
                                },
                                cvars: Vec::new(),
                                configstrings: Vec::new(),
                                retired: Vec::new(),
                                bindings: Vec::new(),
                            },
                            restores: 0,
                            closes: 0,
                        })
                    }
                    _ => panic!("native hook takes native entries"),
                }
            }),
            read_quakec: Box::new(|_, definition| {
                let _ = definition;
                Ok(StubQcCheckpoint {
                    random: SourceRandom::new(11).checkpoint(),
                })
            }),
            read_qvm: Box::new(|_, _| Ok(StubQvmCheckpoint { marker: 9 })),
            read_rerelease: Box::new(|_, definition, declaration| {
                Ok(RereleaseWeaponBehaviorCheckpoint {
                    version: 1,
                    declaration: declaration.clone(),
                    definition: definition.clone(),
                    map: ClassicGuestMap {
                        map: "maps/base1.bsp".to_string(),
                        entities: String::new(),
                        spawn_point: "start".to_string(),
                    },
                    time: 1.0,
                    game: RereleaseSourceSave {
                        native: Vec::new(),
                        deferred_damage: Vec::new(),
                        projections: Vec::new(),
                    },
                    level: RereleaseSourceSave {
                        native: Vec::new(),
                        deferred_damage: Vec::new(),
                        projections: Vec::new(),
                    },
                    cvars: Vec::new(),
                    configstrings: Vec::new(),
                    retired: Vec::new(),
                    bindings: Vec::new(),
                })
            }),
            read_definition: Box::new(|_, expected| Ok(expected.clone())),
            read_attachments: Box::new(|_, _| {
                Ok(WeaponBehaviorAttachmentCheckpoint {
                    version: 1,
                    attachments: Vec::new(),
                })
            }),
        }
    }

    fn probes() -> Rc<RefCell<HostProbes>> {
        Rc::new(RefCell::new(HostProbes {
            quakec_contexts: 0,
            model_index: None,
            pumps: Rc::new(Cell::new(0)),
        }))
    }

    type StubRuntime<'p> = SimulationWeaponBehaviors<'p, StubQc, StubQvm, StubNative, StubDriver, StubActors, u32>;

    fn runtime<'p>(prepared: &'p [PreparedWeaponBehavior], probes: Rc<RefCell<HostProbes>>) -> StubRuntime<'p> {
        StubRuntime::new(prepared, host(probes), StubActors::default(), StubDriver::default()).unwrap()
    }

    fn checkpoint_save(ids: &[&str]) -> SaveJson {
        obj(vec![
            ("version", int(1)),
            (
                "sources",
                arr(ids
                    .iter()
                    .map(|id| {
                        obj(vec![
                            ("definition", obj(vec![("id", str(id))])),
                            ("checkpoint", obj(Vec::new())),
                        ])
                    })
                    .collect()),
            ),
            (
                "attachments",
                obj(vec![("version", int(1)), ("attachments", arr(Vec::new()))]),
            ),
        ])
    }

    #[test]
    fn rejects_duplicate_roles() {
        let prepared = vec![
            quakec_entry("test:rocket-a", ProjectileRole::Rocket),
            qvm_entry("test:rocket-b", ProjectileRole::Rocket),
        ];
        let Err(error) = StubRuntime::new(&prepared, host(probes()), StubActors::default(), StubDriver::default())
        else {
            panic!("expected duplicate roles");
        };
        assert_eq!(error.to_string(), "Multiple selected trajectory behaviors for rocket");
    }

    #[test]
    fn constructs_quakec_and_defers_rest() {
        let prepared = vec![
            quakec_entry("test:rocket", ProjectileRole::Rocket),
            qvm_entry("test:grenade", ProjectileRole::Grenade),
            native_entry("test:nail", ProjectileRole::Nail),
        ];
        let seen = probes();
        let behaviors = runtime(&prepared, Rc::clone(&seen));
        assert!(!behaviors.ready());
        let seen = seen.borrow();
        assert_eq!(seen.quakec_contexts, 1);
        assert_eq!(seen.model_index, Some(1));
        assert_eq!(seen.pumps.get(), 0);
    }

    #[test]
    fn initialize_loading_builds_deferred_in_order() {
        let prepared = vec![
            quakec_entry("test:rocket", ProjectileRole::Rocket),
            qvm_entry("test:grenade", ProjectileRole::Grenade),
            native_entry("test:nail", ProjectileRole::Nail),
        ];
        let seen = probes();
        let mut behaviors = runtime(&prepared, Rc::clone(&seen));
        let mut pumps = 0;
        behaviors.initialize_loading(&mut || pumps += 1).unwrap();
        assert!(behaviors.ready());
        assert_eq!(seen.borrow().pumps.get(), 1);
        assert_eq!(pumps, 1);
        let checkpoint = behaviors.checkpoint_loading(&mut || {}).unwrap();
        assert_eq!(checkpoint.version, 1);
        assert_eq!(checkpoint.sources.len(), 3);
        assert!(matches!(
            checkpoint.sources[0],
            WeaponBehaviorSourceCheckpoint::QuakeC(_)
        ));
        assert!(matches!(checkpoint.sources[1], WeaponBehaviorSourceCheckpoint::Qvm(_)));
        assert!(matches!(
            checkpoint.sources[2],
            WeaponBehaviorSourceCheckpoint::RereleaseNative(_)
        ));
    }

    #[test]
    fn launch_requires_ready_and_dispatches_by_role() {
        let prepared = vec![qvm_entry("test:grenade", ProjectileRole::Grenade)];
        let mut behaviors = runtime(&prepared, probes());
        let owner = IdentityOwner::create("launch-test").unwrap();
        let projectile = owner
            .owned_actor(&owner.actor(3, 1), ProviderId::new("test", "game"))
            .unwrap();
        let input = WeaponBehaviorLaunch {
            projectile: projectile.clone(),
            shooter: owner.actor(1, 1),
            weapon: "test:weapon/grenade".to_string(),
            role: ProjectileRole::Grenade,
            time_seconds: 2.0,
            body: BodyState {
                origin: vec3(0.0, 0.0, 0.0),
                angles: vec3(0.0, 0.0, 0.0),
                velocity: vec3(0.0, 0.0, 0.0),
                bounds: qa_core::math::Bounds {
                    min: vec3(-8.0, -8.0, -8.0),
                    max: vec3(8.0, 8.0, 8.0),
                },
                ground: None,
            },
        };
        assert!(matches!(
            behaviors.launch(&input).unwrap_err(),
            WeaponBehaviorRuntimeError::LoadingRequired
        ));
        behaviors.initialize_loading(&mut || {}).unwrap();
        let update = behaviors.launch(&input).unwrap().unwrap();
        assert_eq!(update.origin, vec3(1.0, 2.0, 3.0));
        let missing = WeaponBehaviorLaunch {
            role: ProjectileRole::Bolt,
            ..input.clone()
        };
        assert!(behaviors.launch(&missing).unwrap().is_none());
        assert!(!behaviors.controls_trajectory(projectile.id()));
        assert!(behaviors.step(&projectile, &input.body, 3.0).unwrap().is_none());
    }

    #[test]
    fn checkpoint_rejects_native_without_loading() {
        let prepared = vec![native_entry("test:nail", ProjectileRole::Nail)];
        let mut behaviors = runtime(&prepared, probes());
        assert!(matches!(
            behaviors.checkpoint().unwrap_err(),
            WeaponBehaviorRuntimeError::ComponentsNotLoaded
        ));
        behaviors.initialize_loading(&mut || {}).unwrap();
        assert!(matches!(
            behaviors.checkpoint().unwrap_err(),
            WeaponBehaviorRuntimeError::NativeCheckpointRequiresLoading
        ));
        let checkpoint = behaviors.checkpoint_loading(&mut || {}).unwrap();
        assert_eq!(checkpoint.sources.len(), 1);
        assert!(matches!(
            checkpoint.sources[0],
            WeaponBehaviorSourceCheckpoint::RereleaseNative(_)
        ));
    }

    #[test]
    fn restore_round_trips_quakec_and_qvm() {
        let prepared = vec![
            quakec_entry("test:rocket", ProjectileRole::Rocket),
            qvm_entry("test:grenade", ProjectileRole::Grenade),
        ];
        let mut behaviors = runtime(&prepared, probes());
        assert!(!behaviors.ready());
        let save = checkpoint_save(&["test:rocket", "test:grenade"]);
        behaviors.restore(SaveReader::at(&save, "weapons")).unwrap();
        behaviors.initialize_loading(&mut || {}).unwrap();
        assert!(behaviors.ready());
        match &behaviors.sources["test:rocket"] {
            WeaponBehaviorEntry::QuakeC(source) => assert_eq!(source.borrow().restores, 1),
            _ => panic!("quakec entry"),
        }
        match &behaviors.sources["test:grenade"] {
            WeaponBehaviorEntry::Qvm(source) => assert_eq!(source.borrow().restores, 1),
            _ => panic!("qvm entry"),
        }
    }

    #[test]
    fn restore_rejects_non_glibc_random() {
        let prepared = vec![quakec_entry("test:rocket", ProjectileRole::Rocket)];
        let seen = probes();
        let mut host = host(Rc::clone(&seen));
        host.read_quakec = Box::new(|_, _| {
            Ok(StubQcCheckpoint {
                random: SourceRandom::with_profile(11, RandomProfile::Q2Rerelease).checkpoint(),
            })
        });
        let mut behaviors = StubRuntime::new(&prepared, host, StubActors::default(), StubDriver::default()).unwrap();
        let save = checkpoint_save(&["test:rocket"]);
        let error = behaviors.restore(SaveReader::at(&save, "weapons")).unwrap_err();
        assert!(error.to_string().contains("source stream"), "{error}");
    }

    #[test]
    fn restore_queues_before_ready_and_rejects_double_queue() {
        let prepared = vec![qvm_entry("test:grenade", ProjectileRole::Grenade)];
        let mut behaviors = runtime(&prepared, probes());
        let save = checkpoint_save(&["test:grenade"]);
        behaviors.restore(SaveReader::at(&save, "weapons")).unwrap();
        let queued = checkpoint_save(&["test:grenade"]);
        assert!(matches!(
            behaviors.restore(SaveReader::at(&queued, "weapons")).unwrap_err(),
            WeaponBehaviorRuntimeError::RestoreAlreadyQueued
        ));
        behaviors.initialize_loading(&mut || {}).unwrap();
        match &behaviors.sources["test:grenade"] {
            WeaponBehaviorEntry::Qvm(source) => assert_eq!(source.borrow().restores, 1),
            _ => panic!("qvm entry"),
        }
    }

    #[test]
    fn restore_rejects_mismatched_duplicate_and_missing() {
        let prepared = vec![
            quakec_entry("test:rocket", ProjectileRole::Rocket),
            qvm_entry("test:grenade", ProjectileRole::Grenade),
        ];
        let mut behaviors = runtime(&prepared, probes());
        behaviors.initialize_loading(&mut || {}).unwrap();
        for ids in [
            vec!["test:rocket", "test:unknown"],
            vec!["test:rocket", "test:rocket"],
            vec!["test:rocket"],
        ] {
            let save = checkpoint_save(&ids);
            assert!(behaviors.restore(SaveReader::at(&save, "weapons")).is_err(), "{ids:?}");
        }
    }

    #[test]
    fn restore_rejects_native_without_loading() {
        let prepared = vec![native_entry("test:nail", ProjectileRole::Nail)];
        let mut behaviors = runtime(&prepared, probes());
        behaviors.initialize_loading(&mut || {}).unwrap();
        let save = checkpoint_save(&["test:nail"]);
        assert!(matches!(
            behaviors.restore(SaveReader::at(&save, "weapons")).unwrap_err(),
            WeaponBehaviorRuntimeError::NativeRestoreRequiresLoading
        ));
        let mut pumps = 0;
        behaviors
            .restore_loading(SaveReader::at(&save, "weapons"), &mut || pumps += 1)
            .unwrap();
        assert_eq!(pumps, 1);
        match &behaviors.sources["test:nail"] {
            WeaponBehaviorEntry::RereleaseNative(source) => assert_eq!(source.borrow().restores, 1),
            _ => panic!("native entry"),
        }
    }

    #[test]
    fn close_collects_failures_and_clears() {
        let prepared = vec![
            quakec_entry("test:rocket", ProjectileRole::Rocket),
            qvm_entry("test:grenade", ProjectileRole::Grenade),
        ];
        let seen = probes();
        let mut host = host(Rc::clone(&seen));
        host.qvm = Box::new(|entry| match entry {
            PreparedWeaponBehavior::Qvm { selection, .. } => Ok(StubQvm {
                definition: selection.definition.clone(),
                checkpoint: StubQvmCheckpoint { marker: 7 },
                restores: 0,
                closes: 0,
                fail_close: true,
            }),
            _ => panic!("qvm hook takes qvm entries"),
        });
        let driver = StubDriver {
            fail_close: true,
            ..StubDriver::default()
        };
        let mut behaviors = StubRuntime::new(&prepared, host, StubActors::default(), driver).unwrap();
        behaviors.initialize_loading(&mut || {}).unwrap();
        let error = behaviors.close().unwrap_err();
        match error {
            WeaponBehaviorRuntimeError::CloseFailed(failures) => assert_eq!(failures.len(), 2),
            _ => panic!("expected close failures: {error}"),
        }
        assert!(behaviors.sources.is_empty());
        assert!(behaviors.definitions.is_empty());
    }

    #[test]
    fn projectile_port_delegates() {
        let prepared = vec![qvm_entry("test:grenade", ProjectileRole::Grenade)];
        let mut behaviors = runtime(&prepared, probes());
        behaviors.initialize_loading(&mut || {}).unwrap();
        let owner = IdentityOwner::create("port-test").unwrap();
        let projectile = owner
            .owned_actor(&owner.actor(3, 1), ProviderId::new("test", "game"))
            .unwrap();
        let body = BodyState {
            origin: vec3(0.0, 0.0, 0.0),
            angles: vec3(0.0, 0.0, 0.0),
            velocity: vec3(0.0, 0.0, 0.0),
            bounds: qa_core::math::Bounds {
                min: vec3(-8.0, -8.0, -8.0),
                max: vec3(8.0, 8.0, 8.0),
            },
            ground: None,
        };
        let port: &mut dyn WeaponBehaviorProjectilePort = &mut behaviors;
        assert!(!port.controls_trajectory(projectile.id()));
        let launched = port
            .launch(&WeaponBehaviorLaunch {
                projectile: projectile.clone(),
                shooter: owner.actor(1, 1),
                weapon: "test:weapon/grenade".to_string(),
                role: ProjectileRole::Grenade,
                time_seconds: 1.0,
                body: body.clone(),
            })
            .unwrap();
        assert!(launched.is_some());
        assert!(port.step(&projectile, &body, 2.0).unwrap().is_none());
    }
}
