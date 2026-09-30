//! Quake III Team Arena (`q3_team_arena`) support: shared mirrors, group error, and tests.
//!
//! Self-containment mirrors: minimal local copies of items the donors import from
//! modules outside this port (sibling q3 donors, engine contracts, and math/text
//! helpers), plus the group error type. Sibling-owned mirrors carry SIBLING-MIRROR
//! notes and unify with the canonical ports at merge time.

use qa_core::identity::{ActorId, IdentityOwner};
use qa_core::math::{add3, scale3, vec3, Bounds, Vec3};
use qa_core::numeric::{q_rand, qvm_float_to_int};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

// Intra-group imports: sibling modules split from the same flat port.

// ---------------------------------------------------------------------------
// SIBLING-MIRROR: base/shared/definitions.ts + movement/q3/constants.ts codes.
// ---------------------------------------------------------------------------

/// Game product (`Product`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Product {
    /// Vanilla Quake III (`baseq3`).
    BaseQ3,
    /// Team Arena expansion (`missionpack`).
    MissionPack,
}

impl Product {
    /// Donor string spelling.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Product::BaseQ3 => "baseq3",
            Product::MissionPack => "missionpack",
        }
    }

    /// Whether this is the Team Arena product.
    #[must_use]
    pub fn is_missionpack(self) -> bool {
        matches!(self, Product::MissionPack)
    }
}

/// Game type codes (`GameType`).
pub mod game_type {
    pub const FFA: i32 = 0;
    pub const TOURNAMENT: i32 = 1;
    pub const SINGLE_PLAYER: i32 = 2;
    pub const TEAM: i32 = 3;
    pub const CTF: i32 = 4;
    pub const ONE_FLAG_CTF: i32 = 5;
    pub const OBELISK: i32 = 6;
    pub const HARVESTER: i32 = 7;
    pub const MAX_GAME_TYPE: i32 = 8;
}

/// Team codes (`Team`).
pub mod team {
    pub const FREE: i32 = 0;
    pub const RED: i32 = 1;
    pub const BLUE: i32 = 2;
    pub const SPECTATOR: i32 = 3;
    pub const NUM_TEAMS: i32 = 4;
}

/// Player movement types (`MoveType`).
pub mod move_type {
    pub const NORMAL: i32 = 0;
    pub const NOCLIP: i32 = 1;
    pub const SPECTATOR: i32 = 2;
    pub const DEAD: i32 = 3;
    pub const FREEZE: i32 = 4;
    pub const INTERMISSION: i32 = 5;
    pub const SPINTERMISSION: i32 = 6;
}

/// Weapon states (`WeaponState`).
pub mod weapon_state {
    pub const READY: i32 = 0;
    pub const RAISING: i32 = 1;
    pub const DROPPING: i32 = 2;
    pub const FIRING: i32 = 3;
}

/// Powerup tags (`Powerup`).
pub mod powerup {
    pub const NONE: i32 = 0;
    pub const QUAD: i32 = 1;
    pub const BATTLESUIT: i32 = 2;
    pub const HASTE: i32 = 3;
    pub const INVIS: i32 = 4;
    pub const REGEN: i32 = 5;
    pub const FLIGHT: i32 = 6;
    pub const REDFLAG: i32 = 7;
    pub const BLUEFLAG: i32 = 8;
    pub const NEUTRALFLAG: i32 = 9;
    pub const SCOUT: i32 = 10;
    pub const GUARD: i32 = 11;
    pub const DOUBLER: i32 = 12;
    pub const AMMOREGEN: i32 = 13;
    pub const INVULNERABILITY: i32 = 14;
    pub const NUM_POWERUPS: i32 = 15;
}

/// Weapon tags (`Weapon`).
pub mod weapon {
    pub const NONE: i32 = 0;
    pub const GAUNTLET: i32 = 1;
    pub const MACHINEGUN: i32 = 2;
    pub const SHOTGUN: i32 = 3;
    pub const GRENADE_LAUNCHER: i32 = 4;
    pub const ROCKET_LAUNCHER: i32 = 5;
    pub const LIGHTNING: i32 = 6;
    pub const RAILGUN: i32 = 7;
    pub const PLASMAGUN: i32 = 8;
    pub const BFG: i32 = 9;
    pub const GRAPPLING_HOOK: i32 = 10;
    pub const NAILGUN: i32 = 11;
    pub const PROX_LAUNCHER: i32 = 12;
    pub const CHAINGUN: i32 = 13;
}

/// Entity event codes (`EntityEvent`).
pub mod entity_event {
    pub const NONE: i32 = 0;
    pub const FOOTSTEP: i32 = 1;
    pub const FOOTSTEP_METAL: i32 = 2;
    pub const FOOTSPLASH: i32 = 3;
    pub const FOOTWADE: i32 = 4;
    pub const SWIM: i32 = 5;
    pub const STEP_4: i32 = 6;
    pub const STEP_8: i32 = 7;
    pub const STEP_12: i32 = 8;
    pub const STEP_16: i32 = 9;
    pub const FALL_SHORT: i32 = 10;
    pub const FALL_MEDIUM: i32 = 11;
    pub const FALL_FAR: i32 = 12;
    pub const JUMP_PAD: i32 = 13;
    pub const JUMP: i32 = 14;
    pub const WATER_TOUCH: i32 = 15;
    pub const WATER_LEAVE: i32 = 16;
    pub const WATER_UNDER: i32 = 17;
    pub const WATER_CLEAR: i32 = 18;
    pub const ITEM_PICKUP: i32 = 19;
    pub const GLOBAL_ITEM_PICKUP: i32 = 20;
    pub const NOAMMO: i32 = 21;
    pub const CHANGE_WEAPON: i32 = 22;
    pub const FIRE_WEAPON: i32 = 23;
    pub const USE_ITEM0: i32 = 24;
    pub const USE_ITEM1: i32 = 25;
    pub const USE_ITEM2: i32 = 26;
    pub const USE_ITEM3: i32 = 27;
    pub const USE_ITEM4: i32 = 28;
    pub const USE_ITEM5: i32 = 29;
    pub const USE_ITEM6: i32 = 30;
    pub const USE_ITEM7: i32 = 31;
    pub const USE_ITEM8: i32 = 32;
    pub const USE_ITEM9: i32 = 33;
    pub const USE_ITEM10: i32 = 34;
    pub const USE_ITEM11: i32 = 35;
    pub const USE_ITEM12: i32 = 36;
    pub const USE_ITEM13: i32 = 37;
    pub const USE_ITEM14: i32 = 38;
    pub const USE_ITEM15: i32 = 39;
    pub const ITEM_RESPAWN: i32 = 40;
    pub const ITEM_POP: i32 = 41;
    pub const PLAYER_TELEPORT_IN: i32 = 42;
    pub const PLAYER_TELEPORT_OUT: i32 = 43;
    pub const GRENADE_BOUNCE: i32 = 44;
    pub const GENERAL_SOUND: i32 = 45;
    pub const GLOBAL_SOUND: i32 = 46;
    pub const GLOBAL_TEAM_SOUND: i32 = 47;
    pub const BULLET_HIT_FLESH: i32 = 48;
    pub const BULLET_HIT_WALL: i32 = 49;
    pub const MISSILE_HIT: i32 = 50;
    pub const MISSILE_MISS: i32 = 51;
    pub const MISSILE_MISS_METAL: i32 = 52;
    pub const RAILTRAIL: i32 = 53;
    pub const SHOTGUN: i32 = 54;
    pub const BULLET: i32 = 55;
    pub const PAIN: i32 = 56;
    pub const DEATH1: i32 = 57;
    pub const DEATH2: i32 = 58;
    pub const DEATH3: i32 = 59;
    pub const OBITUARY: i32 = 60;
    pub const POWERUP_QUAD: i32 = 61;
    pub const POWERUP_BATTLESUIT: i32 = 62;
    pub const POWERUP_REGEN: i32 = 63;
    pub const GIB_PLAYER: i32 = 64;
    pub const SCOREPLUM: i32 = 65;
    pub const PROXIMITY_MINE_STICK: i32 = 66;
    pub const PROXIMITY_MINE_TRIGGER: i32 = 67;
    pub const KAMIKAZE: i32 = 68;
    pub const OBELISKEXPLODE: i32 = 69;
    pub const OBELISKPAIN: i32 = 70;
    pub const INVUL_IMPACT: i32 = 71;
    pub const JUICED: i32 = 72;
    pub const LIGHTNINGBOLT: i32 = 73;
    pub const DEBUG_LINE: i32 = 74;
    pub const STOPLOOPINGSOUND: i32 = 75;
    pub const TAUNT: i32 = 76;
    pub const TAUNT_YES: i32 = 77;
    pub const TAUNT_NO: i32 = 78;
    pub const TAUNT_FOLLOWME: i32 = 79;
    pub const TAUNT_GETFLAG: i32 = 80;
    pub const TAUNT_GUARDBASE: i32 = 81;
    pub const TAUNT_PATROL: i32 = 82;
}

/// Entity type codes (`EntityType`).
pub mod entity_type {
    pub const GENERAL: i32 = 0;
    pub const PLAYER: i32 = 1;
    pub const ITEM: i32 = 2;
    pub const MISSILE: i32 = 3;
    pub const MOVER: i32 = 4;
    pub const BEAM: i32 = 5;
    pub const PORTAL: i32 = 6;
    pub const SPEAKER: i32 = 7;
    pub const PUSH_TRIGGER: i32 = 8;
    pub const TELEPORT_TRIGGER: i32 = 9;
    pub const INVISIBLE: i32 = 10;
    pub const GRAPPLE: i32 = 11;
    pub const TEAM: i32 = 12;
    pub const EVENTS: i32 = 13;
}

/// Persistent player-state indices (`PersistentIndex`).
pub mod persistent_index {
    pub const SCORE: i32 = 0;
    pub const HITS: i32 = 1;
    pub const RANK: i32 = 2;
    pub const TEAM: i32 = 3;
    pub const SPAWN_COUNT: i32 = 4;
    pub const PLAYEREVENTS: i32 = 5;
    pub const ATTACKER: i32 = 6;
    pub const ATTACKEE_ARMOR: i32 = 7;
    pub const KILLED: i32 = 8;
    pub const IMPRESSIVE_COUNT: i32 = 9;
    pub const EXCELLENT_COUNT: i32 = 10;
    pub const DEFEND_COUNT: i32 = 11;
    pub const ASSIST_COUNT: i32 = 12;
    pub const GAUNTLET_FRAG_COUNT: i32 = 13;
    pub const CAPTURES: i32 = 14;
}

/// Trajectory types (`TrajectoryType`).
pub mod trajectory_type {
    pub const STATIONARY: i32 = 0;
    pub const INTERPOLATE: i32 = 1;
    pub const LINEAR: i32 = 2;
    pub const LINEAR_STOP: i32 = 3;
    pub const SINE: i32 = 4;
    pub const GRAVITY: i32 = 5;
}

/// Player animation slots (`PlayerAnimation`).
pub mod player_animation {
    pub const BOTH_DEATH1: i32 = 0;
    pub const BOTH_DEAD1: i32 = 1;
    pub const BOTH_DEATH2: i32 = 2;
    pub const BOTH_DEAD2: i32 = 3;
    pub const BOTH_DEATH3: i32 = 4;
    pub const BOTH_DEAD3: i32 = 5;
    pub const TORSO_GESTURE: i32 = 6;
    pub const TORSO_ATTACK: i32 = 7;
    pub const TORSO_ATTACK2: i32 = 8;
    pub const TORSO_DROP: i32 = 9;
    pub const TORSO_RAISE: i32 = 10;
    pub const TORSO_STAND: i32 = 11;
    pub const TORSO_STAND2: i32 = 12;
    pub const LEGS_WALKCR: i32 = 13;
    pub const LEGS_WALK: i32 = 14;
    pub const LEGS_RUN: i32 = 15;
    pub const LEGS_BACK: i32 = 16;
    pub const LEGS_SWIM: i32 = 17;
    pub const LEGS_JUMP: i32 = 18;
    pub const LEGS_LAND: i32 = 19;
    pub const LEGS_JUMPB: i32 = 20;
    pub const LEGS_LANDB: i32 = 21;
    pub const LEGS_IDLE: i32 = 22;
    pub const LEGS_IDLECR: i32 = 23;
    pub const LEGS_TURN: i32 = 24;
    pub const TORSO_GETFLAG: i32 = 25;
    pub const TORSO_GUARDBASE: i32 = 26;
    pub const TORSO_PATROL: i32 = 27;
    pub const TORSO_FOLLOWME: i32 = 28;
    pub const TORSO_AFFIRMATIVE: i32 = 29;
    pub const TORSO_NEGATIVE: i32 = 30;
    pub const LEGS_BACKCR: i32 = 32;
    pub const LEGS_BACKWALK: i32 = 33;
    pub const FLAG_RUN: i32 = 34;
    pub const FLAG_STAND: i32 = 35;
    pub const FLAG_STAND2RUN: i32 = 36;
}

/// Player movement flag bits (`MoveFlags`).
pub mod move_flags {
    pub const DUCKED: i32 = 1;
    pub const JUMP_HELD: i32 = 2;
    pub const BACKWARDS_JUMP: i32 = 8;
    pub const BACKWARDS_RUN: i32 = 16;
    pub const TIME_LAND: i32 = 32;
    pub const TIME_KNOCKBACK: i32 = 64;
    pub const TIME_WATERJUMP: i32 = 256;
    pub const RESPAWNED: i32 = 512;
    pub const USE_ITEM_HELD: i32 = 1024;
    pub const GRAPPLE_PULL: i32 = 2048;
    pub const FOLLOW: i32 = 4096;
    pub const SCOREBOARD: i32 = 8192;
    pub const INVULEXPAND: i32 = 16384;
}

/// User-command button bits (`CommandButtons`).
pub mod command_buttons {
    pub const ATTACK: i32 = 1;
    pub const TALK: i32 = 2;
    pub const USE_HOLDABLE: i32 = 4;
    pub const GESTURE: i32 = 8;
    pub const WALKING: i32 = 16;
    pub const AFFIRMATIVE: i32 = 32;
    pub const NEGATIVE: i32 = 64;
    pub const GETFLAG: i32 = 128;
    pub const GUARDBASE: i32 = 256;
    pub const PATROL: i32 = 512;
    pub const FOLLOWME: i32 = 1024;
    pub const ANY: i32 = 2048;
}

/// Connection states (`ConnectionState`).
pub mod connection_state {
    pub const DISCONNECTED: i32 = 0;
    pub const CONNECTING: i32 = 1;
    pub const CONNECTED: i32 = 2;
}

/// Spectator states (`SpectatorState`).
pub mod spectator_state {
    pub const NOT: i32 = 0;
    pub const FREE: i32 = 1;
    pub const FOLLOW: i32 = 2;
    pub const SCOREBOARD: i32 = 3;
}

/// Team states (`TeamState`).
pub mod team_state {
    pub const BEGIN: i32 = 0;
    pub const ACTIVE: i32 = 1;
}

/// Entity flag bits (`GameFlags`).
pub mod game_flags {
    pub const GODMODE: i32 = 0x10;
    pub const NOTARGET: i32 = 0x20;
    pub const TEAMSLAVE: i32 = 0x400;
    pub const NO_KNOCKBACK: i32 = 0x800;
    pub const DROPPED_ITEM: i32 = 0x1000;
    pub const NO_BOTS: i32 = 0x2000;
    pub const NO_HUMANS: i32 = 0x4000;
    pub const FORCE_GESTURE: i32 = 0x8000;
}

/// Server entity flag bits (`ServerEntityFlags`).
pub mod server_entity_flags {
    pub const NOCLIENT: i32 = 1;
    pub const CLIENTMASK: i32 = 2;
    pub const BOT: i32 = 8;
    pub const BROADCAST: i32 = 32;
    pub const PORTAL: i32 = 64;
    pub const USE_CURRENT_ORIGIN: i32 = 128;
    pub const SINGLECLIENT: i32 = 256;
    pub const NOSERVERINFO: i32 = 512;
    pub const NOTSINGLECLIENT: i32 = 2048;
}

/// Damage flag bits (`DamageFlags`).
pub mod damage_flags {
    pub const RADIUS: i32 = 0x1;
    pub const NO_ARMOR: i32 = 0x2;
    pub const NO_KNOCKBACK: i32 = 0x4;
    pub const NO_PROTECTION: i32 = 0x8;
    pub const NO_TEAM_PROTECTION: i32 = 0x10;
}

/// World entity slot (`ENTITYNUM_WORLD`).
pub const ENTITYNUM_WORLD: usize = 1022;

/// Null entity slot (`ENTITYNUM_NONE`).
pub const ENTITYNUM_NONE: i32 = 1023;

/// Maximum clients (`MAX_CLIENTS`).
pub const MAX_CLIENTS: usize = 64;

/// Maximum entities (`MAX_GENTITIES`).
pub const MAX_GENTITIES: usize = 1024;

/// Health at or below which a player gibs (`GIB_HEALTH`).
pub const GIB_HEALTH: i32 = -40;

/// Event validity window (`EVENT_VALID_MSEC`).
pub const EVENT_VALID_MSEC: i32 = 300;

/// Event-bit rotation constants (`EV_EVENT_BIT1/BITS`).
pub const EV_EVENT_BIT1: i32 = 0x100;

/// Event-bit mask (`EV_EVENT_BITS`).
pub const EV_EVENT_BITS: i32 = 0x300;

/// Player-state stat slot layout (`StatSchema`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatSchema {
    /// Product this layout belongs to.
    pub product: Product,
    /// Health slot.
    pub health: usize,
    /// Holdable-item slot.
    pub holdable_item: usize,
    /// Persistent-powerup slot (missionpack only).
    pub persistent_powerup: Option<usize>,
    /// Weapons bitmask slot.
    pub weapons: usize,
    /// Armor slot.
    pub armor: usize,
    /// Dead-yaw slot.
    pub dead_yaw: usize,
    /// Clients-ready bitmask slot.
    pub clients_ready: usize,
    /// Maximum-health slot.
    pub max_health: usize,
}

/// Stat slot layout for a product (`statSchema`).
#[must_use]
pub fn stat_schema(product: Product) -> StatSchema {
    match product {
        Product::BaseQ3 => StatSchema {
            product,
            health: 0,
            holdable_item: 1,
            persistent_powerup: None,
            weapons: 2,
            armor: 3,
            dead_yaw: 4,
            clients_ready: 5,
            max_health: 6,
        },
        Product::MissionPack => StatSchema {
            product,
            health: 0,
            holdable_item: 1,
            persistent_powerup: Some(2),
            weapons: 3,
            armor: 4,
            dead_yaw: 5,
            clients_ready: 6,
            max_health: 7,
        },
    }
}

/// Weapon tag count for a product (`weaponCount`).
#[must_use]
pub fn weapon_count(product: Product) -> usize {
    match product {
        Product::BaseQ3 => 11,
        Product::MissionPack => 14,
    }
}

// ---------------------------------------------------------------------------
// SIBLING-MIRROR: player-state slots, commands, trajectories.
// ---------------------------------------------------------------------------

/// Fixed source arrays with checked access (`PlayerStateSlots`).
#[derive(Debug, Clone)]
pub struct SlotArray {
    values: RefCell<Vec<i32>>,
}

impl SlotArray {
    /// Zeroed slots of the given length.
    pub fn new(length: usize) -> Self {
        Self {
            values: RefCell::new(vec![0; length]),
        }
    }

    /// Slot count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.values.borrow().len()
    }

    /// Whether there are no slots.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.borrow().is_empty()
    }

    /// Checked read.
    #[must_use]
    pub fn get(&self, index: usize) -> i32 {
        let values = self.values.borrow();
        match values.get(index) {
            Some(value) => *value,
            None => panic!("Player state slot {index} outside {}", values.len()),
        }
    }

    /// Checked write.
    pub fn set(&self, index: usize, value: i32) {
        let mut values = self.values.borrow_mut();
        if index >= values.len() {
            panic!("Player state slot {index} outside {}", values.len());
        }
        values[index] = value;
    }

    /// Snapshot copy (`copy`).
    #[must_use]
    pub fn copy(&self) -> Vec<i32> {
        self.values.borrow().clone()
    }
}

/// Team scores and other stores shared across modules by reference.
pub type SharedSlots = Rc<SlotArray>;

/// Authoritative client input command (`UserCommand`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UserCommand {
    /// Server time of the command.
    pub server_time: i32,
    /// View angles.
    pub angles: Vec3,
    /// Button bits.
    pub buttons: i32,
    /// Requested weapon.
    pub weapon: i32,
    /// Forward impulse.
    pub forwardmove: i32,
    /// Side impulse.
    pub rightmove: i32,
    /// Vertical impulse.
    pub upmove: i32,
}

impl Default for UserCommand {
    fn default() -> Self {
        Self {
            server_time: 0,
            angles: vec3(0.0, 0.0, 0.0),
            buttons: 0,
            weapon: weapon::NONE,
            forwardmove: 0,
            rightmove: 0,
            upmove: 0,
        }
    }
}

/// Network trajectory (`Trajectory`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Trajectory {
    /// Trajectory type code.
    pub traj_type: i32,
    /// Start time.
    pub time: i32,
    /// Duration (sine/linear-stop).
    pub duration: i32,
    /// Base position.
    pub base: Vec3,
    /// Delta/velocity.
    pub delta: Vec3,
}

impl Default for Trajectory {
    fn default() -> Self {
        Self {
            traj_type: trajectory_type::STATIONARY,
            time: 0,
            duration: 0,
            base: vec3(0.0, 0.0, 0.0),
            delta: vec3(0.0, 0.0, 0.0),
        }
    }
}

pub(crate) fn traj_seconds(milliseconds: i32) -> f32 {
    milliseconds as f32 * 0.001
}

/// Evaluate a trajectory at a time (`evaluateTrajectory`).
#[must_use]
pub fn evaluate_trajectory(traj: &Trajectory, at_time: i32) -> Vec3 {
    match traj.traj_type {
        trajectory_type::STATIONARY | trajectory_type::INTERPOLATE => vec3(traj.base.x, traj.base.y, traj.base.z),
        trajectory_type::LINEAR => {
            let delta_time = traj_seconds(at_time.wrapping_sub(traj.time));
            add3(traj.base, scale3(traj.delta, delta_time))
        }
        trajectory_type::SINE => {
            let fraction = at_time.wrapping_sub(traj.time) as f32 / traj.duration as f32;
            let phase = (fraction * std::f32::consts::PI * 2.0).sin();
            add3(traj.base, scale3(traj.delta, phase))
        }
        trajectory_type::LINEAR_STOP => {
            let end = traj.time.wrapping_add(traj.duration);
            let time = if at_time > end { end } else { at_time };
            let delta_time = traj_seconds(time.wrapping_sub(traj.time)).max(0.0);
            add3(traj.base, scale3(traj.delta, delta_time))
        }
        trajectory_type::GRAVITY => {
            let delta_time = traj_seconds(at_time.wrapping_sub(traj.time));
            let result = add3(traj.base, scale3(traj.delta, delta_time));
            let fall = 0.5 * 800.0 * delta_time * delta_time;
            vec3(result.x, result.y, result.z - fall)
        }
        other => panic!("BG_EvaluateTrajectory: unknown trType: {other}"),
    }
}

/// Item pickup test (`playerTouchesItem`).
#[must_use]
pub fn player_touches_item(player_origin: Vec3, item_position: &Trajectory, at_time: i32) -> bool {
    let origin = evaluate_trajectory(item_position, at_time);
    let x = player_origin.x - origin.x;
    let y = player_origin.y - origin.y;
    let z = player_origin.z - origin.z;
    !(x > 44.0 || x < -50.0 || y > 36.0 || y < -36.0 || z > 36.0 || z < -36.0)
}

// ---------------------------------------------------------------------------
// SIBLING-MIRROR: core/game-numeric.ts + core/numeric.ts helpers.
// ---------------------------------------------------------------------------

pub(crate) fn number_bytes(text: &str, label: &str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(text.len());
    for ch in text.chars() {
        let code = ch as u32;
        if code > 255 {
            panic!("{label} require byte characters");
        }
        bytes.push(code as u8);
    }
    bytes
}

/// `bg_lib` atoi: wrapping decimal parse (`gameAtoi`).
#[must_use]
pub fn game_atoi(text: &str) -> i32 {
    let bytes = number_bytes(text, "Game numbers");
    let mut offset = 0;
    let byte_at = |offset: usize| -> i32 {
        if offset >= bytes.len() {
            return 0;
        }
        let byte = bytes[offset];
        if byte < 128 {
            byte as i32
        } else {
            byte as i32 - 256
        }
    };
    while byte_at(offset) <= 32 && byte_at(offset) != 0 {
        offset += 1;
    }
    if byte_at(offset) == 0 {
        return 0;
    }
    let mut sign = 1i32;
    let lead = byte_at(offset);
    if lead == 43 || lead == 45 {
        offset += 1;
        sign = if lead == 45 { -1 } else { 1 };
    }
    let mut value = 0i32;
    loop {
        let character = byte_at(offset);
        offset += 1;
        if !(48..=57).contains(&character) {
            break;
        }
        value = value.wrapping_mul(10).wrapping_add(character - 48);
    }
    value.wrapping_mul(sign)
}

