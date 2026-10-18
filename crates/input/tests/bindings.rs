use support::RoutedInput as Input;
#[path = "support/console.rs"]
mod support;
use qa_core::sys_events::{DeviceId, EventKind, EventTime, SeatId, SysEvent};
use qa_input::{Target, keys};
#[derive(Default)]
struct Sink(Vec<(SeatId, String)>);
impl Target for Sink {
    fn character(&mut self, _: SeatId, _: char) {}
    fn command(
        &mut self,
        seat: SeatId,
        _: EventTime,
        _: Option<qa_core::sys_events::KeySource>,
        _: Option<qa_core::primitives::CommandProducer>,
        text: &str,
    ) -> bool {
        self.0.push((seat, text.to_owned()));
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
fn config_names_hex_and_modifier_sides_share_one_control_table() {
    for (name, control) in [
        ("W", 26),
        ("0x77", 26),
        ("CTRL", 224),
        ("0x89", 224),
        ("MWHEELUP", 576),
        ("MOUSE32", 580),
        ("JOY32", 575),
        ("AUX32", 671),
        ("SEMICOLON", 51),
        (";", 51),
        ("~", 53),
    ] {
        assert_eq!(keys::parse(name), Some(control));
    }
    for value in 0u8..=255 {
        let name = format!("0x{value:02x}");
        let control = keys::parse(&name).unwrap();
        let mut written = String::new();
        keys::write_name(control, &mut written).unwrap();
        assert_eq!(keys::parse(&written), Some(control));
    }
    assert_eq!(keys::normalize(228), 224);
    assert_eq!(keys::parse("é"), None);
    assert_eq!(keys::parse("0xgg"), None);
}
#[test]
fn command_bindings_keep_release_metadata_quote_spans_and_capacity_atomicity() {
    let mut input = Input::load();
    let policy = input.elapsed_policy(127.0);
    input.seed(EventTime(0));
    let mut sink = Sink::default();
    input
        .bind_text(
            SeatId::FIRST,
            10,
            "+edge; echo \"a;b\"; +jump",
            EventTime(0),
            &mut sink,
        )
        .unwrap();
    assert!(
        input
            .bind_text(
                SeatId::FIRST,
                10,
                &"x".repeat(1025),
                EventTime(0),
                &mut sink
            )
            .is_err()
    );
    for (ms, down, repeat) in [(10, true, false), (15, true, true), (20, false, false)] {
        input.dispatch(
            SysEvent {
                time: EventTime(ms * 1_000_000),
                kind: EventKind::Key {
                    device: DeviceId::Keyboard(0),
                    code: 10,
                    symbol: 0,
                    down,
                    repeat,
                },
            },
            &mut sink,
        );
    }
    assert_eq!(
        sink.0,
        [
            (SeatId::FIRST, "+edge 10 10".to_owned()),
            (SeatId::FIRST, "echo \"a;b\"".to_owned()),
            (SeatId::FIRST, "+jump 10 10".to_owned()),
            (SeatId::FIRST, "-edge 10 20".to_owned()),
            (SeatId::FIRST, "echo \"a;b\"".to_owned()),
            (SeatId::FIRST, "-jump 10 20".to_owned())
        ]
    );
    assert_ne!(
        input.build_frame_with_policy(
            EventTime(30_000_000),
            &[policy; qa_core::sys_events::SeatId::COUNT]
        )[0]
        .buttons
            & qa_core::primitives::buttons::JUMP,
        0
    );
}
#[test]
fn both_ctrl_keys_hold_one_action_and_wheel_axes_dispatch_momentary_commands() {
    let mut input = Input::load();
    let policy = input.elapsed_policy(127.0);
    input.seed(EventTime(0));
    let mut sink = Sink::default();
    input
        .bind_text(SeatId::FIRST, 224, "+forward", EventTime(0), &mut sink)
        .unwrap();
    input
        .bind_text(SeatId::FIRST, 576, "+edge", EventTime(0), &mut sink)
        .unwrap();
    for (ms, code, down) in [(0, 224, true), (10, 228, true), (20, 224, false)] {
        input.dispatch(
            SysEvent {
                time: EventTime(ms * 1_000_000),
                kind: EventKind::Key {
                    device: DeviceId::Keyboard(0),
                    code,
                    symbol: 0,
                    down,
                    repeat: false,
                },
            },
            &mut sink,
        );
    }
    assert_eq!(
        input.build_frame_with_policy(
            EventTime(40_000_000),
            &[policy; qa_core::sys_events::SeatId::COUNT]
        )[0]
        .movement[0],
        127.0
    );
    input.dispatch(
        SysEvent {
            time: EventTime(45_000_000),
            kind: EventKind::MouseWheel {
                device: DeviceId::Mouse(0),
                x: 0,
                y: i32::MAX,
            },
        },
        &mut sink,
    );
    assert_eq!(sink.0.len(), 5);
    input.dispatch(
        SysEvent {
            time: EventTime(50_000_000),
            kind: EventKind::Focus(false),
        },
        &mut sink,
    );
    assert_eq!(
        input.build_frame_with_policy(
            EventTime(60_000_000),
            &[policy; qa_core::sys_events::SeatId::COUNT]
        )[0]
        .movement,
        [0.0; 3]
    );
}
