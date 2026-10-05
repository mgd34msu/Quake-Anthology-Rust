//! Native Quake I monsters: stock id1 gamecode as compiled Rust.
//!
//! Monster records, spawn functions, the shared AI core (`ai.qc`,
//! `fight.qc`, `monsters.qc`, `combat.qc`), per-monster frame tables,
//! and the PlayWorld-level think pass that drives them against the
//! shared collision scene (spawned through
//! [`spawn_map_entities`](super::super::play_world::spawn_map_entities),
//! stepped by `PlayWorld::step_monsters` after the server tick, the same
//! slot the player step uses).
//!
//! qsrc: `progs106/monsters.qc` (walk/fly/swim starts, monster_use,
//! monster_death_use), `progs106/ai.qc` (FindTarget, FoundTarget,
//! HuntTarget, ai_stand/walk/run/charge/melee/face/turn/pain, CheckAnyAttack),
//! `progs106/fight.qc` (CheckAttack, ai_run_melee/missile/slide),
//! `progs106/combat.qc` (T_Damage, Killed, CanDamage),
//! `progs106/dog.qc` (monster_dog), `progs106/player.qc:435-498`
//! (VelocityForDamage, ThrowGib, ThrowHead),
//! `WinQuake/sv_move.c` (movestep, chase dirs, movetogoal — via
//! [`Q1MonsterMovement`](qa_world::movement::q1::monsters::Q1MonsterMovement)),
//! `WinQuake/sv_phys.c` (step freefall, FlyMove impacts, toss physics),
//! `WinQuake/pr_cmds.c` (walkmove, droptofloor, changeyaw).
//!
//! Skeleton scope notes (each lands with its system, not here): sounds
//! queue in [`Q1NativeBehaviors::sounds`](super::native_q1_spawns::Q1NativeBehaviors)
//! for the audio slice; death obituaries wait for the weapons/HUD slice;
//! quad/godmode/invulnerability/teamplay damage gates wait for the
//! powerup/cheat/team slices (nothing carries those fields yet); the
//! dedicated server spawns monsters but never steps them (no scene, no
//! player) until its own pass lands.

use qa_core::identity::{ActorId, OwnedActor};
use qa_core::math::{vec3, Bounds, Vec3};
use qa_world::combat::{CombatState, RegularArmor};
use qa_world::server::{Server, ServerLogic};
use qa_world::session::Simulation;
use qa_world::spawn::{SpawnFields, SpawnRegistry, SpawnRequest};
use qa_world::WorldError;

use super::native_q1_spawns::{q1_can_take_damage, q1_health_of, Q1NativeBehaviors};
use super::native_q1_triggers::{q1_button_fire, q1_use_targets, Q1UseSource};

/// Stock entity flags (`defs.qc:231-240`).
pub const Q1_FLAG_FLY: i32 = 1;
/// Stock entity flags (`defs.qc:231-240`).
pub const Q1_FLAG_SWIM: i32 = 2;
/// Stock entity flags (`defs.qc:231-240`).
pub const Q1_FLAG_MONSTER: i32 = 32;
/// Stock entity flags (`defs.qc:231-240`).
pub const Q1_FLAG_GODMODE: i32 = 64;
/// Stock entity flags (`defs.qc:231-240`).
pub const Q1_FLAG_NOTARGET: i32 = 128;
/// Stock entity flags (`defs.qc:231-240`).
pub const Q1_FLAG_ONGROUND: i32 = 512;
/// Stock entity flags (`defs.qc:231-240`).
pub const Q1_FLAG_PARTIALGROUND: i32 = 1024;

/// Stock takedamage values (`defs.qc:280-282`).
pub const Q1_DAMAGE_NO: u8 = 0;
/// Stock takedamage values (`defs.qc:280-282`).
pub const Q1_DAMAGE_AIM: u8 = 2;

/// Stock enemy-range bands (`defs.qc:266-269`).
pub const Q1_RANGE_MELEE: u8 = 0;
/// Stock enemy-range bands (`defs.qc:266-269`).
pub const Q1_RANGE_NEAR: u8 = 1;
/// Stock enemy-range bands (`defs.qc:266-269`).
pub const Q1_RANGE_MID: u8 = 2;
/// Stock enemy-range bands (`defs.qc:266-269`).
pub const Q1_RANGE_FAR: u8 = 3;

/// Stock attack states (`defs.qc:452-455`).
pub const Q1_AS_STRAIGHT: u8 = 1;
/// Stock attack states (`defs.qc:452-455`).
pub const Q1_AS_SLIDING: u8 = 2;
/// Stock attack states (`defs.qc:452-455`).
pub const Q1_AS_MELEE: u8 = 3;
/// Stock attack states (`defs.qc:452-455`).
pub const Q1_AS_MISSILE: u8 = 4;

/// Stock sound channels (`defs.qc:360-363`).
pub const Q1_CHAN_AUTO: u8 = 0;
/// Stock sound channels (`defs.qc:360-363`).
pub const Q1_CHAN_WEAPON: u8 = 1;
/// Stock sound channels (`defs.qc:360-363`).
pub const Q1_CHAN_VOICE: u8 = 2;
/// Stock sound channels (`defs.qc:360-363`).
pub const Q1_CHAN_ITEM: u8 = 3;

/// Stock attenuations (`defs.qc:366-369`).
pub const Q1_ATTN_NONE: f32 = 0.0;
/// Stock attenuations (`defs.qc:366-369`).
pub const Q1_ATTN_NORM: f32 = 1.0;
/// Stock attenuations (`defs.qc:366-369`).
pub const Q1_ATTN_IDLE: f32 = 2.0;
/// Stock attenuations (`defs.qc:366-369`).
pub const Q1_ATTN_STATIC: f32 = 3.0;

/// Invisibility item bit (`defs.qc:308`): monsters never acquire its carrier.
pub const Q1_IT_INVISIBILITY: u32 = 524_288;

/// Stock player eye height above the feet origin (`VIEW_OFS`, 22).
pub const Q1_VIEW_OFS_Z: f32 = 22.0;

/// Stock monster eye height above the origin (`monsters.qc`, walk/fly starts).
pub const Q1_MONSTER_VIEW_OFS_Z: f32 = 25.0;

/// Stock think cadence for `$frame` macros (every 0.1 s).
pub const Q1_MONSTER_THINK_STEP: f64 = 0.1;

/// Stock gib velocity levels (`VelocityForDamage`, `player.qc:435`).
const GIB_LEVEL_LIGHT: f32 = 0.7;
/// Stock gib velocity levels (`VelocityForDamage`, `player.qc:435`).
const GIB_LEVEL_HEAVY: f32 = 2.0;
/// Stock gib velocity levels (`VelocityForDamage`, `player.qc:435`).
const GIB_LEVEL_EXTREME: f32 = 10.0;

/// Implemented stock monster kinds (one slice each; the roster grows here).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q1MonsterKind {
    /// `monster_dog` (`dog.qc`).
    Dog,
}

impl Q1MonsterKind {
    /// Stock classname for the kind.
    #[must_use]
    pub fn classname(self) -> &'static str {
        match self {
            Q1MonsterKind::Dog => "monster_dog",
        }
    }

    /// Parse an implemented monster classname (`None` keeps the generic
    /// spawn path until that kind's slice lands).
    #[must_use]
    pub fn from_classname(classname: &str) -> Option<Self> {
        match classname {
            "monster_dog" => Some(Q1MonsterKind::Dog),
            _ => None,
        }
    }
}

