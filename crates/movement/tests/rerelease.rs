mod support;

use qa_core::primitives::{
    EntityId, MovementMode, MovementTimer, Plane, PlayerState, RuleSetId, UserCmd, Vec3, buttons,
};
use qa_movement::{TraceServices, pmove, set_bounds};
use qa_world::collision::{Contents, EntityTracePolicy, Trace, TraceQuery};

fn player() -> PlayerState {
    let mut player = PlayerState {
        movement_rules: RuleSetId::Quake2Rerelease,
        trace_rules: RuleSetId::Quake3,
        ..Default::default()
    };
    set_bounds(&mut player);
    player.body.position = Vec3([0.0, 0.0, 64.0]);
    player
}
fn command(buttons: u32) -> UserCmd {
    UserCmd {
        duration_ms: 8,
        server_time_ms: 8,
        buttons,
        ..Default::default()
    }
}
fn contact(query: TraceQuery, normal: Vec3, fraction: f32) -> Trace {
    let mut trace = Trace::clear(query.end);
    trace.fraction = fraction;
    trace.end = query.start.lerp(query.end, fraction);
    trace.plane = Plane::oriented(normal, 0.0);
    trace.entity = Some(EntityId {
        slot: 0,
        generation: 1,
    });
    trace.contents = Contents::SOLID;
    trace
}

#[test]
fn native_land_and_trick_timers_use_fresh_landing_rules_and_publish_impact() {
    for (ducked, n64, z, timer, time) in [
        (false, false, 150.0, MovementTimer::TRICK, 56),
        (true, false, -300.0, MovementTimer::LAND, 120),
        (false, true, -300.0, MovementTimer::LAND, 120),
        (false, false, -300.0, MovementTimer::NONE, 0),
    ] {
        let mut state = player();
        state.body.position.0[2] = 24.0;
        state.body.velocity.0[2] = z;
        state.movement.ducked = ducked;
        state.movement.tuning.n64_physics = n64;
        let result = pmove(
            command(if ducked { buttons::CROUCH } else { 0 }),
            &mut state,
            &mut support::FixtureWorld::default(),
            Default::default(),
        );
        assert_eq!(state.movement.timer, timer);
        assert_eq!(state.movement.remaining_ms, time);
        let clipped = z - z * 1.01;
        assert_eq!(result.impact_delta, z - clipped);
        assert!(!result.jump_sound);
    }
}

#[test]
fn crouching_on_the_landing_frame_does_not_create_an_old_stance_land_timer() {
    let mut state = player();
    state.body.position.0[2] = 24.0;
    state.body.velocity.0[2] = -300.0;
    pmove(
        command(buttons::CROUCH),
        &mut state,
        &mut support::FixtureWorld::default(),
        Default::default(),
    );
    assert!(state.movement.ducked);
    assert_eq!(state.body.maxs.0[2], 4.0);
    assert_eq!(state.movement.timer, MovementTimer::NONE);
    assert_eq!(state.movement.remaining_ms, 0);
}

#[test]
fn n64_crouch_is_disabled_while_its_native_landing_pause_is_retained() {
    let mut state = player();
    state.body.position.0[2] = 24.0;
    state.body.velocity.0[2] = -300.0;
    state.movement.tuning.n64_physics = true;
    pmove(
        command(buttons::CROUCH),
        &mut state,
        &mut support::FixtureWorld::default(),
        Default::default(),
    );
    assert!(!state.movement.ducked);
    assert_eq!(state.body.maxs.0[2], 32.0);
    assert_eq!(state.movement.timer, MovementTimer::LAND);
    assert_eq!(state.movement.remaining_ms, 120);
}

