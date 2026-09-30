//! Quake III base game simulation (`q3_game_sim`) support: shared mirrors, group error, and tests.
//!
//! Self-containment mirrors: minimal local copies of items the donors import from
//! modules outside this port (sibling q3 donors, engine contracts, and math/text
//! helpers), plus the group error type. Sibling-owned mirrors carry SIBLING-MIRROR
//! notes and unify with the canonical ports at merge time.

use qa_core::identity::ActorId;
use qa_core::math::{add3, length3, scale3, sub3, vec3, Bounds, Plane, Vec3};
use qa_core::numeric::{q_rand, qvm_float_to_int};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::game::entities::*;
use crate::q3::base::game::format::*;

// ---------------------------------------------------------------------------
// Product, shared enumerations, and constants (mirror of
// `shared/definitions.ts` + `movement/q3/constants.ts`).
// ---------------------------------------------------------------------------

/// Q3 product family (`Product`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3Product {
    /// Base Quake III.
    Baseq3,
    /// Team Arena mission pack.
    Missionpack,
}

impl Q3Product {
    /// True for the mission pack.
    #[must_use]
    pub fn is_missionpack(self) -> bool {
        self == Q3Product::Missionpack
    }
}

/// Game type (`GameType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum GameType {
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
    /// Overload obelisk.
    Obelisk = 6,
    /// Harvester.
    Harvester = 7,
    /// Type count.
    MaxGameType = 8,
}

/// Team (`Team`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(i32)]
pub enum Team {
    /// No team.
    #[default]
    Free = 0,
    /// Red team.
    Red = 1,
    /// Blue team.
    Blue = 2,
    /// Spectators.
    Spectator = 3,
    /// Team count.
    NumTeams = 4,
}

/// Item type (`ItemType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum ItemType {
    /// Reserved empty item.
    Bad = 0,
    /// Weapon.
    Weapon = 1,
    /// Ammunition.
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum EntityType {
    /// Generic.
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
    /// Temp-entity event base.
    Events = 13,
}

/// Persistent player score fields (`PersistentIndex`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum PersistentIndex {
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
    /// Player events bitmask.
    PlayerEvents = 5,
    /// Last attacker.
    Attacker = 6,
    /// Packed attackee health/armor.
    AttackeeArmor = 7,
    /// Deaths.
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

/// Base stat slots (`BaseStatIndex`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum BaseStatIndex {
    /// Health.
    Health = 0,
    /// Holdable item.
    HoldableItem = 1,
    /// Owned weapons bitmask.
    Weapons = 2,
    /// Armor.
    Armor = 3,
    /// Dead yaw.
    DeadYaw = 4,
    /// Ready clients.
    ClientsReady = 5,
    /// Maximum health.
    MaxHealth = 6,
}

/// Mission-pack stat slots (`MissionpackStatIndex`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum MissionpackStatIndex {
    /// Health.
    Health = 0,
    /// Holdable item.
    HoldableItem = 1,
    /// Persistent powerup item index.
    PersistantPowerup = 2,
    /// Owned weapons bitmask.
    Weapons = 3,
    /// Armor.
    Armor = 4,
    /// Dead yaw.
    DeadYaw = 5,
    /// Ready clients.
    ClientsReady = 6,
    /// Maximum health.
    MaxHealth = 7,
}

/// Weapon (`Weapon`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// Powerup (`Powerup`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum Powerup {
    /// None.
    None = 0,
    /// Quad damage.
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
    /// Ammo regen.
    Ammoregen = 13,
    /// Invulnerability.
    Invulnerability = 14,
    /// Powerup count.
    NumPowerups = 15,
}

/// Holdable (`Holdable`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    /// Holdable count.
    NumHoldable = 6,
}

/// Movement type (`MoveType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(i32)]
pub enum MoveType {
    /// Normal.
    #[default]
    Normal = 0,
    /// Noclip.
    Noclip = 1,
    /// Spectator.
    Spectator = 2,
    /// Dead.
    Dead = 3,
    /// Frozen.
    Freeze = 4,
    /// Intermission.
    Intermission = 5,
    /// Single-player intermission.
    SpIntermission = 6,
}

/// Weapon state (`WeaponState`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(i32)]
pub enum WeaponState {
    /// Ready.
    #[default]
    Ready = 0,
    /// Raising.
    Raising = 1,
    /// Dropping.
    Dropping = 2,
    /// Firing.
    Firing = 3,
}

/// Entity event (`EntityEvent`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// Player animation (`PlayerAnimation`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum PlayerAnimation {
    /// Death 1.
    BothDeath1 = 0,
    /// Dead 1.
    BothDead1 = 1,
    /// Death 2.
    BothDeath2 = 2,
    /// Dead 2.
    BothDead2 = 3,
    /// Death 3.
    BothDeath3 = 4,
    /// Dead 3.
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
    /// Legs crouch walk.
    LegsWalkcr = 13,
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
    /// Legs jump back.
    LegsJumpb = 20,
    /// Legs land back.
    LegsLandb = 21,
    /// Legs idle.
    LegsIdle = 22,
    /// Legs idle crouch.
    LegsIdlecr = 23,
    /// Legs turn.
    LegsTurn = 24,
    /// Torso get flag.
    TorsoGetflag = 25,
    /// Torso guard base.
    TorsoGuardbase = 26,
    /// Torso patrol.
    TorsoPatrol = 27,
    /// Torso follow me.
    TorsoFollowme = 28,
    /// Torso affirmative.
    TorsoAffirmative = 29,
    /// Torso negative.
    TorsoNegative = 30,
    /// Legs crouch backpedal.
    LegsBackcr = 32,
    /// Legs back walk.
    LegsBackwalk = 33,
    /// Flag run.
    FlagRun = 34,
    /// Flag stand.
    FlagStand = 35,
    /// Flag stand-to-run.
    FlagStand2run = 36,
}

/// Mover state (`MoverState`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(i32)]
pub enum MoverState {
    /// At position 1.
    #[default]
    Pos1 = 0,
    /// At position 2.
    Pos2 = 1,
    /// Moving one to two.
    OneToTwo = 2,
    /// Moving two to one.
    TwoToOne = 3,
}

/// Connection state (`ConnectionState`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(i32)]
pub enum ConnectionState {
    /// Disconnected.
    #[default]
    Disconnected = 0,
    /// Connecting.
    Connecting = 1,
    /// Connected.
    Connected = 2,
}

/// Spectator state (`SpectatorState`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(i32)]
pub enum SpectatorState {
    /// Not spectating.
    #[default]
    Not = 0,
    /// Free spectator.
    Free = 1,
    /// Following.
    Follow = 2,
    /// Scoreboard.
    Scoreboard = 3,
}

/// Trajectory type (`TrajectoryType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(i32)]
pub enum TrajectoryType {
    /// Stationary.
    #[default]
    Stationary = 0,
    /// Interpolated.
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

/// Movement flags (`MoveFlags`).
pub mod move_flags {
    /// Knockback timer active.
    pub const TIME_KNOCKBACK: i32 = 64;
    /// Grapple pull active.
    pub const GRAPPLE_PULL: i32 = 2048;
}

/// Game entity flags (`GameFlags`).
pub mod game_flags {
    /// God mode.
    pub const GODMODE: i32 = 0x10;
    /// Item team slave.
    pub const TEAMSLAVE: i32 = 0x400;
    /// Immune to knockback.
    pub const NO_KNOCKBACK: i32 = 0x800;
    /// Dropped item.
    pub const DROPPED_ITEM: i32 = 0x1000;
}

/// Server entity flags (`ServerEntityFlags`).
pub mod server_entity_flags {
    /// Not transmitted.
    pub const NOCLIENT: i32 = 1;
    /// Broadcast.
    pub const BROADCAST: i32 = 32;
    /// Single client.
    pub const SINGLECLIENT: i32 = 256;
}

/// Damage flags (`DamageFlags`, combat.ts).
pub mod damage_flags {
    /// Radius damage.
    pub const RADIUS: i32 = 0x1;
    /// Bypass armor.
    pub const NO_ARMOR: i32 = 0x2;
    /// No knockback.
    pub const NO_KNOCKBACK: i32 = 0x4;
    /// Bypass protection.
    pub const NO_PROTECTION: i32 = 0x8;
    /// Bypass team protection.
    pub const NO_TEAM_PROTECTION: i32 = 0x10;
}

/// World entity number (`ENTITYNUM_WORLD`).
pub const ENTITYNUM_WORLD: i32 = 1022;

/// Null entity number (`ENTITYNUM_NONE`).
pub const ENTITYNUM_NONE: i32 = 1023;

/// Maximum clients (`MAX_CLIENTS`).
pub const MAX_CLIENTS: usize = 64;

/// Maximum entities (`MAX_GENTITIES`).
pub const MAX_GENTITIES: usize = 1024;

/// Gib health threshold (`GIB_HEALTH`).
pub const GIB_HEALTH: i32 = -40;

/// Armor protection fraction (`ARMOR_PROTECTION`).
pub const ARMOR_PROTECTION: f32 = 0.66;

/// Event lifetime milliseconds (`EVENT_VALID_MSEC`).
pub const EVENT_VALID_MSEC: i32 = 300;

/// Event sequence bit 1 (`EV_EVENT_BIT1`).
pub const EV_EVENT_BIT1: i32 = 0x100;

/// Event sequence bits (`EV_EVENT_BITS`).
pub const EV_EVENT_BITS: i32 = 0x300;

/// Stat slot schema (`StatSchema` plus product tag).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatSchema {
    /// Product.
    pub product: Q3Product,
    /// Health slot.
    pub health: i32,
    /// Holdable item slot.
    pub holdable_item: i32,
    /// Weapons bitmask slot.
    pub weapons: i32,
    /// Armor slot.
    pub armor: i32,
    /// Dead yaw slot.
    pub dead_yaw: i32,
    /// Ready clients slot.
    pub clients_ready: i32,
    /// Maximum health slot.
    pub max_health: i32,
    /// Persistent powerup slot (mission pack only).
    pub persistent_powerup: Option<i32>,
}

