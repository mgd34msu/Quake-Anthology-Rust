mod support;

use qa_core::{
    math,
    primitives::{
        Body, MovementMode, MovementTimer, PlayerState, RuleSetId, UserCmd, Vec3, buttons,
    },
};
use qa_movement::{TraceServices, pmove, set_bounds};
use qa_world::collision::{
    Contents, EntityTracePolicy, Trace, TraceQuery, TraceRules, boxes, trace_policy,
};
use std::cell::Cell;

struct World {
    fixture: support::FixtureWorld,
    rules: TraceRules,
    entities: EntityTracePolicy,
    ladder: bool,
    water: bool,
    water_height: f32,
    ledge_points: bool,
    shelf: Option<f32>,
    no_waterjump: bool,
    current: bool,
    ground_current: bool,
    ladder_queries: u32,
    point_queries: Cell<u32>,
}
impl World {
    fn new(rules: RuleSetId) -> Self {
        let (rules, entities) = trace_policy(rules);
        Self {
            fixture: Default::default(),
            rules,
            entities,
            ladder: false,
            water: false,
            water_height: 66.0,
            ledge_points: false,
            shelf: None,
            no_waterjump: false,
            current: false,
            ground_current: false,
            ladder_queries: 0,
            point_queries: Cell::new(0),
        }
    }
}
impl TraceServices for World {
    fn trace(&mut self, query: TraceQuery) -> Result<Trace, qa_world::collision::TraceError> {
        Ok((|| {
            assert_eq!(query.rules, self.rules);
            assert_eq!(query.entity_rules, self.entities);
            let mut result = self.fixture.trace(query).expect("valid movement query");
            if self.ground_current && result.fraction < 1.0 && result.plane.normal.0[2] > 0.7 {
                result.contents |= Contents::from_q2(1 << 18);
            }
            let delta = query.end - query.start;
            if self.ladder
                && delta.0[2] == 0.0
                && delta.0[0] > 0.9
                && delta.0[0] <= 1.00001
                && query.mask.intersects(Contents::SOLID | Contents::LADDER)
            {
                self.ladder_queries += 1;
                result.fraction = 0.5;
                result.end = query.start.lerp(query.end, 0.5);
                result.plane.normal = Vec3([-1.0, 0.0, 0.0]);
                result.contents = Contents::SOLID | Contents::LADDER;
            }
            if let Some(height) = self.shelf {
                let body = Body {
                    position: Vec3([40.0, 0.0, height * 0.5]),
                    mins: Vec3([0.0, -1000.0, -height * 0.5]),
                    maxs: Vec3([1000.0, 1000.0, height * 0.5]),
                    ..Default::default()
                };
                let mut shelf = boxes::trace_box(
                    TraceQuery {
                        mask: Contents::BODY,
                        ..query
                    },
                    &body,
                    None,
                );
                shelf.contents = if shelf.fraction < 1.0 || shelf.start_solid {
                    Contents::SOLID
                } else {
                    Contents::EMPTY
                };
                if shelf.fraction < result.fraction || shelf.all_solid || shelf.start_solid {
                    result = shelf;
                }
            }
            result
        })())
    }
    fn point_contents(
        &self,
        point: Vec3,
        rules: EntityTracePolicy,
    ) -> Result<Contents, qa_world::collision::TraceError> {
        Ok((|| {
            assert_eq!(rules, self.entities);
            self.point_queries.set(self.point_queries.get() + 1);
            if self.ledge_points && point.0[0] > 20.0 && point.0[2] >= 68.0 && point.0[2] < 84.0 {
                return Contents::SOLID;
            }
            if self.water && point.0[2] < self.water_height {
                let mut result = Contents::WATER;
                if self.no_waterjump {
                    result |= Contents::NO_WATERJUMP;
                }
                if self.current {
                    result |= Contents::from_q2(1 << 18);
                }
                result
            } else {
                Contents::EMPTY
            }
        })())
    }
}
fn player(rules: RuleSetId) -> PlayerState {
    let mut player = PlayerState {
        movement_rules: rules,
        trace_rules: RuleSetId::Quake2,
        ..Default::default()
    };
    set_bounds(&mut player);
    player.body.position = Vec3([0.0, 0.0, 64.0]);
    player
}
fn command(ms: u16, movement: [f32; 3], pitch: f32) -> UserCmd {
    UserCmd {
        duration_ms: ms,
        server_time_ms: i32::from(ms),
        movement,
        view_angles: Vec3([pitch, 0.0, 0.0]),
        ..Default::default()
    }
}

