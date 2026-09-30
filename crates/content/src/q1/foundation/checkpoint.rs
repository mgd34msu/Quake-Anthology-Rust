//! Official QC source-state checkpoints
//! (`src/content/q1/foundation/checkpoint.ts`).
//!
//! Official QC source state. Shared bodies, combat, inventory, and RNG
//! belong to the session checkpoint.
//!
//! This module also carries the tagged checkpoint envelope codec
//! (`encodeCheckpointValue`/`decodeCheckpointValue`, donor
//! `src/persistence/value.ts`) used by source state extensions. The
//! shared [`crate::value`] layer deliberately stops at readers, and
//! `qa-content` cannot depend on `qa-world`, so the codec lives here.

use qa_core::identity::{OwnedActor, ProviderId, SavedActorId};
use qa_core::math::{Bounds, Vec3};

use crate::value::{parse_save_json, save_error, SaveJson};

use super::entity::{Q1Actor, Q1AttackState, Q1MonsterMode, Q1MonsterSpecies, Q1MoverState, Q1ProjectileKind};
use super::entity_services::Q1EntityServices;
use super::types::{Q1Basis, Q1Edition, Q1MoveType, Q1Powerup, Q1PrecacheTables, Q1Solid, Q1Weapon};
use crate::q1::{q1_error, Q1Error};

/// Saved callback names (`Q1SavedCallbacks`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q1SavedCallbacks {
    /// Think action name.
    pub think: Option<String>,
    /// Use callback name.
    pub use_callback: Option<String>,
    /// Touch callback name.
    pub touch: Option<String>,
    /// Pain callback name.
    pub pain: Option<String>,
    /// Death callback name.
    pub die: Option<String>,
    /// Blocked callback name.
    pub blocked: Option<String>,
    /// Path-end action name.
    pub path_end: Option<String>,
}

/// Mutable builtin source fields outside shared stores
/// (`Q1EntitySourceState`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q1EntitySourceState {
    /// Model path.
    pub model: String,
    /// Model frame.
    pub frame: i32,
    /// Model skin.
    pub skin: i32,
    /// Effect flags.
    pub effects: i32,
    /// Solidity.
    pub solid: Q1Solid,
    /// Movement type.
    pub movement: Q1MoveType,
    /// Trigger target.
    pub target: String,
    /// Target name.
    pub targetname: String,
    /// Kill target.
    pub killtarget: String,
    /// Center-print message.
    pub message: String,
    /// Trigger delay in seconds.
    pub delay: f64,
    /// Spawn flags.
    pub spawnflags: i32,
    /// Sound set selector.
    pub sounds: i32,
    /// Wait interval in seconds.
    pub wait: f64,
    /// Movement speed.
    pub speed: f64,
    /// Damage value.
    pub damage: f64,
    /// Maximum health.
    pub max_health: f64,
    /// Whether the entity takes aimed damage.
    pub aimed_damage: bool,
    /// Next think time in seconds.
    pub next_think: f64,
    /// Original model path.
    pub original_model: String,
    /// Saved position 1.
    pub pos1: Vec3,
    /// Saved position 2.
    pub pos2: Vec3,
    /// Saved destination 1.
    pub dest1: Vec3,
    /// Saved destination 2.
    pub dest2: Vec3,
    /// Move direction.
    pub movedir: Vec3,
    /// Saved mangle angles.
    pub mangle: Vec3,
    /// Mover position state.
    pub state: Q1MoverState,
    /// Trigger bounds override.
    pub trigger_bounds: Option<Bounds>,
    /// Next allowed attack time in seconds.
    pub attack_finished: f64,
    /// Counter value.
    pub count: f64,
    /// Whether the entity activated.
    pub activated: bool,
    /// Projectile kind, if any.
    pub projectile: Option<Q1ProjectileKind>,
    /// Projectile weapon, if any.
    pub projectile_weapon: Option<Q1Weapon>,
    /// Angular velocity.
    pub angular_velocity: Vec3,
    /// Water depth level.
    pub water_level: i32,
    /// Water contents type.
    pub water_type: i32,
    /// Movement flags.
    pub movement_flags: i32,
    /// Ideal yaw.
    pub ideal_yaw: f64,
    /// Yaw speed.
    pub yaw_speed: f64,
    /// Combat attack state.
    pub attack_state: Q1AttackState,
}

