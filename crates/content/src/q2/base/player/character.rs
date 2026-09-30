//! Q2 character actor (`src/content/q2/base/player/character.ts`).

use std::collections::BTreeMap;

use qa_core::identity::{ActorId, OwnedActor, SavedActorId};
use qa_core::math::{Vec3, add3, dot3, vec3};

use crate::contract::ItemId;
use crate::q2::foundation::checkpoint::{restore_q2_attack, save_q2_actor, save_q2_attack};
use crate::q2::foundation::host::{
    Q2Edition, Q2Entity, Q2Mode, Q2ModelEvent, Q2MotionKind, Q2PresentationEvent, Q2Solid,
    Q2SoundEvent, Q2SoundLoop, Q2SpawnFields,
};
use crate::q2::foundation::items::Q2PlayerPowerups;
use crate::q2::foundation::weapons::vectors::angle_vectors;
use crate::q2::support::contracts::{
    BodyState, CombatState, CombatTraitChanges, DamageDecision, DamageFeedback, DamageReactionKind,
    DeathReaction,
};
use crate::q2::support::tables::{Q2BodyTable, Q2CombatAuthority, Q2InventoryTable};

use super::checkpoint::{Q2CharacterCheckpoint, Q2CharacterEntityFields, Q2PlayerStateCheckpoint};
use super::environment::{q2_falling_damage, q2_world_effects};
use super::types::{
    Q2BodyChanges, Q2CharacterContext, Q2CharacterWeapon, Q2PlayerMovement, Q2PlayerRules,
    Q2PlayerState, Q2PlayerView,
};
use super::view::{
    emit_player_effect, q2_build_view, q2_client_animation, q2_client_effects, q2_damage_feedback,
    q2_death_animation_frames,
};

/// Character gib (`Q2CharacterGib`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2CharacterGib {
    /// Model.
    pub model: String,
    /// Origin.
    pub origin: Vec3,
    /// Velocity.
    pub velocity: Vec3,
    /// Angular velocity.
    pub angular_velocity: Vec3,
    /// Expiry time.
    pub expires_at: f64,
}

/// Character host (`Q2CharacterHost`).
pub trait Q2CharacterHost {
    /// Body table.
    fn bodies(&mut self) -> &mut dyn Q2BodyTable;
    /// Combat authority.
    fn combat(&mut self) -> &mut dyn Q2CombatAuthority;
    /// Inventory table.
    fn inventory(&mut self) -> &mut dyn Q2InventoryTable;
    /// Current time.
    fn now(&self) -> f64;
    /// Random draw.
    fn random(&mut self) -> f64;
    /// Movement observation.
    fn movement(&mut self, actor: &ActorId) -> Q2PlayerMovement;
    /// Point contents.
    fn point_contents(&mut self, point: Vec3) -> i32;
    /// Powerups.
    fn powerups(&mut self, actor: &ActorId) -> Q2PlayerPowerups;
    /// Weapon observation.
    fn weapon(&mut self, actor: &ActorId) -> Option<Q2CharacterWeapon>;
    /// Emit a presentation event.
    fn emit(&mut self, event: Q2PresentationEvent);
    /// Publish a view.
    fn view(&mut self, actor: &ActorId, view: Q2PlayerView);
    /// Emit noise.
    fn noise(&mut self, actor: &ActorId, origin: Vec3);
    /// Apply environment damage.
    fn environment_damage(&mut self, actor: &OwnedActor, amount: f64, means: i32, flags: i32);
    /// React to a death.
    fn died(&mut self, actor: &OwnedActor, reaction: &DeathReaction);
    /// Request a respawn.
    fn request_respawn(&mut self, actor: &OwnedActor);
    /// Set motion and solidity.
    fn motion(&mut self, actor: &OwnedActor, kind: Q2MotionKind, solid: Q2Solid);
    /// Spawn a gib.
    fn spawn_gib(&mut self, gib: Q2CharacterGib);
}

