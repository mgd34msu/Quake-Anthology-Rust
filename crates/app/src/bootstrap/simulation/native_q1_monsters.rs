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
//! `progs106/fight.qc` (CheckAttack, SoldierCheckAttack, ai_run_melee/missile/slide),
//! `progs106/combat.qc` (T_Damage, Killed, CanDamage),
//! `progs106/weapons.qc:203-259` (TraceAttack, FireBullets),
//! `progs106/items.qc:1229-1330,1332-1380` (BackpackTouch, DropBackpack),
//! `progs106/dog.qc` (monster_dog), `progs106/soldier.qc` (monster_army),
//! `progs106/enforcer.qc` (monster_enforcer),
//! `progs106/player.qc:435-498` (VelocityForDamage, ThrowGib, ThrowHead),
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

use qa_bots::scene::{
    PointContentsQuery as ScenePointContentsQuery, PointContentsResult as ScenePointContentsResult,
    Q1MoveRule as SceneQ1MoveRule, QueryTarget as SceneQueryTarget, TraceDetail as SceneTraceDetail,
    TraceHit as SceneTraceHit, TracePolicy as SceneTracePolicy, TraceQuery as SceneTraceQuery,
    TraceResult as SceneTraceResult, TraceShape as SceneTraceShape, VisibilityKind as SceneVisibilityKind,
};
use qa_bots::shared_scene::SharedSceneQueries;
use qa_core::identity::{ActorId, OwnedActor};
use qa_core::math::{angle_vectors, vec3, Bounds, Vec3};
use qa_core::numeric::{NumericOps, Q1_DONOR_PROFILE};
use qa_world::body::translated_body_bounds;
use qa_world::combat::{CombatState, RegularArmor};
use qa_world::movement::q1::monsters::{
    create_q1_monster_movement, Q1MonsterMoveServices, Q1MonsterMoveState, Q1MonsterMovement, Q1MonsterTarget,
};
use qa_world::movement::q1::types::{Q1Trace, Q1TraceMove, Q1TraceQuery, Q1_CONTENTS_SKY, Q1_CONTENTS_SOLID};
use qa_world::movement::types::{TraceHit, TraceShape};
use qa_world::server::{Server, ServerLogic};
use qa_world::session::Simulation;
use qa_world::spawn::{SpawnFields, SpawnRegistry, SpawnRequest};
use qa_world::triggers::TouchContact;
use qa_world::WorldError;

use super::super::play::{q1_blocked_trace, q1_trace_from_scene};
use super::native_q1_items::{build_q1_backpack, Q1Ammo, Q1ItemKind};
use super::native_q1_spawns::{q1_can_take_damage, q1_health_of, q1_remove, Q1NativeBehaviors};
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

/// Stock entity effects (`defs.qc:380-383`).
pub const Q1_EF_MUZZLEFLASH: i32 = 2;
/// Stock entity effects (`defs.qc:380-383`).
pub const Q1_EF_DIMLIGHT: i32 = 8;

/// Implemented stock monster kinds (one slice each; the roster grows here).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q1MonsterKind {
    /// `monster_dog` (`dog.qc`).
    Dog,
    /// `monster_army` (`soldier.qc`).
    Grunt,
    /// `monster_enforcer` (`enforcer.qc`).
    Enforcer,
    /// `monster_ogre` and `monster_ogre_marksman` (`ogre.qc`).
    Ogre,
}

impl Q1MonsterKind {
    /// Stock classname for the kind.
    #[must_use]
    pub fn classname(self) -> &'static str {
        match self {
            Q1MonsterKind::Dog => "monster_dog",
            Q1MonsterKind::Grunt => "monster_army",
            Q1MonsterKind::Enforcer => "monster_enforcer",
            Q1MonsterKind::Ogre => "monster_ogre",
        }
    }

    /// Parse an implemented monster classname (`None` keeps the generic
    /// spawn path until that kind's slice lands).
    #[must_use]
    pub fn from_classname(classname: &str) -> Option<Self> {
        match classname {
            "monster_dog" => Some(Q1MonsterKind::Dog),
            "monster_army" => Some(Q1MonsterKind::Grunt),
            "monster_enforcer" => Some(Q1MonsterKind::Enforcer),
            "monster_ogre" | "monster_ogre_marksman" => Some(Q1MonsterKind::Ogre),
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
    /// Grunt stand 1-8 (`soldier.qc`).
    GruntStand,
    /// Grunt walk (prowl) 1-24.
    GruntWalk,
    /// Grunt run 1-8.
    GruntRun,
    /// Grunt attack (shoot) 1-9.
    GruntAttack,
    /// Grunt pain 1-6.
    GruntPain,
    /// Grunt painb 1-14.
    GruntPainB,
    /// Grunt painc 1-13.
    GruntPainC,
    /// Grunt death 1-10.
    GruntDie,
    /// Grunt deathc 1-11.
    GruntDieC,
    /// Enforcer stand 1-7 (`enforcer.qc`).
    EnforcerStand,
    /// Enforcer walk 1-16.
    EnforcerWalk,
    /// Enforcer run 1-8.
    EnforcerRun,
    /// Enforcer attack thinks 1-14 (frames reuse attack5-8 mid-volley).
    EnforcerAttack,
    /// Enforcer paina 1-4.
    EnforcerPainA,
    /// Enforcer painb 1-5.
    EnforcerPainB,
    /// Enforcer painc 1-8.
    EnforcerPainC,
    /// Enforcer paind 1-19.
    EnforcerPainD,
    /// Enforcer death 1-14.
    EnforcerDie,
    /// Enforcer fdeath 1-11.
    EnforcerFDie,
    /// Ogre stand 1-9 (`ogre.qc`).
    OgreStand,
    /// Ogre walk 1-16.
    OgreWalk,
    /// Ogre run 1-8.
    OgreRun,
    /// Ogre swing 1-14.
    OgreSwing,
    /// Ogre smash 1-14.
    OgreSmash,
    /// Ogre nail thinks 1-7 (frames reuse shoot2 once).
    OgreNail,
    /// Ogre pain 1-5.
    OgrePain,
    /// Ogre painb 1-3.
    OgrePainB,
    /// Ogre painc 1-6.
    OgrePainC,
    /// Ogre paind 1-16.
    OgrePainD,
    /// Ogre paine 1-15.
    OgrePainE,
    /// Ogre death 1-14.
    OgreDie,
    /// Ogre bdeath 1-10.
    OgreBDie,
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
    /// Entity effects bits (`EF_*`; the presentation slice drains them).
    pub effects: i32,
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
    /// Whether the chunk landed (heads read the monster flags instead).
    pub onground: bool,
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

/// Implemented stock projectile kinds (one slice each; monster slices
/// add theirs here).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q1ProjectileKind {
    /// Enforcer laser (`LaunchLaser`, `enforcer.qc`).
    Laser,
    /// Ogre grenade (`OgreFireGrenade`, `ogre.qc`).
    Grenade,
}

/// Live stock projectile gamecode state: flying damage packets with an
/// owner, a touch function, and a removal instant. Origin, angles, and
/// velocity ride the sim body.
#[derive(Debug, Clone)]
pub struct Q1Projectile {
    /// Stock projectile kind.
    pub kind: Q1ProjectileKind,
    /// Firing owner (`None` is stock `world`).
    pub owner: Option<ActorId>,
    /// Master-clock seconds at which the think fires (bolts remove,
    /// grenades explode).
    pub remove_at: f64,
    /// Entity effects bits (`EF_*`; the presentation slice drains them).
    pub effects: i32,
    /// Angular velocity in degrees per second (tumbling grenades).
    pub avelocity: Vec3,
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

/// One queued stock temp entity for the presentation slice to drain:
/// gamecode records the exact broadcast parameters instead of writing
/// the message.
#[derive(Debug, Clone, PartialEq)]
pub enum Q1TempEnt {
    /// `particle` blood from `TraceAttack` (`weapons.qc:203`): puff at
    /// `org` drifting along `vel` (stock `vel * 0.02`), color 73, `count`
    /// particles (`damage * 2`).
    Blood {
        /// Puff origin (`trace_endpos - dir * 4`).
        org: Vec3,
        /// Drift velocity.
        vel: Vec3,
        /// Particle count.
        count: f32,
    },
    /// `TE_GUNSHOT` from `TraceAttack` (`weapons.qc:203`): a wall puff.
    Gunshot {
        /// Puff origin (`trace_endpos - dir * 4`).
        org: Vec3,
    },
    /// `TE_EXPLOSION` from `OgreGrenadeExplode` (`ogre.qc:58`): a
    /// grenade blast. Stock also keeps the bolt alive as a cycling
    /// `s_explod` sprite for 0.6 s; the broadcast carries the visual,
    /// so the bolt removes at once.
    Explosion {
        /// Blast origin (the grenade origin).
        org: Vec3,
    },
}

/// Stock `random()` over the behavior seed: Quake never seeds the C
/// library, so every run draws the same sequence; the LCG below does the
/// same per map load (the seed advances on every draw).
pub fn q1_monster_random(behaviors: &mut Q1NativeBehaviors) -> f32 {
    let bits = (q1_monster_rand_next(behaviors) >> 16) as u32;
    (bits as f32) / (u32::MAX as f32)
}

/// Advance the gamecode random seed one `rand()`-shaped LCG step
/// (`glibc` multiplier).
fn q1_monster_rand_next(behaviors: &mut Q1NativeBehaviors) -> u64 {
    behaviors.monster_rand = behaviors.monster_rand.wrapping_mul(1_103_515_245).wrapping_add(12_345);
    behaviors.monster_rand
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
    // Internal gib-chunk spawn (stock `ThrowGib` actor).
    registry.register(
        "q1:gib",
        Box::new(|fields| {
            Ok(SpawnRequest {
                definition: "q1:gib".to_string(),
                origin: Some(fields.origin),
                combat: None,
                grants: Vec::new(),
            })
        }),
    );
    // Internal laser-bolt spawn (stock `LaunchLaser` actor).
    registry.register(
        "q1:laser",
        Box::new(|fields| {
            Ok(SpawnRequest {
                definition: "q1:laser".to_string(),
                origin: Some(fields.origin),
                combat: None,
                grants: Vec::new(),
            })
        }),
    );
    // Internal grenade spawn (stock `OgreFireGrenade` actor).
    registry.register(
        "q1:grenade",
        Box::new(|fields| {
            Ok(SpawnRequest {
                definition: "q1:grenade".to_string(),
                origin: Some(fields.origin),
                combat: None,
                grants: Vec::new(),
            })
        }),
    );
    // The marksman variant shares the ogre body and controller
    // (`monster_ogre_marksman`, `ogre.qc:455`).
    registry.register(
        "monster_ogre_marksman",
        Box::new(|fields| {
            Ok(SpawnRequest {
                definition: "q1:monster_ogre".to_string(),
                origin: Some(fields.origin),
                combat: None,
                grants: Vec::new(),
            })
        }),
    );
    for classname in [
        "monster_dog",
        "monster_army",
        "monster_enforcer",
        "monster_ogre",
        "path_corner",
    ] {
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

/// Grunt collision bounds (`setsize`, `monster_army`, `soldier.qc`).
pub const Q1_GRUNT_BOUNDS: Bounds = Bounds {
    min: Vec3 {
        x: -16.0,
        y: -16.0,
        z: -24.0,
    },
    max: Vec3 {
        x: 16.0,
        y: 16.0,
        z: 40.0,
    },
};

/// Grunt health (`monster_army`, `soldier.qc`).
pub const Q1_GRUNT_HEALTH: f64 = 30.0;

/// Shells a dying grunt drops (`army_die3`, `soldier.qc`).
pub const Q1_GRUNT_DROP_SHELLS: f64 = 5.0;

/// Enforcer collision bounds (`setsize`, `monster_enforcer`, `enforcer.qc`).
pub const Q1_ENFORCER_BOUNDS: Bounds = Bounds {
    min: Vec3 {
        x: -16.0,
        y: -16.0,
        z: -24.0,
    },
    max: Vec3 {
        x: 16.0,
        y: 16.0,
        z: 40.0,
    },
};

/// Enforcer health (`monster_enforcer`, `enforcer.qc`).
pub const Q1_ENFORCER_HEALTH: f64 = 80.0;

/// Cells a dying enforcer drops (`enf_die3`, `enforcer.qc`).
pub const Q1_ENFORCER_DROP_CELLS: f64 = 5.0;

/// Laser-bolt speed in units/s (`LaunchLaser`, `enforcer.qc`).
pub const Q1_LASER_SPEED: f32 = 600.0;

/// Laser-bolt lifetime in seconds (`LaunchLaser`, `enforcer.qc`).
pub const Q1_LASER_LIFETIME: f64 = 5.0;

/// Laser-bolt strike damage (`Laser_Touch`, `enforcer.qc`).
pub const Q1_LASER_DAMAGE: f64 = 15.0;

/// Enforcer attack model frames by think index: the volley reuses
/// attack5-8 mid-sequence (`enf_atk9..12`, `enforcer.qc`).
const ENFORCER_ATTACK_FRAMES: [i32; 14] = [31, 32, 33, 34, 35, 36, 37, 38, 35, 36, 37, 38, 39, 40];

/// Ogre collision bounds (`VEC_HULL2_MIN/MAX`, `monster_ogre`, `ogre.qc`).
pub const Q1_OGRE_BOUNDS: Bounds = Bounds {
    min: Vec3 {
        x: -32.0,
        y: -32.0,
        z: -24.0,
    },
    max: Vec3 {
        x: 32.0,
        y: 32.0,
        z: 64.0,
    },
};

/// Ogre health (`monster_ogre`, `ogre.qc`).
pub const Q1_OGRE_HEALTH: f64 = 200.0;

/// Rockets a dying ogre drops (`ogre_die3`, `ogre.qc`).
pub const Q1_OGRE_DROP_ROCKETS: f64 = 2.0;

/// Ogre nail-think model frames by think index: shoot2 repeats
/// (`ogre_nail2..3`, `ogre.qc`).
const OGRE_NAIL_FRAMES: [i32; 7] = [61, 62, 62, 63, 64, 65, 66];

/// Grenade lob speed in units/s (`OgreFireGrenade`, `ogre.qc`).
pub const Q1_GRENADE_SPEED: f32 = 600.0;

/// Grenade lob upward velocity (`OgreFireGrenade`, `ogre.qc`).
pub const Q1_GRENADE_UP: f32 = 200.0;

/// Grenade fuse in seconds (`OgreFireGrenade`, `ogre.qc`).
pub const Q1_GRENADE_FUSE: f64 = 2.5;

/// Grenade blast damage (`OgreGrenadeExplode`, `ogre.qc`).
pub const Q1_GRENADE_DAMAGE: f64 = 40.0;

/// Chainsaw strike reach in units (`chainsaw`, `ogre.qc`).
pub const Q1_CHAINSAW_RANGE: f32 = 100.0;

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
/// monster is immune until the pass runs it), mark it solid, record its
/// gamecode state, and count it (`walkmonster_start`, `monsters.qc:122`).
/// No monster spawns in deathmatch (every `monster_*` spawn checks).
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
    let (bounds, health) = match kind {
        Q1MonsterKind::Dog => (Q1_DOG_BOUNDS, Q1_DOG_HEALTH),
        Q1MonsterKind::Grunt => (Q1_GRUNT_BOUNDS, Q1_GRUNT_HEALTH),
        Q1MonsterKind::Enforcer => (Q1_ENFORCER_BOUNDS, Q1_ENFORCER_HEALTH),
        Q1MonsterKind::Ogre => (Q1_OGRE_BOUNDS, Q1_OGRE_HEALTH),
    };
    let now = server.simulation().frame().time.as_seconds_f64();
    server.simulation_mut().set_body_bounds(actor.id(), bounds)?;
    server.simulation_mut().set_combat(
        actor.id(),
        CombatState {
            health,
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
            effects: 0,
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
            // Grunts feud even with their own class (`combat.qc:187`).
            let feud = !same || monster.kind == Q1MonsterKind::Grunt;
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
    // The enforcer barks one of four sight lines at random (`ai.qc:303`).
    let sample = match kind {
        Q1MonsterKind::Dog => "dog/dsight.wav",
        Q1MonsterKind::Grunt => "soldier/sight1.wav",
        Q1MonsterKind::Ogre => "ogre/ogwake.wav",
        Q1MonsterKind::Enforcer => {
            let rsnd = (q1_monster_random(behaviors) * 3.0 + 0.5).floor() as i32;
            if rsnd == 1 {
                "enforcer/sight1.wav"
            } else if rsnd == 2 {
                "enforcer/sight2.wav"
            } else if rsnd == 0 {
                "enforcer/sight3.wav"
            } else {
                "enforcer/sight4.wav"
            }
        }
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
        Q1MonsterKind::Grunt => Q1MonsterThink::Frame(Q1MonsterSeq::GruntStand, 0),
        Q1MonsterKind::Enforcer => Q1MonsterThink::Frame(Q1MonsterSeq::EnforcerStand, 0),
        Q1MonsterKind::Ogre => Q1MonsterThink::Frame(Q1MonsterSeq::OgreStand, 0),
    }
}

/// Stock `th_walk` per kind (the first walk frame).
#[must_use]
pub fn q1_th_walk(kind: Q1MonsterKind) -> Q1MonsterThink {
    match kind {
        Q1MonsterKind::Dog => Q1MonsterThink::Frame(Q1MonsterSeq::DogWalk, 0),
        Q1MonsterKind::Grunt => Q1MonsterThink::Frame(Q1MonsterSeq::GruntWalk, 0),
        Q1MonsterKind::Enforcer => Q1MonsterThink::Frame(Q1MonsterSeq::EnforcerWalk, 0),
        Q1MonsterKind::Ogre => Q1MonsterThink::Frame(Q1MonsterSeq::OgreWalk, 0),
    }
}

/// Stock `th_run` per kind (the first run frame).
#[must_use]
pub fn q1_th_run(kind: Q1MonsterKind) -> Q1MonsterThink {
    match kind {
        Q1MonsterKind::Dog => Q1MonsterThink::Frame(Q1MonsterSeq::DogRun, 0),
        Q1MonsterKind::Grunt => Q1MonsterThink::Frame(Q1MonsterSeq::GruntRun, 0),
        Q1MonsterKind::Enforcer => Q1MonsterThink::Frame(Q1MonsterSeq::EnforcerRun, 0),
        Q1MonsterKind::Ogre => Q1MonsterThink::Frame(Q1MonsterSeq::OgreRun, 0),
    }
}

/// Stock `th_melee` per kind (the first melee frame; `None` is stock
/// `SUB_Null`). The ogre picks its stroke at random (`ogre_melee`,
/// `ogre.qc:405`), so the seed rides along.
pub fn q1_th_melee(behaviors: &mut Q1NativeBehaviors, kind: Q1MonsterKind) -> Option<Q1MonsterThink> {
    match kind {
        Q1MonsterKind::Dog => Some(Q1MonsterThink::Frame(Q1MonsterSeq::DogAttack, 0)),
        Q1MonsterKind::Grunt => None,
        Q1MonsterKind::Enforcer => None,
        Q1MonsterKind::Ogre => {
            if q1_monster_random(behaviors) > 0.5 {
                Some(Q1MonsterThink::Frame(Q1MonsterSeq::OgreSmash, 0))
            } else {
                Some(Q1MonsterThink::Frame(Q1MonsterSeq::OgreSwing, 0))
            }
        }
    }
}

/// Stock `th_missile` per kind (the first missile frame; `None` is stock
/// `SUB_Null`).
#[must_use]
pub fn q1_th_missile(kind: Q1MonsterKind) -> Option<Q1MonsterThink> {
    match kind {
        Q1MonsterKind::Dog => Some(Q1MonsterThink::Frame(Q1MonsterSeq::DogLeap, 0)),
        Q1MonsterKind::Grunt => Some(Q1MonsterThink::Frame(Q1MonsterSeq::GruntAttack, 0)),
        Q1MonsterKind::Enforcer => Some(Q1MonsterThink::Frame(Q1MonsterSeq::EnforcerAttack, 0)),
        Q1MonsterKind::Ogre => Some(Q1MonsterThink::Frame(Q1MonsterSeq::OgreNail, 0)),
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
        Q1MonsterSeq::GruntStand => 8,
        Q1MonsterSeq::GruntWalk => 24,
        Q1MonsterSeq::GruntRun => 8,
        Q1MonsterSeq::GruntAttack => 9,
        Q1MonsterSeq::GruntPain => 6,
        Q1MonsterSeq::GruntPainB => 14,
        Q1MonsterSeq::GruntPainC => 13,
        Q1MonsterSeq::GruntDie => 10,
        Q1MonsterSeq::GruntDieC => 11,
        Q1MonsterSeq::EnforcerStand => 7,
        Q1MonsterSeq::EnforcerWalk => 16,
        Q1MonsterSeq::EnforcerRun => 8,
        Q1MonsterSeq::EnforcerAttack => 14,
        Q1MonsterSeq::EnforcerPainA => 4,
        Q1MonsterSeq::EnforcerPainB => 5,
        Q1MonsterSeq::EnforcerPainC => 8,
        Q1MonsterSeq::EnforcerPainD => 19,
        Q1MonsterSeq::EnforcerDie => 14,
        Q1MonsterSeq::EnforcerFDie => 11,
        Q1MonsterSeq::OgreStand => 9,
        Q1MonsterSeq::OgreWalk => 16,
        Q1MonsterSeq::OgreRun => 8,
        Q1MonsterSeq::OgreSwing => 14,
        Q1MonsterSeq::OgreSmash => 14,
        Q1MonsterSeq::OgreNail => 7,
        Q1MonsterSeq::OgrePain => 5,
        Q1MonsterSeq::OgrePainB => 3,
        Q1MonsterSeq::OgrePainC => 6,
        Q1MonsterSeq::OgrePainD => 16,
        Q1MonsterSeq::OgrePainE => 15,
        Q1MonsterSeq::OgreDie => 14,
        Q1MonsterSeq::OgreBDie => 10,
    }
}

/// Stock model frame index for a sequence position (from the `$frame`
/// order in the kind's `.qc`: dog attack 0-7, death 8-16, deathb 17-25,
/// pain 26-31, painb 32-47, run 48-59, leap 60-68, stand 69-77, walk
/// 78-85; grunt stand 0-7, death 8-17, deathc 18-28, load 29-39, pain
/// 40-45, painb 46-59, painc 60-72, run 73-80, shoot 81-89, prowl
/// 90-113; enforcer stand 0-6, walk 7-22, run 23-30, attack 31-40,
/// death 41-54, fdeath 55-65, paina 66-69, painb 70-74, painc 75-82,
/// paind 83-101; ogre stand 0-8, walk 9-24, run 25-32, swing 33-46,
/// smash 47-60, shoot 61-66, pain 67-71, painb 72-74, painc 75-80,
/// paind 81-96, paine 97-111, death 112-125, bdeath 126-135).
#[must_use]
pub fn q1_seq_frame(seq: Q1MonsterSeq, index: u8) -> i32 {
    // The enforcer volley reuses attack5-8 mid-sequence (`enf_atk9..12`,
    // `enforcer.qc`), so its frames ride a table, not a base.
    if seq == Q1MonsterSeq::EnforcerAttack {
        return ENFORCER_ATTACK_FRAMES.get(usize::from(index)).copied().unwrap_or(40);
    }
    // The nail volley repeats shoot2 (`ogre_nail2..3`, `ogre.qc`), so
    // its frames ride a table too.
    if seq == Q1MonsterSeq::OgreNail {
        return OGRE_NAIL_FRAMES.get(usize::from(index)).copied().unwrap_or(66);
    }
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
        Q1MonsterSeq::GruntStand => 0,
        Q1MonsterSeq::GruntWalk => 90,
        Q1MonsterSeq::GruntRun => 73,
        Q1MonsterSeq::GruntAttack => 81,
        Q1MonsterSeq::GruntPain => 40,
        Q1MonsterSeq::GruntPainB => 46,
        Q1MonsterSeq::GruntPainC => 60,
        Q1MonsterSeq::GruntDie => 8,
        Q1MonsterSeq::GruntDieC => 18,
        Q1MonsterSeq::EnforcerStand => 0,
        Q1MonsterSeq::EnforcerWalk => 7,
        Q1MonsterSeq::EnforcerRun => 23,
        Q1MonsterSeq::EnforcerAttack => 31,
        Q1MonsterSeq::EnforcerPainA => 66,
        Q1MonsterSeq::EnforcerPainB => 70,
        Q1MonsterSeq::EnforcerPainC => 75,
        Q1MonsterSeq::EnforcerPainD => 83,
        Q1MonsterSeq::EnforcerDie => 41,
        Q1MonsterSeq::EnforcerFDie => 55,
        Q1MonsterSeq::OgreStand => 0,
        Q1MonsterSeq::OgreWalk => 9,
        Q1MonsterSeq::OgreRun => 25,
        Q1MonsterSeq::OgreSwing => 33,
        Q1MonsterSeq::OgreSmash => 47,
        Q1MonsterSeq::OgreNail => 61,
        Q1MonsterSeq::OgrePain => 67,
        Q1MonsterSeq::OgrePainB => 72,
        Q1MonsterSeq::OgrePainC => 75,
        Q1MonsterSeq::OgrePainD => 81,
        Q1MonsterSeq::OgrePainE => 97,
        Q1MonsterSeq::OgreDie => 112,
        Q1MonsterSeq::OgreBDie => 126,
    };
    base + i32::from(index)
}

/// Next think after a sequence position: stock `$frame` next-pointers
/// (the kind's `.qc`).
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
        (_, Q1MonsterSeq::GruntStand) => Q1MonsterThink::Frame(Q1MonsterSeq::GruntStand, 0),
        (_, Q1MonsterSeq::GruntWalk) => Q1MonsterThink::Frame(Q1MonsterSeq::GruntWalk, 0),
        (_, Q1MonsterSeq::GruntRun) => Q1MonsterThink::Frame(Q1MonsterSeq::GruntRun, 0),
        (_, Q1MonsterSeq::GruntAttack) => q1_th_run(kind),
        (_, Q1MonsterSeq::GruntPain) => q1_th_run(kind),
        (_, Q1MonsterSeq::GruntPainB) => q1_th_run(kind),
        (_, Q1MonsterSeq::GruntPainC) => q1_th_run(kind),
        // Death tails self-loop (`soldier.qc`); death never exits.
        (_, Q1MonsterSeq::GruntDie) => Q1MonsterThink::Frame(Q1MonsterSeq::GruntDie, index),
        (_, Q1MonsterSeq::GruntDieC) => Q1MonsterThink::Frame(Q1MonsterSeq::GruntDieC, index),
        (_, Q1MonsterSeq::EnforcerStand) => Q1MonsterThink::Frame(Q1MonsterSeq::EnforcerStand, 0),
        (_, Q1MonsterSeq::EnforcerWalk) => Q1MonsterThink::Frame(Q1MonsterSeq::EnforcerWalk, 0),
        (_, Q1MonsterSeq::EnforcerRun) => Q1MonsterThink::Frame(Q1MonsterSeq::EnforcerRun, 0),
        (_, Q1MonsterSeq::EnforcerAttack) => q1_th_run(kind),
        (_, Q1MonsterSeq::EnforcerPainA) => q1_th_run(kind),
        (_, Q1MonsterSeq::EnforcerPainB) => q1_th_run(kind),
        (_, Q1MonsterSeq::EnforcerPainC) => q1_th_run(kind),
        (_, Q1MonsterSeq::EnforcerPainD) => q1_th_run(kind),
        // Death tails self-loop (`enforcer.qc`); death never exits.
        (_, Q1MonsterSeq::EnforcerDie) => Q1MonsterThink::Frame(Q1MonsterSeq::EnforcerDie, index),
        (_, Q1MonsterSeq::EnforcerFDie) => Q1MonsterThink::Frame(Q1MonsterSeq::EnforcerFDie, index),
        (_, Q1MonsterSeq::OgreStand) => Q1MonsterThink::Frame(Q1MonsterSeq::OgreStand, 0),
        (_, Q1MonsterSeq::OgreWalk) => Q1MonsterThink::Frame(Q1MonsterSeq::OgreWalk, 0),
        (_, Q1MonsterSeq::OgreRun) => Q1MonsterThink::Frame(Q1MonsterSeq::OgreRun, 0),
        (_, Q1MonsterSeq::OgreSwing) => q1_th_run(kind),
        (_, Q1MonsterSeq::OgreSmash) => q1_th_run(kind),
        (_, Q1MonsterSeq::OgreNail) => q1_th_run(kind),
        (_, Q1MonsterSeq::OgrePain) => q1_th_run(kind),
        (_, Q1MonsterSeq::OgrePainB) => q1_th_run(kind),
        (_, Q1MonsterSeq::OgrePainC) => q1_th_run(kind),
        (_, Q1MonsterSeq::OgrePainD) => q1_th_run(kind),
        (_, Q1MonsterSeq::OgrePainE) => q1_th_run(kind),
        // Death tails self-loop (`ogre.qc`); death never exits.
        (_, Q1MonsterSeq::OgreDie) => Q1MonsterThink::Frame(Q1MonsterSeq::OgreDie, index),
        (_, Q1MonsterSeq::OgreBDie) => Q1MonsterThink::Frame(Q1MonsterSeq::OgreBDie, index),
    }
}

/// Stock `th_pain` per kind: the dog barks and takes one of the two
/// pain sequences at random (`dog_pain`, `dog.qc`); the grunt holds
/// pain while `pain_finished` runs, then takes one of three sequences
/// at random (`army_pain`, `soldier.qc`).
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
        Q1MonsterKind::Grunt => {
            if monster.pain_finished > now {
                return;
            }
            let roll = q1_monster_random(behaviors);
            let (seq, hold, sample) = if roll < 0.2 {
                (Q1MonsterSeq::GruntPain, 0.6, "soldier/pain1.wav")
            } else if roll < 0.6 {
                (Q1MonsterSeq::GruntPainB, 1.1, "soldier/pain2.wav")
            } else {
                (Q1MonsterSeq::GruntPainC, 1.1, "soldier/pain2.wav")
            };
            q1_monster_sound(behaviors, actor, Q1_CHAN_VOICE, sample, 1.0, Q1_ATTN_NORM);
            if let Some(monster) = behaviors.monsters.get_mut(actor) {
                monster.frame = q1_seq_frame(seq, 0);
                monster.think = Q1MonsterThink::Frame(seq, 0);
                monster.nextthink = now + Q1_MONSTER_THINK_STEP;
                monster.pain_finished = now + hold;
            }
        }
        Q1MonsterKind::Enforcer => {
            // Stock draws the roll before the pain gate (`enf_pain`,
            // `enforcer.qc:219`): one roll picks the bark and the sequence.
            let roll = q1_monster_random(behaviors);
            if monster.pain_finished > now {
                return;
            }
            let sample = if roll < 0.5 {
                "enforcer/pain1.wav"
            } else {
                "enforcer/pain2.wav"
            };
            let (seq, hold) = if roll < 0.2 {
                (Q1MonsterSeq::EnforcerPainA, 1.0)
            } else if roll < 0.4 {
                (Q1MonsterSeq::EnforcerPainB, 1.0)
            } else if roll < 0.7 {
                (Q1MonsterSeq::EnforcerPainC, 1.0)
            } else {
                (Q1MonsterSeq::EnforcerPainD, 2.0)
            };
            q1_monster_sound(behaviors, actor, Q1_CHAN_VOICE, sample, 1.0, Q1_ATTN_NORM);
            if let Some(monster) = behaviors.monsters.get_mut(actor) {
                monster.frame = q1_seq_frame(seq, 0);
                monster.think = Q1MonsterThink::Frame(seq, 0);
                monster.nextthink = now + Q1_MONSTER_THINK_STEP;
                monster.pain_finished = now + hold;
            }
        }
        Q1MonsterKind::Ogre => {
            if monster.pain_finished > now {
                return;
            }
            q1_monster_sound(behaviors, actor, Q1_CHAN_VOICE, "ogre/ogpain1.wav", 1.0, Q1_ATTN_NORM);
            let roll = q1_monster_random(behaviors);
            let (seq, hold) = if roll < 0.25 {
                (Q1MonsterSeq::OgrePain, 1.0)
            } else if roll < 0.5 {
                (Q1MonsterSeq::OgrePainB, 1.0)
            } else if roll < 0.75 {
                (Q1MonsterSeq::OgrePainC, 1.0)
            } else if roll < 0.88 {
                (Q1MonsterSeq::OgrePainD, 2.0)
            } else {
                (Q1MonsterSeq::OgrePainE, 2.0)
            };
            if let Some(monster) = behaviors.monsters.get_mut(actor) {
                monster.frame = q1_seq_frame(seq, 0);
                monster.think = Q1MonsterThink::Frame(seq, 0);
                monster.nextthink = now + Q1_MONSTER_THINK_STEP;
                monster.pain_finished = now + hold;
            }
        }
    }
}

