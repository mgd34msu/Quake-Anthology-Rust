//! Rerelease brain world view from `src/bots/behavior/rerelease/world.ts`.
//!
//! Observations and commands in the shipped `bots/*.txt` vocabulary
//! (item flags, weapon numbers, monster classnames). The game answers
//! queries from its own state; the brain caches nothing across frames
//! except what it explicitly remembers.

use std::collections::HashMap;

use qa_core::math::Vec3;

use crate::behavior::rerelease::nav::RereleaseNavigation;

/// Entity kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BotEntityKind;

impl BotEntityKind {
    /// Player.
    pub const PLAYER: i32 = 0;
    /// Monster.
    pub const MONSTER: i32 = 1;
    /// Item.
    pub const ITEM: i32 = 2;
    /// Interactable.
    pub const INTERACTABLE: i32 = 3;
}

/// One observed entity.
#[derive(Debug, Clone, PartialEq)]
pub struct BotEntityT {
    /// Stable id while the entity lives.
    pub id: i32,
    /// Entity kind.
    pub kind: i32,
    /// Game classname (the `bots/*.txt` key).
    pub classname: String,
    /// Origin.
    pub origin: Vec3,
    /// Aim center.
    pub center: Vec3,
    /// Aim head.
    pub head: Vec3,
    /// Aim feet.
    pub feet: Vec3,
    /// Velocity.
    pub velocity: Vec3,
    /// Health.
    pub health: f32,
    /// Team (0 = none).
    pub team: i32,
    /// Carrying an objective.
    pub carrying_objective: bool,
    /// Dead.
    pub dead: bool,
    /// Invisible.
    pub invisible: bool,
    /// Water level 0-3.
    pub water_level: i32,
    /// Bot-controlled player.
    pub is_bot: bool,
    /// Spawnflags.
    pub spawnflags: i32,
    /// Has shootable health.
    pub has_health: bool,
    /// Has a targetname.
    pub has_targetname: bool,
}

/// One audible noise.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BotSoundT {
    /// Origin.
    pub origin: Vec3,
    /// Source entity id, or -1.
    pub source_id: i32,
    /// Server time made.
    pub time: f32,
    /// Loudness scalar.
    pub loudness: f32,
}

/// The bot's own state.
#[derive(Debug, Clone, PartialEq)]
pub struct BotSelfT {
    /// Entity id.
    pub id: i32,
    /// Origin.
    pub origin: Vec3,
    /// Velocity.
    pub velocity: Vec3,
    /// View angles (pitch, yaw, roll).
    pub view_angles: Vec3,
    /// Eye position.
    pub eye: Vec3,
    /// Health.
    pub health: f32,
    /// Armor.
    pub armor: f32,
    /// QuakeC `items` bitmask.
    pub items: i32,
    /// Ammo by `ammo_name`.
    pub ammo: HashMap<String, i32>,
    /// QuakeC `weapon` bit in hand.
    pub current_weapon: i32,
    /// On ground.
    pub on_ground: bool,
    /// Water level 0-3.
    pub water_level: i32,
    /// Breath seconds left, when known.
    pub air_seconds: Option<f32>,
    /// Standing on a lift/train, when known.
    pub on_lift: Option<bool>,
    /// Team.
    pub team: i32,
    /// Dead.
    pub dead: bool,
    /// Has protection (pentagram).
    pub has_protection: bool,
    /// Armor maximum, when known.
    pub max_armor: Option<f32>,
    /// Carrying an objective.
    pub carrying_objective: bool,
}

/// Trace result.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BotTraceT {
    /// Fraction reached (1 = clear).
    pub fraction: f32,
    /// End position.
    pub endpos: Vec3,
    /// Started inside solid.
    pub startsolid: bool,
    /// Hit entity id.
    pub hit_id: i32,
}

/// Point contents answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BotContents;

impl BotContents {
    /// Empty.
    pub const EMPTY: i32 = 0;
    /// Solid.
    pub const SOLID: i32 = 1;
    /// Water.
    pub const WATER: i32 = 2;
    /// Slime.
    pub const SLIME: i32 = 3;
    /// Lava.
    pub const LAVA: i32 = 4;
    /// Sky.
    pub const SKY: i32 = 5;
}

/// Brain output command.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct BotUsercmdT {
    /// Forward move.
    pub forwardmove: f32,
    /// Side move.
    pub sidemove: f32,
    /// Up move.
    pub upmove: f32,
    /// Buttons (attack/jump/use).
    pub buttons: i32,
    /// Impulse (weapon selection).
    pub impulse: i32,
    /// View angles.
    pub view_angles: Vec3,
}

/// Attack button.
pub const BOT_BUTTON_ATTACK: i32 = 1;
/// Jump button.
pub const BOT_BUTTON_JUMP: i32 = 2;
/// Use button.
pub const BOT_BUTTON_USE: i32 = 4;

/// Empty user command.
#[must_use]
pub fn empty_usercmd() -> BotUsercmdT {
    BotUsercmdT::default()
}

/// The world as the brain may see it.
pub trait BotWorldT {
    /// Server time seconds.
    fn time(&self) -> f32;
    /// Frame length seconds.
    fn frame_time(&self) -> f32;
    /// Own state.
    fn bot_self(&self) -> BotSelfT;
    /// Point trace.
    fn trace_line(&self, start: Vec3, end: Vec3) -> BotTraceT;
    /// Box trace.
    fn trace_box(&self, start: Vec3, mins: Vec3, maxs: Vec3, end: Vec3) -> BotTraceT;
    /// Point contents.
    fn point_contents(&self, point: Vec3) -> i32;
    /// Entities of interest.
    fn entities(&self) -> Vec<BotEntityT>;
    /// Noises since the last call, oldest first.
    fn hearing(&self) -> Vec<BotSoundT>;
    /// Navigation graph, or `None`.
    fn nav(&mut self) -> Option<&mut dyn RereleaseNavigation>;
}
