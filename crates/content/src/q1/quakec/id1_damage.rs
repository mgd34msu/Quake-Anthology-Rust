//! id1 damage bindings (`src/content/q1/quakec/id1-damage.ts`).
//!
//! Donor provenance: `src/content/q1/quakec/id1-damage.ts`
//! (`Id1DamageBinding`, `Id1DamageCall`, `Id1DamageProjection`).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use qa_core::identity::{ActorId, OwnedActor, ProviderId};
use qa_core::math::Vec3;
use qa_core::numeric::float_to_wrapped_i32;

use crate::contract::{
    ArmorState, ModQcArmorStage, ModQcDamageFlags, ModQcDamageScale, ModSourceCall, ProtectionChannel,
    RegularArmorState,
};

use super::super::foundation::gameplay::{DamageOutcome, DamageReaction, DamageRequest};
use super::armor_stage::{qc_armor_stage, QcArmorStage};
use super::damage_call::{project_qc_damage_call, read_qc_damage_call, QcDamageCallValues};
use super::damage_scale::{evaluate_qc_damage_amount, evaluate_qc_damage_scale, qc_damage_scale, QcDamageScale};
use super::id1_program::{id1_program_binding, Id1Attribution, Id1DamageKind, Id1ProgramBinding, Id1ProgramCache};
use super::qc_gameplay::{
    attack_damage_flags, ActorSource, ArmorDamageFlags, ArmorStageInput, DamageGeometry, QcActorRegistry,
    SourceDamageObserver, SourceDamageResult, SourceStoredMutation,
};
use super::qc_view::{
    ArmorIntercept, GameplayAuthority, MachineFn, QcCallSite, QcEntityStoreObservation, QcFunctionBoundary,
    QcFunctionExecution, QcHostSource, QcInlineBoundary, QcInlineContinuation, QcInlineRegion, QcMachineView, QcOpcode,
    QcWordsBuf, SourceArmorStage,
};
use super::{float_identical, fround, QcError};

/// Validated source damage call (donor `Id1DamageCall`).
#[derive(Debug, Clone, PartialEq)]
pub struct Id1DamageCall {
    /// Call site.
    pub call: QcCallSite,
    /// Target actor.
    pub target: ActorId,
    /// Inflicting actor.
    pub inflictor: ActorId,
    /// Attacking actor.
    pub attacker: ActorId,
    /// Damage amount.
    pub amount: f64,
}

/// Damage projection (donor `Id1DamageProjection`).
pub trait Id1DamageProjection {
    /// Admit a composed request.
    fn admit(&self, _request: &DamageRequest) -> bool {
        true
    }
    /// Actor for a reference.
    fn actor(&self, reference: i32) -> Result<ActorId, QcError>;
    /// Reference for an actor.
    fn reference(&self, actor: Option<&ActorId>) -> Result<i32, QcError>;
    /// Whether a reaction continuation is provided (donor
    /// `projection.reaction !== undefined`).
    fn has_reaction(&self) -> bool {
        false
    }
    /// Run a damage reaction.
    fn reaction(
        &self,
        request: &DamageRequest,
        result: &SourceDamageResult,
        execute: &dyn QcFunctionExecution,
    ) -> Result<(), QcError> {
        let _ = (request, result);
        execute.run(None)
    }
    /// Observe a completed outcome.
    fn completed(&self, _request: &DamageRequest, _outcome: &DamageOutcome) {}
}

/// Quad-damage predicate region with its result word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct QuadRegion {
    region: QcInlineRegion,
    result: usize,
}

/// Active damage frame (donor `Id1DamageBinding.active` entry).
struct DamageFrame {
    request: DamageRequest,
    target_reference: i32,
    observer: Rc<dyn SourceDamageObserver>,
    movement_provider: ProviderId,
    cancel_owner: u64,
    regular_scale: f64,
    result: SourceDamageResult,
    reaction_depth: usize,
    health_written: bool,
}

/// Observes validated source damage operations inside the shared
/// authority (donor `Id1DamageBinding`).
pub struct Id1DamageBinding<'a> {
    source: QcHostSource<'a>,
    authority: &'a dyn GameplayAuthority,
    machine: MachineFn<'a>,
    resolve_request: Box<dyn Fn(&Id1DamageCall) -> DamageRequest + 'a>,
    projection: Option<&'a dyn Id1DamageProjection>,
    damage_scale: Option<QcDamageScale>,
    armor_stage: Option<QcArmorStage>,
    quad: Option<QuadRegion>,
    binding: Id1ProgramBinding,
    protection_regular: Rc<RefCell<HashMap<ActorId, Rc<ArmorIntercept>>>>,
    protection_powered: Rc<RefCell<HashMap<ActorId, Rc<ArmorIntercept>>>>,
    active: RefCell<Vec<DamageFrame>>,
    health: usize,
    velocity: usize,
    armor_value: usize,
    armor_type: usize,
    items: usize,
    pain: usize,
    die: usize,
}

