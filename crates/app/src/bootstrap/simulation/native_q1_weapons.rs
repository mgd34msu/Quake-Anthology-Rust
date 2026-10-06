//! Native Quake I player weapons: stock id1 gamecode as compiled Rust.
//!
//! The eight id1 weapons (`weapons.qc`), the player combat state they
//! read and write, and the PlayWorld-level weapon pass that drives
//! them against the shared collision scene (fired through
//! [`PlayWorld::step_weapons`](super::super::play_world::PlayWorld::step_weapons)
//! after the player step, the same slot `PlayerPostThink` owns).
//!
//! qsrc: `progs106/weapons.qc` (every `W_*`, `FireBullets`,
//! `TraceAttack`, multi-damage, `LightningDamage`, missile touches,
//! `W_BestWeapon`, `W_WeaponFrame`), `progs106/combat.qc`
//! (`T_RadiusDamage`, `CanDamage`), `WinQuake/pr_cmds.c` (`PF_aim`),
//! `progs106/client.qc` (spawn parms, `PutClientInServer`,
//! `PlayerDeathThink`, respawn), `progs106/player.qc` (attack frames,
//! `PlayerDie`, `GibPlayer`, pain), `progs106/world.qc` (body queue).
//!
//! Skeleton scope notes (each lands with its system, not here): blood
//! and muzzle-flash particles have no particle queue yet (damage,
//! sounds, and temp entities are live); the quad `SuperDamageSound`
//! waits for the powerup slice; `sv_aim` is the stock 0.93 constant
//! until the cvar slice owns it; backpack drops wait for the DM
//! item-drop slice; weapon pickups wait for the items slice.

use qa_bots::scene::{
    PointContentsQuery as ScenePointContentsQuery, PointContentsResult as ScenePointContentsResult,
    Q1MoveRule as SceneQ1MoveRule, QueryTarget as SceneQueryTarget, TraceDetail as SceneTraceDetail,
    TraceHit as SceneTraceHit, TracePolicy as SceneTracePolicy, TraceQuery as SceneTraceQuery,
    TraceShape as SceneTraceShape,
};
use qa_bots::shared_scene::SharedSceneQueries;
use qa_core::identity::ActorId;
use qa_core::math::{add3, angle_vectors, length3, scale3, vec3, Bounds, Vec3};
use qa_core::numeric::{NumericOps, Q1_DONOR_PROFILE};
use qa_world::body::translated_body_bounds;
use qa_world::collision::q1::CONTENTS_SKY;
use qa_world::combat::{ArmorState, CombatState, RegularArmor};
use qa_world::movement::clip_velocity_q1;
use qa_world::movement::q1::types::{Q1_CONTENTS_EMPTY, Q1_CONTENTS_LAVA, Q1_CONTENTS_SLIME, Q1_CONTENTS_WATER};
use qa_world::server::{Server, ServerLogic};
use qa_world::spawn::SpawnFields;

use super::native_q1_items::Q1Sprint;
use super::native_q1_monsters::{
    q1_can_damage, q1_monster_crandom, q1_monster_random, q1_monster_sound, q1_t_damage, q1_takedamage_aim,
    Q1MonsterCtx, Q1PendingGib, Q1_ATTN_NORM, Q1_ATTN_STATIC, Q1_CHAN_VOICE, Q1_CHAN_WEAPON, Q1_DAMAGE_AIM,
    Q1_FLESH_DAMAGE, Q1_GRENADE_DAMAGE, Q1_LASER_DAMAGE,
};
use super::native_q1_spawns::{q1_can_take_damage, q1_health_of, q1_remove, Q1NativeBehaviors};

/// Stock weapon/item bits (`defs.qc:285-298`). Weapon bits double as
/// `self.weapon` values; selection impulses 1-8 map to them in order.
pub const Q1_IT_SHOTGUN: u32 = 1;
/// Stock weapon/item bits (`defs.qc:285-298`).
pub const Q1_IT_SUPER_SHOTGUN: u32 = 2;
/// Stock weapon/item bits (`defs.qc:285-298`).
pub const Q1_IT_NAILGUN: u32 = 4;
/// Stock weapon/item bits (`defs.qc:285-298`).
pub const Q1_IT_SUPER_NAILGUN: u32 = 8;
/// Stock weapon/item bits (`defs.qc:285-298`).
pub const Q1_IT_GRENADE_LAUNCHER: u32 = 16;
/// Stock weapon/item bits (`defs.qc:285-298`).
pub const Q1_IT_ROCKET_LAUNCHER: u32 = 32;
/// Stock weapon/item bits (`defs.qc:285-298`).
pub const Q1_IT_LIGHTNING: u32 = 64;
/// Stock weapon/item bits (`defs.qc:285-298`).
pub const Q1_IT_SHELLS: u32 = 256;
/// Stock weapon/item bits (`defs.qc:285-298`).
pub const Q1_IT_NAILS: u32 = 512;
/// Stock weapon/item bits (`defs.qc:285-298`).
pub const Q1_IT_ROCKETS: u32 = 1024;
/// Stock weapon/item bits (`defs.qc:285-298`).
pub const Q1_IT_CELLS: u32 = 2048;
/// Stock weapon/item bits (`defs.qc:285-298`).
pub const Q1_IT_AXE: u32 = 4096;
/// Stock ammo-indicator bits cleared and reset by `W_SetCurrentAmmo`.
pub const Q1_IT_AMMO_BITS: u32 = Q1_IT_SHELLS | Q1_IT_NAILS | Q1_IT_ROCKETS | Q1_IT_CELLS;
/// Stock key and powerup bits (`defs.qc:305-311`): stripped by
/// `SetChangeParms`, never carried across a level transition.
pub const Q1_IT_KEY1: u32 = 131_072;
/// Stock key and powerup bits (`defs.qc:305-311`).
pub const Q1_IT_KEY2: u32 = 262_144;
/// Stock key and powerup bits (`defs.qc:308`).
pub const Q1_IT_INVISIBILITY: u32 = 524_288;
/// Stock key and powerup bits (`defs.qc:309`).
pub const Q1_IT_INVULNERABILITY: u32 = 1_048_576;
/// Stock key and powerup bits (`defs.qc:310`).
pub const Q1_IT_SUIT: u32 = 2_097_152;
/// Stock key and powerup bits (`defs.qc:311`).
pub const Q1_IT_QUAD: u32 = 4_194_304;

/// Stock deadflag values (`defs.qc:273-276`).
pub const Q1_DEAD_NO: u8 = 0;
/// Stock deadflag values (`defs.qc:273-276`).
pub const Q1_DEAD_DYING: u8 = 1;
/// Stock deadflag values (`defs.qc:273-276`).
pub const Q1_DEAD_DEAD: u8 = 2;
/// Stock deadflag values (`defs.qc:273-276`).
pub const Q1_DEAD_RESPAWNABLE: u8 = 3;
/// Player death anim ids (`PlayerDie`, `player.qc:524`): the axe death
/// plus the five random falls, with stock frame counts.
pub const Q1_DEATH_AXE: u8 = 0;
/// `player_diea1`: 11 frames.
pub const Q1_DEATH_A: u8 = 1;
/// `player_dieb1`: 9 frames.
pub const Q1_DEATH_B: u8 = 2;
/// `player_diec1`: 15 frames.
pub const Q1_DEATH_C: u8 = 3;
/// `player_died1`: 9 frames.
pub const Q1_DEATH_D: u8 = 4;
/// `player_diee1`: 9 frames.
pub const Q1_DEATH_E: u8 = 5;
/// Stock local-player fallback name (`netname` until the client slice
/// owns names; the engine defaults to "Player").
pub const Q1_PLAYER_NETNAME: &str = "Player";

/// Stock attack button (`button0`, bit 0 of the button word).
pub const Q1_BUTTON_ATTACK: i32 = 1;

/// Stock `sv_aim` default (`pr_cmds.c`): the minimum forward dot for
/// auto-aim acquisition.
pub const Q1_SV_AIM: f32 = 0.93;

/// Stock level-start spawn parms (`SetNewParms`, `client.qc:64`):
/// the inventory a fresh or respawned player carries.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1SpawnParms {
    /// Item bits (`parm1`: axe and shotgun).
    pub items: u32,
    /// Health (`parm2`: 100).
    pub health: f64,
    /// Armor value (`parm3`: 0).
    pub armorvalue: f64,
    /// Shells (`parm4`: 25).
    pub shells: f64,
    /// Nails (`parm5`: 0).
    pub nails: f64,
    /// Rockets (`parm6`: 0).
    pub rockets: f64,
    /// Cells (`parm7`: 0).
    pub cells: f64,
    /// Weapon bit (`parm8`: shotgun).
    pub weapon: u32,
    /// Armor type (`parm9`: 0).
    pub armortype: f64,
}

impl Default for Q1SpawnParms {
    /// Stock `SetNewParms` values.
    fn default() -> Self {
        Self {
            items: Q1_IT_AXE | Q1_IT_SHOTGUN,
            health: 100.0,
            armorvalue: 0.0,
            shells: 25.0,
            nails: 0.0,
            rockets: 0.0,
            cells: 0.0,
            weapon: Q1_IT_SHOTGUN,
            armortype: 0.0,
        }
    }
}

/// Live player attack state: the `player_*` frame functions as data.
/// Stock runs these as 0.1 s thinks; the weapon pass runs them every
/// step instead, which fires at the same instants (frame boundaries
/// quantize to the step clock either way).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Q1PlayerAttack {
    /// No attack anim (stock `player_run`).
    None,
    /// Axe swing in flight (`player_axe1`-`4`, `player.qc:150`):
    /// `W_FireAxe` lands at frame 3 (`fire_at`, 0.2 s after
    /// `W_Attack`), the anim retires at frame 4 (`until`).
    AxeSwing {
        /// Master-clock seconds of the frame-3 fire.
        fire_at: f64,
        /// Master-clock seconds of the frame-4 retire.
        until: f64,
    },
    /// Shot/rocket anim in flight (`player_shot1`-`6` and
    /// `player_rocket1`-`6`, `player.qc:142/233`): six 0.1 s frames
    /// shared by the shotgun, super shotgun, grenade, and rocket
    /// launchers.
    Gun {
        /// Master-clock seconds of the frame-1 fire.
        fired_at: f64,
    },
    /// Nailgun/super-nailgun burst in flight (`player_nail1`/`nail2`,
    /// `player.qc:173`): while the trigger stays held, a shot leaves
    /// every 0.1 s think, alternating barrels.
    Nail {
        /// Master-clock seconds of the next think shot.
        next_fire: f64,
    },
    /// Lightning burst in flight (`player_light1`/`light2`,
    /// `player.qc:202`): while the trigger stays held, a cell burns
    /// every 0.1 s think down a 600-unit beam.
    Lightning {
        /// Master-clock seconds of the next think shot.
        next_fire: f64,
    },
    /// Pain anim in flight (`player_pain1`/`player_axpain1`,
    /// `player.qc:337`): six 0.1 s frames that yield to the next
    /// attack like any replaced think.
    Pain {
        /// Master-clock seconds when the sixth frame lands.
        until: f64,
    },
}

/// One body-queue corpse slot (`CopyToBodyQue`, `world.qc:378`): the
/// visible copy of a respawned coop/DM player. Stock keeps four
/// `bodyque` edicts in a ring; the record holds what the presentation
/// slice needs to draw one.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1Corpse {
    /// Corpse origin (the death spot).
    pub origin: Vec3,
    /// Corpse angles.
    pub angles: Vec3,
    /// Corpse model frame (the death-anim frame).
    pub frame: i32,
    /// Corpse model path (`progs/player.mdl`, or the gib head).
    pub model: String,
}

/// Live stock player gamecode state: the entity fields `weapons.qc`,
/// `client.qc`, and `player.qc` read and write. Origin, angles, and
/// velocity ride the sim body; health and armor ride the combat
/// state; item bits and ammo ride the shared behavior fields.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1PlayerState {
    /// Current weapon bit (`self.weapon`).
    pub weapon: u32,
    /// Current view model (`self.weaponmodel`).
    pub weaponmodel: String,
    /// Current view-model frame (`self.weaponframe`).
    pub weaponframe: i32,
    /// Current ammo count for the weapon (`self.currentammo`).
    pub currentammo: f64,
    /// Attacks resume after this (`self.attack_finished`).
    pub attack_finished: f64,
    /// Death state (`self.deadflag`).
    pub deadflag: u8,
    /// Master-clock seconds when the death anim ends (`PlayerDead`).
    pub dead_until: f64,
    /// Death-anim frame count, for the body-queue frame.
    pub death_frames: u8,
    /// Death-anim id (`Q1_DEATH_AXE`/`A`-`E`, `PlayerDie`).
    pub death_anim: u8,
    /// Cause tag for the fall obituary (`self.deathtype`: "falling"
    /// or empty; the fall-damage slice owns it).
    pub deathtype: String,
    /// Frags (`self.frags`, kept as float like stock).
    pub frags: f64,
    /// Team id (`self.team`; the DM rules slice owns it, 0 is none).
    pub team: i32,
    /// Axe-marked this wound (`self.axhitme`, `weapons.qc:52`).
    pub axhitme: bool,
    /// Whether the corpse became a bouncing head (`ThrowHead`).
    pub gibbed_head: bool,
    /// Pain anims resume after this (nightmare hold reads it).
    pub pain_finished: f64,
    /// Other monsters stay woken until this (`W_Attack` + 1 s).
    pub show_hostile: f64,
    /// Lightning impact sounds resume after this (`self.t_width`).
    pub t_width: f64,
    /// Live attack anim, if any.
    pub attack: Q1PlayerAttack,
    /// Nail barrel side in map units (`+4`/`-4`, `player.qc:183`).
    pub nail_side: f32,
    /// Previous step's button word (respawn release detection).
    pub prev_buttons: i32,
    /// Level-start inventory (coop `setspawnparms` source).
    pub parms: Q1SpawnParms,
    /// Water level 0-3 (stock `self.waterlevel`, sampled per step).
    pub water_level: i32,
    /// Water contents at the waist (`self.watertype`, sampled per
    /// step: -3 water, -4 slime, -5 lava).
    pub water_type: i32,
    /// Respawn facing for the pass to snap (`PutClientInServer`
    /// `fixangle`; `step_weapons` applies and clears it).
    pub respawn_angles: Option<Vec3>,
    /// Body-queue corpses, oldest first (at most four).
    pub body_queue: Vec<Q1Corpse>,
    /// Singleplayer death asks the app to restart the level
    /// (`localcmd ("restart")`, `client.qc:373`).
    pub restart_requested: bool,
    /// View kick (`self.punchangle`, for the presentation slice).
    pub punchangle: Vec3,
}

impl Default for Q1PlayerState {
    /// Fresh player: axe in hand (the spawn loadout promotes the
    /// shotgun), first nail barrel on the right.
    fn default() -> Self {
        Self {
            weapon: Q1_IT_AXE,
            weaponmodel: String::new(),
            weaponframe: 0,
            currentammo: 0.0,
            attack_finished: 0.0,
            deadflag: Q1_DEAD_NO,
            dead_until: 0.0,
            death_anim: Q1_DEATH_A,
            deathtype: String::new(),
            frags: 0.0,
            team: 0,
            axhitme: false,
            death_frames: 0,
            gibbed_head: false,
            pain_finished: 0.0,
            show_hostile: 0.0,
            t_width: 0.0,
            attack: Q1PlayerAttack::None,
            nail_side: 4.0,
            prev_buttons: 0,
            parms: Q1SpawnParms::default(),
            water_level: 0,
            water_type: Q1_CONTENTS_EMPTY,
            respawn_angles: None,
            body_queue: Vec::new(),
            restart_requested: false,
            punchangle: vec3(0.0, 0.0, 0.0),
        }
    }
}

/// One queued stock temp entity (`SVC_TEMPENTITY`) for the
/// presentation slice to drain: weapon impacts are broadcasts, so
/// every fire site records its exact kind and endpoints instead.
#[derive(Debug, Clone, PartialEq)]
pub enum Q1TempEnt {
    /// `TE_GUNSHOT` at the impact point.
    Gunshot {
        /// Impact point.
        at: Vec3,
    },
    /// `TE_SPIKE` at the impact point.
    Spike {
        /// Impact point.
        at: Vec3,
    },
    /// `TE_SUPERSPIKE` at the impact point.
    SuperSpike {
        /// Impact point.
        at: Vec3,
    },
    /// `TE_WIZSPIKE` at the impact point.
    WizSpike {
        /// Impact point.
        at: Vec3,
    },
    /// `TE_KNIGHTSPIKE` at the impact point.
    KnightSpike {
        /// Impact point.
        at: Vec3,
    },
    /// `TE_BLOOD` at the wound (`SpawnBlood`, `combat.qc`).
    Blood {
        /// Wound point.
        at: Vec3,
        /// Particle count (stock passes the damage).
        count: u32,
    },
    /// `TE_EXPLOSION` at the blast center.
    Explosion {
        /// Blast center.
        at: Vec3,
    },
    /// `TE_TELEPORT` at a coop/DM respawn (`spawn_tfog`, `triggers.qc`).
    Teleport {
        /// Fog center.
        at: Vec3,
    },
    /// Death-bubble presentation (`DeathBubbles`, `player.qc:356`):
    /// stock spawns bubble entities; this queues their visuals.
    Bubbles {
        /// Water point.
        at: Vec3,
        /// Bubble count.
        count: u32,
    },
    /// `TE_LIGHTNING2` from muzzle to impact.
    Lightning {
        /// Firing player.
        entity: ActorId,
        /// Muzzle point.
        start: Vec3,
        /// Impact point.
        end: Vec3,
    },
}

/// Stock waist contents (`self.watertype`): the liquid around
/// `origin + 10`, or empty (`q1_sample_water_level` counts the wet
/// samples; this names the wettest one for pain and obituaries).
pub fn q1_sample_water_type(
    scene: &SharedSceneQueries,
    simulation: &qa_world::session::Simulation,
    player: &ActorId,
) -> i32 {
    let Some(body) = simulation.body_state(player) else {
        return Q1_CONTENTS_EMPTY;
    };
    let query = ScenePointContentsQuery {
        point: vec3(body.origin.x, body.origin.y, body.origin.z + 10.0),
        target: SceneQueryTarget::World,
        policy: SceneTracePolicy::Q1 {
            move_rule: SceneQ1MoveRule::Normal,
            hull: None,
        },
        numeric: Q1_DONOR_PROFILE,
        pass_actor: Some(player.clone()),
    };
    if let Ok(ScenePointContentsResult::Q1 { contents }) = scene.point_contents(&query) {
        contents
    } else {
        Q1_CONTENTS_EMPTY
    }
}

/// Stock view-model paths by weapon bit (`W_SetCurrentAmmo`).
#[must_use]
pub fn q1_weapon_model(weapon: u32) -> &'static str {
    match weapon {
        Q1_IT_AXE => "progs/v_axe.mdl",
        Q1_IT_SHOTGUN => "progs/v_shot.mdl",
        Q1_IT_SUPER_SHOTGUN => "progs/v_shot2.mdl",
        Q1_IT_NAILGUN => "progs/v_nail.mdl",
        Q1_IT_SUPER_NAILGUN => "progs/v_nail2.mdl",
        Q1_IT_GRENADE_LAUNCHER => "progs/v_rock.mdl",
        Q1_IT_ROCKET_LAUNCHER => "progs/v_rock2.mdl",
        Q1_IT_LIGHTNING => "progs/v_light.mdl",
        _ => "",
    }
}

/// Stock spawn loadout (`PutClientInServer` + `DecodeLevelParms` over
/// fresh `SetNewParms`, `client.qc:479`): axe and shotgun, 25 shells,
/// shotgun in hand, and the parms snapshotted for coop respawns.
pub fn q1_grant_spawn_loadout(behaviors: &mut Q1NativeBehaviors) {
    let parms = Q1SpawnParms::default();
    behaviors.player_items |= parms.items;
    behaviors.player_ammo.shells = parms.shells;
    behaviors.player_ammo.nails = parms.nails;
    behaviors.player_ammo.rockets = parms.rockets;
    behaviors.player_ammo.cells = parms.cells;
    behaviors.player_state.weapon = parms.weapon;
    behaviors.player_state.parms = parms;
    q1_w_set_current_ammo(behaviors);
}

