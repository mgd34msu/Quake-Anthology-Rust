//! Quake III base-game state, movers, save graph, spawn, targets, triggers, and weapons.
//!
//! Donor provenance (`src/content/q3/base/game/`): `mover.ts`, `numeric.ts`
//! (re-export of `src/core/game-numeric.ts`, ported in full), `personal-portal.ts`,
//! `projectile.ts`, `radius-damage.ts`, `rankings.ts`, `save-callbacks.ts`, `save-level.ts`,
//! `save-module-values.ts`, `save-reader.ts`, `save-state.ts`, `save-values.ts`,
//! `shader-remaps.ts`, `spawn.ts`, `state.ts`, `targets.ts`, `triggers.ts`,
//! `use-participant.ts`, `utilities.ts`, `weapon.ts`.
//!
//! The module is self-contained: it depends only on `std`, `qa-core`, and the
//! pre-existing `crate::value` checkpoint readers. Every import that resolves to a
//! sibling `q3/*` donor outside those 20 files is defined locally as a minimal
//! mirror (see the parent report for the `SIBLING-MIRROR` list) so the merge can
//! unify them with the sibling-owned ports.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use qa_core::identity::ActorId;
use qa_core::identity::SavedActorId;
use qa_core::math::add3;
use qa_core::math::angle_normalize180;
use qa_core::math::angle_vectors;
use qa_core::math::cross3;
use qa_core::math::dot3;
use qa_core::math::length3;
use qa_core::math::normalize3;
use qa_core::math::radius_from_bounds;
use qa_core::math::scale3;
use qa_core::math::sub3;
use qa_core::math::vec3;
use qa_core::math::vector_to_angles;
use qa_core::math::Bounds;
use qa_core::math::Vec3;
use qa_core::numeric::q_rand;
use qa_core::numeric::qvm_float_to_int;

use crate::value::arr;
use crate::value::boolean;
use crate::value::int;
use crate::value::num;
use crate::value::obj;
use crate::value::str;
use crate::value::SaveJson;
use crate::value::SaveReader;
use crate::value::ValueError;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Game failures (`Error`, `RangeError`, `CommonError("drop")`, `TextParseError`).
#[derive(Debug, Clone, PartialEq)]
pub enum Q3GameError {
    /// Fatal drop (`CommonError("drop", ...)`); spawn dispatch must not free on this path.
    Drop(String),
    /// Out-of-range value (`RangeError`).
    Range(String),
    /// Spawn text parse failure (`TextParseError`).
    Parse {
        /// Source name.
        source: String,
        /// 1-based line.
        line: usize,
        /// 1-based column.
        column: usize,
        /// Message.
        message: String,
    },
    /// Ordinary failure (`Error`).
    Failure(String),
}

impl std::fmt::Display for Q3GameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Drop(message) | Self::Range(message) | Self::Failure(message) => write!(f, "{message}"),
            Self::Parse {
                source,
                line,
                column,
                message,
            } => {
                write!(f, "{source}:{line}:{column}: {message}")
            }
        }
    }
}

impl std::error::Error for Q3GameError {}

impl From<ValueError> for Q3GameError {
    fn from(value: ValueError) -> Self {
        Self::Failure(value.0)
    }
}

fn failure(message: impl Into<String>) -> Q3GameError {
    Q3GameError::Failure(message.into())
}

fn range(message: impl Into<String>) -> Q3GameError {
    Q3GameError::Range(message.into())
}

fn drop_error(message: impl Into<String>) -> Q3GameError {
    Q3GameError::Drop(message.into())
}

/// Validate a Latin-1 byte string (`checkByteString` / `byteString` donors).
fn latin1_bytes(value: &str) -> Result<Vec<u8>, Q3GameError> {
    let mut bytes = Vec::with_capacity(value.len());
    for ch in value.chars() {
        let code = ch as u32;
        if code == 0 || code > 255 {
            return Err(range("Spawn strings must contain non-NUL byte characters"));
        }
        bytes.push(code as u8);
    }
    Ok(bytes)
}

/// Rebuild text from Latin-1 bytes.
fn latin1_string(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| char::from_u32(u32::from(*byte)).unwrap_or('\u{FFFD}'))
        .collect()
}

/// Fold ASCII uppercase to lowercase without touching other bytes.
fn ascii_lower(bytes: &[u8]) -> Vec<u8> {
    bytes.iter().map(|byte| byte.to_ascii_lowercase()).collect()
}

// ---------------------------------------------------------------------------
// Shared mirrors: products, constants, and definition enums
// (donor `src/content/q3/base/shared/definitions.ts` and
// `src/movement/q3/constants.ts`)
// ---------------------------------------------------------------------------

/// Game product (`Product`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q3Product {
    /// Base Quake III.
    Baseq3,
    /// Team Arena missionpack.
    Missionpack,
}

/// Default gravity (`DEFAULT_GRAVITY`).
pub const DEFAULT_GRAVITY: f32 = 800.0;
/// Gib health threshold (`GIB_HEALTH`).
pub const GIB_HEALTH: i32 = -40;
/// Armor protection factor (`ARMOR_PROTECTION`).
pub const ARMOR_PROTECTION: f32 = 0.66;
/// Item table bound (`MAX_ITEMS`).
pub const MAX_ITEMS: usize = 256;
/// Missile event validity window (`EVENT_VALID_MSEC`).
pub const EVENT_VALID_MSEC: i32 = 300;
/// Event bit 1 (`EV_EVENT_BIT1`).
pub const EV_EVENT_BIT1: i32 = 0x100;
/// Event bit 2 (`EV_EVENT_BIT2`).
pub const EV_EVENT_BIT2: i32 = 0x200;
/// Event bits mask (`EV_EVENT_BITS`).
pub const EV_EVENT_BITS: i32 = 0x300;
/// World entity number (`ENTITYNUM_WORLD`).
pub const ENTITYNUM_WORLD: usize = 1022;
/// Null entity number (`ENTITYNUM_NONE`).
pub const ENTITYNUM_NONE: usize = 1023;

/// Game type (`GameType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum Q3GameType {
    /// Free for all.
    Ffa = 0,
    /// Tournament.
    Tournament = 1,
    /// Single player.
    SinglePlayer = 2,
    /// Team deathmatch.
    Team = 3,
    /// Capture the flag.
    Ctf = 4,
    /// One-flag CTF.
    OneFctf = 5,
    /// Obelisk.
    Obelisk = 6,
    /// Harvester.
    Harvester = 7,
    /// Game type count.
    MaxGameType = 8,
}

impl Q3GameType {
    /// Convert a stored integer, rejecting unknown values.
    pub fn from_i32(value: i32) -> Result<Self, Q3GameError> {
        match value {
            0 => Ok(Self::Ffa),
            1 => Ok(Self::Tournament),
            2 => Ok(Self::SinglePlayer),
            3 => Ok(Self::Team),
            4 => Ok(Self::Ctf),
            5 => Ok(Self::OneFctf),
            6 => Ok(Self::Obelisk),
            7 => Ok(Self::Harvester),
            8 => Ok(Self::MaxGameType),
            _ => Err(failure(format!("unknown game type {value}"))),
        }
    }
}

/// Team (`Team`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum Q3Team {
    /// No team.
    Free = 0,
    /// Red team.
    Red = 1,
    /// Blue team.
    Blue = 2,
    /// Spectator.
    Spectator = 3,
    /// Team count.
    NumTeams = 4,
}

impl Q3Team {
    /// Convert a stored integer, rejecting unknown values.
    pub fn from_i32(value: i32) -> Result<Self, Q3GameError> {
        match value {
            0 => Ok(Self::Free),
            1 => Ok(Self::Red),
            2 => Ok(Self::Blue),
            3 => Ok(Self::Spectator),
            4 => Ok(Self::NumTeams),
            _ => Err(failure(format!("unknown team {value}"))),
        }
    }
}

/// Item type (`ItemType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum Q3ItemType {
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
    /// Persistant powerup.
    PersistantPowerup = 7,
    /// Team item.
    Team = 8,
}

impl Q3ItemType {
    /// Convert a stored integer, rejecting unknown values.
    pub fn from_i32(value: i32) -> Result<Self, Q3GameError> {
        match value {
            0 => Ok(Self::Bad),
            1 => Ok(Self::Weapon),
            2 => Ok(Self::Ammo),
            3 => Ok(Self::Armor),
            4 => Ok(Self::Health),
            5 => Ok(Self::Powerup),
            6 => Ok(Self::Holdable),
            7 => Ok(Self::PersistantPowerup),
            8 => Ok(Self::Team),
            _ => Err(failure(format!("unknown item type {value}"))),
        }
    }
}

/// Entity type (`EntityType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum Q3EntityType {
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
    /// Event marker base.
    Events = 13,
}

/// Persistant player index (`PersistentIndex`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum Q3PersistentIndex {
    /// Score.
    Score = 0,
    /// Hits.
    Hits = 1,
    /// Rank.
    Rank = 2,
    /// Team.
    Team = 3,
    /// Spawn count.
    SpawnCount = 4,
    /// Player events.
    PlayerEvents = 5,
    /// Attacker.
    Attacker = 6,
    /// Attackee armor.
    AttackeeArmor = 7,
    /// Killed.
    Killed = 8,
    /// Impressive count.
    ImpressiveCount = 9,
    /// Excellent count.
    ExcellentCount = 10,
    /// Defend count.
    DefendCount = 11,
    /// Assist count.
    AssistCount = 12,
    /// Gauntlet frag count.
    GauntletFragCount = 13,
    /// Captures.
    Captures = 14,
}

/// Player stat slots (`StatSchema`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q3StatSchema {
    /// Health slot.
    pub health: usize,
    /// Holdable item slot.
    pub holdable_item: usize,
    /// Weapons slot.
    pub weapons: usize,
    /// Armor slot.
    pub armor: usize,
    /// Dead yaw slot.
    pub dead_yaw: usize,
    /// Clients-ready slot.
    pub clients_ready: usize,
    /// Max health slot.
    pub max_health: usize,
    /// Persistant powerup slot (missionpack only).
    pub persistent_powerup: Option<usize>,
}

/// Stat slots for a product (`statSchema`).
#[must_use]
pub fn stat_schema(product: Q3Product) -> Q3StatSchema {
    match product {
        Q3Product::Baseq3 => Q3StatSchema {
            health: 0,
            holdable_item: 1,
            weapons: 2,
            armor: 3,
            dead_yaw: 4,
            clients_ready: 5,
            max_health: 6,
            persistent_powerup: None,
        },
        Q3Product::Missionpack => Q3StatSchema {
            health: 0,
            holdable_item: 1,
            weapons: 3,
            armor: 4,
            dead_yaw: 5,
            clients_ready: 6,
            max_health: 7,
            persistent_powerup: Some(2),
        },
    }
}

/// Weapon slot count (`weaponCount`).
#[must_use]
pub fn weapon_count(product: Q3Product) -> usize {
    match product {
        Q3Product::Baseq3 => 11,
        Q3Product::Missionpack => 14,
    }
}

/// Movement type (`MoveType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum Q3MoveType {
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

/// Weapon state (`WeaponState`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum Q3WeaponState {
    /// Ready.
    Ready = 0,
    /// Raising.
    Raising = 1,
    /// Dropping.
    Dropping = 2,
    /// Firing.
    Firing = 3,
}

/// Powerup (`Powerup`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum Q3Powerup {
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
    Redflag = 7,
    /// Blue flag.
    Blueflag = 8,
    /// Neutral flag.
    Neutralflag = 9,
    /// Scout.
    Scout = 10,
    /// Guard.
    Guard = 11,
    /// Doubler.
    Doubler = 12,
    /// Ammo regeneration.
    Ammoregen = 13,
    /// Invulnerability.
    Invulnerability = 14,
    /// Powerup count.
    NumPowerups = 15,
}

/// Holdable (`Holdable`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum Q3Holdable {
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
    /// Holdable count.
    NumHoldable = 6,
}

/// Weapon (`Weapon`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum Q3Weapon {
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
    /// Lightning gun.
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

/// Entity event (`EntityEvent`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum Q3EntityEvent {
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
    Noammo = 21,
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
    /// Missile miss metal.
    MissileMissMetal = 52,
    /// Rail trail.
    Railtrail = 53,
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
    Scoreplum = 65,
    /// Proximity mine stick.
    ProximityMineStick = 66,
    /// Proximity mine trigger.
    ProximityMineTrigger = 67,
    /// Kamikaze.
    Kamikaze = 68,
    /// Obelisk explode.
    Obeliskexplode = 69,
    /// Obelisk pain.
    Obeliskpain = 70,
    /// Invulnerability impact.
    InvulImpact = 71,
    /// Juiced.
    Juiced = 72,
    /// Lightning bolt.
    Lightningbolt = 73,
    /// Debug line.
    DebugLine = 74,
    /// Stop looping sound.
    Stoploopingsound = 75,
    /// Taunt.
    Taunt = 76,
    /// Taunt yes.
    TauntYes = 77,
    /// Taunt no.
    TauntNo = 78,
    /// Taunt follow me.
    TauntFollowme = 79,
    /// Taunt get flag.
    TauntGetflag = 80,
    /// Taunt guard base.
    TauntGuardbase = 81,
    /// Taunt patrol.
    TauntPatrol = 82,
}

impl Q3EntityEvent {
    /// Convert a stored integer, accepting any value the donor event ring can hold.
    #[must_use]
    pub fn from_i32(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::None),
            1 => Some(Self::Footstep),
            2 => Some(Self::FootstepMetal),
            3 => Some(Self::Footsplash),
            4 => Some(Self::Footwade),
            5 => Some(Self::Swim),
            6 => Some(Self::Step4),
            7 => Some(Self::Step8),
            8 => Some(Self::Step12),
            9 => Some(Self::Step16),
            10 => Some(Self::FallShort),
            11 => Some(Self::FallMedium),
            12 => Some(Self::FallFar),
            13 => Some(Self::JumpPad),
            14 => Some(Self::Jump),
            15 => Some(Self::WaterTouch),
            16 => Some(Self::WaterLeave),
            17 => Some(Self::WaterUnder),
            18 => Some(Self::WaterClear),
            19 => Some(Self::ItemPickup),
            20 => Some(Self::GlobalItemPickup),
            21 => Some(Self::Noammo),
            22 => Some(Self::ChangeWeapon),
            23 => Some(Self::FireWeapon),
            24 => Some(Self::UseItem0),
            25 => Some(Self::UseItem1),
            26 => Some(Self::UseItem2),
            27 => Some(Self::UseItem3),
            28 => Some(Self::UseItem4),
            29 => Some(Self::UseItem5),
            30 => Some(Self::UseItem6),
            31 => Some(Self::UseItem7),
            32 => Some(Self::UseItem8),
            33 => Some(Self::UseItem9),
            34 => Some(Self::UseItem10),
            35 => Some(Self::UseItem11),
            36 => Some(Self::UseItem12),
            37 => Some(Self::UseItem13),
            38 => Some(Self::UseItem14),
            39 => Some(Self::UseItem15),
            40 => Some(Self::ItemRespawn),
            41 => Some(Self::ItemPop),
            42 => Some(Self::PlayerTeleportIn),
            43 => Some(Self::PlayerTeleportOut),
            44 => Some(Self::GrenadeBounce),
            45 => Some(Self::GeneralSound),
            46 => Some(Self::GlobalSound),
            47 => Some(Self::GlobalTeamSound),
            48 => Some(Self::BulletHitFlesh),
            49 => Some(Self::BulletHitWall),
            50 => Some(Self::MissileHit),
            51 => Some(Self::MissileMiss),
            52 => Some(Self::MissileMissMetal),
            53 => Some(Self::Railtrail),
            54 => Some(Self::Shotgun),
            55 => Some(Self::Bullet),
            56 => Some(Self::Pain),
            57 => Some(Self::Death1),
            58 => Some(Self::Death2),
            59 => Some(Self::Death3),
            60 => Some(Self::Obituary),
            61 => Some(Self::PowerupQuad),
            62 => Some(Self::PowerupBattlesuit),
            63 => Some(Self::PowerupRegen),
            64 => Some(Self::GibPlayer),
            65 => Some(Self::Scoreplum),
            66 => Some(Self::ProximityMineStick),
            67 => Some(Self::ProximityMineTrigger),
            68 => Some(Self::Kamikaze),
            69 => Some(Self::Obeliskexplode),
            70 => Some(Self::Obeliskpain),
            71 => Some(Self::InvulImpact),
            72 => Some(Self::Juiced),
            73 => Some(Self::Lightningbolt),
            74 => Some(Self::DebugLine),
            75 => Some(Self::Stoploopingsound),
            76 => Some(Self::Taunt),
            77 => Some(Self::TauntYes),
            78 => Some(Self::TauntNo),
            79 => Some(Self::TauntFollowme),
            80 => Some(Self::TauntGetflag),
            81 => Some(Self::TauntGuardbase),
            82 => Some(Self::TauntPatrol),
            _ => None,
        }
    }
}

/// Time knockback flag (`MoveFlags.TIME_KNOCKBACK`).
pub const MOVE_FLAG_TIME_KNOCKBACK: i32 = 64;

/// Damage flags (`DamageFlags` from `game/combat.ts`).
pub struct DamageFlags;

impl DamageFlags {
    /// Radius damage.
    pub const RADIUS: i32 = 0x1;
    /// Ignore armor.
    pub const NO_ARMOR: i32 = 0x2;
    /// No knockback.
    pub const NO_KNOCKBACK: i32 = 0x4;
    /// Ignore protection.
    pub const NO_PROTECTION: i32 = 0x8;
    /// Ignore team protection.
    pub const NO_TEAM_PROTECTION: i32 = 0x10;
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

// ---------------------------------------------------------------------------
// Trajectory mirror (donor `src/content/q3/base/shared/trajectory.ts`)
// ---------------------------------------------------------------------------

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

impl TrajectoryType {
    /// Convert a stored integer, rejecting unknown values.
    pub fn from_i32(value: i32) -> Result<Self, Q3GameError> {
        match value {
            0 => Ok(Self::Stationary),
            1 => Ok(Self::Interpolate),
            2 => Ok(Self::Linear),
            3 => Ok(Self::LinearStop),
            4 => Ok(Self::Sine),
            5 => Ok(Self::Gravity),
            _ => Err(drop_error(format!("BG_EvaluateTrajectory: unknown trType: {value}"))),
        }
    }
}

/// Trajectory (`Trajectory`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q3Trajectory {
    /// Type.
    pub trajectory_type: TrajectoryType,
    /// Start time.
    pub time: i32,
    /// Duration.
    pub duration: i32,
    /// Base.
    pub base: Vec3,
    /// Delta.
    pub delta: Vec3,
}

impl Q3Trajectory {
    /// Zero trajectory of a type.
    #[must_use]
    pub fn zero(trajectory_type: TrajectoryType) -> Self {
        Self {
            trajectory_type,
            time: 0,
            duration: 0,
            base: vec3(0.0, 0.0, 0.0),
            delta: vec3(0.0, 0.0, 0.0),
        }
    }
}

fn trajectory_seconds(milliseconds: i32) -> f32 {
    (milliseconds as f32) * 0.001
}

fn periodic_radians(tr: &Q3Trajectory, at_time: i32) -> f32 {
    let fraction = at_time.wrapping_sub(tr.time) as f32 / tr.duration as f32;
    fraction * std::f32::consts::PI * 2.0
}

/// Evaluate a trajectory (`BG_EvaluateTrajectory`).
#[must_use]
pub fn evaluate_trajectory(tr: &Q3Trajectory, at_time: i32) -> Vec3 {
    match tr.trajectory_type {
        TrajectoryType::Stationary | TrajectoryType::Interpolate => tr.base,
        TrajectoryType::Linear => {
            let delta_time = trajectory_seconds(at_time.wrapping_sub(tr.time));
            add3(tr.base, scale3(tr.delta, delta_time))
        }
        TrajectoryType::Sine => {
            let phase = periodic_radians(tr, at_time).sin();
            add3(tr.base, scale3(tr.delta, phase))
        }
        TrajectoryType::LinearStop => {
            let end = tr.time.wrapping_add(tr.duration);
            let time = at_time.min(end);
            let delta_time = trajectory_seconds(time.wrapping_sub(tr.time)).max(0.0);
            add3(tr.base, scale3(tr.delta, delta_time))
        }
        TrajectoryType::Gravity => {
            let delta_time = trajectory_seconds(at_time.wrapping_sub(tr.time));
            let result = add3(tr.base, scale3(tr.delta, delta_time));
            let fall = 0.5 * DEFAULT_GRAVITY * delta_time * delta_time;
            vec3(result.x, result.y, result.z - fall)
        }
    }
}

/// Evaluate a trajectory delta (`BG_EvaluateTrajectoryDelta`).
#[must_use]
pub fn evaluate_trajectory_delta(tr: &Q3Trajectory, at_time: i32) -> Vec3 {
    match tr.trajectory_type {
        TrajectoryType::Stationary | TrajectoryType::Interpolate => vec3(0.0, 0.0, 0.0),
        TrajectoryType::Linear => tr.delta,
        TrajectoryType::Sine => {
            let phase = periodic_radians(tr, at_time).cos() * 0.5;
            scale3(tr.delta, phase)
        }
        TrajectoryType::LinearStop => {
            if at_time > tr.time.wrapping_add(tr.duration) {
                vec3(0.0, 0.0, 0.0)
            } else {
                tr.delta
            }
        }
        TrajectoryType::Gravity => {
            let delta_time = trajectory_seconds(at_time.wrapping_sub(tr.time));
            vec3(tr.delta.x, tr.delta.y, tr.delta.z - DEFAULT_GRAVITY * delta_time)
        }
    }
}

// ---------------------------------------------------------------------------
// Body, contact, collision, and trace mirrors
// (donor `src/contracts/world.ts`, `src/content/q3/base/world.ts`,
// `src/content/q3/base/shared/entity-shared.ts`)
// ---------------------------------------------------------------------------

/// Shared body state (`BodyState`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3BodyState {
    /// Origin.
    pub origin: Vec3,
    /// Angles.
    pub angles: Vec3,
    /// Velocity.
    pub velocity: Vec3,
    /// Bounds.
    pub bounds: Bounds,
    /// Support actor.
    pub ground: Option<ActorId>,
}

/// Linked body snapshot (`LinkedBody`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3LinkedBody {
    /// Actor.
    pub actor: ActorId,
    /// State.
    pub state: Q3BodyState,
    /// Absolute bounds.
    pub absolute_bounds: Bounds,
    /// Link count.
    pub link_count: i32,
}

/// Collision model (`EntityCollisionModel`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3CollisionModel {
    /// Inline BSP model.
    Inline {
        /// Model index.
        index: i32,
    },
    /// Box.
    Box,
    /// Capsule.
    Capsule,
}

impl Default for Q3CollisionModel {
    fn default() -> Self {
        Self::Box
    }
}

/// Touch surface (`TouchContact["surface"]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3TouchSurface {
    /// Name.
    pub name: String,
    /// Native flags.
    pub native_flags: i32,
    /// Native value.
    pub native_value: i32,
}

/// Touch contact (`TouchContact`).
#[derive(Debug, Clone, PartialEq)]
pub struct TouchContact {
    /// Self actor.
    pub self_actor: ActorId,
    /// Other actor.
    pub other: ActorId,
    /// Contact plane.
    pub plane: Option<qa_core::math::Plane>,
    /// Surface.
    pub surface: Option<Q3TouchSurface>,
}

/// Trace shape (`TraceShape`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Q3TraceShape {
    /// Point.
    Point,
    /// Box.
    Box {
        /// Mins.
        mins: Vec3,
        /// Maxs.
        maxs: Vec3,
    },
    /// Capsule.
    Capsule {
        /// Mins.
        mins: Vec3,
        /// Maxs.
        maxs: Vec3,
    },
}

/// Trace solidity (`ServerTraceResult["solidity"]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3Solidity {
    /// Clear.
    Clear,
    /// Start solid.
    StartSolid,
    /// All solid.
    AllSolid,
}

/// Trace contact (`ServerTraceResult["contact"]`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Q3TraceContact {
    /// None.
    None,
    /// Plane.
    Plane {
        /// Normal.
        normal: Vec3,
        /// Distance.
        distance: f32,
    },
}

/// Trace hit (`ActorTraceResult["hit"]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Q3TraceHit {
    /// None.
    None,
    /// World.
    World,
    /// Actor.
    Actor(ActorId),
}

/// Actor trace query (`ActorTraceQuery`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3TraceQuery {
    /// Start.
    pub start: Vec3,
    /// End.
    pub end: Vec3,
    /// Shape.
    pub shape: Q3TraceShape,
    /// Pass actor.
    pub pass_actor: Option<ActorId>,
    /// Mask.
    pub mask: i32,
}

/// Actor trace result (`ActorTraceResult`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3TraceResult {
    /// Fraction.
    pub fraction: f32,
    /// End position.
    pub end: Vec3,
    /// Solidity.
    pub solidity: Q3Solidity,
    /// Contact.
    pub contact: Q3TraceContact,
    /// Contents.
    pub contents: i32,
    /// Surface flags.
    pub surface_flags: i32,
    /// Hit.
    pub hit: Q3TraceHit,
}

/// Link state (`LinkState`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q3LinkState {
    /// Absolute bounds.
    pub absbounds: Bounds,
    /// Linked.
    pub linked: bool,
    /// Link count.
    pub linkcount: i32,
}

// ---------------------------------------------------------------------------
// Entity state / shared / player / item mirrors
// (donor `src/network/q3/state/entity.ts`,
// `src/content/q3/base/shared/entity-shared.ts`,
// `src/content/q3/base/shared/player-state.ts`,
// `src/content/q3/base/shared/items.ts`)
// ---------------------------------------------------------------------------

/// Network entity state (`EntityState` / `EntityStateFields`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3EntityState {
    /// Number.
    pub number: i32,
    /// Entity type.
    pub e_type: i32,
    /// Flags.
    pub e_flags: i32,
    /// Position trajectory.
    pub pos: Q3Trajectory,
    /// Angle trajectory.
    pub apos: Q3Trajectory,
    /// Time.
    pub time: i32,
    /// Time 2.
    pub time2: i32,
    /// Origin.
    pub origin: Vec3,
    /// Origin 2.
    pub origin2: Vec3,
    /// Angles.
    pub angles: Vec3,
    /// Angles 2.
    pub angles2: Vec3,
    /// Other entity number.
    pub other_entity_num: i32,
    /// Other entity number 2.
    pub other_entity_num2: i32,
    /// Ground entity number.
    pub ground_entity_num: i32,
    /// Constant light.
    pub constant_light: i32,
    /// Loop sound.
    pub loop_sound: i32,
    /// Model index.
    pub modelindex: i32,
    /// Model index 2.
    pub modelindex2: i32,
    /// Client number.
    pub client_num: i32,
    /// Frame.
    pub frame: i32,
    /// Solid.
    pub solid: i32,
    /// Event.
    pub event: i32,
    /// Event parameter.
    pub event_parm: i32,
    /// Powerups.
    pub powerups: i32,
    /// Weapon.
    pub weapon: i32,
    /// Legs animation.
    pub legs_anim: i32,
    /// Torso animation.
    pub torso_anim: i32,
    /// Generic 1.
    pub generic1: i32,
}

impl Default for Q3EntityState {
    fn default() -> Self {
        Self {
            number: 0,
            e_type: 0,
            e_flags: 0,
            pos: Q3Trajectory::zero(TrajectoryType::Stationary),
            apos: Q3Trajectory::zero(TrajectoryType::Stationary),
            time: 0,
            time2: 0,
            origin: vec3(0.0, 0.0, 0.0),
            origin2: vec3(0.0, 0.0, 0.0),
            angles: vec3(0.0, 0.0, 0.0),
            angles2: vec3(0.0, 0.0, 0.0),
            other_entity_num: 0,
            other_entity_num2: 0,
            ground_entity_num: 0,
            constant_light: 0,
            loop_sound: 0,
            modelindex: 0,
            modelindex2: 0,
            client_num: 0,
            frame: 0,
            solid: 0,
            event: 0,
            event_parm: 0,
            powerups: 0,
            weapon: 0,
            legs_anim: 0,
            torso_anim: 0,
            generic1: 0,
        }
    }
}

/// Shared entity fields (`EntityShared`, flattened to plain data).
#[derive(Debug, Clone, PartialEq)]
pub struct EntitySharedFields {
    /// Server flags.
    pub sv_flags: i32,
    /// Single client.
    pub single_client: i32,
    /// Collision model.
    pub model: Q3CollisionModel,
    /// Contents.
    pub contents: i32,
    /// Owner number.
    pub owner_num: i32,
    /// Mins.
    pub mins: Vec3,
    /// Maxs.
    pub maxs: Vec3,
    /// Current origin.
    pub current_origin: Vec3,
    /// Current angles.
    pub current_angles: Vec3,
    /// Linked bounds, when linked.
    pub linked_bounds: Option<Bounds>,
    /// Linked flag.
    pub linked: bool,
    /// Absolute-min override (`absmin` setter).
    pub absmin_override: Option<Vec3>,
    /// Absolute-max override (`absmax` setter).
    pub absmax_override: Option<Vec3>,
    /// Previous link snapshot.
    pub previous_link: Option<Q3LinkedBody>,
    /// Support actor (`binding.body` ground projection).
    pub ground: Option<ActorId>,
}

impl Default for EntitySharedFields {
    fn default() -> Self {
        Self {
            sv_flags: 0,
            single_client: 0,
            model: Q3CollisionModel::Box,
            contents: 0,
            owner_num: 0,
            mins: vec3(0.0, 0.0, 0.0),
            maxs: vec3(0.0, 0.0, 0.0),
            current_origin: vec3(0.0, 0.0, 0.0),
            current_angles: vec3(0.0, 0.0, 0.0),
            linked_bounds: None,
            linked: false,
            absmin_override: None,
            absmax_override: None,
            previous_link: None,
            ground: None,
        }
    }
}

impl EntitySharedFields {
    /// Absolute mins (`absmin` getter chain).
    #[must_use]
    pub fn absmin(&self) -> Vec3 {
        self.absmin_override
            .or_else(|| self.linked_bounds.map(|bounds| bounds.min))
            .or_else(|| self.previous_link.as_ref().map(|link| link.absolute_bounds.min))
            .unwrap_or_else(|| vec3(0.0, 0.0, 0.0))
    }

    /// Absolute maxs (`absmax` getter chain).
    #[must_use]
    pub fn absmax(&self) -> Vec3 {
        self.absmax_override
            .or_else(|| self.linked_bounds.map(|bounds| bounds.max))
            .or_else(|| self.previous_link.as_ref().map(|link| link.absolute_bounds.max))
            .unwrap_or_else(|| vec3(0.0, 0.0, 0.0))
    }

    /// Body state projection (`binding.body.read()`).
    #[must_use]
    pub fn body_state(&self, velocity: Vec3) -> Q3BodyState {
        Q3BodyState {
            origin: self.current_origin,
            angles: self.current_angles,
            velocity,
            bounds: Bounds {
                min: self.mins,
                max: self.maxs,
            },
            ground: self.ground.clone(),
        }
    }
}

/// Player slot storage (`PlayerStateSlots`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3PlayerSlots {
    values: Vec<i32>,
}

impl Q3PlayerSlots {
    /// Zeroed slots.
    #[must_use]
    pub fn new(length: usize) -> Self {
        Self {
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
    #[must_use]
    pub fn get(&self, index: usize) -> i32 {
        self.values[index]
    }

    /// Write a slot.
    pub fn set(&mut self, index: usize, value: i32) {
        self.values[index] = value;
    }

    /// Copy all slots (`copy()`).
    #[must_use]
    pub fn copy_vec(&self) -> Vec<i32> {
        self.values.clone()
    }

    /// Restore all slots, requiring an exact count (`restoreSlots`).
    pub fn restore(&mut self, values: &[i32]) -> Result<(), Q3GameError> {
        if values.len() != self.values.len() {
            return Err(failure("Q3 saved player slot count mismatch"));
        }
        self.values.copy_from_slice(values);
        Ok(())
    }
}

/// User command (`UserCommand`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q3UserCommand {
    /// Server time.
    pub server_time: i32,
    /// Angles.
    pub angles: Vec3,
    /// Buttons.
    pub buttons: i32,
    /// Weapon.
    pub weapon: i32,
    /// Forward move.
    pub forwardmove: i32,
    /// Right move.
    pub rightmove: i32,
    /// Up move.
    pub upmove: i32,
}

impl Default for Q3UserCommand {
    fn default() -> Self {
        Self {
            server_time: 0,
            angles: vec3(0.0, 0.0, 0.0),
            buttons: 0,
            weapon: Q3Weapon::None as i32,
            forwardmove: 0,
            rightmove: 0,
            upmove: 0,
        }
    }
}

/// Player state (`PlayerState`, flattened to plain data).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3PlayerState {
    /// Product.
    pub product: Q3Product,
    /// Command time.
    pub command_time: i32,
    /// Movement type.
    pub pm_type: i32,
    /// Bob cycle.
    pub bob_cycle: i32,
    /// Move flags.
    pub pm_flags: i32,
    /// Move time.
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
    pub delta_angles: [i32; 3],
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
    /// Entity flags.
    pub e_flags: i32,
    /// Event sequence.
    pub event_sequence: i32,
    /// Events ring.
    pub events: Q3PlayerSlots,
    /// Event parameters ring.
    pub event_parms: Q3PlayerSlots,
    /// External event.
    pub external_event: i32,
    /// External event parameter.
    pub external_event_parm: i32,
    /// External event time.
    pub external_event_time: i32,
    /// Client number.
    pub client_num: i32,
    /// Weapon.
    pub weapon: i32,
    /// Weapon state.
    pub weapon_state: i32,
    /// View angles.
    pub viewangles: Vec3,
    /// View height.
    pub viewheight: f32,
    /// Damage event.
    pub damage_event: i32,
    /// Damage yaw.
    pub damage_yaw: i32,
    /// Damage pitch.
    pub damage_pitch: i32,
    /// Damage count.
    pub damage_count: i32,
    /// Stats.
    pub stats: Q3PlayerSlots,
    /// Persistant.
    pub persistant: Q3PlayerSlots,
    /// Powerups.
    pub powerups: Q3PlayerSlots,
    /// Ammo.
    pub ammo: Q3PlayerSlots,
    /// Generic 1.
    pub generic1: i32,
    /// Loop sound.
    pub loop_sound: i32,
    /// Jump pad entity.
    pub jumppad_ent: i32,
    /// Ping.
    pub ping: i32,
    /// Pmove frame count.
    pub pmove_framecount: i32,
    /// Jump pad frame.
    pub jumppad_frame: i32,
    /// Entity event sequence.
    pub entity_event_sequence: i32,
}

impl Q3PlayerState {
    /// Zeroed player state for a product (`createPlayerState`).
    #[must_use]
    pub fn new(product: Q3Product) -> Self {
        Self {
            product,
            command_time: 0,
            pm_type: 0,
            bob_cycle: 0,
            pm_flags: 0,
            pm_time: 0,
            origin: vec3(0.0, 0.0, 0.0),
            velocity: vec3(0.0, 0.0, 0.0),
            weapon_time: 0,
            gravity: 0,
            speed: 0,
            delta_angles: [0, 0, 0],
            ground_entity_num: 0,
            legs_timer: 0,
            legs_anim: 0,
            torso_timer: 0,
            torso_anim: 0,
            movement_dir: 0,
            grapple_point: vec3(0.0, 0.0, 0.0),
            e_flags: 0,
            event_sequence: 0,
            events: Q3PlayerSlots::new(2),
            event_parms: Q3PlayerSlots::new(2),
            external_event: 0,
            external_event_parm: 0,
            external_event_time: 0,
            client_num: 0,
            weapon: 0,
            weapon_state: 0,
            viewangles: vec3(0.0, 0.0, 0.0),
            viewheight: 0.0,
            damage_event: 0,
            damage_yaw: 0,
            damage_pitch: 0,
            damage_count: 0,
            stats: Q3PlayerSlots::new(16),
            persistant: Q3PlayerSlots::new(16),
            powerups: Q3PlayerSlots::new(16),
            ammo: Q3PlayerSlots::new(16),
            generic1: 0,
            loop_sound: 0,
            jumppad_ent: 0,
            ping: 0,
            pmove_framecount: 0,
            jumppad_frame: 0,
            entity_event_sequence: 0,
        }
    }

    /// Health stat (`get health()`).
    #[must_use]
    pub fn health(&self) -> i32 {
        self.stats.get(stat_schema(self.product).health)
    }

    /// Write the health stat (`set health()`).
    pub fn set_health(&mut self, value: i32) {
        let slot = stat_schema(self.product).health;
        self.stats.set(slot, value);
    }

    /// Queue a predictable event (`addEvent`).
    pub fn add_event(&mut self, event: i32, parameter: i32) {
        let sequence = self.event_sequence;
        let index = (sequence & 1) as usize;
        self.events.set(index, event);
        self.event_parms.set(index, parameter);
        self.event_sequence = sequence.wrapping_add(1);
    }
}

/// Item definition (`ItemDefinition`: class name, pickup name, type, tag).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3ItemDefinition {
    /// Entity class name.
    pub class_name: Option<String>,
    /// Pickup name.
    pub pickup_name: Option<String>,
    /// Item type.
    pub item_type: Q3ItemType,
    /// Type tag (weapon, powerup, or holdable number).
    pub tag: i32,
}

// ---------------------------------------------------------------------------
// state.ts: game state (`src/content/q3/base/game/state.ts`, g_local.h)
// ---------------------------------------------------------------------------

/// Maximum clients (`MAX_CLIENTS`).
pub const MAX_CLIENTS: usize = 64;
/// Maximum entities (`MAX_GENTITIES`).
pub const MAX_GENTITIES: usize = 1024;

/// Connection state (`ConnectionState`).
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

impl ConnectionState {
    /// Convert a stored integer, rejecting unknown values.
    pub fn from_i32(value: i32) -> Result<Self, Q3GameError> {
        match value {
            0 => Ok(Self::Disconnected),
            1 => Ok(Self::Connecting),
            2 => Ok(Self::Connected),
            _ => Err(failure(format!("unknown connection state {value}"))),
        }
    }
}

/// Spectator state (`SpectatorState`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum SpectatorState {
    /// Not spectating.
    Not = 0,
    /// Free.
    Free = 1,
    /// Follow.
    Follow = 2,
    /// Scoreboard.
    Scoreboard = 3,
}

/// Team state (`TeamState`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum TeamState {
    /// Begin.
    Begin = 0,
    /// Active.
    Active = 1,
}

/// Mover state (`MoverState`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum MoverState {
    /// At position 1.
    Pos1 = 0,
    /// At position 2.
    Pos2 = 1,
    /// Moving 1 to 2.
    OneToTwo = 2,
    /// Moving 2 to 1.
    TwoToOne = 3,
}

impl MoverState {
    /// Convert a stored integer, rejecting unknown values.
    pub fn from_i32(value: i32) -> Result<Self, Q3GameError> {
        match value {
            0 => Ok(Self::Pos1),
            1 => Ok(Self::Pos2),
            2 => Ok(Self::OneToTwo),
            3 => Ok(Self::TwoToOne),
            _ => Err(failure(format!("Invalid mover state {value}"))),
        }
    }
}

/// Game flags (`GameFlags`).
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

/// Player team state (`PlayerTeamState`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerTeamState {
    /// State.
    pub state: i32,
    /// Location.
    pub location: i32,
    /// Captures.
    pub captures: i32,
    /// Base defense.
    pub base_defense: i32,
    /// Carrier defense.
    pub carrier_defense: i32,
    /// Flag recovery.
    pub flag_recovery: i32,
    /// Frag carrier.
    pub frag_carrier: i32,
    /// Assists.
    pub assists: i32,
    /// Last hurt carrier.
    pub last_hurt_carrier: i32,
    /// Last returned flag.
    pub last_returned_flag: i32,
    /// Flag since.
    pub flag_since: i32,
    /// Last fragged carrier.
    pub last_fragged_carrier: i32,
}

impl Default for PlayerTeamState {
    fn default() -> Self {
        Self {
            state: TeamState::Begin as i32,
            location: 0,
            captures: 0,
            base_defense: 0,
            carrier_defense: 0,
            flag_recovery: 0,
            frag_carrier: 0,
            assists: 0,
            last_hurt_carrier: 0,
            last_returned_flag: 0,
            flag_since: 0,
            last_fragged_carrier: 0,
        }
    }
}

/// Client session (`ClientSession`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientSession {
    /// Session team.
    pub session_team: i32,
    /// Spectator time.
    pub spectator_time: i32,
    /// Spectator state.
    pub spectator_state: i32,
    /// Spectator client.
    pub spectator_client: i32,
    /// Wins.
    pub wins: i32,
    /// Losses.
    pub losses: i32,
    /// Team leader.
    pub team_leader: i32,
}

impl Default for ClientSession {
    fn default() -> Self {
        Self {
            session_team: Q3Team::Free as i32,
            spectator_time: 0,
            spectator_state: SpectatorState::Not as i32,
            spectator_client: 0,
            wins: 0,
            losses: 0,
            team_leader: 0,
        }
    }
}

/// Persistant client data (`ClientPersistant`).
#[derive(Debug, Clone, PartialEq)]
pub struct ClientPersistant {
    /// Connected.
    pub connected: i32,
    /// Command.
    pub cmd: Q3UserCommand,
    /// Local client.
    pub local_client: bool,
    /// Initial spawn.
    pub initial_spawn: bool,
    /// Predict item pickup.
    pub predict_item_pickup: bool,
    /// Pmove fixed.
    pub pmove_fixed: bool,
    /// Net name.
    pub netname: String,
    /// Max health.
    pub max_health: i32,
    /// Enter time.
    pub enter_time: i32,
    /// Team state.
    pub team_state: PlayerTeamState,
    /// Vote count.
    pub vote_count: i32,
    /// Team vote count.
    pub team_vote_count: i32,
    /// Team info.
    pub team_info: bool,
}

impl Default for ClientPersistant {
    fn default() -> Self {
        Self {
            connected: ConnectionState::Disconnected as i32,
            cmd: Q3UserCommand::default(),
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

/// Damage/use participant (`DamageParticipant` / `UseParticipant` /
/// `DamageInflictor`): a native entity slot or a shared actor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Participant {
    /// Native entity slot.
    Entity(usize),
    /// Shared actor.
    SharedActor(ActorId),
}

/// Think callback (`EntityThink`).
pub type EntityThink = Rc<dyn Fn(&mut dyn Q3Driver, usize)>;
/// Blocked callback (`EntityBlocked`).
pub type EntityBlocked = Rc<dyn Fn(&mut dyn Q3Driver, usize, &Participant)>;
/// Touch callback (`EntityTouch`).
pub type EntityTouch = Rc<dyn Fn(&mut dyn Q3Driver, usize, &Participant, &TouchContact)>;
/// Use callback (`EntityUse`).
pub type EntityUse = Rc<dyn Fn(&mut dyn Q3Driver, usize, Option<&Participant>, Option<&Participant>)>;
/// Pain callback (`EntityPain`).
pub type EntityPain = Rc<dyn Fn(&mut dyn Q3Driver, usize, &Participant, i32)>;
/// Die callback (`EntityDie`).
pub type EntityDie = Rc<dyn Fn(&mut dyn Q3Driver, usize, &Participant, &Participant, i32, i32)>;

/// Entity classname binding (`GameEntity` classname state).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClassName {
    /// Plain value.
    Value(Option<String>),
    /// Bound to a client netname.
    ClientName(usize),
}

/// Game client (`GameClient`, source-zero gclient_t).
#[derive(Debug, Clone, PartialEq)]
pub struct GameClient {
    /// Player state.
    pub ps: Q3PlayerState,
    /// Persistant data.
    pub pers: ClientPersistant,
    /// Session.
    pub sess: ClientSession,
    /// Ready to exit.
    pub ready_to_exit: bool,
    /// Noclip.
    pub noclip: bool,
    /// Last command time.
    pub last_cmd_time: i32,
    /// Buttons.
    pub buttons: i32,
    /// Old buttons.
    pub old_buttons: i32,
    /// Latched buttons.
    pub latched_buttons: i32,
    /// Old origin.
    pub old_origin: Vec3,
    /// Damage armor.
    pub damage_armor: i32,
    /// Damage blood.
    pub damage_blood: i32,
    /// Damage knockback.
    pub damage_knockback: i32,
    /// Damage from.
    pub damage_from: Vec3,
    /// Damage from world.
    pub damage_from_world: bool,
    /// Accurate count.
    pub accurate_count: i32,
    /// Accuracy shots.
    pub accuracy_shots: i32,
    /// Accuracy hits.
    pub accuracy_hits: i32,
    /// Last killed client.
    pub last_killed_client: i32,
    /// Last hurt client.
    pub last_hurt_client: i32,
    /// Last hurt means of death.
    pub last_hurt_mod: i32,
    /// Respawn time.
    pub respawn_time: i32,
    /// Inactivity time.
    pub inactivity_time: i32,
    /// Inactivity warning.
    pub inactivity_warning: bool,
    /// Reward time.
    pub reward_time: i32,
    /// Air out time.
    pub air_out_time: i32,
    /// Last kill time.
    pub last_kill_time: i32,
    /// Fire held.
    pub fire_held: bool,
    /// Hook entity slot.
    pub hook: Option<usize>,
    /// Switch team time.
    pub switch_team_time: i32,
    /// Time residual.
    pub time_residual: i32,
    /// Persistant powerup entity slot.
    pub persistant_powerup: Option<usize>,
    /// Portal identifier.
    pub portal_id: i32,
    /// Ammo times.
    pub ammo_times: Q3PlayerSlots,
    /// Invulnerability time.
    pub invulnerability_time: i32,
    /// Area bits.
    pub areabits: Option<Vec<u8>>,
}

impl GameClient {
    /// Source-zero client for a product.
    #[must_use]
    pub fn new(product: Q3Product) -> Self {
        Self {
            ps: Q3PlayerState::new(product),
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
            accurate_count: 0,
            accuracy_shots: 0,
            accuracy_hits: 0,
            last_killed_client: 0,
            last_hurt_client: 0,
            last_hurt_mod: 0,
            respawn_time: 0,
            inactivity_time: 0,
            inactivity_warning: false,
            reward_time: 0,
            air_out_time: 0,
            last_kill_time: 0,
            fire_held: false,
            hook: None,
            switch_team_time: 0,
            time_residual: 0,
            persistant_powerup: None,
            portal_id: 0,
            ammo_times: Q3PlayerSlots::new(weapon_count(product)),
            invulnerability_time: 0,
            areabits: None,
        }
    }
}

/// Game entity (`GameEntity`, source-zero gentity_t with slot metadata).
#[derive(Clone)]
pub struct GameEntity {
    /// Entity slot.
    pub slot: usize,
    /// In-use flag (`binding.active()` projection).
    pub inuse: bool,
    /// Actor handle.
    pub actor: ActorId,
    /// Network state.
    pub s: Q3EntityState,
    /// Shared fields.
    pub r: EntitySharedFields,
    /// Client slot.
    pub client: Option<usize>,
    classname: ClassName,
    /// Spawn flags.
    pub spawnflags: i32,
    /// Never free.
    pub never_free: bool,
    /// Flags.
    pub flags: i32,
    /// Model.
    pub model: Option<String>,
    /// Model 2.
    pub model2: Option<String>,
    /// Free time.
    pub freetime: i32,
    /// Event time.
    pub event_time: i32,
    /// Free after event.
    pub free_after_event: bool,
    /// Unlink after event.
    pub unlink_after_event: bool,
    /// Physics object.
    pub physics_object: bool,
    /// Physics bounce.
    pub physics_bounce: i32,
    /// Clip mask.
    pub clipmask: i32,
    /// Mover state.
    pub mover_state: i32,
    /// Sound position 1.
    pub sound_pos1: i32,
    /// Sound 1 to 2.
    pub sound1to2: i32,
    /// Sound 2 to 1.
    pub sound2to1: i32,
    /// Sound position 2.
    pub sound_pos2: i32,
    /// Sound loop.
    pub sound_loop: i32,
    /// Parent slot.
    pub parent: Option<usize>,
    /// Next train slot.
    pub next_train: Option<usize>,
    /// Previous train slot.
    pub prev_train: Option<usize>,
    /// Position 1.
    pub pos1: Vec3,
    /// Position 2.
    pub pos2: Vec3,
    /// Message.
    pub message: Option<String>,
    /// Timestamp.
    pub timestamp: i32,
    /// Angle.
    pub angle: f32,
    /// Target.
    pub target: Option<String>,
    /// Target name.
    pub targetname: Option<String>,
    /// Team.
    pub team: Option<String>,
    /// Target shader name.
    pub target_shader_name: Option<String>,
    /// Target shader new name.
    pub target_shader_new_name: Option<String>,
    /// Target entity slot.
    pub target_ent: Option<usize>,
    /// Speed.
    pub speed: f32,
    /// Move direction.
    pub movedir: Vec3,
    /// Next think time (raw field; scheduling goes through the pool hook).
    pub nextthink: i32,
    /// Think callback.
    pub think: Option<EntityThink>,
    /// Reached callback.
    pub reached: Option<EntityThink>,
    /// Blocked callback.
    pub blocked: Option<EntityBlocked>,
    /// Touch callback.
    pub touch: Option<EntityTouch>,
    /// Use callback.
    pub use_callback: Option<EntityUse>,
    /// Pain callback.
    pub pain: Option<EntityPain>,
    /// Die callback.
    pub die: Option<EntityDie>,
    /// Pain debounce time.
    pub pain_debounce_time: i32,
    /// Fly sound debounce time.
    pub fly_sound_debounce_time: i32,
    /// Last move time.
    pub last_move_time: i32,
    /// Health (`binding.health()` projection).
    pub health: i32,
    /// Takes damage (`binding.takedamage()` projection).
    pub takedamage: bool,
    /// Damage.
    pub damage: i32,
    /// Splash damage.
    pub splash_damage: i32,
    /// Splash radius.
    pub splash_radius: i32,
    /// Means of death.
    pub method_of_death: i32,
    /// Splash means of death.
    pub splash_method_of_death: i32,
    /// Count.
    pub count: i32,
    /// Chain slot.
    pub chain: Option<usize>,
    /// Enemy slot.
    pub enemy: Option<usize>,
    /// Activator slot.
    pub activator: Option<usize>,
    /// Activation participant.
    pub activation: Option<Participant>,
    /// Team chain slot.
    pub teamchain: Option<usize>,
    /// Team master slot.
    pub teammaster: Option<usize>,
    /// Kamikaze time.
    pub kamikaze_time: i32,
    /// Kamikaze shock time.
    pub kamikaze_shock_time: i32,
    /// Water type.
    pub watertype: i32,
    /// Water level.
    pub waterlevel: i32,
    /// Noise index.
    pub noise_index: i32,
    /// Wait.
    pub wait: f32,
    /// Random.
    pub random: f32,
    /// Item table index.
    pub item: Option<usize>,
}

impl GameEntity {
    /// Source-zero entity in a slot with an actor handle.
    pub fn new(slot: usize, actor: ActorId) -> Result<Self, Q3GameError> {
        if slot >= MAX_GENTITIES {
            return Err(range("Game entity slot outside 0..1023"));
        }
        Ok(Self {
            slot,
            inuse: false,
            actor,
            s: Q3EntityState::default(),
            r: EntitySharedFields::default(),
            client: None,
            classname: ClassName::Value(None),
            spawnflags: 0,
            never_free: false,
            flags: 0,
            model: None,
            model2: None,
            freetime: 0,
            event_time: 0,
            free_after_event: false,
            unlink_after_event: false,
            physics_object: false,
            physics_bounce: 0,
            clipmask: 0,
            mover_state: MoverState::Pos1 as i32,
            sound_pos1: 0,
            sound1to2: 0,
            sound2to1: 0,
            sound_pos2: 0,
            sound_loop: 0,
            parent: None,
            next_train: None,
            prev_train: None,
            pos1: vec3(0.0, 0.0, 0.0),
            pos2: vec3(0.0, 0.0, 0.0),
            message: None,
            timestamp: 0,
            angle: 0.0,
            target: None,
            targetname: None,
            team: None,
            target_shader_name: None,
            target_shader_new_name: None,
            target_ent: None,
            speed: 0.0,
            movedir: vec3(0.0, 0.0, 0.0),
            nextthink: 0,
            think: None,
            reached: None,
            blocked: None,
            touch: None,
            use_callback: None,
            pain: None,
            die: None,
            pain_debounce_time: 0,
            fly_sound_debounce_time: 0,
            last_move_time: 0,
            health: 0,
            takedamage: false,
            damage: 0,
            splash_damage: 0,
            splash_radius: 0,
            method_of_death: 0,
            splash_method_of_death: 0,
            count: 0,
            chain: None,
            enemy: None,
            activator: None,
            activation: None,
            teamchain: None,
            teammaster: None,
            kamikaze_time: 0,
            kamikaze_shock_time: 0,
            watertype: 0,
            waterlevel: 0,
            noise_index: 0,
            wait: 0.0,
            random: 0.0,
            item: None,
        })
    }

    /// Read the classname (`get classname()`; client-bound names need the pool).
    #[must_use]
    pub fn classname_value(&self) -> Option<&str> {
        match &self.classname {
            ClassName::Value(value) => value.as_deref(),
            ClassName::ClientName(_) => None,
        }
    }

    /// Read the classname state.
    #[must_use]
    pub fn classname_state(&self) -> &ClassName {
        &self.classname
    }

    /// Write the classname (`set classname()`).
    pub fn set_classname(&mut self, value: Option<String>) {
        self.classname = ClassName::Value(value);
    }

    /// Bind the classname to a client netname (`bindClientName`).
    pub fn bind_client_name(&mut self, client: usize) {
        self.classname = ClassName::ClientName(client);
    }

    /// Capture the classname binding (`captureClassname`).
    #[must_use]
    pub fn capture_classname(&self) -> ClassName {
        self.classname.clone()
    }

    /// Resolve a possibly client-bound classname through client netnames.
    #[must_use]
    pub fn resolve_classname(&self, netname: Option<&str>) -> Option<String> {
        match &self.classname {
            ClassName::Value(value) => value.clone(),
            ClassName::ClientName(_) => netname.map(str::to_string),
        }
    }
}

impl std::fmt::Debug for GameEntity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GameEntity")
            .field("slot", &self.slot)
            .field("inuse", &self.inuse)
            .field("actor", &self.actor)
            .field("s", &self.s)
            .field("r", &self.r)
            .field("client", &self.client)
            .field("classname", &self.classname)
            .field("health", &self.health)
            .field("takedamage", &self.takedamage)
            .field("think", &self.think.is_some())
            .field("reached", &self.reached.is_some())
            .field("blocked", &self.blocked.is_some())
            .field("touch", &self.touch.is_some())
            .field("use_callback", &self.use_callback.is_some())
            .field("pain", &self.pain.is_some())
            .field("die", &self.die.is_some())
            .finish_non_exhaustive()
    }
}

/// Create a game client (`createGameClient`).
#[must_use]
pub fn create_game_client(product: Q3Product) -> GameClient {
    GameClient::new(product)
}

/// Create a game entity (`createGameEntity`).
pub fn create_game_entity(slot: usize, actor: ActorId) -> Result<GameEntity, Q3GameError> {
    GameEntity::new(slot, actor)
}

// ---------------------------------------------------------------------------
// save-callbacks.ts: callback families and catalog
// ---------------------------------------------------------------------------

/// One callback family (`Q3CallbackFamily`); identity compares handle pointers.
pub struct CallbackTable<F: ?Sized> {
    entries: Vec<(String, Rc<F>)>,
}

impl<F: ?Sized> std::fmt::Debug for CallbackTable<F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let ids: Vec<&str> = self.entries.iter().map(|(id, _)| id.as_str()).collect();
        f.debug_struct("CallbackTable").field("ids", &ids).finish()
    }
}

impl<F: ?Sized> Default for CallbackTable<F> {
    fn default() -> Self {
        Self { entries: Vec::new() }
    }
}

impl<F: ?Sized> CallbackTable<F> {
    /// Empty table.
    #[must_use]
    pub fn new() -> Self {
        Self { entries: Vec::new() }
    }

    /// Register an identity (`register`).
    pub fn register(&mut self, id: &str, callback: Rc<F>) -> Result<Rc<F>, Q3GameError> {
        if id.is_empty() {
            return Err(failure("Q3 callback identity is empty"));
        }
        for (known, known_callback) in &self.entries {
            if known == id && !Rc::ptr_eq(known_callback, &callback) {
                return Err(failure(format!("Duplicate Q3 callback identity {id}")));
            }
            if Rc::ptr_eq(known_callback, &callback) && known != id {
                return Err(failure(format!("Q3 callback has two identities: {known}, {id}")));
            }
        }
        if !self.entries.iter().any(|(known, _)| known == id) {
            self.entries.push((id.to_string(), callback.clone()));
        }
        Ok(callback)
    }

    /// Register unless present (`intern`).
    pub fn intern(&mut self, id: &str, callback: Rc<F>) -> Result<Rc<F>, Q3GameError> {
        for (known, known_callback) in &self.entries {
            if known == id {
                return Ok(known_callback.clone());
            }
        }
        self.register(id, callback)
    }

    /// Capture an identity (`capture`).
    pub fn capture(&self, callback: Option<&Rc<F>>) -> Result<Option<String>, Q3GameError> {
        let Some(callback) = callback else { return Ok(None) };
        for (known, known_callback) in &self.entries {
            if Rc::ptr_eq(known_callback, callback) {
                return Ok(Some(known.clone()));
            }
        }
        Err(failure("Unregistered native Q3 callback cannot be saved"))
    }

    /// Resolve an identity (`resolve`).
    pub fn resolve(&self, id: Option<&str>) -> Result<Option<Rc<F>>, Q3GameError> {
        let Some(id) = id else { return Ok(None) };
        for (known, known_callback) in &self.entries {
            if known == id {
                return Ok(Some(known_callback.clone()));
            }
        }
        Err(failure(format!("Unknown native Q3 callback {id}")))
    }
}

/// Callback catalog (`Q3CallbackCatalog`).
#[derive(Debug)]
pub struct Q3CallbackCatalog {
    /// Think family.
    pub think: CallbackTable<dyn Fn(&mut dyn Q3Driver, usize)>,
    /// Reached family.
    pub reached: CallbackTable<dyn Fn(&mut dyn Q3Driver, usize)>,
    /// Blocked family.
    pub blocked: CallbackTable<dyn Fn(&mut dyn Q3Driver, usize, &Participant)>,
    /// Touch family.
    pub touch: CallbackTable<dyn Fn(&mut dyn Q3Driver, usize, &Participant, &TouchContact)>,
    /// Use family.
    pub use_callbacks: CallbackTable<dyn Fn(&mut dyn Q3Driver, usize, Option<&Participant>, Option<&Participant>)>,
    /// Pain family.
    pub pain: CallbackTable<dyn Fn(&mut dyn Q3Driver, usize, &Participant, i32)>,
    /// Die family.
    pub die: CallbackTable<dyn Fn(&mut dyn Q3Driver, usize, &Participant, &Participant, i32, i32)>,
}

impl Q3CallbackCatalog {
    /// Empty catalog.
    #[must_use]
    pub fn new() -> Self {
        Self {
            think: CallbackTable::new(),
            reached: CallbackTable::new(),
            blocked: CallbackTable::new(),
            touch: CallbackTable::new(),
            use_callbacks: CallbackTable::new(),
            pain: CallbackTable::new(),
            die: CallbackTable::new(),
        }
    }
}

impl Default for Q3CallbackCatalog {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// numeric.ts: game numerics (donor `src/core/game-numeric.ts`, bg_lib.c)
// ---------------------------------------------------------------------------

/// Float scan result (`GameFloatScan`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GameFloatScan {
    /// Value.
    pub value: f32,
    /// Next offset.
    pub next_offset: usize,
}

/// Byte-string number cursor (`NumberInput`).
struct NumberInput {
    bytes: Vec<u8>,
    offset: usize,
}

impl NumberInput {
    fn new(text: &str, offset: usize) -> Result<Self, Q3GameError> {
        if offset > text.len() {
            return Err(range("Game number cursor is outside its backing string"));
        }
        let bytes = latin1_bytes(text).map_err(|_| range("Game numbers require byte characters"))?;
        if offset > bytes.len() {
            return Err(range("Game number cursor is outside its backing string"));
        }
        Ok(Self { bytes, offset })
    }

    fn byte(&self) -> Result<i32, Q3GameError> {
        if self.offset > self.bytes.len() {
            return Err(range("Game number scan reads beyond its backing string"));
        }
        if self.offset == self.bytes.len() {
            return Ok(0);
        }
        let byte = self.bytes[self.offset];
        Ok(if byte < 128 {
            i32::from(byte)
        } else {
            i32::from(byte) - 256
        })
    }

    fn take(&mut self) -> Result<i32, Q3GameError> {
        let byte = self.byte()?;
        self.offset += 1;
        Ok(byte)
    }

    fn skip_whitespace(&mut self) -> Result<(), Q3GameError> {
        while self.byte()? <= 32 && self.byte()? != 0 {
            self.offset += 1;
        }
        Ok(())
    }

    fn sign(&mut self) -> Result<i32, Q3GameError> {
        let byte = self.byte()?;
        if byte != 43 && byte != 45 {
            return Ok(1);
        }
        self.offset += 1;
        Ok(if byte == 45 { -1 } else { 1 })
    }
}

fn read_float(input: &mut NumberInput, scan: bool) -> Result<f32, Q3GameError> {
    input.skip_whitespace()?;
    if input.byte()? == 0 {
        return Ok(0.0);
    }
    let sign = input.sign()? as f32;
    let mut value = 0.0f32;
    let mut character = if scan { 48 } else { input.byte()? };
    if input.byte()? != 46 {
        loop {
            character = input.take()?;
            if !(48..=57).contains(&character) {
                break;
            }
            value = value * 10.0 + (character - 48) as f32;
        }
    } else if !scan {
        input.offset += 1;
    }
    if character == 46 {
        let mut fraction = 0.1f32;
        loop {
            character = input.take()?;
            if !(48..=57).contains(&character) {
                break;
            }
            value += (character - 48) as f32 * fraction;
            fraction *= 0.1;
        }
    }
    Ok(value * sign)
}

/// bg_lib atof (`gameAtof`).
pub fn game_atof(text: &str) -> Result<f32, Q3GameError> {
    read_float(&mut NumberInput::new(text, 0)?, false)
}

/// bg_lib atoi (`gameAtoi`).
pub fn game_atoi(text: &str) -> Result<i32, Q3GameError> {
    let mut input = NumberInput::new(text, 0)?;
    input.skip_whitespace()?;
    if input.byte()? == 0 {
        return Ok(0);
    }
    let sign = input.sign()?;
    let mut value = 0i32;
    loop {
        let character = input.take()?;
        if !(48..=57).contains(&character) {
            break;
        }
        value = value.wrapping_mul(10).wrapping_add(character - 48);
    }
    Ok(value.wrapping_mul(sign))
}

/// bg_lib float scan (`scanGameFloat`).
pub fn scan_game_float(text: &str, offset: usize) -> Result<GameFloatScan, Q3GameError> {
    let mut input = NumberInput::new(text, offset)?;
    let value = read_float(&mut input, true)?;
    Ok(GameFloatScan {
        value,
        next_offset: input.offset,
    })
}

/// QVM `sscanf("%f %f %f")` (`scanGameVector`).
pub fn scan_game_vector(text: &str) -> Result<Vec3, Q3GameError> {
    let mut input = NumberInput::new(text, 0)?;
    let x = read_float(&mut input, true)?;
    let y = read_float(&mut input, true)?;
    let z = read_float(&mut input, true)?;
    Ok(vec3(x, y, z))
}

/// Instance-owned bg_lib rand/srand and game random/crandom (`GameRandom`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameRandom {
    seed: i32,
}

impl GameRandom {
    /// New generator (`constructor`).
    pub fn new(seed: i32) -> Self {
        Self {
            seed: seed & 0x7fff_ffff | (seed & i32::MIN),
        }
    }

    /// Current seed.
    #[must_use]
    pub fn seed(&self) -> i32 {
        self.seed
    }

    /// Reset the seed (`reset`).
    pub fn reset(&mut self, seed: i32) {
        self.seed = seed;
    }

    /// bg_lib rand (`rand`).
    pub fn rand(&mut self) -> i32 {
        self.seed = q_rand(self.seed);
        self.seed & 0x7fff
    }

    /// Game random in `[0, 1]` (`random`).
    pub fn random(&mut self) -> f32 {
        self.rand() as f32 / f32::from(0x7fff_i16)
    }

    /// Game random in `[-1, 1]` (`crandom`).
    pub fn crandom(&mut self) -> f32 {
        2.0 * (self.random() - 0.5)
    }
}

impl Default for GameRandom {
    fn default() -> Self {
        Self::new(0)
    }
}

// ---------------------------------------------------------------------------
// Format mirror (donor `src/content/q3/base/game/format.ts`, bg_lib.c)
// ---------------------------------------------------------------------------

/// Game format argument (`GameFormatArgument`).
#[derive(Debug, Clone, PartialEq)]
pub enum GameFormatArg {
    /// Integer (`%d`/`%i`/`%c`-style).
    Int(i32),
    /// Float (`%f`).
    Float(f32),
    /// String (`%s`).
    Text(Option<String>),
}

/// Big format buffer (`BIG_BUFFER_BYTES`).
pub const GAME_FORMAT_BIG_BUFFER: usize = 32_000;
const FORMAT_LADJUST: i32 = 0x04;
const FORMAT_ZEROPAD: i32 = 0x80;

struct FormatOutput {
    value: Vec<u8>,
}

impl FormatOutput {
    fn reserve(&self, bytes: usize) -> Result<(), Q3GameError> {
        if self.value.len() + bytes >= GAME_FORMAT_BIG_BUFFER {
            return Err(range("game format exceeds the 32000-byte Com_sprintf buffer"));
        }
        Ok(())
    }

    fn append_byte(&mut self, byte: i32) -> Result<(), Q3GameError> {
        self.reserve(1)?;
        self.value.push((byte & 255) as u8);
        Ok(())
    }

    fn append_bytes(&mut self, value: &[u8], length: usize) -> Result<(), Q3GameError> {
        self.reserve(length)?;
        self.value.extend_from_slice(&value[..length.min(value.len())]);
        Ok(())
    }

    fn append_repeated(&mut self, byte: i32, count: usize) -> Result<(), Q3GameError> {
        self.reserve(count)?;
        self.value.extend(std::iter::repeat((byte & 255) as u8).take(count));
        Ok(())
    }

    fn finish(&self, max_bytes: usize) -> String {
        let end = self
            .value
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(self.value.len());
        let visible = &self.value[..end.min(max_bytes.saturating_sub(1))];
        latin1_string(visible)
    }
}

fn format_byte_length(value: &[u8], limit: usize) -> usize {
    value
        .iter()
        .take(limit.min(value.len()))
        .take_while(|byte| **byte != 0)
        .count()
}

fn format_argument(args: &[GameFormatArg], index: usize, specifier: &str) -> Result<GameFormatArg, Q3GameError> {
    args.get(index)
        .cloned()
        .ok_or_else(|| range(format!("missing argument {index} for %{specifier}")))
}

fn format_integer(value: &GameFormatArg, index: usize, specifier: &str) -> Result<i32, Q3GameError> {
    match value {
        GameFormatArg::Int(value) => Ok(*value),
        _ => Err(failure(format!("argument {index} for %{specifier} must be a number"))),
    }
}

fn format_float(value: &GameFormatArg, index: usize) -> Result<f32, Q3GameError> {
    match value {
        GameFormatArg::Float(value) => {
            if !value.is_finite() || value.abs() > 2_147_483_647.0 {
                return Err(range(format!(
                    "argument {index} for %f is outside the source's safe int-cast range"
                )));
            }
            Ok(*value)
        }
        _ => Err(failure(format!("argument {index} for %f must be a number"))),
    }
}

fn reversed_integer_bytes(value: i32) -> Vec<i32> {
    let signed = value;
    let mut remaining = if value < 0 { value.wrapping_neg() } else { value };
    let mut bytes = Vec::new();
    loop {
        bytes.push(48 + remaining % 10);
        remaining /= 10;
        if remaining == 0 {
            break;
        }
    }
    if signed < 0 {
        bytes.push(45);
    }
    bytes
}

fn add_int(output: &mut FormatOutput, value: i32, width: i32, flags: i32) -> Result<(), Q3GameError> {
    let reversed = reversed_integer_bytes(value);
    if flags & FORMAT_LADJUST == 0 {
        let padding = (width as i64 - reversed.len() as i64).max(0) as usize;
        output.append_repeated(if flags & FORMAT_ZEROPAD != 0 { 48 } else { 32 }, padding)?;
    }
    for index in (0..reversed.len()).rev() {
        output.append_byte(reversed[index])?;
    }
    if flags & FORMAT_LADJUST != 0 {
        let remaining = width.wrapping_sub(reversed.len() as i32);
        if remaining < 0 {
            return Err(range(
                "left-adjusted integer width would enter the source's negative padding loop",
            ));
        }
        output.append_repeated(if flags & FORMAT_ZEROPAD != 0 { 48 } else { 32 }, remaining as usize)?;
    }
    Ok(())
}

fn add_float(output: &mut FormatOutput, value: f32, width: i32, precision: i32) -> Result<(), Q3GameError> {
    let signed = value;
    let mut remaining = if value < 0.0 { -value } else { value };
    #[allow(clippy::cast_possible_wrap)]
    let integer = remaining.trunc() as i64 as i32;
    let mut reversed = reversed_integer_bytes(integer);
    if signed < 0.0 {
        reversed.push(45);
    }
    let padding = (width as i64 - reversed.len() as i64).max(0) as usize;
    output.append_repeated(32, padding)?;
    for index in (0..reversed.len()).rev() {
        output.append_byte(reversed[index])?;
    }
    let digits = if precision < 0 { 6 } else { precision };
    if digits > 32 {
        return Err(range("float precision would overflow AddFloat's 32-byte digit buffer"));
    }
    if digits == 0 {
        return Ok(());
    }
    output.append_byte(46)?;
    for _ in 0..digits {
        remaining -= remaining.trunc();
        remaining *= 10.0;
        output.append_byte(48 + remaining.trunc() as i32 % 10)?;
    }
    Ok(())
}

fn add_string(output: &mut FormatOutput, value: Option<&str>, width: i32, precision: i32) -> Result<(), Q3GameError> {
    let text = value.unwrap_or("(null)");
    let bytes = latin1_bytes(text).map_err(|_| range("game format strings must contain byte-valued code units"))?;
    let effective = if value.is_none() { -1 } else { precision };
    let length = format_byte_length(&bytes, if effective < 0 { bytes.len() } else { effective as usize });
    output.append_bytes(&bytes, length)?;
    let padding = width.wrapping_sub(length as i32);
    if padding > 0 {
        output.append_repeated(32, padding as usize)?;
    }
    Ok(())
}

/// Format byte strings with QVM bg_lib.c rules (`gameFormat`).
pub fn game_format_sized(format: &str, args: &[GameFormatArg], max_bytes: usize) -> Result<String, Q3GameError> {
    if max_bytes < 1 {
        return Err(range(
            "game format destination capacity must be a positive safe integer",
        ));
    }
    let bytes: Vec<u8> = format
        .chars()
        .map(|ch| {
            let code = ch as u32;
            if code > 255 {
                return Err(range("game format strings must contain byte-valued code units"));
            }
            Ok(code as u8)
        })
        .collect::<Result<_, _>>()?;
    let byte_at = |index: usize| -> Option<u8> {
        let byte = *bytes.get(index)?;
        if byte == 0 {
            None
        } else {
            Some(byte)
        }
    };
    let mut output = FormatOutput { value: Vec::new() };
    let mut cursor = 0usize;
    let mut argument_index = 0usize;
    loop {
        let Some(literal) = byte_at(cursor) else { break };
        if literal != 37 {
            output.append_byte(i32::from(literal))?;
            cursor += 1;
            continue;
        }
        cursor += 1;
        let mut flags = 0i32;
        let mut width = 0i32;
        let mut precision = -1i32;
        loop {
            let Some(specifier) = byte_at(cursor) else {
                return Err(range("unterminated game format specifier"));
            };
            cursor += 1;
            if specifier == 45 {
                flags |= FORMAT_LADJUST;
                continue;
            }
            if specifier == 46 {
                let mut parsed = 0i32;
                loop {
                    let Some(digit) = byte_at(cursor) else { break };
                    if !(48..=57).contains(&digit) {
                        break;
                    }
                    parsed = parsed.wrapping_mul(10).wrapping_add(i32::from(digit) - 48);
                    cursor += 1;
                }
                precision = if parsed < 0 { -1 } else { parsed };
                continue;
            }
            if specifier == 48 {
                flags |= FORMAT_ZEROPAD;
                continue;
            }
            if (49..=57).contains(&specifier) {
                let mut parsed = 0i32;
                let mut digit = specifier;
                while (48..=57).contains(&digit) {
                    parsed = parsed.wrapping_mul(10).wrapping_add(i32::from(digit) - 48);
                    let Some(next) = byte_at(cursor) else {
                        return Err(range("unterminated game format specifier"));
                    };
                    cursor += 1;
                    digit = next;
                }
                width = parsed;
                cursor -= 1;
                continue;
            }
            if specifier == 37 {
                output.append_byte(37)?;
                break;
            }
            let name = String::from(char::from(specifier));
            let argument = format_argument(args, argument_index, &name)?;
            if specifier == 100 || specifier == 105 {
                add_int(
                    &mut output,
                    format_integer(&argument, argument_index, &name)?,
                    width,
                    flags,
                )?;
            } else if specifier == 102 {
                add_float(&mut output, format_float(&argument, argument_index)?, width, precision)?;
            } else if specifier == 115 {
                match &argument {
                    GameFormatArg::Text(value) => add_string(&mut output, value.as_deref(), width, precision)?,
                    _ => {
                        return Err(failure(format!(
                            "argument {argument_index} for %s must be a string or null"
                        )))
                    }
                }
            } else {
                output.append_byte(format_integer(&argument, argument_index, &name)?)?;
            }
            argument_index += 1;
            break;
        }
    }
    Ok(output.finish(max_bytes))
}

/// Format with the default 32000-byte destination (`gameFormat` default).
pub fn game_format(format: &str, args: &[GameFormatArg]) -> Result<String, Q3GameError> {
    game_format_sized(format, args, GAME_FORMAT_BIG_BUFFER)
}

// ---------------------------------------------------------------------------
// Direction-byte mirror (donor `src/content/q3/base/shared/direction-byte.ts`)
// ---------------------------------------------------------------------------

/// Vertex-normal table (`BYTE_DIRECTIONS`).
const BYTE_DIRECTIONS: [[f32; 3]; 162] = [
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

/// Best-fit direction byte (`directionToByte`).
#[must_use]
pub fn direction_to_byte(value: Option<Vec3>) -> usize {
    let Some(value) = value else { return 0 };
    let mut best_dot = 0.0f32;
    let mut best = 0usize;
    for (index, direction) in BYTE_DIRECTIONS.iter().enumerate() {
        let dot = dot3(value, vec3(direction[0], direction[1], direction[2]));
        if dot > best_dot {
            best_dot = dot;
            best = index;
        }
    }
    best
}

/// Direction for a byte (`byteToDirection`).
#[must_use]
pub fn byte_to_direction(byte: i32) -> Vec3 {
    if byte < 0 || byte as usize >= BYTE_DIRECTIONS.len() {
        return vec3(0.0, 0.0, 0.0);
    }
    let direction = BYTE_DIRECTIONS[byte as usize];
    vec3(direction[0], direction[1], direction[2])
}

// ---------------------------------------------------------------------------
// Memory mirror (donor `src/content/q3/base/game/memory.ts`, g_mem.c)
// ---------------------------------------------------------------------------

/// Game memory pool size (`GAME_MEMORY_BYTES`).
pub const GAME_MEMORY_BYTES: usize = 256 * 1024;

/// Pool allocation handle (`GameMemoryAllocation`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameMemoryAllocation {
    offset: usize,
    length: usize,
}

/// Module-owned static storage (`GameMemory`).
pub struct GameMemory {
    pool: Vec<u8>,
    alloc_point: usize,
    debug_integer: Rc<dyn Fn() -> i32>,
    print: Rc<dyn Fn(&str)>,
}

impl GameMemory {
    /// New memory with debug/print hooks.
    #[must_use]
    pub fn new(debug_integer: Rc<dyn Fn() -> i32>, print: Rc<dyn Fn(&str)>) -> Self {
        Self {
            pool: vec![0; GAME_MEMORY_BYTES],
            alloc_point: 0,
            debug_integer,
            print,
        }
    }

    /// Allocated bytes.
    #[must_use]
    pub fn allocated_bytes(&self) -> usize {
        self.alloc_point
    }

    /// Allocate pool storage (`G_Alloc`).
    pub fn allocate(&mut self, size: usize) -> Result<GameMemoryAllocation, Q3GameError> {
        if size > 0x7fff_ffff {
            return Err(range("G_Alloc requires a nonnegative source int size"));
        }
        let aligned = size.wrapping_add(31) & !31;
        if (self.debug_integer)() != 0 {
            let left = GAME_MEMORY_BYTES.wrapping_sub(self.alloc_point).wrapping_sub(aligned) as i32;
            (self.print)(&format!("G_Alloc of {size} bytes ({left} left)\n"));
        }
        if self.alloc_point.saturating_add(size) > GAME_MEMORY_BYTES {
            return Err(drop_error(format!("G_Alloc: failed on allocation of {size} bytes\n")));
        }
        let allocation = GameMemoryAllocation {
            offset: self.alloc_point,
            length: size,
        };
        self.alloc_point = self.alloc_point.wrapping_add(aligned);
        Ok(allocation)
    }

    /// Mutable allocation bytes.
    pub fn alloc_bytes_mut(&mut self, allocation: &GameMemoryAllocation) -> &mut [u8] {
        let len = self.pool.len();
        let end = allocation.offset.saturating_add(allocation.length).min(len);
        &mut self.pool[allocation.offset.min(len)..end]
    }

    /// Read a NUL-terminated string from the allocation offset (`readString`).
    pub fn read_string(&self, allocation: &GameMemoryAllocation) -> Result<String, Q3GameError> {
        if allocation.offset > self.pool.len() {
            return Err(range("Game string reads beyond the source memory pool"));
        }
        let remaining = &self.pool[allocation.offset..];
        let Some(end) = remaining.iter().position(|byte| *byte == 0) else {
            return Err(range("Game string reads beyond the source memory pool"));
        };
        Ok(latin1_string(&remaining[..end]))
    }

    /// Write a NUL-terminated string into the allocation (`writeString`).
    pub fn write_string(&mut self, allocation: &GameMemoryAllocation, value: &str) -> Result<(), Q3GameError> {
        let bytes = latin1_bytes(value).map_err(|_| range("Game strings require non-NUL source bytes"))?;
        if bytes.contains(&0) {
            return Err(range("Game strings require non-NUL source bytes"));
        }
        if bytes.len() + 1 > allocation.length {
            return Err(range("Game string exceeds its source allocation"));
        }
        let end = allocation.offset + bytes.len() + 1;
        if end > self.pool.len() {
            return Err(range("Game string exceeds its source allocation"));
        }
        self.pool[allocation.offset..allocation.offset + bytes.len()].copy_from_slice(&bytes);
        self.pool[allocation.offset + bytes.len()] = 0;
        Ok(())
    }

    /// Rewind the pool (`G_InitMemory`).
    pub fn initialize(&mut self) {
        self.alloc_point = 0;
    }

    /// Print pool status (`status`).
    pub fn status(&self) {
        (self.print)(&format!(
            "Game memory status: {} out of {GAME_MEMORY_BYTES} bytes allocated\n",
            self.alloc_point
        ));
    }
}

// ---------------------------------------------------------------------------
// Driver and subsystem mirrors
// (donor `src/content/q3/base/game/entities.ts`,
// `src/content/q3/base/world.ts`,
// `src/content/q3/base/game/combat.ts`,
// `src/content/q3/base/records.ts`,
// `src/content/q3/base/game/missile.ts` (`MissileRuntime`),
// `src/content/q3/base/game/hitscan.ts`,
// `src/content/q3/base/bot-debug.ts`,
// `src/world/actors/registry.ts`)
// ---------------------------------------------------------------------------

/// Entity pool surface used by this module (`EntityPool`).
pub trait EntityPool {
    /// Product.
    fn product(&self) -> Q3Product;
    /// Active entity count.
    fn num_entities(&self) -> usize;
    /// Maximum clients.
    fn max_clients(&self) -> usize;
    /// Borrow an entity.
    fn entity(&self, slot: usize) -> Option<&GameEntity>;
    /// Mutably borrow an entity.
    fn entity_mut(&mut self, slot: usize) -> Option<&mut GameEntity>;
    /// Borrow a client.
    fn client(&self, slot: usize) -> Option<&GameClient>;
    /// Mutably borrow a client.
    fn client_mut(&mut self, slot: usize) -> Option<&mut GameClient>;
    /// Spawn an entity, returning its slot (`spawn`).
    fn spawn_entity(&mut self) -> Result<usize, Q3GameError>;
    /// Free an entity (`free`).
    fn free_entity(&mut self, slot: usize);
    /// Allocate a temporary event entity (`tempEntity`).
    fn temp_entity(&mut self, origin: Vec3, event: Q3EntityEvent) -> usize;
    /// Add an entity event (`addEvent`).
    fn add_event(&mut self, slot: usize, event: Q3EntityEvent, parm: i32);
    /// Set nextthink and schedule it (`nextthink` setter + `binding.schedule`).
    fn set_nextthink(&mut self, slot: usize, time: i32);
    /// Borrow the callback catalog.
    fn callbacks(&self) -> &Q3CallbackCatalog;
    /// Mutably borrow the callback catalog.
    fn callbacks_mut(&mut self) -> &mut Q3CallbackCatalog;
    /// Borrow the ranking reports.
    fn rankings(&self) -> &Q3RankingReports;
    /// Restore entity/client counts (`restoreCounts`).
    fn restore_counts(&mut self, num_entities: usize, max_clients: usize);
}

/// Server world surface used by this module (`ServerWorld`).
pub trait ServerWorldOps {
    /// Link an entity (`link`).
    fn link(&mut self, slot: usize);
    /// Unlink an entity (`unlink`).
    fn unlink(&mut self, slot: usize);
    /// Link state (`linkState`).
    fn link_state(&self, slot: usize) -> Option<Q3LinkState>;
    /// Entities in bounds (`areaEntities`).
    fn area_entities(&self, bounds: &Bounds, maximum: usize) -> Vec<usize>;
}

/// Actor spatial queries used by this module (`ActorSpatialQueries`).
pub trait SpatialQueries {
    /// Actors in bounds (`areaActors`).
    fn area_actors(&self, bounds: &Bounds, maximum: usize) -> Vec<ActorId>;
    /// Trace an actor (`traceActor`).
    fn trace_actor(&self, query: &Q3TraceQuery) -> Q3TraceResult;
}

/// Shared-actor combat state for accuracy subjects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActorCombatState {
    /// Can take damage.
    pub can_take_damage: bool,
    /// Health.
    pub health: i32,
    /// Team number, when teamed.
    pub team: Option<i32>,
}

/// Combat surface used by this module (`CombatContext` + `damage`).
pub trait CombatContext {
    /// Product.
    fn product(&self) -> Q3Product;
    /// Game type number.
    fn game_type(&self) -> i32;
    /// Time.
    fn time(&self) -> i32;
    /// Apply damage (`damage`); normalizes `direction` in place.
    #[allow(clippy::too_many_arguments)]
    fn damage(
        &mut self,
        target: &Participant,
        inflictor: Option<&Participant>,
        attacker: Option<&Participant>,
        direction: Option<&mut Vec3>,
        point: Option<Vec3>,
        amount: i32,
        flags: i32,
        method: i32,
    );
    /// Shared-actor combat state (`authority.read` projection).
    fn actor_combat_state(&self, actor: &ActorId) -> Option<ActorCombatState>;
}

/// Mover shared-actor body kind (`SharedMoverBody["kind"]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SharedBodyKind {
    /// Player.
    Player,
    /// Movable.
    Movable,
    /// Fixed.
    Fixed,
    /// Attached.
    Attached,
}

/// Shared mover body (`SharedMoverBody`).
#[derive(Debug, Clone, PartialEq)]
pub struct SharedMoverBody {
    /// Actor.
    pub actor: ActorId,
    /// Kind.
    pub kind: SharedBodyKind,
    /// State.
    pub state: Q3BodyState,
    /// Absolute bounds.
    pub absolute_bounds: Bounds,
    /// Clip mask.
    pub clip_mask: i32,
}

/// Mover actor access (`MoverActorAccess` from `mover.ts`).
pub trait MoverActorAccess {
    /// Native slot for an actor (`native`).
    fn native_slot(&self, actor: &ActorId) -> Option<usize>;
    /// Participant for an actor (`participant`).
    fn participant(&self, actor: &ActorId) -> Participant;
    /// Observe a shared body (`observe`).
    fn observe(&self, actor: &ActorId) -> Option<SharedMoverBody>;
    /// Write origin and ground (`write`).
    fn write(&mut self, actor: &ActorId, origin: Vec3, ground: Option<ActorId>);
    /// Link an actor (`link`).
    fn link_actor(&mut self, actor: &ActorId);
    /// Release an actor (`release`).
    fn release(&mut self, actor: &ActorId);
}

/// Debug polygon allocation (`BotDebugPolygons`).
pub trait DebugPolygons {
    /// Create a polygon (`create`).
    fn create(&mut self, color: i32, count: usize, points: &[Vec3]) -> i32;
}

/// Missile launcher surface (`MissileRuntime` fire methods).
pub trait MissileLauncher {
    /// Fire a grenade.
    fn fire_grenade(&mut self, driver: &mut dyn Q3Driver, entity: usize, muzzle: Vec3, direction: Vec3) -> usize;
    /// Fire a rocket.
    fn fire_rocket(&mut self, driver: &mut dyn Q3Driver, entity: usize, muzzle: Vec3, direction: Vec3) -> usize;
    /// Fire plasma.
    fn fire_plasma(&mut self, driver: &mut dyn Q3Driver, entity: usize, muzzle: Vec3, direction: Vec3) -> usize;
    /// Fire the BFG.
    fn fire_bfg(&mut self, driver: &mut dyn Q3Driver, entity: usize, muzzle: Vec3, direction: Vec3) -> usize;
    /// Fire the grapple.
    fn fire_grapple(&mut self, driver: &mut dyn Q3Driver, entity: usize, muzzle: Vec3, direction: Vec3) -> usize;
    /// Fire a nail.
    fn fire_nail(
        &mut self,
        driver: &mut dyn Q3Driver,
        entity: usize,
        muzzle: Vec3,
        forward: Vec3,
        right: Vec3,
        up: Vec3,
    ) -> usize;
    /// Fire a proximity mine.
    fn fire_prox(&mut self, driver: &mut dyn Q3Driver, entity: usize, muzzle: Vec3, direction: Vec3) -> usize;
}

/// Accuracy subject (`Q3AccuracySubject`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccuracySubject {
    /// Actor.
    pub actor: ActorId,
    /// Damageable.
    pub damageable: bool,
    /// Player.
    pub player: bool,
    /// Health.
    pub health: i32,
    /// Team number, when teamed.
    pub team: Option<i32>,
}

/// Bullet attack (`Q3BulletAttack`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BulletAttack {
    /// Forward.
    pub forward: Vec3,
    /// Right.
    pub right: Vec3,
    /// Up.
    pub up: Vec3,
    /// Muzzle.
    pub muzzle: Vec3,
    /// Quad factor.
    pub quad: f32,
}

/// Bullet target (`Q3BulletTarget`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BulletTarget {
    /// Damageable.
    pub damageable: bool,
    /// Player.
    pub player: bool,
    /// Accuracy eligible.
    pub accuracy_eligible: bool,
    /// Invulnerable.
    pub invulnerable: bool,
}

/// Bullet hit event (`Q3BulletHost["emit"]` payload).
#[derive(Debug, Clone, PartialEq)]
pub struct BulletHit {
    /// Point.
    pub point: Vec3,
    /// Normal.
    pub normal: Vec3,
    /// Target.
    pub target: Option<ActorId>,
    /// Flesh.
    pub flesh: bool,
}

/// Contact event (`Q3ContactEvent`).
#[derive(Debug, Clone, PartialEq)]
pub enum ContactEvent {
    /// Hit.
    Hit {
        /// Point.
        point: Vec3,
        /// Normal.
        normal: Vec3,
        /// Target.
        target: ActorId,
    },
    /// Miss.
    Miss {
        /// Point.
        point: Vec3,
        /// Normal.
        normal: Vec3,
    },
    /// Lightning reflection.
    LightningReflection {
        /// Start.
        start: Vec3,
        /// End.
        end: Vec3,
    },
    /// Gauntlet quad.
    GauntletQuad,
}

/// Rail shot trail (`Q3RailTrail`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RailShot {
    /// Start.
    pub start: Vec3,
    /// End.
    pub end: Vec3,
    /// Impact normal, when the shot hit.
    pub impact_normal: Option<Vec3>,
}

/// Rail statistics (`Q3RailStatistics`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RailStatistics {
    /// Streak.
    pub streak: i32,
    /// Hits.
    pub hits: i32,
    /// Impressive count.
    pub impressive_count: i32,
    /// Reward until.
    pub reward_until: i32,
}

/// Rail statistics outcome (`q3RailStatistics` return).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RailStatisticsOutcome {
    /// Streak.
    pub streak: i32,
    /// Hits.
    pub hits: i32,
    /// Impressive count.
    pub impressive_count: i32,
    /// Reward until.
    pub reward_until: i32,
    /// Awarded.
    pub awarded: bool,
}

/// Bullet host services (`Q3BulletHost` weapon side).
pub trait BulletHost {
    /// Trace.
    fn trace_hit(&mut self, driver: &mut dyn Q3Driver, start: Vec3, end: Vec3, pass: Option<&ActorId>)
        -> Q3TraceResult;
    /// Resolve a target.
    fn hit_target(&mut self, driver: &mut dyn Q3Driver, actor: &ActorId) -> Option<BulletTarget>;
    /// Impact hook (`impact?`, default no-op).
    fn impact(&mut self, _driver: &mut dyn Q3Driver, _point: Vec3) {}
    /// Emit a hit.
    fn emit_hit(&mut self, driver: &mut dyn Q3Driver, hit: &BulletHit);
    /// Apply damage.
    fn apply_damage(&mut self, driver: &mut dyn Q3Driver, target: &ActorId, direction: Vec3, point: Vec3, amount: i32);
    /// Credit an accuracy hit.
    fn credit_accuracy(&mut self, driver: &mut dyn Q3Driver);
    /// Invulnerability impact (missionpack; default miss).
    fn invulnerability_impact(
        &mut self,
        driver: &mut dyn Q3Driver,
        target: &ActorId,
        direction: Vec3,
        point: Vec3,
    ) -> InvulnerabilityImpact {
        let _ = (driver, target, direction, point);
        InvulnerabilityImpact::Miss
    }
}

/// Contact host services (`Q3ContactHost` weapon side).
pub trait ContactHost {
    /// Trace.
    fn trace_hit(&mut self, driver: &mut dyn Q3Driver, start: Vec3, end: Vec3, pass: Option<&ActorId>)
        -> Q3TraceResult;
    /// Resolve a target.
    fn hit_target(&mut self, driver: &mut dyn Q3Driver, actor: &ActorId) -> Option<BulletTarget>;
    /// Impact hook (`impact?`, default no-op).
    fn impact(&mut self, _driver: &mut dyn Q3Driver, _point: Vec3) {}
    /// Emit an event.
    fn emit_contact(&mut self, driver: &mut dyn Q3Driver, event: &ContactEvent);
    /// Apply damage.
    fn apply_damage(&mut self, driver: &mut dyn Q3Driver, target: &ActorId, direction: Vec3, point: Vec3, amount: i32);
    /// Credit an accuracy hit.
    fn credit_accuracy(&mut self, driver: &mut dyn Q3Driver);
    /// Invulnerability impact (missionpack; default miss).
    fn invulnerability_impact(
        &mut self,
        driver: &mut dyn Q3Driver,
        target: &ActorId,
        direction: Vec3,
        point: Vec3,
    ) -> InvulnerabilityImpact {
        let _ = (driver, target, direction, point);
        InvulnerabilityImpact::Miss
    }
}

/// Shotgun host services (`Q3ShotgunHost` weapon side).
pub trait ShotgunHost: ContactHost {
    /// Begin a shotgun event, returning its temp-entity slot.
    fn begin_shotgun(&mut self, driver: &mut dyn Q3Driver, muzzle: Vec3, direction: Vec3) -> usize;
    /// Record a pellet seed on the event entity.
    fn emit_shotgun_seed(&mut self, driver: &mut dyn Q3Driver, event_slot: usize, seed: i32);
}

/// Rail host services (`Q3RailHost` weapon side).
pub trait RailHost {
    /// Trace.
    fn trace_hit(&mut self, driver: &mut dyn Q3Driver, start: Vec3, end: Vec3, pass: Option<&ActorId>)
        -> Q3TraceResult;
    /// Resolve a target.
    fn hit_target(&mut self, driver: &mut dyn Q3Driver, actor: &ActorId) -> Option<BulletTarget>;
    /// Impact hook (`impact?`, default no-op).
    fn impact(&mut self, _driver: &mut dyn Q3Driver, _point: Vec3) {}
    /// Whether the shooter is still the same native entity (`alive`).
    fn is_alive(&mut self, driver: &mut dyn Q3Driver) -> bool;
    /// Unlink an actor, returning it when a restore is required (`unlink`).
    fn unlink_actor(&mut self, driver: &mut dyn Q3Driver, actor: &ActorId) -> Option<ActorId>;
    /// Restore an unlinked actor.
    fn restore_actor(&mut self, driver: &mut dyn Q3Driver, actor: &ActorId);
    /// Emit a trail (`trail`).
    fn emit_trail(&mut self, driver: &mut dyn Q3Driver, shot: &RailShot);
    /// Invulnerability impact (missionpack; default miss).
    fn invulnerability_impact(
        &mut self,
        driver: &mut dyn Q3Driver,
        target: &ActorId,
        direction: Vec3,
        point: Vec3,
    ) -> InvulnerabilityImpact {
        let _ = (driver, target, direction, point);
        InvulnerabilityImpact::Miss
    }
}

/// Universal game driver: the subsystem surface behind every runtime in this
/// module. It dissolves the donor host/context objects (`MoverServices`,
/// `CombatContext` carriers, `EntityPoolOptions`, `TargetRuntime`, `TriggerHost`,
/// `WeaponHost`, `PersonalPortalHost`, `TeleportContext`, `ItemLifecycleContext`,
/// `LaunchItemContext`, `DropItemContext`) into one object-safe trait so native
/// callbacks can reach every service through a single handle.
pub trait Q3Driver {
    /// Entity pool.
    fn pool(&mut self) -> &mut dyn EntityPool;
    /// Server world.
    fn world(&mut self) -> &mut dyn ServerWorldOps;
    /// Spatial queries.
    fn spatial(&mut self) -> &mut dyn SpatialQueries;
    /// Combat context.
    fn combat(&mut self) -> &mut dyn CombatContext;
    /// Utility scratch rings.
    fn scratch(&mut self) -> &mut GameUtilityScratch;
    /// Mover actor access.
    fn mover_actors(&mut self) -> &mut dyn MoverActorAccess;
    /// Warn (`warn`).
    fn warn(&mut self, message: &str);
    /// Log (`log`).
    fn log(&mut self, message: &str);
    /// Sound index (`soundIndex`).
    fn sound_index(&mut self, path: &str) -> i32;
    /// Model index (`modelIndex`).
    fn model_index(&mut self, name: Option<&str>) -> i32;
    /// Gravity (`gravity()`).
    fn gravity(&self) -> f32;
    /// Game rand (`rand`).
    fn game_rand(&mut self) -> i32;
    /// Game random (`random`).
    fn game_random(&mut self) -> f32;
    /// Game crandom (`crandom`).
    fn game_crandom(&mut self) -> f32;
    /// Remap a shader (`remapShader`).
    fn remap_shader(&mut self, old: &str, new: &str, time_seconds: f32);
    /// Set a configstring (`setConfigstring`).
    fn set_configstring(&mut self, index: i32, value: &str);
    /// Set a cvar (`setCvar`).
    fn set_cvar(&mut self, name: &str, value: &str);
    /// Send a server command (`sendServerCommand`).
    fn send_server_command(&mut self, client: i32, command: &str);
    /// Use targets (`useTargets`).
    fn use_targets(&mut self, slot: usize, activator: Option<Participant>);
    /// Adjust an area portal (`adjustAreaPortalState`).
    fn adjust_area_portal(&mut self, slot: usize, open: bool);
    /// Return a dropped flag (`returnDroppedFlag`).
    fn return_dropped_flag(&mut self, slot: usize);
    /// Return a team flag (`returnFlag`).
    fn return_flag(&mut self, team: Q3Team);
    /// Add score (`addScore`).
    fn add_score(&mut self, player: usize, origin: Vec3, points: i32);
    /// Explode a missile (`explodeMissile`).
    fn explode_missile(&mut self, slot: usize);
    /// Teleport a player (`teleportPlayer`).
    fn teleport_player(&mut self, player: usize, origin: Vec3, angles: Vec3);
    /// Whether map travel overrides item/portal routing (`mapTravel` present).
    fn map_travel_mode(&self) -> bool;
    /// Map-travel teleport (`mapTravel.teleport`).
    fn map_travel_teleport(&mut self, player: usize, origin: Vec3, angles: Vec3);
    /// Map-travel flag drop (`mapTravel.dropCarriedFlag`).
    fn map_travel_drop_flag(&mut self, player: usize);
    /// Touch an item (`touchItem`).
    fn touch_item(&mut self, item: usize, player: usize, contact: &TouchContact);
    /// Drop an item (`dropItem`).
    fn drop_item(&mut self, entity: usize, item: usize, angle: i32) -> usize;
    /// Dropped-flag think (`droppedFlagThink`).
    fn dropped_flag_think(&mut self, slot: usize);
    /// Check a dropped team item (`checkDroppedTeamItem`).
    fn check_dropped_team_item(&mut self, slot: usize);
    /// Whether an actor is live.
    fn actor_live(&self, actor: &ActorId) -> bool;
    /// Observed origin for a shared actor (`DamageParticipant.origin()`).
    fn actor_origin(&self, actor: &ActorId) -> Option<Vec3>;
    /// Participant for an actor (`actors.participant`).
    fn participant(&self, actor: &ActorId) -> Participant;
    /// Whether an actor is a player.
    fn actor_is_player(&self, actor: &ActorId) -> bool;
    /// Native slot for an actor.
    fn native_slot(&self, actor: &ActorId) -> Option<usize>;
    /// Emit an actor event.
    fn actor_event(&mut self, actor: &ActorId, event: Q3EntityEvent, parm: i32);
    /// Item table length.
    fn item_count(&self) -> usize;
    /// Item at an index (`itemAt`).
    fn item_at(&self, index: usize) -> Option<Q3ItemDefinition>;
    /// Find an item by pickup name (`findItem`).
    fn find_item(&self, pickup_name: &str) -> Option<usize>;
    /// Find an item for a powerup (`findItemForPowerup`).
    fn find_item_for_powerup(&self, powerup: i32) -> Option<usize>;
    /// Set a brush model with an immediate link (`setBrushModel`).
    fn set_brush_model(&mut self, slot: usize, model: Option<&str>);
    /// Fire a hitscan bullet (`q3BulletFire`).
    fn bullet_fire(
        &mut self,
        host: &mut dyn BulletHost,
        shooter: &ActorId,
        attack: &mut BulletAttack,
        spread: i32,
        amount: i32,
    );
    /// Run a gauntlet attack (`q3GauntletAttack`).
    fn gauntlet_attack(
        &mut self,
        host: &mut dyn ContactHost,
        shooter: &ActorId,
        attack: &mut BulletAttack,
        quad: bool,
    ) -> bool;
    /// Fire lightning (`q3LightningFire`).
    fn lightning_fire(&mut self, host: &mut dyn ContactHost, shooter: &ActorId, attack: &mut BulletAttack);
    /// Fire the shotgun (`q3ShotgunFire`).
    fn shotgun_fire(&mut self, host: &mut dyn ShotgunHost, shooter: &ActorId, attack: &mut BulletAttack);
    /// Fire the railgun (`q3RailFire`).
    fn rail_fire(&mut self, host: &mut dyn RailHost, shooter: &ActorId, attack: &mut BulletAttack) -> i32;
    /// Compute rail statistics (`q3RailStatistics`).
    fn rail_statistics(&mut self, state: &RailStatistics, hits: i32, time: i32) -> RailStatisticsOutcome;
}

/// Require a live entity slot (`owned` / `requireOwned` checks).
pub fn require_entity(pool: &dyn EntityPool, slot: usize) -> Result<(), Q3GameError> {
    if pool.entity(slot).is_none() {
        return Err(failure(format!(
            "entity {slot} does not belong to its entity pool or was replaced"
        )));
    }
    Ok(())
}

/// Store a fixed trajectory and current collision origin (`setOrigin`).
pub fn set_origin(entity: &mut GameEntity, origin: Vec3) {
    entity.s.pos = Q3Trajectory {
        trajectory_type: TrajectoryType::Stationary,
        time: 0,
        duration: 0,
        base: origin,
        delta: vec3(0.0, 0.0, 0.0),
    };
    entity.r.current_origin = origin;
}

/// Run a scheduled think (`runThink` + the record `runThink` thunk).
pub fn run_think(driver: &mut dyn Q3Driver, slot: usize, time: i32) -> Result<(), Q3GameError> {
    let nextthink = driver.pool().entity(slot).map(|entity| entity.nextthink).unwrap_or(0);
    if nextthink <= 0 || nextthink > time {
        return Ok(());
    }
    let think = driver.pool().entity(slot).and_then(|entity| entity.think.clone());
    driver.pool().set_nextthink(slot, 0);
    match think {
        Some(think) => {
            think(driver, slot);
            Ok(())
        }
        None => Err(failure("NULL ent->think")),
    }
}

/// Whether an entity rides a support actor (`rides`).
#[must_use]
pub fn rides(entity: &GameEntity, support: &ActorId) -> bool {
    entity.r.ground.as_ref().is_some_and(|ground| ground == support)
}

/// Clear an entity's support with the delayed-continuation marker (`loseGround`).
pub fn lose_ground(entity: &mut GameEntity) {
    entity.r.ground = None;
    entity.s.ground_entity_num = -1;
}

/// Snap a vector toward integers (`snapVector`).
#[must_use]
pub fn snap_vector(value: Vec3) -> Vec3 {
    vec3(
        qvm_float_to_int(value.x) as f32,
        qvm_float_to_int(value.y) as f32,
        qvm_float_to_int(value.z) as f32,
    )
}

/// Snap a vector toward a target (`snapVectorTowards`).
#[must_use]
pub fn snap_vector_towards(value: Vec3, toward: Vec3) -> Vec3 {
    let axis = |v: f32, to: f32| -> f32 { qvm_float_to_int(v).wrapping_add(if to <= v { 0 } else { 1 }) as f32 };
    vec3(
        axis(value.x, toward.x),
        axis(value.y, toward.y),
        axis(value.z, toward.z),
    )
}

/// Apply jump-pad velocity (`touchJumpPad`).
pub fn touch_jump_pad(state: &mut Q3PlayerState, jump_pad: &Q3EntityState) {
    if state.pm_type != Q3MoveType::Normal as i32 || state.powerups.get(Q3Powerup::Flight as usize) != 0 {
        return;
    }
    if state.jumppad_ent != jump_pad.number {
        let pitch = angle_normalize180(f64::from(vector_to_angles(jump_pad.origin2).x)).abs();
        state.add_event(Q3EntityEvent::JumpPad as i32, i32::from(pitch >= 45.0));
    }
    state.jumppad_ent = jump_pad.number;
    state.jumppad_frame = state.pmove_framecount;
    state.velocity = jump_pad.origin2;
}

/// Bounce a velocity off a plane (`q3BounceVelocity`).
#[must_use]
pub fn q3_bounce_velocity(velocity: Vec3, normal: Vec3, half: bool) -> Vec3 {
    let mut delta = add3(velocity, scale3(normal, -2.0 * dot3(velocity, normal)));
    if half {
        delta = scale3(delta, 0.65);
    }
    delta
}

/// Missile impact time (`q3MissileHitTime`).
#[must_use]
pub fn q3_missile_hit_time(previous: i32, time: i32, fraction: f32) -> i32 {
    let elapsed = time.wrapping_sub(previous) as f32;
    qvm_float_to_int(previous as f32 + elapsed * fraction)
}

/// Accuracy-hit test (`q3AccuracyHit`).
#[must_use]
pub fn q3_accuracy_hit(team_game: bool, target: &AccuracySubject, attacker: &AccuracySubject) -> bool {
    target.damageable
        && target.actor != attacker.actor
        && target.player
        && attacker.player
        && target.health > 0
        && (!team_game || target.team != attacker.team)
}

// ---------------------------------------------------------------------------
// utilities.ts: scratch rings, configstrings, target dispatch
// ---------------------------------------------------------------------------

/// Temporary vector with binary32-storing setters (`TemporaryVector`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TempVector {
    /// X.
    pub x: f32,
    /// Y.
    pub y: f32,
    /// Z.
    pub z: f32,
}

/// Fixed 32-byte vector string (`GameMemoryAllocation` ring cell).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3RingString {
    bytes: [u8; 32],
}

impl Q3RingString {
    /// Empty string.
    #[must_use]
    pub fn new() -> Self {
        Self { bytes: [0; 32] }
    }

    /// Write a NUL-terminated string (`writeString`).
    pub fn write_string(&mut self, value: &str) -> Result<(), Q3GameError> {
        let bytes = latin1_bytes(value).map_err(|_| range("Game strings require non-NUL source bytes"))?;
        if bytes.contains(&0) {
            return Err(range("Game strings require non-NUL source bytes"));
        }
        if bytes.len() + 1 > self.bytes.len() {
            return Err(range("Game string exceeds its source allocation"));
        }
        self.bytes.fill(0);
        self.bytes[..bytes.len()].copy_from_slice(&bytes);
        Ok(())
    }

    /// Read the NUL-terminated string (`readString`).
    #[must_use]
    pub fn read_string(&self) -> String {
        let end = self
            .bytes
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(self.bytes.len());
        latin1_string(&self.bytes[..end])
    }
}

impl Default for Q3RingString {
    fn default() -> Self {
        Self::new()
    }
}

/// Per-game scratch rings (`GameUtilityScratch`).
pub struct GameUtilityScratch {
    vectors: [TempVector; 8],
    strings: [Q3RingString; 8],
    vector_index: usize,
    string_index: usize,
    print: Rc<dyn Fn(&str)>,
}

impl GameUtilityScratch {
    /// New scratch with a print hook.
    #[must_use]
    pub fn new(print: Rc<dyn Fn(&str)>) -> Self {
        Self {
            vectors: [TempVector::default(); 8],
            strings: [
                Q3RingString::new(),
                Q3RingString::new(),
                Q3RingString::new(),
                Q3RingString::new(),
                Q3RingString::new(),
                Q3RingString::new(),
                Q3RingString::new(),
                Q3RingString::new(),
            ],
            vector_index: 0,
            string_index: 0,
            print,
        }
    }

    /// Borrow the next temporary vector (`tv`).
    pub fn tv(&mut self, x: f32, y: f32, z: f32) -> TempVector {
        let vector = TempVector { x, y, z };
        self.vectors[self.vector_index] = vector;
        self.vector_index = (self.vector_index + 1) & 7;
        vector
    }

    /// Format a vector into the next ring string (`vtos`).
    pub fn vtos(&mut self, vector: Vec3) -> Result<Q3RingString, Q3GameError> {
        let value = game_format(
            "(%i %i %i)",
            &[
                GameFormatArg::Int(qvm_float_to_int(vector.x)),
                GameFormatArg::Int(qvm_float_to_int(vector.y)),
                GameFormatArg::Int(qvm_float_to_int(vector.z)),
            ],
        )?;
        if value.len() >= 32 {
            (self.print)(&game_format(
                "Com_sprintf: overflow of %i in %i\n",
                &[GameFormatArg::Int(value.len() as i32), GameFormatArg::Int(32)],
            )?);
        }
        let mut string = Q3RingString::new();
        string.write_string(&value[..value.len().min(31)])?;
        self.strings[self.string_index] = string.clone();
        self.string_index = (self.string_index + 1) & 7;
        Ok(string)
    }
}

/// Submit a debug line as a four-point polygon (`DebugLine`).
pub fn debug_line(start: Vec3, end: Vec3, color: i32, polygons: &mut dyn DebugPolygons) -> i32 {
    let direction = normalize3(sub3(end, start));
    let up = vec3(0.0, 1.0_f32 - 1.0, 1.0);
    let dot = dot3(direction, up);
    let cross = if dot > 0.99 || dot < -0.99 {
        vec3(1.0, 0.0, 0.0)
    } else {
        normalize3(cross3(direction, up))
    };
    polygons.create(
        color,
        4,
        &[
            add3(start, scale3(cross, 2.0)),
            add3(start, scale3(cross, -2.0)),
            add3(end, scale3(cross, -2.0)),
            add3(end, scale3(cross, 2.0)),
        ],
    )
}

/// Configstring store (`ConfigStringStore`).
pub trait ConfigStringStore {
    /// Read a configstring.
    fn get(&self, index: usize) -> String;
    /// Write a configstring.
    fn set(&mut self, index: usize, value: &str);
}

/// Configstring registry (`ConfigStringRegistry`).
pub struct ConfigStringRegistry<S: ConfigStringStore> {
    store: S,
}

impl<S: ConfigStringStore> ConfigStringRegistry<S> {
    /// New registry over a store.
    #[must_use]
    pub fn new(store: S) -> Self {
        Self { store }
    }

    /// Find or create a configstring index (`G_FindConfigstringIndex`).
    pub fn find(
        &mut self,
        name: Option<&str>,
        start: usize,
        maximum: usize,
        create: bool,
    ) -> Result<usize, Q3GameError> {
        let Some(name) = name else { return Ok(0) };
        let text = byte_string(name)?;
        if text.is_empty() {
            return Ok(0);
        }
        if start.checked_add(maximum).is_none_or(|end| end > 1024) || maximum < 1 {
            return Err(range("Configstring range must fit MAX_CONFIGSTRINGS"));
        }
        let mut index = 1usize;
        while index < maximum {
            let value = byte_string(&self.store.get(start + index))?;
            let value = value[..value.len().min(1023)].to_string();
            if value.is_empty() {
                break;
            }
            if value == text {
                return Ok(index);
            }
            index += 1;
        }
        if !create {
            return Ok(0);
        }
        if index == maximum {
            return Err(failure("G_FindConfigstringIndex: overflow"));
        }
        self.store.set(start + index, &text);
        Ok(index)
    }

    /// Model index (`modelIndex`).
    pub fn model_index(&mut self, name: Option<&str>) -> Result<usize, Q3GameError> {
        self.find(name, 32, 256, true)
    }

    /// Sound index (`soundIndex`).
    pub fn sound_index(&mut self, name: Option<&str>) -> Result<usize, Q3GameError> {
        self.find(name, 288, 256, true)
    }
}

fn byte_string(value: &str) -> Result<String, Q3GameError> {
    let text = value.split('\0').next().unwrap_or("").to_string();
    for ch in text.chars() {
        if ch as u32 > 255 {
            return Err(range("Game configstrings require byte characters"));
        }
    }
    Ok(text)
}

/// String-valued entity field (`EntityStringField`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntityStringField {
    /// Classname.
    Classname,
    /// Model.
    Model,
    /// Model 2.
    Model2,
    /// Message.
    Message,
    /// Target.
    Target,
    /// Target name.
    Targetname,
    /// Team.
    Team,
    /// Target shader name.
    TargetShaderName,
    /// Target shader new name.
    TargetShaderNewName,
}

fn ascii_fold(value: &str) -> Vec<u8> {
    let text = value.split('\0').next().unwrap_or("");
    ascii_lower(text.as_bytes())
}

/// Find an entity by a string field (`findEntity`).
pub fn find_entity(
    pool: &dyn EntityPool,
    after: Option<usize>,
    field: EntityStringField,
    text: Option<&str>,
) -> Option<usize> {
    let text = text?;
    let folded = ascii_fold(text);
    let mut index = after.map_or(0, |slot| slot + 1);
    while index < pool.num_entities() {
        if let Some(entity) = pool.entity(index) {
            let value = match field {
                EntityStringField::Classname => entity.classname_value().map(str::to_string),
                EntityStringField::Model => entity.model.clone(),
                EntityStringField::Model2 => entity.model2.clone(),
                EntityStringField::Message => entity.message.clone(),
                EntityStringField::Target => entity.target.clone(),
                EntityStringField::Targetname => entity.targetname.clone(),
                EntityStringField::Team => entity.team.clone(),
                EntityStringField::TargetShaderName => entity.target_shader_name.clone(),
                EntityStringField::TargetShaderNewName => entity.target_shader_new_name.clone(),
            };
            if entity.inuse {
                if let Some(value) = value {
                    if ascii_fold(&value) == folded {
                        return Some(index);
                    }
                }
            }
        }
        index += 1;
    }
    None
}

/// Pick a random target by targetname (`G_PickTarget`).
pub fn pick_target(driver: &mut dyn Q3Driver, target_name: Option<&str>) -> Result<Option<usize>, Q3GameError> {
    let Some(target_name) = target_name else {
        driver.warn("G_PickTarget called with NULL targetname\n");
        return Ok(None);
    };
    let mut choices = Vec::new();
    let mut found = None;
    while choices.len() < 32 {
        found = find_entity(driver.pool(), found, EntityStringField::Targetname, Some(target_name));
        let Some(slot) = found else { break };
        choices.push(slot);
    }
    if choices.is_empty() {
        driver.warn(&format!("G_PickTarget: target {target_name} not found\n"));
        return Ok(None);
    }
    let random = driver.game_rand();
    if random < 0 {
        return Err(range("Game rand must return a nonnegative integer"));
    }
    Ok(Some(choices[(random as usize) % choices.len()]))
}

/// Dispatch targets and shader remaps (`useTargets`).
pub fn use_targets(driver: &mut dyn Q3Driver, slot: usize, activator: Option<Participant>) -> Result<(), Q3GameError> {
    let (shader_old, shader_new, target) = match driver.pool().entity(slot) {
        Some(entity) => (
            entity.target_shader_name.clone(),
            entity.target_shader_new_name.clone(),
            entity.target.clone(),
        ),
        None => return Ok(()),
    };
    if let (Some(old), Some(new)) = (shader_old, shader_new) {
        let time = driver.combat().time() as f32 * 0.001;
        driver.remap_shader(&old, &new, time);
    }
    let Some(target) = target else { return Ok(()) };
    let mut current = None;
    loop {
        current = find_entity(driver.pool(), current, EntityStringField::Targetname, Some(&target));
        let Some(target_slot) = current else { break };
        if target_slot == slot {
            driver.warn("WARNING: Entity used itself.\n");
        } else {
            let use_callback = driver
                .pool()
                .entity(target_slot)
                .and_then(|entity| entity.use_callback.clone());
            if let Some(use_callback) = use_callback {
                let other = Participant::Entity(slot);
                use_callback(driver, target_slot, Some(&other), activator.as_ref());
            }
        }
        let inuse = driver.pool().entity(slot).is_some_and(|entity| entity.inuse);
        if !inuse {
            driver.warn("entity was removed while using targets\n");
            return Ok(());
        }
    }
    Ok(())
}

/// Send a command to a team (`teamCommand`).
pub fn team_command(driver: &mut dyn Q3Driver, team: Q3Team, command: &str) {
    let max = driver.pool().max_clients();
    for index in 0..max {
        let send = driver.pool().client(index).is_some_and(|client| {
            client.pers.connected == ConnectionState::Connected as i32 && client.sess.session_team == team as i32
        });
        if send {
            driver.send_server_command(index as i32, command);
        }
    }
}

/// Editor direction conversion (`moveDirection`).
#[must_use]
pub fn move_direction(angles: Vec3) -> (Vec3, Vec3) {
    let vertical = angles.x == 0.0 && angles.z == 0.0;
    let direction = if vertical && angles.y == -1.0 {
        vec3(0.0, 0.0, 1.0)
    } else if vertical && angles.y == -2.0 {
        vec3(0.0, 0.0, -1.0)
    } else {
        angle_vectors(angles).forward
    };
    (direction, vec3(0.0, 0.0, 0.0))
}

// ---------------------------------------------------------------------------
// projectile.ts: shared missile simulation
// ---------------------------------------------------------------------------

/// Shared projectile (`Q3Projectile`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3Projectile {
    /// Actor.
    pub actor: ActorId,
    /// Owner.
    pub owner: ActorId,
    /// Weapon.
    pub weapon: i32,
    /// Direct damage.
    pub direct: i32,
    /// Splash damage.
    pub splash: i32,
    /// Splash radius.
    pub radius: i32,
    /// Means of death.
    pub method: i32,
    /// Splash means of death.
    pub splash_method: i32,
    /// Damage point.
    pub damage_point: Vec3,
    /// Trajectory.
    pub trajectory: Q3Trajectory,
    /// Flags.
    pub flags: i32,
    /// Pass actor.
    pub pass: Option<ActorId>,
}

/// Projectile target (`Q3ProjectileTarget`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q3ProjectileTarget {
    /// Damageable.
    pub damageable: bool,
    /// Player.
    pub player: bool,
    /// Accuracy eligible.
    pub accuracy_eligible: bool,
    /// Invulnerable.
    pub invulnerable: bool,
}

/// Projectile impact event (`Q3ProjectileImpact`).
#[derive(Debug, Clone, PartialEq)]
pub enum Q3ProjectileImpact {
    /// Bounce.
    Bounce {
        /// Normal.
        normal: Vec3,
    },
    /// Impact.
    Impact {
        /// Normal.
        normal: Vec3,
        /// Target.
        target: Option<ActorId>,
        /// Flesh.
        flesh: bool,
        /// Surface flags.
        surface_flags: i32,
    },
}

/// Projectile phase (`phase()`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectilePhase {
    /// Flight.
    Flight,
    /// Event.
    Event,
    /// Attached.
    Attached,
}

/// Reflection outcome (`reflection.impact` return).
#[derive(Debug, Clone, PartialEq)]
pub enum ReflectionOutcome {
    /// Miss.
    Miss,
    /// Hit with a bounce direction.
    Hit {
        /// Bounce direction.
        bounce_direction: Vec3,
    },
}

/// Projectile host (`Q3ProjectileHost`).
pub trait Q3ProjectileHost {
    /// Time.
    fn time(&self) -> i32;
    /// Previous time.
    fn previous_time(&self) -> i32;
    /// Whether the projectile is live (`live()`).
    fn is_live(&mut self) -> bool;
    /// Phase (`phase()`).
    fn phase(&mut self) -> ProjectilePhase;
    /// Event time (`eventTime()`).
    fn event_time(&mut self) -> i32;
    /// Clear the event (`clearEvent?`, default no-op).
    fn clear_event(&mut self) {}
    /// Current origin (`origin()`).
    fn origin(&mut self) -> Vec3;
    /// Move (`move(origin, velocity)`).
    fn move_to(&mut self, origin: Vec3, velocity: Vec3);
    /// Set origin (`setOrigin`).
    fn set_origin(&mut self, origin: Vec3);
    /// Link (`link`).
    fn link(&mut self);
    /// Release (`release`).
    fn release(&mut self);
    /// Trace (`trace`).
    fn trace(&mut self, start: Vec3, end: Vec3, pass: Option<&ActorId>) -> Q3TraceResult;
    /// Resolve a target (`target`).
    fn target(&mut self, actor: &ActorId) -> Option<Q3ProjectileTarget>;
    /// World actor (`worldActor`).
    fn world_actor(&mut self) -> ActorId;
    /// Emit an impact (`emit`).
    fn emit(&mut self, event: &Q3ProjectileImpact);
    /// Retain (`retain`).
    fn retain(&mut self);
    /// Damage (`damage`).
    fn damage(&mut self, target: &ActorId, direction: Vec3, point: Vec3);
    /// Radius damage (`radius`).
    fn radius(&mut self, origin: Vec3, ignore: Option<&ActorId>) -> bool;
    /// Credit accuracy (`accuracy`).
    fn accuracy(&mut self);
    /// Think (`think`).
    fn think(&mut self);
    /// Moved (`moved`).
    fn moved(&mut self);
    /// Reflection impact (`reflection`, default miss for null).
    fn reflection_impact(&mut self, target: &ActorId, direction: Vec3, point: Vec3) -> ReflectionOutcome {
        let _ = (target, direction, point);
        ReflectionOutcome::Miss
    }
    /// Whether reflection applies (`reflection` null check, default false).
    fn has_reflection(&mut self) -> bool {
        false
    }
    /// Special impact (`special.impact`, default false for null).
    fn special_impact(&mut self, trace: &Q3TraceResult, target: &ActorId) -> bool {
        let _ = (trace, target);
        false
    }
    /// Special after-move (`special.afterMove`, default no-op for null).
    fn special_after_move(&mut self) {}
    /// Special no-impact (`special.noImpact`, default no-op for null).
    fn special_no_impact(&mut self) {}
    /// Whether special hooks apply (`special` null check, default false).
    fn has_special(&mut self) -> bool {
        false
    }
}

/// Launch result (`q3LaunchProjectile` return).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProjectileLaunch {
    /// Expiry time.
    pub expires: i32,
    /// Trajectory.
    pub trajectory: Q3Trajectory,
}

/// Build a launch trajectory (`q3LaunchProjectile`).
#[must_use]
pub fn q3_launch_projectile(
    start: Vec3,
    direction: Vec3,
    speed: f32,
    gravity: bool,
    duration: i32,
    time: i32,
) -> ProjectileLaunch {
    ProjectileLaunch {
        expires: time.wrapping_add(duration),
        trajectory: Q3Trajectory {
            trajectory_type: if gravity {
                TrajectoryType::Gravity
            } else {
                TrajectoryType::Linear
            },
            time: time.wrapping_sub(50),
            duration: 0,
            base: start,
            delta: snap_vector(scale3(direction, speed)),
        },
    }
}

fn trace_normal(trace: &Q3TraceResult) -> Vec3 {
    match trace.contact {
        Q3TraceContact::Plane { normal, .. } => normal,
        Q3TraceContact::None => vec3(0.0, 0.0, 0.0),
    }
}

/// Bounce a projectile (`q3BounceProjectile`).
pub fn q3_bounce_projectile(projectile: &mut Q3Projectile, host: &mut dyn Q3ProjectileHost, trace: &Q3TraceResult) {
    let hit_time = q3_missile_hit_time(host.previous_time(), host.time(), trace.fraction);
    let plane = trace_normal(trace);
    let half = projectile.flags & 0x20 != 0;
    let delta = q3_bounce_velocity(evaluate_trajectory_delta(&projectile.trajectory, hit_time), plane, half);
    projectile.trajectory.delta = delta;
    if half && plane.z > 0.2 && length3(delta) < 40.0 {
        host.set_origin(trace.end);
        return;
    }
    let origin = add3(host.origin(), plane);
    host.move_to(origin, delta);
    projectile.trajectory.base = origin;
    projectile.trajectory.time = host.time();
}

/// Explode a projectile (`q3ExplodeProjectile`).
pub fn q3_explode_projectile(projectile: &mut Q3Projectile, host: &mut dyn Q3ProjectileHost) {
    let origin = snap_vector(evaluate_trajectory(&projectile.trajectory, host.time()));
    host.set_origin(origin);
    host.emit(&Q3ProjectileImpact::Impact {
        normal: vec3(0.0, 0.0, 1.0),
        target: None,
        flesh: false,
        surface_flags: 0,
    });
    if !host.is_live() {
        return;
    }
    host.retain();
    if projectile.splash != 0 && host.radius(origin, Some(&projectile.actor.clone())) {
        host.accuracy();
    }
    if host.is_live() {
        host.link();
    }
}

/// Impact a projectile (`q3ImpactProjectile`).
pub fn q3_impact_projectile(projectile: &mut Q3Projectile, host: &mut dyn Q3ProjectileHost, trace: &Q3TraceResult) {
    let actor = match &trace.hit {
        Q3TraceHit::Actor(actor) => actor.clone(),
        _ => host.world_actor(),
    };
    let plane = trace_normal(trace);
    let target = host.target(&actor);
    if target.is_none_or(|target| !target.damageable) && projectile.flags & 0x30 != 0 {
        q3_bounce_projectile(projectile, host, trace);
        host.emit(&Q3ProjectileImpact::Bounce { normal: plane });
        return;
    }
    if host.has_reflection()
        && target.is_some_and(|target| target.damageable && target.invulnerable)
        && projectile.weapon != 12
    {
        let effect = host.reflection_impact(
            &actor,
            normalize3(projectile.trajectory.delta),
            projectile.trajectory.base,
        );
        if !host.is_live() {
            return;
        }
        if let ReflectionOutcome::Hit { bounce_direction } = effect {
            let half = projectile.flags & 0x20;
            projectile.flags &= !0x20;
            let reflected = Q3TraceResult {
                contact: Q3TraceContact::Plane {
                    normal: bounce_direction,
                    distance: 0.0,
                },
                ..trace.clone()
            };
            q3_bounce_projectile(projectile, host, &reflected);
            projectile.flags |= half;
        }
        projectile.pass = Some(actor);
        return;
    }
    let mut hit_client = false;
    if target.is_some_and(|target| target.damageable) && projectile.direct != 0 {
        if target.is_some_and(|target| target.accuracy_eligible) {
            host.accuracy();
            hit_client = true;
        }
        let mut velocity = evaluate_trajectory_delta(&projectile.trajectory, host.time());
        if length3(velocity) == 0.0 {
            velocity = vec3(velocity.x, velocity.y, 1.0);
        }
        let point = projectile.damage_point;
        host.damage(&actor, velocity, point);
        if !host.is_live() {
            return;
        }
    }
    if (host.has_special() && host.special_impact(trace, &actor)) || !host.is_live() {
        return;
    }
    let current = host.target(&actor);
    host.emit(&Q3ProjectileImpact::Impact {
        normal: plane,
        target: Some(actor.clone()),
        flesh: current.is_some_and(|target| target.damageable && target.player),
        surface_flags: trace.surface_flags,
    });
    if !host.is_live() {
        return;
    }
    host.retain();
    let base = projectile.trajectory.base;
    let origin = snap_vector_towards(trace.end, base);
    host.set_origin(origin);
    if projectile.splash != 0 && host.radius(origin, Some(&actor)) && !hit_client {
        host.accuracy();
    }
    if host.is_live() {
        host.link();
    }
}

/// Step a projectile (`q3StepProjectile`).
pub fn q3_step_projectile(projectile: &mut Q3Projectile, host: &mut dyn Q3ProjectileHost) {
    if !host.is_live() {
        return;
    }
    if host.phase() == ProjectilePhase::Event {
        if host.time().wrapping_sub(host.event_time()) > EVENT_VALID_MSEC {
            host.release();
        }
        return;
    }
    if host.time().wrapping_sub(host.event_time()) > EVENT_VALID_MSEC {
        host.clear_event();
    }
    if host.phase() == ProjectilePhase::Attached {
        host.think();
        return;
    }
    let origin = host.origin();
    let destination = evaluate_trajectory(&projectile.trajectory, host.time());
    let pass = projectile.pass.clone();
    let mut trace = host.trace(origin, destination, pass.as_ref());
    if trace.solidity != Q3Solidity::Clear {
        let mut stuck = host.trace(origin, origin, pass.as_ref());
        stuck.fraction = 0.0;
        trace = stuck;
    } else {
        let velocity = evaluate_trajectory_delta(&projectile.trajectory, host.time());
        host.move_to(trace.end, velocity);
    }
    host.link();
    if trace.fraction != 1.0 {
        if trace.surface_flags & 16 != 0 {
            if host.has_special() {
                host.special_no_impact();
            }
            if host.is_live() {
                host.release();
            }
            return;
        }
        q3_impact_projectile(projectile, host, &trace);
        if !host.is_live() || host.phase() != ProjectilePhase::Flight {
            return;
        }
    }
    if host.has_special() {
        host.special_after_move();
    }
    if !host.is_live() {
        return;
    }
    host.moved();
    host.think();
}

// ---------------------------------------------------------------------------
// radius-damage.ts: radius falloff and visibility
// ---------------------------------------------------------------------------

/// Radius target (`Q3RadiusTarget`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q3RadiusTarget {
    /// Origin.
    pub origin: Vec3,
    /// Bounds.
    pub bounds: Bounds,
    /// Accuracy eligible.
    pub accuracy_eligible: bool,
}

/// Radius host (`Q3RadiusHost`).
pub trait Q3RadiusHost {
    /// Spatial queries.
    fn spatial(&mut self) -> &mut dyn SpatialQueries;
    /// Resolve a target.
    fn target(&mut self, actor: &ActorId) -> Option<Q3RadiusTarget>;
    /// Apply damage.
    fn damage(&mut self, actor: &ActorId, direction: Vec3, point: Vec3, amount: i32);
}

/// Visibility check (`q3CanDamage`).
pub fn q3_can_damage(spatial: &dyn SpatialQueries, actor: &ActorId, bounds: &Bounds, origin: Vec3) -> bool {
    let midpoint = scale3(add3(bounds.min, bounds.max), 0.5);
    let trace = |end: Vec3| {
        spatial.trace_actor(&Q3TraceQuery {
            start: origin,
            end,
            shape: Q3TraceShape::Point,
            pass_actor: None,
            mask: 1,
        })
    };
    let center = trace(midpoint);
    if center.fraction == 1.0 || matches!(&center.hit, Q3TraceHit::Actor(hit) if hit == actor) {
        return true;
    }
    for (x, y) in [(15.0, 15.0), (15.0, -15.0), (-15.0, 15.0), (-15.0, -15.0)] {
        if trace(vec3(midpoint.x + x, midpoint.y + y, midpoint.z)).fraction == 1.0 {
            return true;
        }
    }
    false
}

/// Radius damage (`q3RadiusDamage`).
pub fn q3_radius_damage(
    host: &mut dyn Q3RadiusHost,
    origin: Vec3,
    amount: f32,
    radius: f32,
    ignore: Option<&ActorId>,
) -> bool {
    let radius = radius.max(1.0);
    let extent = vec3(radius, radius, radius);
    let candidates = host.spatial().area_actors(
        &Bounds {
            min: sub3(origin, extent),
            max: add3(origin, extent),
        },
        1024,
    );
    let mut hit_client = false;
    for actor in candidates {
        if ignore.is_some_and(|ignored| ignored == &actor) {
            continue;
        }
        let Some(target) = host.target(&actor) else { continue };
        let axis = |value: f32, min: f32, max: f32| -> f32 {
            if value < min {
                min - value
            } else if value > max {
                value - max
            } else {
                0.0
            }
        };
        let distance = length3(vec3(
            axis(origin.x, target.bounds.min.x, target.bounds.max.x),
            axis(origin.y, target.bounds.min.y, target.bounds.max.y),
            axis(origin.z, target.bounds.min.z, target.bounds.max.z),
        ));
        if distance >= radius {
            continue;
        }
        let points = amount * (1.0 - distance / radius);
        if !q3_can_damage(host.spatial(), &actor, &target.bounds, origin) {
            continue;
        }
        if target.accuracy_eligible {
            hit_client = true;
        }
        let direction = add3(sub3(target.origin, origin), vec3(0.0, 0.0, 24.0));
        host.damage(&actor, direction, origin, points.trunc() as i32);
    }
    hit_client
}

// ---------------------------------------------------------------------------
// rankings.ts: ranking reports (g_rankings.c)
// ---------------------------------------------------------------------------

/// Ranking report (`Q3RankingReport`).
#[derive(Debug, Clone, PartialEq)]
pub enum Q3RankingReport {
    /// Integer report.
    Integer {
        /// Self.
        slf: i32,
        /// Other.
        other: i32,
        /// Key.
        key: i32,
        /// Value.
        value: i32,
        /// Accumulate.
        accumulate: bool,
    },
    /// String report.
    String {
        /// Self.
        slf: i32,
        /// Other.
        other: i32,
        /// Key.
        key: i32,
        /// Value.
        value: String,
    },
}

struct RankingInner {
    sink: Option<Rc<dyn Fn(Q3RankingReport)>>,
    warmup: Rc<dyn Fn() -> bool>,
    last_hit: String,
}

/// Ranking reports (`Q3RankingReports`).
#[derive(Clone)]
pub struct Q3RankingReports {
    inner: Rc<RefCell<RankingInner>>,
}

impl std::fmt::Debug for Q3RankingReports {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Q3RankingReports")
            .field("attached", &self.inner.borrow().sink.is_some())
            .finish_non_exhaustive()
    }
}

impl Q3RankingReports {
    /// New reports.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Rc::new(RefCell::new(RankingInner {
                sink: None,
                warmup: Rc::new(|| true),
                last_hit: String::new(),
            })),
        }
    }

    /// Attach a sink (`attach`), returning a detach closure.
    pub fn attach(
        &self,
        sink: Rc<dyn Fn(Q3RankingReport)>,
        warmup: Rc<dyn Fn() -> bool>,
    ) -> Result<Rc<dyn Fn()>, Q3GameError> {
        if self.inner.borrow().sink.is_some() {
            return Err(failure("Ranking report owner already attached"));
        }
        self.inner.borrow_mut().sink = Some(sink);
        self.inner.borrow_mut().warmup = warmup;
        let inner: Weak<RefCell<RankingInner>> = Rc::downgrade(&self.inner);
        Ok(Rc::new(move || {
            if let Some(inner) = inner.upgrade() {
                inner.borrow_mut().sink = None;
                inner.borrow_mut().warmup = Rc::new(|| true);
            }
        }))
    }

    fn is_warmup(&self) -> bool {
        (self.inner.borrow().warmup)()
    }

    fn emit(&self, report: Q3RankingReport) {
        if let Some(sink) = self.inner.borrow().sink.clone() {
            sink(report);
        }
    }

    /// Integer report (`integer`).
    pub fn integer(&self, slf: i32, other: i32, key: i32, value: i32, accumulate: bool) {
        if !self.is_warmup() {
            self.emit(Q3RankingReport::Integer {
                slf,
                other,
                key,
                value,
                accumulate,
            });
        }
    }

    /// String report (`string`).
    pub fn string(&self, slf: i32, other: i32, key: i32, value: &str) {
        if !self.is_warmup() {
            self.emit(Q3RankingReport::String {
                slf,
                other,
                key,
                value: value.to_string(),
            });
        }
    }

    /// Fire-weapon reports.
    pub fn fire_weapon(&self, slf: i32, weapon: i32) {
        if self.is_warmup() || weapon == Q3Weapon::Gauntlet as i32 {
            return;
        }
        self.integer(slf, -1, 1111020002, 1, true);
        match weapon {
            x if x == Q3Weapon::Machinegun as i32 => self.integer(slf, -1, 1111020202, 1, true),
            x if x == Q3Weapon::Shotgun as i32 => self.integer(slf, -1, 1111020302, 1, true),
            x if x == Q3Weapon::GrenadeLauncher as i32 => self.integer(slf, -1, 1111020402, 1, true),
            x if x == Q3Weapon::RocketLauncher as i32 => self.integer(slf, -1, 1111020502, 1, true),
            x if x == Q3Weapon::Lightning as i32 => self.integer(slf, -1, 1111020802, 1, true),
            x if x == Q3Weapon::Railgun as i32 => self.integer(slf, -1, 1111020702, 1, true),
            x if x == Q3Weapon::Plasmagun as i32 => self.integer(slf, -1, 1111020602, 1, true),
            x if x == Q3Weapon::Bfg as i32 => self.integer(slf, -1, 1111020902, 1, true),
            x if x == Q3Weapon::GrapplingHook as i32 => self.integer(slf, -1, 1111021002, 1, true),
            _ => {}
        }
    }

    /// Pickup-weapon reports.
    pub fn pickup_weapon(&self, slf: i32, weapon: i32) {
        if self.is_warmup() {
            return;
        }
        self.integer(slf, -1, 1111020009, 1, true);
        match weapon {
            x if x == Q3Weapon::Gauntlet as i32 => self.integer(slf, -1, 1111020109, 1, true),
            x if x == Q3Weapon::Machinegun as i32 => self.integer(slf, -1, 1111020209, 1, true),
            x if x == Q3Weapon::Shotgun as i32 => self.integer(slf, -1, 1111020309, 1, true),
            x if x == Q3Weapon::GrenadeLauncher as i32 => self.integer(slf, -1, 1111020409, 1, true),
            x if x == Q3Weapon::RocketLauncher as i32 => self.integer(slf, -1, 1111020509, 1, true),
            x if x == Q3Weapon::Lightning as i32 => self.integer(slf, -1, 1111020809, 1, true),
            x if x == Q3Weapon::Railgun as i32 => self.integer(slf, -1, 1111020709, 1, true),
            x if x == Q3Weapon::Plasmagun as i32 => self.integer(slf, -1, 1111020609, 1, true),
            x if x == Q3Weapon::Bfg as i32 => self.integer(slf, -1, 1111020909, 1, true),
            x if x == Q3Weapon::GrapplingHook as i32 => self.integer(slf, -1, 1111021009, 1, true),
            _ => {}
        }
    }

    /// Pickup-ammo reports.
    pub fn pickup_ammo(&self, slf: i32, weapon: i32, quantity: i32) {
        if self.is_warmup() {
            return;
        }
        self.integer(slf, -1, 1111030000, 1, true);
        self.integer(slf, -1, 1111030001, quantity, true);
        match weapon {
            x if x == Q3Weapon::Machinegun as i32 => {
                self.integer(slf, -1, 1111030100, 1, true);
                self.integer(slf, -1, 1111030101, quantity, true);
            }
            x if x == Q3Weapon::Shotgun as i32 => {
                self.integer(slf, -1, 1111030200, 1, true);
                self.integer(slf, -1, 1111030201, quantity, true);
            }
            x if x == Q3Weapon::GrenadeLauncher as i32 => {
                self.integer(slf, -1, 1111030300, 1, true);
                self.integer(slf, -1, 1111030301, quantity, true);
            }
            x if x == Q3Weapon::RocketLauncher as i32 => {
                self.integer(slf, -1, 1111030400, 1, true);
                self.integer(slf, -1, 1111030401, quantity, true);
            }
            x if x == Q3Weapon::Lightning as i32 => {
                self.integer(slf, -1, 1111030700, 1, true);
                self.integer(slf, -1, 1111030701, quantity, true);
            }
            x if x == Q3Weapon::Railgun as i32 => {
                self.integer(slf, -1, 1111030600, 1, true);
                self.integer(slf, -1, 1111030601, quantity, true);
            }
            x if x == Q3Weapon::Plasmagun as i32 => {
                self.integer(slf, -1, 1111030500, 1, true);
                self.integer(slf, -1, 1111030501, quantity, true);
            }
            x if x == Q3Weapon::Bfg as i32 => {
                self.integer(slf, -1, 1111030800, 1, true);
                self.integer(slf, -1, 1111030801, quantity, true);
            }
            _ => {}
        }
    }

    /// Pickup-health reports.
    pub fn pickup_health(&self, slf: i32, quantity: i32) {
        if self.is_warmup() {
            return;
        }
        self.integer(slf, -1, 1111040000, 1, true);
        self.integer(slf, -1, 1111040001, quantity, true);
        match quantity {
            5 => self.integer(slf, -1, 1111040100, 1, true),
            25 => self.integer(slf, -1, 1111040200, 1, true),
            50 => self.integer(slf, -1, 1111040300, 1, true),
            100 => self.integer(slf, -1, 1111040400, 1, true),
            _ => {}
        }
    }

    /// Pickup-armor reports.
    pub fn pickup_armor(&self, slf: i32, quantity: i32) {
        if self.is_warmup() {
            return;
        }
        self.integer(slf, -1, 1111050000, 1, true);
        self.integer(slf, -1, 1111050001, quantity, true);
        match quantity {
            5 => self.integer(slf, -1, 1111050100, 1, true),
            50 => self.integer(slf, -1, 1111050200, 1, true),
            100 => self.integer(slf, -1, 1111050300, 1, true),
            _ => {}
        }
    }

    /// Pickup-powerup reports.
    pub fn pickup_powerup(&self, slf: i32, powerup: i32) {
        if self.is_warmup() {
            return;
        }
        if powerup == Q3Powerup::Redflag as i32 || powerup == Q3Powerup::Blueflag as i32 {
            self.integer(slf, -1, 1111110000, 1, true);
            return;
        }
        self.integer(slf, -1, 1111060000, 1, true);
        match powerup {
            x if x == Q3Powerup::Quad as i32 => self.integer(slf, -1, 1111060100, 1, true),
            x if x == Q3Powerup::Battlesuit as i32 => self.integer(slf, -1, 1111060200, 1, true),
            x if x == Q3Powerup::Haste as i32 => self.integer(slf, -1, 1111060300, 1, true),
            x if x == Q3Powerup::Invis as i32 => self.integer(slf, -1, 1111060400, 1, true),
            x if x == Q3Powerup::Regen as i32 => self.integer(slf, -1, 1111060500, 1, true),
            x if x == Q3Powerup::Flight as i32 => self.integer(slf, -1, 1111060600, 1, true),
            _ => {}
        }
    }

    /// Pickup-holdable reports.
    pub fn pickup_holdable(&self, slf: i32, holdable: i32) {
        if self.is_warmup() {
            return;
        }
        match holdable {
            x if x == Q3Holdable::Medkit as i32 => self.integer(slf, -1, 1111070000, 1, true),
            x if x == Q3Holdable::Teleporter as i32 => self.integer(slf, -1, 1111070100, 1, true),
            _ => {}
        }
    }

    /// Use-holdable reports.
    pub fn use_holdable(&self, slf: i32, holdable: i32) {
        if self.is_warmup() {
            return;
        }
        match holdable {
            x if x == Q3Holdable::Medkit as i32 => self.integer(slf, -1, 1111070001, 1, true),
            x if x == Q3Holdable::Teleporter as i32 => self.integer(slf, -1, 1111070101, 1, true),
            _ => {}
        }
    }

    /// Reward reports.
    pub fn reward(&self, slf: i32, award: i32) {
        if self.is_warmup() {
            return;
        }
        match award {
            0x8000 => self.integer(slf, -1, 1111090000, 1, true),
            0x8 => self.integer(slf, -1, 1111090100, 1, true),
            _ => {}
        }
    }

    /// Capture report.
    pub fn capture(&self, slf: i32) {
        if self.is_warmup() {
            return;
        }
        self.integer(slf, -1, 1111110001, 1, true);
    }

    /// Damage reports.
    #[allow(clippy::too_many_arguments)]
    pub fn damage(
        &self,
        slf: i32,
        attacker: i32,
        damage: i32,
        means_of_death: i32,
        frame: i32,
        attacker_is_client: bool,
        same_team: bool,
    ) {
        if self.is_warmup() {
            return;
        }
        let hit = format!("{frame}:{slf}:{attacker}:{means_of_death}");
        let new_hit = hit != self.inner.borrow().last_hit;
        self.inner.borrow_mut().last_hit = hit;
        if attacker != 1022 && attacker != slf && means_of_death == 2 && attacker_is_client {
            self.integer(attacker, -1, 1111020102, 1, true);
        }
        match means_of_death {
            14 | 15 | 16 | 17 | 18 | 19 | 20 | 22 => return,
            _ => {}
        }
        let splash = match means_of_death {
            5 | 7 | 9 | 13 => damage,
            _ => 0,
        };
        let (key_hit, key_damage, mut key_splash) = match means_of_death {
            2 => (1111020104, 1111020106, -1),
            3 => (1111020204, 1111020206, -1),
            1 => (1111020304, 1111020306, -1),
            4 | 5 => (1111020404, 1111020406, 1111020408),
            6 | 7 => (1111020504, 1111020506, 1111020508),
            8 | 9 => (1111020604, 1111020606, 1111020608),
            10 => (1111020704, 1111020706, -1),
            11 => (1111020804, 1111020806, -1),
            12 | 13 => (1111020904, 1111020906, 1111020908),
            23 => (1111021004, 1111021006, -1),
            _ => (1111021104, 1111021106, -1),
        };
        if means_of_death != 5 && means_of_death != 7 && means_of_death != 9 && means_of_death != 13 {
            key_splash = -1;
        }
        if new_hit {
            self.integer(slf, -1, 1111020004, 1, true);
            self.integer(slf, -1, key_hit, 1, true);
        }
        self.integer(slf, -1, 1111020006, damage, true);
        self.integer(slf, -1, key_damage, damage, true);
        if splash != 0 {
            self.integer(slf, -1, 1111020008, splash, true);
            self.integer(slf, -1, key_splash, splash, true);
        }
        if attacker != 1022 && attacker != slf {
            let (key_hit, key_damage, key_splash) = match means_of_death {
                2 => (1111020103, 1111020105, -1),
                3 => (1111020203, 1111020205, -1),
                1 => (1111020303, 1111020305, -1),
                4 | 5 => (1111020403, 1111020405, 1111020407),
                6 | 7 => (1111020503, 1111020505, 1111020507),
                8 | 9 => (1111020603, 1111020605, 1111020607),
                10 => (1111020703, 1111020705, -1),
                11 => (1111020803, 1111020805, -1),
                12 | 13 => (1111020903, 1111020905, 1111020907),
                23 => (1111021003, 1111021005, -1),
                _ => (1111021103, 1111021105, -1),
            };
            if attacker_is_client {
                if new_hit {
                    self.integer(attacker, -1, 1111020003, 1, true);
                    self.integer(attacker, -1, key_hit, 1, true);
                }
                self.integer(attacker, -1, 1111020005, damage, true);
                self.integer(attacker, -1, key_damage, damage, true);
                if splash != 0 {
                    self.integer(attacker, -1, 1111020007, splash, true);
                    self.integer(attacker, -1, key_splash, splash, true);
                }
            }
        }
        if attacker != slf && same_team && attacker_is_client {
            if new_hit {
                self.integer(slf, -1, 1111100002, 1, true);
                self.integer(attacker, -1, 1111100001, 1, true);
            }
            self.integer(slf, -1, 1111100004, damage, true);
            self.integer(attacker, -1, 1111100003, damage, true);
            if splash != 0 {
                self.integer(slf, -1, 1111100006, splash, true);
                self.integer(attacker, -1, 1111100005, splash, true);
            }
        }
    }

    /// Player-die reports.
    pub fn player_die(&self, slf: i32, attacker: i32, means_of_death: i32) {
        if self.is_warmup() {
            return;
        }
        if attacker == 1022 {
            self.integer(slf, -1, 1111080000, 1, true);
            match means_of_death {
                14 => self.integer(slf, -1, 1111080100, 1, true),
                15 => self.integer(slf, -1, 1111080200, 1, true),
                16 => self.integer(slf, -1, 1111080300, 1, true),
                17 => self.integer(slf, -1, 1111080400, 1, true),
                18 => self.integer(slf, -1, 1111080500, 1, true),
                19 => self.integer(slf, -1, 1111080600, 1, true),
                20 => self.integer(slf, -1, 1111080700, 1, true),
                22 => self.integer(slf, -1, 1111080800, 1, true),
                _ => self.integer(slf, -1, 1111080900, 1, true),
            }
        } else if attacker == slf {
            self.integer(slf, -1, 1111020001, 1, true);
            match means_of_death {
                2 => self.integer(slf, -1, 1111020101, 1, true),
                3 => self.integer(slf, -1, 1111020201, 1, true),
                1 => self.integer(slf, -1, 1111020301, 1, true),
                4 | 5 => self.integer(slf, -1, 1111020401, 1, true),
                6 | 7 => self.integer(slf, -1, 1111020501, 1, true),
                8 | 9 => self.integer(slf, -1, 1111020601, 1, true),
                10 => self.integer(slf, -1, 1111020701, 1, true),
                11 => self.integer(slf, -1, 1111020801, 1, true),
                12 | 13 => self.integer(slf, -1, 1111020901, 1, true),
                23 => self.integer(slf, -1, 1111021001, 1, true),
                _ => self.integer(slf, -1, 1111021101, 1, true),
            }
        } else {
            self.integer(attacker, slf, 1211020000, 1, true);
            match means_of_death {
                2 => self.integer(attacker, slf, 1211020100, 1, true),
                3 => self.integer(attacker, slf, 1211020200, 1, true),
                1 => self.integer(attacker, slf, 1211020300, 1, true),
                4 | 5 => self.integer(attacker, slf, 1211020400, 1, true),
                6 | 7 => self.integer(attacker, slf, 1211020500, 1, true),
                8 | 9 => self.integer(attacker, slf, 1211020600, 1, true),
                10 => self.integer(attacker, slf, 1211020700, 1, true),
                11 => self.integer(attacker, slf, 1211020800, 1, true),
                12 | 13 => self.integer(attacker, slf, 1211020900, 1, true),
                23 => self.integer(attacker, slf, 1211021000, 1, true),
                _ => self.integer(attacker, slf, 1211021100, 1, true),
            }
        }
    }

    /// Weapon-time reports.
    pub fn weapon_time(&self, slf: i32, weapon: i32, time: i32) {
        if time <= 0 || self.is_warmup() {
            return;
        }
        self.integer(slf, -1, 1111020010, time, true);
        match weapon {
            x if x == Q3Weapon::Gauntlet as i32 => self.integer(slf, -1, 1111020110, time, true),
            x if x == Q3Weapon::Machinegun as i32 => self.integer(slf, -1, 1111020210, time, true),
            x if x == Q3Weapon::Shotgun as i32 => self.integer(slf, -1, 1111020310, time, true),
            x if x == Q3Weapon::GrenadeLauncher as i32 => self.integer(slf, -1, 1111020410, time, true),
            x if x == Q3Weapon::RocketLauncher as i32 => self.integer(slf, -1, 1111020510, time, true),
            x if x == Q3Weapon::Lightning as i32 => self.integer(slf, -1, 1111020810, time, true),
            x if x == Q3Weapon::Railgun as i32 => self.integer(slf, -1, 1111020710, time, true),
            x if x == Q3Weapon::Plasmagun as i32 => self.integer(slf, -1, 1111020610, time, true),
            x if x == Q3Weapon::Bfg as i32 => self.integer(slf, -1, 1111020910, time, true),
            x if x == Q3Weapon::GrapplingHook as i32 => self.integer(slf, -1, 1111021010, time, true),
            _ => {}
        }
    }

    /// Team-name report.
    pub fn team_name(&self, slf: i32, name: &str) {
        self.string(slf, -1, 1100100007, name);
    }
}

impl Default for Q3RankingReports {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// shader-remaps.ts: shader remap state (g_utils.c)
// ---------------------------------------------------------------------------

/// Maximum path length (`MAX_QPATH`).
pub const MAX_QPATH: usize = 64;
/// Maximum remaps (`MAX_SHADER_REMAPS`).
pub const MAX_SHADER_REMAPS: usize = 128;
/// Entry buffer bytes.
const ENTRY_BUFFER_BYTES: usize = MAX_QPATH * 2 + 5;
/// State buffer bytes.
const STATE_BUFFER_BYTES: usize = 1024 * 4;

/// Shader remap entry.
#[derive(Debug, Clone, PartialEq)]
pub struct ShaderRemap {
    /// Old name.
    pub old_name: String,
    /// New name.
    pub new_name: String,
    /// Time offset.
    pub time_offset: f32,
}

fn quake_path(value: &str) -> Result<String, Q3GameError> {
    let path = value.split('\0').next().unwrap_or("").to_string();
    for ch in path.chars() {
        if ch as u32 > 255 {
            return Err(range("shader remap paths must contain byte-valued code units"));
        }
    }
    if path.chars().count() >= MAX_QPATH {
        return Err(range("shader remap paths must fit MAX_QPATH including the terminator"));
    }
    Ok(path)
}

fn folded_ascii_byte(value: u8) -> u8 {
    if (97..=122).contains(&value) {
        value - 32
    } else {
        value
    }
}

fn quake_path_equal(left: &str, right: &str) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.bytes()
        .zip(right.bytes())
        .all(|(a, b)| folded_ascii_byte(a) == folded_ascii_byte(b))
}

fn stored_time_offset(value: f64) -> Result<f32, Q3GameError> {
    #[allow(clippy::cast_possible_truncation)]
    let stored = value as f32;
    if !stored.is_finite() || stored.abs() > 2_147_483_647.0 {
        return Err(range(
            "shader remap time is outside the source formatter's safe int-cast range",
        ));
    }
    Ok(stored)
}

/// Shader remap registry (`ShaderRemapRegistry`).
pub struct ShaderRemapRegistry {
    remaps: Vec<ShaderRemap>,
    print: Rc<dyn Fn(&str)>,
}

impl ShaderRemapRegistry {
    /// New registry with a print hook.
    #[must_use]
    pub fn new(print: Rc<dyn Fn(&str)>) -> Self {
        Self {
            remaps: Vec::new(),
            print,
        }
    }

    /// Remap entries.
    #[must_use]
    pub fn remaps(&self) -> &[ShaderRemap] {
        &self.remaps
    }

    /// Capture save state.
    #[must_use]
    pub fn capture_save_state(&self) -> SaveJson {
        arr(self
            .remaps
            .iter()
            .map(|remap| {
                obj(vec![
                    ("oldName", str(&remap.old_name)),
                    ("newName", str(&remap.new_name)),
                    ("timeOffset", num(f64::from(remap.time_offset))),
                ])
            })
            .collect())
    }

    /// Restore save state.
    pub fn restore_save_state(&mut self, value: &SaveJson) -> Result<(), Q3GameError> {
        let reader = SaveReader::at(value, "q3.remaps");
        let remaps: Vec<ShaderRemap> = reader.list(|entry| -> Result<ShaderRemap, Q3GameError> {
            Ok(ShaderRemap {
                old_name: quake_path(&entry.field("oldName").string()?)?,
                new_name: quake_path(&entry.field("newName").string()?)?,
                time_offset: stored_time_offset(entry.field("timeOffset").number()?)?,
            })
        })?;
        let duplicate = remaps.iter().enumerate().any(|(index, entry)| {
            remaps[..index]
                .iter()
                .any(|previous| quake_path_equal(&previous.old_name, &entry.old_name))
        });
        if remaps.len() > MAX_SHADER_REMAPS || duplicate {
            return Err(reader.fail("invalid shader remap table").into());
        }
        self.remaps = remaps;
        Ok(())
    }

    /// Add or update a remap (`AddRemap`).
    pub fn add(&mut self, old_name: &str, new_name: &str, time_offset: f64) -> Result<(), Q3GameError> {
        let old_path = quake_path(old_name)?;
        let new_path = quake_path(new_name)?;
        let stored_time = stored_time_offset(time_offset)?;
        for remap in &mut self.remaps {
            if quake_path_equal(&old_path, &remap.old_name) {
                remap.new_name = new_path;
                remap.time_offset = stored_time;
                return Ok(());
            }
        }
        if self.remaps.len() < MAX_SHADER_REMAPS {
            self.remaps.push(ShaderRemap {
                old_name: old_path,
                new_name: new_path,
                time_offset: stored_time,
            });
        }
        Ok(())
    }

    /// Build the shader-state configstring (`BuildShaderStateConfig`).
    pub fn build_shader_state_config(&self) -> Result<String, Q3GameError> {
        let mut state = String::new();
        for remap in &self.remaps {
            let formatted = game_format(
                "%s=%s:%5.2f@",
                &[
                    GameFormatArg::Text(Some(remap.old_name.clone())),
                    GameFormatArg::Text(Some(remap.new_name.clone())),
                    GameFormatArg::Float(remap.time_offset),
                ],
            )?;
            if formatted.len() >= ENTRY_BUFFER_BYTES {
                (self.print)(&game_format(
                    "Com_sprintf: overflow of %i in %i\n",
                    &[
                        GameFormatArg::Int(formatted.len() as i32),
                        GameFormatArg::Int(ENTRY_BUFFER_BYTES as i32),
                    ],
                )?);
            }
            let entry = formatted[..formatted.len().min(ENTRY_BUFFER_BYTES - 1)].to_string();
            let writable = STATE_BUFFER_BYTES.saturating_sub(state.len()).saturating_sub(1);
            if writable > 0 {
                state.push_str(&entry[..entry.len().min(writable)]);
            }
        }
        Ok(state)
    }
}

// ---------------------------------------------------------------------------
// spawn.ts: spawn variables and entity dispatch (g_spawn.c)
// ---------------------------------------------------------------------------

/// Maximum spawn variables (`MAX_SPAWN_VARS`).
pub const MAX_SPAWN_VARS: usize = 64;
/// Maximum spawn variable characters (`MAX_SPAWN_VARS_CHARS`).
pub const MAX_SPAWN_VARS_CHARS: usize = 4096;
/// Maximum token characters (`TOKEN_MAX` from `common-parse.ts`).
pub const TOKEN_MAX: usize = 1024;

/// Spawn pair (`SpawnPair`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnPair {
    /// Key.
    pub key: String,
    /// Value.
    pub value: String,
}

/// Spawn value with presence (`SpawnValue`).
#[derive(Debug, Clone, PartialEq)]
pub struct SpawnValue<T> {
    /// Present.
    pub present: bool,
    /// Value.
    pub value: T,
}

fn spawn_lower(value: &str) -> Vec<u8> {
    ascii_lower(value.as_bytes())
}

/// Spawn string with escape processing (`G_NewString`).
pub fn new_spawn_string(value: &str, memory: &mut GameMemory) -> Result<String, Q3GameError> {
    let bytes = latin1_bytes(value)?;
    let allocation = memory.allocate(bytes.len() + 1)?;
    {
        let out = memory.alloc_bytes_mut(&allocation);
        let mut output = 0usize;
        let mut index = 0usize;
        while index <= bytes.len() {
            let character = if index == bytes.len() { 0u8 } else { bytes[index] };
            if character == 92 && index < bytes.len() {
                index += 1;
                let next = if index < bytes.len() { bytes[index] } else { 0 };
                out[output] = if next == b'n' { 10 } else { 92 };
                output += 1;
            } else {
                out[output] = character;
                output += 1;
            }
            index += 1;
        }
    }
    memory.read_string(&allocation)
}

/// Ordered spawn variables (`SpawnVariables`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnVariables {
    /// Entries.
    pub entries: Vec<SpawnPair>,
    /// Character count.
    pub character_count: usize,
}

impl SpawnVariables {
    /// New variables with source-limit validation.
    pub fn new(entries: Vec<SpawnPair>) -> Result<Self, Q3GameError> {
        if entries.len() > MAX_SPAWN_VARS {
            return Err(range("G_ParseSpawnVars: MAX_SPAWN_VARS"));
        }
        let mut characters = 0usize;
        for pair in &entries {
            for value in [&pair.key, &pair.value] {
                let bytes = latin1_bytes(value)?;
                if bytes.len() >= TOKEN_MAX {
                    return Err(range("Spawn token exceeds MAX_TOKEN_CHARS"));
                }
                characters += bytes.len() + 1;
                if characters > MAX_SPAWN_VARS_CHARS {
                    return Err(range("G_AddSpawnVarToken: MAX_SPAWN_CHARS"));
                }
            }
        }
        Ok(Self {
            entries,
            character_count: characters,
        })
    }

    /// String value, first match (`string`).
    #[must_use]
    pub fn string(&self, key: &str, default_value: &str) -> SpawnValue<String> {
        let normalized = spawn_lower(key);
        match self.entries.iter().find(|pair| spawn_lower(&pair.key) == normalized) {
            Some(found) => SpawnValue {
                present: true,
                value: found.value.clone(),
            },
            None => SpawnValue {
                present: false,
                value: default_value.to_string(),
            },
        }
    }

    /// Integer value (`int`).
    pub fn int(&self, key: &str, default_value: &str) -> Result<SpawnValue<i32>, Q3GameError> {
        let found = self.string(key, default_value);
        Ok(SpawnValue {
            present: found.present,
            value: game_atoi(&found.value)?,
        })
    }

    /// Float value (`float`).
    pub fn float(&self, key: &str, default_value: &str) -> Result<SpawnValue<f32>, Q3GameError> {
        let found = self.string(key, default_value);
        Ok(SpawnValue {
            present: found.present,
            value: game_atof(&found.value)?,
        })
    }

    /// Vector value (`vector`).
    pub fn vector(&self, key: &str, default_value: &str) -> Result<SpawnValue<Vec3>, Q3GameError> {
        let found = self.string(key, default_value);
        Ok(SpawnValue {
            present: found.present,
            value: scan_game_vector(&found.value)?,
        })
    }
}

/// Spawn token (`Token`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnToken {
    /// Value.
    pub value: String,
    /// Line.
    pub line: usize,
    /// Column.
    pub column: usize,
    /// Quoted.
    pub quoted: bool,
}

/// Native-x86 spawn parser (`SpawnParser`).
#[derive(Debug, Clone)]
pub struct SpawnParser {
    text: Vec<u8>,
    name: String,
    offset: usize,
    line: usize,
    column: usize,
}

impl SpawnParser {
    /// New parser over spawn text.
    pub fn new(text: &str, name: &str) -> Result<Self, Q3GameError> {
        let cut = text.split('\0').next().unwrap_or("");
        latin1_bytes(cut)?;
        Ok(Self {
            text: cut.bytes().collect(),
            name: name.to_string(),
            offset: 0,
            line: 1,
            column: 1,
        })
    }

    fn error(&self, message: &str, token: Option<&SpawnToken>) -> Q3GameError {
        Q3GameError::Parse {
            source: self.name.clone(),
            line: token.map_or(self.line, |token| token.line),
            column: token.map_or(self.column, |token| token.column),
            message: message.to_string(),
        }
    }

    fn advance(&mut self) {
        if self.text.get(self.offset) == Some(&b'\n') {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
        self.offset += 1;
    }

    fn token(&mut self) -> Result<Option<SpawnToken>, Q3GameError> {
        loop {
            while self.offset < self.text.len() {
                let code = self.text[self.offset];
                if code > 32 && code < 128 {
                    break;
                }
                self.advance();
            }
            if self.text[self.offset..].starts_with(b"//") {
                while self.offset < self.text.len() && self.text[self.offset] != b'\n' {
                    self.advance();
                }
            } else if self.text[self.offset..].starts_with(b"/*") {
                self.advance();
                self.advance();
                while self.offset < self.text.len() && !self.text[self.offset..].starts_with(b"*/") {
                    self.advance();
                }
                if self.offset < self.text.len() {
                    self.advance();
                    self.advance();
                }
            } else {
                break;
            }
        }
        if self.offset == self.text.len() {
            return Ok(None);
        }
        let line = self.line;
        let column = self.column;
        let quoted = self.text[self.offset] == b'"';
        let mut value = Vec::new();
        if quoted {
            self.advance();
        }
        while self.offset < self.text.len() {
            let code = self.text[self.offset];
            if quoted && code == b'"' {
                break;
            }
            if !quoted && (code <= 32 || code >= 128) {
                break;
            }
            value.push(code);
            self.advance();
            if value.len() >= TOKEN_MAX {
                let token = SpawnToken {
                    value: latin1_string(&value),
                    line,
                    column,
                    quoted,
                };
                return Err(self.error("Spawn token exceeds MAX_TOKEN_CHARS", Some(&token)));
            }
        }
        if quoted && self.offset < self.text.len() {
            self.advance();
        }
        Ok(Some(SpawnToken {
            value: latin1_string(&value),
            line,
            column,
            quoted,
        }))
    }

    /// Parse the next variable block (`next`).
    pub fn next(&mut self) -> Result<Option<SpawnVariables>, Q3GameError> {
        let Some(opening) = self.token()? else { return Ok(None) };
        if !opening.value.starts_with('{') {
            return Err(self.error(
                &format!("G_ParseSpawnVars: found {} when expecting {{", opening.value),
                Some(&opening),
            ));
        }
        let mut entries = Vec::new();
        let mut characters = 0usize;
        loop {
            let Some(key) = self.token()? else {
                return Err(self.error("G_ParseSpawnVars: EOF without closing brace", None));
            };
            if key.value.starts_with('}') {
                return Ok(Some(SpawnVariables::new(entries)?));
            }
            let Some(value) = self.token()? else {
                return Err(self.error("G_ParseSpawnVars: EOF without closing brace", None));
            };
            if value.value.starts_with('}') {
                return Err(self.error("G_ParseSpawnVars: closing brace without data", Some(&value)));
            }
            if entries.len() == MAX_SPAWN_VARS {
                return Err(self.error("G_ParseSpawnVars: MAX_SPAWN_VARS", Some(&key)));
            }
            characters += key.value.len() + value.value.len() + 2;
            if characters > MAX_SPAWN_VARS_CHARS {
                return Err(self.error("G_AddSpawnVarToken: MAX_SPAWN_CHARS", Some(&value)));
            }
            entries.push(SpawnPair {
                key: key.value,
                value: value.value,
            });
        }
    }
}

/// Parse one spawn field (`G_ParseField`); false means no source field matched.
pub fn parse_spawn_field(
    key: &str,
    value: &str,
    entity: &mut GameEntity,
    memory: &mut GameMemory,
) -> Result<bool, Q3GameError> {
    let field = String::from_utf8_lossy(&spawn_lower(key)).into_owned();
    match field.as_str() {
        "classname" => entity.set_classname(Some(new_spawn_string(value, memory)?)),
        "model" => entity.model = Some(new_spawn_string(value, memory)?),
        "model2" => entity.model2 = Some(new_spawn_string(value, memory)?),
        "target" => entity.target = Some(new_spawn_string(value, memory)?),
        "targetname" => entity.targetname = Some(new_spawn_string(value, memory)?),
        "message" => entity.message = Some(new_spawn_string(value, memory)?),
        "team" => entity.team = Some(new_spawn_string(value, memory)?),
        "targetshadername" => entity.target_shader_name = Some(new_spawn_string(value, memory)?),
        "targetshadernewname" => entity.target_shader_new_name = Some(new_spawn_string(value, memory)?),
        "spawnflags" => entity.spawnflags = game_atoi(value)?,
        "count" => entity.count = game_atoi(value)?,
        "health" => entity.health = game_atoi(value)?,
        "dmg" => entity.damage = game_atoi(value)?,
        "speed" => entity.speed = game_atof(value)?,
        "wait" => entity.wait = game_atof(value)?,
        "random" => entity.random = game_atof(value)?,
        "origin" => entity.s.origin = scan_game_vector(value)?,
        "angles" => entity.s.angles = scan_game_vector(value)?,
        "angle" => entity.s.angles = vec3(0.0, game_atof(value)?, 0.0),
        "light" => {}
        _ => return Ok(false),
    }
    Ok(true)
}

/// Spawn handler (`SpawnHandler`).
pub type SpawnHandler =
    Rc<dyn Fn(&mut dyn Q3Driver, &mut dyn SpawnServices, usize, &SpawnVariables) -> Result<(), Q3GameError>>;

/// Spawn handler table.
#[derive(Default)]
pub struct SpawnHandlerTable {
    map: HashMap<String, SpawnHandler>,
}

impl SpawnHandlerTable {
    /// New table.
    #[must_use]
    pub fn new() -> Self {
        Self { map: HashMap::new() }
    }

    /// Insert a handler.
    pub fn insert(&mut self, classname: &str, handler: SpawnHandler) {
        self.map.insert(classname.to_string(), handler);
    }

    /// Look up a handler.
    #[must_use]
    pub fn get(&self, classname: &str) -> Option<SpawnHandler> {
        self.map.get(classname).cloned()
    }
}

impl std::fmt::Debug for SpawnHandlerTable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut keys: Vec<&str> = self.map.keys().map(String::as_str).collect();
        keys.sort_unstable();
        f.debug_struct("SpawnHandlerTable").field("handlers", &keys).finish()
    }
}

/// Spawn services (`SpawnContext` minus the pool, which the driver owns).
pub trait SpawnServices {
    /// Game memory.
    fn memory(&mut self) -> &mut GameMemory;
    /// Product.
    fn product(&self) -> Q3Product;
    /// Game type number.
    fn game_type(&self) -> i32;
    /// Handler table.
    fn handlers(&self) -> &SpawnHandlerTable;
    /// Spawn an item (`spawnItem`).
    fn spawn_item(
        &mut self,
        driver: &mut dyn Q3Driver,
        slot: usize,
        item: usize,
        variables: &SpawnVariables,
    ) -> Result<(), Q3GameError>;
    /// Warn (`warn`).
    fn warn(&mut self, message: &str);
}

/// Spawn filter reason (`SpawnFilter`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpawnFilter {
    /// Not single player.
    Notsingle,
    /// Not team.
    Notteam,
    /// Not free-for-all.
    Notfree,
    /// Not Team Arena.
    Notta,
    /// Not base Quake III.
    Notq3a,
    /// Game type list.
    Gametype,
}

impl SpawnFilter {
    /// Source key.
    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            Self::Notsingle => "notsingle",
            Self::Notteam => "notteam",
            Self::Notfree => "notfree",
            Self::Notta => "notta",
            Self::Notq3a => "notq3a",
            Self::Gametype => "gametype",
        }
    }
}

/// Spawn dispatch route.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpawnRoute {
    /// Item.
    Item,
    /// Handler.
    Handler,
}

/// Spawn outcome (`SpawnOutcome`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpawnOutcome {
    /// Dispatched.
    Dispatched {
        /// Route.
        route: SpawnRoute,
        /// Slot.
        slot: usize,
        /// Classname.
        classname: String,
    },
    /// Filtered.
    Filtered {
        /// Slot.
        slot: usize,
        /// Reason.
        reason: SpawnFilter,
    },
    /// Unknown classname.
    Unknown {
        /// Slot.
        slot: usize,
        /// Classname.
        classname: Option<String>,
    },
}

fn spawn_excluded(
    variables: &SpawnVariables,
    services: &mut dyn SpawnServices,
) -> Result<Option<SpawnFilter>, Q3GameError> {
    let game_type = services.game_type();
    if game_type == Q3GameType::SinglePlayer as i32 && variables.int("notsingle", "0")?.value != 0 {
        return Ok(Some(SpawnFilter::Notsingle));
    }
    let team_key = if game_type >= Q3GameType::Team as i32 {
        "notteam"
    } else {
        "notfree"
    };
    if variables.int(team_key, "0")?.value != 0 {
        return Ok(Some(if team_key == "notteam" {
            SpawnFilter::Notteam
        } else {
            SpawnFilter::Notfree
        }));
    }
    let product_key = if services.product() == Q3Product::Missionpack {
        "notta"
    } else {
        "notq3a"
    };
    if variables.int(product_key, "0")?.value != 0 {
        return Ok(Some(if product_key == "notta" {
            SpawnFilter::Notta
        } else {
            SpawnFilter::Notq3a
        }));
    }
    let gametype = variables.string("gametype", "");
    if gametype.present && (Q3GameType::Ffa as i32..Q3GameType::MaxGameType as i32).contains(&game_type) {
        let names = [
            "ffa",
            "tournament",
            "single",
            "team",
            "ctf",
            "oneflag",
            "obelisk",
            "harvester",
        ];
        let Some(name) = names.get(game_type as usize) else {
            return Err(range("No source gametype name"));
        };
        if !gametype.value.contains(name) {
            return Ok(Some(SpawnFilter::Gametype));
        }
    }
    Ok(None)
}

/// Spawn one entity from variables (`G_SpawnGEntityFromSpawnVars` / `G_CallSpawn`).
pub fn spawn_entity(
    variables: &SpawnVariables,
    driver: &mut dyn Q3Driver,
    services: &mut dyn SpawnServices,
) -> Result<SpawnOutcome, Q3GameError> {
    let slot = driver.pool().spawn_entity()?;
    for pair in &variables.entries {
        let parsed = {
            let entity = driver
                .pool()
                .entity_mut(slot)
                .ok_or_else(|| failure(format!("entity {slot} vanished after spawn")))?;
            parse_spawn_field(&pair.key, &pair.value, entity, services.memory())
        };
        if let Err(error) = parsed {
            if matches!(error, Q3GameError::Drop(_)) {
                return Err(error);
            }
            driver.pool().free_entity(slot);
            return Err(error);
        }
    }
    if let Some(reason) = spawn_excluded(variables, services)? {
        driver.pool().free_entity(slot);
        return Ok(SpawnOutcome::Filtered { slot, reason });
    }
    let origin = driver
        .pool()
        .entity(slot)
        .map(|entity| entity.s.origin)
        .unwrap_or_else(|| vec3(0.0, 0.0, 0.0));
    if let Some(entity) = driver.pool().entity_mut(slot) {
        entity.s.pos.base = origin;
        entity.r.current_origin = origin;
    }
    let classname = driver
        .pool()
        .entity(slot)
        .and_then(|entity| entity.classname_value().map(str::to_string));
    let Some(classname) = classname else {
        services.warn("G_CallSpawn: NULL classname\n");
        driver.pool().free_entity(slot);
        return Ok(SpawnOutcome::Unknown { slot, classname: None });
    };
    let mut item_index = None;
    for index in 0..driver.item_count() {
        if driver
            .item_at(index)
            .is_some_and(|item| item.class_name.as_deref() == Some(classname.as_str()))
        {
            item_index = Some(index);
            break;
        }
    }
    if let Some(item) = item_index {
        services.spawn_item(driver, slot, item, variables)?;
        return Ok(SpawnOutcome::Dispatched {
            route: SpawnRoute::Item,
            slot,
            classname,
        });
    }
    if let Some(handler) = services.handlers().get(&classname) {
        handler(driver, services, slot, variables)?;
        return Ok(SpawnOutcome::Dispatched {
            route: SpawnRoute::Handler,
            slot,
            classname,
        });
    }
    services.warn(&format!("{classname} doesn't have a spawn function\n"));
    driver.pool().free_entity(slot);
    Ok(SpawnOutcome::Unknown {
        slot,
        classname: Some(classname),
    })
}

/// Worldspawn music configstring.
pub const WORLDSPAWN_CS_MUSIC: i32 = 2;
/// Worldspawn message configstring.
pub const WORLDSPAWN_CS_MESSAGE: i32 = 3;
/// Worldspawn motd configstring.
pub const WORLDSPAWN_CS_MOTD: i32 = 4;
/// Worldspawn warmup configstring.
pub const WORLDSPAWN_CS_WARMUP: i32 = 5;
/// Worldspawn game configstring.
pub const WORLDSPAWN_CS_GAME: i32 = 20;
/// Worldspawn start-time configstring.
pub const WORLDSPAWN_CS_START_TIME: i32 = 21;

/// Worldspawn state (`WorldspawnContext` fields).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorldspawnState {
    /// Start time.
    pub start_time: i32,
    /// Message of the day.
    pub motd: String,
    /// Restarted flag.
    pub restarted: i32,
    /// Warmup flag.
    pub do_warmup: i32,
    /// Warmup time (mutable).
    pub warmup_time: i32,
}

/// Spawn the world entity (`SP_worldspawn`).
pub fn spawn_world(
    variables: &SpawnVariables,
    driver: &mut dyn Q3Driver,
    world: &mut WorldspawnState,
) -> Result<(), Q3GameError> {
    if String::from_utf8_lossy(&spawn_lower(&variables.string("classname", "").value)) != "worldspawn" {
        return Err(failure("SP_worldspawn: The first entity isn't 'worldspawn'"));
    }
    driver.set_configstring(WORLDSPAWN_CS_GAME, "baseq3-1");
    driver.set_configstring(WORLDSPAWN_CS_START_TIME, &world.start_time.to_string());
    driver.set_configstring(WORLDSPAWN_CS_MUSIC, &variables.string("music", "").value);
    driver.set_configstring(WORLDSPAWN_CS_MESSAGE, &variables.string("message", "").value);
    driver.set_configstring(WORLDSPAWN_CS_MOTD, &world.motd.clone());
    driver.set_cvar("g_gravity", &variables.string("gravity", "800").value);
    driver.set_cvar("g_enableDust", &variables.string("enableDust", "0").value);
    driver.set_cvar("g_enableBreath", &variables.string("enableBreath", "0").value);
    let world_entity = driver
        .pool()
        .entity_mut(ENTITYNUM_WORLD)
        .ok_or_else(|| failure("SP_worldspawn: missing world entity"))?;
    world_entity.s.number = ENTITYNUM_WORLD as i32;
    world_entity.set_classname(Some("worldspawn".to_string()));
    driver.set_configstring(WORLDSPAWN_CS_WARMUP, "");
    if world.restarted != 0 {
        driver.set_cvar("g_restarted", "0");
        world.warmup_time = 0;
    } else if world.do_warmup != 0 {
        world.warmup_time = -1;
        driver.set_configstring(WORLDSPAWN_CS_WARMUP, "-1");
        driver.log("Warmup:\n");
    }
    Ok(())
}

/// Spawn report (`SpawnReport`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnReport {
    /// World variables.
    pub world_variables: SpawnVariables,
    /// Outcomes.
    pub outcomes: Vec<SpawnOutcome>,
}

/// Parse and dispatch entities (`spawnEntities`).
pub fn spawn_entities(
    text: &str,
    driver: &mut dyn Q3Driver,
    services: &mut dyn SpawnServices,
    world: &mut WorldspawnState,
    source: &str,
) -> Result<SpawnReport, Q3GameError> {
    let mut parser = SpawnParser::new(text, source)?;
    let Some(world_variables) = parser.next()? else {
        return Err(failure("SpawnEntities: no entities"));
    };
    spawn_world(&world_variables, driver, world)?;
    let mut outcomes = Vec::new();
    while let Some(variables) = parser.next()? {
        outcomes.push(spawn_entity(&variables, driver, services)?);
    }
    Ok(SpawnReport {
        world_variables,
        outcomes,
    })
}

// ---------------------------------------------------------------------------
// use-participant.ts: participant helpers
// ---------------------------------------------------------------------------

/// Use-participant services (`UseParticipantServices`).
pub trait UseParticipantServices {
    /// Whether an actor is live.
    fn live(&self, actor: &ActorId) -> bool;
    /// Whether an actor is a player.
    fn is_player(&self, actor: &ActorId) -> bool;
    /// Native slot for an actor.
    fn native(&self, actor: &ActorId) -> Option<usize>;
    /// Emit an actor event.
    fn event(&mut self, actor: &ActorId, event: i32, parameter: i32);
}

/// Actor for a participant (`useActor`).
pub fn use_actor(pool: &dyn EntityPool, participant: &Participant) -> Result<ActorId, Q3GameError> {
    match participant {
        Participant::Entity(slot) => pool.entity(*slot).map(|entity| entity.actor.clone()).ok_or_else(|| {
            failure(format!(
                "entity {slot} does not belong to its entity pool or was replaced"
            ))
        }),
        Participant::SharedActor(actor) => Ok(actor.clone()),
    }
}

/// Require an activator (`requireUseParticipant`).
pub fn require_use_participant(participant: Option<Participant>) -> Result<Participant, Q3GameError> {
    participant.ok_or_else(|| failure("Target handler requires an activator"))
}

/// Client entity for a participant (`useClient`).
pub fn use_client(driver: &mut dyn Q3Driver, participant: &Participant) -> Result<Option<usize>, Q3GameError> {
    match participant {
        Participant::Entity(slot) => {
            let slot = *slot;
            Ok(
                if driver.pool().entity(slot).is_some_and(|entity| entity.client.is_some()) {
                    Some(slot)
                } else {
                    None
                },
            )
        }
        Participant::SharedActor(actor) => {
            if !driver.actor_live(actor) || !driver.actor_is_player(actor) {
                return Ok(None);
            }
            let native = driver.native_slot(actor);
            match native {
                Some(slot) if driver.pool().entity(slot).is_some_and(|entity| entity.client.is_some()) => {
                    Ok(Some(slot))
                }
                _ => Err(failure("Admitted Q3 map player has no native client behavior record")),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// save-values.ts: value structs with capture/restore/read
// ---------------------------------------------------------------------------

fn vec_to_json(value: Vec3) -> SaveJson {
    obj(vec![
        ("x", num(f64::from(value.x))),
        ("y", num(f64::from(value.y))),
        ("z", num(f64::from(value.z))),
    ])
}

fn vec_from_reader(reader: &SaveReader) -> Result<Vec3, Q3GameError> {
    Ok(crate::value::read_vector(reader.clone())?)
}

fn opt_str_to_json(value: Option<&str>) -> SaveJson {
    value.map_or(SaveJson::Null, str)
}

fn num_i32(value: i32) -> SaveJson {
    int(i64::from(value))
}

fn num_f32(value: f32) -> SaveJson {
    num(f64::from(value))
}

fn read_i32(reader: &SaveReader, field: &str) -> Result<i32, Q3GameError> {
    #[allow(clippy::cast_possible_truncation)]
    Ok(reader.field(field).number()? as i32)
}

fn read_f32(reader: &SaveReader, field: &str) -> Result<f32, Q3GameError> {
    #[allow(clippy::cast_possible_truncation)]
    Ok(reader.field(field).number()? as f32)
}

fn read_opt_string(reader: &SaveReader, field: &str) -> Result<Option<String>, Q3GameError> {
    Ok(reader.field(field).nullable(|value| value.string())?)
}

fn read_vec(reader: &SaveReader, field: &str) -> Result<Vec3, Q3GameError> {
    vec_from_reader(&reader.field(field))
}

/// Entity values (`EntityValues`).
#[derive(Debug, Clone, PartialEq)]
pub struct EntityValues {
    /// Spawn flags.
    pub spawnflags: i32,
    /// Never free.
    pub never_free: bool,
    /// Flags.
    pub flags: i32,
    /// Model.
    pub model: Option<String>,
    /// Model 2.
    pub model2: Option<String>,
    /// Free time.
    pub freetime: i32,
    /// Event time.
    pub event_time: i32,
    /// Free after event.
    pub free_after_event: bool,
    /// Unlink after event.
    pub unlink_after_event: bool,
    /// Physics object.
    pub physics_object: bool,
    /// Physics bounce.
    pub physics_bounce: i32,
    /// Clip mask.
    pub clipmask: i32,
    /// Mover state.
    pub mover_state: i32,
    /// Sound position 1.
    pub sound_pos1: i32,
    /// Sound 1 to 2.
    pub sound1to2: i32,
    /// Sound 2 to 1.
    pub sound2to1: i32,
    /// Sound position 2.
    pub sound_pos2: i32,
    /// Sound loop.
    pub sound_loop: i32,
    /// Position 1.
    pub pos1: Vec3,
    /// Position 2.
    pub pos2: Vec3,
    /// Message.
    pub message: Option<String>,
    /// Timestamp.
    pub timestamp: i32,
    /// Angle.
    pub angle: f32,
    /// Target.
    pub target: Option<String>,
    /// Target name.
    pub targetname: Option<String>,
    /// Team.
    pub team: Option<String>,
    /// Target shader name.
    pub target_shader_name: Option<String>,
    /// Target shader new name.
    pub target_shader_new_name: Option<String>,
    /// Speed.
    pub speed: f32,
    /// Move direction.
    pub movedir: Vec3,
    /// Pain debounce time.
    pub pain_debounce_time: i32,
    /// Fly sound debounce time.
    pub fly_sound_debounce_time: i32,
    /// Last move time.
    pub last_move_time: i32,
    /// Damage.
    pub damage: i32,
    /// Splash damage.
    pub splash_damage: i32,
    /// Splash radius.
    pub splash_radius: i32,
    /// Means of death.
    pub method_of_death: i32,
    /// Splash means of death.
    pub splash_method_of_death: i32,
    /// Count.
    pub count: i32,
    /// Kamikaze time.
    pub kamikaze_time: i32,
    /// Kamikaze shock time.
    pub kamikaze_shock_time: i32,
    /// Water type.
    pub watertype: i32,
    /// Water level.
    pub waterlevel: i32,
    /// Noise index.
    pub noise_index: i32,
    /// Wait.
    pub wait: f32,
    /// Random.
    pub random: f32,
}

/// Capture entity values (`captureEntityValues`).
#[must_use]
pub fn capture_entity_values(source: &GameEntity) -> EntityValues {
    EntityValues {
        spawnflags: source.spawnflags,
        never_free: source.never_free,
        flags: source.flags,
        model: source.model.clone(),
        model2: source.model2.clone(),
        freetime: source.freetime,
        event_time: source.event_time,
        free_after_event: source.free_after_event,
        unlink_after_event: source.unlink_after_event,
        physics_object: source.physics_object,
        physics_bounce: source.physics_bounce,
        clipmask: source.clipmask,
        mover_state: source.mover_state,
        sound_pos1: source.sound_pos1,
        sound1to2: source.sound1to2,
        sound2to1: source.sound2to1,
        sound_pos2: source.sound_pos2,
        sound_loop: source.sound_loop,
        pos1: source.pos1,
        pos2: source.pos2,
        message: source.message.clone(),
        timestamp: source.timestamp,
        angle: source.angle,
        target: source.target.clone(),
        targetname: source.targetname.clone(),
        team: source.team.clone(),
        target_shader_name: source.target_shader_name.clone(),
        target_shader_new_name: source.target_shader_new_name.clone(),
        speed: source.speed,
        movedir: source.movedir,
        pain_debounce_time: source.pain_debounce_time,
        fly_sound_debounce_time: source.fly_sound_debounce_time,
        last_move_time: source.last_move_time,
        damage: source.damage,
        splash_damage: source.splash_damage,
        splash_radius: source.splash_radius,
        method_of_death: source.method_of_death,
        splash_method_of_death: source.splash_method_of_death,
        count: source.count,
        kamikaze_time: source.kamikaze_time,
        kamikaze_shock_time: source.kamikaze_shock_time,
        watertype: source.watertype,
        waterlevel: source.waterlevel,
        noise_index: source.noise_index,
        wait: source.wait,
        random: source.random,
    }
}

/// Restore entity values (`restoreEntityValues`).
pub fn restore_entity_values(target: &mut GameEntity, state: &EntityValues) {
    target.spawnflags = state.spawnflags;
    target.never_free = state.never_free;
    target.flags = state.flags;
    target.model = state.model.clone();
    target.model2 = state.model2.clone();
    target.freetime = state.freetime;
    target.event_time = state.event_time;
    target.free_after_event = state.free_after_event;
    target.unlink_after_event = state.unlink_after_event;
    target.physics_object = state.physics_object;
    target.physics_bounce = state.physics_bounce;
    target.clipmask = state.clipmask;
    target.mover_state = state.mover_state;
    target.sound_pos1 = state.sound_pos1;
    target.sound1to2 = state.sound1to2;
    target.sound2to1 = state.sound2to1;
    target.sound_pos2 = state.sound_pos2;
    target.sound_loop = state.sound_loop;
    target.pos1 = state.pos1;
    target.pos2 = state.pos2;
    target.message = state.message.clone();
    target.timestamp = state.timestamp;
    target.angle = state.angle;
    target.target = state.target.clone();
    target.targetname = state.targetname.clone();
    target.team = state.team.clone();
    target.target_shader_name = state.target_shader_name.clone();
    target.target_shader_new_name = state.target_shader_new_name.clone();
    target.speed = state.speed;
    target.movedir = state.movedir;
    target.pain_debounce_time = state.pain_debounce_time;
    target.fly_sound_debounce_time = state.fly_sound_debounce_time;
    target.last_move_time = state.last_move_time;
    target.damage = state.damage;
    target.splash_damage = state.splash_damage;
    target.splash_radius = state.splash_radius;
    target.method_of_death = state.method_of_death;
    target.splash_method_of_death = state.splash_method_of_death;
    target.count = state.count;
    target.kamikaze_time = state.kamikaze_time;
    target.kamikaze_shock_time = state.kamikaze_shock_time;
    target.watertype = state.watertype;
    target.waterlevel = state.waterlevel;
    target.noise_index = state.noise_index;
    target.wait = state.wait;
    target.random = state.random;
}

/// Encode entity values.
#[must_use]
pub fn entity_values_to_json(state: &EntityValues) -> SaveJson {
    obj(vec![
        ("spawnflags", num_i32(state.spawnflags)),
        ("neverFree", boolean(state.never_free)),
        ("flags", num_i32(state.flags)),
        ("model", opt_str_to_json(state.model.as_deref())),
        ("model2", opt_str_to_json(state.model2.as_deref())),
        ("freetime", num_i32(state.freetime)),
        ("eventTime", num_i32(state.event_time)),
        ("freeAfterEvent", boolean(state.free_after_event)),
        ("unlinkAfterEvent", boolean(state.unlink_after_event)),
        ("physicsObject", boolean(state.physics_object)),
        ("physicsBounce", num_i32(state.physics_bounce)),
        ("clipmask", num_i32(state.clipmask)),
        ("moverState", num_i32(state.mover_state)),
        ("soundPos1", num_i32(state.sound_pos1)),
        ("sound1to2", num_i32(state.sound1to2)),
        ("sound2to1", num_i32(state.sound2to1)),
        ("soundPos2", num_i32(state.sound_pos2)),
        ("soundLoop", num_i32(state.sound_loop)),
        ("pos1", vec_to_json(state.pos1)),
        ("pos2", vec_to_json(state.pos2)),
        ("message", opt_str_to_json(state.message.as_deref())),
        ("timestamp", num_i32(state.timestamp)),
        ("angle", num_f32(state.angle)),
        ("target", opt_str_to_json(state.target.as_deref())),
        ("targetname", opt_str_to_json(state.targetname.as_deref())),
        ("team", opt_str_to_json(state.team.as_deref())),
        ("targetShaderName", opt_str_to_json(state.target_shader_name.as_deref())),
        (
            "targetShaderNewName",
            opt_str_to_json(state.target_shader_new_name.as_deref()),
        ),
        ("speed", num_f32(state.speed)),
        ("movedir", vec_to_json(state.movedir)),
        ("painDebounceTime", num_i32(state.pain_debounce_time)),
        ("flySoundDebounceTime", num_i32(state.fly_sound_debounce_time)),
        ("lastMoveTime", num_i32(state.last_move_time)),
        ("damage", num_i32(state.damage)),
        ("splashDamage", num_i32(state.splash_damage)),
        ("splashRadius", num_i32(state.splash_radius)),
        ("methodOfDeath", num_i32(state.method_of_death)),
        ("splashMethodOfDeath", num_i32(state.splash_method_of_death)),
        ("count", num_i32(state.count)),
        ("kamikazeTime", num_i32(state.kamikaze_time)),
        ("kamikazeShockTime", num_i32(state.kamikaze_shock_time)),
        ("watertype", num_i32(state.watertype)),
        ("waterlevel", num_i32(state.waterlevel)),
        ("noiseIndex", num_i32(state.noise_index)),
        ("wait", num_f32(state.wait)),
        ("random", num_f32(state.random)),
    ])
}

/// Read entity values (`readEntityValues`).
pub fn read_entity_values(reader: &SaveReader) -> Result<EntityValues, Q3GameError> {
    Ok(EntityValues {
        spawnflags: read_i32(reader, "spawnflags")?,
        never_free: reader.field("neverFree").boolean()?,
        flags: read_i32(reader, "flags")?,
        model: read_opt_string(reader, "model")?,
        model2: read_opt_string(reader, "model2")?,
        freetime: read_i32(reader, "freetime")?,
        event_time: read_i32(reader, "eventTime")?,
        free_after_event: reader.field("freeAfterEvent").boolean()?,
        unlink_after_event: reader.field("unlinkAfterEvent").boolean()?,
        physics_object: reader.field("physicsObject").boolean()?,
        physics_bounce: read_i32(reader, "physicsBounce")?,
        clipmask: read_i32(reader, "clipmask")?,
        mover_state: read_i32(reader, "moverState")?,
        sound_pos1: read_i32(reader, "soundPos1")?,
        sound1to2: read_i32(reader, "sound1to2")?,
        sound2to1: read_i32(reader, "sound2to1")?,
        sound_pos2: read_i32(reader, "soundPos2")?,
        sound_loop: read_i32(reader, "soundLoop")?,
        pos1: read_vec(reader, "pos1")?,
        pos2: read_vec(reader, "pos2")?,
        message: read_opt_string(reader, "message")?,
        timestamp: read_i32(reader, "timestamp")?,
        angle: read_f32(reader, "angle")?,
        target: read_opt_string(reader, "target")?,
        targetname: read_opt_string(reader, "targetname")?,
        team: read_opt_string(reader, "team")?,
        target_shader_name: read_opt_string(reader, "targetShaderName")?,
        target_shader_new_name: read_opt_string(reader, "targetShaderNewName")?,
        speed: read_f32(reader, "speed")?,
        movedir: read_vec(reader, "movedir")?,
        pain_debounce_time: read_i32(reader, "painDebounceTime")?,
        fly_sound_debounce_time: read_i32(reader, "flySoundDebounceTime")?,
        last_move_time: read_i32(reader, "lastMoveTime")?,
        damage: read_i32(reader, "damage")?,
        splash_damage: read_i32(reader, "splashDamage")?,
        splash_radius: read_i32(reader, "splashRadius")?,
        method_of_death: read_i32(reader, "methodOfDeath")?,
        splash_method_of_death: read_i32(reader, "splashMethodOfDeath")?,
        count: read_i32(reader, "count")?,
        kamikaze_time: read_i32(reader, "kamikazeTime")?,
        kamikaze_shock_time: read_i32(reader, "kamikazeShockTime")?,
        watertype: read_i32(reader, "watertype")?,
        waterlevel: read_i32(reader, "waterlevel")?,
        noise_index: read_i32(reader, "noiseIndex")?,
        wait: read_f32(reader, "wait")?,
        random: read_f32(reader, "random")?,
    })
}

/// Client values (`ClientValues`).
#[derive(Debug, Clone, PartialEq)]
pub struct ClientValues {
    /// Ready to exit.
    pub ready_to_exit: bool,
    /// Noclip.
    pub noclip: bool,
    /// Last command time.
    pub last_cmd_time: i32,
    /// Buttons.
    pub buttons: i32,
    /// Old buttons.
    pub old_buttons: i32,
    /// Latched buttons.
    pub latched_buttons: i32,
    /// Old origin.
    pub old_origin: Vec3,
    /// Damage armor.
    pub damage_armor: i32,
    /// Damage blood.
    pub damage_blood: i32,
    /// Damage knockback.
    pub damage_knockback: i32,
    /// Damage from.
    pub damage_from: Vec3,
    /// Damage from world.
    pub damage_from_world: bool,
    /// Accurate count.
    pub accurate_count: i32,
    /// Accuracy shots.
    pub accuracy_shots: i32,
    /// Accuracy hits.
    pub accuracy_hits: i32,
    /// Last killed client.
    pub last_killed_client: i32,
    /// Last hurt client.
    pub last_hurt_client: i32,
    /// Last hurt means of death.
    pub last_hurt_mod: i32,
    /// Respawn time.
    pub respawn_time: i32,
    /// Inactivity time.
    pub inactivity_time: i32,
    /// Inactivity warning.
    pub inactivity_warning: bool,
    /// Reward time.
    pub reward_time: i32,
    /// Air out time.
    pub air_out_time: i32,
    /// Last kill time.
    pub last_kill_time: i32,
    /// Fire held.
    pub fire_held: bool,
    /// Switch team time.
    pub switch_team_time: i32,
    /// Time residual.
    pub time_residual: i32,
    /// Portal identifier.
    pub portal_id: i32,
    /// Invulnerability time.
    pub invulnerability_time: i32,
}

/// Capture client values (`captureClientValues`).
#[must_use]
pub fn capture_client_values(source: &GameClient) -> ClientValues {
    ClientValues {
        ready_to_exit: source.ready_to_exit,
        noclip: source.noclip,
        last_cmd_time: source.last_cmd_time,
        buttons: source.buttons,
        old_buttons: source.old_buttons,
        latched_buttons: source.latched_buttons,
        old_origin: source.old_origin,
        damage_armor: source.damage_armor,
        damage_blood: source.damage_blood,
        damage_knockback: source.damage_knockback,
        damage_from: source.damage_from,
        damage_from_world: source.damage_from_world,
        accurate_count: source.accurate_count,
        accuracy_shots: source.accuracy_shots,
        accuracy_hits: source.accuracy_hits,
        last_killed_client: source.last_killed_client,
        last_hurt_client: source.last_hurt_client,
        last_hurt_mod: source.last_hurt_mod,
        respawn_time: source.respawn_time,
        inactivity_time: source.inactivity_time,
        inactivity_warning: source.inactivity_warning,
        reward_time: source.reward_time,
        air_out_time: source.air_out_time,
        last_kill_time: source.last_kill_time,
        fire_held: source.fire_held,
        switch_team_time: source.switch_team_time,
        time_residual: source.time_residual,
        portal_id: source.portal_id,
        invulnerability_time: source.invulnerability_time,
    }
}

/// Restore client values (`restoreClientValues`).
pub fn restore_client_values(target: &mut GameClient, state: &ClientValues) {
    target.ready_to_exit = state.ready_to_exit;
    target.noclip = state.noclip;
    target.last_cmd_time = state.last_cmd_time;
    target.buttons = state.buttons;
    target.old_buttons = state.old_buttons;
    target.latched_buttons = state.latched_buttons;
    target.old_origin = state.old_origin;
    target.damage_armor = state.damage_armor;
    target.damage_blood = state.damage_blood;
    target.damage_knockback = state.damage_knockback;
    target.damage_from = state.damage_from;
    target.damage_from_world = state.damage_from_world;
    target.accurate_count = state.accurate_count;
    target.accuracy_shots = state.accuracy_shots;
    target.accuracy_hits = state.accuracy_hits;
    target.last_killed_client = state.last_killed_client;
    target.last_hurt_client = state.last_hurt_client;
    target.last_hurt_mod = state.last_hurt_mod;
    target.respawn_time = state.respawn_time;
    target.inactivity_time = state.inactivity_time;
    target.inactivity_warning = state.inactivity_warning;
    target.reward_time = state.reward_time;
    target.air_out_time = state.air_out_time;
    target.last_kill_time = state.last_kill_time;
    target.fire_held = state.fire_held;
    target.switch_team_time = state.switch_team_time;
    target.time_residual = state.time_residual;
    target.portal_id = state.portal_id;
    target.invulnerability_time = state.invulnerability_time;
}

/// Encode client values.
#[must_use]
pub fn client_values_to_json(state: &ClientValues) -> SaveJson {
    obj(vec![
        ("readyToExit", boolean(state.ready_to_exit)),
        ("noclip", boolean(state.noclip)),
        ("lastCmdTime", num_i32(state.last_cmd_time)),
        ("buttons", num_i32(state.buttons)),
        ("oldButtons", num_i32(state.old_buttons)),
        ("latchedButtons", num_i32(state.latched_buttons)),
        ("oldOrigin", vec_to_json(state.old_origin)),
        ("damageArmor", num_i32(state.damage_armor)),
        ("damageBlood", num_i32(state.damage_blood)),
        ("damageKnockback", num_i32(state.damage_knockback)),
        ("damageFrom", vec_to_json(state.damage_from)),
        ("damageFromWorld", boolean(state.damage_from_world)),
        ("accurateCount", num_i32(state.accurate_count)),
        ("accuracyShots", num_i32(state.accuracy_shots)),
        ("accuracyHits", num_i32(state.accuracy_hits)),
        ("lastKilledClient", num_i32(state.last_killed_client)),
        ("lastHurtClient", num_i32(state.last_hurt_client)),
        ("lastHurtMod", num_i32(state.last_hurt_mod)),
        ("respawnTime", num_i32(state.respawn_time)),
        ("inactivityTime", num_i32(state.inactivity_time)),
        ("inactivityWarning", boolean(state.inactivity_warning)),
        ("rewardTime", num_i32(state.reward_time)),
        ("airOutTime", num_i32(state.air_out_time)),
        ("lastKillTime", num_i32(state.last_kill_time)),
        ("fireHeld", boolean(state.fire_held)),
        ("switchTeamTime", num_i32(state.switch_team_time)),
        ("timeResidual", num_i32(state.time_residual)),
        ("portalID", num_i32(state.portal_id)),
        ("invulnerabilityTime", num_i32(state.invulnerability_time)),
    ])
}

/// Read client values (`readClientValues`).
pub fn read_client_values(reader: &SaveReader) -> Result<ClientValues, Q3GameError> {
    Ok(ClientValues {
        ready_to_exit: reader.field("readyToExit").boolean()?,
        noclip: reader.field("noclip").boolean()?,
        last_cmd_time: read_i32(reader, "lastCmdTime")?,
        buttons: read_i32(reader, "buttons")?,
        old_buttons: read_i32(reader, "oldButtons")?,
        latched_buttons: read_i32(reader, "latchedButtons")?,
        old_origin: read_vec(reader, "oldOrigin")?,
        damage_armor: read_i32(reader, "damageArmor")?,
        damage_blood: read_i32(reader, "damageBlood")?,
        damage_knockback: read_i32(reader, "damageKnockback")?,
        damage_from: read_vec(reader, "damageFrom")?,
        damage_from_world: reader.field("damageFromWorld").boolean()?,
        accurate_count: read_i32(reader, "accurateCount")?,
        accuracy_shots: read_i32(reader, "accuracyShots")?,
        accuracy_hits: read_i32(reader, "accuracyHits")?,
        last_killed_client: read_i32(reader, "lastKilledClient")?,
        last_hurt_client: read_i32(reader, "lastHurtClient")?,
        last_hurt_mod: read_i32(reader, "lastHurtMod")?,
        respawn_time: read_i32(reader, "respawnTime")?,
        inactivity_time: read_i32(reader, "inactivityTime")?,
        inactivity_warning: reader.field("inactivityWarning").boolean()?,
        reward_time: read_i32(reader, "rewardTime")?,
        air_out_time: read_i32(reader, "airOutTime")?,
        last_kill_time: read_i32(reader, "lastKillTime")?,
        fire_held: reader.field("fireHeld").boolean()?,
        switch_team_time: read_i32(reader, "switchTeamTime")?,
        time_residual: read_i32(reader, "timeResidual")?,
        portal_id: read_i32(reader, "portalID")?,
        invulnerability_time: read_i32(reader, "invulnerabilityTime")?,
    })
}

/// Player values (`PlayerValues`).
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerValues {
    /// Command time.
    pub command_time: i32,
    /// Movement type.
    pub pm_type: i32,
    /// Bob cycle.
    pub bob_cycle: i32,
    /// Move flags.
    pub pm_flags: i32,
    /// Move time.
    pub pm_time: i32,
    /// Weapon time.
    pub weapon_time: i32,
    /// Gravity.
    pub gravity: i32,
    /// Speed.
    pub speed: i32,
    /// Delta angles.
    pub delta_angles: [i32; 3],
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
    /// Entity flags.
    pub e_flags: i32,
    /// Event sequence.
    pub event_sequence: i32,
    /// External event.
    pub external_event: i32,
    /// External event parameter.
    pub external_event_parm: i32,
    /// External event time.
    pub external_event_time: i32,
    /// Client number.
    pub client_num: i32,
    /// Weapon.
    pub weapon: i32,
    /// Weapon state.
    pub weapon_state: i32,
    /// View angles.
    pub viewangles: Vec3,
    /// View height.
    pub viewheight: f32,
    /// Damage event.
    pub damage_event: i32,
    /// Damage yaw.
    pub damage_yaw: i32,
    /// Damage pitch.
    pub damage_pitch: i32,
    /// Damage count.
    pub damage_count: i32,
    /// Generic 1.
    pub generic1: i32,
    /// Loop sound.
    pub loop_sound: i32,
    /// Jump pad entity.
    pub jumppad_ent: i32,
    /// Ping.
    pub ping: i32,
    /// Pmove frame count.
    pub pmove_framecount: i32,
    /// Jump pad frame.
    pub jumppad_frame: i32,
    /// Entity event sequence.
    pub entity_event_sequence: i32,
}

/// Capture player values (`capturePlayerValues`).
#[must_use]
pub fn capture_player_values(source: &Q3PlayerState) -> PlayerValues {
    PlayerValues {
        command_time: source.command_time,
        pm_type: source.pm_type,
        bob_cycle: source.bob_cycle,
        pm_flags: source.pm_flags,
        pm_time: source.pm_time,
        weapon_time: source.weapon_time,
        gravity: source.gravity,
        speed: source.speed,
        delta_angles: source.delta_angles,
        ground_entity_num: source.ground_entity_num,
        legs_timer: source.legs_timer,
        legs_anim: source.legs_anim,
        torso_timer: source.torso_timer,
        torso_anim: source.torso_anim,
        movement_dir: source.movement_dir,
        grapple_point: source.grapple_point,
        e_flags: source.e_flags,
        event_sequence: source.event_sequence,
        external_event: source.external_event,
        external_event_parm: source.external_event_parm,
        external_event_time: source.external_event_time,
        client_num: source.client_num,
        weapon: source.weapon,
        weapon_state: source.weapon_state,
        viewangles: source.viewangles,
        viewheight: source.viewheight,
        damage_event: source.damage_event,
        damage_yaw: source.damage_yaw,
        damage_pitch: source.damage_pitch,
        damage_count: source.damage_count,
        generic1: source.generic1,
        loop_sound: source.loop_sound,
        jumppad_ent: source.jumppad_ent,
        ping: source.ping,
        pmove_framecount: source.pmove_framecount,
        jumppad_frame: source.jumppad_frame,
        entity_event_sequence: source.entity_event_sequence,
    }
}

/// Restore player values (`restorePlayerValues`).
pub fn restore_player_values(target: &mut Q3PlayerState, state: &PlayerValues) {
    target.command_time = state.command_time;
    target.pm_type = state.pm_type;
    target.bob_cycle = state.bob_cycle;
    target.pm_flags = state.pm_flags;
    target.pm_time = state.pm_time;
    target.weapon_time = state.weapon_time;
    target.gravity = state.gravity;
    target.speed = state.speed;
    target.delta_angles = state.delta_angles;
    target.ground_entity_num = state.ground_entity_num;
    target.legs_timer = state.legs_timer;
    target.legs_anim = state.legs_anim;
    target.torso_timer = state.torso_timer;
    target.torso_anim = state.torso_anim;
    target.movement_dir = state.movement_dir;
    target.grapple_point = state.grapple_point;
    target.e_flags = state.e_flags;
    target.event_sequence = state.event_sequence;
    target.external_event = state.external_event;
    target.external_event_parm = state.external_event_parm;
    target.external_event_time = state.external_event_time;
    target.client_num = state.client_num;
    target.weapon = state.weapon;
    target.weapon_state = state.weapon_state;
    target.viewangles = state.viewangles;
    target.viewheight = state.viewheight;
    target.damage_event = state.damage_event;
    target.damage_yaw = state.damage_yaw;
    target.damage_pitch = state.damage_pitch;
    target.damage_count = state.damage_count;
    target.generic1 = state.generic1;
    target.loop_sound = state.loop_sound;
    target.jumppad_ent = state.jumppad_ent;
    target.ping = state.ping;
    target.pmove_framecount = state.pmove_framecount;
    target.jumppad_frame = state.jumppad_frame;
    target.entity_event_sequence = state.entity_event_sequence;
}

/// Encode player values.
#[must_use]
pub fn player_values_to_json(state: &PlayerValues) -> SaveJson {
    obj(vec![
        ("commandTime", num_i32(state.command_time)),
        ("pmType", num_i32(state.pm_type)),
        ("bobCycle", num_i32(state.bob_cycle)),
        ("pmFlags", num_i32(state.pm_flags)),
        ("pmTime", num_i32(state.pm_time)),
        ("weaponTime", num_i32(state.weapon_time)),
        ("gravity", num_i32(state.gravity)),
        ("speed", num_i32(state.speed)),
        (
            "deltaAngles",
            obj(vec![
                ("x", num_i32(state.delta_angles[0])),
                ("y", num_i32(state.delta_angles[1])),
                ("z", num_i32(state.delta_angles[2])),
            ]),
        ),
        ("groundEntityNum", num_i32(state.ground_entity_num)),
        ("legsTimer", num_i32(state.legs_timer)),
        ("legsAnim", num_i32(state.legs_anim)),
        ("torsoTimer", num_i32(state.torso_timer)),
        ("torsoAnim", num_i32(state.torso_anim)),
        ("movementDir", num_i32(state.movement_dir)),
        ("grapplePoint", vec_to_json(state.grapple_point)),
        ("eFlags", num_i32(state.e_flags)),
        ("eventSequence", num_i32(state.event_sequence)),
        ("externalEvent", num_i32(state.external_event)),
        ("externalEventParm", num_i32(state.external_event_parm)),
        ("externalEventTime", num_i32(state.external_event_time)),
        ("clientNum", num_i32(state.client_num)),
        ("weapon", num_i32(state.weapon)),
        ("weaponState", num_i32(state.weapon_state)),
        ("viewangles", vec_to_json(state.viewangles)),
        ("viewheight", num_f32(state.viewheight)),
        ("damageEvent", num_i32(state.damage_event)),
        ("damageYaw", num_i32(state.damage_yaw)),
        ("damagePitch", num_i32(state.damage_pitch)),
        ("damageCount", num_i32(state.damage_count)),
        ("generic1", num_i32(state.generic1)),
        ("loopSound", num_i32(state.loop_sound)),
        ("jumppadEnt", num_i32(state.jumppad_ent)),
        ("ping", num_i32(state.ping)),
        ("pmoveFramecount", num_i32(state.pmove_framecount)),
        ("jumppadFrame", num_i32(state.jumppad_frame)),
        ("entityEventSequence", num_i32(state.entity_event_sequence)),
    ])
}

/// Read player values (`readPlayerValues`).
pub fn read_player_values(reader: &SaveReader) -> Result<PlayerValues, Q3GameError> {
    Ok(PlayerValues {
        command_time: read_i32(reader, "commandTime")?,
        pm_type: read_i32(reader, "pmType")?,
        bob_cycle: read_i32(reader, "bobCycle")?,
        pm_flags: read_i32(reader, "pmFlags")?,
        pm_time: read_i32(reader, "pmTime")?,
        weapon_time: read_i32(reader, "weaponTime")?,
        gravity: read_i32(reader, "gravity")?,
        speed: read_i32(reader, "speed")?,
        delta_angles: {
            let angles = reader.field("deltaAngles");
            [
                read_i32(&angles, "x")?,
                read_i32(&angles, "y")?,
                read_i32(&angles, "z")?,
            ]
        },
        ground_entity_num: read_i32(reader, "groundEntityNum")?,
        legs_timer: read_i32(reader, "legsTimer")?,
        legs_anim: read_i32(reader, "legsAnim")?,
        torso_timer: read_i32(reader, "torsoTimer")?,
        torso_anim: read_i32(reader, "torsoAnim")?,
        movement_dir: read_i32(reader, "movementDir")?,
        grapple_point: read_vec(reader, "grapplePoint")?,
        e_flags: read_i32(reader, "eFlags")?,
        event_sequence: read_i32(reader, "eventSequence")?,
        external_event: read_i32(reader, "externalEvent")?,
        external_event_parm: read_i32(reader, "externalEventParm")?,
        external_event_time: read_i32(reader, "externalEventTime")?,
        client_num: read_i32(reader, "clientNum")?,
        weapon: read_i32(reader, "weapon")?,
        weapon_state: read_i32(reader, "weaponState")?,
        viewangles: read_vec(reader, "viewangles")?,
        viewheight: read_f32(reader, "viewheight")?,
        damage_event: read_i32(reader, "damageEvent")?,
        damage_yaw: read_i32(reader, "damageYaw")?,
        damage_pitch: read_i32(reader, "damagePitch")?,
        damage_count: read_i32(reader, "damageCount")?,
        generic1: read_i32(reader, "generic1")?,
        loop_sound: read_i32(reader, "loopSound")?,
        jumppad_ent: read_i32(reader, "jumppadEnt")?,
        ping: read_i32(reader, "ping")?,
        pmove_framecount: read_i32(reader, "pmoveFramecount")?,
        jumppad_frame: read_i32(reader, "jumppadFrame")?,
        entity_event_sequence: read_i32(reader, "entityEventSequence")?,
    })
}

/// Persistant values (`PersistantValues`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistantValues {
    /// Connected.
    pub connected: i32,
    /// Local client.
    pub local_client: bool,
    /// Initial spawn.
    pub initial_spawn: bool,
    /// Predict item pickup.
    pub predict_item_pickup: bool,
    /// Pmove fixed.
    pub pmove_fixed: bool,
    /// Net name.
    pub netname: String,
    /// Max health.
    pub max_health: i32,
    /// Enter time.
    pub enter_time: i32,
    /// Vote count.
    pub vote_count: i32,
    /// Team vote count.
    pub team_vote_count: i32,
    /// Team info.
    pub team_info: bool,
}

/// Capture persistant values (`capturePersistantValues`).
#[must_use]
pub fn capture_persistant_values(source: &ClientPersistant) -> PersistantValues {
    PersistantValues {
        connected: source.connected,
        local_client: source.local_client,
        initial_spawn: source.initial_spawn,
        predict_item_pickup: source.predict_item_pickup,
        pmove_fixed: source.pmove_fixed,
        netname: source.netname.clone(),
        max_health: source.max_health,
        enter_time: source.enter_time,
        vote_count: source.vote_count,
        team_vote_count: source.team_vote_count,
        team_info: source.team_info,
    }
}

/// Restore persistant values (`restorePersistantValues`).
pub fn restore_persistant_values(target: &mut ClientPersistant, state: &PersistantValues) {
    target.connected = state.connected;
    target.local_client = state.local_client;
    target.initial_spawn = state.initial_spawn;
    target.predict_item_pickup = state.predict_item_pickup;
    target.pmove_fixed = state.pmove_fixed;
    target.netname = state.netname.clone();
    target.max_health = state.max_health;
    target.enter_time = state.enter_time;
    target.vote_count = state.vote_count;
    target.team_vote_count = state.team_vote_count;
    target.team_info = state.team_info;
}

/// Encode persistant values.
#[must_use]
pub fn persistant_values_to_json(state: &PersistantValues) -> SaveJson {
    obj(vec![
        ("connected", num_i32(state.connected)),
        ("localClient", boolean(state.local_client)),
        ("initialSpawn", boolean(state.initial_spawn)),
        ("predictItemPickup", boolean(state.predict_item_pickup)),
        ("pmoveFixed", boolean(state.pmove_fixed)),
        ("netname", str(&state.netname)),
        ("maxHealth", num_i32(state.max_health)),
        ("enterTime", num_i32(state.enter_time)),
        ("voteCount", num_i32(state.vote_count)),
        ("teamVoteCount", num_i32(state.team_vote_count)),
        ("teamInfo", boolean(state.team_info)),
    ])
}

/// Read persistant values (`readPersistantValues`).
pub fn read_persistant_values(reader: &SaveReader) -> Result<PersistantValues, Q3GameError> {
    Ok(PersistantValues {
        connected: read_i32(reader, "connected")?,
        local_client: reader.field("localClient").boolean()?,
        initial_spawn: reader.field("initialSpawn").boolean()?,
        predict_item_pickup: reader.field("predictItemPickup").boolean()?,
        pmove_fixed: reader.field("pmoveFixed").boolean()?,
        netname: reader.field("netname").string()?,
        max_health: read_i32(reader, "maxHealth")?,
        enter_time: read_i32(reader, "enterTime")?,
        vote_count: read_i32(reader, "voteCount")?,
        team_vote_count: read_i32(reader, "teamVoteCount")?,
        team_info: reader.field("teamInfo").boolean()?,
    })
}

/// Team values (`TeamValues`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamValues {
    /// State.
    pub state: i32,
    /// Location.
    pub location: i32,
    /// Captures.
    pub captures: i32,
    /// Base defense.
    pub base_defense: i32,
    /// Carrier defense.
    pub carrier_defense: i32,
    /// Flag recovery.
    pub flag_recovery: i32,
    /// Frag carrier.
    pub frag_carrier: i32,
    /// Assists.
    pub assists: i32,
    /// Last hurt carrier.
    pub last_hurt_carrier: i32,
    /// Last returned flag.
    pub last_returned_flag: i32,
    /// Flag since.
    pub flag_since: i32,
    /// Last fragged carrier.
    pub last_fragged_carrier: i32,
}

/// Capture team values (`captureTeamValues`).
#[must_use]
pub fn capture_team_values(source: &PlayerTeamState) -> TeamValues {
    TeamValues {
        state: source.state,
        location: source.location,
        captures: source.captures,
        base_defense: source.base_defense,
        carrier_defense: source.carrier_defense,
        flag_recovery: source.flag_recovery,
        frag_carrier: source.frag_carrier,
        assists: source.assists,
        last_hurt_carrier: source.last_hurt_carrier,
        last_returned_flag: source.last_returned_flag,
        flag_since: source.flag_since,
        last_fragged_carrier: source.last_fragged_carrier,
    }
}

/// Restore team values (`restoreTeamValues`).
pub fn restore_team_values(target: &mut PlayerTeamState, state: &TeamValues) {
    target.state = state.state;
    target.location = state.location;
    target.captures = state.captures;
    target.base_defense = state.base_defense;
    target.carrier_defense = state.carrier_defense;
    target.flag_recovery = state.flag_recovery;
    target.frag_carrier = state.frag_carrier;
    target.assists = state.assists;
    target.last_hurt_carrier = state.last_hurt_carrier;
    target.last_returned_flag = state.last_returned_flag;
    target.flag_since = state.flag_since;
    target.last_fragged_carrier = state.last_fragged_carrier;
}

/// Encode team values.
#[must_use]
pub fn team_values_to_json(state: &TeamValues) -> SaveJson {
    obj(vec![
        ("state", num_i32(state.state)),
        ("location", num_i32(state.location)),
        ("captures", num_i32(state.captures)),
        ("baseDefense", num_i32(state.base_defense)),
        ("carrierDefense", num_i32(state.carrier_defense)),
        ("flagRecovery", num_i32(state.flag_recovery)),
        ("fragCarrier", num_i32(state.frag_carrier)),
        ("assists", num_i32(state.assists)),
        ("lastHurtCarrier", num_i32(state.last_hurt_carrier)),
        ("lastReturnedFlag", num_i32(state.last_returned_flag)),
        ("flagSince", num_i32(state.flag_since)),
        ("lastFraggedCarrier", num_i32(state.last_fragged_carrier)),
    ])
}

/// Read team values (`readTeamValues`).
pub fn read_team_values(reader: &SaveReader) -> Result<TeamValues, Q3GameError> {
    Ok(TeamValues {
        state: read_i32(reader, "state")?,
        location: read_i32(reader, "location")?,
        captures: read_i32(reader, "captures")?,
        base_defense: read_i32(reader, "baseDefense")?,
        carrier_defense: read_i32(reader, "carrierDefense")?,
        flag_recovery: read_i32(reader, "flagRecovery")?,
        frag_carrier: read_i32(reader, "fragCarrier")?,
        assists: read_i32(reader, "assists")?,
        last_hurt_carrier: read_i32(reader, "lastHurtCarrier")?,
        last_returned_flag: read_i32(reader, "lastReturnedFlag")?,
        flag_since: read_i32(reader, "flagSince")?,
        last_fragged_carrier: read_i32(reader, "lastFraggedCarrier")?,
    })
}

/// Session values (`SessionValues`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionValues {
    /// Session team.
    pub session_team: i32,
    /// Spectator time.
    pub spectator_time: i32,
    /// Spectator state.
    pub spectator_state: i32,
    /// Spectator client.
    pub spectator_client: i32,
    /// Wins.
    pub wins: i32,
    /// Losses.
    pub losses: i32,
    /// Team leader.
    pub team_leader: i32,
}

/// Capture session values (`captureSessionValues`).
#[must_use]
pub fn capture_session_values(source: &ClientSession) -> SessionValues {
    SessionValues {
        session_team: source.session_team,
        spectator_time: source.spectator_time,
        spectator_state: source.spectator_state,
        spectator_client: source.spectator_client,
        wins: source.wins,
        losses: source.losses,
        team_leader: source.team_leader,
    }
}

/// Restore session values (`restoreSessionValues`).
pub fn restore_session_values(target: &mut ClientSession, state: &SessionValues) {
    target.session_team = state.session_team;
    target.spectator_time = state.spectator_time;
    target.spectator_state = state.spectator_state;
    target.spectator_client = state.spectator_client;
    target.wins = state.wins;
    target.losses = state.losses;
    target.team_leader = state.team_leader;
}

/// Encode session values.
#[must_use]
pub fn session_values_to_json(state: &SessionValues) -> SaveJson {
    obj(vec![
        ("sessionTeam", num_i32(state.session_team)),
        ("spectatorTime", num_i32(state.spectator_time)),
        ("spectatorState", num_i32(state.spectator_state)),
        ("spectatorClient", num_i32(state.spectator_client)),
        ("wins", num_i32(state.wins)),
        ("losses", num_i32(state.losses)),
        ("teamLeader", num_i32(state.team_leader)),
    ])
}

/// Read session values (`readSessionValues`).
pub fn read_session_values(reader: &SaveReader) -> Result<SessionValues, Q3GameError> {
    Ok(SessionValues {
        session_team: read_i32(reader, "sessionTeam")?,
        spectator_time: read_i32(reader, "spectatorTime")?,
        spectator_state: read_i32(reader, "spectatorState")?,
        spectator_client: read_i32(reader, "spectatorClient")?,
        wins: read_i32(reader, "wins")?,
        losses: read_i32(reader, "losses")?,
        team_leader: read_i32(reader, "teamLeader")?,
    })
}

/// Network values (`NetworkValues`).
#[derive(Debug, Clone, PartialEq)]
pub struct NetworkValues {
    /// Number.
    pub number: i32,
    /// Entity type.
    pub e_type: i32,
    /// Flags.
    pub e_flags: i32,
    /// Time.
    pub time: i32,
    /// Time 2.
    pub time2: i32,
    /// Origin.
    pub origin: Vec3,
    /// Origin 2.
    pub origin2: Vec3,
    /// Angles.
    pub angles: Vec3,
    /// Angles 2.
    pub angles2: Vec3,
    /// Other entity number.
    pub other_entity_num: i32,
    /// Other entity number 2.
    pub other_entity_num2: i32,
    /// Ground entity number.
    pub ground_entity_num: i32,
    /// Constant light.
    pub constant_light: i32,
    /// Loop sound.
    pub loop_sound: i32,
    /// Model index.
    pub modelindex: i32,
    /// Model index 2.
    pub modelindex2: i32,
    /// Client number.
    pub client_num: i32,
    /// Frame.
    pub frame: i32,
    /// Solid.
    pub solid: i32,
    /// Event.
    pub event: i32,
    /// Event parameter.
    pub event_parm: i32,
    /// Powerups.
    pub powerups: i32,
    /// Weapon.
    pub weapon: i32,
    /// Legs animation.
    pub legs_anim: i32,
    /// Torso animation.
    pub torso_anim: i32,
    /// Generic 1.
    pub generic1: i32,
}

/// Capture network values (`captureNetworkValues`).
#[must_use]
pub fn capture_network_values(source: &Q3EntityState) -> NetworkValues {
    NetworkValues {
        number: source.number,
        e_type: source.e_type,
        e_flags: source.e_flags,
        time: source.time,
        time2: source.time2,
        origin: source.origin,
        origin2: source.origin2,
        angles: source.angles,
        angles2: source.angles2,
        other_entity_num: source.other_entity_num,
        other_entity_num2: source.other_entity_num2,
        ground_entity_num: source.ground_entity_num,
        constant_light: source.constant_light,
        loop_sound: source.loop_sound,
        modelindex: source.modelindex,
        modelindex2: source.modelindex2,
        client_num: source.client_num,
        frame: source.frame,
        solid: source.solid,
        event: source.event,
        event_parm: source.event_parm,
        powerups: source.powerups,
        weapon: source.weapon,
        legs_anim: source.legs_anim,
        torso_anim: source.torso_anim,
        generic1: source.generic1,
    }
}

/// Restore network values (`restoreNetworkValues`).
pub fn restore_network_values(target: &mut Q3EntityState, state: &NetworkValues) {
    target.number = state.number;
    target.e_type = state.e_type;
    target.e_flags = state.e_flags;
    target.time = state.time;
    target.time2 = state.time2;
    target.origin = state.origin;
    target.origin2 = state.origin2;
    target.angles = state.angles;
    target.angles2 = state.angles2;
    target.other_entity_num = state.other_entity_num;
    target.other_entity_num2 = state.other_entity_num2;
    target.ground_entity_num = state.ground_entity_num;
    target.constant_light = state.constant_light;
    target.loop_sound = state.loop_sound;
    target.modelindex = state.modelindex;
    target.modelindex2 = state.modelindex2;
    target.client_num = state.client_num;
    target.frame = state.frame;
    target.solid = state.solid;
    target.event = state.event;
    target.event_parm = state.event_parm;
    target.powerups = state.powerups;
    target.weapon = state.weapon;
    target.legs_anim = state.legs_anim;
    target.torso_anim = state.torso_anim;
    target.generic1 = state.generic1;
}

/// Encode network values.
#[must_use]
pub fn network_values_to_json(state: &NetworkValues) -> SaveJson {
    obj(vec![
        ("number", num_i32(state.number)),
        ("eType", num_i32(state.e_type)),
        ("eFlags", num_i32(state.e_flags)),
        ("time", num_i32(state.time)),
        ("time2", num_i32(state.time2)),
        ("origin", vec_to_json(state.origin)),
        ("origin2", vec_to_json(state.origin2)),
        ("angles", vec_to_json(state.angles)),
        ("angles2", vec_to_json(state.angles2)),
        ("otherEntityNum", num_i32(state.other_entity_num)),
        ("otherEntityNum2", num_i32(state.other_entity_num2)),
        ("groundEntityNum", num_i32(state.ground_entity_num)),
        ("constantLight", num_i32(state.constant_light)),
        ("loopSound", num_i32(state.loop_sound)),
        ("modelindex", num_i32(state.modelindex)),
        ("modelindex2", num_i32(state.modelindex2)),
        ("clientNum", num_i32(state.client_num)),
        ("frame", num_i32(state.frame)),
        ("solid", num_i32(state.solid)),
        ("event", num_i32(state.event)),
        ("eventParm", num_i32(state.event_parm)),
        ("powerups", num_i32(state.powerups)),
        ("weapon", num_i32(state.weapon)),
        ("legsAnim", num_i32(state.legs_anim)),
        ("torsoAnim", num_i32(state.torso_anim)),
        ("generic1", num_i32(state.generic1)),
    ])
}

/// Read network values (`readNetworkValues`).
pub fn read_network_values(reader: &SaveReader) -> Result<NetworkValues, Q3GameError> {
    Ok(NetworkValues {
        number: read_i32(reader, "number")?,
        e_type: read_i32(reader, "eType")?,
        e_flags: read_i32(reader, "eFlags")?,
        time: read_i32(reader, "time")?,
        time2: read_i32(reader, "time2")?,
        origin: read_vec(reader, "origin")?,
        origin2: read_vec(reader, "origin2")?,
        angles: read_vec(reader, "angles")?,
        angles2: read_vec(reader, "angles2")?,
        other_entity_num: read_i32(reader, "otherEntityNum")?,
        other_entity_num2: read_i32(reader, "otherEntityNum2")?,
        ground_entity_num: read_i32(reader, "groundEntityNum")?,
        constant_light: read_i32(reader, "constantLight")?,
        loop_sound: read_i32(reader, "loopSound")?,
        modelindex: read_i32(reader, "modelindex")?,
        modelindex2: read_i32(reader, "modelindex2")?,
        client_num: read_i32(reader, "clientNum")?,
        frame: read_i32(reader, "frame")?,
        solid: read_i32(reader, "solid")?,
        event: read_i32(reader, "event")?,
        event_parm: read_i32(reader, "eventParm")?,
        powerups: read_i32(reader, "powerups")?,
        weapon: read_i32(reader, "weapon")?,
        legs_anim: read_i32(reader, "legsAnim")?,
        torso_anim: read_i32(reader, "torsoAnim")?,
        generic1: read_i32(reader, "generic1")?,
    })
}

/// Level values (`LevelValues`).
#[derive(Debug, Clone, PartialEq)]
pub struct LevelValues {
    /// Time.
    pub time: i32,
    /// Start time.
    pub start_time: i32,
    /// Warmup time.
    pub warmup_time: i32,
    /// Warmup modification count.
    pub warmup_modification_count: i32,
    /// Restarted.
    pub restarted: bool,
    /// Connected clients.
    pub num_connected_clients: i32,
    /// Non-spectator clients.
    pub num_non_spectator_clients: i32,
    /// Playing clients.
    pub num_playing_clients: i32,
    /// Voting clients.
    pub num_voting_clients: i32,
    /// Follow 1.
    pub follow1: i32,
    /// Follow 2.
    pub follow2: i32,
    /// Intermission time.
    pub intermission_time: i32,
    /// Intermission queued.
    pub intermission_queued: i32,
    /// Intermission origin.
    pub intermission_origin: Vec3,
    /// Intermission angle.
    pub intermission_angle: Vec3,
    /// Change map.
    pub changemap: Option<String>,
    /// Ready to exit.
    pub ready_to_exit: bool,
    /// Exit time.
    pub exit_time: i32,
    /// Frame number.
    pub frame_num: i32,
    /// Previous time.
    pub previous_time: i32,
    /// New session.
    pub new_session: bool,
    /// Fry sound.
    pub fry_sound: i32,
}

/// Capture level values (`captureLevelValues`).
#[must_use]
pub fn capture_level_values(source: &Q3GameLevel) -> LevelValues {
    LevelValues {
        time: source.time,
        start_time: source.start_time,
        warmup_time: source.warmup_time,
        warmup_modification_count: source.warmup_modification_count,
        restarted: source.restarted,
        num_connected_clients: source.num_connected_clients,
        num_non_spectator_clients: source.num_non_spectator_clients,
        num_playing_clients: source.num_playing_clients,
        num_voting_clients: source.num_voting_clients,
        follow1: source.follow1,
        follow2: source.follow2,
        intermission_time: source.intermission_time,
        intermission_queued: source.intermission_queued,
        intermission_origin: source.intermission_origin,
        intermission_angle: source.intermission_angle,
        changemap: source.changemap.clone(),
        ready_to_exit: source.ready_to_exit,
        exit_time: source.exit_time,
        frame_num: source.frame_num,
        previous_time: source.previous_time,
        new_session: source.new_session,
        fry_sound: source.fry_sound,
    }
}

/// Restore level values (`restoreLevelValues`).
pub fn restore_level_values(target: &mut Q3GameLevel, state: &LevelValues) {
    target.time = state.time;
    target.start_time = state.start_time;
    target.warmup_time = state.warmup_time;
    target.warmup_modification_count = state.warmup_modification_count;
    target.restarted = state.restarted;
    target.num_connected_clients = state.num_connected_clients;
    target.num_non_spectator_clients = state.num_non_spectator_clients;
    target.num_playing_clients = state.num_playing_clients;
    target.num_voting_clients = state.num_voting_clients;
    target.follow1 = state.follow1;
    target.follow2 = state.follow2;
    target.intermission_time = state.intermission_time;
    target.intermission_queued = state.intermission_queued;
    target.intermission_origin = state.intermission_origin;
    target.intermission_angle = state.intermission_angle;
    target.changemap = state.changemap.clone();
    target.ready_to_exit = state.ready_to_exit;
    target.exit_time = state.exit_time;
    target.frame_num = state.frame_num;
    target.previous_time = state.previous_time;
    target.new_session = state.new_session;
    target.fry_sound = state.fry_sound;
}

/// Encode level values.
#[must_use]
pub fn level_values_to_json(state: &LevelValues) -> SaveJson {
    obj(vec![
        ("time", num_i32(state.time)),
        ("startTime", num_i32(state.start_time)),
        ("warmupTime", num_i32(state.warmup_time)),
        ("warmupModificationCount", num_i32(state.warmup_modification_count)),
        ("restarted", boolean(state.restarted)),
        ("numConnectedClients", num_i32(state.num_connected_clients)),
        ("numNonSpectatorClients", num_i32(state.num_non_spectator_clients)),
        ("numPlayingClients", num_i32(state.num_playing_clients)),
        ("numVotingClients", num_i32(state.num_voting_clients)),
        ("follow1", num_i32(state.follow1)),
        ("follow2", num_i32(state.follow2)),
        ("intermissionTime", num_i32(state.intermission_time)),
        ("intermissionQueued", num_i32(state.intermission_queued)),
        ("intermissionOrigin", vec_to_json(state.intermission_origin)),
        ("intermissionAngle", vec_to_json(state.intermission_angle)),
        ("changemap", opt_str_to_json(state.changemap.as_deref())),
        ("readyToExit", boolean(state.ready_to_exit)),
        ("exitTime", num_i32(state.exit_time)),
        ("frameNum", num_i32(state.frame_num)),
        ("previousTime", num_i32(state.previous_time)),
        ("newSession", boolean(state.new_session)),
        ("frySound", num_i32(state.fry_sound)),
    ])
}

/// Read level values (`readLevelValues`).
pub fn read_level_values(reader: &SaveReader) -> Result<LevelValues, Q3GameError> {
    Ok(LevelValues {
        time: read_i32(reader, "time")?,
        start_time: read_i32(reader, "startTime")?,
        warmup_time: read_i32(reader, "warmupTime")?,
        warmup_modification_count: read_i32(reader, "warmupModificationCount")?,
        restarted: reader.field("restarted").boolean()?,
        num_connected_clients: read_i32(reader, "numConnectedClients")?,
        num_non_spectator_clients: read_i32(reader, "numNonSpectatorClients")?,
        num_playing_clients: read_i32(reader, "numPlayingClients")?,
        num_voting_clients: read_i32(reader, "numVotingClients")?,
        follow1: read_i32(reader, "follow1")?,
        follow2: read_i32(reader, "follow2")?,
        intermission_time: read_i32(reader, "intermissionTime")?,
        intermission_queued: read_i32(reader, "intermissionQueued")?,
        intermission_origin: read_vec(reader, "intermissionOrigin")?,
        intermission_angle: read_vec(reader, "intermissionAngle")?,
        changemap: read_opt_string(reader, "changemap")?,
        ready_to_exit: reader.field("readyToExit").boolean()?,
        exit_time: read_i32(reader, "exitTime")?,
        frame_num: read_i32(reader, "frameNum")?,
        previous_time: read_i32(reader, "previousTime")?,
        new_session: reader.field("newSession").boolean()?,
        fry_sound: read_i32(reader, "frySound")?,
    })
}

/// Read a vector (`readVector`).
pub fn read_save_vector(reader: &SaveReader) -> Result<Vec3, Q3GameError> {
    vec_from_reader(reader)
}

// ---------------------------------------------------------------------------
// save-state.ts / save-reader.ts: graph capture, restore, and reads
// ---------------------------------------------------------------------------

/// Ownership entry (`captureOwnership` row).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnershipEntry {
    /// Actor.
    pub actor: Option<ActorId>,
    /// Active.
    pub active: bool,
    /// Borrowed.
    pub borrowed: bool,
}

/// Client backing storage (`captureClientBacking` row).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientBacking {
    /// Source stats.
    pub source_stats: Vec<i32>,
    /// Special ammo.
    pub special_ammo: Vec<i32>,
}

/// Entity records surface (`Q3EntityRecords`).
pub trait Q3EntityRecords {
    /// Product.
    fn product(&self) -> Q3Product;
    /// Item table length.
    fn item_count(&self) -> usize;
    /// Capture ownership.
    fn capture_ownership(&self) -> Vec<OwnershipEntry>;
    /// Restore ownership.
    fn restore_ownership(&mut self, entries: Vec<OwnershipEntry>);
    /// Capture client backing.
    fn capture_client_backing(&self, slot: usize) -> ClientBacking;
    /// Restore client backing.
    fn restore_client_backing(&mut self, slot: usize, backing: &ClientBacking);
    /// Damage inflictor for an actor (`damageInflictor`).
    fn damage_inflictor(&self, actor: &ActorId) -> Participant;
    /// Restore record callbacks (`restoreCallbacks`).
    fn restore_callbacks(&mut self);
}

/// Saved-actor registry surface (`SessionActorRegistry` picks).
pub trait Q3ActorRegistry {
    /// Resolve a saved actor (`resolveSaved`).
    fn resolve_saved(&self, saved: &SavedActorId) -> Option<ActorId>;
    /// Reference a saved actor (`referenceSaved`).
    fn reference_saved(&self, saved: &SavedActorId) -> ActorId;
}

fn saved_actor_to_json(actor: &SavedActorId) -> SaveJson {
    obj(vec![
        ("slot", int(i64::from(actor.slot))),
        ("generation", int(i64::from(actor.generation))),
    ])
}

/// Read a saved actor (`readQ3Actor`).
pub fn read_q3_actor(reader: &SaveReader) -> Result<SavedActorId, Q3GameError> {
    let slot = reader.field("slot").integer(0)?;
    let generation = reader.field("generation").integer(0)?;
    if slot > i64::from(u32::MAX) || generation > i64::from(u32::MAX) {
        return Err(reader.fail("Q3 source actor outside session range").into());
    }
    #[allow(clippy::cast_possible_truncation)]
    Ok(SavedActorId {
        slot: slot as u32,
        generation: generation as u32,
    })
}

fn read_limited_slot(reader: &SaveReader, maximum: i64) -> Result<usize, Q3GameError> {
    let value = reader.integer(0)?;
    if value >= maximum {
        return Err(reader.fail("Q3 source reference outside retained table").into());
    }
    Ok(value as usize)
}

/// Saved trajectory.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SavedTrajectory {
    /// Raw type number.
    pub trajectory_type: i32,
    /// Time.
    pub time: i32,
    /// Duration.
    pub duration: i32,
    /// Base.
    pub base: Vec3,
    /// Delta.
    pub delta: Vec3,
}

fn trajectory_to_json(trajectory: &SavedTrajectory) -> SaveJson {
    obj(vec![
        ("type", num_i32(trajectory.trajectory_type)),
        ("time", num_i32(trajectory.time)),
        ("duration", num_i32(trajectory.duration)),
        ("base", vec_to_json(trajectory.base)),
        ("delta", vec_to_json(trajectory.delta)),
    ])
}

fn read_trajectory(reader: &SaveReader) -> Result<SavedTrajectory, Q3GameError> {
    Ok(SavedTrajectory {
        trajectory_type: read_i32(reader, "type")?,
        time: read_i32(reader, "time")?,
        duration: read_i32(reader, "duration")?,
        base: read_vec(reader, "base")?,
        delta: read_vec(reader, "delta")?,
    })
}

/// Saved shared fields.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SavedShared {
    /// Server flags.
    pub sv_flags: i32,
    /// Single client.
    pub single_client: i32,
    /// Contents.
    pub contents: i32,
    /// Owner number.
    pub owner_num: i32,
    /// Collision model.
    pub model: Q3CollisionModel,
}

/// Saved body state (`SavedBodyState`).
#[derive(Debug, Clone, PartialEq)]
pub struct SavedBodyState {
    /// Origin.
    pub origin: Vec3,
    /// Angles.
    pub angles: Vec3,
    /// Velocity.
    pub velocity: Vec3,
    /// Bounds.
    pub bounds: Bounds,
    /// Ground.
    pub ground: Option<SavedActorId>,
}

/// Saved link snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct SavedLink {
    /// Actor.
    pub actor: SavedActorId,
    /// State.
    pub state: SavedBodyState,
    /// Absolute bounds.
    pub absolute_bounds: Bounds,
    /// Link count.
    pub link_count: i32,
}

/// Saved private shared state.
#[derive(Debug, Clone, PartialEq)]
pub struct SavedSharedPrivate {
    /// Previous link.
    pub previous_link: Option<SavedLink>,
    /// Absolute-min override.
    pub abs_min_override: Option<Vec3>,
    /// Absolute-max override.
    pub abs_max_override: Option<Vec3>,
}

/// Saved classname (`Q3EntityState["classname"]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SavedClassname {
    /// Plain value.
    Value(Option<String>),
    /// Client name.
    ClientName(usize),
}

/// Saved activation (`Q3EntityState["activation"]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SavedActivation {
    /// Entity slot.
    Entity(usize),
    /// Saved actor.
    Actor(SavedActorId),
}

/// Saved entity (`Q3EntityState`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3GraphEntity {
    /// Values.
    pub values: EntityValues,
    /// Network values.
    pub network: NetworkValues,
    /// Position trajectory.
    pub pos: SavedTrajectory,
    /// Angle trajectory.
    pub apos: SavedTrajectory,
    /// Shared fields.
    pub shared: SavedShared,
    /// Private shared state.
    pub shared_private: SavedSharedPrivate,
    /// Client slot.
    pub client: Option<usize>,
    /// Classname.
    pub classname: SavedClassname,
    /// Parent slot.
    pub parent: Option<usize>,
    /// Next train slot.
    pub next_train: Option<usize>,
    /// Previous train slot.
    pub prev_train: Option<usize>,
    /// Target entity slot.
    pub target_ent: Option<usize>,
    /// Chain slot.
    pub chain: Option<usize>,
    /// Enemy slot.
    pub enemy: Option<usize>,
    /// Activator slot.
    pub activator: Option<usize>,
    /// Team chain slot.
    pub teamchain: Option<usize>,
    /// Team master slot.
    pub teammaster: Option<usize>,
    /// Activation.
    pub activation: Option<SavedActivation>,
    /// Item index.
    pub item: Option<usize>,
    /// Next think.
    pub nextthink: i32,
    /// Think callback.
    pub think: Option<String>,
    /// Reached callback.
    pub reached: Option<String>,
    /// Blocked callback.
    pub blocked: Option<String>,
    /// Touch callback.
    pub touch: Option<String>,
    /// Use callback.
    pub use_callback: Option<String>,
    /// Pain callback.
    pub pain: Option<String>,
    /// Die callback.
    pub die: Option<String>,
}

/// Saved client (`Q3ClientState`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3GraphClient {
    /// Values.
    pub values: ClientValues,
    /// Player values.
    pub player: PlayerValues,
    /// Persistant values.
    pub persistant: PersistantValues,
    /// Command.
    pub command: Q3UserCommand,
    /// Team values.
    pub team: TeamValues,
    /// Session values.
    pub session: SessionValues,
    /// Events.
    pub events: Vec<i32>,
    /// Event parameters.
    pub event_parms: Vec<i32>,
    /// Persistant slots.
    pub persistant_slots: Vec<i32>,
    /// Powerups.
    pub powerups: Vec<i32>,
    /// Ammo times.
    pub ammo_times: Vec<i32>,
    /// Backing storage.
    pub backing: ClientBacking,
    /// Hook slot.
    pub hook: Option<usize>,
    /// Persistant powerup slot.
    pub persistant_powerup: Option<usize>,
    /// Area bits.
    pub areabits: Option<Vec<u8>>,
}

/// Saved ownership row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedOwnership {
    /// Actor.
    pub actor: Option<SavedActorId>,
    /// Active.
    pub active: bool,
    /// Borrowed.
    pub borrowed: bool,
}

/// Saved graph (`Q3GraphState`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3Graph {
    /// Ownership rows.
    pub ownership: Vec<SavedOwnership>,
    /// Entities.
    pub entities: Vec<Q3GraphEntity>,
    /// Clients.
    pub clients: Vec<Q3GraphClient>,
    /// Entity count.
    pub num_entities: usize,
    /// Maximum clients.
    pub max_clients: usize,
}

fn opt_slot_to_json(slot: Option<usize>) -> SaveJson {
    slot.map_or(SaveJson::Null, |slot| int(slot as i64))
}

fn opt_callback_to_json(id: Option<&str>) -> SaveJson {
    id.map_or(SaveJson::Null, str)
}

fn collision_model_to_json(model: &Q3CollisionModel) -> SaveJson {
    match model {
        Q3CollisionModel::Inline { index } => obj(vec![("kind", str("inline")), ("index", num_i32(*index))]),
        Q3CollisionModel::Box => obj(vec![("kind", str("box"))]),
        Q3CollisionModel::Capsule => obj(vec![("kind", str("capsule"))]),
    }
}

fn read_collision_model(reader: &SaveReader) -> Result<Q3CollisionModel, Q3GameError> {
    let kind = reader.field("kind").choice_str(&["inline", "box", "capsule"])?;
    if kind == "inline" {
        #[allow(clippy::cast_possible_truncation)]
        Ok(Q3CollisionModel::Inline {
            index: reader.field("index").integer(0)? as i32,
        })
    } else if kind == "box" {
        Ok(Q3CollisionModel::Box)
    } else {
        Ok(Q3CollisionModel::Capsule)
    }
}

fn saved_body_to_json(state: &SavedBodyState) -> SaveJson {
    obj(vec![
        ("origin", vec_to_json(state.origin)),
        ("angles", vec_to_json(state.angles)),
        ("velocity", vec_to_json(state.velocity)),
        (
            "bounds",
            obj(vec![
                ("min", vec_to_json(state.bounds.min)),
                ("max", vec_to_json(state.bounds.max)),
            ]),
        ),
        (
            "ground",
            state.ground.as_ref().map_or(SaveJson::Null, saved_actor_to_json),
        ),
    ])
}

fn read_saved_body(reader: &SaveReader) -> Result<SavedBodyState, Q3GameError> {
    let bounds = reader.field("bounds");
    Ok(SavedBodyState {
        origin: read_vec(reader, "origin")?,
        angles: read_vec(reader, "angles")?,
        velocity: read_vec(reader, "velocity")?,
        bounds: Bounds {
            min: read_vec(&bounds, "min")?,
            max: read_vec(&bounds, "max")?,
        },
        ground: reader.field("ground").nullable(|value| read_q3_actor(&value))?,
    })
}

fn saved_link_to_json(link: &SavedLink) -> SaveJson {
    obj(vec![
        ("actor", saved_actor_to_json(&link.actor)),
        ("linkCount", num_i32(link.link_count)),
        (
            "absoluteBounds",
            obj(vec![
                ("min", vec_to_json(link.absolute_bounds.min)),
                ("max", vec_to_json(link.absolute_bounds.max)),
            ]),
        ),
        ("state", saved_body_to_json(&link.state)),
    ])
}

fn read_saved_link(reader: &SaveReader) -> Result<SavedLink, Q3GameError> {
    let bounds = reader.field("absoluteBounds");
    #[allow(clippy::cast_possible_truncation)]
    Ok(SavedLink {
        actor: read_q3_actor(&reader.field("actor"))?,
        link_count: reader.field("linkCount").integer(0)? as i32,
        absolute_bounds: Bounds {
            min: read_vec(&bounds, "min")?,
            max: read_vec(&bounds, "max")?,
        },
        state: read_saved_body(&reader.field("state"))?,
    })
}

fn graph_entity_to_json(entity: &Q3GraphEntity) -> SaveJson {
    let classname = match &entity.classname {
        SavedClassname::Value(value) => obj(vec![
            ("kind", str("value")),
            ("value", opt_str_to_json(value.as_deref())),
        ]),
        SavedClassname::ClientName(client) => obj(vec![("kind", str("client-name")), ("client", int(*client as i64))]),
    };
    let activation = entity
        .activation
        .as_ref()
        .map_or(SaveJson::Null, |activation| match activation {
            SavedActivation::Entity(slot) => obj(vec![("kind", str("entity")), ("slot", int(*slot as i64))]),
            SavedActivation::Actor(actor) => obj(vec![("kind", str("actor")), ("actor", saved_actor_to_json(actor))]),
        });
    obj(vec![
        ("values", entity_values_to_json(&entity.values)),
        ("network", network_values_to_json(&entity.network)),
        ("pos", trajectory_to_json(&entity.pos)),
        ("apos", trajectory_to_json(&entity.apos)),
        (
            "shared",
            obj(vec![
                ("svFlags", num_i32(entity.shared.sv_flags)),
                ("singleClient", num_i32(entity.shared.single_client)),
                ("contents", num_i32(entity.shared.contents)),
                ("ownerNum", num_i32(entity.shared.owner_num)),
                ("model", collision_model_to_json(&entity.shared.model)),
            ]),
        ),
        (
            "sharedPrivate",
            obj(vec![
                (
                    "absMinOverride",
                    entity
                        .shared_private
                        .abs_min_override
                        .map_or(SaveJson::Null, vec_to_json),
                ),
                (
                    "absMaxOverride",
                    entity
                        .shared_private
                        .abs_max_override
                        .map_or(SaveJson::Null, vec_to_json),
                ),
                (
                    "previousLink",
                    entity
                        .shared_private
                        .previous_link
                        .as_ref()
                        .map_or(SaveJson::Null, saved_link_to_json),
                ),
            ]),
        ),
        ("client", opt_slot_to_json(entity.client)),
        ("classname", classname),
        ("parent", opt_slot_to_json(entity.parent)),
        ("nextTrain", opt_slot_to_json(entity.next_train)),
        ("prevTrain", opt_slot_to_json(entity.prev_train)),
        ("targetEnt", opt_slot_to_json(entity.target_ent)),
        ("chain", opt_slot_to_json(entity.chain)),
        ("enemy", opt_slot_to_json(entity.enemy)),
        ("activator", opt_slot_to_json(entity.activator)),
        ("teamchain", opt_slot_to_json(entity.teamchain)),
        ("teammaster", opt_slot_to_json(entity.teammaster)),
        ("activation", activation),
        ("item", opt_slot_to_json(entity.item)),
        ("nextthink", num_i32(entity.nextthink)),
        ("think", opt_callback_to_json(entity.think.as_deref())),
        ("reached", opt_callback_to_json(entity.reached.as_deref())),
        ("blocked", opt_callback_to_json(entity.blocked.as_deref())),
        ("touch", opt_callback_to_json(entity.touch.as_deref())),
        ("use", opt_callback_to_json(entity.use_callback.as_deref())),
        ("pain", opt_callback_to_json(entity.pain.as_deref())),
        ("die", opt_callback_to_json(entity.die.as_deref())),
    ])
}

fn read_graph_entity(reader: &SaveReader) -> Result<Q3GraphEntity, Q3GameError> {
    let shared = reader.field("shared");
    let model = shared.field("model");
    let private = reader.field("sharedPrivate");
    let classname = reader.field("classname");
    let name_kind = classname.field("kind").choice_str(&["value", "client-name"])?;
    let read_entity_slot = |reader: &SaveReader, field: &str| -> Result<Option<usize>, Q3GameError> {
        Ok(reader.field(field).nullable(|value| read_limited_slot(&value, 1024))?)
    };
    Ok(Q3GraphEntity {
        values: read_entity_values(&reader.field("values"))?,
        network: read_network_values(&reader.field("network"))?,
        pos: read_trajectory(&reader.field("pos"))?,
        apos: read_trajectory(&reader.field("apos"))?,
        shared: SavedShared {
            sv_flags: read_i32(&shared, "svFlags")?,
            single_client: read_i32(&shared, "singleClient")?,
            contents: read_i32(&shared, "contents")?,
            owner_num: read_i32(&shared, "ownerNum")?,
            model: read_collision_model(&model)?,
        },
        shared_private: SavedSharedPrivate {
            previous_link: private
                .field("previousLink")
                .nullable(|value| read_saved_link(&value))?,
            abs_min_override: private
                .field("absMinOverride")
                .nullable(|value| vec_from_reader(&value))?,
            abs_max_override: private
                .field("absMaxOverride")
                .nullable(|value| vec_from_reader(&value))?,
        },
        client: reader.field("client").nullable(|value| read_limited_slot(&value, 64))?,
        classname: if name_kind == "value" {
            SavedClassname::Value(classname.field("value").nullable(|value| value.string())?)
        } else {
            SavedClassname::ClientName(read_limited_slot(&classname.field("client"), 64)?)
        },
        parent: read_entity_slot(reader, "parent")?,
        next_train: read_entity_slot(reader, "nextTrain")?,
        prev_train: read_entity_slot(reader, "prevTrain")?,
        target_ent: read_entity_slot(reader, "targetEnt")?,
        chain: read_entity_slot(reader, "chain")?,
        enemy: read_entity_slot(reader, "enemy")?,
        activator: read_entity_slot(reader, "activator")?,
        teamchain: read_entity_slot(reader, "teamchain")?,
        teammaster: read_entity_slot(reader, "teammaster")?,
        activation: reader
            .field("activation")
            .nullable(|value| -> Result<SavedActivation, Q3GameError> {
                if value.field("kind").choice_str(&["entity", "actor"])? == "entity" {
                    Ok(SavedActivation::Entity(read_limited_slot(&value.field("slot"), 1024)?))
                } else {
                    Ok(SavedActivation::Actor(read_q3_actor(&value.field("actor"))?))
                }
            })?,
        item: reader.field("item").nullable(|value| -> Result<usize, Q3GameError> {
            #[allow(clippy::cast_possible_truncation)]
            Ok(value.integer(0)? as usize)
        })?,
        nextthink: read_i32(reader, "nextthink")?,
        think: reader.field("think").nullable(|value| value.string())?,
        reached: reader.field("reached").nullable(|value| value.string())?,
        blocked: reader.field("blocked").nullable(|value| value.string())?,
        touch: reader.field("touch").nullable(|value| value.string())?,
        use_callback: reader.field("use").nullable(|value| value.string())?,
        pain: reader.field("pain").nullable(|value| value.string())?,
        die: reader.field("die").nullable(|value| value.string())?,
    })
}

fn numbers_to_json(values: &[i32]) -> SaveJson {
    arr(values.iter().map(|value| num_i32(*value)).collect())
}

fn read_numbers(reader: &SaveReader) -> Result<Vec<i32>, Q3GameError> {
    Ok(reader.list(|value| -> Result<i32, Q3GameError> {
        #[allow(clippy::cast_possible_truncation)]
        Ok(value.number()? as i32)
    })?)
}

fn user_command_to_json(command: &Q3UserCommand) -> SaveJson {
    obj(vec![
        ("serverTime", num_i32(command.server_time)),
        ("angles", vec_to_json(command.angles)),
        ("buttons", num_i32(command.buttons)),
        ("weapon", num_i32(command.weapon)),
        ("forwardmove", num_i32(command.forwardmove)),
        ("rightmove", num_i32(command.rightmove)),
        ("upmove", num_i32(command.upmove)),
    ])
}

fn read_user_command(reader: &SaveReader) -> Result<Q3UserCommand, Q3GameError> {
    Ok(Q3UserCommand {
        server_time: read_i32(reader, "serverTime")?,
        angles: read_vec(reader, "angles")?,
        buttons: read_i32(reader, "buttons")?,
        weapon: read_i32(reader, "weapon")?,
        forwardmove: read_i32(reader, "forwardmove")?,
        rightmove: read_i32(reader, "rightmove")?,
        upmove: read_i32(reader, "upmove")?,
    })
}

fn graph_client_to_json(client: &Q3GraphClient) -> SaveJson {
    obj(vec![
        ("values", client_values_to_json(&client.values)),
        ("player", player_values_to_json(&client.player)),
        ("persistant", persistant_values_to_json(&client.persistant)),
        ("command", user_command_to_json(&client.command)),
        ("team", team_values_to_json(&client.team)),
        ("session", session_values_to_json(&client.session)),
        ("events", numbers_to_json(&client.events)),
        ("eventParms", numbers_to_json(&client.event_parms)),
        ("persistantSlots", numbers_to_json(&client.persistant_slots)),
        ("powerups", numbers_to_json(&client.powerups)),
        ("ammoTimes", numbers_to_json(&client.ammo_times)),
        (
            "backing",
            obj(vec![
                ("sourceStats", numbers_to_json(&client.backing.source_stats)),
                ("specialAmmo", numbers_to_json(&client.backing.special_ammo)),
            ]),
        ),
        ("hook", opt_slot_to_json(client.hook)),
        ("persistantPowerup", opt_slot_to_json(client.persistant_powerup)),
        (
            "areabits",
            client
                .areabits
                .as_ref()
                .map_or(SaveJson::Null, |bytes| SaveJson::Bytes(bytes.clone())),
        ),
    ])
}

fn read_graph_client(reader: &SaveReader) -> Result<Q3GraphClient, Q3GameError> {
    let backing = reader.field("backing");
    Ok(Q3GraphClient {
        values: read_client_values(&reader.field("values"))?,
        player: read_player_values(&reader.field("player"))?,
        persistant: read_persistant_values(&reader.field("persistant"))?,
        command: read_user_command(&reader.field("command"))?,
        team: read_team_values(&reader.field("team"))?,
        session: read_session_values(&reader.field("session"))?,
        events: read_numbers(&reader.field("events"))?,
        event_parms: read_numbers(&reader.field("eventParms"))?,
        persistant_slots: read_numbers(&reader.field("persistantSlots"))?,
        powerups: read_numbers(&reader.field("powerups"))?,
        ammo_times: read_numbers(&reader.field("ammoTimes"))?,
        backing: ClientBacking {
            source_stats: read_numbers(&backing.field("sourceStats"))?,
            special_ammo: read_numbers(&backing.field("specialAmmo"))?,
        },
        hook: reader.field("hook").nullable(|value| read_limited_slot(&value, 1024))?,
        persistant_powerup: reader
            .field("persistantPowerup")
            .nullable(|value| read_limited_slot(&value, 1024))?,
        areabits: reader.field("areabits").nullable(|value| value.bytes())?,
    })
}

/// Encode a graph.
#[must_use]
pub fn graph_to_json(graph: &Q3Graph) -> SaveJson {
    obj(vec![
        (
            "ownership",
            arr(graph
                .ownership
                .iter()
                .map(|entry| {
                    obj(vec![
                        (
                            "actor",
                            entry.actor.as_ref().map_or(SaveJson::Null, saved_actor_to_json),
                        ),
                        ("active", boolean(entry.active)),
                        ("borrowed", boolean(entry.borrowed)),
                    ])
                })
                .collect()),
        ),
        (
            "entities",
            arr(graph.entities.iter().map(graph_entity_to_json).collect()),
        ),
        ("clients", arr(graph.clients.iter().map(graph_client_to_json).collect())),
        ("numEntities", int(graph.num_entities as i64)),
        ("maxClients", int(graph.max_clients as i64)),
    ])
}

/// Read a graph (`readQ3Graph`).
pub fn read_q3_graph(value: &SaveJson) -> Result<Q3Graph, Q3GameError> {
    let reader = SaveReader::at(value, "q3.graph");
    #[allow(clippy::cast_possible_truncation)]
    Ok(Q3Graph {
        ownership: reader
            .field("ownership")
            .list(|entry| -> Result<SavedOwnership, Q3GameError> {
                Ok(SavedOwnership {
                    actor: entry.field("actor").nullable(|value| read_q3_actor(&value))?,
                    active: entry.field("active").boolean()?,
                    borrowed: entry.field("borrowed").boolean()?,
                })
            })?,
        entities: reader.field("entities").list(|entry| read_graph_entity(&entry))?,
        clients: reader.field("clients").list(|entry| read_graph_client(&entry))?,
        num_entities: reader.field("numEntities").integer(64)? as usize,
        max_clients: reader.field("maxClients").integer(1)? as usize,
    })
}

fn saved_trajectory(trajectory: &Q3Trajectory) -> SavedTrajectory {
    SavedTrajectory {
        trajectory_type: trajectory.trajectory_type as i32,
        time: trajectory.time,
        duration: trajectory.duration,
        base: trajectory.base,
        delta: trajectory.delta,
    }
}

fn check_graph_entity(pool: &dyn EntityPool, slot: Option<usize>) -> Result<Option<usize>, Q3GameError> {
    match slot {
        None => Ok(None),
        Some(slot) if pool.entity(slot).is_some() => Ok(Some(slot)),
        Some(_) => Err(failure("Q3 graph contains a foreign entity record")),
    }
}

fn check_graph_client(pool: &dyn EntityPool, slot: Option<usize>) -> Result<Option<usize>, Q3GameError> {
    match slot {
        None => Ok(None),
        Some(slot) if pool.client(slot).is_some() => Ok(Some(slot)),
        Some(_) => Err(failure("Q3 graph contains a foreign client record")),
    }
}

/// Capture the entity/client graph (`captureQ3Graph`).
pub fn capture_q3_graph(records: &dyn Q3EntityRecords, pool: &dyn EntityPool) -> Result<Q3Graph, Q3GameError> {
    let mut entities = Vec::with_capacity(MAX_GENTITIES);
    for slot in 0..MAX_GENTITIES {
        let Some(entity) = pool.entity(slot) else {
            return Err(failure(format!("Q3 graph capture is missing entity slot {slot}")));
        };
        let link = entity.r.previous_link.as_ref().map(|link| SavedLink {
            actor: SavedActorId::from(&link.actor),
            link_count: link.link_count,
            absolute_bounds: link.absolute_bounds,
            state: SavedBodyState {
                origin: link.state.origin,
                angles: link.state.angles,
                velocity: link.state.velocity,
                bounds: link.state.bounds,
                ground: link.state.ground.as_ref().map(SavedActorId::from),
            },
        });
        let classname = match entity.capture_classname() {
            ClassName::Value(value) => SavedClassname::Value(value),
            ClassName::ClientName(client) => {
                check_graph_client(pool, Some(client))?;
                SavedClassname::ClientName(client)
            }
        };
        if let Some(Participant::Entity(slot)) = &entity.activation {
            check_graph_entity(pool, Some(*slot))?;
        }
        if let Some(item) = entity.item {
            if item >= records.item_count() {
                return Err(failure("Q3 entity has an unknown item identity"));
            }
        }
        let callbacks = pool.callbacks();
        entities.push(Q3GraphEntity {
            values: capture_entity_values(entity),
            network: capture_network_values(&entity.s),
            pos: saved_trajectory(&entity.s.pos),
            apos: saved_trajectory(&entity.s.apos),
            shared: SavedShared {
                sv_flags: entity.r.sv_flags,
                single_client: entity.r.single_client,
                contents: entity.r.contents,
                owner_num: entity.r.owner_num,
                model: entity.r.model,
            },
            shared_private: SavedSharedPrivate {
                previous_link: link,
                abs_min_override: entity.r.absmin_override,
                abs_max_override: entity.r.absmax_override,
            },
            client: check_graph_client(pool, entity.client)?,
            classname,
            parent: check_graph_entity(pool, entity.parent)?,
            next_train: check_graph_entity(pool, entity.next_train)?,
            prev_train: check_graph_entity(pool, entity.prev_train)?,
            target_ent: check_graph_entity(pool, entity.target_ent)?,
            chain: check_graph_entity(pool, entity.chain)?,
            enemy: check_graph_entity(pool, entity.enemy)?,
            activator: check_graph_entity(pool, entity.activator)?,
            teamchain: check_graph_entity(pool, entity.teamchain)?,
            teammaster: check_graph_entity(pool, entity.teammaster)?,
            activation: entity.activation.as_ref().map(|activation| match activation {
                Participant::Entity(slot) => SavedActivation::Entity(*slot),
                Participant::SharedActor(actor) => SavedActivation::Actor(SavedActorId::from(actor)),
            }),
            item: entity.item,
            nextthink: entity.nextthink,
            think: callbacks.think.capture(entity.think.as_ref())?,
            reached: callbacks.reached.capture(entity.reached.as_ref())?,
            blocked: callbacks.blocked.capture(entity.blocked.as_ref())?,
            touch: callbacks.touch.capture(entity.touch.as_ref())?,
            use_callback: callbacks.use_callbacks.capture(entity.use_callback.as_ref())?,
            pain: callbacks.pain.capture(entity.pain.as_ref())?,
            die: callbacks.die.capture(entity.die.as_ref())?,
        });
    }
    let mut clients = Vec::with_capacity(MAX_CLIENTS);
    for slot in 0..MAX_CLIENTS {
        let Some(client) = pool.client(slot) else {
            return Err(failure(format!("Q3 graph capture is missing client slot {slot}")));
        };
        clients.push(Q3GraphClient {
            values: capture_client_values(client),
            player: capture_player_values(&client.ps),
            persistant: capture_persistant_values(&client.pers),
            command: client.pers.cmd,
            team: capture_team_values(&client.pers.team_state),
            session: capture_session_values(&client.sess),
            events: client.ps.events.copy_vec(),
            event_parms: client.ps.event_parms.copy_vec(),
            persistant_slots: client.ps.persistant.copy_vec(),
            powerups: client.ps.powerups.copy_vec(),
            ammo_times: client.ammo_times.copy_vec(),
            backing: records.capture_client_backing(slot),
            hook: check_graph_entity(pool, client.hook)?,
            persistant_powerup: check_graph_entity(pool, client.persistant_powerup)?,
            areabits: client.areabits.clone(),
        });
    }
    Ok(Q3Graph {
        ownership: records
            .capture_ownership()
            .into_iter()
            .map(|entry| SavedOwnership {
                actor: entry.actor.as_ref().map(SavedActorId::from),
                active: entry.active,
                borrowed: entry.borrowed,
            })
            .collect(),
        entities,
        clients,
        num_entities: pool.num_entities(),
        max_clients: pool.max_clients(),
    })
}

/// Prepare record storage for a graph (`prepareQ3Graph`).
pub fn prepare_q3_graph(
    records: &mut dyn Q3EntityRecords,
    state: &Q3Graph,
    actors: &dyn Q3ActorRegistry,
) -> Result<(), Q3GameError> {
    if state.entities.len() != MAX_GENTITIES || state.clients.len() != MAX_CLIENTS {
        return Err(failure("Q3 save must retain all entity and client slots"));
    }
    let mut ownership = Vec::with_capacity(state.ownership.len());
    for entry in &state.ownership {
        let actor = entry.actor.as_ref().map(|saved| actors.resolve_saved(saved));
        if entry.actor.is_some() && actor.as_ref().is_some_and(Option::is_none) {
            return Err(failure("Q3 saved ownership actor is not live"));
        }
        ownership.push(OwnershipEntry {
            actor: actor.flatten(),
            active: entry.active,
            borrowed: entry.borrowed,
        });
    }
    records.restore_ownership(ownership);
    for (slot, client) in state.clients.iter().enumerate() {
        records.restore_client_backing(slot, &client.backing);
    }
    Ok(())
}

fn restore_graph_entity(pool: &dyn EntityPool, slot: Option<usize>) -> Result<Option<usize>, Q3GameError> {
    match slot {
        None => Ok(None),
        Some(slot) if pool.entity(slot).is_some() => Ok(Some(slot)),
        Some(slot) => Err(failure(format!("Q3 entity {slot} outside 0..1023"))),
    }
}

fn restore_graph_client(pool: &dyn EntityPool, slot: usize) -> Result<usize, Q3GameError> {
    if pool.client(slot).is_some() {
        Ok(slot)
    } else {
        Err(failure(format!("Q3 client {slot} outside 0..63")))
    }
}

/// Restore the entity/client graph (`restoreQ3Graph`).
pub fn restore_q3_graph(
    records: &dyn Q3EntityRecords,
    pool: &mut dyn EntityPool,
    state: &Q3Graph,
    actors: &dyn Q3ActorRegistry,
) -> Result<(), Q3GameError> {
    if state.entities.len() != MAX_GENTITIES || state.clients.len() != MAX_CLIENTS {
        return Err(failure("Q3 save must retain all entity and client slots"));
    }
    for (slot, saved) in state.entities.iter().enumerate() {
        let target = pool
            .entity_mut(slot)
            .ok_or_else(|| failure(format!("Q3 graph restore is missing entity slot {slot}")))?;
        restore_entity_values(target, &saved.values);
        restore_network_values(&mut target.s, &saved.network);
        target.s.pos = Q3Trajectory {
            trajectory_type: TrajectoryType::from_i32(saved.pos.trajectory_type)?,
            time: saved.pos.time,
            duration: saved.pos.duration,
            base: saved.pos.base,
            delta: saved.pos.delta,
        };
        target.s.apos = Q3Trajectory {
            trajectory_type: TrajectoryType::from_i32(saved.apos.trajectory_type)?,
            time: saved.apos.time,
            duration: saved.apos.duration,
            base: saved.apos.base,
            delta: saved.apos.delta,
        };
        target.r.sv_flags = saved.shared.sv_flags;
        target.r.single_client = saved.shared.single_client;
        target.r.model = saved.shared.model;
        target.r.contents = saved.shared.contents;
        target.r.owner_num = saved.shared.owner_num;
        target.r.absmin_override = saved.shared_private.abs_min_override;
        target.r.absmax_override = saved.shared_private.abs_max_override;
        target.r.previous_link = saved.shared_private.previous_link.as_ref().map(|link| Q3LinkedBody {
            actor: actors.reference_saved(&link.actor),
            link_count: link.link_count,
            absolute_bounds: link.absolute_bounds,
            state: Q3BodyState {
                origin: link.state.origin,
                angles: link.state.angles,
                velocity: link.state.velocity,
                bounds: link.state.bounds,
                ground: link.state.ground.as_ref().map(|ground| actors.reference_saved(ground)),
            },
        });
    }
    for (slot, saved) in state.entities.iter().enumerate() {
        let client = saved
            .client
            .map(|client| restore_graph_client(pool, client))
            .transpose()?;
        let classname = match &saved.classname {
            SavedClassname::Value(value) => ClassName::Value(value.clone()),
            SavedClassname::ClientName(client) => ClassName::ClientName(restore_graph_client(pool, *client)?),
        };
        let parent = restore_graph_entity(pool, saved.parent)?;
        let next_train = restore_graph_entity(pool, saved.next_train)?;
        let prev_train = restore_graph_entity(pool, saved.prev_train)?;
        let target_ent = restore_graph_entity(pool, saved.target_ent)?;
        let chain = restore_graph_entity(pool, saved.chain)?;
        let enemy = restore_graph_entity(pool, saved.enemy)?;
        let activator = restore_graph_entity(pool, saved.activator)?;
        let teamchain = restore_graph_entity(pool, saved.teamchain)?;
        let teammaster = restore_graph_entity(pool, saved.teammaster)?;
        let activation = match saved.activation.as_ref() {
            None => None,
            Some(SavedActivation::Entity(slot)) => {
                let slot = *slot;
                restore_graph_entity(pool, Some(slot))?;
                Some(Participant::Entity(slot))
            }
            Some(SavedActivation::Actor(actor)) => Some(records.damage_inflictor(&actors.reference_saved(actor))),
        };
        let item = saved
            .item
            .map(|item| {
                if item < records.item_count() {
                    Ok(item)
                } else {
                    Err(failure(format!("Q3 saved item {item} is outside the retained table")))
                }
            })
            .transpose()?;
        let callbacks = pool.callbacks();
        let think = callbacks.think.resolve(saved.think.as_deref())?;
        let reached = callbacks.reached.resolve(saved.reached.as_deref())?;
        let blocked = callbacks.blocked.resolve(saved.blocked.as_deref())?;
        let touch = callbacks.touch.resolve(saved.touch.as_deref())?;
        let use_callback = callbacks.use_callbacks.resolve(saved.use_callback.as_deref())?;
        let pain = callbacks.pain.resolve(saved.pain.as_deref())?;
        let die = callbacks.die.resolve(saved.die.as_deref())?;
        let target = pool
            .entity_mut(slot)
            .ok_or_else(|| failure(format!("Q3 graph restore is missing entity slot {slot}")))?;
        target.client = client;
        target.set_classname(None);
        if let ClassName::ClientName(client) = classname {
            target.bind_client_name(client);
        } else if let ClassName::Value(value) = classname {
            target.set_classname(value);
        }
        target.parent = parent;
        target.next_train = next_train;
        target.prev_train = prev_train;
        target.target_ent = target_ent;
        target.chain = chain;
        target.enemy = enemy;
        target.activator = activator;
        target.teamchain = teamchain;
        target.teammaster = teammaster;
        target.activation = activation;
        target.item = item;
        target.nextthink = saved.nextthink;
        target.think = think;
        target.reached = reached;
        target.blocked = blocked;
        target.touch = touch;
        target.use_callback = use_callback;
        target.pain = pain;
        target.die = die;
    }
    for (slot, saved) in state.clients.iter().enumerate() {
        let hook = restore_graph_entity(pool, saved.hook)?;
        let persistant_powerup = restore_graph_entity(pool, saved.persistant_powerup)?;
        let target = pool
            .client_mut(slot)
            .ok_or_else(|| failure(format!("Q3 graph restore is missing client slot {slot}")))?;
        restore_client_values(target, &saved.values);
        restore_player_values(&mut target.ps, &saved.player);
        restore_persistant_values(&mut target.pers, &saved.persistant);
        target.pers.cmd = saved.command;
        restore_team_values(&mut target.pers.team_state, &saved.team);
        restore_session_values(&mut target.sess, &saved.session);
        target.ps.events.restore(&saved.events)?;
        target.ps.event_parms.restore(&saved.event_parms)?;
        target.ps.persistant.restore(&saved.persistant_slots)?;
        target.ps.powerups.restore(&saved.powerups)?;
        target.ammo_times.restore(&saved.ammo_times)?;
        target.hook = hook;
        target.persistant_powerup = persistant_powerup;
        target.areabits = saved.areabits.clone();
    }
    pool.restore_counts(state.num_entities, state.max_clients);
    Ok(())
}

// ---------------------------------------------------------------------------
// save-level.ts: level capture and restore
// ---------------------------------------------------------------------------

/// Vote state (`VoteState`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3VoteState {
    /// Time.
    pub time: i32,
    /// Yes votes.
    pub yes: i32,
    /// No votes.
    pub no: i32,
    /// String.
    pub string: String,
    /// Display string.
    pub display_string: String,
    /// Execute time.
    pub execute_time: i32,
}

impl Default for Q3VoteState {
    fn default() -> Self {
        Self {
            time: 0,
            yes: 0,
            no: 0,
            string: String::new(),
            display_string: String::new(),
            execute_time: 0,
        }
    }
}

/// Team vote state (`TeamVoteState`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3TeamVoteState {
    /// Time.
    pub time: i32,
    /// Yes votes.
    pub yes: i32,
    /// No votes.
    pub no: i32,
    /// String.
    pub string: String,
}

impl Default for Q3TeamVoteState {
    fn default() -> Self {
        Self {
            time: 0,
            yes: 0,
            no: 0,
            string: String::new(),
        }
    }
}

/// Game level (`GameLevel` / `MatchState` fields used by saves).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3GameLevel {
    /// Time.
    pub time: i32,
    /// Start time.
    pub start_time: i32,
    /// Warmup time.
    pub warmup_time: i32,
    /// Warmup modification count.
    pub warmup_modification_count: i32,
    /// Restarted.
    pub restarted: bool,
    /// Connected clients.
    pub num_connected_clients: i32,
    /// Non-spectator clients.
    pub num_non_spectator_clients: i32,
    /// Playing clients.
    pub num_playing_clients: i32,
    /// Voting clients.
    pub num_voting_clients: i32,
    /// Team voting clients.
    pub num_team_voting_clients: [i32; 2],
    /// Sorted clients.
    pub sorted_clients: Vec<i32>,
    /// Follow 1.
    pub follow1: i32,
    /// Follow 2.
    pub follow2: i32,
    /// Intermission time.
    pub intermission_time: i32,
    /// Intermission queued.
    pub intermission_queued: i32,
    /// Intermission origin.
    pub intermission_origin: Vec3,
    /// Intermission angle.
    pub intermission_angle: Vec3,
    /// Change map.
    pub changemap: Option<String>,
    /// Ready to exit.
    pub ready_to_exit: bool,
    /// Exit time.
    pub exit_time: i32,
    /// Frame number.
    pub frame_num: i32,
    /// Previous time.
    pub previous_time: i32,
    /// New session.
    pub new_session: bool,
    /// Fry sound.
    pub fry_sound: i32,
    /// Team scores.
    pub team_scores: Q3PlayerSlots,
    /// Vote.
    pub vote: Q3VoteState,
    /// Team votes.
    pub team_votes: [Q3TeamVoteState; 2],
}

impl Default for Q3GameLevel {
    fn default() -> Self {
        Self {
            time: 0,
            start_time: 0,
            warmup_time: 0,
            warmup_modification_count: 0,
            restarted: false,
            num_connected_clients: 0,
            num_non_spectator_clients: 0,
            num_playing_clients: 0,
            num_voting_clients: 0,
            num_team_voting_clients: [0, 0],
            sorted_clients: vec![0; MAX_CLIENTS],
            follow1: 0,
            follow2: 0,
            intermission_time: 0,
            intermission_queued: 0,
            intermission_origin: vec3(0.0, 0.0, 0.0),
            intermission_angle: vec3(0.0, 0.0, 0.0),
            changemap: None,
            ready_to_exit: false,
            exit_time: 0,
            frame_num: 0,
            previous_time: 0,
            new_session: false,
            fry_sound: 0,
            team_scores: Q3PlayerSlots::new(4),
            vote: Q3VoteState::default(),
            team_votes: [Q3TeamVoteState::default(), Q3TeamVoteState::default()],
        }
    }
}

/// Capture the level (`captureQ3Level`).
#[must_use]
pub fn capture_q3_level(level: &Q3GameLevel) -> SaveJson {
    obj(vec![
        ("values", level_values_to_json(&capture_level_values(level))),
        ("teamScores", numbers_to_json(&level.team_scores.copy_vec())),
        ("numTeamVotingClients", numbers_to_json(&level.num_team_voting_clients)),
        ("sortedClients", numbers_to_json(&level.sorted_clients)),
        (
            "vote",
            obj(vec![
                ("time", num_i32(level.vote.time)),
                ("yes", num_i32(level.vote.yes)),
                ("no", num_i32(level.vote.no)),
                ("string", str(&level.vote.string)),
                ("displayString", str(&level.vote.display_string)),
                ("executeTime", num_i32(level.vote.execute_time)),
            ]),
        ),
        (
            "teamVotes",
            arr(level
                .team_votes
                .iter()
                .map(|vote| {
                    obj(vec![
                        ("time", num_i32(vote.time)),
                        ("yes", num_i32(vote.yes)),
                        ("no", num_i32(vote.no)),
                        ("string", str(&vote.string)),
                    ])
                })
                .collect()),
        ),
    ])
}

/// Restore the level (`restoreQ3Level`).
pub fn restore_q3_level(level: &mut Q3GameLevel, value: &SaveJson) -> Result<(), Q3GameError> {
    let reader = SaveReader::at(value, "q3.level");
    let values = read_level_values(&reader.field("values"))?;
    restore_level_values(level, &values);
    let scores = read_numbers(&reader.field("teamScores"))?;
    let voting = read_numbers(&reader.field("numTeamVotingClients"))?;
    let sorted = read_numbers(&reader.field("sortedClients"))?;
    let teams: Vec<Q3TeamVoteState> =
        reader
            .field("teamVotes")
            .list(|value| -> Result<Q3TeamVoteState, Q3GameError> {
                Ok(Q3TeamVoteState {
                    time: read_i32(&value, "time")?,
                    yes: read_i32(&value, "yes")?,
                    no: read_i32(&value, "no")?,
                    string: value.field("string").string()?,
                })
            })?;
    if scores.len() != level.team_scores.len()
        || voting.len() != 2
        || sorted.len() != level.sorted_clients.len()
        || teams.len() != 2
    {
        return Err(reader.fail("Q3 level table length mismatch").into());
    }
    for (index, score) in scores.iter().enumerate() {
        level.team_scores.set(index, *score);
    }
    let (Some(first), Some(second), Some(red), Some(blue)) =
        (voting.first(), voting.get(1), teams.first(), teams.get(1))
    else {
        return Err(reader.fail("Q3 level team table missing").into());
    };
    level.num_team_voting_clients[0] = *first;
    level.num_team_voting_clients[1] = *second;
    level.sorted_clients.copy_from_slice(&sorted);
    let vote = reader.field("vote");
    level.vote.time = read_i32(&vote, "time")?;
    level.vote.yes = read_i32(&vote, "yes")?;
    level.vote.no = read_i32(&vote, "no")?;
    level.vote.string = vote.field("string").string()?;
    level.vote.display_string = vote.field("displayString").string()?;
    level.vote.execute_time = read_i32(&vote, "executeTime")?;
    level.team_votes[0] = red.clone();
    level.team_votes[1] = blue.clone();
    Ok(())
}

// ---------------------------------------------------------------------------
// save-module-values.ts: module entity and cvar reads
// ---------------------------------------------------------------------------

/// Read a module entity slot (`readModuleEntity`).
pub fn read_module_entity(reader: &SaveReader, pool: &dyn EntityPool) -> Result<usize, Q3GameError> {
    let slot = reader.integer(0)?;
    if slot > 1023 {
        return Err(reader.fail("module source entity slot exceeds table extent").into());
    }
    #[allow(clippy::cast_possible_truncation)]
    let slot = slot as usize;
    if pool.entity(slot).is_none() {
        return Err(reader.fail("module references an absent source entity slot").into());
    }
    Ok(slot)
}

/// Cvar snapshot (`CvarSnapshot`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3CvarSnapshot {
    /// Name.
    pub name: String,
    /// Value.
    pub value: String,
    /// Reset value.
    pub reset_value: String,
    /// Latched value.
    pub latched_value: Option<String>,
    /// Flags.
    pub flags: i32,
    /// Modified.
    pub modified: bool,
    /// Modification count.
    pub modification_count: i32,
    /// Numeric value.
    pub numeric_value: f64,
    /// Integer value.
    pub integer_value: i32,
}

/// Capture a module cvar (`captureModuleCvar`).
#[must_use]
pub fn capture_module_cvar(value: &Q3CvarSnapshot) -> SaveJson {
    obj(vec![
        ("name", str(&value.name)),
        ("value", str(&value.value)),
        ("resetValue", str(&value.reset_value)),
        ("latchedValue", opt_str_to_json(value.latched_value.as_deref())),
        ("flags", num_i32(value.flags)),
        ("modified", boolean(value.modified)),
        ("modificationCount", num_i32(value.modification_count)),
        ("numericValue", num(value.numeric_value)),
        ("integerValue", num_i32(value.integer_value)),
    ])
}

/// Read a module cvar (`readModuleCvar`).
pub fn read_module_cvar(reader: &SaveReader) -> Result<Q3CvarSnapshot, Q3GameError> {
    #[allow(clippy::cast_possible_truncation)]
    Ok(Q3CvarSnapshot {
        name: reader.field("name").string()?,
        value: reader.field("value").string()?,
        reset_value: reader.field("resetValue").string()?,
        latched_value: reader.field("latchedValue").nullable(|value| value.string())?,
        flags: reader.field("flags").integer(i64::MIN)? as i32,
        modified: reader.field("modified").boolean()?,
        modification_count: reader.field("modificationCount").integer(i64::MIN)? as i32,
        numeric_value: reader.field("numericValue").number()?,
        integer_value: reader.field("integerValue").integer(i64::MIN)? as i32,
    })
}

// ---------------------------------------------------------------------------
// mover.ts: push transactions and binary movers (g_mover.c)
// ---------------------------------------------------------------------------

/// Mover-stop effect flag (`EF_MOVER_STOP`).
const EF_MOVER_STOP: i32 = 0x400;
/// Crush means of death (`MOD_CRUSH`).
const MOD_CRUSH: i32 = 17;

/// Convert a fallible runtime result into a fail-fast panic at native-callback
/// boundaries, matching donor throws inside callbacks.
fn or_panic(result: Result<(), Q3GameError>) {
    if let Err(error) = result {
        panic!("{error}");
    }
}

/// Pushed-entity stack entry.
#[derive(Debug, Clone)]
enum PushedEntity {
    /// Native entity.
    Native {
        /// Slot.
        slot: usize,
        /// Saved origin.
        origin: Vec3,
        /// Saved angles.
        angles: Vec3,
        /// Saved client yaw, when the entity had a client.
        yaw: Option<f32>,
    },
    /// Shared body.
    Shared {
        /// Body snapshot.
        body: SharedMoverBody,
    },
}

/// Pusher snapshot for push transactions.
#[derive(Debug, Clone)]
struct PusherSnapshot {
    actor: ActorId,
    e_flags: i32,
    current_origin: Vec3,
    current_angles: Vec3,
    mins: Vec3,
    maxs: Vec3,
    absmin: Vec3,
    absmax: Vec3,
    pos_type: TrajectoryType,
    apos_type: TrajectoryType,
}

fn mover_snapshot(pool: &dyn EntityPool, slot: usize) -> Result<PusherSnapshot, Q3GameError> {
    let Some(entity) = pool.entity(slot) else {
        return Err(failure("Mover entity does not belong to this pool"));
    };
    Ok(PusherSnapshot {
        actor: entity.actor.clone(),
        e_flags: entity.s.e_flags,
        current_origin: entity.r.current_origin,
        current_angles: entity.r.current_angles,
        mins: entity.r.mins,
        maxs: entity.r.maxs,
        absmin: entity.r.absmin(),
        absmax: entity.r.absmax(),
        pos_type: entity.s.pos.trajectory_type,
        apos_type: entity.s.apos.trajectory_type,
    })
}

fn mover_angle_short(angle: f32) -> i32 {
    qvm_float_to_int(angle * 65536.0 / 360.0) & 65535
}

fn push_rotation(origin: Vec3, pusher_origin: Vec3, amove: Vec3) -> Vec3 {
    let axes = angle_vectors(amove);
    let m0 = axes.forward;
    let m1 = scale3(axes.right, -1.0);
    let m2 = axes.up;
    let org = sub3(origin, pusher_origin);
    let rotated = vec3(
        dot3(org, vec3(m0.x, m1.x, m2.x)),
        dot3(org, vec3(m0.y, m1.y, m2.y)),
        dot3(org, vec3(m0.z, m1.z, m2.z)),
    );
    sub3(rotated, org)
}

/// Mover runtime (`MoverRuntime`).
#[derive(Debug, Clone)]
pub struct MoverRuntime {
    /// Previous frame time.
    pub previous_time: i32,
}

impl MoverRuntime {
    /// New runtime.
    #[must_use]
    pub fn new(previous_time: i32) -> Self {
        Self { previous_time }
    }

    fn check_product(&self, driver: &mut dyn Q3Driver) -> Result<(), Q3GameError> {
        if driver.pool().product() != driver.combat().product() {
            return Err(failure("Mover product does not match its entity pool"));
        }
        Ok(())
    }

    fn owned(&self, driver: &mut dyn Q3Driver, slot: usize) -> Result<(), Q3GameError> {
        if driver.pool().entity(slot).is_none() {
            return Err(failure("Mover entity does not belong to this pool"));
        }
        Ok(())
    }

    /// Test an entity position (`testEntityPosition`).
    pub fn test_entity_position(
        &self,
        driver: &mut dyn Q3Driver,
        slot: usize,
    ) -> Result<Option<Participant>, Q3GameError> {
        self.check_product(driver)?;
        self.owned(driver, slot)?;
        let (client_slot, pos_base, mins, maxs, actor, clipmask) = {
            let entity = driver
                .pool()
                .entity(slot)
                .ok_or_else(|| failure("Mover entity does not belong to this pool"))?;
            (
                entity.client,
                entity.s.pos.base,
                entity.r.mins,
                entity.r.maxs,
                entity.actor.clone(),
                entity.clipmask,
            )
        };
        let start = match client_slot {
            None => pos_base,
            Some(client) => driver
                .pool()
                .client(client)
                .map(|client| client.ps.origin)
                .unwrap_or(pos_base),
        };
        let query = Q3TraceQuery {
            start,
            end: start,
            shape: Q3TraceShape::Box { mins, maxs },
            pass_actor: Some(actor),
            mask: if clipmask == 0 { 1 } else { clipmask },
        };
        let result = driver.spatial().trace_actor(&query);
        if result.solidity == Q3Solidity::Clear {
            return Ok(None);
        }
        if let Q3TraceHit::Actor(actor) = &result.hit {
            let participant = driver.mover_actors().participant(actor);
            return Ok(Some(participant));
        }
        self.owned(driver, ENTITYNUM_WORLD)?;
        Ok(Some(Participant::Entity(ENTITYNUM_WORLD)))
    }

    fn try_pushing(
        &self,
        driver: &mut dyn Q3Driver,
        check: usize,
        pusher: &PusherSnapshot,
        mv: Vec3,
        amove: Vec3,
        pushed: &mut Vec<PushedEntity>,
    ) -> Result<bool, Q3GameError> {
        let rider = driver
            .pool()
            .entity(check)
            .is_some_and(|entity| rides(entity, &pusher.actor));
        if pusher.e_flags & EF_MOVER_STOP != 0 && !rider {
            return Ok(false);
        }
        if pushed.len() >= MAX_GENTITIES {
            return Err(failure("pushed stack exceeds MAX_GENTITIES"));
        }
        let (client_slot, pos_base, apos_base) = {
            let entity = driver
                .pool()
                .entity(check)
                .ok_or_else(|| failure("Mover entity does not belong to this pool"))?;
            (entity.client, entity.s.pos.base, entity.s.apos.base)
        };
        let origin = match client_slot {
            None => pos_base,
            Some(client) => driver
                .pool()
                .client(client)
                .map(|client| client.ps.origin)
                .unwrap_or(pos_base),
        };
        let yaw = client_slot.and_then(|client| {
            driver
                .pool()
                .client(client)
                .map(|client| client.ps.delta_angles[1] as f32)
        });
        let saved = PushedEntity::Native {
            slot: check,
            origin,
            angles: apos_base,
            yaw,
        };
        pushed.push(saved.clone());
        let PushedEntity::Native {
            origin: saved_origin, ..
        } = &saved
        else {
            unreachable!()
        };
        let rotation = push_rotation(*saved_origin, pusher.current_origin, amove);
        let client_slot = {
            let entity = driver
                .pool()
                .entity_mut(check)
                .ok_or_else(|| failure("Mover entity does not belong to this pool"))?;
            entity.s.pos.base = add3(add3(entity.s.pos.base, mv), rotation);
            entity.client
        };
        if let Some(client) = client_slot {
            let yaw_add = mover_angle_short(amove.y);
            if let Some(client) = driver.pool().client_mut(client) {
                client.ps.origin = add3(add3(client.ps.origin, mv), rotation);
                client.ps.delta_angles[1] = client.ps.delta_angles[1].wrapping_add(yaw_add);
            }
        }
        let still_rider = driver
            .pool()
            .entity(check)
            .is_some_and(|entity| rides(entity, &pusher.actor));
        if !still_rider {
            if let Some(entity) = driver.pool().entity_mut(check) {
                lose_ground(entity);
            }
        }
        if self.test_entity_position(driver, check)?.is_none() {
            let (client_slot, pos_base) = {
                let entity = driver
                    .pool()
                    .entity(check)
                    .ok_or_else(|| failure("Mover entity does not belong to this pool"))?;
                (entity.client, entity.s.pos.base)
            };
            let origin = match client_slot {
                None => pos_base,
                Some(client) => driver
                    .pool()
                    .client(client)
                    .map(|client| client.ps.origin)
                    .unwrap_or(pos_base),
            };
            if let Some(entity) = driver.pool().entity_mut(check) {
                entity.r.current_origin = origin;
            }
            driver.world().link(check);
            return Ok(true);
        }
        let saved_angles = match &saved {
            PushedEntity::Native { angles, .. } => *angles,
            PushedEntity::Shared { .. } => unreachable!("pushed native entry"),
        };
        let client_slot = {
            let entity = driver
                .pool()
                .entity_mut(check)
                .ok_or_else(|| failure("Mover entity does not belong to this pool"))?;
            entity.s.pos.base = *saved_origin;
            entity.s.apos.base = saved_angles;
            entity.client
        };
        if let Some(client) = client_slot {
            if let Some(client) = driver.pool().client_mut(client) {
                client.ps.origin = *saved_origin;
            }
        }
        if self.test_entity_position(driver, check)?.is_none() {
            if let Some(entity) = driver.pool().entity_mut(check) {
                lose_ground(entity);
            }
            pushed.pop();
            return Ok(true);
        }
        Ok(false)
    }

    fn check_proximity_position(&self, driver: &mut dyn Q3Driver, slot: usize) -> Result<bool, Q3GameError> {
        let (pos_base, movedir, actor) = {
            let entity = driver
                .pool()
                .entity(slot)
                .ok_or_else(|| failure("Mover entity does not belong to this pool"))?;
            (entity.s.pos.base, entity.movedir, entity.actor.clone())
        };
        let start = add3(pos_base, scale3(movedir, 0.125));
        let end = add3(pos_base, scale3(movedir, 2.0));
        let query = Q3TraceQuery {
            start,
            end,
            shape: Q3TraceShape::Point,
            pass_actor: Some(actor),
            mask: 1,
        };
        let trace = driver.spatial().trace_actor(&query);
        Ok(trace.solidity == Q3Solidity::Clear && trace.fraction == 1.0)
    }

    fn shared_position_blocked(&self, driver: &mut dyn Q3Driver, check: &SharedMoverBody) -> bool {
        let Some(body) = driver.mover_actors().observe(&check.actor) else {
            return false;
        };
        let query = Q3TraceQuery {
            start: body.state.origin,
            end: body.state.origin,
            shape: Q3TraceShape::Box {
                mins: body.state.bounds.min,
                maxs: body.state.bounds.max,
            },
            pass_actor: Some(check.actor.clone()),
            mask: if check.clip_mask == 0 { 1 } else { check.clip_mask },
        };
        driver.spatial().trace_actor(&query).solidity != Q3Solidity::Clear
    }

    fn try_pushing_shared(
        &self,
        driver: &mut dyn Q3Driver,
        check: &SharedMoverBody,
        pusher: &PusherSnapshot,
        mv: Vec3,
        amove: Vec3,
        pushed: &mut Vec<PushedEntity>,
    ) -> Result<bool, Q3GameError> {
        let rider = check
            .state
            .ground
            .as_ref()
            .is_some_and(|ground| ground == &pusher.actor);
        if pusher.e_flags & EF_MOVER_STOP != 0 && !rider {
            return Ok(false);
        }
        if pushed.len() >= MAX_GENTITIES {
            return Err(failure("pushed stack exceeds MAX_GENTITIES"));
        }
        pushed.push(PushedEntity::Shared { body: check.clone() });
        let rotation = push_rotation(check.state.origin, pusher.current_origin, amove);
        let ground = if rider { check.state.ground.clone() } else { None };
        driver.mover_actors().write(
            &check.actor,
            add3(add3(check.state.origin, mv), rotation),
            ground.clone(),
        );
        if !self.shared_position_blocked(driver, check) {
            driver.mover_actors().link_actor(&check.actor);
            return Ok(true);
        }
        driver.mover_actors().write(&check.actor, check.state.origin, ground);
        if !self.shared_position_blocked(driver, check) {
            driver.mover_actors().write(&check.actor, check.state.origin, None);
            pushed.pop();
            return Ok(true);
        }
        Ok(false)
    }

    fn restore_pushed(&self, driver: &mut dyn Q3Driver, pushed: &[PushedEntity]) -> Result<(), Q3GameError> {
        for saved in pushed.iter().rev() {
            match saved {
                PushedEntity::Shared { body } => {
                    if let Some(current) = driver.mover_actors().observe(&body.actor) {
                        driver
                            .mover_actors()
                            .write(&body.actor, body.state.origin, current.state.ground.clone());
                        driver.mover_actors().link_actor(&body.actor);
                    }
                }
                PushedEntity::Native {
                    slot,
                    origin,
                    angles,
                    yaw,
                } => {
                    let slot = *slot;
                    let has_client = driver.pool().entity(slot).and_then(|entity| entity.client);
                    if let Some(entity) = driver.pool().entity_mut(slot) {
                        entity.s.pos.base = *origin;
                        entity.s.apos.base = *angles;
                    }
                    if let Some(client) = has_client {
                        let Some(yaw) = yaw else {
                            return Err(failure("pushed entity acquired a client during the transaction"));
                        };
                        if let Some(client) = driver.pool().client_mut(client) {
                            client.ps.delta_angles[1] = qvm_float_to_int(*yaw);
                            client.ps.origin = *origin;
                        }
                    }
                    driver.world().link(slot);
                }
            }
        }
        Ok(())
    }

    fn push_proximity_mine(
        &self,
        driver: &mut dyn Q3Driver,
        slot: usize,
        pusher: &PusherSnapshot,
        mv: Vec3,
        amove: Vec3,
    ) -> Result<bool, Q3GameError> {
        {
            let entity = driver
                .pool()
                .entity_mut(slot)
                .ok_or_else(|| failure("Mover entity does not belong to this pool"))?;
            let axes = angle_vectors(sub3(vec3(0.0, 0.0, 0.0), amove));
            entity.s.pos.base = add3(entity.s.pos.base, mv);
            let org = sub3(entity.s.pos.base, pusher.current_origin);
            let rotated = vec3(dot3(org, axes.forward), -dot3(org, axes.right), dot3(org, axes.up));
            entity.s.pos.base = add3(entity.s.pos.base, sub3(rotated, org));
        }
        if !self.check_proximity_position(driver, slot)? {
            return Ok(false);
        }
        {
            let entity = driver
                .pool()
                .entity_mut(slot)
                .ok_or_else(|| failure("Mover entity does not belong to this pool"))?;
            entity.r.current_origin = entity.s.pos.base;
        }
        driver.world().link(slot);
        Ok(true)
    }

    fn push_part(
        &self,
        driver: &mut dyn Q3Driver,
        pusher_slot: usize,
        mv: Vec3,
        amove: Vec3,
        pushed: &mut Vec<PushedEntity>,
    ) -> Result<Option<Participant>, Q3GameError> {
        let pusher = mover_snapshot(driver.pool(), pusher_slot)?;
        let rotating = pusher.current_angles.x != 0.0
            || pusher.current_angles.y != 0.0
            || pusher.current_angles.z != 0.0
            || amove.x != 0.0
            || amove.y != 0.0
            || amove.z != 0.0;
        let (destination, total) = if rotating {
            let radius = radius_from_bounds(Bounds {
                min: pusher.mins,
                max: pusher.maxs,
            });
            let extent = vec3(radius, radius, radius);
            let position = add3(pusher.current_origin, mv);
            let destination = Bounds {
                min: sub3(position, extent),
                max: add3(position, extent),
            };
            let total = Bounds {
                min: sub3(destination.min, mv),
                max: sub3(destination.max, mv),
            };
            (destination, total)
        } else {
            let bounds = Bounds {
                min: pusher.absmin,
                max: pusher.absmax,
            };
            let destination = Bounds {
                min: add3(bounds.min, mv),
                max: add3(bounds.max, mv),
            };
            let total = Bounds {
                min: add3(bounds.min, vec3(mv.x.min(0.0), mv.y.min(0.0), mv.z.min(0.0))),
                max: add3(bounds.max, vec3(mv.x.max(0.0), mv.y.max(0.0), mv.z.max(0.0))),
            };
            (destination, total)
        };
        driver.world().unlink(pusher_slot);
        let actors = driver.spatial().area_actors(&total, MAX_GENTITIES);
        {
            let entity = driver
                .pool()
                .entity_mut(pusher_slot)
                .ok_or_else(|| failure("Mover entity does not belong to this pool"))?;
            entity.r.current_origin = add3(entity.r.current_origin, mv);
            entity.r.current_angles = add3(entity.r.current_angles, amove);
        }
        driver.world().link(pusher_slot);
        let pusher = mover_snapshot(driver.pool(), pusher_slot)?;
        for actor in actors {
            let observed = driver.mover_actors().observe(&actor);
            let Some(observed) = observed else { continue };
            if observed.kind == SharedBodyKind::Attached {
                continue;
            }
            let native = driver.mover_actors().native_slot(&actor);
            if native.is_none() {
                if observed.kind == SharedBodyKind::Fixed {
                    continue;
                }
                let rider = observed
                    .state
                    .ground
                    .as_ref()
                    .is_some_and(|ground| ground == &pusher.actor);
                if !rider {
                    let bounds = observed.absolute_bounds;
                    if bounds.min.x >= destination.max.x
                        || bounds.min.y >= destination.max.y
                        || bounds.min.z >= destination.max.z
                        || bounds.max.x <= destination.min.x
                        || bounds.max.y <= destination.min.y
                        || bounds.max.z <= destination.min.z
                    {
                        continue;
                    }
                    if !self.shared_position_blocked(driver, &observed) {
                        continue;
                    }
                }
                if self.try_pushing_shared(driver, &observed, &pusher, mv, amove, pushed)? {
                    continue;
                }
                let participant = driver.mover_actors().participant(&actor);
                if pusher.pos_type == TrajectoryType::Sine || pusher.apos_type == TrajectoryType::Sine {
                    let host = Participant::Entity(pusher_slot);
                    driver
                        .combat()
                        .damage(&participant, Some(&host), Some(&host), None, None, 99999, 0, MOD_CRUSH);
                    continue;
                }
                self.restore_pushed(driver, pushed)?;
                return Ok(Some(participant));
            }
            let check = native.unwrap_or(usize::MAX);
            let is_missionpack = driver.combat().product() == Q3Product::Missionpack;
            if is_missionpack {
                let (e_type, classname, enemy) = match driver.pool().entity(check) {
                    Some(entity) => (
                        entity.s.e_type,
                        entity.classname_value().map(str::to_string),
                        entity.enemy,
                    ),
                    None => continue,
                };
                if e_type == Q3EntityType::Missile as i32 && classname.as_deref() == Some("prox mine") {
                    let clear = if enemy == Some(pusher_slot) {
                        self.push_proximity_mine(driver, check, &pusher, mv, amove)?
                    } else {
                        self.check_proximity_position(driver, check)?
                    };
                    if !clear {
                        if let Some(entity) = driver.pool().entity_mut(check) {
                            entity.s.loop_sound = 0;
                        }
                        driver.pool().add_event(check, Q3EntityEvent::ProximityMineTrigger, 0);
                        driver.explode_missile(check);
                        let activator = driver.pool().entity(check).and_then(|entity| entity.activator);
                        if let Some(activator) = activator {
                            driver.pool().free_entity(activator);
                            if let Some(entity) = driver.pool().entity_mut(check) {
                                entity.activator = None;
                            }
                        }
                    }
                    continue;
                }
            }
            let (e_type, physics_object) = match driver.pool().entity(check) {
                Some(entity) => (entity.s.e_type, entity.physics_object),
                None => continue,
            };
            if e_type != Q3EntityType::Item as i32 && e_type != Q3EntityType::Player as i32 && !physics_object {
                continue;
            }
            let rider = driver
                .pool()
                .entity(check)
                .is_some_and(|entity| rides(entity, &pusher.actor));
            if !rider {
                let inside = match driver.pool().entity(check) {
                    Some(entity) => {
                        let bounds = Bounds {
                            min: entity.r.absmin(),
                            max: entity.r.absmax(),
                        };
                        bounds.min.x < destination.max.x
                            && bounds.min.y < destination.max.y
                            && bounds.min.z < destination.max.z
                            && bounds.max.x > destination.min.x
                            && bounds.max.y > destination.min.y
                            && bounds.max.z > destination.min.z
                    }
                    None => false,
                };
                if !inside {
                    continue;
                }
                if self.test_entity_position(driver, check)?.is_none() {
                    continue;
                }
            }
            if self.try_pushing(driver, check, &pusher, mv, amove, pushed)? {
                continue;
            }
            if pusher.pos_type == TrajectoryType::Sine || pusher.apos_type == TrajectoryType::Sine {
                let target = Participant::Entity(check);
                let host = Participant::Entity(pusher_slot);
                driver
                    .combat()
                    .damage(&target, Some(&host), Some(&host), None, None, 99999, 0, MOD_CRUSH);
                continue;
            }
            self.restore_pushed(driver, pushed)?;
            return Ok(Some(Participant::Entity(check)));
        }
        Ok(None)
    }

    /// Run a mover team (`runTeam`).
    pub fn run_team(&self, driver: &mut dyn Q3Driver, slot: usize) -> Result<(), Q3GameError> {
        self.check_product(driver)?;
        self.owned(driver, slot)?;
        let mut pushed = Vec::new();
        let time = driver.combat().time();
        let mut obstacle = None;
        let mut part = Some(slot);
        while let Some(current) = part {
            let (pos, apos, origin, angles) = {
                let entity = driver
                    .pool()
                    .entity(current)
                    .ok_or_else(|| failure("Mover entity does not belong to this pool"))?;
                (
                    entity.s.pos,
                    entity.s.apos,
                    entity.r.current_origin,
                    entity.r.current_angles,
                )
            };
            let mv = sub3(evaluate_trajectory(&pos, time), origin);
            let amove = sub3(evaluate_trajectory(&apos, time), angles);
            obstacle = self.push_part(driver, current, mv, amove, &mut pushed)?;
            if obstacle.is_some() {
                break;
            }
            part = driver.pool().entity(current).and_then(|entity| entity.teamchain);
        }
        if let Some(obstacle) = obstacle {
            let elapsed = time.wrapping_sub(self.previous_time);
            let mut part = Some(slot);
            while let Some(current) = part {
                {
                    let entity = driver
                        .pool()
                        .entity_mut(current)
                        .ok_or_else(|| failure("Mover entity does not belong to this pool"))?;
                    entity.s.pos.time = entity.s.pos.time.wrapping_add(elapsed);
                    entity.s.apos.time = entity.s.apos.time.wrapping_add(elapsed);
                    entity.r.current_origin = evaluate_trajectory(&entity.s.pos, time);
                    entity.r.current_angles = evaluate_trajectory(&entity.s.apos, time);
                }
                driver.world().link(current);
                part = driver.pool().entity(current).and_then(|entity| entity.teamchain);
            }
            let blocked = driver.pool().entity(slot).and_then(|entity| entity.blocked.clone());
            if let Some(blocked) = blocked {
                blocked(driver, slot, &obstacle);
            }
            return Ok(());
        }
        let mut part = Some(slot);
        while let Some(current) = part {
            let (pos_type, pos_time, pos_duration, reached) = {
                let entity = driver
                    .pool()
                    .entity(current)
                    .ok_or_else(|| failure("Mover entity does not belong to this pool"))?;
                (
                    entity.s.pos.trajectory_type,
                    entity.s.pos.time,
                    entity.s.pos.duration,
                    entity.reached.clone(),
                )
            };
            if pos_type == TrajectoryType::LinearStop && time >= pos_time.wrapping_add(pos_duration) {
                if let Some(reached) = reached {
                    reached(driver, current);
                }
            }
            part = driver.pool().entity(current).and_then(|entity| entity.teamchain);
        }
        Ok(())
    }

    /// Run a mover (`run`).
    pub fn run(&self, driver: &mut dyn Q3Driver, slot: usize) -> Result<(), Q3GameError> {
        self.check_product(driver)?;
        self.owned(driver, slot)?;
        let entity = driver
            .pool()
            .entity(slot)
            .ok_or_else(|| failure("Mover entity does not belong to this pool"))?;
        if entity.flags & GameFlags::TEAMSLAVE != 0 {
            return Ok(());
        }
        let moving = entity.s.pos.trajectory_type != TrajectoryType::Stationary
            || entity.s.apos.trajectory_type != TrajectoryType::Stationary;
        if moving {
            self.run_team(driver, slot)?;
        }
        let time = driver.combat().time();
        run_think(driver, slot, time)
    }

    /// Set a mover state (`setState`).
    pub fn set_state(
        &self,
        driver: &mut dyn Q3Driver,
        slot: usize,
        state: MoverState,
        time: i32,
    ) -> Result<(), Q3GameError> {
        self.check_product(driver)?;
        self.owned(driver, slot)?;
        let now = driver.combat().time();
        {
            let entity = driver
                .pool()
                .entity_mut(slot)
                .ok_or_else(|| failure("Mover entity does not belong to this pool"))?;
            entity.mover_state = state as i32;
            let previous = entity.s.pos;
            entity.s.pos = match state {
                MoverState::Pos1 => Q3Trajectory {
                    trajectory_type: TrajectoryType::Stationary,
                    time,
                    base: entity.pos1,
                    ..previous
                },
                MoverState::Pos2 => Q3Trajectory {
                    trajectory_type: TrajectoryType::Stationary,
                    time,
                    base: entity.pos2,
                    ..previous
                },
                MoverState::OneToTwo => Q3Trajectory {
                    trajectory_type: TrajectoryType::LinearStop,
                    time,
                    base: entity.pos1,
                    delta: scale3(sub3(entity.pos2, entity.pos1), 1000.0 / previous.duration as f32),
                    ..previous
                },
                MoverState::TwoToOne => Q3Trajectory {
                    trajectory_type: TrajectoryType::LinearStop,
                    time,
                    base: entity.pos2,
                    delta: scale3(sub3(entity.pos1, entity.pos2), 1000.0 / previous.duration as f32),
                    ..previous
                },
            };
            entity.r.current_origin = evaluate_trajectory(&entity.s.pos, now);
        }
        driver.world().link(slot);
        Ok(())
    }

    /// Match a team to a state (`matchTeam`).
    pub fn match_team(
        &self,
        driver: &mut dyn Q3Driver,
        leader: usize,
        state: MoverState,
        time: i32,
    ) -> Result<(), Q3GameError> {
        self.check_product(driver)?;
        self.owned(driver, leader)?;
        let mut part = Some(leader);
        while let Some(current) = part {
            self.set_state(driver, current, state, time)?;
            part = driver.pool().entity(current).and_then(|entity| entity.teamchain);
        }
        Ok(())
    }

    /// Return a mover to position 1 (`returnToPos1`).
    pub fn return_to_pos1(&self, driver: &mut dyn Q3Driver, slot: usize) -> Result<(), Q3GameError> {
        let time = driver.combat().time();
        self.match_team(driver, slot, MoverState::TwoToOne, time)?;
        let (sound_loop, sound2to1) = {
            let entity = driver
                .pool()
                .entity(slot)
                .ok_or_else(|| failure("Mover entity does not belong to this pool"))?;
            (entity.sound_loop, entity.sound2to1)
        };
        if let Some(entity) = driver.pool().entity_mut(slot) {
            entity.s.loop_sound = sound_loop;
        }
        if sound2to1 != 0 {
            driver.pool().add_event(slot, Q3EntityEvent::GeneralSound, sound2to1);
        }
        Ok(())
    }

    /// Binary-mover reached handler (`reachedBinary`).
    pub fn reached_binary(&self, driver: &mut dyn Q3Driver, slot: usize) -> Result<(), Q3GameError> {
        self.check_product(driver)?;
        self.owned(driver, slot)?;
        let time = driver.combat().time();
        let sound_loop = driver.pool().entity(slot).map(|entity| entity.sound_loop).unwrap_or(0);
        if let Some(entity) = driver.pool().entity_mut(slot) {
            entity.s.loop_sound = sound_loop;
        }
        let mover_state = driver.pool().entity(slot).map(|entity| entity.mover_state).unwrap_or(0);
        if mover_state == MoverState::OneToTwo as i32 {
            self.set_state(driver, slot, MoverState::Pos2, time)?;
            let (sound_pos2, wait) = {
                let entity = driver
                    .pool()
                    .entity(slot)
                    .ok_or_else(|| failure("Mover entity does not belong to this pool"))?;
                (entity.sound_pos2, entity.wait)
            };
            if sound_pos2 != 0 {
                driver.pool().add_event(slot, Q3EntityEvent::GeneralSound, sound_pos2);
            }
            let think = driver
                .pool()
                .callbacks()
                .think
                .resolve(Some("q3.base.game.mover.reachedBinary.think"))?;
            if let Some(entity) = driver.pool().entity_mut(slot) {
                entity.think = think;
            }
            driver.pool().set_nextthink(slot, qvm_float_to_int(time as f32 + wait));
            let activation_missing = driver
                .pool()
                .entity(slot)
                .is_some_and(|entity| entity.activation.is_none());
            if activation_missing {
                if let Some(entity) = driver.pool().entity_mut(slot) {
                    entity.activation = Some(Participant::Entity(slot));
                }
            }
            let activation = driver.pool().entity(slot).and_then(|entity| entity.activation.clone());
            driver.use_targets(slot, activation);
            Ok(())
        } else if mover_state == MoverState::TwoToOne as i32 {
            self.set_state(driver, slot, MoverState::Pos1, time)?;
            let (sound_pos1, teammaster) = {
                let entity = driver
                    .pool()
                    .entity(slot)
                    .ok_or_else(|| failure("Mover entity does not belong to this pool"))?;
                (entity.sound_pos1, entity.teammaster)
            };
            if sound_pos1 != 0 {
                driver.pool().add_event(slot, Q3EntityEvent::GeneralSound, sound_pos1);
            }
            if teammaster.is_none() || teammaster == Some(slot) {
                driver.adjust_area_portal(slot, false);
            }
            Ok(())
        } else {
            Err(failure("Reached_BinaryMover: bad moverState"))
        }
    }

    /// Binary-mover use handler (`useBinary`).
    pub fn use_binary(
        &self,
        driver: &mut dyn Q3Driver,
        slot: usize,
        other: Option<Participant>,
        activator: Option<Participant>,
    ) -> Result<(), Q3GameError> {
        self.check_product(driver)?;
        self.owned(driver, slot)?;
        let flags = driver.pool().entity(slot).map(|entity| entity.flags).unwrap_or(0);
        if flags & GameFlags::TEAMSLAVE != 0 {
            let master = driver.pool().entity(slot).and_then(|entity| entity.teammaster);
            let Some(master) = master else {
                return Err(failure("Mover team slave has no team master"));
            };
            return self.use_binary(driver, master, other, activator);
        }
        let time = driver.combat().time();
        if let Some(entity) = driver.pool().entity_mut(slot) {
            entity.activation = activator;
        }
        let mover_state = driver.pool().entity(slot).map(|entity| entity.mover_state).unwrap_or(0);
        if mover_state == MoverState::Pos1 as i32 {
            self.match_team(driver, slot, MoverState::OneToTwo, time.wrapping_add(50))?;
            let (sound1to2, sound_loop, teammaster) = {
                let entity = driver
                    .pool()
                    .entity(slot)
                    .ok_or_else(|| failure("Mover entity does not belong to this pool"))?;
                (entity.sound1to2, entity.sound_loop, entity.teammaster)
            };
            if sound1to2 != 0 {
                driver.pool().add_event(slot, Q3EntityEvent::GeneralSound, sound1to2);
            }
            if let Some(entity) = driver.pool().entity_mut(slot) {
                entity.s.loop_sound = sound_loop;
            }
            if teammaster.is_none() || teammaster == Some(slot) {
                driver.adjust_area_portal(slot, true);
            }
            Ok(())
        } else if mover_state == MoverState::Pos2 as i32 {
            let wait = driver.pool().entity(slot).map(|entity| entity.wait).unwrap_or(0.0);
            driver.pool().set_nextthink(slot, qvm_float_to_int(time as f32 + wait));
            Ok(())
        } else {
            let (duration, pos_time) = {
                let entity = driver
                    .pool()
                    .entity(slot)
                    .ok_or_else(|| failure("Mover entity does not belong to this pool"))?;
                (entity.s.pos.duration, entity.s.pos.time)
            };
            let partial = (time.wrapping_sub(pos_time)).min(duration);
            let state = if mover_state == MoverState::TwoToOne as i32 {
                MoverState::OneToTwo
            } else {
                MoverState::TwoToOne
            };
            self.match_team(driver, slot, state, time.wrapping_sub(duration.wrapping_sub(partial)))?;
            let sound = {
                let entity = driver
                    .pool()
                    .entity(slot)
                    .ok_or_else(|| failure("Mover entity does not belong to this pool"))?;
                if state == MoverState::OneToTwo {
                    entity.sound1to2
                } else {
                    entity.sound2to1
                }
            };
            if sound != 0 {
                driver.pool().add_event(slot, Q3EntityEvent::GeneralSound, sound);
            }
            Ok(())
        }
    }

    /// Initialize a binary mover (`initializeBinary`).
    pub fn initialize_binary(
        &self,
        driver: &mut dyn Q3Driver,
        slot: usize,
        variables: &SpawnVariables,
    ) -> Result<(), Q3GameError> {
        self.check_product(driver)?;
        self.owned(driver, slot)?;
        let model2 = driver.pool().entity(slot).and_then(|entity| entity.model2.clone());
        if let Some(model2) = model2 {
            let index = driver.model_index(Some(&model2));
            if let Some(entity) = driver.pool().entity_mut(slot) {
                entity.s.modelindex2 = index;
            }
        }
        let noise = variables.string("noise", "100");
        if noise.present {
            let index = driver.sound_index(&noise.value);
            if let Some(entity) = driver.pool().entity_mut(slot) {
                entity.s.loop_sound = index;
            }
        }
        let light = variables.float("light", "100")?;
        let color = variables.vector("color", "1 1 1")?;
        if light.present || color.present {
            let component = |value: f32| -> i32 { 255.min(qvm_float_to_int(value * 255.0)) };
            let intensity = 255.min(qvm_float_to_int(light.value / 4.0));
            let packed = component(color.value.x)
                | (component(color.value.y) << 8)
                | (component(color.value.z) << 16)
                | (intensity << 24);
            if let Some(entity) = driver.pool().entity_mut(slot) {
                entity.s.constant_light = packed;
            }
        }
        let use_callback = driver
            .pool()
            .callbacks()
            .use_callbacks
            .resolve(Some("q3.base.game.mover.initializeBinary.use"))?;
        let reached = driver
            .pool()
            .callbacks()
            .reached
            .resolve(Some("q3.base.game.mover.initializeBinary.reached"))?;
        {
            let entity = driver
                .pool()
                .entity_mut(slot)
                .ok_or_else(|| failure("Mover entity does not belong to this pool"))?;
            entity.use_callback = use_callback;
            entity.reached = reached;
            entity.mover_state = MoverState::Pos1 as i32;
            entity.r.sv_flags = ServerEntityFlags::USE_CURRENT_ORIGIN;
            entity.s.e_type = Q3EntityType::Mover as i32;
            entity.r.current_origin = entity.pos1;
        }
        driver.world().link(slot);
        let (pos1, pos2, speed) = {
            let entity = driver
                .pool()
                .entity(slot)
                .ok_or_else(|| failure("Mover entity does not belong to this pool"))?;
            (entity.pos1, entity.pos2, entity.speed)
        };
        let distance = length3(sub3(pos2, pos1));
        let speed = if speed == 0.0 { 100.0 } else { speed };
        let duration = qvm_float_to_int(distance * 1000.0 / speed).max(1);
        if let Some(entity) = driver.pool().entity_mut(slot) {
            entity.speed = speed;
            let previous = entity.s.pos;
            entity.s.pos = Q3Trajectory {
                trajectory_type: TrajectoryType::Stationary,
                base: entity.pos1,
                delta: scale3(sub3(entity.pos2, entity.pos1), speed),
                duration,
                ..previous
            };
        }
        Ok(())
    }

    /// Door blocked handler (`blockedDoor`).
    pub fn blocked_door(&self, driver: &mut dyn Q3Driver, slot: usize, other: &Participant) -> Result<(), Q3GameError> {
        self.check_product(driver)?;
        self.owned(driver, slot)?;
        match other {
            Participant::SharedActor(actor) => {
                let body = driver.mover_actors().observe(actor);
                let Some(body) = body else { return Ok(()) };
                if body.kind != SharedBodyKind::Player {
                    driver.pool().temp_entity(body.state.origin, Q3EntityEvent::ItemPop);
                    driver.mover_actors().release(&body.actor);
                    return Ok(());
                }
                let (damage, spawnflags) = {
                    let entity = driver
                        .pool()
                        .entity(slot)
                        .ok_or_else(|| failure("Mover entity does not belong to this pool"))?;
                    (entity.damage, entity.spawnflags)
                };
                if damage != 0 {
                    let host = Participant::Entity(slot);
                    driver
                        .combat()
                        .damage(other, Some(&host), Some(&host), None, None, damage, 0, MOD_CRUSH);
                }
                if spawnflags & 4 == 0 {
                    self.use_binary(driver, slot, Some(Participant::Entity(slot)), Some(other.clone()))?;
                }
                Ok(())
            }
            Participant::Entity(other_slot) => {
                let other_slot = *other_slot;
                self.owned(driver, other_slot)?;
                let (client, e_type, origin, item) = {
                    let entity = driver
                        .pool()
                        .entity(other_slot)
                        .ok_or_else(|| failure("Mover entity does not belong to this pool"))?;
                    (entity.client, entity.s.e_type, entity.s.origin, entity.item)
                };
                if client.is_none() {
                    if e_type == Q3EntityType::Item as i32 {
                        let Some(item) = item else {
                            return Err(failure("Blocked item has no item definition"));
                        };
                        let def = driver.item_at(item);
                        if def.is_none() {
                            return Err(failure("Blocked item has no item definition"));
                        }
                        if def.is_some_and(|def| def.item_type == Q3ItemType::Team) {
                            driver.return_dropped_flag(other_slot);
                            return Ok(());
                        }
                    }
                    driver.pool().temp_entity(origin, Q3EntityEvent::ItemPop);
                    driver.pool().free_entity(other_slot);
                    return Ok(());
                }
                let (damage, spawnflags) = {
                    let entity = driver
                        .pool()
                        .entity(slot)
                        .ok_or_else(|| failure("Mover entity does not belong to this pool"))?;
                    (entity.damage, entity.spawnflags)
                };
                if damage != 0 {
                    let host = Participant::Entity(slot);
                    driver
                        .combat()
                        .damage(other, Some(&host), Some(&host), None, None, damage, 0, MOD_CRUSH);
                }
                if spawnflags & 4 != 0 {
                    return Ok(());
                }
                self.use_binary(driver, slot, Some(Participant::Entity(slot)), Some(other.clone()))
            }
        }
    }

    /// Bind save callbacks (`bindSaveCallbacks`).
    pub fn bind_save_callbacks(
        runtime: &Rc<RefCell<MoverRuntime>>,
        driver: &mut dyn Q3Driver,
    ) -> Result<(), Q3GameError> {
        let think_runtime = Rc::clone(runtime);
        driver.pool().callbacks_mut().think.intern(
            "q3.base.game.mover.reachedBinary.think",
            Rc::new(move |driver, slot| {
                let result = think_runtime.borrow_mut().return_to_pos1(driver, slot);
                or_panic(result);
            }),
        )?;
        let use_runtime = Rc::clone(runtime);
        driver.pool().callbacks_mut().use_callbacks.intern(
            "q3.base.game.mover.initializeBinary.use",
            Rc::new(move |driver, slot, other, activator| {
                let result = use_runtime
                    .borrow_mut()
                    .use_binary(driver, slot, other.cloned(), activator.cloned());
                or_panic(result);
            }),
        )?;
        let reached_runtime = Rc::clone(runtime);
        driver.pool().callbacks_mut().reached.intern(
            "q3.base.game.mover.initializeBinary.reached",
            Rc::new(move |driver, slot| {
                let result = reached_runtime.borrow_mut().reached_binary(driver, slot);
                or_panic(result);
            }),
        )?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// personal-portal.ts: missionpack personal portals (g_misc.c)
// ---------------------------------------------------------------------------

/// Corpse contents (`CONTENTS_CORPSE`).
const CONTENTS_CORPSE: i32 = 0x4000000;
/// Trigger contents (`CONTENTS_TRIGGER`).
const CONTENTS_TRIGGER: i32 = 0x40000000;
/// Telefrag means of death (`MOD_TELEFRAG`).
const MOD_TELEFRAG: i32 = 18;
/// Portal health (`PORTAL_HEALTH`).
const PORTAL_HEALTH: i32 = 200;
/// Portal enable delay (`PORTAL_ENABLE_DELAY`).
const PORTAL_ENABLE_DELAY: i32 = 1_000;
/// Portal lifetime (`PORTAL_LIFETIME`).
const PORTAL_LIFETIME: i32 = 2 * 60 * 1_000;
/// Portal destination classname.
const PORTAL_DESTINATION: &str = "hi_portal destination";
/// Portal source classname.
const PORTAL_SOURCE: &str = "hi_portal source";

/// Personal portal runtime (`PersonalPortalRuntime`).
#[derive(Debug, Clone)]
pub struct PersonalPortalRuntime {
    portal_sequence: i32,
}

impl PersonalPortalRuntime {
    /// New runtime.
    #[must_use]
    pub fn new() -> Self {
        Self { portal_sequence: 0 }
    }

    /// Portal sequence.
    #[must_use]
    pub fn portal_sequence(&self) -> i32 {
        self.portal_sequence
    }

    /// Capture save state.
    #[must_use]
    pub fn capture_save_state(&self) -> SaveJson {
        obj(vec![("portalSequence", num_i32(self.portal_sequence))])
    }

    /// Restore save state.
    pub fn restore_save_state(&mut self, value: &SaveJson) -> Result<(), Q3GameError> {
        let reader = SaveReader::at(value, "q3.portals");
        let sequence = reader.field("portalSequence").integer(-2_147_483_648)?;
        if sequence > 2_147_483_647 {
            return Err(reader.fail("portal sequence exceeds source integer range").into());
        }
        #[allow(clippy::cast_possible_truncation)]
        {
            self.portal_sequence = sequence as i32;
        }
        Ok(())
    }

    fn check_host(&self, driver: &mut dyn Q3Driver) -> Result<(), Q3GameError> {
        if driver.combat().product() != Q3Product::Missionpack {
            return Err(failure("Personal portals require a missionpack entity pool"));
        }
        Ok(())
    }

    fn owned(&self, driver: &mut dyn Q3Driver, slot: usize) -> Result<(), Q3GameError> {
        if driver.pool().entity(slot).is_none() {
            return Err(failure(
                "Personal portal entity does not belong to its entity pool or was replaced",
            ));
        }
        Ok(())
    }

    fn player(&self, driver: &mut dyn Q3Driver, slot: usize) -> Result<usize, Q3GameError> {
        self.owned(driver, slot)?;
        driver
            .pool()
            .entity(slot)
            .and_then(|entity| entity.client)
            .ok_or_else(|| failure("Personal portal use requires a client entity"))
    }

    fn free_portal(&self, driver: &mut dyn Q3Driver, slot: usize) -> Result<(), Q3GameError> {
        self.owned(driver, slot)?;
        driver.pool().free_entity(slot);
        Ok(())
    }

    fn destination(&self, driver: &mut dyn Q3Driver, sequence: i32) -> Option<usize> {
        let mut current = None;
        loop {
            current = find_entity(
                driver.pool(),
                current,
                EntityStringField::Classname,
                Some(PORTAL_DESTINATION),
            );
            let Some(slot) = current else { return None };
            if driver
                .pool()
                .entity(slot)
                .is_some_and(|entity| entity.count == sequence)
            {
                return Some(slot);
            }
        }
    }

    fn portal_die(&self, driver: &mut dyn Q3Driver, slot: usize) -> Result<(), Q3GameError> {
        self.free_portal(driver, slot)
    }

    fn drop_carried_flag(&self, driver: &mut dyn Q3Driver, player: usize) -> Result<(), Q3GameError> {
        if driver.map_travel_mode() {
            driver.map_travel_drop_flag(player);
            return Ok(());
        }
        let client = driver
            .pool()
            .entity(player)
            .and_then(|entity| entity.client)
            .ok_or_else(|| failure("Portal touch requires a client entity"))?;
        let powerup = {
            let client = driver
                .pool()
                .client(client)
                .ok_or_else(|| failure("Portal touch requires a client entity"))?;
            if client.ps.powerups.get(Q3Powerup::Neutralflag as usize) != 0 {
                Q3Powerup::Neutralflag as i32
            } else if client.ps.powerups.get(Q3Powerup::Redflag as usize) != 0 {
                Q3Powerup::Redflag as i32
            } else if client.ps.powerups.get(Q3Powerup::Blueflag as usize) != 0 {
                Q3Powerup::Blueflag as i32
            } else {
                Q3Powerup::None as i32
            }
        };
        if powerup == Q3Powerup::None as i32 {
            return Ok(());
        }
        let Some(item) = driver.find_item_for_powerup(powerup) else {
            return Err(failure(format!(
                "Portal carried flag {powerup} is absent from the missionpack item table"
            )));
        };
        driver.drop_item(player, item, 0);
        if let Some(client) = driver.pool().client_mut(client) {
            client.ps.powerups.set(powerup as usize, 0);
        }
        Ok(())
    }

    fn portal_touch(&self, driver: &mut dyn Q3Driver, source: usize, other: usize) -> Result<(), Q3GameError> {
        self.owned(driver, source)?;
        let (health, client) = {
            let entity = driver
                .pool()
                .entity(other)
                .ok_or_else(|| failure("Personal portal entity does not belong to its entity pool or was replaced"))?;
            (entity.health, entity.client)
        };
        if health <= 0 || client.is_none() {
            return Ok(());
        }
        self.owned(driver, other)?;
        self.drop_carried_flag(driver, other)?;
        let count = driver.pool().entity(source).map(|entity| entity.count).unwrap_or(0);
        let destination = self.destination(driver, count);
        let Some(destination) = destination else {
            let (pos1, angles) = {
                let entity = driver.pool().entity(source).ok_or_else(|| {
                    failure("Personal portal entity does not belong to its entity pool or was replaced")
                })?;
                (entity.pos1, entity.s.angles)
            };
            if pos1.x != 0.0 || pos1.y != 0.0 || pos1.z != 0.0 {
                self.teleport(driver, other, pos1, angles);
            }
            let target = Participant::Entity(other);
            driver.combat().damage(
                &target,
                Some(&target.clone()),
                Some(&target),
                None,
                None,
                100_000,
                DamageFlags::NO_PROTECTION,
                MOD_TELEFRAG,
            );
            return Ok(());
        };
        let (origin, angles) = {
            let entity = driver
                .pool()
                .entity(destination)
                .ok_or_else(|| failure("Personal portal entity does not belong to its entity pool or was replaced"))?;
            (entity.s.pos.base, entity.s.angles)
        };
        self.teleport(driver, other, origin, angles);
        Ok(())
    }

    fn teleport(&self, driver: &mut dyn Q3Driver, player: usize, origin: Vec3, angles: Vec3) {
        if driver.map_travel_mode() {
            driver.map_travel_teleport(player, origin, angles);
        } else {
            driver.teleport_player(player, origin, angles);
        }
    }

    fn portal_enable(&self, driver: &mut dyn Q3Driver, slot: usize) -> Result<(), Q3GameError> {
        self.owned(driver, slot)?;
        let touch = driver
            .pool()
            .callbacks()
            .touch
            .resolve(Some("q3.base.game.personal-portal.portalEnable.touch"))?;
        let think = driver
            .pool()
            .callbacks()
            .think
            .resolve(Some("q3.base.game.personal-portal.portalEnable.think"))?;
        if let Some(entity) = driver.pool().entity_mut(slot) {
            entity.touch = touch;
            entity.think = think;
        }
        let time = driver.combat().time();
        driver.pool().set_nextthink(slot, time.wrapping_add(PORTAL_LIFETIME));
        Ok(())
    }

    /// Drop a portal destination (`dropPortalDestination`).
    pub fn drop_portal_destination(&mut self, driver: &mut dyn Q3Driver, player: usize) -> Result<(), Q3GameError> {
        self.check_host(driver)?;
        let client = self.player(driver, player)?;
        let (pos_base, mins, maxs, apos_base) = {
            let entity = driver
                .pool()
                .entity(player)
                .ok_or_else(|| failure("Personal portal entity does not belong to its entity pool or was replaced"))?;
            (entity.s.pos.base, entity.r.mins, entity.r.maxs, entity.s.apos.base)
        };
        let portal = driver.pool().spawn_entity()?;
        let model = driver.model_index(Some("models/powerups/teleporter/tele_exit.md3"));
        let die = driver
            .pool()
            .callbacks()
            .die
            .resolve(Some("q3.base.game.personal-portal.dropPortalDestination.die"))?;
        let think = driver
            .pool()
            .callbacks()
            .think
            .resolve(Some("q3.base.game.personal-portal.portalEnable.think"))?;
        let time = driver.combat().time();
        {
            let entity = driver
                .pool()
                .entity_mut(portal)
                .ok_or_else(|| failure("Personal portal entity does not belong to its entity pool or was replaced"))?;
            entity.s.modelindex = model;
            set_origin(entity, snap_vector(pos_base));
            entity.r.mins = mins;
            entity.r.maxs = maxs;
            entity.set_classname(Some(PORTAL_DESTINATION.to_string()));
            entity.r.contents = CONTENTS_CORPSE;
            entity.takedamage = true;
            entity.health = PORTAL_HEALTH;
            entity.die = die;
            entity.s.angles = apos_base;
            entity.think = think;
        }
        driver.pool().set_nextthink(portal, time.wrapping_add(PORTAL_LIFETIME));
        driver.world().link(portal);
        self.portal_sequence = self.portal_sequence.wrapping_add(1);
        let sequence = self.portal_sequence;
        if let Some(client) = driver.pool().client_mut(client) {
            client.portal_id = sequence;
        }
        if let Some(entity) = driver.pool().entity_mut(portal) {
            entity.count = sequence;
        }
        let Some(item) = driver.find_item("Portal") else {
            return Err(failure("Portal holdable is absent from the missionpack item table"));
        };
        if item < 1 {
            return Err(failure("Portal holdable has no missionpack item index"));
        }
        if let Some(client) = driver.pool().client_mut(client) {
            let slot = stat_schema(Q3Product::Missionpack).holdable_item;
            client.ps.stats.set(slot, item as i32);
        }
        Ok(())
    }

    /// Drop a portal source (`dropPortalSource`).
    pub fn drop_portal_source(&mut self, driver: &mut dyn Q3Driver, player: usize) -> Result<(), Q3GameError> {
        self.check_host(driver)?;
        let client = self.player(driver, player)?;
        let (pos_base, mins, maxs) = {
            let entity = driver
                .pool()
                .entity(player)
                .ok_or_else(|| failure("Personal portal entity does not belong to its entity pool or was replaced"))?;
            (entity.s.pos.base, entity.r.mins, entity.r.maxs)
        };
        let portal = driver.pool().spawn_entity()?;
        let model = driver.model_index(Some("models/powerups/teleporter/tele_enter.md3"));
        let die = driver
            .pool()
            .callbacks()
            .die
            .resolve(Some("q3.base.game.personal-portal.dropPortalDestination.die"))?;
        let time = driver.combat().time();
        {
            let entity = driver
                .pool()
                .entity_mut(portal)
                .ok_or_else(|| failure("Personal portal entity does not belong to its entity pool or was replaced"))?;
            entity.s.modelindex = model;
            set_origin(entity, snap_vector(pos_base));
            entity.r.mins = mins;
            entity.r.maxs = maxs;
            entity.set_classname(Some(PORTAL_SOURCE.to_string()));
            entity.r.contents = CONTENTS_CORPSE | CONTENTS_TRIGGER;
            entity.takedamage = true;
            entity.health = PORTAL_HEALTH;
            entity.die = die;
        }
        driver.world().link(portal);
        let portal_id = driver.pool().client(client).map(|client| client.portal_id).unwrap_or(0);
        if let Some(entity) = driver.pool().entity_mut(portal) {
            entity.count = portal_id;
        }
        if let Some(client) = driver.pool().client_mut(client) {
            client.portal_id = 0;
        }
        let think = driver
            .pool()
            .callbacks()
            .think
            .resolve(Some("q3.base.game.personal-portal.dropPortalSource.think"))?;
        if let Some(entity) = driver.pool().entity_mut(portal) {
            entity.think = think;
        }
        driver
            .pool()
            .set_nextthink(portal, time.wrapping_add(PORTAL_ENABLE_DELAY));
        if let Some(destination) = self.destination(driver, portal_id) {
            let origin = driver.pool().entity(destination).map(|entity| entity.s.pos.base);
            if let (Some(origin), Some(entity)) = (origin, driver.pool().entity_mut(portal)) {
                entity.pos1 = origin;
            }
        }
        Ok(())
    }

    /// Bind save callbacks (`bindSaveCallbacks`).
    pub fn bind_save_callbacks(
        runtime: &Rc<RefCell<PersonalPortalRuntime>>,
        driver: &mut dyn Q3Driver,
    ) -> Result<(), Q3GameError> {
        let touch_runtime = Rc::clone(runtime);
        driver.pool().callbacks_mut().touch.intern(
            "q3.base.game.personal-portal.portalEnable.touch",
            Rc::new(move |driver, slot, other, _contact| {
                if let Participant::Entity(other) = other {
                    let result = touch_runtime.borrow().portal_touch(driver, slot, *other);
                    or_panic(result);
                }
            }),
        )?;
        let free_runtime = Rc::clone(runtime);
        driver.pool().callbacks_mut().think.intern(
            "q3.base.game.personal-portal.portalEnable.think",
            Rc::new(move |driver, slot| {
                let result = free_runtime.borrow().free_portal(driver, slot);
                or_panic(result);
            }),
        )?;
        let die_runtime = Rc::clone(runtime);
        driver.pool().callbacks_mut().die.intern(
            "q3.base.game.personal-portal.dropPortalDestination.die",
            Rc::new(move |driver, slot, _inflictor, _attacker, _damage, _method| {
                let result = die_runtime.borrow().portal_die(driver, slot);
                or_panic(result);
            }),
        )?;
        let enable_runtime = Rc::clone(runtime);
        driver.pool().callbacks_mut().think.intern(
            "q3.base.game.personal-portal.dropPortalSource.think",
            Rc::new(move |driver, slot| {
                let result = enable_runtime.borrow().portal_enable(driver, slot);
                or_panic(result);
            }),
        )?;
        Ok(())
    }
}

impl Default for PersonalPortalRuntime {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// targets.ts: target entities (g_target.c)
// ---------------------------------------------------------------------------

/// Locations configstring base (`CS_LOCATIONS`).
pub const CS_LOCATIONS: i32 = 608;
/// Target laser trace mask (`MASK_TARGET_LASER`).
const MASK_TARGET_LASER: i32 = 0x1 | 0x2000000 | 0x4000000;
/// Target laser means of death (`MOD_TARGET_LASER`).
const MOD_TARGET_LASER: i32 = 21;
/// Broadcast server-command target.
pub const SERVER_COMMAND_BROADCAST: i32 = -1;

/// Target location state (`TargetLocationState`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetLocationState {
    /// Linked.
    pub linked: bool,
    /// Head slot.
    pub head: Option<usize>,
}

impl TargetLocationState {
    /// New state.
    #[must_use]
    pub fn new() -> Self {
        Self {
            linked: false,
            head: None,
        }
    }

    /// Capture save state.
    #[must_use]
    pub fn capture_save_state(&self) -> SaveJson {
        obj(vec![
            ("linked", boolean(self.linked)),
            ("head", opt_slot_to_json(self.head)),
        ])
    }

    /// Restore save state.
    pub fn restore_save_state(&mut self, value: &SaveJson, pool: &dyn EntityPool) -> Result<(), Q3GameError> {
        let reader = SaveReader::at(value, "q3.locations");
        self.linked = reader.field("linked").boolean()?;
        self.head = reader
            .field("head")
            .nullable(|entry| read_module_entity(&entry, pool))?;
        Ok(())
    }

    /// Reset.
    pub fn reset(&mut self) {
        self.linked = false;
        self.head = None;
    }
}

impl Default for TargetLocationState {
    fn default() -> Self {
        Self::new()
    }
}

fn target_owned(driver: &mut dyn Q3Driver, slot: usize) -> Result<(), Q3GameError> {
    if driver.pool().entity(slot).is_none() {
        return Err(failure(
            "Target entity does not belong to its entity pool or was replaced",
        ));
    }
    Ok(())
}

fn target_crandom(driver: &mut dyn Q3Driver) -> Result<f32, Q3GameError> {
    let value = driver.game_crandom();
    if !value.is_finite() || value < -1.0 || value > 1.0 {
        return Err(range("Game crandom() must return a value within [-1, 1]"));
    }
    Ok(value)
}

fn source_float_schedule(time: i32, seconds: f32) -> i32 {
    let milliseconds = seconds * 1000.0;
    qvm_float_to_int(time as f32 + milliseconds)
}

fn target_sound(driver: &mut dyn Q3Driver, slot: usize, sound_index: i32) {
    let origin = driver
        .pool()
        .entity(slot)
        .map(|entity| entity.r.current_origin)
        .unwrap_or_else(|| vec3(0.0, 0.0, 0.0));
    let sound = driver.pool().temp_entity(origin, Q3EntityEvent::GeneralSound);
    if let Some(sound) = driver.pool().entity_mut(sound) {
        sound.s.event_parm = sound_index;
    }
}

fn move_direction_for_target(driver: &mut dyn Q3Driver, slot: usize) -> Vec3 {
    let angles = driver
        .pool()
        .entity(slot)
        .map(|entity| entity.s.angles)
        .unwrap_or_else(|| vec3(0.0, 0.0, 0.0));
    let (direction, zero) = move_direction(angles);
    if let Some(entity) = driver.pool().entity_mut(slot) {
        entity.s.angles = zero;
    }
    direction
}

/// `target_give` use handler.
pub fn use_target_give(
    driver: &mut dyn Q3Driver,
    slot: usize,
    _other: Option<&Participant>,
    activator: Option<&Participant>,
) -> Result<(), Q3GameError> {
    let activator = require_use_participant(activator.cloned())?;
    let player = use_client(driver, &activator)?;
    let Some(player) = player else { return Ok(()) };
    let has_client = driver
        .pool()
        .entity(player)
        .is_some_and(|entity| entity.client.is_some());
    let target = driver.pool().entity(slot).and_then(|entity| entity.target.clone());
    if !has_client {
        return Ok(());
    }
    let Some(target) = target else { return Ok(()) };
    let mut current = None;
    loop {
        current = find_entity(driver.pool(), current, EntityStringField::Targetname, Some(&target));
        let Some(found) = current else { break };
        let (has_item, target_actor) = match driver.pool().entity(found) {
            Some(entity) => (entity.item.is_some(), entity.actor.clone()),
            None => continue,
        };
        if !has_item {
            continue;
        }
        let player_actor = use_actor(driver.pool(), &Participant::Entity(player))?;
        driver.touch_item(
            found,
            player,
            &TouchContact {
                self_actor: target_actor,
                other: player_actor,
                plane: None,
                surface: None,
            },
        );
        driver.pool().set_nextthink(found, 0);
        driver.world().unlink(found);
    }
    Ok(())
}

/// `target_remove_powerups` use handler.
pub fn use_target_remove_powerups(
    driver: &mut dyn Q3Driver,
    _slot: usize,
    _other: Option<&Participant>,
    activator: Option<&Participant>,
) -> Result<(), Q3GameError> {
    let activator = require_use_participant(activator.cloned())?;
    let player = use_client(driver, &activator)?;
    let Some(player) = player else { return Ok(()) };
    let Some(client) = driver.pool().entity(player).and_then(|entity| entity.client) else {
        return Ok(());
    };
    let (red, blue, neutral, length) = {
        let client = driver
            .pool()
            .client(client)
            .ok_or_else(|| failure("Target entity does not belong to its entity pool or was replaced"))?;
        (
            client.ps.powerups.get(Q3Powerup::Redflag as usize),
            client.ps.powerups.get(Q3Powerup::Blueflag as usize),
            client.ps.powerups.get(Q3Powerup::Neutralflag as usize),
            client.ps.powerups.len(),
        )
    };
    if red != 0 {
        driver.return_flag(Q3Team::Red);
    } else if blue != 0 {
        driver.return_flag(Q3Team::Blue);
    } else if neutral != 0 {
        driver.return_flag(Q3Team::Free);
    }
    if let Some(client) = driver.pool().client_mut(client) {
        for index in 0..length {
            client.ps.powerups.set(index, 0);
        }
    }
    Ok(())
}

/// `target_delay` think handler.
pub fn think_target_delay(driver: &mut dyn Q3Driver, slot: usize) -> Result<(), Q3GameError> {
    let activation = driver.pool().entity(slot).and_then(|entity| entity.activation.clone());
    driver.use_targets(slot, activation);
    Ok(())
}

/// `target_delay` use handler.
pub fn use_target_delay(
    driver: &mut dyn Q3Driver,
    locations: &Rc<RefCell<TargetLocationState>>,
    slot: usize,
    _other: Option<&Participant>,
    activator: Option<&Participant>,
) -> Result<(), Q3GameError> {
    bind_target_save_callbacks(driver, locations)?;
    let (wait, random) = {
        let entity = driver
            .pool()
            .entity(slot)
            .ok_or_else(|| failure("Target entity does not belong to its entity pool or was replaced"))?;
        (entity.wait, entity.random)
    };
    let variance = random * target_crandom(driver)?;
    let seconds = wait + variance;
    let nextthink = source_float_schedule(driver.combat().time(), seconds);
    let think = driver
        .pool()
        .callbacks()
        .think
        .resolve(Some("q3.base.game.targets.useTargetDelay.think"))?;
    if let Some(entity) = driver.pool().entity_mut(slot) {
        entity.think = think;
        entity.activation = activator.cloned();
    }
    driver.pool().set_nextthink(slot, nextthink);
    Ok(())
}

/// `target_score` use handler.
pub fn use_target_score(
    driver: &mut dyn Q3Driver,
    slot: usize,
    _other: Option<&Participant>,
    activator: Option<&Participant>,
) -> Result<(), Q3GameError> {
    let activator = require_use_participant(activator.cloned())?;
    let player = use_client(driver, &activator)?;
    if let Some(player) = player {
        let (origin, count) = {
            let entity = driver
                .pool()
                .entity(slot)
                .ok_or_else(|| failure("Target entity does not belong to its entity pool or was replaced"))?;
            (entity.r.current_origin, entity.count)
        };
        driver.add_score(player, origin, count);
    }
    Ok(())
}

/// `target_print` use handler.
pub fn use_target_print(
    driver: &mut dyn Q3Driver,
    slot: usize,
    _other: Option<&Participant>,
    activator: Option<&Participant>,
) -> Result<(), Q3GameError> {
    let activator = require_use_participant(activator.cloned())?;
    let player = use_client(driver, &activator)?;
    let (message, spawnflags) = {
        let entity = driver
            .pool()
            .entity(slot)
            .ok_or_else(|| failure("Target entity does not belong to its entity pool or was replaced"))?;
        (entity.message.clone(), entity.spawnflags)
    };
    let command = game_format("cp \"%s\"", &[GameFormatArg::Text(message)])?;
    if player.is_some() && spawnflags & 4 != 0 {
        driver.send_server_command(player.unwrap_or(0) as i32, &command);
        return Ok(());
    }
    if spawnflags & 3 != 0 {
        if spawnflags & 1 != 0 {
            team_command(driver, Q3Team::Red, &command);
        }
        if spawnflags & 2 != 0 {
            team_command(driver, Q3Team::Blue, &command);
        }
        return Ok(());
    }
    driver.send_server_command(SERVER_COMMAND_BROADCAST, &command);
    Ok(())
}

/// `target_speaker` use handler.
pub fn use_target_speaker(
    driver: &mut dyn Q3Driver,
    slot: usize,
    _other: Option<&Participant>,
    activator: Option<&Participant>,
) -> Result<(), Q3GameError> {
    let (spawnflags, loop_sound, noise_index) = {
        let entity = driver
            .pool()
            .entity(slot)
            .ok_or_else(|| failure("Target entity does not belong to its entity pool or was replaced"))?;
        (entity.spawnflags, entity.s.loop_sound, entity.noise_index)
    };
    if spawnflags & 3 != 0 {
        if let Some(entity) = driver.pool().entity_mut(slot) {
            entity.s.loop_sound = if loop_sound != 0 { 0 } else { noise_index };
        }
        return Ok(());
    }
    if spawnflags & 8 != 0 {
        let participant = require_use_participant(activator.cloned())?;
        match &participant {
            Participant::Entity(native) => {
                driver
                    .pool()
                    .add_event(*native, Q3EntityEvent::GeneralSound, noise_index);
            }
            Participant::SharedActor(actor) => {
                let actor = actor.clone();
                driver.actor_event(&actor, Q3EntityEvent::GeneralSound, noise_index);
            }
        }
    } else if spawnflags & 4 != 0 {
        driver.pool().add_event(slot, Q3EntityEvent::GlobalSound, noise_index);
    } else {
        driver.pool().add_event(slot, Q3EntityEvent::GeneralSound, noise_index);
    }
    Ok(())
}

/// `target_push` use handler.
pub fn use_target_push(
    driver: &mut dyn Q3Driver,
    slot: usize,
    _other: Option<&Participant>,
    activator: Option<&Participant>,
) -> Result<(), Q3GameError> {
    let activator = require_use_participant(activator.cloned())?;
    let player = use_client(driver, &activator)?;
    let Some(player) = player else { return Ok(()) };
    let Some(client) = driver.pool().entity(player).and_then(|entity| entity.client) else {
        return Ok(());
    };
    let (pm_type, flight) = {
        let client = driver
            .pool()
            .client(client)
            .ok_or_else(|| failure("Target entity does not belong to its entity pool or was replaced"))?;
        (client.ps.pm_type, client.ps.powerups.get(Q3Powerup::Flight as usize))
    };
    if pm_type != Q3MoveType::Normal as i32 || flight != 0 {
        return Ok(());
    }
    let origin2 = driver
        .pool()
        .entity(slot)
        .map(|entity| entity.s.origin2)
        .unwrap_or_else(|| vec3(0.0, 0.0, 0.0));
    if let Some(client) = driver.pool().client_mut(client) {
        client.ps.velocity = origin2;
    }
    let time = driver.combat().time();
    let debounce = driver
        .pool()
        .entity(player)
        .map(|entity| entity.fly_sound_debounce_time)
        .unwrap_or(0);
    if debounce < time {
        if let Some(entity) = driver.pool().entity_mut(player) {
            entity.fly_sound_debounce_time = time.wrapping_add(1500);
        }
        let noise = driver.pool().entity(slot).map(|entity| entity.noise_index).unwrap_or(0);
        target_sound(driver, player, noise);
    }
    Ok(())
}

fn laser_think(driver: &mut dyn Q3Driver, slot: usize) -> Result<(), Q3GameError> {
    let enemy = driver.pool().entity(slot).and_then(|entity| entity.enemy);
    if let Some(enemy) = enemy {
        let (origin, mins, maxs) = {
            let entity = driver
                .pool()
                .entity(enemy)
                .ok_or_else(|| failure("Target entity does not belong to its entity pool or was replaced"))?;
            (entity.s.origin, entity.r.mins, entity.r.maxs)
        };
        let self_origin = driver
            .pool()
            .entity(slot)
            .map(|entity| entity.s.origin)
            .unwrap_or_else(|| vec3(0.0, 0.0, 0.0));
        let point = add3(add3(origin, scale3(mins, 0.5)), scale3(maxs, 0.5));
        if let Some(entity) = driver.pool().entity_mut(slot) {
            entity.movedir = normalize3(sub3(point, self_origin));
        }
    }
    let (origin, movedir, actor, damage, activation) = {
        let entity = driver
            .pool()
            .entity(slot)
            .ok_or_else(|| failure("Target entity does not belong to its entity pool or was replaced"))?;
        (
            entity.s.origin,
            entity.movedir,
            entity.actor.clone(),
            entity.damage,
            entity.activation.clone(),
        )
    };
    let end = add3(origin, scale3(movedir, 2048.0));
    let trace = driver.spatial().trace_actor(&Q3TraceQuery {
        start: origin,
        end,
        shape: Q3TraceShape::Point,
        pass_actor: Some(actor),
        mask: MASK_TARGET_LASER,
    });
    if let Q3TraceHit::Actor(hit) = &trace.hit {
        let target = driver.participant(hit);
        if !matches!(target, Participant::Entity(0)) {
            let host = Participant::Entity(slot);
            let mut direction = movedir;
            driver.combat().damage(
                &target,
                Some(&host),
                activation.as_ref(),
                Some(&mut direction),
                Some(trace.end),
                damage,
                DamageFlags::NO_KNOCKBACK,
                MOD_TARGET_LASER,
            );
            if let Some(entity) = driver.pool().entity_mut(slot) {
                entity.movedir = direction;
            }
        }
    }
    if let Some(entity) = driver.pool().entity_mut(slot) {
        entity.s.origin2 = trace.end;
    }
    driver.world().link(slot);
    let nextthink = driver.combat().time().wrapping_add(100);
    driver.pool().set_nextthink(slot, nextthink);
    Ok(())
}

fn laser_on(driver: &mut dyn Q3Driver, slot: usize) -> Result<(), Q3GameError> {
    let missing = driver
        .pool()
        .entity(slot)
        .is_some_and(|entity| entity.activation.is_none());
    if missing {
        if let Some(entity) = driver.pool().entity_mut(slot) {
            entity.activation = Some(Participant::Entity(slot));
        }
    }
    laser_think(driver, slot)
}

fn laser_off(driver: &mut dyn Q3Driver, slot: usize) {
    driver.world().unlink(slot);
    driver.pool().set_nextthink(slot, 0);
}

/// `target_laser` use handler.
pub fn use_target_laser(
    driver: &mut dyn Q3Driver,
    slot: usize,
    _other: Option<&Participant>,
    activator: Option<&Participant>,
) -> Result<(), Q3GameError> {
    if let Some(entity) = driver.pool().entity_mut(slot) {
        entity.activation = activator.cloned();
    }
    let nextthink = driver.pool().entity(slot).map(|entity| entity.nextthink).unwrap_or(0);
    if nextthink > 0 {
        laser_off(driver, slot);
        Ok(())
    } else {
        laser_on(driver, slot)
    }
}

fn start_target_laser(
    driver: &mut dyn Q3Driver,
    locations: &Rc<RefCell<TargetLocationState>>,
    slot: usize,
) -> Result<(), Q3GameError> {
    bind_target_save_callbacks(driver, locations)?;
    if let Some(entity) = driver.pool().entity_mut(slot) {
        entity.s.e_type = Q3EntityType::Beam as i32;
    }
    let target = driver.pool().entity(slot).and_then(|entity| entity.target.clone());
    if let Some(target) = target {
        let found = find_entity(driver.pool(), None, EntityStringField::Targetname, Some(&target));
        if found.is_none() {
            let (classname, origin) = {
                let entity = driver
                    .pool()
                    .entity(slot)
                    .ok_or_else(|| failure("Target entity does not belong to its entity pool or was replaced"))?;
                (entity.classname_value().map(str::to_string), entity.s.origin)
            };
            let at = driver.scratch().vtos(origin)?.read_string();
            driver.warn(&game_format(
                "%s at %s: %s is a bad target\n",
                &[
                    GameFormatArg::Text(classname),
                    GameFormatArg::Text(Some(at)),
                    GameFormatArg::Text(Some(target)),
                ],
            )?);
        }
        if let Some(entity) = driver.pool().entity_mut(slot) {
            entity.enemy = found;
        }
    } else {
        move_direction_for_target(driver, slot);
    }
    let use_callback = driver
        .pool()
        .callbacks()
        .use_callbacks
        .resolve(Some("q3.base.game.targets.startTargetLaser.use"))?;
    let think = driver
        .pool()
        .callbacks()
        .think
        .resolve(Some("q3.base.game.targets.startTargetLaser.think"))?;
    if let Some(entity) = driver.pool().entity_mut(slot) {
        entity.use_callback = use_callback;
        entity.think = think;
        if entity.damage == 0 {
            entity.damage = 1;
        }
    }
    let spawnflags = driver.pool().entity(slot).map(|entity| entity.spawnflags).unwrap_or(0);
    if spawnflags & 1 != 0 {
        laser_on(driver, slot)
    } else {
        laser_off(driver, slot);
        Ok(())
    }
}

/// `target_teleporter` use handler.
pub fn use_target_teleporter(
    driver: &mut dyn Q3Driver,
    slot: usize,
    _other: Option<&Participant>,
    activator: Option<&Participant>,
) -> Result<(), Q3GameError> {
    let activator = require_use_participant(activator.cloned())?;
    let player = use_client(driver, &activator)?;
    let Some(player) = player else { return Ok(()) };
    if driver
        .pool()
        .entity(player)
        .is_some_and(|entity| entity.client.is_none())
    {
        return Ok(());
    }
    let target = driver.pool().entity(slot).and_then(|entity| entity.target.clone());
    let destination = pick_target(driver, target.as_deref())?;
    let Some(destination) = destination else {
        driver.warn("Couldn't find teleporter destination\n");
        return Ok(());
    };
    let (origin, angles) = {
        let entity = driver
            .pool()
            .entity(destination)
            .ok_or_else(|| failure("Target entity does not belong to its entity pool or was replaced"))?;
        (entity.s.origin, entity.s.angles)
    };
    driver.teleport_player(player, origin, angles);
    Ok(())
}

/// `target_kill` use handler.
pub fn use_target_kill(
    driver: &mut dyn Q3Driver,
    _slot: usize,
    _other: Option<&Participant>,
    activator: Option<&Participant>,
) -> Result<(), Q3GameError> {
    let activator = require_use_participant(activator.cloned())?;
    driver.combat().damage(
        &activator,
        None,
        None,
        None,
        None,
        100_000,
        DamageFlags::NO_PROTECTION,
        MOD_TELEFRAG,
    );
    Ok(())
}

/// `target_relay` use handler.
pub fn use_target_relay(
    driver: &mut dyn Q3Driver,
    slot: usize,
    _other: Option<&Participant>,
    activator: Option<&Participant>,
) -> Result<(), Q3GameError> {
    let spawnflags = driver.pool().entity(slot).map(|entity| entity.spawnflags).unwrap_or(0);
    if spawnflags & 3 != 0 && activator.is_none() {
        return Err(failure("Team-filtered target_relay requires an activator"));
    }
    let player = match activator {
        None => None,
        Some(activator) => {
            let required = require_use_participant(Some(activator.clone()))?;
            use_client(driver, &required)?
        }
    };
    let team = match player {
        Some(player) => driver
            .pool()
            .entity(player)
            .and_then(|entity| entity.client)
            .and_then(|client| driver.pool().client(client).map(|client| client.sess.session_team)),
        None => None,
    };
    if spawnflags & 1 != 0 && player.is_some() && team.is_some() && team != Some(Q3Team::Red as i32) {
        return Ok(());
    }
    if spawnflags & 2 != 0 && player.is_some() && team.is_some() && team != Some(Q3Team::Blue as i32) {
        return Ok(());
    }
    if spawnflags & 4 != 0 {
        let target = driver.pool().entity(slot).and_then(|entity| entity.target.clone());
        let selected = pick_target(driver, target.as_deref())?;
        if let Some(selected) = selected {
            let use_callback = driver
                .pool()
                .entity(selected)
                .and_then(|entity| entity.use_callback.clone());
            if let Some(use_callback) = use_callback {
                let other = Participant::Entity(slot);
                use_callback(driver, selected, Some(&other), activator);
            }
        }
        return Ok(());
    }
    driver.use_targets(slot, activator.cloned());
    Ok(())
}

/// Link target locations (`linkTargetLocations`).
pub fn link_target_locations(
    driver: &mut dyn Q3Driver,
    locations: &Rc<RefCell<TargetLocationState>>,
) -> Result<(), Q3GameError> {
    if locations.borrow().linked {
        return Ok(());
    }
    locations.borrow_mut().linked = true;
    locations.borrow_mut().head = None;
    driver.set_configstring(CS_LOCATIONS, "unknown");
    let mut number = 1i32;
    let count = driver.pool().num_entities();
    for index in 0..count {
        let (classname, message) = match driver.pool().entity(index) {
            Some(entity) => (entity.classname_value().map(str::to_string), entity.message.clone()),
            None => {
                return Err(failure(
                    "Target entity does not belong to its entity pool or was replaced",
                ))
            }
        };
        let Some(classname) = classname else { continue };
        if ascii_fold(&classname) != b"target_location" {
            continue;
        }
        if let Some(entity) = driver.pool().entity_mut(index) {
            entity.health = number;
        }
        driver.set_configstring(CS_LOCATIONS + number, &message.unwrap_or_default());
        number += 1;
        let head = locations.borrow().head;
        if let Some(entity) = driver.pool().entity_mut(index) {
            entity.next_train = head;
        }
        locations.borrow_mut().head = Some(index);
    }
    Ok(())
}

fn speaker_sound_path(noise: &str) -> Result<String, Q3GameError> {
    if noise.contains(".wav") {
        game_format_sized("%s", &[GameFormatArg::Text(Some(noise.to_string()))], 64)
    } else {
        game_format_sized("%s.wav", &[GameFormatArg::Text(Some(noise.to_string()))], 64)
    }
}

/// `target_give` spawn handler.
pub fn spawn_target_give(
    driver: &mut dyn Q3Driver,
    locations: &Rc<RefCell<TargetLocationState>>,
    slot: usize,
) -> Result<(), Q3GameError> {
    bind_target_save_callbacks(driver, locations)?;
    target_owned(driver, slot)?;
    let use_callback = driver
        .pool()
        .callbacks()
        .use_callbacks
        .resolve(Some("q3.base.game.targets.spawnTargetGive.use"))?;
    if let Some(entity) = driver.pool().entity_mut(slot) {
        entity.use_callback = use_callback;
    }
    Ok(())
}

/// `target_remove_powerups` spawn handler.
pub fn spawn_target_remove_powerups(
    driver: &mut dyn Q3Driver,
    locations: &Rc<RefCell<TargetLocationState>>,
    slot: usize,
) -> Result<(), Q3GameError> {
    bind_target_save_callbacks(driver, locations)?;
    target_owned(driver, slot)?;
    let use_callback = driver
        .pool()
        .callbacks()
        .use_callbacks
        .resolve(Some("q3.base.game.targets.spawnTargetRemovePowerups.use"))?;
    if let Some(entity) = driver.pool().entity_mut(slot) {
        entity.use_callback = use_callback;
    }
    Ok(())
}

/// `target_delay` spawn handler.
pub fn spawn_target_delay(
    driver: &mut dyn Q3Driver,
    locations: &Rc<RefCell<TargetLocationState>>,
    slot: usize,
    variables: &SpawnVariables,
) -> Result<(), Q3GameError> {
    bind_target_save_callbacks(driver, locations)?;
    target_owned(driver, slot)?;
    let delay = variables.float("delay", "0")?;
    let wait = if delay.present {
        delay.value
    } else {
        variables.float("wait", "1")?.value
    };
    let use_callback = driver
        .pool()
        .callbacks()
        .use_callbacks
        .resolve(Some("q3.base.game.targets.spawnTargetDelay.use"))?;
    if let Some(entity) = driver.pool().entity_mut(slot) {
        entity.wait = if wait == 0.0 { 1.0 } else { wait };
        entity.use_callback = use_callback;
    }
    Ok(())
}

/// `target_score` spawn handler.
pub fn spawn_target_score(
    driver: &mut dyn Q3Driver,
    locations: &Rc<RefCell<TargetLocationState>>,
    slot: usize,
) -> Result<(), Q3GameError> {
    bind_target_save_callbacks(driver, locations)?;
    target_owned(driver, slot)?;
    let use_callback = driver
        .pool()
        .callbacks()
        .use_callbacks
        .resolve(Some("q3.base.game.targets.spawnTargetScore.use"))?;
    if let Some(entity) = driver.pool().entity_mut(slot) {
        if entity.count == 0 {
            entity.count = 1;
        }
        entity.use_callback = use_callback;
    }
    Ok(())
}

/// `target_print` spawn handler.
pub fn spawn_target_print(
    driver: &mut dyn Q3Driver,
    locations: &Rc<RefCell<TargetLocationState>>,
    slot: usize,
) -> Result<(), Q3GameError> {
    bind_target_save_callbacks(driver, locations)?;
    target_owned(driver, slot)?;
    let use_callback = driver
        .pool()
        .callbacks()
        .use_callbacks
        .resolve(Some("q3.base.game.targets.spawnTargetPrint.use"))?;
    if let Some(entity) = driver.pool().entity_mut(slot) {
        entity.use_callback = use_callback;
    }
    Ok(())
}

/// `target_speaker` spawn handler.
pub fn spawn_target_speaker(
    driver: &mut dyn Q3Driver,
    locations: &Rc<RefCell<TargetLocationState>>,
    slot: usize,
    variables: &SpawnVariables,
) -> Result<(), Q3GameError> {
    bind_target_save_callbacks(driver, locations)?;
    target_owned(driver, slot)?;
    let noise = variables.string("noise", "NOSOUND");
    if !noise.present {
        let origin = driver
            .pool()
            .entity(slot)
            .map(|entity| entity.s.origin)
            .unwrap_or_else(|| vec3(0.0, 0.0, 0.0));
        let at = driver.scratch().vtos(origin)?.read_string();
        let message = game_format(
            "target_speaker without a noise key at %s",
            &[GameFormatArg::Text(Some(at))],
        )?;
        return Err(failure(message));
    }
    if noise.value.starts_with('*') {
        if let Some(entity) = driver.pool().entity_mut(slot) {
            entity.spawnflags |= 8;
        }
    }
    let index = driver.sound_index(&speaker_sound_path(&noise.value)?);
    let (wait, random, spawnflags) = {
        let entity = driver
            .pool()
            .entity(slot)
            .ok_or_else(|| failure("Target entity does not belong to its entity pool or was replaced"))?;
        (entity.wait, entity.random, entity.spawnflags)
    };
    let use_callback = driver
        .pool()
        .callbacks()
        .use_callbacks
        .resolve(Some("q3.base.game.targets.spawnTargetSpeaker.use"))?;
    {
        let entity = driver
            .pool()
            .entity_mut(slot)
            .ok_or_else(|| failure("Target entity does not belong to its entity pool or was replaced"))?;
        entity.noise_index = index;
        entity.s.e_type = Q3EntityType::Speaker as i32;
        entity.s.event_parm = index;
        entity.s.frame = qvm_float_to_int(wait * 10.0);
        entity.s.client_num = qvm_float_to_int(random * 10.0);
        if spawnflags & 1 != 0 {
            entity.s.loop_sound = index;
        }
        entity.use_callback = use_callback;
        if spawnflags & 4 != 0 {
            entity.r.sv_flags |= ServerEntityFlags::BROADCAST;
        }
        entity.s.pos.base = entity.s.origin;
    }
    driver.world().link(slot);
    Ok(())
}

/// `target_push` spawn handler.
pub fn spawn_target_push(
    driver: &mut dyn Q3Driver,
    locations: &Rc<RefCell<TargetLocationState>>,
    slot: usize,
) -> Result<(), Q3GameError> {
    bind_target_save_callbacks(driver, locations)?;
    target_owned(driver, slot)?;
    if driver.pool().entity(slot).is_some_and(|entity| entity.speed == 0.0) {
        if let Some(entity) = driver.pool().entity_mut(slot) {
            entity.speed = 1000.0;
        }
    }
    let moved = move_direction_for_target(driver, slot);
    let (speed, spawnflags, target, origin) = {
        let entity = driver
            .pool()
            .entity(slot)
            .ok_or_else(|| failure("Target entity does not belong to its entity pool or was replaced"))?;
        (entity.speed, entity.spawnflags, entity.target.clone(), entity.s.origin)
    };
    let noise = driver.sound_index(if spawnflags & 1 != 0 {
        "sound/world/jumppad.wav"
    } else {
        "sound/misc/windfly.wav"
    });
    let think = driver
        .pool()
        .callbacks()
        .think
        .resolve(Some("q3.base.game.targets.spawnTargetPush.think"))?;
    let use_callback = driver
        .pool()
        .callbacks()
        .use_callbacks
        .resolve(Some("q3.base.game.targets.spawnTargetPush.use"))?;
    if let Some(entity) = driver.pool().entity_mut(slot) {
        entity.s.origin2 = scale3(moved, speed);
        entity.noise_index = noise;
        entity.use_callback = use_callback;
        if target.is_some() {
            entity.r.absmin_override = Some(origin);
            entity.r.absmax_override = Some(origin);
            entity.think = think;
        }
    }
    if target.is_some() {
        let nextthink = driver.combat().time().wrapping_add(100);
        driver.pool().set_nextthink(slot, nextthink);
    }
    Ok(())
}

/// `target_push` aim think handler.
pub fn think_target_push_aim(driver: &mut dyn Q3Driver, slot: usize) -> Result<(), Q3GameError> {
    let origin = {
        let entity = driver
            .pool()
            .entity(slot)
            .ok_or_else(|| failure("Target entity does not belong to its entity pool or was replaced"))?;
        scale3(add3(entity.r.absmin(), entity.r.absmax()), 0.5)
    };
    aim_at_target(driver, slot, origin)
}

/// `target_laser` spawn handler.
pub fn spawn_target_laser(
    driver: &mut dyn Q3Driver,
    locations: &Rc<RefCell<TargetLocationState>>,
    slot: usize,
) -> Result<(), Q3GameError> {
    bind_target_save_callbacks(driver, locations)?;
    target_owned(driver, slot)?;
    let think = driver
        .pool()
        .callbacks()
        .think
        .resolve(Some("q3.base.game.targets.spawnTargetLaser.think"))?;
    if let Some(entity) = driver.pool().entity_mut(slot) {
        entity.think = think;
    }
    let nextthink = driver.combat().time().wrapping_add(100);
    driver.pool().set_nextthink(slot, nextthink);
    Ok(())
}

/// `target_teleporter` spawn handler.
pub fn spawn_target_teleporter(
    driver: &mut dyn Q3Driver,
    locations: &Rc<RefCell<TargetLocationState>>,
    slot: usize,
) -> Result<(), Q3GameError> {
    bind_target_save_callbacks(driver, locations)?;
    target_owned(driver, slot)?;
    let (targetname, classname, origin) = {
        let entity = driver
            .pool()
            .entity(slot)
            .ok_or_else(|| failure("Target entity does not belong to its entity pool or was replaced"))?;
        (
            entity.targetname.clone(),
            entity.classname_value().map(str::to_string),
            entity.s.origin,
        )
    };
    if targetname.is_none() {
        let at = driver.scratch().vtos(origin)?.read_string();
        driver.warn(&game_format(
            "untargeted %s at %s\n",
            &[GameFormatArg::Text(classname), GameFormatArg::Text(Some(at))],
        )?);
    }
    let use_callback = driver
        .pool()
        .callbacks()
        .use_callbacks
        .resolve(Some("q3.base.game.targets.spawnTargetTeleporter.use"))?;
    if let Some(entity) = driver.pool().entity_mut(slot) {
        entity.use_callback = use_callback;
    }
    Ok(())
}

/// `target_kill` spawn handler.
pub fn spawn_target_kill(
    driver: &mut dyn Q3Driver,
    locations: &Rc<RefCell<TargetLocationState>>,
    slot: usize,
) -> Result<(), Q3GameError> {
    bind_target_save_callbacks(driver, locations)?;
    target_owned(driver, slot)?;
    let use_callback = driver
        .pool()
        .callbacks()
        .use_callbacks
        .resolve(Some("q3.base.game.targets.spawnTargetKill.use"))?;
    if let Some(entity) = driver.pool().entity_mut(slot) {
        entity.use_callback = use_callback;
    }
    Ok(())
}

/// `target_location` spawn handler.
pub fn spawn_target_location(
    driver: &mut dyn Q3Driver,
    locations: &Rc<RefCell<TargetLocationState>>,
    slot: usize,
) -> Result<(), Q3GameError> {
    bind_target_save_callbacks(driver, locations)?;
    target_owned(driver, slot)?;
    let think = driver
        .pool()
        .callbacks()
        .think
        .resolve(Some("q3.base.game.targets.spawnTargetLocation.think"))?;
    if let Some(entity) = driver.pool().entity_mut(slot) {
        entity.think = think;
        set_origin(entity, entity.s.origin);
    }
    let nextthink = driver.combat().time().wrapping_add(200);
    driver.pool().set_nextthink(slot, nextthink);
    Ok(())
}

/// `target_relay` spawn handler.
pub fn spawn_target_relay(
    driver: &mut dyn Q3Driver,
    locations: &Rc<RefCell<TargetLocationState>>,
    slot: usize,
) -> Result<(), Q3GameError> {
    bind_target_save_callbacks(driver, locations)?;
    target_owned(driver, slot)?;
    let use_callback = driver
        .pool()
        .callbacks()
        .use_callbacks
        .resolve(Some("q3.base.game.targets.spawnTargetRelay.use"))?;
    if let Some(entity) = driver.pool().entity_mut(slot) {
        entity.use_callback = use_callback;
    }
    Ok(())
}

/// `target_position` spawn handler.
pub fn spawn_target_position(driver: &mut dyn Q3Driver, slot: usize) -> Result<(), Q3GameError> {
    target_owned(driver, slot)?;
    if let Some(entity) = driver.pool().entity_mut(slot) {
        set_origin(entity, entity.s.origin);
    }
    Ok(())
}

/// Spawn table entries (`targetSpawnHandlers`).
pub fn target_spawn_handlers(locations: &Rc<RefCell<TargetLocationState>>) -> SpawnHandlerTable {
    let mut table = SpawnHandlerTable::new();
    let give = Rc::clone(locations);
    table.insert(
        "target_give",
        Rc::new(move |driver, _services, slot, _variables| {
            spawn_target_give(driver, &give, slot)?;
            Ok(())
        }),
    );
    let remove = Rc::clone(locations);
    table.insert(
        "target_remove_powerups",
        Rc::new(move |driver, _services, slot, _variables| {
            spawn_target_remove_powerups(driver, &remove, slot)?;
            Ok(())
        }),
    );
    let delay = Rc::clone(locations);
    table.insert(
        "target_delay",
        Rc::new(move |driver, _services, slot, variables| {
            spawn_target_delay(driver, &delay, slot, variables)?;
            Ok(())
        }),
    );
    let score = Rc::clone(locations);
    table.insert(
        "target_score",
        Rc::new(move |driver, _services, slot, _variables| {
            spawn_target_score(driver, &score, slot)?;
            Ok(())
        }),
    );
    let print = Rc::clone(locations);
    table.insert(
        "target_print",
        Rc::new(move |driver, _services, slot, _variables| {
            spawn_target_print(driver, &print, slot)?;
            Ok(())
        }),
    );
    let speaker = Rc::clone(locations);
    table.insert(
        "target_speaker",
        Rc::new(move |driver, _services, slot, variables| {
            spawn_target_speaker(driver, &speaker, slot, variables)?;
            Ok(())
        }),
    );
    let laser = Rc::clone(locations);
    table.insert(
        "target_laser",
        Rc::new(move |driver, _services, slot, _variables| {
            spawn_target_laser(driver, &laser, slot)?;
            Ok(())
        }),
    );
    let teleporter = Rc::clone(locations);
    table.insert(
        "target_teleporter",
        Rc::new(move |driver, _services, slot, _variables| {
            spawn_target_teleporter(driver, &teleporter, slot)?;
            Ok(())
        }),
    );
    let relay = Rc::clone(locations);
    table.insert(
        "target_relay",
        Rc::new(move |driver, _services, slot, _variables| {
            spawn_target_relay(driver, &relay, slot)?;
            Ok(())
        }),
    );
    table.insert(
        "target_position",
        Rc::new(|driver, _services, slot, _variables| {
            spawn_target_position(driver, slot)?;
            Ok(())
        }),
    );
    let push = Rc::clone(locations);
    table.insert(
        "target_push",
        Rc::new(move |driver, _services, slot, _variables| {
            spawn_target_push(driver, &push, slot)?;
            Ok(())
        }),
    );
    let kill = Rc::clone(locations);
    table.insert(
        "target_kill",
        Rc::new(move |driver, _services, slot, _variables| {
            spawn_target_kill(driver, &kill, slot)?;
            Ok(())
        }),
    );
    let location = Rc::clone(locations);
    table.insert(
        "target_location",
        Rc::new(move |driver, _services, slot, _variables| {
            spawn_target_location(driver, &location, slot)?;
            Ok(())
        }),
    );
    table
}

/// Bind target save callbacks (`bindTargetSaveCallbacks`).
pub fn bind_target_save_callbacks(
    driver: &mut dyn Q3Driver,
    locations: &Rc<RefCell<TargetLocationState>>,
) -> Result<(), Q3GameError> {
    driver.pool().callbacks_mut().use_callbacks.intern(
        "q3.base.game.targets.spawnTargetGive.use",
        Rc::new(|driver, slot, other, activator| {
            or_panic(use_target_give(driver, slot, other, activator));
        }),
    )?;
    driver.pool().callbacks_mut().use_callbacks.intern(
        "q3.base.game.targets.spawnTargetRemovePowerups.use",
        Rc::new(|driver, slot, other, activator| {
            or_panic(use_target_remove_powerups(driver, slot, other, activator));
        }),
    )?;
    driver.pool().callbacks_mut().think.intern(
        "q3.base.game.targets.useTargetDelay.think",
        Rc::new(|driver, slot| {
            or_panic(think_target_delay(driver, slot));
        }),
    )?;
    let delay_locations = Rc::clone(locations);
    driver.pool().callbacks_mut().use_callbacks.intern(
        "q3.base.game.targets.spawnTargetDelay.use",
        Rc::new(move |driver, slot, other, activator| {
            or_panic(use_target_delay(driver, &delay_locations, slot, other, activator));
        }),
    )?;
    driver.pool().callbacks_mut().use_callbacks.intern(
        "q3.base.game.targets.spawnTargetScore.use",
        Rc::new(|driver, slot, other, activator| {
            or_panic(use_target_score(driver, slot, other, activator));
        }),
    )?;
    driver.pool().callbacks_mut().use_callbacks.intern(
        "q3.base.game.targets.spawnTargetPrint.use",
        Rc::new(|driver, slot, other, activator| {
            or_panic(use_target_print(driver, slot, other, activator));
        }),
    )?;
    driver.pool().callbacks_mut().use_callbacks.intern(
        "q3.base.game.targets.spawnTargetSpeaker.use",
        Rc::new(|driver, slot, other, activator| {
            or_panic(use_target_speaker(driver, slot, other, activator));
        }),
    )?;
    driver.pool().callbacks_mut().think.intern(
        "q3.base.game.targets.spawnTargetPush.think",
        Rc::new(|driver, slot| {
            or_panic(think_target_push_aim(driver, slot));
        }),
    )?;
    driver.pool().callbacks_mut().use_callbacks.intern(
        "q3.base.game.targets.spawnTargetPush.use",
        Rc::new(|driver, slot, other, activator| {
            or_panic(use_target_push(driver, slot, other, activator));
        }),
    )?;
    driver.pool().callbacks_mut().use_callbacks.intern(
        "q3.base.game.targets.startTargetLaser.use",
        Rc::new(|driver, slot, other, activator| {
            or_panic(use_target_laser(driver, slot, other, activator));
        }),
    )?;
    driver.pool().callbacks_mut().think.intern(
        "q3.base.game.targets.startTargetLaser.think",
        Rc::new(|driver, slot| {
            or_panic(laser_think(driver, slot));
        }),
    )?;
    let start_locations = Rc::clone(locations);
    driver.pool().callbacks_mut().think.intern(
        "q3.base.game.targets.spawnTargetLaser.think",
        Rc::new(move |driver, slot| {
            or_panic(start_target_laser(driver, &start_locations, slot));
        }),
    )?;
    driver.pool().callbacks_mut().use_callbacks.intern(
        "q3.base.game.targets.spawnTargetTeleporter.use",
        Rc::new(|driver, slot, other, activator| {
            or_panic(use_target_teleporter(driver, slot, other, activator));
        }),
    )?;
    driver.pool().callbacks_mut().use_callbacks.intern(
        "q3.base.game.targets.spawnTargetKill.use",
        Rc::new(|driver, slot, other, activator| {
            or_panic(use_target_kill(driver, slot, other, activator));
        }),
    )?;
    let link_locations = Rc::clone(locations);
    driver.pool().callbacks_mut().think.intern(
        "q3.base.game.targets.spawnTargetLocation.think",
        Rc::new(move |driver, _slot| {
            or_panic(link_target_locations(driver, &link_locations));
        }),
    )?;
    driver.pool().callbacks_mut().use_callbacks.intern(
        "q3.base.game.targets.spawnTargetRelay.use",
        Rc::new(|driver, slot, other, activator| {
            or_panic(use_target_relay(driver, slot, other, activator));
        }),
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// triggers.ts: trigger entities (g_trigger.c, BG_TouchJumpPad)
// ---------------------------------------------------------------------------

/// Frame time (`FRAMETIME`).
const FRAMETIME: i32 = 100;
/// Trigger contents (`CONTENTS_TRIGGER`; renamed to avoid the portal constant).
const TRIGGER_CONTENTS: i32 = 0x40000000;
/// Trigger-hurt means of death (`MOD_TRIGGER_HURT`).
const MOD_TRIGGER_HURT: i32 = 22;

fn trigger_owned(driver: &mut dyn Q3Driver, slot: usize) -> Result<(), Q3GameError> {
    if driver.pool().entity(slot).is_none() {
        return Err(failure(
            "Trigger entity does not belong to its entity pool or was replaced",
        ));
    }
    Ok(())
}

fn source_float_to_int(value: f32) -> i32 {
    if (-2_147_483_648.0..2_147_483_648.0).contains(&value) {
        value.trunc() as i32
    } else {
        -2_147_483_648
    }
}

fn source_schedule(time: i32, wait: f32, random: f32, crandom: f32) -> i32 {
    let seconds = wait + random * crandom;
    let milliseconds = 1000.0 * seconds;
    source_float_to_int(time as f32 + milliseconds)
}

fn checked_crandom(driver: &mut dyn Q3Driver) -> Result<f32, Q3GameError> {
    let value = driver.game_crandom();
    if !value.is_finite() || value < -1.0 || value > 1.0 {
        return Err(range("Game crandom() must return a value within [-1, 1]"));
    }
    Ok(value)
}

/// Shared AimAtTarget math (`aimAtTarget`).
pub fn aim_at_target(driver: &mut dyn Q3Driver, slot: usize, origin: Vec3) -> Result<(), Q3GameError> {
    let target = driver.pool().entity(slot).and_then(|entity| entity.target.clone());
    let found = pick_target(driver, target.as_deref())?;
    let Some(found) = found else {
        driver.pool().free_entity(slot);
        return Ok(());
    };
    let target_origin = driver
        .pool()
        .entity(found)
        .map(|entity| entity.s.origin)
        .unwrap_or_else(|| vec3(0.0, 0.0, 0.0));
    let height = target_origin.z - origin.z;
    let gravity = driver.gravity();
    let time = (height / (0.5 * gravity)).sqrt();
    if time == 0.0 {
        driver.pool().free_entity(slot);
        return Ok(());
    }
    let offset = sub3(target_origin, origin);
    let horizontal = vec3(offset.x, offset.y, 0.0);
    let distance = dot3(horizontal, horizontal).sqrt();
    let direction = if distance == 0.0 {
        horizontal
    } else {
        scale3(horizontal, 1.0 / distance)
    };
    let forward = distance / time;
    let velocity = scale3(direction, forward);
    if let Some(entity) = driver.pool().entity_mut(slot) {
        entity.s.origin2 = vec3(velocity.x, velocity.y, time * gravity);
    }
    Ok(())
}

fn init_trigger(driver: &mut dyn Q3Driver, slot: usize) -> Result<(), Q3GameError> {
    let angles = driver
        .pool()
        .entity(slot)
        .map(|entity| entity.s.angles)
        .unwrap_or_else(|| vec3(0.0, 0.0, 0.0));
    if angles.x != 0.0 || angles.y != 0.0 || angles.z != 0.0 {
        let (direction, zero) = move_direction(angles);
        if let Some(entity) = driver.pool().entity_mut(slot) {
            entity.movedir = direction;
            entity.s.angles = zero;
        }
    }
    let model = driver.pool().entity(slot).and_then(|entity| entity.model.clone());
    driver.set_brush_model(slot, model.as_deref());
    if let Some(entity) = driver.pool().entity_mut(slot) {
        entity.r.contents = TRIGGER_CONTENTS;
        entity.r.sv_flags = ServerEntityFlags::NOCLIENT;
    }
    Ok(())
}

fn multi_wait(driver: &mut dyn Q3Driver, slot: usize) {
    driver.pool().set_nextthink(slot, 0);
}

fn multi_trigger(driver: &mut dyn Q3Driver, slot: usize, activator: Option<&Participant>) -> Result<(), Q3GameError> {
    bind_trigger_save_callbacks(driver)?;
    if let Some(activator) = activator {
        require_use_participant(Some(activator.clone()))?;
    }
    if let Some(entity) = driver.pool().entity_mut(slot) {
        entity.activation = activator.cloned();
    }
    let nextthink = driver.pool().entity(slot).map(|entity| entity.nextthink).unwrap_or(0);
    if nextthink != 0 {
        return Ok(());
    }
    let Some(activator) = activator else {
        return Err(failure("trigger_multiple requires an activator"));
    };
    let player = use_client(driver, activator)?;
    let team = match player {
        Some(player) => driver
            .pool()
            .entity(player)
            .and_then(|entity| entity.client)
            .and_then(|client| driver.pool().client(client).map(|client| client.sess.session_team)),
        None => None,
    };
    let (spawnflags, wait, random) = {
        let entity = driver
            .pool()
            .entity(slot)
            .ok_or_else(|| failure("Trigger entity does not belong to its entity pool or was replaced"))?;
        (entity.spawnflags, entity.wait, entity.random)
    };
    if team.is_some() {
        if spawnflags & 1 != 0 && team != Some(Q3Team::Red as i32) {
            return Ok(());
        }
        if spawnflags & 2 != 0 && team != Some(Q3Team::Blue as i32) {
            return Ok(());
        }
    }
    driver.use_targets(slot, Some(activator.clone()));
    if wait > 0.0 {
        let think = driver
            .pool()
            .callbacks()
            .think
            .resolve(Some("q3.base.game.triggers.multiTrigger.think"))?;
        if let Some(entity) = driver.pool().entity_mut(slot) {
            entity.think = think;
        }
        let nextthink = source_schedule(driver.combat().time(), wait, random, checked_crandom(driver)?);
        driver.pool().set_nextthink(slot, nextthink);
    } else {
        if let Some(entity) = driver.pool().entity_mut(slot) {
            entity.touch = None;
        }
        let think = driver
            .pool()
            .callbacks()
            .think
            .resolve(Some("q3.base.game.triggers.multiTrigger.free"))?;
        if let Some(entity) = driver.pool().entity_mut(slot) {
            entity.think = think;
        }
        let nextthink = driver.combat().time().wrapping_add(FRAMETIME);
        driver.pool().set_nextthink(slot, nextthink);
    }
    Ok(())
}

/// `trigger_multiple` spawn handler.
pub fn spawn_trigger_multiple(
    driver: &mut dyn Q3Driver,
    slot: usize,
    variables: &SpawnVariables,
) -> Result<(), Q3GameError> {
    bind_trigger_save_callbacks(driver)?;
    trigger_owned(driver, slot)?;
    let wait = variables.float("wait", "0.5")?.value;
    let mut random = variables.float("random", "0")?.value;
    if random >= wait && wait >= 0.0 {
        random = wait - FRAMETIME as f32;
        driver.warn("trigger_multiple has random >= wait\n");
    }
    let touch = driver
        .pool()
        .callbacks()
        .touch
        .resolve(Some("q3.base.game.triggers.spawnTriggerMultiple.touch"))?;
    let use_callback = driver
        .pool()
        .callbacks()
        .use_callbacks
        .resolve(Some("q3.base.game.triggers.spawnTriggerMultiple.use"))?;
    if let Some(entity) = driver.pool().entity_mut(slot) {
        entity.wait = wait;
        entity.random = random;
        entity.touch = touch;
        entity.use_callback = use_callback;
    }
    init_trigger(driver, slot)?;
    driver.world().link(slot);
    Ok(())
}

/// `trigger_always` spawn handler.
pub fn spawn_trigger_always(driver: &mut dyn Q3Driver, slot: usize) -> Result<(), Q3GameError> {
    bind_trigger_save_callbacks(driver)?;
    trigger_owned(driver, slot)?;
    let think = driver
        .pool()
        .callbacks()
        .think
        .resolve(Some("q3.base.game.triggers.spawnTriggerAlways.think"))?;
    if let Some(entity) = driver.pool().entity_mut(slot) {
        entity.think = think;
    }
    let nextthink = driver.combat().time().wrapping_add(300);
    driver.pool().set_nextthink(slot, nextthink);
    Ok(())
}

/// `trigger_push` spawn handler.
pub fn spawn_trigger_push(driver: &mut dyn Q3Driver, slot: usize) -> Result<(), Q3GameError> {
    bind_trigger_save_callbacks(driver)?;
    trigger_owned(driver, slot)?;
    init_trigger(driver, slot)?;
    driver.sound_index("sound/world/jumppad.wav");
    let touch = driver
        .pool()
        .callbacks()
        .touch
        .resolve(Some("q3.base.game.triggers.spawnTriggerPush.touch"))?;
    let think = driver
        .pool()
        .callbacks()
        .think
        .resolve(Some("q3.base.game.triggers.spawnTriggerPush.think"))?;
    if let Some(entity) = driver.pool().entity_mut(slot) {
        entity.r.sv_flags &= !ServerEntityFlags::NOCLIENT;
        entity.s.e_type = Q3EntityType::PushTrigger as i32;
        entity.touch = touch;
        entity.think = think;
    }
    let nextthink = driver.combat().time().wrapping_add(FRAMETIME);
    driver.pool().set_nextthink(slot, nextthink);
    driver.world().link(slot);
    Ok(())
}

/// `trigger_teleport` spawn handler.
pub fn spawn_trigger_teleport(driver: &mut dyn Q3Driver, slot: usize) -> Result<(), Q3GameError> {
    bind_trigger_save_callbacks(driver)?;
    trigger_owned(driver, slot)?;
    init_trigger(driver, slot)?;
    driver.sound_index("sound/world/jumppad.wav");
    let touch = driver
        .pool()
        .callbacks()
        .touch
        .resolve(Some("q3.base.game.triggers.spawnTriggerTeleport.touch"))?;
    if let Some(entity) = driver.pool().entity_mut(slot) {
        if entity.spawnflags & 1 != 0 {
            entity.r.sv_flags |= ServerEntityFlags::NOCLIENT;
        } else {
            entity.r.sv_flags &= !ServerEntityFlags::NOCLIENT;
        }
        entity.s.e_type = Q3EntityType::TeleportTrigger as i32;
        entity.touch = touch;
    }
    driver.world().link(slot);
    Ok(())
}

fn sound_at(driver: &mut dyn Q3Driver, participant: &Participant, sound: i32) {
    let origin = match participant {
        Participant::Entity(slot) => driver.pool().entity(*slot).map(|entity| entity.r.current_origin),
        Participant::SharedActor(actor) => driver.actor_origin(actor),
    };
    let Some(origin) = origin else { return };
    let event = driver.pool().temp_entity(origin, Q3EntityEvent::GeneralSound);
    if let Some(event) = driver.pool().entity_mut(event) {
        event.s.event_parm = sound;
    }
}

/// `trigger_hurt` spawn handler.
pub fn spawn_trigger_hurt(driver: &mut dyn Q3Driver, slot: usize) -> Result<(), Q3GameError> {
    bind_trigger_save_callbacks(driver)?;
    trigger_owned(driver, slot)?;
    init_trigger(driver, slot)?;
    let noise = driver.sound_index("sound/world/electro.wav");
    let touch = driver
        .pool()
        .callbacks()
        .touch
        .resolve(Some("q3.base.game.triggers.spawnTriggerHurt.touch"))?;
    let use_callback = if driver
        .pool()
        .entity(slot)
        .is_some_and(|entity| entity.spawnflags & 2 != 0)
    {
        driver
            .pool()
            .callbacks()
            .use_callbacks
            .resolve(Some("q3.base.game.triggers.spawnTriggerHurt.use"))?
    } else {
        None
    };
    if let Some(entity) = driver.pool().entity_mut(slot) {
        entity.noise_index = noise;
        entity.touch = touch;
        if entity.damage == 0 {
            entity.damage = 5;
        }
        entity.r.contents = TRIGGER_CONTENTS;
        if use_callback.is_some() {
            entity.use_callback = use_callback;
        }
    }
    if driver
        .pool()
        .entity(slot)
        .is_some_and(|entity| entity.spawnflags & 1 == 0)
    {
        driver.world().link(slot);
    }
    Ok(())
}

fn timer_think(driver: &mut dyn Q3Driver, slot: usize) -> Result<(), Q3GameError> {
    let (activation, wait, random) = {
        let entity = driver
            .pool()
            .entity(slot)
            .ok_or_else(|| failure("Trigger entity does not belong to its entity pool or was replaced"))?;
        (entity.activation.clone(), entity.wait, entity.random)
    };
    driver.use_targets(slot, activation);
    let nextthink = source_schedule(driver.combat().time(), wait, random, checked_crandom(driver)?);
    driver.pool().set_nextthink(slot, nextthink);
    Ok(())
}

/// `func_timer` spawn handler.
pub fn spawn_func_timer(driver: &mut dyn Q3Driver, slot: usize, variables: &SpawnVariables) -> Result<(), Q3GameError> {
    bind_trigger_save_callbacks(driver)?;
    trigger_owned(driver, slot)?;
    let mut random = variables.float("random", "1")?.value;
    let wait = variables.float("wait", "1")?.value;
    let use_callback = driver
        .pool()
        .callbacks()
        .use_callbacks
        .resolve(Some("q3.base.game.triggers.spawnFuncTimer.use"))?;
    let think = driver
        .pool()
        .callbacks()
        .think
        .resolve(Some("q3.base.game.triggers.spawnFuncTimer.think"))?;
    if let Some(entity) = driver.pool().entity_mut(slot) {
        entity.random = random;
        entity.wait = wait;
        entity.use_callback = use_callback;
        entity.think = think;
    }
    if random >= wait {
        random = wait - FRAMETIME as f32;
        if let Some(entity) = driver.pool().entity_mut(slot) {
            entity.random = random;
        }
        let origin = driver
            .pool()
            .entity(slot)
            .map(|entity| entity.s.origin)
            .unwrap_or_else(|| vec3(0.0, 0.0, 0.0));
        let at = driver.scratch().vtos(origin)?.read_string();
        driver.warn(&game_format(
            "func_timer at %s has random >= wait\n",
            &[GameFormatArg::Text(Some(at))],
        )?);
    }
    if driver
        .pool()
        .entity(slot)
        .is_some_and(|entity| entity.spawnflags & 1 != 0)
    {
        let nextthink = driver.combat().time().wrapping_add(FRAMETIME);
        driver.pool().set_nextthink(slot, nextthink);
        if let Some(entity) = driver.pool().entity_mut(slot) {
            entity.activation = Some(Participant::Entity(slot));
        }
    }
    if let Some(entity) = driver.pool().entity_mut(slot) {
        entity.r.sv_flags = ServerEntityFlags::NOCLIENT;
    }
    Ok(())
}

fn touch_jump_pad_trigger(driver: &mut dyn Q3Driver, slot: usize, other: usize) -> Result<(), Q3GameError> {
    let Some(client) = driver.pool().entity(other).and_then(|entity| entity.client) else {
        return Ok(());
    };
    let pad = driver
        .pool()
        .entity(slot)
        .map(|entity| entity.s.clone())
        .unwrap_or_default();
    if let Some(client) = driver.pool().client_mut(client) {
        touch_jump_pad(&mut client.ps, &pad);
    }
    Ok(())
}

fn touch_teleport(driver: &mut dyn Q3Driver, slot: usize, other: usize) -> Result<(), Q3GameError> {
    let Some(client) = driver.pool().entity(other).and_then(|entity| entity.client) else {
        return Ok(());
    };
    let (pm_type, team) = {
        let client = driver
            .pool()
            .client(client)
            .ok_or_else(|| failure("Trigger entity does not belong to its entity pool or was replaced"))?;
        (client.ps.pm_type, client.sess.session_team)
    };
    let (spawnflags, target) = {
        let entity = driver
            .pool()
            .entity(slot)
            .ok_or_else(|| failure("Trigger entity does not belong to its entity pool or was replaced"))?;
        (entity.spawnflags, entity.target.clone())
    };
    if pm_type == Q3MoveType::Dead as i32 {
        return Ok(());
    }
    if spawnflags & 1 != 0 && team != Q3Team::Spectator as i32 {
        return Ok(());
    }
    let destination = pick_target(driver, target.as_deref())?;
    let Some(destination) = destination else {
        driver.warn("Couldn't find teleporter destination\n");
        return Ok(());
    };
    let (origin, angles) = {
        let entity = driver
            .pool()
            .entity(destination)
            .ok_or_else(|| failure("Trigger entity does not belong to its entity pool or was replaced"))?;
        (entity.s.origin, entity.s.angles)
    };
    driver.teleport_player(other, origin, angles);
    Ok(())
}

fn touch_hurt(driver: &mut dyn Q3Driver, slot: usize, other: &Participant) -> Result<(), Q3GameError> {
    let damageable = match other {
        Participant::Entity(other) => driver.pool().entity(*other).is_some_and(|entity| entity.takedamage),
        Participant::SharedActor(actor) => driver
            .combat()
            .actor_combat_state(actor)
            .is_some_and(|state| state.can_take_damage),
    };
    let time = driver.combat().time();
    let timestamp = driver.pool().entity(slot).map(|entity| entity.timestamp).unwrap_or(0);
    if !damageable || timestamp > time {
        return Ok(());
    }
    let (spawnflags, noise_index, damage) = {
        let entity = driver
            .pool()
            .entity(slot)
            .ok_or_else(|| failure("Trigger entity does not belong to its entity pool or was replaced"))?;
        (entity.spawnflags, entity.noise_index, entity.damage)
    };
    if let Some(entity) = driver.pool().entity_mut(slot) {
        entity.timestamp = time.wrapping_add(if spawnflags & 16 != 0 { 1000 } else { FRAMETIME });
    }
    if spawnflags & 4 == 0 {
        sound_at(driver, other, noise_index);
    }
    let flags = if spawnflags & 8 != 0 {
        DamageFlags::NO_PROTECTION
    } else {
        0
    };
    let host = Participant::Entity(slot);
    driver.combat().damage(
        other,
        Some(&host),
        Some(&host),
        None,
        None,
        damage,
        flags,
        MOD_TRIGGER_HURT,
    );
    Ok(())
}

/// Spawn table entries (`triggerSpawnHandlers`).
#[must_use]
pub fn trigger_spawn_handlers() -> SpawnHandlerTable {
    let mut table = SpawnHandlerTable::new();
    table.insert(
        "trigger_multiple",
        Rc::new(|driver, _services, slot, variables| {
            spawn_trigger_multiple(driver, slot, variables)?;
            Ok(())
        }),
    );
    table.insert(
        "trigger_always",
        Rc::new(|driver, _services, slot, _variables| {
            spawn_trigger_always(driver, slot)?;
            Ok(())
        }),
    );
    table.insert(
        "trigger_push",
        Rc::new(|driver, _services, slot, _variables| {
            spawn_trigger_push(driver, slot)?;
            Ok(())
        }),
    );
    table.insert(
        "trigger_teleport",
        Rc::new(|driver, _services, slot, _variables| {
            spawn_trigger_teleport(driver, slot)?;
            Ok(())
        }),
    );
    table.insert(
        "trigger_hurt",
        Rc::new(|driver, _services, slot, _variables| {
            spawn_trigger_hurt(driver, slot)?;
            Ok(())
        }),
    );
    table.insert(
        "func_timer",
        Rc::new(|driver, _services, slot, variables| {
            spawn_func_timer(driver, slot, variables)?;
            Ok(())
        }),
    );
    table
}

/// Bind trigger save callbacks (`bindTriggerSaveCallbacks`).
pub fn bind_trigger_save_callbacks(driver: &mut dyn Q3Driver) -> Result<(), Q3GameError> {
    driver.pool().callbacks_mut().think.intern(
        "q3.base.game.triggers.multiTrigger.think",
        Rc::new(|driver, slot| {
            multi_wait(driver, slot);
        }),
    )?;
    driver.pool().callbacks_mut().think.intern(
        "q3.base.game.triggers.multiTrigger.free",
        Rc::new(|driver, slot| {
            driver.pool().free_entity(slot);
        }),
    )?;
    driver.pool().callbacks_mut().touch.intern(
        "q3.base.game.triggers.spawnTriggerMultiple.touch",
        Rc::new(|driver, slot, other, _contact| {
            if let Participant::Entity(other) = other {
                let has_client = driver
                    .pool()
                    .entity(*other)
                    .is_some_and(|entity| entity.client.is_some());
                if has_client {
                    or_panic(multi_trigger(driver, slot, Some(&Participant::Entity(*other))));
                }
            }
        }),
    )?;
    driver.pool().callbacks_mut().use_callbacks.intern(
        "q3.base.game.triggers.spawnTriggerMultiple.use",
        Rc::new(|driver, slot, _other, activator| {
            or_panic(multi_trigger(driver, slot, activator));
        }),
    )?;
    driver.pool().callbacks_mut().think.intern(
        "q3.base.game.triggers.spawnTriggerAlways.think",
        Rc::new(|driver, slot| {
            let activator = Participant::Entity(slot);
            driver.use_targets(slot, Some(activator));
            driver.pool().free_entity(slot);
        }),
    )?;
    driver.pool().callbacks_mut().touch.intern(
        "q3.base.game.triggers.spawnTriggerPush.touch",
        Rc::new(|driver, slot, other, _contact| {
            if let Participant::Entity(other) = other {
                let has_client = driver
                    .pool()
                    .entity(*other)
                    .is_some_and(|entity| entity.client.is_some());
                if has_client {
                    or_panic(touch_jump_pad_trigger(driver, slot, *other));
                }
            }
        }),
    )?;
    driver.pool().callbacks_mut().think.intern(
        "q3.base.game.triggers.spawnTriggerPush.think",
        Rc::new(|driver, slot| {
            let origin = driver
                .pool()
                .entity(slot)
                .map(|entity| scale3(add3(entity.r.absmin(), entity.r.absmax()), 0.5))
                .unwrap_or_else(|| vec3(0.0, 0.0, 0.0));
            or_panic(aim_at_target(driver, slot, origin));
        }),
    )?;
    driver.pool().callbacks_mut().touch.intern(
        "q3.base.game.triggers.spawnTriggerTeleport.touch",
        Rc::new(|driver, slot, other, _contact| {
            if let Participant::Entity(other) = other {
                or_panic(touch_teleport(driver, slot, *other));
            }
        }),
    )?;
    driver.pool().callbacks_mut().touch.intern(
        "q3.base.game.triggers.spawnTriggerHurt.touch",
        Rc::new(|driver, slot, other, _contact| {
            or_panic(touch_hurt(driver, slot, other));
        }),
    )?;
    driver.pool().callbacks_mut().use_callbacks.intern(
        "q3.base.game.triggers.spawnTriggerHurt.use",
        Rc::new(|driver, slot, _other, _activator| {
            let linked = driver.pool().entity(slot).is_some_and(|entity| entity.r.linked);
            if linked {
                driver.world().unlink(slot);
            } else {
                driver.world().link(slot);
            }
        }),
    )?;
    driver.pool().callbacks_mut().use_callbacks.intern(
        "q3.base.game.triggers.spawnFuncTimer.use",
        Rc::new(|driver, slot, _other, activator| {
            or_panic(use_func_timer(driver, slot, activator));
        }),
    )?;
    driver.pool().callbacks_mut().think.intern(
        "q3.base.game.triggers.spawnFuncTimer.think",
        Rc::new(|driver, slot| {
            or_panic(timer_think(driver, slot));
        }),
    )?;
    Ok(())
}

fn use_func_timer(driver: &mut dyn Q3Driver, slot: usize, activator: Option<&Participant>) -> Result<(), Q3GameError> {
    if let Some(activator) = activator {
        require_use_participant(Some(activator.clone()))?;
    }
    if let Some(entity) = driver.pool().entity_mut(slot) {
        entity.activation = activator.cloned();
    }
    let nextthink = driver.pool().entity(slot).map(|entity| entity.nextthink).unwrap_or(0);
    if nextthink != 0 {
        driver.pool().set_nextthink(slot, 0);
        Ok(())
    } else {
        timer_think(driver, slot)
    }
}

// ---------------------------------------------------------------------------
// weapon.ts: weapons (g_weapon.c, g_combat.c ray/invulnerability helpers)
// ---------------------------------------------------------------------------

/// Shot trace mask (`MASK_SHOT`).
const MASK_SHOT: i32 = 0x6000001;
/// Award flags (`AWARD_FLAGS`).
const AWARD_FLAGS: i32 = 0x8 | 0x40 | 0x800 | 0x8000 | 0x10000 | 0x20000;
/// Suicide means of death (`MOD_SUICIDE`).
const MOD_SUICIDE: i32 = 26;

/// Sphere intersections (`SphereIntersections`).
#[derive(Debug, Clone, PartialEq)]
pub enum SphereIntersections {
    /// None.
    None,
    /// One.
    One(Vec3),
    /// Two, in source order.
    Two(Vec3, Vec3),
}

/// Invulnerability impact (`InvulnerabilityImpact`).
#[derive(Debug, Clone, PartialEq)]
pub enum InvulnerabilityImpact {
    /// Miss.
    Miss,
    /// Hit.
    Hit {
        /// Impact point.
        impact_point: Vec3,
        /// Bounce direction.
        bounce_direction: Vec3,
    },
}

/// Weapon unlink hook (`WeaponHost.unlink`): returns the actor when a restore is required.
pub type WeaponUnlink = Rc<dyn Fn(&mut dyn Q3Driver, &ActorId) -> Option<ActorId>>;
/// Weapon relink hook (the donor restore closure).
pub type WeaponRelink = Rc<dyn Fn(&mut dyn Q3Driver, &ActorId)>;

fn client_of(pool: &dyn EntityPool, slot: usize) -> Result<usize, Q3GameError> {
    pool.entity(slot)
        .and_then(|entity| entity.client)
        .ok_or_else(|| failure("Weapon attack requires a client entity"))
}

/// Damage factor (`q3WeaponDamageFactor`). The persistant-powerup item tag is
/// resolved by the caller because entity/item links are slot-based here.
#[must_use]
pub fn q3_weapon_damage_factor(
    client: &GameClient,
    quad_factor: f32,
    persistant_powerup_tag: Option<i32>,
    product: Q3Product,
) -> f32 {
    let mut factor = if client.ps.powerups.get(Q3Powerup::Quad as usize) != 0 {
        quad_factor
    } else {
        1.0
    };
    if product == Q3Product::Missionpack && persistant_powerup_tag == Some(Q3Powerup::Doubler as i32) {
        factor *= 2.0;
    }
    factor
}

/// Ray/sphere intersections (`raySphereIntersections`); normalizes `direction` in place.
pub fn ray_sphere_intersections(origin: Vec3, radius: f32, point: Vec3, direction: &mut Vec3) -> SphereIntersections {
    let dir = normalize3(*direction);
    *direction = dir;
    let offset = sub3(point, origin);
    let b = 2.0 * dot3(dir, offset);
    let c = dot3(offset, offset) - radius * radius;
    let discriminant = b * b - 4.0 * c;
    if discriminant > 0.0 {
        let root = discriminant.sqrt();
        let first = (-b + root) / 2.0;
        let second = (-b - root) / 2.0;
        SphereIntersections::Two(add3(point, scale3(dir, first)), add3(point, scale3(dir, second)))
    } else if discriminant == 0.0 {
        SphereIntersections::One(add3(point, scale3(dir, -b / 2.0)))
    } else {
        SphereIntersections::None
    }
}

/// Invulnerability effect (`invulnerabilityEffect`).
pub fn invulnerability_effect(
    driver: &mut dyn Q3Driver,
    target: usize,
    direction: Vec3,
    point: Vec3,
) -> Result<InvulnerabilityImpact, Q3GameError> {
    if driver.pool().product() != Q3Product::Missionpack {
        return Err(failure("Invulnerability effects require missionpack"));
    }
    let Some(client) = driver.pool().entity(target).and_then(|entity| entity.client) else {
        return Ok(InvulnerabilityImpact::Miss);
    };
    let origin = driver
        .pool()
        .client(client)
        .map(|client| client.ps.origin)
        .unwrap_or_else(|| vec3(0.0, 0.0, 0.0));
    let mut backwards = vec3(-direction.x, -direction.y, -direction.z);
    let intersections = ray_sphere_intersections(origin, 42.0, point, &mut backwards);
    let impact_point = match &intersections {
        SphereIntersections::Two(first, _) | SphereIntersections::One(first) => *first,
        SphereIntersections::None => return Ok(InvulnerabilityImpact::Miss),
    };
    let _ = intersections;
    let impact = driver.pool().temp_entity(origin, Q3EntityEvent::InvulImpact);
    let offset = sub3(impact_point, origin);
    let angles = vector_to_angles(offset);
    let mut pitch = angles.x + 90.0;
    if pitch > 360.0 {
        pitch -= 360.0;
    }
    if let Some(impact) = driver.pool().entity_mut(impact) {
        impact.s.angles = vec3(pitch, angles.y, angles.z);
    }
    Ok(InvulnerabilityImpact::Hit {
        impact_point,
        bounce_direction: normalize3(offset),
    })
}

fn accuracy_subject(pool: &dyn EntityPool, slot: usize, actor: &ActorId) -> AccuracySubject {
    let entity = pool.entity(slot);
    let client = entity
        .and_then(|entity| entity.client)
        .and_then(|client| pool.client(client));
    AccuracySubject {
        actor: actor.clone(),
        damageable: entity.is_some_and(|entity| entity.takedamage),
        player: client.is_some(),
        health: client
            .map(|client| client.ps.health())
            .unwrap_or_else(|| entity.map(|entity| entity.health).unwrap_or(0)),
        team: client.map(|client| client.sess.session_team),
    }
}

/// Accuracy-hit test for native entities (`logAccuracyHit`).
pub fn log_accuracy_hit(game_type: i32, driver: &mut dyn Q3Driver, target: usize, attacker: usize) -> bool {
    let target_takedamage = driver.pool().entity(target).is_some_and(|entity| entity.takedamage);
    let target_client = driver.pool().entity(target).and_then(|entity| entity.client);
    let attacker_client = driver.pool().entity(attacker).and_then(|entity| entity.client);
    let target_health = target_client.and_then(|client| driver.pool().client(client).map(|client| client.ps.health()));
    if !target_takedamage || target == attacker || target_client.is_none() || attacker_client.is_none() {
        return false;
    }
    if target_health.is_some_and(|health| health <= 0) {
        return false;
    }
    let target_actor = driver.pool().entity(target).map(|entity| entity.actor.clone());
    let attacker_actor = driver.pool().entity(attacker).map(|entity| entity.actor.clone());
    let (Some(target_actor), Some(attacker_actor)) = (target_actor, attacker_actor) else {
        return false;
    };
    q3_accuracy_hit(
        game_type >= Q3GameType::Team as i32,
        &accuracy_subject(driver.pool(), target, &target_actor),
        &accuracy_subject(driver.pool(), attacker, &attacker_actor),
    )
}

/// Weapon hitscan host (the `bullet`/`contactHost` services).
pub struct WeaponHitscanHost {
    entity: usize,
    attacker: ActorId,
    firing_weapon: i32,
    method: i32,
    unlink: WeaponUnlink,
    relink: WeaponRelink,
}

impl WeaponHitscanHost {
    fn trace(&mut self, driver: &mut dyn Q3Driver, start: Vec3, end: Vec3, pass: Option<&ActorId>) -> Q3TraceResult {
        driver.spatial().trace_actor(&Q3TraceQuery {
            start,
            end,
            shape: Q3TraceShape::Point,
            pass_actor: pass.cloned(),
            mask: MASK_SHOT,
        })
    }

    fn hit_target(&mut self, driver: &mut dyn Q3Driver, actor: &ActorId) -> Option<BulletTarget> {
        let state = driver.combat().actor_combat_state(actor)?;
        let target = driver.participant(actor);
        let native = match &target {
            Participant::Entity(slot) => Some(*slot),
            Participant::SharedActor(_) => None,
        };
        let source_attacker = driver.native_slot(&self.attacker);
        let attacker_state = source_attacker.map_or(
            AccuracySubject {
                actor: self.attacker.clone(),
                damageable: false,
                player: false,
                health: 0,
                team: None,
            },
            |slot| accuracy_subject(driver.pool(), slot, &self.attacker),
        );
        let observed = native.map_or(
            AccuracySubject {
                actor: actor.clone(),
                damageable: state.can_take_damage,
                player: driver.actor_is_player(actor),
                health: state.health,
                team: state.team,
            },
            |slot| accuracy_subject(driver.pool(), slot, actor),
        );
        let team_game = driver.combat().game_type() >= Q3GameType::Team as i32;
        let time = driver.combat().time();
        let invulnerable = native
            .and_then(|slot| driver.pool().entity(slot))
            .and_then(|entity| entity.client)
            .and_then(|client| driver.pool().client(client))
            .is_some_and(|client| client.invulnerability_time > time);
        Some(BulletTarget {
            damageable: observed.damageable,
            player: observed.player,
            accuracy_eligible: q3_accuracy_hit(team_game, &observed, &attacker_state),
            invulnerable,
        })
    }

    fn apply_damage(&mut self, driver: &mut dyn Q3Driver, target: &ActorId, direction: Vec3, point: Vec3, amount: i32) {
        let participant = driver.participant(target);
        let host = Participant::Entity(self.entity);
        let mut direction = direction;
        driver.combat().damage(
            &participant,
            Some(&host),
            Some(&host),
            Some(&mut direction),
            Some(point),
            amount,
            0,
            self.method,
        );
    }

    fn invulnerability_impact(
        &mut self,
        driver: &mut dyn Q3Driver,
        target: &ActorId,
        direction: Vec3,
        point: Vec3,
    ) -> Result<InvulnerabilityImpact, Q3GameError> {
        let participant = driver.participant(target);
        let Participant::Entity(slot) = participant else {
            return Err(failure("Q3 invulnerability requires its actual client behavior"));
        };
        invulnerability_effect(driver, slot, direction, point)
    }
}

impl BulletHost for WeaponHitscanHost {
    fn trace_hit(
        &mut self,
        driver: &mut dyn Q3Driver,
        start: Vec3,
        end: Vec3,
        pass: Option<&ActorId>,
    ) -> Q3TraceResult {
        self.trace(driver, start, end, pass)
    }

    fn hit_target(&mut self, driver: &mut dyn Q3Driver, actor: &ActorId) -> Option<BulletTarget> {
        WeaponHitscanHost::hit_target(self, driver, actor)
    }

    fn emit_hit(&mut self, driver: &mut dyn Q3Driver, hit: &BulletHit) {
        let event = driver.pool().temp_entity(
            hit.point,
            if hit.flesh {
                Q3EntityEvent::BulletHitFlesh
            } else {
                Q3EntityEvent::BulletHitWall
            },
        );
        let flesh_target = if hit.flesh { hit.target.clone() } else { None };
        let (event_parm, other) = match flesh_target {
            Some(target) => {
                let participant = driver.participant(&target);
                match participant {
                    Participant::Entity(slot)
                        if driver.pool().entity(slot).is_some_and(|entity| entity.client.is_some()) =>
                    {
                        let number = driver.pool().entity(slot).map(|entity| entity.s.number).unwrap_or(0);
                        (
                            number,
                            driver
                                .pool()
                                .entity(self.entity)
                                .map(|entity| entity.s.number)
                                .unwrap_or(0),
                        )
                    }
                    _ => panic!("Admitted Q3 map player has no native client behavior record"),
                }
            }
            None => (
                direction_to_byte(Some(hit.normal)) as i32,
                driver
                    .pool()
                    .entity(self.entity)
                    .map(|entity| entity.s.number)
                    .unwrap_or(0),
            ),
        };
        if let Some(event) = driver.pool().entity_mut(event) {
            event.s.event_parm = event_parm;
            event.s.other_entity_num = other;
        }
    }

    fn apply_damage(&mut self, driver: &mut dyn Q3Driver, target: &ActorId, direction: Vec3, point: Vec3, amount: i32) {
        WeaponHitscanHost::apply_damage(self, driver, target, direction, point, amount);
    }

    fn credit_accuracy(&mut self, driver: &mut dyn Q3Driver) {
        if let Ok(client) = client_of(driver.pool(), self.entity) {
            if let Some(client) = driver.pool().client_mut(client) {
                client.accuracy_hits = client.accuracy_hits.wrapping_add(1);
            }
        }
    }

    fn invulnerability_impact(
        &mut self,
        driver: &mut dyn Q3Driver,
        target: &ActorId,
        direction: Vec3,
        point: Vec3,
    ) -> InvulnerabilityImpact {
        WeaponHitscanHost::invulnerability_impact(self, driver, target, direction, point)
            .unwrap_or_else(|error| panic!("{error}"))
    }
}

impl ContactHost for WeaponHitscanHost {
    fn trace_hit(
        &mut self,
        driver: &mut dyn Q3Driver,
        start: Vec3,
        end: Vec3,
        pass: Option<&ActorId>,
    ) -> Q3TraceResult {
        self.trace(driver, start, end, pass)
    }

    fn hit_target(&mut self, driver: &mut dyn Q3Driver, actor: &ActorId) -> Option<BulletTarget> {
        WeaponHitscanHost::hit_target(self, driver, actor)
    }

    fn emit_contact(&mut self, driver: &mut dyn Q3Driver, event: &ContactEvent) {
        match event {
            ContactEvent::GauntletQuad => {
                driver.pool().add_event(self.entity, Q3EntityEvent::PowerupQuad, 0);
            }
            ContactEvent::LightningReflection { start, end } => {
                let event = driver.pool().temp_entity(*start, Q3EntityEvent::Lightningbolt);
                if let Some(event) = driver.pool().entity_mut(event) {
                    event.s.origin2 = *end;
                }
            }
            ContactEvent::Miss { point, normal } => {
                let event = driver.pool().temp_entity(*point, Q3EntityEvent::MissileMiss);
                if let Some(event) = driver.pool().entity_mut(event) {
                    event.s.event_parm = direction_to_byte(Some(*normal)) as i32;
                }
            }
            ContactEvent::Hit { point, normal, target } => {
                let participant = driver.participant(target);
                let slot = match participant {
                    Participant::Entity(slot)
                        if driver.pool().entity(slot).is_some_and(|entity| entity.client.is_some()) =>
                    {
                        slot
                    }
                    _ => panic!("Admitted Q3 map player has no native client behavior record"),
                };
                let number = driver.pool().entity(slot).map(|entity| entity.s.number).unwrap_or(0);
                let event = driver.pool().temp_entity(*point, Q3EntityEvent::MissileHit);
                if let Some(event) = driver.pool().entity_mut(event) {
                    event.s.other_entity_num = number;
                    event.s.event_parm = direction_to_byte(Some(*normal)) as i32;
                    event.s.weapon = self.firing_weapon;
                }
            }
        }
    }

    fn apply_damage(&mut self, driver: &mut dyn Q3Driver, target: &ActorId, direction: Vec3, point: Vec3, amount: i32) {
        WeaponHitscanHost::apply_damage(self, driver, target, direction, point, amount);
    }

    fn credit_accuracy(&mut self, driver: &mut dyn Q3Driver) {
        if driver.combat().actor_combat_state(&self.attacker).is_some() {
            let entity = self.entity;
            if let Ok(client) = client_of(driver.pool(), entity) {
                if let Some(client) = driver.pool().client_mut(client) {
                    client.accuracy_hits = client.accuracy_hits.wrapping_add(1);
                }
            }
        }
    }

    fn invulnerability_impact(
        &mut self,
        driver: &mut dyn Q3Driver,
        target: &ActorId,
        direction: Vec3,
        point: Vec3,
    ) -> InvulnerabilityImpact {
        WeaponHitscanHost::invulnerability_impact(self, driver, target, direction, point)
            .unwrap_or_else(|error| panic!("{error}"))
    }
}

impl ShotgunHost for WeaponHitscanHost {
    fn begin_shotgun(&mut self, driver: &mut dyn Q3Driver, muzzle: Vec3, direction: Vec3) -> usize {
        let event = driver.pool().temp_entity(muzzle, Q3EntityEvent::Shotgun);
        if let Some(event) = driver.pool().entity_mut(event) {
            event.s.origin2 = direction;
        }
        event
    }

    fn emit_shotgun_seed(&mut self, driver: &mut dyn Q3Driver, event_slot: usize, seed: i32) {
        let number = driver
            .pool()
            .entity(self.entity)
            .map(|entity| entity.s.number)
            .unwrap_or(0);
        if let Some(event) = driver.pool().entity_mut(event_slot) {
            event.s.event_parm = seed;
            event.s.other_entity_num = number;
        }
    }
}

impl RailHost for WeaponHitscanHost {
    fn trace_hit(
        &mut self,
        driver: &mut dyn Q3Driver,
        start: Vec3,
        end: Vec3,
        pass: Option<&ActorId>,
    ) -> Q3TraceResult {
        self.trace(driver, start, end, pass)
    }

    fn hit_target(&mut self, driver: &mut dyn Q3Driver, actor: &ActorId) -> Option<BulletTarget> {
        WeaponHitscanHost::hit_target(self, driver, actor)
    }

    fn is_alive(&mut self, driver: &mut dyn Q3Driver) -> bool {
        driver.native_slot(&self.attacker) == Some(self.entity)
    }

    fn unlink_actor(&mut self, driver: &mut dyn Q3Driver, actor: &ActorId) -> Option<ActorId> {
        (self.unlink)(driver, actor)
    }

    fn restore_actor(&mut self, driver: &mut dyn Q3Driver, actor: &ActorId) {
        (self.relink)(driver, actor);
    }

    fn emit_trail(&mut self, driver: &mut dyn Q3Driver, shot: &RailShot) {
        let client_number = driver
            .pool()
            .entity(self.entity)
            .map(|entity| entity.s.client_num)
            .unwrap_or(0);
        let event = driver.pool().temp_entity(shot.end, Q3EntityEvent::Railtrail);
        if let Some(event) = driver.pool().entity_mut(event) {
            event.s.client_num = client_number;
            event.s.origin2 = shot.start;
            event.s.event_parm = shot
                .impact_normal
                .map_or(255, |normal| direction_to_byte(Some(normal)) as i32);
        }
    }

    fn invulnerability_impact(
        &mut self,
        driver: &mut dyn Q3Driver,
        target: &ActorId,
        direction: Vec3,
        point: Vec3,
    ) -> InvulnerabilityImpact {
        WeaponHitscanHost::invulnerability_impact(self, driver, target, direction, point)
            .unwrap_or_else(|error| panic!("{error}"))
    }
}

/// Weapon runtime (`WeaponRuntime`).
pub struct WeaponRuntime {
    /// Quad factor.
    pub quad_factor: f32,
    /// Damage factor override.
    pub damage_factor: Option<Rc<dyn Fn(&mut dyn Q3Driver, usize) -> f32>>,
    launcher: Box<dyn MissileLauncher>,
    unlink: WeaponUnlink,
    relink: WeaponRelink,
}

impl WeaponRuntime {
    /// New runtime.
    pub fn new(
        quad_factor: f32,
        launcher: Box<dyn MissileLauncher>,
        unlink: WeaponUnlink,
        relink: WeaponRelink,
    ) -> Self {
        Self {
            quad_factor,
            damage_factor: None,
            launcher,
            unlink,
            relink,
        }
    }

    fn owned(&self, driver: &mut dyn Q3Driver, slot: usize) -> Result<(), Q3GameError> {
        if driver.pool().entity(slot).is_none() {
            return Err(failure("Weapon entity does not belong to this pool"));
        }
        Ok(())
    }

    fn quad(&self, driver: &mut dyn Q3Driver, slot: usize) -> Result<f32, Q3GameError> {
        if let Some(damage_factor) = &self.damage_factor {
            return Ok(damage_factor(driver, slot));
        }
        let client = client_of(driver.pool(), slot)?;
        let product = driver.combat().product();
        let snapshot = {
            let client_ref = driver
                .pool()
                .client(client)
                .ok_or_else(|| failure("Weapon attack requires a client entity"))?;
            client_ref.clone()
        };
        let item_index = snapshot
            .persistant_powerup
            .and_then(|slot| driver.pool().entity(slot).and_then(|entity| entity.item));
        let tag = item_index.and_then(|index| driver.item_at(index)).map(|item| item.tag);
        Ok(q3_weapon_damage_factor(&snapshot, self.quad_factor, tag, product))
    }

    fn attack(&self, driver: &mut dyn Q3Driver, slot: usize, quad: f32) -> Result<BulletAttack, Q3GameError> {
        let client = client_of(driver.pool(), slot)?;
        let (viewangles, viewheight) = {
            let client_ref = driver
                .pool()
                .client(client)
                .ok_or_else(|| failure("Weapon attack requires a client entity"))?;
            (client_ref.ps.viewangles, client_ref.ps.viewheight)
        };
        let pos_base = {
            let entity = driver
                .pool()
                .entity(slot)
                .ok_or_else(|| failure("Weapon entity does not belong to this pool"))?;
            entity.s.pos.base
        };
        let vectors = angle_vectors(viewangles);
        let eye = vec3(pos_base.x, pos_base.y, pos_base.z + viewheight);
        Ok(BulletAttack {
            forward: vectors.forward,
            right: vectors.right,
            up: vectors.up,
            muzzle: snap_vector(add3(eye, scale3(vectors.forward, 14.0))),
            quad,
        })
    }

    fn scaled(&self, amount: i32, quad: f32) -> i32 {
        qvm_float_to_int(amount as f32 * quad)
    }

    fn contact_host(
        &self,
        driver: &mut dyn Q3Driver,
        slot: usize,
        method: i32,
    ) -> Result<WeaponHitscanHost, Q3GameError> {
        let entity = driver
            .pool()
            .entity(slot)
            .ok_or_else(|| failure("Weapon entity does not belong to this pool"))?;
        Ok(WeaponHitscanHost {
            entity: slot,
            attacker: entity.actor.clone(),
            firing_weapon: entity.s.weapon,
            method,
            unlink: Rc::clone(&self.unlink),
            relink: Rc::clone(&self.relink),
        })
    }

    /// Gauntlet contact check (`checkGauntletAttack`).
    pub fn check_gauntlet_attack(&self, driver: &mut dyn Q3Driver, slot: usize) -> Result<bool, Q3GameError> {
        self.owned(driver, slot)?;
        let mut host = self.contact_host(driver, slot, 2)?;
        let shooter = driver
            .pool()
            .entity(slot)
            .map(|entity| entity.actor.clone())
            .ok_or_else(|| failure("Weapon entity does not belong to this pool"))?;
        let client = client_of(driver.pool(), slot)?;
        let quad = self.quad(driver, slot)?;
        let mut attack = self.attack(driver, slot, quad)?;
        let has_quad = driver
            .pool()
            .client(client)
            .is_some_and(|client| client.ps.powerups.get(Q3Powerup::Quad as usize) != 0);
        Ok(driver.gauntlet_attack(&mut host, &shooter, &mut attack, has_quad))
    }

    fn bullet(
        &self,
        driver: &mut dyn Q3Driver,
        slot: usize,
        attack: &mut BulletAttack,
        spread: i32,
        amount: i32,
    ) -> Result<(), Q3GameError> {
        let mut host = self.contact_host(driver, slot, 3)?;
        let shooter = driver
            .pool()
            .entity(slot)
            .map(|entity| entity.actor.clone())
            .ok_or_else(|| failure("Weapon entity does not belong to this pool"))?;
        driver.bullet_fire(&mut host, &shooter, attack, spread, amount);
        Ok(())
    }

    fn shotgun(&self, driver: &mut dyn Q3Driver, slot: usize, attack: &mut BulletAttack) -> Result<(), Q3GameError> {
        let mut host = self.contact_host(driver, slot, 1)?;
        let shooter = driver
            .pool()
            .entity(slot)
            .map(|entity| entity.actor.clone())
            .ok_or_else(|| failure("Weapon entity does not belong to this pool"))?;
        driver.shotgun_fire(&mut host, &shooter, attack);
        Ok(())
    }

    fn railgun(&self, driver: &mut dyn Q3Driver, slot: usize, attack: &mut BulletAttack) -> Result<(), Q3GameError> {
        let mut host = self.contact_host(driver, slot, 10)?;
        let shooter = driver
            .pool()
            .entity(slot)
            .map(|entity| entity.actor.clone())
            .ok_or_else(|| failure("Weapon entity does not belong to this pool"))?;
        let hits = driver.rail_fire(&mut host, &shooter, attack);
        if driver.native_slot(&shooter) != Some(slot) {
            return Ok(());
        }
        let client = client_of(driver.pool(), slot)?;
        let (accurate_count, accuracy_hits, impressive, reward_until) = {
            let client_ref = driver
                .pool()
                .client(client)
                .ok_or_else(|| failure("Weapon attack requires a client entity"))?;
            (
                client_ref.accurate_count,
                client_ref.accuracy_hits,
                client_ref
                    .ps
                    .persistant
                    .get(Q3PersistentIndex::ImpressiveCount as i32 as usize),
                client_ref.reward_time,
            )
        };
        let time = driver.combat().time();
        let state = driver.rail_statistics(
            &RailStatistics {
                streak: accurate_count,
                hits: accuracy_hits,
                impressive_count: impressive,
                reward_until,
            },
            hits,
            time,
        );
        if let Some(client_ref) = driver.pool().client_mut(client) {
            client_ref.accurate_count = state.streak;
            client_ref.accuracy_hits = state.hits;
            if state.awarded {
                client_ref.ps.persistant.set(
                    Q3PersistentIndex::ImpressiveCount as i32 as usize,
                    state.impressive_count,
                );
                client_ref.ps.e_flags = (client_ref.ps.e_flags & !AWARD_FLAGS) | 0x8000;
                client_ref.reward_time = state.reward_until;
            }
        }
        if state.awarded {
            driver.pool().rankings().reward(slot as i32, 0x8000);
        }
        Ok(())
    }

    fn lightning(&self, driver: &mut dyn Q3Driver, slot: usize, attack: &mut BulletAttack) -> Result<(), Q3GameError> {
        let mut host = self.contact_host(driver, slot, 11)?;
        let shooter = driver
            .pool()
            .entity(slot)
            .map(|entity| entity.actor.clone())
            .ok_or_else(|| failure("Weapon entity does not belong to this pool"))?;
        driver.lightning_fire(&mut host, &shooter, attack);
        Ok(())
    }

    /// Fire the current weapon (`fire`).
    pub fn fire(&mut self, driver: &mut dyn Q3Driver, slot: usize) -> Result<(), Q3GameError> {
        self.owned(driver, slot)?;
        let client = client_of(driver.pool(), slot)?;
        let quad = self.quad(driver, slot)?;
        let weapon = driver.pool().entity(slot).map(|entity| entity.s.weapon).unwrap_or(0);
        driver.pool().rankings().fire_weapon(slot as i32, weapon);
        if weapon != Q3Weapon::GrapplingHook as i32 && weapon != Q3Weapon::Gauntlet as i32 {
            let product = driver.combat().product();
            if let Some(client_ref) = driver.pool().client_mut(client) {
                let shots = if product == Q3Product::Missionpack && weapon == Q3Weapon::Nailgun as i32 {
                    15
                } else {
                    1
                };
                client_ref.accuracy_shots = client_ref.accuracy_shots.wrapping_add(shots);
            }
        }
        let mut attack = self.attack(driver, slot, quad)?;
        if weapon == Q3Weapon::Gauntlet as i32 {
            return Ok(());
        }
        if weapon == Q3Weapon::Lightning as i32 {
            return self.lightning(driver, slot, &mut attack);
        }
        if weapon == Q3Weapon::Shotgun as i32 {
            return self.shotgun(driver, slot, &mut attack);
        }
        if weapon == Q3Weapon::Machinegun as i32 {
            let amount = if driver.combat().game_type() == Q3GameType::Team as i32 {
                5
            } else {
                7
            };
            return self.bullet(driver, slot, &mut attack, 200, amount);
        }
        if weapon == Q3Weapon::GrenadeLauncher as i32 {
            attack.forward = normalize3(vec3(attack.forward.x, attack.forward.y, attack.forward.z + 0.2));
            let projectile = self.launcher.fire_grenade(driver, slot, attack.muzzle, attack.forward);
            let (damage, splash) = driver
                .pool()
                .entity(projectile)
                .map(|entity| (entity.damage, entity.splash_damage))
                .unwrap_or((0, 0));
            if let Some(entity) = driver.pool().entity_mut(projectile) {
                entity.damage = self.scaled(damage, quad);
                entity.splash_damage = self.scaled(splash, quad);
            }
            return Ok(());
        }
        if weapon == Q3Weapon::RocketLauncher as i32 {
            let projectile = self.launcher.fire_rocket(driver, slot, attack.muzzle, attack.forward);
            let (damage, splash) = driver
                .pool()
                .entity(projectile)
                .map(|entity| (entity.damage, entity.splash_damage))
                .unwrap_or((0, 0));
            if let Some(entity) = driver.pool().entity_mut(projectile) {
                entity.damage = self.scaled(damage, quad);
                entity.splash_damage = self.scaled(splash, quad);
            }
            return Ok(());
        }
        if weapon == Q3Weapon::Plasmagun as i32 {
            let projectile = self.launcher.fire_plasma(driver, slot, attack.muzzle, attack.forward);
            let (damage, splash) = driver
                .pool()
                .entity(projectile)
                .map(|entity| (entity.damage, entity.splash_damage))
                .unwrap_or((0, 0));
            if let Some(entity) = driver.pool().entity_mut(projectile) {
                entity.damage = self.scaled(damage, quad);
                entity.splash_damage = self.scaled(splash, quad);
            }
            return Ok(());
        }
        if weapon == Q3Weapon::Railgun as i32 {
            return self.railgun(driver, slot, &mut attack);
        }
        if weapon == Q3Weapon::Bfg as i32 {
            let projectile = self.launcher.fire_bfg(driver, slot, attack.muzzle, attack.forward);
            let (damage, splash) = driver
                .pool()
                .entity(projectile)
                .map(|entity| (entity.damage, entity.splash_damage))
                .unwrap_or((0, 0));
            if let Some(entity) = driver.pool().entity_mut(projectile) {
                entity.damage = self.scaled(damage, quad);
                entity.splash_damage = self.scaled(splash, quad);
            }
            return Ok(());
        }
        if weapon == Q3Weapon::GrapplingHook as i32 {
            let (fire_held, hook) = {
                let client_ref = driver
                    .pool()
                    .client(client)
                    .ok_or_else(|| failure("Weapon attack requires a client entity"))?;
                (client_ref.fire_held, client_ref.hook)
            };
            if !fire_held && hook.is_none() {
                self.launcher.fire_grapple(driver, slot, attack.muzzle, attack.forward);
            }
            if let Some(client_ref) = driver.pool().client_mut(client) {
                client_ref.fire_held = true;
            }
            return Ok(());
        }
        if weapon == Q3Weapon::Nailgun as i32 {
            if driver.combat().product() == Q3Product::Missionpack {
                for _ in 0..15 {
                    let projectile =
                        self.launcher
                            .fire_nail(driver, slot, attack.muzzle, attack.forward, attack.right, attack.up);
                    let (damage, splash) = driver
                        .pool()
                        .entity(projectile)
                        .map(|entity| (entity.damage, entity.splash_damage))
                        .unwrap_or((0, 0));
                    if let Some(entity) = driver.pool().entity_mut(projectile) {
                        entity.damage = self.scaled(damage, quad);
                        entity.splash_damage = self.scaled(splash, quad);
                    }
                }
            }
            return Ok(());
        }
        if weapon == Q3Weapon::ProxLauncher as i32 {
            if driver.combat().product() == Q3Product::Missionpack {
                attack.forward = normalize3(vec3(attack.forward.x, attack.forward.y, attack.forward.z + 0.2));
                let projectile = self.launcher.fire_prox(driver, slot, attack.muzzle, attack.forward);
                let (damage, splash) = driver
                    .pool()
                    .entity(projectile)
                    .map(|entity| (entity.damage, entity.splash_damage))
                    .unwrap_or((0, 0));
                if let Some(entity) = driver.pool().entity_mut(projectile) {
                    entity.damage = self.scaled(damage, quad);
                    entity.splash_damage = self.scaled(splash, quad);
                }
            }
            return Ok(());
        }
        if weapon == Q3Weapon::Chaingun as i32 {
            if driver.combat().product() == Q3Product::Missionpack {
                return self.bullet(driver, slot, &mut attack, 600, 7);
            }
            return Ok(());
        }
        Ok(())
    }

    /// Start the kamikaze timer (`startKamikaze`), returning the timer slot.
    pub fn start_kamikaze(&self, driver: &mut dyn Q3Driver, slot: usize) -> Result<usize, Q3GameError> {
        self.owned(driver, slot)?;
        if driver.combat().product() != Q3Product::Missionpack {
            return Err(failure("Kamikaze requires missionpack"));
        }
        let explosion = driver.pool().spawn_entity()?;
        let time = driver.combat().time();
        if let Some(entity) = driver.pool().entity_mut(explosion) {
            entity.s.e_type = Q3EntityType::Events as i32 + Q3EntityEvent::Kamikaze as i32;
            entity.event_time = time;
        }
        let (client, activator) = {
            let entity = driver
                .pool()
                .entity(slot)
                .ok_or_else(|| failure("Weapon entity does not belong to this pool"))?;
            (entity.client, entity.activator)
        };
        let source_slot = if client.is_some() {
            slot
        } else {
            activator.ok_or_else(|| failure("Kamikaze timer requires its activator"))?
        };
        let source_base = driver
            .pool()
            .entity(source_slot)
            .map(|entity| entity.s.pos.base)
            .unwrap_or_else(|| vec3(0.0, 0.0, 0.0));
        let position = snap_vector(source_base);
        let think = driver
            .pool()
            .callbacks()
            .think
            .resolve(Some("q3.weapon.kamikazeDamage"))?;
        {
            let entity = driver
                .pool()
                .entity_mut(explosion)
                .ok_or_else(|| failure("Weapon entity does not belong to this pool"))?;
            set_origin(entity, position);
            entity.set_classname(Some("kamikaze".to_string()));
            entity.kamikaze_time = time;
            entity.think = think;
            entity.count = 0;
            entity.movedir = vec3(0.0, 0.0, 0.0);
        }
        driver.pool().set_nextthink(explosion, time.wrapping_add(100));
        driver.world().link(explosion);
        if client.is_some() {
            if let Some(entity) = driver.pool().entity_mut(explosion) {
                entity.activator = Some(slot);
            }
            if let Some(entity) = driver.pool().entity_mut(slot) {
                entity.s.e_flags &= !0x200;
            }
            let target = Participant::Entity(slot);
            driver.combat().damage(
                &target,
                Some(&target.clone()),
                Some(&target),
                None,
                None,
                100_000,
                DamageFlags::NO_PROTECTION,
                MOD_SUICIDE,
            );
        } else {
            let (classname, owner) = {
                let entity = driver
                    .pool()
                    .entity(source_slot)
                    .ok_or_else(|| failure("Weapon entity does not belong to this pool"))?;
                (entity.classname_value().map(str::to_string), entity.r.owner_num)
            };
            let activator = if classname.as_deref() == Some("bodyque") {
                if owner < 0 || driver.pool().entity(owner as usize).is_none() {
                    return Err(failure("Weapon entity does not belong to this pool"));
                }
                owner as usize
            } else {
                source_slot
            };
            if let Some(entity) = driver.pool().entity_mut(explosion) {
                entity.activator = Some(activator);
            }
        }
        let event = driver.pool().temp_entity(position, Q3EntityEvent::GlobalTeamSound);
        if let Some(event) = driver.pool().entity_mut(event) {
            event.r.sv_flags |= ServerEntityFlags::BROADCAST;
            event.s.event_parm = 13;
        }
        Ok(explosion)
    }

    fn kamikaze_area(
        &self,
        driver: &mut dyn Q3Driver,
        origin: Vec3,
        attacker: Option<&Participant>,
        amount: i32,
        radius: f32,
        shock: bool,
    ) -> Result<(), Q3GameError> {
        let radius = radius.max(1.0);
        let extent = vec3(radius, radius, radius);
        let candidates = driver.world().area_entities(
            &Bounds {
                min: sub3(origin, extent),
                max: add3(origin, extent),
            },
            1024,
        );
        let time = driver.combat().time();
        for number in candidates {
            let (takedamage, kamikaze_time, kamikaze_shock_time, current_origin) = match driver.pool().entity(number) {
                Some(entity) => (
                    entity.takedamage,
                    entity.kamikaze_time,
                    entity.kamikaze_shock_time,
                    entity.r.current_origin,
                ),
                None => return Err(failure(format!("kamikaze area query returned unknown entity {number}"))),
            };
            if shock {
                if kamikaze_shock_time > time {
                    continue;
                }
            } else if !takedamage || kamikaze_time > time {
                continue;
            }
            let Some(link) = driver.world().link_state(number) else {
                return Err(failure("Kamikaze area query returned an unlinked entity"));
            };
            let axis = |value: f32, min: f32, max: f32| -> f32 {
                if value < min {
                    min - value
                } else if value > max {
                    value - max
                } else {
                    0.0
                }
            };
            let dist = length3(vec3(
                axis(origin.x, link.absbounds.min.x, link.absbounds.max.x),
                axis(origin.y, link.absbounds.min.y, link.absbounds.max.y),
                axis(origin.z, link.absbounds.min.z, link.absbounds.max.z),
            ));
            if dist >= radius {
                continue;
            }
            let offset = sub3(current_origin, origin);
            let mut direction = vec3(offset.x, offset.y, offset.z + 24.0);
            let target = Participant::Entity(number);
            driver.combat().damage(
                &target,
                None,
                attacker,
                Some(&mut direction),
                Some(origin),
                amount,
                DamageFlags::RADIUS | DamageFlags::NO_TEAM_PROTECTION,
                MOD_SUICIDE,
            );
            if shock {
                let horizontal = normalize3(vec3(direction.x, direction.y, 0.0));
                let client = driver.pool().entity(number).and_then(|entity| entity.client);
                if let Some(client) = client {
                    if let Some(client) = driver.pool().client_mut(client) {
                        client.ps.velocity = vec3(horizontal.x * 400.0, horizontal.y * 400.0, 100.0);
                    }
                }
                if let Some(entity) = driver.pool().entity_mut(number) {
                    entity.kamikaze_shock_time = time.wrapping_add(3000);
                }
            } else if let Some(entity) = driver.pool().entity_mut(number) {
                entity.kamikaze_time = time.wrapping_add(3000);
            }
        }
        Ok(())
    }

    fn kamikaze_damage(&self, driver: &mut dyn Q3Driver, slot: usize) -> Result<(), Q3GameError> {
        let time = driver.combat().time();
        let (count, pos_base, activation, movedir) = {
            let entity = driver
                .pool()
                .entity_mut(slot)
                .ok_or_else(|| failure("Weapon entity does not belong to this pool"))?;
            entity.count = entity.count.wrapping_add(100);
            (
                entity.count,
                entity.s.pos.base,
                entity.activation.clone(),
                entity.movedir,
            )
        };
        if count >= 0 {
            #[allow(clippy::cast_possible_truncation)]
            let radius = count.wrapping_mul(1320) / 2000;
            self.kamikaze_area(driver, pos_base, activation.as_ref(), 25, radius as f32, true)?;
        }
        if count >= 250 {
            #[allow(clippy::cast_possible_truncation)]
            let radius = count.wrapping_sub(250).wrapping_mul(720) / 1750;
            self.kamikaze_area(driver, pos_base, activation.as_ref(), 400, radius as f32, false)?;
        }
        if count >= 2000 {
            driver.pool().free_entity(slot);
            return Ok(());
        }
        driver.pool().set_nextthink(slot, time.wrapping_add(100));
        let angles = vec3(driver.game_crandom() * 2.0, driver.game_crandom() * 2.0, 0.0);
        let short = |angle: f32| -> i32 { qvm_float_to_int(angle * 65536.0 / 360.0) & 65535 };
        for index in 0..MAX_CLIENTS {
            let Some(target) = driver.pool().entity(index) else {
                continue;
            };
            let (inuse, client, ground) = (target.inuse, target.client, target.r.ground.clone());
            let Some(client) = client else { continue };
            if !inuse {
                continue;
            }
            if ground.is_some() {
                let (cx, cy, random) = (driver.game_crandom(), driver.game_crandom(), driver.game_random());
                if let Some(client) = driver.pool().client_mut(client) {
                    client.ps.velocity = vec3(
                        client.ps.velocity.x + cx * 120.0,
                        client.ps.velocity.y + cy * 120.0,
                        30.0 + random * 25.0,
                    );
                }
            }
            let delta = sub3(angles, movedir);
            if let Some(client) = driver.pool().client_mut(client) {
                client.ps.delta_angles[0] = client.ps.delta_angles[0].wrapping_add(short(delta.x));
                client.ps.delta_angles[1] = client.ps.delta_angles[1].wrapping_add(short(delta.y));
                client.ps.delta_angles[2] = client.ps.delta_angles[2].wrapping_add(short(delta.z));
            }
        }
        if let Some(entity) = driver.pool().entity_mut(slot) {
            entity.movedir = angles;
        }
        Ok(())
    }

    /// Bind weapon save callbacks (`WeaponRuntime` constructor registration).
    pub fn bind_save_callbacks(
        runtime: &Rc<RefCell<WeaponRuntime>>,
        driver: &mut dyn Q3Driver,
    ) -> Result<(), Q3GameError> {
        let kamikaze = Rc::clone(runtime);
        driver.pool().callbacks_mut().think.register(
            "q3.weapon.kamikazeDamage",
            Rc::new(move |driver, slot| {
                let result = kamikaze.borrow().kamikaze_damage(driver, slot);
                or_panic(result);
            }),
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;

    struct StubPool {
        product: Q3Product,
        entities: Vec<GameEntity>,
        clients: Vec<GameClient>,
        callbacks: Q3CallbackCatalog,
        rankings: Q3RankingReports,
        num_entities: usize,
        events: Vec<(usize, i32, i32)>,
        scheduled: Vec<(usize, i32)>,
        freed: Vec<usize>,
    }

    impl StubPool {
        fn new(owner: &IdentityOwner, product: Q3Product) -> Self {
            let mut entities = Vec::with_capacity(MAX_GENTITIES);
            for slot in 0..MAX_GENTITIES {
                let mut entity = GameEntity::new(slot, owner.actor(slot as u32, 1)).expect("slot");
                entity.s.number = slot as i32;
                entities.push(entity);
            }
            let mut clients = Vec::with_capacity(MAX_CLIENTS);
            for _ in 0..MAX_CLIENTS {
                clients.push(GameClient::new(product));
            }
            Self {
                product,
                entities,
                clients,
                callbacks: Q3CallbackCatalog::new(),
                rankings: Q3RankingReports::new(),
                num_entities: 0,
                events: Vec::new(),
                scheduled: Vec::new(),
                freed: Vec::new(),
            }
        }

        fn use_slot(&mut self, slot: usize) {
            self.entities[slot].inuse = true;
            self.num_entities = self.num_entities.max(slot + 1);
        }
    }

    impl EntityPool for StubPool {
        fn product(&self) -> Q3Product {
            self.product
        }
        fn num_entities(&self) -> usize {
            self.num_entities
        }
        fn max_clients(&self) -> usize {
            MAX_CLIENTS
        }
        fn entity(&self, slot: usize) -> Option<&GameEntity> {
            self.entities.get(slot)
        }
        fn entity_mut(&mut self, slot: usize) -> Option<&mut GameEntity> {
            self.entities.get_mut(slot)
        }
        fn client(&self, slot: usize) -> Option<&GameClient> {
            self.clients.get(slot)
        }
        fn client_mut(&mut self, slot: usize) -> Option<&mut GameClient> {
            self.clients.get_mut(slot)
        }
        fn spawn_entity(&mut self) -> Result<usize, Q3GameError> {
            for slot in 0..MAX_GENTITIES {
                if !self.entities[slot].inuse {
                    self.entities[slot].inuse = true;
                    self.num_entities = self.num_entities.max(slot + 1);
                    return Ok(slot);
                }
            }
            Err(failure("G_Spawn: no free entities"))
        }
        fn free_entity(&mut self, slot: usize) {
            if let Some(entity) = self.entities.get_mut(slot) {
                entity.inuse = false;
            }
            self.freed.push(slot);
        }
        fn temp_entity(&mut self, origin: Vec3, event: Q3EntityEvent) -> usize {
            let slot = self.spawn_entity().expect("temp slot");
            self.entities[slot].s.origin = origin;
            self.entities[slot].s.event = event as i32;
            slot
        }
        fn add_event(&mut self, slot: usize, event: Q3EntityEvent, parm: i32) {
            self.events.push((slot, event as i32, parm));
        }
        fn set_nextthink(&mut self, slot: usize, time: i32) {
            if let Some(entity) = self.entities.get_mut(slot) {
                entity.nextthink = time;
            }
            self.scheduled.push((slot, time));
        }
        fn callbacks(&self) -> &Q3CallbackCatalog {
            &self.callbacks
        }
        fn callbacks_mut(&mut self) -> &mut Q3CallbackCatalog {
            &mut self.callbacks
        }
        fn rankings(&self) -> &Q3RankingReports {
            &self.rankings
        }
        fn restore_counts(&mut self, num_entities: usize, _max_clients: usize) {
            self.num_entities = num_entities;
        }
    }

    #[derive(Clone)]
    struct DamageCall {
        target: Participant,
        amount: i32,
        flags: i32,
        method: i32,
    }

    struct StubCombat {
        product: Q3Product,
        game_type: i32,
        time: i32,
        calls: Vec<DamageCall>,
        states: HashMap<ActorId, ActorCombatState>,
    }

    impl CombatContext for StubCombat {
        fn product(&self) -> Q3Product {
            self.product
        }
        fn game_type(&self) -> i32 {
            self.game_type
        }
        fn time(&self) -> i32 {
            self.time
        }
        fn damage(
            &mut self,
            target: &Participant,
            _inflictor: Option<&Participant>,
            _attacker: Option<&Participant>,
            direction: Option<&mut Vec3>,
            _point: Option<Vec3>,
            amount: i32,
            flags: i32,
            method: i32,
        ) {
            if let Some(direction) = direction {
                *direction = normalize3(*direction);
            }
            self.calls.push(DamageCall {
                target: target.clone(),
                amount,
                flags,
                method,
            });
        }
        fn actor_combat_state(&self, actor: &ActorId) -> Option<ActorCombatState> {
            self.states.get(actor).cloned()
        }
    }

    struct StubWorld {
        linked: std::collections::HashSet<usize>,
        links: HashMap<usize, Q3LinkState>,
        area: Vec<usize>,
    }

    impl ServerWorldOps for StubWorld {
        fn link(&mut self, slot: usize) {
            self.linked.insert(slot);
        }
        fn unlink(&mut self, slot: usize) {
            self.linked.remove(&slot);
        }
        fn link_state(&self, slot: usize) -> Option<Q3LinkState> {
            self.links.get(&slot).copied()
        }
        fn area_entities(&self, _bounds: &Bounds, _maximum: usize) -> Vec<usize> {
            self.area.clone()
        }
    }

    struct StubSpatial {
        trace_result: Q3TraceResult,
        area: Vec<ActorId>,
    }

    fn clear_trace(end: Vec3) -> Q3TraceResult {
        Q3TraceResult {
            fraction: 1.0,
            end,
            solidity: Q3Solidity::Clear,
            contact: Q3TraceContact::None,
            contents: 0,
            surface_flags: 0,
            hit: Q3TraceHit::None,
        }
    }

    impl SpatialQueries for StubSpatial {
        fn area_actors(&self, _bounds: &Bounds, _maximum: usize) -> Vec<ActorId> {
            self.area.clone()
        }
        fn trace_actor(&self, _query: &Q3TraceQuery) -> Q3TraceResult {
            self.trace_result.clone()
        }
    }

    struct StubMoverActors {
        natives: HashMap<ActorId, usize>,
        bodies: HashMap<ActorId, SharedMoverBody>,
        writes: Vec<(ActorId, Vec3)>,
        released: Vec<ActorId>,
    }

    impl MoverActorAccess for StubMoverActors {
        fn native_slot(&self, actor: &ActorId) -> Option<usize> {
            self.natives.get(actor).copied()
        }
        fn participant(&self, actor: &ActorId) -> Participant {
            match self.natives.get(actor) {
                Some(slot) => Participant::Entity(*slot),
                None => Participant::SharedActor(actor.clone()),
            }
        }
        fn observe(&self, actor: &ActorId) -> Option<SharedMoverBody> {
            self.bodies.get(actor).cloned()
        }
        fn write(&mut self, actor: &ActorId, origin: Vec3, ground: Option<ActorId>) {
            self.writes.push((actor.clone(), origin));
            if let Some(body) = self.bodies.get_mut(actor) {
                body.state.origin = origin;
                body.state.ground = ground;
            }
        }
        fn link_actor(&mut self, _actor: &ActorId) {}
        fn release(&mut self, actor: &ActorId) {
            self.released.push(actor.clone());
        }
    }

    struct StubDriver {
        pool: StubPool,
        world: StubWorld,
        spatial: StubSpatial,
        combat: StubCombat,
        scratch: GameUtilityScratch,
        actors: StubMoverActors,
        warns: Vec<String>,
        logs: Vec<String>,
        sounds: HashMap<String, i32>,
        models: HashMap<String, i32>,
        gravity: f32,
        random: GameRandom,
        remaps: Vec<(String, String, f32)>,
        configstrings: HashMap<i32, String>,
        cvars: HashMap<String, String>,
        commands: Vec<(i32, String)>,
        use_targets_calls: Vec<(usize, Option<Participant>)>,
        portals: Vec<(usize, bool)>,
        dropped_flags: Vec<usize>,
        returned_flags: Vec<Q3Team>,
        scores: Vec<(usize, i32)>,
        exploded: Vec<usize>,
        teleports: Vec<(usize, Vec3, Vec3)>,
        map_travel: bool,
        touched_items: Vec<(usize, usize)>,
        dropped_items: Vec<(usize, usize, i32)>,
        flag_thinks: Vec<usize>,
        checked_items: Vec<usize>,
        live: HashMap<ActorId, bool>,
        players: HashMap<ActorId, bool>,
        native_map: HashMap<ActorId, usize>,
        origins: HashMap<ActorId, Vec3>,
        actor_events: Vec<(ActorId, i32, i32)>,
        items: Vec<Q3ItemDefinition>,
        brush_models: Vec<(usize, Option<String>)>,
        bullet_calls: Vec<(i32, i32)>,
        gauntlet_result: bool,
        lightning_calls: usize,
        shotgun_calls: usize,
        rail_hits: i32,
        rail_outcome: RailStatisticsOutcome,
    }

    impl StubDriver {
        fn new(owner: &IdentityOwner, product: Q3Product) -> Self {
            Self {
                pool: StubPool::new(owner, product),
                world: StubWorld {
                    linked: std::collections::HashSet::new(),
                    links: HashMap::new(),
                    area: Vec::new(),
                },
                spatial: StubSpatial {
                    trace_result: clear_trace(vec3(0.0, 0.0, 0.0)),
                    area: Vec::new(),
                },
                combat: StubCombat {
                    product,
                    game_type: 0,
                    time: 1000,
                    calls: Vec::new(),
                    states: HashMap::new(),
                },
                scratch: GameUtilityScratch::new(Rc::new(|_| {})),
                actors: StubMoverActors {
                    natives: HashMap::new(),
                    bodies: HashMap::new(),
                    writes: Vec::new(),
                    released: Vec::new(),
                },
                warns: Vec::new(),
                logs: Vec::new(),
                sounds: HashMap::new(),
                models: HashMap::new(),
                gravity: DEFAULT_GRAVITY,
                random: GameRandom::new(0),
                remaps: Vec::new(),
                configstrings: HashMap::new(),
                cvars: HashMap::new(),
                commands: Vec::new(),
                use_targets_calls: Vec::new(),
                portals: Vec::new(),
                dropped_flags: Vec::new(),
                returned_flags: Vec::new(),
                scores: Vec::new(),
                exploded: Vec::new(),
                teleports: Vec::new(),
                map_travel: false,
                touched_items: Vec::new(),
                dropped_items: Vec::new(),
                flag_thinks: Vec::new(),
                checked_items: Vec::new(),
                live: HashMap::new(),
                players: HashMap::new(),
                native_map: HashMap::new(),
                origins: HashMap::new(),
                actor_events: Vec::new(),
                items: Vec::new(),
                brush_models: Vec::new(),
                bullet_calls: Vec::new(),
                gauntlet_result: false,
                lightning_calls: 0,
                shotgun_calls: 0,
                rail_hits: 0,
                rail_outcome: RailStatisticsOutcome {
                    streak: 0,
                    hits: 0,
                    impressive_count: 0,
                    reward_until: 0,
                    awarded: false,
                },
            }
        }
    }

    impl Q3Driver for StubDriver {
        fn pool(&mut self) -> &mut dyn EntityPool {
            &mut self.pool
        }
        fn world(&mut self) -> &mut dyn ServerWorldOps {
            &mut self.world
        }
        fn spatial(&mut self) -> &mut dyn SpatialQueries {
            &mut self.spatial
        }
        fn combat(&mut self) -> &mut dyn CombatContext {
            &mut self.combat
        }
        fn scratch(&mut self) -> &mut GameUtilityScratch {
            &mut self.scratch
        }
        fn mover_actors(&mut self) -> &mut dyn MoverActorAccess {
            &mut self.actors
        }
        fn warn(&mut self, message: &str) {
            self.warns.push(message.to_string());
        }
        fn log(&mut self, message: &str) {
            self.logs.push(message.to_string());
        }
        fn sound_index(&mut self, path: &str) -> i32 {
            let next = self.sounds.len() as i32 + 1;
            *self.sounds.entry(path.to_string()).or_insert(next)
        }
        fn model_index(&mut self, name: Option<&str>) -> i32 {
            let next = self.models.len() as i32 + 1;
            *self.models.entry(name.unwrap_or("").to_string()).or_insert(next)
        }
        fn gravity(&self) -> f32 {
            self.gravity
        }
        fn game_rand(&mut self) -> i32 {
            self.random.rand()
        }
        fn game_random(&mut self) -> f32 {
            self.random.random()
        }
        fn game_crandom(&mut self) -> f32 {
            self.random.crandom()
        }
        fn remap_shader(&mut self, old: &str, new: &str, time_seconds: f32) {
            self.remaps.push((old.to_string(), new.to_string(), time_seconds));
        }
        fn set_configstring(&mut self, index: i32, value: &str) {
            self.configstrings.insert(index, value.to_string());
        }
        fn set_cvar(&mut self, name: &str, value: &str) {
            self.cvars.insert(name.to_string(), value.to_string());
        }
        fn send_server_command(&mut self, client: i32, command: &str) {
            self.commands.push((client, command.to_string()));
        }
        fn use_targets(&mut self, slot: usize, activator: Option<Participant>) {
            self.use_targets_calls.push((slot, activator));
        }
        fn adjust_area_portal(&mut self, slot: usize, open: bool) {
            self.portals.push((slot, open));
        }
        fn return_dropped_flag(&mut self, slot: usize) {
            self.dropped_flags.push(slot);
        }
        fn return_flag(&mut self, team: Q3Team) {
            self.returned_flags.push(team);
        }
        fn add_score(&mut self, player: usize, _origin: Vec3, points: i32) {
            self.scores.push((player, points));
        }
        fn explode_missile(&mut self, slot: usize) {
            self.exploded.push(slot);
        }
        fn teleport_player(&mut self, player: usize, origin: Vec3, angles: Vec3) {
            self.teleports.push((player, origin, angles));
        }
        fn map_travel_mode(&self) -> bool {
            self.map_travel
        }
        fn map_travel_teleport(&mut self, player: usize, origin: Vec3, angles: Vec3) {
            self.teleports.push((player, origin, angles));
        }
        fn map_travel_drop_flag(&mut self, player: usize) {
            self.dropped_flags.push(player);
        }
        fn touch_item(&mut self, item: usize, player: usize, _contact: &TouchContact) {
            self.touched_items.push((item, player));
        }
        fn drop_item(&mut self, entity: usize, item: usize, angle: i32) -> usize {
            self.dropped_items.push((entity, item, angle));
            self.pool.spawn_entity().expect("drop slot")
        }
        fn dropped_flag_think(&mut self, slot: usize) {
            self.flag_thinks.push(slot);
        }
        fn check_dropped_team_item(&mut self, slot: usize) {
            self.checked_items.push(slot);
        }
        fn actor_live(&self, actor: &ActorId) -> bool {
            self.live.get(actor).copied().unwrap_or(false)
        }
        fn actor_origin(&self, actor: &ActorId) -> Option<Vec3> {
            self.origins.get(actor).copied()
        }
        fn participant(&self, actor: &ActorId) -> Participant {
            match self.native_map.get(actor) {
                Some(slot) => Participant::Entity(*slot),
                None => Participant::SharedActor(actor.clone()),
            }
        }
        fn actor_is_player(&self, actor: &ActorId) -> bool {
            self.players.get(actor).copied().unwrap_or(false)
        }
        fn native_slot(&self, actor: &ActorId) -> Option<usize> {
            self.native_map.get(actor).copied()
        }
        fn actor_event(&mut self, actor: &ActorId, event: Q3EntityEvent, parm: i32) {
            self.actor_events.push((actor.clone(), event as i32, parm));
        }
        fn item_count(&self) -> usize {
            self.items.len()
        }
        fn item_at(&self, index: usize) -> Option<Q3ItemDefinition> {
            self.items.get(index).cloned()
        }
        fn find_item(&self, pickup_name: &str) -> Option<usize> {
            self.items
                .iter()
                .position(|item| item.pickup_name.as_deref() == Some(pickup_name))
        }
        fn find_item_for_powerup(&self, powerup: i32) -> Option<usize> {
            self.items
                .iter()
                .position(|item| item.item_type == Q3ItemType::Powerup && item.tag == powerup)
        }
        fn set_brush_model(&mut self, slot: usize, model: Option<&str>) {
            self.brush_models.push((slot, model.map(str::to_string)));
        }
        fn bullet_fire(
            &mut self,
            _host: &mut dyn BulletHost,
            _shooter: &ActorId,
            _attack: &mut BulletAttack,
            spread: i32,
            amount: i32,
        ) {
            self.bullet_calls.push((spread, amount));
        }
        fn gauntlet_attack(
            &mut self,
            _host: &mut dyn ContactHost,
            _shooter: &ActorId,
            _attack: &mut BulletAttack,
            _quad: bool,
        ) -> bool {
            self.gauntlet_result
        }
        fn lightning_fire(&mut self, _host: &mut dyn ContactHost, _shooter: &ActorId, _attack: &mut BulletAttack) {
            self.lightning_calls += 1;
        }
        fn shotgun_fire(&mut self, _host: &mut dyn ShotgunHost, _shooter: &ActorId, _attack: &mut BulletAttack) {
            self.shotgun_calls += 1;
        }
        fn rail_fire(&mut self, _host: &mut dyn RailHost, _shooter: &ActorId, _attack: &mut BulletAttack) -> i32 {
            self.rail_hits
        }
        fn rail_statistics(&mut self, _state: &RailStatistics, _hits: i32, _time: i32) -> RailStatisticsOutcome {
            self.rail_outcome
        }
    }

    struct StubLauncher {
        fires: Vec<String>,
        damage: i32,
        splash: i32,
    }

    impl MissileLauncher for StubLauncher {
        fn fire_grenade(
            &mut self,
            driver: &mut dyn Q3Driver,
            _entity: usize,
            _muzzle: Vec3,
            _direction: Vec3,
        ) -> usize {
            self.fires.push("grenade".to_string());
            let slot = driver.pool().spawn_entity().expect("missile");
            if let Some(entity) = driver.pool().entity_mut(slot) {
                entity.damage = self.damage;
                entity.splash_damage = self.splash;
            }
            slot
        }
        fn fire_rocket(&mut self, driver: &mut dyn Q3Driver, _entity: usize, _muzzle: Vec3, _direction: Vec3) -> usize {
            self.fires.push("rocket".to_string());
            let slot = driver.pool().spawn_entity().expect("missile");
            if let Some(entity) = driver.pool().entity_mut(slot) {
                entity.damage = self.damage;
                entity.splash_damage = self.splash;
            }
            slot
        }
        fn fire_plasma(&mut self, driver: &mut dyn Q3Driver, _entity: usize, _muzzle: Vec3, _direction: Vec3) -> usize {
            self.fires.push("plasma".to_string());
            let slot = driver.pool().spawn_entity().expect("missile");
            if let Some(entity) = driver.pool().entity_mut(slot) {
                entity.damage = self.damage;
                entity.splash_damage = self.splash;
            }
            slot
        }
        fn fire_bfg(&mut self, driver: &mut dyn Q3Driver, _entity: usize, _muzzle: Vec3, _direction: Vec3) -> usize {
            self.fires.push("bfg".to_string());
            let slot = driver.pool().spawn_entity().expect("missile");
            if let Some(entity) = driver.pool().entity_mut(slot) {
                entity.damage = self.damage;
                entity.splash_damage = self.splash;
            }
            slot
        }
        fn fire_grapple(
            &mut self,
            driver: &mut dyn Q3Driver,
            _entity: usize,
            _muzzle: Vec3,
            _direction: Vec3,
        ) -> usize {
            self.fires.push("grapple".to_string());
            driver.pool().spawn_entity().expect("missile")
        }
        fn fire_nail(
            &mut self,
            driver: &mut dyn Q3Driver,
            _entity: usize,
            _muzzle: Vec3,
            _forward: Vec3,
            _right: Vec3,
            _up: Vec3,
        ) -> usize {
            self.fires.push("nail".to_string());
            let slot = driver.pool().spawn_entity().expect("missile");
            if let Some(entity) = driver.pool().entity_mut(slot) {
                entity.damage = self.damage;
                entity.splash_damage = self.splash;
            }
            slot
        }
        fn fire_prox(&mut self, driver: &mut dyn Q3Driver, _entity: usize, _muzzle: Vec3, _direction: Vec3) -> usize {
            self.fires.push("prox".to_string());
            let slot = driver.pool().spawn_entity().expect("missile");
            if let Some(entity) = driver.pool().entity_mut(slot) {
                entity.damage = self.damage;
                entity.splash_damage = self.splash;
            }
            slot
        }
    }

    struct StubRecords {
        product: Q3Product,
        items: usize,
        ownership: Vec<OwnershipEntry>,
        backing: HashMap<usize, ClientBacking>,
        callbacks_restored: bool,
    }

    impl Q3EntityRecords for StubRecords {
        fn product(&self) -> Q3Product {
            self.product
        }
        fn item_count(&self) -> usize {
            self.items
        }
        fn capture_ownership(&self) -> Vec<OwnershipEntry> {
            self.ownership.clone()
        }
        fn restore_ownership(&mut self, entries: Vec<OwnershipEntry>) {
            self.ownership = entries;
        }
        fn capture_client_backing(&self, slot: usize) -> ClientBacking {
            self.backing.get(&slot).cloned().unwrap_or(ClientBacking {
                source_stats: Vec::new(),
                special_ammo: Vec::new(),
            })
        }
        fn restore_client_backing(&mut self, slot: usize, backing: &ClientBacking) {
            self.backing.insert(slot, backing.clone());
        }
        fn damage_inflictor(&self, actor: &ActorId) -> Participant {
            Participant::SharedActor(actor.clone())
        }
        fn restore_callbacks(&mut self) {
            self.callbacks_restored = true;
        }
    }

    struct StubRegistry {
        map: HashMap<SavedActorId, ActorId>,
    }

    impl Q3ActorRegistry for StubRegistry {
        fn resolve_saved(&self, saved: &SavedActorId) -> Option<ActorId> {
            self.map.get(saved).cloned()
        }
        fn reference_saved(&self, saved: &SavedActorId) -> ActorId {
            self.map.get(saved).cloned().expect("known actor")
        }
    }

    struct StubSpawnServices {
        memory: GameMemory,
        product: Q3Product,
        game_type: i32,
        handlers: SpawnHandlerTable,
        spawned_items: Vec<(usize, usize)>,
        warns: Vec<String>,
    }

    impl SpawnServices for StubSpawnServices {
        fn memory(&mut self) -> &mut GameMemory {
            &mut self.memory
        }
        fn product(&self) -> Q3Product {
            self.product
        }
        fn game_type(&self) -> i32 {
            self.game_type
        }
        fn handlers(&self) -> &SpawnHandlerTable {
            &self.handlers
        }
        fn spawn_item(
            &mut self,
            _driver: &mut dyn Q3Driver,
            slot: usize,
            item: usize,
            _variables: &SpawnVariables,
        ) -> Result<(), Q3GameError> {
            self.spawned_items.push((slot, item));
            Ok(())
        }
        fn warn(&mut self, message: &str) {
            self.warns.push(message.to_string());
        }
    }

    fn test_owner() -> IdentityOwner {
        IdentityOwner::create("test").expect("owner")
    }

    #[test]
    fn game_numeric_matches_bg_lib() {
        assert_eq!(game_atoi("  -42xyz").unwrap(), -42);
        assert_eq!(
            game_atoi("9999999999").unwrap(),
            999_999_999_i32.wrapping_mul(10).wrapping_add(9)
        );
        assert_eq!(game_atof("  -2.5 ").unwrap(), -2.5);
        assert_eq!(game_atof("abc").unwrap(), 0.0);
        assert_eq!(game_atof(".5").unwrap(), 0.5);
        let scan = scan_game_float("1.5 2.5", 0).unwrap();
        assert_eq!(scan.value, 1.5);
        assert_eq!(scan.next_offset, 4);
        assert_eq!(scan_game_vector("1 2 3").unwrap(), vec3(1.0, 2.0, 3.0));
        assert!(scan_game_float("1", 5).is_err());
        let mut random = GameRandom::new(0);
        assert_eq!(random.rand(), 1);
        assert_eq!(random.rand(), 3534);
        random.reset(0);
        assert_eq!(random.rand(), 1);
        let mut seeded = GameRandom::new(7);
        assert_eq!(seeded.rand(), 24_732);
        let mut left = GameRandom::new(7);
        let mut right = GameRandom::new(7);
        assert_eq!(left.rand(), right.rand());
        assert_eq!(left.random(), right.random());
        assert_eq!(left.crandom(), right.crandom());
    }

    #[test]
    fn game_format_matches_bg_lib() {
        assert_eq!(game_format("hi %d", &[GameFormatArg::Int(42)]).unwrap(), "hi 42");
        assert_eq!(game_format("%5d", &[GameFormatArg::Int(42)]).unwrap(), "   42");
        assert_eq!(game_format("%-5d|", &[GameFormatArg::Int(42)]).unwrap(), "42   |");
        assert_eq!(game_format("%05d", &[GameFormatArg::Int(42)]).unwrap(), "00042");
        assert_eq!(game_format("%f", &[GameFormatArg::Float(1.5)]).unwrap(), "1.500000");
        assert_eq!(game_format("%.2f", &[GameFormatArg::Float(1.5)]).unwrap(), "1.50");
        assert_eq!(game_format("%s", &[GameFormatArg::Text(None)]).unwrap(), "(null)");
        assert_eq!(game_format("100%%", &[]).unwrap(), "100%");
        assert_eq!(game_format("%c", &[GameFormatArg::Int(65)]).unwrap(), "A");
        assert_eq!(game_format("%i", &[GameFormatArg::Int(-7)]).unwrap(), "-7");
        assert!(game_format("%d", &[]).is_err());
        assert!(game_format("%d", &[GameFormatArg::Float(1.0)]).is_err());
        assert!(game_format("%s", &[GameFormatArg::Int(1)]).is_err());
        assert!(game_format("%d", &[GameFormatArg::Int(1)],).is_ok());
        assert!(game_format_sized("%d", &[GameFormatArg::Int(1)], 0).is_err());
        let big = "a".repeat(40_000);
        assert!(game_format("%s", &[GameFormatArg::Text(Some(big))]).is_err());
        assert_eq!(
            game_format_sized("%s", &[GameFormatArg::Text(Some("abcdef".to_string()))], 4).unwrap(),
            "abc"
        );
    }

    #[test]
    fn direction_byte_round_trip() {
        assert_eq!(direction_to_byte(None), 0);
        assert_eq!(direction_to_byte(Some(vec3(0.0, 0.0, 0.0))), 0);
        assert_eq!(direction_to_byte(Some(vec3(0.0, 0.0, 1.0))), 5);
        assert_eq!(byte_to_direction(5), vec3(0.0, 0.0, 1.0));
        assert_eq!(byte_to_direction(999), vec3(0.0, 0.0, 0.0));
        assert_eq!(byte_to_direction(-1), vec3(0.0, 0.0, 0.0));
    }

    #[test]
    fn trajectories_evaluate() {
        let linear = Q3Trajectory {
            trajectory_type: TrajectoryType::Linear,
            time: 0,
            duration: 0,
            base: vec3(0.0, 0.0, 0.0),
            delta: vec3(1000.0, 0.0, 0.0),
        };
        assert_eq!(evaluate_trajectory(&linear, 1000), vec3(1000.0, 0.0, 0.0));
        assert_eq!(evaluate_trajectory_delta(&linear, 500), vec3(1000.0, 0.0, 0.0));
        let gravity = Q3Trajectory {
            trajectory_type: TrajectoryType::Gravity,
            ..linear
        };
        assert_eq!(evaluate_trajectory(&gravity, 1000), vec3(1000.0, 0.0, -400.0));
        let sine = Q3Trajectory {
            trajectory_type: TrajectoryType::Sine,
            time: 0,
            duration: 1000,
            base: vec3(0.0, 0.0, 0.0),
            delta: vec3(10.0, 0.0, 0.0),
        };
        let at_quarter = evaluate_trajectory(&sine, 250);
        assert!((at_quarter.x - 10.0).abs() < 0.001);
        let stop = Q3Trajectory {
            trajectory_type: TrajectoryType::LinearStop,
            duration: 500,
            ..linear
        };
        assert_eq!(evaluate_trajectory(&stop, 1000), vec3(500.0, 0.0, 0.0));
        assert_eq!(evaluate_trajectory_delta(&stop, 1000), vec3(0.0, 0.0, 0.0));
        assert!(TrajectoryType::from_i32(9).is_err());
    }

    #[test]
    fn memory_and_spawn_strings() {
        let memory = GameMemory::new(Rc::new(|| 0), Rc::new(|_| {}));
        let mut services = StubSpawnServices {
            memory,
            product: Q3Product::Baseq3,
            game_type: 0,
            handlers: SpawnHandlerTable::new(),
            spawned_items: Vec::new(),
            warns: Vec::new(),
        };
        assert_eq!(new_spawn_string("a\\nb\\\\c", services.memory()).unwrap(), "a\nb\\c");
        assert_eq!(new_spawn_string("plain", services.memory()).unwrap(), "plain");
        assert!(new_spawn_string("trailing\\", services.memory()).is_ok());
        let mut big = GameMemory::new(Rc::new(|| 0), Rc::new(|_| {}));
        assert!(big.allocate(GAME_MEMORY_BYTES + 1).is_err());
        let allocation = big.allocate(8).unwrap();
        big.write_string(&allocation, "hi").unwrap();
        assert_eq!(big.read_string(&allocation).unwrap(), "hi");
        assert!(big.write_string(&allocation, "way too long for eight").is_err());
    }

    #[test]
    fn spawn_parser_and_dispatch() {
        let owner = test_owner();
        let mut driver = StubDriver::new(&owner, Q3Product::Baseq3);
        let mut services = StubSpawnServices {
            memory: GameMemory::new(Rc::new(|| 0), Rc::new(|_| {})),
            product: Q3Product::Baseq3,
            game_type: Q3GameType::SinglePlayer as i32,
            handlers: SpawnHandlerTable::new(),
            spawned_items: Vec::new(),
            warns: Vec::new(),
        };
        services
            .handlers
            .insert("test_thing", Rc::new(|_driver, _services, _slot, _vars| Ok(())));
        let mut parser =
            SpawnParser::new("// comment\n{\n\"classname\" \"test_thing\"\n/* block */\n}", "test").unwrap();
        let variables = parser.next().unwrap().unwrap();
        assert_eq!(variables.entries.len(), 1);
        assert!(parser.next().unwrap().is_none());
        let outcome = spawn_entity(&variables, &mut driver, &mut services).unwrap();
        assert!(matches!(
            outcome,
            SpawnOutcome::Dispatched {
                route: SpawnRoute::Handler,
                ..
            }
        ));
        let filtered = SpawnVariables::new(vec![
            SpawnPair {
                key: "classname".to_string(),
                value: "test_thing".to_string(),
            },
            SpawnPair {
                key: "notsingle".to_string(),
                value: "1".to_string(),
            },
        ])
        .unwrap();
        let outcome = spawn_entity(&filtered, &mut driver, &mut services).unwrap();
        assert!(matches!(
            outcome,
            SpawnOutcome::Filtered {
                reason: SpawnFilter::Notsingle,
                ..
            }
        ));
        let unknown = SpawnVariables::new(vec![SpawnPair {
            key: "classname".to_string(),
            value: "nope".to_string(),
        }])
        .unwrap();
        let outcome = spawn_entity(&unknown, &mut driver, &mut services).unwrap();
        assert!(matches!(outcome, SpawnOutcome::Unknown { .. }));
        assert_eq!(services.warns.len(), 1);
        let mut bad = SpawnParser::new("{ \"a\" ", "bad").unwrap();
        assert!(bad.next().is_err());
    }

    #[test]
    fn state_defaults_and_callbacks() {
        let owner = test_owner();
        let actor = owner.actor(3, 1);
        let entity = GameEntity::new(3, actor.clone()).unwrap();
        assert_eq!(entity.mover_state, MoverState::Pos1 as i32);
        assert!(GameEntity::new(5000, actor).is_err());
        let client = GameClient::new(Q3Product::Baseq3);
        assert_eq!(client.ammo_times.len(), 11);
        assert_eq!(GameClient::new(Q3Product::Missionpack).ammo_times.len(), 14);
        let mut catalog = Q3CallbackCatalog::new();
        let think: EntityThink = Rc::new(|_, _| {});
        catalog.think.register("id.a", Rc::clone(&think)).unwrap();
        assert!(catalog.think.register("id.b", Rc::clone(&think)).is_err());
        assert!(catalog.think.register("", Rc::clone(&think)).is_err());
        assert_eq!(catalog.think.capture(Some(&think)).unwrap(), Some("id.a".to_string()));
        assert!(catalog.think.resolve(Some("missing")).is_err());
        assert!(catalog.think.resolve(None).unwrap().is_none());
        let mut ps = Q3PlayerState::new(Q3Product::Baseq3);
        ps.stats.set(0, 77);
        assert_eq!(ps.health(), 77);
        ps.add_event(5, 6);
        assert_eq!(ps.event_sequence, 1);
        assert_eq!(ps.events.get(0), 5);
    }

    #[test]
    fn utilities_scratch_config_and_dispatch() {
        let owner = test_owner();
        let mut driver = StubDriver::new(&owner, Q3Product::Baseq3);
        let vector = driver.scratch.tv(1.0, 2.0, 3.0);
        assert_eq!(vector, TempVector { x: 1.0, y: 2.0, z: 3.0 });
        let text = driver.scratch.vtos(vec3(1.0, 2.0, 3.0)).unwrap().read_string();
        assert_eq!(text, "(1 2 3)");
        struct Store {
            values: HashMap<usize, String>,
        }
        impl ConfigStringStore for Store {
            fn get(&self, index: usize) -> String {
                self.values.get(&index).cloned().unwrap_or_default()
            }
            fn set(&mut self, index: usize, value: &str) {
                self.values.insert(index, value.to_string());
            }
        }
        let mut registry = ConfigStringRegistry::new(Store { values: HashMap::new() });
        assert_eq!(registry.model_index(Some("models/a")).unwrap(), 1);
        assert_eq!(registry.model_index(Some("models/a")).unwrap(), 1);
        assert_eq!(registry.model_index(None).unwrap(), 0);
        driver.pool.use_slot(10);
        driver.pool.entities[10].set_classname(Some("Target_Thing".to_string()));
        assert_eq!(
            find_entity(&driver.pool, None, EntityStringField::Classname, Some("target_thing")),
            Some(10)
        );
        assert_eq!(
            find_entity(
                &driver.pool,
                Some(10),
                EntityStringField::Classname,
                Some("target_thing")
            ),
            None
        );
        driver.pool.use_slot(11);
        driver.pool.entities[11].set_classname(Some("user".to_string()));
        driver.pool.entities[11].target = Some("Target_Thing".to_string());
        driver.pool.entities[10].targetname = Some("Target_Thing".to_string());
        let fired = Rc::new(RefCell::new(false));
        let fired_clone = Rc::clone(&fired);
        driver
            .pool
            .callbacks_mut()
            .use_callbacks
            .register(
                "test.use",
                Rc::new(move |_, _, _, _| {
                    *fired_clone.borrow_mut() = true;
                }),
            )
            .unwrap();
        let callback = driver.pool.callbacks().use_callbacks.resolve(Some("test.use")).unwrap();
        driver.pool.entities[10].use_callback = callback;
        use_targets(&mut driver, 11, None).unwrap();
        assert!(*fired.borrow());
        let (direction, zero) = move_direction(vec3(0.0, -1.0, 0.0));
        assert_eq!(direction, vec3(0.0, 0.0, 1.0));
        assert_eq!(zero, vec3(0.0, 0.0, 0.0));
    }

    #[test]
    fn projectile_launch_bounce_and_step() {
        let owner = test_owner();
        let actor = owner.actor(1, 1);
        let launch = q3_launch_projectile(vec3(0.0, 0.0, 0.0), vec3(1.0, 0.0, 0.0), 100.0, false, 5000, 1000);
        assert_eq!(launch.expires, 6000);
        assert_eq!(launch.trajectory.delta, vec3(100.0, 0.0, 0.0));
        assert_eq!(q3_missile_hit_time(0, 100, 0.5), 50);
        assert_eq!(
            q3_bounce_velocity(vec3(1.0, -1.0, 0.0), vec3(0.0, 1.0, 0.0), false),
            vec3(1.0, 1.0, 0.0)
        );
        assert_eq!(snap_vector(vec3(1.6, -1.6, 0.0)), vec3(1.0, -1.0, 0.0));
        assert_eq!(
            snap_vector_towards(vec3(1.2, 1.2, 0.0), vec3(5.0, 0.0, 0.0)),
            vec3(2.0, 1.0, 0.0)
        );
        struct Host {
            origin: Vec3,
            live: bool,
            released: bool,
            moved: usize,
            world: ActorId,
        }
        impl Q3ProjectileHost for Host {
            fn time(&self) -> i32 {
                1100
            }
            fn previous_time(&self) -> i32 {
                1000
            }
            fn is_live(&mut self) -> bool {
                self.live
            }
            fn phase(&mut self) -> ProjectilePhase {
                ProjectilePhase::Flight
            }
            fn event_time(&mut self) -> i32 {
                0
            }
            fn origin(&mut self) -> Vec3 {
                self.origin
            }
            fn move_to(&mut self, origin: Vec3, _velocity: Vec3) {
                self.origin = origin;
            }
            fn set_origin(&mut self, origin: Vec3) {
                self.origin = origin;
            }
            fn link(&mut self) {}
            fn release(&mut self) {
                self.released = true;
                self.live = false;
            }
            fn trace(&mut self, _start: Vec3, end: Vec3, _pass: Option<&ActorId>) -> Q3TraceResult {
                clear_trace(end)
            }
            fn target(&mut self, _actor: &ActorId) -> Option<Q3ProjectileTarget> {
                None
            }
            fn world_actor(&mut self) -> ActorId {
                self.world.clone()
            }
            fn emit(&mut self, _event: &Q3ProjectileImpact) {}
            fn retain(&mut self) {}
            fn damage(&mut self, _target: &ActorId, _direction: Vec3, _point: Vec3) {}
            fn radius(&mut self, _origin: Vec3, _ignore: Option<&ActorId>) -> bool {
                false
            }
            fn accuracy(&mut self) {}
            fn think(&mut self) {}
            fn moved(&mut self) {
                self.moved += 1;
            }
        }
        let mut host = Host {
            origin: vec3(0.0, 0.0, 0.0),
            live: true,
            released: false,
            moved: 0,
            world: owner.actor(1022, 1),
        };
        let mut projectile = Q3Projectile {
            actor: actor.clone(),
            owner: actor,
            weapon: 5,
            direct: 100,
            splash: 0,
            radius: 0,
            method: 6,
            splash_method: 7,
            damage_point: vec3(0.0, 0.0, 0.0),
            trajectory: launch.trajectory,
            flags: 0,
            pass: None,
        };
        q3_step_projectile(&mut projectile, &mut host);
        assert_eq!(host.moved, 1);
        assert!(!host.released);
    }

    #[test]
    fn radius_damage_falloff_and_visibility() {
        struct Host {
            spatial: StubSpatial,
            damages: Vec<(ActorId, i32)>,
        }
        impl Q3RadiusHost for Host {
            fn spatial(&mut self) -> &mut dyn SpatialQueries {
                &mut self.spatial
            }
            fn target(&mut self, actor: &ActorId) -> Option<Q3RadiusTarget> {
                Some(Q3RadiusTarget {
                    origin: vec3(51.0, 0.0, 0.0),
                    bounds: Bounds {
                        min: vec3(50.0, -1.0, -1.0),
                        max: vec3(52.0, 1.0, 1.0),
                    },
                    accuracy_eligible: actor.slot() == 1,
                })
            }
            fn damage(&mut self, actor: &ActorId, _direction: Vec3, _point: Vec3, amount: i32) {
                self.damages.push((actor.clone(), amount));
            }
        }
        let owner = test_owner();
        let one = owner.actor(1, 1);
        let two = owner.actor(2, 1);
        let mut host = Host {
            spatial: StubSpatial {
                trace_result: clear_trace(vec3(10.0, 0.0, 0.0)),
                area: vec![one.clone(), two.clone()],
            },
            damages: Vec::new(),
        };
        assert!(q3_radius_damage(&mut host, vec3(0.0, 0.0, 0.0), 100.0, 100.0, None));
        assert_eq!(host.damages.len(), 2);
        assert_eq!(host.damages[0].1, 50);
        host.damages.clear();
        assert!(!q3_radius_damage(
            &mut host,
            vec3(0.0, 0.0, 0.0),
            100.0,
            100.0,
            Some(&one)
        ));
        assert_eq!(host.damages.len(), 1);
    }

    #[test]
    fn rankings_gate_accumulate_and_deduplicate() {
        let reports = Q3RankingReports::new();
        let seen = Rc::new(RefCell::new(Vec::new()));
        let seen_clone = Rc::clone(&seen);
        let detach = reports
            .attach(
                Rc::new(move |report| seen_clone.borrow_mut().push(report)),
                Rc::new(|| false),
            )
            .unwrap();
        assert!(reports.attach(Rc::new(|_| {}), Rc::new(|| false)).is_err());
        reports.fire_weapon(0, Q3Weapon::Machinegun as i32);
        assert_eq!(seen.borrow().len(), 2);
        reports.damage(1, 2, 10, 3, 99, true, false);
        let after_first = seen.borrow().len();
        reports.damage(1, 2, 10, 3, 99, true, false);
        let after_second = seen.borrow().len();
        assert!(after_second > after_first);
        reports.player_die(1, 1022, 14);
        reports.team_name(0, "red");
        detach();
        let before = seen.borrow().len();
        reports.capture(0);
        assert_eq!(seen.borrow().len(), before);
    }

    #[test]
    fn shader_remaps_build_and_round_trip() {
        let mut registry = ShaderRemapRegistry::new(Rc::new(|_| {}));
        registry.add("old", "new", 1.5).unwrap();
        registry.add("OLD", "newer", 2.0).unwrap();
        assert_eq!(registry.remaps().len(), 1);
        assert_eq!(registry.remaps()[0].new_name, "newer");
        assert!(registry.add("bad\0x", "b", 0.0).is_ok());
        assert!(registry.add(&"a".repeat(64), "b", 0.0).is_err());
        let config = registry.build_shader_state_config().unwrap();
        assert!(config.starts_with("old=newer:    2.00@"));
        let saved = registry.capture_save_state();
        let mut restored = ShaderRemapRegistry::new(Rc::new(|_| {}));
        restored.restore_save_state(&saved).unwrap();
        assert_eq!(restored.remaps(), registry.remaps());
        let dup = arr(vec![
            obj(vec![
                ("oldName", str("a")),
                ("newName", str("b")),
                ("timeOffset", num(0.0)),
            ]),
            obj(vec![
                ("oldName", str("A")),
                ("newName", str("c")),
                ("timeOffset", num(0.0)),
            ]),
        ]);
        assert!(restored.restore_save_state(&dup).is_err());
    }

    #[test]
    fn save_values_round_trip() {
        let owner = test_owner();
        let mut entity = GameEntity::new(5, owner.actor(5, 1)).unwrap();
        entity.spawnflags = 3;
        entity.model = Some("m".to_string());
        entity.pos1 = vec3(1.0, 2.0, 3.0);
        entity.wait = 1.5;
        let values = capture_entity_values(&entity);
        let read = read_entity_values(&SaveReader::at(&entity_values_to_json(&values), "test")).unwrap();
        assert_eq!(values, read);
        let mut restored = GameEntity::new(5, owner.actor(5, 1)).unwrap();
        restore_entity_values(&mut restored, &read);
        assert_eq!(restored.spawnflags, 3);
        assert_eq!(restored.pos1, vec3(1.0, 2.0, 3.0));
        let mut client = GameClient::new(Q3Product::Baseq3);
        client.buttons = 9;
        client.ps.viewheight = 26.0;
        client.pers.netname = "name".to_string();
        let client_read = read_client_values(&SaveReader::at(
            &client_values_to_json(&capture_client_values(&client)),
            "test",
        ))
        .unwrap();
        assert_eq!(client_read.buttons, 9);
        let player_read = read_player_values(&SaveReader::at(
            &player_values_to_json(&capture_player_values(&client.ps)),
            "test",
        ))
        .unwrap();
        assert_eq!(player_read.viewheight, 26.0);
        let pers_read = read_persistant_values(&SaveReader::at(
            &persistant_values_to_json(&capture_persistant_values(&client.pers)),
            "test",
        ))
        .unwrap();
        assert_eq!(pers_read.netname, "name");
        let network_read = read_network_values(&SaveReader::at(
            &network_values_to_json(&capture_network_values(&entity.s)),
            "test",
        ))
        .unwrap();
        assert_eq!(network_read.number, entity.s.number);
        let mut level = Q3GameLevel::default();
        level.time = 42;
        level.vote.string = "map x".to_string();
        let level_json = capture_q3_level(&level);
        let mut restored_level = Q3GameLevel::default();
        restore_q3_level(&mut restored_level, &level_json).unwrap();
        assert_eq!(restored_level.time, 42);
        assert_eq!(restored_level.vote.string, "map x");
        let cvar = Q3CvarSnapshot {
            name: "g_x".to_string(),
            value: "1".to_string(),
            reset_value: "0".to_string(),
            latched_value: None,
            flags: 1,
            modified: true,
            modification_count: 2,
            numeric_value: 1.0,
            integer_value: 1,
        };
        let cvar_read = read_module_cvar(&SaveReader::at(&capture_module_cvar(&cvar), "test")).unwrap();
        assert_eq!(cvar_read, cvar);
    }

    #[test]
    fn graph_capture_prepare_restore_round_trip() {
        let owner = test_owner();
        let mut driver = StubDriver::new(&owner, Q3Product::Baseq3);
        driver.pool.use_slot(0);
        driver.pool.use_slot(1);
        driver.pool.use_slot(MAX_CLIENTS - 1);
        driver.pool.entities[0].set_classname(Some("worldspawn".to_string()));
        driver.pool.entities[1].set_classname(Some("thing".to_string()));
        driver.pool.entities[1].parent = Some(0);
        driver.pool.entities[1].client = Some(0);
        driver.pool.clients[0].ps.persistant.set(0, 66);
        let think: EntityThink = Rc::new(|_, _| {});
        driver
            .pool
            .callbacks_mut()
            .think
            .register("test.think", Rc::clone(&think))
            .unwrap();
        driver.pool.entities[1].think = Some(think);
        let mut records = StubRecords {
            product: Q3Product::Baseq3,
            items: 4,
            ownership: (0..MAX_GENTITIES)
                .map(|slot| OwnershipEntry {
                    actor: Some(owner.actor(slot as u32, 1)),
                    active: slot < 2,
                    borrowed: false,
                })
                .collect(),
            backing: HashMap::new(),
            callbacks_restored: false,
        };
        let graph = capture_q3_graph(&records, &driver.pool).unwrap();
        assert_eq!(graph.entities.len(), MAX_GENTITIES);
        assert_eq!(graph.clients.len(), MAX_CLIENTS);
        let json = graph_to_json(&graph);
        let parsed = read_q3_graph(&json).unwrap();
        assert_eq!(
            parsed.entities[1].values.spawnflags,
            graph.entities[1].values.spawnflags
        );
        assert_eq!(parsed.entities[1].think, Some("test.think".to_string()));
        let mut registry_map = HashMap::new();
        for slot in 0..MAX_GENTITIES {
            let actor = owner.actor(slot as u32, 1);
            registry_map.insert(SavedActorId::from(&actor), actor);
        }
        let registry = StubRegistry { map: registry_map };
        prepare_q3_graph(&mut records, &parsed, &registry).unwrap();
        let mut fresh = StubDriver::new(&owner, Q3Product::Baseq3);
        fresh
            .pool
            .callbacks_mut()
            .think
            .register("test.think", Rc::new(|_, _| {}))
            .unwrap();
        restore_q3_graph(&records, &mut fresh.pool, &parsed, &registry).unwrap();
        assert_eq!(fresh.pool.entities[1].parent, Some(0));
        assert!(fresh.pool.entities[1].think.is_some());
        assert_eq!(fresh.pool.clients[0].ps.persistant.get(0), 66);
        assert_eq!(fresh.pool.num_entities(), MAX_CLIENTS);
        let bad_value = int(5000);
        let bad_reader = SaveReader::at(&bad_value, "test");
        assert!(read_module_entity(&bad_reader, &driver.pool).is_err());
    }

    #[test]
    fn mover_binary_cycle() {
        let owner = test_owner();
        let mut driver = StubDriver::new(&owner, Q3Product::Baseq3);
        driver.combat.product = Q3Product::Baseq3;
        let runtime = MoverRuntime::new(900);
        let slot = driver.pool.spawn_entity().unwrap();
        driver.pool.entities[slot].pos1 = vec3(0.0, 0.0, 0.0);
        driver.pool.entities[slot].pos2 = vec3(0.0, 0.0, 100.0);
        let variables = SpawnVariables::new(Vec::new()).unwrap();
        let rt = Rc::new(RefCell::new(runtime));
        MoverRuntime::bind_save_callbacks(&rt, &mut driver).unwrap();
        rt.borrow().initialize_binary(&mut driver, slot, &variables).unwrap();
        assert_eq!(driver.pool.entities[slot].s.e_type, Q3EntityType::Mover as i32);
        assert!(driver.pool.entities[slot].s.pos.duration >= 1);
        rt.borrow().use_binary(&mut driver, slot, None, None).unwrap();
        assert_eq!(driver.pool.entities[slot].mover_state, MoverState::OneToTwo as i32);
        assert_eq!(driver.portals, vec![(slot, true)]);
        rt.borrow()
            .set_state(&mut driver, slot, MoverState::OneToTwo, 1000)
            .unwrap();
        rt.borrow().reached_binary(&mut driver, slot).unwrap();
        assert_eq!(driver.pool.entities[slot].mover_state, MoverState::Pos2 as i32);
        assert_eq!(driver.use_targets_calls.len(), 1);
        rt.borrow().return_to_pos1(&mut driver, slot).unwrap();
        assert_eq!(driver.pool.entities[slot].mover_state, MoverState::TwoToOne as i32);
        driver.combat.product = Q3Product::Missionpack;
        assert!(rt.borrow().run(&mut driver, slot).is_err());
    }

    #[test]
    fn personal_portals_drop_touch_and_save() {
        let owner = test_owner();
        let mut driver = StubDriver::new(&owner, Q3Product::Missionpack);
        driver.combat.product = Q3Product::Missionpack;
        driver.items.push(Q3ItemDefinition {
            class_name: None,
            pickup_name: None,
            item_type: Q3ItemType::Bad,
            tag: 0,
        });
        driver.items.push(Q3ItemDefinition {
            class_name: Some("item_portal".to_string()),
            pickup_name: Some("Portal".to_string()),
            item_type: Q3ItemType::Holdable,
            tag: 4,
        });
        let player = driver.pool.spawn_entity().unwrap();
        driver.pool.entities[player].client = Some(0);
        driver.pool.entities[player].health = 100;
        driver.pool.entities[player].takedamage = true;
        let rt = Rc::new(RefCell::new(PersonalPortalRuntime::new()));
        PersonalPortalRuntime::bind_save_callbacks(&rt, &mut driver).unwrap();
        rt.borrow_mut().drop_portal_destination(&mut driver, player).unwrap();
        assert_eq!(driver.pool.clients[0].portal_id, 1);
        rt.borrow_mut().drop_portal_source(&mut driver, player).unwrap();
        assert_eq!(driver.pool.clients[0].portal_id, 0);
        let saved = rt.borrow().capture_save_state();
        let mut restored = PersonalPortalRuntime::new();
        restored.restore_save_state(&saved).unwrap();
        assert_eq!(restored.portal_sequence(), 1);
        driver.combat.product = Q3Product::Baseq3;
        assert!(rt.borrow_mut().drop_portal_destination(&mut driver, player).is_err());
    }

    #[test]
    fn targets_dispatch_and_link() {
        let owner = test_owner();
        let mut driver = StubDriver::new(&owner, Q3Product::Baseq3);
        let locations = Rc::new(RefCell::new(TargetLocationState::new()));
        bind_target_save_callbacks(&mut driver, &locations).unwrap();
        let table = target_spawn_handlers(&locations);
        assert!(table.get("target_delay").is_some());
        assert!(table.get("target_push").is_some());
        assert!(table.get("target_location").is_some());
        let delay = driver.pool.spawn_entity().unwrap();
        let variables = SpawnVariables::new(Vec::new()).unwrap();
        spawn_target_delay(&mut driver, &locations, delay, &variables).unwrap();
        assert_eq!(driver.pool.entities[delay].wait, 1.0);
        let player = driver.pool.spawn_entity().unwrap();
        driver.pool.entities[player].client = Some(0);
        driver.pool.entities[player].takedamage = true;
        let activator = Participant::Entity(player);
        use_target_delay(&mut driver, &locations, delay, None, Some(&activator)).unwrap();
        assert!(driver.pool.entities[delay].nextthink > 1000);
        think_target_delay(&mut driver, delay).unwrap();
        assert_eq!(driver.use_targets_calls.len(), 1);
        let score = driver.pool.spawn_entity().unwrap();
        spawn_target_score(&mut driver, &locations, score).unwrap();
        assert_eq!(driver.pool.entities[score].count, 1);
        use_target_score(&mut driver, score, None, Some(&activator)).unwrap();
        assert_eq!(driver.scores, vec![(player, 1)]);
        let print = driver.pool.spawn_entity().unwrap();
        driver.pool.entities[print].message = Some("hello".to_string());
        use_target_print(&mut driver, print, None, Some(&activator)).unwrap();
        assert_eq!(
            driver.commands,
            vec![(SERVER_COMMAND_BROADCAST, "cp \"hello\"".to_string())]
        );
        let speaker = driver.pool.spawn_entity().unwrap();
        driver.pool.entities[speaker].spawnflags = 1;
        driver.pool.entities[speaker].noise_index = 7;
        use_target_speaker(&mut driver, speaker, None, None).unwrap();
        assert_eq!(driver.pool.entities[speaker].s.loop_sound, 7);
        let kill = driver.pool.spawn_entity().unwrap();
        use_target_kill(&mut driver, kill, None, Some(&activator)).unwrap();
        assert_eq!(driver.combat.calls.len(), 1);
        assert_eq!(driver.combat.calls[0].amount, 100_000);
        let relay = driver.pool.spawn_entity().unwrap();
        driver.pool.entities[relay].spawnflags = 1;
        assert!(use_target_relay(&mut driver, relay, None, None).is_err());
        let location = driver.pool.spawn_entity().unwrap();
        driver.pool.entities[location].set_classname(Some("target_location".to_string()));
        driver.pool.entities[location].message = Some("base".to_string());
        link_target_locations(&mut driver, &locations).unwrap();
        assert!(locations.borrow().linked);
        assert_eq!(driver.pool.entities[location].health, 1);
        assert_eq!(driver.configstrings.get(&CS_LOCATIONS), Some(&"unknown".to_string()));
        let saved = locations.borrow().capture_save_state();
        let mut restored = TargetLocationState::new();
        restored.restore_save_state(&saved, &driver.pool).unwrap();
        assert_eq!(restored.head, Some(location));
    }

    #[test]
    fn triggers_aim_fire_and_schedule() {
        let owner = test_owner();
        let mut driver = StubDriver::new(&owner, Q3Product::Baseq3);
        bind_trigger_save_callbacks(&mut driver).unwrap();
        let table = trigger_spawn_handlers();
        assert!(table.get("trigger_multiple").is_some());
        assert!(table.get("func_timer").is_some());
        let pad = driver.pool.spawn_entity().unwrap();
        let dest = driver.pool.spawn_entity().unwrap();
        driver.pool.entities[pad].target = Some("dest".to_string());
        driver.pool.entities[dest].targetname = Some("dest".to_string());
        driver.pool.entities[dest].s.origin = vec3(100.0, 0.0, 200.0);
        aim_at_target(&mut driver, pad, vec3(0.0, 0.0, 0.0)).unwrap();
        let velocity = driver.pool.entities[pad].s.origin2;
        assert!(velocity.x > 0.0);
        assert!(velocity.z > 0.0);
        let multi = driver.pool.spawn_entity().unwrap();
        let variables = SpawnVariables::new(vec![SpawnPair {
            key: "wait".to_string(),
            value: "2".to_string(),
        }])
        .unwrap();
        spawn_trigger_multiple(&mut driver, multi, &variables).unwrap();
        assert_eq!(driver.pool.entities[multi].wait, 2.0);
        assert!(driver.world.linked.contains(&multi));
        let player = driver.pool.spawn_entity().unwrap();
        driver.pool.entities[player].client = Some(0);
        let activator = Participant::Entity(player);
        multi_trigger(&mut driver, multi, Some(&activator)).unwrap();
        assert_eq!(driver.use_targets_calls.len(), 1);
        assert!(driver.pool.entities[multi].nextthink > 1000);
        let hurt = driver.pool.spawn_entity().unwrap();
        spawn_trigger_hurt(&mut driver, hurt).unwrap();
        assert_eq!(driver.pool.entities[hurt].damage, 5);
        driver.pool.entities[player].takedamage = true;
        touch_hurt(&mut driver, hurt, &activator).unwrap();
        assert_eq!(driver.combat.calls.len(), 1);
        assert_eq!(driver.combat.calls[0].method, MOD_TRIGGER_HURT);
        assert!(driver.pool.entities[hurt].timestamp > 1000);
        let timer = driver.pool.spawn_entity().unwrap();
        let timer_vars = SpawnVariables::new(Vec::new()).unwrap();
        spawn_func_timer(&mut driver, timer, &timer_vars).unwrap();
        use_func_timer(&mut driver, timer, Some(&activator)).unwrap();
        assert_eq!(driver.use_targets_calls.len(), 2);
    }

    #[test]
    fn weapons_fire_and_kamikaze() {
        let owner = test_owner();
        let mut driver = StubDriver::new(&owner, Q3Product::Baseq3);
        let shooter = driver.pool.spawn_entity().unwrap();
        driver.pool.entities[shooter].client = Some(0);
        driver.pool.entities[shooter].s.weapon = Q3Weapon::Machinegun as i32;
        driver.pool.entities[shooter].s.pos.base = vec3(0.0, 0.0, 0.0);
        driver.pool.clients[0].ps.viewheight = 26.0;
        let launcher = Box::new(StubLauncher {
            fires: Vec::new(),
            damage: 10,
            splash: 5,
        });
        let mut runtime = WeaponRuntime::new(3.0, launcher, Rc::new(|_, _| None), Rc::new(|_, _| {}));
        runtime.fire(&mut driver, shooter).unwrap();
        assert_eq!(driver.bullet_calls, vec![(200, 7)]);
        assert_eq!(driver.pool.clients[0].accuracy_shots, 1);
        let mut intersections_dir = vec3(1.0, 0.0, 0.0);
        let hits = ray_sphere_intersections(vec3(0.0, 0.0, 0.0), 1.0, vec3(-3.0, 0.0, 0.0), &mut intersections_dir);
        assert!(matches!(hits, SphereIntersections::Two(_, _)));
        let mut miss_dir = vec3(0.0, 1.0, 0.0);
        assert_eq!(
            ray_sphere_intersections(vec3(0.0, 0.0, 0.0), 1.0, vec3(0.0, 0.0, 5.0), &mut miss_dir),
            SphereIntersections::None
        );
        let client = GameClient::new(Q3Product::Missionpack);
        assert_eq!(q3_weapon_damage_factor(&client, 3.0, None, Q3Product::Missionpack), 1.0);
        let mut quad_client = GameClient::new(Q3Product::Missionpack);
        quad_client.ps.powerups.set(Q3Powerup::Quad as usize, 9999);
        assert_eq!(
            q3_weapon_damage_factor(
                &quad_client,
                3.0,
                Some(Q3Powerup::Doubler as i32),
                Q3Product::Missionpack
            ),
            6.0
        );
        assert!(!log_accuracy_hit(0, &mut driver, shooter, shooter));
        let victim = driver.pool.spawn_entity().unwrap();
        driver.pool.entities[victim].client = Some(1);
        driver.pool.entities[victim].takedamage = true;
        driver.pool.clients[1].ps.stats.set(0, 100);
        driver.pool.entities[shooter].takedamage = true;
        driver.pool.clients[0].ps.stats.set(0, 100);
        assert!(log_accuracy_hit(0, &mut driver, victim, shooter));
        let mut mission = StubDriver::new(&owner, Q3Product::Missionpack);
        mission.combat.product = Q3Product::Missionpack;
        let holder = mission.pool.spawn_entity().unwrap();
        mission.pool.entities[holder].client = Some(0);
        let reports = mission.pool.rankings().clone();
        let seen = Rc::new(RefCell::new(Vec::new()));
        let seen_clone = Rc::clone(&seen);
        reports
            .attach(
                Rc::new(move |report| seen_clone.borrow_mut().push(report)),
                Rc::new(|| false),
            )
            .unwrap();
        let launcher = Box::new(StubLauncher {
            fires: Vec::new(),
            damage: 10,
            splash: 5,
        });
        let runtime = WeaponRuntime::new(3.0, launcher, Rc::new(|_, _| None), Rc::new(|_, _| {}));
        let rt = Rc::new(RefCell::new(runtime));
        WeaponRuntime::bind_save_callbacks(&rt, &mut mission).unwrap();
        let timer = rt.borrow().start_kamikaze(&mut mission, holder).unwrap();
        assert_eq!(
            mission.pool.entities[timer].s.e_type,
            Q3EntityType::Events as i32 + Q3EntityEvent::Kamikaze as i32
        );
        assert_eq!(mission.pool.entities[timer].activator, Some(holder));
        assert_eq!(mission.combat.calls.len(), 1);
        mission.world.area = vec![holder];
        mission.world.links.insert(
            holder,
            Q3LinkState {
                absbounds: Bounds {
                    min: vec3(-16.0, -16.0, -24.0),
                    max: vec3(16.0, 16.0, 32.0),
                },
                linked: true,
                linkcount: 1,
            },
        );
        rt.borrow().kamikaze_damage(&mut mission, timer).unwrap();
        assert_eq!(mission.pool.entities[timer].count, 100);
        mission.pool.entities[timer].count = 2000;
        rt.borrow().kamikaze_damage(&mut mission, timer).unwrap();
        assert!(mission.pool.freed.contains(&timer));
    }

    #[test]
    fn use_participant_branches() {
        let owner = test_owner();
        let mut driver = StubDriver::new(&owner, Q3Product::Baseq3);
        assert!(require_use_participant(None).is_err());
        let slot = driver.pool.spawn_entity().unwrap();
        driver.pool.entities[slot].client = Some(0);
        let native = Participant::Entity(slot);
        assert_eq!(
            use_actor(&driver.pool, &native).unwrap(),
            driver.pool.entities[slot].actor.clone()
        );
        assert_eq!(use_client(&mut driver, &native).unwrap(), Some(slot));
        driver.pool.entities[slot].client = None;
        assert_eq!(use_client(&mut driver, &native).unwrap(), None);
        let shared = owner.actor(9000, 1);
        driver.live.insert(shared.clone(), true);
        driver.players.insert(shared.clone(), true);
        driver.native_map.insert(shared.clone(), slot);
        driver.pool.entities[slot].client = Some(0);
        assert_eq!(
            use_client(&mut driver, &Participant::SharedActor(shared.clone())).unwrap(),
            Some(slot)
        );
        driver.players.insert(shared.clone(), false);
        assert_eq!(
            use_client(&mut driver, &Participant::SharedActor(shared)).unwrap(),
            None
        );
    }

    #[test]
    fn spawn_world_and_level_errors() {
        let owner = test_owner();
        let mut driver = StubDriver::new(&owner, Q3Product::Baseq3);
        driver.pool.use_slot(ENTITYNUM_WORLD);
        let mut services = StubSpawnServices {
            memory: GameMemory::new(Rc::new(|| 0), Rc::new(|_| {})),
            product: Q3Product::Baseq3,
            game_type: 0,
            handlers: SpawnHandlerTable::new(),
            spawned_items: Vec::new(),
            warns: Vec::new(),
        };
        let mut world = WorldspawnState {
            start_time: 7,
            motd: "hi".to_string(),
            restarted: 0,
            do_warmup: 1,
            warmup_time: 0,
        };
        let report = spawn_entities(
            "{ \"classname\" \"worldspawn\" \"music\" \"m\" }",
            &mut driver,
            &mut services,
            &mut world,
            "test",
        )
        .unwrap();
        assert_eq!(report.outcomes.len(), 0);
        assert_eq!(world.warmup_time, -1);
        assert_eq!(driver.configstrings.get(&WORLDSPAWN_CS_MOTD), Some(&"hi".to_string()));
        assert_eq!(
            driver.configstrings.get(&WORLDSPAWN_CS_START_TIME),
            Some(&"7".to_string())
        );
        let mut level = Q3GameLevel::default();
        let bad = obj(vec![
            ("values", level_values_to_json(&capture_level_values(&level))),
            ("teamScores", numbers_to_json(&[1, 2])),
            ("numTeamVotingClients", numbers_to_json(&[0, 0])),
            ("sortedClients", numbers_to_json(&level.sorted_clients)),
            (
                "vote",
                obj(vec![
                    ("time", num_i32(0)),
                    ("yes", num_i32(0)),
                    ("no", num_i32(0)),
                    ("string", str("")),
                    ("displayString", str("")),
                    ("executeTime", num_i32(0)),
                ]),
            ),
            (
                "teamVotes",
                arr(vec![
                    obj(vec![
                        ("time", num_i32(0)),
                        ("yes", num_i32(0)),
                        ("no", num_i32(0)),
                        ("string", str("")),
                    ]),
                    obj(vec![
                        ("time", num_i32(0)),
                        ("yes", num_i32(0)),
                        ("no", num_i32(0)),
                        ("string", str("")),
                    ]),
                ]),
            ),
        ]);
        assert!(restore_q3_level(&mut level, &bad).is_err());
    }
}
