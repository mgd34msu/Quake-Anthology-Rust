//! QuakeC weapon behavior source: donor bytecode trajectories.
//!
//! Provenance: `src/app/bootstrap/simulation/quakec-weapon-behavior.ts`.

use std::cell::Cell;
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::collections::HashSet;
use std::rc::Rc;
use std::rc::Weak;

use qa_content::contract::ItemId;
use qa_core::identity::ActorId;
use qa_core::identity::OwnedActor;
use qa_core::identity::SavedActorId;
use qa_core::math::Bounds;
use qa_core::math::Vec3;
use qa_core::numeric::NumericOps;
use qa_core::numeric::Q1_DONOR_PROFILE;
use qa_guest::error::GuestError;
use qa_guest::qc::builtins::create_qc_builtins;
use qa_guest::qc::builtins::QcBuiltinServices;
use qa_guest::qc::builtins::QcHostBuiltinName;
use qa_guest::qc::builtins::QcHostKind;
use qa_guest::qc::builtins::QcRandomSource;
use qa_guest::qc::builtins::QcSharedRandom;
use qa_guest::qc::machine::QcBuiltin;
use qa_guest::qc::machine::QcMachine;
use qa_guest::qc::machine::QcMachineOptions;
use qa_guest::qc::machine::QcMachineSnapshot;
use qa_guest::qc::memory::QcEntityMemory;
use qa_guest::qc::mod_provider::QcApiKind;
use qa_guest::qc::mod_provider::QcFunctionView;
use qa_guest::qc::mod_provider::QcProgramView;
use qa_guest::qc::profile::classic_qc_entity_layout;
use qa_guest::qc::program::QcProgram;
use qa_guest::qc::weapon_behavior_profile::qc_weapon_behavior_capability_error;
use qa_guest::qvm::weapon_behavior_profile::same_weapon_behavior;
use qa_guest::qvm::weapon_behavior_profile::WeaponBehaviorCallback;
use qa_guest::qvm::weapon_behavior_profile::WeaponBehaviorDefinition;
use qa_guest::qvm::weapon_behavior_profile::WeaponBehaviorRole;
use qa_world::body::BodyState;
use qa_world::collision::Q1Move;
use qa_world::save::records::read_saved_actor;
use qa_world::save::shared::read_random;
use qa_world::save::shared::SaveRandomState;
use qa_world::save::value::SaveJson;
use qa_world::save::value::SaveReader;
use qa_world::WorldError;

use super::random::RandomCheckpoint;
use super::random::SourceRandom;
use super::types::SimulationMode;

/// Mirror of `TraceHit` from donor `src/contracts/scene.ts` (canonical home:
/// collision-lane scene port); unify post-merge.
#[derive(Debug, Clone, PartialEq)]
pub enum TraceHit {
    /// No contact.
    None,
    /// World contact.
    World {
        /// Model index.
        model: i32,
    },
    /// Actor contact.
    Actor {
        /// Hit actor.
        actor: ActorId,
    },
}

/// Mirror of `WeaponBehaviorLaunch` from donor `src/contracts/weapon-behavior.ts`
/// (canonical home: `qa_guest::qvm::weapon_behavior_profile`, which absorbs that
/// contract); unify post-merge.
#[derive(Debug, Clone, PartialEq)]
pub struct WeaponBehaviorLaunch {
    /// Projectile actor.
    pub projectile: OwnedActor,
    /// Shooter actor.
    pub shooter: ActorId,
    /// Fired weapon item.
    pub weapon: ItemId,
    /// Projectile role.
    pub role: WeaponBehaviorRole,
    /// Launch time in seconds.
    pub time_seconds: f64,
    /// Launch body.
    pub body: BodyState,
}

/// Mirror of `WeaponTrajectoryUpdate` from donor `src/contracts/weapon-behavior.ts`
/// (canonical home: `qa_guest::qvm::weapon_behavior_profile`); unify post-merge.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WeaponTrajectoryUpdate {
    /// Origin.
    pub origin: Vec3,
    /// Velocity.
    pub velocity: Vec3,
    /// Angles.
    pub angles: Vec3,
}

/// Mirror of `WeaponBehaviorInstance` from donor `src/contracts/weapon-behavior.ts`
/// (canonical home: `qa_guest::qvm::weapon_behavior_profile`); unify post-merge.
pub trait WeaponBehaviorInstance {
    /// Step failure.
    type Error;
    /// Behavior definition.
    fn definition(&self) -> &WeaponBehaviorDefinition;
    /// Initial trajectory.
    fn initial(&self) -> &WeaponTrajectoryUpdate;
    /// Step the trajectory, returning `None` when retired.
    fn step(&self, body: &BodyState, time_seconds: f64) -> Result<Option<WeaponTrajectoryUpdate>, Self::Error>;
    /// Release the attachment.
    fn close(&self);
}

/// Mirror of `WeaponBehaviorSource` from donor `src/contracts/weapon-behavior.ts`
/// (canonical home: `qa_guest::qvm::weapon_behavior_profile`); unify post-merge.
pub trait WeaponBehaviorSource {
    /// Attach failure.
    type Error;
    /// Attached instance type.
    type Instance: WeaponBehaviorInstance<Error = Self::Error>;
    /// Behavior definition.
    fn definition(&self) -> &WeaponBehaviorDefinition;
    /// Attach a launch, returning `None` when the source declines the shot.
    fn attach(&self, launch: WeaponBehaviorLaunch) -> Result<Option<Self::Instance>, Self::Error>;
    /// Resume a restored projectile.
    fn resume(&self, projectile: &ActorId) -> Result<Self::Instance, Self::Error>;
}

/// Validates a saved weapon behavior definition against the loaded one.
/// Donor `readWeaponBehaviorDefinition` (`src/world/gameplay/weapon-behaviors.ts`)
/// has no ported home; callers inject the check through this seam.
pub trait WeaponBehaviorDefinitionReader {
    /// Read and validate the saved definition.
    fn read_definition(
        &self,
        reader: SaveReader<'_>,
        expected: &WeaponBehaviorDefinition,
    ) -> Result<WeaponBehaviorDefinition, WorldError>;
}

/// Mirror of `QcWeaponBehaviorTarget` from the donor module.
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
    /// Target name.
    pub name: String,
    /// Solid flag.
    pub solid: bool,
}

/// Resolved source model.
#[derive(Debug, Clone, PartialEq)]
pub struct QcWeaponModel {
    /// Model index.
    pub index: i32,
    /// Model bounds.
    pub bounds: Bounds,
}

/// Q1 point-trace result surface used by the behavior source.
#[derive(Debug, Clone, PartialEq)]
pub struct QcWeaponTrace {
    /// Trace fraction.
    pub fraction: f64,
    /// Trace end.
    pub end: Vec3,
    /// All solid.
    pub all_solid: bool,
    /// Start solid.
    pub start_solid: bool,
    /// In water.
    pub in_water: bool,
    /// In open.
    pub in_open: bool,
    /// Source plane distance.
    pub plane_distance: f64,
    /// Source plane normal.
    pub plane_normal: Vec3,
    /// Hit.
    pub hit: TraceHit,
}

/// Scene query seam. Donor `Pick<SceneQueries, 'trace' | 'pointContents'>`
/// (`src/world/collision/index.ts`) has no ported home; the fixed Q1
/// point/world policy of this module is folded into the seam.
pub trait QcWeaponScene {
    /// Trace a point segment under the Q1 move policy.
    fn trace_q1_point(&self, start: Vec3, end: Vec3, movement: Q1Move, pass_actor: Option<ActorId>) -> QcWeaponTrace;
    /// Sample Q1 contents at a point.
    fn point_contents_q1(&self, point: Vec3) -> i32;
}

/// Live target listing.
pub type QcWeaponTargets = Rc<dyn Fn() -> Vec<QcWeaponBehaviorTarget>>;
/// Aim vector lookup.
pub type QcWeaponAim = Rc<dyn Fn(&ActorId, f64) -> Vec3>;
/// Source model lookup.
pub type QcWeaponModelLookup = Rc<dyn Fn(&str) -> Option<QcWeaponModel>>;
/// Print sink.
pub type QcWeaponPrint = Rc<dyn Fn(Option<&ActorId>, &str)>;

/// Mirror of `QcWeaponBehaviorOptions` from the donor module.
pub struct QcWeaponBehaviorOptions {
    /// Behavior definition.
    pub definition: WeaponBehaviorDefinition,
    /// QuakeC program.
    pub program: QcProgram,
    /// Random source.
    pub random: SourceRandom,
    /// Scene queries.
    pub scene: Rc<dyn QcWeaponScene>,
    /// Simulation mode.
    pub mode: SimulationMode,
    /// Live targets.
    pub targets: QcWeaponTargets,
    /// Aim vector for a shooter and projectile speed.
    pub aim: QcWeaponAim,
    /// Resolve a source model.
    pub model: QcWeaponModelLookup,
    /// Print to a recipient, or broadcast.
    pub print: QcWeaponPrint,
}

