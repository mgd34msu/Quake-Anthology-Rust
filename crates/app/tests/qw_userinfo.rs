use qa_app::{
    Runtime,
    client_policy::ClientPolicy,
    host::{FrameHost, FrameSource},
    map::SpawnAnchor,
};
use qa_console::{commands::Console, views::Context};
use qa_core::{
    primitives::{ClientId, RuleSetId},
    sys_events::{EventKind, EventTime, SeatId, SysEvent, SysEventQueue},
    text::info_value,
};
use qa_network::commands::packet::Protocol;
use qa_session::timing::TickRate;
use std::time::Duration;

#[derive(Default)]
struct Source {
    time: u64,
}
impl FrameSource for Source {
    fn begin_frame(&mut self) -> Duration {
        Duration::from_millis(self.time)
    }
    fn poll_events(&mut self, queue: &mut SysEventQueue) {
        queue
            .push(SysEvent {
                time: EventTime::from_milliseconds(self.time),
                kind: EventKind::Time,
            })
            .unwrap();
    }
    fn wait_time(&mut self, _: Duration) -> Duration {
        panic!("uncapped fixture")
    }
    fn elapsed(&self) -> Duration {
        Duration::ZERO
    }
    fn present(&mut self) {}
}

fn field<'a>(info: &'a [u8], name: &[u8]) -> &'a [u8] {
    info_value(info, name, false).map_or(b"", |range| &info[range])
}
fn setting(host: &mut FrameHost, seat: SeatId, name: &str, value: &str) {
    let view = host
        .console
        .cvars
        .bind(
            name,
            Context {
                source: RuleSetId::QuakeWorld,
                seat,
                ..Default::default()
            },
        )
        .unwrap();
    host.console.cvars.force_write(view, value).unwrap();
}
fn advance(host: &mut FrameHost, source: &mut Source) {
    for _ in 0..8 {
        source.time += 20;
        assert_eq!(host.frame(source, true).drains, 2);
    }
}

#[test]
fn userinfo_changes_travel_through_each_native_connection_and_peer_events() {
    let mut runtime = Runtime::load(2, []).unwrap();
    let seats = [SeatId::FIRST, SeatId::ALL[1]];
    let clients: [ClientId; 2] = std::array::from_fn(|index| {
        runtime
            .connect_local(
                seats[index],
                SpawnAnchor {
                    position: Default::default(),
                    angles: Default::default(),
                    entity: 0,
                    fixture_fallback: false,
                },
                ClientPolicy::select(
                    Some(RuleSetId::Quake3),
                    None,
                    Some(RuleSetId::Quake2),
                    Some(RuleSetId::Quake),
                    None,
                )
                .unwrap(),
                Protocol::QuakeWorld28,
                index as u32,
            )
            .unwrap()
    });
    let mut host = FrameHost::load(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        vec![],
    )
    .unwrap();
    for index in 0..2 {
        host.local_clients[seats[index].index()] = Some(clients[index]);
    }
    // These catalog entries are global. The connections retain independent
    // native userinfo records and receive the server's own name correction.
    setting(&mut host, seats[0], "name", "First");
    setting(&mut host, seats[0], "skin", "red");
    setting(&mut host, seats[0], "topcolor", "4");
    let mut source = Source::default();
    advance(&mut host, &mut source);
    for (index, name, skin, color) in [
        (0, b"First".as_slice(), b"red".as_slice(), b"4".as_slice()),
        (1, b"(1)First", b"red", b"4"),
    ] {
        let info = host.runtime.server.userinfo(clients[index]).unwrap();
        assert_eq!(field(info, b"name"), name);
        assert_eq!(field(info, b"skin"), skin);
        assert_eq!(field(info, b"topcolor"), color);
        for receiver in clients {
            let info = host.runtime.client_resources[receiver.0 as usize]
                .as_ref()
                .unwrap()
                .state
                .peer(index)
                .unwrap()
                .text;
            assert_eq!(field(info, b"name"), name);
            assert_eq!(field(info, b"skin"), skin);
        }
    }
    host.runtime
        .network
        .get_mut(clients[1], qa_core::loopback::Endpoint::Client)
        .unwrap()
        .channel
        .queue_client_command(Protocol::QuakeWorld28, b"setinfo \"skin\" \"blue\"")
        .unwrap();
    advance(&mut host, &mut source);
    setting(&mut host, seats[0], "topcolor", "12");
    host.runtime
        .network
        .get_mut(clients[0], qa_core::loopback::Endpoint::Client)
        .unwrap()
        .channel
        .queue_client_command(Protocol::QuakeWorld28, b"setinfo \"skin\" \"\"")
        .unwrap();
    advance(&mut host, &mut source);
    assert_eq!(
        field(host.runtime.server.userinfo(clients[0]).unwrap(), b"skin"),
        b""
    );
    assert_eq!(
        field(
            host.runtime.server.userinfo(clients[0]).unwrap(),
            b"topcolor"
        ),
        b"12"
    );
    assert_eq!(
        field(host.runtime.server.userinfo(clients[1]).unwrap(), b"skin"),
        b"blue"
    );
    for client in clients {
        assert!(host.runtime.server.clients[client.0 as usize].is_active());
    }
}
