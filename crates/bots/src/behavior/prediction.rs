//! Bot movement projection from `src/bots/behavior/prediction.ts`.
//!
//! Source prediction requests replay selected movement over detached
//! input: no actor writes, no source frame loop. Each frame builds
//! provider input from the previous result, steps the selected
//! provider in prediction mode, and classifies stop events from actual
//! movement output. End velocity is the per-frame displacement rate.

use qa_core::math::Vec3;

use crate::behavior::q3::navigation_types::BotMovementPrediction;
use crate::behavior::q3::travel::types::{BotMovementStop, BotTravelPredictionResult};
use crate::error::BotsError;
use crate::movement_contract::{MovementExecution, MovementInput, MovementProvider, MovementResult, MovementServices};

/// Movement projection: provider plus input/stop classification.
pub trait BotMovementProjection {
    /// Selected movement provider.
    fn provider(&self) -> &dyn MovementProvider;
    /// Services for the projection.
    fn services(&self) -> MovementServices;
    /// Build input for a frame.
    fn input(&self, previous: Option<&MovementResult>, frame: i32, command_move: Vec3) -> MovementInput;
    /// Classify a stop from movement output.
    fn stop(&self, previous: Option<&MovementResult>, result: &MovementResult, frame: i32) -> BotMovementStop;
}

/// Project bot movement over detached input.
pub fn project_bot_movement(
    query: &BotMovementPrediction,
    projection: &dyn BotMovementProjection,
) -> Result<BotTravelPredictionResult, BotsError> {
    let provider = projection.provider();
    let services = projection.services();
    let mut end = query.origin;
    let mut velocity = query.velocity;
    let mut frames = 0;
    let mut stop_event = 0;
    let mut end_area: Option<i32> = None;
    let mut previous: Option<MovementResult> = None;
    let no_command = Vec3 { x: 0.0, y: 0.0, z: 0.0 };
    for frame in 0..query.max_frames {
        let command = if frame < query.command_frames {
            query.command_move
        } else {
            no_command
        };
        let input = projection.input(previous.as_ref(), frame, command);
        if input.execution != MovementExecution::Prediction || input.kind != provider.kind() {
            return Err(BotsError::PredictionMode);
        }
        if input.profile.id != *provider.id() {
            return Err(BotsError::PredictorMismatch);
        }
        let result = provider.advance(&input, &services);
        let MovementResult::Active { state, .. } = &result else {
            return Err(BotsError::BotLifetime(
                "movement projection removed the predicted actor".to_owned(),
            ));
        };
        let next_end = state.origin();
        if query.frame_time > 0.0 {
            velocity = Vec3 {
                x: (next_end.x - end.x) / query.frame_time,
                y: (next_end.y - end.y) / query.frame_time,
                z: (next_end.z - end.z) / query.frame_time,
            };
        }
        end = next_end;
        let stop = projection.stop(previous.as_ref(), &result, frame);
        end_area = stop.area;
        stop_event = stop.events & query.stop_events;
        if stop_event != 0 {
            end = stop.origin;
        }
        frames += 1;
        previous = Some(result);
        if stop_event != 0 {
            break;
        }
    }
    Ok(BotTravelPredictionResult {
        end,
        velocity,
        frames,
        stop_event,
        end_area,
    })
}
