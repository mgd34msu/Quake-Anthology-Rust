//! Q2 player checkpoints (`src/content/q2/base/player/checkpoint.ts`).

use qa_core::identity::SavedActorId;
use qa_core::math::Vec3;

use crate::q2::foundation::checkpoint::Q2AttackCheckpoint;

use super::types::{Q2PlayerRules, Q2PlayerState};

/// Player state checkpoint (`Q2PlayerStateCheckpoint`).
///
/// The embedded state's chase target is always `None`; the saved chase
/// target travels alongside.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2PlayerStateCheckpoint {
    /// State with a cleared chase target.
    pub state: Q2PlayerState,
    /// Saved chase target.
    pub chase_target: Option<SavedActorId>,
}

/// Landmark carry checkpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2LandmarkCarryCheckpoint {
    /// Landmark name.
    pub name: String,
    /// Relative origin.
    pub relative_origin: Vec3,
    /// Relative velocity.
    pub relative_velocity: Vec3,
    /// Relative view angles.
    pub relative_view_angles: Vec3,
    /// Saved player.
    pub player: SavedActorId,
}

/// Player intermission checkpoint (`Q2PlayerIntermissionCheckpoint`).
#[derive(Debug, Clone, PartialEq)]
pub enum Q2PlayerIntermissionCheckpoint {
    /// Playing.
    Playing,
    /// Intermission.
    Intermission {
        /// Map.
        map: String,
        /// Started time.
        started: f64,
        /// Whether exiting.
        exit: bool,
        /// Landmark.
        landmark: Option<Q2LandmarkCarryCheckpoint>,
    },
}

/// Player checkpoint entry.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2PlayerCheckpointEntry {
    /// Actor.
    pub actor: SavedActorId,
    /// State.
    pub state: Q2PlayerStateCheckpoint,
}

/// Players checkpoint (`Q2PlayersCheckpoint`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2PlayersCheckpoint {
    /// Version.
    pub version: u32,
    /// Corpse index.
    pub corpse_index: i32,
    /// Death animation.
    pub death_animation: i32,
    /// Pain animation.
    pub pain_animation: i32,
    /// Rules.
    pub rules: Q2PlayerRules,
    /// Intermission.
    pub intermission: Q2PlayerIntermissionCheckpoint,
    /// Players.
    pub players: Vec<Q2PlayerCheckpointEntry>,
}

/// Character entity checkpoint fields.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2CharacterEntityFields {
    /// Model.
    pub model: String,
    /// Model 2.
    pub model2: String,
    /// Model 3.
    pub model3: String,
    /// Model 4.
    pub model4: String,
    /// Skin.
    pub skin: i32,
    /// Frame.
    pub frame: i32,
    /// Old frame.
    pub old_frame: i32,
    /// Scale.
    pub scale: f64,
    /// Effects.
    pub effects: i64,
    /// Render flags.
    pub render_flags: i32,
    /// Flags.
    pub flags: i64,
    /// Server flags.
    pub server_flags: i32,
    /// View height.
    pub view_height: i32,
    /// Maximum health.
    pub max_health: f64,
    /// Sound.
    pub sound: String,
    /// Whether visible.
    pub visible: bool,
}

/// Character checkpoint (`Q2CharacterCheckpoint`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2CharacterCheckpoint {
    /// Version.
    pub version: u32,
    /// Pain index.
    pub pain_index: i32,
    /// Death index.
    pub death_index: i32,
    /// State.
    pub state: Q2PlayerStateCheckpoint,
    /// Rules.
    pub rules: Q2PlayerRules,
    /// Entity fields.
    pub entity: Q2CharacterEntityFields,
    /// Last attack.
    pub last_attack: Option<Q2AttackCheckpoint>,
}