impl<'a> Id1DamageBinding<'a> {
    /// Bind damage for a program (donor `Id1DamageBinding` constructor).
    ///
    /// The program cache is shared with the caller so a damage call
    /// qualified by `id1_program_binding` is picked up here, matching
    /// the donor's module-global derivation table.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        source: QcHostSource<'a>,
        authority: &'a dyn GameplayAuthority,
        machine: MachineFn<'a>,
        resolve_request: Box<dyn Fn(&Id1DamageCall) -> DamageRequest + 'a>,
        projection: Option<&'a dyn Id1DamageProjection>,
        declared_armor: Option<&ModQcArmorStage>,
        declared_damage: Option<&ModSourceCall>,
        declared_scale: Option<(&ModSourceCall, &ModQcDamageScale)>,
        cache: &mut Id1ProgramCache,
    ) -> Result<Self, QcError> {
        let binding = id1_program_binding(cache, source.program, declared_damage)?;
        let armor_stage = qc_armor_stage(source.program, declared_armor)?;
        let damage_scale = declared_scale
            .map(|(call, scale)| qc_damage_scale(source.program, call, Some(scale)))
            .transpose()?
            .flatten();
        let program = source.program;
        let quad_quakeworld =
            program.digest == "sha256:ff51cb5e77360d72b93487d89198dcf94629b92f8bae100fc6ea48a6c12a7830";
        let quad_netquake = program.digest == "sha256:f2619787f9aa0f057246eea1665b622b4691b5c5a800b1a46133d1fe8b771580";
        let quad = if damage_scale.is_some() {
            None
        } else if quad_quakeworld {
            Some(QuadRegion {
                region: QcInlineRegion {
                    function_index: 83,
                    entry: 364,
                    exit: 369,
                    replaceable: false,
                    standalone: None,
                },
                result: 875,
            })
        } else if quad_netquake {
            Some(QuadRegion {
                region: QcInlineRegion {
                    function_index: 117,
                    entry: 1426,
                    exit: 1428,
                    replaceable: false,
                    standalone: None,
                },
                result: 1593,
            })
        } else {
            None
        };
        if quad.is_some() {
            let predicate: &[(usize, QcOpcode, u16, u16, u16)] = if quad_quakeworld {
                &[
                    (364, QcOpcode::LoadF, 857, 396, 870),
                    (365, QcOpcode::Gt, 870, 31, 871),
                    (366, QcOpcode::LoadS, 856, 124, 872),
                    (367, QcOpcode::NeS, 872, 873, 874),
                    (368, QcOpcode::And, 871, 874, 875),
                    (369, QcOpcode::IfNot, 875, 8, 0),
                ]
            } else {
                &[
                    (1426, QcOpcode::LoadF, 1582, 377, 1592),
                    (1427, QcOpcode::Gt, 1592, 31, 1593),
                    (1428, QcOpcode::IfNot, 1593, 3, 0),
                ]
            };
            for (index, opcode, a, b, c) in predicate {
                let actual = program.statements.get(*index);
                if actual
                    .is_none_or(|actual| actual.opcode != *opcode || actual.a != *a || actual.b != *b || actual.c != *c)
                {
                    return Err(QcError::program(
                        "QC source Quad predicate differs from its original artifact",
                        program.source,
                    ));
                }
            }
        }
        let layout = &binding.damage;
        let damage = program.function_at(layout.index)?.clone();
        if damage.index != layout.index
            || damage.first_statement != layout.first_statement
            || damage.parameter_start != layout.parameter_start
            || damage.local_words != layout.local_words
            || matches!(layout.kind, Id1DamageKind::Sites { .. })
                && (damage.parameter_sizes.len() != 4 || damage.parameter_sizes.iter().any(|size| *size != 1))
        {
            return Err(QcError::program("id1 damage function layout mismatch", program.source));
        }
        if let Id1DamageKind::Sites { statements, .. } = &layout.kind {
            for (index, opcode, a, b, c) in statements {
                let value = program.statements.get(*index);
                if value.is_none_or(|value| value.opcode != *opcode || value.a != *a || value.b != *b || value.c != *c)
                {
                    return Err(QcError::program(
                        format!("id1 damage statement {index} mismatch"),
                        program.source,
                    ));
                }
            }
        }
        let field = |name: &str| -> Result<usize, QcError> {
            program
                .field_named(name)
                .map(|field| field.offset)
                .ok_or_else(|| QcError::program(format!("missing id1 field {name}"), program.source))
        };
        Ok(Self {
            health: field("health")?,
            velocity: field("velocity")?,
            armor_value: field("armorvalue")?,
            armor_type: field("armortype")?,
            items: field(&binding.armor_field)?,
            pain: field("th_pain")?,
            die: field("th_die")?,
            source,
            authority,
            machine,
            resolve_request,
            projection,
            damage_scale,
            armor_stage,
            quad,
            binding,
            protection_regular: Rc::new(RefCell::new(HashMap::new())),
            protection_powered: Rc::new(RefCell::new(HashMap::new())),
            active: RefCell::new(Vec::new()),
        })
    }

    /// Machine accessor with ownership check (donor `vm()`).
    fn vm(&self) -> Result<&'a dyn QcMachineView, QcError> {
        let vm = (self.machine)();
        if vm.program_digest() != self.source.program.digest {
            return Err(QcError::program(
                "id1 damage binding belongs to another machine",
                self.source.program.source,
            ));
        }
        Ok(vm)
    }

    /// Function boundary (donor `functionBoundary`).
    pub fn function_boundary(&'a self) -> QcFunctionBoundary<'a> {
        let layout = &self.binding.damage;
        let narrow = matches!(layout.kind, Id1DamageKind::Sites { .. })
            && !self.projection.is_some_and(|projection| projection.has_reaction());
        let functions = if narrow {
            std::collections::HashSet::from([layout.index])
        } else {
            self.source
                .program
                .functions
                .iter()
                .filter(|function| function.index > 0 && function.first_statement > 0 && !function.named_builtin)
                .map(|function| function.index)
                .collect()
        };
        QcFunctionBoundary {
            functions,
            run: Box::new(move |call, execute| self.run_function(call, execute)),
        }
    }

    /// Inline boundary (donor `inlineBoundary`).
    pub fn inline_boundary(&'a self) -> QcInlineBoundary<'a> {
        let mut regions = Vec::new();
        if let Some(scale) = &self.damage_scale {
            regions.push(scale.region);
        }
        if let Some(quad) = &self.quad {
            regions.push(quad.region);
        }
        if let Some(stage) = &self.armor_stage {
            regions.push(stage.region);
        }
        QcInlineBoundary {
            regions,
            run: Box::new(move |region, execute| self.run_inline(region, execute)),
        }
    }

    /// Function dispatch.
    fn run_function(&self, call: &QcCallSite, execute: &dyn QcFunctionExecution) -> Result<(), QcError> {
        if call.function_index != self.binding.damage.index {
            return self.observe_native_function(call, execute);
        }
        let vm = self.vm()?;
        let source_call = read_qc_damage_call(vm, &self.binding.damage.call)?;
        let actor = |reference: i32| -> Result<ActorId, QcError> {
            if let Some(projection) = self.projection {
                return projection.actor(reference);
            }
            let slot = vm.entity_slot(reference)?;
            let value = self.source.slots.at(slot);
            match value {
                Some(value) if self.source.actors.is_live(value.id()) => Ok(value.id().clone()),
                _ => Err(QcError::program(
                    "id1 damage references a free source actor",
                    self.source.program.source,
                )),
            }
        };
        let captured = Id1DamageCall {
            call: *call,
            target: actor(source_call.self_reference)?,
            inflictor: actor(source_call.inflictor)?,
            attacker: actor(source_call.attacker)?,
            amount: source_call.amount,
        };
        let request = (self.resolve_request)(&captured);
        let same_reference = |actor: Option<&ActorId>, captured: &ActorId, reference: i32| -> bool {
            match actor {
                None => reference == 0,
                Some(actor) => actor == captured,
            }
        };
        if request.target != captured.target
            || fround(request.amount) != captured.amount
            || !same_reference(
                request.attack.attacker.as_ref(),
                &captured.attacker,
                source_call.attacker,
            )
            || !same_reference(
                request.attack.inflictor.as_ref(),
                &captured.inflictor,
                source_call.inflictor,
            )
        {
            return Err(QcError::program(
                "id1 damage provenance changed source arguments",
                self.source.program.source,
            ));
        }
        let executed = Cell::new(false);
        let outer = request.clone();
        let outcome = self.authority.apply(
            request,
            Some(&mut |composed| {
                self.authority.run_source_damage(composed, &mut |observer, effective| {
                    self.run_source_damage(
                        vm,
                        &captured,
                        &outer,
                        &source_call,
                        observer,
                        effective,
                        execute,
                        &executed,
                    )
                })
            }),
        )?;
        if let Some(projection) = self.projection {
            projection.completed(&outer, &outcome);
        }
        if !executed.get() {
            execute.skip([0, 0, 0]);
        }
        Ok(())
    }

    /// Source damage execution.
    #[allow(clippy::too_many_arguments)]
    fn run_source_damage(
        &self,
        vm: &dyn QcMachineView,
        captured: &Id1DamageCall,
        outer: &DamageRequest,
        source_call: &QcDamageCallValues,
        observer: &Rc<dyn SourceDamageObserver>,
        effective: DamageRequest,
        execute: &dyn QcFunctionExecution,
        executed: &Cell<bool>,
    ) -> Result<SourceDamageResult, QcError> {
        if self.projection.is_some_and(|projection| !projection.admit(&effective))
            || !self.source.actors.is_live(&effective.target)
        {
            return Ok(SourceDamageResult {
                applied_damage: 0.0,
                reaction: DamageReaction::None,
            });
        }
        if self.authority.damage_operation_active() && !damage_metadata_equal(outer, &effective) {
            return Err(QcError::program(
                "QuakeC damage call accepts actor and amount changes; independent damage metadata requires a replacement",
                self.source.program.source,
            ));
        }
        if !fround(effective.amount).is_finite() {
            return Err(QcError::program(
                "QuakeC damage amount exceeds binary32 range",
                self.source.program.source,
            ));
        }
        let reference_for = |actor: Option<&ActorId>| -> Result<i32, QcError> {
            if let Some(projection) = self.projection {
                return projection.reference(actor);
            }
            match actor {
                None => vm.entity_reference(0),
                Some(actor) => {
                    let slot = self.source.actors.source_of(actor);
                    match slot {
                        Some(ActorSource { provider, slot }) if provider == self.source.slots.provider() => {
                            vm.entity_reference(slot)
                        }
                        _ => Err(QcError::program(
                            "QuakeC damage actor has no source projection",
                            self.source.program.source,
                        )),
                    }
                }
            }
        };
        let target_reference = reference_for(Some(&effective.target))?;
        let inflictor_reference = reference_for(effective.attack.inflictor.as_ref())?;
        let attacker_reference = reference_for(effective.attack.attacker.as_ref())?;
        let mut regular_scale = 1.0;
        if let Some(stage) = &self.armor_stage {
            for site in &stage.stage.regular_scale {
                if site.statement as usize == captured.call.statement
                    && self.source.program.function_named(&site.caller)?.index == captured.call.caller
                {
                    regular_scale = site.scale;
                    break;
                }
            }
        }
        self.active.borrow_mut().push(DamageFrame {
            request: effective.clone(),
            target_reference,
            observer: Rc::clone(observer),
            movement_provider: effective.attack.movement_provider.clone(),
            cancel_owner: execute.cancel_owner(),
            regular_scale,
            result: SourceDamageResult {
                applied_damage: 0.0,
                reaction: DamageReaction::None,
            },
            reaction_depth: 0,
            health_written: false,
        });
        let result = (|| -> Result<SourceDamageResult, QcError> {
            executed.set(true);
            project_qc_damage_call(
                vm,
                &self.binding.damage.call,
                source_call,
                &QcDamageCallValues {
                    self_reference: target_reference,
                    inflictor: inflictor_reference,
                    attacker: attacker_reference,
                    amount: effective.amount,
                },
                execute,
            )?;
            if matches!(self.binding.damage.kind, Id1DamageKind::Calls { .. }) {
                let active = self.active.borrow();
                let frame = active
                    .last()
                    .ok_or_else(|| QcError::program("missing damage frame", self.source.program.source))?;
                if frame.result.reaction == DamageReaction::None
                    && frame.health_written
                    && self.source.actors.is_live(&effective.target)
                    && vm.entity_float(vm.entity_slot(target_reference)?, self.health)? <= 0.0
                {
                    let result = SourceDamageResult {
                        applied_damage: frame.result.applied_damage,
                        reaction: DamageReaction::Death,
                    };
                    let observer = Rc::clone(&frame.observer);
                    drop(active);
                    if let Some(frame) = self.active.borrow_mut().last_mut() {
                        frame.result = result;
                    }
                    observer.before_reaction(&result);
                    return Ok(result);
                }
            }
            Ok(self
                .active
                .borrow()
                .last()
                .map(|frame| frame.result)
                .unwrap_or(SourceDamageResult {
                    applied_damage: 0.0,
                    reaction: DamageReaction::None,
                }))
        })();
        self.active.borrow_mut().pop();
        result
    }

    /// Inline dispatch.
    fn run_inline(&self, region: &QcInlineRegion, execute: &dyn QcInlineContinuation) -> Result<(), QcError> {
        if self
            .damage_scale
            .as_ref()
            .is_some_and(|scale| region.entry == scale.region.entry)
        {
            let skip = self.active.borrow().last().is_some_and(|frame| {
                frame.request.attack.damage_powerup_owner.as_ref() == Some(&self.source.slots.provider())
            });
            return if skip { execute.skip_to_join() } else { execute.run() };
        }
        if self.quad.is_none_or(|quad| region.entry != quad.region.entry) {
            return self.run_armor(execute);
        }
        let suppress = self
            .active
            .borrow()
            .last()
            .is_some_and(|frame| frame.request.attack.damage_powerup_owner.is_some());
        execute.run()?;
        if suppress {
            if let Some(quad) = self.quad {
                self.vm()?.set_global_float(quad.result, 0.0)?;
            }
        }
        Ok(())
    }

    /// Damage multiplier query (donor `damageMultiplier`).
    pub fn damage_multiplier(&self, actor: &ActorId, reference: i32, seconds: f64) -> Result<Option<f64>, QcError> {
        match &self.damage_scale {
            None => Ok(None),
            Some(scale) => evaluate_qc_damage_scale(self.vm()?, scale, actor, reference, seconds).map(Some),
        }
    }

    /// Damage amount query (donor `damageAmount`).
    pub fn damage_amount(
        &self,
        actor: &ActorId,
        reference: i32,
        seconds: f64,
        amount: f64,
    ) -> Result<Option<f64>, QcError> {
        match &self.damage_scale {
            None => Ok(None),
            Some(scale) => evaluate_qc_damage_amount(self.vm()?, scale, actor, reference, seconds, amount).map(Some),
        }
    }

    /// Protection stage for a channel (donor `protectionStage`).
    pub fn protection_stage(&self, actor: &OwnedActor, channel: ProtectionChannel) -> Option<QcArmorStageHandle<'_>> {
        let replaceable = self.armor_stage.as_ref().is_some_and(|stage| stage.region.replaceable);
        if self.armor_stage.is_none() || channel == ProtectionChannel::Regular && !replaceable {
            return None;
        }
        let owners = match channel {
            ProtectionChannel::Regular => Rc::clone(&self.protection_regular),
            ProtectionChannel::Powered => Rc::clone(&self.protection_powered),
        };
        Some(QcArmorStageHandle {
            actor: actor.clone(),
            channel,
            owners,
            actors: self.source.actors,
        })
    }

    /// Armor interception (donor `runArmor`).
    fn run_armor(&self, execute: &dyn QcInlineContinuation) -> Result<(), QcError> {
        let frame = self.active.borrow().last().map(|frame| {
            (
                frame.request.clone(),
                frame.target_reference,
                frame.cancel_owner,
                frame.regular_scale,
                frame.reaction_depth,
            )
        });
        let (request, target_reference, cancel_owner, regular_scale, reaction_depth) = match frame {
            Some(frame) => frame,
            None => return execute.run(),
        };
        let Some(plan) = self.armor_stage.as_ref() else {
            return execute.run();
        };
        if reaction_depth > 0 {
            return execute.run();
        }
        let vm = self.vm()?;
        let Some(actor) = self.source.actors.resolve_owned(&request.target) else {
            return Err(QcError::cancelled(cancel_owner, [0, 0, 0]));
        };
        let target_word = plan.stage.target as usize;
        let damage_word = plan.stage.damage as usize;
        let saved_word = plan.stage.saved as usize;
        if vm.global_int(target_word)? != target_reference {
            return Err(QcError::program(
                "QC armor stage changed its damage target",
                self.source.program.source,
            ));
        }
        let power = self.protection_powered.borrow().get(actor.id()).cloned();
        let regular = self.protection_regular.borrow().get(actor.id()).cloned();
        if power.is_none() && regular.is_none() {
            return execute.run();
        }
        let live = || -> Result<(), QcError> {
            if self.source.actors.resolve_owned(actor.id()).as_ref() != Some(&actor) {
                return Err(QcError::cancelled(cancel_owner, [0, 0, 0]));
            }
            Ok(())
        };
        let word = match plan.stage.flags {
            ModQcDamageFlags::None => 0,
            ModQcDamageFlags::Bits { word, .. } => float_to_wrapped_i32(vm.global_float(word as usize)?),
        };
        let captured = attack_damage_flags(&request);
        let numeric = vm.numeric();
        let originating = ArmorDamageFlags {
            regular_protection_scale: Some(
                numeric.mul(captured.armor.regular_protection_scale.unwrap_or(1.0), regular_scale),
            ),
            ..captured.armor
        };
        let flags = match plan.stage.flags {
            ModQcDamageFlags::None => originating,
            ModQcDamageFlags::Bits {
                no_armor,
                no_power_armor,
                no_regular_armor,
                energy,
                ..
            } => {
                #[allow(clippy::cast_possible_wrap)]
                let bit = |mask: u32| mask as i32;
                ArmorDamageFlags {
                    no_armor: if no_armor == 0 {
                        originating.no_armor
                    } else {
                        word & bit(no_armor) != 0
                    },
                    no_power_armor: if no_power_armor == 0 {
                        originating.no_power_armor
                    } else {
                        word & bit(no_power_armor) != 0
                    },
                    no_regular_armor: if no_regular_armor == 0 {
                        originating.no_regular_armor
                    } else {
                        word & bit(no_regular_armor) != 0
                    },
                    energy: if energy == 0 {
                        originating.energy
                    } else {
                        word & bit(energy) != 0
                    },
                    ..originating
                }
            }
        };
        let input = ArmorStageInput {
            request: request.clone(),
            amount: vm.global_float(damage_word)?,
            flags,
            geometry: DamageGeometry {
                direction: request.direction,
                point: request.point,
                normal: request.normal,
            },
        };
        let power_saved = power
            .map(|power| power(&input, &|| Ok(0.0)))
            .transpose()?
            .unwrap_or(0.0);
        live()?;
        if !fround(power_saved).is_finite() {
            return Err(QcError::program(
                "Armor savings exceed QC binary32 range",
                self.source.program.source,
            ));
        }
        let original = vm.global_int(damage_word)?;
        let result = (|| -> Result<(), QcError> {
            let amount = numeric.sub(input.amount, power_saved);
            vm.set_global_float(damage_word, amount)?;
            let executed = Cell::new(false);
            let original_regular = || -> Result<f64, QcError> {
                execute.run()?;
                executed.set(true);
                vm.global_float(saved_word)
            };
            // Power may remove the regular owner while this frame is suspended.
            let current = self.protection_regular.borrow().get(actor.id()).cloned();
            let regular_input = ArmorStageInput {
                request: request.clone(),
                amount,
                flags: input.flags,
                geometry: DamageGeometry {
                    direction: request.direction,
                    point: request.point,
                    normal: request.normal,
                },
            };
            let regular_saved = match current {
                None => original_regular()?,
                Some(current) => current(&regular_input, &original_regular)?,
            };
            live()?;
            if !fround(regular_saved).is_finite() {
                return Err(QcError::program(
                    "Armor savings exceed QC binary32 range",
                    self.source.program.source,
                ));
            }
            if !executed.get() {
                execute.skip_to_join()?;
            }
            vm.set_global_float(saved_word, numeric.add(regular_saved, power_saved))?;
            Ok(())
        })();
        vm.set_global_int(damage_word, original)?;
        result
    }

    /// Native function observation (donor `observeNativeFunction`).
    fn observe_native_function(&self, call: &QcCallSite, execute: &dyn QcFunctionExecution) -> Result<(), QcError> {
        let frame = self.active.borrow().last().map(|frame| {
            (
                frame.request.clone(),
                frame.target_reference,
                frame.result,
                frame.reaction_depth,
                frame.health_written,
                Rc::clone(&frame.observer),
            )
        });
        let Some((request, target_reference, result, reaction_depth, health_written, observer)) = frame else {
            return execute.run(None);
        };
        if reaction_depth > 0 {
            return execute.run(None);
        }
        let layout = &self.binding.damage;
        if let Id1DamageKind::Sites { death, pain, .. } = &layout.kind {
            if result.reaction == DamageReaction::None
                || call.caller != layout.index
                || call.statement
                    != (if result.reaction == DamageReaction::Death {
                        death.0
                    } else {
                        pain.0
                    })
            {
                return execute.run(None);
            }
            self.bump_reaction_depth(1);
            let outcome = match self.projection {
                Some(projection) if projection.has_reaction() => projection.reaction(&request, &result, execute),
                _ => execute.run(None),
            };
            self.bump_reaction_depth(-1);
            return outcome;
        }
        if result.reaction != DamageReaction::None {
            return execute.run(None);
        }
        let vm = self.vm()?;
        let reactions = match &layout.kind {
            Id1DamageKind::Calls { reactions } => reactions,
            Id1DamageKind::Sites { .. } => return execute.run(None),
        };
        let Some(reaction) = reactions.get(&call.statement) else {
            return execute.run(None);
        };
        let slot = vm.entity_slot(target_reference)?;
        let hook = if *reaction == DamageReaction::Death {
            self.die
        } else {
            self.pain
        };
        if vm.global_int(vm.global_offset("self")?)? != target_reference
            || call.function_index != vm.entity_int(slot, hook)? as usize
        {
            return execute.run(None);
        }
        if (vm.entity_float(slot, self.health)? <= 0.0) != (*reaction == DamageReaction::Death) {
            return Err(QcError::program(
                "Native damage reaction disagrees with target health",
                self.source.program.source,
            ));
        }
        if !health_written {
            return Err(QcError::program(
                "Native damage reaction precedes target health mutation",
                self.source.program.source,
            ));
        }
        let updated = SourceDamageResult {
            applied_damage: result.applied_damage,
            reaction: *reaction,
        };
        self.set_frame_result(updated);
        observer.before_reaction(&updated);
        self.bump_reaction_depth(1);
        let outcome = match self.projection {
            Some(projection) if projection.has_reaction() => projection.reaction(&request, &updated, execute),
            _ => execute.run(None),
        };
        self.bump_reaction_depth(-1);
        outcome
    }

    /// Adjust the top reaction depth.
    fn bump_reaction_depth(&self, delta: i32) {
        if let Some(frame) = self.active.borrow_mut().last_mut() {
            if delta >= 0 {
                frame.reaction_depth = frame.reaction_depth.saturating_add(delta as usize);
            } else {
                frame.reaction_depth = frame.reaction_depth.saturating_sub((-delta) as usize);
            }
        }
    }

    /// Replace the top frame result.
    fn set_frame_result(&self, result: SourceDamageResult) {
        if let Some(frame) = self.active.borrow_mut().last_mut() {
            frame.result = result;
        }
    }

    /// Read armor words (donor `readArmor`).
    pub fn read_armor(&self, words: &QcWordsBuf) -> Result<ArmorState, QcError> {
        let items = float_to_wrapped_i32(words.float(self.items)?);
        let [green, yellow, red] = self.binding.armor_masks;
        let item = if items & red != 0 {
            Some("q1:item_armorInv")
        } else if items & yellow != 0 {
            Some("q1:item_armor2")
        } else if items & green != 0 {
            Some("q1:item_armor1")
        } else {
            None
        };
        Ok(ArmorState {
            regular: match item {
                None => RegularArmorState::None,
                Some(item) => RegularArmorState::Q1 {
                    points: words.float(self.armor_value)?,
                    absorption: words.float(self.armor_type)?,
                    item: item.to_string(),
                },
            },
            powered: crate::contract::PoweredProtectionState::None,
        })
    }

    /// Observe a source call (donor `observeCall`).
    pub fn observe_call(&self, call: &QcCallSite) -> Result<(), QcError> {
        if matches!(self.binding.damage.kind, Id1DamageKind::Calls { .. }) {
            return Ok(());
        }
        let frame = self
            .active
            .borrow()
            .last()
            .map(|frame| (frame.target_reference, frame.result, Rc::clone(&frame.observer)));
        let layout = &self.binding.damage;
        let (death, pain, take) = match &layout.kind {
            Id1DamageKind::Sites { death, pain, take, .. } => (*death, *pain, *take),
            Id1DamageKind::Calls { .. } => return Ok(()),
        };
        let Some((target_reference, current, observer)) = frame else {
            return Ok(());
        };
        if call.caller != layout.index || (call.statement != death.0 && call.statement != pain.0) {
            return Ok(());
        }
        let vm = self.vm()?;
        if self.binding.attribution == Id1Attribution::Native {
            let slot = vm.entity_slot(target_reference)?;
            let death_call = call.statement == death.0;
            let bad = if death_call {
                vm.arg_int(0)? != target_reference || vm.entity_float(slot, self.health)? > 0.0
            } else {
                vm.global_int(vm.global_offset("self")?)? != target_reference
                    || vm.entity_float(slot, self.health)? <= 0.0
                    || !float_identical(vm.arg_float(1)?, current.applied_damage)
            };
            if bad {
                return Err(QcError::program(
                    "Unsupported native damage reaction context",
                    self.source.program.source,
                ));
            }
        }
        let reaction_word = if call.statement == death.0 { death.1 } else { pain.1 };
        if call.function_index != vm.global_int(reaction_word)? as usize {
            return Ok(());
        }
        let result = SourceDamageResult {
            applied_damage: if self.binding.attribution == Id1Attribution::Native {
                current.applied_damage
            } else {
                vm.global_float(take)?
            },
            reaction: if call.statement == death.0 {
                DamageReaction::Death
            } else {
                DamageReaction::Pain
            },
        };
        self.set_frame_result(result);
        observer.before_reaction(&result);
        Ok(())
    }

    /// Observe an entity store (donor `observeEntityStore`).
    pub fn observe_entity_store(&self, store: &QcEntityStoreObservation) -> Result<(), QcError> {
        let frame = self.active.borrow().last().map(|frame| {
            (
                frame.target_reference,
                frame.result,
                frame.reaction_depth,
                frame.movement_provider.clone(),
                Rc::clone(&frame.observer),
            )
        });
        let Some((target_reference, result, reaction_depth, movement_provider, observer)) = frame else {
            return Ok(());
        };
        let layout = &self.binding.damage;
        let native = matches!(layout.kind, Id1DamageKind::Calls { .. });
        let combat_store = [
            self.health,
            self.velocity,
            self.armor_value,
            self.armor_type,
            self.items,
        ]
        .contains(&store.word);
        if native && reaction_depth > 0 {
            return Ok(());
        }
        if native && combat_store && store.reference != target_reference {
            return Err(QcError::program(
                "Unsupported native damage redirects a combat store to another actor",
                self.source.program.source,
            ));
        }
        if store.reference != target_reference {
            return Ok(());
        }
        if native && combat_store && result.reaction != DamageReaction::None {
            return Err(QcError::program(
                "Native damage combat store follows its reaction continuation",
                self.source.program.source,
            ));
        }
        if !native && store.function_index != layout.index {
            return Ok(());
        }
        let before_f32 = |bytes: &[u8]| -> Result<f64, QcError> {
            bytes
                .get(0..4)
                .and_then(|bytes| bytes.try_into().ok())
                .map(|bytes: [u8; 4]| f64::from(f32::from_le_bytes(bytes)))
                .ok_or_else(|| QcError::program("short entity store observation", self.source.program.source))
        };
        if store.word == self.health {
            if let Id1DamageKind::Sites { health_store, .. } = &layout.kind {
                if store.statement != *health_store {
                    return Err(QcError::program(
                        "Unsupported native damage health store",
                        self.source.program.source,
                    ));
                }
            }
            let before = before_f32(&store.before)?;
            let after = before_f32(&store.after)?;
            observer.stored(SourceStoredMutation::Health { before, after });
            let applied = if native {
                result.applied_damage + before - after
            } else {
                let take = match &layout.kind {
                    Id1DamageKind::Sites { take, .. } => *take,
                    Id1DamageKind::Calls { .. } => 0,
                };
                self.vm()?.global_float(take)?
            };
            if let Some(frame) = self.active.borrow_mut().last_mut() {
                frame.health_written = true;
                frame.result = SourceDamageResult {
                    applied_damage: applied,
                    reaction: DamageReaction::None,
                };
            }
        } else if store.word == self.armor_value || store.word == self.armor_type || store.word == self.items {
            let vm = self.vm()?;
            let slot = vm.entity_slot(store.reference)?;
            let current = QcWordsBuf::from_bytes(vm.entity_snapshot(slot)?)?;
            let mut previous = current.clone();
            previous.set_bytes(store.word * 4, &store.before)?;
            observer.stored(SourceStoredMutation::Armor {
                before: self.read_armor(&previous)?,
                after: self.read_armor(&current)?,
            });
        } else if store.word == self.velocity {
            let vector = |bytes: &[u8]| -> Result<Vec3, QcError> {
                let component = |offset: usize| -> Result<f32, QcError> {
                    bytes
                        .get(offset..offset + 4)
                        .and_then(|bytes| bytes.try_into().ok())
                        .map(|bytes: [u8; 4]| f32::from_le_bytes(bytes))
                        .ok_or_else(|| QcError::program("short entity store observation", self.source.program.source))
                };
                Ok(Vec3 {
                    x: component(0)?,
                    y: component(4)?,
                    z: component(8)?,
                })
            };
            observer.stored(SourceStoredMutation::SourceVelocity {
                before: vector(&store.before)?,
                after: vector(&store.after)?,
                movement_provider,
            });
        }
        Ok(())
    }
}

