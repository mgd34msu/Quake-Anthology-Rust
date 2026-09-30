//! Quake III presentation scene (`q3_present_scene`) support: shared mirrors, group error, and tests.
//!
//! Self-containment mirrors: minimal local copies of items the donors import from
//! modules outside this port (sibling q3 donors, engine contracts, and math/text
//! helpers), plus the group error type. Sibling-owned mirrors carry SIBLING-MIRROR
//! notes and unify with the canonical ports at merge time.

use qa_core::math::{
    add3, cross3, length3, normalize3_or_zero, perpendicular_vector, scale3, sub3, vec3, Axis, Bounds, Plane, Vec3,
    Vec4,
};
use qa_core::numeric::q_crandom;
use std::fmt;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::presentation::marks::*;
use crate::q3::presentation::movement_host::*;
use crate::q3::presentation::ref_entity::*;
use crate::q3::presentation::refdef::*;

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

/// Failure of a presentation operation (`RangeError`, `CommonError("drop")`,
/// or a plain `Error` in the donors).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresentError {
    /// Machine-readable class.
    pub kind: PresentErrorKind,
    /// Human-readable message.
    pub message: String,
}

/// Error class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresentErrorKind {
    /// Out-of-range input (`RangeError`).
    Range,
    /// Dropped client (`CommonError("drop")`).
    Drop,
    /// Invalid state or bug (`Error`).
    State,
}

impl PresentError {
    /// Range error.
    #[must_use]
    pub fn range(message: impl Into<String>) -> Self {
        Self {
            kind: PresentErrorKind::Range,
            message: message.into(),
        }
    }

    /// Drop error.
    #[must_use]
    pub fn drop(message: impl Into<String>) -> Self {
        Self {
            kind: PresentErrorKind::Drop,
            message: message.into(),
        }
    }

    /// State error.
    #[must_use]
    pub fn state(message: impl Into<String>) -> Self {
        Self {
            kind: PresentErrorKind::State,
            message: message.into(),
        }
    }
}

impl fmt::Display for PresentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.message)
    }
}

impl std::error::Error for PresentError {}

/// Presentation result.
pub type PresentResult<T> = Result<T, PresentError>;

// ---------------------------------------------------------------------------
// Sibling mirrors: shared enumerations (base/shared/definitions.ts,
// movement/q3/constants.ts)
// ---------------------------------------------------------------------------

/// Game product (`Product`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Product {
    /// Base Quake III.
    #[default]
    BaseQ3,
    /// Team Arena.
    MissionPack,
}

/// Game type (`GameType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
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
    OneFlagCtf = 5,
    /// Overload.
    Obelisk = 6,
    /// Harvester.
    Harvester = 7,
    /// Sentinel.
    MaxGameType = 8,
}

impl GameType {
    /// Convert from a raw integer.
    #[must_use]
    pub fn from_i32(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::Ffa),
            1 => Some(Self::Tournament),
            2 => Some(Self::SinglePlayer),
            3 => Some(Self::Team),
            4 => Some(Self::Ctf),
            5 => Some(Self::OneFlagCtf),
            6 => Some(Self::Obelisk),
            7 => Some(Self::Harvester),
            8 => Some(Self::MaxGameType),
            _ => None,
        }
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
}

impl Team {
    /// Convert from a raw integer.
    #[must_use]
    pub fn from_i32(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::Free),
            1 => Some(Self::Red),
            2 => Some(Self::Blue),
            3 => Some(Self::Spectator),
            _ => None,
        }
    }
}

/// Item type (`ItemType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum ItemType {
    /// Bad.
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
    /// Team item.
    Team = 8,
}

impl ItemType {
    /// Convert from a raw integer.
    #[must_use]
    pub fn from_i32(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::Bad),
            1 => Some(Self::Weapon),
            2 => Some(Self::Ammo),
            3 => Some(Self::Armor),
            4 => Some(Self::Health),
            5 => Some(Self::Powerup),
            6 => Some(Self::Holdable),
            7 => Some(Self::PersistantPowerup),
            8 => Some(Self::Team),
            _ => None,
        }
    }
}

/// Entity type (`EntityType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
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
    /// Event floor; entity types at or above this are events, not entities.
    Events = 13,
}

impl EntityType {
    /// Convert from a raw integer.
    #[must_use]
    pub fn from_i32(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::General),
            1 => Some(Self::Player),
            2 => Some(Self::Item),
            3 => Some(Self::Missile),
            4 => Some(Self::Mover),
            5 => Some(Self::Beam),
            6 => Some(Self::Portal),
            7 => Some(Self::Speaker),
            8 => Some(Self::PushTrigger),
            9 => Some(Self::TeleportTrigger),
            10 => Some(Self::Invisible),
            11 => Some(Self::Grapple),
            12 => Some(Self::Team),
            13 => Some(Self::Events),
            _ => None,
        }
    }
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

impl Weapon {
    /// Convert from a raw integer.
    #[must_use]
    pub fn from_i32(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::None),
            1 => Some(Self::Gauntlet),
            2 => Some(Self::Machinegun),
            3 => Some(Self::Shotgun),
            4 => Some(Self::GrenadeLauncher),
            5 => Some(Self::RocketLauncher),
            6 => Some(Self::Lightning),
            7 => Some(Self::Railgun),
            8 => Some(Self::Plasmagun),
            9 => Some(Self::Bfg),
            10 => Some(Self::GrapplingHook),
            11 => Some(Self::Nailgun),
            12 => Some(Self::ProxLauncher),
            13 => Some(Self::Chaingun),
            _ => None,
        }
    }
}

/// Weapon count (`weaponCount`).
#[must_use]
pub const fn weapon_count(product: Product) -> i32 {
    match product {
        Product::BaseQ3 => 11,
        Product::MissionPack => 14,
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
    /// Regen.
    Regen = 5,
    /// Flight.
    Flight = 6,
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
}

/// Movement type (`MoveType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
#[derive(Default)]
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
    /// Freeze.
    Freeze = 4,
    /// Intermission.
    Intermission = 5,
    /// Single-player intermission.
    SinglePlayerIntermission = 6,
}

impl MoveType {
    /// Convert from a raw integer.
    #[must_use]
    pub fn from_i32(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::Normal),
            1 => Some(Self::Noclip),
            2 => Some(Self::Spectator),
            3 => Some(Self::Dead),
            4 => Some(Self::Freeze),
            5 => Some(Self::Intermission),
            6 => Some(Self::SinglePlayerIntermission),
            _ => None,
        }
    }
}

/// Weapon state (`WeaponState`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
#[derive(Default)]
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

impl WeaponState {
    /// Convert from a raw integer.
    #[must_use]
    pub fn from_i32(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::Ready),
            1 => Some(Self::Raising),
            2 => Some(Self::Dropping),
            3 => Some(Self::Firing),
            _ => None,
        }
    }
}

/// Movement flags (`MoveFlags`).
pub struct MoveFlags;

impl MoveFlags {
    /// Ducked.
    pub const DUCKED: i32 = 1;
    /// Following.
    pub const FOLLOW: i32 = 4096;
}

/// Player animation slots (`PlayerAnimation`).
pub struct PlayerAnimation;

impl PlayerAnimation {
    /// Torso gesture.
    pub const TORSO_GESTURE: usize = 6;
    /// Torso attack.
    pub const TORSO_ATTACK: usize = 7;
    /// Torso attack 2.
    pub const TORSO_ATTACK2: usize = 8;
    /// Torso drop.
    pub const TORSO_DROP: usize = 9;
    /// Legs crouch walk.
    pub const LEGS_WALKCR: i32 = 13;
    /// Legs crouch idle.
    pub const LEGS_IDLECR: i32 = 23;
}

/// Persistent index (`PersistentIndex`).
pub struct PersistentIndex;

impl PersistentIndex {
    /// Team.
    pub const PERS_TEAM: usize = 3;
}

