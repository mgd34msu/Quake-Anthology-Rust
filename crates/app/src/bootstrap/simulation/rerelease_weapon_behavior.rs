//! Rerelease native weapon trajectory component.
//!
//! Port of donor `src/app/bootstrap/simulation/rerelease-weapon-behavior.ts`
//! (`retireRereleaseWeaponActor`, `RereleaseWeaponBehaviorSource` and the
//! behavior options/checkpoint shapes).
//!
//! `NativeWeaponBehaviorDeclaration` is imported from
//! [`qa_compat::q2::rerelease::native_weapon_declaration`]: the worktree
//! already carries the exact donor shape with its `read`/`same` support, so
//! redefining it here would fork the type and break interop with the
//! declaration readers. The coordinator should swap any opaque to the compat
//! type.
//!
//! The Rust [`RereleaseWeaponProfile`] drives a headless
//! [`SyntheticWeaponModule`] instead of guest memory: each binding keeps a
//! host slot (entities, capture, inuse) plus a module slot (profile state).
//! Capture seeds the module record from the host record's origin/angles;
//! game-private fields (velocity, think scheduling, generation) start zeroed
//! because no game code runs. The DLL import interception splits in two: the
//! source hook answers the pure cases (forbidden imports panic like the
//! donor throws; network/sound/print imports sink to void), while
//! [`RereleaseWeaponBehaviorSource::intercept_link`] ports the
//! linkentity/unlinkentity memory math for drivers to call directly.
//! Trajectory instances are data plus source-side step/close methods (Rust
//! cannot capture the source in a closure); engine release observation is a
//! [`RereleaseWeaponBehaviorSource::notify_actor_released`] driver hook (the
//! engine registry has no release subscription); cvar transfer state for
//! checkpoints is an options seam (the registry has no transfer capture).

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use qa_compat::q2::rerelease::host::SourceSave;
use qa_compat::q2::rerelease::layouts::{edict_layout, field_offset};
use qa_compat::q2::rerelease::native_weapon_declaration::{
    same_native_weapon_declaration, NativeWeaponBehaviorDeclaration,
};
use qa_compat::q2::rerelease::weapon_behavior_profile::{
    weapon_initialization_entities, with_weapon_provisioning, RereleaseWeaponProfile, SyntheticWeaponModule,
    WeaponBehaviorError, WeaponBodyState, WeaponClientFields, WeaponCommand, WeaponCvar, WeaponCvarRegistry,
    WeaponEntityFields, WeaponProfileDeclaration, WeaponShooter,
};
use qa_content::contract::{
    same_weapon_behavior, ContentDigest, ModuleIdentity as ContentModuleIdentity, NativeAbi,
    NativeCallAbi as ContentAbi, ProjectileRole, WeaponBehaviorCallback, WeaponBehaviorDefinition,
};
use qa_content::paths::normalize_resource_path;
use qa_content::q2::foundation::host::{Q2FoundationHost, Q2PresentationEvent};
use qa_content::q2::support::contracts::{BodyState as EngineBodyState, WeaponBehaviorLaunch, WeaponTrajectoryUpdate};
use qa_core::cvar::CvarRegistry;
use qa_core::identity::{ActorId, OwnedActor, SavedActorId};
use qa_core::math::{Bounds, Vec3};
use qa_guest::core::contracts::{GuestCallResult, GuestCallValue};
use qa_guest::core::memory::SparseGuestMemory;

use super::classic_guest_services::GuestCommandLine;
use super::classic_guest_world::ClassicGuestMap;
use super::q2_native_world::native_module_identity_parts;
use super::rerelease_guest_services::RereleaseGuestServices;
use super::rerelease_guest_services_contract::{RereleaseGuestServicesOptions, RereleaseGuestServicesPort};
use super::rerelease_guest_source::{
    GuestClock, InterceptImport, PreparedRereleaseGuest, RereleaseGuestSource, RereleaseGuestSourceOptions,
};
use super::types::RereleaseSemanticBindings;
use crate::persistence::recipe::ExecutionImplementation;

/// Weapon component error.
#[derive(Debug, thiserror::Error)]
pub enum RereleaseWeaponError {
    /// Invalid component use or data.
    #[error("invalid API2023 weapon component: {0}")]
    Invalid(String),
    /// Host or guest failure.
    #[error("API2023 weapon host failure: {0}")]
    Host(String),
}

impl RereleaseWeaponError {
    /// Invalid-use error.
    #[must_use]
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::Invalid(message.into())
    }
}

/// Weapon component result.
pub type RereleaseWeaponResult<T> = Result<T, RereleaseWeaponError>;

fn host_mapped(error: impl ToString) -> RereleaseWeaponError {
    RereleaseWeaponError::Host(error.to_string())
}

fn weapon_mapped(error: WeaponBehaviorError) -> RereleaseWeaponError {
    RereleaseWeaponError::invalid(error.to_string())
}

/// Authority marker: source inventory stays private; the primary keeps
/// trigger, ammo, impact and presentation authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WeaponBehaviorOwnership;

/// Shooter snapshot plus engine identity for weapon binding.
#[derive(Debug, Clone)]
pub struct RereleaseWeaponActor {
    /// Shooter body.
    pub body: EngineBodyState,
    /// View angles.
    pub view_angles: Vec3,
    /// View height.
    pub view_height: i32,
    /// Engine actor.
    pub actor: OwnedActor,
    /// Shooter userinfo.
    pub userinfo: String,
}

/// Cvar transfer-state capture seam (donor
/// `CvarRegistry.captureWorldTransferState`, canonical home:
/// `qa_core::cvar`); unify post-merge.
pub type RereleaseCvarCaptureFn = Rc<dyn Fn(&CvarRegistry) -> Vec<u8>>;

/// Cvar transfer-state restore seam (donor
/// `CvarRegistry.restoreSaveState`, canonical home: `qa_core::cvar`); unify
/// post-merge.
pub type RereleaseCvarRestoreFn = Rc<dyn Fn(&mut CvarRegistry, &[u8]) -> RereleaseWeaponResult<()>>;

/// Construction options for [`RereleaseWeaponBehaviorSource::create`].
pub struct RereleaseWeaponBehaviorOptions {
    /// Prepared guest module.
    pub prepared: PreparedRereleaseGuest,
    /// Accepted behavior declaration.
    pub declaration: NativeWeaponBehaviorDeclaration,
    /// Guest import services options.
    pub services: RereleaseGuestServicesOptions,
    /// Guest clock.
    pub clock: GuestClock,
    /// Component map.
    pub map: ClassicGuestMap,
    /// Authority marker.
    pub ownership: WeaponBehaviorOwnership,
    /// Shooter resolver.
    pub actor: Rc<dyn Fn(ActorId) -> RereleaseWeaponActor>,
    /// Frame pump for loading operations.
    pub next_frame: Box<dyn FnMut()>,
    /// Cvar capture seam for checkpoints.
    pub capture_cvars: Option<RereleaseCvarCaptureFn>,
    /// Cvar restore seam for checkpoints.
    pub restore_cvars: Option<RereleaseCvarRestoreFn>,
}

/// Entity binding kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WeaponBindingKind {
    /// Real source client.
    Client,
    /// Mirrored target.
    Target,
    /// Captured projectile.
    Projectile,
}

/// Bound entity record.
#[derive(Debug, Clone)]
pub struct WeaponBinding {
    actor: OwnedActor,
    kind: WeaponBindingKind,
    generation: i32,
    module_slot: u32,
}

impl WeaponBinding {
    /// Bound actor.
    #[must_use]
    pub fn actor(&self) -> &OwnedActor {
        &self.actor
    }

    /// Binding kind.
    #[must_use]
    pub fn kind(&self) -> WeaponBindingKind {
        self.kind
    }

    /// Spawn generation.
    #[must_use]
    pub fn generation(&self) -> i32 {
        self.generation
    }
}

/// Remove the primary identity before source cleanup can reenter or reuse
/// its native slot.
pub fn retire_rerelease_weapon_actor(
    slots: &mut HashMap<ActorId, u32>,
    bindings: &mut HashMap<u32, WeaponBinding>,
    retired: &mut HashMap<ActorId, WeaponTrajectoryUpdate>,
    actor: &ActorId,
    dispose: impl FnOnce(u32, &WeaponBinding) -> RereleaseWeaponResult<()>,
) -> RereleaseWeaponResult<()> {
    retired.remove(actor);
    let slot = slots.get(actor).copied();
    let bound = slot.and_then(|slot| bindings.get(&slot));
    match (slot, bound) {
        (Some(slot), Some(binding)) if binding.actor.id() == actor => {
            slots.remove(actor);
            let binding = bindings.remove(&slot).expect("binding present");
            dispose(slot, &binding)
        }
        _ => Ok(()),
    }
}

/// Native trajectory source (donor `WeaponBehaviorSource`).
pub trait WeaponBehaviorSource {
    /// Behavior definition.
    fn definition(&self) -> RereleaseWeaponResult<WeaponBehaviorDefinition>;
    /// Attach a projectile launch.
    fn attach(&mut self, launch: &WeaponBehaviorLaunch) -> RereleaseWeaponResult<Option<WeaponBehaviorInstance>>;
    /// Resume a retained or retired trajectory.
    fn resume(&mut self, projectile: &ActorId) -> RereleaseWeaponResult<WeaponBehaviorInstance>;
}

/// Live trajectory instance (donor `WeaponBehaviorInstance`).
///
/// The donor closes over the source; Rust drives instances through
/// [`RereleaseWeaponBehaviorSource::step_instance`] and
/// [`RereleaseWeaponBehaviorSource::close_instance`] with the source passed
/// explicitly.
#[derive(Debug, Clone)]
pub struct WeaponBehaviorInstance {
    /// Behavior definition.
    pub definition: WeaponBehaviorDefinition,
    /// Initial trajectory.
    pub initial: WeaponTrajectoryUpdate,
    /// Bound host slot (`None` once retired).
    pub slot: Option<u32>,
    /// Projectile actor.
    pub actor: ActorId,
    /// Whether the instance closed.
    pub closed: bool,
}

/// Saved configstring entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeaponConfigstringEntry {
    /// Configstring index.
    pub index: i32,
    /// Configstring value.
    pub value: String,
}

/// Saved retired trajectory.
#[derive(Debug, Clone, PartialEq)]
pub struct WeaponRetiredEntry {
    /// Saved projectile.
    pub actor: SavedActorId,
    /// Final trajectory.
    pub trajectory: WeaponTrajectoryUpdate,
}

/// Saved entity binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeaponBindingEntry {
    /// Host slot.
    pub slot: u32,
    /// Saved actor.
    pub actor: SavedActorId,
    /// Binding kind.
    pub kind: WeaponBindingKind,
    /// Spawn generation.
    pub generation: i32,
}

/// Saved weapon component checkpoint.
#[derive(Debug, Clone)]
pub struct RereleaseWeaponBehaviorCheckpoint {
    /// Checkpoint version (always 1).
    pub version: u32,
    /// Accepted declaration.
    pub declaration: NativeWeaponBehaviorDeclaration,
    /// Behavior definition.
    pub definition: WeaponBehaviorDefinition,
    /// Component map.
    pub map: ClassicGuestMap,
    /// Component time in seconds.
    pub time: f64,
    /// Captured cvar transfer state.
    pub cvars: Vec<u8>,
    /// Game save.
    pub game: SourceSave,
    /// Level save.
    pub level: SourceSave,
    /// Configstrings.
    pub configstrings: Vec<WeaponConfigstringEntry>,
    /// Retired trajectories.
    pub retired: Vec<WeaponRetiredEntry>,
    /// Entity bindings.
    pub bindings: Vec<WeaponBindingEntry>,
}

/// Engine adapter that sinks presentation events: the weapon component owns
/// no presentation authority (donor `{...options.services.engine, emit: () => undefined}`).
struct EngineWithoutEmit {
    inner: Box<dyn Q2FoundationHost>,
}

impl EngineWithoutEmit {
    fn new(inner: Box<dyn Q2FoundationHost>) -> Self {
        Self { inner }
    }
}