/// Character options (`Q2CharacterOptions`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2CharacterOptions {
    /// Edition.
    pub edition: Option<Q2Edition>,
    /// Model.
    pub model: String,
    /// Skin.
    pub skin: i32,
    /// Slot.
    pub slot: i32,
    /// Mode.
    pub mode: Q2Mode,
    /// Deathmatch flags.
    pub deathmatch_flags: i32,
    /// Whether environment runs.
    pub environment: bool,
    /// View rules override.
    pub view_rules: Option<Q2PlayerRules>,
}

/// Character animation priority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q2CharacterAnimation {
    /// Attack.
    Attack,
    /// Pain.
    Pain,
    /// Reverse.
    Reverse,
}

/// A Q2 character on an existing actor (`Q2CharacterActor`).
///
/// It allocates no actor, body, arsenal or inventory.
pub struct Q2CharacterActor {
    /// Owned actor.
    pub actor: OwnedActor,
    /// Entity continuation.
    pub entity: Q2Entity,
    /// Player state.
    pub state: Q2PlayerState,
    /// Rules.
    pub rules: Q2PlayerRules,
    /// Options.
    pub options: Q2CharacterOptions,
    /// Pain index.
    pain_index: i32,
    /// Death index.
    death_index: i32,
}

/// Character context over an actor and host.
struct CharacterContext<'a> {
    /// Character.
    character: &'a mut Q2CharacterActor,
    /// Host.
    host: &'a mut dyn Q2CharacterHost,
}

impl Q2CharacterContext for CharacterContext<'_> {
    fn actor_id(&self) -> ActorId {
        self.character.actor.id().clone()
    }

    fn owned_actor(&self) -> OwnedActor {
        self.character.actor.clone()
    }

    fn now(&mut self) -> f64 {
        self.host.now()
    }

    fn random(&mut self) -> f64 {
        self.host.random()
    }

    fn movement(&mut self) -> Q2PlayerMovement {
        let actor = self.character.actor.id().clone();
        self.host.movement(&actor)
    }

    fn rules(&self) -> Q2PlayerRules {
        self.character.rules.clone()
    }

    fn powerups(&mut self) -> Q2PlayerPowerups {
        let actor = self.character.actor.id().clone();
        self.host.powerups(&actor)
    }

    fn weapon_state(&mut self) -> Option<Q2CharacterWeapon> {
        let actor = self.character.actor.id().clone();
        self.host.weapon(&actor)
    }

    fn environment_damage(&mut self, amount: f64, means: i32, flags: i32) {
        let actor = self.character.actor.clone();
        self.host.environment_damage(&actor, amount, means, flags);
    }

    fn noise(&mut self, origin: Vec3) {
        let actor = self.character.actor.id().clone();
        self.host.noise(&actor, origin);
    }

    fn body(&mut self) -> BodyState {
        let actor = self.character.actor.id().clone();
        self.host
            .bodies()
            .read(&actor)
            .expect("Q2 character actor has no shared body")
    }

    fn move_body(&mut self, changes: Q2BodyChanges, link: bool) {
        let mut body = self.body();
        if let Some(origin) = changes.origin {
            body.origin = origin;
        }
        if let Some(angles) = changes.angles {
            body.angles = angles;
        }
        if let Some(velocity) = changes.velocity {
            body.velocity = velocity;
        }
        if let Some(bounds) = changes.bounds {
            body.bounds = bounds;
        }
        if let Some(ground) = changes.ground {
            body.ground = ground;
        }
        let actor = self.character.actor.clone();
        self.host.bodies().write(&actor, &body);
        if link {
            self.host.bodies().link(&actor, None);
        }
    }

    fn sound(&mut self, path: &str, channel: i32, volume: f64, attenuation: f64) {
        let origin = self.body().origin;
        let actor = self.character.actor.id().clone();
        self.host.emit(Q2PresentationEvent::Sound(Q2SoundEvent {
            actor: Some(actor),
            origin,
            path: path.to_string(),
            channel,
            volume,
            attenuation,
            reliable: false,
            loop_: Q2SoundLoop::Once,
            loop_owner: None,
        }));
    }

    fn emit(&mut self, event: Q2PresentationEvent) {
        self.host.emit(event);
    }

    fn point_contents(&mut self, point: Vec3) -> i32 {
        self.host.point_contents(point)
    }

    fn combat(&mut self) -> Option<CombatState> {
        let actor = self.character.actor.id().clone();
        self.host.combat().read(&actor)
    }

    fn inventory_count(&mut self, item: &ItemId) -> f64 {
        let actor = self.character.actor.id().clone();
        self.host.inventory().count(&actor, item)
    }

    fn mode(&self) -> Q2Mode {
        self.character.options.mode
    }

    fn deathmatch_flags(&self) -> i32 {
        self.character.options.deathmatch_flags
    }

    fn edition(&self) -> Q2Edition {
        self.character.options.edition.unwrap_or(Q2Edition::Classic)
    }

    fn state_snapshot(&mut self) -> Q2PlayerState {
        self.character.state.clone()
    }

    fn with_state<R>(&mut self, f: impl FnOnce(&mut Q2PlayerState) -> R) -> R {
        f(&mut self.character.state)
    }

    fn entity_snapshot(&mut self) -> Q2Entity {
        self.character.entity.clone()
    }

    fn with_entity<R>(&mut self, f: impl FnOnce(&mut Q2Entity) -> R) -> R {
        f(&mut self.character.entity)
    }
}