/// Stock `th_die` per kind: past the gib threshold the victim bursts
/// (chunks plus the head); otherwise the dog drops unsolid into one of
/// the two death sequences at random (`dog_die`, `dog.qc`), while the
/// grunt stays solid into one of its two death sequences — solidity
/// drops in the third death frame (`army_die`, `soldier.qc`).
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
        Q1MonsterKind::Grunt => {
            if health < -35.0 {
                q1_monster_sound(behaviors, actor, Q1_CHAN_VOICE, "player/udeath.wav", 1.0, Q1_ATTN_NORM);
                q1_throw_head(behaviors, simulation, actor, "progs/h_guard.mdl", health);
                q1_throw_gib(behaviors, simulation, actor, "progs/gib1.mdl", health);
                q1_throw_gib(behaviors, simulation, actor, "progs/gib2.mdl", health);
                q1_throw_gib(behaviors, simulation, actor, "progs/gib3.mdl", health);
                return;
            }
            q1_monster_sound(behaviors, actor, Q1_CHAN_VOICE, "soldier/death1.wav", 1.0, Q1_ATTN_NORM);
            let seq = if q1_monster_random(behaviors) < 0.5 {
                Q1MonsterSeq::GruntDie
            } else {
                Q1MonsterSeq::GruntDieC
            };
            if let Some(monster) = behaviors.monsters.get_mut(actor) {
                monster.frame = q1_seq_frame(seq, 0);
                monster.think = Q1MonsterThink::Frame(seq, 0);
                monster.nextthink = now + Q1_MONSTER_THINK_STEP;
            }
        }
        Q1MonsterKind::Enforcer => {
            if health < -35.0 {
                q1_monster_sound(behaviors, actor, Q1_CHAN_VOICE, "player/udeath.wav", 1.0, Q1_ATTN_NORM);
                q1_throw_head(behaviors, simulation, actor, "progs/h_mega.mdl", health);
                q1_throw_gib(behaviors, simulation, actor, "progs/gib1.mdl", health);
                q1_throw_gib(behaviors, simulation, actor, "progs/gib2.mdl", health);
                q1_throw_gib(behaviors, simulation, actor, "progs/gib3.mdl", health);
                return;
            }
            q1_monster_sound(
                behaviors,
                actor,
                Q1_CHAN_VOICE,
                "enforcer/death1.wav",
                1.0,
                Q1_ATTN_NORM,
            );
            let seq = if q1_monster_random(behaviors) > 0.5 {
                Q1MonsterSeq::EnforcerDie
            } else {
                Q1MonsterSeq::EnforcerFDie
            };
            if let Some(monster) = behaviors.monsters.get_mut(actor) {
                monster.frame = q1_seq_frame(seq, 0);
                monster.think = Q1MonsterThink::Frame(seq, 0);
                monster.nextthink = now + Q1_MONSTER_THINK_STEP;
            }
        }
        Q1MonsterKind::Ogre => {
            if health < -80.0 {
                q1_monster_sound(behaviors, actor, Q1_CHAN_VOICE, "player/udeath.wav", 1.0, Q1_ATTN_NORM);
                q1_throw_head(behaviors, simulation, actor, "progs/h_ogre.mdl", health);
                q1_throw_gib(behaviors, simulation, actor, "progs/gib3.mdl", health);
                q1_throw_gib(behaviors, simulation, actor, "progs/gib3.mdl", health);
                q1_throw_gib(behaviors, simulation, actor, "progs/gib3.mdl", health);
                return;
            }
            q1_monster_sound(behaviors, actor, Q1_CHAN_VOICE, "ogre/ogdth.wav", 1.0, Q1_ATTN_NORM);
            let seq = if q1_monster_random(behaviors) < 0.5 {
                Q1MonsterSeq::OgreDie
            } else {
                Q1MonsterSeq::OgreBDie
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
            onground: false,
        },
    );
}

/// Monster movement services over the live sim bodies, the shared
/// collision scene, and the gamecode records: origins/angles/bounds
/// ride the body, flags/ideal-yaw/enemy ride the monster record, and
/// traces run the scene with the mover passed (stock `SV_Move`
/// `passedict`).
pub struct Q1NativeMonsterServices<'a, 'b> {
    /// Q1 donor numerics.
    ops: NumericOps,
    /// Shared collision scene (relinked each pass).
    scene: &'a SharedSceneQueries,
    /// Live simulation (body reads and writes).
    simulation: &'a mut Simulation,
    /// Gamecode records and the random seed.
    behaviors: &'b mut Q1NativeBehaviors,
}

impl Q1MonsterMoveServices for Q1NativeMonsterServices<'_, '_> {
    fn numeric(&self) -> NumericOps {
        self.ops
    }

    fn trace(&mut self, actor: &ActorId, start: Vec3, end: Vec3, bounds: Option<Bounds>) -> Q1Trace {
        let shape = bounds.map_or(SceneTraceShape::Point, |bounds| SceneTraceShape::Box { bounds });
        let query = SceneTraceQuery {
            start,
            end,
            shape,
            target: SceneQueryTarget::World,
            policy: SceneTracePolicy::Q1 {
                move_rule: SceneQ1MoveRule::Normal,
                hull: None,
            },
            numeric: Q1_DONOR_PROFILE,
            pass_actor: Some(actor.clone()),
        };
        match self.scene.trace(&query) {
            Ok(trace) => q1_trace_from_scene(&trace),
            Err(_) => q1_blocked_trace(&Q1TraceQuery {
                start,
                end,
                shape: bounds.map_or(TraceShape::Point, TraceShape::Box),
                policy: Q1TraceMove::Normal,
            }),
        }
    }

    fn point_contents(&mut self, actor: &ActorId, point: Vec3) -> i32 {
        let query = ScenePointContentsQuery {
            point,
            target: SceneQueryTarget::World,
            policy: SceneTracePolicy::Q1 {
                move_rule: SceneQ1MoveRule::Normal,
                hull: None,
            },
            numeric: Q1_DONOR_PROFILE,
            pass_actor: Some(actor.clone()),
        };
        match self.scene.point_contents(&query) {
            Ok(ScenePointContentsResult::Q1 { contents }) => contents,
            _ => Q1_CONTENTS_SOLID,
        }
    }

    fn next_random(&mut self) -> i32 {
        (q1_monster_rand_next(self.behaviors) >> 16) as i32
    }

    fn read(&mut self, actor: &ActorId) -> Option<Q1MonsterMoveState> {
        let body = self.simulation.body_state(actor)?;
        let monster = self.behaviors.monsters.get(actor)?;
        Some(Q1MonsterMoveState {
            origin: body.origin,
            angles: body.angles,
            bounds: body.bounds,
            absolute_bounds: translated_body_bounds(&body),
            flags: monster.flags,
            ground: body
                .ground
                .as_ref()
                .map_or(TraceHit::None, |ground| TraceHit::Actor { actor: ground.clone() }),
            ideal_yaw: monster.ideal_yaw,
            yaw_speed: monster.yaw_speed,
            enemy: monster.enemy.clone(),
        })
    }

    fn read_target(&mut self, actor: &ActorId) -> Option<Q1MonsterTarget> {
        let body = self.simulation.body_state(actor)?;
        Some(Q1MonsterTarget {
            origin: body.origin,
            absolute_bounds: translated_body_bounds(&body),
        })
    }

    fn write(&mut self, actor: &OwnedActor, state: Q1MonsterMoveState) {
        let _ignored = self.simulation.set_body_origin(actor.id(), state.origin);
        let _ignored = self.simulation.set_body_angles(actor.id(), state.angles);
        // The body table exposes no ground setter (pusher riders read
        // `ground`, nothing writes it), so the movement ground hit has
        // no sim carrier; the flags bit below is the live grounding.
        if let Some(monster) = self.behaviors.monsters.get_mut(actor.id()) {
            monster.flags = state.flags;
            monster.ideal_yaw = state.ideal_yaw;
        }
    }

    fn link(&mut self, actor: &OwnedActor, _touch_triggers: bool) {
        // The next server tick's trigger sweep fires touches for moved
        // bodies (the same one-tick latency the player step has), so
        // the link only refreshes the body table here.
        let _ignored = self.simulation.link_body(actor.id());
    }
}

/// Per-pass monster context: the server, gamecode, scene, and clock.
pub struct Q1MonsterCtx<'s, 'b, 'c, L: ServerLogic> {
    /// Live server (spawn, bodies, movers, triggers).
    pub server: &'s mut Server<L>,
    /// Gamecode records.
    pub behaviors: &'b mut Q1NativeBehaviors,
    /// Shared collision scene.
    pub scene: &'c SharedSceneQueries,
    /// Master-clock seconds.
    pub now: f64,
    /// Last tick length in seconds (physics integration step).
    pub dt: f64,
}

impl<L: ServerLogic> Q1MonsterCtx<'_, '_, '_, L> {
    /// Fresh monster-movement builtins over the context.
    fn movement(&mut self) -> Q1MonsterMovement<Q1NativeMonsterServices<'_, '_>> {
        let ops = NumericOps::select(Q1_DONOR_PROFILE).expect("Q1 donor numeric profile");
        let simulation = self.server.simulation_mut();
        create_q1_monster_movement(Q1NativeMonsterServices {
            ops,
            scene: self.scene,
            simulation,
            behaviors: &mut *self.behaviors,
        })
    }

    /// Resolve a live owned actor, if it still exists.
    fn owned(&self, actor: &ActorId) -> Option<OwnedActor> {
        self.server.simulation().registry().resolve_owned(actor)
    }
}

/// Run one monster pass: spawn queued gibs, run due monster thinks in
/// record order, step airborne bodies, toss gibs, and settle tossing
/// backpacks. The pass runs at the PlayWorld level after the server
/// tick (the player step's slot), so moves touch on the next tick's
/// sweep.
pub fn q1_monster_pass<L: ServerLogic>(
    server: &mut Server<L>,
    behaviors: &mut Q1NativeBehaviors,
    scene: &SharedSceneQueries,
) {
    let frame = server.simulation().frame();
    let mut ctx = Q1MonsterCtx {
        server,
        behaviors,
        scene,
        now: frame.time.as_seconds_f64(),
        dt: frame.elapsed.as_seconds_f64().max(0.0),
    };
    q1_spawn_pending_gibs(&mut ctx);
    // Stock `SV_CleanupEnts` (`sv_main.c:554`): muzzle flashes live one
    // server frame, so last pass's flashes clear before thinks run.
    for monster in ctx.behaviors.monsters.values_mut() {
        monster.effects &= !Q1_EF_MUZZLEFLASH;
    }
    let actors: Vec<ActorId> = ctx.behaviors.monsters.keys().cloned().collect();
    for actor in actors {
        q1_monster_actor(&mut ctx, &actor);
    }
    let gibs: Vec<ActorId> = ctx.behaviors.gibs.keys().cloned().collect();
    for actor in gibs {
        q1_gib_actor(&mut ctx, &actor);
    }
    let bolts: Vec<ActorId> = ctx.behaviors.projectiles.keys().cloned().collect();
    for actor in bolts {
        q1_projectile_actor(&mut ctx, &actor);
    }
    let tossed: Vec<ActorId> = ctx
        .behaviors
        .items
        .iter()
        .filter(|(_, item)| !item.taken && matches!(item.kind, Q1ItemKind::Backpack { settled: false, .. }))
        .map(|(id, _)| id.clone())
        .collect();
    for actor in tossed {
        q1_backpack_actor(&mut ctx, &actor);
    }
}

/// Spawn every queued `ThrowGib` chunk: point-sized unsolid bodies with
/// toss velocity, spinning, removed on schedule.
fn q1_spawn_pending_gibs<L: ServerLogic>(ctx: &mut Q1MonsterCtx<'_, '_, '_, L>) {
    let pending = std::mem::take(&mut ctx.behaviors.pending_gibs);
    for gib in pending {
        let fields = SpawnFields {
            classname: "q1:gib".to_string(),
            origin: gib.at,
            ..SpawnFields::default()
        };
        let actor = match ctx.server.spawn_entity(&fields) {
            Ok(actor) => actor,
            Err(_) => continue,
        };
        let simulation = ctx.server.simulation_mut();
        let point = Bounds {
            min: vec3(0.0, 0.0, 0.0),
            max: vec3(0.0, 0.0, 0.0),
        };
        if simulation.set_body_bounds(actor.id(), point).is_err()
            || simulation.set_body_velocity(actor.id(), gib.velocity).is_err()
        {
            let _ignored = simulation.release(&actor);
            continue;
        }
        ctx.behaviors.gibs.insert(
            actor.id(),
            Q1Gib {
                avelocity: gib.avelocity,
                remove_at: Some(gib.remove_at),
                onground: false,
            },
        );
    }
}

/// Step one monster actor: heads belong to the gib pass; due thinks run
/// the `$frame` macro (frame plus advance first, so bodies redirect by
/// overwriting); airborne step bodies freefall after thinking.
fn q1_monster_actor<L: ServerLogic>(ctx: &mut Q1MonsterCtx<'_, '_, '_, L>, actor: &ActorId) {
    if ctx.behaviors.gibs.contains_key(actor) {
        return;
    }
    let Some(monster) = ctx.behaviors.monsters.get(actor).cloned() else {
        return;
    };
    if monster.nextthink <= ctx.now && monster.nextthink >= 0.0 {
        match monster.think {
            Q1MonsterThink::StartGo => q1_start_go(ctx, actor),
            Q1MonsterThink::FoundTarget => {
                let simulation = ctx.server.simulation_mut();
                q1_found_target(ctx.behaviors, simulation, actor);
            }
            Q1MonsterThink::Frame(seq, index) => {
                let kind = monster.kind;
                if let Some(monster) = ctx.behaviors.monsters.get_mut(actor) {
                    monster.frame = q1_seq_frame(seq, index);
                    monster.think = q1_seq_next(kind, seq, index);
                    monster.nextthink = ctx.now + Q1_MONSTER_THINK_STEP;
                }
                q1_monster_frame(ctx, actor, kind, seq, index);
            }
        }
    }
    let airborne = ctx
        .behaviors
        .monsters
        .get(actor)
        .is_some_and(|monster| monster.flags & (Q1_FLAG_ONGROUND | Q1_FLAG_FLY | Q1_FLAG_SWIM) == 0);
    if airborne {
        q1_step_physics(ctx, actor);
    }
}

/// Step one gib actor: scheduled chunks remove (`SUB_Remove`); airborne
/// chunks and heads toss.
fn q1_gib_actor<L: ServerLogic>(ctx: &mut Q1MonsterCtx<'_, '_, '_, L>, actor: &ActorId) {
    let Some(gib) = ctx.behaviors.gibs.get(actor).cloned() else {
        return;
    };
    if gib.remove_at.is_some_and(|at| at <= ctx.now) {
        let (simulation, movers, triggers) = ctx.server.simulation_movers_and_triggers_mut();
        q1_remove(ctx.behaviors, simulation, movers, triggers, actor);
        return;
    }
    // Heads ground through the monster flags; chunks ground through
    // their own record (stock `FL_ONGROUND` in both cases).
    let airborne = match ctx.behaviors.monsters.get(actor) {
        Some(monster) => monster.flags & Q1_FLAG_ONGROUND == 0,
        None => !gib.onground,
    };
    if airborne {
        q1_toss_physics(ctx, actor);
    }
}

/// Native touch dispatch for Q1 patrol corners: corners are marked
/// triggers, so travelling monsters touch them in the sweep.
pub fn q1_monster_touch(
    behaviors: &mut Q1NativeBehaviors,
    simulation: &mut Simulation,
    _movers: &mut qa_world::movers::MoverTable,
    _triggers: &mut qa_world::triggers::TriggerTable,
    contact: &TouchContact,
) {
    q1_movetarget_touch(behaviors, simulation, &contact.trigger, &contact.other);
}

/// Stock `t_movetarget` (`ai.qc:98`): a monster reaching its own corner
/// (fighting monsters ignore corners) turns toward the next corner, or
/// stands down when the route ends. Runs in the trigger sweep: corners
/// are marked triggers, so travelling monsters touch them.
pub fn q1_movetarget_touch(
    behaviors: &mut Q1NativeBehaviors,
    simulation: &mut Simulation,
    contact_trigger: &ActorId,
    contact_other: &ActorId,
) {
    let Some(corner) = behaviors.movetargets.get(contact_trigger).cloned() else {
        return;
    };
    let Some(monster) = behaviors.monsters.get(contact_other).cloned() else {
        return;
    };
    if monster.movetarget.as_ref() != Some(contact_trigger) {
        return;
    }
    if monster.enemy.is_some() {
        return;
    }
    let now = simulation.frame().time.as_seconds_f64();
    let next = corner.target.as_deref().and_then(|target| {
        behaviors
            .by_targetname
            .get(target)
            .and_then(|matches| matches.first().cloned())
    });
    let Some(monster_mut) = behaviors.monsters.get_mut(contact_other) else {
        return;
    };
    monster_mut.goalentity = next.clone();
    monster_mut.movetarget = next.clone();
    if let Some(next) = next {
        let from = simulation.body_state(contact_other).map(|body| body.origin);
        let to = simulation.body_state(&next).map(|body| body.origin);
        if let (Some(from), Some(to)) = (from, to) {
            if let Some(monster_mut) = behaviors.monsters.get_mut(contact_other) {
                monster_mut.ideal_yaw = q1_vectoyaw(vec3(to.x - from.x, to.y - from.y, to.z - from.z));
            }
        }
    } else if let Some(monster_mut) = behaviors.monsters.get_mut(contact_other) {
        monster_mut.pausetime = now + 999_999.0;
        let kind = monster.kind;
        monster_mut.think = q1_th_stand(kind);
        monster_mut.nextthink = now + Q1_MONSTER_THINK_STEP;
    }
}

/// Eye position of a sight party: monsters read their record offset,
/// the player reads the stock view height, anything else reads raw.
fn q1_eye_of<L: ServerLogic>(ctx: &Q1MonsterCtx<'_, '_, '_, L>, actor: &ActorId) -> Option<Vec3> {
    let body = ctx.server.simulation().body_state(actor)?;
    if let Some(monster) = ctx.behaviors.monsters.get(actor) {
        return Some(vec3(
            body.origin.x + monster.view_ofs.x,
            body.origin.y + monster.view_ofs.y,
            body.origin.z + monster.view_ofs.z,
        ));
    }
    if Some(actor) == ctx.behaviors.player.as_ref() {
        return Some(vec3(body.origin.x, body.origin.y, body.origin.z + Q1_VIEW_OFS_Z));
    }
    Some(body.origin)
}

/// One stock `traceline ... TRUE` (`ai.qc`, `combat.qc`): the sight line
/// skips every box-solid party (stock `MOVE_NOMONSTERS` skips all
/// non-BSP entities, viewer and target included) but still stops at the
/// world and doors.
fn q1_sight_trace<L: ServerLogic>(
    ctx: &Q1MonsterCtx<'_, '_, '_, L>,
    spot1: Vec3,
    spot2: Vec3,
) -> Option<SceneTraceResult> {
    let mut excluded: Vec<ActorId> = ctx.behaviors.monsters.keys().cloned().collect();
    if let Some(player) = ctx.behaviors.player.as_ref() {
        excluded.push(player.clone());
    }
    let query = SceneTraceQuery {
        start: spot1,
        end: spot2,
        shape: SceneTraceShape::Point,
        target: SceneQueryTarget::World,
        policy: SceneTracePolicy::Q1 {
            move_rule: SceneQ1MoveRule::NoMonsters,
            hull: None,
        },
        numeric: Q1_DONOR_PROFILE,
        pass_actor: None,
    };
    ctx.scene.trace_excluding(&query, &excluded).ok()
}

/// Whether a sight trace crossed from open space into water or back
/// (`trace_inopen && trace_inwater`, `ai.qc`).
fn q1_trace_crossed_media(trace: &SceneTraceResult) -> bool {
    match &trace.detail {
        SceneTraceDetail::Q1 { in_open, in_water, .. } => *in_open && *in_water,
        _ => false,
    }
}

/// Stock `visible` (`ai.qc`): a clear no-monsters sight line between
/// eyes that never crosses media.
fn q1_visible<L: ServerLogic>(ctx: &Q1MonsterCtx<'_, '_, '_, L>, viewer: &ActorId, targ: &ActorId) -> bool {
    let (Some(spot1), Some(spot2)) = (q1_eye_of(ctx, viewer), q1_eye_of(ctx, targ)) else {
        return false;
    };
    let Some(trace) = q1_sight_trace(ctx, spot1, spot2) else {
        return false;
    };
    if q1_trace_crossed_media(&trace) {
        return false;
    }
    trace.fraction == 1.0
}

