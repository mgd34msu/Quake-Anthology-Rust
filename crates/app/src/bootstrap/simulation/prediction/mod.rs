//! Movement prediction (donor `src/app/bootstrap/simulation/prediction/*`).
//!
//! Re-exports mirror donor `src/app/bootstrap/simulation/prediction.ts`; the
//! presentation barrel entries land with the prediction presentation port.

pub mod qw_source_state;
pub mod runtime;
pub mod sequence;
pub mod source_state;
pub mod step;
#[cfg(test)]
mod test_support;
pub mod types;

pub use runtime::SelectedMovementPrediction;
pub use sequence::predict_movement_sequence;
pub use source_state::q2_prediction_snapshot;
pub use step::{copy_prediction_snapshot, predict_movement_command};
pub use types::{
    MovementPredictionOptions, MovementPredictionResult, MovementPredictionSnapshot, MovementProbeOptions,
    PredictionCommand,
};