impl Q2CharacterActor {
    /// Build a character on an existing actor.
    pub fn new(actor: OwnedActor, options: Q2CharacterOptions, now: f64) -> Self {
        let mut entity = Q2Entity::new(
            actor.clone(),
            Q2SpawnFields {
                ordinal: -1,
                classname: "player".to_string(),
                values: BTreeMap::new(),
            },
        );
        entity.model = options.model.clone();
        entity.skin = options.skin;
        entity.view_height = 22;
        entity.render_flags = if options.edition == Some(Q2Edition::Rerelease) {
            32768
        } else {
            0
        };
        let mut state = Q2PlayerState::new(options.slot, now);
        state.air_finished = now + 12.0;
        let rules = options.view_rules.clone().unwrap_or_default();
        Q2CharacterActor {
            actor,
            entity,
            state,
            rules,
            options,
            pain_index: 0,
            death_index: 0,
        }
    }

    /// Capture the character.
    pub fn capture(&self) -> Q2CharacterCheckpoint {
        let mut state = self.state.clone();
        let chase = state.chase_target.clone();
        state.chase_target = None;
        let entity = &self.entity;
        Q2CharacterCheckpoint {
            version: 1,
            pain_index: self.pain_index,
            death_index: self.death_index,
            state: Q2PlayerStateCheckpoint {
                state,
                chase_target: save_q2_actor(chase.as_ref()),
            },
            rules: self.rules.clone(),
            entity: Q2CharacterEntityFields {
                model: entity.model.clone(),
                model2: entity.model2.clone(),
                model3: entity.model3.clone(),
                model4: entity.model4.clone(),
                skin: entity.skin,
                frame: entity.frame,
                old_frame: entity.old_frame,
                scale: entity.scale,
                effects: entity.effects,
                render_flags: entity.render_flags,
                flags: entity.flags,
                server_flags: entity.server_flags,
                view_height: entity.view_height,
                max_health: entity.max_health,
                sound: entity.sound.clone(),
                visible: entity.visible,
            },
            last_attack: self.entity.last_attack.as_ref().map(save_q2_attack),
        }
    }

