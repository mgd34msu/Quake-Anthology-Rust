//! Quake III foundation: character.
//!
//! Donor provenance: `src/content/q3/foundation/character.ts`.

use crate::contract::{ArmorState, InventoryEntry, PoweredProtectionState, RegularArmorState};
use qa_core::identity::{ActorId, OwnedActor, ProviderId};
use qa_core::math::{vec3, Bounds, Vec3};
use qa_core::time::FrameContext;
use qa_world::movement::q3::animation::{run_q3_animation_operation, Q3AnimationContext};
use qa_world::movement::q3::constants::{entity_event, player_animation};
use qa_world::movement::q3::types::{Q3AnimationRequest, Q3AnimationStepResult, Q3Product};
use qa_world::movement::types::{ActorAnimationState, AnimationState, LocomotionAnimation};
use std::cell::RefCell;
use std::rc::Rc;
use thiserror::Error;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::foundation::animation::*;
use crate::q3::foundation::animation_config::{game_atoi, AnimationConfigError};
use crate::q3::foundation::arsenal::*;

// ---------------------------------------------------------------------------
// character.ts: ClientSpawn, player_die, character animation admission.
// ---------------------------------------------------------------------------

/// Character failure (donor `TypeError` and `Error` throws plus wrapped
/// animation-config failures).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CharacterError {
    /// Wrong provider or state kind (donor `TypeError`).
    #[error("{0}")]
    Type(String),
    /// Operation failure (donor `Error`).
    #[error("{0}")]
    Failed(String),
    /// Wrapped animation config failure.
    #[error(transparent)]
    AnimationConfig(#[from] AnimationConfigError),
}

fn failed(message: impl Into<String>) -> CharacterError {
    CharacterError::Failed(message.into())
}

fn type_error(message: impl Into<String>) -> CharacterError {
    CharacterError::Type(message.into())
}

/// Animation step input (`AnimationStepInput`).
#[derive(Debug, Clone, PartialEq)]
pub struct AnimationStepInput {
    /// Frame clock.
    pub frame: FrameContext,
    /// Animation.
    pub animation: ActorAnimationState,
    /// Locomotion.
    pub locomotion: LocomotionAnimation,
    /// Moving backwards.
    pub backwards: bool,
    /// Force selection.
    pub force: bool,
}

/// Character collision bounds (`Q3_CHARACTER_BOUNDS`).
pub const Q3_CHARACTER_BOUNDS: Bounds = Bounds {
    min: Vec3 {
        x: -15.0,
        y: -15.0,
        z: -24.0,
    },
    max: Vec3 {
        x: 15.0,
        y: 15.0,
        z: 32.0,
    },
};

/// Character view height (`Q3_CHARACTER_VIEW_HEIGHT`).
pub const Q3_CHARACTER_VIEW_HEIGHT: i32 = 26;

/// Combat state (`CombatState`, foundation-read fields).
#[derive(Debug, Clone, PartialEq)]
pub struct CombatState {
    /// Health.
    pub health: i32,
    /// Armor.
    pub armor: ArmorState,
    /// Mass.
    pub mass: f64,
    /// Can take damage.
    pub can_take_damage: bool,
    /// Invulnerable.
    pub invulnerable: bool,
    /// No knockback.
    pub no_knockback: bool,
    /// Team.
    pub team: Option<String>,
}

/// Combat trait changes (`Partial<CombatTraits>`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CombatTraitChanges {
    /// Can take damage.
    pub can_take_damage: Option<bool>,
    /// Mass.
    pub mass: Option<f64>,
    /// Invulnerable.
    pub invulnerable: Option<bool>,
    /// Team.
    pub team: Option<Option<String>>,
    /// No knockback.
    pub no_knockback: Option<bool>,
}

impl CombatTraitChanges {
    /// Copy every trait from a combat state.
    #[must_use]
    pub fn from_combat(combat: &CombatState) -> Self {
        Self {
            can_take_damage: Some(combat.can_take_damage),
            mass: Some(combat.mass),
            invulnerable: Some(combat.invulnerable),
            team: Some(combat.team.clone()),
            no_knockback: Some(combat.no_knockback),
        }
    }
}

/// Body state (`BodyState`).
#[derive(Debug, Clone, PartialEq)]
pub struct BodyState {
    /// Origin.
    pub origin: Vec3,
    /// Angles.
    pub angles: Vec3,
    /// Velocity.
    pub velocity: Vec3,
    /// Bounds.
    pub bounds: Bounds,
    /// Ground actor.
    pub ground: Option<ActorId>,
}

/// Character event (`Q3CharacterEvent`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3CharacterEvent {
    /// Actor.
    pub actor: OwnedActor,
    /// Sequence.
    pub sequence: i32,
    /// Time in milliseconds.
    pub time_ms: i32,
    /// Event.
    pub event: i32,
    /// Parameter.
    pub parameter: i32,
}

/// Death context (`Q3CharacterDeathContext`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q3CharacterDeathContext {
    /// Blood.
    pub blood: bool,
    /// No item drop.
    pub no_drop: bool,
    /// Suicide.
    pub suicide: bool,
    /// Killer source slot.
    pub killer_source_slot: i32,
}

