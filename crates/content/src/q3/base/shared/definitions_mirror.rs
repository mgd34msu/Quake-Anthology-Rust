//! SIBLING-MIRROR of `src/content/q3/base/shared/definitions.ts`.
//!
//! The canonical port is owned by sibling lane impl-content-q3 and will
//! union-merge at `crate::q3::base::shared::definitions`; this module keeps the
//! predecessor flat-port content so the base group compiles standalone. Delete
//! at unification and re-point imports at the canonical module.

// Intra-group imports: sibling modules split from the same flat port.

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
