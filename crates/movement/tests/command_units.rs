mod support;

use qa_core::primitives::{MovementMode, PlayerState, RuleSetId, ThinkTime, UserCmd, Vec3};
use qa_gameplay::rules::command_units;
use qa_movement::{MoveOptions, MovePhase, MovementResult, entry, no_boundary, set_bounds};

fn run(rules: RuleSetId, command: UserCmd, options: MoveOptions) -> (PlayerState, MovementResult) {
    let mut player = PlayerState {
        movement_rules: rules,
        trace_rules: RuleSetId::Quake2,
        health: 100,
        ..Default::default()
    };
    set_bounds(&mut player);
    player.movement.mode = MovementMode::Noclip;
    player.body.position = Vec3([0.0, 0.0, 64.0]);
    let mut boundary = no_boundary;
    // Cached provider entries must consume units too, without the public pmove wrapper.
    let result = entry(rules)(
        command,
        &mut player,
        &mut support::FixtureWorld::default(),
        options,
        &mut boundary,
    )
    .unwrap();
    (player, result)
}

fn assert_same_motion(actual: &PlayerState, expected: &PlayerState) {
    assert_eq!(
        actual.body.position.0.map(f32::to_bits),
        expected.body.position.0.map(f32::to_bits)
    );
    assert_eq!(
        actual.body.velocity.0.map(f32::to_bits),
        expected.body.velocity.0.map(f32::to_bits)
    );
    assert_eq!(
        actual.view_angles.0.map(f32::to_bits),
        expected.view_angles.0.map(f32::to_bits)
    );
    assert_eq!(actual.movement, expected.movement);
    assert_eq!(actual.trace_rules, RuleSetId::Quake2);
}

#[test]
fn complete_and_provider_integrate_convert_to_the_independent_movement_units_once() {
    for (input, movement, axes, native_axes) in [
        (
            RuleSetId::Quake3,
            RuleSetId::Quake,
            [127.0, 0.0, 0.0],
            [320.0, 0.0, 0.0],
        ),
        (
            RuleSetId::Quake3,
            RuleSetId::QuakeWorld,
            [127.0, 0.0, 0.0],
            [320.0, 0.0, 0.0],
        ),
        (
            RuleSetId::Quake3,
            RuleSetId::Quake2,
            [127.0, 0.0, 0.0],
            [200.0, 0.0, 0.0],
        ),
        (
            RuleSetId::Quake3,
            RuleSetId::Quake2Rerelease,
            [127.0, 0.0, 0.0],
            [200.0, 0.0, 0.0],
        ),
        (
            RuleSetId::Quake2,
            RuleSetId::Quake3,
            [200.0, 0.0, 0.0],
            [127.0, 0.0, 0.0],
        ),
        (
            RuleSetId::Quake,
            RuleSetId::Quake3,
            [200.0, 0.0, 0.0],
            [79.0, 0.0, 0.0],
        ),
        (
            RuleSetId::Quake,
            RuleSetId::Quake2,
            [200.25, -5.5, 1.25],
            [200.25, -5.5, 1.25],
        ),
    ] {
        for phase in [MovePhase::Complete, MovePhase::Integrate { seconds: 0.016 }] {
            let command = UserCmd {
                units: command_units(input),
                movement: axes,
                duration_ms: 16,
                duration_ns: 16_000_000,
                server_time_ms: 16,
                ..Default::default()
            };
            let native = UserCmd {
                units: command_units(movement),
                movement: native_axes,
                ..command
            };
            let options = MoveOptions {
                phase,
                ..Default::default()
            };
            let (actual, result) = run(movement, command, options);
            let (expected, native_result) = run(movement, native, options);
            assert_same_motion(&actual, &expected);
            assert_eq!(
                result.steps, native_result.steps,
                "{input:?}→{movement:?} {phase:?}"
            );
            assert_eq!(command.units, command_units(input));
            assert_eq!(command.movement.map(f32::to_bits), axes.map(f32::to_bits));
        }
    }
}

#[test]
fn native_control_converts_before_the_shared_raw_wish_and_scalar_motion_boundary() {
    let command = UserCmd {
        units: command_units(RuleSetId::Quake3),
        movement: [127.0, 0.0, 0.0],
        ..Default::default()
    };
    let options = MoveOptions {
        phase: MovePhase::Control {
            body_angles: Vec3::default(),
            clock: ThinkTime::Seconds(7.0),
        },
        ..Default::default()
    };
    let (actual, result) = run(RuleSetId::Quake, command, options);
    assert_eq!(
        actual.body.velocity.0.map(f32::to_bits),
        [320.0f32, 0.0, 0.0].map(f32::to_bits)
    );
    assert_eq!(actual.body.position, Vec3([0.0, 0.0, 64.0]));
    assert_eq!(result.steps, 0);
}

#[test]
fn simulation_duration_admission_does_not_convert_movement_units() {
    let command = UserCmd {
        units: command_units(RuleSetId::Quake2),
        movement: [200.0, -0.0, 0.0],
        duration_ms: 255,
        ..Default::default()
    };
    let admitted = qa_movement::prepare_command(RuleSetId::Quake3, command);
    assert_eq!(admitted.duration_ms, 200);
    assert_eq!(admitted.units, command.units);
    assert_eq!(
        admitted.movement.map(f32::to_bits),
        command.movement.map(f32::to_bits)
    );
}