/// Stat schema for a product (`statSchema`).
#[must_use]
pub fn stat_schema(product: Q3Product) -> StatSchema {
    match product {
        Q3Product::Baseq3 => StatSchema {
            product,
            health: BaseStatIndex::Health as i32,
            holdable_item: BaseStatIndex::HoldableItem as i32,
            weapons: BaseStatIndex::Weapons as i32,
            armor: BaseStatIndex::Armor as i32,
            dead_yaw: BaseStatIndex::DeadYaw as i32,
            clients_ready: BaseStatIndex::ClientsReady as i32,
            max_health: BaseStatIndex::MaxHealth as i32,
            persistent_powerup: None,
        },
        Q3Product::Missionpack => StatSchema {
            product,
            health: MissionpackStatIndex::Health as i32,
            holdable_item: MissionpackStatIndex::HoldableItem as i32,
            weapons: MissionpackStatIndex::Weapons as i32,
            armor: MissionpackStatIndex::Armor as i32,
            dead_yaw: MissionpackStatIndex::DeadYaw as i32,
            clients_ready: MissionpackStatIndex::ClientsReady as i32,
            max_health: MissionpackStatIndex::MaxHealth as i32,
            persistent_powerup: Some(MissionpackStatIndex::PersistantPowerup as i32),
        },
    }
}

/// Weapon slot count for a product (`weaponCount`).
#[must_use]
pub fn weapon_count(product: Q3Product) -> usize {
    match product {
        Q3Product::Baseq3 => 11,
        Q3Product::Missionpack => 14,
    }
}

// ---------------------------------------------------------------------------
// Game random (mirror of `game/numeric.ts` `GameRandom`).
// ---------------------------------------------------------------------------

/// Random operations used across the simulation.
pub trait SimRandom {
    /// `rand()` nonnegative `0..=0x7fff` draw.
    fn rand_value(&mut self) -> i32;
    /// `random()` unit draw.
    fn random_value(&mut self) -> f32;
    /// `crandom()` centered draw.
    fn crandom_value(&mut self) -> f32;
}

/// Instance-owned `bg_lib` rand/srand (`GameRandom`).
#[derive(Debug, Clone)]
pub struct GameRandomMirror {
    /// Current seed.
    seed: i32,
}

impl GameRandomMirror {
    /// Build with a seed (`new GameRandom(seed)`).
    #[must_use]
    pub fn new(seed: i32) -> Self {
        Self { seed }
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

    /// Next `rand()` draw.
    pub fn rand_value(&mut self) -> i32 {
        self.seed = q_rand(self.seed);
        self.seed & 0x7fff
    }

    /// Next `random()` draw.
    pub fn random_value(&mut self) -> f32 {
        let draw = self.rand_value();
        (f64::from(draw) / f64::from(0x7fff)) as f32
    }

    /// Next `crandom()` draw.
    pub fn crandom_value(&mut self) -> f32 {
        let inner = (f64::from(self.random_value()) - 0.5) as f32;
        (2.0 * f64::from(inner)) as f32
    }
}

impl SimRandom for GameRandomMirror {
    fn rand_value(&mut self) -> i32 {
        GameRandomMirror::rand_value(self)
    }

    fn random_value(&mut self) -> f32 {
        GameRandomMirror::random_value(self)
    }

    fn crandom_value(&mut self) -> f32 {
        GameRandomMirror::crandom_value(self)
    }
}

// ---------------------------------------------------------------------------
// Trajectory, entity state, shared state (mirrors of `shared/trajectory.ts`,
// network `entity.ts`, `shared/entity-shared.ts`).
// ---------------------------------------------------------------------------

/// Trajectory record (`Trajectory`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Trajectory {
    /// Trajectory type.
    pub traj_type: TrajectoryType,
    /// Start time milliseconds.
    pub time: i32,
    /// Duration milliseconds.
    pub duration: i32,
    /// Base position.
    pub base: Vec3,
    /// Delta (velocity or amplitude).
    pub delta: Vec3,
}

impl Default for Trajectory {
    fn default() -> Self {
        Self {
            traj_type: TrajectoryType::Stationary,
            time: 0,
            duration: 0,
            base: vec3(0.0, 0.0, 0.0),
            delta: vec3(0.0, 0.0, 0.0),
        }
    }
}