#[test]
fn native_circular_pitch_clamp_keeps_rerelease_float_ties_and_classic_short_semantics() {
    for rules in [RuleSetId::Quake2, RuleSetId::Quake2Rerelease] {
        for pitch in [
            -90.125, 89.0, 89.125, 179.875, 180.0, 270.875, 271.0, 360.125,
        ] {
            let mut state = player();
            state.movement_rules = rules;
            state.movement.mode = MovementMode::Spectator;
            let angles = Vec3([pitch, 45.123, 12.5]);
            let mut expected = angles;
            if rules == RuleSetId::Quake2 {
                expected.0 = expected.0.map(|angle| {
                    let short = qa_core::math::angle_to_short(
                        angle,
                        qa_core::math::AngleShortForm::Factored,
                    ) as i16;
                    qa_core::math::short_to_angle(i32::from(short))
                });
            }
            if expected.0[0] > 89.0 && expected.0[0] < 180.0 {
                expected.0[0] = 89.0;
            } else if expected.0[0] < 271.0 && expected.0[0] >= 180.0 {
                expected.0[0] = 271.0;
            }
            let mut cmd = command(0);
            cmd.view_angles = angles;
            pmove(
                cmd,
                &mut state,
                &mut support::FixtureWorld::default(),
                Default::default(),
            );
            assert_eq!(
                state.view_angles.0.map(f32::to_bits),
                expected.0.map(f32::to_bits),
                "{rules:?} pitch{pitch}"
            );
        }
    }
}

#[test]
fn native_teleport_angle_pause_zeroes_pitch_and_roll_without_narrowing_rerelease_yaw() {
    for rules in [RuleSetId::Quake2, RuleSetId::Quake2Rerelease] {
        let mut state = player();
        state.movement_rules = rules;
        state.movement.timer = MovementTimer::TELEPORT;
        state.movement.remaining_ms = 100;
        state.movement.delta_angles = Vec3([12.5, 3.125, 17.25]);
        let mut cmd = command(0);
        cmd.view_angles = Vec3([100.0, 45.123, 32.5]);
        let yaw = if rules == RuleSetId::Quake2 {
            let encode = |angle| {
                qa_core::math::angle_to_short(angle, qa_core::math::AngleShortForm::Factored) as i16
            };
            qa_core::math::short_to_angle(i32::from(
                encode(cmd.view_angles.0[1]).wrapping_add(encode(state.movement.delta_angles.0[1])),
            ))
        } else {
            cmd.view_angles.0[1] + state.movement.delta_angles.0[1]
        };
        pmove(
            cmd,
            &mut state,
            &mut support::FixtureWorld::default(),
            Default::default(),
        );
        assert_eq!(
            state.view_angles.0.map(f32::to_bits),
            Vec3([0.0, yaw, 0.0]).0.map(f32::to_bits)
        );
    }
}

#[test]
fn rerelease_air_ducking_uses_solid_then_water_probes_and_refuses_the_water_surface() {
    struct Below {
        solid: bool,
        water: bool,
        masks: Vec<Contents>,
    }
    impl TraceServices for Below {
        fn trace(&mut self, query: TraceQuery) -> Trace {
            if query.end - query.start == Vec3([0.0, 0.0, -8.0]) {
                self.masks.push(query.mask);
                if query.mask == (Contents::SOLID | Contents::WINDOW) && self.solid
                    || query.mask == Contents::FLUID && self.water
                {
                    return contact(query, Vec3([0.0, 0.0, 1.0]), 0.5);
                }
            }
            Trace::clear(query.end)
        }
        fn point_contents(&self, _: Vec3, _: EntityTracePolicy) -> Contents {
            Contents::EMPTY
        }
    }
    for (solid, water, ducked) in [
        (false, false, true),
        (false, true, false),
        (true, true, true),
    ] {
        let mut state = player();
        state.movement.tuning.no_step = true;
        let mut world = Below {
            solid,
            water,
            masks: Vec::new(),
        };
        pmove(
            command(buttons::CROUCH),
            &mut state,
            &mut world,
            Default::default(),
        );
        assert_eq!(state.movement.ducked, ducked);
        assert_eq!(world.masks[0], Contents::SOLID | Contents::WINDOW);
        assert_eq!(world.masks.len(), if solid { 1 } else { 2 });
        if !solid {
            assert_eq!(world.masks[1], Contents::FLUID);
        }
    }
}

