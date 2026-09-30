//! Quake III presentation scene and render submission (`src/content/q3/presentation`).
//!
//! Donor provenance: `scene-host.ts`, `collision-host.ts`, `movement-host.ts`,
//! `character-resources.ts`, `model-access.ts`, `refdef.ts`, `scene.ts`,
//! `ref-entity.ts`, `marks.ts`, `mark-projector.ts`, `effects.ts`,
//! `local-entities.ts`, `entities.ts`, `weapons.ts`, `media.ts`, `view.ts`
//! (`cg_ents.c`, `cg_effects.c`, `cg_localents.c`, `cg_marks.c`,
//! `cg_view.c`, `cg_weapons.c`, `cg_main.c`, `tr_marks.c`).
//!
//! Types imported by the donors from modules outside this port (base shared
//! state, prediction, render contracts, audio) are mirrored locally in the
//! `mirror` section so this module stays self-contained. Async donor
//! registration boundaries are synchronous here; callers supply trait
//! implementations that have already loaded their bytes.

use std::collections::HashSet;
use std::fmt;

use qa_core::math::{
    add3, angle_mod, angle_vectors, angles_to_axis, box_on_plane_side, cross3, dot3, length3, normalize3,
    normalize3_or_zero, perpendicular_vector, rotate_point_around_vector, scale3, sub3, vec2, vec3, vec4, Axis, Bounds,
    Plane, Vec2, Vec3, Vec4,
};
use qa_core::numeric::{q_crandom, q_random, qvm_float_to_int};

use crate::md3::{interpolate_md3_tags, normalize_fast3, Md3Model};
use crate::md5::{Md5AnimationFrame, sample_md5_pose};
use crate::q3scene::joint_attachment_tag;

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Product {
    /// Base Quake III.
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
pub enum WeaponState {
    /// Ready.
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

fn trajectory_seconds(milliseconds: i32) -> f32 {
    (milliseconds as f32) * 0.001
}

fn periodic_radians(trajectory: &Trajectory, at_time: i32) -> f32 {
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
// movement-host.ts
// ---------------------------------------------------------------------------

/// Movement trace with the hit entity (`MovementTrace`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MovementTrace {
    /// Base trace.
    pub base: TraceResult,
    /// Hit entity number.
    pub entity_num: i32,
}

/// Movement callback bundle (`PresentationMovementOptions`).
pub trait PresentationMovementOptions {
    /// Original server time.
    fn original_server_time(&self) -> Option<i32> {
        None
    }
    /// Trace.
    fn trace(&self, start: Vec3, end: Vec3, bounds: Bounds, skip_number: i32, mask: i32) -> MovementTrace;
    /// Point contents.
    fn point_contents(&self, point: Vec3, pass_entity: i32) -> i32;
    /// Trace mask.
    fn trace_mask(&self) -> i32;
    /// Fixed msec, when fixed-step physics is on.
    fn fixed_msec(&self) -> Option<i32>;
    /// Footsteps disabled.
    fn no_footsteps(&self) -> bool;
    /// Gauntlet hit pending.
    fn gauntlet_hit(&self) -> bool;
}

/// Command timing owner (`"q3" | "provider"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandTiming {
    /// Quake III timing.
    Q3,
    /// Provider timing.
    Provider,
}

/// Bounds returned by player movement.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MoveBounds {
    /// Player bounds.
    pub bounds: Bounds,
}

/// Movement host (`PresentationMovementHost`).
pub trait PresentationMovementHost {
    /// Command timing owner.
    fn command_timing(&self) -> CommandTiming;
    /// Move the player.
    fn move_player(
        &mut self,
        state: &mut SourcePlayerState,
        command: &UserCommand,
        options: &dyn PresentationMovementOptions,
    ) -> MoveBounds;
    /// Update view angles from the command.
    fn update_view_angles(&mut self, state: &mut SourcePlayerState, command: &UserCommand);
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

impl Default for Product {
    fn default() -> Self {
        Self::BaseQ3
    }
}

impl Default for MoveType {
    fn default() -> Self {
        Self::Normal
    }
}

impl Default for WeaponState {
    fn default() -> Self {
        Self::Ready
    }
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
// ref-entity.ts
// ---------------------------------------------------------------------------

/// Decoded model payload (`DecodedModel`, minimal mirror).
#[derive(Debug, Clone, PartialEq)]
pub enum Q3DecodedModel {
    /// Quake III MD3.
    Md3(Md3Model),
    /// MD5 with joint names and animation frames.
    Md5(PresentMd5),
    /// Model exposing `bounds` directly.
    Bounded {
        /// Bounds.
        bounds: Bounds,
    },
    /// Brush model referencing world submodels.
    Brush {
        /// World submodel bounds.
        models: Vec<Bounds>,
        /// Selected submodel.
        index: usize,
    },
    /// Framed model exposing `frames[0].bounds`.
    Framed {
        /// Frame bounds.
        frames: Vec<Bounds>,
    },
}

/// MD5 presentation payload.
#[derive(Debug, Clone, PartialEq)]
pub struct PresentMd5 {
    /// Joint names in pose order.
    pub joint_names: Vec<String>,
    /// Animation frames.
    pub frames: Vec<Md5AnimationFrame>,
}

/// Default scene model (`SceneDefaultModel`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneDefaultModel {
    /// Always `*default`.
    pub path: String,
}

/// Loaded scene model (`SceneLoadedModel`).
#[derive(Debug, Clone, PartialEq)]
pub struct SceneLoadedModel {
    /// Path.
    pub path: String,
    /// Decoded model.
    pub model: Q3DecodedModel,
    /// Resource.
    pub resource: PresentResource,
}

/// Inline scene model (`SceneInlineModel`).
#[derive(Debug, Clone, PartialEq)]
pub struct SceneInlineModel {
    /// Path.
    pub path: String,
    /// Index.
    pub index: usize,
    /// Geometry.
    pub geometry: PresentWorld,
    /// Resource.
    pub resource: PresentResource,
    /// Bounds.
    pub bounds: Bounds,
}

/// Scene model (`SceneModel`).
#[derive(Debug, Clone, PartialEq)]
pub enum SceneModel {
    /// Default placeholder.
    Default(SceneDefaultModel),
    /// Loaded model.
    Loaded(SceneLoadedModel),
    /// Inline brush model.
    Inline(SceneInlineModel),
}

impl SceneModel {
    /// Default model.
    #[must_use]
    pub fn default_model() -> Self {
        Self::Default(SceneDefaultModel {
            path: "*default".to_string(),
        })
    }

    /// Whether this is the default placeholder.
    #[must_use]
    pub fn is_default(&self) -> bool {
        matches!(self, Self::Default(_))
    }
}

/// Default model constant (`DEFAULT_MODEL`).
#[must_use]
pub fn default_model() -> SceneModel {
    SceneModel::default_model()
}

/// Scene shader (`SceneShader`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SceneShader {
    /// Name.
    pub name: String,
}

impl SceneShader {
    /// New shader.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

/// Scene skin (`SceneSkin`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneSkin {
    /// Path.
    pub path: String,
    /// Surfaces.
    pub surfaces: Vec<SkinMapping>,
}

/// Minimum light render flag.
pub const RF_MINLIGHT: i32 = 1;
/// Third-person render flag.
pub const RF_THIRD_PERSON: i32 = 2;
/// First-person render flag.
pub const RF_FIRST_PERSON: i32 = 4;
/// Depth-hack render flag.
pub const RF_DEPTHHACK: i32 = 8;
/// No-shadow render flag.
pub const RF_NOSHADOW: i32 = 64;
/// Lighting-origin render flag.
pub const RF_LIGHTING_ORIGIN: i32 = 128;
/// Shadow-plane render flag.
pub const RF_SHADOW_PLANE: i32 = 256;
/// Wrap-frames render flag.
pub const RF_WRAP_FRAMES: i32 = 512;

/// Shading fields shared by shaded entities (`ShadedEntity`).
#[derive(Debug, Clone, PartialEq)]
pub struct ShadedFields {
    /// Render flags.
    pub render_flags: i32,
    /// Custom shader.
    pub custom_shader: Option<SceneShader>,
    /// Source byte channels, including zero defaults.
    pub shader_rgba: Vec4,
    /// Shader texture coordinate.
    pub shader_tex_coord: Vec2,
    /// Seconds subtracted from the scene shader clock.
    pub shader_time: f32,
}

impl Default for ShadedFields {
    fn default() -> Self {
        Self {
            render_flags: 0,
            custom_shader: None,
            shader_rgba: vec4(0.0, 0.0, 0.0, 0.0),
            shader_tex_coord: vec2(0.0, 0.0),
            shader_time: 0.0,
        }
    }
}

/// Model reference entity (`RefModelEntity`).
#[derive(Debug, Clone, PartialEq)]
pub struct RefModelEntity {
    /// Shading.
    pub shading: ShadedFields,
    /// Model.
    pub model: SceneModel,
    /// Origin.
    pub origin: Vec3,
    /// Old origin.
    pub old_origin: Vec3,
    /// Axis.
    pub axis: Axis,
    /// Non-normalized axes.
    pub non_normalized_axes: bool,
    /// Lighting origin.
    pub lighting_origin: Vec3,
    /// Shadow plane.
    pub shadow_plane: f32,
    /// Frame.
    pub frame: i32,
    /// Old frame.
    pub old_frame: i32,
    /// Back lerp.
    pub back_lerp: f32,
    /// Skin number.
    pub skin_num: i32,
    /// Custom skin.
    pub custom_skin: Option<SceneSkin>,
}

/// Sprite reference entity (`RefSpriteEntity`).
#[derive(Debug, Clone, PartialEq)]
pub struct RefSpriteEntity {
    /// Shading.
    pub shading: ShadedFields,
    /// Origin.
    pub origin: Vec3,
    /// Radius.
    pub radius: f32,
    /// Rotation.
    pub rotation: f32,
}

/// Beam reference entity (`RefBeamEntity`).
#[derive(Debug, Clone, PartialEq)]
pub struct RefBeamEntity {
    /// Shading.
    pub shading: ShadedFields,
    /// Origin.
    pub origin: Vec3,
    /// Old origin.
    pub old_origin: Vec3,
    /// Axis.
    pub axis: Axis,
    /// Radius (used for sprite-fog selection; beam geometry is radius four).
    pub radius: f32,
}

/// Rail-core reference entity (`RefRailCoreEntity`).
#[derive(Debug, Clone, PartialEq)]
pub struct RefRailCoreEntity {
    /// Shading.
    pub shading: ShadedFields,
    /// Origin.
    pub origin: Vec3,
    /// Old origin.
    pub old_origin: Vec3,
    /// Radius.
    pub radius: f32,
}

/// Rail-rings reference entity (`RefRailRingsEntity`).
#[derive(Debug, Clone, PartialEq)]
pub struct RefRailRingsEntity {
    /// Shading.
    pub shading: ShadedFields,
    /// Origin.
    pub origin: Vec3,
    /// Old origin.
    pub old_origin: Vec3,
    /// Radius.
    pub radius: f32,
}

/// Lightning reference entity (`RefLightningEntity`).
#[derive(Debug, Clone, PartialEq)]
pub struct RefLightningEntity {
    /// Shading.
    pub shading: ShadedFields,
    /// Origin.
    pub origin: Vec3,
    /// Old origin.
    pub old_origin: Vec3,
    /// Radius.
    pub radius: f32,
}

/// Portal reference entity (`RefPortalEntity`).
#[derive(Debug, Clone, PartialEq)]
pub struct RefPortalEntity {
    /// Render flags.
    pub render_flags: i32,
    /// Origin.
    pub origin: Vec3,
    /// Old origin.
    pub old_origin: Vec3,
    /// Axis.
    pub axis: Axis,
    /// Frame.
    pub frame: i32,
    /// Old frame.
    pub old_frame: i32,
    /// Skin number.
    pub skin_num: i32,
}

/// Reference entity (`RefEntity`).
#[derive(Debug, Clone, PartialEq)]
pub enum RefEntity {
    /// Model.
    Model(RefModelEntity),
    /// Sprite.
    Sprite(RefSpriteEntity),
    /// Beam.
    Beam(RefBeamEntity),
    /// Rail core.
    RailCore(RefRailCoreEntity),
    /// Rail rings.
    RailRings(RefRailRingsEntity),
    /// Lightning.
    Lightning(RefLightningEntity),
    /// Portal surface.
    Portal(RefPortalEntity),
}

impl RefEntity {
    /// Entity kind name.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Model(_) => "model",
            Self::Sprite(_) => "sprite",
            Self::Beam(_) => "beam",
            Self::RailCore(_) => "rail-core",
            Self::RailRings(_) => "rail-rings",
            Self::Lightning(_) => "lightning",
            Self::Portal(_) => "portal-surface",
        }
    }

    /// Origin for light placement (all but portal expose `origin`).
    #[must_use]
    pub fn origin(&self) -> Vec3 {
        match self {
            Self::Model(entity) => entity.origin,
            Self::Sprite(entity) => entity.origin,
            Self::Beam(entity) => entity.origin,
            Self::RailCore(entity) => entity.origin,
            Self::RailRings(entity) => entity.origin,
            Self::Lightning(entity) => entity.origin,
            Self::Portal(entity) => entity.origin,
        }
    }
}

/// Polygon vertex (`RefPolyVertex`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RefPolyVertex {
    /// Position.
    pub position: Vec3,
    /// Texture coordinate.
    pub tex_coord: Vec2,
    /// Source `polyVert_t.modulate` byte channels.
    pub color: Vec4,
}

/// Reference polygon (`RefPoly`).
#[derive(Debug, Clone, PartialEq)]
pub struct RefPoly {
    /// Shader.
    pub shader: Option<SceneShader>,
    /// Vertices.
    pub vertices: Vec<RefPolyVertex>,
}

/// Source model handle: typed model or numeric VM handle.
#[derive(Debug, Clone, PartialEq)]
pub enum ModelRef {
    /// Loaded model.
    Loaded(SceneModel),
    /// Numeric handle.
    Handle(i32),
}

/// Source shader handle: typed shader or numeric VM handle.
#[derive(Debug, Clone, PartialEq)]
pub enum ShaderRef {
    /// Loaded shader.
    Loaded(SceneShader),
    /// Numeric handle.
    Handle(i32),
}

/// Source skin handle: typed skin or numeric VM handle.
#[derive(Debug, Clone, PartialEq)]
pub enum SkinRef {
    /// Loaded skin.
    Loaded(SceneSkin),
    /// Numeric handle.
    Handle(i32),
}

/// Source model entity with numeric-capable handles (`SourceHandles`).
#[derive(Debug, Clone, PartialEq)]
pub struct SourceModelEntity {
    /// Render flags.
    pub render_flags: i32,
    /// Custom shader or handle.
    pub custom_shader: Option<ShaderRef>,
    /// Shader RGBA.
    pub shader_rgba: Vec4,
    /// Shader texture coordinate.
    pub shader_tex_coord: Vec2,
    /// Shader time.
    pub shader_time: f32,
    /// Model or handle.
    pub model: ModelRef,
    /// Origin.
    pub origin: Vec3,
    /// Old origin.
    pub old_origin: Vec3,
    /// Axis.
    pub axis: Axis,
    /// Non-normalized axes.
    pub non_normalized_axes: bool,
    /// Lighting origin.
    pub lighting_origin: Vec3,
    /// Shadow plane.
    pub shadow_plane: f32,
    /// Frame.
    pub frame: i32,
    /// Old frame.
    pub old_frame: i32,
    /// Back lerp.
    pub back_lerp: f32,
    /// Skin number.
    pub skin_num: i32,
    /// Custom skin or handle.
    pub custom_skin: Option<SkinRef>,
    /// Radius (poly records).
    pub radius: f32,
    /// Rotation (poly records).
    pub rotation: f32,
}

/// Source polygon record (`SourceRefEntityRecord` with kind `poly`).
#[derive(Debug, Clone, PartialEq)]
pub struct SourcePolyRecord {
    /// Fields.
    pub fields: SourceModelEntity,
}

/// Source reference entity (`SourceRefEntity`).
#[derive(Debug, Clone, PartialEq)]
pub enum SourceRefEntity {
    /// Model with handles.
    Model(SourceModelEntity),
    /// Sprite with handles.
    Sprite(SourceSpriteEntity),
    /// Beam with handles.
    Beam(SourceBeamEntity),
    /// Rail core with handles.
    RailCore(SourceRailEntity),
    /// Rail rings with handles.
    RailRings(SourceRailEntity),
    /// Lightning with handles.
    Lightning(SourceRailEntity),
    /// Portal (no handles).
    Portal(RefPortalEntity),
    /// Full poly record.
    Poly(SourcePolyRecord),
}

/// Source sprite entity with numeric-capable handles.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceSpriteEntity {
    /// Render flags.
    pub render_flags: i32,
    /// Custom shader or handle.
    pub custom_shader: Option<ShaderRef>,
    /// Shader RGBA.
    pub shader_rgba: Vec4,
    /// Shader texture coordinate.
    pub shader_tex_coord: Vec2,
    /// Shader time.
    pub shader_time: f32,
    /// Origin.
    pub origin: Vec3,
    /// Radius.
    pub radius: f32,
    /// Rotation.
    pub rotation: f32,
}

/// Source beam entity with numeric-capable handles.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceBeamEntity {
    /// Render flags.
    pub render_flags: i32,
    /// Custom shader or handle.
    pub custom_shader: Option<ShaderRef>,
    /// Shader RGBA.
    pub shader_rgba: Vec4,
    /// Shader texture coordinate.
    pub shader_tex_coord: Vec2,
    /// Shader time.
    pub shader_time: f32,
    /// Origin.
    pub origin: Vec3,
    /// Old origin.
    pub old_origin: Vec3,
    /// Axis.
    pub axis: Axis,
    /// Radius.
    pub radius: f32,
}

/// Source rail/lightning entity with numeric-capable handles.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceRailEntity {
    /// Render flags.
    pub render_flags: i32,
    /// Custom shader or handle.
    pub custom_shader: Option<ShaderRef>,
    /// Shader RGBA.
    pub shader_rgba: Vec4,
    /// Shader texture coordinate.
    pub shader_tex_coord: Vec2,
    /// Shader time.
    pub shader_time: f32,
    /// Origin.
    pub origin: Vec3,
    /// Old origin.
    pub old_origin: Vec3,
    /// Radius.
    pub radius: f32,
}

/// Admitted reference entity (`Q3AdmittedRefEntity`).
#[derive(Debug, Clone, PartialEq)]
pub enum Q3AdmittedRefEntity {
    /// Typed entity.
    Entity(RefEntity),
    /// Full poly record.
    Poly(SourcePolyRecord),
}

fn zero_vec3() -> Vec3 {
    vec3(0.0, 0.0, 0.0)
}

fn zero_axis() -> Axis {
    [zero_vec3(), zero_vec3(), zero_vec3()]
}

/// Create a model entity (`createModelEntity`).
#[must_use]
pub fn create_model_entity(model: SceneModel) -> RefModelEntity {
    RefModelEntity {
        shading: ShadedFields::default(),
        model,
        origin: zero_vec3(),
        old_origin: zero_vec3(),
        axis: zero_axis(),
        non_normalized_axes: false,
        lighting_origin: zero_vec3(),
        shadow_plane: 0.0,
        frame: 0,
        old_frame: 0,
        back_lerp: 0.0,
        skin_num: 0,
        custom_skin: None,
    }
}

/// Create a sprite entity (`createSpriteEntity`).
#[must_use]
pub fn create_sprite_entity() -> RefSpriteEntity {
    RefSpriteEntity {
        shading: ShadedFields::default(),
        origin: zero_vec3(),
        radius: 0.0,
        rotation: 0.0,
    }
}

/// Create a beam entity (`createBeamEntity`).
#[must_use]
pub fn create_beam_entity() -> RefBeamEntity {
    RefBeamEntity {
        shading: ShadedFields::default(),
        origin: zero_vec3(),
        old_origin: zero_vec3(),
        axis: zero_axis(),
        radius: 0.0,
    }
}

/// Create a rail-core entity (`createRailCoreEntity`).
#[must_use]
pub fn create_rail_core_entity() -> RefRailCoreEntity {
    RefRailCoreEntity {
        shading: ShadedFields::default(),
        origin: zero_vec3(),
        old_origin: zero_vec3(),
        radius: 0.0,
    }
}

/// Create a rail-rings entity (`createRailRingsEntity`).
#[must_use]
pub fn create_rail_rings_entity() -> RefRailRingsEntity {
    RefRailRingsEntity {
        shading: ShadedFields::default(),
        origin: zero_vec3(),
        old_origin: zero_vec3(),
        radius: 0.0,
    }
}

/// Create a lightning entity (`createLightningEntity`).
#[must_use]
pub fn create_lightning_entity() -> RefLightningEntity {
    RefLightningEntity {
        shading: ShadedFields::default(),
        origin: zero_vec3(),
        old_origin: zero_vec3(),
        radius: 0.0,
    }
}

/// Create a portal entity (`createPortalEntity`).
#[must_use]
pub fn create_portal_entity() -> RefPortalEntity {
    RefPortalEntity {
        render_flags: 0,
        origin: zero_vec3(),
        old_origin: zero_vec3(),
        axis: zero_axis(),
        frame: 0,
        old_frame: 0,
        skin_num: 0,
    }
}

/// Copy a reference entity (`copyRefEntity`; resource identity retained).
#[must_use]
pub fn copy_ref_entity(entity: &RefEntity) -> RefEntity {
    entity.clone()
}

/// Copy an admitted reference entity (`copyRefEntity` overload).
#[must_use]
pub fn copy_admitted_ref_entity(entity: &Q3AdmittedRefEntity) -> Q3AdmittedRefEntity {
    entity.clone()
}

/// Copy a source reference entity (`copySourceRefEntity`).
#[must_use]
pub fn copy_source_ref_entity(entity: &SourceRefEntity) -> SourceRefEntity {
    entity.clone()
}

/// Copy a reference polygon (`copyRefPoly`).
#[must_use]
pub fn copy_ref_poly(poly: &RefPoly) -> RefPoly {
    poly.clone()
}

// ---------------------------------------------------------------------------
// refdef.ts
// ---------------------------------------------------------------------------

/// Render text: eight rows (`RenderText`).
pub type RenderText = [String; 8];

/// Copy render text with source bounds checks (`copyRenderText`).
pub fn copy_render_text(source: &RenderText) -> PresentResult<RenderText> {
    for row in source {
        let bytes = row.as_bytes();
        if bytes.len() > 32 || (bytes.len() == 32 && !bytes.contains(&0)) {
            return Err(PresentError::range(
                "refdef render string requires a NUL within 32 bytes",
            ));
        }
        if !row.is_ascii() {
            return Err(PresentError::range("refdef render strings require byte characters"));
        }
    }
    Ok(source.clone())
}

/// No-world-model render flag.
pub const RDF_NOWORLDMODEL: i32 = 1;
/// Hyperspace render flag.
pub const RDF_HYPERSPACE: i32 = 4;

/// Reference definition (`Refdef`).
#[derive(Debug, Clone, PartialEq)]
pub struct Refdef {
    /// X.
    pub x: i32,
    /// Y.
    pub y: i32,
    /// Width.
    pub width: i32,
    /// Height.
    pub height: i32,
    /// Horizontal FOV.
    pub fov_x: f32,
    /// Vertical FOV.
    pub fov_y: f32,
    /// View origin.
    pub view_origin: Vec3,
    /// View axis.
    pub view_axis: Axis,
    /// Time.
    pub time: i32,
    /// Render flags.
    pub render_flags: i32,
    /// Area mask (32 bytes).
    pub area_mask: [u8; 32],
    /// Text.
    pub text: RenderText,
}

impl Default for Refdef {
    fn default() -> Self {
        Self {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
            fov_x: 0.0,
            fov_y: 0.0,
            view_origin: zero_vec3(),
            view_axis: zero_axis(),
            time: 0,
            render_flags: 0,
            area_mask: [0; 32],
            text: ["", "", "", "", "", "", "", ""].map(str::to_string),
        }
    }
}

/// Create a refdef (`createRefdef`).
#[must_use]
pub fn create_refdef() -> Refdef {
    Refdef::default()
}

/// Copy a refdef (`copyRefdef`).
#[must_use]
pub fn copy_refdef(source: &Refdef) -> Refdef {
    source.clone()
}

// ---------------------------------------------------------------------------
// model-access.ts
// ---------------------------------------------------------------------------

/// Model bounds (`modelBounds`).
#[must_use]
pub fn model_bounds(source: &SceneModel) -> Bounds {
    let zero = Bounds {
        min: zero_vec3(),
        max: zero_vec3(),
    };
    match source {
        SceneModel::Default(_) => zero,
        SceneModel::Inline(inline) => inline.bounds,
        SceneModel::Loaded(loaded) => match &loaded.model {
            Q3DecodedModel::Bounded { bounds } => *bounds,
            Q3DecodedModel::Brush { models, index } => models.get(*index).copied().unwrap_or(zero),
            Q3DecodedModel::Framed { frames } => frames.first().copied().unwrap_or(zero),
            Q3DecodedModel::Md3(_) | Q3DecodedModel::Md5(_) => zero,
        },
    }
}

/// Interpolated model tag (`lerpModelTag`).
pub fn lerp_model_tag(source: &SceneModel, name: &str, start: i32, end: i32, fraction: f32) -> Option<ModelTag> {
    let SceneModel::Loaded(loaded) = source else {
        return None;
    };
    match &loaded.model {
        Q3DecodedModel::Md3(model) => {
            let last = model.frames.len().saturating_sub(1);
            let first = model
                .tags
                .get(start.min(last as i32).max(0) as usize)?
                .iter()
                .find(|tag| tag.name == name)?;
            let second = model
                .tags
                .get(end.min(last as i32).max(0) as usize)?
                .iter()
                .find(|tag| tag.name == name)?;
            let tag = interpolate_md3_tags(first, second, name, fraction);
            Some(ModelTag {
                origin: tag.origin,
                axes: tag.axes,
            })
        }
        Q3DecodedModel::Md5(md5) => {
            let index = md5.joint_names.iter().position(|joint| joint == name)?;
            let pose = sample_md5_pose(&md5.frames, end, start, 1.0 - fraction)
                .get(index)
                .copied()?;
            let tag = joint_attachment_tag(name, &pose);
            Some(ModelTag {
                origin: tag.origin,
                axes: tag.axis,
            })
        }
        _ => None,
    }
}

/// Interpolated tag pose.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelTag {
    /// Origin.
    pub origin: Vec3,
    /// Axes.
    pub axes: Axis,
}

// ---------------------------------------------------------------------------
// character-resources.ts
// ---------------------------------------------------------------------------

/// Character media paths (`Q3_CHARACTER_SOUNDS`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q3CharacterSounds {
    /// Weapon change.
    pub select_sound: &'static str,
    /// Gib splash.
    pub gib_sound: &'static str,
    /// Teleport in.
    pub tele_in_sound: &'static str,
    /// Teleport out.
    pub tele_out_sound: &'static str,
    /// Respawn.
    pub respawn_sound: &'static str,
    /// Land.
    pub land_sound: &'static str,
    /// Water in.
    pub watr_in_sound: &'static str,
    /// Water out.
    pub watr_out_sound: &'static str,
    /// Water under.
    pub watr_un_sound: &'static str,
    /// Jump pad.
    pub jump_pad_sound: &'static str,
}

/// Character media paths.
pub const Q3_CHARACTER_SOUNDS: Q3CharacterSounds = Q3CharacterSounds {
    select_sound: "sound/weapons/change.wav",
    gib_sound: "sound/player/gibsplt1.wav",
    tele_in_sound: "sound/world/telein.wav",
    tele_out_sound: "sound/world/teleout.wav",
    respawn_sound: "sound/items/respawn1.wav",
    land_sound: "sound/player/land1.wav",
    watr_in_sound: "sound/player/watr_in.wav",
    watr_out_sound: "sound/player/watr_out.wav",
    watr_un_sound: "sound/player/watr_un.wav",
    jump_pad_sound: "sound/world/jumppad.wav",
};

/// Footstep kind (`keyof ClientMedia["footsteps"]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FootstepKind {
    /// Normal.
    Normal,
    /// Boot.
    Boot,
    /// Flesh.
    Flesh,
    /// Mech.
    Mech,
    /// Energy.
    Energy,
    /// Splash.
    Splash,
    /// Metal.
    Metal,
}

/// Footstep path table (`Q3_FOOTSTEP_PATHS`).
pub const Q3_FOOTSTEP_PATHS: [(FootstepKind, &str); 7] = [
    (FootstepKind::Normal, "step"),
    (FootstepKind::Boot, "boot"),
    (FootstepKind::Flesh, "flesh"),
    (FootstepKind::Mech, "mech"),
    (FootstepKind::Energy, "energy"),
    (FootstepKind::Splash, "splash"),
    (FootstepKind::Metal, "clank"),
];

/// Custom sound fallback model (`q3CustomSoundFallback` result).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CustomSoundFallback {
    /// Sarge.
    Sarge,
    /// James.
    James,
}

impl CustomSoundFallback {
    /// Model name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Sarge => "sarge",
            Self::James => "james",
        }
    }
}

/// Custom sound fallback (`q3CustomSoundFallback`).
#[must_use]
pub const fn q3_custom_sound_fallback(product: Product, team_game: bool) -> CustomSoundFallback {
    match (product, team_game) {
        (Product::MissionPack, true) => CustomSoundFallback::James,
        _ => CustomSoundFallback::Sarge,
    }
}

// ---------------------------------------------------------------------------
// scene.ts
// ---------------------------------------------------------------------------

/// Scene admission origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SceneAdmissionOrigin {
    /// Native.
    Native,
    /// Mixed.
    Mixed,
}

/// Scene admission identity (`SceneAdmissionIdentity`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Q3SceneAdmissionId {
    /// Origin.
    pub origin: SceneAdmissionOrigin,
    /// Unique token.
    pub token: u64,
}

static ADMISSION_TOKEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Scene admission snapshot (`Q3SceneAdmission`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3SceneAdmission {
    /// Identity.
    pub id: Q3SceneAdmissionId,
    /// Entities.
    pub entities: Vec<Q3AdmittedRefEntity>,
    /// Polygons.
    pub polygons: Vec<Q3AdmittedPoly>,
}

/// Snapshot a scene admission (`snapshotQ3SceneAdmission`).
#[must_use]
pub fn snapshot_q3_scene_admission(
    origin: SceneAdmissionOrigin,
    entities: Vec<Q3AdmittedRefEntity>,
    polygons: Vec<Q3AdmittedPoly>,
) -> Q3SceneAdmission {
    let token = ADMISSION_TOKEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    Q3SceneAdmission {
        id: Q3SceneAdmissionId { origin, token },
        entities,
        polygons,
    }
}

/// Fog selection (`Q3FogSelection`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q3FogSelection {
    /// Index.
    pub index: i32,
    /// Volume.
    pub volume: FogVolume,
}

/// Admitted polygon with fog (`Q3AdmittedPoly`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3AdmittedPoly {
    /// Polygon.
    pub poly: RefPoly,
    /// Fog.
    pub fog: Option<Q3FogSelection>,
}

/// Admit a polygon with fog selection (`admitQ3Poly`).
pub fn admit_q3_poly(poly: &RefPoly, fogs: &[Q3FogSelection]) -> PresentResult<Q3AdmittedPoly> {
    let copied = copy_ref_poly(poly);
    if fogs.is_empty() {
        return Ok(Q3AdmittedPoly {
            poly: copied,
            fog: None,
        });
    }
    let first = copied
        .vertices
        .first()
        .ok_or_else(|| PresentError::range("Source polygon fog requires its first admitted vertex"))?;
    let mut min = first.position;
    let mut max = first.position;
    for vertex in &copied.vertices {
        min.x = min.x.min(vertex.position.x);
        min.y = min.y.min(vertex.position.y);
        min.z = min.z.min(vertex.position.z);
        max.x = max.x.max(vertex.position.x);
        max.y = max.y.max(vertex.position.y);
        max.z = max.z.max(vertex.position.z);
    }
    let fog = fogs
        .iter()
        .find(|selection| {
            let bounds = selection.volume.bounds;
            max.x >= bounds.min.x
                && max.y >= bounds.min.y
                && max.z >= bounds.min.z
                && min.x <= bounds.max.x
                && min.y <= bounds.max.y
                && min.z <= bounds.max.z
        })
        .copied();
    Ok(Q3AdmittedPoly { poly: copied, fog })
}

/// Procedural fog selection (`q3ProceduralFog`).
#[must_use]
pub fn q3_procedural_fog(origin: Vec3, radius: f32, fogs: &[Q3FogSelection]) -> Option<Q3FogSelection> {
    fogs.iter()
        .find(|selection| {
            let bounds = selection.volume.bounds;
            origin.x - radius < bounds.max.x
                && origin.x + radius > bounds.min.x
                && origin.y - radius < bounds.max.y
                && origin.y + radius > bounds.min.y
                && origin.z - radius < bounds.max.z
                && origin.z + radius > bounds.min.z
        })
        .copied()
}

/// Geometry admission reference (`Q3GeometryAdmission`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3GeometryAdmission {
    /// Reference entity.
    RefEntity {
        /// Index.
        index: usize,
    },
    /// Polygon.
    Polygon {
        /// Index.
        index: usize,
    },
}

/// Presented special entity (beam or default model).
#[derive(Debug, Clone, PartialEq)]
pub struct PresentedSpecialEntity {
    /// Entity index.
    pub entity_index: usize,
    /// Source.
    pub source: SpecialEntitySource,
}

/// Special entity source.
#[derive(Debug, Clone, PartialEq)]
pub enum SpecialEntitySource {
    /// Beam.
    Beam(RefBeamEntity),
    /// Model.
    Model(RefModelEntity),
}

/// Presented portal.
#[derive(Debug, Clone, PartialEq)]
pub struct PresentedPortal {
    /// Entity index.
    pub entity_index: usize,
    /// Source.
    pub source: RefPortalEntity,
}

/// Presented model (`PresentedModel`).
#[derive(Debug, Clone, PartialEq)]
pub struct PresentedModel {
    /// Entity index.
    pub entity_index: usize,
    /// Scene entity.
    pub entity: PresentSceneEntity,
    /// Model source options.
    pub options: ModelSourceOptions,
    /// Source entity.
    pub source: RefModelEntity,
}

/// Prepare an authored model for the shared renderer (`prepareQ3Model`).
#[must_use]
pub fn prepare_q3_model(
    entity: &RefModelEntity,
    actor: Option<ActorId>,
    entity_index: usize,
) -> Option<PresentedModel> {
    let model = match &entity.model {
        SceneModel::Default(_) => return None,
        SceneModel::Inline(inline) => PresentEntityModel::BrushModel {
            world: inline.geometry.clone(),
            model: inline.index,
        },
        SceneModel::Loaded(loaded) => PresentEntityModel::Decoded(loaded.model.clone()),
    };
    let resource = match &entity.model {
        SceneModel::Loaded(loaded) => loaded.resource.clone(),
        SceneModel::Inline(inline) => inline.resource.clone(),
        SceneModel::Default(_) => return None,
    };
    Some(PresentedModel {
        entity_index,
        entity: PresentSceneEntity {
            actor,
            resource,
            model,
            origin: entity.origin,
            axis: entity.axis,
            previous_origin: entity.old_origin,
            frame: entity.frame,
            previous_frame: entity.old_frame,
            back_lerp: entity.back_lerp,
            skin: entity.skin_num,
            color: vec4(
                entity.shading.shader_rgba.x / 255.0,
                entity.shading.shader_rgba.y / 255.0,
                entity.shading.shader_rgba.z / 255.0,
                entity.shading.shader_rgba.w / 255.0,
            ),
            shader_time: entity.shading.shader_time,
            render_flags: entity.shading.render_flags,
            lighting_origin: entity.lighting_origin,
            shadow_plane: entity.shadow_plane,
        },
        source: entity.clone(),
        options: ModelSourceOptions {
            custom_shader: entity.shading.custom_shader.as_ref().map(|shader| shader.name.clone()),
            custom_skin: entity.custom_skin.as_ref().map(|skin| skin.surfaces.clone()),
            non_normalized_axes: entity.non_normalized_axes,
        },
    })
}

/// Presented effect geometry (`PresentedGeometry`).
#[derive(Debug, Clone, PartialEq)]
pub struct PresentedGeometry {
    /// Admission.
    pub admission: Q3GeometryAdmission,
    /// Shader.
    pub shader: Option<SceneShader>,
    /// Source.
    pub source: PresentedGeometrySource,
}

/// Effect geometry source.
#[derive(Debug, Clone, PartialEq)]
pub enum PresentedGeometrySource {
    /// Sprite.
    Sprite(RefSpriteEntity),
    /// Rail core.
    RailCore(RefRailCoreEntity),
    /// Rail rings.
    RailRings(RefRailRingsEntity),
    /// Lightning.
    Lightning(RefLightningEntity),
    /// Polygon.
    Poly(Q3AdmittedPoly),
}

/// Scene content (`Q3SceneContent`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3SceneContent {
    /// Admission.
    pub admission: Q3SceneAdmission,
    /// Models.
    pub models: Vec<PresentedModel>,
    /// Effects.
    pub effects: Vec<PresentedGeometry>,
    /// Special entities.
    pub special_entities: Vec<PresentedSpecialEntity>,
    /// Portals.
    pub portals: Vec<PresentedPortal>,
    /// Lights.
    pub lights: Vec<PresentSceneLight>,
}

/// Presented scene (`Q3PresentedScene`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3PresentedScene {
    /// Content.
    pub content: Q3SceneContent,
    /// Seat.
    pub seat: SeatId,
    /// Viewport.
    pub viewport: Rect,
    /// Camera.
    pub camera: SceneCamera,
    /// Source refdef.
    pub source: Refdef,
}

/// Scene target (`Q3SceneTarget`).
pub trait Q3SceneTarget {
    /// Seat.
    fn seat(&self) -> SeatId;
    /// Viewport.
    fn viewport(&self) -> Rect;
    /// Far clip.
    fn far_clip(&self) -> f32;
    /// Near clip.
    fn near_clip(&self) -> f32;
    /// Rail settings.
    fn rail(&self) -> RailSettings;
    /// Fog selections.
    fn fog_selections(&self) -> Vec<Q3FogSelection>;
    /// Print.
    fn print(&mut self, text: &str);
    /// Actor for an entity.
    fn actor(&self, entity: &RefModelEntity) -> Option<ActorId>;
    /// Publish a scene.
    fn publish(&mut self, scene: Q3PresentedScene);
}

/// Per-seat scene recorder (`Q3SceneRecorder`).
pub struct Q3SceneRecorder {
    /// Target.
    pub target: Box<dyn Q3SceneTarget>,
    /// Entities.
    entities: Vec<Q3AdmittedRefEntity>,
    /// Polygons.
    polygons: Vec<Q3AdmittedPoly>,
    /// Lights.
    lights: Vec<PresentSceneLight>,
}

impl Q3SceneRecorder {
    /// New recorder.
    pub fn new(target: Box<dyn Q3SceneTarget>) -> Self {
        Self {
            target,
            entities: Vec::new(),
            polygons: Vec::new(),
            lights: Vec::new(),
        }
    }

    /// Clear the scene.
    pub fn clear_scene(&mut self) {
        self.entities.clear();
        self.polygons.clear();
        self.lights.clear();
    }

    /// Add a reference entity.
    pub fn add_ref_entity(&mut self, entity: &Q3AdmittedRefEntity) {
        self.entities.push(copy_admitted_ref_entity(entity));
    }

    /// Add a polygon.
    pub fn add_poly(&mut self, poly: &RefPoly) -> PresentResult<()> {
        if poly.shader.is_none() {
            self.target.print("^3WARNING: RE_AddPolyToScene: NULL poly shader\n");
            return Ok(());
        }
        let fogs = self.target.fog_selections();
        self.polygons.push(admit_q3_poly(poly, &fogs)?);
        Ok(())
    }

    /// Add a light.
    pub fn add_light(&mut self, light: &DynamicLight) {
        self.lights.push(PresentSceneLight {
            origin: light.origin,
            radius: light.radius,
            color: light.color,
            additive: light.additive,
        });
    }

    /// Capture scene content.
    pub fn capture(&self) -> Q3SceneContent {
        let admission = snapshot_q3_scene_admission(
            SceneAdmissionOrigin::Native,
            self.entities.clone(),
            self.polygons.clone(),
        );
        let mut models = Vec::new();
        let mut effects = Vec::new();
        let mut portals = Vec::new();
        let mut special_entities = Vec::new();
        for (entity_index, entity) in admission.entities.iter().enumerate() {
            match entity {
                Q3AdmittedRefEntity::Poly(_) => continue,
                Q3AdmittedRefEntity::Entity(RefEntity::Portal(source)) => {
                    portals.push(PresentedPortal {
                        entity_index,
                        source: source.clone(),
                    });
                }
                Q3AdmittedRefEntity::Entity(RefEntity::Beam(source)) => {
                    special_entities.push(PresentedSpecialEntity {
                        entity_index,
                        source: SpecialEntitySource::Beam(source.clone()),
                    });
                }
                Q3AdmittedRefEntity::Entity(RefEntity::Model(source)) => {
                    if source.model.is_default() {
                        special_entities.push(PresentedSpecialEntity {
                            entity_index,
                            source: SpecialEntitySource::Model(source.clone()),
                        });
                        continue;
                    }
                    if let Some(prepared) = prepare_q3_model(source, self.target.actor(source), entity_index) {
                        models.push(prepared);
                    }
                }
                Q3AdmittedRefEntity::Entity(RefEntity::Sprite(source)) => {
                    effects.push(PresentedGeometry {
                        admission: Q3GeometryAdmission::RefEntity { index: entity_index },
                        shader: source.shading.custom_shader.clone(),
                        source: PresentedGeometrySource::Sprite(source.clone()),
                    });
                }
                Q3AdmittedRefEntity::Entity(RefEntity::RailCore(source)) => {
                    effects.push(PresentedGeometry {
                        admission: Q3GeometryAdmission::RefEntity { index: entity_index },
                        shader: source.shading.custom_shader.clone(),
                        source: PresentedGeometrySource::RailCore(source.clone()),
                    });
                }
                Q3AdmittedRefEntity::Entity(RefEntity::RailRings(source)) => {
                    effects.push(PresentedGeometry {
                        admission: Q3GeometryAdmission::RefEntity { index: entity_index },
                        shader: source.shading.custom_shader.clone(),
                        source: PresentedGeometrySource::RailRings(source.clone()),
                    });
                }
                Q3AdmittedRefEntity::Entity(RefEntity::Lightning(source)) => {
                    effects.push(PresentedGeometry {
                        admission: Q3GeometryAdmission::RefEntity { index: entity_index },
                        shader: source.shading.custom_shader.clone(),
                        source: PresentedGeometrySource::Lightning(source.clone()),
                    });
                }
            }
        }
        for (index, poly) in admission.polygons.iter().enumerate() {
            effects.push(PresentedGeometry {
                admission: Q3GeometryAdmission::Polygon { index },
                shader: poly.poly.shader.clone(),
                source: PresentedGeometrySource::Poly(poly.clone()),
            });
        }
        Q3SceneContent {
            admission,
            models,
            effects,
            portals,
            special_entities,
            lights: self.lights.clone(),
        }
    }

    /// Render a scene.
    pub fn render_scene(&mut self, input: &Refdef) {
        let source = copy_refdef(input);
        let content = self.capture();
        let viewport = Rect {
            x: self.target.viewport().x + source.x,
            y: self.target.viewport().y + source.y,
            width: source.width,
            height: source.height,
        };
        let camera = SceneCamera {
            viewport,
            origin: source.view_origin,
            axis: source.view_axis,
            projection: perspective_projection(
                source.fov_x,
                source.fov_y,
                self.target.far_clip(),
                self.target.near_clip(),
            ),
        };
        self.target.publish(Q3PresentedScene {
            content,
            seat: self.target.seat(),
            viewport,
            camera,
            source,
        });
    }
}

// ---------------------------------------------------------------------------
// mark-projector.ts
// ---------------------------------------------------------------------------

/// Mark surface (`MarkSurface`).
#[derive(Debug, Clone, PartialEq)]
pub enum MarkSurface {
    /// Skipped.
    Skip,
    /// Planar face.
    Face {
        /// Surface flags.
        surface_flags: i32,
        /// Content flags.
        content_flags: i32,
        /// Plane.
        plane: Plane,
        /// Vertices.
        vertices: Vec<MarkVertex>,
        /// Indices.
        indices: Vec<usize>,
    },
    /// Patch grid.
    Grid {
        /// Surface flags.
        surface_flags: i32,
        /// Content flags.
        content_flags: i32,
        /// Mesh.
        mesh: PresentPatchMesh,
    },
}

/// Mark vertex (`MaterialVertex`, minimal mirror: position only).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MarkVertex {
    /// Position.
    pub position: Vec3,
}

/// Patch mesh (`PatchMesh`, minimal mirror).
#[derive(Debug, Clone, PartialEq)]
pub struct PresentPatchMesh {
    /// Width.
    pub width: usize,
    /// Height.
    pub height: usize,
    /// Row-major vertices.
    pub vertices: Vec<PatchVertex>,
}

/// Patch vertex.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PatchVertex {
    /// Position.
    pub position: Vec3,
    /// Normal.
    pub normal: Vec3,
}

/// BSP child reference (`BspNode.children` entry).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarkChild {
    /// Whether the child is a node (`true`) or leaf (`false`).
    pub is_node: bool,
    /// Index.
    pub index: usize,
}

/// BSP node (`BspNode`, minimal mirror).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarkNode {
    /// Plane index.
    pub plane: usize,
    /// Children.
    pub children: [MarkChild; 2],
}

/// BSP leaf surface range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarkLeaf {
    /// First surface.
    pub first_surface: usize,
    /// Surface count.
    pub surface_count: usize,
}

/// Mark geometry map (`MarkGeometry.map`).
#[derive(Debug, Clone, PartialEq)]
pub struct MarkMap {
    /// Nodes.
    pub nodes: Vec<MarkNode>,
    /// Planes.
    pub planes: Vec<Plane>,
    /// Leaves.
    pub leaves: Vec<MarkLeaf>,
    /// Leaf surfaces.
    pub leaf_surfaces: Vec<usize>,
    /// Surface count.
    pub surface_count: usize,
}

/// Mark geometry (`MarkGeometry`).
#[derive(Debug, Clone, PartialEq)]
pub struct MarkGeometry {
    /// Map.
    pub map: MarkMap,
    /// Surfaces.
    pub surfaces: Vec<MarkSurface>,
}

/// Mark fragment (`MarkFragment`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarkFragment {
    /// First point.
    pub first_point: usize,
    /// Point count.
    pub point_count: usize,
}

/// Mark fragments (`MarkFragments`).
#[derive(Debug, Clone, PartialEq)]
pub struct MarkFragments {
    /// Points.
    pub points: Vec<Vec3>,
    /// Fragments.
    pub fragments: Vec<MarkFragment>,
}

/// Mark projection query (`MarkProjection`).
#[derive(Debug, Clone, PartialEq)]
pub struct MarkProjection {
    /// Points.
    pub points: Vec<Vec3>,
    /// Projection.
    pub projection: Vec3,
    /// Maximum points.
    pub max_points: usize,
    /// Maximum fragments.
    pub max_fragments: usize,
}

/// Borrowed mark projection with output stores (`SourceMarkProjection`).
pub trait SourceMarkProjection {
    /// Input point count.
    fn point_count(&self) -> usize;
    /// Maximum points.
    fn max_points(&self) -> usize;
    /// Maximum fragments.
    fn max_fragments(&self) -> usize;
    /// Read an input point.
    fn read_point(&self, index: usize) -> PresentResult<Vec3>;
    /// Read the projection vector.
    fn read_projection(&self) -> Vec3;
    /// Write a fragment.
    fn write_fragment(&mut self, index: usize, fragment: MarkFragment);
    /// Write points.
    fn write_points(&mut self, first_point: usize, points: &[Vec3]);
}

/// Maximum clip vertices.
const MAX_CLIP_VERTICES: usize = 64;
/// Maximum mark surfaces per query.
const MAX_MARK_SURFACES: usize = 64;
/// Clip epsilon.
const CLIP_EPSILON: f32 = 0.5;

fn mark_at<T>(values: &[T], index: usize) -> PresentResult<&T> {
    values
        .get(index)
        .ok_or_else(|| PresentError::range(format!("mark geometry index {index} outside {}", values.len())))
}

fn finite_vector(value: Vec3) -> PresentResult<Vec3> {
    if !(value.x.is_finite() && value.y.is_finite() && value.z.is_finite()) {
        return Err(PresentError::range("mark coordinates must be finite float32 values"));
    }
    Ok(value)
}

fn check_capacity(value: usize) -> PresentResult<()> {
    if value > 1_000_000 {
        return Err(PresentError::range(
            "mark buffer capacity must be an integer in [0, 1000000]",
        ));
    }
    Ok(())
}

fn box_side(bounds: &Bounds, plane: &Plane) -> u32 {
    let axis = if plane.normal.x == 1.0 {
        Some('x')
    } else if plane.normal.y == 1.0 {
        Some('y')
    } else if plane.normal.z == 1.0 {
        Some('z')
    } else {
        None
    };
    if let Some(axis) = axis {
        let (min, max) = match axis {
            'x' => (bounds.min.x, bounds.max.x),
            'y' => (bounds.min.y, bounds.max.y),
            _ => (bounds.min.z, bounds.max.z),
        };
        // q_math.c axial fast path treats max == plane distance as behind.
        if plane.distance <= min {
            return 1;
        }
        if plane.distance >= max {
            return 2;
        }
        return 3;
    }
    box_on_plane_side(*bounds, *plane)
}

/// Chop a polygon behind a plane (`R_ChopPolyBehindPlane`).
fn chop(points: &[Vec3], plane: &Plane) -> Vec<Vec3> {
    if points.len() >= MAX_CLIP_VERTICES - 2 {
        return Vec::new();
    }
    let distances: Vec<f32> = points
        .iter()
        .map(|point| dot3(*point, plane.normal) - plane.distance)
        .collect();
    let sides: Vec<u8> = distances
        .iter()
        .map(|distance| {
            if *distance > CLIP_EPSILON {
                0
            } else if *distance < -CLIP_EPSILON {
                1
            } else {
                2
            }
        })
        .collect();
    if !sides.contains(&0) {
        return Vec::new();
    }
    if !sides.contains(&1) {
        return points.to_vec();
    }
    let mut result = Vec::new();
    for (index, first) in points.iter().enumerate() {
        let side = sides[index];
        let next = (index + 1) % points.len();
        if side == 2 {
            result.push(*first);
            continue;
        }
        if side == 0 {
            result.push(*first);
        }
        let next_side = sides[next];
        if next_side == 2 || next_side == side {
            continue;
        }
        let difference = distances[index] - distances[next];
        let fraction = if difference == 0.0 {
            0.0
        } else {
            distances[index] / difference
        };
        result.push(add3(*first, scale3(sub3(points[next], *first), fraction)));
    }
    result
}

/// BSP mark projector (`BspMarkProjector`).
#[derive(Debug, Clone, PartialEq)]
pub struct BspMarkProjector {
    /// Geometry.
    pub geometry: MarkGeometry,
}

impl BspMarkProjector {
    /// New projector.
    pub fn new(geometry: MarkGeometry) -> PresentResult<Self> {
        if geometry.surfaces.len() != geometry.map.surface_count {
            return Err(PresentError::range("mark geometry must retain every BSP surface index"));
        }
        Ok(Self { geometry })
    }

    /// Project mark fragments (`markFragments`).
    pub fn mark_fragments(&self, query: &MarkProjection) -> PresentResult<MarkFragments> {
        struct Owned {
            query: MarkProjection,
            points: Vec<Vec3>,
            fragments: Vec<MarkFragment>,
        }
        impl SourceMarkProjection for Owned {
            fn point_count(&self) -> usize {
                self.query.points.len()
            }
            fn max_points(&self) -> usize {
                self.query.max_points
            }
            fn max_fragments(&self) -> usize {
                self.query.max_fragments
            }
            fn read_point(&self, index: usize) -> PresentResult<Vec3> {
                mark_at(&self.query.points, index).copied()
            }
            fn read_projection(&self) -> Vec3 {
                self.query.projection
            }
            fn write_fragment(&mut self, _index: usize, fragment: MarkFragment) {
                self.fragments.push(fragment);
            }
            fn write_points(&mut self, _first_point: usize, points: &[Vec3]) {
                self.points.extend_from_slice(points);
            }
        }
        let mut owned = Owned {
            query: query.clone(),
            points: Vec::new(),
            fragments: Vec::new(),
        };
        self.mark_fragments_record(&mut owned)?;
        Ok(MarkFragments {
            points: owned.points,
            fragments: owned.fragments,
        })
    }

    /// Project into borrowed output stores (`markFragmentsRecord`).
    pub fn mark_fragments_record(&self, query: &mut dyn SourceMarkProjection) -> PresentResult<usize> {
        check_capacity(query.max_points())?;
        check_capacity(query.max_fragments())?;
        if query.point_count() < 1 {
            return Err(PresentError::range("mark projection requires at least one input point"));
        }
        let projection = finite_vector(query.read_projection())?;
        let direction = normalize3_or_zero(projection);
        let mut points = Vec::new();
        let mut minimum = vec3(99999.0, 99999.0, 99999.0);
        let mut maximum = vec3(-99999.0, -99999.0, -99999.0);
        for index in 0..query.point_count() {
            let point = finite_vector(query.read_point(index)?)?;
            if index < MAX_CLIP_VERTICES {
                points.push(point);
            }
            for bound_point in [point, add3(point, projection), add3(point, scale3(direction, -20.0))] {
                minimum.x = minimum.x.min(bound_point.x);
                minimum.y = minimum.y.min(bound_point.y);
                minimum.z = minimum.z.min(bound_point.z);
                maximum.x = maximum.x.max(bound_point.x);
                maximum.y = maximum.y.max(bound_point.y);
                maximum.z = maximum.z.max(bound_point.z);
            }
        }
        if query.max_fragments() == 0 || query.max_points() == 0 {
            return Ok(0);
        }
        let count = points.len();
        let mut planes = Vec::new();
        for index in 0..count {
            let point = *mark_at(&points, index)?;
            let edge = sub3(*mark_at(&points, (index + 1) % count)?, point);
            let reverse = sub3(point, add3(point, projection));
            let normal = normalize_fast3(cross3(edge, reverse));
            planes.push(Plane {
                normal,
                distance: dot3(normal, point),
            });
        }
        let first = *mark_at(&points, 0)?;
        let inverse = scale3(direction, -1.0);
        planes.push(Plane {
            normal: direction,
            distance: dot3(direction, first) - 32.0,
        });
        planes.push(Plane {
            normal: inverse,
            distance: dot3(inverse, first) - 20.0,
        });
        let surfaces = self.box_surfaces(
            &Bounds {
                min: minimum,
                max: maximum,
            },
            direction,
        )?;
        let mut returned_points = 0usize;
        let mut returned_fragments = 0usize;
        let max_points = query.max_points();
        let max_fragments = query.max_fragments();
        let append = |triangle: [Vec3; 3],
                        query: &mut dyn SourceMarkProjection,
                        returned_points: &mut usize,
                        returned_fragments: &mut usize| {
            let mut clipped = triangle.to_vec();
            for plane in &planes {
                clipped = chop(&clipped, plane);
                if clipped.is_empty() {
                    return;
                }
            }
            if clipped.len() + *returned_points > max_points {
                return;
            }
            query.write_fragment(
                *returned_fragments,
                MarkFragment {
                    first_point: *returned_points,
                    point_count: clipped.len(),
                },
            );
            query.write_points(*returned_points, &clipped);
            *returned_points += clipped.len();
            *returned_fragments += 1;
        };
        for surface in &surfaces {
            match surface {
                MarkSurface::Skip => {}
                MarkSurface::Face {
                    plane,
                    vertices,
                    indices,
                    ..
                } => {
                    if dot3(plane.normal, direction) > -0.5 {
                        continue;
                    }
                    let mut index = 0;
                    while index < indices.len() {
                        let triangle = [
                            add3(
                                mark_at(vertices, *mark_at(indices, index)?)?.position,
                                scale3(plane.normal, 0.0),
                            ),
                            add3(
                                mark_at(vertices, *mark_at(indices, index + 1)?)?.position,
                                scale3(plane.normal, 0.0),
                            ),
                            add3(
                                mark_at(vertices, *mark_at(indices, index + 2)?)?.position,
                                scale3(plane.normal, 0.0),
                            ),
                        ];
                        append(triangle, &mut *query, &mut returned_points, &mut returned_fragments);
                        if returned_fragments == max_fragments {
                            return Ok(returned_fragments);
                        }
                        index += 3;
                    }
                }
                MarkSurface::Grid { mesh, .. } => {
                    for row in 0..mesh.height.saturating_sub(1) {
                        for column in 0..mesh.width.saturating_sub(1) {
                            let base = row * mesh.width + column;
                            for (indexes, threshold) in [
                                ([base, base + mesh.width, base + 1], -0.1f32),
                                ([base + 1, base + mesh.width, base + mesh.width + 1], -0.05f32),
                            ] {
                                let triangle: Vec<Vec3> = indexes
                                    .iter()
                                    .map(|index| {
                                        mark_at(&mesh.vertices, *index)
                                            .map(|vertex| add3(vertex.position, scale3(vertex.normal, 0.0)))
                                    })
                                    .collect::<PresentResult<_>>()?;
                                let normal = normalize_fast3(cross3(
                                    sub3(triangle[0], triangle[1]),
                                    sub3(triangle[2], triangle[1]),
                                ));
                                if dot3(normal, direction) >= threshold {
                                    continue;
                                }
                                append(
                                    [triangle[0], triangle[1], triangle[2]],
                                    &mut *query,
                                    &mut returned_points,
                                    &mut returned_fragments,
                                );
                                if returned_fragments == max_fragments {
                                    return Ok(returned_fragments);
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok(returned_fragments)
    }

    /// Surfaces overlapping a bounds box (`boxSurfaces`).
    fn box_surfaces(&self, bounds: &Bounds, direction: Vec3) -> PresentResult<Vec<MarkSurface>> {
        let map = &self.geometry.map;
        let mut visited = HashSet::new();
        let mut result = Vec::new();
        let mut stack: Vec<i64> = if map.nodes.is_empty() {
            if map.leaves.is_empty() {
                Vec::new()
            } else {
                vec![-1]
            }
        } else {
            vec![0]
        };
        while let Some(index) = stack.pop() {
            if index >= 0 {
                let node = mark_at(&map.nodes, index as usize)?;
                let side = box_side(bounds, mark_at(&map.planes, node.plane)?);
                if side & 2 != 0 {
                    let child = node.children[1];
                    stack.push(if child.is_node {
                        child.index as i64
                    } else {
                        -1 - child.index as i64
                    });
                }
                if side & 1 != 0 {
                    let child = node.children[0];
                    stack.push(if child.is_node {
                        child.index as i64
                    } else {
                        -1 - child.index as i64
                    });
                }
                continue;
            }
            let leaf = mark_at(&map.leaves, (-index - 1) as usize)?;
            for offset in 0..leaf.surface_count {
                if result.len() >= MAX_MARK_SURFACES {
                    break;
                }
                let surface_index = *mark_at(&map.leaf_surfaces, leaf.first_surface + offset)?;
                if !visited.insert(surface_index) {
                    continue;
                }
                let surface = mark_at(&self.geometry.surfaces, surface_index)?;
                match surface {
                    MarkSurface::Skip => continue,
                    MarkSurface::Face {
                        surface_flags,
                        content_flags,
                        ..
                    }
                    | MarkSurface::Grid {
                        surface_flags,
                        content_flags,
                        ..
                    } => {
                        if surface_flags & 0x30 != 0 || content_flags & 64 != 0 {
                            continue;
                        }
                    }
                }
                if let MarkSurface::Face { plane, .. } = surface {
                    if box_side(bounds, plane) != 3 || dot3(plane.normal, direction) > -0.5 {
                        continue;
                    }
                }
                result.push(surface.clone());
            }
        }
        Ok(result)
    }
}

/// World surface for mark projection (`worldMarkProjector` input).
#[derive(Debug, Clone, PartialEq)]
pub enum PresentMarkWorldSurface {
    /// Quake III material surface.
    Q3 {
        /// Surface flags.
        surface_flags: i32,
        /// Content flags.
        content_flags: i32,
        /// Prepared grid, when present.
        grid: Option<PresentPatchMesh>,
        /// Whether the source surface is planar.
        planar: bool,
        /// Plane, when present.
        plane: Option<Plane>,
        /// Vertices.
        vertices: Vec<MarkVertex>,
        /// Indices.
        indices: Vec<usize>,
    },
    /// Shared foreign surface.
    Shared {
        /// Plane, when present.
        plane: Option<Plane>,
        /// Vertices.
        vertices: Vec<MarkVertex>,
        /// Indices.
        indices: Vec<usize>,
        /// Excluded by material rules.
        excluded: bool,
    },
}

/// World mark geometry input.
#[derive(Debug, Clone, PartialEq)]
pub struct PresentMarkWorld {
    /// Nodes.
    pub nodes: Vec<MarkNode>,
    /// Planes.
    pub planes: Vec<Plane>,
    /// Leaves.
    pub leaves: Vec<MarkLeaf>,
    /// Leaf surfaces.
    pub leaf_surfaces: Vec<usize>,
    /// Surfaces.
    pub surfaces: Vec<PresentMarkWorldSurface>,
}

/// Build a projector from prepared world geometry (`worldMarkProjector`).
pub fn world_mark_projector(world: &PresentMarkWorld) -> PresentResult<BspMarkProjector> {
    let mut surfaces = Vec::with_capacity(world.surfaces.len());
    for surface in &world.surfaces {
        match surface {
            PresentMarkWorldSurface::Q3 {
                surface_flags,
                content_flags,
                grid,
                planar,
                plane,
                vertices,
                indices,
            } => {
                if let Some(mesh) = grid {
                    surfaces.push(MarkSurface::Grid {
                        surface_flags: *surface_flags,
                        content_flags: *content_flags,
                        mesh: mesh.clone(),
                    });
                } else if !planar || plane.is_none() {
                    surfaces.push(MarkSurface::Skip);
                } else {
                    surfaces.push(MarkSurface::Face {
                        surface_flags: *surface_flags,
                        content_flags: *content_flags,
                        plane: plane.unwrap_or(Plane {
                            normal: zero_vec3(),
                            distance: 0.0,
                        }),
                        vertices: vertices.clone(),
                        indices: indices.clone(),
                    });
                }
            }
            PresentMarkWorldSurface::Shared {
                plane,
                vertices,
                indices,
                excluded,
            } => {
                if plane.is_none() || *excluded {
                    surfaces.push(MarkSurface::Skip);
                } else {
                    surfaces.push(MarkSurface::Face {
                        surface_flags: 0,
                        content_flags: 0,
                        plane: plane.unwrap_or(Plane {
                            normal: zero_vec3(),
                            distance: 0.0,
                        }),
                        vertices: vertices.clone(),
                        indices: indices.clone(),
                    });
                }
            }
        }
    }
    let surface_count = surfaces.len();
    BspMarkProjector::new(MarkGeometry {
        map: MarkMap {
            nodes: world.nodes.clone(),
            planes: world.planes.clone(),
            leaves: world.leaves.clone(),
            leaf_surfaces: world.leaf_surfaces.clone(),
            surface_count,
        },
        surfaces,
    })
}

// ---------------------------------------------------------------------------
// marks.ts
// ---------------------------------------------------------------------------

/// Maximum mark polygons.
pub const MAX_MARK_POLYS: usize = 256;
/// Maximum vertices per mark polygon.
pub const MAX_MARK_POLY_VERTICES: usize = 10;
/// Maximum mark fragments.
pub const MAX_MARK_FRAGMENTS: usize = 128;
/// Maximum mark points.
pub const MAX_MARK_POINTS: usize = 384;
/// Mark total lifetime milliseconds.
pub const MARK_TOTAL_TIME: i32 = 10000;
/// Mark fade time milliseconds.
pub const MARK_FADE_TIME: i32 = 1000;

/// Impact mark request (`ImpactMarkRequest`).
#[derive(Debug, Clone, PartialEq)]
pub struct ImpactMarkRequest {
    /// Shader.
    pub shader: Option<SceneShader>,
    /// Origin.
    pub origin: Vec3,
    /// Direction.
    pub direction: Vec3,
    /// Orientation degrees.
    pub orientation: f32,
    /// Unit color.
    pub color: Vec4,
    /// Alpha fade.
    pub alpha_fade: bool,
    /// Radius.
    pub radius: f32,
    /// Temporary (returned, not stored).
    pub temporary: bool,
}

/// Impact mark options (`ImpactMarkOptions`).
pub trait ImpactMarkOptions {
    /// Clock milliseconds (signed 32-bit).
    fn clock(&self) -> i32;
    /// Marks enabled.
    fn enabled(&self) -> bool;
    /// Energy shader.
    fn energy_shader(&self) -> Option<SceneShader>;
}

/// Stored mark.
#[derive(Debug, Clone, PartialEq)]
struct StoredMark {
    time: i32,
    shader: Option<SceneShader>,
    alpha_fade: bool,
    color: Vec4,
    vertices: Vec<RefPolyVertex>,
}

fn mark_finite(value: f32, name: &str) -> PresentResult<f32> {
    if !value.is_finite() {
        return Err(PresentError::range(format!("{name} must be finite float32")));
    }
    Ok(value)
}

fn mark_position(value: Vec3) -> PresentResult<Vec3> {
    Ok(vec3(
        mark_finite(value.x, "mark coordinate")?,
        mark_finite(value.y, "mark coordinate")?,
        mark_finite(value.z, "mark coordinate")?,
    ))
}

fn mark_byte(value: f32) -> f32 {
    ((value.trunc() as i32) & 255) as f32
}

fn mark_poly(shader: Option<SceneShader>, vertices: &[RefPolyVertex]) -> RefPoly {
    RefPoly {
        shader,
        vertices: vertices.to_vec(),
    }
}

fn mark_unit_color(value: f32) -> PresentResult<f32> {
    let result = mark_finite(value, "mark color")?;
    if !(0.0..=1.0).contains(&result) {
        return Err(PresentError::range("mark colors must be in [0, 1]"));
    }
    Ok(result)
}

/// Impact mark system (`ImpactMarkSystem`).
pub struct ImpactMarkSystem {
    /// Projector.
    projector: BspMarkProjector,
    /// Options.
    options: Box<dyn ImpactMarkOptions>,
    /// Active marks, newest first.
    active: Vec<StoredMark>,
}

impl ImpactMarkSystem {
    /// New system.
    pub fn new(projector: BspMarkProjector, options: Box<dyn ImpactMarkOptions>) -> Self {
        Self {
            projector,
            options,
            active: Vec::new(),
        }
    }

    /// Reset.
    pub fn reset(&mut self) {
        self.active.clear();
    }

    /// Active mark count.
    #[must_use]
    pub fn active_mark_count(&self) -> usize {
        self.active.len()
    }

    /// Add an impact mark (`impactMark`); returns temporary polys.
    pub fn impact_mark(&mut self, request: &ImpactMarkRequest) -> PresentResult<Vec<RefPoly>> {
        if !self.options.enabled() {
            return Ok(Vec::new());
        }
        let radius = mark_finite(request.radius, "mark radius")?;
        if radius <= 0.0 {
            return Err(PresentError::range("CG_ImpactMark called with <= 0 radius"));
        }
        let origin = mark_position(request.origin)?;
        let direction = mark_position(request.direction)?;
        if dot3(direction, direction) == 0.0 {
            return Ok(Vec::new());
        }
        let normal = normalize3_or_zero(direction);
        let axis2 = rotate_point_around_vector(
            normal,
            perpendicular_vector(normal),
            f64::from(mark_finite(request.orientation, "mark orientation")?),
        );
        let axis1 = cross3(normal, axis2);
        let scale = 0.5 / radius;
        let point = |first: f32, second: f32| {
            vec3(
                origin.x + first * (radius * axis1.x) + second * (radius * axis2.x),
                origin.y + first * (radius * axis1.y) + second * (radius * axis2.y),
                origin.z + first * (radius * axis1.z) + second * (radius * axis2.z),
            )
        };
        let fragments = self.projector.mark_fragments(&MarkProjection {
            points: vec![point(-1.0, -1.0), point(1.0, -1.0), point(1.0, 1.0), point(-1.0, 1.0)],
            projection: scale3(direction, -20.0),
            max_points: MAX_MARK_POINTS,
            max_fragments: MAX_MARK_FRAGMENTS,
        })?;
        let color = vec4(
            mark_unit_color(request.color.x)?,
            mark_unit_color(request.color.y)?,
            mark_unit_color(request.color.z)?,
            mark_unit_color(request.color.w)?,
        );
        let modulate = vec4(
            mark_byte(color.x * 255.0),
            mark_byte(color.y * 255.0),
            mark_byte(color.z * 255.0),
            mark_byte(color.w * 255.0),
        );
        let mut temporary = Vec::new();
        for fragment in &fragments.fragments {
            let end = fragment.first_point + fragment.point_count.min(MAX_MARK_POLY_VERTICES);
            let vertices: Vec<RefPolyVertex> = fragments.points[fragment.first_point..end.min(fragments.points.len())]
                .iter()
                .map(|point| {
                    let delta = sub3(*point, origin);
                    RefPolyVertex {
                        position: *point,
                        tex_coord: vec2(0.5 + dot3(delta, axis1) * scale, 0.5 + dot3(delta, axis2) * scale),
                        color: modulate,
                    }
                })
                .collect();
            if request.temporary {
                temporary.push(mark_poly(request.shader.clone(), &vertices));
                continue;
            }
            if self.active.len() == MAX_MARK_POLYS {
                let oldest = self
                    .active
                    .last()
                    .ok_or_else(|| PresentError::state("full mark pool has no oldest polygon"))?;
                let oldest_time = oldest.time;
                while self.active.last().is_some_and(|mark| mark.time == oldest_time) {
                    self.active.pop();
                }
            }
            let now = self.now();
            self.active.unshift_insert(StoredMark {
                time: now,
                shader: request.shader.clone(),
                alpha_fade: request.alpha_fade,
                color,
                vertices,
            });
        }
        Ok(temporary)
    }

    /// Emit active marks (`addMarks`).
    pub fn add_marks(&mut self) -> Vec<RefPoly> {
        if !self.options.enabled() {
            return Vec::new();
        }
        let mut output = Vec::new();
        let now = self.now();
        let mut index = 0;
        while index < self.active.len() {
            let expires = self.active[index].time.wrapping_add(MARK_TOTAL_TIME);
            if now > expires {
                self.active.remove(index);
                continue;
            }
            let is_energy = self.active[index].shader == self.options.energy_shader();
            if is_energy {
                let age = now.wrapping_sub(self.active[index].time) as f32;
                let fade = (450.0 - 450.0 * (age / 3000.0)).trunc() as i32;
                let first = self.active[index].vertices.first();
                if fade < 255 && first.is_some_and(|vertex| vertex.color.x != 0.0) {
                    let color = self.active[index].color;
                    let faded = fade.max(0) as f32;
                    let rgb = vec3(
                        mark_byte(color.x * faded),
                        mark_byte(color.y * faded),
                        mark_byte(color.z * faded),
                    );
                    for vertex in &mut self.active[index].vertices {
                        vertex.color.x = rgb.x;
                        vertex.color.y = rgb.y;
                        vertex.color.z = rgb.z;
                    }
                }
            }
            let remaining = expires.wrapping_sub(now);
            if remaining < MARK_FADE_TIME {
                let fade = (255i32.wrapping_mul(remaining) / MARK_FADE_TIME) & 255;
                if self.active[index].alpha_fade {
                    for vertex in &mut self.active[index].vertices {
                        vertex.color.w = fade as f32;
                    }
                } else {
                    let color = self.active[index].color;
                    let faded = fade as f32;
                    let rgb = vec3(
                        mark_byte(color.x * faded),
                        mark_byte(color.y * faded),
                        mark_byte(color.z * faded),
                    );
                    for vertex in &mut self.active[index].vertices {
                        vertex.color.x = rgb.x;
                        vertex.color.y = rgb.y;
                        vertex.color.z = rgb.z;
                    }
                }
            }
            let mark = &self.active[index];
            output.push(mark_poly(mark.shader.clone(), &mark.vertices));
            index += 1;
        }
        output
    }

    fn now(&self) -> i32 {
        self.options.clock()
    }
}

trait VecUnshift<T> {
    fn unshift_insert(&mut self, value: T);
}

impl<T> VecUnshift<T> for Vec<T> {
    fn unshift_insert(&mut self, value: T) {
        self.insert(0, value);
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
// local-entities.ts
// ---------------------------------------------------------------------------

/// Maximum local entities.
pub const MAX_LOCAL_ENTITIES: usize = 512;
/// Puff-don't-scale flag.
pub const LE_PUFF_DONT_SCALE: i32 = 1;
/// Tumble flag.
pub const LE_TUMBLE: i32 = 2;
/// Sound 1 flag.
pub const LE_SOUND1: i32 = 4;
/// Sound 2 flag.
pub const LE_SOUND2: i32 = 8;

/// Local entity type (`leType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LocalEntityType {
    /// Move, scale, fade.
    MoveScaleFade,
    /// Fall, scale, fade.
    FallScaleFade,
    /// Scale, fade.
    ScaleFade,
    /// Score plum.
    ScorePlum,
    /// Sprite explosion.
    SpriteExplosion,
    /// Fragment.
    Fragment,
    /// Kamikaze.
    Kamikaze,
    /// Invulnerability impact.
    InvulImpact,
    /// Invulnerability juiced.
    InvulJuiced,
    /// Fade RGB.
    FadeRgb,
    /// Explosion.
    Explosion,
    /// Mark.
    Mark,
    /// Show reference entity.
    ShowRefEntity,
}

/// Local entity mark type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalMarkType {
    /// None.
    None,
    /// Burn.
    Burn,
    /// Blood.
    Blood,
}

/// Local entity bounce sound type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalBounceSoundType {
    /// None.
    None,
    /// Blood.
    Blood,
    /// Brass.
    Brass,
}

/// Local entity record (`LocalEntity`).
#[derive(Debug, Clone, PartialEq)]
pub struct LocalEntity {
    /// Type.
    pub le_type: LocalEntityType,
    /// Reference entity.
    pub ref_entity: RefEntity,
    /// Flags.
    pub le_flags: i32,
    /// Start time.
    pub start_time: i32,
    /// End time.
    pub end_time: i32,
    /// Fade-in time.
    pub fade_in_time: i32,
    /// Life rate.
    pub life_rate: f32,
    /// Position trajectory.
    pub pos: Trajectory,
    /// Angle trajectory.
    pub angles: Trajectory,
    /// Bounce factor.
    pub bounce_factor: f32,
    /// Color.
    pub color: Vec4,
    /// Radius.
    pub radius: f32,
    /// Light.
    pub light: f32,
    /// Light color.
    pub light_color: Vec3,
    /// Mark type.
    pub le_mark_type: LocalMarkType,
    /// Bounce sound type.
    pub le_bounce_sound_type: LocalBounceSoundType,
}

/// Local entity media (`LocalEntityMedia`).
#[derive(Debug, Clone, PartialEq)]
pub struct LocalEntityMedia {
    /// Blood trail shader.
    pub blood_trail_shader: Option<SceneShader>,
    /// Blood mark shader.
    pub blood_mark_shader: Option<SceneShader>,
    /// Burn mark shader.
    pub burn_mark_shader: Option<SceneShader>,
    /// Number shaders (0-9, minus).
    pub number_shaders: Vec<Option<SceneShader>>,
    /// Gib bounce sounds.
    pub gib_bounce_sounds: [Option<PresentSound>; 3],
}

/// Mission local entity media (`MissionLocalEntityMedia`).
#[derive(Debug, Clone, PartialEq)]
pub struct MissionLocalEntityMedia {
    /// Base.
    pub base: LocalEntityMedia,
    /// Kamikaze shock wave.
    pub kamikaze_shock_wave: SceneModel,
    /// Kamikaze explode sound.
    pub kamikaze_explode_sound: Option<PresentSound>,
    /// Kamikaze implode sound.
    pub kamikaze_implode_sound: Option<PresentSound>,
}

/// Local entity host media variant.
#[derive(Debug, Clone, PartialEq)]
pub enum LocalEntityHostMedia {
    /// Base.
    Base(LocalEntityMedia),
    /// Mission.
    Mission(MissionLocalEntityMedia),
}

/// Local entity host (`LocalEntityHost`).
pub struct LocalEntityHost {
    /// Prediction.
    pub prediction: Box<dyn PresentPrediction>,
    /// Collision.
    pub collision: Box<dyn PresentCollision>,
    /// Audio.
    pub audio: Box<dyn PresentAudio>,
    /// Client number.
    pub client_num: i32,
    /// Random.
    pub random: Box<dyn PresentRandom>,
    /// Marks.
    pub marks: Box<dyn PresentMarks>,
    /// Product.
    pub product: Product,
    /// Media.
    pub media: LocalEntityHostMedia,
}

/// Local entity frame (`LocalEntityFrame`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LocalEntityFrame {
    /// Time.
    pub time: i32,
    /// Frame time.
    pub frame_time: i32,
    /// View origin.
    pub view_origin: Vec3,
}

/// Collected local entity scene (`LocalEntityScene`).
#[derive(Debug, Clone, PartialEq)]
pub struct LocalEntityScene {
    /// Entities.
    pub entities: Vec<RefEntity>,
    /// Dynamic lights.
    pub dynamic_lights: Vec<DynamicLight>,
}

/// Local entity scene sink (`LocalEntitySceneSink`).
pub trait LocalEntitySceneSink {
    /// Add a reference entity.
    fn add_ref_entity(&mut self, entity: &RefEntity);
    /// Add a light.
    fn add_light(&mut self, light: &DynamicLight);
}

/// Local entity handle (slot index plus generation).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LocalEntityHandle {
    /// Slot index.
    pub index: usize,
    /// Generation.
    pub generation: u64,
}

#[derive(Debug, Clone, PartialEq)]
struct LocalSlot {
    prev: i32,
    next: i32,
    entity: Option<LocalEntity>,
    generation: u64,
    active: bool,
}

fn le_byte(value: f32) -> f32 {
    (qvm_float_to_int(value) & 255) as f32
}

fn le_remaining(entity: &LocalEntity, time: i32) -> f32 {
    (entity.end_time.wrapping_sub(time) as f32) * entity.life_rate
}

fn le_rgba(color: Vec4, scale: f32) -> Vec4 {
    vec4(
        le_byte(color.x * scale),
        le_byte(color.y * scale),
        le_byte(color.z * scale),
        le_byte(color.w * scale),
    )
}

fn le_scaled_axis(axis: &Axis, scale: f32) -> Axis {
    [scale3(axis[0], scale), scale3(axis[1], scale), scale3(axis[2], scale)]
}

/// Local entity pool (`LocalEntityPool`).
#[derive(Debug, Clone, PartialEq)]
pub struct LocalEntityPool {
    /// Product.
    pub product: Product,
    slots: Vec<LocalSlot>,
    head: i32,
    tail: i32,
    free_head: i32,
    count: usize,
    generation: u64,
}

impl LocalEntityPool {
    /// New pool.
    pub fn new(product: Product) -> Self {
        let mut pool = Self {
            product,
            slots: Vec::new(),
            head: -1,
            tail: -1,
            free_head: 0,
            count: 0,
            generation: 0,
        };
        pool.initialize();
        pool
    }

    /// Active count.
    #[must_use]
    pub fn active_count(&self) -> usize {
        self.count
    }

    /// Active entities newest-first.
    #[must_use]
    pub fn active_entities(&self) -> Vec<LocalEntity> {
        let mut entities = Vec::new();
        let mut index = self.head;
        while index != -1 {
            if let Some(entity) = &self.slots[index as usize].entity {
                entities.push(entity.clone());
            }
            index = self.slots[index as usize].next;
        }
        entities
    }

    /// Initialize all slots.
    pub fn initialize(&mut self) {
        self.head = -1;
        self.tail = -1;
        self.free_head = 0;
        self.count = 0;
        self.slots = (0..MAX_LOCAL_ENTITIES)
            .map(|index| LocalSlot {
                prev: -1,
                next: if index + 1 == MAX_LOCAL_ENTITIES {
                    -1
                } else {
                    index as i32 + 1
                },
                entity: None,
                generation: 0,
                active: false,
            })
            .collect();
    }

    /// Allocate a record (`allocate`).
    pub fn allocate(&mut self, le_type: LocalEntityType, ref_entity: RefEntity) -> PresentResult<LocalEntityHandle> {
        if self.product == Product::BaseQ3
            && matches!(
                le_type,
                LocalEntityType::Kamikaze
                    | LocalEntityType::InvulImpact
                    | LocalEntityType::InvulJuiced
                    | LocalEntityType::ShowRefEntity
            )
        {
            return Err(PresentError::state(format!(
                "{le_type:?} requires missionpack local entities"
            )));
        }
        match le_type {
            LocalEntityType::MoveScaleFade
            | LocalEntityType::FallScaleFade
            | LocalEntityType::ScaleFade
            | LocalEntityType::ScorePlum
            | LocalEntityType::SpriteExplosion => {
                if !matches!(ref_entity, RefEntity::Sprite(_)) {
                    return Err(PresentError::state(format!("{le_type:?} requires a sprite")));
                }
            }
            LocalEntityType::Fragment
            | LocalEntityType::Kamikaze
            | LocalEntityType::InvulImpact
            | LocalEntityType::InvulJuiced => {
                if !matches!(ref_entity, RefEntity::Model(_)) {
                    return Err(PresentError::state(format!("{le_type:?} requires a model")));
                }
            }
            LocalEntityType::FadeRgb | LocalEntityType::Explosion => {
                if matches!(ref_entity, RefEntity::Portal(_)) {
                    return Err(PresentError::state(format!("{le_type:?} requires a shaded entity")));
                }
            }
            LocalEntityType::Mark | LocalEntityType::ShowRefEntity => {}
        }
        if self.free_head == -1 {
            self.free_slot(self.tail)?;
        }
        let index = self.free_head as usize;
        let next_free = self.slots[index].next;
        self.free_head = next_free;
        self.generation += 1;
        let generation = self.generation;
        self.slots[index].entity = Some(LocalEntity {
            le_type,
            ref_entity,
            le_flags: 0,
            start_time: 0,
            end_time: 0,
            fade_in_time: 0,
            life_rate: 0.0,
            pos: Trajectory::default(),
            angles: Trajectory::default(),
            bounce_factor: 0.0,
            color: vec4(0.0, 0.0, 0.0, 0.0),
            radius: 0.0,
            light: 0.0,
            light_color: zero_vec3(),
            le_mark_type: LocalMarkType::None,
            le_bounce_sound_type: LocalBounceSoundType::None,
        });
        self.slots[index].active = true;
        self.slots[index].generation = generation;
        self.slots[index].prev = -1;
        self.slots[index].next = self.head;
        if self.head != -1 {
            self.slots[self.head as usize].prev = index as i32;
        } else {
            self.tail = index as i32;
        }
        self.head = index as i32;
        self.count += 1;
        Ok(LocalEntityHandle { index, generation })
    }

    /// Whether a handle is active (`isActive`).
    #[must_use]
    pub fn is_active(&self, handle: LocalEntityHandle) -> bool {
        self.slots
            .get(handle.index)
            .is_some_and(|slot| slot.active && slot.generation == handle.generation && slot.entity.is_some())
    }

    /// Read a record by slot, live or stale (`record`).
    pub fn read_record(&self, index: usize) -> PresentResult<&LocalEntity> {
        self.slots
            .get(index)
            .and_then(|slot| slot.entity.as_ref())
            .ok_or_else(|| PresentError::state("Local entity slot has no record"))
    }

    /// Read a live record by handle.
    pub fn get(&self, handle: LocalEntityHandle) -> Option<&LocalEntity> {
        self.is_active(handle)
            .then(|| self.slots[handle.index].entity.as_ref())
            .flatten()
    }

    /// Read a live record by handle, mutably.
    pub fn get_mut(&mut self, handle: LocalEntityHandle) -> Option<&mut LocalEntity> {
        if self.is_active(handle) {
            self.slots[handle.index].entity.as_mut()
        } else {
            None
        }
    }

    /// Free a record (`free`).
    pub fn free(&mut self, handle: LocalEntityHandle) -> PresentResult<()> {
        if !self.is_active(handle) {
            return Err(PresentError::state("CG_FreeLocalEntity: not active"));
        }
        self.free_slot(handle.index as i32)?;
        Ok(())
    }

    fn free_slot(&mut self, index: i32) -> PresentResult<()> {
        if index < 0 {
            return Err(PresentError::state("CG_FreeLocalEntity: not active"));
        }
        let slot = self
            .slots
            .get(index as usize)
            .ok_or_else(|| PresentError::range(format!("Invalid local entity slot {index}")))?;
        if !slot.active {
            return Err(PresentError::state("CG_FreeLocalEntity: not active"));
        }
        let (prev, next) = (slot.prev, slot.next);
        if prev == -1 {
            self.head = next;
        } else {
            self.slots[prev as usize].next = next;
        }
        if next == -1 {
            self.tail = prev;
        } else {
            self.slots[next as usize].prev = prev;
        }
        self.slots[index as usize].active = false;
        self.slots[index as usize].next = self.free_head;
        self.free_head = index;
        self.count -= 1;
        Ok(())
    }

    /// Visit oldest-first with the cached-prev semantics of the C array
    /// (`forEachOldestFirst`).
    pub fn for_each_oldest_first(&mut self, mut visit: impl FnMut(&mut Self, LocalEntityHandle)) {
        let mut index = self.tail;
        while index != -1 {
            let current = index as usize;
            let next = self.slots[current].prev;
            let handle = LocalEntityHandle {
                index: current,
                generation: self.slots[current].generation,
            };
            visit(self, handle);
            index = next;
        }
    }
}

/// Local entity processor (`LocalEntitySystem`).
pub struct LocalEntitySystem {
    /// Host.
    pub host: LocalEntityHost,
}

impl LocalEntitySystem {
    /// New system.
    pub fn new(host: LocalEntityHost) -> Self {
        Self { host }
    }

    fn check_pool(&self, pool: &LocalEntityPool) -> PresentResult<()> {
        if pool.product != self.host.product {
            return Err(PresentError::state("Local entity media product differs from its pool"));
        }
        Ok(())
    }

    /// Collect owned snapshots (`collectEntities`).
    pub fn collect_entities(
        &mut self,
        pool: &mut LocalEntityPool,
        effects: &mut ClientEffects,
        frame: &LocalEntityFrame,
    ) -> PresentResult<LocalEntityScene> {
        struct Collector {
            scene: LocalEntityScene,
        }
        impl LocalEntitySceneSink for Collector {
            fn add_ref_entity(&mut self, entity: &RefEntity) {
                self.scene.entities.push(copy_ref_entity(entity));
            }
            fn add_light(&mut self, light: &DynamicLight) {
                self.scene.dynamic_lights.push(*light);
            }
        }
        let mut collector = Collector {
            scene: LocalEntityScene {
                entities: Vec::new(),
                dynamic_lights: Vec::new(),
            },
        };
        self.add_entities(pool, effects, frame, &mut collector)?;
        Ok(collector.scene)
    }

    /// Add frame entities (`addEntities`).
    pub fn add_entities(
        &mut self,
        pool: &mut LocalEntityPool,
        effects: &mut ClientEffects,
        frame: &LocalEntityFrame,
        scene: &mut dyn LocalEntitySceneSink,
    ) -> PresentResult<()> {
        self.check_pool(pool)?;
        // Drive the loop here (rather than inside the pool) so callbacks can
        // borrow the pool, effects, host, and scene independently.
        let mut index = pool.tail;
        let mut first_error: Option<PresentError> = None;
        while index != -1 {
            let current = index as usize;
            // Cache the source prev pointer before each callback.
            let next = pool.slots[current].prev;
            let handle = LocalEntityHandle {
                index: current,
                generation: pool.slots[current].generation,
            };
            let entity = pool.read_record(current)?.clone();
            if frame.time >= entity.end_time {
                pool.free(handle)?;
            } else {
                let result = match entity.le_type {
                    LocalEntityType::Mark => Ok(()),
                    LocalEntityType::Fragment => Self::fragment(&mut self.host, pool, effects, handle, frame, scene),
                    LocalEntityType::MoveScaleFade | LocalEntityType::FallScaleFade | LocalEntityType::ScaleFade => {
                        Self::scale_fade(&mut self.host, pool, handle, frame, scene)
                    }
                    LocalEntityType::FadeRgb => {
                        if let Some(live) = pool.get_mut(handle) {
                            let fade = le_remaining(live, frame.time) * 255.0;
                            let rgba = le_rgba(live.color, fade);
                            set_shading_rgba(&mut live.ref_entity, rgba);
                            let entity = live.ref_entity.clone();
                            scene.add_ref_entity(&entity);
                        }
                        Ok(())
                    }
                    LocalEntityType::Explosion => {
                        if let Some(live) = pool.get(handle) {
                            let entity = live.ref_entity.clone();
                            scene.add_ref_entity(&entity);
                        }
                        if let Some(live) = pool.get(handle) {
                            let live = live.clone();
                            Self::explosion_light(&live, frame.time, scene);
                        }
                        Ok(())
                    }
                    LocalEntityType::SpriteExplosion => Self::sprite_explosion(pool, handle, frame, scene),
                    LocalEntityType::ScorePlum => Self::score_plum(&mut self.host, pool, handle, frame, scene),
                    LocalEntityType::Kamikaze => Self::kamikaze(&mut self.host, pool, handle, frame, scene),
                    LocalEntityType::InvulImpact | LocalEntityType::ShowRefEntity => {
                        if let Some(live) = pool.get(handle) {
                            let entity = live.ref_entity.clone();
                            scene.add_ref_entity(&entity);
                        }
                        Ok(())
                    }
                    LocalEntityType::InvulJuiced => Self::invulnerability_juiced(pool, effects, handle, frame, scene),
                };
                if let Err(error) = result {
                    first_error = Some(error);
                    break;
                }
            }
            index = next;
        }
        if let Some(error) = first_error {
            return Err(error);
        }
        Ok(())
    }

    fn scale_fade(
        _host: &mut LocalEntityHost,
        pool: &mut LocalEntityPool,
        handle: LocalEntityHandle,
        frame: &LocalEntityFrame,
        scene: &mut dyn LocalEntitySceneSink,
    ) -> PresentResult<()> {
        let submission = {
            let Some(live) = pool.get_mut(handle) else {
                return Ok(());
            };
            let mut c = le_remaining(live, frame.time);
            if live.le_type == LocalEntityType::MoveScaleFade
                && live.fade_in_time > live.start_time
                && frame.time < live.fade_in_time
            {
                c = 1.0 - ((live.fade_in_time - frame.time) as f32) / ((live.fade_in_time - live.start_time) as f32);
            }
            let alpha = le_byte(255.0 * c * live.color.w);
            let radius_base = live.radius;
            let le_type = live.le_type;
            let le_flags = live.le_flags;
            let pos = live.pos;
            let mut submission = None;
            if let RefEntity::Sprite(re) = &mut live.ref_entity {
                re.shading.shader_rgba.w = alpha;
                if le_type != LocalEntityType::MoveScaleFade || le_flags & LE_PUFF_DONT_SCALE == 0 {
                    re.radius = radius_base * (1.0 - c)
                        + if le_type == LocalEntityType::FallScaleFade {
                            16.0
                        } else {
                            8.0
                        };
                }
                if le_type == LocalEntityType::MoveScaleFade {
                    re.origin = evaluate_trajectory(&pos, frame.time);
                } else if le_type == LocalEntityType::FallScaleFade {
                    re.origin.z = pos.base.z - (1.0 - c) * pos.delta.z;
                }
                if length3(sub3(re.origin, frame.view_origin)) < radius_base {
                    submission = Some(None);
                } else {
                    submission = Some(Some(RefEntity::Sprite(re.clone())));
                }
            }
            submission
        };
        match submission {
            Some(None) => {
                pool.free(handle)?;
            }
            Some(Some(entity)) => {
                scene.add_ref_entity(&entity);
            }
            None => {}
        }
        Ok(())
    }

    fn explosion_light(entity: &LocalEntity, time: i32, scene: &mut dyn LocalEntitySceneSink) {
        if entity.light == 0.0 {
            return;
        }
        let mut light = ((time - entity.start_time) as f32) / ((entity.end_time - entity.start_time) as f32);
        light = if light < 0.5 { 1.0 } else { 1.0 - (light - 0.5) * 2.0 };
        scene.add_light(&DynamicLight {
            origin: entity.ref_entity.origin(),
            radius: entity.light * light,
            color: entity.light_color,
            additive: false,
        });
    }

    fn sprite_explosion(
        pool: &mut LocalEntityPool,
        handle: LocalEntityHandle,
        frame: &LocalEntityFrame,
        scene: &mut dyn LocalEntitySceneSink,
    ) -> PresentResult<()> {
        let Some(live) = pool.get(handle) else { return Ok(()) };
        let live = live.clone();
        let c = (((live.end_time - frame.time) as f32) / ((live.end_time - live.start_time) as f32)).min(1.0);
        if let RefEntity::Sprite(re) = &live.ref_entity {
            let mut re = re.clone();
            re.shading.shader_rgba = vec4(255.0, 255.0, 255.0, le_byte(255.0 * c * 0.33));
            re.radius = 42.0 * (1.0 - c) + 30.0;
            scene.add_ref_entity(&RefEntity::Sprite(re));
        }
        Self::explosion_light(&live, frame.time, scene);
        Ok(())
    }

    fn fragment(
        host: &mut LocalEntityHost,
        pool: &mut LocalEntityPool,
        effects: &mut ClientEffects,
        handle: LocalEntityHandle,
        frame: &LocalEntityFrame,
        scene: &mut dyn LocalEntitySceneSink,
    ) -> PresentResult<()> {
        let Some(live) = pool.get(handle) else { return Ok(()) };
        if live.pos.type_ == TrajectoryType::Stationary {
            let t = live.end_time - frame.time;
            if t < 1000 {
                let Some(live) = pool.get_mut(handle) else {
                    return Ok(());
                };
                if let RefEntity::Model(re) = &mut live.ref_entity {
                    re.lighting_origin = re.origin;
                    re.shading.render_flags |= RF_LIGHTING_ORIGIN;
                    let origin = re.origin;
                    re.origin.z = origin.z - 16.0 * (1.0 - (t as f32) / 1000.0);
                    let entity = RefEntity::Model(re.clone());
                    scene.add_ref_entity(&entity);
                    re.origin = origin;
                }
            } else if let Some(live) = pool.get(handle) {
                let entity = live.ref_entity.clone();
                scene.add_ref_entity(&entity);
            }
            return Ok(());
        }
        let pos = live.pos;
        let origin = match &live.ref_entity {
            RefEntity::Model(re) => re.origin,
            other => other.origin(),
        };
        let new_origin = evaluate_trajectory(&pos, frame.time);
        let zero = zero_vec3();
        let trace = host
            .prediction
            .trace_mover(origin, new_origin, Bounds { min: zero, max: zero }, -1, 1);
        if trace.base.fraction == 1.0 {
            let Some(live) = pool.get_mut(handle) else {
                return Ok(());
            };
            let angles = live.angles;
            let tumble = live.le_flags & LE_TUMBLE != 0;
            let blood = live.le_bounce_sound_type == LocalBounceSoundType::Blood;
            if let RefEntity::Model(re) = &mut live.ref_entity {
                re.origin = new_origin;
                if tumble {
                    re.axis = angles_to_axis(evaluate_trajectory(&angles, frame.time));
                }
                let entity = RefEntity::Model(re.clone());
                scene.add_ref_entity(&entity);
            }
            if blood {
                Self::blood_trail(host, pool, effects, handle.index, frame)?;
            }
            return Ok(());
        }
        if host.collision.collision_contents(trace.base.end) & 0x80000000u32 as i32 != 0 {
            pool.free(handle)?;
            return Ok(());
        }
        let normal = match trace.base.contact {
            TraceContact::Plane { plane } => plane.normal,
            TraceContact::None => zero_vec3(),
        };
        if matches!(trace.base.contact, TraceContact::None) && trace.base.solidity != TraceSolidity::AllSolid {
            return Err(PresentError::state("Fragment impact has no trace plane"));
        }
        Self::bounce_mark(host, pool, handle, trace.base.end, normal);
        Self::bounce_sound(host, pool, handle, trace.base.end);
        Self::reflect_velocity(pool, handle, &trace.base, normal, frame);
        if let Some(live) = pool.get(handle) {
            let entity = live.ref_entity.clone();
            scene.add_ref_entity(&entity);
        }
        Ok(())
    }

    fn reflect_velocity(
        pool: &mut LocalEntityPool,
        handle: LocalEntityHandle,
        trace: &TraceResult,
        normal: Vec3,
        frame: &LocalEntityFrame,
    ) {
        let Some(live) = pool.get_mut(handle) else { return };
        let hit_time =
            qvm_float_to_int(((frame.time - frame.frame_time) as f32) + (frame.frame_time as f32) * trace.fraction);
        let velocity = evaluate_trajectory_delta(&live.pos, hit_time);
        let dot = dot3(velocity, normal);
        let delta = scale3(add3(velocity, scale3(normal, -2.0 * dot)), live.bounce_factor);
        let stationary = trace.solidity == TraceSolidity::AllSolid
            || (normal.z > 0.0 && (delta.z < 40.0 || delta.z < (-(frame.frame_time as f32)) * delta.z));
        live.pos.base = trace.end;
        live.pos.time = frame.time;
        live.pos.delta = delta;
        if stationary {
            live.pos.type_ = TrajectoryType::Stationary;
        }
    }

    fn bounce_mark(
        host: &mut LocalEntityHost,
        pool: &mut LocalEntityPool,
        handle: LocalEntityHandle,
        origin: Vec3,
        normal: Vec3,
    ) {
        let (mark_type, blood_shader, burn_shader) = match pool.get_mut(handle) {
            Some(live) => {
                let media = match &host.media {
                    LocalEntityHostMedia::Base(media) => media,
                    LocalEntityHostMedia::Mission(media) => &media.base,
                };
                (
                    live.le_mark_type,
                    media.blood_mark_shader.clone(),
                    media.burn_mark_shader.clone(),
                )
            }
            None => return,
        };
        if mark_type != LocalMarkType::None {
            let blood = mark_type == LocalMarkType::Blood;
            let radius = (if blood { 16 } else { 8 } + (host.random.rand() & if blood { 31 } else { 15 })) as f32;
            let orientation = host.random.random() * 360.0;
            host.marks.impact_mark(&ImpactMarkRequest {
                shader: if blood { blood_shader } else { burn_shader },
                origin,
                direction: normal,
                orientation,
                color: vec4(1.0, 1.0, 1.0, 1.0),
                alpha_fade: true,
                radius,
                temporary: false,
            });
        }
        if let Some(live) = pool.get_mut(handle) {
            live.le_mark_type = LocalMarkType::None;
        }
    }

    fn bounce_sound(host: &mut LocalEntityHost, pool: &mut LocalEntityPool, handle: LocalEntityHandle, origin: Vec3) {
        let bounce_type = pool.get(handle).map(|live| live.le_bounce_sound_type);
        if bounce_type == Some(LocalBounceSoundType::Blood) && host.random.rand() & 1 != 0 {
            let value = host.random.rand() & 3;
            let media = match &host.media {
                LocalEntityHostMedia::Base(media) => media,
                LocalEntityHostMedia::Mission(media) => &media.base,
            };
            let sound = if value == 0 {
                media.gib_bounce_sounds[0].clone()
            } else if value == 1 {
                media.gib_bounce_sounds[1].clone()
            } else {
                media.gib_bounce_sounds[2].clone()
            };
            host.audio.start_sound(
                sound,
                &SoundOptions {
                    entity: ENTITYNUM_WORLD,
                    channel: 0,
                    origin: SoundOrigin::Fixed { position: origin },
                    volume: 127,
                },
            );
        }
        if let Some(live) = pool.get_mut(handle) {
            live.le_bounce_sound_type = LocalBounceSoundType::None;
        }
    }

    fn blood_trail(
        host: &mut LocalEntityHost,
        pool: &mut LocalEntityPool,
        effects: &mut ClientEffects,
        index: usize,
        frame: &LocalEntityFrame,
    ) -> PresentResult<()> {
        let step = 150;
        let start = step * (((frame.time - frame.frame_time + step) / step) as i32);
        let end = step * ((frame.time / step) as i32);
        let mut time = start;
        let shader = match &host.media {
            LocalEntityHostMedia::Base(media) => media.blood_trail_shader.clone(),
            LocalEntityHostMedia::Mission(media) => media.base.blood_trail_shader.clone(),
        };
        let effect_frame = EffectFrame {
            time: frame.time,
            product: host.product,
            snap_client: None,
            predicted_client: 0,
        };
        while time <= end {
            let origin = evaluate_trajectory(&pool.read_record(index)?.pos, time);
            let handle = effects.smoke_puff(
                pool,
                &effect_frame,
                &SmokePuffOptions {
                    origin,
                    velocity: zero_vec3(),
                    radius: 20.0,
                    color: vec4(1.0, 1.0, 1.0, 1.0),
                    duration: 2000,
                    start_time: time,
                    fade_in_time: 0,
                    flags: 0,
                    shader: shader.clone(),
                },
            )?;
            if let Some(blood) = pool.get_mut(handle) {
                blood.le_type = LocalEntityType::FallScaleFade;
                blood.pos.delta.z = 40.0;
            }
            time += step;
        }
        Ok(())
    }

    fn score_plum(
        host: &mut LocalEntityHost,
        pool: &mut LocalEntityPool,
        handle: LocalEntityHandle,
        frame: &LocalEntityFrame,
        scene: &mut dyn LocalEntitySceneSink,
    ) -> PresentResult<()> {
        let Some(live) = pool.get(handle) else { return Ok(()) };
        let live = live.clone();
        let c = le_remaining(&live, frame.time);
        let mut score = qvm_float_to_int(live.radius);
        let mut color = if score < 0 {
            vec4(255.0, 17.0, 17.0, 255.0)
        } else if score >= 50 {
            vec4(255.0, 0.0, 255.0, 255.0)
        } else if score >= 20 {
            vec4(0.0, 0.0, 255.0, 255.0)
        } else if score >= 10 {
            vec4(255.0, 255.0, 0.0, 255.0)
        } else if score >= 2 {
            vec4(0.0, 255.0, 0.0, 255.0)
        } else {
            vec4(255.0, 255.0, 255.0, 255.0)
        };
        if c < 0.25 {
            color.w = le_byte(255.0 * 4.0 * c);
        }
        let RefEntity::Sprite(mut re) = live.ref_entity.clone() else {
            return Ok(());
        };
        re.shading.shader_rgba = color;
        re.radius = 4.0;
        let mut origin = vec3(live.pos.base.x, live.pos.base.y, live.pos.base.z + (110.0 - c * 100.0));
        let direction = normalize3(cross3(sub3(frame.view_origin, origin), vec3(0.0, 0.0, 1.0)));
        let phase = c * 2.0 * std::f32::consts::PI;
        origin = add3(origin, scale3(direction, -10.0 + 20.0 * phase.sin()));
        if length3(sub3(origin, frame.view_origin)) < 20.0 {
            pool.free(handle)?;
            return Ok(());
        }
        let negative = score < 0;
        if negative {
            score = -score;
        }
        let mut digits = Vec::new();
        loop {
            digits.push((score % 10) as usize);
            score /= 10;
            if score == 0 {
                break;
            }
        }
        if negative {
            digits.push(10);
        }
        let media = match &host.media {
            LocalEntityHostMedia::Base(media) => media,
            LocalEntityHostMedia::Mission(media) => &media.base,
        };
        for (index, _) in digits.iter().enumerate() {
            re.origin = add3(
                origin,
                scale3(direction, ((digits.len() as f32) / 2.0 - index as f32) * 8.0),
            );
            let digit = digits[digits.len() - 1 - index];
            let shader = media
                .number_shaders
                .get(digit)
                .and_then(|shader| shader.clone())
                .ok_or_else(|| PresentError::state("Missing score digit shader"))?;
            re.shading.custom_shader = Some(shader);
            scene.add_ref_entity(&RefEntity::Sprite(re.clone()));
        }
        if let Some(live) = pool.get_mut(handle) {
            live.ref_entity = RefEntity::Sprite(re);
        }
        Ok(())
    }

    fn kamikaze(
        host: &mut LocalEntityHost,
        pool: &mut LocalEntityPool,
        handle: LocalEntityHandle,
        frame: &LocalEntityFrame,
        scene: &mut dyn LocalEntitySceneSink,
    ) -> PresentResult<()> {
        if host.product != Product::MissionPack {
            return Err(PresentError::state("Kamikaze requires missionpack media"));
        }
        let Some(live) = pool.get(handle) else { return Ok(()) };
        let t = frame.time - live.start_time;
        let axis = angles_to_axis(zero_vec3());
        if t > 0 && t < 2000 {
            let sounded = pool.get(handle).is_some_and(|live| live.le_flags & LE_SOUND1 != 0);
            if !sounded {
                let sound = match &host.media {
                    LocalEntityHostMedia::Mission(media) => media.kamikaze_explode_sound.clone(),
                    LocalEntityHostMedia::Base(_) => None,
                };
                let client_num = host.client_num;
                host.audio.start_sound(
                    sound,
                    &SoundOptions {
                        entity: client_num,
                        channel: 0,
                        origin: SoundOrigin::Local,
                        volume: 127,
                    },
                );
                if let Some(live) = pool.get_mut(handle) {
                    live.le_flags |= LE_SOUND1;
                }
            }
            Self::shockwave(host, pool, handle, &axis, t, 0, 2000, 1500, 1320, scene)?;
        }
        if t > 250 && t < 2250 {
            let remaining = pool
                .get(handle)
                .map(|live| le_remaining(live, frame.time))
                .unwrap_or(0.0);
            let c = if t < 2000 {
                ((t - 250) as f32) / 1750.0
            } else {
                let sounded = pool.get(handle).is_some_and(|live| live.le_flags & LE_SOUND2 != 0);
                if !sounded {
                    let sound = match &host.media {
                        LocalEntityHostMedia::Mission(media) => media.kamikaze_implode_sound.clone(),
                        LocalEntityHostMedia::Base(_) => None,
                    };
                    let client_num = host.client_num;
                    host.audio.start_sound(
                        sound,
                        &SoundOptions {
                            entity: client_num,
                            channel: 0,
                            origin: SoundOrigin::Local,
                            volume: 127,
                        },
                    );
                    if let Some(live) = pool.get_mut(handle) {
                        live.le_flags |= LE_SOUND2;
                    }
                }
                ((2250 - t) as f32) / 250.0
            };
            let Some(live) = pool.get_mut(handle) else {
                return Ok(());
            };
            if let RefEntity::Model(re) = &mut live.ref_entity {
                re.shading.shader_rgba = le_rgba(live.color, remaining * 255.0);
                re.axis = le_scaled_axis(&axis, (c * 720.0) / 72.0);
                re.non_normalized_axes = true;
                let origin = re.origin;
                let entity = RefEntity::Model(re.clone());
                scene.add_ref_entity(&entity);
                scene.add_light(&DynamicLight {
                    origin,
                    radius: c * 1000.0,
                    color: vec3(1.0, 1.0, c),
                    additive: false,
                });
            }
        }
        if t > 2000 && t < 3000 {
            let needs_angles = pool.get(handle).is_some_and(|live| {
                live.angles.base.x == 0.0 && live.angles.base.y == 0.0 && live.angles.base.z == 0.0
            });
            if needs_angles {
                let angles = vec3(
                    host.random.random() * 360.0,
                    host.random.random() * 360.0,
                    host.random.random() * 360.0,
                );
                if let Some(live) = pool.get_mut(handle) {
                    live.angles.base = angles;
                }
            }
            let axis = pool
                .get(handle)
                .map(|live| angles_to_axis(live.angles.base))
                .unwrap_or(axis);
            Self::shockwave(host, pool, handle, &axis, t, 2000, 3000, 2500, 704, scene)?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn shockwave(
        host: &mut LocalEntityHost,
        pool: &mut LocalEntityPool,
        handle: LocalEntityHandle,
        axis: &Axis,
        time: i32,
        start: i32,
        end: i32,
        fade: i32,
        radius: i32,
        scene: &mut dyn LocalEntitySceneSink,
    ) -> PresentResult<()> {
        let LocalEntityHostMedia::Mission(media) = &host.media else {
            return Err(PresentError::state("Shockwave requires missionpack media"));
        };
        let Some(live) = pool.get(handle) else { return Ok(()) };
        let (shader_time, origin) = match &live.ref_entity {
            RefEntity::Model(re) => (re.shading.shader_time, re.origin),
            other => (0.0, other.origin()),
        };
        let mut re = create_model_entity(media.kamikaze_shock_wave.clone());
        re.shading.shader_time = shader_time;
        re.origin = origin;
        let c = ((time - start) as f32) / ((end - start) as f32);
        re.axis = le_scaled_axis(axis, (c * radius as f32) / 88.0);
        re.non_normalized_axes = true;
        let alpha = if time > fade {
            ((time - fade) as f32) / ((end - fade) as f32)
        } else {
            0.0
        };
        let channel = le_byte(255.0 - alpha * 255.0);
        re.shading.shader_rgba = vec4(channel, channel, channel, channel);
        scene.add_ref_entity(&RefEntity::Model(re));
        Ok(())
    }

    fn invulnerability_juiced(
        pool: &mut LocalEntityPool,
        effects: &mut ClientEffects,
        handle: LocalEntityHandle,
        frame: &LocalEntityFrame,
        scene: &mut dyn LocalEntitySceneSink,
    ) -> PresentResult<()> {
        let Some(live) = pool.get(handle) else { return Ok(()) };
        let t = frame.time - live.start_time;
        if t > 3000 {
            let xy = 1.0 + 0.3 * ((t - 3000) as f32) / 2000.0;
            let z = 0.7 + 0.3 * ((2000 - (t - 3000)) as f32) / 2000.0;
            if let Some(live) = pool.get_mut(handle) {
                if let RefEntity::Model(re) = &mut live.ref_entity {
                    re.axis[0].x = xy;
                    re.axis[1].y = xy;
                    re.axis[2].z = z;
                }
            }
        }
        if t > 5000 {
            let origin = pool
                .get(handle)
                .map(|live| live.ref_entity.origin())
                .unwrap_or(zero_vec3());
            if let Some(live) = pool.get_mut(handle) {
                live.end_time = 0;
            }
            let product = effects.product();
            let effect_frame = EffectFrame {
                time: frame.time,
                product,
                snap_client: None,
                predicted_client: 0,
            };
            effects.gib_player(pool, &effect_frame, origin)?;
        } else if let Some(live) = pool.get(handle) {
            let entity = live.ref_entity.clone();
            scene.add_ref_entity(&entity);
        }
        Ok(())
    }
}

fn set_shading_rgba(entity: &mut RefEntity, rgba: Vec4) {
    match entity {
        RefEntity::Model(re) => re.shading.shader_rgba = rgba,
        RefEntity::Sprite(re) => re.shading.shader_rgba = rgba,
        RefEntity::Beam(re) => re.shading.shader_rgba = rgba,
        RefEntity::RailCore(re) => re.shading.shader_rgba = rgba,
        RefEntity::RailRings(re) => re.shading.shader_rgba = rgba,
        RefEntity::Lightning(re) => re.shading.shader_rgba = rgba,
        RefEntity::Portal(_) => {}
    }
}

// ---------------------------------------------------------------------------
// effects.ts
// ---------------------------------------------------------------------------

/// Mission effect media (`MissionEffectMedia`).
#[derive(Debug, Clone, PartialEq)]
pub struct MissionEffectMedia {
    /// Lightning shader.
    pub lightning_shader: Option<SceneShader>,
    /// Kamikaze effect model.
    pub kamikaze_effect_model: SceneModel,
    /// Dish flash model.
    pub dish_flash_model: SceneModel,
    /// Rocket explosion shader.
    pub rocket_explosion_shader: Option<SceneShader>,
    /// Obelisk hit sounds.
    pub obelisk_hit_sounds: [Option<PresentSound>; 3],
    /// Invulnerability impact model.
    pub invulnerability_impact_model: SceneModel,
    /// Invulnerability impact sounds.
    pub invulnerability_impact_sounds: [Option<PresentSound>; 3],
    /// Invulnerability juiced model.
    pub invulnerability_juiced_model: SceneModel,
    /// Invulnerability juiced sound.
    pub invulnerability_juiced_sound: Option<PresentSound>,
}

/// Effect media variant.
#[derive(Debug, Clone, PartialEq)]
pub enum EffectMediaVariant {
    /// Base.
    Base {
        /// Teleport effect shader.
        teleport_effect_shader: Option<SceneShader>,
    },
    /// Mission.
    Mission(MissionEffectMedia),
}

/// Effect media (`EffectMedia`).
#[derive(Debug, Clone, PartialEq)]
pub struct EffectMedia {
    /// Water bubble shader.
    pub water_bubble_shader: Option<SceneShader>,
    /// Rage Pro smoke shader.
    pub smoke_puff_rage_pro_shader: Option<SceneShader>,
    /// Blood explosion shader.
    pub blood_explosion_shader: Option<SceneShader>,
    /// Teleport effect model.
    pub teleport_effect_model: SceneModel,
    /// Gib skull.
    pub gib_skull: SceneModel,
    /// Gib brain.
    pub gib_brain: SceneModel,
    /// Gib abdomen.
    pub gib_abdomen: SceneModel,
    /// Gib arm.
    pub gib_arm: SceneModel,
    /// Gib chest.
    pub gib_chest: SceneModel,
    /// Gib fist.
    pub gib_fist: SceneModel,
    /// Gib foot.
    pub gib_foot: SceneModel,
    /// Gib forearm.
    pub gib_forearm: SceneModel,
    /// Gib intestine.
    pub gib_intestine: SceneModel,
    /// Gib leg.
    pub gib_leg: SceneModel,
    /// Smoke 2.
    pub smoke2: SceneModel,
    /// Variant.
    pub variant: EffectMediaVariant,
}

impl EffectMedia {
    /// Product.
    #[must_use]
    pub fn product(&self) -> Product {
        match &self.variant {
            EffectMediaVariant::Base { .. } => Product::BaseQ3,
            EffectMediaVariant::Mission(_) => Product::MissionPack,
        }
    }
}

/// Effect options (`EffectOptions`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EffectOptions {
    /// No projectile trail.
    pub no_projectile_trail: bool,
    /// Blood.
    pub blood: bool,
    /// Gibs.
    pub gibs: bool,
    /// Score plum.
    pub score_plum: bool,
    /// Hardware.
    pub hardware_rage_pro: bool,
}

/// Effect imports (`EffectImports`).
pub trait EffectImports {
    /// Cgame `rand()` (0..32767).
    fn random_integer(&mut self) -> i32;
    /// Start a sound.
    fn start_sound(&mut self, origin: Vec3, entity: i32, channel: i32, sound: Option<PresentSound>);
}

/// Smoke puff options (`SmokePuffOptions`).
#[derive(Debug, Clone, PartialEq)]
pub struct SmokePuffOptions {
    /// Origin.
    pub origin: Vec3,
    /// Velocity.
    pub velocity: Vec3,
    /// Radius.
    pub radius: f32,
    /// Color.
    pub color: Vec4,
    /// Duration.
    pub duration: i32,
    /// Start time.
    pub start_time: i32,
    /// Fade-in time.
    pub fade_in_time: i32,
    /// Flags.
    pub flags: i32,
    /// Shader.
    pub shader: Option<SceneShader>,
}

/// Explosion options (`ExplosionOptions`).
#[derive(Debug, Clone, PartialEq)]
pub struct ExplosionOptions {
    /// Origin.
    pub origin: Vec3,
    /// Direction.
    pub direction: Option<Vec3>,
    /// Model.
    pub model: SceneModel,
    /// Shader.
    pub shader: Option<SceneShader>,
    /// Duration.
    pub duration: i32,
    /// Sprite.
    pub sprite: bool,
}

fn effect_identity_axis() -> Axis {
    [vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)]
}

fn effect_seconds(time: i32) -> f32 {
    (time as f32) / 1000.0
}

fn effect_life_rate(start: i32, end: i32) -> f32 {
    1.0 / (end.wrapping_sub(start) as f32)
}

fn effect_byte(value: f32) -> f32 {
    (qvm_float_to_int(value * 255.0) & 255) as f32
}

/// Client effects (`ClientEffects`).
pub struct ClientEffects {
    /// Media.
    pub media: EffectMedia,
    /// Options.
    pub options: EffectOptions,
    /// Imports.
    pub imports: Box<dyn EffectImports>,
    smoke_seed: i32,
    last_score_position: Vec3,
}

impl ClientEffects {
    /// New effects.
    pub fn new(
        product: Product,
        pool_product: Product,
        media: EffectMedia,
        options: EffectOptions,
        imports: Box<dyn EffectImports>,
    ) -> PresentResult<Self> {
        if product != media.product() {
            return Err(PresentError::state("Effect media product differs from cgame product"));
        }
        if product != pool_product {
            return Err(PresentError::state("Effect pool product differs from cgame product"));
        }
        Ok(Self {
            media,
            options,
            imports,
            smoke_seed: 0x92,
            last_score_position: zero_vec3(),
        })
    }

    /// Product.
    #[must_use]
    pub fn product(&self) -> Product {
        self.media.product()
    }

    fn rand(&mut self) -> PresentResult<i32> {
        let value = self.imports.random_integer();
        if !(0..=32767).contains(&value) {
            return Err(PresentError::range("cgame rand() must return a 15-bit integer"));
        }
        Ok(value)
    }

    fn random(&mut self) -> PresentResult<f32> {
        Ok(((self.rand()? & 32767) as f32) / 32767.0)
    }

    fn crandom(&mut self) -> PresentResult<f32> {
        Ok(2.0 * (self.random()? - 0.5))
    }

    fn mission(&self) -> PresentResult<&MissionEffectMedia> {
        match &self.media.variant {
            EffectMediaVariant::Mission(media) => Ok(media),
            EffectMediaVariant::Base { .. } => Err(PresentError::state("Missionpack effect requested in baseq3")),
        }
    }

    /// Bubble trail (`bubbleTrail`).
    pub fn bubble_trail(
        &mut self,
        pool: &mut LocalEntityPool,
        frame: &EffectFrame,
        start: Vec3,
        end: Vec3,
        spacing: f32,
    ) -> PresentResult<()> {
        if self.options.no_projectile_trail {
            return Ok(());
        }
        if !spacing.is_finite() || spacing.trunc() < 1.0 || spacing > 2_147_483_647.0 {
            return Err(PresentError::range(
                "Bubble spacing must have a positive integer divisor",
            ));
        }
        let difference = sub3(end, start);
        let length = length3(difference);
        let direction = normalize3(difference);
        let mut i = (self.rand()? % spacing.trunc() as i32) as f32;
        let mut position = add3(start, scale3(direction, i));
        let step = scale3(direction, spacing);
        while i < length {
            let mut re = create_sprite_entity();
            let handle = pool.allocate(LocalEntityType::MoveScaleFade, RefEntity::Sprite(re.clone()))?;
            re.shading.shader_time = effect_seconds(frame.time);
            re.radius = 3.0;
            re.shading.custom_shader = self.media.water_bubble_shader.clone();
            re.shading.shader_rgba = vec4(255.0, 255.0, 255.0, 255.0);
            let random = self.random()?;
            let crandom = [self.crandom()?, self.crandom()?, self.crandom()?];
            if let Some(le) = pool.get_mut(handle) {
                le.le_flags = LE_PUFF_DONT_SCALE;
                le.start_time = frame.time;
                le.end_time = qvm_float_to_int(frame.time.wrapping_add(1000) as f32 + random * 250.0);
                le.life_rate = effect_life_rate(le.start_time, le.end_time);
                le.color = vec4(0.0, 0.0, 0.0, 1.0);
                le.pos = Trajectory {
                    type_: TrajectoryType::Linear,
                    time: frame.time,
                    duration: 0,
                    base: position,
                    delta: vec3(crandom[0] * 5.0, crandom[1] * 5.0, crandom[2] * 5.0 + 6.0),
                };
                le.ref_entity = RefEntity::Sprite(re);
            }
            position = add3(position, step);
            i = qvm_float_to_int(i + spacing) as f32;
        }
        Ok(())
    }

    /// Smoke puff (`smokePuff`).
    pub fn smoke_puff(
        &mut self,
        pool: &mut LocalEntityPool,
        _frame: &EffectFrame,
        options: &SmokePuffOptions,
    ) -> PresentResult<LocalEntityHandle> {
        let mut re = create_sprite_entity();
        let handle = pool.allocate(LocalEntityType::MoveScaleFade, RefEntity::Sprite(re.clone()))?;
        let rotation = q_random(self.smoke_seed);
        self.smoke_seed = rotation.seed;
        re.rotation = (rotation.value as f32) * 360.0;
        re.radius = options.radius;
        re.shading.shader_time = effect_seconds(options.start_time);
        re.origin = options.origin;
        re.shading.custom_shader = options.shader.clone();
        if self.options.hardware_rage_pro {
            re.shading.custom_shader = self.media.smoke_puff_rage_pro_shader.clone();
            re.shading.shader_rgba = vec4(255.0, 255.0, 255.0, 255.0);
        } else {
            re.shading.shader_rgba = vec4(
                effect_byte(options.color.x),
                effect_byte(options.color.y),
                effect_byte(options.color.z),
                255.0,
            );
        }
        if let Some(le) = pool.get_mut(handle) {
            le.le_flags = options.flags;
            le.radius = options.radius;
            le.start_time = options.start_time;
            le.fade_in_time = options.fade_in_time;
            le.end_time = qvm_float_to_int(le.start_time as f32 + options.duration as f32);
            le.life_rate = effect_life_rate(
                if le.fade_in_time > le.start_time {
                    le.fade_in_time
                } else {
                    le.start_time
                },
                le.end_time,
            );
            le.color = options.color;
            le.pos = Trajectory {
                type_: TrajectoryType::Linear,
                time: le.start_time,
                duration: 0,
                base: options.origin,
                delta: options.velocity,
            };
            le.ref_entity = RefEntity::Sprite(re);
        }
        Ok(handle)
    }

    /// Spawn effect (`spawnEffect`).
    pub fn spawn_effect(
        &mut self,
        pool: &mut LocalEntityPool,
        frame: &EffectFrame,
        origin: Vec3,
    ) -> PresentResult<LocalEntityHandle> {
        let mut re = create_model_entity(self.media.teleport_effect_model.clone());
        let handle = pool.allocate(LocalEntityType::FadeRgb, RefEntity::Model(re.clone()))?;
        let base = matches!(self.media.variant, EffectMediaVariant::Base { .. });
        re.shading.shader_time = effect_seconds(frame.time);
        re.axis = effect_identity_axis();
        re.origin = vec3(origin.x, origin.y, origin.z + if base { -24.0 } else { 16.0 });
        if let EffectMediaVariant::Base { teleport_effect_shader } = &self.media.variant {
            re.shading.custom_shader = teleport_effect_shader.clone();
        }
        if let Some(le) = pool.get_mut(handle) {
            le.start_time = frame.time;
            le.end_time = frame.time.wrapping_add(500);
            le.life_rate = effect_life_rate(le.start_time, le.end_time);
            le.color = vec4(1.0, 1.0, 1.0, 1.0);
            le.ref_entity = RefEntity::Model(re);
        }
        Ok(handle)
    }

    /// Make an explosion (`makeExplosion`).
    ///
    /// The donor merges model and sprite records for sprite explosions; the
    /// extra model/old-origin fields are never read by sprite-explosion
    /// processing, so sprite explosions are plain sprite records here.
    pub fn make_explosion(
        &mut self,
        pool: &mut LocalEntityPool,
        frame: &EffectFrame,
        options: &ExplosionOptions,
    ) -> PresentResult<LocalEntityHandle> {
        let duration = options.duration;
        if duration <= 0 {
            return Err(PresentError::range(format!("CG_MakeExplosion: msec = {duration}")));
        }
        if options.sprite && options.direction.is_none() {
            return Err(PresentError::state("Sprite explosion requires a direction vector"));
        }
        let offset = self.rand()? & 63;
        let handle = if options.sprite {
            let direction = options
                .direction
                .ok_or_else(|| PresentError::state("Sprite explosion requires a direction vector"))?;
            let mut re = create_sprite_entity();
            let handle = pool.allocate(LocalEntityType::SpriteExplosion, RefEntity::Sprite(re.clone()))?;
            re.rotation = (self.rand()? % 360) as f32;
            re.origin = add3(scale3(direction, 16.0), options.origin);
            if let Some(le) = pool.get_mut(handle) {
                le.ref_entity = RefEntity::Sprite(re);
            }
            handle
        } else {
            let mut re = create_model_entity(default_model());
            let handle = pool.allocate(LocalEntityType::Explosion, RefEntity::Model(re.clone()))?;
            if let Some(direction) = options.direction {
                let angle = self.rand()? % 360;
                let forward = direction;
                let perpendicular = perpendicular_vector(forward);
                let side = if angle == 0 {
                    perpendicular
                } else {
                    rotate_point_around_vector(forward, perpendicular, f64::from(angle))
                };
                re.axis = [forward, side, cross3(forward, side)];
            } else {
                re.axis = effect_identity_axis();
            }
            re.origin = options.origin;
            re.old_origin = options.origin;
            re.model = options.model.clone();
            if let Some(le) = pool.get_mut(handle) {
                le.ref_entity = RefEntity::Model(re);
            }
            handle
        };
        if let Some(le) = pool.get_mut(handle) {
            le.start_time = frame.time.wrapping_sub(offset);
            le.end_time = le.start_time.wrapping_add(duration);
            let shader_time = effect_seconds(le.start_time);
            let rgba = le.ref_entity_shading_rgba();
            set_shading_rgba(&mut le.ref_entity, rgba);
            set_shading_time(&mut le.ref_entity, shader_time);
            set_custom_shader(&mut le.ref_entity, options.shader.clone());
            le.color = vec4(1.0, 1.0, 1.0, 0.0);
        }
        Ok(handle)
    }

    /// Bleed (`bleed`).
    pub fn bleed(
        &mut self,
        pool: &mut LocalEntityPool,
        frame: &EffectFrame,
        origin: Vec3,
        entity_num: i32,
    ) -> PresentResult<()> {
        if !self.options.blood {
            return Ok(());
        }
        let Some(snap_client) = frame.snap_client else {
            return Err(PresentError::state("CG_Bleed requires a current snapshot"));
        };
        self.bleed_at(pool, frame, origin, entity_num == snap_client)?;
        Ok(())
    }

    /// Bleed at a point (`bleedAt`).
    pub fn bleed_at(
        &mut self,
        pool: &mut LocalEntityPool,
        frame: &EffectFrame,
        origin: Vec3,
        hide_in_first_person: bool,
    ) -> PresentResult<Option<RefSpriteEntity>> {
        if !self.options.blood {
            return Ok(None);
        }
        let mut re = create_sprite_entity();
        let handle = pool.allocate(LocalEntityType::Explosion, RefEntity::Sprite(re.clone()))?;
        re.origin = origin;
        re.rotation = (self.rand()? % 360) as f32;
        re.radius = 24.0;
        re.shading.custom_shader = self.media.blood_explosion_shader.clone();
        if hide_in_first_person {
            re.shading.render_flags |= RF_THIRD_PERSON;
        }
        if let Some(le) = pool.get_mut(handle) {
            le.start_time = frame.time;
            le.end_time = le.start_time.wrapping_add(500);
            le.ref_entity = RefEntity::Sprite(re.clone());
        }
        Ok(Some(re))
    }

    /// Launch a gib (`launchGib`).
    pub fn launch_gib(
        &mut self,
        pool: &mut LocalEntityPool,
        frame: &EffectFrame,
        origin: Vec3,
        velocity: Vec3,
        model: SceneModel,
    ) -> PresentResult<LocalEntityHandle> {
        let mut re = create_model_entity(model);
        let handle = pool.allocate(LocalEntityType::Fragment, RefEntity::Model(re.clone()))?;
        let random = self.random()?;
        re.origin = origin;
        re.axis = effect_identity_axis();
        if let Some(le) = pool.get_mut(handle) {
            le.start_time = frame.time;
            le.end_time = qvm_float_to_int(le.start_time.wrapping_add(5000) as f32 + random * 3000.0);
            le.pos = Trajectory {
                type_: TrajectoryType::Gravity,
                time: frame.time,
                duration: 0,
                base: origin,
                delta: velocity,
            };
            le.bounce_factor = 0.6;
            le.le_bounce_sound_type = LocalBounceSoundType::Blood;
            le.le_mark_type = LocalMarkType::Blood;
            le.ref_entity = RefEntity::Model(re);
        }
        Ok(handle)
    }

    /// Gib a player (`gibPlayer`).
    pub fn gib_player(&mut self, pool: &mut LocalEntityPool, frame: &EffectFrame, origin: Vec3) -> PresentResult<()> {
        if !self.options.blood {
            return Ok(());
        }
        let first_velocity = vec3(
            self.crandom()? * 250.0,
            self.crandom()? * 250.0,
            250.0 + self.crandom()? * 250.0,
        );
        let head = if self.rand()? & 1 != 0 {
            self.media.gib_skull.clone()
        } else {
            self.media.gib_brain.clone()
        };
        self.launch_gib(pool, frame, origin, first_velocity, head)?;
        if !self.options.gibs {
            return Ok(());
        }
        let models = [
            self.media.gib_abdomen.clone(),
            self.media.gib_arm.clone(),
            self.media.gib_chest.clone(),
            self.media.gib_fist.clone(),
            self.media.gib_foot.clone(),
            self.media.gib_forearm.clone(),
            self.media.gib_intestine.clone(),
            self.media.gib_leg.clone(),
            self.media.gib_leg.clone(),
        ];
        for model in models {
            let next = vec3(
                self.crandom()? * 250.0,
                self.crandom()? * 250.0,
                250.0 + self.crandom()? * 250.0,
            );
            self.launch_gib(pool, frame, origin, next, model)?;
        }
        Ok(())
    }

    /// Launch debris (`launchExplode`).
    pub fn launch_explode(
        &mut self,
        pool: &mut LocalEntityPool,
        frame: &EffectFrame,
        origin: Vec3,
        velocity: Vec3,
        model: SceneModel,
    ) -> PresentResult<LocalEntityHandle> {
        let mut re = create_model_entity(model);
        let handle = pool.allocate(LocalEntityType::Fragment, RefEntity::Model(re.clone()))?;
        let random = self.random()?;
        re.origin = origin;
        re.axis = effect_identity_axis();
        if let Some(le) = pool.get_mut(handle) {
            le.start_time = frame.time;
            le.end_time = qvm_float_to_int(le.start_time.wrapping_add(10000) as f32 + random * 6000.0);
            le.pos = Trajectory {
                type_: TrajectoryType::Gravity,
                time: frame.time,
                duration: 0,
                base: origin,
                delta: velocity,
            };
            le.bounce_factor = 0.1;
            le.le_bounce_sound_type = LocalBounceSoundType::Brass;
            le.le_mark_type = LocalMarkType::None;
            le.ref_entity = RefEntity::Model(re);
        }
        Ok(handle)
    }

    /// Big explosion debris (`bigExplode`).
    pub fn big_explode(&mut self, pool: &mut LocalEntityPool, frame: &EffectFrame, origin: Vec3) -> PresentResult<()> {
        if !self.options.blood {
            return Ok(());
        }
        for scale in [1.0, 1.0, 1.5, 2.0, 2.5] {
            let velocity = vec3(
                self.crandom()? * 100.0 * scale,
                self.crandom()? * 100.0 * scale,
                150.0 + self.crandom()? * 100.0,
            );
            self.launch_explode(pool, frame, origin, velocity, self.media.smoke2.clone())?;
        }
        Ok(())
    }

    /// Score plum (`scorePlum`).
    pub fn score_plum(
        &mut self,
        pool: &mut LocalEntityPool,
        frame: &EffectFrame,
        client: i32,
        origin: Vec3,
        score: i32,
    ) -> PresentResult<()> {
        if client != frame.predicted_client || !self.options.score_plum {
            return Ok(());
        }
        let re = create_sprite_entity();
        let handle = pool.allocate(LocalEntityType::ScorePlum, RefEntity::Sprite(re))?;
        let z = origin.z;
        let last_z = self.last_score_position.z;
        let base = vec3(
            origin.x,
            origin.y,
            if z >= last_z - 20.0 && z <= last_z + 20.0 {
                z - 20.0
            } else {
                z
            },
        );
        if let Some(le) = pool.get_mut(handle) {
            le.start_time = frame.time;
            le.end_time = frame.time.wrapping_add(4000);
            le.life_rate = effect_life_rate(le.start_time, le.end_time);
            le.color = vec4(1.0, 1.0, 1.0, 1.0);
            le.radius = score as f32;
            le.pos.base = base;
            if let RefEntity::Sprite(re) = &mut le.ref_entity {
                re.radius = 16.0;
            }
        }
        self.last_score_position = origin;
        Ok(())
    }

    /// Lightning bolt beam (`lightningBoltBeam`).
    pub fn lightning_bolt_beam(
        &mut self,
        pool: &mut LocalEntityPool,
        frame: &EffectFrame,
        start: Vec3,
        end: Vec3,
    ) -> PresentResult<LocalEntityHandle> {
        let shader = self.mission()?.lightning_shader.clone();
        let mut re = create_lightning_entity();
        let handle = pool.allocate(LocalEntityType::ShowRefEntity, RefEntity::Lightning(re.clone()))?;
        re.origin = start;
        re.old_origin = end;
        re.shading.custom_shader = shader;
        if let Some(le) = pool.get_mut(handle) {
            le.start_time = frame.time;
            le.end_time = frame.time.wrapping_add(50);
            le.ref_entity = RefEntity::Lightning(re);
        }
        Ok(handle)
    }

    /// Kamikaze effect (`kamikazeEffect`).
    pub fn kamikaze_effect(
        &mut self,
        pool: &mut LocalEntityPool,
        frame: &EffectFrame,
        origin: Vec3,
    ) -> PresentResult<LocalEntityHandle> {
        let model = self.mission()?.kamikaze_effect_model.clone();
        let mut re = create_model_entity(model);
        let handle = pool.allocate(LocalEntityType::Kamikaze, RefEntity::Model(re.clone()))?;
        re.shading.shader_time = effect_seconds(frame.time);
        re.origin = origin;
        if let Some(le) = pool.get_mut(handle) {
            le.start_time = frame.time;
            le.end_time = frame.time.wrapping_add(3000);
            le.life_rate = effect_life_rate(le.start_time, le.end_time);
            le.color = vec4(1.0, 1.0, 1.0, 1.0);
            le.ref_entity = RefEntity::Model(re);
        }
        Ok(handle)
    }

    /// Obelisk explosion (`obeliskExplode`).
    pub fn obelisk_explode(
        &mut self,
        pool: &mut LocalEntityPool,
        frame: &EffectFrame,
        origin: Vec3,
    ) -> PresentResult<()> {
        let media = self.mission()?.clone();
        let handle = self.make_explosion(
            pool,
            frame,
            &ExplosionOptions {
                origin: vec3(origin.x, origin.y, origin.z + 64.0),
                direction: Some(zero_vec3()),
                model: media.dish_flash_model,
                shader: media.rocket_explosion_shader,
                duration: 600,
                sprite: true,
            },
        )?;
        if let Some(le) = pool.get_mut(handle) {
            le.light = 300.0;
            le.light_color = vec3(1.0, 0.75, 0.0);
        }
        Ok(())
    }

    fn hit_sound(&mut self, sounds: &[Option<PresentSound>; 3]) -> PresentResult<Option<PresentSound>> {
        let choice = self.rand()? & 3;
        Ok(if choice < 2 {
            sounds[0].clone()
        } else if choice == 2 {
            sounds[1].clone()
        } else {
            sounds[2].clone()
        })
    }

    /// Obelisk pain (`obeliskPain`).
    pub fn obelisk_pain(&mut self, _pool: &mut LocalEntityPool, origin: Vec3) -> PresentResult<()> {
        let sounds = self.mission()?.obelisk_hit_sounds.clone();
        let sound = self.hit_sound(&sounds)?;
        self.imports.start_sound(origin, 1023, 5, sound);
        Ok(())
    }

    /// Invulnerability impact (`invulnerabilityImpact`).
    pub fn invulnerability_impact(
        &mut self,
        pool: &mut LocalEntityPool,
        frame: &EffectFrame,
        origin: Vec3,
        angles: Vec3,
    ) -> PresentResult<LocalEntityHandle> {
        let media = self.mission()?.clone();
        let mut re = create_model_entity(media.invulnerability_impact_model);
        let handle = pool.allocate(LocalEntityType::InvulImpact, RefEntity::Model(re.clone()))?;
        re.shading.shader_time = effect_seconds(frame.time);
        re.origin = origin;
        re.axis = angles_to_axis(angles);
        if let Some(le) = pool.get_mut(handle) {
            le.start_time = frame.time;
            le.end_time = frame.time.wrapping_add(1000);
            le.life_rate = effect_life_rate(le.start_time, le.end_time);
            le.color = vec4(1.0, 1.0, 1.0, 1.0);
            le.ref_entity = RefEntity::Model(re);
        }
        let sound = self.hit_sound(&media.invulnerability_impact_sounds)?;
        self.imports.start_sound(origin, 1023, 5, sound);
        Ok(handle)
    }

    /// Invulnerability juiced (`invulnerabilityJuiced`).
    pub fn invulnerability_juiced(
        &mut self,
        pool: &mut LocalEntityPool,
        frame: &EffectFrame,
        origin: Vec3,
    ) -> PresentResult<LocalEntityHandle> {
        let media = self.mission()?.clone();
        let mut re = create_model_entity(media.invulnerability_juiced_model);
        let handle = pool.allocate(LocalEntityType::InvulJuiced, RefEntity::Model(re.clone()))?;
        re.shading.shader_time = effect_seconds(frame.time);
        re.origin = origin;
        re.axis = angles_to_axis(zero_vec3());
        if let Some(le) = pool.get_mut(handle) {
            le.start_time = frame.time;
            le.end_time = frame.time.wrapping_add(10000);
            le.life_rate = effect_life_rate(le.start_time, le.end_time);
            le.color = vec4(1.0, 1.0, 1.0, 1.0);
            le.ref_entity = RefEntity::Model(re);
        }
        self.imports
            .start_sound(origin, 1023, 5, media.invulnerability_juiced_sound);
        Ok(handle)
    }
}

trait RefEntityShading {
    fn ref_entity_shading_rgba(&self) -> Vec4;
}

impl RefEntityShading for LocalEntity {
    fn ref_entity_shading_rgba(&self) -> Vec4 {
        match &self.ref_entity {
            RefEntity::Model(re) => re.shading.shader_rgba,
            RefEntity::Sprite(re) => re.shading.shader_rgba,
            RefEntity::Beam(re) => re.shading.shader_rgba,
            RefEntity::RailCore(re) => re.shading.shader_rgba,
            RefEntity::RailRings(re) => re.shading.shader_rgba,
            RefEntity::Lightning(re) => re.shading.shader_rgba,
            RefEntity::Portal(_) => vec4(0.0, 0.0, 0.0, 0.0),
        }
    }
}

fn set_shading_time(entity: &mut RefEntity, time: f32) {
    match entity {
        RefEntity::Model(re) => re.shading.shader_time = time,
        RefEntity::Sprite(re) => re.shading.shader_time = time,
        RefEntity::Beam(re) => re.shading.shader_time = time,
        RefEntity::RailCore(re) => re.shading.shader_time = time,
        RefEntity::RailRings(re) => re.shading.shader_time = time,
        RefEntity::Lightning(re) => re.shading.shader_time = time,
        RefEntity::Portal(_) => {}
    }
}

fn set_custom_shader(entity: &mut RefEntity, shader: Option<SceneShader>) {
    match entity {
        RefEntity::Model(re) => re.shading.custom_shader = shader,
        RefEntity::Sprite(re) => re.shading.custom_shader = shader,
        RefEntity::Beam(re) => re.shading.custom_shader = shader,
        RefEntity::RailCore(re) => re.shading.custom_shader = shader,
        RefEntity::RailRings(re) => re.shading.custom_shader = shader,
        RefEntity::Lightning(re) => re.shading.custom_shader = shader,
        RefEntity::Portal(_) => {}
    }
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
const BYTE_DIRECTIONS: [[f32; 3]; 162] = [
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

// ---------------------------------------------------------------------------
// entities.ts
// ---------------------------------------------------------------------------

/// Solid brush-model marker.
pub const SOLID_BMODEL: i32 = 0xffffff;
/// Item channel.
pub const CHAN_ITEM: i32 = 4;
/// Body channel.
pub const CHAN_BODY: i32 = 5;

/// Missile trail kind (`MissileTrail`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MissileTrail {
    /// Rocket.
    Rocket,
    /// Grenade.
    Grenade,
    /// Grapple.
    Grapple,
    /// Nail.
    Nail,
    /// Plasma.
    Plasma,
}

/// Packet weapon info (`PacketWeaponInfo`).
#[derive(Debug, Clone, PartialEq)]
pub struct PacketWeaponInfo {
    /// Weapon model.
    pub weapon_model: SceneModel,
    /// Weapon midpoint.
    pub weapon_midpoint: Vec3,
    /// Barrel model.
    pub barrel_model: Option<SceneModel>,
    /// Missile model.
    pub missile_model: SceneModel,
    /// Missile render effects.
    pub missile_renderfx: i32,
    /// Missile sound.
    pub missile_sound: Option<PresentSound>,
    /// Missile dynamic light.
    pub missile_dlight: f32,
    /// Missile light color.
    pub missile_dlight_color: Vec3,
    /// Missile trail.
    pub missile_trail: Option<MissileTrail>,
    /// Trail radius.
    pub trail_radius: f32,
    /// Trail time.
    pub trail_time: i32,
}

/// Packet item visual (`PacketItemVisual`).
#[derive(Debug, Clone, PartialEq)]
pub struct PacketItemVisual {
    /// Models.
    pub models: [SceneModel; 2],
    /// Second model present.
    pub has_second: bool,
    /// Icon.
    pub icon: Option<SceneShader>,
}

/// Packet mission media (`PacketMissionMedia`).
#[derive(Debug, Clone, PartialEq)]
pub struct PacketMissionMedia {
    /// Weapon hover sound.
    pub weapon_hover_sound: Option<PresentSound>,
    /// Blue prox mine.
    pub blue_prox_mine: SceneModel,
    /// Overload base model.
    pub overload_base_model: SceneModel,
    /// Overload energy model.
    pub overload_energy_model: SceneModel,
    /// Overload lights model.
    pub overload_lights_model: SceneModel,
    /// Overload target model.
    pub overload_target_model: SceneModel,
    /// Obelisk respawn sound.
    pub obelisk_respawn_sound: Option<PresentSound>,
    /// Harvester model.
    pub harvester_model: SceneModel,
    /// Harvester neutral model.
    pub harvester_neutral_model: SceneModel,
    /// Harvester red skin.
    pub harvester_red_skin: Option<SceneSkin>,
    /// Harvester blue skin.
    pub harvester_blue_skin: Option<SceneSkin>,
}

/// Packet entity media variant.
#[derive(Debug, Clone, PartialEq)]
pub enum PacketEntityMediaVariant {
    /// Base.
    Base,
    /// Mission.
    Mission(PacketMissionMedia),
}

/// Inline model entry.
#[derive(Debug, Clone, PartialEq)]
pub struct InlineModelEntry {
    /// Model.
    pub model: SceneModel,
    /// Midpoint.
    pub midpoint: Vec3,
}

/// Packet entity media (`PacketEntityMedia`).
#[derive(Debug, Clone, PartialEq)]
pub struct PacketEntityMedia {
    /// Game models.
    pub game_models: Vec<SceneModel>,
    /// Game sounds.
    pub game_sounds: Vec<Option<PresentSound>>,
    /// Inline models.
    pub inline_models: Vec<InlineModelEntry>,
    /// Items.
    pub items: Vec<PacketItemVisual>,
    /// Weapons.
    pub weapons: Vec<PacketWeaponInfo>,
    /// Plasma ball shader.
    pub plasma_ball_shader: Option<SceneShader>,
    /// Red flag base model.
    pub red_flag_base_model: SceneModel,
    /// Blue flag base model.
    pub blue_flag_base_model: SceneModel,
    /// Neutral flag base model.
    pub neutral_flag_base_model: SceneModel,
    /// Variant.
    pub variant: PacketEntityMediaVariant,
}

impl PacketEntityMedia {
    /// Product.
    #[must_use]
    pub fn product(&self) -> Product {
        match &self.variant {
            PacketEntityMediaVariant::Base => Product::BaseQ3,
            PacketEntityMediaVariant::Mission(_) => Product::MissionPack,
        }
    }
}

/// Packet entity imports (`PacketEntityImports`).
pub trait PacketEntityImports {
    /// Whether an entity body is hidden.
    fn body_hidden(&self, _entity: i32) -> bool {
        false
    }
    /// Pose an entity.
    fn pose_entity(&mut self, _entity: &mut ClientEntity) {}
    /// Add a reference entity.
    fn add_ref_entity(&mut self, entity: RefEntity);
    /// Add a light.
    fn add_light(&mut self, light: DynamicLight);
    /// Update a sound position.
    fn update_sound_position(&mut self, entity: i32, origin: Vec3);
    /// Add a loop sound.
    fn add_loop_sound(
        &mut self,
        entity: i32,
        origin: Vec3,
        velocity: Vec3,
        sound: Option<PresentSound>,
        real_loop: bool,
    );
    /// Start a sound.
    fn start_sound(&mut self, origin: Option<Vec3>, entity: i32, channel: i32, sound: Option<PresentSound>);
    /// Shared cgame `rand()`.
    fn random_integer(&mut self) -> i32;
    /// Present a player.
    fn present_player(&mut self, entity: &ClientEntity);
    /// Missile trail.
    fn missile_trail(&mut self, kind: MissileTrail, entity: &ClientEntity, weapon: &PacketWeaponInfo);
    /// Grapple trail.
    fn grapple_trail(&mut self, entity: &ClientEntity, weapon: &PacketWeaponInfo);
    /// Add an entity with powerups.
    fn add_entity_with_powerups(&mut self, entity: RefModelEntity, state: &EntityState, team: Team);
}

/// Packet entity options (`PacketEntityOptions`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PacketEntityOptions {
    /// Game type.
    pub game_type: GameType,
    /// Smooth clients.
    pub smooth_clients: bool,
    /// Simple items.
    pub simple_items: bool,
    /// Obelisk respawn delay.
    pub obelisk_respawn_delay: i32,
}

fn indexed<T: Clone>(values: &[T], index: i32, name: &str) -> PresentResult<T> {
    values
        .get(index as usize)
        .cloned()
        .ok_or_else(|| PresentError::range(format!("{name} index {index} is not registered")))
}

fn identity_axis_full() -> Axis {
    [vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)]
}

fn scale_axis_full(axis: &Axis, scale: f32) -> Axis {
    [scale3(axis[0], scale), scale3(axis[1], scale), scale3(axis[2], scale)]
}

fn multiply_axis(left: &Axis, right: &Axis) -> Axis {
    let x = vec3(right[0].x, right[1].x, right[2].x);
    let y = vec3(right[0].y, right[1].y, right[2].y);
    let z = vec3(right[0].z, right[1].z, right[2].z);
    [
        vec3(dot3(left[0], x), dot3(left[0], y), dot3(left[0], z)),
        vec3(dot3(left[1], x), dot3(left[1], y), dot3(left[1], z)),
        vec3(dot3(left[2], x), dot3(left[2], y), dot3(left[2], z)),
    ]
}

fn tag_orientation(parent: &RefModelEntity, model: &SceneModel, name: &str) -> (Vec3, Axis) {
    match lerp_model_tag(model, name, parent.old_frame, parent.frame, 1.0 - parent.back_lerp) {
        None => (zero_vec3(), identity_axis_full()),
        Some(tag) => (tag.origin, tag.axes),
    }
}

fn tag_origin(parent: &RefModelEntity, tag: Vec3) -> Vec3 {
    let mut origin = add3(parent.origin, scale3(parent.axis[0], tag.x));
    origin = add3(origin, scale3(parent.axis[1], tag.y));
    add3(origin, scale3(parent.axis[2], tag.z))
}

/// Position an entity on a tag (`positionEntityOnTag`).
pub fn position_entity_on_tag(
    entity: &mut RefModelEntity,
    parent: &RefModelEntity,
    parent_model: &SceneModel,
    tag_name: &str,
) {
    let (origin, axis) = tag_orientation(parent, parent_model, tag_name);
    entity.origin = tag_origin(parent, origin);
    entity.axis = multiply_axis(&axis, &parent.axis);
    entity.back_lerp = parent.back_lerp;
}

/// Position a rotated entity on a tag (`positionRotatedEntityOnTag`).
pub fn position_rotated_entity_on_tag(
    entity: &mut RefModelEntity,
    parent: &RefModelEntity,
    parent_model: &SceneModel,
    tag_name: &str,
) {
    let (origin, axis) = tag_orientation(parent, parent_model, tag_name);
    entity.origin = tag_origin(parent, origin);
    entity.axis = multiply_axis(&multiply_axis(&entity.axis, &axis), &parent.axis);
}

/// Adjust a position for a mover (`adjustPositionForMover`).
#[must_use]
pub fn adjust_position_for_mover(
    state: &ClientGameState,
    input: Vec3,
    mover_num: i32,
    from_time: i32,
    to_time: i32,
) -> Vec3 {
    if mover_num <= 0 || mover_num >= ENTITYNUM_WORLD {
        return input;
    }
    let Some(mover) = state.entity_ref(mover_num as usize) else {
        return input;
    };
    let current = mover.current_state.clone();
    if current.e_type != EntityType::Mover as i32 {
        return input;
    }
    let old_origin = evaluate_trajectory(&current.pos, from_time);
    let _ = evaluate_trajectory(&current.apos, from_time);
    let origin = evaluate_trajectory(&current.pos, to_time);
    let _ = evaluate_trajectory(&current.apos, to_time);
    add3(input, sub3(origin, old_origin))
}

fn entity_lerp_angle(from: f32, to: f32, fraction: f32) -> f32 {
    let mut to = to;
    if to - from > 180.0 {
        to -= 360.0;
    }
    if to - from < -180.0 {
        to += 360.0;
    }
    from + fraction * (to - from)
}

fn entity_interpolate(from: Vec3, to: Vec3, fraction: f32) -> Vec3 {
    add3(from, scale3(sub3(to, from), fraction))
}

fn direction_axis(direction: Vec3, yaw: f32) -> Axis {
    let mut side = perpendicular_vector(direction);
    if yaw != 0.0 {
        side = rotate_point_around_vector(direction, side, f64::from(yaw));
    }
    [direction, side, cross3(direction, side)]
}

fn missile_direction(delta: Vec3) -> Vec3 {
    if length3(delta) == 0.0 {
        vec3(0.0, 0.0, 1.0)
    } else {
        normalize3_or_zero(delta)
    }
}

/// Packet entity presenter (`PacketEntityPresenter`).
pub struct PacketEntityPresenter {
    /// Media.
    pub media: PacketEntityMedia,
    /// Imports.
    pub imports: Box<dyn PacketEntityImports>,
    /// Item table.
    pub items: Box<dyn PresentItemTable>,
}

impl PacketEntityPresenter {
    /// New presenter.
    pub fn new(
        product: Product,
        media: PacketEntityMedia,
        imports: Box<dyn PacketEntityImports>,
        items: Box<dyn PresentItemTable>,
    ) -> PresentResult<Self> {
        if product != media.product() {
            return Err(PresentError::state(
                "packet entity media product differs from cgame state",
            ));
        }
        Ok(Self { media, imports, items })
    }

    fn body(&mut self, number: i32, reference: RefEntity) {
        if !self.imports.body_hidden(number) {
            self.imports.add_ref_entity(reference);
        }
    }

    /// Set an entity sound position (`setEntitySoundPosition`).
    pub fn set_entity_sound_position(
        &mut self,
        state: &mut ClientGameState,
        target: PresentEntityTarget,
    ) -> PresentResult<()> {
        let (number, origin, solid, modelindex) = match target {
            PresentEntityTarget::Predicted => {
                let entity = &state.predicted_player_entity;
                (
                    entity.current_state.number,
                    entity.lerp_origin,
                    entity.current_state.solid,
                    entity.current_state.modelindex,
                )
            }
            PresentEntityTarget::Indexed(index) => {
                let entity = state.entity_at(index);
                (
                    entity.current_state.number,
                    entity.lerp_origin,
                    entity.current_state.solid,
                    entity.current_state.modelindex,
                )
            }
        };
        let origin = if solid == SOLID_BMODEL {
            add3(
                origin,
                indexed(&self.media.inline_models, modelindex, "inline model")?.midpoint,
            )
        } else {
            origin
        };
        self.imports.update_sound_position(number, origin);
        Ok(())
    }

    /// Calculate lerp positions (`calculateLerpPositions`).
    pub fn calculate_lerp_positions(
        &mut self,
        state: &mut ClientGameState,
        target: PresentEntityTarget,
        smooth_clients: bool,
    ) -> PresentResult<()> {
        let snap_time = state
            .snap
            .as_ref()
            .ok_or_else(|| PresentError::state("CG_CalcEntityLerpPositions: cg.snap == NULL"))?
            .server_time;
        let time = state.time;
        let frame_interpolation = state.frame_interpolation;
        // Borrow the entity once for flag/type updates.
        let (number, interpolate, pos_type) = {
            let entity = match target {
                PresentEntityTarget::Predicted => &mut state.predicted_player_entity,
                PresentEntityTarget::Indexed(index) => state.entity_at(index),
            };
            if !smooth_clients && entity.current_state.number < 64 {
                entity.current_state.pos.type_ = TrajectoryType::Interpolate;
                entity.next_state.pos.type_ = TrajectoryType::Interpolate;
            }
            (
                entity.current_state.number,
                entity.interpolate,
                entity.current_state.pos.type_,
            )
        };
        if interpolate
            && (pos_type == TrajectoryType::Interpolate || (pos_type == TrajectoryType::LinearStop && number < 64))
        {
            let next_time = state
                .next_snap
                .as_ref()
                .ok_or_else(|| PresentError::drop("CG_InterpoateEntityPosition: cg.nextSnap == NULL"))?
                .server_time;
            let entity = match target {
                PresentEntityTarget::Predicted => &mut state.predicted_player_entity,
                PresentEntityTarget::Indexed(index) => state.entity_at(index),
            };
            let from_pos = evaluate_trajectory(&entity.current_state.pos, snap_time);
            let to_pos = evaluate_trajectory(&entity.next_state.pos, next_time);
            entity.lerp_origin = entity_interpolate(from_pos, to_pos, frame_interpolation);
            let a = evaluate_trajectory(&entity.current_state.apos, snap_time);
            let b = evaluate_trajectory(&entity.next_state.apos, next_time);
            entity.lerp_angles = vec3(
                entity_lerp_angle(a.x, b.x, frame_interpolation),
                entity_lerp_angle(a.y, b.y, frame_interpolation),
                entity_lerp_angle(a.z, b.z, frame_interpolation),
            );
            return Ok(());
        }
        let (pos, apos, ground) = {
            let entity = match target {
                PresentEntityTarget::Predicted => &state.predicted_player_entity,
                PresentEntityTarget::Indexed(index) => state.entity_at(index),
            };
            (
                entity.current_state.pos,
                entity.current_state.apos,
                entity.current_state.ground_entity_num,
            )
        };
        let mut lerp_origin = evaluate_trajectory(&pos, time);
        let lerp_angles = evaluate_trajectory(&apos, time);
        if !matches!(target, PresentEntityTarget::Predicted) {
            lerp_origin = adjust_position_for_mover(state, lerp_origin, ground, snap_time, time);
        }
        let entity = match target {
            PresentEntityTarget::Predicted => &mut state.predicted_player_entity,
            PresentEntityTarget::Indexed(index) => state.entity_at(index),
        };
        entity.lerp_origin = lerp_origin;
        entity.lerp_angles = lerp_angles;
        Ok(())
    }

    fn effects(&mut self, state: &mut ClientGameState, target: PresentEntityTarget) -> PresentResult<()> {
        self.set_entity_sound_position(state, target)?;
        let (number, loop_sound, e_type, lerp_origin, constant_light) = {
            let entity = match target {
                PresentEntityTarget::Predicted => &state.predicted_player_entity,
                PresentEntityTarget::Indexed(index) => state.entity_at(index),
            };
            (
                entity.current_state.number,
                entity.current_state.loop_sound,
                entity.current_state.e_type,
                entity.lerp_origin,
                entity.current_state.constant_light,
            )
        };
        if loop_sound != 0 {
            let sound = indexed(&self.media.game_sounds, loop_sound, "sound")?;
            self.imports.add_loop_sound(
                number,
                lerp_origin,
                zero_vec3(),
                sound,
                e_type == EntityType::Speaker as i32,
            );
        }
        if constant_light != 0 {
            let light = constant_light as u32;
            self.imports.add_light(DynamicLight {
                origin: lerp_origin,
                radius: (((light >> 24) & 255) * 4) as f32,
                color: vec3(
                    (light & 255) as f32,
                    ((light >> 8) & 255) as f32,
                    ((light >> 16) & 255) as f32,
                ),
                additive: false,
            });
        }
        Ok(())
    }

    fn general(&mut self, state: &mut ClientGameState, target: PresentEntityTarget) -> PresentResult<()> {
        let (modelindex, frame, lerp_origin, lerp_angles, number) = {
            let entity = match target {
                PresentEntityTarget::Predicted => &state.predicted_player_entity,
                PresentEntityTarget::Indexed(index) => state.entity_at(index),
            };
            (
                entity.current_state.modelindex,
                entity.current_state.frame,
                entity.lerp_origin,
                entity.lerp_angles,
                entity.current_state.number,
            )
        };
        if modelindex == 0 {
            return Ok(());
        }
        let mut re = create_model_entity(indexed(&self.media.game_models, modelindex, "game model")?);
        re.frame = frame;
        re.old_frame = frame;
        re.origin = lerp_origin;
        re.old_origin = lerp_origin;
        if state
            .snap
            .as_ref()
            .is_some_and(|snap| number == snap.player_state.client_num)
        {
            re.shading.render_flags |= RF_THIRD_PERSON;
        }
        re.axis = angles_to_axis(lerp_angles);
        self.body(number, RefEntity::Model(re));
        Ok(())
    }

    fn speaker(&mut self, state: &mut ClientGameState, target: PresentEntityTarget) -> PresentResult<()> {
        let (client_num, number, event_parm, frame, misc_time) = {
            let entity = match target {
                PresentEntityTarget::Predicted => &state.predicted_player_entity,
                PresentEntityTarget::Indexed(index) => state.entity_at(index),
            };
            (
                entity.current_state.client_num,
                entity.current_state.number,
                entity.current_state.event_parm,
                entity.current_state.frame,
                entity.misc_time,
            )
        };
        if client_num == 0 || state.time < misc_time {
            return Ok(());
        }
        let sound = indexed(&self.media.game_sounds, event_parm, "sound")?;
        self.imports.start_sound(None, number, CHAN_ITEM, sound);
        let random = ((self.imports.random_integer() & 0x7fff) as f32) / 0x7fff as f32;
        let crandom = 2.0 * (random - 0.5);
        let misc = qvm_float_to_int(
            state.time.wrapping_add(frame.wrapping_mul(100)) as f32 + client_num.wrapping_mul(100) as f32 * crandom,
        );
        let entity = match target {
            PresentEntityTarget::Predicted => &mut state.predicted_player_entity,
            PresentEntityTarget::Indexed(index) => state.entity_at(index),
        };
        entity.misc_time = misc;
        Ok(())
    }

    fn item(
        &mut self,
        state: &mut ClientGameState,
        target: PresentEntityTarget,
        options: &PacketEntityOptions,
    ) -> PresentResult<()> {
        let (modelindex, number, e_flags, misc_time) = {
            let entity = match target {
                PresentEntityTarget::Predicted => &state.predicted_player_entity,
                PresentEntityTarget::Indexed(index) => state.entity_at(index),
            };
            (
                entity.current_state.modelindex,
                entity.current_state.number,
                entity.current_state.e_flags,
                entity.misc_time,
            )
        };
        if modelindex as usize >= self.items.item_count(state.product) {
            return Err(PresentError::drop(format!("Bad item index {modelindex} on entity")));
        }
        if modelindex == 0 || e_flags & 0x80 != 0 {
            return Ok(());
        }
        let item = self
            .items
            .item_at(state.product, modelindex as usize)
            .ok_or_else(|| PresentError::drop(format!("Bad item index {modelindex} on entity")))?;
        let visual = indexed(&self.media.items, modelindex, "item visual")?;
        if options.simple_items && item.item_type != ItemType::Team {
            let lerp_origin = match target {
                PresentEntityTarget::Predicted => state.predicted_player_entity.lerp_origin,
                PresentEntityTarget::Indexed(index) => state.entity_at(index).lerp_origin,
            };
            let mut re = create_sprite_entity();
            re.origin = lerp_origin;
            re.radius = 14.0;
            re.shading.custom_shader = visual.icon.clone();
            re.shading.shader_rgba = vec4(255.0, 255.0, 255.0, 255.0);
            self.body(number, RefEntity::Sprite(re));
            return Ok(());
        }
        let scale = 0.005 + number as f32 * 0.00001;
        let bob = 4.0 + (((state.time + 1000) as f32) * scale).cos() * 4.0;
        let fast = item.item_type == ItemType::Health;
        let (angles, axis) = if fast {
            (state.auto_angles_fast, state.auto_axis_fast)
        } else {
            (state.auto_angles, state.auto_axis)
        };
        let weapon = if item.item_type == ItemType::Weapon {
            Some(indexed(&self.media.weapons, item.tag, "weapon")?)
        } else {
            None
        };
        let mut re = create_model_entity(visual.models[0].clone());
        {
            let entity = match target {
                PresentEntityTarget::Predicted => &mut state.predicted_player_entity,
                PresentEntityTarget::Indexed(index) => state.entity_at(index),
            };
            entity.lerp_origin = add3(entity.lerp_origin, vec3(0.0, 0.0, bob));
            entity.lerp_angles = angles;
            re.axis = axis;
            if let Some(weapon) = &weapon {
                let midpoint = weapon.weapon_midpoint;
                let offset = add3(
                    add3(scale3(re.axis[0], midpoint.x), scale3(re.axis[1], midpoint.y)),
                    scale3(re.axis[2], midpoint.z),
                );
                entity.lerp_origin = add3(sub3(entity.lerp_origin, offset), vec3(0.0, 0.0, 8.0));
            }
            re.origin = entity.lerp_origin;
            re.old_origin = entity.lerp_origin;
        }
        let msec = state.time.wrapping_sub(misc_time);
        let mut fraction = 1.0f32;
        if msec >= 0 && msec < 1000 {
            fraction = msec as f32 / 1000.0;
            re.axis = scale_axis_full(&re.axis, fraction);
            re.non_normalized_axes = true;
        }
        if item.item_type == ItemType::Weapon || item.item_type == ItemType::Armor {
            re.shading.render_flags |= RF_MINLIGHT;
        }
        if item.item_type == ItemType::Weapon {
            re.axis = scale_axis_full(&re.axis, 1.5);
            re.non_normalized_axes = true;
            if let PacketEntityMediaVariant::Mission(media) = &self.media.variant {
                let lerp_origin = match target {
                    PresentEntityTarget::Predicted => state.predicted_player_entity.lerp_origin,
                    PresentEntityTarget::Indexed(index) => state.entity_at(index).lerp_origin,
                };
                self.imports.add_loop_sound(
                    number,
                    lerp_origin,
                    zero_vec3(),
                    media.weapon_hover_sound.clone(),
                    false,
                );
            }
        }
        if self.media.product() == Product::MissionPack
            && item.item_type == ItemType::Holdable
            && item.tag == Holdable::Kamikaze as i32
        {
            re.axis = scale_axis_full(&re.axis, 2.0);
            re.non_normalized_axes = true;
        }
        self.body(number, RefEntity::Model(re.clone()));
        if self.media.product() == Product::MissionPack {
            if let Some(weapon) = &weapon {
                if let Some(barrel_model) = &weapon.barrel_model {
                    if !barrel_model.is_default() {
                        let mut barrel = create_model_entity(barrel_model.clone());
                        barrel.lighting_origin = re.lighting_origin;
                        barrel.shadow_plane = re.shadow_plane;
                        barrel.shading.render_flags = re.shading.render_flags;
                        position_rotated_entity_on_tag(&mut barrel, &re, &weapon.weapon_model, "tag_barrel");
                        barrel.axis = re.axis;
                        barrel.non_normalized_axes = re.non_normalized_axes;
                        self.body(number, RefEntity::Model(barrel));
                    }
                }
            }
        }
        if !options.simple_items
            && (item.item_type == ItemType::Health || item.item_type == ItemType::Powerup)
            && visual.has_second
            && !visual.models[1].is_default()
        {
            re.model = visual.models[1].clone();
            let mut yaw = 0.0;
            if item.item_type == ItemType::Powerup {
                re.origin = add3(re.origin, vec3(0.0, 0.0, 12.0));
                yaw = ((state.time & 1023) * 360) as f32 / -1024.0;
            }
            re.axis = angles_to_axis(vec3(0.0, yaw, 0.0));
            if fraction != 1.0 {
                re.axis = scale_axis_full(&re.axis, fraction);
                re.non_normalized_axes = true;
            }
            self.body(number, RefEntity::Model(re));
        }
        Ok(())
    }

    fn weapon_info(
        &mut self,
        state: &mut ClientGameState,
        target: PresentEntityTarget,
    ) -> PresentResult<PacketWeaponInfo> {
        // Source intentionally uses > rather than >= WP_NUM_WEAPONS.
        let count = if state.product == Product::MissionPack { 14 } else { 11 };
        let weapon = {
            let entity = match target {
                PresentEntityTarget::Predicted => &mut state.predicted_player_entity,
                PresentEntityTarget::Indexed(index) => state.entity_at(index),
            };
            if entity.current_state.weapon > count {
                entity.current_state.weapon = Weapon::None as i32;
            }
            entity.current_state.weapon
        };
        indexed(&self.media.weapons, weapon, "weapon")
    }

    fn missile(&mut self, state: &mut ClientGameState, target: PresentEntityTarget) -> PresentResult<()> {
        let weapon = self.weapon_info(state, target)?;
        let current = match target {
            PresentEntityTarget::Predicted => state.predicted_player_entity.current_state.clone(),
            PresentEntityTarget::Indexed(index) => state.entity_at(index).current_state.clone(),
        };
        let lerp_origin = match target {
            PresentEntityTarget::Predicted => state.predicted_player_entity.lerp_origin,
            PresentEntityTarget::Indexed(index) => state.entity_at(index).lerp_origin,
        };
        {
            let entity = match target {
                PresentEntityTarget::Predicted => &mut state.predicted_player_entity,
                PresentEntityTarget::Indexed(index) => state.entity_at(index),
            };
            entity.lerp_angles = vec3(current.angles.x, current.angles.y, current.angles.z);
        }
        if let Some(kind) = weapon.missile_trail {
            let entity = match target {
                PresentEntityTarget::Predicted => state.predicted_player_entity.clone(),
                PresentEntityTarget::Indexed(index) => state.entity_at(index).clone(),
            };
            self.imports.missile_trail(kind, &entity, &weapon);
        }
        if weapon.missile_dlight != 0.0 {
            self.imports.add_light(DynamicLight {
                origin: lerp_origin,
                radius: weapon.missile_dlight,
                color: weapon.missile_dlight_color,
                additive: false,
            });
        }
        if weapon.missile_sound.is_some() {
            let velocity = evaluate_trajectory_delta(&current.pos, state.time);
            self.imports.add_loop_sound(
                current.number,
                lerp_origin,
                velocity,
                weapon.missile_sound.clone(),
                false,
            );
        }
        if current.weapon == Weapon::Plasmagun as i32 {
            let mut re = create_sprite_entity();
            re.origin = lerp_origin;
            re.radius = 16.0;
            re.shading.custom_shader = self.media.plasma_ball_shader.clone();
            self.body(current.number, RefEntity::Sprite(re));
            return Ok(());
        }
        let mut re = create_model_entity(weapon.missile_model.clone());
        re.origin = lerp_origin;
        re.old_origin = lerp_origin;
        re.skin_num = state.client_frame & 1;
        re.shading.render_flags = weapon.missile_renderfx | RF_NOSHADOW;
        if self.media.product() == Product::MissionPack
            && current.weapon == Weapon::ProxLauncher as i32
            && current.generic1 == Team::Blue as i32
        {
            if let PacketEntityMediaVariant::Mission(media) = &self.media.variant {
                re.model = media.blue_prox_mine.clone();
            }
        }
        let direction = missile_direction(current.pos.delta);
        if current.pos.type_ != TrajectoryType::Stationary {
            re.axis = direction_axis(direction, (state.time / 4) as f32);
        } else if state.product == Product::MissionPack && current.weapon == Weapon::ProxLauncher as i32 {
            let lerp_angles = match target {
                PresentEntityTarget::Predicted => state.predicted_player_entity.lerp_angles,
                PresentEntityTarget::Indexed(index) => state.entity_at(index).lerp_angles,
            };
            re.axis = angles_to_axis(lerp_angles);
        } else {
            re.axis = direction_axis(direction, current.time as f32);
        }
        if !self.imports.body_hidden(current.number) {
            self.imports.add_entity_with_powerups(re, &current, Team::Free);
        }
        Ok(())
    }

    fn grapple(&mut self, state: &mut ClientGameState, target: PresentEntityTarget) -> PresentResult<()> {
        let weapon = self.weapon_info(state, target)?;
        let current = match target {
            PresentEntityTarget::Predicted => state.predicted_player_entity.current_state.clone(),
            PresentEntityTarget::Indexed(index) => state.entity_at(index).current_state.clone(),
        };
        let lerp_origin = match target {
            PresentEntityTarget::Predicted => state.predicted_player_entity.lerp_origin,
            PresentEntityTarget::Indexed(index) => state.entity_at(index).lerp_origin,
        };
        {
            let entity = match target {
                PresentEntityTarget::Predicted => &mut state.predicted_player_entity,
                PresentEntityTarget::Indexed(index) => state.entity_at(index),
            };
            entity.lerp_angles = vec3(current.angles.x, current.angles.y, current.angles.z);
        }
        let entity = match target {
            PresentEntityTarget::Predicted => state.predicted_player_entity.clone(),
            PresentEntityTarget::Indexed(index) => state.entity_at(index).clone(),
        };
        self.imports.grapple_trail(&entity, &weapon);
        let mut re = create_model_entity(weapon.missile_model.clone());
        re.origin = lerp_origin;
        re.old_origin = lerp_origin;
        re.skin_num = state.client_frame & 1;
        re.shading.render_flags = weapon.missile_renderfx | RF_NOSHADOW;
        // CG_Grapple only fills axis[0]; the two cleared axes remain zero.
        re.axis = [missile_direction(current.pos.delta), re.axis[1], re.axis[2]];
        self.body(current.number, RefEntity::Model(re));
        Ok(())
    }

    fn mover(&mut self, state: &mut ClientGameState, target: PresentEntityTarget) -> PresentResult<()> {
        let (solid, modelindex, modelindex2, number, lerp_origin, lerp_angles) = {
            let entity = match target {
                PresentEntityTarget::Predicted => &state.predicted_player_entity,
                PresentEntityTarget::Indexed(index) => state.entity_at(index),
            };
            (
                entity.current_state.solid,
                entity.current_state.modelindex,
                entity.current_state.modelindex2,
                entity.current_state.number,
                entity.lerp_origin,
                entity.lerp_angles,
            )
        };
        let model = if solid == SOLID_BMODEL {
            indexed(&self.media.inline_models, modelindex, "inline model")?.model
        } else {
            indexed(&self.media.game_models, modelindex, "game model")?
        };
        let mut re = create_model_entity(model);
        re.origin = lerp_origin;
        re.old_origin = lerp_origin;
        re.axis = angles_to_axis(lerp_angles);
        re.shading.render_flags = RF_NOSHADOW;
        re.skin_num = (state.time >> 6) & 1;
        self.body(number, RefEntity::Model(re.clone()));
        if modelindex2 != 0 {
            re.skin_num = 0;
            re.model = indexed(&self.media.game_models, modelindex2, "game model")?;
            self.body(number, RefEntity::Model(re));
        }
        Ok(())
    }

    /// Beam entity (`beam`).
    pub fn beam(&mut self, state: &mut ClientGameState, target: PresentEntityTarget) {
        let (number, base, origin2) = match target {
            PresentEntityTarget::Predicted => {
                let entity = &state.predicted_player_entity;
                (
                    entity.current_state.number,
                    entity.current_state.pos.base,
                    entity.current_state.origin2,
                )
            }
            PresentEntityTarget::Indexed(index) => {
                let entity = state.entity_at(index);
                (
                    entity.current_state.number,
                    entity.current_state.pos.base,
                    entity.current_state.origin2,
                )
            }
        };
        let mut re = create_beam_entity();
        re.origin = base;
        re.old_origin = origin2;
        re.shading.render_flags = RF_NOSHADOW;
        re.axis = [vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)];
        self.body(number, RefEntity::Beam(re));
    }

    fn portal(&mut self, state: &mut ClientGameState, target: PresentEntityTarget) {
        let current = match target {
            PresentEntityTarget::Predicted => state.predicted_player_entity.current_state.clone(),
            PresentEntityTarget::Indexed(index) => state.entity_at(index).current_state.clone(),
        };
        let lerp_origin = match target {
            PresentEntityTarget::Predicted => state.predicted_player_entity.lerp_origin,
            PresentEntityTarget::Indexed(index) => state.entity_at(index).lerp_origin,
        };
        let mut re = create_portal_entity();
        re.origin = lerp_origin;
        re.old_origin = current.origin2;
        let forward = byte_to_direction(current.event_parm);
        let side = sub3(zero_vec3(), perpendicular_vector(forward));
        re.axis = [forward, side, cross3(forward, side)];
        re.old_frame = current.powerups;
        re.frame = current.frame;
        re.skin_num = qvm_float_to_int(current.client_num as f32 / 256.0 * 360.0);
        self.body(current.number, RefEntity::Portal(re));
    }

    fn team(
        &mut self,
        state: &mut ClientGameState,
        target: PresentEntityTarget,
        options: &PacketEntityOptions,
    ) -> PresentResult<()> {
        let current = match target {
            PresentEntityTarget::Predicted => state.predicted_player_entity.current_state.clone(),
            PresentEntityTarget::Indexed(index) => state.entity_at(index).current_state.clone(),
        };
        let lerp_origin = match target {
            PresentEntityTarget::Predicted => state.predicted_player_entity.lerp_origin,
            PresentEntityTarget::Indexed(index) => state.entity_at(index).lerp_origin,
        };
        let mut re = create_model_entity(default_model());
        re.origin = lerp_origin;
        re.lighting_origin = lerp_origin;
        re.axis = angles_to_axis(current.angles);
        if options.game_type == GameType::Ctf
            || (state.product == Product::MissionPack && options.game_type == GameType::OneFlagCtf)
        {
            re.model = if current.modelindex == Team::Red as i32 {
                self.media.red_flag_base_model.clone()
            } else if current.modelindex == Team::Blue as i32 {
                self.media.blue_flag_base_model.clone()
            } else {
                self.media.neutral_flag_base_model.clone()
            };
            self.body(current.number, RefEntity::Model(re));
            return Ok(());
        }
        let PacketEntityMediaVariant::Mission(media) = self.media.variant.clone() else {
            return Ok(());
        };
        if options.game_type == GameType::Harvester {
            re.model = if current.modelindex == Team::Red as i32 || current.modelindex == Team::Blue as i32 {
                media.harvester_model.clone()
            } else {
                media.harvester_neutral_model.clone()
            };
            re.custom_skin = if current.modelindex == Team::Red as i32 {
                media.harvester_red_skin.clone()
            } else if current.modelindex == Team::Blue as i32 {
                media.harvester_blue_skin.clone()
            } else {
                None
            };
            self.body(current.number, RefEntity::Model(re));
            return Ok(());
        }
        if options.game_type != GameType::Obelisk {
            return Ok(());
        }
        re.model = media.overload_base_model.clone();
        self.body(current.number, RefEntity::Model(re.clone()));
        let health = qvm_float_to_int(current.modelindex2 as f32) & 255;
        if current.frame == 1 {
            re.shading.shader_rgba = vec4(255.0, health as f32, health as f32, 255.0);
            re.model = media.overload_energy_model.clone();
            self.body(current.number, RefEntity::Model(re.clone()));
        }
        if current.frame != 2 {
            {
                let entity = match target {
                    PresentEntityTarget::Predicted => &mut state.predicted_player_entity,
                    PresentEntityTarget::Indexed(index) => state.entity_at(index),
                };
                entity.misc_time = 0;
                entity.muzzle_flash_time = 0;
            }
            re.shading.shader_rgba = vec4(255.0, health as f32, health as f32, 255.0);
            re.model = media.overload_lights_model.clone();
            self.body(current.number, RefEntity::Model(re.clone()));
            re.origin = add3(re.origin, vec3(0.0, 0.0, 56.0));
            re.model = media.overload_target_model.clone();
            self.body(current.number, RefEntity::Model(re));
            return Ok(());
        }
        let time = state.time;
        let misc_time = {
            let entity = match target {
                PresentEntityTarget::Predicted => &mut state.predicted_player_entity,
                PresentEntityTarget::Indexed(index) => state.entity_at(index),
            };
            if entity.misc_time == 0 {
                entity.misc_time = time;
            }
            entity.misc_time
        };
        let elapsed = state.time.wrapping_sub(misc_time);
        let threshold = options.obelisk_respawn_delay.wrapping_sub(5).wrapping_mul(1000);
        let scale = if elapsed > threshold {
            (((elapsed - threshold) as f32) / (threshold as f32)).min(1.0)
        } else {
            0.0
        };
        let color = (qvm_float_to_int(scale * 255.0) & 255) as f32;
        re.shading.shader_rgba = vec4(color, color, color, color);
        re.model = media.overload_lights_model.clone();
        self.body(current.number, RefEntity::Model(re.clone()));
        if elapsed > threshold {
            let muzzle = match target {
                PresentEntityTarget::Predicted => state.predicted_player_entity.muzzle_flash_time,
                PresentEntityTarget::Indexed(index) => state.entity_at(index).muzzle_flash_time,
            };
            if muzzle == 0 {
                self.imports
                    .start_sound(Some(lerp_origin), 1023, CHAN_BODY, media.obelisk_respawn_sound.clone());
                let entity = match target {
                    PresentEntityTarget::Predicted => &mut state.predicted_player_entity,
                    PresentEntityTarget::Indexed(index) => state.entity_at(index),
                };
                entity.muzzle_flash_time = 1;
            }
            let spin = 16.0 * (1.0 - scale).acos() * 180.0 / std::f32::consts::PI;
            re.axis = scale_axis_full(
                &angles_to_axis(vec3(current.angles.x, current.angles.y + spin, current.angles.z)),
                scale,
            );
            // Source leaves nonNormalizedAxes false even while scaling this target.
            re.shading.shader_rgba = vec4(255.0, 255.0, 255.0, 255.0);
            re.origin = add3(re.origin, vec3(0.0, 0.0, 56.0));
            re.model = media.overload_target_model.clone();
            self.body(current.number, RefEntity::Model(re));
        }
        Ok(())
    }

    /// Add an entity (`addEntity`).
    pub fn add_entity(
        &mut self,
        state: &mut ClientGameState,
        target: PresentEntityTarget,
        options: &PacketEntityOptions,
    ) -> PresentResult<()> {
        let type_ = match target {
            PresentEntityTarget::Predicted => state.predicted_player_entity.current_state.e_type,
            PresentEntityTarget::Indexed(index) => state.entity_at(index).current_state.e_type,
        };
        if type_ >= EntityType::Events as i32 {
            return Ok(());
        }
        self.calculate_lerp_positions(state, target, options.smooth_clients)?;
        match target {
            PresentEntityTarget::Predicted => {
                // Pose through a cloned entity to keep the borrow simple; the
                // donor pose hook only reads interpolated positions.
                let mut entity = state.predicted_player_entity.clone();
                self.imports.pose_entity(&mut entity);
                state.predicted_player_entity = entity;
            }
            PresentEntityTarget::Indexed(index) => {
                let mut entity = state.entity_at(index).clone();
                self.imports.pose_entity(&mut entity);
                *state.entity_at(index) = entity;
            }
        }
        self.effects(state, target)?;
        match EntityType::from_i32(type_) {
            Some(EntityType::Invisible) | Some(EntityType::PushTrigger) | Some(EntityType::TeleportTrigger) => Ok(()),
            Some(EntityType::General) => self.general(state, target),
            Some(EntityType::Player) => {
                let entity = match target {
                    PresentEntityTarget::Predicted => state.predicted_player_entity.clone(),
                    PresentEntityTarget::Indexed(index) => state.entity_at(index).clone(),
                };
                self.imports.present_player(&entity);
                Ok(())
            }
            Some(EntityType::Item) => self.item(state, target, options),
            Some(EntityType::Missile) => self.missile(state, target),
            Some(EntityType::Mover) => self.mover(state, target),
            Some(EntityType::Beam) => {
                self.beam(state, target);
                Ok(())
            }
            Some(EntityType::Portal) => {
                self.portal(state, target);
                Ok(())
            }
            Some(EntityType::Speaker) => self.speaker(state, target),
            Some(EntityType::Grapple) => self.grapple(state, target),
            Some(EntityType::Team) => self.team(state, target, options),
            Some(EntityType::Events) | None => Err(PresentError::drop(format!("Bad entity type: {type_}\n"))),
        }
    }

    /// Add packet entities (`addPacketEntities`).
    pub fn add_packet_entities(
        &mut self,
        state: &mut ClientGameState,
        options: &PacketEntityOptions,
    ) -> PresentResult<()> {
        if state.snap.is_none() {
            return Err(PresentError::state("CG_AddPacketEntities: cg.snap == NULL"));
        }
        let snap_time = state.snap.as_ref().map(|snap| snap.server_time).unwrap_or(0);
        let delta = state
            .next_snap
            .as_ref()
            .map(|next| next.server_time.wrapping_sub(snap_time))
            .unwrap_or(0);
        state.frame_interpolation = if delta == 0 {
            0.0
        } else {
            (state.time.wrapping_sub(snap_time) as f32) / (delta as f32)
        };
        state.auto_angles = vec3(0.0, ((state.time & 2047) * 360) as f32 / 2048.0, 0.0);
        state.auto_angles_fast = vec3(0.0, ((state.time & 1023) * 360) as f32 / 1024.0, 0.0);
        state.auto_axis = angles_to_axis(state.auto_angles);
        state.auto_axis_fast = angles_to_axis(state.auto_angles_fast);
        let predicted = state.predicted_player_state.clone();
        player_state_to_entity_state(&predicted, &mut state.predicted_player_entity.current_state);
        self.add_entity(state, PresentEntityTarget::Predicted, options)?;
        let client_num = state
            .snap
            .as_ref()
            .map(|snap| snap.player_state.client_num)
            .unwrap_or(0);
        self.calculate_lerp_positions(
            state,
            PresentEntityTarget::Indexed(client_num.max(0) as usize),
            options.smooth_clients,
        )?;
        let numbers: Vec<i32> = state
            .snap
            .as_ref()
            .map(|snap| snap.entities.iter().map(|entity| entity.number).collect())
            .unwrap_or_default();
        for number in numbers {
            self.add_entity(state, PresentEntityTarget::Indexed(number.max(0) as usize), options)?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// weapons.ts
// ---------------------------------------------------------------------------

/// Water contents.
pub const CONTENTS_WATER: i32 = 32;
/// Shot mask.
pub const MASK_SHOT: i32 = 1 | 0x2000000 | 0x4000000;
/// No-impact surface flag.
pub const SURF_NOIMPACT: i32 = 16;
/// Metal-steps surface flag.
pub const SURF_METALSTEPS: i32 = 4096;

/// Impact sound (`ImpactSound`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum ImpactSound {
    /// Default.
    Default = 0,
    /// Metal.
    Metal = 1,
    /// Flesh.
    Flesh = 2,
}

/// Shotgun trace result.
#[derive(Debug, Clone, PartialEq)]
pub struct ShotgunTrace<Target> {
    /// End.
    pub end: Vec3,
    /// Normal.
    pub normal: Vec3,
    /// Surface flags.
    pub surface_flags: i32,
    /// Target.
    pub target: Option<Target>,
}

/// Shotgun presentation host (`ShotgunPresentationHost`).
pub trait ShotgunPresentationHost<Target> {
    /// Smoke enabled.
    fn smoke_enabled(&self) -> bool;
    /// Trace.
    fn trace(&self, start: Vec3, end: Vec3) -> ShotgunTrace<Target>;
    /// Water boundary.
    fn water(&self, start: Vec3, end: Vec3) -> Vec3;
    /// Contents.
    fn contents(&self, point: Vec3) -> i32;
    /// Whether a target is a player.
    fn is_player(&self, target: &Target) -> bool;
    /// Blood at a point.
    fn blood(&mut self, point: Vec3, normal: Vec3, target: Target);
    /// Wall impact.
    fn wall(&mut self, point: Vec3, normal: Vec3, sound: ImpactSound);
    /// Bubbles.
    fn bubbles(&mut self, start: Vec3, end: Vec3);
    /// Smoke.
    fn smoke(&mut self, origin: Vec3);
}

/// Emit shotgun presentation (`emitShotgunPresentation`).
pub fn emit_shotgun_presentation<Target>(host: &mut dyn ShotgunPresentationHost<Target>, shot: &Q3ShotgunEvent) {
    if host.smoke_enabled() && host.contents(shot.muzzle) & CONTENTS_WATER == 0 {
        let direction = normalize3(sub3(shot.direction, shot.muzzle));
        host.smoke(add3(shot.muzzle, scale3(direction, 32.0)));
    }
    for end in q3_shotgun_endpoints(shot.muzzle, shot.direction, shot.seed) {
        let trace = host.trace(shot.muzzle, end);
        let source_contents = host.contents(shot.muzzle);
        let destination_contents = host.contents(trace.end);
        if source_contents == destination_contents {
            if source_contents & CONTENTS_WATER != 0 {
                host.bubbles(shot.muzzle, trace.end);
            }
        } else if source_contents & CONTENTS_WATER != 0 {
            let water = host.water(end, shot.muzzle);
            host.bubbles(shot.muzzle, water);
        } else if destination_contents & CONTENTS_WATER != 0 {
            let water = host.water(shot.muzzle, end);
            host.bubbles(trace.end, water);
        }
        if trace.surface_flags & SURF_NOIMPACT != 0 {
            continue;
        }
        match trace.target {
            Some(target) if host.is_player(&target) => host.blood(trace.end, trace.normal, target),
            _ => host.wall(
                trace.end,
                trace.normal,
                if trace.surface_flags & SURF_METALSTEPS != 0 {
                    ImpactSound::Metal
                } else {
                    ImpactSound::Default
                },
            ),
        }
    }
}

/// Bullet hit (`BulletHit`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BulletHit {
    /// Wall.
    Wall {
        /// Normal.
        normal: Vec3,
    },
    /// Flesh.
    Flesh {
        /// Entity number.
        entity_num: i32,
    },
}

/// Ejected brass kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EjectBrass {
    /// Machinegun.
    Machinegun,
    /// Shotgun.
    Shotgun,
    /// Nailgun.
    Nailgun,
}

/// Client weapon info (`ClientWeaponInfo`).
#[derive(Debug, Clone, PartialEq)]
pub struct ClientWeaponInfo {
    /// Packet info.
    pub packet: PacketWeaponInfo,
    /// Item.
    pub item: Option<ItemDefinition>,
    /// Hands model.
    pub hands_model: SceneModel,
    /// Flash model.
    pub flash_model: SceneModel,
    /// Ammo model.
    pub ammo_model: SceneModel,
    /// Weapon icon.
    pub weapon_icon: Option<SceneShader>,
    /// Ammo icon.
    pub ammo_icon: Option<SceneShader>,
    /// Flash light color.
    pub flash_dlight_color: Vec3,
    /// Flash sounds.
    pub flash_sounds: [Option<PresentSound>; 4],
    /// Ejected brass.
    pub eject_brass: Option<EjectBrass>,
    /// Ready sound.
    pub ready_sound: Option<PresentSound>,
    /// Firing sound.
    pub firing_sound: Option<PresentSound>,
    /// Looping fire sound.
    pub loop_fire_sound: bool,
}

fn empty_packet_weapon() -> PacketWeaponInfo {
    PacketWeaponInfo {
        weapon_model: default_model(),
        weapon_midpoint: zero_vec3(),
        barrel_model: None,
        missile_model: default_model(),
        missile_renderfx: 0,
        missile_sound: None,
        missile_dlight: 0.0,
        missile_dlight_color: zero_vec3(),
        missile_trail: None,
        trail_radius: 0.0,
        trail_time: 0,
    }
}

fn empty_weapon() -> ClientWeaponInfo {
    ClientWeaponInfo {
        packet: empty_packet_weapon(),
        item: None,
        hands_model: default_model(),
        flash_model: default_model(),
        ammo_model: default_model(),
        weapon_icon: None,
        ammo_icon: None,
        flash_dlight_color: zero_vec3(),
        flash_sounds: [None, None, None, None],
        eject_brass: None,
        ready_sound: None,
        firing_sound: None,
        loop_fire_sound: false,
    }
}

/// Client weapon selection (`ClientWeaponSelection`).
#[derive(Default)]
pub struct ClientWeaponSelection {
    /// Selection callback.
    pub on_select: Option<Box<dyn FnMut(i32)>>,
}

impl ClientWeaponSelection {
    /// New selection.
    #[must_use]
    pub fn new() -> Self {
        Self { on_select: None }
    }

    fn selectable(&self, state: &ClientGameState, number: i32) -> PresentResult<bool> {
        let snap = state
            .snap
            .as_ref()
            .ok_or_else(|| PresentError::state("CG_WeaponSelectable: cg.snap == NULL"))?;
        Ok(snap.player_state.ammo.get(number as usize) != 0
            && snap.player_state.stats.get(stat_schema(state.product).weapons) & (1 << number) != 0)
    }

    /// Next weapon.
    pub fn next_weapon(&mut self, state: &mut ClientGameState) -> PresentResult<()> {
        self.cycle_weapon(state, 1)
    }

    /// Previous weapon.
    pub fn previous_weapon(&mut self, state: &mut ClientGameState) -> PresentResult<()> {
        self.cycle_weapon(state, -1)
    }

    fn cycle_weapon(&mut self, state: &mut ClientGameState, direction: i32) -> PresentResult<()> {
        let followed = state
            .snap
            .as_ref()
            .is_some_and(|snap| snap.player_state.pm_flags & MoveFlags::FOLLOW != 0);
        if state.snap.is_none() || followed {
            return Ok(());
        }
        state.weapon_select_time = state.time;
        let original = state.weapon_select;
        for _ in 0..16 {
            state.weapon_select = (state.weapon_select + direction + 16) % 16;
            if state.weapon_select != Weapon::Gauntlet as i32 && self.selectable(state, state.weapon_select)? {
                let selected = state.weapon_select;
                if let Some(on_select) = self.on_select.as_mut() {
                    on_select(selected);
                }
                return Ok(());
            }
        }
        state.weapon_select = original;
        Ok(())
    }

    /// Select a weapon.
    pub fn select_weapon(&mut self, state: &mut ClientGameState, number: i32) {
        let followed = state
            .snap
            .as_ref()
            .is_some_and(|snap| snap.player_state.pm_flags & MoveFlags::FOLLOW != 0);
        if state.snap.is_none() || followed || number < 1 || number > 15 {
            return;
        }
        state.weapon_select_time = state.time;
        let has = state
            .snap
            .as_ref()
            .is_some_and(|snap| snap.player_state.stats.get(stat_schema(state.product).weapons) & (1 << number) != 0);
        if has {
            state.weapon_select = number;
            if let Some(on_select) = self.on_select.as_mut() {
                on_select(number);
            }
        }
    }

    /// Change on empty (`outOfAmmoChange`).
    pub fn out_of_ammo_change(&mut self, state: &mut ClientGameState) -> PresentResult<()> {
        state.weapon_select_time = state.time;
        for number in (1..=15).rev() {
            if self.selectable(state, number)? {
                state.weapon_select = number;
                return Ok(());
            }
        }
        Ok(())
    }
}

/// Weapon presentation models.
#[derive(Debug, Clone, PartialEq)]
pub struct WeaponPresentationModels {
    /// Machinegun brass.
    pub machinegun_brass: SceneModel,
    /// Shotgun brass.
    pub shotgun_brass: SceneModel,
    /// Dish flash.
    pub dish_flash: SceneModel,
    /// Ring flash.
    pub ring_flash: SceneModel,
    /// Bullet flash.
    pub bullet_flash: SceneModel,
}

/// Weapon presentation shaders.
#[derive(Debug, Clone, PartialEq)]
pub struct WeaponPresentationShaders {
    /// Smoke puff.
    pub smoke_puff: Option<SceneShader>,
    /// Nail puff.
    pub nail_puff: Option<SceneShader>,
    /// Shotgun smoke puff.
    pub shotgun_smoke_puff: Option<SceneShader>,
    /// Invisibility.
    pub invis: Option<SceneShader>,
    /// Battle weapon.
    pub battle_weapon: Option<SceneShader>,
    /// Quad weapon.
    pub quad_weapon: Option<SceneShader>,
    /// Select.
    pub select: Option<SceneShader>,
    /// No ammo.
    pub noammo: Option<SceneShader>,
    /// Hole mark.
    pub hole_mark: Option<SceneShader>,
    /// Burn mark.
    pub burn_mark: Option<SceneShader>,
    /// Energy mark.
    pub energy_mark: Option<SceneShader>,
    /// Bullet mark.
    pub bullet_mark: Option<SceneShader>,
    /// Tracer.
    pub tracer: Option<SceneShader>,
}

/// Weapon presentation sounds.
#[derive(Debug, Clone, PartialEq)]
pub struct WeaponPresentationSounds {
    /// Quad.
    pub quad: Option<PresentSound>,
    /// Nail hit flesh.
    pub nail_hit_flesh: Option<PresentSound>,
    /// Nail hit metal.
    pub nail_hit_metal: Option<PresentSound>,
    /// Nail hit.
    pub nail_hit: Option<PresentSound>,
    /// Prox explosion.
    pub prox_explosion: Option<PresentSound>,
    /// Rocket explosion.
    pub rocket_explosion: Option<PresentSound>,
    /// Plasma explosion.
    pub plasma_explosion: Option<PresentSound>,
    /// Chaingun hit flesh.
    pub chaingun_hit_flesh: Option<PresentSound>,
    /// Chaingun hit metal.
    pub chaingun_hit_metal: Option<PresentSound>,
    /// Chaingun hit.
    pub chaingun_hit: Option<PresentSound>,
    /// Ricochet 1.
    pub ricochet1: Option<PresentSound>,
    /// Ricochet 2.
    pub ricochet2: Option<PresentSound>,
    /// Ricochet 3.
    pub ricochet3: Option<PresentSound>,
    /// Tracer.
    pub tracer: Option<PresentSound>,
}

/// Weapon presentation media (`WeaponPresentationMedia`).
#[derive(Debug, Clone, PartialEq)]
pub struct WeaponPresentationMedia {
    /// Models.
    pub models: WeaponPresentationModels,
    /// Shaders.
    pub shaders: WeaponPresentationShaders,
    /// Sounds.
    pub sounds: WeaponPresentationSounds,
}

/// Weapon presentation settings (`WeaponPresentationSettings`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WeaponPresentationSettings {
    /// Brass time.
    pub brass_time: i32,
    /// Rail trail time.
    pub rail_trail_time: i32,
    /// Old rail.
    pub old_rail: bool,
    /// No projectile trail.
    pub no_projectile_trail: bool,
    /// Old plasma.
    pub old_plasma: bool,
    /// Old rocket.
    pub old_rocket: bool,
    /// True lightning blend.
    pub true_lightning: f32,
    /// Draw gun.
    pub draw_gun: bool,
    /// FOV.
    pub fov: f32,
    /// Gun X.
    pub gun_x: f32,
    /// Gun Y.
    pub gun_y: f32,
    /// Gun Z.
    pub gun_z: f32,
    /// Gun frame override.
    pub gun_frame: i32,
    /// Tracer length.
    pub tracer_length: f32,
    /// Tracer width.
    pub tracer_width: f32,
    /// Tracer chance.
    pub tracer_chance: f32,
    /// Rage Pro hardware.
    pub hardware_rage_pro: bool,
}

/// Weapon selection drawing (`WeaponSelectionDrawing`).
pub trait WeaponSelectionDrawing {
    /// Fade color.
    fn fade_color(&self, start: i32, duration: i32) -> Option<Vec4>;
    /// Set color.
    fn set_color(&mut self, color: Option<Vec4>);
    /// Draw a picture.
    fn draw_pic(&mut self, x: i32, y: i32, width: i32, height: i32, shader: Option<SceneShader>);
    /// Draw string length.
    fn draw_string_length(&self, text: &str) -> usize;
    /// Draw a big string.
    fn draw_big_string_color(&mut self, x: i32, y: i32, text: &str, color: Vec4);
}

/// Client animation reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientAnimRef {
    /// First frame.
    pub first_frame: i32,
}

/// Client info view (`ClientInfo`, minimal mirror).
#[derive(Debug, Clone, PartialEq)]
pub struct ClientInfoView {
    /// Color 1.
    pub color1: Vec3,
    /// Color 2.
    pub color2: Vec3,
    /// Animations.
    pub animations: Vec<Option<ClientAnimRef>>,
}

/// Client weapon host (`ClientWeaponHost`).
pub trait ClientWeaponHost {
    /// Add a reference entity.
    fn add_ref_entity(&mut self, entity: RefEntity);
    /// Add a light.
    fn add_light(&mut self, light: DynamicLight);
    /// Start a sound.
    fn start_sound(&mut self, origin: Option<Vec3>, entity: i32, channel: i32, sound: Option<PresentSound>);
    /// Add a loop sound.
    fn add_loop_sound(
        &mut self,
        entity: i32,
        origin: Vec3,
        velocity: Vec3,
        sound: Option<PresentSound>,
        real_loop: bool,
    );
    /// Trace with entity skipping.
    fn trace_mover(&self, start: Vec3, end: Vec3, bounds: Bounds, skip_number: i32, mask: i32) -> MovementTrace;
    /// Point contents with entity passing.
    fn point_contents_pred(&self, point: Vec3, pass_entity: i32) -> i32;
    /// Raw shape trace.
    fn collision_trace(&self, start: Vec3, end: Vec3, mask: i32) -> TraceResult;
    /// Raw point contents.
    fn collision_contents(&self, point: Vec3) -> i32;
    /// Random integer.
    fn rand_i32(&mut self) -> i32;
    /// Random fraction.
    fn random_f32(&mut self) -> f32;
    /// Centered random fraction.
    fn crandom_f32(&mut self) -> f32;
    /// Effects.
    fn weapon_effects(&mut self) -> &mut ClientEffects;
    /// Project an impact mark.
    fn impact_mark(&mut self, request: &ImpactMarkRequest) -> Vec<RefPoly>;
    /// Particle explosion.
    fn particle_explosion(&mut self, request: &ParticleExplosion);
    /// Media.
    fn weapon_media(&self) -> &WeaponPresentationMedia;
    /// Settings.
    fn weapon_settings(&self) -> WeaponPresentationSettings;
    /// Client info.
    fn client_info_view(&self, number: i32) -> ClientInfoView;
    /// Load a sound synchronously.
    fn load_sound(&mut self, path: &str) -> Option<PresentSound>;
    /// Add a polygon.
    fn add_poly(&mut self, poly: RefPoly);
    /// Drawing.
    fn drawing(&mut self) -> &mut dyn WeaponSelectionDrawing;
}

fn weapon_ma(origin: Vec3, scale: f32, direction: Vec3) -> Vec3 {
    add3(origin, scale3(direction, scale))
}

fn weapon_transform(value: Vec3, axis: &Axis) -> Vec3 {
    vec3(
        dot3(value, vec3(axis[0].x, axis[1].x, axis[2].x)),
        dot3(value, vec3(axis[0].y, axis[1].y, axis[2].y)),
        dot3(value, vec3(axis[0].z, axis[1].z, axis[2].z)),
    )
}

fn weapon_bytes(color: Vec3, scale: f32, alpha: f32) -> Vec4 {
    vec4(
        (qvm_float_to_int(color.x * scale) & 255) as f32,
        (qvm_float_to_int(color.y * scale) & 255) as f32,
        (qvm_float_to_int(color.z * scale) & 255) as f32,
        alpha,
    )
}

/// Client weapon runtime (`ClientWeaponRuntime`).
pub struct ClientWeaponRuntime {
    /// Selection.
    pub selection: ClientWeaponSelection,
    /// Registry.
    pub registry: ClientWeaponMediaRegistry,
    /// Host.
    pub host: Box<dyn ClientWeaponHost>,
}

impl ClientWeaponRuntime {
    /// New runtime.
    pub fn new(
        product: Product,
        registry: ClientWeaponMediaRegistry,
        host: Box<dyn ClientWeaponHost>,
    ) -> PresentResult<Self> {
        if product != registry.product {
            return Err(PresentError::state("Weapon media product differs from cgame state"));
        }
        Ok(Self {
            selection: ClientWeaponSelection::new(),
            registry,
            host,
        })
    }

    /// Fire a weapon (`fireWeapon`).
    pub fn fire_weapon(
        &mut self,
        state: &mut ClientGameState,
        pool: &mut LocalEntityPool,
        frame: &EffectFrame,
        target: PresentEntityTarget,
    ) -> PresentResult<()> {
        let ent = match target {
            PresentEntityTarget::Predicted => state.predicted_player_entity.current_state.clone(),
            PresentEntityTarget::Indexed(index) => state.entity_at(index).current_state.clone(),
        };
        if ent.weapon == Weapon::None as i32 {
            return Ok(());
        }
        if ent.weapon >= weapon_count(state.product) {
            return Err(PresentError::drop("CG_FireWeapon: ent->weapon >= WP_NUM_WEAPONS"));
        }
        let weapon = self.registry.weapon(ent.weapon)?.clone();
        {
            let time = state.time;
            let entity = match target {
                PresentEntityTarget::Predicted => &mut state.predicted_player_entity,
                PresentEntityTarget::Indexed(index) => state.entity_at(index),
            };
            entity.muzzle_flash_time = time;
        }
        let lightning_firing = match target {
            PresentEntityTarget::Predicted => state.predicted_player_entity.player.lightning_firing,
            PresentEntityTarget::Indexed(index) => state.entity_at(index).player.lightning_firing,
        };
        if ent.weapon == Weapon::Lightning as i32 && lightning_firing != 0 {
            return Ok(());
        }
        if ent.powerups & (1 << Powerup::Quad as i32) != 0 {
            let quad = self.host.weapon_media().sounds.quad.clone();
            self.host.start_sound(None, ent.number, 4, quad);
        }
        let length = weapon
            .flash_sounds
            .iter()
            .position(|sound| sound.is_none())
            .unwrap_or(4);
        if length > 0 {
            let sound = weapon.flash_sounds[(self.host.rand_i32() % length as i32).max(0) as usize % 4].clone();
            if sound.is_some() {
                self.host.start_sound(None, ent.number, 2, sound);
            }
        }
        if weapon.eject_brass.is_some() && self.host.weapon_settings().brass_time > 0 {
            self.eject_brass(
                state,
                pool,
                frame,
                target,
                weapon.eject_brass.unwrap_or(EjectBrass::Machinegun),
            )?;
        }
        Ok(())
    }

    fn eject_brass(
        &mut self,
        state: &mut ClientGameState,
        pool: &mut LocalEntityPool,
        frame: &EffectFrame,
        target: PresentEntityTarget,
        kind: EjectBrass,
    ) -> PresentResult<()> {
        let (lerp_origin, lerp_angles) = match target {
            PresentEntityTarget::Predicted => (
                state.predicted_player_entity.lerp_origin,
                state.predicted_player_entity.lerp_angles,
            ),
            PresentEntityTarget::Indexed(index) => {
                (state.entity_at(index).lerp_origin, state.entity_at(index).lerp_angles)
            }
        };
        let axis = angles_to_axis(lerp_angles);
        let time = state.time;
        if kind == EjectBrass::Nailgun {
            let shader = self.host.weapon_media().shaders.smoke_puff.clone();
            let handle = self.host.weapon_effects().smoke_puff(
                pool,
                frame,
                &SmokePuffOptions {
                    origin: add3(lerp_origin, weapon_transform(vec3(0.0, -12.0, 24.0), &axis)),
                    velocity: vec3(0.0, 0.0, 64.0),
                    radius: 32.0,
                    color: vec4(1.0, 1.0, 1.0, 0.33),
                    duration: 700,
                    start_time: time,
                    fade_in_time: 0,
                    flags: 0,
                    shader,
                },
            )?;
            if let Some(smoke) = pool.get_mut(handle) {
                smoke.le_type = LocalEntityType::ScaleFade;
            }
            return Ok(());
        }
        let brass_time = self.host.weapon_settings().brass_time;
        if brass_time <= 0 {
            return Ok(());
        }
        let shotgun = kind == EjectBrass::Shotgun;
        for i in 0..if shotgun { 2 } else { 1 } {
            let model = if shotgun {
                self.host.weapon_media().models.shotgun_brass.clone()
            } else {
                self.host.weapon_media().models.machinegun_brass.clone()
            };
            let re = create_model_entity(model);
            let handle = pool.allocate(LocalEntityType::Fragment, RefEntity::Model(re))?;
            let velocity = if shotgun {
                vec3(
                    60.0 + 60.0 * self.host.crandom_f32(),
                    (if i == 0 { 40.0 } else { -40.0 }) + 10.0 * self.host.crandom_f32(),
                    100.0 + 50.0 * self.host.crandom_f32(),
                )
            } else {
                vec3(
                    0.0,
                    -50.0 + 40.0 * self.host.crandom_f32(),
                    100.0 + 50.0 * self.host.crandom_f32(),
                )
            };
            let random = self.host.random_f32();
            let rand_bits = self.host.rand_i32();
            let end_time = qvm_float_to_int(
                time.wrapping_add(brass_time * if shotgun { 3 } else { 1 }) as f32
                    + (if shotgun { brass_time } else { brass_time / 4 }) as f32 * random,
            );
            let pos_time = if shotgun {
                time
            } else {
                time.wrapping_sub(rand_bits & 15)
            };
            let origin = add3(
                lerp_origin,
                weapon_transform(vec3(8.0, if shotgun { 0.0 } else { -4.0 }, 24.0), &axis),
            );
            let water = if self.host.point_contents_pred(origin, -1) & CONTENTS_WATER != 0 {
                0.1
            } else {
                1.0
            };
            if let Some(le) = pool.get_mut(handle) {
                le.start_time = time;
                le.end_time = end_time;
                le.pos = Trajectory {
                    type_: TrajectoryType::Gravity,
                    time: pos_time,
                    duration: 0,
                    base: origin,
                    delta: scale3(weapon_transform(velocity, &axis), water),
                };
                if let RefEntity::Model(re) = &mut le.ref_entity {
                    re.origin = origin;
                    re.axis = identity_axis_full();
                }
                le.bounce_factor = if shotgun { 0.3 } else { 0.4 * water };
                le.angles = Trajectory {
                    type_: TrajectoryType::Linear,
                    time,
                    duration: 0,
                    base: vec3(
                        (self.host.rand_i32() & 31) as f32,
                        (self.host.rand_i32() & 31) as f32,
                        (self.host.rand_i32() & 31) as f32,
                    ),
                    delta: if shotgun {
                        vec3(1.0, 0.5, 0.0)
                    } else {
                        vec3(2.0, 1.0, 0.0)
                    },
                };
                le.le_flags = LE_TUMBLE;
                le.le_bounce_sound_type = LocalBounceSoundType::Brass;
                le.le_mark_type = LocalMarkType::None;
            }
        }
        Ok(())
    }

    /// Rail trail (`railTrail`).
    pub fn rail_trail(
        &mut self,
        time: i32,
        pool: &mut LocalEntityPool,
        frame: &EffectFrame,
        client_num: i32,
        start: &mut Vec3,
        end: Vec3,
    ) -> PresentResult<()> {
        let effects = self.registry.effects.clone();
        emit_rail_trail(time, &effects, &mut *self.host, pool, frame, client_num, start, end)
    }

    /// Missile trail (`missileTrail`).
    pub fn missile_trail(
        &mut self,
        state: &mut ClientGameState,
        pool: &mut LocalEntityPool,
        frame: &EffectFrame,
        kind: MissileTrail,
        target: PresentEntityTarget,
        weapon: &PacketWeaponInfo,
    ) -> PresentResult<()> {
        if kind == MissileTrail::Grapple {
            self.grapple_trail(state, target)?;
            return Ok(());
        }
        if kind == MissileTrail::Plasma {
            self.plasma_trail(state, pool, target)?;
            return Ok(());
        }
        if self.host.weapon_settings().no_projectile_trail {
            return Ok(());
        }
        let (pos, trail_time) = {
            let entity = match target {
                PresentEntityTarget::Predicted => &state.predicted_player_entity,
                PresentEntityTarget::Indexed(index) => state.entity_at(index),
            };
            (entity.current_state.pos, entity.trail_time)
        };
        let time = state.time;
        let mut t = 50 * (((trail_time + 50) / 50) as i32);
        let origin = evaluate_trajectory(&pos, time);
        let contents = self.host.point_contents_pred(origin, -1);
        if pos.type_ == TrajectoryType::Stationary {
            let entity = match target {
                PresentEntityTarget::Predicted => &mut state.predicted_player_entity,
                PresentEntityTarget::Indexed(index) => state.entity_at(index),
            };
            entity.trail_time = time;
            return Ok(());
        }
        let previous = evaluate_trajectory(&pos, trail_time);
        let last_contents = self.host.point_contents_pred(previous, -1);
        {
            let entity = match target {
                PresentEntityTarget::Predicted => &mut state.predicted_player_entity,
                PresentEntityTarget::Indexed(index) => state.entity_at(index),
            };
            entity.trail_time = time;
        }
        if contents & (32 | 16 | 8) != 0 {
            if contents & last_contents & CONTENTS_WATER != 0 {
                self.host
                    .weapon_effects()
                    .bubble_trail(pool, frame, previous, origin, 8.0)?;
            }
            return Ok(());
        }
        while t <= time {
            let shader = if kind == MissileTrail::Nail {
                self.host.weapon_media().shaders.nail_puff.clone()
            } else {
                self.host.weapon_media().shaders.smoke_puff.clone()
            };
            let handle = self.host.weapon_effects().smoke_puff(
                pool,
                frame,
                &SmokePuffOptions {
                    origin: evaluate_trajectory(&pos, t),
                    velocity: zero_vec3(),
                    radius: weapon.trail_radius,
                    color: vec4(1.0, 1.0, 1.0, 0.33),
                    duration: weapon.trail_time,
                    start_time: t,
                    fade_in_time: 0,
                    flags: 0,
                    shader,
                },
            )?;
            if let Some(smoke) = pool.get_mut(handle) {
                smoke.le_type = LocalEntityType::ScaleFade;
            }
            t += 50;
        }
        Ok(())
    }

    fn plasma_trail(
        &mut self,
        state: &ClientGameState,
        pool: &mut LocalEntityPool,
        target: PresentEntityTarget,
    ) -> PresentResult<()> {
        let (pos, lerp_angles, weapon) = match target {
            PresentEntityTarget::Predicted => (
                state.predicted_player_entity.current_state.pos,
                state.predicted_player_entity.lerp_angles,
                state.predicted_player_entity.current_state.weapon,
            ),
            PresentEntityTarget::Indexed(index) => match state.entity_ref(index) {
                Some(entity) => (
                    entity.current_state.pos,
                    entity.lerp_angles,
                    entity.current_state.weapon,
                ),
                None => return Ok(()),
            },
        };
        let origin = evaluate_trajectory(&pos, state.time);
        let effects = self.registry.effects.clone();
        let flash_color = self.registry.weapon(weapon)?.flash_dlight_color;
        emit_plasma_trail(
            state.time,
            origin,
            lerp_angles,
            flash_color,
            effects.rail_rings_shader.clone(),
            &mut *self.host,
            pool,
        )
    }

    /// Grapple trail (`grappleTrail`).
    pub fn grapple_trail(&mut self, state: &mut ClientGameState, target: PresentEntityTarget) -> PresentResult<()> {
        let (pos, other) = match target {
            PresentEntityTarget::Predicted => (
                state.predicted_player_entity.current_state.pos,
                state.predicted_player_entity.current_state.other_entity_num,
            ),
            PresentEntityTarget::Indexed(index) => (
                state.entity_at(index).current_state.pos,
                state.entity_at(index).current_state.other_entity_num,
            ),
        };
        let origin = evaluate_trajectory(&pos, state.time);
        {
            let time = state.time;
            let entity = match target {
                PresentEntityTarget::Predicted => &mut state.predicted_player_entity,
                PresentEntityTarget::Indexed(index) => state.entity_at(index),
            };
            entity.trail_time = time;
        }
        let owner = state.entity_at(other.max(0) as usize).clone();
        let Some((start, end)) = q3_grapple_cable(owner.lerp_origin, angle_vectors(owner.lerp_angles).up, origin)
        else {
            return Ok(());
        };
        let mut beam = create_lightning_entity();
        beam.origin = start;
        beam.old_origin = end;
        beam.shading.custom_shader = self.registry.effects.lightning_shader.clone();
        beam.shading.shader_rgba = vec4(255.0, 255.0, 255.0, 255.0);
        self.host.add_ref_entity(RefEntity::Lightning(beam));
        Ok(())
    }

    /// Missile hit wall (`missileHitWall`).
    pub fn missile_hit_wall(
        &mut self,
        product: Product,
        pool: &mut LocalEntityPool,
        frame: &EffectFrame,
        weapon: Weapon,
        client_num: i32,
        origin: Vec3,
        direction: Vec3,
        sound_type: ImpactSound,
    ) -> PresentResult<()> {
        let effects = self.registry.effects.clone();
        emit_weapon_impact(
            product,
            &effects,
            &mut *self.host,
            pool,
            frame,
            weapon,
            client_num,
            origin,
            direction,
            sound_type,
        )
    }

    /// Missile hit player (`missileHitPlayer`).
    pub fn missile_hit_player(
        &mut self,
        product: Product,
        pool: &mut LocalEntityPool,
        frame: &EffectFrame,
        weapon: Weapon,
        origin: Vec3,
        direction: Vec3,
        entity_num: i32,
    ) -> PresentResult<()> {
        self.host.weapon_effects().bleed(pool, frame, origin, entity_num)?;
        if weapon == Weapon::GrenadeLauncher
            || weapon == Weapon::RocketLauncher
            || (product == Product::MissionPack
                && (weapon == Weapon::Nailgun || weapon == Weapon::Chaingun || weapon == Weapon::ProxLauncher))
        {
            self.missile_hit_wall(product, pool, frame, weapon, 0, origin, direction, ImpactSound::Flesh)?;
        }
        Ok(())
    }

    /// Shotgun fire (`shotgunFire`).
    pub fn shotgun_fire(
        &mut self,
        state: &mut ClientGameState,
        pool: &mut LocalEntityPool,
        frame: &EffectFrame,
        es: &EntityState,
    ) -> PresentResult<()> {
        struct Adapter<'a> {
            host: &'a mut dyn ClientWeaponHost,
            state: &'a ClientGameState,
            pool: &'a mut LocalEntityPool,
            frame: EffectFrame,
            product: Product,
            effects_registry: RegisteredWeaponEffects,
            shooter: i32,
            smoke_enabled: bool,
            error: Option<PresentError>,
        }
        impl ShotgunPresentationHost<i32> for Adapter<'_> {
            fn smoke_enabled(&self) -> bool {
                self.smoke_enabled
            }
            fn trace(&self, start: Vec3, end: Vec3) -> ShotgunTrace<i32> {
                let zero = zero_vec3();
                let trace = self
                    .host
                    .trace_mover(start, end, Bounds { min: zero, max: zero }, self.shooter, MASK_SHOT);
                ShotgunTrace {
                    end: trace.base.end,
                    normal: match trace.base.contact {
                        TraceContact::Plane { plane } => plane.normal,
                        TraceContact::None => zero_vec3(),
                    },
                    surface_flags: trace.base.surface_flags,
                    target: Some(trace.entity_num),
                }
            }
            fn water(&self, start: Vec3, end: Vec3) -> Vec3 {
                self.host.collision_trace(start, end, CONTENTS_WATER).end
            }
            fn contents(&self, point: Vec3) -> i32 {
                self.host.collision_contents(point)
            }
            fn is_player(&self, target: &i32) -> bool {
                self.state
                    .entity_ref((*target).max(0) as usize)
                    .is_some_and(|entity| entity.current_state.e_type == EntityType::Player as i32)
            }
            fn blood(&mut self, point: Vec3, _normal: Vec3, target: i32) {
                // Shotgun flesh routes through missileHitPlayer, which bleeds
                // only for shotguns (no wall impact).
                let result = self.host.weapon_effects().bleed(self.pool, &self.frame, point, target);
                if self.error.is_none() {
                    self.error = result.err();
                }
            }
            fn wall(&mut self, point: Vec3, normal: Vec3, sound: ImpactSound) {
                let effects = self.effects_registry.clone();
                let result = emit_weapon_impact(
                    self.product,
                    &effects,
                    self.host,
                    self.pool,
                    &self.frame,
                    Weapon::Shotgun,
                    0,
                    point,
                    normal,
                    sound,
                );
                if self.error.is_none() {
                    self.error = result.err();
                }
            }
            fn bubbles(&mut self, start: Vec3, end: Vec3) {
                let result = self
                    .host
                    .weapon_effects()
                    .bubble_trail(self.pool, &self.frame, start, end, 32.0);
                if self.error.is_none() {
                    self.error = result.err();
                }
            }
            fn smoke(&mut self, origin: Vec3) {
                let shader = self.host.weapon_media().shaders.shotgun_smoke_puff.clone();
                let time = self.frame.time;
                let result = self.host.weapon_effects().smoke_puff(
                    self.pool,
                    &self.frame,
                    &SmokePuffOptions {
                        origin,
                        velocity: vec3(0.0, 0.0, 8.0),
                        radius: 32.0,
                        color: vec4(1.0, 1.0, 1.0, 0.33),
                        duration: 900,
                        start_time: time,
                        fade_in_time: 0,
                        flags: LE_PUFF_DONT_SCALE,
                        shader,
                    },
                );
                if self.error.is_none() {
                    self.error = result.err().map(|_| PresentError::state("shotgun smoke failed"));
                }
            }
        }
        let smoke_enabled = !self.host.weapon_settings().hardware_rage_pro;
        let effects_registry = self.registry.effects.clone();
        let product = state.product;
        let mut adapter = Adapter {
            host: &mut *self.host,
            state: &*state,
            pool,
            frame: *frame,
            product,
            effects_registry,
            shooter: es.other_entity_num,
            smoke_enabled,
            error: None,
        };
        emit_shotgun_presentation(
            &mut adapter,
            &Q3ShotgunEvent {
                muzzle: es.pos.base,
                direction: es.origin2,
                seed: es.event_parm,
            },
        );
        if let Some(error) = adapter.error {
            return Err(error);
        }
        Ok(())
    }

    fn muzzle_point(&mut self, state: &mut ClientGameState, entity_num: i32) -> PresentResult<Option<Vec3>> {
        let snap = state
            .snap
            .clone()
            .ok_or_else(|| PresentError::state("CG_CalcMuzzlePoint: cg.snap == NULL"))?;
        if entity_num == snap.player_state.client_num {
            let origin = add3(snap.player_state.origin, vec3(0.0, 0.0, snap.player_state.viewheight));
            return Ok(Some(weapon_ma(
                origin,
                14.0,
                angle_vectors(snap.player_state.viewangles).forward,
            )));
        }
        let cent = state.entity_at(entity_num.max(0) as usize).clone();
        if !cent.current_valid {
            return Ok(None);
        }
        let anim = cent.current_state.legs_anim & !128;
        let height = if anim == PlayerAnimation::LEGS_WALKCR || anim == PlayerAnimation::LEGS_IDLECR {
            12.0
        } else {
            26.0
        };
        Ok(Some(weapon_ma(
            add3(cent.current_state.pos.base, vec3(0.0, 0.0, height)),
            14.0,
            angle_vectors(cent.current_state.apos.base).forward,
        )))
    }

    /// Bullet impact (`bullet`).
    pub fn bullet(
        &mut self,
        state: &mut ClientGameState,
        pool: &mut LocalEntityPool,
        frame: &EffectFrame,
        end: Vec3,
        source: i32,
        hit: BulletHit,
    ) -> PresentResult<()> {
        if source >= 0 && self.host.weapon_settings().tracer_chance > 0.0 {
            if let Some(start) = self.muzzle_point(state, source)? {
                let a = self.host.collision_contents(start);
                let b = self.host.collision_contents(end);
                if a == b && a & CONTENTS_WATER != 0 {
                    self.host.weapon_effects().bubble_trail(pool, frame, start, end, 32.0)?;
                } else if a & CONTENTS_WATER != 0 {
                    let water = self.host.collision_trace(end, start, CONTENTS_WATER).end;
                    self.host
                        .weapon_effects()
                        .bubble_trail(pool, frame, start, water, 32.0)?;
                } else if b & CONTENTS_WATER != 0 {
                    let water = self.host.collision_trace(start, end, CONTENTS_WATER).end;
                    self.host.weapon_effects().bubble_trail(pool, frame, water, end, 32.0)?;
                }
                if self.host.random_f32() < self.host.weapon_settings().tracer_chance {
                    self.tracer(state, source, start, end);
                }
            }
        }
        match hit {
            BulletHit::Flesh { entity_num } => {
                self.host.weapon_effects().bleed(pool, frame, end, entity_num)?;
            }
            BulletHit::Wall { normal } => {
                self.missile_hit_wall(
                    state.product,
                    pool,
                    frame,
                    Weapon::Machinegun,
                    0,
                    end,
                    normal,
                    ImpactSound::Default,
                )?;
            }
        }
        Ok(())
    }

    /// Tracer (`tracer`).
    pub fn tracer(&mut self, state: &ClientGameState, _source: i32, source: Vec3, destination: Vec3) {
        let delta = sub3(destination, source);
        let length = length3(delta);
        if length < 100.0 {
            return;
        }
        let forward = normalize3(delta);
        let settings = self.host.weapon_settings();
        let begin = 50.0 + self.host.random_f32() * (length - 60.0);
        let end = (begin + settings.tracer_length).min(length);
        let start = weapon_ma(source, begin, forward);
        let finish = weapon_ma(source, end, forward);
        let axis = state.refdef.view_axis;
        let right = normalize3(weapon_ma(
            scale3(axis[1], dot3(forward, axis[2])),
            -dot3(forward, axis[1]),
            axis[2],
        ));
        let white = vec4(255.0, 255.0, 255.0, 255.0);
        let shader = self.host.weapon_media().shaders.tracer.clone();
        let tracer_sound = self.host.weapon_media().sounds.tracer.clone();
        self.host.add_poly(RefPoly {
            shader,
            vertices: vec![
                RefPolyVertex {
                    position: weapon_ma(finish, settings.tracer_width, right),
                    tex_coord: vec2(0.0, 1.0),
                    color: white,
                },
                RefPolyVertex {
                    position: weapon_ma(finish, -settings.tracer_width, right),
                    tex_coord: vec2(1.0, 0.0),
                    color: white,
                },
                RefPolyVertex {
                    position: weapon_ma(start, -settings.tracer_width, right),
                    tex_coord: vec2(1.0, 1.0),
                    color: white,
                },
                RefPolyVertex {
                    position: weapon_ma(start, settings.tracer_width, right),
                    tex_coord: vec2(0.0, 0.0),
                    color: white,
                },
            ],
        });
        self.host
            .start_sound(Some(scale3(add3(start, finish), 0.5)), 1022, 0, tracer_sound);
    }

    fn lightning_bolt(
        &mut self,
        state: &mut ClientGameState,
        target: PresentEntityTarget,
        origin: Vec3,
    ) -> PresentResult<()> {
        let cent = match target {
            PresentEntityTarget::Predicted => state.predicted_player_entity.clone(),
            PresentEntityTarget::Indexed(index) => state.entity_at(index).clone(),
        };
        if cent.current_state.weapon != Weapon::Lightning as i32 {
            return Ok(());
        }
        let mut angles = cent.lerp_angles;
        let true_lightning = self.host.weapon_settings().true_lightning;
        if cent.current_state.number == state.predicted_player_state.client_num && true_lightning != 0.0 {
            let view = state.refdef_view_angles;
            let blend = |actual: f32, view: f32| {
                let mut a = actual - view;
                if a > 180.0 {
                    a -= 360.0;
                }
                if a < -180.0 {
                    a += 360.0;
                }
                let mut angle = view + a * (1.0 - true_lightning);
                if angle < 0.0 {
                    angle += 360.0;
                }
                if angle > 360.0 {
                    angle -= 360.0;
                }
                angle
            };
            angles = vec3(
                blend(angles.x, view.x),
                blend(angles.y, view.y),
                blend(angles.z, view.z),
            );
        }
        let forward = angle_vectors(angles).forward;
        let muzzle = weapon_ma(add3(cent.lerp_origin, vec3(0.0, 0.0, 26.0)), 14.0, forward);
        let zero = zero_vec3();
        let trace = self.host.trace_mover(
            muzzle,
            weapon_ma(muzzle, 768.0, forward),
            Bounds { min: zero, max: zero },
            cent.current_state.number,
            MASK_SHOT,
        );
        let mut beam = create_lightning_entity();
        beam.origin = origin;
        beam.old_origin = trace.base.end;
        beam.shading.custom_shader = self.registry.effects.lightning_shader.clone();
        self.host.add_ref_entity(RefEntity::Lightning(beam.clone()));
        if trace.base.fraction < 1.0 {
            let mut re = create_model_entity(self.registry.effects.lightning_explosion_model.clone());
            re.origin = weapon_ma(trace.base.end, -16.0, normalize3(sub3(beam.old_origin, beam.origin)));
            re.axis = angles_to_axis(vec3(
                (self.host.rand_i32() % 360) as f32,
                (self.host.rand_i32() % 360) as f32,
                (self.host.rand_i32() % 360) as f32,
            ));
            self.host.add_ref_entity(RefEntity::Model(re));
        }
        Ok(())
    }

    fn spin_angle(&mut self, state: &mut ClientGameState, target: PresentEntityTarget) -> f32 {
        let player = match target {
            PresentEntityTarget::Predicted => state.predicted_player_entity.player,
            PresentEntityTarget::Indexed(index) => state.entity_at(index).player,
        };
        let mut delta = state.time.wrapping_sub(player.barrel_time);
        let angle = if player.barrel_spinning {
            player.barrel_angle + delta as f32 * 0.9
        } else {
            if delta > 1000 {
                delta = 1000;
            }
            let speed = 0.5 * (0.9 + ((1000 - delta) as f32) / 1000.0);
            player.barrel_angle + delta as f32 * speed
        };
        let current = match target {
            PresentEntityTarget::Predicted => state.predicted_player_entity.current_state.clone(),
            PresentEntityTarget::Indexed(index) => state.entity_at(index).current_state.clone(),
        };
        let firing = current.e_flags & 256 != 0;
        if player.barrel_spinning != firing {
            let time = state.time;
            let entity = match target {
                PresentEntityTarget::Predicted => &mut state.predicted_player_entity,
                PresentEntityTarget::Indexed(index) => state.entity_at(index),
            };
            entity.player.barrel_time = time;
            entity.player.barrel_angle = angle_mod(f64::from(angle)) as f32;
            entity.player.barrel_spinning = firing;
            if state.product == Product::MissionPack && current.weapon == Weapon::Chaingun as i32 && !firing {
                let sound = self.host.load_sound("sound/weapons/vulcan/wvulwind.wav");
                self.host.start_sound(None, current.number, 2, sound);
            }
        }
        angle
    }

    fn add_weapon_with_powerups(&mut self, gun: &RefModelEntity, powerups: i32) {
        let shaders = self.host.weapon_media().shaders.clone();
        if powerups & (1 << Powerup::Invis as i32) != 0 {
            let mut gun = gun.clone();
            gun.shading.custom_shader = shaders.invis;
            self.host.add_ref_entity(RefEntity::Model(gun));
            return;
        }
        self.host.add_ref_entity(RefEntity::Model(gun.clone()));
        if powerups & (1 << Powerup::Battlesuit as i32) != 0 {
            let mut gun = gun.clone();
            gun.shading.custom_shader = shaders.battle_weapon;
            self.host.add_ref_entity(RefEntity::Model(gun));
        }
        if powerups & (1 << Powerup::Quad as i32) != 0 {
            let mut gun = gun.clone();
            gun.shading.custom_shader = shaders.quad_weapon;
            self.host.add_ref_entity(RefEntity::Model(gun));
        }
    }

    /// Add a player weapon (`addPlayerWeapon`).
    #[allow(clippy::too_many_arguments)]
    pub fn add_player_weapon(
        &mut self,
        state: &mut ClientGameState,
        pool: &mut LocalEntityPool,
        frame: &EffectFrame,
        parent: &RefModelEntity,
        ps: Option<&SourcePlayerState>,
        target: PresentEntityTarget,
        _team: Team,
    ) -> PresentResult<()> {
        let cent = match target {
            PresentEntityTarget::Predicted => state.predicted_player_entity.clone(),
            PresentEntityTarget::Indexed(index) => state.entity_at(index).clone(),
        };
        let weapon_num = cent.current_state.weapon;
        let weapon = self.registry.require_weapon(weapon_num)?.clone();
        let attached = |model: SceneModel| {
            let mut re = create_model_entity(model);
            re.lighting_origin = parent.lighting_origin;
            re.shadow_plane = parent.shadow_plane;
            re.shading.render_flags = parent.shading.render_flags;
            re
        };
        let mut gun = attached(weapon.packet.weapon_model.clone());
        if ps.is_some() {
            if state.predicted_player_state.weapon == Weapon::Railgun as i32
                && state.predicted_player_state.weapon_state == WeaponState::Firing
            {
                let fraction = state.predicted_player_state.weapon_time as f32 / 1500.0;
                let color = (qvm_float_to_int(255.0 * (1.0 - fraction)) & 255) as f32;
                gun.shading.shader_rgba = vec4(color, 0.0, color, 0.0);
            } else {
                gun.shading.shader_rgba = vec4(255.0, 255.0, 255.0, 255.0);
            }
        }
        if gun.model.is_default() {
            return Ok(());
        }
        if ps.is_none() {
            let entity = match target {
                PresentEntityTarget::Predicted => &mut state.predicted_player_entity,
                PresentEntityTarget::Indexed(index) => state.entity_at(index),
            };
            entity.player.lightning_firing = 0;
            if cent.current_state.e_flags & 256 != 0 && weapon.firing_sound.is_some() {
                self.host.add_loop_sound(
                    cent.current_state.number,
                    cent.lerp_origin,
                    zero_vec3(),
                    weapon.firing_sound.clone(),
                    false,
                );
                let entity = match target {
                    PresentEntityTarget::Predicted => &mut state.predicted_player_entity,
                    PresentEntityTarget::Indexed(index) => state.entity_at(index),
                };
                entity.player.lightning_firing = 1;
            } else if weapon.ready_sound.is_some() {
                self.host.add_loop_sound(
                    cent.current_state.number,
                    cent.lerp_origin,
                    zero_vec3(),
                    weapon.ready_sound.clone(),
                    false,
                );
            }
        }
        position_entity_on_tag(&mut gun, parent, &parent.model, "tag_weapon");
        self.add_weapon_with_powerups(&gun, cent.current_state.powerups);
        if let Some(barrel_model) = weapon.packet.barrel_model.clone() {
            let mut barrel = attached(barrel_model);
            let spin = self.spin_angle(state, target);
            barrel.axis = angles_to_axis(vec3(0.0, 0.0, spin));
            position_rotated_entity_on_tag(&mut barrel, &gun, &weapon.packet.weapon_model, "tag_barrel");
            self.add_weapon_with_powerups(&barrel, cent.current_state.powerups);
        }
        let non_predicted_index = cent.current_state.client_num.max(0) as usize;
        let non_predicted = state.entity_at(non_predicted_index).clone();
        if !((weapon_num == Weapon::Lightning as i32
            || weapon_num == Weapon::Gauntlet as i32
            || weapon_num == Weapon::GrapplingHook as i32)
            && non_predicted.current_state.e_flags & 256 != 0)
        {
            let railgun_flash = match target {
                PresentEntityTarget::Predicted => state.predicted_player_entity.player.railgun_flash,
                PresentEntityTarget::Indexed(index) => state.entity_at(index).player.railgun_flash,
            };
            if state.time.wrapping_sub(cent.muzzle_flash_time) > 20 && !railgun_flash {
                return Ok(());
            }
        }
        let mut flash = attached(weapon.flash_model.clone());
        if flash.model.is_default() {
            return Ok(());
        }
        flash.axis = angles_to_axis(vec3(0.0, 0.0, self.host.crandom_f32() * 10.0));
        if weapon_num == Weapon::Railgun as i32 {
            let color = self.host.client_info_view(cent.current_state.client_num).color1;
            flash.shading.shader_rgba = weapon_bytes(color, 255.0, 0.0);
        }
        position_rotated_entity_on_tag(&mut flash, &gun, &weapon.packet.weapon_model, "tag_flash");
        self.host.add_ref_entity(RefEntity::Model(flash.clone()));
        if ps.is_some()
            || state.rendering_third_person
            || cent.current_state.number != state.predicted_player_state.client_num
        {
            self.lightning_bolt(state, PresentEntityTarget::Indexed(non_predicted_index), flash.origin)?;
            if weapon_num == Weapon::Railgun as i32 {
                let railgun_flash = match target {
                    PresentEntityTarget::Predicted => state.predicted_player_entity.player.railgun_flash,
                    PresentEntityTarget::Indexed(index) => state.entity_at(index).player.railgun_flash,
                };
                if railgun_flash {
                    let impact = match target {
                        PresentEntityTarget::Predicted => state.predicted_player_entity.player.railgun_impact,
                        PresentEntityTarget::Indexed(index) => state.entity_at(index).player.railgun_impact,
                    };
                    {
                        let entity = match target {
                            PresentEntityTarget::Predicted => &mut state.predicted_player_entity,
                            PresentEntityTarget::Indexed(index) => state.entity_at(index),
                        };
                        entity.player.railgun_flash = true;
                    }
                    let mut start = flash.origin;
                    self.rail_trail(
                        state.time,
                        pool,
                        frame,
                        cent.current_state.client_num,
                        &mut start,
                        impact,
                    )?;
                }
            }
            let color = weapon.flash_dlight_color;
            if color.x != 0.0 || color.y != 0.0 || color.z != 0.0 {
                let radius = 300.0 + (self.host.rand_i32() & 31) as f32;
                self.host.add_light(DynamicLight {
                    origin: flash.origin,
                    radius,
                    color,
                    additive: false,
                });
            }
        }
        Ok(())
    }

    fn weapon_position(&self, state: &ClientGameState) -> (Vec3, Vec3) {
        let scale = if state.bob_cycle & 1 != 0 {
            -state.xyspeed
        } else {
            state.xyspeed
        };
        let roll = scale * state.bob_frac_sin * 0.005;
        let yaw = scale * state.bob_frac_sin * 0.01;
        let pitch = state.xyspeed * state.bob_frac_sin * 0.005;
        let mut origin = state.refdef.view_origin;
        let mut angles = add3(state.refdef_view_angles, vec3(pitch, yaw, roll));
        let delta = state.time.wrapping_sub(state.land_time);
        if delta < 150 {
            origin = add3(origin, vec3(0.0, 0.0, state.land_change * 0.25 * delta as f32 / 150.0));
        } else if delta < 450 {
            origin = add3(
                origin,
                vec3(0.0, 0.0, state.land_change * 0.25 * (450 - delta) as f32 / 300.0),
            );
        }
        let drift = (state.xyspeed + 40.0) * (state.time as f32 * 0.001).sin() * 0.01;
        angles = add3(angles, vec3(drift, drift, drift));
        (origin, angles)
    }

    fn map_torso_frame(&mut self, client_num: i32, frame: i32) -> PresentResult<i32> {
        let animations = self.host.client_info_view(client_num).animations;
        for index in [
            PlayerAnimation::TORSO_DROP,
            PlayerAnimation::TORSO_ATTACK,
            PlayerAnimation::TORSO_ATTACK2,
        ] {
            let animation = animations
                .get(index)
                .and_then(|slot| *slot)
                .ok_or_else(|| PresentError::state(format!("Missing weapon torso animation {index}")))?;
            let length = if index == PlayerAnimation::TORSO_DROP { 9 } else { 6 };
            if frame >= animation.first_frame && frame < animation.first_frame + length {
                return Ok(frame - animation.first_frame + if index == PlayerAnimation::TORSO_DROP { 6 } else { 1 });
            }
        }
        Ok(0)
    }

    /// Add the view weapon (`addViewWeapon`).
    pub fn add_view_weapon(
        &mut self,
        state: &mut ClientGameState,
        pool: &mut LocalEntityPool,
        frame: &EffectFrame,
        ps: &SourcePlayerState,
    ) -> PresentResult<()> {
        if ps.persistant.get(PersistentIndex::PERS_TEAM) == Team::Spectator as i32
            || ps.pm_type == MoveType::Intermission
            || state.rendering_third_person
        {
            return Ok(());
        }
        let settings = self.host.weapon_settings();
        if !settings.draw_gun {
            if state.predicted_player_state.e_flags & 256 != 0 {
                let origin = weapon_ma(state.refdef.view_origin, -8.0, state.refdef.view_axis[2]);
                let target = PresentEntityTarget::Indexed(ps.client_num.max(0) as usize);
                self.lightning_bolt(state, target, origin)?;
            }
            return Ok(());
        }
        if state.test_gun {
            return Ok(());
        }
        let offset = if settings.fov > 90.0 {
            -0.2 * (settings.fov - 90.0)
        } else {
            0.0
        };
        let cent = state.predicted_player_entity.clone();
        let weapon = self.registry.require_weapon(ps.weapon)?.clone();
        let (position_origin, position_angles) = self.weapon_position(state);
        let mut hand = create_model_entity(weapon.hands_model.clone());
        hand.origin = weapon_ma(
            weapon_ma(
                weapon_ma(position_origin, settings.gun_x, state.refdef.view_axis[0]),
                settings.gun_y,
                state.refdef.view_axis[1],
            ),
            settings.gun_z + offset,
            state.refdef.view_axis[2],
        );
        hand.axis = angles_to_axis(position_angles);
        if settings.gun_frame != 0 {
            hand.frame = settings.gun_frame;
            hand.old_frame = settings.gun_frame;
            hand.back_lerp = 0.0;
        } else {
            let client_num = cent.current_state.client_num;
            hand.frame = self.map_torso_frame(client_num, cent.player.torso.frame)?;
            hand.old_frame = self.map_torso_frame(client_num, cent.player.torso.old_frame)?;
            hand.back_lerp = cent.player.torso.back_lerp;
        }
        hand.shading.render_flags = RF_DEPTHHACK | RF_FIRST_PERSON | RF_MINLIGHT;
        let team = ps.persistant.get(PersistentIndex::PERS_TEAM);
        if Team::from_i32(team).is_none() {
            return Err(PresentError::range("Invalid view weapon team"));
        }
        let team = Team::from_i32(team).unwrap_or(Team::Free);
        let ps_owned = ps.clone();
        self.add_player_weapon(
            state,
            pool,
            frame,
            &hand,
            Some(&ps_owned),
            PresentEntityTarget::Predicted,
            team,
        )
    }

    /// Draw the weapon selection (`drawWeaponSelect`).
    pub fn draw_weapon_select(&mut self, state: &mut ClientGameState) -> PresentResult<()> {
        if state.predicted_player_state.health <= 0 {
            return Ok(());
        }
        let color = match self.host.drawing().fade_color(state.weapon_select_time, 1400) {
            Some(color) => color,
            None => return Ok(()),
        };
        self.host.drawing().set_color(Some(color));
        state.item_pickup_time = 0;
        let snap = state
            .snap
            .clone()
            .ok_or_else(|| PresentError::state("CG_DrawWeaponSelect: cg.snap == NULL"))?;
        let bits = snap.player_state.stats.get(stat_schema(state.product).weapons);
        let mut count = 0;
        for i in 1..16 {
            if bits & (1 << i) != 0 {
                count += 1;
            }
        }
        let mut x = 320 - count * 20;
        for i in 1..16 {
            if bits & (1 << i) == 0 {
                continue;
            }
            let icon = self.registry.require_weapon(i)?.weapon_icon.clone();
            self.host.drawing().draw_pic(x, 380, 32, 32, icon);
            if i == state.weapon_select {
                let select = self.host.weapon_media().shaders.select.clone();
                self.host.drawing().draw_pic(x - 4, 376, 40, 40, select);
            }
            if snap.player_state.ammo.get(i as usize) == 0 {
                let noammo = self.host.weapon_media().shaders.noammo.clone();
                self.host.drawing().draw_pic(x, 380, 32, 32, noammo);
            }
            x += 40;
        }
        let item = self.registry.weapon(state.weapon_select)?.item.clone();
        if let Some(item) = item {
            if let Some(name) = item.pickup_name {
                let width = self.host.drawing().draw_string_length(&name) * 16;
                self.host
                    .drawing()
                    .draw_big_string_color((640 - width as i32) / 2, 358, &name, color);
            }
        }
        self.host.drawing().set_color(None);
        Ok(())
    }
}

/// Registered weapon effects (`RegisteredWeaponEffects`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RegisteredWeaponEffects {
    /// Lightning shader.
    pub lightning_shader: Option<SceneShader>,
    /// Lightning explosion model.
    pub lightning_explosion_model: SceneModel,
    /// Lightning hit sounds.
    pub lightning_hit_sounds: [Option<PresentSound>; 3],
    /// Bullet explosion shader.
    pub bullet_explosion_shader: Option<SceneShader>,
    /// Rocket explosion shader.
    pub rocket_explosion_shader: Option<SceneShader>,
    /// Grenade explosion shader.
    pub grenade_explosion_shader: Option<SceneShader>,
    /// Plasma explosion shader.
    pub plasma_explosion_shader: Option<SceneShader>,
    /// Rail explosion shader.
    pub rail_explosion_shader: Option<SceneShader>,
    /// BFG explosion shader.
    pub bfg_explosion_shader: Option<SceneShader>,
    /// Rail rings shader.
    pub rail_rings_shader: Option<SceneShader>,
    /// Rail core shader.
    pub rail_core_shader: Option<SceneShader>,
}

impl Default for SceneModel {
    fn default() -> Self {
        Self::default_model()
    }
}

/// Renderer resources (`RendererResources`, minimal mirror, synchronous).
pub trait PresentRendererResources {
    /// Register a model.
    fn register_model(&mut self, path: &str) -> SceneModel;
    /// Register a skin.
    fn register_skin(&mut self, path: &str) -> Option<SceneSkin>;
    /// Register a shader.
    fn register_shader(&mut self, name: &str) -> Option<SceneShader>;
    /// Register a shader without mipmaps.
    fn register_shader_no_mip(&mut self, name: &str) -> Option<SceneShader>;
    /// Load a world.
    fn load_world(&mut self, mapname: &str) -> PresentWorldScene;
    /// Load particle animations.
    fn load_particle_animations(&mut self) -> PresentParticleAnimations;
}

/// Loaded world scene (`WorldScene`, minimal mirror).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresentWorldScene {
    /// Submodel count.
    pub model_count: usize,
}

/// Particle animations (`ParticleAnimations`, minimal mirror).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresentParticleAnimations {
    /// Animation names.
    pub names: Vec<String>,
}

/// Weapon registration audio (`WeaponRegistrationAudio`, synchronous).
pub trait WeaponRegistrationAudio {
    /// Register a sound (uncompressed).
    fn register_sound(&mut self, path: &str) -> Option<PresentSound>;
}

/// Client weapon media registry (`ClientWeaponMediaRegistry`).
pub struct ClientWeaponMediaRegistry {
    /// Product.
    pub product: Product,
    /// Resources.
    pub resources: Box<dyn PresentRendererResources>,
    /// Audio.
    pub audio: Box<dyn WeaponRegistrationAudio>,
    /// Effects.
    pub effects: RegisteredWeaponEffects,
    weapon_records: [ClientWeaponInfo; 16],
    item_records: Vec<PacketItemVisual>,
    registered_weapons: HashSet<i32>,
    ready_weapons: HashSet<i32>,
    registered_items: HashSet<i32>,
}

impl ClientWeaponMediaRegistry {
    /// New registry.
    pub fn new(
        product: Product,
        resources: Box<dyn PresentRendererResources>,
        audio: Box<dyn WeaponRegistrationAudio>,
    ) -> Self {
        Self {
            product,
            resources,
            audio,
            effects: RegisteredWeaponEffects {
                lightning_explosion_model: default_model(),
                ..RegisteredWeaponEffects::default()
            },
            weapon_records: std::array::from_fn(|_| empty_weapon()),
            item_records: Vec::new(),
            registered_weapons: HashSet::new(),
            ready_weapons: HashSet::new(),
            registered_items: HashSet::new(),
        }
    }

    /// Weapons.
    #[must_use]
    pub fn weapons(&self) -> &[ClientWeaponInfo; 16] {
        &self.weapon_records
    }

    /// Items.
    #[must_use]
    pub fn items(&self) -> &[PacketItemVisual] {
        &self.item_records
    }

    /// Weapon by number.
    pub fn weapon(&self, number: i32) -> PresentResult<&ClientWeaponInfo> {
        self.weapon_records
            .get(number as usize)
            .ok_or_else(|| PresentError::range(format!("Invalid weapon media index {number}")))
    }

    /// Require a registered weapon.
    pub fn require_weapon(&self, number: i32) -> PresentResult<&ClientWeaponInfo> {
        if number != 0 && !self.ready_weapons.contains(&number) {
            return Err(PresentError::state(format!(
                "Weapon {number} must finish registration before synchronous presentation"
            )));
        }
        self.weapon(number)
    }

    /// Register a weapon (`registerWeapon`).
    pub fn register_weapon(&mut self, number: i32, items: &dyn PresentItemTable) -> PresentResult<()> {
        self.register_weapon_now(number, items)
    }

    /// Register item visuals (`registerItemVisuals`).
    pub fn register_item_visuals(&mut self, number: i32, items: &dyn PresentItemTable) -> PresentResult<()> {
        self.register_item_now(number, items)
    }

    fn ensure_items(&mut self, items: &dyn PresentItemTable) {
        let count = items.item_count(self.product);
        if self.item_records.len() < count {
            self.item_records.resize_with(count, || PacketItemVisual {
                models: [default_model(), default_model()],
                has_second: false,
                icon: None,
            });
        }
    }

    fn register_item_now(&mut self, number: i32, items: &dyn PresentItemTable) -> PresentResult<()> {
        let count = items.item_count(self.product);
        if number < 0 || number as usize >= count {
            return Err(PresentError::drop(format!(
                "CG_RegisterItemVisuals: itemNum {number} out of range [0-{}]",
                count.saturating_sub(1)
            )));
        }
        let item = items.item_at(self.product, number as usize).ok_or_else(|| {
            PresentError::drop(format!(
                "CG_RegisterItemVisuals: itemNum {number} out of range [0-{}]",
                count.saturating_sub(1)
            ))
        })?;
        if self.registered_items.contains(&number) {
            return Ok(());
        }
        self.registered_items.insert(number);
        self.ensure_items(items);
        let model = match &item.world_models[0] {
            None => default_model(),
            Some(path) => self.resources.register_model(path),
        };
        let icon = match &item.icon {
            None => None,
            Some(icon) => self.resources.register_shader(icon),
        };
        self.item_records[number as usize] = PacketItemVisual {
            models: [model.clone(), default_model()],
            has_second: false,
            icon: icon.clone(),
        };
        if item.item_type == ItemType::Weapon {
            self.register_weapon_now(item.tag, items)?;
        }
        if (item.item_type == ItemType::Powerup
            || item.item_type == ItemType::Health
            || item.item_type == ItemType::Armor
            || item.item_type == ItemType::Holdable)
            && item.world_models[1].is_some()
        {
            let second = self
                .resources
                .register_model(item.world_models[1].as_ref().unwrap_or(&String::new()).as_str());
            self.item_records[number as usize] = PacketItemVisual {
                models: [model, second],
                has_second: true,
                icon,
            };
        }
        Ok(())
    }

    fn register_weapon_now(&mut self, number: i32, items: &dyn PresentItemTable) -> PresentResult<()> {
        self.weapon(number)?;
        if number == 0 || self.registered_weapons.contains(&number) {
            return Ok(());
        }
        self.registered_weapons.insert(number);
        let count = items.item_count(self.product);
        let mut found = None;
        for index in 0..count {
            if let Some(item) = items.item_at(self.product, index) {
                if item.item_type == ItemType::Weapon && item.tag == number {
                    found = Some((index, item));
                    break;
                }
            }
        }
        let (index, item) = found.ok_or_else(|| PresentError::drop(format!("Couldn't find weapon {number}")))?;
        let mut weapon = empty_weapon();
        weapon.item = Some(item.clone());
        self.weapon_records[number as usize] = weapon;
        self.register_item_now(index as i32, items)?;
        let path = item.world_models[0].clone();
        let icon = item.icon.clone();
        let (Some(path), Some(icon)) = (path, icon) else {
            return Err(PresentError::state(format!(
                "Weapon {number} has no world model or icon"
            )));
        };
        let model = self.resources.register_model(&path);
        let bounds = model_bounds(&model);
        let midpoint = vec3(
            bounds.min.x + 0.5 * (bounds.max.x - bounds.min.x),
            bounds.min.y + 0.5 * (bounds.max.y - bounds.min.y),
            bounds.min.z + 0.5 * (bounds.max.z - bounds.min.z),
        );
        {
            let weapon = &mut self.weapon_records[number as usize];
            weapon.packet.weapon_model = model;
            weapon.packet.weapon_midpoint = midpoint;
        }
        let weapon_icon = self.resources.register_shader(&icon);
        let ammo_icon = self.resources.register_shader(&icon);
        {
            let weapon = &mut self.weapon_records[number as usize];
            weapon.weapon_icon = weapon_icon;
            weapon.ammo_icon = ammo_icon;
        }
        for index in 0..count {
            if let Some(candidate) = items.item_at(self.product, index) {
                if candidate.item_type == ItemType::Ammo && candidate.tag == number {
                    if let Some(ammo_path) = candidate.world_models[0].clone() {
                        let ammo_model = self.resources.register_model(&ammo_path);
                        self.weapon_records[number as usize].ammo_model = ammo_model;
                    }
                    break;
                }
            }
        }
        let stem = path
            .find('.')
            .map(|dot| path[..dot].to_string())
            .unwrap_or(path.clone());
        let flash = self.resources.register_model(&format!("{stem}_flash.md3"));
        let barrel = self.resources.register_model(&format!("{stem}_barrel.md3"));
        let mut hands = self.resources.register_model(&format!("{stem}_hand.md3"));
        if hands.is_default() {
            hands = self
                .resources
                .register_model("models/weapons2/shotgun/shotgun_hand.md3");
        }
        {
            let weapon = &mut self.weapon_records[number as usize];
            weapon.flash_model = flash;
            weapon.packet.barrel_model = if barrel.is_default() { None } else { Some(barrel) };
            weapon.hands_model = hands;
        }
        self.register_weapon_specific(number)?;
        self.ready_weapons.insert(number);
        Ok(())
    }

    fn register_weapon_specific(&mut self, number: i32) -> PresentResult<()> {
        let weapon = Weapon::from_i32(number);
        match weapon {
            Some(Weapon::Gauntlet) => {
                self.weapon_records[number as usize].flash_dlight_color = vec3(0.6, 0.6, 1.0);
                let firing = self.audio.register_sound("sound/weapons/melee/fstrun.wav");
                let flash = self.audio.register_sound("sound/weapons/melee/fstatck.wav");
                let weapon = &mut self.weapon_records[number as usize];
                weapon.firing_sound = firing;
                weapon.flash_sounds = [flash, None, None, None];
            }
            Some(Weapon::Lightning) => {
                self.weapon_records[number as usize].flash_dlight_color = vec3(0.6, 0.6, 1.0);
                let ready = self.audio.register_sound("sound/weapons/melee/fsthum.wav");
                let firing = self.audio.register_sound("sound/weapons/lightning/lg_hum.wav");
                let flash = self.audio.register_sound("sound/weapons/lightning/lg_fire.wav");
                let shader = self.resources.register_shader("lightningBoltNew");
                let model = self.resources.register_model("models/weaphits/crackle.md3");
                let hit = [
                    self.audio.register_sound("sound/weapons/lightning/lg_hit.wav"),
                    self.audio.register_sound("sound/weapons/lightning/lg_hit2.wav"),
                    self.audio.register_sound("sound/weapons/lightning/lg_hit3.wav"),
                ];
                let weapon = &mut self.weapon_records[number as usize];
                weapon.ready_sound = ready;
                weapon.firing_sound = firing;
                weapon.flash_sounds = [flash, None, None, None];
                self.effects.lightning_shader = shader;
                self.effects.lightning_explosion_model = model;
                self.effects.lightning_hit_sounds = hit;
            }
            Some(Weapon::GrapplingHook) => {
                let shader = self.resources.register_shader("lightningBoltNew");
                let model = self.resources.register_model("models/ammo/rocket/rocket.md3");
                let ready = self.audio.register_sound("sound/weapons/melee/fsthum.wav");
                let firing = self.audio.register_sound("sound/weapons/melee/fstrun.wav");
                let weapon = &mut self.weapon_records[number as usize];
                weapon.flash_dlight_color = vec3(0.6, 0.6, 1.0);
                weapon.packet.missile_model = model;
                weapon.packet.missile_trail = Some(MissileTrail::Grapple);
                weapon.packet.missile_dlight = 200.0;
                weapon.packet.trail_time = 2000;
                weapon.packet.trail_radius = 64.0;
                weapon.packet.missile_dlight_color = vec3(1.0, 0.75, 0.0);
                weapon.ready_sound = ready;
                weapon.firing_sound = firing;
                self.effects.lightning_shader = shader;
            }
            Some(Weapon::Chaingun) => {
                let firing = self.audio.register_sound("sound/weapons/vulcan/wvulfire.wav");
                let flashes = [
                    self.audio.register_sound("sound/weapons/vulcan/vulcanf1b.wav"),
                    self.audio.register_sound("sound/weapons/vulcan/vulcanf2b.wav"),
                    self.audio.register_sound("sound/weapons/vulcan/vulcanf3b.wav"),
                    self.audio.register_sound("sound/weapons/vulcan/vulcanf4b.wav"),
                ];
                let shader = self.resources.register_shader("bulletExplosion");
                let weapon = &mut self.weapon_records[number as usize];
                weapon.firing_sound = firing;
                weapon.loop_fire_sound = true;
                weapon.flash_dlight_color = vec3(1.0, 1.0, 0.0);
                weapon.flash_sounds = flashes;
                weapon.eject_brass = Some(EjectBrass::Machinegun);
                self.effects.bullet_explosion_shader = shader;
            }
            Some(Weapon::Machinegun) => {
                let flashes = [
                    self.audio.register_sound("sound/weapons/machinegun/machgf1b.wav"),
                    self.audio.register_sound("sound/weapons/machinegun/machgf2b.wav"),
                    self.audio.register_sound("sound/weapons/machinegun/machgf3b.wav"),
                    self.audio.register_sound("sound/weapons/machinegun/machgf4b.wav"),
                ];
                let shader = self.resources.register_shader("bulletExplosion");
                let weapon = &mut self.weapon_records[number as usize];
                weapon.flash_dlight_color = vec3(1.0, 1.0, 0.0);
                weapon.flash_sounds = flashes;
                weapon.eject_brass = Some(EjectBrass::Machinegun);
                self.effects.bullet_explosion_shader = shader;
            }
            Some(Weapon::Shotgun) => {
                let flash = self.audio.register_sound("sound/weapons/shotgun/sshotf1b.wav");
                let weapon = &mut self.weapon_records[number as usize];
                weapon.flash_dlight_color = vec3(1.0, 1.0, 0.0);
                weapon.flash_sounds = [flash, None, None, None];
                weapon.eject_brass = Some(EjectBrass::Shotgun);
            }
            Some(Weapon::RocketLauncher) => {
                let model = self.resources.register_model("models/ammo/rocket/rocket.md3");
                let sound = self.audio.register_sound("sound/weapons/rocket/rockfly.wav");
                let flash = self.audio.register_sound("sound/weapons/rocket/rocklf1a.wav");
                let shader = self.resources.register_shader("rocketExplosion");
                let weapon = &mut self.weapon_records[number as usize];
                weapon.packet.missile_model = model;
                weapon.packet.missile_sound = sound;
                weapon.packet.missile_trail = Some(MissileTrail::Rocket);
                weapon.packet.missile_dlight = 200.0;
                weapon.packet.trail_time = 2000;
                weapon.packet.trail_radius = 64.0;
                weapon.packet.missile_dlight_color = vec3(1.0, 0.75, 0.0);
                weapon.flash_dlight_color = vec3(1.0, 0.75, 0.0);
                weapon.flash_sounds = [flash, None, None, None];
                self.effects.rocket_explosion_shader = shader;
            }
            Some(Weapon::ProxLauncher) | Some(Weapon::GrenadeLauncher) => {
                let prox = weapon == Some(Weapon::ProxLauncher);
                let model = self.resources.register_model(if prox {
                    "models/weaphits/proxmine.md3"
                } else {
                    "models/ammo/grenade1.md3"
                });
                let flash = self.audio.register_sound(if prox {
                    "sound/weapons/proxmine/wstbfire.wav"
                } else {
                    "sound/weapons/grenade/grenlf1a.wav"
                });
                let shader = self.resources.register_shader("grenadeExplosion");
                let weapon_record = &mut self.weapon_records[number as usize];
                weapon_record.packet.missile_model = model;
                weapon_record.packet.missile_trail = Some(MissileTrail::Grenade);
                weapon_record.packet.trail_time = 700;
                weapon_record.packet.trail_radius = 32.0;
                weapon_record.flash_dlight_color = vec3(1.0, 0.7, 0.0);
                weapon_record.flash_sounds = [flash, None, None, None];
                self.effects.grenade_explosion_shader = shader;
            }
            Some(Weapon::Nailgun) => {
                let model = self.resources.register_model("models/weaphits/nail.md3");
                let flash = self.audio.register_sound("sound/weapons/nailgun/wnalfire.wav");
                let weapon = &mut self.weapon_records[number as usize];
                weapon.eject_brass = Some(EjectBrass::Nailgun);
                weapon.packet.missile_trail = Some(MissileTrail::Nail);
                weapon.packet.trail_radius = 16.0;
                weapon.packet.trail_time = 250;
                weapon.packet.missile_model = model;
                weapon.flash_dlight_color = vec3(1.0, 0.75, 0.0);
                weapon.flash_sounds = [flash, None, None, None];
            }
            Some(Weapon::Plasmagun) => {
                let sound = self.audio.register_sound("sound/weapons/plasma/lasfly.wav");
                let flash = self.audio.register_sound("sound/weapons/plasma/hyprbf1a.wav");
                let plasma = self.resources.register_shader("plasmaExplosion");
                let rings = self.resources.register_shader("railDisc");
                let weapon = &mut self.weapon_records[number as usize];
                weapon.packet.missile_trail = Some(MissileTrail::Plasma);
                weapon.packet.missile_sound = sound;
                weapon.flash_dlight_color = vec3(0.6, 0.6, 1.0);
                weapon.flash_sounds = [flash, None, None, None];
                self.effects.plasma_explosion_shader = plasma;
                self.effects.rail_rings_shader = rings;
            }
            Some(Weapon::Railgun) => {
                let ready = self.audio.register_sound("sound/weapons/railgun/rg_hum.wav");
                let flash = self.audio.register_sound("sound/weapons/railgun/railgf1a.wav");
                let explosion = self.resources.register_shader("railExplosion");
                let rings = self.resources.register_shader("railDisc");
                let core = self.resources.register_shader("railCore");
                let weapon = &mut self.weapon_records[number as usize];
                weapon.ready_sound = ready;
                weapon.flash_dlight_color = vec3(1.0, 0.5, 0.0);
                weapon.flash_sounds = [flash, None, None, None];
                self.effects.rail_explosion_shader = explosion;
                self.effects.rail_rings_shader = rings;
                self.effects.rail_core_shader = core;
            }
            Some(Weapon::Bfg) => {
                let ready = self.audio.register_sound("sound/weapons/bfg/bfg_hum.wav");
                let flash = self.audio.register_sound("sound/weapons/bfg/bfg_fire.wav");
                let shader = self.resources.register_shader("bfgExplosion");
                let model = self.resources.register_model("models/weaphits/bfg.md3");
                let sound = self.audio.register_sound("sound/weapons/rocket/rockfly.wav");
                let weapon = &mut self.weapon_records[number as usize];
                weapon.ready_sound = ready;
                weapon.flash_dlight_color = vec3(1.0, 0.7, 1.0);
                weapon.flash_sounds = [flash, None, None, None];
                weapon.packet.missile_model = model;
                weapon.packet.missile_sound = sound;
                self.effects.bfg_explosion_shader = shader;
            }
            _ => {
                let flash = self.audio.register_sound("sound/weapons/rocket/rocklf1a.wav");
                let weapon = &mut self.weapon_records[number as usize];
                weapon.flash_dlight_color = vec3(1.0, 1.0, 1.0);
                weapon.flash_sounds = [flash, None, None, None];
            }
        }
        Ok(())
    }
}

/// Emit a weapon impact (`emitWeaponImpact`).
#[allow(clippy::too_many_arguments)]
pub fn emit_weapon_impact(
    product: Product,
    registry_effects: &RegisteredWeaponEffects,
    host: &mut dyn ClientWeaponHost,
    pool: &mut LocalEntityPool,
    frame: &EffectFrame,
    weapon: Weapon,
    client_num: i32,
    origin: Vec3,
    direction: Vec3,
    sound_type: ImpactSound,
) -> PresentResult<()> {
    let media = host.weapon_media().clone();
    let mark: Option<SceneShader>;
    let mut shader: Option<SceneShader> = None;
    let mut model = default_model();
    let mut sound: Option<PresentSound> = None;
    let radius: f32;
    let mut light = 0.0f32;
    let mut light_color = vec3(1.0, 1.0, 0.0);
    let mut sprite = false;
    let mut duration = 600;
    let impact_weapon = if product == Product::BaseQ3 && (weapon == Weapon::ProxLauncher || weapon == Weapon::Chaingun)
    {
        Weapon::None
    } else {
        weapon
    };
    match impact_weapon {
        Weapon::Nailgun | Weapon::None | Weapon::Gauntlet | Weapon::GrapplingHook => {
            if product == Product::MissionPack {
                sound = match sound_type {
                    ImpactSound::Flesh => media.sounds.nail_hit_flesh.clone(),
                    ImpactSound::Metal => media.sounds.nail_hit_metal.clone(),
                    ImpactSound::Default => media.sounds.nail_hit.clone(),
                };
                mark = media.shaders.hole_mark.clone();
                radius = 12.0;
            } else {
                let r = host.rand_i32() & 3;
                sound = registry_effects.lightning_hit_sounds[if r < 2 {
                    1
                } else if r == 2 {
                    0
                } else {
                    2
                }]
                .clone();
                mark = media.shaders.hole_mark.clone();
                radius = 12.0;
            }
        }
        Weapon::Lightning => {
            let r = host.rand_i32() & 3;
            sound = registry_effects.lightning_hit_sounds[if r < 2 {
                1
            } else if r == 2 {
                0
            } else {
                2
            }]
            .clone();
            mark = media.shaders.hole_mark.clone();
            radius = 12.0;
        }
        Weapon::ProxLauncher => {
            model = media.models.dish_flash.clone();
            shader = registry_effects.grenade_explosion_shader.clone();
            sound = media.sounds.prox_explosion.clone();
            mark = media.shaders.burn_mark.clone();
            radius = 64.0;
            light = 300.0;
            sprite = true;
        }
        Weapon::GrenadeLauncher => {
            model = media.models.dish_flash.clone();
            shader = registry_effects.grenade_explosion_shader.clone();
            sound = media.sounds.rocket_explosion.clone();
            mark = media.shaders.burn_mark.clone();
            radius = 64.0;
            light = 300.0;
            sprite = true;
        }
        Weapon::RocketLauncher => {
            model = media.models.dish_flash.clone();
            shader = registry_effects.rocket_explosion_shader.clone();
            sound = media.sounds.rocket_explosion.clone();
            mark = media.shaders.burn_mark.clone();
            radius = 64.0;
            light = 300.0;
            sprite = true;
            duration = 1000;
            light_color = vec3(1.0, 0.75, 0.0);
            if !host.weapon_settings().old_rocket {
                host.particle_explosion(&ParticleExplosion {
                    animation: "explode1".to_string(),
                    origin: weapon_ma(origin, 24.0, direction),
                    velocity: scale3(direction, 64.0),
                    duration: 1400,
                    size_start: 20.0,
                    size_end: 30.0,
                });
            }
        }
        Weapon::Railgun => {
            model = media.models.ring_flash.clone();
            shader = registry_effects.rail_explosion_shader.clone();
            sound = media.sounds.plasma_explosion.clone();
            mark = media.shaders.energy_mark.clone();
            radius = 24.0;
        }
        Weapon::Plasmagun => {
            model = media.models.ring_flash.clone();
            shader = registry_effects.plasma_explosion_shader.clone();
            sound = media.sounds.plasma_explosion.clone();
            mark = media.shaders.energy_mark.clone();
            radius = 16.0;
        }
        Weapon::Bfg => {
            model = media.models.dish_flash.clone();
            shader = registry_effects.bfg_explosion_shader.clone();
            sound = media.sounds.rocket_explosion.clone();
            mark = media.shaders.burn_mark.clone();
            radius = 32.0;
            sprite = true;
        }
        Weapon::Shotgun => {
            model = media.models.bullet_flash.clone();
            shader = registry_effects.bullet_explosion_shader.clone();
            mark = media.shaders.bullet_mark.clone();
            radius = 4.0;
        }
        Weapon::Chaingun => {
            model = media.models.bullet_flash.clone();
            mark = media.shaders.bullet_mark.clone();
            // Donor selects flesh/metal first, then overwrites with ricochet; keep the final value.
            let r = host.rand_i32() & 3;
            sound = if r < 2 {
                media.sounds.ricochet1.clone()
            } else if r == 2 {
                media.sounds.ricochet2.clone()
            } else {
                media.sounds.ricochet3.clone()
            };
            radius = 8.0;
        }
        Weapon::Machinegun => {
            model = media.models.bullet_flash.clone();
            shader = registry_effects.bullet_explosion_shader.clone();
            mark = media.shaders.bullet_mark.clone();
            let r = host.rand_i32() & 3;
            sound = if r == 0 {
                media.sounds.ricochet1.clone()
            } else if r == 1 {
                media.sounds.ricochet2.clone()
            } else {
                media.sounds.ricochet3.clone()
            };
            radius = 8.0;
        }
    }
    if sound.is_some() {
        host.start_sound(Some(origin), 1022, 0, sound);
    }
    if !model.is_default() {
        let handle = host.weapon_effects().make_explosion(
            pool,
            frame,
            &ExplosionOptions {
                origin,
                direction: Some(direction),
                model,
                shader,
                duration,
                sprite,
            },
        )?;
        if let Some(le) = pool.get_mut(handle) {
            le.light = light;
            le.light_color = light_color;
            if weapon == Weapon::Railgun {
                let color = host.client_info_view(client_num).color1;
                le.color = vec4(color.x, color.y, color.z, le.color.w);
            }
        }
    }
    let color = if weapon == Weapon::Railgun {
        host.client_info_view(client_num).color2
    } else {
        vec3(1.0, 1.0, 1.0)
    };
    let orientation = host.random_f32() * 360.0;
    let alpha_fade = mark == media.shaders.energy_mark;
    host.impact_mark(&ImpactMarkRequest {
        shader: mark,
        origin,
        direction,
        orientation,
        color: vec4(color.x, color.y, color.z, 1.0),
        alpha_fade,
        radius,
        temporary: false,
    });
    Ok(())
}

/// Emit a rail trail (`emitRailTrail`).
#[allow(clippy::too_many_arguments)]
pub fn emit_rail_trail(
    time: i32,
    registry_effects: &RegisteredWeaponEffects,
    host: &mut dyn ClientWeaponHost,
    pool: &mut LocalEntityPool,
    _frame: &EffectFrame,
    client_num: i32,
    start: &mut Vec3,
    end: Vec3,
) -> PresentResult<()> {
    let ci = host.client_info_view(client_num);
    let settings = host.weapon_settings();
    start.z -= 4.0;
    let mut position = *start;
    let delta = sub3(end, *start);
    let length = length3(delta);
    let direction = normalize3(delta);
    let temp = perpendicular_vector(direction);
    let axis: Vec<Vec3> = (0..36)
        .map(|i| rotate_point_around_vector(direction, temp, f64::from(i * 10)))
        .collect();
    let mut re = create_rail_core_entity();
    let handle = pool.allocate(LocalEntityType::FadeRgb, RefEntity::RailCore(re.clone()))?;
    re.shading.shader_time = time as f32 / 1000.0;
    re.shading.custom_shader = registry_effects.rail_core_shader.clone();
    re.origin = *start;
    re.old_origin = end;
    re.shading.shader_rgba = weapon_bytes(ci.color1, 255.0, 255.0);
    if let Some(le) = pool.get_mut(handle) {
        le.start_time = time;
        le.end_time = qvm_float_to_int(time as f32 + settings.rail_trail_time as f32);
        le.life_rate = 1.0 / (le.end_time.wrapping_sub(time) as f32);
        le.color = vec4(ci.color1.x * 0.75, ci.color1.y * 0.75, ci.color1.z * 0.75, 1.0);
        le.ref_entity = RefEntity::RailCore(re.clone());
    }
    position = weapon_ma(position, 20.0, direction);
    let step = scale3(direction, 5.0);
    if settings.old_rail {
        if let Some(le) = pool.get_mut(handle) {
            if let RefEntity::RailCore(re) = &mut le.ref_entity {
                re.origin = add3(re.origin, vec3(0.0, 0.0, -8.0));
                re.old_origin = add3(re.old_origin, vec3(0.0, 0.0, -8.0));
            }
        }
        return Ok(());
    }
    let mut skip = -1;
    let mut j = 18usize;
    let mut i = 0i32;
    while (i as f32) < length {
        if i != skip {
            skip = i + 5;
            let mut re = create_sprite_entity();
            let handle = pool.allocate(LocalEntityType::MoveScaleFade, RefEntity::Sprite(re.clone()))?;
            let side = axis[j].clone();
            re.shading.shader_time = time as f32 / 1000.0;
            re.radius = 1.1;
            re.shading.custom_shader = registry_effects.rail_rings_shader.clone();
            re.shading.shader_rgba = weapon_bytes(ci.color2, 255.0, 255.0);
            if let Some(le) = pool.get_mut(handle) {
                le.le_flags = LE_PUFF_DONT_SCALE;
                le.start_time = time;
                le.end_time = time.wrapping_add(i >> 1).wrapping_add(600);
                le.life_rate = 1.0 / (le.end_time.wrapping_sub(time) as f32);
                le.color = vec4(ci.color2.x * 0.75, ci.color2.y * 0.75, ci.color2.z * 0.75, 1.0);
                le.pos = Trajectory {
                    type_: TrajectoryType::Linear,
                    time,
                    duration: 0,
                    base: weapon_ma(position, 4.0, side),
                    delta: scale3(side, 6.0),
                };
                le.ref_entity = RefEntity::Sprite(re);
            }
        }
        position = add3(position, step);
        j = (j + 1) % 36;
        i += 5;
    }
    Ok(())
}

/// Emit a plasma trail (`emitPlasmaTrail`).
#[allow(clippy::too_many_arguments)]
pub fn emit_plasma_trail(
    time: i32,
    origin: Vec3,
    angles: Vec3,
    flash_color: Vec3,
    rail_rings_shader: Option<SceneShader>,
    host: &mut dyn ClientWeaponHost,
    pool: &mut LocalEntityPool,
) -> PresentResult<()> {
    let settings = host.weapon_settings();
    if settings.no_projectile_trail || settings.old_plasma {
        return Ok(());
    }
    let mut re = create_sprite_entity();
    let handle = pool.allocate(LocalEntityType::MoveScaleFade, RefEntity::Sprite(re.clone()))?;
    let velocity = vec3(
        60.0 - 120.0 * host.crandom_f32(),
        40.0 - 80.0 * host.crandom_f32(),
        100.0 - 200.0 * host.crandom_f32(),
    );
    let axis = angles_to_axis(angles);
    re.origin = add3(origin, weapon_transform(vec3(2.0, 2.0, 2.0), &axis));
    let water = if host.point_contents_pred(re.origin, -1) & CONTENTS_WATER != 0 {
        0.1
    } else {
        1.0
    };
    re.shading.shader_time = time as f32 / 1000.0;
    re.radius = 0.25;
    re.shading.custom_shader = rail_rings_shader;
    re.shading.shader_rgba = weapon_bytes(flash_color, 63.0, 63.0);
    let rand_bits = [host.rand_i32() & 31, host.rand_i32() & 31, host.rand_i32() & 31];
    if let Some(le) = pool.get_mut(handle) {
        le.le_flags = LE_TUMBLE;
        le.start_time = time;
        le.end_time = time.wrapping_add(600);
        le.pos = Trajectory {
            type_: TrajectoryType::Gravity,
            time,
            duration: 0,
            base: re.origin,
            delta: scale3(weapon_transform(velocity, &axis), water),
        };
        le.bounce_factor = 0.3;
        le.color = vec4(flash_color.x * 0.2, flash_color.y * 0.2, flash_color.z * 0.2, 0.25);
        le.angles = Trajectory {
            type_: TrajectoryType::Linear,
            time,
            duration: 0,
            base: vec3(rand_bits[0] as f32, rand_bits[1] as f32, rand_bits[2] as f32),
            delta: vec3(1.0, 0.5, 0.0),
        };
        le.ref_entity = RefEntity::Sprite(re);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// media.ts
// ---------------------------------------------------------------------------

/// Client media sounds (`ClientMediaSounds`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ClientMediaSounds {
    /// One minute.
    pub one_minute_sound: Option<PresentSound>,
    /// Five minutes.
    pub five_minute_sound: Option<PresentSound>,
    /// Sudden death.
    pub sudden_death_sound: Option<PresentSound>,
    /// One frag.
    pub one_frag_sound: Option<PresentSound>,
    /// Two frags.
    pub two_frag_sound: Option<PresentSound>,
    /// Three frags.
    pub three_frag_sound: Option<PresentSound>,
    /// Count three.
    pub count3_sound: Option<PresentSound>,
    /// Count two.
    pub count2_sound: Option<PresentSound>,
    /// Count one.
    pub count1_sound: Option<PresentSound>,
    /// Fight.
    pub count_fight_sound: Option<PresentSound>,
    /// Prepare.
    pub count_prepare_sound: Option<PresentSound>,
    /// Prepare team.
    pub count_prepare_team_sound: Option<PresentSound>,
    /// Capture award.
    pub capture_award_sound: Option<PresentSound>,
    /// Red leads.
    pub red_leads_sound: Option<PresentSound>,
    /// Blue leads.
    pub blue_leads_sound: Option<PresentSound>,
    /// Teams tied.
    pub teams_tied_sound: Option<PresentSound>,
    /// Hit team.
    pub hit_team_sound: Option<PresentSound>,
    /// Red scored.
    pub red_scored_sound: Option<PresentSound>,
    /// Blue scored.
    pub blue_scored_sound: Option<PresentSound>,
    /// Capture your team.
    pub capture_your_team_sound: Option<PresentSound>,
    /// Capture opponent.
    pub capture_opponent_sound: Option<PresentSound>,
    /// Return your team.
    pub return_your_team_sound: Option<PresentSound>,
    /// Return opponent.
    pub return_opponent_sound: Option<PresentSound>,
    /// Taken your team.
    pub taken_your_team_sound: Option<PresentSound>,
    /// Taken opponent.
    pub taken_opponent_sound: Option<PresentSound>,
    /// Red flag returned.
    pub red_flag_returned_sound: Option<PresentSound>,
    /// Blue flag returned.
    pub blue_flag_returned_sound: Option<PresentSound>,
    /// Enemy took your flag.
    pub enemy_took_your_flag_sound: Option<PresentSound>,
    /// Your team took enemy flag.
    pub your_team_took_enemy_flag_sound: Option<PresentSound>,
    /// Neutral flag returned.
    pub neutral_flag_returned_sound: Option<PresentSound>,
    /// Your team took the flag.
    pub your_team_took_the_flag_sound: Option<PresentSound>,
    /// Enemy took the flag.
    pub enemy_took_the_flag_sound: Option<PresentSound>,
    /// You have flag.
    pub you_have_flag_sound: Option<PresentSound>,
    /// Holy shit.
    pub holy_shit_sound: Option<PresentSound>,
    /// Base under attack.
    pub your_base_is_under_attack_sound: Option<PresentSound>,
    /// Tracer.
    pub tracer_sound: Option<PresentSound>,
    /// Select.
    pub select_sound: Option<PresentSound>,
    /// Wear off.
    pub wear_off_sound: Option<PresentSound>,
    /// Use nothing.
    pub use_nothing_sound: Option<PresentSound>,
    /// Gib.
    pub gib_sound: Option<PresentSound>,
    /// Gib bounce 1.
    pub gib_bounce1_sound: Option<PresentSound>,
    /// Gib bounce 2.
    pub gib_bounce2_sound: Option<PresentSound>,
    /// Gib bounce 3.
    pub gib_bounce3_sound: Option<PresentSound>,
    /// Use invulnerability.
    pub use_invulnerability_sound: Option<PresentSound>,
    /// Invulnerability impact 1.
    pub invulnerability_impact_sound1: Option<PresentSound>,
    /// Invulnerability impact 2.
    pub invulnerability_impact_sound2: Option<PresentSound>,
    /// Invulnerability impact 3.
    pub invulnerability_impact_sound3: Option<PresentSound>,
    /// Invulnerability juiced.
    pub invulnerability_juiced_sound: Option<PresentSound>,
    /// Obelisk hit 1.
    pub obelisk_hit_sound1: Option<PresentSound>,
    /// Obelisk hit 2.
    pub obelisk_hit_sound2: Option<PresentSound>,
    /// Obelisk hit 3.
    pub obelisk_hit_sound3: Option<PresentSound>,
    /// Obelisk respawn.
    pub obelisk_respawn_sound: Option<PresentSound>,
    /// Ammo regen.
    pub ammoregen_sound: Option<PresentSound>,
    /// Doubler.
    pub doubler_sound: Option<PresentSound>,
    /// Guard.
    pub guard_sound: Option<PresentSound>,
    /// Scout.
    pub scout_sound: Option<PresentSound>,
    /// Teleport in.
    pub tele_in_sound: Option<PresentSound>,
    /// Teleport out.
    pub tele_out_sound: Option<PresentSound>,
    /// Respawn.
    pub respawn_sound: Option<PresentSound>,
    /// No ammo.
    pub no_ammo_sound: Option<PresentSound>,
    /// Talk.
    pub talk_sound: Option<PresentSound>,
    /// Land.
    pub land_sound: Option<PresentSound>,
    /// Hit.
    pub hit_sound: Option<PresentSound>,
    /// Hit high armor.
    pub hit_sound_high_armor: Option<PresentSound>,
    /// Hit low armor.
    pub hit_sound_low_armor: Option<PresentSound>,
    /// Impressive.
    pub impressive_sound: Option<PresentSound>,
    /// Excellent.
    pub excellent_sound: Option<PresentSound>,
    /// Denied.
    pub denied_sound: Option<PresentSound>,
    /// Humiliation.
    pub humiliation_sound: Option<PresentSound>,
    /// Assist.
    pub assist_sound: Option<PresentSound>,
    /// Defend.
    pub defend_sound: Option<PresentSound>,
    /// First impressive.
    pub first_impressive_sound: Option<PresentSound>,
    /// First excellent.
    pub first_excellent_sound: Option<PresentSound>,
    /// First humiliation.
    pub first_humiliation_sound: Option<PresentSound>,
    /// Taken lead.
    pub taken_lead_sound: Option<PresentSound>,
    /// Tied lead.
    pub tied_lead_sound: Option<PresentSound>,
    /// Lost lead.
    pub lost_lead_sound: Option<PresentSound>,
    /// Vote now.
    pub vote_now: Option<PresentSound>,
    /// Vote passed.
    pub vote_passed: Option<PresentSound>,
    /// Vote failed.
    pub vote_failed: Option<PresentSound>,
    /// Water in.
    pub watr_in_sound: Option<PresentSound>,
    /// Water out.
    pub watr_out_sound: Option<PresentSound>,
    /// Water under.
    pub watr_un_sound: Option<PresentSound>,
    /// Jump pad.
    pub jump_pad_sound: Option<PresentSound>,
    /// Flight.
    pub flight_sound: Option<PresentSound>,
    /// Medkit.
    pub medkit_sound: Option<PresentSound>,
    /// Quad.
    pub quad_sound: Option<PresentSound>,
    /// Ricochet 1.
    pub sfx_ric1: Option<PresentSound>,
    /// Ricochet 2.
    pub sfx_ric2: Option<PresentSound>,
    /// Ricochet 3.
    pub sfx_ric3: Option<PresentSound>,
    /// Railgun fire.
    pub sfx_railg: Option<PresentSound>,
    /// Rocket explosion.
    pub sfx_rockexp: Option<PresentSound>,
    /// Plasma explosion.
    pub sfx_plasmaexp: Option<PresentSound>,
    /// Prox explosion.
    pub sfx_proxexp: Option<PresentSound>,
    /// Nail hit.
    pub sfx_nghit: Option<PresentSound>,
    /// Nail hit flesh.
    pub sfx_nghitflesh: Option<PresentSound>,
    /// Nail hit metal.
    pub sfx_nghitmetal: Option<PresentSound>,
    /// Chaingun hit.
    pub sfx_chghit: Option<PresentSound>,
    /// Chaingun hit flesh.
    pub sfx_chghitflesh: Option<PresentSound>,
    /// Chaingun hit metal.
    pub sfx_chghitmetal: Option<PresentSound>,
    /// Weapon hover.
    pub weapon_hover_sound: Option<PresentSound>,
    /// Kamikaze explode.
    pub kamikaze_explode_sound: Option<PresentSound>,
    /// Kamikaze implode.
    pub kamikaze_implode_sound: Option<PresentSound>,
    /// Kamikaze far.
    pub kamikaze_far_sound: Option<PresentSound>,
    /// Winner.
    pub winner_sound: Option<PresentSound>,
    /// Loser.
    pub loser_sound: Option<PresentSound>,
    /// You suck.
    pub you_suck_sound: Option<PresentSound>,
    /// Prox impl.
    pub wstbimpl_sound: Option<PresentSound>,
    /// Prox impm.
    pub wstbimpm_sound: Option<PresentSound>,
    /// Prox impd.
    pub wstbimpd_sound: Option<PresentSound>,
    /// Prox actv.
    pub wstbactv_sound: Option<PresentSound>,
    /// Regen.
    pub regen_sound: Option<PresentSound>,
    /// Protect.
    pub protect_sound: Option<PresentSound>,
    /// N health.
    pub n_health_sound: Option<PresentSound>,
    /// Grenade bounce 1.
    pub hgrenb1a_sound: Option<PresentSound>,
    /// Grenade bounce 2.
    pub hgrenb2a_sound: Option<PresentSound>,
}

/// Client media graphics (`ClientMediaGraphics`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ClientMediaGraphics {
    /// Charset shader.
    pub charset_shader: Option<SceneShader>,
    /// White shader.
    pub white_shader: Option<SceneShader>,
    /// Charset prop.
    pub charset_prop: Option<SceneShader>,
    /// Charset prop glow.
    pub charset_prop_glow: Option<SceneShader>,
    /// Charset prop B.
    pub charset_prop_b: Option<SceneShader>,
    /// Number shaders.
    pub number_shaders: [Option<SceneShader>; 11],
    /// Bot skill shaders.
    pub bot_skill_shaders: [Option<SceneShader>; 5],
    /// View blood.
    pub view_blood_shader: Option<SceneShader>,
    /// Defer.
    pub defer_shader: Option<SceneShader>,
    /// Scoreboard name.
    pub scoreboard_name: Option<SceneShader>,
    /// Scoreboard ping.
    pub scoreboard_ping: Option<SceneShader>,
    /// Scoreboard score.
    pub scoreboard_score: Option<SceneShader>,
    /// Scoreboard time.
    pub scoreboard_time: Option<SceneShader>,
    /// Smoke puff.
    pub smoke_puff_shader: Option<SceneShader>,
    /// Smoke puff Rage Pro.
    pub smoke_puff_rage_pro_shader: Option<SceneShader>,
    /// Shotgun smoke puff.
    pub shotgun_smoke_puff_shader: Option<SceneShader>,
    /// Nail puff.
    pub nail_puff_shader: Option<SceneShader>,
    /// Blue prox mine.
    pub blue_prox_mine: SceneModel,
    /// Plasma ball.
    pub plasma_ball_shader: Option<SceneShader>,
    /// Blood trail.
    pub blood_trail_shader: Option<SceneShader>,
    /// Lagometer.
    pub lagometer_shader: Option<SceneShader>,
    /// Connection.
    pub connection_shader: Option<SceneShader>,
    /// Water bubble.
    pub water_bubble_shader: Option<SceneShader>,
    /// Tracer.
    pub tracer_shader: Option<SceneShader>,
    /// Select.
    pub select_shader: Option<SceneShader>,
    /// Crosshairs.
    pub crosshair_shader: [Option<SceneShader>; 10],
    /// Back tile.
    pub back_tile_shader: Option<SceneShader>,
    /// No ammo.
    pub noammo_shader: Option<SceneShader>,
    /// Quad.
    pub quad_shader: Option<SceneShader>,
    /// Quad weapon.
    pub quad_weapon_shader: Option<SceneShader>,
    /// Battle suit.
    pub battle_suit_shader: Option<SceneShader>,
    /// Battle weapon.
    pub battle_weapon_shader: Option<SceneShader>,
    /// Invisibility.
    pub invis_shader: Option<SceneShader>,
    /// Regen.
    pub regen_shader: Option<SceneShader>,
    /// Haste puff.
    pub haste_puff_shader: Option<SceneShader>,
    /// Red cube model.
    pub red_cube_model: SceneModel,
    /// Blue cube model.
    pub blue_cube_model: SceneModel,
    /// Red cube icon.
    pub red_cube_icon: Option<SceneShader>,
    /// Blue cube icon.
    pub blue_cube_icon: Option<SceneShader>,
    /// Red flag model.
    pub red_flag_model: SceneModel,
    /// Blue flag model.
    pub blue_flag_model: SceneModel,
    /// Red flag shaders.
    pub red_flag_shader: [Option<SceneShader>; 3],
    /// Blue flag shaders.
    pub blue_flag_shader: [Option<SceneShader>; 3],
    /// Flag pole model.
    pub flag_pole_model: SceneModel,
    /// Flag flap model.
    pub flag_flap_model: SceneModel,
    /// Red flag flap skin.
    pub red_flag_flap_skin: Option<SceneSkin>,
    /// Blue flag flap skin.
    pub blue_flag_flap_skin: Option<SceneSkin>,
    /// Neutral flag flap skin.
    pub neutral_flag_flap_skin: Option<SceneSkin>,
    /// Red flag base model.
    pub red_flag_base_model: SceneModel,
    /// Blue flag base model.
    pub blue_flag_base_model: SceneModel,
    /// Neutral flag base model.
    pub neutral_flag_base_model: SceneModel,
    /// Neutral flag model.
    pub neutral_flag_model: SceneModel,
    /// Flag shaders.
    pub flag_shader: [Option<SceneShader>; 4],
    /// Overload base model.
    pub overload_base_model: SceneModel,
    /// Overload target model.
    pub overload_target_model: SceneModel,
    /// Overload lights model.
    pub overload_lights_model: SceneModel,
    /// Overload energy model.
    pub overload_energy_model: SceneModel,
    /// Harvester model.
    pub harvester_model: SceneModel,
    /// Harvester red skin.
    pub harvester_red_skin: Option<SceneSkin>,
    /// Harvester blue skin.
    pub harvester_blue_skin: Option<SceneSkin>,
    /// Harvester neutral model.
    pub harvester_neutral_model: SceneModel,
    /// Red kamikaze shader.
    pub red_kamikaze_shader: Option<SceneShader>,
    /// Dust puff shader.
    pub dust_puff_shader: Option<SceneShader>,
    /// Friend shader.
    pub friend_shader: Option<SceneShader>,
    /// Red quad shader.
    pub red_quad_shader: Option<SceneShader>,
    /// Team status bar.
    pub team_status_bar: Option<SceneShader>,
    /// Blue kamikaze shader.
    pub blue_kamikaze_shader: Option<SceneShader>,
    /// Armor model.
    pub armor_model: SceneModel,
    /// Armor icon.
    pub armor_icon: Option<SceneShader>,
    /// Machinegun brass model.
    pub machinegun_brass_model: SceneModel,
    /// Shotgun brass model.
    pub shotgun_brass_model: SceneModel,
    /// Gib abdomen.
    pub gib_abdomen: SceneModel,
    /// Gib arm.
    pub gib_arm: SceneModel,
    /// Gib chest.
    pub gib_chest: SceneModel,
    /// Gib fist.
    pub gib_fist: SceneModel,
    /// Gib foot.
    pub gib_foot: SceneModel,
    /// Gib forearm.
    pub gib_forearm: SceneModel,
    /// Gib intestine.
    pub gib_intestine: SceneModel,
    /// Gib leg.
    pub gib_leg: SceneModel,
    /// Gib skull.
    pub gib_skull: SceneModel,
    /// Gib brain.
    pub gib_brain: SceneModel,
    /// Smoke 2.
    pub smoke2: SceneModel,
    /// Balloon shader.
    pub balloon_shader: Option<SceneShader>,
    /// Blood explosion shader.
    pub blood_explosion_shader: Option<SceneShader>,
    /// Bullet flash model.
    pub bullet_flash_model: SceneModel,
    /// Ring flash model.
    pub ring_flash_model: SceneModel,
    /// Dish flash model.
    pub dish_flash_model: SceneModel,
    /// Teleport effect model.
    pub teleport_effect_model: SceneModel,
    /// Teleport effect shader.
    pub teleport_effect_shader: Option<SceneShader>,
    /// Kamikaze effect model.
    pub kamikaze_effect_model: SceneModel,
    /// Kamikaze shock wave.
    pub kamikaze_shock_wave: SceneModel,
    /// Kamikaze head model.
    pub kamikaze_head_model: SceneModel,
    /// Kamikaze head trail.
    pub kamikaze_head_trail: SceneModel,
    /// Guard powerup model.
    pub guard_powerup_model: SceneModel,
    /// Scout powerup model.
    pub scout_powerup_model: SceneModel,
    /// Doubler powerup model.
    pub doubler_powerup_model: SceneModel,
    /// Ammo regen powerup model.
    pub ammo_regen_powerup_model: SceneModel,
    /// Invulnerability impact model.
    pub invulnerability_impact_model: SceneModel,
    /// Invulnerability juiced model.
    pub invulnerability_juiced_model: SceneModel,
    /// Medkit usage model.
    pub medkit_usage_model: SceneModel,
    /// Heart shader.
    pub heart_shader: Option<SceneShader>,
    /// Invulnerability powerup model.
    pub invulnerability_powerup_model: SceneModel,
    /// Medal impressive.
    pub medal_impressive: Option<SceneShader>,
    /// Medal excellent.
    pub medal_excellent: Option<SceneShader>,
    /// Medal gauntlet.
    pub medal_gauntlet: Option<SceneShader>,
    /// Medal defend.
    pub medal_defend: Option<SceneShader>,
    /// Medal assist.
    pub medal_assist: Option<SceneShader>,
    /// Medal capture.
    pub medal_capture: Option<SceneShader>,
    /// Bullet mark.
    pub bullet_mark_shader: Option<SceneShader>,
    /// Burn mark.
    pub burn_mark_shader: Option<SceneShader>,
    /// Hole mark.
    pub hole_mark_shader: Option<SceneShader>,
    /// Energy mark.
    pub energy_mark_shader: Option<SceneShader>,
    /// Shadow mark.
    pub shadow_mark_shader: Option<SceneShader>,
    /// Wake mark.
    pub wake_mark_shader: Option<SceneShader>,
    /// Blood mark.
    pub blood_mark_shader: Option<SceneShader>,
    /// Patrol.
    pub patrol_shader: Option<SceneShader>,
    /// Assault.
    pub assault_shader: Option<SceneShader>,
    /// Camp.
    pub camp_shader: Option<SceneShader>,
    /// Follow.
    pub follow_shader: Option<SceneShader>,
    /// Defend.
    pub defend_shader: Option<SceneShader>,
    /// Team leader.
    pub team_leader_shader: Option<SceneShader>,
    /// Retrieve.
    pub retrieve_shader: Option<SceneShader>,
    /// Escort.
    pub escort_shader: Option<SceneShader>,
    /// Cursor.
    pub cursor: Option<SceneShader>,
    /// Size cursor.
    pub size_cursor: Option<SceneShader>,
    /// Select cursor.
    pub select_cursor: Option<SceneShader>,
    /// Flag shaders.
    pub flag_shaders: [Option<SceneShader>; 3],
}

/// Footstep sound bank.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FootstepBank {
    /// Normal.
    pub normal: [Option<PresentSound>; 4],
    /// Boot.
    pub boot: [Option<PresentSound>; 4],
    /// Flesh.
    pub flesh: [Option<PresentSound>; 4],
    /// Mech.
    pub mech: [Option<PresentSound>; 4],
    /// Energy.
    pub energy: [Option<PresentSound>; 4],
    /// Splash.
    pub splash: [Option<PresentSound>; 4],
    /// Metal.
    pub metal: [Option<PresentSound>; 4],
}

impl FootstepBank {
    /// Bank by kind.
    pub fn get_mut(&mut self, kind: FootstepKind) -> &mut [Option<PresentSound>; 4] {
        match kind {
            FootstepKind::Normal => &mut self.normal,
            FootstepKind::Boot => &mut self.boot,
            FootstepKind::Flesh => &mut self.flesh,
            FootstepKind::Mech => &mut self.mech,
            FootstepKind::Energy => &mut self.energy,
            FootstepKind::Splash => &mut self.splash,
            FootstepKind::Metal => &mut self.metal,
        }
    }
}

/// Client game static state (`ClientGameStaticState`, minimal mirror).
#[derive(Debug, Clone, PartialEq)]
pub struct ClientGameStaticState {
    /// Product.
    pub product: Product,
    /// Game type.
    pub game_type: GameType,
    /// Map name.
    pub mapname: String,
    /// Game models.
    pub game_models: Vec<SceneModel>,
    /// Game sounds.
    pub game_sounds: Vec<Option<PresentSound>>,
}

impl ClientGameStaticState {
    /// New static state.
    #[must_use]
    pub fn new(product: Product, game_type: GameType, mapname: impl Into<String>) -> Self {
        Self {
            product,
            game_type,
            mapname: mapname.into(),
            game_models: vec![default_model(); 256],
            game_sounds: vec![None; 256],
        }
    }
}

/// Client sound bank (`ClientSoundBank`, minimal mirror, synchronous).
pub trait ClientSoundBank {
    /// Register a sound.
    fn register_sound(&mut self, path: &str, compressed: bool) -> Option<PresentSound>;
}

/// Client media host (`ClientMediaHost`).
pub trait ClientMediaHost {
    /// Product.
    fn product(&self) -> Product;
    /// Game type.
    fn game_type(&self) -> GameType;
    /// Map name.
    fn mapname(&self) -> String;
    /// Client number.
    fn client_num(&self) -> i32;
    /// Config string.
    fn config_string(&self, index: usize) -> String;
    /// Build script.
    fn build_script(&self) -> bool;
    /// Loading string.
    fn loading_string(&mut self, text: &str);
    /// Loading item.
    fn loading_item(&mut self, index: usize);
    /// Loading client.
    fn loading_client(&mut self, index: usize);
    /// Clear the scene.
    fn clear_scene(&mut self);
    /// Reset the refdef.
    fn reset_refdef(&mut self);
    /// Load voice chats.
    fn load_voice_chats(&mut self);
    /// Build the spectator string.
    fn build_spectator_string(&mut self);
    /// New client info.
    fn new_client_info(&mut self, index: usize, info: &str);
}

/// Registered client graphics (`RegisteredClientGraphics`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredClientGraphics {
    /// World.
    pub world: PresentWorldScene,
    /// Particle animations.
    pub particle_animations: PresentParticleAnimations,
}

/// Client event media (`ClientEventMedia`, minimal mirror).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientEventMedia {
    /// Sounds.
    pub sounds: ClientMediaSounds,
    /// Footsteps.
    pub footsteps: FootstepBank,
    /// Game sounds.
    pub game_sounds: Vec<Option<PresentSound>>,
    /// Smoke puff shader.
    pub smoke_puff_shader: Option<SceneShader>,
}

/// Player media (`PlayerMedia`, minimal mirror).
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerMedia {
    /// Graphics.
    pub graphics: ClientMediaGraphics,
    /// Flight sound.
    pub flight_sound: Option<PresentSound>,
}

/// Mission player media (`MissionPlayerMedia`, minimal mirror).
#[derive(Debug, Clone, PartialEq)]
pub struct MissionPlayerMedia {
    /// Graphics.
    pub graphics: ClientMediaGraphics,
}

/// Particle media (`ParticleMedia`, minimal mirror).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParticleMedia {
    /// Tracer shader.
    pub tracer_shader: Option<SceneShader>,
    /// Smoke puff shader.
    pub smoke_puff_shader: Option<SceneShader>,
    /// Water bubble shader.
    pub water_bubble_shader: Option<SceneShader>,
}

/// Client media (`ClientMedia`).
pub struct ClientMedia {
    /// Sounds.
    pub sounds: ClientMediaSounds,
    /// Graphics.
    pub graphics: ClientMediaGraphics,
    /// Weapon registry.
    pub weapon_registry: ClientWeaponMediaRegistry,
    /// Footsteps.
    pub footsteps: FootstepBank,
    /// Inline models.
    pub inline_models: Vec<InlineModelEntry>,
    /// Product.
    pub product: Product,
    /// Static state.
    pub static_state: ClientGameStaticState,
    /// Resources.
    pub resources: Box<dyn PresentRendererResources>,
    /// Sound bank.
    pub sound_bank: Box<dyn ClientSoundBank>,
}

impl ClientMedia {
    /// New media.
    pub fn new(
        product: Product,
        static_state: ClientGameStaticState,
        resources: Box<dyn PresentRendererResources>,
        sound_bank: Box<dyn ClientSoundBank>,
        weapon_audio: Box<dyn WeaponRegistrationAudio>,
        weapon_resources: Box<dyn PresentRendererResources>,
    ) -> PresentResult<Self> {
        if product != static_state.product {
            return Err(PresentError::state("Client media product differs from cgs"));
        }
        Ok(Self {
            sounds: ClientMediaSounds::default(),
            graphics: ClientMediaGraphics::default(),
            weapon_registry: ClientWeaponMediaRegistry::new(product, weapon_resources, weapon_audio),
            footsteps: FootstepBank::default(),
            inline_models: vec![InlineModelEntry {
                model: default_model(),
                midpoint: zero_vec3(),
            }],
            product,
            static_state,
            resources,
            sound_bank,
        })
    }

    /// Event media.
    #[must_use]
    pub fn events(&self) -> ClientEventMedia {
        ClientEventMedia {
            sounds: self.sounds.clone(),
            footsteps: self.footsteps.clone(),
            game_sounds: self.static_state.game_sounds.clone(),
            smoke_puff_shader: self.graphics.smoke_puff_shader.clone(),
        }
    }

    /// Player media.
    #[must_use]
    pub fn players(&self) -> PlayerMedia {
        PlayerMedia {
            graphics: self.graphics.clone(),
            flight_sound: self.sounds.flight_sound.clone(),
        }
    }

    /// Mission player media.
    pub fn mission_players(&self) -> PresentResult<MissionPlayerMedia> {
        if self.product != Product::MissionPack {
            return Err(PresentError::state("Mission player media requested in baseq3"));
        }
        Ok(MissionPlayerMedia {
            graphics: self.graphics.clone(),
        })
    }

    /// Packet media.
    #[must_use]
    pub fn packet(&self) -> PacketEntityMedia {
        let graphics = &self.graphics;
        let sounds = &self.sounds;
        PacketEntityMedia {
            game_models: self.static_state.game_models.clone(),
            game_sounds: self.static_state.game_sounds.clone(),
            inline_models: self.inline_models.clone(),
            items: self.weapon_registry.items().to_vec(),
            weapons: self
                .weapon_registry
                .weapons()
                .iter()
                .map(|weapon| weapon.packet.clone())
                .collect(),
            plasma_ball_shader: graphics.plasma_ball_shader.clone(),
            red_flag_base_model: graphics.red_flag_base_model.clone(),
            blue_flag_base_model: graphics.blue_flag_base_model.clone(),
            neutral_flag_base_model: graphics.neutral_flag_base_model.clone(),
            variant: if self.product == Product::BaseQ3 {
                PacketEntityMediaVariant::Base
            } else {
                PacketEntityMediaVariant::Mission(PacketMissionMedia {
                    weapon_hover_sound: sounds.weapon_hover_sound.clone(),
                    blue_prox_mine: graphics.blue_prox_mine.clone(),
                    overload_base_model: graphics.overload_base_model.clone(),
                    overload_energy_model: graphics.overload_energy_model.clone(),
                    overload_lights_model: graphics.overload_lights_model.clone(),
                    overload_target_model: graphics.overload_target_model.clone(),
                    obelisk_respawn_sound: sounds.obelisk_respawn_sound.clone(),
                    harvester_model: graphics.harvester_model.clone(),
                    harvester_neutral_model: graphics.harvester_neutral_model.clone(),
                    harvester_red_skin: graphics.harvester_red_skin.clone(),
                    harvester_blue_skin: graphics.harvester_blue_skin.clone(),
                })
            },
        }
    }

    /// Effect media.
    #[must_use]
    pub fn effects(&self) -> EffectMedia {
        let graphics = &self.graphics;
        let sounds = &self.sounds;
        EffectMedia {
            water_bubble_shader: graphics.water_bubble_shader.clone(),
            smoke_puff_rage_pro_shader: graphics.smoke_puff_rage_pro_shader.clone(),
            blood_explosion_shader: graphics.blood_explosion_shader.clone(),
            teleport_effect_model: graphics.teleport_effect_model.clone(),
            gib_skull: graphics.gib_skull.clone(),
            gib_brain: graphics.gib_brain.clone(),
            gib_abdomen: graphics.gib_abdomen.clone(),
            gib_arm: graphics.gib_arm.clone(),
            gib_chest: graphics.gib_chest.clone(),
            gib_fist: graphics.gib_fist.clone(),
            gib_foot: graphics.gib_foot.clone(),
            gib_forearm: graphics.gib_forearm.clone(),
            gib_intestine: graphics.gib_intestine.clone(),
            gib_leg: graphics.gib_leg.clone(),
            smoke2: graphics.smoke2.clone(),
            variant: if self.product == Product::BaseQ3 {
                EffectMediaVariant::Base {
                    teleport_effect_shader: graphics.teleport_effect_shader.clone(),
                }
            } else {
                EffectMediaVariant::Mission(MissionEffectMedia {
                    lightning_shader: self.weapon_registry.effects.lightning_shader.clone(),
                    kamikaze_effect_model: graphics.kamikaze_effect_model.clone(),
                    dish_flash_model: graphics.dish_flash_model.clone(),
                    rocket_explosion_shader: self.weapon_registry.effects.rocket_explosion_shader.clone(),
                    obelisk_hit_sounds: [
                        sounds.obelisk_hit_sound1.clone(),
                        sounds.obelisk_hit_sound2.clone(),
                        sounds.obelisk_hit_sound3.clone(),
                    ],
                    invulnerability_impact_model: graphics.invulnerability_impact_model.clone(),
                    invulnerability_impact_sounds: [
                        sounds.invulnerability_impact_sound1.clone(),
                        sounds.invulnerability_impact_sound2.clone(),
                        sounds.invulnerability_impact_sound3.clone(),
                    ],
                    invulnerability_juiced_model: graphics.invulnerability_juiced_model.clone(),
                    invulnerability_juiced_sound: sounds.invulnerability_juiced_sound.clone(),
                })
            },
        }
    }

    /// Particle media.
    #[must_use]
    pub fn particles(&self) -> ParticleMedia {
        ParticleMedia {
            tracer_shader: self.graphics.tracer_shader.clone(),
            smoke_puff_shader: self.graphics.smoke_puff_shader.clone(),
            water_bubble_shader: self.graphics.water_bubble_shader.clone(),
        }
    }

    /// Local entity media.
    #[must_use]
    pub fn local_entities(&self) -> LocalEntityHostMedia {
        let graphics = &self.graphics;
        let sounds = &self.sounds;
        let base = LocalEntityMedia {
            blood_trail_shader: graphics.blood_trail_shader.clone(),
            blood_mark_shader: graphics.blood_mark_shader.clone(),
            burn_mark_shader: graphics.burn_mark_shader.clone(),
            number_shaders: graphics.number_shaders.to_vec(),
            gib_bounce_sounds: [
                sounds.gib_bounce1_sound.clone(),
                sounds.gib_bounce2_sound.clone(),
                sounds.gib_bounce3_sound.clone(),
            ],
        };
        if self.product == Product::BaseQ3 {
            LocalEntityHostMedia::Base(base)
        } else {
            LocalEntityHostMedia::Mission(MissionLocalEntityMedia {
                base,
                kamikaze_shock_wave: graphics.kamikaze_shock_wave.clone(),
                kamikaze_explode_sound: sounds.kamikaze_explode_sound.clone(),
                kamikaze_implode_sound: sounds.kamikaze_implode_sound.clone(),
            })
        }
    }

    /// Weapon media.
    #[must_use]
    pub fn weapons(&self) -> WeaponPresentationMedia {
        let graphics = &self.graphics;
        let sounds = &self.sounds;
        WeaponPresentationMedia {
            models: WeaponPresentationModels {
                machinegun_brass: graphics.machinegun_brass_model.clone(),
                shotgun_brass: graphics.shotgun_brass_model.clone(),
                dish_flash: graphics.dish_flash_model.clone(),
                ring_flash: graphics.ring_flash_model.clone(),
                bullet_flash: graphics.bullet_flash_model.clone(),
            },
            shaders: WeaponPresentationShaders {
                smoke_puff: graphics.smoke_puff_shader.clone(),
                nail_puff: graphics.nail_puff_shader.clone(),
                shotgun_smoke_puff: graphics.shotgun_smoke_puff_shader.clone(),
                invis: graphics.invis_shader.clone(),
                battle_weapon: graphics.battle_weapon_shader.clone(),
                quad_weapon: graphics.quad_weapon_shader.clone(),
                select: graphics.select_shader.clone(),
                noammo: graphics.noammo_shader.clone(),
                hole_mark: graphics.hole_mark_shader.clone(),
                burn_mark: graphics.burn_mark_shader.clone(),
                energy_mark: graphics.energy_mark_shader.clone(),
                bullet_mark: graphics.bullet_mark_shader.clone(),
                tracer: graphics.tracer_shader.clone(),
            },
            sounds: WeaponPresentationSounds {
                quad: sounds.quad_sound.clone(),
                nail_hit_flesh: sounds.sfx_nghitflesh.clone(),
                nail_hit_metal: sounds.sfx_nghitmetal.clone(),
                nail_hit: sounds.sfx_nghit.clone(),
                prox_explosion: sounds.sfx_proxexp.clone(),
                rocket_explosion: sounds.sfx_rockexp.clone(),
                plasma_explosion: sounds.sfx_plasmaexp.clone(),
                chaingun_hit_flesh: sounds.sfx_chghitflesh.clone(),
                chaingun_hit_metal: sounds.sfx_chghitmetal.clone(),
                chaingun_hit: sounds.sfx_chghit.clone(),
                ricochet1: sounds.sfx_ric1.clone(),
                ricochet2: sounds.sfx_ric2.clone(),
                ricochet3: sounds.sfx_ric3.clone(),
                tracer: sounds.tracer_sound.clone(),
            },
        }
    }
}

fn validate_media(media: &ClientMedia, host: &dyn ClientMediaHost) -> PresentResult<()> {
    if host.product() != media.product {
        return Err(PresentError::state(
            "Client media registration requires its canonical cgame state",
        ));
    }
    Ok(())
}

fn item_bits(host: &dyn ClientMediaHost) -> PresentResult<String> {
    let bits = host.config_string(27);
    if bits.len() > MAX_ITEMS {
        return Err(PresentError::range("CS_ITEMS exceeds source MAX_ITEMS precache buffer"));
    }
    Ok(bits)
}

/// Register item sounds (`registerItemSounds`).
pub fn register_item_sounds(media: &mut ClientMedia, number: i32, items: &dyn PresentItemTable) -> PresentResult<()> {
    let item = items
        .item_at(media.product, number as usize)
        .ok_or_else(|| PresentError::drop(format!("Bad item index {number} on entity")))?;
    if let Some(pickup) = item.pickup_sound {
        media.sound_bank.register_sound(&pickup, false);
    }
    let bytes = item.sounds.as_bytes();
    let mut offset = 0;
    while offset < bytes.len() {
        let start = offset;
        while offset < bytes.len() && bytes[offset] != b' ' {
            offset += 1;
        }
        let length = offset - start;
        if length >= 64 || length < 5 {
            return Err(PresentError::state(format!(
                "PrecacheItem: {} has bad precache string",
                item.class_name
            )));
        }
        let name = &item.sounds[start..offset];
        if offset < bytes.len() {
            offset += 1;
        }
        if name.len() >= 3 && &name[name.len() - 3..] == "wav" {
            media.sound_bank.register_sound(name, false);
        }
    }
    Ok(())
}

/// Register loading graphics (`registerClientLoadingGraphics`).
pub fn register_client_loading_graphics(media: &mut ClientMedia) {
    media.graphics.charset_shader = media.resources.register_shader("gfx/2d/bigchars");
    media.graphics.white_shader = media.resources.register_shader("white");
    media.graphics.charset_prop = media.resources.register_shader_no_mip("menu/art/font1_prop.tga");
    media.graphics.charset_prop_glow = media.resources.register_shader_no_mip("menu/art/font1_prop_glo.tga");
    media.graphics.charset_prop_b = media.resources.register_shader_no_mip("menu/art/font2_prop.tga");
}

/// Register client sounds (`registerClientSounds`).
pub fn register_client_sounds(
    media: &mut ClientMedia,
    host: &mut dyn ClientMediaHost,
    items: &dyn PresentItemTable,
) -> PresentResult<()> {
    validate_media(media, host)?;
    let mission = media.product == Product::MissionPack;
    let game_type = media.static_state.game_type;
    if mission {
        host.load_voice_chats();
    }
    let bank = &mut media.sound_bank;
    let sounds = &mut media.sounds;
    sounds.one_minute_sound = bank.register_sound("sound/feedback/1_minute.wav", true);
    sounds.five_minute_sound = bank.register_sound("sound/feedback/5_minute.wav", true);
    sounds.sudden_death_sound = bank.register_sound("sound/feedback/sudden_death.wav", true);
    sounds.one_frag_sound = bank.register_sound("sound/feedback/1_frag.wav", true);
    sounds.two_frag_sound = bank.register_sound("sound/feedback/2_frags.wav", true);
    sounds.three_frag_sound = bank.register_sound("sound/feedback/3_frags.wav", true);
    sounds.count3_sound = bank.register_sound("sound/feedback/three.wav", true);
    sounds.count2_sound = bank.register_sound("sound/feedback/two.wav", true);
    sounds.count1_sound = bank.register_sound("sound/feedback/one.wav", true);
    sounds.count_fight_sound = bank.register_sound("sound/feedback/fight.wav", true);
    sounds.count_prepare_sound = bank.register_sound("sound/feedback/prepare.wav", true);
    if mission {
        sounds.count_prepare_team_sound = bank.register_sound("sound/feedback/prepare_team.wav", true);
    }
    if game_type >= GameType::Team || host.build_script() {
        sounds.capture_award_sound = bank.register_sound("sound/teamplay/flagcapture_yourteam.wav", true);
        sounds.red_leads_sound = bank.register_sound("sound/feedback/redleads.wav", true);
        sounds.blue_leads_sound = bank.register_sound("sound/feedback/blueleads.wav", true);
        sounds.teams_tied_sound = bank.register_sound("sound/feedback/teamstied.wav", true);
        sounds.hit_team_sound = bank.register_sound("sound/feedback/hit_teammate.wav", true);
        sounds.red_scored_sound = bank.register_sound("sound/teamplay/voc_red_scores.wav", true);
        sounds.blue_scored_sound = bank.register_sound("sound/teamplay/voc_blue_scores.wav", true);
        sounds.capture_your_team_sound = bank.register_sound("sound/teamplay/flagcapture_yourteam.wav", true);
        sounds.capture_opponent_sound = bank.register_sound("sound/teamplay/flagcapture_opponent.wav", true);
        sounds.return_your_team_sound = bank.register_sound("sound/teamplay/flagreturn_yourteam.wav", true);
        sounds.return_opponent_sound = bank.register_sound("sound/teamplay/flagreturn_opponent.wav", true);
        sounds.taken_your_team_sound = bank.register_sound("sound/teamplay/flagtaken_yourteam.wav", true);
        sounds.taken_opponent_sound = bank.register_sound("sound/teamplay/flagtaken_opponent.wav", true);
        if game_type == GameType::Ctf || host.build_script() {
            sounds.red_flag_returned_sound = bank.register_sound("sound/teamplay/voc_red_returned.wav", true);
            sounds.blue_flag_returned_sound = bank.register_sound("sound/teamplay/voc_blue_returned.wav", true);
            sounds.enemy_took_your_flag_sound = bank.register_sound("sound/teamplay/voc_enemy_flag.wav", true);
            sounds.your_team_took_enemy_flag_sound = bank.register_sound("sound/teamplay/voc_team_flag.wav", true);
        }
        if mission {
            if game_type == GameType::OneFlagCtf || host.build_script() {
                sounds.neutral_flag_returned_sound =
                    bank.register_sound("sound/teamplay/flagreturn_opponent.wav", true);
                sounds.your_team_took_the_flag_sound = bank.register_sound("sound/teamplay/voc_team_1flag.wav", true);
                sounds.enemy_took_the_flag_sound = bank.register_sound("sound/teamplay/voc_enemy_1flag.wav", true);
            }
            if game_type == GameType::OneFlagCtf || game_type == GameType::Ctf || host.build_script() {
                sounds.you_have_flag_sound = bank.register_sound("sound/teamplay/voc_you_flag.wav", true);
                sounds.holy_shit_sound = bank.register_sound("sound/feedback/voc_holyshit.wav", true);
            }
            if game_type == GameType::Obelisk || host.build_script() {
                sounds.your_base_is_under_attack_sound =
                    bank.register_sound("sound/teamplay/voc_base_attack.wav", true);
            }
        } else {
            sounds.you_have_flag_sound = bank.register_sound("sound/teamplay/voc_you_flag.wav", true);
            sounds.holy_shit_sound = bank.register_sound("sound/feedback/voc_holyshit.wav", true);
            sounds.neutral_flag_returned_sound = bank.register_sound("sound/teamplay/flagreturn_opponent.wav", true);
            sounds.your_team_took_the_flag_sound = bank.register_sound("sound/teamplay/voc_team_1flag.wav", true);
            sounds.enemy_took_the_flag_sound = bank.register_sound("sound/teamplay/voc_enemy_1flag.wav", true);
        }
    }
    sounds.tracer_sound = bank.register_sound("sound/weapons/machinegun/buletby1.wav", false);
    sounds.select_sound = bank.register_sound(Q3_CHARACTER_SOUNDS.select_sound, false);
    sounds.wear_off_sound = bank.register_sound("sound/items/wearoff.wav", false);
    sounds.use_nothing_sound = bank.register_sound("sound/items/use_nothing.wav", false);
    sounds.gib_sound = bank.register_sound(Q3_CHARACTER_SOUNDS.gib_sound, false);
    sounds.gib_bounce1_sound = bank.register_sound("sound/player/gibimp1.wav", false);
    sounds.gib_bounce2_sound = bank.register_sound("sound/player/gibimp2.wav", false);
    sounds.gib_bounce3_sound = bank.register_sound("sound/player/gibimp3.wav", false);
    if mission {
        sounds.use_invulnerability_sound = bank.register_sound("sound/items/invul_activate.wav", false);
        sounds.invulnerability_impact_sound1 = bank.register_sound("sound/items/invul_impact_01.wav", false);
        sounds.invulnerability_impact_sound2 = bank.register_sound("sound/items/invul_impact_02.wav", false);
        sounds.invulnerability_impact_sound3 = bank.register_sound("sound/items/invul_impact_03.wav", false);
        sounds.invulnerability_juiced_sound = bank.register_sound("sound/items/invul_juiced.wav", false);
        sounds.obelisk_hit_sound1 = bank.register_sound("sound/items/obelisk_hit_01.wav", false);
        sounds.obelisk_hit_sound2 = bank.register_sound("sound/items/obelisk_hit_02.wav", false);
        sounds.obelisk_hit_sound3 = bank.register_sound("sound/items/obelisk_hit_03.wav", false);
        sounds.obelisk_respawn_sound = bank.register_sound("sound/items/obelisk_respawn.wav", false);
        sounds.ammoregen_sound = bank.register_sound("sound/items/cl_ammoregen.wav", false);
        sounds.doubler_sound = bank.register_sound("sound/items/cl_doubler.wav", false);
        sounds.guard_sound = bank.register_sound("sound/items/cl_guard.wav", false);
        sounds.scout_sound = bank.register_sound("sound/items/cl_scout.wav", false);
    }
    sounds.tele_in_sound = bank.register_sound(Q3_CHARACTER_SOUNDS.tele_in_sound, false);
    sounds.tele_out_sound = bank.register_sound(Q3_CHARACTER_SOUNDS.tele_out_sound, false);
    sounds.respawn_sound = bank.register_sound(Q3_CHARACTER_SOUNDS.respawn_sound, false);
    sounds.no_ammo_sound = bank.register_sound("sound/weapons/noammo.wav", false);
    sounds.talk_sound = bank.register_sound("sound/player/talk.wav", false);
    sounds.land_sound = bank.register_sound(Q3_CHARACTER_SOUNDS.land_sound, false);
    sounds.hit_sound = bank.register_sound("sound/feedback/hit.wav", false);
    if mission {
        sounds.hit_sound_high_armor = bank.register_sound("sound/feedback/hithi.wav", false);
        sounds.hit_sound_low_armor = bank.register_sound("sound/feedback/hitlo.wav", false);
    }
    sounds.impressive_sound = bank.register_sound("sound/feedback/impressive.wav", true);
    sounds.excellent_sound = bank.register_sound("sound/feedback/excellent.wav", true);
    sounds.denied_sound = bank.register_sound("sound/feedback/denied.wav", true);
    sounds.humiliation_sound = bank.register_sound("sound/feedback/humiliation.wav", true);
    sounds.assist_sound = bank.register_sound("sound/feedback/assist.wav", true);
    sounds.defend_sound = bank.register_sound("sound/feedback/defense.wav", true);
    if mission {
        sounds.first_impressive_sound = bank.register_sound("sound/feedback/first_impressive.wav", true);
        sounds.first_excellent_sound = bank.register_sound("sound/feedback/first_excellent.wav", true);
        sounds.first_humiliation_sound = bank.register_sound("sound/feedback/first_gauntlet.wav", true);
    }
    sounds.taken_lead_sound = bank.register_sound("sound/feedback/takenlead.wav", true);
    sounds.tied_lead_sound = bank.register_sound("sound/feedback/tiedlead.wav", true);
    sounds.lost_lead_sound = bank.register_sound("sound/feedback/lostlead.wav", true);
    if mission {
        sounds.vote_now = bank.register_sound("sound/feedback/vote_now.wav", true);
        sounds.vote_passed = bank.register_sound("sound/feedback/vote_passed.wav", true);
        sounds.vote_failed = bank.register_sound("sound/feedback/vote_failed.wav", true);
    }
    sounds.watr_in_sound = bank.register_sound(Q3_CHARACTER_SOUNDS.watr_in_sound, false);
    sounds.watr_out_sound = bank.register_sound(Q3_CHARACTER_SOUNDS.watr_out_sound, false);
    sounds.watr_un_sound = bank.register_sound(Q3_CHARACTER_SOUNDS.watr_un_sound, false);
    sounds.jump_pad_sound = bank.register_sound(Q3_CHARACTER_SOUNDS.jump_pad_sound, false);

    for i in 0..4 {
        for (kind, name) in Q3_FOOTSTEP_PATHS {
            let path = format!("sound/player/footsteps/{}{}.wav", name, i + 1);
            media.footsteps.get_mut(kind)[i] = media.sound_bank.register_sound(&path, false);
        }
    }

    // Source copies CS_ITEMS, but its sound filtering condition is commented out.
    item_bits(host)?;
    let item_count = items.item_count(media.product);
    for i in 1..item_count {
        register_item_sounds(media, i as i32, items)?;
    }
    for i in 1..256 {
        let name = host.config_string(288 + i);
        if name.is_empty() {
            break;
        }
        if name.starts_with('*') {
            continue;
        }
        media.static_state.game_sounds[i] = media.sound_bank.register_sound(&name, false);
    }

    let bank = &mut media.sound_bank;
    let sounds = &mut media.sounds;
    sounds.flight_sound = bank.register_sound("sound/items/flight.wav", false);
    sounds.medkit_sound = bank.register_sound("sound/items/use_medkit.wav", false);
    sounds.quad_sound = bank.register_sound("sound/items/damage3.wav", false);
    sounds.sfx_ric1 = bank.register_sound("sound/weapons/machinegun/ric1.wav", false);
    sounds.sfx_ric2 = bank.register_sound("sound/weapons/machinegun/ric2.wav", false);
    sounds.sfx_ric3 = bank.register_sound("sound/weapons/machinegun/ric3.wav", false);
    sounds.sfx_railg = bank.register_sound("sound/weapons/railgun/railgf1a.wav", false);
    sounds.sfx_rockexp = bank.register_sound("sound/weapons/rocket/rocklx1a.wav", false);
    sounds.sfx_plasmaexp = bank.register_sound("sound/weapons/plasma/plasmx1a.wav", false);
    if mission {
        sounds.sfx_proxexp = bank.register_sound("sound/weapons/proxmine/wstbexpl.wav", false);
        sounds.sfx_nghit = bank.register_sound("sound/weapons/nailgun/wnalimpd.wav", false);
        sounds.sfx_nghitflesh = bank.register_sound("sound/weapons/nailgun/wnalimpl.wav", false);
        sounds.sfx_nghitmetal = bank.register_sound("sound/weapons/nailgun/wnalimpm.wav", false);
        sounds.sfx_chghit = bank.register_sound("sound/weapons/vulcan/wvulimpd.wav", false);
        sounds.sfx_chghitflesh = bank.register_sound("sound/weapons/vulcan/wvulimpl.wav", false);
        sounds.sfx_chghitmetal = bank.register_sound("sound/weapons/vulcan/wvulimpm.wav", false);
        sounds.weapon_hover_sound = bank.register_sound("sound/weapons/weapon_hover.wav", false);
        sounds.kamikaze_explode_sound = bank.register_sound("sound/items/kam_explode.wav", false);
        sounds.kamikaze_implode_sound = bank.register_sound("sound/items/kam_implode.wav", false);
        sounds.kamikaze_far_sound = bank.register_sound("sound/items/kam_explode_far.wav", false);
        sounds.winner_sound = bank.register_sound("sound/feedback/voc_youwin.wav", false);
        sounds.loser_sound = bank.register_sound("sound/feedback/voc_youlose.wav", false);
        sounds.you_suck_sound = bank.register_sound("sound/misc/yousuck.wav", false);
        sounds.wstbimpl_sound = bank.register_sound("sound/weapons/proxmine/wstbimpl.wav", false);
        sounds.wstbimpm_sound = bank.register_sound("sound/weapons/proxmine/wstbimpm.wav", false);
        sounds.wstbimpd_sound = bank.register_sound("sound/weapons/proxmine/wstbimpd.wav", false);
        sounds.wstbactv_sound = bank.register_sound("sound/weapons/proxmine/wstbactv.wav", false);
    }
    sounds.regen_sound = bank.register_sound("sound/items/regen.wav", false);
    sounds.protect_sound = bank.register_sound("sound/items/protect3.wav", false);
    sounds.n_health_sound = bank.register_sound("sound/items/n_health.wav", false);
    sounds.hgrenb1a_sound = bank.register_sound("sound/weapons/grenade/hgrenb1a.wav", false);
    sounds.hgrenb2a_sound = bank.register_sound("sound/weapons/grenade/hgrenb2a.wav", false);
    if mission {
        for name in [
            "sound/player/james/death1.wav",
            "sound/player/james/death2.wav",
            "sound/player/james/death3.wav",
            "sound/player/james/jump1.wav",
            "sound/player/james/pain25_1.wav",
            "sound/player/james/pain75_1.wav",
            "sound/player/james/pain100_1.wav",
            "sound/player/james/falling1.wav",
            "sound/player/james/gasp.wav",
            "sound/player/james/drown.wav",
            "sound/player/james/fall1.wav",
            "sound/player/james/taunt.wav",
            "sound/player/janet/death1.wav",
            "sound/player/janet/death2.wav",
            "sound/player/janet/death3.wav",
            "sound/player/janet/jump1.wav",
            "sound/player/janet/pain25_1.wav",
            "sound/player/janet/pain75_1.wav",
            "sound/player/janet/pain100_1.wav",
            "sound/player/janet/falling1.wav",
            "sound/player/janet/gasp.wav",
            "sound/player/janet/drown.wav",
            "sound/player/janet/fall1.wav",
            "sound/player/janet/taunt.wav",
        ] {
            bank.register_sound(name, false);
        }
    }
    Ok(())
}

/// Register client graphics (`registerClientGraphics`).
pub fn register_client_graphics(
    media: &mut ClientMedia,
    host: &mut dyn ClientMediaHost,
    items: &dyn PresentItemTable,
) -> PresentResult<RegisteredClientGraphics> {
    validate_media(media, host)?;
    let mission = media.product == Product::MissionPack;
    let game_type = media.static_state.game_type;
    host.reset_refdef();
    host.clear_scene();
    let mapname = host.mapname();
    host.loading_string(&mapname);
    let world = media.resources.load_world(&mapname);
    host.loading_string("game media");

    for (i, name) in [
        "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "minus",
    ]
    .iter()
    .enumerate()
    {
        media.graphics.number_shaders[i] = media.resources.register_shader(&format!("gfx/2d/numbers/{name}_32b"));
    }
    for (i, name) in ["skill1", "skill2", "skill3", "skill4", "skill5"].iter().enumerate() {
        media.graphics.bot_skill_shaders[i] = media.resources.register_shader(&format!("menu/art/{name}.tga"));
    }
    media.graphics.view_blood_shader = media.resources.register_shader("viewBloodBlend");
    media.graphics.defer_shader = media.resources.register_shader_no_mip("gfx/2d/defer.tga");
    media.graphics.scoreboard_name = media.resources.register_shader_no_mip("menu/tab/name.tga");
    media.graphics.scoreboard_ping = media.resources.register_shader_no_mip("menu/tab/ping.tga");
    media.graphics.scoreboard_score = media.resources.register_shader_no_mip("menu/tab/score.tga");
    media.graphics.scoreboard_time = media.resources.register_shader_no_mip("menu/tab/time.tga");
    media.graphics.smoke_puff_shader = media.resources.register_shader("smokePuff");
    media.graphics.smoke_puff_rage_pro_shader = media.resources.register_shader("smokePuffRagePro");
    media.graphics.shotgun_smoke_puff_shader = media.resources.register_shader("shotgunSmokePuff");
    if mission {
        media.graphics.nail_puff_shader = media.resources.register_shader("nailtrail");
        media.graphics.blue_prox_mine = media.resources.register_model("models/weaphits/proxmineb.md3");
    }
    media.graphics.plasma_ball_shader = media.resources.register_shader("sprites/plasma1");
    media.graphics.blood_trail_shader = media.resources.register_shader("bloodTrail");
    media.graphics.lagometer_shader = media.resources.register_shader("lagometer");
    media.graphics.connection_shader = media.resources.register_shader("disconnected");
    media.graphics.water_bubble_shader = media.resources.register_shader("waterBubble");
    media.graphics.tracer_shader = media.resources.register_shader("gfx/misc/tracer");
    media.graphics.select_shader = media.resources.register_shader("gfx/2d/select");
    for i in 0..10 {
        let name = format!("gfx/2d/crosshair{}", (b'a' + i as u8) as char);
        media.graphics.crosshair_shader[i] = media.resources.register_shader(&name);
    }
    media.graphics.back_tile_shader = media.resources.register_shader("gfx/2d/backtile");
    media.graphics.noammo_shader = media.resources.register_shader("icons/noammo");
    media.graphics.quad_shader = media.resources.register_shader("powerups/quad");
    media.graphics.quad_weapon_shader = media.resources.register_shader("powerups/quadWeapon");
    media.graphics.battle_suit_shader = media.resources.register_shader("powerups/battleSuit");
    media.graphics.battle_weapon_shader = media.resources.register_shader("powerups/battleWeapon");
    media.graphics.invis_shader = media.resources.register_shader("powerups/invisibility");
    media.graphics.regen_shader = media.resources.register_shader("powerups/regen");
    media.graphics.haste_puff_shader = media.resources.register_shader("hasteSmokePuff");
    if game_type == GameType::Ctf
        || mission && (game_type == GameType::OneFlagCtf || game_type == GameType::Harvester)
        || host.build_script()
    {
        media.graphics.red_cube_model = media.resources.register_model("models/powerups/orb/r_orb.md3");
        media.graphics.blue_cube_model = media.resources.register_model("models/powerups/orb/b_orb.md3");
        media.graphics.red_cube_icon = media.resources.register_shader("icons/skull_red");
        media.graphics.blue_cube_icon = media.resources.register_shader("icons/skull_blue");
    }
    if game_type == GameType::Ctf
        || mission && (game_type == GameType::OneFlagCtf || game_type == GameType::Harvester)
        || host.build_script()
    {
        media.graphics.red_flag_model = media.resources.register_model("models/flags/r_flag.md3");
        media.graphics.blue_flag_model = media.resources.register_model("models/flags/b_flag.md3");
        media.graphics.red_flag_shader[0] = media.resources.register_shader_no_mip("icons/iconf_red1");
        media.graphics.red_flag_shader[1] = media.resources.register_shader_no_mip("icons/iconf_red2");
        media.graphics.red_flag_shader[2] = media.resources.register_shader_no_mip("icons/iconf_red3");
        media.graphics.blue_flag_shader[0] = media.resources.register_shader_no_mip("icons/iconf_blu1");
        media.graphics.blue_flag_shader[1] = media.resources.register_shader_no_mip("icons/iconf_blu2");
        media.graphics.blue_flag_shader[2] = media.resources.register_shader_no_mip("icons/iconf_blu3");
        if mission {
            media.graphics.flag_pole_model = media.resources.register_model("models/flag2/flagpole.md3");
            media.graphics.flag_flap_model = media.resources.register_model("models/flag2/flagflap3.md3");
            media.graphics.red_flag_flap_skin = media.resources.register_skin("models/flag2/red.skin");
            media.graphics.blue_flag_flap_skin = media.resources.register_skin("models/flag2/blue.skin");
            media.graphics.neutral_flag_flap_skin = media.resources.register_skin("models/flag2/white.skin");
            media.graphics.red_flag_base_model = media
                .resources
                .register_model("models/mapobjects/flagbase/red_base.md3");
            media.graphics.blue_flag_base_model = media
                .resources
                .register_model("models/mapobjects/flagbase/blue_base.md3");
            media.graphics.neutral_flag_base_model = media
                .resources
                .register_model("models/mapobjects/flagbase/ntrl_base.md3");
        }
    }
    if mission {
        if game_type == GameType::OneFlagCtf || host.build_script() {
            media.graphics.neutral_flag_model = media.resources.register_model("models/flags/n_flag.md3");
            media.graphics.flag_shader[0] = media.resources.register_shader_no_mip("icons/iconf_neutral1");
            media.graphics.flag_shader[1] = media.resources.register_shader_no_mip("icons/iconf_red2");
            media.graphics.flag_shader[2] = media.resources.register_shader_no_mip("icons/iconf_blu2");
            media.graphics.flag_shader[3] = media.resources.register_shader_no_mip("icons/iconf_neutral3");
        }
        if game_type == GameType::Obelisk || host.build_script() {
            media.graphics.overload_base_model = media.resources.register_model("models/powerups/overload_base.md3");
            media.graphics.overload_target_model =
                media.resources.register_model("models/powerups/overload_target.md3");
            media.graphics.overload_lights_model =
                media.resources.register_model("models/powerups/overload_lights.md3");
            media.graphics.overload_energy_model =
                media.resources.register_model("models/powerups/overload_energy.md3");
        }
        if game_type == GameType::Harvester || host.build_script() {
            media.graphics.harvester_model = media
                .resources
                .register_model("models/powerups/harvester/harvester.md3");
            media.graphics.harvester_red_skin = media.resources.register_skin("models/powerups/harvester/red.skin");
            media.graphics.harvester_blue_skin = media.resources.register_skin("models/powerups/harvester/blue.skin");
            media.graphics.harvester_neutral_model =
                media.resources.register_model("models/powerups/obelisk/obelisk.md3");
        }
        media.graphics.red_kamikaze_shader = media.resources.register_shader("models/weaphits/kamikred");
        media.graphics.dust_puff_shader = media.resources.register_shader("hasteSmokePuff");
    }
    if game_type >= GameType::Team || host.build_script() {
        media.graphics.friend_shader = media.resources.register_shader("sprites/foe");
        media.graphics.red_quad_shader = media.resources.register_shader("powerups/blueflag");
        media.graphics.team_status_bar = media.resources.register_shader("gfx/2d/colorbar.tga");
        if mission {
            media.graphics.blue_kamikaze_shader = media.resources.register_shader("models/weaphits/kamikblu");
        }
    }
    media.graphics.armor_model = media.resources.register_model("models/powerups/armor/armor_yel.md3");
    media.graphics.armor_icon = media.resources.register_shader_no_mip("icons/iconr_yellow");
    media.graphics.machinegun_brass_model = media.resources.register_model("models/weapons2/shells/m_shell.md3");
    media.graphics.shotgun_brass_model = media.resources.register_model("models/weapons2/shells/s_shell.md3");
    media.graphics.gib_abdomen = media.resources.register_model("models/gibs/abdomen.md3");
    media.graphics.gib_arm = media.resources.register_model("models/gibs/arm.md3");
    media.graphics.gib_chest = media.resources.register_model("models/gibs/chest.md3");
    media.graphics.gib_fist = media.resources.register_model("models/gibs/fist.md3");
    media.graphics.gib_foot = media.resources.register_model("models/gibs/foot.md3");
    media.graphics.gib_forearm = media.resources.register_model("models/gibs/forearm.md3");
    media.graphics.gib_intestine = media.resources.register_model("models/gibs/intestine.md3");
    media.graphics.gib_leg = media.resources.register_model("models/gibs/leg.md3");
    media.graphics.gib_skull = media.resources.register_model("models/gibs/skull.md3");
    media.graphics.gib_brain = media.resources.register_model("models/gibs/brain.md3");
    media.graphics.smoke2 = media.resources.register_model("models/weapons2/shells/s_shell.md3");
    media.graphics.balloon_shader = media.resources.register_shader("sprites/balloon3");
    media.graphics.blood_explosion_shader = media.resources.register_shader("bloodExplosion");
    media.graphics.bullet_flash_model = media.resources.register_model("models/weaphits/bullet.md3");
    media.graphics.ring_flash_model = media.resources.register_model("models/weaphits/ring02.md3");
    media.graphics.dish_flash_model = media.resources.register_model("models/weaphits/boom01.md3");
    if mission {
        media.graphics.teleport_effect_model = media.resources.register_model("models/powerups/pop.md3");
    } else {
        media.graphics.teleport_effect_model = media.resources.register_model("models/misc/telep.md3");
        media.graphics.teleport_effect_shader = media.resources.register_shader("teleportEffect");
    }
    if mission {
        media.graphics.kamikaze_effect_model = media.resources.register_model("models/weaphits/kamboom2.md3");
        media.graphics.kamikaze_shock_wave = media.resources.register_model("models/weaphits/kamwave.md3");
        media.graphics.kamikaze_head_model = media.resources.register_model("models/powerups/kamikazi.md3");
        media.graphics.kamikaze_head_trail = media.resources.register_model("models/powerups/trailtest.md3");
        media.graphics.guard_powerup_model = media.resources.register_model("models/powerups/guard_player.md3");
        media.graphics.scout_powerup_model = media.resources.register_model("models/powerups/scout_player.md3");
        media.graphics.doubler_powerup_model = media.resources.register_model("models/powerups/doubler_player.md3");
        media.graphics.ammo_regen_powerup_model = media.resources.register_model("models/powerups/ammo_player.md3");
        media.graphics.invulnerability_impact_model =
            media.resources.register_model("models/powerups/shield/impact.md3");
        media.graphics.invulnerability_juiced_model =
            media.resources.register_model("models/powerups/shield/juicer.md3");
        media.graphics.medkit_usage_model = media.resources.register_model("models/powerups/regen.md3");
        media.graphics.heart_shader = media
            .resources
            .register_shader_no_mip("ui/assets/statusbar/selectedhealth.tga");
    }
    media.graphics.invulnerability_powerup_model = media.resources.register_model("models/powerups/shield/shield.md3");
    media.graphics.medal_impressive = media.resources.register_shader_no_mip("medal_impressive");
    media.graphics.medal_excellent = media.resources.register_shader_no_mip("medal_excellent");
    media.graphics.medal_gauntlet = media.resources.register_shader_no_mip("medal_gauntlet");
    media.graphics.medal_defend = media.resources.register_shader_no_mip("medal_defend");
    media.graphics.medal_assist = media.resources.register_shader_no_mip("medal_assist");
    media.graphics.medal_capture = media.resources.register_shader_no_mip("medal_capture");

    let bits = item_bits(host)?;
    let bytes = bits.as_bytes();
    let item_count = items.item_count(media.product);
    for i in 1..item_count {
        if bytes.get(i) == Some(&b'1') || host.build_script() {
            host.loading_item(i);
            media.weapon_registry.register_item_visuals(i as i32, items)?;
        }
    }

    media.graphics.bullet_mark_shader = media.resources.register_shader("gfx/damage/bullet_mrk");
    media.graphics.burn_mark_shader = media.resources.register_shader("gfx/damage/burn_med_mrk");
    media.graphics.hole_mark_shader = media.resources.register_shader("gfx/damage/hole_lg_mrk");
    media.graphics.energy_mark_shader = media.resources.register_shader("gfx/damage/plasma_mrk");
    media.graphics.shadow_mark_shader = media.resources.register_shader("markShadow");
    media.graphics.wake_mark_shader = media.resources.register_shader("wake");
    media.graphics.blood_mark_shader = media.resources.register_shader("bloodMark");

    for i in 1..world.model_count {
        let model = media.resources.register_model(&format!("*{i}"));
        let bounds = model_bounds(&model);
        let midpoint = |min: f32, max: f32| min + 0.5 * (max - min);
        media.inline_models.push(InlineModelEntry {
            model,
            midpoint: vec3(
                midpoint(bounds.min.x, bounds.max.x),
                midpoint(bounds.min.y, bounds.max.y),
                midpoint(bounds.min.z, bounds.max.z),
            ),
        });
    }

    for i in 1..256 {
        let name = host.config_string(32 + i);
        if name.is_empty() {
            break;
        }
        media.static_state.game_models[i] = media.resources.register_model(&name);
    }
    if mission {
        media.graphics.patrol_shader = media.resources.register_shader_no_mip("ui/assets/statusbar/patrol.tga");
        media.graphics.assault_shader = media
            .resources
            .register_shader_no_mip("ui/assets/statusbar/assault.tga");
        media.graphics.camp_shader = media.resources.register_shader_no_mip("ui/assets/statusbar/camp.tga");
        media.graphics.follow_shader = media.resources.register_shader_no_mip("ui/assets/statusbar/follow.tga");
        media.graphics.defend_shader = media.resources.register_shader_no_mip("ui/assets/statusbar/defend.tga");
        media.graphics.team_leader_shader = media
            .resources
            .register_shader_no_mip("ui/assets/statusbar/team_leader.tga");
        media.graphics.retrieve_shader = media
            .resources
            .register_shader_no_mip("ui/assets/statusbar/retrieve.tga");
        media.graphics.escort_shader = media.resources.register_shader_no_mip("ui/assets/statusbar/escort.tga");
        media.graphics.cursor = media.resources.register_shader_no_mip("menu/art/3_cursor2");
        media.graphics.size_cursor = media.resources.register_shader_no_mip("ui/assets/sizecursor.tga");
        media.graphics.select_cursor = media.resources.register_shader_no_mip("ui/assets/selectcursor.tga");
        media.graphics.flag_shaders[0] = media
            .resources
            .register_shader_no_mip("ui/assets/statusbar/flag_in_base.tga");
        media.graphics.flag_shaders[1] = media
            .resources
            .register_shader_no_mip("ui/assets/statusbar/flag_capture.tga");
        media.graphics.flag_shaders[2] = media
            .resources
            .register_shader_no_mip("ui/assets/statusbar/flag_missing.tga");
        media.resources.register_model("models/players/james/lower.md3");
        media.resources.register_model("models/players/james/upper.md3");
        media.resources.register_model("models/players/heads/james/james.md3");
        media.resources.register_model("models/players/janet/lower.md3");
        media.resources.register_model("models/players/janet/upper.md3");
        media.resources.register_model("models/players/heads/janet/janet.md3");
    }
    let particle_animations = media.resources.load_particle_animations();
    Ok(RegisteredClientGraphics {
        world,
        particle_animations,
    })
}

/// Register clients (`registerClients`).
pub fn register_clients(media: &mut ClientMedia, host: &mut dyn ClientMediaHost) -> PresentResult<()> {
    validate_media(media, host)?;
    let client_num = host.client_num();
    host.loading_client(client_num.max(0) as usize);
    let info = host.config_string(544 + client_num.max(0) as usize);
    host.new_client_info(client_num.max(0) as usize, &info);
    for i in 0..64 {
        if i == client_num {
            continue;
        }
        let info = host.config_string(544 + i as usize);
        if info.is_empty() {
            continue;
        }
        host.loading_client(i as usize);
        host.new_client_info(i as usize, &info);
    }
    host.build_spectator_string();
    Ok(())
}

// ---------------------------------------------------------------------------
// view.ts
// ---------------------------------------------------------------------------

/// View settings (`ViewSettings`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewSettings {
    /// Video width.
    pub video_width: i32,
    /// Video height.
    pub video_height: i32,
    /// View size.
    pub view_size: i32,
    /// Third person.
    pub third_person: bool,
    /// Third-person range.
    pub third_person_range: f32,
    /// Third-person angle.
    pub third_person_angle: f32,
    /// Camera mode.
    pub camera_mode: bool,
    /// Camera orbit integer.
    pub camera_orbit_integer: i32,
    /// Camera orbit value.
    pub camera_orbit_value: f32,
    /// Camera orbit delay.
    pub camera_orbit_delay: i32,
    /// Error decay.
    pub error_decay: f32,
    /// Run pitch.
    pub run_pitch: f32,
    /// Run roll.
    pub run_roll: f32,
    /// Bob pitch.
    pub bob_pitch: f32,
    /// Bob roll.
    pub bob_roll: f32,
    /// Bob up.
    pub bob_up: f32,
    /// FOV.
    pub fov: f32,
    /// Zoom FOV.
    pub zoom_fov: f32,
    /// DM flags.
    pub dm_flags: i32,
    /// Gun X.
    pub gun_x: f32,
    /// Gun Y.
    pub gun_y: f32,
    /// Gun Z.
    pub gun_z: f32,
}

/// View host (`ViewHost`).
pub trait ViewHost {
    /// Settings.
    fn settings(&self) -> ViewSettings;
    /// Set view size.
    fn set_view_size(&mut self, value: i32);
    /// Set third-person angle value.
    fn set_third_person_angle_value(&mut self, value: f32);
    /// Register a model.
    fn register_model(&mut self, path: &str) -> SceneModel;
    /// Print.
    fn print(&mut self, message: &str);
}

fn view_multiply_add(origin: Vec3, scale: f32, direction: Vec3) -> Vec3 {
    add3(origin, scale3(direction, scale))
}

/// View runtime (`ViewRuntime`).
pub struct ViewRuntime {
    /// Host.
    pub host: Box<dyn ViewHost>,
    model_revision: u64,
}

impl ViewRuntime {
    /// New runtime.
    #[must_use]
    pub fn new(host: Box<dyn ViewHost>) -> Self {
        Self {
            host,
            model_revision: 0,
        }
    }

    /// Calculate view values (`calculateViewValues`).
    pub fn calculate_view_values(
        &mut self,
        state: &mut ClientGameState,
        prediction: &dyn PresentPrediction,
    ) -> PresentResult<bool> {
        if state.snap.is_none() {
            return Err(PresentError::state("CG_CalcViewValues requires a current snapshot"));
        }
        let settings = self.host.settings();
        let ps = state.predicted_player_state.clone();
        state.refdef = create_refdef();
        let mut size = if state
            .snap
            .as_ref()
            .is_some_and(|snap| snap.player_state.pm_type == MoveType::Intermission)
        {
            100
        } else {
            settings.view_size
        };
        if size < 30 {
            self.host.set_view_size(30);
            size = 30;
        } else if size > 100 {
            self.host.set_view_size(100);
            size = 100;
        }
        state.refdef.width = (settings.video_width.wrapping_mul(size) / 100) & !1;
        state.refdef.height = (settings.video_height.wrapping_mul(size) / 100) & !1;
        state.refdef.x = (settings.video_width - state.refdef.width) / 2;
        state.refdef.y = (settings.video_height - state.refdef.height) / 2;
        if state.refdef.width <= 0 || state.refdef.height <= 0 {
            return Err(PresentError::range("Camera view requires a positive viewport"));
        }
        state.rendering_third_person =
            settings.third_person || state.snap.as_ref().is_some_and(|snap| snap.player_state.health <= 0);
        state.refdef.view_origin = ps.origin;
        state.refdef_view_angles = ps.viewangles;
        if ps.pm_type == MoveType::Intermission {
            state.refdef.view_axis = angles_to_axis(state.refdef_view_angles);
            return self.calculate_fov(state, prediction, &settings);
        }
        state.bob_cycle = (ps.bob_cycle & 128) >> 7;
        state.bob_frac_sin = (((ps.bob_cycle & 127) as f32) / 127.0 * std::f32::consts::PI)
            .sin()
            .abs();
        state.xyspeed = (ps.velocity.x * ps.velocity.x + ps.velocity.y * ps.velocity.y).sqrt();
        let mut third_person_angle = settings.third_person_angle;
        if settings.camera_orbit_integer != 0 && state.time > state.next_orbit_time {
            state.next_orbit_time = state.time.wrapping_add(settings.camera_orbit_delay);
            third_person_angle += settings.camera_orbit_value;
            self.host.set_third_person_angle_value(third_person_angle);
        }
        if settings.error_decay > 0.0 {
            let elapsed = state.time.wrapping_sub(state.predicted_error_time);
            let factor = (settings.error_decay - elapsed as f32) / settings.error_decay;
            if factor > 0.0 && factor < 1.0 {
                state.refdef.view_origin = view_multiply_add(state.refdef.view_origin, factor, state.predicted_error);
            } else {
                state.predicted_error_time = 0;
            }
        }
        if state.rendering_third_person {
            self.offset_third_person(state, prediction, &settings, third_person_angle);
        } else {
            self.offset_first_person(state, &settings)?;
        }
        state.refdef.view_axis = angles_to_axis(state.refdef_view_angles);
        if state.hyperspace {
            state.refdef.render_flags |= RDF_NOWORLDMODEL | RDF_HYPERSPACE;
        }
        self.calculate_fov(state, prediction, &settings)
    }

    /// Finish the refdef (`finishRefdef`).
    pub fn finish_refdef(&mut self, state: &mut ClientGameState) -> PresentResult<Refdef> {
        if state.snap.is_none() {
            return Err(PresentError::state("Finishing the view requires a current snapshot"));
        }
        state.refdef.time = state.time;
        let mut mask = [0u8; 32];
        if let Some(snap) = &state.snap {
            let length = snap.area_mask.len().min(32);
            mask[..length].copy_from_slice(&snap.area_mask[..length]);
        }
        state.refdef.area_mask = mask;
        Ok(copy_refdef(&state.refdef))
    }

    /// Zoom down.
    pub fn zoom_down(&mut self, state: &mut ClientGameState) {
        if state.zoomed {
            return;
        }
        state.zoomed = true;
        state.zoom_time = state.time;
    }

    /// Zoom up.
    pub fn zoom_up(&mut self, state: &mut ClientGameState) {
        if !state.zoomed {
            return;
        }
        state.zoomed = false;
        state.zoom_time = state.time;
    }

    fn offset_third_person(
        &mut self,
        state: &mut ClientGameState,
        prediction: &dyn PresentPrediction,
        settings: &ViewSettings,
        angle: f32,
    ) {
        let ps = state.predicted_player_state.clone();
        state.refdef.view_origin = vec3(
            state.refdef.view_origin.x,
            state.refdef.view_origin.y,
            state.refdef.view_origin.z + ps.viewheight,
        );
        let mut focus_angles = state.refdef_view_angles;
        if ps.health <= 0 {
            let yaw = ps.stats.get(stat_schema(ps.product).dead_yaw) as f32;
            focus_angles = vec3(focus_angles.x, yaw, focus_angles.z);
            state.refdef_view_angles = vec3(state.refdef_view_angles.x, yaw, state.refdef_view_angles.z);
        }
        if focus_angles.x > 45.0 {
            focus_angles = vec3(45.0, focus_angles.y, focus_angles.z);
        }
        let mut focus_point = view_multiply_add(state.refdef.view_origin, 512.0, angle_vectors(focus_angles).forward);
        let mut view = vec3(
            state.refdef.view_origin.x,
            state.refdef.view_origin.y,
            state.refdef.view_origin.z + 8.0,
        );
        state.refdef_view_angles = vec3(
            state.refdef_view_angles.x * 0.5,
            state.refdef_view_angles.y,
            state.refdef_view_angles.z,
        );
        let vectors = angle_vectors(state.refdef_view_angles);
        let radians = angle / 180.0 * std::f32::consts::PI;
        view = view_multiply_add(view, -settings.third_person_range * radians.cos(), vectors.forward);
        view = view_multiply_add(view, -settings.third_person_range * radians.sin(), vectors.right);
        if !settings.camera_mode {
            let bounds = Bounds {
                min: vec3(-4.0, -4.0, -4.0),
                max: vec3(4.0, 4.0, 4.0),
            };
            let trace = prediction.trace_mover(state.refdef.view_origin, view, bounds, ps.client_num, 1);
            if trace.base.fraction != 1.0 {
                view = vec3(
                    trace.base.end.x,
                    trace.base.end.y,
                    trace.base.end.z + (1.0 - trace.base.fraction) * 32.0,
                );
                view = prediction
                    .trace_mover(state.refdef.view_origin, view, bounds, ps.client_num, 1)
                    .base
                    .end;
            }
        }
        state.refdef.view_origin = view;
        focus_point = sub3(focus_point, view);
        let distance = (focus_point.x * focus_point.x + focus_point.y * focus_point.y)
            .sqrt()
            .max(1.0);
        state.refdef_view_angles = vec3(
            -180.0 / std::f32::consts::PI * focus_point.z.atan2(distance),
            state.refdef_view_angles.y - angle,
            state.refdef_view_angles.z,
        );
    }

    fn offset_first_person(&mut self, state: &mut ClientGameState, settings: &ViewSettings) -> PresentResult<()> {
        if state.snap.is_none() {
            return Err(PresentError::state("First-person offset requires a snapshot"));
        }
        if state
            .snap
            .as_ref()
            .is_some_and(|snap| snap.player_state.pm_type == MoveType::Intermission)
        {
            return Ok(());
        }
        let ps = state.predicted_player_state.clone();
        if state.snap.as_ref().is_some_and(|snap| snap.player_state.health <= 0) {
            let yaw = state
                .snap
                .as_ref()
                .map(|snap| snap.player_state.stats.get(stat_schema(ps.product).dead_yaw) as f32)
                .unwrap_or(0.0);
            state.refdef_view_angles = vec3(-15.0, yaw, 40.0);
            state.refdef.view_origin = vec3(
                state.refdef.view_origin.x,
                state.refdef.view_origin.y,
                state.refdef.view_origin.z + ps.viewheight,
            );
            return Ok(());
        }
        let mut angles = add3(state.refdef_view_angles, state.kick_angles);
        if state.damage_time != 0.0 {
            let mut ratio = state.time as f32 - state.damage_time;
            if ratio < 100.0 {
                ratio /= 100.0;
            } else {
                ratio = 1.0 - (ratio - 100.0) / 400.0;
            }
            if state.time as f32 - state.damage_time < 100.0 || ratio > 0.0 {
                angles = vec3(
                    angles.x + ratio * state.damage_pitch,
                    angles.y,
                    angles.z + ratio * state.damage_roll,
                );
            }
        }
        angles = vec3(
            angles.x + dot3(ps.velocity, state.refdef.view_axis[0]) * settings.run_pitch,
            angles.y,
            angles.z - dot3(ps.velocity, state.refdef.view_axis[1]) * settings.run_roll,
        );
        let speed = state.xyspeed.max(200.0);
        let mut pitch = state.bob_frac_sin * settings.bob_pitch * speed;
        let mut roll = state.bob_frac_sin * settings.bob_roll * speed;
        if ps.pm_flags & MoveFlags::DUCKED != 0 {
            pitch *= 3.0;
            roll *= 3.0;
        }
        if state.bob_cycle & 1 != 0 {
            roll = -roll;
        }
        state.refdef_view_angles = vec3(angles.x + pitch, angles.y, angles.z + roll);
        let mut height = state.refdef.view_origin.z + ps.viewheight;
        let duck_delta = state.time.wrapping_sub(state.duck_time);
        if duck_delta < 100 {
            height -= state.duck_change * (100 - duck_delta) as f32 / 100.0;
        }
        height += (state.bob_frac_sin * state.xyspeed * settings.bob_up).min(6.0);
        let land_delta = state.time.wrapping_sub(state.land_time) as f32;
        if land_delta < 150.0 {
            height += state.land_change * (land_delta / 150.0);
        } else if land_delta < 450.0 {
            height += state.land_change * (1.0 - (land_delta - 150.0) / 300.0);
        }
        let step_delta = state.time.wrapping_sub(state.step_time);
        if step_delta < 200 {
            height -= state.step_change * (200 - step_delta) as f32 / 200.0;
        }
        state.refdef.view_origin = add3(
            vec3(state.refdef.view_origin.x, state.refdef.view_origin.y, height),
            state.kick_origin,
        );
        Ok(())
    }

    fn calculate_fov(
        &mut self,
        state: &mut ClientGameState,
        prediction: &dyn PresentPrediction,
        settings: &ViewSettings,
    ) -> PresentResult<bool> {
        let mut fov = 90.0f32;
        if state.predicted_player_state.pm_type != MoveType::Intermission {
            fov = if settings.dm_flags & 16 != 0 {
                90.0
            } else {
                settings.fov.clamp(1.0, 160.0)
            };
            let zoom = settings.zoom_fov.clamp(1.0, 160.0);
            let fraction = state.time.wrapping_sub(state.zoom_time) as f32 / 150.0;
            if state.zoomed {
                fov = if fraction > 1.0 {
                    zoom
                } else {
                    fov + fraction * (zoom - fov)
                };
            } else if fraction <= 1.0 {
                fov = zoom + fraction * (fov - zoom);
            }
        }
        let radians = fov / 360.0 * std::f32::consts::PI;
        let tangent = radians.sin() / radians.cos();
        let x = state.refdef.width as f32 / tangent;
        let mut vertical = (state.refdef.height as f32).atan2(x) * 360.0 / std::f32::consts::PI;
        let in_water = prediction.point_contents_pred(state.refdef.view_origin, -1) & (8 | 16 | 32) != 0;
        if in_water {
            let phase = state.time as f32 / 1000.0 * 0.4 * std::f32::consts::PI * 2.0;
            let wave = phase.sin();
            fov += wave;
            vertical -= wave;
        }
        state.refdef.fov_x = fov;
        state.refdef.fov_y = vertical;
        state.zoom_sensitivity = if state.zoomed { vertical / 75.0 } else { 1.0 };
        Ok(in_water)
    }

    /// Damage blend blob (`damageBlendBlob`).
    pub fn damage_blend_blob(
        &mut self,
        state: &ClientGameState,
        shader: Option<SceneShader>,
        rage_pro: bool,
    ) -> Option<RefSpriteEntity> {
        let elapsed = ((state.time as f32 - state.damage_time).trunc() as i32) as i32;
        if state.damage_value == 0 || rage_pro || elapsed <= 0 || elapsed >= 500 {
            return None;
        }
        let mut entity = create_sprite_entity();
        entity.shading.render_flags = RF_FIRST_PERSON;
        entity.origin = view_multiply_add(state.refdef.view_origin, 8.0, state.refdef.view_axis[0]);
        entity.origin = view_multiply_add(entity.origin, state.damage_x * -8.0, state.refdef.view_axis[1]);
        entity.origin = view_multiply_add(entity.origin, state.damage_y * 8.0, state.refdef.view_axis[2]);
        entity.radius = state.damage_value as f32 * 3.0;
        entity.shading.custom_shader = shader;
        entity.shading.shader_rgba = vec4(
            255.0,
            255.0,
            255.0,
            ((200.0 * (1.0 - elapsed as f32 / 500.0)).trunc() as i32 & 255) as f32,
        );
        Some(entity)
    }

    /// Clear the test model (`clearTestModel`).
    pub fn clear_test_model(&mut self, state: &mut ClientGameState) {
        self.model_revision += 1;
        state.test_model_name = String::new();
        state.test_model_entity = create_model_entity(default_model());
        state.test_gun = false;
    }

    /// Test a model (`testModel`).
    pub fn test_model(&mut self, state: &mut ClientGameState, name: Option<&str>, back_lerp: Option<f32>) {
        self.request_test_model(state, name, back_lerp, false);
    }

    /// Test a gun (`testGun`).
    pub fn test_gun(&mut self, state: &mut ClientGameState, name: Option<&str>, back_lerp: Option<f32>) {
        self.request_test_model(state, name, back_lerp, true);
    }

    fn request_test_model(
        &mut self,
        state: &mut ClientGameState,
        name: Option<&str>,
        back_lerp: Option<f32>,
        gun: bool,
    ) {
        self.model_revision += 1;
        let mut entity = create_model_entity(default_model());
        if let Some(name) = name {
            let nul = name.find('\0');
            let end = nul.map_or(63.min(name.len()), |index| index.min(63));
            state.test_model_name = name[..end].to_string();
            let model = self.host.register_model(&state.test_model_name.clone());
            entity.model = model.clone();
            if let Some(back_lerp) = back_lerp {
                entity.back_lerp = back_lerp;
                entity.frame = 1;
            }
            if model.is_default() {
                self.host.print("Can't register model\n");
            } else {
                entity.origin = view_multiply_add(state.refdef.view_origin, 100.0, state.refdef.view_axis[0]);
                entity.axis = angles_to_axis(vec3(0.0, 180.0 + state.refdef_view_angles.y, 0.0));
                state.test_gun = false;
            }
        }
        state.test_model_entity = entity;
        if gun {
            state.test_gun = true;
            state.test_model_entity.shading.render_flags = RF_MINLIGHT | RF_DEPTHHACK | RF_FIRST_PERSON;
        }
    }

    /// Next model frame.
    pub fn next_model_frame(&mut self, state: &mut ClientGameState) {
        state.test_model_entity.frame = state.test_model_entity.frame.wrapping_add(1);
        let frame = state.test_model_entity.frame;
        self.host.print(&format!("frame {frame}\n"));
    }

    /// Previous model frame.
    pub fn previous_model_frame(&mut self, state: &mut ClientGameState) {
        state.test_model_entity.frame = 0.max(state.test_model_entity.frame.wrapping_sub(1));
        let frame = state.test_model_entity.frame;
        self.host.print(&format!("frame {frame}\n"));
    }

    /// Next model skin.
    pub fn next_model_skin(&mut self, state: &mut ClientGameState) {
        state.test_model_entity.skin_num = state.test_model_entity.skin_num.wrapping_add(1);
        let skin = state.test_model_entity.skin_num;
        self.host.print(&format!("skin {skin}\n"));
    }

    /// Previous model skin.
    pub fn previous_model_skin(&mut self, state: &mut ClientGameState) {
        state.test_model_entity.skin_num = 0.max(state.test_model_entity.skin_num.wrapping_sub(1));
        let skin = state.test_model_entity.skin_num;
        self.host.print(&format!("skin {skin}\n"));
    }

    /// Add the test model (`addTestModel`).
    pub fn add_test_model(&mut self, state: &mut ClientGameState) -> Option<RefEntity> {
        if state.test_model_entity.model.is_default() {
            return None;
        }
        let model = self.host.register_model(&state.test_model_name.clone());
        state.test_model_entity.model = model.clone();
        if model.is_default() {
            self.host.print("Can't register model\n");
            return None;
        }
        if state.test_gun {
            let settings = self.host.settings();
            state.test_model_entity.axis = state.refdef.view_axis;
            state.test_model_entity.origin =
                view_multiply_add(state.refdef.view_origin, settings.gun_x, state.refdef.view_axis[0]);
            state.test_model_entity.origin = view_multiply_add(
                state.test_model_entity.origin,
                settings.gun_y,
                state.refdef.view_axis[1],
            );
            state.test_model_entity.origin = view_multiply_add(
                state.test_model_entity.origin,
                settings.gun_z,
                state.refdef.view_axis[2],
            );
        }
        Some(copy_ref_entity(&RefEntity::Model(state.test_model_entity.clone())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
