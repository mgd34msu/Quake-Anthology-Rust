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
    release_lines: u32,
    last_producer: Option<qa_core::primitives::CommandProducer>,
}
impl Default for Sink {
    fn default() -> Self {
        Self {
            router: support::Router::default(),
            release_lines: 0,
            last_producer: None,
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
        producer: Option<qa_core::primitives::CommandProducer>,
        text: &str,
    ) -> bool {
        self.command_bytes(seat, time, source, producer, text.as_bytes())
    }
    fn command_bytes(
        &mut self,
        seat: SeatId,
        time: EventTime,
        source: Option<KeySource>,
        producer: Option<qa_core::primitives::CommandProducer>,
        text: &[u8],
    ) -> bool {
        self.release_lines += u32::from(text.starts_with(b"-"));
        self.last_producer = producer;
        self.router
            .command_bytes(seat, time, source, producer, text)
    }
    fn cancel_commands(
        &mut self,
        source: KeySource,
        producer: Option<qa_core::primitives::CommandProducer>,
    ) {
        self.router.console.discard_acquisition(source, producer);
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
    callback: impl FnMut(&mut Input, InputAtom, &mut Sink) -> InputCapture,
) {
    walk(input, DispatchCursor::new(event), sink, callback);
}
fn walk(
    input: &mut Input,
    mut cursor: DispatchCursor<'_>,
    sink: &mut Sink,
    mut callback: impl FnMut(&mut Input, InputAtom, &mut Sink) -> InputCapture,
) {
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
            (0, u32::from(clear))
        );
        input.dispatch(key(11, DeviceId::Keyboard(0), 26, false, false), &mut sink);
        input.dispatch(key(12, DeviceId::Keyboard(0), 26, true, false), &mut sink);
        input.dispatch(key(13, DeviceId::Keyboard(0), 26, false, false), &mut sink);
        sink.router.execute(&mut input);
        assert_eq!(
            (sink.router.host.presses, sink.router.host.releases),
            (1, 1 + u32::from(clear))
        );
    }
}

#[test]
fn native_clear_orders_keys_and_releases_all_holders_before_each_zero_time_callback() {
    let mut input = Input::load();
    let mut sink = Sink::default();
    let first = SeatId::FIRST;
    let second = SeatId::ALL[1];
    let keyboard = DeviceId::Keyboard(0);
    let extra = DeviceId::Keyboard(9);
    let other = DeviceId::Keyboard(22);
    assert!(input.assign(extra, first));
    assert!(input.assign(other, second));
    for control in [4, 30, 224, 905, 70] {
        input
            .bind_text(control, "+edge", EventTime(0), &mut sink)
            .unwrap();
    }
    for (device, control) in [
        (keyboard, 4),
        (keyboard, 30),
        (keyboard, 224),
        (keyboard, 228),
        (keyboard, 905),
        (extra, 224),
        (keyboard, 70),
        (other, 4),
    ] {
        input.dispatch(key(10, device, control, true, false), &mut sink);
    }
    sink.router.execute(&mut input);
    assert_eq!(sink.router.host.presses, 8);
    let mut callbacks = Vec::new();
    walk(
        &mut input,
        DispatchCursor::clear_seat(first, EventTime::from_milliseconds(123)),
        &mut sink,
        |input, atom, sink| {
            let InputAtom::Key {
                seat,
                source,
                time,
                down,
                repeat,
            } = atom
            else {
                panic!("key-up")
            };
            assert_eq!((seat, down, repeat), (first, false, false));
            assert!(!input.is_down(seat, source.control));
            let native = keys::native_number(source.control);
            let releases = match native {
                Some(49) => 1,
                Some(97) => 2,
                Some(137) => {
                    assert!(!input.is_down(first, 224));
                    assert!(!input.is_down(first, 905));
                    6
                }
                None => 7,
                _ => panic!("unexpected key"),
            };
            assert_eq!(sink.release_lines, releases);
            assert_eq!(
                time,
                if native.is_some() {
                    EventTime(0)
                } else {
                    EventTime::from_milliseconds(123)
                }
            );
            // The admitted bind release is visible to the synchronous guest callback.
            sink.router.execute(input);
            assert_eq!(sink.router.host.releases, releases);
            assert_eq!(
                sink.router.host.last_context.unwrap().event_time,
                Some(time)
            );
            callbacks.push(native);
            InputCapture::Caught
        },
    );
    assert_eq!(callbacks, [Some(49), Some(97), Some(137), None]);
    assert!(input.is_down(second, 4));
    assert_eq!(sink.router.host.releases, 7);
}

