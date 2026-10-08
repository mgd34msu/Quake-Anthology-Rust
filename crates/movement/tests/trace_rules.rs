mod support;

use qa_core::primitives::{MovementRules, PlayerState, PlayerTail, UserCmd, Vec3};
use qa_movement::TraceServices;
use qa_world::collision::{Contents, Trace, TraceQuery, TraceRules};

struct CheckedQueries {
    world: support::FixtureWorld,
    expected: TraceRules,
    calls: u32,
    position_tests: u32,
    ground_tests: u32,
    point_tests: u32,
    ledge_tests: u32,
}

impl TraceServices for CheckedQueries {
    fn trace(&mut self, query: TraceQuery) -> Trace {
        assert_eq!(query.rules, self.expected);
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

    fn point_contents(&self, point: Vec3) -> Contents {
        self.world.point_contents(point)
    }
}

#[test]
fn every_movement_probe_gets_player_rules_independent_of_its_module_tail() {
    for movement_rules in [
        MovementRules::Quake,
        MovementRules::QuakeWorld,
        MovementRules::Quake2,
        MovementRules::Quake2Rerelease,
        MovementRules::Quake3,
    ] {
        let mut player = PlayerState {
            movement_rules,
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
            MovementRules::Quake2 | MovementRules::Quake2Rerelease | MovementRules::Quake3
        );
        if player.movement.ducked {
            player.body.maxs.0[2] = 4.0;
        }
        let mut queries = CheckedQueries {
            world: support::FixtureWorld::default(),
            expected: if movement_rules == MovementRules::Quake3 {
                TraceRules::ARENA
            } else {
                TraceRules::LEGACY
            },
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
                movement: [127, 0, 0],
                ..Default::default()
            },
            &mut player,
            &mut queries,
        );
        assert!(queries.calls > 0);
        assert_eq!(result.traces, queries.calls);
        if movement_rules == MovementRules::Quake {
            assert!(queries.ledge_tests > 0 && queries.point_tests > 0);
        } else {
            assert!(queries.ground_tests > 0);
        }
        if matches!(
            movement_rules,
            MovementRules::Quake2 | MovementRules::Quake2Rerelease | MovementRules::Quake3
        ) {
            assert!(queries.position_tests > 0);
            assert!(!player.movement.ducked);
        }
        assert_eq!(player.tail, PlayerTail::Q2 { weapon_frame: 9 });
    }
}
