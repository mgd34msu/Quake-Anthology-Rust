mod support;

use qa_core::{
    math,
    primitives::{
        AngleOffset, AngleOffsets, Body, MovementMode, MovementTimer, PlayerState,
        PredictionPolicy, RuleSetId, UserCmd, Vec3,
    },
};
use qa_movement::{
    MoveOptions, TraceServices, no_boundary, pmove, predict_pitch_drift, predict_view_angles,
};
use qa_world::collision::{Contents, EntityTracePolicy, Trace, TraceError, TraceQuery};

fn body_bits(body: Body) -> [[u32; 3]; 4] {
    [body.position, body.velocity, body.mins, body.maxs].map(|row| row.0.map(f32::to_bits))
}

fn player(rules: RuleSetId, mode: MovementMode) -> PlayerState {
    let mut player = PlayerState {
        movement_rules: rules,
        trace_rules: RuleSetId::QuakeWorld,
        health: 100,
        prediction: PredictionPolicy {
            position: true,
            angles: false,
        },
        view_angles: Vec3([0.0, 90.0, -0.0]),
        ..Default::default()
    };
    qa_movement::set_bounds(&mut player);
    player.movement.mode = mode;
    player.body.position = Vec3([0.13, 0.17, 64.13]);
    player
}

fn angle_offset_bits(offsets: AngleOffsets) -> [(u8, u32); 3] {
    offsets.0.map(|offset| match offset {
        AngleOffset::Degrees(value) => (0, value.to_bits()),
        AngleOffset::Units(value) => (1, value as u32),
    })
}

#[test]
fn pitch_drift_retains_dead_q3_view_and_delta_words() {
    for health in [0, -1] {
        for mode in [
            MovementMode::Walk,
            MovementMode::Noclip,
            MovementMode::Frozen,
        ] {
            let mut state = player(RuleSetId::Quake3, mode);
            state.health = health;
            state.view_angles = Vec3([-0.0, 133.25, -17.125]);
            state.movement.delta_angles = AngleOffsets([
                AngleOffset::Units(32768),
                AngleOffset::Units(i32::MAX - 1),
                AngleOffset::Degrees(-0.0),
            ]);
            let view = state.view_angles.0.map(f32::to_bits);
            let offsets = angle_offset_bits(state.movement.delta_angles);
            let command = UserCmd {
                view_angles: Vec3([40.0, 5.0, 2.0]),
                ..Default::default()
            };
            let admission = predict_view_angles(command, &mut state);
            predict_pitch_drift(command, &mut state, 40.0, 32.0, admission);
            assert_eq!(state.view_angles.0.map(f32::to_bits), view);
            assert_eq!(angle_offset_bits(state.movement.delta_angles), offsets);
        }
    }
}

#[test]
fn q3_pitch_drift_uses_native_short_projection_and_keeps_accepted_yaw_roll() {
    for (health, mode) in [(100, MovementMode::Walk), (0, MovementMode::Spectator)] {
        for pitch in [32.987f32, 90.0] {
            let mut state = player(RuleSetId::Quake3, mode);
            state.health = health;
            state.view_angles = Vec3([40.0, 133.25, -0.0]);
            state.movement.delta_angles = AngleOffsets([
                AngleOffset::Units(123),
                AngleOffset::Units(i32::MAX - 1),
                AngleOffset::Units(i32::MIN + 1),
            ]);
            let admitted_command = UserCmd {
                view_angles: Vec3([40.0, 133.25, -90.0]),
                ..Default::default()
            };
            let admission = predict_view_angles(admitted_command, &mut state);
            let command = UserCmd {
                view_angles: Vec3([40.0, 5.0, 2.0]),
                ..admitted_command
            };
            let accepted = state.view_angles;
            let body = body_bits(state.body);
            let mut expected = player(RuleSetId::Quake3, mode);
            expected.health = health;
            expected.movement.delta_angles = state.movement.delta_angles;
            predict_view_angles(
                UserCmd {
                    view_angles: Vec3([pitch, 5.0, 2.0]),
                    ..command
                },
                &mut expected,
            );

            predict_pitch_drift(command, &mut state, 40.0, pitch, admission);

            assert_eq!(
                state.view_angles.0[0].to_bits(),
                expected.view_angles.0[0].to_bits()
            );
            assert_eq!(state.view_angles.0[1].to_bits(), accepted.0[1].to_bits());
            assert_eq!(state.view_angles.0[2].to_bits(), accepted.0[2].to_bits());
            assert_eq!(
                angle_offset_bits(state.movement.delta_angles),
                angle_offset_bits(expected.movement.delta_angles)
            );
            assert_eq!(body_bits(state.body), body);
            if pitch == 90.0 {
                assert_eq!(state.movement.delta_angles.0[0], AngleOffset::Units(-384));
            } else {
                assert_ne!(
                    state.view_angles.0[0].to_bits(),
                    (pitch + math::short_to_angle(123)).to_bits()
                );
            }
        }
    }
}

