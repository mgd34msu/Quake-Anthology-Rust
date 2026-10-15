use qa_core::{
    command_history::{Acknowledgement, CommandHistory},
    primitives::{
        AngleOffset, AngleOffsets, Body, MovementMode, MovementTimer, PlayerState,
        PredictionPolicy, RuleSetId, UserCmd, Vec3,
    },
};
use qa_movement::TraceServices;
use qa_session::prediction::Prediction;
use qa_world::collision::{Contents, EntityTracePolicy, Trace, TraceError, TraceQuery};

struct NoQueries;
impl TraceServices for NoQueries {
    fn trace(&mut self, _: TraceQuery) -> Result<Trace, TraceError> {
        panic!("nonpositional prediction must not trace")
    }
    fn point_contents(&self, _: Vec3, _: EntityTracePolicy) -> Result<Contents, TraceError> {
        panic!("nonpositional prediction must not sample contents")
    }
}

struct FreeSpace;
impl TraceServices for FreeSpace {
    fn trace(&mut self, query: TraceQuery) -> Result<Trace, TraceError> {
        Ok(Trace::clear(query.end))
    }
    fn point_contents(&self, _: Vec3, _: EntityTracePolicy) -> Result<Contents, TraceError> {
        Ok(Contents::EMPTY)
    }
}

fn body_bits(body: Body) -> [[u32; 3]; 4] {
    [body.position, body.velocity, body.mins, body.maxs].map(|v| v.0.map(f32::to_bits))
}

#[test]
fn intermission_lock_survives_angle_only_and_exhausted_history_prediction() {
    for rules in RuleSetId::ALL {
        for mode in [
            MovementMode::Intermission,
            MovementMode::SinglePlayerIntermission,
        ] {
            let mut prediction = Prediction::default();
            let player = &mut prediction.player;
            player.health = 100;
            player.movement_rules = rules;
            player.trace_rules = RuleSetId::Quake2;
            player.movement.mode = mode;
            player.view_angles = Vec3([-0.0, 117.25, 6.5]);
            player.movement.delta_angles = AngleOffsets([AngleOffset::Units(17000); 3]);
            player.prediction.position = false;
            let angles = player.view_angles.0.map(f32::to_bits);
            let delta = player.movement.delta_angles;
            let command = UserCmd {
                duration_ms: 16,
                server_time_ms: 16,
                view_angles: Vec3([180.0, 0.0, 90.0]),
                ..Default::default()
            };
            prediction.advance(command, &mut NoQueries).unwrap();
            assert_eq!(prediction.player.view_angles.0.map(f32::to_bits), angles);
            assert_eq!(prediction.player.movement.delta_angles, delta);
            prediction.player.prediction.position = true;
            prediction.remember_snapshot(Some(Acknowledgement::Number(0)));
            let mut history = CommandHistory::load(2, 0).unwrap();
            for number in 1..=3 {
                history.append(UserCmd {
                    server_time_ms: number * 16,
                    ..command
                });
            }
            assert_eq!(prediction.replay(&history, &mut NoQueries).unwrap(), 0);
            assert_eq!(prediction.history_resyncs, 1);
            assert_eq!(prediction.replay(&history, &mut NoQueries).unwrap(), 0);
            assert_eq!(prediction.player.view_angles.0.map(f32::to_bits), angles);
            assert_eq!(prediction.player.movement.delta_angles, delta);
            assert_eq!(prediction.player.movement.mode, mode);
            assert_eq!(prediction.player.prediction.angles, true);
            assert_eq!(prediction.player.movement_rules, rules);
            assert_eq!(prediction.player.trace_rules, RuleSetId::Quake2);
        }
    }
}