/// `bg_lib` atof: decimal prefix only (`gameAtof`).
#[must_use]
pub fn game_atof(text: &str) -> f32 {
    let bytes = number_bytes(text, "Game numbers");
    let mut offset = 0;
    let byte_at = |offset: usize| -> i32 {
        if offset >= bytes.len() {
            return 0;
        }
        let byte = bytes[offset];
        if byte < 128 {
            byte as i32
        } else {
            byte as i32 - 256
        }
    };
    while byte_at(offset) <= 32 && byte_at(offset) != 0 {
        offset += 1;
    }
    if byte_at(offset) == 0 {
        return 0.0;
    }
    let mut sign = 1.0f32;
    let lead = byte_at(offset);
    if lead == 43 || lead == 45 {
        offset += 1;
        sign = if lead == 45 { -1.0 } else { 1.0 };
    }
    let mut value = 0.0f32;
    let mut character = byte_at(offset);
    if byte_at(offset) != 46 {
        loop {
            character = byte_at(offset);
            offset += 1;
            if !(48..=57).contains(&character) {
                break;
            }
            value = value * 10.0 + (character - 48) as f32;
        }
    } else {
        offset += 1;
    }
    if character == 46 {
        let mut fraction = 0.1f32;
        loop {
            character = byte_at(offset);
            offset += 1;
            if !(48..=57).contains(&character) {
                break;
            }
            value += (character - 48) as f32 * fraction;
            fraction *= 0.1;
        }
    }
    value * sign
}

/// Instance-owned `bg_lib` rand/srand and game random (`GameRandom`).
#[derive(Debug)]
pub struct GameRandom {
    seed: Cell<i32>,
}

impl GameRandom {
    /// Fresh generator.
    pub fn new(seed: i32) -> Self {
        Self { seed: Cell::new(seed) }
    }

    /// Current seed.
    #[must_use]
    pub fn seed(&self) -> i32 {
        self.seed.get()
    }

    /// Reset the seed.
    pub fn reset(&self, seed: i32) {
        self.seed.set(seed);
    }

    /// Low-15-bit game rand.
    pub fn rand(&self) -> i32 {
        let next = q_rand(self.seed.get());
        self.seed.set(next);
        next & 0x7fff
    }

    /// `[0, 1]` game random.
    pub fn random(&self) -> f32 {
        self.rand() as f32 / 0x7fff as f32
    }

    /// `[-1, 1]` game random.
    pub fn crandom(&self) -> f32 {
        2.0 * (self.random() - 0.5)
    }
}

/// ASCII case fold (`asciiFold`).
#[must_use]
pub fn ascii_fold(text: &str) -> String {
    text.chars()
        .map(|ch| {
            if ch.is_ascii_uppercase() {
                ch.to_ascii_lowercase()
            } else {
                ch
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// SIBLING-MIRROR: base/game/format.ts printf subset.
// ---------------------------------------------------------------------------

/// Printf argument (`GameFormatArgument`).
#[derive(Debug, Clone, PartialEq)]
pub enum FormatArg {
    /// Signed 32-bit integer (`%d`/`%i`/`%c`).
    Int(i32),
    /// Byte string (`%s`).
    Text(String),
    /// Null string pointer (prints `(null)`).
    Null,
}

impl From<i32> for FormatArg {
    fn from(value: i32) -> Self {
        FormatArg::Int(value)
    }
}

impl From<String> for FormatArg {
    fn from(value: String) -> Self {
        FormatArg::Text(value)
    }
}

impl From<&str> for FormatArg {
    fn from(value: &str) -> Self {
        FormatArg::Text(value.to_string())
    }
}

impl From<Option<String>> for FormatArg {
    fn from(value: Option<String>) -> Self {
        match value {
            Some(text) => FormatArg::Text(text),
            None => FormatArg::Null,
        }
    }
}

/// `Com_sprintf` staging buffer size.
pub const BIG_BUFFER_BYTES: usize = 32_000;

pub(crate) struct FormatOutput {
    value: String,
}

impl FormatOutput {
    fn reserve(&self, bytes: usize) {
        if self.value.len() + bytes >= BIG_BUFFER_BYTES {
            panic!("game format exceeds the 32000-byte Com_sprintf buffer");
        }
    }

    fn append_byte(&mut self, byte: i32) {
        self.reserve(1);
        self.value.push((byte & 255) as u8 as char);
    }

    fn append_bytes(&mut self, value: &str, length: usize) {
        self.reserve(length);
        self.value.push_str(&value.chars().take(length).collect::<String>());
    }

    fn append_repeated(&mut self, byte: i32, count: usize) {
        self.reserve(count);
        for _ in 0..count {
            self.value.push((byte & 255) as u8 as char);
        }
    }

    fn finish(&self, max_bytes: usize) -> String {
        let visible = match self.value.find('\0') {
            Some(nul) => &self.value[..nul],
            None => &self.value[..],
        };
        visible.chars().take(max_bytes - 1).collect()
    }
}

pub(crate) fn format_byte(format: &[u8], index: usize) -> Option<u8> {
    if index >= format.len() {
        return None;
    }
    let byte = format[index];
    if byte == 0 {
        return None;
    }
    Some(byte)
}

pub(crate) fn integer_argument(args: &[FormatArg], index: usize, specifier: char) -> i32 {
    match args.get(index) {
        Some(FormatArg::Int(value)) => *value,
        Some(_) => panic!("argument {index} for %{specifier} must be a number"),
        None => panic!("missing argument {index} for %{specifier}"),
    }
}

pub(crate) fn add_int(output: &mut FormatOutput, value: i32, width: usize, left: bool, zero_pad: bool) {
    let text = format!("{value}");
    let bytes = text.len();
    if !left {
        let padding = width.saturating_sub(bytes);
        output.append_repeated(if zero_pad { 48 } else { 32 }, padding);
    }
    for ch in text.chars() {
        output.append_byte(ch as i32);
    }
    if left {
        let padding = width.saturating_sub(bytes);
        output.append_repeated(if zero_pad { 48 } else { 32 }, padding);
    }
}

pub(crate) fn add_float(output: &mut FormatOutput, value: f32, width: usize, precision: i32) {
    let mut remaining = if value < 0.0 { -value } else { value };
    let integer = remaining.trunc();
    let digits = if precision < 0 { 6 } else { precision };
    if digits > 32 {
        panic!("float precision would overflow AddFloat's 32-byte digit buffer");
    }
    let mut text = if value < 0.0 {
        format!("-{}", integer as i64)
    } else {
        format!("{}", integer as i64)
    };
    if digits != 0 {
        text.push('.');
        for _ in 0..digits {
            remaining -= remaining.trunc();
            remaining *= 10.0;
            let digit = (remaining.trunc() as i32) % 10;
            text.push((48 + digit) as u8 as char);
        }
    }
    let padding = width.saturating_sub(text.len());
    output.append_repeated(32, padding);
    for ch in text.chars() {
        output.append_byte(ch as i32);
    }
}

pub(crate) fn add_string(output: &mut FormatOutput, value: Option<&str>, width: usize, precision: i32) {
    let text = value.unwrap_or("(null)");
    let effective = if value.is_none() || precision < 0 {
        text.len()
    } else {
        precision as usize
    };
    let mut length = 0;
    for ch in text.chars().take(effective) {
        let code = ch as u32;
        if code == 0 {
            break;
        }
        if code > 255 {
            panic!("game format strings must contain byte-valued code units");
        }
        length += 1;
    }
    output.append_bytes(text, length);
    output.append_repeated(32, width.saturating_sub(length));
}

/// QVM `bg_lib.c` printf with `Q_strncpyz` bounds (`gameFormat`).
#[must_use]
pub fn game_format(format: &str, args: &[FormatArg], max_bytes: usize) -> String {
    if max_bytes < 1 {
        panic!("game format destination capacity must be a positive safe integer");
    }
    for ch in format.chars() {
        if ch == '\0' {
            break;
        }
        if ch as u32 > 255 {
            panic!("game format strings must contain byte-valued code units");
        }
    }
    let bytes: Vec<u8> = format.chars().take_while(|ch| *ch != '\0').map(|ch| ch as u8).collect();
    let mut output = FormatOutput { value: String::new() };
    let mut cursor = 0;
    let mut argument_index = 0;
    loop {
        let literal = format_byte(&bytes, cursor);
        let Some(literal) = literal else { break };
        if literal != 37 {
            output.append_byte(literal as i32);
            cursor += 1;
            continue;
        }
        cursor += 1;
        let mut left = false;
        let mut zero_pad = false;
        let mut width: usize = 0;
        let mut precision: i32 = -1;
        loop {
            let Some(specifier) = format_byte(&bytes, cursor) else {
                panic!("unterminated game format specifier");
            };
            cursor += 1;
            if specifier == 45 {
                left = true;
                continue;
            }
            if specifier == 46 {
                let mut parsed = 0i32;
                loop {
                    let digit = format_byte(&bytes, cursor);
                    match digit {
                        Some(d) if (48..=57).contains(&d) => {
                            parsed = parsed.wrapping_mul(10).wrapping_add(d as i32 - 48);
                            cursor += 1;
                        }
                        _ => break,
                    }
                }
                precision = if parsed < 0 { -1 } else { parsed };
                continue;
            }
            if specifier == 48 {
                zero_pad = true;
                continue;
            }
            if (49..=57).contains(&specifier) {
                let mut parsed = 0i32;
                let mut digit = specifier;
                loop {
                    parsed = parsed.wrapping_mul(10).wrapping_add(digit as i32 - 48);
                    let Some(next) = format_byte(&bytes, cursor) else {
                        panic!("unterminated game format specifier");
                    };
                    cursor += 1;
                    digit = next;
                    if !(48..=57).contains(&digit) {
                        break;
                    }
                }
                width = parsed.max(0) as usize;
                cursor -= 1;
                continue;
            }
            if specifier == 37 {
                output.append_byte(37);
                break;
            }
            let spec_char = specifier as char;
            let argument = args.get(argument_index).cloned();
            match specifier {
                100 | 105 => {
                    let value = match argument {
                        Some(FormatArg::Int(value)) => value,
                        Some(_) => panic!("argument {argument_index} for %{spec_char} must be a number"),
                        None => panic!("missing argument {argument_index} for %{spec_char}"),
                    };
                    add_int(&mut output, value, width, left, zero_pad);
                }
                102 => {
                    let value = match argument {
                        Some(FormatArg::Int(value)) => value as f32,
                        Some(_) => panic!("argument {argument_index} for %f must be a number"),
                        None => panic!("missing argument {argument_index} for %f"),
                    };
                    if !value.is_finite() || value.abs() > 2_147_483_647.0 {
                        panic!("argument {argument_index} for %f is outside the source's safe int-cast range");
                    }
                    add_float(&mut output, value, width, precision);
                }
                115 => match argument {
                    Some(FormatArg::Text(text)) => add_string(&mut output, Some(&text), width, precision),
                    Some(FormatArg::Null) => add_string(&mut output, None, width, precision),
                    Some(FormatArg::Int(_)) => {
                        panic!("argument {argument_index} for %s must be a string or null")
                    }
                    None => panic!("missing argument {argument_index} for %s"),
                },
                _ => {
                    let value = integer_argument(&argument.map_or(Vec::new(), |arg| vec![arg]), 0, spec_char);
                    output.append_byte(value);
                }
            }
            argument_index += 1;
            break;
        }
    }
    output.finish(max_bytes)
}

/// [`game_format`] with the default `Com_sprintf` capacity.
#[must_use]
pub fn game_format_default(format: &str, args: &[FormatArg]) -> String {
    game_format(format, args, BIG_BUFFER_BYTES)
}

// ---------------------------------------------------------------------------
// SIBLING-MIRROR: persistence values, spawn variables, item definitions.
// ---------------------------------------------------------------------------

/// Checkpoint value (`unknown` JSON-ish save payloads).
#[derive(Debug, Clone, PartialEq)]
pub enum SaveValue {
    /// Null.
    Null,
    /// Boolean.
    Bool(bool),
    /// Integer.
    Int(i64),
    /// String.
    Str(String),
    /// Array.
    List(Vec<SaveValue>),
    /// Record.
    Map(HashMap<String, SaveValue>),
}

impl SaveValue {
    /// Build a record from pairs.
    pub fn map(pairs: Vec<(&str, SaveValue)>) -> Self {
        SaveValue::Map(pairs.into_iter().map(|(key, value)| (key.to_string(), value)).collect())
    }
}

/// Checkpoint reader (`SaveReader`).
#[derive(Debug, Clone)]
pub struct SaveReader<'a> {
    value: &'a SaveValue,
    path: String,
}

impl<'a> SaveReader<'a> {
    /// Root reader.
    pub fn new(value: &'a SaveValue, path: &str) -> Self {
        Self {
            value,
            path: path.to_string(),
        }
    }

    /// Field reader.
    #[must_use]
    pub fn field(&self, name: &str) -> SaveReader<'a> {
        match self.value {
            SaveValue::Map(map) => match map.get(name) {
                Some(value) => SaveReader {
                    value,
                    path: format!("{}.{}", self.path, name),
                },
                None => self.fail("expected a record"),
            },
            _ => self.fail("expected a record"),
        }
    }

    /// String value.
    #[must_use]
    pub fn string(&self) -> String {
        match self.value {
            SaveValue::Str(value) => value.clone(),
            _ => self.fail("expected a string"),
        }
    }

    /// Boolean value.
    #[must_use]
    pub fn boolean(&self) -> bool {
        match self.value {
            SaveValue::Bool(value) => *value,
            _ => self.fail("expected a boolean"),
        }
    }

    /// Integer value with a minimum.
    #[must_use]
    pub fn integer(&self, minimum: i64) -> i64 {
        match self.value {
            SaveValue::Int(value) if *value >= minimum => *value,
            _ => self.fail("expected an integer in range"),
        }
    }

    /// Value restricted to the given choices.
    #[must_use]
    pub fn choice(&self, choices: &[i64]) -> i64 {
        match self.value {
            SaveValue::Int(value) if choices.contains(value) => *value,
            _ => self.fail("expected a listed choice"),
        }
    }

    /// Array value.
    pub fn list<T>(&self, read: impl Fn(&SaveReader) -> T) -> Vec<T> {
        match self.value {
            SaveValue::List(entries) => entries
                .iter()
                .enumerate()
                .map(|(index, entry)| {
                    read(&SaveReader {
                        value: entry,
                        path: format!("{}[{index}]", self.path),
                    })
                })
                .collect(),
            _ => self.fail("expected an array"),
        }
    }

    /// Nullable value.
    pub fn nullable<T>(&self, read: impl Fn(&SaveReader) -> T) -> Option<T> {
        match self.value {
            SaveValue::Null => None,
            _ => Some(read(self)),
        }
    }

    /// Fail with a save-format error.
    pub fn fail(&self, message: &str) -> ! {
        panic!("save format error at {}: {message}", self.path);
    }
}

/// Spawn variable lookup result (`SpawnValue`).
#[derive(Debug, Clone, PartialEq)]
pub struct SpawnValue<T> {
    /// Whether the key was present.
    pub present: bool,
    /// Parsed value.
    pub value: T,
}

/// Map-entity spawn variables (`SpawnVariables`).
#[derive(Debug, Clone, Default)]
pub struct SpawnVariables {
    /// Raw key/value pairs.
    pub entries: Vec<(String, String)>,
}

impl SpawnVariables {
    /// Build from pairs.
    pub fn new(entries: Vec<(String, String)>) -> Self {
        if entries.len() > 64 {
            panic!("G_ParseSpawnVars: MAX_SPAWN_VARS");
        }
        Self { entries }
    }

    /// Case-insensitive string lookup.
    #[must_use]
    pub fn string(&self, key: &str, default: &str) -> SpawnValue<String> {
        let normalized = ascii_fold(key);
        match self.entries.iter().find(|pair| ascii_fold(&pair.0) == normalized) {
            Some(pair) => SpawnValue {
                present: true,
                value: pair.1.clone(),
            },
            None => SpawnValue {
                present: false,
                value: default.to_string(),
            },
        }
    }

    /// Case-insensitive integer lookup.
    #[must_use]
    pub fn int(&self, key: &str, default: &str) -> SpawnValue<i32> {
        let found = self.string(key, default);
        SpawnValue {
            present: found.present,
            value: game_atoi(&found.value),
        }
    }

    /// Case-insensitive float lookup.
    #[must_use]
    pub fn float(&self, key: &str, default: &str) -> SpawnValue<f32> {
        let found = self.string(key, default);
        SpawnValue {
            present: found.present,
            value: game_atof(&found.value),
        }
    }
}

/// Item definition subset used by team rules (`ItemDefinition`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemDefinition {
    /// Item tag (powerup/weapon code).
    pub tag: i32,
    /// Entity class name.
    pub class_name: String,
    /// Pickup display name.
    pub pickup_name: Option<String>,
}

/// Set a `key\value` info pair, Q3 dialect (`setInfoValue`).
pub fn set_info_value(
    input: &str,
    key_input: &str,
    value_input: &str,
    maximum_length: usize,
    print: &dyn Fn(&str),
) -> String {
    let info = input.to_string();
    let key = key_input.to_string();
    let value = value_input.to_string();
    if info.len() >= maximum_length {
        panic!("Info_SetValueForKey: oversize infostring");
    }
    if key.contains('\\') || value.contains('\\') {
        print("Can't use keys or values with a \\\n");
        return info;
    }
    if key.contains(';') || value.contains(';') {
        print("Can't use keys or values with a semicolon\n");
        return info;
    }
    if key.contains('"') || value.contains('"') {
        print("Can't use keys or values with a \"\n");
        return info;
    }
    let mut result = info.clone();
    let mut cursor = 0;
    while cursor < info.len() {
        let start = cursor;
        if info.as_bytes().get(cursor) == Some(&b'\\') {
            cursor += 1;
        }
        let rest = &info[cursor..];
        let Some(separator) = rest.find('\\') else { break };
        let separator = cursor + separator;
        let after = &info[separator + 1..];
        let end = match after.find('\\') {
            Some(next) => separator + 1 + next,
            None => info.len(),
        };
        if info[cursor..separator] == key {
            result = format!("{}{}", &info[..start], &info[end..]);
            break;
        }
        cursor = end;
    }
    if value.is_empty() {
        return result;
    }
    let mut pair = format!("\\{key}\\{value}");
    if pair.len() >= maximum_length {
        pair = pair.chars().take(maximum_length - 1).collect();
    }
    if pair.len() + result.len() > maximum_length {
        print("Info string length exceeded\n");
        return result;
    }
    format!("{result}{pair}")
}

// ---------------------------------------------------------------------------
// SIBLING-MIRROR: base/game/state.ts entity and client records.
// ---------------------------------------------------------------------------

/// Shared entity handle (aliases its pool slot).
pub type EntityRef = Rc<RefCell<GameEntity>>;

/// Shared client handle (aliases its entity record).
pub type ClientRef = Rc<RefCell<GameClient>>;

/// Damage participant: an entity or a foreign shared actor
/// (`DamageParticipant`).
#[derive(Clone)]
pub enum DamageParticipant {
    /// Pool entity.
    Entity(EntityRef),
    /// Foreign shared actor.
    SharedActor(ActorId),
}

impl DamageParticipant {
    /// Borrow as an entity when applicable.
    #[must_use]
    pub fn as_entity(&self) -> Option<&EntityRef> {
        match self {
            DamageParticipant::Entity(entity) => Some(entity),
            DamageParticipant::SharedActor(_) => None,
        }
    }
}

/// Touch contact placeholder (unused by team rules).
#[derive(Debug, Clone, Copy, Default)]
pub struct TouchContact;

/// Think callback (`EntityThink`).
pub type ThinkCallback = Rc<dyn Fn(&EntityRef)>;

/// Pain callback (`EntityPain`).
pub type PainCallback = Rc<dyn Fn(&EntityRef, &DamageParticipant, f32)>;

/// Touch callback (`EntityTouch`).
pub type TouchCallback = Rc<dyn Fn(&EntityRef, &DamageParticipant, &TouchContact)>;

/// Death callback (`EntityDie`).
pub type DieCallback = Rc<dyn Fn(&EntityRef, &DamageParticipant, &DamageParticipant, i32, i32)>;

/// Entity state (`EntityState`; fields touched by team rules).
#[derive(Debug, Clone, PartialEq)]
pub struct EntityState {
    /// Entity number.
    pub number: i32,
    /// Entity type code.
    pub e_type: i32,
    /// Entity flag bits.
    pub e_flags: i32,
    /// Position trajectory.
    pub pos: Trajectory,
    /// Angle trajectory.
    pub apos: Trajectory,
    /// Current origin.
    pub origin: Vec3,
    /// Current angles.
    pub angles: Vec3,
    /// Secondary angles.
    pub angles2: Vec3,
    /// Other entity number.
    pub other_entity_num: i32,
    /// Ground entity number.
    pub ground_entity_num: i32,
    /// Looping sound index.
    pub loop_sound: i32,
    /// Model index.
    pub modelindex: i32,
    /// Secondary model index.
    pub modelindex2: i32,
    /// Client number.
    pub client_num: i32,
    /// Animation frame.
    pub frame: i32,
    /// Current event.
    pub event: i32,
    /// Event parameter.
    pub event_parm: i32,
    /// Active powerup bits.
    pub powerups: i32,
    /// Current weapon.
    pub weapon: i32,
    /// Legs animation.
    pub legs_anim: i32,
    /// Torso animation.
    pub torso_anim: i32,
    /// Generic counter.
    pub generic1: i32,
}

impl Default for EntityState {
    fn default() -> Self {
        Self {
            number: 0,
            e_type: entity_type::GENERAL,
            e_flags: 0,
            pos: Trajectory::default(),
            apos: Trajectory::default(),
            origin: vec3(0.0, 0.0, 0.0),
            angles: vec3(0.0, 0.0, 0.0),
            angles2: vec3(0.0, 0.0, 0.0),
            other_entity_num: 0,
            ground_entity_num: 0,
            loop_sound: 0,
            modelindex: 0,
            modelindex2: 0,
            client_num: 0,
            frame: 0,
            event: 0,
            event_parm: 0,
            powerups: 0,
            weapon: weapon::NONE,
            legs_anim: 0,
            torso_anim: 0,
            generic1: 0,
        }
    }
}

/// Collision model selector (`EntityCollisionModel`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollisionModel {
    /// Box hull.
    Box,
    /// Capsule hull.
    Capsule,
}

/// Server-side shared entity data (`EntityShared`).
#[derive(Debug, Clone)]
pub struct EntityShared {
    /// Server flags.
    pub sv_flags: i32,
    /// Single-client target.
    pub single_client: i32,
    /// Collision model.
    pub model: CollisionModel,
    /// Contents mask.
    pub contents: i32,
    /// Owner number.
    pub owner_num: i32,
    /// Collision minimums.
    pub mins: Vec3,
    /// Collision maximums.
    pub maxs: Vec3,
    /// Current collision origin.
    pub stored_origin: Vec3,
    origin_view: Rc<RefCell<Option<Vec3>>>,
}

impl EntityShared {
    /// Fresh shared record.
    pub fn new() -> Self {
        Self {
            sv_flags: 0,
            single_client: 0,
            model: CollisionModel::Box,
            contents: 0,
            owner_num: ENTITYNUM_NONE,
            mins: vec3(0.0, 0.0, 0.0),
            maxs: vec3(0.0, 0.0, 0.0),
            stored_origin: vec3(0.0, 0.0, 0.0),
            origin_view: Rc::new(RefCell::new(None)),
        }
    }

    /// Current origin, honoring a temporary origin view.
    #[must_use]
    pub fn current_origin(&self) -> Vec3 {
        self.origin_view.borrow().unwrap_or(self.stored_origin)
    }

    /// Store the current origin.
    pub fn set_current_origin(&mut self, origin: Vec3) {
        if self.origin_view.borrow().is_some() {
            *self.origin_view.borrow_mut() = Some(origin);
        } else {
            self.stored_origin = origin;
        }
    }
}

impl Default for EntityShared {
    fn default() -> Self {
        Self::new()
    }
}

/// Run a closure with a temporary current origin (`withCurrentOrigin`).
pub fn with_entity_origin(entity: &EntityRef, origin: Vec3, run: impl FnOnce()) {
    let view = entity.borrow().r.origin_view.clone();
    *view.borrow_mut() = Some(origin);
    run();
    *view.borrow_mut() = None;
}

/// Classname storage, including client-name borrowing (`GameEntity`).
#[derive(Clone)]
pub enum Classname {
    /// Plain value.
    Value(Option<String>),
    /// Borrowed from a client's netname.
    ClientName(ClientRef),
}

/// Game entity (`GameEntity`; fields touched by team rules).
#[derive(Clone)]
pub struct GameEntity {
    /// Pool slot.
    pub slot: usize,
    /// Actor handle.
    pub actor: ActorId,
    /// Whether the slot is live.
    pub inuse: bool,
    /// Never recycled by `free`.
    pub never_free: bool,
    /// Class name.
    pub classname: Classname,
    /// Entity state.
    pub s: EntityState,
    /// Shared collision data.
    pub r: EntityShared,
    /// Owning client record.
    pub client: Option<ClientRef>,
    /// Health.
    pub health: i32,
    /// Takes damage.
    pub takedamage: bool,
    /// Scratch damage accumulator.
    pub damage: i32,
    /// Entity flag bits.
    pub flags: i32,
    /// Physics object.
    pub physics_object: bool,
    /// Physics bounce fraction.
    pub physics_bounce: i32,
    /// Timestamp.
    pub timestamp: i32,
    /// Next think time.
    pub nextthink: i32,
    /// Think callback.
    pub think: Option<ThinkCallback>,
    /// Touch callback.
    pub touch: Option<TouchCallback>,
    /// Pain callback.
    pub pain: Option<PainCallback>,
    /// Death callback.
    pub die: Option<DieCallback>,
    /// Pain debounce time.
    pub pain_debounce_time: i32,
    /// Last event time.
    pub event_time: i32,
    /// Free after the event expires.
    pub free_after_event: bool,
    /// Last free time.
    pub freetime: i32,
    /// Spawn flags.
    pub spawnflags: i32,
    /// Collision mask.
    pub clipmask: i32,
    /// Water type.
    pub watertype: i32,
    /// Water level.
    pub waterlevel: i32,
    /// Generic counter.
    pub count: i32,
    /// Location message.
    pub message: Option<String>,
    /// Target name reference.
    pub target: Option<String>,
    /// Own targetname.
    pub targetname: Option<String>,
    /// Current enemy.
    pub enemy: Option<EntityRef>,
    /// Activator entity.
    pub activator: Option<EntityRef>,
    /// Next entity in the location chain.
    pub next_train: Option<EntityRef>,
    /// Carried item definition.
    pub item: Option<ItemDefinition>,
    /// Ground actor.
    pub ground: Option<ActorId>,
}

