//! qsrc key/button contracts through the real shared console and host drain.
use qa_app::{
    Runtime,
    host::{FrameHost, FrameSource},
};
use qa_console::{
    commands::Console,
    views::{Context, Source as CommandSource},
};
use qa_core::{
    primitives::buttons,
    sys_events::{DeviceId, EventKind, EventTime, SeatId, SysEvent, SysEventQueue},
};
use qa_session::timing::TickRate;
use std::time::Duration;

struct Source(u64);
impl FrameSource for Source {
    fn begin_frame(&mut self, queue: &mut SysEventQueue) {
        self.poll_events(queue);
    }
    fn poll_events(&mut self, queue: &mut SysEventQueue) {
        queue
            .push(SysEvent {
                time: EventTime(self.0 * 1_000_000),
                kind: EventKind::Time,
            })
            .unwrap();
    }
    fn wait_events(&mut self, _: &mut SysEventQueue, _: Duration) {
        panic!("uncapped fixture");
    }
    fn elapsed(&self) -> Duration {
        Duration::ZERO
    }
    fn present(&mut self) {}
}
fn host(source: CommandSource) -> FrameHost {
    FrameHost::load(
        Console::new(Context {
            source,
            ..Context::default()
        }),
        Runtime::load().unwrap(),
        TickRate::FrameDriven,
        vec![],
    )
    .unwrap()
}
fn event(host: &mut FrameHost, ms: u64, kind: EventKind<'_>) {
    host.queue
        .push(SysEvent {
            time: EventTime(ms * 1_000_000),
            kind,
        })
        .unwrap();
}
fn key(host: &mut FrameHost, ms: u64, code: u16, down: bool, repeat: bool) {
    event(
        host,
        ms,
        EventKind::Key {
            device: DeviceId::Keyboard,
            code,
            symbol: 0,
            down,
            repeat,
        },
    );
}
#[test]
fn every_source_config_reaches_one_table_and_preserves_repeat_and_two_key_time() {
    for source in CommandSource::ALL {
        let mut host = host(source);
        event(
            &mut host,
            0,
            EventKind::ConsoleLine(
                "unbindall; bind w +forward; bind UPARROW +forward; bind MOUSE1 +attack",
            ),
        );
        host.frame(&mut Source(0), true);
        key(&mut host, 10, 26, true, false);
        for ms in [15, 20, 25] {
            key(&mut host, ms, 26, true, true);
        }
        key(&mut host, 26, 82, true, false);
        key(&mut host, 30, 26, false, false);
        let frame = host.frame(&mut Source(40), true);
        assert_eq!(frame.commands[0].movement[0], 95); // 30/40 * 127
        assert_eq!(frame.commands[1].movement, [0; 3]);
        assert_eq!(host.key_repeats, 3);
        key(&mut host, 50, 82, false, false);
        assert_eq!(
            host.frame(&mut Source(60), true).commands[0].movement[0],
            63
        );
        assert_eq!(
            host.frame(&mut Source(80), true).commands[0].movement,
            [0; 3]
        );
        assert!(host.console.idle());
    }
}
#[test]
fn alias_button_metadata_and_manual_commands_keep_the_originating_seat() {
    let mut host = host(CommandSource::Quake2);
    let second = SeatId::new(1).unwrap();
    host.runtime.input.assign(DeviceId::Controller(42), second);
    event(
        &mut host,
        0,
        EventKind::ConsoleLine(
            "unbindall; alias +held +forward; alias -held -forward; bind JOY1 +held",
        ),
    );
    host.frame(&mut Source(0), true);
    event(
        &mut host,
        10,
        EventKind::ControllerButton {
            device: DeviceId::Controller(42),
            button: 0,
            down: true,
        },
    );
    let commands = host.frame(&mut Source(20), true).commands;
    assert_eq!(commands[0].movement, [0; 3]);
    assert!(commands[1].movement[0] > 0);
    event(
        &mut host,
        25,
        EventKind::DeviceRemoved(DeviceId::Controller(42)),
    );
    host.frame(&mut Source(40), true);
    assert_eq!(
        host.frame(&mut Source(60), true).commands[1].movement,
        [0; 3]
    );
    let context = Context {
        seat: second,
        ..host.console.cvars.context()
    };
    host.console
        .append_line("+attack 7 60; +attack 8 65; -attack 7 70", context)
        .unwrap();
    assert_eq!(
        host.frame(&mut Source(80), true).commands[1].buttons & buttons::ATTACK,
        buttons::ATTACK
    );
    host.console.append_line("-attack", context).unwrap();
    assert_eq!(
        host.frame(&mut Source(100), true).commands[1].buttons & buttons::ATTACK,
        0
    );
    assert_eq!(
        host.frame(&mut Source(120), true).commands[0].buttons & buttons::ATTACK,
        0
    );
}
#[test]
fn held_rebind_releases_old_action_and_repeat_cannot_acquire_replacement() {
    let mut host = host(CommandSource::Quake3);
    event(&mut host, 0, EventKind::ConsoleLine("bind w +forward"));
    host.frame(&mut Source(0), true);
    key(&mut host, 10, 26, true, false);
    host.frame(&mut Source(20), true);
    event(&mut host, 25, EventKind::ConsoleLine("bind w +back"));
    host.frame(&mut Source(40), true);
    key(&mut host, 45, 26, true, true);
    assert_eq!(
        host.frame(&mut Source(60), true).commands[0].movement,
        [0; 3]
    );
    key(&mut host, 65, 26, false, false);
    key(&mut host, 70, 26, true, false);
    assert_eq!(
        host.frame(&mut Source(80), true).commands[0].movement[0],
        -63
    );
    event(
        &mut host,
        90,
        EventKind::ConsoleLine("unbindall; bind w +forward"),
    );
    host.frame(&mut Source(100), true);
    key(&mut host, 110, 26, true, true);
    assert_eq!(
        host.frame(&mut Source(120), true).commands[0].movement,
        [0; 3]
    );
}
#[test]
fn q3_numbered_button_aliases_reach_the_native_command_projection() {
    for number in 0..15 {
        let mut host = host(CommandSource::Quake);
        let command = format!("+button{number} 7 0");
        event(&mut host, 0, EventKind::ConsoleLine(&command));
        let frame = host.frame(&mut Source(20), true);
        let projected = qa_network::commands::to_q3_usercmd(&frame.commands[0], 0);
        assert_eq!(projected.buttons & (1 << number), 1 << number);
        event(
            &mut host,
            25,
            EventKind::ConsoleLine(&format!("-button{number} 7 25")),
        );
        host.frame(&mut Source(40), true);
        assert_eq!(
            qa_network::commands::to_q3_usercmd(&host.frame(&mut Source(60), true).commands[0], 0)
                .buttons
                & (1 << number),
            0
        );
    }
}