#[test]
fn rerelease_dead_and_gib_dimensions_are_native_even_with_n64_enabled() {
    for mode in [MovementMode::Dead, MovementMode::Gib] {
        let mut state = player();
        state.movement.mode = mode;
        state.movement.tuning.n64_physics = true;
        pmove(
            command(0),
            &mut state,
            &mut support::FixtureWorld::default(),
            Default::default(),
        );
        if mode == MovementMode::Dead {
            assert!(state.movement.ducked);
            assert_eq!(
                (
                    state.body.mins.0[2],
                    state.body.maxs.0[2],
                    state.view_offset.0[2]
                ),
                (-24.0, 4.0, -2.0)
            );
        } else {
            assert_eq!(
                (
                    state.body.mins.0[2],
                    state.body.maxs.0[2],
                    state.view_offset.0[2]
                ),
                (0.0, 16.0, 8.0)
            );
        }
    }
}

#[test]
fn initial_snap_uses_native_candidate_order_between_old_stance_and_ducking() {
    struct Initial {
        calls: Vec<(Vec3, Vec3, Vec3)>,
        initial: usize,
    }
    impl TraceServices for Initial {
        fn trace(&mut self, query: TraceQuery) -> Trace {
            self.calls.push((query.start, query.end, query.maxs));
            if query.start == query.end && query.maxs.0[2] == 32.0 && self.initial < 3 {
                self.initial += 1;
                let mut trace = Trace::clear(query.end);
                trace.all_solid = self.initial < 3;
                trace.start_solid = trace.all_solid;
                return trace;
            }
            if query.end.0[2] - query.start.0[2] == -0.25 {
                return contact(query, Vec3([0.0, 0.0, 1.0]), 0.0);
            }
            Trace::clear(query.end)
        }
        fn point_contents(&self, _: Vec3, _: EntityTracePolicy) -> Contents {
            Contents::EMPTY
        }
    }
    let mut state = player();
    state.movement.timer = MovementTimer::TELEPORT;
    state.movement.remaining_ms = 100;
    let mut world = Initial {
        calls: Vec::new(),
        initial: 0,
    };
    pmove(
        command(buttons::CROUCH),
        &mut state,
        &mut world,
        qa_movement::MoveOptions { initial_snap: true },
    );
    let origin = Vec3([0.0, 0.0, 64.0]);
    assert_eq!(
        world.calls[0],
        (origin, Vec3([0.0, 0.0, 63.75]), Vec3([16.0, 16.0, 32.0]))
    );
    for (index, x) in [0.0, -1.0, 1.0].into_iter().enumerate() {
        let position = Vec3([x, 0.0, 64.0]);
        assert_eq!(
            world.calls[index + 1],
            (position, position, Vec3([16.0, 16.0, 32.0]))
        );
    }
    let accepted = Vec3([1.0, 0.0, 64.0]);
    assert_eq!(
        world.calls[4],
        (accepted, accepted, Vec3([16.0, 16.0, 4.0]))
    );
    assert_eq!(
        world.calls[5],
        (accepted, Vec3([1.0, 0.0, 63.75]), Vec3([16.0, 16.0, 4.0]))
    );
    assert_eq!(state.body.position, accepted);
    assert!(state.movement.ducked);
}

#[test]
fn initial_snap_updates_the_native_recovery_fallback_origin() {
    struct Initial {
        stationary: usize,
        accepts: usize,
    }
    impl TraceServices for Initial {
        fn trace(&mut self, query: TraceQuery) -> Trace {
            let mut trace = Trace::clear(query.end);
            if query.start == query.end {
                self.stationary += 1;
                trace.start_solid = self.stationary != self.accepts;
                trace.all_solid = trace.start_solid;
            }
            trace
        }
        fn point_contents(&self, _: Vec3, _: EntityTracePolicy) -> Contents {
            Contents::EMPTY
        }
    }
    for (accepts, expected_x) in [(2, -1.0), (usize::MAX, 0.0)] {
        let mut state = player();
        state.movement.timer = MovementTimer::TELEPORT;
        state.movement.remaining_ms = 100;
        let mut world = Initial {
            stationary: 0,
            accepts,
        };
        pmove(
            command(0),
            &mut state,
            &mut world,
            qa_movement::MoveOptions { initial_snap: true },
        );
        assert_eq!(state.body.position, Vec3([expected_x, 0.0, 64.0]));
    }
}

