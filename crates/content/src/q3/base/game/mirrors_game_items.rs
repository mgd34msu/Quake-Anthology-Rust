//! Quake III base game items (`q3_game_items`) support: shared mirrors, group error, and tests.
//!
//! Self-containment mirrors: minimal local copies of items the donors import from
//! modules outside this port (sibling q3 donors, engine contracts, and math/text
//! helpers), plus the group error type. Sibling-owned mirrors carry SIBLING-MIRROR
//! notes and unify with the canonical ports at merge time.

use qa_core::math::{add3, dot3, scale3, vec3, Bounds, Vec3};
use qa_core::numeric::qvm_float_to_int;
use std::collections::HashSet;
use thiserror::Error;

// Intra-group imports: sibling modules split from the same flat port.

/// Game-items failure: range/invalid/missing map the donor `RangeError` and
/// `Error`; `Drop` maps the donor `CommonError("drop", ...)`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Q3GameItemsError {
    /// Value outside its valid range (donor `RangeError`).
    #[error("range error: {0}")]
    Range(String),
    /// Invalid state or argument (donor `Error`).
    #[error("invalid: {0}")]
    Invalid(String),
    /// Missing required record (donor `Error` on absent lookups).
    #[error("missing: {0}")]
    Missing(String),
    /// Fatal game drop (donor `CommonError("drop", ...)`).
    #[error("drop: {0}")]
    Drop(String),
}

/// Game-items result.
pub type Q3GameItemsResult<T> = Result<T, Q3GameItemsError>;

pub(crate) fn range(message: impl Into<String>) -> Q3GameItemsError {
    Q3GameItemsError::Range(message.into())
}

pub(crate) fn invalid(message: impl Into<String>) -> Q3GameItemsError {
    Q3GameItemsError::Invalid(message.into())
}

pub(crate) fn missing(message: impl Into<String>) -> Q3GameItemsError {
    Q3GameItemsError::Missing(message.into())
}

pub(crate) fn drop_error(message: impl Into<String>) -> Q3GameItemsError {
    Q3GameItemsError::Drop(message.into())
}

/// Quake III product table selector (`Product`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Product {
    /// `baseq3`.
    Baseq3,
    /// `missionpack`.
    Missionpack,
}

impl Product {
    /// Donor product name.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Product::Baseq3 => "baseq3",
            Product::Missionpack => "missionpack",
        }
    }
}

/// Maximum clients (`MAX_CLIENTS`).
pub const MAX_CLIENTS: usize = 64;

/// Maximum entities (`MAX_GENTITIES`).
pub const MAX_GENTITIES: usize = 1024;

/// World entity number.
pub const ENTITYNUM_WORLD: usize = 1022;

/// Null entity number.
pub const ENTITYNUM_NONE: i32 = 1023;

/// Default gravity (`DEFAULT_GRAVITY`).
pub const DEFAULT_GRAVITY: f32 = 800.0;

/// Gib health threshold (`GIB_HEALTH`).
pub const GIB_HEALTH: i32 = -40;

/// Game type (`GameType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum GameType {
    /// Free-for-all.
    Ffa = 0,
    /// Tournament.
    Tournament = 1,
    /// Single player.
    SinglePlayer = 2,
    /// Team.
    Team = 3,
    /// Capture the flag.
    Ctf = 4,
    /// One-flag CTF.
    OneFctf = 5,
    /// Obelisk.
    Obelisk = 6,
    /// Harvester.
    Harvester = 7,
    /// Sentinel.
    MaxGameType = 8,
}

impl GameType {
    /// Raw value.
    #[must_use]
    pub fn raw(self) -> i32 {
        self as i32
    }
}

/// Team (`Team`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum Team {
    /// Free.
    Free = 0,
    /// Red.
    Red = 1,
    /// Blue.
    Blue = 2,
    /// Spectator.
    Spectator = 3,
    /// Sentinel.
    NumTeams = 4,
}

impl Team {
    /// Raw value.
    #[must_use]
    pub fn raw(self) -> i32 {
        self as i32
    }

    /// Parse a raw team value.
    pub fn from_raw(value: i32) -> Q3GameItemsResult<Team> {
        match value {
            0 => Ok(Team::Free),
            1 => Ok(Team::Red),
            2 => Ok(Team::Blue),
            3 => Ok(Team::Spectator),
            4 => Ok(Team::NumTeams),
            _ => Err(range(format!("team {value} outside 0..=4"))),
        }
    }
}

/// Item type (`ItemType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum ItemType {
    /// Invalid.
    Bad = 0,
    /// Weapon.
    Weapon = 1,
    /// Ammo.
    Ammo = 2,
    /// Armor.
    Armor = 3,
    /// Health.
    Health = 4,
    /// Powerup.
    Powerup = 5,
    /// Holdable.
    Holdable = 6,
    /// Persistent powerup.
    PersistantPowerup = 7,
    /// Team objective.
    Team = 8,
}

/// Entity type (`EntityType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum EntityType {
    /// General.
    General = 0,
    /// Player.
    Player = 1,
    /// Item.
    Item = 2,
    /// Missile.
    Missile = 3,
    /// Mover.
    Mover = 4,
    /// Beam.
    Beam = 5,
    /// Portal.
    Portal = 6,
    /// Speaker.
    Speaker = 7,
    /// Push trigger.
    PushTrigger = 8,
    /// Teleport trigger.
    TeleportTrigger = 9,
    /// Invisible.
    Invisible = 10,
    /// Grapple.
    Grapple = 11,
    /// Team.
    Team = 12,
    /// Events.
    Events = 13,
}

/// Weapon (`Weapon`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum Weapon {
    /// None.
    None = 0,
    /// Gauntlet.
    Gauntlet = 1,
    /// Machinegun.
    Machinegun = 2,
    /// Shotgun.
    Shotgun = 3,
    /// Grenade launcher.
    GrenadeLauncher = 4,
    /// Rocket launcher.
    RocketLauncher = 5,
    /// Lightning.
    Lightning = 6,
    /// Railgun.
    Railgun = 7,
    /// Plasmagun.
    Plasmagun = 8,
    /// BFG.
    Bfg = 9,
    /// Grappling hook.
    GrapplingHook = 10,
    /// Nailgun.
    Nailgun = 11,
    /// Proximity launcher.
    ProxLauncher = 12,
    /// Chaingun.
    Chaingun = 13,
}

impl Weapon {
    /// Raw value.
    #[must_use]
    pub fn raw(self) -> i32 {
        self as i32
    }

    /// Parse a raw weapon value.
    pub fn from_raw(value: i32) -> Q3GameItemsResult<Weapon> {
        match value {
            0 => Ok(Weapon::None),
            1 => Ok(Weapon::Gauntlet),
            2 => Ok(Weapon::Machinegun),
            3 => Ok(Weapon::Shotgun),
            4 => Ok(Weapon::GrenadeLauncher),
            5 => Ok(Weapon::RocketLauncher),
            6 => Ok(Weapon::Lightning),
            7 => Ok(Weapon::Railgun),
            8 => Ok(Weapon::Plasmagun),
            9 => Ok(Weapon::Bfg),
            10 => Ok(Weapon::GrapplingHook),
            11 => Ok(Weapon::Nailgun),
            12 => Ok(Weapon::ProxLauncher),
            13 => Ok(Weapon::Chaingun),
            _ => Err(range(format!("weapon {value} outside 0..=13"))),
        }
    }
}

/// Weapon count per product (`weaponCount`).
#[must_use]
pub fn weapon_count(product: Product) -> i32 {
    match product {
        Product::Baseq3 => 11,
        Product::Missionpack => 14,
    }
}

/// Powerup (`Powerup`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum Powerup {
    /// None.
    None = 0,
    /// Quad.
    Quad = 1,
    /// Battlesuit.
    Battlesuit = 2,
    /// Haste.
    Haste = 3,
    /// Invisibility.
    Invis = 4,
    /// Regeneration.
    Regen = 5,
    /// Flight.
    Flight = 6,
    /// Red flag.
    RedFlag = 7,
    /// Blue flag.
    BlueFlag = 8,
    /// Neutral flag.
    NeutralFlag = 9,
    /// Scout.
    Scout = 10,
    /// Guard.
    Guard = 11,
    /// Doubler.
    Doubler = 12,
    /// Ammo regeneration.
    AmmoRegen = 13,
    /// Invulnerability.
    Invulnerability = 14,
    /// Sentinel.
    NumPowerups = 15,
}

impl Powerup {
    /// Raw value.
    #[must_use]
    pub fn raw(self) -> i32 {
        self as i32
    }
}

/// Holdable (`Holdable`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum Holdable {
    /// None.
    None = 0,
    /// Teleporter.
    Teleporter = 1,
    /// Medkit.
    Medkit = 2,
    /// Kamikaze.
    Kamikaze = 3,
    /// Portal.
    Portal = 4,
    /// Invulnerability.
    Invulnerability = 5,
    /// Sentinel.
    NumHoldable = 6,
}

/// Entity event (`EntityEvent`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum EntityEvent {
    /// None.
    None = 0,
    /// Footstep.
    Footstep = 1,
    /// Metal footstep.
    FootstepMetal = 2,
    /// Footsplash.
    Footsplash = 3,
    /// Footwade.
    Footwade = 4,
    /// Swim.
    Swim = 5,
    /// Step 4.
    Step4 = 6,
    /// Step 8.
    Step8 = 7,
    /// Step 12.
    Step12 = 8,
    /// Step 16.
    Step16 = 9,
    /// Short fall.
    FallShort = 10,
    /// Medium fall.
    FallMedium = 11,
    /// Far fall.
    FallFar = 12,
    /// Jump pad.
    JumpPad = 13,
    /// Jump.
    Jump = 14,
    /// Water touch.
    WaterTouch = 15,
    /// Water leave.
    WaterLeave = 16,
    /// Water under.
    WaterUnder = 17,
    /// Water clear.
    WaterClear = 18,
    /// Item pickup.
    ItemPickup = 19,
    /// Global item pickup.
    GlobalItemPickup = 20,
    /// No ammo.
    NoAmmo = 21,
    /// Change weapon.
    ChangeWeapon = 22,
    /// Fire weapon.
    FireWeapon = 23,
    /// Use item 0.
    UseItem0 = 24,
    /// Use item 1.
    UseItem1 = 25,
    /// Use item 2.
    UseItem2 = 26,
    /// Use item 3.
    UseItem3 = 27,
    /// Use item 4.
    UseItem4 = 28,
    /// Use item 5.
    UseItem5 = 29,
    /// Use item 6.
    UseItem6 = 30,
    /// Use item 7.
    UseItem7 = 31,
    /// Use item 8.
    UseItem8 = 32,
    /// Use item 9.
    UseItem9 = 33,
    /// Use item 10.
    UseItem10 = 34,
    /// Use item 11.
    UseItem11 = 35,
    /// Use item 12.
    UseItem12 = 36,
    /// Use item 13.
    UseItem13 = 37,
    /// Use item 14.
    UseItem14 = 38,
    /// Use item 15.
    UseItem15 = 39,
    /// Item respawn.
    ItemRespawn = 40,
    /// Item pop.
    ItemPop = 41,
    /// Player teleport in.
    PlayerTeleportIn = 42,
    /// Player teleport out.
    PlayerTeleportOut = 43,
    /// Grenade bounce.
    GrenadeBounce = 44,
    /// General sound.
    GeneralSound = 45,
    /// Global sound.
    GlobalSound = 46,
    /// Global team sound.
    GlobalTeamSound = 47,
    /// Bullet hit flesh.
    BulletHitFlesh = 48,
    /// Bullet hit wall.
    BulletHitWall = 49,
    /// Missile hit.
    MissileHit = 50,
    /// Missile miss.
    MissileMiss = 51,
    /// Missile metal miss.
    MissileMissMetal = 52,
    /// Rail trail.
    RailTrail = 53,
    /// Shotgun.
    Shotgun = 54,
    /// Bullet.
    Bullet = 55,
    /// Pain.
    Pain = 56,
    /// Death 1.
    Death1 = 57,
    /// Death 2.
    Death2 = 58,
    /// Death 3.
    Death3 = 59,
    /// Obituary.
    Obituary = 60,
    /// Quad powerup.
    PowerupQuad = 61,
    /// Battlesuit powerup.
    PowerupBattlesuit = 62,
    /// Regen powerup.
    PowerupRegen = 63,
    /// Gib player.
    GibPlayer = 64,
    /// Score plum.
    ScorePlum = 65,
    /// Proximity mine stick.
    ProximityMineStick = 66,
    /// Proximity mine trigger.
    ProximityMineTrigger = 67,
    /// Kamikaze.
    Kamikaze = 68,
    /// Obelisk explode.
    ObeliskExplode = 69,
    /// Obelisk pain.
    ObeliskPain = 70,
    /// Invulnerability impact.
    InvulImpact = 71,
    /// Juiced.
    Juiced = 72,
    /// Lightning bolt.
    LightningBolt = 73,
    /// Debug line.
    DebugLine = 74,
    /// Stop looping sound.
    StopLoopingSound = 75,
    /// Taunt.
    Taunt = 76,
    /// Taunt yes.
    TauntYes = 77,
    /// Taunt no.
    TauntNo = 78,
    /// Taunt follow me.
    TauntFollowMe = 79,
    /// Taunt get flag.
    TauntGetFlag = 80,
    /// Taunt guard base.
    TauntGuardBase = 81,
    /// Taunt patrol.
    TauntPatrol = 82,
}

/// Trajectory type (`TrajectoryType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum TrajectoryType {
    /// Stationary.
    Stationary = 0,
    /// Interpolate.
    Interpolate = 1,
    /// Linear.
    Linear = 2,
    /// Linear stop.
    LinearStop = 3,
    /// Sine.
    Sine = 4,
    /// Gravity.
    Gravity = 5,
}

/// Player move flags (`MoveFlags`).
pub struct MoveFlags;

impl MoveFlags {
    /// Ducked.
    pub const DUCKED: i32 = 1;
    /// Jump held.
    pub const JUMP_HELD: i32 = 2;
    /// Backwards jump.
    pub const BACKWARDS_JUMP: i32 = 8;
    /// Backwards run.
    pub const BACKWARDS_RUN: i32 = 16;
    /// Time land.
    pub const TIME_LAND: i32 = 32;
    /// Time knockback.
    pub const TIME_KNOCKBACK: i32 = 64;
    /// Time waterjump.
    pub const TIME_WATERJUMP: i32 = 256;
    /// Respawned.
    pub const RESPAWNED: i32 = 512;
    /// Use-item held.
    pub const USE_ITEM_HELD: i32 = 1024;
    /// Grapple pull.
    pub const GRAPPLE_PULL: i32 = 2048;
    /// Follow.
    pub const FOLLOW: i32 = 4096;
    /// Scoreboard.
    pub const SCOREBOARD: i32 = 8192;
    /// Invulnerability expand.
    pub const INVUL_EXPAND: i32 = 16384;
}

/// Player move type (`MoveType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum MoveType {
    /// Normal.
    Normal = 0,
    /// Noclip.
    Noclip = 1,
    /// Spectator.
    Spectator = 2,
    /// Dead.
    Dead = 3,
    /// Freeze.
    Freeze = 4,
    /// Intermission.
    Intermission = 5,
    /// Single-player intermission.
    SpIntermission = 6,
}

/// Server entity flags (`ServerEntityFlags`).
pub struct ServerEntityFlags;

impl ServerEntityFlags {
    /// No client.
    pub const NOCLIENT: i32 = 1;
    /// Client mask.
    pub const CLIENTMASK: i32 = 2;
    /// Bot.
    pub const BOT: i32 = 8;
    /// Broadcast.
    pub const BROADCAST: i32 = 32;
    /// Portal.
    pub const PORTAL: i32 = 64;
    /// Use current origin.
    pub const USE_CURRENT_ORIGIN: i32 = 128;
    /// Single client.
    pub const SINGLECLIENT: i32 = 256;
    /// No server info.
    pub const NOSERVERINFO: i32 = 512;
    /// Not single client.
    pub const NOTSINGLECLIENT: i32 = 2048;
}

/// Game entity flags (`GameFlags`).
pub struct GameFlags;

impl GameFlags {
    /// God mode.
    pub const GODMODE: i32 = 0x10;
    /// Notarget.
    pub const NOTARGET: i32 = 0x20;
    /// Team slave.
    pub const TEAMSLAVE: i32 = 0x400;
    /// No knockback.
    pub const NO_KNOCKBACK: i32 = 0x800;
    /// Dropped item.
    pub const DROPPED_ITEM: i32 = 0x1000;
    /// No bots.
    pub const NO_BOTS: i32 = 0x2000;
    /// No humans.
    pub const NO_HUMANS: i32 = 0x4000;
    /// Force gesture.
    pub const FORCE_GESTURE: i32 = 0x8000;
}

/// Mover state (`MoverState`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum MoverState {
    /// At position 1.
    Pos1 = 0,
    /// At position 2.
    Pos2 = 1,
    /// Moving one to two.
    OneToTwo = 2,
    /// Moving two to one.
    TwoToOne = 3,
}

/// Client connection state (`ConnectionState`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum ConnectionState {
    /// Disconnected.
    Disconnected = 0,
    /// Connecting.
    Connecting = 1,
    /// Connected.
    Connected = 2,
}

/// Baseq3 stat slots (`BaseStatIndex`).
pub struct BaseStatIndex;

impl BaseStatIndex {
    /// Health.
    pub const HEALTH: usize = 0;
    /// Holdable item.
    pub const HOLDABLE_ITEM: usize = 1;
    /// Weapons.
    pub const WEAPONS: usize = 2;
    /// Armor.
    pub const ARMOR: usize = 3;
    /// Dead yaw.
    pub const DEAD_YAW: usize = 4;
    /// Clients ready.
    pub const CLIENTS_READY: usize = 5;
    /// Max health.
    pub const MAX_HEALTH: usize = 6;
}

/// Missionpack stat slots (`MissionpackStatIndex`).
pub struct MissionpackStatIndex;

impl MissionpackStatIndex {
    /// Health.
    pub const HEALTH: usize = 0;
    /// Holdable item.
    pub const HOLDABLE_ITEM: usize = 1;
    /// Persistent powerup.
    pub const PERSISTANT_POWERUP: usize = 2;
    /// Weapons.
    pub const WEAPONS: usize = 3;
    /// Armor.
    pub const ARMOR: usize = 4;
    /// Dead yaw.
    pub const DEAD_YAW: usize = 5;
    /// Clients ready.
    pub const CLIENTS_READY: usize = 6;
    /// Max health.
    pub const MAX_HEALTH: usize = 7;
}

/// Persistent slots (`PersistentIndex`).
pub struct PersistentIndex;

impl PersistentIndex {
    /// Score.
    pub const SCORE: usize = 0;
    /// Hits.
    pub const HITS: usize = 1;
    /// Rank.
    pub const RANK: usize = 2;
    /// Team.
    pub const TEAM: usize = 3;
    /// Spawn count.
    pub const SPAWN_COUNT: usize = 4;
    /// Player events.
    pub const PLAYEREVENTS: usize = 5;
    /// Attacker.
    pub const ATTACKER: usize = 6;
    /// Attackee armor.
    pub const ATTACKEE_ARMOR: usize = 7;
    /// Killed.
    pub const KILLED: usize = 8;
    /// Impressive count.
    pub const IMPRESSIVE_COUNT: usize = 9;
    /// Excellent count.
    pub const EXCELLENT_COUNT: usize = 10;
    /// Defend count.
    pub const DEFEND_COUNT: usize = 11;
    /// Assist count.
    pub const ASSIST_COUNT: usize = 12;
    /// Gauntlet frag count.
    pub const GAUNTLET_FRAG_COUNT: usize = 13;
    /// Captures.
    pub const CAPTURES: usize = 14;
}

/// Damage flags (`DamageFlags`).
pub struct DamageFlags;

impl DamageFlags {
    /// No knockback.
    pub const NO_KNOCKBACK: i32 = 0x4;
    /// No protection.
    pub const NO_PROTECTION: i32 = 0x8;
}

/// Stat slot schema (`StatSchema` + `statSchema`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatSchema {
    /// Health slot.
    pub health: usize,
    /// Holdable-item slot.
    pub holdable_item: usize,
    /// Weapons bitmask slot.
    pub weapons: usize,
    /// Armor slot.
    pub armor: usize,
    /// Dead yaw slot.
    pub dead_yaw: usize,
    /// Clients-ready slot.
    pub clients_ready: usize,
    /// Max-health slot.
    pub max_health: usize,
}

/// Stat schema for a product (`statSchema`).
#[must_use]
pub fn stat_schema(product: Product) -> StatSchema {
    match product {
        Product::Baseq3 => StatSchema {
            health: BaseStatIndex::HEALTH,
            holdable_item: BaseStatIndex::HOLDABLE_ITEM,
            weapons: BaseStatIndex::WEAPONS,
            armor: BaseStatIndex::ARMOR,
            dead_yaw: BaseStatIndex::DEAD_YAW,
            clients_ready: BaseStatIndex::CLIENTS_READY,
            max_health: BaseStatIndex::MAX_HEALTH,
        },
        Product::Missionpack => StatSchema {
            health: MissionpackStatIndex::HEALTH,
            holdable_item: MissionpackStatIndex::HOLDABLE_ITEM,
            weapons: MissionpackStatIndex::WEAPONS,
            armor: MissionpackStatIndex::ARMOR,
            dead_yaw: MissionpackStatIndex::DEAD_YAW,
            clients_ready: MissionpackStatIndex::CLIENTS_READY,
            max_health: MissionpackStatIndex::MAX_HEALTH,
        },
    }
}

/// Trajectory (`Trajectory`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Trajectory {
    /// Trajectory type.
    pub ty: TrajectoryType,
    /// Start time (ms).
    pub time: i32,
    /// Duration (ms).
    pub duration: i32,
    /// Base position.
    pub base: Vec3,
    /// Delta (velocity or amplitude).
    pub delta: Vec3,
}

