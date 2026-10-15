#[path = "support/console.rs"]
mod support;

use qa_console::{
    command_text::Arguments,
    commands::{CommandError, Console},
    views::Context,
};
use qa_core::{
    primitives::{ModuleId, RuleSetId},
    sys_events::*,
};
use qa_input::{DispatchCursor, DispatchStep, Input, InputAtom, InputCapture, Target, keys};

struct Sink {
    router: support::Router,
}
impl Default for Sink {
    fn default() -> Self {
        Self {
            router: support::Router::default(),
        }
    }
}
impl Target for Sink {
    fn character(&mut self, _: SeatId, _: char) {}
    fn command(
        &mut self,
        seat: SeatId,
        time: EventTime,
        source: Option<KeySource>,
        text: &str,
    ) -> bool {
        self.command_bytes(seat, time, source, text.as_bytes())
    }
    fn command_bytes(
        &mut self,
        seat: SeatId,
        time: EventTime,
        source: Option<KeySource>,
        text: &[u8],
    ) -> bool {
        self.router.command_bytes(seat, time, source, text)
    }
    fn cancel_commands(&mut self, source: KeySource) {
        self.router.console.discard_source(source);
    }
}
fn event(ms: u64, kind: EventKind<'_>) -> SysEvent<'_> {
    SysEvent {
        time: EventTime::from_milliseconds(ms),
        kind,
    }
}
fn key(ms: u64, device: DeviceId, code: u16, down: bool, repeat: bool) -> SysEvent<'static> {
    event(
        ms,
        EventKind::Key {
            device,
            code,
            symbol: 0,
            down,
            repeat,
        },
    )
}
fn route(
    input: &mut Input,
    event: SysEvent<'_>,
    sink: &mut Sink,
    mut callback: impl FnMut(&mut Input, InputAtom, &mut Sink) -> InputCapture,
) {
    let mut cursor = DispatchCursor::new(event);
    loop {
        let DispatchStep::Atom { atom, resume } = input.advance(cursor, sink) else {
            break;
        };
        let captured = callback(input, atom, sink);
        cursor = input.resume(resume, captured, sink);
    }
}

#[test]
fn current_physical_state_and_acquired_release_precede_routing() {
    let mut input = Input::load();
    let mut sink = Sink::default();
    input
        .bind_text(26, "+edge", EventTime(0), &mut sink)
        .unwrap();
    let device = DeviceId::Keyboard(17);
    let mut calls = 0;
    for (ms, down, repeat) in [
        (10, true, false),
        (11, true, true),
        (20, false, false),
        (21, false, false),
    ] {
        route(
            &mut input,
            key(ms, device, 26, down, repeat),
            &mut sink,
            |input, atom, sink| {
                let InputAtom::Key {
                    seat,
                    source,
                    down: routed,
                    repeat: repeated,
                    time,
                } = atom
                else {
                    panic!("key")
                };
                assert_eq!(
                    (source, routed, repeated, time),
                    (
                        KeySource {
                            device,
                            control: 26
                        },
                        down,
                        repeat,
                        EventTime::from_milliseconds(ms)
                    )
                );
                assert_eq!(input.is_down(seat, 26), down);
                if ms == 20 {
                    assert!(!sink.router.console.module_parts().1.is_empty());
                }
                calls += 1;
                InputCapture::Pass
            },
        );
        sink.router.execute(&mut input);
    }
    assert_eq!(calls, 4);
    assert_eq!(
        (sink.router.host.presses, sink.router.host.releases),
        (1, 1)
    );
    let context = sink.router.host.last_context.unwrap();
    assert_eq!(
        context.key_source,
        Some(KeySource {
            device,
            control: 26
        })
    );
    assert_eq!(context.event_time, Some(EventTime::from_milliseconds(20)));
}