/// Player-state stat layout (`statSchema`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatSchema {
    /// Health slot.
    pub health: usize,
    /// Holdable slot.
    pub holdable_item: usize,
    /// Weapon bits slot.
    pub weapons: usize,
    /// Armor slot.
    pub armor: usize,
    /// Dead yaw slot.
    pub dead_yaw: usize,
    /// Clients-ready slot.
    pub clients_ready: usize,
    /// Max health slot.
    pub max_health: usize,
}

/// Stat layout for a product (`statSchema`).
#[must_use]
pub const fn stat_schema(product: Product) -> StatSchema {
    match product {
        Product::BaseQ3 => StatSchema {
            health: 0,
            holdable_item: 1,
            weapons: 2,
            armor: 3,
            dead_yaw: 4,
            clients_ready: 5,
            max_health: 6,
        },
        Product::MissionPack => StatSchema {
            health: 0,
            holdable_item: 1,
            weapons: 3,
            armor: 4,
            dead_yaw: 5,
            clients_ready: 6,
            max_health: 7,
        },
    }
}

/// Default gravity (`DEFAULT_GRAVITY`).
pub const DEFAULT_GRAVITY: f32 = 800.0;

/// Maximum items (`MAX_ITEMS`).
pub const MAX_ITEMS: usize = 256;

/// World entity number (`ENTITYNUM_WORLD`).
pub const ENTITYNUM_WORLD: i32 = 1022;

/// No entity (`ENTITYNUM_NONE`).
pub const ENTITYNUM_NONE: i32 = 1023;

// ---------------------------------------------------------------------------
// Sibling mirrors: trajectory (base/shared/trajectory.ts)
// ---------------------------------------------------------------------------

/// Trajectory type (`TrajectoryType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum TrajectoryType {
    /// Stationary.
    Stationary = 0,
    /// Interpolated.
    Interpolate = 1,
    /// Linear.
    Linear = 2,
    /// Linear with stop.
    LinearStop = 3,
    /// Sine.
    Sine = 4,
    /// Gravity.
    Gravity = 5,
}

impl TrajectoryType {
    /// Convert from a raw integer.
    #[must_use]
    pub fn from_i32(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::Stationary),
            1 => Some(Self::Interpolate),
            2 => Some(Self::Linear),
            3 => Some(Self::LinearStop),
            4 => Some(Self::Sine),
            5 => Some(Self::Gravity),
            _ => None,
        }
    }
}

/// Trajectory (`Trajectory`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Trajectory {
    /// Type.
    pub type_: TrajectoryType,
    /// Start time.
    pub time: i32,
    /// Duration.
    pub duration: i32,
    /// Base.
    pub base: Vec3,
    /// Delta.
    pub delta: Vec3,
}

impl Default for Trajectory {
    fn default() -> Self {
        Self {
            type_: TrajectoryType::Stationary,
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

pub(crate) fn periodic_radians(trajectory: &Trajectory, at_time: i32) -> f32 {
    let fraction = ((at_time.wrapping_sub(trajectory.time)) as f32) / (trajectory.duration as f32);
    fraction * std::f32::consts::PI * 2.0
}

/// Evaluate a trajectory (`evaluateTrajectory`).
pub fn evaluate_trajectory(trajectory: &Trajectory, at_time: i32) -> Vec3 {
    match trajectory.type_ {
        TrajectoryType::Stationary | TrajectoryType::Interpolate => trajectory.base,
        TrajectoryType::Linear => {
            let delta_time = trajectory_seconds(at_time.wrapping_sub(trajectory.time));
            add3(trajectory.base, scale3(trajectory.delta, delta_time))
        }
        TrajectoryType::Sine => {
            let phase = periodic_radians(trajectory, at_time).sin();
            add3(trajectory.base, scale3(trajectory.delta, phase))
        }
        TrajectoryType::LinearStop => {
            let end = trajectory.time.wrapping_add(trajectory.duration);
            let time = if at_time > end { end } else { at_time };
            let delta_time = trajectory_seconds(time.wrapping_sub(trajectory.time)).max(0.0);
            add3(trajectory.base, scale3(trajectory.delta, delta_time))
        }
        TrajectoryType::Gravity => {
            let delta_time = trajectory_seconds(at_time.wrapping_sub(trajectory.time));
            let result = add3(trajectory.base, scale3(trajectory.delta, delta_time));
            let fall = 0.5 * DEFAULT_GRAVITY * delta_time * delta_time;
            vec3(result.x, result.y, result.z - fall)
        }
    }
}

/// Evaluate a trajectory delta (`evaluateTrajectoryDelta`).
pub fn evaluate_trajectory_delta(trajectory: &Trajectory, at_time: i32) -> Vec3 {
    match trajectory.type_ {
        TrajectoryType::Stationary | TrajectoryType::Interpolate => vec3(0.0, 0.0, 0.0),
        TrajectoryType::Linear => trajectory.delta,
        TrajectoryType::Sine => {
            let phase = periodic_radians(trajectory, at_time).cos() * 0.5;
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
            let delta_time = trajectory_seconds(at_time.wrapping_sub(trajectory.time));
            vec3(
                trajectory.delta.x,
                trajectory.delta.y,
                trajectory.delta.z - DEFAULT_GRAVITY * delta_time,
            )
        }
    }
}

// ---------------------------------------------------------------------------
// Sibling mirrors: collision trace (world/collision/q3/world.ts)
// ---------------------------------------------------------------------------

/// Trace shape (`TraceShape`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TraceShape {
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

/// Trace query (`TraceQuery`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TraceQuery {
    /// Start.
    pub start: Vec3,
    /// End.
    pub end: Vec3,
    /// Shape.
    pub shape: TraceShape,
    /// Contents mask.
    pub mask: i32,
    /// Model index override.
    pub model_index: Option<i32>,
    /// Test curves.
    pub curves: bool,
    /// Player curve clipping.
    pub player_curve_clip: bool,
}

/// Trace contact (`TraceContact`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TraceContact {
    /// No contact plane.
    None,
    /// Impact plane.
    Plane {
        /// Plane.
        plane: Plane,
    },
}

/// Trace solidity (`solidity`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceSolidity {
    /// Clear.
    Clear,
    /// Start solid.
    StartSolid,
    /// All solid.
    AllSolid,
}

/// Trace result (`TraceResult`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TraceResult {
    /// Fraction.
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
}

/// Opaque collision counters handle (`CollisionWorld.counters`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CollisionCounters;

/// Collision world surface used by presentation (`CollisionWorld`).
pub trait PresentCollisionWorld {
    /// Trace.
    fn trace(&self, query: &TraceQuery) -> TraceResult;
    /// Point contents.
    fn point_contents(&self, point: Vec3) -> i32;
    /// Transformed trace against a positioned model.
    fn transformed_trace(&self, query: &TraceQuery, origin: Vec3, angles: Vec3) -> TraceResult;
    /// Transformed point contents.
    fn transformed_point_contents(&self, point: Vec3, model_index: i32, origin: Vec3, angles: Vec3) -> i32;
    /// Counters.
    fn counters(&self) -> CollisionCounters;
}

// ---------------------------------------------------------------------------
// Sibling mirrors: player state (base/shared/player-state.ts,
// network/q3/state/entity.ts)
// ---------------------------------------------------------------------------

/// User command (`UserCommand`, minimal mirror: only forwarded here).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UserCommand {
    /// Server time.
    pub server_time: i32,
}

/// Indexed player-state slots (stats/persistant/ammo).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PlayerStateSlots {
    /// Values.
    pub values: Vec<i32>,
}

impl PlayerStateSlots {
    /// Read a slot, defaulting to zero.
    #[must_use]
    pub fn get(&self, index: usize) -> i32 {
        self.values.get(index).copied().unwrap_or(0)
    }

