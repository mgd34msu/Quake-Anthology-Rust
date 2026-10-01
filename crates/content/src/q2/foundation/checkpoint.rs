//! Foundation checkpoint data (`src/content/q2/foundation/checkpoint.ts`).
//!
//! Data structs plus capture/restore helpers. Byte framing lives with
//! the `qa-app` persistence codecs, which convert these records; this
//! module never touches save bytes.

use qa_core::identity::{ActorId, OwnedActor, SavedActorId};
use qa_core::math::Vec3;

use super::host::{Q2Counters, Q2Entity, Q2GameServices, Q2MotionKind, Q2Solid};
use crate::q2::support::contracts::AttackProvenance;

/// Save an actor reference (`saveQ2Actor`).
pub fn save_q2_actor(actor: Option<&ActorId>) -> Option<SavedActorId> {
    actor.map(SavedActorId::from)
}

/// Restore an actor reference through the checkpoint domain
/// (`restoreQ2Actor`).
pub fn restore_q2_actor(game: &mut Q2GameServices, actor: SavedActorId) -> OwnedActor {
    game.host.actors().resolve_saved(actor).unwrap_or_else(|| {
        panic!(
            "Q2 checkpoint references missing actor {}:{}",
            actor.slot, actor.generation
        )
    })
}

/// Attack provenance checkpoint (`Q2AttackCheckpoint`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2AttackCheckpoint {
    /// Saved attacker.
    pub attacker: Option<SavedActorId>,
    /// Saved inflictor.
    pub inflictor: Option<SavedActorId>,
    /// Saved originating projectile.
    pub originating_projectile: Option<SavedActorId>,
    /// Remaining provenance fields.
    pub attack: AttackProvenance,
}

/// Save attack provenance (`saveQ2Attack`).
pub fn save_q2_attack(attack: &AttackProvenance) -> Q2AttackCheckpoint {
    Q2AttackCheckpoint {
        attacker: save_q2_actor(attack.attacker.as_ref()),
        inflictor: save_q2_actor(attack.inflictor.as_ref()),
        originating_projectile: save_q2_actor(attack.originating_projectile.as_ref()),
        attack: AttackProvenance {
            attacker: None,
            inflictor: None,
            originating_projectile: None,
            ..attack.clone()
        },
    }
}

/// Restore attack provenance (`restoreQ2Attack`).
pub fn restore_q2_attack(
    checkpoint: &Q2AttackCheckpoint,
    reference: &mut dyn FnMut(SavedActorId) -> ActorId,
) -> AttackProvenance {
    AttackProvenance {
        attacker: checkpoint.attacker.map(&mut *reference),
        inflictor: checkpoint.inflictor.map(&mut *reference),
        originating_projectile: checkpoint.originating_projectile.map(&mut *reference),
        ..checkpoint.attack.clone()
    }
}

/// Plain entity value fields (`Q2EntityCheckpoint["values"]`).
///
/// Everything except the actor, spawn record, links, last attack and
/// callback slots.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2EntityValues {
    /// Classname.
    pub classname: String,
    /// Trigger target.
    pub target: String,
    /// Target name.
    pub targetname: String,
    /// Kill target.
    pub killtarget: String,
    /// Combat target.
    pub combat_target: String,
    /// Death target.
    pub death_target: String,
    /// Health target.
    pub health_target: String,
    /// Item target.
    pub item_target: String,
    /// Message.
    pub message: String,
    /// Model path.
    pub model: String,
    /// Attached models.
    pub model2: String,
    /// Attached models.
    pub model3: String,
    /// Attached models.
    pub model4: String,
    /// Spawn flags.
    pub spawnflags: i32,
    /// Delay seconds.
    pub delay: f64,
    /// Wait seconds.
    pub wait: f64,
    /// Speed.
    pub speed: f64,
    /// Acceleration.
    pub accel: f64,
    /// Deceleration.
    pub decel: f64,
    /// Damage.
    pub damage: f64,
    /// Damage radius.
    pub damage_radius: f64,
    /// Radius damage.
    pub radius_damage: f64,
    /// Count.
    pub count: i32,
    /// Maximum health.
    pub max_health: f64,
    /// View height.
    pub view_height: i32,
    /// Frame.
    pub frame: i32,
    /// Old frame.
    pub old_frame: i32,
    /// Scale.
    pub scale: f64,
    /// Alpha.
    pub alpha: f64,
    /// Skin.
    pub skin: i32,
    /// Effects.
    pub effects: i64,
    /// Render flags.
    pub render_flags: i32,
    /// Flags.
    pub flags: i64,
    /// Server flags.
    pub server_flags: i32,
    /// Light level.
    pub light_level: i32,
    /// Power cubes.
    pub power_cubes: i32,
    /// Timestamp.
    pub timestamp: f64,
    /// Noise path.
    pub noise: String,
    /// Sound path.
    pub sound: String,
    /// Volume.
    pub volume: f64,
    /// Attenuation.
    pub attenuation: f64,
    /// Random variance.
    pub random: f64,
    /// Map name.
    pub map: String,
    /// Style.
    pub style: i32,
    /// Whether a transition started.
    pub transition_started: bool,
    /// Clip mask.
    pub clip_mask: i32,
    /// Whether a projectile.
    pub projectile: bool,
    /// Whether dodgeable.
    pub dodgeable: bool,
    /// Laser immunity.
    pub laser_immune: bool,
    /// Damageable target.
    pub damageable_target: bool,
    /// Whether visible.
    pub visible: bool,
    /// Solidity.
    pub solid: Q2Solid,
    /// Motion kind.
    pub motion: Q2MotionKind,
    /// Gravity scale.
    pub gravity: f64,
    /// Gravity vector.
    pub gravity_vector: Vec3,
    /// Angular velocity.
    pub angular_velocity: Vec3,
    /// Move direction.
    pub movedir: Vec3,
    /// Position 1.
    pub pos1: Vec3,
    /// Position 2.
    pub pos2: Vec3,
    /// Next think time.
    pub next_think: Option<f64>,
}