impl GameEntity {
    /// Fresh entity for a slot.
    pub fn new(slot: usize, actor: ActorId) -> Self {
        Self {
            slot,
            actor,
            inuse: false,
            never_free: false,
            classname: Classname::Value(None),
            s: EntityState::default(),
            r: EntityShared::new(),
            client: None,
            health: 0,
            takedamage: false,
            damage: 0,
            flags: 0,
            physics_object: false,
            physics_bounce: 0,
            timestamp: 0,
            nextthink: 0,
            think: None,
            touch: None,
            pain: None,
            die: None,
            pain_debounce_time: 0,
            event_time: 0,
            free_after_event: false,
            freetime: 0,
            spawnflags: 0,
            clipmask: 0,
            watertype: 0,
            waterlevel: 0,
            count: 0,
            message: None,
            target: None,
            targetname: None,
            enemy: None,
            activator: None,
            next_train: None,
            item: None,
            ground: None,
        }
    }

    /// Read the class name.
    #[must_use]
    pub fn classname(&self) -> Option<String> {
        match &self.classname {
            Classname::Value(value) => value.clone(),
            Classname::ClientName(client) => Some(client.borrow().pers.netname.clone()),
        }
    }

    /// Write the class name.
    pub fn set_classname(&mut self, value: Option<String>) {
        self.classname = Classname::Value(value);
    }

    /// Borrow the class name from a client record.
    pub fn bind_client_name(&mut self, client: &ClientRef) {
        self.classname = Classname::ClientName(client.clone());
    }
}

/// Authority-copy mode for `copy_from` (no bindings exist in this mirror,
/// so both modes copy every field).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorityCopy {
    /// Keep authority-owned stores.
    PreserveAuthority,
    /// Replace authority-owned stores.
    ReplaceAuthority,
}

/// Player state (`PlayerState`).
#[derive(Debug, Clone)]
pub struct PlayerState {
    /// Product.
    pub product: Product,
    /// Last command time.
    pub command_time: i32,
    /// Movement type code.
    pub pm_type: i32,
    /// Bob cycle.
    pub bob_cycle: i32,
    /// Movement flag bits.
    pub pm_flags: i32,
    /// Movement timer.
    pub pm_time: i32,
    /// Origin.
    pub origin: Vec3,
    /// Velocity.
    pub velocity: Vec3,
    /// Weapon time.
    pub weapon_time: i32,
    /// Gravity.
    pub gravity: i32,
    /// Speed.
    pub speed: i32,
    /// Delta angles.
    pub delta_angles: Vec3,
    /// Ground entity number.
    pub ground_entity_num: i32,
    /// Legs timer.
    pub legs_timer: i32,
    /// Legs animation.
    pub legs_anim: i32,
    /// Torso timer.
    pub torso_timer: i32,
    /// Torso animation.
    pub torso_anim: i32,
    /// Movement direction.
    pub movement_dir: i32,
    /// Grapple point.
    pub grapple_point: Vec3,
    /// Entity flag bits.
    pub e_flags: i32,
    /// Predictable event sequence.
    pub event_sequence: i32,
    /// Predictable event ring.
    pub events: SlotArray,
    /// Predictable event parameters.
    pub event_parms: SlotArray,
    /// External event.
    pub external_event: i32,
    /// External event parameter.
    pub external_event_parm: i32,
    /// External event time.
    pub external_event_time: i32,
    /// Client number.
    pub client_num: i32,
    /// Current weapon.
    pub weapon: i32,
    /// Weapon state.
    pub weapon_state: i32,
    /// View angles.
    pub viewangles: Vec3,
    /// View height.
    pub viewheight: i32,
    /// Damage event counter.
    pub damage_event: i32,
    /// Damage yaw.
    pub damage_yaw: i32,
    /// Damage pitch.
    pub damage_pitch: i32,
    /// Damage count.
    pub damage_count: i32,
    /// Stat slots.
    pub stats: SlotArray,
    /// Persistent slots.
    pub persistant: SlotArray,
    /// Powerup slots.
    pub powerups: SlotArray,
    /// Ammo slots.
    pub ammo: SlotArray,
    /// Generic counter.
    pub generic1: i32,
    /// Looping sound.
    pub loop_sound: i32,
    /// Jump-pad entity.
    pub jumppad_ent: i32,
    /// Ping.
    pub ping: i32,
    /// Pmove frame count.
    pub pmove_framecount: i32,
    /// Jump-pad frame.
    pub jumppad_frame: i32,
    /// Consumed event sequence.
    pub entity_event_sequence: i32,
}

impl PlayerState {
    /// Source-zero player state.
    pub fn new(product: Product) -> Self {
        Self {
            product,
            command_time: 0,
            pm_type: move_type::NORMAL,
            bob_cycle: 0,
            pm_flags: 0,
            pm_time: 0,
            origin: vec3(0.0, 0.0, 0.0),
            velocity: vec3(0.0, 0.0, 0.0),
            weapon_time: 0,
            gravity: 0,
            speed: 0,
            delta_angles: vec3(0.0, 0.0, 0.0),
            ground_entity_num: 0,
            legs_timer: 0,
            legs_anim: 0,
            torso_timer: 0,
            torso_anim: 0,
            movement_dir: 0,
            grapple_point: vec3(0.0, 0.0, 0.0),
            e_flags: 0,
            event_sequence: 0,
            events: SlotArray::new(2),
            event_parms: SlotArray::new(2),
            external_event: 0,
            external_event_parm: 0,
            external_event_time: 0,
            client_num: 0,
            weapon: weapon::NONE,
            weapon_state: weapon_state::READY,
            viewangles: vec3(0.0, 0.0, 0.0),
            viewheight: 0,
            damage_event: 0,
            damage_yaw: 0,
            damage_pitch: 0,
            damage_count: 0,
            stats: SlotArray::new(16),
            persistant: SlotArray::new(16),
            powerups: SlotArray::new(16),
            ammo: SlotArray::new(16),
            generic1: 0,
            loop_sound: 0,
            jumppad_ent: 0,
            ping: 0,
            pmove_framecount: 0,
            jumppad_frame: 0,
            entity_event_sequence: 0,
        }
    }

    /// Health (`stats[STAT_HEALTH]`).
    #[must_use]
    pub fn health(&self) -> i32 {
        self.stats.get(stat_schema(self.product).health)
    }

    /// Write health.
    pub fn set_health(&mut self, value: i32) {
        let slot = stat_schema(self.product).health;
        self.stats.set(slot, value);
    }

    /// Full copy (`copyFrom`).
    pub fn copy_from(&mut self, source: &PlayerState, _mode: AuthorityCopy) {
        self.product = source.product;
        self.command_time = source.command_time;
        self.pm_type = source.pm_type;
        self.bob_cycle = source.bob_cycle;
        self.pm_flags = source.pm_flags;
        self.pm_time = source.pm_time;
        self.origin = source.origin;
        self.velocity = source.velocity;
        self.weapon_time = source.weapon_time;
        self.gravity = source.gravity;
        self.speed = source.speed;
        self.delta_angles = source.delta_angles;
        self.ground_entity_num = source.ground_entity_num;
        self.legs_timer = source.legs_timer;
        self.legs_anim = source.legs_anim;
        self.torso_timer = source.torso_timer;
        self.torso_anim = source.torso_anim;
        self.movement_dir = source.movement_dir;
        self.grapple_point = source.grapple_point;
        self.e_flags = source.e_flags;
        self.event_sequence = source.event_sequence;
        copy_slots(&self.events, &source.events);
        copy_slots(&self.event_parms, &source.event_parms);
        self.external_event = source.external_event;
        self.external_event_parm = source.external_event_parm;
        self.external_event_time = source.external_event_time;
        self.client_num = source.client_num;
        self.weapon = source.weapon;
        self.weapon_state = source.weapon_state;
        self.viewangles = source.viewangles;
        self.viewheight = source.viewheight;
        self.damage_event = source.damage_event;
        self.damage_yaw = source.damage_yaw;
        self.damage_pitch = source.damage_pitch;
        self.damage_count = source.damage_count;
        copy_slots(&self.stats, &source.stats);
        copy_slots(&self.persistant, &source.persistant);
        copy_slots(&self.powerups, &source.powerups);
        copy_slots(&self.ammo, &source.ammo);
        self.generic1 = source.generic1;
        self.loop_sound = source.loop_sound;
        self.jumppad_ent = source.jumppad_ent;
        self.ping = source.ping;
        self.pmove_framecount = source.pmove_framecount;
        self.jumppad_frame = source.jumppad_frame;
        self.entity_event_sequence = source.entity_event_sequence;
    }
}

pub(crate) fn copy_slots(target: &SlotArray, source: &SlotArray) {
    for index in 0..target.len() {
        target.set(index, source.get(index));
    }
}

/// Persistent team state (`PlayerTeamState`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PlayerTeamState {
    /// Team state code.
    pub state: i32,
    /// Location marker.
    pub location: i32,
    /// Captures.
    pub captures: i32,
    /// Base defenses.
    pub base_defense: i32,
    /// Carrier defenses.
    pub carrier_defense: i32,
    /// Flag recoveries.
    pub flag_recovery: i32,
    /// Carrier frags.
    pub frag_carrier: i32,
    /// Assists.
    pub assists: i32,
    /// Last time this client hurt a carrier.
    pub last_hurt_carrier: f32,
    /// Last flag-return time.
    pub last_returned_flag: f32,
    /// Flag-hold start time.
    pub flag_since: f32,
    /// Last carrier-frag time.
    pub last_fragged_carrier: f32,
}

/// Session record (`ClientSession`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientSession {
    /// Session team code.
    pub session_team: i32,
    /// Spectator start time.
    pub spectator_time: i32,
    /// Spectator state code.
    pub spectator_state: i32,
    /// Followed client.
    pub spectator_client: i32,
    /// Wins.
    pub wins: i32,
    /// Losses.
    pub losses: i32,
    /// Team leader flag.
    pub team_leader: i32,
}

impl Default for ClientSession {
    fn default() -> Self {
        Self {
            session_team: team::FREE,
            spectator_time: 0,
            spectator_state: spectator_state::NOT,
            spectator_client: 0,
            wins: 0,
            losses: 0,
            team_leader: 0,
        }
    }
}

/// Persistent client record (`ClientPersistant`).
#[derive(Debug, Clone, PartialEq)]
pub struct ClientPersistant {
    /// Connection state code.
    pub connected: i32,
    /// Latest command.
    pub cmd: UserCommand,
    /// Local client.
    pub local_client: bool,
    /// Initial spawn consumed.
    pub initial_spawn: bool,
    /// Predict item pickup.
    pub predict_item_pickup: bool,
    /// Fixed pmove.
    pub pmove_fixed: bool,
    /// Net name.
    pub netname: String,
    /// Maximum health.
    pub max_health: i32,
    /// Enter time.
    pub enter_time: i32,
    /// Team state.
    pub team_state: PlayerTeamState,
    /// Votes called.
    pub vote_count: i32,
    /// Team votes called.
    pub team_vote_count: i32,
    /// Receives team info.
    pub team_info: bool,
}

impl Default for ClientPersistant {
    fn default() -> Self {
        Self {
            connected: connection_state::DISCONNECTED,
            cmd: UserCommand::default(),
            local_client: false,
            initial_spawn: false,
            predict_item_pickup: false,
            pmove_fixed: false,
            netname: String::new(),
            max_health: 0,
            enter_time: 0,
            team_state: PlayerTeamState::default(),
            vote_count: 0,
            team_vote_count: 0,
            team_info: false,
        }
    }
}

/// Game client (`GameClient`; fields touched by team rules).
#[derive(Clone)]
pub struct GameClient {
    /// Player state.
    pub ps: PlayerState,
    /// Persistent record.
    pub pers: ClientPersistant,
    /// Session record.
    pub sess: ClientSession,
    /// Ready to exit intermission.
    pub ready_to_exit: bool,
    /// Noclip cheat.
    pub noclip: bool,
    /// Last command time.
    pub last_cmd_time: i32,
    /// Buttons.
    pub buttons: i32,
    /// Previous buttons.
    pub old_buttons: i32,
    /// Latched buttons.
    pub latched_buttons: i32,
    /// Previous origin.
    pub old_origin: Vec3,
    /// Absorbed armor damage.
    pub damage_armor: i32,
    /// Absorbed health damage.
    pub damage_blood: i32,
    /// Damage knockback.
    pub damage_knockback: i32,
    /// Damage direction source.
    pub damage_from: Vec3,
    /// Damage came from the world.
    pub damage_from_world: bool,
    /// Accuracy shots.
    pub accuracy_shots: i32,
    /// Accuracy hits.
    pub accuracy_hits: i32,
    /// Last killed client.
    pub last_killed_client: i32,
    /// Last hurt means of death.
    pub last_hurt_mod: i32,
    /// Respawn time.
    pub respawn_time: i32,
    /// Inactivity deadline.
    pub inactivity_time: i32,
    /// Inactivity warned.
    pub inactivity_warning: bool,
    /// Reward expiry.
    pub reward_time: i32,
    /// Air deadline.
    pub air_out_time: i32,
    /// Fire held.
    pub fire_held: bool,
    /// Grapple hook.
    pub hook: Option<EntityRef>,
    /// Team-switch throttle.
    pub switch_team_time: i32,
    /// Timer residual.
    pub time_residual: i32,
    /// Portal id.
    pub portal_id: i32,
    /// Ammo-regeneration timers.
    pub ammo_times: SlotArray,
    /// Invulnerability deadline.
    pub invulnerability_time: i32,
}

impl GameClient {
    /// Source-zero client.
    pub fn new(product: Product) -> Self {
        Self {
            ps: PlayerState::new(product),
            pers: ClientPersistant::default(),
            sess: ClientSession::default(),
            ready_to_exit: false,
            noclip: false,
            last_cmd_time: 0,
            buttons: 0,
            old_buttons: 0,
            latched_buttons: 0,
            old_origin: vec3(0.0, 0.0, 0.0),
            damage_armor: 0,
            damage_blood: 0,
            damage_knockback: 0,
            damage_from: vec3(0.0, 0.0, 0.0),
            damage_from_world: false,
            accuracy_shots: 0,
            accuracy_hits: 0,
            last_killed_client: 0,
            last_hurt_mod: 0,
            respawn_time: 0,
            inactivity_time: 0,
            inactivity_warning: false,
            reward_time: 0,
            air_out_time: 0,
            fire_held: false,
            hook: None,
            switch_team_time: 0,
            time_residual: 0,
            portal_id: 0,
            ammo_times: SlotArray::new(weapon_count(product)),
            invulnerability_time: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// SIBLING-MIRROR: base/game/entities.ts pool core + utilities + ground.
// ---------------------------------------------------------------------------

/// Pool engine hooks (`EntityPoolOptions` subset).
pub struct PoolHooks {
    /// Current game time.
    pub time: Rc<dyn Fn() -> i32>,
    /// Map start time.
    pub map_start_time: i32,
    /// Link an entity.
    pub link: Rc<dyn Fn(&EntityRef)>,
    /// Unlink an entity.
    pub unlink: Rc<dyn Fn(&EntityRef)>,
    /// Engine print.
    pub print: Rc<dyn Fn(&str)>,
}

/// Save-callback registries (`pool.callbacks`).
#[derive(Default)]
pub struct CallbackRegistries {
    think: RefCell<HashMap<String, ThinkCallback>>,
    pain: RefCell<HashMap<String, PainCallback>>,
    die: RefCell<HashMap<String, DieCallback>>,
    touch: RefCell<HashMap<String, TouchCallback>>,
}

impl CallbackRegistries {
    /// Register a think callback.
    pub fn intern_think(&self, name: &str, callback: ThinkCallback) {
        self.think.borrow_mut().insert(name.to_string(), callback);
    }

    /// Resolve a think callback.
    #[must_use]
    pub fn resolve_think(&self, name: &str) -> ThinkCallback {
        self.think
            .borrow()
            .get(name)
            .cloned()
            .unwrap_or_else(|| panic!("unknown think callback {name}"))
    }

    /// Register a pain callback.
    pub fn intern_pain(&self, name: &str, callback: PainCallback) {
        self.pain.borrow_mut().insert(name.to_string(), callback);
    }

    /// Resolve a pain callback.
    #[must_use]
    pub fn resolve_pain(&self, name: &str) -> PainCallback {
        self.pain
            .borrow()
            .get(name)
            .cloned()
            .unwrap_or_else(|| panic!("unknown pain callback {name}"))
    }

    /// Register a death callback.
    pub fn intern_die(&self, name: &str, callback: DieCallback) {
        self.die.borrow_mut().insert(name.to_string(), callback);
    }

    /// Resolve a death callback.
    #[must_use]
    pub fn resolve_die(&self, name: &str) -> DieCallback {
        self.die
            .borrow()
            .get(name)
            .cloned()
            .unwrap_or_else(|| panic!("unknown die callback {name}"))
    }

    /// Register a touch callback.
    pub fn intern_touch(&self, name: &str, callback: TouchCallback) {
        self.touch.borrow_mut().insert(name.to_string(), callback);
    }

    /// Resolve a touch callback.
    #[must_use]
    pub fn resolve_touch(&self, name: &str) -> TouchCallback {
        self.touch
            .borrow()
            .get(name)
            .cloned()
            .unwrap_or_else(|| panic!("unknown touch callback {name}"))
    }
}

/// Rankings reporting (`pool.rankings`).
pub trait RankingsHost {
    /// Report a holdable use.
    fn use_holdable(&self, slot: usize, holdable: i32);
    /// Report a capture.
    fn capture(&self, slot: usize);
    /// Report a powerup pickup.
    fn pickup_powerup(&self, slot: usize, powerup: i32);
}

/// Shared entity pool.
pub type PoolRef = Rc<EntityPool>;

/// Loaded-module entity/client storage (`EntityPool`).
pub struct EntityPool {
    entities: RefCell<Vec<EntityRef>>,
    num_entities: Cell<usize>,
    clients: Vec<ClientRef>,
    max_clients: usize,
    product: Product,
    hooks: PoolHooks,
    /// Save-callback registries.
    pub callbacks: CallbackRegistries,
    /// Rankings reporting.
    pub rankings: Box<dyn RankingsHost>,
}

impl EntityPool {
    /// Fresh pool with linked client records.
    pub fn new(product: Product, max_clients: usize, hooks: PoolHooks, rankings: Box<dyn RankingsHost>) -> Self {
        if !(1..=MAX_CLIENTS).contains(&max_clients) {
            panic!("Entity pool maxClients outside 1..64");
        }
        let identity =
            IdentityOwner::create("q3-team-arena").unwrap_or_else(|_| panic!("Entity pool identity must have a name"));
        let entities: Vec<EntityRef> = (0..MAX_GENTITIES)
            .map(|slot| Rc::new(RefCell::new(GameEntity::new(slot, identity.actor(slot as u32, 0)))))
            .collect();
        let clients: Vec<ClientRef> = (0..MAX_CLIENTS)
            .map(|_| Rc::new(RefCell::new(GameClient::new(product))))
            .collect();
        for index in 0..max_clients {
            entities[index].borrow_mut().client = Some(clients[index].clone());
        }
        Self {
            entities: RefCell::new(entities),
            num_entities: Cell::new(MAX_CLIENTS),
            clients,
            max_clients,
            product,
            hooks,
            callbacks: CallbackRegistries::default(),
            rankings,
        }
    }

    /// Pool product.
    #[must_use]
    pub fn product(&self) -> Product {
        self.product
    }

    /// Configured client count.
    #[must_use]
    pub fn max_clients(&self) -> usize {
        self.max_clients
    }

    /// Open entity high-water mark.
    #[must_use]
    pub fn num_entities(&self) -> usize {
        self.num_entities.get()
    }

    /// All client records.
    #[must_use]
    pub fn clients(&self) -> &[ClientRef] {
        &self.clients
    }

    /// Optional slot lookup.
    #[must_use]
    pub fn get(&self, slot: usize) -> Option<EntityRef> {
        self.entities.borrow().get(slot).cloned()
    }

    /// Checked slot lookup.
    #[must_use]
    pub fn at(&self, slot: usize) -> EntityRef {
        match self.get(slot) {
            Some(entity) => entity,
            None => panic!("Game entity {slot} is unavailable"),
        }
    }

    /// Checked client lookup.
    #[must_use]
    pub fn client_at(&self, slot: usize) -> ClientRef {
        match self.clients.get(slot) {
            Some(client) => client.clone(),
            None => panic!("Game client {slot} is unavailable"),
        }
    }

    /// Pool index of a client record.
    #[must_use]
    pub fn client_index(&self, client: &ClientRef) -> Option<usize> {
        self.clients.iter().position(|entry| Rc::ptr_eq(entry, client))
    }

    /// Whether an entity belongs to this pool.
    fn owned(&self, entity: &EntityRef) {
        match self.get(entity.borrow().slot) {
            Some(owned) if Rc::ptr_eq(&owned, entity) => {}
            _ => panic!("Game entity does not belong to this pool"),
        }
    }

    fn initialize_slot(&self, slot: usize) -> EntityRef {
        let entity = self.at(slot);
        {
            let mut body = entity.borrow_mut();
            let actor = body.actor.clone();
            *body = GameEntity::new(slot, actor);
            body.inuse = true;
            body.client = if slot < self.max_clients {
                Some(self.client_at(slot))
            } else {
                None
            };
        }
        init_game_entity(&entity);
        entity
    }

    /// Allocate an entity (`spawn`).
    pub fn spawn(&self) -> EntityRef {
        let now = (self.hooks.time)();
        for index in MAX_CLIENTS..self.num_entities.get() {
            let entity = self.at(index);
            let body = entity.borrow();
            if body.inuse {
                continue;
            }
            if body.freetime > self.hooks.map_start_time.wrapping_add(2000) && now.wrapping_sub(body.freetime) < 1000 {
                continue;
            }
            drop(body);
            return self.initialize_slot(index);
        }
        if self.num_entities.get() == ENTITYNUM_WORLD {
            for index in 0..MAX_GENTITIES {
                let name = self.at(index).borrow().classname();
                (self.hooks.print)(&game_format(
                    "%4i: %s\n",
                    &[FormatArg::Int(index as i32), FormatArg::from(name)],
                    BIG_BUFFER_BYTES,
                ));
            }
            panic!("G_Spawn: no free entities");
        }
        let slot = self.num_entities.get();
        self.num_entities.set(slot + 1);
        self.initialize_slot(slot)
    }

    /// Release an entity (`free`).
    pub fn free(&self, entity: &EntityRef) {
        self.owned(entity);
        (self.hooks.unlink)(entity);
        if entity.borrow().never_free {
            return;
        }
        let now = (self.hooks.time)();
        {
            let mut body = entity.borrow_mut();
            let slot = body.slot;
            let actor = body.actor.clone();
            *body = GameEntity::new(slot, actor);
            body.set_classname(Some("freed".to_string()));
            body.freetime = now;
        }
    }

    /// Spawn a temporary event entity (`tempEntity`).
    pub fn temp_entity(&self, origin: Vec3, event: i32) -> EntityRef {
        let entity = self.spawn();
        {
            let mut body = entity.borrow_mut();
            body.s.e_type = entity_type::EVENTS + event;
            body.set_classname(Some("tempEntity".to_string()));
            body.event_time = (self.hooks.time)();
            body.free_after_event = true;
        }
        set_origin(
            &entity,
            vec3(
                qvm_float_to_int(origin.x) as f32,
                qvm_float_to_int(origin.y) as f32,
                qvm_float_to_int(origin.z) as f32,
            ),
        );
        (self.hooks.link)(&entity);
        entity
    }

    /// Publish an entity event (`addEvent`).
    pub fn add_event(&self, entity: &EntityRef, event: i32, parameter: i32) {
        self.owned(entity);
        if event == 0 {
            let number = entity.borrow().s.number;
            (self.hooks.print)(&game_format(
                "G_AddEvent: zero event added for entity %i\n",
                &[FormatArg::Int(number)],
                BIG_BUFFER_BYTES,
            ));
            return;
        }
        let now = (self.hooks.time)();
        let client = entity.borrow().client.clone();
        match client {
            Some(client) => {
                let mut record = client.borrow_mut();
                let bits = ((record.ps.external_event & EV_EVENT_BITS) + EV_EVENT_BIT1) & EV_EVENT_BITS;
                record.ps.external_event = event | bits;
                record.ps.external_event_parm = parameter;
                record.ps.external_event_time = now;
            }
            None => {
                let mut body = entity.borrow_mut();
                let bits = ((body.s.event & EV_EVENT_BITS) + EV_EVENT_BIT1) & EV_EVENT_BITS;
                body.s.event = event | bits;
                body.s.event_parm = parameter;
            }
        }
        entity.borrow_mut().event_time = now;
    }

    /// Activate a client slot.
    pub fn activate_client(&self, slot: usize) -> EntityRef {
        let entity = self.at(slot);
        entity.borrow_mut().inuse = true;
        entity.borrow_mut().client = Some(self.client_at(slot));
        entity
    }

    /// Deactivate a client slot. The client record stays linked: team rules
    /// assume every client slot always resolves a client.
    pub fn deactivate_client(&self, slot: usize) {
        self.at(slot).borrow_mut().inuse = false;
    }

    /// Find an entity by actor handle.
    #[must_use]
    pub fn native_by_actor(&self, actor: &ActorId) -> Option<EntityRef> {
        self.entities
            .borrow()
            .iter()
            .find(|entity| entity.borrow().actor == *actor)
            .cloned()
    }

    /// Format a vector (`utilities.vtos`).
    #[must_use]
    pub fn vtos(&self, vector: Vec3) -> String {
        let value = game_format(
            "(%i %i %i)",
            &[
                FormatArg::Int(qvm_float_to_int(vector.x)),
                FormatArg::Int(qvm_float_to_int(vector.y)),
                FormatArg::Int(qvm_float_to_int(vector.z)),
            ],
            BIG_BUFFER_BYTES,
        );
        if value.len() >= 32 {
            (self.hooks.print)(&game_format(
                "Com_sprintf: overflow of %i in %i\n",
                &[FormatArg::Int(value.len() as i32), FormatArg::Int(32)],
                BIG_BUFFER_BYTES,
            ));
        }
        value.chars().take(31).collect()
    }
}

/// Reset per-spawn entity fields (`initGameEntity`).
pub fn init_game_entity(entity: &EntityRef) {
    let mut body = entity.borrow_mut();
    body.set_classname(Some("noclass".to_string()));
    body.s.number = body.slot as i32;
    body.r.owner_num = ENTITYNUM_NONE;
}

/// Store a fixed origin trajectory (`setOrigin`).
pub fn set_origin(entity: &EntityRef, origin: Vec3) {
    let mut body = entity.borrow_mut();
    body.s.pos = Trajectory {
        traj_type: trajectory_type::STATIONARY,
        time: 0,
        duration: 0,
        base: vec3(origin.x, origin.y, origin.z),
        delta: vec3(0.0, 0.0, 0.0),
    };
    body.r.set_current_origin(vec3(origin.x, origin.y, origin.z));
}

/// Entity string fields searchable by [`find_entity`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntityStringField {
    /// Class name.
    Classname,
    /// Targetname.
    Targetname,
}

/// Case-insensitive entity search (`findEntity`).
#[must_use]
pub fn find_entity(
    pool: &EntityPool,
    after: Option<&EntityRef>,
    field: EntityStringField,
    value: Option<&str>,
) -> Option<EntityRef> {
    let wanted = value?;
    let folded = ascii_fold(wanted);
    let start = after.map_or(0, |entity| entity.borrow().slot + 1);
    for index in start..pool.num_entities() {
        let entity = pool.at(index);
        let body = entity.borrow();
        if !body.inuse {
            continue;
        }
        let current = match field {
            EntityStringField::Classname => body.classname(),
            EntityStringField::Targetname => body.targetname.clone(),
        };
        match current {
            Some(current) if ascii_fold(&current) == folded => return Some(entity.clone()),
            _ => {}
        }
    }
    None
}

/// Pick a random target by name (`pickTarget`).
pub fn pick_target(
    pool: &EntityPool,
    random_int: &dyn Fn() -> i32,
    warn: &dyn Fn(&str),
    target_name: Option<&str>,
) -> Option<EntityRef> {
    let Some(name) = target_name else {
        warn("G_PickTarget called with NULL targetname\n");
        return None;
    };
    let mut choices = Vec::new();
    let mut found: Option<EntityRef> = None;
    while choices.len() < 32 {
        found = find_entity(pool, found.as_ref(), EntityStringField::Targetname, Some(name));
        match found.clone() {
            Some(entity) => choices.push(entity),
            None => break,
        }
    }
    if choices.is_empty() {
        warn(&format!("G_PickTarget: target {name} not found\n"));
        return None;
    }
    let random = random_int();
    if random < 0 {
        panic!("Game rand must return a nonnegative integer");
    }
    let choice = choices.get((random as usize) % choices.len()).cloned();
    match choice {
        Some(entity) => Some(entity),
        None => panic!("Target selection index invariant"),
    }
}

/// Resolve an actor ground reference to an entity number.
#[must_use]
pub fn ground_number(ground: Option<&ActorId>, pool: &EntityPool) -> i32 {
    match ground {
        None => ENTITYNUM_NONE,
        Some(actor) => pool
            .native_by_actor(actor)
            .map_or(ENTITYNUM_NONE, |entity| entity.borrow().slot as i32),
    }
}

/// Write an entity's ground reference (`writeGround`).
pub fn write_ground(entity: &EntityRef, ground: Option<ActorId>, pool: &EntityPool) {
    let number = ground_number(ground.as_ref(), pool);
    let mut body = entity.borrow_mut();
    body.ground = ground;
    body.s.ground_entity_num = number;
}

/// Trace-hit ground reference (`traceGround`).
pub fn trace_ground(entity: &EntityRef, hit: &TraceHit, pool: &EntityPool) {
    let ground = match hit {
        TraceHit::Actor(actor) => Some(actor.clone()),
        TraceHit::World => Some(pool.at(ENTITYNUM_WORLD).borrow().actor.clone()),
        TraceHit::None => None,
    };
    write_ground(entity, ground, pool);
}

/// Resolve a checkpoint entity reference (`readModuleEntity`).
#[must_use]
pub fn read_module_entity(reader: &SaveReader, pool: &EntityPool) -> EntityRef {
    let slot = reader.integer(0);
    if slot > 1023 {
        reader.fail("module source entity slot exceeds table extent");
    }
    match pool.get(slot as usize) {
        Some(entity) => entity,
        None => reader.fail("module references an absent source entity slot"),
    }
}

pub(crate) fn snap_component(component: f32) -> f32 {
    if (-2_147_483_648.0..2_147_483_648.0).contains(&component) {
        component.trunc()
    } else {
        -2_147_483_648.0
    }
}

pub(crate) fn copy_position(value: Vec3, snap: bool) -> Vec3 {
    if snap {
        vec3(
            snap_component(value.x),
            snap_component(value.y),
            snap_component(value.z),
        )
    } else {
        value
    }
}

pub(crate) fn convert_player_state(
    ps: &mut PlayerState,
    state: &mut EntityState,
    snap: bool,
    extrapolation_time: Option<i32>,
) {
    state.e_type =
        if ps.pm_type == move_type::INTERMISSION || ps.pm_type == move_type::SPECTATOR || ps.health() <= GIB_HEALTH {
            entity_type::INVISIBLE
        } else {
            entity_type::PLAYER
        };
    state.number = ps.client_num;
    state.pos = Trajectory {
        traj_type: if extrapolation_time.is_none() {
            trajectory_type::INTERPOLATE
        } else {
            trajectory_type::LINEAR_STOP
        },
        base: copy_position(ps.origin, snap),
        delta: ps.velocity,
        time: extrapolation_time.unwrap_or(state.pos.time),
        duration: extrapolation_time.map_or(state.pos.duration, |_| 50),
    };
    state.apos.traj_type = trajectory_type::INTERPOLATE;
    state.apos.base = copy_position(ps.viewangles, snap);
    state.angles2.y = ps.movement_dir as f32;
    state.legs_anim = ps.legs_anim;
    state.torso_anim = ps.torso_anim;
    state.client_num = ps.client_num;
    state.e_flags = if ps.health() <= 0 {
        ps.e_flags | 1
    } else {
        ps.e_flags & !1
    };
    if ps.external_event != 0 {
        state.event = ps.external_event;
        state.event_parm = ps.external_event_parm;
    } else if ps.entity_event_sequence < ps.event_sequence {
        let oldest = ps.event_sequence.wrapping_sub(2);
        if ps.entity_event_sequence < oldest {
            ps.entity_event_sequence = oldest;
        }
        let slot = (ps.entity_event_sequence & 1) as usize;
        state.event = ps.events.get(slot) | ((ps.entity_event_sequence & 3) << 8);
        state.event_parm = ps.event_parms.get(slot);
        ps.entity_event_sequence = ps.entity_event_sequence.wrapping_add(1);
    }
    state.weapon = ps.weapon;
    state.ground_entity_num = ps.ground_entity_num;
    state.powerups = 0;
    for index in 0..ps.powerups.len() {
        if ps.powerups.get(index) != 0 {
            state.powerups |= 1 << index;
        }
    }
    state.loop_sound = ps.loop_sound;
    state.generic1 = ps.generic1;
}

/// Map a player state onto an entity state (`playerStateToEntityState`).
pub fn player_state_to_entity_state(ps: &mut PlayerState, state: &mut EntityState, snap: bool) {
    convert_player_state(ps, state, snap, None);
}

/// Map with linear extrapolation (`playerStateToEntityStateExtraPolate`).
pub fn player_state_to_entity_state_extrapolate(ps: &mut PlayerState, state: &mut EntityState, time: i32, snap: bool) {
    convert_player_state(ps, state, snap, Some(time));
}

// ---------------------------------------------------------------------------
// Host traits: engine services the team rules consume.
// ---------------------------------------------------------------------------

/// Trace shape (`TraceShape`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TraceShape {
    /// Point trace.
    Point,
    /// Box trace.
    Box {
        /// Minimums.
        mins: Vec3,
        /// Maximums.
        maxs: Vec3,
    },
    /// Capsule trace.
    Capsule {
        /// Minimums.
        mins: Vec3,
        /// Maximums.
        maxs: Vec3,
    },
}

/// Trace solidity (`solidity`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceSolidity {
    /// Clear path.
    Clear,
    /// Starts inside solid.
    StartSolid,
    /// Entirely inside solid.
    AllSolid,
}