/// Stock `CanDamage` (`combat.qc:18`): push targets trace to their
/// center (a hit on the target counts); everyone else needs one clear
/// line to the origin or a corner offset.
fn q1_can_damage<L: ServerLogic>(ctx: &mut Q1MonsterCtx<'_, '_, '_, L>, targ: &ActorId, inflictor: &ActorId) -> bool {
    let Some(from) = ctx.server.simulation().body_state(inflictor).map(|body| body.origin) else {
        return false;
    };
    if ctx.server.movers_mut().get(targ).is_some() {
        let Some(body) = ctx.server.simulation().body_state(targ) else {
            return false;
        };
        let bounds = translated_body_bounds(&body);
        let center = vec3(
            (bounds.min.x + bounds.max.x) / 2.0,
            (bounds.min.y + bounds.max.y) / 2.0,
            (bounds.min.z + bounds.max.z) / 2.0,
        );
        let Some(trace) = q1_sight_trace(ctx, from, center) else {
            return false;
        };
        return trace.fraction == 1.0 || matches!(&trace.hit, SceneTraceHit::Actor { actor } if actor == targ);
    }
    let Some(base) = ctx.server.simulation().body_state(targ).map(|body| body.origin) else {
        return false;
    };
    let offsets = [
        vec3(0.0, 0.0, 0.0),
        vec3(15.0, 15.0, 0.0),
        vec3(-15.0, -15.0, 0.0),
        vec3(-15.0, 15.0, 0.0),
        vec3(15.0, -15.0, 0.0),
    ];
    for offset in offsets {
        let to = vec3(base.x + offset.x, base.y + offset.y, base.z + offset.z);
        if let Some(trace) = q1_sight_trace(ctx, from, to) {
            if trace.fraction == 1.0 {
                return true;
            }
        }
    }
    false
}

/// Stock `range` band for an eye distance (`ai.qc:147`).
#[must_use]
pub fn q1_range_band(distance: f32) -> u8 {
    if distance < 120.0 {
        Q1_RANGE_MELEE
    } else if distance < 500.0 {
        Q1_RANGE_NEAR
    } else if distance < 1000.0 {
        Q1_RANGE_MID
    } else {
        Q1_RANGE_FAR
    }
}

/// Stock `range` (`ai.qc:147`): the eye-distance band to a target.
fn q1_range<L: ServerLogic>(ctx: &Q1MonsterCtx<'_, '_, '_, L>, viewer: &ActorId, targ: &ActorId) -> u8 {
    let (Some(spot1), Some(spot2)) = (q1_eye_of(ctx, viewer), q1_eye_of(ctx, targ)) else {
        return Q1_RANGE_FAR;
    };
    let delta = vec3(spot1.x - spot2.x, spot1.y - spot2.y, spot1.z - spot2.z);
    q1_range_band((delta.x * delta.x + delta.y * delta.y + delta.z * delta.z).sqrt())
}

/// Stock `infront` (`ai.qc:195`): the target sits within the forward
/// cone (dot above 0.3).
fn q1_infront<L: ServerLogic>(ctx: &Q1MonsterCtx<'_, '_, '_, L>, viewer: &ActorId, targ: &ActorId) -> bool {
    let viewer_body = ctx.server.simulation().body_state(viewer);
    let targ_body = ctx.server.simulation().body_state(targ);
    let (Some(viewer_body), Some(targ_body)) = (viewer_body, targ_body) else {
        return false;
    };
    let forward = angle_vectors(viewer_body.angles).forward;
    let delta = vec3(
        targ_body.origin.x - viewer_body.origin.x,
        targ_body.origin.y - viewer_body.origin.y,
        targ_body.origin.z - viewer_body.origin.z,
    );
    let len = (delta.x * delta.x + delta.y * delta.y + delta.z * delta.z).sqrt();
    if len == 0.0 {
        return false;
    }
    (delta.x / len) * forward.x + (delta.y / len) * forward.y + (delta.z / len) * forward.z > 0.3
}

/// Stock `checkclient` for one client (`pr_cmds.c`): the admitted
/// player when the PVS connects the monster's eye leaf to the player's.
fn q1_checkclient<L: ServerLogic>(ctx: &Q1MonsterCtx<'_, '_, '_, L>, viewer: &ActorId) -> Option<ActorId> {
    let player = ctx.behaviors.player.clone()?;
    let spot1 = q1_eye_of(ctx, viewer)?;
    let spot2 = q1_eye_of(ctx, &player)?;
    let from = ctx.scene.point_leaf(spot1).ok()?;
    let to = ctx.scene.point_leaf(spot2).ok()?;
    if ctx.scene.cluster_visible(from, to, SceneVisibilityKind::Pvs).ok()? {
        Some(player)
    } else {
        None
    }
}

/// Stock `FindTarget` (`ai.qc:343`): ambush monsters ignore the shared
/// sighting and wait for a real look; everyone else takes a woken
/// monster's enemy or the PVS client, gated by range, line of sight,
/// and facing (near needs hostility or facing, mid needs facing).
fn q1_find_target<L: ServerLogic>(ctx: &mut Q1MonsterCtx<'_, '_, '_, L>, actor: &ActorId) -> bool {
    let now = ctx.now;
    let (spawnflags, enemy) = match ctx.behaviors.monsters.get(actor) {
        Some(monster) => (monster.spawnflags, monster.enemy.clone()),
        None => return false,
    };
    let client = if ctx.behaviors.sight_entity_time >= now - 0.1 && spawnflags & 3 == 0 {
        let Some(sight) = ctx.behaviors.sight_entity.clone() else {
            return false;
        };
        let sight_enemy = ctx
            .behaviors
            .monsters
            .get(&sight)
            .and_then(|monster| monster.enemy.clone());
        if sight_enemy == enemy {
            return false;
        }
        sight
    } else {
        let Some(client) = q1_checkclient(ctx, actor) else {
            return false;
        };
        client
    };
    if Some(&client) == enemy.as_ref() {
        return false;
    }
    if Some(&client) == ctx.behaviors.player.as_ref() {
        if ctx.behaviors.player_flags & Q1_FLAG_NOTARGET != 0 {
            return false;
        }
        if ctx.behaviors.player_items & Q1_IT_INVISIBILITY != 0 {
            return false;
        }
    }
    let range = q1_range(ctx, actor, &client);
    if range == Q1_RANGE_FAR {
        return false;
    }
    if !q1_visible(ctx, actor, &client) {
        return false;
    }
    if range == Q1_RANGE_NEAR {
        let hostile = ctx
            .behaviors
            .monsters
            .get(&client)
            .is_some_and(|monster| monster.show_hostile >= now);
        if !hostile && !q1_infront(ctx, actor, &client) {
            return false;
        }
    } else if range == Q1_RANGE_MID && !q1_infront(ctx, actor, &client) {
        return false;
    }
    // The sighting chains to the player behind a woken monster
    // (`ai.qc:405`); anything else is not a target.
    let mut found = client;
    if Some(&found) != ctx.behaviors.player.as_ref() {
        let chained = ctx
            .behaviors
            .monsters
            .get(&found)
            .and_then(|monster| monster.enemy.clone());
        let Some(chained) = chained else {
            return false;
        };
        if Some(&chained) != ctx.behaviors.player.as_ref() {
            return false;
        }
        found = chained;
    }
    if let Some(monster) = ctx.behaviors.monsters.get_mut(actor) {
        monster.enemy = Some(found);
    }
    let simulation = ctx.server.simulation_mut();
    q1_found_target(ctx.behaviors, simulation, actor);
    true
}

/// Stock `walkmove` yaw step (`pr_cmds.c:1146`): grounded, flying, and
/// swimming monsters step; airborne step monsters stand still.
fn q1_walkmove<L: ServerLogic>(ctx: &mut Q1MonsterCtx<'_, '_, '_, L>, actor: &ActorId, yaw: f64, dist: f64) -> bool {
    let grounded = ctx
        .behaviors
        .monsters
        .get(actor)
        .is_some_and(|monster| monster.flags & (Q1_FLAG_ONGROUND | Q1_FLAG_FLY | Q1_FLAG_SWIM) != 0);
    if !grounded {
        return false;
    }
    let Some(owned) = ctx.owned(actor) else {
        return false;
    };
    ctx.movement().walk_move(&owned, yaw, dist).unwrap_or(false)
}

/// Stock `movetogoal` (`sv_move.c:393`): chase the current goal entity
/// (nothing to chase without one).
fn q1_movetogoal<L: ServerLogic>(ctx: &mut Q1MonsterCtx<'_, '_, '_, L>, actor: &ActorId, dist: f64) {
    let goal = ctx
        .behaviors
        .monsters
        .get(actor)
        .and_then(|monster| monster.goalentity.clone());
    let (Some(goal), Some(owned)) = (goal, ctx.owned(actor)) else {
        return;
    };
    let _ignored = ctx.movement().move_to_goal(&owned, &goal, dist, false);
}

/// Stock `ChangeYaw` (`pr_cmds.c:1412`): turn toward the ideal yaw at
/// yaw speed.
fn q1_change_yaw<L: ServerLogic>(ctx: &mut Q1MonsterCtx<'_, '_, '_, L>, actor: &ActorId) {
    let Some(owned) = ctx.owned(actor) else {
        return;
    };
    ctx.movement().change_yaw(&owned);
}

/// Stock `checkbottom` (`pr_cmds.c:1271`): every bottom corner stands
/// over ground within step height.
fn q1_check_bottom<L: ServerLogic>(ctx: &mut Q1MonsterCtx<'_, '_, '_, L>, actor: &ActorId) -> bool {
    ctx.movement().check_bottom(actor)
}

/// Stock `ai_forward` (`ai.qc:424`).
pub fn q1_ai_forward<L: ServerLogic>(ctx: &mut Q1MonsterCtx<'_, '_, '_, L>, actor: &ActorId, dist: f64) {
    let yaw = ctx
        .server
        .simulation()
        .body_state(actor)
        .map_or(0.0, |body| f64::from(body.angles.y));
    q1_walkmove(ctx, actor, yaw, dist);
}

/// Stock `ai_back` (`ai.qc:429`).
fn q1_ai_back<L: ServerLogic>(ctx: &mut Q1MonsterCtx<'_, '_, '_, L>, actor: &ActorId, dist: f64) {
    let yaw = ctx
        .server
        .simulation()
        .body_state(actor)
        .map_or(0.0, |body| f64::from(body.angles.y));
    q1_walkmove(ctx, actor, yaw + 180.0, dist);
}

/// Stock `ai_pain` (`ai.qc:442`): stagger back.
fn q1_ai_pain<L: ServerLogic>(ctx: &mut Q1MonsterCtx<'_, '_, '_, L>, actor: &ActorId, dist: f64) {
    q1_ai_back(ctx, actor, dist);
}

/// Stock `ai_painforward` (`ai.qc:462`).
pub fn q1_ai_painforward<L: ServerLogic>(ctx: &mut Q1MonsterCtx<'_, '_, '_, L>, actor: &ActorId, dist: f64) {
    let yaw = ctx
        .behaviors
        .monsters
        .get(actor)
        .map_or(0.0, |monster| monster.ideal_yaw);
    q1_walkmove(ctx, actor, yaw, dist);
}

/// Stock `ai_walk` (`ai.qc:474`): notice the player or keep patrolling.
fn q1_ai_walk<L: ServerLogic>(ctx: &mut Q1MonsterCtx<'_, '_, '_, L>, actor: &ActorId, dist: f64) {
    if q1_find_target(ctx, actor) {
        return;
    }
    q1_movetogoal(ctx, actor, dist);
}

/// Stock `ai_stand` (`ai.qc:492`): notice the player, walk the beat
/// when the pause lapses, else hold.
fn q1_ai_stand<L: ServerLogic>(ctx: &mut Q1MonsterCtx<'_, '_, '_, L>, actor: &ActorId) {
    if q1_find_target(ctx, actor) {
        return;
    }
    let now = ctx.now;
    let Some(monster) = ctx.behaviors.monsters.get(actor).cloned() else {
        return;
    };
    if now > monster.pausetime {
        if let Some(monster) = ctx.behaviors.monsters.get_mut(actor) {
            let think = q1_th_walk(monster.kind);
            let frame = match think {
                Q1MonsterThink::Frame(seq, index) => q1_seq_frame(seq, index),
                _ => monster.frame,
            };
            monster.frame = frame;
            monster.think = think;
            monster.nextthink = now + Q1_MONSTER_THINK_STEP;
        }
    }
}

/// Stock `ai_turn` (`ai.qc:511`): notice the player or turn in place.
pub fn q1_ai_turn<L: ServerLogic>(ctx: &mut Q1MonsterCtx<'_, '_, '_, L>, actor: &ActorId) {
    if q1_find_target(ctx, actor) {
        return;
    }
    q1_change_yaw(ctx, actor);
}

/// Stock `ai_face` (`fight.qc:135`): stay facing the enemy.
fn q1_ai_face<L: ServerLogic>(ctx: &mut Q1MonsterCtx<'_, '_, '_, L>, actor: &ActorId) {
    let enemy = ctx
        .behaviors
        .monsters
        .get(actor)
        .and_then(|monster| monster.enemy.clone());
    let Some(enemy) = enemy else {
        return;
    };
    let from = ctx.server.simulation().body_state(actor).map(|body| body.origin);
    let to = ctx.server.simulation().body_state(&enemy).map(|body| body.origin);
    if let (Some(from), Some(to)) = (from, to) {
        let yaw = q1_vectoyaw(vec3(to.x - from.x, to.y - from.y, to.z - from.z));
        if let Some(monster) = ctx.behaviors.monsters.get_mut(actor) {
            monster.ideal_yaw = yaw;
        }
        q1_change_yaw(ctx, actor);
    }
}

/// Stock `ai_charge` (`fight.qc:157`): face the enemy and close.
fn q1_ai_charge<L: ServerLogic>(ctx: &mut Q1MonsterCtx<'_, '_, '_, L>, actor: &ActorId, dist: f64) {
    q1_ai_face(ctx, actor);
    q1_movetogoal(ctx, actor, dist);
}

/// Stock `ai_charge_side` (`fight.qc:163`): face the enemy and fly by
/// to its left.
pub fn q1_ai_charge_side<L: ServerLogic>(ctx: &mut Q1MonsterCtx<'_, '_, '_, L>, actor: &ActorId) {
    let enemy = ctx
        .behaviors
        .monsters
        .get(actor)
        .and_then(|monster| monster.enemy.clone());
    let Some(enemy) = enemy else {
        return;
    };
    let from = ctx.server.simulation().body_state(actor);
    let to = ctx.server.simulation().body_state(&enemy).map(|body| body.origin);
    let (Some(from), Some(to)) = (from, to) else {
        return;
    };
    let yaw = q1_vectoyaw(vec3(to.x - from.origin.x, to.y - from.origin.y, to.z - from.origin.z));
    if let Some(monster) = ctx.behaviors.monsters.get_mut(actor) {
        monster.ideal_yaw = yaw;
    }
    q1_change_yaw(ctx, actor);
    let right = angle_vectors(from.angles).right;
    let aim = vec3(to.x - 30.0 * right.x, to.y - 30.0 * right.y, to.z - 30.0 * right.z);
    let heading = q1_vectoyaw(vec3(
        aim.x - from.origin.x,
        aim.y - from.origin.y,
        aim.z - from.origin.z,
    ));
    q1_walkmove(ctx, actor, heading, 20.0);
}

/// Stock `ai_melee` (`fight.qc:190`): a 60-unit slash for light damage.
pub fn q1_ai_melee<L: ServerLogic>(ctx: &mut Q1MonsterCtx<'_, '_, '_, L>, actor: &ActorId) {
    let enemy = ctx
        .behaviors
        .monsters
        .get(actor)
        .and_then(|monster| monster.enemy.clone());
    let Some(enemy) = enemy else {
        return;
    };
    let from = ctx.server.simulation().body_state(actor).map(|body| body.origin);
    let to = ctx.server.simulation().body_state(&enemy).map(|body| body.origin);
    let (Some(from), Some(to)) = (from, to) else {
        return;
    };
    let delta = vec3(from.x - to.x, from.y - to.y, from.z - to.z);
    if (delta.x * delta.x + delta.y * delta.y + delta.z * delta.z).sqrt() > 60.0 {
        return;
    }
    let damage = f64::from(
        q1_monster_random(ctx.behaviors) + q1_monster_random(ctx.behaviors) + q1_monster_random(ctx.behaviors),
    ) * 3.0;
    let me = actor.clone();
    let (simulation, movers, triggers) = ctx.server.simulation_movers_and_triggers_mut();
    q1_t_damage(
        ctx.behaviors,
        simulation,
        movers,
        triggers,
        &enemy,
        Some(&me),
        Some(&me),
        damage,
    );
}

/// Stock `ai_melee_side` (`fight.qc:209`): fly by, then slash when the
/// line is clear.
pub fn q1_ai_melee_side<L: ServerLogic>(ctx: &mut Q1MonsterCtx<'_, '_, '_, L>, actor: &ActorId) {
    let enemy = ctx
        .behaviors
        .monsters
        .get(actor)
        .and_then(|monster| monster.enemy.clone());
    let Some(enemy) = enemy else {
        return;
    };
    q1_ai_charge_side(ctx, actor);
    let from = ctx.server.simulation().body_state(actor).map(|body| body.origin);
    let to = ctx.server.simulation().body_state(&enemy).map(|body| body.origin);
    let (Some(from), Some(to)) = (from, to) else {
        return;
    };
    let delta = vec3(from.x - to.x, from.y - to.y, from.z - to.z);
    if (delta.x * delta.x + delta.y * delta.y + delta.z * delta.z).sqrt() > 60.0 {
        return;
    }
    if !q1_can_damage(ctx, &enemy, actor) {
        return;
    }
    let damage = f64::from(
        q1_monster_random(ctx.behaviors) + q1_monster_random(ctx.behaviors) + q1_monster_random(ctx.behaviors),
    ) * 3.0;
    let me = actor.clone();
    let (simulation, movers, triggers) = ctx.server.simulation_movers_and_triggers_mut();
    q1_t_damage(
        ctx.behaviors,
        simulation,
        movers,
        triggers,
        &enemy,
        Some(&me),
        Some(&me),
        damage,
    );
}

/// Stock `FacingIdeal` (`ai.qc:575`): turned within 45 degrees of ideal.
fn q1_facing_ideal<L: ServerLogic>(ctx: &Q1MonsterCtx<'_, '_, '_, L>, actor: &ActorId) -> bool {
    let angles = ctx.server.simulation().body_state(actor).map(|body| body.angles);
    let ideal = ctx.behaviors.monsters.get(actor).map(|monster| monster.ideal_yaw);
    let (Some(angles), Some(ideal)) = (angles, ideal) else {
        return false;
    };
    let delta = qa_core::math::angle_mod(f64::from(angles.y) - ideal);
    !(delta > 45.0 && delta < 315.0)
}

/// Per-think enemy memo: stock `enemy_vis`, `enemy_range`, `enemy_yaw`
/// (`fight.qc:22-23`), recomputed each `ai_run` (per-monster attack
/// checks read it; `enemy_infront` rejoins when a check needs it).
pub struct Q1EnemyMemo {
    /// Whether the enemy is visible.
    pub vis: bool,
    /// Range band to the enemy.
    pub range: u8,
    /// Compass yaw to the enemy.
    pub yaw: f64,
}

/// Stock `ai_run_melee` (`ai.qc:612`): turn onto the enemy, then slash.
fn q1_ai_run_melee<L: ServerLogic>(ctx: &mut Q1MonsterCtx<'_, '_, '_, L>, actor: &ActorId, memo: &Q1EnemyMemo) {
    if let Some(monster) = ctx.behaviors.monsters.get_mut(actor) {
        monster.ideal_yaw = memo.yaw;
    }
    q1_change_yaw(ctx, actor);
    if q1_facing_ideal(ctx, actor) {
        // `AS_MELEE` is only ever set where `th_melee` exists; without
        // one the run holds (stock would call `SUB_Null`).
        let kind = ctx.behaviors.monsters.get(actor).map(|monster| monster.kind);
        let think = kind.and_then(|kind| q1_th_melee(ctx.behaviors, kind));
        let Some(think) = think else {
            return;
        };
        if let Some(monster) = ctx.behaviors.monsters.get_mut(actor) {
            let frame = match think {
                Q1MonsterThink::Frame(seq, index) => q1_seq_frame(seq, index),
                _ => monster.frame,
            };
            monster.frame = frame;
            monster.think = think;
            monster.attack_state = Q1_AS_STRAIGHT;
        }
    }
}

/// Stock `ai_run_missile` (`ai.qc:632`): turn onto the enemy, then fire.
fn q1_ai_run_missile<L: ServerLogic>(ctx: &mut Q1MonsterCtx<'_, '_, '_, L>, actor: &ActorId, memo: &Q1EnemyMemo) {
    if let Some(monster) = ctx.behaviors.monsters.get_mut(actor) {
        monster.ideal_yaw = memo.yaw;
    }
    q1_change_yaw(ctx, actor);
    if q1_facing_ideal(ctx, actor) {
        // `AS_MISSILE` is only ever set where `th_missile` exists;
        // without one the run holds (stock would call `SUB_Null`).
        let kind = ctx.behaviors.monsters.get(actor).map(|monster| monster.kind);
        let Some(think) = kind.and_then(q1_th_missile) else {
            return;
        };
        if let Some(monster) = ctx.behaviors.monsters.get_mut(actor) {
            let frame = match think {
                Q1MonsterThink::Frame(seq, index) => q1_seq_frame(seq, index),
                _ => monster.frame,
            };
            monster.frame = frame;
            monster.think = think;
            monster.attack_state = Q1_AS_STRAIGHT;
        }
    }
}

/// Stock `ai_run_slide` (`ai.qc:652`): strafe at the same range,
/// flipping sides when blocked.
fn q1_ai_run_slide<L: ServerLogic>(
    ctx: &mut Q1MonsterCtx<'_, '_, '_, L>,
    actor: &ActorId,
    memo: &Q1EnemyMemo,
    dist: f64,
) {
    if let Some(monster) = ctx.behaviors.monsters.get_mut(actor) {
        monster.ideal_yaw = memo.yaw;
    }
    q1_change_yaw(ctx, actor);
    let (ideal, lefty) = match ctx.behaviors.monsters.get(actor) {
        Some(monster) => (monster.ideal_yaw, monster.lefty),
        None => return,
    };
    let ofs = if lefty { 90.0 } else { -90.0 };
    if q1_walkmove(ctx, actor, ideal + ofs, dist) {
        return;
    }
    if let Some(monster) = ctx.behaviors.monsters.get_mut(actor) {
        monster.lefty = !monster.lefty;
    }
    q1_walkmove(ctx, actor, ideal - ofs, dist);
}

/// Whether a monster has a clear shot at its enemy: a `MOVE_NORMAL`
/// eye-to-eye trace (monsters in the way block) that lands on the
/// enemy without crossing media (the shared `CheckAttack` preamble,
/// `fight.qc:49`).
fn q1_clear_shot<L: ServerLogic>(ctx: &mut Q1MonsterCtx<'_, '_, '_, L>, actor: &ActorId, enemy: &ActorId) -> bool {
    let (Some(spot1), Some(spot2)) = (q1_eye_of(ctx, actor), q1_eye_of(ctx, enemy)) else {
        return false;
    };
    let query = SceneTraceQuery {
        start: spot1,
        end: spot2,
        shape: SceneTraceShape::Point,
        target: SceneQueryTarget::World,
        policy: SceneTracePolicy::Q1 {
            move_rule: SceneQ1MoveRule::Normal,
            hull: None,
        },
        numeric: Q1_DONOR_PROFILE,
        pass_actor: Some(actor.clone()),
    };
    ctx.scene.trace(&query).ok().is_some_and(|trace| {
        !q1_trace_crossed_media(&trace) && matches!(&trace.hit, SceneTraceHit::Actor { actor: hit } if hit == enemy)
    })
}

/// Stock `CheckAttack` (`fight.qc:49`): a clear shot (monsters in the
/// way count as blocking) starts a melee or missile attack by range
/// and chance. `has_melee`/`has_missile` are the `th_melee`/`th_missile`
/// presence checks; the knight swing selection lands with its slice.
pub fn q1_check_attack<L: ServerLogic>(
    ctx: &mut Q1MonsterCtx<'_, '_, '_, L>,
    actor: &ActorId,
    memo: &Q1EnemyMemo,
    has_melee: bool,
    has_missile: bool,
) -> bool {
    let enemy = ctx
        .behaviors
        .monsters
        .get(actor)
        .and_then(|monster| monster.enemy.clone());
    let Some(enemy) = enemy else {
        return false;
    };
    if !q1_clear_shot(ctx, actor, &enemy) {
        return false;
    }
    if memo.range == Q1_RANGE_MELEE && has_melee {
        let kind = ctx.behaviors.monsters.get(actor).map(|monster| monster.kind);
        let think = kind.and_then(|kind| q1_th_melee(ctx.behaviors, kind));
        let Some(think) = think else {
            return false;
        };
        if let Some(monster) = ctx.behaviors.monsters.get_mut(actor) {
            let frame = match think {
                Q1MonsterThink::Frame(seq, index) => q1_seq_frame(seq, index),
                _ => monster.frame,
            };
            monster.frame = frame;
            monster.think = think;
        }
        return true;
    }
    if !has_missile {
        return false;
    }
    let now = ctx.now;
    let attack_finished = ctx
        .behaviors
        .monsters
        .get(actor)
        .map_or(0.0, |monster| monster.attack_finished);
    if now < attack_finished || memo.range == Q1_RANGE_FAR {
        return false;
    }
    let chance = match memo.range {
        r if r == Q1_RANGE_MELEE => {
            if let Some(monster) = ctx.behaviors.monsters.get_mut(actor) {
                monster.attack_finished = 0.0;
            }
            0.9
        }
        r if r == Q1_RANGE_NEAR => {
            if has_melee {
                0.2
            } else {
                0.4
            }
        }
        r if r == Q1_RANGE_MID => {
            if has_melee {
                0.05
            } else {
                0.1
            }
        }
        _ => 0.0,
    };
    if f64::from(q1_monster_random(ctx.behaviors)) < chance {
        let hold = f64::from(q1_monster_random(ctx.behaviors)) * 2.0;
        let nightmare = ctx.behaviors.skill == 3;
        let kind = ctx.behaviors.monsters.get(actor).map(|monster| monster.kind);
        let Some(think) = kind.and_then(q1_th_missile) else {
            return false;
        };
        if let Some(monster) = ctx.behaviors.monsters.get_mut(actor) {
            let frame = match think {
                Q1MonsterThink::Frame(seq, index) => q1_seq_frame(seq, index),
                _ => monster.frame,
            };
            monster.frame = frame;
            monster.think = think;
            // `SUB_AttackFinished (2*random())`: nightmares skip it.
            monster.cnt = 0;
            if !nightmare {
                monster.attack_finished = now + hold;
            }
        }
        return true;
    }
    false
}

