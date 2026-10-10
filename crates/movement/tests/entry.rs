mod support;
use qa_core::primitives::{PlayerState, PlayerTail, RuleSetId, UserCmd, Vec3};
use qa_movement::{pmove, set_bounds};
fn player(rules: RuleSetId) -> PlayerState {
    let mut state = PlayerState {
        movement_rules: rules,
        trace_rules: rules,
        tail: PlayerTail::Q2 { weapon_frame: 17 },
        ..Default::default()
    };
    set_bounds(&mut state);
    state.body.position = Vec3([0.0, 0.0, 24.0]);
    state.movement.grounded = true;
    state
}
#[test]
fn q3_view_angles_use_native_signed_shorts_and_pitch_delta_clamp() {
    use qa_core::primitives::MovementMode;
    let mut world = support::FixtureWorld::default();
    for (pitch, yaw, roll, expected, delta) in [
        (0., 180., 270., [0., -180., -90.], 0.),
        (0., 450., -450., [0., 90., -90.], 0.),
        (90., 0., 0., [16000. * (360. / 65536.), 0., 0.], -384.),
        (-90., 0., 0., [-16000. * (360. / 65536.), 0., 0.], -65152.),
    ] {
        let mut state = player(RuleSetId::Quake3);
        state.movement.mode = MovementMode::Frozen;
        let command = UserCmd {
            server_time_ms: 16,
            view_angles: Vec3([pitch, yaw, roll]),
            ..Default::default()
        };
        pmove(command, &mut state, &mut world);
        assert_eq!(state.view_angles, Vec3(expected));
        assert_eq!(state.movement.delta_angles.0[0], delta * (360. / 65536.));
        // Native clamping adjusts delta_angles; the same command remains stable.
        pmove(
            UserCmd {
                server_time_ms: 32,
                ..command
            },
            &mut state,
            &mut world,
        );
        assert_eq!(state.view_angles, Vec3(expected));
    }
}
#[test]
fn native_command_scheduling_preserves_qw_odd_halves_and_q3_absolute_time() {
    let mut world = support::FixtureWorld::default();
    let mut qw = player(RuleSetId::QuakeWorld);
    let result = pmove(
        UserCmd {
            duration_ms: 101,
            movement: [320.0, 0.0, 0.0],
            ..Default::default()
        },
        &mut qw,
        &mut world,
    );
    let mut expected = player(RuleSetId::QuakeWorld);
    for _ in 0..2 {
        pmove(
            UserCmd {
                duration_ms: 50,
                movement: [320.0, 0.0, 0.0],
                ..Default::default()
            },
            &mut expected,
            &mut world,
        );
    }
    assert_eq!(result.steps, 2);
    assert_eq!(qw.body.position, expected.body.position);
    assert_eq!(qw.body.velocity, expected.body.velocity);
    let mut q3 = player(RuleSetId::Quake3);
    assert_eq!(
        pmove(
            UserCmd {
                server_time_ms: 200,
                ..Default::default()
            },
            &mut q3,
            &mut world
        )
        .steps,
        4
    );
    assert_eq!(q3.movement.command_time_ms, 200);
    assert_eq!(
        pmove(
            UserCmd {
                server_time_ms: 199,
                ..Default::default()
            },
            &mut q3,
            &mut world
        )
        .steps,
        0
    );
    assert_eq!(
        pmove(
            UserCmd {
                server_time_ms: 2000,
                ..Default::default()
            },
            &mut q3,
            &mut world
        )
        .steps,
        16
    );
}
#[test]
fn netquake_uses_precise_duration_and_module_owns_jump() {
    let mut state = player(RuleSetId::Quake);
    let mut world = support::FixtureWorld::default();
    let command = UserCmd {
        duration_ms: 11,
        duration_ns: 11_764_705,
        movement: [320.0, 0.0, 200.0],
        ..Default::default()
    };
    pmove(command, &mut state, &mut world);
    let speed = (10.0f64 * 0.011764705 * 320.0) as f32;
    assert_eq!(state.body.velocity.0[0], speed);
    assert_eq!(state.body.position.0[0], speed * 0.011764705f32);
    assert_eq!(state.body.position.0[2], 24.0);
    assert_eq!(state.tail, PlayerTail::Q2 { weapon_frame: 17 });
}
#[test]
fn classic_quantizes_while_rerelease_keeps_floating_state() {
    let mut world = support::FixtureWorld::default();
    let mut classic = player(RuleSetId::Quake2);
    let mut rr = player(RuleSetId::Quake2Rerelease);
    classic.body.position.0[0] = 0.06;
    rr.body.position.0[0] = 0.06;
    let command = UserCmd {
        duration_ms: 16,
        movement: [300.0, 0.0, 0.0],
        ..Default::default()
    };
    pmove(command, &mut classic, &mut world);
    pmove(command, &mut rr, &mut world);
    assert_eq!(classic.body.position.0[0], 0.75);
    assert!((rr.body.position.0[0] - 0.828).abs() < 0.000001);
    assert_eq!(classic.body.mins.0[2], rr.body.mins.0[2]);
}