/// Armor stage handle (donor `protectionStage` result).
pub struct QcArmorStageHandle<'a> {
    actor: OwnedActor,
    channel: ProtectionChannel,
    owners: Rc<RefCell<HashMap<ActorId, Rc<ArmorIntercept>>>>,
    actors: &'a dyn QcActorRegistry,
}

impl SourceArmorStage for QcArmorStageHandle<'_> {
    fn bind(&self, intercept: ArmorIntercept) -> Result<Box<dyn FnOnce()>, QcError> {
        if !self.actors.is_owned(&self.actor) {
            return Err(QcError::program("actor is not owned", "progs.dat"));
        }
        // `OwnedActor` has no `Hash` impl and `ActorId` already carries
        // the session token, so the owners map is keyed by `ActorId`.
        if self.owners.borrow().contains_key(self.actor.id()) {
            let channel = match self.channel {
                ProtectionChannel::Regular => "regular",
                ProtectionChannel::Powered => "powered",
            };
            return Err(QcError::program(
                format!("QC {channel} armor stage already has an owner"),
                "progs.dat",
            ));
        }
        let intercept = Rc::new(intercept);
        self.owners
            .borrow_mut()
            .insert(self.actor.id().clone(), Rc::clone(&intercept));
        let owners = Rc::clone(&self.owners);
        let actor = self.actor.id().clone();
        Ok(Box::new(move || {
            let current = owners.borrow().get(&actor).cloned();
            if current.is_some_and(|current| Rc::ptr_eq(&current, &intercept)) {
                owners.borrow_mut().remove(&actor);
            }
        }))
    }
}