/// One `$frame` sequence of a monster model (order and lengths from the
/// kind's `.qc`; the index counts from 0 within the sequence).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q1MonsterSeq {
    /// Dog stand 1-9 (`dog.qc`).
    DogStand,
    /// Dog walk 1-8.
    DogWalk,
    /// Dog run 1-12.
    DogRun,
    /// Dog attack 1-8.
    DogAttack,
    /// Dog leap 1-9.
    DogLeap,
    /// Dog pain 1-6.
    DogPain,
    /// Dog painb 1-16.
    DogPainB,
    /// Dog death 1-9.
    DogDie,
    /// Dog deathb 1-9.
    DogDieB,
}

/// One monster think slot: stock `think` as data (`monsters.qc`, `ai.qc`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q1MonsterThink {
    /// `walkmonster_start_go` (`monsters.qc:69`).
    StartGo,
    /// Delayed `FoundTarget` from `monster_use` (`monsters.qc:21`).
    FoundTarget,
    /// A `$frame` function: sequence plus 0-based index.
    Frame(Q1MonsterSeq, u8),
}

/// Monster touch functions as data (stock assigns function pointers).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Q1MonsterTouch {
    /// `SUB_Null`: no touch function.
    #[default]
    None,
    /// `Dog_JumpTouch` (`dog.qc`).
    JumpTouch,
}

/// Live stock monster gamecode state: the entity fields `ai.qc`,
/// `fight.qc`, and the kind's `.qc` read and write. Origin, angles,
/// velocity, and bounds ride the sim body; health and armor ride the
/// combat state.
#[derive(Debug, Clone)]
pub struct Q1Monster {
    /// Stock monster kind.
    pub kind: Q1MonsterKind,
    /// Current think function.
    pub think: Q1MonsterThink,
    /// Master-clock seconds at which the think fires.
    pub nextthink: f64,
    /// Current model frame index (stock `self.frame`).
    pub frame: i32,
    /// Current enemy (`None` is stock `world`).
    pub enemy: Option<ActorId>,
    /// Previous player enemy, for `ai_run` fall-through (`ai.qc`).
    pub oldenemy: Option<ActorId>,
    /// Current move goal (enemy or movetarget).
    pub goalentity: Option<ActorId>,
    /// Current patrol corner.
    pub movetarget: Option<ActorId>,
    /// Desired yaw in degrees.
    pub ideal_yaw: f64,
    /// Turn rate in degrees per think.
    pub yaw_speed: f64,
    /// Eye offset above the origin.
    pub view_ofs: Vec3,
    /// Stand until this clock instant (`ai_stand`).
    pub pausetime: f64,
    /// Missile attacks resume after this (`SUB_AttackFinished`).
    pub attack_finished: f64,
    /// Pain frames resume after this (nightmare hold).
    pub pain_finished: f64,
    /// Last-seen clock plus 5 s (`ai_run`).
    pub search_time: f64,
    /// Other monsters stay woken until this (`FoundTarget`).
    pub show_hostile: f64,
    /// Current attack state (`AS_*`).
    pub attack_state: u8,
    /// Strafe direction for `ai_run_slide` (`ai.qc`).
    pub lefty: bool,
    /// Nightmare refire count (`SUB_AttackFinished`).
    pub cnt: i32,
    /// Entity flags (`FL_*`).
    pub flags: i32,
    /// Spawn flags (ambush lives in the low bits).
    pub spawnflags: i32,
    /// Damage susceptibility (`DAMAGE_*`).
    pub takedamage: u8,
    /// Touch function.
    pub touch: Q1MonsterTouch,
    /// Firing inputs for `monster_death_use`.
    pub source: Q1UseSource,
    /// Whether `th_die` ran (stock keys off health/takedamage).
    pub dead: bool,
}

/// Live `path_corner` record (`ai.qc:87`): the body holds the corner
/// origin; the record holds the onward link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q1MoveTarget {
    /// Next corner's targetname (`None` ends the route).
    pub target: Option<String>,
}

/// Live gib record: `ThrowGib` chunks and `ThrowHead` heads
/// (`player.qc:459-498`). Spawned chunks are fresh actors; a head keeps
/// the dead monster's actor with its AI record retired.
#[derive(Debug, Clone)]
pub struct Q1Gib {
    /// Angular velocity in degrees per second.
    pub avelocity: Vec3,
    /// Master-clock removal instant (`None` for heads: stock keeps them).
    pub remove_at: Option<f64>,
}

/// One queued `ThrowGib` spawn: the damage path cannot spawn actors, so
/// the monster pass spawns and links these before stepping physics.
#[derive(Debug, Clone)]
pub struct Q1PendingGib {
    /// Gib model path (`progs/gib3.mdl`).
    pub model: String,
    /// Spawn origin (the victim's origin).
    pub at: Vec3,
    /// Toss velocity (`VelocityForDamage`).
    pub velocity: Vec3,
    /// Angular velocity (600-degree random spins).
    pub avelocity: Vec3,
    /// Master-clock removal instant (10-20 s out).
    pub remove_at: f64,
}

/// One queued monster sound for the audio slice to drain: stock `sound`
/// plays immediately, but gamecode has no audio path yet, so every call
/// site records its exact channel/sample/volume/attenuation instead.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1Sound {
    /// Sound source.
    pub entity: ActorId,
    /// Stock channel (`CHAN_*`).
    pub channel: u8,
    /// Sample path.
    pub sample: String,
    /// Volume (stock 0-1).
    pub volume: f32,
    /// Stock attenuation (`ATTN_*`).
    pub attenuation: f32,
}

/// Stock `random()` over the behavior seed: Quake never seeds the C
/// library, so every run draws the same sequence; the LCG below does the
/// same per map load (the seed advances on every draw).
pub fn q1_monster_random(behaviors: &mut Q1NativeBehaviors) -> f32 {
    // `rand()`-shaped LCG (`glibc` multiplier): the low 32 bits feed the
    // unit float, matching `random()`'s 0-inclusive range.
    behaviors.monster_rand = behaviors.monster_rand.wrapping_mul(1_103_515_245).wrapping_add(12_345);
    let bits = (behaviors.monster_rand >> 16) as u32;
    (bits as f32) / (u32::MAX as f32)
}

/// Stock `crandom()`: centered `-1..1` from two `random()` draws.
pub fn q1_monster_crandom(behaviors: &mut Q1NativeBehaviors) -> f32 {
    2.0 * (q1_monster_random(behaviors) - 0.5)
}

/// Whether the classname has a native monster spawn (other `monster_*`
/// records keep the generic path until their slice lands).
#[must_use]
pub fn q1_is_monster(classname: &str) -> bool {
    Q1MonsterKind::from_classname(classname).is_some()
}

/// Whether the classname is a native monster patrol corner.
#[must_use]
pub fn q1_is_movetarget(classname: &str) -> bool {
    classname == "path_corner"
}

/// Register the native monster and patrol-corner spawn functions. Dogs
/// spawn bodies at the map origin for [`build_q1_monster`] to size;
/// corners spawn bodiless points for [`build_q1_movetarget`] to volume.
pub fn register_q1_monster_spawns(registry: &mut SpawnRegistry) {
    for classname in ["monster_dog", "path_corner"] {
        let definition = format!("q1:{classname}");
        registry.register(
            classname,
            Box::new(move |fields| {
                Ok(SpawnRequest {
                    definition: definition.clone(),
                    origin: Some(fields.origin),
                    combat: None,
                    grants: Vec::new(),
                })
            }),
        );
    }
}

/// Dog collision bounds (`setsize`, `dog.qc` `monster_dog`).
pub const Q1_DOG_BOUNDS: Bounds = Bounds {
    min: Vec3 {
        x: -32.0,
        y: -32.0,
        z: -24.0,
    },
    max: Vec3 {
        x: 32.0,
        y: 32.0,
        z: 40.0,
    },
};