/// Owned `entityState_t` storage (`EntityState`).
#[derive(Debug, Clone, PartialEq)]
pub struct EntityState {
    /// Entity number.
    pub number: i32,
    /// Entity type.
    pub e_type: i32,
    /// Entity flags.
    pub e_flags: i32,
    /// Position trajectory.
    pub pos: Trajectory,
    /// Angle trajectory.
    pub apos: Trajectory,
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
    /// Powerups bitmask.
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

impl Default for EntityState {
    fn default() -> Self {
        Self {
            number: 0,
            e_type: 0,
            e_flags: 0,
            pos: Trajectory::default(),
            apos: Trajectory::default(),
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

/// Server-side entity collision record (`EntityShared` plus body ground).
#[derive(Debug, Clone, PartialEq)]
pub struct EntityShared {
    /// Server flags.
    pub sv_flags: i32,
    /// Single-client target.
    pub single_client: i32,
    /// Contents mask.
    pub contents: i32,
    /// Owner entity number.
    pub owner_num: i32,
    /// Bounding minimum.
    pub mins: Vec3,
    /// Bounding maximum.
    pub maxs: Vec3,
    /// Current collision origin.
    pub current_origin: Vec3,
    /// Current collision angles.
    pub current_angles: Vec3,
    /// Supporting ground actor.
    pub ground: Option<ActorId>,
    /// Linked into collision.
    pub linked: bool,
    /// Link count.
    pub link_count: i32,
}

impl Default for EntityShared {
    fn default() -> Self {
        Self {
            sv_flags: 0,
            single_client: 0,
            contents: 0,
            owner_num: 0,
            mins: vec3(0.0, 0.0, 0.0),
            maxs: vec3(0.0, 0.0, 0.0),
            current_origin: vec3(0.0, 0.0, 0.0),
            current_angles: vec3(0.0, 0.0, 0.0),
            ground: None,
            linked: false,
            link_count: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Player state (mirror of `shared/player-state.ts`).
// ---------------------------------------------------------------------------

/// Integer slot vector (`PlayerStateSlots`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerStateSlots {
    /// Slot values.
    values: Vec<i32>,
}

impl PlayerStateSlots {
    /// Zeroed slots of a length.
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

    /// True when there are no slots.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Read a slot (`get`).
    #[must_use]
    pub fn get(&self, index: i32) -> i32 {
        if index < 0 || index as usize >= self.values.len() {
            panic!("Player state slot {} outside {}", index, self.values.len());
        }
        self.values[index as usize]
    }

    /// Write a slot (`set`).
    pub fn set(&mut self, index: i32, value: i32) {
        if index < 0 || index as usize >= self.values.len() {
            panic!("Player state slot {} outside {}", index, self.values.len());
        }
        self.values[index as usize] = value;
    }

    /// Copy all slot values (`copy`).
    #[must_use]
    pub fn copy(&self) -> Vec<i32> {
        self.values.clone()
    }
}

/// Predictable event record (`PredictableEvent`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PredictableEvent {
    /// Sequence number.
    pub sequence: i32,
    /// Event id.
    pub event: i32,
    /// Event parameter.
    pub parameter: i32,
}

/// Event-debug module side (`PredictableEventDebug.module`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventDebugModule {
    /// Game module.
    Game,
    /// Client game module.
    Cgame,
}

/// Predictable-event debug sink (`PredictableEventDebug`).
#[derive(Clone)]
pub struct EventDebugMirror {
    /// Module side.
    pub module: EventDebugModule,
    /// `showEvents()` value reader.
    pub show_events: Rc<dyn Fn() -> String>,
    /// Debug printer.
    pub print: Rc<dyn Fn(String)>,
}

// `bg_misc.c:eventnames`, including its omitted `EV_OBELISKPAIN` entry.
pub(crate) const EVENT_NAMES: [&str; 76] = [
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

/// Minimal leading-number scan matching `nativeAtof` for event-debug flags.
pub(crate) fn debug_flag(text: &str) -> f64 {
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() && bytes[index].is_ascii_whitespace() {
        index += 1;
    }
    let mut sign = 1.0;
    if index < bytes.len() && (bytes[index] == b'+' || bytes[index] == b'-') {
        if bytes[index] == b'-' {
            sign = -1.0;
        }
        index += 1;
    }
    let mut value = 0.0;
    while index < bytes.len() && bytes[index].is_ascii_digit() {
        value = value * 10.0 + f64::from(bytes[index] - b'0');
        index += 1;
    }
    if index < bytes.len() && bytes[index] == b'.' {
        index += 1;
        let mut place = 0.1;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            value += f64::from(bytes[index] - b'0') * place;
            place *= 0.1;
            index += 1;
        }
    }
    sign * value
}

/// Owned `playerState_t` storage (`PlayerState`).
#[derive(Clone)]
pub struct PlayerState {
    /// Product.
    pub product: Q3Product,
    /// Movement type.
    pub pm_type: MoveType,
    /// Movement flags.
    pub pm_flags: i32,
    /// Movement timer.
    pub pm_time: i32,
    /// Authoritative origin.
    pub origin: Vec3,
    /// Legs animation.
    pub legs_anim: i32,
    /// Torso animation.
    pub torso_anim: i32,
    /// Entity flags.
    pub e_flags: i32,
    /// Predictable-event sequence.
    pub event_sequence: i32,
    /// Predictable events ring.
    pub events: PlayerStateSlots,
    /// Predictable event parameters ring.
    pub event_parms: PlayerStateSlots,
    /// External event.
    pub external_event: i32,
    /// External event parameter.
    pub external_event_parm: i32,
    /// External event time.
    pub external_event_time: i32,
    /// Client number.
    pub client_num: i32,
    /// Current weapon.
    pub weapon: Weapon,
    /// Weapon state.
    pub weapon_state: WeaponState,
    /// View angles.
    pub viewangles: Vec3,
    /// Stats slots.
    pub stats: PlayerStateSlots,
    /// Persistent score slots.
    pub persistant: PlayerStateSlots,
    /// Powerup timer slots.
    pub powerups: PlayerStateSlots,
    /// Ammo slots.
    pub ammo: PlayerStateSlots,
    /// Generic 1.
    pub generic1: i32,
    /// Pmove frame count (event debug only).
    pub pmove_framecount: i32,
    /// Event debug sink.
    event_debug: Option<EventDebugMirror>,
}

impl PlayerState {
    /// Zeroed player state for a product (`createPlayerState`).
    #[must_use]
    pub fn new(product: Q3Product) -> Self {
        Self {
            product,
            pm_type: MoveType::Normal,
            pm_flags: 0,
            pm_time: 0,
            origin: vec3(0.0, 0.0, 0.0),
            legs_anim: 0,
            torso_anim: 0,
            e_flags: 0,
            event_sequence: 0,
            events: PlayerStateSlots::new(2),
            event_parms: PlayerStateSlots::new(2),
            external_event: 0,
            external_event_parm: 0,
            external_event_time: 0,
            client_num: 0,
            weapon: Weapon::None,
            weapon_state: WeaponState::Ready,
            viewangles: vec3(0.0, 0.0, 0.0),
            stats: PlayerStateSlots::new(16),
            persistant: PlayerStateSlots::new(16),
            powerups: PlayerStateSlots::new(16),
            ammo: PlayerStateSlots::new(16),
            generic1: 0,
            pmove_framecount: 0,
            event_debug: None,
        }
    }

    /// Install or clear the event-debug sink (`setEventDebug`).
    pub fn set_event_debug(&mut self, debug: Option<EventDebugMirror>) {
        self.event_debug = debug;
    }

    /// Queue a predictable event (`addEvent`).
    pub fn add_event(&mut self, event: i32, parameter: i32) -> PredictableEvent {
        if let Some(debug) = self.event_debug.clone() {
            let flag: String = (debug.show_events)().chars().take(255).collect();
            if debug_flag(&flag) != 0.0 {
                let name = if event >= 0 && (event as usize) < EVENT_NAMES.len() {
                    EVENT_NAMES[event as usize]
                } else {
                    panic!("bg_misc.c eventnames has no entry for {event}");
                };
                let label = match debug.module {
                    EventDebugModule::Game => " game",
                    EventDebugModule::Cgame => "Cgame",
                };
                (debug.print)(format!(
                    "{label} event svt {:>5} -> {:>5}: num = {:>20} parm {parameter}\n",
                    self.pmove_framecount, self.event_sequence, name
                ));
            }
        }
        let sequence = self.event_sequence;
        self.events.set(sequence & 1, event);
        self.event_parms.set(sequence & 1, parameter);
        self.event_sequence = sequence.wrapping_add(1);
        PredictableEvent {
            sequence,
            event,
            parameter,
        }
    }
}

// ---------------------------------------------------------------------------
// Client (mirror of `game/state.ts` `GameClient`).
// ---------------------------------------------------------------------------

/// User command, weapon selection only (`UserCommand`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UserCommand {
    /// Selected weapon.
    pub weapon: i32,
}

/// Persistent client data (`ClientPersistant`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientPersistant {
    /// Connection state.
    pub connected: ConnectionState,
    /// Latest user command.
    pub cmd: UserCommand,
    /// Predict item pickups.
    pub predict_item_pickup: bool,
    /// Network name.
    pub netname: String,
}

impl Default for ClientPersistant {
    fn default() -> Self {
        Self {
            connected: ConnectionState::Disconnected,
            cmd: UserCommand::default(),
            predict_item_pickup: false,
            netname: String::new(),
        }
    }
}

/// Session data (`ClientSession`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientSession {
    /// Session team.
    pub session_team: Team,
    /// Spectator state.
    pub spectator_state: SpectatorState,
    /// Followed client number.
    pub spectator_client: i32,
}

impl Default for ClientSession {
    fn default() -> Self {
        Self {
            session_team: Team::Free,
            spectator_state: SpectatorState::Not,
            spectator_client: 0,
        }
    }
}

/// Source-zero game client (`GameClient`).
#[derive(Clone)]
pub struct GameClient {
    /// Player state.
    pub ps: PlayerState,
    /// Persistent data.
    pub pers: ClientPersistant,
    /// Session data.
    pub sess: ClientSession,
    /// Noclip cheat.
    pub noclip: bool,
    /// Absorbed armor damage accumulator.
    pub damage_armor: i32,
    /// Blood damage accumulator.
    pub damage_blood: i32,
    /// Knockback accumulator.
    pub damage_knockback: i32,
    /// Damage source direction.
    pub damage_from: Vec3,
    /// Damage came from the world.
    pub damage_from_world: bool,
    /// Last killed client.
    pub last_killed_client: i32,
    /// Last hurt client.
    pub last_hurt_client: i32,
    /// Last hurt means of death.
    pub last_hurt_mod: i32,
    /// Respawn time.
    pub respawn_time: i32,
    /// Reward expiry time.
    pub reward_time: i32,
    /// Last kill time.
    pub last_kill_time: i32,
    /// Active grapple hook.
    pub hook: Option<EntityRef>,
    /// Carried persistent powerup.
    pub persistant_powerup: Option<EntityRef>,
    /// Invulnerability expiry time.
    pub invulnerability_time: i32,
    /// Per-weapon ammo timers.
    pub ammo_times: PlayerStateSlots,
}

impl GameClient {
    /// Zeroed client for a product (`new GameClient(product)`).
    #[must_use]
    pub fn new(product: Q3Product) -> Self {
        Self {
            ps: PlayerState::new(product),
            pers: ClientPersistant::default(),
            sess: ClientSession::default(),
            noclip: false,
            damage_armor: 0,
            damage_blood: 0,
            damage_knockback: 0,
            damage_from: vec3(0.0, 0.0, 0.0),
            damage_from_world: false,
            last_killed_client: 0,
            last_hurt_client: 0,
            last_hurt_mod: 0,
            respawn_time: 0,
            reward_time: 0,
            last_kill_time: 0,
            hook: None,
            persistant_powerup: None,
            invulnerability_time: 0,
            ammo_times: PlayerStateSlots::new(weapon_count(product)),
        }
    }
}

// ---------------------------------------------------------------------------
// Identity and items (mirrors of `contracts/identity.ts`, `shared/items.ts`).
// ---------------------------------------------------------------------------

/// Provider identity (`ProviderId`).
pub type ProviderId = String;

/// Namespaced item identity (`ItemId`).
pub type ItemId = String;

/// Actor bound to its owning provider (`OwnedActor`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedActor {
    /// Actor handle.
    pub id: ActorId,
    /// Owning provider.
    pub owner: ProviderId,
}

/// Item definition (`ItemDefinition`).
#[derive(Debug, Clone, PartialEq)]
pub struct ItemDefinition {
    /// Spawn class name.
    pub class_name: Option<String>,
    /// Pickup display name.
    pub pickup_name: Option<String>,
    /// Default quantity.
    pub quantity: i32,
    /// Item type.
    pub item_type: ItemType,
    /// Type tag (weapon, powerup, or holdable id, else 0).
    pub tag: i32,
}

/// Product item table (`itemList` plus `BG_*` lookup helpers).
#[derive(Debug, Clone)]
pub struct Q3ItemTable {
    /// Table entries; index zero is the reserved empty item.
    pub items: Vec<ItemDefinition>,
}

impl Q3ItemTable {
    /// Build from entries.
    #[must_use]
    pub fn new(items: Vec<ItemDefinition>) -> Self {
        Self { items }
    }

    /// Entry count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// True when the table is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Entry by index (`itemAt`).
    #[must_use]
    pub fn item_at(&self, index: i32) -> &ItemDefinition {
        if index < 0 || index as usize >= self.items.len() {
            panic!("Item index out of range: {index}");
        }
        &self.items[index as usize]
    }

    /// Index of an entry (`indexOf`).
    #[must_use]
    pub fn index_of(&self, item: &ItemDefinition) -> Option<usize> {
        self.items.iter().position(|entry| entry == item)
    }

