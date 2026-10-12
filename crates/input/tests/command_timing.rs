#[path = "support/console.rs"]
mod support;
use qa_core::{
    primitives::{CommandIntent, Vec3, WeaponId},
    sys_events::{EventKind, EventTime, SeatId, SysEvent},
};
use qa_input::{Input, Target, UserCmdBuilder};
struct Sink;
impl Target for Sink {
    fn character(&mut self, _: SeatId, _: char) {}
    fn command(
        &mut self,
        _: SeatId,
        _: EventTime,
        _: Option<qa_core::sys_events::KeySource>,
        _: &str,
    ) {
    }
}
#[test]
fn command_duration_keeps_submillisecond_precision_and_saturates_the_native_width() {
    for ns in [
        0,
        999_999,
        1_000_001,
        65_535_999_999,
        65_536_000_000,
        u64::MAX,
    ] {
        let command = UserCmdBuilder::build(
            EventTime(ns),
            EventTime(0),
            CommandIntent::default(),
            support::catalog_policy(qa_core::primitives::RuleSetId::Quake),
        );
        assert_eq!(command.duration_ns, ns);
        assert_eq!(
            command.duration_ms,
            (ns / 1_000_000).min(u64::from(u16::MAX)) as u16
        );
    }
}
#[test]
fn first_time_event_excludes_startup_and_builder_has_no_seat_or_history() {
    let mut input = Input::load();
    let policy = support::catalog_policy(qa_core::primitives::RuleSetId::Quake2);
    input.dispatch(
        SysEvent {
            time: EventTime(5_000_000_000),
            kind: EventKind::Time,
        },
        &mut Sink,
    );
    assert_eq!(
        input.build_frame_with_policy(
            EventTime(5_000_000_000),
            &[policy; qa_core::sys_events::SeatId::COUNT]
        )[0]
        .duration_ms,
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
        input.build_frame_with_policy(
            EventTime(5_020_000_000),
            &[policy; qa_core::sys_events::SeatId::COUNT]
        )[0]
        .duration_ms,
        20
    );
    let intent = CommandIntent {
        view_angles: Vec3([15.0, 25.0, 0.0]),
        weapon: Some(WeaponId(3)),
        ..CommandIntent::moving([0.1, -0.2, 0.3])
    };
    let first = UserCmdBuilder::build(
        EventTime(25_000_000),
        EventTime(9_000_000_000),
        intent,
        support::catalog_policy(qa_core::primitives::RuleSetId::Quake3),
    );
    let other = UserCmdBuilder::build(
        EventTime(10_000_000),
        EventTime(50_000_000),
        intent,
        support::catalog_policy(qa_core::primitives::RuleSetId::Quake3),
    );
    assert_eq!(first.duration_ms, 25);
    assert_eq!(other.duration_ms, 10);
    assert_eq!(first.weapon, Some(WeaponId(3)));
    assert_eq!(first.movement, other.movement);
    assert_eq!(first.server_time_ms, 9000);
    assert_eq!(other.server_time_ms, 50);
}
