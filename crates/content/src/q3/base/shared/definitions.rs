//! Quake III shared gameplay definitions (`bg_public.h`, `q_shared.h`).
//!
//! Donor provenance: `src/content/q3/base/shared/definitions.ts` (ported
//! from id Software's `code/game/bg_public.h` and `q_shared.h`,
//! GPL-2.0-or-later). The movement families (`MoveType`, `WeaponState`,
//! `Powerup`, `Holdable`, `Weapon`, `EntityEvent`) are donor enums here; the
//! discriminants reuse the world movement constants so the values stay
//! single-sourced.

use qa_guest::qvm::artifacts::QvmProduct;
use qa_world::movement::q3::constants::{entity_event, holdable, move_type, powerup, weapon, weapon_state};
use qa_world::movement::q3::types::Q3Product;

/// Q3 product: base game or mission pack.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Product {
    /// Base Quake III Arena.
    Baseq3,
    /// Team Arena mission pack.
    Missionpack,
}

impl Product {
    /// Donor spelling.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Baseq3 => "baseq3",
            Self::Missionpack => "missionpack",
        }
    }
}

impl From<Product> for Q3Product {
    fn from(product: Product) -> Self {
        match product {
            Product::Baseq3 => Self::BaseQ3,
            Product::Missionpack => Self::MissionPack,
        }
    }
}

impl From<Q3Product> for Product {
    fn from(product: Q3Product) -> Self {
        match product {
            Q3Product::BaseQ3 => Self::Baseq3,
            Q3Product::MissionPack => Self::Missionpack,
        }
    }
}

impl From<QvmProduct> for Product {
    fn from(product: QvmProduct) -> Self {
        match product {
            QvmProduct::Baseq3 => Self::Baseq3,
            QvmProduct::Missionpack => Self::Missionpack,
        }
    }
}

/// Default gravity.
pub const DEFAULT_GRAVITY: i32 = 800;
/// Health at or below which bodies gib.
pub const GIB_HEALTH: i32 = -40;
/// Armor protection fraction.
pub const ARMOR_PROTECTION: f64 = 0.66;
/// Maximum item definitions.
pub const MAX_ITEMS: i32 = 256;
/// Milliseconds an event stays valid.
pub const EVENT_VALID_MSEC: i32 = 300;
/// First event bit.
pub const EV_EVENT_BIT1: i32 = 0x100;
/// Second event bit.
pub const EV_EVENT_BIT2: i32 = 0x200;
/// Both event bits.
pub const EV_EVENT_BITS: i32 = 0x300;

/// Game type.
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
    Gt1fctf = 5,
    /// Overload obelisk.
    GtObelisk = 6,
    /// Harvester.
    GtHarvester = 7,
    /// Game type count.
    GtMaxGameType = 8,
}

impl GameType {
    /// Raw source value lookup.
    #[must_use]
    pub const fn from_i32(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::GtFfa),
            1 => Some(Self::GtTournament),
            2 => Some(Self::GtSinglePlayer),
            3 => Some(Self::GtTeam),
            4 => Some(Self::GtCtf),
            5 => Some(Self::Gt1fctf),
            6 => Some(Self::GtObelisk),
            7 => Some(Self::GtHarvester),
            8 => Some(Self::GtMaxGameType),
            _ => None,
        }
    }
}

/// Team.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum Team {
    /// Free.
    TeamFree = 0,
    /// Red.
    TeamRed = 1,
    /// Blue.
    TeamBlue = 2,
    /// Spectator.
    TeamSpectator = 3,
    /// Team count.
    TeamNumTeams = 4,
}

impl Team {
    /// Raw source value lookup.
    #[must_use]
    pub const fn from_i32(value: i32) -> Option<Self> {
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

/// Item type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum ItemType {
    /// Reserved empty item.
    ItBad = 0,
    /// Weapon.
    ItWeapon = 1,
    /// Ammo.
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

/// Entity type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum EntityType {
    /// General.
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
    /// Team.
    EtTeam = 12,
    /// Events.
    EtEvents = 13,
}

/// Persistent player-state index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum PersistentIndex {
    /// Score.
    PersScore = 0,
    /// Hits.
    PersHits = 1,
    /// Rank.
    PersRank = 2,
    /// Team.
    PersTeam = 3,
    /// Spawn count.
    PersSpawnCount = 4,
    /// Player events.
    PersPlayerevents = 5,
    /// Attacker.
    PersAttacker = 6,
    /// Attackee armor.
    PersAttackeeArmor = 7,
    /// Killed.
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
    /// Captures.
    PersCaptures = 14,
}