/// Dog health (`monster_dog`, `dog.qc`).
pub const Q1_DOG_HEALTH: f64 = 25.0;

/// Corner touch volume (`setsize`, `t_movetarget`, `ai.qc`).
const MOVETARGET_BOUNDS: Bounds = Bounds {
    min: Vec3 {
        x: -8.0,
        y: -8.0,
        z: -8.0,
    },
    max: Vec3 { x: 8.0, y: 8.0, z: 8.0 },
};

/// Finish a spawned monster actor: size its body, grant pre-start
/// combat (stock `takedamage` arms in `walkmonster_start_go`, so the
/// dog is immune until the pass runs it), mark it solid, record its
/// gamecode state, and count it (`walkmonster_start`, `monsters.qc:122`).
/// Dogs never spawn in deathmatch (`monster_dog`, `dog.qc`).
pub fn build_q1_monster<L: ServerLogic>(
    server: &mut Server<L>,
    behaviors: &mut Q1NativeBehaviors,
    actor: &OwnedActor,
    fields: &SpawnFields,
) -> Result<(), WorldError> {
    let Some(kind) = Q1MonsterKind::from_classname(fields.classname.as_str()) else {
        return Err(WorldError::BadSpawnFields(format!(
            "not a native monster: {}",
            fields.classname
        )));
    };
    if behaviors.deathmatch {
        return Err(WorldError::BadSpawnFields(format!(
            "{} removed in deathmatch",
            fields.classname
        )));
    }
    let now = server.simulation().frame().time.as_seconds_f64();
    server.simulation_mut().set_body_bounds(actor.id(), Q1_DOG_BOUNDS)?;
    server.simulation_mut().set_combat(
        actor.id(),
        CombatState {
            health: Q1_DOG_HEALTH,
            can_take_damage: false,
            ..CombatState::default()
        },
    )?;
    behaviors.solids.insert(actor.id());
    // `walkmonster_start`: delay the floor drop past door spawns and
    // spread think times so monsters never share a think instant.
    let nextthink = now + f64::from(q1_monster_random(behaviors)) * 0.5;
    behaviors.monsters.insert(
        actor.id(),
        Q1Monster {
            kind,
            think: Q1MonsterThink::StartGo,
            nextthink,
            frame: 0,
            enemy: None,
            oldenemy: None,
            goalentity: None,
            movetarget: None,
            ideal_yaw: 0.0,
            yaw_speed: 0.0,
            view_ofs: vec3(0.0, 0.0, 0.0),
            pausetime: 0.0,
            attack_finished: 0.0,
            pain_finished: 0.0,
            search_time: 0.0,
            show_hostile: 0.0,
            attack_state: Q1_AS_STRAIGHT,
            lefty: false,
            cnt: 0,
            flags: 0,
            spawnflags: fields.spawnflags,
            takedamage: Q1_DAMAGE_NO,
            touch: Q1MonsterTouch::None,
            source: Q1UseSource::from_fields(fields),
            dead: false,
        },
    );
    behaviors.total_monsters += 1;
    Ok(())
}

/// Finish a spawned `path_corner` actor: size its touch volume, mark it
/// a trigger, and record its onward link (`path_corner`, `ai.qc:87`).
pub fn build_q1_movetarget<L: ServerLogic>(
    server: &mut Server<L>,
    behaviors: &mut Q1NativeBehaviors,
    actor: &OwnedActor,
    fields: &SpawnFields,
) -> Result<(), WorldError> {
    server.simulation_mut().set_body_bounds(actor.id(), MOVETARGET_BOUNDS)?;
    server.mark_trigger(actor.id())?;
    behaviors.movetargets.insert(
        actor.id(),
        Q1MoveTarget {
            target: fields.target.clone(),
        },
    );
    Ok(())
}

/// Queue one monster sound with its exact stock call-site parameters.
pub fn q1_monster_sound(
    behaviors: &mut Q1NativeBehaviors,
    entity: &ActorId,
    channel: u8,
    sample: &str,
    volume: f32,
    attenuation: f32,
) {
    behaviors.sounds.push(Q1Sound {
        entity: entity.clone(),
        channel,
        sample: sample.to_string(),
        volume,
        attenuation,
    });
}

/// Stock `T_Damage` (`combat.qc:102`): the only function that ever
/// reduces health. Armor saves first (ceil), momentum shoves walking
/// victims, then the take applies: death runs [`q1_killed`], survival
/// turns monsters on their attacker and runs `th_pain`.
///
/// Quad, godmode, invulnerability, teamplay, and the client damage
/// totals have no carrier fields yet (the powerup/cheat/team slices own
/// them), so those gates are absent, not weakened: nothing is immune
/// that stock would wound.
#[allow(clippy::too_many_arguments)]
pub fn q1_t_damage(
    behaviors: &mut Q1NativeBehaviors,
    simulation: &mut Simulation,
    movers: &mut qa_world::movers::MoverTable,
    triggers: &mut qa_world::triggers::TriggerTable,
    targ: &ActorId,
    inflictor: Option<&ActorId>,
    attacker: Option<&ActorId>,
    damage: f64,
) {
    if !q1_can_take_damage(simulation, targ) {
        return;
    }
    // Buttons and triggers read the activator back off this
    // (`damage_attacker`, `combat.qc:112`).
    behaviors.damage_attacker = attacker.cloned();
    // Armor save from the worn Q1 armor, if any (`combat.qc:119`).
    let mut save = 0.0;
    if let Some(combat) = simulation.combat_state(targ).cloned() {
        if let RegularArmor::Q1 { points, absorption, .. } = &combat.armor.regular {
            save = (absorption * damage).ceil();
            let mut points = *points;
            let mut absorption = *absorption;
            if save >= points {
                save = points;
                absorption = 0.0;
                points = 0.0;
                if Some(targ) == behaviors.player.as_ref() {
                    behaviors.player_items &= !(super::native_q1_spawns::IT_ARMOR1
                        | super::native_q1_spawns::IT_ARMOR2
                        | super::native_q1_spawns::IT_ARMOR3);
                }
            } else {
                points -= save;
            }
            let _ignored = simulation.set_combat(
                targ,
                CombatState {
                    armor: qa_world::combat::ArmorState {
                        regular: RegularArmor::Q1 {
                            points,
                            absorption,
                            item: match &combat.armor.regular {
                                RegularArmor::Q1 { item, .. } => item.clone(),
                                _ => String::new(),
                            },
                        },
                        ..combat.armor
                    },
                    ..combat
                },
            );
        }
    }
    let take = (damage - save).ceil();
    // Momentum shove for walking victims (`combat.qc:141`): only the
    // admitted player walks.
    if inflictor.is_some() && Some(targ) == behaviors.player.as_ref() {
        let inflictor = inflictor.cloned().unwrap_or_else(|| targ.clone());
        let from = simulation.body_state(&inflictor).map(|body| {
            let bounds = qa_world::body::translated_body_bounds(&body);
            vec3(
                (bounds.min.x + bounds.max.x) / 2.0,
                (bounds.min.y + bounds.max.y) / 2.0,
                (bounds.min.z + bounds.max.z) / 2.0,
            )
        });
        let at = simulation.body_state(targ).map(|body| body.origin);
        if let (Some(from), Some(at)) = (from, at) {
            let mut dir = vec3(at.x - from.x, at.y - from.y, at.z - from.z);
            let len = (dir.x * dir.x + dir.y * dir.y + dir.z * dir.z).sqrt();
            if len != 0.0 {
                dir = vec3(dir.x / len, dir.y / len, dir.z / len);
                if let Some(body) = simulation.body_state(targ) {
                    let push = (damage * 8.0) as f32;
                    let _ignored = simulation.set_body_velocity(
                        targ,
                        vec3(
                            body.velocity.x + dir.x * push,
                            body.velocity.y + dir.y * push,
                            body.velocity.z + dir.z * push,
                        ),
                    );
                }
            }
        }
    }
    let health = simulation.damage_q1(targ, take);
    if health <= 0.0 {
        q1_killed(behaviors, simulation, movers, triggers, targ, attacker);
        return;
    }
    // A wounded monster turns on its attacker (`combat.qc:180`): same
    // class stays friendly, except grunts, who always feud.
    if behaviors.monsters.contains_key(targ) && attacker.is_some() {
        let attacker = attacker.cloned().unwrap_or_else(|| targ.clone());
        let mad = {
            let Some(monster) = behaviors.monsters.get(targ) else {
                return;
            };
            let same = behaviors
                .monsters
                .get(&attacker)
                .is_some_and(|other| other.kind == monster.kind);
            // The grunt exception (`monster_army` feuds even with its
            // own class, `combat.qc:187`) lands with the grunt slice;
            // every implemented kind feuds only across classes today.
            let feud = !same;
            let enemy = monster.enemy.clone();
            &attacker != targ && Some(&attacker) != enemy.as_ref() && feud
        };
        if mad {
            let player_enemy = behaviors
                .monsters
                .get(targ)
                .and_then(|monster| monster.enemy.clone())
                .is_some_and(|enemy| Some(&enemy) == behaviors.player.as_ref());
            if player_enemy {
                let old = behaviors.monsters.get(targ).and_then(|monster| monster.enemy.clone());
                if let Some(monster) = behaviors.monsters.get_mut(targ) {
                    monster.oldenemy = old;
                    monster.enemy = Some(attacker);
                }
            } else if let Some(monster) = behaviors.monsters.get_mut(targ) {
                monster.enemy = Some(attacker);
            }
            q1_found_target(behaviors, simulation, targ);
        }
    }
    // Pain: stock runs `th_pain` unconditionally, then nightmares hold
    // pain frames for 5 s (`combat.qc:198`).
    if behaviors.monsters.contains_key(targ) {
        q1_monster_th_pain(behaviors, simulation, targ);
        if behaviors.skill == 3 {
            let now = simulation.frame().time.as_seconds_f64();
            if let Some(monster) = behaviors.monsters.get_mut(targ) {
                monster.pain_finished = now + 5.0;
            }
        }
    }
}