    /// Entry by pickup name, ASCII case-insensitive (`findItem`).
    #[must_use]
    pub fn find_item(&self, pickup_name: &str) -> Option<&ItemDefinition> {
        let folded = pickup_name.to_ascii_lowercase();
        self.items.iter().find(|item| {
            item.pickup_name
                .as_ref()
                .is_some_and(|name| name.to_ascii_lowercase() == folded)
        })
    }

    /// Entry granting a powerup (`findItemForPowerup`).
    #[must_use]
    pub fn find_item_for_powerup(&self, powerup: i32) -> Option<&ItemDefinition> {
        self.items.iter().find(|item| {
            matches!(
                item.item_type,
                ItemType::Powerup | ItemType::Team | ItemType::PersistantPowerup
            ) && item.tag == powerup
        })
    }

    /// Entry granting a weapon (`findItemForWeapon`).
    #[must_use]
    pub fn find_item_for_weapon(&self, weapon: i32) -> &ItemDefinition {
        self.items
            .iter()
            .find(|item| item.item_type == ItemType::Weapon && item.tag == weapon)
            .unwrap_or_else(|| panic!("Couldn't find item for weapon {weapon}"))
    }

    /// Armor pickup eligibility (`canQ3ArmorBeGrabbed`).
    #[must_use]
    pub fn can_q3_armor_be_grabbed(&self, ps: &PlayerInventory) -> bool {
        if ps.product == Q3Product::Missionpack {
            if self.item_at(ps.persistent_powerup_index).tag == Powerup::Scout as i32 {
                return false;
            }
            let upper = if self.item_at(ps.persistent_powerup_index).tag == Powerup::Guard as i32 {
                ps.max_health
            } else {
                ps.max_health * 2
            };
            return ps.armor < upper;
        }
        ps.armor < ps.max_health * 2
    }

    /// Pickup eligibility (`canItemBeGrabbed`).
    #[must_use]
    pub fn can_item_be_grabbed(&self, gametype: i32, ent: &PickupEntity, ps: &PlayerInventory) -> bool {
        if ent.model_index < 1 || ent.model_index as usize >= self.items.len() {
            panic!("BG_CanItemBeGrabbed: index out of range");
        }
        let item = self.item_at(ent.model_index);
        match item.item_type {
            ItemType::Weapon => true,
            ItemType::Ammo => ps.ammo(item.tag) < 200,
            ItemType::Armor => self.can_q3_armor_be_grabbed(ps),
            ItemType::Health => {
                if ps.product == Q3Product::Missionpack
                    && self.item_at(ps.persistent_powerup_index).tag == Powerup::Guard as i32
                {
                    return ps.health < ps.max_health;
                }
                ps.health < ps.max_health * (i32::from(item.quantity == 5 || item.quantity == 100) + 1)
            }
            ItemType::Powerup => true,
            ItemType::PersistantPowerup => {
                if ps.product == Q3Product::Baseq3 || ps.persistent_powerup_index != 0 {
                    return false;
                }
                if (ent.generic1 & 2) != 0 && ps.team != Team::Red as i32 {
                    return false;
                }
                if (ent.generic1 & 4) != 0 && ps.team != Team::Blue as i32 {
                    return false;
                }
                true
            }
            ItemType::Team => {
                if ps.product == Q3Product::Missionpack && gametype == GameType::OneFctf as i32 {
                    if item.tag == Powerup::Neutralflag as i32 {
                        return true;
                    }
                    if ps.team == Team::Red as i32
                        && item.tag == Powerup::Blueflag as i32
                        && ps.powerup(Powerup::Neutralflag as i32) != 0
                    {
                        return true;
                    }
                    if ps.team == Team::Blue as i32
                        && item.tag == Powerup::Redflag as i32
                        && ps.powerup(Powerup::Neutralflag as i32) != 0
                    {
                        return true;
                    }
                }
                if gametype == GameType::Ctf as i32 {
                    if ps.team == Team::Red as i32 {
                        return item.tag == Powerup::Blueflag as i32
                            || (item.tag == Powerup::Redflag as i32
                                && (ent.model_index2 != 0 || ps.powerup(Powerup::Blueflag as i32) != 0));
                    }
                    if ps.team == Team::Blue as i32 {
                        return item.tag == Powerup::Redflag as i32
                            || (item.tag == Powerup::Blueflag as i32
                                && (ent.model_index2 != 0 || ps.powerup(Powerup::Redflag as i32) != 0));
                    }
                }
                ps.product == Q3Product::Missionpack && gametype == GameType::Harvester as i32
            }
            ItemType::Holdable => ps.holdable_item == 0,
            ItemType::Bad => panic!("BG_CanItemBeGrabbed: IT_BAD"),
        }
    }
}

/// Pickup eligibility entity fields (`PickupEntity`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PickupEntity {
    /// Item table model index.
    pub model_index: i32,
    /// Dropped marker.
    pub model_index2: i32,
    /// Generic flags.
    pub generic1: i32,
}

/// Player inventory view (`PlayerInventory`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerInventory {
    /// Product.
    pub product: Q3Product,
    /// Health.
    pub health: i32,
    /// Armor.
    pub armor: i32,
    /// Maximum health.
    pub max_health: i32,
    /// Holdable item.
    pub holdable_item: i32,
    /// Team.
    pub team: i32,
    /// Ammo slots.
    pub ammo: Vec<i32>,
    /// Powerup timer slots.
    pub powerups: Vec<i32>,
    /// Persistent powerup item index (mission pack).
    pub persistent_powerup_index: i32,
}

impl PlayerInventory {
    /// Ammo for a weapon tag.
    #[must_use]
    pub fn ammo(&self, weapon: i32) -> i32 {
        self.ammo.get(weapon as usize).copied().unwrap_or(0)
    }

    /// Powerup timer for a powerup tag.
    #[must_use]
    pub fn powerup(&self, powerup: i32) -> i32 {
        self.powerups.get(powerup as usize).copied().unwrap_or(0)
    }
}

/// Inventory view of a client (`q3ItemInventory`).
#[must_use]
pub fn q3_item_inventory(client: &GameClient) -> PlayerInventory {
    let ps = &client.ps;
    let schema = stat_schema(ps.product);
    PlayerInventory {
        product: ps.product,
        health: ps.stats.get(schema.health),
        armor: ps.stats.get(schema.armor),
        max_health: ps.stats.get(schema.max_health),
        holdable_item: ps.stats.get(schema.holdable_item),
        team: ps.persistant.get(PersistentIndex::Team as i32),
        ammo: ps.ammo.copy(),
        powerups: ps.powerups.copy(),
        persistent_powerup_index: if ps.product == Q3Product::Missionpack {
            ps.stats.get(MissionpackStatIndex::PersistantPowerup as i32)
        } else {
            0
        },
    }
}

// ---------------------------------------------------------------------------
// Game entity, participants, callbacks (mirrors of `game/state.ts`,
// `game/use-participant.ts`, `game/save-callbacks.ts`).
// ---------------------------------------------------------------------------

/// Shared entity handle.
pub type EntityRef = Rc<RefCell<GameEntity>>;

/// Pool handle shared with saved callbacks.
pub type PoolHandle = Rc<RefCell<EntityPool>>;

/// Non-native damage participant (`{ kind: "shared-actor", ... }`).
#[derive(Debug, Clone, PartialEq)]
pub struct SharedActor {
    /// Actor handle.
    pub actor: ActorId,
    /// World origin, when known.
    pub origin: Option<Vec3>,
}

/// Damage/use participant (`DamageParticipant` / `UseParticipant`).
#[derive(Clone)]
pub enum DamageParticipant {
    /// Native game entity.
    Native(EntityRef),
    /// Shared actor outside the game DLL.
    Shared(SharedActor),
}

/// Actor handle of a participant (`useActor`).
#[must_use]
pub fn use_actor(participant: &DamageParticipant) -> ActorId {
    match participant {
        DamageParticipant::Native(entity) => entity.borrow().actor.id.clone(),
        DamageParticipant::Shared(shared) => shared.actor.clone(),
    }
}

/// Touch contact surface (`TouchContact.surface`).
#[derive(Debug, Clone, PartialEq)]
pub struct TouchSurfaceMirror {
    /// Surface name.
    pub name: String,
    /// Native flags.
    pub native_flags: i32,
    /// Native value.
    pub native_value: i32,
}

/// Touch contact (`TouchContact`).
#[derive(Debug, Clone, PartialEq)]
pub struct TouchContactMirror {
    /// Touching entity.
    pub this_actor: OwnedActor,
    /// Touched actor.
    pub other: ActorId,
    /// Contact plane.
    pub plane: Option<Plane>,
    /// Contact surface.
    pub surface: Option<TouchSurfaceMirror>,
}

/// Think callback (`EntityThink`).
pub type ThinkCallback = Rc<dyn Fn(EntityRef)>;

/// Reached callback (same shape as think).
pub type ReachedCallback = Rc<dyn Fn(EntityRef)>;

/// Blocked callback (`EntityBlocked`).
pub type BlockedCallback = Rc<dyn Fn(EntityRef, DamageParticipant)>;

/// Touch callback (`EntityTouch`).
pub type TouchCallback = Rc<dyn Fn(EntityRef, DamageParticipant, TouchContactMirror)>;

/// Use callback (`EntityUse`).
pub type UseCallback = Rc<dyn Fn(EntityRef, Option<DamageParticipant>, Option<DamageParticipant>)>;

