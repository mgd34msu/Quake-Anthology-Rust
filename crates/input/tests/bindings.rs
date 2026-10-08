use qa_core::sys_events::{DeviceId, EventKind, EventTime, SeatId, SysEvent};
use qa_input::{Input, Target, keys};
#[derive(Default)]
struct Sink(Vec<(SeatId, String)>);
impl Target for Sink {
    fn character(&mut self, _: SeatId, _: char) {}
    fn command(&mut self, seat: SeatId, _: EventTime, text: &str) {
        self.0.push((seat, text.to_owned()));
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
    let mut sink = Sink::default();
    input
        .bind_text(10, "+edge; echo \"a;b\"; +jump", EventTime(0), &mut sink)
        .unwrap();
    assert!(
        input
            .bind_text(10, &"x".repeat(1025), EventTime(0), &mut sink)
            .is_err()
    );
    for (ms, down, repeat) in [(10, true, false), (15, true, true), (20, false, false)] {
        input.dispatch(
            SysEvent {
                time: EventTime(ms * 1_000_000),
                kind: EventKind::Key {
                    device: DeviceId::Keyboard,
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
            (SeatId::FIRST, "-edge 10 20".to_owned()),
            (SeatId::FIRST, "echo \"a;b\"".to_owned())
        ]
    );
    assert_ne!(
        input.build_frame(EventTime(30_000_000), [127; 3], [0.022; 2], [None; 4])[0].buttons
            & qa_core::primitives::buttons::JUMP,
        0
    );
}
#[test]
fn both_ctrl_keys_hold_one_action_and_wheel_axes_dispatch_momentary_commands() {
    let mut input = Input::load();
    let mut sink = Sink::default();
    input
        .bind_text(224, "+forward", EventTime(0), &mut sink)
        .unwrap();
    input
        .bind_text(576, "+edge", EventTime(0), &mut sink)
        .unwrap();
    for (ms, code, down) in [(0, 224, true), (10, 228, true), (20, 224, false)] {
        input.dispatch(
            SysEvent {
                time: EventTime(ms * 1_000_000),
                kind: EventKind::Key {
                    device: DeviceId::Keyboard,
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
        input.build_frame(EventTime(40_000_000), [127; 3], [0.022; 2], [None; 4])[0].movement[0],
        127
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
    assert_eq!(sink.0.len(), 2);
    input.dispatch(
        SysEvent {
            time: EventTime(50_000_000),
            kind: EventKind::Focus(false),
        },
        &mut sink,
    );
    assert_eq!(
        input.build_frame(EventTime(60_000_000), [127; 3], [0.022; 2], [None; 4])[0].movement,
        [0; 3]
    );
}