#[test]
fn pending_press_cannot_acquire_replacement_after_rebind_or_clear_states() {
    for clear in [false, true] {
        let mut input = Input::load();
        let mut sink = Sink::default();
        input
            .bind_text(26, "+edge", EventTime(0), &mut sink)
            .unwrap();
        route(
            &mut input,
            key(10, DeviceId::Keyboard(0), 26, true, false),
            &mut sink,
            |input, _, sink| {
                if clear {
                    input.clear_seat(SeatId::FIRST, EventTime::from_milliseconds(10), sink);
                } else {
                    input
                        .bind_text(
                            26,
                            "+edge; echo replacement",
                            EventTime::from_milliseconds(10),
                            sink,
                        )
                        .unwrap();
                    assert!(input.is_down(SeatId::FIRST, 26));
                }
                InputCapture::Pass
            },
        );
        sink.router.execute(&mut input);
        assert_eq!(
            (sink.router.host.presses, sink.router.host.releases),
            (0, 0)
        );
        input.dispatch(key(11, DeviceId::Keyboard(0), 26, false, false), &mut sink);
        input.dispatch(key(12, DeviceId::Keyboard(0), 26, true, false), &mut sink);
        input.dispatch(key(13, DeviceId::Keyboard(0), 26, false, false), &mut sink);
        sink.router.execute(&mut input);
        assert_eq!(
            (sink.router.host.presses, sink.router.host.releases),
            (1, 1)
        );
    }
}

#[test]
fn unrelated_unchanged_and_rejected_binding_changes_keep_pending_press() {
    for change in 0..3 {
        let mut input = Input::load();
        let mut sink = Sink::default();
        input
            .bind_text(26, "+edge", EventTime(0), &mut sink)
            .unwrap();
        route(
            &mut input,
            key(10, DeviceId::Keyboard(0), 26, true, false),
            &mut sink,
            |input, _, sink| {
                match change {
                    0 => input.bind_text(22, "+edge", EventTime(0), sink).unwrap(),
                    1 => input.bind_text(26, "+edge", EventTime(0), sink).unwrap(),
                    _ => assert!(
                        input
                            .bind_bytes(26, &[b'x'; 1025], EventTime(0), sink)
                            .is_err()
                    ),
                }
                InputCapture::Pass
            },
        );
        input.dispatch(key(20, DeviceId::Keyboard(0), 26, false, false), &mut sink);
        sink.router.execute(&mut input);
        assert_eq!(
            (sink.router.host.presses, sink.router.host.releases),
            (1, 1)
        );
    }
}

#[test]
fn caught_press_and_repeat_never_acquire_until_fresh_down() {
    let mut input = Input::load();
    let mut sink = Sink::default();
    input
        .bind_text(26, "+edge", EventTime(0), &mut sink)
        .unwrap();
    route(
        &mut input,
        key(10, DeviceId::Keyboard(0), 26, true, false),
        &mut sink,
        |_, _, _| InputCapture::Caught,
    );
    input.dispatch(key(11, DeviceId::Keyboard(0), 26, true, true), &mut sink);
    input.dispatch(key(12, DeviceId::Keyboard(0), 26, false, false), &mut sink);
    sink.router.execute(&mut input);
    assert_eq!(
        (sink.router.host.presses, sink.router.host.releases),
        (0, 0)
    );
    input.dispatch(key(13, DeviceId::Keyboard(0), 26, true, false), &mut sink);
    sink.router.execute(&mut input);
    assert_eq!(sink.router.host.presses, 1);
}

#[test]
fn wheel_and_text_resume_in_order_with_current_text_seat() {
    let mut input = Input::load();
    let mut sink = Sink::default();
    let mut count = 0;
    route(
        &mut input,
        event(
            10,
            EventKind::MouseWheel {
                device: DeviceId::Mouse(0),
                x: i32::MIN,
                y: i32::MAX,
            },
        ),
        &mut sink,
        |input, atom, _| {
            let InputAtom::Key {
                seat, source, down, ..
            } = atom
            else {
                panic!("wheel key")
            };
            assert_eq!(
                (source.control, down),
                [(576, true), (576, false), (578, true), (578, false)][count]
            );
            assert_eq!(input.is_down(seat, source.control), down);
            count += 1;
            InputCapture::Caught
        },
    );
    assert_eq!(count, 4);
    let mut text = String::new();
    route(
        &mut input,
        event(
            20,
            EventKind::Text {
                device: DeviceId::Keyboard(0),
                text: "aλb",
            },
        ),
        &mut sink,
        |input, atom, _| {
            let InputAtom::Character { seat, value, .. } = atom else {
                panic!("character")
            };
            assert_eq!(
                seat,
                if text.is_empty() {
                    SeatId::FIRST
                } else {
                    SeatId::ALL[1]
                }
            );
            text.push(value);
            input.set_text_seat(SeatId::ALL[1]);
            InputCapture::Caught
        },
    );
    assert_eq!(text, "aλb");
}