/// Stock `CheckDogMelee` (`dog.qc`): melee range always slashes.
fn q1_check_dog_melee<L: ServerLogic>(
    ctx: &mut Q1MonsterCtx<'_, '_, '_, L>,
    actor: &ActorId,
    memo: &Q1EnemyMemo,
) -> bool {
    if memo.range == Q1_RANGE_MELEE {
        if let Some(monster) = ctx.behaviors.monsters.get_mut(actor) {
            monster.attack_state = Q1_AS_MELEE;
        }
        return true;
    }
    false
}

/// Stock `CheckDogJump` (`dog.qc`): leap when the bodies overlap
/// vertically and the flat distance sits between 80 and 150.
fn q1_check_dog_jump<L: ServerLogic>(ctx: &Q1MonsterCtx<'_, '_, '_, L>, actor: &ActorId) -> bool {
    let enemy = ctx
        .behaviors
        .monsters
        .get(actor)
        .and_then(|monster| monster.enemy.clone());
    let Some(enemy) = enemy else {
        return false;
    };
    let me = ctx.server.simulation().body_state(actor);
    let foe = ctx.server.simulation().body_state(&enemy);
    let (Some(me), Some(foe)) = (me, foe) else {
        return false;
    };
    let foe_size_z = foe.bounds.max.z - foe.bounds.min.z;
    if me.origin.z + me.bounds.min.z > foe.origin.z + foe.bounds.min.z + 0.75 * foe_size_z {
        return false;
    }
    if me.origin.z + me.bounds.max.z < foe.origin.z + foe.bounds.min.z + 0.25 * foe_size_z {
        return false;
    }
    let dx = foe.origin.x - me.origin.x;
    let dy = foe.origin.y - me.origin.y;
    let dist = (dx * dx + dy * dy).sqrt();
    if dist < 80.0 || dist > 150.0 {
        return false;
    }
    true
}

/// Stock `DogCheckAttack` (`dog.qc`): slash in melee range, leap at
/// jump distance, else keep running.
fn q1_dog_check_attack<L: ServerLogic>(
    ctx: &mut Q1MonsterCtx<'_, '_, '_, L>,
    actor: &ActorId,
    memo: &Q1EnemyMemo,
) -> bool {
    if q1_check_dog_melee(ctx, actor, memo) {
        if let Some(monster) = ctx.behaviors.monsters.get_mut(actor) {
            monster.attack_state = Q1_AS_MELEE;
        }
        return true;
    }
    if q1_check_dog_jump(ctx, actor) {
        if let Some(monster) = ctx.behaviors.monsters.get_mut(actor) {
            monster.attack_state = Q1_AS_MISSILE;
        }
        return true;
    }
    false
}

/// Stock `SoldierCheckAttack` (`fight.qc:235`): a clear shot starts
/// the missile attack by range and chance, holding the next one for
/// `1 + random()` seconds and sometimes flipping the strafe side.
fn q1_soldier_check_attack<L: ServerLogic>(
    ctx: &mut Q1MonsterCtx<'_, '_, '_, L>,
    actor: &ActorId,
    memo: &Q1EnemyMemo,
) -> bool {
    let enemy = ctx
        .behaviors
        .monsters
        .get(actor)
        .and_then(|monster| monster.enemy.clone());
    let Some(enemy) = enemy else {
        return false;
    };
    if !q1_clear_shot(ctx, actor, &enemy) {
        return false;
    }
    let now = ctx.now;
    let attack_finished = ctx
        .behaviors
        .monsters
        .get(actor)
        .map_or(0.0, |monster| monster.attack_finished);
    if now < attack_finished || memo.range == Q1_RANGE_FAR {
        return false;
    }
    let chance = match memo.range {
        r if r == Q1_RANGE_MELEE => 0.9,
        r if r == Q1_RANGE_NEAR => 0.4,
        r if r == Q1_RANGE_MID => 0.05,
        _ => 0.0,
    };
    if f64::from(q1_monster_random(ctx.behaviors)) < chance {
        let kind = ctx.behaviors.monsters.get(actor).map(|monster| monster.kind);
        let Some(think) = kind.and_then(q1_th_missile) else {
            return false;
        };
        // `SUB_AttackFinished (1 + random())`: nightmares skip the hold.
        let hold = 1.0 + f64::from(q1_monster_random(ctx.behaviors));
        let nightmare = ctx.behaviors.skill == 3;
        if let Some(monster) = ctx.behaviors.monsters.get_mut(actor) {
            let frame = match think {
                Q1MonsterThink::Frame(seq, index) => q1_seq_frame(seq, index),
                _ => monster.frame,
            };
            monster.frame = frame;
            monster.think = think;
            monster.cnt = 0;
            if !nightmare {
                monster.attack_finished = now + hold;
            }
        }
        if q1_monster_random(ctx.behaviors) < 0.3 {
            if let Some(monster) = ctx.behaviors.monsters.get_mut(actor) {
                monster.lefty = !monster.lefty;
            }
        }
        return true;
    }
    false
}

/// Stock `OgreCheckAttack` (`fight.qc:354`): melee range with a damage
/// line slashes; otherwise a clear shot under far range always lobs —
/// stock computes the range chances but never rolls them, so the
/// missile holds `1 + 2*random()` seconds and fires.
fn q1_ogre_check_attack<L: ServerLogic>(
    ctx: &mut Q1MonsterCtx<'_, '_, '_, L>,
    actor: &ActorId,
    memo: &Q1EnemyMemo,
) -> bool {
    let enemy = ctx
        .behaviors
        .monsters
        .get(actor)
        .and_then(|monster| monster.enemy.clone());
    let Some(enemy) = enemy else {
        return false;
    };
    if memo.range == Q1_RANGE_MELEE && q1_can_damage(ctx, &enemy, actor) {
        if let Some(monster) = ctx.behaviors.monsters.get_mut(actor) {
            monster.attack_state = Q1_AS_MELEE;
        }
        return true;
    }
    let now = ctx.now;
    let attack_finished = ctx
        .behaviors
        .monsters
        .get(actor)
        .map_or(0.0, |monster| monster.attack_finished);
    if now < attack_finished || memo.range == Q1_RANGE_FAR {
        return false;
    }
    if !q1_clear_shot(ctx, actor, &enemy) {
        return false;
    }
    let hold = 1.0 + f64::from(q1_monster_random(ctx.behaviors)) * 2.0;
    let nightmare = ctx.behaviors.skill == 3;
    if let Some(monster) = ctx.behaviors.monsters.get_mut(actor) {
        monster.attack_state = Q1_AS_MISSILE;
        // `SUB_AttackFinished (1 + 2*random())`: nightmares skip the hold.
        monster.cnt = 0;
        if !nightmare {
            monster.attack_finished = now + hold;
        }
    }
    true
}

/// Stock `CheckAnyAttack` (`ai.qc:592`): per-classname attack checks.
/// Kinds join the dispatch as their slices land; everyone else runs
/// the generic check.
fn q1_check_any_attack<L: ServerLogic>(
    ctx: &mut Q1MonsterCtx<'_, '_, '_, L>,
    actor: &ActorId,
    memo: &Q1EnemyMemo,
) -> bool {
    if !memo.vis {
        return false;
    }
    let kind = ctx.behaviors.monsters.get(actor).map(|monster| monster.kind);
    match kind {
        Some(Q1MonsterKind::Dog) => q1_dog_check_attack(ctx, actor, memo),
        Some(Q1MonsterKind::Grunt) => q1_soldier_check_attack(ctx, actor, memo),
        // Enforcers run the generic check (`CheckAttack`,
        // `fight.qc:57`): no `th_melee`, `th_missile` armed.
        Some(Q1MonsterKind::Enforcer) => q1_check_attack(ctx, actor, memo, false, true),
        Some(Q1MonsterKind::Ogre) => q1_ogre_check_attack(ctx, actor, memo),
        None => false,
    }
}

/// Stock `ai_run` (`ai.qc:677`): chase and kill the enemy — dead
/// enemies fall through to the old enemy or the beat, coop re-scans
/// for new victims, and sightings refresh the hunt.
fn q1_ai_run<L: ServerLogic>(ctx: &mut Q1MonsterCtx<'_, '_, '_, L>, actor: &ActorId, dist: f64) {
    let now = ctx.now;
    let Some(monster) = ctx.behaviors.monsters.get(actor).cloned() else {
        return;
    };
    let Some(enemy) = monster.enemy.clone() else {
        return;
    };
    if q1_health_of(ctx.server.simulation(), &enemy) <= 0.0 {
        if let Some(monster) = ctx.behaviors.monsters.get_mut(actor) {
            monster.enemy = None;
        }
        let old_healthy = monster
            .oldenemy
            .as_ref()
            .is_some_and(|old| q1_health_of(ctx.server.simulation(), old) > 0.0);
        if old_healthy {
            if let Some(monster) = ctx.behaviors.monsters.get_mut(actor) {
                monster.enemy = monster.oldenemy.clone();
            }
            let simulation = ctx.server.simulation_mut();
            q1_hunt_target(ctx.behaviors, simulation, actor);
        } else if let Some(monster) = ctx.behaviors.monsters.get_mut(actor) {
            let think = if monster.movetarget.is_some() {
                q1_th_walk(monster.kind)
            } else {
                q1_th_stand(monster.kind)
            };
            let frame = match think {
                Q1MonsterThink::Frame(seq, index) => q1_seq_frame(seq, index),
                _ => monster.frame,
            };
            monster.frame = frame;
            monster.think = think;
            monster.nextthink = now + Q1_MONSTER_THINK_STEP;
        }
        return;
    }
    if let Some(monster) = ctx.behaviors.monsters.get_mut(actor) {
        monster.show_hostile = now + 1.0;
    }
    let vis = q1_visible(ctx, actor, &enemy);
    if vis {
        if let Some(monster) = ctx.behaviors.monsters.get_mut(actor) {
            monster.search_time = now + 5.0;
        }
    }
    let search_time = ctx
        .behaviors
        .monsters
        .get(actor)
        .map_or(0.0, |monster| monster.search_time);
    if ctx.behaviors.coop && search_time < now && q1_find_target(ctx, actor) {
        return;
    }
    let range = q1_range(ctx, actor, &enemy);
    let from = ctx.server.simulation().body_state(actor).map(|body| body.origin);
    let to = ctx.server.simulation().body_state(&enemy).map(|body| body.origin);
    let yaw = match (from, to) {
        (Some(from), Some(to)) => q1_vectoyaw(vec3(to.x - from.x, to.y - from.y, to.z - from.z)),
        _ => 0.0,
    };
    let memo = Q1EnemyMemo { vis, range, yaw };
    let attack_state = ctx
        .behaviors
        .monsters
        .get(actor)
        .map_or(Q1_AS_STRAIGHT, |monster| monster.attack_state);
    if attack_state == Q1_AS_MISSILE {
        q1_ai_run_missile(ctx, actor, &memo);
        return;
    }
    if attack_state == Q1_AS_MELEE {
        q1_ai_run_melee(ctx, actor, &memo);
        return;
    }
    if q1_check_any_attack(ctx, actor, &memo) {
        return;
    }
    let attack_state = ctx
        .behaviors
        .monsters
        .get(actor)
        .map_or(Q1_AS_STRAIGHT, |monster| monster.attack_state);
    if attack_state == Q1_AS_SLIDING {
        q1_ai_run_slide(ctx, actor, &memo, dist);
        return;
    }
    q1_movetogoal(ctx, actor, dist);
}

/// Stock gravity in units/s^2 (`sv_gravity`, `sv_main.c`).
const Q1_GRAVITY: f32 = 800.0;
/// Stock velocity clamp (`sv_maxvelocity`, `sv_main.c`).
const Q1_MAX_VELOCITY: f32 = 2000.0;

/// Stock `SV_CheckVelocity` (`sv_phys.c:90`): NaNs zero, components
/// clamp to max velocity.
fn q1_check_velocity(velocity: Vec3) -> Vec3 {
    vec3(
        q1_check_component(velocity.x),
        q1_check_component(velocity.y),
        q1_check_component(velocity.z),
    )
}

/// One clamped velocity component.
fn q1_check_component(value: f32) -> f32 {
    if !value.is_finite() {
        return 0.0;
    }
    value.clamp(-Q1_MAX_VELOCITY, Q1_MAX_VELOCITY)
}

/// Stock `SV_AddGravity` (`sv_phys.c:371`): pull down one step.
fn q1_add_gravity(velocity: Vec3, dt: f64) -> Vec3 {
    vec3(velocity.x, velocity.y, velocity.z - Q1_GRAVITY * dt as f32)
}

/// Stock `ClipVelocity` (`sv_phys.c`): slide `input` off `normal` with
/// `overbounce`, quieting sub-epsilon creep (`STOP_EPSILON`).
fn q1_clip_velocity(input: Vec3, normal: Vec3, overbounce: f32) -> Vec3 {
    let backoff = (input.x * normal.x + input.y * normal.y + input.z * normal.z) * overbounce;
    vec3(
        q1_quiet_creep(input.x - normal.x * backoff),
        q1_quiet_creep(input.y - normal.y * backoff),
        q1_quiet_creep(input.z - normal.z * backoff),
    )
}

/// Zero sub-epsilon slide components.
fn q1_quiet_creep(value: f32) -> f32 {
    if value > -0.1 && value < 0.1 {
        0.0
    } else {
        value
    }
}

/// Impact plane normal of a move trace: the contact plane, else the
/// stored source plane.
fn q1_trace_normal(trace: &Q1Trace) -> Vec3 {
    match trace.contact {
        qa_world::movement::types::TraceContact::Plane(plane) => plane.normal,
        qa_world::movement::types::TraceContact::None => trace.source_plane.normal,
    }
}

/// Whether a move-trace hit blocks as BSP: the world, or a brush-model
/// actor (doors).
fn q1_hit_is_bsp(behaviors: &Q1NativeBehaviors, hit: &TraceHit) -> bool {
    match hit {
        TraceHit::World { .. } => true,
        TraceHit::Actor { actor } => behaviors.brush_models.contains_key(actor),
        TraceHit::None => false,
    }
}

/// Stock `SV_Impact` (`sv_phys.c`): run both parties' touch functions —
/// the mover's, then the victim's (a mid-leap victim leaps on).
fn q1_impact<L: ServerLogic>(ctx: &mut Q1MonsterCtx<'_, '_, '_, L>, actor: &ActorId, hit: &TraceHit) {
    let other = match hit {
        TraceHit::Actor { actor } => Some(actor.clone()),
        _ => None,
    };
    let touch = ctx.behaviors.monsters.get(actor).map(|monster| monster.touch);
    if touch == Some(Q1MonsterTouch::JumpTouch) {
        q1_dog_jump_touch(ctx, actor, other.as_ref());
    }
    if ctx
        .behaviors
        .projectiles
        .get(actor)
        .is_some_and(|projectile| projectile.kind == Q1ProjectileKind::Grenade)
    {
        q1_ogre_grenade_touch(ctx, actor, other.as_ref());
    }
    if let Some(other) = other {
        let touch = ctx.behaviors.monsters.get(&other).map(|monster| monster.touch);
        if touch == Some(Q1MonsterTouch::JumpTouch) {
            q1_dog_jump_touch(ctx, &other, Some(actor));
        }
        if ctx
            .behaviors
            .projectiles
            .get(&other)
            .is_some_and(|projectile| projectile.kind == Q1ProjectileKind::Grenade)
        {
            q1_ogre_grenade_touch(ctx, &other, Some(actor));
        }
    }
}

/// Stock `SV_FlyMove` (`sv_phys.c`): slide the body along its velocity
/// through up to four bump planes, grounding on BSP floors and
/// impacting every bump. Returns the blocked flags.
fn q1_fly_move<L: ServerLogic>(ctx: &mut Q1MonsterCtx<'_, '_, '_, L>, actor: &ActorId, dt: f64) -> i32 {
    let mut blocked = 0;
    let mut planes: Vec<Vec3> = Vec::new();
    let mut time_left = dt;
    for _ in 0..4 {
        let Some(body) = ctx.server.simulation().body_state(actor) else {
            break;
        };
        if body.velocity.x == 0.0 && body.velocity.y == 0.0 && body.velocity.z == 0.0 {
            break;
        }
        let end = vec3(
            body.origin.x + body.velocity.x * time_left as f32,
            body.origin.y + body.velocity.y * time_left as f32,
            body.origin.z + body.velocity.z * time_left as f32,
        );
        let trace = ctx
            .movement()
            .services_mut()
            .trace(actor, body.origin, end, Some(body.bounds));
        if trace.all_solid {
            let _ignored = ctx
                .server
                .simulation_mut()
                .set_body_velocity(actor, vec3(0.0, 0.0, 0.0));
            return 3;
        }
        if trace.fraction > 0.0 {
            let _ignored = ctx.server.simulation_mut().set_body_origin(actor, trace.end);
            planes.clear();
        }
        if trace.fraction == 1.0 {
            break;
        }
        let normal = q1_trace_normal(&trace);
        if normal.z > 0.7 {
            blocked |= 1;
            if q1_hit_is_bsp(ctx.behaviors, &trace.hit) {
                if let Some(monster) = ctx.behaviors.monsters.get_mut(actor) {
                    monster.flags |= Q1_FLAG_ONGROUND;
                }
            }
        }
        if normal.z == 0.0 {
            blocked |= 2;
        }
        q1_impact(ctx, actor, &trace.hit);
        if ctx.server.simulation().body_state(actor).is_none() {
            break;
        }
        time_left -= time_left * trace.fraction;
        if planes.len() >= 4 {
            let _ignored = ctx
                .server
                .simulation_mut()
                .set_body_velocity(actor, vec3(0.0, 0.0, 0.0));
            return 3;
        }
        planes.push(normal);
        let Some(body) = ctx.server.simulation().body_state(actor) else {
            break;
        };
        let mut settled = body.velocity;
        let mut creased = true;
        for (i, plane) in planes.iter().enumerate() {
            let candidate = q1_clip_velocity(body.velocity, *plane, 1.0);
            let mut ok = true;
            for (j, other) in planes.iter().enumerate() {
                if j != i && candidate.x * other.x + candidate.y * other.y + candidate.z * other.z < 0.0 {
                    ok = false;
                    break;
                }
            }
            if ok {
                settled = candidate;
                creased = false;
                break;
            }
        }
        if creased {
            // Along the crease: stock crosses the planes (two-plane
            // creases only; deeper corners stop dead).
            if planes.len() == 2 {
                let (first, second) = (planes[0], planes[1]);
                let dir = vec3(
                    first.y * second.z - first.z * second.y,
                    first.z * second.x - first.x * second.z,
                    first.x * second.y - first.y * second.x,
                );
                let along = body.velocity.x * dir.x + body.velocity.y * dir.y + body.velocity.z * dir.z;
                settled = vec3(dir.x * along, dir.y * along, dir.z * along);
            } else {
                settled = vec3(0.0, 0.0, 0.0);
            }
        }
        let _ignored = ctx.server.simulation_mut().set_body_velocity(actor, settled);
    }
    blocked
}

/// Stock `SV_Physics_Step` (`sv_phys.c`): airborne step bodies fall
/// with gravity, slide, link, and thump down hard landings.
fn q1_step_physics<L: ServerLogic>(ctx: &mut Q1MonsterCtx<'_, '_, '_, L>, actor: &ActorId) {
    if ctx.dt <= 0.0 {
        return;
    }
    let Some(body) = ctx.server.simulation().body_state(actor) else {
        return;
    };
    let hitsound = body.velocity.z < Q1_GRAVITY * -0.1;
    let velocity = q1_check_velocity(q1_add_gravity(body.velocity, ctx.dt));
    let _ignored = ctx.server.simulation_mut().set_body_velocity(actor, velocity);
    q1_fly_move(ctx, actor, ctx.dt);
    let _ignored = ctx.server.simulation_mut().link_body(actor);
    let grounded = ctx
        .behaviors
        .monsters
        .get(actor)
        .is_some_and(|monster| monster.flags & Q1_FLAG_ONGROUND != 0);
    if grounded && hitsound {
        q1_monster_sound(
            ctx.behaviors,
            actor,
            Q1_CHAN_AUTO,
            "demon/dland2.wav",
            1.0,
            Q1_ATTN_NORM,
        );
    }
}

/// Stock `SV_Physics_Toss` core (`sv_phys.c:1245`): fall, spin,
/// slide, bounce at `backoff`, and rest on quiet landings. Returns
/// whether the body came to rest this step; callers retire their own
/// records (gib spin, monster grounding, backpack settling).
fn q1_toss_step<L: ServerLogic>(
    ctx: &mut Q1MonsterCtx<'_, '_, '_, L>,
    actor: &ActorId,
    spin: Vec3,
    backoff: f32,
) -> bool {
    if ctx.dt <= 0.0 {
        return false;
    }
    let Some(body) = ctx.server.simulation().body_state(actor) else {
        return false;
    };
    let velocity = q1_check_velocity(q1_add_gravity(body.velocity, ctx.dt));
    let angles = vec3(
        body.angles.x + spin.x * ctx.dt as f32,
        body.angles.y + spin.y * ctx.dt as f32,
        body.angles.z + spin.z * ctx.dt as f32,
    );
    let _ignored = ctx.server.simulation_mut().set_body_velocity(actor, velocity);
    let _ignored = ctx.server.simulation_mut().set_body_angles(actor, angles);
    let end = vec3(
        body.origin.x + velocity.x * ctx.dt as f32,
        body.origin.y + velocity.y * ctx.dt as f32,
        body.origin.z + velocity.z * ctx.dt as f32,
    );
    let trace = ctx
        .movement()
        .services_mut()
        .trace(actor, body.origin, end, Some(body.bounds));
    // Stock `SV_PushEntity`: land at the end position, link, and impact
    // whatever stopped the slide.
    let _ignored = ctx.server.simulation_mut().set_body_origin(actor, trace.end);
    let _ignored = ctx.server.simulation_mut().link_body(actor);
    if trace.fraction == 1.0 {
        return false;
    }
    q1_impact(ctx, actor, &trace.hit);
    let Some(body) = ctx.server.simulation().body_state(actor) else {
        return false;
    };
    let normal = q1_trace_normal(&trace);
    let bounced = q1_clip_velocity(body.velocity, normal, backoff);
    let _ignored = ctx.server.simulation_mut().set_body_velocity(actor, bounced);
    if normal.z > 0.7 && bounced.z < 60.0 {
        let _ignored = ctx
            .server
            .simulation_mut()
            .set_body_velocity(actor, vec3(0.0, 0.0, 0.0));
        return true;
    }
    false
}

/// Stock `SV_Physics_Toss` (`sv_phys.c:1245`): airborne gibs fall,
/// spin, slide, bounce (1.5 backoff), and rest on quiet landings.
fn q1_toss_physics<L: ServerLogic>(ctx: &mut Q1MonsterCtx<'_, '_, '_, L>, actor: &ActorId) {
    let spin = ctx
        .behaviors
        .gibs
        .get(actor)
        .map_or(vec3(0.0, 0.0, 0.0), |gib| gib.avelocity);
    if !q1_toss_step(ctx, actor, spin, 1.5) {
        return;
    }
    if let Some(monster) = ctx.behaviors.monsters.get_mut(actor) {
        monster.flags |= Q1_FLAG_ONGROUND;
    }
    if let Some(gib) = ctx.behaviors.gibs.get_mut(actor) {
        gib.avelocity = vec3(0.0, 0.0, 0.0);
        gib.onground = true;
    }
}

/// Step one tossing backpack (`DropBackpack`, `items.qc:1332`):
/// backpacks never spin, bounce at the toss backoff, and settle on
/// quiet landings.
fn q1_backpack_actor<L: ServerLogic>(ctx: &mut Q1MonsterCtx<'_, '_, '_, L>, actor: &ActorId) {
    if !q1_toss_step(ctx, actor, vec3(0.0, 0.0, 0.0), 1.5) {
        return;
    }
    if let Some(item) = ctx.behaviors.items.get_mut(actor) {
        if let Q1ItemKind::Backpack { settled, .. } = &mut item.kind {
            *settled = true;
        }
    }
}

/// Stock `droptofloor` (`pr_cmds.c:1189`): sink the body 256 units to
/// the floor, grounding on whatever stops it.
fn q1_droptofloor<L: ServerLogic>(ctx: &mut Q1MonsterCtx<'_, '_, '_, L>, actor: &ActorId) -> bool {
    let Some(body) = ctx.server.simulation().body_state(actor) else {
        return false;
    };
    let end = vec3(body.origin.x, body.origin.y, body.origin.z - 256.0);
    let trace = ctx
        .movement()
        .services_mut()
        .trace(actor, body.origin, end, Some(body.bounds));
    if trace.fraction == 1.0 || trace.all_solid {
        return false;
    }
    let _ignored = ctx.server.simulation_mut().set_body_origin(actor, trace.end);
    let _ignored = ctx.server.simulation_mut().link_body(actor);
    if let Some(monster) = ctx.behaviors.monsters.get_mut(actor) {
        monster.flags |= Q1_FLAG_ONGROUND;
    }
    true
}

