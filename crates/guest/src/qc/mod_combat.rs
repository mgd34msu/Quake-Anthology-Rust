//! QuakeC combat lowering into the original damage function.
//!
//! Ported from `src/compat/qc/mod-combat.ts`.
//!
//! Local mirrors (owned by other workers' modules, noted here):
//! `QcCombatMachine` mirrors the entity-word surface of `QcMachine` from
//! `src/compat/qc/machine.ts`; `QcCombatServices` mirrors the
//! actor/body/combat surface of `ModHostServices` from
//! `src/world/session/mods.ts`; `Id1CombatLayout` mirrors
//! `Id1ProgramBinding` from `src/content/q1/quakec/id1-program.ts`;
//! `DamageRequest`, `DamageOutcome`, and armor states mirror
//! `src/contracts/gameplay.ts`. The empty-armor grant mirrors
//! `qcEmptyArmor` from `src/content/q1/quakec/armor-points.ts`.
//!
//! Adaptation: the donor completes the damage boundary reentrantly while
//! source executes; this port completes it through
//! `QcCombatMachine::take_damage_outcome`, which the machine fills when
//! source reaches its damage authority call.

use qa_core::identity::{ActorId, OwnedActor, ProviderId};
use qa_core::math::Vec3;
use qa_core::time::SourceTime;

use super::mod_protection::DamageDelivery;
use super::mod_provider::{
    ItemId, ModCallbackInput, ModCombatDeclaration, ModQcArmorStageFlags, ModRuntimeValue, ModSourceCall, PoweredKind,
    QcModInputs, QcProgramView, QcValueType,
};
use crate::error::GuestError;

/// Validate a combat declaration against its program.
pub fn validate_qc_mod_combat(program: &dyn QcProgramView, combat: &ModCombatDeclaration) -> Result<(), GuestError> {
    if let Some(empty) = combat.empty_armor.as_ref() {
        validate_empty_armor(&empty.item, empty.absorption)?;
    }
    if let Some(stage) = combat.armor_stage.as_ref() {
        if stage.entry >= stage.exit || stage.target < 0 || stage.damage < 0 || stage.saved < 0 {
            return Err(GuestError::invalid("QC armor stage requires a valid source region"));
        }
        if let ModQcArmorStageFlags::Bits { word, .. } = stage.flags {
            if word < 0 {
                return Err(GuestError::invalid("QC armor stage requires a valid source region"));
            }
        }
    }
    if let Some(scale) = combat.damage_scale.as_ref() {
        if program.function_named(&scale.function).is_none() || scale.entry >= scale.exit {
            return Err(GuestError::invalid("QC damage scale requires a valid source region"));
        }
    }
    for name in [
        "health",
        "takedamage",
        "flags",
        "invincible_finished",
        "armorvalue",
        "armortype",
    ] {
        if program.field_type(name) != Some(QcValueType::Float) {
            return Err(GuestError::invalid(format!("QC combat requires float field {name}")));
        }
    }
    if program.function_named(&combat.damage.function).is_none() {
        return Err(GuestError::invalid("QC combat requires its declared damage function"));
    }
    Ok(())
}

/// Validate a points-only armor grant.
fn validate_empty_armor(item: &str, absorption: f64) -> Result<(), GuestError> {
    if !["q1:item_armor1", "q1:item_armor2", "q1:item_armorInv"].contains(&item)
        || !absorption.is_finite()
        || f64::from(absorption as f32) != absorption
        || absorption < 0.0
    {
        return Err(GuestError::invalid(
            "QC points-only armor requires an authored item and finite nonnegative absorption",
        ));
    }
    Ok(())
}

/// Canonical damage provenance for authored source damage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DamageProvenance {
    /// Attack sequence number.
    pub sequence: u64,
}

/// Damage request.
#[derive(Debug, Clone, PartialEq)]
pub struct DamageRequest {
    /// Target actor.
    pub target: ActorId,
    /// Damage amount.
    pub amount: f64,
    /// Knockback scalar.
    pub knockback: f64,
    /// Damage direction.
    pub direction: Vec3,
    /// Damage point.
    pub point: Vec3,
    /// Damage normal.
    pub normal: Vec3,
    /// Delivery mode.
    pub delivery: DamageDelivery,
    /// Attacker, if any.
    pub attacker: Option<ActorId>,
    /// Inflictor, if any.
    pub inflictor: Option<ActorId>,
    /// Attack time.
    pub time: SourceTime,
    /// Original death type.
    pub death_type: String,
    /// Provenance sequence.
    pub sequence: u64,
}

/// Damage reaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DamageReaction {
    /// No reaction.
    None,
    /// Pain reaction.
    Pain,
    /// Death reaction.
    Death,
}

/// Committed damage decision.
#[derive(Debug, Clone, PartialEq)]
pub struct DamageDecision {
    /// Damage applied to health.
    pub applied_damage: f64,
    /// Reaction.
    pub reaction: DamageReaction,
}

