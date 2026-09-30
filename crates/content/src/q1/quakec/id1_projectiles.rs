//! Projectile id1 attacks (`src/content/q1/quakec/id1-projectiles.ts`).
//!
//! Donor provenance: `src/content/q1/quakec/id1-projectiles.ts`
//! (`Id1ProjectileAttacks`, `Id1ProjectileAttack`).

use std::cell::RefCell;
use std::collections::HashMap;

use qa_core::identity::{ActorId, SavedActorId};
use qa_core::math::Vec3;

use crate::contract::ItemId;
use crate::value::{namespaced, SaveReader};

use super::id1_damage::Id1DamageCall;
use super::id1_program::{id1_program_snapshot, Id1Attribution, Id1ProgramBinding};
use super::qc_view::{
    MachineFn, QcCallSite, QcEntityStoreObservation, QcFunctionBoundary, QcFunctionExecution, QcHostSource,
    QcMachineView,
};
use super::{FrameGuard, QcError};

/// Projectile launch (donor `Id1ProjectileAttack.launch`).
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectileLaunch {
    /// Launching owner.
    pub owner: ActorId,
    /// Emission time.
    pub emitted_at: f64,
}

/// Projectile trace (donor `Id1ProjectileAttack.trace`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProjectileTrace {
    /// Trace end.
    pub point: Vec3,
    /// Trace plane normal.
    pub normal: Vec3,
}

/// Projectile attack (donor `Id1ProjectileAttack`).
#[derive(Debug, Clone, PartialEq)]
pub struct Id1ProjectileAttack {
    /// Attack weapon.
    pub weapon: ItemId,
    /// Attack time.
    pub time: f64,
    /// Launch provenance.
    pub launch: Option<ProjectileLaunch>,
    /// Lightning trace.
    pub trace: Option<ProjectileTrace>,
}

/// Projectile emission (donor `Emission`).
#[derive(Debug, Clone, PartialEq)]
struct Emission {
    owner: ActorId,
    emitted_at: f64,
    weapon: ItemId,
}

/// Saved emission (donor `capture` entry).
#[derive(Debug, Clone, PartialEq)]
pub struct SavedProjectileEmission {
    /// Projectile actor.
    pub actor: SavedActorId,
    /// Launching owner.
    pub owner: SavedActorId,
    /// Emission time.
    pub emitted_at: f64,
    /// Attack weapon.
    pub weapon: ItemId,
}

/// Projectile attacks (donor `Id1ProjectileAttacks`).
pub struct Id1ProjectileAttacks<'a> {
    source: QcHostSource<'a>,
    machine: MachineFn<'a>,
    emissions: RefCell<HashMap<ActorId, Emission>>,
    firing: RefCell<Vec<Emission>>,
    weapons: HashMap<usize, ItemId>,
    sites: HashMap<usize, usize>,
    owner_field: Option<usize>,
    damage_function: usize,
    radius_function: usize,
    lightning_function: usize,
    launch_spike_function: usize,
}

impl<'a> Id1ProjectileAttacks<'a> {
    /// Bind projectile attacks (donor `Id1ProjectileAttacks`
    /// constructor).
    pub fn new(source: QcHostSource<'a>, machine: MachineFn<'a>) -> Result<Self, QcError> {
        let binding: Id1ProgramBinding = id1_program_snapshot(source.program)?;
        if binding.attribution == Id1Attribution::Native {
            let damage = source.program.function_named("T_Damage")?.index;
            return Ok(Self {
                source,
                machine,
                emissions: RefCell::new(HashMap::new()),
                firing: RefCell::new(Vec::new()),
                weapons: HashMap::new(),
                sites: HashMap::new(),
                owner_field: None,
                damage_function: damage,
                radius_function: 0,
                lightning_function: 0,
                launch_spike_function: 0,
            });
        }
        let mut weapons = HashMap::new();
        for (name, item) in [
            ("W_FireLightning", "q1:weapon/lightning"),
            ("W_FireSuperSpikes", "q1:weapon/supernailgun"),
            ("W_FireSpikes", "q1:weapon/nailgun"),
            ("FireSpikes", "q1:weapon/nailgun"),
            ("W_FireRocket", "q1:weapon/rocketlauncher"),
        ] {
            weapons.insert(source.program.function_named(name)?.index, item.to_string());
        }
        let at = |name: &str| -> Result<usize, QcError> { Ok(source.program.function_named(name)?.index) };
        let quakeworld =
            source.program.digest == "sha256:ff51cb5e77360d72b93487d89198dcf94629b92f8bae100fc6ea48a6c12a7830";
        let sites: HashMap<usize, usize> = if quakeworld {
            [
                (3256, 147),
                (580, 84),
                (3900, 158),
                (3973, 159),
                (3388, 149),
                (3478, 151),
                (3720, 154),
            ]
            .into_iter()
            .collect()
        } else {
            [
                (3783, 183),
                (1623, 118),
                (1629, 118),
                (4330, 193),
                (4392, 194),
                (3910, 185),
                (3943, 185),
                (3968, 185),
            ]
            .into_iter()
            .collect()
        };
        let owner_field = source
            .program
            .field_named("owner")
            .map(|field| field.offset)
            .ok_or_else(|| QcError::program("Missing projectile owner field", source.program.source))?;
        Ok(Self {
            source,
            machine,
            emissions: RefCell::new(HashMap::new()),
            firing: RefCell::new(Vec::new()),
            weapons,
            sites,
            owner_field: Some(owner_field),
            damage_function: binding.damage.index,
            radius_function: at("T_RadiusDamage")?,
            lightning_function: at("W_FireLightning")?,
            launch_spike_function: at("launch_spike")?,
        })
    }