impl Default for Trajectory {
    fn default() -> Self {
        Trajectory {
            ty: TrajectoryType::Stationary,
            time: 0,
            duration: 0,
            base: vec3(0.0, 0.0, 0.0),
            delta: vec3(0.0, 0.0, 0.0),
        }
    }
}

pub(crate) fn trajectory_seconds(milliseconds: i32) -> f32 {
    milliseconds as f32 * 0.001
}

pub(crate) fn periodic_radians(trajectory: &Trajectory, at_time: i32) -> f32 {
    let fraction = at_time.wrapping_sub(trajectory.time) as f32 / trajectory.duration as f32;
    fraction * std::f32::consts::PI * 2.0
}

/// Evaluate a trajectory (`evaluateTrajectory`).
pub fn evaluate_trajectory(trajectory: &Trajectory, at_time: i32) -> Vec3 {
    match trajectory.ty {
        TrajectoryType::Stationary | TrajectoryType::Interpolate => trajectory.base,
        TrajectoryType::Linear => {
            let dt = trajectory_seconds(at_time.wrapping_sub(trajectory.time));
            add3(trajectory.base, scale3(trajectory.delta, dt))
        }
        TrajectoryType::Sine => {
            let phase = (periodic_radians(trajectory, at_time) as f64).sin() as f32;
            add3(trajectory.base, scale3(trajectory.delta, phase))
        }
        TrajectoryType::LinearStop => {
            let end = trajectory.time.wrapping_add(trajectory.duration);
            let time = if at_time > end { end } else { at_time };
            let dt = trajectory_seconds(time.wrapping_sub(trajectory.time)).max(0.0);
            add3(trajectory.base, scale3(trajectory.delta, dt))
        }
        TrajectoryType::Gravity => {
            let dt = trajectory_seconds(at_time.wrapping_sub(trajectory.time));
            let result = add3(trajectory.base, scale3(trajectory.delta, dt));
            let fall = 0.5 * DEFAULT_GRAVITY * dt * dt;
            vec3(result.x, result.y, result.z - fall)
        }
    }
}

/// Evaluate a trajectory delta (`evaluateTrajectoryDelta`).
pub fn evaluate_trajectory_delta(trajectory: &Trajectory, at_time: i32) -> Vec3 {
    match trajectory.ty {
        TrajectoryType::Stationary | TrajectoryType::Interpolate => vec3(0.0, 0.0, 0.0),
        TrajectoryType::Linear => trajectory.delta,
        TrajectoryType::Sine => {
            let phase = (periodic_radians(trajectory, at_time) as f64).cos() as f32 * 0.5;
            scale3(trajectory.delta, phase)
        }
        TrajectoryType::LinearStop => {
            if at_time > trajectory.time.wrapping_add(trajectory.duration) {
                vec3(0.0, 0.0, 0.0)
            } else {
                trajectory.delta
            }
        }
        TrajectoryType::Gravity => {
            let dt = trajectory_seconds(at_time.wrapping_sub(trajectory.time));
            vec3(
                trajectory.delta.x,
                trajectory.delta.y,
                trajectory.delta.z - DEFAULT_GRAVITY * dt,
            )
        }
    }
}

/// QVM angle vectors (`qvmAngleVectors`).
#[must_use]
pub fn qvm_angle_vectors(angles: Vec3) -> qa_core::math::AngleVectors {
    const ANGLE_RADIANS: f64 = std::f64::consts::PI * 2.0 / 360.0;
    let yaw = f64::from(angles.y) * ANGLE_RADIANS;
    let pitch = f64::from(angles.x) * ANGLE_RADIANS;
    let roll = f64::from(angles.z) * ANGLE_RADIANS;
    let sy = yaw.sin() as f32;
    let cy = yaw.cos() as f32;
    let sp = pitch.sin() as f32;
    let cp = pitch.cos() as f32;
    let sr = roll.sin() as f32;
    let cr = roll.cos() as f32;
    qa_core::math::AngleVectors {
        forward: vec3(cp * cy, cp * sy, -sp),
        right: vec3(-sr * sp * cy + -cr * -sy, -sr * sp * sy + -cr * cy, -sr * cp),
        up: vec3(cr * sp * cy + -sr * -sy, cr * sp * sy + -sr * cy, cr * cp),
    }
}

/// `bg_lib` atoi (`gameAtoi`): wrapping decimal-prefix parse.
#[must_use]
pub fn game_atoi(text: &str) -> i32 {
    let bytes = text.as_bytes();
    let mut offset = 0;
    while offset < bytes.len() && matches!(bytes[offset], b'\t' | b'\n' | 0x0C | b'\r' | b' ') {
        offset += 1;
    }
    if offset >= bytes.len() || bytes[offset] == 0 {
        return 0;
    }
    let mut sign = 1i32;
    if bytes[offset] == b'+' || bytes[offset] == b'-' {
        if bytes[offset] == b'-' {
            sign = -1;
        }
        offset += 1;
    }
    let mut value = 0i32;
    while offset < bytes.len() {
        let byte = bytes[offset];
        if !byte.is_ascii_digit() {
            break;
        }
        value = value.wrapping_mul(10).wrapping_add(i32::from(byte - b'0'));
        offset += 1;
    }
    value.wrapping_mul(sign)
}

/// `bg_lib` atof (`gameAtof`): decimal prefix only, binary32 operations.
#[must_use]
pub fn game_atof(text: &str) -> f32 {
    let bytes = text.as_bytes();
    let mut offset = 0;
    while offset < bytes.len() && matches!(bytes[offset], b'\t' | b'\n' | 0x0C | b'\r' | b' ') {
        offset += 1;
    }
    if offset >= bytes.len() || bytes[offset] == 0 {
        return 0.0;
    }
    let mut sign = 1.0f32;
    if bytes[offset] == b'+' || bytes[offset] == b'-' {
        if bytes[offset] == b'-' {
            sign = -1.0;
        }
        offset += 1;
    }
    let mut value = 0.0f32;
    let mut character = if offset < bytes.len() { bytes[offset] } else { 0 };
    if character != b'.' {
        loop {
            character = if offset < bytes.len() { bytes[offset] } else { 0 };
            offset += 1;
            if !character.is_ascii_digit() {
                break;
            }
            value = value * 10.0 + f32::from(character - b'0');
        }
    } else {
        offset += 1;
    }
    if character == b'.' {
        let mut fraction = 0.1f32;
        loop {
            character = if offset < bytes.len() { bytes[offset] } else { 0 };
            offset += 1;
            if !character.is_ascii_digit() {
                break;
            }
            value += f32::from(character - b'0') * fraction;
            fraction *= 0.1;
        }
    }
    value * sign
}

/// Vertex-normal table for [`direction_to_byte`] (`BYTE_DIRECTIONS`).
pub const BYTE_DIRECTIONS: [[f32; 3]; 162] = [
    [-0.525731, 0.000000, 0.850651],
    [-0.442863, 0.238856, 0.864188],
    [-0.295242, 0.000000, 0.955423],
    [-0.309017, 0.500000, 0.809017],
    [-0.162460, 0.262866, 0.951056],
    [0.000000, 0.000000, 1.000000],
    [0.000000, 0.850651, 0.525731],
    [-0.147621, 0.716567, 0.681718],
    [0.147621, 0.716567, 0.681718],
    [0.000000, 0.525731, 0.850651],
    [0.309017, 0.500000, 0.809017],
    [0.525731, 0.000000, 0.850651],
    [0.295242, 0.000000, 0.955423],
    [0.442863, 0.238856, 0.864188],
    [0.162460, 0.262866, 0.951056],
    [-0.681718, 0.147621, 0.716567],
    [-0.809017, 0.309017, 0.500000],
    [-0.587785, 0.425325, 0.688191],
    [-0.850651, 0.525731, 0.000000],
    [-0.864188, 0.442863, 0.238856],
    [-0.716567, 0.681718, 0.147621],
    [-0.688191, 0.587785, 0.425325],
    [-0.500000, 0.809017, 0.309017],
    [-0.238856, 0.864188, 0.442863],
    [-0.425325, 0.688191, 0.587785],
    [-0.716567, 0.681718, -0.147621],
    [-0.500000, 0.809017, -0.309017],
    [-0.525731, 0.850651, 0.000000],
    [0.000000, 0.850651, -0.525731],
    [-0.238856, 0.864188, -0.442863],
    [0.000000, 0.955423, -0.295242],
    [-0.262866, 0.951056, -0.162460],
    [0.000000, 1.000000, 0.000000],
    [0.000000, 0.955423, 0.295242],
    [-0.262866, 0.951056, 0.162460],
    [0.238856, 0.864188, 0.442863],
    [0.262866, 0.951056, 0.162460],
    [0.500000, 0.809017, 0.309017],
    [0.238856, 0.864188, -0.442863],
    [0.262866, 0.951056, -0.162460],
    [0.500000, 0.809017, -0.309017],
    [0.850651, 0.525731, 0.000000],
    [0.716567, 0.681718, 0.147621],
    [0.716567, 0.681718, -0.147621],
    [0.525731, 0.850651, 0.000000],
    [0.425325, 0.688191, 0.587785],
    [0.864188, 0.442863, 0.238856],
    [0.688191, 0.587785, 0.425325],
    [0.809017, 0.309017, 0.500000],
    [0.681718, 0.147621, 0.716567],
    [0.587785, 0.425325, 0.688191],
    [0.955423, 0.295242, 0.000000],
    [1.000000, 0.000000, 0.000000],
    [0.951056, 0.162460, 0.262866],
    [0.850651, -0.525731, 0.000000],
    [0.955423, -0.295242, 0.000000],
    [0.864188, -0.442863, 0.238856],
    [0.951056, -0.162460, 0.262866],
    [0.809017, -0.309017, 0.500000],
    [0.681718, -0.147621, 0.716567],
    [0.850651, 0.000000, 0.525731],
    [0.864188, 0.442863, -0.238856],
    [0.809017, 0.309017, -0.500000],
    [0.951056, 0.162460, -0.262866],
    [0.525731, 0.000000, -0.850651],
    [0.681718, 0.147621, -0.716567],
    [0.681718, -0.147621, -0.716567],
    [0.850651, 0.000000, -0.525731],
    [0.809017, -0.309017, -0.500000],
    [0.864188, -0.442863, -0.238856],
    [0.951056, -0.162460, -0.262866],
    [0.147621, 0.716567, -0.681718],
    [0.309017, 0.500000, -0.809017],
    [0.425325, 0.688191, -0.587785],
    [0.442863, 0.238856, -0.864188],
    [0.587785, 0.425325, -0.688191],
    [0.688191, 0.587785, -0.425325],
    [-0.147621, 0.716567, -0.681718],
    [-0.309017, 0.500000, -0.809017],
    [0.000000, 0.525731, -0.850651],
    [-0.525731, 0.000000, -0.850651],
    [-0.442863, 0.238856, -0.864188],
    [-0.295242, 0.000000, -0.955423],
    [-0.162460, 0.262866, -0.951056],
    [0.000000, 0.000000, -1.000000],
    [0.295242, 0.000000, -0.955423],
    [0.162460, 0.262866, -0.951056],
    [-0.442863, -0.238856, -0.864188],
    [-0.309017, -0.500000, -0.809017],
    [-0.162460, -0.262866, -0.951056],
    [0.000000, -0.850651, -0.525731],
    [-0.147621, -0.716567, -0.681718],
    [0.147621, -0.716567, -0.681718],
    [0.000000, -0.525731, -0.850651],
    [0.309017, -0.500000, -0.809017],
    [0.442863, -0.238856, -0.864188],
    [0.162460, -0.262866, -0.951056],
    [0.238856, -0.864188, -0.442863],
    [0.500000, -0.809017, -0.309017],
    [0.425325, -0.688191, -0.587785],
    [0.716567, -0.681718, -0.147621],
    [0.688191, -0.587785, -0.425325],
    [0.587785, -0.425325, -0.688191],
    [0.000000, -0.955423, -0.295242],
    [0.000000, -1.000000, 0.000000],
    [0.262866, -0.951056, -0.162460],
    [0.000000, -0.850651, 0.525731],
    [0.000000, -0.955423, 0.295242],
    [0.238856, -0.864188, 0.442863],
    [0.262866, -0.951056, 0.162460],
    [0.500000, -0.809017, 0.309017],
    [0.716567, -0.681718, 0.147621],
    [0.525731, -0.850651, 0.000000],
    [-0.238856, -0.864188, -0.442863],
    [-0.500000, -0.809017, -0.309017],
    [-0.262866, -0.951056, -0.162460],
    [-0.850651, -0.525731, 0.000000],
    [-0.716567, -0.681718, -0.147621],
    [-0.716567, -0.681718, 0.147621],
    [-0.525731, -0.850651, 0.000000],
    [-0.500000, -0.809017, 0.309017],
    [-0.238856, -0.864188, 0.442863],
    [-0.262866, -0.951056, 0.162460],
    [-0.864188, -0.442863, 0.238856],
    [-0.809017, -0.309017, 0.500000],
    [-0.688191, -0.587785, 0.425325],
    [-0.681718, -0.147621, 0.716567],
    [-0.442863, -0.238856, 0.864188],
    [-0.587785, -0.425325, 0.688191],
    [-0.309017, -0.500000, 0.809017],
    [-0.147621, -0.716567, 0.681718],
    [-0.425325, -0.688191, 0.587785],
    [-0.162460, -0.262866, 0.951056],
    [0.442863, -0.238856, 0.864188],
    [0.162460, -0.262866, 0.951056],
    [0.309017, -0.500000, 0.809017],
    [0.147621, -0.716567, 0.681718],
    [0.000000, -0.525731, 0.850651],
    [0.425325, -0.688191, 0.587785],
    [0.587785, -0.425325, 0.688191],
    [0.688191, -0.587785, 0.425325],
    [-0.955423, 0.295242, 0.000000],
    [-0.951056, 0.162460, 0.262866],
    [-1.000000, 0.000000, 0.000000],
    [-0.850651, 0.000000, 0.525731],
    [-0.955423, -0.295242, 0.000000],
    [-0.951056, -0.162460, 0.262866],
    [-0.864188, 0.442863, -0.238856],
    [-0.951056, 0.162460, -0.262866],
    [-0.809017, 0.309017, -0.500000],
    [-0.864188, -0.442863, -0.238856],
    [-0.951056, -0.162460, -0.262866],
    [-0.809017, -0.309017, -0.500000],
    [-0.681718, 0.147621, -0.716567],
    [-0.681718, -0.147621, -0.716567],
    [-0.850651, 0.000000, -0.525731],
    [-0.688191, 0.587785, -0.425325],
    [-0.587785, 0.425325, -0.688191],
    [-0.425325, 0.688191, -0.587785],
    [-0.425325, -0.688191, -0.587785],
    [-0.587785, -0.425325, -0.688191],
    [-0.688191, -0.587785, -0.425325],
];

/// Pack a direction into a byte (`directionToByte`); null maps to 0.
#[must_use]
pub fn direction_to_byte(value: Option<Vec3>) -> i32 {
    let Some(value) = value else {
        return 0;
    };
    let mut best_dot = 0.0f32;
    let mut best = 0;
    for (index, row) in BYTE_DIRECTIONS.iter().enumerate() {
        let candidate = vec3(row[0], row[1], row[2]);
        let dot = dot3(value, candidate);
        if dot > best_dot {
            best_dot = dot;
            best = index;
        }
    }
    best as i32
}

/// Item kind: discriminated union of item type + tag (`ItemDefinition`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ItemKind {
    /// Invalid.
    Bad,
    /// Weapon with weapon tag.
    Weapon(Weapon),
    /// Ammo with weapon tag.
    Ammo(Weapon),
    /// Armor.
    Armor,
    /// Health.
    Health,
    /// Powerup with powerup tag.
    Powerup(Powerup),
    /// Holdable with holdable tag.
    Holdable(Holdable),
    /// Persistent powerup with powerup tag.
    PersistantPowerup(Powerup),
    /// Team objective with powerup tag.
    Team(Powerup),
}

impl ItemKind {
    /// Item type of this kind.
    #[must_use]
    pub fn item_type(self) -> ItemType {
        match self {
            ItemKind::Bad => ItemType::Bad,
            ItemKind::Weapon(_) => ItemType::Weapon,
            ItemKind::Ammo(_) => ItemType::Ammo,
            ItemKind::Armor => ItemType::Armor,
            ItemKind::Health => ItemType::Health,
            ItemKind::Powerup(_) => ItemType::Powerup,
            ItemKind::Holdable(_) => ItemType::Holdable,
            ItemKind::PersistantPowerup(_) => ItemType::PersistantPowerup,
            ItemKind::Team(_) => ItemType::Team,
        }
    }
}

/// Item definition subset used by game item logic (`ItemDefinition`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ItemDefinition {
    /// Entity class name.
    pub class_name: Option<String>,
    /// Default quantity.
    pub quantity: i32,
    /// Discriminated type + tag.
    pub kind: ItemKind,
}

impl ItemDefinition {
    /// Item type.
    #[must_use]
    pub fn item_type(&self) -> ItemType {
        self.kind.item_type()
    }

    /// Weapon tag; fails for non-weapon/ammo items.
    pub fn weapon_tag(&self) -> Q3GameItemsResult<Weapon> {
        match self.kind {
            ItemKind::Weapon(tag) | ItemKind::Ammo(tag) => Ok(tag),
            _ => Err(invalid("item has no weapon tag")),
        }
    }

    /// Powerup tag; fails for other items.
    pub fn powerup_tag(&self) -> Q3GameItemsResult<Powerup> {
        match self.kind {
            ItemKind::Powerup(tag) | ItemKind::PersistantPowerup(tag) | ItemKind::Team(tag) => Ok(tag),
            _ => Err(invalid("item has no powerup tag")),
        }
    }

    /// Holdable tag; fails for other items.
    pub fn holdable_tag(&self) -> Q3GameItemsResult<Holdable> {
        match self.kind {
            ItemKind::Holdable(tag) => Ok(tag),
            _ => Err(invalid("item has no holdable tag")),
        }
    }
}

/// Item table host (`itemList`/`itemAt`/`findItemForWeapon`).
pub trait ItemTable {
    /// Index of an item definition, or `None` when absent.
    fn index_of(&self, product: Product, item: &ItemDefinition) -> Option<usize>;
    /// Item at an index.
    fn item_at(&self, product: Product, index: usize) -> Q3GameItemsResult<ItemDefinition>;
    /// Item for a weapon tag.
    fn find_item_for_weapon(&self, product: Product, weapon: Weapon) -> Q3GameItemsResult<ItemDefinition>;
}

/// Item registration host (`ItemRegistry`).
pub trait ItemRegistry {
    /// Registry product.
    fn product(&self) -> Product;
    /// Register an item (precache).
    fn register(&mut self, item: &ItemDefinition);
}

/// Player-state integer slots (`PlayerStateSlots`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerStateSlots {
    values: Vec<i32>,
}

impl PlayerStateSlots {
    /// Zeroed slots.
    #[must_use]
    pub fn new(length: usize) -> PlayerStateSlots {
        PlayerStateSlots {
            values: vec![0; length],
        }
    }

    /// Slot count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Whether there are no slots.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Read a slot.
    pub fn get(&self, index: usize) -> Q3GameItemsResult<i32> {
        self.values
            .get(index)
            .copied()
            .ok_or_else(|| range(format!("player state slot {index} outside {}", self.values.len())))
    }

    /// Write a slot.
    pub fn set(&mut self, index: usize, value: i32) -> Q3GameItemsResult<()> {
        let len = self.values.len();
        let slot = self
            .values
            .get_mut(index)
            .ok_or_else(|| range(format!("player state slot {index} outside {len}")))?;
        *slot = value;
        Ok(())
    }

    /// Fill every slot.
    pub fn fill(&mut self, value: i32) {
        self.values.fill(value);
    }
}

/// Pool slot address of an entity.
pub type Slot = usize;

/// Actor identity (`ActorId`); the actor number is the pool slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ActorId(pub u32);

impl ActorId {
    /// Actor for a pool slot.
    #[must_use]
    pub fn from_slot(slot: Slot) -> ActorId {
        ActorId(slot as u32)
    }

    /// Pool slot, when in range.
    #[must_use]
    pub fn slot(self) -> Option<Slot> {
        let slot = self.0 as usize;
        (slot < MAX_GENTITIES).then_some(slot)
    }
}

/// Owned actor handle (`OwnedActor`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OwnedActor {
    /// Actor identity.
    pub id: ActorId,
}

/// Event-bit constants (`EV_EVENT_BIT1`/`EV_EVENT_BITS`).
pub const EV_EVENT_BIT1: i32 = 0x100;

/// Event-bit mask.
pub const EV_EVENT_BITS: i32 = 0x300;

/// Dropped-item lifetime in ms.
pub const DROPPED_ITEM_LIFETIME: i32 = 30_000;

/// Entity state subset (`EntityState`).
#[derive(Debug, Clone, PartialEq)]
pub struct EntityState {
    /// Entity type (raw; temp entities add the event, mirroring the donor).
    pub e_type: i32,
    /// Entity number.
    pub number: i32,
    /// Client number.
    pub client_num: i32,
    /// Spawn origin.
    pub origin: Vec3,
    /// Secondary origin (portals).
    pub origin2: Vec3,
    /// Angles.
    pub angles: Vec3,
    /// Secondary angles.
    pub angles2: Vec3,
    /// Position trajectory.
    pub pos: Trajectory,
    /// Angle trajectory.
    pub apos: Trajectory,
    /// Weapon.
    pub weapon: Weapon,
    /// Model index.
    pub modelindex: i32,
    /// Secondary model index.
    pub modelindex2: i32,
    /// Entity flags.
    pub e_flags: i32,
    /// Event.
    pub event: i32,
    /// Event parameter.
    pub event_parm: i32,
    /// Other entity number.
    pub other_entity_num: i32,
    /// Ground entity number.
    pub ground_entity_num: i32,
    /// Loop sound.
    pub loop_sound: i32,
    /// Generic 1.
    pub generic1: i32,
    /// Powerups bitmask.
    pub powerups: i32,
    /// Frame.
    pub frame: i32,
    /// Legs animation.
    pub legs_anim: i32,
    /// Torso animation.
    pub torso_anim: i32,
}