/// QuakeC weapon behavior failure.
#[derive(Debug, thiserror::Error)]
pub enum QuakeCWeaponBehaviorError {
    /// Guest execution failure.
    #[error(transparent)]
    Guest(#[from] GuestError),
    /// Checkpoint read failure.
    #[error(transparent)]
    World(#[from] WorldError),
    /// Artifact lacks the behavior capability.
    #[error("{0}")]
    Capability(String),
    /// Definition does not match the program.
    #[error("Weapon behavior requires its exact declared QuakeC artifact")]
    ArtifactMismatch,
    /// Callback is not QuakeC.
    #[error("Behavior callback is not QuakeC")]
    NotQuakeC,
    /// Callback identity differs from its module.
    #[error("Behavior callback identity differs from its source module")]
    CallbackMismatch,
    /// Entity field is missing.
    #[error("Behavior source lacks entity field {0}")]
    MissingField(String),
    /// Launch role differs.
    #[error("Source behavior projectile role differs")]
    RoleMismatch,
    /// Launch already active.
    #[error("Weapon behavior launch is already active")]
    LaunchActive,
    /// Source spawned several projectiles.
    #[error("Behavior source produced multiple actors; a declared multi-projectile composition is required")]
    MultipleActors,
    /// Spawned projectile was not captured.
    #[error("Behavior projectile was not captured")]
    MissingProjectile,
    /// Restored projectile has no behavior.
    #[error("Saved projectile has no restored QuakeC behavior")]
    MissingRestored,
    /// Projectile already attached.
    #[error("QuakeC projectile behavior already has an attachment")]
    AlreadyAttached,
    /// Save during a launch.
    #[error("Cannot save during a weapon behavior launch")]
    SaveDuringLaunch,
    /// Checkpoint mismatch.
    #[error("Incompatible QuakeC weapon behavior checkpoint")]
    BadCheckpoint,
    /// Random source was not restored by the caller.
    #[error("Weapon behavior random source was not restored")]
    RandomMismatch,
    /// Saved free slot is invalid.
    #[error("Invalid saved behavior free slot")]
    BadFreeSlot,
    /// Saved entity slot is invalid.
    #[error("Invalid saved behavior entity slot")]
    BadEntitySlot,
    /// Saved actor reference is duplicated.
    #[error("Duplicate saved behavior actor reference")]
    DuplicateActor,
    /// Saved entity ownership is incomplete.
    #[error("Saved behavior entity ownership is incomplete")]
    IncompleteOwnership,
    /// Retired slot is invalid.
    #[error("Invalid retired behavior slot")]
    BadRetiredSlot,
    /// Trajectory callback spawned outside a launch.
    #[error("Trajectory callback spawned an undeclared additional actor")]
    SpawnUndeclared,
    /// Trajectory behavior removed a foreign actor.
    #[error("Trajectory behavior cannot remove another owner’s actor")]
    RemoveForeign,
    /// Source model is missing.
    #[error("No source model: {0}")]
    NoSourceModel(String),
    /// Aim has no shared shooter.
    #[error("Behavior aim has no shared shooter")]
    AimShooter,
}
impl QcRandomSource for SourceRandom {
    fn next_integer(&mut self) -> i32 {
        self.next_integer() as i32
    }

    fn next_unit(&mut self) -> f64 {
        f64::from(self.next_unit())
    }
}

/// Adapter so [`qc_weapon_behavior_capability_error`] can read a [`QcProgram`].
struct ProgramView<'p>(&'p QcProgram);

impl QcProgramView for ProgramView<'_> {
    fn digest(&self) -> &str {
        // QcProgramView borrows; the caller formats `algorithm:value` separately.
        &self.0.digest.value
    }

    fn api_kind(&self) -> QcApiKind {
        if self.0.api.is_quakeworld() {
            QcApiKind::Q1Quakeworld
        } else {
            QcApiKind::Q1Netquake
        }
    }

    fn field_type(&self, name: &str) -> Option<qa_guest::qc::mod_provider::QcValueType> {
        use qa_guest::qc::mod_provider::QcValueType as View;
        use qa_guest::qc::program::QcValueType as Program;
        self.0.field_named(name).map(|definition| match definition.value_type {
            Program::Void => View::Void,
            Program::String => View::String,
            Program::Float => View::Float,
            Program::Vector => View::Vector,
            Program::Entity => View::Entity,
            Program::Field => View::Field,
            Program::Function => View::Function,
            Program::Pointer => View::Pointer,
            Program::Opaque => View::Opaque,
        })
    }

    fn global_type(&self, name: &str) -> Option<qa_guest::qc::mod_provider::QcValueType> {
        use qa_guest::qc::mod_provider::QcValueType as View;
        use qa_guest::qc::program::QcValueType as Program;
        self.0.global_named(name).map(|definition| match definition.value_type {
            Program::Void => View::Void,
            Program::String => View::String,
            Program::Float => View::Float,
            Program::Vector => View::Vector,
            Program::Entity => View::Entity,
            Program::Field => View::Field,
            Program::Function => View::Function,
            Program::Pointer => View::Pointer,
            Program::Opaque => View::Opaque,
        })
    }

    fn function_named(&self, name: &str) -> Option<QcFunctionView> {
        self.0.function_named(name).ok().map(|function| QcFunctionView {
            index: function.index as i32,
            name: function.name.clone(),
            first_statement: function.first_statement,
            parameter_start: function.parameter_start as i32,
            parameter_sizes: function.parameter_sizes.iter().map(|size| i32::from(*size)).collect(),
            named_builtin: function.named_builtin,
        })
    }

    fn function_at(&self, index: i32) -> Option<QcFunctionView> {
        let index = usize::try_from(index).ok()?;
        self.0.function_at(index).ok().map(|function| QcFunctionView {
            index: function.index as i32,
            name: function.name.clone(),
            first_statement: function.first_statement,
            parameter_start: function.parameter_start as i32,
            parameter_sizes: function.parameter_sizes.iter().map(|size| i32::from(*size)).collect(),
            named_builtin: function.named_builtin,
        })
    }

    fn functions(&self) -> Vec<QcFunctionView> {
        self.0
            .functions
            .iter()
            .map(|function| QcFunctionView {
                index: function.index as i32,
                name: function.name.clone(),
                first_statement: function.first_statement,
                parameter_start: function.parameter_start as i32,
                parameter_sizes: function.parameter_sizes.iter().map(|size| i32::from(*size)).collect(),
                named_builtin: function.named_builtin,
            })
            .collect()
    }
}

/// Mirror of `QuakeCWeaponBehaviorCheckpoint` from the donor module.
#[derive(Debug, Clone, PartialEq)]
pub struct QuakeCWeaponBehaviorCheckpoint {
    /// Checkpoint version.
    pub version: i64,
    /// Behavior definition.
    pub definition: WeaponBehaviorDefinition,
    /// Simulation mode.
    pub mode: SimulationMode,
    /// Source time in seconds.
    pub source_time: f64,
    /// Random state.
    pub random: SaveRandomState,
    /// Machine snapshot.
    pub machine: QcMachineSnapshot,
    /// Free slots.
    pub free: Vec<u32>,
    /// Retired slots.
    pub retired: Vec<u32>,
    /// Actor bindings.
    pub bindings: Vec<QuakeCWeaponBinding>,
}

/// Saved actor binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuakeCWeaponBinding {
    /// Entity slot.
    pub slot: u32,
    /// Saved actor.
    pub actor: SavedActorId,
    /// Binding kind.
    pub kind: QuakeCWeaponBindingKind,
}

/// Saved binding kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuakeCWeaponBindingKind {
    /// Mirrored target.
    Target,
    /// Owned projectile.
    Projectile,
}

fn save_random_state(checkpoint: &RandomCheckpoint) -> SaveRandomState {
    match checkpoint {
        RandomCheckpoint::Glibc(state) => SaveRandomState::GlibcRandom {
            words: state.words.iter().map(|word| i64::from(*word)).collect(),
            front: state.front as i64,
            rear: state.rear as i64,
            draws: state.draws,
        },
        RandomCheckpoint::Mt19937(state) => SaveRandomState::RereleaseMt19937 {
            words: state.words.to_vec(),
            index: state.index as u32,
            draws: state.draws,
        },
    }
}

fn program_digest(program: &QcProgram) -> String {
    format!("{}:{}", program.digest.algorithm, program.digest.value)
}

struct Shared {
    scene: Rc<dyn QcWeaponScene>,
    mode: SimulationMode,
    targets: QcWeaponTargets,
    aim: QcWeaponAim,
    model: QcWeaponModelLookup,
    print: QcWeaponPrint,
    field_offsets: HashMap<String, usize>,
    random: Rc<RefCell<SourceRandom>>,
    slots: HashMap<ActorId, u32>,
    actors: HashMap<u32, ActorId>,
    free: BTreeSet<u32>,
    projectiles: HashSet<u32>,
    retired: BTreeSet<u32>,
    bound: HashSet<u32>,
    source_time: f64,
    spawned: Option<Vec<u32>>,
    launching: Option<WeaponBehaviorLaunch>,
}

/// Executes donor bytecode. Only its projectile trajectory crosses into the
/// selected launcher's actor.
pub struct QuakeCWeaponBehaviorSource {
    definition: WeaponBehaviorDefinition,
    machine: Rc<RefCell<QcMachine>>,
    shared: Rc<RefCell<Shared>>,
}

impl std::fmt::Debug for QuakeCWeaponBehaviorSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QuakeCWeaponBehaviorSource")
            .field("definition", &self.definition)
            .finish_non_exhaustive()
    }
}