/// Damage outcome.
#[derive(Debug, Clone, PartialEq)]
pub enum DamageOutcome {
    /// Target went stale.
    StaleTarget {
        /// Original request.
        request: DamageRequest,
    },
    /// Committed decision.
    Committed {
        /// Decision.
        decision: DamageDecision,
        /// Whether the target survived.
        survived: bool,
    },
}

/// Regular armor state.
#[derive(Debug, Clone, PartialEq)]
pub enum QcCombatRegular {
    /// No armor.
    None,
    /// Quake armor.
    Q1 {
        /// Armor points.
        points: f64,
        /// Absorption fraction.
        absorption: f64,
        /// Armor item.
        item: ItemId,
    },
}

/// Armor state.
#[derive(Debug, Clone, PartialEq)]
pub struct QcCombatArmor {
    /// Regular armor.
    pub regular: QcCombatRegular,
    /// Powered protection (always none in source words).
    pub powered: PoweredKind,
}

/// Combat state projection.
#[derive(Debug, Clone, PartialEq)]
pub struct QcCombatState {
    /// Health.
    pub health: f64,
    /// Armor.
    pub armor: QcCombatArmor,
    /// Mass.
    pub mass: f64,
    /// Whether the actor can take damage.
    pub can_take_damage: bool,
    /// Whether the actor is invulnerable.
    pub invulnerable: bool,
    /// Shared team.
    pub team: Option<String>,
}

/// Pain reaction for source delivery.
#[derive(Debug, Clone, PartialEq)]
pub struct QcCombatPain {
    /// Reacting actor.
    pub target: OwnedActor,
    /// Attacker, if any.
    pub attacker: Option<ActorId>,
    /// Damage applied.
    pub damage: f64,
    /// Knockback kick.
    pub kick: f64,
}

/// Death reaction for source delivery.
#[derive(Debug, Clone, PartialEq)]
pub struct QcCombatDeath {
    /// Reacting actor.
    pub target: OwnedActor,
    /// Attacker, if any.
    pub attacker: Option<ActorId>,
    /// Inflictor, if any.
    pub inflictor: Option<ActorId>,
    /// Damage applied.
    pub damage: f64,
    /// Knockback kick.
    pub kick: f64,
    /// Death point.
    pub point: Vec3,
}

/// Effective reaction after host callbacks.
#[derive(Debug, Clone, PartialEq)]
pub struct ReactionEffect {
    /// Whether source should execute.
    pub execute: bool,
    /// Effective attacker.
    pub attacker: Option<ActorId>,
    /// Effective damage.
    pub damage: f64,
}

/// id1 combat field layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Id1CombatLayout {
    /// Armor-bit field.
    pub armor_field: String,
    /// Green, yellow, red armor masks.
    pub armor_masks: [i32; 3],
}

/// Original id1 combat layout value.
#[must_use]
pub fn id1_combat_layout() -> Id1CombatLayout {
    Id1CombatLayout {
        armor_field: "items".to_string(),
        armor_masks: [8192, 16384, 32768],
    }
}

/// Machine surface for combat words.
pub trait QcCombatMachine {
    /// Program metadata view.
    fn program(&self) -> &dyn QcProgramView;
    /// Read a float by reference and field.
    fn float_for(&self, reference: i32, field: &str) -> Result<f32, GuestError>;
    /// Write a float by reference and field.
    fn set_float_for(&mut self, reference: i32, field: &str, value: f32) -> Result<(), GuestError>;
    /// Read an integer by reference and field.
    fn int_for(&self, reference: i32, field: &str) -> Result<i32, GuestError>;
    /// Read a managed string.
    fn strings_get(&self, index: i32) -> Result<String, GuestError>;
    /// Read a global float.
    fn global_float(&self, name: &str) -> Result<f64, GuestError>;
    /// Take a pending damage outcome, if source completed the boundary.
    fn take_damage_outcome(&mut self) -> Option<DamageOutcome>;
}

/// Host services for combat.
pub trait QcCombatServices {
    /// Current source time.
    fn now(&self) -> SourceTime;
    /// Whether an actor is live.
    fn is_live(&self, actor: &ActorId) -> bool;
    /// Resolve an owned actor.
    fn resolve_owned(&self, actor: &ActorId) -> Option<OwnedActor>;
    /// Source provider and slot.
    fn source_of(&self, actor: &ActorId) -> Option<(ProviderId, u32)>;
    /// Entity reference for an actor.
    fn reference_for_actor(&self, actor: &ActorId) -> Option<i32>;
    /// Actor for an entity reference.
    fn actor_for_reference(&self, reference: i32) -> Option<ActorId>;
    /// Body origin for damage points.
    fn body_origin(&self, actor: &ActorId) -> Option<Vec3>;
    /// Canonical damage provenance.
    fn damage_context(&self) -> Option<DamageProvenance>;
    /// Shared team of a match player.
    fn player_team(&self, _actor: &ActorId) -> Option<String> {
        None
    }
}