#[test]
fn classic_ladder_uses_signed_pitch_and_vertical_controls_before_acceleration() {
    for (pitch, up, expected_z) in [(-30.0, 0.0, 16.0), (30.0, 0.0, -16.0), (0.0, 200.0, 16.0)] {
        let mut state = player(RuleSetId::Quake2);
        let mut world = World::new(state.trace_rules);
        world.ladder = true;
        let forward = if up == 0.0 { 300.0 } else { 0.0 };
        pmove(
            command(8, [forward, 0.0, up], pitch),
            &mut state,
            &mut world,
            Default::default(),
            qa_movement::no_boundary,
        )
        .expect("valid movement query");
        assert!(state.movement.ladder);
        assert_eq!(state.body.velocity.0[2], expected_z);
        assert!(state.body.velocity.0[0].abs() <= 2.0);
        assert_eq!(world.ladder_queries, 1);
    }
}

#[test]
fn ladder_idle_braking_uses_ground_friction_and_approaches_zero() {
    for rules in [RuleSetId::Quake2, RuleSetId::Quake2Rerelease] {
        let mut state = player(rules);
        state.body.velocity.0[2] = 100.0;
        let mut world = World::new(state.trace_rules);
        world.ladder = true;
        pmove(
            command(8, [0.0; 3], 0.0),
            &mut state,
            &mut world,
            Default::default(),
            qa_movement::no_boundary,
        )
        .expect("valid movement query");
        let after_friction = 100.0f32 * ((100.0 - 100.0 * 6.0 * 0.008) / 100.0);
        let expected = after_friction - 800.0 * 0.008;
        assert_eq!(
            state.body.velocity.0[2],
            if rules == RuleSetId::Quake2 {
                math::narrow_eighth(expected)
            } else {
                expected
            }
        );
        assert!(state.movement.ladder);
    }
}

#[test]
fn classic_ladder_flag_is_transient_but_rerelease_retains_it_during_a_native_timer() {
    for rules in [RuleSetId::Quake2, RuleSetId::Quake2Rerelease] {
        let mut state = player(rules);
        state.movement.ladder = true;
        state.movement.remaining_ms = 500;
        state.movement.timer = MovementTimer::LAND;
        let mut world = World::new(state.trace_rules);
        world.ladder = true;
        pmove(
            command(8, [0.0; 3], 0.0),
            &mut state,
            &mut world,
            Default::default(),
            qa_movement::no_boundary,
        )
        .expect("valid movement query");
        assert_eq!(state.movement.ladder, rules == RuleSetId::Quake2Rerelease);
        assert_eq!(world.ladder_queries, 0);
    }
}

#[test]
fn rerelease_ladder_strafe_projects_onto_the_ladder_plane_and_stops_at_waist_water() {
    let mut state = player(RuleSetId::Quake2Rerelease);
    let mut world = World::new(state.trace_rules);
    world.ladder = true;
    pmove(
        command(8, [0.0, 100.0, 0.0], 0.0),
        &mut state,
        &mut world,
        Default::default(),
        qa_movement::no_boundary,
    )
    .expect("valid movement query");
    assert!(state.movement.ladder);
    let acceleration = 10.0 * (8.0f32 * 0.001) * 50.0;
    assert_eq!(state.body.velocity, Vec3([0.0, -acceleration, 0.0]));
    assert_eq!(world.ladder_queries, 2);
    let mut submerged = player(RuleSetId::Quake2Rerelease);
    world.water = true;
    pmove(
        command(8, [0.0, 100.0, 0.0], 0.0),
        &mut submerged,
        &mut world,
        Default::default(),
        qa_movement::no_boundary,
    )
    .expect("valid movement query");
    assert!(!submerged.movement.ladder);
}