/// Every mutable builtin source field outside shared stores appears
/// here (`captureEntitySourceState`).
#[must_use]
pub fn capture_entity_source_state(entity: &Q1Actor) -> Q1EntitySourceState {
    Q1EntitySourceState {
        model: entity.model.clone(),
        frame: entity.frame,
        skin: entity.skin,
        effects: entity.effects,
        solid: entity.solid,
        movement: entity.movement,
        target: entity.target.clone(),
        targetname: entity.targetname.clone(),
        killtarget: entity.killtarget.clone(),
        message: entity.message.clone(),
        delay: entity.delay,
        spawnflags: entity.spawnflags,
        sounds: entity.sounds,
        wait: entity.wait,
        speed: entity.speed,
        damage: entity.damage,
        max_health: entity.max_health,
        aimed_damage: entity.aimed_damage,
        next_think: entity.next_think,
        original_model: entity.original_model.clone(),
        pos1: entity.pos1,
        pos2: entity.pos2,
        dest1: entity.dest1,
        dest2: entity.dest2,
        movedir: entity.movedir,
        mangle: entity.mangle,
        state: entity.state,
        trigger_bounds: entity.trigger_bounds,
        attack_finished: entity.attack_finished,
        count: entity.count,
        activated: entity.activated,
        projectile: entity.projectile,
        projectile_weapon: entity.projectile_weapon,
        angular_velocity: entity.angular_velocity,
        water_level: entity.water_level,
        water_type: entity.water_type,
        movement_flags: entity.movement_flags,
        ideal_yaw: entity.ideal_yaw,
        yaw_speed: entity.yaw_speed,
        attack_state: entity.attack_state,
    }
}

/// Apply captured source state to an entity (donor `Object.assign`
/// sites in restore and clone paths).
pub fn apply_entity_source_state(entity: &mut Q1Actor, state: &Q1EntitySourceState) {
    entity.model = state.model.clone();
    entity.frame = state.frame;
    entity.skin = state.skin;
    entity.effects = state.effects;
    entity.solid = state.solid;
    entity.movement = state.movement;
    entity.target = state.target.clone();
    entity.targetname = state.targetname.clone();
    entity.killtarget = state.killtarget.clone();
    entity.message = state.message.clone();
    entity.delay = state.delay;
    entity.spawnflags = state.spawnflags;
    entity.sounds = state.sounds;
    entity.wait = state.wait;
    entity.speed = state.speed;
    entity.damage = state.damage;
    entity.max_health = state.max_health;
    entity.aimed_damage = state.aimed_damage;
    entity.next_think = state.next_think;
    entity.original_model = state.original_model.clone();
    entity.pos1 = state.pos1;
    entity.pos2 = state.pos2;
    entity.dest1 = state.dest1;
    entity.dest2 = state.dest2;
    entity.movedir = state.movedir;
    entity.mangle = state.mangle;
    entity.state = state.state;
    entity.trigger_bounds = state.trigger_bounds;
    entity.attack_finished = state.attack_finished;
    entity.count = state.count;
    entity.activated = state.activated;
    entity.projectile = state.projectile;
    entity.projectile_weapon = state.projectile_weapon;
    entity.angular_velocity = state.angular_velocity;
    entity.water_level = state.water_level;
    entity.water_type = state.water_type;
    entity.movement_flags = state.movement_flags;
    entity.ideal_yaw = state.ideal_yaw;
    entity.yaw_speed = state.yaw_speed;
    entity.attack_state = state.attack_state;
}

/// Save an actor id (`saveQ1Actor`).
#[must_use]
pub fn save_q1_actor(actor: Option<&qa_core::identity::ActorId>) -> Option<SavedActorId> {
    actor.map(SavedActorId::from)
}

/// Saved entity string field.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1SavedField {
    /// Field key.
    pub key: String,
    /// Field value.
    pub value: String,
}

/// Saved entity actor reference.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1SavedReference {
    /// Reference key.
    pub key: String,
    /// Referenced actor.
    pub actor: Option<SavedActorId>,
}

/// Saved monster state (`Q1SavedEntity["monster"]`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q1SavedMonster {
    /// Species.
    pub species: Q1MonsterSpecies,
    /// Behavior mode.
    pub mode: Q1MonsterMode,
    /// Frame index within the sequence.
    pub frame_index: usize,
    /// Step distances per frame.
    pub sequence: Vec<f64>,
    /// First model frame of the sequence.
    pub first_frame: i32,
    /// Current enemy.
    pub enemy: Option<SavedActorId>,
    /// Previous enemy.
    pub old_enemy: Option<SavedActorId>,
    /// Patrol path name.
    pub path: String,
    /// Patrol pause expiry in seconds.
    pub pause_until: f64,
    /// Next allowed attack time in seconds.
    pub attack_finished: f64,
    /// Pain cooldown expiry in seconds.
    pub pain_finished: f64,
    /// Target search expiry in seconds.
    pub search_until: f64,
    /// Whether the death drop ran.
    pub death_drop: bool,
    /// Whether the attack refired.
    pub refired: bool,
}

/// Saved mover completion (`Q1SavedEntity["move"]`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q1SavedMove {
    /// Move destination.
    pub destination: Vec3,
    /// Completion action name.
    pub done: String,
}

/// Saved entity (`Q1SavedEntity`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q1SavedEntity {
    /// Saved actor.
    pub actor: SavedActorId,
    /// Source slot.
    pub source_slot: Option<u32>,
    /// Actor provider.
    pub actor_provider: ProviderId,
    /// Entity classname.
    pub classname: String,
    /// Source entity ordinal.
    pub source_ordinal: Option<i32>,
    /// Captured source state.
    pub state: Q1EntitySourceState,
    /// String fields.
    pub fields: Vec<Q1SavedField>,
    /// Actor references.
    pub references: Vec<Q1SavedReference>,
    /// Owning actor.
    pub owner: Option<SavedActorId>,
    /// Activating actor.
    pub activator: Option<SavedActorId>,
    /// Linked door group.
    pub door_group: Vec<SavedActorId>,
    /// Monster state, if any.
    pub monster: Option<Q1SavedMonster>,
    /// Pending mover completion, if any.
    pub move_completion: Option<Q1SavedMove>,
    /// Callback names.
    pub callbacks: Q1SavedCallbacks,
}