#[test]
fn native_clear_reentry_skips_released_keys_and_releases_a_caught_current_binding() {
    let mut input = Input::load();
    let mut sink = Sink::default();
    for control in [4, 30] {
        input
            .bind_text(control, "+edge; raw tail", EventTime(0), &mut sink)
            .unwrap();
    }
    assert!(sink.router.console.register("raw", raw));
    for control in [4, 30] {
        route(
            &mut input,
            key(10, DeviceId::Keyboard(0), control, true, false),
            &mut sink,
            |_, _, _| InputCapture::Caught,
        );
    }
    sink.router.execute(&mut input);
    assert_eq!(sink.router.host.presses, 0);
    let mut callbacks = Vec::new();
    walk(
        &mut input,
        DispatchCursor::clear_seat(SeatId::FIRST, EventTime(99)),
        &mut sink,
        |input, atom, sink| {
            let InputAtom::Key { source, time, .. } = atom else {
                panic!("key-up")
            };
            callbacks.push(keys::native_number(source.control).unwrap());
            assert_eq!(time, EventTime(0));
            assert_eq!(sink.release_lines, 1);
            walk(
                input,
                DispatchCursor::clear_seat(SeatId::FIRST, EventTime(101)),
                sink,
                |input, atom, sink| {
                    let InputAtom::Key { source, time, .. } = atom else {
                        panic!("key-up")
                    };
                    callbacks.push(keys::native_number(source.control).unwrap());
                    assert_eq!(time, EventTime(0));
                    assert!(!input.is_down(SeatId::FIRST, source.control));
                    assert_eq!(sink.release_lines, 2);
                    InputCapture::Caught
                },
            );
            InputCapture::Caught
        },
    );
    assert_eq!(callbacks, [49, 97]);
    sink.router.execute(&mut input);
    assert_eq!(
        (sink.router.host.presses, sink.router.host.releases),
        (0, 2)
    );
    assert_eq!(sink.router.host.raw_calls, 2);
    assert_eq!(sink.router.host.raw.as_bytes(), b"tail");
}

#[test]
fn native_clear_suppresses_console_key_ups_and_their_unacquired_bindings() {
    let mut input = Input::load();
    let mut sink = Sink::default();
    for native in [b'`', b'~'] {
        let control = keys::from_native(native).unwrap();
        input
            .bind_text(control, "+edge", EventTime(0), &mut sink)
            .unwrap();
        route(
            &mut input,
            key(10, DeviceId::Keyboard(0), control, true, false),
            &mut sink,
            |_, _, _| InputCapture::Caught,
        );
    }
    walk(
        &mut input,
        DispatchCursor::clear_seat(SeatId::FIRST, EventTime(99)),
        &mut sink,
        |_, _, _| panic!("console key must not reach the module catcher"),
    );
    for native in [b'`', b'~'] {
        assert!(!input.is_down(SeatId::FIRST, keys::from_native(native).unwrap()));
    }
    assert_eq!(sink.release_lines, 0);
}