impl QuakeCWeaponBehaviorSource {
    /// Create a behavior source, validating the definition against the program.
    pub fn new(options: QcWeaponBehaviorOptions) -> Result<Self, QuakeCWeaponBehaviorError> {
        if let Some(capability) = qc_weapon_behavior_capability_error(&ProgramView(&options.program)) {
            return Err(QuakeCWeaponBehaviorError::Capability(capability));
        }
        let definition = options.definition;
        if definition.module.digest != program_digest(&options.program)
            || !matches!(definition.fire, WeaponBehaviorCallback::QuakeC { .. })
            || (definition.activate.is_some()
                && !matches!(definition.activate, Some(WeaponBehaviorCallback::QuakeC { .. })))
        {
            return Err(QuakeCWeaponBehaviorError::ArtifactMismatch);
        }
        let mut callbacks = vec![&definition.fire];
        if let Some(activate) = &definition.activate {
            callbacks.push(activate);
        }
        for callback in callbacks {
            let WeaponBehaviorCallback::QuakeC { module, function_index } = callback else {
                return Err(QuakeCWeaponBehaviorError::NotQuakeC);
            };
            if !module.same_module(&definition.module) || *function_index == 0 {
                return Err(QuakeCWeaponBehaviorError::CallbackMismatch);
            }
            match options.program.functions.get(*function_index) {
                Some(function) if function.first_statement >= 0 && function.parameter_sizes.is_empty() => {}
                _ => return Err(QuakeCWeaponBehaviorError::CallbackMismatch),
            }
        }
        let mut field_offsets = HashMap::new();
        for name in [
            "origin",
            "velocity",
            "angles",
            "mins",
            "maxs",
            "health",
            "solid",
            "classname",
            "netname",
            "v_angle",
            "nextthink",
            "think",
            "chain",
            "model",
            "modelindex",
        ] {
            if let Some(definition) = options.program.field_named(name) {
                field_offsets.insert(name.to_string(), definition.offset);
            }
        }
        let random: Rc<RefCell<SourceRandom>> = Rc::new(RefCell::new(options.random));
        let shared = Rc::new(RefCell::new(Shared {
            scene: options.scene,
            mode: options.mode,
            targets: options.targets,
            aim: options.aim,
            model: options.model,
            print: options.print,
            field_offsets,
            random: Rc::clone(&random),
            slots: HashMap::new(),
            actors: HashMap::new(),
            free: BTreeSet::new(),
            projectiles: HashSet::new(),
            retired: BTreeSet::new(),
            bound: HashSet::new(),
            source_time: 0.0,
            spawned: None,
            launching: None,
        }));
        let host = builtin_host(Rc::downgrade(&shared));
        let free_entities = Rc::downgrade(&shared);
        let entities = QcEntityMemory::new(classic_qc_entity_layout(&options.program), 8192, 1)?;
        let numeric = NumericOps::select(Q1_DONOR_PROFILE).map_err(|_| QuakeCWeaponBehaviorError::ArtifactMismatch)?;
        let kind = if options.program.api.is_quakeworld() {
            QcHostKind::Quakeworld
        } else {
            QcHostKind::Netquake
        };
        let builtins = create_qc_builtins(QcBuiltinServices {
            kind,
            random: Some(random as QcSharedRandom),
            is_free_entity: Some(Rc::new(move |slot| {
                free_entities
                    .upgrade()
                    .is_some_and(|shared| shared.borrow().free.contains(&slot))
            })),
            prepare_entities: None,
            host: Some(host),
            extensions: HashSet::new(),
        });
        let machine = QcMachine::new(QcMachineOptions::new(
            options.program,
            numeric,
            entities,
            builtins,
            Rc::new(|| true),
        ))?;
        let machine = Rc::new(RefCell::new(machine));
        let source = Self {
            definition,
            machine,
            shared,
        };
        source.set_global(
            "deathmatch",
            if source.shared.borrow().mode == SimulationMode::Deathmatch {
                1.0
            } else {
                0.0
            },
        )?;
        let coop = source.shared.borrow().mode == SimulationMode::Coop;
        source.set_global("coop", if coop { 1.0 } else { 0.0 })?;
        Ok(source)
    }

    fn set_global(&self, name: &str, value: f64) -> Result<(), QuakeCWeaponBehaviorError> {
        let machine = self.machine.borrow();
        if let Ok(offset) = machine.global_offset(name) {
            drop(machine);
            self.machine
                .borrow_mut()
                .globals_mut()
                .set_float(offset, value as f32)?;
        }
        Ok(())
    }
}
fn field_offset(shared: &Shared, name: &str) -> Result<usize, QuakeCWeaponBehaviorError> {
    shared
        .field_offsets
        .get(name)
        .copied()
        .ok_or_else(|| QuakeCWeaponBehaviorError::MissingField(name.to_string()))
}

fn set_machine_global(machine: &mut QcMachine, name: &str, value: f64) -> Result<(), QuakeCWeaponBehaviorError> {
    if let Ok(offset) = machine.global_offset(name) {
        machine.globals_mut().set_float(offset, value as f32)?;
    }
    Ok(())
}

fn allocate_slot(machine: &mut QcMachine, shared: &mut Shared) -> Result<u32, QuakeCWeaponBehaviorError> {
    if let Some(slot) = shared.free.iter().next().copied() {
        shared.free.remove(&slot);
        machine.entities_mut().clear_slot(slot)?;
        return Ok(slot);
    }
    let slot = machine.entities().count() as u32;
    machine.entities_mut().set_count(slot as usize + 1)?;
    Ok(slot)
}

fn reference_slot(
    machine: &mut QcMachine,
    shared: &mut Shared,
    actor: &ActorId,
) -> Result<i32, QuakeCWeaponBehaviorError> {
    for slot in shared.projectiles.iter().copied().collect::<Vec<_>>() {
        if shared.actors.get(&slot) == Some(actor) {
            return Ok(machine.entities().reference(slot)?);
        }
    }
    let slot = match shared.slots.get(actor).copied() {
        Some(slot) => slot,
        None => {
            let slot = allocate_slot(machine, shared)?;
            shared.slots.insert(actor.clone(), slot);
            shared.actors.insert(slot, actor.clone());
            slot
        }
    };
    Ok(machine.entities().reference(slot)?)
}

fn project_body(
    machine: &mut QcMachine,
    shared: &Shared,
    slot: u32,
    body: &BodyState,
) -> Result<(), QuakeCWeaponBehaviorError> {
    let entities = machine.entities_mut();
    entities.set_slot_vector(slot, field_offset(shared, "origin")?, body.origin)?;
    entities.set_slot_vector(slot, field_offset(shared, "velocity")?, body.velocity)?;
    entities.set_slot_vector(slot, field_offset(shared, "angles")?, body.angles)?;
    entities.set_slot_vector(slot, field_offset(shared, "mins")?, body.bounds.min)?;
    entities.set_slot_vector(slot, field_offset(shared, "maxs")?, body.bounds.max)?;
    Ok(())
}

fn refresh_targets(
    machine: &mut QcMachine,
    shared: &mut Shared,
    excluded: Option<&ActorId>,
) -> Result<(), QuakeCWeaponBehaviorError> {
    let targets = (shared.targets)();
    let mut live = HashSet::new();
    for target in &targets {
        if Some(&target.actor) == excluded {
            continue;
        }
        live.insert(target.actor.clone());
        let reference = reference_slot(machine, shared, &target.actor)?;
        let slot = machine.entities().slot(reference)?;
        if shared.projectiles.contains(&slot) {
            continue;
        }
        project_body(machine, shared, slot, &target.body)?;
        let health = field_offset(shared, "health")?;
        let solid = field_offset(shared, "solid")?;
        let classname = field_offset(shared, "classname")?;
        let netname = field_offset(shared, "netname")?;
        machine
            .entities_mut()
            .set_slot_float(slot, health, target.health as f32)?;
        machine
            .entities_mut()
            .set_slot_float(slot, solid, if target.solid { 2.0 } else { 0.0 })?;
        let classname_ref = machine.strings_mut().allocate(&target.classname)?;
        machine.entities_mut().set_slot_int(slot, classname, classname_ref)?;
        let netname_ref = machine.strings_mut().allocate(&target.name)?;
        machine.entities_mut().set_slot_int(slot, netname, netname_ref)?;
    }
    let stale: Vec<u32> = shared
        .slots
        .iter()
        .filter(|(actor, _)| !live.contains(*actor))
        .map(|(_, slot)| *slot)
        .collect();
    for slot in stale {
        let health = field_offset(shared, "health")?;
        let solid = field_offset(shared, "solid")?;
        machine.entities_mut().set_slot_float(slot, health, 0.0)?;
        machine.entities_mut().set_slot_float(slot, solid, 0.0)?;
    }
    Ok(())
}

fn invoke_callback(
    machine: &mut QcMachine,
    function_index: usize,
    slot: u32,
    time: f64,
) -> Result<(), QuakeCWeaponBehaviorError> {
    let reference = machine.entities().reference(slot)?;
    let self_offset = machine.global_offset("self")?;
    let other_offset = machine.global_offset("other")?;
    let prior_self = machine.globals().int(self_offset)?;
    let prior_other = machine.globals().int(other_offset)?;
    machine.globals_mut().set_int(self_offset, reference)?;
    machine.globals_mut().set_int(other_offset, 0)?;
    set_machine_global(machine, "time", time)?;
    let outcome = machine.execute(function_index, 0);
    machine.globals_mut().set_int(self_offset, prior_self)?;
    machine.globals_mut().set_int(other_offset, prior_other)?;
    outcome?;
    Ok(())
}

fn upgrade(shared: &Weak<RefCell<Shared>>, machine: &QcMachine) -> Result<Rc<RefCell<Shared>>, GuestError> {
    shared
        .upgrade()
        .ok_or_else(|| machine.fail("QuakeC weapon behavior source was released"))
}

fn builtin_host(shared: Weak<RefCell<Shared>>) -> HashMap<QcHostBuiltinName, QcBuiltin> {
    let mut host: HashMap<QcHostBuiltinName, QcBuiltin> = HashMap::new();
    {
        let shared = Weak::clone(&shared);
        host.insert(
            QcHostBuiltinName::Spawn,
            Rc::new(move |machine: &mut QcMachine| {
                spawn_builtin(&shared, machine).map_err(|error| machine.fail(error.to_string()))
            }),
        );
    }
    {
        let shared = Weak::clone(&shared);
        host.insert(
            QcHostBuiltinName::Remove,
            Rc::new(move |machine: &mut QcMachine| {
                remove_builtin(&shared, machine).map_err(|error| machine.fail(error.to_string()))
            }),
        );
    }
    {
        let shared = Weak::clone(&shared);
        host.insert(
            QcHostBuiltinName::Setorigin,
            Rc::new(move |machine: &mut QcMachine| {
                setorigin_builtin(&shared, machine).map_err(|error| machine.fail(error.to_string()))
            }),
        );
    }
    {
        let shared = Weak::clone(&shared);
        host.insert(
            QcHostBuiltinName::Setsize,
            Rc::new(move |machine: &mut QcMachine| {
                setsize_builtin(&shared, machine).map_err(|error| machine.fail(error.to_string()))
            }),
        );
    }
    {
        let shared = Weak::clone(&shared);
        host.insert(
            QcHostBuiltinName::Setmodel,
            Rc::new(move |machine: &mut QcMachine| {
                setmodel_builtin(&shared, machine).map_err(|error| machine.fail(error.to_string()))
            }),
        );
    }
    host.insert(
        QcHostBuiltinName::Sound,
        // Presentation belongs to the selected launcher, so captured donor
        // sound does not publish a second weapon sound.
        Rc::new(|_machine: &mut QcMachine| Ok(())),
    );
    {
        let shared = Weak::clone(&shared);
        host.insert(
            QcHostBuiltinName::Sprint,
            Rc::new(move |machine: &mut QcMachine| {
                sprint_builtin(&shared, machine).map_err(|error| machine.fail(error.to_string()))
            }),
        );
    }
    {
        let shared = Weak::clone(&shared);
        host.insert(
            QcHostBuiltinName::Bprint,
            Rc::new(move |machine: &mut QcMachine| {
                broadcast_builtin(&shared, machine, 0).map_err(|error| machine.fail(error.to_string()))
            }),
        );
    }
    {
        let shared = Weak::clone(&shared);
        host.insert(
            QcHostBuiltinName::Dprint,
            Rc::new(move |machine: &mut QcMachine| {
                broadcast_builtin(&shared, machine, 0).map_err(|error| machine.fail(error.to_string()))
            }),
        );
    }
    {
        let shared = Weak::clone(&shared);
        host.insert(
            QcHostBuiltinName::Aim,
            Rc::new(move |machine: &mut QcMachine| {
                aim_builtin(&shared, machine).map_err(|error| machine.fail(error.to_string()))
            }),
        );
    }
    {
        let shared = Weak::clone(&shared);
        host.insert(
            QcHostBuiltinName::Pointcontents,
            Rc::new(move |machine: &mut QcMachine| {
                pointcontents_builtin(&shared, machine).map_err(|error| machine.fail(error.to_string()))
            }),
        );
    }
    {
        let shared = Weak::clone(&shared);
        host.insert(
            QcHostBuiltinName::Traceline,
            Rc::new(move |machine: &mut QcMachine| {
                traceline_builtin(&shared, machine).map_err(|error| machine.fail(error.to_string()))
            }),
        );
    }
    {
        let shared = Weak::clone(&shared);
        host.insert(
            QcHostBuiltinName::Findradius,
            Rc::new(move |machine: &mut QcMachine| {
                findradius_builtin(&shared, machine).map_err(|error| machine.fail(error.to_string()))
            }),
        );
    }
    host
}