/// Saved player arsenal state (`Q1SavedPlayer["state"]`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q1SavedPlayerState {
    /// Fade alpha (absent in older saves).
    pub alpha: Option<f64>,
    /// Model scale (absent in older saves).
    pub scale: Option<f64>,
    /// Selected weapon.
    pub weapon: Q1Weapon,
    /// Whether the primary weapon is holstered.
    pub primary_holstered: bool,
    /// Next allowed attack time in seconds.
    pub attack_finished: f64,
    /// Whether attack is held.
    pub attack_held: bool,
    /// Whether jump is held.
    pub jump_held: bool,
    /// Teleport control lock expiry in seconds.
    pub teleport_until: f64,
    /// View model frame.
    pub weapon_frame: i32,
    /// Weapon animation start in seconds.
    pub weapon_animation_at: f64,
    /// Weapon animation base frame.
    pub weapon_animation_base: i32,
    /// Whether the weapon fires continuously.
    pub continuous_firing: bool,
    /// Next continuous-fire time in seconds.
    pub next_weapon_frame: f64,
    /// Next lightning-loop time in seconds.
    pub lightning_sound_at: f64,
    /// View punch angles.
    pub punch_angles: Vec3,
    /// Nailgun muzzle side.
    pub nail_side: f64,
    /// Maximum health.
    pub max_health: f64,
    /// Next megahealth rot time in seconds.
    pub mega_rot_at: f64,
    /// Hostility expiry in seconds.
    pub hostile_until: f64,
    /// View angles.
    pub view_angles: Vec3,
    /// Water depth level.
    pub water_level: i32,
    /// Air supply expiry in seconds.
    pub air_finished: f64,
    /// Pending drown damage.
    pub drown_damage: f64,
    /// Next drown tick in seconds.
    pub drown_at: f64,
    /// Next hazard tick in seconds.
    pub hazard_at: f64,
    /// Automatic weapon switch policy.
    pub auto_switch: super::types::Q1AutoSwitch,
}

/// Saved powerup expiry.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1SavedPowerup {
    /// Powerup kind.
    pub kind: Q1Powerup,
    /// Expiry in seconds.
    pub expires: f64,
}

/// Saved player (`Q1SavedPlayer`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q1SavedPlayer {
    /// Actor provider.
    pub actor_provider: ProviderId,
    /// Saved actor.
    pub actor: SavedActorId,
    /// Arsenal state.
    pub state: Q1SavedPlayerState,
    /// Active powerups.
    pub powerups: Vec<Q1SavedPowerup>,
}

/// Saved intermission.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1SavedIntermission {
    /// Destination map.
    pub map: String,
    /// Travel cause.
    pub cause: Option<SavedActorId>,
    /// Exit availability in seconds.
    pub exit_after: f64,
}

/// Saved state extension bytes.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1SavedExtension {
    /// Extension id.
    pub id: String,
    /// Captured bytes.
    pub bytes: Vec<u8>,
}

/// Foundation checkpoint (`Q1FoundationCheckpoint`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q1FoundationCheckpoint {
    /// Checkpoint format (`"q1-foundation"`).
    pub format: String,
    /// Operating provider.
    pub provider: ProviderId,
    /// Checkpoint version (`5`).
    pub version: u32,
    /// Precache tables.
    pub precaches: Q1PrecacheTables,
    /// Content edition.
    pub edition: Q1Edition,
    /// Source time in seconds.
    pub time: f64,
    /// Frame elapsed seconds.
    pub frame_seconds: f64,
    /// Pending forced retouches.
    pub force_retouch: i32,
    /// Saved QC basis.
    pub basis: Q1Basis,
    /// Attack sequence.
    pub sequence: u64,
    /// Next dynamic source slot.
    pub next_dynamic_slot: u32,
    /// Total secrets.
    pub total_secrets: i32,
    /// Secrets found.
    pub found_secrets: i32,
    /// Total monsters.
    pub total_monsters: i32,
    /// Monsters killed.
    pub killed_monsters: i32,
    /// World type.
    pub world_type: i32,
    /// Map name.
    pub map_name: String,
    /// World actor.
    pub world: Option<SavedActorId>,
    /// Sight entity.
    pub sight_entity: Option<SavedActorId>,
    /// Sight time in seconds.
    pub sight_time: f64,
    /// Pending intermission.
    pub intermission: Option<Q1SavedIntermission>,
    /// Saved entities.
    pub entities: Vec<Q1SavedEntity>,
    /// Saved players.
    pub players: Vec<Q1SavedPlayer>,
    /// Saved extensions.
    pub extensions: Vec<Q1SavedExtension>,
}