#[test]
fn native_clear_forces_one_canonical_binding_only_without_an_acquired_alias() {
    for acquired in [None, Some(224), Some(228)] {
        let mut input = Input::load();
        let mut sink = Sink::default();
        assert!(sink.router.console.register("raw", raw));
        input
            .bind_text(224, "+edge; raw tail", EventTime(0), &mut sink)
            .unwrap();
        for control in [224, 228] {
            route(
                &mut input,
                key(10, DeviceId::Keyboard(0), control, true, false),
                &mut sink,
                |_, _, _| {
                    if acquired == Some(control) {
                        InputCapture::Pass
                    } else {
                        InputCapture::Caught
                    }
                },
            );
        }
        sink.router.execute(&mut input);
        sink.router.host.raw_calls = 0;
        let mut callbacks = 0;
        walk(
            &mut input,
            DispatchCursor::clear_seat(SeatId::FIRST, EventTime(99)),
            &mut sink,
            |input, atom, sink| {
                let InputAtom::Key { source, .. } = atom else {
                    panic!("key-up")
                };
                assert_eq!(keys::native_number(source.control), Some(137));
                assert!(!input.is_down(SeatId::FIRST, 224));
                assert_eq!(sink.release_lines, 1);
                callbacks += 1;
                InputCapture::Caught
            },
        );
        assert_eq!(callbacks, 1);
        sink.router.execute(&mut input);
        assert_eq!(sink.router.host.releases, 1);
        assert_eq!(sink.router.host.raw_calls, 1);
        assert_eq!(sink.router.host.raw.as_bytes(), b"tail");
    }
    let mut input = Input::load();
    let mut sink = Sink::default();
    assert!(sink.router.console.register("raw", raw));
    input
        .bind_text(224, "+edge; raw canonical", EventTime(0), &mut sink)
        .unwrap();
    input
        .bind_text(905, "+edge; raw alias", EventTime(0), &mut sink)
        .unwrap();
    route(
        &mut input,
        key(10, DeviceId::Keyboard(0), 905, true, false),
        &mut sink,
        |_, _, _| InputCapture::Caught,
    );
    input.clear_seat(SeatId::FIRST, EventTime(99), &mut sink);
    sink.router.execute(&mut input);
    assert_eq!(sink.router.host.releases, 1);
    assert_eq!(sink.router.host.raw_calls, 1);
    assert_eq!(sink.router.host.raw.as_bytes(), b"canonical");
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

fn producer(module: u16, generation: u32) -> qa_core::primitives::CommandProducer {
    qa_core::primitives::CommandProducer {
        module: ModuleId(module),
        anchor: qa_core::primitives::EntityId {
            slot: 3,
            generation,
        },
    }
}

#[test]
fn scoped_alias_binding_keeps_its_definition_owner_at_the_physical_press() {
    let mut input = Input::load();
    let mut sink = Sink::default();
    let owners = [producer(7, 1), producer(8, 1)];
    sink.router.host.producers = owners.map(Some);
    for (owner, text) in [
        (owners[0], "alias fire +edge; bind w fire"),
        (owners[1], "alias fire -edge; bind s fire"),
    ] {
        sink.router
            .console
            .append(
                text,
                Context {
                    producer: Some(owner),
                    ..Context::default()
                },
            )
            .unwrap();
        sink.router.execute(&mut input);
    }
    for (control, owner) in [(26, owners[0]), (22, owners[1])] {
        assert_eq!(input.binding(control).unwrap().producer(), Some(owner));
        input.dispatch(
            key(10, DeviceId::Keyboard(0), control, true, false),
            &mut sink,
        );
        assert_eq!(sink.last_producer, Some(owner));
        sink.router.execute(&mut input);
        assert_eq!(sink.router.host.last_context.unwrap().producer, Some(owner));
        input.dispatch(
            key(20, DeviceId::Keyboard(0), control, false, false),
            &mut sink,
        );
    }
    assert_eq!(
        (sink.router.host.presses, sink.router.host.releases),
        (1, 1)
    );
}

#[test]
fn identical_bytes_with_a_new_owner_release_the_acquired_old_definition_once() {
    let mut input = Input::load();
    let mut sink = Sink::default();
    let old = producer(7, 1);
    let new = producer(7, 2);
    sink.router.host.producers = [Some(old), Some(new)];
    input
        .bind_bytes_owned(26, b"+edge", Some(old), EventTime(0), &mut sink)
        .unwrap();
    input.dispatch(key(10, DeviceId::Keyboard(0), 26, true, false), &mut sink);
    sink.router.execute(&mut input);
    input
        .bind_bytes_owned(
            26,
            b"+edge",
            Some(new),
            EventTime::from_milliseconds(20),
            &mut sink,
        )
        .unwrap();
    assert_eq!(sink.last_producer, Some(old));
    assert_eq!(input.binding(26).unwrap().producer(), Some(new));
    sink.router.execute(&mut input);
    assert_eq!(sink.router.host.last_context.unwrap().producer, Some(old));
    assert_eq!(
        (sink.router.host.presses, sink.router.host.releases),
        (1, 1)
    );
    for (ms, down, repeat) in [(21, true, true), (22, false, false), (23, true, false)] {
        input.dispatch(key(ms, DeviceId::Keyboard(0), 26, down, repeat), &mut sink);
    }
    sink.router.execute(&mut input);
    assert_eq!(sink.router.host.last_context.unwrap().producer, Some(new));
    assert_eq!(
        (sink.router.host.presses, sink.router.host.releases),
        (2, 1)
    );
    assert_eq!(sink.release_lines, 1);
    // A human replacement of identical bytes must also replace ownership.
    input
        .bind_bytes(26, b"+edge", EventTime::from_milliseconds(24), &mut sink)
        .unwrap();
    assert_eq!(input.binding(26).unwrap().producer(), None);
    assert_eq!(sink.last_producer, Some(new));
}

#[test]
fn retirement_is_exact_in_generation_and_retention_preserves_human_and_other_owner() {
    let mut input = Input::load();
    let mut sink = Sink::default();
    let old = producer(7, 1);
    let reused = producer(7, 2);
    let other = producer(8, 1);
    for (control, owner) in [
        (26, Some(old)),
        (22, Some(reused)),
        (10, Some(other)),
        (11, None),
    ] {
        input
            .bind_bytes_owned(control, b"+edge", owner, EventTime(0), &mut sink)
            .unwrap();
    }
    input.dispatch(key(10, DeviceId::Keyboard(17), 26, true, false), &mut sink);
    input.retire_bindings(reused, EventTime::from_milliseconds(20), &mut sink);
    assert!(input.binding(22).is_none());
    assert_eq!(input.binding(26).unwrap().producer(), Some(old));
    assert_eq!(sink.release_lines, 0);
    input.retain_bindings(
        |owner| owner == other,
        EventTime::from_milliseconds(30),
        &mut sink,
    );
    assert!(input.binding(26).is_none());
    assert!(input.binding(22).is_none());
    assert_eq!(input.binding(10).unwrap().producer(), Some(other));
    assert_eq!(input.binding(11).unwrap().producer(), None);
    assert_eq!((sink.release_lines, sink.last_producer), (1, Some(old)));
    assert!(input.is_down(SeatId::FIRST, 26));
    input.dispatch(key(40, DeviceId::Keyboard(17), 26, false, false), &mut sink);
    input.retire_bindings(old, EventTime::from_milliseconds(50), &mut sink);
    assert_eq!(sink.release_lines, 1);
}

#[test]
fn retirement_cancels_a_dead_owned_hold_without_clearing_a_human_holder() {
    let mut input = Input::load();
    let mut sink = Sink::default();
    let owner = producer(7, 1);
    sink.router.host.producers = [Some(owner), None];
    input.seed(EventTime(0));
    input
        .bind_bytes_owned(26, b"+forward", Some(owner), EventTime(0), &mut sink)
        .unwrap();
    input
        .bind_bytes(10, b"+forward", EventTime(0), &mut sink)
        .unwrap();
    for (device, control) in [(DeviceId::Keyboard(17), 26), (DeviceId::Keyboard(18), 10)] {
        input.dispatch(key(10, device, control, true, false), &mut sink);
    }
    sink.router.execute(&mut input);
    let mut policy = sink.router.policy(RuleSetId::Quake2);
    policy.speed = [100.0; 3];
    policy.move_multiplier = 1.0;
    input.build_frame_with_policy(EventTime::from_milliseconds(20), &[policy; SeatId::COUNT]);
    sink.router.host.producers[0] = None;
    input.retire_bindings(owner, EventTime::from_milliseconds(30), &mut sink);
    sink.router.execute(&mut input);
    assert_eq!(
        input.build_frame_with_policy(EventTime::from_milliseconds(40), &[policy; SeatId::COUNT])
            [0]
        .movement[0],
        100.0
    );
    input.dispatch(key(50, DeviceId::Keyboard(18), 10, false, false), &mut sink);
    sink.router.execute(&mut input);
    input.build_frame_with_policy(EventTime::from_milliseconds(60), &[policy; SeatId::COUNT]);
    assert_eq!(
        input.build_frame_with_policy(EventTime::from_milliseconds(80), &[policy; SeatId::COUNT])
            [0]
        .movement[0],
        0.0
    );
    assert_eq!(sink.release_lines, 2);
}

#[test]
fn retiring_one_owner_preserves_another_holder_on_the_same_physical_key() {
    let mut input = Input::load();
    let mut sink = Sink::default();
    let owner = producer(7, 1);
    sink.router.host.producers = [Some(owner), None];
    input.seed(EventTime(0));
    let source = KeySource {
        device: DeviceId::Keyboard(17),
        control: 26,
    };
    for producer in [Some(owner), None] {
        assert!(sink.router.command(
            SeatId::FIRST,
            EventTime(0),
            Some(source),
            producer,
            "+forward"
        ));
    }
    sink.router.execute(&mut input);
    let mut policy = sink.router.policy(RuleSetId::Quake2);
    policy.speed = [100.0; 3];
    policy.move_multiplier = 1.0;
    input.build_frame_with_policy(EventTime::from_milliseconds(10), &[policy; SeatId::COUNT]);
    input.retire_bindings(owner, EventTime::from_milliseconds(20), &mut sink);
    assert_eq!(
        input.build_frame_with_policy(EventTime::from_milliseconds(30), &[policy; SeatId::COUNT])
            [0]
        .movement[0],
        100.0
    );
    assert!(sink.router.command(
        SeatId::FIRST,
        EventTime::from_milliseconds(40),
        Some(source),
        None,
        "-forward"
    ));
    sink.router.execute(&mut input);
    input.build_frame_with_policy(EventTime::from_milliseconds(50), &[policy; SeatId::COUNT]);
    assert_eq!(
        input.build_frame_with_policy(EventTime::from_milliseconds(60), &[policy; SeatId::COUNT])
            [0]
        .movement[0],
        0.0
    );
}

#[test]
fn retirement_clears_an_owned_action_when_key_up_release_is_still_queued() {
    let mut input = Input::load();
    let mut sink = Sink::default();
    let owner = producer(7, 1);
    sink.router.host.producers = [Some(owner), None];
    input.seed(EventTime(0));
    input
        .bind_bytes_owned(26, b"+forward", Some(owner), EventTime(0), &mut sink)
        .unwrap();
    input.dispatch(key(10, DeviceId::Keyboard(17), 26, true, false), &mut sink);
    sink.router.execute(&mut input);
    let mut policy = sink.router.policy(RuleSetId::Quake2);
    policy.speed = [100.0; 3];
    policy.move_multiplier = 1.0;
    input.build_frame_with_policy(EventTime::from_milliseconds(20), &[policy; SeatId::COUNT]);
    input.dispatch(key(30, DeviceId::Keyboard(17), 26, false, false), &mut sink);
    assert!(!input.is_down(SeatId::FIRST, 26));
    sink.router.host.producers[0] = None;
    input.retain_producers(|_| false, EventTime::from_milliseconds(40), &mut sink);
    sink.router.execute(&mut input);
    input.build_frame_with_policy(EventTime::from_milliseconds(50), &[policy; SeatId::COUNT]);
    assert_eq!(
        input.build_frame_with_policy(EventTime::from_milliseconds(60), &[policy; SeatId::COUNT])
            [0]
        .movement[0],
        0.0
    );
    assert_eq!(sink.release_lines, 1);
}

#[test]
fn owned_manual_release_and_retirement_use_the_exact_module_lifetime() {
    let mut input = Input::load();
    let mut sink = Sink::default();
    let old = producer(7, 1);
    let replacement = producer(7, 2);
    sink.router.host.producers = [Some(old), Some(replacement)];
    input.seed(EventTime(0));
    for owner in [old, replacement] {
        assert!(
            sink.router
                .command(SeatId::FIRST, EventTime(0), None, Some(owner), "+forward")
        );
    }
    sink.router.execute(&mut input);
    let mut policy = sink.router.policy(RuleSetId::Quake2);
    policy.speed = [100.0; 3];
    policy.move_multiplier = 1.0;
    input.build_frame_with_policy(EventTime::from_milliseconds(10), &[policy; SeatId::COUNT]);
    input.retain_producers(
        |owner| owner == replacement,
        EventTime::from_milliseconds(20),
        &mut sink,
    );
    assert_eq!(
        input.build_frame_with_policy(EventTime::from_milliseconds(30), &[policy; SeatId::COUNT])
            [0]
        .movement[0],
        100.0
    );
    assert!(sink.router.command(
        SeatId::FIRST,
        EventTime::from_milliseconds(40),
        None,
        Some(old),
        "-forward"
    ));
    sink.router.execute(&mut input);
    assert_eq!(
        input.build_frame_with_policy(EventTime::from_milliseconds(50), &[policy; SeatId::COUNT])
            [0]
        .movement[0],
        100.0
    );
    assert!(sink.router.command(
        SeatId::FIRST,
        EventTime::from_milliseconds(60),
        None,
        Some(replacement),
        "-forward"
    ));
    sink.router.execute(&mut input);
    input.build_frame_with_policy(EventTime::from_milliseconds(70), &[policy; SeatId::COUNT]);
    assert_eq!(
        input.build_frame_with_policy(EventTime::from_milliseconds(80), &[policy; SeatId::COUNT])
            [0]
        .movement[0],
        0.0
    );
}

#[test]
fn rejected_owned_release_preserves_a_human_holder_on_the_same_key() {
    let mut input = Input::load();
    let mut sink = Sink::default();
    let owner = producer(7, 1);
    sink.router.host.producers = [Some(owner), None];
    input.seed(EventTime(0));
    input
        .bind_bytes_owned(26, b"+forward", Some(owner), EventTime(0), &mut sink)
        .unwrap();
    input.dispatch(key(0, DeviceId::Keyboard(17), 26, true, false), &mut sink);
    let source = KeySource {
        device: DeviceId::Keyboard(17),
        control: 26,
    };
    assert!(
        sink.router
            .command(SeatId::FIRST, EventTime(0), Some(source), None, "+forward")
    );
    sink.router.execute(&mut input);
    // A different producer cannot invalidate the existing physical acquisition.
    input.cancel_source(
        source,
        Some(producer(8, 1)),
        EventTime::from_milliseconds(1),
    );
    let mut policy = sink.router.policy(RuleSetId::Quake2);
    policy.speed = [100.0; 3];
    policy.move_multiplier = 1.0;
    input.build_frame_with_policy(EventTime::from_milliseconds(10), &[policy; SeatId::COUNT]);
    sink.router
        .console
        .append(&" ".repeat(65530), Context::default())
        .unwrap();
    input.retire_bindings(owner, EventTime::from_milliseconds(20), &mut sink);
    assert!(input.binding(26).is_none());
    assert_eq!(sink.release_lines, 1);
    assert!(input.is_down(SeatId::FIRST, 26));
    assert_eq!(
        input.build_frame_with_policy(EventTime::from_milliseconds(30), &[policy; SeatId::COUNT])
            [0]
        .movement[0],
        100.0
    );
    sink.router.execute(&mut input);
    assert!(sink.router.command(
        SeatId::FIRST,
        EventTime::from_milliseconds(40),
        Some(source),
        None,
        "-forward"
    ));
    sink.router.execute(&mut input);
    input.build_frame_with_policy(EventTime::from_milliseconds(50), &[policy; SeatId::COUNT]);
    assert_eq!(
        input.build_frame_with_policy(EventTime::from_milliseconds(60), &[policy; SeatId::COUNT])
            [0]
        .movement[0],
        0.0
    );
    input.dispatch(key(70, DeviceId::Keyboard(17), 26, false, false), &mut sink);
    assert_eq!(sink.release_lines, 1);
}