impl Q2FoundationHost for EngineWithoutEmit {
    fn actors(&mut self) -> &mut dyn qa_content::q2::support::tables::Q2ActorRegistry {
        self.inner.actors()
    }
    fn bodies(&mut self) -> &mut dyn qa_content::q2::support::tables::Q2BodyTable {
        self.inner.bodies()
    }
    fn callbacks(&mut self) -> &mut dyn qa_content::q2::support::tables::Q2CallbackTable {
        self.inner.callbacks()
    }
    fn combat(&mut self) -> &mut dyn qa_content::q2::support::tables::Q2CombatAuthority {
        self.inner.combat()
    }
    fn inventory(&mut self) -> &mut dyn qa_content::q2::support::tables::Q2InventoryTable {
        self.inner.inventory()
    }
    fn now(&self) -> f64 {
        self.inner.now()
    }
    fn frame_seconds(&self) -> f64 {
        self.inner.frame_seconds()
    }
    fn gravity(&self) -> f64 {
        self.inner.gravity()
    }
    fn random(&mut self) -> f64 {
        self.inner.random()
    }
    fn schedule(&mut self, actor: &OwnedActor, due_seconds: Option<f64>) {
        self.inner.schedule(actor, due_seconds);
    }
    fn touch_triggers(&mut self, actor: &OwnedActor) {
        self.inner.touch_triggers(actor);
    }
    fn trace(
        &mut self,
        request: &qa_content::q2::foundation::host::Q2TraceRequest,
    ) -> qa_content::q2::support::contracts::TraceResult {
        self.inner.trace(request)
    }
    fn point_contents(&mut self, point: Vec3) -> i32 {
        self.inner.point_contents(point)
    }
    fn in_pvs(&mut self, first: Vec3, second: Vec3) -> bool {
        self.inner.in_pvs(first, second)
    }
    fn in_phs(&mut self, first: Vec3, second: Vec3) -> bool {
        self.inner.in_phs(first, second)
    }
    fn areas_connected(&mut self, first: Vec3, second: Vec3) -> bool {
        self.inner.areas_connected(first, second)
    }
    fn nearby(&mut self, origin: Vec3, radius: f64) -> Vec<ActorId> {
        self.inner.nearby(origin, radius)
    }
    fn players(&mut self) -> Vec<ActorId> {
        self.inner.players()
    }
    fn world_actor(&mut self) -> ActorId {
        self.inner.world_actor()
    }
    fn is_player(&mut self, actor: &ActorId) -> bool {
        self.inner.is_player(actor)
    }
    fn is_monster(&mut self, actor: &ActorId) -> bool {
        self.inner.is_monster(actor)
    }
    fn inline_model_bounds(&mut self, model: i32) -> Bounds {
        self.inner.inline_model_bounds(model)
    }
    fn set_solid(&mut self, actor: &OwnedActor, solid: qa_content::q2::foundation::host::Q2Solid, model: Option<i32>) {
        self.inner.set_solid(actor, solid, model);
    }
    fn set_motion(&mut self, motion: &qa_content::q2::foundation::host::Q2Motion) {
        self.inner.set_motion(motion);
    }
    fn set_area_portal(&mut self, portal: i32, open: bool) {
        self.inner.set_area_portal(portal, open);
    }
    fn emit(&mut self, _event: Q2PresentationEvent) {}
    fn player_view_state(&mut self, player: &ActorId) -> Option<qa_content::q2::foundation::host::Q2PlayerViewState> {
        self.inner.player_view_state(player)
    }
    fn key_consumed(&mut self, player: &ActorId) {
        self.inner.key_consumed(player);
    }
    fn prepare_level_change(
        &mut self,
        map: &str,
        landmark: Option<&qa_content::q2::foundation::host::Q2LandmarkCarry>,
        server_flags: i32,
    ) {
        self.inner.prepare_level_change(map, landmark, server_flags);
    }
    fn transition(&mut self, intent: qa_content::q2::support::contracts::TransitionIntent) {
        self.inner.transition(intent);
    }
    fn diagnostic(&mut self, message: &str) {
        self.inner.diagnostic(message);
    }
}

/// [`WeaponCvarRegistry`] over the shared registry.
struct RegistryProvisioning<'a> {
    cvars: &'a mut CvarRegistry,
}

impl WeaponCvarRegistry for RegistryProvisioning<'_> {
    fn capture(&self) -> Vec<(String, String)> {
        self.cvars
            .snapshots(0)
            .into_iter()
            .map(|snapshot| (snapshot.name, snapshot.value))
            .collect()
    }

    fn get(&self, name: &str) -> Option<String> {
        self.cvars.get(name).map(|snapshot| snapshot.value)
    }

    fn set(&mut self, name: &str, value: &str) {
        if let Err(error) = self.cvars.set(name, value, true) {
            panic!("native weapon provisioning cvar failed: {error}");
        }
    }

    fn restore(&mut self, state: &[(String, String)]) {
        for (name, value) in state {
            if let Err(error) = self.cvars.set(name, value, true) {
                panic!("native weapon provisioning restore failed: {error}");
            }
        }
    }
}

/// Validate a typed declaration against its executable module (donor
/// `readNativeWeaponDeclaration` module-identity leg).
fn check_declaration(
    declaration: &NativeWeaponBehaviorDeclaration,
    requested_path: &str,
    digest: &str,
) -> RereleaseWeaponResult<()> {
    let path = normalize_resource_path(&declaration.artifact_path)
        .map_err(|error| RereleaseWeaponError::invalid(format!("native profile artifact path: {error}")))?;
    if path != requested_path || declaration.artifact_digest != digest {
        return Err(RereleaseWeaponError::invalid(
            "native profile artifact identity differs",
        ));
    }
    Ok(())
}

/// Project a typed declaration onto the compact profile declaration.
fn profile_declaration(declaration: &NativeWeaponBehaviorDeclaration) -> WeaponProfileDeclaration {
    WeaponProfileDeclaration {
        id: declaration.id.clone(),
        title: declaration.title.clone(),
        role: declaration.role.clone(),
        artifact_path: declaration.artifact_path.clone(),
        artifact_digest: declaration.artifact_digest.clone(),
        entity: WeaponEntityFields {
            byte_length: declaration.entity.byte_length,
            origin: declaration.entity.origin,
            angles: declaration.entity.angles,
            velocity: declaration.entity.velocity,
            client: declaration.entity.client,
            owner: declaration.entity.owner,
            view_height: declaration.entity.view_height,
            generation: declaration.entity.generation,
            next_think: declaration.entity.next_think,
            think_callback: declaration.entity.think_callback,
            think_registration: declaration.entity.think_registration,
            touch_callback: declaration.entity.touch_callback,
        },
        client: WeaponClientFields {
            byte_length: declaration.client.byte_length,
            weapon: declaration.client.weapon,
            view_angles: declaration.client.view_angles,
            forward: declaration.client.forward,
        },
        equipped_byte_length: declaration.equipped_weapon.byte_length,
        equipped_callback: declaration.equipped_weapon.callback,
        equipped_expected: declaration.equipped_weapon.expected.rva,
        time_rva: declaration.time_rva,
        think_tag: declaration.think_tag,
        allocate_rva: declaration.allocate.rva,
        free_rva: declaration.free.rva,
        projectile_touch_rva: declaration.projectile_touch.rva,
        equip: declaration.equip.calls.iter().map(|call| call.rva).collect(),
        launch: declaration.launch.calls.iter().map(|call| call.rva).collect(),
        activate_rva: declaration.activate_rva,
        fire_rva: declaration.fire_rva,
        initialization_classes: declaration.initialization_classes.clone(),
        equipment: declaration
            .equipment
            .iter()
            .map(|command| WeaponCommand {
                arguments: command.arguments.clone(),
                tail: command.tail.clone(),
            })
            .collect(),
        ammunition: WeaponCommand {
            arguments: declaration.ammunition.arguments.clone(),
            tail: declaration.ammunition.tail.clone(),
        },
        initial_cvars: declaration
            .initial_cvars
            .iter()
            .map(|cvar| WeaponCvar {
                name: cvar.name.clone(),
                value: cvar.value.clone(),
            })
            .collect(),
        provisioning_cvars: declaration
            .provisioning_cvars
            .iter()
            .map(|cvar| WeaponCvar {
                name: cvar.name.clone(),
                value: cvar.value.clone(),
            })
            .collect(),
    }
}

fn projectile_role(role: &str) -> RereleaseWeaponResult<ProjectileRole> {
    match role {
        "rocket" => Ok(ProjectileRole::Rocket),
        "grenade" => Ok(ProjectileRole::Grenade),
        "nail" => Ok(ProjectileRole::Nail),
        "bolt" => Ok(ProjectileRole::Bolt),
        "plasma" => Ok(ProjectileRole::Plasma),
        "energy" => Ok(ProjectileRole::Energy),
        "grapple" => Ok(ProjectileRole::Grapple),
        _ => Err(RereleaseWeaponError::invalid(format!("unknown projectile role {role}"))),
    }
}

/// In-flight launch capture (donor `capture` field).
struct WeaponCapture {
    /// Shooter module slot.
    shooter: u32,
    /// Projectile actor.
    projectile: OwnedActor,
    /// Captured projectile module slots.
    slots: HashSet<u32>,
}

/// Native trajectory component (donor `RereleaseWeaponBehaviorSource`).
///
/// The headless port keeps the donor's state machine (bindings, slots,
/// instances, retirements, reentrancy guard) and slot/generation math, but
/// no guest code runs: `ClientCommand` equipment, `launch`, and `think`
/// record synthetic invocations instead of entering the DLL, `project`/
/// `trajectory`/`generation` read the synthetic module's records, and the
/// capture leg synthesizes the single projectile the game spawn would have
/// linked. Engine release observation is a
/// [`RereleaseWeaponBehaviorSource::notify_actor_released`] driver hook
/// because the engine registry exposes no release subscription.
pub struct RereleaseWeaponBehaviorSource {
    declaration: NativeWeaponBehaviorDeclaration,
    definition: WeaponBehaviorDefinition,
    map: ClassicGuestMap,
    actor: Rc<dyn Fn(ActorId) -> RereleaseWeaponActor>,
    capture_cvars: Option<RereleaseCvarCaptureFn>,
    restore_cvars: Option<RereleaseCvarRestoreFn>,
    command: Rc<RefCell<GuestCommandLine>>,
    max_clients: u32,
    source: Option<RereleaseGuestSource>,
    services: Option<RereleaseGuestServices>,
    profile: Option<RereleaseWeaponProfile>,
    module: Option<SyntheticWeaponModule>,
    bindings: HashMap<u32, WeaponBinding>,
    slots: HashMap<ActorId, u32>,
    instances: HashSet<ActorId>,
    retired: HashMap<ActorId, WeaponTrajectoryUpdate>,
    capture: Option<WeaponCapture>,
    pending_releases: HashSet<ActorId>,
    busy: bool,
    closed: bool,
    time: f64,
    initialization_entities: String,
}

/// Host import interception for the weapon component (donor
/// `RereleaseWeaponBehaviorSource.intercept`, name legs).
///
/// Portal mutation and clipboard access panic like the donor throws;
/// network, sound, and print imports sink to void because selected weapon
/// presentation stays primary; everything else (including
/// linkentity/unlinkentity, whose record math lives in
/// [`RereleaseWeaponBehaviorSource::intercept_link`]) falls through to the
/// host.
fn intercept_import_name(_api: &str, name: &str, _args: &[GuestCallValue]) -> Option<GuestCallResult> {
    match name {
        "SetAreaPortalState" => {
            panic!("Native trajectory component cannot mutate selected world portals")
        }
        "SendToClipBoard" => panic!("Native trajectory component has no clipboard capability"),
        "WriteChar" | "WriteByte" | "WriteShort" | "WriteLong" | "WriteFloat" | "WriteAngle" | "WritePosition"
        | "WriteDir" | "WriteString" | "WriteEntity" | "unicast" | "multicast" | "sound" | "positioned_sound"
        | "local_sound" | "Broadcast_Print" | "Client_Print" | "Center_Print" | "Loc_Print" => {
            Some(GuestCallResult::Void)
        }
        _ => None,
    }
}