fn saved_owned(actor: &OwnedActor) -> SavedActorId {
    SavedActorId::from(actor.id())
}

fn save_entity(game: &mut Q1EntityServices, id: &qa_core::identity::ActorId) -> Result<Q1SavedEntity, Q1Error> {
    let entity = game
        .entity_ref(id)
        .ok_or_else(|| q1_error("Missing Q1 save entity"))?
        .clone();
    let source = game.host.actors.source_of(id);
    let done = entity
        .move_completion
        .as_ref()
        .map(|completion| completion.done.clone());
    if entity.move_completion.is_some() && done.is_none() {
        return Err(q1_error("Q1 move has no named completion"));
    }
    Ok(Q1SavedEntity {
        actor: saved_owned(&entity.actor),
        source_slot: source.map(|source| source.slot),
        actor_provider: entity.actor.owner().clone(),
        classname: entity.classname.clone(),
        source_ordinal: entity.source_ordinal,
        state: capture_entity_source_state(&entity),
        fields: entity
            .fields
            .iter()
            .map(|(key, value)| Q1SavedField {
                key: key.clone(),
                value: value.clone(),
            })
            .collect(),
        references: entity
            .references
            .iter()
            .map(|(key, actor)| Q1SavedReference {
                key: key.clone(),
                actor: save_q1_actor(actor.as_ref()),
            })
            .collect(),
        owner: save_q1_actor(entity.owner.as_ref()),
        activator: save_q1_actor(entity.activator.as_ref()),
        door_group: entity.door_group.iter().map(SavedActorId::from).collect(),
        monster: entity.monster.as_ref().map(|monster| Q1SavedMonster {
            species: monster.species,
            mode: monster.mode,
            frame_index: monster.frame_index,
            sequence: monster.sequence.clone(),
            first_frame: monster.first_frame,
            enemy: save_q1_actor(monster.enemy.as_ref()),
            old_enemy: save_q1_actor(monster.old_enemy.as_ref()),
            path: monster.path.clone(),
            pause_until: monster.pause_until,
            attack_finished: monster.attack_finished,
            pain_finished: monster.pain_finished,
            search_until: monster.search_until,
            death_drop: monster.death_drop,
            refired: monster.refired,
        }),
        move_completion: match (&entity.move_completion, done) {
            (Some(completion), Some(done)) => Some(Q1SavedMove {
                destination: completion.destination,
                done,
            }),
            _ => None,
        },
        callbacks: Q1SavedCallbacks {
            think: entity.think.clone(),
            use_callback: entity.use_callback.clone(),
            touch: entity.touch.clone(),
            pain: entity.pain.clone(),
            die: entity.die.clone(),
            blocked: entity.blocked.clone(),
            path_end: entity.path_end.clone(),
        },
    })
}

/// Capture a foundation checkpoint (`captureFoundation`).
pub fn capture_foundation(
    game: &mut Q1EntityServices,
    sequence: u64,
    next_dynamic_slot: u32,
) -> Result<Q1FoundationCheckpoint, Q1Error> {
    let entity_ids: Vec<qa_core::identity::ActorId> = game.entity_ids();
    let mut entities = Vec::with_capacity(entity_ids.len());
    for id in &entity_ids {
        entities.push(save_entity(game, id)?);
    }
    let players = game
        .players_snapshot()
        .into_iter()
        .map(|player| Q1SavedPlayer {
            actor: saved_owned(&player.actor),
            actor_provider: player.actor.owner().clone(),
            state: Q1SavedPlayerState {
                alpha: Some(player.alpha),
                scale: Some(player.scale),
                weapon: player.weapon,
                primary_holstered: player.primary_holstered,
                attack_finished: player.attack_finished,
                attack_held: player.attack_held,
                jump_held: player.jump_held,
                teleport_until: player.teleport_until,
                weapon_frame: player.weapon_frame,
                weapon_animation_at: player.weapon_animation_at,
                weapon_animation_base: player.weapon_animation_base,
                continuous_firing: player.continuous_firing,
                next_weapon_frame: player.next_weapon_frame,
                lightning_sound_at: player.lightning_sound_at,
                punch_angles: player.punch_angles,
                nail_side: player.nail_side,
                max_health: player.max_health,
                mega_rot_at: player.mega_rot_at,
                hostile_until: player.hostile_until,
                view_angles: player.view_angles,
                water_level: player.water_level,
                air_finished: player.air_finished,
                drown_damage: player.drown_damage,
                drown_at: player.drown_at,
                hazard_at: player.hazard_at,
                auto_switch: player.auto_switch,
            },
            powerups: player
                .powerups
                .iter()
                .map(|(kind, expires)| Q1SavedPowerup {
                    kind: *kind,
                    expires: *expires,
                })
                .collect(),
        })
        .collect();
    Ok(Q1FoundationCheckpoint {
        format: String::from("q1-foundation"),
        provider: game.provider(),
        version: 5,
        precaches: Q1PrecacheTables {
            phase: game.precaches.phase(),
            models: game.precaches.models().to_vec(),
            sounds: game.precaches.sounds().to_vec(),
        },
        edition: game.options().edition,
        time: game.time,
        frame_seconds: game.frame_seconds,
        force_retouch: game.force_retouch,
        basis: game.basis,
        sequence,
        next_dynamic_slot,
        total_secrets: game.total_secrets,
        found_secrets: game.found_secrets,
        total_monsters: game.total_monsters,
        killed_monsters: game.killed_monsters,
        world_type: game.world_type,
        map_name: game.map_name.clone(),
        world: game.world.as_ref().map(SavedActorId::from),
        sight_entity: game.sight_entity.as_ref().map(SavedActorId::from),
        sight_time: game.sight_time,
        intermission: game.intermission.as_ref().map(|intermission| Q1SavedIntermission {
            map: intermission.map.clone(),
            cause: save_q1_actor(intermission.cause.as_ref()),
            exit_after: intermission.exit_after,
        }),
        entities,
        players,
        extensions: game
            .state_extensions
            .values()
            .map(|extension| Q1SavedExtension {
                id: extension.id().to_string(),
                bytes: extension.capture(),
            })
            .collect(),
    })
}