/// Trace hit record (`hit`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TraceHit {
    /// No hit.
    None,
    /// World hit.
    World,
    /// Actor hit.
    Actor(ActorId),
}

/// Actor trace query (`ActorTraceQuery`).
#[derive(Debug, Clone, PartialEq)]
pub struct ActorTraceQuery {
    /// Start point.
    pub start: Vec3,
    /// End point.
    pub end: Vec3,
    /// Trace shape.
    pub shape: TraceShape,
    /// Actor to ignore.
    pub pass_actor: Option<ActorId>,
    /// Contents mask.
    pub mask: i32,
}

/// Actor trace result (`ActorTraceResult`).
#[derive(Debug, Clone, PartialEq)]
pub struct ActorTraceResult {
    /// Stopped end point.
    pub end: Vec3,
    /// Path solidity.
    pub solidity: TraceSolidity,
    /// Hit record.
    pub hit: TraceHit,
}

/// Link state (`LinkState`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LinkState {
    /// Absolute bounds.
    pub absbounds: Bounds,
    /// Whether linked.
    pub linked: bool,
    /// Link count.
    pub linkcount: i32,
}

/// Server world plus spatial queries (`ServerWorld` + `ActorSpatialQueries`).
pub trait Q3World {
    /// Link an entity.
    fn link(&self, entity: &EntityRef);
    /// Unlink an entity number.
    fn unlink(&self, number: i32);
    /// Link state for an entity number.
    fn link_state(&self, number: i32) -> Option<LinkState>;
    /// Contents at a point.
    fn point_contents(&self, point: Vec3, pass_entity: i32) -> i32;
    /// Trace against actors.
    fn trace_actor(&self, query: &ActorTraceQuery) -> ActorTraceResult;
    /// Actors overlapping bounds.
    fn area_actors(&self, bounds: &Bounds, maximum: usize) -> Vec<ActorId>;
    /// Bounds/actor contact test.
    fn contact_actor(&self, bounds: &Bounds, actor: &ActorId) -> bool;
}

/// Shared world handle.
pub type WorldRef = Rc<dyn Q3World>;

/// Combat services used by team rules (`CombatContext` subset).
pub trait Combat {
    /// Product.
    fn product(&self) -> Product;
    /// Current time.
    fn time(&self) -> i32;
    /// Game type code.
    fn game_type(&self) -> i32;
    /// Queued intermission time.
    fn intermission_queued(&self) -> i32;
    /// Entity pool.
    fn pool(&self) -> PoolRef;
    /// Apply damage (`damage`).
    #[allow(clippy::too_many_arguments)]
    fn damage(
        &self,
        target: &EntityRef,
        inflictor: Option<&DamageParticipant>,
        attacker: Option<&DamageParticipant>,
        direction: Option<Vec3>,
        point: Option<Vec3>,
        amount: i32,
        flags: i32,
        method: i32,
    );
}

/// Shared combat handle.
pub type CombatRef = Rc<dyn Combat>;

/// Weapon services (`WeaponRuntime` subset).
pub trait WeaponHost {
    /// Fire the current weapon.
    fn fire(&self, entity: &EntityRef);
    /// Start a kamikaze countdown.
    fn start_kamikaze(&self, entity: &EntityRef);
}

/// Personal-portal services (`PersonalPortalRuntime` subset).
pub trait PortalHost {
    /// Drop a portal source.
    fn drop_portal_source(&self, entity: &EntityRef);
    /// Drop a portal destination.
    fn drop_portal_destination(&self, entity: &EntityRef);
}

/// Item-drop services (`DropItemContext`).
pub trait DropHost {
    /// Drop an item near an entity.
    fn drop_item(&self, entity: &EntityRef, item: &ItemDefinition, angle: i32) -> EntityRef;
}

/// Death services (`DeathRuntime` subset).
pub trait DeathHost {
    /// Kill a player.
    fn player_die(
        &self,
        target: &EntityRef,
        inflictor: Option<&DamageParticipant>,
        attacker: Option<&DamageParticipant>,
        damage: i32,
        method: i32,
    );
    /// Toss a client's items.
    fn toss_client_items(&self, entity: &EntityRef);
    /// Toss persistent powerups.
    fn toss_client_persistant_powerups(&self, entity: &EntityRef);
    /// Toss harvester cubes.
    fn toss_client_cubes(&self, entity: &EntityRef);
}

/// Item services (`items.ts` + `ItemLifecycleContext` subsets).
pub trait ItemHost {
    /// Item by product and index.
    fn item_at(&self, product: Product, index: usize) -> ItemDefinition;
    /// Item by pickup name.
    fn find_item(&self, product: Product, name: &str) -> Option<ItemDefinition>;
    /// Item carrying a powerup.
    fn find_item_for_powerup(&self, product: Product, powerup: i32) -> Option<ItemDefinition>;
    /// Spawn an item entity (`spawnItem`).
    fn spawn_item(&self, entity: &EntityRef, item: &ItemDefinition, vars: &SpawnVariables, disabled: bool);
    /// Finish spawning an item (`finishSpawningItem`).
    fn finish_spawning_item(&self, entity: &EntityRef);
    /// Touch an item (`touchItem`).
    fn touch_item(&self, entity: &EntityRef, other: &DamageParticipant, contact: &TouchContact);
}

/// Target services (`TargetUseContext` subset).
pub trait TargetsHost {
    /// Fire an entity's targets (`useTargets`).
    fn use_targets(&self, used: Option<&EntityRef>, activator: Option<&DamageParticipant>);
}

/// Teleport services (`TeleportContext`).
pub trait TeleportHost {
    /// Teleport a player (`teleportPlayer`).
    fn teleport_player(&self, entity: &EntityRef, origin: Vec3, angles: Vec3);
}

/// Cvar value snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CvarValue {
    /// String value.
    pub value: String,
    /// Integer value.
    pub integer_value: i32,
}

/// VM cvar snapshot (`CvarSnapshot` subset).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CvarSnapshot {
    /// String value.
    pub value: String,
    /// Integer value.
    pub integer_value: i32,
    /// Modification count.
    pub modification_count: i32,
}

/// Cvar registry (`CvarRegistry` subset).
pub trait CvarRegistry {
    /// Read a cvar.
    fn get(&self, name: &str) -> Option<CvarValue>;
    /// Write a cvar.
    fn set(&self, name: &str, value: &str, force: bool);
}

/// Config-string services (`ConfigStringRegistry` subset).
pub trait ConfigStrings {
    /// Model index for a path.
    fn model_index(&self, name: &str) -> i32;
}

/// Session cvar name (`SessionCvarName`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SessionCvarName {
    /// World session record.
    Session,
    /// Per-client session record.
    SessionN(usize),
}

impl SessionCvarName {
    /// Cvar spelling.
    #[must_use]
    pub fn as_string(&self) -> String {
        match self {
            SessionCvarName::Session => "session".to_string(),
            SessionCvarName::SessionN(index) => format!("session{index}"),
        }
    }
}

/// Session cvar storage (`SessionCvarService`).
pub trait SessionCvarService {
    /// Read a session cvar.
    fn get(&self, name: &SessionCvarName) -> String;
    /// Write a session cvar.
    fn set(&self, name: &SessionCvarName, value: &str);
}

/// Session services (`SessionServices`).
pub trait SessionServices {
    /// Engine print.
    fn print(&self, message: &str);
    /// Announce a team change.
    fn broadcast_team_change(&self, client_num: usize, old_team: i32);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q3::team_arena::client_admission::*;
    use crate::q3::team_arena::client_policy::*;
    use crate::q3::team_arena::client_spawn::*;
    use crate::q3::team_arena::client_think::*;
    use crate::q3::team_arena::commands::*;
    use crate::q3::team_arena::r#match::*;
    use crate::q3::team_arena::server_commands::*;
    use crate::q3::team_arena::team::*;

    use qa_core::identity::{ActorId, IdentityOwner};

    use crate::q3::team_arena::arenas::*;
    use std::cell::{Cell, RefCell};
    use std::collections::HashMap;
    use std::rc::Rc;

    use crate::q3::team_arena::client_effects::*;
    use crate::q3::team_arena::client_events::*;

    use crate::q3::team_arena::foreign_objectives::*;
    use crate::q3::team_arena::movement_host::*;
    use crate::q3::team_arena::objective_placement::*;

    use crate::q3::team_arena::session::*;

    struct TestRankings {
        calls: RefCell<Vec<String>>,
    }

    impl RankingsHost for TestRankings {
        fn use_holdable(&self, slot: usize, holdable: i32) {
            self.calls.borrow_mut().push(format!("holdable {slot} {holdable}"));
        }

        fn capture(&self, slot: usize) {
            self.calls.borrow_mut().push(format!("capture {slot}"));
        }

        fn pickup_powerup(&self, slot: usize, powerup: i32) {
            self.calls.borrow_mut().push(format!("powerup {slot} {powerup}"));
        }
    }

    fn test_pool(product: Product, max_clients: usize, time: Rc<Cell<i32>>, log: Rc<RefCell<Vec<String>>>) -> PoolRef {
        let time_hook: Rc<dyn Fn() -> i32> = Rc::new({
            let time = time.clone();
            move || time.get()
        });
        let link_log = log.clone();
        let unlink_log = log.clone();
        let print_log = log.clone();
        Rc::new(EntityPool::new(
            product,
            max_clients,
            PoolHooks {
                time: time_hook,
                map_start_time: 0,
                link: Rc::new(move |entity: &EntityRef| {
                    link_log.borrow_mut().push(format!("link {}", entity.borrow().slot));
                }),
                unlink: Rc::new(move |entity: &EntityRef| {
                    unlink_log.borrow_mut().push(format!("unlink {}", entity.borrow().slot));
                }),
                print: Rc::new(move |text: &str| {
                    print_log.borrow_mut().push(format!("print {text}"));
                }),
            },
            Box::new(TestRankings {
                calls: RefCell::new(Vec::new()),
            }),
        ))
    }

    struct StubWorld {
        links: RefCell<Vec<usize>>,
        unlinks: RefCell<Vec<i32>>,
        states: RefCell<HashMap<i32, LinkState>>,
        contents: Cell<i32>,
        trace: RefCell<ActorTraceResult>,
        area: RefCell<Vec<ActorId>>,
        contact: Cell<bool>,
    }

    impl StubWorld {
        fn new() -> Self {
            Self {
                links: RefCell::new(Vec::new()),
                unlinks: RefCell::new(Vec::new()),
                states: RefCell::new(HashMap::new()),
                contents: Cell::new(0),
                trace: RefCell::new(ActorTraceResult {
                    end: vec3(0.0, 0.0, 0.0),
                    solidity: TraceSolidity::Clear,
                    hit: TraceHit::None,
                }),
                area: RefCell::new(Vec::new()),
                contact: Cell::new(false),
            }
        }
    }

    impl Q3World for StubWorld {
        fn link(&self, entity: &EntityRef) {
            self.links.borrow_mut().push(entity.borrow().slot);
        }

        fn unlink(&self, number: i32) {
            self.unlinks.borrow_mut().push(number);
        }

        fn link_state(&self, number: i32) -> Option<LinkState> {
            self.states.borrow().get(&number).copied()
        }

        fn point_contents(&self, _point: Vec3, _pass_entity: i32) -> i32 {
            self.contents.get()
        }

        fn trace_actor(&self, _query: &ActorTraceQuery) -> ActorTraceResult {
            self.trace.borrow().clone()
        }

        fn area_actors(&self, _bounds: &Bounds, _maximum: usize) -> Vec<ActorId> {
            self.area.borrow().clone()
        }

        fn contact_actor(&self, _bounds: &Bounds, _actor: &ActorId) -> bool {
            self.contact.get()
        }
    }

    struct StubCombat {
        product: Product,
        time: Cell<i32>,
        game_type: Cell<i32>,
        intermission_queued: Cell<i32>,
        pool: PoolRef,
        damage_calls: RefCell<Vec<(usize, i32, i32)>>,
    }

    impl Combat for StubCombat {
        fn product(&self) -> Product {
            self.product
        }

        fn time(&self) -> i32 {
            self.time.get()
        }

        fn game_type(&self) -> i32 {
            self.game_type.get()
        }

        fn intermission_queued(&self) -> i32 {
            self.intermission_queued.get()
        }

        fn pool(&self) -> PoolRef {
            self.pool.clone()
        }

        fn damage(
            &self,
            target: &EntityRef,
            _inflictor: Option<&DamageParticipant>,
            _attacker: Option<&DamageParticipant>,
            _direction: Option<Vec3>,
            _point: Option<Vec3>,
            amount: i32,
            flags: i32,
            method: i32,
        ) {
            let _ = flags;
            self.damage_calls
                .borrow_mut()
                .push((target.borrow().slot, amount, method));
        }
    }

    struct StubItems {
        tags: RefCell<HashMap<usize, i32>>,
        by_name: RefCell<HashMap<String, ItemDefinition>>,
        by_powerup: RefCell<HashMap<i32, ItemDefinition>>,
        log: RefCell<Vec<String>>,
    }

    impl StubItems {
        fn new() -> Self {
            Self {
                tags: RefCell::new(HashMap::new()),
                by_name: RefCell::new(HashMap::new()),
                by_powerup: RefCell::new(HashMap::new()),
                log: RefCell::new(Vec::new()),
            }
        }
    }

    impl ItemHost for StubItems {
        fn item_at(&self, _product: Product, index: usize) -> ItemDefinition {
            ItemDefinition {
                tag: self.tags.borrow().get(&index).copied().unwrap_or(index as i32),
                class_name: format!("item{index}"),
                pickup_name: None,
            }
        }

        fn find_item(&self, _product: Product, name: &str) -> Option<ItemDefinition> {
            self.by_name.borrow().get(name).cloned()
        }

        fn find_item_for_powerup(&self, _product: Product, powerup: i32) -> Option<ItemDefinition> {
            self.by_powerup.borrow().get(&powerup).cloned()
        }

        fn spawn_item(&self, entity: &EntityRef, item: &ItemDefinition, _vars: &SpawnVariables, disabled: bool) {
            entity.borrow_mut().item = Some(item.clone());
            self.log.borrow_mut().push(format!("spawn disabled={disabled}"));
        }

        fn finish_spawning_item(&self, _entity: &EntityRef) {
            self.log.borrow_mut().push("finish".to_string());
        }

        fn touch_item(&self, entity: &EntityRef, _other: &DamageParticipant, _contact: &TouchContact) {
            self.log.borrow_mut().push(format!("touch {}", entity.borrow().slot));
        }
    }

    struct StubEffects {
        combat: CombatRef,
        items: Rc<StubItems>,
        intermission_time: Cell<i32>,
        smooth: Cell<bool>,
        fry: Cell<i32>,
        random: Cell<i32>,
        sounds: RefCell<Vec<(usize, i32, i32)>>,
        indexes: RefCell<Vec<String>>,
        spectator_frames: RefCell<Vec<usize>>,
    }

    impl EffectsCore for StubEffects {
        fn combat(&self) -> CombatRef {
            self.combat.clone()
        }

        fn items(&self) -> Rc<dyn ItemHost> {
            self.items.clone()
        }
    }

    impl EffectsHost for StubEffects {
        fn intermission_time(&self) -> i32 {
            self.intermission_time.get()
        }

        fn smooth_clients(&self) -> bool {
            self.smooth.get()
        }

        fn fry_sound(&self) -> i32 {
            self.fry.get()
        }

        fn random_int(&self) -> i32 {
            self.random.get()
        }

        fn sound_index(&self, path: &str) -> i32 {
            self.indexes.borrow_mut().push(path.to_string());
            self.indexes.borrow().len() as i32
        }

        fn sound(&self, entity: &EntityRef, channel: i32, sound_index: i32) {
            self.sounds
                .borrow_mut()
                .push((entity.borrow().slot, channel, sound_index));
        }

        fn spectator_end_frame(&self, entity: &EntityRef) {
            self.spectator_frames.borrow_mut().push(entity.borrow().slot);
        }
    }

    struct StubCvars {
        values: RefCell<HashMap<String, (String, i32)>>,
        sets: RefCell<Vec<(String, String)>>,
    }

    impl CvarRegistry for StubCvars {
        fn get(&self, name: &str) -> Option<CvarValue> {
            self.values.borrow().get(name).map(|(value, integer)| CvarValue {
                value: value.clone(),
                integer_value: *integer,
            })
        }