/// Source dispatch for combat.
pub trait QcCombatDispatch {
    /// Invoke a source call.
    fn invoke(&mut self, call: &ModSourceCall, inputs: &QcModInputs) -> Result<f64, GuestError>;
    /// Deliver a pain reaction through host callbacks.
    fn pain_callback(&mut self, reaction: &QcCombatPain) -> Result<Option<(Option<ActorId>, f64)>, GuestError>;
    /// Deliver a death reaction through host callbacks.
    fn die_callback(&mut self, reaction: &QcCombatDeath) -> Result<bool, GuestError>;
}

/// Damage inputs for the declared damage call.
#[must_use]
pub fn qc_damage_inputs(request: &DamageRequest, seconds: f64) -> QcModInputs {
    let mut inputs = QcModInputs::new();
    inputs.insert(
        ModCallbackInput::Self_,
        ModRuntimeValue::Actor(Some(request.target.clone())),
    );
    inputs.insert(
        ModCallbackInput::Attacker,
        ModRuntimeValue::Actor(request.attacker.clone()),
    );
    inputs.insert(
        ModCallbackInput::Inflictor,
        ModRuntimeValue::Actor(request.inflictor.clone()),
    );
    inputs.insert(ModCallbackInput::Amount, ModRuntimeValue::Float(request.amount));
    inputs.insert(ModCallbackInput::Knockback, ModRuntimeValue::Float(request.knockback));
    inputs.insert(ModCallbackInput::Point, ModRuntimeValue::Vector(request.point));
    inputs.insert(ModCallbackInput::Direction, ModRuntimeValue::Vector(request.direction));
    inputs.insert(ModCallbackInput::Normal, ModRuntimeValue::Vector(request.normal));
    inputs.insert(ModCallbackInput::Time, ModRuntimeValue::Float(seconds));
    inputs
}

struct IncomingDamage {
    request: DamageRequest,
    entered: bool,
    outcome: Option<DamageOutcome>,
}

/// Combat lowering configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct QcCombatConfig {
    /// Combat declaration.
    pub declaration: ModCombatDeclaration,
    /// Module identifier.
    pub module: String,
    /// Owning provider.
    pub provider: ProviderId,
    /// Combat field layout.
    pub layout: Id1CombatLayout,
    /// Whether the program pins original id1 behavior.
    pub pinned_id1: bool,
}

/// Combat lowering over original bytecode fields.
pub struct QcModCombat<S, M, D> {
    declaration: ModCombatDeclaration,
    module: String,
    provider: ProviderId,
    layout: Id1CombatLayout,
    pinned_id1: bool,
    services: S,
    machine: M,
    dispatch: D,
    incoming: Vec<IncomingDamage>,
}

impl<S: QcCombatServices, M: QcCombatMachine, D: QcCombatDispatch> QcModCombat<S, M, D> {
    /// Build over a combat configuration.
    pub fn new(
        config: QcCombatConfig,
        program: &dyn QcProgramView,
        services: S,
        machine: M,
        dispatch: D,
    ) -> Result<Self, GuestError> {
        validate_qc_mod_combat(program, &config.declaration)?;
        Ok(Self {
            declaration: config.declaration,
            module: config.module,
            provider: config.provider,
            layout: config.layout,
            pinned_id1: config.pinned_id1,
            services,
            machine,
            dispatch,
            incoming: Vec::new(),
        })
    }

    /// Module identifier.
    #[must_use]
    pub fn module(&self) -> &str {
        &self.module
    }

    /// Current time in seconds.
    fn seconds(&self) -> f64 {
        self.services.now().as_seconds_f64()
    }

    /// Entity reference for an actor.
    fn reference(&self, actor: Option<&ActorId>) -> Result<i32, GuestError> {
        match actor {
            None => Ok(0),
            Some(actor) => self
                .services
                .reference_for_actor(actor)
                .ok_or_else(|| GuestError::invalid("QC combat actor has no source reference")),
        }
    }

    /// Admit an owned source actor.
    pub fn admit(&mut self, actor: &OwnedActor) -> Result<(), GuestError> {
        let source = self.services.source_of(actor.id());
        if source.as_ref().is_none_or(|(provider, _)| provider != &self.provider) {
            return Err(GuestError::invalid("QC combat admission requires its own source actor"));
        }
        // Touch every combat word so missing fields fail at admission.
        let reference = self.reference(Some(actor.id()))?;
        for name in ["health", "takedamage", "invincible_finished", "armorvalue", "armortype"] {
            self.machine.float_for(reference, name)?;
        }
        let field = self.layout.armor_field.clone();
        self.machine.float_for(reference, &field)?;
        Ok(())
    }