/// Stock `Killed` (`combat.qc:56`): floor health at -99, count the kill,
/// retire the victim from damage and touch, fire `monster_death_use`,
/// and run `th_die`. Shootable buttons die through `button_killed`
/// (`buttons.qc:62`); shootable doors keep stock immunity until the
/// damage-routing slice grants them combat.
///
/// The `SVC_KILLEDMONSTER` broadcast and `ClientObituary` wait for the
/// network/HUD slices; the counters they feed are live now.
pub fn q1_killed(
    behaviors: &mut Q1NativeBehaviors,
    simulation: &mut Simulation,
    movers: &mut qa_world::movers::MoverTable,
    triggers: &mut qa_world::triggers::TriggerTable,
    targ: &ActorId,
    attacker: Option<&ActorId>,
) {
    if let Some(combat) = simulation.combat_state(targ).cloned() {
        if combat.health < -99.0 {
            let _ignored = simulation.set_combat(
                targ,
                CombatState {
                    health: -99.0,
                    ..combat
                },
            );
        }
    }
    if behaviors.buttons.contains_key(targ) {
        // `button_killed`: the activator is the stored damage attacker,
        // health restores, damage retires until `button_return`.
        if let Some(button) = behaviors.buttons.get_mut(targ) {
            button.enemy = behaviors.damage_attacker.clone();
        }
        let max_health = behaviors.buttons.get(targ).map_or(0.0, |button| button.max_health);
        if let Some(combat) = simulation.combat_state(targ).cloned() {
            let _ignored = simulation.set_combat(
                targ,
                CombatState {
                    health: max_health,
                    can_take_damage: false,
                    ..combat
                },
            );
        }
        q1_button_fire(simulation, movers, targ);
        return;
    }
    let Some(monster) = behaviors.monsters.get(targ).cloned() else {
        // Players die in the player slice; nothing else takes damage.
        return;
    };
    if monster.flags & Q1_FLAG_MONSTER != 0 {
        behaviors.killed_monsters += 1;
    }
    if let Some(combat) = simulation.combat_state(targ).cloned() {
        let _ignored = simulation.set_combat(
            targ,
            CombatState {
                can_take_damage: false,
                ..combat
            },
        );
    }
    if let Some(monster) = behaviors.monsters.get_mut(targ) {
        // Stock blames the killer before firing death targets
        // (`combat.qc:73`), so they run with the killer active.
        monster.enemy = attacker.cloned();
        monster.touch = Q1MonsterTouch::None;
        monster.dead = true;
    }
    q1_monster_death_use(behaviors, simulation, movers, triggers, targ);
    q1_monster_th_die(behaviors, simulation, targ, &monster);
}

/// Stock `monster_death_use` (`monsters.qc:49`): grounded corpses lose
/// fly/swim, then the death target fires with the killer as activator.
fn q1_monster_death_use(
    behaviors: &mut Q1NativeBehaviors,
    simulation: &mut Simulation,
    movers: &mut qa_world::movers::MoverTable,
    triggers: &mut qa_world::triggers::TriggerTable,
    targ: &ActorId,
) {
    if let Some(monster) = behaviors.monsters.get_mut(targ) {
        monster.flags &= !(Q1_FLAG_FLY | Q1_FLAG_SWIM);
    }
    let source = behaviors.monsters.get(targ).map(|monster| monster.source.clone());
    let enemy = behaviors.monsters.get(targ).and_then(|monster| monster.enemy.clone());
    if let Some(source) = source {
        if source.target.as_deref().is_some_and(|target| !target.is_empty()) {
            q1_use_targets(behaviors, simulation, movers, triggers, &source, enemy.as_ref());
        }
    }
}

/// Stock `monster_use` (`monsters.qc:21`): using a monster turns it on
/// the activator (players only, never the invisible or untargetable),
/// with the reaction delayed a tick so a teleported monster's sight
/// sound still plays at the old spot.
pub fn q1_monster_use(
    behaviors: &mut Q1NativeBehaviors,
    simulation: &mut Simulation,
    actor: &ActorId,
    activator: Option<&ActorId>,
) {
    let Some(monster) = behaviors.monsters.get(actor) else {
        return;
    };
    if monster.enemy.is_some() || q1_health_of(simulation, actor) <= 0.0 {
        return;
    }
    let Some(activator) = activator else {
        return;
    };
    if Some(activator) != behaviors.player.as_ref() {
        return;
    }
    if behaviors.player_items & Q1_IT_INVISIBILITY != 0 {
        return;
    }
    if behaviors.player_flags & Q1_FLAG_NOTARGET != 0 {
        return;
    }
    let now = simulation.frame().time.as_seconds_f64();
    if let Some(monster) = behaviors.monsters.get_mut(actor) {
        monster.enemy = Some(activator.clone());
        monster.nextthink = now + Q1_MONSTER_THINK_STEP;
        monster.think = Q1MonsterThink::FoundTarget;
    }
}

/// Stock `SightSound` (`ai.qc:279`): the per-classname wake bark. Kinds
/// queue their line here as their slices land.
fn q1_sight_sound(behaviors: &mut Q1NativeBehaviors, actor: &ActorId, kind: Q1MonsterKind) {
    let sample = match kind {
        Q1MonsterKind::Dog => "dog/dsight.wav",
    };
    q1_monster_sound(behaviors, actor, Q1_CHAN_VOICE, sample, 1.0, Q1_ATTN_NORM);
}

