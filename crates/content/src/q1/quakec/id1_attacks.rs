//! Synchronous id1 attacks (`src/content/q1/quakec/id1-attacks.ts`).
//!
//! Donor provenance: `src/content/q1/quakec/id1-attacks.ts`
//! (`Id1SynchronousAttacks`, `Id1SynchronousAttack`).

use std::cell::RefCell;
use std::collections::HashMap;

use qa_core::identity::ActorId;
use qa_core::math::Vec3;

use crate::contract::ItemId;

use super::id1_damage::Id1DamageCall;
use super::id1_program::{id1_damage_multiplier, id1_program_snapshot, Id1AttacksLayout, Id1ProgramBinding};
use super::qc_view::{MachineFn, QcCallSite, QcFunctionBoundary, QcFunctionExecution, QcHostSource, QcMachineView};
use super::{FrameGuard, QcError};

/// Synchronous attack (donor `Id1SynchronousAttack`).
#[derive(Debug, Clone, PartialEq)]
pub struct Id1SynchronousAttack {
    /// Attacking actor.
    pub actor: ActorId,
    /// Attack weapon.
    pub weapon: ItemId,
    /// Attack time.
    pub time: f64,
    /// Knockback.
    pub knockback: f64,
    /// Hit direction.
    pub direction: Vec3,
    /// Hit point.
    pub point: Vec3,
    /// Hit normal.
    pub normal: Vec3,
}

/// Captured target trace.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Trace {
    point: Vec3,
    normal: Vec3,
}

/// Active attack (donor `Attack`).
#[derive(Debug, Clone, PartialEq)]
struct Attack {
    actor: ActorId,
    reference: i32,
    weapon: ItemId,
    time: f64,
    traces: HashMap<ActorId, Trace>,
}

/// Synchronous axe and shotgun attacks (donor `Id1SynchronousAttacks`).
pub struct Id1SynchronousAttacks<'a> {
    source: QcHostSource<'a>,
    machine: MachineFn<'a>,
    binding: Id1ProgramBinding,
    active: RefCell<Vec<Attack>>,
}

impl<'a> Id1SynchronousAttacks<'a> {
    /// Bind synchronous attacks (donor `Id1SynchronousAttacks`
    /// constructor).
    pub fn new(source: QcHostSource<'a>, machine: MachineFn<'a>) -> Result<Self, QcError> {
        Ok(Self {
            binding: id1_program_snapshot(source.program)?,
            source,
            machine,
            active: RefCell::new(Vec::new()),
        })
    }

    /// Fail when an attack is active (donor `assertIdle`).
    pub fn assert_idle(&self) -> Result<(), QcError> {
        if !self.active.borrow().is_empty() {
            return Err(QcError::program(
                "Cannot save during an id1 attack",
                self.source.program.source,
            ));
        }
        Ok(())
    }

