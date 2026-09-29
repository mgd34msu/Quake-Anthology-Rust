//! Quake I movement result assembly.
//!
//! Donor provenance: `src/movement/q1/result.ts`.

use qa_core::math::Vec3;

use super::common::MovementContext;
use super::super::types::{
    LocomotionAnimation, MovementContinuation, MovementResultFields, TraceHit, UserCommand,
};
use super::types::{
    Q1AnimationStepInput, Q1MovementHooks, Q1MovementResult, Q1MovementServices, Q1MovementState,
    Q1State, Q1WeaponStepInput, QwMovementResult, QwMovementState,
};

/// Finish a Q1 step: run weapon/animation owners, select locomotion, and
/// assemble the result. Mirrors donor `finishMovement`.
use super::super::types::MovementError;

/// Either finished result.
#[derive(Debug, Clone, PartialEq)]
pub enum FinishOutcome<A, B> {
    /// NetQuake result.
    Netquake(A),
    /// QuakeWorld result.
    Qw(B),
}

pub fn finish_movement<S: Q1MovementServices, H: Q1MovementHooks>(
    context: &mut MovementContext<'_, S, H>,
    state: Q1State,
    mut view_angles: Vec3,
    mut ground: TraceHit,
    mut water_level: i32,
    mut water_type: i32,
) -> std::result::Result<FinishOutcome<Q1MovementResult, QwMovementResult>, MovementError> {
    finish_inner(context, state, &mut view_angles, &mut ground, &mut water_level, &mut water_type)
}

#[allow(clippy::too_many_arguments)]
fn finish_inner<S: Q1MovementServices, H: Q1MovementHooks>(
    context: &mut MovementContext<'_, S, H>,
    state: Q1State,
    view_angles: &mut Vec3,
    ground: &mut TraceHit,
    water_level: &mut i32,
    water_type: &mut i32,
) -> std::result::Result<
    FinishOutcome<Q1MovementResult, QwMovementResult>,
    MovementError,
