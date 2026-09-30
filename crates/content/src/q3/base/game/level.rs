//! Quake III base/game: level.
//!
//! Donor provenance: `src/content/q3/base/game/level.ts`.

use qa_core::math::{vec3, Vec3};

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::game::mirrors_game_items::*;

// ---------------------------------------------------------------------------
// level.ts: GameLevel
// ---------------------------------------------------------------------------

/// Vote state (`VoteState`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VoteState {
    /// Vote time.
    pub time: i32,
    /// Yes votes.
    pub yes: i32,
    /// No votes.
    pub no: i32,
    /// Vote string.
    pub string: String,
    /// Display string.
    pub display_string: String,
    /// Execute time.
    pub execute_time: i32,
}

/// Team vote state (`TeamVoteState`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TeamVoteState {
    /// Vote time.
    pub time: i32,
    /// Yes votes.
    pub yes: i32,
    /// No votes.
    pub no: i32,
    /// Vote string.
    pub string: String,
}

/// Match state (`MatchState`).
#[derive(Debug, Clone, PartialEq)]
pub struct MatchState {
    /// Time.
    pub time: i32,
    /// Start time.
    pub start_time: i32,
    /// Warmup time.
    pub warmup_time: i32,
    /// Warmup modification count.
    pub warmup_modification_count: i32,
    /// Restarted.
    pub restarted: bool,
    /// Connected clients.
    pub num_connected_clients: i32,
    /// Non-spectator clients.
    pub num_non_spectator_clients: i32,
    /// Playing clients.
    pub num_playing_clients: i32,
    /// Voting clients.
    pub num_voting_clients: i32,
    /// Team voting clients.
    pub num_team_voting_clients: [i32; 2],
    /// Sorted clients.
    pub sorted_clients: Vec<i32>,
    /// Follow 1.
    pub follow1: i32,
    /// Follow 2.
    pub follow2: i32,
    /// Intermission time.
    pub intermission_time: i32,
    /// Intermission queued.
    pub intermission_queued: i32,
    /// Intermission origin.
    pub intermission_origin: Vec3,
    /// Intermission angle.
    pub intermission_angle: Vec3,
    /// Change map.
    pub changemap: Option<String>,
    /// Ready to exit.
    pub ready_to_exit: bool,
    /// Exit time.
    pub exit_time: i32,
    /// Vote.
    pub vote: VoteState,
    /// Team votes.
    pub team_votes: [TeamVoteState; 2],
}

impl Default for MatchState {
    fn default() -> Self {
        MatchState {
            time: 0,
            start_time: 0,
            warmup_time: 0,
            warmup_modification_count: 0,
            restarted: false,
            num_connected_clients: 0,
            num_non_spectator_clients: 0,
            num_playing_clients: 0,
            num_voting_clients: 0,
            num_team_voting_clients: [0, 0],
            sorted_clients: vec![0; MAX_CLIENTS],
            follow1: 0,
            follow2: 0,
            intermission_time: 0,
            intermission_queued: 0,
            intermission_origin: vec3(0.0, 0.0, 0.0),
            intermission_angle: vec3(0.0, 0.0, 0.0),
            changemap: None,
            ready_to_exit: false,
            exit_time: 0,
            vote: VoteState::default(),
            team_votes: [TeamVoteState::default(), TeamVoteState::default()],
        }
    }
}

/// Game level (`GameLevel`).
#[derive(Debug, Clone, PartialEq)]
pub struct GameLevel {
    /// Match state base.
    pub base: MatchState,
    /// Frame number.
    pub frame_num: i32,
    /// Previous time.
    pub previous_time: i32,
    /// New session.
    pub new_session: bool,
    /// Fry sound.
    pub fry_sound: i32,
    /// Team scores.
    pub team_scores: PlayerStateSlots,
}

impl GameLevel {
    /// Fresh level.
    #[must_use]
    pub fn new() -> GameLevel {
        GameLevel {
            base: MatchState::default(),
            frame_num: 0,
            previous_time: 0,
            new_session: false,
            fry_sound: 0,
            team_scores: PlayerStateSlots::new(4),
        }
    }

    /// Reset the level (`clear`); every field returns to fresh zero state.
    pub fn clear(&mut self) {
        *self = GameLevel::new();
    }
}

impl Default for GameLevel {
    fn default() -> Self {
        GameLevel::new()
    }
}