/// Stock `walkmonster_start_go` (`monsters.qc:69`): drop to the floor,
/// arm damage, face the spawn yaw, and stand — or walk out along a
/// patrol route when the spawn target names a corner.
fn q1_start_go<L: ServerLogic>(ctx: &mut Q1MonsterCtx<'_, '_, '_, L>, actor: &ActorId) {
    let now = ctx.now;
    if let Some(body) = ctx.server.simulation().body_state(actor) {
        let _ignored = ctx
            .server
            .simulation_mut()
            .set_body_origin(actor, vec3(body.origin.x, body.origin.y, body.origin.z + 1.0));
    }
    q1_droptofloor(ctx, actor);
    q1_walkmove(ctx, actor, 0.0, 0.0);
    if let Some(combat) = ctx.server.simulation().combat_state(actor).cloned() {
        let _ignored = ctx.server.simulation_mut().set_combat(
            actor,
            CombatState {
                can_take_damage: true,
                ..combat
            },
        );
    }
    let spawn_yaw = ctx
        .server
        .simulation()
        .body_state(actor)
        .map_or(0.0, |body| f64::from(body.angles.y));
    if let Some(monster) = ctx.behaviors.monsters.get_mut(actor) {
        monster.takedamage = Q1_DAMAGE_AIM;
        monster.ideal_yaw = spawn_yaw;
        if monster.yaw_speed == 0.0 {
            monster.yaw_speed = 20.0;
        }
        monster.view_ofs = vec3(0.0, 0.0, Q1_MONSTER_VIEW_OFS_Z);
        monster.flags |= Q1_FLAG_MONSTER;
    }
    let target = ctx
        .behaviors
        .monsters
        .get(actor)
        .and_then(|monster| monster.source.target.clone());
    if let Some(target) = target {
        let goal = ctx
            .behaviors
            .by_targetname
            .get(&target)
            .and_then(|matches| matches.first().cloned());
        let from = ctx.server.simulation().body_state(actor).map(|body| body.origin);
        let to = goal
            .as_ref()
            .and_then(|goal| ctx.server.simulation().body_state(goal).map(|body| body.origin));
        if let (Some(from), to) = (from, to) {
            let to = to.unwrap_or(vec3(0.0, 0.0, 0.0));
            let yaw = q1_vectoyaw(vec3(to.x - from.x, to.y - from.y, to.z - from.z));
            if let Some(monster) = ctx.behaviors.monsters.get_mut(actor) {
                monster.ideal_yaw = yaw;
            }
        }
        let is_corner = goal
            .as_ref()
            .is_some_and(|goal| ctx.behaviors.movetargets.contains_key(goal));
        if let Some(monster) = ctx.behaviors.monsters.get_mut(actor) {
            monster.goalentity = goal.clone();
            monster.movetarget = goal;
            let think = if is_corner {
                q1_th_walk(monster.kind)
            } else {
                monster.pausetime = 99_999_999.0;
                q1_th_stand(monster.kind)
            };
            let frame = match think {
                Q1MonsterThink::Frame(seq, index) => q1_seq_frame(seq, index),
                _ => monster.frame,
            };
            monster.frame = frame;
            monster.think = think;
        }
    } else if let Some(monster) = ctx.behaviors.monsters.get_mut(actor) {
        monster.pausetime = 99_999_999.0;
        let think = q1_th_stand(monster.kind);
        let frame = match think {
            Q1MonsterThink::Frame(seq, index) => q1_seq_frame(seq, index),
            _ => monster.frame,
        };
        monster.frame = frame;
        monster.think = think;
    }
    let spread = f64::from(q1_monster_random(ctx.behaviors)) * 0.5;
    if let Some(monster) = ctx.behaviors.monsters.get_mut(actor) {
        monster.nextthink = now + Q1_MONSTER_THINK_STEP + spread;
    }
}

/// Stock `dog_bite` (`dog.qc`): charge, then wound nearby enemies in
/// the open for `(r+r+r)*8`.
fn q1_dog_bite<L: ServerLogic>(ctx: &mut Q1MonsterCtx<'_, '_, '_, L>, actor: &ActorId) {
    let enemy = ctx
        .behaviors
        .monsters
        .get(actor)
        .and_then(|monster| monster.enemy.clone());
    let Some(enemy) = enemy else {
        return;
    };
    q1_ai_charge(ctx, actor, 10.0);
    if !q1_can_damage(ctx, &enemy, actor) {
        return;
    }
    let from = ctx.server.simulation().body_state(actor).map(|body| body.origin);
    let to = ctx.server.simulation().body_state(&enemy).map(|body| body.origin);
    let (Some(from), Some(to)) = (from, to) else {
        return;
    };
    let delta = vec3(from.x - to.x, from.y - to.y, from.z - to.z);
    if (delta.x * delta.x + delta.y * delta.y + delta.z * delta.z).sqrt() > 100.0 {
        return;
    }
    let damage = f64::from(
        q1_monster_random(ctx.behaviors) + q1_monster_random(ctx.behaviors) + q1_monster_random(ctx.behaviors),
    ) * 8.0;
    let me = actor.clone();
    let (simulation, movers, triggers) = ctx.server.simulation_movers_and_triggers_mut();
    q1_t_damage(
        ctx.behaviors,
        simulation,
        movers,
        triggers,
        &enemy,
        Some(&me),
        Some(&me),
        damage,
    );
}

/// Stock multi-damage accumulator (`weapons.qc:152-189`): pellets
/// striking one victim combine into a single `T_Damage`. Stock keeps
/// this in globals; the burst owns it here.
struct Q1MultiDamage {
    /// Victim of the open accumulation (`None` is stock `world`).
    ent: Option<ActorId>,
    /// Combined damage so far.
    damage: f64,
}

/// Stock `ApplyMultiDamage` (`weapons.qc:167`): damage the accumulated
/// victim once, blamed on the firer.
fn q1_apply_multi_damage<L: ServerLogic>(
    ctx: &mut Q1MonsterCtx<'_, '_, '_, L>,
    multi: &mut Q1MultiDamage,
    firer: &ActorId,
) {
    let Some(ent) = multi.ent.clone() else {
        return;
    };
    let damage = multi.damage;
    multi.ent = None;
    multi.damage = 0.0;
    let me = firer.clone();
    let (simulation, movers, triggers) = ctx.server.simulation_movers_and_triggers_mut();
    q1_t_damage(
        ctx.behaviors,
        simulation,
        movers,
        triggers,
        &ent,
        Some(&me),
        Some(&me),
        damage,
    );
}

/// Stock `AddMultiDamage` (`weapons.qc:175`): fold a pellet into the
/// open accumulation, flushing when the victim changes.
fn q1_add_multi_damage<L: ServerLogic>(
    ctx: &mut Q1MonsterCtx<'_, '_, '_, L>,
    multi: &mut Q1MultiDamage,
    firer: &ActorId,
    hit: &ActorId,
    damage: f64,
) {
    if multi.ent.as_ref() != Some(hit) {
        q1_apply_multi_damage(ctx, multi, firer);
        multi.damage = damage;
        multi.ent = Some(hit.clone());
    } else {
        multi.damage += damage;
    }
}

/// Stock `TraceAttack` (`weapons.qc:203`): one pellet strike — blood
/// and combined damage on the damageable, a wall puff otherwise. The
/// spread axes ride the call (stock reads the `FireBullets`
/// `makevectors` globals).
#[allow(clippy::too_many_arguments)]
fn q1_trace_attack<L: ServerLogic>(
    ctx: &mut Q1MonsterCtx<'_, '_, '_, L>,
    multi: &mut Q1MultiDamage,
    firer: &ActorId,
    trace: &Q1Trace,
    dir: Vec3,
    right: Vec3,
    up: Vec3,
    damage: f64,
) {
    // `normalize (dir + v_up*crandom() + v_right*crandom())`: the up
    // jitter draws first, exactly like stock.
    let up_jitter = q1_monster_crandom(ctx.behaviors);
    let right_jitter = q1_monster_crandom(ctx.behaviors);
    let raw = vec3(
        dir.x + up.x * up_jitter + right.x * right_jitter,
        dir.y + up.y * up_jitter + right.y * right_jitter,
        dir.z + up.z * up_jitter + right.z * right_jitter,
    );
    let len = (raw.x * raw.x + raw.y * raw.y + raw.z * raw.z).sqrt();
    let unit = if len == 0.0 {
        vec3(0.0, 0.0, 0.0)
    } else {
        vec3(raw.x / len, raw.y / len, raw.z / len)
    };
    let normal = q1_trace_normal(trace);
    let vel = vec3(
        (unit.x + 2.0 * normal.x) * 200.0,
        (unit.y + 2.0 * normal.y) * 200.0,
        (unit.z + 2.0 * normal.z) * 200.0,
    );
    let org = vec3(
        trace.end.x - dir.x * 4.0,
        trace.end.y - dir.y * 4.0,
        trace.end.z - dir.z * 4.0,
    );
    let victim = match &trace.hit {
        TraceHit::Actor { actor } => Some(actor.clone()),
        _ => None,
    };
    if victim
        .as_ref()
        .is_some_and(|victim| q1_can_take_damage(ctx.server.simulation(), victim))
    {
        let victim = victim.expect("gated victim");
        // `SpawnBlood (org, vel*0.2, damage)` sends `particle (org,
        // vel*0.1, 73, damage*2)`, so the puff drifts at `vel * 0.02`.
        ctx.behaviors.temp_ents.push(Q1TempEnt::Blood {
            org,
            vel: vec3(vel.x * 0.02, vel.y * 0.02, vel.z * 0.02),
            count: (damage * 2.0) as f32,
        });
        q1_add_multi_damage(ctx, multi, firer, &victim, damage);
    } else {
        ctx.behaviors.temp_ents.push(Q1TempEnt::Gunshot { org });
    }
}

/// Stock `FireBullets` (`weapons.qc:236`): a shotgun burst from the
/// muzzle — one `MOVE_NORMAL` trace per pellet, each strike a
/// `TraceAttack`, all combining into one damage call per victim.
/// Monsters never set `v_angle`, so the spread axes come from zero
/// angles, exactly like stock.
pub fn q1_fire_bullets<L: ServerLogic>(
    ctx: &mut Q1MonsterCtx<'_, '_, '_, L>,
    actor: &ActorId,
    shotcount: u32,
    dir: Vec3,
    spread: Vec3,
) {
    let axes = angle_vectors(vec3(0.0, 0.0, 0.0));
    let body = ctx.server.simulation().body_state(actor);
    let Some(body) = body else {
        return;
    };
    let bounds = translated_body_bounds(&body);
    let src = vec3(
        body.origin.x + axes.forward.x * 10.0,
        body.origin.y + axes.forward.y * 10.0,
        bounds.min.z + (bounds.max.z - bounds.min.z) * 0.7,
    );
    let mut multi = Q1MultiDamage { ent: None, damage: 0.0 };
    for _ in 0..shotcount {
        // Stock draws the right jitter before the up jitter.
        let right_jitter = q1_monster_crandom(ctx.behaviors);
        let up_jitter = q1_monster_crandom(ctx.behaviors);
        let direction = vec3(
            dir.x + axes.right.x * right_jitter * spread.x + axes.up.x * up_jitter * spread.y,
            dir.y + axes.right.y * right_jitter * spread.x + axes.up.y * up_jitter * spread.y,
            dir.z + axes.right.z * right_jitter * spread.x + axes.up.z * up_jitter * spread.y,
        );
        let end = vec3(
            src.x + direction.x * 2048.0,
            src.y + direction.y * 2048.0,
            src.z + direction.z * 2048.0,
        );
        let trace = ctx.movement().services_mut().trace(actor, src, end, None);
        if trace.fraction != 1.0 {
            q1_trace_attack(ctx, &mut multi, actor, &trace, direction, axes.right, axes.up, 4.0);
        }
    }
    q1_apply_multi_damage(ctx, &mut multi, actor);
}

/// Stock `army_fire` (`soldier.qc`): face the enemy, bark the shotgun,
/// and fire four pellets at the dodge-lead point.
fn q1_army_fire<L: ServerLogic>(ctx: &mut Q1MonsterCtx<'_, '_, '_, L>, actor: &ActorId) {
    let enemy = ctx
        .behaviors
        .monsters
        .get(actor)
        .and_then(|monster| monster.enemy.clone());
    let Some(enemy) = enemy else {
        return;
    };
    q1_ai_face(ctx, actor);
    q1_monster_sound(
        ctx.behaviors,
        actor,
        Q1_CHAN_WEAPON,
        "soldier/sattck1.wav",
        1.0,
        Q1_ATTN_NORM,
    );
    let foe = ctx.server.simulation().body_state(&enemy);
    let me = ctx.server.simulation().body_state(actor);
    let (Some(foe), Some(me)) = (foe, me) else {
        return;
    };
    // Fire somewhat behind the player, so a dodging player is harder
    // to hit (`soldier.qc`).
    let aim = vec3(
        foe.origin.x - foe.velocity.x * 0.2,
        foe.origin.y - foe.velocity.y * 0.2,
        foe.origin.z - foe.velocity.z * 0.2,
    );
    let raw = vec3(aim.x - me.origin.x, aim.y - me.origin.y, aim.z - me.origin.z);
    let len = (raw.x * raw.x + raw.y * raw.y + raw.z * raw.z).sqrt();
    let dir = if len == 0.0 {
        vec3(0.0, 0.0, 0.0)
    } else {
        vec3(raw.x / len, raw.y / len, raw.z / len)
    };
    q1_fire_bullets(ctx, actor, 4, dir, vec3(0.1, 0.1, 0.0));
}

/// Stock `SUB_CheckRefire` (`subs.qc:306`): nightmares rewind one
/// attack sequence while the enemy stays visible (non-nightmares fall
/// through).
fn q1_sub_check_refire<L: ServerLogic>(ctx: &mut Q1MonsterCtx<'_, '_, '_, L>, actor: &ActorId, rewind: Q1MonsterThink) {
    if ctx.behaviors.skill != 3 {
        return;
    }
    if ctx.behaviors.monsters.get(actor).map_or(1, |monster| monster.cnt) == 1 {
        return;
    }
    let enemy = ctx
        .behaviors
        .monsters
        .get(actor)
        .and_then(|monster| monster.enemy.clone());
    let Some(enemy) = enemy else {
        return;
    };
    if !q1_visible(ctx, actor, &enemy) {
        return;
    }
    if let Some(monster) = ctx.behaviors.monsters.get_mut(actor) {
        monster.cnt = 1;
        monster.think = rewind;
    }
}

/// Stock `DropBackpack` (`items.qc:1332`): drop the carrier's ammo as a
/// tossing backpack 24 units under the carrier origin. Empty carriers
/// drop nothing; failed spawns drop, like gib chunks.
fn q1_drop_backpack<L: ServerLogic>(ctx: &mut Q1MonsterCtx<'_, '_, '_, L>, actor: &ActorId, ammo: Q1Ammo) {
    if ammo.shells + ammo.nails + ammo.rockets + ammo.cells == 0.0 {
        return;
    }
    let Some(at) = ctx.server.simulation().body_state(actor).map(|body| body.origin) else {
        return;
    };
    let at = vec3(at.x, at.y, at.z - 24.0);
    let velocity = vec3(
        -100.0 + q1_monster_random(ctx.behaviors) * 200.0,
        -100.0 + q1_monster_random(ctx.behaviors) * 200.0,
        300.0,
    );
    let fields = SpawnFields {
        classname: "q1:backpack".to_string(),
        origin: at,
        ..SpawnFields::default()
    };
    let actor = match ctx.server.spawn_entity(&fields) {
        Ok(actor) => actor,
        Err(_) => return,
    };
    if build_q1_backpack(ctx.server, ctx.behaviors, &actor, velocity, ammo).is_err() {
        let _ignored = ctx.server.simulation_mut().release(&actor);
    }
}

/// Stock `Dog_JumpTouch` (`dog.qc`): fast bodies wound what they hit;
/// landed dogs run again, floating dogs wait for the ground — unless
/// flagged grounded on an edge, which re-leaps them free.
fn q1_dog_jump_touch<L: ServerLogic>(ctx: &mut Q1MonsterCtx<'_, '_, '_, L>, actor: &ActorId, other: Option<&ActorId>) {
    if q1_health_of(ctx.server.simulation(), actor) <= 0.0 {
        return;
    }
    if let Some(other) = other {
        if q1_can_take_damage(ctx.server.simulation(), other) {
            let speed = ctx.server.simulation().body_state(actor).map_or(0.0, |body| {
                let v = body.velocity;
                (v.x * v.x + v.y * v.y + v.z * v.z).sqrt()
            });
            if speed > 300.0 {
                let damage = 10.0 + f64::from(q1_monster_random(ctx.behaviors)) * 10.0;
                let me = actor.clone();
                let victim = other.clone();
                let (simulation, movers, triggers) = ctx.server.simulation_movers_and_triggers_mut();
                q1_t_damage(
                    ctx.behaviors,
                    simulation,
                    movers,
                    triggers,
                    &victim,
                    Some(&me),
                    Some(&me),
                    damage,
                );
            }
        }
    }
    if !q1_check_bottom(ctx, actor) {
        let grounded = ctx
            .behaviors
            .monsters
            .get(actor)
            .is_some_and(|monster| monster.flags & Q1_FLAG_ONGROUND != 0);
        if grounded {
            // Only leapers re-leap (the dog always has `th_missile`).
            let kind = ctx.behaviors.monsters.get(actor).map(|monster| monster.kind);
            let Some(think) = kind.and_then(q1_th_missile) else {
                return;
            };
            if let Some(monster) = ctx.behaviors.monsters.get_mut(actor) {
                monster.touch = Q1MonsterTouch::None;
                monster.think = think;
                monster.nextthink = ctx.now + Q1_MONSTER_THINK_STEP;
            }
        }
        return;
    }
    if let Some(monster) = ctx.behaviors.monsters.get_mut(actor) {
        monster.touch = Q1MonsterTouch::None;
        monster.think = q1_th_run(monster.kind);
        monster.nextthink = ctx.now + Q1_MONSTER_THINK_STEP;
    }
}

/// One dog `$frame` body (`dog.qc`): idle barks, gait calls, the bite
/// stroke, the leap launch, and pain footwork.
fn q1_dog_frame<L: ServerLogic>(ctx: &mut Q1MonsterCtx<'_, '_, '_, L>, actor: &ActorId, seq: Q1MonsterSeq, index: u8) {
    match (seq, index) {
        (Q1MonsterSeq::DogStand, _) => q1_ai_stand(ctx, actor),
        (Q1MonsterSeq::DogWalk, 0) => {
            if q1_monster_random(ctx.behaviors) < 0.2 {
                q1_monster_sound(ctx.behaviors, actor, Q1_CHAN_VOICE, "dog/idle.wav", 1.0, Q1_ATTN_IDLE);
            }
            q1_ai_walk(ctx, actor, 8.0);
        }
        (Q1MonsterSeq::DogWalk, _) => q1_ai_walk(ctx, actor, 8.0),
        (Q1MonsterSeq::DogRun, 0) => {
            if q1_monster_random(ctx.behaviors) < 0.2 {
                q1_monster_sound(ctx.behaviors, actor, Q1_CHAN_VOICE, "dog/idle.wav", 1.0, Q1_ATTN_IDLE);
            }
            q1_ai_run(ctx, actor, 16.0);
        }
        (Q1MonsterSeq::DogRun, 1) | (Q1MonsterSeq::DogRun, 2) => q1_ai_run(ctx, actor, 32.0),
        (Q1MonsterSeq::DogRun, 3) => q1_ai_run(ctx, actor, 20.0),
        (Q1MonsterSeq::DogRun, 4) => q1_ai_run(ctx, actor, 64.0),
        (Q1MonsterSeq::DogRun, 5) => q1_ai_run(ctx, actor, 32.0),
        (Q1MonsterSeq::DogRun, 6) => q1_ai_run(ctx, actor, 16.0),
        (Q1MonsterSeq::DogRun, 7) | (Q1MonsterSeq::DogRun, 8) => q1_ai_run(ctx, actor, 32.0),
        (Q1MonsterSeq::DogRun, 9) => q1_ai_run(ctx, actor, 20.0),
        (Q1MonsterSeq::DogRun, 10) => q1_ai_run(ctx, actor, 64.0),
        (Q1MonsterSeq::DogRun, _) => q1_ai_run(ctx, actor, 32.0),
        (Q1MonsterSeq::DogAttack, 3) => {
            q1_monster_sound(
                ctx.behaviors,
                actor,
                Q1_CHAN_VOICE,
                "dog/dattack1.wav",
                1.0,
                Q1_ATTN_NORM,
            );
            q1_dog_bite(ctx, actor);
        }
        (Q1MonsterSeq::DogAttack, _) => q1_ai_charge(ctx, actor, 10.0),
        (Q1MonsterSeq::DogLeap, 0) => q1_ai_face(ctx, actor),
        (Q1MonsterSeq::DogLeap, 1) => {
            q1_ai_face(ctx, actor);
            if let Some(monster) = ctx.behaviors.monsters.get_mut(actor) {
                monster.touch = Q1MonsterTouch::JumpTouch;
            }
            if let Some(body) = ctx.server.simulation().body_state(actor) {
                let forward = angle_vectors(body.angles).forward;
                let _ignored = ctx
                    .server
                    .simulation_mut()
                    .set_body_origin(actor, vec3(body.origin.x, body.origin.y, body.origin.z + 1.0));
                let _ignored = ctx.server.simulation_mut().set_body_velocity(
                    actor,
                    vec3(forward.x * 300.0, forward.y * 300.0, forward.z * 300.0 + 200.0),
                );
            }
            if let Some(monster) = ctx.behaviors.monsters.get_mut(actor) {
                monster.flags &= !Q1_FLAG_ONGROUND;
            }
        }
        (Q1MonsterSeq::DogLeap, _) => {}
        (Q1MonsterSeq::DogPain, _) => {}
        (Q1MonsterSeq::DogPainB, 2) => q1_ai_pain(ctx, actor, 4.0),
        (Q1MonsterSeq::DogPainB, 3) | (Q1MonsterSeq::DogPainB, 4) => q1_ai_pain(ctx, actor, 12.0),
        (Q1MonsterSeq::DogPainB, 5) => q1_ai_pain(ctx, actor, 2.0),
        (Q1MonsterSeq::DogPainB, 7) => q1_ai_pain(ctx, actor, 4.0),
        (Q1MonsterSeq::DogPainB, 9) => q1_ai_pain(ctx, actor, 10.0),
        (Q1MonsterSeq::DogPainB, _) => {}
        (Q1MonsterSeq::DogDie, _) | (Q1MonsterSeq::DogDieB, _) => {}
        // Other kinds never dispatch here.
        _ => {}
    }
}

/// Grunt walk step distances (`army_walk1..24`, `soldier.qc`).
const GRUNT_WALK_STEPS: [f64; 24] = [
    1.0, 1.0, 1.0, 1.0, 2.0, 3.0, 4.0, 4.0, 2.0, 2.0, 2.0, 1.0, 0.0, 1.0, 1.0, 1.0, 3.0, 3.0, 3.0, 3.0, 2.0, 1.0, 1.0,
    1.0,
];

/// Grunt run step distances (`army_run1..8`, `soldier.qc`).
const GRUNT_RUN_STEPS: [f64; 8] = [11.0, 15.0, 10.0, 10.0, 8.0, 15.0, 10.0, 8.0];

/// Enforcer walk travel per frame (`enf_walk1..16`, `enforcer.qc`).
const ENFORCER_WALK_STEPS: [f64; 16] = [
    2.0, 4.0, 4.0, 3.0, 1.0, 2.0, 2.0, 1.0, 2.0, 4.0, 4.0, 1.0, 2.0, 3.0, 4.0, 2.0,
];

/// Enforcer run travel per frame (`enf_run1..8`, `enforcer.qc`).
const ENFORCER_RUN_STEPS: [f64; 8] = [18.0, 14.0, 7.0, 12.0, 14.0, 14.0, 7.0, 11.0];

/// Ogre walk travel per frame (`ogre_walk1..16`, `ogre.qc`).
const OGRE_WALK_STEPS: [f64; 16] = [
    3.0, 2.0, 2.0, 2.0, 2.0, 5.0, 3.0, 2.0, 3.0, 1.0, 2.0, 3.0, 3.0, 3.0, 3.0, 4.0,
];

/// Ogre run travel per frame (`ogre_run1..8`, `ogre.qc`).
const OGRE_RUN_STEPS: [f64; 8] = [9.0, 12.0, 8.0, 22.0, 16.0, 4.0, 13.0, 24.0];

/// Stock grunt death drop (`army_die3`/`army_cdie3`, `soldier.qc`):
/// unsolid, then the 5-shell backpack.
fn q1_grunt_death_drop<L: ServerLogic>(ctx: &mut Q1MonsterCtx<'_, '_, '_, L>, actor: &ActorId) {
    ctx.behaviors.solids.remove(actor);
    q1_drop_backpack(
        ctx,
        actor,
        Q1Ammo {
            shells: Q1_GRUNT_DROP_SHELLS,
            ..Q1Ammo::default()
        },
    );
}

/// One grunt `$frame` body (`soldier.qc`): idle barks, gait calls, the
/// shotgun stroke with its muzzle flash and nightmare refire, pain
/// footwork, and the death drops.
fn q1_grunt_frame<L: ServerLogic>(
    ctx: &mut Q1MonsterCtx<'_, '_, '_, L>,
    actor: &ActorId,
    seq: Q1MonsterSeq,
    index: u8,
) {
    match (seq, index) {
        (Q1MonsterSeq::GruntStand, _) => q1_ai_stand(ctx, actor),
        (Q1MonsterSeq::GruntWalk, 0) => {
            if q1_monster_random(ctx.behaviors) < 0.2 {
                q1_monster_sound(
                    ctx.behaviors,
                    actor,
                    Q1_CHAN_VOICE,
                    "soldier/idle.wav",
                    1.0,
                    Q1_ATTN_IDLE,
                );
            }
            q1_ai_walk(ctx, actor, GRUNT_WALK_STEPS[0]);
        }
        (Q1MonsterSeq::GruntWalk, i) => {
            q1_ai_walk(ctx, actor, GRUNT_WALK_STEPS.get(usize::from(i)).copied().unwrap_or(0.0));
        }
        (Q1MonsterSeq::GruntRun, 0) => {
            if q1_monster_random(ctx.behaviors) < 0.2 {
                q1_monster_sound(
                    ctx.behaviors,
                    actor,
                    Q1_CHAN_VOICE,
                    "soldier/idle.wav",
                    1.0,
                    Q1_ATTN_IDLE,
                );
            }
            q1_ai_run(ctx, actor, GRUNT_RUN_STEPS[0]);
        }
        (Q1MonsterSeq::GruntRun, i) => {
            q1_ai_run(ctx, actor, GRUNT_RUN_STEPS.get(usize::from(i)).copied().unwrap_or(0.0));
        }
        (Q1MonsterSeq::GruntAttack, 4) => {
            q1_ai_face(ctx, actor);
            q1_army_fire(ctx, actor);
            if let Some(monster) = ctx.behaviors.monsters.get_mut(actor) {
                monster.effects |= Q1_EF_MUZZLEFLASH;
            }
        }
        (Q1MonsterSeq::GruntAttack, 6) => {
            q1_ai_face(ctx, actor);
            if let Some(rewind) = q1_th_missile(Q1MonsterKind::Grunt) {
                q1_sub_check_refire(ctx, actor, rewind);
            }
        }
        (Q1MonsterSeq::GruntAttack, _) => q1_ai_face(ctx, actor),
        (Q1MonsterSeq::GruntPain, _) => {}
        (Q1MonsterSeq::GruntPainB, 1) => q1_ai_painforward(ctx, actor, 13.0),
        (Q1MonsterSeq::GruntPainB, 2) => q1_ai_painforward(ctx, actor, 9.0),
        (Q1MonsterSeq::GruntPainB, 11) => q1_ai_pain(ctx, actor, 2.0),
        (Q1MonsterSeq::GruntPainB, _) => {}
        (Q1MonsterSeq::GruntPainC, 1) => q1_ai_pain(ctx, actor, 1.0),
        (Q1MonsterSeq::GruntPainC, 4) | (Q1MonsterSeq::GruntPainC, 5) => q1_ai_painforward(ctx, actor, 1.0),
        (Q1MonsterSeq::GruntPainC, 7) => q1_ai_pain(ctx, actor, 1.0),
        (Q1MonsterSeq::GruntPainC, 8) => q1_ai_painforward(ctx, actor, 4.0),
        (Q1MonsterSeq::GruntPainC, 9) => q1_ai_painforward(ctx, actor, 3.0),
        (Q1MonsterSeq::GruntPainC, 10) => q1_ai_painforward(ctx, actor, 6.0),
        (Q1MonsterSeq::GruntPainC, 11) => q1_ai_painforward(ctx, actor, 8.0),
        (Q1MonsterSeq::GruntPainC, _) => {}
        (Q1MonsterSeq::GruntDie, 2) => q1_grunt_death_drop(ctx, actor),
        (Q1MonsterSeq::GruntDie, _) => {}
        (Q1MonsterSeq::GruntDieC, 1) => q1_ai_back(ctx, actor, 5.0),
        (Q1MonsterSeq::GruntDieC, 2) => {
            q1_grunt_death_drop(ctx, actor);
            q1_ai_back(ctx, actor, 4.0);
        }
        (Q1MonsterSeq::GruntDieC, 3) => q1_ai_back(ctx, actor, 13.0),
        (Q1MonsterSeq::GruntDieC, 4) => q1_ai_back(ctx, actor, 3.0),
        (Q1MonsterSeq::GruntDieC, 5) => q1_ai_back(ctx, actor, 4.0),
        (Q1MonsterSeq::GruntDieC, _) => {}
        // Other kinds never dispatch here.
        _ => {}
    }
}

