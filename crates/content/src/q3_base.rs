//! Quake III base-game root: shared gameplay records, world queries, settings, and items.
//!
//! Donor provenance: `src/content/q3/base/bot-debug.ts`,
//! `src/content/q3/base/combat-bridge.ts`, `src/content/q3/base/map-spawns.ts`,
//! `src/content/q3/base/records.ts`, `src/content/q3/base/settings.ts`,
//! `src/content/q3/base/world-adapter.ts`, `src/content/q3/base/world.ts`,
//! `src/content/q3/base/shared/definitions.ts`,
//! `src/content/q3/base/shared/direction-byte.ts`,
//! `src/content/q3/base/shared/entity-shared.ts`,
//! `src/content/q3/base/shared/entity-state.ts` (via
//! `src/network/q3/state/entity.ts`), `src/content/q3/base/shared/items.ts`,
//! `src/content/q3/base/shared/jump-pad.ts`,
//! `src/content/q3/base/shared/player-state.ts`,
//! `src/content/q3/base/shared/slide-move.ts`,
//! `src/content/q3/base/shared/snapshot-state.ts`,
//! `src/content/q3/base/shared/trajectory.ts`, and
//! `src/content/q3/product-restriction.ts` (`bg_public.h`, `bg_misc.c`,
//! `q_math.c`, `q_shared.h`, `g_local.h`, `g_combat.c`, `files.c`).
//!
//! `src/content/q3/base/index.ts` is a pure re-export barrel over in-scope
//! game files and carries no content of its own, so it is noted here and
//! skipped. `entity-state.ts` and `slide-move.ts` are barrels over
//! out-of-scope or in-scope content; their content is ported below.
//!
//! Numeric mapping: donor `number` values that id's C sources store as
//! `float` are `f32` here (matching [`qa_core::math::Vec3`]); integer
//! words (times, indices, flags, health, armor) are `i32`. Donor `Math.imul`
//! and `| 0` map to wrapping `i32` arithmetic. Session-corruption throws
//! (missing projections, null think callbacks) are panics; map/save-data
//! failures are [`Q3BaseError`] or [`ValueError`](crate::value::ValueError).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use qa_core::cvar::{flags as cvar_flags, CvarRegistry, CvarSnapshot};
use qa_core::identity::{ActorId, OwnedActor, ProviderId};
use qa_core::math::{add3, angle_normalize180, dot3, scale3, vec3, vector_to_angles, Bounds, Plane, Vec3};
use qa_core::numeric::{native_atof, NumericProfile, Q3_BINARY32_PROFILE};
use qa_core::time::SourceTime;
use thiserror::Error;

use crate::value::{arr, boolean, int, num, obj, str as save_str, SaveJson, SaveReader, ValueError};

/// Base-game failure for map/save-data inputs (donor `Error`, `RangeError`,
/// and `CommonError("drop", ...)`).
#[derive(Debug, Clone, PartialEq, Error)]
pub enum Q3BaseError {
    /// Dropped-operation failure (donor `CommonError("drop", ...)`).
    #[error("drop: {0}")]
    Drop(String),
    /// Invalid map/save/entity input (donor `Error`).
    #[error("invalid: {0}")]
    Invalid(String),
    /// Out-of-range map/save/entity input (donor `RangeError`).
    #[error("range: {0}")]
    Range(String),
}

// ---------------------------------------------------------------------------
// shared/definitions.ts (+ movement/q3/constants.ts re-exports)
// ---------------------------------------------------------------------------

/// Q3 product family (`Product`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Product {
    /// Base game.
    BaseQ3,
    /// Missionpack / Team Arena.
    Missionpack,
}

/// Default gravity (`DEFAULT_GRAVITY`).
pub const DEFAULT_GRAVITY: i32 = 800;
/// Health at or below which a player gibs (`GIB_HEALTH`).
pub const GIB_HEALTH: i32 = -40;
/// Q3 armor absorption (`ARMOR_PROTECTION`).
pub const ARMOR_PROTECTION: f32 = 0.66;
/// Item table bound (`MAX_ITEMS`).
pub const MAX_ITEMS: i32 = 256;
/// Event lifetime in milliseconds (`EVENT_VALID_MSEC`).
pub const EVENT_VALID_MSEC: i32 = 300;
/// Event sequence bit 1 (`EV_EVENT_BIT1`).
pub const EV_EVENT_BIT1: i32 = 0x100;
/// Event sequence bit 2 (`EV_EVENT_BIT2`).
pub const EV_EVENT_BIT2: i32 = 0x200;
/// Event sequence bits (`EV_EVENT_BITS`).
pub const EV_EVENT_BITS: i32 = 0x300;

/// Game type (`GameType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum GameType {
    /// Free for all.
    GtFfa = 0,
    /// Tournament.
    GtTournament = 1,
    /// Single player.
    GtSinglePlayer = 2,
    /// Team deathmatch.
    GtTeam = 3,
    /// Capture the flag.
    GtCtf = 4,
    /// One-flag CTF.
    Gt1Fctf = 5,
    /// Overload (obelisks).
    GtObelisk = 6,
    /// Harvester.
    GtHarvester = 7,
    /// Game type count.
    GtMaxGameType = 8,
}

impl GameType {
    /// Convert a raw integer tag, if it names a game type.
    #[must_use]
    pub fn from_i32(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::GtFfa),
            1 => Some(Self::GtTournament),
            2 => Some(Self::GtSinglePlayer),
            3 => Some(Self::GtTeam),
            4 => Some(Self::GtCtf),
            5 => Some(Self::Gt1Fctf),
            6 => Some(Self::GtObelisk),
            7 => Some(Self::GtHarvester),
            8 => Some(Self::GtMaxGameType),
            _ => None,
        }
    }
}

/// Team (`Team`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum Team {
    /// No team.
    TeamFree = 0,
    /// Red team.
    TeamRed = 1,
    /// Blue team.
    TeamBlue = 2,
    /// Spectator.
    TeamSpectator = 3,
    /// Team count.
    TeamNumTeams = 4,
}

impl Team {
    /// Convert a raw integer tag, if it names a team.
    #[must_use]
    pub fn from_i32(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::TeamFree),
            1 => Some(Self::TeamRed),
            2 => Some(Self::TeamBlue),
            3 => Some(Self::TeamSpectator),
            4 => Some(Self::TeamNumTeams),
            _ => None,
        }
    }
}

/// Item type (`ItemType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum ItemType {
    /// Reserved empty item.
    ItBad = 0,
    /// Weapon.
    ItWeapon = 1,
    /// Ammunition.
    ItAmmo = 2,
    /// Armor.
    ItArmor = 3,
    /// Health.
    ItHealth = 4,
    /// Powerup.
    ItPowerup = 5,
    /// Holdable.
    ItHoldable = 6,
    /// Persistant powerup.
    ItPersistantPowerup = 7,
    /// Team item.
    ItTeam = 8,
}

impl ItemType {
    /// Convert a raw integer tag, if it names an item type.
    #[must_use]
    pub fn from_i32(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::ItBad),
            1 => Some(Self::ItWeapon),
            2 => Some(Self::ItAmmo),
            3 => Some(Self::ItArmor),
            4 => Some(Self::ItHealth),
            5 => Some(Self::ItPowerup),
            6 => Some(Self::ItHoldable),
            7 => Some(Self::ItPersistantPowerup),
            8 => Some(Self::ItTeam),
            _ => None,
        }
    }
}

/// Entity type (`EntityType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum EntityType {
    /// General entity.
    EtGeneral = 0,
    /// Player.
    EtPlayer = 1,
    /// Item.
    EtItem = 2,
    /// Missile.
    EtMissile = 3,
    /// Mover.
    EtMover = 4,
    /// Beam.
    EtBeam = 5,
    /// Portal.
    EtPortal = 6,
    /// Speaker.
    EtSpeaker = 7,
    /// Push trigger.
    EtPushTrigger = 8,
    /// Teleport trigger.
    EtTeleportTrigger = 9,
    /// Invisible.
    EtInvisible = 10,
    /// Grapple.
    EtGrapple = 11,
    /// Team entity.
    EtTeam = 12,
    /// Event-only entity.
    EtEvents = 13,
}

impl EntityType {
    /// Convert a raw integer tag, if it names an entity type.
    #[must_use]
    pub fn from_i32(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::EtGeneral),
            1 => Some(Self::EtPlayer),
            2 => Some(Self::EtItem),
            3 => Some(Self::EtMissile),
            4 => Some(Self::EtMover),
            5 => Some(Self::EtBeam),
            6 => Some(Self::EtPortal),
            7 => Some(Self::EtSpeaker),
            8 => Some(Self::EtPushTrigger),
            9 => Some(Self::EtTeleportTrigger),
            10 => Some(Self::EtInvisible),
            11 => Some(Self::EtGrapple),
            12 => Some(Self::EtTeam),
            13 => Some(Self::EtEvents),
            _ => None,
        }
    }
}

/// Persistant player indices (`PersistentIndex`, `PERS_*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum PersistentIndex {
    /// Score.
    PersScore = 0,
    /// Hit count.
    PersHits = 1,
    /// Rank.
    PersRank = 2,
    /// Team.
    PersTeam = 3,
    /// Spawn count.
    PersSpawnCount = 4,
    /// Player events.
    PersPlayerEvents = 5,
    /// Attacker client number.
    PersAttacker = 6,
    /// Attackee armor snapshot.
    PersAttackeeArmor = 7,
    /// Killed flag.
    PersKilled = 8,
    /// Impressive count.
    PersImpressiveCount = 9,
    /// Excellent count.
    PersExcellentCount = 10,
    /// Defend count.
    PersDefendCount = 11,
    /// Assist count.
    PersAssistCount = 12,
    /// Gauntlet frag count.
    PersGauntletFragCount = 13,
    /// Capture count.
    PersCaptures = 14,
}

/// Base-game stat indices (`BaseStatIndex`, `STAT_*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum BaseStatIndex {
    /// Health.
    StatHealth = 0,
    /// Holdable item.
    StatHoldableItem = 1,
    /// Weapon bits.
    StatWeapons = 2,
    /// Armor.
    StatArmor = 3,
    /// Dead yaw.
    StatDeadYaw = 4,
    /// Clients ready mask.
    StatClientsReady = 5,
    /// Maximum health.
    StatMaxHealth = 6,
}

/// Missionpack stat indices (`MissionpackStatIndex`, `STAT_*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum MissionpackStatIndex {
    /// Health.
    StatHealth = 0,
    /// Holdable item.
    StatHoldableItem = 1,
    /// Persistant powerup.
    StatPersistantPowerup = 2,
    /// Weapon bits.
    StatWeapons = 3,
    /// Armor.
    StatArmor = 4,
    /// Dead yaw.
    StatDeadYaw = 5,
    /// Clients ready mask.
    StatClientsReady = 6,
    /// Maximum health.
    StatMaxHealth = 7,
}

/// Shared stat slot indices (`StatSchema`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatIndices {
    /// Health slot.
    pub health: usize,
    /// Holdable item slot.
    pub holdable_item: usize,
    /// Weapon bits slot.
    pub weapons: usize,
    /// Armor slot.
    pub armor: usize,
    /// Dead yaw slot.
    pub dead_yaw: usize,
    /// Clients ready slot.
    pub clients_ready: usize,
    /// Maximum health slot.
    pub max_health: usize,
}

/// Product-tagged stat schema (`statSchema` result).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatSchema {
    /// Base-game schema.
    Base {
        /// Shared indices.
        indices: StatIndices,
    },
    /// Missionpack schema with its extra powerup slot.
    Missionpack {
        /// Shared indices.
        indices: StatIndices,
        /// Persistant powerup slot.
        persistent_powerup: usize,
    },
}

impl StatSchema {
    /// Shared stat indices.
    #[must_use]
    pub fn indices(&self) -> &StatIndices {
        match self {
            Self::Base { indices } | Self::Missionpack { indices, .. } => indices,
        }
    }

    /// Owning product.
    #[must_use]
    pub fn product(&self) -> Product {
        match self {
            Self::Base { .. } => Product::BaseQ3,
            Self::Missionpack { .. } => Product::Missionpack,
        }
    }

    /// Persistant powerup slot, missionpack only.
    #[must_use]
    pub fn persistent_powerup(&self) -> Option<usize> {
        match self {
            Self::Base { .. } => None,
            Self::Missionpack { persistent_powerup, .. } => Some(*persistent_powerup),
        }
    }
}

/// Stat schema for a product (`statSchema`).
#[must_use]
pub fn stat_schema(product: Product) -> StatSchema {
    match product {
        Product::BaseQ3 => StatSchema::Base {
            indices: StatIndices {
                health: BaseStatIndex::StatHealth as usize,
                holdable_item: BaseStatIndex::StatHoldableItem as usize,
                weapons: BaseStatIndex::StatWeapons as usize,
                armor: BaseStatIndex::StatArmor as usize,
                dead_yaw: BaseStatIndex::StatDeadYaw as usize,
                clients_ready: BaseStatIndex::StatClientsReady as usize,
                max_health: BaseStatIndex::StatMaxHealth as usize,
            },
        },
        Product::Missionpack => StatSchema::Missionpack {
            indices: StatIndices {
                health: MissionpackStatIndex::StatHealth as usize,
                holdable_item: MissionpackStatIndex::StatHoldableItem as usize,
                weapons: MissionpackStatIndex::StatWeapons as usize,
                armor: MissionpackStatIndex::StatArmor as usize,
                dead_yaw: MissionpackStatIndex::StatDeadYaw as usize,
                clients_ready: MissionpackStatIndex::StatClientsReady as usize,
                max_health: MissionpackStatIndex::StatMaxHealth as usize,
            },
            persistent_powerup: MissionpackStatIndex::StatPersistantPowerup as usize,
        },
    }
}

/// Player movement type (`MoveType`, re-exported from movement constants).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum MoveType {
    /// Normal movement.
    PmNormal = 0,
    /// Noclip.
    PmNoclip = 1,
    /// Spectator.
    PmSpectator = 2,
    /// Dead.
    PmDead = 3,
    /// Frozen.
    PmFreeze = 4,
    /// Intermission.
    PmIntermission = 5,
    /// Single-player intermission.
    PmSpIntermission = 6,
}

impl MoveType {
    /// Convert a raw integer tag, if it names a movement type.
    #[must_use]
    pub fn from_i32(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::PmNormal),
            1 => Some(Self::PmNoclip),
            2 => Some(Self::PmSpectator),
            3 => Some(Self::PmDead),
            4 => Some(Self::PmFreeze),
            5 => Some(Self::PmIntermission),
            6 => Some(Self::PmSpIntermission),
            _ => None,
        }
    }
}

/// Weapon state (`WeaponState`, re-exported from movement constants).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum WeaponState {
    /// Ready.
    WeaponReady = 0,
    /// Raising.
    WeaponRaising = 1,
    /// Dropping.
    WeaponDropping = 2,
    /// Firing.
    WeaponFiring = 3,
}

impl WeaponState {
    /// Convert a raw integer tag, if it names a weapon state.
    #[must_use]
    pub fn from_i32(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::WeaponReady),
            1 => Some(Self::WeaponRaising),
            2 => Some(Self::WeaponDropping),
            3 => Some(Self::WeaponFiring),
            _ => None,
        }
    }
}

/// Powerup tag (`Powerup`, re-exported from movement constants).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum Powerup {
    /// None.
    PwNone = 0,
    /// Quad damage.
    PwQuad = 1,
    /// Battlesuit.
    PwBattlesuit = 2,
    /// Haste.
    PwHaste = 3,
    /// Invisibility.
    PwInvis = 4,
    /// Regeneration.
    PwRegen = 5,
    /// Flight.
    PwFlight = 6,
    /// Red flag.
    PwRedFlag = 7,
    /// Blue flag.
    PwBlueFlag = 8,
    /// Neutral flag.
    PwNeutralFlag = 9,
    /// Scout.
    PwScout = 10,
    /// Guard.
    PwGuard = 11,
    /// Doubler.
    PwDoubler = 12,
    /// Ammo regen.
    PwAmmoregen = 13,
    /// Invulnerability.
    PwInvulnerability = 14,
    /// Powerup count.
    PwNumPowerups = 15,
}

impl Powerup {
    /// Convert a raw integer tag, if it names a powerup.
    #[must_use]
    pub fn from_i32(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::PwNone),
            1 => Some(Self::PwQuad),
            2 => Some(Self::PwBattlesuit),
            3 => Some(Self::PwHaste),
            4 => Some(Self::PwInvis),
            5 => Some(Self::PwRegen),
            6 => Some(Self::PwFlight),
            7 => Some(Self::PwRedFlag),
            8 => Some(Self::PwBlueFlag),
            9 => Some(Self::PwNeutralFlag),
            10 => Some(Self::PwScout),
            11 => Some(Self::PwGuard),
            12 => Some(Self::PwDoubler),
            13 => Some(Self::PwAmmoregen),
            14 => Some(Self::PwInvulnerability),
            15 => Some(Self::PwNumPowerups),
            _ => None,
        }
    }
}

/// Holdable tag (`Holdable`, re-exported from movement constants).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum Holdable {
    /// None.
    HiNone = 0,
    /// Teleporter.
    HiTeleporter = 1,
    /// Medkit.
    HiMedkit = 2,
    /// Kamikaze.
    HiKamikaze = 3,
    /// Portal.
    HiPortal = 4,
    /// Invulnerability.
    HiInvulnerability = 5,
    /// Holdable count.
    HiNumHoldable = 6,
}

impl Holdable {
    /// Convert a raw integer tag, if it names a holdable.
    #[must_use]
    pub fn from_i32(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::HiNone),
            1 => Some(Self::HiTeleporter),
            2 => Some(Self::HiMedkit),
            3 => Some(Self::HiKamikaze),
            4 => Some(Self::HiPortal),
            5 => Some(Self::HiInvulnerability),
            6 => Some(Self::HiNumHoldable),
            _ => None,
        }
    }
}

/// Weapon tag (`Weapon`, re-exported from movement constants).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum Weapon {
    /// None.
    WpNone = 0,
    /// Gauntlet.
    WpGauntlet = 1,
    /// Machinegun.
    WpMachinegun = 2,
    /// Shotgun.
    WpShotgun = 3,
    /// Grenade launcher.
    WpGrenadeLauncher = 4,
    /// Rocket launcher.
    WpRocketLauncher = 5,
    /// Lightning gun.
    WpLightning = 6,
    /// Railgun.
    WpRailgun = 7,
    /// Plasma gun.
    WpPlasmagun = 8,
    /// BFG.
    WpBfg = 9,
    /// Grappling hook.
    WpGrapplingHook = 10,
    /// Nailgun.
    WpNailgun = 11,
    /// Proximity mine launcher.
    WpProxLauncher = 12,
    /// Chaingun.
    WpChaingun = 13,
}

impl Weapon {
    /// Convert a raw integer tag, if it names a weapon.
    #[must_use]
    pub fn from_i32(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::WpNone),
            1 => Some(Self::WpGauntlet),
            2 => Some(Self::WpMachinegun),
            3 => Some(Self::WpShotgun),
            4 => Some(Self::WpGrenadeLauncher),
            5 => Some(Self::WpRocketLauncher),
            6 => Some(Self::WpLightning),
            7 => Some(Self::WpRailgun),
            8 => Some(Self::WpPlasmagun),
            9 => Some(Self::WpBfg),
            10 => Some(Self::WpGrapplingHook),
            11 => Some(Self::WpNailgun),
            12 => Some(Self::WpProxLauncher),
            13 => Some(Self::WpChaingun),
            _ => None,
        }
    }
}

/// Entity event (`EntityEvent`, re-exported from movement constants).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum EntityEvent {
    /// None.
    EvNone = 0,
    /// Footstep.
    EvFootstep = 1,
    /// Metal footstep.
    EvFootstepMetal = 2,
    /// Splash footstep.
    EvFootsplash = 3,
    /// Wade footstep.
    EvFootwade = 4,
    /// Swim.
    EvSwim = 5,
    /// 4-unit step.
    EvStep4 = 6,
    /// 8-unit step.
    EvStep8 = 7,
    /// 12-unit step.
    EvStep12 = 8,
    /// 16-unit step.
    EvStep16 = 9,
    /// Short fall.
    EvFallShort = 10,
    /// Medium fall.
    EvFallMedium = 11,
    /// Far fall.
    EvFallFar = 12,
    /// Jump pad.
    EvJumpPad = 13,
    /// Jump.
    EvJump = 14,
    /// Water touch.
    EvWaterTouch = 15,
    /// Water leave.
    EvWaterLeave = 16,
    /// Water under.
    EvWaterUnder = 17,
    /// Water clear.
    EvWaterClear = 18,
    /// Item pickup.
    EvItemPickup = 19,
    /// Global item pickup.
    EvGlobalItemPickup = 20,
    /// No ammunition.
    EvNoAmmo = 21,
    /// Change weapon.
    EvChangeWeapon = 22,
    /// Fire weapon.
    EvFireWeapon = 23,
    /// Use item 0.
    EvUseItem0 = 24,
    /// Use item 1.
    EvUseItem1 = 25,
    /// Use item 2.
    EvUseItem2 = 26,
    /// Use item 3.
    EvUseItem3 = 27,
    /// Use item 4.
    EvUseItem4 = 28,
    /// Use item 5.
    EvUseItem5 = 29,
    /// Use item 6.
    EvUseItem6 = 30,
    /// Use item 7.
    EvUseItem7 = 31,
    /// Use item 8.
    EvUseItem8 = 32,
    /// Use item 9.
    EvUseItem9 = 33,
    /// Use item 10.
    EvUseItem10 = 34,
    /// Use item 11.
    EvUseItem11 = 35,
    /// Use item 12.
    EvUseItem12 = 36,
    /// Use item 13.
    EvUseItem13 = 37,
    /// Use item 14.
    EvUseItem14 = 38,
    /// Use item 15.
    EvUseItem15 = 39,
    /// Item respawn.
    EvItemRespawn = 40,
    /// Item pop.
    EvItemPop = 41,
    /// Player teleport in.
    EvPlayerTeleportIn = 42,
    /// Player teleport out.
    EvPlayerTeleportOut = 43,
    /// Grenade bounce.
    EvGrenadeBounce = 44,
    /// General sound.
    EvGeneralSound = 45,
    /// Global sound.
    EvGlobalSound = 46,
    /// Global team sound.
    EvGlobalTeamSound = 47,
    /// Bullet hit flesh.
    EvBulletHitFlesh = 48,
    /// Bullet hit wall.
    EvBulletHitWall = 49,
    /// Missile hit.
    EvMissileHit = 50,
    /// Missile miss.
    EvMissileMiss = 51,
    /// Metal missile miss.
    EvMissileMissMetal = 52,
    /// Rail trail.
    EvRailtrail = 53,
    /// Shotgun.
    EvShotgun = 54,
    /// Bullet.
    EvBullet = 55,
    /// Pain.
    EvPain = 56,
    /// Death 1.
    EvDeath1 = 57,
    /// Death 2.
    EvDeath2 = 58,
    /// Death 3.
    EvDeath3 = 59,
    /// Obituary.
    EvObituary = 60,
    /// Quad powerup.
    EvPowerupQuad = 61,
    /// Battlesuit powerup.
    EvPowerupBattlesuit = 62,
    /// Regen powerup.
    EvPowerupRegen = 63,
    /// Gib player.
    EvGibPlayer = 64,
    /// Score plume.
    EvScoreplum = 65,
    /// Proximity mine stick.
    EvProximityMineStick = 66,
    /// Proximity mine trigger.
    EvProximityMineTrigger = 67,
    /// Kamikaze.
    EvKamikaze = 68,
    /// Obelisk explode.
    EvObeliskExplode = 69,
    /// Obelisk pain.
    EvObeliskPain = 70,
    /// Invulnerability impact.
    EvInvulImpact = 71,
    /// Juiced.
    EvJuiced = 72,
    /// Lightning bolt.
    EvLightningBolt = 73,
    /// Debug line.
    EvDebugLine = 74,
    /// Stop looping sound.
    EvStopLoopingSound = 75,
    /// Taunt.
    EvTaunt = 76,
    /// Affirmative taunt.
    EvTauntYes = 77,
    /// Negative taunt.
    EvTauntNo = 78,
    /// Follow-me taunt.
    EvTauntFollowMe = 79,
    /// Get-flag taunt.
    EvTauntGetFlag = 80,
    /// Guard-base taunt.
    EvTauntGuardBase = 81,
    /// Patrol taunt.
    EvTauntPatrol = 82,
}

/// Weapon count for a product (`weaponCount`).
#[must_use]
pub fn weapon_count(product: Product) -> i32 {
    match product {
        Product::BaseQ3 => 11,
        Product::Missionpack => 14,
    }
}

/// Whether a weapon belongs to a product (`weaponAvailable`).
#[must_use]
pub fn weapon_available(product: Product, weapon: Weapon) -> bool {
    (weapon as i32) > (Weapon::WpNone as i32) && (weapon as i32) < weapon_count(product)
}

// ---------------------------------------------------------------------------
// shared/trajectory.ts
// ---------------------------------------------------------------------------

/// Trajectory type (`TrajectoryType`, `trType_t`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum TrajectoryType {
    /// Stationary.
    TrStationary = 0,
    /// Interpolated.
    TrInterpolate = 1,
    /// Linear.
    TrLinear = 2,
    /// Linear with stop.
    TrLinearStop = 3,
    /// Sine.
    TrSine = 4,
    /// Gravity.
    TrGravity = 5,
}

impl TrajectoryType {
    /// Convert a raw integer tag, if it names a trajectory type.
    ///
    /// Unknown tags are rejected here; the donor reports them from
    /// `BG_EvaluateTrajectory`/`BG_EvaluateTrajectoryDelta` with a drop
    /// error, which [`Q3BaseError::Drop`] preserves at this boundary.
    pub fn from_i32(value: i32) -> Result<Self, Q3BaseError> {
        match value {
            0 => Ok(Self::TrStationary),
            1 => Ok(Self::TrInterpolate),
            2 => Ok(Self::TrLinear),
            3 => Ok(Self::TrLinearStop),
            4 => Ok(Self::TrSine),
            5 => Ok(Self::TrGravity),
            _ => Err(Q3BaseError::Drop(format!(
                "BG_EvaluateTrajectory: unknown trType: {value}"
            ))),
        }
    }
}

/// Trajectory record (`Trajectory`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Trajectory {
    /// Trajectory type.
    pub trajectory_type: TrajectoryType,
    /// Start time in milliseconds.
    pub time: i32,
    /// Duration in milliseconds.
    pub duration: i32,
    /// Base position.
    pub base: Vec3,
    /// Delta (velocity or amplitude).
    pub delta: Vec3,
}

impl Trajectory {
    /// Zero trajectory of one type.
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

fn trajectory_periodic_radians(tr: &Trajectory, at_time: i32) -> f32 {
    let fraction = at_time.wrapping_sub(tr.time) as f32 / (tr.duration as f32);
    (fraction * std::f32::consts::PI) * 2.0
}

/// Evaluate a trajectory at a millisecond time (`BG_EvaluateTrajectory`).
///
/// Sine phases use `f32` trigonometry, matching the C source's `sinf`.
#[must_use]
pub fn evaluate_trajectory(tr: &Trajectory, at_time: i32) -> Vec3 {
    match tr.trajectory_type {
        TrajectoryType::TrStationary | TrajectoryType::TrInterpolate => tr.base,
        TrajectoryType::TrLinear => {
            let delta_time = trajectory_seconds(at_time.wrapping_sub(tr.time));
            add3(tr.base, scale3(tr.delta, delta_time))
        }
        TrajectoryType::TrSine => {
            let phase = trajectory_periodic_radians(tr, at_time).sin();
            add3(tr.base, scale3(tr.delta, phase))
        }
        TrajectoryType::TrLinearStop => {
            let end = tr.time.wrapping_add(tr.duration);
            let time = if at_time > end { end } else { at_time };
            let delta_time = trajectory_seconds(time.wrapping_sub(tr.time)).max(0.0);
            add3(tr.base, scale3(tr.delta, delta_time))
        }
        TrajectoryType::TrGravity => {
            let delta_time = trajectory_seconds(at_time.wrapping_sub(tr.time));
            let result = add3(tr.base, scale3(tr.delta, delta_time));
            let fall = (0.5 * (DEFAULT_GRAVITY as f32) * delta_time) * delta_time;
            vec3(result.x, result.y, result.z - fall)
        }
    }
}

/// Evaluate a trajectory velocity at a millisecond time
/// (`BG_EvaluateTrajectoryDelta`).
#[must_use]
pub fn evaluate_trajectory_delta(tr: &Trajectory, at_time: i32) -> Vec3 {
    match tr.trajectory_type {
        TrajectoryType::TrStationary | TrajectoryType::TrInterpolate => vec3(0.0, 0.0, 0.0),
        TrajectoryType::TrLinear => tr.delta,
        TrajectoryType::TrSine => {
            // The source uses a half-amplitude cosine, independent of duration.
            let phase = trajectory_periodic_radians(tr, at_time).cos() * 0.5;
            scale3(tr.delta, phase)
        }
        TrajectoryType::TrLinearStop => {
            if at_time > tr.time.wrapping_add(tr.duration) {
                vec3(0.0, 0.0, 0.0)
            } else {
                tr.delta
            }
        }
        TrajectoryType::TrGravity => {
            let delta_time = trajectory_seconds(at_time.wrapping_sub(tr.time));
            vec3(
                tr.delta.x,
                tr.delta.y,
                tr.delta.z - (DEFAULT_GRAVITY as f32) * delta_time,
            )
        }
    }
}

// ---------------------------------------------------------------------------
// shared/direction-byte.ts
// ---------------------------------------------------------------------------

/// Direction table entry count (`NUM_VERTEX_NORMALS`).
pub const NUM_VERTEX_NORMALS: usize = 162;

/// Byte direction table (`bytedirs`, `q_math.c`).
pub const BYTE_DIRECTIONS: [Vec3; NUM_VERTEX_NORMALS] = [
    Vec3 {
        x: -0.525731,
        y: 0.000000,
        z: 0.850651,
    },
    Vec3 {
        x: -0.442863,
        y: 0.238856,
        z: 0.864188,
    },
    Vec3 {
        x: -0.295242,
        y: 0.000000,
        z: 0.955423,
    },
    Vec3 {
        x: -0.309017,
        y: 0.500000,
        z: 0.809017,
    },
    Vec3 {
        x: -0.162460,
        y: 0.262866,
        z: 0.951056,
    },
    Vec3 {
        x: 0.000000,
        y: 0.000000,
        z: 1.000000,
    },
    Vec3 {
        x: 0.000000,
        y: 0.850651,
        z: 0.525731,
    },
    Vec3 {
        x: -0.147621,
        y: 0.716567,
        z: 0.681718,
    },
    Vec3 {
        x: 0.147621,
        y: 0.716567,
        z: 0.681718,
    },
    Vec3 {
        x: 0.000000,
        y: 0.525731,
        z: 0.850651,
    },
    Vec3 {
        x: 0.309017,
        y: 0.500000,
        z: 0.809017,
    },
    Vec3 {
        x: 0.525731,
        y: 0.000000,
        z: 0.850651,
    },
    Vec3 {
        x: 0.295242,
        y: 0.000000,
        z: 0.955423,
    },
    Vec3 {
        x: 0.442863,
        y: 0.238856,
        z: 0.864188,
    },
    Vec3 {
        x: 0.162460,
        y: 0.262866,
        z: 0.951056,
    },
    Vec3 {
        x: -0.681718,
        y: 0.147621,
        z: 0.716567,
    },
    Vec3 {
        x: -0.809017,
        y: 0.309017,
        z: 0.500000,
    },
    Vec3 {
        x: -0.587785,
        y: 0.425325,
        z: 0.688191,
    },
    Vec3 {
        x: -0.850651,
        y: 0.525731,
        z: 0.000000,
    },
    Vec3 {
        x: -0.864188,
        y: 0.442863,
        z: 0.238856,
    },
    Vec3 {
        x: -0.716567,
        y: 0.681718,
        z: 0.147621,
    },
    Vec3 {
        x: -0.688191,
        y: 0.587785,
        z: 0.425325,
    },
    Vec3 {
        x: -0.500000,
        y: 0.809017,
        z: 0.309017,
    },
    Vec3 {
        x: -0.238856,
        y: 0.864188,
        z: 0.442863,
    },
    Vec3 {
        x: -0.425325,
        y: 0.688191,
        z: 0.587785,
    },
    Vec3 {
        x: -0.716567,
        y: 0.681718,
        z: -0.147621,
    },
    Vec3 {
        x: -0.500000,
        y: 0.809017,
        z: -0.309017,
    },
    Vec3 {
        x: -0.525731,
        y: 0.850651,
        z: 0.000000,
    },
    Vec3 {
        x: 0.000000,
        y: 0.850651,
        z: -0.525731,
    },
    Vec3 {
        x: -0.238856,
        y: 0.864188,
        z: -0.442863,
    },
    Vec3 {
        x: 0.000000,
        y: 0.955423,
        z: -0.295242,
    },
    Vec3 {
        x: -0.262866,
        y: 0.951056,
        z: -0.162460,
    },
    Vec3 {
        x: 0.000000,
        y: 1.000000,
        z: 0.000000,
    },
    Vec3 {
        x: 0.000000,
        y: 0.955423,
        z: 0.295242,
    },
    Vec3 {
        x: -0.262866,
        y: 0.951056,
        z: 0.162460,
    },
    Vec3 {
        x: 0.238856,
        y: 0.864188,
        z: 0.442863,
    },
    Vec3 {
        x: 0.262866,
        y: 0.951056,
        z: 0.162460,
    },
    Vec3 {
        x: 0.500000,
        y: 0.809017,
        z: 0.309017,
    },
    Vec3 {
        x: 0.238856,
        y: 0.864188,
        z: -0.442863,
    },
    Vec3 {
        x: 0.262866,
        y: 0.951056,
        z: -0.162460,
    },
    Vec3 {
        x: 0.500000,
        y: 0.809017,
        z: -0.309017,
    },
    Vec3 {
        x: 0.850651,
        y: 0.525731,
        z: 0.000000,
    },
    Vec3 {
        x: 0.716567,
        y: 0.681718,
        z: 0.147621,
    },
    Vec3 {
        x: 0.716567,
        y: 0.681718,
        z: -0.147621,
    },
    Vec3 {
        x: 0.525731,
        y: 0.850651,
        z: 0.000000,
    },
    Vec3 {
        x: 0.425325,
        y: 0.688191,
        z: 0.587785,
    },
    Vec3 {
        x: 0.864188,
        y: 0.442863,
        z: 0.238856,
    },
    Vec3 {
        x: 0.688191,
        y: 0.587785,
        z: 0.425325,
    },
    Vec3 {
        x: 0.809017,
        y: 0.309017,
        z: 0.500000,
    },
    Vec3 {
        x: 0.681718,
        y: 0.147621,
        z: 0.716567,
    },
    Vec3 {
        x: 0.587785,
        y: 0.425325,
        z: 0.688191,
    },
    Vec3 {
        x: 0.955423,
        y: 0.295242,
        z: 0.000000,
    },
    Vec3 {
        x: 1.000000,
        y: 0.000000,
        z: 0.000000,
    },
    Vec3 {
        x: 0.951056,
        y: 0.162460,
        z: 0.262866,
    },
    Vec3 {
        x: 0.850651,
        y: -0.525731,
        z: 0.000000,
    },
    Vec3 {
        x: 0.955423,
        y: -0.295242,
        z: 0.000000,
    },
    Vec3 {
        x: 0.864188,
        y: -0.442863,
        z: 0.238856,
    },
    Vec3 {
        x: 0.951056,
        y: -0.162460,
        z: 0.262866,
    },
    Vec3 {
        x: 0.809017,
        y: -0.309017,
        z: 0.500000,
    },
    Vec3 {
        x: 0.681718,
        y: -0.147621,
        z: 0.716567,
    },
    Vec3 {
        x: 0.850651,
        y: 0.000000,
        z: 0.525731,
    },
    Vec3 {
        x: 0.864188,
        y: 0.442863,
        z: -0.238856,
    },
    Vec3 {
        x: 0.809017,
        y: 0.309017,
        z: -0.500000,
    },
    Vec3 {
        x: 0.951056,
        y: 0.162460,
        z: -0.262866,
    },
    Vec3 {
        x: 0.525731,
        y: 0.000000,
        z: -0.850651,
    },
    Vec3 {
        x: 0.681718,
        y: 0.147621,
        z: -0.716567,
    },
    Vec3 {
        x: 0.681718,
        y: -0.147621,
        z: -0.716567,
    },
    Vec3 {
        x: 0.850651,
        y: 0.000000,
        z: -0.525731,
    },
    Vec3 {
        x: 0.809017,
        y: -0.309017,
        z: -0.500000,
    },
    Vec3 {
        x: 0.864188,
        y: -0.442863,
        z: -0.238856,
    },
    Vec3 {
        x: 0.951056,
        y: -0.162460,
        z: -0.262866,
    },
    Vec3 {
        x: 0.147621,
        y: 0.716567,
        z: -0.681718,
    },
    Vec3 {
        x: 0.309017,
        y: 0.500000,
        z: -0.809017,
    },
    Vec3 {
        x: 0.425325,
        y: 0.688191,
        z: -0.587785,
    },
    Vec3 {
        x: 0.442863,
        y: 0.238856,
        z: -0.864188,
    },
    Vec3 {
        x: 0.587785,
        y: 0.425325,
        z: -0.688191,
    },
    Vec3 {
        x: 0.688191,
        y: 0.587785,
        z: -0.425325,
    },
    Vec3 {
        x: -0.147621,
        y: 0.716567,
        z: -0.681718,
    },
    Vec3 {
        x: -0.309017,
        y: 0.500000,
        z: -0.809017,
    },
    Vec3 {
        x: 0.000000,
        y: 0.525731,
        z: -0.850651,
    },
    Vec3 {
        x: -0.525731,
        y: 0.000000,
        z: -0.850651,
    },
    Vec3 {
        x: -0.442863,
        y: 0.238856,
        z: -0.864188,
    },
    Vec3 {
        x: -0.295242,
        y: 0.000000,
        z: -0.955423,
    },
    Vec3 {
        x: -0.162460,
        y: 0.262866,
        z: -0.951056,
    },
    Vec3 {
        x: 0.000000,
        y: 0.000000,
        z: -1.000000,
    },
    Vec3 {
        x: 0.295242,
        y: 0.000000,
        z: -0.955423,
    },
    Vec3 {
        x: 0.162460,
        y: 0.262866,
        z: -0.951056,
    },
    Vec3 {
        x: -0.442863,
        y: -0.238856,
        z: -0.864188,
    },
    Vec3 {
        x: -0.309017,
        y: -0.500000,
        z: -0.809017,
    },
    Vec3 {
        x: -0.162460,
        y: -0.262866,
        z: -0.951056,
    },
    Vec3 {
        x: 0.000000,
        y: -0.850651,
        z: -0.525731,
    },
    Vec3 {
        x: -0.147621,
        y: -0.716567,
        z: -0.681718,
    },
    Vec3 {
        x: 0.147621,
        y: -0.716567,
        z: -0.681718,
    },
    Vec3 {
        x: 0.000000,
        y: -0.525731,
        z: -0.850651,
    },
    Vec3 {
        x: 0.309017,
        y: -0.500000,
        z: -0.809017,
    },
    Vec3 {
        x: 0.442863,
        y: -0.238856,
        z: -0.864188,
    },
    Vec3 {
        x: 0.162460,
        y: -0.262866,
        z: -0.951056,
    },
    Vec3 {
        x: 0.238856,
        y: -0.864188,
        z: -0.442863,
    },
    Vec3 {
        x: 0.500000,
        y: -0.809017,
        z: -0.309017,
    },
    Vec3 {
        x: 0.425325,
        y: -0.688191,
        z: -0.587785,
    },
    Vec3 {
        x: 0.716567,
        y: -0.681718,
        z: -0.147621,
    },
    Vec3 {
        x: 0.688191,
        y: -0.587785,
        z: -0.425325,
    },
    Vec3 {
        x: 0.587785,
        y: -0.425325,
        z: -0.688191,
    },
    Vec3 {
        x: 0.000000,
        y: -0.955423,
        z: -0.295242,
    },
    Vec3 {
        x: 0.000000,
        y: -1.000000,
        z: 0.000000,
    },
    Vec3 {
        x: 0.262866,
        y: -0.951056,
        z: -0.162460,
    },
    Vec3 {
        x: 0.000000,
        y: -0.850651,
        z: 0.525731,
    },
    Vec3 {
        x: 0.000000,
        y: -0.955423,
        z: 0.295242,
    },
    Vec3 {
        x: 0.238856,
        y: -0.864188,
        z: 0.442863,
    },
    Vec3 {
        x: 0.262866,
        y: -0.951056,
        z: 0.162460,
    },
    Vec3 {
        x: 0.500000,
        y: -0.809017,
        z: 0.309017,
    },
    Vec3 {
        x: 0.716567,
        y: -0.681718,
        z: 0.147621,
    },
    Vec3 {
        x: 0.525731,
        y: -0.850651,
        z: 0.000000,
    },
    Vec3 {
        x: -0.238856,
        y: -0.864188,
        z: -0.442863,
    },
    Vec3 {
        x: -0.500000,
        y: -0.809017,
        z: -0.309017,
    },
    Vec3 {
        x: -0.262866,
        y: -0.951056,
        z: -0.162460,
    },
    Vec3 {
        x: -0.850651,
        y: -0.525731,
        z: 0.000000,
    },
    Vec3 {
        x: -0.716567,
        y: -0.681718,
        z: -0.147621,
    },
    Vec3 {
        x: -0.716567,
        y: -0.681718,
        z: 0.147621,
    },
    Vec3 {
        x: -0.525731,
        y: -0.850651,
        z: 0.000000,
    },
    Vec3 {
        x: -0.500000,
        y: -0.809017,
        z: 0.309017,
    },
    Vec3 {
        x: -0.238856,
        y: -0.864188,
        z: 0.442863,
    },
    Vec3 {
        x: -0.262866,
        y: -0.951056,
        z: 0.162460,
    },
    Vec3 {
        x: -0.864188,
        y: -0.442863,
        z: 0.238856,
    },
    Vec3 {
        x: -0.809017,
        y: -0.309017,
        z: 0.500000,
    },
    Vec3 {
        x: -0.688191,
        y: -0.587785,
        z: 0.425325,
    },
    Vec3 {
        x: -0.681718,
        y: -0.147621,
        z: 0.716567,
    },
    Vec3 {
        x: -0.442863,
        y: -0.238856,
        z: 0.864188,
    },
    Vec3 {
        x: -0.587785,
        y: -0.425325,
        z: 0.688191,
    },
    Vec3 {
        x: -0.309017,
        y: -0.500000,
        z: 0.809017,
    },
    Vec3 {
        x: -0.147621,
        y: -0.716567,
        z: 0.681718,
    },
    Vec3 {
        x: -0.425325,
        y: -0.688191,
        z: 0.587785,
    },
    Vec3 {
        x: -0.162460,
        y: -0.262866,
        z: 0.951056,
    },
    Vec3 {
        x: 0.442863,
        y: -0.238856,
        z: 0.864188,
    },
    Vec3 {
        x: 0.162460,
        y: -0.262866,
        z: 0.951056,
    },
    Vec3 {
        x: 0.309017,
        y: -0.500000,
        z: 0.809017,
    },
    Vec3 {
        x: 0.147621,
        y: -0.716567,
        z: 0.681718,
    },
    Vec3 {
        x: 0.000000,
        y: -0.525731,
        z: 0.850651,
    },
    Vec3 {
        x: 0.425325,
        y: -0.688191,
        z: 0.587785,
    },
    Vec3 {
        x: 0.587785,
        y: -0.425325,
        z: 0.688191,
    },
    Vec3 {
        x: 0.688191,
        y: -0.587785,
        z: 0.425325,
    },
    Vec3 {
        x: -0.955423,
        y: 0.295242,
        z: 0.000000,
    },
    Vec3 {
        x: -0.951056,
        y: 0.162460,
        z: 0.262866,
    },
    Vec3 {
        x: -1.000000,
        y: 0.000000,
        z: 0.000000,
    },
    Vec3 {
        x: -0.850651,
        y: 0.000000,
        z: 0.525731,
    },
    Vec3 {
        x: -0.955423,
        y: -0.295242,
        z: 0.000000,
    },
    Vec3 {
        x: -0.951056,
        y: -0.162460,
        z: 0.262866,
    },
    Vec3 {
        x: -0.864188,
        y: 0.442863,
        z: -0.238856,
    },
    Vec3 {
        x: -0.951056,
        y: 0.162460,
        z: -0.262866,
    },
    Vec3 {
        x: -0.809017,
        y: 0.309017,
        z: -0.500000,
    },
    Vec3 {
        x: -0.864188,
        y: -0.442863,
        z: -0.238856,
    },
    Vec3 {
        x: -0.951056,
        y: -0.162460,
        z: -0.262866,
    },
    Vec3 {
        x: -0.809017,
        y: -0.309017,
        z: -0.500000,
    },
    Vec3 {
        x: -0.681718,
        y: 0.147621,
        z: -0.716567,
    },
    Vec3 {
        x: -0.681718,
        y: -0.147621,
        z: -0.716567,
    },
    Vec3 {
        x: -0.850651,
        y: 0.000000,
        z: -0.525731,
    },
    Vec3 {
        x: -0.688191,
        y: 0.587785,
        z: -0.425325,
    },
    Vec3 {
        x: -0.587785,
        y: 0.425325,
        z: -0.688191,
    },
    Vec3 {
        x: -0.425325,
        y: 0.688191,
        z: -0.587785,
    },
    Vec3 {
        x: -0.425325,
        y: -0.688191,
        z: -0.587785,
    },
    Vec3 {
        x: -0.587785,
        y: -0.425325,
        z: -0.688191,
    },
    Vec3 {
        x: -0.688191,
        y: -0.587785,
        z: -0.425325,
    },
];

/// Zero direction returned for out-of-table bytes.
pub const ZERO_DIRECTION: Vec3 = Vec3 { x: 0.0, y: 0.0, z: 0.0 };

/// Compress a direction to a table index (`DirToByte`).
///
/// A missing direction compresses to index zero, matching the source.
#[must_use]
pub fn direction_to_byte(value: Option<Vec3>) -> usize {
    let Some(value) = value else {
        return 0;
    };
    let mut best_dot = 0.0f32;
    let mut best = 0;
    for (index, candidate) in BYTE_DIRECTIONS.iter().enumerate() {
        let dot = dot3(value, *candidate);
        if dot > best_dot {
            best_dot = dot;
            best = index;
        }
    }
    best
}

/// Expand a table byte to a direction (`ByteToDir`).
///
/// Out-of-table bytes expand to the zero direction.
#[must_use]
pub fn byte_to_direction(byte: i32) -> Vec3 {
    if byte < 0 {
        return ZERO_DIRECTION;
    }
    BYTE_DIRECTIONS.get(byte as usize).copied().unwrap_or(ZERO_DIRECTION)
}

// ---------------------------------------------------------------------------
// shared/entity-state.ts (via network/q3/state/entity.ts)
// ---------------------------------------------------------------------------

/// Owned `entityState_t` storage (`EntityState`).
///
/// Source enum storage retains raw integer tags; creation follows
/// memset-zero with stationary trajectories.
#[derive(Debug, Clone, PartialEq)]
pub struct EntityState {
    /// Entity number.
    pub number: i32,
    /// Entity type tag.
    pub e_type: i32,
    /// Entity flags.
    pub e_flags: i32,
    /// Position trajectory.
    pub pos: Trajectory,
    /// Angle trajectory.
    pub apos: Trajectory,
    /// Time.
    pub time: i32,
    /// Secondary time.
    pub time2: i32,
    /// Origin.
    pub origin: Vec3,
    /// Secondary origin.
    pub origin2: Vec3,
    /// Angles.
    pub angles: Vec3,
    /// Secondary angles.
    pub angles2: Vec3,
    /// Other entity number.
    pub other_entity_num: i32,
    /// Second other entity number.
    pub other_entity_num2: i32,
    /// Ground entity number.
    pub ground_entity_num: i32,
    /// Constant light.
    pub constant_light: i32,
    /// Loop sound.
    pub loop_sound: i32,
    /// Model index.
    pub modelindex: i32,
    /// Second model index.
    pub modelindex2: i32,
    /// Client number.
    pub client_num: i32,
    /// Frame.
    pub frame: i32,
    /// Solid encoding.
    pub solid: i32,
    /// Event.
    pub event: i32,
    /// Event parameter.
    pub event_parm: i32,
    /// Powerup bits.
    pub powerups: i32,
    /// Weapon tag.
    pub weapon: i32,
    /// Legs animation.
    pub legs_anim: i32,
    /// Torso animation.
    pub torso_anim: i32,
    /// Generic value.
    pub generic1: i32,
}

impl Default for EntityState {
    fn default() -> Self {
        Self::new()
    }
}

impl EntityState {
    /// Zero entity state with stationary trajectories.
    #[must_use]
    pub fn new() -> Self {
        let zero = vec3(0.0, 0.0, 0.0);
        Self {
            number: 0,
            e_type: 0,
            e_flags: 0,
            pos: Trajectory::zero(TrajectoryType::TrStationary),
            apos: Trajectory::zero(TrajectoryType::TrStationary),
            time: 0,
            time2: 0,
            origin: zero,
            origin2: zero,
            angles: zero,
            angles2: zero,
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

    /// Deep copy.
    #[must_use]
    pub fn copy(&self) -> Self {
        self.clone()
    }

    /// Copy every field from a source state.
    pub fn copy_from_state(&mut self, source: &EntityState) {
        copy_entity_state_fields(self, source);
    }
}

/// Copy every entity state field (`copyEntityStateFields`).
pub fn copy_entity_state_fields(target: &mut EntityState, source: &EntityState) {
    *target = source.clone();
}

// ---------------------------------------------------------------------------
// shared/player-state.ts (+ movement/q3/constants.ts re-exports)
// ---------------------------------------------------------------------------

/// World entity number (`ENTITYNUM_WORLD`).
pub const ENTITYNUM_WORLD: i32 = 1022;
/// No-entity number (`ENTITYNUM_NONE`).
pub const ENTITYNUM_NONE: i32 = 1023;

/// Player movement flags (`MoveFlags`, re-exported from movement constants).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum MoveFlags {
    /// Ducked.
    Ducked = 1,
    /// Jump held.
    JumpHeld = 2,
    /// Backwards jump.
    BackwardsJump = 8,
    /// Backwards run.
    BackwardsRun = 16,
    /// Landing timer active.
    TimeLand = 32,
    /// Knockback timer active.
    TimeKnockback = 64,
    /// Water-jump timer active.
    TimeWaterJump = 256,
    /// Respawned.
    Respawned = 512,
    /// Use-item held.
    UseItemHeld = 1024,
    /// Grapple pull.
    GrapplePull = 2048,
    /// Following.
    Follow = 4096,
    /// Scoreboard.
    Scoreboard = 8192,
    /// Invulnerability expansion.
    InvulExpand = 16384,
}

/// User command buttons (`CommandButtons`, re-exported from movement
/// constants).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum CommandButtons {
    /// Attack.
    Attack = 1,
    /// Talk.
    Talk = 2,
    /// Use holdable.
    UseHoldable = 4,
    /// Gesture.
    Gesture = 8,
    /// Walking.
    Walking = 16,
    /// Affirmative.
    Affirmative = 32,
    /// Negative.
    Negative = 64,
    /// Get flag.
    GetFlag = 128,
    /// Guard base.
    GuardBase = 256,
    /// Patrol.
    Patrol = 512,
    /// Follow me.
    FollowMe = 1024,
    /// Any.
    Any = 2048,
}

/// Player animation slots (`PlayerAnimation`, re-exported from movement
/// constants).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum PlayerAnimation {
    /// First death.
    BothDeath1 = 0,
    /// First dead.
    BothDead1 = 1,
    /// Second death.
    BothDeath2 = 2,
    /// Second dead.
    BothDead2 = 3,
    /// Third death.
    BothDeath3 = 4,
    /// Third dead.
    BothDead3 = 5,
    /// Torso gesture.
    TorsoGesture = 6,
    /// Torso attack.
    TorsoAttack = 7,
    /// Torso attack 2.
    TorsoAttack2 = 8,
    /// Torso drop.
    TorsoDrop = 9,
    /// Torso raise.
    TorsoRaise = 10,
    /// Torso stand.
    TorsoStand = 11,
    /// Torso stand 2.
    TorsoStand2 = 12,
    /// Crouched legs walk.
    LegsWalkCr = 13,
    /// Legs walk.
    LegsWalk = 14,
    /// Legs run.
    LegsRun = 15,
    /// Legs backpedal.
    LegsBack = 16,
    /// Legs swim.
    LegsSwim = 17,
    /// Legs jump.
    LegsJump = 18,
    /// Legs land.
    LegsLand = 19,
    /// Backwards legs jump.
    LegsJumpB = 20,
    /// Backwards legs land.
    LegsLandB = 21,
    /// Legs idle.
    LegsIdle = 22,
    /// Crouched legs idle.
    LegsIdleCr = 23,
    /// Legs turn.
    LegsTurn = 24,
    /// Torso get flag.
    TorsoGetFlag = 25,
    /// Torso guard base.
    TorsoGuardBase = 26,
    /// Torso patrol.
    TorsoPatrol = 27,
    /// Torso follow me.
    TorsoFollowMe = 28,
    /// Torso affirmative.
    TorsoAffirmative = 29,
    /// Torso negative.
    TorsoNegative = 30,
    /// Crouched backpedal.
    LegsBackCr = 32,
    /// Backwards legs walk.
    LegsBackWalk = 33,
    /// Flag run.
    FlagRun = 34,
    /// Flag stand.
    FlagStand = 35,
    /// Flag stand-to-run.
    FlagStand2Run = 36,
}

/// User command (`UserCommand`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UserCommand {
    /// Server time.
    pub server_time: i32,
    /// Command angles.
    pub angles: Vec3,
    /// Buttons.
    pub buttons: i32,
    /// Selected weapon tag.
    pub weapon: i32,
    /// Forward movement.
    pub forwardmove: i32,
    /// Right movement.
    pub rightmove: i32,
    /// Up movement.
    pub upmove: i32,
}

/// Fixed source slot storage owned elsewhere (`PlayerSlotBinding`).
pub trait PlayerSlotBinding {
    /// Read a slot.
    fn read(&self, index: usize) -> i32;
    /// Write a slot.
    fn write(&self, index: usize, value: i32);
}

/// Fixed source arrays with checked access, including unused protocol slots
/// (`PlayerStateSlots`).
#[derive(Clone)]
pub struct PlayerStateSlots {
    values: Vec<i32>,
    binding: Option<Rc<dyn PlayerSlotBinding>>,
    stored: Option<Rc<dyn Fn(usize, i32)>>,
}

impl std::fmt::Debug for PlayerStateSlots {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlayerStateSlots")
            .field("values", &self.values)
            .field("bound", &self.binding.is_some())
            .finish()
    }
}

impl PlayerStateSlots {
    /// Fixed slots, optionally over source values, an owner binding, and a
    /// store notification.
    ///
    /// # Panics
    ///
    /// Panics when `source_values` does not hold exactly `length` values.
    #[must_use]
    pub fn new(
        length: usize,
        source_values: Option<Vec<i32>>,
        binding: Option<Rc<dyn PlayerSlotBinding>>,
        stored: Option<Rc<dyn Fn(usize, i32)>>,
    ) -> Self {
        if let Some(values) = source_values {
            assert!(
                values.len() == length,
                "Player state source slots require {length} values, got {}",
                values.len()
            );
            Self {
                values,
                binding,
                stored,
            }
        } else {
            Self {
                values: vec![0; length],
                binding,
                stored,
            }
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
    ///
    /// # Panics
    ///
    /// Panics when `index` is outside the slots.
    #[must_use]
    pub fn get(&self, index: usize) -> i32 {
        if let Some(binding) = &self.binding {
            if index < self.values.len() {
                return binding.read(index);
            }
            panic!("Player state slot {index} outside {}", self.values.len());
        }
        *self
            .values
            .get(index)
            .unwrap_or_else(|| panic!("Player state slot {index} outside {}", self.values.len()))
    }

    /// Write a slot, then report the stored value.
    ///
    /// # Panics
    ///
    /// Panics when `index` is outside the slots.
    pub fn set(&mut self, index: usize, value: i32) {
        if index >= self.values.len() {
            panic!("Player state slot {index} outside {}", self.values.len());
        }
        if let Some(binding) = &self.binding {
            binding.write(index, value);
        } else {
            self.values[index] = value;
        }
        if let Some(stored) = &self.stored {
            let current = self.get(index);
            stored(index, current);
        }
    }

    /// Copy every slot value.
    #[must_use]
    pub fn copy(&self) -> Vec<i32> {
        (0..self.values.len()).map(|index| self.get(index)).collect()
    }
}

fn copy_slots(target: &mut PlayerStateSlots, source: &PlayerStateSlots) {
    for index in 0..target.len() {
        target.set(index, source.get(index));
    }
}

/// Predictable event record (`PredictableEvent`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PredictableEvent {
    /// Event sequence at capture.
    pub sequence: i32,
    /// Event tag.
    pub event: i32,
    /// Event parameter.
    pub parameter: i32,
}

/// Event-debug module (`PredictableEventDebug` module word).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventDebugModule {
    /// Game module.
    Game,
    /// Client-game module.
    Cgame,
}

/// Source event-debug sink (`PredictableEventDebug`).
pub trait PredictableEventDebug {
    /// Owning module.
    fn module(&self) -> EventDebugModule;
    /// `showevents` cvar text.
    fn show_events(&self) -> String;
    /// Print a debug line.
    fn print(&self, message: &str);
}

// `bg_misc.c:eventnames`, including its omitted `EV_OBELISKPAIN` entry.
const EVENT_NAMES: [&str; 76] = [
    "EV_NONE",
    "EV_FOOTSTEP",
    "EV_FOOTSTEP_METAL",
    "EV_FOOTSPLASH",
    "EV_FOOTWADE",
    "EV_SWIM",
    "EV_STEP_4",
    "EV_STEP_8",
    "EV_STEP_12",
    "EV_STEP_16",
    "EV_FALL_SHORT",
    "EV_FALL_MEDIUM",
    "EV_FALL_FAR",
    "EV_JUMP_PAD",
    "EV_JUMP",
    "EV_WATER_TOUCH",
    "EV_WATER_LEAVE",
    "EV_WATER_UNDER",
    "EV_WATER_CLEAR",
    "EV_ITEM_PICKUP",
    "EV_GLOBAL_ITEM_PICKUP",
    "EV_NOAMMO",
    "EV_CHANGE_WEAPON",
    "EV_FIRE_WEAPON",
    "EV_USE_ITEM0",
    "EV_USE_ITEM1",
    "EV_USE_ITEM2",
    "EV_USE_ITEM3",
    "EV_USE_ITEM4",
    "EV_USE_ITEM5",
    "EV_USE_ITEM6",
    "EV_USE_ITEM7",
    "EV_USE_ITEM8",
    "EV_USE_ITEM9",
    "EV_USE_ITEM10",
    "EV_USE_ITEM11",
    "EV_USE_ITEM12",
    "EV_USE_ITEM13",
    "EV_USE_ITEM14",
    "EV_USE_ITEM15",
    "EV_ITEM_RESPAWN",
    "EV_ITEM_POP",
    "EV_PLAYER_TELEPORT_IN",
    "EV_PLAYER_TELEPORT_OUT",
    "EV_GRENADE_BOUNCE",
    "EV_GENERAL_SOUND",
    "EV_GLOBAL_SOUND",
    "EV_GLOBAL_TEAM_SOUND",
    "EV_BULLET_HIT_FLESH",
    "EV_BULLET_HIT_WALL",
    "EV_MISSILE_HIT",
    "EV_MISSILE_MISS",
    "EV_MISSILE_MISS_METAL",
    "EV_RAILTRAIL",
    "EV_SHOTGUN",
    "EV_BULLET",
    "EV_PAIN",
    "EV_DEATH1",
    "EV_DEATH2",
    "EV_DEATH3",
    "EV_OBITUARY",
    "EV_POWERUP_QUAD",
    "EV_POWERUP_BATTLESUIT",
    "EV_POWERUP_REGEN",
    "EV_GIB_PLAYER",
    "EV_SCOREPLUM",
    "EV_PROXIMITY_MINE_STICK",
    "EV_PROXIMITY_MINE_TRIGGER",
    "EV_KAMIKAZE",
    "EV_OBELISKEXPLODE",
    "EV_INVUL_IMPACT",
    "EV_JUICED",
    "EV_LIGHTNINGBOLT",
    "EV_DEBUG_LINE",
    "EV_STOPLOOPINGSOUND",
    "EV_TAUNT",
];

/// Owned `playerState_t` storage binding (`PlayerAuthorityBinding`).
///
/// The engine transports game-defined integer words unchanged.
pub trait PlayerAuthorityBinding {
    /// Read the authoritative origin.
    fn origin(&self) -> Vec3;
    /// Write the authoritative origin.
    fn set_origin(&self, value: Vec3);
    /// Read the authoritative velocity.
    fn read_velocity(&self) -> Vec3;
    /// Write the authoritative velocity.
    fn set_velocity(&self, value: Vec3);
    /// Stat slot binding.
    fn stats(&self) -> Rc<dyn PlayerSlotBinding>;
    /// Ammunition slot binding.
    fn ammo(&self) -> Rc<dyn PlayerSlotBinding>;
}

/// Authority handling for [`PlayerState::copy_from`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorityStores {
    /// Keep authority-owned words (origin, velocity, health/armor/weapon
    /// stats, ammunition).
    PreserveAuthority,
    /// Copy authority-owned words as well.
    ReplaceAuthority,
}

/// Owned `playerState_t` storage (`PlayerStateRecord`).
///
/// Movement, weapon, and weapon-state tags are raw `i32` words, matching
/// the donor's `SourcePlayerState` layer used by game logic; the typed
/// `PlayerState<MoveType, Weapon, WeaponState>` generic is TypeScript-only
/// machinery over the same storage.
#[derive(Clone)]
pub struct PlayerState {
    product: Product,
    authority: Option<Rc<dyn PlayerAuthorityBinding>>,
    event_debug: Option<Rc<dyn PredictableEventDebug>>,
    origin_value: Vec3,
    velocity_value: Vec3,
    /// Command time.
    pub command_time: i32,
    /// Movement type tag.
    pub pm_type: i32,
    /// Bob cycle.
    pub bob_cycle: i32,
    /// Movement flags.
    pub pm_flags: i32,
    /// Movement timer.
    pub pm_time: i32,
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
    /// Entity flags.
    pub e_flags: i32,
    /// Event sequence.
    pub event_sequence: i32,
    /// Predictable events.
    pub events: PlayerStateSlots,
    /// Predictable event parameters.
    pub event_parms: PlayerStateSlots,
    /// External event.
    pub external_event: i32,
    /// External event parameter.
    pub external_event_parm: i32,
    /// External event time.
    pub external_event_time: i32,
    /// Client number.
    pub client_num: i32,
    /// Weapon tag.
    pub weapon: i32,
    /// Weapon state tag.
    pub weapon_state: i32,
    /// View angles.
    pub viewangles: Vec3,
    /// View height.
    pub viewheight: i32,
    /// Damage event.
    pub damage_event: i32,
    /// Damage yaw.
    pub damage_yaw: i32,
    /// Damage pitch.
    pub damage_pitch: i32,
    /// Damage count.
    pub damage_count: i32,
    /// Stats.
    pub stats: PlayerStateSlots,
    /// Persistant stats.
    pub persistant: PlayerStateSlots,
    /// Powerups.
    pub powerups: PlayerStateSlots,
    /// Ammunition.
    pub ammo: PlayerStateSlots,
    /// Generic value.
    pub generic1: i32,
    /// Loop sound.
    pub loop_sound: i32,
    /// Jump-pad entity.
    pub jumppad_ent: i32,
    /// Ping.
    pub ping: i32,
    /// Player-move frame count.
    pub pmove_framecount: i32,
    /// Jump-pad frame.
    pub jumppad_frame: i32,
    /// Consumed entity event sequence.
    pub entity_event_sequence: i32,
}

impl std::fmt::Debug for PlayerState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlayerState")
            .field("product", &self.product)
            .field("command_time", &self.command_time)
            .field("pm_type", &self.pm_type)
            .field("event_sequence", &self.event_sequence)
            .field("client_num", &self.client_num)
            .field("weapon", &self.weapon)
            .finish()
    }
}

/// Source player state layer (`SourcePlayerState`).
pub type SourcePlayerState = PlayerState;

impl PlayerState {
    /// Zero player state with normal movement, no weapon, and a ready
    /// weapon (`PlayerState` construction).
    #[must_use]
    pub fn new(product: Product, authority: Option<Rc<dyn PlayerAuthorityBinding>>) -> Self {
        let stats = PlayerStateSlots::new(16, None, authority.as_ref().map(|binding| binding.stats()), None);
        let ammo = PlayerStateSlots::new(16, None, authority.as_ref().map(|binding| binding.ammo()), None);
        let zero = vec3(0.0, 0.0, 0.0);
        Self {
            product,
            authority,
            event_debug: None,
            origin_value: zero,
            velocity_value: zero,
            command_time: 0,
            pm_type: MoveType::PmNormal as i32,
            bob_cycle: 0,
            pm_flags: 0,
            pm_time: 0,
            weapon_time: 0,
            gravity: 0,
            speed: 0,
            delta_angles: zero,
            ground_entity_num: 0,
            legs_timer: 0,
            legs_anim: 0,
            torso_timer: 0,
            torso_anim: 0,
            movement_dir: 0,
            grapple_point: zero,
            e_flags: 0,
            event_sequence: 0,
            events: PlayerStateSlots::new(2, None, None, None),
            event_parms: PlayerStateSlots::new(2, None, None, None),
            external_event: 0,
            external_event_parm: 0,
            external_event_time: 0,
            client_num: 0,
            weapon: Weapon::WpNone as i32,
            weapon_state: WeaponState::WeaponReady as i32,
            viewangles: zero,
            viewheight: 0,
            damage_event: 0,
            damage_yaw: 0,
            damage_pitch: 0,
            damage_count: 0,
            stats,
            persistant: PlayerStateSlots::new(16, None, None, None),
            powerups: PlayerStateSlots::new(16, None, None, None),
            ammo,
            generic1: 0,
            loop_sound: 0,
            jumppad_ent: 0,
            ping: 0,
            pmove_framecount: 0,
            jumppad_frame: 0,
            entity_event_sequence: 0,
        }
    }

    /// Owning product.
    #[must_use]
    pub fn product(&self) -> Product {
        self.product
    }

    /// Origin, from the authority when bound.
    #[must_use]
    pub fn origin(&self) -> Vec3 {
        self.authority
            .as_ref()
            .map_or(self.origin_value, |binding| binding.origin())
    }

    /// Write the origin, through the authority when bound.
    pub fn set_origin(&mut self, value: Vec3) {
        if let Some(binding) = &self.authority {
            binding.set_origin(value);
        } else {
            self.origin_value = value;
        }
    }

    /// Velocity, from the authority when bound.
    #[must_use]
    pub fn velocity(&self) -> Vec3 {
        self.authority
            .as_ref()
            .map_or(self.velocity_value, |binding| binding.read_velocity())
    }

    /// Write the velocity, through the authority when bound.
    pub fn set_velocity(&mut self, value: Vec3) {
        if let Some(binding) = &self.authority {
            binding.set_velocity(value);
        } else {
            self.velocity_value = value;
        }
    }

    /// Health from the product's health stat slot.
    #[must_use]
    pub fn health(&self) -> i32 {
        self.stats.get(stat_schema(self.product).indices().health)
    }

    /// Write health to the product's health stat slot.
    pub fn set_health(&mut self, value: i32) {
        let slot = stat_schema(self.product).indices().health;
        self.stats.set(slot, value);
    }

    /// Install or clear event debugging.
    pub fn set_event_debug(&mut self, debug: Option<Rc<dyn PredictableEventDebug>>) {
        self.event_debug = debug;
    }

    /// Deep copy without authority.
    #[must_use]
    pub fn copy(&self) -> Self {
        let mut result = Self::new(self.product, None);
        result.copy_from(self, AuthorityStores::ReplaceAuthority);
        result
    }

    /// Copy every field from a source state.
    pub fn copy_from(&mut self, source: &PlayerState, stores: AuthorityStores) {
        let preserve = self.authority.is_some() && stores == AuthorityStores::PreserveAuthority;
        self.product = source.product;
        self.command_time = source.command_time;
        self.pm_type = source.pm_type;
        self.bob_cycle = source.bob_cycle;
        self.pm_flags = source.pm_flags;
        self.pm_time = source.pm_time;
        if !preserve {
            self.set_origin(source.origin());
            self.set_velocity(source.velocity());
        }
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
        copy_slots(&mut self.events, &source.events);
        copy_slots(&mut self.event_parms, &source.event_parms);
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
        let schema = stat_schema(self.product);
        let indices = *schema.indices();
        for index in 0..self.stats.len() {
            if preserve && (index == indices.health || index == indices.armor || index == indices.weapons) {
                continue;
            }
            self.stats.set(index, source.stats.get(index));
        }
        copy_slots(&mut self.persistant, &source.persistant);
        copy_slots(&mut self.powerups, &source.powerups);
        if !preserve {
            copy_slots(&mut self.ammo, &source.ammo);
        }
        self.generic1 = source.generic1;
        self.loop_sound = source.loop_sound;
        self.jumppad_ent = source.jumppad_ent;
        self.ping = source.ping;
        self.pmove_framecount = source.pmove_framecount;
        self.jumppad_frame = source.jumppad_frame;
        self.entity_event_sequence = source.entity_event_sequence;
    }

    /// Queue a predictable event (`BG_AddPredictableEventToPlayerstate`).
    ///
    /// # Panics
    ///
    /// Panics when event debugging is enabled and `event` has no
    /// `bg_misc.c` event name.
    pub fn add_event(&mut self, event: i32, parameter: i32) -> PredictableEvent {
        if let Some(debug) = &self.event_debug {
            let text: String = debug.show_events().chars().take(255).collect();
            if native_atof(&text).is_ok_and(|value| value != 0.0) {
                let name = usize::try_from(event)
                    .ok()
                    .and_then(|index| EVENT_NAMES.get(index))
                    .unwrap_or_else(|| panic!("bg_misc.c eventnames has no entry for {event}"));
                let label = match debug.module() {
                    EventDebugModule::Game => " game",
                    EventDebugModule::Cgame => "Cgame",
                };
                debug.print(&format!(
                    "{label} event svt {:>5} -> {:>5}: num = {name:>20} parm {parameter}\n",
                    self.pmove_framecount, self.event_sequence
                ));
            }
        }
        let sequence = self.event_sequence;
        let slot = (sequence & 1) as usize;
        self.events.set(slot, event);
        self.event_parms.set(slot, parameter);
        self.event_sequence = sequence.wrapping_add(1);
        PredictableEvent {
            sequence,
            event,
            parameter,
        }
    }
}

/// Create a retail zero player state (`createPlayerState`).
#[must_use]
pub fn create_player_state(product: Product, authority: Option<Rc<dyn PlayerAuthorityBinding>>) -> PlayerState {
    PlayerState::new(product, authority)
}

// ---------------------------------------------------------------------------
// shared/items.ts
// ---------------------------------------------------------------------------

/// Item type plus tag word (`ItemDefinition` type/tag pair).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKindTag {
    /// Weapon with its weapon tag.
    Weapon(Weapon),
    /// Ammunition with its weapon tag.
    Ammo(Weapon),
    /// Powerup with its powerup tag.
    Powerup(Powerup),
    /// Persistant powerup with its powerup tag.
    PersistantPowerup(Powerup),
    /// Team item with its powerup tag.
    Team(Powerup),
    /// Holdable with its holdable tag.
    Holdable(Holdable),
    /// Reserved empty item.
    Bad,
    /// Armor.
    Armor,
    /// Health.
    Health,
}

/// Item definition (`ItemDefinition`, `bg_itemlist`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemDefinition {
    /// Class name.
    pub class_name: Option<&'static str>,
    /// Pickup sound.
    pub pickup_sound: Option<&'static str>,
    /// World models.
    pub world_models: [Option<&'static str>; 4],
    /// Icon.
    pub icon: Option<&'static str>,
    /// Pickup name.
    pub pickup_name: Option<&'static str>,
    /// Quantity.
    pub quantity: i32,
    /// Precaches.
    pub precaches: &'static str,
    /// Sounds.
    pub sounds: &'static str,
    /// Type plus tag.
    pub kind: ItemKindTag,
}

impl ItemDefinition {
    /// Item type.
    #[must_use]
    pub fn item_type(&self) -> ItemType {
        match self.kind {
            ItemKindTag::Weapon(_) => ItemType::ItWeapon,
            ItemKindTag::Ammo(_) => ItemType::ItAmmo,
            ItemKindTag::Powerup(_) => ItemType::ItPowerup,
            ItemKindTag::PersistantPowerup(_) => ItemType::ItPersistantPowerup,
            ItemKindTag::Team(_) => ItemType::ItTeam,
            ItemKindTag::Holdable(_) => ItemType::ItHoldable,
            ItemKindTag::Bad => ItemType::ItBad,
            ItemKindTag::Armor => ItemType::ItArmor,
            ItemKindTag::Health => ItemType::ItHealth,
        }
    }

    /// Weapon tag for weapon and ammunition items.
    #[must_use]
    pub fn weapon_tag(&self) -> Option<Weapon> {
        match self.kind {
            ItemKindTag::Weapon(tag) | ItemKindTag::Ammo(tag) => Some(tag),
            _ => None,
        }
    }

    /// Powerup tag for powerup, persistant-powerup, and team items.
    #[must_use]
    pub fn powerup_tag(&self) -> Option<Powerup> {
        match self.kind {
            ItemKindTag::Powerup(tag) | ItemKindTag::PersistantPowerup(tag) | ItemKindTag::Team(tag) => Some(tag),
            _ => None,
        }
    }

    /// Holdable tag for holdable items.
    #[must_use]
    pub fn holdable_tag(&self) -> Option<Holdable> {
        match self.kind {
            ItemKindTag::Holdable(tag) => Some(tag),
            _ => None,
        }
    }
}

// Index zero is the source's reserved empty item. The terminal C marker is
// excluded.
static ITEM_DEFINITIONS: [ItemDefinition; 52] = [
    ItemDefinition {
        class_name: None,
        pickup_sound: None,
        world_models: [None, None, None, None],
        icon: None,
        pickup_name: None,
        quantity: 0,
        kind: ItemKindTag::Bad,
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("item_armor_shard"),
        pickup_sound: Some("sound/misc/ar1_pkup.wav"),
        world_models: [
            Some("models/powerups/armor/shard.md3"),
            Some("models/powerups/armor/shard_sphere.md3"),
            None,
            None,
        ],
        icon: Some("icons/iconr_shard"),
        pickup_name: Some("Armor Shard"),
        quantity: 5,
        kind: ItemKindTag::Armor,
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("item_armor_combat"),
        pickup_sound: Some("sound/misc/ar2_pkup.wav"),
        world_models: [
            Some("models/powerups/armor/armor_yel.md3"),
            None,
            None,
            None,
        ],
        icon: Some("icons/iconr_yellow"),
        pickup_name: Some("Armor"),
        quantity: 50,
        kind: ItemKindTag::Armor,
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("item_armor_body"),
        pickup_sound: Some("sound/misc/ar2_pkup.wav"),
        world_models: [
            Some("models/powerups/armor/armor_red.md3"),
            None,
            None,
            None,
        ],
        icon: Some("icons/iconr_red"),
        pickup_name: Some("Heavy Armor"),
        quantity: 100,
        kind: ItemKindTag::Armor,
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("item_health_small"),
        pickup_sound: Some("sound/items/s_health.wav"),
        world_models: [
            Some("models/powerups/health/small_cross.md3"),
            Some("models/powerups/health/small_sphere.md3"),
            None,
            None,
        ],
        icon: Some("icons/iconh_green"),
        pickup_name: Some("5 Health"),
        quantity: 5,
        kind: ItemKindTag::Health,
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("item_health"),
        pickup_sound: Some("sound/items/n_health.wav"),
        world_models: [
            Some("models/powerups/health/medium_cross.md3"),
            Some("models/powerups/health/medium_sphere.md3"),
            None,
            None,
        ],
        icon: Some("icons/iconh_yellow"),
        pickup_name: Some("25 Health"),
        quantity: 25,
        kind: ItemKindTag::Health,
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("item_health_large"),
        pickup_sound: Some("sound/items/l_health.wav"),
        world_models: [
            Some("models/powerups/health/large_cross.md3"),
            Some("models/powerups/health/large_sphere.md3"),
            None,
            None,
        ],
        icon: Some("icons/iconh_red"),
        pickup_name: Some("50 Health"),
        quantity: 50,
        kind: ItemKindTag::Health,
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("item_health_mega"),
        pickup_sound: Some("sound/items/m_health.wav"),
        world_models: [
            Some("models/powerups/health/mega_cross.md3"),
            Some("models/powerups/health/mega_sphere.md3"),
            None,
            None,
        ],
        icon: Some("icons/iconh_mega"),
        pickup_name: Some("Mega Health"),
        quantity: 100,
        kind: ItemKindTag::Health,
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("weapon_gauntlet"),
        pickup_sound: Some("sound/misc/w_pkup.wav"),
        world_models: [
            Some("models/weapons2/gauntlet/gauntlet.md3"),
            None,
            None,
            None,
        ],
        icon: Some("icons/iconw_gauntlet"),
        pickup_name: Some("Gauntlet"),
        quantity: 0,
        kind: ItemKindTag::Weapon(Weapon::WpGauntlet),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("weapon_shotgun"),
        pickup_sound: Some("sound/misc/w_pkup.wav"),
        world_models: [
            Some("models/weapons2/shotgun/shotgun.md3"),
            None,
            None,
            None,
        ],
        icon: Some("icons/iconw_shotgun"),
        pickup_name: Some("Shotgun"),
        quantity: 10,
        kind: ItemKindTag::Weapon(Weapon::WpShotgun),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("weapon_machinegun"),
        pickup_sound: Some("sound/misc/w_pkup.wav"),
        world_models: [
            Some("models/weapons2/machinegun/machinegun.md3"),
            None,
            None,
            None,
        ],
        icon: Some("icons/iconw_machinegun"),
        pickup_name: Some("Machinegun"),
        quantity: 40,
        kind: ItemKindTag::Weapon(Weapon::WpMachinegun),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("weapon_grenadelauncher"),
        pickup_sound: Some("sound/misc/w_pkup.wav"),
        world_models: [
            Some("models/weapons2/grenadel/grenadel.md3"),
            None,
            None,
            None,
        ],
        icon: Some("icons/iconw_grenade"),
        pickup_name: Some("Grenade Launcher"),
        quantity: 10,
        kind: ItemKindTag::Weapon(Weapon::WpGrenadeLauncher),
        precaches: "",
        sounds: "sound/weapons/grenade/hgrenb1a.wav sound/weapons/grenade/hgrenb2a.wav",
    },
    ItemDefinition {
        class_name: Some("weapon_rocketlauncher"),
        pickup_sound: Some("sound/misc/w_pkup.wav"),
        world_models: [
            Some("models/weapons2/rocketl/rocketl.md3"),
            None,
            None,
            None,
        ],
        icon: Some("icons/iconw_rocket"),
        pickup_name: Some("Rocket Launcher"),
        quantity: 10,
        kind: ItemKindTag::Weapon(Weapon::WpRocketLauncher),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("weapon_lightning"),
        pickup_sound: Some("sound/misc/w_pkup.wav"),
        world_models: [
            Some("models/weapons2/lightning/lightning.md3"),
            None,
            None,
            None,
        ],
        icon: Some("icons/iconw_lightning"),
        pickup_name: Some("Lightning Gun"),
        quantity: 100,
        kind: ItemKindTag::Weapon(Weapon::WpLightning),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("weapon_railgun"),
        pickup_sound: Some("sound/misc/w_pkup.wav"),
        world_models: [
            Some("models/weapons2/railgun/railgun.md3"),
            None,
            None,
            None,
        ],
        icon: Some("icons/iconw_railgun"),
        pickup_name: Some("Railgun"),
        quantity: 10,
        kind: ItemKindTag::Weapon(Weapon::WpRailgun),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("weapon_plasmagun"),
        pickup_sound: Some("sound/misc/w_pkup.wav"),
        world_models: [
            Some("models/weapons2/plasma/plasma.md3"),
            None,
            None,
            None,
        ],
        icon: Some("icons/iconw_plasma"),
        pickup_name: Some("Plasma Gun"),
        quantity: 50,
        kind: ItemKindTag::Weapon(Weapon::WpPlasmagun),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("weapon_bfg"),
        pickup_sound: Some("sound/misc/w_pkup.wav"),
        world_models: [Some("models/weapons2/bfg/bfg.md3"), None, None, None],
        icon: Some("icons/iconw_bfg"),
        pickup_name: Some("BFG10K"),
        quantity: 20,
        kind: ItemKindTag::Weapon(Weapon::WpBfg),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("weapon_grapplinghook"),
        pickup_sound: Some("sound/misc/w_pkup.wav"),
        world_models: [
            Some("models/weapons2/grapple/grapple.md3"),
            None,
            None,
            None,
        ],
        icon: Some("icons/iconw_grapple"),
        pickup_name: Some("Grappling Hook"),
        quantity: 0,
        kind: ItemKindTag::Weapon(Weapon::WpGrapplingHook),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("ammo_shells"),
        pickup_sound: Some("sound/misc/am_pkup.wav"),
        world_models: [
            Some("models/powerups/ammo/shotgunam.md3"),
            None,
            None,
            None,
        ],
        icon: Some("icons/icona_shotgun"),
        pickup_name: Some("Shells"),
        quantity: 10,
        kind: ItemKindTag::Ammo(Weapon::WpShotgun),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("ammo_bullets"),
        pickup_sound: Some("sound/misc/am_pkup.wav"),
        world_models: [
            Some("models/powerups/ammo/machinegunam.md3"),
            None,
            None,
            None,
        ],
        icon: Some("icons/icona_machinegun"),
        pickup_name: Some("Bullets"),
        quantity: 50,
        kind: ItemKindTag::Ammo(Weapon::WpMachinegun),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("ammo_grenades"),
        pickup_sound: Some("sound/misc/am_pkup.wav"),
        world_models: [
            Some("models/powerups/ammo/grenadeam.md3"),
            None,
            None,
            None,
        ],
        icon: Some("icons/icona_grenade"),
        pickup_name: Some("Grenades"),
        quantity: 5,
        kind: ItemKindTag::Ammo(Weapon::WpGrenadeLauncher),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("ammo_cells"),
        pickup_sound: Some("sound/misc/am_pkup.wav"),
        world_models: [
            Some("models/powerups/ammo/plasmaam.md3"),
            None,
            None,
            None,
        ],
        icon: Some("icons/icona_plasma"),
        pickup_name: Some("Cells"),
        quantity: 30,
        kind: ItemKindTag::Ammo(Weapon::WpPlasmagun),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("ammo_lightning"),
        pickup_sound: Some("sound/misc/am_pkup.wav"),
        world_models: [
            Some("models/powerups/ammo/lightningam.md3"),
            None,
            None,
            None,
        ],
        icon: Some("icons/icona_lightning"),
        pickup_name: Some("Lightning"),
        quantity: 60,
        kind: ItemKindTag::Ammo(Weapon::WpLightning),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("ammo_rockets"),
        pickup_sound: Some("sound/misc/am_pkup.wav"),
        world_models: [
            Some("models/powerups/ammo/rocketam.md3"),
            None,
            None,
            None,
        ],
        icon: Some("icons/icona_rocket"),
        pickup_name: Some("Rockets"),
        quantity: 5,
        kind: ItemKindTag::Ammo(Weapon::WpRocketLauncher),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("ammo_slugs"),
        pickup_sound: Some("sound/misc/am_pkup.wav"),
        world_models: [
            Some("models/powerups/ammo/railgunam.md3"),
            None,
            None,
            None,
        ],
        icon: Some("icons/icona_railgun"),
        pickup_name: Some("Slugs"),
        quantity: 10,
        kind: ItemKindTag::Ammo(Weapon::WpRailgun),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("ammo_bfg"),
        pickup_sound: Some("sound/misc/am_pkup.wav"),
        world_models: [Some("models/powerups/ammo/bfgam.md3"), None, None, None],
        icon: Some("icons/icona_bfg"),
        pickup_name: Some("Bfg Ammo"),
        quantity: 15,
        kind: ItemKindTag::Ammo(Weapon::WpBfg),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("holdable_teleporter"),
        pickup_sound: Some("sound/items/holdable.wav"),
        world_models: [
            Some("models/powerups/holdable/teleporter.md3"),
            None,
            None,
            None,
        ],
        icon: Some("icons/teleporter"),
        pickup_name: Some("Personal Teleporter"),
        quantity: 60,
        kind: ItemKindTag::Holdable(Holdable::HiTeleporter),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("holdable_medkit"),
        pickup_sound: Some("sound/items/holdable.wav"),
        world_models: [
            Some("models/powerups/holdable/medkit.md3"),
            Some("models/powerups/holdable/medkit_sphere.md3"),
            None,
            None,
        ],
        icon: Some("icons/medkit"),
        pickup_name: Some("Medkit"),
        quantity: 60,
        kind: ItemKindTag::Holdable(Holdable::HiMedkit),
        precaches: "",
        sounds: "sound/items/use_medkit.wav",
    },
    ItemDefinition {
        class_name: Some("item_quad"),
        pickup_sound: Some("sound/items/quaddamage.wav"),
        world_models: [
            Some("models/powerups/instant/quad.md3"),
            Some("models/powerups/instant/quad_ring.md3"),
            None,
            None,
        ],
        icon: Some("icons/quad"),
        pickup_name: Some("Quad Damage"),
        quantity: 30,
        kind: ItemKindTag::Powerup(Powerup::PwQuad),
        precaches: "",
        sounds: "sound/items/damage2.wav sound/items/damage3.wav",
    },
    ItemDefinition {
        class_name: Some("item_enviro"),
        pickup_sound: Some("sound/items/protect.wav"),
        world_models: [
            Some("models/powerups/instant/enviro.md3"),
            Some("models/powerups/instant/enviro_ring.md3"),
            None,
            None,
        ],
        icon: Some("icons/envirosuit"),
        pickup_name: Some("Battle Suit"),
        quantity: 30,
        kind: ItemKindTag::Powerup(Powerup::PwBattlesuit),
        precaches: "",
        sounds: "sound/items/airout.wav sound/items/protect3.wav",
    },
    ItemDefinition {
        class_name: Some("item_haste"),
        pickup_sound: Some("sound/items/haste.wav"),
        world_models: [
            Some("models/powerups/instant/haste.md3"),
            Some("models/powerups/instant/haste_ring.md3"),
            None,
            None,
        ],
        icon: Some("icons/haste"),
        pickup_name: Some("Speed"),
        quantity: 30,
        kind: ItemKindTag::Powerup(Powerup::PwHaste),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("item_invis"),
        pickup_sound: Some("sound/items/invisibility.wav"),
        world_models: [
            Some("models/powerups/instant/invis.md3"),
            Some("models/powerups/instant/invis_ring.md3"),
            None,
            None,
        ],
        icon: Some("icons/invis"),
        pickup_name: Some("Invisibility"),
        quantity: 30,
        kind: ItemKindTag::Powerup(Powerup::PwInvis),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("item_regen"),
        pickup_sound: Some("sound/items/regeneration.wav"),
        world_models: [
            Some("models/powerups/instant/regen.md3"),
            Some("models/powerups/instant/regen_ring.md3"),
            None,
            None,
        ],
        icon: Some("icons/regen"),
        pickup_name: Some("Regeneration"),
        quantity: 30,
        kind: ItemKindTag::Powerup(Powerup::PwRegen),
        precaches: "",
        sounds: "sound/items/regen.wav",
    },
    ItemDefinition {
        class_name: Some("item_flight"),
        pickup_sound: Some("sound/items/flight.wav"),
        world_models: [
            Some("models/powerups/instant/flight.md3"),
            Some("models/powerups/instant/flight_ring.md3"),
            None,
            None,
        ],
        icon: Some("icons/flight"),
        pickup_name: Some("Flight"),
        quantity: 60,
        kind: ItemKindTag::Powerup(Powerup::PwFlight),
        precaches: "",
        sounds: "sound/items/flight.wav",
    },
    ItemDefinition {
        class_name: Some("team_CTF_redflag"),
        pickup_sound: None,
        world_models: [Some("models/flags/r_flag.md3"), None, None, None],
        icon: Some("icons/iconf_red1"),
        pickup_name: Some("Red Flag"),
        quantity: 0,
        kind: ItemKindTag::Team(Powerup::PwRedFlag),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("team_CTF_blueflag"),
        pickup_sound: None,
        world_models: [Some("models/flags/b_flag.md3"), None, None, None],
        icon: Some("icons/iconf_blu1"),
        pickup_name: Some("Blue Flag"),
        quantity: 0,
        kind: ItemKindTag::Team(Powerup::PwBlueFlag),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("holdable_kamikaze"),
        pickup_sound: Some("sound/items/holdable.wav"),
        world_models: [Some("models/powerups/kamikazi.md3"), None, None, None],
        icon: Some("icons/kamikaze"),
        pickup_name: Some("Kamikaze"),
        quantity: 60,
        kind: ItemKindTag::Holdable(Holdable::HiKamikaze),
        precaches: "",
        sounds: "sound/items/kamikazerespawn.wav",
    },
    ItemDefinition {
        class_name: Some("holdable_portal"),
        pickup_sound: Some("sound/items/holdable.wav"),
        world_models: [
            Some("models/powerups/holdable/porter.md3"),
            None,
            None,
            None,
        ],
        icon: Some("icons/portal"),
        pickup_name: Some("Portal"),
        quantity: 60,
        kind: ItemKindTag::Holdable(Holdable::HiPortal),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("holdable_invulnerability"),
        pickup_sound: Some("sound/items/holdable.wav"),
        world_models: [
            Some("models/powerups/holdable/invulnerability.md3"),
            None,
            None,
            None,
        ],
        icon: Some("icons/invulnerability"),
        pickup_name: Some("Invulnerability"),
        quantity: 60,
        kind: ItemKindTag::Holdable(Holdable::HiInvulnerability),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("ammo_nails"),
        pickup_sound: Some("sound/misc/am_pkup.wav"),
        world_models: [
            Some("models/powerups/ammo/nailgunam.md3"),
            None,
            None,
            None,
        ],
        icon: Some("icons/icona_nailgun"),
        pickup_name: Some("Nails"),
        quantity: 20,
        kind: ItemKindTag::Ammo(Weapon::WpNailgun),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("ammo_mines"),
        pickup_sound: Some("sound/misc/am_pkup.wav"),
        world_models: [
            Some("models/powerups/ammo/proxmineam.md3"),
            None,
            None,
            None,
        ],
        icon: Some("icons/icona_proxlauncher"),
        pickup_name: Some("Proximity Mines"),
        quantity: 10,
        kind: ItemKindTag::Ammo(Weapon::WpProxLauncher),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("ammo_belt"),
        pickup_sound: Some("sound/misc/am_pkup.wav"),
        world_models: [
            Some("models/powerups/ammo/chaingunam.md3"),
            None,
            None,
            None,
        ],
        icon: Some("icons/icona_chaingun"),
        pickup_name: Some("Chaingun Belt"),
        quantity: 100,
        kind: ItemKindTag::Ammo(Weapon::WpChaingun),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("item_scout"),
        pickup_sound: Some("sound/items/scout.wav"),
        world_models: [Some("models/powerups/scout.md3"), None, None, None],
        icon: Some("icons/scout"),
        pickup_name: Some("Scout"),
        quantity: 30,
        kind: ItemKindTag::PersistantPowerup(Powerup::PwScout),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("item_guard"),
        pickup_sound: Some("sound/items/guard.wav"),
        world_models: [Some("models/powerups/guard.md3"), None, None, None],
        icon: Some("icons/guard"),
        pickup_name: Some("Guard"),
        quantity: 30,
        kind: ItemKindTag::PersistantPowerup(Powerup::PwGuard),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("item_doubler"),
        pickup_sound: Some("sound/items/doubler.wav"),
        world_models: [Some("models/powerups/doubler.md3"), None, None, None],
        icon: Some("icons/doubler"),
        pickup_name: Some("Doubler"),
        quantity: 30,
        kind: ItemKindTag::PersistantPowerup(Powerup::PwDoubler),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("item_ammoregen"),
        pickup_sound: Some("sound/items/ammoregen.wav"),
        world_models: [Some("models/powerups/ammo.md3"), None, None, None],
        icon: Some("icons/ammo_regen"),
        pickup_name: Some("Ammo Regen"),
        quantity: 30,
        kind: ItemKindTag::PersistantPowerup(Powerup::PwAmmoregen),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("team_CTF_neutralflag"),
        pickup_sound: None,
        world_models: [Some("models/flags/n_flag.md3"), None, None, None],
        icon: Some("icons/iconf_neutral1"),
        pickup_name: Some("Neutral Flag"),
        quantity: 0,
        kind: ItemKindTag::Team(Powerup::PwNeutralFlag),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("item_redcube"),
        pickup_sound: Some("sound/misc/am_pkup.wav"),
        world_models: [Some("models/powerups/orb/r_orb.md3"), None, None, None],
        icon: Some("icons/iconh_rorb"),
        pickup_name: Some("Red Cube"),
        quantity: 0,
        kind: ItemKindTag::Team(Powerup::PwNone),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("item_bluecube"),
        pickup_sound: Some("sound/misc/am_pkup.wav"),
        world_models: [Some("models/powerups/orb/b_orb.md3"), None, None, None],
        icon: Some("icons/iconh_borb"),
        pickup_name: Some("Blue Cube"),
        quantity: 0,
        kind: ItemKindTag::Team(Powerup::PwNone),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("weapon_nailgun"),
        pickup_sound: Some("sound/misc/w_pkup.wav"),
        world_models: [
            Some("models/weapons/nailgun/nailgun.md3"),
            None,
            None,
            None,
        ],
        icon: Some("icons/iconw_nailgun"),
        pickup_name: Some("Nailgun"),
        quantity: 10,
        kind: ItemKindTag::Weapon(Weapon::WpNailgun),
        precaches: "",
        sounds: "",
    },
    ItemDefinition {
        class_name: Some("weapon_prox_launcher"),
        pickup_sound: Some("sound/misc/w_pkup.wav"),
        world_models: [
            Some("models/weapons/proxmine/proxmine.md3"),
            None,
            None,
            None,
        ],
        icon: Some("icons/iconw_proxlauncher"),
        pickup_name: Some("Prox Launcher"),
        quantity: 5,
        kind: ItemKindTag::Weapon(Weapon::WpProxLauncher),
        precaches: "",
        sounds: "sound/weapons/proxmine/wstbtick.wav sound/weapons/proxmine/wstbactv.wav sound/weapons/proxmine/wstbimpl.wav sound/weapons/proxmine/wstbimpm.wav sound/weapons/proxmine/wstbimpd.wav sound/weapons/proxmine/wstbactv.wav",
    },
    ItemDefinition {
        class_name: Some("weapon_chaingun"),
        pickup_sound: Some("sound/misc/w_pkup.wav"),
        world_models: [
            Some("models/weapons/vulcan/vulcan.md3"),
            None,
            None,
            None,
        ],
        icon: Some("icons/iconw_chaingun"),
        pickup_name: Some("Chaingun"),
        quantity: 80,
        kind: ItemKindTag::Weapon(Weapon::WpChaingun),
        precaches: "",
        sounds: "sound/weapons/vulcan/wvulwind.wav",
    },
];

/// Base-game item count (the missionpack tail starts at index 36).
pub const BASE_ITEM_COUNT: usize = 36;

/// Item list for a product (`itemList`).
#[must_use]
pub fn item_list(product: Product) -> &'static [ItemDefinition] {
    match product {
        Product::BaseQ3 => &ITEM_DEFINITIONS[..BASE_ITEM_COUNT],
        Product::Missionpack => &ITEM_DEFINITIONS[..],
    }
}

/// Item definition by index (`itemAt`).
pub fn item_at(product: Product, index: i32) -> Result<&'static ItemDefinition, Q3BaseError> {
    let list = item_list(product);
    usize::try_from(index)
        .ok()
        .and_then(|slot| list.get(slot))
        .ok_or_else(|| Q3BaseError::Range(format!("Item index out of range: {index}")))
}

/// Find an item by pickup name, ASCII case-insensitive (`BG_FindItem`).
#[must_use]
pub fn find_item(product: Product, pickup_name: &str) -> Option<&'static ItemDefinition> {
    let folded = ascii_fold(pickup_name);
    item_list(product)
        .iter()
        .find(|item| item.pickup_name.is_some_and(|name| ascii_fold(name) == folded))
}

/// Find a powerup, team, or persistant-powerup item by tag
/// (`BG_FindItemForPowerup`).
#[must_use]
pub fn find_item_for_powerup(product: Product, powerup: Powerup) -> Option<&'static ItemDefinition> {
    item_list(product)
        .iter()
        .find(|item| item.powerup_tag() == Some(powerup))
}

/// Find a holdable item by tag (`BG_FindItemForHoldable`).
pub fn find_item_for_holdable(product: Product, holdable: Holdable) -> Result<&'static ItemDefinition, Q3BaseError> {
    item_list(product)
        .iter()
        .find(|item| item.holdable_tag() == Some(holdable))
        .ok_or_else(|| Q3BaseError::Drop("HoldableItem not found".to_string()))
}

/// Find a weapon item by tag (`BG_FindItemForWeapon`).
pub fn find_item_for_weapon(product: Product, weapon: Weapon) -> Result<&'static ItemDefinition, Q3BaseError> {
    item_list(product)
        .iter()
        .find(|item| item.weapon_tag() == Some(weapon) && item.item_type() == ItemType::ItWeapon)
        .ok_or_else(|| Q3BaseError::Drop(format!("Couldn't find item for weapon {}", weapon as i32)))
}

/// Pickup entity words read by the grab rules (`PickupEntity`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PickupEntity {
    /// Item model index.
    pub model_index: i32,
    /// Second model index.
    pub model_index2: i32,
    /// Generic value.
    pub generic1: i32,
}

/// Player inventory read by the grab rules (`PlayerInventory`).
pub trait PlayerInventory {
    /// Owning product.
    fn product(&self) -> Product;
    /// Health.
    fn health(&self) -> i32;
    /// Armor points.
    fn armor(&self) -> i32;
    /// Maximum health.
    fn max_health(&self) -> i32;
    /// Holdable item tag.
    fn holdable_item(&self) -> i32;
    /// Team tag.
    fn team(&self) -> i32;
    /// Ammunition for a weapon.
    fn ammo(&self, weapon: Weapon) -> i32;
    /// Powerup time remaining.
    fn powerup(&self, powerup: Powerup) -> i32;
    /// Persistant powerup item index (missionpack only).
    fn persistent_powerup_index(&self) -> i32 {
        0
    }
}

/// Whether armor can be grabbed (`canQ3ArmorBeGrabbed`, inlined source).
pub fn can_q3_armor_be_grabbed(ps: &dyn PlayerInventory) -> Result<bool, Q3BaseError> {
    if ps.product() == Product::Missionpack {
        if item_at(ps.product(), ps.persistent_powerup_index())?.powerup_tag() == Some(Powerup::PwScout) {
            return Ok(false);
        }
        let upper_bound =
            if item_at(ps.product(), ps.persistent_powerup_index())?.powerup_tag() == Some(Powerup::PwGuard) {
                ps.max_health()
            } else {
                ps.max_health() * 2
            };
        return Ok(ps.armor() < upper_bound);
    }
    Ok(ps.armor() < ps.max_health() * 2)
}

/// Whether an item can be grabbed (`BG_CanItemBeGrabbed`).
pub fn can_item_be_grabbed(gametype: i32, ent: &PickupEntity, ps: &dyn PlayerInventory) -> Result<bool, Q3BaseError> {
    if ent.model_index < 1 || ent.model_index >= item_list(ps.product()).len() as i32 {
        return Err(Q3BaseError::Drop("BG_CanItemBeGrabbed: index out of range".to_string()));
    }
    let item = item_at(ps.product(), ent.model_index)?;
    match item.item_type() {
        ItemType::ItWeapon => Ok(true),
        ItemType::ItAmmo => Ok(ps.ammo(item.weapon_tag().unwrap_or(Weapon::WpNone)) < 200),
        ItemType::ItArmor => can_q3_armor_be_grabbed(ps),
        ItemType::ItHealth => {
            if ps.product() == Product::Missionpack
                && item_at(ps.product(), ps.persistent_powerup_index())?.powerup_tag() == Some(Powerup::PwGuard)
            {
                return Ok(ps.health() < ps.max_health());
            }
            let limit = if item.quantity == 5 || item.quantity == 100 {
                2
            } else {
                1
            };
            Ok(ps.health() < ps.max_health() * limit)
        }
        ItemType::ItPowerup => Ok(true),
        ItemType::ItPersistantPowerup => {
            if ps.product() == Product::BaseQ3 || ps.persistent_powerup_index() != 0 {
                return Ok(false);
            }
            if (ent.generic1 & 2) != 0 && ps.team() != Team::TeamRed as i32 {
                return Ok(false);
            }
            if (ent.generic1 & 4) != 0 && ps.team() != Team::TeamBlue as i32 {
                return Ok(false);
            }
            Ok(true)
        }
        ItemType::ItTeam => {
            let tag = item.powerup_tag().unwrap_or(Powerup::PwNone);
            if ps.product() == Product::Missionpack && gametype == GameType::Gt1Fctf as i32 {
                if tag == Powerup::PwNeutralFlag {
                    return Ok(true);
                }
                if ps.team() == Team::TeamRed as i32
                    && tag == Powerup::PwBlueFlag
                    && ps.powerup(Powerup::PwNeutralFlag) != 0
                {
                    return Ok(true);
                }
                if ps.team() == Team::TeamBlue as i32
                    && tag == Powerup::PwRedFlag
                    && ps.powerup(Powerup::PwNeutralFlag) != 0
                {
                    return Ok(true);
                }
            }
            if gametype == GameType::GtCtf as i32 {
                if ps.team() == Team::TeamRed as i32 {
                    return Ok(tag == Powerup::PwBlueFlag
                        || (tag == Powerup::PwRedFlag
                            && (ent.model_index2 != 0 || ps.powerup(Powerup::PwBlueFlag) != 0)));
                }
                if ps.team() == Team::TeamBlue as i32 {
                    return Ok(tag == Powerup::PwRedFlag
                        || (tag == Powerup::PwBlueFlag
                            && (ent.model_index2 != 0 || ps.powerup(Powerup::PwRedFlag) != 0)));
                }
            }
            Ok(ps.product() == Product::Missionpack && gametype == GameType::GtHarvester as i32)
        }
        ItemType::ItHoldable => Ok(ps.holdable_item() == 0),
        ItemType::ItBad => Err(Q3BaseError::Drop("BG_CanItemBeGrabbed: IT_BAD".to_string())),
    }
}

/// Whether a player origin touches an item (`BG_PlayerTouchesItem`).
#[must_use]
pub fn player_touches_item(player_origin: Vec3, item_position: &Trajectory, at_time: i32) -> bool {
    let origin = evaluate_trajectory(item_position, at_time);
    let x = player_origin.x - origin.x;
    let y = player_origin.y - origin.y;
    let z = player_origin.z - origin.z;
    !(x > 44.0 || x < -50.0 || y > 36.0 || y < -36.0 || z > 36.0 || z < -36.0)
}

/// ASCII case fold (`asciiFold`).
#[must_use]
pub fn ascii_fold(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_ascii_uppercase() {
                (c as u8 + 32) as char
            } else {
                c
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Session body mirrors (contracts/world.ts BodyState/LinkedBody, minimal)
// ---------------------------------------------------------------------------

/// Actor body state (`BodyState`).
#[derive(Debug, Clone, PartialEq)]
pub struct BodyState {
    /// Origin.
    pub origin: Vec3,
    /// Angles.
    pub angles: Vec3,
    /// Velocity.
    pub velocity: Vec3,
    /// Local bounds.
    pub bounds: Bounds,
    /// Ground actor.
    pub ground: Option<ActorId>,
}

/// Linked body with its captured absolute bounds (`LinkedBody`).
#[derive(Debug, Clone, PartialEq)]
pub struct LinkedBody {
    /// Actor.
    pub actor: ActorId,
    /// State at link time.
    pub state: BodyState,
    /// Absolute bounds visible to spatial queries.
    pub absolute_bounds: Bounds,
    /// Link count.
    pub link_count: i32,
}

/// Zero body state for unowned record slots.
pub const ZERO_BODY: BodyState = BodyState {
    origin: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
    angles: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
    velocity: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
    bounds: Bounds {
        min: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
        max: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
    },
    ground: None,
};

// ---------------------------------------------------------------------------
// shared/entity-shared.ts
// ---------------------------------------------------------------------------

/// Entity collision model (`EntityCollisionModel`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntityCollisionModel {
    /// Inline BSP model with its index.
    Inline {
        /// Model index.
        index: i32,
    },
    /// Bounding box.
    Box,
    /// Capsule.
    Capsule,
}

/// Server entity flags (`ServerEntityFlags`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum ServerEntityFlags {
    /// Not networked to clients.
    Noclient = 1,
    /// Client mask follows.
    Clientmask = 2,
    /// Bot.
    Bot = 8,
    /// Broadcast.
    Broadcast = 32,
    /// Portal.
    Portal = 64,
    /// Use current origin.
    UseCurrentOrigin = 128,
    /// Single client follows.
    Singleclient = 256,
    /// Excluded from server info.
    Noserverinfo = 512,
    /// Inverted single client.
    Notsingleclient = 2048,
}

impl ServerEntityFlags {
    /// Flag bits.
    #[must_use]
    pub fn bits(self) -> i32 {
        self as i32
    }
}

/// Body fields forwarded to the shared authority (`EntityBodyBinding`).
pub trait EntityBodyBinding {
    /// Read the body state.
    fn read(&self) -> BodyState;
    /// Write the body state.
    fn write(&self, value: BodyState);
    /// Read the linked body, if linked.
    fn linked(&self) -> Option<LinkedBody>;
}

/// Private entity-shared save words.
#[derive(Debug, Clone, PartialEq)]
pub struct PrivateEntityState {
    /// Previous link record.
    pub previous_link: Option<LinkedBody>,
    /// Absolute minimum override.
    pub abs_min_override: Option<Vec3>,
    /// Absolute maximum override.
    pub abs_max_override: Option<Vec3>,
}

/// Source `entityShared_t` metadata (`EntityShared`).
///
/// Body fields forward to the shared authority; the remaining words are
/// source metadata.
pub struct EntityShared {
    body: Rc<dyn EntityBodyBinding>,
    previous_link: Option<LinkedBody>,
    current_origin_view: Option<Vec3>,
    abs_min_override: Option<Vec3>,
    abs_max_override: Option<Vec3>,
    /// Server flags.
    pub sv_flags: i32,
    /// Single-client target.
    pub single_client: i32,
    /// Collision model.
    pub model: EntityCollisionModel,
    /// Contents mask.
    pub contents: i32,
    /// Owner entity number.
    pub owner_num: i32,
}

impl std::fmt::Debug for EntityShared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EntityShared")
            .field("sv_flags", &self.sv_flags)
            .field("model", &self.model)
            .field("contents", &self.contents)
            .field("owner_num", &self.owner_num)
            .finish()
    }
}

impl EntityShared {
    /// Shared metadata over a body binding.
    #[must_use]
    pub fn new(body: Rc<dyn EntityBodyBinding>) -> Self {
        Self {
            body,
            previous_link: None,
            current_origin_view: None,
            abs_min_override: None,
            abs_max_override: None,
            sv_flags: 0,
            single_client: 0,
            model: EntityCollisionModel::Box,
            contents: 0,
            owner_num: 0,
        }
    }

    /// Whether the body is linked.
    #[must_use]
    pub fn linked(&self) -> bool {
        self.body.linked().is_some()
    }

    /// Capture the current link record.
    pub fn capture_link(&mut self) {
        if let Some(linked) = self.body.linked() {
            self.previous_link = Some(linked);
        }
    }

    /// Link count, falling back to the previous link record.
    #[must_use]
    pub fn linkcount(&self) -> i32 {
        if let Some(linked) = self.body.linked() {
            linked.link_count
        } else {
            self.previous_link.as_ref().map_or(0, |link| link.link_count)
        }
    }

    /// Local minimum bounds.
    #[must_use]
    pub fn mins(&self) -> Vec3 {
        self.body.read().bounds.min
    }

    /// Write the local minimum bounds.
    pub fn set_mins(&mut self, value: Vec3) {
        let mut state = self.body.read();
        state.bounds.min = value;
        self.body.write(state);
    }

    /// Local maximum bounds.
    #[must_use]
    pub fn maxs(&self) -> Vec3 {
        self.body.read().bounds.max
    }

    /// Write the local maximum bounds.
    pub fn set_maxs(&mut self, value: Vec3) {
        let mut state = self.body.read();
        state.bounds.max = value;
        self.body.write(state);
    }

    /// Current collision origin, from the temporary view when active.
    #[must_use]
    pub fn current_origin(&self) -> Vec3 {
        self.current_origin_view.unwrap_or_else(|| self.body.read().origin)
    }

    /// Write the current collision origin.
    pub fn set_current_origin(&mut self, value: Vec3) {
        let mut state = self.body.read();
        state.origin = value;
        self.body.write(state);
        if let Some(view) = self.current_origin_view.as_mut() {
            *view = value;
        }
    }

    /// Run a call under a temporary snapped origin view.
    ///
    /// `ClientThink` links a snapped source origin while `ps.origin`
    /// retains movement precision.
    pub fn with_current_origin<R>(&mut self, origin: Vec3, call: impl FnOnce() -> R) -> R {
        let previous = self.current_origin_view;
        self.current_origin_view = Some(origin);
        let result = call();
        self.current_origin_view = previous;
        result
    }

    /// Current collision angles.
    #[must_use]
    pub fn current_angles(&self) -> Vec3 {
        self.body.read().angles
    }

    /// Write the current collision angles.
    pub fn set_current_angles(&mut self, value: Vec3) {
        let mut state = self.body.read();
        state.angles = value;
        self.body.write(state);
    }

    /// Capture private save words.
    pub fn capture_private_state(&self) -> Result<PrivateEntityState, Q3BaseError> {
        if self.current_origin_view.is_some() {
            return Err(Q3BaseError::Invalid(
                "Cannot save inside a Q3 temporary origin view".to_string(),
            ));
        }
        Ok(PrivateEntityState {
            previous_link: self.previous_link.clone(),
            abs_min_override: self.abs_min_override,
            abs_max_override: self.abs_max_override,
        })
    }

    /// Restore private save words.
    pub fn restore_private_state(&mut self, state: &PrivateEntityState) -> Result<(), Q3BaseError> {
        if self.current_origin_view.is_some() {
            return Err(Q3BaseError::Invalid(
                "Cannot restore inside a Q3 temporary origin view".to_string(),
            ));
        }
        self.previous_link = state.previous_link.clone();
        self.abs_min_override = state.abs_min_override;
        self.abs_max_override = state.abs_max_override;
        Ok(())
    }

    /// Clear absolute-bounds overrides.
    pub fn clear_bounds_overrides(&mut self) {
        self.abs_min_override = None;
        self.abs_max_override = None;
    }

    /// Override the absolute minimum bounds.
    pub fn set_absmin(&mut self, value: Vec3) {
        self.abs_min_override = Some(value);
    }

    /// Override the absolute maximum bounds.
    pub fn set_absmax(&mut self, value: Vec3) {
        self.abs_max_override = Some(value);
    }

    /// Absolute minimum bounds.
    #[must_use]
    pub fn absmin(&self) -> Vec3 {
        self.abs_min_override.unwrap_or_else(|| {
            self.body
                .linked()
                .map(|linked| linked.absolute_bounds.min)
                .or_else(|| self.previous_link.as_ref().map(|link| link.absolute_bounds.min))
                .unwrap_or_else(|| vec3(0.0, 0.0, 0.0))
        })
    }

    /// Absolute maximum bounds.
    #[must_use]
    pub fn absmax(&self) -> Vec3 {
        self.abs_max_override.unwrap_or_else(|| {
            self.body
                .linked()
                .map(|linked| linked.absolute_bounds.max)
                .or_else(|| self.previous_link.as_ref().map(|link| link.absolute_bounds.max))
                .unwrap_or_else(|| vec3(0.0, 0.0, 0.0))
        })
    }
}

/// Shared entity state words (`SharedEntityState`).
pub type SharedEntityState = EntityState;

/// Entity with shared state words (`SharedEntity`).
pub trait SharedEntity {
    /// Entity state words.
    fn entity_state(&self) -> &EntityState;
    /// Shared collision metadata.
    fn shared(&self) -> &EntityShared;
}

// ---------------------------------------------------------------------------
// shared/jump-pad.ts
// ---------------------------------------------------------------------------

/// Apply jump-pad velocity to a player state (`BG_TouchJumpPad`).
pub fn touch_jump_pad(state: &mut SourcePlayerState, jump_pad: &EntityState) {
    if state.pm_type != MoveType::PmNormal as i32 || state.powerups.get(Powerup::PwFlight as usize) != 0 {
        return;
    }
    if state.jumppad_ent != jump_pad.number {
        let pitch = angle_normalize180(f64::from(vector_to_angles(jump_pad.origin2).x)).abs();
        state.add_event(EntityEvent::EvJumpPad as i32, i32::from(pitch >= 45.0));
    }
    state.jumppad_ent = jump_pad.number;
    state.jumppad_frame = state.pmove_framecount;
    state.set_velocity(vec3(jump_pad.origin2.x, jump_pad.origin2.y, jump_pad.origin2.z));
}

// ---------------------------------------------------------------------------
// shared/snapshot-state.ts
// ---------------------------------------------------------------------------

// `q_shared.h`'s `SnapVector` casts to int, unlike `Sys_SnapVector`'s
// nearest rounding. The out-of-range conversion matches the native x86
// source's indefinite integer.
fn source_snap_component(component: f32) -> f32 {
    if (-2147483648.0..2147483648.0).contains(&component) {
        component.trunc() + 0.0
    } else {
        -2147483648.0
    }
}

fn copy_position(value: Vec3, snap: bool) -> Vec3 {
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

fn convert_player_state(ps: &mut SourcePlayerState, s: &mut EntityState, snap: bool, extrapolation_time: Option<i32>) {
    s.e_type = if ps.pm_type == MoveType::PmIntermission as i32
        || ps.pm_type == MoveType::PmSpectator as i32
        || ps.health() <= GIB_HEALTH
    {
        EntityType::EtInvisible as i32
    } else {
        EntityType::EtPlayer as i32
    };
    s.number = ps.client_num;
    s.pos = Trajectory {
        trajectory_type: if extrapolation_time.is_none() {
            TrajectoryType::TrInterpolate
        } else {
            TrajectoryType::TrLinearStop
        },
        base: copy_position(ps.origin(), snap),
        delta: ps.velocity(),
        time: extrapolation_time.unwrap_or(s.pos.time),
        duration: if extrapolation_time.is_none() {
            s.pos.duration
        } else {
            50
        },
    };
    s.apos.trajectory_type = TrajectoryType::TrInterpolate;
    s.apos.base = copy_position(ps.viewangles, snap);
    s.angles2.y = ps.movement_dir as f32;
    s.legs_anim = ps.legs_anim;
    s.torso_anim = ps.torso_anim;
    s.client_num = ps.client_num;
    s.e_flags = if ps.health() <= 0 {
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
        let slot = (ps.entity_event_sequence & 1) as usize;
        s.event = ps.events.get(slot) | ((ps.entity_event_sequence & 3) << 8);
        s.event_parm = ps.event_parms.get(slot);
        ps.entity_event_sequence = ps.entity_event_sequence.wrapping_add(1);
    }

    s.weapon = ps.weapon;
    s.ground_entity_num = ps.ground_entity_num;
    s.powerups = 0;
    for index in 0..ps.powerups.len() {
        if ps.powerups.get(index) != 0 {
            s.powerups |= 1 << index;
        }
    }
    s.loop_sound = ps.loop_sound;
    s.generic1 = ps.generic1;
}

/// Convert a player state to an entity state
/// (`BG_PlayerStateToEntityState`).
///
/// Updates source-owned fields and consumes one pending predictable event.
pub fn player_state_to_entity_state(ps: &mut SourcePlayerState, destination: &mut EntityState, snap: bool) {
    convert_player_state(ps, destination, snap, None);
}

/// Convert a player state to an extrapolated entity state
/// (`BG_PlayerStateToEntityStateExtraPolate`).
///
/// Publishes at most 50ms of linear extrapolation, matching the source's
/// fixed duration.
pub fn player_state_to_entity_state_extra_polate(
    ps: &mut SourcePlayerState,
    destination: &mut EntityState,
    time: i32,
    snap: bool,
) {
    convert_player_state(ps, destination, snap, Some(time));
}

// ---------------------------------------------------------------------------
// world.ts (+ shared/slide-move.ts)
// ---------------------------------------------------------------------------

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

/// Server trace query (`ServerTraceQuery`).
#[derive(Debug, Clone, PartialEq)]
pub struct ServerTraceQuery {
    /// Start point.
    pub start: Vec3,
    /// End point.
    pub end: Vec3,
    /// Trace shape.
    pub shape: TraceShape,
    /// Entity number to pass through.
    pub pass_entity_num: i32,
    /// Contents mask.
    pub mask: i32,
}

/// Trace solidity (`ServerTraceResult` solidity word).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceSolidity {
    /// Clear.
    Clear,
    /// Started solid.
    StartSolid,
    /// Entirely solid.
    AllSolid,
}

/// Trace contact (`TraceContact`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TraceContact {
    /// No contact.
    None,
    /// Plane contact.
    Plane {
        /// Contact plane.
        plane: Plane,
    },
}

/// Server trace result (`ServerTraceResult`).
#[derive(Debug, Clone, PartialEq)]
pub struct ServerTraceResult {
    /// Fraction traveled.
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

/// Movement trace (`MovementTrace` via `slide-move.ts`).
pub type MovementTrace = ServerTraceResult;

/// Actor trace query (`ActorTraceQuery`).
#[derive(Debug, Clone, PartialEq)]
pub struct ActorTraceQuery {
    /// Start point.
    pub start: Vec3,
    /// End point.
    pub end: Vec3,
    /// Trace shape.
    pub shape: TraceShape,
    /// Actor to pass through.
    pub pass_actor: Option<ActorId>,
    /// Contents mask.
    pub mask: i32,
}

/// Actor trace hit (`ActorTraceResult` hit word).
#[derive(Debug, Clone, PartialEq)]
pub enum ActorTraceHit {
    /// No hit.
    None,
    /// World hit.
    World,
    /// Actor hit.
    Actor {
        /// Hit actor.
        actor: ActorId,
    },
}

/// Actor trace result (`ActorTraceResult`).
#[derive(Debug, Clone, PartialEq)]
pub struct ActorTraceResult {
    /// Fraction traveled.
    pub fraction: f32,
    /// End position.
    pub end: Vec3,
    /// Hit record.
    pub hit: ActorTraceHit,
    /// Contact.
    pub contact: TraceContact,
    /// Solidity.
    pub solidity: TraceSolidity,
    /// Contents.
    pub contents: i32,
    /// Surface flags.
    pub surface_flags: i32,
}

/// Actor spatial queries (`ActorSpatialQueries`).
pub trait ActorSpatialQueries {
    /// Actors overlapping bounds, at most `maximum`.
    fn area_actors(&self, bounds: Bounds, maximum: usize) -> Vec<ActorId>;
    /// Trace against actors.
    fn trace_actor(&self, query: &ActorTraceQuery) -> ActorTraceResult;
    /// Whether bounds contact an actor.
    fn contact_actor(&self, bounds: Bounds, actor: &ActorId, capsule: bool) -> bool;
}

/// Link state (`LinkState`).
#[derive(Debug, Clone, PartialEq)]
pub struct LinkState {
    /// Absolute bounds.
    pub absbounds: Bounds,
    /// Whether linked.
    pub linked: bool,
    /// Link count.
    pub linkcount: i32,
}

/// Source-shaped operations over the session collision and body owners
/// (`ServerWorld`). No world storage lives here.
pub trait ServerWorld {
    /// Whether bounds contact an entity.
    fn entity_contact(&self, bounds: Bounds, entity_num: i32, capsule: bool) -> bool;
    /// Trace against the world.
    fn trace(&self, query: &ServerTraceQuery) -> ServerTraceResult;
    /// Entities overlapping bounds, at most `maximum` (source default
    /// 1024).
    fn area_entities(&self, bounds: Bounds, maximum: usize) -> Vec<i32>;
    /// Contents at a point.
    fn point_contents(&self, point: Vec3, pass_entity_num: i32) -> i32;
    /// Link state for an entity number.
    fn link_state(&self, number: i32) -> Option<LinkState>;
    /// Link an entity.
    fn link(&self, entity: EntityRef);
    /// Unlink an entity number.
    fn unlink(&self, number: i32);
}

/// Combined server world plus actor queries.
pub trait Q3ServerWorld: ServerWorld + ActorSpatialQueries {}

impl<T: ServerWorld + ActorSpatialQueries> Q3ServerWorld for T {}

// ---------------------------------------------------------------------------
// Gameplay mirrors (contracts/gameplay.ts, world/gameplay/*, minimal)
// ---------------------------------------------------------------------------

/// Namespaced item identifier (`ItemId`, `namespace:name`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ItemId(pub String);

impl ItemId {
    /// Build an item identifier.
    #[must_use]
    pub fn new(text: &str) -> Self {
        Self(text.to_string())
    }

    /// Identifier text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Q1 armor effect word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q1ArmorEffect {
    /// Bypass armor.
    Bypass,
    /// Half effectiveness.
    HalfEffectiveness,
}

/// Q2 classic game word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q2ClassicGame {
    /// Base game.
    Base,
    /// Xatrix.
    Xatrix,
    /// Rogue.
    Rogue,
    /// Capture the flag.
    Ctf,
}

/// Q2 native cause encoding (`Q2NativeCause`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Q2NativeCause {
    /// Classic encoding.
    Classic {
        /// Game.
        game: Q2ClassicGame,
        /// Native value.
        value: i32,
    },
    /// Rerelease encoding.
    Rerelease {
        /// Native identifier.
        id: i32,
        /// Friendly fire.
        friendly_fire: bool,
        /// No point loss.
        no_point_loss: bool,
    },
}

/// Environment hazard word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvironmentHazard {
    /// Fall.
    Fall,
    /// Drown.
    Drown,
    /// Lava.
    Lava,
    /// Slime.
    Slime,
    /// Crush.
    Crush,
    /// Trigger.
    Trigger,
}

/// Attack cause word (`AttackProvenance` cause).
#[derive(Debug, Clone, PartialEq)]
pub enum AttackCause {
    /// Quake cause.
    Q1 {
        /// Death type.
        death_type: String,
        /// Armor effect.
        armor_effect: Option<Q1ArmorEffect>,
    },
    /// Quake II cause.
    Q2 {
        /// Canonical means of death.
        means_of_death: i32,
        /// Damage flags.
        damage_flags: i32,
        /// Native encoding.
        native: Option<Q2NativeCause>,
    },
    /// Quake III cause.
    Q3 {
        /// Means of death.
        means_of_death: i32,
        /// Damage flags.
        damage_flags: i32,
    },
    /// Environment cause.
    Environment {
        /// Hazard.
        hazard: EnvironmentHazard,
    },
}

/// Attack provenance (`AttackProvenance`).
#[derive(Debug, Clone, PartialEq)]
pub struct AttackProvenance {
    /// Sequence number.
    pub sequence: i32,
    /// Attack time.
    pub time: SourceTime,
    /// Attacker.
    pub attacker: Option<ActorId>,
    /// Inflictor.
    pub inflictor: Option<ActorId>,
    /// Originating projectile.
    pub originating_projectile: Option<ActorId>,
    /// Weapon item.
    pub weapon: Option<ItemId>,
    /// Weapon provider.
    pub weapon_provider: ProviderId,
    /// Damage powerup owner, when this source already applied its
    /// damage modifier.
    pub damage_powerup_owner: Option<ProviderId>,
    /// Combat provider.
    pub combat_provider: ProviderId,
    /// Inventory provider.
    pub inventory_provider: ProviderId,
    /// Movement provider.
    pub movement_provider: ProviderId,
    /// Cause.
    pub cause: AttackCause,
}

/// Damage delivery word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DamageDelivery {
    /// Direct damage.
    Direct,
    /// Radius damage.
    Radius,
}

/// Damage request (`DamageRequest`).
#[derive(Debug, Clone, PartialEq)]
pub struct DamageRequest {
    /// Attack provenance.
    pub attack: AttackProvenance,
    /// Target.
    pub target: ActorId,
    /// Amount.
    pub amount: f32,
    /// Knockback.
    pub knockback: f32,
    /// Direction.
    pub direction: Vec3,
    /// Point.
    pub point: Vec3,
    /// Normal.
    pub normal: Vec3,
    /// Delivery.
    pub delivery: DamageDelivery,
}

/// Regular armor state (`RegularArmorState`).
#[derive(Debug, Clone, PartialEq)]
pub enum RegularArmorState {
    /// No armor.
    None,
    /// Quake armor.
    Q1 {
        /// Points.
        points: i32,
        /// Absorption.
        absorption: f32,
        /// Item.
        item: ItemId,
    },
    /// Quake II armor.
    Q2 {
        /// Points.
        points: i32,
        /// Normal protection.
        normal_protection: f32,
        /// Energy protection.
        energy_protection: f32,
        /// Item.
        item: ItemId,
    },
    /// Quake III armor.
    Q3 {
        /// Points.
        points: i32,
        /// Protection.
        protection: f32,
    },
    /// Source armor.
    Source {
        /// Points.
        points: i32,
        /// Item.
        item: Option<ItemId>,
    },
}

/// Powered protection state (`PoweredProtectionState`).
#[derive(Debug, Clone, PartialEq)]
pub enum PoweredProtectionState {
    /// No powered protection.
    None,
    /// Screen.
    Screen {
        /// Cells.
        cells: i32,
    },
    /// Shield.
    Shield {
        /// Cells.
        cells: i32,
    },
}

/// Armor state (`ArmorState`).
#[derive(Debug, Clone, PartialEq)]
pub struct ArmorState {
    /// Regular armor.
    pub regular: RegularArmorState,
    /// Powered protection.
    pub powered: PoweredProtectionState,
}

/// Protection channel (`ProtectionChannel`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtectionChannel {
    /// Regular channel.
    Regular,
    /// Powered channel.
    Powered,
}

/// Armor stage word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArmorStage {
    /// Power stage.
    Power,
    /// Regular stage.
    Regular,
}

/// Armor damage flags (`ArmorDamageFlags`).
#[derive(Debug, Clone, PartialEq)]
pub struct ArmorDamageFlags {
    /// Active stage.
    pub stage: Option<ArmorStage>,
    /// Skip all armor.
    pub no_armor: bool,
    /// Skip power armor.
    pub no_power_armor: bool,
    /// Skip regular armor.
    pub no_regular_armor: bool,
    /// Energy damage.
    pub energy: bool,
    /// Regular protection scale.
    pub regular_protection_scale: Option<f32>,
}

/// Armor computation result (`ArmorResult`).
#[derive(Debug, Clone, PartialEq)]
pub struct ArmorResult {
    /// Resulting armor.
    pub armor: ArmorState,
    /// Power damage saved.
    pub power_saved: i32,
    /// Regular damage saved.
    pub regular_saved: i32,
}

/// Armor stage input (`ArmorStageInput`).
#[derive(Debug, Clone, PartialEq)]
pub struct ArmorStageInput {
    /// Damage request.
    pub request: DamageRequest,
    /// Direction.
    pub direction: Vec3,
    /// Point.
    pub point: Vec3,
    /// Normal.
    pub normal: Vec3,
    /// Amount.
    pub amount: i32,
    /// Flags.
    pub flags: ArmorDamageFlags,
}

/// Armor stage result (`ArmorStageResult`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArmorStageResult {
    /// Damage saved.
    pub saved: i32,
}

/// Victim armor context (`VictimArmorContext`).
#[derive(Debug, Clone, PartialEq)]
pub struct VictimArmorContext {
    /// Screen facing dot.
    pub screen_facing_dot: f32,
    /// Damage arithmetic.
    pub arithmetic: VictimArithmetic,
    /// Quake II source profile.
    pub q2: Option<VictimQ2Profile>,
}

/// Victim armor arithmetic word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VictimArithmetic {
    /// Binary32.
    Binary32,
    /// Binary64.
    Binary64,
}

/// Quake II victim armor profile word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VictimQ2Profile {
    /// Classic or rerelease product.
    pub rerelease: bool,
    /// Capture-the-flag rules.
    pub ctf: bool,
    /// Victim alive.
    pub alive: bool,
}

/// Victim armor policy (`VictimArmorPolicy`).
pub type VictimArmorPolicy = Rc<dyn Fn(&DamageRequest, &ArmorState, i32, &ArmorDamageFlags) -> ArmorResult>;

/// Native victim armor over a per-request context (`nativeVictimArmor`).
#[must_use]
pub fn native_victim_armor(context: Rc<dyn Fn(&DamageRequest) -> VictimArmorContext>) -> VictimArmorPolicy {
    Rc::new(move |request, armor, damage, flags| absorb_native_armor(armor, damage, flags, &context(request)))
}

/// Native armor absorption (`absorbNativeArmor`).
///
/// # Panics
///
/// Panics when Q2 armor runs without a Q2 source profile, or when
/// source armor runs without its absorption binding.
#[must_use]
pub fn absorb_native_armor(
    armor: &ArmorState,
    damage: i32,
    flags: &ArmorDamageFlags,
    context: &VictimArmorContext,
) -> ArmorResult {
    let q2_regular = flags.stage != Some(ArmorStage::Power) && matches!(armor.regular, RegularArmorState::Q2 { .. });
    let q2_powered = flags.stage != Some(ArmorStage::Regular) && !matches!(armor.powered, PoweredProtectionState::None);
    if (q2_regular || q2_powered) && context.q2.is_none() {
        panic!("Q2 victim armor requires an explicit classic or rerelease source profile");
    }
    if damage == 0
        || flags.no_armor
        || (matches!(armor.regular, RegularArmorState::None) && matches!(armor.powered, PoweredProtectionState::None))
    {
        return ArmorResult {
            armor: armor.clone(),
            power_saved: 0,
            regular_saved: 0,
        };
    }
    let multiply = |left: f32, right: f32| -> f32 {
        if context.arithmetic == VictimArithmetic::Binary32 {
            left * right
        } else {
            (f64::from(left) * f64::from(right)) as f32
        }
    };
    let protection_scale = flags.regular_protection_scale.unwrap_or(1.0);
    let rerelease = context.q2.is_some_and(|profile| profile.rerelease);
    let facing_limit = 0.3f32;
    let mut power_saved = 0;
    let mut powered = armor.powered.clone();
    let powered_cells = match &powered {
        PoweredProtectionState::None => 0,
        PoweredProtectionState::Screen { cells } | PoweredProtectionState::Shield { cells } => *cells,
    };
    let powered_kind = match &powered {
        PoweredProtectionState::None => None,
        PoweredProtectionState::Screen { .. } => Some(0),
        PoweredProtectionState::Shield { .. } => Some(1),
    };
    if flags.stage != Some(ArmorStage::Regular)
        && !flags.no_power_armor
        && (!rerelease || context.q2.is_some_and(|profile| profile.alive))
        && powered_kind.is_some()
        && powered_cells > 0
        && (powered_kind != Some(0) || context.screen_facing_dot > facing_limit)
    {
        let is_screen = powered_kind == Some(0);
        let damage_per_cell = if is_screen || context.q2.is_some_and(|profile| profile.ctf) {
            1
        } else {
            2
        };
        // `i64` intermediates match the donor's exact float division of
        // integer words.
        let divided_damage = if is_screen {
            damage / 3
        } else {
            ((2 * i64::from(damage)) / 3) as i32
        };
        let protected_damage = if rerelease {
            divided_damage.max(1)
        } else {
            divided_damage
        };
        let doubled_cost = if rerelease {
            flags.energy
        } else {
            flags.no_regular_armor
        };
        let base_available = powered_cells * damage_per_cell;
        let divided_available = if doubled_cost {
            base_available / 2
        } else {
            base_available
        };
        let available = if rerelease {
            divided_available.max(1)
        } else {
            divided_available
        };
        power_saved = available.min(protected_damage);
        let used = (power_saved / damage_per_cell) * if doubled_cost { 2 } else { 1 };
        let remaining = if rerelease {
            0.max(powered_cells - damage_per_cell.max(used))
        } else {
            powered_cells - used
        };
        powered = match powered {
            PoweredProtectionState::Screen { .. } => PoweredProtectionState::Screen { cells: remaining },
            PoweredProtectionState::Shield { .. } => PoweredProtectionState::Shield { cells: remaining },
            PoweredProtectionState::None => PoweredProtectionState::None,
        };
    }
    let mut regular = armor.regular.clone();
    let mut regular_saved = 0;
    if !flags.no_regular_armor && flags.stage != Some(ArmorStage::Power) {
        match &regular {
            RegularArmorState::None => {}
            RegularArmorState::Source { .. } => {
                panic!("Source regular armor requires its original absorption binding");
            }
            RegularArmorState::Q1 { points, absorption, .. } => {
                let points = *points;
                let absorption = *absorption;
                regular_saved = points
                    .min(multiply(multiply(absorption, protection_scale), (damage - power_saved) as f32).ceil() as i32);
                regular = RegularArmorState::Q1 {
                    points: points - regular_saved,
                    absorption: if regular_saved >= points { 0.0 } else { absorption },
                    item: match &armor.regular {
                        RegularArmorState::Q1 { item, .. } => item.clone(),
                        _ => unreachable!("Q1 armor shape changed during absorption"),
                    },
                };
            }
            RegularArmorState::Q2 {
                points,
                normal_protection,
                energy_protection,
                ..
            } => {
                let points = *points;
                let protection = if flags.energy {
                    *energy_protection
                } else {
                    *normal_protection
                };
                regular_saved = points
                    .min(multiply(multiply(protection, protection_scale), (damage - power_saved) as f32).ceil() as i32);
                regular = match &armor.regular {
                    RegularArmorState::Q2 {
                        normal_protection,
                        energy_protection,
                        item,
                        ..
                    } => RegularArmorState::Q2 {
                        points: points - regular_saved,
                        normal_protection: *normal_protection,
                        energy_protection: *energy_protection,
                        item: item.clone(),
                    },
                    _ => unreachable!("Q2 armor shape changed during absorption"),
                };
            }
            RegularArmorState::Q3 { points, protection, .. } => {
                let points = *points;
                let protection = *protection;
                regular_saved =
                    points.min((((damage - power_saved) as f32) * (protection * protection_scale)).ceil() as i32);
                regular = RegularArmorState::Q3 {
                    points: points - regular_saved,
                    protection,
                };
            }
        }
    }
    ArmorResult {
        armor: if regular == armor.regular && powered == armor.powered {
            armor.clone()
        } else {
            ArmorState { regular, powered }
        },
        power_saved,
        regular_saved,
    }
}

/// Decoded damage flags (`attackDamageFlags`).
#[derive(Debug, Clone, PartialEq)]
pub struct AttackDamageFlags {
    /// Armor flags.
    pub armor: ArmorDamageFlags,
    /// Skip knockback.
    pub no_knockback: bool,
    /// Skip protection.
    pub no_protection: bool,
    /// Skip team protection.
    pub no_team_protection: bool,
    /// Destroy armor.
    pub destroy_armor: bool,
}

/// Decode native damage flags by origin (`attackDamageFlags`).
#[must_use]
pub fn attack_damage_flags(request: &DamageRequest) -> AttackDamageFlags {
    let cause = &request.attack.cause;
    let q2 = match cause {
        AttackCause::Q2 { damage_flags, .. } => *damage_flags,
        _ => 0,
    };
    let q3 = match cause {
        AttackCause::Q3 { damage_flags, .. } => *damage_flags,
        _ => 0,
    };
    AttackDamageFlags {
        armor: ArmorDamageFlags {
            stage: None,
            no_armor: ((q2 | q3) & 2) != 0
                || matches!(
                    cause,
                    AttackCause::Q1 {
                        armor_effect: Some(Q1ArmorEffect::Bypass),
                        ..
                    }
                ),
            no_power_armor: (q2 & 0x100) != 0,
            no_regular_armor: (q2 & 0x80) != 0,
            energy: (q2 & 4) != 0,
            regular_protection_scale: Some(
                if matches!(
                    cause,
                    AttackCause::Q1 {
                        armor_effect: Some(Q1ArmorEffect::HalfEffectiveness),
                        ..
                    }
                ) {
                    0.5
                } else {
                    1.0
                },
            ),
        },
        no_knockback: (q2 & 8) != 0 || (q3 & 4) != 0,
        no_protection: (q2 & 0x20) != 0 || (q3 & 8) != 0,
        no_team_protection: (q3 & 0x10) != 0,
        destroy_armor: (q2 & 0x40) != 0,
    }
}

/// Combat state snapshot (`CombatState`).
#[derive(Debug, Clone, PartialEq)]
pub struct CombatState {
    /// Health.
    pub health: i32,
    /// Armor.
    pub armor: ArmorState,
    /// Mass.
    pub mass: i32,
    /// Whether damage is admitted.
    pub can_take_damage: bool,
    /// Invulnerable.
    pub invulnerable: bool,
    /// Source immunity to damage momentum.
    pub no_knockback: bool,
    /// Team word.
    pub team: Option<String>,
}

/// Damage mutation (`DamageMutation`).
#[derive(Debug, Clone, PartialEq)]
pub enum DamageMutation {
    /// Health change.
    Health {
        /// Health before.
        before: i32,
        /// Health after.
        after: i32,
    },
    /// Armor change.
    Armor {
        /// Armor before.
        before: ArmorState,
        /// Armor after.
        after: ArmorState,
    },
    /// Source velocity change.
    SourceVelocity {
        /// Velocity before.
        before: Vec3,
        /// Velocity after.
        after: Vec3,
        /// Movement provider.
        movement_provider: ProviderId,
    },
    /// Impulse.
    Impulse {
        /// Impulse vector.
        impulse: Vec3,
        /// Movement provider.
        movement_provider: ProviderId,
    },
}

/// Damage reaction word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DamageReaction {
    /// No reaction.
    None,
    /// Pain.
    Pain,
    /// Death.
    Death,
}

/// Damage feedback word (`DamageDecision` feedback).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DamageFeedback {
    /// Quake II feedback.
    Q2 {
        /// Power armor saved.
        power_armor: i32,
        /// Armor saved.
        armor: i32,
        /// Blood.
        blood: i32,
        /// Knockback.
        knockback: i32,
    },
    /// Quake III feedback.
    Q3 {
        /// Knockback.
        knockback: i32,
        /// Battlesuit absorbed.
        battlesuit: bool,
    },
}

/// Completed combat result (`CombatResult`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CombatResult {
    /// Applied damage.
    pub applied_damage: i32,
    /// Reaction.
    pub reaction: DamageReaction,
    /// Feedback.
    pub feedback: Option<DamageFeedback>,
}

/// Damage decision (`DamageDecision`).
#[derive(Debug, Clone, PartialEq)]
pub struct DamageDecision {
    /// Damage request.
    pub request: DamageRequest,
    /// Mutations.
    pub mutations: Vec<DamageMutation>,
    /// Applied damage.
    pub applied_damage: i32,
    /// Reaction.
    pub reaction: DamageReaction,
    /// Feedback.
    pub feedback: Option<DamageFeedback>,
}

/// Current combat state accessors (`CurrentCombatState`).
pub trait CurrentCombatState {
    /// Current target state.
    fn target(&self) -> Option<CombatState>;
    /// Current attacker state.
    fn attacker(&self) -> Option<CombatState>;
}

/// Combat progress (`CombatProgress`).
#[derive(Clone)]
pub enum CombatProgress {
    /// Completed decision.
    Complete {
        /// Damage request.
        request: DamageRequest,
        /// Mutations.
        mutations: Vec<DamageMutation>,
        /// Result.
        result: CombatResult,
    },
    /// Armor stage awaiting its store.
    ArmorStage {
        /// Channel.
        channel: ProtectionChannel,
        /// Damage request.
        request: DamageRequest,
        /// Mutations so far.
        mutations: Vec<DamageMutation>,
        /// Stage input.
        input: ArmorStageInput,
        /// Fallback computation.
        fallback: Rc<dyn Fn(&ArmorState) -> ArmorResult>,
        /// Resume with a stage result.
        resume: Rc<dyn Fn(ArmorStageResult, &dyn CurrentCombatState) -> CombatProgress>,
    },
    /// Source continuation awaiting fresh state.
    SourceContinuation {
        /// Damage request.
        request: DamageRequest,
        /// Mutations so far.
        mutations: Vec<DamageMutation>,
        /// Resume with fresh state.
        resume: Rc<dyn Fn(&dyn CurrentCombatState) -> CombatProgress>,
    },
}

impl std::fmt::Debug for CombatProgress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Complete {
                request,
                mutations,
                result,
            } => f
                .debug_struct("Complete")
                .field("request", request)
                .field("mutations", mutations)
                .field("result", result)
                .finish(),
            Self::ArmorStage {
                channel,
                request,
                mutations,
                input,
                ..
            } => f
                .debug_struct("ArmorStage")
                .field("channel", channel)
                .field("request", request)
                .field("mutations", mutations)
                .field("input", input)
                .finish(),
            Self::SourceContinuation { request, mutations, .. } => f
                .debug_struct("SourceContinuation")
                .field("request", request)
                .field("mutations", mutations)
                .finish(),
        }
    }
}

/// Combat policy (`CombatPolicy`, decision layer).
#[derive(Clone)]
pub struct CombatPolicy {
    /// Provider.
    pub id: ProviderId,
    /// Decide a request over target and attacker snapshots.
    pub decide: Rc<dyn Fn(&DamageRequest, &CombatState, Option<&CombatState>) -> CombatProgress>,
}

impl std::fmt::Debug for CombatPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CombatPolicy").field("id", &self.id).finish()
    }
}

/// Damage outcome (`DamageOutcome`).
#[derive(Debug, Clone, PartialEq)]
pub enum DamageOutcome {
    /// Stale target.
    StaleTarget {
        /// Damage request.
        request: DamageRequest,
    },
    /// Committed decision.
    Committed {
        /// Decision.
        decision: DamageDecision,
        /// Whether the target survived.
        survived: bool,
    },
}

/// Source damage modifier (`SourceDamageModifier`).
#[derive(Clone)]
pub struct SourceDamageModifier {
    /// Owning provider.
    pub owner: ProviderId,
    /// Transform an attacker amount.
    pub transform: Rc<dyn Fn(Option<&ActorId>, f32) -> f32>,
}

impl std::fmt::Debug for SourceDamageModifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SourceDamageModifier")
            .field("owner", &self.owner)
            .finish()
    }
}

/// Quake III combat context (`Q3CombatContext`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3CombatContext {
    /// Target is a player.
    pub player: bool,
    /// Attacker is a player.
    pub attacker_player: bool,
    /// Attacker maximum health.
    pub attacker_max_health: i32,
    /// Attacker guard reduction applies.
    pub attacker_guard: bool,
    /// Intermission is queued.
    pub intermission: bool,
    /// Target noclips.
    pub noclip: bool,
    /// Missionpack invulnerability blocks.
    pub missionpack_invulnerability: bool,
    /// Target takes no knockback.
    pub no_knockback: bool,
    /// Knockback scale.
    pub knockback_scale: f32,
    /// Friendly fire.
    pub friendly_fire: bool,
    /// Battlesuit absorption.
    pub battlesuit: bool,
    /// Falling damage.
    pub falling: bool,
    /// Juiced damage.
    pub juiced: bool,
    /// Proximity protection.
    pub proximity_protected: bool,
    /// Product.
    pub product: Product,
}

fn combat_self_damage(request: &DamageRequest) -> bool {
    request
        .attack
        .attacker
        .as_ref()
        .is_some_and(|attacker| *attacker == request.target)
}

fn combat_same_team(target: &CombatState, attacker: Option<&CombatState>) -> bool {
    target
        .team
        .as_ref()
        .is_some_and(|team| !team.is_empty() && attacker.and_then(|state| state.team.as_ref()) == Some(team))
}

fn combat_decision(
    request: &DamageRequest,
    mutations: Vec<DamageMutation>,
    applied_damage: i32,
    reaction: DamageReaction,
    feedback: Option<DamageFeedback>,
) -> CombatProgress {
    CombatProgress::Complete {
        request: request.clone(),
        mutations,
        result: CombatResult {
            applied_damage,
            reaction,
            feedback,
        },
    }
}

fn combat_continuation(
    request: &DamageRequest,
    mutations: Vec<DamageMutation>,
    resume: Rc<dyn Fn(&dyn CurrentCombatState) -> CombatProgress>,
) -> CombatProgress {
    CombatProgress::SourceContinuation {
        request: request.clone(),
        mutations,
        resume,
    }
}

fn combat_armor_stage(
    channel: ProtectionChannel,
    request: &DamageRequest,
    amount: i32,
    flags: &ArmorDamageFlags,
    armor: &VictimArmorPolicy,
    resume: Rc<dyn Fn(i32, &dyn CurrentCombatState) -> CombatProgress>,
) -> CombatProgress {
    let stage = match channel {
        ProtectionChannel::Powered => ArmorStage::Power,
        ProtectionChannel::Regular => ArmorStage::Regular,
    };
    let mut staged = flags.clone();
    staged.stage = Some(stage);
    let input = ArmorStageInput {
        request: request.clone(),
        direction: request.direction,
        point: request.point,
        normal: request.normal,
        amount,
        flags: staged.clone(),
    };
    let fallback_armor = armor.clone();
    let fallback_request = request.clone();
    let fallback_amount = amount;
    CombatProgress::ArmorStage {
        channel,
        request: request.clone(),
        mutations: Vec::new(),
        input,
        fallback: Rc::new(move |current| fallback_armor(&fallback_request, current, fallback_amount, &staged)),
        resume: Rc::new(move |result, current| resume(result.saved, current)),
    }
}

fn combat_add_impulse(request: &DamageRequest, mutations: &mut Vec<DamageMutation>, direction: Vec3, amount: f32) {
    if amount != 0.0 {
        mutations.push(DamageMutation::Impulse {
            impulse: combat_scale(direction, amount),
            movement_provider: request.attack.movement_provider.clone(),
        });
    }
}

fn combat_scale(direction: Vec3, amount: f32) -> Vec3 {
    let x = direction.x;
    let y = direction.y;
    let z = direction.z;
    let length = (x * x + y * y + z * z).sqrt();
    if length == 0.0 {
        return vec3(0.0, 0.0, 0.0);
    }
    let inverse = 1.0 / length;
    vec3((x * inverse) * amount, (y * inverse) * amount, (z * inverse) * amount)
}

/// Quake III combat policy (`createQ3CombatPolicy`).
///
/// Request amounts truncate to `i32` damage words, matching the source's
/// integer pipeline for in-range amounts.
#[must_use]
pub fn create_q3_combat_policy(
    id: ProviderId,
    armor: VictimArmorPolicy,
    context: Rc<dyn Fn(&DamageRequest, &CombatState, Option<&CombatState>) -> Q3CombatContext>,
) -> CombatPolicy {
    CombatPolicy {
        id,
        decide: Rc::new(move |request, target, attacker| {
            if !target.can_take_damage {
                return combat_decision(request, Vec::new(), 0, DamageReaction::None, None);
            }
            let context = context(request, target, attacker);
            if context.intermission || context.noclip || (context.missionpack_invulnerability && !context.juiced) {
                return combat_decision(request, Vec::new(), 0, DamageReaction::None, None);
            }
            let flags = attack_damage_flags(request);
            let mut damage = request.amount as i32;
            if context.attacker_player && !combat_self_damage(request) {
                let maximum = if context.attacker_guard {
                    context.attacker_max_health / 2
                } else {
                    context.attacker_max_health
                };
                damage = damage.wrapping_mul(maximum) / 100;
            }
            let mut mutations = Vec::new();
            let knockback = if context.no_knockback || target.no_knockback || flags.no_knockback {
                0
            } else {
                damage.min(200)
            };
            let battlesuit = Rc::new(RefCell::new(false));
            let finishing = |mutations: Vec<DamageMutation>,
                             applied: i32,
                             reaction: DamageReaction,
                             battlesuit: bool|
             -> CombatProgress {
                combat_decision(
                    request,
                    mutations,
                    applied,
                    reaction,
                    Some(DamageFeedback::Q3 { knockback, battlesuit }),
                )
            };
            if context.player && !context.no_knockback && !target.no_knockback && !flags.no_knockback {
                combat_add_impulse(
                    request,
                    &mut mutations,
                    request.direction,
                    (context.knockback_scale * (knockback as f32)) / 200.0,
                );
            }
            if !flags.no_protection {
                let check_team = context.product == Product::BaseQ3 || (!context.juiced && !flags.no_team_protection);
                if (check_team
                    && !combat_self_damage(request)
                    && combat_same_team(target, attacker)
                    && !context.friendly_fire)
                    || context.proximity_protected
                    || target.invulnerable
                {
                    return finishing(mutations, 0, DamageReaction::None, false);
                }
            }
            if context.battlesuit {
                *battlesuit.borrow_mut() = true;
                if request.delivery == DamageDelivery::Radius || context.falling {
                    return finishing(mutations, 0, DamageReaction::None, true);
                }
                damage /= 2;
            }
            if combat_self_damage(request) {
                damage /= 2;
            }
            damage = damage.max(1);
            let amount = damage;
            let armor_power = armor.clone();
            let armor_regular = armor.clone();
            let flags_power = flags.clone();
            let battlesuit_inner = battlesuit.clone();
            let owned = (*request).clone();
            combat_continuation(
                request,
                mutations,
                Rc::new({
                    let owned = owned.clone();
                    move |_| {
                        let flags_regular = flags_power.clone();
                        let armor_inner = armor_regular.clone();
                        let battlesuit = battlesuit_inner.clone();
                        let owned = owned.clone();
                        combat_armor_stage(
                            ProtectionChannel::Powered,
                            &owned,
                            amount,
                            &flags_power.armor,
                            &armor_power,
                            Rc::new({
                                let owned = owned.clone();
                                move |power_saved, current| {
                                    if current.target().is_none() {
                                        return combat_decision(&owned, Vec::new(), 0, DamageReaction::None, None);
                                    }
                                    let owned = owned.clone();
                                    let battlesuit = battlesuit.clone();
                                    combat_armor_stage(
                                        ProtectionChannel::Regular,
                                        &owned,
                                        amount.wrapping_sub(power_saved),
                                        &flags_regular.armor,
                                        &armor_inner,
                                        Rc::new({
                                            let owned = owned.clone();
                                            let battlesuit = battlesuit.clone();
                                            move |saved, state| {
                                                let Some(latest) = state.target() else {
                                                    return combat_decision(
                                                        &owned,
                                                        Vec::new(),
                                                        0,
                                                        DamageReaction::None,
                                                        None,
                                                    );
                                                };
                                                let take = amount.wrapping_sub(power_saved.wrapping_add(saved));
                                                let feedback = DamageFeedback::Q3 {
                                                    knockback,
                                                    battlesuit: *battlesuit.borrow(),
                                                };
                                                if take == 0 {
                                                    return combat_decision(
                                                        &owned,
                                                        Vec::new(),
                                                        0,
                                                        DamageReaction::None,
                                                        Some(feedback),
                                                    );
                                                }
                                                let health = (latest.health.wrapping_sub(take)).max(-999);
                                                combat_decision(
                                                    &owned,
                                                    vec![DamageMutation::Health {
                                                        before: latest.health,
                                                        after: health,
                                                    }],
                                                    take,
                                                    if health <= 0 {
                                                        DamageReaction::Death
                                                    } else {
                                                        DamageReaction::Pain
                                                    },
                                                    Some(feedback),
                                                )
                                            }
                                        }),
                                    )
                                }
                            }),
                        )
                    }
                }),
            )
        }),
    }
}

// ---------------------------------------------------------------------------
// Sibling mirrors (game/state.ts, game/combat.ts, game/entities.ts,
// game/use-participant.ts, foundation/arsenal.ts)
// ---------------------------------------------------------------------------
//
// The game-layer files below are owned by sibling ports; this module
// defines the minimal shapes its own donors use so it stays
// self-contained. The parent unifies these with the sibling definitions
// at merge.

/// Client slot count (`MAX_CLIENTS`, game/state.ts).
pub const MAX_CLIENTS: usize = 64;
/// Entity slot count (`MAX_GENTITIES`, game/state.ts).
pub const MAX_GENTITIES: usize = 1024;

/// Game entity flags (`GameFlags`, game/state.ts).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum GameFlags {
    /// God mode.
    Godmode = 0x10,
    /// Notarget.
    Notarget = 0x20,
    /// Team slave.
    Teamslave = 0x400,
    /// No knockback.
    NoKnockback = 0x800,
    /// Dropped item.
    DroppedItem = 0x1000,
    /// No bots.
    NoBots = 0x2000,
    /// No humans.
    NoHumans = 0x4000,
    /// Force gesture.
    ForceGesture = 0x8000,
}

impl GameFlags {
    /// Flag bits.
    #[must_use]
    pub fn bits(self) -> i32 {
        self as i32
    }
}

/// Weapon inventory binding (`Q3WeaponItem`, foundation/arsenal.ts).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3WeaponItem {
    /// Weapon tag.
    pub weapon: Weapon,
    /// Weapon item.
    pub item: ItemId,
    /// Ammunition item.
    pub ammo: Option<ItemId>,
}

const Q3_WEAPON_ITEM_DATA: [(i32, &str, Option<&str>); 13] = [
    (1, "q3:weapon/gauntlet", None),
    (2, "q3:weapon/machinegun", Some("q3:ammo/machinegun")),
    (3, "q3:weapon/shotgun", Some("q3:ammo/shotgun")),
    (4, "q3:weapon/grenadelauncher", Some("q3:ammo/grenadelauncher")),
    (5, "q3:weapon/rocketlauncher", Some("q3:ammo/rocketlauncher")),
    (6, "q3:weapon/lightning", Some("q3:ammo/lightning")),
    (7, "q3:weapon/railgun", Some("q3:ammo/railgun")),
    (8, "q3:weapon/plasmagun", Some("q3:ammo/plasmagun")),
    (9, "q3:weapon/bfg", Some("q3:ammo/bfg")),
    (10, "q3:weapon/grapple", None),
    (11, "q3:weapon/nailgun", Some("q3:ammo/nailgun")),
    (12, "q3:weapon/proxlauncher", Some("q3:ammo/proxlauncher")),
    (13, "q3:weapon/chaingun", Some("q3:ammo/chaingun")),
];

/// Weapon inventory bindings (`Q3_WEAPON_ITEMS`, foundation/arsenal.ts).
#[must_use]
pub fn q3_weapon_items() -> Vec<Q3WeaponItem> {
    Q3_WEAPON_ITEM_DATA
        .iter()
        .map(|(weapon, item, ammo)| Q3WeaponItem {
            weapon: Weapon::from_i32(*weapon).unwrap_or(Weapon::WpNone),
            item: ItemId::new(item),
            ammo: ammo.map(ItemId::new),
        })
        .collect()
}

/// Weapon inventory binding by tag (`q3WeaponItem`,
/// foundation/arsenal.ts).
#[must_use]
pub fn q3_weapon_item(weapon: i32) -> Option<Q3WeaponItem> {
    q3_weapon_items()
        .into_iter()
        .find(|entry| entry.weapon as i32 == weapon)
}

/// Shared game entity handle.
pub type EntityRef = Rc<RefCell<GameEntity>>;
/// Shared game client handle.
pub type ClientRef = Rc<RefCell<GameClient>>;

/// Damage participant (`DamageParticipant`/`UseParticipant`/`DamageInflictor`,
/// game/state.ts).
#[derive(Clone)]
pub enum DamageParticipant {
    /// Native entity.
    Native(EntityRef),
    /// Shared foreign actor.
    SharedActor(SharedParticipant),
}

impl std::fmt::Debug for DamageParticipant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Native(entity) => f.debug_tuple("Native").field(&entity.borrow().slot).finish(),
            Self::SharedActor(participant) => f.debug_tuple("SharedActor").field(&participant.actor).finish(),
        }
    }
}

/// Shared foreign participant (game/state.ts `shared-actor` layer).
#[derive(Clone)]
pub struct SharedParticipant {
    /// Actor.
    pub actor: ActorId,
    origin: Rc<dyn Fn() -> Option<Vec3>>,
}

impl SharedParticipant {
    /// Shared participant with an origin resolver.
    #[must_use]
    pub fn new(actor: ActorId, origin: Rc<dyn Fn() -> Option<Vec3>>) -> Self {
        Self { actor, origin }
    }

    /// Resolve the current origin, if live.
    #[must_use]
    pub fn origin(&self) -> Option<Vec3> {
        (self.origin)()
    }
}

/// Use participant (`UseParticipant`, game/state.ts).
pub type UseParticipant = DamageParticipant;
/// Damage inflictor (`DamageInflictor`, game/state.ts).
pub type DamageInflictor = DamageParticipant;

/// Actor for a participant (`useActor`, game/use-participant.ts).
#[must_use]
pub fn use_actor(participant: &DamageParticipant) -> ActorId {
    match participant {
        DamageParticipant::Native(entity) => entity.borrow().actor().id().clone(),
        DamageParticipant::SharedActor(shared) => shared.actor.clone(),
    }
}

/// Entity think callback (`EntityThink`, game/state.ts).
pub type EntityThinkCallback = Rc<dyn Fn(EntityRef)>;
/// Entity touch callback (`EntityTouch`, game/state.ts).
pub type EntityTouchCallback = Rc<dyn Fn(EntityRef, DamageParticipant, TouchContact)>;
/// Entity use callback (`EntityUse`, game/state.ts).
pub type EntityUseCallback = Rc<dyn Fn(EntityRef, Option<UseParticipant>, Option<UseParticipant>)>;
/// Entity pain callback (`EntityPain`, game/state.ts).
pub type EntityPainCallback = Rc<dyn Fn(EntityRef, DamageParticipant, i32)>;
/// Entity die callback (`EntityDie`, game/state.ts).
pub type EntityDieCallback = Rc<dyn Fn(EntityRef, DamageInflictor, DamageParticipant, i32, i32)>;

/// Source-zero `gentity_t` binding (`GameEntityBinding`, game/state.ts).
pub trait GameEntityBinding {
    /// Body binding.
    fn body(&self) -> Rc<dyn EntityBodyBinding>;
    /// Owning actor.
    fn actor(&self) -> OwnedActor;
    /// Whether the slot is live.
    fn active(&self) -> bool;
    /// Health.
    fn health(&self) -> i32;
    /// Write health.
    fn set_health(&self, value: i32);
    /// Whether damage is admitted.
    fn takes_damage(&self) -> bool;
    /// Write damage admission.
    fn set_takes_damage(&self, value: bool);
    /// Schedule the next think.
    fn schedule(&self, nextthink: i32);
    /// Run a think at a time.
    fn run_think(&self, time_milliseconds: i32);
}

/// Source-zero `gentity_t` (`GameEntity`, game/state.ts).
///
/// This mirror carries the words the base root reads and writes; the
/// sibling game-layer port owns the full record.
pub struct GameEntity {
    /// Owned-table slot.
    pub slot: usize,
    /// Session binding.
    pub binding: Rc<dyn GameEntityBinding>,
    /// Entity state words.
    pub s: EntityState,
    /// Shared collision metadata.
    pub r: EntityShared,
    /// Client record, for player slots.
    pub client: Option<ClientRef>,
    /// Game flags.
    pub flags: i32,
    /// Team name.
    pub team: Option<String>,
    /// Next teammate in the chain.
    pub teamchain: Option<EntityRef>,
    /// Team master (weak; the master owns the chain).
    pub teammaster: Option<Weak<RefCell<GameEntity>>>,
    /// Target name.
    pub targetname: Option<String>,
    nextthink_value: i32,
    /// Think callback.
    pub think: Option<EntityThinkCallback>,
    /// Touch callback.
    pub touch: Option<EntityTouchCallback>,
    /// Use callback.
    pub use_action: Option<EntityUseCallback>,
    /// Pain callback.
    pub pain: Option<EntityPainCallback>,
    /// Die callback.
    pub die: Option<EntityDieCallback>,
}

impl std::fmt::Debug for GameEntity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GameEntity")
            .field("slot", &self.slot)
            .field("flags", &self.flags)
            .field("team", &self.team)
            .finish()
    }
}

impl GameEntity {
    /// Source-zero entity over a binding.
    ///
    /// # Panics
    ///
    /// Panics when `slot` is outside `0..1023`.
    #[must_use]
    pub fn new(slot: usize, binding: Rc<dyn GameEntityBinding>) -> Self {
        assert!(slot < MAX_GENTITIES, "Game entity slot outside 0..1023");
        let r = EntityShared::new(binding.body());
        Self {
            slot,
            binding,
            s: EntityState::new(),
            r,
            client: None,
            flags: 0,
            team: None,
            teamchain: None,
            teammaster: None,
            targetname: None,
            nextthink_value: 0,
            think: None,
            touch: None,
            use_action: None,
            pain: None,
            die: None,
        }
    }

    /// Whether the slot is live.
    #[must_use]
    pub fn inuse(&self) -> bool {
        self.binding.active()
    }

    /// Owning actor.
    #[must_use]
    pub fn actor(&self) -> OwnedActor {
        self.binding.actor()
    }

    /// Health.
    #[must_use]
    pub fn health(&self) -> i32 {
        self.binding.health()
    }

    /// Write health.
    pub fn set_health(&self, value: i32) {
        self.binding.set_health(value);
    }

    /// Whether damage is admitted.
    #[must_use]
    pub fn takes_damage(&self) -> bool {
        self.binding.takes_damage()
    }

    /// Write damage admission.
    pub fn set_takes_damage(&self, value: bool) {
        self.binding.set_takes_damage(value);
    }

    /// Next think time.
    #[must_use]
    pub fn nextthink(&self) -> i32 {
        self.nextthink_value
    }

    /// Write the next think time and schedule it.
    pub fn set_nextthink(&mut self, value: i32) {
        self.nextthink_value = value;
        self.binding.schedule(value);
    }

    /// Restore a think time without scheduling (save hydration).
    pub fn restore_nextthink(&mut self, value: i32) {
        self.nextthink_value = value;
    }

    /// Reset source metadata in place, keeping the slot and binding.
    pub fn reset(&mut self) {
        let binding = self.binding.clone();
        let slot = self.slot;
        *self = Self::new(slot, binding);
    }
}

impl SharedEntity for GameEntity {
    fn entity_state(&self) -> &EntityState {
        &self.s
    }

    fn shared(&self) -> &EntityShared {
        &self.r
    }
}

/// Client session words (`ClientSession`, game/state.ts, minimal).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientSession {
    /// Session team.
    pub session_team: Team,
}

impl Default for ClientSession {
    fn default() -> Self {
        Self {
            session_team: Team::TeamFree,
        }
    }
}

/// Source-zero `gclient_t` (`GameClient`, game/state.ts).
///
/// This mirror carries the words the base root reads and writes; the
/// sibling game-layer port owns the full record.
pub struct GameClient {
    /// Player state.
    pub ps: PlayerState,
    /// Session.
    pub sess: ClientSession,
    /// Noclip.
    pub noclip: bool,
    /// Invulnerability time.
    pub invulnerability_time: i32,
    /// Ammunition timers.
    pub ammo_times: PlayerStateSlots,
}

impl std::fmt::Debug for GameClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GameClient")
            .field("ps", &self.ps)
            .field("sess", &self.sess)
            .finish()
    }
}

impl GameClient {
    /// Source-zero client with an optional authority and ammunition
    /// store notification.
    #[must_use]
    pub fn new(
        product: Product,
        authority: Option<Rc<dyn PlayerAuthorityBinding>>,
        ammo_timer_stored: Option<Rc<dyn Fn(usize, i32)>>,
    ) -> Self {
        Self {
            ps: create_player_state(product, authority),
            sess: ClientSession::default(),
            noclip: false,
            invulnerability_time: 0,
            ammo_times: PlayerStateSlots::new(weapon_count(product) as usize, None, None, ammo_timer_stored),
        }
    }
}

/// Touch contact (`TouchContact`, contracts/world.ts, minimal).
#[derive(Debug, Clone, PartialEq)]
pub struct TouchContact {
    /// Other actor.
    pub other: ActorId,
}

/// Pain reaction (`PainReaction`, contracts/world.ts, minimal).
#[derive(Debug, Clone, PartialEq)]
pub struct PainReaction {
    /// Attack provenance, if any.
    pub attack: Option<AttackProvenance>,
    /// Attacker.
    pub attacker: Option<ActorId>,
    /// Damage.
    pub damage: i32,
}

/// Death reaction (`DeathReaction`, contracts/world.ts, minimal).
#[derive(Debug, Clone, PartialEq)]
pub struct DeathReaction {
    /// Attack provenance, if any.
    pub attack: Option<AttackProvenance>,
    /// Attacker.
    pub attacker: Option<ActorId>,
    /// Damage.
    pub damage: i32,
    /// Inflictor.
    pub inflictor: Option<ActorId>,
    /// Point.
    pub point: Vec3,
}

/// Actor callbacks bound by the records (`ActorCallbacks`,
/// contracts/world.ts, minimal).
#[derive(Clone)]
pub struct ActorCallbacks {
    /// Think callback.
    pub think: Rc<dyn Fn()>,
    /// Touch callback.
    pub touch: Rc<dyn Fn(&TouchContact)>,
    /// Use callback over other and activator actors.
    pub use_action: Rc<dyn Fn(Option<ActorId>, Option<ActorId>)>,
    /// Pain callback.
    pub pain: Rc<dyn Fn(&PainReaction)>,
    /// Die callback.
    pub die: Rc<dyn Fn(&DeathReaction)>,
}

/// Damage admission word (`admitDamage` result).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DamageAdmission {
    /// Continue to combat.
    Continue,
    /// Handled by the game.
    Handled,
}

/// Damage admission callback.
pub type DamageAdmissionFn = Rc<dyn Fn(&DamageRequest) -> DamageAdmission>;

/// Session actor registry services (`SessionActorRegistry`, minimal).
pub trait Q3SessionActors {
    /// Assert an actor handle is owned.
    fn assert_owned(&self, actor: &OwnedActor) -> Result<(), Q3BaseError>;
    /// Allocate an actor at a source slot.
    fn allocate_at_source(&self, provider: &ProviderId, slot: usize, definition: &str) -> OwnedActor;
    /// Whether an actor is live.
    fn is_live(&self, actor: &ActorId) -> bool;
    /// Observe actor release; returns an unobserve callback.
    fn on_release(&self, callback: Box<dyn Fn(&OwnedActor)>) -> Box<dyn Fn()>;
    /// Release an actor.
    fn release(&self, actor: &OwnedActor);
    /// Resolve an owned handle.
    fn resolve_owned(&self, actor: &ActorId) -> Option<OwnedActor>;
}

/// Shared body table services (`SharedBodyTable`, minimal).
pub trait Q3SessionBodies {
    /// Create a body.
    fn create(&self, actor: &OwnedActor, state: BodyState);
    /// Read a body.
    fn read(&self, actor: &ActorId) -> Option<BodyState>;
    /// Write a body.
    fn write(&self, actor: &OwnedActor, state: BodyState);
    /// Read a linked body.
    fn linked(&self, actor: &ActorId) -> Option<LinkedBody>;
    /// Link a body, optionally at a snapped origin.
    fn link(&self, actor: &OwnedActor, origin: Option<Vec3>);
    /// Unlink a body.
    fn unlink(&self, actor: &OwnedActor);
}

/// Gameplay authority services (`GameplayAuthority`, minimal).
pub trait Q3SessionCombat {
    /// Read combat state.
    fn read(&self, actor: &ActorId) -> Option<CombatState>;
    /// Create combat state with optional damage admission.
    fn create(&self, actor: &OwnedActor, initial: CombatState, admit_damage: Option<DamageAdmissionFn>);
    /// Write health.
    fn set_health(&self, actor: &OwnedActor, health: i32);
    /// Write damage admission.
    fn set_can_take_damage(&self, actor: &OwnedActor, can_take_damage: bool);
    /// Write regular armor points.
    fn set_regular_points(&self, actor: &OwnedActor, points: i32, initial: RegularArmorState);
    /// Bind damage admission.
    fn bind_damage_admission(&self, actor: &OwnedActor, admit_damage: DamageAdmissionFn);
    /// Apply a damage request.
    fn apply(&self, request: DamageRequest) -> DamageOutcome;
}

/// Inventory entry (`InventoryEntry`, minimal).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InventoryEntry {
    /// Item.
    pub item: ItemId,
    /// Count.
    pub count: i32,
    /// Capacity.
    pub capacity: i32,
}

/// Shared inventory table services (`SharedInventoryTable`, minimal).
pub trait Q3SessionInventory {
    /// Whether an inventory exists.
    fn has(&self, actor: &ActorId) -> bool;
    /// Create an inventory.
    fn create(&self, actor: &OwnedActor, entries: Vec<InventoryEntry>);
    /// Count an item.
    fn count(&self, actor: &ActorId, item: &ItemId) -> i32;
    /// Configure an item count and capacity.
    fn configure(&self, actor: &OwnedActor, item: &ItemId, count: i32, capacity: i32);
}

/// Actor callback table services (`ActorCallbackTable`, minimal).
pub trait Q3ActorCallbacks {
    /// Bind actor callbacks.
    fn bind(&self, actor: &OwnedActor, callbacks: ActorCallbacks);
}

/// Damage call record (`Q3DamageCall`, game/combat.ts).
#[derive(Clone)]
pub struct Q3DamageCall {
    /// Target.
    pub target: EntityRef,
    /// Inflictor participant.
    pub source: DamageParticipant,
    /// Attacker participant.
    pub owner: UseParticipant,
    /// Direction.
    pub direction: Option<Vec3>,
    /// Point.
    pub point: Option<Vec3>,
    /// Amount.
    pub amount: f32,
    /// Flags.
    pub flags: i32,
    /// Means of death.
    pub method_of_death: i32,
}

impl std::fmt::Debug for Q3DamageCall {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Q3DamageCall")
            .field("target", &self.target.borrow().slot)
            .field("amount", &self.amount)
            .field("flags", &self.flags)
            .field("method_of_death", &self.method_of_death)
            .finish()
    }
}

/// Damage diagnostic (`DamageDiagnostic`, game/combat.ts).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DamageDiagnostic {
    /// Time.
    pub time: i32,
    /// Entity number.
    pub entity_num: i32,
    /// Health.
    pub health: i32,
    /// Damage.
    pub damage: i32,
    /// Armor.
    pub armor: i32,
}

/// Combat actor services (`CombatContext` actors word, game/combat.ts).
#[derive(Clone)]
pub struct CombatActors {
    /// Whether an actor is live.
    pub is_live: Rc<dyn Fn(&ActorId) -> bool>,
    /// Participant for an actor.
    pub participant: Rc<dyn Fn(&ActorId) -> DamageParticipant>,
    /// Projectile parent.
    pub parent: Rc<dyn Fn(&ActorId) -> Option<ActorId>>,
    /// Linked bounds.
    pub linked_bounds: Rc<dyn Fn(&ActorId) -> Option<Bounds>>,
    /// Whether an actor is a player.
    pub is_player: Rc<dyn Fn(&ActorId) -> bool>,
}

/// Combat context (`CombatContext`, game/combat.ts).
///
/// Product-specific words are `None` on baseq3, matching the donor's
/// discriminated union.
#[derive(Clone)]
pub struct CombatContext {
    /// Product.
    pub product: Product,
    /// Gameplay authority.
    pub authority: Rc<dyn Q3SessionCombat>,
    /// Entity pool.
    pub entities: EntityPoolRef,
    /// Spatial queries.
    pub spatial: Rc<dyn Q3ServerWorld>,
    /// Source damage modifier.
    pub source_damage_modifier: Option<SourceDamageModifier>,
    /// Actor services.
    pub actors: CombatActors,
    /// Current time.
    pub time: Rc<dyn Fn() -> i32>,
    /// Queued intermission.
    pub intermission_queued: Rc<dyn Fn() -> i32>,
    /// Game type tag.
    pub game_type: Rc<dyn Fn() -> i32>,
    /// Friendly fire.
    pub friendly_fire: Rc<dyn Fn() -> bool>,
    /// Knockback scale.
    pub knockback: Rc<dyn Fn() -> f32>,
    /// Damage debug sink.
    pub debug_damage: Option<Rc<dyn Fn(DamageDiagnostic)>>,
    /// Capture attack provenance.
    pub attack: Rc<
        dyn Fn(&DamageParticipant, &DamageParticipant, Option<ItemId>, i32, i32, Option<ActorId>) -> AttackProvenance,
    >,
    /// Run an apply while retaining a source call.
    pub dispatch: Rc<dyn Fn(Q3DamageCall, &dyn Fn() -> DamageOutcome) -> DamageOutcome>,
    /// Carrier hurt hook.
    pub check_hurt_carrier: Rc<dyn Fn(EntityRef, EntityRef)>,
    /// Accuracy hit hook.
    pub log_accuracy_hit: Rc<dyn Fn(EntityRef, EntityRef) -> bool>,
    /// Obelisk attack hook (missionpack only).
    pub check_obelisk_attack: Option<Rc<dyn Fn(EntityRef, &DamageParticipant) -> bool>>,
    /// Invulnerability effect hook (missionpack only).
    pub invulnerability_effect: Option<Rc<dyn Fn(EntityRef, Vec3, Vec3)>>,
}

/// Entity pool view (`EntityPool`, game/entities.ts, minimal).
pub trait Q3EntityPool {
    /// Entity count.
    fn num_entities(&self) -> usize;
    /// Entity at an index.
    fn entity_at(&self, index: usize) -> EntityRef;
}

/// Shared entity pool handle.
pub type EntityPoolRef = Rc<dyn Q3EntityPool>;

// ---------------------------------------------------------------------------
// records.ts
// ---------------------------------------------------------------------------

/// Record host services (`Q3RecordHost`).
pub trait Q3RecordHost {
    /// Session actors.
    fn actors(&self) -> Rc<dyn Q3SessionActors>;
    /// Shared bodies.
    fn bodies(&self) -> Rc<dyn Q3SessionBodies>;
    /// Gameplay authority.
    fn combat(&self) -> Rc<dyn Q3SessionCombat>;
    /// Shared inventory.
    fn inventory(&self) -> Rc<dyn Q3SessionInventory>;
    /// Actor callbacks.
    fn callbacks(&self) -> Rc<dyn Q3ActorCallbacks>;
    /// Ammunition timer store notification.
    fn ammo_timer_stored(&self, _actor: &ActorId, _weapon: usize, _value: i32) {}
    /// Schedule an actor think.
    fn schedule(&self, actor: &OwnedActor, due_milliseconds: Option<i32>);
    /// Run an actor think.
    fn run_think(&self, actor: &OwnedActor, time_milliseconds: i32);
    /// Innermost retained source damage call.
    fn damage_call(&self) -> Option<Q3DamageCall>;
    /// Admit damage for an entity.
    fn admit_damage(&self, _entity: EntityRef, _request: &DamageRequest) -> DamageAdmission {
        DamageAdmission::Continue
    }
    /// Project a foreign actor into the source-slot view.
    fn foreign(&self, actor: &ActorId) -> Option<EntityRef>;
    /// Whether an actor is a player.
    fn is_player(&self, actor: &ActorId) -> bool;
}

/// Owned slot state for save capture (`captureOwnership` words).
#[derive(Debug, Clone, PartialEq)]
pub struct SlotOwnership {
    /// Owned actor.
    pub actor: Option<OwnedActor>,
    /// Active.
    pub active: bool,
    /// Borrowed.
    pub borrowed: bool,
}

/// Private client backing snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientBackingSnapshot {
    /// Source stats.
    pub source_stats: [i32; 16],
    /// Special ammunition.
    pub special_ammo: [i32; 16],
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ClientBacking {
    source_stats: [i32; 16],
    special_ammo: [i32; 16],
}

struct SourceRecord {
    entity: EntityRef,
    actor: Option<OwnedActor>,
    active: bool,
    borrowed: bool,
}

struct RecordsInner {
    slots: Vec<SourceRecord>,
    clients: Vec<ClientRef>,
    backing: Vec<ClientBacking>,
    unobserve: Option<Box<dyn Fn()>>,
}

struct RecordsCore {
    host: Rc<dyn Q3RecordHost>,
    provider: ProviderId,
    product: Product,
    inner: RefCell<RecordsInner>,
}

/// `gentity_t` private records (`Q3EntityRecords`).
///
/// Lifetime, body, combat, and inventory come from the session owners;
/// the records own source slots, clients, and private backing.
#[derive(Clone)]
pub struct Q3EntityRecords {
    core: Rc<RecordsCore>,
}

impl std::fmt::Debug for Q3EntityRecords {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Q3EntityRecords")
            .field("provider", &self.core.provider)
            .field("product", &self.core.product)
            .finish()
    }
}

struct RecordBodyBinding {
    core: Weak<RecordsCore>,
    slot: usize,
}

impl EntityBodyBinding for RecordBodyBinding {
    fn read(&self) -> BodyState {
        let Some(core) = self.core.upgrade() else {
            return ZERO_BODY.clone();
        };
        let actor = core.record_actor(self.slot);
        actor.as_ref().map_or_else(
            || ZERO_BODY.clone(),
            |owned| core.host.bodies().read(owned.id()).unwrap_or_else(|| ZERO_BODY.clone()),
        )
    }

    fn write(&self, value: BodyState) {
        if let Some(core) = self.core.upgrade() {
            let actor = core.ensure_actor(self.slot);
            core.host.bodies().write(&actor, value);
        }
    }

    fn linked(&self) -> Option<LinkedBody> {
        let core = self.core.upgrade()?;
        let actor = core.record_actor(self.slot)?;
        core.host.bodies().linked(actor.id())
    }
}

struct RecordEntityBinding {
    core: Weak<RecordsCore>,
    slot: usize,
    body: Rc<dyn EntityBodyBinding>,
}

impl GameEntityBinding for RecordEntityBinding {
    fn body(&self) -> Rc<dyn EntityBodyBinding> {
        self.body.clone()
    }

    fn actor(&self) -> OwnedActor {
        self.core
            .upgrade()
            .unwrap_or_else(|| panic!("Q3 records are closed"))
            .ensure_actor(self.slot)
    }

    fn active(&self) -> bool {
        self.core.upgrade().is_some_and(|core| {
            if !core.record_active(self.slot) {
                return false;
            }
            let actor = core.ensure_actor(self.slot);
            core.host.actors().is_live(actor.id())
        })
    }

    fn health(&self) -> i32 {
        self.core.upgrade().map_or(0, |core| {
            core.record_actor(self.slot).map_or(0, |actor| {
                core.host.combat().read(actor.id()).map_or(0, |state| state.health)
            })
        })
    }

    fn set_health(&self, value: i32) {
        if let Some(core) = self.core.upgrade() {
            let actor = core.ensure_actor(self.slot);
            core.host.combat().set_health(&actor, value);
        }
    }

    fn takes_damage(&self) -> bool {
        self.core.upgrade().is_some_and(|core| {
            core.record_actor(self.slot).is_some_and(|actor| {
                core.host
                    .combat()
                    .read(actor.id())
                    .is_some_and(|state| state.can_take_damage)
            })
        })
    }

    fn set_takes_damage(&self, value: bool) {
        if let Some(core) = self.core.upgrade() {
            let actor = core.ensure_actor(self.slot);
            core.host.combat().set_can_take_damage(&actor, value);
        }
    }

    fn schedule(&self, nextthink: i32) {
        if let Some(core) = self.core.upgrade() {
            if let Some(actor) = core.record_actor(self.slot) {
                core.host
                    .schedule(&actor, if nextthink <= 0 { None } else { Some(nextthink) });
            }
        }
    }

    fn run_think(&self, time_milliseconds: i32) {
        if let Some(core) = self.core.upgrade() {
            let actor = core.ensure_actor(self.slot);
            core.host.run_think(&actor, time_milliseconds);
        }
    }
}

struct RecordStatBinding {
    core: Weak<RecordsCore>,
    slot: usize,
}

impl PlayerSlotBinding for RecordStatBinding {
    fn read(&self, index: usize) -> i32 {
        self.core
            .upgrade()
            .unwrap_or_else(|| panic!("Q3 records are closed"))
            .stat_read(self.slot, index)
    }

    fn write(&self, index: usize, value: i32) {
        if let Some(core) = self.core.upgrade() {
            core.stat_write(self.slot, index, value);
        }
    }
}

struct RecordAmmoBinding {
    core: Weak<RecordsCore>,
    slot: usize,
}

impl PlayerSlotBinding for RecordAmmoBinding {
    fn read(&self, index: usize) -> i32 {
        self.core
            .upgrade()
            .unwrap_or_else(|| panic!("Q3 records are closed"))
            .ammo_read(self.slot, index)
    }

    fn write(&self, index: usize, value: i32) {
        if let Some(core) = self.core.upgrade() {
            core.ammo_write(self.slot, index, value);
        }
    }
}

struct RecordPlayerAuthority {
    core: Weak<RecordsCore>,
    slot: usize,
    stats: Rc<dyn PlayerSlotBinding>,
    ammo: Rc<dyn PlayerSlotBinding>,
}

impl PlayerAuthorityBinding for RecordPlayerAuthority {
    fn origin(&self) -> Vec3 {
        self.core
            .upgrade()
            .map_or_else(|| vec3(0.0, 0.0, 0.0), |core| core.record_body(self.slot).origin)
    }

    fn set_origin(&self, value: Vec3) {
        if let Some(core) = self.core.upgrade() {
            let mut body = core.record_body(self.slot);
            body.origin = value;
            let actor = core.ensure_actor(self.slot);
            core.host.bodies().write(&actor, body);
        }
    }

    fn read_velocity(&self) -> Vec3 {
        self.core
            .upgrade()
            .map_or_else(|| vec3(0.0, 0.0, 0.0), |core| core.record_body(self.slot).velocity)
    }

    fn set_velocity(&self, value: Vec3) {
        if let Some(core) = self.core.upgrade() {
            let mut body = core.record_body(self.slot);
            body.velocity = value;
            let actor = core.ensure_actor(self.slot);
            core.host.bodies().write(&actor, body);
        }
    }

    fn stats(&self) -> Rc<dyn PlayerSlotBinding> {
        self.stats.clone()
    }

    fn ammo(&self) -> Rc<dyn PlayerSlotBinding> {
        self.ammo.clone()
    }
}

impl RecordsCore {
    fn record_entity(self: &Rc<Self>, slot: usize) -> EntityRef {
        self.inner
            .borrow()
            .slots
            .get(slot)
            .unwrap_or_else(|| panic!("Q3 entity {slot} outside 0..1023"))
            .entity
            .clone()
    }

    fn record_actor(self: &Rc<Self>, slot: usize) -> Option<OwnedActor> {
        self.inner
            .borrow()
            .slots
            .get(slot)
            .unwrap_or_else(|| panic!("Q3 entity {slot} outside 0..1023"))
            .actor
            .clone()
    }

    fn record_active(self: &Rc<Self>, slot: usize) -> bool {
        self.inner
            .borrow()
            .slots
            .get(slot)
            .unwrap_or_else(|| panic!("Q3 entity {slot} outside 0..1023"))
            .active
    }

    fn client_ref(self: &Rc<Self>, slot: usize) -> ClientRef {
        self.inner
            .borrow()
            .clients
            .get(slot)
            .unwrap_or_else(|| panic!("Q3 client {slot} outside 0..63"))
            .clone()
    }

    fn record_body(self: &Rc<Self>, slot: usize) -> BodyState {
        let actor = self.record_actor(slot);
        actor.map_or_else(
            || ZERO_BODY.clone(),
            |owned| self.host.bodies().read(owned.id()).unwrap_or_else(|| ZERO_BODY.clone()),
        )
    }

    fn ensure_actor(self: &Rc<Self>, slot: usize) -> OwnedActor {
        if let Some(actor) = self.record_actor(slot) {
            self.host.actors().assert_owned(&actor).expect("Q3 actor ownership");
            return actor;
        }
        let definition = if slot < MAX_CLIENTS { "q3:player" } else { "q3:entity" };
        let actor = self.host.actors().allocate_at_source(&self.provider, slot, definition);
        self.host.bodies().create(&actor, ZERO_BODY.clone());
        self.inner.borrow_mut().slots[slot].actor = Some(actor.clone());
        self.bind_owned_services(slot);
        actor
    }

    fn admit_fn(self: &Rc<Self>, slot: usize) -> DamageAdmissionFn {
        let host = self.host.clone();
        let entity = self.record_entity(slot);
        Rc::new(move |request| host.admit_damage(entity.clone(), request))
    }

    fn bind_owned_services(self: &Rc<Self>, slot: usize) {
        let Some(actor) = self.record_actor(slot) else {
            panic!("Cannot bind Q3 services without an actor");
        };
        if self.host.combat().read(actor.id()).is_none() {
            self.host.combat().create(
                &actor,
                CombatState {
                    health: 0,
                    armor: ArmorState {
                        regular: RegularArmorState::Q3 {
                            points: 0,
                            protection: 0.66,
                        },
                        powered: PoweredProtectionState::None,
                    },
                    mass: 200,
                    can_take_damage: false,
                    invulnerable: false,
                    no_knockback: false,
                    team: None,
                },
                Some(self.admit_fn(slot)),
            );
        } else {
            self.host.combat().bind_damage_admission(&actor, self.admit_fn(slot));
        }
        if !self.host.inventory().has(actor.id()) {
            self.host.inventory().create(&actor, Vec::new());
        }
        self.bind_callbacks(slot);
    }

    fn native_by_actor(self: &Rc<Self>, actor: Option<&ActorId>) -> Option<EntityRef> {
        let actor = actor?;
        self.inner
            .borrow()
            .slots
            .iter()
            .find(|record| record.actor.as_ref().is_some_and(|owned| owned.id() == actor))
            .map(|record| record.entity.clone())
    }

    fn by_actor(self: &Rc<Self>, actor: Option<&ActorId>) -> Option<EntityRef> {
        let actor = actor?;
        if let Some(native) = self.native_by_actor(Some(actor)) {
            return Some(native);
        }
        self.host.foreign(actor)
    }

    fn damage_inflictor(self: &Rc<Self>, actor: Option<&ActorId>) -> DamageParticipant {
        let Some(actor) = actor else {
            return DamageParticipant::Native(self.record_entity(ENTITYNUM_WORLD as usize));
        };
        if let Some(native) = self.native_by_actor(Some(actor)) {
            return DamageParticipant::Native(native);
        }
        let host = self.host.clone();
        let actor = actor.clone();
        DamageParticipant::SharedActor(SharedParticipant::new(
            actor.clone(),
            Rc::new(move || {
                let body = host.bodies().read(&actor)?;
                if host.actors().is_live(&actor) {
                    Some(body.origin)
                } else {
                    None
                }
            }),
        ))
    }

    fn use_participant(self: &Rc<Self>, actor: Option<&ActorId>) -> Option<DamageParticipant> {
        actor.map(|actor| self.damage_inflictor(Some(actor)))
    }

    fn bind_callbacks(self: &Rc<Self>, slot: usize) {
        let Some(actor) = self.record_actor(slot) else {
            panic!("Cannot bind inactive Q3 record callbacks");
        };
        let entity = self.record_entity(slot);
        let host = self.host.clone();
        let think_entity = entity.clone();
        let think = Rc::new(move || {
            let callback = {
                let mut borrowed = think_entity.borrow_mut();
                borrowed.set_nextthink(0);
                borrowed.think.clone()
            };
            let Some(callback) = callback else {
                panic!("NULL ent->think");
            };
            callback(think_entity.clone());
        });
        let touch_entity = entity.clone();
        let touch_core = Rc::downgrade(self);
        let touch = Rc::new(move |contact: &TouchContact| {
            let callback = touch_entity.borrow().touch.clone();
            let Some(callback) = callback else {
                return;
            };
            let other = touch_core.upgrade().map_or_else(
                || DamageParticipant::SharedActor(SharedParticipant::new(contact.other.clone(), Rc::new(|| None))),
                |core| core.damage_inflictor(Some(&contact.other)),
            );
            // Native entities resolve through the inflictor's native
            // branch, matching the donor's slot lookup.
            callback(touch_entity.clone(), other, contact.clone());
        });
        let use_entity = entity.clone();
        let use_core = Rc::downgrade(self);
        let use_action = Rc::new(move |other: Option<ActorId>, activator: Option<ActorId>| {
            let callback = use_entity.borrow().use_action.clone();
            if let Some(callback) = callback {
                let (other, activator) = use_core.upgrade().map_or((None, None), |core| {
                    (
                        core.use_participant(other.as_ref()),
                        core.use_participant(activator.as_ref()),
                    )
                });
                callback(use_entity.clone(), other, activator);
            }
        });
        let pain_entity = entity.clone();
        let pain_core = Rc::downgrade(self);
        let pain = Rc::new(move |reaction: &PainReaction| {
            let callback = pain_entity.borrow().pain.clone();
            if let Some(callback) = callback {
                let other = pain_core.upgrade().map_or_else(
                    || {
                        DamageParticipant::SharedActor(SharedParticipant::new(
                            reaction
                                .attacker
                                .clone()
                                .unwrap_or_else(|| use_actor(&DamageParticipant::Native(pain_entity.clone()))),
                            Rc::new(|| None),
                        ))
                    },
                    |core| core.damage_inflictor(reaction.attacker.as_ref()),
                );
                callback(pain_entity.clone(), other, reaction.damage);
            }
        });
        let die_entity = entity.clone();
        let die_core = Rc::downgrade(self);
        let die_host = host.clone();
        let die = Rc::new(move |reaction: &DeathReaction| {
            let call = die_host.damage_call();
            let callback = die_entity.borrow().die.clone();
            let Some(callback) = callback else {
                panic!("G_Damage lethal target has no die callback");
            };
            let method_of_death = call.map_or_else(
                || match &reaction.attack {
                    Some(attack) => match &attack.cause {
                        AttackCause::Q3 { means_of_death, .. } => *means_of_death,
                        _ => 0,
                    },
                    None => 0,
                },
                |call| call.method_of_death,
            );
            let (inflictor, attacker) = die_core.upgrade().map_or_else(
                || {
                    (
                        DamageParticipant::Native(die_entity.clone()),
                        DamageParticipant::Native(die_entity.clone()),
                    )
                },
                |core| {
                    (
                        core.damage_inflictor(reaction.inflictor.as_ref()),
                        core.damage_inflictor(reaction.attacker.as_ref()),
                    )
                },
            );
            callback(
                die_entity.clone(),
                inflictor,
                attacker,
                reaction.damage,
                method_of_death,
            );
        });
        host.callbacks().bind(
            &actor,
            ActorCallbacks {
                think,
                touch,
                use_action,
                pain,
                die,
            },
        );
    }

    fn stat_read(self: &Rc<Self>, slot: usize, index: usize) -> i32 {
        let schema = stat_schema(self.product);
        let indices = *schema.indices();
        if index == indices.health {
            return self.record_entity(slot).borrow().health();
        }
        if index == indices.armor {
            let actor = self.record_actor(slot);
            return actor.map_or(0, |owned| {
                match self.host.combat().read(owned.id()).map(|state| state.armor.regular) {
                    Some(RegularArmorState::Q3 { points, .. })
                    | Some(RegularArmorState::Q1 { points, .. })
                    | Some(RegularArmorState::Q2 { points, .. })
                    | Some(RegularArmorState::Source { points, .. }) => points,
                    Some(RegularArmorState::None) | None => 0,
                }
            });
        }
        if index == indices.weapons {
            let Some(actor) = self.record_actor(slot) else {
                return 0;
            };
            return q3_weapon_items().iter().fold(0, |bits, weapon| {
                bits | i32::from(self.host.inventory().count(actor.id(), &weapon.item) > 0) << (weapon.weapon as i32)
            });
        }
        *self
            .inner
            .borrow()
            .backing
            .get(slot)
            .unwrap_or_else(|| panic!("Q3 client backing slot outside 0..63"))
            .source_stats
            .get(index)
            .unwrap_or_else(|| panic!("Q3 stat {index} outside 0..15"))
    }

    fn stat_write(self: &Rc<Self>, slot: usize, index: usize, value: i32) {
        let schema = stat_schema(self.product);
        let indices = *schema.indices();
        if index == indices.health {
            let actor = self.ensure_actor(slot);
            self.host.combat().set_health(&actor, value);
            return;
        }
        if index == indices.armor {
            let actor = self.ensure_actor(slot);
            self.host.combat().set_regular_points(
                &actor,
                value,
                RegularArmorState::Q3 {
                    points: value,
                    protection: 0.66,
                },
            );
            return;
        }
        if index == indices.weapons {
            let actor = self.ensure_actor(slot);
            for weapon in q3_weapon_items() {
                self.host.inventory().configure(
                    &actor,
                    &weapon.item,
                    i32::from(value & (1 << (weapon.weapon as i32)) != 0),
                    1,
                );
            }
            return;
        }
        if index >= 16 {
            panic!("Q3 stat {index} outside 0..15");
        }
        self.inner.borrow_mut().backing[slot].source_stats[index] = value;
    }

    fn ammo_read(self: &Rc<Self>, slot: usize, index: usize) -> i32 {
        if let Some(weapon) = index
            .try_into()
            .ok()
            .and_then(q3_weapon_item)
            .and_then(|weapon| weapon.ammo.clone())
        {
            return self
                .record_actor(slot)
                .map_or(0, |actor| self.host.inventory().count(actor.id(), &weapon));
        }
        *self
            .inner
            .borrow()
            .backing
            .get(slot)
            .unwrap_or_else(|| panic!("Q3 client backing slot outside 0..63"))
            .special_ammo
            .get(index)
            .unwrap_or_else(|| panic!("Q3 ammo {index} outside 0..15"))
    }

    fn ammo_write(self: &Rc<Self>, slot: usize, index: usize, value: i32) {
        if let Some(weapon) = index
            .try_into()
            .ok()
            .and_then(q3_weapon_item)
            .and_then(|weapon| weapon.ammo.clone())
        {
            let actor = self.ensure_actor(slot);
            self.host.inventory().configure(&actor, &weapon, value, 200);
            return;
        }
        if index >= 16 {
            panic!("Q3 ammo {index} outside 0..15");
        }
        self.inner.borrow_mut().backing[slot].special_ammo[index] = value;
    }
}

impl Q3EntityRecords {
    /// Records over a session host, provider, and product.
    #[must_use]
    pub fn new(host: Rc<dyn Q3RecordHost>, provider: ProviderId, product: Product) -> Self {
        let core = Rc::new_cyclic(|weak: &Weak<RecordsCore>| {
            let mut slots = Vec::with_capacity(MAX_GENTITIES);
            for slot in 0..MAX_GENTITIES {
                let body: Rc<dyn EntityBodyBinding> = Rc::new(RecordBodyBinding {
                    core: weak.clone(),
                    slot,
                });
                let binding: Rc<dyn GameEntityBinding> = Rc::new(RecordEntityBinding {
                    core: weak.clone(),
                    slot,
                    body,
                });
                slots.push(SourceRecord {
                    entity: Rc::new(RefCell::new(GameEntity::new(slot, binding))),
                    actor: None,
                    active: false,
                    borrowed: false,
                });
            }
            let backing = vec![
                ClientBacking {
                    source_stats: [0; 16],
                    special_ammo: [0; 16],
                };
                MAX_CLIENTS
            ];
            let mut clients = Vec::with_capacity(MAX_CLIENTS);
            for slot in 0..MAX_CLIENTS {
                let stats: Rc<dyn PlayerSlotBinding> = Rc::new(RecordStatBinding {
                    core: weak.clone(),
                    slot,
                });
                let ammo: Rc<dyn PlayerSlotBinding> = Rc::new(RecordAmmoBinding {
                    core: weak.clone(),
                    slot,
                });
                let authority: Rc<dyn PlayerAuthorityBinding> = Rc::new(RecordPlayerAuthority {
                    core: weak.clone(),
                    slot,
                    stats,
                    ammo,
                });
                let notify = weak.clone();
                let notify_host = host.clone();
                let stored: Rc<dyn Fn(usize, i32)> = Rc::new(move |weapon, value| {
                    if let Some(core) = notify.upgrade() {
                        if let Some(actor) = core.record_actor(slot) {
                            notify_host.ammo_timer_stored(actor.id(), weapon, value);
                        }
                    }
                });
                clients.push(Rc::new(RefCell::new(GameClient::new(
                    product,
                    Some(authority),
                    Some(stored),
                ))));
            }
            RecordsCore {
                host,
                provider,
                product,
                inner: RefCell::new(RecordsInner {
                    slots,
                    clients,
                    backing,
                    unobserve: None,
                }),
            }
        });
        let release = Rc::downgrade(&core);
        let unobserve = core.host.actors().on_release(Box::new(move |actor| {
            if let Some(core) = release.upgrade() {
                let mut inner = core.inner.borrow_mut();
                for record in &mut inner.slots {
                    if record.actor.as_ref() == Some(actor) {
                        record.actor = None;
                        record.active = false;
                    }
                }
            }
        }));
        core.inner.borrow_mut().unobserve = Some(unobserve);
        Self { core }
    }

    /// Session host.
    #[must_use]
    pub fn host(&self) -> Rc<dyn Q3RecordHost> {
        self.core.host.clone()
    }

    /// Owning provider.
    #[must_use]
    pub fn provider(&self) -> &ProviderId {
        &self.core.provider
    }

    /// Product.
    #[must_use]
    pub fn product(&self) -> Product {
        self.core.product
    }

    /// Stop observing actor release.
    pub fn close(&self) {
        if let Some(unobserve) = self.core.inner.borrow_mut().unobserve.take() {
            unobserve();
        }
    }

    /// Entity at a slot.
    #[must_use]
    pub fn get(&self, slot: usize) -> Option<EntityRef> {
        self.core
            .inner
            .borrow()
            .slots
            .get(slot)
            .map(|record| record.entity.clone())
    }

    /// Client at a slot.
    ///
    /// # Panics
    ///
    /// Panics when `slot` is outside `0..63`.
    #[must_use]
    pub fn client(&self, slot: usize) -> ClientRef {
        self.core.client_ref(slot)
    }

    /// Capture slot ownership words.
    #[must_use]
    pub fn capture_ownership(&self) -> Vec<SlotOwnership> {
        self.core
            .inner
            .borrow()
            .slots
            .iter()
            .map(|record| SlotOwnership {
                actor: record.actor.clone(),
                active: record.active,
                borrowed: record.borrowed,
            })
            .collect()
    }

    /// Restore slot ownership words into fresh records.
    pub fn restore_ownership(&self, states: &[SlotOwnership]) -> Result<(), Q3BaseError> {
        if states.len() != MAX_GENTITIES {
            return Err(Q3BaseError::Invalid(
                "Restored Q3 ownership requires all retained slots".to_string(),
            ));
        }
        let mut seen: Vec<&OwnedActor> = Vec::new();
        for state in states {
            if let Some(actor) = &state.actor {
                self.core.host.actors().assert_owned(actor)?;
                if seen.contains(&actor) {
                    return Err(Q3BaseError::Invalid("Duplicate restored Q3 actor".to_string()));
                }
                seen.push(actor);
                if !state.borrowed && actor.owner() != &self.core.provider {
                    return Err(Q3BaseError::Invalid(
                        "Q3 owned actor has a foreign provider".to_string(),
                    ));
                }
            } else if state.active {
                return Err(Q3BaseError::Invalid("Active Q3 slot has no actor".to_string()));
            }
        }
        let mut inner = self.core.inner.borrow_mut();
        for (slot, state) in states.iter().enumerate() {
            let record = &mut inner.slots[slot];
            if record.actor.is_some() {
                return Err(Q3BaseError::Invalid(
                    "Q3 ownership hydration requires fresh records".to_string(),
                ));
            }
            record.actor = state.actor.clone();
            record.active = state.active;
            record.borrowed = state.borrowed;
        }
        Ok(())
    }

    /// Rebind callbacks for restored owned records.
    pub fn restore_callbacks(&self) {
        let owned: Vec<usize> = self
            .core
            .inner
            .borrow()
            .slots
            .iter()
            .enumerate()
            .filter(|(_, record)| record.actor.is_some() && !record.borrowed)
            .map(|(slot, _)| slot)
            .collect();
        for slot in owned {
            if let Some(actor) = self.core.record_actor(slot) {
                self.core
                    .host
                    .combat()
                    .bind_damage_admission(&actor, self.core.admit_fn(slot));
            }
            self.core.bind_callbacks(slot);
        }
    }

    /// Capture private client backing.
    ///
    /// # Panics
    ///
    /// Panics when `slot` is outside `0..63`.
    #[must_use]
    pub fn capture_client_backing(&self, slot: usize) -> ClientBackingSnapshot {
        let inner = self.core.inner.borrow();
        let Some(backing) = inner.backing.get(slot) else {
            panic!("Q3 client backing slot outside 0..63");
        };
        ClientBackingSnapshot {
            source_stats: backing.source_stats,
            special_ammo: backing.special_ammo,
        }
    }

    /// Restore private client backing.
    pub fn restore_client_backing(&self, slot: usize, state: &ClientBackingSnapshot) -> Result<(), Q3BaseError> {
        let mut inner = self.core.inner.borrow_mut();
        let Some(backing) = inner.backing.get_mut(slot) else {
            return Err(Q3BaseError::Invalid("Invalid Q3 private client backing".to_string()));
        };
        backing.source_stats = state.source_stats;
        backing.special_ammo = state.special_ammo;
        Ok(())
    }

    /// Restore an existing owned actor without reallocating its identity
    /// or body.
    pub fn adopt(&self, slot: usize, actor: OwnedActor) -> Result<EntityRef, Q3BaseError> {
        self.core.host.actors().assert_owned(&actor)?;
        let valid = slot >= MAX_CLIENTS
            && slot < ENTITYNUM_WORLD as usize
            && actor.owner() == &self.core.provider
            && self.core.record_actor(slot).is_none()
            && self.core.native_by_actor(Some(actor.id())).is_none()
            && self.core.host.bodies().read(actor.id()).is_some();
        if !valid {
            return Err(Q3BaseError::Invalid(
                "Q3 owned actor adoption requires an unused entity slot and its existing body".to_string(),
            ));
        }
        {
            let mut inner = self.core.inner.borrow_mut();
            let record = &mut inner.slots[slot];
            record.actor = Some(actor);
            record.active = true;
            record.borrowed = false;
        }
        let entity = self.core.record_entity(slot);
        entity.borrow_mut().s.number = slot as i32;
        self.core.bind_owned_services(slot);
        Ok(entity)
    }

    /// Attach an already admitted actor without creating another actor
    /// or changing its callbacks.
    pub fn attach(&self, slot: usize, actor: OwnedActor, player: bool) -> Result<EntityRef, Q3BaseError> {
        self.core.host.actors().assert_owned(&actor)?;
        {
            let mut inner = self.core.inner.borrow_mut();
            let Some(record) = inner.slots.get_mut(slot) else {
                panic!("Q3 entity {slot} outside 0..1023");
            };
            if record.actor.as_ref().is_some_and(|owned| *owned != actor) {
                return Err(Q3BaseError::Invalid(format!("Q3 slot {slot} already has an actor")));
            }
            record.actor = Some(actor);
            record.active = true;
            record.borrowed = true;
        }
        let entity = self.core.record_entity(slot);
        if player {
            let client = self.core.client_ref(slot);
            entity.borrow_mut().client = Some(client);
        }
        entity.borrow_mut().s.number = slot as i32;
        Ok(entity)
    }

    /// Activate a slot, allocating its actor.
    ///
    /// # Panics
    ///
    /// Panics when `slot` is outside `0..1023`.
    #[must_use]
    pub fn activate(&self, slot: usize) -> EntityRef {
        if slot >= MAX_GENTITIES {
            panic!("Q3 entity {slot} outside 0..1023");
        }
        self.core.ensure_actor(slot);
        self.core.inner.borrow_mut().slots[slot].active = true;
        self.core.record_entity(slot)
    }

    /// Deactivate a client slot, releasing its owned actor.
    ///
    /// # Panics
    ///
    /// Panics when `slot` is outside `0..1023`.
    pub fn deactivate_client(&self, slot: usize) {
        if slot >= MAX_GENTITIES {
            panic!("Q3 entity {slot} outside 0..1023");
        }
        let actor = self.core.record_actor(slot);
        let borrowed = self.core.inner.borrow().slots[slot].borrowed;
        if let Some(actor) = actor {
            if !borrowed {
                self.core.host.actors().release(&actor);
            }
        }
        let mut inner = self.core.inner.borrow_mut();
        inner.slots[slot].actor = None;
        inner.slots[slot].active = false;
        inner.slots[slot].borrowed = false;
    }

    /// Release an entity and reset its source metadata in place.
    ///
    /// # Panics
    ///
    /// Panics when the entity belongs to another record owner.
    pub fn release(&self, entity: EntityRef) {
        let slot = entity.borrow().slot;
        let owned = self.core.record_entity(slot);
        if !Rc::ptr_eq(&owned, &entity) {
            panic!("Q3 entity belongs to another record owner");
        }
        let actor = self.core.record_actor(slot);
        let borrowed = self.core.inner.borrow().slots[slot].borrowed;
        if let Some(actor) = actor {
            if !borrowed {
                self.core.host.actors().release(&actor);
            }
        }
        {
            let mut inner = self.core.inner.borrow_mut();
            inner.slots[slot].actor = None;
            inner.slots[slot].active = false;
            inner.slots[slot].borrowed = false;
        }
        entity.borrow_mut().reset();
    }

    /// Native entity for an actor.
    #[must_use]
    pub fn native_by_actor(&self, actor: Option<&ActorId>) -> Option<EntityRef> {
        self.core.native_by_actor(actor)
    }

    /// Native or foreign entity for an actor.
    #[must_use]
    pub fn by_actor(&self, actor: Option<&ActorId>) -> Option<EntityRef> {
        self.core.by_actor(actor)
    }

    /// Use participant for an actor.
    #[must_use]
    pub fn use_participant(&self, actor: Option<&ActorId>) -> Option<DamageParticipant> {
        self.core.use_participant(actor)
    }

    /// Damage inflictor for an actor.
    #[must_use]
    pub fn damage_inflictor(&self, actor: Option<&ActorId>) -> DamageParticipant {
        self.core.damage_inflictor(actor)
    }
}

// ---------------------------------------------------------------------------
// combat-bridge.ts
// ---------------------------------------------------------------------------

/// Combat bridge host services (`Q3CombatBridgeHost`).
///
/// Missionpack-only hooks keep defaults that baseq3 never calls, matching
/// the donor's product-discriminated union.
pub trait Q3CombatBridgeHost {
    /// Gameplay authority.
    fn authority(&self) -> Rc<dyn Q3SessionCombat>;
    /// Entity pool.
    fn entities(&self) -> EntityPoolRef;
    /// Entity records.
    fn records(&self) -> Q3EntityRecords;
    /// Server world with actor queries.
    fn world(&self) -> Rc<dyn Q3ServerWorld>;
    /// Weapon provider.
    fn weapon_provider(&self) -> ProviderId;
    /// Damage powerup owner override.
    fn damage_powerup_owner(&self) -> Option<ProviderId> {
        None
    }
    /// Source damage modifier.
    fn source_damage_modifier(&self) -> Option<SourceDamageModifier> {
        None
    }
    /// Combat provider.
    fn combat_provider(&self) -> ProviderId;
    /// Inventory provider.
    fn inventory_provider(&self) -> ProviderId;
    /// Movement provider.
    fn movement_provider(&self) -> ProviderId;
    /// Victim armor context.
    fn armor_context(&self, request: &DamageRequest) -> VictimArmorContext;
    /// Current time.
    fn time(&self) -> i32;
    /// Queued intermission.
    fn intermission_queued(&self) -> i32;
    /// Game type tag.
    fn game_type(&self) -> i32;
    /// Friendly fire.
    fn friendly_fire(&self) -> bool;
    /// Knockback scale.
    fn knockback(&self) -> f32;
    /// Product.
    fn product(&self) -> Product;
    /// Damage debug sink.
    fn debug_damage(&self, _diagnostic: &DamageDiagnostic) {}
    /// Carrier hurt hook.
    fn check_hurt_carrier(&self, target: EntityRef, attacker: EntityRef);
    /// Accuracy hit hook.
    fn log_accuracy_hit(&self, target: EntityRef, attacker: EntityRef) -> bool;
    /// Source damage feedback (`q3DamageFeedback`, game/combat.ts).
    fn damage_feedback(&self, call: &Q3DamageCall, decision: &DamageDecision);
    /// Foreign damage feedback (`q3ForeignDamageFeedback`, game/combat.ts).
    fn foreign_damage_feedback(&self, target: EntityRef, owner: Option<EntityRef>, decision: &DamageDecision);
    /// Projectile parent (missionpack only).
    fn projectile_parent(&self, _actor: &ActorId) -> Option<ActorId> {
        None
    }
    /// Obelisk attack hook (missionpack only).
    fn check_obelisk_attack(&self, _target: EntityRef, _attacker: EntityRef) -> bool {
        false
    }
    /// Invulnerability effect hook (missionpack only).
    fn invulnerability_effect(&self, _target: EntityRef, _direction: Vec3, _point: Vec3) {}
}

struct DamageCallGuard {
    calls: Rc<RefCell<Vec<Q3DamageCall>>>,
}

impl Drop for DamageCallGuard {
    fn drop(&mut self) {
        self.calls.borrow_mut().pop();
    }
}

/// Source combat context and feedback over the shared damage authority
/// (`Q3CombatBridge`).
#[derive(Clone)]
pub struct Q3CombatBridge {
    host: Rc<dyn Q3CombatBridgeHost>,
    context: CombatContext,
    calls: Rc<RefCell<Vec<Q3DamageCall>>>,
    sequence: Rc<Cell<i32>>,
}

impl std::fmt::Debug for Q3CombatBridge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Q3CombatBridge")
            .field("product", &self.context.product)
            .finish()
    }
}

struct ProjectedCurrent<'a> {
    bridge: Q3CombatBridge,
    request: DamageRequest,
    inner: &'a dyn CurrentCombatState,
}

impl CurrentCombatState for ProjectedCurrent<'_> {
    fn target(&self) -> Option<CombatState> {
        self.inner
            .target()
            .map(|state| self.bridge.source_state(&self.request, &state, false))
    }

    fn attacker(&self) -> Option<CombatState> {
        self.inner
            .attacker()
            .map(|state| self.bridge.source_state(&self.request, &state, true))
    }
}

impl Q3CombatBridge {
    /// Bridge over a host, building the shared combat context.
    #[must_use]
    pub fn new(host: Rc<dyn Q3CombatBridgeHost>) -> Self {
        let calls: Rc<RefCell<Vec<Q3DamageCall>>> = Rc::new(RefCell::new(Vec::new()));
        let sequence: Rc<Cell<i32>> = Rc::new(Cell::new(0));
        let records = host.records();
        let product = host.product();

        let actors_is_live_records = records.clone();
        let is_live: Rc<dyn Fn(&ActorId) -> bool> =
            Rc::new(move |actor| actors_is_live_records.host().actors().is_live(actor));
        let participant_records = records.clone();
        let participant: Rc<dyn Fn(&ActorId) -> DamageParticipant> =
            Rc::new(move |actor| participant_records.damage_inflictor(Some(actor)));
        let parent_host = host.clone();
        let parent: Rc<dyn Fn(&ActorId) -> Option<ActorId>> = Rc::new(move |actor| {
            if parent_host.product() == Product::Missionpack {
                parent_host.projectile_parent(actor)
            } else {
                None
            }
        });
        let bounds_records = records.clone();
        let linked_bounds: Rc<dyn Fn(&ActorId) -> Option<Bounds>> = Rc::new(move |actor| {
            bounds_records
                .host()
                .bodies()
                .linked(actor)
                .map(|linked| linked.absolute_bounds)
        });
        let player_records = records.clone();
        let is_player: Rc<dyn Fn(&ActorId) -> bool> = Rc::new(move |actor| {
            if let Some(native) = player_records.native_by_actor(Some(actor)) {
                native.borrow().client.is_some()
            } else {
                player_records.host().is_player(actor)
            }
        });

        let time_host = host.clone();
        let intermission_host = host.clone();
        let game_type_host = host.clone();
        let friendly_fire_host = host.clone();
        let knockback_host = host.clone();
        let debug_host = host.clone();
        let debug_damage: Option<Rc<dyn Fn(DamageDiagnostic)>> = Some(Rc::new(move |diagnostic| {
            debug_host.debug_damage(&diagnostic);
        }));

        let attack_host = host.clone();
        let attack_sequence = sequence.clone();
        let attack: Rc<
            dyn Fn(
                &DamageParticipant,
                &DamageParticipant,
                Option<ItemId>,
                i32,
                i32,
                Option<ActorId>,
            ) -> AttackProvenance,
        > = Rc::new(
            move |inflictor, attacker, weapon, means_of_death, flags, originating_projectile| {
                let order = attack_sequence.get();
                attack_sequence.set(order.wrapping_add(1));
                let inflictor_weapon = match inflictor {
                    DamageParticipant::Native(entity) => entity.borrow().s.weapon,
                    DamageParticipant::SharedActor(_) => 0,
                };
                let attacker_weapon = match attacker {
                    DamageParticipant::Native(entity) => entity.borrow().s.weapon,
                    DamageParticipant::SharedActor(_) => 0,
                };
                let fallback = if inflictor_weapon != 0 {
                    inflictor_weapon
                } else {
                    attacker_weapon
                };
                AttackProvenance {
                    sequence: order,
                    time: SourceTime::Milliseconds(attack_host.time()),
                    attacker: Some(use_actor(attacker)),
                    inflictor: Some(use_actor(inflictor)),
                    originating_projectile,
                    weapon: weapon.or_else(|| q3_weapon_item(fallback).map(|entry| entry.item.clone())),
                    weapon_provider: attack_host.weapon_provider(),
                    damage_powerup_owner: Some(
                        attack_host
                            .damage_powerup_owner()
                            .unwrap_or_else(|| attack_host.weapon_provider()),
                    ),
                    combat_provider: attack_host.combat_provider(),
                    inventory_provider: attack_host.inventory_provider(),
                    movement_provider: attack_host.movement_provider(),
                    cause: AttackCause::Q3 {
                        means_of_death,
                        damage_flags: flags,
                    },
                }
            },
        );

        let dispatch_calls = calls.clone();
        let dispatch: Rc<dyn Fn(Q3DamageCall, &dyn Fn() -> DamageOutcome) -> DamageOutcome> =
            Rc::new(move |call, operation| {
                dispatch_calls.borrow_mut().push(call);
                let _guard = DamageCallGuard {
                    calls: dispatch_calls.clone(),
                };
                operation()
            });

        let carrier_host = host.clone();
        let accuracy_host = host.clone();

        let (check_obelisk_attack, invulnerability_effect) = if product == Product::Missionpack {
            let obelisk_host = host.clone();
            let obelisk_records = records.clone();
            let check: Rc<dyn Fn(EntityRef, &DamageParticipant) -> bool> = Rc::new(move |target, attacker| {
                let native = match attacker {
                    DamageParticipant::Native(entity) => Some(entity.clone()),
                    DamageParticipant::SharedActor(shared) => obelisk_records.native_by_actor(Some(&shared.actor)),
                };
                if native.is_none() && obelisk_records.host().is_player(&use_actor(attacker)) {
                    panic!("Admitted Q3 map player has no native client behavior record");
                }
                native.is_some_and(|native| obelisk_host.check_obelisk_attack(target, native))
            });
            let effect_host = host.clone();
            let effect: Rc<dyn Fn(EntityRef, Vec3, Vec3)> = Rc::new(move |target, direction, point| {
                effect_host.invulnerability_effect(target, direction, point);
            });
            (Some(check), Some(effect))
        } else {
            (None, None)
        };

        let context = CombatContext {
            product,
            authority: host.authority(),
            entities: host.entities(),
            spatial: host.world(),
            source_damage_modifier: host.source_damage_modifier(),
            actors: CombatActors {
                is_live,
                participant,
                parent,
                linked_bounds,
                is_player,
            },
            time: Rc::new(move || time_host.time()),
            intermission_queued: Rc::new(move || intermission_host.intermission_queued()),
            game_type: Rc::new(move || game_type_host.game_type()),
            friendly_fire: Rc::new(move || friendly_fire_host.friendly_fire()),
            knockback: Rc::new(move || knockback_host.knockback()),
            debug_damage,
            attack,
            dispatch,
            check_hurt_carrier: Rc::new(move |target, attacker| {
                carrier_host.check_hurt_carrier(target, attacker);
            }),
            log_accuracy_hit: Rc::new(move |target, attacker| accuracy_host.log_accuracy_hit(target, attacker)),
            check_obelisk_attack,
            invulnerability_effect,
        };
        Self {
            host,
            context,
            calls,
            sequence,
        }
    }

    /// Shared combat context.
    #[must_use]
    pub fn context(&self) -> &CombatContext {
        &self.context
    }

    /// Capture the sequence save word.
    ///
    /// # Panics
    ///
    /// Panics while a combat call is retained.
    #[must_use]
    pub fn capture_save_state(&self) -> SaveJson {
        if !self.calls.borrow().is_empty() {
            panic!("Cannot save Q3 during a combat call");
        }
        obj(vec![("sequence", int(i64::from(self.sequence.get())))])
    }

    /// Restore the sequence save word.
    pub fn restore_save_state(&self, value: &SaveJson) -> Result<(), ValueError> {
        if !self.calls.borrow().is_empty() {
            panic!("Cannot restore Q3 during a combat call");
        }
        let sequence = SaveReader::at(value, "q3.combatBridge").field("sequence").integer(0)?;
        self.sequence.set(sequence as i32);
        Ok(())
    }

    /// Innermost retained source call.
    #[must_use]
    pub fn current_call(&self) -> Option<Q3DamageCall> {
        self.calls.borrow().last().cloned()
    }

    /// Shared `GameplayAuthority.beforeReaction` hook, called before
    /// dispatching actor callbacks.
    pub fn before_reaction(&self, decision: &DamageDecision) {
        let current = self.current_call();
        if let AttackCause::Q3 { .. } = decision.request.attack.cause {
            if let Some(call) = &current {
                if call.target.borrow().actor().id().clone() == decision.request.target {
                    self.host.damage_feedback(call, decision);
                    return;
                }
            }
        }
        let records = self.host.records();
        let target = records.native_by_actor(Some(&decision.request.target));
        let attacker = decision.request.attack.attacker.clone();
        let owner = attacker
            .as_ref()
            .and_then(|attacker| records.native_by_actor(Some(attacker)));
        if let Some(target) = target {
            self.host.foreign_damage_feedback(target, owner, decision);
        }
    }

    /// Combat policy for the selected provider; victims retain their own
    /// armor policy.
    #[must_use]
    pub fn policy(&self) -> CombatPolicy {
        let host = self.host.clone();
        let armor_host = host.clone();
        let armor = native_victim_armor(Rc::new(move |request| armor_host.armor_context(request)));
        let context_bridge = self.clone();
        let context = Rc::new(
            move |request: &DamageRequest, target: &CombatState, attacker: Option<&CombatState>| {
                context_bridge.policy_context(request, target, attacker)
            },
        );
        let inner = create_q3_combat_policy(host.combat_provider(), armor, context);
        let bridge = self.clone();
        CombatPolicy {
            id: inner.id.clone(),
            decide: Rc::new(move |request, target, attacker| {
                let projected_target = bridge.source_state(request, target, false);
                let projected_attacker = attacker.map(|state| bridge.source_state(request, state, true));
                let progress = (inner.decide)(request, &projected_target, projected_attacker.as_ref());
                bridge.progress(progress)
            }),
        }
    }

    fn policy_context(
        &self,
        request: &DamageRequest,
        _target: &CombatState,
        _attacker: Option<&CombatState>,
    ) -> Q3CombatContext {
        let host = &self.host;
        let records = host.records();
        let target = records.native_by_actor(Some(&request.target));
        let owner = request
            .attack
            .attacker
            .as_ref()
            .and_then(|attacker| records.native_by_actor(Some(attacker)));
        let target_client = target.as_ref().and_then(|entity| entity.borrow().client.clone());
        let owner_client = owner.as_ref().and_then(|entity| entity.borrow().client.clone());
        let method = match &request.attack.cause {
            AttackCause::Q3 { means_of_death, .. } => *means_of_death,
            _ => -1,
        };
        let parent_actor =
            if host.product() == Product::Missionpack && method == 25 && request.attack.inflictor.is_some() {
                request
                    .attack
                    .inflictor
                    .as_ref()
                    .and_then(|inflictor| (self.context.actors.parent)(inflictor))
            } else {
                None
            };
        let parent = records.native_by_actor(parent_actor.as_ref());
        let schema = stat_schema(host.product());
        let guard = owner_client.as_ref().is_some_and(|client| {
            schema.product() == Product::Missionpack
                && schema.persistent_powerup().is_some_and(|slot| {
                    item_at(Product::Missionpack, client.borrow().ps.stats.get(slot))
                        .map(|item| item.powerup_tag() == Some(Powerup::PwGuard))
                        .unwrap_or(false)
                })
        });
        let target_borrow = target_client.as_ref().map(|client| client.borrow());
        let owner_borrow = owner_client.as_ref().map(|client| client.borrow());
        Q3CombatContext {
            player: target_client.is_some(),
            attacker_player: owner_client.is_some(),
            attacker_max_health: owner_borrow
                .as_ref()
                .map_or(100, |client| client.ps.stats.get(schema.indices().max_health)),
            attacker_guard: guard,
            intermission: host.intermission_queued() != 0,
            noclip: target_borrow.as_ref().is_some_and(|client| client.noclip),
            missionpack_invulnerability: host.product() == Product::Missionpack
                && target_borrow
                    .as_ref()
                    .is_some_and(|client| client.invulnerability_time > host.time()),
            no_knockback: target
                .as_ref()
                .is_some_and(|entity| entity.borrow().flags & GameFlags::NoKnockback.bits() != 0),
            knockback_scale: host.knockback(),
            friendly_fire: host.friendly_fire(),
            battlesuit: target_borrow
                .as_ref()
                .is_some_and(|client| client.ps.powerups.get(Powerup::PwBattlesuit as usize) != 0),
            falling: method == 19,
            juiced: method == 27,
            proximity_protected: host.product() == Product::Missionpack
                && method == 25
                && (target
                    .as_ref()
                    .is_some_and(|target| owner.as_ref().is_some_and(|owner| Rc::ptr_eq(target, owner)))
                    || parent
                        .as_ref()
                        .is_some_and(|parent| self.same_team(target.as_ref(), Some(parent)))),
            product: host.product(),
        }
    }

    fn source_state(&self, request: &DamageRequest, state: &CombatState, attacker: bool) -> CombatState {
        let host = &self.host;
        let records = host.records();
        let actor = if attacker {
            request.attack.attacker.as_ref()
        } else {
            Some(&request.target)
        };
        let entity = records.native_by_actor(actor);
        let mut next = state.clone();
        next.invulnerable = state.invulnerable
            || entity
                .as_ref()
                .is_some_and(|entity| entity.borrow().flags & GameFlags::Godmode.bits() != 0);
        next.team = entity
            .as_ref()
            .and_then(|entity| entity.borrow().client.clone())
            .filter(|_| host.game_type() >= GameType::GtTeam as i32)
            .map(|client| format!("q3-team:{}", client.borrow().sess.session_team as i32))
            .or_else(|| state.team.clone());
        next
    }

    fn progress(&self, value: CombatProgress) -> CombatProgress {
        match value {
            CombatProgress::Complete { .. } => value,
            CombatProgress::SourceContinuation {
                request,
                mutations,
                resume,
            } => {
                let bridge = self.clone();
                CombatProgress::SourceContinuation {
                    request: request.clone(),
                    mutations,
                    resume: Rc::new(move |current| {
                        let projected = ProjectedCurrent {
                            bridge: bridge.clone(),
                            request: request.clone(),
                            inner: current,
                        };
                        bridge.progress(resume(&projected))
                    }),
                }
            }
            CombatProgress::ArmorStage {
                channel,
                request,
                mutations,
                input,
                fallback,
                resume,
            } => {
                let bridge = self.clone();
                CombatProgress::ArmorStage {
                    channel,
                    request: request.clone(),
                    mutations,
                    input,
                    fallback,
                    resume: Rc::new(move |result, current| {
                        let projected = ProjectedCurrent {
                            bridge: bridge.clone(),
                            request: request.clone(),
                            inner: current,
                        };
                        bridge.progress(resume(result, &projected))
                    }),
                }
            }
        }
    }

    fn same_team(&self, first: Option<&EntityRef>, second: Option<&EntityRef>) -> bool {
        let (Some(first), Some(second)) = (first, second) else {
            return false;
        };
        let first = first.borrow();
        let second = second.borrow();
        first.client.as_ref().is_some()
            && second.client.as_ref().is_some()
            && self.host.game_type() >= GameType::GtTeam as i32
            && first.client.as_ref().is_some_and(|client| {
                second
                    .client
                    .as_ref()
                    .is_some_and(|other| client.borrow().sess.session_team == other.borrow().sess.session_team)
            })
    }
}

// ---------------------------------------------------------------------------
// world-adapter.ts
// ---------------------------------------------------------------------------

/// Collision shape word (`ActorCollision` shape layer).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActorCollisionShape {
    /// Inline model.
    InlineModel {
        /// Model index.
        model: i32,
    },
    /// Bounding box.
    Box,
    /// Capsule.
    Capsule,
}

/// Actor collision record (`ActorCollision`, minimal).
#[derive(Debug, Clone, PartialEq)]
pub struct ActorCollision {
    /// Shape.
    pub shape: ActorCollisionShape,
    /// Contents mask.
    pub contents: i32,
    /// Owner actor.
    pub owner: Option<ActorId>,
    /// Collision role.
    pub role: ActorCollisionRole,
    /// Monster.
    pub monster: bool,
    /// Dead monster.
    pub dead_monster: bool,
}

/// Actor collision role word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActorCollisionRole {
    /// Trigger.
    Trigger,
    /// Solid.
    Solid,
}

/// Q3 trace query over the shared scene (`TraceQuery`, Q3 layer).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3TraceQuery {
    /// Start point.
    pub start: Vec3,
    /// End point.
    pub end: Vec3,
    /// Trace shape.
    pub shape: TraceShape,
    /// Actor to pass through.
    pub pass_actor: Option<ActorId>,
    /// Contents mask.
    pub mask: i32,
    /// Curve collision.
    pub curves: bool,
    /// Player curve clipping.
    pub player_curve_clip: bool,
    /// Numeric profile.
    pub numeric: NumericProfile,
}

/// Q3 trace hit word.
#[derive(Debug, Clone, PartialEq)]
pub enum Q3TraceHit {
    /// No hit.
    None,
    /// World hit.
    World,
    /// Actor hit.
    Actor {
        /// Hit actor.
        actor: ActorId,
    },
}

/// Q3 trace result (`TraceResult`, Q3 layer).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3TraceResult {
    /// Fraction traveled.
    pub fraction: f32,
    /// End position.
    pub end: Vec3,
    /// Hit record.
    pub hit: Q3TraceHit,
    /// Contact.
    pub contact: TraceContact,
    /// Started solid.
    pub start_solid: bool,
    /// Entirely solid.
    pub all_solid: bool,
    /// Contents.
    pub contents: i32,
    /// Surface flags.
    pub surface_flags: i32,
}

/// World adapter host services (`Q3WorldAdapterHost`).
pub trait Q3WorldAdapterHost {
    /// Trace the shared scene.
    fn trace_scene(&self, query: &Q3TraceQuery) -> Q3TraceResult;
    /// Contents at a point.
    fn point_contents_scene(&self, query: &Q3TraceQuery, point: Vec3) -> i32;
    /// Actors overlapping bounds.
    fn query_actors(&self, bounds: Bounds) -> Vec<ActorId>;
    /// Spatial collision record.
    fn spatial_collision(&self, actor: &ActorId) -> Option<ActorCollision>;
    /// Body state.
    fn body_state(&self, actor: &ActorId) -> Option<BodyState>;
    /// Linked body.
    fn linked_body(&self, actor: &ActorId) -> Option<LinkedBody>;
    /// Update collision metadata before the link hook publishes it.
    fn set_collision(&self, actor: &OwnedActor, collision: ActorCollision);
    /// Link a body, optionally at a snapped origin.
    fn link_body(&self, actor: &OwnedActor, origin: Option<Vec3>);
    /// Unlink a body.
    fn unlink_body(&self, actor: &OwnedActor);
    /// Curve collision.
    fn curves(&self) -> bool;
    /// Player curve clipping.
    fn player_curve_clip(&self) -> bool;
    /// Model geometry trace start-solid word.
    fn geometry_trace_start_solid(&self, query: &Q3TraceQuery, model: i32, origin: Vec3, angles: Vec3) -> bool;
    /// Actor body trace start-solid word (`traceActorBody` layer).
    fn body_trace_start_solid(&self, query: &Q3TraceQuery, body: &BodyState, collision: &ActorCollision) -> bool;
}

/// Q3 source query words over the shared geometry and actor index
/// (`Q3WorldAdapter`).
#[derive(Clone)]
pub struct Q3WorldAdapter {
    host: Rc<dyn Q3WorldAdapterHost>,
    records: Q3EntityRecords,
}

impl std::fmt::Debug for Q3WorldAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Q3WorldAdapter").finish()
    }
}

impl Q3WorldAdapter {
    /// Adapter over a host and records.
    #[must_use]
    pub fn new(host: Rc<dyn Q3WorldAdapterHost>, records: Q3EntityRecords) -> Self {
        Self { host, records }
    }

    fn actor(&self, number: i32) -> Option<ActorId> {
        if number == ENTITYNUM_WORLD || number == ENTITYNUM_NONE || number < 0 {
            return None;
        }
        let entity = usize::try_from(number).ok().and_then(|slot| self.records.get(slot))?;
        if entity.borrow().inuse() {
            Some(entity.borrow().actor().id().clone())
        } else {
            None
        }
    }

    fn query(&self, input: &ActorTraceQuery) -> Q3TraceQuery {
        Q3TraceQuery {
            start: input.start,
            end: input.end,
            shape: input.shape,
            pass_actor: input.pass_actor.clone(),
            mask: input.mask,
            curves: self.host.curves(),
            player_curve_clip: self.host.player_curve_clip(),
            numeric: Q3_BINARY32_PROFILE,
        }
    }

    /// Unlink an actor, returning a restore callback.
    #[must_use]
    pub fn unlink_actor(&self, actor: &ActorId) -> Option<Box<dyn Fn()>> {
        let owner = self.records.host().actors().resolve_owned(actor)?;
        if self.host.linked_body(actor).is_none() {
            return None;
        }
        let native = self.records.native_by_actor(Some(actor));
        if let Some(native) = &native {
            native.borrow_mut().r.capture_link();
        }
        self.host.unlink_body(&owner);
        let adapter = self.clone();
        let records = self.records.clone();
        let host = self.host.clone();
        Some(Box::new(move || {
            if !records.host().actors().is_live(owner.id()) || records.host().bodies().read(owner.id()).is_none() {
                return;
            }
            if native.as_ref().is_some_and(|native| {
                records
                    .native_by_actor(Some(owner.id()))
                    .as_ref()
                    .is_some_and(|current| Rc::ptr_eq(current, native))
            }) {
                adapter.link(
                    native
                        .clone()
                        .unwrap_or_else(|| panic!("Q3 restore lost its native entity")),
                );
            } else {
                host.link_body(&owner, None);
            }
        }))
    }
}

impl ServerWorld for Q3WorldAdapter {
    fn entity_contact(&self, bounds: Bounds, entity_num: i32, capsule: bool) -> bool {
        let Some(entity) = usize::try_from(entity_num).ok().and_then(|slot| self.records.get(slot)) else {
            return false;
        };
        if !entity.borrow().inuse() {
            return false;
        }
        let actor = entity.borrow().actor().id().clone();
        self.contact_actor(bounds, &actor, capsule)
    }

    fn trace(&self, query: &ServerTraceQuery) -> ServerTraceResult {
        let actor_query = ActorTraceQuery {
            start: query.start,
            end: query.end,
            shape: query.shape,
            pass_actor: self.actor(query.pass_entity_num),
            mask: query.mask,
        };
        let result = self.trace_actor(&actor_query);
        let entity_num = match &result.hit {
            ActorTraceHit::None => ENTITYNUM_NONE,
            ActorTraceHit::World => ENTITYNUM_WORLD,
            ActorTraceHit::Actor { actor } => {
                let Some(entity) = self.records.by_actor(Some(actor)) else {
                    panic!("Shared collision actor has no Q3 source projection");
                };
                let slot = entity.borrow().slot;
                slot as i32
            }
        };
        ServerTraceResult {
            fraction: result.fraction,
            end: result.end,
            entity_num,
            solidity: result.solidity,
            contact: result.contact,
            contents: result.contents,
            surface_flags: result.surface_flags,
        }
    }

    fn area_entities(&self, bounds: Bounds, maximum: usize) -> Vec<i32> {
        let mut output = Vec::new();
        for actor in self.area_actors(bounds, maximum) {
            let Some(entity) = self.records.by_actor(Some(&actor)) else {
                panic!("Shared spatial actor has no Q3 source projection");
            };
            output.push(entity.borrow().slot as i32);
        }
        output
    }

    fn point_contents(&self, point: Vec3, pass_entity_num: i32) -> i32 {
        let query = self.query(&ActorTraceQuery {
            start: point,
            end: point,
            shape: TraceShape::Point,
            pass_actor: self.actor(pass_entity_num),
            mask: -1,
        });
        self.host.point_contents_scene(&query, point)
    }

    fn link_state(&self, number: i32) -> Option<LinkState> {
        let actor = self.actor(number);
        let linked = actor.as_ref().and_then(|actor| self.host.linked_body(actor))?;
        Some(LinkState {
            absbounds: linked.absolute_bounds,
            linked: true,
            linkcount: linked.link_count,
        })
    }

    fn link(&self, entity: EntityRef) {
        // NaN truncates through the source's float clamps to a zero
        // bitwise byte.
        let byte = |value: f32| -> i32 {
            if value.is_nan() {
                0
            } else {
                (value.trunc() as i32).clamp(1, 255)
            }
        };
        let (actor, model, contents, mins, maxs, owner_num, current_origin) = {
            let borrowed = entity.borrow();
            (
                borrowed.actor(),
                borrowed.r.model,
                borrowed.r.contents,
                borrowed.r.mins(),
                borrowed.r.maxs(),
                borrowed.r.owner_num,
                borrowed.r.current_origin(),
            )
        };
        let solid = match model {
            EntityCollisionModel::Inline { .. } => 0xffffff,
            EntityCollisionModel::Box | EntityCollisionModel::Capsule => {
                if contents & (1 | 0x2000000) == 0 {
                    0
                } else {
                    (byte(maxs.z + 32.0) << 16) | (byte(-mins.z) << 8) | byte(maxs.x)
                }
            }
        };
        {
            let mut borrowed = entity.borrow_mut();
            borrowed.r.clear_bounds_overrides();
            borrowed.s.solid = solid;
        }
        let shape = match model {
            EntityCollisionModel::Inline { index } => ActorCollisionShape::InlineModel { model: index },
            EntityCollisionModel::Box => ActorCollisionShape::Box,
            EntityCollisionModel::Capsule => ActorCollisionShape::Capsule,
        };
        self.host.set_collision(
            &actor,
            ActorCollision {
                shape,
                contents,
                owner: self.actor(owner_num),
                role: if contents == 0x40000000 {
                    ActorCollisionRole::Trigger
                } else {
                    ActorCollisionRole::Solid
                },
                monster: false,
                dead_monster: false,
            },
        );
        self.host.link_body(&actor, Some(current_origin));
        entity.borrow_mut().r.capture_link();
    }

    fn unlink(&self, number: i32) {
        let entity = usize::try_from(number).ok().and_then(|slot| self.records.get(slot));
        if let Some(entity) = entity {
            if entity.borrow().inuse() {
                let actor = entity.borrow().actor();
                entity.borrow_mut().r.capture_link();
                self.host.unlink_body(&actor);
            }
        }
    }
}

impl ActorSpatialQueries for Q3WorldAdapter {
    fn area_actors(&self, bounds: Bounds, maximum: usize) -> Vec<ActorId> {
        self.host.query_actors(bounds).into_iter().take(maximum).collect()
    }

    fn trace_actor(&self, query: &ActorTraceQuery) -> ActorTraceResult {
        let result = self.host.trace_scene(&self.query(query));
        ActorTraceResult {
            fraction: result.fraction,
            end: result.end,
            hit: match result.hit {
                Q3TraceHit::None => ActorTraceHit::None,
                Q3TraceHit::World => ActorTraceHit::World,
                Q3TraceHit::Actor { actor } => ActorTraceHit::Actor { actor },
            },
            contact: result.contact,
            solidity: if result.all_solid {
                TraceSolidity::AllSolid
            } else if result.start_solid {
                TraceSolidity::StartSolid
            } else {
                TraceSolidity::Clear
            },
            contents: result.contents,
            surface_flags: result.surface_flags,
        }
    }

    fn contact_actor(&self, bounds: Bounds, actor: &ActorId, capsule: bool) -> bool {
        let (Some(spatial), Some(body)) = (self.host.spatial_collision(actor), self.host.body_state(actor)) else {
            return false;
        };
        let origin = vec3(0.0, 0.0, 0.0);
        let shape = if capsule {
            TraceShape::Capsule {
                mins: bounds.min,
                maxs: bounds.max,
            }
        } else {
            TraceShape::Box {
                mins: bounds.min,
                maxs: bounds.max,
            }
        };
        let query = self.query(&ActorTraceQuery {
            start: origin,
            end: origin,
            shape,
            pass_actor: None,
            mask: -1,
        });
        if let ActorCollisionShape::InlineModel { model } = spatial.shape {
            return self
                .host
                .geometry_trace_start_solid(&query, model, body.origin, body.angles);
        }
        self.host.body_trace_start_solid(&query, &body, &spatial)
    }
}

// ---------------------------------------------------------------------------
// map-spawns.ts
// ---------------------------------------------------------------------------

/// Spawn variables (`SpawnVariables`, game/spawn.ts, minimal).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SpawnVariables {
    /// Key/value pairs.
    pub vars: Vec<(String, String)>,
}

/// Spawn handler (`SpawnHandler`, game/spawn.ts).
pub type SpawnHandler = Rc<dyn Fn(EntityRef, &SpawnVariables)>;

/// Spawn handler host services (`Q3SpawnHandlersHost`).
///
/// Handler tables and spawn functions live in the sibling game-layer and
/// team-arena ports; the host supplies them so the parent can unify at
/// merge.
pub trait Q3SpawnHandlersHost {
    /// Product.
    fn product(&self) -> Product;
    /// Misc spawn handlers (`miscSpawnHandlers`).
    fn misc_handlers(&self) -> HashMap<String, SpawnHandler>;
    /// Mover spawn handlers (`MoverSpawnRuntime.handlers`).
    fn mover_handlers(&self) -> HashMap<String, SpawnHandler>;
    /// Trigger spawn handlers (`triggerSpawnHandlers`).
    fn trigger_handlers(&self) -> HashMap<String, SpawnHandler>;
    /// Target spawn handlers (`targetSpawnHandlers`).
    fn target_handlers(&self) -> HashMap<String, SpawnHandler>;
    /// Player start spawn (`spawnPlayerStart`).
    fn spawn_player_start(&self) -> SpawnHandler;
    /// Deathmatch point spawn (`spawnDeathmatchPoint`).
    fn spawn_deathmatch_point(&self) -> SpawnHandler;
    /// Team point spawn (`spawnTeamPoint`).
    fn spawn_team_point(&self) -> SpawnHandler;
    /// Team obelisk spawn (`TeamRuntime.spawnTeamObelisk`).
    fn spawn_team_obelisk(&self, entity: EntityRef, team: Team);
    /// Neutral obelisk spawn (`TeamRuntime.spawnNeutralObelisk`).
    fn spawn_neutral_obelisk(&self, entity: EntityRef);
}

/// All source classname routes, including the two intentionally empty
/// native spawn functions (`createQ3SpawnHandlers`).
pub fn create_q3_spawn_handlers(
    host: Rc<dyn Q3SpawnHandlersHost>,
) -> Result<HashMap<String, SpawnHandler>, Q3BaseError> {
    let mut handlers = HashMap::new();
    handlers.insert("info_player_start".to_string(), host.spawn_player_start());
    handlers.insert("info_player_deathmatch".to_string(), host.spawn_deathmatch_point());
    handlers.insert(
        "info_player_intermission".to_string(),
        Rc::new(|_: EntityRef, _: &SpawnVariables| {}) as SpawnHandler,
    );
    handlers.insert(
        "item_botroam".to_string(),
        Rc::new(|_: EntityRef, _: &SpawnVariables| {}) as SpawnHandler,
    );
    for classname in [
        "team_CTF_redplayer",
        "team_CTF_blueplayer",
        "team_CTF_redspawn",
        "team_CTF_bluespawn",
    ] {
        handlers.insert(classname.to_string(), host.spawn_team_point());
    }
    handlers.extend(host.misc_handlers());
    handlers.extend(host.mover_handlers());
    handlers.extend(host.trigger_handlers());
    handlers.extend(host.target_handlers());
    let Some(remove) = handlers.get("info_null").cloned() else {
        return Err(Q3BaseError::Invalid(
            "Source info_null handler is unavailable".to_string(),
        ));
    };
    handlers.insert("func_group".to_string(), remove);
    if host.product() == Product::Missionpack {
        let red = host.clone();
        handlers.insert(
            "team_redobelisk".to_string(),
            Rc::new(move |entity, _| red.spawn_team_obelisk(entity, Team::TeamRed)),
        );
        let blue = host.clone();
        handlers.insert(
            "team_blueobelisk".to_string(),
            Rc::new(move |entity, _| blue.spawn_team_obelisk(entity, Team::TeamBlue)),
        );
        handlers.insert(
            "team_neutralobelisk".to_string(),
            Rc::new(move |entity, _| host.spawn_neutral_obelisk(entity)),
        );
    }
    Ok(handlers)
}

/// Linked entity team counts (`findQ3EntityTeams` result).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntityTeamCounts {
    /// Team count.
    pub teams: i32,
    /// Chained entity count.
    pub entities: i32,
}

/// Link entity teams (`findQ3EntityTeams`).
///
/// Preserves prepended teammate order and transfers each slave's
/// targetname to its master.
pub fn find_q3_entity_teams(pool: &dyn Q3EntityPool) -> EntityTeamCounts {
    let mut teams = 0;
    let mut entities = 0;
    let num = pool.num_entities();
    let mut index = 1;
    while index < num {
        let master = pool.entity_at(index);
        let (inuse, team, slave) = {
            let borrowed = master.borrow();
            (
                borrowed.inuse(),
                borrowed.team.clone(),
                borrowed.flags & GameFlags::Teamslave.bits() != 0,
            )
        };
        if !inuse || team.is_none() || slave {
            index += 1;
            continue;
        }
        master.borrow_mut().teammaster = Some(Rc::downgrade(&master));
        teams += 1;
        entities += 1;
        let mut next = index + 1;
        while next < num {
            let entity = pool.entity_at(next);
            let (inuse, team_name, slave, same) = {
                let borrowed = entity.borrow();
                (
                    borrowed.inuse(),
                    borrowed.team.clone(),
                    borrowed.flags & GameFlags::Teamslave.bits() != 0,
                    borrowed.team == team,
                )
            };
            if !inuse || team_name.is_none() || slave || !same {
                next += 1;
                continue;
            }
            entities += 1;
            let head = master.borrow_mut().teamchain.take();
            entity.borrow_mut().teamchain = head;
            master.borrow_mut().teamchain = Some(entity.clone());
            entity.borrow_mut().teammaster = Some(Rc::downgrade(&master));
            entity.borrow_mut().flags |= GameFlags::Teamslave.bits();
            if let Some(targetname) = entity.borrow_mut().targetname.take() {
                master.borrow_mut().targetname = Some(targetname);
            }
            next += 1;
        }
        index += 1;
    }
    EntityTeamCounts { teams, entities }
}

// ---------------------------------------------------------------------------
// settings.ts
// ---------------------------------------------------------------------------

/// Cvar definition (`CvarDefinition`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CvarDefinition {
    /// Name.
    pub name: String,
    /// Default value.
    pub value: String,
    /// Flags.
    pub flags: u32,
    /// Announce changes.
    pub track: bool,
    /// Remap team shaders on change.
    pub team_shader: bool,
}

fn cvar_definition(name: &str, value: &str, flags: u32, track: bool, team_shader: bool) -> CvarDefinition {
    CvarDefinition {
        name: name.to_string(),
        value: value.to_string(),
        flags,
        track,
        team_shader,
    }
}

/// Q3 game cvar definitions (`q3GameCvarDefinitions`).
#[must_use]
pub fn q3_game_cvar_definitions(product: Product) -> Vec<CvarDefinition> {
    let archive = cvar_flags::ARCHIVE;
    let server_info = cvar_flags::SERVER_INFO;
    let user_info = cvar_flags::USER_INFO;
    let latch = cvar_flags::LATCH;
    let read_only = cvar_flags::READ_ONLY;
    let no_restart = cvar_flags::NO_RESTART;
    let system_info = cvar_flags::SYSTEM_INFO;
    let mut definitions = vec![
        cvar_definition("sv_cheats", "", 0, false, false),
        cvar_definition("g_restarted", "0", read_only, false, false),
        cvar_definition("g_gametype", "0", server_info | user_info | latch, false, false),
        cvar_definition("sv_maxclients", "8", server_info | latch | archive, false, false),
        cvar_definition("g_maxGameClients", "0", server_info | latch | archive, false, false),
        cvar_definition("dmflags", "0", server_info | archive, true, false),
        cvar_definition("fraglimit", "20", server_info | archive | no_restart, true, false),
        cvar_definition("timelimit", "0", server_info | archive | no_restart, true, false),
        cvar_definition("capturelimit", "8", server_info | archive | no_restart, true, false),
        cvar_definition("g_synchronousClients", "0", system_info, false, false),
        cvar_definition("g_friendlyFire", "0", archive, true, false),
        cvar_definition("g_teamAutoJoin", "0", archive, false, false),
        cvar_definition("g_teamForceBalance", "0", archive, false, false),
        cvar_definition("g_warmup", "20", archive, true, false),
        cvar_definition("g_doWarmup", "0", 0, true, false),
        cvar_definition("g_log", "games.log", archive, false, false),
        cvar_definition("g_logSync", "0", archive, false, false),
        cvar_definition("g_password", "", user_info, false, false),
        cvar_definition("g_banIPs", "", archive, false, false),
        cvar_definition("g_filterBan", "1", archive, false, false),
        cvar_definition("g_needpass", "0", server_info | read_only, false, false),
        cvar_definition("dedicated", "0", 0, false, false),
        cvar_definition("g_speed", "320", 0, true, false),
        cvar_definition("g_gravity", "800", 0, true, false),
        cvar_definition("g_knockback", "1000", 0, true, false),
        cvar_definition("g_quadfactor", "3", 0, true, false),
        cvar_definition("g_weaponrespawn", "5", 0, true, false),
        cvar_definition("g_weaponTeamRespawn", "30", 0, true, false),
        cvar_definition("g_forcerespawn", "20", 0, true, false),
        cvar_definition("g_inactivity", "0", 0, true, false),
        cvar_definition("g_debugMove", "0", 0, false, false),
        cvar_definition("g_debugDamage", "0", 0, false, false),
        cvar_definition("g_debugAlloc", "0", 0, false, false),
        cvar_definition("g_motd", "", 0, false, false),
        cvar_definition("com_blood", "1", 0, false, false),
        cvar_definition("g_podiumDist", "80", 0, false, false),
        cvar_definition("g_podiumDrop", "70", 0, false, false),
        cvar_definition("g_allowVote", "1", archive, false, false),
        cvar_definition("g_listEntity", "0", 0, false, false),
    ];
    if product == Product::Missionpack {
        definitions.extend([
            cvar_definition("g_obeliskHealth", "2500", 0, false, false),
            cvar_definition("g_obeliskRegenPeriod", "1", 0, false, false),
            cvar_definition("g_obeliskRegenAmount", "15", 0, false, false),
            cvar_definition("g_obeliskRespawnDelay", "10", server_info, false, false),
            cvar_definition("g_cubeTimeout", "30", 0, false, false),
            cvar_definition("g_redteam", "Stroggs", archive | server_info | user_info, true, true),
            cvar_definition("g_blueteam", "Pagans", archive | server_info | user_info, true, true),
            cvar_definition("ui_singlePlayerActive", "", 0, false, false),
            cvar_definition("g_enableDust", "0", server_info, true, false),
            cvar_definition("g_enableBreath", "0", server_info, true, false),
            cvar_definition("g_proxMineTimeout", "20000", 0, false, false),
        ]);
    }
    definitions.extend([
        cvar_definition("g_smoothClients", "1", 0, false, false),
        cvar_definition("pmove_fixed", "0", system_info, false, false),
        cvar_definition("pmove_msec", "8", system_info, false, false),
        cvar_definition("g_rankings", "0", 0, false, false),
        cvar_definition("sv_enableRankings", "0", 0, false, false),
        cvar_definition("sv_rankingsActive", "0", read_only, false, false),
    ]);
    definitions
}

/// Settings host services (`Q3SettingsHost`).
pub trait Q3SettingsHost {
    /// Cvar registry.
    fn cvars(&self) -> Rc<RefCell<CvarRegistry>>;
    /// Send a server command.
    fn send_server_command(&self, client: i32, command: String);
    /// Remap team shaders.
    fn remap_teams(&self);
    /// Format a tracked cvar change (`gameFormat('print "Server: %s
    /// changed to %s\n"', ...)`, game/format.ts).
    fn format_tracked_change(&self, name: &str, value: &str) -> String;
}

/// Capture a module cvar snapshot (`captureModuleCvar`,
/// game/save-module-values.ts).
#[must_use]
pub fn capture_module_cvar(value: &CvarSnapshot) -> SaveJson {
    obj(vec![
        ("name", save_str(&value.name)),
        ("value", save_str(&value.value)),
        ("resetValue", save_str(&value.reset_value)),
        (
            "latchedValue",
            value
                .latched_value
                .as_ref()
                .map_or(SaveJson::Null, |latched| save_str(latched)),
        ),
        ("flags", int(i64::from(value.flags))),
        ("modified", boolean(value.modified)),
        ("modificationCount", int(i64::from(value.modification_count))),
        ("numericValue", num(f64::from(value.numeric_value))),
        ("integerValue", int(i64::from(value.integer_value))),
    ])
}

/// Read a module cvar snapshot (`readModuleCvar`,
/// game/save-module-values.ts).
pub fn read_module_cvar(reader: SaveReader<'_>) -> Result<CvarSnapshot, ValueError> {
    Ok(CvarSnapshot {
        name: reader.field("name").string()?,
        value: reader.field("value").string()?,
        reset_value: reader.field("resetValue").string()?,
        latched_value: reader.field("latchedValue").nullable(|item| item.string())?,
        flags: reader.field("flags").integer(i64::MIN)? as u32,
        modified: reader.field("modified").boolean()?,
        modification_count: reader.field("modificationCount").integer(i64::MIN)? as u32,
        numeric_value: reader.field("numericValue").number()? as f32,
        integer_value: reader.field("integerValue").integer(i64::MIN)? as i32,
    })
}

/// Q3 game settings (`Q3GameSettings`).
///
/// Copies values at the source `G_UpdateCvars` point, independent of
/// changes to the shared cvars.
pub struct Q3GameSettings {
    host: Rc<dyn Q3SettingsHost>,
    product: Product,
    definitions: Vec<CvarDefinition>,
    snapshots: RefCell<HashMap<String, CvarSnapshot>>,
}

impl std::fmt::Debug for Q3GameSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Q3GameSettings")
            .field("product", &self.product)
            .finish()
    }
}

impl Q3GameSettings {
    /// Settings over a host and product.
    #[must_use]
    pub fn new(host: Rc<dyn Q3SettingsHost>, product: Product) -> Self {
        let definitions = q3_game_cvar_definitions(product);
        Self {
            host,
            product,
            definitions,
            snapshots: RefCell::new(HashMap::new()),
        }
    }

    /// Owning product.
    #[must_use]
    pub fn product(&self) -> Product {
        self.product
    }

    /// Cvar definitions.
    #[must_use]
    pub fn definitions(&self) -> &[CvarDefinition] {
        &self.definitions
    }

    /// Capture snapshot save words.
    #[must_use]
    pub fn capture_save_state(&self) -> SaveJson {
        let snapshots = self.snapshots.borrow();
        arr(self
            .definitions
            .iter()
            .filter_map(|definition| snapshots.get(&definition.name))
            .map(capture_module_cvar)
            .collect())
    }

    /// Restore snapshot save words.
    pub fn restore_save_state(&self, value: &SaveJson) -> Result<(), ValueError> {
        let reader = SaveReader::at(value, "q3.settings");
        let snapshots = reader.list(read_module_cvar)?;
        let definitions: HashMap<String, &str> = self
            .definitions
            .iter()
            .map(|definition| (ascii_fold(&definition.name), definition.name.as_str()))
            .collect();
        let mut restored = HashMap::new();
        for snapshot in &snapshots {
            let Some(name) = definitions.get(&ascii_fold(&snapshot.name)) else {
                return Err(reader.fail("invalid settings snapshot names"));
            };
            if restored.contains_key(*name) {
                return Err(reader.fail("invalid settings snapshot names"));
            }
            restored.insert((*name).to_string(), snapshot.clone());
        }
        if restored.len() != self.definitions.len() {
            return Err(reader.fail("invalid settings snapshot names"));
        }
        *self.snapshots.borrow_mut() = restored;
        Ok(())
    }

    /// Register every game cvar.
    ///
    /// # Panics
    ///
    /// Panics when a game cvar cannot be registered.
    pub fn register(&self, build_date: &str) {
        for definition in &self.definitions {
            if definition.name == "g_restarted" {
                let gamename = self.host.cvars().borrow_mut().register(
                    "gamename",
                    "baseq3",
                    cvar_flags::SERVER_INFO | cvar_flags::READ_ONLY,
                );
                if !matches!(gamename, Ok(Some(_))) {
                    panic!("Could not register Q3 game cvar gamename");
                }
                let gamedate = self.host
                    .cvars()
                    .borrow_mut()
                    .register("gamedate", build_date, cvar_flags::READ_ONLY);
                if !matches!(gamedate, Ok(Some(_))) {
                    panic!("Could not register Q3 game cvar gamedate");
                }
            }
            let current =
                self.host
                    .cvars()
                    .borrow_mut()
                    .register(&definition.name, &definition.value, definition.flags);
            let Ok(Some(current)) = current else {
                panic!("Could not register Q3 game cvar {}", definition.name);
            };
            self.snapshots.borrow_mut().insert(definition.name.clone(), current);
        }
    }

    /// Snapshot for a registered cvar.
    ///
    /// # Panics
    ///
    /// Panics when the cvar is not registered.
    #[must_use]
    pub fn snapshot(&self, name: &str) -> CvarSnapshot {
        self.snapshots
            .borrow()
            .get(name)
            .unwrap_or_else(|| panic!("Unregistered Q3 game cvar {name}"))
            .clone()
    }

    /// Integer value for a registered cvar.
    ///
    /// # Panics
    ///
    /// Panics when the cvar is not registered.
    #[must_use]
    pub fn integer(&self, name: &str) -> i32 {
        self.snapshot(name).integer_value
    }

    /// Numeric value for a registered cvar.
    ///
    /// # Panics
    ///
    /// Panics when the cvar is not registered.
    #[must_use]
    pub fn number(&self, name: &str) -> f32 {
        self.snapshot(name).numeric_value
    }

    /// String value for a registered cvar.
    ///
    /// # Panics
    ///
    /// Panics when the cvar is not registered.
    #[must_use]
    pub fn string(&self, name: &str) -> String {
        self.snapshot(name).value.clone()
    }

    /// Copy changed values at the source `G_UpdateCvars` point.
    ///
    /// # Panics
    ///
    /// Panics when a game cvar disappeared from the registry.
    pub fn update(&self) {
        let mut remapped = false;
        for definition in &self.definitions {
            let previous = self.snapshot(&definition.name);
            let Some(current) = self.host.cvars().borrow().get(&definition.name) else {
                panic!("Game cvar disappeared: {}", definition.name);
            };
            self.snapshots
                .borrow_mut()
                .insert(definition.name.clone(), current.clone());
            if previous.modification_count == current.modification_count {
                continue;
            }
            if definition.track {
                self.host
                    .send_server_command(-1, self.host.format_tracked_change(&definition.name, &current.value));
            }
            if definition.team_shader {
                remapped = true;
            }
        }
        if remapped {
            self.host.remap_teams();
        }
    }
}

// ---------------------------------------------------------------------------
// product-restriction.ts (+ core/q3-product-policy.ts restriction layer)
// ---------------------------------------------------------------------------

/// Scrambled product identifier (`FS_SetRestrictions` words).
pub const SCRAMBLED_PRODUCT_ID: [u8; 152] = [
    220, 129, 255, 108, 244, 163, 171, 55, 133, 65, 199, 36, 140, 222, 53, 99, 65, 171, 175, 232, 236, 193, 210, 250,
    169, 104, 231, 231, 21, 201, 170, 208, 135, 175, 130, 136, 85, 215, 71, 23, 96, 32, 96, 83, 44, 240, 219, 138, 184,
    215, 73, 27, 196, 247, 55, 139, 148, 68, 78, 203, 213, 238, 139, 23, 45, 205, 118, 186, 236, 230, 231, 107, 212, 1,
    10, 98, 30, 20, 116, 180, 216, 248, 166, 35, 45, 22, 215, 229, 35, 116, 250, 167, 117, 3, 57, 55, 201, 229, 218,
    222, 128, 12, 141, 149, 32, 110, 168, 215, 184, 53, 31, 147, 62, 12, 138, 67, 132, 54, 125, 6, 221, 148, 140, 4,
    21, 44, 198, 3, 126, 12, 100, 236, 61, 42, 44, 251, 15, 135, 14, 134, 89, 92, 177, 246, 152, 106, 124, 78, 118, 80,
    28, 42,
];

/// Team Arena UI word for the prerelease demo policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TeamArenaUi {
    /// Retail UI.
    Retail,
    /// Demo UI.
    Demo,
}

/// Q3 product policy (`Q3ProductPolicy`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3ProductPolicy {
    /// Retail.
    Retail,
    /// Prerelease demo.
    PrereleaseDemo {
        /// Team Arena UI.
        team_arena_ui: TeamArenaUi,
    },
    /// Prerelease Team Arena demo.
    PrereleaseTaDemo,
}

/// Q3 mount restriction (`Q3MountRestriction`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3MountRestriction {
    /// No restriction.
    None,
    /// Demo restriction.
    Demo {
        /// Demo directory.
        directory: &'static str,
        /// Demo pak checksum.
        pak_checksum: u32,
    },
}

/// Mount restriction for a policy (`q3MountRestriction`).
#[must_use]
pub fn q3_mount_restriction(policy: Q3ProductPolicy, fs_restrict: bool) -> Q3MountRestriction {
    if fs_restrict || matches!(policy, Q3ProductPolicy::PrereleaseDemo { .. }) {
        Q3MountRestriction::Demo {
            directory: "demota",
            pak_checksum: 437558517,
        }
    } else {
        Q3MountRestriction::None
    }
}

/// Resolve the mount restriction before selecting recipe artifacts; a demo
/// restriction changes the mounted content (`resolveQ3MountRestriction`).
///
/// The donor reads the product identifier asynchronously; this sync port
/// takes the already-read bytes (`None` when the read yields nothing).
pub fn resolve_q3_mount_restriction(
    policy: Q3ProductPolicy,
    fs_restrict: bool,
    product_id: Option<&[u8]>,
) -> Result<Q3MountRestriction, Q3BaseError> {
    let forced = q3_mount_restriction(policy, fs_restrict);
    if matches!(forced, Q3MountRestriction::Demo { .. }) {
        return Ok(forced);
    }
    let Some(product_id) = product_id else {
        return Ok(q3_mount_restriction(policy, true));
    };
    let mut seed: i32 = 5000;
    for (index, scrambled) in SCRAMBLED_PRODUCT_ID.iter().enumerate() {
        if (scrambled ^ ((seed & 255) as u8)) != product_id.get(index).copied().unwrap_or(0) {
            return Err(Q3BaseError::Invalid("Invalid product identification".to_string()));
        }
        seed = seed.wrapping_mul(69069).wrapping_add(1);
    }
    Ok(forced)
}

// ---------------------------------------------------------------------------
// bot-debug.ts
// ---------------------------------------------------------------------------

/// Bot debug polygon sink (`BotDebugPolygons`).
pub trait BotDebugPolygons {
    /// Create a debug polygon, returning its identifier.
    fn create(&mut self, color: i32, count: usize, points: &[Vec3]) -> i32;
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::cmd::Dialect;
    use qa_core::identity::IdentityOwner;

    fn test_owner() -> IdentityOwner {
        IdentityOwner::create("q3_base_test").expect("owner")
    }

    fn test_provider() -> ProviderId {
        ProviderId::new("q3", "test")
    }

    // -- definitions ------------------------------------------------------

    #[test]
    fn stat_schemas_match_source_slots() {
        let base = stat_schema(Product::BaseQ3);
        assert_eq!(base.product(), Product::BaseQ3);
        assert_eq!(base.persistent_powerup(), None);
        let indices = base.indices();
        assert_eq!(
            (indices.health, indices.weapons, indices.armor, indices.max_health),
            (0, 2, 3, 6)
        );
        let pack = stat_schema(Product::Missionpack);
        assert_eq!(pack.product(), Product::Missionpack);
        assert_eq!(pack.persistent_powerup(), Some(2));
        let indices = pack.indices();
        assert_eq!(
            (indices.health, indices.weapons, indices.armor, indices.max_health),
            (0, 3, 4, 7)
        );
    }

    #[test]
    fn weapon_availability_follows_product() {
        assert_eq!(weapon_count(Product::BaseQ3), 11);
        assert_eq!(weapon_count(Product::Missionpack), 14);
        assert!(weapon_available(Product::BaseQ3, Weapon::WpGrapplingHook));
        assert!(!weapon_available(Product::BaseQ3, Weapon::WpNailgun));
        assert!(weapon_available(Product::Missionpack, Weapon::WpChaingun));
        assert!(!weapon_available(Product::Missionpack, Weapon::WpNone));
        assert_eq!(GameType::GtTeam as i32, 3);
        assert_eq!(Team::TeamSpectator as i32, 3);
        assert_eq!(EntityType::EtEvents as i32, 13);
        assert_eq!(EV_EVENT_BITS, EV_EVENT_BIT1 | EV_EVENT_BIT2);
    }

    // -- trajectory -------------------------------------------------------

    fn linear_fixture() -> Trajectory {
        Trajectory {
            trajectory_type: TrajectoryType::TrLinear,
            time: 1000,
            duration: 0,
            base: vec3(1.0, 2.0, 3.0),
            delta: vec3(100.0, 0.0, -50.0),
        }
    }

    #[test]
    fn linear_trajectory_scales_by_seconds() {
        let at = evaluate_trajectory(&linear_fixture(), 1500);
        assert_eq!(at, vec3(51.0, 2.0, -22.0));
        assert_eq!(
            evaluate_trajectory_delta(&linear_fixture(), 9999),
            vec3(100.0, 0.0, -50.0)
        );
    }

    #[test]
    fn gravity_trajectory_falls_quadratically() {
        let tr = Trajectory {
            trajectory_type: TrajectoryType::TrGravity,
            ..linear_fixture()
        };
        let at = evaluate_trajectory(&tr, 2000);
        assert_eq!(at.x, 101.0);
        assert_eq!(at.z, 3.0 - 50.0 - 400.0);
        let delta = evaluate_trajectory_delta(&tr, 2000);
        assert_eq!(delta, vec3(100.0, 0.0, -850.0));
    }

    #[test]
    fn sine_and_stop_trajectories_match_source() {
        let sine = Trajectory {
            trajectory_type: TrajectoryType::TrSine,
            time: 0,
            duration: 1000,
            base: vec3(0.0, 0.0, 0.0),
            delta: vec3(0.0, 0.0, 10.0),
        };
        assert_eq!(evaluate_trajectory(&sine, 0), vec3(0.0, 0.0, 0.0));
        let stop = Trajectory {
            trajectory_type: TrajectoryType::TrLinearStop,
            time: 0,
            duration: 100,
            base: vec3(0.0, 0.0, 0.0),
            delta: vec3(10.0, 0.0, 0.0),
        };
        assert_eq!(evaluate_trajectory(&stop, 50), vec3(0.5, 0.0, 0.0));
        assert_eq!(evaluate_trajectory(&stop, 5000), vec3(1.0, 0.0, 0.0));
        assert_eq!(evaluate_trajectory_delta(&stop, 5000), vec3(0.0, 0.0, 0.0));
        let stationary = Trajectory::zero(TrajectoryType::TrStationary);
        assert_eq!(evaluate_trajectory(&stationary, 1234), vec3(0.0, 0.0, 0.0));
    }

    #[test]
    fn unknown_trajectory_tag_is_a_drop_error() {
        assert!(TrajectoryType::from_i32(5).is_ok());
        let error = TrajectoryType::from_i32(99).unwrap_err();
        assert!(matches!(error, Q3BaseError::Drop(_)));
    }

    // -- direction byte ---------------------------------------------------

    #[test]
    fn direction_table_round_trips() {
        assert_eq!(BYTE_DIRECTIONS.len(), NUM_VERTEX_NORMALS);
        assert_eq!(BYTE_DIRECTIONS[5], vec3(0.0, 0.0, 1.0));
        for (index, direction) in BYTE_DIRECTIONS.iter().enumerate() {
            assert_eq!(
                direction_to_byte(Some(*direction)),
                index,
                "entry {index} must win its own dot search"
            );
            assert_eq!(byte_to_direction(index as i32), *direction);
        }
    }

    #[test]
    fn direction_byte_edges_match_source() {
        assert_eq!(direction_to_byte(None), 0);
        assert_eq!(direction_to_byte(Some(vec3(0.0, 0.0, 0.0))), 0);
        assert_eq!(byte_to_direction(-1), ZERO_DIRECTION);
        assert_eq!(byte_to_direction(162), ZERO_DIRECTION);
        assert_eq!(byte_to_direction(999), ZERO_DIRECTION);
    }

    // -- entity state -----------------------------------------------------

    #[test]
    fn entity_state_copies_every_field() {
        let mut source = EntityState::new();
        source.number = 7;
        source.pos = linear_fixture();
        source.origin2 = vec3(1.0, 2.0, 3.0);
        let copy = source.copy();
        assert_eq!(copy, source);
        let mut target = EntityState::new();
        target.copy_from_state(&source);
        assert_eq!(target, source);
        assert_eq!(EntityState::default(), EntityState::new());
    }

    // -- player state -----------------------------------------------------

    #[test]
    fn player_state_defaults_match_retail() {
        let ps = create_player_state(Product::BaseQ3, None);
        assert_eq!(ps.pm_type, MoveType::PmNormal as i32);
        assert_eq!(ps.weapon, Weapon::WpNone as i32);
        assert_eq!(ps.weapon_state, WeaponState::WeaponReady as i32);
        assert_eq!(ps.health(), 0);
    }

    #[test]
    fn player_health_uses_product_schema() {
        let mut base = create_player_state(Product::BaseQ3, None);
        base.set_health(125);
        assert_eq!(base.stats.get(0), 125);
        let mut pack = create_player_state(Product::Missionpack, None);
        pack.set_health(200);
        assert_eq!(pack.health(), 200);
    }

    #[test]
    fn player_events_cycle_slots_and_sequence() {
        let mut ps = create_player_state(Product::BaseQ3, None);
        let first = ps.add_event(13, 1);
        assert_eq!(
            first,
            PredictableEvent {
                sequence: 0,
                event: 13,
                parameter: 1
            }
        );
        let second = ps.add_event(14, 0);
        assert_eq!(second.sequence, 1);
        assert_eq!(ps.events.get(0), 13);
        assert_eq!(ps.event_parms.get(0), 1);
        assert_eq!(ps.events.get(1), 14);
        assert_eq!(ps.event_sequence, 2);
    }

    struct DebugSink {
        text: String,
        lines: RefCell<Vec<String>>,
    }

    impl PredictableEventDebug for DebugSink {
        fn module(&self) -> EventDebugModule {
            EventDebugModule::Game
        }

        fn show_events(&self) -> String {
            self.text.clone()
        }

        fn print(&self, message: &str) {
            self.lines.borrow_mut().push(message.to_string());
        }
    }

    #[test]
    fn player_event_debug_prints_source_line() {
        let mut ps = create_player_state(Product::BaseQ3, None);
        let sink = Rc::new(DebugSink {
            text: "1".to_string(),
            lines: RefCell::new(Vec::new()),
        });
        ps.set_event_debug(Some(sink.clone()));
        ps.pmove_framecount = 41;
        ps.add_event(EntityEvent::EvJumpPad as i32, 1);
        let lines = sink.lines.borrow();
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("EV_JUMP_PAD"), "{}", lines[0]);
        assert!(lines[0].contains("parm 1"), "{}", lines[0]);
    }

    #[test]
    fn player_copy_preserves_authority_words() {
        let mut ps = create_player_state(Product::BaseQ3, None);
        ps.set_origin(vec3(1.0, 2.0, 3.0));
        ps.set_health(90);
        let copy = ps.copy();
        assert_eq!(copy.origin(), vec3(1.0, 2.0, 3.0));
        assert_eq!(copy.health(), 90);
        let mut other = create_player_state(Product::Missionpack, None);
        other.copy_from(&ps, AuthorityStores::ReplaceAuthority);
        assert_eq!(other.product(), Product::BaseQ3);
        assert_eq!(other.origin(), vec3(1.0, 2.0, 3.0));
    }

    // -- items ------------------------------------------------------------

    struct FixedInventory {
        product: Product,
        health: i32,
        armor: i32,
        max_health: i32,
        holdable_item: i32,
        team: i32,
        ammo: i32,
        powerup: i32,
        persistent: i32,
    }

    impl PlayerInventory for FixedInventory {
        fn product(&self) -> Product {
            self.product
        }

        fn health(&self) -> i32 {
            self.health
        }

        fn armor(&self) -> i32 {
            self.armor
        }

        fn max_health(&self) -> i32 {
            self.max_health
        }

        fn holdable_item(&self) -> i32 {
            self.holdable_item
        }

        fn team(&self) -> i32 {
            self.team
        }

        fn ammo(&self, _weapon: Weapon) -> i32 {
            self.ammo
        }

        fn powerup(&self, _powerup: Powerup) -> i32 {
            self.powerup
        }

        fn persistent_powerup_index(&self) -> i32 {
            self.persistent
        }
    }

    fn base_inventory() -> FixedInventory {
        FixedInventory {
            product: Product::BaseQ3,
            health: 100,
            armor: 0,
            max_health: 100,
            holdable_item: 0,
            team: Team::TeamFree as i32,
            ammo: 0,
            powerup: 0,
            persistent: 0,
        }
    }

    #[test]
    fn item_lists_split_at_the_missionpack_tail() {
        assert_eq!(item_list(Product::BaseQ3).len(), BASE_ITEM_COUNT);
        assert_eq!(item_list(Product::Missionpack).len(), 52);
        assert_eq!(item_at(Product::BaseQ3, 0).unwrap().item_type(), ItemType::ItBad);
        assert_eq!(item_at(Product::BaseQ3, 8).unwrap().pickup_name, Some("Gauntlet"));
        assert!(item_at(Product::BaseQ3, 36).is_err());
        assert_eq!(item_at(Product::Missionpack, 51).unwrap().pickup_name, Some("Chaingun"));
    }

    #[test]
    fn item_finds_cover_names_tags_and_errors() {
        assert_eq!(
            find_item(Product::BaseQ3, "quad damage").unwrap().class_name,
            Some("item_quad")
        );
        assert!(find_item(Product::BaseQ3, "missing").is_none());
        assert_eq!(
            find_item_for_powerup(Product::BaseQ3, Powerup::PwFlight)
                .unwrap()
                .class_name,
            Some("item_flight")
        );
        assert_eq!(
            find_item_for_holdable(Product::BaseQ3, Holdable::HiMedkit)
                .unwrap()
                .class_name,
            Some("holdable_medkit")
        );
        assert!(find_item_for_holdable(Product::BaseQ3, Holdable::HiNumHoldable).is_err());
        assert_eq!(
            find_item_for_weapon(Product::BaseQ3, Weapon::WpShotgun)
                .unwrap()
                .class_name,
            Some("weapon_shotgun")
        );
        assert!(find_item_for_weapon(Product::BaseQ3, Weapon::WpNone).is_err());
    }

    #[test]
    fn grab_rules_match_bg_canitemgrabbed() {
        let ps = base_inventory();
        let weapon = PickupEntity {
            model_index: 10,
            model_index2: 0,
            generic1: 0,
        };
        assert!(can_item_be_grabbed(0, &weapon, &ps).unwrap());
        let mut full_ammo = base_inventory();
        full_ammo.ammo = 200;
        let ammo = PickupEntity {
            model_index: 18,
            model_index2: 0,
            generic1: 0,
        };
        assert!(!can_item_be_grabbed(0, &ammo, &full_ammo).unwrap());
        let mut hurt = base_inventory();
        hurt.health = 50;
        let health = PickupEntity {
            model_index: 5,
            model_index2: 0,
            generic1: 0,
        };
        assert!(can_item_be_grabbed(0, &health, &hurt).unwrap());
        assert!(!can_item_be_grabbed(0, &health, &ps).unwrap());
        let mut red = base_inventory();
        red.team = Team::TeamRed as i32;
        let blue_flag = PickupEntity {
            model_index: 35,
            model_index2: 0,
            generic1: 0,
        };
        assert!(can_item_be_grabbed(GameType::GtCtf as i32, &blue_flag, &red).unwrap());
        let red_flag = PickupEntity {
            model_index: 34,
            model_index2: 0,
            generic1: 0,
        };
        assert!(!can_item_be_grabbed(GameType::GtCtf as i32, &red_flag, &red).unwrap());
        let mut holding = base_inventory();
        holding.holdable_item = 1;
        let teleporter = PickupEntity {
            model_index: 26,
            model_index2: 0,
            generic1: 0,
        };
        assert!(!can_item_be_grabbed(0, &teleporter, &holding).unwrap());
        let bad = PickupEntity {
            model_index: 0,
            model_index2: 0,
            generic1: 0,
        };
        assert!(can_item_be_grabbed(0, &bad, &ps).is_err());
    }

    #[test]
    fn armor_grab_rules_cover_scout_and_guard() {
        let mut scout = base_inventory();
        scout.product = Product::Missionpack;
        scout.persistent = 42;
        assert!(!can_q3_armor_be_grabbed(&scout).unwrap());
        let mut guard = base_inventory();
        guard.product = Product::Missionpack;
        guard.persistent = 43;
        guard.armor = 100;
        assert!(!can_q3_armor_be_grabbed(&guard).unwrap());
        guard.armor = 99;
        assert!(can_q3_armor_be_grabbed(&guard).unwrap());
        let mut plain = base_inventory();
        plain.armor = 199;
        assert!(can_q3_armor_be_grabbed(&plain).unwrap());
        plain.armor = 200;
        assert!(!can_q3_armor_be_grabbed(&plain).unwrap());
    }

    #[test]
    fn player_touch_uses_source_bounds() {
        let item = Trajectory {
            trajectory_type: TrajectoryType::TrStationary,
            time: 0,
            duration: 0,
            base: vec3(0.0, 0.0, 0.0),
            delta: vec3(0.0, 0.0, 0.0),
        };
        assert!(player_touches_item(vec3(0.0, 0.0, 0.0), &item, 0));
        assert!(!player_touches_item(vec3(45.0, 0.0, 0.0), &item, 0));
        assert!(!player_touches_item(vec3(0.0, 37.0, 0.0), &item, 0));
    }

    // -- jump pad / snapshot ----------------------------------------------

    #[test]
    fn jump_pad_applies_velocity_and_event() {
        let mut ps = create_player_state(Product::BaseQ3, None);
        ps.pmove_framecount = 9;
        let mut pad = EntityState::new();
        pad.number = 12;
        pad.origin2 = vec3(0.0, 0.0, 700.0);
        touch_jump_pad(&mut ps, &pad);
        assert_eq!(ps.velocity(), vec3(0.0, 0.0, 700.0));
        assert_eq!(ps.jumppad_ent, 12);
        assert_eq!(ps.jumppad_frame, 9);
        assert_eq!(ps.events.get(0), EntityEvent::EvJumpPad as i32);
        assert_eq!(ps.event_parms.get(0), 1);
    }

    #[test]
    fn jump_pad_ignores_flight_and_dead() {
        let mut ps = create_player_state(Product::BaseQ3, None);
        ps.powerups.set(Powerup::PwFlight as usize, 9999);
        let mut pad = EntityState::new();
        pad.origin2 = vec3(0.0, 0.0, 700.0);
        touch_jump_pad(&mut ps, &pad);
        assert_eq!(ps.velocity(), vec3(0.0, 0.0, 0.0));
        let mut dead = create_player_state(Product::BaseQ3, None);
        dead.pm_type = MoveType::PmDead as i32;
        touch_jump_pad(&mut dead, &pad);
        assert_eq!(dead.jumppad_ent, 0);
    }

    #[test]
    fn snapshot_conversion_consumes_events_and_snaps() {
        let mut ps = create_player_state(Product::BaseQ3, None);
        ps.client_num = 3;
        ps.set_origin(vec3(10.7, -4.2, 0.5));
        ps.set_health(100);
        ps.add_event(EntityEvent::EvJump as i32, 0);
        ps.powerups.set(Powerup::PwQuad as usize, 30);
        let mut entity = EntityState::new();
        player_state_to_entity_state(&mut ps, &mut entity, true);
        assert_eq!(entity.e_type, EntityType::EtPlayer as i32);
        assert_eq!(entity.number, 3);
        assert_eq!(entity.pos.base, vec3(10.0, -4.0, 0.0));
        assert_eq!(entity.event, EntityEvent::EvJump as i32);
        assert_eq!(entity.powerups, 1 << (Powerup::PwQuad as i32));
        assert_eq!(ps.entity_event_sequence, 1);
        let mut gibbed = create_player_state(Product::BaseQ3, None);
        gibbed.set_health(GIB_HEALTH);
        let mut hidden = EntityState::new();
        player_state_to_entity_state(&mut gibbed, &mut hidden, false);
        assert_eq!(hidden.e_type, EntityType::EtInvisible as i32);
    }

    #[test]
    fn snapshot_extrapolation_uses_linear_stop() {
        let mut ps = create_player_state(Product::BaseQ3, None);
        ps.set_origin(vec3(0.0, 0.0, 0.0));
        ps.set_velocity(vec3(100.0, 0.0, 0.0));
        ps.set_health(100);
        let mut entity = EntityState::new();
        player_state_to_entity_state_extra_polate(&mut ps, &mut entity, 500, false);
        assert_eq!(entity.pos.trajectory_type, TrajectoryType::TrLinearStop);
        assert_eq!(entity.pos.time, 500);
        assert_eq!(entity.pos.duration, 50);
        let moved: MovementTrace = ServerTraceResult {
            fraction: 1.0,
            end: vec3(0.0, 0.0, 0.0),
            entity_num: ENTITYNUM_NONE,
            solidity: TraceSolidity::Clear,
            contact: TraceContact::None,
            contents: 0,
            surface_flags: 0,
        };
        assert_eq!(moved.entity_num, ENTITYNUM_NONE);
    }

    // -- session fakes ------------------------------------------------------

    struct FakeActors {
        owner: IdentityOwner,
        owned: RefCell<HashMap<ActorId, OwnedActor>>,
        live: RefCell<Vec<ActorId>>,
        watchers: RefCell<Vec<Box<dyn Fn(&OwnedActor)>>>,
        next_generation: Cell<u32>,
    }

    impl FakeActors {
        fn new() -> Self {
            Self {
                owner: test_owner(),
                owned: RefCell::new(HashMap::new()),
                live: RefCell::new(Vec::new()),
                watchers: RefCell::new(Vec::new()),
                next_generation: Cell::new(1),
            }
        }
    }

    impl Q3SessionActors for FakeActors {
        fn assert_owned(&self, actor: &OwnedActor) -> Result<(), Q3BaseError> {
            if self.owner.owns_owned(actor) {
                Ok(())
            } else {
                Err(Q3BaseError::Invalid("foreign actor".to_string()))
            }
        }

        fn allocate_at_source(&self, provider: &ProviderId, slot: usize, _definition: &str) -> OwnedActor {
            let generation = self.next_generation.get();
            self.next_generation.set(generation + 1);
            let id = self.owner.actor(slot as u32, generation);
            let owned = self.owner.owned_actor(&id, provider.clone()).expect("owned");
            self.owned.borrow_mut().insert(id.clone(), owned.clone());
            self.live.borrow_mut().push(id);
            owned
        }

        fn is_live(&self, actor: &ActorId) -> bool {
            self.live.borrow().contains(actor)
        }

        fn on_release(&self, callback: Box<dyn Fn(&OwnedActor)>) -> Box<dyn Fn()> {
            self.watchers.borrow_mut().push(callback);
            Box::new(|| {})
        }

        fn release(&self, actor: &OwnedActor) {
            self.live.borrow_mut().retain(|id| id != actor.id());
            for watcher in self.watchers.borrow().iter() {
                watcher(actor);
            }
        }

        fn resolve_owned(&self, actor: &ActorId) -> Option<OwnedActor> {
            self.owned.borrow().get(actor).cloned()
        }
    }

    struct FakeBodies {
        states: RefCell<HashMap<ActorId, BodyState>>,
        linked: RefCell<HashMap<ActorId, LinkedBody>>,
    }

    impl FakeBodies {
        fn new() -> Self {
            Self {
                states: RefCell::new(HashMap::new()),
                linked: RefCell::new(HashMap::new()),
            }
        }
    }

    impl Q3SessionBodies for FakeBodies {
        fn create(&self, actor: &OwnedActor, state: BodyState) {
            self.states.borrow_mut().insert(actor.id().clone(), state);
        }

        fn read(&self, actor: &ActorId) -> Option<BodyState> {
            self.states.borrow().get(actor).cloned()
        }

        fn write(&self, actor: &OwnedActor, state: BodyState) {
            self.states.borrow_mut().insert(actor.id().clone(), state);
        }

        fn linked(&self, actor: &ActorId) -> Option<LinkedBody> {
            self.linked.borrow().get(actor).cloned()
        }

        fn link(&self, actor: &OwnedActor, origin: Option<Vec3>) {
            let mut state = self
                .states
                .borrow()
                .get(actor.id())
                .cloned()
                .unwrap_or_else(|| ZERO_BODY.clone());
            if let Some(origin) = origin {
                state.origin = origin;
            }
            let count = self
                .linked
                .borrow()
                .get(actor.id())
                .map_or(1, |linked| linked.link_count + 1);
            self.linked.borrow_mut().insert(
                actor.id().clone(),
                LinkedBody {
                    actor: actor.id().clone(),
                    state: state.clone(),
                    absolute_bounds: state.bounds,
                    link_count: count,
                },
            );
            self.states.borrow_mut().insert(actor.id().clone(), state);
        }

        fn unlink(&self, actor: &OwnedActor) {
            self.linked.borrow_mut().remove(actor.id());
        }
    }

    struct FakeCombat {
        states: RefCell<HashMap<ActorId, CombatState>>,
    }

    impl FakeCombat {
        fn new() -> Self {
            Self {
                states: RefCell::new(HashMap::new()),
            }
        }
    }

    impl Q3SessionCombat for FakeCombat {
        fn read(&self, actor: &ActorId) -> Option<CombatState> {
            self.states.borrow().get(actor).cloned()
        }

        fn create(&self, actor: &OwnedActor, initial: CombatState, _admit_damage: Option<DamageAdmissionFn>) {
            self.states.borrow_mut().insert(actor.id().clone(), initial);
        }

        fn set_health(&self, actor: &OwnedActor, health: i32) {
            if let Some(state) = self.states.borrow_mut().get_mut(actor.id()) {
                state.health = health;
            }
        }

        fn set_can_take_damage(&self, actor: &OwnedActor, can_take_damage: bool) {
            if let Some(state) = self.states.borrow_mut().get_mut(actor.id()) {
                state.can_take_damage = can_take_damage;
            }
        }

        fn set_regular_points(&self, actor: &OwnedActor, points: i32, initial: RegularArmorState) {
            let mut states = self.states.borrow_mut();
            let state = states.get_mut(actor.id()).expect("combat state");
            state.armor.regular = match &state.armor.regular {
                RegularArmorState::None => initial,
                RegularArmorState::Q1 { absorption, item, .. } => RegularArmorState::Q1 {
                    points,
                    absorption: *absorption,
                    item: item.clone(),
                },
                RegularArmorState::Q2 {
                    normal_protection,
                    energy_protection,
                    item,
                    ..
                } => RegularArmorState::Q2 {
                    points,
                    normal_protection: *normal_protection,
                    energy_protection: *energy_protection,
                    item: item.clone(),
                },
                RegularArmorState::Q3 { protection, .. } => RegularArmorState::Q3 {
                    points,
                    protection: *protection,
                },
                RegularArmorState::Source { item, .. } => RegularArmorState::Source {
                    points,
                    item: item.clone(),
                },
            };
        }

        fn bind_damage_admission(&self, _actor: &OwnedActor, _admit_damage: DamageAdmissionFn) {}

        fn apply(&self, request: DamageRequest) -> DamageOutcome {
            DamageOutcome::StaleTarget { request }
        }
    }

    struct FakeInventory {
        entries: RefCell<HashMap<(ActorId, ItemId), (i32, i32)>>,
    }

    impl FakeInventory {
        fn new() -> Self {
            Self {
                entries: RefCell::new(HashMap::new()),
            }
        }
    }

    impl Q3SessionInventory for FakeInventory {
        fn has(&self, _actor: &ActorId) -> bool {
            true
        }

        fn create(&self, _actor: &OwnedActor, entries: Vec<InventoryEntry>) {
            for entry in entries {
                self.entries
                    .borrow_mut()
                    .insert((_actor.id().clone(), entry.item), (entry.count, entry.capacity));
            }
        }

        fn count(&self, actor: &ActorId, item: &ItemId) -> i32 {
            self.entries
                .borrow()
                .get(&(actor.clone(), item.clone()))
                .map_or(0, |(count, _)| *count)
        }

        fn configure(&self, actor: &OwnedActor, item: &ItemId, count: i32, capacity: i32) {
            self.entries
                .borrow_mut()
                .insert((actor.id().clone(), item.clone()), (count, capacity));
        }
    }

    struct FakeCallbacks {
        bound: RefCell<HashMap<ActorId, ActorCallbacks>>,
    }

    impl FakeCallbacks {
        fn new() -> Self {
            Self {
                bound: RefCell::new(HashMap::new()),
            }
        }
    }

    impl Q3ActorCallbacks for FakeCallbacks {
        fn bind(&self, actor: &OwnedActor, callbacks: ActorCallbacks) {
            self.bound.borrow_mut().insert(actor.id().clone(), callbacks);
        }
    }

    struct FakeRecordHost {
        actors: Rc<FakeActors>,
        bodies: Rc<FakeBodies>,
        combat: Rc<FakeCombat>,
        inventory: Rc<FakeInventory>,
        callbacks: Rc<FakeCallbacks>,
        scheduled: RefCell<Vec<(ActorId, Option<i32>)>>,
        foreign: RefCell<HashMap<ActorId, EntityRef>>,
        players: RefCell<Vec<ActorId>>,
        call: RefCell<Option<Q3DamageCall>>,
    }

    impl FakeRecordHost {
        fn new() -> Self {
            Self {
                actors: Rc::new(FakeActors::new()),
                bodies: Rc::new(FakeBodies::new()),
                combat: Rc::new(FakeCombat::new()),
                inventory: Rc::new(FakeInventory::new()),
                callbacks: Rc::new(FakeCallbacks::new()),
                scheduled: RefCell::new(Vec::new()),
                foreign: RefCell::new(HashMap::new()),
                players: RefCell::new(Vec::new()),
                call: RefCell::new(None),
            }
        }
    }

    impl Q3RecordHost for FakeRecordHost {
        fn actors(&self) -> Rc<dyn Q3SessionActors> {
            self.actors.clone()
        }

        fn bodies(&self) -> Rc<dyn Q3SessionBodies> {
            self.bodies.clone()
        }

        fn combat(&self) -> Rc<dyn Q3SessionCombat> {
            self.combat.clone()
        }

        fn inventory(&self) -> Rc<dyn Q3SessionInventory> {
            self.inventory.clone()
        }

        fn callbacks(&self) -> Rc<dyn Q3ActorCallbacks> {
            self.callbacks.clone()
        }

        fn schedule(&self, actor: &OwnedActor, due_milliseconds: Option<i32>) {
            self.scheduled.borrow_mut().push((actor.id().clone(), due_milliseconds));
        }

        fn run_think(&self, _actor: &OwnedActor, _time_milliseconds: i32) {}

        fn damage_call(&self) -> Option<Q3DamageCall> {
            self.call.borrow().clone()
        }

        fn foreign(&self, actor: &ActorId) -> Option<EntityRef> {
            self.foreign.borrow().get(actor).cloned()
        }

        fn is_player(&self, actor: &ActorId) -> bool {
            self.players.borrow().contains(actor)
        }
    }

    fn test_records() -> (Rc<FakeRecordHost>, Q3EntityRecords) {
        let host = Rc::new(FakeRecordHost::new());
        let records = Q3EntityRecords::new(host.clone(), test_provider(), Product::BaseQ3);
        (host, records)
    }

    // -- records ------------------------------------------------------------

    #[test]
    fn records_activate_and_mirror_stats() {
        let (host, records) = test_records();
        let entity = records.activate(3);
        assert!(entity.borrow().inuse());
        assert_eq!(entity.borrow().slot, 3);
        assert!(host.callbacks.bound.borrow().len() == 1);
        let client = records.client(3);
        client.borrow_mut().ps.stats.set(0, 120);
        let actor = entity.borrow().actor();
        assert_eq!(host.combat.read(actor.id()).unwrap().health, 120);
        client.borrow_mut().ps.stats.set(3, 45);
        assert!(matches!(
            host.combat.read(actor.id()).unwrap().armor.regular,
            RegularArmorState::Q3 { points: 45, .. }
        ));
        client
            .borrow_mut()
            .ps
            .stats
            .set(2, (1 << (Weapon::WpShotgun as i32)) | (1 << (Weapon::WpBfg as i32)));
        let shotgun = q3_weapon_item(Weapon::WpShotgun as i32).unwrap();
        let bfg = q3_weapon_item(Weapon::WpBfg as i32).unwrap();
        let machinegun = q3_weapon_item(Weapon::WpMachinegun as i32).unwrap();
        assert_eq!(host.inventory.count(actor.id(), &shotgun.item), 1);
        assert_eq!(host.inventory.count(actor.id(), &bfg.item), 1);
        assert_eq!(host.inventory.count(actor.id(), &machinegun.item), 0);
        client.borrow_mut().ps.ammo.set(Weapon::WpShotgun as usize, 12);
        assert_eq!(host.inventory.count(actor.id(), &shotgun.ammo.unwrap()), 12);
    }

    #[test]
    fn records_attach_adopt_and_release() {
        let (host, records) = test_records();
        let owned = host.actors.allocate_at_source(&test_provider(), 500, "q3:entity");
        records.host().bodies().create(&owned, ZERO_BODY.clone());
        let adopted = records.adopt(500, owned.clone()).expect("adopt");
        assert_eq!(adopted.borrow().s.number, 500);
        assert!(adopted.borrow().inuse());
        assert!(records.adopt(501, owned).is_err());

        let foreign_owned = host
            .actors
            .allocate_at_source(&ProviderId::new("other", "game"), 501, "q3:entity");
        let attached = records.attach(9, foreign_owned, true).expect("attach");
        assert!(attached.borrow().client.is_some());
        assert_eq!(attached.borrow().s.number, 9);

        records.release(adopted.clone());
        assert_eq!(adopted.borrow().s.number, 0);
        assert!(!adopted.borrow().inuse());
    }

    #[test]
    fn records_ownership_round_trips_into_fresh_records() {
        let (_host, records) = test_records();
        records.activate(1);
        records.activate(70);
        let ownership = records.capture_ownership();
        assert_eq!(ownership.len(), MAX_GENTITIES);
        assert!(ownership[1].active);
        assert!(ownership[70].active);
        assert!(!ownership[2].active);

        let (host2, fresh) = test_records();
        let _ = host2;
        // Ownership words reference foreign actors, so hydration fails
        // instead of aliasing another session's actors.
        assert!(fresh.restore_ownership(&ownership).is_err());
        assert!(fresh.restore_ownership(&ownership[..10]).is_err());

        let backing = records.capture_client_backing(1);
        assert_eq!(backing.source_stats, [0; 16]);
        fresh.restore_client_backing(1, &backing).expect("backing");
        assert!(fresh.restore_client_backing(99, &backing).is_err());
    }

    #[test]
    fn records_resolve_natives_foreigners_and_inflictors() {
        let (host, records) = test_records();
        let entity = records.activate(4);
        let actor = entity.borrow().actor().id().clone();
        assert!(records.native_by_actor(None).is_none());
        assert!(Rc::ptr_eq(&records.native_by_actor(Some(&actor)).unwrap(), &entity));
        assert!(records.by_actor(Some(&actor)).is_some());
        let owner = test_owner();
        let ghost = owner.actor(900, 1);
        assert!(records.by_actor(Some(&ghost)).is_none());
        host.foreign.borrow_mut().insert(ghost.clone(), entity.clone());
        assert!(records.by_actor(Some(&ghost)).is_some());
        let world = records.damage_inflictor(None);
        assert!(matches!(world, DamageParticipant::Native(_)));
        let foreign = records.damage_inflictor(Some(&ghost));
        assert!(matches!(foreign, DamageParticipant::SharedActor(_)));
        assert!(records.use_participant(None).is_none());
    }

    #[test]
    fn records_dispatch_bound_callbacks() {
        let (host, records) = test_records();
        let entity = records.activate(11);
        let actor = entity.borrow().actor().id().clone();
        let fired = Rc::new(RefCell::new(Vec::new()));
        let think_fired = fired.clone();
        entity.borrow_mut().think = Some(Rc::new(move |_| {
            think_fired.borrow_mut().push("think");
        }));
        let pain_fired = fired.clone();
        entity.borrow_mut().pain = Some(Rc::new(move |_, _, damage| {
            pain_fired.borrow_mut().push(if damage == 7 { "pain" } else { "bad" });
        }));
        let die_fired = fired.clone();
        entity.borrow_mut().die = Some(Rc::new(move |_, _, _, _, method| {
            die_fired.borrow_mut().push(if method == 9 { "die" } else { "bad" });
        }));
        let bound = host.callbacks.bound.borrow().get(&actor).expect("bound").clone();
        (bound.think)();
        (bound.pain)(&PainReaction {
            attack: None,
            attacker: None,
            damage: 7,
        });
        host.call.borrow_mut().replace(Q3DamageCall {
            target: entity.clone(),
            source: DamageParticipant::Native(entity.clone()),
            owner: DamageParticipant::Native(entity.clone()),
            direction: None,
            point: None,
            amount: 7.0,
            flags: 0,
            method_of_death: 9,
        });
        (bound.die)(&DeathReaction {
            attack: None,
            attacker: None,
            damage: 7,
            inflictor: None,
            point: vec3(0.0, 0.0, 0.0),
        });
        assert_eq!(*fired.borrow(), vec!["think", "pain", "die"]);
        assert_eq!(entity.borrow().nextthink(), 0);
        records.close();
    }

    // -- combat bridge --------------------------------------------------------

    struct FakePool {
        records: Q3EntityRecords,
        num: usize,
    }

    impl Q3EntityPool for FakePool {
        fn num_entities(&self) -> usize {
            self.num
        }

        fn entity_at(&self, index: usize) -> EntityRef {
            self.records.get(index).expect("pool entity")
        }
    }

    struct FakeBridgeHost {
        records_host: Rc<FakeRecordHost>,
        records: Q3EntityRecords,
        pool: EntityPoolRef,
        world: Rc<dyn Q3ServerWorld>,
        time: Cell<i32>,
        intermission: Cell<i32>,
        game_type: Cell<i32>,
        feedback: RefCell<Vec<String>>,
    }

    impl FakeBridgeHost {
        fn new(records_host: Rc<FakeRecordHost>, records: Q3EntityRecords) -> Rc<Self> {
            let world_host = Rc::new(FakeWorldHost::new());
            let world: Rc<dyn Q3ServerWorld> = Rc::new(Q3WorldAdapter::new(world_host, records.clone()));
            Rc::new(Self {
                records_host,
                pool: Rc::new(FakePool {
                    records: records.clone(),
                    num: MAX_CLIENTS,
                }),
                world,
                records,
                time: Cell::new(1000),
                intermission: Cell::new(0),
                game_type: Cell::new(0),
                feedback: RefCell::new(Vec::new()),
            })
        }
    }

    impl Q3CombatBridgeHost for FakeBridgeHost {
        fn authority(&self) -> Rc<dyn Q3SessionCombat> {
            self.records_host.combat.clone()
        }

        fn entities(&self) -> EntityPoolRef {
            self.pool.clone()
        }

        fn records(&self) -> Q3EntityRecords {
            self.records.clone()
        }

        fn world(&self) -> Rc<dyn Q3ServerWorld> {
            self.world.clone()
        }

        fn weapon_provider(&self) -> ProviderId {
            ProviderId::new("q3", "weapon")
        }

        fn combat_provider(&self) -> ProviderId {
            ProviderId::new("q3", "combat")
        }

        fn inventory_provider(&self) -> ProviderId {
            ProviderId::new("q3", "inventory")
        }

        fn movement_provider(&self) -> ProviderId {
            ProviderId::new("q3", "movement")
        }

        fn armor_context(&self, _request: &DamageRequest) -> VictimArmorContext {
            VictimArmorContext {
                screen_facing_dot: 0.0,
                arithmetic: VictimArithmetic::Binary32,
                q2: None,
            }
        }

        fn time(&self) -> i32 {
            self.time.get()
        }

        fn intermission_queued(&self) -> i32 {
            self.intermission.get()
        }

        fn game_type(&self) -> i32 {
            self.game_type.get()
        }

        fn friendly_fire(&self) -> bool {
            false
        }

        fn knockback(&self) -> f32 {
            1000.0
        }

        fn product(&self) -> Product {
            Product::BaseQ3
        }

        fn check_hurt_carrier(&self, _target: EntityRef, _attacker: EntityRef) {}

        fn log_accuracy_hit(&self, _target: EntityRef, _attacker: EntityRef) -> bool {
            false
        }

        fn damage_feedback(&self, _call: &Q3DamageCall, _decision: &DamageDecision) {
            self.feedback.borrow_mut().push("damage".to_string());
        }

        fn foreign_damage_feedback(&self, _target: EntityRef, _owner: Option<EntityRef>, _decision: &DamageDecision) {
            self.feedback.borrow_mut().push("foreign".to_string());
        }
    }

    fn test_attack(target: &ActorId, attacker: Option<&ActorId>) -> AttackProvenance {
        let _ = target;
        AttackProvenance {
            sequence: 0,
            time: SourceTime::Milliseconds(1000),
            attacker: attacker.cloned(),
            inflictor: attacker.cloned(),
            originating_projectile: None,
            weapon: None,
            weapon_provider: ProviderId::new("q3", "weapon"),
            damage_powerup_owner: None,
            combat_provider: ProviderId::new("q3", "combat"),
            inventory_provider: ProviderId::new("q3", "inventory"),
            movement_provider: ProviderId::new("q3", "movement"),
            cause: AttackCause::Q3 {
                means_of_death: 7,
                damage_flags: 0,
            },
        }
    }

    fn test_request(target: ActorId, attacker: Option<ActorId>, amount: f32) -> DamageRequest {
        DamageRequest {
            attack: test_attack(&target, attacker.as_ref()),
            target,
            amount,
            knockback: amount,
            direction: vec3(1.0, 0.0, 0.0),
            point: vec3(0.0, 0.0, 0.0),
            normal: vec3(0.0, 0.0, 0.0),
            delivery: DamageDelivery::Direct,
        }
    }

    fn test_combat_state(health: i32) -> CombatState {
        CombatState {
            health,
            armor: ArmorState {
                regular: RegularArmorState::Q3 {
                    points: 0,
                    protection: 0.66,
                },
                powered: PoweredProtectionState::None,
            },
            mass: 200,
            can_take_damage: true,
            invulnerable: false,
            no_knockback: false,
            team: None,
        }
    }

    struct FixedCurrent {
        target: Option<CombatState>,
        attacker: Option<CombatState>,
    }

    impl CurrentCombatState for FixedCurrent {
        fn target(&self) -> Option<CombatState> {
            self.target.clone()
        }

        fn attacker(&self) -> Option<CombatState> {
            self.attacker.clone()
        }
    }

    fn drive_progress(
        progress: CombatProgress,
        target: &CombatState,
        attacker: Option<&CombatState>,
    ) -> (CombatResult, Vec<DamageMutation>) {
        let mut progress = progress;
        let mut armor = target.armor.clone();
        let mut seen = Vec::new();
        loop {
            match progress {
                CombatProgress::Complete {
                    result, mut mutations, ..
                } => {
                    seen.append(&mut mutations);
                    return (result, seen);
                }
                CombatProgress::SourceContinuation {
                    mut mutations, resume, ..
                } => {
                    seen.append(&mut mutations);
                    let current = FixedCurrent {
                        target: Some(target.clone()),
                        attacker: attacker.cloned(),
                    };
                    progress = resume(&current);
                }
                CombatProgress::ArmorStage {
                    channel,
                    mut mutations,
                    resume,
                    fallback,
                    ..
                } => {
                    seen.append(&mut mutations);
                    let result = fallback(&armor);
                    armor = result.armor.clone();
                    let saved = match channel {
                        ProtectionChannel::Powered => result.power_saved,
                        ProtectionChannel::Regular => result.regular_saved,
                    };
                    let current = FixedCurrent {
                        target: Some(target.clone()),
                        attacker: attacker.cloned(),
                    };
                    progress = resume(ArmorStageResult { saved }, &current);
                }
            }
        }
    }

    #[test]
    fn bridge_captures_provenance_and_routes_feedback() {
        let (records_host, records) = test_records();
        let target = records.activate(5);
        target.borrow_mut().s.weapon = Weapon::WpShotgun as i32;
        let attacker = records.activate(6);
        let host = FakeBridgeHost::new(records_host, records.clone());
        let bridge = Q3CombatBridge::new(host.clone());

        let provenance = (bridge.context().attack)(
            &DamageParticipant::Native(target.clone()),
            &DamageParticipant::Native(attacker.clone()),
            None,
            7,
            0,
            None,
        );
        assert_eq!(provenance.sequence, 0);
        assert_eq!(provenance.weapon, Some(ItemId::new("q3:weapon/shotgun")));
        let second = (bridge.context().attack)(
            &DamageParticipant::Native(target.clone()),
            &DamageParticipant::Native(attacker.clone()),
            Some(ItemId::new("q3:weapon/bfg")),
            7,
            0,
            None,
        );
        assert_eq!(second.sequence, 1);
        assert_eq!(second.weapon, Some(ItemId::new("q3:weapon/bfg")));

        let saved = bridge.capture_save_state();
        let bridge2 = Q3CombatBridge::new(host.clone());
        bridge2.restore_save_state(&saved).expect("restore");
        let third = (bridge2.context().attack)(
            &DamageParticipant::Native(target.clone()),
            &DamageParticipant::Native(attacker.clone()),
            None,
            7,
            0,
            None,
        );
        assert_eq!(third.sequence, 2);

        let target_id = target.borrow().actor().id().clone();
        let attacker_id = attacker.borrow().actor().id().clone();
        let decision = DamageDecision {
            request: test_request(target_id.clone(), Some(attacker_id), 40.0),
            mutations: Vec::new(),
            applied_damage: 40,
            reaction: DamageReaction::Pain,
            feedback: None,
        };
        let outcome = (bridge.context().dispatch)(
            Q3DamageCall {
                target: target.clone(),
                source: DamageParticipant::Native(attacker.clone()),
                owner: DamageParticipant::Native(attacker.clone()),
                direction: None,
                point: None,
                amount: 40.0,
                flags: 0,
                method_of_death: 7,
            },
            &|| DamageOutcome::StaleTarget {
                request: test_request(target_id.clone(), None, 0.0),
            },
        );
        assert!(matches!(outcome, DamageOutcome::StaleTarget { .. }));
        assert!(bridge.current_call().is_none());
        bridge.before_reaction(&DamageDecision {
            request: test_request(target_id, None, 10.0),
            ..decision.clone()
        });
        assert_eq!(*host.feedback.borrow(), vec!["foreign".to_string()]);
    }

    #[test]
    fn bridge_policy_decides_q3_damage_flow() {
        let (records_host, records) = test_records();
        let target = records.activate(5);
        target.borrow_mut().client = Some(records.client(5));
        let attacker = records.activate(6);
        attacker.borrow_mut().client = Some(records.client(6));
        attacker
            .borrow()
            .client
            .as_ref()
            .unwrap()
            .borrow_mut()
            .ps
            .stats
            .set(BaseStatIndex::StatMaxHealth as usize, 100);
        let host = FakeBridgeHost::new(records_host, records);
        let bridge = Q3CombatBridge::new(host.clone());
        let policy = bridge.policy();

        let target_id = target.borrow().actor().id().clone();
        let attacker_id = attacker.borrow().actor().id().clone();
        let request = test_request(target_id, Some(attacker_id), 50.0);
        let state = test_combat_state(100);
        let progress = (policy.decide)(&request, &state, Some(&state));
        let (result, mutations) = drive_progress(progress, &state, Some(&state));
        assert_eq!(result.applied_damage, 50);
        assert_eq!(result.reaction, DamageReaction::Pain);
        assert!(mutations
            .iter()
            .any(|mutation| matches!(mutation, DamageMutation::Health { before: 100, after: 50 })));
        assert!(mutations
            .iter()
            .any(|mutation| matches!(mutation, DamageMutation::Impulse { .. })));

        host.intermission.set(1);
        let held = (policy.decide)(&request, &state, Some(&state));
        let (held_result, _) = drive_progress(held, &state, Some(&state));
        assert_eq!(held_result.applied_damage, 0);
        assert_eq!(held_result.reaction, DamageReaction::None);
    }

    #[test]
    fn bridge_policy_blocks_godmode_targets() {
        let (records_host, records) = test_records();
        let target = records.activate(5);
        target.borrow_mut().flags |= GameFlags::Godmode.bits();
        let host = FakeBridgeHost::new(records_host, records);
        let bridge = Q3CombatBridge::new(host);
        let policy = bridge.policy();
        let target_id = target.borrow().actor().id().clone();
        let request = test_request(target_id, None, 50.0);
        let state = test_combat_state(100);
        let (result, _) = drive_progress((policy.decide)(&request, &state, None), &state, None);
        assert_eq!(result.applied_damage, 0);
    }

    #[test]
    fn native_armor_absorbs_q3_points() {
        let armor = ArmorState {
            regular: RegularArmorState::Q3 {
                points: 50,
                protection: 0.66,
            },
            powered: PoweredProtectionState::None,
        };
        let flags = ArmorDamageFlags {
            stage: None,
            no_armor: false,
            no_power_armor: false,
            no_regular_armor: false,
            energy: false,
            regular_protection_scale: Some(1.0),
        };
        let context = VictimArmorContext {
            screen_facing_dot: 0.0,
            arithmetic: VictimArithmetic::Binary32,
            q2: None,
        };
        let result = absorb_native_armor(&armor, 100, &flags, &context);
        assert_eq!(result.regular_saved, 50);
        assert_eq!(result.power_saved, 0);
    }

    // -- world adapter --------------------------------------------------------

    struct FakeWorldHost {
        bodies: FakeBodies,
        trace_result: RefCell<Q3TraceResult>,
        actors: RefCell<Vec<ActorId>>,
        collisions: RefCell<HashMap<ActorId, ActorCollision>>,
    }

    impl FakeWorldHost {
        fn new() -> Self {
            Self {
                bodies: FakeBodies::new(),
                trace_result: RefCell::new(Q3TraceResult {
                    fraction: 1.0,
                    end: vec3(0.0, 0.0, 0.0),
                    hit: Q3TraceHit::None,
                    contact: TraceContact::None,
                    start_solid: false,
                    all_solid: false,
                    contents: 0,
                    surface_flags: 0,
                }),
                actors: RefCell::new(Vec::new()),
                collisions: RefCell::new(HashMap::new()),
            }
        }
    }

    impl Q3WorldAdapterHost for FakeWorldHost {
        fn trace_scene(&self, _query: &Q3TraceQuery) -> Q3TraceResult {
            self.trace_result.borrow().clone()
        }

        fn point_contents_scene(&self, query: &Q3TraceQuery, _point: Vec3) -> i32 {
            assert_eq!(query.mask, -1);
            3
        }

        fn query_actors(&self, _bounds: Bounds) -> Vec<ActorId> {
            self.actors.borrow().clone()
        }

        fn spatial_collision(&self, actor: &ActorId) -> Option<ActorCollision> {
            self.collisions.borrow().get(actor).cloned()
        }

        fn body_state(&self, actor: &ActorId) -> Option<BodyState> {
            self.bodies.read(actor)
        }

        fn linked_body(&self, actor: &ActorId) -> Option<LinkedBody> {
            self.bodies.linked(actor)
        }

        fn set_collision(&self, actor: &OwnedActor, collision: ActorCollision) {
            self.collisions.borrow_mut().insert(actor.id().clone(), collision);
        }

        fn link_body(&self, actor: &OwnedActor, origin: Option<Vec3>) {
            self.bodies.link(actor, origin);
        }

        fn unlink_body(&self, actor: &OwnedActor) {
            self.bodies.unlink(actor);
        }

        fn curves(&self) -> bool {
            true
        }

        fn player_curve_clip(&self) -> bool {
            false
        }

        fn geometry_trace_start_solid(&self, _query: &Q3TraceQuery, _model: i32, _origin: Vec3, _angles: Vec3) -> bool {
            true
        }

        fn body_trace_start_solid(
            &self,
            _query: &Q3TraceQuery,
            _body: &BodyState,
            _collision: &ActorCollision,
        ) -> bool {
            false
        }
    }

    #[test]
    fn adapter_maps_hits_and_encodes_solid() {
        let (_records_host, records) = test_records();
        let entity = records.activate(20);
        let actor = entity.borrow().actor().id().clone();
        let host = Rc::new(FakeWorldHost::new());
        host.trace_result.borrow_mut().hit = Q3TraceHit::Actor { actor: actor.clone() };
        host.trace_result.borrow_mut().fraction = 0.5;
        let adapter = Q3WorldAdapter::new(host.clone(), records.clone());

        let result = adapter.trace(&ServerTraceQuery {
            start: vec3(0.0, 0.0, 0.0),
            end: vec3(0.0, 0.0, 10.0),
            shape: TraceShape::Point,
            pass_entity_num: ENTITYNUM_NONE,
            mask: 1,
        });
        assert_eq!(result.entity_num, 20);
        assert_eq!(result.fraction, 0.5);

        host.trace_result.borrow_mut().hit = Q3TraceHit::None;
        let clear = adapter.trace(&ServerTraceQuery {
            start: vec3(0.0, 0.0, 0.0),
            end: vec3(0.0, 0.0, 10.0),
            shape: TraceShape::Point,
            pass_entity_num: ENTITYNUM_NONE,
            mask: 1,
        });
        assert_eq!(clear.entity_num, ENTITYNUM_NONE);

        host.actors.borrow_mut().push(actor.clone());
        assert_eq!(
            adapter.area_entities(
                Bounds {
                    min: vec3(0.0, 0.0, 0.0),
                    max: vec3(1.0, 1.0, 1.0),
                },
                1024
            ),
            vec![20]
        );
        assert_eq!(adapter.point_contents(vec3(0.0, 0.0, 0.0), ENTITYNUM_NONE), 3);

        entity.borrow_mut().r.contents = 1;
        entity.borrow_mut().r.set_mins(vec3(-15.0, -15.0, -24.0));
        entity.borrow_mut().r.set_maxs(vec3(15.0, 15.0, 32.0));
        adapter.link(entity.clone());
        assert_eq!(entity.borrow().s.solid, (64 << 16) | (24 << 8) | 15);
        let stored = host.collisions.borrow().get(&actor).expect("collision").clone();
        assert_eq!(stored.role, ActorCollisionRole::Solid);
        assert!(adapter.link_state(20).is_some());
        assert!(adapter.link_state(21).is_none());
        let restore = adapter.unlink_actor(&actor).expect("restore");
        assert!(adapter.link_state(20).is_none());
        restore();
        assert!(adapter.link_state(20).is_some());
    }

    #[test]
    fn adapter_contact_uses_model_or_body_paths() {
        let (_records_host, records) = test_records();
        let entity = records.activate(30);
        let actor = entity.borrow().actor().id().clone();
        let host = Rc::new(FakeWorldHost::new());
        let adapter = Q3WorldAdapter::new(host.clone(), records.clone());
        let bounds = Bounds {
            min: vec3(0.0, 0.0, 0.0),
            max: vec3(1.0, 1.0, 1.0),
        };
        assert!(!adapter.contact_actor(bounds, &actor, false));
        let owned = entity.borrow().actor();
        host.bodies.create(&owned, ZERO_BODY.clone());
        host.collisions.borrow_mut().insert(
            actor.clone(),
            ActorCollision {
                shape: ActorCollisionShape::InlineModel { model: 2 },
                contents: 1,
                owner: None,
                role: ActorCollisionRole::Solid,
                monster: false,
                dead_monster: false,
            },
        );
        assert!(adapter.contact_actor(bounds, &actor, false));
        assert!(adapter.entity_contact(bounds, 30, false));
        assert!(!adapter.entity_contact(bounds, 31, false));
    }

    // -- map spawns -----------------------------------------------------------

    struct FakeSpawnHost {
        product: Product,
        obelisks: RefCell<Vec<String>>,
    }

    impl Q3SpawnHandlersHost for FakeSpawnHost {
        fn product(&self) -> Product {
            self.product
        }

        fn misc_handlers(&self) -> HashMap<String, SpawnHandler> {
            let mut handlers: HashMap<String, SpawnHandler> = HashMap::new();
            handlers.insert(
                "info_null".to_string(),
                Rc::new(|entity: EntityRef, _: &SpawnVariables| {
                    entity.borrow_mut().flags |= 1;
                }),
            );
            handlers
        }

        fn mover_handlers(&self) -> HashMap<String, SpawnHandler> {
            HashMap::new()
        }

        fn trigger_handlers(&self) -> HashMap<String, SpawnHandler> {
            HashMap::new()
        }

        fn target_handlers(&self) -> HashMap<String, SpawnHandler> {
            HashMap::new()
        }

        fn spawn_player_start(&self) -> SpawnHandler {
            Rc::new(|_, _| {})
        }

        fn spawn_deathmatch_point(&self) -> SpawnHandler {
            Rc::new(|_, _| {})
        }

        fn spawn_team_point(&self) -> SpawnHandler {
            Rc::new(|_, _| {})
        }

        fn spawn_team_obelisk(&self, _entity: EntityRef, team: Team) {
            self.obelisks.borrow_mut().push(format!("team:{}", team as i32));
        }

        fn spawn_neutral_obelisk(&self, _entity: EntityRef) {
            self.obelisks.borrow_mut().push("neutral".to_string());
        }
    }

    #[test]
    fn spawn_handlers_cover_routes_and_obelisks() {
        let host = Rc::new(FakeSpawnHost {
            product: Product::Missionpack,
            obelisks: RefCell::new(Vec::new()),
        });
        let handlers = create_q3_spawn_handlers(host.clone()).expect("handlers");
        assert!(handlers.contains_key("info_player_start"));
        assert!(handlers.contains_key("info_player_intermission"));
        assert!(handlers.contains_key("item_botroam"));
        assert!(handlers.contains_key("func_group"));
        assert!(handlers.contains_key("team_redobelisk"));
        let (_records_host, records) = test_records();
        let entity = records.activate(40);
        handlers["func_group"](entity.clone(), &SpawnVariables::default());
        assert_eq!(entity.borrow().flags, 1);
        handlers["team_redobelisk"](entity.clone(), &SpawnVariables::default());
        handlers["team_blueobelisk"](entity.clone(), &SpawnVariables::default());
        handlers["team_neutralobelisk"](entity, &SpawnVariables::default());
        assert_eq!(
            *host.obelisks.borrow(),
            vec![
                format!("team:{}", Team::TeamRed as i32),
                format!("team:{}", Team::TeamBlue as i32),
                "neutral".to_string()
            ]
        );

        let base = Rc::new(FakeSpawnHost {
            product: Product::BaseQ3,
            obelisks: RefCell::new(Vec::new()),
        });
        let base_handlers = create_q3_spawn_handlers(base).expect("base");
        assert!(!base_handlers.contains_key("team_redobelisk"));
    }

    struct EmptySpawnHost;

    impl Q3SpawnHandlersHost for EmptySpawnHost {
        fn product(&self) -> Product {
            Product::BaseQ3
        }

        fn misc_handlers(&self) -> HashMap<String, SpawnHandler> {
            HashMap::new()
        }

        fn mover_handlers(&self) -> HashMap<String, SpawnHandler> {
            HashMap::new()
        }

        fn trigger_handlers(&self) -> HashMap<String, SpawnHandler> {
            HashMap::new()
        }

        fn target_handlers(&self) -> HashMap<String, SpawnHandler> {
            HashMap::new()
        }

        fn spawn_player_start(&self) -> SpawnHandler {
            Rc::new(|_, _| {})
        }

        fn spawn_deathmatch_point(&self) -> SpawnHandler {
            Rc::new(|_, _| {})
        }

        fn spawn_team_point(&self) -> SpawnHandler {
            Rc::new(|_, _| {})
        }

        fn spawn_team_obelisk(&self, _entity: EntityRef, _team: Team) {}

        fn spawn_neutral_obelisk(&self, _entity: EntityRef) {}
    }

    #[test]
    fn spawn_handlers_require_info_null() {
        let host = Rc::new(EmptySpawnHost);
        assert!(create_q3_spawn_handlers(host).is_err());
    }

    struct DirectBinding {
        active: bool,
        actor: OwnedActor,
        body: Rc<FakeDirectBody>,
    }

    struct FakeDirectBody {
        state: RefCell<BodyState>,
    }

    impl EntityBodyBinding for FakeDirectBody {
        fn read(&self) -> BodyState {
            self.state.borrow().clone()
        }

        fn write(&self, value: BodyState) {
            *self.state.borrow_mut() = value;
        }

        fn linked(&self) -> Option<LinkedBody> {
            None
        }
    }

    impl GameEntityBinding for DirectBinding {
        fn body(&self) -> Rc<dyn EntityBodyBinding> {
            self.body.clone()
        }

        fn actor(&self) -> OwnedActor {
            self.actor.clone()
        }

        fn active(&self) -> bool {
            self.active
        }

        fn health(&self) -> i32 {
            0
        }

        fn set_health(&self, _value: i32) {}

        fn takes_damage(&self) -> bool {
            false
        }

        fn set_takes_damage(&self, _value: bool) {}

        fn schedule(&self, _nextthink: i32) {}

        fn run_think(&self, _time_milliseconds: i32) {}
    }

    struct VecPool {
        entities: Vec<EntityRef>,
    }

    impl Q3EntityPool for VecPool {
        fn num_entities(&self) -> usize {
            self.entities.len()
        }

        fn entity_at(&self, index: usize) -> EntityRef {
            self.entities[index].clone()
        }
    }

    fn team_entity(owner: &IdentityOwner, slot: usize, team: Option<&str>) -> EntityRef {
        let actor = owner
            .owned_actor(&owner.actor(slot as u32, 1), ProviderId::new("q3", "test"))
            .expect("owned");
        let binding = Rc::new(DirectBinding {
            active: true,
            actor,
            body: Rc::new(FakeDirectBody {
                state: RefCell::new(ZERO_BODY.clone()),
            }),
        });
        let entity = Rc::new(RefCell::new(GameEntity::new(slot, binding)));
        entity.borrow_mut().team = team.map(str::to_string);
        entity
    }

    #[test]
    fn entity_teams_chain_and_transfer_targetnames() {
        let owner = test_owner();
        let first = team_entity(&owner, 1, Some("alpha"));
        let second = team_entity(&owner, 2, Some("alpha"));
        second.borrow_mut().targetname = Some("slave-target".to_string());
        let third = team_entity(&owner, 3, Some("beta"));
        let pool = VecPool {
            entities: vec![
                team_entity(&owner, 0, None),
                first.clone(),
                second.clone(),
                third.clone(),
            ],
        };
        let counts = find_q3_entity_teams(&pool);
        assert_eq!(counts, EntityTeamCounts { teams: 2, entities: 3 });
        assert_eq!(first.borrow().targetname, Some("slave-target".to_string()));
        assert_eq!(second.borrow().targetname, None);
        assert_ne!(second.borrow().flags & GameFlags::Teamslave.bits(), 0);
        assert!(Rc::ptr_eq(first.borrow().teamchain.as_ref().unwrap(), &second));
        assert!(third.borrow().teamchain.is_none());
    }

    // -- settings ---------------------------------------------------------------

    struct FakeSettingsHost {
        cvars: Rc<RefCell<CvarRegistry>>,
        commands: RefCell<Vec<(i32, String)>>,
        remapped: Cell<bool>,
    }

    impl FakeSettingsHost {
        fn new() -> Self {
            Self {
                cvars: Rc::new(RefCell::new(CvarRegistry::new(Dialect::Q3))),
                commands: RefCell::new(Vec::new()),
                remapped: Cell::new(false),
            }
        }
    }

    impl Q3SettingsHost for FakeSettingsHost {
        fn cvars(&self) -> Rc<RefCell<CvarRegistry>> {
            self.cvars.clone()
        }

        fn send_server_command(&self, client: i32, command: String) {
            self.commands.borrow_mut().push((client, command));
        }

        fn remap_teams(&self) {
            self.remapped.set(true);
        }

        fn format_tracked_change(&self, name: &str, value: &str) -> String {
            format!("print \"Server: {name} changed to {value}\n\"")
        }
    }

    #[test]
    fn settings_register_update_and_save_round_trip() {
        let host = Rc::new(FakeSettingsHost::new());
        let settings = Q3GameSettings::new(host.clone(), Product::BaseQ3);
        assert_eq!(settings.definitions().len(), 45);
        let pack = Q3GameSettings::new(host.clone(), Product::Missionpack);
        assert_eq!(pack.definitions().len(), 56);
        settings.register("2026-09-30");
        assert_eq!(settings.integer("fraglimit"), 20);
        assert_eq!(settings.number("g_speed"), 320.0);
        assert_eq!(settings.string("g_motd"), "");
        host.cvars.borrow_mut().set("fraglimit", "30", true).expect("set");
        settings.update();
        assert_eq!(settings.integer("fraglimit"), 30);
        let commands = host.commands.borrow();
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].0, -1);
        assert!(commands[0].1.contains("fraglimit"), "{}", commands[0].1);
        assert!(commands[0].1.contains("30"), "{}", commands[0].1);
        drop(commands);

        let saved = settings.capture_save_state();
        let host2 = Rc::new(FakeSettingsHost::new());
        let restored = Q3GameSettings::new(host2.clone(), Product::BaseQ3);
        restored.register("2026-09-30");
        restored.restore_save_state(&saved).expect("restore");
        assert_eq!(restored.integer("fraglimit"), 30);
        assert!(restored.restore_save_state(&arr(Vec::new())).is_err());
    }

    #[test]
    fn settings_remap_team_shaders_on_change() {
        let host = Rc::new(FakeSettingsHost::new());
        let settings = Q3GameSettings::new(host.clone(), Product::Missionpack);
        settings.register("2026-09-30");
        host.cvars.borrow_mut().set("g_redteam", "Rangers", true).expect("set");
        settings.update();
        assert!(host.remapped.get());
    }

    // -- product restriction ----------------------------------------------------

    fn valid_product_id() -> Vec<u8> {
        let mut seed: i32 = 5000;
        SCRAMBLED_PRODUCT_ID
            .iter()
            .map(|scrambled| {
                let byte = scrambled ^ ((seed & 255) as u8);
                seed = seed.wrapping_mul(69069).wrapping_add(1);
                byte
            })
            .collect()
    }

    #[test]
    fn mount_restriction_matches_fs_setrestrictions() {
        assert_eq!(
            q3_mount_restriction(Q3ProductPolicy::Retail, false),
            Q3MountRestriction::None
        );
        assert_eq!(
            q3_mount_restriction(Q3ProductPolicy::Retail, true),
            Q3MountRestriction::Demo {
                directory: "demota",
                pak_checksum: 437558517
            }
        );
        assert!(matches!(
            q3_mount_restriction(
                Q3ProductPolicy::PrereleaseDemo {
                    team_arena_ui: TeamArenaUi::Retail
                },
                false
            ),
            Q3MountRestriction::Demo { .. }
        ));
        let valid = valid_product_id();
        assert_eq!(
            resolve_q3_mount_restriction(Q3ProductPolicy::Retail, false, Some(&valid)).expect("valid"),
            Q3MountRestriction::None
        );
        assert!(matches!(
            resolve_q3_mount_restriction(Q3ProductPolicy::Retail, false, None).expect("missing"),
            Q3MountRestriction::Demo { .. }
        ));
        let mut corrupt = valid;
        corrupt[7] ^= 0xff;
        assert!(resolve_q3_mount_restriction(Q3ProductPolicy::Retail, false, Some(&corrupt)).is_err());
    }

    // -- bot debug ----------------------------------------------------------------

    struct FakePolygons {
        next: i32,
    }

    impl BotDebugPolygons for FakePolygons {
        fn create(&mut self, _color: i32, count: usize, points: &[Vec3]) -> i32 {
            assert_eq!(count, points.len());
            let id = self.next;
            self.next += 1;
            id
        }
    }

    #[test]
    fn bot_debug_polygons_allocate_ids() {
        let mut sink = FakePolygons { next: 4 };
        let id = sink.create(2, 1, &[vec3(1.0, 2.0, 3.0)]);
        assert_eq!(id, 4);
        assert_eq!(sink.create(2, 0, &[]), 5);
    }
}