/// Stock `W_BestWeapon` (`weapons.qc:827`): lightning over everything
/// (above water, with cells), then super nailgun, super shotgun,
/// nailgun, shotgun, axe. Ties break toward the cheaper ammo the
/// same way stock does (minimum usable rounds per weapon).
#[must_use]
pub fn q1_w_best_weapon(behaviors: &Q1NativeBehaviors) -> u32 {
    let state = &behaviors.player_state;
    let items = behaviors.player_items;
    let ammo = &behaviors.player_ammo;
    if state.water_level <= 1 && ammo.cells >= 1.0 && items & Q1_IT_LIGHTNING != 0 {
        return Q1_IT_LIGHTNING;
    }
    if ammo.nails >= 2.0 && items & Q1_IT_SUPER_NAILGUN != 0 {
        return Q1_IT_SUPER_NAILGUN;
    }
    if ammo.shells >= 2.0 && items & Q1_IT_SUPER_SHOTGUN != 0 {
        return Q1_IT_SUPER_SHOTGUN;
    }
    if ammo.nails >= 1.0 && items & Q1_IT_NAILGUN != 0 {
        return Q1_IT_NAILGUN;
    }
    if ammo.shells >= 1.0 && items & Q1_IT_SHOTGUN != 0 {
        return Q1_IT_SHOTGUN;
    }
    Q1_IT_AXE
}

/// Stock `W_SetCurrentAmmo` (`weapons.qc:758`): drop any attack anim
/// (`player_run`), refresh the ammo-indicator bits and the current
/// count for the held weapon, and reset the view model.
pub fn q1_w_set_current_ammo(behaviors: &mut Q1NativeBehaviors) {
    behaviors.player_state.attack = Q1PlayerAttack::None;
    behaviors.player_items &= !Q1_IT_AMMO_BITS;
    let weapon = behaviors.player_state.weapon;
    let ammo = &behaviors.player_ammo;
    let (current, bit) = match weapon {
        Q1_IT_SHOTGUN | Q1_IT_SUPER_SHOTGUN => (ammo.shells, Q1_IT_SHELLS),
        Q1_IT_NAILGUN | Q1_IT_SUPER_NAILGUN => (ammo.nails, Q1_IT_NAILS),
        Q1_IT_GRENADE_LAUNCHER | Q1_IT_ROCKET_LAUNCHER => (ammo.rockets, Q1_IT_ROCKETS),
        Q1_IT_LIGHTNING => (ammo.cells, Q1_IT_CELLS),
        _ => (0.0, 0),
    };
    behaviors.player_items |= bit;
    behaviors.player_state.currentammo = current;
    behaviors.player_state.weaponmodel = q1_weapon_model(weapon).to_string();
    behaviors.player_state.weaponframe = 0;
}

/// Stock `W_CheckNoAmmo` (`weapons.qc:846`): loaded weapons (and the
/// axe, always) may fire; an empty weapon drops to the best one and
/// the attack fizzles this step.
fn q1_w_check_no_ammo(behaviors: &mut Q1NativeBehaviors) -> bool {
    if behaviors.player_state.currentammo > 0.0 {
        return true;
    }
    if behaviors.player_state.weapon == Q1_IT_AXE {
        return true;
    }
    behaviors.player_state.weapon = q1_w_best_weapon(behaviors);
    q1_w_set_current_ammo(behaviors);
    false
}

/// Queue a stock `sprint` line for the player.
fn q1_sprint(behaviors: &mut Q1NativeBehaviors, player: &ActorId, text: &str) {
    behaviors.sprints.push(Q1Sprint {
        target: player.clone(),
        text: text.to_string(),
    });
}

/// Stock `W_ChangeWeapon` (`weapons.qc:948`): impulses 1-8 select the
/// matching owned weapon with usable ammo, refusing with the stock
/// messages otherwise.
pub fn q1_w_change_weapon(behaviors: &mut Q1NativeBehaviors, player: &ActorId, impulse: i32) {
    let (weapon, enough_ammo) = match impulse {
        1 => (Q1_IT_AXE, true),
        2 => (Q1_IT_SHOTGUN, behaviors.player_ammo.shells >= 1.0),
        3 => (Q1_IT_SUPER_SHOTGUN, behaviors.player_ammo.shells >= 2.0),
        4 => (Q1_IT_NAILGUN, behaviors.player_ammo.nails >= 1.0),
        5 => (Q1_IT_SUPER_NAILGUN, behaviors.player_ammo.nails >= 2.0),
        6 => (Q1_IT_GRENADE_LAUNCHER, behaviors.player_ammo.rockets >= 1.0),
        7 => (Q1_IT_ROCKET_LAUNCHER, behaviors.player_ammo.rockets >= 1.0),
        8 => (Q1_IT_LIGHTNING, behaviors.player_ammo.cells >= 1.0),
        _ => return,
    };
    if behaviors.player_items & weapon == 0 {
        q1_sprint(behaviors, player, "no weapon.\n");
        return;
    }
    if !enough_ammo {
        q1_sprint(behaviors, player, "not enough ammo.\n");
        return;
    }
    behaviors.player_state.weapon = weapon;
    q1_w_set_current_ammo(behaviors);
}

/// Stock `CycleWeaponCommand` (`weapons.qc:1061`): impulse 10 steps to
/// the next owned weapon with usable ammo. The axe has no gate, so
/// the loop always terminates.
pub fn q1_cycle_weapon(behaviors: &mut Q1NativeBehaviors) {
    loop {
        let ammo = &behaviors.player_ammo;
        let (next, enough_ammo) = match behaviors.player_state.weapon {
            Q1_IT_LIGHTNING => (Q1_IT_AXE, true),
            Q1_IT_AXE => (Q1_IT_SHOTGUN, ammo.shells >= 1.0),
            Q1_IT_SHOTGUN => (Q1_IT_SUPER_SHOTGUN, ammo.shells >= 2.0),
            Q1_IT_SUPER_SHOTGUN => (Q1_IT_NAILGUN, ammo.nails >= 1.0),
            Q1_IT_NAILGUN => (Q1_IT_SUPER_NAILGUN, ammo.nails >= 2.0),
            Q1_IT_SUPER_NAILGUN => (Q1_IT_GRENADE_LAUNCHER, ammo.rockets >= 1.0),
            Q1_IT_GRENADE_LAUNCHER => (Q1_IT_ROCKET_LAUNCHER, ammo.rockets >= 1.0),
            _ => (Q1_IT_LIGHTNING, ammo.cells >= 1.0),
        };
        behaviors.player_state.weapon = next;
        if behaviors.player_items & next != 0 && enough_ammo {
            q1_w_set_current_ammo(behaviors);
            return;
        }
    }
}

/// Stock `CycleWeaponReverseCommand` (`weapons.qc:1135`): impulse 12
/// steps to the previous owned weapon with usable ammo.
pub fn q1_cycle_weapon_reverse(behaviors: &mut Q1NativeBehaviors) {
    loop {
        let ammo = &behaviors.player_ammo;
        let (next, enough_ammo) = match behaviors.player_state.weapon {
            Q1_IT_LIGHTNING => (Q1_IT_ROCKET_LAUNCHER, ammo.rockets >= 1.0),
            Q1_IT_ROCKET_LAUNCHER => (Q1_IT_GRENADE_LAUNCHER, ammo.rockets >= 1.0),
            Q1_IT_GRENADE_LAUNCHER => (Q1_IT_SUPER_NAILGUN, ammo.nails >= 2.0),
            Q1_IT_SUPER_NAILGUN => (Q1_IT_NAILGUN, ammo.nails >= 1.0),
            Q1_IT_NAILGUN => (Q1_IT_SUPER_SHOTGUN, ammo.shells >= 2.0),
            Q1_IT_SUPER_SHOTGUN => (Q1_IT_SHOTGUN, ammo.shells >= 1.0),
            Q1_IT_SHOTGUN => (Q1_IT_AXE, true),
            _ => (Q1_IT_LIGHTNING, ammo.cells >= 1.0),
        };
        behaviors.player_state.weapon = next;
        if behaviors.player_items & next != 0 && enough_ammo {
            q1_w_set_current_ammo(behaviors);
            return;
        }
    }
}

/// Stock `ImpulseCommands` (`weapons.qc:1230`): weapon impulses select
/// or cycle; cheat impulses (9, 11, 255) belong to the cheat slice
/// and are consumed unhandled until it lands.
pub fn q1_impulse_commands(behaviors: &mut Q1NativeBehaviors, player: &ActorId, impulse: i32) {
    if (1..=8).contains(&impulse) {
        q1_w_change_weapon(behaviors, player, impulse);
    }
    if impulse == 10 {
        q1_cycle_weapon(behaviors);
    }
    if impulse == 12 {
        q1_cycle_weapon_reverse(behaviors);
    }
}

/// One stock `traceline` result over the shared scene.
pub struct Q1LineHit {
    /// Trace fraction (1.0 is a clean miss).
    pub fraction: f64,
    /// Trace end position.
    pub endpos: Vec3,
    /// Hit actor, if the trace struck one.
    pub hit_actor: Option<ActorId>,
    /// Impact plane normal (blood-spray direction).
    pub plane_normal: Vec3,
}

/// Stock `traceline` (`pr_cmds.c`): a point trace against world hulls
/// and linked bodies, skipping `pass`. A failed trace blocks at the
/// start (callers are infallible gamecode; stopping beats shooting
/// through the world).
pub fn q1_traceline(
    scene: &SharedSceneQueries,
    start: Vec3,
    end: Vec3,
    move_rule: SceneQ1MoveRule,
    pass: &ActorId,
) -> Q1LineHit {
    q1_traceline_inner(scene, start, end, move_rule, pass, None)
}

/// Stock missile leg trace: like `q1_traceline`, but the owner's own
/// body never blocks its missile (stock `spike_touch` and
/// `T_MissileTouch` return on `other == self.owner`, so the flight
/// continues past them; without the exclusion a downward shot starts
/// inside the owner and the start-solid owner hit swallows the floor).
fn q1_missile_leg_trace(
    scene: &SharedSceneQueries,
    start: Vec3,
    end: Vec3,
    missile: &ActorId,
    owner: &ActorId,
) -> Q1LineHit {
    q1_traceline_inner(scene, start, end, SceneQ1MoveRule::Missile, missile, Some(owner))
}

/// Shared `traceline` core: `exclude` drops one extra body from the
/// trace (the missile owner for fly legs, nobody otherwise).
fn q1_traceline_inner(
    scene: &SharedSceneQueries,
    start: Vec3,
    end: Vec3,
    move_rule: SceneQ1MoveRule,
    pass: &ActorId,
    exclude: Option<&ActorId>,
) -> Q1LineHit {
    let query = SceneTraceQuery {
        start,
        end,
        shape: SceneTraceShape::Point,
        target: SceneQueryTarget::World,
        policy: SceneTracePolicy::Q1 { move_rule, hull: None },
        numeric: Q1_DONOR_PROFILE,
        pass_actor: Some(pass.clone()),
    };
    let empty = Vec3 { x: 0.0, y: 0.0, z: 0.0 };
    let traced = match exclude {
        Some(owner) => scene.trace_excluding(&query, std::slice::from_ref(owner)),
        None => scene.trace(&query),
    };
    let Ok(trace) = traced else {
        return Q1LineHit {
            fraction: 0.0,
            endpos: start,
            hit_actor: None,
            plane_normal: empty,
        };
    };
    let plane_normal = match &trace.detail {
        SceneTraceDetail::Q1 { source_plane, .. } => source_plane.normal,
        _ => empty,
    };
    Q1LineHit {
        fraction: trace.fraction,
        endpos: trace.end,
        hit_actor: match &trace.hit {
            SceneTraceHit::Actor { actor } => Some(actor.clone()),
            _ => None,
        },
        plane_normal,
    }
}

/// Stock `aim` (`PF_aim`, `pr_cmds.c:1333`): the straight shot stands
/// when it strikes a damageable target; otherwise the best visible
/// damageable target within the `sv_aim` cone bends the shot toward
/// its height. Teammates never qualify (no teamplay carrier yet, so
/// the check compares combat teams only when both sides carry one).
pub fn q1_aim<L: ServerLogic>(
    server: &Server<L>,
    behaviors: &Q1NativeBehaviors,
    scene: &SharedSceneQueries,
    player: &ActorId,
    forward: Vec3,
) -> Vec3 {
    let simulation = server.simulation();
    let Some(body) = simulation.body_state(player) else {
        return forward;
    };
    let start = vec3(body.origin.x, body.origin.y, body.origin.z + 20.0);
    let straight = vec3(
        start.x + forward.x * 2048.0,
        start.y + forward.y * 2048.0,
        start.z + forward.z * 2048.0,
    );
    let direct = q1_traceline(scene, start, straight, SceneQ1MoveRule::Normal, player);
    if let Some(hit) = direct.hit_actor.as_ref() {
        if q1_is_aim_target(server, behaviors, player, hit) {
            return forward;
        }
    }
    let mut best_dot = Q1_SV_AIM;
    let mut best: Option<ActorId> = None;
    for candidate in simulation.body_actors() {
        if &candidate == player {
            continue;
        }
        if !q1_is_aim_target(server, behaviors, player, &candidate) {
            continue;
        }
        let Some(target) = simulation.body_state(&candidate) else {
            continue;
        };
        let bounds = translated_body_bounds(&target);
        let center = vec3(
            (bounds.min.x + bounds.max.x) / 2.0,
            (bounds.min.y + bounds.max.y) / 2.0,
            (bounds.min.z + bounds.max.z) / 2.0,
        );
        let mut dir = vec3(center.x - start.x, center.y - start.y, center.z - start.z);
        let len = (dir.x * dir.x + dir.y * dir.y + dir.z * dir.z).sqrt();
        if len == 0.0 {
            continue;
        }
        dir = vec3(dir.x / len, dir.y / len, dir.z / len);
        let dot = dir.x * forward.x + dir.y * forward.y + dir.z * forward.z;
        if dot < best_dot {
            continue;
        }
        let sight = q1_traceline(scene, start, center, SceneQ1MoveRule::Normal, player);
        if sight.hit_actor.as_ref() == Some(&candidate) {
            best_dot = dot;
            best = Some(candidate);
        }
    }
    let Some(best) = best else {
        return forward;
    };
    let Some(target) = simulation.body_state(&best) else {
        return forward;
    };
    // Stock bends only the height: forward reach times the forward
    // dot, with the true height difference, normalized.
    let dir = vec3(
        target.origin.x - body.origin.x,
        target.origin.y - body.origin.y,
        target.origin.z - body.origin.z,
    );
    let dist = dir.x * forward.x + dir.y * forward.y + dir.z * forward.z;
    let mut bent = vec3(forward.x * dist, forward.y * dist, dir.z);
    let len = (bent.x * bent.x + bent.y * bent.y + bent.z * bent.z).sqrt();
    if len == 0.0 {
        return forward;
    }
    bent = vec3(bent.x / len, bent.y / len, bent.z / len);
    bent
}

/// Whether stock auto-aim may acquire `candidate`: damageable at the
/// `DAMAGE_AIM` level, and never a teammate. Players always qualify;
/// monsters qualify once `walkmonster_start_go` arms them.
fn q1_is_aim_target<L: ServerLogic>(
    server: &Server<L>,
    behaviors: &Q1NativeBehaviors,
    player: &ActorId,
    candidate: &ActorId,
) -> bool {
    let simulation = server.simulation();
    if !q1_can_take_damage(simulation, candidate) {
        return false;
    }
    if candidate != player
        && !behaviors
            .monsters
            .get(candidate)
            .is_some_and(|monster| monster.takedamage == Q1_DAMAGE_AIM)
    {
        return false;
    }
    match (
        simulation.combat_state(player).and_then(|combat| combat.team.clone()),
        simulation
            .combat_state(candidate)
            .and_then(|combat| combat.team.clone()),
    ) {
        (Some(mine), Some(theirs)) => mine != theirs,
        _ => true,
    }
}

/// Live fire context: the server, gamecode, and scene a weapon fire
/// reads and writes, plus the shooter's view for the step.
pub struct Q1WeaponFire<'server, 'behaviors, 'scene, L: ServerLogic> {
    /// Live server (bodies, combat, movers, triggers).
    pub server: &'server mut Server<L>,
    /// Live native gamecode state.
    pub behaviors: &'behaviors mut Q1NativeBehaviors,
    /// Relinked collision scene for `traceline`.
    pub scene: &'scene SharedSceneQueries,
    /// Firing player.
    pub player: ActorId,
    /// Player view angles in degrees (`self.v_angle`).
    pub view_angles: Vec3,
    /// Master-clock seconds (`time`).
    pub now: f64,
}

/// Stock `W_FireAxe` (`weapons.qc:37`): a 64-unit melee trace for 20
/// damage, or a wall impact (sound plus `TE_GUNSHOT`) on a clean miss
/// of anything damageable.
pub fn q1_fire_axe<L: ServerLogic>(ctx: &mut Q1WeaponFire<'_, '_, '_, L>) {
    let vectors = angle_vectors(ctx.view_angles);
    let Some(body) = ctx.server.simulation().body_state(&ctx.player) else {
        return;
    };
    let start = vec3(body.origin.x, body.origin.y, body.origin.z + 16.0);
    let end = vec3(
        start.x + vectors.forward.x * 64.0,
        start.y + vectors.forward.y * 64.0,
        start.z + vectors.forward.z * 64.0,
    );
    let hit = q1_traceline(ctx.scene, start, end, SceneQ1MoveRule::Normal, &ctx.player);
    if hit.fraction == 1.0 {
        return;
    }
    let org = vec3(
        hit.endpos.x - vectors.forward.x * 4.0,
        hit.endpos.y - vectors.forward.y * 4.0,
        hit.endpos.z - vectors.forward.z * 4.0,
    );
    let damageable = hit
        .hit_actor
        .as_ref()
        .is_some_and(|actor| q1_can_take_damage(ctx.server.simulation(), actor));
    if damageable {
        let victim = hit.hit_actor.clone().expect("damageable hit actor");
        // Stock marks axe victims for the pain sound (`weapons.qc:52`);
        // only players read the mark (`PainSound`, `player.qc:287`).
        if Some(&victim) == ctx.behaviors.player.as_ref() {
            ctx.behaviors.player_state.axhitme = true;
        }
        let player = ctx.player.clone();
        let (simulation, movers, triggers) = ctx.server.simulation_movers_and_triggers_mut();
        q1_t_damage(
            ctx.behaviors,
            simulation,
            movers,
            triggers,
            &victim,
            Some(&player),
            Some(&player),
            20.0,
        );
    } else {
        q1_monster_sound(ctx.behaviors, &ctx.player, 1, "player/axhit2.wav", 1.0, 1.0);
        ctx.behaviors.temp_ents.push(Q1TempEnt::Gunshot { at: org });
    }
}