impl RereleaseWeaponBehaviorSource {
    /// Create a weapon component over a prepared guest.
    ///
    /// Async donor loading (`initLoading`, `spawnEntitiesLoading`) runs
    /// synchronously through the `next_frame` pump; the source entity spawn
    /// has no compat counterpart, so creation publishes the (empty) spawn
    /// and drains messages instead. Partial state drops on error (RAII)
    /// rather than raising the donor's aggregate cleanup error.
    pub fn create(options: RereleaseWeaponBehaviorOptions) -> RereleaseWeaponResult<Self> {
        let RereleaseWeaponBehaviorOptions {
            prepared,
            declaration,
            services: mut services_options,
            clock,
            map,
            ownership: _,
            actor,
            mut next_frame,
            capture_cvars,
            restore_cvars,
        } = options;
        let (requested_path, digest) = match &prepared.execution.implementation {
            ExecutionImplementation::Native { artifact, .. } => {
                (artifact.requested_path.clone(), artifact.digest.clone())
            }
            _ => {
                return Err(RereleaseWeaponError::invalid(
                    "native trajectory behavior requires a native executable profile",
                ));
            }
        };
        check_declaration(&declaration, &requested_path, &digest)?;
        let module_identity = native_module_identity_parts(&prepared.execution, prepared.primary.as_ref());
        let role = projectile_role(&declaration.role)?;
        let content_module = ContentModuleIdentity {
            id: module_identity.id.clone(),
            artifact_path: module_identity.artifact_path.clone(),
            digest: ContentDigest(digest.clone()),
            revision: module_identity.revision.clone(),
        };
        let callback = |rva: u64| WeaponBehaviorCallback::NativeArtifact {
            module: content_module.clone(),
            image_offset: rva,
            abi: ContentAbi::Native(NativeAbi::WindowsX86_64),
        };
        let fire = callback(declaration.fire_rva);
        let activate = declaration.activate_rva.map(callback);
        let definition = WeaponBehaviorDefinition {
            id: declaration.id.clone(),
            title: declaration.title.clone(),
            module: content_module,
            role,
            activate,
            fire,
        };
        let command = Rc::new(RefCell::new(GuestCommandLine {
            arguments: Vec::new(),
            args: String::new(),
        }));
        let command_reader = Rc::clone(&command);
        let engine = services_options.base.engine;
        services_options.base.engine = Box::new(EngineWithoutEmit::new(engine));
        services_options.base.command = Rc::new(move || command_reader.borrow().clone());
        services_options.base.add_command = Box::new(|_| {
            panic!("Native weapon component cannot execute server commands");
        });
        services_options.debug_shapes = Box::new(|_| {});
        services_options.world_text = Box::new(|_| {});
        services_options.semantic_bindings = Some(RereleaseSemanticBindings {
            project: None,
            generation: None,
            bound: None,
            bind: Rc::new(|_, _| {
                panic!("Native component cannot replace primary actor authorities");
            }),
            foreign_address: Rc::new(|_| {
                panic!("Native component foreign address needs a live guest module");
            }),
        });
        let max_clients = services_options.base.max_clients;
        let mut services = RereleaseGuestServices::new(services_options).map_err(host_mapped)?;
        let intercept: InterceptImport = Box::new(intercept_import_name);
        let mut source = RereleaseGuestSource::create(
            prepared,
            RereleaseGuestSourceOptions {
                clock,
                instruction_budget: None,
                foreign_damage: None,
                pickups: None,
                intercept_import: Some(intercept),
            },
        )
        .map_err(host_mapped)?;
        services.bind_host(&mut source.host).map_err(host_mapped)?;
        let profile = RereleaseWeaponProfile::new(&digest, &requested_path, Some(profile_declaration(&declaration)))
            .map_err(weapon_mapped)?
            .ok_or_else(|| RereleaseWeaponError::invalid("native trajectory declaration has no definition"))?;
        let memory = SparseGuestMemory::new(module_identity, 8, 0x1_0000).map_err(host_mapped)?;
        let module = SyntheticWeaponModule::new(memory, declaration.time_rva, declaration.entity.byte_length)
            .map_err(weapon_mapped)?;
        let initialization_entities =
            weapon_initialization_entities(&map.entities, &declaration.initialization_classes)
                .map_err(weapon_mapped)?;
        for preset in &profile.declaration.initial_cvars {
            services
                .cvars_mut()
                .set(&preset.name, &preset.value, true)
                .map_err(host_mapped)?;
        }
        source.init_loading(next_frame.as_mut()).map_err(host_mapped)?;
        services.complete_spawn();
        services.drain_messages();
        Ok(Self {
            declaration,
            definition,
            map,
            actor,
            capture_cvars,
            restore_cvars,
            command,
            max_clients,
            source: Some(source),
            services: Some(services),
            profile: Some(profile),
            module: Some(module),
            bindings: HashMap::new(),
            slots: HashMap::new(),
            instances: HashSet::new(),
            retired: HashMap::new(),
            capture: None,
            pending_releases: HashSet::new(),
            busy: false,
            closed: false,
            time: 0.0,
            initialization_entities,
        })
    }

    /// Accepted declaration.
    #[must_use]
    pub fn declaration(&self) -> &NativeWeaponBehaviorDeclaration {
        &self.declaration
    }

    /// Behavior definition.
    #[must_use]
    pub fn definition(&self) -> &WeaponBehaviorDefinition {
        &self.definition
    }

    /// Component map.
    #[must_use]
    pub fn map(&self) -> &ClassicGuestMap {
        &self.map
    }

    /// Component time in seconds.
    #[must_use]
    pub fn time(&self) -> f64 {
        self.time
    }

    /// Bound entity count.
    #[must_use]
    pub fn binding_count(&self) -> usize {
        self.bindings.len()
    }

    /// Notify the component that the engine released an actor (donor
    /// `engine.actors.onRelease` subscription leg).
    pub fn notify_actor_released(&mut self, actor: &ActorId) -> RereleaseWeaponResult<()> {
        if self.busy {
            self.pending_releases.insert(actor.clone());
            Ok(())
        } else {
            self.release_actor(actor)
        }
    }

    fn retained(&mut self) -> RereleaseWeaponResult<Retained<'_>> {
        if self.closed
            || self.source.is_none()
            || self.services.is_none()
            || self.profile.is_none()
            || self.module.is_none()
        {
            return Err(RereleaseWeaponError::invalid(
                "Native weapon component is not initialized",
            ));
        }
        Ok(Retained {
            source: self.source.as_mut().expect("checked"),
            services: self.services.as_mut().expect("checked"),
            profile: self.profile.as_mut().expect("checked"),
            module: self.module.as_mut().expect("checked"),
        })
    }

    fn operation<T>(&mut self, run: impl FnOnce(&mut Self) -> RereleaseWeaponResult<T>) -> RereleaseWeaponResult<T> {
        if self.busy {
            return Err(RereleaseWeaponError::invalid(
                "Native weapon component operation is already active",
            ));
        }
        self.busy = true;
        let result = run(self);
        self.busy = false;
        let flush = self.flush_releases();
        if let Some(services) = self.services.as_mut() {
            services.drain_messages();
        }
        flush?;
        result
    }

    fn flush_releases(&mut self) -> RereleaseWeaponResult<()> {
        let pending: Vec<ActorId> = self.pending_releases.drain().collect();
        let mut failures = Vec::new();
        for actor in pending {
            if let Err(error) = self.release_actor(&actor) {
                failures.push(error.to_string());
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(RereleaseWeaponError::invalid(format!(
                "Native component actor retirement failed: {}",
                failures.join("; ")
            )))
        }
    }
}

/// Borrowed retained component parts.
struct Retained<'a> {
    source: &'a mut RereleaseGuestSource,
    services: &'a mut RereleaseGuestServices,
    profile: &'a mut RereleaseWeaponProfile,
    module: &'a mut SyntheticWeaponModule,
}

/// Convert an engine shooter snapshot to the profile shooter shape.
fn to_shooter(target: &RereleaseWeaponActor) -> WeaponShooter {
    WeaponShooter {
        body: WeaponBodyState {
            origin: target.body.origin,
            angles: target.body.angles,
            velocity: target.body.velocity,
        },
        view_angles: target.view_angles,
        view_height: target.view_height,
    }
}

impl RereleaseWeaponBehaviorSource {
    fn set_time(&mut self, seconds: f64) -> RereleaseWeaponResult<()> {
        if !seconds.is_finite() || seconds < 0.0 {
            return Err(RereleaseWeaponError::invalid(
                "Native weapon invocation time must be finite and nonnegative",
            ));
        }
        self.time = seconds;
        let retained = self.retained()?;
        retained
            .profile
            .set_time(retained.module, (seconds * 1000.0).round() as i64);
        Ok(())
    }

    fn live(&mut self, _host: u32, binding: &WeaponBinding) -> RereleaseWeaponResult<bool> {
        let retained = self.retained()?;
        let record = match retained.module.entity(binding.module_slot) {
            Ok(record) => record,
            Err(_) => return Ok(false),
        };
        let layout = edict_layout();
        let inuse = field_offset(&layout, "inuse").map_err(host_mapped)? as i64;
        let flag = retained
            .module
            .memory
            .read_u8(retained.module.memory.offset(record, inuse).map_err(host_mapped)?)
            .map_err(host_mapped)?;
        let generation = retained
            .profile
            .generation(retained.module, binding.module_slot)
            .map_err(weapon_mapped)?;
        Ok(flag != 0 && generation == binding.generation)
    }

    /// Mark a synthetic record live. Allocation marks records live
    /// because no game spawn runs to set `inuse`.
    fn set_inuse(&mut self, module_slot: u32, live: bool) -> RereleaseWeaponResult<()> {
        let retained = self.retained()?;
        let record = retained.module.entity(module_slot).map_err(weapon_mapped)?;
        let layout = edict_layout();
        let offset = field_offset(&layout, "inuse").map_err(host_mapped)? as i64;
        retained
            .module
            .memory
            .write_u8(
                retained.module.memory.offset(record, offset).map_err(host_mapped)?,
                u8::from(live),
            )
            .map_err(host_mapped)?;
        Ok(())
    }

    fn remember(
        &mut self,
        host: u32,
        module_slot: u32,
        actor: OwnedActor,
        kind: WeaponBindingKind,
    ) -> RereleaseWeaponResult<()> {
        let generation = {
            let retained = self.retained()?;
            retained
                .profile
                .generation(retained.module, module_slot)
                .map_err(weapon_mapped)?
        };
        self.bindings.insert(
            host,
            WeaponBinding {
                actor: actor.clone(),
                kind,
                generation,
                module_slot,
            },
        );
        self.slots.insert(actor.id().clone(), host);
        Ok(())
    }

    /// Next host slot above the client range for mirrored targets and
    /// projectiles (donor game entity slots live above the client slots).
    fn next_target_slot(&self) -> u32 {
        let mut slot = self.max_clients + 1;
        while self.bindings.contains_key(&slot) {
            slot += 1;
        }
        slot
    }

    fn host_for_module(&self, module_slot: u32) -> Option<u32> {
        self.bindings
            .iter()
            .find_map(|(host, binding)| (binding.module_slot == module_slot).then_some(*host))
    }

    /// Mirror a target actor into a private record (donor `mirror`,
    /// reached via `semanticBindings.foreignAddress`).
    ///
    /// The bound-hook path never fires headless, so drivers call this
    /// directly to admit targets.
    pub fn mirror_actor(&mut self, actor: &ActorId) -> RereleaseWeaponResult<u32> {
        if let Some(previous) = self.slots.get(actor).copied() {
            return Ok(previous);
        }
        let resolver = Rc::clone(&self.actor);
        let target = resolver(actor.clone());
        let module_slot = {
            let retained = self.retained()?;
            let slot = retained.profile.allocate(retained.module).map_err(weapon_mapped)?;
            retained
                .profile
                .project(
                    retained.module,
                    slot,
                    &WeaponBodyState {
                        origin: target.body.origin,
                        angles: target.body.angles,
                        velocity: target.body.velocity,
                    },
                )
                .map_err(weapon_mapped)?;
            slot
        };
        self.set_inuse(module_slot, true)?;
        let host = self.next_target_slot();
        self.remember(host, module_slot, target.actor.clone(), WeaponBindingKind::Target)?;
        Ok(host)
    }