/// Base-game stat index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum BaseStatIndex {
    /// Health.
    StatHealth = 0,
    /// Holdable item.
    StatHoldableItem = 1,
    /// Weapons.
    StatWeapons = 2,
    /// Armor.
    StatArmor = 3,
    /// Dead yaw.
    StatDeadYaw = 4,
    /// Clients ready.
    StatClientsReady = 5,
    /// Maximum health.
    StatMaxHealth = 6,
}

/// Mission-pack stat index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum MissionpackStatIndex {
    /// Health.
    StatHealth = 0,
    /// Holdable item.
    StatHoldableItem = 1,
    /// Persistant powerup.
    StatPersistantPowerup = 2,
    /// Weapons.
    StatWeapons = 3,
    /// Armor.
    StatArmor = 4,
    /// Dead yaw.
    StatDeadYaw = 5,
    /// Clients ready.
    StatClientsReady = 6,
    /// Maximum health.
    StatMaxHealth = 7,
}

/// Base-game stat layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BaseStatSchema {
    /// Product.
    pub product: Product,
    /// Health stat.
    pub health: i32,
    /// Holdable-item stat.
    pub holdable_item: i32,
    /// Weapons stat.
    pub weapons: i32,
    /// Armor stat.
    pub armor: i32,
    /// Dead-yaw stat.
    pub dead_yaw: i32,
    /// Clients-ready stat.
    pub clients_ready: i32,
    /// Maximum-health stat.
    pub max_health: i32,
}

/// Mission-pack stat layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MissionpackStatSchema {
    /// Product.
    pub product: Product,
    /// Health stat.
    pub health: i32,
    /// Holdable-item stat.
    pub holdable_item: i32,
    /// Persistant-powerup stat.
    pub persistent_powerup: i32,
    /// Weapons stat.
    pub weapons: i32,
    /// Armor stat.
    pub armor: i32,
    /// Dead-yaw stat.
    pub dead_yaw: i32,
    /// Clients-ready stat.
    pub clients_ready: i32,
    /// Maximum-health stat.
    pub max_health: i32,
}

/// Stat layout for a product.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatSchema {
    /// Base-game layout.
    Base(BaseStatSchema),
    /// Mission-pack layout.
    Missionpack(MissionpackStatSchema),
}

/// Stat layout for a product (`statSchema`).
#[must_use]
pub fn stat_schema(product: Product) -> StatSchema {
    match product {
        Product::Baseq3 => StatSchema::Base(BaseStatSchema {
            product,
            health: BaseStatIndex::StatHealth as i32,
            holdable_item: BaseStatIndex::StatHoldableItem as i32,
            weapons: BaseStatIndex::StatWeapons as i32,
            armor: BaseStatIndex::StatArmor as i32,
            dead_yaw: BaseStatIndex::StatDeadYaw as i32,
            clients_ready: BaseStatIndex::StatClientsReady as i32,
            max_health: BaseStatIndex::StatMaxHealth as i32,
        }),
        Product::Missionpack => StatSchema::Missionpack(MissionpackStatSchema {
            product,
            health: MissionpackStatIndex::StatHealth as i32,
            holdable_item: MissionpackStatIndex::StatHoldableItem as i32,
            persistent_powerup: MissionpackStatIndex::StatPersistantPowerup as i32,
            weapons: MissionpackStatIndex::StatWeapons as i32,
            armor: MissionpackStatIndex::StatArmor as i32,
            dead_yaw: MissionpackStatIndex::StatDeadYaw as i32,
            clients_ready: MissionpackStatIndex::StatClientsReady as i32,
            max_health: MissionpackStatIndex::StatMaxHealth as i32,
        }),
    }
}

impl StatSchema {
    /// Health stat slot.
    #[must_use]
    pub fn health(self) -> usize {
        match self {
            Self::Base(layout) => layout.health as usize,
            Self::Missionpack(layout) => layout.health as usize,
        }
    }

