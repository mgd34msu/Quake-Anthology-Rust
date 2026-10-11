mod support;

use qa_core::{
    math,
    primitives::{
        MovementMode, MovementTimer, PlayerState, PlayerTail, RuleSetId, UserCmd, Vec3, buttons,
    },
};
use qa_movement::{TraceServices, pmove, set_bounds};
use qa_world::collision::{
    Contents, EntityTracePolicy, Trace, TraceQuery, TraceRules, trace_policy,
};
use std::cell::Cell;

struct World {
    fixture: support::FixtureWorld,
    rules: TraceRules,
    entities: EntityTracePolicy,
    traces: u32,
    moving_traces: u32,
    contents: Cell<u32>,
}
impl World {
    fn new(rules: RuleSetId) -> Self {
        let (rules, entities) = trace_policy(rules);
        Self {
            fixture: support::FixtureWorld::default(),
            rules,
            entities,
            traces: 0,
            moving_traces: 0,
            contents: Cell::new(0),
        }
    }
}
impl TraceServices for World {
    fn trace(&mut self, query: TraceQuery) -> Trace {
        assert_eq!(query.rules, self.rules);
        assert_eq!(query.entity_rules, self.entities);
        self.traces += 1;
        self.moving_traces += u32::from(query.start != query.end);
        self.fixture.trace(query)
    }
    fn point_contents(&self, point: Vec3, rules: EntityTracePolicy) -> Contents {
        assert_eq!(rules, self.entities);
        self.contents.set(self.contents.get() + 1);
        self.fixture.point_contents(point, rules)
    }
}
fn player(rules: RuleSetId, mode: MovementMode) -> PlayerState {
    let mut state = PlayerState {
        movement_rules: rules,
        trace_rules: RuleSetId::Quake2,
        tail: PlayerTail::Q2 { weapon_frame: 17 },
        ..Default::default()
    };
    set_bounds(&mut state);
    state.body.position = Vec3([0.0, 0.0, 64.0]);
    state.movement.mode = mode;
    state
}
fn command(ms: u16, movement: [f32; 3], angles: Vec3) -> UserCmd {
    UserCmd {
        duration_ms: ms,
        server_time_ms: i32::from(ms),
        movement,
        view_angles: angles,
        ..Default::default()
    }
}

#[test]
fn q3_spectator_clips_while_noclip_uses_direct_motion_and_foreign_trace_rules() {
    for mode in [MovementMode::Spectator, MovementMode::Noclip] {
        let mut state = player(RuleSetId::Quake3, mode);
        state.body.position.0[0] = 220.0;
        state.body.velocity = Vec3([2000.0, 0.0, 0.0]);
        let mut world = World::new(state.trace_rules);
        let result = pmove(
            command(16, [0.0; 3], Vec3::default()),
            &mut state,
            &mut world,
        );
        if mode == MovementMode::Spectator {
            assert!(state.body.position.0[0] <= 225.0001);
            assert!(world.moving_traces > 0);
        } else {
            assert!(state.body.position.0[0] > 225.0);
            assert_eq!(world.traces, 0);
        }
        assert_eq!(result.traces, world.traces);
        assert_eq!(state.movement_rules, RuleSetId::Quake3);
        assert_eq!(state.trace_rules, RuleSetId::Quake2);
        assert_eq!(state.tail, PlayerTail::Q2 { weapon_frame: 17 });
    }
}

#[test]
fn classic_spectator_snaps_without_rejecting_a_position_beyond_a_wall() {
    let mut state = player(RuleSetId::Quake2, MovementMode::Spectator);
    state.body.position.0[0] = 220.0;
    let mut world = World::new(state.trace_rules);
    pmove(
        command(100, [300.0, 0.0, 0.0], Vec3::default()),
        &mut state,
        &mut world,
    );
    assert_eq!(state.body.position.0[0], 250.0);
    assert_eq!(state.body.velocity.0[0], 300.0);
    assert_eq!(world.traces, 0);
}

#[test]
fn pitched_forward_free_movement_uses_the_full_view_basis() {
    for (rules, mode, forward) in [
        (RuleSetId::Quake3, MovementMode::Spectator, 127.0),
        (RuleSetId::Quake3, MovementMode::Noclip, 127.0),
        (RuleSetId::Quake3, MovementMode::Fly, 127.0),
        (RuleSetId::Quake2, MovementMode::Spectator, 300.0),
        (RuleSetId::QuakeWorld, MovementMode::Spectator, 320.0),
    ] {
        let mut state = player(rules, mode);
        let mut world = World::new(state.trace_rules);
        pmove(
            command(16, [forward, 0.0, 0.0], Vec3([45.0, 0.0, 0.0])),
            &mut state,
            &mut world,
        );
        assert!(state.body.velocity.0[0] > 0.0, "{rules:?} {mode:?}");
        assert!(state.body.velocity.0[2] < 0.0, "{rules:?} {mode:?}");
        assert!(
            (state.body.velocity.0[0] + state.body.velocity.0[2]).abs() < 0.0001,
            "{rules:?} {mode:?}"
        );
        assert!(state.body.position.0[2] < 64.0);
    }
}