    /// Bind a shooter to a real source client slot.
    ///
    /// The compat host exposes client reservation but no connect/begin
    /// handshake, so reservation doubles as admission: a failed
    /// reservation runs the donor's rejection cleanup.
    fn client(&mut self, actor: &ActorId) -> RereleaseWeaponResult<u32> {
        let resolver = Rc::clone(&self.actor);
        let target = resolver(actor.clone());
        if let Some(previous) = self.slots.get(actor).copied() {
            let existing = self.bindings.get(&previous).cloned();
            match existing {
                Some(binding) if binding.kind == WeaponBindingKind::Client => {
                    let retained = self.retained()?;
                    retained
                        .profile
                        .project_shooter(retained.module, binding.module_slot, &to_shooter(&target))
                        .map_err(weapon_mapped)?;
                    return Ok(previous);
                }
                Some(binding) => {
                    let retained = self.retained()?;
                    retained
                        .profile
                        .free(retained.module, binding.module_slot)
                        .map_err(weapon_mapped)?;
                    self.bindings.remove(&previous);
                    self.slots.remove(actor);
                }
                None => {
                    self.bindings.remove(&previous);
                    self.slots.remove(actor);
                }
            }
        }
        let mut slot = 1u32;
        while slot <= self.max_clients && self.bindings.contains_key(&slot) {
            slot += 1;
        }
        if slot > self.max_clients {
            return Err(RereleaseWeaponError::invalid(
                "Native weapon component exhausted its real source client capacity",
            ));
        }
        let module_slot = {
            let retained = self.retained()?;
            let slot = retained.profile.allocate(retained.module).map_err(weapon_mapped)?;
            let client_length = retained.profile.declaration.client.byte_length;
            let client_offset = retained.profile.declaration.entity.client;
            retained
                .module
                .attach_client(slot, client_length, client_offset)
                .map_err(weapon_mapped)?;
            slot
        };
        self.set_inuse(module_slot, true)?;
        self.remember(slot, module_slot, target.actor.clone(), WeaponBindingKind::Client)?;
        let admitted = self.source.as_mut().expect("checked").host.reserve_client(slot);
        if admitted.is_err() {
            self.bindings.remove(&slot);
            self.slots.remove(actor);
            let _ = self
                .source
                .as_mut()
                .expect("checked")
                .host
                .release_client_reservation(slot);
            return Err(RereleaseWeaponError::invalid(
                "Native weapon source rejected the shooter userinfo",
            ));
        }
        self.remember(slot, module_slot, target.actor.clone(), WeaponBindingKind::Client)?;
        {
            let retained = self.retained()?;
            retained
                .profile
                .project_shooter(retained.module, module_slot, &to_shooter(&target))
                .map_err(weapon_mapped)?;
        }
        let equipment = self.profile.as_ref().expect("checked").declaration.equipment.clone();
        for command in &equipment {
            self.client_command(module_slot, &command.arguments, &command.tail)?;
        }
        let retained = self.retained()?;
        retained
            .profile
            .equip(retained.module, module_slot)
            .map_err(weapon_mapped)?;
        Ok(slot)
    }

    /// Run an equipment command under provisioning discipline.
    ///
    /// The provisioning wrapper genuinely enforces the declared cvar
    /// capability; the guest `ClientCommand` dispatch and cvar refresh have
    /// no compat counterpart, so they are no-ops headless.
    fn client_command(&mut self, _module: u32, arguments: &[String], tail: &str) -> RereleaseWeaponResult<()> {
        *self.command.borrow_mut() = GuestCommandLine {
            arguments: arguments.to_vec(),
            args: tail.to_string(),
        };
        let result = (|| {
            let retained = self.retained()?;
            let provisioning = retained.profile.declaration.provisioning_cvars.clone();
            let mut cvars = RegistryProvisioning {
                cvars: retained.services.cvars_mut(),
            };
            let mut refresh = || {};
            with_weapon_provisioning(&provisioning, &mut cvars, &mut refresh, || {}).map_err(weapon_mapped)
        })();
        *self.command.borrow_mut() = GuestCommandLine {
            arguments: Vec::new(),
            args: String::new(),
        };
        result
    }

    /// Attach a projectile launch, returning its trajectory instance, or
    /// `None` when the launch emits no projectile.
    pub fn attach(&mut self, launch: &WeaponBehaviorLaunch) -> RereleaseWeaponResult<Option<WeaponBehaviorInstance>> {
        self.operation(|owner| {
            if launch.role != owner.definition.role || owner.slots.contains_key(launch.projectile.id()) {
                return Err(RereleaseWeaponError::invalid(
                    "Incompatible or duplicate native projectile attachment",
                ));
            }
            owner.set_time(launch.time_seconds)?;
            let shooter_host = owner.client(&launch.shooter)?;
            let shooter_module = owner.bindings.get(&shooter_host).expect("shooter bound").module_slot;
            let ammunition = owner.profile.as_ref().expect("checked").declaration.ammunition.clone();
            owner.client_command(shooter_module, &ammunition.arguments, &ammunition.tail)?;
            owner.capture = Some(WeaponCapture {
                shooter: shooter_module,
                projectile: launch.projectile.clone(),
                slots: HashSet::new(),
            });
            let launched = (|| {
                let retained = owner.retained()?;
                retained
                    .profile
                    .launch(retained.module, shooter_module)
                    .map_err(weapon_mapped)
            })();
            if let Err(error) = launched {
                owner.capture = None;
                return Err(error);
            }
            // No guest code runs to spawn and link the projectile, so
            // synthesize the single record the game spawn would have
            // linked and run it through the capture gate.
            let synthesized = owner.synthesize_projectile(shooter_module, launch);
            let capture = owner.capture.take();
            let mut slots: Vec<u32> = capture
                .map(|capture| capture.slots.into_iter().collect())
                .unwrap_or_default();
            slots.sort_unstable();
            if let Err(error) = synthesized {
                for slot in &slots {
                    if let Some(host) = owner.host_for_module(*slot) {
                        owner.bindings.remove(&host);
                    }
                }
                owner.slots.remove(launch.projectile.id());
                return Err(error);
            }
            if slots.is_empty() {
                return Ok(None);
            }
            if slots.len() != 1 {
                for slot in &slots {
                    let freed = (|| {
                        let retained = owner.retained()?;
                        retained.profile.free(retained.module, *slot).map_err(weapon_mapped)
                    })();
                    freed?;
                    if let Some(host) = owner.host_for_module(*slot) {
                        owner.bindings.remove(&host);
                    }
                }
                owner.slots.remove(launch.projectile.id());
                return Err(RereleaseWeaponError::invalid(
                    "Native launch emitted multiple projectiles; one-to-many trajectory composition is required",
                ));
            }
            let host = owner
                .slots
                .get(launch.projectile.id())
                .copied()
                .ok_or_else(|| RereleaseWeaponError::invalid("Native captured projectile disappeared"))?;
            owner.instance(host).map(Some)
        })
    }

    /// Synthesize the projectile record a game spawn would have linked,
    /// running the capture gate and link math over it.
    fn synthesize_projectile(
        &mut self,
        shooter_module: u32,
        launch: &WeaponBehaviorLaunch,
    ) -> RereleaseWeaponResult<u32> {
        let module_slot = {
            let retained = self.retained()?;
            let slot = retained.profile.allocate(retained.module).map_err(weapon_mapped)?;
            retained
                .profile
                .project(
                    retained.module,
                    slot,
                    &WeaponBodyState {
                        origin: launch.body.origin,
                        angles: launch.body.angles,
                        velocity: launch.body.velocity,
                    },
                )
                .map_err(weapon_mapped)?;
            let record = retained.module.entity(slot).map_err(weapon_mapped)?;
            let shooter = retained.module.entity(shooter_module).map_err(weapon_mapped)?;
            let owner_offset = retained.profile.declaration.entity.owner as i64;
            retained
                .module
                .memory
                .write_pointer(
                    retained
                        .module
                        .memory
                        .offset(record, owner_offset)
                        .map_err(host_mapped)?,
                    Some(shooter),
                )
                .map_err(host_mapped)?;
            let touch_offset = retained.profile.declaration.entity.touch_callback as i64;
            let touch_rva = retained.profile.declaration.projectile_touch_rva as i64;
            let touch = retained
                .module
                .memory
                .offset(retained.module.image_base, touch_rva)
                .map_err(host_mapped)?;
            retained
                .module
                .memory
                .write_pointer(
                    retained
                        .module
                        .memory
                        .offset(record, touch_offset)
                        .map_err(host_mapped)?,
                    Some(touch),
                )
                .map_err(host_mapped)?;
            slot
        };
        self.set_inuse(module_slot, true)?;
        // The link leg runs the capture gate and binds matching
        // projectiles; a rejected record is freed and the launch fizzles
        // to an empty capture (donor `attach` returns null).
        self.intercept_link(module_slot, true)?;
        let captured = self
            .capture
            .as_ref()
            .is_some_and(|capture| capture.slots.contains(&module_slot));
        if !captured {
            let retained = self.retained()?;
            retained
                .profile
                .free(retained.module, module_slot)
                .map_err(weapon_mapped)?;
        }
        Ok(module_slot)
    }

    /// Donor capture gate: the projectile's owner is the shooter record
    /// and its touch callback is the declared projectile-touch entry.
    fn capture_matches(&mut self, module_slot: u32, shooter_module: u32) -> RereleaseWeaponResult<bool> {
        let retained = self.retained()?;
        let record = retained.module.entity(module_slot).map_err(weapon_mapped)?;
        let shooter = retained.module.entity(shooter_module).map_err(weapon_mapped)?;
        let owner_offset = retained.profile.declaration.entity.owner as i64;
        let owner = retained
            .module
            .memory
            .read_pointer(
                retained
                    .module
                    .memory
                    .offset(record, owner_offset)
                    .map_err(host_mapped)?,
            )
            .map_err(host_mapped)?;
        let touch_offset = retained.profile.declaration.entity.touch_callback as i64;
        let touch = retained
            .module
            .memory
            .read_pointer(
                retained
                    .module
                    .memory
                    .offset(record, touch_offset)
                    .map_err(host_mapped)?,
            )
            .map_err(host_mapped)?;
        let touch_rva = retained.profile.declaration.projectile_touch_rva as i64;
        let expected = retained
            .module
            .memory
            .offset(retained.module.image_base, touch_rva)
            .map_err(host_mapped)?;
        Ok(owner == Some(shooter) && touch == Some(expected))
    }

    /// Port of the donor intercept's linkentity/unlinkentity record math
    /// for drivers that link synthetic records directly. Slot 0 (world)
    /// is a no-op; when a launch capture is active and the record passes
    /// the capture gate it joins the capture set.
    pub fn intercept_link(&mut self, module_slot: u32, linking: bool) -> RereleaseWeaponResult<()> {
        if module_slot == 0 {
            return Ok(());
        }
        if linking {
            let gated = self
                .capture
                .as_ref()
                .map(|capture| (capture.shooter, capture.projectile.clone()));
            if let Some((shooter, projectile)) = gated {
                if self.capture_matches(module_slot, shooter)? {
                    if let Some(capture) = self.capture.as_mut() {
                        capture.slots.insert(module_slot);
                    }
                    let host = self.next_target_slot();
                    self.remember(host, module_slot, projectile, WeaponBindingKind::Projectile)?;
                }
            }
        }
        let retained = self.retained()?;
        let record = retained.module.entity(module_slot).map_err(weapon_mapped)?;
        let layout = edict_layout();
        let at = |module: &mut SyntheticWeaponModule, name: &str| {
            field_offset(&layout, name)
                .map_err(host_mapped)
                .and_then(|offset| module.memory.offset(record, offset as i64).map_err(host_mapped))
        };
        let solid_at = at(retained.module, "solid")?;
        let solid = retained.module.memory.read_u8(solid_at).map_err(host_mapped)?;
        let linked = at(retained.module, "linked")?;
        retained
            .module
            .memory
            .write_u8(linked, u8::from(linking && solid != 0))
            .map_err(host_mapped)?;
        if linking {
            for axis in 0..3i64 {
                let displacement = axis * 4;
                let read = |module: &mut SyntheticWeaponModule, name: &str| {
                    let base = at(module, name)?;
                    let cell = module.memory.offset(base, displacement).map_err(host_mapped)?;
                    module.memory.read_f32(cell).map_err(host_mapped)
                };
                let origin = read(retained.module, "s.origin")?;
                let min = read(retained.module, "mins")?;
                let max = read(retained.module, "maxs")?;
                let write = |module: &mut SyntheticWeaponModule, name: &str, value: f32| {
                    let base = at(module, name)?;
                    let cell = module.memory.offset(base, displacement).map_err(host_mapped)?;
                    module.memory.write_f32(cell, value).map_err(host_mapped)
                };
                write(retained.module, "absmin", origin + min - 1.0)?;
                write(retained.module, "absmax", origin + max + 1.0)?;
                write(retained.module, "size", max - min)?;
            }
            let count_at = at(retained.module, "linkcount")?;
            let count = retained.module.memory.read_i32(count_at).map_err(host_mapped)?;
            retained
                .module
                .memory
                .write_i32(count_at, count + 1)
                .map_err(host_mapped)?;
        }
        Ok(())
    }
}