#[test]
fn intermission_pitch_drift_uses_the_same_view_lock_for_every_movement_role() {
    for rules in RuleSetId::ALL {
        for mode in [
            MovementMode::Intermission,
            MovementMode::SinglePlayerIntermission,
        ] {
            let mut state = player(rules, mode);
            let view = state.view_angles.0.map(f32::to_bits);
            let offsets = angle_offset_bits(state.movement.delta_angles);
            let admission = predict_view_angles(UserCmd::default(), &mut state);
            predict_pitch_drift(UserCmd::default(), &mut state, 40.0, 32.0, admission);
            assert_eq!(state.view_angles.0.map(f32::to_bits), view);
            assert_eq!(angle_offset_bits(state.movement.delta_angles), offsets);
        }
    }
}

#[test]
fn raw_native_pitch_drift_assigns_the_exact_new_sample_including_signed_zero() {
    for rules in [
        RuleSetId::Quake,
        RuleSetId::QuakeWorld,
        RuleSetId::Quake2,
        RuleSetId::Quake2Rerelease,
    ] {
        for (previous, pitch) in [(1.0, f32::from_bits(1)), (1.0, -0.0), (-0.0, 0.0)] {
            let mut state = player(rules, MovementMode::Walk);
            state.view_angles = Vec3([previous, 133.25, -0.0]);
            state.movement.delta_angles = AngleOffsets([AngleOffset::Degrees(-0.0); 3]);
            let command = UserCmd {
                view_angles: state.view_angles,
                ..Default::default()
            };
            let admission = predict_view_angles(command, &mut state);
            let accepted = state.view_angles;
            predict_pitch_drift(command, &mut state, previous, pitch, admission);
            assert_eq!(
                state.view_angles.0.map(f32::to_bits),
                [pitch, accepted.0[1], accepted.0[2]].map(f32::to_bits)
            );
        }
    }
}

#[test]
fn bit_identical_raw_pitch_does_not_reproject_or_reclamp_accepted_q3_view() {
    for raw in [-0.0, 40.0, f32::from_bits(0x7fc0_1234)] {
        let mut state = player(RuleSetId::Quake3, MovementMode::Walk);
        let admission = predict_view_angles(UserCmd::default(), &mut state);
        state.view_angles = Vec3([f32::from_bits(0x7fc0_5678), -0.0, 133.25]);
        state.movement.delta_angles = AngleOffsets([
            AngleOffset::Units(i32::MAX - 1),
            AngleOffset::Units(i32::MIN + 1),
            AngleOffset::Degrees(-0.0),
        ]);
        let view = state.view_angles.0.map(f32::to_bits);
        let offsets = angle_offset_bits(state.movement.delta_angles);
        predict_pitch_drift(UserCmd::default(), &mut state, raw, raw, admission);
        assert_eq!(state.view_angles.0.map(f32::to_bits), view);
        assert_eq!(angle_offset_bits(state.movement.delta_angles), offsets);
    }
}

#[test]
fn actual_movement_admission_retains_a_teleport_pitch_lock_after_its_timer_expires() {
    for rules in [RuleSetId::Quake2, RuleSetId::Quake2Rerelease] {
        let mut state = player(rules, MovementMode::Walk);
        state.movement.timer = MovementTimer::TELEPORT;
        state.movement.remaining_ms = 1;
        let command = UserCmd {
            units: qa_gameplay::rules::command_units(rules),
            duration_ms: 16,
            server_time_ms: 16,
            view_angles: Vec3([40.0, 133.25, -90.0]),
            ..Default::default()
        };
        let result = pmove(
            command,
            &mut state,
            &mut support::FixtureWorld::default(),
            MoveOptions::default(),
            no_boundary,
        )
        .unwrap();
        assert!(!state.movement.timer.contains(MovementTimer::TELEPORT));
        assert_eq!(state.view_angles.0[0].to_bits(), 0.0f32.to_bits());
        let accepted = state.view_angles.0.map(f32::to_bits);
        let offsets = angle_offset_bits(state.movement.delta_angles);

        predict_pitch_drift(command, &mut state, 40.0, 32.0, result.view_angles);

        assert_eq!(state.view_angles.0.map(f32::to_bits), accepted, "{rules:?}");
        assert_eq!(angle_offset_bits(state.movement.delta_angles), offsets);
    }
}