/// Stock `FireBullets` (`weapons.qc:236`): `shots` pellets down `dir`
/// with `spread`, each a 2048-unit trace for 4 damage, combined per
/// victim through multi-damage (`ClearMultiDamage` /
/// `AddMultiDamage` / `ApplyMultiDamage`, `weapons.qc:162`).
pub fn q1_fire_bullets<L: ServerLogic>(ctx: &mut Q1WeaponFire<'_, '_, '_, L>, shots: u32, dir: Vec3, spread: Vec3) {
    let vectors = angle_vectors(ctx.view_angles);
    let Some(body) = ctx.server.simulation().body_state(&ctx.player) else {
        return;
    };
    let bounds = translated_body_bounds(&body);
    let size_z = bounds.max.z - bounds.min.z;
    let src = vec3(
        body.origin.x + vectors.forward.x * 10.0,
        body.origin.y + vectors.forward.y * 10.0,
        bounds.min.z + size_z * 0.7,
    );
    let mut multi_ent: Option<ActorId> = None;
    let mut multi_damage = 0.0;
    for _ in 0..shots {
        let rx = q1_monster_crandom(ctx.behaviors);
        let ry = q1_monster_crandom(ctx.behaviors);
        // Stock draws one `crandom` per axis per pellet (`FireBullets`
        // calls `crandom()` twice per pellet); `TraceAttack` draws two
        // more per impact for the blood spray.
        let direction = vec3(
            dir.x + rx * spread.x * vectors.right.x + ry * spread.y * vectors.up.x,
            dir.y + rx * spread.x * vectors.right.y + ry * spread.y * vectors.up.y,
            dir.z + rx * spread.x * vectors.right.z + ry * spread.y * vectors.up.z,
        );
        let end = vec3(
            src.x + direction.x * 2048.0,
            src.y + direction.y * 2048.0,
            src.z + direction.z * 2048.0,
        );
        let hit = q1_traceline(ctx.scene, src, end, SceneQ1MoveRule::Normal, &ctx.player);
        if hit.fraction == 1.0 {
            continue;
        }
        let _blood_x = q1_monster_crandom(ctx.behaviors);
        let _blood_y = q1_monster_crandom(ctx.behaviors);
        let org = vec3(
            hit.endpos.x - direction.x * 4.0,
            hit.endpos.y - direction.y * 4.0,
            hit.endpos.z - direction.z * 4.0,
        );
        let damageable = hit
            .hit_actor
            .as_ref()
            .is_some_and(|actor| q1_can_take_damage(ctx.server.simulation(), actor));
        if damageable {
            let victim = hit.hit_actor.clone().expect("damageable pellet victim");
            ctx.behaviors.temp_ents.push(Q1TempEnt::Blood { at: org, count: 4 });
            if multi_ent.as_ref() != Some(&victim) {
                q1_apply_multi_damage(ctx, multi_ent.take(), multi_damage);
                multi_ent = Some(victim);
                multi_damage = 4.0;
            } else {
                multi_damage += 4.0;
            }
        } else {
            ctx.behaviors.temp_ents.push(Q1TempEnt::Gunshot { at: org });
        }
    }
    q1_apply_multi_damage(ctx, multi_ent.take(), multi_damage);
}

/// Stock `ApplyMultiDamage` (`weapons.qc:168`): flush one victim's
/// accumulated pellet damage through `T_Damage`.
fn q1_apply_multi_damage<L: ServerLogic>(ctx: &mut Q1WeaponFire<'_, '_, '_, L>, victim: Option<ActorId>, damage: f64) {
    let Some(victim) = victim else {
        return;
    };
    let player = ctx.player.clone();
    let (simulation, movers, triggers) = ctx.server.simulation_movers_and_triggers_mut();
    q1_t_damage(
        ctx.behaviors,
        simulation,
        movers,
        triggers,
        &victim,
        Some(&player),
        Some(&player),
        damage,
    );
}

/// Stock `W_FireShotgun` (`weapons.qc:265`): one shell, 6 pellets at
/// 0.04 spread, view kick -2.
pub fn q1_fire_shotgun<L: ServerLogic>(ctx: &mut Q1WeaponFire<'_, '_, '_, L>) {
    q1_monster_sound(ctx.behaviors, &ctx.player, 1, "weapons/guncock.wav", 1.0, 1.0);
    ctx.behaviors.player_state.punchangle = vec3(-2.0, 0.0, 0.0);
    ctx.behaviors.player_ammo.shells -= 1.0;
    ctx.behaviors.player_state.currentammo = ctx.behaviors.player_ammo.shells;
    let vectors = angle_vectors(ctx.view_angles);
    let dir = q1_aim(ctx.server, ctx.behaviors, ctx.scene, &ctx.player, vectors.forward);
    q1_fire_bullets(ctx, 6, dir, vec3(0.04, 0.04, 0.0));
}

/// Stock `W_FireSuperShotgun` (`weapons.qc:284`): two shells, 14
/// pellets at 0.14/0.08 spread, view kick -4; a single remaining
/// shell fires the plain shotgun instead.
pub fn q1_fire_super_shotgun<L: ServerLogic>(ctx: &mut Q1WeaponFire<'_, '_, '_, L>) {
    if ctx.behaviors.player_state.currentammo == 1.0 {
        q1_fire_shotgun(ctx);
        return;
    }
    q1_monster_sound(ctx.behaviors, &ctx.player, 1, "weapons/shotgn2.wav", 1.0, 1.0);
    ctx.behaviors.player_state.punchangle = vec3(-4.0, 0.0, 0.0);
    ctx.behaviors.player_ammo.shells -= 2.0;
    ctx.behaviors.player_state.currentammo = ctx.behaviors.player_ammo.shells;
    let vectors = angle_vectors(ctx.view_angles);
    let dir = q1_aim(ctx.server, ctx.behaviors, ctx.scene, &ctx.player, vectors.forward);
    q1_fire_bullets(ctx, 14, dir, vec3(0.14, 0.08, 0.0));
}

/// Stock `W_Attack` (`weapons.qc:878`): the ammo gate, the monster
/// wakeup, and the per-weapon fire. Axe damage lands 0.2 s later at
/// the swing's frame 3 (`player.qc:152`); nailguns and the lightning
/// gun keep firing on their thinks while held.
pub fn q1_w_attack<L: ServerLogic>(ctx: &mut Q1WeaponFire<'_, '_, '_, L>) {
    if !q1_w_check_no_ammo(ctx.behaviors) {
        return;
    }
    ctx.behaviors.player_state.show_hostile = ctx.now + 1.0;
    let weapon = ctx.behaviors.player_state.weapon;
    if weapon == Q1_IT_AXE {
        q1_monster_sound(ctx.behaviors, &ctx.player, 1, "weapons/ax1.wav", 1.0, 1.0);
        // One of four swing anims (`random()` picks); all fire frame 3.
        let _swing = q1_monster_random(ctx.behaviors);
        ctx.behaviors.player_state.weaponframe = 1;
        ctx.behaviors.player_state.attack = Q1PlayerAttack::AxeSwing {
            fire_at: ctx.now + 0.2,
            until: ctx.now + 0.4,
        };
        ctx.behaviors.player_state.attack_finished = ctx.now + 0.5;
    } else if weapon == Q1_IT_SHOTGUN {
        ctx.behaviors.player_state.weaponframe = 1;
        ctx.behaviors.player_state.attack = Q1PlayerAttack::Gun { fired_at: ctx.now };
        q1_fire_shotgun(ctx);
        ctx.behaviors.player_state.attack_finished = ctx.now + 0.5;
    } else if weapon == Q1_IT_SUPER_SHOTGUN {
        ctx.behaviors.player_state.weaponframe = 1;
        ctx.behaviors.player_state.attack = Q1PlayerAttack::Gun { fired_at: ctx.now };
        q1_fire_super_shotgun(ctx);
        ctx.behaviors.player_state.attack_finished = ctx.now + 0.7;
    } else if weapon == Q1_IT_NAILGUN || weapon == Q1_IT_SUPER_NAILGUN {
        q1_start_nail_burst(ctx);
    } else if weapon == Q1_IT_GRENADE_LAUNCHER {
        ctx.behaviors.player_state.weaponframe = 1;
        ctx.behaviors.player_state.attack = Q1PlayerAttack::Gun { fired_at: ctx.now };
        q1_fire_grenade(ctx);
        ctx.behaviors.player_state.attack_finished = ctx.now + 0.6;
    } else if weapon == Q1_IT_ROCKET_LAUNCHER {
        ctx.behaviors.player_state.weaponframe = 1;
        ctx.behaviors.player_state.attack = Q1PlayerAttack::Gun { fired_at: ctx.now };
        q1_fire_rocket(ctx);
        ctx.behaviors.player_state.attack_finished = ctx.now + 0.8;
    } else if weapon == Q1_IT_LIGHTNING {
        q1_start_light_burst(ctx);
    }
}

/// Stock lightning entry (`W_Attack`, `weapons.qc:882`): the first shot
/// leaves through `player_light1`, the 0.1 s cadence arms, and the
/// generator whines on `CHAN_AUTO` (`W_CheckNoAmmo` already guaranteed
/// a cell, so entry always fires).
fn q1_start_light_burst<L: ServerLogic>(ctx: &mut Q1WeaponFire<'_, '_, '_, L>) {
    ctx.behaviors.player_state.weaponframe = 1;
    q1_fire_lightning(ctx);
    ctx.behaviors.player_state.attack_finished = ctx.now + 0.1;
    q1_monster_sound(ctx.behaviors, &ctx.player, 0, "weapons/lstart.wav", 1.0, 1.0);
    ctx.behaviors.player_state.attack = Q1PlayerAttack::Lightning {
        next_fire: ctx.now + 0.1,
    };
}

/// Stock `player_nail1` entry (`player.qc:173`): fire the first spike
/// off the current barrel, flip barrels, and arm the 0.1 s think.
fn q1_start_nail_burst<L: ServerLogic>(ctx: &mut Q1WeaponFire<'_, '_, '_, L>) {
    ctx.behaviors.player_state.weaponframe = 1;
    let side = ctx.behaviors.player_state.nail_side;
    q1_fire_spikes(ctx, side);
    ctx.behaviors.player_state.nail_side = -side;
    // A dry fire re-arms through the best weapon (`W_SetCurrentAmmo`
    // retires the burst); otherwise the think owns the trigger now.
    if ctx.behaviors.player_state.attack == Q1PlayerAttack::None
        && (ctx.behaviors.player_state.weapon == Q1_IT_NAILGUN
            || ctx.behaviors.player_state.weapon == Q1_IT_SUPER_NAILGUN)
    {
        ctx.behaviors.player_state.attack = Q1PlayerAttack::Nail {
            next_fire: ctx.now + 0.1,
        };
    }
}

/// Run one due attack-anim think: the axe swing fires at frame 3
/// and retires at frame 4; the shot/rocket anims run six frames;
/// the nail and lightning bursts fire every 0.1 s while held (stock
/// `player_nail1`/`nail2`, `player.qc:173`, and
/// `player_light1`/`light2`, `player.qc:202`) and retire on release.
/// Idleness clears the view-model frame (`player_run`).
fn q1_attack_think<L: ServerLogic>(ctx: &mut Q1WeaponFire<'_, '_, '_, L>, buttons: i32) {
    let attack = ctx.behaviors.player_state.attack;
    match attack {
        // Stock `player_run`/`player_stand` clear the view-model
        // frame every think (`player.qc:93/118`).
        Q1PlayerAttack::None => {
            ctx.behaviors.player_state.weaponframe = 0;
        }
        Q1PlayerAttack::AxeSwing { fire_at, until } => {
            if ctx.now >= fire_at && ctx.behaviors.player_state.weaponframe < 3 {
                ctx.behaviors.player_state.weaponframe = 3;
                q1_fire_axe(ctx);
            }
            if ctx.now >= until {
                ctx.behaviors.player_state.attack = Q1PlayerAttack::None;
                ctx.behaviors.player_state.weaponframe = 0;
            }
        }
        Q1PlayerAttack::Gun { fired_at } => {
            #[allow(clippy::cast_possible_truncation)]
            let step = ((ctx.now - fired_at) / 0.1).floor() as i32;
            if step >= 6 {
                ctx.behaviors.player_state.attack = Q1PlayerAttack::None;
                ctx.behaviors.player_state.weaponframe = 0;
            } else {
                ctx.behaviors.player_state.weaponframe = step + 1;
            }
        }
        Q1PlayerAttack::Nail { next_fire } => {
            if buttons & Q1_BUTTON_ATTACK == 0 {
                ctx.behaviors.player_state.attack = Q1PlayerAttack::None;
                return;
            }
            if ctx.now >= next_fire {
                let frame = ctx.behaviors.player_state.weaponframe + 1;
                ctx.behaviors.player_state.weaponframe = if frame == 9 { 1 } else { frame };
                let side = ctx.behaviors.player_state.nail_side;
                q1_fire_spikes(ctx, side);
                ctx.behaviors.player_state.nail_side = -side;
                if ctx.behaviors.player_state.attack != Q1PlayerAttack::None {
                    ctx.behaviors.player_state.attack = Q1PlayerAttack::Nail {
                        next_fire: ctx.now + 0.1,
                    };
                }
            }
        }
        Q1PlayerAttack::Pain { until } => {
            if ctx.now >= until {
                ctx.behaviors.player_state.attack = Q1PlayerAttack::None;
            }
        }
        Q1PlayerAttack::Lightning { next_fire } => {
            if buttons & Q1_BUTTON_ATTACK == 0 {
                ctx.behaviors.player_state.attack = Q1PlayerAttack::None;
                return;
            }
            if ctx.now >= next_fire {
                let frame = ctx.behaviors.player_state.weaponframe + 1;
                ctx.behaviors.player_state.weaponframe = if frame == 5 { 1 } else { frame };
                q1_fire_lightning(ctx);
                ctx.behaviors.player_state.attack_finished = ctx.now + 0.1;
                if ctx.behaviors.player_state.attack != Q1PlayerAttack::None {
                    ctx.behaviors.player_state.attack = Q1PlayerAttack::Lightning {
                        next_fire: ctx.now + 0.1,
                    };
                }
            }
        }
    }
}

/// Stock `W_WeaponFrame` (`weapons.qc:1257`): impulses select while
/// the trigger is idle, and a held trigger attacks once the refire
/// gate opens. (The quad growl waits for the powerup slice.)
pub fn q1_w_weapon_frame<L: ServerLogic>(ctx: &mut Q1WeaponFire<'_, '_, '_, L>, buttons: i32, impulse: i32) {
    if ctx.now < ctx.behaviors.player_state.attack_finished {
        return;
    }
    if impulse != 0 {
        let player = ctx.player.clone();
        q1_impulse_commands(ctx.behaviors, &player, impulse);
    }
    if buttons & Q1_BUTTON_ATTACK != 0 {
        q1_w_attack(ctx);
    }
}

/// Run one weapon pass for the player: the idle ammo downgrade
/// (`PlayerPreThink`, `client.qc:941`), the weapon frame, and the
/// attack-anim think. Dead players neither downgrade nor fire (death
/// thinks own them once the player slice lands).
pub fn q1_weapon_pass<L: ServerLogic>(
    server: &mut Server<L>,
    behaviors: &mut Q1NativeBehaviors,
    scene: &SharedSceneQueries,
    player: &ActorId,
    view_angles: Vec3,
    buttons: i32,
    impulse: i32,
) {
    let now = server.simulation().frame().time.as_seconds_f64();
    if behaviors.player_state.deadflag != Q1_DEAD_NO {
        let (simulation, _, _) = server.simulation_movers_and_triggers_mut();
        q1_player_death_think(behaviors, simulation, player, now, buttons);
        let mut ctx = Q1WeaponFire {
            server,
            behaviors,
            scene,
            player: player.clone(),
            view_angles,
            now,
        };
        q1_missile_pass(&mut ctx);
        return;
    }
    if now > behaviors.player_state.attack_finished
        && behaviors.player_state.currentammo == 0.0
        && behaviors.player_state.weapon != Q1_IT_AXE
    {
        behaviors.player_state.weapon = q1_w_best_weapon(behaviors);
        q1_w_set_current_ammo(behaviors);
    }
    let mut ctx = Q1WeaponFire {
        server,
        behaviors,
        scene,
        player: player.clone(),
        view_angles,
        now,
    };
    q1_w_weapon_frame(&mut ctx, buttons, impulse);
    q1_attack_think(&mut ctx, buttons);
    q1_missile_pass(&mut ctx);
}

/// Stock water level (`SV_CheckWater` shape): the deepest of feet,
/// waist, and eye samples that sits in liquid, counting up only
/// through consecutive liquid from the feet.
pub fn q1_sample_water_level(
    scene: &SharedSceneQueries,
    simulation: &qa_world::session::Simulation,
    player: &ActorId,
) -> i32 {
    let Some(body) = simulation.body_state(player) else {
        return 0;
    };
    let feet = body.origin.z + body.bounds.min.z + 1.0;
    let waist = body.origin.z + 10.0;
    let eye = body.origin.z + 22.0;
    let mut level = 0;
    for z in [feet, waist, eye] {
        let query = ScenePointContentsQuery {
            point: vec3(body.origin.x, body.origin.y, z),
            target: SceneQueryTarget::World,
            policy: SceneTracePolicy::Q1 {
                move_rule: SceneQ1MoveRule::Normal,
                hull: None,
            },
            numeric: Q1_DONOR_PROFILE,
            pass_actor: Some(player.clone()),
        };
        let liquid = matches!(
            scene.point_contents(&query),
            Ok(ScenePointContentsResult::Q1 { contents })
                if contents == Q1_CONTENTS_WATER
                    || contents == Q1_CONTENTS_SLIME
                    || contents == Q1_CONTENTS_LAVA
        );
        if !liquid {
            break;
        }
        level += 1;
    }
    level
}

/// Stock missile kinds (`launch_spike`, `W_FireGrenade`, `W_FireRocket`,
/// `weapons.qc:607-747`; `LaunchLaser`, `enforcer.qc:74`;
/// `OgreFireGrenade`, `ogre.qc:90`; `ZombieFireGrenade`, `zombie.qc`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q1MissileKind {
    /// Nailgun spike: 9 damage (`spike_touch`).
    Spike,
    /// Super-nailgun spike: 18 damage (`superspike_touch`).
    SuperSpike,
    /// Rocket: 100-120 direct plus 120 radius (`T_MissileTouch`).
    Rocket,
    /// Grenade: 120 radius on fuse or damageable contact (`GrenadeTouch`).
    Grenade,
    /// Enforcer laser: 15 damage to the living, else a wall puff
    /// (`Laser_Touch`, `enforcer.qc:41`).
    Laser,
    /// Ogre grenade: 40 radius on fuse or `DAMAGE_AIM` contact
    /// (`OgreGrenadeTouch`, `ogre.qc:71`).
    OgreGrenade,
    /// Zombie flesh chunk: 10 damage to the damageable, else a miss
    /// thud and a dead stop, removing on the next touch
    /// (`ZombieGrenadeTouch`, `zombie.qc`).
    ZombieFlesh,
    /// Scrag spike: 9 damage, `TE_WIZSPIKE` walls (`spike_touch`
    /// with classname `wizspike`, `weapons.qc:675`). The green tracer
    /// rides the `w_spike.mdl` model flags, not gamecode.
    WizSpike,
    /// Hell knight spike: 9 damage, `TE_KNIGHTSPIKE` walls
    /// (`spike_touch` with classname `knightspike`,
    /// `weapons.qc:675`).
    KnightSpike,
}

impl Q1MissileKind {
    /// Stock classname for the kind.
    #[must_use]
    pub fn classname(self) -> &'static str {
        match self {
            Q1MissileKind::Spike => "spike",
            Q1MissileKind::SuperSpike => "superspike",
            Q1MissileKind::Rocket => "missile",
            Q1MissileKind::Grenade => "grenade",
            Q1MissileKind::Laser => "laser",
            Q1MissileKind::OgreGrenade => "grenade",
            Q1MissileKind::ZombieFlesh => "zombie_flesh",
            Q1MissileKind::WizSpike => "wizspike",
            Q1MissileKind::KnightSpike => "knightspike",
        }
    }
}

/// Live missile record: spikes, rockets, lasers, and grenades in flight.
/// Position and velocity ride the sim body; the fuse and kind ride here.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1Missile {
    /// Missile kind.
    pub kind: Q1MissileKind,
    /// Firing player (touches skip the owner, stock `other == owner`).
    pub owner: ActorId,
    /// Angular velocity in degrees per second (grenades spin 300s).
    pub avelocity: Vec3,
    /// Entity effects bits (`EF_*`; the presentation slice drains them).
    pub effects: i32,
    /// Master-clock fuse instant for grenades (`None` otherwise).
    pub fuse_at: Option<f64>,
    /// Master-clock removal instant (spikes 6 s, rockets 5 s).
    pub remove_at: f64,
    /// Whether the grenade settled (stock `FL_ONGROUND` rest).
    pub onground: bool,
    /// Master-clock spawn instant: stock moves missiles from the next
    /// frame on (`SV_Physics` runs after gamecode), so the spawn pass
    /// never moves them.
    pub born_at: f64,
    /// Whether a flesh chunk already missed once (`touch = SUB_Remove`,
    /// `zombie.qc`): the next touch removes it. Other kinds leave this
    /// false.
    pub spent: bool,
}