    /// Write a slot, growing as needed.
    pub fn set(&mut self, index: usize, value: i32) {
        if self.values.len() <= index {
            self.values.resize(index + 1, 0);
        }
        self.values[index] = value;
    }
}

/// Source player state (`SourcePlayerState`, fields touched by this port).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SourcePlayerState {
    /// Product.
    pub product: Product,
    /// Client number.
    pub client_num: i32,
    /// Origin.
    pub origin: Vec3,
    /// View angles.
    pub viewangles: Vec3,
    /// View height.
    pub viewheight: f32,
    /// Velocity.
    pub velocity: Vec3,
    /// Bob cycle.
    pub bob_cycle: i32,
    /// Movement type.
    pub pm_type: MoveType,
    /// Movement flags.
    pub pm_flags: i32,
    /// Health.
    pub health: i32,
    /// Stats.
    pub stats: PlayerStateSlots,
    /// Persistant.
    pub persistant: PlayerStateSlots,
    /// Ammo.
    pub ammo: PlayerStateSlots,
    /// Weapon.
    pub weapon: i32,
    /// Weapon state.
    pub weapon_state: WeaponState,
    /// Weapon time.
    pub weapon_time: i32,
    /// Entity flags.
    pub e_flags: i32,
    /// Ground entity.
    pub ground_entity_num: i32,
}

/// Entity state (`EntityState`).
#[derive(Debug, Clone, PartialEq)]
pub struct EntityState {
    /// Number.
    pub number: i32,
    /// Entity type.
    pub e_type: i32,
    /// Flags.
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
    /// Other entity.
    pub other_entity_num: i32,
    /// Other entity 2.
    pub other_entity_num2: i32,
    /// Ground entity.
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

/// Copy predicted player state into its entity (`playerStateToEntityState`,
/// minimal mirror: identity and pose fields read downstream).
pub fn player_state_to_entity_state(player: &SourcePlayerState, entity: &mut EntityState) {
    entity.number = player.client_num;
    entity.e_type = EntityType::Player as i32;
    entity.pos.base = player.origin;
    entity.apos.base = player.viewangles;
    entity.angles = player.viewangles;
    entity.client_num = player.client_num;
    entity.weapon = player.weapon;
    entity.e_flags = player.e_flags;
    entity.ground_entity_num = player.ground_entity_num;
}

// ---------------------------------------------------------------------------
// Sibling mirrors: lighting, audio, identity, render contracts
// ---------------------------------------------------------------------------

/// Dynamic light (`DynamicLight`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DynamicLight {
    /// Origin.
    pub origin: Vec3,
    /// Radius.
    pub radius: f32,
    /// Color.
    pub color: Vec3,
    /// Additive (`RE_AddAdditiveLightToScene`).
    pub additive: bool,
}

/// Lighting sample (`LightingSample`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LightingSample {
    /// Ambient light in source byte units.
    pub ambient_light: Vec3,
    /// Directed light in source byte units.
    pub directed_light: Vec3,
    /// Light direction.
    pub light_dir: Vec3,
}

/// Decoded sound handle (`PcmSound`, minimal mirror: handle by path).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PresentSound {
    /// Source path.
    pub path: String,
}

impl PresentSound {
    /// New handle.
    #[must_use]
    pub fn new(path: impl Into<String>) -> Self {
        Self { path: path.into() }
    }
}

/// Sound origin (`StartSoundOptions.origin`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SoundOrigin {
    /// Fixed position.
    Fixed {
        /// Position.
        position: Vec3,
    },
    /// Local (listener).
    Local,
}

/// Sound start options (`StartSoundOptions`, minimal mirror).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SoundOptions {
    /// Entity.
    pub entity: i32,
    /// Channel.
    pub channel: i32,
    /// Origin.
    pub origin: SoundOrigin,
    /// Volume.
    pub volume: i32,
}

/// Actor identity (`ActorId`).
pub type ActorId = u32;

/// Seat identity (`SeatId`).
pub type SeatId = u32;

/// Viewport rectangle (`Rect`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    /// X.
    pub x: i32,
    /// Y.
    pub y: i32,
    /// Width.
    pub width: i32,
    /// Height.
    pub height: i32,
}

/// Perspective projection (`perspectiveProjection`, minimal mirror).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PerspectiveProjection {
    /// Horizontal FOV.
    pub fov_x: f32,
    /// Vertical FOV.
    pub fov_y: f32,
    /// Far clip.
    pub far_clip: f32,
    /// Near clip.
    pub near_clip: f32,
}

/// Build a perspective projection (`perspectiveProjection`).
#[must_use]
pub fn perspective_projection(fov_x: f32, fov_y: f32, far_clip: f32, near_clip: f32) -> PerspectiveProjection {
    PerspectiveProjection {
        fov_x,
        fov_y,
        far_clip,
        near_clip,
    }
}

/// Scene camera (`SceneCamera`, minimal mirror).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SceneCamera {
    /// Viewport.
    pub viewport: Rect,
    /// Origin.
    pub origin: Vec3,
    /// Axis.
    pub axis: Axis,
    /// Projection.
    pub projection: PerspectiveProjection,
}

/// Railgun beam settings (`RailSettings`, opaque mirror).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RailSettings;

/// Fog volume (`FogVolume`, minimal mirror).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FogVolume {
    /// Bounds.
    pub bounds: Bounds,
}

/// Scene light (`SceneLight`, minimal mirror).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PresentSceneLight {
    /// Origin.
    pub origin: Vec3,
    /// Radius.
    pub radius: f32,
    /// Color.
    pub color: Vec3,
    /// Additive.
    pub additive: bool,
}

/// Model source options (`ModelSourceOptions`, minimal mirror).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ModelSourceOptions {
    /// Custom shader override.
    pub custom_shader: Option<String>,
    /// Custom skin surfaces.
    pub custom_skin: Option<Vec<SkinMapping>>,
    /// Non-normalized axes.
    pub non_normalized_axes: bool,
}

/// Skin surface mapping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkinMapping {
    /// Surface name.
    pub name: String,
    /// Shader name.
    pub shader: String,
}

/// Presented entity model.
#[derive(Debug, Clone, PartialEq)]
pub enum PresentEntityModel {
    /// Brush model reference.
    BrushModel {
        /// World.
        world: PresentWorld,
        /// Model index.
        model: usize,
    },
    /// Decoded model.
    Decoded(Q3DecodedModel),
}

/// Scene entity (`SceneEntity`, minimal mirror).
#[derive(Debug, Clone, PartialEq)]
pub struct PresentSceneEntity {
    /// Actor.
    pub actor: Option<ActorId>,
    /// Resource.
    pub resource: PresentResource,
    /// Model.
    pub model: PresentEntityModel,
    /// Origin.
    pub origin: Vec3,
    /// Axis.
    pub axis: Axis,
    /// Previous origin.
    pub previous_origin: Vec3,
    /// Frame.
    pub frame: i32,
    /// Previous frame.
    pub previous_frame: i32,
    /// Back lerp.
    pub back_lerp: f32,
    /// Skin.
    pub skin: i32,
    /// Color (unit).
    pub color: Vec4,
    /// Shader time seconds.
    pub shader_time: f32,
    /// Render flags.
    pub render_flags: i32,
    /// Lighting origin.
    pub lighting_origin: Vec3,
    /// Shadow plane.
    pub shadow_plane: f32,
}

/// World handle for inline models (`DecodedWorld`, minimal mirror).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PresentWorld {
    /// Name.
    pub name: String,
}

impl PresentWorld {
    /// New handle.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

/// Resolved resource reference (`ResolvedResourceReference`, minimal mirror).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PresentResource {
    /// Path.
    pub path: String,
}

impl PresentResource {
    /// New reference.
    #[must_use]
    pub fn new(path: impl Into<String>) -> Self {
        Self { path: path.into() }
    }
}

// ---------------------------------------------------------------------------
// Shared presentation services (prediction/random/marks/particles mirrors)
// ---------------------------------------------------------------------------