#[test]
fn angle_only_admission_keeps_the_native_raw_pitch_policy_during_teleport() {
    for rules in [RuleSetId::Quake2, RuleSetId::Quake2Rerelease] {
        let mut state = player(rules, MovementMode::Walk);
        state.movement.timer = MovementTimer::TELEPORT;
        state.movement.remaining_ms = 1;
        let command = UserCmd {
            view_angles: Vec3([40.0, 133.25, -90.0]),
            ..Default::default()
        };
        let admission = predict_view_angles(command, &mut state);
        let accepted = state.view_angles;

        predict_pitch_drift(command, &mut state, 40.0, 32.0, admission);

        assert_eq!(state.view_angles.0[0].to_bits(), 32.0f32.to_bits());
        assert_eq!(state.view_angles.0[1].to_bits(), accepted.0[1].to_bits());
        assert_eq!(state.view_angles.0[2].to_bits(), accepted.0[2].to_bits());
        assert!(state.movement.timer.contains(MovementTimer::TELEPORT));
        assert_eq!(state.movement.remaining_ms, 1);
    }
}

#[test]
fn intermission_locks_controls_independently_of_movement_and_trace_roles() {
    for rules in RuleSetId::ALL {
        for mode in [
            MovementMode::Frozen,
            MovementMode::Intermission,
            MovementMode::SinglePlayerIntermission,
        ] {
            for angle_only in [false, true] {
                let mut state = player(rules, mode);
                state.body.position = Vec3([17.0, -8.0, 64.0]);
                state.body.velocity = Vec3([12.0, -4.0, 8.0]);
                state.view_angles = Vec3([-0.0, 13.75, 97.25]);
                state.movement.delta_angles = AngleOffsets([
                    AngleOffset::Degrees(-0.0),
                    AngleOffset::Units(i32::MAX - 1),
                    AngleOffset::Units(i32::MIN + 1),
                ]);
                state.movement.timer = MovementTimer::TELEPORT;
                state.movement.remaining_ms = 83;
                let body = body_bits(state.body);
                let view = state.view_angles.0.map(f32::to_bits);
                let deltas = angle_offset_bits(state.movement.delta_angles);
                let command = UserCmd {
                    units: qa_gameplay::rules::command_units(rules),
                    server_time_ms: 150,
                    duration_ms: 150,
                    duration_ns: 150_000_000,
                    movement: [127.0, 64.0, 32.0],
                    view_angles: Vec3([90.0, 180.0, 270.0]),
                    ..Default::default()
                };
                if angle_only {
                    predict_view_angles(command, &mut state);
                } else {
                    pmove(
                        command,
                        &mut state,
                        &mut RejectTrace,
                        MoveOptions::default(),
                        no_boundary,
                    )
                    .unwrap();
                    // Q3 advances commandTime even when the step stops before
                    // movement, as PmoveSingle does before its mode checks.
                    if rules == RuleSetId::Quake3 {
                        assert_eq!(state.movement.command_time_ms, 150);
                    }
                }
                assert_eq!(body_bits(state.body), body, "{rules:?} {mode:?}");
                if mode == MovementMode::Frozen {
                    assert_ne!(state.view_angles.0.map(f32::to_bits), view);
                } else {
                    assert_eq!(state.view_angles.0.map(f32::to_bits), view);
                    assert_eq!(angle_offset_bits(state.movement.delta_angles), deltas);
                }
                assert_eq!(state.movement.timer, MovementTimer::TELEPORT);
                assert_eq!(state.movement.remaining_ms, 83);
                assert_eq!(state.movement_rules, rules);
                assert_eq!(state.trace_rules, RuleSetId::QuakeWorld);
            }
        }
    }
}