impl Default for EntityState {
    fn default() -> Self {
        EntityState {
            e_type: EntityType::General as i32,
            number: 0,
            client_num: 0,
            origin: vec3(0.0, 0.0, 0.0),
            origin2: vec3(0.0, 0.0, 0.0),
            angles: vec3(0.0, 0.0, 0.0),
            angles2: vec3(0.0, 0.0, 0.0),
            pos: Trajectory::default(),
            apos: Trajectory::default(),
            weapon: Weapon::None,
            modelindex: 0,
            modelindex2: 0,
            e_flags: 0,
            event: 0,
            event_parm: 0,
            other_entity_num: 0,
            ground_entity_num: ENTITYNUM_NONE,
            loop_sound: 0,
            generic1: 0,
            powerups: 0,
            frame: 0,
            legs_anim: 0,
            torso_anim: 0,
        }
    }
}

/// Shared entity subset (`EntityShared` / `r`).
#[derive(Debug, Clone, PartialEq)]
pub struct EntityShared {
    /// Current origin.
    pub current_origin: Vec3,
    /// Current angles.
    pub current_angles: Vec3,
    /// Bounds minimum.
    pub mins: Vec3,
    /// Bounds maximum.
    pub maxs: Vec3,
    /// Absolute bounds minimum.
    pub absmin: Vec3,
    /// Absolute bounds maximum.
    pub absmax: Vec3,
    /// Contents mask.
    pub contents: i32,
    /// Owner entity number.
    pub owner_num: i32,
    /// Server flags.
    pub sv_flags: i32,
}

impl Default for EntityShared {
    fn default() -> Self {
        EntityShared {
            current_origin: vec3(0.0, 0.0, 0.0),
            current_angles: vec3(0.0, 0.0, 0.0),
            mins: vec3(0.0, 0.0, 0.0),
            maxs: vec3(0.0, 0.0, 0.0),
            absmin: vec3(0.0, 0.0, 0.0),
            absmax: vec3(0.0, 0.0, 0.0),
            contents: 0,
            owner_num: ENTITYNUM_NONE,
            sv_flags: 0,
        }
    }
}

/// Player state subset (`PlayerState`).
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerState {
    /// Product.
    pub product: Product,
    /// Client number.
    pub client_num: i32,
    /// Origin.
    pub origin: Vec3,
    /// Velocity.
    pub velocity: Vec3,
    /// View angles.
    pub viewangles: Vec3,
    /// Delta angles.
    pub delta_angles: [i32; 3],
    /// PM time.
    pub pm_time: i32,
    /// PM flags.
    pub pm_flags: i32,
    /// PM type.
    pub pm_type: MoveType,
    /// Entity flags.
    pub e_flags: i32,
    /// Health.
    pub health: i32,
    /// Stat slots.
    pub stats: PlayerStateSlots,
    /// Persistent slots.
    pub persistant: PlayerStateSlots,
    /// Powerup slots.
    pub powerups: PlayerStateSlots,
    /// Ammo slots indexed by weapon.
    pub ammo: PlayerStateSlots,
    /// Current weapon.
    pub weapon: Weapon,
    /// Legs animation.
    pub legs_anim: i32,
    /// Torso animation.
    pub torso_anim: i32,
    /// Movement direction.
    pub movement_dir: f32,
    /// Ground entity number.
    pub ground_entity_num: i32,
    /// Loop sound.
    pub loop_sound: i32,
    /// Generic 1.
    pub generic1: i32,
    /// External event.
    pub external_event: i32,
    /// External event parameter.
    pub external_event_parm: i32,
    /// External event time.
    pub external_event_time: i32,
    /// Event sequence.
    pub event_sequence: i32,
    /// Entity event sequence.
    pub entity_event_sequence: i32,
    /// Predictable events.
    pub events: PlayerStateSlots,
    /// Predictable event parameters.
    pub event_parms: PlayerStateSlots,
    /// Grapple point.
    pub grapple_point: Vec3,
}

impl PlayerState {
    /// Zero player state for a product.
    #[must_use]
    pub fn new(product: Product) -> PlayerState {
        PlayerState {
            product,
            client_num: 0,
            origin: vec3(0.0, 0.0, 0.0),
            velocity: vec3(0.0, 0.0, 0.0),
            viewangles: vec3(0.0, 0.0, 0.0),
            delta_angles: [0, 0, 0],
            pm_time: 0,
            pm_flags: 0,
            pm_type: MoveType::Normal,
            e_flags: 0,
            health: 0,
            stats: PlayerStateSlots::new(16),
            persistant: PlayerStateSlots::new(16),
            powerups: PlayerStateSlots::new(16),
            ammo: PlayerStateSlots::new(16),
            weapon: Weapon::None,
            legs_anim: 0,
            torso_anim: 0,
            movement_dir: 0.0,
            ground_entity_num: ENTITYNUM_NONE,
            loop_sound: 0,
            generic1: 0,
            external_event: 0,
            external_event_parm: 0,
            external_event_time: 0,
            event_sequence: 0,
            entity_event_sequence: 0,
            events: PlayerStateSlots::new(2),
            event_parms: PlayerStateSlots::new(2),
            grapple_point: vec3(0.0, 0.0, 0.0),
        }
    }
}

/// Persistent client subset (`ClientPersistant`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientPersistant {
    /// Connection state.
    pub connected: ConnectionState,
    /// Command view angles.
    pub cmd_angles: [i32; 3],
    /// Maximum health.
    pub max_health: i32,
}

impl Default for ClientPersistant {
    fn default() -> Self {
        ClientPersistant {
            connected: ConnectionState::Disconnected,
            cmd_angles: [0, 0, 0],
            max_health: 0,
        }
    }
}

/// Client session subset (`ClientSession`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientSession {
    /// Session team.
    pub session_team: Team,
}

impl Default for ClientSession {
    fn default() -> Self {
        ClientSession {
            session_team: Team::Free,
        }
    }
}

/// Game client subset (`GameClient`).
#[derive(Debug, Clone, PartialEq)]
pub struct GameClient {
    /// Player state.
    pub ps: PlayerState,
    /// Persistent data.
    pub pers: ClientPersistant,
    /// Session data.
    pub sess: ClientSession,
    /// Grapple hook entity.
    pub hook: Option<Slot>,
    /// Persistent powerup entity.
    pub persistant_powerup: Option<Slot>,
    /// Per-weapon ammo times.
    pub ammo_times: PlayerStateSlots,
    /// Invulnerability expiry.
    pub invulnerability_time: i32,
    /// Accuracy hits.
    pub accuracy_hits: i32,
}

impl GameClient {
    /// Zero client for a product.
    #[must_use]
    pub fn new(product: Product) -> GameClient {
        GameClient {
            ps: PlayerState::new(product),
            pers: ClientPersistant::default(),
            sess: ClientSession::default(),
            hook: None,
            persistant_powerup: None,
            ammo_times: PlayerStateSlots::new(14),
            invulnerability_time: 0,
            accuracy_hits: 0,
        }
    }
}

/// Save-callback name; the donor's string-keyed callback identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CallbackName(pub &'static str);

/// String-keyed callback registry (`intern`/`resolve` per donor catalogs).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CallbackTable {
    names: HashSet<&'static str>,
}

impl CallbackTable {
    /// Register a callback name.
    pub fn intern(&mut self, name: &'static str) {
        self.names.insert(name);
    }

    /// Resolve a registered name.
    pub fn resolve(&self, name: &'static str) -> Q3GameItemsResult<CallbackName> {
        if self.names.contains(name) {
            Ok(CallbackName(name))
        } else {
            Err(missing(format!("callback {name} is not registered")))
        }
    }

    /// Whether a name is registered.
    #[must_use]
    pub fn contains(&self, name: &'static str) -> bool {
        self.names.contains(name)
    }
}

/// Damage participant (`DamageParticipant`): a pooled entity or a shared actor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DamageParticipant {
    /// Pooled entity slot.
    Entity(Slot),
    /// Shared actor without a pooled record.
    SharedActor(ActorId),
}

/// Game entity subset (`GameEntity`).
#[derive(Debug, Clone, PartialEq)]
pub struct GameEntity {
    /// Pool slot.
    pub slot: Slot,
    /// Generation (bumped on free).
    pub generation: u32,
    /// In-use flag.
    pub inuse: bool,
    /// Linked flag.
    pub linked: bool,
    /// Class name.
    pub classname: Option<String>,
    /// Brush model name.
    pub model: Option<String>,
    /// Target name.
    pub target: Option<String>,
    /// Targetname.
    pub targetname: Option<String>,
    /// Item definition.
    pub item: Option<ItemDefinition>,
    /// Entity state.
    pub s: EntityState,
    /// Shared state.
    pub r: EntityShared,
    /// Client record.
    pub client: Option<GameClient>,
    /// Health.
    pub health: i32,
    /// Takes damage.
    pub takedamage: bool,
    /// Physics bounce factor.
    pub physics_bounce: f32,
    /// Game flags.
    pub flags: i32,
    /// Spawn flags.
    pub spawnflags: i32,
    /// Next think time.
    pub nextthink: i32,
    /// Think callback.
    pub think: Option<CallbackName>,
    /// Touch callback.
    pub touch: Option<CallbackName>,
    /// Use callback.
    pub use_cb: Option<CallbackName>,
    /// Die callback.
    pub die: Option<CallbackName>,
    /// Blocked callback.
    pub blocked: Option<CallbackName>,
    /// Reached callback.
    pub reached: Option<CallbackName>,
    /// Enemy slot.
    pub enemy: Option<Slot>,
    /// Parent slot.
    pub parent: Option<Slot>,
    /// Activator slot.
    pub activator: Option<Slot>,
    /// Target entity slot.
    pub target_ent: Option<Slot>,
    /// Team chain slot.
    pub teamchain: Option<Slot>,
    /// Next train slot.
    pub next_train: Option<Slot>,
    /// Count (quantity override / trigger axis / arming flag).
    pub count: i32,
    /// Random spread / misc float.
    pub random: f32,
    /// Move direction.
    pub movedir: Vec3,
    /// Speed.
    pub speed: f32,
    /// Wait.
    pub wait: f32,
    /// Damage.
    pub damage: i32,
    /// Splash damage.
    pub splash_damage: i32,
    /// Splash radius.
    pub splash_radius: f32,
    /// Means of death.
    pub method_of_death: i32,
    /// Splash means of death.
    pub splash_method_of_death: i32,
    /// Clip mask.
    pub clipmask: i32,
    /// Door/plat sounds.
    pub sound1to2: i32,
    /// Door/plat sounds.
    pub sound2to1: i32,
    /// Door/plat sounds.
    pub sound_pos1: i32,
    /// Door/plat sounds.
    pub sound_pos2: i32,
    /// Looping sound.
    pub sound_loop: i32,
    /// Mover position 1.
    pub pos1: Vec3,
    /// Mover position 2.
    pub pos2: Vec3,
    /// Mover state.
    pub mover_state: MoverState,
    /// Free after event.
    pub free_after_event: bool,
    /// Event time.
    pub event_time: i32,
    /// Ground actor (shared-body support mirror).
    pub ground: Option<ActorId>,
}

impl GameEntity {
    /// Fresh pooled record.
    #[must_use]
    pub fn new(slot: Slot, generation: u32) -> GameEntity {
        GameEntity {
            slot,
            generation,
            inuse: false,
            linked: false,
            classname: None,
            model: None,
            target: None,
            targetname: None,
            item: None,
            s: EntityState::default(),
            r: EntityShared::default(),
            client: None,
            health: 0,
            takedamage: false,
            physics_bounce: 0.0,
            flags: 0,
            spawnflags: 0,
            nextthink: 0,
            think: None,
            touch: None,
            use_cb: None,
            die: None,
            blocked: None,
            reached: None,
            enemy: None,
            parent: None,
            activator: None,
            target_ent: None,
            teamchain: None,
            next_train: None,
            count: 0,
            random: 0.0,
            movedir: vec3(0.0, 0.0, 0.0),
            speed: 0.0,
            wait: 0.0,
            damage: 0,
            splash_damage: 0,
            splash_radius: 0.0,
            method_of_death: 0,
            splash_method_of_death: 0,
            clipmask: 0,
            sound1to2: 0,
            sound2to1: 0,
            sound_pos1: 0,
            sound_pos2: 0,
            sound_loop: 0,
            pos1: vec3(0.0, 0.0, 0.0),
            pos2: vec3(0.0, 0.0, 0.0),
            mover_state: MoverState::Pos1,
            free_after_event: false,
            event_time: 0,
            ground: None,
        }
    }

    /// Client record or an error.
    pub fn client(&self) -> Q3GameItemsResult<&GameClient> {
        self.client
            .as_ref()
            .ok_or_else(|| invalid("entity has no client record"))
    }

    /// Mutable client record or an error.
    pub fn client_mut(&mut self) -> Q3GameItemsResult<&mut GameClient> {
        self.client
            .as_mut()
            .ok_or_else(|| invalid("entity has no client record"))
    }
}

/// Fired event record for host dispatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FiredEvent {
    /// Entity slot.
    pub slot: Slot,
    /// Event.
    pub event: EntityEvent,
    /// Parameter.
    pub parm: i32,
    /// Time.
    pub time: i32,
}

/// Entity pool (`EntityPool` subset).
#[derive(Debug)]
pub struct EntityPool {
    pub(crate) product: Product,
    pub(crate) entities: Vec<GameEntity>,
    pub(crate) generations: Vec<u32>,
    pub(crate) high_water: usize,
    /// Current game time for event stamps (host sets per frame).
    pub time: i32,
    /// Think callbacks.
    pub think_cbs: CallbackTable,
    /// Touch callbacks.
    pub touch_cbs: CallbackTable,
    /// Use callbacks.
    pub use_cbs: CallbackTable,
    /// Die callbacks.
    pub die_cbs: CallbackTable,
    /// Blocked callbacks.
    pub blocked_cbs: CallbackTable,
    /// Reached callbacks.
    pub reached_cbs: CallbackTable,
    /// Fired non-client event log.
    pub events: Vec<FiredEvent>,
    /// Diagnostic prints (e.g. zero-event warnings).
    pub prints: Vec<String>,
}

impl EntityPool {
    /// Empty pool with a world entity at slot 1022.
    #[must_use]
    pub fn new(product: Product) -> EntityPool {
        let mut entities = Vec::with_capacity(MAX_GENTITIES);
        for slot in 0..MAX_GENTITIES {
            entities.push(GameEntity::new(slot, 0));
        }
        let mut pool = EntityPool {
            product,
            entities,
            generations: vec![0; MAX_GENTITIES],
            high_water: MAX_CLIENTS,
            time: 0,
            think_cbs: CallbackTable::default(),
            touch_cbs: CallbackTable::default(),
            use_cbs: CallbackTable::default(),
            die_cbs: CallbackTable::default(),
            blocked_cbs: CallbackTable::default(),
            reached_cbs: CallbackTable::default(),
            events: Vec::new(),
            prints: Vec::new(),
        };
        let world = pool.at_mut(ENTITYNUM_WORLD).expect("world slot in range");
        world.inuse = true;
        world.classname = Some("worldspawn".to_string());
        world.s.number = ENTITYNUM_WORLD as i32;
        pool
    }

    /// Pool product.
    #[must_use]
    pub fn product(&self) -> Product {
        self.product
    }

    /// Entity high-water mark (`numEntities`).
    #[must_use]
    pub fn num_entities(&self) -> usize {
        self.high_water
    }

    /// Record by slot regardless of use (`at`).
    pub fn at(&self, slot: Slot) -> Q3GameItemsResult<&GameEntity> {
        self.entities
            .get(slot)
            .ok_or_else(|| range(format!("entity slot {slot} outside {MAX_GENTITIES}")))
    }

    /// Mutable record by slot regardless of use.
    pub fn at_mut(&mut self, slot: Slot) -> Q3GameItemsResult<&mut GameEntity> {
        self.entities
            .get_mut(slot)
            .ok_or_else(|| range(format!("entity slot {slot} outside {MAX_GENTITIES}")))
    }

    /// Live record by slot (`get`).
    #[must_use]
    pub fn get(&self, slot: Slot) -> Option<&GameEntity> {
        self.entities.get(slot).filter(|entity| entity.inuse)
    }

    /// Live mutable record by slot.
    pub fn get_mut(&mut self, slot: Slot) -> Option<&mut GameEntity> {
        self.entities.get_mut(slot).filter(|entity| entity.inuse)
    }

    /// Generation counter for a slot.
    pub fn generation(&self, slot: Slot) -> Q3GameItemsResult<u32> {
        self.generations
            .get(slot)
            .copied()
            .ok_or_else(|| range(format!("entity slot {slot} outside {MAX_GENTITIES}")))
    }

    /// Ownership check: the slot must hold a live record.
    pub fn require_owned(&self, slot: Slot) -> Q3GameItemsResult<()> {
        if self.get(slot).is_some() {
            Ok(())
        } else {
            Err(invalid(format!(
                "entity {slot} does not belong to its pool or was freed"
            )))
        }
    }

    /// Spawn a record (`spawn`).
    pub fn spawn(&mut self) -> Q3GameItemsResult<Slot> {
        for slot in 0..MAX_GENTITIES {
            if !self.entities[slot].inuse {
                let generation = self.generations[slot];
                let mut entity = GameEntity::new(slot, generation);
                entity.inuse = true;
                entity.s.number = slot as i32;
                self.entities[slot] = entity;
                self.high_water = self.high_water.max(slot + 1);
                return Ok(slot);
            }
        }
        Err(drop_error("G_Spawn: no free entities"))
    }

    /// Free a record (`free`).
    pub fn free(&mut self, slot: Slot) -> Q3GameItemsResult<()> {
        self.require_owned(slot)?;
        self.generations[slot] = self.generations[slot].wrapping_add(1);
        let generation = self.generations[slot];
        self.entities[slot] = GameEntity::new(slot, generation);
        Ok(())
    }

    /// Live slot for an actor (`nativeByActor`).
    #[must_use]
    pub fn native_by_actor(&self, actor: ActorId) -> Option<Slot> {
        actor
            .slot()
            .filter(|slot| self.get(*slot).is_some_and(|entity| entity.inuse))
    }

    /// Mark a record linked (`options.link` pool half).
    pub fn link(&mut self, slot: Slot) -> Q3GameItemsResult<()> {
        self.require_owned(slot)?;
        self.entities[slot].linked = true;
        Ok(())
    }

    /// Mark a record unlinked.
    pub fn unlink(&mut self, slot: Slot) -> Q3GameItemsResult<()> {
        self.require_owned(slot)?;
        self.entities[slot].linked = false;
        Ok(())
    }

    /// Disjoint mutable pair.
    pub fn pair(&mut self, first: Slot, second: Slot) -> Q3GameItemsResult<(&mut GameEntity, &mut GameEntity)> {
        if first == second {
            return Err(invalid("entity pair slots must differ"));
        }
        if first >= MAX_GENTITIES || second >= MAX_GENTITIES {
            return Err(range("entity pair slot outside 1024"));
        }
        let (low, high) = if first < second {
            (first, second)
        } else {
            (second, first)
        };
        let (head, tail) = self.entities.split_at_mut(high);
        let low_ref = &mut head[low];
        let high_ref = &mut tail[0];
        if !low_ref.inuse || !high_ref.inuse {
            return Err(invalid("entity pair record was freed"));
        }
        if first < second {
            Ok((low_ref, high_ref))
        } else {
            Ok((high_ref, low_ref))
        }
    }

    /// Add an entity event (`addEvent`).
    pub fn add_event(&mut self, slot: Slot, event: EntityEvent, parameter: i32) -> Q3GameItemsResult<()> {
        self.require_owned(slot)?;
        if event == EntityEvent::None {
            let number = self.entities[slot].s.number;
            self.prints
                .push(format!("G_AddEvent: zero event added for entity {number}\n"));
            return Ok(());
        }
        let now = self.time;
        let entity = &mut self.entities[slot];
        if let Some(client) = entity.client.as_mut() {
            let bits = ((client.ps.external_event & EV_EVENT_BITS) + EV_EVENT_BIT1) & EV_EVENT_BITS;
            client.ps.external_event = event as i32 | bits;
            client.ps.external_event_parm = parameter;
            client.ps.external_event_time = now;
        } else {
            let bits = ((entity.s.event & EV_EVENT_BITS) + EV_EVENT_BIT1) & EV_EVENT_BITS;
            entity.s.event = event as i32 | bits;
            entity.s.event_parm = parameter;
        }
        entity.event_time = now;
        self.events.push(FiredEvent {
            slot,
            event,
            parm: parameter,
            time: now,
        });
        Ok(())
    }

    /// Spawn a temporary event entity (`tempEntity`).
    pub fn temp_entity(
        &mut self,
        world: &mut dyn WorldOps,
        origin: Vec3,
        event: EntityEvent,
    ) -> Q3GameItemsResult<Slot> {
        let slot = self.spawn()?;
        {
            let entity = &mut self.entities[slot];
            entity.s.e_type = EntityType::Events as i32 + event as i32;
            entity.classname = Some("tempEntity".to_string());
            entity.event_time = self.time;
            entity.free_after_event = true;
            let snapped = vec3(
                qvm_float_to_int(origin.x) as f32,
                qvm_float_to_int(origin.y) as f32,
                qvm_float_to_int(origin.z) as f32,
            );
            entity.s.pos = Trajectory {
                ty: TrajectoryType::Stationary,
                time: 0,
                duration: 0,
                base: snapped,
                delta: vec3(0.0, 0.0, 0.0),
            };
            entity.r.current_origin = snapped;
        }
        world.link(self, slot)?;
        Ok(slot)
    }