/// Stock enforcer death drop (`enf_die3`/`enf_fdie3`, `enforcer.qc`):
/// unsolid, then the 5-cell backpack.
fn q1_enforcer_death_drop<L: ServerLogic>(ctx: &mut Q1MonsterCtx<'_, '_, '_, L>, actor: &ActorId) {
    ctx.behaviors.solids.remove(actor);
    q1_drop_backpack(
        ctx,
        actor,
        Q1Ammo {
            cells: Q1_ENFORCER_DROP_CELLS,
            ..Q1Ammo::default()
        },
    );
}

/// One enforcer `$frame` body (`enforcer.qc`): idle barks, gait calls,
/// the two-bolt volley with its muzzle flashes and nightmare refire,
/// pain footwork, and the death drops. The firing frames launch bolts
/// without facing (`enf_atk6`, `enf_atk10`).
fn q1_enforcer_frame<L: ServerLogic>(
    ctx: &mut Q1MonsterCtx<'_, '_, '_, L>,
    actor: &ActorId,
    seq: Q1MonsterSeq,
    index: u8,
) {
    match (seq, index) {
        (Q1MonsterSeq::EnforcerStand, _) => q1_ai_stand(ctx, actor),
        (Q1MonsterSeq::EnforcerWalk, 0) => {
            if q1_monster_random(ctx.behaviors) < 0.2 {
                q1_monster_sound(
                    ctx.behaviors,
                    actor,
                    Q1_CHAN_VOICE,
                    "enforcer/idle1.wav",
                    1.0,
                    Q1_ATTN_IDLE,
                );
            }
            q1_ai_walk(ctx, actor, ENFORCER_WALK_STEPS[0]);
        }
        (Q1MonsterSeq::EnforcerWalk, i) => {
            q1_ai_walk(
                ctx,
                actor,
                ENFORCER_WALK_STEPS.get(usize::from(i)).copied().unwrap_or(0.0),
            );
        }
        (Q1MonsterSeq::EnforcerRun, 0) => {
            if q1_monster_random(ctx.behaviors) < 0.2 {
                q1_monster_sound(
                    ctx.behaviors,
                    actor,
                    Q1_CHAN_VOICE,
                    "enforcer/idle1.wav",
                    1.0,
                    Q1_ATTN_IDLE,
                );
            }
            q1_ai_run(ctx, actor, ENFORCER_RUN_STEPS[0]);
        }
        (Q1MonsterSeq::EnforcerRun, i) => {
            q1_ai_run(
                ctx,
                actor,
                ENFORCER_RUN_STEPS.get(usize::from(i)).copied().unwrap_or(0.0),
            );
        }
        (Q1MonsterSeq::EnforcerAttack, 5) | (Q1MonsterSeq::EnforcerAttack, 9) => {
            q1_enforcer_fire(ctx, actor);
        }
        (Q1MonsterSeq::EnforcerAttack, 13) => {
            q1_ai_face(ctx, actor);
            if let Some(rewind) = q1_th_missile(Q1MonsterKind::Enforcer) {
                q1_sub_check_refire(ctx, actor, rewind);
            }
        }
        (Q1MonsterSeq::EnforcerAttack, _) => q1_ai_face(ctx, actor),
        (Q1MonsterSeq::EnforcerPainA, _) | (Q1MonsterSeq::EnforcerPainB, _) | (Q1MonsterSeq::EnforcerPainC, _) => {}
        (Q1MonsterSeq::EnforcerPainD, 3) => q1_ai_painforward(ctx, actor, 2.0),
        (Q1MonsterSeq::EnforcerPainD, 4)
        | (Q1MonsterSeq::EnforcerPainD, 10)
        | (Q1MonsterSeq::EnforcerPainD, 11)
        | (Q1MonsterSeq::EnforcerPainD, 12) => q1_ai_painforward(ctx, actor, 1.0),
        (Q1MonsterSeq::EnforcerPainD, 15) | (Q1MonsterSeq::EnforcerPainD, 16) => q1_ai_pain(ctx, actor, 1.0),
        (Q1MonsterSeq::EnforcerPainD, _) => {}
        (Q1MonsterSeq::EnforcerDie, 2) => q1_enforcer_death_drop(ctx, actor),
        (Q1MonsterSeq::EnforcerDie, 3) => q1_ai_forward(ctx, actor, 14.0),
        (Q1MonsterSeq::EnforcerDie, 4) => q1_ai_forward(ctx, actor, 2.0),
        (Q1MonsterSeq::EnforcerDie, 8) => q1_ai_forward(ctx, actor, 3.0),
        (Q1MonsterSeq::EnforcerDie, 9) | (Q1MonsterSeq::EnforcerDie, 10) | (Q1MonsterSeq::EnforcerDie, 11) => {
            q1_ai_forward(ctx, actor, 5.0)
        }
        (Q1MonsterSeq::EnforcerDie, _) => {}
        (Q1MonsterSeq::EnforcerFDie, 2) => q1_enforcer_death_drop(ctx, actor),
        (Q1MonsterSeq::EnforcerFDie, _) => {}
        // Other kinds never dispatch here.
        _ => {}
    }
}

/// Stock `enforcer_fire` (`enforcer.qc:104`): flash the muzzle and
/// launch a bolt from the gun tip at the enemy origin.
fn q1_enforcer_fire<L: ServerLogic>(ctx: &mut Q1MonsterCtx<'_, '_, '_, L>, actor: &ActorId) {
    let enemy = ctx
        .behaviors
        .monsters
        .get(actor)
        .and_then(|monster| monster.enemy.clone());
    let Some(enemy) = enemy else {
        return;
    };
    if let Some(monster) = ctx.behaviors.monsters.get_mut(actor) {
        monster.effects |= Q1_EF_MUZZLEFLASH;
    }
    let me = ctx.server.simulation().body_state(actor);
    let foe = ctx.server.simulation().body_state(&enemy);
    let (Some(me), Some(foe)) = (me, foe) else {
        return;
    };
    let axes = angle_vectors(me.angles);
    let org = vec3(
        me.origin.x + axes.forward.x * 30.0 + axes.right.x * 8.5,
        me.origin.y + axes.forward.y * 30.0 + axes.right.y * 8.5,
        me.origin.z + axes.forward.z * 30.0 + axes.right.z * 8.5 + 16.0,
    );
    let dir = vec3(
        foe.origin.x - me.origin.x,
        foe.origin.y - me.origin.y,
        foe.origin.z - me.origin.z,
    );
    q1_launch_laser(ctx, actor, org, dir);
}

/// Stock `vectoangles` (`pr_cmds.c:428`): integer pitch/yaw in degrees,
/// wrapped to 0-360, roll zero. Vertical vectors yaw zero and pitch
/// straight up (90) or down (270).
fn q1_vectoangles(vec: Vec3) -> Vec3 {
    if vec.x == 0.0 && vec.y == 0.0 {
        let pitch = if vec.z > 0.0 { 90.0 } else { 270.0 };
        return vec3(pitch, 0.0, 0.0);
    }
    let mut yaw = (vec.y.atan2(vec.x) * 180.0 / std::f32::consts::PI).trunc();
    if yaw < 0.0 {
        yaw += 360.0;
    }
    let forward = (vec.x * vec.x + vec.y * vec.y).sqrt();
    let mut pitch = (vec.z.atan2(forward) * 180.0 / std::f32::consts::PI).trunc();
    if pitch < 0.0 {
        pitch += 360.0;
    }
    vec3(pitch, yaw, 0.0)
}

/// Stock `LaunchLaser` (`enforcer.qc:74`): bark the shot, then spawn a
/// point bolt with a dim light, aimed along `dir` at 600 u/s, gone in
/// 5 s.
fn q1_launch_laser<L: ServerLogic>(ctx: &mut Q1MonsterCtx<'_, '_, '_, L>, owner: &ActorId, org: Vec3, dir: Vec3) {
    q1_monster_sound(
        ctx.behaviors,
        owner,
        Q1_CHAN_WEAPON,
        "enforcer/enfire.wav",
        1.0,
        Q1_ATTN_NORM,
    );
    let len = (dir.x * dir.x + dir.y * dir.y + dir.z * dir.z).sqrt();
    let unit = if len == 0.0 {
        vec3(0.0, 0.0, 0.0)
    } else {
        vec3(dir.x / len, dir.y / len, dir.z / len)
    };
    let velocity = vec3(
        unit.x * Q1_LASER_SPEED,
        unit.y * Q1_LASER_SPEED,
        unit.z * Q1_LASER_SPEED,
    );
    let fields = SpawnFields {
        classname: "q1:laser".to_string(),
        origin: org,
        ..SpawnFields::default()
    };
    let bolt = match ctx.server.spawn_entity(&fields) {
        Ok(bolt) => bolt,
        Err(_) => return,
    };
    let point = Bounds {
        min: vec3(0.0, 0.0, 0.0),
        max: vec3(0.0, 0.0, 0.0),
    };
    let simulation = ctx.server.simulation_mut();
    if simulation.set_body_bounds(bolt.id(), point).is_err()
        || simulation.set_body_velocity(bolt.id(), velocity).is_err()
        || simulation.set_body_angles(bolt.id(), q1_vectoangles(velocity)).is_err()
    {
        let _ignored = simulation.release(&bolt);
        return;
    }
    let _ignored = simulation.link_body(bolt.id());
    ctx.behaviors.solids.insert(bolt.id());
    ctx.behaviors.projectiles.insert(
        bolt.id(),
        Q1Projectile {
            kind: Q1ProjectileKind::Laser,
            owner: Some(owner.clone()),
            remove_at: ctx.now + Q1_LASER_LIFETIME,
            effects: Q1_EF_DIMLIGHT,
            avelocity: vec3(0.0, 0.0, 0.0),
        },
    );
}

/// Stock `Laser_Touch` (`enforcer.qc:41`): owner touches pass through;
/// sky bolts vanish silently; anything else stops the bolt with a
/// crack — blood and 15 damage for the living, a wall puff for the
/// rest — then the bolt removes.
fn q1_laser_touch<L: ServerLogic>(ctx: &mut Q1MonsterCtx<'_, '_, '_, L>, actor: &ActorId, other: Option<&ActorId>) {
    let projectile = ctx.behaviors.projectiles.get(actor).cloned();
    let Some(projectile) = projectile else {
        return;
    };
    if projectile.owner.as_ref() == other {
        return;
    }
    let Some(body) = ctx.server.simulation().body_state(actor) else {
        return;
    };
    if ctx.movement().services_mut().point_contents(actor, body.origin) == Q1_CONTENTS_SKY {
        let (simulation, movers, triggers) = ctx.server.simulation_movers_and_triggers_mut();
        q1_remove(ctx.behaviors, simulation, movers, triggers, actor);
        return;
    }
    q1_monster_sound(
        ctx.behaviors,
        actor,
        Q1_CHAN_WEAPON,
        "enforcer/enfstop.wav",
        1.0,
        Q1_ATTN_STATIC,
    );
    let speed =
        (body.velocity.x * body.velocity.x + body.velocity.y * body.velocity.y + body.velocity.z * body.velocity.z)
            .sqrt();
    let unit = if speed == 0.0 {
        vec3(0.0, 0.0, 0.0)
    } else {
        vec3(
            body.velocity.x / speed,
            body.velocity.y / speed,
            body.velocity.z / speed,
        )
    };
    let org = vec3(
        body.origin.x - 8.0 * unit.x,
        body.origin.y - 8.0 * unit.y,
        body.origin.z - 8.0 * unit.z,
    );
    let living = other.is_some_and(|other| q1_can_take_damage(ctx.server.simulation(), other));
    if living {
        let victim = other.expect("gated victim").clone();
        // `SpawnBlood (org, vel*0.2, 15)` sends `particle (org,
        // vel*0.02, 73, 30)` (`weapons.qc:120`).
        ctx.behaviors.temp_ents.push(Q1TempEnt::Blood {
            org,
            vel: vec3(body.velocity.x * 0.02, body.velocity.y * 0.02, body.velocity.z * 0.02),
            count: (Q1_LASER_DAMAGE * 2.0) as f32,
        });
        let me = actor.clone();
        let owner = projectile.owner.clone();
        let (simulation, movers, triggers) = ctx.server.simulation_movers_and_triggers_mut();
        q1_t_damage(
            ctx.behaviors,
            simulation,
            movers,
            triggers,
            &victim,
            Some(&me),
            owner.as_ref(),
            Q1_LASER_DAMAGE,
        );
    } else {
        ctx.behaviors.temp_ents.push(Q1TempEnt::Gunshot { org });
    }
    let (simulation, movers, triggers) = ctx.server.simulation_movers_and_triggers_mut();
    q1_remove(ctx.behaviors, simulation, movers, triggers, actor);
}

/// Stock `SV_Physics_Fly` core (`sv_phys.c`): flyers keep velocity
/// (no gravity), travel to the trace end, link, and touch whatever
/// stopped them.
fn q1_fly_physics<L: ServerLogic>(ctx: &mut Q1MonsterCtx<'_, '_, '_, L>, actor: &ActorId) {
    if ctx.dt <= 0.0 {
        return;
    }
    let Some(body) = ctx.server.simulation().body_state(actor) else {
        return;
    };
    let end = vec3(
        body.origin.x + body.velocity.x * ctx.dt as f32,
        body.origin.y + body.velocity.y * ctx.dt as f32,
        body.origin.z + body.velocity.z * ctx.dt as f32,
    );
    let trace = ctx.movement().services_mut().trace(actor, body.origin, end, None);
    // Stock `SV_PushEntity`: land at the end position, link, and touch
    // whatever stopped the flight.
    let _ignored = ctx.server.simulation_mut().set_body_origin(actor, trace.end);
    let _ignored = ctx.server.simulation_mut().link_body(actor);
    if trace.fraction == 1.0 {
        return;
    }
    let other = match &trace.hit {
        TraceHit::Actor { actor } => Some(actor.clone()),
        _ => None,
    };
    q1_laser_touch(ctx, actor, other.as_ref());
}

/// Step one projectile actor: bolts remove on schedule (`SUB_Remove`)
/// and fly while live; grenades explode on schedule
/// (`OgreGrenadeExplode`) and bounce while live.
fn q1_projectile_actor<L: ServerLogic>(ctx: &mut Q1MonsterCtx<'_, '_, '_, L>, actor: &ActorId) {
    let Some(projectile) = ctx.behaviors.projectiles.get(actor).cloned() else {
        return;
    };
    match projectile.kind {
        Q1ProjectileKind::Laser => {
            if projectile.remove_at <= ctx.now {
                let (simulation, movers, triggers) = ctx.server.simulation_movers_and_triggers_mut();
                q1_remove(ctx.behaviors, simulation, movers, triggers, actor);
                return;
            }
            q1_fly_physics(ctx, actor);
        }
        Q1ProjectileKind::Grenade => {
            if projectile.remove_at <= ctx.now {
                q1_ogre_grenade_explode(ctx, actor);
                return;
            }
            q1_grenade_physics(ctx, actor);
        }
    }
}

/// Stock ogre death drop (`ogre_die3`/`ogre_bdie3`, `ogre.qc`):
/// unsolid, then the 2-rocket backpack.
fn q1_ogre_death_drop<L: ServerLogic>(ctx: &mut Q1MonsterCtx<'_, '_, '_, L>, actor: &ActorId) {
    ctx.behaviors.solids.remove(actor);
    q1_drop_backpack(
        ctx,
        actor,
        Q1Ammo {
            rockets: Q1_OGRE_DROP_ROCKETS,
            ..Q1Ammo::default()
        },
    );
}

/// Stock `chainsaw` (`ogre.qc:136`): charge the enemy, then rip nearby
/// flesh in the open for `(r+r+r)*4`, spraying meat on side strokes.
fn q1_chainsaw<L: ServerLogic>(ctx: &mut Q1MonsterCtx<'_, '_, '_, L>, actor: &ActorId, side: f64) {
    let enemy = ctx
        .behaviors
        .monsters
        .get(actor)
        .and_then(|monster| monster.enemy.clone());
    let Some(enemy) = enemy else {
        return;
    };
    if !q1_can_damage(ctx, &enemy, actor) {
        return;
    }
    q1_ai_charge(ctx, actor, 10.0);
    let from = ctx.server.simulation().body_state(actor).map(|body| body.origin);
    let to = ctx.server.simulation().body_state(&enemy).map(|body| body.origin);
    let (Some(from), Some(to)) = (from, to) else {
        return;
    };
    let delta = vec3(from.x - to.x, from.y - to.y, from.z - to.z);
    if (delta.x * delta.x + delta.y * delta.y + delta.z * delta.z).sqrt() > Q1_CHAINSAW_RANGE {
        return;
    }
    let damage = f64::from(
        q1_monster_random(ctx.behaviors) + q1_monster_random(ctx.behaviors) + q1_monster_random(ctx.behaviors),
    ) * 4.0;
    let me = actor.clone();
    let (simulation, movers, triggers) = ctx.server.simulation_movers_and_triggers_mut();
    q1_t_damage(
        ctx.behaviors,
        simulation,
        movers,
        triggers,
        &enemy,
        Some(&me),
        Some(&me),
        damage,
    );
    if side != 0.0 {
        let Some(body) = ctx.server.simulation().body_state(actor) else {
            return;
        };
        let axes = angle_vectors(body.angles);
        let org = vec3(
            body.origin.x + axes.forward.x * 16.0,
            body.origin.y + axes.forward.y * 16.0,
            body.origin.z + axes.forward.z * 16.0,
        );
        let vel = if side == 1.0 {
            let spray = q1_monster_crandom(ctx.behaviors) * 100.0;
            vec3(axes.right.x * spray, axes.right.y * spray, axes.right.z * spray)
        } else {
            let push = side as f32;
            vec3(axes.right.x * push, axes.right.y * push, axes.right.z * push)
        };
        q1_spawn_meat_spray(ctx, org, vel);
    }
}

/// Stock `SpawnMeatSpray` (`weapons.qc:89`): queue a bouncing zom-gib
/// chunk, kicked skyward, gone in 1 s.
fn q1_spawn_meat_spray<L: ServerLogic>(ctx: &mut Q1MonsterCtx<'_, '_, '_, L>, org: Vec3, vel: Vec3) {
    let up = 250.0 + 50.0 * q1_monster_random(ctx.behaviors);
    ctx.behaviors.pending_gibs.push(Q1PendingGib {
        model: "progs/zom_gib.mdl".to_string(),
        at: org,
        velocity: vec3(vel.x, vel.y, vel.z + up),
        avelocity: vec3(3000.0, 1000.0, 2000.0),
        remove_at: ctx.now + 1.0,
    });
}

/// One ogre `$frame` body (`ogre.qc`): idle barks, gait calls, the
/// swing and smash strokes with their yaw wobble, the grenade lob,
/// pain footwork, and the death drops.
fn q1_ogre_frame<L: ServerLogic>(ctx: &mut Q1MonsterCtx<'_, '_, '_, L>, actor: &ActorId, seq: Q1MonsterSeq, index: u8) {
    match (seq, index) {
        (Q1MonsterSeq::OgreStand, 4) => {
            if q1_monster_random(ctx.behaviors) < 0.2 {
                q1_monster_sound(
                    ctx.behaviors,
                    actor,
                    Q1_CHAN_VOICE,
                    "ogre/ogidle.wav",
                    1.0,
                    Q1_ATTN_IDLE,
                );
            }
            q1_ai_stand(ctx, actor);
        }
        (Q1MonsterSeq::OgreStand, _) => q1_ai_stand(ctx, actor),
        (Q1MonsterSeq::OgreWalk, 2) => {
            q1_ai_walk(ctx, actor, OGRE_WALK_STEPS[2]);
            if q1_monster_random(ctx.behaviors) < 0.2 {
                q1_monster_sound(
                    ctx.behaviors,
                    actor,
                    Q1_CHAN_VOICE,
                    "ogre/ogidle.wav",
                    1.0,
                    Q1_ATTN_IDLE,
                );
            }
        }
        (Q1MonsterSeq::OgreWalk, 5) => {
            q1_ai_walk(ctx, actor, OGRE_WALK_STEPS[5]);
            if q1_monster_random(ctx.behaviors) < 0.1 {
                q1_monster_sound(
                    ctx.behaviors,
                    actor,
                    Q1_CHAN_VOICE,
                    "ogre/ogdrag.wav",
                    1.0,
                    Q1_ATTN_IDLE,
                );
            }
        }
        (Q1MonsterSeq::OgreWalk, i) => {
            q1_ai_walk(ctx, actor, OGRE_WALK_STEPS.get(usize::from(i)).copied().unwrap_or(0.0));
        }
        (Q1MonsterSeq::OgreRun, 0) => {
            q1_ai_run(ctx, actor, OGRE_RUN_STEPS[0]);
            if q1_monster_random(ctx.behaviors) < 0.2 {
                q1_monster_sound(
                    ctx.behaviors,
                    actor,
                    Q1_CHAN_VOICE,
                    "ogre/ogidle2.wav",
                    1.0,
                    Q1_ATTN_IDLE,
                );
            }
        }
        (Q1MonsterSeq::OgreRun, i) => {
            q1_ai_run(ctx, actor, OGRE_RUN_STEPS.get(usize::from(i)).copied().unwrap_or(0.0));
        }
        (Q1MonsterSeq::OgreSwing, 0) => {
            q1_ai_charge(ctx, actor, 11.0);
            q1_monster_sound(
                ctx.behaviors,
                actor,
                Q1_CHAN_WEAPON,
                "ogre/ogsawatk.wav",
                1.0,
                Q1_ATTN_NORM,
            );
        }
        (Q1MonsterSeq::OgreSwing, 1) => q1_ai_charge(ctx, actor, 1.0),
        (Q1MonsterSeq::OgreSwing, 2) => q1_ai_charge(ctx, actor, 4.0),
        (Q1MonsterSeq::OgreSwing, 3) => q1_ai_charge(ctx, actor, 13.0),
        (Q1MonsterSeq::OgreSwing, 4) => {
            q1_ai_charge(ctx, actor, 9.0);
            q1_chainsaw(ctx, actor, 0.0);
            q1_ogre_swing_yaw(ctx, actor);
        }
        (Q1MonsterSeq::OgreSwing, 5) => {
            q1_chainsaw(ctx, actor, 200.0);
            q1_ogre_swing_yaw(ctx, actor);
        }
        (Q1MonsterSeq::OgreSwing, 6) | (Q1MonsterSeq::OgreSwing, 7) | (Q1MonsterSeq::OgreSwing, 8) => {
            q1_chainsaw(ctx, actor, 0.0);
            q1_ogre_swing_yaw(ctx, actor);
        }
        (Q1MonsterSeq::OgreSwing, 9) => {
            q1_chainsaw(ctx, actor, -200.0);
            q1_ogre_swing_yaw(ctx, actor);
        }
        (Q1MonsterSeq::OgreSwing, 10) => {
            q1_chainsaw(ctx, actor, 0.0);
            q1_ogre_swing_yaw(ctx, actor);
        }
        (Q1MonsterSeq::OgreSwing, 11) => q1_ai_charge(ctx, actor, 3.0),
        (Q1MonsterSeq::OgreSwing, 12) => q1_ai_charge(ctx, actor, 8.0),
        (Q1MonsterSeq::OgreSwing, 13) => q1_ai_charge(ctx, actor, 9.0),
        (Q1MonsterSeq::OgreSmash, 0) => {
            q1_ai_charge(ctx, actor, 6.0);
            q1_monster_sound(
                ctx.behaviors,
                actor,
                Q1_CHAN_WEAPON,
                "ogre/ogsawatk.wav",
                1.0,
                Q1_ATTN_NORM,
            );
        }
        (Q1MonsterSeq::OgreSmash, 1) | (Q1MonsterSeq::OgreSmash, 2) => q1_ai_charge(ctx, actor, 0.0),
        (Q1MonsterSeq::OgreSmash, 3) => q1_ai_charge(ctx, actor, 1.0),
        (Q1MonsterSeq::OgreSmash, 4) => q1_ai_charge(ctx, actor, 4.0),
        (Q1MonsterSeq::OgreSmash, 5) | (Q1MonsterSeq::OgreSmash, 6) => {
            q1_ai_charge(ctx, actor, 4.0);
            q1_chainsaw(ctx, actor, 0.0);
        }
        (Q1MonsterSeq::OgreSmash, 7) => {
            q1_ai_charge(ctx, actor, 10.0);
            q1_chainsaw(ctx, actor, 0.0);
        }
        (Q1MonsterSeq::OgreSmash, 8) => {
            q1_ai_charge(ctx, actor, 13.0);
            q1_chainsaw(ctx, actor, 0.0);
        }
        (Q1MonsterSeq::OgreSmash, 9) => q1_chainsaw(ctx, actor, 1.0),
        (Q1MonsterSeq::OgreSmash, 10) => {
            q1_ai_charge(ctx, actor, 2.0);
            q1_chainsaw(ctx, actor, 0.0);
            // Slight variation (`ogre_smash11`, `ogre.qc`).
            let wobble = f64::from(q1_monster_random(ctx.behaviors)) * 0.2;
            if let Some(monster) = ctx.behaviors.monsters.get_mut(actor) {
                monster.nextthink += wobble;
            }
        }
        (Q1MonsterSeq::OgreSmash, 11) => q1_ai_charge(ctx, actor, 0.0),
        (Q1MonsterSeq::OgreSmash, 12) => q1_ai_charge(ctx, actor, 4.0),
        (Q1MonsterSeq::OgreSmash, 13) => q1_ai_charge(ctx, actor, 12.0),
        (Q1MonsterSeq::OgreNail, 3) => {
            q1_ai_face(ctx, actor);
            q1_ogre_fire_grenade(ctx, actor);
        }
        (Q1MonsterSeq::OgreNail, _) => q1_ai_face(ctx, actor),
        (Q1MonsterSeq::OgrePain, _) | (Q1MonsterSeq::OgrePainB, _) | (Q1MonsterSeq::OgrePainC, _) => {}
        (Q1MonsterSeq::OgrePainD, 1) | (Q1MonsterSeq::OgrePainE, 1) => q1_ai_pain(ctx, actor, 10.0),
        (Q1MonsterSeq::OgrePainD, 2) | (Q1MonsterSeq::OgrePainE, 2) => q1_ai_pain(ctx, actor, 9.0),
        (Q1MonsterSeq::OgrePainD, 3) | (Q1MonsterSeq::OgrePainE, 3) => q1_ai_pain(ctx, actor, 4.0),
        (Q1MonsterSeq::OgrePainD, _) | (Q1MonsterSeq::OgrePainE, _) => {}
        (Q1MonsterSeq::OgreDie, 2) => q1_ogre_death_drop(ctx, actor),
        (Q1MonsterSeq::OgreDie, _) => {}
        (Q1MonsterSeq::OgreBDie, 1) => q1_ai_forward(ctx, actor, 5.0),
        (Q1MonsterSeq::OgreBDie, 2) => q1_ogre_death_drop(ctx, actor),
        (Q1MonsterSeq::OgreBDie, 3) => q1_ai_forward(ctx, actor, 1.0),
        (Q1MonsterSeq::OgreBDie, 4) => q1_ai_forward(ctx, actor, 3.0),
        (Q1MonsterSeq::OgreBDie, 5) => q1_ai_forward(ctx, actor, 7.0),
        (Q1MonsterSeq::OgreBDie, 6) => q1_ai_forward(ctx, actor, 25.0),
        (Q1MonsterSeq::OgreBDie, _) => {}
        // Other kinds never dispatch here.
        _ => {}
    }
}