/// Spawn input (`Q3CharacterSpawn`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3CharacterSpawn {
    /// Body.
    pub body: BodyState,
    /// Combat.
    pub combat: CombatState,
    /// Inventory.
    pub inventory: Vec<InventoryEntry>,
}

/// Character checkpoint (`Q3CharacterCheckpoint`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3CharacterCheckpoint {
    /// Version (always 1).
    pub version: u32,
    /// Product.
    pub product: Q3Product,
    /// Animation.
    pub animation: AnimationState,
    /// Source flags.
    pub flags: i32,
    /// Event sequence.
    pub event_sequence: i32,
    /// Respawn time.
    pub respawn_time: i32,
    /// Spawn count.
    pub spawn_count: i32,
    /// Dead.
    pub dead: bool,
    /// Gibbed.
    pub gibbed: bool,
    /// Initialized.
    pub initialized: bool,
}

/// Death animation checkpoint (`Q3DeathAnimationCheckpoint`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q3DeathAnimationCheckpoint {
    /// Version (always 1).
    pub version: u32,
    /// Index.
    pub index: u32,
}

/// Placement mode (`Q3CharacterServices` placement arm).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3Placement {
    /// Source game placement.
    SourceGame,
    /// Character placement.
    Character,
}

/// Bound character callbacks (pain reads current health itself, as the donor
/// closure does; die runs the shared die path).
pub struct Q3CharacterCallbackSet {
    /// Pain handler.
    pub pain: Box<dyn Fn()>,
    /// Die handler.
    pub die: Box<dyn Fn()>,
}

/// Character services (`Q3CharacterServices`).
pub trait Q3CharacterServices {
    /// Read a body.
    fn read_body(&self, actor: &ActorId) -> Option<BodyState>;
    /// Write a body.
    fn write_body(&self, actor: &OwnedActor, body: BodyState);
    /// Link a body.
    fn link_body(&self, actor: &OwnedActor);
    /// Unlink a body.
    fn unlink_body(&self, actor: &OwnedActor);
    /// Bind pain/die callbacks.
    fn bind_callbacks(&self, actor: &OwnedActor, callbacks: Q3CharacterCallbackSet);
    /// Read combat.
    fn read_combat(&self, actor: &ActorId) -> Option<CombatState>;
    /// Create combat.
    fn create_combat(&self, actor: &OwnedActor, combat: CombatState);
    /// Set health.
    fn set_health(&self, actor: &OwnedActor, health: i32);
    /// Set armor.
    fn set_armor(&self, actor: &OwnedActor, armor: ArmorState);
    /// Set traits.
    fn set_traits(&self, actor: &OwnedActor, traits: CombatTraitChanges);
    /// Inventory presence.
    fn has_inventory(&self, actor: &ActorId) -> bool;
    /// Create inventory.
    fn create_inventory(&self, actor: &OwnedActor, entries: Vec<InventoryEntry>);
    /// Inventory entries.
    fn inventory_entries(&self, actor: &ActorId) -> Vec<InventoryEntry>;
    /// Configure one entry.
    fn configure_inventory(&self, actor: &OwnedActor, entry: InventoryEntry);
    /// Current time in milliseconds.
    fn time_ms(&self) -> i32;
    /// Emit an event.
    fn emit(&self, event: Q3CharacterEvent);
    /// Death context.
    fn death_context(&self, actor: &OwnedActor) -> Q3CharacterDeathContext;
    /// Placement mode.
    fn placement(&self) -> Q3Placement;
    /// Run spawn targets.
    fn spawn_targets(&self, actor: &OwnedActor);
    /// Run kill box.
    fn kill_box(&self, actor: &OwnedActor);
}

/// Global death animation cycling (`Q3DeathAnimationSequence`).
#[derive(Debug, Clone, Default)]
pub struct Q3DeathAnimationSequence {
    index: u32,
}

impl Q3DeathAnimationSequence {
    /// Fresh sequence.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Capture the index.
    #[must_use]
    pub fn capture(&self) -> Q3DeathAnimationCheckpoint {
        Q3DeathAnimationCheckpoint {
            version: 1,
            index: self.index,
        }
    }

    /// Restore the index.
    pub fn restore(&mut self, checkpoint: &Q3DeathAnimationCheckpoint) -> Result<(), CharacterError> {
        if checkpoint.version != 1 || checkpoint.index > 2 {
            return Err(type_error("Invalid Q3 death animation checkpoint"));
        }
        self.index = checkpoint.index;
        Ok(())
    }

    /// Next death animation and event.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> (i32, i32) {
        let result = if self.index == 0 {
            (player_animation::BOTH_DEATH1, entity_event::DEATH1)
        } else if self.index == 1 {
            (player_animation::BOTH_DEATH2, entity_event::DEATH2)
        } else {
            (player_animation::BOTH_DEATH3, entity_event::DEATH3)
        };
        self.index = (self.index + 1) % 3;
        result
    }
}

