mod support;

use qa_core::{
    math,
    primitives::{
        AngleOffset, AngleOffsets, Body, MovementMode, MovementTimer, PlayerState,
        PredictionPolicy, RuleSetId, UserCmd, Vec3,
    },
};
use qa_movement::{MoveOptions, TraceServices, no_boundary, pmove, predict_view_angles};
use qa_world::collision::{Contents, EntityTracePolicy, Trace, TraceError, TraceQuery};

fn body_bits(body: Body) -> [[u32; 3]; 4] {
    [body.position, body.velocity, body.mins, body.maxs].map(|row| row.0.map(f32::to_bits))
}

fn player(rules: RuleSetId, mode: MovementMode) -> PlayerState {
    let mut player = PlayerState {
        movement_rules: rules,
        trace_rules: RuleSetId::QuakeWorld,
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
        let view_offset = state.view_offset;
        let command = UserCmd {
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
            state.view_offset.0.map(f32::to_bits),
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
            if mode != MovementMode::Frozen {
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
