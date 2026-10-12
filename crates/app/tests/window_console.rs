use qa_app::{Runtime, host::FrameHost};
use qa_console::{commands::Console, views::Context};
use qa_core::sys_events::{DeviceId, EventKind, EventTime, SeatId, SysEvent};
use qa_session::timing::TickRate;

fn push(host: &mut FrameHost, kind: EventKind<'_>, ms: u64) {
    host.queue
        .push(SysEvent {
            time: EventTime::from_milliseconds(ms),
            kind,
        })
        .unwrap();
}

fn key(host: &mut FrameHost, control: u16, down: bool, repeat: bool, ms: u64) {
    push(
        host,
        EventKind::Key {
            device: DeviceId::Keyboard(0),
            code: control,
            symbol: 0,
            down,
            repeat,
        },
        ms,
    );
}

#[test]
fn queued_window_text_reaches_the_one_console_and_repeat_edits_do_not_move() {
    let mut host = FrameHost::load(
        Console::new(Context::default()).unwrap(),
        Runtime::load(64, std::iter::empty()).unwrap(),
        TickRate::FrameDriven,
        vec![],
    )
    .unwrap();
    push(&mut host, EventKind::Time, 0);
    key(&mut host, 53, true, false, 1);
    key(&mut host, 53, true, true, 2);
    push(
        &mut host,
        EventKind::Text {
            device: DeviceId::Keyboard(0),
            text: "cg_fov 123x",
        },
        3,
    );
    key(&mut host, 42, true, true, 4);
    key(&mut host, 26, true, false, 5);
    key(&mut host, 40, true, false, 6);
    push(&mut host, EventKind::Time, 20);
    host.drain(&mut Default::default());
    host.console.execute_frame(&mut host.runtime);
    assert_eq!(host.console.editor.seat(), Some(SeatId::FIRST));
    assert_eq!(
        host.console
            .cvars
            .value(host.console.cvars.find("cg_fov").unwrap()),
        123.0
    );
    // This fixture selects the native Q2 input policy: console capture produces
    // no movement or action bits. Q3 intentionally emits its native TALK bit.
    let command = host.runtime.input.build_frame_with_policy(
        EventTime::from_milliseconds(20),
        &[qa_input::InputPolicy::native(qa_core::primitives::RuleSetId::Quake2);
            qa_core::sys_events::SeatId::COUNT],
    )[0];
    assert_eq!(command.movement, [0.0; 3]);
    assert_eq!(command.buttons, 0);
    // A key first pressed in the console does not acquire a game binding when
    // the catcher closes. A fresh press does, via the same physical held state.
    key(&mut host, 41, true, false, 21);
    key(&mut host, 26, false, false, 22);
    key(&mut host, 26, true, false, 23);
    push(&mut host, EventKind::Time, 40);
    host.drain(&mut Default::default());
    // Both native and custom bindings now execute through the queued console.
    host.console.execute_frame(&mut host.runtime);
    let command = host.runtime.input.build_frame_with_policy(
        EventTime::from_milliseconds(40),
        &[qa_input::InputPolicy::native(qa_core::primitives::RuleSetId::Quake2);
            qa_core::sys_events::SeatId::COUNT],
    )[0];
    assert!(command.movement[0] > 0.0);
}

#[test]
fn full_command_buffer_reports_queued_stdin_rejection_through_the_output_ring() {
    let mut host = FrameHost::load(
        Console::new(Context::default()).unwrap(),
        Runtime::load(64, std::iter::empty()).unwrap(),
        TickRate::FrameDriven,
        vec![],
    )
    .unwrap();
    host.console
        .append(
            &"x".repeat(qa_console::command_buffer::CAPACITY - 1),
            Context::default(),
        )
        .unwrap();
    push(&mut host, EventKind::ConsoleLine("cg_fov 123"), 1);
    host.drain(&mut Default::default());
    let mut batch = host
        .runtime
        .server
        .events
        .batch(host.runtime.server.presentation)
        .unwrap();
    let record = host.runtime.server.events.next(&mut batch).unwrap();
    let qa_core::events::FrameEvent::Print(print) = record.event else {
        panic!("stdin rejection must use the one print ring");
    };
    assert!(
        host.runtime
            .server
            .events
            .texts
            .get(print.text)
            .unwrap()
            .starts_with(b"Input command:")
    );
}