    /// Read combat state.
    pub fn read_state(&self, actor: &ActorId) -> Result<QcCombatState, GuestError> {
        let reference = self.reference(Some(actor))?;
        Ok(QcCombatState {
            health: f64::from(self.machine.float_for(reference, "health")?),
            armor: self.read_armor(reference)?,
            mass: 200.0,
            can_take_damage: self.machine.float_for(reference, "takedamage")? != 0.0,
            invulnerable: f64::from(self.machine.float_for(reference, "invincible_finished")?) > self.seconds(),
            team: self.services.player_team(actor),
        })
    }

    /// Read armor from source words.
    fn read_armor(&self, reference: i32) -> Result<QcCombatArmor, GuestError> {
        let field = self.layout.armor_field.clone();
        let items = self.machine.float_for(reference, &field)? as i32;
        let [green, yellow, red] = self.layout.armor_masks;
        let item = if items & red != 0 {
            Some("q1:item_armorInv")
        } else if items & yellow != 0 {
            Some("q1:item_armor2")
        } else if items & green != 0 {
            Some("q1:item_armor1")
        } else {
            None
        };
        Ok(QcCombatArmor {
            regular: match item {
                None => QcCombatRegular::None,
                Some(item) => QcCombatRegular::Q1 {
                    points: f64::from(self.machine.float_for(reference, "armorvalue")?),
                    absorption: f64::from(self.machine.float_for(reference, "armortype")?),
                    item: item.to_string(),
                },
            },
            powered: PoweredKind::None,
        })
    }

    /// Write health.
    pub fn write_health(&mut self, actor: &ActorId, health: f64) -> Result<(), GuestError> {
        let reference = self.reference(Some(actor))?;
        self.machine.set_float_for(reference, "health", health as f32)
    }

    /// Validate armor for source storage.
    fn validate_armor(&self, armor: &QcCombatArmor) -> Result<(), GuestError> {
        if armor.powered != PoweredKind::None {
            return Err(GuestError::invalid("Cannot store foreign armor in source QC fields"));
        }
        Ok(())
    }

    /// Write armor.
    pub fn write_armor(&mut self, actor: &ActorId, armor: &QcCombatArmor) -> Result<(), GuestError> {
        self.validate_armor(armor)?;
        let reference = self.reference(Some(actor))?;
        let [green, yellow, red] = self.layout.armor_masks;
        let mask = green | yellow | red;
        let (points, absorption, bit) = match &armor.regular {
            QcCombatRegular::None => (0.0, 0.0, 0),
            QcCombatRegular::Q1 {
                points,
                absorption,
                item,
            } => {
                let bit = if item == "q1:item_armorInv" {
                    red
                } else if item == "q1:item_armor2" {
                    yellow
                } else if item == "q1:item_armor1" {
                    green
                } else {
                    return Err(GuestError::invalid("Cannot store foreign armor in source QC fields"));
                };
                (*points, *absorption, bit)
            }
        };
        self.machine.set_float_for(reference, "armorvalue", points as f32)?;
        self.machine.set_float_for(reference, "armortype", absorption as f32)?;
        let field = self.layout.armor_field.clone();
        let current = self.machine.float_for(reference, &field)? as i32;
        self.machine
            .set_float_for(reference, &field, ((current & !mask) | bit) as f32)?;
        Ok(())
    }

    /// Points-only armor grant.
    #[must_use]
    pub fn empty_armor_grant(&self, points: f64) -> Option<QcCombatRegular> {
        let source = self
            .declaration
            .empty_armor
            .as_ref()
            .map(|empty| (empty.item.clone(), empty.absorption))
            .or_else(|| self.pinned_id1.then(|| ("q1:item_armorInv".to_string(), 0.8)))?;
        Some(QcCombatRegular::Q1 {
            points,
            absorption: f64::from(source.1 as f32),
            item: source.0,
        })
    }

    /// Apply canonical damage through the declared damage call.
    pub fn apply(&mut self, request: &DamageRequest) -> Result<DamageOutcome, GuestError> {
        self.incoming.push(IncomingDamage {
            request: request.clone(),
            entered: false,
            outcome: None,
        });
        let inputs = qc_damage_inputs(request, self.seconds());
        let call = self.declaration.damage.clone();
        let result = self.dispatch.invoke(&call, &inputs);
        let outcome = self.machine.take_damage_outcome();
        let mut entry = self
            .incoming
            .pop()
            .ok_or_else(|| GuestError::invalid("QC damage boundary is unbalanced"))?;
        entry.outcome = outcome;
        result?;
        entry
            .outcome
            .ok_or_else(|| GuestError::invalid("QC damage did not complete its shared authority boundary"))
    }