/// Restore source objects around existing authority tables
/// (`restoreFoundation`). It never runs a spawn function.
pub fn restore_foundation(game: &mut Q1EntityServices, checkpoint: &Q1FoundationCheckpoint) -> Result<(), Q1Error> {
    if checkpoint.format != "q1-foundation"
        || checkpoint.version != 5
        || checkpoint.edition != game.options().edition
        || checkpoint.provider != game.provider()
    {
        return Err(q1_error("Incompatible Q1 source checkpoint"));
    }
    if !game.entities.is_empty() || !game.players.is_empty() {
        return Err(q1_error("Restore Q1 source state into a fresh provider"));
    }
    game.precaches.restore(&checkpoint.precaches)?;
    for saved in &checkpoint.entities {
        let actor = game.host.actors.resolve_saved(&saved.actor).ok_or_else(|| {
            q1_error(format!(
                "Missing restored Q1 actor {}/{}",
                saved.actor.slot, saved.actor.generation
            ))
        })?;
        let source = game.host.actors.source_of(actor.id());
        if actor.owner() != &saved.actor_provider
            || source.as_ref().map(|source| source.slot) != saved.source_slot
            || source.as_ref().is_some_and(|source| &source.provider != actor.owner())
            || game.entities.contains_key(actor.id())
        {
            return Err(q1_error("Q1 restored source-slot mismatch"));
        }
        if game.host.bodies.read(actor.id()).is_none() || game.host.combat.read(actor.id()).is_none() {
            return Err(q1_error("Restore shared Q1 body and combat stores before source state"));
        }
        let id = game.attach_existing(actor, &saved.classname, None, saved.source_ordinal)?;
        let state = saved.state.clone();
        game.update_entity(&id, |entity| apply_entity_source_state(entity, &state))?;
        for field in &saved.fields {
            let key = field.key.clone();
            let value = field.value.clone();
            game.update_entity(&id, |entity| {
                entity.fields.insert(key, value);
            })?;
        }
    }
    for saved in &checkpoint.entities {
        let id = game
            .host
            .actors
            .resolve_saved(&saved.actor)
            .and_then(|owned| game.entities.get(owned.id()).map(|entity| entity.actor.id().clone()))
            .ok_or_else(|| q1_error("Saved Q1 source reference points outside source entities"))?;
        for entry in &saved.references {
            let actor = entry
                .actor
                .as_ref()
                .map(|saved| game.host.actors.reference_saved(saved));
            let key = entry.key.clone();
            game.update_entity(&id, |entity| {
                entity.references.insert(key, actor);
            })?;
        }
        let owner = saved
            .owner
            .as_ref()
            .map(|saved| game.host.actors.reference_saved(saved));
        let activator = saved
            .activator
            .as_ref()
            .map(|saved| game.host.actors.reference_saved(saved));
        let mut door_group = Vec::with_capacity(saved.door_group.len());
        for member in &saved.door_group {
            door_group.push(
                game.host
                    .actors
                    .resolve_saved(member)
                    .and_then(|owned| game.entities.get(owned.id()).map(|entity| entity.actor.id().clone()))
                    .ok_or_else(|| q1_error("Saved Q1 source reference points outside source entities"))?,
            );
        }
        let monster = saved.monster.as_ref().map(|monster| super::entity::Q1Monster {
            species: monster.species,
            mode: monster.mode,
            frame_index: monster.frame_index,
            sequence: monster.sequence.clone(),
            first_frame: monster.first_frame,
            enemy: monster
                .enemy
                .as_ref()
                .map(|saved| game.host.actors.reference_saved(saved)),
            old_enemy: monster
                .old_enemy
                .as_ref()
                .map(|saved| game.host.actors.reference_saved(saved)),
            path: monster.path.clone(),
            pause_until: monster.pause_until,
            attack_finished: monster.attack_finished,
            pain_finished: monster.pain_finished,
            search_until: monster.search_until,
            death_drop: monster.death_drop,
            refired: monster.refired,
        });
        // Callback names revalidate against the registered handlers,
        // exactly like the donor's `named.*` rebinds.
        let callbacks = saved.callbacks.clone();
        let think = callbacks
            .think
            .as_deref()
            .map(|name| game.named.action(name))
            .transpose()?;
        let use_callback = callbacks
            .use_callback
            .as_deref()
            .map(|name| game.named.use_callback(name))
            .transpose()?;
        let touch = callbacks
            .touch
            .as_deref()
            .map(|name| game.named.touch(name))
            .transpose()?;
        let pain = callbacks
            .pain
            .as_deref()
            .map(|name| game.named.pain(name))
            .transpose()?;
        let die = callbacks.die.as_deref().map(|name| game.named.die(name)).transpose()?;
        let blocked = callbacks
            .blocked
            .as_deref()
            .map(|name| game.named.blocked(name))
            .transpose()?;
        let path_end = callbacks
            .path_end
            .as_deref()
            .map(|name| game.named.action(name))
            .transpose()?;
        let move_completion = saved
            .move_completion
            .as_ref()
            .map(|saved| {
                game.named.action(&saved.done).map(|done| super::entity::Q1Move {
                    destination: saved.destination,
                    done,
                })
            })
            .transpose()?;
        game.update_entity(&id, |entity| {
            entity.owner = owner;
            entity.activator = activator;
            entity.door_group = door_group;
            entity.monster = monster;
            entity.move_completion = move_completion;
            entity.think = think;
            entity.use_callback = use_callback;
            entity.touch = touch;
            entity.pain = pain;
            entity.die = die;
            entity.blocked = blocked;
            entity.path_end = path_end;
        })?;
    }
    for saved in &checkpoint.players {
        let actor = game.host.actors.resolve_saved(&saved.actor).ok_or_else(|| {
            q1_error(format!(
                "Missing restored Q1 actor {}/{}",
                saved.actor.slot, saved.actor.generation
            ))
        })?;
        if actor.owner() != &saved.actor_provider {
            return Err(q1_error("Q1 restored player owner mismatch"));
        }
        if game.players.contains_key(actor.id()) {
            return Err(q1_error("Duplicate saved Q1 player"));
        }
        let mut powerups = std::collections::HashMap::new();
        for powerup in &saved.powerups {
            powerups.insert(powerup.kind, powerup.expires);
        }
        game.players.insert(
            actor.id().clone(),
            super::types::Q1PlayerState {
                alpha: saved.state.alpha.unwrap_or(0.0),
                scale: saved.state.scale.unwrap_or(0.0),
                actor: actor.clone(),
                weapon: saved.state.weapon,
                primary_holstered: saved.state.primary_holstered,
                attack_finished: saved.state.attack_finished,
                attack_held: saved.state.attack_held,
                jump_held: saved.state.jump_held,
                teleport_until: saved.state.teleport_until,
                weapon_frame: saved.state.weapon_frame,
                weapon_animation_at: saved.state.weapon_animation_at,
                weapon_animation_base: saved.state.weapon_animation_base,
                continuous_firing: saved.state.continuous_firing,
                next_weapon_frame: saved.state.next_weapon_frame,
                lightning_sound_at: saved.state.lightning_sound_at,
                punch_angles: saved.state.punch_angles,
                nail_side: saved.state.nail_side,
                max_health: saved.state.max_health,
                mega_rot_at: saved.state.mega_rot_at,
                hostile_until: saved.state.hostile_until,
                view_angles: saved.state.view_angles,
                water_level: saved.state.water_level,
                air_finished: saved.state.air_finished,
                drown_damage: saved.state.drown_damage,
                drown_at: saved.state.drown_at,
                hazard_at: saved.state.hazard_at,
                auto_switch: saved.state.auto_switch,
                powerups,
            },
        );
    }
    game.time = checkpoint.time;
    game.frame_seconds = checkpoint.frame_seconds;
    game.force_retouch = checkpoint.force_retouch;
    game.basis = checkpoint.basis;
    game.total_secrets = checkpoint.total_secrets;
    game.found_secrets = checkpoint.found_secrets;
    game.total_monsters = checkpoint.total_monsters;
    game.killed_monsters = checkpoint.killed_monsters;
    game.world_type = checkpoint.world_type;
    game.map_name = checkpoint.map_name.clone();
    game.world = checkpoint
        .world
        .as_ref()
        .map(|saved| {
            game.host
                .actors
                .resolve_saved(saved)
                .and_then(|owned| game.entities.get(owned.id()).map(|entity| entity.actor.id().clone()))
                .ok_or_else(|| q1_error("Saved Q1 source reference points outside source entities"))
        })
        .transpose()?;
    game.sight_entity = checkpoint
        .sight_entity
        .as_ref()
        .map(|saved| {
            game.host
                .actors
                .resolve_saved(saved)
                .and_then(|owned| game.entities.get(owned.id()).map(|entity| entity.actor.id().clone()))
                .ok_or_else(|| q1_error("Saved Q1 source reference points outside source entities"))
        })
        .transpose()?;
    game.sight_time = checkpoint.sight_time;
    game.intermission = checkpoint.intermission.as_ref().map(|intermission| {
        let cause = intermission
            .cause
            .as_ref()
            .map(|saved| game.host.actors.reference_saved(saved));
        super::entity_services::Q1Intermission {
            map: intermission.map.clone(),
            cause,
            exit_after: intermission.exit_after,
        }
    });
    let mut restored = std::collections::HashSet::new();
    for saved in &checkpoint.extensions {
        let extension = game
            .state_extensions
            .get_mut(&saved.id)
            .filter(|_| !restored.contains(&saved.id))
            .ok_or_else(|| q1_error(format!("Unknown or duplicate Q1 saved extension: {}", saved.id)))?;
        extension.restore(&saved.bytes)?;
        restored.insert(saved.id.clone());
    }
    for id in game.state_extensions.keys() {
        if !restored.contains(id) {
            return Err(q1_error(format!("Missing Q1 saved extension: {id}")));
        }
    }
    Ok(())
}