#[test]
fn q3_nonpositive_health_locks_view_and_all_delta_words_in_movement_and_angle_only() {
    // PM_UpdateViewAngles returns before writing either viewangles or deltas
    // when the player is not a spectator and STAT_HEALTH is nonpositive.
    for health in [0, -1, i32::MIN] {
        for mode in [
            MovementMode::Walk,
            MovementMode::Fly,
            MovementMode::Noclip,
            MovementMode::Dead,
            MovementMode::Gib,
            MovementMode::Frozen,
            MovementMode::Intermission,
            MovementMode::SinglePlayerIntermission,
        ] {
            for angle_only in [false, true] {
                let mut state = player(RuleSetId::Quake3, mode);
                state.health = health;
                state.view_angles = Vec3([-0.0, 13.75, 97.25]);
                state.movement.delta_angles = AngleOffsets([
                    AngleOffset::Degrees(-0.0),
                    AngleOffset::Units(i32::MAX - 1),
                    AngleOffset::Units(i32::MIN + 1),
                ]);
                let view = state.view_angles.0.map(f32::to_bits);
                let deltas = angle_offset_bits(state.movement.delta_angles);
                let cmd = UserCmd {
                    server_time_ms: 150,
                    duration_ms: 150,
                    view_angles: Vec3([90.0, 180.0, 270.0]),
                    ..Default::default()
                };
                if angle_only {
                    predict_view_angles(cmd, &mut state);
                } else {
                    pmove(
                        cmd,
                        &mut state,
                        &mut support::FixtureWorld::default(),
                        MoveOptions::default(),
                        no_boundary,
                    )
                    .unwrap();
                }
                assert_eq!(
                    state.view_angles.0.map(f32::to_bits),
                    view,
                    "{health} {mode:?} {angle_only}"
                );
                assert_eq!(
                    angle_offset_bits(state.movement.delta_angles),
                    deltas,
                    "{health} {mode:?} {angle_only}"
                );
                assert_eq!(state.trace_rules, RuleSetId::QuakeWorld);
            }
        }
    }
}

#[test]
fn q3_alive_and_dead_spectator_views_keep_native_short_and_pitch_clamp() {
    for (health, mode) in [
        (100, MovementMode::Walk),
        (1, MovementMode::Noclip),
        (0, MovementMode::Spectator),
        (-1, MovementMode::Spectator),
    ] {
        for angle_only in [false, true] {
            let mut state = player(RuleSetId::Quake3, mode);
            state.health = health;
            state.movement.delta_angles = AngleOffsets::default();
            let cmd = UserCmd {
                server_time_ms: 16,
                duration_ms: 16,
                view_angles: Vec3([90.0, 180.0, 270.0]),
                ..Default::default()
            };
            if angle_only {
                predict_view_angles(cmd, &mut state);
            } else {
                pmove(
                    cmd,
                    &mut state,
                    &mut support::FixtureWorld::default(),
                    MoveOptions::default(),
                    no_boundary,
                )
                .unwrap();
            }
            let expected = Vec3([16000.0 * (360.0 / 65536.0), -180.0, -90.0]);
            assert_eq!(
                state.view_angles.0.map(f32::to_bits),
                expected.0.map(f32::to_bits),
                "{health} {mode:?} {angle_only}"
            );
            assert_eq!(state.movement.delta_angles.0[0], AngleOffset::Units(-384));
        }
    }
}

#[test]
fn nonpositive_health_does_not_inherit_q3_angle_lock_under_another_movement_role() {
    for rules in [
        RuleSetId::Quake,
        RuleSetId::QuakeWorld,
        RuleSetId::Quake2,
        RuleSetId::Quake2Rerelease,
    ] {
        let mut state = player(rules, MovementMode::Dead);
        state.health = 0;
        state.movement.delta_angles = AngleOffsets::default();
        let command = UserCmd {
            view_angles: Vec3([12.5, 45.0, -90.0]),
            ..Default::default()
        };
        predict_view_angles(command, &mut state);
        assert_eq!(state.view_angles, command.view_angles);
    }
}

