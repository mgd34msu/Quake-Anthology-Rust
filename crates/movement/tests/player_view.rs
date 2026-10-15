mod support;

use qa_core::primitives::{PlayerState, RuleSetId, UserCmd, Vec3, buttons};
use qa_movement::{no_boundary, pmove, set_bounds};

fn player(rules: RuleSetId, raw: Vec3, height: Option<f32>, z: f32) -> PlayerState {
    let mut state = PlayerState {
        movement_rules: rules,
        trace_rules: rules,
        health: 100,
        ..Default::default()
    };
    state.view.accept_offset(raw, height);
    set_bounds(&mut state);
    assert_eq!(state.view.raw_offset(), Some((raw, height)));
    state.body.position = Vec3([0.0, 0.0, z]);
    state
}

#[test]
fn authored_camera_offsets_survive_native_and_foreign_crouch_movement() {
    for (rules, crouched_height) in [
        (RuleSetId::Quake2, -2.0),
        (RuleSetId::Quake2Rerelease, -2.0),
        (RuleSetId::Quake3, 12.0),
    ] {
        for (raw, height) in [
            (Vec3([3.0, 4.0, 8.0]), None),
            (Vec3([3.0, 4.0, 3.0]), Some(5.0)),
        ] {
            let mut state = player(rules, raw, height, 24.0);
            state.movement.grounded = true;
            pmove(
                UserCmd {
                    units: qa_gameplay::rules::command_units(rules),
                    duration_ms: 8,
                    server_time_ms: 8,
                    buttons: buttons::CROUCH,
                    ..Default::default()
                },
                &mut state,
                &mut support::FixtureWorld::default(),
                Default::default(),
                no_boundary,
            )
            .unwrap();
            assert!(state.movement.ducked, "{rules:?}");
            assert_eq!(state.movement.view_height, crouched_height, "{rules:?}");
            assert_eq!(state.view.raw_offset(), Some((raw, height)), "{rules:?}");
            assert_eq!(state.eye(), state.body.position + Vec3([3.0, 4.0, 8.0]));
        }
    }
}

#[test]
fn movement_driven_camera_follows_crouch_without_an_authored_offset() {
    for (rules, crouched_height) in [
        (RuleSetId::Quake2, -2.0),
        (RuleSetId::Quake2Rerelease, -2.0),
        (RuleSetId::Quake3, 12.0),
    ] {
        let mut state = PlayerState {
            movement_rules: rules,
            trace_rules: rules,
            health: 100,
            ..Default::default()
        };
        set_bounds(&mut state);
        state.body.position = Vec3([0.0, 0.0, 24.0]);
        state.movement.grounded = true;
        pmove(
            UserCmd {
                units: qa_gameplay::rules::command_units(rules),
                duration_ms: 8,
                server_time_ms: 8,
                buttons: buttons::CROUCH,
                ..Default::default()
            },
            &mut state,
            &mut support::FixtureWorld::default(),
            Default::default(),
            no_boundary,
        )
        .unwrap();
        assert!(state.movement.ducked, "{rules:?}");
        assert_eq!(state.view.raw_offset(), None);
        assert_eq!(
            state.eye(),
            state.body.position + Vec3([0.0, 0.0, crouched_height]),
            "{rules:?}"
        );
    }
}

#[test]
fn water_probes_use_caller_rules_without_replacing_authored_camera_offsets() {
    for (rules, expected_water_level) in [
        (RuleSetId::Quake, 3),
        (RuleSetId::QuakeWorld, 2),
        (RuleSetId::Quake2, 2),
        (RuleSetId::Quake2Rerelease, 2),
        (RuleSetId::Quake3, 2),
    ] {
        let raw = Vec3([3.0, 4.0, 8.0]);
        let mut state = player(rules, raw, None, 48.0);
        pmove(
            UserCmd {
                units: qa_gameplay::rules::command_units(rules),
                duration_ms: 8,
                server_time_ms: 8,
                ..Default::default()
            },
            &mut state,
            &mut support::FixtureWorld {
                water: true,
                ..Default::default()
            },
            Default::default(),
            no_boundary,
        )
        .unwrap();
        assert_eq!(
            state.movement.water_level, expected_water_level,
            "{rules:?}"
        );
        assert_eq!(state.view.raw_offset(), Some((raw, None)), "{rules:?}");
    }
}

#[test]
fn physical_water_samples_keep_native_addition_order_at_liquid_boundary() {
    use qa_core::primitives::MovementTimer;
    use qa_movement::TraceServices;
    use qa_world::collision::{Contents, EntityTracePolicy, Trace, TraceError, TraceQuery};
    use std::cell::RefCell;

    struct Probes {
        points: RefCell<Vec<Vec3>>,
        surface: f32,
    }
    impl TraceServices for Probes {
        fn trace(&mut self, query: TraceQuery) -> Result<Trace, TraceError> {
            Ok(Trace::clear(query.end))
        }
        fn point_contents(
            &self,
            point: Vec3,
            _: EntityTracePolicy,
        ) -> Result<Contents, TraceError> {
            self.points.borrow_mut().push(point);
            Ok(if point.0[2] < self.surface {
                Contents::WATER
            } else {
                Contents::EMPTY
            })
        }
    }

    for (rules, middle, top) in [
        (RuleSetId::Quake2, -11.0f32, 12.0),
        (RuleSetId::Quake2Rerelease, -11.0, 12.0),
        (RuleSetId::Quake3, -9.0, 16.0),
    ] {
        let mut state = PlayerState {
            movement_rules: rules,
            trace_rules: rules,
            health: 100,
            ..Default::default()
        };
        set_bounds(&mut state);
        state.body.position.0[2] = f32::from_bits((-10.0f32).to_bits() + 1);
        state.movement.timer = MovementTimer::TELEPORT;
        state.movement.remaining_ms = 1000;
        let mut probes = Probes {
            points: RefCell::new(Vec::new()),
            surface: top,
        };
        pmove(
            UserCmd {
                units: qa_gameplay::rules::command_units(rules),
                duration_ms: 8,
                server_time_ms: 8,
                ..Default::default()
            },
            &mut state,
            &mut probes,
            Default::default(),
            no_boundary,
        )
        .unwrap();
        let points = probes.points.borrow();
        assert_eq!(points[0].0[2].to_bits(), (-33.0f32).to_bits(), "{rules:?}");
        assert_eq!(points[1].0[2].to_bits(), middle.to_bits(), "{rules:?}");
        assert_eq!(points[2].0[2].to_bits(), top.to_bits(), "{rules:?}");
        assert_eq!(state.movement.water_level, 2, "{rules:?}");
    }
}