        fn set(&self, name: &str, value: &str, _force: bool) {
            self.sets.borrow_mut().push((name.to_string(), value.to_string()));
        }
    }

    #[test]
    fn codes_match_donor_values() {
        assert_eq!(game_type::ONE_FLAG_CTF, 5);
        assert_eq!(game_type::HARVESTER, 7);
        assert_eq!(team::NUM_TEAMS, 4);
        assert_eq!(move_type::SPINTERMISSION, 6);
        assert_eq!(powerup::INVULNERABILITY, 14);
        assert_eq!(weapon::CHAINGUN, 13);
        assert_eq!(entity_event::TAUNT_PATROL, 82);
        assert_eq!(entity_event::KAMIKAZE, 68);
        assert_eq!(entity_type::EVENTS, 13);
        assert_eq!(persistent_index::CAPTURES, 14);
        assert_eq!(trajectory_type::GRAVITY, 5);
        assert_eq!(player_animation::FLAG_STAND2RUN, 36);
        assert_eq!(move_flags::INVULEXPAND, 16384);
        assert_eq!(command_buttons::ANY, 2048);
        assert_eq!(connection_state::CONNECTED, 2);
        assert_eq!(spectator_state::SCOREBOARD, 3);
        assert_eq!(team_state::ACTIVE, 1);
        assert_eq!(game_flags::FORCE_GESTURE, 0x8000);
        assert_eq!(server_entity_flags::NOTSINGLECLIENT, 2048);
        assert_eq!(damage_flags::NO_TEAM_PROTECTION, 0x10);
        assert_eq!(ENTITYNUM_WORLD, 1022);
        assert_eq!(ENTITYNUM_NONE, 1023);
        assert_eq!(MAX_CLIENTS, 64);
        assert_eq!(MAX_GENTITIES, 1024);
        assert_eq!(GIB_HEALTH, -40);
        assert_eq!(EV_EVENT_BITS, 0x300);
        assert_eq!(flag_status::DROPPED, 4);
        assert_eq!(global_team_sound::KAMIKAZE, 13);
        let base = stat_schema(Product::BaseQ3);
        assert_eq!((base.health, base.weapons, base.max_health), (0, 2, 6));
        assert_eq!(base.persistent_powerup, None);
        let mp = stat_schema(Product::MissionPack);
        assert_eq!((mp.weapons, mp.max_health), (3, 7));
        assert_eq!(mp.persistent_powerup, Some(2));
        assert_eq!(weapon_count(Product::BaseQ3), 11);
        assert_eq!(weapon_count(Product::MissionPack), 14);
    }

    #[test]
    fn game_atoi_wraps_and_skips() {
        assert_eq!(game_atoi("  -42x"), -42);
        assert_eq!(game_atoi("+7"), 7);
        assert_eq!(game_atoi(""), 0);
        assert_eq!(game_atoi("   "), 0);
        assert_eq!(game_atoi("2147483648"), -2147483648);
        assert_eq!(game_atoi("9999999999"), 1410065407);
        assert_eq!(game_atoi("12abc34"), 12);
    }

    #[test]
    fn game_atof_reads_decimal_prefix() {
        assert_eq!(game_atof("1.5"), 1.5);
        assert_eq!(game_atof("-2.25x"), -2.25);
        assert_eq!(game_atof("abc"), 0.0);
        assert_eq!(game_atof(".5"), 0.5);
        assert_eq!(game_atof("3."), 3.0);
        assert_eq!(game_atof("  10"), 10.0);
    }

    #[test]
    fn game_random_is_deterministic() {
        let first = GameRandom::new(1);
        let second = GameRandom::new(1);
        for _ in 0..8 {
            assert_eq!(first.rand(), second.rand());
        }
        assert_eq!(GameRandom::new(0).rand(), 1);
        let fraction = GameRandom::new(42).random();
        assert!((0.0..=1.0).contains(&fraction));
    }

    #[test]
    fn game_format_covers_used_specifiers() {
        let args = |values: Vec<FormatArg>| values;
        assert_eq!(game_format_default("%i", &args(vec![42.into()])), "42");
        assert_eq!(game_format_default("%3i:", &args(vec![5.into()])), "  5:");
        assert_eq!(game_format_default("%-3i:", &args(vec![5.into()])), "5  :");
        assert_eq!(game_format_default("%03i", &args(vec![5.into()])), "005");
        assert_eq!(game_format_default("%d", &args(vec![(-7).into()])), "-7");
        assert_eq!(game_format_default("%s", &args(vec!["hi".into()])), "hi");
        assert_eq!(game_format_default("%s", &args(vec![FormatArg::Null])), "(null)");
        assert_eq!(game_format_default("%c", &args(vec![94.into()])), "^");
        assert_eq!(game_format_default("%%", &args(vec![])), "%");
        assert_eq!(
            game_format_default("n\\%s\\t\\%i", &args(vec!["bob".into(), 1.into()])),
            "n\\bob\\t\\1"
        );
        assert_eq!(game_format("ab\0cd", &args(vec![]), 32), "ab");
        assert_eq!(game_format("%s%s", &args(vec!["aa".into(), "bb".into()]), 3), "aa");
        assert_eq!(game_format("%.2s", &args(vec!["abcdef".into()]), 32), "ab");
    }

    #[test]
    fn trajectories_evaluate() {
        let linear = Trajectory {
            traj_type: trajectory_type::LINEAR,
            time: 1000,
            base: vec3(1.0, 2.0, 3.0),
            delta: vec3(10.0, 0.0, 0.0),
            ..Trajectory::default()
        };
        assert_eq!(evaluate_trajectory(&linear, 2000), vec3(11.0, 2.0, 3.0));
        let gravity = Trajectory {
            traj_type: trajectory_type::GRAVITY,
            time: 0,
            base: vec3(0.0, 0.0, 100.0),
            delta: vec3(0.0, 0.0, 0.0),
            ..Trajectory::default()
        };
        assert_eq!(evaluate_trajectory(&gravity, 1000), vec3(0.0, 0.0, -300.0));
        let stopped = Trajectory {
            traj_type: trajectory_type::LINEAR_STOP,
            time: 0,
            duration: 500,
            base: vec3(0.0, 0.0, 0.0),
            delta: vec3(2.0, 0.0, 0.0),
        };
        assert_eq!(evaluate_trajectory(&stopped, 5000), vec3(1.0, 0.0, 0.0));
        let sine = Trajectory {
            traj_type: trajectory_type::SINE,
            time: 0,
            duration: 1000,
            base: vec3(5.0, 0.0, 0.0),
            delta: vec3(2.0, 0.0, 0.0),
        };
        let at = evaluate_trajectory(&sine, 250);
        assert!((at.x - 7.0).abs() < 1e-5);
        assert!(!player_touches_item(vec3(100.0, 0.0, 0.0), &Trajectory::default(), 0));
        assert!(player_touches_item(vec3(0.0, 0.0, 0.0), &Trajectory::default(), 0));
    }

    #[test]
    fn info_values_round_trip() {
        let printed = Rc::new(RefCell::new(Vec::new()));
        let print = {
            let printed = printed.clone();
            move |text: &str| printed.borrow_mut().push(text.to_string())
        };
        let info = set_info_value("", "name", "bob", 1024, &print);
        assert_eq!(info, "\\name\\bob");
        let info = set_info_value(&info, "name", "al", 1024, &print);
        assert_eq!(info, "\\name\\al");
        let info = set_info_value(&info, "name", "", 1024, &print);
        assert_eq!(info, "");
        let before = "\\a\\b".to_string();
        assert_eq!(set_info_value(&before, "k;ey", "v", 1024, &print), before);
        assert!(!printed.borrow().is_empty());
    }

    #[test]
    fn slots_check_bounds() {
        let slots = SlotArray::new(4);
        slots.set(3, 9);
        assert_eq!(slots.get(3), 9);
        assert_eq!(slots.copy(), vec![0, 0, 0, 9]);
    }

    #[test]
    #[should_panic(expected = "outside 4")]
    fn slots_reject_overflow() {
        let _ = SlotArray::new(4).get(4);
    }

    #[test]
    fn pool_spawns_and_recycles() {
        let time = Rc::new(Cell::new(5000));
        let log = Rc::new(RefCell::new(Vec::new()));
        let pool = test_pool(Product::BaseQ3, 2, time.clone(), log);
        assert_eq!(pool.num_entities(), MAX_CLIENTS);
        let first = pool.spawn();
        assert_eq!(first.borrow().slot, MAX_CLIENTS);
        assert_eq!(first.borrow().s.number, MAX_CLIENTS as i32);
        let temp = pool.temp_entity(vec3(1.5, 2.5, 3.5), entity_event::RAILTRAIL);
        assert_eq!(temp.borrow().s.e_type, entity_type::EVENTS + entity_event::RAILTRAIL);
        assert_eq!(temp.borrow().s.pos.base, vec3(1.0, 2.0, 3.0));
        assert!(temp.borrow().free_after_event);
        pool.free(&first);
        time.set(5500);
        let reused = pool.spawn();
        // Freed less than a second ago, so the pool extends instead.
        assert_ne!(reused.borrow().slot, first.borrow().slot);
        pool.free(&reused);
        pool.free(&temp);
        time.set(9000);
        let recycled = pool.spawn();
        assert_eq!(recycled.borrow().slot, MAX_CLIENTS);
        assert_eq!(recycled.borrow().classname(), Some("noclass".to_string()));
    }

    #[test]
    fn pool_events_set_bits() {
        let time = Rc::new(Cell::new(100));
        let log = Rc::new(RefCell::new(Vec::new()));
        let pool = test_pool(Product::BaseQ3, 2, time, log);
        let entity = pool.at(0);
        pool.add_event(&entity, entity_event::PAIN, 55);
        let record = entity.borrow().client.clone().unwrap();
        assert_eq!(record.borrow().ps.external_event, entity_event::PAIN | EV_EVENT_BIT1);
        assert_eq!(record.borrow().ps.external_event_parm, 55);
        let temp = pool.temp_entity(vec3(0.0, 0.0, 0.0), entity_event::BULLET);
        pool.add_event(&temp, entity_event::BULLET_HIT_WALL, 3);
        assert_eq!(temp.borrow().s.event, entity_event::BULLET_HIT_WALL | EV_EVENT_BIT1);
        assert_eq!(pool.vtos(vec3(1.2, -3.7, 4.0)), "(1 -3 4)");
    }

    #[test]
    fn entity_search_is_case_insensitive() {
        let time = Rc::new(Cell::new(0));
        let log = Rc::new(RefCell::new(Vec::new()));
        let pool = test_pool(Product::BaseQ3, 1, time, log);
        let entity = pool.spawn();
        entity
            .borrow_mut()
            .set_classname(Some("Info_Player_Deathmatch".to_string()));
        let found = find_entity(
            &pool,
            None,
            EntityStringField::Classname,
            Some("info_player_deathmatch"),
        );
        assert!(found.map(|entry| Rc::ptr_eq(&entry, &entity)).unwrap_or(false));
        assert!(find_entity(
            &pool,
            Some(&entity),
            EntityStringField::Classname,
            Some("info_player_deathmatch")
        )
        .is_none());
    }

    #[test]
    fn target_pick_warns_and_selects() {
        let time = Rc::new(Cell::new(0));
        let log = Rc::new(RefCell::new(Vec::new()));
        let pool = test_pool(Product::BaseQ3, 1, time, log);
        let warnings = Rc::new(RefCell::new(Vec::new()));
        let warn = {
            let warnings = warnings.clone();
            move |text: &str| warnings.borrow_mut().push(text.to_string())
        };
        assert!(pick_target(&pool, &|| 0, &warn, None).is_none());
        assert!(pick_target(&pool, &|| 0, &warn, Some("missing")).is_none());
        assert_eq!(warnings.borrow().len(), 2);
        for name in ["t1", "t1"] {
            let entity = pool.spawn();
            entity.borrow_mut().targetname = Some(name.to_string());
        }
        let picked = pick_target(&pool, &|| 1, &warn, Some("t1")).unwrap();
        assert_eq!(picked.borrow().targetname, Some("t1".to_string()));
    }

    #[test]
    fn snapshot_mapping_marks_dead_and_events() {
        let mut ps = PlayerState::new(Product::BaseQ3);
        ps.client_num = 3;
        ps.origin = vec3(10.0, 20.0, 30.0);
        ps.velocity = vec3(1.0, 0.0, 0.0);
        ps.viewangles = vec3(0.0, 90.0, 0.0);
        ps.movement_dir = 2;
        ps.set_health(-50);
        ps.events.set(0, entity_event::JUMP);
        ps.event_sequence = 1;
        ps.powerups.set(2, 999);
        let mut state = EntityState::default();
        player_state_to_entity_state(&mut ps, &mut state, true);
        assert_eq!(state.e_type, entity_type::INVISIBLE);
        assert_eq!(state.number, 3);
        assert_eq!(state.event, entity_event::JUMP);
        assert_eq!(state.powerups, 1 << 2);
        assert_eq!(ps.entity_event_sequence, 1);
        player_state_to_entity_state_extrapolate(&mut ps, &mut state, 500, true);
        assert_eq!(state.pos.traj_type, trajectory_type::LINEAR_STOP);
        assert_eq!(state.pos.time, 500);
        assert_eq!(state.pos.duration, 50);
    }

    struct StubSessionCvars {
        values: RefCell<HashMap<String, String>>,
    }

    impl SessionCvarService for StubSessionCvars {
        fn get(&self, name: &SessionCvarName) -> String {
            self.values.borrow().get(&name.as_string()).cloned().unwrap_or_default()
        }

        fn set(&self, name: &SessionCvarName, value: &str) {
            self.values.borrow_mut().insert(name.as_string(), value.to_string());
        }
    }

    struct StubSessionServices {
        prints: RefCell<Vec<String>>,
        teams: RefCell<Vec<(usize, i32)>>,
    }

    impl SessionServices for StubSessionServices {
        fn print(&self, message: &str) {
            self.prints.borrow_mut().push(message.to_string());
        }

        fn broadcast_team_change(&self, client_num: usize, old_team: i32) {
            self.teams.borrow_mut().push((client_num, old_team));
        }
    }

    struct TeamUserinfo {
        team: String,
    }

    impl SessionUserinfo for TeamUserinfo {
        fn value_for_key(&self, _key: &str) -> String {
            self.team.clone()
        }
    }

    fn session_fixture(game_type: i32) -> (Rc<SessionWorld>, Rc<GameSessionManager>, Rc<StubSessionCvars>) {
        let time = Rc::new(Cell::new(1000));
        let log = Rc::new(RefCell::new(Vec::new()));
        let pool = test_pool(Product::BaseQ3, 4, time, log);
        let world = Rc::new(SessionWorld {
            clients: pool.clients()[..4].to_vec(),
            max_clients: 4,
            team_scores: Rc::new(SlotArray::new(4)),
            game_type: Cell::new(game_type),
            team_auto_join: Cell::new(false),
            max_game_clients: Cell::new(0),
            time: Cell::new(1000),
            num_non_spectator_clients: Cell::new(0),
            new_session: Cell::new(false),
        });
        let cvars = Rc::new(StubSessionCvars {
            values: RefCell::new(HashMap::new()),
        });
        let manager = Rc::new(GameSessionManager::new(
            world.clone(),
            Rc::new(StubSessionServices {
                prints: RefCell::new(Vec::new()),
                teams: RefCell::new(Vec::new()),
            }),
            cvars.clone(),
        ));
        (world, manager, cvars)
    }

    #[test]
    fn session_write_read_round_trip() {
        let (world, manager, cvars) = session_fixture(game_type::FFA);
        {
            let client = &world.clients[1];
            let mut record = client.borrow_mut();
            record.sess.session_team = team::RED;
            record.sess.spectator_time = 11;
            record.sess.spectator_state = spectator_state::FOLLOW;
            record.sess.spectator_client = 2;
            record.sess.wins = 3;
            record.sess.losses = 4;
            record.sess.team_leader = 1;
        }
        manager.write_client(1);
        assert_eq!(cvars.get(&SessionCvarName::SessionN(1)), "1 11 2 2 3 4 1");
        world.clients[1].borrow_mut().sess = ClientSession::default();
        manager.read_client(1);
        let record = world.clients[1].borrow();
        assert_eq!(record.sess.session_team, team::RED);
        assert_eq!(
            (record.sess.wins, record.sess.losses, record.sess.team_leader),
            (3, 4, 1)
        );
    }

    #[test]
    fn session_initialize_assigns_teams() {
        let (world, manager, _) = session_fixture(game_type::FFA);
        manager.initialize_client(0, &TeamUserinfo { team: "s".to_string() });
        assert_eq!(world.clients[0].borrow().sess.session_team, team::SPECTATOR);
        manager.initialize_client(1, &TeamUserinfo { team: "".to_string() });
        assert_eq!(world.clients[1].borrow().sess.session_team, team::FREE);

        let (world, manager, _) = session_fixture(game_type::TOURNAMENT);
        world.num_non_spectator_clients.set(2);
        manager.initialize_client(0, &TeamUserinfo { team: "".to_string() });
        assert_eq!(world.clients[0].borrow().sess.session_team, team::SPECTATOR);

        let (world, manager, _) = session_fixture(game_type::TEAM);
        world.team_auto_join.set(true);
        world.clients[0].borrow_mut().pers.connected = connection_state::CONNECTED;
        world.clients[0].borrow_mut().sess.session_team = team::RED;
        manager.initialize_client(1, &TeamUserinfo { team: "".to_string() });
        assert_eq!(world.clients[1].borrow().sess.session_team, team::BLUE);

        let (world, manager, _) = session_fixture(game_type::CTF);
        manager.initialize_world();
        assert!(world.new_session.get());
        manager.write_world();
    }

    #[test]
    fn placements_cover_modes() {
        let placed = |names: &[&str]| -> Vec<ObjectivePlacement> {
            names
                .iter()
                .map(|name| ObjectivePlacement {
                    classname: Some(name.to_string()),
                })
                .collect()
        };
        assert_eq!(
            check_objective_placements(Product::BaseQ3, game_type::FFA, &[]),
            ObjectivePlacementResult::Ready
        );
        assert_eq!(
            check_objective_placements(
                Product::BaseQ3,
                game_type::CTF,
                &placed(&["team_CTF_redflag", "team_CTF_blueflag"]),
            ),
            ObjectivePlacementResult::Ready
        );
        assert_eq!(
            check_objective_placements(Product::BaseQ3, game_type::CTF, &placed(&["team_CTF_redflag"])),
            ObjectivePlacementResult::MissingObjectives {
                classnames: vec![ObjectiveClassname::BlueFlag],
            }
        );
        assert_eq!(
            check_objective_placements(Product::BaseQ3, game_type::ONE_FLAG_CTF, &[]),
            ObjectivePlacementResult::UnsupportedMode {
                product: Product::BaseQ3,
                game_type: game_type::ONE_FLAG_CTF,
            }
        );
        assert_eq!(
            check_objective_placements(
                Product::MissionPack,
                game_type::HARVESTER,
                &placed(&["team_redobelisk", "team_blueobelisk", "team_neutralobelisk"]),
            ),
            ObjectivePlacementResult::Ready
        );
    }

