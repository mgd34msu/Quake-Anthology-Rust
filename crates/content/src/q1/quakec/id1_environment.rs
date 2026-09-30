//! Environmental id1 damage (`src/content/q1/quakec/id1-environment.ts`).
//!
//! Donor provenance: `src/content/q1/quakec/id1-environment.ts`
//! (`Id1Environment`, `Id1EnvironmentalDamage`, `Id1PhysicsCallback`).
//!
//! The donor drives vector math through `createMutableVectorMath`
//! (`src/core/math.ts`); this port inlines the used operations
//! (`VectorAdd`, `VectorScale`, `VectorSubtract`, `VectorNormalize`)
//! over [`NumericOps`].

use qa_core::identity::ActorId;
use qa_core::math::Vec3;
use qa_core::numeric::{Arithmetic, NumericOps};

use super::id1_damage::Id1DamageCall;
use super::id1_program::{id1_damage_multiplier, id1_program_snapshot, EnvContext, Id1ProgramBinding, NativeEnv};
use super::qc_gameplay::DamageCause;
use super::qc_view::{MachineFn, QcHostSource, QcOpcode};
use super::QcError;

/// Blocked and touch continuations report map hazards through the
/// same boundary (donor `Id1PhysicsCallback`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Id1PhysicsCallback {
    /// Callback kind.
    pub kind: EnvCallbackKind,
    /// Acting entity.
    pub actor: ActorId,
    /// Other entity.
    pub other: ActorId,
    /// Callback function index.
    pub function_index: usize,
}

/// Physics callback kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EnvCallbackKind {
    /// Blocked callback.
    Blocked,
    /// Touch callback.
    Touch,
}

/// Environmental damage (donor `Id1EnvironmentalDamage`).
#[derive(Debug, Clone, PartialEq)]
pub struct Id1EnvironmentalDamage {
    /// Damage cause.
    pub cause: DamageCause,
    /// Damage time.
    pub time: f64,
    /// Hit direction.
    pub direction: Vec3,
    /// Hit point.
    pub point: Vec3,
    /// Knockback.
    pub knockback: f64,
}

/// Environmental damage scope (donor `Id1Environment`).
pub struct Id1Environment<'a> {
    source: QcHostSource<'a>,
    machine: MachineFn<'a>,
    binding: Id1ProgramBinding,
}

impl<'a> Id1Environment<'a> {
    /// Bind environmental damage sites (donor `Id1Environment`
    /// constructor).
    pub fn new(source: QcHostSource<'a>, machine: MachineFn<'a>) -> Result<Self, QcError> {
        let binding = id1_program_snapshot(source.program)?;
        for site in &binding.environment {
            let caller = source.program.function_at(site.caller)?;
            let statement = source.program.statements.get(site.statement);
            if caller.name != site.name
                || statement.is_none_or(|statement| {
                    statement.opcode != QcOpcode::Call4
                        || usize::from(statement.a) != binding.damage.global
                        || statement.b != 0
                        || statement.c != 0
                })
            {
                return Err(QcError::program(
                    format!("id1 environmental statement {} mismatch", site.statement),
                    source.program.source,
                ));
            }
        }
        Ok(Self {
            source,
            machine,
            binding,
        })
    }