/// Cgame random stream (`GameRandom`, minimal mirror).
pub trait PresentRandom {
    /// 15-bit integer.
    fn rand(&mut self) -> i32;
    /// Unit fraction.
    fn random(&mut self) -> f32;
    /// Centered fraction.
    fn crandom(&mut self) -> f32;
}

/// Prediction trace service (`PredictionRuntime`, minimal mirror).
pub trait PresentPrediction {
    /// Trace with entity skipping.
    fn trace_mover(&self, start: Vec3, end: Vec3, bounds: Bounds, skip_number: i32, mask: i32) -> MovementTrace;
    /// Point contents with entity passing.
    fn point_contents_pred(&self, point: Vec3, pass_entity: i32) -> i32;
}

/// Raw collision service.
pub trait PresentCollision {
    /// Shape trace.
    fn collision_trace(&self, start: Vec3, end: Vec3, mask: i32) -> TraceResult;
    /// Raw point contents.
    fn collision_contents(&self, point: Vec3) -> i32;
}

/// Mark projection service.
pub trait PresentMarks {
    /// Project an impact mark.
    fn impact_mark(&mut self, request: &ImpactMarkRequest) -> Vec<RefPoly>;
}

/// Particle explosion request (`ParticleExplosionRequest`, minimal mirror).
#[derive(Debug, Clone, PartialEq)]
pub struct ParticleExplosion {
    /// Animation name.
    pub animation: String,
    /// Origin.
    pub origin: Vec3,
    /// Velocity.
    pub velocity: Vec3,
    /// Duration.
    pub duration: i32,
    /// Start size.
    pub size_start: f32,
    /// End size.
    pub size_end: f32,
}

/// Particle service (`ParticleSystem`, minimal mirror).
pub trait PresentParticles {
    /// Spawn an explosion.
    fn particle_explosion(&mut self, request: &ParticleExplosion);
}

/// Audio service.
pub trait PresentAudio {
    /// Start a positioned sound.
    fn start_sound(&mut self, sound: Option<PresentSound>, options: &SoundOptions);
}

/// Effect frame: clock and snapshot identity for effect constructors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EffectFrame {
    /// Time milliseconds.
    pub time: i32,
    /// Product.
    pub product: Product,
    /// Snapshot player client, when a snapshot is current.
    pub snap_client: Option<i32>,
    /// Predicted player client.
    pub predicted_client: i32,
}

// ---------------------------------------------------------------------------
// Sibling mirrors: client entities and game state (presentation/state.ts)
// ---------------------------------------------------------------------------

/// Animation lerp frame.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LerpFrame {
    /// Frame.
    pub frame: i32,
    /// Old frame.
    pub old_frame: i32,
    /// Back lerp.
    pub back_lerp: f32,
}

/// Player animation state (`ClientEntity.player`, minimal mirror).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayerAnimState {
    /// Torso frame.
    pub torso: LerpFrame,
    /// Barrel time.
    pub barrel_time: i32,
    /// Barrel angle.
    pub barrel_angle: f32,
    /// Barrel spinning.
    pub barrel_spinning: bool,
    /// Lightning firing.
    pub lightning_firing: i32,
    /// Railgun flash latched.
    pub railgun_flash: bool,
    /// Railgun impact point.
    pub railgun_impact: Vec3,
}

impl Default for PlayerAnimState {
    fn default() -> Self {
        Self {
            torso: LerpFrame::default(),
            barrel_time: 0,
            barrel_angle: 0.0,
            barrel_spinning: false,
            lightning_firing: 0,
            railgun_flash: false,
            railgun_impact: zero_vec3(),
        }
    }
}

/// Client entity (`ClientEntity`, minimal mirror).
#[derive(Debug, Clone, PartialEq)]
pub struct ClientEntity {
    /// Current state.
    pub current_state: EntityState,
    /// Next state.
    pub next_state: EntityState,
    /// Interpolate.
    pub interpolate: bool,
    /// Lerped origin.
    pub lerp_origin: Vec3,
    /// Lerped angles.
    pub lerp_angles: Vec3,
    /// Current valid.
    pub current_valid: bool,
    /// Misc time.
    pub misc_time: i32,
    /// Muzzle flash time.
    pub muzzle_flash_time: i32,
    /// Trail time.
    pub trail_time: i32,
    /// Player animation.
    pub player: PlayerAnimState,
}

impl Default for ClientEntity {
    fn default() -> Self {
        Self {
            current_state: EntityState::default(),
            next_state: EntityState::default(),
            interpolate: false,
            lerp_origin: zero_vec3(),
            lerp_angles: zero_vec3(),
            current_valid: false,
            misc_time: 0,
            muzzle_flash_time: 0,
            trail_time: 0,
            player: PlayerAnimState::default(),
        }
    }
}

/// Snapshot (`RetailSnapshot`, minimal mirror).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PresentSnapshot {
    /// Server time.
    pub server_time: i32,
    /// Entities.
    pub entities: Vec<EntityState>,
    /// Player state.
    pub player_state: SourcePlayerState,
    /// Area mask.
    pub area_mask: Vec<u8>,
}

/// Entity selection: predicted player or indexed entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresentEntityTarget {
    /// Predicted player entity.
    Predicted,
    /// Indexed entity.
    Indexed(usize),
}

/// Client game state (`ClientGameState`, fields touched by this port).
#[derive(Debug, Clone, PartialEq)]
pub struct ClientGameState {
    /// Product.
    pub product: Product,
    /// Time.
    pub time: i32,
    /// Client number.
    pub client_num: i32,
    /// Current snapshot.
    pub snap: Option<PresentSnapshot>,
    /// Next snapshot.
    pub next_snap: Option<PresentSnapshot>,
    /// Predicted player state.
    pub predicted_player_state: SourcePlayerState,
    /// Predicted player entity.
    pub predicted_player_entity: ClientEntity,
    /// Entities by number.
    pub entities: Vec<ClientEntity>,
    /// Frame interpolation.
    pub frame_interpolation: f32,
    /// Auto angles.
    pub auto_angles: Vec3,
    /// Fast auto angles.
    pub auto_angles_fast: Vec3,
    /// Auto axis.
    pub auto_axis: Axis,
    /// Fast auto axis.
    pub auto_axis_fast: Axis,
    /// Refdef.
    pub refdef: Refdef,
    /// Refdef view angles.
    pub refdef_view_angles: Vec3,
    /// Rendering third person.
    pub rendering_third_person: bool,
    /// Bob cycle.
    pub bob_cycle: i32,
    /// Bob fraction sine.
    pub bob_frac_sin: f32,
    /// XY speed.
    pub xyspeed: f32,
    /// Land time.
    pub land_time: i32,
    /// Land change.
    pub land_change: f32,
    /// Duck time.
    pub duck_time: i32,
    /// Duck change.
    pub duck_change: f32,
    /// Step time.
    pub step_time: i32,
    /// Step change.
    pub step_change: f32,
    /// Damage time.
    pub damage_time: f32,
    /// Damage pitch.
    pub damage_pitch: f32,
    /// Damage roll.
    pub damage_roll: f32,
    /// Damage value.
    pub damage_value: i32,
    /// Damage X.
    pub damage_x: f32,
    /// Damage Y.
    pub damage_y: f32,
    /// Kick angles.
    pub kick_angles: Vec3,
    /// Kick origin.
    pub kick_origin: Vec3,
    /// Hyperspace.
    pub hyperspace: bool,
    /// Zoomed.
    pub zoomed: bool,
    /// Zoom time.
    pub zoom_time: i32,
    /// Zoom sensitivity.
    pub zoom_sensitivity: f32,
    /// Test gun.
    pub test_gun: bool,
    /// Test model name.
    pub test_model_name: String,
    /// Test model entity.
    pub test_model_entity: RefModelEntity,
    /// Weapon select.
    pub weapon_select: i32,
    /// Weapon select time.
    pub weapon_select_time: i32,
    /// Item pickup time.
    pub item_pickup_time: i32,
    /// Next orbit time.
    pub next_orbit_time: i32,
    /// Predicted error.
    pub predicted_error: Vec3,
    /// Predicted error time.
    pub predicted_error_time: i32,
    /// Client frame.
    pub client_frame: i32,
}

