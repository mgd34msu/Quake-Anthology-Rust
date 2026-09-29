//! Quake III jump-pad touch (`BG_TouchJumpPad`).
//!
//! Donor provenance: `src/movement/q3/jump-pad.ts` (from id Software
//! `code/game/bg_misc.c`).

use qa_core::identity::{same_actor, ActorId, ProviderId};
use qa_core::math::{angle_normalize180, vector_to_angles, Vec3};

use super::super::types::PredictableMovementEvent;
use super::constants::{entity_event, move_type};
use super::types::Q3MovementState;

/// Jump-pad touch result.
#[derive(Debug, Clone, PartialEq)]
pub struct Q3JumpPadResult {
    /// Updated state.
    pub state: Q3MovementState,
    /// Predictable event, when the pad changed.
    pub event: Option<PredictableMovementEvent>,
}

/// Touch a jump pad; only normal movers without flight trigger.
#[must_use]
pub fn touch_q3_jump_pad(
    state: &Q3MovementState,
    pad: &ActorId,
    velocity: Vec3,
    flight: bool,
    provider: &ProviderId,
) -> Q3JumpPadResult {
    if state.movement_type != move_type::NORMAL || flight {
        return Q3JumpPadResult {
            state: state.clone(),
            event: None,
        };
    }
    let changed = state.jump_pad.as_ref().is_none_or(|current| !same_actor(current, pad));
    let pitch = angle_normalize180(f64::from(vector_to_angles(velocity).x)).abs();
    Q3JumpPadResult {
        state: Q3MovementState {
            jump_pad: Some(pad.clone()),
            jump_pad_frame: state.movement_frame,
            velocity: Vec3 {
                x: velocity.x,
                y: velocity.y,
                z: velocity.z,
            },
            predictable_event_sequence: if changed {
                state.predictable_event_sequence.wrapping_add(1)
            } else {
                state.predictable_event_sequence
            },
            ..state.clone()
        },
        event: changed.then(|| PredictableMovementEvent {
            provider: provider.clone(),
            sequence: state.predictable_event_sequence,
            event: entity_event::JUMP_PAD,
            parameter: i32::from(pitch >= 45.0),
        }),
    }
}

/// End of `CG_TouchTriggerPrediction`, after all eligible source triggers
/// have run.
#[must_use]
pub fn finish_q3_jump_pad_prediction(state: &Q3MovementState) -> Q3MovementState {
    if state.jump_pad_frame == state.movement_frame {
        state.clone()
    } else {
        Q3MovementState {
            jump_pad: None,
            jump_pad_frame: 0,
            ..state.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;
    use qa_core::math::vec3;

    use super::super::super::types::TraceHit;

    fn state() -> Q3MovementState {
        Q3MovementState {
            command_time_milliseconds: 100,
            movement_type: move_type::NORMAL,
            bob_cycle: 0,
            movement_flags: 0,
            movement_time_milliseconds: 0,
            origin: vec3(0.0, 0.0, 0.0),
            velocity: vec3(0.0, 0.0, 0.0),
            gravity: 800.0,
            speed: 320.0,
            delta_angle_words: [0, 0, 0],
            movement_direction: 0,
            grapple_point: vec3(0.0, 0.0, 0.0),
            flags: 0,
            view_angles: vec3(0.0, 0.0, 0.0),
            view_height: 26.0,
            ground: TraceHit::None,
            predictable_event_sequence: 7,
            jump_pad: None,
            movement_frame: 3,
            jump_pad_frame: 0,
        }
    }

    #[test]
    fn first_touch_emits_event_and_sequence() {
        let owner = IdentityOwner::create("q3-pad").unwrap();
        let pad = owner.actor(9, 0);
        let result = touch_q3_jump_pad(
            &state(),
            &pad,
            vec3(0.0, 0.0, 500.0),
            false,
            &ProviderId::new("q3", "test"),
        );
        assert_eq!(result.state.jump_pad, Some(pad));
        assert_eq!(result.state.predictable_event_sequence, 8);
        let event = result.event.unwrap();
        assert_eq!((event.event, event.sequence), (entity_event::JUMP_PAD, 7));
    }

    #[test]
    fn repeat_touch_skips_event() {
        let owner = IdentityOwner::create("q3-pad").unwrap();
        let pad = owner.actor(9, 0);
        let first = touch_q3_jump_pad(
            &state(),
            &pad,
            vec3(0.0, 0.0, 500.0),
            false,
            &ProviderId::new("q3", "test"),
        );
        let second = touch_q3_jump_pad(
            &first.state,
            &pad,
            vec3(0.0, 0.0, 500.0),
            false,
            &ProviderId::new("q3", "test"),
        );
        assert!(second.event.is_none());
        assert_eq!(second.state.predictable_event_sequence, 8);
    }

    #[test]
    fn flight_and_noclip_ignore_pads() {
        let owner = IdentityOwner::create("q3-pad").unwrap();
        let pad = owner.actor(9, 0);
        let flying = touch_q3_jump_pad(
            &state(),
            &pad,
            vec3(0.0, 0.0, 500.0),
            true,
            &ProviderId::new("q3", "test"),
        );
        assert!(flying.event.is_none());
        assert_eq!(flying.state, state());
        let mut noclip = state();
        noclip.movement_type = move_type::NOCLIP;
        let result = touch_q3_jump_pad(
            &noclip,
            &pad,
            vec3(0.0, 0.0, 500.0),
            false,
            &ProviderId::new("q3", "test"),
        );
        assert_eq!(result.state, noclip);
    }

    #[test]
    fn stale_pad_clears_after_prediction() {
        let owner = IdentityOwner::create("q3-pad").unwrap();
        let pad = owner.actor(9, 0);
        let touched = touch_q3_jump_pad(
            &state(),
            &pad,
            vec3(0.0, 0.0, 500.0),
            false,
            &ProviderId::new("q3", "test"),
        )
        .state;
        assert_eq!(finish_q3_jump_pad_prediction(&touched), touched);
        let mut stale = touched;
        stale.movement_frame = 4;
        let cleared = finish_q3_jump_pad_prediction(&stale);
        assert_eq!((cleared.jump_pad, cleared.jump_pad_frame), (None, 0));
    }
}
