mod support;
use qa_core::primitives::{MovementRules, PlayerState, PlayerTail, UserCmd, Vec3};
use qa_movement::{pmove, set_bounds};
fn player(rules: MovementRules) -> PlayerState {
    let mut state = PlayerState {
        movement_rules: rules,
        tail: PlayerTail::Q2 { weapon_frame: 17 },
        ..Default::default()
    };
    set_bounds(&mut state);
    state.body.position = Vec3([0.0, 0.0, 24.0]);
    state.movement.grounded = true;
    state
}
#[test]
fn native_command_scheduling_preserves_qw_odd_halves_and_q3_absolute_time() {
    let mut world = support::FixtureWorld::default();
    let mut qw = player(MovementRules::QuakeWorld);
    let result = pmove(
        UserCmd {
            duration_ms: 101,
            movement: [320, 0, 0],
            ..Default::default()
        },
        &mut qw,
        &mut world,
    );
    let mut expected = player(MovementRules::QuakeWorld);
    for _ in 0..2 {
        pmove(
            UserCmd {
                duration_ms: 50,
                movement: [320, 0, 0],
                ..Default::default()
            },
            &mut expected,
            &mut world,
        );
    }
    assert_eq!(result.steps, 2);
    assert_eq!(qw.body.position, expected.body.position);
    assert_eq!(qw.body.velocity, expected.body.velocity);
    let mut q3 = player(MovementRules::Quake3);
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
    let mut state = player(MovementRules::Quake);
    let mut world = support::FixtureWorld::default();
    let command = UserCmd {
        duration_ms: 11,
        duration_ns: 11_764_705,
        movement: [320, 0, 200],
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
    let mut classic = player(MovementRules::Quake2);
    let mut rr = player(MovementRules::Quake2Rerelease);
    classic.body.position.0[0] = 0.06;
    rr.body.position.0[0] = 0.06;
    let command = UserCmd {
        duration_ms: 16,
        movement: [300, 0, 0],
        ..Default::default()
    };
    pmove(command, &mut classic, &mut world);
    pmove(command, &mut rr, &mut world);
    assert_eq!(classic.body.position.0[0], 0.75);
    assert!((rr.body.position.0[0] - 0.828).abs() < 0.000001);
    assert_eq!(classic.body.mins.0[2], rr.body.mins.0[2]);
}