/// Stock `spawn()` parameters for one missile: the fire functions fill
/// every field (stock assigns each edict field explicitly).
pub struct Q1MissileSpawn {
    /// Spike, super spike, scrag spike, grenade, rocket, laser, or
    /// ogre grenade.
    pub kind: Q1MissileKind,
    /// Firer (touch immunity, radius credit).
    pub owner: ActorId,
    /// Spawn origin.
    pub origin: Vec3,
    /// Initial velocity.
    pub velocity: Vec3,
    /// Spin in degrees per second (grenades tumble at 300).
    pub avelocity: Vec3,
    /// Entity effects bits (`EF_*`; lasers fly lit).
    pub effects: i32,
    /// Master-clock detonation for grenades (`None` flies straight).
    pub fuse_at: Option<f64>,
    /// Master-clock `SUB_Remove` for flyers.
    pub remove_at: f64,
    /// Master-clock spawn instant (no movement in the spawn pass).
    pub born_at: f64,
}

/// Stock `spawn()` for a missile: a point-sized unsolid body with toss
/// velocity plus its gamecode record. Drops on spawn failure (callers
/// are infallible gamecode).
pub fn q1_spawn_missile<L: ServerLogic>(
    server: &mut Server<L>,
    behaviors: &mut Q1NativeBehaviors,
    spawn: Q1MissileSpawn,
) -> Option<ActorId> {
    let fields = SpawnFields {
        classname: "q1:missile".to_string(),
        origin: spawn.origin,
        ..SpawnFields::default()
    };
    let actor = server.spawn_entity(&fields).ok()?;
    let simulation = server.simulation_mut();
    let point = Bounds {
        min: vec3(0.0, 0.0, 0.0),
        max: vec3(0.0, 0.0, 0.0),
    };
    if simulation.set_body_bounds(actor.id(), point).is_err()
        || simulation.set_body_velocity(actor.id(), spawn.velocity).is_err()
    {
        let _ignored = simulation.release(&actor);
        return None;
    }
    behaviors.missiles.insert(
        actor.id(),
        Q1Missile {
            kind: spawn.kind,
            owner: spawn.owner,
            avelocity: spawn.avelocity,
            effects: spawn.effects,
            fuse_at: spawn.fuse_at,
            remove_at: spawn.remove_at,
            onground: false,
            born_at: spawn.born_at,
            spent: false,
        },
    );
    Some(actor.id().clone())
}

/// Stock `launch_spike` (`weapons.qc:607`): a 1000 u/s flymissile from
/// `origin` down `dir`, living 6 s.
pub fn q1_launch_spike<L: ServerLogic>(
    ctx: &mut Q1WeaponFire<'_, '_, '_, L>,
    origin: Vec3,
    dir: Vec3,
    super_spike: bool,
) {
    let velocity = vec3(dir.x * 1000.0, dir.y * 1000.0, dir.z * 1000.0);
    q1_spawn_missile(
        ctx.server,
        ctx.behaviors,
        Q1MissileSpawn {
            kind: if super_spike {
                Q1MissileKind::SuperSpike
            } else {
                Q1MissileKind::Spike
            },
            owner: ctx.player.clone(),
            origin,
            velocity,
            avelocity: vec3(0.0, 0.0, 0.0),
            effects: 0,
            fuse_at: None,
            remove_at: ctx.now + 6.0,
            born_at: ctx.now,
        },
    );
}

/// Stock `W_FireSuperSpikes` (`weapons.qc:627`): two nails, an
/// 18-damage super spike, view kick -2.
pub fn q1_fire_super_spikes<L: ServerLogic>(ctx: &mut Q1WeaponFire<'_, '_, '_, L>) {
    q1_monster_sound(ctx.behaviors, &ctx.player, 1, "weapons/spike2.wav", 1.0, 1.0);
    ctx.behaviors.player_state.attack_finished = ctx.now + 0.2;
    ctx.behaviors.player_ammo.nails -= 2.0;
    ctx.behaviors.player_state.currentammo = ctx.behaviors.player_ammo.nails;
    let vectors = angle_vectors(ctx.view_angles);
    let dir = q1_aim(ctx.server, ctx.behaviors, ctx.scene, &ctx.player, vectors.forward);
    let Some(body) = ctx.server.simulation().body_state(&ctx.player) else {
        return;
    };
    q1_launch_spike(ctx, vec3(body.origin.x, body.origin.y, body.origin.z + 16.0), dir, true);
    ctx.behaviors.player_state.punchangle = vec3(-2.0, 0.0, 0.0);
}

/// Stock `W_FireSpikes` (`weapons.qc:643`): the super nailgun spends
/// two nails per shot while it can (one nail left fires the plain
/// nailgun's single spike instead); the nailgun spends one. Firing
/// dry re-arms through the best weapon and returns.
pub fn q1_fire_spikes<L: ServerLogic>(ctx: &mut Q1WeaponFire<'_, '_, '_, L>, side: f32) {
    if ctx.behaviors.player_ammo.nails >= 2.0 && ctx.behaviors.player_state.weapon == Q1_IT_SUPER_NAILGUN {
        q1_fire_super_spikes(ctx);
        return;
    }
    if ctx.behaviors.player_ammo.nails < 1.0 {
        ctx.behaviors.player_state.weapon = q1_w_best_weapon(ctx.behaviors);
        q1_w_set_current_ammo(ctx.behaviors);
        return;
    }
    q1_monster_sound(ctx.behaviors, &ctx.player, 1, "weapons/rocket1i.wav", 1.0, 1.0);
    ctx.behaviors.player_state.attack_finished = ctx.now + 0.2;
    ctx.behaviors.player_ammo.nails -= 1.0;
    ctx.behaviors.player_state.currentammo = ctx.behaviors.player_ammo.nails;
    let vectors = angle_vectors(ctx.view_angles);
    let dir = q1_aim(ctx.server, ctx.behaviors, ctx.scene, &ctx.player, vectors.forward);
    let Some(body) = ctx.server.simulation().body_state(&ctx.player) else {
        return;
    };
    q1_launch_spike(
        ctx,
        vec3(
            body.origin.x + vectors.right.x * side,
            body.origin.y + vectors.right.y * side,
            body.origin.z + 16.0 + vectors.right.z * side,
        ),
        dir,
        false,
    );
    ctx.behaviors.player_state.punchangle = vec3(-2.0, 0.0, 0.0);
}

/// Stock `W_FireGrenade` (`weapons.qc:549`): one rocket, a bouncing
/// grenade with a 2.5 s fuse, view kick -2. A pitched view lobs along
/// it; a level view fires the aim direction flat with a +200 rise.
pub fn q1_fire_grenade<L: ServerLogic>(ctx: &mut Q1WeaponFire<'_, '_, '_, L>) {
    ctx.behaviors.player_ammo.rockets -= 1.0;
    ctx.behaviors.player_state.currentammo = ctx.behaviors.player_ammo.rockets;
    q1_monster_sound(ctx.behaviors, &ctx.player, 1, "weapons/grenade.wav", 1.0, 1.0);
    ctx.behaviors.player_state.punchangle = vec3(-2.0, 0.0, 0.0);
    let vectors = angle_vectors(ctx.view_angles);
    let velocity = if ctx.view_angles.x != 0.0 {
        let jx = q1_monster_crandom(ctx.behaviors);
        let jy = q1_monster_crandom(ctx.behaviors);
        vec3(
            vectors.forward.x * 600.0 + vectors.up.x * 200.0 + jx * vectors.right.x * 10.0 + jy * vectors.up.x * 10.0,
            vectors.forward.y * 600.0 + vectors.up.y * 200.0 + jx * vectors.right.y * 10.0 + jy * vectors.up.y * 10.0,
            vectors.forward.z * 600.0 + vectors.up.z * 200.0 + jx * vectors.right.z * 10.0 + jy * vectors.up.z * 10.0,
        )
    } else {
        let dir = q1_aim(ctx.server, ctx.behaviors, ctx.scene, &ctx.player, vectors.forward);
        vec3(dir.x * 600.0, dir.y * 600.0, 200.0)
    };
    let Some(body) = ctx.server.simulation().body_state(&ctx.player) else {
        return;
    };
    q1_spawn_missile(
        ctx.server,
        ctx.behaviors,
        Q1MissileSpawn {
            kind: Q1MissileKind::Grenade,
            owner: ctx.player.clone(),
            origin: body.origin,
            velocity,
            avelocity: vec3(300.0, 300.0, 300.0),
            effects: 0,
            fuse_at: Some(ctx.now + 2.5),
            remove_at: ctx.now + 2.5,
            born_at: ctx.now,
        },
    );
}

/// Stock `W_FireRocket` (`weapons.qc:374`): one rocket, a 1000 u/s
/// missile living 5 s, view kick -2.
pub fn q1_fire_rocket<L: ServerLogic>(ctx: &mut Q1WeaponFire<'_, '_, '_, L>) {
    ctx.behaviors.player_ammo.rockets -= 1.0;
    ctx.behaviors.player_state.currentammo = ctx.behaviors.player_ammo.rockets;
    q1_monster_sound(ctx.behaviors, &ctx.player, 1, "weapons/sgun1.wav", 1.0, 1.0);
    ctx.behaviors.player_state.punchangle = vec3(-2.0, 0.0, 0.0);
    let vectors = angle_vectors(ctx.view_angles);
    let dir = q1_aim(ctx.server, ctx.behaviors, ctx.scene, &ctx.player, vectors.forward);
    let Some(body) = ctx.server.simulation().body_state(&ctx.player) else {
        return;
    };
    q1_spawn_missile(
        ctx.server,
        ctx.behaviors,
        Q1MissileSpawn {
            kind: Q1MissileKind::Rocket,
            owner: ctx.player.clone(),
            origin: vec3(
                body.origin.x + vectors.forward.x * 8.0,
                body.origin.y + vectors.forward.y * 8.0,
                body.origin.z + vectors.forward.z * 8.0 + 16.0,
            ),
            velocity: vec3(dir.x * 1000.0, dir.y * 1000.0, dir.z * 1000.0),
            avelocity: vec3(0.0, 0.0, 0.0),
            effects: 0,
            fuse_at: None,
            remove_at: ctx.now + 5.0,
            born_at: ctx.now,
        },
    );
}

/// Stock `W_FireLightning` (`weapons.qc:465`): one cell down a
/// straight 600-unit beam for 30 damage, the impact crack throttled
/// to 0.6 s. Firing dry re-arms through the best weapon; firing
/// past waist-deep water discharges every cell into a `35 * cells`
/// radius blast instead (no beam, no sound, stock shows nothing).
pub fn q1_fire_lightning<L: ServerLogic>(ctx: &mut Q1WeaponFire<'_, '_, '_, L>) {
    if ctx.behaviors.player_ammo.cells < 1.0 {
        ctx.behaviors.player_state.weapon = q1_w_best_weapon(ctx.behaviors);
        q1_w_set_current_ammo(ctx.behaviors);
        return;
    }
    if ctx.behaviors.player_state.water_level > 1 {
        let cells = ctx.behaviors.player_ammo.cells;
        ctx.behaviors.player_ammo.cells = 0.0;
        q1_w_set_current_ammo(ctx.behaviors);
        let player = ctx.player.clone();
        q1_t_radius_damage(ctx, &player, &player, 35.0 * cells, None);
        return;
    }
    if ctx.behaviors.player_state.t_width < ctx.now {
        q1_monster_sound(ctx.behaviors, &ctx.player, 1, "weapons/lhit.wav", 1.0, 1.0);
        ctx.behaviors.player_state.t_width = ctx.now + 0.6;
    }
    ctx.behaviors.player_state.punchangle = vec3(-2.0, 0.0, 0.0);
    ctx.behaviors.player_ammo.cells -= 1.0;
    ctx.behaviors.player_state.currentammo = ctx.behaviors.player_ammo.cells;
    let vectors = angle_vectors(ctx.view_angles);
    let Some(body) = ctx.server.simulation().body_state(&ctx.player) else {
        return;
    };
    let org = vec3(body.origin.x, body.origin.y, body.origin.z + 16.0);
    let end = vec3(
        org.x + vectors.forward.x * 600.0,
        org.y + vectors.forward.y * 600.0,
        org.z + vectors.forward.z * 600.0,
    );
    // The beam visual ignores monsters (stock passes `TRUE` for
    // `nomonsters`); the damage traces below do not.
    let hit = q1_traceline(ctx.scene, org, end, SceneQ1MoveRule::NoMonsters, &ctx.player);
    ctx.behaviors.temp_ents.push(Q1TempEnt::Lightning {
        entity: ctx.player.clone(),
        start: org,
        end: hit.endpos,
    });
    let p2 = vec3(
        hit.endpos.x + vectors.forward.x * 4.0,
        hit.endpos.y + vectors.forward.y * 4.0,
        hit.endpos.z + vectors.forward.z * 4.0,
    );
    let from = ctx.player.clone();
    q1_lightning_damage(ctx, body.origin, p2, &from, 30.0);
}

/// Stock `LightningDamage` (`weapons.qc:421`): three 30-damage traces
/// (center plus two offset copies), each new victim wounded once.
/// The blue impact particles have no drain channel yet (stock
/// `particle()` is not a temp entity), and the deathmatch victim
/// launch keys off the engine-global `other` touch residue, which has
/// no deterministic value here — both stay out with this note.
pub(crate) fn q1_lightning_damage<L: ServerLogic>(
    ctx: &mut Q1WeaponFire<'_, '_, '_, L>,
    p1: Vec3,
    p2: Vec3,
    from: &ActorId,
    damage: f64,
) {
    let delta = vec3(p2.x - p1.x, p2.y - p1.y, p2.z - p1.z);
    let length = length3(delta);
    if length == 0.0 {
        return;
    }
    // Stock quirk (`weapons.qc:431`): the lanes assign in order, so
    // `f_y = f_x` reads the already-negated lane — both lanes land on
    // `-fy`, and the "parallel" traces skew instead of flanking.
    let unit = scale3(delta, 1.0 / length);
    let f = vec3(-unit.y * 16.0, -unit.y * 16.0, 0.0);
    let e1 = q1_lightning_bolt(ctx, p1, p2, from, damage, None, None);
    let e2 = q1_lightning_bolt(ctx, add3(p1, f), add3(p2, f), from, damage, e1.as_ref(), None);
    q1_lightning_bolt(
        ctx,
        vec3(p1.x - f.x, p1.y - f.y, p1.z - f.z),
        vec3(p2.x - f.x, p2.y - f.y, p2.z - f.z),
        from,
        damage,
        e1.as_ref(),
        e2.as_ref(),
    );
}

/// One lightning damage trace: the first blocking damageable not
/// already wounded by this shot takes the full damage.
fn q1_lightning_bolt<L: ServerLogic>(
    ctx: &mut Q1WeaponFire<'_, '_, '_, L>,
    p1: Vec3,
    p2: Vec3,
    from: &ActorId,
    damage: f64,
    skip1: Option<&ActorId>,
    skip2: Option<&ActorId>,
) -> Option<ActorId> {
    let hit = q1_traceline(ctx.scene, p1, p2, SceneQ1MoveRule::Normal, from);
    let victim = hit.hit_actor.clone();
    if let Some(ref target) = victim {
        if Some(target) != skip1 && Some(target) != skip2 && q1_can_take_damage(ctx.server.simulation(), target) {
            let (simulation, movers, triggers) = ctx.server.simulation_movers_and_triggers_mut();
            q1_t_damage(
                ctx.behaviors,
                simulation,
                movers,
                triggers,
                target,
                Some(from),
                Some(from),
                damage,
            );
        }
    }
    victim
}

/// Stock gravity in map units per second squared (`sv_gravity`, 800):
/// grenades integrate it every pass (`SV_Physics_Toss`, `sv_phys.c`).
const Q1_GRAVITY: f32 = 800.0;

/// Longest single missile trace in map units: a pass subdivides longer
/// travel so fast missiles cannot tunnel past thin targets at low tick
/// rates (the engine moves once per frame; slices only shorten legs).
const Q1_MISSILE_SUBSTEP: f32 = 20.0;

/// Run one missile pass: fuses and lifetimes first (stock `SV_RunThink`
/// runs before movement), then fly and bounce moves with touch dispatch.
fn q1_missile_pass<L: ServerLogic>(ctx: &mut Q1WeaponFire<'_, '_, '_, L>) {
    let dt = (ctx.server.simulation().frame().elapsed.as_seconds_f64() as f32).max(0.0);
    let actors: Vec<ActorId> = ctx.behaviors.missiles.keys().cloned().collect();
    for actor in actors {
        q1_missile_actor(ctx, &actor, dt);
    }
}

/// Run one missile record: the fuse and lifetime think before movement
/// (stock `SV_RunThink` order); settled grenades wait out their fuse.
fn q1_missile_actor<L: ServerLogic>(ctx: &mut Q1WeaponFire<'_, '_, '_, L>, actor: &ActorId, dt: f32) {
    let Some(missile) = ctx.behaviors.missiles.get(actor).cloned() else {
        return;
    };
    if missile.fuse_at.is_some_and(|at| at <= ctx.now) {
        // Only grenades carry fuses; the ogre's runs weaker and
        // flashes at the origin (`OgreGrenadeExplode`, `ogre.qc:53`).
        let (damage, backoff) = match missile.kind {
            Q1MissileKind::OgreGrenade => (Q1_GRENADE_DAMAGE, 0.0),
            _ => (120.0, 8.0),
        };
        q1_grenade_explode(ctx, actor, None, damage, backoff);
        return;
    }
    if missile.remove_at <= ctx.now {
        q1_remove_missile(ctx, actor);
        return;
    }
    if missile.onground {
        return;
    }
    if missile.born_at == ctx.now {
        return;
    }
    match missile.kind {
        Q1MissileKind::Spike
        | Q1MissileKind::SuperSpike
        | Q1MissileKind::WizSpike
        | Q1MissileKind::KnightSpike
        | Q1MissileKind::Rocket
        | Q1MissileKind::Laser => {
            q1_fly_missile(ctx, actor, &missile, dt);
        }
        Q1MissileKind::Grenade | Q1MissileKind::OgreGrenade | Q1MissileKind::ZombieFlesh => {
            q1_bounce_grenade(ctx, actor, &missile, dt);
        }
    }
}

/// Stock fly-missile move (`SV_Physics` `MOVETYPE_FLYMISSILE`): straight
/// travel with touch dispatch on the first blocking hit (no gravity,
/// no bounce; `sv_phys.c`).
fn q1_fly_missile<L: ServerLogic>(
    ctx: &mut Q1WeaponFire<'_, '_, '_, L>,
    actor: &ActorId,
    missile: &Q1Missile,
    dt: f32,
) {
    let Some(body) = ctx.server.simulation().body_state(actor) else {
        q1_remove_missile(ctx, actor);
        return;
    };
    let start = body.origin;
    let travel = scale3(body.velocity, dt);
    let steps = ((length3(travel) / Q1_MISSILE_SUBSTEP).ceil() as u32).max(1);
    let mut from = start;
    for step in 1..=steps {
        #[allow(clippy::cast_precision_loss)]
        let to = add3(start, scale3(travel, step as f32 / steps as f32));
        if !q1_fly_leg(ctx, actor, missile, from, to) {
            return;
        }
        from = to;
    }
    let _ignored = ctx.server.simulation_mut().set_body_origin(actor, from);
}

/// Trace one flight leg: `true` keeps flying past `to`, `false` means a
/// touch consumed the flight. Trigger hits never block (the engine
/// never clips triggers), so the leg re-traces past them; the owner
/// never appears (the leg trace excludes it, stock touch-refusal).
fn q1_fly_leg<L: ServerLogic>(
    ctx: &mut Q1WeaponFire<'_, '_, '_, L>,
    actor: &ActorId,
    missile: &Q1Missile,
    from: Vec3,
    to: Vec3,
) -> bool {
    let mut leg = from;
    for _ in 0..5 {
        let hit = q1_missile_leg_trace(ctx.scene, leg, to, actor, &missile.owner);
        if hit.fraction >= 1.0 {
            return true;
        }
        let _ignored = ctx.server.simulation_mut().set_body_origin(actor, hit.endpos);
        if q1_missile_impact(ctx, actor, missile, &hit) {
            return false;
        }
        leg = hit.endpos;
    }
    true
}