fn spawn_builtin(shared: &Weak<RefCell<Shared>>, machine: &mut QcMachine) -> Result<(), QuakeCWeaponBehaviorError> {
    let shared = upgrade(shared, machine)?;
    let mut shared = shared.borrow_mut();
    if shared.spawned.is_none() {
        return Err(QuakeCWeaponBehaviorError::SpawnUndeclared);
    }
    let slot = allocate_slot(machine, &mut shared)?;
    if let Some(spawned) = shared.spawned.as_mut() {
        spawned.push(slot);
    }
    let reference = machine.entities().reference(slot)?;
    machine.return_int(reference)?;
    Ok(())
}

fn remove_builtin(shared: &Weak<RefCell<Shared>>, machine: &mut QcMachine) -> Result<(), QuakeCWeaponBehaviorError> {
    let target = machine.arg_int(0)?;
    let slot = machine.entities().slot(target)?;
    let shared = upgrade(shared, machine)?;
    let mut shared = shared.borrow_mut();
    let spawned = shared.spawned.as_ref().is_some_and(|spawned| spawned.contains(&slot));
    if !shared.projectiles.contains(&slot) && !spawned {
        return Err(QuakeCWeaponBehaviorError::RemoveForeign);
    }
    shared.retired.insert(slot);
    Ok(())
}

fn setorigin_builtin(shared: &Weak<RefCell<Shared>>, machine: &mut QcMachine) -> Result<(), QuakeCWeaponBehaviorError> {
    let target = machine.arg_int(0)?;
    let origin = machine.arg_vector(1)?;
    let slot = machine.entities().slot(target)?;
    let shared = upgrade(shared, machine)?;
    let shared = shared.borrow();
    let field = field_offset(&shared, "origin")?;
    machine.entities_mut().set_slot_vector(slot, field, origin)?;
    Ok(())
}

fn setsize_builtin(shared: &Weak<RefCell<Shared>>, machine: &mut QcMachine) -> Result<(), QuakeCWeaponBehaviorError> {
    let target = machine.arg_int(0)?;
    let mins = machine.arg_vector(1)?;
    let maxs = machine.arg_vector(2)?;
    let slot = machine.entities().slot(target)?;
    let shared = upgrade(shared, machine)?;
    let shared = shared.borrow();
    let mins_field = field_offset(&shared, "mins")?;
    let maxs_field = field_offset(&shared, "maxs")?;
    machine.entities_mut().set_slot_vector(slot, mins_field, mins)?;
    machine.entities_mut().set_slot_vector(slot, maxs_field, maxs)?;
    Ok(())
}

fn setmodel_builtin(shared: &Weak<RefCell<Shared>>, machine: &mut QcMachine) -> Result<(), QuakeCWeaponBehaviorError> {
    let target = machine.arg_int(0)?;
    let name = machine.arg_string(1)?;
    let index_word = machine.arg_int(1)?;
    let slot = machine.entities().slot(target)?;
    let shared = upgrade(shared, machine)?;
    let shared = shared.borrow();
    let model = (shared.model)(&name).ok_or_else(|| QuakeCWeaponBehaviorError::NoSourceModel(name.clone()))?;
    let model_field = field_offset(&shared, "model")?;
    let modelindex_field = field_offset(&shared, "modelindex")?;
    let mins_field = field_offset(&shared, "mins")?;
    let maxs_field = field_offset(&shared, "maxs")?;
    machine.entities_mut().set_slot_int(slot, model_field, index_word)?;
    machine
        .entities_mut()
        .set_slot_float(slot, modelindex_field, model.index as f32)?;
    machine
        .entities_mut()
        .set_slot_vector(slot, mins_field, model.bounds.min)?;
    machine
        .entities_mut()
        .set_slot_vector(slot, maxs_field, model.bounds.max)?;
    Ok(())
}

fn sprint_builtin(shared: &Weak<RefCell<Shared>>, machine: &mut QcMachine) -> Result<(), QuakeCWeaponBehaviorError> {
    let target = machine.arg_int(0)?;
    let slot = machine.entities().slot(target)?;
    let text = machine.var_string(1)?;
    let shared = upgrade(shared, machine)?;
    let shared = shared.borrow();
    let recipient = shared.actors.get(&slot).cloned();
    (shared.print)(recipient.as_ref(), &text);
    Ok(())
}

fn broadcast_builtin(
    shared: &Weak<RefCell<Shared>>,
    machine: &mut QcMachine,
    first: usize,
) -> Result<(), QuakeCWeaponBehaviorError> {
    let text = machine.var_string(first)?;
    let shared = upgrade(shared, machine)?;
    let shared = shared.borrow();
    (shared.print)(None, &text);
    Ok(())
}

fn aim_builtin(shared: &Weak<RefCell<Shared>>, machine: &mut QcMachine) -> Result<(), QuakeCWeaponBehaviorError> {
    let target = machine.arg_int(0)?;
    let speed = f64::from(machine.arg_float(1)?);
    let slot = machine.entities().slot(target)?;
    let shared = upgrade(shared, machine)?;
    let shared = shared.borrow();
    let actor = shared
        .actors
        .get(&slot)
        .cloned()
        .ok_or(QuakeCWeaponBehaviorError::AimShooter)?;
    let vector = (shared.aim)(&actor, speed);
    machine.return_vector(vector)?;
    Ok(())
}

fn pointcontents_builtin(
    shared: &Weak<RefCell<Shared>>,
    machine: &mut QcMachine,
) -> Result<(), QuakeCWeaponBehaviorError> {
    let point = machine.arg_vector(0)?;
    let shared = upgrade(shared, machine)?;
    let shared = shared.borrow();
    let contents = shared.scene.point_contents_q1(point);
    machine.return_float(contents as f32)?;
    Ok(())
}

fn traceline_builtin(shared: &Weak<RefCell<Shared>>, machine: &mut QcMachine) -> Result<(), QuakeCWeaponBehaviorError> {
    let start = machine.arg_vector(0)?;
    let end = machine.arg_vector(1)?;
    let mode = f64::from(machine.arg_float(2)?).trunc() as i32;
    let entity = machine.arg_int(3)?;
    let movement = if mode == 1 {
        Q1Move::NoMonsters
    } else if mode == 2 {
        Q1Move::Missile
    } else {
        Q1Move::Normal
    };
    let shared = upgrade(shared, machine)?;
    let mut shared = shared.borrow_mut();
    let slot = machine.entities().slot(entity)?;
    let actor = shared
        .actors
        .get(&slot)
        .cloned()
        .or_else(|| shared.launching.as_ref().map(|launch| launch.projectile.id().clone()));
    let trace = shared.scene.trace_q1_point(start, end, movement, actor);
    set_machine_global(machine, "trace_fraction", trace.fraction)?;
    set_machine_global(machine, "trace_allsolid", f64::from(u8::from(trace.all_solid)))?;
    set_machine_global(machine, "trace_startsolid", f64::from(u8::from(trace.start_solid)))?;
    set_machine_global(machine, "trace_inwater", f64::from(u8::from(trace.in_water)))?;
    set_machine_global(machine, "trace_inopen", f64::from(u8::from(trace.in_open)))?;
    set_machine_global(machine, "trace_plane_dist", trace.plane_distance)?;
    let endpos = machine.global_offset("trace_endpos")?;
    machine.globals_mut().set_vector(endpos, trace.end)?;
    let normal = machine.global_offset("trace_plane_normal")?;
    machine.globals_mut().set_vector(normal, trace.plane_normal)?;
    let ent = match trace.hit {
        TraceHit::Actor { ref actor } => reference_slot(machine, &mut shared, actor)?,
        _ => 0,
    };
    let ent_offset = machine.global_offset("trace_ent")?;
    machine.globals_mut().set_int(ent_offset, ent)?;
    Ok(())
}