impl RereleaseWeaponBehaviorSource {
    /// Resume a retained or retired trajectory.
    pub fn resume(&mut self, projectile: &ActorId) -> RereleaseWeaponResult<WeaponBehaviorInstance> {
        self.operation(|owner| {
            if let Some(trajectory) = owner.retired.get(projectile).cloned() {
                return owner.retired_instance(projectile.clone(), trajectory);
            }
            let slot = owner.slots.get(projectile).copied();
            let kind = slot.and_then(|host| owner.bindings.get(&host).map(WeaponBinding::kind));
            match (slot, kind) {
                (Some(host), Some(WeaponBindingKind::Projectile)) => owner.instance(host),
                _ => Err(RereleaseWeaponError::invalid(
                    "Saved native trajectory has no retained source projectile",
                )),
            }
        })
    }

    fn instance(&mut self, host: u32) -> RereleaseWeaponResult<WeaponBehaviorInstance> {
        let binding = self
            .bindings
            .get(&host)
            .cloned()
            .ok_or_else(|| RereleaseWeaponError::invalid("Native projectile binding is missing"))?;
        let actor = binding.actor.id().clone();
        if self.instances.contains(&actor) {
            return Err(RereleaseWeaponError::invalid(
                "Native projectile already has a trajectory owner",
            ));
        }
        self.instances.insert(actor.clone());
        let initial = {
            let retained = self.retained()?;
            let update = retained
                .profile
                .trajectory(retained.module, binding.module_slot)
                .map_err(weapon_mapped)?;
            WeaponTrajectoryUpdate {
                origin: update.origin,
                velocity: update.velocity,
                angles: update.angles,
            }
        };
        Ok(WeaponBehaviorInstance {
            definition: self.definition.clone(),
            initial,
            slot: Some(host),
            actor,
            closed: false,
        })
    }

    fn retired_instance(
        &mut self,
        actor: ActorId,
        initial: WeaponTrajectoryUpdate,
    ) -> RereleaseWeaponResult<WeaponBehaviorInstance> {
        if self.instances.contains(&actor) {
            return Err(RereleaseWeaponError::invalid(
                "Native projectile already has a trajectory owner",
            ));
        }
        self.instances.insert(actor.clone());
        Ok(WeaponBehaviorInstance {
            definition: self.definition.clone(),
            initial,
            slot: None,
            actor,
            closed: false,
        })
    }

    /// Step a trajectory instance (donor `WeaponBehaviorInstance.step`).
    ///
    /// Engine liveness is observed through
    /// [`RereleaseWeaponBehaviorSource::notify_actor_released`] only; an
    /// instance whose actor lost its trajectory owner reads as closed.
    pub fn step_instance(
        &mut self,
        instance: &mut WeaponBehaviorInstance,
        body: &EngineBodyState,
        time_seconds: f64,
    ) -> RereleaseWeaponResult<Option<WeaponTrajectoryUpdate>> {
        if instance.closed || !self.instances.contains(&instance.actor) {
            return Err(RereleaseWeaponError::invalid("Native trajectory instance is closed"));
        }
        self.operation(|owner| {
            owner.set_time(time_seconds)?;
            if owner.retired.contains_key(&instance.actor) {
                return Ok(None);
            }
            let host = instance
                .slot
                .ok_or_else(|| RereleaseWeaponError::invalid("Native trajectory instance is closed"))?;
            let binding = owner
                .bindings
                .get(&host)
                .cloned()
                .ok_or_else(|| RereleaseWeaponError::invalid("Native projectile binding is missing"))?;
            if !owner.live(host, &binding)? {
                return owner.retire_trajectory(&binding, body);
            }
            {
                let retained = owner.retained()?;
                retained
                    .profile
                    .project(
                        retained.module,
                        binding.module_slot,
                        &WeaponBodyState {
                            origin: body.origin,
                            angles: body.angles,
                            velocity: body.velocity,
                        },
                    )
                    .map_err(weapon_mapped)?;
            }
            let next = {
                let retained = owner.retained()?;
                retained
                    .profile
                    .next_think(retained.module, binding.module_slot)
                    .map_err(weapon_mapped)?
            };
            if next <= 0 || next > (time_seconds * 1000.0).round() as i64 {
                return Ok(None);
            }
            {
                let retained = owner.retained()?;
                retained
                    .profile
                    .think(retained.module, binding.module_slot)
                    .map_err(weapon_mapped)?;
            }
            if !owner.live(host, &binding)? {
                return owner.retire_trajectory(&binding, body);
            }
            let retained = owner.retained()?;
            let update = retained
                .profile
                .trajectory(retained.module, binding.module_slot)
                .map_err(weapon_mapped)?;
            Ok(Some(WeaponTrajectoryUpdate {
                origin: update.origin,
                velocity: update.velocity,
                angles: update.angles,
            }))
        })
    }

    fn retire_trajectory(
        &mut self,
        binding: &WeaponBinding,
        body: &EngineBodyState,
    ) -> RereleaseWeaponResult<Option<WeaponTrajectoryUpdate>> {
        let actor = binding.actor.id().clone();
        self.retired.insert(
            actor.clone(),
            WeaponTrajectoryUpdate {
                origin: body.origin,
                velocity: body.velocity,
                angles: body.angles,
            },
        );
        if let Some(host) = self.slots.remove(&actor) {
            self.bindings.remove(&host);
        }
        Ok(None)
    }

    /// Close a trajectory instance (donor `WeaponBehaviorInstance.close`).
    pub fn close_instance(&mut self, instance: WeaponBehaviorInstance) -> RereleaseWeaponResult<()> {
        if instance.closed {
            return Ok(());
        }
        self.instances.remove(&instance.actor);
        if instance.slot.is_none() {
            self.retired.remove(&instance.actor);
            Ok(())
        } else {
            self.release_actor(&instance.actor)
        }
    }

    fn release_actor(&mut self, actor: &ActorId) -> RereleaseWeaponResult<()> {
        self.instances.remove(actor);
        let mut slots = std::mem::take(&mut self.slots);
        let mut bindings = std::mem::take(&mut self.bindings);
        let mut retired = std::mem::take(&mut self.retired);
        let result = retire_rerelease_weapon_actor(&mut slots, &mut bindings, &mut retired, actor, |host, binding| {
            if self.closed {
                return Ok(());
            }
            if binding.kind == WeaponBindingKind::Client {
                // The connect handshake has no compat counterpart, so
                // only the client reservation is released.
                let retained = self.retained()?;
                retained
                    .source
                    .host
                    .release_client_reservation(host)
                    .map_err(host_mapped)?;
                Ok(())
            } else if self.live(host, binding)? {
                let retained = self.retained()?;
                retained
                    .profile
                    .free(retained.module, binding.module_slot)
                    .map_err(weapon_mapped)
            } else {
                Ok(())
            }
        });
        self.slots = slots;
        self.bindings = bindings;
        self.retired = retired;
        result
    }

    /// Capture the component checkpoint (donor `checkpoint`, synchronous).
    pub fn checkpoint(&mut self) -> RereleaseWeaponResult<RereleaseWeaponBehaviorCheckpoint> {
        if self.busy {
            return Err(RereleaseWeaponError::invalid(
                "Cannot save an active native weapon call",
            ));
        }
        self.busy = true;
        let result = self.checkpoint_inner();
        self.busy = false;
        self.flush_releases()?;
        result
    }

    fn checkpoint_inner(&mut self) -> RereleaseWeaponResult<RereleaseWeaponBehaviorCheckpoint> {
        let game = self
            .retained()?
            .source
            .host
            .write_save("game", false)
            .map_err(host_mapped)?;
        let level = self
            .retained()?
            .source
            .host
            .write_save("level", false)
            .map_err(host_mapped)?;
        if !self.pending_releases.is_empty() {
            return Err(RereleaseWeaponError::invalid(
                "Primary actors changed during native component capture",
            ));
        }
        let capture = self
            .capture_cvars
            .clone()
            .ok_or_else(|| RereleaseWeaponError::invalid("Native weapon checkpoint needs a cvar capture seam"))?;
        let cvars = capture(self.retained()?.services.cvars());
        let mut configstrings: Vec<WeaponConfigstringEntry> = self
            .retained()?
            .services
            .configstrings()
            .into_iter()
            .map(|(index, value)| WeaponConfigstringEntry { index, value })
            .collect();
        configstrings.sort_by_key(|entry| entry.index);
        let retired: Vec<WeaponRetiredEntry> = self
            .retired
            .iter()
            .map(|(actor, trajectory)| WeaponRetiredEntry {
                actor: SavedActorId::from(actor),
                trajectory: *trajectory,
            })
            .collect();
        let snapshot: Vec<(u32, WeaponBinding)> = self
            .bindings
            .iter()
            .map(|(host, binding)| (*host, binding.clone()))
            .collect();
        let mut bindings = Vec::with_capacity(snapshot.len());
        for (host, binding) in &snapshot {
            let retained = self.retained()?;
            let generation = retained
                .profile
                .generation(retained.module, binding.module_slot)
                .map_err(weapon_mapped)?;
            bindings.push(WeaponBindingEntry {
                slot: *host,
                actor: SavedActorId::from(binding.actor.id()),
                kind: binding.kind,
                generation,
            });
        }
        Ok(RereleaseWeaponBehaviorCheckpoint {
            version: 1,
            declaration: self.declaration.clone(),
            definition: self.definition.clone(),
            map: self.map.clone(),
            time: self.time,
            cvars,
            game,
            level,
            configstrings,
            retired,
            bindings,
        })
    }

    /// Restore a captured checkpoint (donor `restore`, synchronous).
    ///
    /// Saved records are reallocated fresh and reseeded with their saved
    /// generations (the donor's level save populates them); the entity
    /// spawn has no compat counterpart, so restoration validates the
    /// recomputed initialization entities instead of spawning.
    pub fn restore(
        &mut self,
        saved: &RereleaseWeaponBehaviorCheckpoint,
        actor: &dyn Fn(SavedActorId) -> OwnedActor,
    ) -> RereleaseWeaponResult<()> {
        if self.busy
            || !self.bindings.is_empty()
            || saved.version != 1
            || !same_weapon_behavior(&saved.definition, &self.definition)
            || !same_native_weapon_declaration(&saved.declaration, &self.declaration)
            || saved.map.map != self.map.map
            || saved.map.entities != self.map.entities
            || saved.map.spawn_point != self.map.spawn_point
            || !saved.time.is_finite()
            || saved.time < 0.0
        {
            return Err(RereleaseWeaponError::invalid(
                "Incompatible native trajectory checkpoint",
            ));
        }
        self.busy = true;
        let result = self.restore_inner(saved, actor);
        self.busy = false;
        self.flush_releases()?;
        result
    }

