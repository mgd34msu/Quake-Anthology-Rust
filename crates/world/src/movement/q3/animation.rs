//! Quake III animation operations (`PM_*` animation from `bg_pmove.c`).
//!
//! Donor provenance: `src/movement/q3/animation.ts`.

use super::super::types::{
    ActorAnimationState, AnimationState, MovementEffect, MovementError, PredictableMovementEvent,
};
use super::constants::{command_buttons as B, entity_event, move_type, player_animation as A};
use super::types::{Q3AnimationRequest, Q3AnimationStepResult, Q3HookContext, Q3Product};

/// Animation operation context, mirroring donor `Q3AnimationContext`.
#[derive(Debug, Clone, PartialEq)]
pub struct Q3AnimationContext {
    /// Animation snapshot.
    pub animation: ActorAnimationState,
    /// Dead flag.
    pub dead: bool,
    /// Elapsed milliseconds.
    pub elapsed_milliseconds: i32,
    /// Command buttons.
    pub buttons: i32,
    /// Product selector.
    pub product: Q3Product,
    /// Event sequence.
    pub event_sequence: i32,
}

fn result(context: &Q3AnimationContext, state: AnimationState, events: &[i32]) -> Q3AnimationStepResult {
    let mut effects = Vec::new();
    if state != context.animation.state {
        effects.push(MovementEffect::Animation {
            provider: context.animation.provider.clone(),
            before: context.animation.state,
            after: state,
        });
    }
    for event in events {
        effects.push(MovementEffect::Event(PredictableMovementEvent {
            provider: context.animation.provider.clone(),
            sequence: context.event_sequence,
            event: *event,
            parameter: 0,
        }));
    }
    Q3AnimationStepResult {
        animation: ActorAnimationState {
            provider: context.animation.provider.clone(),
            state,
        },
        effects,
    }
}

fn q3_state(context: &Q3AnimationContext) -> Result<(i32, i32, i32, i32), MovementError> {
    match context.animation.state {
        AnimationState::Q3 {
            legs,
            torso,
            legs_timer_milliseconds,
            torso_timer_milliseconds,
        } => Ok((legs, torso, legs_timer_milliseconds, torso_timer_milliseconds)),
        _ => Err(MovementError::Contract(
            "Q3 animation operations require the selected Q3 character adapter",
        )),
    }
}