#[test]
fn ladder_excludes_water_friction_and_native_currents_adjust_the_shared_wish() {
    let mut dry = player(RuleSetId::Quake2);
    let mut wet = player(RuleSetId::Quake2);
    dry.body.velocity = Vec3([100.0, 0.0, 0.0]);
    wet.body.velocity = dry.body.velocity;
    let mut world = World::new(dry.trace_rules);
    world.ladder = true;
    pmove(
        command(8, [0.0; 3], 0.0),
        &mut dry,
        &mut world,
        Default::default(),
        qa_movement::no_boundary,
    )
    .expect("valid movement query");
    world.water = true;
    pmove(
        command(8, [0.0; 3], 0.0),
        &mut wet,
        &mut world,
        Default::default(),
        qa_movement::no_boundary,
    )
    .expect("valid movement query");
    assert_eq!(wet.body.velocity.0[0], dry.body.velocity.0[0]);
    let mut current = player(RuleSetId::Quake2);
    world.ladder = false;
    world.current = true;
    pmove(
        command(8, [0.0; 3], 0.0),
        &mut current,
        &mut world,
        Default::default(),
        qa_movement::no_boundary,
    )
    .expect("valid movement query");
    assert!(current.body.velocity.0[0] > 0.0);

    let mut conveyor = player(RuleSetId::Quake2);
    conveyor.body.position.0[2] = 24.0;
    world.water = false;
    world.current = false;
    world.ground_current = true;
    pmove(
        command(8, [0.0; 3], 0.0),
        &mut conveyor,
        &mut world,
        Default::default(),
        qa_movement::no_boundary,
    )
    .expect("valid movement query");
    assert_eq!(conveyor.body.velocity.0[0], 8.0);
}

#[test]
fn q2_and_q3_water_jump_preserve_their_impulses_timer_phase_and_gravity_order() {
    for rules in [RuleSetId::Quake2, RuleSetId::Quake3] {
        let mut state = player(rules);
        let mut world = World::new(state.trace_rules);
        world.water = true;
        world.ledge_points = true;
        pmove(
            command(8, [0.0; 3], 0.0),
            &mut state,
            &mut world,
            Default::default(),
            qa_movement::no_boundary,
        )
        .expect("valid movement query");
        assert!(state.movement.timer.contains(MovementTimer::WATER_JUMP));
        assert_eq!(
            state.movement.remaining_ms,
            if rules == RuleSetId::Quake2 {
                2032
            } else {
                2000
            }
        );
        let expected_z = if rules == RuleSetId::Quake2 {
            math::narrow_eighth(350.0 - 800.0 * 0.008)
        } else {
            math::round_vector(Vec3([0.0, 0.0, (350.0 - 800.0 * 0.008) - 800.0 * 0.008])).0[2]
        };
        assert_eq!(state.body.velocity.0[2], expected_z);
        assert_eq!(
            state.body.velocity.0[0],
            if rules == RuleSetId::Quake2 {
                50.0
            } else {
                200.0
            }
        );
    }
}

#[test]
fn water_jump_ignores_input_and_cancels_native_timers_on_descent() {
    for rules in [
        RuleSetId::Quake2,
        RuleSetId::Quake2Rerelease,
        RuleSetId::Quake3,
    ] {
        let mut state = player(rules);
        state.body.velocity = Vec3([10.0, 0.0, 10.0]);
        state.movement.timer = MovementTimer(MovementTimer::WATER_JUMP.0 | MovementTimer::LAND.0);
        state.movement.remaining_ms = 500;
        let mut world = World::new(state.trace_rules);
        pmove(
            command(16, [-300.0, 100.0, 127.0], 0.0),
            &mut state,
            &mut world,
            Default::default(),
            qa_movement::no_boundary,
        )
        .expect("valid movement query");
        assert_eq!(state.movement.timer, MovementTimer::NONE);
        assert_eq!(state.movement.remaining_ms, 0);
        assert_eq!(state.body.velocity.0[0], 10.0);
        assert!(state.body.velocity.0[2] < 0.0);
    }
}