    /// Restore the character.
    pub fn restore(
        &mut self,
        checkpoint: &Q2CharacterCheckpoint,
        resolve_actor: &mut dyn FnMut(SavedActorId) -> ActorId,
    ) {
        self.pain_index = checkpoint.pain_index;
        self.death_index = checkpoint.death_index;
        let mut state = checkpoint.state.state.clone();
        state.chase_target = checkpoint
            .state
            .chase_target
            .map(&mut *resolve_actor);
        self.state = state;
        self.rules = checkpoint.rules.clone();
        let entity = &mut self.entity;
        let saved = &checkpoint.entity;
        entity.model = saved.model.clone();
        entity.model2 = saved.model2.clone();
        entity.model3 = saved.model3.clone();
        entity.model4 = saved.model4.clone();
        entity.skin = saved.skin;
        entity.frame = saved.frame;
        entity.old_frame = saved.old_frame;
        entity.scale = saved.scale;
        entity.effects = saved.effects;
        entity.render_flags = saved.render_flags;
        entity.flags = saved.flags;
        entity.server_flags = saved.server_flags;
        entity.view_height = saved.view_height;
        entity.max_health = saved.max_health;
        entity.sound = saved.sound.clone();
        entity.visible = saved.visible;
        self.entity.last_attack = checkpoint
            .last_attack
            .as_ref()
            .map(|attack| restore_q2_attack(attack, resolve_actor));
    }

    /// Read the shared body.
    fn body(&mut self, host: &mut dyn Q2CharacterHost) -> BodyState {
        host.bodies()
            .read(self.actor.id())
            .expect("Q2 character actor has no shared body")
    }

    /// Player pain (no-op).
    pub fn pain(&mut self) {}

    /// Record damage feedback.
    pub fn record_damage(
        &mut self,
        host: &mut dyn Q2CharacterHost,
        decision: &DamageDecision,
    ) {
        self.entity.last_attack = Some(decision.request.attack.clone());
        if decision.reaction == DamageReactionKind::Death {
            return;
        }
        let feedback = match decision.feedback.as_ref() {
            Some(DamageFeedback::Q2 {
                power_armor,
                armor,
                blood,
                knockback,
            }) => Some((*power_armor, *armor, *blood, *knockback)),
            _ => None,
        };
        self.state.damage_blood += feedback.map_or(decision.applied_damage, |(_, _, blood, _)| blood);
        self.state.damage_armor += feedback.map_or(0.0, |(_, armor, _, _)| armor);
        self.state.damage_power_armor += feedback.map_or(0.0, |(power, _, _, _)| power);
        self.state.damage_knockback +=
            feedback.map_or(decision.request.knockback, |(_, _, _, knockback)| knockback);
        self.state.damage_from = decision.request.point;
        if feedback.map_or(0.0, |(power, _, _, _)| power) > 0.0 {
            self.state.power_armor_time = host.now() + 0.2;
        }
    }