fn findradius_builtin(
    shared: &Weak<RefCell<Shared>>,
    machine: &mut QcMachine,
) -> Result<(), QuakeCWeaponBehaviorError> {
    let center = machine.arg_vector(0)?;
    let radius = f64::from(machine.arg_float(1)?);
    let numeric = machine.numeric();
    let shared = upgrade(shared, machine)?;
    let shared = shared.borrow();
    let origin_field = field_offset(&shared, "origin")?;
    let mins_field = field_offset(&shared, "mins")?;
    let maxs_field = field_offset(&shared, "maxs")?;
    let solid_field = field_offset(&shared, "solid")?;
    let chain_field = field_offset(&shared, "chain")?;
    let count = machine.entities().count();
    let mut chain = 0;
    for slot in 1..count as u32 {
        if shared.free.contains(&slot) || shared.retired.contains(&slot) {
            continue;
        }
        if machine.entities().slot_float(slot, solid_field)? == 0.0 {
            continue;
        }
        let origin = machine.entities().slot_vector(slot, origin_field)?;
        let mins = machine.entities().slot_vector(slot, mins_field)?;
        let maxs = machine.entities().slot_vector(slot, maxs_field)?;
        let distance = |center: f32, origin: f32, min: f32, max: f32| {
            numeric.sub(
                f64::from(center),
                numeric.add(
                    f64::from(origin),
                    numeric.mul(numeric.add(f64::from(min), f64::from(max)), 0.5),
                ),
            )
        };
        let x = distance(center.x, origin.x, mins.x, maxs.x);
        let y = distance(center.y, origin.y, mins.y, maxs.y);
        let z = distance(center.z, origin.z, mins.z, maxs.z);
        let length = numeric.sqrt(numeric.add(numeric.add(numeric.mul(x, x), numeric.mul(y, y)), numeric.mul(z, z)));
        if length > radius {
            continue;
        }
        machine.entities_mut().set_slot_int(slot, chain_field, chain)?;
        chain = machine.entities().reference(slot)?;
    }
    machine.return_int(chain)?;
    Ok(())
}
/// Attached QuakeC projectile trajectory.
pub struct QcWeaponInstance {
    definition: WeaponBehaviorDefinition,
    initial: WeaponTrajectoryUpdate,
    slot: u32,
    machine: Rc<RefCell<QcMachine>>,
    shared: Rc<RefCell<Shared>>,
    closed: Cell<bool>,
}

impl std::fmt::Debug for QcWeaponInstance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QcWeaponInstance")
            .field("definition", &self.definition)
            .field("initial", &self.initial)
            .field("slot", &self.slot)
            .finish_non_exhaustive()
    }
}

impl WeaponBehaviorInstance for QcWeaponInstance {
    type Error = QuakeCWeaponBehaviorError;

    fn definition(&self) -> &WeaponBehaviorDefinition {
        &self.definition
    }

    fn initial(&self) -> &WeaponTrajectoryUpdate {
        &self.initial
    }

    fn step(
        &self,
        body: &BodyState,
        time_seconds: f64,
    ) -> Result<Option<WeaponTrajectoryUpdate>, QuakeCWeaponBehaviorError> {
        if self.closed.get() || self.shared.borrow().retired.contains(&self.slot) {
            return Ok(None);
        }
        self.shared.borrow_mut().source_time = time_seconds;
        {
            let mut machine = self.machine.borrow_mut();
            let mut shared = self.shared.borrow_mut();
            refresh_targets(&mut machine, &mut shared, None)?;
            project_body(&mut machine, &shared, self.slot, body)?;
        }
        let nextthink = {
            let machine = self.machine.borrow();
            let shared = self.shared.borrow();
            machine
                .entities()
                .slot_float(self.slot, field_offset(&shared, "nextthink")?)?
        };
        if nextthink <= 0.0 || f64::from(nextthink) > time_seconds {
            return Ok(None);
        }
        let think = {
            let machine = self.machine.borrow();
            let shared = self.shared.borrow();
            machine
                .entities()
                .slot_int(self.slot, field_offset(&shared, "think")?)?
        };
        if think == 0 {
            return Ok(None);
        }
        {
            let shared = self.shared.borrow();
            let nextthink = field_offset(&shared, "nextthink")?;
            self.machine
                .borrow_mut()
                .entities_mut()
                .set_slot_float(self.slot, nextthink, 0.0)?;
        }
        invoke_callback(&mut self.machine.borrow_mut(), think as usize, self.slot, time_seconds)?;
        if self.shared.borrow().retired.contains(&self.slot) {
            return Ok(None);
        }
        let machine = self.machine.borrow();
        let shared = self.shared.borrow();
        Ok(Some(WeaponTrajectoryUpdate {
            origin: machine
                .entities()
                .slot_vector(self.slot, field_offset(&shared, "origin")?)?,
            velocity: machine
                .entities()
                .slot_vector(self.slot, field_offset(&shared, "velocity")?)?,
            angles: machine
                .entities()
                .slot_vector(self.slot, field_offset(&shared, "angles")?)?,
        }))
    }

    fn close(&self) {
        if self.closed.get() {
            return;
        }
        self.closed.set(true);
        let mut shared = self.shared.borrow_mut();
        shared.bound.remove(&self.slot);
        shared.retired.remove(&self.slot);
        shared.free.insert(self.slot);
        shared.actors.remove(&self.slot);
        shared.projectiles.remove(&self.slot);
    }
}

impl WeaponBehaviorSource for QuakeCWeaponBehaviorSource {
    type Error = QuakeCWeaponBehaviorError;
    type Instance = QcWeaponInstance;

    fn definition(&self) -> &WeaponBehaviorDefinition {
        &self.definition
    }

    fn attach(&self, launch: WeaponBehaviorLaunch) -> Result<Option<QcWeaponInstance>, QuakeCWeaponBehaviorError> {
        if launch.role != self.definition.role {
            return Err(QuakeCWeaponBehaviorError::RoleMismatch);
        }
        if self.shared.borrow().launching.is_some() {
            return Err(QuakeCWeaponBehaviorError::LaunchActive);
        }
        self.shared.borrow_mut().source_time = launch.time_seconds;
        {
            let mut machine = self.machine.borrow_mut();
            let mut shared = self.shared.borrow_mut();
            refresh_targets(&mut machine, &mut shared, Some(&launch.projectile.id().clone()))?;
        }
        let shooter = {
            let mut machine = self.machine.borrow_mut();
            let mut shared = self.shared.borrow_mut();
            let reference = reference_slot(&mut machine, &mut shared, &launch.shooter)?;
            machine.entities().slot(reference)?
        };
        {
            let machine = self.machine.borrow();
            let shared = self.shared.borrow();
            let angles_field = field_offset(&shared, "angles")?;
            let v_angle_field = field_offset(&shared, "v_angle")?;
            let angles = machine.entities().slot_vector(shooter, angles_field)?;
            drop(machine);
            self.machine
                .borrow_mut()
                .entities_mut()
                .set_slot_vector(shooter, v_angle_field, angles)?;
        }
        let WeaponBehaviorCallback::QuakeC {
            function_index: fire, ..
        } = self.definition.fire
        else {
            return Err(QuakeCWeaponBehaviorError::NotQuakeC);
        };
        let activate = match &self.definition.activate {
            None => None,
            Some(WeaponBehaviorCallback::QuakeC { function_index, .. }) => Some(*function_index),
            Some(_) => return Err(QuakeCWeaponBehaviorError::NotQuakeC),
        };
        self.shared.borrow_mut().launching = Some(launch.clone());
        self.shared.borrow_mut().spawned = Some(Vec::new());
        let outcome = (|| -> Result<(), QuakeCWeaponBehaviorError> {
            if let Some(activate) = activate {
                invoke_callback(&mut self.machine.borrow_mut(), activate, shooter, launch.time_seconds)?;
            }
            invoke_callback(&mut self.machine.borrow_mut(), fire, shooter, launch.time_seconds)?;
            Ok(())
        })();
        let spawned = self.shared.borrow_mut().spawned.take().unwrap_or_default();
        self.shared.borrow_mut().launching = None;
        if let Err(error) = outcome {
            let mut shared = self.shared.borrow_mut();
            for slot in spawned {
                shared.retired.remove(&slot);
                shared.free.insert(slot);
            }
            return Err(error);
        }
        if spawned.is_empty() {
            return Ok(None);
        }
        if spawned.len() != 1 {
            let mut shared = self.shared.borrow_mut();
            for slot in spawned {
                shared.retired.remove(&slot);
                shared.free.insert(slot);
            }
            return Err(QuakeCWeaponBehaviorError::MultipleActors);
        }
        let slot = spawned[0];
        {
            let mut shared = self.shared.borrow_mut();
            shared.actors.insert(slot, launch.projectile.id().clone());
            shared.projectiles.insert(slot);
        }
        Ok(Some(self.instance(slot)?))
    }

    fn resume(&self, projectile: &ActorId) -> Result<QcWeaponInstance, QuakeCWeaponBehaviorError> {
        let slot = {
            let shared = self.shared.borrow();
            shared
                .projectiles
                .iter()
                .find(|slot| shared.actors.get(*slot) == Some(projectile))
                .copied()
        };
        match slot {
            Some(slot) => self.instance(slot),
            None => Err(QuakeCWeaponBehaviorError::MissingRestored),
        }
    }
}

impl QuakeCWeaponBehaviorSource {
    fn instance(&self, slot: u32) -> Result<QcWeaponInstance, QuakeCWeaponBehaviorError> {
        if self.shared.borrow().bound.contains(&slot) {
            return Err(QuakeCWeaponBehaviorError::AlreadyAttached);
        }
        self.shared.borrow_mut().bound.insert(slot);
        let machine = self.machine.borrow();
        let shared = self.shared.borrow();
        let initial = WeaponTrajectoryUpdate {
            origin: machine.entities().slot_vector(slot, field_offset(&shared, "origin")?)?,
            velocity: machine
                .entities()
                .slot_vector(slot, field_offset(&shared, "velocity")?)?,
            angles: machine.entities().slot_vector(slot, field_offset(&shared, "angles")?)?,
        };
        Ok(QcWeaponInstance {
            definition: self.definition.clone(),
            initial,
            slot,
            machine: Rc::clone(&self.machine),
            shared: Rc::clone(&self.shared),
            closed: Cell::new(false),
        })
    }

    /// Capture the source checkpoint.
    pub fn checkpoint(&self) -> Result<QuakeCWeaponBehaviorCheckpoint, QuakeCWeaponBehaviorError> {
        let shared = self.shared.borrow();
        if shared.launching.is_some() {
            return Err(QuakeCWeaponBehaviorError::SaveDuringLaunch);
        }
        let machine = self.machine.borrow_mut().snapshot()?;
        let random = save_random_state(&shared.random.borrow().checkpoint());
        let free = shared.free.iter().copied().collect();
        let retired = shared.retired.iter().copied().collect();
        let bindings = shared
            .actors
            .iter()
            .map(|(slot, actor)| QuakeCWeaponBinding {
                slot: *slot,
                actor: SavedActorId::from(actor),
                kind: if shared.projectiles.contains(slot) {
                    QuakeCWeaponBindingKind::Projectile
                } else {
                    QuakeCWeaponBindingKind::Target
                },
            })
            .collect();
        Ok(QuakeCWeaponBehaviorCheckpoint {
            version: 1,
            definition: self.definition.clone(),
            mode: shared.mode,
            source_time: shared.source_time,
            random,
            machine,
            free,
            retired,
            bindings,
        })
    }