#[test]
fn rolled_free_strafe_keeps_the_right_vectors_vertical_component() {
    for mode in [MovementMode::Spectator, MovementMode::Noclip] {
        let mut state = player(RuleSetId::Quake3, mode);
        let mut world = World::new(state.trace_rules);
        pmove(
            command(16, [0.0, 127.0, 0.0], Vec3([0.0, 0.0, 45.0])),
            &mut state,
            &mut world,
        );
        assert!(state.body.velocity.0[1] < 0.0);
        assert!(state.body.velocity.0[2] < 0.0);
        assert!((state.body.velocity.0[1] - state.body.velocity.0[2]).abs() < 0.0001);
    }
}

#[test]
fn q3_free_wish_scaling_keeps_each_native_operation_order() {
    let axes = [127.0f32, 63.0, 37.0];
    let angles = Vec3([45.0, 0.0, 45.0]);
    let basis = math::angle_vectors_radians(math::radians_from_degrees_f32(angles));
    let scale = 320.0 * 127.0 / (127.0 * math::length(Vec3(axes)));
    let dt = (16.0f64 * 0.001) as f32;
    for mode in [MovementMode::Spectator, MovementMode::Noclip] {
        // qsrc bg_pmove.c PM_FlyMove scales each basis before command products;
        // PM_NoclipMove scales the normalized wishspeed instead.
        let mut wish = if mode == MovementMode::Spectator {
            (basis.forward * scale) * axes[0] + (basis.right * scale) * axes[1]
        } else {
            basis.forward * axes[0] + basis.right * axes[1]
        };
        wish.0[2] += if mode == MovementMode::Spectator {
            scale * axes[2]
        } else {
            axes[2]
        };
        let length = math::normalize(&mut wish);
        let speed = if mode == MovementMode::Spectator {
            length
        } else {
            length * scale
        };
        let acceleration = if mode == MovementMode::Spectator {
            8.0
        } else {
            10.0
        };
        let expected = wish * (acceleration * dt * speed).min(speed);
        let mut state = player(RuleSetId::Quake3, mode);
        // Cached ordinary acceleration must not replace pm_flyaccelerate.
        state.movement.tuning.accelerate = Some(10.0);
        let mut world = World::new(state.trace_rules);
        pmove(command(16, axes, angles), &mut state, &mut world);
        assert_eq!(state.body.velocity, expected, "{mode:?}");
    }
}

#[test]
fn q3_free_friction_ignores_stale_ground_and_water_without_snapping_velocity() {
    let dt = (16.0f64 * 0.001) as f32;
    let initial = 17.25f32;
    for mode in [MovementMode::Spectator, MovementMode::Noclip] {
        let drop = if mode == MovementMode::Spectator {
            initial * 5.0 * dt
        } else {
            initial.max(100.0) * (6.0 * 1.5) * dt
        };
        let expected = initial * ((initial - drop).max(0.0) / initial);
        let mut state = player(RuleSetId::Quake3, mode);
        state.body.velocity = Vec3([0.0, 0.0, initial]);
        state.movement.grounded = true;
        state.movement.water_level = 3;
        state.movement.timer = MovementTimer::LAND;
        state.movement.remaining_ms = 10;
        let mut world = World::new(state.trace_rules);
        pmove(
            command(16, [0.0; 3], Vec3::default()),
            &mut state,
            &mut world,
        );
        assert_eq!(state.body.velocity.0[2], expected, "{mode:?}");
        assert_ne!(expected, expected.round());
        assert_eq!(state.body.position.0[2], 64.0 + expected * dt);
        assert_eq!(state.movement.remaining_ms, 0);
        assert_eq!(state.movement.timer, MovementTimer::NONE);
        assert_eq!(world.contents.get(), 0);
    }
}

#[test]
fn low_speed_native_free_modes_clear_the_correct_velocity_components() {
    for (rules, mode, expected) in [
        (
            RuleSetId::Quake3,
            MovementMode::Spectator,
            Vec3([0.0, 0.0, 0.5]),
        ),
        (RuleSetId::Quake3, MovementMode::Noclip, Vec3::default()),
        (RuleSetId::Quake2, MovementMode::Spectator, Vec3::default()),
        (
            RuleSetId::QuakeWorld,
            MovementMode::Spectator,
            Vec3::default(),
        ),
    ] {
        let mut state = player(rules, mode);
        state.body.velocity = Vec3([0.25, -0.25, 0.5]);
        let mut world = World::new(state.trace_rules);
        pmove(
            command(16, [0.0; 3], Vec3::default()),
            &mut state,
            &mut world,
        );
        assert_eq!(state.body.velocity, expected, "{rules:?} {mode:?}");
    }
}