    /// Format a vector (`vtos`).
    #[must_use]
    pub fn vtos(vector: Vec3) -> String {
        format!(
            "({} {} {})",
            qvm_float_to_int(vector.x),
            qvm_float_to_int(vector.y),
            qvm_float_to_int(vector.z)
        )
    }
}

/// Trace solidity (`solidity`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TraceSolidity {
    /// Clear path.
    Clear,
    /// Started inside solid.
    StartSolid,
    /// Entirely solid.
    AllSolid,
}

/// Trace contact (`contact`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TraceContact {
    /// No contact.
    None,
    /// Plane contact.
    Plane {
        /// Plane normal.
        normal: Vec3,
    },
}

/// Trace hit identity (`hit`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TraceHit {
    /// No hit.
    None,
    /// World hit.
    World,
    /// Actor hit.
    Actor(ActorId),
}

/// Actor trace result (`ActorTraceResult`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ActorTraceResult {
    /// Fraction travelled.
    pub fraction: f32,
    /// End position.
    pub end: Vec3,
    /// Solidity.
    pub solidity: TraceSolidity,
    /// Contact.
    pub contact: TraceContact,
    /// Contents.
    pub contents: i32,
    /// Surface flags.
    pub surface_flags: i32,
    /// Hit identity.
    pub hit: TraceHit,
}

/// Server trace result (`ServerTraceResult`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ServerTraceResult {
    /// Fraction travelled.
    pub fraction: f32,
    /// End position.
    pub end: Vec3,
    /// Hit entity number.
    pub entity_num: i32,
    /// Solidity.
    pub solidity: TraceSolidity,
    /// Contact.
    pub contact: TraceContact,
    /// Contents.
    pub contents: i32,
    /// Surface flags.
    pub surface_flags: i32,
}

impl ServerTraceResult {
    /// Reinterpret as an actor trace against a resolved actor.
    #[must_use]
    pub fn with_actor_hit(self, actor: ActorId) -> ActorTraceResult {
        ActorTraceResult {
            fraction: self.fraction,
            end: self.end,
            solidity: self.solidity,
            contact: self.contact,
            contents: self.contents,
            surface_flags: self.surface_flags,
            hit: TraceHit::Actor(actor),
        }
    }
}

/// Trace shape (`TraceShape`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TraceShape {
    /// Point trace.
    Point,
    /// Box trace.
    Box {
        /// Minimum corner.
        mins: Vec3,
        /// Maximum corner.
        maxs: Vec3,
    },
    /// Capsule trace.
    Capsule {
        /// Minimum corner.
        mins: Vec3,
        /// Maximum corner.
        maxs: Vec3,
    },
}

/// Actor trace query (`ActorTraceQuery`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ActorTraceQuery {
    /// Start.
    pub start: Vec3,
    /// End.
    pub end: Vec3,
    /// Shape.
    pub shape: TraceShape,
    /// Actor to ignore.
    pub pass_actor: Option<ActorId>,
    /// Contents mask.
    pub mask: i32,
}

/// Server world host (`ServerWorld` + `ActorSpatialQueries`).
pub trait WorldOps {
    /// Link an entity.
    fn link(&mut self, pool: &mut EntityPool, slot: Slot) -> Q3GameItemsResult<()>;
    /// Unlink an entity number.
    fn unlink(&mut self, pool: &mut EntityPool, number: i32) -> Q3GameItemsResult<()>;
    /// Trace an actor sweep.
    fn trace_actor(&mut self, pool: &EntityPool, query: &ActorTraceQuery) -> ActorTraceResult;
    /// Contents at a point.
    fn point_contents(&mut self, pool: &EntityPool, point: Vec3, pass_entity_num: i32) -> i32;
    /// Actors touching bounds.
    fn area_actors(&mut self, pool: &EntityPool, bounds: Bounds, maximum: usize) -> Vec<ActorId>;
}

/// Authority record for a damageable actor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthorityState {
    /// Can take damage.
    pub can_take_damage: bool,
    /// Health.
    pub health: i32,
    /// Team, when teamed.
    pub team: Option<Team>,
}

/// Accuracy-hit target description (`q3AccuracyHit` input).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccuracyTarget {
    /// Actor.
    pub actor: ActorId,
    /// Damageable.
    pub damageable: bool,
    /// Player.
    pub player: bool,
    /// Health.
    pub health: i32,
    /// Team, when teamed.
    pub team: Option<Team>,
}

/// Combat host (`CombatContext` + damage/radius/accuracy surface).
pub trait CombatOps {
    /// Current time.
    fn time(&self) -> i32;
    /// Previous frame time.
    fn previous_time(&self) -> i32;
    /// Game type.
    fn game_type(&self) -> i32;
    /// Product.
    fn product(&self) -> Product;
    /// Spawn a temporary event entity.
    fn temp_entity(
        &mut self,
        pool: &mut EntityPool,
        world: &mut dyn WorldOps,
        origin: Vec3,
        event: EntityEvent,
    ) -> Q3GameItemsResult<Slot> {
        pool.temp_entity(world, origin, event)
    }
    /// Add an entity event.
    fn add_event(&mut self, pool: &mut EntityPool, slot: Slot, event: EntityEvent, parm: i32) -> Q3GameItemsResult<()> {
        pool.add_event(slot, event, parm)
    }
    /// Apply damage (`damage`).
    #[allow(clippy::too_many_arguments)]
    fn damage(
        &mut self,
        pool: &mut EntityPool,
        target: Slot,
        inflictor: DamageParticipant,
        attacker: DamageParticipant,
        direction: Option<Vec3>,
        point: Option<Vec3>,
        amount: i32,
        flags: i32,
        method: i32,
        projectile: Option<ActorId>,
    ) -> Q3GameItemsResult<()>;
    /// Line-of-damage check (`canDamage`).
    fn can_damage(&mut self, pool: &mut EntityPool, target: Slot, origin: Vec3) -> bool;
    /// Splash damage (`radiusDamage`).
    #[allow(clippy::too_many_arguments)]
    fn radius_damage(
        &mut self,
        pool: &mut EntityPool,
        origin: Vec3,
        attacker: Slot,
        damage: i32,
        radius: f32,
        ignore: Option<Slot>,
        method: i32,
        projectile: Option<ActorId>,
    ) -> bool;
    /// Accuracy attribution (`q3AccuracyHit`).
    fn accuracy_hit(&mut self, team_game: bool, target: &AccuracyTarget, attacker: &AccuracyTarget) -> bool;
    /// Whether an actor is a player.
    fn is_player(&self, pool: &EntityPool, actor: ActorId) -> bool {
        pool.native_by_actor(actor)
            .is_some_and(|slot| pool.get(slot).is_some_and(|entity| entity.client.is_some()))
    }
    /// Whether an actor is linked.
    fn linked_bounds(&self, pool: &EntityPool, actor: ActorId) -> bool {
        pool.native_by_actor(actor)
            .is_some_and(|slot| pool.get(slot).is_some_and(|entity| entity.linked))
    }
    /// Pooled participant for an actor.
    fn participant(&self, pool: &EntityPool, actor: ActorId) -> Option<Slot> {
        pool.native_by_actor(actor)
    }
    /// Authority record for an actor.
    fn authority(&self, pool: &EntityPool, actor: ActorId) -> Option<AuthorityState> {
        let slot = pool.native_by_actor(actor)?;
        let entity = pool.get(slot)?;
        Some(AuthorityState {
            can_take_damage: entity.takedamage,
            health: entity.health,
            team: entity.client.as_ref().map(|client| client.sess.session_team),
        })
    }
}

/// Game random host (`GameRandom`).
pub trait GameRandom {
    /// Nonnegative integer (`rand`).
    fn rand_int(&mut self) -> i32;
    /// Unit random (`random`).
    fn random(&mut self) -> f32;
    /// Signed unit random (`crandom`).
    fn crandom(&mut self) -> f32;
}

/// Mover core host (`MoverRuntime` surface used by mover-spawn).
pub trait MoverCore {
    /// Current time.
    fn time(&self) -> i32;
    /// Shared sound index.
    fn sound_index(&mut self, path: &str) -> i32;
    /// Use a binary mover (`useBinary`).
    fn use_binary(
        &mut self,
        pool: &mut EntityPool,
        entity: Slot,
        other: Option<DamageParticipant>,
        activator: Option<DamageParticipant>,
    ) -> Q3GameItemsResult<()>;
    /// Match a mover team to a state (`matchTeam`).
    fn match_team(
        &mut self,
        pool: &mut EntityPool,
        leader: Slot,
        state: MoverState,
        time: i32,
    ) -> Q3GameItemsResult<()>;
    /// Blocked-door handler (`blockedDoor`).
    fn blocked_door(&mut self, pool: &mut EntityPool, entity: Slot, other: DamageParticipant) -> Q3GameItemsResult<()>;
    /// Initialize a binary mover (`initializeBinary`).
    fn initialize_binary(
        &mut self,
        pool: &mut EntityPool,
        entity: Slot,
        variables: &SpawnVariables,
    ) -> Q3GameItemsResult<()>;
    /// Set mover state (`setState`).
    fn set_state(&mut self, pool: &mut EntityPool, entity: Slot, state: MoverState, time: i32)
        -> Q3GameItemsResult<()>;
}

/// Spawn variable value (`SpawnValue`).
#[derive(Debug, Clone, PartialEq)]
pub struct SpawnValue<T> {
    /// Whether the key was present.
    pub present: bool,
    /// Parsed value or parsed default.
    pub value: T,
}

/// Spawn variables (`SpawnVariables`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SpawnVariables {
    entries: Vec<(String, String)>,
}

impl SpawnVariables {
    /// Variables from key/value pairs.
    #[must_use]
    pub fn new(entries: Vec<(String, String)>) -> SpawnVariables {
        SpawnVariables { entries }
    }

    fn lookup(&self, key: &str) -> Option<&str> {
        let normalized = key.to_ascii_lowercase();
        self.entries
            .iter()
            .find(|pair| pair.0.to_ascii_lowercase() == normalized)
            .map(|pair| pair.1.as_str())
    }

    /// String value.
    #[must_use]
    pub fn string(&self, key: &str, default_value: &str) -> SpawnValue<String> {
        match self.lookup(key) {
            None => SpawnValue {
                present: false,
                value: default_value.to_string(),
            },
            Some(found) => SpawnValue {
                present: true,
                value: found.to_string(),
            },
        }
    }

    /// Integer value.
    #[must_use]
    pub fn int(&self, key: &str, default_value: &str) -> SpawnValue<i32> {
        let found = self.string(key, default_value);
        SpawnValue {
            present: found.present,
            value: game_atoi(&found.value),
        }
    }

    /// Float value.
    #[must_use]
    pub fn float(&self, key: &str, default_value: &str) -> SpawnValue<f32> {
        let found = self.string(key, default_value);
        SpawnValue {
            present: found.present,
            value: game_atof(&found.value),
        }
    }
}

/// Set an entity origin (`setOrigin`).
pub fn set_origin(pool: &mut EntityPool, slot: Slot, origin: Vec3) -> Q3GameItemsResult<()> {
    pool.require_owned(slot)?;
    let entity = &mut pool.entities[slot];
    entity.s.pos = Trajectory {
        ty: TrajectoryType::Stationary,
        time: 0,
        duration: 0,
        base: origin,
        delta: vec3(0.0, 0.0, 0.0),
    };
    entity.r.current_origin = origin;
    Ok(())
}

/// Run a due think callback (`runThink`); dispatch runs the named callback.
pub fn run_think(
    pool: &mut EntityPool,
    slot: Slot,
    time: i32,
    dispatch: &mut dyn FnMut(&mut EntityPool, Slot, CallbackName) -> Q3GameItemsResult<()>,
) -> Q3GameItemsResult<()> {
    pool.require_owned(slot)?;
    let think_time = pool.entities[slot].nextthink;
    if think_time <= 0 || think_time > time {
        return Ok(());
    }
    pool.entities[slot].nextthink = 0;
    if let Some(name) = pool.entities[slot].think {
        dispatch(pool, slot, name)?;
    }
    Ok(())
}

/// Ground entity number (`groundNumber`).
#[must_use]
pub fn ground_number(ground: Option<ActorId>, pool: &EntityPool) -> i32 {
    match ground.and_then(|actor| pool.native_by_actor(actor)) {
        Some(slot) => slot as i32,
        None => ENTITYNUM_NONE,
    }
}

/// Record trace ground (`traceGround`).
pub fn trace_ground(pool: &mut EntityPool, slot: Slot, hit: TraceHit) -> Q3GameItemsResult<()> {
    pool.require_owned(slot)?;
    let ground = match hit {
        TraceHit::Actor(actor) => Some(actor),
        TraceHit::World => Some(ActorId::from_slot(ENTITYNUM_WORLD)),
        TraceHit::None => None,
    };
    let number = ground_number(ground, pool);
    let entity = &mut pool.entities[slot];
    entity.ground = ground;
    entity.s.ground_entity_num = number;
    Ok(())
}

/// Move direction (`moveDirection`).
#[must_use]
pub fn move_direction(angles: Vec3) -> (Vec3, Vec3) {
    let vertical = angles.x == 0.0 && angles.z == 0.0;
    let direction = if vertical && angles.y == -1.0 {
        vec3(0.0, 0.0, 1.0)
    } else if vertical && angles.y == -2.0 {
        vec3(0.0, 0.0, -1.0)
    } else {
        qvm_angle_vectors(angles).forward
    };
    (direction, vec3(0.0, 0.0, 0.0))
}

/// String-valued entity field selector (`EntityStringField`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EntityStringField {
    /// Class name.
    Classname,
    /// Target.
    Target,
    /// Targetname.
    Targetname,
    /// Model.
    Model,
}

pub(crate) fn ascii_fold(value: &str) -> String {
    let cut = value.find('\0').map_or(value, |index| &value[..index]);
    cut.bytes()
        .map(|byte| {
            if byte.is_ascii_uppercase() {
                (byte + 32) as char
            } else {
                byte as char
            }
        })
        .collect()
}

pub(crate) fn string_field(entity: &GameEntity, field: EntityStringField) -> Option<&str> {
    match field {
        EntityStringField::Classname => entity.classname.as_deref(),
        EntityStringField::Target => entity.target.as_deref(),
        EntityStringField::Targetname => entity.targetname.as_deref(),
        EntityStringField::Model => entity.model.as_deref(),
    }
}

/// Find an entity by string field (`findEntity`).
#[must_use]
pub fn find_entity(
    pool: &EntityPool,
    after: Option<Slot>,
    field: EntityStringField,
    value: Option<&str>,
) -> Option<Slot> {
    let wanted = ascii_fold(value?);
    let start = after.map_or(0, |slot| slot + 1);
    for index in start..pool.num_entities() {
        let Ok(entity) = pool.at(index) else {
            continue;
        };
        if !entity.inuse {
            continue;
        }
        if let Some(text) = string_field(entity, field) {
            if ascii_fold(text) == wanted {
                return Some(index);
            }
        }
    }
    None
}

/// Target selection services (`TargetSelectionContext` pool-free half).
pub struct TargetSelection<'a> {
    /// Nonnegative random integer.
    pub random_int: &'a mut dyn FnMut() -> i32,
    /// Warning sink.
    pub warn: &'a mut dyn FnMut(&str),
}

/// Pick a target by targetname (`pickTarget`).
pub fn pick_target(
    pool: &EntityPool,
    selection: &mut TargetSelection<'_>,
    target_name: Option<&str>,
) -> Q3GameItemsResult<Option<Slot>> {
    let Some(target_name) = target_name else {
        (selection.warn)("G_PickTarget called with NULL targetname\n");
        return Ok(None);
    };
    let mut choices = Vec::new();
    let mut found = None;
    while choices.len() < 32 {
        found = find_entity(pool, found, EntityStringField::Targetname, Some(target_name));
        let Some(slot) = found else {
            break;
        };
        choices.push(slot);
    }
    if choices.is_empty() {
        (selection.warn)(&format!("G_PickTarget: target {target_name} not found\n"));
        return Ok(None);
    }
    let random = (selection.random_int)();
    if random < 0 {
        return Err(range("game rand must return a nonnegative integer"));
    }
    let choice = choices[(random as usize) % choices.len()];
    Ok(Some(choice))
}

pub(crate) fn source_snap_component(component: f32) -> f32 {
    if (-2_147_483_648.0..2_147_483_648.0).contains(&component) {
        component.trunc()
    } else {
        -2_147_483_648.0
    }
}

pub(crate) fn copy_position(value: Vec3, snap: bool) -> Vec3 {
    if snap {
        vec3(
            source_snap_component(value.x),
            source_snap_component(value.y),
            source_snap_component(value.z),
        )
    } else {
        value
    }
}

/// Publish player state to entity state (`playerStateToEntityState`).
pub fn player_state_to_entity_state(pool: &mut EntityPool, slot: Slot, snap: bool) -> Q3GameItemsResult<()> {
    pool.require_owned(slot)?;
    let entity = &mut pool.entities[slot];
    let (client_slot, s) = (&mut entity.client, &mut entity.s);
    let Some(client) = client_slot.as_mut() else {
        return Err(invalid("player state sync requires a client entity"));
    };
    let ps = &mut client.ps;
    s.e_type = if ps.pm_type == MoveType::Intermission || ps.pm_type == MoveType::Spectator || ps.health <= GIB_HEALTH {
        EntityType::Invisible as i32
    } else {
        EntityType::Player as i32
    };
    s.number = ps.client_num;
    s.pos = Trajectory {
        ty: TrajectoryType::Interpolate,
        time: s.pos.time,
        duration: s.pos.duration,
        base: copy_position(ps.origin, snap),
        delta: ps.velocity,
    };
    s.apos = Trajectory {
        ty: TrajectoryType::Interpolate,
        time: s.apos.time,
        duration: s.apos.duration,
        base: copy_position(ps.viewangles, snap),
        delta: s.apos.delta,
    };
    s.angles2 = vec3(s.angles2.x, ps.movement_dir, s.angles2.z);
    s.legs_anim = ps.legs_anim;
    s.torso_anim = ps.torso_anim;
    s.client_num = ps.client_num;
    s.e_flags = if ps.health <= 0 {
        ps.e_flags | 1
    } else {
        ps.e_flags & !1
    };
    if ps.external_event != 0 {
        s.event = ps.external_event;
        s.event_parm = ps.external_event_parm;
    } else if ps.entity_event_sequence < ps.event_sequence {
        let oldest = ps.event_sequence.wrapping_sub(2);
        if ps.entity_event_sequence < oldest {
            ps.entity_event_sequence = oldest;
        }
        let index = (ps.entity_event_sequence & 1) as usize;
        s.event = ps.events.get(index)? | ((ps.entity_event_sequence & 3) << 8);
        s.event_parm = ps.event_parms.get(index)?;
        ps.entity_event_sequence = ps.entity_event_sequence.wrapping_add(1);
    }
    s.weapon = ps.weapon;
    s.ground_entity_num = ps.ground_entity_num;
    s.powerups = 0;
    for index in 0..ps.powerups.len() {
        if ps.powerups.get(index)? != 0 {
            s.powerups |= 1 << index;
        }
    }
    s.loop_sound = ps.loop_sound;
    s.generic1 = ps.generic1;
    Ok(())
}