/// Stock `HuntTarget` (`ai.qc:270`): chase the enemy from the next
/// think, holding missile attacks for a second first.
fn q1_hunt_target(behaviors: &mut Q1NativeBehaviors, simulation: &mut Simulation, actor: &ActorId) {
    let now = simulation.frame().time.as_seconds_f64();
    let enemy_at = behaviors
        .monsters
        .get(actor)
        .and_then(|monster| monster.enemy.clone())
        .and_then(|enemy| simulation.body_state(&enemy).map(|body| body.origin));
    let at = simulation.body_state(actor).map(|body| body.origin);
    let Some(monster) = behaviors.monsters.get_mut(actor) else {
        return;
    };
    monster.goalentity = monster.enemy.clone();
    monster.think = q1_th_run(monster.kind);
    if let (Some(at), Some(enemy_at)) = (at, enemy_at) {
        monster.ideal_yaw = q1_vectoyaw(vec3(enemy_at.x - at.x, enemy_at.y - at.y, enemy_at.z - at.z));
    }
    monster.nextthink = now + Q1_MONSTER_THINK_STEP;
    // `SUB_AttackFinished (1)` (`subs.qc:297`): nightmares skip the hold.
    monster.cnt = 0;
    if behaviors.skill != 3 {
        monster.attack_finished = now + 1.0;
    }
}

/// Stock `FoundTarget` (`ai.qc:321`): publish the sighting for a tick
/// (player victims wake nearby monsters), bark, and hunt.
pub fn q1_found_target(behaviors: &mut Q1NativeBehaviors, simulation: &mut Simulation, actor: &ActorId) {
    let now = simulation.frame().time.as_seconds_f64();
    let player_victim = behaviors
        .monsters
        .get(actor)
        .and_then(|monster| monster.enemy.clone())
        .is_some_and(|enemy| Some(&enemy) == behaviors.player.as_ref());
    if player_victim {
        behaviors.sight_entity = Some(actor.clone());
        behaviors.sight_entity_time = now;
    }
    if let Some(monster) = behaviors.monsters.get_mut(actor) {
        monster.show_hostile = now + 1.0;
    }
    if let Some(monster) = behaviors.monsters.get(actor).cloned() {
        q1_sight_sound(behaviors, actor, monster.kind);
    }
    q1_hunt_target(behaviors, simulation, actor);
}

/// Stock `vectoyaw`: compass yaw of a direction in degrees.
pub fn q1_vectoyaw(dir: Vec3) -> f64 {
    let yaw = f64::from(dir.y).atan2(f64::from(dir.x)) * 180.0 / std::f64::consts::PI;
    if yaw < 0.0 {
        yaw + 360.0
    } else {
        yaw
    }
}

/// Stock `th_stand` per kind (the first stand frame).
#[must_use]
pub fn q1_th_stand(kind: Q1MonsterKind) -> Q1MonsterThink {
    match kind {
        Q1MonsterKind::Dog => Q1MonsterThink::Frame(Q1MonsterSeq::DogStand, 0),
    }
}

/// Stock `th_walk` per kind (the first walk frame).
#[must_use]
pub fn q1_th_walk(kind: Q1MonsterKind) -> Q1MonsterThink {
    match kind {
        Q1MonsterKind::Dog => Q1MonsterThink::Frame(Q1MonsterSeq::DogWalk, 0),
    }
}

/// Stock `th_run` per kind (the first run frame).
#[must_use]
pub fn q1_th_run(kind: Q1MonsterKind) -> Q1MonsterThink {
    match kind {
        Q1MonsterKind::Dog => Q1MonsterThink::Frame(Q1MonsterSeq::DogRun, 0),
    }
}

/// Stock `th_melee` per kind (the first melee frame).
#[must_use]
pub fn q1_th_melee(kind: Q1MonsterKind) -> Q1MonsterThink {
    match kind {
        Q1MonsterKind::Dog => Q1MonsterThink::Frame(Q1MonsterSeq::DogAttack, 0),
    }
}

/// Stock `th_missile` per kind (the first missile frame).
#[must_use]
pub fn q1_th_missile(kind: Q1MonsterKind) -> Q1MonsterThink {
    match kind {
        Q1MonsterKind::Dog => Q1MonsterThink::Frame(Q1MonsterSeq::DogLeap, 0),
    }
}

/// Length of a `$frame` sequence in frames.
#[must_use]
pub fn q1_seq_len(seq: Q1MonsterSeq) -> u8 {
    match seq {
        Q1MonsterSeq::DogStand => 9,
        Q1MonsterSeq::DogWalk => 8,
        Q1MonsterSeq::DogRun => 12,
        Q1MonsterSeq::DogAttack => 8,
        Q1MonsterSeq::DogLeap => 9,
        Q1MonsterSeq::DogPain => 6,
        Q1MonsterSeq::DogPainB => 16,
        Q1MonsterSeq::DogDie => 9,
        Q1MonsterSeq::DogDieB => 9,
    }
}

/// Stock model frame index for a sequence position (from the `$frame`
/// order in the kind's `.qc`: dog attack 0-7, death 8-16, deathb 17-25,
/// pain 26-31, painb 32-47, run 48-59, leap 60-68, stand 69-77, walk 78-85).
#[must_use]
pub fn q1_seq_frame(seq: Q1MonsterSeq, index: u8) -> i32 {
    let base = match seq {
        Q1MonsterSeq::DogStand => 69,
        Q1MonsterSeq::DogWalk => 78,
        Q1MonsterSeq::DogRun => 48,
        Q1MonsterSeq::DogAttack => 0,
        Q1MonsterSeq::DogLeap => 60,
        Q1MonsterSeq::DogPain => 26,
        Q1MonsterSeq::DogPainB => 32,
        Q1MonsterSeq::DogDie => 8,
        Q1MonsterSeq::DogDieB => 17,
    };
    base + i32::from(index)
}

/// Next think after a sequence position: stock `$frame` next-pointers
/// (`dog.qc`).
#[must_use]
pub fn q1_seq_next(kind: Q1MonsterKind, seq: Q1MonsterSeq, index: u8) -> Q1MonsterThink {
    let len = q1_seq_len(seq);
    if index + 1 < len {
        return Q1MonsterThink::Frame(seq, index + 1);
    }
    match (kind, seq) {
        (_, Q1MonsterSeq::DogStand) => Q1MonsterThink::Frame(Q1MonsterSeq::DogStand, 0),
        (_, Q1MonsterSeq::DogWalk) => Q1MonsterThink::Frame(Q1MonsterSeq::DogWalk, 0),
        (_, Q1MonsterSeq::DogRun) => Q1MonsterThink::Frame(Q1MonsterSeq::DogRun, 0),
        (_, Q1MonsterSeq::DogAttack) => q1_th_run(kind),
        // Leap and death tails self-loop (`dog.qc`); the leap exits
        // through `Dog_JumpTouch`, death never does.
        (_, Q1MonsterSeq::DogLeap) => Q1MonsterThink::Frame(Q1MonsterSeq::DogLeap, index),
        (_, Q1MonsterSeq::DogPain) => q1_th_run(kind),
        (_, Q1MonsterSeq::DogPainB) => q1_th_run(kind),
        (_, Q1MonsterSeq::DogDie) => Q1MonsterThink::Frame(Q1MonsterSeq::DogDie, index),
        (_, Q1MonsterSeq::DogDieB) => Q1MonsterThink::Frame(Q1MonsterSeq::DogDieB, index),
    }
}