> {
    let (command, actor_id, command_sequence, arsenal, animation, environment, frame) = (
        context.input_command(),
        context.input.actor().id().clone(),
        context.input.fields().command_sequence,
        context.input.fields().arsenal.clone(),
        context.input.fields().animation.clone(),
        context.input.fields().environment.clone(),
        context.input.frame().clone(),
    );
    let mut state = context.source_state(state);
    if !matches!(state, Q1State::Netquake(_) | Q1State::Quakeworld(_)) {
        return Err(MovementError::Contract("Source client output changed movement dialect"));
    }
    let weapon = context.services.weapon_step(
        Q1WeaponStepInput {
            actor: context.input.actor(),
            command: &command,
            frame: &frame,
            arsenal: &arsenal,
            animation: &animation,
            environment: &environment,
            gauntlet_hit: false,
        },
        &state,
    );
    for effect in weapon.effects {
        context.effect(effect);
    }
    match weapon.continuation {
        Some(MovementContinuation::ActorRemoved) => {
            let effects = std::mem::take(&mut context.effects);
            return Ok(match state {
                Q1State::Netquake(_) => {
                    FinishOutcome::Netquake(crate::movement::types::MovementOutcome::ActorRemoved {
                        actor: actor_id,
                        command_sequence,
                        effects,
                    })
                }
                Q1State::Quakeworld(_) => {
                    FinishOutcome::Qw(crate::movement::types::MovementOutcome::ActorRemoved {
                        actor: actor_id,
                        command_sequence,
                        effects,
                    })
                }
            });
        }
        Some(MovementContinuation::Continue(next)) => {
            let same = matches!(
                (&state, &next),
                (Q1State::Netquake(_), Q1State::Netquake(_))
                    | (Q1State::Quakeworld(_), Q1State::Quakeworld(_))
            );
            if !same {
                return Err(MovementError::Contract("Weapon callback changed movement family"));
            }
            match &next {
                Q1State::Netquake(inner) => {
                    *view_angles = inner.view_angles;
                    *ground = inner.ground.clone();
                    *water_level = inner.water_level;
                    *water_type = inner.water_type;
                }
                Q1State::Quakeworld(inner) => {
                    *view_angles = inner.angles;
                    *ground = inner.ground.clone();
                }
            }
            state = next;
        }
        None => {}
    }
    let horizontal_speed = match &state {
        Q1State::Netquake(inner) => context.math.horizontal(inner.velocity),
        Q1State::Quakeworld(inner) => context.math.horizontal(inner.velocity),
    };
    let forward_move = match &command {
        UserCommand::Q1Netquake(command) => command.forward_move,
        UserCommand::Q1Quakeworld(command) => command.forward_move,
        _ => 0.0,
    };
    let locomotion: LocomotionAnimation = if *water_level >= 2 {
        LocomotionAnimation::Swim
    } else if matches!(ground, TraceHit::None) {
        LocomotionAnimation::Jump
    } else if horizontal_speed > 0.0 {
        if forward_move < 0.0 {
            LocomotionAnimation::Backward
        } else {
            LocomotionAnimation::Run
        }
    } else {
        LocomotionAnimation::Idle
    };
    let animation_step = context.services.animation_step(Q1AnimationStepInput {
        actor: context.input.actor(),
        frame: &frame,
        animation: &weapon.animation,
        locomotion,
        backwards: forward_move < 0.0,
        force: false,
    });
    for effect in animation_step.effects {
        context.effect(effect);
    }
    let fields = MovementResultFields {
        actor: actor_id,
        command_sequence,
        bounds: context.bounds(),
        view_angles: *view_angles,
        view_height: context.view_height(),
        ground: ground.clone(),
        water_level: *water_level,
        water_type: *water_type,
        horizontal_speed,
        contacts: context.contacts.clone(),
        effects: context.effects.clone(),
        arsenal: weapon.arsenal,
        animation: animation_step.animation,
    };
    match state {
        Q1State::Netquake(state) => Ok(FinishOutcome::Netquake(
            crate::movement::types::MovementOutcome::Active { fields, state },
        )),
        Q1State::Quakeworld(state) => Ok(FinishOutcome::Qw(
            crate::movement::types::MovementOutcome::Active { fields, state },
        )),
    }
}

/// Finish a NetQuake step, rejecting dialect changes.
pub fn finish_netquake<S: Q1MovementServices, H: Q1MovementHooks>(
    context: &mut MovementContext<'_, S, H>,
    state: Q1MovementState,
    view_angles: Vec3,
    ground: TraceHit,
    water_level: i32,
    water_type: i32,
) -> std::result::Result<Q1MovementResult, MovementError> {
    match finish_movement(context, Q1State::Netquake(state), view_angles, ground, water_level, water_type)? {
        FinishOutcome::Netquake(result) => Ok(result),
        FinishOutcome::Qw(_) => Err(MovementError::Contract(
            "Weapon callback changed NetQuake movement family",
        )),
    }
}