    /// Build a request for authored source damage.
    pub fn authored_request(
        &mut self,
        target: &ActorId,
        attacker: Option<&ActorId>,
        inflictor: Option<&ActorId>,
        amount: f64,
    ) -> Result<DamageRequest, GuestError> {
        if let Some(entry) = self.incoming.last_mut() {
            if !entry.entered {
                entry.entered = true;
                return Ok(entry.request.clone());
            }
        }
        let context = self
            .services
            .damage_context()
            .ok_or_else(|| GuestError::invalid("Authored QC damage requires canonical attack provenance"))?;
        let reference = self.reference(Some(target))?;
        let death_type = if self.machine.program().field_type("deathtype").is_some() {
            let index = self.machine.int_for(reference, "deathtype")?;
            self.machine.strings_get(index)?
        } else {
            String::new()
        };
        Ok(DamageRequest {
            target: target.clone(),
            amount,
            knockback: 0.0,
            direction: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            normal: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            point: self
                .services
                .body_origin(target)
                .unwrap_or(Vec3 { x: 0.0, y: 0.0, z: 0.0 }),
            delivery: DamageDelivery::Direct,
            attacker: attacker.cloned(),
            inflictor: inflictor.cloned(),
            time: SourceTime::Seconds(self.machine.global_float("time")? as f32),
            death_type,
            sequence: context.sequence,
        })
    }

    /// Route a source reaction through host callbacks.
    pub fn react(
        &mut self,
        request: &DamageRequest,
        applied: f64,
        reaction: DamageReaction,
        point: Vec3,
    ) -> Result<ReactionEffect, GuestError> {
        let owner = self.services.resolve_owned(&request.target);
        let Some(owner) = owner else {
            return Ok(ReactionEffect {
                execute: false,
                attacker: None,
                damage: applied,
            });
        };
        match reaction {
            DamageReaction::Pain => {
                let pain = QcCombatPain {
                    target: owner,
                    attacker: request.attacker.clone(),
                    damage: applied,
                    kick: request.knockback,
                };
                match self.dispatch.pain_callback(&pain)? {
                    Some((attacker, damage)) => Ok(ReactionEffect {
                        execute: true,
                        attacker,
                        damage,
                    }),
                    None => Ok(ReactionEffect {
                        execute: false,
                        attacker: None,
                        damage: applied,
                    }),
                }
            }
            DamageReaction::Death => {
                let death = QcCombatDeath {
                    target: owner,
                    attacker: request.attacker.clone(),
                    inflictor: request.inflictor.clone(),
                    damage: applied,
                    kick: request.knockback,
                    point,
                };
                Ok(ReactionEffect {
                    execute: self.dispatch.die_callback(&death)?,
                    attacker: request.attacker.clone(),
                    damage: applied,
                })
            }
            DamageReaction::None => Err(GuestError::invalid("QC reaction boundary has no source reaction")),
        }
    }

    /// Deliver a pain reaction to source.
    pub fn pain(&mut self, reaction: &QcCombatPain) -> Result<(), GuestError> {
        self.actor_reaction(
            reaction.target.id(),
            &reaction.attacker,
            reaction.damage,
            "th_pain",
            &[ModCallbackInput::Attacker, ModCallbackInput::Amount],
        )
    }

    /// Deliver a death reaction to source.
    pub fn die(&mut self, reaction: &QcCombatDeath) -> Result<(), GuestError> {
        self.actor_reaction(reaction.target.id(), &None, reaction.damage, "th_die", &[])
    }