    /// Environmental damage scope (donor `resolve`).
    pub fn resolve(
        &self,
        call: &Id1DamageCall,
        callback: Option<&Id1PhysicsCallback>,
    ) -> Result<Option<Id1EnvironmentalDamage>, QcError> {
        let Some(site) = self
            .binding
            .environment
            .iter()
            .find(|site| site.caller == call.call.caller && site.statement == call.call.statement)
        else {
            return Ok(None);
        };
        let vm = (self.machine)();
        if vm.program_digest() != self.source.program.digest
            || vm.global_int(self.binding.damage.global)? != self.binding.damage.index as i32
            || call.call.function_index != self.binding.damage.index
        {
            return Err(QcError::program(
                "Unmatched id1 environmental machine or function",
                self.source.program.source,
            ));
        }
        let reference = |actor: &ActorId| -> Result<i32, QcError> {
            let source = self.source.actors.source_of(actor);
            match source {
                Some(source)
                    if self.source.actors.is_live(actor) && source.provider == self.source.slots.provider() =>
                {
                    vm.entity_reference(source.slot)
                }
                _ => Err(QcError::program(
                    "id1 environment references a foreign or stale actor",
                    self.source.program.source,
                )),
            }
        };
        let target = reference(&call.target)?;
        let inflictor = reference(&call.inflictor)?;
        let attacker = reference(&call.attacker)?;
        let current = vm.global_int(vm.global_offset("self")?)?;
        let other = vm.global_int(vm.global_offset("other")?)?;
        let field = |name: &str| -> Result<usize, QcError> {
            vm.field_offset(name).map_err(|_| {
                QcError::program(
                    format!("Missing environmental field {name}"),
                    self.source.program.source,
                )
            })
        };
        let expected_attacker = if site.attacker_goalentity {
            vm.entity_int(vm.entity_slot(inflictor)?, field("goalentity")?)?
        } else {
            inflictor
        };
        let text = |reference: i32, name: &str| -> Result<String, QcError> {
            vm.strings_get(vm.entity_int(vm.entity_slot(reference)?, field(name)?)?)
        };
        let mut cause = DamageCause::Environment { hazard: site.hazard };
        if let Some(native) = site.native {
            if vm.arg_int(0)? != target || vm.arg_int(1)? != inflictor || vm.arg_int(2)? != attacker {
                return Err(QcError::program(
                    "Native environmental arguments changed",
                    self.source.program.source,
                ));
            }
            if vm.arg_float(3)? != call.amount {
                return Err(QcError::program(
                    "Native environmental damage changed",
                    self.source.program.source,
                ));
            }
            let inflictor_slot = vm.entity_slot(inflictor)?;
            if native == NativeEnv::Barrel {
                if text(inflictor, "classname")? != "explo_box" {
                    return Ok(None);
                }
                if current != inflictor
                    || attacker != inflictor
                    || self.source.program.function_named("barrel_explode")?.index
                        != vm.entity_int(inflictor_slot, field("th_die")?)? as usize
                {
                    return Err(QcError::program(
                        "Unmatched native barrel damage",
                        self.source.program.source,
                    ));
                }
            } else {
                let Some(callback) = callback else {
                    return Err(QcError::program(
                        "Unmatched native map touch",
                        self.source.program.source,
                    ));
                };
                if callback.kind != EnvCallbackKind::Touch
                    || callback.function_index != site.caller
                    || callback.actor != call.inflictor
                    || reference(&callback.other)? != other
                    || current != inflictor
                {
                    return Err(QcError::program(
                        "Unmatched native map touch",
                        self.source.program.source,
                    ));
                }
                let owner = vm.entity_int(inflictor_slot, field("owner")?)?;
                if native == NativeEnv::Teledeath {
                    let classname = text(inflictor, "classname")?;
                    let expected_class = if site.statement == 9808 || site.statement == 9817 {
                        "teledeath3".to_string()
                    } else if site.statement == 9828 {
                        "teledeath2".to_string()
                    } else {
                        classname.clone()
                    };
                    let victim_matches = if site.statement == 9828 {
                        target == owner
                    } else if site.statement == 9817 {
                        let parameter = self.source.program.function_at(site.caller)?.parameter_start;
                        owner == other && target == vm.global_int(parameter)? && target != other && target != inflictor
                    } else {
                        target == other
                    };
                    if attacker != inflictor
                        || call.amount != 50_000.0
                        || !victim_matches
                        || classname != expected_class
                        || (classname != "teledeath" && classname != "teledeath2" && classname != "teledeath3")
                    {
                        return Err(QcError::program(
                            "Unmatched native teledeath branch",
                            self.source.program.source,
                        ));
                    }
                    cause = DamageCause::Q1 {
                        death_type: classname,
                        armor_effect: None,
                    };
                } else {
                    if target != other {
                        return Err(QcError::program(
                            "Unmatched native map victim",
                            self.source.program.source,
                        ));
                    }
                    if native == NativeEnv::Spike || native == NativeEnv::Laser {
                        let owner_class = text(owner, "classname")?;
                        if !owner_class.starts_with("trap_") {
                            return Ok(None);
                        }
                        let amount = if native == NativeEnv::Laser {
                            15.0
                        } else if site.statement == 3900 {
                            9.0
                        } else {
                            18.0
                        };
                        if attacker != owner || call.amount != amount {
                            return Err(QcError::program(
                                "Unmatched native trap owner or damage",
                                self.source.program.source,
                            ));
                        }
                    } else if native == NativeEnv::Fireball {
                        if text(inflictor, "classname")? != "fireball" || call.amount != 20.0 {
                            return Err(QcError::program(
                                "Unmatched native fireball",
                                self.source.program.source,
                            ));
                        }
                    } else if text(inflictor, "classname")? != "trigger_changelevel" || call.amount != 50_000.0 {
                        return Err(QcError::program(
                            "Unmatched native exit punishment",
                            self.source.program.source,
                        ));
                    }
                    cause = DamageCause::Q1 {
                        death_type: text(target, "deathtype")?,
                        armor_effect: None,
                    };
                }
            }
            if native == NativeEnv::Barrel {
                cause = DamageCause::Q1 {
                    death_type: text(target, "deathtype")?,
                    armor_effect: None,
                };
            }
        } else if site.context == EnvContext::World {
            if target != current || inflictor != 0 || attacker != 0 || vm.global_int(vm.global_offset("world")?)? != 0 {
                return Err(QcError::program(
                    "Unmatched id1 world hazard arguments",
                    self.source.program.source,
                ));
            }
        } else {
            let context_matches = matches!(
                (callback.map(|callback| callback.kind), site.context),
                (Some(EnvCallbackKind::Blocked), EnvContext::Blocked)
                    | (Some(EnvCallbackKind::Touch), EnvContext::Touch)
            );
            if callback.is_none()
                || !context_matches
                || callback.is_some_and(|callback| {
                    callback.function_index != site.caller
                        || call.target != callback.other
                        || call.inflictor != callback.actor
                })
                || current != inflictor
                || other != target
                || attacker != expected_attacker
            {
                return Err(QcError::program(
                    "Unmatched id1 environmental callback",
                    self.source.program.source,
                ));
            }
        }
        let victim = vm.entity_slot(target)?;
        let time = vm.global_float(vm.global_offset("time")?)?;
        let point = vm.entity_vector(victim, field("origin")?)?;
        let mut direction = Vec3 { x: 0.0, y: 0.0, z: 0.0 };
        let mut knockback = 0.0;
        if inflictor != 0 && vm.entity_float(vm.entity_slot(inflictor)?, field("movetype")?)? == 3.0 {
            let numeric = vm.numeric();
            let min = vm.entity_vector(vm.entity_slot(inflictor)?, field("absmin")?)?;
            let max = vm.entity_vector(vm.entity_slot(inflictor)?, field("absmax")?)?;
            direction = vector_add(numeric, min, max);
            direction = vector_scale(numeric, direction, 0.5);
            direction = vector_sub(numeric, point, direction);
            vector_normalize(numeric, &mut direction);
            knockback = numeric.mul(
                call.amount,
                id1_damage_multiplier(self.source.program, vm, attacker, inflictor)?,
            );
        }
        Ok(Some(Id1EnvironmentalDamage {
            cause,
            time,
            direction,
            point,
            knockback,
        }))
    }
}