#[test]
fn angles_only_prediction_uses_native_unclamped_command_delta_without_movement() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld, RuleSetId::Quake2] {
        for command_angles in [Vec3([179.0, -0.0, 360.0]), Vec3([-270.0, 721.0, -17.25])] {
            let mut prediction = Prediction::default();
            let player = &mut prediction.player;
            player.movement_rules = rules;
            player.trace_rules = RuleSetId::QuakeWorld;
            player.prediction = PredictionPolicy {
                position: false,
                angles: true,
            };
            player.body.position = Vec3([17.125, -0.0, 137.0]);
            player.body.velocity = Vec3([3.0, 7.0, -1.0]);
            player.view_angles = Vec3([-19.0, 117.0, 7.0]);
            player.movement.delta_angles = AngleOffsets([
                AngleOffset::Units(8192),
                AngleOffset::Units(-32768),
                AngleOffset::Units(2048),
            ]);
            player.movement.timer = MovementTimer::TELEPORT;
            player.movement.remaining_ms = 17;
            player.movement.command_time_ms = 73;
            let body = body_bits(player.body);
            let movement = player.movement;
            // Original Q2 cl_pred.c:216-223. This branch neither quantizes the
            // command nor applies Pmove's pitch/teleport handling.
            let expected = command_angles + player.movement.delta_angles.degrees();
            let result = prediction
                .advance(
                    UserCmd {
                        duration_ms: 16,
                        server_time_ms: 100,
                        movement: [127.0, 64.0, 32.0],
                        units: qa_gameplay::rules::command_units(rules),
                        view_angles: command_angles,
                        ..Default::default()
                    },
                    &mut NoQueries,
                )
                .unwrap();
            assert_eq!(
                (result.steps, result.traces, result.contact_count),
                (0, 0, 0)
            );
            assert_eq!(body_bits(prediction.player.body), body);
            assert_eq!(prediction.player.movement, movement);
            assert_eq!(
                prediction.player.view_angles.0.map(f32::to_bits),
                expected.0.map(f32::to_bits)
            );
        }
    }
}

#[test]
fn foreign_no_position_permission_retains_q3_angle_rounding_and_pitch_delta_adjustment() {
    let mut prediction = Prediction::default();
    let player = &mut prediction.player;
    player.health = 100;
    player.movement_rules = RuleSetId::Quake3;
    player.trace_rules = RuleSetId::Quake2;
    player.prediction = PredictionPolicy {
        position: false,
        angles: true,
    };
    player.body.position = Vec3([13.0, -0.0, 27.0]);
    player.movement.delta_angles = AngleOffsets([AngleOffset::Units(8192); 3]);
    player.movement.command_time_ms = 1000;
    player.movement.remaining_ms = 19;
    player.movement.timer = MovementTimer::TELEPORT;
    let body = body_bits(player.body);
    let command = UserCmd {
        duration_ms: 16,
        server_time_ms: 16,
        view_angles: Vec3([179.0, 11.25, -0.0]),
        ..Default::default()
    };
    prediction.advance(command, &mut NoQueries).unwrap();
    assert_eq!(body_bits(prediction.player.body), body);
    assert_eq!(prediction.player.movement.command_time_ms, 1000);
    assert_eq!(prediction.player.movement.remaining_ms, 19);
    assert_eq!(prediction.player.movement.timer, MovementTimer::TELEPORT);
    assert_eq!(
        prediction.player.view_angles,
        Vec3([qa_core::math::short_to_angle(-16000), 56.25, 45.0])
    );
    assert_eq!(
        prediction.player.movement.delta_angles.0[0],
        AngleOffset::Units(
            -16000
                - i32::from(qa_core::math::angle_to_short(
                    command.view_angles.0[0],
                    qa_core::math::AngleShortForm::Factored
                ) as u16)
        )
    );
}

