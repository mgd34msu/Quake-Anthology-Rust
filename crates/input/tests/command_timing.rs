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
        _: Option<qa_core::primitives::CommandProducer>,
        _: &str,
    ) -> bool {
        true
    }
    fn cancel_commands(
        &mut self,
        _: qa_core::sys_events::KeySource,
        _: Option<qa_core::primitives::CommandProducer>,
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

#[test]
fn native_client_finish_replaces_only_long_qw_and_q2_command_durations() {
    for rules in [
        qa_core::primitives::RuleSetId::QuakeWorld,
        qa_core::primitives::RuleSetId::Quake2,
    ] {
        let policy = support::catalog_policy(rules);
        let intent = CommandIntent {
            view_angles: Vec3([-0.0, 217.5, -90.0]),
            buttons: 0x8765_4321,
            impulse: 177,
            light_level: 245,
            weapon: Some(WeaponId(23)),
            ..CommandIntent::moving([0.125, -0.25, 0.5])
        };
        let build =
            |ns| UserCmdBuilder::build(EventTime(ns), EventTime(5_500_000_000), intent, policy);
        let baseline = build(250_000_000);
        for (ns, expected) in [
            (0, 0),
            (249_999_999, 249),
            (250_999_999, 250),
            (251_000_000, 100),
            (255_000_000, 100),
            (1_000_000_000, 100),
            (u64::MAX, 100),
        ] {
            let command = build(ns);
            assert_eq!(command.duration_ms, expected, "{rules:?} {ns}ns");
            assert_eq!(command.duration_ns, u64::from(expected) * 1_000_000);
            assert_eq!(command.server_time_ms, 5500);
            assert_eq!(
                command.movement.map(f32::to_bits),
                baseline.movement.map(f32::to_bits)
            );
            assert_eq!(
                command.view_angles.0.map(f32::to_bits),
                intent.view_angles.0.map(f32::to_bits)
            );
            assert_eq!(command.buttons, intent.buttons);
            assert_eq!(command.impulse, intent.impulse);
            assert_eq!(command.light_level, intent.light_level);
            assert_eq!(command.weapon, intent.weapon);
        }
    }
}

#[test]
fn finish_move_fallback_does_not_shorten_the_input_angle_sampling_frame() {
    for rules in [
        qa_core::primitives::RuleSetId::QuakeWorld,
        qa_core::primitives::RuleSetId::Quake2,
    ] {
        let mut input = Input::load();
        input.seed(EventTime(0));
        input.button(
            SeatId::FIRST,
            qa_input::Action::TurnLeft,
            true,
            Some(support::keyboard(26)),
            None,
            EventTime(0),
        );
        let mut policy = support::catalog_policy(rules);
        policy.angle_speed[0] = 100.0;
        let command =
            input.build_frame_with_policy(EventTime(251_000_000), &[policy; SeatId::COUNT])[0];
        let yaw = 0.251_f32 * 100.0;
        let expected = if rules == qa_core::primitives::RuleSetId::QuakeWorld {
            qa_core::math::anglemod(yaw * 0.5)
        } else {
            yaw
        };
        assert_eq!(command.view_angles.0[1].to_bits(), expected.to_bits());
        assert_eq!(
            (command.duration_ms, command.duration_ns),
            (100, 100_000_000)
        );
    }
}
