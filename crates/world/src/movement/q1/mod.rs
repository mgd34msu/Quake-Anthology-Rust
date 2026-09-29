//! Quake I movement providers.
//!
//! Donor provenance: `src/movement/q1/index.ts` (re-exports).

pub mod common;
pub mod monsters;
pub mod netquake;
pub mod player_actions;
pub mod pusher;
pub mod quakeworld;
pub mod result;
pub mod types;
pub mod water_transition;

pub use netquake::{create_q1_movement_provider, move_netquake};
pub use quakeworld::{create_qw_movement_provider, move_quake_world};
pub use pusher::{move_q1_pusher, step_q1_pusher};
pub use player_actions::{Q1JumpAction, Q1JumpResult, q1_check_water_jump, q1_player_jump};
pub use pusher::{Q1PushInput, Q1PusherInput, Q1PusherMovement};
pub use monsters::{Q1_FLAG_PARTIALGROUND, Q1MonsterMoveServices, Q1MonsterMoveState,
    Q1MonsterMovement, create_q1_monster_movement};
pub use types::*;
pub use water_transition::{Q1WaterTransition, q1_water_transition};