/// Finish a QuakeWorld step, rejecting dialect changes.
pub fn finish_quakeworld<S: Q1MovementServices, H: Q1MovementHooks>(
    context: &mut MovementContext<'_, S, H>,
    state: QwMovementState,
    view_angles: Vec3,
    ground: TraceHit,
    water_level: i32,
    water_type: i32,
) -> std::result::Result<QwMovementResult, MovementError> {
    match finish_movement(
        context,
        Q1State::Quakeworld(state),
        view_angles,
        ground,
        water_level,
        water_type,
    )? {
        FinishOutcome::Qw(result) => Ok(result),
        FinishOutcome::Netquake(_) => Err(MovementError::Contract(
            "Weapon callback changed QuakeWorld movement family",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::{IdentityOwner, ProviderId};
    use qa_core::math::{Bounds, vec3};
    use qa_core::numeric::{NumericOps, Q1_DONOR_PROFILE};
    use qa_core::time::{ClockProfile, FrameContext, FramePhase, SourceTime};

    use super::super::common::MovementContext;
    use super::super::types::{
        NoQ1Hooks, Q1AnimationStepResult, Q1Edition, Q1MovementInput, Q1MovementOptions,
        Q1MovementProfile, Q1MovementState, Q1PlayerInput, Q1Trace, Q1TraceQuery,
        Q1WeaponStepResult,
    };
    use super::super::super::types::{
        ActorAnimationState, AnimationState, ArsenalState, MovementContinuation, MovementEnvironment,
        MovementExecution, MovementInputFields, MovementTouchContact, Q1UserCommand, TraceContact,
        UserCommand, WeaponState,
    };

    struct NullServices {
        ops: NumericOps,
    }

    impl Q1MovementServices for NullServices {
        fn numeric(&self) -> NumericOps {
            self.ops
        }
        fn trace(&mut self, query: Q1TraceQuery) -> Q1Trace {
            Q1Trace {
                fraction: 1.0,
                end: query.end,
                start_solid: false,
                all_solid: false,
                contact: TraceContact::None,
                hit: TraceHit::None,
                in_open: true,
                in_water: false,
                source_plane: qa_core::math::Plane {
                    normal: vec3(0.0, 0.0, 1.0),
                    distance: 0.0,
                },
                surface_flags: None,
            }
        }
        fn point_contents(&mut self, _point: Vec3) -> i32 {
            -1
        }
        fn touch(
            &mut self,
            _contact: MovementTouchContact,
            state: Q1State,
        ) -> MovementContinuation<Q1State> {
            MovementContinuation::Continue(state)
        }
        fn weapon_step(
            &mut self,
            input: super::super::types::Q1WeaponStepInput<'_>,
            _state: &Q1State,
        ) -> Q1WeaponStepResult {
            Q1WeaponStepResult {
                continuation: None,
                arsenal: (*input.arsenal).clone(),
                animation: (*input.animation).clone(),
                effects: Vec::new(),
            }
        }
        fn animation_step(
            &mut self,
            input: super::super::types::Q1AnimationStepInput<'_>,
        ) -> Q1AnimationStepResult {
            Q1AnimationStepResult {
                animation: (*input.animation).clone(),
                effects: Vec::new(),
            }
        }
    }

    fn fixture(forward_move: f64, velocity: Vec3) -> (Q1MovementInput, NullServices) {
        let owner = IdentityOwner::create("q1-result").unwrap();
        let id = owner.actor(1, 0);
        let actor = owner.owned_actor(&id, ProviderId::new("q1", "test")).unwrap();
        let input = Q1MovementInput {
            fields: MovementInputFields {
                actor,
                command_sequence: 7,
                frame: FrameContext {
                    frame: 1,
                    time: SourceTime::Seconds(1.0),
                    elapsed: SourceTime::Seconds(0.05),
                    phase: FramePhase::EntityPhysics,
                },
                shape: super::super::super::types::TraceShape::Box(Bounds {
                    min: vec3(-16.0, -16.0, -24.0),
                    max: vec3(16.0, 16.0, 32.0),
                }),
                current_bounds: None,
                environment: MovementEnvironment::default(),
                arsenal: ArsenalState {
                    provider: ProviderId::new("q1", "test"),
                    active_weapon: None,
                    state: WeaponState::Q1 {
                        frame: 0,
                        attack_finished_seconds: 0.0,
                        source_weapon: 0,
                    },
                    ammo: Vec::new(),
                },
                animation: ActorAnimationState {
                    provider: ProviderId::new("q1", "test"),
                    state: AnimationState::Q1 {
                        frame: 0,
                        next_frame_seconds: 0.0,
                    },
                },
                execution: MovementExecution::Authoritative,
            },
            command: Q1UserCommand {
                acknowledged_server_time_seconds: 0.0,
                view_angles: vec3(0.0, 0.0, 0.0),
                forward_move,
                side_move: 0.0,
                up_move: 0.0,
                buttons: 0,
                impulse: 0,
            },
            state: Q1MovementState {
                origin: vec3(0.0, 0.0, 0.0),
                velocity,
                angles: vec3(0.0, 0.0, 0.0),
                old_origin: vec3(0.0, 0.0, 0.0),
                angular_velocity: vec3(0.0, 0.0, 0.0),
                view_angles: vec3(1.0, 2.0, 3.0),
                punch_angles: vec3(0.0, 0.0, 0.0),
                move_type: 3,
                flags: 0,
                ground: TraceHit::None,
                water_level: 0,
                water_type: -1,
                teleport_time_seconds: 0.0,
                water_jump_direction: vec3(0.0, 0.0, 0.0),
                ideal_pitch: 0.0,
                fix_angle: false,
                health: 100.0,
            },
            profile: Q1MovementProfile {
                id: ProviderId::new("q1", "test"),
                clock: ClockProfile::Q1Netquake {
                    minimum_frame_seconds: 0.001,
                    maximum_frame_seconds: 0.1,
                    fixed_frame_seconds: None,
                },
                numeric: Q1_DONOR_PROFILE,
                edition: Q1Edition::Classic,
                parameters: crate::movement::Q1MovementParameters {
                    gravity: 800.0,
                    stop_speed: 100.0,
                    max_speed: 320.0,
                    spectator_max_speed: 500.0,
                    accelerate: 10.0,
                    air_accelerate: 1.0,
                    water_accelerate: 4.0,
                    friction: 4.0,
                    water_friction: 2.0,
                    entity_gravity: 1.0,
                },
                edge_friction: 2.0,
                no_clip_angle_hack: false,
            },
        };
        let services = NullServices {
            ops: NumericOps::select(Q1_DONOR_PROFILE).unwrap(),
        };
        (input, services)
    }

    #[test]
    fn finish_reports_grounded_run() {
        let (input, mut services) = fixture(200.0, vec3(100.0, 0.0, 0.0));
        let state = input.state.clone();
        let mut context = MovementContext::new(
            Q1PlayerInput::Netquake(input),
            &mut services,
            Q1MovementOptions::<NoQ1Hooks>::default(),
        );
        let result = finish_netquake(
            &mut context,
            state,
            vec3(1.0, 2.0, 3.0),
            TraceHit::World { model: 0 },
            0,
            -1,
        )
        .unwrap();
        match result {
            crate::movement::types::MovementOutcome::Active { fields, state } => {
                assert_eq!(fields.ground, TraceHit::World { model: 0 });
                assert_eq!(fields.horizontal_speed, 100.0);
                assert_eq!(state.move_type, 3);
                let _ = UserCommand::Q1Netquake(Q1UserCommand {
                    acknowledged_server_time_seconds: 0.0,
                    view_angles: vec3(0.0, 0.0, 0.0),
                    forward_move: 0.0,
                    side_move: 0.0,
                    up_move: 0.0,
                    buttons: 0,
                    impulse: 0,
                });
            }
            _ => panic!("expected active result"),
        }
    }

    #[test]
    fn finish_reports_airborne_and_swim() {
        let (input, mut services) = fixture(0.0, vec3(0.0, 0.0, -50.0));
        let state = input.state.clone();
        let mut context = MovementContext::new(
            Q1PlayerInput::Netquake(input),
            &mut services,
            Q1MovementOptions::<NoQ1Hooks>::default(),
        );
        let result =
            finish_netquake(&mut context, state, vec3(0.0, 0.0, 0.0), TraceHit::None, 3, -3).unwrap();
        match result {
            crate::movement::types::MovementOutcome::Active { fields, .. } => {
                assert_eq!(fields.water_level, 3);
                assert_eq!(fields.water_type, -3);
            }
            _ => panic!("expected active result"),
        }
    }
}