    /// Run death.
    pub fn die(&mut self, host: &mut dyn Q2CharacterHost, reaction: &DeathReaction) {
        let body = self.body(host);
        let mut moved = body.clone();
        moved.angles = vec3(0.0, body.angles.y, 0.0);
        moved.bounds.max = vec3(body.bounds.max.x, body.bounds.max.y, -8.0);
        host.bodies().write(&self.actor, &moved);
        host.motion(&self.actor, Q2MotionKind::Toss, Q2Solid::Box);
        let first = !self.state.dead;
        if first {
            self.state.dead = true;
            self.state.respawn_time = host.now() + 1.0;
            let attacker = match reaction.pain.attacker.clone() {
                Some(attacker) if attacker != *self.actor.id() => Some(attacker),
                _ => None,
            };
            let killer = match attacker {
                None => match reaction.inflictor.clone() {
                    None => None,
                    Some(inflictor) if inflictor == *self.actor.id() => None,
                    Some(inflictor) => host.bodies().read(&inflictor),
                },
                Some(attacker) => host.bodies().read(&attacker),
            };
            self.state.killer_yaw = match killer {
                None => f64::from(body.angles.y),
                Some(killer) => {
                    (f64::from(killer.origin.y - body.origin.y)
                        .atan2(f64::from(killer.origin.x - body.origin.x))
                        * 180.0
                        / std::f64::consts::PI
                        + 360.0)
                        % 360.0
                }
            };
            host.died(&self.actor, reaction);
        }
        let health = host.combat().read(self.actor.id()).map_or(0.0, |combat| combat.health);
        if health < -40.0 && !self.state.gibbed {
            self.sound(host, "misc/udeath.wav", 4);
            let half = vec3(
                (body.bounds.max.x - body.bounds.min.x) * 0.5,
                (body.bounds.max.y - body.bounds.min.y) * 0.5,
                (body.bounds.max.z - body.bounds.min.z) * 0.5,
            );
            let center = add3(body.origin, add3(body.bounds.min, half));
            for _ in 0..4 {
                let origin = add3(
                    center,
                    vec3(
                        (host.random() * 2.0 - 1.0) as f32 * half.x,
                        (host.random() * 2.0 - 1.0) as f32 * half.y,
                        (host.random() * 2.0 - 1.0) as f32 * half.z,
                    ),
                );
                let impulse = self.gib_velocity(host, reaction.pain.damage);
                let velocity = add3(
                    body.velocity,
                    vec3(impulse.x * 0.5, impulse.y * 0.5, impulse.z * 0.5),
                );
                let spin = vec3(
                    (host.random() * 600.0) as f32,
                    (host.random() * 600.0) as f32,
                    (host.random() * 600.0) as f32,
                );
                let expires_at = host.now() + 10.0 + host.random() * 10.0;
                host.spawn_gib(Q2CharacterGib {
                    model: "models/objects/gibs/sm_meat/tris.md2".to_string(),
                    origin,
                    velocity: vec3(
                        velocity.x.max(-300.0).min(300.0),
                        velocity.y.max(-300.0).min(300.0),
                        velocity.z.max(200.0).min(500.0),
                    ),
                    angular_velocity: spin,
                    expires_at,
                });
            }
            let head = host.random() < 0.5;
            self.entity.model = if head {
                "models/objects/gibs/head2/tris.md2".to_string()
            } else {
                "models/objects/gibs/skull/tris.md2".to_string()
            };
            self.entity.skin = if head { 1 } else { 0 };
            self.entity.frame = 0;
            self.entity.effects = 2;
            let impulse = self.gib_velocity(host, reaction.pain.damage);
            let current = self.body(host);
            let mut moved = current.clone();
            moved.origin = add3(current.origin, vec3(0.0, 0.0, 32.0));
            moved.velocity = add3(current.velocity, impulse);
            moved.bounds.min = vec3(-16.0, -16.0, 0.0);
            moved.bounds.max = vec3(16.0, 16.0, 16.0);
            host.bodies().write(&self.actor, &moved);
            host.motion(&self.actor, Q2MotionKind::Bounce, Q2Solid::None);
            host.combat().set_traits(
                &self.actor,
                &CombatTraitChanges {
                    can_take_damage: Some(false),
                    mass: None,
                    invulnerable: None,
                    team: None,
                    no_knockback: None,
                },
            );
            self.state.gibbed = true;
        } else if first {
            self.death_index = (self.death_index + 1) % 3;
            self.state.animation_priority = 5;
            let ducked = host.movement(self.actor.id()).ducked;
            let frames = q2_death_animation_frames(ducked, self.death_index);
            self.entity.frame = frames.0;
            self.state.animation_end = frames.1;
            let variant = (host.random() * 4.0).floor() as i32 + 1;
            self.sound(host, &format!("*death{variant}.wav"), 2);
        }
        host.bodies().link(&self.actor, None);
        self.show(host);
    }

