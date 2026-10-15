mod support;

use qa_core::primitives::{
    MovementMagnitude, MovementMode, MovementTuning, PlayerState, RuleSetId, ThinkTime, UserCmd,
    Vec3,
};
use qa_movement::{MoveOptions, MovePhase, effective_tuning, pmove};

#[test]
fn q2_air_acceleration_selects_its_native_coefficient_and_zero_branch() {
    for (rules, air, accelerate, duration_ms, wish, expected) in [
        (RuleSetId::Quake2, 1.0, 10.0, 100, 150.0, 30.0f32),
        (RuleSetId::Quake2, 1.0, 0.5, 100, 150.0, 7.5),
        (RuleSetId::Quake2Rerelease, 1.0, 10.0, 100, 150.0, 15.0),
        (RuleSetId::Quake2Rerelease, 0.5, 10.0, 100, 150.0, 7.5),
        (RuleSetId::Quake2Rerelease, 1.0, 0.5, 100, 150.0, 15.0),
        (RuleSetId::Quake2, 0.0, 10.0, 200, 300.0, 60.0),
        (RuleSetId::Quake2Rerelease, 0.0, 10.0, 200, 300.0, 60.0),
    ] {
        let mut state = PlayerState {
            movement_rules: rules,
            trace_rules: RuleSetId::Quake3,
            health: 100,
            ..Default::default()
        };
        qa_movement::set_bounds(&mut state);
        state.body.position = Vec3([0.0, 0.0, 64.0]);
        state.movement.tuning.gravity = Some(MovementMagnitude::Base(0.0));
        state.movement.tuning.no_step = true;
        state.movement.tuning.accelerate = Some(accelerate);
        state.movement.tuning.air_accelerate = Some(air);
        pmove(
            UserCmd {
                units: qa_gameplay::rules::command_units(rules),
                movement: [wish, 0.0, 0.0],
                duration_ms,
                server_time_ms: i32::from(duration_ms),
                ..Default::default()
            },
            &mut state,
            &mut support::FixtureWorld::default(),
            MoveOptions::default(),
            qa_movement::no_boundary,
        )
        .expect("air movement over a clear floor");
        assert_eq!(
            state.body.velocity.0[0].to_bits(),
            expected.to_bits(),
            "{rules:?} air={air} accelerate={accelerate} duration_ms={duration_ms} wish={wish}",
        );
    }
}

#[test]
fn effective_limits_share_selected_defaults_and_existing_multiplier_order() {
    for (rules, speed, spectator) in [
        (RuleSetId::Quake, 320.0, 320.0),
        (RuleSetId::QuakeWorld, 320.0, 500.0),
        (RuleSetId::Quake2, 300.0, 300.0),
        (RuleSetId::Quake2Rerelease, 300.0, 300.0),
        (RuleSetId::Quake3, 320.0, 320.0),
    ] {
        let limits = effective_tuning(rules, MovementTuning::default());
        assert_eq!(limits.gravity.to_bits(), 800.0f32.to_bits());
        assert_eq!(limits.speed.to_bits(), (speed as f32).to_bits());
        assert_eq!(
            limits.spectator_speed.to_bits(),
            (spectator as f32).to_bits()
        );
        let tuning = MovementTuning {
            gravity: Some(MovementMagnitude::Base(137.125)),
            max_speed: Some(MovementMagnitude::Base(416.25)),
            spectator_max_speed: Some(612.5),
            gravity_multiplier: 0.3,
            speed_multiplier: 1.75,
            ..Default::default()
        };
        let limits = effective_tuning(rules, tuning);
        assert_eq!(limits.gravity.to_bits(), (137.125f32 * 0.3).to_bits());
        assert_eq!(limits.speed.to_bits(), (416.25f32 * 1.75).to_bits());
        assert_eq!(
            limits.spectator_speed.to_bits(),
            (612.5f32 * 1.75).to_bits()
        );
    }
}

#[test]
fn native_zero_multiplier_limits_reach_control_and_ordinary_gravity() {
    let mut state = PlayerState {
        movement_rules: RuleSetId::Quake,
        trace_rules: RuleSetId::Quake3,
        health: 100,
        ..Default::default()
    };
    state.movement.mode = MovementMode::Noclip;
    state.movement.tuning.speed_multiplier = 0.0;
    state.movement.tuning.max_speed = Some(MovementMagnitude::native(200.0, 0.0));
    pmove(
        UserCmd {
            units: qa_gameplay::rules::command_units(RuleSetId::Quake),
            movement: [400.0, 0.0, 0.0],
            ..Default::default()
        },
        &mut state,
        &mut support::FixtureWorld::default(),
        MoveOptions {
            phase: MovePhase::Control {
                body_angles: Vec3::default(),
                clock: ThinkTime::Seconds(0.0),
            },
            ..Default::default()
        },
        qa_movement::no_boundary,
    )
    .expect("native control");
    assert_eq!(state.body.velocity, Vec3([200.0, 0.0, 0.0]));
    assert_eq!(state.body.position, Vec3::default());

    for (gravity, expected) in [
        (MovementMagnitude::Base(800.0), 0.0f32),
        (MovementMagnitude::native(800.0, 0.0), -13.0),
    ] {
        let mut state = PlayerState {
            movement_rules: RuleSetId::Quake3,
            trace_rules: RuleSetId::Quake2,
            health: 100,
            ..Default::default()
        };
        qa_movement::set_bounds(&mut state);
        state.body.position = Vec3([0.0, 0.0, 64.0]);
        state.movement.tuning.gravity_multiplier = 0.0;
        state.movement.tuning.gravity = Some(gravity);
        pmove(
            UserCmd {
                server_time_ms: 16,
                ..Default::default()
            },
            &mut state,
            &mut support::FixtureWorld::default(),
            Default::default(),
            qa_movement::no_boundary,
        )
        .expect("ordinary native gravity");
        assert_eq!(state.body.velocity.0[2].to_bits(), expected.to_bits());
        assert_eq!(state.trace_rules, RuleSetId::Quake2);
    }
}