#[test]
fn caught_mouse_does_not_also_feed_the_shared_command_builder() {
    for capture in [InputCapture::Pass, InputCapture::Caught] {
        let mut input = Input::load();
        let mut legacy = Input::load();
        let mut sink = Sink::default();
        input.seed(EventTime(0));
        legacy.seed(EventTime(0));
        let mouse = event(
            10,
            EventKind::Mouse {
                device: DeviceId::Mouse(0),
                dx: 20,
                dy: 10,
            },
        );
        let mut calls = 0;
        route(&mut input, mouse, &mut sink, |_, atom, _| {
            assert!(matches!(atom, InputAtom::Mouse { dx: 20, dy: 10, .. }));
            calls += 1;
            capture
        });
        assert_eq!(calls, 1);
        if capture == InputCapture::Pass {
            legacy.dispatch(mouse, &mut sink);
        }
        let policies = [sink.router.policy(RuleSetId::Quake3); SeatId::COUNT];
        let at = EventTime::from_milliseconds(20);
        let command = input.build_frame_with_policy(at, &policies)[0];
        let expected = legacy.build_frame_with_policy(at, &policies)[0];
        assert_eq!(
            command.view_angles.0.map(f32::to_bits),
            expected.view_angles.0.map(f32::to_bits)
        );
        assert_eq!(
            command.view_angles.0.iter().any(|&v| v != 0.0),
            capture == InputCapture::Pass
        );
    }
}

#[test]
fn capture_then_focus_release_current_holds_in_device_and_control_order() {
    let mut input = Input::load();
    let mut sink = Sink::default();
    assert!(input.assign(DeviceId::Keyboard(9), SeatId::ALL[1]));
    for control in [26, 27, 513] {
        input
            .bind_text(control, "+edge", EventTime(0), &mut sink)
            .unwrap();
    }
    for (device, control) in [
        (DeviceId::Keyboard(0), 26),
        (DeviceId::Mouse(0), 513),
        (DeviceId::Keyboard(9), 27),
    ] {
        input.dispatch(key(1, device, control, true, false), &mut sink);
    }
    sink.router.execute(&mut input);
    let mut mouse_ups = 0;
    route(
        &mut input,
        event(2, EventKind::MouseCapture(false)),
        &mut sink,
        |input, atom, _| {
            let InputAtom::Key {
                seat,
                source,
                down: false,
                ..
            } = atom
            else {
                panic!("release")
            };
            assert_eq!(
                source,
                KeySource {
                    device: DeviceId::Mouse(0),
                    control: 513
                }
            );
            assert!(!input.is_down(seat, source.control));
            mouse_ups += 1;
            InputCapture::Caught
        },
    );
    assert_eq!(mouse_ups, 1);
    assert!(input.is_down(SeatId::FIRST, 26));
    let mut keyboard_ups = 0;
    route(
        &mut input,
        event(3, EventKind::Focus(false)),
        &mut sink,
        |input, atom, _| {
            let InputAtom::Key {
                seat,
                source,
                down: false,
                ..
            } = atom
            else {
                panic!("release")
            };
            assert_eq!(
                source,
                [
                    KeySource {
                        device: DeviceId::Keyboard(0),
                        control: 26
                    },
                    KeySource {
                        device: DeviceId::Keyboard(9),
                        control: 27
                    }
                ][keyboard_ups]
            );
            assert!(!input.is_down(seat, source.control));
            keyboard_ups += 1;
            InputCapture::Caught
        },
    );
    assert_eq!(keyboard_ups, 2);
    sink.router.execute(&mut input);
    assert_eq!(
        (sink.router.host.presses, sink.router.host.releases),
        (3, 3)
    );
    assert!(input.assign(DeviceId::Keyboard(0), SeatId::FIRST));
    assert!(input.assign(DeviceId::Keyboard(9), SeatId::ALL[1]));
}