    /// Gib velocity.
    fn gib_velocity(&mut self, host: &mut dyn Q2CharacterHost, damage: f64) -> Vec3 {
        let factor = if damage < 50.0 { 0.7 } else { 1.2 };
        vec3(
            ((host.random() * 2.0 - 1.0) * 100.0 * factor) as f32,
            ((host.random() * 2.0 - 1.0) * 100.0 * factor) as f32,
            ((200.0 + host.random() * 100.0) * factor) as f32,
        )
    }

    /// Play a sound.
    fn sound(&mut self, host: &mut dyn Q2CharacterHost, path: &str, channel: i32) {
        let origin = self.body(host).origin;
        host.emit(Q2PresentationEvent::Sound(Q2SoundEvent {
            actor: Some(self.actor.id().clone()),
            origin,
            path: path.to_string(),
            channel,
            volume: 1.0,
            attenuation: 1.0,
            reliable: false,
            loop_: Q2SoundLoop::Once,
            loop_owner: None,
        }));
    }

    /// Show the model.
    fn show(&mut self, host: &mut dyn Q2CharacterHost) {
        let entity = &self.entity;
        host.emit(Q2PresentationEvent::Model(Q2ModelEvent {
            actor: self.actor.id().clone(),
            path: entity.model.clone(),
            attached_models: vec![
                entity.model2.clone(),
                entity.model3.clone(),
                entity.model4.clone(),
            ],
            frame: entity.frame,
            old_frame: entity.old_frame,
            scale: entity.scale,
            alpha: entity.alpha,
            skin: entity.skin,
            effects: entity.effects,
            render_flags: entity.render_flags,
        }));
    }

    /// Run after client think.
    pub fn after_client_think(&mut self, host: &mut dyn Q2CharacterHost) {
        let buttons = host.movement(self.actor.id()).buttons;
        self.state.latched_buttons |= buttons & !self.state.buttons;
        self.state.buttons = buttons;
    }

    /// Run begin frame.
    pub fn begin_frame(&mut self, host: &mut dyn Q2CharacterHost) {
        if self.state.dead
            && host.now() > self.state.respawn_time
            && (self.state.latched_buttons
                & (if self.options.mode == Q2Mode::Deathmatch {
                    1
                } else {
                    -1
                })
                != 0
                || self.options.mode == Q2Mode::Deathmatch
                    && self.options.deathmatch_flags & 1024 != 0)
        {
            self.state.latched_buttons = 0;
            host.request_respawn(&self.actor);
            return;
        }
        if !self.state.dead {
            self.state.latched_buttons = 0;
        }
    }

    /// Run after respawn.
    pub fn respawned(&mut self, host: &mut dyn Q2CharacterHost) {
        let now = host.now();
        let state = &mut self.state;
        state.dead = false;
        state.gibbed = false;
        state.respawn_time = now;
        state.air_finished = now + 12.0;
        state.drown_damage = 2.0;
        state.old_water_level = 0;
        state.old_velocity = vec3(0.0, 0.0, 0.0);
        state.damage_alpha = 0.0;
        state.bonus_alpha = 0.0;
        state.fall_time = 0.0;
        state.damage_time = 0.0;
        state.damage_blood = 0.0;
        state.damage_armor = 0.0;
        state.damage_power_armor = 0.0;
        state.damage_knockback = 0.0;
        state.animation_priority = 0;
        state.animation_end = 39;
        self.entity.frame = 0;
        self.entity.model = self.options.model.clone();
        self.entity.skin = self.options.skin;
        self.entity.effects = 0;
        self.entity.render_flags = if self.options.edition == Some(Q2Edition::Rerelease) {
            32768
        } else {
            0
        };
        self.entity.view_height = 22;
        self.state.event = "q2:player-teleport".to_string();
        self.show(host);
    }

    /// Set an animation.
    pub fn set_animation(
        &mut self,
        priority: Q2CharacterAnimation,
        first: i32,
        last: i32,
    ) {
        self.state.animation_priority = match priority {
            Q2CharacterAnimation::Attack => 4,
            Q2CharacterAnimation::Pain => 3,
            Q2CharacterAnimation::Reverse => 6,
        };
        self.entity.frame = first;
        self.state.animation_end = last;
    }