impl ClientGameState {
    /// New state.
    #[must_use]
    pub fn new(product: Product) -> Self {
        Self {
            product,
            time: 0,
            client_num: 0,
            snap: None,
            next_snap: None,
            predicted_player_state: SourcePlayerState {
                product,
                ..SourcePlayerState::default()
            },
            predicted_player_entity: ClientEntity::default(),
            entities: Vec::new(),
            frame_interpolation: 0.0,
            auto_angles: zero_vec3(),
            auto_angles_fast: zero_vec3(),
            auto_axis: zero_axis(),
            auto_axis_fast: zero_axis(),
            refdef: Refdef::default(),
            refdef_view_angles: zero_vec3(),
            rendering_third_person: false,
            bob_cycle: 0,
            bob_frac_sin: 0.0,
            xyspeed: 0.0,
            land_time: 0,
            land_change: 0.0,
            duck_time: 0,
            duck_change: 0.0,
            step_time: 0,
            step_change: 0.0,
            damage_time: 0.0,
            damage_pitch: 0.0,
            damage_roll: 0.0,
            damage_value: 0,
            damage_x: 0.0,
            damage_y: 0.0,
            kick_angles: zero_vec3(),
            kick_origin: zero_vec3(),
            hyperspace: false,
            zoomed: false,
            zoom_time: 0,
            zoom_sensitivity: 1.0,
            test_gun: false,
            test_model_name: String::new(),
            test_model_entity: create_model_entity(default_model()),
            weapon_select: 0,
            weapon_select_time: 0,
            item_pickup_time: 0,
            next_orbit_time: 0,
            predicted_error: zero_vec3(),
            predicted_error_time: 0,
            client_frame: 0,
        }
    }

    /// Entity by number, growing storage (`entityAt`).
    pub fn entity_at(&mut self, number: usize) -> &mut ClientEntity {
        if self.entities.len() <= number {
            self.entities.resize_with(number + 1, ClientEntity::default);
        }
        &mut self.entities[number]
    }

    /// Entity by number (read-only).
    #[must_use]
    pub fn entity_ref(&self, number: usize) -> Option<&ClientEntity> {
        self.entities.get(number)
    }

    /// Selected entity (read-only).
    #[must_use]
    pub fn target_ref(&self, target: PresentEntityTarget) -> &ClientEntity {
        match target {
            PresentEntityTarget::Predicted => &self.predicted_player_entity,
            PresentEntityTarget::Indexed(index) => self.entities.get(index).unwrap_or(&self.predicted_player_entity),
        }
    }
}

// ---------------------------------------------------------------------------
// Sibling mirrors: items (base/shared/items.ts), direction byte,
// ballistics, grapple, hitscan
// ---------------------------------------------------------------------------

/// Item definition (`ItemDefinition`, minimal mirror).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemDefinition {
    /// Class name.
    pub class_name: String,
    /// Pickup name.
    pub pickup_name: Option<String>,
    /// Pickup sound.
    pub pickup_sound: Option<String>,
    /// Extra sounds (space separated).
    pub sounds: String,
    /// World models.
    pub world_models: [Option<String>; 2],
    /// Icon.
    pub icon: Option<String>,
    /// Item type.
    pub item_type: ItemType,
    /// Tag.
    pub tag: i32,
}

/// Item table (`itemList`/`itemAt`, minimal mirror).
pub trait PresentItemTable {
    /// Item count for a product.
    fn item_count(&self, product: Product) -> usize;
    /// Item by index.
    fn item_at(&self, product: Product, index: usize) -> Option<ItemDefinition>;
}