/// Set a client view angle (`setClientViewAngle`).
pub fn set_client_view_angle(pool: &mut EntityPool, slot: Slot, angles: Vec3) -> Q3GameItemsResult<()> {
    pool.require_owned(slot)?;
    let delta = |angle: f32, command: i32| -> i32 {
        let scaled = angle * 65536.0 / 360.0;
        let short = qvm_float_to_int(scaled) & 65535;
        short.wrapping_sub(command)
    };
    let entity = &mut pool.entities[slot];
    let Some(client) = entity.client.as_mut() else {
        return Err(invalid("view angle update requires a client entity"));
    };
    let command = client.pers.cmd_angles;
    client.ps.delta_angles = [
        delta(angles.x, command[0]),
        delta(angles.y, command[1]),
        delta(angles.z, command[2]),
    ];
    entity.s.angles = angles;
    if let Some(client) = entity.client.as_mut() {
        client.ps.viewangles = entity.s.angles;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q3::base::game::missile::*;
    use crate::q3::base::game::mover_spawn::*;
    use qa_core::math::scale3;
    use qa_core::math::vec3;
    use qa_core::math::Bounds;
    use qa_core::math::Vec3;

    use crate::q3::base::game::item_motion::*;
    use crate::q3::base::game::item_pickup::*;
    use crate::q3::base::game::level::*;
    use crate::q3::base::game::memory::*;
    use crate::q3::base::game::misc::*;
    use crate::q3::base::game::misc_spawn::*;

    fn item_def(class_name: &str, quantity: i32, kind: ItemKind) -> ItemDefinition {
        ItemDefinition {
            class_name: Some(class_name.to_string()),
            quantity,
            kind,
        }
    }

    struct TestItems {
        list: Vec<ItemDefinition>,
    }

    fn test_items() -> TestItems {
        TestItems {
            list: vec![
                ItemDefinition {
                    class_name: None,
                    quantity: 0,
                    kind: ItemKind::Bad,
                },
                item_def("weapon_rocketlauncher", 10, ItemKind::Weapon(Weapon::RocketLauncher)),
                item_def("weapon_plasmagun", 50, ItemKind::Weapon(Weapon::Plasmagun)),
                item_def("weapon_grenadelauncher", 5, ItemKind::Weapon(Weapon::GrenadeLauncher)),
                item_def("ammo_rockets", 5, ItemKind::Ammo(Weapon::RocketLauncher)),
                item_def("item_armor_shard", 5, ItemKind::Armor),
                item_def("item_health_small", 5, ItemKind::Health),
                item_def("item_health_mega", 100, ItemKind::Health),
                item_def("item_quad", 30, ItemKind::Powerup(Powerup::Quad)),
                item_def("holdable_kamikaze", 1, ItemKind::Holdable(Holdable::Kamikaze)),
                item_def("item_guard", 1, ItemKind::PersistantPowerup(Powerup::Guard)),
                item_def("team_CTF_redflag", 0, ItemKind::Team(Powerup::RedFlag)),
            ],
        }
    }

    impl ItemTable for TestItems {
        fn index_of(&self, _product: Product, item: &ItemDefinition) -> Option<usize> {
            self.list.iter().position(|candidate| candidate == item)
        }

        fn item_at(&self, _product: Product, index: usize) -> Q3GameItemsResult<ItemDefinition> {
            self.list
                .get(index)
                .cloned()
                .ok_or_else(|| range(format!("item index {index} out of range")))
        }

        fn find_item_for_weapon(&self, _product: Product, weapon: Weapon) -> Q3GameItemsResult<ItemDefinition> {
            self.list
                .iter()
                .find(|item| item.kind == ItemKind::Weapon(weapon))
                .cloned()
                .ok_or_else(|| drop_error(format!("couldn't find item for weapon {}", weapon as i32)))
        }
    }

    struct DamageCall {
        amount: i32,
        flags: i32,
        method: i32,
    }

    struct TestCombat {
        time: i32,
        previous_time: i32,
        game_type: i32,
        product: Product,
        damage_calls: Vec<DamageCall>,
        can_damage_value: bool,
        radius_value: bool,
        accuracy_value: bool,
    }

    impl TestCombat {
        fn new() -> TestCombat {
            TestCombat {
                time: 1000,
                previous_time: 900,
                game_type: GameType::Ffa as i32,
                product: Product::Baseq3,
                damage_calls: Vec::new(),
                can_damage_value: true,
                radius_value: true,
                accuracy_value: true,
            }
        }
    }

    impl CombatOps for TestCombat {
        fn time(&self) -> i32 {
            self.time
        }

        fn previous_time(&self) -> i32 {
            self.previous_time
        }

        fn game_type(&self) -> i32 {
            self.game_type
        }

        fn product(&self) -> Product {
            self.product
        }

        fn damage(
            &mut self,
            _pool: &mut EntityPool,
            _target: Slot,
            _inflictor: DamageParticipant,
            _attacker: DamageParticipant,
            _direction: Option<Vec3>,
            _point: Option<Vec3>,
            amount: i32,
            flags: i32,
            method: i32,
            _projectile: Option<ActorId>,
        ) -> Q3GameItemsResult<()> {
            self.damage_calls.push(DamageCall { amount, flags, method });
            Ok(())
        }

        fn can_damage(&mut self, _pool: &mut EntityPool, _target: Slot, _origin: Vec3) -> bool {
            self.can_damage_value
        }

        fn radius_damage(
            &mut self,
            _pool: &mut EntityPool,
            _origin: Vec3,
            _attacker: Slot,
            _damage: i32,
            _radius: f32,
            _ignore: Option<Slot>,
            _method: i32,
            _projectile: Option<ActorId>,
        ) -> bool {
            self.radius_value
        }

        fn accuracy_hit(&mut self, _team_game: bool, _target: &AccuracyTarget, _attacker: &AccuracyTarget) -> bool {
            self.accuracy_value
        }
    }

    struct TestWorld {
        trace_result: ActorTraceResult,
        contents: i32,
        actors: Vec<ActorId>,
        link_log: Vec<Slot>,
        unlink_log: Vec<i32>,
    }

    impl TestWorld {
        fn clear_trace() -> ActorTraceResult {
            ActorTraceResult {
                fraction: 1.0,
                end: vec3(10.0, 20.0, 30.0),
                solidity: TraceSolidity::Clear,
                contact: TraceContact::None,
                contents: 0,
                surface_flags: 0,
                hit: TraceHit::None,
            }
        }

        fn new() -> TestWorld {
            TestWorld {
                trace_result: TestWorld::clear_trace(),
                contents: 0,
                actors: Vec::new(),
                link_log: Vec::new(),
                unlink_log: Vec::new(),
            }
        }
    }

    impl WorldOps for TestWorld {
        fn link(&mut self, pool: &mut EntityPool, slot: Slot) -> Q3GameItemsResult<()> {
            pool.link(slot)?;
            self.link_log.push(slot);
            Ok(())
        }

        fn unlink(&mut self, pool: &mut EntityPool, number: i32) -> Q3GameItemsResult<()> {
            if number >= 0 {
                pool.unlink(number as usize)?;
            }
            self.unlink_log.push(number);
            Ok(())
        }

        fn trace_actor(&mut self, _pool: &EntityPool, _query: &ActorTraceQuery) -> ActorTraceResult {
            self.trace_result
        }

        fn point_contents(&mut self, _pool: &EntityPool, _point: Vec3, _pass: i32) -> i32 {
            self.contents
        }

        fn area_actors(&mut self, _pool: &EntityPool, _bounds: Bounds, maximum: usize) -> Vec<ActorId> {
            self.actors.iter().take(maximum).copied().collect()
        }
    }

    struct TestRandom {
        int_value: i32,
        random_value: f32,
        crandom_value: f32,
    }

    impl GameRandom for TestRandom {
        fn rand_int(&mut self) -> i32 {
            self.int_value
        }

        fn random(&mut self) -> f32 {
            self.random_value
        }

        fn crandom(&mut self) -> f32 {
            self.crandom_value
        }
    }

    struct TestDriver {
        launches: Vec<ProjectileLaunch>,
        bounces: usize,
        explodes: usize,
        impacts: usize,
        steps: usize,
    }

    impl TestDriver {
        fn new() -> TestDriver {
            TestDriver {
                launches: Vec::new(),
                bounces: 0,
                explodes: 0,
                impacts: 0,
                steps: 0,
            }
        }
    }

    impl ProjectileDriver for TestDriver {
        fn launch(
            &mut self,
            start: Vec3,
            direction: Vec3,
            speed: f32,
            gravity: bool,
            duration: i32,
            time: i32,
        ) -> ProjectileLaunch {
            let fired = ProjectileLaunch {
                expires: time.wrapping_add(duration),
                trajectory: Trajectory {
                    ty: if gravity {
                        TrajectoryType::Gravity
                    } else {
                        TrajectoryType::Linear
                    },
                    time: time.wrapping_sub(50),
                    duration: 0,
                    base: start,
                    delta: snap_vector(scale3(direction, speed)),
                },
            };
            self.launches.push(fired);
            fired
        }

        fn bounce(&mut self, context: &mut ProjectileContext<'_>, _trace: &ActorTraceResult) -> Q3GameItemsResult<()> {
            self.bounces += 1;
            context.state.trajectory.delta = vec3(1.0, 2.0, 3.0);
            Ok(())
        }

        fn explode(&mut self, context: &mut ProjectileContext<'_>) -> Q3GameItemsResult<()> {
            self.explodes += 1;
            context.emit(&ImpactEmit::Impact {
                normal: vec3(0.0, 0.0, 1.0),
                target: None,
                flesh: false,
                surface_flags: 0,
            })?;
            context.retain()
        }

        fn impact(&mut self, context: &mut ProjectileContext<'_>, trace: &ActorTraceResult) -> Q3GameItemsResult<()> {
            self.impacts += 1;
            context.emit(&ImpactEmit::Impact {
                normal: trace_normal(trace),
                target: None,
                flesh: false,
                surface_flags: trace.surface_flags,
            })
        }

        fn step(&mut self, context: &mut ProjectileContext<'_>) -> Q3GameItemsResult<()> {
            self.steps += 1;
            context.think(self)
        }
    }

    struct TestMissileHost {
        combat: TestCombat,
        world: TestWorld,
        bodies: BodyTable,
        random: TestRandom,
        missionpack: bool,
        prox_timeout: i32,
        sounds: Vec<String>,
        invuln_outcome: InvulnerabilityOutcome,
        behavior_launch: Option<BodyState>,
        behavior_step: Option<BodyState>,
    }

    impl TestMissileHost {
        fn base() -> TestMissileHost {
            TestMissileHost {
                combat: TestCombat::new(),
                world: TestWorld::new(),
                bodies: BodyTable::default(),
                random: TestRandom {
                    int_value: 0,
                    random_value: 0.5,
                    crandom_value: 0.0,
                },
                missionpack: false,
                prox_timeout: 30000,
                sounds: Vec::new(),
                invuln_outcome: InvulnerabilityOutcome::Miss,
                behavior_launch: None,
                behavior_step: None,
            }
        }

        fn missionpack() -> TestMissileHost {
            let mut host = TestMissileHost::base();
            host.missionpack = true;
            host.combat.product = Product::Missionpack;
            host
        }
    }

    impl MissileHost for TestMissileHost {
        fn combat(&mut self) -> &mut dyn CombatOps {
            &mut self.combat
        }

        fn world(&mut self) -> &mut dyn WorldOps {
            &mut self.world
        }

        fn combat_and_world(&mut self) -> (&mut dyn CombatOps, &mut dyn WorldOps) {
            (&mut self.combat, &mut self.world)
        }

        fn bodies(&mut self) -> &mut BodyTable {
            &mut self.bodies
        }

        fn random(&mut self) -> &mut dyn GameRandom {
            &mut self.random
        }

        fn has_weapon_behavior(&self) -> bool {
            self.behavior_launch.is_some() || self.behavior_step.is_some()
        }

        fn weapon_behavior_launch(
            &mut self,
            _projectile: ActorId,
            _shooter: ActorId,
            _weapon: Weapon,
            _time_seconds: f64,
            _origin: Vec3,
            _velocity: Vec3,
        ) -> Option<BodyState> {
            self.behavior_launch
        }

        fn weapon_behavior_step(
            &mut self,
            _projectile: ActorId,
            _origin: Vec3,
            _velocity: Vec3,
            _time_seconds: f64,
        ) -> Option<BodyState> {
            self.behavior_step
        }

        fn is_missionpack(&self) -> bool {
            self.missionpack
        }

        fn prox_mine_timeout(&self) -> i32 {
            self.prox_timeout
        }

        fn missionpack_sound_index(&mut self, path: &str) -> i32 {
            self.sounds.push(path.to_string());
            self.sounds.len() as i32
        }

        fn invulnerability_impact(
            &mut self,
            _pool: &mut EntityPool,
            _target: Slot,
            _direction: Vec3,
            _point: Vec3,
        ) -> InvulnerabilityOutcome {
            self.invuln_outcome
        }
    }

    struct TestRegistry {
        product: Product,
        registered: Vec<ItemDefinition>,
    }

    impl ItemRegistry for TestRegistry {
        fn product(&self) -> Product {
            self.product
        }

        fn register(&mut self, item: &ItemDefinition) {
            self.registered.push(item.clone());
        }
    }

    struct TestMovers {
        time: i32,
        sounds: Vec<String>,
        use_log: Vec<Slot>,
        match_log: Vec<(Slot, MoverState)>,
        init_log: Vec<Slot>,
        state_log: Vec<(Slot, MoverState)>,
        blocked_log: Vec<Slot>,
    }

    impl MoverCore for TestMovers {
        fn time(&self) -> i32 {
            self.time
        }

        fn sound_index(&mut self, path: &str) -> i32 {
            self.sounds.push(path.to_string());
            self.sounds.len() as i32
        }

        fn use_binary(
            &mut self,
            _pool: &mut EntityPool,
            entity: Slot,
            _other: Option<DamageParticipant>,
            _activator: Option<DamageParticipant>,
        ) -> Q3GameItemsResult<()> {
            self.use_log.push(entity);
            Ok(())
        }

        fn match_team(
            &mut self,
            _pool: &mut EntityPool,
            leader: Slot,
            state: MoverState,
            _time: i32,
        ) -> Q3GameItemsResult<()> {
            self.match_log.push((leader, state));
            Ok(())
        }

        fn blocked_door(
            &mut self,
            _pool: &mut EntityPool,
            entity: Slot,
            _other: DamageParticipant,
        ) -> Q3GameItemsResult<()> {
            self.blocked_log.push(entity);
            Ok(())
        }

        fn initialize_binary(
            &mut self,
            _pool: &mut EntityPool,
            entity: Slot,
            _variables: &SpawnVariables,
        ) -> Q3GameItemsResult<()> {
            self.init_log.push(entity);
            Ok(())
        }

        fn set_state(
            &mut self,
            pool: &mut EntityPool,
            entity: Slot,
            state: MoverState,
            _time: i32,
        ) -> Q3GameItemsResult<()> {
            pool.at_mut(entity)?.mover_state = state;
            self.state_log.push((entity, state));
            Ok(())
        }
    }

    struct TestMoverSpawnHost {
        movers: TestMovers,
        combat: TestCombat,
        world: TestWorld,
        warnings: Vec<String>,
        used_targets: Vec<Slot>,
        brush_models: Vec<Slot>,
    }

    impl TestMoverSpawnHost {
        fn new() -> TestMoverSpawnHost {
            TestMoverSpawnHost {
                movers: TestMovers {
                    time: 500,
                    sounds: Vec::new(),
                    use_log: Vec::new(),
                    match_log: Vec::new(),
                    init_log: Vec::new(),
                    state_log: Vec::new(),
                    blocked_log: Vec::new(),
                },
                combat: TestCombat::new(),
                world: TestWorld::new(),
                warnings: Vec::new(),
                used_targets: Vec::new(),
                brush_models: Vec::new(),
            }
        }
    }

    impl MoverSpawnHost for TestMoverSpawnHost {
        fn movers(&mut self) -> &mut dyn MoverCore {
            &mut self.movers
        }

        fn combat_and_world(&mut self) -> (&mut dyn CombatOps, &mut dyn WorldOps) {
            (&mut self.combat, &mut self.world)
        }

        fn gravity(&self) -> f32 {
            800.0
        }

        fn set_brush_model(&mut self, pool: &mut EntityPool, slot: Slot, _name: Option<&str>) -> Q3GameItemsResult<()> {
            self.brush_models.push(slot);
            pool.at_mut(slot)?.r.mins = vec3(-16.0, -16.0, -24.0);
            pool.at_mut(slot)?.r.maxs = vec3(16.0, 16.0, 32.0);
            pool.at_mut(slot)?.r.absmin = vec3(-16.0, -16.0, -24.0);
            pool.at_mut(slot)?.r.absmax = vec3(16.0, 16.0, 32.0);
            Ok(())
        }

        fn remap_shader(&mut self, _old_name: &str, _new_name: &str, _time_seconds: f32) {}

        fn use_targets(
            &mut self,
            _pool: &mut EntityPool,
            entity: Slot,
            _activator: Option<DamageParticipant>,
        ) -> Q3GameItemsResult<()> {
            self.used_targets.push(entity);
            Ok(())
        }

        fn warn(&mut self, message: &str) {
            self.warnings.push(message.to_string());
        }
    }

    fn player_slot(pool: &mut EntityPool, product: Product) -> Slot {
        let slot = pool.spawn().unwrap();
        pool.at_mut(slot).unwrap().client = Some(GameClient::new(product));
        pool.at_mut(slot).unwrap().health = 100;
        pool.at_mut(slot).unwrap().takedamage = true;
        slot
    }

    fn item_slot(pool: &mut EntityPool, items: &TestItems, index: usize) -> Slot {
        let slot = pool.spawn().unwrap();
        pool.at_mut(slot).unwrap().item = Some(items.list[index].clone());
        slot
    }

    #[test]
    fn trajectory_evaluation_matches_donor() {
        let linear = Trajectory {
            ty: TrajectoryType::Linear,
            time: 1000,
            duration: 0,
            base: vec3(0.0, 0.0, 0.0),
            delta: vec3(100.0, 0.0, 0.0),
        };
        assert_eq!(evaluate_trajectory(&linear, 1500), vec3(50.0, 0.0, 0.0));
        assert_eq!(evaluate_trajectory_delta(&linear, 9999), vec3(100.0, 0.0, 0.0));
        let gravity = Trajectory {
            ty: TrajectoryType::Gravity,
            ..linear
        };
        let at = evaluate_trajectory(&gravity, 2000);
        assert!((at.x - 100.0).abs() < 0.001);
        assert!((at.z + 400.0).abs() < 0.5);
        let delta = evaluate_trajectory_delta(&gravity, 2000);
        assert!((delta.z + 800.0).abs() < 0.5);
        let stop = Trajectory {
            ty: TrajectoryType::LinearStop,
            duration: 500,
            ..linear
        };
        assert_eq!(evaluate_trajectory(&stop, 2000), vec3(50.0, 0.0, 0.0));
        assert_eq!(evaluate_trajectory_delta(&stop, 2000), vec3(0.0, 0.0, 0.0));
        let sine = Trajectory {
            ty: TrajectoryType::Sine,
            duration: 1000,
            delta: vec3(0.0, 0.0, 10.0),
            ..linear
        };
        let mid = evaluate_trajectory(&sine, 1250);
        assert!((mid.z - 10.0).abs() < 0.01, "{mid:?}");
        let still = Trajectory::default();
        assert_eq!(evaluate_trajectory(&still, 4242), vec3(0.0, 0.0, 0.0));
        assert_eq!(evaluate_trajectory_delta(&still, 4242), vec3(0.0, 0.0, 0.0));
    }

    #[test]
    fn angle_vectors_and_tables() {
        let forward = qvm_angle_vectors(vec3(0.0, 90.0, 0.0)).forward;
        assert!(forward.x.abs() < 1e-5);
        assert!((forward.y - 1.0).abs() < 1e-5);
        assert_eq!(direction_to_byte(None), 0);
        assert_eq!(direction_to_byte(Some(vec3(0.0, 0.0, 1.0))), 5);
        assert_eq!(BYTE_DIRECTIONS.len(), 162);
        assert_eq!(weapon_count(Product::Baseq3), 11);
        assert_eq!(weapon_count(Product::Missionpack), 14);
        assert_eq!(stat_schema(Product::Baseq3).weapons, 2);
        assert_eq!(stat_schema(Product::Missionpack).weapons, 3);
    }

    #[test]
    fn game_number_parsing() {
        assert_eq!(game_atoi("  -12x"), -12);
        assert_eq!(game_atoi("+7"), 7);
        assert_eq!(game_atoi(""), 0);
        assert_eq!(game_atoi("abc"), 0);
        assert_eq!(game_atoi("9999999999"), 1_410_065_407);
        assert_eq!(game_atof("3.5"), 3.5);
        assert_eq!(game_atof(".5"), 0.5);
        assert_eq!(game_atof("-2.25 ignored"), -2.25);
        assert_eq!(game_atof(""), 0.0);
        assert_eq!(game_atof("10"), 10.0);
    }

    #[test]
    fn item_tables_and_quantities() {
        let items = test_items();
        let weapon_ctx = WeaponPickupContext {
            game_type: GameType::Ffa as i32,
            weapon_respawn_seconds: 5,
            team_weapon_respawn_seconds: 30,
        };
        assert_eq!(q3_item_respawn_seconds(&items.list[1], &weapon_ctx).unwrap(), 5);
        assert_eq!(
            q3_item_respawn_seconds(&items.list[4], &weapon_ctx).unwrap(),
            RESPAWN_AMMO
        );
        assert_eq!(q3_item_respawn_seconds(&items.list[5], &weapon_ctx).unwrap(), 25);
        assert_eq!(q3_item_respawn_seconds(&items.list[6], &weapon_ctx).unwrap(), 35);
        assert_eq!(q3_item_respawn_seconds(&items.list[7], &weapon_ctx).unwrap(), 35);
        assert_eq!(q3_item_respawn_seconds(&items.list[8], &weapon_ctx).unwrap(), 120);
        assert_eq!(q3_item_respawn_seconds(&items.list[9], &weapon_ctx).unwrap(), 60);
        assert_eq!(q3_item_respawn_seconds(&items.list[10], &weapon_ctx).unwrap(), -1);
        assert!(q3_item_respawn_seconds(&items.list[11], &weapon_ctx).is_err());
        assert!(q3_item_respawn_seconds(&items.list[0], &weapon_ctx).is_err());
        let team_ctx = WeaponPickupContext {
            game_type: GameType::Team as i32,
            ..weapon_ctx
        };
        assert_eq!(q3_weapon_respawn_seconds(&team_ctx), 30);
        assert_eq!(
            q3_weapon_pickup_quantity(&WeaponPickupQuantity {
                count: -1,
                quantity: 10,
                dropped: false,
                game_type: 0,
                current_ammo: 0
            }),
            0
        );
        assert_eq!(
            q3_weapon_pickup_quantity(&WeaponPickupQuantity {
                count: 0,
                quantity: 10,
                dropped: false,
                game_type: 0,
                current_ammo: 3
            }),
            7
        );
        assert_eq!(
            q3_weapon_pickup_quantity(&WeaponPickupQuantity {
                count: 0,
                quantity: 10,
                dropped: false,
                game_type: 0,
                current_ammo: 50
            }),
            1
        );
        assert_eq!(
            q3_weapon_pickup_quantity(&WeaponPickupQuantity {
                count: 0,
                quantity: 10,
                dropped: true,
                game_type: 0,
                current_ammo: 3
            }),
            10
        );
    }

    #[test]
    fn pickup_flows_cover_all_types() {
        let items = test_items();
        let mut pool = EntityPool::new(Product::Baseq3);
        let player = player_slot(&mut pool, Product::Baseq3);
        let ammo = item_slot(&mut pool, &items, 4);
        assert_eq!(pickup_ammo(&mut pool, &items, ammo, player).unwrap(), 40);
        assert_eq!(
            pool.at(player)
                .unwrap()
                .client
                .as_ref()
                .unwrap()
                .ps
                .ammo
                .get(Weapon::RocketLauncher as usize)
                .unwrap(),
            5
        );
        let weapon = item_slot(&mut pool, &items, 1);
        let ctx = WeaponPickupContext {
            game_type: GameType::Ffa as i32,
            weapon_respawn_seconds: 5,
            team_weapon_respawn_seconds: 30,
        };
        assert_eq!(pickup_weapon(&mut pool, &items, weapon, player, &ctx).unwrap(), 5);
        let client = pool.at(player).unwrap().client.as_ref().unwrap();
        assert_ne!(
            client.ps.stats.get(2).unwrap() & (1 << Weapon::RocketLauncher as i32),
            0
        );
        let armor = item_slot(&mut pool, &items, 5);
        pool.at_mut(player)
            .unwrap()
            .client
            .as_mut()
            .unwrap()
            .ps
            .stats
            .set(6, 100)
            .unwrap();
        assert_eq!(pickup_armor(&mut pool, &items, armor, player).unwrap(), 25);
        let health = item_slot(&mut pool, &items, 6);
        pool.at_mut(player).unwrap().health = 90;
        assert_eq!(pickup_health(&mut pool, &items, health, player).unwrap(), 35);
        assert_eq!(pool.at(player).unwrap().health, 95);
        let holdable = item_slot(&mut pool, &items, 9);
        assert_eq!(pickup_holdable(&mut pool, &items, holdable, player).unwrap(), 60);
        let client = pool.at(player).unwrap().client.as_ref().unwrap();
        assert_eq!(client.ps.stats.get(1).unwrap(), 9);
        assert_ne!(client.ps.e_flags & 0x200, 0);
        let mut trace = |_a: Vec3, _b: Vec3| PowerupSightTrace { fraction: 1.0 };
        let mut powerup_ctx = PowerupPickupContext {
            time: 1234,
            game_type: GameType::Ffa as i32,
            trace_solid_line: &mut trace,
        };
        let powerup = item_slot(&mut pool, &items, 8);
        assert_eq!(
            pickup_powerup(&mut pool, &items, powerup, player, &mut powerup_ctx).unwrap(),
            120
        );
        let client = pool.at(player).unwrap().client.as_ref().unwrap();
        assert_eq!(client.ps.powerups.get(Powerup::Quad as usize).unwrap(), 1000 + 30_000);
        let mut handicap = |_client: i32| "100".to_string();
        let mut sight = |_a: Vec3, _b: Vec3| PowerupSightTrace { fraction: 1.0 };
        let mut full = ItemPickupContext {
            game_type: GameType::Ffa as i32,
            weapon_respawn_seconds: 5,
            team_weapon_respawn_seconds: 30,
            time: 2000,
            trace_solid_line: &mut sight,
            handicap_for_client: &mut handicap,
        };
        let ammo2 = item_slot(&mut pool, &items, 4);
        assert_eq!(pickup_item(&mut pool, &items, ammo2, player, &mut full).unwrap(), 40);
        let flag = item_slot(&mut pool, &items, 11);
        assert!(pickup_item(&mut pool, &items, flag, player, &mut full).is_err());
    }

    #[test]
    fn persistent_powerup_and_denied_reward() {
        let items = test_items();
        let mut pool = EntityPool::new(Product::Missionpack);
        let player = player_slot(&mut pool, Product::Missionpack);
        let guard = item_slot(&mut pool, &items, 10);
        assert_eq!(
            pickup_persistent_powerup(&mut pool, &items, guard, player, "80").unwrap(),
            -1
        );
        let client = pool.at(player).unwrap().client.as_ref().unwrap();
        assert_eq!(client.ps.stats.get(2).unwrap(), 10);
        assert_eq!(client.pers.max_health, 160);
        assert_eq!(pool.at(player).unwrap().health, 160);
        // Witness facing the pickup sees a denied reward toggle.
        let witness = player_slot(&mut pool, Product::Missionpack);
        pool.at_mut(witness).unwrap().client.as_mut().unwrap().pers.connected = ConnectionState::Connected;
        pool.at_mut(witness)
            .unwrap()
            .client
            .as_mut()
            .unwrap()
            .ps
            .stats
            .set(0, 100)
            .unwrap();
        pool.at_mut(witness).unwrap().client.as_mut().unwrap().ps.origin = vec3(100.0, 0.0, 0.0);
        pool.at_mut(witness).unwrap().client.as_mut().unwrap().ps.viewangles = vec3(0.0, 180.0, 0.0);
        let powerup = item_slot(&mut pool, &items, 8);
        pool.at_mut(powerup).unwrap().s.pos.base = vec3(0.0, 0.0, 0.0);
        let mut trace = |_a: Vec3, _b: Vec3| PowerupSightTrace { fraction: 1.0 };
        let mut ctx = PowerupPickupContext {
            time: 2000,
            game_type: GameType::Ffa as i32,
            trace_solid_line: &mut trace,
        };
        pickup_powerup(&mut pool, &items, powerup, player, &mut ctx).unwrap();
        let witness_client = pool.at(witness).unwrap().client.as_ref().unwrap();
        assert_eq!(witness_client.ps.persistant.get(5).unwrap(), 1);
        // Baseq3 rejects persistent powerups.
        let mut base_pool = EntityPool::new(Product::Baseq3);
        let base_player = player_slot(&mut base_pool, Product::Baseq3);
        let base_guard = item_slot(&mut base_pool, &items, 10);
        assert!(pickup_persistent_powerup(&mut base_pool, &items, base_guard, base_player, "100").is_err());
    }

    #[test]
    fn launch_and_drop_items() {
        let items = test_items();
        let mut pool = EntityPool::new(Product::Baseq3);
        let mut checked = Vec::new();
        let mut check = |pool: &mut EntityPool, slot: Slot| {
            checked.push(slot);
            let _ = pool;
            Ok(())
        };
        let mut ctx = LaunchItemContext {
            product: Product::Baseq3,
            game_type: GameType::Ffa as i32,
            time: 1000,
            items: &items,
            check_dropped_team_item: &mut check,
        };
        let slot = launch_item(
            &mut pool,
            &mut ctx,
            &items.list[4],
            vec3(1.0, 2.0, 3.0),
            vec3(0.0, 0.0, 10.0),
        )
        .unwrap();
        let entity = pool.at(slot).unwrap();
        assert_eq!(entity.s.e_type, EntityType::Item as i32);
        assert_eq!(entity.s.modelindex, 4);
        assert_eq!(entity.think, Some(CallbackName(LAUNCH_ITEM_THINK)));
        assert_eq!(entity.nextthink, 31000);
        assert!(checked.is_empty());
        launch_item_think(&mut pool, slot).unwrap();
        assert!(pool.get(slot).is_none());
        // Team drop in CTF uses the flag think and runs the check.
        let mut checked = Vec::new();
        let mut check = |_pool: &mut EntityPool, slot: Slot| {
            checked.push(slot);
            Ok(())
        };
        let mut ctx = LaunchItemContext {
            product: Product::Baseq3,
            game_type: GameType::Ctf as i32,
            time: 1000,
            items: &items,
            check_dropped_team_item: &mut check,
        };
        let flag = launch_item(
            &mut pool,
            &mut ctx,
            &items.list[11],
            vec3(0.0, 0.0, 0.0),
            vec3(0.0, 0.0, 0.0),
        )
        .unwrap();
        assert_eq!(pool.at(flag).unwrap().think, Some(CallbackName(DROPPED_FLAG_THINK)));
        assert_eq!(checked, vec![flag]);
        // Drop forward from an entity.
        let owner = player_slot(&mut pool, Product::Baseq3);
        pool.at_mut(owner).unwrap().s.apos.base = vec3(0.0, 0.0, 0.0);
        pool.at_mut(owner).unwrap().s.pos.base = vec3(5.0, 5.0, 5.0);
        let mut check = |_pool: &mut EntityPool, _slot: Slot| Ok(());
        let mut random = || 0.5f32;
        let mut drop = DropItemContext {
            launch: LaunchItemContext {
                product: Product::Baseq3,
                game_type: GameType::Ffa as i32,
                time: 2000,
                items: &items,
                check_dropped_team_item: &mut check,
            },
            random: &mut random,
        };
        let tossed = drop_item(&mut pool, owner, &mut drop, &items.list[4], 0.0).unwrap();
        assert_eq!(pool.at(tossed).unwrap().s.pos.base, vec3(5.0, 5.0, 5.0));
        assert_eq!(pool.at(tossed).unwrap().s.pos.delta.z, 200.0);
        // Bad random is rejected.
        let mut bad_random = || f32::NAN;
        let mut check = |_pool: &mut EntityPool, _slot: Slot| Ok(());
        let mut bad = DropItemContext {
            launch: LaunchItemContext {
                product: Product::Baseq3,
                game_type: GameType::Ffa as i32,
                time: 2000,
                items: &items,
                check_dropped_team_item: &mut check,
            },
            random: &mut bad_random,
        };
        assert!(drop_item(&mut pool, owner, &mut bad, &items.list[4], 0.0).is_err());
    }

    #[test]
    fn run_item_frames_and_bounce() {
        let mut pool = EntityPool::new(Product::Baseq3);
        // Stationary item runs think only.
        let still = pool.spawn().unwrap();
        pool.at_mut(still).unwrap().s.pos.ty = TrajectoryType::Stationary;
        pool.at_mut(still).unwrap().nextthink = 500;
        let mut world = TestWorld::new();
        let mut freed = Vec::new();
        let mut free_team = |_pool: &mut EntityPool, slot: Slot| {
            freed.push(slot);
            Ok(())
        };
        let mut ran = Vec::new();
        let mut think = |_pool: &mut EntityPool, slot: Slot, _name: CallbackName| {
            ran.push(slot);
            Ok(())
        };
        let mut ctx = RunItemContext {
            time: 1000,
            previous_time: 900,
            world: &mut world,
            free_team_entity: &mut free_team,
            think: &mut think,
        };
        bind_launch_save_callbacks(&mut pool);
        pool.at_mut(still).unwrap().think = Some(CallbackName(LAUNCH_ITEM_THINK));
        run_item(&mut pool, still, &mut ctx).unwrap();
        assert_eq!(ran, vec![still]);
        // Lost support converts to gravity.
        let falling = pool.spawn().unwrap();
        pool.at_mut(falling).unwrap().s.ground_entity_num = -1;
        pool.at_mut(falling).unwrap().s.pos.ty = TrajectoryType::Linear;
        pool.at_mut(falling).unwrap().s.pos.delta = vec3(0.0, 0.0, 0.0);
        let mut world = TestWorld::new();
        let mut free_team = |_pool: &mut EntityPool, _slot: Slot| Ok(());
        let mut think = |_pool: &mut EntityPool, _slot: Slot, _name: CallbackName| Ok(());
        let mut ctx = RunItemContext {
            time: 1000,
            previous_time: 900,
            world: &mut world,
            free_team_entity: &mut free_team,
            think: &mut think,
        };
        run_item(&mut pool, falling, &mut ctx).unwrap();
        assert_eq!(pool.at(falling).unwrap().s.pos.ty, TrajectoryType::Gravity);
        // Nodrop frees the item after a partial trace.
        let mut world = TestWorld::new();
        world.trace_result.fraction = 0.5;
        world.trace_result.end = vec3(1.0, 1.0, 1.0);
        world.contents = i32::MIN;
        let dropping = pool.spawn().unwrap();
        pool.at_mut(dropping).unwrap().s.pos.ty = TrajectoryType::Linear;
        let mut free_team = |_pool: &mut EntityPool, _slot: Slot| Ok(());
        let mut think = |_pool: &mut EntityPool, _slot: Slot, _name: CallbackName| Ok(());
        let mut ctx = RunItemContext {
            time: 1000,
            previous_time: 900,
            world: &mut world,
            free_team_entity: &mut free_team,
            think: &mut think,
        };
        run_item(&mut pool, dropping, &mut ctx).unwrap();
        assert!(pool.get(dropping).is_none());
    }

    #[test]
    fn bounce_settles_or_reflects() {
        let mut pool = EntityPool::new(Product::Baseq3);
        let slot = pool.spawn().unwrap();
        pool.at_mut(slot).unwrap().s.pos = Trajectory {
            ty: TrajectoryType::Gravity,
            time: 900,
            duration: 0,
            base: vec3(0.0, 0.0, 100.0),
            delta: vec3(0.0, 0.0, -10.0),
        };
        pool.at_mut(slot).unwrap().physics_bounce = 0.5;
        let trace = ActorTraceResult {
            fraction: 0.5,
            end: vec3(0.0, 0.0, 5.0),
            solidity: TraceSolidity::Clear,
            contact: TraceContact::Plane {
                normal: vec3(0.0, 0.0, 1.0),
            },
            contents: 1,
            surface_flags: 0,
            hit: TraceHit::World,
        };
        bounce_item(
            &mut pool,
            slot,
            &trace,
            ItemFrameTime {
                time: 1000,
                previous_time: 900,
            },
        )
        .unwrap();
        // Reflected upward velocity is small, so the item settles.
        assert_eq!(pool.at(slot).unwrap().s.pos.ty, TrajectoryType::Stationary);
        assert_eq!(pool.at(slot).unwrap().r.current_origin, vec3(0.0, 0.0, 6.0));
        // A wall bounce keeps flying.
        let wall = pool.spawn().unwrap();
        pool.at_mut(wall).unwrap().s.pos = Trajectory {
            ty: TrajectoryType::Gravity,
            time: 900,
            duration: 0,
            base: vec3(0.0, 0.0, 100.0),
            delta: vec3(100.0, 0.0, 0.0),
        };
        pool.at_mut(wall).unwrap().physics_bounce = 0.5;
        pool.at_mut(wall).unwrap().r.current_origin = vec3(10.0, 0.0, 100.0);
        let trace = ActorTraceResult {
            contact: TraceContact::Plane {
                normal: vec3(-1.0, 0.0, 0.0),
            },
            hit: TraceHit::World,
            ..trace
        };
        bounce_item(
            &mut pool,
            wall,
            &trace,
            ItemFrameTime {
                time: 1000,
                previous_time: 900,
            },
        )
        .unwrap();
        assert_eq!(pool.at(wall).unwrap().r.current_origin, vec3(9.0, 0.0, 100.0));
    }

    #[test]
    fn game_memory_bump_and_save() {
        let prints = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let sink = std::rc::Rc::clone(&prints);
        let mut memory = GameMemory::new(Box::new(|| 1), Box::new(move |text| sink.borrow_mut().push(text)));
        let first = memory.allocate(64).unwrap();
        assert_eq!(first.offset(), 0);
        assert_eq!(memory.allocated_bytes(), 64);
        first.write_string(&mut memory, "hello").unwrap();
        assert_eq!(first.read_string(&memory).unwrap(), "hello");
        assert!(first.write_string(&mut memory, &"x".repeat(64)).is_err());
        let captured = memory.capture_allocation(&first).unwrap();
        let restored = memory.restore_allocation(&captured).unwrap();
        assert_eq!(restored.read_string(&memory).unwrap(), "hello");
        let save = memory.capture_save_state();
        memory.allocate(32).unwrap();
        assert_eq!(memory.allocated_bytes(), 96);
        memory.restore_save_state(&save).unwrap();
        assert_eq!(memory.allocated_bytes(), 64);
        memory.initialize();
        assert_eq!(memory.allocated_bytes(), 0);
        memory.status();
        assert!(!prints.borrow().is_empty());
        assert!(memory.allocate(usize::MAX).is_err());
        let bad = GameMemorySave {
            pool: vec![0; 10],
            alloc_point: 0,
        };
        assert!(memory.restore_save_state(&bad).is_err());
    }

    #[test]
    fn game_level_clear_resets() {
        let mut level = GameLevel::new();
        level.frame_num = 42;
        level.base.time = 99;
        level.team_scores.set(0, 7).unwrap();
        level.base.vote.yes = 3;
        level.clear();
        assert_eq!(level, GameLevel::new());
    }

    #[test]
    fn portals_spawn_and_locate() {
        let mut pool = EntityPool::new(Product::Baseq3);
        let mut world = TestWorld::new();
        let surface = pool.spawn().unwrap();
        spawn_portal_surface(&mut pool, &mut world, 1000, surface).unwrap();
        assert_eq!(pool.at(surface).unwrap().s.e_type, EntityType::Portal as i32);
        assert_eq!(pool.at(surface).unwrap().s.origin2, vec3(0.0, 0.0, 0.0));
        let camera = pool.spawn().unwrap();
        spawn_portal_camera(&mut pool, &mut world, camera, 180.0).unwrap();
        assert_eq!(pool.at(camera).unwrap().s.client_num, 128);
        // Targeted surface schedules the locate think.
        let target = pool.spawn().unwrap();
        pool.at_mut(target).unwrap().targetname = Some("cam".to_string());
        pool.at_mut(target).unwrap().s.origin = vec3(10.0, 0.0, 0.0);
        let surface2 = pool.spawn().unwrap();
        pool.at_mut(surface2).unwrap().target = Some("cam".to_string());
        spawn_portal_surface(&mut pool, &mut world, 1000, surface2).unwrap();
        assert_eq!(pool.at(surface2).unwrap().nextthink, 1100);
        let mut warnings = Vec::new();
        let mut rand = || 0;
        let mut ctx = PortalContext {
            world: &mut world,
            time: 1000,
            random_int: &mut rand,
            warn: &mut |text: &str| warnings.push(text.to_string()),
        };
        locate_camera(&mut pool, &mut ctx, surface2).unwrap();
        assert_eq!(pool.at(surface2).unwrap().s.origin2, vec3(10.0, 0.0, 0.0));
        assert_eq!(warnings, vec!["G_PickTarget called with NULL targetname\n".to_string()]);
        // Missing target warns with the donor typo and frees.
        let lost = pool.spawn().unwrap();
        pool.at_mut(lost).unwrap().target = Some("nope".to_string());
        let mut world = TestWorld::new();
        let mut warnings = Vec::new();
        let mut rand = || 0;
        let mut ctx = PortalContext {
            world: &mut world,
            time: 1000,
            random_int: &mut rand,
            warn: &mut |text: &str| warnings.push(text.to_string()),
        };
        locate_camera(&mut pool, &mut ctx, lost).unwrap();
        assert!(pool.get(lost).is_none());
        assert!(warnings.iter().any(|text| text.contains("misc_partal_surface")));
    }

    #[test]
    fn teleport_and_kill_box() {
        let mut pool = EntityPool::new(Product::Baseq3);
        let player = player_slot(&mut pool, Product::Baseq3);
        pool.at_mut(player).unwrap().client.as_mut().unwrap().ps.health = 100;
        pool.at_mut(player).unwrap().client.as_mut().unwrap().ps.origin = vec3(0.0, 0.0, 0.0);
        let mut combat = TestCombat::new();
        let mut world = TestWorld::new();
        let mut ctx = TeleportContext {
            combat: &mut combat,
            world: &mut world,
        };
        teleport_player(
            &mut pool,
            &mut ctx,
            player,
            vec3(100.0, 200.0, 50.0),
            vec3(0.0, 90.0, 0.0),
        )
        .unwrap();
        let client = pool.at(player).unwrap().client.as_ref().unwrap();
        assert_eq!(client.ps.origin, vec3(100.0, 200.0, 51.0));
        assert_eq!(client.ps.pm_time, 160);
        assert_eq!(pool.at(player).unwrap().s.e_type, EntityType::Player as i32);
        let temps = (0..pool.num_entities())
            .filter(|slot| {
                pool.at(*slot)
                    .map(|entity| entity.classname.as_deref() == Some("tempEntity"))
                    .unwrap_or(false)
            })
            .count();
        assert_eq!(temps, 2);
        assert!(pool.at(player).unwrap().linked);
        // Kill box hits linked players in the area only.
        let mut pool = EntityPool::new(Product::Baseq3);
        let player = player_slot(&mut pool, Product::Baseq3);
        pool.at_mut(player).unwrap().linked = true;
        let victim = player_slot(&mut pool, Product::Baseq3);
        pool.at_mut(victim).unwrap().linked = true;
        let prop = pool.spawn().unwrap();
        pool.at_mut(prop).unwrap().linked = true;
        let mut combat = TestCombat::new();
        let mut world = TestWorld::new();
        world.actors = vec![
            ActorId::from_slot(player),
            ActorId::from_slot(victim),
            ActorId::from_slot(prop),
        ];
        kill_box(&mut pool, &mut combat, &mut world, player).unwrap();
        assert_eq!(combat.damage_calls.len(), 2);
        assert_eq!(combat.damage_calls[0].amount, 100000);
        assert_eq!(combat.damage_calls[0].flags, DamageFlags::NO_PROTECTION);
        assert_eq!(combat.damage_calls[0].method, 18);
    }

    #[test]
    fn misc_spawns_cover_table() {
        let handlers = misc_spawn_handlers();
        assert_eq!(handlers.len(), 11);
        let mut pool = EntityPool::new(Product::Baseq3);
        let mut runtime = MissileRuntime::new();
        runtime.bind_save_callbacks(&mut pool);
        let mut missile_host = TestMissileHost::base();
        let mut driver = TestDriver::new();
        let mut registry = TestRegistry {
            product: Product::Baseq3,
            registered: Vec::new(),
        };
        let items = test_items();
        let mut random = TestRandom {
            int_value: 0,
            random_value: 0.5,
            crandom_value: 0.0,
        };
        let mut world = TestWorld::new();
        let mut warnings = Vec::new();
        let mut host = MiscSpawnHost {
            missiles: &mut runtime,
            missile_host: &mut missile_host,
            missile_driver: &mut driver,
            item_registry: &mut registry,
            items: &items,
            random: &mut random,
            world: &mut world,
            time: 1000,
            warn: &mut |text: &str| warnings.push(text.to_string()),
        };
        let vars = SpawnVariables::new(Vec::new());
        let shooter_slot = pool.spawn().unwrap();
        run_misc_spawn(
            &mut pool,
            &mut host,
            &MiscSpawn::Shooter(Weapon::RocketLauncher),
            shooter_slot,
            &vars,
        )
        .unwrap();
        assert_eq!(pool.at(shooter_slot).unwrap().s.weapon, Weapon::RocketLauncher);
        assert!((pool.at(shooter_slot).unwrap().random - (std::f32::consts::PI / 180.0).sin()).abs() < 1e-6);
        shooter_use(&mut pool, &mut host, shooter_slot).unwrap();
        assert!(pool.events.iter().any(|event| event.event == EntityEvent::FireWeapon));
        let null = pool.spawn().unwrap();
        run_misc_spawn(&mut pool, &mut host, &MiscSpawn::InfoNull, null, &vars).unwrap();
        assert!(pool.get(null).is_none());
        let camp = pool.spawn().unwrap();
        pool.at_mut(camp).unwrap().s.origin = vec3(7.0, 8.0, 9.0);
        run_misc_spawn(&mut pool, &mut host, &MiscSpawn::InfoCamp, camp, &vars).unwrap();
        assert_eq!(pool.at(camp).unwrap().s.pos.base, vec3(7.0, 8.0, 9.0));
        assert_eq!(driver.launches.len(), 1);
        assert_eq!(registry.registered.len(), 1);
        assert!(warnings.is_empty());
    }

    #[test]
    fn missile_parameters_and_snapping() {
        let rocket = q3_missile_parameters(Weapon::RocketLauncher).unwrap();
        assert_eq!((rocket.speed, rocket.duration, rocket.direct), (900.0, 15000, 100));
        assert!(q3_missile_parameters(Weapon::Shotgun).is_err());
        assert_eq!(snap_vector(vec3(1.9, -2.1, 0.5)), vec3(1.0, -2.0, 0.0));
        assert_eq!(
            snap_vector_towards(vec3(1.9, 2.1, 3.0), vec3(0.0, 9.0, 3.0)),
            vec3(1.0, 3.0, 3.0)
        );
        let mut random = TestRandom {
            int_value: 0,
            random_value: 0.5,
            crandom_value: 0.0,
        };
        let nail = q3_nail_velocity(
            vec3(0.0, 0.0, 0.0),
            vec3(1.0, 0.0, 0.0),
            vec3(0.0, 1.0, 0.0),
            vec3(0.0, 0.0, 1.0),
            &mut random,
        );
        assert!(nail.x > 1000.0 && nail.y == 0.0 && nail.z == 0.0, "{nail:?}");
    }

    #[test]
    fn missile_fire_paths() {
        let mut pool = EntityPool::new(Product::Baseq3);
        let mut runtime = MissileRuntime::new();
        runtime.bind_save_callbacks(&mut pool);
        let mut host = TestMissileHost::base();
        let mut driver = TestDriver::new();
        let owner = player_slot(&mut pool, Product::Baseq3);
        let mut dir = MissileDirection { x: 3.0, y: 0.0, z: 0.0 };
        let rocket = runtime
            .fire_rocket(&mut pool, &mut host, &mut driver, owner, vec3(0.0, 0.0, 0.0), &mut dir)
            .unwrap();
        assert_eq!((dir.x, dir.y, dir.z), (1.0, 0.0, 0.0));
        assert_eq!(pool.at(rocket).unwrap().s.e_type, EntityType::Missile as i32);
        assert_eq!(pool.at(rocket).unwrap().think, Some(CallbackName(MISSILE_LAUNCH_THINK)));
        assert_eq!(pool.at(rocket).unwrap().damage, 100);
        let mut dir = MissileDirection { x: 0.0, y: 1.0, z: 0.0 };
        let grenade = runtime
            .fire_grenade(&mut pool, &mut host, &mut driver, owner, vec3(0.0, 0.0, 0.0), &mut dir)
            .unwrap();
        assert_eq!(pool.at(grenade).unwrap().s.e_flags, 0x20);
        let mut dir = MissileDirection { x: 0.0, y: 0.0, z: 1.0 };
        let hook = runtime
            .fire_grapple(&mut pool, &mut host, &mut driver, owner, vec3(0.0, 0.0, 0.0), &mut dir)
            .unwrap();
        assert_eq!(pool.at(hook).unwrap().classname.as_deref(), Some("hook"));
        assert_eq!(pool.at(owner).unwrap().client.as_ref().unwrap().hook, Some(hook));
        assert_eq!(
            runtime.owner_of(&pool, ActorId::from_slot(hook)),
            Some(ActorId::from_slot(owner))
        );
        // Proximity requires missionpack.
        let mut dir = MissileDirection { x: 1.0, y: 0.0, z: 0.0 };
        assert!(runtime
            .fire_prox(&mut pool, &mut host, &mut driver, owner, vec3(0.0, 0.0, 0.0), &mut dir)
            .is_err());
        // Bounce writes driver state back.
        let trace = ServerTraceResult {
            fraction: 0.5,
            end: vec3(1.0, 1.0, 1.0),
            entity_num: ENTITYNUM_WORLD as i32,
            solidity: TraceSolidity::Clear,
            contact: TraceContact::Plane {
                normal: vec3(0.0, 0.0, 1.0),
            },
            contents: 1,
            surface_flags: 0,
        };
        runtime
            .bounce(&mut pool, &mut host, &mut driver, rocket, &trace)
            .unwrap();
        assert_eq!(pool.at(rocket).unwrap().s.pos.delta, vec3(1.0, 2.0, 3.0));
        // Owned-actor stepping dispatches the driver.
        assert!(runtime
            .run_owned(
                &mut pool,
                &mut host,
                &mut driver,
                OwnedActor {
                    id: ActorId::from_slot(rocket)
                }
            )
            .unwrap());
        assert!(!runtime
            .run_owned(&mut pool, &mut host, &mut driver, OwnedActor { id: ActorId(999) })
            .unwrap());
        assert_eq!(driver.steps, 1);
    }

    #[test]
    fn proximity_mine_end_to_end() {
        let mut pool = EntityPool::new(Product::Missionpack);
        let mut runtime = MissileRuntime::new();
        runtime.bind_save_callbacks(&mut pool);
        let mut host = TestMissileHost::missionpack();
        let mut driver = TestDriver::new();
        let owner = player_slot(&mut pool, Product::Missionpack);
        let mut dir = MissileDirection {
            x: 0.0,
            y: 0.0,
            z: -1.0,
        };
        let mine = runtime
            .fire_prox(&mut pool, &mut host, &mut driver, owner, vec3(0.0, 0.0, 50.0), &mut dir)
            .unwrap();
        // Stick to the world through the projectile context.
        let trace = ActorTraceResult {
            fraction: 0.5,
            end: vec3(0.0, 0.0, 0.0),
            solidity: TraceSolidity::Clear,
            contact: TraceContact::Plane {
                normal: vec3(0.0, 0.0, 1.0),
            },
            contents: 1,
            surface_flags: 0,
            hit: TraceHit::World,
        };
        let entity = pool.at(mine).unwrap().clone();
        let mut ctx = ProjectileContext {
            runtime: &mut runtime,
            pool: &mut pool,
            host: &mut host,
            slot: mine,
            state: ProjectileState {
                trajectory: entity.s.pos,
                flags: entity.s.e_flags,
            },
            spec: ProjectileSpec {
                weapon: entity.s.weapon,
                direct: entity.damage,
                splash: entity.splash_damage,
                radius: entity.splash_radius,
                method: entity.method_of_death,
                splash_method: entity.splash_method_of_death,
                damage_point: entity.s.origin,
            },
        };
        assert!(ctx.special_impact(&trace, ActorId::from_slot(ENTITYNUM_WORLD)).unwrap());
        assert!(ctx.has_special().unwrap());
        assert!(ctx.has_reflection());
        assert_eq!(pool.at(mine).unwrap().think, Some(CallbackName(MISSILE_SPECIAL_THINK)));
        // Arm the mine; a trigger entity appears.
        assert!(runtime
            .dispatch_think(
                &mut pool,
                &mut host,
                &mut driver,
                mine,
                CallbackName(MISSILE_SPECIAL_THINK)
            )
            .unwrap());
        let trigger = pool.at(mine).unwrap().activator.unwrap();
        assert!(runtime.is_proximity_trigger(&pool, trigger).unwrap());
        assert_eq!(pool.at(trigger).unwrap().classname.as_deref(), Some("proxmine_trigger"));
        // An enemy in radius trips the trigger.
        let victim = player_slot(&mut pool, Product::Missionpack);
        pool.at_mut(victim).unwrap().s.pos.base = vec3(1.0, 0.0, 0.0);
        pool.at_mut(trigger).unwrap().s.pos.base = vec3(0.0, 0.0, 0.0);
        assert!(runtime
            .dispatch_touch(&mut pool, &mut host, trigger, DamageParticipant::Entity(victim))
            .unwrap());
        assert!(pool.get(trigger).is_none());
        assert_eq!(pool.at(mine).unwrap().nextthink, host.combat.time + 500);
        // The armed think explodes through the driver.
        assert!(runtime
            .dispatch_think(
                &mut pool,
                &mut host,
                &mut driver,
                mine,
                CallbackName(MISSILE_PROXIMITY_DIE_THINK)
            )
            .unwrap());
        assert_eq!(driver.explodes, 1);
        assert!(pool.at(mine).unwrap().free_after_event);
    }

    #[test]
    fn hook_attach_think_and_release() {
        let mut pool = EntityPool::new(Product::Baseq3);
        let mut runtime = MissileRuntime::new();
        runtime.bind_save_callbacks(&mut pool);
        let mut host = TestMissileHost::base();
        let mut driver = TestDriver::new();
        let owner = player_slot(&mut pool, Product::Baseq3);
        let victim = player_slot(&mut pool, Product::Baseq3);
        let mut dir = MissileDirection { x: 1.0, y: 0.0, z: 0.0 };
        let hook = runtime
            .fire_grapple(&mut pool, &mut host, &mut driver, owner, vec3(0.0, 0.0, 0.0), &mut dir)
            .unwrap();
        let trace = ActorTraceResult {
            fraction: 0.5,
            end: vec3(5.0, 0.0, 0.0),
            solidity: TraceSolidity::Clear,
            contact: TraceContact::Plane {
                normal: vec3(-1.0, 0.0, 0.0),
            },
            contents: 1,
            surface_flags: 0,
            hit: TraceHit::Actor(ActorId::from_slot(victim)),
        };
        let entity = pool.at(hook).unwrap().clone();
        let mut ctx = ProjectileContext {
            runtime: &mut runtime,
            pool: &mut pool,
            host: &mut host,
            slot: hook,
            state: ProjectileState {
                trajectory: entity.s.pos,
                flags: entity.s.e_flags,
            },
            spec: ProjectileSpec {
                weapon: entity.s.weapon,
                direct: entity.damage,
                splash: entity.splash_damage,
                radius: entity.splash_radius,
                method: entity.method_of_death,
                splash_method: entity.splash_method_of_death,
                damage_point: entity.s.origin,
            },
        };
        assert!(ctx.special_impact(&trace, ActorId::from_slot(victim)).unwrap());
        assert_eq!(pool.at(hook).unwrap().s.e_type, EntityType::Grapple as i32);
        runtime.hook_think(&mut pool, hook).unwrap();
        assert_eq!(
            pool.at(owner).unwrap().client.as_ref().unwrap().ps.grapple_point,
            pool.at(hook).unwrap().r.current_origin
        );
        // Releasing the hook actor clears the owner hook.
        runtime.released(&mut pool, ActorId::from_slot(hook)).unwrap();
        assert!(pool.at(owner).unwrap().client.as_ref().unwrap().hook.is_none());
        // Save round trip preserves owners.
        let mut dir = MissileDirection { x: 1.0, y: 0.0, z: 0.0 };
        let _rocket = runtime
            .fire_rocket(&mut pool, &mut host, &mut driver, owner, vec3(0.0, 0.0, 0.0), &mut dir)
            .unwrap();
        let save = runtime.capture_save_state();
        let mut fresh = MissileRuntime::new();
        let resolve = |id: u32| -> Q3GameItemsResult<ActorId> { Ok(ActorId(id)) };
        fresh.restore_save_state(&pool, &save, &resolve).unwrap();
        assert_eq!(fresh.capture_save_state(), save);
        let mut duplicated = save.clone();
        duplicated.push(save[0]);
        assert!(fresh.restore_save_state(&pool, &duplicated, &resolve).is_err());
    }

    #[test]
    fn projectile_context_surface() {
        let mut pool = EntityPool::new(Product::Baseq3);
        let mut runtime = MissileRuntime::new();
        runtime.bind_save_callbacks(&mut pool);
        let mut host = TestMissileHost::base();
        let mut driver = TestDriver::new();
        let owner = player_slot(&mut pool, Product::Baseq3);
        let target = player_slot(&mut pool, Product::Baseq3);
        pool.at_mut(target).unwrap().takedamage = true;
        pool.at_mut(target).unwrap().health = 80;
        let mut dir = MissileDirection { x: 1.0, y: 0.0, z: 0.0 };
        let bolt = runtime
            .fire_rocket(&mut pool, &mut host, &mut driver, owner, vec3(0.0, 0.0, 0.0), &mut dir)
            .unwrap();
        let entity = pool.at(bolt).unwrap().clone();
        let mut ctx = ProjectileContext {
            runtime: &mut runtime,
            pool: &mut pool,
            host: &mut host,
            slot: bolt,
            state: ProjectileState {
                trajectory: entity.s.pos,
                flags: entity.s.e_flags,
            },
            spec: ProjectileSpec {
                weapon: entity.s.weapon,
                direct: entity.damage,
                splash: entity.splash_damage,
                radius: entity.splash_radius,
                method: entity.method_of_death,
                splash_method: entity.splash_method_of_death,
                damage_point: entity.s.origin,
            },
        };
        assert!(ctx.live().unwrap());
        assert_eq!(ctx.phase().unwrap(), ProjectilePhase::Flight);
        assert_eq!(ctx.world_actor(), ActorId::from_slot(ENTITYNUM_WORLD));
        let record = ctx.target(ActorId::from_slot(target)).unwrap().unwrap();
        assert!(record.damageable && record.player && record.accuracy_eligible);
        assert!(ctx.target(ActorId(999)).unwrap().is_none());
        ctx.emit(&ImpactEmit::Bounce {
            normal: vec3(0.0, 0.0, 1.0),
        })
        .unwrap();
        ctx.accuracy().unwrap();
        assert!(ctx.radius(vec3(0.0, 0.0, 0.0), None).unwrap());
        ctx.after_move().unwrap();
        ctx.no_impact().unwrap();
        assert_eq!(pool.at(owner).unwrap().client.as_ref().unwrap().accuracy_hits, 1);
        assert!(pool
            .events
            .iter()
            .any(|event| event.event == EntityEvent::GrenadeBounce));
    }

    #[test]
    fn mover_spawns_cover_table() {
        let handlers = mover_spawn_handlers();
        assert_eq!(handlers.len(), 9);
        let mut pool = EntityPool::new(Product::Baseq3);
        let mut host = TestMoverSpawnHost::new();
        let vars = SpawnVariables::new(Vec::new());
        // Door geometry and scheduling.
        let door = pool.spawn().unwrap();
        pool.at_mut(door).unwrap().s.angles = vec3(0.0, 90.0, 0.0);
        run_mover_spawn(&mut pool, &mut host, &MoverSpawn::Door, door, &vars).unwrap();
        assert!((pool.at(door).unwrap().pos2.y - 24.0).abs() < 0.01);
        assert_eq!(pool.at(door).unwrap().nextthink, 600);
        assert_eq!(
            pool.at(door).unwrap().think,
            Some(CallbackName(MOVER_DOOR_SPAWN_TRIGGER))
        );
        assert!(dispatch_mover_think(&mut pool, &mut host, door, CallbackName(MOVER_DOOR_SPAWN_TRIGGER)).unwrap());
        assert_eq!(host.movers.match_log.len(), 1);
        // Plat drops below its origin.
        let plat = pool.spawn().unwrap();
        run_mover_spawn(&mut pool, &mut host, &MoverSpawn::Plat, plat, &vars).unwrap();
        assert!((pool.at(plat).unwrap().pos1.z + 48.0).abs() < 0.01);
        // Button arms touch or health.
        let button = pool.spawn().unwrap();
        run_mover_spawn(&mut pool, &mut host, &MoverSpawn::Button, button, &vars).unwrap();
        assert_eq!(pool.at(button).unwrap().touch, Some(CallbackName(MOVER_BUTTON_TOUCH)));
        // Train without a target warns and frees.
        let train = pool.spawn().unwrap();
        run_mover_spawn(&mut pool, &mut host, &MoverSpawn::Train, train, &vars).unwrap();
        assert!(pool.get(train).is_none());
        assert!(!host.warnings.is_empty());
        // Rotating, bobbing, pendulum, static shapes.
        let rotating = pool.spawn().unwrap();
        run_mover_spawn(&mut pool, &mut host, &MoverSpawn::Rotating, rotating, &vars).unwrap();
        assert_eq!(pool.at(rotating).unwrap().s.apos.delta.y, 100.0);
        let bobbing = pool.spawn().unwrap();
        run_mover_spawn(&mut pool, &mut host, &MoverSpawn::Bobbing, bobbing, &vars).unwrap();
        assert_eq!(pool.at(bobbing).unwrap().s.pos.ty, TrajectoryType::Sine);
        assert_eq!(pool.at(bobbing).unwrap().s.pos.duration, 4000);
        let pendulum = pool.spawn().unwrap();
        run_mover_spawn(&mut pool, &mut host, &MoverSpawn::Pendulum, pendulum, &vars).unwrap();
        assert!(pool.at(pendulum).unwrap().s.pos.duration > 0);
        let statik = pool.spawn().unwrap();
        pool.at_mut(statik).unwrap().s.origin = vec3(3.0, 4.0, 5.0);
        run_mover_spawn(&mut pool, &mut host, &MoverSpawn::Static, statik, &vars).unwrap();
        assert_eq!(pool.at(statik).unwrap().r.current_origin, vec3(3.0, 4.0, 5.0));
        // Path corner without a targetname warns and frees.
        let corner = pool.spawn().unwrap();
        run_mover_spawn(&mut pool, &mut host, &MoverSpawn::PathCorner, corner, &vars).unwrap();
        assert!(pool.get(corner).is_none());
    }

    #[test]
    fn mover_train_path_and_triggers() {
        let mut pool = EntityPool::new(Product::Baseq3);
        let mut host = TestMoverSpawnHost::new();
        let vars = SpawnVariables::new(Vec::new());
        let c1 = pool.spawn().unwrap();
        pool.at_mut(c1).unwrap().classname = Some("path_corner".to_string());
        pool.at_mut(c1).unwrap().targetname = Some("t1".to_string());
        pool.at_mut(c1).unwrap().target = Some("t2".to_string());
        pool.at_mut(c1).unwrap().s.origin = vec3(0.0, 0.0, 0.0);
        let c2 = pool.spawn().unwrap();
        pool.at_mut(c2).unwrap().classname = Some("path_corner".to_string());
        pool.at_mut(c2).unwrap().targetname = Some("t2".to_string());
        pool.at_mut(c2).unwrap().target = Some("t1".to_string());
        pool.at_mut(c2).unwrap().s.origin = vec3(100.0, 0.0, 0.0);
        let train = pool.spawn().unwrap();
        pool.at_mut(train).unwrap().target = Some("t1".to_string());
        pool.at_mut(train).unwrap().speed = 100.0;
        run_mover_spawn(&mut pool, &mut host, &MoverSpawn::Train, train, &vars).unwrap();
        assert!(dispatch_mover_think(&mut pool, &mut host, train, CallbackName(MOVER_TRAIN_THINK)).unwrap());
        assert_eq!(pool.at(train).unwrap().next_train, Some(c2));
        assert_eq!(pool.at(train).unwrap().pos2, vec3(100.0, 0.0, 0.0));
        assert_eq!(host.used_targets, vec![c1]);
        assert_eq!(host.movers.state_log.last(), Some(&(train, MoverState::OneToTwo)));
        // A cycle that never returns to the first corner is an error.
        let loop_a = pool.spawn().unwrap();
        pool.at_mut(loop_a).unwrap().classname = Some("path_corner".to_string());
        pool.at_mut(loop_a).unwrap().targetname = Some("loop_a".to_string());
        pool.at_mut(loop_a).unwrap().target = Some("loop_b".to_string());
        let loop_b = pool.spawn().unwrap();
        pool.at_mut(loop_b).unwrap().classname = Some("path_corner".to_string());
        pool.at_mut(loop_b).unwrap().targetname = Some("loop_b".to_string());
        pool.at_mut(loop_b).unwrap().target = Some("loop_b".to_string());
        let loop_train = pool.spawn().unwrap();
        pool.at_mut(loop_train).unwrap().target = Some("loop_a".to_string());
        pool.at_mut(loop_train).unwrap().speed = 100.0;
        run_mover_spawn(&mut pool, &mut host, &MoverSpawn::Train, loop_train, &vars).unwrap();
        assert!(dispatch_mover_think(&mut pool, &mut host, loop_train, CallbackName(MOVER_TRAIN_THINK)).is_err());
        // Door trigger touch teleports spectators and uses for players.
        let door = pool.spawn().unwrap();
        run_mover_spawn(&mut pool, &mut host, &MoverSpawn::Door, door, &vars).unwrap();
        spawn_door_trigger(&mut pool, &mut host, door).unwrap();
        let trigger = (0..pool.num_entities())
            .find(|slot| {
                pool.at(*slot)
                    .map(|entity| entity.classname.as_deref() == Some("door_trigger"))
                    .unwrap_or(false)
            })
            .unwrap();
        assert!(is_door_trigger(&pool, trigger).unwrap());
        let spectator = player_slot(&mut pool, Product::Baseq3);
        pool.at_mut(spectator)
            .unwrap()
            .client
            .as_mut()
            .unwrap()
            .sess
            .session_team = Team::Spectator;
        pool.at_mut(spectator).unwrap().client.as_mut().unwrap().ps.health = 100;
        pool.at_mut(spectator).unwrap().s.origin = vec3(0.0, 0.0, 0.0);
        door_touch(&mut pool, &mut host, trigger, DamageParticipant::Entity(spectator)).unwrap();
        assert_ne!(
            pool.at(spectator).unwrap().client.as_ref().unwrap().ps.origin,
            vec3(0.0, 0.0, 0.0)
        );
        let player = player_slot(&mut pool, Product::Baseq3);
        assert!(dispatch_mover_touch(&mut pool, &mut host, trigger, DamageParticipant::Entity(player)).unwrap());
        assert_eq!(host.movers.use_log.last(), Some(&door));
        // Plat and button touches.
        let plat = pool.spawn().unwrap();
        run_mover_spawn(&mut pool, &mut host, &MoverSpawn::Plat, plat, &vars).unwrap();
        pool.at_mut(plat).unwrap().mover_state = MoverState::Pos2;
        pool.at_mut(player).unwrap().client.as_mut().unwrap().ps.health = 50;
        assert!(dispatch_mover_touch(&mut pool, &mut host, plat, DamageParticipant::Entity(player)).unwrap());
        assert_eq!(pool.at(plat).unwrap().nextthink, 1500);
        let button = pool.spawn().unwrap();
        run_mover_spawn(&mut pool, &mut host, &MoverSpawn::Button, button, &vars).unwrap();
        assert!(dispatch_mover_touch(&mut pool, &mut host, button, DamageParticipant::Entity(player)).unwrap());
        assert_eq!(host.movers.use_log.last(), Some(&button));
        assert!(dispatch_mover_blocked(&mut pool, &mut host, door, DamageParticipant::Entity(player)).unwrap());
        assert!(dispatch_mover_reached(&mut pool, &mut host, train, CallbackName(MOVER_TRAIN_REACHED)).unwrap());
    }

    #[test]
    fn pool_events_think_and_targets() {
        let mut pool = EntityPool::new(Product::Baseq3);
        let slot = pool.spawn().unwrap();
        pool.add_event(slot, EntityEvent::None, 0).unwrap();
        assert_eq!(pool.prints.len(), 1);
        pool.time = 777;
        pool.add_event(slot, EntityEvent::FireWeapon, 3).unwrap();
        assert_eq!(pool.at(slot).unwrap().s.event & 0xff, EntityEvent::FireWeapon as i32);
        assert_eq!(pool.at(slot).unwrap().event_time, 777);
        let mut world = TestWorld::new();
        let temp = pool
            .temp_entity(&mut world, vec3(1.9, 2.1, 3.5), EntityEvent::Jump)
            .unwrap();
        assert_eq!(
            pool.at(temp).unwrap().s.e_type,
            EntityType::Events as i32 + EntityEvent::Jump as i32
        );
        assert_eq!(pool.at(temp).unwrap().r.current_origin, vec3(1.0, 2.0, 3.0));
        // Think dispatch clears the schedule first.
        bind_launch_save_callbacks(&mut pool);
        pool.at_mut(slot).unwrap().think = Some(CallbackName(LAUNCH_ITEM_THINK));
        pool.at_mut(slot).unwrap().nextthink = 100;
        let mut ran = Vec::new();
        run_think(&mut pool, slot, 1000, &mut |_pool, slot, name| {
            ran.push((slot, name));
            Ok(())
        })
        .unwrap();
        assert_eq!(pool.at(slot).unwrap().nextthink, 0);
        assert_eq!(ran.len(), 1);
        // Target selection warns and caps.
        let target = pool.spawn().unwrap();
        pool.at_mut(target).unwrap().targetname = Some("goal".to_string());
        let mut warnings = Vec::new();
        let mut rand = || 0;
        let mut selection = TargetSelection {
            random_int: &mut rand,
            warn: &mut |text: &str| warnings.push(text.to_string()),
        };
        assert_eq!(pick_target(&pool, &mut selection, Some("goal")).unwrap(), Some(target));
        assert_eq!(pick_target(&pool, &mut selection, None).unwrap(), None);
        assert_eq!(pick_target(&pool, &mut selection, Some("missing")).unwrap(), None);
        assert_eq!(warnings.len(), 2);
        assert_eq!(
            find_entity(&pool, None, EntityStringField::Targetname, Some("GOAL")),
            Some(target)
        );
        assert_eq!(EntityPool::vtos(vec3(1.9, -2.1, 0.0)), "(1 -2 0)");
        assert!(pool.think_cbs.resolve("unknown").is_err());
    }

    #[test]
    fn missile_extra_fires_and_dispatch() {
        let mut pool = EntityPool::new(Product::Missionpack);
        let mut runtime = MissileRuntime::new();
        runtime.bind_save_callbacks(&mut pool);
        let mut host = TestMissileHost::missionpack();
        let mut driver = TestDriver::new();
        let owner = player_slot(&mut pool, Product::Missionpack);
        let mut dir = MissileDirection { x: 1.0, y: 0.0, z: 0.0 };
        let plasma = runtime
            .fire_plasma(&mut pool, &mut host, &mut driver, owner, vec3(0.0, 0.0, 0.0), &mut dir)
            .unwrap();
        assert_eq!(pool.at(plasma).unwrap().damage, 20);
        let mut dir = MissileDirection { x: 1.0, y: 0.0, z: 0.0 };
        let bfg = runtime
            .fire_bfg(&mut pool, &mut host, &mut driver, owner, vec3(0.0, 0.0, 0.0), &mut dir)
            .unwrap();
        assert_eq!(pool.at(bfg).unwrap().damage, 100);
        let nail = runtime
            .fire_nail(
                &mut pool,
                &mut host,
                &mut driver,
                owner,
                vec3(0.0, 0.0, 0.0),
                vec3(1.0, 0.0, 0.0),
                vec3(0.0, 1.0, 0.0),
                vec3(0.0, 0.0, 1.0),
            )
            .unwrap();
        assert_eq!(pool.at(nail).unwrap().s.pos.delta, vec3(1455.0, 0.0, 0.0));
        assert!(pool.at(nail).unwrap().parent.is_none());
        // Unknown callbacks are unhandled; plain entities have no prox hooks.
        assert!(!runtime
            .dispatch_think(&mut pool, &mut host, &mut driver, plasma, CallbackName("nope"))
            .unwrap());
        assert!(!runtime
            .dispatch_touch(&mut pool, &mut host, plasma, DamageParticipant::Entity(owner))
            .unwrap());
        assert!(!runtime.dispatch_die(&mut pool, &mut host, plasma).unwrap());
        // Impact delegates to the driver.
        let trace = ServerTraceResult {
            fraction: 0.5,
            end: vec3(1.0, 1.0, 1.0),
            entity_num: ENTITYNUM_WORLD as i32,
            solidity: TraceSolidity::Clear,
            contact: TraceContact::Plane {
                normal: vec3(0.0, 0.0, 1.0),
            },
            contents: 1,
            surface_flags: 0,
        };
        runtime
            .impact(&mut pool, &mut host, &mut driver, plasma, &trace)
            .unwrap();
        assert_eq!(driver.impacts, 1);
        // Context surface extras.
        let entity = pool.at(plasma).unwrap().clone();
        let mut ctx = ProjectileContext {
            runtime: &mut runtime,
            pool: &mut pool,
            host: &mut host,
            slot: plasma,
            state: ProjectileState {
                trajectory: entity.s.pos,
                flags: entity.s.e_flags,
            },
            spec: ProjectileSpec {
                weapon: entity.s.weapon,
                direct: entity.damage,
                splash: entity.splash_damage,
                radius: entity.splash_radius,
                method: entity.method_of_death,
                splash_method: entity.splash_method_of_death,
                damage_point: entity.s.origin,
            },
        };
        assert_eq!(ctx.time(), 1000);
        assert_eq!(ctx.previous_time(), 900);
        assert_eq!(ctx.origin().unwrap(), vec3(0.0, 0.0, 0.0));
        assert_eq!(ctx.event_time().unwrap(), 0);
        ctx.clear_event().unwrap();
        ctx.link().unwrap();
        ctx.move_body(vec3(1.0, 1.0, 1.0), vec3(2.0, 2.0, 2.0)).unwrap();
        let swept = ctx.trace(vec3(0.0, 0.0, 0.0), vec3(5.0, 5.0, 5.0), None).unwrap();
        assert_eq!(swept.fraction, 1.0);
        assert_eq!(
            ctx.reflection_impact(ActorId::from_slot(owner), vec3(0.0, 0.0, 1.0), vec3(0.0, 0.0, 0.0))
                .unwrap(),
            InvulnerabilityOutcome::Miss
        );
        ctx.set_origin_stop(vec3(9.0, 9.0, 9.0)).unwrap();
        assert_eq!(pool.at(plasma).unwrap().r.current_origin, vec3(9.0, 9.0, 9.0));
        // Shooter finish locks onto the named target.
        let mut pool = EntityPool::new(Product::Baseq3);
        let mut runtime = MissileRuntime::new();
        runtime.bind_save_callbacks(&mut pool);
        let mut missile_host = TestMissileHost::base();
        let mut driver = TestDriver::new();
        let mut registry = TestRegistry {
            product: Product::Baseq3,
            registered: Vec::new(),
        };
        let items = test_items();
        let mut random = TestRandom {
            int_value: 0,
            random_value: 0.5,
            crandom_value: 0.0,
        };
        let mut world = TestWorld::new();
        let mut host = MiscSpawnHost {
            missiles: &mut runtime,
            missile_host: &mut missile_host,
            missile_driver: &mut driver,
            item_registry: &mut registry,
            items: &items,
            random: &mut random,
            world: &mut world,
            time: 1000,
            warn: &mut |_text: &str| {},
        };
        let foe = pool.spawn().unwrap();
        pool.at_mut(foe).unwrap().targetname = Some("foe".to_string());
        let shooter_slot = pool.spawn().unwrap();
        pool.at_mut(shooter_slot).unwrap().target = Some("foe".to_string());
        assert!(dispatch_misc_think(&mut pool, &mut host, shooter_slot, CallbackName(MISC_SHOOTER_THINK)).unwrap());
        assert_eq!(pool.at(shooter_slot).unwrap().enemy, Some(foe));
        assert!(!dispatch_misc_think(&mut pool, &mut host, shooter_slot, CallbackName("nope")).unwrap());
        assert!(!dispatch_misc_use(&mut pool, &mut host, shooter_slot, CallbackName("nope")).unwrap());
    }

    #[test]
    fn proximity_merge_invuln_and_hook_miss() {
        let mut pool = EntityPool::new(Product::Missionpack);
        let mut runtime = MissileRuntime::new();
        runtime.bind_save_callbacks(&mut pool);
        let mut host = TestMissileHost::missionpack();
        let mut driver = TestDriver::new();
        let owner = player_slot(&mut pool, Product::Missionpack);
        let victim = player_slot(&mut pool, Product::Missionpack);
        pool.at_mut(victim).unwrap().s.e_type = EntityType::Player as i32;
        // First mine attaches to the victim.
        let mut dir = MissileDirection {
            x: 0.0,
            y: 0.0,
            z: -1.0,
        };
        let mine = runtime
            .fire_prox(&mut pool, &mut host, &mut driver, owner, vec3(0.0, 0.0, 50.0), &mut dir)
            .unwrap();
        let trace = ActorTraceResult {
            fraction: 0.5,
            end: vec3(0.0, 0.0, 0.0),
            solidity: TraceSolidity::Clear,
            contact: TraceContact::Plane {
                normal: vec3(0.0, 0.0, 1.0),
            },
            contents: 1,
            surface_flags: 0,
            hit: TraceHit::Actor(ActorId::from_slot(victim)),
        };
        let entity = pool.at(mine).unwrap().clone();
        let mut ctx = ProjectileContext {
            runtime: &mut runtime,
            pool: &mut pool,
            host: &mut host,
            slot: mine,
            state: ProjectileState {
                trajectory: entity.s.pos,
                flags: entity.s.e_flags,
            },
            spec: ProjectileSpec {
                weapon: entity.s.weapon,
                direct: entity.damage,
                splash: entity.splash_damage,
                radius: entity.splash_radius,
                method: entity.method_of_death,
                splash_method: entity.splash_method_of_death,
                damage_point: entity.s.origin,
            },
        };
        assert!(ctx.special_impact(&trace, ActorId::from_slot(victim)).unwrap());
        assert_eq!(
            pool.at(mine).unwrap().think,
            Some(CallbackName(MISSILE_PROXIMITY_ON_PLAYER))
        );
        assert_ne!(
            pool.at(victim).unwrap().client.as_ref().unwrap().ps.e_flags & EF_TICKING,
            0
        );
        // The engine copies ps.e_flags to s.e_flags each frame; the merge branch
        // reads the snapshot copy.
        pool.at_mut(victim).unwrap().s.e_flags |= EF_TICKING;
        // A second mine merges into the ticking victim's activator.
        let mut dir = MissileDirection {
            x: 0.0,
            y: 0.0,
            z: -1.0,
        };
        let mine2 = runtime
            .fire_prox(&mut pool, &mut host, &mut driver, owner, vec3(0.0, 0.0, 50.0), &mut dir)
            .unwrap();
        let entity = pool.at(mine2).unwrap().clone();
        let mut ctx = ProjectileContext {
            runtime: &mut runtime,
            pool: &mut pool,
            host: &mut host,
            slot: mine2,
            state: ProjectileState {
                trajectory: entity.s.pos,
                flags: entity.s.e_flags,
            },
            spec: ProjectileSpec {
                weapon: entity.s.weapon,
                direct: entity.damage,
                splash: entity.splash_damage,
                radius: entity.splash_radius,
                method: entity.method_of_death,
                splash_method: entity.splash_method_of_death,
                damage_point: entity.s.origin,
            },
        };
        assert!(ctx.special_impact(&trace, ActorId::from_slot(victim)).unwrap());
        assert_eq!(
            pool.at(mine2).unwrap().think,
            Some(CallbackName(MISSILE_PROXIMITY_PLAYER_THINK))
        );
        assert_eq!(pool.at(mine).unwrap().splash_damage, 200);
        assert_eq!(pool.at(mine).unwrap().splash_radius, 225.0);
        // Invulnerable victims burn the mine off with damage instead of exploding.
        pool.at_mut(victim)
            .unwrap()
            .client
            .as_mut()
            .unwrap()
            .invulnerability_time = 5000;
        assert!(runtime
            .dispatch_think(
                &mut pool,
                &mut host,
                &mut driver,
                mine,
                CallbackName(MISSILE_PROXIMITY_ON_PLAYER)
            )
            .unwrap());
        assert_eq!(host.combat.damage_calls.len(), 1);
        assert_eq!(host.combat.damage_calls[0].method, 27);
        assert_eq!(
            pool.at(victim).unwrap().client.as_ref().unwrap().invulnerability_time,
            0
        );
        // Hook missing the world still grapples; freeing works through dispatch.
        let mut dir = MissileDirection { x: 1.0, y: 0.0, z: 0.0 };
        let hook = runtime
            .fire_grapple(&mut pool, &mut host, &mut driver, owner, vec3(0.0, 0.0, 0.0), &mut dir)
            .unwrap();
        let trace = ActorTraceResult {
            hit: TraceHit::World,
            ..trace
        };
        let entity = pool.at(hook).unwrap().clone();
        let mut ctx = ProjectileContext {
            runtime: &mut runtime,
            pool: &mut pool,
            host: &mut host,
            slot: hook,
            state: ProjectileState {
                trajectory: entity.s.pos,
                flags: entity.s.e_flags,
            },
            spec: ProjectileSpec {
                weapon: entity.s.weapon,
                direct: entity.damage,
                splash: entity.splash_damage,
                radius: entity.splash_radius,
                method: entity.method_of_death,
                splash_method: entity.splash_method_of_death,
                damage_point: entity.s.origin,
            },
        };
        assert!(ctx.special_impact(&trace, ActorId::from_slot(ENTITYNUM_WORLD)).unwrap());
        assert_eq!(pool.at(hook).unwrap().s.e_type, EntityType::Grapple as i32);
        assert_ne!(
            pool.at(owner).unwrap().client.as_ref().unwrap().ps.pm_flags & MoveFlags::GRAPPLE_PULL,
            0
        );
        assert!(runtime
            .dispatch_think(
                &mut pool,
                &mut host,
                &mut driver,
                hook,
                CallbackName(MISSILE_GRAPPLE_THINK)
            )
            .unwrap());
        assert!(pool.get(hook).is_none());
    }

    #[test]
    fn item_motion_errors_and_team_nodrop() {
        let items = test_items();
        let mut pool = EntityPool::new(Product::Baseq3);
        // Wrong product and foreign items are rejected.
        let mut check = |_pool: &mut EntityPool, _slot: Slot| Ok(());
        let mut ctx = LaunchItemContext {
            product: Product::Missionpack,
            game_type: GameType::Ffa as i32,
            time: 1000,
            items: &items,
            check_dropped_team_item: &mut check,
        };
        assert!(launch_item(
            &mut pool,
            &mut ctx,
            &items.list[4],
            vec3(0.0, 0.0, 0.0),
            vec3(0.0, 0.0, 0.0)
        )
        .is_err());
        let foreign = item_def("foreign", 1, ItemKind::Armor);
        let mut check = |_pool: &mut EntityPool, _slot: Slot| Ok(());
        let mut ctx = LaunchItemContext {
            product: Product::Baseq3,
            game_type: GameType::Ffa as i32,
            time: 1000,
            items: &items,
            check_dropped_team_item: &mut check,
        };
        assert!(launch_item(&mut pool, &mut ctx, &foreign, vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 0.0)).is_err());
        // Team items in nodrop route to the team free instead of pool free.
        let flag = pool.spawn().unwrap();
        pool.at_mut(flag).unwrap().s.pos.ty = TrajectoryType::Linear;
        pool.at_mut(flag).unwrap().item = Some(items.list[11].clone());
        let mut world = TestWorld::new();
        world.trace_result.fraction = 0.25;
        world.contents = i32::MIN;
        let mut freed = Vec::new();
        let mut free_team = |pool: &mut EntityPool, slot: Slot| {
            freed.push(slot);
            pool.free(slot)
        };
        let mut think = |_pool: &mut EntityPool, _slot: Slot, _name: CallbackName| Ok(());
        let mut ctx = RunItemContext {
            time: 1000,
            previous_time: 900,
            world: &mut world,
            free_team_entity: &mut free_team,
            think: &mut think,
        };
        run_item(&mut pool, flag, &mut ctx).unwrap();
        assert_eq!(freed, vec![flag]);
    }

    #[test]
    fn mover_dispatch_extras() {
        let mut pool = EntityPool::new(Product::Baseq3);
        let mut host = TestMoverSpawnHost::new();
        let vars = SpawnVariables::new(Vec::new());
        let door = pool.spawn().unwrap();
        run_mover_spawn(&mut pool, &mut host, &MoverSpawn::Door, door, &vars).unwrap();
        assert!(dispatch_mover_think(&mut pool, &mut host, door, CallbackName(MOVER_DOOR_MATCH_TEAM)).unwrap());
        assert_eq!(host.movers.match_log, vec![(door, MoverState::Pos1)]);
        assert!(!dispatch_mover_think(&mut pool, &mut host, door, CallbackName("nope")).unwrap());
        let train = pool.spawn().unwrap();
        pool.at_mut(train).unwrap().s.pos.ty = TrajectoryType::Stationary;
        assert!(dispatch_mover_think(&mut pool, &mut host, train, CallbackName(MOVER_REACHED_TRAIN_THINK)).unwrap());
        assert_eq!(pool.at(train).unwrap().s.pos.ty, TrajectoryType::LinearStop);
        assert_eq!(pool.at(train).unwrap().s.pos.time, 500);
        assert!(!dispatch_mover_reached(&mut pool, &mut host, train, CallbackName("nope")).unwrap());
        let plain = pool.spawn().unwrap();
        assert!(!dispatch_mover_touch(&mut pool, &mut host, plain, DamageParticipant::Entity(door)).unwrap());
        assert!(!dispatch_mover_blocked(&mut pool, &mut host, plain, DamageParticipant::Entity(door)).unwrap());
    }

    #[test]
    fn snapshot_sync_and_view_angles() {
        let mut pool = EntityPool::new(Product::Baseq3);
        let player = player_slot(&mut pool, Product::Baseq3);
        pool.at_mut(player).unwrap().client.as_mut().unwrap().ps.health = 100;
        pool.at_mut(player).unwrap().client.as_mut().unwrap().ps.origin = vec3(1.9, 2.1, 3.5);
        pool.at_mut(player)
            .unwrap()
            .client
            .as_mut()
            .unwrap()
            .ps
            .powerups
            .set(Powerup::Quad as usize, 5)
            .unwrap();
        player_state_to_entity_state(&mut pool, player, true).unwrap();
        assert_eq!(pool.at(player).unwrap().s.e_type, EntityType::Player as i32);
        assert_eq!(pool.at(player).unwrap().s.pos.base, vec3(1.0, 2.0, 3.0));
        assert_eq!(pool.at(player).unwrap().s.powerups, 1 << Powerup::Quad as i32);
        set_client_view_angle(&mut pool, player, vec3(0.0, 90.0, 0.0)).unwrap();
        let client = pool.at(player).unwrap().client.as_ref().unwrap();
        assert_eq!(client.ps.delta_angles[1], 16384);
        assert_eq!(client.ps.viewangles, vec3(0.0, 90.0, 0.0));
        pool.at_mut(player).unwrap().client.as_mut().unwrap().ps.health = -50;
        player_state_to_entity_state(&mut pool, player, false).unwrap();
        assert_eq!(pool.at(player).unwrap().s.e_type, EntityType::Invisible as i32);
    }
}