/// Stock `th_pain` per kind: the dog barks and takes one of the two
/// pain sequences at random (`dog_pain`, `dog.qc`).
pub fn q1_monster_th_pain(behaviors: &mut Q1NativeBehaviors, simulation: &mut Simulation, actor: &ActorId) {
    let now = simulation.frame().time.as_seconds_f64();
    let Some(monster) = behaviors.monsters.get(actor).cloned() else {
        return;
    };
    match monster.kind {
        Q1MonsterKind::Dog => {
            q1_monster_sound(behaviors, actor, Q1_CHAN_VOICE, "dog/dpain1.wav", 1.0, Q1_ATTN_NORM);
            let seq = if q1_monster_random(behaviors) > 0.5 {
                Q1MonsterSeq::DogPain
            } else {
                Q1MonsterSeq::DogPainB
            };
            if let Some(monster) = behaviors.monsters.get_mut(actor) {
                monster.frame = q1_seq_frame(seq, 0);
                monster.think = Q1MonsterThink::Frame(seq, 0);
                monster.nextthink = now + Q1_MONSTER_THINK_STEP;
            }
        }
    }
}

/// Stock `th_die` per kind: past the gib threshold the dog bursts
/// (three chunks plus the head); otherwise it drops unsolid into one
/// of the two death sequences at random (`dog_die`, `dog.qc`).
pub fn q1_monster_th_die(
    behaviors: &mut Q1NativeBehaviors,
    simulation: &mut Simulation,
    actor: &ActorId,
    monster: &Q1Monster,
) {
    let now = simulation.frame().time.as_seconds_f64();
    let health = q1_health_of(simulation, actor);
    match monster.kind {
        Q1MonsterKind::Dog => {
            if health < -35.0 {
                q1_monster_sound(behaviors, actor, Q1_CHAN_VOICE, "player/udeath.wav", 1.0, Q1_ATTN_NORM);
                for _ in 0..3 {
                    q1_throw_gib(behaviors, simulation, actor, "progs/gib3.mdl", health);
                }
                q1_throw_head(behaviors, simulation, actor, "progs/h_dog.mdl", health);
                return;
            }
            q1_monster_sound(behaviors, actor, Q1_CHAN_VOICE, "dog/ddeath.wav", 1.0, Q1_ATTN_NORM);
            behaviors.solids.remove(actor);
            let seq = if q1_monster_random(behaviors) > 0.5 {
                Q1MonsterSeq::DogDie
            } else {
                Q1MonsterSeq::DogDieB
            };
            if let Some(monster) = behaviors.monsters.get_mut(actor) {
                monster.frame = q1_seq_frame(seq, 0);
                monster.think = Q1MonsterThink::Frame(seq, 0);
                monster.nextthink = now + Q1_MONSTER_THINK_STEP;
            }
        }
    }
}

/// Stock `VelocityForDamage` (`player.qc:435`): gib toss velocity from
/// the killing damage (light hits lob, heavy hits hurl).
fn q1_velocity_for_damage(behaviors: &mut Q1NativeBehaviors, damage: f64) -> Vec3 {
    let mut velocity = vec3(
        100.0 * q1_monster_crandom(behaviors),
        100.0 * q1_monster_crandom(behaviors),
        200.0 + 100.0 * q1_monster_random(behaviors),
    );
    let level = if damage > -50.0 {
        GIB_LEVEL_LIGHT
    } else if damage > -200.0 {
        GIB_LEVEL_HEAVY
    } else {
        GIB_LEVEL_EXTREME
    };
    velocity = vec3(velocity.x * level, velocity.y * level, velocity.z * level);
    velocity
}

/// Stock `ThrowGib` (`player.qc:459`): queue one bouncing chunk at the
/// victim's origin, spinning on all axes, gone in 10-20 s.
fn q1_throw_gib(
    behaviors: &mut Q1NativeBehaviors,
    simulation: &mut Simulation,
    actor: &ActorId,
    model: &str,
    damage: f64,
) {
    let now = simulation.frame().time.as_seconds_f64();
    let Some(at) = simulation.body_state(actor).map(|body| body.origin) else {
        return;
    };
    let velocity = q1_velocity_for_damage(behaviors, damage);
    let avelocity = vec3(
        q1_monster_random(behaviors) * 600.0,
        q1_monster_random(behaviors) * 600.0,
        q1_monster_random(behaviors) * 600.0,
    );
    let lifetime = 10.0 + f64::from(q1_monster_random(behaviors)) * 10.0;
    behaviors.pending_gibs.push(Q1PendingGib {
        model: model.to_string(),
        at,
        velocity,
        avelocity,
        remove_at: now + lifetime,
    });
}

/// Stock `ThrowHead` (`player.qc:480`): the victim's own actor becomes
/// the bouncing head — unsolid, untargetable, dropped 24 units, eyed at
/// 8 — and stays down (stock never removes it).
fn q1_throw_head(
    behaviors: &mut Q1NativeBehaviors,
    simulation: &mut Simulation,
    actor: &ActorId,
    _model: &str,
    damage: f64,
) {
    let velocity = q1_velocity_for_damage(behaviors, damage);
    let avelocity = vec3(0.0, q1_monster_crandom(behaviors) * 600.0, 0.0);
    behaviors.solids.remove(actor);
    if let Some(body) = simulation.body_state(actor) {
        let _ignored = simulation.set_body_bounds(
            actor,
            Bounds {
                min: vec3(-16.0, -16.0, 0.0),
                max: vec3(16.0, 16.0, 56.0),
            },
        );
        let _ignored = simulation.set_body_origin(actor, vec3(body.origin.x, body.origin.y, body.origin.z - 24.0));
        let _ignored = simulation.set_body_velocity(actor, velocity);
    }
    if let Some(monster) = behaviors.monsters.get_mut(actor) {
        monster.flags &= !Q1_FLAG_ONGROUND;
        monster.view_ofs = vec3(0.0, 0.0, 8.0);
        monster.frame = 0;
        monster.nextthink = -1.0;
    }
    behaviors.gibs.insert(
        actor,
        Q1Gib {
            avelocity,
            remove_at: None,
        },
    );
}

#[cfg(test)]
mod tests {
    use qa_core::identity::ProviderId;
    use qa_world::body::BodyState;
    use qa_world::combat::ArmorState;

    use super::*;
    use crate::options::ApplicationOptions;
    use crate::startup::{open_server, StartupConfig};

    fn test_server() -> Server<qa_guest::server::GuestServerLogic> {
        let config = StartupConfig::from_options(&ApplicationOptions::default()).unwrap();
        open_server(&config).unwrap()
    }

    fn dog_fields(pairs: &[(&str, &str)]) -> SpawnFields {
        let mut full = vec![("classname", "monster_dog"), ("origin", "0 0 0")];
        full.extend_from_slice(pairs);
        SpawnFields::parse(&full).unwrap()
    }

    fn spawn_dog(
        server: &mut Server<qa_guest::server::GuestServerLogic>,
        behaviors: &mut Q1NativeBehaviors,
        fields: &SpawnFields,
    ) -> OwnedActor {
        register_q1_monster_spawns(server.spawns_mut());
        let actor = server.spawn_entity(fields).unwrap();
        build_q1_monster(server, behaviors, &actor, fields).unwrap();
        actor
    }

    fn spawn_player(server: &mut Server<qa_guest::server::GuestServerLogic>, origin: Vec3) -> OwnedActor {
        let player = server
            .simulation_mut()
            .spawn(
                ProviderId::new("q1", "test"),
                "q1:test_player",
                Some(BodyState {
                    origin,
                    angles: vec3(0.0, 0.0, 0.0),
                    velocity: vec3(0.0, 0.0, 0.0),
                    bounds: Bounds {
                        min: vec3(-16.0, -16.0, -24.0),
                        max: vec3(16.0, 16.0, 32.0),
                    },
                    ground: None,
                }),
                None,
                Vec::new(),
            )
            .unwrap();
        server
            .simulation_mut()
            .set_combat(player.id(), CombatState::default())
            .unwrap();
        player
    }