/// Pain callback (`EntityPain`).
pub type PainCallback = Rc<dyn Fn(EntityRef, DamageParticipant, i32)>;

/// Die callback (`EntityDie`).
pub type DieCallback = Rc<dyn Fn(EntityRef, Option<DamageParticipant>, Option<DamageParticipant>, i32, i32)>;

/// Function identity for callback families.
pub trait CallbackIdentity {
    /// True when both handles name the same callback.
    fn same_callback(left: &Self, right: &Self) -> bool;
}

impl CallbackIdentity for ThinkCallback {
    fn same_callback(left: &Self, right: &Self) -> bool {
        Rc::ptr_eq(left, right)
    }
}

impl CallbackIdentity for BlockedCallback {
    fn same_callback(left: &Self, right: &Self) -> bool {
        Rc::ptr_eq(left, right)
    }
}

impl CallbackIdentity for TouchCallback {
    fn same_callback(left: &Self, right: &Self) -> bool {
        Rc::ptr_eq(left, right)
    }
}

impl CallbackIdentity for UseCallback {
    fn same_callback(left: &Self, right: &Self) -> bool {
        Rc::ptr_eq(left, right)
    }
}

impl CallbackIdentity for PainCallback {
    fn same_callback(left: &Self, right: &Self) -> bool {
        Rc::ptr_eq(left, right)
    }
}

impl CallbackIdentity for DieCallback {
    fn same_callback(left: &Self, right: &Self) -> bool {
        Rc::ptr_eq(left, right)
    }
}

/// Named native-callback family (`Q3CallbackFamily`).
#[derive(Debug, Clone)]
pub struct Q3CallbackFamily<F> {
    /// Callbacks by identity.
    by_id: HashMap<String, F>,
    /// Identities by callback.
    by_fn: Vec<(F, String)>,
}

impl<F: Clone + CallbackIdentity> Q3CallbackFamily<F> {
    /// Empty family.
    #[must_use]
    pub fn new() -> Self {
        Self {
            by_id: HashMap::new(),
            by_fn: Vec::new(),
        }
    }

    /// Register a callback identity (`register`).
    pub fn register(&mut self, id: &str, callback: F) -> F {
        if id.is_empty() {
            panic!("Q3 callback identity is empty");
        }
        if let Some(previous) = self.by_id.get(id) {
            if !F::same_callback(previous, &callback) {
                panic!("Duplicate Q3 callback identity {id}");
            }
        }
        for (known, identity) in &self.by_fn {
            if F::same_callback(known, &callback) && identity != id {
                panic!("Q3 callback has two identities: {identity}, {id}");
            }
        }
        self.by_id.insert(id.to_string(), callback.clone());
        if !self.by_fn.iter().any(|(known, _)| F::same_callback(known, &callback)) {
            self.by_fn.push((callback.clone(), id.to_string()));
        }
        callback
    }

    /// Register unless the identity already exists (`intern`).
    pub fn intern(&mut self, id: &str, callback: F) -> F {
        if let Some(existing) = self.by_id.get(id) {
            return existing.clone();
        }
        self.register(id, callback)
    }

    /// Identity of a callback (`capture`).
    #[must_use]
    pub fn capture(&self, callback: Option<&F>) -> Option<String> {
        let callback = callback?;
        for (known, identity) in &self.by_fn {
            if F::same_callback(known, callback) {
                return Some(identity.clone());
            }
        }
        panic!("Unregistered native Q3 callback cannot be saved");
    }

    /// Callback for an identity (`resolve`).
    #[must_use]
    pub fn resolve(&self, id: Option<&str>) -> Option<F> {
        let id = id?;
        match self.by_id.get(id) {
            Some(callback) => Some(callback.clone()),
            None => panic!("Unknown native Q3 callback {id}"),
        }
    }
}

impl<F: Clone + CallbackIdentity> Default for Q3CallbackFamily<F> {
    fn default() -> Self {
        Self::new()
    }
}

/// Native-callback catalog (`Q3CallbackCatalog`).
#[derive(Clone, Default)]
pub struct Q3CallbackCatalog {
    /// Think callbacks.
    pub think: Q3CallbackFamily<ThinkCallback>,
    /// Reached callbacks.
    pub reached: Q3CallbackFamily<ReachedCallback>,
    /// Blocked callbacks.
    pub blocked: Q3CallbackFamily<BlockedCallback>,
    /// Touch callbacks.
    pub touch: Q3CallbackFamily<TouchCallback>,
    /// Use callbacks.
    pub use_callbacks: Q3CallbackFamily<UseCallback>,
    /// Pain callbacks.
    pub pain: Q3CallbackFamily<PainCallback>,
    /// Die callbacks.
    pub die: Q3CallbackFamily<DieCallback>,
}

impl Q3CallbackCatalog {
    /// Empty catalog.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

/// Source-zero game entity (`GameEntity`).
pub struct GameEntity {
    /// Table slot.
    pub slot: usize,
    /// Owned actor handle.
    pub actor: OwnedActor,
    /// In-use flag.
    pub inuse: bool,
    /// Entity state.
    pub s: EntityState,
    /// Shared collision record.
    pub r: EntityShared,
    /// Client record for player entities.
    pub client: Option<GameClient>,
    /// Class name.
    pub classname: Option<String>,
    /// Spawn flags.
    pub spawnflags: i32,
    /// Never freed.
    pub never_free: bool,
    /// Game flags bitmask.
    pub flags: i32,
    /// Free time.
    pub freetime: i32,
    /// Event time.
    pub event_time: i32,
    /// Free after the event expires.
    pub free_after_event: bool,
    /// Unlink after the event expires.
    pub unlink_after_event: bool,
    /// Physics bounce factor.
    pub physics_bounce: f32,
    /// Collision mask.
    pub clipmask: i32,
    /// Mover state.
    pub mover_state: MoverState,
    /// Next think time.
    pub nextthink: i32,
    /// Think callback.
    pub think: Option<ThinkCallback>,
    /// Reached callback.
    pub reached: Option<ReachedCallback>,
    /// Blocked callback.
    pub blocked: Option<BlockedCallback>,
    /// Touch callback.
    pub touch: Option<TouchCallback>,
    /// Use callback.
    pub use_callback: Option<UseCallback>,
    /// Pain callback.
    pub pain: Option<PainCallback>,
    /// Die callback.
    pub die: Option<DieCallback>,
    /// Health.
    pub health: i32,
    /// Can take damage.
    pub takedamage: bool,
    /// Item count override.
    pub count: i32,
    /// Activating entity.
    pub activator: Option<EntityRef>,
    /// Team chain link.
    pub teamchain: Option<EntityRef>,
    /// Team master.
    pub teammaster: Option<EntityRef>,
    /// Team name.
    pub team: Option<String>,
    /// Target name.
    pub targetname: Option<String>,
    /// Target.
    pub target: Option<String>,
    /// Wait (respawn override seconds).
    pub wait: f32,
    /// Random respawn spread.
    pub random: f32,
    /// Contained item.
    pub item: Option<ItemDefinition>,
    /// Speed (or powerup global-sound suppression).
    pub speed: f32,
}

impl GameEntity {
    /// Fresh entity for a slot and actor (`new GameEntity(slot, binding)`).
    #[must_use]
    pub fn new(slot: usize, actor: OwnedActor) -> Self {
        assert!(slot < MAX_GENTITIES, "Game entity slot outside 0..1023");
        Self {
            slot,
            actor,
            inuse: false,
            s: EntityState::default(),
            r: EntityShared::default(),
            client: None,
            classname: None,
            spawnflags: 0,
            never_free: false,
            flags: 0,
            freetime: 0,
            event_time: 0,
            free_after_event: false,
            unlink_after_event: false,
            physics_bounce: 0.0,
            clipmask: 0,
            mover_state: MoverState::Pos1,
            nextthink: 0,
            think: None,
            reached: None,
            blocked: None,
            touch: None,
            use_callback: None,
            pain: None,
            die: None,
            health: 0,
            takedamage: false,
            count: 0,
            activator: None,
            teamchain: None,
            teammaster: None,
            team: None,
            targetname: None,
            target: None,
            wait: 0.0,
            random: 0.0,
            item: None,
            speed: 0.0,
        }
    }