/// Vector add with binary32 stores (donor `VectorAdd` plus stores).
fn vector_add(numeric: NumericOps, left: Vec3, right: Vec3) -> Vec3 {
    Vec3 {
        x: numeric.store(numeric.add(f64::from(left.x), f64::from(right.x))),
        y: numeric.store(numeric.add(f64::from(left.y), f64::from(right.y))),
        z: numeric.store(numeric.add(f64::from(left.z), f64::from(right.z))),
    }
}

/// Vector scale with binary32 stores (donor `VectorScale` plus stores).
fn vector_scale(numeric: NumericOps, value: Vec3, scale: f64) -> Vec3 {
    Vec3 {
        x: numeric.store(numeric.mul(f64::from(value.x), scale)),
        y: numeric.store(numeric.mul(f64::from(value.y), scale)),
        z: numeric.store(numeric.mul(f64::from(value.z), scale)),
    }
}

/// Vector subtract with binary32 stores (donor `VectorSubtract` plus
/// stores).
fn vector_sub(numeric: NumericOps, left: Vec3, right: Vec3) -> Vec3 {
    Vec3 {
        x: numeric.store(numeric.sub(f64::from(left.x), f64::from(right.x))),
        y: numeric.store(numeric.sub(f64::from(left.y), f64::from(right.y))),
        z: numeric.store(numeric.sub(f64::from(left.z), f64::from(right.z))),
    }
}

/// Normalize with binary32 stores (donor `VectorNormalize` plus
/// stores).
fn vector_normalize(numeric: NumericOps, value: &mut Vec3) {
    let dot = |edge: Vec3| -> f64 {
        numeric.add(
            numeric.add(
                numeric.mul(f64::from(edge.x), f64::from(edge.x)),
                numeric.mul(f64::from(edge.y), f64::from(edge.y)),
            ),
            numeric.mul(f64::from(edge.z), f64::from(edge.z)),
        )
    };
    let length = numeric.sqrt(dot(*value));
    let donor = matches!(numeric.profile.arithmetic, Arithmetic::DonorBinary64(_));
    if donor {
        if length == 0.0 || length.is_nan() {
            return;
        }
    } else if length == 0.0 {
        return;
    }
    let inverse = numeric.div(1.0, length);
    *value = vector_scale(numeric, *value, inverse);
}