    /// Stock `walkmonster_start_go` arming, minus the floor drop (the
    /// pass owns movement): damageable with the monster flag set.
    fn arm_dog(
        server: &mut Server<qa_guest::server::GuestServerLogic>,
        behaviors: &mut Q1NativeBehaviors,
        dog: &ActorId,
    ) {
        let combat = server.simulation().combat_state(dog).cloned().unwrap();
        server
            .simulation_mut()
            .set_combat(
                dog,
                CombatState {
                    can_take_damage: true,
                    ..combat
                },
            )
            .unwrap();
        let monster = behaviors.monsters.get_mut(dog).unwrap();
        monster.flags |= Q1_FLAG_MONSTER;
        monster.takedamage = Q1_DAMAGE_AIM;
    }

    #[test]
    fn dog_spawn_sizes_counts_and_defers_start() {
        let mut server = test_server();
        let mut behaviors = Q1NativeBehaviors::new();
        let fields = dog_fields(&[]);
        let dog = spawn_dog(&mut server, &mut behaviors, &fields);
        let body = server.simulation().body_state(dog.id()).unwrap();
        assert_eq!(body.bounds.min, Q1_DOG_BOUNDS.min);
        assert_eq!(body.bounds.max, Q1_DOG_BOUNDS.max);
        let combat = server.simulation().combat_state(dog.id()).unwrap();
        assert_eq!(combat.health, Q1_DOG_HEALTH);
        assert!(!combat.can_take_damage);
        assert!(behaviors.solids.contains(dog.id()));
        let monster = behaviors.monsters.get(dog.id()).unwrap();
        assert_eq!(monster.kind, Q1MonsterKind::Dog);
        assert_eq!(monster.think, Q1MonsterThink::StartGo);
        assert!((0.0..0.5).contains(&monster.nextthink));
        assert_eq!(behaviors.total_monsters, 1);
        assert_eq!(behaviors.killed_monsters, 0);
    }

    #[test]
    fn dog_spawn_removed_in_deathmatch() {
        let mut server = test_server();
        let mut behaviors = Q1NativeBehaviors::new();
        behaviors.deathmatch = true;
        register_q1_monster_spawns(server.spawns_mut());
        let fields = dog_fields(&[]);
        let actor = server.spawn_entity(&fields).unwrap();
        assert!(build_q1_monster(&mut server, &mut behaviors, &actor, &fields).is_err());
    }

    #[test]
    fn damage_runs_pain_and_barks() {
        let mut server = test_server();
        let mut behaviors = Q1NativeBehaviors::new();
        let player = spawn_player(&mut server, vec3(100.0, 0.0, 0.0));
        behaviors.set_player(Some(player.id().clone()));
        let fields = dog_fields(&[]);
        let dog = spawn_dog(&mut server, &mut behaviors, &fields);
        arm_dog(&mut server, &mut behaviors, dog.id());
        let (simulation, movers, triggers) = server.simulation_movers_and_triggers_mut();
        q1_t_damage(
            &mut behaviors,
            simulation,
            movers,
            triggers,
            dog.id(),
            None,
            Some(player.id()),
            5.0,
        );
        assert_eq!(q1_health_of(server.simulation(), dog.id()), 20.0);
        let monster = behaviors.monsters.get(dog.id()).unwrap();
        assert!(matches!(
            monster.think,
            Q1MonsterThink::Frame(Q1MonsterSeq::DogPain, 0) | Q1MonsterThink::Frame(Q1MonsterSeq::DogPainB, 0)
        ));
        assert_eq!(monster.enemy.as_ref(), Some(player.id()));
        assert!(behaviors
            .sounds
            .iter()
            .any(|sound| sound.sample == "dog/dpain1.wav" && sound.channel == Q1_CHAN_VOICE));
    }

    #[test]
    fn damage_kills_counts_and_runs_death() {
        let mut server = test_server();
        let mut behaviors = Q1NativeBehaviors::new();
        let player = spawn_player(&mut server, vec3(100.0, 0.0, 0.0));
        behaviors.set_player(Some(player.id().clone()));
        let fields = dog_fields(&[]);
        let dog = spawn_dog(&mut server, &mut behaviors, &fields);
        arm_dog(&mut server, &mut behaviors, dog.id());
        let (simulation, movers, triggers) = server.simulation_movers_and_triggers_mut();
        // 30 damage leaves -5: dead, but above the -35 gib threshold.
        q1_t_damage(
            &mut behaviors,
            simulation,
            movers,
            triggers,
            dog.id(),
            None,
            Some(player.id()),
            30.0,
        );
        assert!(q1_health_of(server.simulation(), dog.id()) <= 0.0);
        assert_eq!(behaviors.killed_monsters, 1);
        let monster = behaviors.monsters.get(dog.id()).unwrap();
        assert!(monster.dead);
        assert!(matches!(
            monster.think,
            Q1MonsterThink::Frame(Q1MonsterSeq::DogDie, 0) | Q1MonsterThink::Frame(Q1MonsterSeq::DogDieB, 0)
        ));
        assert!(!behaviors.solids.contains(dog.id()));
        assert!(!q1_can_take_damage(server.simulation(), dog.id()));
        assert!(behaviors.sounds.iter().any(|sound| sound.sample == "dog/ddeath.wav"));
    }

    #[test]
    fn gib_threshold_bursts_chunks_and_head() {
        let mut server = test_server();
        let mut behaviors = Q1NativeBehaviors::new();
        let player = spawn_player(&mut server, vec3(100.0, 0.0, 0.0));
        behaviors.set_player(Some(player.id().clone()));
        let fields = dog_fields(&[]);
        let dog = spawn_dog(&mut server, &mut behaviors, &fields);
        arm_dog(&mut server, &mut behaviors, dog.id());
        let (simulation, movers, triggers) = server.simulation_movers_and_triggers_mut();
        q1_t_damage(
            &mut behaviors,
            simulation,
            movers,
            triggers,
            dog.id(),
            None,
            Some(player.id()),
            1000.0,
        );
        assert_eq!(q1_health_of(server.simulation(), dog.id()), -99.0);
        assert_eq!(behaviors.pending_gibs.len(), 3);
        assert!(behaviors.gibs.contains_key(dog.id()));
        assert_eq!(behaviors.gibs.get(dog.id()).unwrap().remove_at, None);
        assert_eq!(behaviors.monsters.get(dog.id()).unwrap().nextthink, -1.0);
        assert!(behaviors.sounds.iter().any(|sound| sound.sample == "player/udeath.wav"));
    }