    /// Restore a checkpoint. The caller restores `random` first; the source
    /// verifies it matches the checkpoint before adopting it.
    pub fn restore(
        &self,
        checkpoint: &QuakeCWeaponBehaviorCheckpoint,
        actor_reference: &dyn Fn(&SavedActorId) -> ActorId,
        random: SourceRandom,
    ) -> Result<(), QuakeCWeaponBehaviorError> {
        {
            let shared = self.shared.borrow();
            if checkpoint.version != 1
                || !same_weapon_behavior(&checkpoint.definition, &self.definition)
                || checkpoint.mode != shared.mode
                || !checkpoint.source_time.is_finite()
                || !shared.actors.is_empty()
                || shared.launching.is_some()
            {
                return Err(QuakeCWeaponBehaviorError::BadCheckpoint);
            }
        }
        if save_random_state(&random.checkpoint()) != checkpoint.random {
            return Err(QuakeCWeaponBehaviorError::RandomMismatch);
        }
        let count = checkpoint.machine.entity_count;
        let valid_slot = |slot: u32| slot > 0 && (slot as usize) < count;
        let mut free = BTreeSet::new();
        for slot in &checkpoint.free {
            if !valid_slot(*slot) || !free.insert(*slot) {
                return Err(QuakeCWeaponBehaviorError::BadFreeSlot);
            }
        }
        let mut occupied = HashSet::new();
        let mut references = HashSet::new();
        let mut bindings = Vec::with_capacity(checkpoint.bindings.len());
        for binding in &checkpoint.bindings {
            if !valid_slot(binding.slot) || !occupied.insert(binding.slot) || free.contains(&binding.slot) {
                return Err(QuakeCWeaponBehaviorError::BadEntitySlot);
            }
            let actor = actor_reference(&binding.actor);
            if !references.insert(actor.clone()) {
                return Err(QuakeCWeaponBehaviorError::DuplicateActor);
            }
            bindings.push((binding.slot, actor, binding.kind));
        }
        if occupied.len() + free.len() != count - 1 {
            return Err(QuakeCWeaponBehaviorError::IncompleteOwnership);
        }
        let mut retired = BTreeSet::new();
        for slot in &checkpoint.retired {
            let bound = bindings
                .iter()
                .any(|(bound, _, kind)| bound == slot && *kind == QuakeCWeaponBindingKind::Projectile);
            if !retired.insert(*slot) || !bound {
                return Err(QuakeCWeaponBehaviorError::BadRetiredSlot);
            }
        }
        self.machine.borrow_mut().restore(&checkpoint.machine)?;
        {
            let shared = self.shared.borrow();
            *shared.random.borrow_mut() = random;
        }
        {
            let mut shared = self.shared.borrow_mut();
            shared.source_time = checkpoint.source_time;
            shared.free = free;
            shared.retired = retired;
            for (slot, actor, kind) in bindings {
                shared.actors.insert(slot, actor.clone());
                if kind == QuakeCWeaponBindingKind::Projectile {
                    shared.projectiles.insert(slot);
                } else {
                    shared.slots.insert(actor, slot);
                }
            }
        }
        Ok(())
    }
}

/// Read a saved behavior checkpoint.
pub fn read_quake_c_weapon_behavior_checkpoint(
    reader: &SaveReader,
    expected: &WeaponBehaviorDefinition,
    definitions: &impl WeaponBehaviorDefinitionReader,
) -> Result<QuakeCWeaponBehaviorCheckpoint, WorldError> {
    let machine = reader.field("machine");
    let definition = definitions.read_definition(reader.field("definition"), expected)?;
    let mode = match reader
        .field("mode")
        .choice_str(&["singleplayer", "coop", "deathmatch"])?
        .as_str()
    {
        "singleplayer" => SimulationMode::Singleplayer,
        "coop" => SimulationMode::Coop,
        _ => SimulationMode::Deathmatch,
    };
    let entity_count = machine.field("entityCount").integer(1)?;
    let statement = machine.field("statement").integer(i64::MIN)?;
    let argument_count = machine.field("argumentCount").integer(0)?;
    let snapshot = QcMachineSnapshot {
        globals: machine.field("globals").bytes()?,
        entities: machine.field("entities").bytes()?,
        entity_count: usize::try_from(entity_count)
            .map_err(|_| machine.field("entityCount").fail("expected an integer in range"))?,
        strings: machine.field("strings").bytes()?,
        statement: usize::try_from(statement)
            .map_err(|_| machine.field("statement").fail("expected an integer in range"))?,
        function_index: usize::try_from(machine.field("functionIndex").literal_i64(0)?)
            .map_err(|_| machine.field("functionIndex").fail("expected an integer in range"))?,
        argument_count: usize::try_from(argument_count)
            .map_err(|_| machine.field("argumentCount").fail("expected an integer in range"))?,
        profiling: machine
            .field("profiling")
            .list(|value| value.integer(0))?
            .into_iter()
            .map(|value| {
                u32::try_from(value).map_err(|_| machine.field("profiling").fail("expected an integer in range"))
            })
            .collect::<Result<Vec<_>, _>>()?,
        trace_enabled: machine.field("traceEnabled").boolean()?,
    };
    let free = reader
        .field("free")
        .list(|value| value.integer(1))?
        .into_iter()
        .map(|value| u32::try_from(value).map_err(|_| reader.field("free").fail("expected an integer in range")))
        .collect::<Result<Vec<_>, _>>()?;
    let retired = reader
        .field("retired")
        .list(|value| value.integer(1))?
        .into_iter()
        .map(|value| u32::try_from(value).map_err(|_| reader.field("retired").fail("expected an integer in range")))
        .collect::<Result<Vec<_>, _>>()?;
    let bindings = reader.field("bindings").list(|entry| {
        let slot = entry.field("slot").integer(1)?;
        let slot = u32::try_from(slot).map_err(|_| entry.field("slot").fail("expected an integer in range"))?;
        let kind = match entry.field("kind").choice_str(&["target", "projectile"])?.as_str() {
            "target" => QuakeCWeaponBindingKind::Target,
            _ => QuakeCWeaponBindingKind::Projectile,
        };
        Ok(QuakeCWeaponBinding {
            slot,
            actor: read_saved_actor(entry.field("actor"))?,
            kind,
        })
    })?;
    Ok(QuakeCWeaponBehaviorCheckpoint {
        version: reader.field("version").literal_i64(1)?,
        definition,
        mode,
        source_time: reader.field("sourceTime").finite()?,
        random: read_random(reader.field("random"))?,
        machine: snapshot,
        free,
        retired,
        bindings,
    })
}
/// Capture a behavior definition in donor save shape. Shared by the QVM
/// weapon behavior checkpoint, which embeds the same declaration.
#[must_use]
pub fn capture_weapon_behavior_definition(definition: &WeaponBehaviorDefinition) -> SaveJson {
    use qa_world::save::value::SaveJson;
    use qa_world::save::value::{int, obj, str as json_str};

    use qa_guest::qvm::mod_provider::ModuleId;

    fn capture_module(module: &ModuleId) -> SaveJson {
        obj(vec![
            ("id", json_str(&module.id)),
            ("artifactPath", json_str(&module.artifact_path)),
            ("digest", json_str(&module.digest)),
            ("revision", json_str(&module.revision)),
        ])
    }

    fn capture_callback(callback: &WeaponBehaviorCallback) -> SaveJson {
        match callback {
            WeaponBehaviorCallback::QuakeC { module, function_index } => obj(vec![
                ("kind", json_str("quakec")),
                ("module", capture_module(module)),
                ("functionIndex", int(*function_index as i64)),
            ]),
            WeaponBehaviorCallback::Qvm {
                module,
                instruction_index,
            } => obj(vec![
                ("kind", json_str("qvm")),
                ("module", capture_module(module)),
                ("instructionIndex", int(*instruction_index as i64)),
            ]),
            WeaponBehaviorCallback::NativeArtifact {
                module,
                image_offset,
                abi,
            } => obj(vec![
                ("kind", json_str("native-artifact")),
                ("module", capture_module(module)),
                ("imageOffset", SaveJson::BigInt(*image_offset as i128)),
                (
                    "abi",
                    obj(vec![
                        ("kind", json_str(&abi.kind)),
                        ("call", json_str(&abi.call)),
                        ("image", json_str(&abi.image)),
                        ("pointerBytes", int(i64::from(abi.pointer_bytes))),
                    ]),
                ),
            ]),
        }
    }

    obj(vec![
        ("id", json_str(&definition.id)),
        ("title", json_str(&definition.title)),
        ("role", json_str(definition.role.name())),
        ("aspect", json_str(&definition.aspect)),
        ("module", capture_module(&definition.module)),
        ("fire", capture_callback(&definition.fire)),
        (
            "activate",
            definition.activate.as_ref().map_or(SaveJson::Null, capture_callback),
        ),
    ])
}