    fn restore_inner(
        &mut self,
        saved: &RereleaseWeaponBehaviorCheckpoint,
        actor: &dyn Fn(SavedActorId) -> OwnedActor,
    ) -> RereleaseWeaponResult<()> {
        let restore = self
            .restore_cvars
            .clone()
            .ok_or_else(|| RereleaseWeaponError::invalid("Native weapon restore needs a cvar restore seam"))?;
        restore(self.retained()?.services.cvars_mut(), &saved.cvars)?;
        let recomputed = weapon_initialization_entities(&saved.map.entities, &self.declaration.initialization_classes)
            .map_err(weapon_mapped)?;
        if recomputed != self.initialization_entities {
            return Err(RereleaseWeaponError::invalid(
                "Native component initialization entities changed",
            ));
        }
        self.retained()?
            .source
            .host
            .read_save(&saved.game)
            .map_err(host_mapped)?;
        {
            let retained = self.retained()?;
            let table: HashMap<i32, String> = saved
                .configstrings
                .iter()
                .map(|entry| (entry.index, entry.value.clone()))
                .collect();
            retained.services.restore_configstrings(&table).map_err(host_mapped)?;
        }
        for binding in &saved.bindings {
            if binding.slot < 1 || self.bindings.contains_key(&binding.slot) {
                return Err(RereleaseWeaponError::invalid("Invalid native component saved slot"));
            }
            let owner = actor(binding.actor);
            if self.slots.contains_key(owner.id()) {
                return Err(RereleaseWeaponError::invalid("Duplicate native component saved actor"));
            }
            let module_slot = {
                let retained = self.retained()?;
                let slot = retained.profile.allocate(retained.module).map_err(weapon_mapped)?;
                if binding.kind == WeaponBindingKind::Client {
                    let client_length = retained.profile.declaration.client.byte_length;
                    let client_offset = retained.profile.declaration.entity.client;
                    retained
                        .module
                        .attach_client(slot, client_length, client_offset)
                        .map_err(weapon_mapped)?;
                }
                let record = retained.module.entity(slot).map_err(weapon_mapped)?;
                let generation_offset = retained.profile.declaration.entity.generation as i64;
                retained
                    .module
                    .memory
                    .write_i32(
                        retained
                            .module
                            .memory
                            .offset(record, generation_offset)
                            .map_err(host_mapped)?,
                        binding.generation,
                    )
                    .map_err(host_mapped)?;
                slot
            };
            self.set_inuse(module_slot, true)?;
            self.bindings.insert(
                binding.slot,
                WeaponBinding {
                    actor: owner.clone(),
                    kind: binding.kind,
                    generation: binding.generation,
                    module_slot,
                },
            );
            self.slots.insert(owner.id().clone(), binding.slot);
        }
        for retired in &saved.retired {
            let owner = actor(retired.actor);
            if self.slots.contains_key(owner.id()) || self.retired.contains_key(owner.id()) {
                return Err(RereleaseWeaponError::invalid("Invalid retired native trajectory actor"));
            }
            self.retired.insert(owner.id().clone(), retired.trajectory);
        }
        self.retained()?
            .source
            .host
            .read_save(&saved.level)
            .map_err(host_mapped)?;
        let snapshot: Vec<(u32, WeaponBinding)> = self
            .bindings
            .iter()
            .map(|(host, binding)| (*host, binding.clone()))
            .collect();
        for (host, binding) in &snapshot {
            let retained = self.retained()?;
            let generation = retained
                .profile
                .generation(retained.module, binding.module_slot)
                .map_err(weapon_mapped)?;
            if generation != binding.generation {
                return Err(RereleaseWeaponError::invalid(
                    "Native source save changed projectile generation",
                ));
            }
            if binding.kind == WeaponBindingKind::Client {
                retained.source.host.reserve_client(*host).map_err(host_mapped)?;
            }
        }
        self.time = saved.time;
        {
            let retained = self.retained()?;
            retained
                .profile
                .set_time(retained.module, (saved.time * 1000.0).round() as i64);
        }
        self.services.as_mut().expect("checked").drain_messages();
        Ok(())
    }

    /// Close the component, releasing the source and clearing bindings.
    pub fn close(&mut self) -> RereleaseWeaponResult<()> {
        if self.closed {
            return Ok(());
        }
        if self.busy {
            return Err(RereleaseWeaponError::invalid(
                "Cannot close an active native weapon component",
            ));
        }
        self.closed = true;
        let result = match self.source.as_mut() {
            Some(source) => source.close().map_err(host_mapped),
            None => Ok(()),
        };
        self.bindings.clear();
        self.slots.clear();
        self.instances.clear();
        self.retired.clear();
        self.pending_releases.clear();
        self.source = None;
        self.services = None;
        self.profile = None;
        self.module = None;
        result
    }
}

impl WeaponBehaviorSource for RereleaseWeaponBehaviorSource {
    fn definition(&self) -> RereleaseWeaponResult<WeaponBehaviorDefinition> {
        Ok(self.definition.clone())
    }

    fn attach(&mut self, launch: &WeaponBehaviorLaunch) -> RereleaseWeaponResult<Option<WeaponBehaviorInstance>> {
        RereleaseWeaponBehaviorSource::attach(self, launch)
    }