    /// Machine accessor with ownership check (donor `vm()`).
    fn vm(&self) -> Result<&'a dyn QcMachineView, QcError> {
        let vm = (self.machine)();
        if vm.program_digest() != self.source.program.digest {
            return Err(QcError::program(
                "id1 attacks belong to another machine",
                self.source.program.source,
            ));
        }
        Ok(vm)
    }

    /// Field offset (donor `field`).
    fn field(&self, vm: &dyn QcMachineView, name: &str) -> Result<usize, QcError> {
        vm.field_offset(name)
            .map_err(|_| QcError::program(format!("Missing id1 attack field {name}"), self.source.program.source))
    }

    /// Current trace (donor `trace`).
    fn trace(&self, vm: &dyn QcMachineView) -> Result<Trace, QcError> {
        Ok(Trace {
            point: vm.global_vector(vm.global_offset("trace_endpos")?)?,
            normal: vm.global_vector(vm.global_offset("trace_plane_normal")?)?,
        })
    }

    /// Compose attack boundaries (donor `compose`).
    pub fn compose(&'a self, damage: QcFunctionBoundary<'a>) -> QcFunctionBoundary<'a> {
        let Some(layout) = self.binding.attacks.clone() else {
            return damage;
        };
        let mut functions = damage.functions.clone();
        functions.extend([layout.axe, layout.shotgun, layout.super_shotgun, layout.add_multi]);
        QcFunctionBoundary {
            functions,
            run: Box::new(move |call, execute| self.run(call, execute, &damage, &layout)),
        }
    }

    /// Boundary dispatch.
    fn run(
        &self,
        call: &QcCallSite,
        execute: &dyn QcFunctionExecution,
        damage: &QcFunctionBoundary,
        layout: &Id1AttacksLayout,
    ) -> Result<(), QcError> {
        if damage.functions.contains(&call.function_index) {
            return (damage.run)(call, execute);
        }
        let vm = self.vm()?;
        if call.function_index == layout.add_multi {
            if call.caller == layout.trace_attack && !self.active.borrow().is_empty() {
                let slot = vm.entity_slot(vm.arg_int(0)?)?;
                if let Some(target) = self.source.slots.at(slot) {
                    let trace = self.trace(vm)?;
                    if let Some(attack) = self.active.borrow_mut().last_mut() {
                        attack.traces.insert(target.id().clone(), trace);
                    }
                }
            }
            return execute.run(None);
        }
        let reference = vm.global_int(vm.global_offset("self")?)?;
        let slot = vm.entity_slot(reference)?;
        let actor = self.source.slots.at(slot);
        match actor {
            Some(actor) if self.source.actors.is_live(actor.id()) => {
                if vm.strings_get(vm.entity_int(slot, self.field(vm, "classname")?)?)? != "player" {
                    return Err(QcError::program(
                        "id1 player attack requires a source player",
                        self.source.program.source,
                    ));
                }
                let weapon = if call.function_index == layout.axe {
                    "q1:weapon/axe"
                } else if call.function_index == layout.shotgun {
                    "q1:weapon/shotgun"
                } else {
                    "q1:weapon/supershotgun"
                };
                let guard = FrameGuard::push(
                    &self.active,
                    Attack {
                        actor: actor.id().clone(),
                        reference,
                        weapon: weapon.to_string(),
                        time: vm.global_float(vm.global_offset("time")?)?,
                        traces: HashMap::new(),
                    },
                );
                let outcome = execute.run(None);
                guard.defuse();
                outcome
            }
            _ => Err(QcError::program(
                "id1 attack has no live source actor",
                self.source.program.source,
            )),
        }
    }

    /// Synchronous attack scope (donor `resolve`).
    pub fn resolve(&self, call: &Id1DamageCall) -> Result<Option<Id1SynchronousAttack>, QcError> {
        let Some(layout) = self.binding.attacks.clone() else {
            return Ok(None);
        };
        let axe = layout.axe_damage.contains(&call.call.statement);
        let shotgun = call.call.caller == layout.trace_attack
            || call.call.caller == layout.add_multi
            || call.call.caller == layout.apply_multi_damage.0 && call.call.statement == layout.apply_multi_damage.1;
        if !axe && !shotgun {
            return Ok(None);
        }
        let vm = self.vm()?;
        let attack = self.active.borrow().last().cloned();
        let Some(attack) = attack else {
            return Err(QcError::program(
                "Unmatched id1 synchronous damage scope",
                self.source.program.source,
            ));
        };
        if !self.source.actors.is_live(&attack.actor)
            || attack.actor != call.attacker
            || attack.actor != call.inflictor
            || vm.global_int(vm.global_offset("self")?)? != attack.reference
            || vm.global_int(self.binding.damage.global)? != self.binding.damage.index as i32
            || axe != (attack.weapon == "q1:weapon/axe")
        {
            return Err(QcError::program(
                "Unmatched id1 synchronous damage scope",
                self.source.program.source,
            ));
        }
        let target = self.source.actors.source_of(&call.target);
        let Some(target) = target else {
            return Err(QcError::program(
                "id1 attack target has no source projection",
                self.source.program.source,
            ));
        };
        if target.provider != self.source.slots.provider() {
            return Err(QcError::program(
                "id1 attack target has no source projection",
                self.source.program.source,
            ));
        }
        let trace = if axe {
            self.trace(vm)?
        } else {
            attack.traces.get(&call.target).copied().ok_or_else(|| {
                QcError::program(
                    "id1 shotgun damage has no captured target trace",
                    self.source.program.source,
                )
            })?
        };
        let owner = vm.entity_slot(attack.reference)?;
        let victim = target.slot;
        let min = vm.entity_vector(owner, self.field(vm, "absmin")?)?;
        let max = vm.entity_vector(owner, self.field(vm, "absmax")?)?;
        let origin = vm.entity_vector(victim, self.field(vm, "origin")?)?;
        let numeric = vm.numeric();
        let component = |point: f32, edge: f32, opposite: f32| -> f64 {
            numeric.sub(
                f64::from(point),
                numeric.mul(0.5, numeric.add(f64::from(edge), f64::from(opposite))),
            )
        };
        let delta = [
            component(origin.x, min.x, max.x),
            component(origin.y, min.y, max.y),
            component(origin.z, min.z, max.z),
        ];
        let magnitude = numeric.sqrt(numeric.add(
            numeric.add(numeric.mul(delta[0], delta[0]), numeric.mul(delta[1], delta[1])),
            numeric.mul(delta[2], delta[2]),
        ));
        let inverse = if magnitude == 0.0 {
            0.0
        } else {
            numeric.div(1.0, magnitude)
        };
        Ok(Some(Id1SynchronousAttack {
            actor: attack.actor.clone(),
            weapon: attack.weapon.clone(),
            time: attack.time,
            knockback: numeric.mul(
                call.amount,
                id1_damage_multiplier(self.source.program, vm, attack.reference, attack.reference)?,
            ),
            direction: Vec3 {
                x: numeric.store(numeric.mul(delta[0], inverse)),
                y: numeric.store(numeric.mul(delta[1], inverse)),
                z: numeric.store(numeric.mul(delta[2], inverse)),
            },
            point: trace.point,
            normal: trace.normal,
        }))
    }
}