/// Whether two requests agree on everything except actor and amount
/// changes (donor `isDeepStrictEqual` over nulled provenance).
fn damage_metadata_equal(left: &DamageRequest, right: &DamageRequest) -> bool {
    same_value(left.knockback, right.knockback)
        && vec_same_value(left.direction, right.direction)
        && vec_same_value(left.point, right.point)
        && vec_same_value(left.normal, right.normal)
        && left.delivery == right.delivery
        && left.attack.sequence == right.attack.sequence
        && time_same_value(&left.attack.time, &right.attack.time)
        && left.attack.originating_projectile == right.attack.originating_projectile
        && left.attack.weapon == right.attack.weapon
        && left.attack.weapon_provider == right.attack.weapon_provider
        && left.attack.damage_powerup_owner == right.attack.damage_powerup_owner
        && left.attack.combat_provider == right.attack.combat_provider
        && left.attack.inventory_provider == right.attack.inventory_provider
        && left.attack.movement_provider == right.attack.movement_provider
        && left.attack.cause == right.attack.cause
}

/// Same-value float equality (donor `Object.is` inside
/// `isDeepStrictEqual`).
fn same_value(left: f64, right: f64) -> bool {
    left.total_cmp(&right) == std::cmp::Ordering::Equal
}

/// Same-value vector equality.
fn vec_same_value(left: Vec3, right: Vec3) -> bool {
    same_value(f64::from(left.x), f64::from(right.x))
        && same_value(f64::from(left.y), f64::from(right.y))
        && same_value(f64::from(left.z), f64::from(right.z))
}

/// Same-value source-time equality.
fn time_same_value(left: &qa_core::time::SourceTime, right: &qa_core::time::SourceTime) -> bool {
    match (left, right) {
        (qa_core::time::SourceTime::Seconds(left), qa_core::time::SourceTime::Seconds(right)) => {
            left.total_cmp(right) == std::cmp::Ordering::Equal
        }
        (qa_core::time::SourceTime::Milliseconds(left), qa_core::time::SourceTime::Milliseconds(right)) => {
            left == right
        }
        _ => false,
    }
}