impl Q2EntityValues {
    /// Capture value fields from an entity.
    pub fn capture(entity: &Q2Entity) -> Self {
        Self {
            classname: entity.classname.clone(),
            target: entity.target.clone(),
            targetname: entity.targetname.clone(),
            killtarget: entity.killtarget.clone(),
            combat_target: entity.combat_target.clone(),
            death_target: entity.death_target.clone(),
            health_target: entity.health_target.clone(),
            item_target: entity.item_target.clone(),
            message: entity.message.clone(),
            model: entity.model.clone(),
            model2: entity.model2.clone(),
            model3: entity.model3.clone(),
            model4: entity.model4.clone(),
            spawnflags: entity.spawnflags,
            delay: entity.delay,
            wait: entity.wait,
            speed: entity.speed,
            accel: entity.accel,
            decel: entity.decel,
            damage: entity.damage,
            damage_radius: entity.damage_radius,
            radius_damage: entity.radius_damage,
            count: entity.count,
            max_health: entity.max_health,
            view_height: entity.view_height,
            frame: entity.frame,
            old_frame: entity.old_frame,
            scale: entity.scale,
            alpha: entity.alpha,
            skin: entity.skin,
            effects: entity.effects,
            render_flags: entity.render_flags,
            flags: entity.flags,
            server_flags: entity.server_flags,
            light_level: entity.light_level,
            power_cubes: entity.power_cubes,
            timestamp: entity.timestamp,
            noise: entity.noise.clone(),
            sound: entity.sound.clone(),
            volume: entity.volume,
            attenuation: entity.attenuation,
            random: entity.random,
            map: entity.map.clone(),
            style: entity.style,
            transition_started: entity.transition_started,
            clip_mask: entity.clip_mask,
            projectile: entity.projectile,
            dodgeable: entity.dodgeable,
            laser_immune: entity.laser_immune,
            damageable_target: entity.damageable_target,
            visible: entity.visible,
            solid: entity.solid,
            motion: entity.motion,
            gravity: entity.gravity,
            gravity_vector: entity.gravity_vector,
            angular_velocity: entity.angular_velocity,
            movedir: entity.movedir,
            pos1: entity.pos1,
            pos2: entity.pos2,
            next_think: entity.next_think,
        }
    }