    /// Clear all fields for release, keeping slot and actor identity.
    pub fn reset(&mut self) {
        let fresh = Self::new(self.slot, self.actor.clone());
        *self = fresh;
    }
}

// ---------------------------------------------------------------------------
// Rankings, utility scratch, spawn variables, death animation, arsenal
// (mirrors of `game/rankings.ts`, `game/utilities.ts`, `game/spawn.ts`,
// `foundation/character.ts`, `foundation/arsenal.ts`).
// ---------------------------------------------------------------------------

/// Recorded ranking report call.
#[derive(Debug, Clone, PartialEq)]
pub enum RankingCall {
    /// Damage report.
    Damage {
        /// Victim slot.
        victim: i32,
        /// Attacker slot.
        attacker: i32,
        /// Damage plus armor.
        damage: i32,
        /// Ranked means of death.
        means_of_death: i32,
        /// Frame time.
        frame: i32,
        /// Attacker is a client.
        attacker_is_client: bool,
        /// Same team.
        same_team: bool,
    },
    /// Death report.
    PlayerDie {
        /// Victim slot.
        victim: i32,
        /// Killer slot.
        killer: i32,
        /// Ranked means of death.
        means_of_death: i32,
    },
    /// Award report.
    Reward {
        /// Awardee slot.
        client: i32,
        /// Award bit.
        award: i32,
    },
    /// Weapon pickup report.
    PickupWeapon {
        /// Client slot.
        client: i32,
        /// Weapon tag.
        weapon: i32,
    },
    /// Ammo pickup report.
    PickupAmmo {
        /// Client slot.
        client: i32,
        /// Weapon tag.
        weapon: i32,
        /// Quantity.
        quantity: i32,
    },
    /// Health pickup report.
    PickupHealth {
        /// Client slot.
        client: i32,
        /// Quantity.
        quantity: i32,
    },
    /// Armor pickup report.
    PickupArmor {
        /// Client slot.
        client: i32,
        /// Quantity.
        quantity: i32,
    },
    /// Powerup pickup report.
    PickupPowerup {
        /// Client slot.
        client: i32,
        /// Powerup tag.
        powerup: i32,
    },
    /// Holdable pickup report.
    PickupHoldable {
        /// Client slot.
        client: i32,
        /// Holdable tag.
        holdable: i32,
    },
}

/// Ranking reports sink (`Q3RankingReports`).
#[derive(Debug, Clone, Default)]
pub struct Q3RankingReports {
    /// Recorded report calls.
    pub calls: Vec<RankingCall>,
}

impl Q3RankingReports {
    /// Empty sink.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Damage report (`damage`).
    #[allow(clippy::too_many_arguments)]
    pub fn damage(
        &mut self,
        victim: i32,
        attacker: i32,
        damage: i32,
        means_of_death: i32,
        frame: i32,
        attacker_is_client: bool,
        same_team: bool,
    ) {
        self.calls.push(RankingCall::Damage {
            victim,
            attacker,
            damage,
            means_of_death,
            frame,
            attacker_is_client,
            same_team,
        });
    }

    /// Death report (`playerDie`).
    pub fn player_die(&mut self, victim: i32, killer: i32, means_of_death: i32) {
        self.calls.push(RankingCall::PlayerDie {
            victim,
            killer,
            means_of_death,
        });
    }

    /// Award report (`reward`).
    pub fn reward(&mut self, client: i32, award: i32) {
        self.calls.push(RankingCall::Reward { client, award });
    }

    /// Weapon pickup report (`pickupWeapon`).
    pub fn pickup_weapon(&mut self, client: i32, weapon: i32) {
        self.calls.push(RankingCall::PickupWeapon { client, weapon });
    }

    /// Ammo pickup report (`pickupAmmo`).
    pub fn pickup_ammo(&mut self, client: i32, weapon: i32, quantity: i32) {
        self.calls.push(RankingCall::PickupAmmo {
            client,
            weapon,
            quantity,
        });
    }

    /// Health pickup report (`pickupHealth`).
    pub fn pickup_health(&mut self, client: i32, quantity: i32) {
        self.calls.push(RankingCall::PickupHealth { client, quantity });
    }

    /// Armor pickup report (`pickupArmor`).
    pub fn pickup_armor(&mut self, client: i32, quantity: i32) {
        self.calls.push(RankingCall::PickupArmor { client, quantity });
    }

    /// Powerup pickup report (`pickupPowerup`).
    pub fn pickup_powerup(&mut self, client: i32, powerup: i32) {
        self.calls.push(RankingCall::PickupPowerup { client, powerup });
    }

    /// Holdable pickup report (`pickupHoldable`).
    pub fn pickup_holdable(&mut self, client: i32, holdable: i32) {
        self.calls.push(RankingCall::PickupHoldable { client, holdable });
    }
}

/// Temporary vector ring slot.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TemporaryVector {
    /// X component.
    pub x: f32,
    /// Y component.
    pub y: f32,
    /// Z component.
    pub z: f32,
}

/// Fixed 32-byte game string allocation (`GameMemoryAllocation`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameMemoryAllocation {
    /// Backing bytes.
    data: [u8; 32],
}

impl GameMemoryAllocation {
    /// Zeroed allocation.
    #[must_use]
    pub fn new() -> Self {
        Self { data: [0; 32] }
    }

    /// Write a string with NUL termination (`writeString`).
    pub fn write_string(&mut self, value: &str) {
        self.data = [0; 32];
        let bytes = value.as_bytes();
        let count = bytes.len().min(31);
        self.data[..count].copy_from_slice(&bytes[..count]);
    }

    /// Read up to the NUL terminator (`readString`).
    #[must_use]
    pub fn read_string(&self) -> String {
        self.data
            .iter()
            .take_while(|byte| **byte != 0)
            .map(|byte| char::from(*byte))
            .collect()
    }
}

impl Default for GameMemoryAllocation {
    fn default() -> Self {
        Self::new()
    }
}

/// Per-game `tv`/`vtos` scratch rings (`GameUtilityScratch`).
#[derive(Clone)]
pub struct GameUtilityScratch {
    /// Temporary vector ring.
    vectors: [TemporaryVector; 8],
    /// Vector string ring.
    strings: [GameMemoryAllocation; 8],
    /// Vector ring index.
    vector_index: usize,
    /// String ring index.
    string_index: usize,
    /// Overflow printer.
    print: Rc<dyn Fn(String)>,
}

impl GameUtilityScratch {
    /// Scratch rings with a printer.
    #[must_use]
    pub fn new(print: Rc<dyn Fn(String)>) -> Self {
        Self {
            vectors: [TemporaryVector::default(); 8],
            strings: [GameMemoryAllocation::new(); 8],
            vector_index: 0,
            string_index: 0,
            print,
        }
    }

    /// Next temporary vector (`tv`).
    pub fn tv(&mut self, x: f32, y: f32, z: f32) -> TemporaryVector {
        let vector = TemporaryVector { x, y, z };
        self.vectors[self.vector_index] = vector;
        self.vector_index = (self.vector_index + 1) & 7;
        vector
    }

    /// Next vector string (`vtos`).
    pub fn vtos(&mut self, vector: Vec3) -> GameMemoryAllocation {
        let value = game_format(
            "(%i %i %i)",
            &[
                GameFormatArgument::Int(qvm_float_to_int(vector.x)),
                GameFormatArgument::Int(qvm_float_to_int(vector.y)),
                GameFormatArgument::Int(qvm_float_to_int(vector.z)),
            ],
        );
        if value.chars().count() >= 32 {
            (self.print)(game_format(
                "Com_sprintf: overflow of %i in %i\n",
                &[
                    GameFormatArgument::Int(value.chars().count() as i32),
                    GameFormatArgument::Int(32),
                ],
            ));
        }
        let slot = &mut self.strings[self.string_index];
        self.string_index = (self.string_index + 1) & 7;
        let clipped: String = value.chars().take(31).collect();
        slot.write_string(&clipped);
        *slot
    }
}

/// Spawn variable value (`SpawnValue`).
#[derive(Debug, Clone, PartialEq)]
pub struct SpawnValue<T> {
    /// Key was present.
    pub present: bool,
    /// Parsed value.
    pub value: T,
}

/// Entity spawn variables (`SpawnVariables`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SpawnVariables {
    /// Key/value pairs.
    pub entries: Vec<(String, String)>,
}

impl SpawnVariables {
    /// Build from pairs.
    #[must_use]
    pub fn new(entries: Vec<(String, String)>) -> Self {
        Self { entries }
    }

    /// String value for a key (`string`).
    #[must_use]
    pub fn string(&self, key: &str, default_value: &str) -> SpawnValue<String> {
        let normalized = key.to_ascii_lowercase();
        match self
            .entries
            .iter()
            .find(|(name, _)| name.to_ascii_lowercase() == normalized)
        {
            Some((_, value)) => SpawnValue {
                present: true,
                value: value.clone(),
            },
            None => SpawnValue {
                present: false,
                value: default_value.to_string(),
            },
        }
    }

    /// Float value for a key (`float`).
    #[must_use]
    pub fn float(&self, key: &str, default_value: &str) -> SpawnValue<f32> {
        let found = self.string(key, default_value);
        SpawnValue {
            present: found.present,
            value: game_atof(&found.value),
        }
    }
}

/// `bg_lib` atof over byte characters (`gameAtof`).
#[must_use]
pub fn game_atof(text: &str) -> f32 {
    for character in text.chars() {
        if (character as u32) > 255 {
            panic!("Game numbers require byte characters");
        }
    }
    let bytes: Vec<u8> = text.chars().map(|character| character as u8).collect();
    let mut offset: usize = 0;
    let byte = |offset: usize| -> i16 {
        if offset > bytes.len() {
            panic!("Game number scan reads beyond its backing string");
        }
        if offset == bytes.len() {
            return 0;
        }
        let raw = bytes[offset];
        if raw < 128 {
            i16::from(raw)
        } else {
            i16::from(raw) - 256
        }
    };
    while byte(offset) <= 32 && byte(offset) != 0 {
        offset += 1;
    }
    if byte(offset) == 0 {
        return 0.0;
    }
    let mut sign = 1.0f32;
    if byte(offset) == 43 || byte(offset) == 45 {
        if byte(offset) == 45 {
            sign = -1.0;
        }
        offset += 1;
    }
    let mut value = 0.0f32;
    let mut character = byte(offset);
    if byte(offset) != 46 {
        loop {
            let taken = byte(offset);
            offset += 1;
            character = taken;
            if !(48..=57).contains(&character) {
                break;
            }
            value = value * 10.0 + f32::from(character - 48);
        }
    } else {
        offset += 1;
    }
    if character == 46 {
        let mut fraction = 0.1f32;
        loop {
            let taken = byte(offset);
            offset += 1;
            character = taken;
            if !(48..=57).contains(&character) {
                break;
            }
            value += f32::from(character - 48) * fraction;
            fraction *= 0.1;
        }
    }
    value * sign
}