#[test]
fn seat_clear_and_owned_catchers_preserve_other_devices_and_owners() {
    let mut input = Input::load();
    let mut sink = Sink::default();
    let second = SeatId::ALL[1];
    assert!(input.assign(DeviceId::Keyboard(22), second));
    input
        .bind_text(224, "+edge", EventTime(0), &mut sink)
        .unwrap();
    for device in [DeviceId::Keyboard(0), DeviceId::Keyboard(22)] {
        input.dispatch(key(1, device, 228, true, false), &mut sink);
    }
    sink.router.execute(&mut input);
    input.clear_seat(second, EventTime::from_milliseconds(2), &mut sink);
    sink.router.execute(&mut input);
    assert!(input.is_down(SeatId::FIRST, 224));
    assert!(!input.is_down(second, 224));
    assert_eq!(
        (sink.router.host.presses, sink.router.host.releases),
        (2, 1)
    );
    let owner = ModuleId(7);
    input.replace_owned_key_catchers(SeatId::FIRST, key_catchers::UI, owner, EventTime(0));
    input.set_catcher(SeatId::FIRST, key_catchers::CONSOLE, true, EventTime(0));
    input.set_catcher(SeatId::FIRST, key_catchers::UI, true, EventTime(0));
    assert_eq!(
        input.catcher_module(SeatId::FIRST, key_catchers::UI),
        Some(owner)
    );
    input.replace_owned_key_catchers(second, key_catchers::CGAME, ModuleId(8), EventTime(0));
    input.retire_catcher_module(owner, EventTime(0));
    assert_eq!(input.key_catchers(SeatId::FIRST), key_catchers::CONSOLE);
    assert_eq!(
        input.catcher_module(second, key_catchers::CGAME),
        Some(ModuleId(8))
    );
    assert_eq!(
        input.catcher_module(second, key_catchers::UI | key_catchers::CGAME),
        None
    );
}

fn raw(
    _: &mut Console<support::Capture>,
    host: &mut support::Capture,
    args: &Arguments<'_>,
    _: Context,
) -> Result<(), CommandError> {
    host.raw.set_bytes(args.get_bytes(1)).unwrap();
    host.raw_calls += 1;
    Ok(())
}
#[test]
fn native_raw_binding_uses_existing_console_and_preserves_empty_vs_absent() {
    let mut input = Input::load();
    let mut sink = Sink::default();
    assert!(sink.router.console.register("raw", raw));
    let text = b"raw \"a\xff;b\"; +edge";
    input.bind_bytes(26, text, EventTime(0), &mut sink).unwrap();
    assert_eq!(input.binding(26).unwrap().bytes(), text);
    input.dispatch(key(10, DeviceId::Keyboard(0), 26, true, false), &mut sink);
    input.dispatch(key(20, DeviceId::Keyboard(0), 26, false, false), &mut sink);
    sink.router.execute(&mut input);
    assert_eq!(sink.router.host.raw.as_bytes(), b"a\xff;b");
    assert_eq!(sink.router.host.raw_calls, 1);
    assert_eq!(
        (sink.router.host.presses, sink.router.host.releases),
        (1, 1)
    );
    input.bind_bytes(26, b"", EventTime(0), &mut sink).unwrap();
    assert_eq!(input.binding(26).unwrap().bytes(), b"");
    input.bind_text(26, "", EventTime(0), &mut sink).unwrap();
    assert!(input.binding(26).is_none());
}

#[test]
fn native_slots_round_trip_without_case_or_shifted_symbol_aliases() {
    let mut occupied = [false; 1024];
    for code in 0..=u8::MAX {
        let control = keys::from_native(code).unwrap();
        assert_eq!(keys::native_number(control), Some(code));
        assert!(!std::mem::replace(
            &mut occupied[usize::from(control)],
            true
        ));
        assert_eq!(keys::parse(&format!("0x{code:02x}")), Some(control));
    }
    assert_ne!(keys::from_native(65), keys::from_native(97));
    assert_ne!(keys::from_native(33), keys::from_native(49));
    assert_eq!(keys::parse("W"), keys::parse("w"));
    assert_ne!(keys::parse("W"), keys::parse("0x57"));
}