    #[test]
    fn foreign_objectives_translate_and_place() {
        let q3 = ForeignWorld {
            kind: WorldKind::Q3Bsp,
            entities: "{\n\"classname\" \"team_CTF_redflag\"\n}\n{\n\"classname\" \"team_CTF_blueflag\"\n}\n{\n\"classname\" \"info_player_deathmatch\"\n}\n".to_string(),
        };
        match adapt_foreign_q3_objectives(&q3, Product::BaseQ3, game_type::CTF, &[]) {
            ForeignObjectiveAdaptation::Ready { translated, .. } => assert_eq!(translated, 0),
            other => panic!("unexpected {other:?}"),
        }
        match adapt_foreign_q3_objectives(&q3, Product::BaseQ3, game_type::ONE_FLAG_CTF, &[]) {
            ForeignObjectiveAdaptation::UnsupportedMode { .. } => {}
            other => panic!("unexpected {other:?}"),
        }
        let q1 = ForeignWorld {
            kind: WorldKind::Q1Bsp,
            entities: "{\n\"classname\" \"item_flag_team1\"\n}\n{\n\"classname\" \"item_flag_team2\"\n}\n{\n\"classname\" \"info_player_start\"\n}\n".to_string(),
        };
        match adapt_foreign_q3_objectives(&q1, Product::BaseQ3, game_type::CTF, &[]) {
            ForeignObjectiveAdaptation::Ready { entities, translated } => {
                assert_eq!(translated, 3);
                assert!(entities.contains("\"team_CTF_redflag\""));
                assert!(entities.contains("\"info_player_deathmatch\""));
            }
            other => panic!("unexpected {other:?}"),
        }
        let bare = ForeignWorld {
            kind: WorldKind::Q1Bsp,
            entities: "{\n\"classname\" \"info_player_start\"\n}\n".to_string(),
        };
        let explicit = vec![
            ExplicitObjectivePlacement {
                classname: ObjectiveClassname::RedFlag,
                origin: vec3(1.0, 2.0, 3.0),
            },
            ExplicitObjectivePlacement {
                classname: ObjectiveClassname::BlueFlag,
                origin: vec3(4.0, 5.0, 6.0),
            },
        ];
        match adapt_foreign_q3_objectives(&bare, Product::BaseQ3, game_type::CTF, &explicit) {
            ForeignObjectiveAdaptation::Ready { entities, .. } => {
                assert!(entities.contains("\"1 2 3\""));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    #[should_panic(expected = "authored player spawn")]
    fn foreign_objectives_require_spawn() {
        let world = ForeignWorld {
            kind: WorldKind::Q1Bsp,
            entities: "{\n\"classname\" \"item_flag_team1\"\n}\n{\n\"classname\" \"item_flag_team2\"\n}\n".to_string(),
        };
        let _ = adapt_foreign_q3_objectives(&world, Product::BaseQ3, game_type::CTF, &[]);
    }

    #[test]
    fn admission_names_and_configs() {
        assert_eq!(clean_client_name("  ^1Bob  ^2"), "^1Bob  ^2");
        assert_eq!(clean_client_name(""), "UnnamedPlayer");
        assert_eq!(clean_client_name("^0^1"), "UnnamedPlayer");
        assert_eq!(clean_client_name("a    b"), "a   b");
        assert_eq!(client_info_value("\\name\\bob\\TEAM\\red", "team"), "red");
        assert_eq!(client_info_value("name\\bob", "missing"), "");
        let time = Rc::new(Cell::new(0));
        let log = Rc::new(RefCell::new(Vec::new()));
        let pool = test_pool(Product::BaseQ3, 1, time, log);
        let client = pool.client_at(0);
        client.borrow_mut().pers.netname = "bob".to_string();
        client.borrow_mut().sess.session_team = team::RED;
        let config = client_presentation_config(
            &client,
            "\\model\\sarge\\headmodel\\sarge\\color1\\4\\color2\\5",
            game_type::FFA,
            None,
        );
        assert!(config.starts_with("n\\bob\\t\\1\\model\\sarge"));
        assert!(config.contains("\\c1\\4\\c2\\5"));
    }

    struct StubServerHost {
        cvars: RefCell<HashMap<String, CvarSnapshot>>,
        prints: RefCell<Vec<String>>,
        commands: RefCell<Vec<(i32, String)>>,
        console: RefCell<Vec<String>>,
        teams: RefCell<Vec<(usize, String)>>,
    }

    impl GameServerCommandHost for StubServerHost {
        fn read_vm_cvar(&self, name: ServerCommandCvar) -> CvarSnapshot {
            self.cvars.borrow().get(name.as_str()).cloned().unwrap_or(CvarSnapshot {
                value: String::new(),
                integer_value: 0,
                modification_count: 0,
            })
        }

        fn print(&self, text: &str) {
            self.prints.borrow_mut().push(text.to_string());
        }

        fn send_server_command(&self, client_num: i32, text: &str) {
            self.commands.borrow_mut().push((client_num, text.to_string()));
        }

        fn execute_console_now(&self, text: &str) {
            self.console.borrow_mut().push(text.to_string());
        }

        fn set_team(&self, entity: &EntityRef, team: &str) {
            self.teams.borrow_mut().push((entity.borrow().slot, team.to_string()));
        }

        fn bots(&self) -> ServerCommandCapability {
            ServerCommandCapability::Unavailable {
                reason: "no bots".to_string(),
            }
        }

        fn memory(&self) -> ServerCommandCapability {
            ServerCommandCapability::Available { run: Rc::new(|_| {}) }
        }

        fn podium(&self) -> ServerCommandCapability {
            ServerCommandCapability::Available { run: Rc::new(|_| {}) }
        }
    }

    fn server_fixture() -> (GameServerCommandRuntime, Rc<StubServerHost>, Rc<StubCvars>, PoolRef) {
        let time = Rc::new(Cell::new(0));
        let log = Rc::new(RefCell::new(Vec::new()));
        let pool = test_pool(Product::BaseQ3, 4, time, log);
        let cvars = Rc::new(StubCvars {
            values: RefCell::new(HashMap::new()),
            sets: RefCell::new(Vec::new()),
        });
        let host = Rc::new(StubServerHost {
            cvars: RefCell::new(HashMap::new()),
            prints: RefCell::new(Vec::new()),
            commands: RefCell::new(Vec::new()),
            console: RefCell::new(Vec::new()),
            teams: RefCell::new(Vec::new()),
        });
        let runtime = GameServerCommandRuntime::new(
            pool.clone(),
            cvars.clone(),
            host.clone(),
            GameServerCommandState::default(),
        );
        (runtime, host, cvars, pool)
    }

    fn argv(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| word.to_string()).collect()
    }

    #[test]
    fn server_ip_filters_round_trip() {
        let (runtime, host, cvars, _) = server_fixture();
        assert!(runtime.console_command(&argv(&["addip", "192.168.1.*"])));
        assert_eq!(
            cvars.sets.borrow().last(),
            Some(&("g_banIPs".to_string(), "192.168.1.* ".to_string()))
        );
        host.cvars.borrow_mut().insert(
            "g_filterBan".to_string(),
            CvarSnapshot {
                value: "1".to_string(),
                integer_value: 1,
                modification_count: 0,
            },
        );
        assert!(runtime.filter_packet("192.168.1.7"));
        assert!(!runtime.filter_packet("10.0.0.1"));
        assert!(runtime.console_command(&argv(&["removeip", "192.168.1.*"])));
        assert!(host.prints.borrow().iter().any(|line| line == "Removed.\n"));
        assert!(!runtime.filter_packet("192.168.1.7"));
        assert!(runtime.console_command(&argv(&["removeip", "10.0.0.*"])));
        assert!(host.prints.borrow().iter().any(|line| line.contains("Didn't find")));
    }

    #[test]
    fn server_bans_reload_and_list() {
        let (runtime, host, _, _) = server_fixture();
        host.cvars.borrow_mut().insert(
            "g_banIPs".to_string(),
            CvarSnapshot {
                value: "10.1.1.1  ".to_string(),
                integer_value: 0,
                modification_count: 3,
            },
        );
        host.cvars.borrow_mut().insert(
            "g_filterBan".to_string(),
            CvarSnapshot {
                value: "1".to_string(),
                integer_value: 1,
                modification_count: 0,
            },
        );
        runtime.process_ip_bans();
        assert!(runtime.filter_packet("10.1.1.1"));
        assert!(runtime.console_command(&argv(&["listip"])));
        assert_eq!(host.console.borrow().as_slice(), ["g_banIPs\n"]);
        let saved = runtime.capture_save_state();
        runtime.restore_save_state(&saved);
        assert!(runtime.filter_packet("10.1.1.1"));
    }

    #[test]
    fn server_console_lists_forces_and_chats() {
        let (runtime, host, _, pool) = server_fixture();
        let entity = pool.at(0);
        entity.borrow_mut().inuse = true;
        entity.borrow_mut().set_classname(Some("player".to_string()));
        entity.borrow_mut().client.clone().unwrap().borrow_mut().pers.connected = connection_state::CONNECTED;
        // The listing skips slot 0 (world), so the listed player lives on slot 1.
        pool.at(1).borrow_mut().inuse = true;
        pool.at(1).borrow_mut().set_classname(Some("player".to_string()));
        assert!(runtime.console_command(&argv(&["entitylist"])));
        assert!(host.prints.borrow().iter().any(|line| line.contains("player")));
        assert!(runtime.console_command(&argv(&["forceteam", "0", "red"])));
        assert_eq!(host.teams.borrow().as_slice(), [(0, "red".to_string())]);
        host.cvars.borrow_mut().insert(
            "dedicated".to_string(),
            CvarSnapshot {
                value: "1".to_string(),
                integer_value: 1,
                modification_count: 0,
            },
        );
        assert!(runtime.console_command(&argv(&["say", "hello", "there"])));
        assert!(host
            .commands
            .borrow()
            .iter()
            .any(|(_, text)| text.contains("hello there")));
        assert!(runtime.console_command(&argv(&["game_memory"])));
    }

    #[test]
    #[should_panic(expected = "addbot unavailable")]
    fn server_missing_capability_panics() {
        let (runtime, _, _, _) = server_fixture();
        runtime.console_command(&argv(&["addbot"]));
    }

    struct StubSpawnShort {
        respawns: RefCell<Vec<usize>>,
    }

    impl SpawnShort for StubSpawnShort {
        fn select_spawn_point(&self, _avoid: Vec3) -> SpawnPoint {
            panic!("unexpected spawn selection");
        }

        fn respawn(&self, entity: &EntityRef) {
            self.respawns.borrow_mut().push(entity.borrow().slot);
        }
    }

    struct StubMatchHost {
        product: Product,
        state: MatchStateRef,
        pool: PoolRef,
        team_scores: SharedSlots,
        random: GameRandom,
        spawn: Rc<StubSpawnShort>,
        settings: RefCell<MatchSettings>,
        log: RefCell<Vec<String>>,
        configstrings: RefCell<HashMap<i32, String>>,
        cvars: RefCell<HashMap<String, String>>,
        single_player: Cell<bool>,
    }

    impl MatchHost for StubMatchHost {
        fn product(&self) -> Product {
            self.product
        }

        fn state(&self) -> &MatchStateRef {
            &self.state
        }

        fn pool(&self) -> PoolRef {
            self.pool.clone()
        }

        fn team_scores(&self) -> SharedSlots {
            self.team_scores.clone()
        }

        fn random(&self) -> &GameRandom {
            &self.random
        }

        fn spawn(&self) -> Rc<dyn SpawnShort> {
            self.spawn.clone()
        }

        fn settings(&self) -> MatchSettings {
            self.settings.borrow().clone()
        }

        fn set_team(&self, entity: &EntityRef, team: &str) {
            self.log
                .borrow_mut()
                .push(format!("setteam {} {team}", entity.borrow().slot));
        }

        fn stop_following(&self, entity: &EntityRef) {
            self.log.borrow_mut().push(format!("stop {}", entity.borrow().slot));
        }

        fn send_scoreboard(&self, entity: &EntityRef) {
            self.log
                .borrow_mut()
                .push(format!("scoreboard {}", entity.borrow().slot));
        }

        fn client_userinfo_changed(&self, client_num: usize) {
            self.log.borrow_mut().push(format!("userinfo {client_num}"));
        }

        fn write_session_data(&self) {
            self.log.borrow_mut().push("session".to_string());
        }

        fn append_console_command(&self, text: &str) {
            self.log.borrow_mut().push(format!("console {text}"));
        }

        fn send_server_command(&self, client_num: i32, text: &str) {
            self.log.borrow_mut().push(format!("server {client_num} {text}"));
        }

        fn set_configstring(&self, index: i32, text: &str) {
            self.configstrings.borrow_mut().insert(index, text.to_string());
        }

        fn set_cvar(&self, name: &str, value: &str) {
            self.cvars.borrow_mut().insert(name.to_string(), value.to_string());
        }

        fn log(&self, text: &str) {
            self.log.borrow_mut().push(format!("log {text}"));
        }

        fn warn(&self, text: &str) {
            self.log.borrow_mut().push(format!("warn {text}"));
        }

        fn bot_interbreed_end_match(&self) {
            self.log.borrow_mut().push("bots".to_string());
        }

        fn update_tournament_info(&self) {
            self.log.borrow_mut().push("tourney".to_string());
        }

        fn spawn_models_on_victory_pads(&self) {
            self.log.borrow_mut().push("podium".to_string());
        }

        fn single_player(&self) -> bool {
            self.single_player.get()
        }
    }

    fn match_settings(game_type: i32) -> MatchSettings {
        MatchSettings {
            game_type,
            time_limit: 0,
            frag_limit: 0,
            capture_limit: 0,
            warmup_seconds: 0,
            warmup_modification_count: 0,
            password: String::new(),
            password_modification_count: 0,
        }
    }

    fn match_fixture(game_type: i32) -> (Rc<MatchRuntime>, Rc<StubMatchHost>) {
        let time = Rc::new(Cell::new(0));
        let log = Rc::new(RefCell::new(Vec::new()));
        let pool = test_pool(Product::BaseQ3, 4, time, log);
        let host = Rc::new(StubMatchHost {
            product: Product::BaseQ3,
            state: Rc::new(RefCell::new(MatchState::default())),
            pool,
            team_scores: Rc::new(SlotArray::new(4)),
            random: GameRandom::new(7),
            spawn: Rc::new(StubSpawnShort {
                respawns: RefCell::new(Vec::new()),
            }),
            settings: RefCell::new(match_settings(game_type)),
            log: RefCell::new(Vec::new()),
            configstrings: RefCell::new(HashMap::new()),
            cvars: RefCell::new(HashMap::new()),
            single_player: Cell::new(false),
        });
        let runtime = Rc::new(MatchRuntime::new(host.clone(), MatchModuleState::new()));
        (runtime, host)
    }

    fn connect_client(host: &StubMatchHost, slot: usize, team_code: i32, score: i32) {
        let client = host.pool.client_at(slot);
        let mut record = client.borrow_mut();
        record.pers.connected = connection_state::CONNECTED;
        record.sess.session_team = team_code;
        record.ps.persistant.set(persistent_index::SCORE as usize, score);
        drop(record);
        host.pool.at(slot).borrow_mut().inuse = true;
    }

    #[test]
    fn match_ranks_and_scores() {
        let (runtime, host) = match_fixture(game_type::FFA);
        connect_client(&host, 0, team::FREE, 5);
        connect_client(&host, 1, team::FREE, 9);
        connect_client(&host, 2, team::SPECTATOR, 0);
        runtime.calculate_ranks();
        let state = host.state.borrow();
        assert_eq!(state.num_connected_clients, 3);
        assert_eq!(state.num_playing_clients, 2);
        assert_eq!(state.sorted_clients[0], 1);
        assert_eq!(state.sorted_clients[1], 0);
        drop(state);
        assert_eq!(
            host.pool
                .client_at(1)
                .borrow()
                .ps
                .persistant
                .get(persistent_index::RANK as usize),
            0
        );
        assert_eq!(host.configstrings.borrow().get(&6), Some(&"9".to_string()));
        assert_eq!(host.configstrings.borrow().get(&7), Some(&"5".to_string()));
        assert!(!runtime.score_is_tied());
        host.pool
            .client_at(0)
            .borrow_mut()
            .ps
            .persistant
            .set(persistent_index::SCORE as usize, 9);
        runtime.calculate_ranks();
        assert!(runtime.score_is_tied());
    }

    #[test]
    fn match_votes_and_cvars() {
        let (runtime, host) = match_fixture(game_type::FFA);
        connect_client(&host, 0, team::FREE, 0);
        connect_client(&host, 1, team::FREE, 0);
        runtime.calculate_ranks();
        host.state.borrow_mut().vote.time = 1000;
        host.state.borrow_mut().vote.yes = 2;
        host.state.borrow_mut().time = 2000;
        runtime.check_vote();
        assert!(host.log.borrow().iter().any(|line| line.contains("Vote passed")));
        assert_eq!(host.state.borrow().vote.execute_time, 5000);
        host.state.borrow_mut().time = 6000;
        runtime.check_vote();
        assert!(host.log.borrow().iter().any(|line| line.starts_with("console")));
        host.settings.borrow_mut().password = "secret".to_string();
        host.settings.borrow_mut().password_modification_count = 1;
        runtime.check_cvars();
        assert_eq!(host.cvars.borrow().get("g_needpass"), Some(&"1".to_string()));
        runtime.check_cvars();
        assert_eq!(host.cvars.borrow().len(), 1);
    }

    #[test]
    fn match_team_votes_and_leaders() {
        let (runtime, host) = match_fixture(game_type::TEAM);
        connect_client(&host, 0, team::RED, 0);
        connect_client(&host, 1, team::RED, 0);
        runtime.calculate_ranks();
        host.state.borrow_mut().team_votes[0].time = 100;
        host.state.borrow_mut().team_votes[0].string = "leader 1".to_string();
        host.state.borrow_mut().team_votes[0].yes = 2;
        host.state.borrow_mut().time = 200;
        runtime.check_team_vote(team::RED);
        assert_eq!(host.pool.client_at(1).borrow().sess.team_leader, 1);
        host.pool.client_at(1).borrow_mut().sess.team_leader = 0;
        runtime.check_team_leader(team::RED);
        assert_eq!(host.pool.client_at(0).borrow().sess.team_leader, 1);
        runtime.print_team(team::RED, "hello");
        assert_eq!(
            host.log.borrow().iter().filter(|line| line.contains("hello")).count(),
            2
        );
    }

    #[test]
    fn match_exit_rules_and_intermission() {
        let (runtime, host) = match_fixture(game_type::FFA);
        connect_client(&host, 0, team::FREE, 3);
        connect_client(&host, 1, team::FREE, 1);
        host.pool.at(2).borrow_mut().inuse = true;
        host.pool
            .at(2)
            .borrow_mut()
            .set_classname(Some("info_player_intermission".to_string()));
        runtime.calculate_ranks();
        host.settings.borrow_mut().time_limit = 10;
        host.state.borrow_mut().time = 10 * 60_000 + 1;
        runtime.check_exit_rules();
        assert_ne!(host.state.borrow().intermission_queued, 0);
        host.state.borrow_mut().time += 2000;
        runtime.check_exit_rules();
        assert_ne!(host.state.borrow().intermission_time, 0);
        assert!(host.log.borrow().iter().any(|line| line.contains("Timelimit")));
        let saved = runtime.capture_save_state();
        runtime.restore_save_state(&saved);
        runtime.move_client_to_intermission(&host.pool.at(0));
        assert_eq!(host.pool.at(0).borrow().s.e_type, entity_type::GENERAL);
    }

    struct StubTeamHost {
        product: Product,
        pool: PoolRef,
        world: Rc<StubWorld>,
        game_type: Cell<i32>,
        time: Cell<i32>,
        team_scores: SharedSlots,
        sorted: RefCell<Vec<i32>>,
        location_head: RefCell<Option<EntityRef>>,
        obelisk: Cell<Option<ObeliskSettings>>,
        log: RefCell<Vec<String>>,
        configstrings: RefCell<HashMap<i32, String>>,
        scores: RefCell<Vec<(usize, i32)>>,
        ranks: Cell<i32>,
        respawned: RefCell<Vec<usize>>,
        pvs: Cell<bool>,
    }

    impl TeamHost for StubTeamHost {
        fn product(&self) -> Product {
            self.product
        }

        fn pool(&self) -> PoolRef {
            self.pool.clone()
        }

        fn world(&self) -> WorldRef {
            self.world.clone()
        }

        fn game_type(&self) -> i32 {
            self.game_type.get()
        }

        fn time(&self) -> i32 {
            self.time.get()
        }

        fn team_scores(&self) -> SharedSlots {
            self.team_scores.clone()
        }

        fn sorted_clients(&self) -> Vec<i32> {
            self.sorted.borrow().clone()
        }

        fn location_head(&self) -> Option<EntityRef> {
            self.location_head.borrow().clone()
        }

        fn obelisk_settings(&self) -> Option<ObeliskSettings> {
            self.obelisk.get()
        }

        fn send_server_command(&self, client_num: i32, text: &str) {
            self.log.borrow_mut().push(format!("server {client_num} {text}"));
        }

        fn set_configstring(&self, index: i32, text: &str) {
            self.configstrings.borrow_mut().insert(index, text.to_string());
        }

        fn warn(&self, text: &str) {
            self.log.borrow_mut().push(format!("warn {text}"));
        }

        fn add_score(&self, player: &EntityRef, _origin: Vec3, score: i32) {
            self.scores.borrow_mut().push((player.borrow().slot, score));
        }

        fn calculate_ranks(&self) {
            self.ranks.set(self.ranks.get() + 1);
        }

        fn respawn_item(&self, item: &EntityRef) {
            self.respawned.borrow_mut().push(item.borrow().slot);
        }

        fn in_pvs(&self, _first: Vec3, _second: Vec3) -> bool {
            self.pvs.get()
        }
    }

    fn team_fixture(product: Product, game_type: i32) -> (TeamRuntime, Rc<StubTeamHost>) {
        // Mid-game clock so freed-slot protection applies (map start is time 0).
        let time = Rc::new(Cell::new(3000));
        let log = Rc::new(RefCell::new(Vec::new()));
        let pool = test_pool(product, 4, time, log);
        for slot in 0..2 {
            pool.at(slot).borrow_mut().inuse = true;
            pool.client_at(slot).borrow_mut().pers.connected = connection_state::CONNECTED;
        }
        let host = Rc::new(StubTeamHost {
            product,
            pool,
            world: Rc::new(StubWorld::new()),
            game_type: Cell::new(game_type),
            time: Cell::new(3000),
            team_scores: Rc::new(SlotArray::new(4)),
            sorted: RefCell::new(vec![0, 1]),
            location_head: RefCell::new(None),
            obelisk: Cell::new(None),
            log: RefCell::new(Vec::new()),
            configstrings: RefCell::new(HashMap::new()),
            scores: RefCell::new(Vec::new()),
            ranks: Cell::new(0),
            respawned: RefCell::new(Vec::new()),
            pvs: Cell::new(true),
        });
        let runtime = TeamRuntime::new(host.clone());
        (runtime, host)
    }

    fn flag_base(host: &StubTeamHost, classname: &str) -> EntityRef {
        let entity = host.pool.spawn();
        entity.borrow_mut().set_classname(Some(classname.to_string()));
        entity.borrow_mut().item = Some(ItemDefinition {
            tag: if classname.contains("red") {
                powerup::REDFLAG
            } else {
                powerup::BLUEFLAG
            },
            class_name: classname.to_string(),
            pickup_name: None,
        });
        entity
    }

    #[test]
    fn team_helpers_cover_codes() {
        assert_eq!(other_team(team::RED), team::BLUE);
        assert_eq!(other_team(team::FREE), team::FREE);
        assert_eq!(team_name(team::SPECTATOR), "SPECTATOR");
        assert_eq!(other_team_name(team::BLUE), "RED");
        assert_eq!(team_color_string(team::RED), "^1");
        assert_eq!(team_color_string(9), "^7");
        let (runtime, host) = team_fixture(Product::BaseQ3, game_type::CTF);
        host.pool.client_at(0).borrow_mut().sess.session_team = team::RED;
        host.pool.client_at(1).borrow_mut().sess.session_team = team::RED;
        assert!(on_same_team(game_type::CTF, &host.pool.at(0), &host.pool.at(1)));
        assert!(!on_same_team(game_type::FFA, &host.pool.at(0), &host.pool.at(1)));
        runtime.print_message(None, "hi \"there\"\n");
        assert!(host.log.borrow().iter().any(|line| line.contains("hi 'there'")));
        spawn_team_point(&host.pool.at(0));
    }

    #[test]
    fn team_flag_take_and_capture() {
        let (runtime, host) = team_fixture(Product::BaseQ3, game_type::CTF);
        runtime.init_game();
        host.pool.client_at(0).borrow_mut().sess.session_team = team::RED;
        host.pool.client_at(0).borrow_mut().pers.netname = "red".to_string();
        host.pool.client_at(1).borrow_mut().sess.session_team = team::BLUE;
        host.pool.client_at(1).borrow_mut().pers.netname = "blue".to_string();
        let red_base = flag_base(&host, "team_CTF_redflag");
        let blue_base = flag_base(&host, "team_CTF_blueflag");
        assert_eq!(runtime.touch_enemy_flag(&blue_base, &host.pool.at(0), team::BLUE), -1);
        assert_ne!(
            host.pool
                .client_at(0)
                .borrow()
                .ps
                .powerups
                .get(powerup::BLUEFLAG as usize),
            0
        );
        assert_eq!(host.configstrings.borrow().get(&23), Some(&"01".to_string()));
        assert_eq!(runtime.touch_our_flag(&red_base, &host.pool.at(0), team::RED), 0);
        assert_eq!(host.team_scores.get(team::RED as usize), 1);
        assert_eq!(
            host.pool
                .client_at(0)
                .borrow()
                .ps
                .persistant
                .get(persistent_index::CAPTURES as usize),
            1
        );
        assert_eq!(host.ranks.get(), 1);
    }

    #[test]
    fn team_flag_return_and_frag_bonus() {
        let (runtime, host) = team_fixture(Product::BaseQ3, game_type::CTF);
        host.pool.client_at(0).borrow_mut().sess.session_team = team::RED;
        host.pool.client_at(0).borrow_mut().pers.netname = "red".to_string();
        host.pool.client_at(1).borrow_mut().sess.session_team = team::BLUE;
        host.pool.client_at(1).borrow_mut().pers.netname = "blue".to_string();
        let red_base = flag_base(&host, "team_CTF_redflag");
        flag_base(&host, "team_CTF_blueflag");
        let dropped = host.pool.spawn();
        dropped.borrow_mut().set_classname(Some("team_CTF_redflag".to_string()));
        dropped.borrow_mut().flags |= game_flags::DROPPED_ITEM;
        dropped.borrow_mut().item = Some(ItemDefinition {
            tag: powerup::REDFLAG,
            class_name: "team_CTF_redflag".to_string(),
            pickup_name: None,
        });
        runtime.check_dropped_item(&dropped);
        runtime.return_flag(team::RED);
        assert!(!dropped.borrow().inuse);
        assert!(host.respawned.borrow().contains(&red_base.borrow().slot));
        host.pool
            .client_at(1)
            .borrow_mut()
            .ps
            .powerups
            .set(powerup::REDFLAG as usize, 99999);
        runtime.frag_bonuses(&host.pool.at(1), Some(&host.pool.at(0)));
        assert!(host
            .scores
            .borrow()
            .iter()
            .any(|(slot, score)| *slot == 0 && *score == 2));
        assert_eq!(host.pool.client_at(0).borrow().pers.team_state.frag_carrier, 1);
    }

    #[test]
    fn team_locations_and_status() {
        let (runtime, host) = team_fixture(Product::BaseQ3, game_type::CTF);
        let marker = host.pool.spawn();
        marker.borrow_mut().count = 3;
        marker.borrow_mut().message = Some("Base".to_string());
        marker.borrow_mut().health = 7;
        host.location_head.borrow_mut().replace(marker);
        host.pool.client_at(0).borrow_mut().sess.session_team = team::RED;
        assert_eq!(
            runtime.get_location_message(&host.pool.at(0), 64),
            Some("^3Base^7".to_string())
        );
        host.time.set(5000);
        runtime.check_team_status();
        assert_eq!(host.pool.client_at(0).borrow().pers.team_state.location, 7);
        let saved = runtime.capture_save_state();
        runtime.restore_save_state(&saved);
    }

    #[test]
    fn team_obelisk_lifecycle() {
        let (runtime, host) = team_fixture(Product::MissionPack, game_type::OBELISK);
        host.obelisk.set(Some(ObeliskSettings {
            health: 100,
            regen_period_seconds: 1,
            regen_amount: 25,
            respawn_delay_seconds: 5,
        }));
        let marker = host.pool.spawn();
        marker.borrow_mut().set_classname(Some("team_redobelisk".to_string()));
        marker.borrow_mut().s.origin = vec3(0.0, 0.0, 10.0);
        host.world.trace.borrow_mut().end = vec3(0.0, 0.0, 0.0);
        runtime.spawn_team_obelisk(&marker, team::RED);
        assert_eq!(marker.borrow().s.e_type, entity_type::TEAM);
        let obelisk = find_entity(&host.pool, None, EntityStringField::Classname, Some("noclass")).unwrap();
        assert_eq!(obelisk.borrow().health, 100);
        host.pool.client_at(0).borrow_mut().sess.session_team = team::RED;
        assert!(runtime.check_obelisk_attack(&obelisk, &host.pool.at(0)));
        host.pool.client_at(1).borrow_mut().sess.session_team = team::BLUE;
        host.time.set(30_000);
        let before = host.pool.num_entities();
        assert!(!runtime.check_obelisk_attack(&obelisk, &host.pool.at(1)));
        // The attacked announcement spawns a broadcast temp entity.
        assert_eq!(host.pool.num_entities(), before + 1);
    }

    fn effects_fixture(product: Product) -> (PoolRef, Rc<StubCombat>, Rc<StubItems>, Rc<StubEffects>) {
        let time = Rc::new(Cell::new(5000));
        let log = Rc::new(RefCell::new(Vec::new()));
        let pool = test_pool(product, 2, time, log);
        let items = Rc::new(StubItems::new());
        let combat = Rc::new(StubCombat {
            product,
            time: Cell::new(5000),
            game_type: Cell::new(game_type::FFA),
            intermission_queued: Cell::new(0),
            pool: pool.clone(),
            damage_calls: RefCell::new(Vec::new()),
        });
        let effects = Rc::new(StubEffects {
            combat: combat.clone() as CombatRef,
            items: items.clone(),
            intermission_time: Cell::new(0),
            smooth: Cell::new(false),
            fry: Cell::new(9),
            random: Cell::new(0),
            sounds: RefCell::new(Vec::new()),
            indexes: RefCell::new(Vec::new()),
            spectator_frames: RefCell::new(Vec::new()),
        });
        (pool, combat, items, effects)
    }

    #[test]
    fn effect_feedback_and_timers() {
        let (pool, _combat, _items, effects) = effects_fixture(Product::BaseQ3);
        let entity = pool.at(0);
        let client = pool.client_at(0);
        client.borrow_mut().damage_blood = 30;
        client.borrow_mut().damage_armor = 10;
        client.borrow_mut().damage_from = vec3(1.0, 0.0, 0.0);
        damage_feedback(effects.as_ref(), &entity);
        assert_eq!(client.borrow().ps.damage_count, 40);
        assert_eq!(client.borrow().damage_blood, 0);
        assert_eq!(client.borrow().ps.damage_event, 1);
        assert_eq!(entity.borrow().pain_debounce_time, 5700);

        let rule = q3_ammo_regeneration_rule(weapon::ROCKET_LAUNCHER);
        assert_eq!((rule.max, rule.increment, rule.time), (10, 1, 1750));
        assert_eq!(step_q3_ammo_regeneration(&rule, 5, 1000, 800), (50, Some(6)));
        assert_eq!(step_q3_ammo_regeneration(&rule, 10, 9999, 10), (0, None));

        entity.borrow_mut().health = 80;
        client
            .borrow_mut()
            .ps
            .stats
            .set(stat_schema(Product::BaseQ3).max_health, 100);
        client.borrow_mut().ps.powerups.set(powerup::REGEN as usize, 99999);
        client_timer_actions(effects.as_ref(), &entity, 1000, None);
        assert_eq!(entity.borrow().health, 95);
        client.borrow_mut().ps.powerups.set(powerup::REGEN as usize, 0);
        entity.borrow_mut().health = 150;
        client_timer_actions(effects.as_ref(), &entity, 1000, None);
        assert_eq!(entity.borrow().health, 149);
    }

    #[test]
    fn effect_speed_powerups_and_frames() {
        let (pool, _combat, items, effects) = effects_fixture(Product::MissionPack);
        let entity = pool.at(0);
        let client = pool.client_at(0);
        client.borrow_mut().ps.stats.set(2, 2);
        items.tags.borrow_mut().insert(2, powerup::SCOUT);
        assert_eq!(client_speed_multiplier(items.as_ref(), &client.borrow().ps), 1.5);
        items.tags.borrow_mut().insert(2, powerup::NONE);
        client.borrow_mut().ps.powerups.set(powerup::HASTE as usize, 99999);
        assert_eq!(client_speed_multiplier(items.as_ref(), &client.borrow().ps), 1.3);
        client.borrow_mut().ps.powerups.set(4, 100);
        update_q3_client_powerups(effects.as_ref(), &client);
        assert_eq!(client.borrow().ps.powerups.get(4), 0);
        client.borrow_mut().invulnerability_time = 99999;
        update_q3_client_powerups(effects.as_ref(), &client);
        assert_eq!(client.borrow().ps.powerups.get(powerup::INVULNERABILITY as usize), 5000);

        client.borrow_mut().sess.session_team = team::SPECTATOR;
        client_end_frame(effects.as_ref(), &entity);
        assert_eq!(effects.spectator_frames.borrow().as_slice(), [0]);
        client.borrow_mut().sess.session_team = team::FREE;
        client.borrow_mut().pers.connected = connection_state::CONNECTED;
        entity.borrow_mut().health = 88;
        client_end_frame(effects.as_ref(), &entity);
        assert_eq!(client.borrow().ps.health(), 88);
        assert_eq!(entity.borrow().s.pos.traj_type, trajectory_type::INTERPOLATE);
    }

    struct StubWeapons {
        fires: RefCell<Vec<usize>>,
        kamikaze: RefCell<Vec<usize>>,
    }

    impl WeaponHost for StubWeapons {
        fn fire(&self, entity: &EntityRef) {
            self.fires.borrow_mut().push(entity.borrow().slot);
        }

        fn start_kamikaze(&self, entity: &EntityRef) {
            self.kamikaze.borrow_mut().push(entity.borrow().slot);
        }
    }

    struct StubDrops {
        pool: PoolRef,
        calls: RefCell<Vec<(usize, i32)>>,
    }

    impl DropHost for StubDrops {
        fn drop_item(&self, entity: &EntityRef, item: &ItemDefinition, angle: i32) -> EntityRef {
            self.calls.borrow_mut().push((entity.borrow().slot, angle));
            let dropped = self.pool.spawn();
            dropped.borrow_mut().item = Some(item.clone());
            dropped
        }
    }

    struct StubTeleport {
        calls: RefCell<Vec<(usize, Vec3, Vec3)>>,
    }

    impl TeleportHost for StubTeleport {
        fn teleport_player(&self, entity: &EntityRef, origin: Vec3, angles: Vec3) {
            self.calls.borrow_mut().push((entity.borrow().slot, origin, angles));
        }
    }

    struct StubSelector {
        pose: SpawnPose,
    }

    impl SpawnSelector for StubSelector {
        fn select_spawn_point(&self, _avoid: Vec3) -> SpawnPoint {
            SpawnPoint {
                origin: self.pose.origin,
                angles: self.pose.angles,
                entity: Rc::new(RefCell::new(GameEntity::new(0, ActorIdPlaceholder::actor()))),
            }
        }
    }

    struct ActorIdPlaceholder;

    impl ActorIdPlaceholder {
        fn actor() -> ActorId {
            IdentityOwner::create("test").unwrap().actor(0, 0)
        }
    }

    #[test]
    fn client_events_dispatch() {
        let (pool, combat, items, _) = effects_fixture(Product::BaseQ3);
        let weapons = Rc::new(StubWeapons {
            fires: RefCell::new(Vec::new()),
            kamikaze: RefCell::new(Vec::new()),
        });
        let drops = Rc::new(StubDrops {
            pool: pool.clone(),
            calls: RefCell::new(Vec::new()),
        });
        let teleport = Rc::new(StubTeleport {
            calls: RefCell::new(Vec::new()),
        });
        let entity = pool.at(0);
        entity.borrow_mut().s.e_type = entity_type::PLAYER;
        let client = pool.client_at(0);
        client.borrow_mut().ps.event_sequence = 1;
        client.borrow_mut().ps.events.set(0, entity_event::FALL_FAR);
        let context = ClientEvents {
            world: Rc::new(StubWorld::new()),
            weapons: weapons.clone(),
            spawns: Rc::new(StubSelector {
                pose: SpawnPose {
                    origin: vec3(7.0, 8.0, 9.0),
                    angles: vec3(0.0, 45.0, 0.0),
                },
            }),
            drops: drops.clone(),
            items: items.clone(),
            teleport: teleport.clone(),
            dmflags: 0,
            primary_attack_allowed: None,
            product: Product::BaseQ3,
            combat: combat.clone() as CombatRef,
            personal_portal: None,
        };
        client_events(&context, &entity, 0);
        assert_eq!(combat.damage_calls.borrow().as_slice(), [(0, 10, 19)]);
        assert_eq!(entity.borrow().pain_debounce_time, 5200);

        client.borrow_mut().ps.events.set(0, entity_event::FIRE_WEAPON);
        client_events(&context, &entity, 0);
        assert_eq!(weapons.fires.borrow().as_slice(), [0]);

        client.borrow_mut().ps.events.set(0, entity_event::USE_ITEM2);
        client
            .borrow_mut()
            .ps
            .stats
            .set(stat_schema(Product::BaseQ3).max_health, 100);
        client_events(&context, &entity, 0);
        assert_eq!(entity.borrow().health, 125);

        items.by_powerup.borrow_mut().insert(
            powerup::REDFLAG,
            ItemDefinition {
                tag: powerup::REDFLAG,
                class_name: "team_CTF_redflag".to_string(),
                pickup_name: None,
            },
        );
        client.borrow_mut().ps.powerups.set(powerup::REDFLAG as usize, 10_000);
        client.borrow_mut().ps.events.set(0, entity_event::USE_ITEM1);
        client_events(&context, &entity, 0);
        assert_eq!(client.borrow().ps.powerups.get(powerup::REDFLAG as usize), 0);
        assert_eq!(drops.calls.borrow().len(), 1);
        assert_eq!(teleport.calls.borrow().len(), 1);
        assert_eq!(teleport.calls.borrow()[0].1, vec3(7.0, 8.0, 9.0));
    }

    struct StubPolicyHost {
        pool: PoolRef,
        world: Rc<StubWorld>,
        time: Cell<i32>,
        inactivity: Cell<i32>,
        follow1: Cell<i32>,
        follow2: Cell<i32>,
        moves: RefCell<Vec<(usize, i32)>>,
        movement: RefCell<ClientMovementResult>,
        touched: RefCell<Vec<usize>>,
        cycles: RefCell<Vec<(usize, i32)>>,
        begins: RefCell<Vec<usize>>,
        drops: RefCell<Vec<(usize, String)>>,
        commands: RefCell<Vec<(i32, String)>>,
    }

    impl MovementHost for StubPolicyHost {
        fn move_client(
            &self,
            entity: &EntityRef,
            _command: &UserCommand,
            options: &ClientMovementOptions,
        ) -> ClientMovementResult {
            self.moves.borrow_mut().push((entity.borrow().slot, options.trace_mask));
            self.movement.borrow().clone()
        }
    }

    impl ClientPolicyHost for StubPolicyHost {
        fn pool(&self) -> PoolRef {
            self.pool.clone()
        }

        fn world(&self) -> WorldRef {
            self.world.clone()
        }

        fn time(&self) -> i32 {
            self.time.get()
        }

        fn inactivity_seconds(&self) -> i32 {
            self.inactivity.get()
        }

        fn follow1(&self) -> i32 {
            self.follow1.get()
        }

        fn follow2(&self) -> i32 {
            self.follow2.get()
        }

        fn touch_triggers(&self, entity: &EntityRef) {
            self.touched.borrow_mut().push(entity.borrow().slot);
        }

        fn follow_cycle(&self, entity: &EntityRef, direction: i32) {
            self.cycles.borrow_mut().push((entity.borrow().slot, direction));
        }

        fn client_begin(&self, client_num: usize) {
            self.begins.borrow_mut().push(client_num);
        }

        fn drop_client(&self, client_num: usize, reason: &str) {
            self.drops.borrow_mut().push((client_num, reason.to_string()));
        }

        fn send_server_command(&self, client_num: i32, text: &str) {
            self.commands.borrow_mut().push((client_num, text.to_string()));
        }
    }

    fn policy_fixture() -> (PoolRef, Rc<StubPolicyHost>) {
        let time = Rc::new(Cell::new(60_000));
        let log = Rc::new(RefCell::new(Vec::new()));
        let pool = test_pool(Product::BaseQ3, 2, time, log);
        let host = Rc::new(StubPolicyHost {
            pool: pool.clone(),
            world: Rc::new(StubWorld::new()),
            time: Cell::new(60_000),
            inactivity: Cell::new(60),
            follow1: Cell::new(0),
            follow2: Cell::new(0),
            moves: RefCell::new(Vec::new()),
            movement: RefCell::new(ClientMovementResult {
                contacts: Vec::new(),
                bounds: Bounds {
                    min: vec3(-15.0, -15.0, -24.0),
                    max: vec3(15.0, 15.0, 32.0),
                },
                waterlevel: 0,
                watertype: 0,
                xyspeed: 0.0,
            }),
            touched: RefCell::new(Vec::new()),
            cycles: RefCell::new(Vec::new()),
            begins: RefCell::new(Vec::new()),
            drops: RefCell::new(Vec::new()),
            commands: RefCell::new(Vec::new()),
        });
        (pool, host)
    }

    #[test]
    fn policy_spectator_moves_and_cycles() {
        let (pool, host) = policy_fixture();
        let entity = pool.at(0);
        pool.client_at(0).borrow_mut().sess.session_team = team::SPECTATOR;
        let command = UserCommand {
            buttons: command_buttons::ATTACK,
            ..UserCommand::default()
        };
        spectator_think(host.as_ref(), &entity, &command);
        assert_eq!(pool.client_at(0).borrow().ps.pm_type, move_type::SPECTATOR);
        assert_eq!(pool.client_at(0).borrow().ps.speed, 400);
        assert_eq!(host.moves.borrow().as_slice(), [(0, 1 | 0x10000)]);
        assert_eq!(host.touched.borrow().as_slice(), [0]);
        assert!(host.world.unlinks.borrow().contains(&0));
        assert_eq!(host.cycles.borrow().as_slice(), [(0, 1)]);

        pool.client_at(1).borrow_mut().pers.connected = connection_state::CONNECTED;
        pool.client_at(1).borrow_mut().sess.session_team = team::RED;
        pool.client_at(1).borrow_mut().ps.origin = vec3(5.0, 6.0, 7.0);
        pool.client_at(0).borrow_mut().sess.spectator_state = spectator_state::FOLLOW;
        pool.client_at(0).borrow_mut().sess.spectator_client = 1;
        spectator_client_end_frame(host.as_ref(), &entity);
        assert_eq!(pool.client_at(0).borrow().ps.origin, vec3(5.0, 6.0, 7.0));
        assert_ne!(pool.client_at(0).borrow().ps.pm_flags & move_flags::FOLLOW, 0);
    }

    #[test]
    fn policy_inactivity_and_intermission() {
        let (pool, host) = policy_fixture();
        let client = pool.client_at(0);
        client.borrow_mut().inactivity_time = 1000;
        assert!(!client_inactivity_timer(host.as_ref(), &client));
        assert_eq!(host.drops.borrow().len(), 1);
        client.borrow_mut().inactivity_time = 65_000;
        client.borrow_mut().inactivity_warning = false;
        assert!(client_inactivity_timer(host.as_ref(), &client));
        assert!(client.borrow().inactivity_warning);
        assert!(host
            .commands
            .borrow()
            .iter()
            .any(|(_, text)| text.contains("inactivity drop")));
        host.inactivity.set(0);
        assert!(client_inactivity_timer(host.as_ref(), &client));
        assert_eq!(client.borrow().inactivity_time, 120_000);

        client.borrow_mut().pers.cmd.buttons = command_buttons::ATTACK;
        client.borrow_mut().buttons = 0;
        client_intermission_think(&client);
        assert!(client.borrow().ready_to_exit);
    }

    struct StubTouches {
        native: RefCell<HashMap<ActorId, EntityRef>>,
        triggers: RefCell<Vec<ActorId>>,
        calls: RefCell<Vec<(ActorId, ActorId)>>,
    }

    impl TouchAccess for StubTouches {
        fn native(&self, actor: &ActorId) -> Option<EntityRef> {
            self.native.borrow().get(actor).cloned()
        }

        fn is_trigger(&self, actor: &ActorId) -> bool {
            self.triggers.borrow().contains(actor)
        }

        fn touch(&self, this: &ActorId, other: &ActorId) {
            self.calls.borrow_mut().push((this.clone(), other.clone()));
        }
    }

    struct StubThinkHost {
        pool: PoolRef,
        world: Rc<StubWorld>,
        touches: Rc<StubTouches>,
        effects: Rc<StubEffects>,
        items: Rc<StubItems>,
        frame: Cell<ClientThinkFrame>,
        settings: RefCell<ClientThinkSettings>,
        moves: RefCell<Vec<usize>>,
        movement: RefCell<ClientMovementResult>,
        pmove_msec: Cell<i32>,
        intermissions: RefCell<Vec<usize>>,
        spectators: RefCell<Vec<usize>>,
        inactivity: Cell<bool>,
        hooks: RefCell<Vec<usize>>,
        gauntlet: Cell<bool>,
        events: RefCell<Vec<(usize, i32)>>,
        respawns: RefCell<Vec<usize>>,
        console: RefCell<Vec<String>>,
        door_trigger: Cell<bool>,
        aas: RefCell<Vec<Vec3>>,
    }

    fn think_settings() -> ClientThinkSettings {
        ClientThinkSettings {
            debug_move: 0,
            synchronous_clients: false,
            pmove_fixed: false,
            pmove_msec: 8,
            gravity: 800.0,
            speed: 320.0,
            dmflags: 0,
            smooth_clients: false,
            force_respawn_seconds: 0,
            single_player: false,
        }
    }

    fn movement_result() -> ClientMovementResult {
        ClientMovementResult {
            contacts: Vec::new(),
            bounds: Bounds {
                min: vec3(-15.0, -15.0, -24.0),
                max: vec3(15.0, 15.0, 32.0),
            },
            waterlevel: 0,
            watertype: 0,
            xyspeed: 0.0,
        }
    }

    impl MovementHost for StubThinkHost {
        fn move_client(
            &self,
            entity: &EntityRef,
            _command: &UserCommand,
            _options: &ClientMovementOptions,
        ) -> ClientMovementResult {
            self.moves.borrow_mut().push(entity.borrow().slot);
            self.movement.borrow().clone()
        }
    }

    impl ClientThinkHost for StubThinkHost {
        fn pool(&self) -> PoolRef {
            self.pool.clone()
        }

        fn world(&self) -> WorldRef {
            self.world.clone()
        }

        fn touches(&self) -> Rc<dyn TouchAccess> {
            self.touches.clone()
        }

        fn effects(&self) -> EffectsCoreRef {
            self.effects.clone()
        }

        fn items(&self) -> Rc<dyn ItemHost> {
            self.items.clone()
        }

        fn timer_ownership(&self, _actor: &ActorId) -> Option<ClientTimerOwnership> {
            None
        }

        fn speed_multiplier(&self, _actor: &ActorId) -> Option<f32> {
            None
        }

        fn frame(&self) -> ClientThinkFrame {
            self.frame.get()
        }

        fn settings(&self) -> ClientThinkSettings {
            *self.settings.borrow()
        }

        fn set_pmove_msec(&self, ms: i32) {
            self.pmove_msec.set(ms);
        }

        fn intermission_think(&self, client: &ClientRef) {
            self.intermissions
                .borrow_mut()
                .push(self.pool.client_index(client).unwrap());
        }

        fn spectator_think(&self, entity: &EntityRef, _command: &UserCommand) {
            self.spectators.borrow_mut().push(entity.borrow().slot);
        }

        fn check_inactivity(&self, _client: &ClientRef) -> bool {
            self.inactivity.get()
        }

        fn free_hook(&self, hook: &EntityRef) {
            self.hooks.borrow_mut().push(hook.borrow().slot);
        }

        fn check_gauntlet_attack(&self, _entity: &EntityRef) -> bool {
            self.gauntlet.get()
        }

        fn client_events(&self, entity: &EntityRef, old_sequence: i32) {
            self.events.borrow_mut().push((entity.borrow().slot, old_sequence));
        }

        fn respawn(&self, entity: &EntityRef) {
            self.respawns.borrow_mut().push(entity.borrow().slot);
        }

        fn append_console_command(&self, command: &str) {
            self.console.borrow_mut().push(command.to_string());
        }

        fn is_door_trigger(&self, _entity: &EntityRef) -> bool {
            self.door_trigger.get()
        }

        fn bot_test_aas(&self, origin: Vec3) {
            self.aas.borrow_mut().push(origin);
        }
    }

    fn think_fixture(product: Product) -> (ClientThinkRuntime, Rc<StubThinkHost>) {
        let (pool, combat, items, effects) = effects_fixture(product);
        let host = Rc::new(StubThinkHost {
            pool: pool.clone(),
            world: Rc::new(StubWorld::new()),
            touches: Rc::new(StubTouches {
                native: RefCell::new(HashMap::new()),
                triggers: RefCell::new(Vec::new()),
                calls: RefCell::new(Vec::new()),
            }),
            effects,
            items,
            frame: Cell::new(ClientThinkFrame {
                time: 1000,
                intermission_time: 0,
                intermission_queued: 0,
            }),
            settings: RefCell::new(think_settings()),
            moves: RefCell::new(Vec::new()),
            movement: RefCell::new(movement_result()),
            pmove_msec: Cell::new(8),
            intermissions: RefCell::new(Vec::new()),
            spectators: RefCell::new(Vec::new()),
            inactivity: Cell::new(true),
            hooks: RefCell::new(Vec::new()),
            gauntlet: Cell::new(false),
            events: RefCell::new(Vec::new()),
            respawns: RefCell::new(Vec::new()),
            console: RefCell::new(Vec::new()),
            door_trigger: Cell::new(false),
            aas: RefCell::new(Vec::new()),
        });
        let _ = combat;
        (ClientThinkRuntime::new(host.clone()), host)
    }

    #[test]
    fn think_runs_moves_and_branches() {
        let (runtime, host) = think_fixture(Product::BaseQ3);
        let entity = host.pool.at(0);
        let client = host.pool.client_at(0);
        client.borrow_mut().pers.connected = connection_state::CONNECTED;
        client.borrow_mut().ps.set_health(100);
        client.borrow_mut().pers.cmd.server_time = 100;
        runtime.client_think_real(&entity);
        assert_eq!(host.moves.borrow().as_slice(), [0]);
        assert_eq!(client.borrow().ps.pm_type, move_type::NORMAL);
        assert_eq!(client.borrow().ps.gravity, 800);
        assert_eq!(entity.borrow().r.mins, vec3(-15.0, -15.0, -24.0));
        assert_eq!(host.events.borrow().as_slice(), [(0, 0)]);
        assert!(host.world.links.borrow().contains(&0));

        client.borrow_mut().ps.set_health(0);
        client.borrow_mut().respawn_time = 500;
        runtime.client_think_real(&entity);
        assert_eq!(client.borrow().ps.pm_type, move_type::DEAD);

        client.borrow_mut().sess.session_team = team::SPECTATOR;
        client.borrow_mut().sess.spectator_state = spectator_state::FREE;
        runtime.client_think_real(&entity);
        assert_eq!(host.spectators.borrow().as_slice(), [0]);

        client.borrow_mut().sess.session_team = team::FREE;
        host.frame.set(ClientThinkFrame {
            time: 1000,
            intermission_time: 900,
            intermission_queued: 0,
        });
        runtime.client_think_real(&entity);
        assert_eq!(host.intermissions.borrow().as_slice(), [0]);
    }

    #[test]
    fn think_triggers_and_invulnerability() {
        let (runtime, host) = think_fixture(Product::MissionPack);
        let entity = host.pool.at(0);
        let client = host.pool.client_at(0);
        client.borrow_mut().ps.set_health(100);
        let item = host.pool.spawn();
        item.borrow_mut().s.e_type = entity_type::ITEM;
        item.borrow_mut().r.contents = 0x40000000;
        item.borrow_mut().touch = Some(Rc::new(|_, _, _| {}));
        host.world.area.borrow_mut().push(item.borrow().actor.clone());
        host.touches
            .native
            .borrow_mut()
            .insert(item.borrow().actor.clone(), item.clone());
        runtime.touch_triggers(&entity);
        assert_eq!(host.touches.calls.borrow().len(), 1);

        client
            .borrow_mut()
            .ps
            .powerups
            .set(powerup::INVULNERABILITY as usize, 99999);
        expand_q3_invulnerability(&host.pool, host.world.as_ref(), &entity);
        assert_ne!(client.borrow().ps.pm_flags & move_flags::INVULEXPAND, 0);
        assert_eq!(host.world.links.borrow().len(), 2);
    }

    struct StubTargets {
        calls: RefCell<Vec<(Option<usize>, bool)>>,
    }

    impl TargetsHost for StubTargets {
        fn use_targets(&self, used: Option<&EntityRef>, activator: Option<&DamageParticipant>) {
            self.calls
                .borrow_mut()
                .push((used.map(|entity| entity.borrow().slot), activator.is_some()));
        }
    }

    struct StubSpawnHost {
        pool: PoolRef,
        world: Rc<StubWorld>,
        is_player: Cell<bool>,
        random: GameRandom,
        think: Rc<ClientThinkRuntime>,
        frame: Cell<ClientSpawnFrame>,
        command: RefCell<UserCommand>,
        handicap: RefCell<String>,
        intermission: Cell<SpawnPose>,
        moved: RefCell<Vec<usize>>,
        killed: RefCell<Vec<usize>>,
        player_die_cb: DieCallback,
        body_die_cb: DieCallback,
        selected: Cell<bool>,
        selected_calls: RefCell<Vec<usize>>,
        effects: EffectsRef,
        targets: Rc<StubTargets>,
    }

    impl ClientSpawnHost for StubSpawnHost {
        fn pool(&self) -> PoolRef {
            self.pool.clone()
        }

        fn world(&self) -> WorldRef {
            self.world.clone()
        }

        fn is_player(&self, _actor: &ActorId) -> bool {
            self.is_player.get()
        }

        fn random(&self) -> &GameRandom {
            &self.random
        }

        fn think_runtime(&self) -> Rc<ClientThinkRuntime> {
            self.think.clone()
        }

        fn frame(&self) -> ClientSpawnFrame {
            self.frame.get()
        }

        fn user_command(&self, _client_num: usize) -> UserCommand {
            *self.command.borrow()
        }

        fn handicap(&self, _client_num: usize) -> String {
            self.handicap.borrow().clone()
        }

        fn find_intermission_point(&self) -> SpawnPose {
            self.intermission.get()
        }

        fn move_to_intermission(&self, entity: &EntityRef) {
            self.moved.borrow_mut().push(entity.borrow().slot);
        }

        fn kill_box(&self, entity: &EntityRef) {
            self.killed.borrow_mut().push(entity.borrow().slot);
        }

        fn player_die(&self) -> DieCallback {
            self.player_die_cb.clone()
        }

        fn body_die(&self) -> DieCallback {
            self.body_die_cb.clone()
        }

        fn has_selected_player(&self) -> bool {
            self.selected.get()
        }

        fn selected_player(&self, entity: &EntityRef, _pose: &SpawnPose) {
            self.selected_calls.borrow_mut().push(entity.borrow().slot);
        }

        fn effects(&self) -> EffectsRef {
            self.effects.clone()
        }

        fn targets(&self) -> Rc<dyn TargetsHost> {
            self.targets.clone()
        }
    }

    fn spawn_fixture() -> (ClientSpawnRuntime, Rc<StubSpawnHost>, Rc<StubThinkHost>) {
        let (pool, _combat, items, effects) = effects_fixture(Product::BaseQ3);
        let world = Rc::new(StubWorld::new());
        let think_host = Rc::new(StubThinkHost {
            pool: pool.clone(),
            world: world.clone(),
            touches: Rc::new(StubTouches {
                native: RefCell::new(HashMap::new()),
                triggers: RefCell::new(Vec::new()),
                calls: RefCell::new(Vec::new()),
            }),
            effects: effects.clone(),
            items,
            frame: Cell::new(ClientThinkFrame {
                time: 1000,
                intermission_time: 0,
                intermission_queued: 0,
            }),
            settings: RefCell::new(think_settings()),
            moves: RefCell::new(Vec::new()),
            movement: RefCell::new(movement_result()),
            pmove_msec: Cell::new(8),
            intermissions: RefCell::new(Vec::new()),
            spectators: RefCell::new(Vec::new()),
            inactivity: Cell::new(true),
            hooks: RefCell::new(Vec::new()),
            gauntlet: Cell::new(false),
            events: RefCell::new(Vec::new()),
            respawns: RefCell::new(Vec::new()),
            console: RefCell::new(Vec::new()),
            door_trigger: Cell::new(false),
            aas: RefCell::new(Vec::new()),
        });
        let think = Rc::new(ClientThinkRuntime::new(think_host.clone()));
        let host = Rc::new(StubSpawnHost {
            pool: pool.clone(),
            world,
            is_player: Cell::new(false),
            random: GameRandom::new(0),
            think,
            frame: Cell::new(ClientSpawnFrame {
                time: 1000,
                game_type: game_type::FFA,
                inactivity_seconds: 0,
                intermission_time: 0,
            }),
            command: RefCell::new(UserCommand::default()),
            handicap: RefCell::new(String::new()),
            intermission: Cell::new(SpawnPose {
                origin: vec3(0.0, 0.0, 0.0),
                angles: vec3(0.0, 0.0, 0.0),
            }),
            moved: RefCell::new(Vec::new()),
            killed: RefCell::new(Vec::new()),
            player_die_cb: Rc::new(|_, _, _, _, _| {}),
            body_die_cb: Rc::new(|_, _, _, _, _| {}),
            selected: Cell::new(false),
            selected_calls: RefCell::new(Vec::new()),
            effects: effects.clone() as EffectsRef,
            targets: Rc::new(StubTargets {
                calls: RefCell::new(Vec::new()),
            }),
        });
        let runtime = ClientSpawnRuntime::new(host.clone(), ClientSpawnState::default());
        (runtime, host, think_host)
    }

    fn deathmatch_point(host: &StubSpawnHost, origin: Vec3) -> EntityRef {
        let entity = host.pool.spawn();
        entity
            .borrow_mut()
            .set_classname(Some("info_player_deathmatch".to_string()));
        entity.borrow_mut().s.origin = origin;
        entity
    }

    #[test]
    fn spawn_view_angle_and_selection() {
        let (runtime, host, _) = spawn_fixture();
        let entity = host.pool.at(0);
        set_client_view_angle(&entity, vec3(0.0, 90.0, 0.0));
        assert_eq!(host.pool.client_at(0).borrow().ps.delta_angles.y, 16384.0);
        assert_eq!(host.pool.client_at(0).borrow().ps.viewangles, vec3(0.0, 90.0, 0.0));

        let near = deathmatch_point(&host, vec3(0.0, 0.0, 0.0));
        deathmatch_point(&host, vec3(100.0, 0.0, 0.0));
        let far = deathmatch_point(&host, vec3(200.0, 0.0, 0.0));
        host.is_player.set(true);
        host.world.area.borrow_mut().push(near.borrow().actor.clone());
        assert!(runtime.spot_would_telefrag(&near));
        host.world.area.borrow_mut().clear();
        assert!(!runtime.spot_would_telefrag(&near));
        let selected = runtime.select_spawn_point(vec3(0.0, 0.0, 0.0));
        assert_eq!(selected.origin.x, 200.0);
        assert_eq!(selected.origin.z, 9.0);
        assert!(Rc::ptr_eq(&selected.entity, &far));
        let nearest = runtime
            .select_nearest_deathmatch_spawn_point(vec3(90.0, 0.0, 0.0))
            .unwrap();
        assert_eq!(nearest.borrow().s.origin.x, 100.0);
    }

    #[test]
    fn spawn_body_queue_rotates() {
        let (runtime, host, _) = spawn_fixture();
        runtime.init_body_queue();
        let entity = host.pool.at(0);
        entity.borrow_mut().inuse = true;
        entity.borrow_mut().health = 100;
        let first = runtime.copy_to_body_queue(&entity).unwrap();
        assert_eq!(first.borrow().s.e_flags, 1);
        assert_eq!(first.borrow().s.number, first.borrow().slot as i32);
        let second = runtime.copy_to_body_queue(&entity).unwrap();
        assert!(!Rc::ptr_eq(&first, &second));
        let saved = runtime.capture_save_state();
        runtime.restore_save_state(&saved);
    }

    #[test]
    fn spawn_client_runs_full_path() {
        let (runtime, host, think) = spawn_fixture();
        deathmatch_point(&host, vec3(10.0, 20.0, 30.0));
        let entity = host.pool.at(0);
        entity.borrow_mut().inuse = true;
        host.pool.client_at(0).borrow_mut().pers.connected = connection_state::CONNECTED;
        runtime.client_spawn(&entity);
        let client = host.pool.client_at(0);
        assert_eq!(client.borrow().pers.team_state.state, team_state::ACTIVE);
        assert_eq!(client.borrow().ps.weapon, weapon::MACHINEGUN);
        assert_eq!(entity.borrow().health, 125);
        assert_ne!(client.borrow().ps.pm_flags & move_flags::RESPAWNED, 0);
        assert_eq!(client.borrow().ps.command_time, 900);
        assert_eq!(client.borrow().last_cmd_time, 1000);
        assert_eq!(host.killed.borrow().as_slice(), [0]);
        assert!(!host.targets.calls.borrow().is_empty());
        assert!(think.world.links.borrow().contains(&0));
    }

    struct StubImports {
        commands: RefCell<Vec<(i32, String)>>,
        configstrings: RefCell<HashMap<i32, String>>,
        console: RefCell<Vec<String>>,
        cvars: RefCell<HashMap<String, String>>,
        userinfo: RefCell<HashMap<usize, String>>,
        logs: RefCell<Vec<String>>,
        prints: RefCell<Vec<String>>,
    }

    impl CommandImports for StubImports {
        fn send_server_command(&self, client_num: i32, text: &str) {
            self.commands.borrow_mut().push((client_num, text.to_string()));
        }

        fn set_configstring(&self, index: i32, text: &str) {
            self.configstrings.borrow_mut().insert(index, text.to_string());
        }

        fn append_console_command(&self, text: &str) {
            self.console.borrow_mut().push(text.to_string());
        }

        fn get_cvar(&self, name: &str) -> String {
            self.cvars.borrow().get(name).cloned().unwrap_or_default()
        }

        fn get_userinfo(&self, client_num: usize) -> String {
            self.userinfo.borrow().get(&client_num).cloned().unwrap_or_default()
        }

        fn set_userinfo(&self, client_num: usize, text: &str) {
            self.userinfo.borrow_mut().insert(client_num, text.to_string());
        }

        fn log(&self, text: &str) {
            self.logs.borrow_mut().push(text.to_string());
        }

        fn print(&self, text: &str) {
            self.prints.borrow_mut().push(text.to_string());
        }
    }

    struct StubDeath {
        dies: RefCell<Vec<(usize, i32, i32)>>,
        tossed: RefCell<Vec<usize>>,
        tossed_pp: RefCell<Vec<usize>>,
        tossed_cubes: RefCell<Vec<usize>>,
    }

    impl DeathHost for StubDeath {
        fn player_die(
            &self,
            target: &EntityRef,
            _inflictor: Option<&DamageParticipant>,
            _attacker: Option<&DamageParticipant>,
            damage: i32,
            method: i32,
        ) {
            self.dies.borrow_mut().push((target.borrow().slot, damage, method));
        }

        fn toss_client_items(&self, entity: &EntityRef) {
            self.tossed.borrow_mut().push(entity.borrow().slot);
        }

        fn toss_client_persistant_powerups(&self, entity: &EntityRef) {
            self.tossed_pp.borrow_mut().push(entity.borrow().slot);
        }

        fn toss_client_cubes(&self, entity: &EntityRef) {
            self.tossed_cubes.borrow_mut().push(entity.borrow().slot);
        }
    }

    struct StubCommandHost {
        pool: PoolRef,
        match_state: MatchStateRef,
        team_scores: SharedSlots,
        settings: RefCell<CommandSettings>,
        imports: Rc<StubImports>,
        locations: RefCell<HashMap<usize, String>>,
        death: Rc<StubDeath>,
        bodies: RefCell<Vec<usize>>,
        begins: RefCell<Vec<usize>>,
        userinfos: RefCell<Vec<usize>>,
        intermission: Cell<bool>,
        leaders: RefCell<Vec<(i32, usize)>>,
        checked_leaders: RefCell<Vec<i32>>,
        items: Rc<StubItems>,
        teleport: Rc<StubTeleport>,
    }

    impl GameCommandHost for StubCommandHost {
        fn pool(&self) -> PoolRef {
            self.pool.clone()
        }

        fn match_state(&self) -> &MatchStateRef {
            &self.match_state
        }

        fn team_scores(&self) -> SharedSlots {
            self.team_scores.clone()
        }

        fn settings(&self) -> CommandSettings {
            self.settings.borrow().clone()
        }

        fn imports(&self) -> Rc<dyn CommandImports> {
            self.imports.clone()
        }

        fn team_location_message(&self, entity: &EntityRef, _capacity: usize) -> Option<String> {
            self.locations.borrow().get(&entity.borrow().slot).cloned()
        }

        fn death(&self) -> Rc<dyn DeathHost> {
            self.death.clone()
        }

        fn copy_to_body_queue(&self, entity: &EntityRef) -> Option<EntityRef> {
            self.bodies.borrow_mut().push(entity.borrow().slot);
            None
        }

        fn admission_begin(&self, client_num: usize) {
            self.begins.borrow_mut().push(client_num);
        }

        fn admission_userinfo_changed(&self, client_num: usize) {
            self.userinfos.borrow_mut().push(client_num);
        }

        fn match_begin_intermission(&self) {
            self.intermission.set(true);
        }

        fn match_set_leader(&self, team_code: i32, client_num: usize) {
            self.leaders.borrow_mut().push((team_code, client_num));
        }

        fn match_check_team_leader(&self, team_code: i32) {
            self.checked_leaders.borrow_mut().push(team_code);
        }

        fn items(&self) -> Rc<dyn ItemHost> {
            self.items.clone()
        }

        fn teleport(&self) -> Rc<dyn TeleportHost> {
            self.teleport.clone()
        }
    }

    fn command_fixture() -> (GameCommandRuntime, Rc<StubCommandHost>) {
        let time = Rc::new(Cell::new(0));
        let log = Rc::new(RefCell::new(Vec::new()));
        let pool = test_pool(Product::BaseQ3, 4, time, log);
        let host = Rc::new(StubCommandHost {
            pool,
            match_state: Rc::new(RefCell::new(MatchState::default())),
            team_scores: Rc::new(SlotArray::new(4)),
            settings: RefCell::new(CommandSettings {
                game_type: game_type::FFA,
                cheats: true,
                team_force_balance: false,
                max_game_clients: 0,
                dedicated: false,
                allow_vote: true,
            }),
            imports: Rc::new(StubImports {
                commands: RefCell::new(Vec::new()),
                configstrings: RefCell::new(HashMap::new()),
                console: RefCell::new(Vec::new()),
                cvars: RefCell::new(HashMap::new()),
                userinfo: RefCell::new(HashMap::new()),
                logs: RefCell::new(Vec::new()),
                prints: RefCell::new(Vec::new()),
            }),
            locations: RefCell::new(HashMap::new()),
            death: Rc::new(StubDeath {
                dies: RefCell::new(Vec::new()),
                tossed: RefCell::new(Vec::new()),
                tossed_pp: RefCell::new(Vec::new()),
                tossed_cubes: RefCell::new(Vec::new()),
            }),
            bodies: RefCell::new(Vec::new()),
            begins: RefCell::new(Vec::new()),
            userinfos: RefCell::new(Vec::new()),
            intermission: Cell::new(false),
            leaders: RefCell::new(Vec::new()),
            checked_leaders: RefCell::new(Vec::new()),
            items: Rc::new(StubItems::new()),
            teleport: Rc::new(StubTeleport {
                calls: RefCell::new(Vec::new()),
            }),
        });
        let runtime = GameCommandRuntime::new(host.clone());
        (runtime, host)
    }

    #[test]
    fn command_argument_helpers() {
        let args = CommandArguments::new(&argv(&["say", "hello", "world"]));
        assert_eq!(args.concat(1), "hello world");
        assert_eq!(args.at(9, 64), "");
        assert_eq!(concat_command_args(&argv(&["say", "a", "b"]), 1), "a b");
        assert_eq!(sanitize("A\x1b[B"), "ab");
        assert_eq!(clean_name("^1Bob^^"), "bob^^");
    }

    #[test]
    fn command_dispatch_score_team_vote_kill() {
        let (runtime, host) = command_fixture();
        host.pool.at(0).borrow_mut().inuse = true;
        host.pool.client_at(0).borrow_mut().pers.connected = connection_state::CONNECTED;
        host.pool.client_at(0).borrow_mut().pers.netname = "bob".to_string();
        host.match_state.borrow_mut().num_connected_clients = 1;
        host.match_state.borrow_mut().sorted_clients[0] = 0;
        runtime.dispatch(0, &argv(&["score"]));
        assert!(host
            .imports
            .commands
            .borrow()
            .iter()
            .any(|(_, text)| text.starts_with("scores 1 0 0")));
        runtime.dispatch(0, &argv(&["frobnicate"]));
        assert!(host
            .imports
            .commands
            .borrow()
            .iter()
            .any(|(_, text)| text.contains("unknown cmd frobnicate")));

        host.pool.at(0).borrow_mut().health = 100;
        runtime.dispatch(0, &argv(&["kill"]));
        assert_eq!(host.pool.at(0).borrow().health, -999);
        assert_eq!(host.death.dies.borrow().as_slice(), [(0, 100_000, 20)]);

        host.match_state.borrow_mut().time = 5000;
        runtime.dispatch(0, &argv(&["callvote", "map", "q3dm1"]));
        assert_eq!(host.match_state.borrow().vote.time, 5000);
        assert_eq!(host.match_state.borrow().vote.string, "map q3dm1");
        assert_eq!(host.imports.configstrings.borrow().get(&10), Some(&"1".to_string()));
        host.pool.at(1).borrow_mut().inuse = true;
        host.pool.client_at(1).borrow_mut().pers.connected = connection_state::CONNECTED;
        runtime.dispatch(1, &argv(&["vote", "yes"]));
        assert_eq!(host.match_state.borrow().vote.yes, 2);
        runtime.dispatch(1, &argv(&["vote", "no"]));
        assert!(host
            .imports
            .commands
            .borrow()
            .iter()
            .any(|(_, text)| text.contains("already cast")));
    }

    #[test]
    fn command_set_team_give_say_task() {
        let (runtime, host) = command_fixture();
        host.pool.at(0).borrow_mut().inuse = true;
        host.pool.at(0).borrow_mut().health = 100;
        host.pool.client_at(0).borrow_mut().pers.connected = connection_state::CONNECTED;
        runtime.set_team(&host.pool.at(0), "spectator");
        assert_eq!(host.pool.client_at(0).borrow().sess.session_team, team::SPECTATOR);
        assert_eq!(host.death.dies.borrow().len(), 1);
        assert_eq!(host.begins.borrow().as_slice(), [0]);
        assert_eq!(host.userinfos.borrow().as_slice(), [0]);

        host.pool.at(0).borrow_mut().health = 100;
        runtime.dispatch(0, &argv(&["give", "all"]));
        let schema = stat_schema(Product::BaseQ3);
        assert_eq!(host.pool.client_at(0).borrow().ps.stats.get(schema.weapons), 1022);
        assert_eq!(host.pool.client_at(0).borrow().ps.ammo.get(2), 999);
        assert_eq!(host.pool.client_at(0).borrow().ps.stats.get(schema.armor), 200);

        host.settings.borrow_mut().dedicated = true;
        runtime.dispatch(0, &argv(&["say", "hi"]));
        assert!(host.imports.prints.borrow().iter().any(|line| line.contains("hi")));

        runtime.dispatch(0, &argv(&["teamtask", "3"]));
        assert!(host
            .imports
            .userinfo
            .borrow()
            .get(&0)
            .map(|info| info.contains("teamtask"))
            .unwrap_or(false));
        assert!(host.userinfos.borrow().contains(&0));
    }

    struct StubAdmissionCommands {
        teams: RefCell<Vec<(usize, i32)>>,
        stops: RefCell<Vec<usize>>,
    }

    impl ClientAdmissionCommands for StubAdmissionCommands {
        fn broadcast_team_change(&self, client_num: usize, old_team: i32) {
            self.teams.borrow_mut().push((client_num, old_team));
        }

        fn stop_following(&self, entity: &EntityRef) {
            self.stops.borrow_mut().push(entity.borrow().slot);
        }
    }

    struct StubAdmissionHost {
        product: Product,
        pool: PoolRef,
        team_scores: SharedSlots,
        world: Rc<StubWorld>,
        match_state: MatchStateRef,
        new_session: Cell<bool>,
        session: Rc<GameSessionManager>,
        spawns: RefCell<Vec<usize>>,
        death: Rc<StubDeath>,
        ranks: Cell<i32>,
        commands: Rc<StubAdmissionCommands>,
        settings: RefCell<ClientAdmissionSettings>,
        userinfo: RefCell<HashMap<usize, String>>,
        configstrings: RefCell<HashMap<i32, String>>,
        commands_log: RefCell<Vec<(i32, String)>>,
        logs: RefCell<Vec<String>>,
        banned: Cell<bool>,
    }

    impl ClientAdmissionHost for StubAdmissionHost {
        fn product(&self) -> Product {
            self.product
        }

        fn pool(&self) -> PoolRef {
            self.pool.clone()
        }

        fn team_scores(&self) -> SharedSlots {
            self.team_scores.clone()
        }

        fn world(&self) -> WorldRef {
            self.world.clone()
        }

        fn match_state(&self) -> MatchStateRef {
            self.match_state.clone()
        }

        fn new_session(&self) -> bool {
            self.new_session.get()
        }

        fn session(&self) -> Rc<GameSessionManager> {
            self.session.clone()
        }

        fn spawn_client(&self, entity: &EntityRef) {
            self.spawns.borrow_mut().push(entity.borrow().slot);
        }

        fn death(&self) -> Rc<dyn DeathHost> {
            self.death.clone()
        }

        fn calculate_ranks(&self) {
            self.ranks.set(self.ranks.get() + 1);
        }

        fn commands(&self) -> Rc<dyn ClientAdmissionCommands> {
            self.commands.clone()
        }

        fn bots(&self) -> ClientBotServices {
            ClientBotServices::Available {
                remove_queued_begin: Rc::new(|_| {}),
                connect: Rc::new(|_, _| true),
                shutdown_client: Rc::new(|_, _| {}),
            }
        }

        fn settings(&self) -> ClientAdmissionSettings {
            self.settings.borrow().clone()
        }

        fn get_userinfo(&self, client_num: usize) -> String {
            self.userinfo.borrow().get(&client_num).cloned().unwrap_or_default()
        }

        fn set_configstring(&self, index: i32, value: &str) {
            self.configstrings.borrow_mut().insert(index, value.to_string());
        }

        fn send_server_command(&self, client_num: i32, value: &str) {
            self.commands_log.borrow_mut().push((client_num, value.to_string()));
        }

        fn log(&self, value: &str) {
            self.logs.borrow_mut().push(value.to_string());
        }

        fn filter_packet(&self, _address: &str) -> bool {
            self.banned.get()
        }
    }

    fn admission_fixture() -> (ClientAdmissionRuntime, Rc<StubAdmissionHost>, Rc<StubSessionCvars>) {
        let time = Rc::new(Cell::new(0));
        let log = Rc::new(RefCell::new(Vec::new()));
        let pool = test_pool(Product::BaseQ3, 4, time, log);
        let world = Rc::new(SessionWorld {
            clients: pool.clients()[..4].to_vec(),
            max_clients: 4,
            team_scores: Rc::new(SlotArray::new(4)),
            game_type: Cell::new(game_type::FFA),
            team_auto_join: Cell::new(false),
            max_game_clients: Cell::new(0),
            time: Cell::new(0),
            num_non_spectator_clients: Cell::new(0),
            new_session: Cell::new(false),
        });
        let cvars = Rc::new(StubSessionCvars {
            values: RefCell::new(HashMap::new()),
        });
        let session = Rc::new(GameSessionManager::new(
            world,
            Rc::new(StubSessionServices {
                prints: RefCell::new(Vec::new()),
                teams: RefCell::new(Vec::new()),
            }),
            cvars.clone(),
        ));
        let host = Rc::new(StubAdmissionHost {
            product: Product::BaseQ3,
            pool: pool.clone(),
            team_scores: Rc::new(SlotArray::new(4)),
            world: Rc::new(StubWorld::new()),
            match_state: Rc::new(RefCell::new(MatchState::default())),
            new_session: Cell::new(true),
            session,
            spawns: RefCell::new(Vec::new()),
            death: Rc::new(StubDeath {
                dies: RefCell::new(Vec::new()),
                tossed: RefCell::new(Vec::new()),
                tossed_pp: RefCell::new(Vec::new()),
                tossed_cubes: RefCell::new(Vec::new()),
            }),
            ranks: Cell::new(0),
            commands: Rc::new(StubAdmissionCommands {
                teams: RefCell::new(Vec::new()),
                stops: RefCell::new(Vec::new()),
            }),
            settings: RefCell::new(ClientAdmissionSettings {
                game_type: game_type::FFA,
                password: String::new(),
            }),
            userinfo: RefCell::new(HashMap::new()),
            configstrings: RefCell::new(HashMap::new()),
            commands_log: RefCell::new(Vec::new()),
            logs: RefCell::new(Vec::new()),
            banned: Cell::new(false),
        });
        (ClientAdmissionRuntime::new(host.clone()), host, cvars)
    }

    #[test]
    fn admission_connect_begin_disconnect() {
        let (runtime, host, cvars) = admission_fixture();
        host.userinfo.borrow_mut().insert(0, "\\name\\bob".to_string());
        host.banned.set(true);
        assert_eq!(
            runtime.connect(0, true, false),
            Some("You are banned from this server.".to_string())
        );
        host.banned.set(false);
        host.settings.borrow_mut().password = "pw".to_string();
        assert_eq!(runtime.connect(0, true, false), Some("Invalid password".to_string()));
        host.settings.borrow_mut().password = String::new();

        assert_eq!(runtime.connect(0, true, false), None);
        assert_eq!(
            host.pool.client_at(0).borrow().pers.connected,
            connection_state::CONNECTING
        );
        assert_eq!(host.pool.client_at(0).borrow().sess.session_team, team::FREE);
        assert!(cvars.values.borrow().contains_key("session0"));
        assert!(host.logs.borrow().iter().any(|line| line.contains("ClientConnect")));

        runtime.begin(0);
        assert_eq!(
            host.pool.client_at(0).borrow().pers.connected,
            connection_state::CONNECTED
        );
        assert_eq!(host.spawns.borrow().as_slice(), [0]);
        assert!(host.logs.borrow().iter().any(|line| line.contains("ClientBegin")));

        host.pool.at(0).borrow_mut().inuse = true;
        host.pool.client_at(0).borrow_mut().ps.set_health(100);
        runtime.disconnect(0);
        assert_eq!(
            host.pool.client_at(0).borrow().pers.connected,
            connection_state::DISCONNECTED
        );
        assert_eq!(host.configstrings.borrow().get(&544), Some(&String::new()));
        assert!(host.logs.borrow().iter().any(|line| line.contains("ClientDisconnect")));
        assert_eq!(host.death.tossed.borrow().as_slice(), [0]);
    }

    struct StubConfig {
        indexes: RefCell<Vec<String>>,
    }

    impl ConfigStrings for StubConfig {
        fn model_index(&self, name: &str) -> i32 {
            self.indexes.borrow_mut().push(name.to_string());
            7
        }
    }

    struct StubArenaHost {
        match_runtime: Rc<MatchRuntime>,
        world: Rc<StubWorld>,
        cvars: Rc<StubCvars>,
        config: Rc<StubConfig>,
    }

    impl ArenaHost for StubArenaHost {
        fn match_runtime(&self) -> Rc<MatchRuntime> {
            self.match_runtime.clone()
        }

        fn world(&self) -> WorldRef {
            self.world.clone()
        }

        fn cvars(&self) -> Rc<dyn CvarRegistry> {
            self.cvars.clone()
        }

        fn config(&self) -> Rc<dyn ConfigStrings> {
            self.config.clone()
        }
    }

    #[test]
    fn arena_postgame_podium_abort() {
        let (match_runtime, match_host) = match_fixture(game_type::FFA);
        connect_client(&match_host, 0, team::FREE, 5);
        connect_client(&match_host, 1, team::FREE, 9);
        match_host.pool.at(1).borrow_mut().r.sv_flags |= server_entity_flags::BOT;
        match_runtime.calculate_ranks();
        let host = Rc::new(StubArenaHost {
            match_runtime: match_runtime.clone(),
            world: Rc::new(StubWorld::new()),
            cvars: Rc::new(StubCvars {
                values: RefCell::new(HashMap::from([
                    ("g_podiumDist".to_string(), ("80".to_string(), 80)),
                    ("g_podiumDrop".to_string(), ("70".to_string(), 70)),
                ])),
                sets: RefCell::new(Vec::new()),
            }),
            config: Rc::new(StubConfig {
                indexes: RefCell::new(Vec::new()),
            }),
        });
        let runtime = ArenaRuntime::new(host.clone());
        runtime.update_tournament_info();
        assert!(match_host.log.borrow().iter().any(|line| line.contains("postgame 2 0")));
        runtime.spawn_models_on_victory_pads();
        assert!(host.config.indexes.borrow().iter().any(|name| name.contains("podium4")));
        let saved = runtime.capture_save_state();
        runtime.restore_save_state(&saved);
        let before = host.world.links.borrow().len();
        runtime.abort_podium();
        assert_eq!(host.world.links.borrow().len(), before);
        match_host.settings.borrow_mut().game_type = game_type::SINGLE_PLAYER;
        runtime.abort_podium();
    }
}