/// Death-animation checkpoint (`Q3DeathAnimationCheckpoint`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q3DeathAnimationCheckpoint {
    /// Checkpoint version.
    pub version: i32,
    /// Sequence index.
    pub index: i32,
}

/// Cycling death-animation selector (`Q3DeathAnimationSequence`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3DeathAnimationSequence {
    /// Current index.
    index: i32,
}

impl Q3DeathAnimationSequence {
    /// Fresh sequence.
    #[must_use]
    pub fn new() -> Self {
        Self { index: 0 }
    }

    /// Capture the sequence position (`capture`).
    #[must_use]
    pub fn capture(&self) -> Q3DeathAnimationCheckpoint {
        Q3DeathAnimationCheckpoint {
            version: 1,
            index: self.index,
        }
    }

    /// Restore the sequence position (`restore`).
    pub fn restore(&mut self, checkpoint: Q3DeathAnimationCheckpoint) {
        if checkpoint.version != 1 || checkpoint.index < 0 || checkpoint.index > 2 {
            panic!("Invalid Q3 death animation checkpoint");
        }
        self.index = checkpoint.index;
    }

    /// Next death animation and event (`next`).
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> (PlayerAnimation, EntityEvent) {
        let result = if self.index == 0 {
            (PlayerAnimation::BothDeath1, EntityEvent::Death1)
        } else if self.index == 1 {
            (PlayerAnimation::BothDeath2, EntityEvent::Death2)
        } else {
            (PlayerAnimation::BothDeath3, EntityEvent::Death3)
        };
        self.index = (self.index + 1) % 3;
        result
    }
}

impl Default for Q3DeathAnimationSequence {
    fn default() -> Self {
        Self::new()
    }
}

/// Weapon inventory identity (`Q3WeaponItem`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q3WeaponItem {
    /// Weapon id.
    pub weapon: i32,
    /// Weapon item id.
    pub item: &'static str,
    /// Ammo item id, when consumable.
    pub ammo: Option<&'static str>,
}

/// Weapon inventory table (`Q3_WEAPON_ITEMS`).
pub const Q3_WEAPON_ITEMS: [Q3WeaponItem; 13] = [
    Q3WeaponItem {
        weapon: Weapon::Gauntlet as i32,
        item: "q3:weapon/gauntlet",
        ammo: None,
    },
    Q3WeaponItem {
        weapon: Weapon::Machinegun as i32,
        item: "q3:weapon/machinegun",
        ammo: Some("q3:ammo/machinegun"),
    },
    Q3WeaponItem {
        weapon: Weapon::Shotgun as i32,
        item: "q3:weapon/shotgun",
        ammo: Some("q3:ammo/shotgun"),
    },
    Q3WeaponItem {
        weapon: Weapon::GrenadeLauncher as i32,
        item: "q3:weapon/grenadelauncher",
        ammo: Some("q3:ammo/grenadelauncher"),
    },
    Q3WeaponItem {
        weapon: Weapon::RocketLauncher as i32,
        item: "q3:weapon/rocketlauncher",
        ammo: Some("q3:ammo/rocketlauncher"),
    },
    Q3WeaponItem {
        weapon: Weapon::Lightning as i32,
        item: "q3:weapon/lightning",
        ammo: Some("q3:ammo/lightning"),
    },
    Q3WeaponItem {
        weapon: Weapon::Railgun as i32,
        item: "q3:weapon/railgun",
        ammo: Some("q3:ammo/railgun"),
    },
    Q3WeaponItem {
        weapon: Weapon::Plasmagun as i32,
        item: "q3:weapon/plasmagun",
        ammo: Some("q3:ammo/plasmagun"),
    },
    Q3WeaponItem {
        weapon: Weapon::Bfg as i32,
        item: "q3:weapon/bfg",
        ammo: Some("q3:ammo/bfg"),
    },
    Q3WeaponItem {
        weapon: Weapon::GrapplingHook as i32,
        item: "q3:weapon/grapple",
        ammo: None,
    },
    Q3WeaponItem {
        weapon: Weapon::Nailgun as i32,
        item: "q3:weapon/nailgun",
        ammo: Some("q3:ammo/nailgun"),
    },
    Q3WeaponItem {
        weapon: Weapon::ProxLauncher as i32,
        item: "q3:weapon/proxlauncher",
        ammo: Some("q3:ammo/proxlauncher"),
    },
    Q3WeaponItem {
        weapon: Weapon::Chaingun as i32,
        item: "q3:weapon/chaingun",
        ammo: Some("q3:ammo/chaingun"),
    },
];

/// Weapon inventory identity by weapon id (`q3WeaponItem`).
#[must_use]
pub fn q3_weapon_item(weapon: i32) -> Option<Q3WeaponItem> {
    Q3_WEAPON_ITEMS.iter().find(|entry| entry.weapon == weapon).copied()
}

/// Snap a vector toward integer coordinates (`snapVector`, missile.ts).
#[must_use]
pub fn snap_vector(value: Vec3) -> Vec3 {
    vec3(
        qvm_float_to_int(value.x) as f32,
        qvm_float_to_int(value.y) as f32,
        qvm_float_to_int(value.z) as f32,
    )
}

/// Snap a vector toward integer coordinates biased at `toward`
/// (`snapVectorTowards`, missile.ts).
#[must_use]
pub fn snap_vector_towards(value: Vec3, toward: Vec3) -> Vec3 {
    let axis = |v: f32, to: f32| -> f32 { qvm_float_to_int(v).wrapping_add(i32::from(to > v)) as f32 };
    vec3(
        axis(value.x, toward.x),
        axis(value.y, toward.y),
        axis(value.z, toward.z),
    )
}

// ---------------------------------------------------------------------------
// Traces, spatial queries, server world (mirrors of `base/world.ts`).
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

/// Trace solidity (`solidity`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceSolidity {
    /// Clear path.
    Clear,
    /// Started solid.
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
    Plane(Plane),
}

/// Trace hit (`hit`).
#[derive(Debug, Clone, PartialEq)]
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
    /// Actor to pass through.
    pub pass_actor: Option<ActorId>,
    /// Contents mask.
    pub mask: i32,
}

/// Actor trace result (`ActorTraceResult`).
#[derive(Debug, Clone, PartialEq)]
pub struct ActorTraceResult {
    /// Completed fraction.
    pub fraction: f32,
    /// End position.
    pub end: Vec3,
    /// Solidity.
    pub solidity: TraceSolidity,
    /// Contact.
    pub contact: TraceContact,
    /// Contents hit.
    pub contents: i32,
    /// Surface flags hit.
    pub surface_flags: i32,
    /// Hit record.
    pub hit: TraceHit,
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

/// Server trace result (`ServerTraceResult`).
#[derive(Debug, Clone, PartialEq)]
pub struct ServerTraceResult {
    /// Completed fraction.
    pub fraction: f32,
    /// End position.
    pub end: Vec3,
    /// Entity number hit.
    pub entity_num: i32,
    /// Solidity.
    pub solidity: TraceSolidity,
    /// Contact.
    pub contact: TraceContact,
    /// Contents hit.
    pub contents: i32,
    /// Surface flags hit.
    pub surface_flags: i32,
}

/// Actor spatial queries (`ActorSpatialQueries`).
#[derive(Clone)]
#[allow(clippy::type_complexity)]
pub struct SpatialQueries {
    /// Actors overlapping bounds (`areaActors`).
    pub area_actors: Rc<dyn Fn(&Bounds, i32) -> Vec<ActorId>>,
    /// Trace against actors (`traceActor`).
    pub trace_actor: Rc<dyn Fn(&ActorTraceQuery) -> ActorTraceResult>,
}

/// Server world operations (`ServerWorld`).
#[derive(Clone)]
pub struct ServerWorldMirror {
    /// Entity-number trace (`trace`).
    pub trace: Rc<dyn Fn(&ServerTraceQuery) -> ServerTraceResult>,
    /// Actor trace (`traceActor`).
    pub trace_actor: Rc<dyn Fn(&ActorTraceQuery) -> ActorTraceResult>,
    /// Point contents (`pointContents`).
    pub point_contents: Rc<dyn Fn(Vec3, i32) -> i32>,
    /// Link an entity (`link`).
    pub link: Rc<dyn Fn(EntityRef)>,
    /// Unlink an entity number (`unlink`).
    pub unlink: Rc<dyn Fn(i32)>,
}

// ---------------------------------------------------------------------------
// Combat records (mirrors of `contracts/gameplay.ts` and
// `world/gameplay/damage-modifier.ts`).
// ---------------------------------------------------------------------------

/// Quake II native cause (`Q2NativeCause`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Q2NativeCause {
    /// Classic DLL cause.
    Classic {
        /// Game.
        game: String,
        /// Native value.
        value: i32,
    },
    /// Rerelease cause.
    Rerelease {
        /// Cause id.
        id: i32,
        /// Friendly fire.
        friendly_fire: bool,
        /// No point loss.
        no_point_loss: bool,
    },
}

/// Quake I armor effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArmorEffect {
    /// Bypass armor.
    Bypass,
    /// Half effectiveness.
    HalfEffectiveness,
}

