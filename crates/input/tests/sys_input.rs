use support::RoutedInput as Input;
#[path = "support/console.rs"]
mod support;
use qa_core::{
    primitives::buttons,
    sys_events::{DeviceId, EventKind, EventTime, SeatId, SysEvent},
};
use qa_input::{Action, Binding, Target};
#[derive(Default)]
struct Sink {
    characters: String,
}
impl Target for Sink {
    fn character(&mut self, _: SeatId, value: char) {
        self.characters.push(value);
    }
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
fn key(input: &mut Input, sink: &mut Sink, ms: u64, code: u16, down: bool, repeat: bool) {
    device_key(input, sink, DeviceId::Keyboard(0), ms, code, down, repeat);
}
fn device_key(
    input: &mut Input,
    sink: &mut impl Target,
    device: DeviceId,
    ms: u64,
    code: u16,
    down: bool,
    repeat: bool,
) {
    input.dispatch(
        SysEvent {
            time: EventTime(ms * 1_000_000),
            kind: EventKind::Key {
                device,
                code,
                symbol: 0,
                down,
                repeat,
            },
        },
        sink,
    );
}

#[test]
fn physical_holds_survive_consumption_repeats_and_duplicate_releases() {
    struct Consuming;
    impl Target for Consuming {
        fn key(&mut self, _: SeatId, _: u16, _: bool, _: bool) -> bool {
            true
        }
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
    let mut input = Input::load();
    input.seed(EventTime(0));
    let mut sink = Consuming;
    for code in 0..1024 {
        device_key(
            &mut input,
            &mut sink,
            DeviceId::Keyboard(0),
            1,
            code,
            true,
            false,
        );
        device_key(
            &mut input,
            &mut sink,
            DeviceId::Keyboard(0),
            2,
            code,
            true,
            true,
        );
    }
    assert!(!input.assign(DeviceId::Keyboard(0), SeatId::FIRST));
    assert_eq!(frame(&mut input, 3)[0].buttons, buttons::ANY);
    for code in 0..1023 {
        device_key(
            &mut input,
            &mut sink,
            DeviceId::Keyboard(0),
            4,
            code,
            false,
            false,
        );
        device_key(
            &mut input,
            &mut sink,
            DeviceId::Keyboard(0),
            5,
            code,
            false,
            false,
        );
    }
    assert_eq!(frame(&mut input, 6)[0].buttons, buttons::ANY);
    device_key(
        &mut input,
        &mut sink,
        DeviceId::Keyboard(0),
        7,
        1024,
        true,
        false,
    );
    device_key(
        &mut input,
        &mut sink,
        DeviceId::Keyboard(0),
        7,
        1023,
        false,
        false,
    );
    assert_eq!(frame(&mut input, 8)[0].buttons, 0);
    assert!(input.assign(DeviceId::Keyboard(0), SeatId::FIRST));
}

#[test]
fn rebind_keeps_both_modifier_holds_until_their_physical_releases() {
    let mut input = Input::load();
    input.seed(EventTime(0));
    let mut sink = Sink::default();
    for code in [224, 228] {
        key(&mut input, &mut sink, 1, code, true, false);
    }
    assert_eq!(
        frame(&mut input, 2)[0].buttons,
        buttons::ANY | buttons::CROUCH
    );
    assert!(input.bind(224, None, EventTime(2_000_000), &mut sink));
    key(&mut input, &mut sink, 3, 224, true, true);
    key(&mut input, &mut sink, 3, 228, true, true);
    assert!(!input.assign(DeviceId::Keyboard(0), SeatId::FIRST));
    assert_eq!(frame(&mut input, 4)[0].buttons, buttons::ANY);
    key(&mut input, &mut sink, 5, 224, false, false);
    assert_eq!(frame(&mut input, 6)[0].buttons, buttons::ANY);
    key(&mut input, &mut sink, 7, 228, false, false);
    assert_eq!(frame(&mut input, 8)[0].buttons, 0);
    assert!(input.assign(DeviceId::Keyboard(0), SeatId::FIRST));
}

#[test]
fn removing_one_held_device_preserves_other_seats_and_focus_clears_all() {
    let mut input = Input::load();
    input.seed(EventTime(0));
    let mut sink = Sink::default();
    let second = SeatId::new(1).unwrap();
    assert!(input.assign(DeviceId::Controller(42), second));
    for device in [DeviceId::Keyboard(0), DeviceId::Controller(42)] {
        device_key(&mut input, &mut sink, device, 1, 1023, true, false);
    }
    assert!(!input.assign(DeviceId::Controller(42), second));
    let commands = frame(&mut input, 2);
    assert_eq!(commands[0].buttons, buttons::ANY);
    assert_eq!(commands[1].buttons, buttons::ANY);
    input.dispatch(
        SysEvent {
            time: EventTime(3_000_000),
            kind: EventKind::DeviceRemoved(DeviceId::Controller(42)),
        },
        &mut sink,
    );
    let commands = frame(&mut input, 4);
    assert_eq!(commands[0].buttons, buttons::ANY);
    assert_eq!(commands[1].buttons, 0);
    assert!(input.assign(DeviceId::Controller(42), second));
    device_key(
        &mut input,
        &mut sink,
        DeviceId::Controller(42),
        5,
        1023,
        true,
        false,
    );
    input.dispatch(
        SysEvent {
            time: EventTime(6_000_000),
            kind: EventKind::Focus(false),
        },
        &mut sink,
    );
    assert!(
        frame(&mut input, 7)
            .iter()
            .all(|command| command.buttons == 0)
    );
    assert!(input.assign(DeviceId::Keyboard(0), SeatId::FIRST));
    assert!(input.assign(DeviceId::Controller(42), second));
}
fn frame(input: &mut Input, ms: u64) -> [qa_core::primitives::UserCmd; 4] {
    let policy = input.elapsed_policy(200.0);
    input.build_frame_with_policy(
        EventTime(ms * 1_000_000),
        &[policy; qa_core::sys_events::SeatId::COUNT],
    )
}
#[test]
fn repeated_down_preserves_partial_frame_time_and_two_keys_hold_one_action() {
    let mut input = Input::load();
    input.seed(EventTime(0));
    let mut sink = Sink::default();
    input.bind(
        82,
        Some(Binding::for_action(Action::Forward)),
        EventTime(0),
        &mut sink,
    );
    key(&mut input, &mut sink, 10, 26, true, false);
    for time in [15, 20, 25] {
        key(&mut input, &mut sink, time, 26, true, true);
    }
    key(&mut input, &mut sink, 26, 82, true, false);
    key(&mut input, &mut sink, 30, 26, false, false);
    assert_eq!(frame(&mut input, 40)[0].movement[0], 150.0); // down 30/40 ms
    assert_eq!(frame(&mut input, 60)[0].movement[0], 200.0);
    key(&mut input, &mut sink, 70, 82, false, false);
    assert_eq!(frame(&mut input, 80)[0].movement[0], 100.0);
    assert_eq!(frame(&mut input, 100)[0].movement[0], 0.0);
}
#[test]
fn command_edges_and_focus_loss_do_not_stick() {
    let mut input = Input::load();
    input.seed(EventTime(0));
    let mut sink = Sink::default();
    input
        .bind_text(10, "+edge", EventTime(0), &mut sink)
        .unwrap();
    key(&mut input, &mut sink, 1, 10, true, false);
    key(&mut input, &mut sink, 2, 10, true, true);
    key(&mut input, &mut sink, 3, 10, false, false);
    assert_eq!(
        (input.router.host.presses, input.router.host.releases),
        (1, 1)
    );
    key(&mut input, &mut sink, 4, 26, true, false);
    assert!(input.bind(
        26,
        Some(Binding::for_action(Action::Back)),
        EventTime(4_000_000),
        &mut sink
    ));
    key(&mut input, &mut sink, 4, 44, true, false);
    assert_eq!(
        frame(&mut input, 5)[0].buttons & buttons::JUMP,
        buttons::JUMP
    );
    input.dispatch(
        SysEvent {
            time: EventTime(6_000_000),
            kind: EventKind::Focus(false),
        },
        &mut sink,
    );
    assert_eq!(frame(&mut input, 10)[0].movement, [0.0; 3]);
    assert_eq!(frame(&mut input, 15)[0].buttons, 0);
    assert!(input.bind(
        26,
        Some(Binding::for_action(Action::Back)),
        EventTime(15_000_000),
        &mut sink
    ));
    key(&mut input, &mut sink, 16, 10, true, false);
    input.dispatch(
        SysEvent {
            time: EventTime(17_000_000),
            kind: EventKind::Focus(false),
        },
        &mut sink,
    );
    assert_eq!(
        (input.router.host.presses, input.router.host.releases),
        (2, 2)
    );
}
#[test]
fn two_devices_route_independently_and_disconnect_releases_actions() {
    let mut input = Input::load();
    input.seed(EventTime(0));
    let mut sink = Sink::default();
    let second = SeatId::new(1).unwrap();
    input.assign(DeviceId::Controller(42), second);
    key(&mut input, &mut sink, 0, 26, true, false);
    input.dispatch(
        SysEvent {
            time: EventTime(0),
            kind: EventKind::ControllerAxis {
                device: DeviceId::Controller(42),
                axis: 0,
                value: 16384,
            },
        },
        &mut sink,
    );
    input.dispatch(
        SysEvent {
            time: EventTime(1),
            kind: EventKind::ControllerButton {
                device: DeviceId::Controller(42),
                button: 0,
                down: true,
            },
        },
        &mut sink,
    );
    let commands = frame(&mut input, 20);
    assert_eq!(commands[0].movement, [200.0, 0.0, 0.0]);
    // Native Q2 input also represents the controller's jump button as upmove.
    assert_eq!(commands[1].movement, [0.0, 100.0, 200.0]);
    assert_eq!(commands[0].buttons & buttons::JUMP, 0);
    assert_eq!(commands[1].buttons & buttons::JUMP, buttons::JUMP);
    input.dispatch(
        SysEvent {
            time: EventTime(25_000_000),
            kind: EventKind::DeviceRemoved(DeviceId::Controller(42)),
        },
        &mut sink,
    );
    // The released jump contributes its five held milliseconds to this frame.
    assert_eq!(frame(&mut input, 40)[1].movement, [0.0, 0.0, 50.0]);
    assert_eq!(frame(&mut input, 60)[1].movement, [0.0; 3]);
}

#[test]
fn queued_text_dispatches_unicode_without_a_private_remainder() {
    use qa_core::sys_events::SysEventQueue;
    let mut queue = SysEventQueue::load(2, 32).unwrap();
    let mut input = Input::load();
    let mut sink = Sink::default();
    queue
        .push(SysEvent {
            time: EventTime(1),
            kind: EventKind::Text {
                device: DeviceId::Keyboard(0),
                text: "aλ🦀",
            },
        })
        .unwrap();
    while let Some(event) = queue.pop() {
        input.dispatch(event, &mut sink);
    }
    assert_eq!(sink.characters, "aλ🦀");
    input.dispatch(
        SysEvent {
            time: EventTime(2),
            kind: EventKind::Text {
                device: DeviceId::Mouse(99),
                text: "ignored",
            },
        },
        &mut sink,
    );
    assert_eq!(sink.characters, "aλ🦀");
    assert_eq!(qa_input::keys::mouse_button(0), None);
    assert_eq!(qa_input::keys::mouse_button(1), Some(513));
    assert_eq!(qa_input::keys::mouse_button(31), Some(543));
    assert_eq!(qa_input::keys::mouse_button(32), Some(580));
    assert_eq!(qa_input::keys::mouse_button(33), None);
}