#[test]
fn angle_only_uses_the_classic_unclamped_sum_and_changes_no_other_hot_state() {
    // Original cl_pred.c:216-223 adds raw cl.viewangles to each native
    // SHORT2ANGLE delta, including the no-prediction teleport case.
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld, RuleSetId::Quake2] {
        let mut state = player(rules, MovementMode::Frozen);
        state.prediction = PredictionPolicy {
            position: false,
            angles: true,
        };
        state.body.velocity = Vec3([31.25, -2.5, -0.0]);
        state.movement.command_time_ms = 4000;
        state.movement.grounded = true;
        state.movement.ladder = true;
        state.movement.jump_held = true;
        state.movement.water_level = 2;
        state.movement.water_contents = Contents::WATER.0;
        state.movement.timer = MovementTimer::TELEPORT;
        state.movement.remaining_ms = 123;
        let native = [16384, -32768, 8192];
        state.movement.delta_angles = AngleOffsets(native.map(AngleOffset::Units));
        let body = body_bits(state.body);
        let movement = state.movement;
        let view_offset = state.eye_offset();
        let command = UserCmd {
            units: qa_gameplay::rules::command_units(rules),
            duration_ms: 250,
            server_time_ms: -1,
            movement: [127.0, 64.0, 32.0],
            view_angles: Vec3([100.123, 361.25, -450.5]),
            ..Default::default()
        };
        let expected = Vec3(std::array::from_fn(|i| {
            command.view_angles.0[i] + native[i] as f32 * (360.0 / 65536.0)
        }));

        predict_view_angles(command, &mut state);

        assert_eq!(
            state.view_angles.0.map(f32::to_bits),
            expected.0.map(f32::to_bits)
        );
        assert_eq!(body_bits(state.body), body);
        assert_eq!(state.movement, movement);
        assert_eq!(
            state.eye_offset().0.map(f32::to_bits),
            view_offset.0.map(f32::to_bits)
        );
        assert_eq!(state.movement_rules, rules);
        assert_eq!(state.trace_rules, RuleSetId::QuakeWorld);
        assert_eq!(
            state.prediction,
            PredictionPolicy {
                position: false,
                angles: true
            }
        );
    }
}

#[test]
fn angle_only_retains_float_deltas_and_signed_zero_without_command_narrowing() {
    let mut state = player(RuleSetId::Quake2Rerelease, MovementMode::Frozen);
    state.movement.delta_angles = AngleOffsets([
        AngleOffset::Degrees(-0.0),
        AngleOffset::Degrees(3.125),
        AngleOffset::Units(i32::MAX - 1),
    ]);
    let command = UserCmd {
        view_angles: Vec3([-0.0, 45.123, -90.125]),
        ..Default::default()
    };
    let delta = state.movement.delta_angles;
    let expected = Vec3([
        -0.0,
        45.123 + 3.125,
        -90.125 + math::short_to_angle(i32::MAX - 1),
    ]);

    predict_view_angles(command, &mut state);

    assert_eq!(
        state.view_angles.0.map(f32::to_bits),
        expected.0.map(f32::to_bits)
    );
    assert_eq!(state.movement.delta_angles, delta);
    assert_ne!(state.view_angles.0[1].to_bits(), 48.24609375f32.to_bits());
}

#[test]
fn q3_angle_only_keeps_native_short_wrapping_and_pitch_delta_without_advancing_time() {
    let mut state = player(RuleSetId::Quake3, MovementMode::Noclip);
    state.health = 100;
    state.prediction = PredictionPolicy {
        position: false,
        angles: true,
    };
    state.movement.delta_angles = AngleOffsets([
        AngleOffset::Units(0),
        AngleOffset::Units(i32::MAX - 1),
        AngleOffset::Units(i32::MIN + 1),
    ]);
    state.movement.command_time_ms = 4000;
    state.movement.timer = MovementTimer::TELEPORT;
    state.movement.remaining_ms = 123;
    state.movement.ladder = true;
    state.movement.jump_held = true;
    let body = body_bits(state.body);
    let mut expected_movement = state.movement;
    expected_movement.delta_angles.0[0] = AngleOffset::Units(-384);

    predict_view_angles(
        UserCmd {
            duration_ms: 16,
            server_time_ms: 16,
            view_angles: Vec3([90.0, 90.0, -90.0]),
            ..Default::default()
        },
        &mut state,
    );

    let expected = Vec3([
        16000.0 * (360.0 / 65536.0),
        16382.0 * (360.0 / 65536.0),
        -16383.0 * (360.0 / 65536.0),
    ]);
    assert_eq!(
        state.view_angles.0.map(f32::to_bits),
        expected.0.map(f32::to_bits)
    );
    assert_eq!(state.movement, expected_movement);
    assert_eq!(body_bits(state.body), body);
    assert_eq!(state.trace_rules, RuleSetId::QuakeWorld);
}