#[test]
fn qw_water_jump_uses_seconds_and_only_held_jump_decrements_the_native_clock() {
    for held in [false, true] {
        let mut state = player(RuleSetId::QuakeWorld);
        let mut world = World::new(state.trace_rules);
        world.water = true;
        world.water_height = 70.0;
        world.ledge_points = true;
        let mut cmd = command(8, [300.0, 0.0, 0.0], 0.0);
        cmd.buttons = if held { buttons::JUMP } else { 0 };
        pmove(
            cmd,
            &mut state,
            &mut world,
            Default::default(),
            qa_movement::no_boundary,
        )
        .expect("valid movement query");
        assert_eq!(
            state.movement.water_jump_seconds,
            if held { 2.0 - 0.008 } else { 2.0 }
        );
        assert_eq!(state.body.velocity.0[0], 50.0);
        assert_eq!(state.body.velocity.0[2], 310.0);
        assert_eq!(state.movement.jump_held, held);
    }
    let mut state = player(RuleSetId::QuakeWorld);
    state.body.velocity.0[2] = -181.0;
    let mut world = World::new(state.trace_rules);
    world.water = true;
    world.water_height = 70.0;
    world.ledge_points = true;
    pmove(
        command(8, [0.0; 3], 0.0),
        &mut state,
        &mut world,
        Default::default(),
        qa_movement::no_boundary,
    )
    .expect("valid movement query");
    assert_eq!(state.movement.water_jump_seconds, 0.0);
}

#[test]
fn rerelease_water_jump_preview_accepts_a_dry_ledge_and_keeps_live_contacts_isolated() {
    let mut state = player(RuleSetId::Quake2Rerelease);
    let mut world = World::new(state.trace_rules);
    world.water = true;
    world.shelf = Some(80.0);
    let result = pmove(
        command(8, [300.0, 0.0, 0.0], 0.0),
        &mut state,
        &mut world,
        Default::default(),
        qa_movement::no_boundary,
    )
    .expect("valid movement query");
    assert!(state.movement.timer.contains(MovementTimer::WATER_JUMP));
    assert_eq!(state.movement.remaining_ms, 2040);
    assert_eq!(
        state.body.velocity,
        Vec3([50.0, 0.0, 350.0 - 800.0 * 0.008])
    );
    assert!(state.body.position.0[0] < 1.0 && state.body.position.0[2] < 68.0);
    assert_eq!(result.contact_count, 0);
    assert!(result.traces > 10);
}

#[test]
fn rerelease_water_jump_rejects_no_intent_forbidden_water_and_missing_or_wet_floor() {
    for (intent, no_waterjump, shelf) in [
        (false, false, Some(80.0)),
        (true, true, Some(80.0)),
        (true, false, None),
        (true, false, Some(32.0)),
    ] {
        let mut state = player(RuleSetId::Quake2Rerelease);
        let mut world = World::new(state.trace_rules);
        world.water = true;
        world.no_waterjump = no_waterjump;
        world.shelf = shelf;
        pmove(
            command(8, [if intent { 300.0 } else { 0.0 }, 0.0, 0.0], 0.0),
            &mut state,
            &mut world,
            Default::default(),
            qa_movement::no_boundary,
        )
        .expect("valid movement query");
        assert!(!state.movement.timer.contains(MovementTimer::WATER_JUMP));
    }
}

#[test]
fn q3_native_movement_does_not_acquire_ladder_behavior_from_foreign_geometry() {
    let mut state = player(RuleSetId::Quake3);
    let mut world = World::new(state.trace_rules);
    world.ladder = true;
    pmove(
        command(8, [127.0, 0.0, 0.0], 0.0),
        &mut state,
        &mut world,
        Default::default(),
        qa_movement::no_boundary,
    )
    .expect("valid movement query");
    assert!(!state.movement.ladder);
    assert_eq!(world.ladder_queries, 0);
    assert!(state.body.velocity.0[2] < 0.0);
    assert_eq!(state.movement.mode, MovementMode::Walk);
}