#[test]
fn classic_spectator_applies_stop_friction_then_returns_without_integrating() {
    let dt = (16.0f64 * 0.001) as f32;
    for (rules, friction) in [(RuleSetId::Quake2, 6.0), (RuleSetId::QuakeWorld, 4.0)] {
        let mut state = player(rules, MovementMode::Spectator);
        state.body.velocity = Vec3([100.0, 0.0, 0.0]);
        let mut world = World::new(state.trace_rules);
        let mut expected = 100.0 * ((100.0 - 100.0 * (friction * 1.5) * dt) / 100.0);
        if rules == RuleSetId::Quake2 {
            expected = math::narrow_eighth(expected);
        }
        pmove(
            command(16, [0.0; 3], Vec3::default()),
            &mut state,
            &mut world,
        );
        assert_eq!(state.body.velocity.0[0], expected);
        assert_eq!(state.body.position, Vec3([0.0, 0.0, 64.0]));
        assert_eq!(world.traces, 0);
    }
}

#[test]
fn q3_flight_adds_native_flight_friction_and_keeps_ordinary_final_snapping() {
    let dt = (16.0f64 * 0.001) as f32;
    for (height, ground_drop) in [(64.0, 0.0), (24.0, 100.0 * 6.0 * dt)] {
        let mut state = player(RuleSetId::Quake3, MovementMode::Fly);
        state.body.position.0[2] = height;
        state.body.velocity = Vec3([100.0, 0.0, 0.0]);
        let mut world = World::new(state.trace_rules);
        let drop = ground_drop + 100.0 * 3.0 * dt;
        let velocity = 100.0 * ((100.0 - drop) / 100.0);
        pmove(
            command(16, [0.0; 3], Vec3::default()),
            &mut state,
            &mut world,
        );
        assert_eq!(state.body.velocity.0[0], velocity.round_ties_even());
        assert_eq!(state.body.position.0[0], velocity * dt);
        assert!(world.contents.get() > 0);
    }
}

#[test]
fn talk_is_exclusive_in_every_q3_substep_and_preserves_view_angles() {
    for mode in [
        MovementMode::Walk,
        MovementMode::Spectator,
        MovementMode::Noclip,
    ] {
        let mut actual = player(RuleSetId::Quake3, mode);
        let mut expected = player(RuleSetId::Quake3, mode);
        actual.body.velocity = Vec3([20.0, -15.0, 12.0]);
        expected.body.velocity = actual.body.velocity;
        let mut world = World::new(actual.trace_rules);
        let command = UserCmd {
            buttons: buttons::TALK | buttons::ATTACK | buttons::JUMP | buttons::CROUCH,
            ..command(200, [127.0, 63.0, 127.0], Vec3([30.0, 60.0, 0.0]))
        };
        let actual_result = pmove(command, &mut actual, &mut world);
        let expected_result = pmove(
            UserCmd {
                buttons: buttons::TALK,
                movement: [0.0; 3],
                ..command
            },
            &mut expected,
            &mut world,
        );
        assert_eq!(actual.body.position, expected.body.position, "{mode:?}");
        assert_eq!(actual.body.velocity, expected.body.velocity, "{mode:?}");
        assert_eq!(actual.body.mins, expected.body.mins, "{mode:?}");
        assert_eq!(actual.body.maxs, expected.body.maxs, "{mode:?}");
        assert_eq!(actual.movement, expected.movement, "{mode:?}");
        assert_eq!(actual.view_angles, expected.view_angles);
        assert_ne!(actual.view_angles, Vec3::default());
        assert_eq!(actual_result.steps, 4);
        assert_eq!(actual_result.steps, expected_result.steps);
    }
}

#[test]
fn other_movement_roles_do_not_inherit_q3_talk_exclusivity() {
    for rules in [
        RuleSetId::Quake,
        RuleSetId::QuakeWorld,
        RuleSetId::Quake2,
        RuleSetId::Quake2Rerelease,
    ] {
        let mut actual = player(rules, MovementMode::Walk);
        let mut expected = player(rules, MovementMode::Walk);
        let mut world = World::new(actual.trace_rules);
        let command = command(16, [300.0, 0.0, 0.0], Vec3::default());
        pmove(
            UserCmd {
                buttons: buttons::TALK,
                ..command
            },
            &mut actual,
            &mut world,
        );
        pmove(command, &mut expected, &mut world);
        assert_eq!(actual.body.position, expected.body.position, "{rules:?}");
        assert_eq!(actual.body.velocity, expected.body.velocity, "{rules:?}");
        assert_eq!(actual.movement, expected.movement, "{rules:?}");
        assert!(actual.body.velocity.0[0] > 0.0);
    }
}