#[test]
fn disabled_angles_move_using_the_accepted_view_without_mutating_delta_pitch() {
    let mut prediction = Prediction::default();
    let player = &mut prediction.player;
    player.health = 100;
    player.movement_rules = RuleSetId::Quake3;
    player.trace_rules = RuleSetId::Quake;
    player.movement.mode = MovementMode::Noclip;
    player.prediction = PredictionPolicy {
        position: true,
        angles: false,
    };
    player.view_angles = Vec3([-0.0, 90.0, 0.0]);
    player.movement.delta_angles = AngleOffsets([AngleOffset::Units(17000); 3]);
    let accepted = player.view_angles;
    let delta = player.movement.delta_angles;
    let command = UserCmd {
        duration_ms: 16,
        server_time_ms: 16,
        movement: [127.0, 0.0, 0.0],
        units: qa_gameplay::rules::command_units(RuleSetId::Quake3),
        view_angles: Vec3([180.0, 0.0, 17.0]),
        ..Default::default()
    };
    let result = prediction.advance(command, &mut NoQueries).unwrap();
    assert!(result.steps > 0);
    assert!(prediction.player.body.position.0[1] > 0.0);
    assert_eq!(
        prediction.player.view_angles.0.map(f32::to_bits),
        accepted.0.map(f32::to_bits)
    );
    assert_eq!(prediction.player.movement.delta_angles, delta);

    // SERVER uses the same mover with its ordinary invocation policy; CLIENT's
    // prediction permission must never become a server movement suppression.
    qa_movement::pmove(
        UserCmd {
            server_time_ms: 32,
            ..command
        },
        &mut prediction.player,
        &mut NoQueries,
        Default::default(),
        qa_movement::no_boundary,
    )
    .unwrap();
    assert_ne!(
        prediction.player.view_angles.0.map(f32::to_bits),
        accepted.0.map(f32::to_bits)
    );
}

#[test]
fn both_disabled_preserves_every_hot_field_even_with_a_nonzero_command() {
    let mut prediction = Prediction::default();
    prediction.player.prediction = PredictionPolicy::DISABLED;
    prediction.player.view_angles = Vec3([-0.0, 133.0, -19.0]);
    let view = prediction.player.view_angles;
    let body = body_bits(prediction.player.body);
    let movement = prediction.player.movement;
    prediction
        .advance(
            UserCmd {
                duration_ms: 100,
                server_time_ms: 200,
                movement: [127.0; 3],
                units: qa_gameplay::rules::command_units(RuleSetId::default()),
                view_angles: Vec3([90.0; 3]),
                ..Default::default()
            },
            &mut NoQueries,
        )
        .unwrap();
    assert_eq!(prediction.player.movement, movement);
    assert_eq!(body_bits(prediction.player.body), body);
    assert_eq!(
        prediction.player.view_angles.0.map(f32::to_bits),
        view.0.map(f32::to_bits)
    );
}

fn teleport_prediction(rules: RuleSetId, policy: PredictionPolicy) -> Prediction {
    let mut player = PlayerState {
        movement_rules: rules,
        trace_rules: RuleSetId::QuakeWorld,
        health: 100,
        prediction: policy,
        view_angles: Vec3([-0.0, 133.25, -17.125]),
        ..Default::default()
    };
    qa_movement::set_bounds(&mut player);
    player.body.position = Vec3([17.125, -0.0, 137.0]);
    player.movement.mode = MovementMode::Walk;
    player.movement.delta_angles = AngleOffsets([
        AngleOffset::Degrees(7.25),
        AngleOffset::Degrees(11.5),
        AngleOffset::Degrees(-5.0),
    ]);
    player.movement.timer = MovementTimer::TELEPORT;
    player.movement.remaining_ms = 8;
    Prediction::new(player)
}

fn teleport_command(rules: RuleSetId) -> UserCmd {
    UserCmd {
        duration_ms: 16,
        duration_ns: 16_000_000,
        server_time_ms: 16,
        view_angles: Vec3([40.0, 13.25, 17.125]),
        units: qa_gameplay::rules::command_units(rules),
        ..Default::default()
    }
}

