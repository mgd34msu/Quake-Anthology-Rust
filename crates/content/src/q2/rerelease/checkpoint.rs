//! Q2 rerelease checkpoints (`src/content/q2/rerelease/checkpoint.ts`).
//!
//! Gameplay logic adapted from id Software's Quake II game and the
//! rerelease game DLL (GPL-2.0-or-later).
//!
//! Checkpoints are data-only structs; capture and restore live with the
//! owning modules.

use qa_core::identity::SavedActorId;
use qa_core::math::Vec3;

use super::campaign::{Q2RereleaseLevelEntry, Q2RereleaseMission};
use super::goals::Q2RereleaseGoalsCheckpoint;
use super::types::{Q2FogState, Q2RereleaseOptions, Q2RereleasePlayerState};

/// Rerelease player checkpoint entry.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2RereleasePlayerCheckpointEntry {
    /// Actor.
    pub actor: SavedActorId,
    /// State.
    pub state: Q2RereleasePlayerState,
}

/// Rerelease squad spawn checkpoint entry.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2RereleaseSquadSpawn {
    /// Actor.
    pub actor: SavedActorId,
    /// Origin.
    pub origin: Vec3,
    /// Angles.
    pub angles: Vec3,
}

/// Rerelease intermission camera checkpoint.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2RereleaseIntermissionCamera {
    /// Origin.
    pub origin: Vec3,
    /// Angles.
    pub angles: Vec3,
}

/// Rerelease players checkpoint (`Q2RereleasePlayersCheckpoint`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2RereleasePlayersCheckpoint {
    /// Version.
    pub version: u32,
    /// Options.
    pub options: Q2RereleaseOptions,
    /// Coop restart time.
    pub coop_restart_time: f64,
    /// Whether the killbox is deadly.
    pub deadly_kill_box: bool,
    /// Intermission flags.
    pub intermission_flags: i32,
    /// Intermission fade expiry.
    pub intermission_fade_until: Option<f64>,
    /// Intermission camera.
    pub intermission_camera: Option<Q2RereleaseIntermissionCamera>,
    /// Whether the intermission camera is set.
    pub intermission_camera_set: bool,
    /// Player states.
    pub players: Vec<Q2RereleasePlayerCheckpointEntry>,
    /// Squad spawns.
    pub squad_spawns: Vec<Q2RereleaseSquadSpawn>,
}

/// Rerelease sky checkpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2RereleaseSkyCheckpoint {
    /// Name.
    pub name: String,
    /// Rotation.
    pub rotation: f64,
    /// Whether auto rotating.
    pub auto_rotate: bool,
    /// Axis.
    pub axis: Vec3,
}

/// Rerelease point-of-interest checkpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2RereleasePoiCheckpoint {
    /// Actor.
    pub actor: SavedActorId,
    /// Origin.
    pub origin: Vec3,
    /// Image.
    pub image: String,
    /// Dynamic actor.
    pub dynamic: Option<SavedActorId>,
}

/// Rerelease campaign checkpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2RereleaseCampaignCheckpoint {
    /// Cross-unit flags.
    pub cross_unit_flags: i32,
    /// Visited maps.
    pub visited_maps: Vec<String>,
    /// Level entries.
    pub levels: Vec<Q2RereleaseLevelEntry>,
    /// Mission objectives.
    pub mission: Q2RereleaseMission,
}

/// Rerelease health bar checkpoint.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2RereleaseHealthBarCheckpoint {
    /// Controller.
    pub controller: SavedActorId,
    /// Target.
    pub target: SavedActorId,
    /// Dead expiry.
    pub dead_until: Option<f64>,
}

/// Rerelease health target checkpoint.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2RereleaseHealthTarget {
    /// Controller.
    pub controller: SavedActorId,
    /// Target.
    pub target: SavedActorId,
}

/// Rerelease pickup record checkpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2RereleasePickupRecord {
    /// Actor.
    pub actor: SavedActorId,
    /// Slots.
    pub slots: Vec<i32>,
}