fn capture_save_random_state(state: &SaveRandomState) -> SaveJson {
    use qa_world::save::value::SaveJson;
    use qa_world::save::value::{arr, int, obj, str as json_str};

    let draw_count = |draws: u64| int(draws.min(i64::MAX as u64) as i64);
    match state {
        SaveRandomState::Q3Lcg { seed, draws } => obj(vec![
            ("kind", json_str("q3-lcg")),
            ("seed", int(*seed)),
            ("draws", draw_count(*draws)),
        ]),
        SaveRandomState::MsvcrtRand { seed, draws } => obj(vec![
            ("kind", json_str("msvcrt-rand")),
            ("seed", int(*seed)),
            ("draws", draw_count(*draws)),
        ]),
        SaveRandomState::GlibcRandom {
            words,
            front,
            rear,
            draws,
        } => obj(vec![
            ("kind", json_str("glibc-random")),
            ("words", arr(words.iter().map(|word| int(*word)).collect())),
            ("front", int(*front)),
            ("rear", int(*rear)),
            ("draws", draw_count(*draws)),
        ]),
        SaveRandomState::RereleaseMt19937 { words, index, draws } => obj(vec![
            ("kind", json_str("q2-rerelease-mt19937")),
            ("words", arr(words.iter().map(|word| int(i64::from(*word))).collect())),
            ("index", int(i64::from(*index))),
            ("distribution", json_str("msvc-2022-17.6")),
            ("draws", draw_count(*draws)),
        ]),
        SaveRandomState::Guest { module, bytes, draws } => obj(vec![
            ("kind", json_str("guest")),
            ("module", json_str(module)),
            ("bytes", SaveJson::Bytes(bytes.clone())),
            ("draws", draw_count(*draws)),
        ]),
    }
}

/// Capture a behavior checkpoint in donor save shape.
#[must_use]
pub fn capture_quake_c_weapon_behavior_checkpoint(checkpoint: &QuakeCWeaponBehaviorCheckpoint) -> SaveJson {
    use qa_world::save::value::SaveJson;
    use qa_world::save::value::{arr, boolean, int, num, obj, str as json_str};

    let machine = &checkpoint.machine;
    obj(vec![
        ("version", int(checkpoint.version)),
        ("definition", capture_weapon_behavior_definition(&checkpoint.definition)),
        (
            "mode",
            json_str(match checkpoint.mode {
                SimulationMode::Singleplayer => "singleplayer",
                SimulationMode::Coop => "coop",
                SimulationMode::Deathmatch => "deathmatch",
            }),
        ),
        ("sourceTime", num(checkpoint.source_time)),
        ("random", capture_save_random_state(&checkpoint.random)),
        (
            "machine",
            obj(vec![
                ("globals", SaveJson::Bytes(machine.globals.clone())),
                ("entities", SaveJson::Bytes(machine.entities.clone())),
                ("entityCount", int(machine.entity_count as i64)),
                ("strings", SaveJson::Bytes(machine.strings.clone())),
                ("statement", int(machine.statement as i64)),
                ("functionIndex", int(machine.function_index as i64)),
                ("argumentCount", int(machine.argument_count as i64)),
                (
                    "profiling",
                    arr(machine.profiling.iter().map(|value| int(i64::from(*value))).collect()),
                ),
                ("traceEnabled", boolean(machine.trace_enabled)),
            ]),
        ),
        (
            "free",
            arr(checkpoint.free.iter().map(|slot| int(i64::from(*slot))).collect()),
        ),
        (
            "retired",
            arr(checkpoint.retired.iter().map(|slot| int(i64::from(*slot))).collect()),
        ),
        (
            "bindings",
            arr(checkpoint
                .bindings
                .iter()
                .map(|binding| {
                    obj(vec![
                        ("slot", int(i64::from(binding.slot))),
                        (
                            "actor",
                            obj(vec![
                                ("slot", int(i64::from(binding.actor.slot))),
                                ("generation", int(i64::from(binding.actor.generation))),
                            ]),
                        ),
                        (
                            "kind",
                            json_str(match binding.kind {
                                QuakeCWeaponBindingKind::Target => "target",
                                QuakeCWeaponBindingKind::Projectile => "projectile",
                            }),
                        ),
                    ])
                })
                .collect()),
        ),
    ])
}

#[cfg(test)]
mod tests {
    use qa_core::identity::IdentityOwner;
    use qa_core::identity::ProviderId;
    use qa_core::math::vec3;
    use qa_core::math::Bounds;
    use qa_guest::qc::program::load_qc_program;
    use qa_guest::qc::program::QcOpcode;
    use qa_guest::qvm::mod_provider::ModuleId;
    use qa_world::save::value::arr;
    use qa_world::save::value::boolean;
    use qa_world::save::value::int;
    use qa_world::save::value::num;
    use qa_world::save::value::obj;
    use qa_world::save::value::str as json_str;
    use qa_world::save::value::SaveJson;

    use super::*;

    /// Minimal progs.dat encoder for fixture programs.
    fn program_with_fire(calls_spawn: bool) -> QcProgram {
        // (raw type, word offset, name)
        let globals: Vec<(u16, u16, &str)> = vec![
            (4, 28, "self"),
            (4, 29, "other"),
            (2, 30, "time"),
            (2, 31, "deathmatch"),
            (2, 32, "coop"),
            (2, 33, "trace_fraction"),
            (2, 34, "trace_allsolid"),
            (2, 35, "trace_startsolid"),
            (2, 36, "trace_inwater"),
            (2, 37, "trace_inopen"),
            (2, 38, "trace_plane_dist"),
            (3, 39, "trace_endpos"),
            (3, 42, "trace_plane_normal"),
            (4, 45, "trace_ent"),
            (6, 46, "spawn_fn"),
        ];
        let fields: Vec<(u16, u16, &str)> = vec![
            (3, 0, "origin"),
            (3, 3, "velocity"),
            (3, 6, "angles"),
            (3, 9, "mins"),
            (3, 12, "maxs"),
            (3, 15, "v_angle"),
            (2, 18, "health"),
            (2, 19, "solid"),
            (2, 20, "nextthink"),
            (1, 21, "classname"),
            (1, 22, "netname"),
            (6, 23, "think"),
            (4, 24, "chain"),
            (1, 25, "model"),
            (2, 26, "modelindex"),
        ];
        let mut strings = vec![0u8];
        let mut offsets = HashMap::new();
        for name in ["fire", "test.qc", "spawn"]
            .into_iter()
            .chain(globals.iter().map(|(_, _, name)| *name))
            .chain(fields.iter().map(|(_, _, name)| *name))
        {
            if offsets.contains_key(name) {
                continue;
            }
            offsets.insert(name, strings.len() as i32);
            strings.extend_from_slice(name.as_bytes());
            strings.push(0);
        }
        let statements: Vec<(u16, u16, u16, u16)> = if calls_spawn {
            vec![
                (QcOpcode::Done as u16, 0, 0, 0),
                (QcOpcode::Call0 as u16, 46, 0, 0),
                (QcOpcode::Done as u16, 0, 0, 0),
            ]
        } else {
            vec![(QcOpcode::Done as u16, 0, 0, 0), (QcOpcode::Done as u16, 0, 0, 0)]
        };
        // (first, params, locals, profile, name, file, count)
        let functions = [
            (0i32, 0i32, 0i32, 0i32, offsets["fire"], offsets["test.qc"], 0i32),
            (1, 0, 0, 0, offsets["fire"], offsets["test.qc"], 0),
            (-14i32, 0, 0, 0, offsets["spawn"], offsets["test.qc"], 0),
        ];
        let mut blobs: Vec<Vec<u8>> = Vec::new();
        let mut statement_blob = Vec::new();
        for (op, a, b, c) in &statements {
            for word in [*op, *a, *b, *c] {
                statement_blob.extend_from_slice(&word.to_le_bytes());
            }
        }
        blobs.push(statement_blob);
        let definitions = |entries: &[(u16, u16, &str)]| {
            let mut blob = Vec::new();
            for (raw, at, name) in entries {
                blob.extend_from_slice(&raw.to_le_bytes());
                blob.extend_from_slice(&at.to_le_bytes());
                blob.extend_from_slice(&offsets[*name].to_le_bytes());
            }
            blob
        };
        blobs.push(definitions(&globals));
        blobs.push(definitions(&fields));
        let mut function_blob = Vec::new();
        for (first, params, locals, profile, name, file, count) in &functions {
            for word in [*first, *params, *locals, *profile, *name, *file, *count] {
                function_blob.extend_from_slice(&word.to_le_bytes());
            }
            function_blob.extend_from_slice(&[0u8; 8]);
        }
        blobs.push(function_blob);
        blobs.push(strings.clone());
        let mut values = vec![0u8; 47 * 4];
        values[46 * 4..46 * 4 + 4].copy_from_slice(&2i32.to_le_bytes());
        blobs.push(values);
        let counts = [
            statements.len() as i32,
            globals.len() as i32,
            fields.len() as i32,
            functions.len() as i32,
            strings.len() as i32,
            47,
        ];
        let mut image = Vec::new();
        image.extend_from_slice(&6i32.to_le_bytes());
        image.extend_from_slice(&5927i32.to_le_bytes());
        let mut offset = 60i32;
        for (blob, count) in blobs.iter().zip(counts) {
            image.extend_from_slice(&offset.to_le_bytes());
            image.extend_from_slice(&count.to_le_bytes());
            offset += blob.len() as i32;
        }
        image.extend_from_slice(&27i32.to_le_bytes());
        for blob in &blobs {
            image.extend_from_slice(blob);
        }
        load_qc_program(&image, None, "test.dat").unwrap()
    }

    fn module_for(program: &QcProgram) -> ModuleId {
        ModuleId {
            id: "test:behavior".to_string(),
            artifact_path: "progs.dat".to_string(),
            digest: program_digest(program),
            revision: "r1".to_string(),
        }
    }

    fn definition_for(program: &QcProgram) -> WeaponBehaviorDefinition {
        let module = module_for(program);
        WeaponBehaviorDefinition {
            id: "qc:test".to_string(),
            title: "Test".to_string(),
            role: WeaponBehaviorRole::Rocket,
            aspect: "trajectory".to_string(),
            module: module.clone(),
            fire: WeaponBehaviorCallback::QuakeC {
                module: module.clone(),
                function_index: 1,
            },
            activate: None,
        }
    }

    struct NullScene;

    impl QcWeaponScene for NullScene {
        fn trace_q1_point(
            &self,
            _start: Vec3,
            end: Vec3,
            _movement: Q1Move,
            _pass_actor: Option<ActorId>,
        ) -> QcWeaponTrace {
            QcWeaponTrace {
                fraction: 1.0,
                end,
                all_solid: false,
                start_solid: false,
                in_water: false,
                in_open: true,
                plane_distance: 0.0,
                plane_normal: vec3(0.0, 0.0, 1.0),
                hit: TraceHit::None,
            }
        }

