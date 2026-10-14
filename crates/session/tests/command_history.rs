use qa_core::{
    command_history::{Acknowledgement, CommandHistory},
    primitives::{MovementMode, PlayerState, RuleSetId, UserCmd, Vec3},
};
use qa_movement::TraceServices;
use qa_session::prediction::Prediction;
use qa_world::collision::{Contents, EntityTracePolicy, Trace, TraceError, TraceQuery};

struct FreeSpace;
impl TraceServices for FreeSpace {
    fn trace(&mut self, query: TraceQuery) -> Result<Trace, TraceError> {
        Ok(Trace::clear(query.end))
    }
    fn point_contents(&self, _: Vec3, _: EntityTracePolicy) -> Result<Contents, TraceError> {
        Ok(Contents::EMPTY)
    }
}
fn player(rules: RuleSetId) -> PlayerState {
    let mut player = PlayerState::default();
    player.movement_rules = rules;
    player.trace_rules = RuleSetId::Quake;
    player.movement.mode = MovementMode::Noclip;
    qa_movement::set_bounds(&mut player);
    player
}
fn command(number: i32) -> UserCmd {
    UserCmd {
        duration_ms: 16,
        duration_ns: 16_000_000,
        server_time_ms: number * 16,
        movement: [127., 19., 0.],
        view_angles: Vec3([0., 45., 0.]),
        ..UserCmd::default()
    }
}
fn motion(prediction: &Prediction) -> ([u32; 3], [u32; 3]) {
    (
        prediction.player.body.position.0.map(f32::to_bits),
        prediction.player.body.velocity.0.map(f32::to_bits),
    )
}

#[test]
fn number_and_time_acknowledgements_replay_without_double_movement_for_every_role() {
    for rules in RuleSetId::ALL {
        for ack in [Acknowledgement::Number(1), Acknowledgement::ServerTime(16)] {
            let mut history = CommandHistory::load(8193, 0).unwrap();
            let mut authoritative = Prediction::new(player(rules));
            history.append(command(1));
            authoritative.advance(command(1), &mut FreeSpace).unwrap();
            let mut predicted = Prediction::new(player(rules));
            predicted.apply_snapshot(&authoritative.player);
            predicted.remember_snapshot(Some(ack));
            for number in 2..=4 {
                history.append(command(number));
                authoritative
                    .advance(command(number), &mut FreeSpace)
                    .unwrap();
            }
            assert_eq!(predicted.replay(&history, &mut FreeSpace).unwrap(), 3);
            assert_eq!(motion(&predicted), motion(&authoritative), "{rules:?}");
            let first = motion(&predicted);
            assert_eq!(predicted.replay(&history, &mut FreeSpace).unwrap(), 3);
            assert_eq!(motion(&predicted), first);
            history.append(command(5));
            authoritative.advance(command(5), &mut FreeSpace).unwrap();
            assert_eq!(predicted.replay(&history, &mut FreeSpace).unwrap(), 4);
            assert_eq!(motion(&predicted), motion(&authoritative));
            assert_eq!(predicted.player.trace_rules, RuleSetId::Quake);
            assert_eq!(predicted.history_resyncs, 0);
        }
    }
}

#[test]
fn exhausted_history_retains_authoritative_position_until_a_new_snapshot() {
    let mut history = CommandHistory::load(3, 0).unwrap();
    for number in 1..=5 {
        history.append(command(number));
    }
    let mut predicted = Prediction::new(player(RuleSetId::Quake3));
    predicted.player.body.position = Vec3([100., 20., 30.]);
    predicted.remember_snapshot(Some(Acknowledgement::Number(0)));
    let baseline = predicted.player.body.position;
    assert_eq!(predicted.replay(&history, &mut FreeSpace).unwrap(), 0);
    assert_eq!(predicted.player.body.position, baseline);
    assert_eq!(predicted.history_resyncs, 1);
    history.append(command(6));
    assert_eq!(predicted.replay(&history, &mut FreeSpace).unwrap(), 0);
    assert_eq!(predicted.player.body.position, baseline);
    assert_eq!(predicted.history_resyncs, 1);
    predicted.remember_snapshot(Some(Acknowledgement::Number(5)));
    assert_eq!(predicted.replay(&history, &mut FreeSpace).unwrap(), 1);
    assert_ne!(predicted.player.body.position, baseline);
}

#[test]
fn absent_native_ack_and_new_lifetimes_keep_incremental_prediction() {
    let mut history = CommandHistory::load(128, 0).unwrap();
    let mut predicted = Prediction::new(player(RuleSetId::Quake3));
    predicted.remember_snapshot(None);
    for number in 1..=3 {
        history.append(command(number));
        assert_eq!(predicted.replay(&history, &mut FreeSpace).unwrap(), 1);
    }
    assert!(predicted.player.body.position.0[0] > 0.0);
    assert_eq!(predicted.history_resyncs, 0);
    predicted.observe_command_progress(Some(2));
    assert_eq!(
        predicted.packet_acknowledged(),
        Some(Acknowledgement::Number(2))
    );
    predicted.clear_history();
    assert_eq!(predicted.packet_acknowledged(), None);
}
