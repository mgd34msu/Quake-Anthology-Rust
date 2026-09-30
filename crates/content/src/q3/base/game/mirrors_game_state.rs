//! Quake III base game state (`q3_game_state`) support: shared mirrors, group error, and tests.
//!
//! Self-containment mirrors: minimal local copies of items the donors import from
//! modules outside this port (sibling q3 donors, engine contracts, and math/text
//! helpers), plus the group error type. Sibling-owned mirrors carry SIBLING-MIRROR
//! notes and unify with the canonical ports at merge time.

use crate::value::ValueError;
use qa_core::identity::ActorId;
use qa_core::math::add3;
use qa_core::math::angle_normalize180;
use qa_core::math::dot3;
use qa_core::math::scale3;
use qa_core::math::vec3;
use qa_core::math::vector_to_angles;
use qa_core::math::Bounds;
use qa_core::math::Vec3;
use qa_core::numeric::qvm_float_to_int;
use std::rc::Rc;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::game::rankings::*;
use crate::q3::base::game::save_callbacks::*;
use crate::q3::base::game::state::*;
use crate::q3::base::game::utilities::*;
use crate::q3::base::game::weapon::*;

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

pub(crate) fn failure(message: impl Into<String>) -> Q3GameError {
    Q3GameError::Failure(message.into())
}

pub(crate) fn range(message: impl Into<String>) -> Q3GameError {
    Q3GameError::Range(message.into())
}

pub(crate) fn drop_error(message: impl Into<String>) -> Q3GameError {
    Q3GameError::Drop(message.into())
}

/// Validate a Latin-1 byte string (`checkByteString` / `byteString` donors).
pub(crate) fn latin1_bytes(value: &str) -> Result<Vec<u8>, Q3GameError> {
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
pub(crate) fn latin1_string(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| char::from_u32(u32::from(*byte)).unwrap_or('\u{FFFD}'))
        .collect()
}

/// Fold ASCII uppercase to lowercase without touching other bytes.
pub(crate) fn ascii_lower(bytes: &[u8]) -> Vec<u8> {
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

pub(crate) fn trajectory_seconds(milliseconds: i32) -> f32 {
    (milliseconds as f32) * 0.001
}

pub(crate) fn periodic_radians(tr: &Q3Trajectory, at_time: i32) -> f32 {
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Q3CollisionModel {
    /// Inline BSP model.
    Inline {
        /// Model index.
        index: i32,
    },
    /// Box.
    #[default]
    Box,
    /// Capsule.
    Capsule,
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

pub(crate) const FORMAT_LADJUST: i32 = 0x04;

pub(crate) const FORMAT_ZEROPAD: i32 = 0x80;

pub(crate) struct FormatOutput {
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
        self.value.extend(std::iter::repeat_n((byte & 255) as u8, count));
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

pub(crate) fn format_byte_length(value: &[u8], limit: usize) -> usize {
    value
        .iter()
        .take(limit.min(value.len()))
        .take_while(|byte| **byte != 0)
        .count()
}

pub(crate) fn format_argument(
    args: &[GameFormatArg],
    index: usize,
    specifier: &str,
) -> Result<GameFormatArg, Q3GameError> {
    args.get(index)
        .cloned()
        .ok_or_else(|| range(format!("missing argument {index} for %{specifier}")))
}

pub(crate) fn format_integer(value: &GameFormatArg, index: usize, specifier: &str) -> Result<i32, Q3GameError> {
    match value {
        GameFormatArg::Int(value) => Ok(*value),
        _ => Err(failure(format!("argument {index} for %{specifier} must be a number"))),
    }
}

pub(crate) fn format_float(value: &GameFormatArg, index: usize) -> Result<f32, Q3GameError> {
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

pub(crate) fn reversed_integer_bytes(value: i32) -> Vec<i32> {
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

pub(crate) fn add_int(output: &mut FormatOutput, value: i32, width: i32, flags: i32) -> Result<(), Q3GameError> {
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

pub(crate) fn add_float(output: &mut FormatOutput, value: f32, width: i32, precision: i32) -> Result<(), Q3GameError> {
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

pub(crate) fn add_string(
    output: &mut FormatOutput,
    value: Option<&str>,
    width: i32,
    precision: i32,
) -> Result<(), Q3GameError> {
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
    while let Some(literal) = byte_at(cursor) {
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
                while let Some(digit) = byte_at(cursor) {
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
pub(crate) const BYTE_DIRECTIONS: [[f32; 3]; 162] = [
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q3::base::game::projectile::*;
    use crate::q3::base::game::radius_damage::*;
    use crate::q3::base::game::save_state::*;
    use crate::q3::base::game::spawn::*;
    use crate::q3::base::game::use_participant::*;
    use crate::value::arr;

    use crate::value::int;
    use crate::value::num;
    use crate::value::obj;
    use crate::value::str;

    use crate::value::SaveReader;

    use qa_core::identity::ActorId;
    use qa_core::identity::SavedActorId;

    use qa_core::math::normalize3;

    use qa_core::math::vec3;

    use qa_core::math::Bounds;
    use qa_core::math::Vec3;

    use crate::q3::base::game::mover::*;
    use crate::q3::base::game::numeric::*;
    use crate::q3::base::game::personal_portal::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    use crate::q3::base::game::save_level::*;
    use crate::q3::base::game::save_module_values::*;
    use crate::q3::base::game::save_reader::*;

    use crate::q3::base::game::save_values::*;
    use crate::q3::base::game::shader_remaps::*;

    use crate::q3::base::game::targets::*;
    use crate::q3::base::game::triggers::*;

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
        amount: i32,
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
            _target: &Participant,
            _inflictor: Option<&Participant>,
            _attacker: Option<&Participant>,
            direction: Option<&mut Vec3>,
            _point: Option<Vec3>,
            amount: i32,
            _flags: i32,
            method: i32,
        ) {
            if let Some(direction) = direction {
                *direction = normalize3(*direction);
            }
            self.calls.push(DamageCall { amount, method });
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
        let mut level = Q3GameLevel {
            time: 42,
            ..Default::default()
        };
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