#[test]
fn landing_from_water_jump_keeps_the_new_native_landing_cooldown() {
    for (rules, expected) in [(RuleSetId::Quake2, 136), (RuleSetId::Quake3, 242)] {
        let mut state = player(rules);
        state.body.position.0[2] = 24.0;
        state.body.velocity.0[2] = -300.0;
        state.movement.timer = MovementTimer(MovementTimer::WATER_JUMP.0 | MovementTimer::LAND.0);
        state.movement.remaining_ms = 500;
        let mut world = World::new(state.trace_rules);
        pmove(
            command(8, [0.0; 3], 0.0),
            &mut state,
            &mut world,
            Default::default(),
            qa_movement::no_boundary,
        )
        .expect("valid movement query");
        assert!(
            state.movement.timer.contains(MovementTimer::LAND),
            "{rules:?}"
        );
        assert!(!state.movement.timer.contains(MovementTimer::WATER_JUMP));
        assert_eq!(state.movement.remaining_ms, expected);
    }
}

#[test]
fn rerelease_swim_buttons_accelerate_without_a_jump_impulse_and_crouch_wins() {
    let impulse = 10.0 * (8.0f32 * 0.001) * 100.0;
    for (buttons, expected_z) in [
        (buttons::JUMP, impulse),
        (buttons::CROUCH, -impulse),
        (buttons::JUMP | buttons::CROUCH, -impulse),
    ] {
        let mut state = player(RuleSetId::Quake2Rerelease);
        let mut world = World::new(state.trace_rules);
        world.water = true;
        world.water_height = 100.0;
        let mut cmd = command(8, [0.0, 0.0, 99.0], 0.0);
        cmd.buttons = buttons;
        pmove(
            cmd,
            &mut state,
            &mut world,
            Default::default(),
            qa_movement::no_boundary,
        )
        .expect("valid movement query");
        assert_eq!(state.body.velocity.0[2], expected_z);
    }
    let mut state = player(RuleSetId::Quake2Rerelease);
    state.body.position.0[2] = 24.0;
    let mut world = World::new(state.trace_rules);
    world.water = true;
    world.water_height = 100.0;
    pmove(
        command(8, [0.0; 3], 0.0),
        &mut state,
        &mut world,
        Default::default(),
        qa_movement::no_boundary,
    )
    .expect("valid movement query");
    assert_eq!(state.body.velocity, Vec3::default());
    assert!(state.movement.grounded);
}

#[test]
fn native_q2_water_speed_caps_do_not_reuse_the_air_duck_cap() {
    struct LowCeiling(World);
    impl TraceServices for LowCeiling {
        fn trace(&mut self, query: TraceQuery) -> Result<Trace, qa_world::collision::TraceError> {
            Ok((|| {
                if query.start == query.end && query.maxs.0[2] == 32.0 {
                    let mut trace = Trace::clear(query.end);
                    trace.start_solid = true;
                    trace.all_solid = true;
                    return trace;
                }
                self.0.trace(query).expect("valid movement query")
            })())
        }
        fn point_contents(
            &self,
            point: Vec3,
            rules: EntityTracePolicy,
        ) -> Result<Contents, qa_world::collision::TraceError> {
            Ok((|| {
                self.0
                    .point_contents(point, rules)
                    .expect("valid movement query")
            })())
        }
    }
    for (rules, speed) in [
        (RuleSetId::Quake2, 150.0),
        (RuleSetId::Quake2Rerelease, 100.0),
    ] {
        let mut state = player(rules);
        state.movement.grounded = true;
        // Rerelease cannot enter a crouch while fully submerged and airborne.
        // Start crouched beneath a ceiling so its native unduck probe retains
        // that stance; this fixture isolates the water-speed cap.
        state.movement.ducked = rules == RuleSetId::Quake2Rerelease;
        let mut world = World::new(state.trace_rules);
        world.water = true;
        world.water_height = 100.0;
        let mut world = LowCeiling(world);
        let mut cmd = command(8, [300.0, 0.0, 0.0], 0.0);
        cmd.buttons = buttons::CROUCH;
        let mut direction = Vec3([300.0, 0.0, -200.0]);
        math::normalize(&mut direction);
        let mut expected = direction * (10.0 * (8.0f32 * 0.001) * speed);
        if rules == RuleSetId::Quake2 {
            expected.0 = expected.0.map(math::narrow_eighth);
        }
        pmove(
            cmd,
            &mut state,
            &mut world,
            Default::default(),
            qa_movement::no_boundary,
        )
        .expect("valid movement query");
        assert!(state.movement.ducked);
        assert_eq!(state.body.velocity, expected, "{rules:?}");
    }
}