    fn resume(&mut self, projectile: &ActorId) -> RereleaseWeaponResult<WeaponBehaviorInstance> {
        RereleaseWeaponBehaviorSource::resume(self, projectile)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::collections::HashMap;

    use qa_bots::scene::{
        LeafQueryResult, PointContentsQuery, PointContentsResult, SceneQueries, TraceQuery,
        TraceResult as BotsTraceResult, VisibilityKind,
    };
    use qa_compat::q2::rerelease::native_weapon_declaration::{
        DeclCalls, DeclClientFields, DeclCommand, DeclCvar, DeclEntityFields, DeclEntry, DeclEquippedWeapon,
        DeclRegistrationLayout,
    };
    use qa_compat::q2::rerelease::navigation::{GoalStatus, NavRuntime, NavigationServices};
    use qa_content::contract::{ArmorState, InventoryEntry};
    use qa_content::hash::sha256_hex;
    use qa_content::q2::foundation::host::{
        Q2FoundationHost, Q2LandmarkCarry, Q2Motion, Q2PlayerViewState, Q2PresentationEvent, Q2Solid, Q2TraceRequest,
    };
    use qa_content::q2::support::contracts::{
        ActorObservation, BodyAttachment, CombatState, CombatTraitChanges, DamageOutcome, DamageRequest, LinkedBody,
        PowerArmorCells, Q2BspPlane, Q2TraceFields, TraceContact as EngineContact, TraceFamily, TraceHit as EngineHit,
        TraceResult as EngineTraceResult, TransitionIntent,
    };
    use qa_content::q2::support::tables::{
        Q2ActorRegistry, Q2BodyTable, Q2CallbackTable, Q2CombatAuthority, Q2InventoryTable,
    };
    use qa_core::cmd::Dialect;
    use qa_core::identity::{IdentityOwner, ProviderId};
    use qa_core::numeric::{NumericOps, Q2_DONOR_PROFILE};
    use qa_world::spatial::QueryRole;

    use super::super::classic_guest_services::{ClassicGuestServicesOptions, GuestCommandLine};
    use super::super::q2_native_world::fixtures::*;
    use super::super::rerelease_guest_services_contract::RereleaseGuestClipboard;
    use super::super::rerelease_guest_services_contract::RereleaseGuestServicesOptions;
    use super::super::source_hosts::ActorHostScene;
    use super::*;
    use crate::persistence::recipe::ExecutionImplementation;

    const ZERO: Vec3 = Vec3 { x: 0.0, y: 0.0, z: 0.0 };
    const ZERO_BOUNDS: Bounds = Bounds { min: ZERO, max: ZERO };

    struct FakeEngine {
        owner: Rc<IdentityOwner>,
        next_slot: u32,
        actors: HashMap<ActorId, (ProviderId, u32, String)>,
        bodies: HashMap<ActorId, EngineBodyState>,
        bound: HashMap<ActorId, bool>,
        combats: HashMap<ActorId, CombatState>,
        emitted: Rc<RefCell<Vec<String>>>,
        world: ActorId,
    }

    impl FakeEngine {
        fn new(emitted: Rc<RefCell<Vec<String>>>) -> Self {
            let owner = Rc::new(IdentityOwner::create("weapon-behavior-test").expect("owner"));
            let world = owner.actor(0, 0);
            Self {
                owner,
                next_slot: 1,
                actors: HashMap::new(),
                bodies: HashMap::new(),
                bound: HashMap::new(),
                combats: HashMap::new(),
                emitted,
                world,
            }
        }
    }

    impl Q2ActorRegistry for FakeEngine {
        fn allocate(&mut self, owner: &ProviderId, definition: &str) -> OwnedActor {
            let slot = self.next_slot;
            self.next_slot += 1;
            let id = self.owner.actor(slot, 1);
            self.actors
                .insert(id.clone(), (owner.clone(), slot, definition.to_string()));
            self.owner.owned_actor(&id, owner.clone()).expect("owned")
        }

        fn allocate_at_source(&mut self, owner: &ProviderId, source_slot: u32, definition: &str) -> OwnedActor {
            let id = self.owner.actor(source_slot, 1);
            self.actors
                .insert(id.clone(), (owner.clone(), source_slot, definition.to_string()));
            self.owner.owned_actor(&id, owner.clone()).expect("owned")
        }

        fn source_of(&self, actor: &ActorId) -> Option<(ProviderId, u32)> {
            self.actors.get(actor).map(|(owner, slot, _)| (owner.clone(), *slot))
        }

        fn release(&mut self, actor: &OwnedActor) {
            self.actors.remove(actor.id());
        }

        fn is_live(&self, actor: &ActorId) -> bool {
            *actor == self.world || self.actors.contains_key(actor)
        }

        fn resolve_owned(&self, actor: &ActorId) -> Option<OwnedActor> {
            self.actors
                .get(actor)
                .map(|(owner, _, _)| self.owner.owned_actor(actor, owner.clone()).expect("owned"))
        }

        fn observations(&self) -> Vec<ActorObservation> {
            self.actors
                .iter()
                .map(|(id, (owner, _, definition))| ActorObservation {
                    id: id.clone(),
                    owner: owner.clone(),
                    definition: definition.clone(),
                })
                .collect()
        }

        fn resolve_saved(&self, _saved: SavedActorId) -> Option<OwnedActor> {
            None
        }

        fn reference_saved(&self, saved: SavedActorId) -> ActorId {
            self.owner.actor(saved.slot, saved.generation)
        }

        fn assert_owned(&self, _actor: &OwnedActor) {}
    }

    impl Q2BodyTable for FakeEngine {
        fn create(&mut self, actor: &OwnedActor, initial: &EngineBodyState) {
            self.bodies.insert(actor.id().clone(), initial.clone());
        }

        fn read(&self, actor: &ActorId) -> Option<EngineBodyState> {
            self.bodies.get(actor).cloned()
        }

        fn write(&mut self, actor: &OwnedActor, state: &EngineBodyState) {
            self.bodies.insert(actor.id().clone(), state.clone());
        }

        fn attach(&mut self, _actor: &OwnedActor, _attachment: &BodyAttachment) {}

        fn detach(&mut self, _actor: &OwnedActor) {}

        fn attachment(&self, _actor: &ActorId) -> Option<BodyAttachment> {
            None
        }

        fn linked(&self, _actor: &ActorId) -> Option<LinkedBody> {
            None
        }

        fn link(&mut self, _actor: &OwnedActor, _origin: Option<Vec3>) {}

        fn unlink(&mut self, _actor: &OwnedActor) {}
    }

    impl Q2CallbackTable for FakeEngine {
        fn bind(&mut self, actor: &OwnedActor) {
            self.bound.insert(actor.id().clone(), true);
        }

        fn unbind(&mut self, actor: &ActorId) {
            self.bound.remove(actor);
        }

        fn is_bound(&self, actor: &ActorId) -> bool {
            self.bound.contains_key(actor)
        }

        fn forward_use(&mut self, _actor: &OwnedActor, _other: Option<&ActorId>, _activator: Option<&ActorId>) {}
    }

    impl Q2CombatAuthority for FakeEngine {
        fn create(&mut self, actor: &OwnedActor, initial: &CombatState) {
            self.combats.insert(actor.id().clone(), initial.clone());
        }

        fn read(&self, actor: &ActorId) -> Option<CombatState> {
            self.combats.get(actor).cloned()
        }

        fn set_health(&mut self, actor: &OwnedActor, health: f64) {
            if let Some(state) = self.combats.get_mut(actor.id()) {
                state.health = health;
            }
        }

        fn set_armor(&mut self, actor: &OwnedActor, armor: &ArmorState) {
            if let Some(state) = self.combats.get_mut(actor.id()) {
                state.armor = armor.clone();
            }
        }

        fn set_regular_points(
            &mut self,
            _actor: &OwnedActor,
            _points: f64,
            _initial: Option<&qa_content::contract::RegularArmorState>,
        ) {
        }

        fn set_regular_armor(&mut self, _actor: &OwnedActor, _regular: &qa_content::contract::RegularArmorState) {}

        fn set_powered_protection(
            &mut self,
            _actor: &OwnedActor,
            _powered: &qa_content::contract::PoweredProtectionState,
        ) {
        }

        fn set_traits(&mut self, _actor: &OwnedActor, _changes: &CombatTraitChanges) {}

        fn bind_power_armor_cells(&mut self, _actor: &OwnedActor, _cells: Box<dyn PowerArmorCells>) {}

        fn apply(&mut self, input: &DamageRequest) -> DamageOutcome {
            DamageOutcome::StaleTarget { request: input.clone() }
        }
    }

    impl Q2InventoryTable for FakeEngine {
        fn create(&mut self, _actor: &OwnedActor, _entries: &[InventoryEntry]) {}

        fn entries(&self, _actor: &ActorId) -> Vec<InventoryEntry> {
            Vec::new()
        }

        fn has(&self, _actor: &ActorId) -> bool {
            false
        }

        fn count(&self, _actor: &ActorId, _item: &qa_content::contract::ItemId) -> f64 {
            0.0
        }

        fn consume(&mut self, _actor: &OwnedActor, _item: &qa_content::contract::ItemId, _count: f64) -> bool {
            false
        }

        fn give(&mut self, _actor: &OwnedActor, _item: &qa_content::contract::ItemId, _count: f64) -> f64 {
            0.0
        }

        fn configure(&mut self, _actor: &OwnedActor, _entry: &InventoryEntry) {}

        fn adjust_source_counter(
            &mut self,
            _actor: &OwnedActor,
            _item: &qa_content::contract::ItemId,
            delta: f64,
        ) -> f64 {
            delta
        }
    }

    impl Q2FoundationHost for FakeEngine {
        fn actors(&mut self) -> &mut dyn Q2ActorRegistry {
            self
        }

        fn bodies(&mut self) -> &mut dyn Q2BodyTable {
            self
        }

        fn callbacks(&mut self) -> &mut dyn Q2CallbackTable {
            self
        }

        fn combat(&mut self) -> &mut dyn Q2CombatAuthority {
            self
        }

        fn inventory(&mut self) -> &mut dyn Q2InventoryTable {
            self
        }

        fn now(&self) -> f64 {
            0.0
        }

        fn frame_seconds(&self) -> f64 {
            0.1
        }

        fn gravity(&self) -> f64 {
            800.0
        }

        fn random(&mut self) -> f64 {
            0.5
        }

        fn schedule(&mut self, _actor: &OwnedActor, _due_seconds: Option<f64>) {}

        fn touch_triggers(&mut self, _actor: &OwnedActor) {}

        fn trace(&mut self, request: &Q2TraceRequest) -> EngineTraceResult {
            EngineTraceResult {
                fraction: 1.0,
                end: request.end,
                start_solid: false,
                all_solid: false,
                contact: EngineContact::None,
                hit: EngineHit::None,
                family: TraceFamily::Q2(Q2TraceFields {
                    contents: 0,
                    surface: None,
                    source_plane: Q2BspPlane {
                        normal: ZERO,
                        distance: 0.0,
                        plane_type: 0,
                        signbits: 0,
                    },
                    secondary: None,
                }),
            }
        }

        fn point_contents(&mut self, _point: Vec3) -> i32 {
            0
        }

        fn in_pvs(&mut self, _first: Vec3, _second: Vec3) -> bool {
            true
        }

        fn in_phs(&mut self, _first: Vec3, _second: Vec3) -> bool {
            true
        }

        fn areas_connected(&mut self, _first: Vec3, _second: Vec3) -> bool {
            true
        }

        fn nearby(&mut self, _origin: Vec3, _radius: f64) -> Vec<ActorId> {
            Vec::new()
        }

        fn players(&mut self) -> Vec<ActorId> {
            Vec::new()
        }

        fn world_actor(&mut self) -> ActorId {
            self.world.clone()
        }

        fn is_player(&mut self, _actor: &ActorId) -> bool {
            false
        }

        fn is_monster(&mut self, _actor: &ActorId) -> bool {
            false
        }

        fn inline_model_bounds(&mut self, _model: i32) -> Bounds {
            ZERO_BOUNDS
        }

        fn set_solid(&mut self, _actor: &OwnedActor, _solid: Q2Solid, _model: Option<i32>) {}

        fn set_motion(&mut self, _motion: &Q2Motion) {}

        fn set_area_portal(&mut self, _portal: i32, _open: bool) {}

        fn emit(&mut self, event: Q2PresentationEvent) {
            self.emitted.borrow_mut().push(format!("{event:?}"));
        }

        fn player_view_state(&mut self, _player: &ActorId) -> Option<Q2PlayerViewState> {
            None
        }

        fn key_consumed(&mut self, _player: &ActorId) {}

        fn prepare_level_change(&mut self, _map: &str, _landmark: Option<&Q2LandmarkCarry>, _server_flags: i32) {}

        fn transition(&mut self, _intent: TransitionIntent) {}

        fn diagnostic(&mut self, _message: &str) {}
    }

    struct FakeScene;

    impl FakeScene {
        fn miss() -> BotsTraceResult {
            BotsTraceResult {
                fraction: 1.0,
                end: ZERO,
                start_solid: false,
                all_solid: false,
                contact: qa_bots::scene::TraceContact::None,
                hit: qa_bots::scene::TraceHit::None,
                detail: qa_bots::scene::TraceDetail::Q1 {
                    in_open: true,
                    in_water: false,
                    source_plane: qa_core::math::Plane {
                        normal: ZERO,
                        distance: 0.0,
                    },
                    surface_flags: None,
                },
            }
        }
    }

    impl SceneQueries for FakeScene {
        fn trace(&self, _query: &TraceQuery) -> BotsTraceResult {
            Self::miss()
        }

        fn point_contents(&self, _query: &PointContentsQuery) -> PointContentsResult {
            PointContentsResult::Q2 { stored: 0, merged: 0 }
        }

        fn box_leaves(&self, _bounds: &Bounds, _limit: usize) -> LeafQueryResult {
            LeafQueryResult {
                leaves: Vec::new(),
                topnode: None,
                overflow: false,
            }
        }

        fn areas_connected(&self, _first: i32, _second: i32) -> bool {
            true
        }

        fn cluster_visible(&self, _from: i32, _to: i32, _kind: VisibilityKind) -> bool {
            true
        }
    }

    impl ActorHostScene for FakeScene {
        fn trace_excluding(&self, _query: &TraceQuery, _excluded: &[ActorId]) -> BotsTraceResult {
            Self::miss()
        }

        fn geometry_trace(&self, _query: &TraceQuery) -> BotsTraceResult {
            Self::miss()
        }

        fn point_leaf(&self, _point: Vec3) -> i32 {
            0
        }

        fn leaf_cluster(&self, _leaf: i32) -> i32 {
            0
        }

        fn leaf_area(&self, _leaf: i32) -> i32 {
            0
        }

        fn model_bounds(&self, _model: i32) -> Bounds {
            ZERO_BOUNDS
        }

        fn query_actors(&self, _bounds: &Bounds, _role: QueryRole) -> Vec<qa_world::spatial::SpatialActor> {
            Vec::new()
        }

        fn model_count(&self) -> usize {
            0
        }

        fn q2_texture_info(&self) -> Option<Vec<super::super::source_hosts::SceneTextureInfo>> {
            None
        }
    }

    struct FakeNav;

    impl NavigationServices for FakeNav {
        fn runtime(&self) -> Option<&NavRuntime> {
            None
        }

        fn move_to_point(&mut self, _actor: u32, _point: Vec3, _tolerance: f32) -> GoalStatus {
            0
        }

        fn follow_actor(&mut self, _actor: u32, _target: u32) -> GoalStatus {
            0
        }
    }

    fn provider() -> ProviderId {
        ProviderId {
            namespace: "test".to_string(),
            name: "weapon".to_string(),
        }
    }

    fn test_owner() -> Rc<IdentityOwner> {
        Rc::new(IdentityOwner::create("weapon-harness").expect("owner"))
    }

    fn declaration(digest: &str) -> NativeWeaponBehaviorDeclaration {
        let entry = |rva: u64| DeclEntry {
            rva,
            registration: None,
        };
        NativeWeaponBehaviorDeclaration {
            version: 1,
            kind: "native-weapon-profile".to_string(),
            abi: "windows-x86_64".to_string(),
            artifact_path: "q2game.dll".to_string(),
            artifact_digest: digest.to_string(),
            id: "rl".to_string(),
            title: "Rocket Launcher".to_string(),
            role: "rocket".to_string(),
            aspect: "first-person".to_string(),
            entity: DeclEntityFields {
                byte_length: 16384,
                origin: 0,
                angles: 12,
                velocity: 24,
                client: 40,
                owner: 48,
                view_height: 56,
                generation: 60,
                next_think: 64,
                think_callback: 72,
                think_registration: 80,
                touch_callback: 88,
            },
            client: DeclClientFields {
                byte_length: 256,
                weapon: 0,
                view_angles: 8,
                forward: 20,
            },
            equipped_weapon: DeclEquippedWeapon {
                byte_length: 64,
                callback: 0,
                expected: entry(0x500),
            },
            time_storage: "int64".to_string(),
            time_rva: 0x40,
            think_signature: "think".to_string(),
            think_tag: 7,
            think_registration: DeclRegistrationLayout {
                byte_length: 32,
                name: 0,
                tag: 8,
                callback: 16,
            },
            allocate_signature: "allocate".to_string(),
            allocate: entry(0x100),
            free_signature: "free".to_string(),
            free: entry(0x110),
            projectile_touch: entry(0x120),
            equip: DeclCalls {
                signature: "equip".to_string(),
                calls: vec![entry(0x200)],
            },
            launch: DeclCalls {
                signature: "launch".to_string(),
                calls: vec![entry(0x300)],
            },
            activate_rva: None,
            fire_rva: 0x300,
            initialization_classes: vec!["worldspawn".to_string()],
            equipment: vec![DeclCommand {
                arguments: vec!["give".to_string(), "rockets".to_string()],
                tail: "rockets".to_string(),
            }],
            ammunition: DeclCommand {
                arguments: vec!["use".to_string(), "rl".to_string()],
                tail: "rl".to_string(),
            },
            initial_cvars: vec![DeclCvar {
                name: "w_skill".to_string(),
                value: "2".to_string(),
            }],
            provisioning_cvars: vec![DeclCvar {
                name: "w_prov".to_string(),
                value: "1".to_string(),
            }],
        }
    }

    fn prepared_with_digest() -> (PreparedRereleaseGuest, String) {
        let mut artifacts = HashMap::new();
        let bytes = minimal_pe(0x8664, 0x20b, &["GetGameAPI", "GetCGameAPI"]);
        artifacts.insert("q2game.dll".to_string(), bytes);
        let mut execution = rerelease_execution();
        let digest = format!("sha256:{}", sha256_hex(&artifacts["q2game.dll"]));
        if let ExecutionImplementation::Native { artifact, .. } = &mut execution.implementation {
            artifact.digest = digest.clone();
        }
        let mounts = FakeMounts::new(artifacts, None);
        let guest =
            super::super::rerelease_guest_source::prepare_rerelease_guest(&execution, &mounts).expect("prepared");
        (guest, digest)
    }

    fn services_options() -> RereleaseGuestServicesOptions {
        let mut cvars = CvarRegistry::new(Dialect::Q2Classic);
        cvars.set("w_prov", "0", true).expect("register");
        let base = ClassicGuestServicesOptions {
            primary_world: None,
            pickup_profile: None,
            pickups: None,
            damage_provenance: None,
            engine: Box::new(FakeEngine::new(Rc::new(RefCell::new(Vec::new())))),
            scene: Rc::new(FakeScene),
            cvars,
            numeric: NumericOps::select(Q2_DONOR_PROFILE).expect("numeric"),
            map_path: "q2dm1".to_string(),
            max_clients: 4,
            accepts_client: None,
            admit: Box::new(|_, _| {}),
            collision: Box::new(|_, _| {}),
            print: Box::new(|_| {}),
            command: Rc::new(|| GuestCommandLine {
                arguments: Vec::new(),
                args: String::new(),
            }),
            add_command: Box::new(|_| {}),
            debug_graph: Box::new(|_, _| {}),
        };
        RereleaseGuestServicesOptions {
            base,
            frame_milliseconds: 100,
            localize: Rc::new(|format, _| format.to_string()),
            clipboard: RereleaseGuestClipboard::Dedicated,
            debug_shapes: Box::new(|_| {}),
            world_text: Box::new(|_| {}),
            navigation: Box::new(FakeNav),
            semantic_bindings: None,
            foreign_damage: None,
        }
    }

    fn entities_text() -> String {
        "{\n\"classname\" \"worldspawn\"\n\"message\" \"test\"\n}\n{\n\"classname\" \"info_player_start\"\n\"origin\" \"0 0 0\"\n}\n".to_string()
    }

    struct HarnessDeps {
        owner: Rc<IdentityOwner>,
        provider: ProviderId,
        pumps: Rc<Cell<usize>>,
    }

    fn weapon_options() -> (RereleaseWeaponBehaviorOptions, HarnessDeps) {
        let (guest, digest) = prepared_with_digest();
        let owner = test_owner();
        let provider = provider();
        let actor_owner = owner.clone();
        let actor_provider = provider.clone();
        let pumps = Rc::new(Cell::new(0));
        let pump = pumps.clone();
        let options = RereleaseWeaponBehaviorOptions {
            prepared: guest,
            declaration: declaration(&digest),
            services: services_options(),
            clock: super::super::rerelease_guest_source::GuestClock {
                now_milliseconds: Rc::new(|| 0),
                performance_counter: Rc::new(|| 0),
                performance_frequency: 1,
            },
            map: ClassicGuestMap {
                map: "q2dm1".to_string(),
                entities: entities_text(),
                spawn_point: "start".to_string(),
            },
            ownership: WeaponBehaviorOwnership,
            actor: Rc::new(move |id| RereleaseWeaponActor {
                body: EngineBodyState {
                    origin: ZERO,
                    angles: ZERO,
                    velocity: ZERO,
                    bounds: ZERO_BOUNDS,
                    ground: None,
                },
                view_angles: ZERO,
                view_height: 22,
                actor: actor_owner.owned_actor(&id, actor_provider.clone()).expect("owned"),
                userinfo: "\\name\\shooter".to_string(),
            }),
            next_frame: Box::new(move || {
                pump.set(pump.get() + 1);
            }),
            capture_cvars: Some(Rc::new(|cvars| {
                cvars
                    .snapshots(0)
                    .into_iter()
                    .map(|snapshot| format!("{}={}\n", snapshot.name, snapshot.value))
                    .collect::<String>()
                    .into_bytes()
            })),
            restore_cvars: Some(Rc::new(|cvars, bytes| {
                let text = String::from_utf8(bytes.to_vec())
                    .map_err(|error| RereleaseWeaponError::invalid(error.to_string()))?;
                for line in text.lines() {
                    let (name, value) = line
                        .split_once('=')
                        .ok_or_else(|| RereleaseWeaponError::invalid("bad cvar line"))?;
                    cvars.set(name, value, true).map_err(host_mapped)?;
                }
                Ok(())
            })),
        };
        (options, HarnessDeps { owner, provider, pumps })
    }

    struct Harness {
        component: RereleaseWeaponBehaviorSource,
        owner: Rc<IdentityOwner>,
        provider: ProviderId,
        pumps: Rc<Cell<usize>>,
    }

    fn harness() -> Harness {
        let (options, deps) = weapon_options();
        let component = RereleaseWeaponBehaviorSource::create(options).expect("component");
        Harness {
            component,
            owner: deps.owner,
            provider: deps.provider,
            pumps: deps.pumps,
        }
    }

    fn launch(owner: &IdentityOwner, provider: &ProviderId, role: ProjectileRole) -> WeaponBehaviorLaunch {
        WeaponBehaviorLaunch {
            projectile: owner.owned_actor(&owner.actor(11, 1), provider.clone()).expect("owned"),
            shooter: owner.actor(7, 1),
            weapon: "weapon_rocketlauncher".to_string(),
            role,
            time_seconds: 1.5,
            body: EngineBodyState {
                origin: Vec3 { x: 1.0, y: 2.0, z: 3.0 },
                angles: ZERO,
                velocity: Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 10.0,
                },
                bounds: ZERO_BOUNDS,
                ground: None,
            },
        }
    }

