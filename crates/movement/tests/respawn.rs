mod support;

use qa_core::primitives::{MovementMode, PlayerState, RuleSetId, UserCmd, Vec3, buttons};
use qa_movement::{TraceServices, pmove, set_bounds};
use qa_world::collision::{
    Contents, EntityTracePolicy, Trace, TraceError, TraceQuery, TraceRules, trace_policy,
};

struct World {
    fixture: support::FixtureWorld,
    rules: TraceRules,
    entity_rules: EntityTracePolicy,
    fail: bool,
}
impl World {
    fn new(rules: RuleSetId) -> Self {
        let (rules, entity_rules) = trace_policy(rules);
        Self {
            fixture: Default::default(),
            rules,
            entity_rules,
            fail: false,
        }
    }
}
impl TraceServices for World {
    fn trace(&mut self, query: TraceQuery) -> Result<Trace, TraceError> {
        assert_eq!(query.rules, self.rules);
        assert_eq!(query.entity_rules, self.entity_rules);
        if self.fail {
            return Err(TraceError::Scratch);
        }
        self.fixture.trace(query)
    }
    fn point_contents(
        &self,
        point: Vec3,
        rules: EntityTracePolicy,
    ) -> Result<Contents, TraceError> {
        assert_eq!(rules, self.entity_rules);
        self.fixture.point_contents(point, rules)
    }
}
fn player(movement: RuleSetId, trace: RuleSetId) -> PlayerState {
    let mut player = PlayerState {
        movement_rules: movement,
        trace_rules: trace,
        health: 100,
        ..Default::default()
    };
    set_bounds(&mut player);
    player.body.position = Vec3([0.0, 0.0, 24.0]);
    player.movement.grounded = true;
    player.movement.respawned = true;
    player
}
fn command(end: i32, buttons: u32) -> UserCmd {
    UserCmd {
        duration_ms: 16,
        server_time_ms: end,
        movement: [0.0, 0.0, 127.0],
        buttons,
        ..Default::default()
    }
}

#[test]
fn q3_respawn_holds_block_jump_with_independent_trace_rules() {
    for trace in RuleSetId::ALL {
        for held in [
            buttons::ATTACK,
            buttons::USE,
            buttons::ATTACK | buttons::USE,
        ] {
            let mut state = player(RuleSetId::Quake3, trace);
            let mut world = World::new(trace);
            let result = pmove(
                command(150, held | buttons::JUMP),
                &mut state,
                &mut world,
                Default::default(),
                qa_movement::no_boundary,
            )
            .unwrap();
            assert_eq!(result.steps, 3);
            assert!(state.movement.respawned);
            assert!(!state.movement.jump_held);
            assert_eq!(state.body.position.0[2], 24.0);
            assert_eq!(state.body.velocity.0[2], 0.0);
            assert_eq!(state.trace_rules, trace);
        }
    }
}

#[test]
fn q3_releasing_attack_and_use_allows_a_still_held_jump() {
    let mut state = player(RuleSetId::Quake3, RuleSetId::Quake2);
    let mut world = World::new(state.trace_rules);
    pmove(
        command(16, buttons::ATTACK | buttons::JUMP),
        &mut state,
        &mut world,
        Default::default(),
        qa_movement::no_boundary,
    )
    .unwrap();
    pmove(
        command(32, buttons::JUMP),
        &mut state,
        &mut world,
        Default::default(),
        qa_movement::no_boundary,
    )
    .unwrap();
    assert!(!state.movement.respawned);
    assert!(state.movement.jump_held);
    assert!(state.body.position.0[2] > 24.0);
    assert!(state.body.velocity.0[2] > 200.0);
}

#[test]
fn q3_respawn_observes_attack_before_talk_and_retains_talk_mutation_between_steps() {
    for (end, respawned, steps) in [(16, true, 1), (150, false, 3)] {
        let mut state = player(RuleSetId::Quake3, RuleSetId::Quake2);
        let mut world = World::new(state.trace_rules);
        let result = pmove(
            command(end, buttons::TALK | buttons::ATTACK | buttons::JUMP),
            &mut state,
            &mut world,
            Default::default(),
            qa_movement::no_boundary,
        )
        .unwrap();
        assert_eq!(result.steps, steps);
        assert_eq!(state.movement.respawned, respawned);
        assert!(!state.movement.jump_held);
        assert_eq!(state.body.position.0[2], 24.0);
    }
}

#[test]
fn q3_respawn_release_precedes_free_or_frozen_returns_and_requires_positive_health() {
    for mode in [
        MovementMode::Spectator,
        MovementMode::Noclip,
        MovementMode::Frozen,
    ] {
        for health in [-1, 0, 1] {
            let mut state = player(RuleSetId::Quake3, RuleSetId::Quake2);
            state.movement.mode = mode;
            state.health = health;
            let mut world = World::new(state.trace_rules);
            pmove(
                UserCmd {
                    server_time_ms: 16,
                    duration_ms: 16,
                    ..Default::default()
                },
                &mut state,
                &mut world,
                Default::default(),
                qa_movement::no_boundary,
            )
            .unwrap();
            assert_eq!(
                state.movement.respawned,
                health <= 0,
                "{mode:?} health{health}"
            );
        }
    }
}

#[test]
fn respawn_jump_behavior_comes_from_the_selected_movement_capability() {
    let mut state = player(RuleSetId::Quake2, RuleSetId::Quake3);
    let mut world = World::new(state.trace_rules);
    pmove(
        command(16, buttons::ATTACK | buttons::JUMP),
        &mut state,
        &mut world,
        Default::default(),
        qa_movement::no_boundary,
    )
    .unwrap();
    assert!(state.movement.respawned);
    assert!(state.movement.jump_held);
    assert!(state.body.position.0[2] > 24.0);
}

#[test]
fn failed_complete_move_restores_the_respawn_latch() {
    let mut state = player(RuleSetId::Quake3, RuleSetId::Quake2);
    let saved = state.movement;
    let mut world = World::new(state.trace_rules);
    world.fail = true;
    assert_eq!(
        pmove(
            command(16, buttons::JUMP),
            &mut state,
            &mut world,
            Default::default(),
            qa_movement::no_boundary
        )
        .unwrap_err(),
        TraceError::Scratch
    );
    assert_eq!(state.movement, saved);
}