/// A character adapter selects this only for an actual Q3 animation state.
pub fn run_q3_animation_operation(
    request: Q3AnimationRequest,
    context: &Q3AnimationContext,
) -> Result<Q3AnimationStepResult, MovementError> {
    let (legs, torso, legs_timer, torso_timer) = q3_state(context)?;
    match request {
        Q3AnimationRequest::Legs { animation, force } => {
            let timer = if force { 0 } else { legs_timer };
            if context.dead || timer > 0 || (!force && (legs & !128) == animation) {
                let state = if timer == legs_timer {
                    context.animation.state
                } else {
                    AnimationState::Q3 {
                        legs,
                        torso,
                        legs_timer_milliseconds: timer,
                        torso_timer_milliseconds: torso_timer,
                    }
                };
                return Ok(result(context, state, &[]));
            }
            Ok(result(
                context,
                AnimationState::Q3 {
                    legs: ((legs & 128) ^ 128) | animation,
                    torso,
                    legs_timer_milliseconds: timer,
                    torso_timer_milliseconds: torso_timer,
                },
                &[],
            ))
        }
        Q3AnimationRequest::LegsTimer { milliseconds } => Ok(result(
            context,
            AnimationState::Q3 {
                legs,
                torso,
                legs_timer_milliseconds: milliseconds,
                torso_timer_milliseconds: torso_timer,
            },
            &[],
        )),
        Q3AnimationRequest::DropTimers => {
            let elapsed = context.elapsed_milliseconds;
            Ok(result(
                context,
                AnimationState::Q3 {
                    legs,
                    torso,
                    legs_timer_milliseconds: if legs_timer > 0 {
                        0.max(legs_timer - elapsed)
                    } else {
                        legs_timer
                    },
                    torso_timer_milliseconds: if torso_timer > 0 {
                        0.max(torso_timer - elapsed)
                    } else {
                        torso_timer
                    },
                },
                &[],
            ))
        }
        Q3AnimationRequest::Gesture => {
            if torso_timer != 0 {
                return Ok(result(context, context.animation.state, &[]));
            }
            if context.buttons & B::GESTURE != 0 {
                return Ok(result(
                    context,
                    AnimationState::Q3 {
                        legs,
                        torso: if !context.dead {
                            ((torso & 128) ^ 128) | A::TORSO_GESTURE
                        } else {
                            torso
                        },
                        legs_timer_milliseconds: legs_timer,
                        torso_timer_milliseconds: 34 * 66 + 50,
                    },
                    &[entity_event::TAUNT],
                ));
            }
            if context.product == Q3Product::MissionPack {
                let gestures: [(i32, i32); 6] = [
                    (B::GETFLAG, A::TORSO_GETFLAG),
                    (B::GUARDBASE, A::TORSO_GUARDBASE),
                    (B::PATROL, A::TORSO_PATROL),
                    (B::FOLLOWME, A::TORSO_FOLLOWME),
                    (B::AFFIRMATIVE, A::TORSO_AFFIRMATIVE),
                    (B::NEGATIVE, A::TORSO_NEGATIVE),
                ];
                for (button, animation) in gestures {
                    if context.buttons & button != 0 {
                        return Ok(result(
                            context,
                            AnimationState::Q3 {
                                legs,
                                torso: if !context.dead {
                                    ((torso & 128) ^ 128) | animation
                                } else {
                                    torso
                                },
                                legs_timer_milliseconds: legs_timer,
                                torso_timer_milliseconds: 600,
                            },
                            &[],
                        ));
                    }
                }
            }
            Ok(result(context, context.animation.state, &[]))
        }
    }
}

/// `PM_StartTorsoAnim` and `PM_ContinueTorsoAnim` for the selected Q3
/// character.
pub fn run_q3_torso_operation(
    animation: i32,
    context: &Q3AnimationContext,
    continue_animation: bool,
) -> Result<Q3AnimationStepResult, MovementError> {
    let (legs, torso, legs_timer, torso_timer) = q3_state(context)
        .map_err(|_| MovementError::Contract("Q3 torso operations require the selected Q3 character adapter"))?;
    if context.dead || (continue_animation && ((torso & !128) == animation || torso_timer > 0)) {
        return Ok(result(context, context.animation.state, &[]));
    }
    Ok(result(
        context,
        AnimationState::Q3 {
            legs,
            torso: ((torso & 128) ^ 128) | animation,
            legs_timer_milliseconds: legs_timer,
            torso_timer_milliseconds: torso_timer,
        },
        &[],
    ))
}

fn character_context(context: &Q3HookContext) -> Q3AnimationContext {
    Q3AnimationContext {
        animation: context.animation.clone(),
        dead: context.motion.pm_type >= move_type::DEAD,
        elapsed_milliseconds: context.frame.elapsed.as_milliseconds_truncated(),
        buttons: context.command.buttons,
        product: context.motion.product,
        event_sequence: context.motion.event_sequence,
    }
}

/// Source animation adapter at a locomotion call site.
pub fn q3_source_animation(
    request: Q3AnimationRequest,
    context: &Q3HookContext,
) -> Result<Q3AnimationStepResult, MovementError> {
    let animation_context = character_context(context);
    run_q3_animation_operation(request, &animation_context)
}

/// Source torso adapter at a locomotion call site.
pub fn q3_source_torso(
    animation: i32,
    context: &Q3HookContext,
    continue_animation: bool,
) -> Result<Q3AnimationStepResult, MovementError> {
    let animation_context = character_context(context);
    run_q3_torso_operation(animation, &animation_context, continue_animation)
}

#[cfg(test)]
mod tests {
    use qa_core::identity::ProviderId;

    use super::*;