        fn point_contents_q1(&self, _point: Vec3) -> i32 {
            -1
        }
    }

    fn body() -> BodyState {
        BodyState {
            origin: vec3(0.0, 0.0, 0.0),
            angles: vec3(0.0, 0.0, 0.0),
            velocity: vec3(0.0, 0.0, 0.0),
            bounds: Bounds {
                min: vec3(-16.0, -16.0, -16.0),
                max: vec3(16.0, 16.0, 16.0),
            },
            ground: None,
        }
    }

    fn options(program: QcProgram, definition: WeaponBehaviorDefinition) -> QcWeaponBehaviorOptions {
        QcWeaponBehaviorOptions {
            definition,
            program,
            random: SourceRandom::new(7),
            scene: Rc::new(NullScene),
            mode: SimulationMode::Deathmatch,
            targets: Rc::new(Vec::new),
            aim: Rc::new(|_, _| vec3(1.0, 0.0, 0.0)),
            model: Rc::new(|_| None),
            print: Rc::new(|_, _| {}),
        }
    }

    #[test]
    fn rejects_artifact_mismatch() {
        let program = program_with_fire(true);
        let mut definition = definition_for(&program);
        definition.module.digest = "sha256:dead".to_string();
        let error = QuakeCWeaponBehaviorSource::new(options(program, definition)).unwrap_err();
        assert!(matches!(error, QuakeCWeaponBehaviorError::ArtifactMismatch));
    }

    #[test]
    fn rejects_non_quakec_fire() {
        let program = program_with_fire(true);
        let mut definition = definition_for(&program);
        definition.fire = WeaponBehaviorCallback::Qvm {
            module: definition.module.clone(),
            instruction_index: 1,
        };
        let error = QuakeCWeaponBehaviorSource::new(options(program, definition)).unwrap_err();
        assert!(matches!(error, QuakeCWeaponBehaviorError::ArtifactMismatch));
    }

    #[test]
    fn rejects_unknown_callback() {
        let program = program_with_fire(true);
        let mut definition = definition_for(&program);
        definition.fire = WeaponBehaviorCallback::QuakeC {
            module: definition.module.clone(),
            function_index: 99,
        };
        let error = QuakeCWeaponBehaviorSource::new(options(program, definition)).unwrap_err();
        assert!(matches!(error, QuakeCWeaponBehaviorError::CallbackMismatch));
    }

    #[test]
    fn attach_declines_without_spawn() {
        let owner = IdentityOwner::create("decline").unwrap();
        let program = program_with_fire(false);
        let definition = definition_for(&program);
        let source = QuakeCWeaponBehaviorSource::new(options(program, definition)).unwrap();
        let projectile = owner
            .owned_actor(&owner.actor(5, 0), ProviderId::new("test", "qc"))
            .unwrap();
        let launch = WeaponBehaviorLaunch {
            projectile,
            shooter: owner.actor(1, 0),
            weapon: "q1:weapon/rocket".to_string(),
            role: WeaponBehaviorRole::Rocket,
            time_seconds: 1.0,
            body: body(),
        };
        assert!(source.attach(launch).unwrap().is_none());
    }

    #[test]
    fn attach_spawns_resumes_and_closes() {
        let owner = IdentityOwner::create("spawn").unwrap();
        let program = program_with_fire(true);
        let definition = definition_for(&program);
        let source = QuakeCWeaponBehaviorSource::new(options(program, definition)).unwrap();
        let projectile_id = owner.actor(5, 0);
        let projectile = owner
            .owned_actor(&projectile_id, ProviderId::new("test", "qc"))
            .unwrap();
        let launch = WeaponBehaviorLaunch {
            projectile,
            shooter: owner.actor(1, 0),
            weapon: "q1:weapon/rocket".to_string(),
            role: WeaponBehaviorRole::Rocket,
            time_seconds: 1.0,
            body: body(),
        };
        let instance = source.attach(launch).unwrap().expect("spawned");
        assert_eq!(instance.initial().origin, vec3(0.0, 0.0, 0.0));
        assert!(source.resume(&projectile_id).is_err());
        instance.close();
        assert!(matches!(
            source.resume(&projectile_id).unwrap_err(),
            QuakeCWeaponBehaviorError::MissingRestored
        ));
    }

    #[test]
    fn step_returns_none_before_think() {
        let owner = IdentityOwner::create("step").unwrap();
        let program = program_with_fire(true);
        let definition = definition_for(&program);
        let source = QuakeCWeaponBehaviorSource::new(options(program, definition)).unwrap();
        let projectile_id = owner.actor(5, 0);
        let projectile = owner
            .owned_actor(&projectile_id, ProviderId::new("test", "qc"))
            .unwrap();
        let launch = WeaponBehaviorLaunch {
            projectile,
            shooter: owner.actor(1, 0),
            weapon: "q1:weapon/rocket".to_string(),
            role: WeaponBehaviorRole::Rocket,
            time_seconds: 1.0,
            body: body(),
        };
        let instance = source.attach(launch).unwrap().expect("spawned");
        assert_eq!(instance.step(&body(), 2.0).unwrap(), None);
        instance.close();
    }

    #[test]
    fn rejects_role_mismatch() {
        let owner = IdentityOwner::create("role").unwrap();
        let program = program_with_fire(true);
        let definition = definition_for(&program);
        let source = QuakeCWeaponBehaviorSource::new(options(program, definition)).unwrap();
        let projectile = owner
            .owned_actor(&owner.actor(5, 0), ProviderId::new("test", "qc"))
            .unwrap();
        let launch = WeaponBehaviorLaunch {
            projectile,
            shooter: owner.actor(1, 0),
            weapon: "q1:weapon/grenade".to_string(),
            role: WeaponBehaviorRole::Grenade,
            time_seconds: 1.0,
            body: body(),
        };
        assert!(matches!(
            source.attach(launch).unwrap_err(),
            QuakeCWeaponBehaviorError::RoleMismatch
        ));
    }

    #[test]
    fn checkpoint_restore_round_trip() {
        let owner = IdentityOwner::create("restore").unwrap();
        let program = program_with_fire(true);
        let definition = definition_for(&program);
        let source = QuakeCWeaponBehaviorSource::new(options(program, definition)).unwrap();
        let projectile_id = owner.actor(5, 0);
        let projectile = owner
            .owned_actor(&projectile_id, ProviderId::new("test", "qc"))
            .unwrap();
        let launch = WeaponBehaviorLaunch {
            projectile,
            shooter: owner.actor(1, 0),
            weapon: "q1:weapon/rocket".to_string(),
            role: WeaponBehaviorRole::Rocket,
            time_seconds: 1.0,
            body: body(),
        };
        let instance = source.attach(launch).unwrap().expect("spawned");
        let checkpoint = source.checkpoint().unwrap();
        assert_eq!(checkpoint.version, 1);
        instance.close();

        let program = program_with_fire(true);
        let definition = definition_for(&program);
        let restored = QuakeCWeaponBehaviorSource::new(options(program, definition)).unwrap();
        let saved_random = source.shared.borrow().random.borrow().checkpoint();
        let mut random = SourceRandom::new(999);
        random.restore(&saved_random).unwrap();
        let by_slot = |saved: &SavedActorId| owner.actor(saved.slot, saved.generation);
        restored.restore(&checkpoint, &by_slot, random).unwrap();
        assert!(restored.resume(&projectile_id).is_ok());
    }

    #[test]
    fn restore_rejects_foreign_random() {
        let owner = IdentityOwner::create("random").unwrap();
        let program = program_with_fire(true);
        let definition = definition_for(&program);
        let source = QuakeCWeaponBehaviorSource::new(options(program, definition)).unwrap();
        let checkpoint = source.checkpoint().unwrap();
        let program = program_with_fire(true);
        let definition = definition_for(&program);
        let restored = QuakeCWeaponBehaviorSource::new(options(program, definition)).unwrap();
        let mut divergent = SourceRandom::new(7);
        divergent.next_integer();
        let error = restored
            .restore(
                &checkpoint,
                &|saved| owner.actor(saved.slot, saved.generation),
                divergent,
            )
            .unwrap_err();
        assert!(matches!(error, QuakeCWeaponBehaviorError::RandomMismatch));
    }

    struct TestDefinitions;

    impl WeaponBehaviorDefinitionReader for TestDefinitions {
        fn read_definition(
            &self,
            reader: SaveReader<'_>,
            expected: &WeaponBehaviorDefinition,
        ) -> Result<WeaponBehaviorDefinition, WorldError> {
            reader.field("id").literal_str(&expected.id)?;
            Ok(expected.clone())
        }
    }

    #[test]
    fn reads_saved_checkpoint() {
        let program = program_with_fire(true);
        let expected = definition_for(&program);
        let value = obj(vec![
            ("version", int(1)),
            ("definition", obj(vec![("id", json_str(&expected.id))])),
            ("mode", json_str("deathmatch")),
            ("sourceTime", num(3.5)),
            (
                "random",
                obj(vec![
                    ("kind", json_str("glibc-random")),
                    ("draws", int(0)),
                    ("words", arr(vec![int(1), int(2)])),
                    ("front", int(3)),
                    ("rear", int(0)),
                ]),
            ),
            (
                "machine",
                obj(vec![
                    ("globals", SaveJson::Bytes(vec![0, 0, 0, 0])),
                    ("entities", SaveJson::Bytes(vec![0, 0, 0, 0])),
                    ("entityCount", int(1)),
                    ("strings", SaveJson::Bytes(vec![0])),
                    ("statement", int(0)),
                    ("functionIndex", int(0)),
                    ("argumentCount", int(0)),
                    ("profiling", arr(vec![])),
                    ("traceEnabled", boolean(false)),
                ]),
            ),
            ("free", arr(vec![])),
            ("retired", arr(vec![])),
            ("bindings", arr(vec![])),
        ]);
        let reader = SaveReader::new(&value);
        let checkpoint = read_quake_c_weapon_behavior_checkpoint(&reader, &expected, &TestDefinitions).unwrap();
        assert_eq!(checkpoint.version, 1);
        assert_eq!(checkpoint.mode, SimulationMode::Deathmatch);
        assert_eq!(checkpoint.source_time, 3.5);
        assert_eq!(checkpoint.machine.entity_count, 1);
    }
}