/// Stock `T_RadiusDamage` (`combat.qc:212`): wound every solid
/// damageable within `damage + 40` of the inflictor, falling off half
/// a point per unit from the victim center, halved on the attacker,
/// gated on a damage line. The sim's damageables are the player, the
/// monsters, and the shootable buttons. (The shambler half-damage
/// exception lands with its slice.)
pub fn q1_radius_damage<L: ServerLogic>(
    ctx: &mut Q1MonsterCtx<'_, '_, '_, L>,
    inflictor: &ActorId,
    attacker: Option<&ActorId>,
    damage: f64,
    ignore: Option<&ActorId>,
) {
    let Some(from) = ctx.server.simulation().body_state(inflictor).map(|body| body.origin) else {
        return;
    };
    let radius = (damage + 40.0) as f32;
    let mut victims: Vec<ActorId> = Vec::new();
    if let Some(player) = ctx.behaviors.player.clone() {
        victims.push(player);
    }
    victims.extend(ctx.behaviors.monsters.keys().cloned());
    victims.extend(ctx.behaviors.buttons.keys().cloned());
    for victim in victims {
        if Some(&victim) == ignore {
            continue;
        }
        // Stock `findradius` skips the unsolid.
        if !ctx.behaviors.solids.contains(&victim) {
            continue;
        }
        if !q1_can_take_damage(ctx.server.simulation(), &victim) {
            continue;
        }
        let Some(body) = ctx.server.simulation().body_state(&victim) else {
            continue;
        };
        let bounds = translated_body_bounds(&body);
        let center = vec3(
            (bounds.min.x + bounds.max.x) / 2.0,
            (bounds.min.y + bounds.max.y) / 2.0,
            (bounds.min.z + bounds.max.z) / 2.0,
        );
        let delta = vec3(from.x - center.x, from.y - center.y, from.z - center.z);
        let dist = (delta.x * delta.x + delta.y * delta.y + delta.z * delta.z).sqrt();
        if dist > radius {
            continue;
        }
        let mut points = damage - 0.5 * f64::from(dist);
        if points < 0.0 {
            points = 0.0;
        }
        if Some(&victim) == attacker {
            points *= 0.5;
        }
        if points <= 0.0 {
            continue;
        }
        if !q1_can_damage(ctx, &victim, inflictor) {
            continue;
        }
        let me = inflictor.clone();
        let (simulation, movers, triggers) = ctx.server.simulation_movers_and_triggers_mut();
        q1_t_damage(
            ctx.behaviors,
            simulation,
            movers,
            triggers,
            &victim,
            Some(&me),
            attacker,
            points,
        );
    }
}

/// Whether stock would read `DAMAGE_AIM` off an entity: players always
/// aim, monsters read their armed record (`OgreGrenadeTouch`, `ogre.qc:75`).
fn q1_takedamage_aim<L: ServerLogic>(ctx: &Q1MonsterCtx<'_, '_, '_, L>, actor: &ActorId) -> bool {
    if Some(actor) == ctx.behaviors.player.as_ref() {
        return true;
    }
    ctx.behaviors
        .monsters
        .get(actor)
        .is_some_and(|monster| monster.takedamage == Q1_DAMAGE_AIM)
}

/// Stock `OgreFireGrenade` (`ogre.qc:90`): flash the muzzle, bark, and
/// lob a tumbling grenade from the feet at the enemy, fused 2.5 s.
fn q1_ogre_fire_grenade<L: ServerLogic>(ctx: &mut Q1MonsterCtx<'_, '_, '_, L>, actor: &ActorId) {
    let enemy = ctx
        .behaviors
        .monsters
        .get(actor)
        .and_then(|monster| monster.enemy.clone());
    let Some(enemy) = enemy else {
        return;
    };
    if let Some(monster) = ctx.behaviors.monsters.get_mut(actor) {
        monster.effects |= Q1_EF_MUZZLEFLASH;
    }
    q1_monster_sound(
        ctx.behaviors,
        actor,
        Q1_CHAN_WEAPON,
        "weapons/grenade.wav",
        1.0,
        Q1_ATTN_NORM,
    );
    let me = ctx.server.simulation().body_state(actor);
    let foe = ctx.server.simulation().body_state(&enemy);
    let (Some(me), Some(foe)) = (me, foe) else {
        return;
    };
    let raw = vec3(
        foe.origin.x - me.origin.x,
        foe.origin.y - me.origin.y,
        foe.origin.z - me.origin.z,
    );
    let len = (raw.x * raw.x + raw.y * raw.y + raw.z * raw.z).sqrt();
    let unit = if len == 0.0 {
        vec3(0.0, 0.0, 0.0)
    } else {
        vec3(raw.x / len, raw.y / len, raw.z / len)
    };
    let velocity = vec3(unit.x * Q1_GRENADE_SPEED, unit.y * Q1_GRENADE_SPEED, Q1_GRENADE_UP);
    let fields = SpawnFields {
        classname: "q1:grenade".to_string(),
        origin: me.origin,
        ..SpawnFields::default()
    };
    let grenade = match ctx.server.spawn_entity(&fields) {
        Ok(grenade) => grenade,
        Err(_) => return,
    };
    let point = Bounds {
        min: vec3(0.0, 0.0, 0.0),
        max: vec3(0.0, 0.0, 0.0),
    };
    let simulation = ctx.server.simulation_mut();
    if simulation.set_body_bounds(grenade.id(), point).is_err()
        || simulation.set_body_velocity(grenade.id(), velocity).is_err()
        || simulation
            .set_body_angles(grenade.id(), q1_vectoangles(velocity))
            .is_err()
    {
        let _ignored = simulation.release(&grenade);
        return;
    }
    let _ignored = simulation.link_body(grenade.id());
    ctx.behaviors.solids.insert(grenade.id());
    ctx.behaviors.projectiles.insert(
        grenade.id(),
        Q1Projectile {
            kind: Q1ProjectileKind::Grenade,
            owner: Some(actor.clone()),
            remove_at: ctx.now + Q1_GRENADE_FUSE,
            effects: 0,
            avelocity: vec3(300.0, 300.0, 300.0),
        },
    );
}

/// Stock `OgreGrenadeTouch` (`ogre.qc:71`): owner touches pass through;
/// aimed victims detonate; everything else bounces audibly, stilling
/// the spin on dead stops.
fn q1_ogre_grenade_touch<L: ServerLogic>(
    ctx: &mut Q1MonsterCtx<'_, '_, '_, L>,
    actor: &ActorId,
    other: Option<&ActorId>,
) {
    let projectile = ctx.behaviors.projectiles.get(actor).cloned();
    let Some(projectile) = projectile else {
        return;
    };
    if projectile.owner.as_ref() == other {
        return;
    }
    if other.is_some_and(|other| q1_takedamage_aim(ctx, other)) {
        q1_ogre_grenade_explode(ctx, actor);
        return;
    }
    q1_monster_sound(
        ctx.behaviors,
        actor,
        Q1_CHAN_VOICE,
        "weapons/bounce.wav",
        1.0,
        Q1_ATTN_NORM,
    );
    let stopped = ctx
        .server
        .simulation()
        .body_state(actor)
        .is_some_and(|body| body.velocity.x == 0.0 && body.velocity.y == 0.0 && body.velocity.z == 0.0);
    if stopped {
        if let Some(projectile) = ctx.behaviors.projectiles.get_mut(actor) {
            projectile.avelocity = vec3(0.0, 0.0, 0.0);
        }
    }
}

/// Stock `OgreGrenadeExplode` (`ogre.qc:53`): radius damage around the
/// grenade, the blast crack and flash, then the grenade removes (the
/// `s_explode` sprite ride collapses into the broadcast).
fn q1_ogre_grenade_explode<L: ServerLogic>(ctx: &mut Q1MonsterCtx<'_, '_, '_, L>, actor: &ActorId) {
    let projectile = ctx.behaviors.projectiles.get(actor).cloned();
    let Some(projectile) = projectile else {
        return;
    };
    let owner = projectile.owner.clone();
    q1_radius_damage(ctx, actor, owner.as_ref(), Q1_GRENADE_DAMAGE, None);
    q1_monster_sound(
        ctx.behaviors,
        actor,
        Q1_CHAN_VOICE,
        "weapons/r_exp3.wav",
        1.0,
        Q1_ATTN_NORM,
    );
    if let Some(body) = ctx.server.simulation().body_state(actor) {
        ctx.behaviors.temp_ents.push(Q1TempEnt::Explosion { org: body.origin });
    }
    let (simulation, movers, triggers) = ctx.server.simulation_movers_and_triggers_mut();
    q1_remove(ctx.behaviors, simulation, movers, triggers, actor);
}

/// Step one bouncing grenade (`SV_Physics_Toss` with the stock 1.5
/// bounce backoff, `sv_phys.c`): tumble, fall, bounce, and rest the
/// spin on quiet landings. Touches dispatch through `SV_Impact`.
fn q1_grenade_physics<L: ServerLogic>(ctx: &mut Q1MonsterCtx<'_, '_, '_, L>, actor: &ActorId) {
    let spin = ctx
        .behaviors
        .projectiles
        .get(actor)
        .map_or(vec3(0.0, 0.0, 0.0), |projectile| projectile.avelocity);
    if !q1_toss_step(ctx, actor, spin, 1.5) {
        return;
    }
    if let Some(projectile) = ctx.behaviors.projectiles.get_mut(actor) {
        projectile.avelocity = vec3(0.0, 0.0, 0.0);
    }
}

/// Swing-stroke yaw wobble (`ogre_swing5..11`, `ogre.qc`): the saw
/// drags the ogre around up to 25 degrees a stroke.
fn q1_ogre_swing_yaw<L: ServerLogic>(ctx: &mut Q1MonsterCtx<'_, '_, '_, L>, actor: &ActorId) {
    let wobble = f64::from(q1_monster_random(ctx.behaviors)) * 25.0;
    if let Some(body) = ctx.server.simulation().body_state(actor) {
        let _ignored = ctx
            .server
            .simulation_mut()
            .set_body_angles(actor, vec3(body.angles.x, body.angles.y + wobble as f32, body.angles.z));
    }
}