    #[test]
    fn retire_removes_binding_and_runs_dispose() {
        let owner = test_owner();
        let actor = owner.owned_actor(&owner.actor(3, 1), provider()).expect("owned");
        let mut slots = HashMap::new();
        slots.insert(actor.id().clone(), 9);
        let mut bindings = HashMap::new();
        bindings.insert(
            9,
            WeaponBinding {
                actor: actor.clone(),
                kind: WeaponBindingKind::Projectile,
                generation: 4,
                module_slot: 12,
            },
        );
        let mut retired = HashMap::new();
        retired.insert(
            actor.id().clone(),
            WeaponTrajectoryUpdate {
                origin: ZERO,
                velocity: ZERO,
                angles: ZERO,
            },
        );
        let mut disposed = Vec::new();
        retire_rerelease_weapon_actor(&mut slots, &mut bindings, &mut retired, actor.id(), |slot, binding| {
            assert_eq!(binding.kind(), WeaponBindingKind::Projectile);
            disposed.push((slot, binding.generation()));
            Ok(())
        })
        .expect("retire");
        assert!(slots.is_empty() && bindings.is_empty() && retired.is_empty());
        assert_eq!(disposed, vec![(9, 4)]);
    }

    #[test]
    fn retire_ignores_unknown_actor() {
        let owner = test_owner();
        let actor = owner.owned_actor(&owner.actor(3, 1), provider()).expect("owned");
        let mut slots = HashMap::new();
        let mut bindings = HashMap::new();
        let mut retired = HashMap::new();
        retire_rerelease_weapon_actor(&mut slots, &mut bindings, &mut retired, actor.id(), |_, _| {
            panic!("dispose must not run")
        })
        .expect("retire");
        assert!(slots.is_empty() && bindings.is_empty() && retired.is_empty());
    }

    #[test]
    fn intercept_import_legs() {
        for name in [
            "WriteChar",
            "WriteByte",
            "WriteShort",
            "WriteLong",
            "WriteFloat",
            "WriteAngle",
            "WritePosition",
            "WriteDir",
            "WriteString",
            "WriteEntity",
            "unicast",
            "multicast",
            "sound",
            "positioned_sound",
            "local_sound",
            "Broadcast_Print",
            "Client_Print",
            "Center_Print",
            "Loc_Print",
        ] {
            assert!(
                matches!(intercept_import_name("game", name, &[]), Some(GuestCallResult::Void)),
                "{name}"
            );
        }
        assert!(intercept_import_name("game", "linkentity", &[]).is_none());
        assert!(intercept_import_name("game", "unlinkentity", &[]).is_none());
        assert!(intercept_import_name("game", "Whatever", &[]).is_none());
    }

    #[test]
    #[should_panic(expected = "cannot mutate selected world portals")]
    fn intercept_denies_portals() {
        intercept_import_name("game", "SetAreaPortalState", &[]);
    }

    #[test]
    #[should_panic(expected = "no clipboard capability")]
    fn intercept_denies_clipboard() {
        intercept_import_name("game", "SendToClipBoard", &[]);
    }

    #[test]
    fn projectile_roles_map() {
        use ProjectileRole::*;
        for (name, role) in [
            ("rocket", Rocket),
            ("grenade", Grenade),
            ("nail", Nail),
            ("bolt", Bolt),
            ("plasma", Plasma),
            ("energy", Energy),
            ("grapple", Grapple),
        ] {
            assert_eq!(projectile_role(name).expect("role"), role);
        }
        assert!(projectile_role("bfg").is_err());
    }

    #[test]
    fn check_declaration_accepts_and_rejects() {
        let (_, digest) = prepared_with_digest();
        let good = declaration(&digest);
        check_declaration(&good, "q2game.dll", &digest).expect("accept");
        let mut bad = declaration(&digest);
        bad.artifact_path = "other.dll".to_string();
        assert!(check_declaration(&bad, "q2game.dll", &digest).is_err());
        let mut bad = declaration(&digest);
        bad.artifact_digest = "sha256:zzz".to_string();
        assert!(check_declaration(&bad, "q2game.dll", &digest).is_err());
    }

    #[test]
    fn create_builds_component() {
        let harness = harness();
        assert!(harness.pumps.get() >= 1);
        assert_eq!(harness.component.binding_count(), 0);
        assert_eq!(harness.component.time(), 0.0);
        assert_eq!(harness.component.definition().id, "rl");
        assert_eq!(harness.component.definition().role, ProjectileRole::Rocket);
        assert_eq!(harness.component.declaration().title, "Rocket Launcher");
        let services = harness.component.services.as_ref().expect("services");
        assert_eq!(services.cvars().get("w_skill").expect("preset").value, "2");
        assert_eq!(services.cvars().get("w_prov").expect("provisioning").value, "0");
    }

    #[test]
    fn create_rejects_artifact_mismatch() {
        let (mut options, _) = weapon_options();
        options.declaration.artifact_digest = "sha256:zzz".to_string();
        assert!(RereleaseWeaponBehaviorSource::create(options).is_err());
    }

    #[test]
    fn attach_step_close_cycle() {
        let mut harness = harness();
        let input = launch(&harness.owner, &harness.provider, ProjectileRole::Rocket);
        let mut instance = harness.component.attach(&input).expect("attach").expect("instance");
        assert_eq!(instance.initial.origin, input.body.origin);
        assert_eq!(instance.initial.velocity, input.body.velocity);
        assert_eq!(harness.component.binding_count(), 2);
        assert!(harness.component.attach(&input).is_err());
        let stepped = harness
            .component
            .step_instance(&mut instance, &input.body, 2.0)
            .expect("step");
        assert!(stepped.is_none());
        assert_eq!(harness.component.time(), 2.0);
        assert_eq!(harness.component.binding_count(), 2);
        harness.component.close_instance(instance).expect("close");
        assert_eq!(harness.component.binding_count(), 1);
    }

    #[test]
    fn attach_rejects_wrong_role() {
        let mut harness = harness();
        let input = launch(&harness.owner, &harness.provider, ProjectileRole::Grenade);
        assert!(harness.component.attach(&input).is_err());
    }

    #[test]
    fn checkpoint_restore_roundtrip() {
        let mut first = harness();
        let input = launch(&first.owner, &first.provider, ProjectileRole::Rocket);
        let _instance = first.component.attach(&input).expect("attach").expect("instance");
        first
            .component
            .source
            .as_mut()
            .expect("source")
            .host
            .script_save(b"game-save".to_vec());
        first
            .component
            .source
            .as_mut()
            .expect("source")
            .host
            .script_save(b"level-save".to_vec());
        let saved = first.component.checkpoint().expect("checkpoint");
        assert_eq!(saved.version, 1);
        assert_eq!(saved.bindings.len(), 2);
        assert!(!saved.cvars.is_empty());
        let mut fresh = harness();
        let owner = fresh.owner.clone();
        let provider = fresh.provider.clone();
        fresh
            .component
            .restore(&saved, &|saved| {
                owner
                    .owned_actor(&owner.actor(saved.slot, saved.generation), provider.clone())
                    .expect("owned")
            })
            .expect("restore");
        assert_eq!(fresh.component.binding_count(), 2);
        assert_eq!(fresh.component.time(), saved.time);
        let mut resumed = fresh.component.resume(&fresh.owner.actor(11, 1)).expect("resume");
        assert!(resumed.slot.is_some());
        let stepped = fresh
            .component
            .step_instance(&mut resumed, &input.body, 3.0)
            .expect("step");
        assert!(stepped.is_none());
    }

    #[test]
    fn restore_rejects_incompatible() {
        let mut first = harness();
        let input = launch(&first.owner, &first.provider, ProjectileRole::Rocket);
        let _instance = first.component.attach(&input).expect("attach").expect("instance");
        first
            .component
            .source
            .as_mut()
            .expect("source")
            .host
            .script_save(b"game-save".to_vec());
        first
            .component
            .source
            .as_mut()
            .expect("source")
            .host
            .script_save(b"level-save".to_vec());
        let mut saved = first.component.checkpoint().expect("checkpoint");
        saved.version = 2;
        let mut fresh = harness();
        let owner = fresh.owner.clone();
        let provider = fresh.provider.clone();
        assert!(fresh
            .component
            .restore(&saved, &|saved| {
                owner
                    .owned_actor(&owner.actor(saved.slot, saved.generation), provider.clone())
                    .expect("owned")
            })
            .is_err());
    }

    #[test]
    fn mirror_and_release_cycle() {
        let mut harness = harness();
        let target = harness.owner.actor(21, 1);
        let host = harness.component.mirror_actor(&target).expect("mirror");
        assert!(host > 4);
        assert_eq!(harness.component.binding_count(), 1);
        assert_eq!(harness.component.mirror_actor(&target).expect("again"), host);
        harness.component.notify_actor_released(&target).expect("release");
        assert_eq!(harness.component.binding_count(), 0);
    }

    #[test]
    fn emit_sink_drops_presentation() {
        let emitted = Rc::new(RefCell::new(Vec::new()));
        let mut adapter = EngineWithoutEmit::new(Box::new(FakeEngine::new(emitted.clone())));
        let owner = test_owner();
        adapter.emit(Q2PresentationEvent::Visibility {
            actor: owner.actor(1, 1),
            visible: true,
        });
        assert!(emitted.borrow().is_empty());
        assert_eq!(adapter.now(), 0.0);
        assert_eq!(adapter.gravity(), 800.0);
    }

    #[test]
    fn close_is_idempotent_and_poisoned_ops_fail() {
        let mut harness = harness();
        harness.component.close().expect("close");
        harness.component.close().expect("close-again");
        let input = launch(&harness.owner, &harness.provider, ProjectileRole::Rocket);
        assert!(harness.component.attach(&input).is_err());
    }
}
