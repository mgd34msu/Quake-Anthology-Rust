mod support;

use qa_core::primitives::{Body, Inventory, MovementState, PlayerState, RuleSetId, UserCmd, Vec3};
use qa_movement::{TraceServices, entry, pmove, set_bounds};
use qa_world::collision::{Contents, EntityTracePolicy, Trace, TraceError, TraceQuery};
use std::cell::Cell;

struct FaultWorld {
    fixture: support::FixtureWorld,
    trace_calls: usize,
    fail_at: usize,
    fail_contents: bool,
    contents_failed: Cell<bool>,
    progressed: bool,
    initial_position: Vec3,
}
impl FaultWorld {
    fn new(position: Vec3) -> Self {
        Self {
            fixture: Default::default(),
            trace_calls: 0,
            fail_at: usize::MAX,
            fail_contents: false,
            contents_failed: Cell::new(false),
            progressed: false,
            initial_position: position,
        }
    }
}
impl TraceServices for FaultWorld {
    fn trace(&mut self, query: TraceQuery) -> Result<Trace, TraceError> {
        self.trace_calls += 1;
        if self.trace_calls == self.fail_at {
            self.progressed = query.start.0[0] != self.initial_position.0[0];
            return Err(TraceError::Scratch);
        }
        self.fixture.trace(query)
    }
    fn point_contents(
        &self,
        point: Vec3,
        policy: EntityTracePolicy,
    ) -> Result<Contents, TraceError> {
        if self.fail_contents {
            self.contents_failed.set(true);
            return Err(TraceError::Scratch);
        }
        self.fixture.point_contents(point, policy)
    }
}

fn player(rules: RuleSetId) -> PlayerState {
    let mut state = PlayerState {
        movement_rules: rules,
        trace_rules: rules,
        inventory: Inventory::with_capacity(3, 2),
        ..Default::default()
    };
    set_bounds(&mut state);
    state.body.position = Vec3([0.13, 0.17, 64.13]);
    state.body.velocity = Vec3([101.17, 2.31, -0.0]);
    state.view_angles = Vec3([12.0, 34.0, 56.0]);
    state.view_offset = Vec3([1.0, 2.0, 3.0]);
    state.inventory.item_counts[1] = 7;
    state.inventory.collectibles = 13;
    state
}
fn command(ms: u16) -> UserCmd {
    UserCmd {
        duration_ms: ms,
        server_time_ms: i32::from(ms),
        movement: [200.0, 3.0, 0.0],
        view_angles: Vec3([-20.0, 0.0, 5.0]),
        ..Default::default()
    }
}
fn body_bits(body: Body) -> [[u32; 3]; 4] {
    [body.position, body.velocity, body.mins, body.maxs].map(|row| row.0.map(f32::to_bits))
}
struct HotState {
    body: [[u32; 3]; 4],
    movement: MovementState,
    view: [u32; 3],
    offset: [u32; 3],
    inventory: *const i32,
}
impl HotState {
    fn save(player: &PlayerState) -> Self {
        Self {
            body: body_bits(player.body),
            movement: player.movement,
            view: player.view_angles.0.map(f32::to_bits),
            offset: player.view_offset.0.map(f32::to_bits),
            inventory: player.inventory.item_counts.as_ptr(),
        }
    }
    fn assert_restored(&self, player: &PlayerState) {
        assert_eq!(body_bits(player.body), self.body);
        assert_eq!(player.movement, self.movement);
        assert_eq!(player.view_angles.0.map(f32::to_bits), self.view);
        assert_eq!(player.view_offset.0.map(f32::to_bits), self.offset);
        assert_eq!(player.inventory.item_counts.as_ptr(), self.inventory);
        assert_eq!(player.inventory.item_counts[1], 7);
        assert_eq!(player.inventory.collectibles, 13);
    }
}

#[test]
fn every_selected_entry_rolls_back_the_command_on_trace_error() {
    for rules in [
        RuleSetId::Quake,
        RuleSetId::QuakeWorld,
        RuleSetId::Quake2,
        RuleSetId::Quake2Rerelease,
        RuleSetId::Quake3,
    ] {
        let mut state = player(rules);
        let saved = HotState::save(&state);
        let mut world = FaultWorld::new(state.body.position);
        world.fail_at = 1;
        assert_eq!(
            entry(rules)(command(16), &mut state, &mut world, Default::default()).unwrap_err(),
            TraceError::Scratch,
            "{rules:?}"
        );
        saved.assert_restored(&state);
    }
}

#[test]
fn qw_and_q3_roll_back_earlier_completed_substeps() {
    for (rules, first_ms, total_ms) in [
        (RuleSetId::QuakeWorld, 50, 100),
        (RuleSetId::Quake3, 66, 200),
    ] {
        let mut baseline = player(rules);
        let mut baseline_world = FaultWorld::new(baseline.body.position);
        let first = pmove(
            command(first_ms),
            &mut baseline,
            &mut baseline_world,
            Default::default(),
        )
        .expect("first substep");
        assert_eq!(first.steps, 1);
        assert!(baseline_world.trace_calls > 0);
        let mut state = player(rules);
        let saved = HotState::save(&state);
        let mut world = FaultWorld::new(state.body.position);
        world.fail_at = baseline_world.trace_calls + 1;
        assert_eq!(
            pmove(
                command(total_ms),
                &mut state,
                &mut world,
                Default::default()
            )
            .unwrap_err(),
            TraceError::Scratch
        );
        assert!(
            world.progressed,
            "first substep moved before rejection: {rules:?}"
        );
        saved.assert_restored(&state);
    }
}

#[test]
fn contents_failure_is_not_empty_water_and_restores_the_command() {
    let mut state = player(RuleSetId::Quake);
    let saved = HotState::save(&state);
    let mut world = FaultWorld::new(state.body.position);
    world.fail_contents = true;
    assert_eq!(
        pmove(command(16), &mut state, &mut world, Default::default()).unwrap_err(),
        TraceError::Scratch
    );
    assert!(world.contents_failed.get());
    saved.assert_restored(&state);
}
