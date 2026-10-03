//! Q2 character actor (`src/content/q2/base/player/character.ts`).

use std::collections::BTreeMap;

use qa_core::identity::{ActorId, OwnedActor, SavedActorId};
use qa_core::math::{add3, dot3, vec3, Vec3};

use crate::contract::ItemId;
use crate::q2::foundation::checkpoint::{restore_q2_attack, save_q2_actor, save_q2_attack};
use crate::q2::foundation::host::{
    Q2Edition, Q2Entity, Q2Mode, Q2ModelEvent, Q2MotionKind, Q2PresentationEvent, Q2Solid, Q2SoundEvent, Q2SoundLoop,
    Q2SpawnFields,
};
use crate::q2::foundation::items::Q2PlayerPowerups;
use crate::q2::foundation::weapons::vectors::angle_vectors;
use crate::q2::support::contracts::{
    BodyState, CombatState, CombatTraitChanges, DamageDecision, DamageFeedback, DamageReactionKind, DeathReaction,
};
use crate::q2::support::tables::{Q2BodyTable, Q2CombatAuthority, Q2InventoryTable};

use super::checkpoint::{Q2CharacterCheckpoint, Q2CharacterEntityFields, Q2PlayerStateCheckpoint};
use super::environment::{q2_falling_damage, q2_world_effects};
use super::types::{
    Q2BodyChanges, Q2CharacterContext, Q2CharacterWeapon, Q2PlayerMovement, Q2PlayerRules, Q2PlayerState, Q2PlayerView,
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

/// Character host clock (`now`).
pub type Q2CharacterClock = Box<dyn Fn() -> f64>;
/// Character host random draw (`random`).
pub type Q2CharacterRandom = Box<dyn FnMut() -> f64>;
/// Character host movement observation (`movement`).
pub type Q2CharacterMovementFn = Box<dyn FnMut(&ActorId) -> Q2PlayerMovement>;
/// Character host point-contents query (`point_contents`).
pub type Q2CharacterPointContents = Box<dyn FnMut(Vec3) -> i32>;
/// Character host powerup observation (`powerups`).
pub type Q2CharacterPowerupsFn = Box<dyn FnMut(&ActorId) -> Q2PlayerPowerups>;
/// Character host weapon observation (`weapon`).
pub type Q2CharacterWeaponFn = Box<dyn FnMut(&ActorId) -> Option<Q2CharacterWeapon>>;
/// Character host presentation sink (`emit`).
pub type Q2CharacterEmit = Box<dyn FnMut(Q2PresentationEvent)>;
/// Character host view sink (`view`).
pub type Q2CharacterViewFn = Box<dyn FnMut(&ActorId, Q2PlayerView)>;
/// Character host noise sink (`noise`).
pub type Q2CharacterNoise = Box<dyn FnMut(&ActorId, Vec3)>;
/// Character host environment damage (`environment_damage`).
pub type Q2CharacterEnvironmentDamage = Box<dyn FnMut(&OwnedActor, f64, i32, i32)>;
/// Character host death reaction (`died`).
pub type Q2CharacterDied = Box<dyn FnMut(&OwnedActor, &DeathReaction)>;
/// Character host respawn request (`request_respawn`).
pub type Q2CharacterRespawn = Box<dyn FnMut(&OwnedActor)>;
/// Character host motion update (`motion`).
pub type Q2CharacterMotion = Box<dyn FnMut(&OwnedActor, Q2MotionKind, Q2Solid)>;
/// Character host gib spawn (`spawn_gib`).
pub type Q2CharacterSpawnGib = Box<dyn FnMut(Q2CharacterGib)>;

/// Production character host over engine services (`Q2CharacterHost`).
///
/// Each field is one donor host member: shared tables cross as trait
/// objects, scalar and observation services cross as hooks so the engine
/// binds them over its own tables, clock, scene, and match without a
/// content dependency on the simulation.
///
/// Missing siblings: the runtime character lane owns construction; the
/// pending caller chain is the `combat_before_reaction` character arm,
/// the QuakeC client-spawn respawn arm, and the
/// `q2_character_after_client_think` host upgrade. Uncalled until it lands.
pub struct Q2CharacterHostServices {
    /// Shared body table.
    pub bodies: Box<dyn Q2BodyTable>,
    /// Gameplay combat authority.
    pub combat: Box<dyn Q2CombatAuthority>,
    /// Shared inventory table.
    pub inventory: Box<dyn Q2InventoryTable>,
    /// Current source time in seconds.
    pub clock: Q2CharacterClock,
    /// Unit random draw.
    pub random: Q2CharacterRandom,
    /// Movement observation.
    pub movement: Q2CharacterMovementFn,
    /// Point-contents query.
    pub point_contents: Q2CharacterPointContents,
    /// Powerup observation.
    pub powerups: Q2CharacterPowerupsFn,
    /// Weapon observation.
    pub weapon: Q2CharacterWeaponFn,
    /// Presentation sink.
    pub emit: Q2CharacterEmit,
    /// View sink.
    pub view: Q2CharacterViewFn,
    /// Noise sink.
    pub noise: Q2CharacterNoise,
    /// Environment damage.
    pub environment_damage: Q2CharacterEnvironmentDamage,
    /// Death reaction.
    pub died: Q2CharacterDied,
    /// Respawn request.
    pub request_respawn: Q2CharacterRespawn,
    /// Motion update.
    pub motion: Q2CharacterMotion,
    /// Gib spawn.
    pub spawn_gib: Q2CharacterSpawnGib,
}

impl Q2CharacterHost for Q2CharacterHostServices {
    fn bodies(&mut self) -> &mut dyn Q2BodyTable {
        &mut *self.bodies
    }

    fn combat(&mut self) -> &mut dyn Q2CombatAuthority {
        &mut *self.combat
    }

    fn inventory(&mut self) -> &mut dyn Q2InventoryTable {
        &mut *self.inventory
    }

    fn now(&self) -> f64 {
        (self.clock)()
    }

    fn random(&mut self) -> f64 {
        (self.random)()
    }

    fn movement(&mut self, actor: &ActorId) -> Q2PlayerMovement {
        (self.movement)(actor)
    }

    fn point_contents(&mut self, point: Vec3) -> i32 {
        (self.point_contents)(point)
    }

    fn powerups(&mut self, actor: &ActorId) -> Q2PlayerPowerups {
        (self.powerups)(actor)
    }

    fn weapon(&mut self, actor: &ActorId) -> Option<Q2CharacterWeapon> {
        (self.weapon)(actor)
    }

    fn emit(&mut self, event: Q2PresentationEvent) {
        (self.emit)(event);
    }

    fn view(&mut self, actor: &ActorId, view: Q2PlayerView) {
        (self.view)(actor, view);
    }

    fn noise(&mut self, actor: &ActorId, origin: Vec3) {
        (self.noise)(actor, origin);
    }

    fn environment_damage(&mut self, actor: &OwnedActor, amount: f64, means: i32, flags: i32) {
        (self.environment_damage)(actor, amount, means, flags);
    }

    fn died(&mut self, actor: &OwnedActor, reaction: &DeathReaction) {
        (self.died)(actor, reaction);
    }

    fn request_respawn(&mut self, actor: &OwnedActor) {
        (self.request_respawn)(actor);
    }

    fn motion(&mut self, actor: &OwnedActor, kind: Q2MotionKind, solid: Q2Solid) {
        (self.motion)(actor, kind, solid);
    }

    fn spawn_gib(&mut self, gib: Q2CharacterGib) {
        (self.spawn_gib)(gib);
    }
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
        state.chase_target = checkpoint.state.chase_target.map(&mut *resolve_actor);
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
    pub fn record_damage(&mut self, host: &mut dyn Q2CharacterHost, decision: &DamageDecision) {
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
        self.state.damage_knockback += feedback.map_or(decision.request.knockback, |(_, _, _, knockback)| knockback);
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
                    (f64::from(killer.origin.y - body.origin.y).atan2(f64::from(killer.origin.x - body.origin.x))
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
                let velocity = add3(body.velocity, vec3(impulse.x * 0.5, impulse.y * 0.5, impulse.z * 0.5));
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
                        velocity.x.clamp(-300.0, 300.0),
                        velocity.y.clamp(-300.0, 300.0),
                        velocity.z.clamp(200.0, 500.0),
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
            attached_models: vec![entity.model2.clone(), entity.model3.clone(), entity.model4.clone()],
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
            && (self.state.latched_buttons & (if self.options.mode == Q2Mode::Deathmatch { 1 } else { -1 }) != 0
                || self.options.mode == Q2Mode::Deathmatch && self.options.deathmatch_flags & 1024 != 0)
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
    pub fn set_animation(&mut self, priority: Q2CharacterAnimation, first: i32, last: i32) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::{HashMap, HashSet};
    use std::rc::Rc;

    use qa_core::identity::{IdentityOwner, ProviderId};
    use qa_core::math::{Bounds, Vec4};
    use qa_core::time::SourceTime;

    use crate::contract::{ArmorState, InventoryEntry, PoweredProtectionState, RegularArmorState};
    use crate::q2::support::contracts::{
        AttackCause, BodyAttachment, DamageDelivery, DamageOutcome, DamageRequest, LinkedBody, PainReaction,
        PowerArmorCells,
    };

    #[derive(Default)]
    struct FakeBodies {
        records: HashMap<ActorId, BodyState>,
        attachments: HashMap<ActorId, BodyAttachment>,
        linked: HashSet<ActorId>,
    }

    impl Q2BodyTable for FakeBodies {
        fn create(&mut self, actor: &OwnedActor, initial: &BodyState) {
            self.records.insert(actor.id().clone(), initial.clone());
        }

        fn read(&self, actor: &ActorId) -> Option<BodyState> {
            self.records.get(actor).cloned()
        }

        fn write(&mut self, actor: &OwnedActor, state: &BodyState) {
            self.records.insert(actor.id().clone(), state.clone());
        }

        fn attach(&mut self, actor: &OwnedActor, attachment: &BodyAttachment) {
            self.attachments.insert(actor.id().clone(), attachment.clone());
        }

        fn detach(&mut self, actor: &OwnedActor) {
            self.attachments.remove(actor.id());
        }

        fn attachment(&self, actor: &ActorId) -> Option<BodyAttachment> {
            self.attachments.get(actor).cloned()
        }

        fn linked(&self, actor: &ActorId) -> Option<LinkedBody> {
            let state = self.records.get(actor)?.clone();
            if !self.linked.contains(actor) {
                return None;
            }
            Some(LinkedBody {
                actor: actor.clone(),
                absolute_bounds: state.bounds,
                state,
                link_count: 1,
            })
        }

        fn link(&mut self, actor: &OwnedActor, _origin: Option<Vec3>) {
            self.linked.insert(actor.id().clone());
        }

        fn unlink(&mut self, actor: &OwnedActor) {
            self.linked.remove(actor.id());
        }
    }

    #[derive(Default)]
    struct FakeCombat {
        records: HashMap<ActorId, CombatState>,
    }

    impl FakeCombat {
        fn seed(&mut self, actor: &ActorId, health: f64) {
            self.records.insert(
                actor.clone(),
                CombatState {
                    health,
                    armor: ArmorState {
                        regular: RegularArmorState::None,
                        powered: PoweredProtectionState::None,
                    },
                    mass: 200.0,
                    can_take_damage: true,
                    invulnerable: false,
                    no_knockback: false,
                    team: None,
                },
            );
        }
    }

    impl Q2CombatAuthority for FakeCombat {
        fn create(&mut self, actor: &OwnedActor, initial: &CombatState) {
            self.records.insert(actor.id().clone(), initial.clone());
        }

        fn read(&self, actor: &ActorId) -> Option<CombatState> {
            self.records.get(actor).cloned()
        }

        fn set_health(&mut self, actor: &OwnedActor, health: f64) {
            if let Some(record) = self.records.get_mut(actor.id()) {
                record.health = health;
            }
        }

        fn set_armor(&mut self, actor: &OwnedActor, armor: &ArmorState) {
            if let Some(record) = self.records.get_mut(actor.id()) {
                record.armor = armor.clone();
            }
        }

        fn set_regular_points(&mut self, _actor: &OwnedActor, _points: f64, _initial: Option<&RegularArmorState>) {}

        fn set_regular_armor(&mut self, actor: &OwnedActor, regular: &RegularArmorState) {
            if let Some(record) = self.records.get_mut(actor.id()) {
                record.armor.regular = regular.clone();
            }
        }

        fn set_powered_protection(&mut self, actor: &OwnedActor, powered: &PoweredProtectionState) {
            if let Some(record) = self.records.get_mut(actor.id()) {
                record.armor.powered = powered.clone();
            }
        }

        fn set_traits(&mut self, actor: &OwnedActor, changes: &CombatTraitChanges) {
            if let Some(record) = self.records.get_mut(actor.id()) {
                if let Some(can_take_damage) = changes.can_take_damage {
                    record.can_take_damage = can_take_damage;
                }
                if let Some(mass) = changes.mass {
                    record.mass = mass;
                }
                if let Some(invulnerable) = changes.invulnerable {
                    record.invulnerable = invulnerable;
                }
                if let Some(team) = changes.team.clone() {
                    record.team = team;
                }
                if let Some(no_knockback) = changes.no_knockback {
                    record.no_knockback = no_knockback;
                }
            }
        }

        fn bind_power_armor_cells(&mut self, _actor: &OwnedActor, _cells: Box<dyn PowerArmorCells>) {}

        fn apply(&mut self, input: &DamageRequest) -> DamageOutcome {
            DamageOutcome::StaleTarget { request: input.clone() }
        }
    }

    #[derive(Default)]
    struct FakeInventory {
        counts: HashMap<(ActorId, ItemId), f64>,
    }

    impl Q2InventoryTable for FakeInventory {
        fn create(&mut self, _actor: &OwnedActor, _entries: &[InventoryEntry]) {}

        fn entries(&self, _actor: &ActorId) -> Vec<InventoryEntry> {
            Vec::new()
        }

        fn has(&self, actor: &ActorId) -> bool {
            self.counts.keys().any(|(id, _)| id == actor)
        }

        fn count(&self, actor: &ActorId, item: &ItemId) -> f64 {
            self.counts.get(&(actor.clone(), item.clone())).copied().unwrap_or(0.0)
        }

        fn consume(&mut self, actor: &OwnedActor, item: &ItemId, count: f64) -> bool {
            let entry = self.counts.entry((actor.id().clone(), item.clone())).or_insert(0.0);
            if *entry < count {
                return false;
            }
            *entry -= count;
            true
        }

        fn give(&mut self, actor: &OwnedActor, item: &ItemId, count: f64) -> f64 {
            let entry = self.counts.entry((actor.id().clone(), item.clone())).or_insert(0.0);
            *entry += count;
            count
        }

        fn configure(&mut self, _actor: &OwnedActor, _entry: &InventoryEntry) {}

        fn adjust_source_counter(&mut self, actor: &OwnedActor, item: &ItemId, delta: f64) -> f64 {
            let entry = self.counts.entry((actor.id().clone(), item.clone())).or_insert(0.0);
            *entry += delta;
            *entry
        }
    }

    #[derive(Default)]
    struct HostRecorder {
        events: Vec<Q2PresentationEvent>,
        views: Vec<(ActorId, Q2PlayerView)>,
        noises: Vec<(ActorId, Vec3)>,
        damage: Vec<(OwnedActor, f64, i32, i32)>,
        deaths: Vec<OwnedActor>,
        respawns: Vec<OwnedActor>,
        motions: Vec<(OwnedActor, Q2MotionKind, Q2Solid)>,
        gibs: Vec<Q2CharacterGib>,
    }

    fn test_owned(slot: u32) -> OwnedActor {
        let owner = IdentityOwner::create("q2-character-test").expect("owner");
        let id = owner.actor(slot, 1);
        owner.owned_actor(&id, ProviderId::new("q2", "test")).expect("owned")
    }

    fn test_movement(buttons: i32) -> Q2PlayerMovement {
        Q2PlayerMovement {
            view_angles: vec3(0.0, 90.0, 0.0),
            command_angles: vec3(0.0, 90.0, 0.0),
            water_level: 0,
            water_type: 0,
            grounded: true,
            ducked: false,
            buttons,
            standing_bounds: Bounds {
                min: vec3(-16.0, -16.0, -24.0),
                max: vec3(16.0, 16.0, 32.0),
            },
            animate_q2: true,
        }
    }

    fn test_view() -> Q2PlayerView {
        Q2PlayerView {
            angles: vec3(1.0, 2.0, 3.0),
            offset: vec3(4.0, 5.0, 6.0),
            kick_angles: vec3(0.0, 0.0, 0.0),
            gun_angles: vec3(0.0, 0.0, 0.0),
            gun_offset: vec3(0.0, 0.0, 0.0),
            blend: Vec4 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
                w: 0.0,
            },
            fov: 90,
            underwater: false,
            flashes: 0,
            health: 100.0,
            armor: 0.0,
            ammo: 0.0,
            score: 0,
            selected_item: None,
            timer: None,
            spectator: false,
            layouts: 0,
        }
    }

    fn test_host(recorder: Rc<RefCell<HostRecorder>>, buttons: Rc<RefCell<i32>>, now: f64) -> Q2CharacterHostServices {
        let emit = Rc::clone(&recorder);
        let view_sink = Rc::clone(&recorder);
        let noise = Rc::clone(&recorder);
        let damage = Rc::clone(&recorder);
        let died = Rc::clone(&recorder);
        let respawn = Rc::clone(&recorder);
        let motion = Rc::clone(&recorder);
        let gib = Rc::clone(&recorder);
        Q2CharacterHostServices {
            bodies: Box::new(FakeBodies::default()),
            combat: Box::new(FakeCombat::default()),
            inventory: Box::new(FakeInventory::default()),
            clock: Box::new(move || now),
            random: Box::new(|| 0.5),
            movement: Box::new(move |_| test_movement(*buttons.borrow())),
            point_contents: Box::new(|_| 0),
            powerups: Box::new(|_| Q2PlayerPowerups::default()),
            weapon: Box::new(|_| None),
            emit: Box::new(move |event| emit.borrow_mut().events.push(event)),
            view: Box::new(move |actor, view| view_sink.borrow_mut().views.push((actor.clone(), view))),
            noise: Box::new(move |actor, origin| noise.borrow_mut().noises.push((actor.clone(), origin))),
            environment_damage: Box::new(move |actor, amount, means, flags| {
                damage.borrow_mut().damage.push((actor.clone(), amount, means, flags));
            }),
            died: Box::new(move |actor, _| died.borrow_mut().deaths.push(actor.clone())),
            request_respawn: Box::new(move |actor| respawn.borrow_mut().respawns.push(actor.clone())),
            motion: Box::new(move |actor, kind, solid| motion.borrow_mut().motions.push((actor.clone(), kind, solid))),
            spawn_gib: Box::new(move |gibbed| gib.borrow_mut().gibs.push(gibbed)),
        }
    }

    fn test_character(actor: OwnedActor) -> Q2CharacterActor {
        Q2CharacterActor::new(
            actor,
            Q2CharacterOptions {
                edition: Some(Q2Edition::Classic),
                model: "models/player.md2".to_string(),
                skin: 0,
                slot: 0,
                mode: Q2Mode::Deathmatch,
                deathmatch_flags: 0,
                environment: false,
                view_rules: None,
            },
            100.0,
        )
    }

    fn seed_body(host: &mut Q2CharacterHostServices, actor: &OwnedActor) {
        host.bodies().create(
            actor,
            &BodyState {
                origin: vec3(0.0, 0.0, 0.0),
                angles: vec3(0.0, 90.0, 0.0),
                velocity: vec3(0.0, 0.0, 0.0),
                bounds: Bounds {
                    min: vec3(-16.0, -16.0, -24.0),
                    max: vec3(16.0, 16.0, 32.0),
                },
                ground: None,
            },
        );
    }

    fn seed_combat(host: &mut Q2CharacterHostServices, actor: &ActorId, health: f64) {
        let mut combat = FakeCombat::default();
        combat.seed(actor, health);
        host.combat = Box::new(combat);
    }

    fn test_attack(target: &ActorId) -> DamageRequest {
        let provider = ProviderId::new("q2", "test");
        DamageRequest {
            attack: crate::q2::support::contracts::AttackProvenance {
                sequence: 1,
                time: SourceTime::Seconds(100.0),
                attacker: None,
                inflictor: None,
                originating_projectile: None,
                weapon: None,
                weapon_provider: provider.clone(),
                damage_powerup_owner: None,
                combat_provider: provider.clone(),
                inventory_provider: provider.clone(),
                movement_provider: provider,
                cause: AttackCause::Q2 {
                    means_of_death: 1,
                    damage_flags: 0,
                    native: None,
                },
            },
            target: target.clone(),
            amount: 25.0,
            knockback: 100.0,
            direction: vec3(0.0, 0.0, 1.0),
            point: vec3(1.0, 2.0, 3.0),
            normal: vec3(0.0, 0.0, 1.0),
            delivery: DamageDelivery::Direct,
        }
    }

    #[test]
    fn host_delegates_every_service() {
        let actor = test_owned(1);
        let id = actor.id().clone();
        let recorder = Rc::new(RefCell::new(HostRecorder::default()));
        let buttons = Rc::new(RefCell::new(7));
        let mut host = test_host(Rc::clone(&recorder), Rc::clone(&buttons), 100.0);

        seed_body(&mut host, &actor);
        assert_eq!(host.bodies().read(&id).expect("body").angles.y, 90.0);
        host.combat().create(
            &actor,
            &CombatState {
                health: 80.0,
                armor: ArmorState {
                    regular: RegularArmorState::None,
                    powered: PoweredProtectionState::None,
                },
                mass: 200.0,
                can_take_damage: true,
                invulnerable: false,
                no_knockback: false,
                team: None,
            },
        );
        assert_eq!(host.combat().read(&id).expect("combat").health, 80.0);
        assert_eq!(host.inventory().give(&actor, &"shells".to_string(), 10.0), 10.0);
        assert_eq!(host.inventory().count(&id, &"shells".to_string()), 10.0);
        assert_eq!(host.now(), 100.0);
        assert_eq!(host.random(), 0.5);
        assert_eq!(host.movement(&id).buttons, 7);
        assert_eq!(host.point_contents(vec3(0.0, 0.0, 0.0)), 0);
        assert_eq!(host.powerups(&id), Q2PlayerPowerups::default());
        assert_eq!(host.weapon(&id), None);
        host.emit(Q2PresentationEvent::EntityEvent {
            actor: id.clone(),
            event: 7,
        });
        let view = test_view();
        host.view(&id, view.clone());
        host.noise(&id, vec3(1.0, 2.0, 3.0));
        host.environment_damage(&actor, 5.0, 3, 1);
        let reaction = DeathReaction {
            pain: PainReaction {
                attack: None,
                this: actor.clone(),
                attacker: None,
                kick: 0.0,
                damage: 10.0,
            },
            inflictor: None,
            point: vec3(0.0, 0.0, 0.0),
        };
        host.died(&actor, &reaction);
        host.request_respawn(&actor);
        host.motion(&actor, Q2MotionKind::Toss, Q2Solid::Box);
        host.spawn_gib(Q2CharacterGib {
            model: "gib.md2".to_string(),
            origin: vec3(0.0, 0.0, 0.0),
            velocity: vec3(0.0, 0.0, 0.0),
            angular_velocity: vec3(0.0, 0.0, 0.0),
            expires_at: 110.0,
        });

        let recorder = recorder.borrow();
        assert!(matches!(
            recorder.events.as_slice(),
            [Q2PresentationEvent::EntityEvent { event: 7, .. }]
        ));
        assert_eq!(recorder.views.as_slice(), [(id.clone(), view)]);
        assert_eq!(recorder.noises.as_slice(), [(id, vec3(1.0, 2.0, 3.0))]);
        assert_eq!(recorder.damage.len(), 1);
        assert_eq!(recorder.damage[0].1, 5.0);
        assert_eq!(recorder.deaths.len(), 1);
        assert_eq!(recorder.deaths[0], actor);
        assert_eq!(recorder.respawns.len(), 1);
        assert_eq!(recorder.respawns[0], actor);
        assert_eq!(
            recorder.motions.as_slice(),
            [(actor.clone(), Q2MotionKind::Toss, Q2Solid::Box)]
        );
        assert_eq!(recorder.gibs.len(), 1);
        assert_eq!(recorder.gibs[0].model, "gib.md2");
    }

    #[test]
    fn character_latch_runs_through_host() {
        let actor = test_owned(2);
        let mut character = test_character(actor);
        let recorder = Rc::new(RefCell::new(HostRecorder::default()));
        let buttons = Rc::new(RefCell::new(5));
        let mut host = test_host(recorder, Rc::clone(&buttons), 100.0);

        character.after_client_think(&mut host);
        assert_eq!(character.state.buttons, 5);
        assert_eq!(character.state.latched_buttons, 5);
        *buttons.borrow_mut() = 4;
        character.after_client_think(&mut host);
        assert_eq!(character.state.buttons, 4);
        assert_eq!(character.state.latched_buttons, 5);
    }

    #[test]
    fn character_begin_frame_respawn_gate() {
        let actor = test_owned(3);
        let mut character = test_character(actor.clone());
        let recorder = Rc::new(RefCell::new(HostRecorder::default()));
        let buttons = Rc::new(RefCell::new(0));
        let mut host = test_host(Rc::clone(&recorder), buttons, 200.0);

        character.state.dead = true;
        character.state.latched_buttons = 1;
        character.begin_frame(&mut host);
        assert_eq!(character.state.latched_buttons, 0);
        assert_eq!(recorder.borrow().respawns.len(), 1);
        assert_eq!(recorder.borrow().respawns[0], actor);

        character.state.dead = false;
        character.state.latched_buttons = 7;
        character.begin_frame(&mut host);
        assert_eq!(character.state.latched_buttons, 0);
        assert_eq!(recorder.borrow().respawns.len(), 1);
    }

    #[test]
    fn character_records_damage_through_host() {
        let actor = test_owned(4);
        let id = actor.id().clone();
        let mut character = test_character(actor);
        let recorder = Rc::new(RefCell::new(HostRecorder::default()));
        let buttons = Rc::new(RefCell::new(0));
        let mut host = test_host(recorder, buttons, 100.0);

        let request = test_attack(&id);
        character.record_damage(
            &mut host,
            &DamageDecision {
                request: request.clone(),
                mutations: Vec::new(),
                applied_damage: 25.0,
                reaction: DamageReactionKind::Pain,
                feedback: None,
            },
        );
        assert_eq!(character.state.damage_blood, 25.0);
        assert_eq!(character.state.damage_from, vec3(1.0, 2.0, 3.0));
        assert_eq!(character.entity.last_attack.as_ref().expect("attack").sequence, 1);
        assert_eq!(character.state.power_armor_time, 0.0);

        character.record_damage(
            &mut host,
            &DamageDecision {
                request,
                mutations: Vec::new(),
                applied_damage: 25.0,
                reaction: DamageReactionKind::Pain,
                feedback: Some(DamageFeedback::Q2 {
                    power_armor: 5.0,
                    armor: 10.0,
                    blood: 20.0,
                    knockback: 50.0,
                }),
            },
        );
        assert_eq!(character.state.damage_blood, 45.0);
        assert_eq!(character.state.damage_armor, 10.0);
        assert_eq!(character.state.power_armor_time, 100.2);
    }

    #[test]
    fn character_gib_flow_spawns_gibs() {
        let actor = test_owned(5);
        let id = actor.id().clone();
        let mut character = test_character(actor.clone());
        let recorder = Rc::new(RefCell::new(HostRecorder::default()));
        let buttons = Rc::new(RefCell::new(0));
        let mut host = test_host(Rc::clone(&recorder), buttons, 100.0);
        seed_body(&mut host, &actor);
        seed_combat(&mut host, &id, -50.0);

        character.die(
            &mut host,
            &DeathReaction {
                pain: PainReaction {
                    attack: None,
                    this: actor.clone(),
                    attacker: None,
                    kick: 0.0,
                    damage: 120.0,
                },
                inflictor: None,
                point: vec3(0.0, 0.0, 0.0),
            },
        );

        assert!(character.state.gibbed);
        assert!(character.entity.model.contains("gibs"));
        assert_eq!(character.state.killer_yaw, 90.0);
        assert!(!host.combat().read(&id).expect("combat").can_take_damage);
        let recorder = recorder.borrow();
        assert_eq!(recorder.gibs.len(), 4);
        assert_eq!(recorder.deaths.len(), 1);
        assert_eq!(recorder.deaths[0], actor);
        assert!(recorder
            .motions
            .contains(&(actor.clone(), Q2MotionKind::Toss, Q2Solid::Box)));
        assert!(recorder.motions.contains(&(actor, Q2MotionKind::Bounce, Q2Solid::None)));
        assert!(!recorder.events.is_empty());
    }

    #[test]
    fn character_end_frame_intermission_publishes_view() {
        let actor = test_owned(6);
        let id = actor.id().clone();
        let mut character = test_character(actor);
        let recorder = Rc::new(RefCell::new(HostRecorder::default()));
        let buttons = Rc::new(RefCell::new(0));
        let mut host = test_host(Rc::clone(&recorder), buttons, 100.0);
        seed_body(&mut host, &character.actor.clone());

        character.end_frame(&mut host, true);

        let recorder = recorder.borrow();
        assert_eq!(recorder.views.len(), 1);
        assert_eq!(recorder.views[0].0, id);
    }

    #[test]
    fn character_respawned_resets() {
        let actor = test_owned(7);
        let mut character = test_character(actor);
        let recorder = Rc::new(RefCell::new(HostRecorder::default()));
        let buttons = Rc::new(RefCell::new(0));
        let mut host = test_host(Rc::clone(&recorder), buttons, 100.0);

        character.state.dead = true;
        character.state.gibbed = true;
        character.state.damage_blood = 99.0;
        character.respawned(&mut host);

        assert!(!character.state.dead);
        assert!(!character.state.gibbed);
        assert_eq!(character.state.damage_blood, 0.0);
        assert_eq!(character.state.air_finished, 112.0);
        assert_eq!(character.entity.model, "models/player.md2");
        assert_eq!(character.state.event, "q2:player-teleport");
        assert!(!recorder.borrow().events.is_empty());
    }
}
