//! Quake III movement provider.
//!
//! Donor provenance: `src/movement/q3/index.ts` (re-exports).

pub mod animation;
pub mod constants;
pub mod jump_pad;
pub mod pmove;
pub mod postures;
pub mod prediction;
pub mod provider;
pub mod slide_move;
pub mod types;
pub mod view;
pub mod weapon;

pub use animation::{run_q3_animation_operation, run_q3_torso_operation, q3_source_animation, q3_source_torso};
pub use jump_pad::{finish_q3_jump_pad_prediction, touch_q3_jump_pad};
pub use pmove::{drop_q3_movement_timers, move_player, q3_grapple_velocity, qvm_angle_vectors, snap, update_view_angles};
pub use postures::{Q3_SOURCE_POSTURES, Q3_SOURCE_STANDING_BOUNDS, q3_invulnerability_pose};
pub use prediction::{update_q3_prediction_view, Q3PredictionRuntime};
pub use provider::{create_q3_movement_provider, move_q3, q3_command};
pub use slide_move::{clip_velocity, clip_velocity_overbounce, slide_move, step_slide_move};
pub use types::*;
pub use view::q3_view_angles;
pub use weapon::{q3_weapon_delay, run_q3_weapon_step, step_q3_holdable};