    /// Restore value fields into an entity.
    pub fn restore(&self, entity: &mut Q2Entity) {
        entity.classname.clone_from(&self.classname);
        entity.target.clone_from(&self.target);
        entity.targetname.clone_from(&self.targetname);
        entity.killtarget.clone_from(&self.killtarget);
        entity.combat_target.clone_from(&self.combat_target);
        entity.death_target.clone_from(&self.death_target);
        entity.health_target.clone_from(&self.health_target);
        entity.item_target.clone_from(&self.item_target);
        entity.message.clone_from(&self.message);
        entity.model.clone_from(&self.model);
        entity.model2.clone_from(&self.model2);
        entity.model3.clone_from(&self.model3);
        entity.model4.clone_from(&self.model4);
        entity.spawnflags = self.spawnflags;
        entity.delay = self.delay;
        entity.wait = self.wait;
        entity.speed = self.speed;
        entity.accel = self.accel;
        entity.decel = self.decel;
        entity.damage = self.damage;
        entity.damage_radius = self.damage_radius;
        entity.radius_damage = self.radius_damage;
        entity.count = self.count;
        entity.max_health = self.max_health;
        entity.view_height = self.view_height;
        entity.frame = self.frame;
        entity.old_frame = self.old_frame;
        entity.scale = self.scale;
        entity.alpha = self.alpha;
        entity.skin = self.skin;
        entity.effects = self.effects;
        entity.render_flags = self.render_flags;
        entity.flags = self.flags;
        entity.server_flags = self.server_flags;
        entity.light_level = self.light_level;
        entity.power_cubes = self.power_cubes;
        entity.timestamp = self.timestamp;
        entity.noise.clone_from(&self.noise);
        entity.sound.clone_from(&self.sound);
        entity.volume = self.volume;
        entity.attenuation = self.attenuation;
        entity.random = self.random;
        entity.map.clone_from(&self.map);
        entity.style = self.style;
        entity.transition_started = self.transition_started;
        entity.clip_mask = self.clip_mask;
        entity.projectile = self.projectile;
        entity.dodgeable = self.dodgeable;
        entity.laser_immune = self.laser_immune;
        entity.damageable_target = self.damageable_target;
        entity.visible = self.visible;
        entity.solid = self.solid;
        entity.motion = self.motion;
        entity.gravity = self.gravity;
        entity.gravity_vector = self.gravity_vector;
        entity.angular_velocity = self.angular_velocity;
        entity.movedir = self.movedir;
        entity.pos1 = self.pos1;
        entity.pos2 = self.pos2;
        entity.next_think = self.next_think;
    }
}

/// Entity link fields (`Q2EntityCheckpoint["links"]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Q2EntityLinks {
    /// Activator.
    pub activator: Option<SavedActorId>,
    /// Enemy.
    pub enemy: Option<SavedActorId>,
    /// Owner.
    pub owner: Option<SavedActorId>,
    /// Goal.
    pub goal: Option<SavedActorId>,
    /// Team master.
    pub team_master: Option<SavedActorId>,
    /// Team chain.
    pub team_chain: Option<SavedActorId>,
    /// Chain.
    pub chain: Option<SavedActorId>,
    /// Beam.
    pub beam: Option<SavedActorId>,
    /// Second beam.
    pub beam2: Option<SavedActorId>,
    /// Proboscis.
    pub proboscus: Option<SavedActorId>,
}

impl Q2EntityLinks {
    /// Capture link fields from an entity.
    pub fn capture(entity: &Q2Entity) -> Self {
        Self {
            activator: save_q2_actor(entity.activator.as_ref()),
            enemy: save_q2_actor(entity.enemy.as_ref()),
            owner: save_q2_actor(entity.owner.as_ref()),
            goal: save_q2_actor(entity.goal.as_ref()),
            team_master: save_q2_actor(entity.team_master.as_ref()),
            team_chain: save_q2_actor(entity.team_chain.as_ref()),
            chain: save_q2_actor(entity.chain.as_ref()),
            beam: save_q2_actor(entity.beam.as_ref()),
            beam2: save_q2_actor(entity.beam2.as_ref()),
            proboscus: save_q2_actor(entity.proboscus.as_ref()),
        }
    }
}

/// Entity callback names (`Q2EntityCheckpoint["callbacks"]`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Q2EntityCallbackNames {
    /// Think callback name.
    pub think: Option<String>,
    /// Pre-physics callback name.
    pub prethink: Option<String>,
    /// Post-physics callback name.
    pub postthink: Option<String>,
    /// Use callback name.
    pub use_: Option<String>,
    /// Touch callback name.
    pub touch: Option<String>,
    /// Pain callback name.
    pub pain: Option<String>,
    /// Die callback name.
    pub die: Option<String>,
    /// Blocked callback name.
    pub blocked: Option<String>,
}

/// One entity checkpoint (`Q2EntityCheckpoint`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2EntityCheckpoint {
    /// Saved actor.
    pub actor: SavedActorId,
    /// Source slot.
    pub source_slot: Option<u32>,
    /// Spawn classname.
    pub classname: String,
    /// Spawn ordinal.
    pub ordinal: i32,
    /// Spawn values.
    pub spawn_values: Vec<(String, String)>,
    /// Value fields.
    pub values: Q2EntityValues,
    /// Link fields.
    pub links: Q2EntityLinks,
    /// Last attack.
    pub last_attack: Option<Q2AttackCheckpoint>,
    /// Callback names.
    pub callbacks: Q2EntityCallbackNames,
}

/// Foundation checkpoint (`Q2FoundationCheckpoint`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2FoundationCheckpoint {
    /// Checkpoint version (always 1).
    pub version: u32,
    /// Next source slot.
    pub next_source_slot: u32,
    /// Attack sequence.
    pub sequence: u64,
    /// Freed slots with release times.
    pub freed_slots: Vec<(u32, f64)>,
    /// Source counters.
    pub counters: Q2Counters,
    /// Entity checkpoints.
    pub entities: Vec<Q2EntityCheckpoint>,
}