    /// Armor stat slot.
    #[must_use]
    pub fn armor(self) -> usize {
        match self {
            Self::Base(layout) => layout.armor as usize,
            Self::Missionpack(layout) => layout.armor as usize,
        }
    }

    /// Holdable-item stat slot.
    #[must_use]
    pub fn holdable_item(self) -> usize {
        match self {
            Self::Base(layout) => layout.holdable_item as usize,
            Self::Missionpack(layout) => layout.holdable_item as usize,
        }
    }

    /// Clients-ready stat slot.
    #[must_use]
    pub fn clients_ready(self) -> usize {
        match self {
            Self::Base(layout) => layout.clients_ready as usize,
            Self::Missionpack(layout) => layout.clients_ready as usize,
        }
    }
}

/// Weapon count for a product.
#[must_use]
pub fn weapon_count(product: Product) -> i32 {
    match product {
        Product::Baseq3 => 11,
        Product::Missionpack => 14,
    }
}

/// Whether a weapon belongs to a product.
#[must_use]
pub fn weapon_available(product: Product, weapon: Weapon) -> bool {
    weapon as i32 > Weapon::WpNone as i32 && (weapon as i32) < weapon_count(product)
}

/// Player movement type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum MoveType {
    /// Normal.
    PmNormal = move_type::NORMAL,
    /// Noclip.
    PmNoclip = move_type::NOCLIP,
    /// Spectator.
    PmSpectator = move_type::SPECTATOR,
    /// Dead.
    PmDead = move_type::DEAD,
    /// Freeze.
    PmFreeze = move_type::FREEZE,
    /// Intermission.
    PmIntermission = move_type::INTERMISSION,
    /// Single-player intermission.
    PmSpintermission = move_type::SPINTERMISSION,
}

/// Weapon state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum WeaponState {
    /// Ready.
    WeaponReady = weapon_state::READY,
    /// Raising.
    WeaponRaising = weapon_state::RAISING,
    /// Dropping.
    WeaponDropping = weapon_state::DROPPING,
    /// Firing.
    WeaponFiring = weapon_state::FIRING,
}

/// Powerup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum Powerup {
    /// None.
    PwNone = powerup::NONE,
    /// Quad.
    PwQuad = powerup::QUAD,
    /// Battlesuit.
    PwBattlesuit = powerup::BATTLESUIT,
    /// Haste.
    PwHaste = powerup::HASTE,
    /// Invisibility.
    PwInvis = powerup::INVIS,
    /// Regen.
    PwRegen = powerup::REGEN,
    /// Flight.
    PwFlight = powerup::FLIGHT,
    /// Red flag.
    PwRedflag = powerup::REDFLAG,
    /// Blue flag.
    PwBlueflag = powerup::BLUEFLAG,
    /// Neutral flag.
    PwNeutralflag = powerup::NEUTRALFLAG,
    /// Scout.
    PwScout = powerup::SCOUT,
    /// Guard.
    PwGuard = powerup::GUARD,
    /// Doubler.
    PwDoubler = powerup::DOUBLER,
    /// Ammo regen.
    PwAmmoregen = powerup::AMMOREGEN,
    /// Invulnerability.
    PwInvulnerability = powerup::INVULNERABILITY,
    /// Powerup count.
    PwNumPowerups = powerup::NUM_POWERUPS,
}

/// Holdable item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum Holdable {
    /// None.
    HiNone = holdable::NONE,
    /// Teleporter.
    HiTeleporter = holdable::TELEPORTER,
    /// Medkit.
    HiMedkit = holdable::MEDKIT,
    /// Kamikaze.
    HiKamikaze = holdable::KAMIKAZE,
    /// Portal.
    HiPortal = holdable::PORTAL,
    /// Invulnerability.
    HiInvulnerability = holdable::INVULNERABILITY,
    /// Holdable count.
    HiNumHoldable = holdable::NUM_HOLDABLE,
}