#[test]
fn classic_initial_snap_uses_eighth_units_before_ground_categorization() {
    struct Initial {
        calls: Vec<(Vec3, Vec3)>,
    }
    impl TraceServices for Initial {
        fn trace(&mut self, query: TraceQuery) -> Trace {
            self.calls.push((query.start, query.end));
            let mut trace = Trace::clear(query.end);
            trace.all_solid = self.calls.len() == 1;
            trace.start_solid = trace.all_solid;
            trace
        }
        fn point_contents(&self, _: Vec3, _: EntityTracePolicy) -> Contents {
            Contents::EMPTY
        }
    }
    let mut state = player();
    state.movement_rules = RuleSetId::Quake2;
    state.movement.timer = MovementTimer::TELEPORT;
    state.movement.remaining_ms = 100;
    let mut world = Initial { calls: Vec::new() };
    pmove(
        command(0),
        &mut state,
        &mut world,
        qa_movement::MoveOptions { initial_snap: true },
    );
    assert_eq!(
        world.calls[0],
        (Vec3([0.0, 0.0, 64.0]), Vec3([0.0, 0.0, 64.0]))
    );
    assert_eq!(
        world.calls[1],
        (Vec3([-0.125, 0.0, 64.0]), Vec3([-0.125, 0.0, 64.0]))
    );
    assert_eq!(
        world.calls[2],
        (Vec3([-0.125, 0.0, 64.0]), Vec3([-0.125, 0.0, 63.75]))
    );
    assert_eq!(state.body.position, Vec3([-0.125, 0.0, 64.0]));
}

#[test]
fn water_jump_cancellation_precedes_rerelease_landing_and_timer_expiry() {
    let mut state = player();
    state.body.position.0[2] = 24.0;
    state.body.velocity.0[2] = -300.0;
    state.movement.ducked = true;
    state.movement.timer = MovementTimer(
        MovementTimer::WATER_JUMP.0 | MovementTimer::TRICK.0 | MovementTimer::TELEPORT.0,
    );
    state.movement.remaining_ms = 500;
    pmove(
        command(buttons::CROUCH),
        &mut state,
        &mut support::FixtureWorld::default(),
        Default::default(),
    );
    assert_eq!(state.movement.timer, MovementTimer::LAND);
    assert_eq!(state.movement.remaining_ms, 120);
    state.movement.timer = MovementTimer::TRICK;
    state.movement.remaining_ms = 8;
    pmove(
        command(0),
        &mut state,
        &mut support::FixtureWorld::default(),
        Default::default(),
    );
    assert_eq!(state.movement.timer, MovementTimer::NONE);
    assert_eq!(state.movement.remaining_ms, 0);
}

#[test]
fn native_landing_gate_preserves_jump_held_until_the_timer_allows_release() {
    for rules in [RuleSetId::Quake2, RuleSetId::Quake2Rerelease] {
        let mut state = player();
        state.movement_rules = rules;
        state.body.position.0[2] = 24.0;
        state.movement.grounded = true;
        state.movement.jump_held = true;
        state.movement.timer = MovementTimer::LAND;
        state.movement.remaining_ms = 16;
        let mut world = support::FixtureWorld::default();
        pmove(command(0), &mut state, &mut world, Default::default());
        assert!(state.movement.jump_held, "{rules:?}");
        assert_eq!(state.movement.remaining_ms, 8);
        pmove(command(0), &mut state, &mut world, Default::default());
        assert!(!state.movement.jump_held, "{rules:?}");
        assert_eq!(state.movement.remaining_ms, 0);
    }
}