/// Rerelease trigger sound time checkpoint.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2RereleaseTriggerSoundTime {
    /// Actor.
    pub actor: SavedActorId,
    /// Time.
    pub time: f64,
}

/// Rerelease light checkpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q2RereleaseLightCheckpoint {
    /// Actor.
    pub actor: SavedActorId,
    /// Whether active.
    pub active: bool,
}

/// Rerelease Q64 eye checkpoint.
///
/// Owned semantically by the Q64 module (`q64/index.ts`); defined here so
/// the module checkpoint stays data-only until that batch lands.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2RereleaseQ64EyeState {
    /// Neutral angles.
    pub neutral_angles: Vec3,
    /// Eye position.
    pub eye_position: Vec3,
    /// Vision cone.
    pub vision_cone: f64,
}

/// Rerelease Q64 camera checkpoint.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2RereleaseQ64CameraState {
    /// Remaining time.
    pub remaining: f64,
    /// Distance.
    pub distance: f64,
    /// Speed.
    pub speed: f64,
    /// Angles.
    pub angles: Vec3,
}

/// Rerelease Q64 dummy checkpoint.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2RereleaseQ64DummyState {
    /// Fade remaining.
    pub fade_remaining: f64,
    /// Fade duration.
    pub fade_duration: f64,
    /// Whether fading.
    pub fading: bool,
}

/// Rerelease Q64 eye checkpoint entry.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2RereleaseQ64EyeCheckpoint {
    /// Actor.
    pub actor: SavedActorId,
    /// State.
    pub state: Q2RereleaseQ64EyeState,
}

/// Rerelease Q64 camera checkpoint entry.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2RereleaseQ64CameraCheckpoint {
    /// Actor.
    pub actor: SavedActorId,
    /// State.
    pub state: Q2RereleaseQ64CameraState,
}

/// Rerelease Q64 dummy checkpoint entry.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2RereleaseQ64DummyCheckpoint {
    /// Actor.
    pub actor: SavedActorId,
    /// State.
    pub state: Q2RereleaseQ64DummyState,
}

/// Rerelease Q64 checkpoint (`Q2RereleaseQ64Checkpoint`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2RereleaseQ64Checkpoint {
    /// Eyes.
    pub eyes: Vec<Q2RereleaseQ64EyeCheckpoint>,
    /// Cameras.
    pub cameras: Vec<Q2RereleaseQ64CameraCheckpoint>,
    /// Dummies.
    pub dummies: Vec<Q2RereleaseQ64DummyCheckpoint>,
}

/// Rerelease module checkpoint (`Q2RereleaseModuleCheckpoint`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2RereleaseModuleCheckpoint {
    /// Version.
    pub version: u32,
    /// World fog.
    pub world_fog: Q2FogState,
    /// Story.
    pub story: String,
    /// Sky.
    pub sky: Q2RereleaseSkyCheckpoint,
    /// Point of interest.
    pub poi: Option<Q2RereleasePoiCheckpoint>,
    /// POI stage.
    pub poi_stage: i32,
    /// Last autosave time.
    pub last_auto_save: f64,
    /// Campaign.
    pub campaign: Q2RereleaseCampaignCheckpoint,
    /// Health bars.
    pub health_bars: Vec<Option<Q2RereleaseHealthBarCheckpoint>>,
    /// Health targets.
    pub health_targets: Vec<Q2RereleaseHealthTarget>,
    /// Pickups by actor.
    pub picked_up_by: Vec<Q2RereleasePickupRecord>,
    /// Trigger sound times.
    pub trigger_sound_times: Vec<Q2RereleaseTriggerSoundTime>,
    /// Q64 state.
    pub q64: Q2RereleaseQ64Checkpoint,
    /// Lights.
    pub lights: Vec<Q2RereleaseLightCheckpoint>,
    /// Goals.
    pub goals: Q2RereleaseGoalsCheckpoint,
}