/// Stock toss move (`SV_Physics_Toss`, `sv_phys.c`): gravity integrates
/// before the push, and the clip runs after the touch each pass.
fn q1_bounce_grenade<L: ServerLogic>(
    ctx: &mut Q1WeaponFire<'_, '_, '_, L>,
    actor: &ActorId,
    missile: &Q1Missile,
    dt: f32,
) {
    let Some(body) = ctx.server.simulation().body_state(actor) else {
        q1_remove_missile(ctx, actor);
        return;
    };
    let velocity = vec3(body.velocity.x, body.velocity.y, body.velocity.z - Q1_GRAVITY * dt);
    // Stock commits gravity before the push, so a touch mid-pass clips
    // the integrated velocity, not last pass's.
    let _ignored = ctx.server.simulation_mut().set_body_velocity(actor, velocity);
    let start = body.origin;
    let travel = scale3(velocity, dt);
    let steps = ((length3(travel) / Q1_MISSILE_SUBSTEP).ceil() as u32).max(1);
    let mut from = start;
    for step in 1..=steps {
        #[allow(clippy::cast_precision_loss)]
        let to = add3(start, scale3(travel, step as f32 / steps as f32));
        if !q1_bounce_leg(ctx, actor, missile, from, to) {
            return;
        }
        from = to;
    }
    let _ignored = ctx.server.simulation_mut().set_body_origin(actor, from);
}

/// Trace one bounce leg: gravity already committed, so a blocking hit
/// rewrites the velocity through the bounce clip instead.
fn q1_bounce_leg<L: ServerLogic>(
    ctx: &mut Q1WeaponFire<'_, '_, '_, L>,
    actor: &ActorId,
    missile: &Q1Missile,
    from: Vec3,
    to: Vec3,
) -> bool {
    let mut leg = from;
    for _ in 0..5 {
        let hit = q1_traceline(ctx.scene, leg, to, SceneQ1MoveRule::Missile, actor);
        if hit.fraction >= 1.0 {
            return true;
        }
        let _ignored = ctx.server.simulation_mut().set_body_origin(actor, hit.endpos);
        if q1_missile_impact(ctx, actor, missile, &hit) {
            return false;
        }
        leg = hit.endpos;
    }
    true
}

/// Stock sky read for missile touches: `pointcontents(self.origin) ==
/// CONTENT_SKY` (`combat.qc` spike/rocket touches).
fn q1_point_is_sky(scene: &SharedSceneQueries, point: Vec3) -> bool {
    let query = ScenePointContentsQuery {
        point,
        target: SceneQueryTarget::World,
        policy: SceneTracePolicy::Q1 {
            move_rule: SceneQ1MoveRule::Missile,
            hull: None,
        },
        numeric: Q1_DONOR_PROFILE,
        pass_actor: None,
    };
    matches!(
        scene.point_contents(&query),
        Ok(ScenePointContentsResult::Q1 { contents }) if contents == CONTENTS_SKY
    )
}

/// Dispatch one missile impact: `true` ends the flight (a touch owned
/// the hit), `false` flies on through the owner and trigger volumes.
fn q1_missile_impact<L: ServerLogic>(
    ctx: &mut Q1WeaponFire<'_, '_, '_, L>,
    actor: &ActorId,
    missile: &Q1Missile,
    hit: &Q1LineHit,
) -> bool {
    if hit.hit_actor.as_ref().is_some_and(|other| other == &missile.owner) {
        return false;
    }
    if hit
        .hit_actor
        .as_ref()
        .is_some_and(|other| ctx.server.simulation_and_triggers().1.is_trigger(other))
    {
        return false;
    }
    // Sky swallows the straight flyers at the surface (their touches
    // check `pointcontents(self.origin) == CONTENT_SKY`); grenades
    // bounce off the sky face like any other solid.
    let flies_straight = matches!(
        missile.kind,
        Q1MissileKind::Spike
            | Q1MissileKind::SuperSpike
            | Q1MissileKind::WizSpike
            | Q1MissileKind::KnightSpike
            | Q1MissileKind::Rocket
            | Q1MissileKind::Laser
    );
    if flies_straight && q1_point_is_sky(ctx.scene, hit.endpos) {
        q1_remove_missile(ctx, actor);
        return true;
    }
    match missile.kind {
        Q1MissileKind::Spike => q1_spike_impact(ctx, actor, missile, hit, 9.0, Q1SpikeWall::Spike),
        Q1MissileKind::SuperSpike => q1_spike_impact(ctx, actor, missile, hit, 18.0, Q1SpikeWall::SuperSpike),
        Q1MissileKind::WizSpike => q1_spike_impact(ctx, actor, missile, hit, 9.0, Q1SpikeWall::WizSpike),
        Q1MissileKind::KnightSpike => q1_spike_impact(ctx, actor, missile, hit, 9.0, Q1SpikeWall::KnightSpike),
        Q1MissileKind::Rocket => {
            q1_rocket_impact(ctx, actor, missile, hit);
            true
        }
        Q1MissileKind::Grenade => {
            q1_grenade_impact(ctx, actor, missile, hit);
            true
        }
        Q1MissileKind::Laser => q1_laser_impact(ctx, actor, missile, hit),
        Q1MissileKind::OgreGrenade => {
            q1_ogre_grenade_impact(ctx, actor, missile, hit);
            true
        }
        Q1MissileKind::ZombieFlesh => {
            q1_zombie_flesh_impact(ctx, actor, missile, hit);
            true
        }
    }
}

/// Wall-impact temp ent for `spike_touch` (`weapons.qc:675`): stock
/// picks by missile classname (`wizspike`, `knightspike`, else the
/// plain spike).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q1SpikeWall {
    /// `TE_SPIKE`.
    Spike,
    /// `TE_SUPERSPIKE`.
    SuperSpike,
    /// `TE_WIZSPIKE`.
    WizSpike,
    /// `TE_KNIGHTSPIKE`.
    KnightSpike,
}

/// Stock `spike_touch` / `superspike_touch` (`weapons.qc:675`): blood
/// and 9 (nailgun, scrag) or 18 (super nailgun) damage to anything
/// damageable, else a wall spark; the spike never survives the hit.
fn q1_spike_impact<L: ServerLogic>(
    ctx: &mut Q1WeaponFire<'_, '_, '_, L>,
    actor: &ActorId,
    missile: &Q1Missile,
    hit: &Q1LineHit,
    damage: f64,
    wall: Q1SpikeWall,
) -> bool {
    let victim = hit
        .hit_actor
        .clone()
        .filter(|other| other != &missile.owner && q1_can_take_damage(ctx.server.simulation(), other));
    if let Some(victim) = victim {
        let velocity = ctx
            .server
            .simulation()
            .body_state(actor)
            .map(|body| body.velocity)
            .unwrap_or(vec3(0.0, 0.0, 0.0));
        let speed = length3(velocity);
        let dir = if speed > 0.0 {
            scale3(velocity, 1.0 / speed)
        } else {
            vec3(0.0, 0.0, 1.0)
        };
        let at = vec3(dir.x * 6.0, dir.y * 6.0, dir.z * 6.0);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        ctx.behaviors.temp_ents.push(Q1TempEnt::Blood {
            at,
            count: damage as u32,
        });
        let (simulation, movers, triggers) = ctx.server.simulation_movers_and_triggers_mut();
        q1_t_damage(
            ctx.behaviors,
            simulation,
            movers,
            triggers,
            &victim,
            Some(actor),
            Some(&missile.owner),
            damage,
        );
    } else {
        let ent = match wall {
            Q1SpikeWall::Spike => Q1TempEnt::Spike { at: hit.endpos },
            Q1SpikeWall::SuperSpike => Q1TempEnt::SuperSpike { at: hit.endpos },
            Q1SpikeWall::WizSpike => Q1TempEnt::WizSpike { at: hit.endpos },
            Q1SpikeWall::KnightSpike => Q1TempEnt::KnightSpike { at: hit.endpos },
        };
        ctx.behaviors.temp_ents.push(ent);
    }
    q1_remove_missile(ctx, actor);
    true
}

/// Stock `Laser_Touch` (`enforcer.qc:41`): the dispatcher already
/// refused the owner and the sky, so the bolt stops here with a
/// crack — blood and 15 damage for the living, a wall puff for the
/// rest — then removes.
fn q1_laser_impact<L: ServerLogic>(
    ctx: &mut Q1WeaponFire<'_, '_, '_, L>,
    actor: &ActorId,
    missile: &Q1Missile,
    hit: &Q1LineHit,
) -> bool {
    q1_monster_sound(
        ctx.behaviors,
        actor,
        Q1_CHAN_WEAPON,
        "enforcer/enfstop.wav",
        1.0,
        Q1_ATTN_STATIC,
    );
    let velocity = ctx
        .server
        .simulation()
        .body_state(actor)
        .map(|body| body.velocity)
        .unwrap_or(vec3(0.0, 0.0, 0.0));
    let speed = length3(velocity);
    let dir = if speed > 0.0 {
        scale3(velocity, 1.0 / speed)
    } else {
        vec3(0.0, 0.0, 1.0)
    };
    let org = vec3(
        hit.endpos.x - dir.x * 8.0,
        hit.endpos.y - dir.y * 8.0,
        hit.endpos.z - dir.z * 8.0,
    );
    let living = hit
        .hit_actor
        .as_ref()
        .is_some_and(|other| other != &missile.owner && q1_health_of(ctx.server.simulation(), other) != 0.0);
    if living {
        let victim = hit.hit_actor.clone().expect("gated victim");
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        ctx.behaviors.temp_ents.push(Q1TempEnt::Blood {
            at: org,
            count: Q1_LASER_DAMAGE as u32,
        });
        let (simulation, movers, triggers) = ctx.server.simulation_movers_and_triggers_mut();
        q1_t_damage(
            ctx.behaviors,
            simulation,
            movers,
            triggers,
            &victim,
            Some(actor),
            Some(&missile.owner),
            Q1_LASER_DAMAGE,
        );
    } else {
        ctx.behaviors.temp_ents.push(Q1TempEnt::Gunshot { at: org });
    }
    q1_remove_missile(ctx, actor);
    true
}

/// Stock `T_MissileTouch` (`combat.qc:296`): the thump, direct damage to
/// anything still standing (shamblers take half), then the 120-radius
/// blast with the direct victim excluded from the falloff.
fn q1_rocket_impact<L: ServerLogic>(
    ctx: &mut Q1WeaponFire<'_, '_, '_, L>,
    actor: &ActorId,
    missile: &Q1Missile,
    hit: &Q1LineHit,
) {
    q1_monster_sound(ctx.behaviors, actor, 0, "weapons/r_exp3.wav", 1.0, 1.0);
    if let Some(other) = hit.hit_actor.clone() {
        let health = q1_health_of(ctx.server.simulation(), &other);
        if health != 0.0 {
            let mut damage = f64::from(100.0 + q1_monster_random(ctx.behaviors) * 20.0);
            if q1_is_shambler(ctx.behaviors, &other) {
                damage *= 0.5;
            }
            let (simulation, movers, triggers) = ctx.server.simulation_movers_and_triggers_mut();
            q1_t_damage(
                ctx.behaviors,
                simulation,
                movers,
                triggers,
                &other,
                Some(actor),
                Some(&missile.owner),
                damage,
            );
        }
    }
    q1_grenade_explode(ctx, actor, hit.hit_actor.as_ref(), 120.0, 8.0);
}

/// Stock `GrenadeTouch` (`weapons.qc:714`) plus the toss clip
/// (`SV_Physics_Toss`): a direct hit on anything damageable detonates,
/// else the thump and a 1.5-backoff bounce, settling on flat ground
/// under 60 upward speed.
fn q1_grenade_impact<L: ServerLogic>(
    ctx: &mut Q1WeaponFire<'_, '_, '_, L>,
    actor: &ActorId,
    missile: &Q1Missile,
    hit: &Q1LineHit,
) {
    q1_monster_sound(ctx.behaviors, actor, 0, "weapons/bounce.wav", 1.0, 1.0);
    if hit
        .hit_actor
        .as_ref()
        .is_some_and(|other| other != &missile.owner && q1_can_take_damage(ctx.server.simulation(), other))
    {
        q1_grenade_explode(ctx, actor, None, 120.0, 8.0);
        return;
    }
    q1_grenade_bounce(ctx, actor, hit);
}

/// Stock `OgreGrenadeTouch` (`ogre.qc:71`): the dispatcher already
/// refused the owner, so aimed victims detonate the grenade and
/// everything else bounces audibly off it, stilling the spin on
/// dead stops.
fn q1_ogre_grenade_impact<L: ServerLogic>(
    ctx: &mut Q1WeaponFire<'_, '_, '_, L>,
    actor: &ActorId,
    missile: &Q1Missile,
    hit: &Q1LineHit,
) {
    if hit
        .hit_actor
        .as_ref()
        .is_some_and(|other| other != &missile.owner && q1_takedamage_aim(ctx.behaviors, other))
    {
        q1_grenade_explode(ctx, actor, None, Q1_GRENADE_DAMAGE, 0.0);
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
    q1_grenade_bounce(ctx, actor, hit);
    let stopped = ctx
        .server
        .simulation()
        .body_state(actor)
        .is_some_and(|body| body.velocity.x == 0.0 && body.velocity.y == 0.0 && body.velocity.z == 0.0);
    if stopped {
        if let Some(record) = ctx.behaviors.missiles.get_mut(actor) {
            record.avelocity = vec3(0.0, 0.0, 0.0);
        }
    }
}

/// Stock `ZombieGrenadeTouch` (`zombie.qc`): the dispatcher already
/// refused the owner, so the damageable takes 10 with a wet thump and
/// the chunk removes; anything else thuds a miss and stops dead with
/// `touch = SUB_Remove`, so the next touch removes it silently.
fn q1_zombie_flesh_impact<L: ServerLogic>(
    ctx: &mut Q1WeaponFire<'_, '_, '_, L>,
    actor: &ActorId,
    missile: &Q1Missile,
    hit: &Q1LineHit,
) {
    if hit
        .hit_actor
        .as_ref()
        .is_some_and(|other| other != &missile.owner && q1_can_take_damage(ctx.server.simulation(), other))
    {
        let victim = hit.hit_actor.clone().expect("gated victim");
        let (simulation, movers, triggers) = ctx.server.simulation_movers_and_triggers_mut();
        q1_t_damage(
            ctx.behaviors,
            simulation,
            movers,
            triggers,
            &victim,
            Some(actor),
            Some(&missile.owner),
            Q1_FLESH_DAMAGE,
        );
        q1_monster_sound(
            ctx.behaviors,
            actor,
            Q1_CHAN_WEAPON,
            "zombie/z_hit.wav",
            1.0,
            Q1_ATTN_NORM,
        );
        q1_remove_missile(ctx, actor);
        return;
    }
    if missile.spent {
        q1_remove_missile(ctx, actor);
        return;
    }
    q1_monster_sound(
        ctx.behaviors,
        actor,
        Q1_CHAN_WEAPON,
        "zombie/z_miss.wav",
        1.0,
        Q1_ATTN_NORM,
    );
    let _ignored = ctx
        .server
        .simulation_mut()
        .set_body_velocity(actor, vec3(0.0, 0.0, 0.0));
    if let Some(record) = ctx.behaviors.missiles.get_mut(actor) {
        record.avelocity = vec3(0.0, 0.0, 0.0);
        record.spent = true;
    }
}

/// Stock toss clip (`SV_Physics_Toss`, `sv_phys.c`): the 1.5-backoff
/// bounce, settling on flat ground under 60 upward speed. Both
/// grenade touches share it; only their detonate rules differ.
fn q1_grenade_bounce<L: ServerLogic>(ctx: &mut Q1WeaponFire<'_, '_, '_, L>, actor: &ActorId, hit: &Q1LineHit) {
    let velocity = ctx
        .server
        .simulation()
        .body_state(actor)
        .map(|body| body.velocity)
        .unwrap_or(vec3(0.0, 0.0, 0.0));
    let ops = NumericOps::select(Q1_DONOR_PROFILE).expect("Q1 donor numeric profile");
    let normal = hit.plane_normal;
    let mut clipped = clip_velocity_q1(velocity, normal, 1.5, &ops);
    let mut onground = false;
    if normal.z > 0.7 && clipped.z < 60.0 {
        clipped = vec3(0.0, 0.0, 0.0);
        onground = true;
    }
    let _ignored = ctx.server.simulation_mut().set_body_velocity(actor, clipped);
    if let Some(record) = ctx.behaviors.missiles.get_mut(actor) {
        record.onground = onground;
    }
}

/// Stock `BecomeExplosion` (`weapons.qc:698`) and `OgreGrenadeExplode`
/// (`ogre.qc:53`): the radius blast with `ignore` excluded, the flash
/// pulled `flash_backoff` units off the surface along the flight
/// line (8 for player ordnance, 0 for ogre grenades), and the missile
/// retired. Monster-vs-monster detonations outside the missile pass
/// call in through the same shape.
pub(crate) fn q1_grenade_explode<L: ServerLogic>(
    ctx: &mut Q1WeaponFire<'_, '_, '_, L>,
    actor: &ActorId,
    ignore: Option<&ActorId>,
    damage: f64,
    flash_backoff: f32,
) {
    let attacker = ctx
        .behaviors
        .missiles
        .get(actor)
        .map(|missile| missile.owner.clone())
        .unwrap_or_else(|| ctx.player.clone());
    q1_monster_sound(ctx.behaviors, actor, 0, "weapons/r_exp3.wav", 1.0, 1.0);
    q1_t_radius_damage(ctx, actor, &attacker, damage, ignore);
    let (origin, velocity) = ctx
        .server
        .simulation()
        .body_state(actor)
        .map(|body| (body.origin, body.velocity))
        .unwrap_or((vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 1.0)));
    let speed = length3(velocity);
    let dir = if speed > 0.0 {
        scale3(velocity, 1.0 / speed)
    } else {
        vec3(0.0, 0.0, 1.0)
    };
    let at = vec3(
        origin.x - dir.x * flash_backoff,
        origin.y - dir.y * flash_backoff,
        origin.z - dir.z * flash_backoff,
    );
    ctx.behaviors.temp_ents.push(Q1TempEnt::Explosion { at });
    q1_remove_missile(ctx, actor);
}

/// Stock `T_RadiusDamage` (`combat.qc:224`): linear `damage - 0.5 * dist`
/// falloff over `damage + 40` units, the attacker at half strength,
/// `CanDamage` gating every victim, shamblers halved again.
fn q1_t_radius_damage<L: ServerLogic>(
    ctx: &mut Q1WeaponFire<'_, '_, '_, L>,
    inflictor: &ActorId,
    attacker: &ActorId,
    damage: f64,
    ignore: Option<&ActorId>,
) {
    let Some(origin) = ctx.server.simulation().body_state(inflictor).map(|body| body.origin) else {
        return;
    };
    let victims: Vec<ActorId> = {
        let simulation = ctx.server.simulation();
        simulation
            .body_actors()
            .into_iter()
            .filter(|actor| Some(actor) != ignore && q1_can_take_damage(simulation, actor))
            .collect()
    };
    for victim in &victims {
        let Some(center) = ctx.server.simulation().body_state(victim).map(|body| {
            let bounds = translated_body_bounds(&body);
            vec3(
                (bounds.min.x + bounds.max.x) / 2.0,
                (bounds.min.y + bounds.max.y) / 2.0,
                (bounds.min.z + bounds.max.z) / 2.0,
            )
        }) else {
            continue;
        };
        let offset = vec3(origin.x - center.x, origin.y - center.y, origin.z - center.z);
        let mut points = damage - 0.5 * f64::from(length3(offset));
        if points <= 0.0 {
            continue;
        }
        if victim == attacker {
            points *= 0.5;
        }
        let damageable = {
            let mut monster_ctx = Q1MonsterCtx {
                server: &mut *ctx.server,
                behaviors: &mut *ctx.behaviors,
                scene: ctx.scene,
                now: ctx.now,
                dt: 0.0,
            };
            q1_can_damage(&mut monster_ctx, victim, inflictor)
        };
        if !damageable {
            continue;
        }
        if q1_is_shambler(ctx.behaviors, victim) {
            points *= 0.5;
        }
        let (simulation, movers, triggers) = ctx.server.simulation_movers_and_triggers_mut();
        q1_t_damage(
            ctx.behaviors,
            simulation,
            movers,
            triggers,
            victim,
            Some(inflictor),
            Some(attacker),
            points,
        );
    }
}