const BASE64_ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Encode bytes as standard padded base64.
#[must_use]
pub fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let word = (u32::from(chunk[0]) << 16)
            | (u32::from(chunk.get(1).copied().unwrap_or(0)) << 8)
            | u32::from(chunk.get(2).copied().unwrap_or(0));
        out.push(BASE64_ALPHABET[((word >> 18) & 63) as usize] as char);
        out.push(BASE64_ALPHABET[((word >> 12) & 63) as usize] as char);
        if chunk.len() > 1 {
            out.push(BASE64_ALPHABET[((word >> 6) & 63) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(BASE64_ALPHABET[(word & 63) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

fn base64_digit(byte: u8) -> Option<u32> {
    match byte {
        b'A'..=b'Z' => Some(u32::from(byte - b'A')),
        b'a'..=b'z' => Some(u32::from(byte - b'a') + 26),
        b'0'..=b'9' => Some(u32::from(byte - b'0') + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

/// Decode canonical base64 (padding plus zeroed trailing bits
/// enforced, donor `canonicalBase64`).
fn base64_decode(text: &str) -> Result<Vec<u8>, Q1Error> {
    let bytes = text.as_bytes();
    if !bytes.len().is_multiple_of(4) {
        return Err(q1_error("checkpoint: unknown tagged checkpoint value"));
    }
    let padding = if text.ends_with("==") {
        2
    } else if text.ends_with('=') {
        1
    } else {
        0
    };
    let end = bytes.len() - padding;
    let mut digits = Vec::with_capacity(end);
    for &byte in &bytes[..end] {
        digits.push(base64_digit(byte).ok_or_else(|| q1_error("checkpoint: unknown tagged checkpoint value"))?);
    }
    for &byte in &bytes[end..] {
        if byte != b'=' {
            return Err(q1_error("checkpoint: unknown tagged checkpoint value"));
        }
    }
    if let Some(&last) = digits.last() {
        if padding == 2 && last & 15 != 0 {
            return Err(q1_error("checkpoint: unknown tagged checkpoint value"));
        }
        if padding == 1 && last & 3 != 0 {
            return Err(q1_error("checkpoint: unknown tagged checkpoint value"));
        }
    }
    let mut out = Vec::new();
    for chunk in digits.chunks(4) {
        let mut word = 0u32;
        for (index, &digit) in chunk.iter().enumerate() {
            word |= digit << (18 - index * 6);
        }
        let count = if chunk.len() == 4 { 3 } else { chunk.len() - 1 };
        for index in 0..count {
            out.push(((word >> (16 - index * 8)) & 0xff) as u8);
        }
    }
    Ok(out)
}

fn escape_into(text: &str, out: &mut String) {
    out.push('"');
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            character if (character as u32) < 32 => {
                out.push_str(&format!("\\u{:04x}", character as u32));
            }
            character => out.push(character),
        }
    }
    out.push('"');
}

fn render_number(value: f64, out: &mut String) {
    if value == 0.0 && value.is_sign_negative() {
        out.push_str(r#"{"$qts":"number","value":"-0"}"#);
    } else if value.is_nan() {
        out.push_str(r#"{"$qts":"number","value":"NaN"}"#);
    } else if value == f64::INFINITY {
        out.push_str(r#"{"$qts":"number","value":"Infinity"}"#);
    } else if value == f64::NEG_INFINITY {
        out.push_str(r#"{"$qts":"number","value":"-Infinity"}"#);
    } else {
        out.push_str(&format!("{value:?}"));
    }
}

fn render(value: &SaveJson, out: &mut String) {
    match value {
        SaveJson::Null => out.push_str("null"),
        SaveJson::Bool(true) => out.push_str("true"),
        SaveJson::Bool(false) => out.push_str("false"),
        SaveJson::Number(value) => render_number(*value, out),
        SaveJson::BigInt(value) => {
            out.push_str(r#"{"$qts":"bigint","value":""#);
            out.push_str(&format!("{value}"));
            out.push_str(r#""}"#);
        }
        SaveJson::Bytes(bytes) => {
            out.push_str(r#"{"$qts":"bytes","value":""#);
            out.push_str(&base64_encode(bytes));
            out.push_str(r#""}"#);
        }
        SaveJson::String(text) => escape_into(text, out),
        SaveJson::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                render(item, out);
            }
            out.push(']');
        }
        SaveJson::Object(members) => {
            for (key, _) in members {
                assert_ne!(*key, "$qts", "checkpoint contains reserved $qts key");
            }
            out.push('{');
            for (index, (key, item)) in members.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                escape_into(key, out);
                out.push(':');
                render(item, out);
            }
            out.push('}');
        }
    }
}

/// Encode a checkpoint value to tagged JSON bytes
/// (`encodeCheckpointValue`, donor `src/persistence/value.ts`).
#[must_use]
pub fn encode_checkpoint_value(value: &SaveJson) -> Vec<u8> {
    let mut out = String::new();
    render(value, &mut out);
    out.into_bytes()
}

fn valid_bigint_text(text: &str) -> bool {
    let digits = text.strip_prefix('-').unwrap_or(text);
    if digits.is_empty() {
        return false;
    }
    if digits.len() > 1 && digits.starts_with('0') {
        return false;
    }
    digits.bytes().all(|byte| byte.is_ascii_digit())
}

fn untag(value: &SaveJson, path: &str) -> Result<SaveJson, Q1Error> {
    match value {
        SaveJson::Object(members) if members.iter().any(|(key, _)| key == "$qts") => {
            if members.len() != 2 {
                return Err(q1_error(format!("{path}: invalid tagged checkpoint value")));
            }
            let tag = members.iter().find(|(key, _)| key == "$qts").map(|(_, value)| value);
            let encoded = members.iter().find(|(key, _)| key == "value").map(|(_, value)| value);
            let (Some(SaveJson::String(tag)), Some(SaveJson::String(encoded))) = (tag, encoded) else {
                return Err(q1_error(format!("{path}: invalid tagged checkpoint value")));
            };
            match tag.as_str() {
                "bigint" if valid_bigint_text(encoded) => encoded
                    .parse::<i128>()
                    .map(SaveJson::BigInt)
                    .map_err(|_| q1_error(format!("{path}: invalid tagged checkpoint value"))),
                "bytes" => base64_decode(encoded).map(SaveJson::Bytes),
                "number" => match encoded.as_str() {
                    "-0" => Ok(SaveJson::Number(-0.0)),
                    "NaN" => Ok(SaveJson::Number(f64::NAN)),
                    "Infinity" => Ok(SaveJson::Number(f64::INFINITY)),
                    "-Infinity" => Ok(SaveJson::Number(f64::NEG_INFINITY)),
                    _ => Err(q1_error(format!("{path}: unknown tagged checkpoint value"))),
                },
                _ => Err(q1_error(format!("{path}: unknown tagged checkpoint value"))),
            }
        }
        SaveJson::Object(members) => members
            .iter()
            .map(|(key, item)| untag(item, &format!("{path}.{key}")).map(|value| (key.clone(), value)))
            .collect::<Result<Vec<_>, _>>()
            .map(SaveJson::Object),
        SaveJson::Array(items) => items
            .iter()
            .enumerate()
            .map(|(index, item)| untag(item, &format!("{path}[{index}]")))
            .collect::<Result<Vec<_>, _>>()
            .map(SaveJson::Array),
        leaf => Ok(leaf.clone()),
    }
}

/// Decode tagged checkpoint JSON bytes (`decodeCheckpointValue`, donor
/// `src/persistence/value.ts`).
pub fn decode_checkpoint_value(bytes: &[u8]) -> Result<SaveJson, Q1Error> {
    let text = std::str::from_utf8(bytes).map_err(|_| save_error("checkpoint", "invalid checkpoint text"))?;
    let parsed = parse_save_json(text)?;
    untag(&parsed, "checkpoint")
}

#[cfg(test)]
mod tests {
    use crate::value::{arr, boolean, int, num, obj, str};

    use super::*;

    #[test]
    fn envelope_round_trips_tagged_values() {
        let value = obj(vec![
            ("name", str("x")),
            ("count", int(3)),
            ("pi", num(3.5)),
            ("yes", boolean(true)),
            ("nothing", SaveJson::Null),
            ("big", SaveJson::BigInt(-42)),
            ("raw", SaveJson::Bytes(vec![0, 1, 2, 250])),
            ("list", arr(vec![num(1.0), str("two")])),
        ]);
        let bytes = encode_checkpoint_value(&value);
        let decoded = decode_checkpoint_value(&bytes).expect("decode");
        assert_eq!(decoded, value);
    }

    #[test]
    fn envelope_rejects_bad_tags() {
        assert!(decode_checkpoint_value(br#"{"$qts":"bytes","value":"!!!"}"#).is_err());
        assert!(decode_checkpoint_value(br#"{"$qts":"number","value":"maybe"}"#).is_err());
    }
}