/// Direction-byte table (`BYTE_DIRECTIONS`, 162 entries).
#[rustfmt::skip]
pub(crate) const BYTE_DIRECTIONS: [[f32; 3]; 162] = [
    [-0.525731, 0.000000, 0.850651], [-0.442863, 0.238856, 0.864188],
    [-0.295242, 0.000000, 0.955423], [-0.309017, 0.500000, 0.809017],
    [-0.162460, 0.262866, 0.951056], [0.000000, 0.000000, 1.000000],
    [0.000000, 0.850651, 0.525731], [-0.147621, 0.716567, 0.681718],
    [0.147621, 0.716567, 0.681718], [0.000000, 0.525731, 0.850651],
    [0.309017, 0.500000, 0.809017], [0.525731, 0.000000, 0.850651],
    [0.295242, 0.000000, 0.955423], [0.442863, 0.238856, 0.864188],
    [0.162460, 0.262866, 0.951056], [-0.681718, 0.147621, 0.716567],
    [-0.809017, 0.309017, 0.500000], [-0.587785, 0.425325, 0.688191],
    [-0.850651, 0.525731, 0.000000], [-0.864188, 0.442863, 0.238856],
    [-0.716567, 0.681718, 0.147621], [-0.688191, 0.587785, 0.425325],
    [-0.500000, 0.809017, 0.309017], [-0.238856, 0.864188, 0.442863],
    [-0.425325, 0.688191, 0.587785], [-0.716567, 0.681718, -0.147621],
    [-0.500000, 0.809017, -0.309017], [-0.525731, 0.850651, 0.000000],
    [0.000000, 0.850651, -0.525731], [-0.238856, 0.864188, -0.442863],
    [0.000000, 0.955423, -0.295242], [-0.262866, 0.951056, -0.162460],
    [0.000000, 1.000000, 0.000000], [0.000000, 0.955423, 0.295242],
    [-0.262866, 0.951056, 0.162460], [0.238856, 0.864188, 0.442863],
    [0.262866, 0.951056, 0.162460], [0.500000, 0.809017, 0.309017],
    [0.238856, 0.864188, -0.442863], [0.262866, 0.951056, -0.162460],
    [0.500000, 0.809017, -0.309017], [0.850651, 0.525731, 0.000000],
    [0.716567, 0.681718, 0.147621], [0.716567, 0.681718, -0.147621],
    [0.525731, 0.850651, 0.000000], [0.425325, 0.688191, 0.587785],
    [0.864188, 0.442863, 0.238856], [0.688191, 0.587785, 0.425325],
    [0.809017, 0.309017, 0.500000], [0.681718, 0.147621, 0.716567],
    [0.587785, 0.425325, 0.688191], [0.955423, 0.295242, 0.000000],
    [1.000000, 0.000000, 0.000000], [0.951056, 0.162460, 0.262866],
    [0.850651, -0.525731, 0.000000], [0.955423, -0.295242, 0.000000],
    [0.864188, -0.442863, 0.238856], [0.951056, -0.162460, 0.262866],
    [0.809017, -0.309017, 0.500000], [0.681718, -0.147621, 0.716567],
    [0.850651, 0.000000, 0.525731], [0.864188, 0.442863, -0.238856],
    [0.809017, 0.309017, -0.500000], [0.951056, 0.162460, -0.262866],
    [0.525731, 0.000000, -0.850651], [0.681718, 0.147621, -0.716567],
    [0.681718, -0.147621, -0.716567], [0.850651, 0.000000, -0.525731],
    [0.809017, -0.309017, -0.500000], [0.864188, -0.442863, -0.238856],
    [0.951056, -0.162460, -0.262866], [0.147621, 0.716567, -0.681718],
    [0.309017, 0.500000, -0.809017], [0.425325, 0.688191, -0.587785],
    [0.442863, 0.238856, -0.864188], [0.587785, 0.425325, -0.688191],
    [0.688191, 0.587785, -0.425325], [-0.147621, 0.716567, -0.681718],
    [-0.309017, 0.500000, -0.809017], [0.000000, 0.525731, -0.850651],
    [-0.525731, 0.000000, -0.850651], [-0.442863, 0.238856, -0.864188],
    [-0.295242, 0.000000, -0.955423], [-0.162460, 0.262866, -0.951056],
    [0.000000, 0.000000, -1.000000], [0.295242, 0.000000, -0.955423],
    [0.162460, 0.262866, -0.951056], [-0.442863, -0.238856, -0.864188],
    [-0.309017, -0.500000, -0.809017], [-0.162460, -0.262866, -0.951056],
    [0.000000, -0.850651, -0.525731], [-0.147621, -0.716567, -0.681718],
    [0.147621, -0.716567, -0.681718], [0.000000, -0.525731, -0.850651],
    [0.309017, -0.500000, -0.809017], [0.442863, -0.238856, -0.864188],
    [0.162460, -0.262866, -0.951056], [0.238856, -0.864188, -0.442863],
    [0.500000, -0.809017, -0.309017], [0.425325, -0.688191, -0.587785],
    [0.716567, -0.681718, -0.147621], [0.688191, -0.587785, -0.425325],
    [0.587785, -0.425325, -0.688191], [0.000000, -0.955423, -0.295242],
    [0.000000, -1.000000, 0.000000], [0.262866, -0.951056, -0.162460],
    [0.000000, -0.850651, 0.525731], [0.000000, -0.955423, 0.295242],
    [0.238856, -0.864188, 0.442863], [0.262866, -0.951056, 0.162460],
    [0.500000, -0.809017, 0.309017], [0.716567, -0.681718, 0.147621],
    [0.525731, -0.850651, 0.000000], [-0.238856, -0.864188, -0.442863],
    [-0.500000, -0.809017, -0.309017], [-0.262866, -0.951056, -0.162460],
    [-0.850651, -0.525731, 0.000000], [-0.716567, -0.681718, -0.147621],
    [-0.716567, -0.681718, 0.147621], [-0.525731, -0.850651, 0.000000],
    [-0.500000, -0.809017, 0.309017], [-0.238856, -0.864188, 0.442863],
    [-0.262866, -0.951056, 0.162460], [-0.864188, -0.442863, 0.238856],
    [-0.809017, -0.309017, 0.500000], [-0.688191, -0.587785, 0.425325],
    [-0.681718, -0.147621, 0.716567], [-0.442863, -0.238856, 0.864188],
    [-0.587785, -0.425325, 0.688191], [-0.309017, -0.500000, 0.809017],
    [-0.147621, -0.716567, 0.681718], [-0.425325, -0.688191, 0.587785],
    [-0.162460, -0.262866, 0.951056], [0.442863, -0.238856, 0.864188],
    [0.162460, -0.262866, 0.951056], [0.309017, -0.500000, 0.809017],
    [0.147621, -0.716567, 0.681718], [0.000000, -0.525731, 0.850651],
    [0.425325, -0.688191, 0.587785], [0.587785, -0.425325, 0.688191],
    [0.688191, -0.587785, 0.425325], [-0.955423, 0.295242, 0.000000],
    [-0.951056, 0.162460, 0.262866], [-1.000000, 0.000000, 0.000000],
    [-0.850651, 0.000000, 0.525731], [-0.955423, -0.295242, 0.000000],
    [-0.951056, -0.162460, 0.262866], [-0.864188, 0.442863, -0.238856],
    [-0.951056, 0.162460, -0.262866], [-0.809017, 0.309017, -0.500000],
    [-0.864188, -0.442863, -0.238856], [-0.951056, -0.162460, -0.262866],
    [-0.809017, -0.309017, -0.500000], [-0.681718, 0.147621, -0.716567],
    [-0.681718, -0.147621, -0.716567], [-0.850651, 0.000000, -0.525731],
    [-0.688191, 0.587785, -0.425325], [-0.587785, 0.425325, -0.688191],
    [-0.425325, 0.688191, -0.587785], [-0.425325, -0.688191, -0.587785],
    [-0.587785, -0.425325, -0.688191], [-0.688191, -0.587785, -0.425325],
];

/// Direction from a byte (`byteToDirection`).
#[must_use]
pub fn byte_to_direction(byte: i32) -> Vec3 {
    if byte < 0 || byte as usize >= BYTE_DIRECTIONS.len() {
        return zero_vec3();
    }
    let entry = BYTE_DIRECTIONS[byte as usize];
    vec3(entry[0], entry[1], entry[2])
}

/// Shotgun event (`Q3ShotgunEvent`, minimal mirror).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q3ShotgunEvent {
    /// Muzzle.
    pub muzzle: Vec3,
    /// Direction.
    pub direction: Vec3,
    /// Seed.
    pub seed: i32,
}

/// Shotgun pellet endpoints (`q3ShotgunEndpoints`).
#[must_use]
pub fn q3_shotgun_endpoints(origin: Vec3, direction: Vec3, initial_seed: i32) -> Vec<Vec3> {
    let forward = normalize3_or_zero(direction);
    let right = perpendicular_vector(forward);
    let up = cross3(forward, right);
    let mut seed = initial_seed;
    let mut ends = Vec::with_capacity(11);
    for _ in 0..11 {
        let r = q_crandom(seed);
        let u = q_crandom(r.seed);
        seed = u.seed;
        let horizontal = (r.value as f32) * 700.0 * 16.0;
        let vertical = (u.value as f32) * 700.0 * 16.0;
        ends.push(add3(
            add3(add3(origin, scale3(forward, 131072.0)), scale3(right, horizontal)),
            scale3(up, vertical),
        ));
    }
    ends
}