/// Maximum health from a handicap (`q3MaximumHealth`).
pub fn q3_maximum_health(handicap: &str) -> Result<i32, CharacterError> {
    let maximum = game_atoi(handicap)?;
    Ok(if !(1..=100).contains(&maximum) { 100 } else { maximum })
}

/// Initial combat state (`q3InitialCombat`).
pub fn q3_initial_combat(handicap: &str, team: Option<String>) -> Result<CombatState, CharacterError> {
    Ok(CombatState {
        health: q3_maximum_health(handicap)?.wrapping_add(25),
        armor: ArmorState {
            regular: RegularArmorState::Q3 {
                points: 0.0,
                protection: f64::from(0.66f32),
            },
            powered: PoweredProtectionState::None,
        },
        mass: 200.0,
        can_take_damage: true,
        invulnerable: false,
        no_knockback: false,
        team,
    })
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Q3CharacterInner {
    animation: AnimationState,
    flags: i32,
    sequence: i32,
    respawn_time: i32,
    spawn_count: i32,
    dead: bool,
    gibbed: bool,
    initialized: bool,
}

/// Q3 character actor (`Q3CharacterActor`). Clones share identity, like the
/// donor object; bound callbacks reenter through the shared state.
#[derive(Debug, Clone)]
pub struct Q3CharacterActor<S> {
    actor: OwnedActor,
    provider: ProviderId,
    product: Q3Product,
    services: Rc<S>,
    death_animations: Rc<RefCell<Q3DeathAnimationSequence>>,
    inner: Rc<RefCell<Q3CharacterInner>>,
}

impl<S: Q3CharacterServices + 'static> Q3CharacterActor<S> {
    /// Bind a character to its services.
    pub fn new(
        actor: OwnedActor,
        provider: ProviderId,
        product: Q3Product,
        services: Rc<S>,
        death_animations: Rc<RefCell<Q3DeathAnimationSequence>>,
    ) -> Self {
        Self {
            actor,
            provider,
            product,
            services,
            death_animations,
            inner: Rc::new(RefCell::new(Q3CharacterInner {
                animation: q3_spawn_animation(),
                flags: 0,
                sequence: 0,
                respawn_time: 0,
                spawn_count: 0,
                dead: false,
                gibbed: false,
                initialized: false,
            })),
        }
    }

    /// Current animation.
    #[must_use]
    pub fn animation(&self) -> ActorAnimationState {
        ActorAnimationState {
            provider: self.provider.clone(),
            state: self.inner.borrow().animation,
        }
    }

    /// Source flags.
    #[must_use]
    pub fn source_flags(&self) -> i32 {
        self.inner.borrow().flags
    }

    /// Event sequence.
    #[must_use]
    pub fn event_sequence(&self) -> i32 {
        self.inner.borrow().sequence
    }

    /// Respawn eligibility time.
    #[must_use]
    pub fn respawn_eligible_after_ms(&self) -> i32 {
        self.inner.borrow().respawn_time
    }

    /// Spawn count.
    #[must_use]
    pub fn spawns(&self) -> i32 {
        self.inner.borrow().spawn_count
    }

    /// Jump event.
    pub fn jump(&self) {
        Self::emit_inner(&self.inner, &self.services, &self.actor, entity_event::JUMP, 0);
    }

    fn emit_inner(
        inner: &Rc<RefCell<Q3CharacterInner>>,
        services: &Rc<S>,
        actor: &OwnedActor,
        event: i32,
        parameter: i32,
    ) {
        let sequence = {
            let mut guard = inner.borrow_mut();
            let sequence = guard.sequence;
            guard.sequence = guard.sequence.wrapping_add(1);
            sequence
        };
        services.emit(Q3CharacterEvent {
            actor: actor.clone(),
            sequence,
            time_ms: services.time_ms(),
            event,
            parameter,
        });
    }

    fn emit(&self, event: i32, parameter: i32) {
        Self::emit_inner(&self.inner, &self.services, &self.actor, event, parameter);
    }

    /// Capture a checkpoint.
    #[must_use]
    pub fn capture(&self) -> Q3CharacterCheckpoint {
        let inner = self.inner.borrow();
        Q3CharacterCheckpoint {
            version: 1,
            product: self.product,
            animation: inner.animation,
            flags: inner.flags,
            event_sequence: inner.sequence,
            respawn_time: inner.respawn_time,
            spawn_count: inner.spawn_count,
            dead: inner.dead,
            gibbed: inner.gibbed,
            initialized: inner.initialized,
        }
    }

    /// Restore a checkpoint.
    pub fn restore(&self, checkpoint: &Q3CharacterCheckpoint) -> Result<(), CharacterError> {
        if checkpoint.version != 1 || checkpoint.product != self.product {
            return Err(type_error("Q3 character checkpoint belongs to another source product"));
        }
        if !checkpoint.initialized && self.inner.borrow().initialized {
            return Err(failed(
                "Cannot restore an unadmitted Q3 character over an admitted binding",
            ));
        }
        if checkpoint.initialized && !self.inner.borrow().initialized {
            if self.services.read_body(self.actor.id()).is_none()
                || self.services.read_combat(self.actor.id()).is_none()
                || !self.services.has_inventory(self.actor.id())
            {
                return Err(failed("Restore shared Q3 character stores before private state"));
            }
            self.bind_callbacks();
        }
        let mut inner = self.inner.borrow_mut();
        inner.animation = checkpoint.animation;
        inner.flags = checkpoint.flags;
        inner.sequence = checkpoint.event_sequence;
        inner.respawn_time = checkpoint.respawn_time;
        inner.spawn_count = checkpoint.spawn_count;
        inner.dead = checkpoint.dead;
        inner.gibbed = checkpoint.gibbed;
        Ok(())
    }

    fn bind_callbacks(&self) {
        if self.inner.borrow().initialized {
            return;
        }
        let pain_inner = Rc::clone(&self.inner);
        let pain_services = Rc::clone(&self.services);
        let pain_actor = self.actor.clone();
        let die_inner = Rc::clone(&self.inner);
        let die_services = Rc::clone(&self.services);
        let die_actor = self.actor.clone();
        let die_animations = Rc::clone(&self.death_animations);
        self.services.bind_callbacks(
            &self.actor,
            Q3CharacterCallbackSet {
                pain: Box::new(move || {
                    let health = pain_services
                        .read_combat(pain_actor.id())
                        .map_or(0, |combat| combat.health);
                    Self::emit_inner(&pain_inner, &pain_services, &pain_actor, entity_event::PAIN, health);
                }),
                die: Box::new(move || {
                    Self::die_inner(&die_inner, &die_services, &die_actor, &die_animations)
                        .expect("Q3 character die failed");
                }),
            },
        );
        self.inner.borrow_mut().initialized = true;
    }

    /// Spawn the character.
    pub fn spawn(&self, input: &Q3CharacterSpawn) -> Result<(), CharacterError> {
        if self.services.read_combat(self.actor.id()).is_none() {
            self.services.create_combat(&self.actor, input.combat.clone());
        } else {
            self.services.set_health(&self.actor, input.combat.health);
            self.services.set_armor(&self.actor, input.combat.armor.clone());
            self.services
                .set_traits(&self.actor, CombatTraitChanges::from_combat(&input.combat));
        }
        if !self.services.has_inventory(self.actor.id()) {
            self.services.create_inventory(&self.actor, input.inventory.clone());
        } else {
            for entry in self.services.inventory_entries(self.actor.id()) {
                let mut cleared = entry.clone();
                cleared.count = 0.0;
                self.services.configure_inventory(&self.actor, cleared);
            }
            for entry in &input.inventory {
                self.services.configure_inventory(&self.actor, entry.clone());
            }
        }
        {
            let mut inner = self.inner.borrow_mut();
            inner.animation = q3_spawn_animation();
            inner.flags = (inner.flags & (4 | 0x4000 | 0x80000)) ^ 4;
            inner.dead = false;
            inner.gibbed = false;
            inner.respawn_time = self.services.time_ms();
            inner.spawn_count = inner.spawn_count.wrapping_add(1);
        }
        self.services.write_body(&self.actor, input.body.clone());
        self.bind_callbacks();
        if self.services.placement() != Q3Placement::SourceGame {
            self.services.kill_box(&self.actor);
            self.services.link_body(&self.actor);
            self.services.spawn_targets(&self.actor);
            if self.inner.borrow().spawn_count > 1 {
                self.emit(entity_event::PLAYER_TELEPORT_IN, 0);
            }
        }
        Ok(())
    }

    fn gib_inner(inner: &Rc<RefCell<Q3CharacterInner>>, services: &Rc<S>, actor: &OwnedActor, killer_source_slot: i32) {
        {
            let mut guard = inner.borrow_mut();
            guard.gibbed = true;
            guard.flags |= 0x80;
        }
        services.set_traits(
            actor,
            CombatTraitChanges {
                can_take_damage: Some(false),
                ..CombatTraitChanges::default()
            },
        );
        services.unlink_body(actor);
        Self::emit_inner(inner, services, actor, entity_event::GIB_PLAYER, killer_source_slot);
    }

    fn die_inner(
        inner: &Rc<RefCell<Q3CharacterInner>>,
        services: &Rc<S>,
        actor: &OwnedActor,
        death_animations: &Rc<RefCell<Q3DeathAnimationSequence>>,
    ) -> Result<(), CharacterError> {
        if inner.borrow().gibbed {
            return Ok(());
        }
        let health = services
            .read_combat(actor.id())
            .map(|combat| combat.health)
            .ok_or_else(|| failed("Q3 character has no health binding"))?;
        let context = services.death_context(actor);
        if inner.borrow().dead {
            if health <= -40 && context.blood {
                Self::gib_inner(inner, services, actor, context.killer_source_slot);
            } else if !context.blood && health <= -40 {
                services.set_health(actor, -39);
            }
            return Ok(());
        }
        {
            let mut guard = inner.borrow_mut();
            guard.dead = true;
            guard.flags |= 1;
            guard.respawn_time = services.time_ms().wrapping_add(1700);
        }
        if let Some(body) = services.read_body(actor.id()) {
            services.write_body(
                actor,
                BodyState {
                    angles: vec3(0.0, body.angles.y, 0.0),
                    bounds: Bounds {
                        max: vec3(body.bounds.max.x, body.bounds.max.y, -8.0),
                        ..body.bounds
                    },
                    ..body
                },
            );
            services.link_body(actor);
        }
        if (health <= -40 && !context.no_drop && context.blood) || context.suicide {
            Self::gib_inner(inner, services, actor, context.killer_source_slot);
        } else {
            if health <= -40 {
                services.set_health(actor, -39);
            }
            let (animation, event) = death_animations.borrow_mut().next();
            {
                let mut guard = inner.borrow_mut();
                let AnimationState::Q3 { legs, torso, .. } = &mut guard.animation else {
                    return Err(type_error("Q3 character animation requires Q3 animation"));
                };
                *legs = (*legs & ANIMATION_TOGGLE_BIT ^ ANIMATION_TOGGLE_BIT) | animation;
                *torso = (*torso & ANIMATION_TOGGLE_BIT ^ ANIMATION_TOGGLE_BIT) | animation;
            }
            Self::emit_inner(inner, services, actor, event, context.killer_source_slot);
        }
        Ok(())
    }

    /// Run death after lethal damage committed.
    pub fn die(&self) -> Result<(), CharacterError> {
        Self::die_inner(&self.inner, &self.services, &self.actor, &self.death_animations)
    }

    /// Respawn eligibility (`wantsRespawn`).
    #[must_use]
    pub fn wants_respawn(&self, now_ms: i32, attack: bool, use_holdable: bool, force_respawn_seconds: i32) -> bool {
        let inner = self.inner.borrow();
        if !inner.dead || now_ms <= inner.respawn_time {
            return false;
        }
        attack
            || use_holdable
            || (force_respawn_seconds > 0
                && now_ms.wrapping_sub(inner.respawn_time) > force_respawn_seconds.wrapping_mul(1000))
    }

    /// Commit an animation from the animation owner.
    pub fn commit_animation(&self, animation: &ActorAnimationState) -> Result<(), CharacterError> {
        if animation.provider != self.provider {
            return Err(type_error("Q3 character animation belongs to another provider"));
        }
        if !matches!(animation.state, AnimationState::Q3 { .. }) {
            return Err(type_error("Q3 character animation requires Q3 animation"));
        }
        self.inner.borrow_mut().animation = animation.state;
        Ok(())
    }
}