    /// Captured emissions in deterministic order (donor `capture`).
    pub fn capture(&self) -> Result<Vec<SavedProjectileEmission>, QcError> {
        if !self.firing.borrow().is_empty() {
            return Err(QcError::program(
                "Cannot save during projectile emission",
                self.source.program.source,
            ));
        }
        let mut entries: Vec<SavedProjectileEmission> = self
            .emissions
            .borrow()
            .iter()
            .map(|(actor, emission)| SavedProjectileEmission {
                actor: SavedActorId {
                    slot: actor.slot(),
                    generation: actor.generation(),
                },
                owner: SavedActorId {
                    slot: emission.owner.slot(),
                    generation: emission.owner.generation(),
                },
                emitted_at: emission.emitted_at,
                weapon: emission.weapon.clone(),
            })
            .collect();
        entries.sort_by(|left, right| {
            (left.actor.slot, left.actor.generation, left.weapon.clone()).cmp(&(
                right.actor.slot,
                right.actor.generation,
                right.weapon.clone(),
            ))
        });
        Ok(entries)
    }

    /// Restore captured emissions (donor `restore`).
    pub fn restore(&self, reader: SaveReader) -> Result<(), QcError> {
        let entries = reader.list(|entry| {
            let reference = |name: &str| -> Result<ActorId, QcError> {
                let value = entry.field(name);
                let slot = u32::try_from(value.field("slot").integer(0)?)
                    .map_err(|_| QcError::from(value.field("slot").fail("expected an integer in range")))?;
                let generation = u32::try_from(value.field("generation").integer(0)?)
                    .map_err(|_| QcError::from(value.field("generation").fail("expected an integer in range")))?;
                Ok(self.source.actors.reference_saved(SavedActorId { slot, generation }))
            };
            Ok::<_, QcError>((
                reference("actor")?,
                Emission {
                    owner: reference("owner")?,
                    emitted_at: entry.field("emittedAt").finite()?,
                    weapon: namespaced(entry.field("weapon"))?,
                },
            ))
        })?;
        let mut emissions = HashMap::with_capacity(entries.len());
        for (actor, emission) in entries {
            if emissions.insert(actor, emission).is_some() {
                return Err(QcError::program(
                    "Duplicate saved projectile emission",
                    self.source.program.source,
                ));
            }
        }
        *self.emissions.borrow_mut() = emissions;
        Ok(())
    }