    fn context() -> Q3AnimationContext {
        Q3AnimationContext {
            animation: ActorAnimationState {
                provider: ProviderId::new("q3", "test"),
                state: AnimationState::Q3 {
                    legs: A::LEGS_IDLE,
                    torso: A::TORSO_STAND,
                    legs_timer_milliseconds: 0,
                    torso_timer_milliseconds: 0,
                },
            },
            dead: false,
            elapsed_milliseconds: 50,
            buttons: 0,
            product: Q3Product::BaseQ3,
            event_sequence: 3,
        }
    }

    #[test]
    fn legs_switch_toggles_animation_bit() {
        let out = run_q3_animation_operation(
            Q3AnimationRequest::Legs {
                animation: A::LEGS_RUN,
                force: false,
            },
            &context(),
        )
        .unwrap();
        match out.animation.state {
            AnimationState::Q3 { legs, .. } => {
                assert_eq!(legs & !128, A::LEGS_RUN);
                assert_eq!(legs & 128, 128);
            }
            _ => panic!("family changed"),
        }
        assert_eq!(out.effects.len(), 1);
    }

    #[test]
    fn legs_repeat_without_force_is_identity() {
        let mut context = context();
        context.animation.state = AnimationState::Q3 {
            legs: A::LEGS_RUN,
            torso: A::TORSO_STAND,
            legs_timer_milliseconds: 0,
            torso_timer_milliseconds: 0,
        };
        let out = run_q3_animation_operation(
            Q3AnimationRequest::Legs {
                animation: A::LEGS_RUN,
                force: false,
            },
            &context,
        )
        .unwrap();
        assert!(out.effects.is_empty());
    }

    #[test]
    fn timers_drop_with_elapsed() {
        let mut context = context();
        context.animation.state = AnimationState::Q3 {
            legs: A::LEGS_RUN,
            torso: A::TORSO_STAND,
            legs_timer_milliseconds: 130,
            torso_timer_milliseconds: 20,
        };
        let out = run_q3_animation_operation(Q3AnimationRequest::DropTimers, &context).unwrap();
        match out.animation.state {
            AnimationState::Q3 {
                legs_timer_milliseconds,
                torso_timer_milliseconds,
                ..
            } => {
                assert_eq!((legs_timer_milliseconds, torso_timer_milliseconds), (80, 0));
            }
            _ => panic!("family changed"),
        }
    }

    #[test]
    fn gesture_emits_taunt() {
        let mut context = context();
        context.buttons = B::GESTURE;
        let out = run_q3_animation_operation(Q3AnimationRequest::Gesture, &context).unwrap();
        assert_eq!(out.effects.len(), 2);
        match out.animation.state {
            AnimationState::Q3 {
                torso_timer_milliseconds,
                ..
            } => assert_eq!(torso_timer_milliseconds, 34 * 66 + 50),
            _ => panic!("family changed"),
        }
    }

    #[test]
    fn torso_continue_holds_running_animation() {
        let mut context = context();
        context.animation.state = AnimationState::Q3 {
            legs: A::LEGS_IDLE,
            torso: A::TORSO_ATTACK,
            legs_timer_milliseconds: 0,
            torso_timer_milliseconds: 100,
        };
        let out = run_q3_torso_operation(A::TORSO_ATTACK, &context, true).unwrap();
        assert!(out.effects.is_empty());
        let out = run_q3_torso_operation(A::TORSO_DROP, &context, false).unwrap();
        match out.animation.state {
            AnimationState::Q3 { torso, .. } => assert_eq!(torso & !128, A::TORSO_DROP),
            _ => panic!("family changed"),
        }
    }

    #[test]
    fn foreign_states_are_rejected() {
        let mut context = context();
        context.animation.state = AnimationState::Q1 {
            frame: 0,
            next_frame_seconds: 0.0,
        };
        assert!(run_q3_animation_operation(Q3AnimationRequest::Gesture, &context).is_err());
        assert!(run_q3_torso_operation(A::TORSO_STAND, &context, false).is_err());
    }
}