/// Weapon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum Weapon {
    /// None.
    WpNone = weapon::NONE,
    /// Gauntlet.
    WpGauntlet = weapon::GAUNTLET,
    /// Machinegun.
    WpMachinegun = weapon::MACHINEGUN,
    /// Shotgun.
    WpShotgun = weapon::SHOTGUN,
    /// Grenade launcher.
    WpGrenadeLauncher = weapon::GRENADE_LAUNCHER,
    /// Rocket launcher.
    WpRocketLauncher = weapon::ROCKET_LAUNCHER,
    /// Lightning gun.
    WpLightning = weapon::LIGHTNING,
    /// Railgun.
    WpRailgun = weapon::RAILGUN,
    /// Plasmagun.
    WpPlasmagun = weapon::PLASMAGUN,
    /// BFG.
    WpBfg = weapon::BFG,
    /// Grappling hook.
    WpGrapplingHook = weapon::GRAPPLING_HOOK,
    /// Nailgun.
    WpNailgun = weapon::NAILGUN,
    /// Proximity launcher.
    WpProxLauncher = weapon::PROX_LAUNCHER,
    /// Chaingun.
    WpChaingun = weapon::CHAINGUN,
}

/// Entity event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum EntityEvent {
    /// None.
    EvNone = entity_event::NONE,
    /// Footstep.
    EvFootstep = entity_event::FOOTSTEP,
    /// Metal footstep.
    EvFootstepMetal = entity_event::FOOTSTEP_METAL,
    /// Footsplash.
    EvFootsplash = entity_event::FOOTSPLASH,
    /// Footwade.
    EvFootwade = entity_event::FOOTWADE,
    /// Swim.
    EvSwim = entity_event::SWIM,
    /// Four-unit step.
    EvStep4 = entity_event::STEP_4,
    /// Eight-unit step.
    EvStep8 = entity_event::STEP_8,
    /// Twelve-unit step.
    EvStep12 = entity_event::STEP_12,
    /// Sixteen-unit step.
    EvStep16 = entity_event::STEP_16,
    /// Short fall.
    EvFallShort = entity_event::FALL_SHORT,
    /// Medium fall.
    EvFallMedium = entity_event::FALL_MEDIUM,
    /// Far fall.
    EvFallFar = entity_event::FALL_FAR,
    /// Jump pad.
    EvJumpPad = entity_event::JUMP_PAD,
    /// Jump.
    EvJump = entity_event::JUMP,
    /// Water touch.
    EvWaterTouch = entity_event::WATER_TOUCH,
    /// Water leave.
    EvWaterLeave = entity_event::WATER_LEAVE,
    /// Water under.
    EvWaterUnder = entity_event::WATER_UNDER,
    /// Water clear.
    EvWaterClear = entity_event::WATER_CLEAR,
    /// Item pickup.
    EvItemPickup = entity_event::ITEM_PICKUP,
    /// Global item pickup.
    EvGlobalItemPickup = entity_event::GLOBAL_ITEM_PICKUP,
    /// No ammo.
    EvNoammo = entity_event::NOAMMO,
    /// Change weapon.
    EvChangeWeapon = entity_event::CHANGE_WEAPON,
    /// Fire weapon.
    EvFireWeapon = entity_event::FIRE_WEAPON,
    /// Use item 0.
    EvUseItem0 = entity_event::USE_ITEM0,
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
    EvItemRespawn = entity_event::ITEM_RESPAWN,
    /// Item pop.
    EvItemPop = entity_event::ITEM_POP,
    /// Teleport in.
    EvPlayerTeleportIn = entity_event::PLAYER_TELEPORT_IN,
    /// Teleport out.
    EvPlayerTeleportOut = entity_event::PLAYER_TELEPORT_OUT,
    /// Grenade bounce.
    EvGrenadeBounce = entity_event::GRENADE_BOUNCE,
    /// General sound.
    EvGeneralSound = entity_event::GENERAL_SOUND,
    /// Global sound.
    EvGlobalSound = entity_event::GLOBAL_SOUND,
    /// Global team sound.
    EvGlobalTeamSound = entity_event::GLOBAL_TEAM_SOUND,
    /// Bullet hit flesh.
    EvBulletHitFlesh = entity_event::BULLET_HIT_FLESH,
    /// Bullet hit wall.
    EvBulletHitWall = entity_event::BULLET_HIT_WALL,
    /// Missile hit.
    EvMissileHit = entity_event::MISSILE_HIT,
    /// Missile miss.
    EvMissileMiss = entity_event::MISSILE_MISS,
    /// Missile miss metal.
    EvMissileMissMetal = entity_event::MISSILE_MISS_METAL,
    /// Rail trail.
    EvRailtrail = entity_event::RAILTRAIL,
    /// Shotgun.
    EvShotgun = entity_event::SHOTGUN,
    /// Bullet.
    EvBullet = entity_event::BULLET,
    /// Pain.
    EvPain = entity_event::PAIN,
    /// Death 1.
    EvDeath1 = entity_event::DEATH1,
    /// Death 2.
    EvDeath2 = entity_event::DEATH2,
    /// Death 3.
    EvDeath3 = entity_event::DEATH3,
    /// Obituary.
    EvObituary = entity_event::OBITUARY,
    /// Quad powerup.
    EvPowerupQuad = entity_event::POWERUP_QUAD,
    /// Battlesuit powerup.
    EvPowerupBattlesuit = entity_event::POWERUP_BATTLESUIT,
    /// Regen powerup.
    EvPowerupRegen = entity_event::POWERUP_REGEN,
    /// Gib player.
    EvGibPlayer = entity_event::GIB_PLAYER,
    /// Score plum.
    EvScoreplum = entity_event::SCOREPLUM,
    /// Proximity mine stick.
    EvProximityMineStick = entity_event::PROXIMITY_MINE_STICK,
    /// Proximity mine trigger.
    EvProximityMineTrigger = entity_event::PROXIMITY_MINE_TRIGGER,
    /// Kamikaze.
    EvKamikaze = entity_event::KAMIKAZE,
    /// Obelisk explode.
    EvObeliskexplode = entity_event::OBELISKEXPLODE,
    /// Obelisk pain.
    EvObeliskpain = entity_event::OBELISKPAIN,
    /// Invulnerability impact.
    EvInvulImpact = entity_event::INVUL_IMPACT,
    /// Juiced.
    EvJuiced = entity_event::JUICED,
    /// Lightning bolt.
    EvLightningbolt = entity_event::LIGHTNINGBOLT,
    /// Debug line.
    EvDebugLine = entity_event::DEBUG_LINE,
    /// Stop looping sound.
    EvStoploopingsound = entity_event::STOPLOOPINGSOUND,
    /// Taunt.
    EvTaunt = entity_event::TAUNT,
    /// Taunt yes.
    EvTauntYes = entity_event::TAUNT_YES,
    /// Taunt no.
    EvTauntNo = entity_event::TAUNT_NO,
    /// Taunt follow me.
    EvTauntFollowme = entity_event::TAUNT_FOLLOWME,
    /// Taunt get flag.
    EvTauntGetflag = entity_event::TAUNT_GETFLAG,
    /// Taunt guard base.
    EvTauntGuardbase = entity_event::TAUNT_GUARDBASE,
    /// Taunt patrol.
    EvTauntPatrol = entity_event::TAUNT_PATROL,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enums_match_donor_values() {
        assert_eq!(Weapon::WpNone as i32, 0);
        assert_eq!(Weapon::WpGauntlet as i32, 1);
        assert_eq!(Weapon::WpMachinegun as i32, 2);
        assert_eq!(Weapon::WpGrapplingHook as i32, 10);
        assert_eq!(Weapon::WpNailgun as i32, 11);
        assert_eq!(Weapon::WpChaingun as i32, 13);
        assert_eq!(Powerup::PwQuad as i32, 1);
        assert_eq!(Powerup::PwNeutralflag as i32, 9);
        assert_eq!(Powerup::PwNumPowerups as i32, 15);
        assert_eq!(Holdable::HiTeleporter as i32, 1);
        assert_eq!(Holdable::HiNumHoldable as i32, 6);
        assert_eq!(MoveType::PmNormal as i32, 0);
        assert_eq!(MoveType::PmSpintermission as i32, 6);
        assert_eq!(WeaponState::WeaponReady as i32, 0);
        assert_eq!(WeaponState::WeaponFiring as i32, 3);
        assert_eq!(EntityEvent::EvNone as i32, 0);
        assert_eq!(EntityEvent::EvUseItem0 as i32, 24);
        assert_eq!(EntityEvent::EvUseItem15 as i32, 39);
        assert_eq!(EntityEvent::EvTauntPatrol as i32, 82);
        assert_eq!(GameType::GtCtf as i32, 4);
        assert_eq!(GameType::GtMaxGameType as i32, 8);
        assert_eq!(Team::TeamSpectator as i32, 3);
        assert_eq!(ItemType::ItTeam as i32, 8);
        assert_eq!(EntityType::EtEvents as i32, 13);
        assert_eq!(PersistentIndex::PersCaptures as i32, 14);
        assert_eq!((DEFAULT_GRAVITY, GIB_HEALTH, MAX_ITEMS), (800, -40, 256));
        assert_eq!(ARMOR_PROTECTION, 0.66);
        assert_eq!((EV_EVENT_BIT1, EV_EVENT_BIT2, EV_EVENT_BITS), (0x100, 0x200, 0x300));
        assert_eq!(EVENT_VALID_MSEC, 300);
    }

    #[test]
    fn stat_schemas_match_products() {
        let StatSchema::Base(base) = stat_schema(Product::Baseq3) else {
            panic!("baseq3 must use the base schema");
        };
        assert_eq!(base.product, Product::Baseq3);
        assert_eq!(
            (
                base.health,
                base.holdable_item,
                base.weapons,
                base.armor,
                base.dead_yaw,
                base.clients_ready,
                base.max_health
            ),
            (0, 1, 2, 3, 4, 5, 6)
        );
        let StatSchema::Missionpack(pack) = stat_schema(Product::Missionpack) else {
            panic!("missionpack must use the missionpack schema");
        };
        assert_eq!(pack.product, Product::Missionpack);
        assert_eq!(pack.persistent_powerup, 2);
        assert_eq!(
            (
                pack.health,
                pack.holdable_item,
                pack.weapons,
                pack.armor,
                pack.dead_yaw,
                pack.clients_ready,
                pack.max_health
            ),
            (0, 1, 3, 4, 5, 6, 7)
        );
    }

    #[test]
    fn weapon_availability_is_product_gated() {
        assert_eq!(weapon_count(Product::Baseq3), 11);
        assert_eq!(weapon_count(Product::Missionpack), 14);
        assert!(!weapon_available(Product::Baseq3, Weapon::WpNone));
        assert!(weapon_available(Product::Baseq3, Weapon::WpGrapplingHook));
        assert!(!weapon_available(Product::Baseq3, Weapon::WpNailgun));
        assert!(weapon_available(Product::Missionpack, Weapon::WpChaingun));
    }

    #[test]
    fn products_convert_across_layers() {
        assert_eq!(Product::Baseq3.as_str(), "baseq3");
        assert_eq!(Product::Missionpack.as_str(), "missionpack");
        assert_eq!(Q3Product::from(Product::Baseq3), Q3Product::BaseQ3);
        assert_eq!(Q3Product::from(Product::Missionpack), Q3Product::MissionPack);
        assert_eq!(Product::from(Q3Product::BaseQ3), Product::Baseq3);
        assert_eq!(Product::from(QvmProduct::Missionpack), Product::Missionpack);
    }
    #[test]
    fn product_spellings() {
        assert_eq!(Product::Baseq3.as_str(), "baseq3");
        assert_eq!(Product::Missionpack.as_str(), "missionpack");
    }
    #[test]
    fn game_type_ordering_matches_source_comparisons() {
        assert!((GameType::GtTeam as i32) < (GameType::GtCtf as i32));
        assert!((GameType::GtTournament as i32) < (GameType::GtTeam as i32));
        assert_eq!(GameType::from_i32(4), Some(GameType::GtCtf));
        assert_eq!(GameType::from_i32(99), None);
        assert_eq!(Team::from_i32(3), Some(Team::TeamSpectator));
        assert_eq!(PersistentIndex::PersRank as i32, 2);
    }
    #[test]
    fn stat_schema_slots() {
        let base = stat_schema(Product::Baseq3);
        assert_eq!(base.health(), BaseStatIndex::StatHealth as usize);
        assert_eq!(base.armor(), BaseStatIndex::StatArmor as usize);
        let mission = stat_schema(Product::Missionpack);
        assert_eq!(mission.health(), MissionpackStatIndex::StatHealth as usize);
        assert_eq!(mission.armor(), MissionpackStatIndex::StatArmor as usize);
        assert!((ARMOR_PROTECTION - 0.66).abs() < f64::EPSILON);
    }
}
