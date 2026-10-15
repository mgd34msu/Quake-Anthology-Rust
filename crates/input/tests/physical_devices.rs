use support::RoutedInput as Input;
#[path = "support/console.rs"]
mod support;
use qa_core::{
    primitives::{UserCmd, buttons},
    sys_events::{
        DeviceId, EventKind, EventTime, MAX_INPUT_DEVICES, SeatId, SysEvent, key_catchers,
    },
};
use qa_input::Target;

#[derive(Default)]
struct Sink {
    text: Vec<(SeatId, char)>,
}
impl Target for Sink {
    fn character(&mut self, seat: SeatId, value: char) {
        self.text.push((seat, value));
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
fn event(input: &mut Input, sink: &mut Sink, ms: u64, kind: EventKind<'_>) {
    input.dispatch(
        SysEvent {
            time: EventTime::from_milliseconds(ms),
            kind,
        },
        sink,
    );
}
fn key(input: &mut Input, sink: &mut Sink, ms: u64, id: u32, code: u16, down: bool) {
    event(
        input,
        sink,
        ms,
        EventKind::Key {
            device: DeviceId::Keyboard(id),
            code,
            symbol: 0,
            down,
            repeat: false,
        },
    );
}
fn frame(input: &mut Input, ms: u64) -> [UserCmd; SeatId::COUNT] {
    let policy = input.elapsed_policy(200.0);
    input.build_frame_with_policy(
        EventTime::from_milliseconds(ms),
        &[policy; qa_core::sys_events::SeatId::COUNT],
    )
}

#[test]
fn physical_keyboard_ids_route_independently_and_first_use_keeps_single_seat_input() {
    let mut input = Input::load();
    let mut sink = Sink::default();
    let second = SeatId::new(1).unwrap();
    input.seed(EventTime(0));
    assert!(input.assign(DeviceId::Keyboard(28), second));
    key(&mut input, &mut sink, 0, 17, 26, true);
    key(&mut input, &mut sink, 0, 28, 26, true);
    let commands = frame(&mut input, 20);
    assert_eq!(commands[0].movement[0], 200.0);
    assert_eq!(commands[1].movement[0], 200.0);
    key(&mut input, &mut sink, 20, 17, 26, false);
    let commands = frame(&mut input, 40);
    assert_eq!(commands[0].movement[0], 0.0);
    assert_eq!(commands[1].movement[0], 200.0);
    event(
        &mut input,
        &mut sink,
        40,
        EventKind::DeviceRemoved(DeviceId::Keyboard(28)),
    );
    let commands = frame(&mut input, 60);
    assert_eq!(commands[1].movement[0], 0.0);
    assert_eq!(input.rejected_devices(), 0);
}

#[test]
fn unknown_text_uses_explicit_editor_seat_not_a_physical_keyboard_assignment() {
    let mut input = Input::load();
    let mut sink = Sink::default();
    let second = SeatId::new(1).unwrap();
    input.assign(DeviceId::Keyboard(17), SeatId::FIRST);
    input.set_text_seat(second);
    key(&mut input, &mut sink, 0, 17, 26, true);
    event(
        &mut input,
        &mut sink,
        1,
        EventKind::Text {
            device: DeviceId::Keyboard(0),
            text: "aλ",
        },
    );
    assert_eq!(sink.text, [(second, 'a'), (second, 'λ')]);
}

#[test]
fn capture_changes_release_mouse_holds_and_filter_history_without_releasing_keyboard() {
    let mut input = Input::load();
    let mut sink = Sink::default();
    input.seed(EventTime(0));
    key(&mut input, &mut sink, 0, 17, 44, true);
    event(
        &mut input,
        &mut sink,
        0,
        EventKind::MouseButton {
            device: DeviceId::Mouse(28),
            button: 1,
            down: true,
        },
    );
    event(
        &mut input,
        &mut sink,
        0,
        EventKind::Mouse {
            device: DeviceId::Mouse(28),
            dx: 20,
            dy: 0,
        },
    );
    let first = frame(&mut input, 20)[0];
    assert_ne!(first.buttons & buttons::ATTACK, 0);
    event(&mut input, &mut sink, 20, EventKind::MouseCapture(false));
    let next = frame(&mut input, 40)[0];
    assert_eq!(next.buttons & buttons::ATTACK, 0);
    assert_ne!(next.buttons & buttons::JUMP, 0);
    assert_eq!(next.view_angles, first.view_angles);
    assert!(input.assign(DeviceId::Mouse(28), SeatId::new(1).unwrap()));
    assert!(!input.assign(DeviceId::Keyboard(17), SeatId::FIRST));
}

#[test]
fn full_device_table_counts_rejections_and_removal_reuses_a_slot() {
    let mut input = Input::load();
    input.seed(EventTime(0));
    let mut sink = Sink::default();
    for id in 1..=MAX_INPUT_DEVICES as u32 {
        key(&mut input, &mut sink, 0, id, 1023, true);
    }
    assert_eq!(input.rejected_devices(), 2); // Keyboard(0) and Mouse(0) reserve two slots.
    event(
        &mut input,
        &mut sink,
        1,
        EventKind::DeviceRemoved(DeviceId::Keyboard(1)),
    );
    key(&mut input, &mut sink, 2, 999, 26, true);
    assert_eq!(input.rejected_devices(), 2);
    assert!(frame(&mut input, 20)[0].movement[0] > 0.0);
}

#[test]
fn catcher_suppresses_only_its_seat_and_keeps_release_accounting() {
    let mut input = Input::load();
    let mut sink = Sink::default();
    let second = SeatId::ALL[1];
    input.assign(DeviceId::Keyboard(28), second);
    input.seed(EventTime(0));
    key(&mut input, &mut sink, 0, 0, 26, true);
    key(&mut input, &mut sink, 0, 28, 26, true);
    let policies = [support::catalog_policy(qa_core::primitives::RuleSetId::Quake3); SeatId::COUNT];
    input.set_catcher(SeatId::FIRST, key_catchers::CONSOLE, true, EventTime(0));
    event(
        &mut input,
        &mut sink,
        1,
        EventKind::Mouse {
            device: DeviceId::Mouse(0),
            dx: 100,
            dy: 100,
        },
    );
    let commands = input.build_frame_with_policy(EventTime::from_milliseconds(20), &policies);
    assert_eq!(commands[0].movement, [0.0; 3]);
    assert_eq!(commands[0].buttons, buttons::TALK);
    assert_eq!(commands[0].view_angles, Default::default());
    assert!(commands[1].movement[0] > 0.0);
    key(&mut input, &mut sink, 21, 0, 26, false);
    input.set_catcher(
        SeatId::FIRST,
        key_catchers::CONSOLE,
        false,
        EventTime::from_milliseconds(22),
    );
    let commands = input.build_frame_with_policy(EventTime::from_milliseconds(40), &policies);
    assert_eq!(commands[0].movement, [0.0; 3]);
    assert_eq!(commands[0].view_angles, Default::default());
    assert!(commands[1].movement[0] > 0.0);
}
