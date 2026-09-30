//! Quake III foundation: character.
//!
//! Donor provenance: `src/content/q3/foundation/character.ts`.

use crate::contract::{ArmorState, InventoryEntry, PoweredProtectionState, RegularArmorState};
use qa_core::identity::{ActorId, OwnedActor, ProviderId};
use qa_core::math::{vec3, Bounds, Vec3};
use qa_core::time::SourceTime;
use qa_world::movement::types::AnimationState;
use std::cell::RefCell;
use std::rc::Rc;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::foundation::animation::*;
use crate::q3::foundation::arsenal::*;
use crate::q3::foundation::mirrors::*;

// ---------------------------------------------------------------------------
// character.ts: ClientSpawn, player_die, character animation admission.
// ---------------------------------------------------------------------------

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
    pub animation: Q3AnimationState,
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
    pub fn restore(&mut self, checkpoint: &Q3DeathAnimationCheckpoint) -> Result<(), Q3FoundationError> {
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
            (Q3PlayerAnimation::BOTH_DEATH1, Q3EntityEvent::DEATH1)
        } else if self.index == 1 {
            (Q3PlayerAnimation::BOTH_DEATH2, Q3EntityEvent::DEATH2)
        } else {
            (Q3PlayerAnimation::BOTH_DEATH3, Q3EntityEvent::DEATH3)
        };
        self.index = (self.index + 1) % 3;
        result
    }
}

/// Maximum health from a handicap (`q3MaximumHealth`).
pub fn q3_maximum_health(handicap: &str) -> Result<i32, Q3FoundationError> {
    let maximum = game_atoi(handicap)?;
    Ok(if !(1..=100).contains(&maximum) { 100 } else { maximum })
}

/// Initial combat state (`q3InitialCombat`).
pub fn q3_initial_combat(handicap: &str, team: Option<String>) -> Result<CombatState, Q3FoundationError> {
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
    animation: Q3AnimationState,
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

/// Spawn animation in the hook-owned shape. The merged timers are integers,
/// so the widening is exact.
fn spawn_animation_state() -> Q3AnimationState {
    let AnimationState::Q3 {
        legs,
        torso,
        legs_timer_milliseconds,
        torso_timer_milliseconds,
    } = q3_spawn_animation()
    else {
        panic!("Q3 spawn animation is always Q3");
    };
    Q3AnimationState {
        legs,
        torso,
        legs_timer_ms: f64::from(legs_timer_milliseconds),
        torso_timer_ms: f64::from(torso_timer_milliseconds),
    }
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
                animation: spawn_animation_state(),
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
            state: self.inner.borrow().animation.clone(),
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
        Self::emit_inner(&self.inner, &self.services, &self.actor, Q3EntityEvent::JUMP, 0);
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
            animation: inner.animation.clone(),
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
    pub fn restore(&self, checkpoint: &Q3CharacterCheckpoint) -> Result<(), Q3FoundationError> {
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
        inner.animation = checkpoint.animation.clone();
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
                    Self::emit_inner(&pain_inner, &pain_services, &pain_actor, Q3EntityEvent::PAIN, health);
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
    pub fn spawn(&self, input: &Q3CharacterSpawn) -> Result<(), Q3FoundationError> {
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
            inner.animation = spawn_animation_state();
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
                self.emit(Q3EntityEvent::PLAYER_TELEPORT_IN, 0);
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
        Self::emit_inner(inner, services, actor, Q3EntityEvent::GIB_PLAYER, killer_source_slot);
    }

    fn die_inner(
        inner: &Rc<RefCell<Q3CharacterInner>>,
        services: &Rc<S>,
        actor: &OwnedActor,
        death_animations: &Rc<RefCell<Q3DeathAnimationSequence>>,
    ) -> Result<(), Q3FoundationError> {
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
                guard.animation.legs = (guard.animation.legs & ANIMATION_TOGGLE_BIT ^ ANIMATION_TOGGLE_BIT) | animation;
                guard.animation.torso =
                    (guard.animation.torso & ANIMATION_TOGGLE_BIT ^ ANIMATION_TOGGLE_BIT) | animation;
            }
            Self::emit_inner(inner, services, actor, event, context.killer_source_slot);
        }
        Ok(())
    }

    /// Run death after lethal damage committed.
    pub fn die(&self) -> Result<(), Q3FoundationError> {
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
    pub fn commit_animation(&self, animation: &ActorAnimationState) -> Result<(), Q3FoundationError> {
        if animation.provider != self.provider {
            return Err(type_error("Q3 character animation belongs to another provider"));
        }
        self.inner.borrow_mut().animation = animation.state.clone();
        Ok(())
    }
}

/// Step semantic locomotion into Q3 clips (`stepQ3CharacterAnimation`).
#[must_use]
pub fn step_q3_character_animation(
    input: &AnimationStepInput,
    product: Q3Product,
    dead: bool,
    event_sequence: i32,
) -> AnimationStepResult {
    let elapsed_ms = match input.frame.elapsed {
        SourceTime::Milliseconds(value) => f64::from(value),
        SourceTime::Seconds(value) => (f64::from(value) * 1000.0).trunc(),
    };
    let context = Q3AnimationContext {
        animation: input.animation.clone(),
        dead,
        elapsed_ms,
        buttons: 0,
        product,
        event_sequence,
    };
    let dropped = run_q3_animation_operation(&Q3AnimationRequest::DropTimers, &context);
    let animation = match input.locomotion {
        LocomotionAnimation::Idle => Q3PlayerAnimation::LEGS_IDLE,
        LocomotionAnimation::Walk => {
            if input.backwards {
                Q3PlayerAnimation::LEGS_BACKWALK
            } else {
                Q3PlayerAnimation::LEGS_WALK
            }
        }
        LocomotionAnimation::Run => {
            if input.backwards {
                Q3PlayerAnimation::LEGS_BACK
            } else {
                Q3PlayerAnimation::LEGS_RUN
            }
        }
        LocomotionAnimation::Backward => Q3PlayerAnimation::LEGS_BACK,
        LocomotionAnimation::Crouch => {
            if input.backwards {
                Q3PlayerAnimation::LEGS_BACKCR
            } else {
                Q3PlayerAnimation::LEGS_WALKCR
            }
        }
        LocomotionAnimation::Jump => {
            if input.backwards {
                Q3PlayerAnimation::LEGS_JUMPB
            } else {
                Q3PlayerAnimation::LEGS_JUMP
            }
        }
        LocomotionAnimation::Land => {
            if input.backwards {
                Q3PlayerAnimation::LEGS_LANDB
            } else {
                Q3PlayerAnimation::LEGS_LAND
            }
        }
        LocomotionAnimation::Swim => Q3PlayerAnimation::LEGS_SWIM,
    };
    let selected = run_q3_animation_operation(
        &Q3AnimationRequest::Legs {
            animation,
            force: input.force,
        },
        &Q3AnimationContext {
            animation: dropped.animation.clone(),
            ..context.clone()
        },
    );
    if input.locomotion != LocomotionAnimation::Land {
        let mut effects = dropped.effects;
        effects.extend(selected.effects);
        return AnimationStepResult {
            animation: selected.animation,
            effects,
        };
    }
    let landed = run_q3_animation_operation(
        &Q3AnimationRequest::LegsTimer { milliseconds: 130 },
        &Q3AnimationContext {
            animation: selected.animation.clone(),
            ..context
        },
    );
    let mut effects = dropped.effects;
    effects.extend(selected.effects);
    effects.extend(landed.effects);
    AnimationStepResult {
        animation: landed.animation,
        effects,
    }
}