/// Environment hazard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvHazard {
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

/// Attack cause (`AttackProvenance.cause`).
#[derive(Debug, Clone, PartialEq)]
pub enum AttackCause {
    /// Quake I cause.
    Q1 {
        /// Death type.
        death_type: String,
        /// Armor effect override.
        armor_effect: Option<ArmorEffect>,
    },
    /// Quake II cause.
    Q2 {
        /// Means of death.
        means_of_death: i32,
        /// Damage flags.
        damage_flags: i32,
        /// Native cause.
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
        hazard: EnvHazard,
    },
}

/// Source clock time (`SourceTime`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceTime {
    /// Millisecond clock.
    Milliseconds {
        /// Millisecond value.
        value: i32,
    },
}

/// Captured attack provenance (`AttackProvenance`).
#[derive(Debug, Clone, PartialEq)]
pub struct AttackProvenance {
    /// Sequence number.
    pub sequence: i32,
    /// Source time.
    pub time: SourceTime,
    /// Attacker actor.
    pub attacker: Option<ActorId>,
    /// Inflictor actor.
    pub inflictor: Option<ActorId>,
    /// Originating projectile.
    pub originating_projectile: Option<ActorId>,
    /// Weapon item.
    pub weapon: Option<ItemId>,
    /// Weapon provider.
    pub weapon_provider: ProviderId,
    /// Provider that already applied its damage modifier.
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

/// Attacker damage policy (`SourceDamageModifier`).
#[derive(Clone)]
pub struct SourceDamageModifier {
    /// Owning provider.
    pub owner: ProviderId,
    /// Amount transform over the live attacker.
    pub transform: Rc<dyn Fn(Option<ActorId>, f32) -> f32>,
}

/// Damage delivery (`DamageRequest.delivery`).
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
    /// Target actor.
    pub target: ActorId,
    /// Damage amount.
    pub amount: f32,
    /// Knockback amount.
    pub knockback: f32,
    /// Impulse direction.
    pub direction: Vec3,
    /// Impact point.
    pub point: Vec3,
    /// Impact normal.
    pub normal: Vec3,
    /// Delivery.
    pub delivery: DamageDelivery,
}

/// Regular armor state (`RegularArmorState`).
#[derive(Debug, Clone, PartialEq)]
pub enum RegularArmor {
    /// No armor.
    None,
    /// Quake I armor.
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
        /// Protection fraction.
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PoweredProtection {
    /// No powered protection.
    None,
    /// Screen with cells.
    Screen {
        /// Cells.
        cells: i32,
    },
    /// Shield with cells.
    Shield {
        /// Cells.
        cells: i32,
    },
}

/// Armor state (`ArmorState`).
#[derive(Debug, Clone, PartialEq)]
pub struct ArmorState {
    /// Regular armor.
    pub regular: RegularArmor,
    /// Powered protection.
    pub powered: PoweredProtection,
}

/// Combat state (`CombatState`).
#[derive(Debug, Clone, PartialEq)]
pub struct CombatState {
    /// Health.
    pub health: i32,
    /// Armor.
    pub armor: ArmorState,
    /// Mass.
    pub mass: f32,
    /// Can take damage.
    pub can_take_damage: bool,
    /// Invulnerable.
    pub invulnerable: bool,
    /// Immune to knockback.
    pub no_knockback: bool,
    /// Team identity.
    pub team: Option<String>,
}

/// Committed damage mutation (`DamageMutation`).
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

/// Damage reaction (`DamageDecision.reaction`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DamageReaction {
    /// No reaction.
    None,
    /// Pain.
    Pain,
    /// Death.
    Death,
}

/// Source damage feedback (`DamageDecision.feedback`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DamageFeedback {
    /// Quake II feedback.
    Q2 {
        /// Power armor saved.
        power_armor: i32,
        /// Armor saved.
        armor: i32,
        /// Blood damage.
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

/// Committed damage decision (`DamageDecision`).
#[derive(Debug, Clone, PartialEq)]
pub struct DamageDecision {
    /// Request.
    pub request: DamageRequest,
    /// Committed mutations.
    pub mutations: Vec<DamageMutation>,
    /// Applied health damage.
    pub applied_damage: i32,
    /// Reaction.
    pub reaction: DamageReaction,
    /// Source feedback.
    pub feedback: Option<DamageFeedback>,
}

/// Damage outcome (`DamageOutcome`).
#[derive(Debug, Clone, PartialEq)]
pub enum DamageOutcome {
    /// Target went stale.
    StaleTarget {
        /// Request.
        request: DamageRequest,
    },
    /// Committed decision.
    Committed {
        /// Decision.
        decision: DamageDecision,
        /// Target survived.
        survived: bool,
    },
}

/// Gameplay authority operations (`GameplayAuthority`).
#[derive(Clone)]
#[allow(clippy::type_complexity)]
pub struct CombatAuthority {
    /// Apply a damage request (`apply`).
    pub apply: Rc<dyn Fn(DamageRequest) -> DamageOutcome>,
    /// Read combat state (`read`).
    pub read: Rc<dyn Fn(&ActorId) -> Option<CombatState>>,
}

/// Apply the source damage modifier (`applySourceDamageModifier`).
#[must_use]
pub fn apply_source_damage_modifier(
    request: &DamageRequest,
    modifier: Option<&SourceDamageModifier>,
    is_live: &dyn Fn(&ActorId) -> bool,
) -> DamageRequest {
    let Some(modifier) = modifier else {
        return request.clone();
    };
    let current =
        |actor: &Option<ActorId>| -> Option<ActorId> { actor.as_ref().filter(|handle| is_live(handle)).cloned() };
    let mut attack = request.attack.clone();
    attack.attacker = current(&request.attack.attacker);
    attack.inflictor = current(&request.attack.inflictor);
    let amount = if attack.damage_powerup_owner.as_ref() == Some(&modifier.owner) {
        request.amount
    } else {
        (modifier.transform)(attack.attacker.clone(), request.amount)
    };
    attack.damage_powerup_owner = Some(modifier.owner.clone());
    DamageRequest {
        attack,
        amount,
        ..request.clone()
    }
}

// ---------------------------------------------------------------------------
// Radius damage (mirror of `game/radius-damage.ts`).
// ---------------------------------------------------------------------------

/// Radius-damage target record (`Q3RadiusTarget`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3RadiusTarget {
    /// Target origin.
    pub origin: Vec3,
    /// Target bounds.
    pub bounds: Bounds,
    /// Eligible for accuracy credit.
    pub accuracy_eligible: bool,
}

/// Radius-damage host (`Q3RadiusHost`).
#[derive(Clone)]
#[allow(clippy::type_complexity)]
pub struct Q3RadiusHost {
    /// Spatial queries.
    pub spatial: SpatialQueries,
    /// Target record by actor.
    pub target: Rc<dyn Fn(&ActorId) -> Option<Q3RadiusTarget>>,
    /// Damage application.
    pub damage: Rc<dyn Fn(&ActorId, Vec3, Vec3, i32)>,
}

/// Visibility check for radius damage (`q3CanDamage`).
#[must_use]
pub fn q3_can_damage(spatial: &SpatialQueries, actor: &ActorId, bounds: &Bounds, origin: Vec3) -> bool {
    let midpoint = scale3(add3(bounds.min, bounds.max), 0.5);
    let trace = |end: Vec3| -> ActorTraceResult {
        (spatial.trace_actor)(&ActorTraceQuery {
            start: origin,
            end,
            shape: TraceShape::Point,
            pass_actor: None,
            mask: 1,
        })
    };
    let center = trace(midpoint);
    if center.fraction == 1.0 {
        return true;
    }
    if let TraceHit::Actor(hit) = &center.hit {
        if hit == actor {
            return true;
        }
    }
    for (x, y) in [(15.0, 15.0), (15.0, -15.0), (-15.0, 15.0), (-15.0, -15.0)] {
        let end = vec3(midpoint.x + x, midpoint.y + y, midpoint.z);
        if trace(end).fraction == 1.0 {
            return true;
        }
    }
    false
}

/// Radius falloff damage over shared actors (`q3RadiusDamage`).
pub fn q3_radius_damage(host: &Q3RadiusHost, origin: Vec3, amount: f32, radius: f32, ignore: Option<&ActorId>) -> bool {
    let radius = radius.max(1.0);
    let extent = vec3(radius, radius, radius);
    let candidates = (host.spatial.area_actors)(
        &Bounds {
            min: sub3(origin, extent),
            max: add3(origin, extent),
        },
        1024,
    );
    let mut hit_client = false;
    for actor in &candidates {
        if ignore.is_some_and(|ignored| ignored == actor) {
            continue;
        }
        let Some(target) = (host.target)(actor) else {
            continue;
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
        let distance = length3(vec3(
            axis(origin.x, target.bounds.min.x, target.bounds.max.x),
            axis(origin.y, target.bounds.min.y, target.bounds.max.y),
            axis(origin.z, target.bounds.min.z, target.bounds.max.z),
        ));
        if distance >= radius {
            continue;
        }
        let points = amount * (1.0 - distance / radius);
        if !q3_can_damage(&host.spatial, actor, &target.bounds, origin) {
            continue;
        }
        if target.accuracy_eligible {
            hit_client = true;
        }
        (host.damage)(
            actor,
            add3(sub3(target.origin, origin), vec3(0.0, 0.0, 24.0)),
            origin,
            points.trunc() as i32,
        );
    }
    hit_client
}