#[test]
fn a_trick_landing_rechecks_the_same_jump_after_final_categorization() {
    struct Landing {
        quarter_traces: usize,
    }
    impl TraceServices for Landing {
        fn trace(&mut self, query: TraceQuery) -> Trace {
            if query.end.0[2] - query.start.0[2] == -0.25 {
                self.quarter_traces += 1;
                if self.quarter_traces == 2 {
                    return contact(query, Vec3([0.0, 0.0, 1.0]), 0.0);
                }
            }
            Trace::clear(query.end)
        }
        fn point_contents(&self, _: Vec3, _: EntityTracePolicy) -> Contents {
            Contents::EMPTY
        }
    }
    let mut state = player();
    state.body.velocity.0[2] = 150.0;
    state.movement.tuning.no_step = true;
    let result = pmove(
        command(buttons::JUMP),
        &mut state,
        &mut Landing { quarter_traces: 0 },
        Default::default(),
    );
    assert_eq!(
        state.body.velocity.0[2],
        (150.0 - 800.0 * (8.0f32 * 0.001)) + 270.0
    );
    assert_eq!(state.movement.timer, MovementTimer::TRICK);
    assert_eq!(state.movement.remaining_ms, 64);
    assert!(!state.movement.grounded);
    assert!(state.movement.jump_held && result.jump_sound);
}

#[test]
fn raised_stair_path_checks_the_lower_endpoint_then_uses_the_real_descent() {
    struct Stair {
        blocked: bool,
        drops: Vec<(Vec3, Vec3)>,
    }
    impl TraceServices for Stair {
        fn trace(&mut self, query: TraceQuery) -> Trace {
            if query.end.0[2] - query.start.0[2] < -1.0 {
                self.drops.push((query.start, query.end));
                let mut trace = contact(query, Vec3([0.0, 0.0, 1.0]), 0.5);
                if self.drops.len() == 1 {
                    trace.all_solid = self.blocked;
                } else {
                    trace.end = query.end;
                }
                return trace;
            }
            Trace::clear(query.end)
        }
        fn point_contents(&self, _: Vec3, _: EntityTracePolicy) -> Contents {
            Contents::EMPTY
        }
    }
    for blocked in [false, true] {
        let mut state = player();
        state.movement.mode = MovementMode::Spectator;
        state.body.velocity = Vec3([100.0, 0.0, 100.0]);
        let mut world = Stair {
            blocked,
            drops: Vec::new(),
        };
        let result = pmove(command(0), &mut state, &mut world, Default::default());
        assert_eq!(world.drops[0].1.0[2], 63.0);
        assert_eq!(world.drops.len(), if blocked { 1 } else { 2 });
        assert_eq!(result.step_clip, !blocked);
        if !blocked {
            assert_eq!(world.drops[1].0, world.drops[0].0);
            assert!(world.drops[1].1.0[2] > 64.0);
            assert_eq!(state.body.position.0[2], world.drops[1].1.0[2]);
        }
    }
}

#[test]
fn grounded_stair_descent_follows_the_floor_but_ladder_motion_does_not() {
    struct Down {
        ladder: bool,
        drops: usize,
    }
    impl TraceServices for Down {
        fn trace(&mut self, query: TraceQuery) -> Trace {
            let delta = query.end - query.start;
            if delta.0[2] == -0.25 {
                return contact(query, Vec3([0.0, 0.0, 1.0]), 0.0);
            }
            if self.ladder && delta == Vec3([1.0, 0.0, 0.0]) {
                let mut trace = contact(query, Vec3([-1.0, 0.0, 0.0]), 0.5);
                trace.contents = Contents::LADDER;
                return trace;
            }
            if delta.0[2] == -18.0 {
                self.drops += 1;
                if self.drops == 3 {
                    return contact(query, Vec3([0.0, 0.0, 1.0]), 8.0 / 18.0);
                }
                let mut trace = Trace::clear(query.end);
                trace.plane = Plane::oriented(Vec3([0.0, 0.0, 1.0]), 0.0);
                return trace;
            }
            Trace::clear(query.end)
        }
        fn point_contents(&self, _: Vec3, _: EntityTracePolicy) -> Contents {
            Contents::EMPTY
        }
    }
    for ladder in [false, true] {
        let mut state = player();
        state.movement.grounded = true;
        let mut world = Down { ladder, drops: 0 };
        let mut cmd = command(0);
        cmd.movement[0] = if ladder { 0.0 } else { 100.0 };
        pmove(cmd, &mut state, &mut world, Default::default());
        assert_eq!(world.drops, if ladder { 2 } else { 3 });
        assert_eq!(state.body.position.0[2], if ladder { 64.0 } else { 56.0 });
        assert!(state.movement.grounded);
    }
}