/// Step semantic locomotion into Q3 clips (`stepQ3CharacterAnimation`).
/// The donor throws a `TypeError` for non-Q3 animation; the merged
/// operation reports it as [`CharacterError::Type`].
pub fn step_q3_character_animation(
    input: &AnimationStepInput,
    product: Q3Product,
    dead: bool,
    event_sequence: i32,
) -> Result<Q3AnimationStepResult, CharacterError> {
    let context = Q3AnimationContext {
        animation: input.animation.clone(),
        dead,
        elapsed_milliseconds: input.frame.elapsed.as_milliseconds_truncated(),
        buttons: 0,
        product,
        event_sequence,
    };
    let dropped = run_q3_animation_operation(Q3AnimationRequest::DropTimers, &context)
        .map_err(|error| type_error(error.to_string()))?;
    let animation = match input.locomotion {
        LocomotionAnimation::Idle => player_animation::LEGS_IDLE,
        LocomotionAnimation::Walk => {
            if input.backwards {
                player_animation::LEGS_BACKWALK
            } else {
                player_animation::LEGS_WALK
            }
        }
        LocomotionAnimation::Run => {
            if input.backwards {
                player_animation::LEGS_BACK
            } else {
                player_animation::LEGS_RUN
            }
        }
        LocomotionAnimation::Backward => player_animation::LEGS_BACK,
        LocomotionAnimation::Crouch => {
            if input.backwards {
                player_animation::LEGS_BACKCR
            } else {
                player_animation::LEGS_WALKCR
            }
        }
        LocomotionAnimation::Jump => {
            if input.backwards {
                player_animation::LEGS_JUMPB
            } else {
                player_animation::LEGS_JUMP
            }
        }
        LocomotionAnimation::Land => {
            if input.backwards {
                player_animation::LEGS_LANDB
            } else {
                player_animation::LEGS_LAND
            }
        }
        LocomotionAnimation::Swim => player_animation::LEGS_SWIM,
    };
    let selected = run_q3_animation_operation(
        Q3AnimationRequest::Legs {
            animation,
            force: input.force,
        },
        &Q3AnimationContext {
            animation: dropped.animation.clone(),
            ..context.clone()
        },
    )
    .map_err(|error| type_error(error.to_string()))?;
    if input.locomotion != LocomotionAnimation::Land {
        let mut effects = dropped.effects;
        effects.extend(selected.effects);
        return Ok(Q3AnimationStepResult {
            animation: selected.animation,
            effects,
        });
    }
    let landed = run_q3_animation_operation(
        Q3AnimationRequest::LegsTimer { milliseconds: 130 },
        &Q3AnimationContext {
            animation: selected.animation.clone(),
            ..context
        },
    )
    .map_err(|error| type_error(error.to_string()))?;
    let mut effects = dropped.effects;
    effects.extend(selected.effects);
    effects.extend(landed.effects);
    Ok(Q3AnimationStepResult {
        animation: landed.animation,
        effects,
    })
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::collections::HashMap;
    use std::rc::Rc;

    use qa_core::identity::{IdentityOwner, SavedActorId};
    use qa_core::time::{FramePhase, SourceTime};
    use qa_world::movement::q3::types::q3_animation_state;

    use super::*;

    fn test_frame(elapsed: SourceTime) -> FrameContext {
        FrameContext {
            frame: 1,
            time: SourceTime::Milliseconds(100),
            elapsed,
            phase: FramePhase::FrameEntry,
        }
    }

    fn test_actor() -> (OwnedActor, ProviderId) {
        let owner = IdentityOwner::create("test").unwrap();
        let provider = ProviderId::new("q3", "test");
        let owned = owner.owned_actor(&owner.actor(3, 1), provider.clone()).unwrap();
        (owned, provider)
    }

    fn actor_key(actor: &ActorId) -> SavedActorId {
        SavedActorId::from(actor)
    }

    struct FakeServices {
        bodies: RefCell<HashMap<SavedActorId, BodyState>>,
        linked: RefCell<HashMap<SavedActorId, bool>>,
        combat: RefCell<HashMap<SavedActorId, CombatState>>,
        inventory: RefCell<HashMap<SavedActorId, Vec<InventoryEntry>>>,
        events: RefCell<Vec<Q3CharacterEvent>>,
        time: Cell<i32>,
        placement: Q3Placement,
        death_ctx: Q3CharacterDeathContext,
        callbacks: RefCell<HashMap<SavedActorId, Q3CharacterCallbackSet>>,
        spawn_targets_calls: Cell<u32>,
        kill_box_calls: Cell<u32>,
    }

    impl FakeServices {
        fn new(placement: Q3Placement) -> Self {
            Self {
                bodies: RefCell::new(HashMap::new()),
                linked: RefCell::new(HashMap::new()),
                combat: RefCell::new(HashMap::new()),
                inventory: RefCell::new(HashMap::new()),
                events: RefCell::new(Vec::new()),
                time: Cell::new(1000),
                placement,
                death_ctx: Q3CharacterDeathContext {
                    blood: true,
                    no_drop: false,
                    suicide: false,
                    killer_source_slot: 2,
                },
                callbacks: RefCell::new(HashMap::new()),
                spawn_targets_calls: Cell::new(0),
                kill_box_calls: Cell::new(0),
            }
        }

        fn fire_pain(&self, actor: &ActorId) {
            let guard = self.callbacks.borrow();
            (guard.get(&actor_key(actor)).unwrap().pain)();
        }

        fn fire_die(&self, actor: &ActorId) {
            let guard = self.callbacks.borrow();
            (guard.get(&actor_key(actor)).unwrap().die)();
        }
    }

    impl Q3CharacterServices for FakeServices {
        fn read_body(&self, actor: &ActorId) -> Option<BodyState> {
            self.bodies.borrow().get(&actor_key(actor)).cloned()
        }

        fn write_body(&self, actor: &OwnedActor, body: BodyState) {
            self.bodies.borrow_mut().insert(actor_key(actor.id()), body);
        }

        fn link_body(&self, actor: &OwnedActor) {
            self.linked.borrow_mut().insert(actor_key(actor.id()), true);
        }

        fn unlink_body(&self, actor: &OwnedActor) {
            self.linked.borrow_mut().insert(actor_key(actor.id()), false);
        }

        fn bind_callbacks(&self, actor: &OwnedActor, callbacks: Q3CharacterCallbackSet) {
            self.callbacks.borrow_mut().insert(actor_key(actor.id()), callbacks);
        }

        fn read_combat(&self, actor: &ActorId) -> Option<CombatState> {
            self.combat.borrow().get(&actor_key(actor)).cloned()
        }

        fn create_combat(&self, actor: &OwnedActor, combat: CombatState) {
            self.combat.borrow_mut().insert(actor_key(actor.id()), combat);
        }

        fn set_health(&self, actor: &OwnedActor, health: i32) {
            if let Some(combat) = self.combat.borrow_mut().get_mut(&actor_key(actor.id())) {
                combat.health = health;
            }
        }

        fn set_armor(&self, actor: &OwnedActor, armor: ArmorState) {
            if let Some(combat) = self.combat.borrow_mut().get_mut(&actor_key(actor.id())) {
                combat.armor = armor;
            }
        }

        fn set_traits(&self, actor: &OwnedActor, traits: CombatTraitChanges) {
            if let Some(combat) = self.combat.borrow_mut().get_mut(&actor_key(actor.id())) {
                if let Some(value) = traits.can_take_damage {
                    combat.can_take_damage = value;
                }
                if let Some(value) = traits.mass {
                    combat.mass = value;
                }
                if let Some(value) = traits.invulnerable {
                    combat.invulnerable = value;
                }
                if let Some(value) = traits.team {
                    combat.team = value;
                }
                if let Some(value) = traits.no_knockback {
                    combat.no_knockback = value;
                }
            }
        }

        fn has_inventory(&self, actor: &ActorId) -> bool {
            self.inventory.borrow().contains_key(&actor_key(actor))
        }

        fn create_inventory(&self, actor: &OwnedActor, entries: Vec<InventoryEntry>) {
            self.inventory.borrow_mut().insert(actor_key(actor.id()), entries);
        }

        fn inventory_entries(&self, actor: &ActorId) -> Vec<InventoryEntry> {
            self.inventory
                .borrow()
                .get(&actor_key(actor))
                .cloned()
                .unwrap_or_default()
        }

        fn configure_inventory(&self, actor: &OwnedActor, entry: InventoryEntry) {
            let mut guard = self.inventory.borrow_mut();
            let entries = guard.entry(actor_key(actor.id())).or_default();
            if let Some(existing) = entries.iter_mut().find(|item| item.item == entry.item) {
                *existing = entry;
            } else {
                entries.push(entry);
            }
        }

        fn time_ms(&self) -> i32 {
            self.time.get()
        }

        fn emit(&self, event: Q3CharacterEvent) {
            self.events.borrow_mut().push(event);
        }

        fn death_context(&self, _actor: &OwnedActor) -> Q3CharacterDeathContext {
            self.death_ctx
        }

        fn placement(&self) -> Q3Placement {
            self.placement
        }

        fn spawn_targets(&self, _actor: &OwnedActor) {
            self.spawn_targets_calls.set(self.spawn_targets_calls.get() + 1);
        }

        fn kill_box(&self, _actor: &OwnedActor) {
            self.kill_box_calls.set(self.kill_box_calls.get() + 1);
        }
    }

    fn spawn_input() -> Q3CharacterSpawn {
        Q3CharacterSpawn {
            body: BodyState {
                origin: vec3(0.0, 0.0, 0.0),
                angles: vec3(0.0, 90.0, 0.0),
                velocity: vec3(0.0, 0.0, 0.0),
                bounds: Q3_CHARACTER_BOUNDS,
                ground: None,
            },
            combat: q3_initial_combat("100", None).unwrap(),
            inventory: Vec::new(),
        }
    }

    #[test]
    fn character_spawns_dies_and_gibs() {
        let (actor, provider) = test_actor();
        let services = Rc::new(FakeServices::new(Q3Placement::Character));
        let deaths = Rc::new(RefCell::new(Q3DeathAnimationSequence::new()));
        let character = Q3CharacterActor::new(
            actor.clone(),
            provider.clone(),
            Q3Product::BaseQ3,
            Rc::clone(&services),
            Rc::clone(&deaths),
        );
        character.spawn(&spawn_input()).unwrap();
        assert_eq!(character.spawns(), 1);
        assert_eq!(character.source_flags(), 4);
        assert_eq!(services.kill_box_calls.get(), 1);
        assert!(services.events.borrow().is_empty());
        character.spawn(&spawn_input()).unwrap();
        assert_eq!(services.events.borrow().len(), 1);
        assert_eq!(services.events.borrow()[0].event, entity_event::PLAYER_TELEPORT_IN);
        character.jump();
        assert_eq!(services.events.borrow().len(), 2);

        services.fire_pain(actor.id());
        let pain = services.events.borrow_mut().pop().unwrap();
        assert_eq!(pain.event, entity_event::PAIN);
        assert_eq!(pain.parameter, 125);

        character.die().unwrap();
        let death = services.events.borrow_mut().pop().unwrap();
        assert_eq!(death.event, entity_event::DEATH1);
        assert_eq!(death.parameter, 2);
        assert_eq!(
            q3_animation_state(&character.animation()).unwrap().0 & !ANIMATION_TOGGLE_BIT,
            player_animation::BOTH_DEATH1
        );
        assert_eq!(character.respawn_eligible_after_ms(), 2700);
        assert!(!character.wants_respawn(2000, true, false, 0));
        assert!(character.wants_respawn(3000, true, false, 0));

        services.set_health(&actor, -50);
        character.die().unwrap();
        let gib = services.events.borrow_mut().pop().unwrap();
        assert_eq!(gib.event, entity_event::GIB_PLAYER);
        assert_eq!(character.source_flags() & 0x80, 0x80);
        assert_eq!(services.linked.borrow().get(&actor_key(actor.id())), Some(&false));
        character.die().unwrap();

        let checkpoint = character.capture();
        assert_eq!(checkpoint.version, 1);
        assert!(checkpoint.dead && checkpoint.gibbed && checkpoint.initialized);
        let owner2 = IdentityOwner::create("test2").unwrap();
        let actor2 = owner2.owned_actor(&owner2.actor(9, 1), provider.clone()).unwrap();
        let other = Q3CharacterActor::new(
            actor2,
            provider,
            Q3Product::BaseQ3,
            Rc::clone(&services),
            Rc::clone(&deaths),
        );
        assert!(other.restore(&checkpoint).is_err());
        other.spawn(&spawn_input()).unwrap();
        let foreign = Q3CharacterCheckpoint {
            product: Q3Product::MissionPack,
            ..checkpoint.clone()
        };
        assert!(other.restore(&foreign).is_err());
        let unadmitted = Q3CharacterCheckpoint {
            initialized: false,
            ..checkpoint.clone()
        };
        assert!(other.restore(&unadmitted).is_err());
    }

    #[test]
    fn character_suicide_gibs_and_death_cycles() {
        let (actor, provider) = test_actor();
        let services = Rc::new(FakeServices {
            death_ctx: Q3CharacterDeathContext {
                blood: false,
                no_drop: false,
                suicide: true,
                killer_source_slot: 0,
            },
            ..FakeServices::new(Q3Placement::SourceGame)
        });
        let character = Q3CharacterActor::new(
            actor.clone(),
            provider.clone(),
            Q3Product::BaseQ3,
            Rc::clone(&services),
            Rc::new(RefCell::new(Q3DeathAnimationSequence::new())),
        );
        character.spawn(&spawn_input()).unwrap();
        assert_eq!(services.kill_box_calls.get(), 0);
        character.die().unwrap();
        assert_eq!(
            services.events.borrow_mut().pop().unwrap().event,
            entity_event::GIB_PLAYER
        );

        let mut sequence = Q3DeathAnimationSequence::new();
        assert_eq!(sequence.next(), (0, entity_event::DEATH1));
        assert_eq!(sequence.next(), (2, entity_event::DEATH2));
        assert_eq!(sequence.next(), (4, entity_event::DEATH3));
        assert_eq!(sequence.next(), (0, entity_event::DEATH1));
        let checkpoint = sequence.capture();
        sequence.next();
        sequence.restore(&checkpoint).unwrap();
        assert_eq!(sequence.next(), (2, entity_event::DEATH2));
        assert!(sequence
            .restore(&Q3DeathAnimationCheckpoint { version: 2, index: 0 })
            .is_err());

        assert_eq!(q3_maximum_health("80").unwrap(), 80);
        assert_eq!(q3_maximum_health("500").unwrap(), 100);
        assert_eq!(
            q3_initial_combat("90", Some("red".to_string())).unwrap(),
            CombatState {
                health: 115,
                armor: ArmorState {
                    regular: RegularArmorState::Q3 {
                        points: 0.0,
                        protection: f64::from(0.66f32),
                    },
                    powered: PoweredProtectionState::None,
                },
                mass: 200.0,
                can_take_damage: true,
                invulnerable: false,
                no_knockback: false,
                team: Some("red".to_string()),
            }
        );

        let wrong = ActorAnimationState {
            provider: ProviderId::new("q3", "other"),
            state: q3_spawn_animation(),
        };
        assert!(character.commit_animation(&wrong).is_err());
        let own = ActorAnimationState {
            provider,
            state: AnimationState::Q3 {
                legs: 5,
                torso: player_animation::TORSO_STAND,
                legs_timer_milliseconds: 0,
                torso_timer_milliseconds: 0,
            },
        };
        character.commit_animation(&own).unwrap();
        assert_eq!(q3_animation_state(&character.animation()).unwrap().0, 5);
        assert!(character.wants_respawn(5000, false, false, 1));
        services.fire_die(actor.id());
    }

    #[test]
    fn character_animation_steps_map_locomotion() {
        let (_, provider) = test_actor();
        let input = AnimationStepInput {
            frame: test_frame(SourceTime::Milliseconds(16)),
            animation: ActorAnimationState {
                provider,
                state: AnimationState::Q3 {
                    legs: player_animation::LEGS_IDLE,
                    torso: player_animation::TORSO_STAND,
                    legs_timer_milliseconds: 100,
                    torso_timer_milliseconds: 0,
                },
            },
            locomotion: LocomotionAnimation::Run,
            backwards: true,
            force: true,
        };
        let result = step_q3_character_animation(&input, Q3Product::BaseQ3, false, 3).unwrap();
        assert_eq!(
            q3_animation_state(&result.animation).unwrap().0 & !ANIMATION_TOGGLE_BIT,
            player_animation::LEGS_BACK
        );
        assert_eq!(q3_animation_state(&result.animation).unwrap().2, 0);
        assert!(!result.effects.is_empty());

        let land = AnimationStepInput {
            locomotion: LocomotionAnimation::Land,
            backwards: false,
            force: true,
            ..input.clone()
        };
        let result = step_q3_character_animation(&land, Q3Product::BaseQ3, false, 3).unwrap();
        assert_eq!(q3_animation_state(&result.animation).unwrap().2, 130);

        let dead = step_q3_character_animation(&input, Q3Product::BaseQ3, true, 3).unwrap();
        assert_eq!(
            q3_animation_state(&dead.animation).unwrap().0 & !ANIMATION_TOGGLE_BIT,
            player_animation::LEGS_IDLE
        );
    }
}