/// Shambler check for the two half-damage paths (stock compares
/// `classname == "monster_shambler"`): by classname string so it keeps
/// working when the monster lane lands the kind.
fn q1_is_shambler(behaviors: &Q1NativeBehaviors, actor: &ActorId) -> bool {
    behaviors
        .monsters
        .get(actor)
        .is_some_and(|monster| monster.kind.classname() == "monster_shambler")
}

/// Retire a missile: stock `SUB_Remove` frees the edict; here the body
/// releases and the record drops.
fn q1_remove_missile<L: ServerLogic>(ctx: &mut Q1WeaponFire<'_, '_, '_, L>, actor: &ActorId) {
    let (simulation, movers, triggers) = ctx.server.simulation_movers_and_triggers_mut();
    q1_remove(ctx.behaviors, simulation, movers, triggers, actor);
}

/// Stock QuakeC `rint`: round half up (`floor(x + 0.5)`; stock
/// only ever rounds non-negative rolls).
fn q1_rint(value: f32) -> i32 {
    (value + 0.5).floor() as i32
}

/// Resolve an attacker for pain and obituary branches: the player,
/// a monster kind, a door, or empty for the world and the unbuilt
/// (missiles never attack directly — stock credits their owner).
fn q1_attacker_classname<'b>(behaviors: &'b Q1NativeBehaviors, actor: &ActorId) -> &'b str {
    if Some(actor) == behaviors.player.as_ref() {
        return "player";
    }
    if let Some(monster) = behaviors.monsters.get(actor) {
        return monster.kind.classname();
    }
    if behaviors.doors.contains_key(actor) {
        return "door";
    }
    ""
}

/// Stock `player_pain` (`player.qc:337`): the wound anim and sound,
/// gated while a weapon anim owns the frames or invisibility hides
/// the eyes. Called from `T_Damage`'s pain tail for live players.
pub(crate) fn q1_player_pain(
    behaviors: &mut Q1NativeBehaviors,
    simulation: &qa_world::session::Simulation,
    targ: &ActorId,
) {
    if Some(targ) != behaviors.player.as_ref() {
        return;
    }
    if behaviors.player_state.weaponframe != 0 {
        return;
    }
    if behaviors.player_items & Q1_IT_INVISIBILITY != 0 {
        return;
    }
    let now = simulation.frame().time.as_seconds_f64();
    q1_pain_sound(behaviors, simulation, targ);
    behaviors.player_state.weaponframe = 0;
    behaviors.player_state.attack = Q1PlayerAttack::Pain { until: now + 0.6 };
}

/// Stock `PainSound` (`player.qc:269`): teleport sizzle, drowning and
/// burn gurgles, the 0.5 s gate, the axe-hit cry, then pain1-6.
fn q1_pain_sound(behaviors: &mut Q1NativeBehaviors, simulation: &qa_world::session::Simulation, targ: &ActorId) {
    let now = simulation.frame().time.as_seconds_f64();
    if q1_health_of(simulation, targ) < 0.0 {
        return;
    }
    let attacker_class = behaviors
        .damage_attacker
        .as_ref()
        .map(|attacker| q1_attacker_classname(behaviors, attacker))
        .unwrap_or("");
    if attacker_class == "teledeath" {
        q1_monster_sound(behaviors, targ, 2, "player/teledth1.wav", 1.0, 0.0);
        return;
    }
    let water_level = behaviors.player_state.water_level;
    let water_type = behaviors.player_state.water_type;
    if water_type == Q1_CONTENTS_WATER && water_level == 3 {
        q1_death_bubbles(behaviors, simulation, targ, 1);
        let drown = q1_monster_random(behaviors) > 0.5;
        q1_monster_sound(
            behaviors,
            targ,
            2,
            if drown {
                "player/drown1.wav"
            } else {
                "player/drown2.wav"
            },
            1.0,
            1.0,
        );
        return;
    }
    if water_type == Q1_CONTENTS_SLIME || water_type == Q1_CONTENTS_LAVA {
        let burn = q1_monster_random(behaviors) > 0.5;
        q1_monster_sound(
            behaviors,
            targ,
            2,
            if burn { "player/lburn1.wav" } else { "player/lburn2.wav" },
            1.0,
            1.0,
        );
        return;
    }
    if behaviors.player_state.pain_finished > now {
        behaviors.player_state.axhitme = false;
        return;
    }
    behaviors.player_state.pain_finished = now + 0.5;
    if behaviors.player_state.axhitme {
        behaviors.player_state.axhitme = false;
        q1_monster_sound(behaviors, targ, 2, "player/axhit1.wav", 1.0, 1.0);
        return;
    }
    let rs = q1_rint(q1_monster_random(behaviors) * 5.0 + 1.0);
    let sample = match rs {
        1 => "player/pain1.wav",
        2 => "player/pain2.wav",
        3 => "player/pain3.wav",
        4 => "player/pain4.wav",
        5 => "player/pain5.wav",
        _ => "player/pain6.wav",
    };
    q1_monster_sound(behaviors, targ, 2, sample, 1.0, 1.0);
}

/// Stock gib velocity (`VelocityForDamage`, `player.qc:435`): wide
/// random toss, scaled up as the killing wound deepens.
fn q1_velocity_for_damage(behaviors: &mut Q1NativeBehaviors, dm: f64) -> Vec3 {
    let v = vec3(
        100.0 * q1_monster_crandom(behaviors),
        100.0 * q1_monster_crandom(behaviors),
        200.0 + 100.0 * q1_monster_random(behaviors),
    );
    let scale = if dm > -50.0 {
        0.7
    } else if dm > -200.0 {
        2.0
    } else {
        10.0
    };
    scale3(v, scale)
}

/// Stock `DeathBubbles` (`player.qc:381`): the bubble-spawner entity
/// has no runtime here, so the visual count queues directly.
fn q1_death_bubbles(
    behaviors: &mut Q1NativeBehaviors,
    simulation: &qa_world::session::Simulation,
    targ: &ActorId,
    count: u32,
) {
    let Some(body) = simulation.body_state(targ) else {
        return;
    };
    behaviors.temp_ents.push(Q1TempEnt::Bubbles {
        at: vec3(body.origin.x, body.origin.y, body.origin.z + 24.0),
        count,
    });
}

/// Stock `DeathSound` (`player.qc:399`): underwater burbling, else one
/// of death1-5 on the voice channel, audible everywhere.
fn q1_death_sound(behaviors: &mut Q1NativeBehaviors, simulation: &qa_world::session::Simulation, targ: &ActorId) {
    if behaviors.player_state.water_level == 3 {
        q1_death_bubbles(behaviors, simulation, targ, 20);
        q1_monster_sound(behaviors, targ, 2, "player/h2odeath.wav", 1.0, 0.0);
        return;
    }
    let rs = q1_rint(q1_monster_random(behaviors) * 4.0 + 1.0);
    let sample = match rs {
        1 => "player/death1.wav",
        2 => "player/death2.wav",
        3 => "player/death3.wav",
        4 => "player/death4.wav",
        _ => "player/death5.wav",
    };
    q1_monster_sound(behaviors, targ, 2, sample, 1.0, 0.0);
}

/// Stock `PlayerDie` (`player.qc:524`): strip the powerup state, drop
/// the view model, pop the corpse, and gib past -40 health or run a
/// death anim. Called from `Killed` after the obituary with damage
/// already off. Powerup timers live with the powerup slice (nothing
/// to clear yet); the coop/DM backpack waits for the items slice,
/// which owns item entities.
pub(crate) fn q1_player_die(
    behaviors: &mut Q1NativeBehaviors,
    simulation: &mut qa_world::session::Simulation,
    targ: &ActorId,
) {
    behaviors.player_items &= !Q1_IT_INVISIBILITY;
    behaviors.player_state.weaponmodel.clear();
    behaviors.player_state.deadflag = Q1_DEAD_DYING;
    behaviors.solids.remove(targ);
    behaviors.player_state.attack = Q1PlayerAttack::None;
    if let Some(body) = simulation.body_state(targ) {
        let mut vz = body.velocity.z;
        if vz < 10.0 {
            vz += q1_monster_random(behaviors) * 300.0;
        }
        let _ignored = simulation.set_body_velocity(targ, vec3(body.velocity.x, body.velocity.y, vz));
    }
    let health = q1_health_of(simulation, targ);
    if health < -40.0 {
        q1_gib_player(behaviors, simulation, targ, health);
        return;
    }
    q1_death_sound(behaviors, simulation, targ);
    if let Some(body) = simulation.body_state(targ) {
        let _ignored = simulation.set_body_angles(targ, vec3(0.0, body.angles.y, 0.0));
    }
    let now = simulation.frame().time.as_seconds_f64();
    if behaviors.player_state.weapon == Q1_IT_AXE {
        behaviors.player_state.death_anim = Q1_DEATH_AXE;
    } else {
        // The `temp1` cvar does not exist here; death always rolls.
        #[allow(clippy::cast_possible_truncation)]
        let roll = 1 + (q1_monster_random(behaviors) * 6.0).floor() as i32;
        behaviors.player_state.death_anim = match roll {
            1 => Q1_DEATH_A,
            2 => Q1_DEATH_B,
            3 => Q1_DEATH_C,
            4 => Q1_DEATH_D,
            _ => Q1_DEATH_E,
        };
    }
    behaviors.player_state.death_frames = 1;
    behaviors.player_state.dead_until = now + 0.1;
}

/// Stock `GibPlayer` (`player.qc:497`): the player becomes the
/// bouncing head, three flesh chunks fly, and the gib cry sounds.
/// The head spin has no angular channel on bodies (noted); the
/// teledeath sizzles stay live for the teleporter slice.
fn q1_gib_player(
    behaviors: &mut Q1NativeBehaviors,
    simulation: &mut qa_world::session::Simulation,
    targ: &ActorId,
    health: f64,
) {
    behaviors.player_state.gibbed_head = true;
    let point = Bounds {
        min: vec3(-16.0, -16.0, 0.0),
        max: vec3(16.0, 16.0, 56.0),
    };
    let _ignored = simulation.set_body_bounds(targ, point);
    let Some(body) = simulation.body_state(targ) else {
        return;
    };
    let at = vec3(body.origin.x, body.origin.y, body.origin.z - 24.0);
    let _ignored = simulation.set_body_origin(targ, at);
    let _ignored = simulation.set_body_velocity(targ, q1_velocity_for_damage(behaviors, health));
    behaviors.player_state.deadflag = Q1_DEAD_DEAD;
    let now = simulation.frame().time.as_seconds_f64();
    for model in ["progs/gib1.mdl", "progs/gib2.mdl", "progs/gib3.mdl"] {
        let velocity = q1_velocity_for_damage(behaviors, health);
        let avelocity = vec3(
            q1_monster_random(behaviors) * 600.0,
            q1_monster_random(behaviors) * 600.0,
            q1_monster_random(behaviors) * 600.0,
        );
        let remove_at = now + 10.0 + f64::from(q1_monster_random(behaviors)) * 10.0;
        behaviors.pending_gibs.push(Q1PendingGib {
            model: model.to_string(),
            at,
            velocity,
            avelocity,
            remove_at,
        });
    }
    let attacker_class = behaviors
        .damage_attacker
        .as_ref()
        .map(|attacker| q1_attacker_classname(behaviors, attacker))
        .unwrap_or("");
    if attacker_class == "teledeath" || attacker_class == "teledeath2" {
        q1_monster_sound(behaviors, targ, 2, "player/teledth1.wav", 1.0, 0.0);
        return;
    }
    if q1_monster_random(behaviors) < 0.5 {
        q1_monster_sound(behaviors, targ, 2, "player/gib.wav", 1.0, 0.0);
    } else {
        q1_monster_sound(behaviors, targ, 2, "player/udeath.wav", 1.0, 0.0);
    }
}

/// Stock `ClientObituary` (`client.qc:1197`): the death line plus
/// frag credit, composed into one `sprint` (stock `bprint`s the same
/// pieces; broadcast and targeted prints coincide in singleplayer).
/// Called from `Killed` before `th_die`, so the victim's weapon,
/// water, and health still read pre-death. Telefrag lines wait for
/// the teleporter slice (no owner tracking yet); killer weapon and
/// water read the shared player state until deathmatch clients land.
pub(crate) fn q1_client_obituary(
    behaviors: &mut Q1NativeBehaviors,
    simulation: &qa_world::session::Simulation,
    targ: &ActorId,
    attacker: Option<&ActorId>,
) {
    let rnum = q1_monster_random(behaviors);
    let name = Q1_PLAYER_NETNAME.to_string();
    let attacker_class = attacker
        .map(|actor| q1_attacker_classname(behaviors, actor).to_string())
        .unwrap_or_default();
    let line;
    if attacker_class == "player" {
        if attacker == Some(targ) {
            behaviors.player_state.frags -= 1.0;
            if behaviors.player_state.weapon == Q1_IT_LIGHTNING && behaviors.player_state.water_level > 1 {
                line = format!("{name} discharges into the water.\n");
            } else if behaviors.player_state.weapon == Q1_IT_GRENADE_LAUNCHER {
                line = format!("{name} tries to put the pin back in\n");
            } else {
                line = format!("{name} becomes bored with life\n");
            }
        } else if behaviors.teamplay == 2
            && behaviors.player_state.team > 0
            && Some(behaviors.player_state.team) == attacker.map(|_| behaviors.player_state.team)
        {
            // Teams collapse onto the one player state until
            // deathmatch clients land; the rule itself is live.
            if rnum < 0.25 {
                line = format!("{name} mows down a teammate\n");
            } else if rnum < 0.50 {
                line = format!("{name} checks his glasses\n");
            } else if rnum < 0.75 {
                line = format!("{name} gets a frag for the other team\n");
            } else {
                line = format!("{name} loses another friend\n");
            }
            behaviors.player_state.frags -= 1.0;
        } else {
            behaviors.player_state.frags += 1.0;
            let health = q1_health_of(simulation, targ);
            let weapon = behaviors.player_state.weapon;
            let (death, death2) = if weapon == Q1_IT_AXE {
                (" was ax-murdered by ", "\n")
            } else if weapon == Q1_IT_SHOTGUN {
                (" chewed on ", "'s boomstick\n")
            } else if weapon == Q1_IT_SUPER_SHOTGUN {
                (" ate 2 loads of ", "'s buckshot\n")
            } else if weapon == Q1_IT_NAILGUN {
                (" was nailed by ", "\n")
            } else if weapon == Q1_IT_SUPER_NAILGUN {
                (" was punctured by ", "\n")
            } else if weapon == Q1_IT_GRENADE_LAUNCHER {
                if health < -40.0 {
                    (" was gibbed by ", "'s grenade\n")
                } else {
                    (" eats ", "'s pineapple\n")
                }
            } else if weapon == Q1_IT_ROCKET_LAUNCHER {
                if health < -40.0 {
                    (" was gibbed by ", "'s rocket\n")
                } else {
                    (" rides ", "'s rocket\n")
                }
            } else if weapon == Q1_IT_LIGHTNING {
                if behaviors.player_state.water_level > 1 {
                    (" accepts ", "'s discharge\n")
                } else {
                    (" accepts ", "'s shaft\n")
                }
            } else {
                (" was killed by ", "\n")
            };
            line = format!("{name}{death}{name}{death2}");
        }
    } else {
        behaviors.player_state.frags -= 1.0;
        if behaviors.monsters.contains_key(attacker.unwrap_or(targ)) {
            let class = q1_attacker_classname(behaviors, attacker.unwrap_or(targ));
            let suffix = match class {
                "monster_army" => " was shot by a Grunt\n",
                "monster_demon1" => " was eviscerated by a Fiend\n",
                "monster_dog" => " was mauled by a Rottweiler\n",
                "monster_dragon" => " was fried by a Dragon\n",
                "monster_enforcer" => " was blasted by an Enforcer\n",
                "monster_fish" => " was fed to the Rotfish\n",
                "monster_hell_knight" => " was slain by a Death Knight\n",
                "monster_knight" => " was slashed by a Knight\n",
                "monster_ogre" => " was destroyed by an Ogre\n",
                "monster_oldone" => " became one with Shub-Niggurath\n",
                "monster_shalrath" => " was exploded by a Vore\n",
                "monster_shambler" => " was smashed by a Shambler\n",
                "monster_tarbaby" => " was slimed by a Spawn\n",
                "monster_vomit" => " was vomited on by a Vomitus\n",
                "monster_wizard" => " was scragged by a Scrag\n",
                "monster_zombie" => " joins the Zombies\n",
                // Stock prints the bare name for unmatched kinds.
                _ => "",
            };
            line = format!("{name}{suffix}");
        } else if attacker_class == "explo_box" {
            line = format!("{name} blew up\n");
        } else if attacker.is_some() && attacker_class == "door" {
            line = format!("{name} was squished\n");
        } else if attacker_class == "trap_shooter" || attacker_class == "trap_spikeshooter" {
            line = format!("{name} was spiked\n");
        } else if attacker_class == "fireball" {
            line = format!("{name} ate a lavaball\n");
        } else if attacker_class == "trigger_changelevel" {
            line = format!("{name} tried to leave\n");
        } else if behaviors.player_state.water_type == Q1_CONTENTS_WATER {
            if rnum < 0.5 {
                line = format!("{name} sleeps with the fishes\n");
            } else {
                line = format!("{name} sucks it down\n");
            }
        } else if behaviors.player_state.water_type == Q1_CONTENTS_SLIME {
            if rnum < 0.5 {
                line = format!("{name} gulped a load of slime\n");
            } else {
                line = format!("{name} can't exist on slime alone\n");
            }
        } else if behaviors.player_state.water_type == Q1_CONTENTS_LAVA {
            let health = q1_health_of(simulation, targ);
            if health < -15.0 {
                line = format!("{name} burst into flames\n");
            } else if rnum < 0.5 {
                line = format!("{name} turned into hot slag\n");
            } else {
                line = format!("{name} visits the Volcano God\n");
            }
        } else if behaviors.player_state.deathtype == "falling" {
            behaviors.player_state.deathtype.clear();
            line = format!("{name} fell to his death\n");
        } else {
            line = format!("{name} died\n");
        }
    }
    q1_sprint(behaviors, targ, &line);
}

/// Death-anim frame counts by anim id (`player.qc:602-659`).
fn q1_death_anim_frames(anim: u8) -> u8 {
    match anim {
        Q1_DEATH_A => 11,
        Q1_DEATH_C => 15,
        _ => 9,
    }
}

/// Stock `PlayerDead` (`player.qc:428`): the last death frame retires
/// the think and opens the respawn wait.
fn q1_player_dead(behaviors: &mut Q1NativeBehaviors) {
    behaviors.player_state.deadflag = Q1_DEAD_DEAD;
}