/// One monster `$frame` body, dispatched per kind.
fn q1_monster_frame<L: ServerLogic>(
    ctx: &mut Q1MonsterCtx<'_, '_, '_, L>,
    actor: &ActorId,
    kind: Q1MonsterKind,
    seq: Q1MonsterSeq,
    index: u8,
) {
    match kind {
        Q1MonsterKind::Dog => q1_dog_frame(ctx, actor, seq, index),
        Q1MonsterKind::Grunt => q1_grunt_frame(ctx, actor, seq, index),
        Q1MonsterKind::Enforcer => q1_enforcer_frame(ctx, actor, seq, index),
        Q1MonsterKind::Ogre => q1_ogre_frame(ctx, actor, seq, index),
    }
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

    fn monster_fields(classname: &str, pairs: &[(&str, &str)]) -> SpawnFields {
        let mut full = vec![("classname", classname), ("origin", "0 0 0")];
        full.extend_from_slice(pairs);
        SpawnFields::parse(&full).unwrap()
    }

    fn dog_fields(pairs: &[(&str, &str)]) -> SpawnFields {
        monster_fields("monster_dog", pairs)
    }

    fn grunt_fields(pairs: &[(&str, &str)]) -> SpawnFields {
        monster_fields("monster_army", pairs)
    }

    fn enforcer_fields(pairs: &[(&str, &str)]) -> SpawnFields {
        monster_fields("monster_enforcer", pairs)
    }

    fn ogre_fields(pairs: &[(&str, &str)]) -> SpawnFields {
        monster_fields("monster_ogre", pairs)
    }

    fn spawn_monster(
        server: &mut Server<qa_guest::server::GuestServerLogic>,
        behaviors: &mut Q1NativeBehaviors,
        fields: &SpawnFields,
    ) -> OwnedActor {
        register_q1_monster_spawns(server.spawns_mut());
        let actor = server.spawn_entity(fields).unwrap();
        build_q1_monster(server, behaviors, &actor, fields).unwrap();
        actor
    }

    fn spawn_dog(
        server: &mut Server<qa_guest::server::GuestServerLogic>,
        behaviors: &mut Q1NativeBehaviors,
        fields: &SpawnFields,
    ) -> OwnedActor {
        spawn_monster(server, behaviors, fields)
    }

    fn spawn_grunt(
        server: &mut Server<qa_guest::server::GuestServerLogic>,
        behaviors: &mut Q1NativeBehaviors,
        fields: &SpawnFields,
    ) -> OwnedActor {
        spawn_monster(server, behaviors, fields)
    }

    fn spawn_enforcer(
        server: &mut Server<qa_guest::server::GuestServerLogic>,
        behaviors: &mut Q1NativeBehaviors,
        fields: &SpawnFields,
    ) -> OwnedActor {
        spawn_monster(server, behaviors, fields)
    }

    fn spawn_ogre(
        server: &mut Server<qa_guest::server::GuestServerLogic>,
        behaviors: &mut Q1NativeBehaviors,
        fields: &SpawnFields,
    ) -> OwnedActor {
        spawn_monster(server, behaviors, fields)
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
    fn arm_monster(
        server: &mut Server<qa_guest::server::GuestServerLogic>,
        behaviors: &mut Q1NativeBehaviors,
        monster: &ActorId,
    ) {
        let combat = server.simulation().combat_state(monster).cloned().unwrap();
        server
            .simulation_mut()
            .set_combat(
                monster,
                CombatState {
                    can_take_damage: true,
                    ..combat
                },
            )
            .unwrap();
        let monster = behaviors.monsters.get_mut(monster).unwrap();
        monster.flags |= Q1_FLAG_MONSTER;
        monster.takedamage = Q1_DAMAGE_AIM;
    }

    fn arm_dog(
        server: &mut Server<qa_guest::server::GuestServerLogic>,
        behaviors: &mut Q1NativeBehaviors,
        dog: &ActorId,
    ) {
        arm_monster(server, behaviors, dog);
    }

    fn arm_grunt(
        server: &mut Server<qa_guest::server::GuestServerLogic>,
        behaviors: &mut Q1NativeBehaviors,
        grunt: &ActorId,
    ) {
        arm_monster(server, behaviors, grunt);
    }

    fn arm_enforcer(
        server: &mut Server<qa_guest::server::GuestServerLogic>,
        behaviors: &mut Q1NativeBehaviors,
        enforcer: &ActorId,
    ) {
        arm_monster(server, behaviors, enforcer);
    }

    fn arm_ogre(
        server: &mut Server<qa_guest::server::GuestServerLogic>,
        behaviors: &mut Q1NativeBehaviors,
        ogre: &ActorId,
    ) {
        arm_monster(server, behaviors, ogre);
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

    #[test]
    fn grunt_spawn_sizes_counts_and_defers_start() {
        let mut server = test_server();
        let mut behaviors = Q1NativeBehaviors::new();
        let fields = grunt_fields(&[]);
        let grunt = spawn_grunt(&mut server, &mut behaviors, &fields);
        let body = server.simulation().body_state(grunt.id()).unwrap();
        assert_eq!(body.bounds.min, Q1_GRUNT_BOUNDS.min);
        assert_eq!(body.bounds.max, Q1_GRUNT_BOUNDS.max);
        let combat = server.simulation().combat_state(grunt.id()).unwrap();
        assert_eq!(combat.health, Q1_GRUNT_HEALTH);
        assert!(!combat.can_take_damage);
        assert!(behaviors.solids.contains(grunt.id()));
        let monster = behaviors.monsters.get(grunt.id()).unwrap();
        assert_eq!(monster.kind, Q1MonsterKind::Grunt);
        assert_eq!(monster.think, Q1MonsterThink::StartGo);
        assert!((0.0..0.5).contains(&monster.nextthink));
        assert_eq!(monster.effects, 0);
        assert_eq!(behaviors.total_monsters, 1);
        assert_eq!(
            Q1MonsterKind::from_classname("monster_army"),
            Some(Q1MonsterKind::Grunt)
        );
        assert_eq!(Q1MonsterKind::Grunt.classname(), "monster_army");
        assert!(q1_th_melee(&mut behaviors, Q1MonsterKind::Grunt).is_none());
        assert_eq!(
            q1_th_missile(Q1MonsterKind::Grunt),
            Some(Q1MonsterThink::Frame(Q1MonsterSeq::GruntAttack, 0))
        );
    }

    #[test]
    fn grunt_pain_gate_then_three_way() {
        let mut server = test_server();
        let mut behaviors = Q1NativeBehaviors::new();
        let player = spawn_player(&mut server, vec3(100.0, 0.0, 0.0));
        behaviors.set_player(Some(player.id().clone()));
        let fields = grunt_fields(&[]);
        let grunt = spawn_grunt(&mut server, &mut behaviors, &fields);
        arm_grunt(&mut server, &mut behaviors, grunt.id());
        // A running pain hold swallows the pain entirely.
        behaviors.monsters.get_mut(grunt.id()).unwrap().pain_finished = 999.0;
        let (simulation, movers, triggers) = server.simulation_movers_and_triggers_mut();
        q1_t_damage(
            &mut behaviors,
            simulation,
            movers,
            triggers,
            grunt.id(),
            None,
            Some(player.id()),
            5.0,
        );
        let monster = behaviors.monsters.get(grunt.id()).unwrap();
        // The feud still hunts (sight bark plus run), but held pain
        // adds no pain sequence and no pain bark.
        assert_eq!(
            monster.think,
            Q1MonsterThink::Frame(Q1MonsterSeq::GruntRun, 0),
            "held pain keeps the hunt think"
        );
        assert!(
            behaviors
                .sounds
                .iter()
                .all(|sound| sound.sample != "soldier/pain1.wav" && sound.sample != "soldier/pain2.wav"),
            "held pain stays pain-silent"
        );
        // A fresh wound takes one of the three pain sequences.
        behaviors.monsters.get_mut(grunt.id()).unwrap().pain_finished = 0.0;
        let (simulation, movers, triggers) = server.simulation_movers_and_triggers_mut();
        q1_t_damage(
            &mut behaviors,
            simulation,
            movers,
            triggers,
            grunt.id(),
            None,
            Some(player.id()),
            5.0,
        );
        assert_eq!(q1_health_of(server.simulation(), grunt.id()), 20.0);
        let monster = behaviors.monsters.get(grunt.id()).unwrap();
        assert!(matches!(
            monster.think,
            Q1MonsterThink::Frame(Q1MonsterSeq::GruntPain, 0)
                | Q1MonsterThink::Frame(Q1MonsterSeq::GruntPainB, 0)
                | Q1MonsterThink::Frame(Q1MonsterSeq::GruntPainC, 0)
        ));
        assert!(monster.pain_finished > 0.0);
        assert!(behaviors
            .sounds
            .iter()
            .any(|sound| sound.sample == "soldier/pain1.wav" || sound.sample == "soldier/pain2.wav"));
    }

    #[test]
    fn grunt_dies_solid_then_drops() {
        let mut server = test_server();
        let mut behaviors = Q1NativeBehaviors::new();
        let player = spawn_player(&mut server, vec3(100.0, 0.0, 0.0));
        behaviors.set_player(Some(player.id().clone()));
        let fields = grunt_fields(&[]);
        let grunt = spawn_grunt(&mut server, &mut behaviors, &fields);
        arm_grunt(&mut server, &mut behaviors, grunt.id());
        let (simulation, movers, triggers) = server.simulation_movers_and_triggers_mut();
        // 35 damage leaves -5: dead, but above the -35 gib threshold.
        q1_t_damage(
            &mut behaviors,
            simulation,
            movers,
            triggers,
            grunt.id(),
            None,
            Some(player.id()),
            35.0,
        );
        assert!(q1_health_of(server.simulation(), grunt.id()) <= 0.0);
        assert_eq!(behaviors.killed_monsters, 1);
        let monster = behaviors.monsters.get(grunt.id()).unwrap();
        assert!(monster.dead);
        assert!(matches!(
            monster.think,
            Q1MonsterThink::Frame(Q1MonsterSeq::GruntDie, 0) | Q1MonsterThink::Frame(Q1MonsterSeq::GruntDieC, 0)
        ));
        // Solidity drops in the third death frame, not in `th_die`.
        assert!(behaviors.solids.contains(grunt.id()));
        assert!(!q1_can_take_damage(server.simulation(), grunt.id()));
        assert!(behaviors
            .sounds
            .iter()
            .any(|sound| sound.sample == "soldier/death1.wav"));
    }

    #[test]
    fn grunt_gib_threshold_bursts() {
        let mut server = test_server();
        let mut behaviors = Q1NativeBehaviors::new();
        let player = spawn_player(&mut server, vec3(100.0, 0.0, 0.0));
        behaviors.set_player(Some(player.id().clone()));
        let fields = grunt_fields(&[]);
        let grunt = spawn_grunt(&mut server, &mut behaviors, &fields);
        arm_grunt(&mut server, &mut behaviors, grunt.id());
        let (simulation, movers, triggers) = server.simulation_movers_and_triggers_mut();
        q1_t_damage(
            &mut behaviors,
            simulation,
            movers,
            triggers,
            grunt.id(),
            None,
            Some(player.id()),
            1000.0,
        );
        assert_eq!(q1_health_of(server.simulation(), grunt.id()), -99.0);
        assert_eq!(behaviors.pending_gibs.len(), 3);
        assert!(behaviors.gibs.contains_key(grunt.id()));
        assert!(behaviors.sounds.iter().any(|sound| sound.sample == "player/udeath.wav"));
    }

    #[test]
    fn grunt_feuds_with_own_class() {
        let mut server = test_server();
        let mut behaviors = Q1NativeBehaviors::new();
        let player = spawn_player(&mut server, vec3(500.0, 0.0, 0.0));
        behaviors.set_player(Some(player.id().clone()));
        let fields = grunt_fields(&[]);
        let first = spawn_grunt(&mut server, &mut behaviors, &fields);
        let second = spawn_grunt(&mut server, &mut behaviors, &fields);
        arm_grunt(&mut server, &mut behaviors, first.id());
        arm_grunt(&mut server, &mut behaviors, second.id());
        let (simulation, movers, triggers) = server.simulation_movers_and_triggers_mut();
        // Same-class grunts feud (`combat.qc:187`).
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
        let monster = behaviors.monsters.get(second.id()).unwrap();
        assert_eq!(monster.enemy.as_ref(), Some(first.id()));
        assert!(behaviors
            .sounds
            .iter()
            .any(|sound| sound.sample == "soldier/sight1.wav"));
    }

    #[test]
    fn grunt_sequence_tables_match_stock() {
        use Q1MonsterSeq::*;
        assert_eq!(q1_seq_len(GruntStand), 8);
        assert_eq!(q1_seq_len(GruntWalk), 24);
        assert_eq!(q1_seq_len(GruntRun), 8);
        assert_eq!(q1_seq_len(GruntAttack), 9);
        assert_eq!(q1_seq_len(GruntPain), 6);
        assert_eq!(q1_seq_len(GruntPainB), 14);
        assert_eq!(q1_seq_len(GruntPainC), 13);
        assert_eq!(q1_seq_len(GruntDie), 10);
        assert_eq!(q1_seq_len(GruntDieC), 11);
        assert_eq!(q1_seq_frame(GruntStand, 0), 0);
        assert_eq!(q1_seq_frame(GruntDie, 9), 17);
        assert_eq!(q1_seq_frame(GruntDieC, 0), 18);
        assert_eq!(q1_seq_frame(GruntPain, 0), 40);
        assert_eq!(q1_seq_frame(GruntPainB, 13), 59);
        assert_eq!(q1_seq_frame(GruntPainC, 12), 72);
        assert_eq!(q1_seq_frame(GruntRun, 0), 73);
        assert_eq!(q1_seq_frame(GruntAttack, 0), 81);
        assert_eq!(q1_seq_frame(GruntWalk, 23), 113);
        assert_eq!(
            q1_seq_next(Q1MonsterKind::Grunt, GruntAttack, 8),
            q1_th_run(Q1MonsterKind::Grunt)
        );
        assert_eq!(
            q1_seq_next(Q1MonsterKind::Grunt, GruntPainC, 12),
            q1_th_run(Q1MonsterKind::Grunt)
        );
        assert_eq!(
            q1_seq_next(Q1MonsterKind::Grunt, GruntDie, 9),
            Q1MonsterThink::Frame(GruntDie, 9)
        );
        assert_eq!(
            q1_seq_next(Q1MonsterKind::Grunt, GruntDieC, 10),
            Q1MonsterThink::Frame(GruntDieC, 10)
        );
    }

    #[test]
    fn enforcer_spawn_sizes_counts_and_defers_start() {
        let mut server = test_server();
        let mut behaviors = Q1NativeBehaviors::new();
        let fields = enforcer_fields(&[]);
        let enforcer = spawn_enforcer(&mut server, &mut behaviors, &fields);
        let body = server.simulation().body_state(enforcer.id()).unwrap();
        assert_eq!(body.bounds.min, Q1_ENFORCER_BOUNDS.min);
        assert_eq!(body.bounds.max, Q1_ENFORCER_BOUNDS.max);
        let combat = server.simulation().combat_state(enforcer.id()).unwrap();
        assert_eq!(combat.health, Q1_ENFORCER_HEALTH);
        assert!(!combat.can_take_damage);
        assert!(behaviors.solids.contains(enforcer.id()));
        let monster = behaviors.monsters.get(enforcer.id()).unwrap();
        assert_eq!(monster.kind, Q1MonsterKind::Enforcer);
        assert_eq!(monster.think, Q1MonsterThink::StartGo);
        assert!((0.0..0.5).contains(&monster.nextthink));
        assert_eq!(monster.effects, 0);
        assert_eq!(behaviors.total_monsters, 1);
        assert_eq!(
            Q1MonsterKind::from_classname("monster_enforcer"),
            Some(Q1MonsterKind::Enforcer)
        );
        assert_eq!(Q1MonsterKind::Enforcer.classname(), "monster_enforcer");
        assert!(q1_th_melee(&mut behaviors, Q1MonsterKind::Enforcer).is_none());
        assert_eq!(
            q1_th_missile(Q1MonsterKind::Enforcer),
            Some(Q1MonsterThink::Frame(Q1MonsterSeq::EnforcerAttack, 0))
        );
    }

    #[test]
    fn enforcer_pain_gate_then_four_way() {
        let mut server = test_server();
        let mut behaviors = Q1NativeBehaviors::new();
        let player = spawn_player(&mut server, vec3(100.0, 0.0, 0.0));
        behaviors.set_player(Some(player.id().clone()));
        let fields = enforcer_fields(&[]);
        let enforcer = spawn_enforcer(&mut server, &mut behaviors, &fields);
        arm_enforcer(&mut server, &mut behaviors, enforcer.id());
        // A running pain hold swallows the pain entirely.
        behaviors.monsters.get_mut(enforcer.id()).unwrap().pain_finished = 999.0;
        let (simulation, movers, triggers) = server.simulation_movers_and_triggers_mut();
        q1_t_damage(
            &mut behaviors,
            simulation,
            movers,
            triggers,
            enforcer.id(),
            None,
            Some(player.id()),
            5.0,
        );
        let monster = behaviors.monsters.get(enforcer.id()).unwrap();
        // The feud still hunts (sight bark plus run), but held pain
        // adds no pain sequence and no pain bark.
        assert_eq!(
            monster.think,
            Q1MonsterThink::Frame(Q1MonsterSeq::EnforcerRun, 0),
            "held pain keeps the hunt think"
        );
        assert!(
            behaviors
                .sounds
                .iter()
                .all(|sound| sound.sample != "enforcer/pain1.wav" && sound.sample != "enforcer/pain2.wav"),
            "held pain stays pain-silent"
        );
        // A fresh wound takes one of the four pain sequences.
        behaviors.monsters.get_mut(enforcer.id()).unwrap().pain_finished = 0.0;
        let (simulation, movers, triggers) = server.simulation_movers_and_triggers_mut();
        q1_t_damage(
            &mut behaviors,
            simulation,
            movers,
            triggers,
            enforcer.id(),
            None,
            Some(player.id()),
            5.0,
        );
        assert_eq!(q1_health_of(server.simulation(), enforcer.id()), 70.0);
        let monster = behaviors.monsters.get(enforcer.id()).unwrap();
        assert!(matches!(
            monster.think,
            Q1MonsterThink::Frame(Q1MonsterSeq::EnforcerPainA, 0)
                | Q1MonsterThink::Frame(Q1MonsterSeq::EnforcerPainB, 0)
                | Q1MonsterThink::Frame(Q1MonsterSeq::EnforcerPainC, 0)
                | Q1MonsterThink::Frame(Q1MonsterSeq::EnforcerPainD, 0)
        ));
        assert!(monster.pain_finished > 0.0);
        assert!(behaviors
            .sounds
            .iter()
            .any(|sound| sound.sample == "enforcer/pain1.wav" || sound.sample == "enforcer/pain2.wav"));
    }

    #[test]
    fn enforcer_dies_solid_then_drops() {
        let mut server = test_server();
        let mut behaviors = Q1NativeBehaviors::new();
        let player = spawn_player(&mut server, vec3(100.0, 0.0, 0.0));
        behaviors.set_player(Some(player.id().clone()));
        let fields = enforcer_fields(&[]);
        let enforcer = spawn_enforcer(&mut server, &mut behaviors, &fields);
        arm_enforcer(&mut server, &mut behaviors, enforcer.id());
        let (simulation, movers, triggers) = server.simulation_movers_and_triggers_mut();
        // 85 damage leaves -5: dead, but above the -35 gib threshold.
        q1_t_damage(
            &mut behaviors,
            simulation,
            movers,
            triggers,
            enforcer.id(),
            None,
            Some(player.id()),
            85.0,
        );
        assert!(q1_health_of(server.simulation(), enforcer.id()) <= 0.0);
        assert_eq!(behaviors.killed_monsters, 1);
        let monster = behaviors.monsters.get(enforcer.id()).unwrap();
        assert!(monster.dead);
        assert!(matches!(
            monster.think,
            Q1MonsterThink::Frame(Q1MonsterSeq::EnforcerDie, 0) | Q1MonsterThink::Frame(Q1MonsterSeq::EnforcerFDie, 0)
        ));
        // Solidity drops in the third death frame, not in `th_die`.
        assert!(behaviors.solids.contains(enforcer.id()));
        assert!(!q1_can_take_damage(server.simulation(), enforcer.id()));
        assert!(behaviors
            .sounds
            .iter()
            .any(|sound| sound.sample == "enforcer/death1.wav"));
    }

    #[test]
    fn enforcer_gib_threshold_bursts() {
        let mut server = test_server();
        let mut behaviors = Q1NativeBehaviors::new();
        let player = spawn_player(&mut server, vec3(100.0, 0.0, 0.0));
        behaviors.set_player(Some(player.id().clone()));
        let fields = enforcer_fields(&[]);
        let enforcer = spawn_enforcer(&mut server, &mut behaviors, &fields);
        arm_enforcer(&mut server, &mut behaviors, enforcer.id());
        let (simulation, movers, triggers) = server.simulation_movers_and_triggers_mut();
        q1_t_damage(
            &mut behaviors,
            simulation,
            movers,
            triggers,
            enforcer.id(),
            None,
            Some(player.id()),
            1000.0,
        );
        assert_eq!(q1_health_of(server.simulation(), enforcer.id()), -99.0);
        assert_eq!(behaviors.pending_gibs.len(), 3);
        assert!(behaviors.gibs.contains_key(enforcer.id()));
        assert!(behaviors.sounds.iter().any(|sound| sound.sample == "player/udeath.wav"));
    }

    #[test]
    fn infighting_turns_enforcer_on_other_class() {
        let mut server = test_server();
        let mut behaviors = Q1NativeBehaviors::new();
        let player = spawn_player(&mut server, vec3(500.0, 0.0, 0.0));
        behaviors.set_player(Some(player.id().clone()));
        let fields = enforcer_fields(&[]);
        let first = spawn_enforcer(&mut server, &mut behaviors, &fields);
        let second = spawn_enforcer(&mut server, &mut behaviors, &fields);
        arm_enforcer(&mut server, &mut behaviors, first.id());
        arm_enforcer(&mut server, &mut behaviors, second.id());
        let (simulation, movers, triggers) = server.simulation_movers_and_triggers_mut();
        // Same-class enforcers stay friendly: only grunts feud their
        // own class, so no feud here, only pain (`combat.qc:185`).
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
        let grunt_fields = grunt_fields(&[]);
        let grunt = spawn_grunt(&mut server, &mut behaviors, &grunt_fields);
        arm_grunt(&mut server, &mut behaviors, grunt.id());
        let (simulation, movers, triggers) = server.simulation_movers_and_triggers_mut();
        // A grunt attacker turns the enforcer around with a hunt.
        q1_t_damage(
            &mut behaviors,
            simulation,
            movers,
            triggers,
            second.id(),
            Some(grunt.id()),
            Some(grunt.id()),
            5.0,
        );
        let monster = behaviors.monsters.get(second.id()).unwrap();
        assert_eq!(monster.enemy.as_ref(), Some(grunt.id()));
        assert!(behaviors
            .sounds
            .iter()
            .any(|sound| sound.sample.starts_with("enforcer/sight")));
    }

    #[test]
    fn enforcer_sequence_tables_match_stock() {
        use Q1MonsterSeq::*;
        assert_eq!(q1_seq_len(EnforcerStand), 7);
        assert_eq!(q1_seq_len(EnforcerWalk), 16);
        assert_eq!(q1_seq_len(EnforcerRun), 8);
        assert_eq!(q1_seq_len(EnforcerAttack), 14);
        assert_eq!(q1_seq_len(EnforcerPainA), 4);
        assert_eq!(q1_seq_len(EnforcerPainB), 5);
        assert_eq!(q1_seq_len(EnforcerPainC), 8);
        assert_eq!(q1_seq_len(EnforcerPainD), 19);
        assert_eq!(q1_seq_len(EnforcerDie), 14);
        assert_eq!(q1_seq_len(EnforcerFDie), 11);
        assert_eq!(q1_seq_frame(EnforcerStand, 6), 6);
        assert_eq!(q1_seq_frame(EnforcerWalk, 15), 22);
        assert_eq!(q1_seq_frame(EnforcerRun, 7), 30);
        assert_eq!(q1_seq_frame(EnforcerAttack, 0), 31);
        assert_eq!(q1_seq_frame(EnforcerAttack, 7), 38);
        // The volley reuses attack5-8 mid-sequence.
        assert_eq!(q1_seq_frame(EnforcerAttack, 8), 35);
        assert_eq!(q1_seq_frame(EnforcerAttack, 11), 38);
        assert_eq!(q1_seq_frame(EnforcerAttack, 13), 40);
        assert_eq!(q1_seq_frame(EnforcerDie, 0), 41);
        assert_eq!(q1_seq_frame(EnforcerDie, 13), 54);
        assert_eq!(q1_seq_frame(EnforcerFDie, 0), 55);
        assert_eq!(q1_seq_frame(EnforcerFDie, 10), 65);
        assert_eq!(q1_seq_frame(EnforcerPainA, 3), 69);
        assert_eq!(q1_seq_frame(EnforcerPainB, 4), 74);
        assert_eq!(q1_seq_frame(EnforcerPainC, 7), 82);
        assert_eq!(q1_seq_frame(EnforcerPainD, 0), 83);
        assert_eq!(q1_seq_frame(EnforcerPainD, 18), 101);
        assert_eq!(
            q1_seq_next(Q1MonsterKind::Enforcer, EnforcerAttack, 13),
            q1_th_run(Q1MonsterKind::Enforcer)
        );
        assert_eq!(
            q1_seq_next(Q1MonsterKind::Enforcer, EnforcerPainD, 18),
            q1_th_run(Q1MonsterKind::Enforcer)
        );
        assert_eq!(
            q1_seq_next(Q1MonsterKind::Enforcer, EnforcerStand, 6),
            Q1MonsterThink::Frame(EnforcerStand, 0)
        );
        assert_eq!(
            q1_seq_next(Q1MonsterKind::Enforcer, EnforcerDie, 13),
            Q1MonsterThink::Frame(EnforcerDie, 13)
        );
        assert_eq!(
            q1_seq_next(Q1MonsterKind::Enforcer, EnforcerFDie, 10),
            Q1MonsterThink::Frame(EnforcerFDie, 10)
        );
    }

    #[test]
    fn vectoangles_matches_stock() {
        // Cardinal yaw, level pitch (`pr_cmds.c:428`).
        assert_eq!(q1_vectoangles(vec3(600.0, 0.0, 0.0)), vec3(0.0, 0.0, 0.0));
        assert_eq!(q1_vectoangles(vec3(0.0, 600.0, 0.0)), vec3(0.0, 90.0, 0.0));
        // Negative yaw wraps to 0-360.
        assert_eq!(q1_vectoangles(vec3(0.0, -600.0, 0.0)), vec3(0.0, 270.0, 0.0));
        // Vertical vectors yaw zero, pitch straight up or down.
        assert_eq!(q1_vectoangles(vec3(0.0, 0.0, 600.0)), vec3(90.0, 0.0, 0.0));
        assert_eq!(q1_vectoangles(vec3(0.0, 0.0, -600.0)), vec3(270.0, 0.0, 0.0));
        // Downward pitch wraps like stock.
        assert_eq!(q1_vectoangles(vec3(600.0, 0.0, -600.0)), vec3(315.0, 0.0, 0.0));
    }

    #[test]
    fn ogre_spawn_sizes_counts_and_defers_start() {
        let mut server = test_server();
        let mut behaviors = Q1NativeBehaviors::new();
        let fields = ogre_fields(&[]);
        let ogre = spawn_ogre(&mut server, &mut behaviors, &fields);
        let body = server.simulation().body_state(ogre.id()).unwrap();
        assert_eq!(body.bounds.min, Q1_OGRE_BOUNDS.min);
        assert_eq!(body.bounds.max, Q1_OGRE_BOUNDS.max);
        let combat = server.simulation().combat_state(ogre.id()).unwrap();
        assert_eq!(combat.health, Q1_OGRE_HEALTH);
        assert!(!combat.can_take_damage);
        assert!(behaviors.solids.contains(ogre.id()));
        let monster = behaviors.monsters.get(ogre.id()).unwrap();
        assert_eq!(monster.kind, Q1MonsterKind::Ogre);
        assert_eq!(monster.think, Q1MonsterThink::StartGo);
        assert!((0.0..0.5).contains(&monster.nextthink));
        assert_eq!(monster.effects, 0);
        assert_eq!(behaviors.total_monsters, 1);
        assert_eq!(Q1MonsterKind::from_classname("monster_ogre"), Some(Q1MonsterKind::Ogre));
        assert_eq!(Q1MonsterKind::Ogre.classname(), "monster_ogre");
        assert!(matches!(
            q1_th_melee(&mut behaviors, Q1MonsterKind::Ogre),
            Some(Q1MonsterThink::Frame(Q1MonsterSeq::OgreSmash, 0))
                | Some(Q1MonsterThink::Frame(Q1MonsterSeq::OgreSwing, 0))
        ));
        assert_eq!(
            q1_th_missile(Q1MonsterKind::Ogre),
            Some(Q1MonsterThink::Frame(Q1MonsterSeq::OgreNail, 0))
        );
    }

    #[test]
    fn ogre_marksman_shares_the_ogre_controller() {
        let mut server = test_server();
        let mut behaviors = Q1NativeBehaviors::new();
        assert_eq!(
            Q1MonsterKind::from_classname("monster_ogre_marksman"),
            Some(Q1MonsterKind::Ogre)
        );
        let fields = monster_fields("monster_ogre_marksman", &[]);
        let marksman = spawn_ogre(&mut server, &mut behaviors, &fields);
        let body = server.simulation().body_state(marksman.id()).unwrap();
        assert_eq!(body.bounds.min, Q1_OGRE_BOUNDS.min);
        assert_eq!(body.bounds.max, Q1_OGRE_BOUNDS.max);
        let combat = server.simulation().combat_state(marksman.id()).unwrap();
        assert_eq!(combat.health, Q1_OGRE_HEALTH);
        let monster = behaviors.monsters.get(marksman.id()).unwrap();
        assert_eq!(monster.kind, Q1MonsterKind::Ogre);
        assert_eq!(monster.think, Q1MonsterThink::StartGo);
        assert_eq!(behaviors.total_monsters, 1);
    }

    #[test]
    fn ogre_pain_gate_then_five_way() {
        let mut server = test_server();
        let mut behaviors = Q1NativeBehaviors::new();
        let player = spawn_player(&mut server, vec3(100.0, 0.0, 0.0));
        behaviors.set_player(Some(player.id().clone()));
        let fields = ogre_fields(&[]);
        let ogre = spawn_ogre(&mut server, &mut behaviors, &fields);
        arm_ogre(&mut server, &mut behaviors, ogre.id());
        // A running pain hold swallows the pain entirely.
        behaviors.monsters.get_mut(ogre.id()).unwrap().pain_finished = 999.0;
        let (simulation, movers, triggers) = server.simulation_movers_and_triggers_mut();
        q1_t_damage(
            &mut behaviors,
            simulation,
            movers,
            triggers,
            ogre.id(),
            None,
            Some(player.id()),
            5.0,
        );
        let monster = behaviors.monsters.get(ogre.id()).unwrap();
        // The feud still hunts (sight bark plus run), but held pain
        // adds no pain sequence and no pain bark.
        assert_eq!(
            monster.think,
            Q1MonsterThink::Frame(Q1MonsterSeq::OgreRun, 0),
            "held pain keeps the hunt think"
        );
        assert!(
            behaviors.sounds.iter().all(|sound| sound.sample != "ogre/ogpain1.wav"),
            "held pain stays pain-silent"
        );
        // A fresh wound takes one of the five pain sequences.
        behaviors.monsters.get_mut(ogre.id()).unwrap().pain_finished = 0.0;
        let (simulation, movers, triggers) = server.simulation_movers_and_triggers_mut();
        q1_t_damage(
            &mut behaviors,
            simulation,
            movers,
            triggers,
            ogre.id(),
            None,
            Some(player.id()),
            5.0,
        );
        assert_eq!(q1_health_of(server.simulation(), ogre.id()), 190.0);
        let monster = behaviors.monsters.get(ogre.id()).unwrap();
        assert!(matches!(
            monster.think,
            Q1MonsterThink::Frame(Q1MonsterSeq::OgrePain, 0)
                | Q1MonsterThink::Frame(Q1MonsterSeq::OgrePainB, 0)
                | Q1MonsterThink::Frame(Q1MonsterSeq::OgrePainC, 0)
                | Q1MonsterThink::Frame(Q1MonsterSeq::OgrePainD, 0)
                | Q1MonsterThink::Frame(Q1MonsterSeq::OgrePainE, 0)
        ));
        assert!(monster.pain_finished > 0.0);
        assert!(behaviors.sounds.iter().any(|sound| sound.sample == "ogre/ogpain1.wav"));
    }

    #[test]
    fn ogre_dies_solid_then_drops() {
        let mut server = test_server();
        let mut behaviors = Q1NativeBehaviors::new();
        let player = spawn_player(&mut server, vec3(100.0, 0.0, 0.0));
        behaviors.set_player(Some(player.id().clone()));
        let fields = ogre_fields(&[]);
        let ogre = spawn_ogre(&mut server, &mut behaviors, &fields);
        arm_ogre(&mut server, &mut behaviors, ogre.id());
        let (simulation, movers, triggers) = server.simulation_movers_and_triggers_mut();
        // 250 damage leaves -50: dead, but above the -80 gib threshold.
        q1_t_damage(
            &mut behaviors,
            simulation,
            movers,
            triggers,
            ogre.id(),
            None,
            Some(player.id()),
            250.0,
        );
        assert!(q1_health_of(server.simulation(), ogre.id()) <= 0.0);
        assert_eq!(behaviors.killed_monsters, 1);
        let monster = behaviors.monsters.get(ogre.id()).unwrap();
        assert!(monster.dead);
        assert!(matches!(
            monster.think,
            Q1MonsterThink::Frame(Q1MonsterSeq::OgreDie, 0) | Q1MonsterThink::Frame(Q1MonsterSeq::OgreBDie, 0)
        ));
        // Solidity drops in the third death frame, not in `th_die`.
        assert!(behaviors.solids.contains(ogre.id()));
        assert!(!q1_can_take_damage(server.simulation(), ogre.id()));
        assert!(behaviors.sounds.iter().any(|sound| sound.sample == "ogre/ogdth.wav"));
    }

    #[test]
    fn ogre_gib_threshold_bursts() {
        let mut server = test_server();
        let mut behaviors = Q1NativeBehaviors::new();
        let player = spawn_player(&mut server, vec3(100.0, 0.0, 0.0));
        behaviors.set_player(Some(player.id().clone()));
        let fields = ogre_fields(&[]);
        let ogre = spawn_ogre(&mut server, &mut behaviors, &fields);
        arm_ogre(&mut server, &mut behaviors, ogre.id());
        let (simulation, movers, triggers) = server.simulation_movers_and_triggers_mut();
        q1_t_damage(
            &mut behaviors,
            simulation,
            movers,
            triggers,
            ogre.id(),
            None,
            Some(player.id()),
            1000.0,
        );
        assert_eq!(q1_health_of(server.simulation(), ogre.id()), -99.0);
        assert_eq!(behaviors.pending_gibs.len(), 3);
        assert!(behaviors.gibs.contains_key(ogre.id()));
        assert!(behaviors.sounds.iter().any(|sound| sound.sample == "player/udeath.wav"));
    }

    #[test]
    fn infighting_turns_ogre_on_other_class() {
        let mut server = test_server();
        let mut behaviors = Q1NativeBehaviors::new();
        let player = spawn_player(&mut server, vec3(500.0, 0.0, 0.0));
        behaviors.set_player(Some(player.id().clone()));
        let fields = ogre_fields(&[]);
        let first = spawn_ogre(&mut server, &mut behaviors, &fields);
        let second = spawn_ogre(&mut server, &mut behaviors, &fields);
        arm_ogre(&mut server, &mut behaviors, first.id());
        arm_ogre(&mut server, &mut behaviors, second.id());
        let (simulation, movers, triggers) = server.simulation_movers_and_triggers_mut();
        // Same-class ogres stay friendly: only grunts feud their own
        // class, so no feud here, only pain (`combat.qc:185`).
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
        let grunt_fields = grunt_fields(&[]);
        let grunt = spawn_grunt(&mut server, &mut behaviors, &grunt_fields);
        arm_grunt(&mut server, &mut behaviors, grunt.id());
        let (simulation, movers, triggers) = server.simulation_movers_and_triggers_mut();
        // A grunt attacker turns the ogre around with a hunt.
        q1_t_damage(
            &mut behaviors,
            simulation,
            movers,
            triggers,
            second.id(),
            Some(grunt.id()),
            Some(grunt.id()),
            5.0,
        );
        let monster = behaviors.monsters.get(second.id()).unwrap();
        assert_eq!(monster.enemy.as_ref(), Some(grunt.id()));
        assert!(behaviors.sounds.iter().any(|sound| sound.sample == "ogre/ogwake.wav"));
    }

    #[test]
    fn ogre_sequence_tables_match_stock() {
        use Q1MonsterSeq::*;
        assert_eq!(q1_seq_len(OgreStand), 9);
        assert_eq!(q1_seq_len(OgreWalk), 16);
        assert_eq!(q1_seq_len(OgreRun), 8);
        assert_eq!(q1_seq_len(OgreSwing), 14);
        assert_eq!(q1_seq_len(OgreSmash), 14);
        assert_eq!(q1_seq_len(OgreNail), 7);
        assert_eq!(q1_seq_len(OgrePain), 5);
        assert_eq!(q1_seq_len(OgrePainB), 3);
        assert_eq!(q1_seq_len(OgrePainC), 6);
        assert_eq!(q1_seq_len(OgrePainD), 16);
        assert_eq!(q1_seq_len(OgrePainE), 15);
        assert_eq!(q1_seq_len(OgreDie), 14);
        assert_eq!(q1_seq_len(OgreBDie), 10);
        assert_eq!(q1_seq_frame(OgreStand, 8), 8);
        assert_eq!(q1_seq_frame(OgreWalk, 15), 24);
        assert_eq!(q1_seq_frame(OgreRun, 7), 32);
        assert_eq!(q1_seq_frame(OgreSwing, 13), 46);
        assert_eq!(q1_seq_frame(OgreSmash, 0), 47);
        assert_eq!(q1_seq_frame(OgreSmash, 13), 60);
        // The nail volley repeats shoot2.
        assert_eq!(q1_seq_frame(OgreNail, 0), 61);
        assert_eq!(q1_seq_frame(OgreNail, 1), 62);
        assert_eq!(q1_seq_frame(OgreNail, 2), 62);
        assert_eq!(q1_seq_frame(OgreNail, 6), 66);
        assert_eq!(q1_seq_frame(OgrePain, 4), 71);
        assert_eq!(q1_seq_frame(OgrePainB, 2), 74);
        assert_eq!(q1_seq_frame(OgrePainC, 5), 80);
        assert_eq!(q1_seq_frame(OgrePainD, 15), 96);
        assert_eq!(q1_seq_frame(OgrePainE, 0), 97);
        assert_eq!(q1_seq_frame(OgrePainE, 14), 111);
        assert_eq!(q1_seq_frame(OgreDie, 0), 112);
        assert_eq!(q1_seq_frame(OgreDie, 13), 125);
        assert_eq!(q1_seq_frame(OgreBDie, 0), 126);
        assert_eq!(q1_seq_frame(OgreBDie, 9), 135);
        assert_eq!(
            q1_seq_next(Q1MonsterKind::Ogre, OgreNail, 6),
            q1_th_run(Q1MonsterKind::Ogre)
        );
        assert_eq!(
            q1_seq_next(Q1MonsterKind::Ogre, OgreSwing, 13),
            q1_th_run(Q1MonsterKind::Ogre)
        );
        assert_eq!(
            q1_seq_next(Q1MonsterKind::Ogre, OgreStand, 8),
            Q1MonsterThink::Frame(OgreStand, 0)
        );
        assert_eq!(
            q1_seq_next(Q1MonsterKind::Ogre, OgreDie, 13),
            Q1MonsterThink::Frame(OgreDie, 13)
        );
        assert_eq!(
            q1_seq_next(Q1MonsterKind::Ogre, OgreBDie, 9),
            Q1MonsterThink::Frame(OgreBDie, 9)
        );
    }
}