#[test]
fn rerelease_angle_only_keeps_teleport_pitch_and_roll_in_the_raw_sum() {
    let mut state = player(RuleSetId::Quake2Rerelease, MovementMode::Frozen);
    state.movement.timer = MovementTimer::TELEPORT;
    state.movement.remaining_ms = 123;
    state.movement.delta_angles = Vec3([12.5, 3.125, 17.25]).into();
    let movement = state.movement;
    let body = body_bits(state.body);

    predict_view_angles(
        UserCmd {
            view_angles: Vec3([100.0, 45.123, 32.5]),
            ..Default::default()
        },
        &mut state,
    );

    assert_eq!(
        state.view_angles.0.map(f32::to_bits),
        Vec3([100.0 + 12.5, 45.123 + 3.125, 32.5 + 17.25])
            .0
            .map(f32::to_bits)
    );
    assert_eq!(state.movement, movement);
    assert_eq!(body_bits(state.body), body);
}

#[test]
fn position_only_movement_uses_the_accepted_view_in_every_movement_mode() {
    assert!(!MoveOptions::default().keep_view_angles);
    for rules in RuleSetId::ALL {
        for mode in [
            MovementMode::Walk,
            MovementMode::Fly,
            MovementMode::Noclip,
            MovementMode::Spectator,
            MovementMode::Frozen,
            MovementMode::Intermission,
            MovementMode::SinglePlayerIntermission,
        ] {
            let mut state = player(rules, mode);
            state.movement.delta_angles = AngleOffsets([
                AngleOffset::Units(32768),
                AngleOffset::Units(i32::MAX - 1),
                AngleOffset::Degrees(-0.0),
            ]);
            let accepted = state.view_angles.0.map(f32::to_bits);
            let delta = state.movement.delta_angles;
            let command = UserCmd {
                units: qa_gameplay::rules::command_units(rules),
                duration_ms: 16,
                server_time_ms: 16,
                movement: [100.0, 0.0, 0.0],
                view_angles: Vec3([100.0, 0.0, 33.0]),
                ..Default::default()
            };

            pmove(
                command,
                &mut state,
                &mut support::FixtureWorld::default(),
                MoveOptions {
                    keep_view_angles: true,
                    ..Default::default()
                },
                no_boundary,
            )
            .unwrap();

            assert_eq!(
                state.view_angles.0.map(f32::to_bits),
                accepted,
                "{rules:?} {mode:?}"
            );
            assert_eq!(state.movement.delta_angles, delta, "{rules:?} {mode:?}");
            if mode.allows_movement() {
                assert!(state.body.velocity.0[1] > 0.0, "{rules:?} {mode:?}");
                assert!(
                    state.body.velocity.0[0].abs() < state.body.velocity.0[1].abs() * 0.01,
                    "{rules:?} {mode:?}: {:?}",
                    state.body.velocity
                );
            }
        }
    }
}

struct RejectTrace;
impl TraceServices for RejectTrace {
    fn trace(&mut self, _: TraceQuery) -> Result<Trace, TraceError> {
        Err(TraceError::Scratch)
    }
    fn point_contents(&self, _: Vec3, _: EntityTracePolicy) -> Result<Contents, TraceError> {
        Err(TraceError::Scratch)
    }
}

#[test]
fn failed_position_only_movement_preserves_accepted_view_and_delta_bits() {
    for rules in RuleSetId::ALL {
        let mut state = player(rules, MovementMode::Walk);
        state.movement.delta_angles = AngleOffsets([
            AngleOffset::Units(i32::MIN + 1),
            AngleOffset::Units(i32::MAX - 1),
            AngleOffset::Degrees(-0.0),
        ]);
        let body = body_bits(state.body);
        let movement = state.movement;
        let view = state.view_angles.0.map(f32::to_bits);
        let result = pmove(
            UserCmd {
                units: qa_gameplay::rules::command_units(rules),
                duration_ms: 16,
                server_time_ms: 16,
                movement: [100.0, 0.0, 0.0],
                view_angles: Vec3([90.0, 180.0, 45.0]),
                ..Default::default()
            },
            &mut state,
            &mut RejectTrace,
            MoveOptions {
                keep_view_angles: true,
                ..Default::default()
            },
            no_boundary,
        );
        assert_eq!(result.unwrap_err(), TraceError::Scratch);
        assert_eq!(body_bits(state.body), body);
        assert_eq!(state.movement, movement);
        assert_eq!(state.view_angles.0.map(f32::to_bits), view);
    }
}