    /// Machine accessor with ownership check (donor `vm()`).
    fn vm(&self) -> Result<&'a dyn QcMachineView, QcError> {
        let vm = (self.machine)();
        if vm.program_digest() != self.source.program.digest {
            return Err(QcError::program(
                "Projectile observer belongs to another VM",
                self.source.program.source,
            ));
        }
        Ok(vm)
    }

    /// Actor at a reference (donor `actor`).
    fn actor(&self, vm: &dyn QcMachineView, reference: i32) -> Result<ActorId, QcError> {
        let slot = vm.entity_slot(reference)?;
        match self.source.slots.at(slot) {
            Some(actor) if self.source.actors.is_live(actor.id()) => Ok(actor.id().clone()),
            _ => Err(QcError::program(
                "Projectile references a free actor",
                self.source.program.source,
            )),
        }
    }

    /// Compose firing boundaries (donor `compose`).
    pub fn compose(&'a self, inner: QcFunctionBoundary<'a>) -> QcFunctionBoundary<'a> {
        let mut functions = inner.functions.clone();
        functions.extend(self.weapons.keys().copied());
        QcFunctionBoundary {
            functions,
            run: Box::new(move |call, execute| self.run(call, execute, &inner)),
        }
    }

    /// Boundary dispatch.
    fn run(
        &self,
        call: &QcCallSite,
        execute: &dyn QcFunctionExecution,
        inner: &QcFunctionBoundary,
    ) -> Result<(), QcError> {
        let Some(weapon) = self.weapons.get(&call.function_index).cloned() else {
            return (inner.run)(call, execute);
        };
        let vm = self.vm()?;
        let guard = FrameGuard::push(
            &self.firing,
            Emission {
                weapon,
                owner: self.actor(vm, vm.global_int(vm.global_offset("self")?)?)?,
                emitted_at: vm.global_float(vm.global_offset("time")?)?,
            },
        );
        let outcome = if inner.functions.contains(&call.function_index) {
            (inner.run)(call, execute)
        } else {
            execute.run(None)
        };
        guard.defuse();
        outcome
    }

    /// Observe an entity store (donor `observeStore`).
    pub fn observe_store(&self, store: &QcEntityStoreObservation) -> Result<(), QcError> {
        if Some(store.word) != self.owner_field {
            return Ok(());
        }
        if store.function_index != self.launch_spike_function
            && (!self.weapons.contains_key(&store.function_index) || store.function_index == self.lightning_function)
        {
            return Ok(());
        }
        let emission = self.firing.borrow().last().cloned();
        let Some(emission) = emission else {
            return Ok(());
        };
        let owner = store
            .after
            .get(0..4)
            .and_then(|bytes| bytes.try_into().ok())
            .map(|bytes: [u8; 4]| i32::from_le_bytes(bytes));
        let Some(owner) = owner else {
            return Err(QcError::program(
                "short projectile owner observation",
                self.source.program.source,
            ));
        };
        if self.actor(self.vm()?, owner)? != emission.owner {
            return Ok(());
        }
        let projectile = self.actor(self.vm()?, store.reference)?;
        self.emissions
            .borrow_mut()
            .retain(|actor, _| self.source.actors.is_live(actor));
        if projectile != emission.owner {
            self.emissions.borrow_mut().insert(projectile, emission);
        }
        Ok(())
    }

    /// Projectile attack scope (donor `resolve`).
    pub fn resolve(&self, call: &Id1DamageCall) -> Result<Option<Id1ProjectileAttack>, QcError> {
        if call.call.function_index != self.damage_function
            || self.sites.get(&call.call.statement) != Some(&call.call.caller)
        {
            return Ok(None);
        }
        let vm = self.vm()?;
        if self.actor(vm, vm.arg_int(0)?)? != call.target
            || self.actor(vm, vm.arg_int(1)?)? != call.inflictor
            || self.actor(vm, vm.arg_int(2)?)? != call.attacker
            || vm.arg_float(3)? != call.amount
        {
            return Err(QcError::program(
                "Projectile damage arguments changed",
                self.source.program.source,
            ));
        }
        let launch = self.emissions.borrow().get(&call.inflictor).cloned();
        let active = self.firing.borrow().last().cloned();
        if launch.is_none()
            && (active.is_none() || active.as_ref().is_some_and(|active| call.inflictor != active.owner))
        {
            return Ok(None);
        }
        let emission = launch.clone().or(active.clone());
        let Some(emission) = emission else {
            return Ok(None);
        };
        if launch.is_none()
            && (active.is_none()
                || (call.call.caller != self.radius_function
                    && !self.weapons.contains_key(&call.call.caller)
                    && active
                        .as_ref()
                        .is_some_and(|active| Some(&active.weapon) != self.weapons.get(&self.lightning_function))))
        {
            return Ok(None);
        }
        let lightning = active
            .as_ref()
            .is_some_and(|active| Some(&active.weapon) == self.weapons.get(&self.lightning_function));
        let has_launch = launch.is_some();
        let trace = if !has_launch
            && lightning
            && call.call.caller != self.radius_function
            && call.call.caller != self.lightning_function
        {
            Some(ProjectileTrace {
                point: vm.global_vector(vm.global_offset("trace_endpos")?)?,
                normal: vm.global_vector(vm.global_offset("trace_plane_normal")?)?,
            })
        } else {
            None
        };
        Ok(Some(Id1ProjectileAttack {
            weapon: emission.weapon.clone(),
            time: vm.global_float(vm.global_offset("time")?)?,
            launch: launch.map(|launch| ProjectileLaunch {
                owner: launch.owner,
                emitted_at: launch.emitted_at,
            }),
            trace,
        }))
    }
}