    #[test]
    fn infighting_turns_dog_on_attacker() {
        let mut server = test_server();
        let mut behaviors = Q1NativeBehaviors::new();
        let player = spawn_player(&mut server, vec3(500.0, 0.0, 0.0));
        behaviors.set_player(Some(player.id().clone()));
        let fields = dog_fields(&[]);
        let first = spawn_dog(&mut server, &mut behaviors, &fields);
        let second = spawn_dog(&mut server, &mut behaviors, &fields);
        arm_dog(&mut server, &mut behaviors, first.id());
        arm_dog(&mut server, &mut behaviors, second.id());
        let (simulation, movers, triggers) = server.simulation_movers_and_triggers_mut();
        // Same-class dogs stay friendly: no feud, only pain.
        q1_t_damage(
            &mut behaviors,
            simulation,
            movers,
            triggers,
            second.id(),
            Some(first.id()),
            Some(first.id()),
            5.0,
        );
        assert_eq!(behaviors.monsters.get(second.id()).unwrap().enemy, None);
        let (simulation, movers, triggers) = server.simulation_movers_and_triggers_mut();
        // A non-monster attacker turns the dog around with a hunt.
        q1_t_damage(
            &mut behaviors,
            simulation,
            movers,
            triggers,
            second.id(),
            Some(player.id()),
            Some(player.id()),
            5.0,
        );
        let monster = behaviors.monsters.get(second.id()).unwrap();
        assert_eq!(monster.enemy.as_ref(), Some(player.id()));
        // Stock runs `th_pain` after the feud, so pain wins the think.
        assert!(matches!(
            monster.think,
            Q1MonsterThink::Frame(Q1MonsterSeq::DogPain, 0) | Q1MonsterThink::Frame(Q1MonsterSeq::DogPainB, 0)
        ));
    }

    #[test]
    fn monster_use_wakes_dog_on_player_only() {
        let mut server = test_server();
        let mut behaviors = Q1NativeBehaviors::new();
        let player = spawn_player(&mut server, vec3(100.0, 0.0, 0.0));
        behaviors.set_player(Some(player.id().clone()));
        let fields = dog_fields(&[]);
        let dog = spawn_dog(&mut server, &mut behaviors, &fields);
        arm_dog(&mut server, &mut behaviors, dog.id());
        q1_monster_use(&mut behaviors, server.simulation_mut(), dog.id(), Some(player.id()));
        let monster = behaviors.monsters.get(dog.id()).unwrap();
        assert_eq!(monster.enemy.as_ref(), Some(player.id()));
        assert_eq!(monster.think, Q1MonsterThink::FoundTarget);
    }

    #[test]
    fn monster_use_refuses_invalid_activations() {
        let mut server = test_server();
        let mut behaviors = Q1NativeBehaviors::new();
        let player = spawn_player(&mut server, vec3(100.0, 0.0, 0.0));
        behaviors.set_player(Some(player.id().clone()));
        let fields = dog_fields(&[]);
        let dog = spawn_dog(&mut server, &mut behaviors, &fields);
        arm_dog(&mut server, &mut behaviors, dog.id());
        // Invisible carriers never wake monsters.
        behaviors.player_items |= Q1_IT_INVISIBILITY;
        q1_monster_use(&mut behaviors, server.simulation_mut(), dog.id(), Some(player.id()));
        assert_eq!(behaviors.monsters.get(dog.id()).unwrap().enemy, None);
        // Neither do notarget carriers.
        behaviors.player_items &= !Q1_IT_INVISIBILITY;
        behaviors.player_flags |= Q1_FLAG_NOTARGET;
        q1_monster_use(&mut behaviors, server.simulation_mut(), dog.id(), Some(player.id()));
        assert_eq!(behaviors.monsters.get(dog.id()).unwrap().enemy, None);
        // Nor non-player activators.
        behaviors.player_flags &= !Q1_FLAG_NOTARGET;
        let other = spawn_dog(&mut server, &mut behaviors, &fields);
        q1_monster_use(&mut behaviors, server.simulation_mut(), dog.id(), Some(other.id()));
        assert_eq!(behaviors.monsters.get(dog.id()).unwrap().enemy, None);
    }

    #[test]
    fn death_use_fires_target_with_killer_active() {
        use super::super::native_q1_triggers::{Q1Trigger, Q1TriggerKind};
        let mut server = test_server();
        let mut behaviors = Q1NativeBehaviors::new();
        let player = spawn_player(&mut server, vec3(100.0, 0.0, 0.0));
        behaviors.set_player(Some(player.id().clone()));
        // Relay `t1` centerprints its message to player activators.
        let relay = server
            .simulation_mut()
            .spawn(ProviderId::new("q1", "test"), "q1:test_relay", None, None, Vec::new())
            .unwrap();
        behaviors.triggers.insert(
            relay.id(),
            Q1Trigger {
                kind: Q1TriggerKind::Relay,
                source: Q1UseSource {
                    target: None,
                    killtarget: None,
                    message: Some("down".to_string()),
                    delay: 0.0,
                },
                noise: None,
            },
        );
        behaviors
            .by_targetname
            .insert("t1".to_string(), vec![relay.id().clone()]);
        let fields = dog_fields(&[("target", "t1")]);
        let dog = spawn_dog(&mut server, &mut behaviors, &fields);
        arm_dog(&mut server, &mut behaviors, dog.id());
        let (simulation, movers, triggers) = server.simulation_movers_and_triggers_mut();
        q1_t_damage(
            &mut behaviors,
            simulation,
            movers,
            triggers,
            dog.id(),
            None,
            Some(player.id()),
            100.0,
        );
        assert_eq!(behaviors.centerprints.len(), 1);
        assert_eq!(behaviors.centerprints[0].target, *player.id());
    }

    #[test]
    fn armor_saves_before_health() {
        let mut server = test_server();
        let mut behaviors = Q1NativeBehaviors::new();
        let player = spawn_player(&mut server, vec3(100.0, 0.0, 0.0));
        behaviors.set_player(Some(player.id().clone()));
        let combat = server.simulation().combat_state(player.id()).cloned().unwrap();
        server
            .simulation_mut()
            .set_combat(
                player.id(),
                CombatState {
                    armor: ArmorState {
                        regular: RegularArmor::Q1 {
                            points: 100.0,
                            absorption: 0.3,
                            item: "q1:item_armor1".to_string(),
                        },
                        ..combat.armor
                    },
                    ..combat
                },
            )
            .unwrap();
        let (simulation, movers, triggers) = server.simulation_movers_and_triggers_mut();
        q1_t_damage(
            &mut behaviors,
            simulation,
            movers,
            triggers,
            player.id(),
            None,
            Some(player.id()),
            10.0,
        );
        // save = ceil(0.3 * 10) = 3, take = ceil(10 - 3) = 7.
        assert_eq!(q1_health_of(server.simulation(), player.id()), 93.0);
        let combat = server.simulation().combat_state(player.id()).unwrap();
        match &combat.armor.regular {
            RegularArmor::Q1 { points, .. } => assert_eq!(*points, 97.0),
            _ => panic!("armor lost its Q1 shape"),
        }
    }

    #[test]
    fn dog_sequence_tables_match_stock() {
        use Q1MonsterSeq::*;
        assert_eq!(q1_seq_len(DogStand), 9);
        assert_eq!(q1_seq_len(DogWalk), 8);
        assert_eq!(q1_seq_len(DogRun), 12);
        assert_eq!(q1_seq_len(DogAttack), 8);
        assert_eq!(q1_seq_len(DogLeap), 9);
        assert_eq!(q1_seq_len(DogPain), 6);
        assert_eq!(q1_seq_len(DogPainB), 16);
        assert_eq!(q1_seq_len(DogDie), 9);
        assert_eq!(q1_seq_len(DogDieB), 9);
        assert_eq!(q1_seq_frame(DogAttack, 0), 0);
        assert_eq!(q1_seq_frame(DogStand, 0), 69);
        assert_eq!(q1_seq_frame(DogWalk, 7), 85);
        assert_eq!(
            q1_seq_next(Q1MonsterKind::Dog, DogAttack, 7),
            q1_th_run(Q1MonsterKind::Dog)
        );
        assert_eq!(
            q1_seq_next(Q1MonsterKind::Dog, DogLeap, 8),
            Q1MonsterThink::Frame(DogLeap, 8)
        );
        assert_eq!(
            q1_seq_next(Q1MonsterKind::Dog, DogDie, 8),
            Q1MonsterThink::Frame(DogDie, 8)
        );
    }
}