/// Grapple cable (`q3GrappleCable`).
#[must_use]
pub fn q3_grapple_cable(origin: Vec3, up: Vec3, point: Vec3) -> Option<(Vec3, Vec3)> {
    let start = add3(add3(origin, vec3(0.0, 0.0, 26.0)), scale3(up, -6.0));
    if length3(sub3(start, point)) < 64.0 {
        return None;
    }
    Some((start, point))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q3::presentation::scene::*;

    use qa_core::math::{vec2, vec3, vec4, Bounds, Vec3};

    use crate::q3::presentation::character_resources::*;
    use crate::q3::presentation::effects::*;
    use crate::q3::presentation::entities::*;
    use crate::q3::presentation::local_entities::*;
    use crate::q3::presentation::mark_projector::*;

    use crate::q3::presentation::model_access::*;

    #[test]
    fn render_text_bounds() {
        let text: RenderText = ["a", "b", "c", "d", "e", "f", "g", "h"].map(str::to_string);
        assert_eq!(copy_render_text(&text).unwrap(), text);
        let mut bad = text.clone();
        bad[0] = "x".repeat(33);
        assert!(copy_render_text(&bad).is_err());
        let mut bad = text.clone();
        bad[1] = "é".to_string();
        assert!(copy_render_text(&bad).is_err());
        let refdef = create_refdef();
        assert_eq!(refdef.area_mask.len(), 32);
        assert_eq!(copy_refdef(&refdef), refdef);
    }

    #[test]
    fn entity_factories() {
        let model = create_model_entity(default_model());
        assert_eq!(model.shading.shader_rgba, vec4(0.0, 0.0, 0.0, 0.0));
        assert_eq!(RefEntity::Model(model.clone()).kind(), "model");
        assert_eq!(copy_ref_entity(&RefEntity::Model(model)).kind(), "model");
        assert_eq!(create_sprite_entity().radius, 0.0);
        assert_eq!(create_beam_entity().radius, 0.0);
        assert_eq!(create_portal_entity().frame, 0);
        let poly = RefPoly {
            shader: Some(SceneShader::new("s")),
            vertices: vec![RefPolyVertex {
                position: zero_vec3(),
                tex_coord: vec2(0.0, 0.0),
                color: vec4(1.0, 2.0, 3.0, 4.0),
            }],
        };
        assert_eq!(copy_ref_poly(&poly), poly);
    }

    #[test]
    fn fog_admission() {
        let poly = RefPoly {
            shader: Some(SceneShader::new("s")),
            vertices: vec![RefPolyVertex {
                position: vec3(1.0, 1.0, 1.0),
                tex_coord: vec2(0.0, 0.0),
                color: vec4(1.0, 1.0, 1.0, 1.0),
            }],
        };
        let fogs = vec![Q3FogSelection {
            index: 2,
            volume: FogVolume {
                bounds: Bounds {
                    min: zero_vec3(),
                    max: vec3(2.0, 2.0, 2.0),
                },
            },
        }];
        let admitted = admit_q3_poly(&poly, &fogs).unwrap();
        assert_eq!(admitted.fog.map(|fog| fog.index), Some(2));
        assert!(q3_procedural_fog(vec3(1.0, 1.0, 1.0), 0.5, &fogs).is_some());
        assert!(q3_procedural_fog(vec3(50.0, 50.0, 50.0), 0.5, &fogs).is_none());
        let empty = RefPoly {
            shader: Some(SceneShader::new("s")),
            vertices: Vec::new(),
        };
        assert!(admit_q3_poly(&empty, &fogs).is_err());
        assert!(admit_q3_poly(&empty, &[]).unwrap().fog.is_none());
    }

    #[test]
    fn scene_capture_routes_entities() {
        struct Target {
            published: Vec<Q3PresentedScene>,
        }
        impl Q3SceneTarget for Target {
            fn seat(&self) -> SeatId {
                1
            }
            fn viewport(&self) -> Rect {
                Rect {
                    x: 0,
                    y: 0,
                    width: 640,
                    height: 480,
                }
            }
            fn far_clip(&self) -> f32 {
                1000.0
            }
            fn near_clip(&self) -> f32 {
                1.0
            }
            fn rail(&self) -> RailSettings {
                RailSettings
            }
            fn fog_selections(&self) -> Vec<Q3FogSelection> {
                Vec::new()
            }
            fn print(&mut self, _text: &str) {}
            fn actor(&self, _entity: &RefModelEntity) -> Option<ActorId> {
                Some(7)
            }
            fn publish(&mut self, scene: Q3PresentedScene) {
                self.published.push(scene);
            }
        }
        let mut recorder = Q3SceneRecorder::new(Box::new(Target { published: Vec::new() }));
        let mut sprite = create_sprite_entity();
        sprite.shading.custom_shader = Some(SceneShader::new("fx"));
        recorder.add_ref_entity(&Q3AdmittedRefEntity::Entity(RefEntity::Sprite(sprite)));
        recorder.add_ref_entity(&Q3AdmittedRefEntity::Entity(RefEntity::Beam(create_beam_entity())));
        recorder.add_light(&DynamicLight {
            origin: zero_vec3(),
            radius: 10.0,
            color: vec3(1.0, 1.0, 1.0),
            additive: false,
        });
        let content = recorder.capture();
        assert_eq!(content.effects.len(), 1);
        assert_eq!(content.special_entities.len(), 1);
        assert_eq!(content.lights.len(), 1);
        let mut refdef = create_refdef();
        refdef.width = 640;
        refdef.height = 480;
        refdef.fov_x = 90.0;
        refdef.fov_y = 60.0;
        recorder.render_scene(&refdef);
    }

    #[test]
    fn trajectory_evaluation() {
        let linear = Trajectory {
            type_: TrajectoryType::Linear,
            time: 0,
            duration: 0,
            base: vec3(1.0, 2.0, 3.0),
            delta: vec3(10.0, 0.0, 0.0),
        };
        assert_eq!(evaluate_trajectory(&linear, 1000), vec3(11.0, 2.0, 3.0));
        assert_eq!(evaluate_trajectory_delta(&linear, 500), vec3(10.0, 0.0, 0.0));
        let gravity = Trajectory {
            type_: TrajectoryType::Gravity,
            time: 0,
            duration: 0,
            base: zero_vec3(),
            delta: zero_vec3(),
        };
        let fallen = evaluate_trajectory(&gravity, 1000);
        assert!((fallen.z + 400.0).abs() < 0.01);
        let stop = Trajectory {
            type_: TrajectoryType::LinearStop,
            time: 0,
            duration: 500,
            base: zero_vec3(),
            delta: vec3(10.0, 0.0, 0.0),
        };
        assert_eq!(evaluate_trajectory(&stop, 5000).x, 5.0);
        assert_eq!(evaluate_trajectory_delta(&stop, 5000), zero_vec3());
    }

    #[test]
    fn character_resources() {
        assert_eq!(Q3_CHARACTER_SOUNDS.select_sound, "sound/weapons/change.wav");
        assert_eq!(Q3_FOOTSTEP_PATHS.len(), 7);
        assert_eq!(
            q3_custom_sound_fallback(Product::MissionPack, true),
            CustomSoundFallback::James
        );
        assert_eq!(
            q3_custom_sound_fallback(Product::BaseQ3, true),
            CustomSoundFallback::Sarge
        );
        assert_eq!(CustomSoundFallback::Sarge.name(), "sarge");
    }

    #[test]
    fn model_accessors() {
        assert_eq!(
            model_bounds(&default_model()),
            Bounds {
                min: zero_vec3(),
                max: zero_vec3()
            }
        );
        let inline_model = SceneModel::Inline(SceneInlineModel {
            path: "*1".to_string(),
            index: 1,
            geometry: PresentWorld::new("world"),
            resource: PresentResource::new("*1"),
            bounds: Bounds {
                min: vec3(1.0, 1.0, 1.0),
                max: vec3(2.0, 2.0, 2.0),
            },
        });
        assert_eq!(model_bounds(&inline_model).max, vec3(2.0, 2.0, 2.0));
        assert!(lerp_model_tag(&default_model(), "tag", 0, 0, 0.0).is_none());
    }

    #[test]
    fn mark_projection_empty() {
        let projector = BspMarkProjector::new(MarkGeometry {
            map: MarkMap {
                nodes: Vec::new(),
                planes: Vec::new(),
                leaves: Vec::new(),
                leaf_surfaces: Vec::new(),
                surface_count: 0,
            },
            surfaces: Vec::new(),
        })
        .unwrap();
        let fragments = projector
            .mark_fragments(&MarkProjection {
                points: vec![zero_vec3()],
                projection: vec3(0.0, 0.0, -1.0),
                max_points: 10,
                max_fragments: 10,
            })
            .unwrap();
        assert!(fragments.fragments.is_empty());
        struct Zero;
        impl SourceMarkProjection for Zero {
            fn point_count(&self) -> usize {
                1
            }
            fn max_points(&self) -> usize {
                0
            }
            fn max_fragments(&self) -> usize {
                0
            }
            fn read_point(&self, _index: usize) -> PresentResult<Vec3> {
                Ok(zero_vec3())
            }
            fn read_projection(&self) -> Vec3 {
                vec3(0.0, 0.0, -1.0)
            }
            fn write_fragment(&mut self, _index: usize, _fragment: MarkFragment) {}
            fn write_points(&mut self, _first_point: usize, _points: &[Vec3]) {}
        }
        assert_eq!(projector.mark_fragments_record(&mut Zero).unwrap(), 0);
    }

    struct MarkOptions {
        clock: i32,
        enabled: bool,
    }
    impl ImpactMarkOptions for MarkOptions {
        fn clock(&self) -> i32 {
            self.clock
        }
        fn enabled(&self) -> bool {
            self.enabled
        }
        fn energy_shader(&self) -> Option<SceneShader> {
            None
        }
    }

    fn mark_projector() -> BspMarkProjector {
        BspMarkProjector::new(MarkGeometry {
            map: MarkMap {
                nodes: Vec::new(),
                planes: Vec::new(),
                leaves: Vec::new(),
                leaf_surfaces: Vec::new(),
                surface_count: 0,
            },
            surfaces: Vec::new(),
        })
        .unwrap()
    }

    #[test]
    fn impact_marks_validate() {
        let mut system = ImpactMarkSystem::new(
            mark_projector(),
            Box::new(MarkOptions {
                clock: 1000,
                enabled: true,
            }),
        );
        let request = ImpactMarkRequest {
            shader: Some(SceneShader::new("mark")),
            origin: zero_vec3(),
            direction: vec3(0.0, 0.0, 1.0),
            orientation: 0.0,
            color: vec4(1.0, 1.0, 1.0, 1.0),
            alpha_fade: true,
            radius: 8.0,
            temporary: true,
        };
        assert!(system.impact_mark(&request).unwrap().is_empty());
        assert_eq!(system.active_mark_count(), 0);
        let bad = ImpactMarkRequest {
            radius: 0.0,
            ..request.clone()
        };
        assert!(system.impact_mark(&bad).is_err());
        let zero_dir = ImpactMarkRequest {
            direction: zero_vec3(),
            ..request.clone()
        };
        assert!(system.impact_mark(&zero_dir).unwrap().is_empty());
        let bad_color = ImpactMarkRequest {
            color: vec4(2.0, 0.0, 0.0, 1.0),
            ..request.clone()
        };
        assert!(system.impact_mark(&bad_color).is_err());
        system.reset();
    }

    #[test]
    fn local_pool_lifecycle() {
        let mut pool = LocalEntityPool::new(Product::BaseQ3);
        let sprite = RefEntity::Sprite(create_sprite_entity());
        let handle = pool.allocate(LocalEntityType::MoveScaleFade, sprite).unwrap();
        assert!(pool.is_active(handle));
        assert_eq!(pool.active_count(), 1);
        assert!(pool.get(handle).is_some());
        pool.free(handle).unwrap();
        assert!(!pool.is_active(handle));
        assert!(pool.free(handle).is_err());
        let model = RefEntity::Model(create_model_entity(default_model()));
        assert!(pool.allocate(LocalEntityType::Kamikaze, model).is_err());
        let sprite = RefEntity::Sprite(create_sprite_entity());
        assert!(pool.allocate(LocalEntityType::Fragment, sprite).is_err());
    }

    #[test]
    fn local_pool_evicts_oldest() {
        let mut pool = LocalEntityPool::new(Product::MissionPack);
        for _ in 0..MAX_LOCAL_ENTITIES {
            pool.allocate(LocalEntityType::Mark, RefEntity::Sprite(create_sprite_entity()))
                .unwrap();
        }
        assert_eq!(pool.active_count(), MAX_LOCAL_ENTITIES);
        pool.allocate(LocalEntityType::Mark, RefEntity::Sprite(create_sprite_entity()))
            .unwrap();
        assert_eq!(pool.active_count(), MAX_LOCAL_ENTITIES);
    }

    struct TestEffects {
        next_rand: i32,
        sounds: Vec<(Vec3, i32, i32, Option<PresentSound>)>,
    }
    impl EffectImports for TestEffects {
        fn random_integer(&mut self) -> i32 {
            let value = self.next_rand;
            self.next_rand = (self.next_rand + 1) % 32768;
            value
        }
        fn start_sound(&mut self, origin: Vec3, entity: i32, channel: i32, sound: Option<PresentSound>) {
            self.sounds.push((origin, entity, channel, sound));
        }
    }

    fn test_media() -> EffectMedia {
        EffectMedia {
            water_bubble_shader: None,
            smoke_puff_rage_pro_shader: None,
            blood_explosion_shader: None,
            teleport_effect_model: default_model(),
            gib_skull: default_model(),
            gib_brain: default_model(),
            gib_abdomen: default_model(),
            gib_arm: default_model(),
            gib_chest: default_model(),
            gib_fist: default_model(),
            gib_foot: default_model(),
            gib_forearm: default_model(),
            gib_intestine: default_model(),
            gib_leg: default_model(),
            smoke2: default_model(),
            variant: EffectMediaVariant::Base {
                teleport_effect_shader: None,
            },
        }
    }

    fn test_options() -> EffectOptions {
        EffectOptions {
            no_projectile_trail: false,
            blood: true,
            gibs: true,
            score_plum: true,
            hardware_rage_pro: false,
        }
    }

    #[test]
    fn effects_smoke_and_explosion() {
        let mut pool = LocalEntityPool::new(Product::BaseQ3);
        let mut effects = ClientEffects::new(
            Product::BaseQ3,
            Product::BaseQ3,
            test_media(),
            test_options(),
            Box::new(TestEffects {
                next_rand: 7,
                sounds: Vec::new(),
            }),
        )
        .unwrap();
        let frame = EffectFrame {
            time: 1000,
            product: Product::BaseQ3,
            snap_client: Some(0),
            predicted_client: 0,
        };
        let handle = effects
            .smoke_puff(
                &mut pool,
                &frame,
                &SmokePuffOptions {
                    origin: zero_vec3(),
                    velocity: zero_vec3(),
                    radius: 10.0,
                    color: vec4(1.0, 1.0, 1.0, 1.0),
                    duration: 500,
                    start_time: 1000,
                    fade_in_time: 0,
                    flags: 0,
                    shader: None,
                },
            )
            .unwrap();
        assert_eq!(pool.get(handle).map(|le| le.end_time), Some(1500));
        let bad = effects.make_explosion(
            &mut pool,
            &frame,
            &ExplosionOptions {
                origin: zero_vec3(),
                direction: None,
                model: default_model(),
                shader: None,
                duration: 0,
                sprite: false,
            },
        );
        assert!(bad.is_err());
        effects
            .bubble_trail(&mut pool, &frame, zero_vec3(), vec3(0.0, 0.0, 64.0), 16.0)
            .unwrap();
        assert!(pool.active_count() >= 2);
        effects.gib_player(&mut pool, &frame, zero_vec3()).unwrap();
        assert!(pool.active_count() >= 12);
    }

    #[test]
    fn shotgun_endpoints_and_cable() {
        let ends = q3_shotgun_endpoints(zero_vec3(), vec3(1.0, 0.0, 0.0), 42);
        assert_eq!(ends.len(), 11);
        assert!(q3_grapple_cable(zero_vec3(), vec3(0.0, 0.0, 1.0), zero_vec3()).is_none());
        assert!(q3_grapple_cable(zero_vec3(), vec3(0.0, 0.0, 1.0), vec3(500.0, 0.0, 0.0)).is_some());
        assert_eq!(byte_to_direction(5), vec3(0.0, 0.0, 1.0));
        assert_eq!(byte_to_direction(999), zero_vec3());
    }

    #[test]
    fn tag_placement_identity() {
        let parent = create_model_entity(default_model());
        let mut entity = create_model_entity(default_model());
        position_entity_on_tag(&mut entity, &parent, &default_model(), "tag_missing");
        assert_eq!(entity.origin, zero_vec3());
        position_rotated_entity_on_tag(&mut entity, &parent, &default_model(), "tag_missing");
        assert_eq!(entity.origin, zero_vec3());
        let state = ClientGameState::new(Product::BaseQ3);
        assert_eq!(
            adjust_position_for_mover(&state, vec3(1.0, 2.0, 3.0), 0, 0, 100),
            vec3(1.0, 2.0, 3.0)
        );
    }

    #[test]
    fn stat_schema_and_counts() {
        assert_eq!(stat_schema(Product::BaseQ3).weapons, 2);
        assert_eq!(stat_schema(Product::MissionPack).weapons, 3);
        assert_eq!(weapon_count(Product::BaseQ3), 11);
        assert_eq!(weapon_count(Product::MissionPack), 14);
        assert_eq!(EntityType::from_i32(13), Some(EntityType::Events));
        assert_eq!(Weapon::from_i32(7), Some(Weapon::Railgun));
    }
}
