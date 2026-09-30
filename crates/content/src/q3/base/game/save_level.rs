//! Quake III base/game: save level.
//!
//! Donor provenance: `src/content/q3/base/game/save-level.ts`.

use crate::value::arr;
use crate::value::obj;
use crate::value::str;
use crate::value::SaveJson;
use crate::value::SaveReader;
use qa_core::math::vec3;
use qa_core::math::Vec3;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::game::save_reader::*;
use crate::q3::base::game::save_state::*;
use crate::q3::base::game::save_values::*;
use crate::q3::base::game::state::*;
use crate::q3::base::game::state::{Q3GameError, Q3PlayerSlots};

// ---------------------------------------------------------------------------
// save-level.ts: level capture and restore
// ---------------------------------------------------------------------------

/// Vote state (`VoteState`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Q3VoteState {
    /// Time.
    pub time: i32,
    /// Yes votes.
    pub yes: i32,
    /// No votes.
    pub no: i32,
    /// String.
    pub string: String,
    /// Display string.
    pub display_string: String,
    /// Execute time.
    pub execute_time: i32,
}

/// Team vote state (`TeamVoteState`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Q3TeamVoteState {
    /// Time.
    pub time: i32,
    /// Yes votes.
    pub yes: i32,
    /// No votes.
    pub no: i32,
    /// String.
    pub string: String,
}

/// Game level (`GameLevel` / `MatchState` fields used by saves).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3GameLevel {
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
    /// Frame number.
    pub frame_num: i32,
    /// Previous time.
    pub previous_time: i32,
    /// New session.
    pub new_session: bool,
    /// Fry sound.
    pub fry_sound: i32,
    /// Team scores.
    pub team_scores: Q3PlayerSlots,
    /// Vote.
    pub vote: Q3VoteState,
    /// Team votes.
    pub team_votes: [Q3TeamVoteState; 2],
}

impl Default for Q3GameLevel {
    fn default() -> Self {
        Self {
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
            frame_num: 0,
            previous_time: 0,
            new_session: false,
            fry_sound: 0,
            team_scores: Q3PlayerSlots::new(4),
            vote: Q3VoteState::default(),
            team_votes: [Q3TeamVoteState::default(), Q3TeamVoteState::default()],
        }
    }
}

/// Capture the level (`captureQ3Level`).
#[must_use]
pub fn capture_q3_level(level: &Q3GameLevel) -> SaveJson {
    obj(vec![
        ("values", level_values_to_json(&capture_level_values(level))),
        ("teamScores", numbers_to_json(&level.team_scores.copy_vec())),
        ("numTeamVotingClients", numbers_to_json(&level.num_team_voting_clients)),
        ("sortedClients", numbers_to_json(&level.sorted_clients)),
        (
            "vote",
            obj(vec![
                ("time", num_i32(level.vote.time)),
                ("yes", num_i32(level.vote.yes)),
                ("no", num_i32(level.vote.no)),
                ("string", str(&level.vote.string)),
                ("displayString", str(&level.vote.display_string)),
                ("executeTime", num_i32(level.vote.execute_time)),
            ]),
        ),
        (
            "teamVotes",
            arr(level
                .team_votes
                .iter()
                .map(|vote| {
                    obj(vec![
                        ("time", num_i32(vote.time)),
                        ("yes", num_i32(vote.yes)),
                        ("no", num_i32(vote.no)),
                        ("string", str(&vote.string)),
                    ])
                })
                .collect()),
        ),
    ])
}

/// Restore the level (`restoreQ3Level`).
pub fn restore_q3_level(level: &mut Q3GameLevel, value: &SaveJson) -> Result<(), Q3GameError> {
    let reader = SaveReader::at(value, "q3.level");
    let values = read_level_values(&reader.field("values"))?;
    restore_level_values(level, &values);
    let scores = read_numbers(&reader.field("teamScores"))?;
    let voting = read_numbers(&reader.field("numTeamVotingClients"))?;
    let sorted = read_numbers(&reader.field("sortedClients"))?;
    let teams: Vec<Q3TeamVoteState> =
        reader
            .field("teamVotes")
            .list(|value| -> Result<Q3TeamVoteState, Q3GameError> {
                Ok(Q3TeamVoteState {
                    time: read_i32(&value, "time")?,
                    yes: read_i32(&value, "yes")?,
                    no: read_i32(&value, "no")?,
                    string: value.field("string").string()?,
                })
            })?;
    if scores.len() != level.team_scores.len()
        || voting.len() != 2
        || sorted.len() != level.sorted_clients.len()
        || teams.len() != 2
    {
        return Err(reader.fail("Q3 level table length mismatch").into());
    }
    for (index, score) in scores.iter().enumerate() {
        level.team_scores.set(index, *score);
    }
    let (Some(first), Some(second), Some(red), Some(blue)) =
        (voting.first(), voting.get(1), teams.first(), teams.get(1))
    else {
        return Err(reader.fail("Q3 level team table missing").into());
    };
    level.num_team_voting_clients[0] = *first;
    level.num_team_voting_clients[1] = *second;
    level.sorted_clients.copy_from_slice(&sorted);
    let vote = reader.field("vote");
    level.vote.time = read_i32(&vote, "time")?;
    level.vote.yes = read_i32(&vote, "yes")?;
    level.vote.no = read_i32(&vote, "no")?;
    level.vote.string = vote.field("string").string()?;
    level.vote.display_string = vote.field("displayString").string()?;
    level.vote.execute_time = read_i32(&vote, "executeTime")?;
    level.team_votes[0] = red.clone();
    level.team_votes[1] = blue.clone();
    Ok(())
}