#[test]
fn native_recovery_uses_the_clear_face_and_one_eighth_unit_separation() {
    let mut state = player();
    state.body.position.0[2] = 10.0;
    state.movement.tuning.no_step = true;
    let result = pmove(
        command(0),
        &mut state,
        &mut support::FixtureWorld::default(),
        Default::default(),
    );
    assert_eq!(state.body.position.0[2], 24.125);
    assert_eq!(state.body.velocity, Vec3::default());
    assert!(result.traces > 10);
}

#[test]
fn failed_native_recovery_restores_the_previous_position_without_growing_state() {
    struct Sealed {
        stationary: usize,
    }
    impl TraceServices for Sealed {
        fn trace(&mut self, query: TraceQuery) -> Trace {
            let mut trace = Trace::clear(query.end);
            if query.start == query.end {
                self.stationary += 1;
                trace.start_solid = true;
                trace.all_solid = true;
                trace.fraction = 0.0;
                trace.end = query.start;
            }
            trace
        }
        fn point_contents(&self, _: Vec3, _: EntityTracePolicy) -> Contents {
            Contents::EMPTY
        }
    }
    let mut state = player();
    let previous = state.body.position;
    state.body.velocity.0[0] = 100.0;
    state.movement.tuning.no_step = true;
    let mut world = Sealed { stationary: 0 };
    pmove(command(0), &mut state, &mut world, Default::default());
    assert_eq!(state.body.position, previous);
    assert_eq!(world.stationary, 32);
}

#[test]
fn repeated_native_planes_use_the_same_position_recovery() {
    struct Wall {
        hits: usize,
        recovered: bool,
    }
    impl TraceServices for Wall {
        fn trace(&mut self, query: TraceQuery) -> Trace {
            let delta = query.end - query.start;
            if (delta.0[0] != 0.0 || delta.0[1] != 0.0)
                && query.mins == Vec3([-16.0, -16.0, -24.0])
                && self.hits < 2
            {
                self.hits += 1;
                return contact(query, Vec3([1.0, 0.0, 0.0]), 0.0);
            }
            let a = query.start.0[0] + query.mins.0[0];
            let b = query.end.0[0] + query.mins.0[0];
            let mut trace = Trace::clear(query.end);
            if a < 0.0 {
                trace.start_solid = true;
                trace.all_solid = b < 0.0;
            } else if b < 0.0 {
                trace = contact(query, Vec3([1.0, 0.0, 0.0]), a / (a - b));
            }
            if self.hits == 2 && query.start == query.end && query.start.0[0] == 16.125 {
                self.recovered = true;
            }
            trace
        }
        fn point_contents(&self, _: Vec3, _: EntityTracePolicy) -> Contents {
            Contents::EMPTY
        }
    }
    let mut state = player();
    state.body.velocity = Vec3([-100.0, 100.0, 0.0]);
    state.movement.tuning.no_step = true;
    let mut world = Wall {
        hits: 0,
        recovered: false,
    };
    pmove(command(0), &mut state, &mut world, Default::default());
    assert_eq!(world.hits, 2);
    assert!(world.recovered);
    assert!(state.body.position.0[0] >= 16.125);
}
