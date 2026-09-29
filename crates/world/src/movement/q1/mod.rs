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

pub use monsters::{
    create_q1_monster_movement, Q1MonsterMoveServices, Q1MonsterMoveState, Q1MonsterMovement, Q1_FLAG_PARTIALGROUND,
};
pub use netquake::{create_q1_movement_provider, move_netquake};
pub use player_actions::{q1_check_water_jump, q1_player_jump, Q1JumpAction, Q1JumpResult};
pub use pusher::{move_q1_pusher, step_q1_pusher};
pub use pusher::{Q1PushInput, Q1PusherInput, Q1PusherMovement};
pub use quakeworld::{create_qw_movement_provider, move_quake_world};
pub use types::*;
pub use water_transition::{q1_water_transition, Q1WaterTransition};