    /// Run end frame.
    pub fn end_frame(&mut self, host: &mut dyn Q2CharacterHost, intermission: bool) {
        let mut context = CharacterContext {
            character: &mut *self,
            host: &mut *host,
        };
        if intermission {
            let view = q2_build_view(&mut context, 0, true);
            let actor = context.actor_id();
            context.host.view(&actor, view);
            return;
        }
        let snapshot = context.state_snapshot();
        let movement = context.movement();
        context.with_entity(|entity| {
            entity.view_height = if snapshot.gibbed {
                8
            } else if snapshot.dead || movement.ducked {
                -2
            } else {
                22
            };
        });
        if context.character.options.environment {
            q2_world_effects(&mut context);
        }
        let body = context.body();
        let movement = context.movement();
        let vectors = angle_vectors(movement.view_angles);
        let side = dot3(body.velocity, vectors.right);
        let rules = context.rules();
        let roll = (if side < 0.0 { -1.0 } else { 1.0 })
            * (side.abs() as f64 * rules.roll_angle / rules.roll_speed).min(rules.roll_angle);
        let pitch = if movement.view_angles.x > 180.0 {
            movement.view_angles.x - 360.0
        } else {
            movement.view_angles.x
        };
        context.move_body(
            Q2BodyChanges {
                angles: Some(vec3(pitch / 3.0, movement.view_angles.y, roll as f32 * 4.0)),
                ..Q2BodyChanges::default()
            },
            true,
        );
        let speed = f64::from(body.velocity.x).hypot(f64::from(body.velocity.y));
        context.with_state(|state| {
            if speed < 5.0 {
                state.bob_move = 0.0;
                state.bob_time = 0.0;
            } else if movement.grounded {
                state.bob_move = if speed > 210.0 {
                    0.25
                } else if speed > 100.0 {
                    0.125
                } else {
                    0.0625
                };
            }
            state.bob_time += state.bob_move;
        });
        if context.character.options.environment {
            q2_falling_damage(&mut context);
        }
        let pain_index = context.character.pain_index;
        let feedback = q2_damage_feedback(&mut context, pain_index);
        context.character.pain_index = feedback.1;
        let view = q2_build_view(&mut context, feedback.0, false);
        let actor = context.actor_id();
        context.host.view(&actor, view.clone());
        let snapshot = context.state_snapshot();
        let movement = context.movement();
        let cycle = (if movement.ducked {
            snapshot.bob_time * 4.0
        } else {
            snapshot.bob_time
        })
        .trunc() as i32;
        if snapshot.event.is_empty()
            && movement.grounded
            && speed > 225.0
            && (snapshot.bob_time + snapshot.bob_move).trunc() as i32 != cycle
        {
            context.with_state(|state| {
                state.event = "q2:footstep".to_string();
            });
        }
        let event = context.state_snapshot().event;
        if !event.is_empty() {
            let number = if event == "q2:footstep" {
                Some(2)
            } else if event == "q2:fall-short" {
                Some(3)
            } else if event == "q2:fall" {
                Some(4)
            } else if event == "q2:fall-far" {
                Some(5)
            } else if event == "q2:player-teleport" {
                Some(6)
            } else {
                None
            };
            match number {
                Some(number) => context.emit(Q2PresentationEvent::EntityEvent {
                    actor: context.actor_id(),
                    event: number,
                }),
                None => {
                    let origin = context.body().origin;
                    emit_player_effect(&mut context, &event, origin);
                }
            }
            context.with_state(|state| {
                state.event.clear();
            });
        }
        q2_client_effects(&mut context);
        q2_client_animation(&mut context);
        let velocity = context.body().velocity;
        context.with_state(|state| {
            state.old_velocity = velocity;
            state.old_view_angles = view.angles;
        });
        self.show(host);
    }
}
