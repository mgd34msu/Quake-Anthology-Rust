use qa_app::{
    Runtime,
    host::{FrameHost, FrameSource},
};
use qa_console::{commands::Console, views::Context};
use qa_core::{
    primitives::{ModuleId, PlayerTail},
    sys_events::{EventKind, EventTime, SysEvent, SysEventQueue},
};
use qa_session::{clients::Connection, timing::TickRate};
use std::time::Duration;

#[derive(Default)]
struct Source {
    requests: Vec<bool>,
    polls: usize,
    disable_on_first: bool,
}
impl FrameSource for Source {
    fn begin_frame(&mut self) -> Duration {
        Duration::from_millis(20)
    }
    fn request_mouse_capture(&mut self, requested: bool) {
        self.requests.push(requested);
    }
    fn poll_events(&mut self, queue: &mut SysEventQueue) {
        self.polls += 1;
        if self.disable_on_first && self.polls == 1 {
            queue
                .push(SysEvent {
                    time: EventTime::from_milliseconds(20),
                    kind: EventKind::ConsoleLine("in_mouse 0"),
                })
                .unwrap();
        }
        queue
            .push(SysEvent {
                time: EventTime::from_milliseconds(20),
                kind: EventKind::Time,
            })
            .unwrap();
    }
    fn wait_time(&mut self, _: Duration) -> Duration {
        Duration::from_millis(20)
    }
    fn elapsed(&self) -> Duration {
        Duration::ZERO
    }
    fn present(&mut self) {}
}
fn host() -> FrameHost {
    FrameHost::load(
        Console::new(Context::default()).unwrap(),
        Runtime::load(64, std::iter::empty()).unwrap(),
        TickRate::FrameDriven,
        vec![],
    )
    .unwrap()
}
fn connect(host: &mut FrameHost) -> qa_core::primitives::ClientId {
    let client = host
        .runtime
        .server
        .connect(Connection::Local, ModuleId(1), PlayerTail::default(), None)
        .unwrap();
    host.local_clients[0] = Some(client);
    host.local_worlds[0] = Some(qa_render::world::WorldId(0));
    client
}

#[test]
fn capture_requires_connected_game_input_and_releases_after_disconnect() {
    let mut host = host();
    let mut source = Source::default();
    host.frame(&mut source, true);
    assert_eq!(source.requests, [false, false]);
    let client = connect(&mut host);
    source.requests.clear();
    host.frame(&mut source, true);
    assert_eq!(source.requests, [true, true]);
    assert!(host.runtime.server.disconnect(client));
    source.requests.clear();
    host.frame(&mut source, true);
    assert_eq!(source.requests, [false, false]);
    assert_eq!(source.polls, 6);
}

#[test]
fn first_intake_console_change_applies_at_second_intake_without_an_extra_poll() {
    let mut host = host();
    connect(&mut host);
    let mut source = Source {
        disable_on_first: true,
        ..Source::default()
    };
    host.frame(&mut source, true);
    assert_eq!(source.requests, [true, false]);
    assert_eq!(source.polls, 2);
}

#[test]
fn quit_requests_release_before_returning() {
    let mut host = host();
    connect(&mut host);
    host.console
        .append_line("quit", Context::default())
        .unwrap();
    let mut source = Source::default();
    host.frame(&mut source, true);
    assert_eq!(source.requests, [true, false]);
    assert_eq!(source.polls, 1);
    assert!(host.runtime.quit);
}