/// Stock `PlayerDeathThink` (`client.qc:692`): corpse friction, the
/// death-anim advance, then the release-and-press respawn wait.
/// Owns the weapon pass while `deadflag` is set (`client.qc:921`).
fn q1_player_death_think(
    behaviors: &mut Q1NativeBehaviors,
    simulation: &mut qa_world::session::Simulation,
    player: &ActorId,
    now: f64,
    buttons: i32,
) {
    if let Some(body) = simulation.body_state(player) {
        if body.ground.is_some() {
            let dt = (simulation.frame().elapsed.as_seconds_f64() as f32).max(0.0);
            let speed = length3(body.velocity);
            // Stock sheds 20 units per 0.1 s think; scale to the step.
            let updated = speed - 200.0 * dt;
            let _ignored = simulation.set_body_velocity(
                player,
                if updated <= 0.0 {
                    vec3(0.0, 0.0, 0.0)
                } else {
                    scale3(body.velocity, updated / speed)
                },
            );
        }
    }
    match behaviors.player_state.deadflag {
        Q1_DEAD_DYING => {
            if now >= behaviors.player_state.dead_until {
                let frames = q1_death_anim_frames(behaviors.player_state.death_anim);
                if behaviors.player_state.death_frames >= frames {
                    q1_player_dead(behaviors);
                } else {
                    behaviors.player_state.death_frames += 1;
                    behaviors.player_state.dead_until = now + 0.1;
                }
            }
        }
        Q1_DEAD_DEAD => {
            if buttons != 0 {
                return;
            }
            behaviors.player_state.deadflag = Q1_DEAD_RESPAWNABLE;
        }
        Q1_DEAD_RESPAWNABLE => {
            if buttons == 0 {
                return;
            }
            // Stock clears the latched buttons before respawning; ours
            // arrive fresh per pass, so there is nothing to clear.
            q1_respawn(behaviors, simulation, player, now);
        }
        _ => {}
    }
}

/// Stock `respawn` (`client.qc:353`): coop restores the level-start
/// parms, deathmatch deals fresh ones, singleplayer restarts the
/// level. Both multiplayer modes keep a body-queue copy first.
fn q1_respawn(
    behaviors: &mut Q1NativeBehaviors,
    simulation: &mut qa_world::session::Simulation,
    player: &ActorId,
    now: f64,
) {
    if behaviors.coop {
        q1_copy_to_bodyque(behaviors, simulation, player);
        q1_put_client_in_server(behaviors, simulation, player, now);
    } else if behaviors.deathmatch {
        q1_copy_to_bodyque(behaviors, simulation, player);
        behaviors.player_state.parms = Q1SpawnParms::default();
        q1_put_client_in_server(behaviors, simulation, player, now);
    } else {
        behaviors.player_state.restart_requested = true;
    }
}

/// Stock `CopyToBodyQue` (`world.qc:378`): the four-slot corpse ring,
/// oldest dropped. Gibbed players leave their head behind.
fn q1_copy_to_bodyque(behaviors: &mut Q1NativeBehaviors, simulation: &qa_world::session::Simulation, player: &ActorId) {
    let Some(body) = simulation.body_state(player) else {
        return;
    };
    if behaviors.player_state.body_queue.len() >= 4 {
        behaviors.player_state.body_queue.remove(0);
    }
    behaviors.player_state.body_queue.push(Q1Corpse {
        origin: body.origin,
        angles: body.angles,
        frame: i32::from(behaviors.player_state.death_frames),
        model: if behaviors.player_state.gibbed_head {
            "progs/h_player.mdl".to_string()
        } else {
            "progs/player.mdl".to_string()
        },
    });
}

/// Stock `SelectSpawnPoint` (`client.qc:407`): test starts win,
/// coop/DM cycle their spots from the cursor (DM skips occupied
/// ones), singleplayer prefers `start2` past episode breaks. A map
/// with no usable spot keeps the corpse position (stock `error()`s;
/// gamecode never panics).
fn q1_select_spawn_point(
    behaviors: &mut Q1NativeBehaviors,
    simulation: &qa_world::session::Simulation,
) -> Option<super::native_q1_spawns::Q1SpawnSpot> {
    if let Some(spot) = behaviors
        .spawn_spots
        .iter()
        .find(|spot| spot.classname == "testplayerstart")
    {
        return Some(spot.clone());
    }
    if behaviors.coop {
        if let Some(spot) = q1_next_spawn_spot(behaviors, "info_player_coop") {
            return Some(spot);
        }
        if let Some(spot) = q1_next_spawn_spot(behaviors, "info_player_start") {
            return Some(spot);
        }
    } else if behaviors.deathmatch {
        if let Some(spot) = q1_next_open_deathmatch_spot(behaviors, simulation) {
            return Some(spot);
        }
    }
    if behaviors.serverflags != 0 {
        if let Some(spot) = behaviors
            .spawn_spots
            .iter()
            .find(|spot| spot.classname == "info_player_start2")
        {
            return Some(spot.clone());
        }
    }
    behaviors
        .spawn_spots
        .iter()
        .find(|spot| spot.classname == "info_player_start")
        .cloned()
}

/// Stock `find` from the cursor: the next spot of a class in spawn
/// order, wrapping once, with the cursor advanced past it.
fn q1_next_spawn_spot(
    behaviors: &mut Q1NativeBehaviors,
    classname: &str,
) -> Option<super::native_q1_spawns::Q1SpawnSpot> {
    let count = behaviors.spawn_spots.len();
    if count == 0 {
        return None;
    }
    let start = behaviors.lastspawn_spot.map_or(0, |index| (index + 1) % count);
    for step in 0..count {
        let index = (start + step) % count;
        if behaviors.spawn_spots[index].classname == classname {
            behaviors.lastspawn_spot = Some(index);
            return Some(behaviors.spawn_spots[index].clone());
        }
    }
    None
}

/// Stock deathmatch cycling: the next `info_player_deathmatch` spot
/// with no player within 32 units, else the cycle-start spot even
/// occupied (stock returns `lastspawn` after a full loop).
fn q1_next_open_deathmatch_spot(
    behaviors: &mut Q1NativeBehaviors,
    simulation: &qa_world::session::Simulation,
) -> Option<super::native_q1_spawns::Q1SpawnSpot> {
    let count = behaviors.spawn_spots.len();
    if count == 0 {
        return None;
    }
    let start = behaviors.lastspawn_spot.map_or(0, |index| (index + 1) % count);
    let player_origin = behaviors
        .player
        .as_ref()
        .and_then(|player| simulation.body_state(player))
        .map(|body| body.origin);
    let mut fallback = None;
    for step in 0..count {
        let index = (start + step) % count;
        if behaviors.spawn_spots[index].classname != "info_player_deathmatch" {
            continue;
        }
        if fallback.is_none() {
            fallback = Some((index, behaviors.spawn_spots[index].clone()));
        }
        let clear = player_origin.is_none_or(|origin| {
            let offset = vec3(
                origin.x - behaviors.spawn_spots[index].origin.x,
                origin.y - behaviors.spawn_spots[index].origin.y,
                origin.z - behaviors.spawn_spots[index].origin.z,
            );
            length3(offset) >= 32.0
        });
        if clear {
            behaviors.lastspawn_spot = Some(index);
            return Some(behaviors.spawn_spots[index].clone());
        }
    }
    if let Some((index, spot)) = fallback {
        behaviors.lastspawn_spot = Some(index);
        return Some(spot);
    }
    None
}

