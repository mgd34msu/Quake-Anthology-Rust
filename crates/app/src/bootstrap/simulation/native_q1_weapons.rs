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
use qa_core::math::{angle_vectors, vec3, Vec3};
use qa_core::numeric::Q1_DONOR_PROFILE;
use qa_world::body::translated_body_bounds;
use qa_world::movement::q1::types::{Q1_CONTENTS_LAVA, Q1_CONTENTS_SLIME, Q1_CONTENTS_WATER};
use qa_world::server::{Server, ServerLogic};

use super::native_q1_items::Q1Sprint;
use super::native_q1_monsters::{q1_monster_crandom, q1_monster_random, q1_monster_sound, q1_t_damage, Q1_DAMAGE_AIM};
use super::native_q1_spawns::{q1_can_take_damage, Q1NativeBehaviors};

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

/// Stock deadflag values (`defs.qc:273-276`).
pub const Q1_DEAD_NO: u8 = 0;
/// Stock deadflag values (`defs.qc:273-276`).
pub const Q1_DEAD_DYING: u8 = 1;
/// Stock deadflag values (`defs.qc:273-276`).
pub const Q1_DEAD_DEAD: u8 = 2;
/// Stock deadflag values (`defs.qc:273-276`).
pub const Q1_DEAD_RESPAWNABLE: u8 = 3;

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
    /// Axe swing in flight; `W_FireAxe` lands at `fire_at` (frame 3,
    /// 0.2 s after `W_Attack`, `player.qc:152`).
    AxeSwing {
        /// Master-clock seconds of the frame-3 fire.
        fire_at: f64,
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
    /// `TE_EXPLOSION` at the blast center.
    Explosion {
        /// Blast center.
        at: Vec3,
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
    let Ok(trace) = scene.trace(&query) else {
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
/// the swing's frame 3 (`player.qc:152`); the remaining five weapons
/// land in the projectile and lightning slices.
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
        ctx.behaviors.player_state.attack = Q1PlayerAttack::AxeSwing { fire_at: ctx.now + 0.2 };
        ctx.behaviors.player_state.attack_finished = ctx.now + 0.5;
    } else if weapon == Q1_IT_SHOTGUN {
        ctx.behaviors.player_state.weaponframe = 1;
        q1_fire_shotgun(ctx);
        ctx.behaviors.player_state.attack_finished = ctx.now + 0.5;
    } else if weapon == Q1_IT_SUPER_SHOTGUN {
        ctx.behaviors.player_state.weaponframe = 1;
        q1_fire_super_shotgun(ctx);
        ctx.behaviors.player_state.attack_finished = ctx.now + 0.7;
    }
}

/// Run one due attack-anim think: the axe swing fires at frame 3 and
/// the anim retires (`player_run`).
fn q1_attack_think<L: ServerLogic>(ctx: &mut Q1WeaponFire<'_, '_, '_, L>) {
    let attack = ctx.behaviors.player_state.attack;
    match attack {
        Q1PlayerAttack::None => {}
        Q1PlayerAttack::AxeSwing { fire_at } => {
            if ctx.now >= fire_at {
                ctx.behaviors.player_state.attack = Q1PlayerAttack::None;
                ctx.behaviors.player_state.weaponframe = 3;
                q1_fire_axe(ctx);
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
    if behaviors.player_state.deadflag != Q1_DEAD_NO {
        return;
    }
    let now = server.simulation().frame().time.as_seconds_f64();
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
    q1_attack_think(&mut ctx);
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
}
