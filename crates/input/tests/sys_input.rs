use qa_core::{
    primitives::buttons,
    sys_events::{DeviceId, EventKind, EventTime, SeatId, SysEvent},
};
use qa_input::{Action, Binding, CommandIntent, Input, Target, UserCmdBuilder};
#[derive(Default)]
struct Sink {
    presses: u32,
    releases: u32,
    characters: String,
}
impl Target for Sink {
    fn character(&mut self, _: SeatId, value: char) {
        self.characters.push(value);
    }
    fn command(&mut self, _: SeatId, text: &str) {
        match text {
            "press" => self.presses += 1,
            "release" => self.releases += 1,
            _ => panic!(),
        }
    }
}
fn key(input: &mut Input, sink: &mut Sink, ms: u64, code: u16, down: bool, repeat: bool) {
    input.dispatch(
        SysEvent {
            time: EventTime(ms * 1_000_000),
            kind: EventKind::Key {
                device: DeviceId::Keyboard,
                code,
                symbol: 0,
                down,
                repeat,
            },
        },
        sink,
    );
}
fn frame(input: &mut Input, ms: u64) -> [qa_core::primitives::UserCmd; 4] {
    input.build_frame(EventTime(ms * 1_000_000), [200; 3], [0.022; 2], [None; 4])
}
#[test]
fn repeated_down_preserves_partial_frame_time_and_two_keys_hold_one_action() {
    let mut input = Input::load();
    let mut sink = Sink::default();
    input.bind(82, Some(Binding::Action(Action::Forward)));
    key(&mut input, &mut sink, 10, 26, true, false);
    for time in [15, 20, 25] {
        key(&mut input, &mut sink, time, 26, true, true);
    }
    key(&mut input, &mut sink, 26, 82, true, false);
    key(&mut input, &mut sink, 30, 26, false, false);
    assert_eq!(frame(&mut input, 40)[0].movement[0], 150); // down 30/40 ms
    assert_eq!(frame(&mut input, 60)[0].movement[0], 200);
    key(&mut input, &mut sink, 70, 82, false, false);
    assert_eq!(frame(&mut input, 80)[0].movement[0], 100);
    assert_eq!(frame(&mut input, 100)[0].movement[0], 0);
}
#[test]
fn command_edges_and_focus_loss_do_not_stick() {
    let mut input = Input::load();
    let mut sink = Sink::default();
    input.bind(
        10,
        Some(Binding::Command {
            press: "press".into(),
            release: Some("release".into()),
        }),
    );
    key(&mut input, &mut sink, 1, 10, true, false);
    key(&mut input, &mut sink, 2, 10, true, true);
    key(&mut input, &mut sink, 3, 10, false, false);
    assert_eq!((sink.presses, sink.releases), (1, 1));
    key(&mut input, &mut sink, 4, 26, true, false);
    assert!(!input.bind(26, Some(Binding::Action(Action::Back))));
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
    assert_eq!(frame(&mut input, 10)[0].movement, [0; 3]);
    assert_eq!(frame(&mut input, 15)[0].buttons, 0);
    assert!(input.bind(26, Some(Binding::Action(Action::Back))));
    key(&mut input, &mut sink, 16, 10, true, false);
    input.dispatch(
        SysEvent {
            time: EventTime(17_000_000),
            kind: EventKind::Focus(false),
        },
        &mut sink,
    );
    assert_eq!((sink.presses, sink.releases), (2, 2));
}
#[test]
fn two_devices_route_independently_and_bots_use_the_same_builder() {
    let mut input = Input::load();
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
    assert_eq!(commands[0].movement, [200, 0, 0]);
    assert_eq!(commands[1].movement, [0, 100, 0]);
    assert_eq!(commands[0].buttons & buttons::JUMP, 0);
    assert_eq!(commands[1].buttons & buttons::JUMP, buttons::JUMP);
    input.dispatch(
        SysEvent {
            time: EventTime(25_000_000),
            kind: EventKind::DeviceRemoved(DeviceId::Controller(42)),
        },
        &mut sink,
    );
    assert_eq!(frame(&mut input, 40)[1].movement, [0; 3]);
    let intent = CommandIntent {
        movement: [75, -45, 17],
        buttons: buttons::ATTACK,
        ..CommandIntent::default()
    };
    let mut bots = [None; 4];
    bots[2] = Some(intent);
    let commands = input.build_frame(EventTime(60_000_000), [200; 3], [0.022; 2], bots);
    let mut same = UserCmdBuilder::default();
    same.build(
        SeatId::new(2).unwrap(),
        EventTime(40_000_000),
        CommandIntent::default(),
    );
    let bot = same.build(SeatId::new(2).unwrap(), EventTime(60_000_000), intent);
    assert_eq!(commands[2].movement, bot.movement);
    assert_eq!(commands[2].buttons, bot.buttons);
    assert_eq!(commands[2].duration_ms, bot.duration_ms);
}