/// Stock `PutClientInServer` (`client.qc:479`): full health, solid and
/// damageable again, decoded parms in hand, placed on the spawn
/// spot with the view snapped (`fixangle` rides `respawn_angles`;
/// `step_weapons` applies it). Air, powerup, and effect timers live
/// with their slices; the respawn fog cracker plays at once (stock
/// delays it 0.2 s on a temp entity with no channel here).
#[allow(clippy::too_many_lines)]
fn q1_put_client_in_server(
    behaviors: &mut Q1NativeBehaviors,
    simulation: &mut qa_world::session::Simulation,
    player: &ActorId,
    now: f64,
) {
    if behaviors.serverflags != 0 && behaviors.mapname == "start" {
        behaviors.player_state.parms = Q1SpawnParms::default();
    }
    let parms = behaviors.player_state.parms.clone();
    behaviors.player_items = parms.items;
    behaviors.player_ammo.shells = parms.shells;
    behaviors.player_ammo.nails = parms.nails;
    behaviors.player_ammo.rockets = parms.rockets;
    behaviors.player_ammo.cells = parms.cells;
    behaviors.player_state.weapon = parms.weapon;
    behaviors.player_max_health = 100.0;
    if let Some(combat) = simulation.combat_state(player).cloned() {
        let armor = if parms.armorvalue > 0.0 {
            let item = if parms.armortype >= 0.8 {
                "q1:item_armorInv"
            } else if parms.armortype >= 0.6 {
                "q1:item_armor2"
            } else {
                "q1:item_armor1"
            };
            RegularArmor::Q1 {
                points: parms.armorvalue,
                absorption: parms.armortype,
                item: item.to_string(),
            }
        } else {
            RegularArmor::None
        };
        let _ignored = simulation.set_combat(
            player,
            CombatState {
                health: parms.health,
                can_take_damage: true,
                armor: ArmorState {
                    regular: armor,
                    ..combat.armor
                },
                ..combat
            },
        );
    }
    q1_w_set_current_ammo(behaviors);
    behaviors.player_state.attack_finished = now;
    behaviors.player_state.deadflag = Q1_DEAD_NO;
    behaviors.player_state.death_frames = 0;
    behaviors.player_state.gibbed_head = false;
    behaviors.player_state.attack = Q1PlayerAttack::None;
    behaviors.player_state.show_hostile = 0.0;
    behaviors.solids.insert(player);
    if let Some(spot) = q1_select_spawn_point(behaviors, simulation) {
        let _ignored = simulation.set_body_origin(player, vec3(spot.origin.x, spot.origin.y, spot.origin.z + 1.0));
        let _ignored = simulation.set_body_angles(player, spot.angles);
        let _ignored = simulation.set_body_velocity(player, vec3(0.0, 0.0, 0.0));
        behaviors.player_angles = spot.angles;
        behaviors.player_state.respawn_angles = Some(spot.angles);
        if behaviors.coop || behaviors.deathmatch {
            let vectors = angle_vectors(spot.angles);
            let fog = vec3(
                spot.origin.x + vectors.forward.x * 20.0,
                spot.origin.y + vectors.forward.y * 20.0,
                spot.origin.z + vectors.forward.z * 20.0 + 1.0,
            );
            behaviors.temp_ents.push(Q1TempEnt::Teleport { at: fog });
            let roll = q1_monster_random(behaviors) * 5.0;
            let sample = if roll < 1.0 {
                "misc/r_tele1.wav"
            } else if roll < 2.0 {
                "misc/r_tele2.wav"
            } else if roll < 3.0 {
                "misc/r_tele3.wav"
            } else if roll < 4.0 {
                "misc/r_tele4.wav"
            } else {
                "misc/r_tele5.wav"
            };
            q1_monster_sound(behaviors, player, 2, sample, 1.0, 1.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use qa_core::identity::ProviderId;
    use qa_world::body::BodyState;
    use qa_world::combat::CombatState;

    use super::*;
    use crate::options::ApplicationOptions;
    use crate::startup::{open_server, StartupConfig};

    fn test_server() -> Server<qa_guest::server::GuestServerLogic> {
        let config = StartupConfig::from_options(&ApplicationOptions::default()).unwrap();
        open_server(&config).unwrap()
    }

    fn spawn_player(server: &mut Server<qa_guest::server::GuestServerLogic>) -> ActorId {
        let player = server
            .simulation_mut()
            .spawn(
                ProviderId::new("q1", "test"),
                "q1:test_player",
                Some(BodyState {
                    origin: vec3(0.0, 0.0, 24.0),
                    angles: vec3(0.0, 0.0, 0.0),
                    velocity: vec3(0.0, 0.0, 0.0),
                    bounds: qa_core::math::Bounds {
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
        player.id().clone()
    }

    fn loaded_behaviors() -> Q1NativeBehaviors {
        let mut behaviors = Q1NativeBehaviors::new();
        q1_grant_spawn_loadout(&mut behaviors);
        behaviors
    }

    #[test]
    fn spawn_loadout_matches_set_new_parms() {
        let behaviors = loaded_behaviors();
        assert_ne!(behaviors.player_items & Q1_IT_AXE, 0);
        assert_ne!(behaviors.player_items & Q1_IT_SHOTGUN, 0);
        assert_eq!(behaviors.player_items & Q1_IT_AMMO_BITS, Q1_IT_SHELLS);
        assert_eq!(behaviors.player_ammo.shells, 25.0);
        assert_eq!(behaviors.player_ammo.nails, 0.0);
        assert_eq!(behaviors.player_state.weapon, Q1_IT_SHOTGUN);
        assert_eq!(behaviors.player_state.currentammo, 25.0);
        assert_eq!(behaviors.player_state.weaponmodel, "progs/v_shot.mdl");
        assert_eq!(behaviors.player_state.weaponframe, 0);
        assert_eq!(behaviors.player_state.parms.weapon, Q1_IT_SHOTGUN);
        assert_eq!(behaviors.player_state.parms.shells, 25.0);
    }

    #[test]
    fn best_weapon_orders_lightning_to_axe() {
        let mut behaviors = loaded_behaviors();
        behaviors.player_items |= Q1_IT_SUPER_SHOTGUN
            | Q1_IT_NAILGUN
            | Q1_IT_SUPER_NAILGUN
            | Q1_IT_GRENADE_LAUNCHER
            | Q1_IT_ROCKET_LAUNCHER
            | Q1_IT_LIGHTNING;
        behaviors.player_ammo.nails = 10.0;
        behaviors.player_ammo.shells = 10.0;
        behaviors.player_ammo.rockets = 10.0;
        behaviors.player_ammo.cells = 0.0;
        assert_eq!(q1_w_best_weapon(&behaviors), Q1_IT_SUPER_NAILGUN);
        behaviors.player_ammo.nails = 1.0;
        assert_eq!(q1_w_best_weapon(&behaviors), Q1_IT_SUPER_SHOTGUN);
        behaviors.player_ammo.shells = 1.0;
        assert_eq!(q1_w_best_weapon(&behaviors), Q1_IT_NAILGUN);
        behaviors.player_ammo.nails = 0.0;
        assert_eq!(q1_w_best_weapon(&behaviors), Q1_IT_SHOTGUN);
        behaviors.player_ammo.shells = 0.0;
        assert_eq!(q1_w_best_weapon(&behaviors), Q1_IT_AXE);
        // Lightning outranks everything above water with cells.
        behaviors.player_ammo.cells = 5.0;
        behaviors.player_state.water_level = 0;
        assert_eq!(q1_w_best_weapon(&behaviors), Q1_IT_LIGHTNING);
        // Submerged past the waist, stock will not pick it.
        behaviors.player_state.water_level = 2;
        assert_eq!(q1_w_best_weapon(&behaviors), Q1_IT_AXE);
    }

    #[test]
    fn set_current_ammo_refreshes_bits_counts_models() {
        let mut behaviors = loaded_behaviors();
        behaviors.player_state.weapon = Q1_IT_SUPER_NAILGUN;
        behaviors.player_ammo.nails = 37.0;
        q1_w_set_current_ammo(&mut behaviors);
        assert_eq!(behaviors.player_items & Q1_IT_AMMO_BITS, Q1_IT_NAILS);
        assert_eq!(behaviors.player_state.currentammo, 37.0);
        assert_eq!(behaviors.player_state.weaponmodel, "progs/v_nail2.mdl");
        behaviors.player_state.weapon = Q1_IT_AXE;
        q1_w_set_current_ammo(&mut behaviors);
        assert_eq!(behaviors.player_items & Q1_IT_AMMO_BITS, 0);
        assert_eq!(behaviors.player_state.currentammo, 0.0);
        assert_eq!(behaviors.player_state.weaponmodel, "progs/v_axe.mdl");
    }

    #[test]
    fn change_weapon_selects_owned_with_ammo() {
        let mut server = test_server();
        let player = spawn_player(&mut server);
        let mut behaviors = loaded_behaviors();
        behaviors.player_items |= Q1_IT_SUPER_SHOTGUN;
        q1_w_change_weapon(&mut behaviors, &player, 3);
        assert_eq!(behaviors.player_state.weapon, Q1_IT_SUPER_SHOTGUN);
        assert_eq!(behaviors.player_state.currentammo, 25.0);
        assert!(behaviors.sprints.is_empty());
        // Unknown impulses refuse nothing and change nothing.
        q1_w_change_weapon(&mut behaviors, &player, 0);
        q1_w_change_weapon(&mut behaviors, &player, 99);
        assert_eq!(behaviors.player_state.weapon, Q1_IT_SUPER_SHOTGUN);
        assert!(behaviors.sprints.is_empty());
    }

    #[test]
    fn change_weapon_refuses_with_stock_messages() {
        let mut server = test_server();
        let player = spawn_player(&mut server);
        let mut behaviors = loaded_behaviors();
        q1_w_change_weapon(&mut behaviors, &player, 7);
        assert_eq!(behaviors.player_state.weapon, Q1_IT_SHOTGUN);
        assert_eq!(behaviors.sprints.len(), 1);
        assert_eq!(behaviors.sprints[0].text, "no weapon.\n");
        assert_eq!(behaviors.sprints[0].target, player);
        behaviors.player_items |= Q1_IT_ROCKET_LAUNCHER;
        q1_w_change_weapon(&mut behaviors, &player, 7);
        assert_eq!(behaviors.player_state.weapon, Q1_IT_SHOTGUN);
        assert_eq!(behaviors.sprints.len(), 2);
        assert_eq!(behaviors.sprints[1].text, "not enough ammo.\n");
        behaviors.player_ammo.rockets = 3.0;
        q1_w_change_weapon(&mut behaviors, &player, 7);
        assert_eq!(behaviors.player_state.weapon, Q1_IT_ROCKET_LAUNCHER);
        assert_eq!(behaviors.player_state.currentammo, 3.0);
        assert_eq!(behaviors.sprints.len(), 2);
    }

    #[test]
    fn cycle_weapon_walks_owned_with_gates() {
        let mut behaviors = loaded_behaviors();
        behaviors.player_items |= Q1_IT_SUPER_SHOTGUN | Q1_IT_NAILGUN;
        behaviors.player_ammo.nails = 5.0;
        // Shotgun -> super shotgun (2+ shells) -> nailgun -> axe.
        q1_cycle_weapon(&mut behaviors);
        assert_eq!(behaviors.player_state.weapon, Q1_IT_SUPER_SHOTGUN);
        q1_cycle_weapon(&mut behaviors);
        assert_eq!(behaviors.player_state.weapon, Q1_IT_NAILGUN);
        q1_cycle_weapon(&mut behaviors);
        assert_eq!(behaviors.player_state.weapon, Q1_IT_AXE);
        q1_cycle_weapon(&mut behaviors);
        assert_eq!(behaviors.player_state.weapon, Q1_IT_SHOTGUN);
        // Reverse walks back down the same roster.
        q1_cycle_weapon_reverse(&mut behaviors);
        assert_eq!(behaviors.player_state.weapon, Q1_IT_AXE);
        q1_cycle_weapon_reverse(&mut behaviors);
        assert_eq!(behaviors.player_state.weapon, Q1_IT_NAILGUN);
        // A single shell starves the super shotgun both directions.
        behaviors.player_ammo.shells = 1.0;
        behaviors.player_state.weapon = Q1_IT_SHOTGUN;
        q1_w_set_current_ammo(&mut behaviors);
        q1_cycle_weapon(&mut behaviors);
        assert_eq!(behaviors.player_state.weapon, Q1_IT_NAILGUN);
    }

    #[test]
    fn impulse_commands_route_and_consume() {
        let mut server = test_server();
        let player = spawn_player(&mut server);
        let mut behaviors = loaded_behaviors();
        behaviors.player_items |= Q1_IT_SUPER_SHOTGUN;
        q1_impulse_commands(&mut behaviors, &player, 1);
        assert_eq!(behaviors.player_state.weapon, Q1_IT_AXE);
        q1_impulse_commands(&mut behaviors, &player, 10);
        assert_eq!(behaviors.player_state.weapon, Q1_IT_SHOTGUN);
        q1_impulse_commands(&mut behaviors, &player, 12);
        assert_eq!(behaviors.player_state.weapon, Q1_IT_AXE);
        // Cheat impulses are consumed unhandled until that slice lands.
        q1_impulse_commands(&mut behaviors, &player, 9);
        q1_impulse_commands(&mut behaviors, &player, 11);
        q1_impulse_commands(&mut behaviors, &player, 255);
        assert_eq!(behaviors.player_state.weapon, Q1_IT_AXE);
        assert!(behaviors.sprints.is_empty());
    }

    #[test]
    fn empty_weapon_downgrades_through_best() {
        let mut behaviors = loaded_behaviors();
        behaviors.player_items |= Q1_IT_SUPER_SHOTGUN;
        behaviors.player_state.weapon = Q1_IT_SUPER_SHOTGUN;
        // Drained shells: the gate drops to the axe (best with no
        // usable ammo) and fizzles this attack.
        behaviors.player_ammo.shells = 0.0;
        behaviors.player_state.currentammo = 0.0;
        assert!(!q1_w_check_no_ammo(&mut behaviors));
        assert_eq!(behaviors.player_state.weapon, Q1_IT_AXE);
        assert_eq!(behaviors.player_state.currentammo, 0.0);
        assert!(q1_w_check_no_ammo(&mut behaviors));
        behaviors.player_state.weapon = Q1_IT_AXE;
        behaviors.player_state.currentammo = 0.0;
        assert!(q1_w_check_no_ammo(&mut behaviors));
    }

    fn armed_behaviors() -> (Server<qa_guest::server::GuestServerLogic>, Q1NativeBehaviors, ActorId) {
        let mut server = test_server();
        let player = spawn_player(&mut server);
        let mut behaviors = loaded_behaviors();
        behaviors.player = Some(player.clone());
        (server, behaviors, player)
    }

    #[test]
    fn pain_plays_and_arms_the_gate() {
        let (server, mut behaviors, player) = armed_behaviors();
        q1_player_pain(&mut behaviors, server.simulation(), &player);
        assert!(
            behaviors
                .sounds
                .iter()
                .any(|sound| sound.sample.starts_with("player/pain") && sound.channel == 2),
            "pain1-6 on CHAN_VOICE, got {:?}",
            behaviors.sounds
        );
        assert!(matches!(behaviors.player_state.attack, Q1PlayerAttack::Pain { .. }));
        assert_eq!(behaviors.player_state.weaponframe, 0);
        assert!(behaviors.player_state.pain_finished > 0.0);
    }

    #[test]
    fn pain_gate_suppresses_and_clears_axhitme() {
        let (server, mut behaviors, player) = armed_behaviors();
        let now = server.simulation().frame().time.as_seconds_f64();
        behaviors.player_state.pain_finished = now + 5.0;
        behaviors.player_state.axhitme = true;
        q1_player_pain(&mut behaviors, server.simulation(), &player);
        assert!(behaviors.sounds.is_empty(), "gated pain stays silent");
        assert!(!behaviors.player_state.axhitme);
    }

    #[test]
    fn pain_drowns_when_submerged() {
        let (server, mut behaviors, player) = armed_behaviors();
        behaviors.player_state.water_level = 3;
        behaviors.player_state.water_type = Q1_CONTENTS_WATER;
        q1_player_pain(&mut behaviors, server.simulation(), &player);
        assert!(
            behaviors
                .sounds
                .iter()
                .any(|sound| sound.sample == "player/drown1.wav" || sound.sample == "player/drown2.wav"),
            "submerged wound drowns, got {:?}",
            behaviors.sounds
        );
        assert!(
            behaviors
                .temp_ents
                .iter()
                .any(|ent| matches!(ent, Q1TempEnt::Bubbles { count: 1, .. })),
            "drowning bubbles once"
        );
    }

    #[test]
    fn pain_burns_in_slime_and_lava() {
        let (server, mut behaviors, player) = armed_behaviors();
        behaviors.player_state.water_type = Q1_CONTENTS_SLIME;
        q1_player_pain(&mut behaviors, server.simulation(), &player);
        assert!(
            behaviors
                .sounds
                .iter()
                .any(|sound| sound.sample == "player/lburn1.wav" || sound.sample == "player/lburn2.wav"),
            "slime wound burns, got {:?}",
            behaviors.sounds
        );
        behaviors.sounds.clear();
        behaviors.player_state.water_type = Q1_CONTENTS_LAVA;
        behaviors.player_state.pain_finished = 0.0;
        q1_player_pain(&mut behaviors, server.simulation(), &player);
        assert!(
            behaviors
                .sounds
                .iter()
                .any(|sound| sound.sample == "player/lburn1.wav" || sound.sample == "player/lburn2.wav"),
            "lava wound burns"
        );
    }

    #[test]
    fn pain_axe_mark_cries_once() {
        let (server, mut behaviors, player) = armed_behaviors();
        behaviors.player_state.axhitme = true;
        q1_player_pain(&mut behaviors, server.simulation(), &player);
        assert!(
            behaviors.sounds.iter().any(|sound| sound.sample == "player/axhit1.wav"),
            "axe-marked wound cries axhit1"
        );
        assert!(!behaviors.player_state.axhitme);
    }

    #[test]
    fn obituary_suicide_lines() {
        let (server, mut behaviors, player) = armed_behaviors();
        behaviors.player_state.weapon = Q1_IT_LIGHTNING;
        behaviors.player_state.water_level = 2;
        q1_client_obituary(&mut behaviors, server.simulation(), &player, Some(&player));
        assert_eq!(
            behaviors.sprints.last().expect("sprint").text,
            "Player discharges into the water.\n"
        );
        assert_eq!(behaviors.player_state.frags, -1.0);
        behaviors.player_state.weapon = Q1_IT_GRENADE_LAUNCHER;
        q1_client_obituary(&mut behaviors, server.simulation(), &player, Some(&player));
        assert_eq!(
            behaviors.sprints.last().expect("sprint").text,
            "Player tries to put the pin back in\n"
        );
        behaviors.player_state.weapon = Q1_IT_SHOTGUN;
        q1_client_obituary(&mut behaviors, server.simulation(), &player, Some(&player));
        assert_eq!(
            behaviors.sprints.last().expect("sprint").text,
            "Player becomes bored with life\n"
        );
        assert_eq!(behaviors.player_state.frags, -3.0);
    }

    #[test]
    fn obituary_world_lines() {
        let (server, mut behaviors, player) = armed_behaviors();
        q1_client_obituary(&mut behaviors, server.simulation(), &player, None);
        assert_eq!(behaviors.sprints.last().expect("sprint").text, "Player died\n");
        behaviors.player_state.deathtype = "falling".to_string();
        q1_client_obituary(&mut behaviors, server.simulation(), &player, None);
        assert_eq!(
            behaviors.sprints.last().expect("sprint").text,
            "Player fell to his death\n"
        );
        assert_eq!(behaviors.player_state.deathtype, "");
        behaviors.player_state.water_type = Q1_CONTENTS_WATER;
        q1_client_obituary(&mut behaviors, server.simulation(), &player, None);
        let text = behaviors.sprints.last().expect("sprint").text.clone();
        assert!(
            text == "Player sleeps with the fishes\n" || text == "Player sucks it down\n",
            "water line, got {text:?}"
        );
    }

    #[test]
    fn death_anim_frame_counts() {
        assert_eq!(q1_death_anim_frames(Q1_DEATH_AXE), 9);
        assert_eq!(q1_death_anim_frames(Q1_DEATH_A), 11);
        assert_eq!(q1_death_anim_frames(Q1_DEATH_B), 9);
        assert_eq!(q1_death_anim_frames(Q1_DEATH_C), 15);
        assert_eq!(q1_death_anim_frames(Q1_DEATH_D), 9);
        assert_eq!(q1_death_anim_frames(Q1_DEATH_E), 9);
    }

    #[test]
    fn die_gibs_past_minus_40() {
        let (mut server, mut behaviors, player) = armed_behaviors();
        server
            .simulation_mut()
            .set_combat(
                &player,
                CombatState {
                    health: -50.0,
                    ..CombatState::default()
                },
            )
            .unwrap();
        let (simulation, _, _) = server.simulation_movers_and_triggers_mut();
        q1_player_die(&mut behaviors, simulation, &player);
        assert_eq!(behaviors.player_state.deadflag, Q1_DEAD_DEAD);
        assert!(behaviors.player_state.gibbed_head);
        assert_eq!(behaviors.pending_gibs.len(), 3);
        assert_eq!(behaviors.pending_gibs[0].model, "progs/gib1.mdl");
        assert!(
            behaviors
                .sounds
                .iter()
                .any(|sound| sound.sample == "player/gib.wav" || sound.sample == "player/udeath.wav"),
            "gib cry, got {:?}",
            behaviors.sounds
        );
    }

    #[test]
    fn die_runs_anim_above_minus_40() {
        let (mut server, mut behaviors, player) = armed_behaviors();
        server
            .simulation_mut()
            .set_combat(
                &player,
                CombatState {
                    health: -10.0,
                    ..CombatState::default()
                },
            )
            .unwrap();
        behaviors.player_state.weapon = Q1_IT_AXE;
        let (simulation, _, _) = server.simulation_movers_and_triggers_mut();
        q1_player_die(&mut behaviors, simulation, &player);
        assert_eq!(behaviors.player_state.deadflag, Q1_DEAD_DYING);
        assert_eq!(behaviors.player_state.death_anim, Q1_DEATH_AXE);
        assert_eq!(behaviors.player_state.death_frames, 1);
        assert!(
            behaviors
                .sounds
                .iter()
                .any(|sound| sound.sample.starts_with("player/death")),
            "death cry, got {:?}",
            behaviors.sounds
        );
    }

    #[test]
    fn body_queue_rings_past_four() {
        let (server, mut behaviors, player) = armed_behaviors();
        for frame in 1..=5 {
            behaviors.player_state.death_frames = frame;
            q1_copy_to_bodyque(&mut behaviors, server.simulation(), &player);
        }
        assert_eq!(behaviors.player_state.body_queue.len(), 4);
        assert_eq!(behaviors.player_state.body_queue[0].frame, 2, "oldest dropped");
        assert_eq!(behaviors.player_state.body_queue[3].frame, 5);
    }

    fn test_scene() -> SharedSceneQueries {
        use qa_bots::q1_collision::{
            Q1BspChild, Q1CollisionGeometry, Q1CollisionLeaf, Q1CollisionModel, Q1CollisionNode,
        };
        use qa_bots::scene::{BspPlane, IndexRange};
        use qa_bots::shared_scene::DecodedCollisionWorld;
        use qa_world::hull::{ClipChild, ClipNode};

        let geometry = Q1CollisionGeometry {
            models: vec![Q1CollisionModel {
                bounds: qa_core::math::Bounds {
                    min: vec3(-64.0, -64.0, -64.0),
                    max: vec3(64.0, 64.0, 64.0),
                },
                headnodes: vec![0, 0, 0],
                visible_leaves: 0,
                faces: IndexRange { first: 0, count: 0 },
            }],
            nodes: vec![Q1CollisionNode {
                plane: 0,
                children: [Q1BspChild::Leaf(0), Q1BspChild::Leaf(1)],
            }],
            clipnodes: vec![ClipNode {
                plane: 0,
                children: [ClipChild::Contents(-2), ClipChild::Contents(-1)],
            }],
            planes: vec![BspPlane {
                normal: vec3(1.0, 0.0, 0.0),
                distance: 0.0,
                plane_type: 0,
                signbits: 0,
            }],
            leaves: vec![
                Q1CollisionLeaf {
                    contents: -2,
                    visibility_offset: None,
                },
                Q1CollisionLeaf {
                    contents: -1,
                    visibility_offset: None,
                },
            ],
            faces: vec![],
            texture_info: vec![],
            textures: vec![],
            vertices: vec![],
            edges: vec![],
            surface_edges: vec![],
            visibility: vec![],
            brush_list: vec![],
        };
        SharedSceneQueries::new(DecodedCollisionWorld::Q1(geometry)).unwrap()
    }

    fn flesh_ctx<'a>(
        server: &'a mut Server<qa_guest::server::GuestServerLogic>,
        behaviors: &'a mut Q1NativeBehaviors,
        scene: &'a SharedSceneQueries,
        player: &ActorId,
    ) -> Q1WeaponFire<'a, 'a, 'a, qa_guest::server::GuestServerLogic> {
        Q1WeaponFire {
            server,
            behaviors,
            scene,
            player: player.clone(),
            view_angles: vec3(0.0, 0.0, 0.0),
            now: 1.0,
        }
    }

    fn spawn_flesh(
        server: &mut Server<qa_guest::server::GuestServerLogic>,
        behaviors: &mut Q1NativeBehaviors,
        owner: &ActorId,
    ) -> ActorId {
        super::super::native_q1_spawns::register_q1_spawns(server.spawns_mut());
        q1_spawn_missile(
            server,
            behaviors,
            Q1MissileSpawn {
                kind: Q1MissileKind::ZombieFlesh,
                owner: owner.clone(),
                origin: vec3(0.0, 0.0, 24.0),
                velocity: vec3(600.0, 0.0, 200.0),
                avelocity: vec3(3000.0, 1000.0, 2000.0),
                effects: 0,
                fuse_at: None,
                remove_at: 3.5,
                born_at: 1.0,
            },
        )
        .expect("flesh spawns")
    }

    #[test]
    fn zombie_flesh_wounds_and_removes() {
        let mut server = test_server();
        let mut behaviors = Q1NativeBehaviors::new();
        let scene = test_scene();
        let player = spawn_player(&mut server);
        behaviors.set_player(Some(player.clone()));
        let owner = spawn_player(&mut server);
        let flesh = spawn_flesh(&mut server, &mut behaviors, &owner);
        let missile = behaviors.missiles.get(&flesh).cloned().expect("flesh record");
        let hit = Q1LineHit {
            fraction: 0.5,
            endpos: vec3(10.0, 0.0, 24.0),
            hit_actor: Some(player.clone()),
            plane_normal: vec3(-1.0, 0.0, 0.0),
        };
        let mut ctx = flesh_ctx(&mut server, &mut behaviors, &scene, &player);
        q1_zombie_flesh_impact(&mut ctx, &flesh, &missile, &hit);
        assert_eq!(q1_health_of(ctx.server.simulation(), &player), 90.0);
        assert!(
            ctx.behaviors
                .sounds
                .iter()
                .any(|sound| sound.sample == "zombie/z_hit.wav"),
            "strikes thump wet"
        );
        assert!(!ctx.behaviors.missiles.contains_key(&flesh), "struck flesh removes");
    }

    #[test]
    fn zombie_flesh_miss_stops_then_removes() {
        let mut server = test_server();
        let mut behaviors = Q1NativeBehaviors::new();
        let scene = test_scene();
        let player = spawn_player(&mut server);
        behaviors.set_player(Some(player.clone()));
        let owner = spawn_player(&mut server);
        let flesh = spawn_flesh(&mut server, &mut behaviors, &owner);
        let wall = Q1LineHit {
            fraction: 0.5,
            endpos: vec3(10.0, 0.0, 24.0),
            hit_actor: None,
            plane_normal: vec3(-1.0, 0.0, 0.0),
        };
        let missile = behaviors.missiles.get(&flesh).cloned().expect("flesh record");
        let mut ctx = flesh_ctx(&mut server, &mut behaviors, &scene, &player);
        q1_zombie_flesh_impact(&mut ctx, &flesh, &missile, &wall);
        // First miss: thud, dead stop, still flying with `touch = Remove` armed.
        assert!(
            ctx.behaviors
                .sounds
                .iter()
                .any(|sound| sound.sample == "zombie/z_miss.wav"),
            "misses thud"
        );
        let body = ctx.server.simulation().body_state(&flesh).expect("flesh body");
        assert_eq!(body.velocity, vec3(0.0, 0.0, 0.0));
        let record = ctx.behaviors.missiles.get(&flesh).expect("spent flesh still flies");
        assert_eq!(record.avelocity, vec3(0.0, 0.0, 0.0));
        assert!(record.spent);
        // Second touch removes silently.
        let missile = record.clone();
        q1_zombie_flesh_impact(&mut ctx, &flesh, &missile, &wall);
        assert!(!ctx.behaviors.missiles.contains_key(&flesh), "re-touched flesh removes");
        assert_eq!(
            ctx.behaviors
                .sounds
                .iter()
                .filter(|sound| sound.sample == "zombie/z_miss.wav")
                .count(),
            1,
            "the removal touch stays silent"
        );
    }

    #[test]
    fn shambler_takes_half_grenade_blast() {
        use super::super::native_q1_monsters::{build_q1_monster, register_q1_monster_spawns, Q1_FLAG_MONSTER};

        let mut server = test_server();
        let mut behaviors = Q1NativeBehaviors::new();
        let scene = test_scene();
        let player = spawn_player(&mut server);
        register_q1_monster_spawns(server.spawns_mut());
        let fields = SpawnFields::parse(&[("classname", "monster_shambler"), ("origin", "0 0 0")]).unwrap();
        let shambler = server.spawn_entity(&fields).unwrap();
        build_q1_monster(&mut server, &mut behaviors, &shambler, &fields).unwrap();
        let combat = server.simulation().combat_state(shambler.id()).cloned().unwrap();
        server
            .simulation_mut()
            .set_combat(
                shambler.id(),
                CombatState {
                    can_take_damage: true,
                    ..combat
                },
            )
            .unwrap();
        let monster = behaviors.monsters.get_mut(shambler.id()).unwrap();
        monster.flags |= Q1_FLAG_MONSTER;
        monster.takedamage = Q1_DAMAGE_AIM;
        super::super::native_q1_spawns::register_q1_spawns(server.spawns_mut());
        let grenade = q1_spawn_missile(
            &mut server,
            &mut behaviors,
            Q1MissileSpawn {
                kind: Q1MissileKind::Grenade,
                owner: player.clone(),
                // The lane's radius falloff measures to the victim's
                // center, so park the blast on it for a full-120 hit.
                origin: vec3(0.0, 0.0, 20.0),
                velocity: vec3(0.0, 0.0, 0.0),
                avelocity: vec3(0.0, 0.0, 0.0),
                effects: 0,
                fuse_at: None,
                remove_at: 99.0,
                born_at: 0.0,
            },
        )
        .unwrap();
        let mut ctx = flesh_ctx(&mut server, &mut behaviors, &scene, &player);
        q1_grenade_explode(&mut ctx, &grenade, None, 120.0, 8.0);
        // Point-blank 120 halves to 60 (`combat.qc:236`).
        assert_eq!(q1_health_of(ctx.server.simulation(), shambler.id()), 540.0);
    }

    #[test]
    fn wizspike_wall_sparks_green() {
        let mut server = test_server();
        let mut behaviors = Q1NativeBehaviors::new();
        let scene = test_scene();
        let player = spawn_player(&mut server);
        behaviors.set_player(Some(player.clone()));
        super::super::native_q1_spawns::register_q1_spawns(server.spawns_mut());
        let spike = q1_spawn_missile(
            &mut server,
            &mut behaviors,
            Q1MissileSpawn {
                kind: Q1MissileKind::WizSpike,
                owner: player.clone(),
                origin: vec3(0.0, 0.0, 24.0),
                velocity: vec3(600.0, 0.0, 0.0),
                avelocity: vec3(0.0, 0.0, 0.0),
                effects: 0,
                fuse_at: None,
                remove_at: 7.0,
                born_at: 1.0,
            },
        )
        .expect("spike spawns");
        let missile = behaviors.missiles.get(&spike).cloned().expect("spike record");
        let wall = Q1LineHit {
            fraction: 0.5,
            endpos: vec3(10.0, 0.0, 24.0),
            hit_actor: None,
            plane_normal: vec3(-1.0, 0.0, 0.0),
        };
        let mut ctx = flesh_ctx(&mut server, &mut behaviors, &scene, &player);
        assert!(q1_missile_impact(&mut ctx, &spike, &missile, &wall));
        assert!(ctx.behaviors.missiles.get(&spike).is_none(), "spikes remove on walls");
        assert!(
            ctx.behaviors.temp_ents.iter().any(|ent| matches!(
                ent,
                Q1TempEnt::WizSpike { at } if *at == vec3(10.0, 0.0, 24.0)
            )),
            "walls spark TE_WIZSPIKE, not TE_SPIKE"
        );
    }
}