#[test]
fn q2_pitch_drift_keeps_the_lock_admitted_before_movement_expired_teleport() {
    for rules in [RuleSetId::Quake2, RuleSetId::Quake2Rerelease] {
        for replay in [false, true] {
            let mut prediction = teleport_prediction(
                rules,
                PredictionPolicy {
                    position: true,
                    angles: true,
                },
            );
            let command = teleport_command(rules);
            if replay {
                prediction.remember_snapshot(Some(Acknowledgement::Number(0)));
                let mut history = CommandHistory::load(2, 0).unwrap();
                history.append(command);
                assert_eq!(prediction.replay(&history, &mut FreeSpace).unwrap(), 1);
            } else {
                assert!(prediction.advance(command, &mut FreeSpace).unwrap().steps > 0);
            }
            assert_eq!(prediction.player.movement.remaining_ms, 0, "{rules:?}");
            assert!(
                !prediction
                    .player
                    .movement
                    .timer
                    .contains(MovementTimer::TELEPORT)
            );
            assert_eq!(prediction.player.view_angles.0[0], 0.0);
            assert_eq!(prediction.player.view_angles.0[2], 0.0);
            let view = prediction.player.view_angles.0.map(f32::to_bits);
            let body = body_bits(prediction.player.body);
            let movement = prediction.player.movement;

            prediction.drift_pitch(command, 40.0, 100.5);

            assert_eq!(prediction.player.view_angles.0.map(f32::to_bits), view);
            assert_eq!(body_bits(prediction.player.body), body);
            assert_eq!(prediction.player.movement, movement);
        }
    }
}

#[test]
fn q2_angle_only_and_exhausted_history_drift_keep_the_raw_angle_admission() {
    for rules in [RuleSetId::Quake2, RuleSetId::Quake2Rerelease] {
        for exhausted in [false, true] {
            let mut prediction = teleport_prediction(
                rules,
                PredictionPolicy {
                    position: exhausted,
                    angles: true,
                },
            );
            let command = teleport_command(rules);
            let body = body_bits(prediction.player.body);
            let movement = prediction.player.movement;
            let mut history = CommandHistory::load(1, 0).unwrap();
            if exhausted {
                prediction.remember_snapshot(Some(Acknowledgement::Number(0)));
                history.append(command);
                history.append(UserCmd {
                    server_time_ms: 32,
                    ..command
                });
                assert_eq!(prediction.replay(&history, &mut NoQueries).unwrap(), 0);
                assert_eq!(prediction.history_resyncs, 1);
            } else {
                prediction.advance(command, &mut NoQueries).unwrap();
            }
            assert_eq!(prediction.player.view_angles.0[0], 47.25);
            let accepted = prediction.player.view_angles;

            prediction.drift_pitch(command, 40.0, 100.5);

            assert_eq!(
                prediction.player.view_angles.0.map(f32::to_bits),
                [107.75, accepted.0[1], accepted.0[2]].map(f32::to_bits),
                "{rules:?} exhausted={exhausted}"
            );
            assert_eq!(body_bits(prediction.player.body), body);
            assert_eq!(prediction.player.movement, movement);
            if exhausted {
                // A later frame stays on the raw-angle fallback until a new
                // authoritative snapshot provides an admitted history window.
                assert_eq!(prediction.replay(&history, &mut NoQueries).unwrap(), 0);
                prediction.drift_pitch(command, 40.0, 100.5);
                assert_eq!(prediction.player.view_angles.0[0], 107.75);
                assert_eq!(prediction.history_resyncs, 1);
            }
        }
    }
}

#[test]
fn q2_disabled_angles_are_not_changed_by_drift_after_advance_or_replay() {
    for rules in [RuleSetId::Quake2, RuleSetId::Quake2Rerelease] {
        for position in [false, true] {
            for replay in [false, true] {
                let mut prediction = teleport_prediction(
                    rules,
                    PredictionPolicy {
                        position,
                        angles: false,
                    },
                );
                let command = teleport_command(rules);
                let view = prediction.player.view_angles.0.map(f32::to_bits);
                let delta = prediction.player.movement.delta_angles;
                if replay {
                    prediction.remember_snapshot(Some(Acknowledgement::Number(0)));
                    let mut history = CommandHistory::load(2, 0).unwrap();
                    history.append(command);
                    assert_eq!(prediction.replay(&history, &mut FreeSpace).unwrap(), 1);
                } else {
                    prediction.advance(command, &mut FreeSpace).unwrap();
                }
                let body = body_bits(prediction.player.body);
                let movement = prediction.player.movement;

                prediction.drift_pitch(command, 40.0, 100.5);

                assert_eq!(prediction.player.view_angles.0.map(f32::to_bits), view);
                assert_eq!(prediction.player.movement.delta_angles, delta);
                assert_eq!(body_bits(prediction.player.body), body);
                assert_eq!(prediction.player.movement, movement);
            }
        }
    }
}
