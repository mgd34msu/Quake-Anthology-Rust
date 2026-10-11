mod support;

use qa_core::primitives::{PlayerState, PlayerTail, RuleSetId, UserCmd, Vec3};
use qa_movement::TraceServices;
use qa_world::collision::{Contents, EntityTracePolicy, Trace, TraceQuery, TraceRules};
use std::cell::Cell;

struct CheckedQueries {
    world: support::FixtureWorld,
    expected: TraceRules,
    expected_entities: EntityTracePolicy,
    contents_calls: Cell<u32>,
    calls: u32,
    position_tests: u32,
    ground_tests: u32,
    point_tests: u32,
    ledge_tests: u32,
}

impl TraceServices for CheckedQueries {
    fn trace(&mut self, query: TraceQuery) -> Trace {
        assert_eq!(query.rules, self.expected);
        assert_eq!(query.entity_rules, self.expected_entities);
        assert_eq!(query.pass, qa_core::primitives::CollisionOwner::None);
        assert!(query.excluded.is_empty());
        self.calls += 1;
        self.position_tests += u32::from(query.start == query.end);
        self.ground_tests += u32::from(
            query.start.0[0] == query.end.0[0]
                && query.start.0[1] == query.end.0[1]
                && query.end.0[2] < query.start.0[2],
        );
        self.point_tests +=
            u32::from(query.mins == Vec3::default() && query.maxs == Vec3::default());
        self.ledge_tests += u32::from(query.start.0[2] - query.end.0[2] == 34.0);
        self.world.trace(query)
    }

    fn point_contents(&self, point: Vec3, rules: EntityTracePolicy) -> Contents {
        assert_eq!(rules, self.expected_entities);
        self.contents_calls.set(self.contents_calls.get() + 1);
        self.world.point_contents(point, rules)
    }
}

#[test]
fn every_movement_probe_gets_independent_trace_rules_and_module_tail() {
    // Explicit native caller table: Q1/QW use SV_Move filtering, Q2/RR use
    // SV_Trace filtering with classic/revised clipping, Q3 uses SV_Trace/CM_BoxTrace.
    // The production selector resolver is deliberately not this oracle.
    let trace_policies = [
        (
            RuleSetId::Quake,
            TraceRules::LEGACY,
            qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake).1,
        ),
        (
            RuleSetId::QuakeWorld,
            TraceRules::LEGACY,
            qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake).1,
        ),
        (
            RuleSetId::Quake2,
            TraceRules::LEGACY,
            qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake2).1,
        ),
        (
            RuleSetId::Quake2Rerelease,
            TraceRules::RERELEASE,
            qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake2Rerelease).1,
        ),
        (
            RuleSetId::Quake3,
            TraceRules::ARENA,
            qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake3).1,
        ),
    ];
    assert_eq!(trace_policies.map(|policy| policy.0), RuleSetId::ALL);
    for movement_rules in RuleSetId::ALL {
        for (trace_rules, expected, expected_entities) in trace_policies {
            let mut player = PlayerState {
                movement_rules,
                trace_rules,
                tail: PlayerTail::Q2 { weapon_frame: 9 },
                ..Default::default()
            };
            qa_movement::set_bounds(&mut player);
            player.body.position = Vec3([0.0, 0.0, 24.0]);
            player.body.velocity = Vec3([32.0, 0.0, 0.0]);
            player.movement.grounded = true;
            // The first command stands up from a crouch, exercising the direct
            // stationary trace that previously bypassed the ordinary step helper.
            player.movement.ducked = matches!(
                movement_rules,
                RuleSetId::Quake2 | RuleSetId::Quake2Rerelease | RuleSetId::Quake3
            );
            if player.movement.ducked {
                player.body.maxs.0[2] = 4.0;
            }
            let mut queries = CheckedQueries {
                world: support::FixtureWorld::default(),
                expected,
                expected_entities,
                contents_calls: Cell::new(0),
                calls: 0,
                position_tests: 0,
                ground_tests: 0,
                point_tests: 0,
                ledge_tests: 0,
            };
            let result = qa_movement::pmove(
                UserCmd {
                    duration_ms: 16,
                    server_time_ms: 16,
                    movement: [127.0, 0.0, 0.0],
                    ..Default::default()
                },
                &mut player,
                &mut queries,
            );
            assert!(queries.calls > 0);
            assert!(queries.contents_calls.get() > 0);
            assert_eq!(result.traces, queries.calls);
            if movement_rules == RuleSetId::Quake {
                assert!(queries.ledge_tests > 0 && queries.point_tests > 0);
            } else {
                assert!(queries.ground_tests > 0);
            }
            if matches!(
                movement_rules,
                RuleSetId::Quake2 | RuleSetId::Quake2Rerelease | RuleSetId::Quake3
            ) {
                assert!(queries.position_tests > 0);
                assert!(!player.movement.ducked);
            }
            assert_eq!(player.tail, PlayerTail::Q2 { weapon_frame: 9 });
            assert_eq!(player.movement_rules, movement_rules);
            assert_eq!(player.trace_rules, trace_rules);
        }
    }
}