    /// Invoke a reaction field.
    fn actor_reaction(
        &mut self,
        actor: &ActorId,
        attacker: &Option<ActorId>,
        damage: f64,
        field: &str,
        args: &[ModCallbackInput],
    ) -> Result<(), GuestError> {
        let reference = self.reference(Some(actor))?;
        let index = self.machine.int_for(reference, field)?;
        if index == 0 {
            return Ok(());
        }
        let name = self
            .machine
            .program()
            .function_at(index)
            .map(|function| function.name)
            .ok_or_else(|| GuestError::invalid("QC reaction names an unknown source function"))?;
        let mut inputs = QcModInputs::new();
        inputs.insert(ModCallbackInput::Self_, ModRuntimeValue::Actor(Some(actor.clone())));
        inputs.insert(ModCallbackInput::Attacker, ModRuntimeValue::Actor(attacker.clone()));
        inputs.insert(ModCallbackInput::Amount, ModRuntimeValue::Float(damage));
        inputs.insert(ModCallbackInput::Time, ModRuntimeValue::Float(self.seconds()));
        let call = ModSourceCall {
            function: name,
            arguments: args
                .iter()
                .map(|name| super::mod_provider::ModCallbackValue::Input(*name))
                .collect(),
            globals: [(ModCallbackInput::Self_, "self"), (ModCallbackInput::Time, "time")]
                .into_iter()
                .map(|(input, name)| super::mod_provider::ModSourceGlobal {
                    name: name.to_string(),
                    value: super::mod_provider::ModCallbackValue::Input(input),
                })
                .collect(),
        };
        self.dispatch.invoke(&call, &inputs).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use qa_core::identity::IdentityOwner;
    use qa_core::math::vec3;

    use super::super::mod_provider::{ModQcEmptyArmor, QcApiKind, QcFunctionView};

    struct FakeProgram {
        fields: HashMap<String, QcValueType>,
        functions: HashMap<String, QcFunctionView>,
        by_index: HashMap<i32, QcFunctionView>,
    }

    impl QcProgramView for FakeProgram {
        fn digest(&self) -> &str {
            "abc"
        }

        fn api_kind(&self) -> QcApiKind {
            QcApiKind::Q1Netquake
        }

        fn field_type(&self, name: &str) -> Option<QcValueType> {
            self.fields.get(name).copied()
        }

        fn global_type(&self, _name: &str) -> Option<QcValueType> {
            None
        }

        fn function_named(&self, name: &str) -> Option<QcFunctionView> {
            self.functions.get(name).cloned()
        }

        fn function_at(&self, index: i32) -> Option<QcFunctionView> {
            self.by_index.get(&index).cloned()
        }

        fn functions(&self) -> Vec<QcFunctionView> {
            self.functions.values().cloned().collect()
        }
    }

    struct FakeServices {
        owner: IdentityOwner,
        live: Vec<ActorId>,
        sources: HashMap<ActorId, (ProviderId, u32)>,
        context: Option<DamageProvenance>,
    }

    impl QcCombatServices for FakeServices {
        fn now(&self) -> SourceTime {
            SourceTime::Seconds(7.0)
        }

        fn is_live(&self, actor: &ActorId) -> bool {
            self.live.contains(actor)
        }

        fn resolve_owned(&self, actor: &ActorId) -> Option<OwnedActor> {
            self.live
                .iter()
                .find(|live| *live == actor)
                .and_then(|actor| self.owner.owned_actor(actor, ProviderId::new("mod", "test")).ok())
        }

        fn source_of(&self, actor: &ActorId) -> Option<(ProviderId, u32)> {
            self.sources.get(actor).cloned()
        }

        fn reference_for_actor(&self, actor: &ActorId) -> Option<i32> {
            Some(actor.slot() as i32)
        }

        fn actor_for_reference(&self, reference: i32) -> Option<ActorId> {
            self.live.iter().find(|actor| actor.slot() as i32 == reference).cloned()
        }

        fn body_origin(&self, _actor: &ActorId) -> Option<Vec3> {
            Some(vec3(1.0, 2.0, 3.0))
        }

        fn damage_context(&self) -> Option<DamageProvenance> {
            self.context.clone()
        }
    }

    struct FakeMachine {
        program: FakeProgram,
        floats: HashMap<(i32, String), f32>,
        ints: HashMap<(i32, String), i32>,
        strings: Vec<String>,
        time: f64,
        outcome: Option<DamageOutcome>,
    }

    impl QcCombatMachine for FakeMachine {
        fn program(&self) -> &dyn QcProgramView {
            &self.program
        }

        fn float_for(&self, reference: i32, field: &str) -> Result<f32, GuestError> {
            Ok(self.floats.get(&(reference, field.to_string())).copied().unwrap_or(0.0))
        }

        fn set_float_for(&mut self, reference: i32, field: &str, value: f32) -> Result<(), GuestError> {
            self.floats.insert((reference, field.to_string()), value);
            Ok(())
        }

        fn int_for(&self, reference: i32, field: &str) -> Result<i32, GuestError> {
            Ok(self.ints.get(&(reference, field.to_string())).copied().unwrap_or(0))
        }

        fn strings_get(&self, index: i32) -> Result<String, GuestError> {
            self.strings
                .get(index as usize)
                .cloned()
                .ok_or_else(|| GuestError::invalid("String index is out of range"))
        }

        fn global_float(&self, name: &str) -> Result<f64, GuestError> {
            if name == "time" {
                Ok(self.time)
            } else {
                Err(GuestError::invalid(format!("Unknown global {name}")))
            }
        }

        fn take_damage_outcome(&mut self) -> Option<DamageOutcome> {
            self.outcome.take()
        }
    }

    struct FakeDispatch {
        calls: Vec<(String, QcModInputs)>,
        pain: Option<(Option<ActorId>, f64)>,
        die: bool,
    }

    impl QcCombatDispatch for FakeDispatch {
        fn invoke(&mut self, call: &ModSourceCall, inputs: &QcModInputs) -> Result<f64, GuestError> {
            self.calls.push((call.function.clone(), inputs.clone()));
            Ok(0.0)
        }

        fn pain_callback(&mut self, _reaction: &QcCombatPain) -> Result<Option<(Option<ActorId>, f64)>, GuestError> {
            Ok(self.pain.clone())
        }

        fn die_callback(&mut self, _reaction: &QcCombatDeath) -> Result<bool, GuestError> {
            Ok(self.die)
        }
    }

    fn program() -> FakeProgram {
        let mut fields = HashMap::new();
        for name in [
            "health",
            "takedamage",
            "flags",
            "invincible_finished",
            "armorvalue",
            "armortype",
            "items",
            "th_pain",
            "th_die",
        ] {
            fields.insert(name.to_string(), QcValueType::Float);
        }
        let damage = QcFunctionView {
            index: 3,
            name: "T_Damage".to_string(),
            first_statement: 1,
            parameter_start: 0,
            parameter_sizes: Vec::new(),
            named_builtin: false,
        };
        FakeProgram {
            fields,
            functions: [("T_Damage".to_string(), damage.clone())].into_iter().collect(),
            by_index: [(3, damage)].into_iter().collect(),
        }
    }

    fn declaration() -> ModCombatDeclaration {
        ModCombatDeclaration {
            damage: ModSourceCall {
                function: "T_Damage".to_string(),
                arguments: Vec::new(),
                globals: Vec::new(),
            },
            damage_scale: None,
            armor_stage: None,
            empty_armor: None,
        }
    }

    fn fixture() -> QcModCombat<FakeServices, FakeMachine, FakeDispatch> {
        QcModCombat::new(
            QcCombatConfig {
                declaration: declaration(),
                module: "test:mod".to_string(),
                provider: ProviderId::new("mod", "test"),
                layout: id1_combat_layout(),
                pinned_id1: true,
            },
            &program(),
            FakeServices {
                owner: IdentityOwner::create("combat").unwrap(),
                live: Vec::new(),
                sources: HashMap::new(),
                context: Some(DamageProvenance { sequence: 9 }),
            },
            FakeMachine {
                program: program(),
                floats: HashMap::new(),
                ints: HashMap::new(),
                strings: vec![String::new()],
                time: 7.0,
                outcome: None,
            },
            FakeDispatch {
                calls: Vec::new(),
                pain: None,
                die: false,
            },
        )
        .unwrap()
    }

    fn join(combat: &mut QcModCombat<FakeServices, FakeMachine, FakeDispatch>) -> OwnedActor {
        let actor = combat.services.owner.actor(2, 1);
        combat.services.live.push(actor.clone());
        combat
            .services
            .sources
            .insert(actor.clone(), (ProviderId::new("mod", "test"), 2));
        combat.services.resolve_owned(&actor).unwrap()
    }

    fn request(target: &ActorId) -> DamageRequest {
        DamageRequest {
            target: target.clone(),
            amount: 25.0,
            knockback: 4.0,
            direction: vec3(0.0, 0.0, 1.0),
            point: vec3(1.0, 2.0, 3.0),
            normal: vec3(0.0, 0.0, 1.0),
            delivery: DamageDelivery::Direct,
            attacker: None,
            inflictor: None,
            time: SourceTime::Seconds(7.0),
            death_type: String::new(),
            sequence: 9,
        }
    }

    #[test]
    fn validation_accepts_and_rejects() {
        assert!(validate_qc_mod_combat(&program(), &declaration()).is_ok());
        let mut sparse = program();
        sparse.fields.remove("health");
        assert!(validate_qc_mod_combat(&sparse, &declaration()).is_err());
        let mut bad = declaration();
        bad.empty_armor = Some(ModQcEmptyArmor {
            item: "q1:item_shells".to_string(),
            absorption: 0.5,
        });
        assert!(validate_qc_mod_combat(&program(), &bad).is_err());
    }

    #[test]
    fn admit_read_write_round_trip() {
        let mut combat = fixture();
        let owned = join(&mut combat);
        combat.admit(&owned).unwrap();
        assert_eq!(combat.module(), "test:mod");
        combat.write_health(owned.id(), 80.0).unwrap();
        combat
            .write_armor(
                owned.id(),
                &QcCombatArmor {
                    regular: QcCombatRegular::Q1 {
                        points: 60.0,
                        absorption: 0.6,
                        item: "q1:item_armor2".to_string(),
                    },
                    powered: PoweredKind::None,
                },
            )
            .unwrap();
        let state = combat.read_state(owned.id()).unwrap();
        assert_eq!(state.health, 80.0);
        assert_eq!(state.mass, 200.0);
        assert!(matches!(state.armor.regular, QcCombatRegular::Q1 { .. }));
        assert_eq!(combat.machine.floats.get(&(2, "items".to_string())), Some(&16384.0));
        combat
            .write_armor(
                owned.id(),
                &QcCombatArmor {
                    regular: QcCombatRegular::None,
                    powered: PoweredKind::None,
                },
            )
            .unwrap();
        assert!(matches!(
            combat.read_state(owned.id()).unwrap().armor.regular,
            QcCombatRegular::None
        ));
    }

    #[test]
    fn write_armor_rejects_foreign() {
        let mut combat = fixture();
        let owned = join(&mut combat);
        combat.admit(&owned).unwrap();
        assert!(combat
            .write_armor(
                owned.id(),
                &QcCombatArmor {
                    regular: QcCombatRegular::None,
                    powered: PoweredKind::Screen
                }
            )
            .is_err());
        assert!(combat
            .write_armor(
                owned.id(),
                &QcCombatArmor {
                    regular: QcCombatRegular::Q1 {
                        points: 10.0,
                        absorption: 0.3,
                        item: "q1:item_shells".to_string()
                    },
                    powered: PoweredKind::None,
                }
            )
            .is_err());
    }

    #[test]
    fn empty_armor_grant_uses_declared_then_pinned() {
        let combat = fixture();
        let grant = combat.empty_armor_grant(100.0).unwrap();
        assert!(matches!(grant, QcCombatRegular::Q1 { ref item, .. } if item == "q1:item_armorInv"));
        let mut declared = declaration();
        declared.empty_armor = Some(ModQcEmptyArmor {
            item: "q1:item_armor1".to_string(),
            absorption: 0.3,
        });
        let pinned = QcModCombat::new(
            QcCombatConfig {
                declaration: declared,
                module: "test:mod".to_string(),
                provider: ProviderId::new("mod", "test"),
                layout: id1_combat_layout(),
                pinned_id1: false,
            },
            &program(),
            FakeServices {
                owner: IdentityOwner::create("c2").unwrap(),
                live: Vec::new(),
                sources: HashMap::new(),
                context: None,
            },
            FakeMachine {
                program: program(),
                floats: HashMap::new(),
                ints: HashMap::new(),
                strings: vec![],
                time: 0.0,
                outcome: None,
            },
            FakeDispatch {
                calls: Vec::new(),
                pain: None,
                die: false,
            },
        )
        .unwrap();
        assert!(
            matches!(pinned.empty_armor_grant(50.0).unwrap(), QcCombatRegular::Q1 { ref item, .. } if item == "q1:item_armor1")
        );
    }

    #[test]
    fn apply_requires_completed_boundary() {
        let mut combat = fixture();
        let owned = join(&mut combat);
        let call_request = request(owned.id());
        assert!(combat.apply(&call_request).is_err());
        combat.machine.outcome = Some(DamageOutcome::Committed {
            decision: DamageDecision {
                applied_damage: 20.0,
                reaction: DamageReaction::Pain,
            },
            survived: true,
        });
        let outcome = combat.apply(&call_request).unwrap();
        assert!(matches!(outcome, DamageOutcome::Committed { .. }));
        assert_eq!(combat.dispatch.calls.last().unwrap().0, "T_Damage");
        let inputs = &combat.dispatch.calls.last().unwrap().1;
        assert_eq!(
            inputs.get(&ModCallbackInput::Amount),
            Some(&ModRuntimeValue::Float(25.0))
        );
    }

    #[test]
    fn authored_request_builds_from_context() {
        let mut combat = fixture();
        let owned = join(&mut combat);
        let built = combat.authored_request(owned.id(), None, None, 12.0).unwrap();
        assert_eq!(built.amount, 12.0);
        assert_eq!(built.sequence, 9);
        assert_eq!(built.point, vec3(1.0, 2.0, 3.0));
        combat.services.context = None;
        assert!(combat.authored_request(owned.id(), None, None, 1.0).is_err());
    }

    #[test]
    fn react_routes_through_callbacks() {
        let mut combat = fixture();
        let owned = join(&mut combat);
        combat.dispatch.pain = Some((None, 18.0));
        combat.dispatch.die = true;
        let call_request = request(owned.id());
        let pain = combat
            .react(&call_request, 18.0, DamageReaction::Pain, vec3(0.0, 0.0, 0.0))
            .unwrap();
        assert!(pain.execute);
        let death = combat
            .react(&call_request, 100.0, DamageReaction::Death, vec3(0.0, 0.0, 0.0))
            .unwrap();
        assert!(death.execute);
        assert!(combat
            .react(&call_request, 0.0, DamageReaction::None, vec3(0.0, 0.0, 0.0))
            .is_err());
        combat.dispatch.pain = None;
        assert!(
            !combat
                .react(&call_request, 5.0, DamageReaction::Pain, vec3(0.0, 0.0, 0.0))
                .unwrap()
                .execute
        );
    }

    #[test]
    fn pain_and_die_invoke_reaction_fields() {
        let mut combat = fixture();
        let owned = join(&mut combat);
        combat.machine.ints.insert((2, "th_pain".to_string()), 3);
        combat
            .pain(&QcCombatPain {
                target: owned.clone(),
                attacker: None,
                damage: 9.0,
                kick: 1.0,
            })
            .unwrap();
        assert_eq!(combat.dispatch.calls.last().unwrap().0, "T_Damage");
        combat
            .die(&QcCombatDeath {
                target: owned,
                attacker: None,
                inflictor: None,
                damage: 99.0,
                kick: 0.0,
                point: vec3(0.0, 0.0, 0.0),
            })
            .unwrap();
        // th_die is zero, so no second call.
        assert_eq!(combat.dispatch.calls.len(), 1);
    }
}
