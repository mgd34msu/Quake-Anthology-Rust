use qa_core::{
    primitives::{CommandIntent, Vec3, WeaponId},
    sys_events::{EventKind, EventTime, SeatId, SysEvent},
};
use qa_input::{Input, Target, UserCmdBuilder};
use std::time::Duration;
struct Sink;
impl Target for Sink {
    fn character(&mut self, _: SeatId, _: char) {}
    fn command(&mut self, _: SeatId, _: EventTime, _: &str) {}
}
#[test]
fn first_time_event_excludes_startup_and_builder_has_no_seat_or_history() {
    let mut input = Input::load();
    input.dispatch(
        SysEvent {
            time: EventTime(5_000_000_000),
            kind: EventKind::Time,
        },
        &mut Sink,
    );
    assert_eq!(
        input.build_frame(EventTime(5_000_000_000), [127; 3], [0.022; 2])[0].duration_ms,
        0
    );
    input.dispatch(
        SysEvent {
            time: EventTime(5_020_000_000),
            kind: EventKind::Time,
        },
        &mut Sink,
    );
    assert_eq!(
        input.build_frame(EventTime(5_020_000_000), [127; 3], [0.022; 2])[0].duration_ms,
        20
    );
    let intent = CommandIntent {
        view_angles: Vec3([15.0, 25.0, 0.0]),
        weapon: Some(WeaponId(3)),
        ..CommandIntent::moving([0.1, -0.2, 0.3])
    };
    let first = UserCmdBuilder::build(
        Duration::from_millis(25),
        EventTime(9_000_000_000),
        intent,
        qa_input::InputPolicy::native(qa_core::primitives::RuleSetId::Quake3),
    );
    let other = UserCmdBuilder::build(
        Duration::from_millis(10),
        EventTime(50_000_000),
        intent,
        qa_input::InputPolicy::native(qa_core::primitives::RuleSetId::Quake3),
    );
    assert_eq!(first.duration_ms, 25);
    assert_eq!(other.duration_ms, 10);
    assert_eq!(first.weapon, Some(WeaponId(3)));
    assert_eq!(first.movement, other.movement);
    assert_eq!(first.server_time_ms, 9000);
    assert_eq!(other.server_time_ms, 50);
}
