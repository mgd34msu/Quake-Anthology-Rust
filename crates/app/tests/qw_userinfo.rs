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

fn pair(native: [u32; 2], capacity: Option<usize>) -> (FrameHost, [ClientId; 2]) {
    let mut runtime = Runtime::load(3, []).unwrap();
    if let Some(capacity) = capacity {
        use qa_core::events::{EventRing, OutputTarget};
        runtime.server.events = EventRing::load(capacity, 4, 40, 8192).unwrap();
        runtime.server.presentation = runtime
            .server
            .events
            .bind(OutputTarget::Presentation)
            .unwrap();
    }
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
                native[index],
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
    (host, clients)
}

#[test]
fn userinfo_changes_travel_through_each_native_connection_and_peer_events() {
    let (mut host, clients) = pair([0, 1], None);
    let seats = [SeatId::FIRST, SeatId::ALL[1]];
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

#[test]
fn one_slot_ring_eventually_delivers_both_peers_with_recipient_owned_numbers() {
    let (mut host, clients) = pair([2, 7], Some(1));
    let mut source = Source::default();
    for _ in 0..8 {
        advance(&mut host, &mut source);
    }
    for (id, info) in [
        (
            clients[0],
            b"\\name\\Alpha\\skin\\red\\_password\\secret".as_slice(),
        ),
        (clients[1], b"\\name\\Beta\\skin\\blue".as_slice()),
    ] {
        host.runtime
            .server
            .replace_userinfo_raw(id, info, 2048)
            .unwrap();
    }
    for _ in 0..16 {
        advance(&mut host, &mut source);
    }
    for (receiver, slots) in [(clients[0], [2, 0]), (clients[1], [0, 7])] {
        let state = &host.runtime.client_resources[receiver.0 as usize]
            .as_ref()
            .unwrap()
            .state;
        for (index, name, skin) in [
            (0, b"Alpha".as_slice(), b"red".as_slice()),
            (1, b"Beta", b"blue"),
        ] {
            let peer = state.peer(slots[index]).unwrap();
            assert_eq!(
                field(peer.text, b"name"),
                name,
                "receiver {:?}, peer {:?}",
                receiver,
                peer.text
            );
            assert_eq!(field(peer.text, b"skin"), skin);
            assert_eq!(field(peer.text, b"_password"), b"");
            assert_eq!(
                peer.metadata.user_id,
                Some(
                    host.runtime
                        .server
                        .userinfo_record(clients[index])
                        .unwrap()
                        .stamp
                        .user_id
                )
            );
        }
    }
    assert_eq!(
        field(
            host.runtime.server.userinfo(clients[0]).unwrap(),
            b"_password"
        ),
        b"secret"
    );
    assert!(host.runtime.server.world_frame > 0);
    for id in clients {
        assert!(host.runtime.server.clients[id.0 as usize].is_active());
    }
}

#[test]
fn disconnected_peer_clears_its_old_number_before_a_new_admission_reuses_the_client_slot() {
    let (mut host, clients) = pair([2, 7], None);
    let mut source = Source::default();
    advance(&mut host, &mut source);
    host.runtime
        .server
        .replace_userinfo_raw(clients[0], b"\\name\\Departed", 2048)
        .unwrap();
    advance(&mut host, &mut source);
    let old = host
        .runtime
        .server
        .userinfo_record(clients[0])
        .unwrap()
        .stamp;
    let other = clients[1];
    assert_eq!(
        field(
            host.runtime.client_resources[other.0 as usize]
                .as_ref()
                .unwrap()
                .state
                .peer(0)
                .unwrap()
                .text,
            b"name"
        ),
        b"Departed"
    );
    assert!(host.runtime.disconnect_client(clients[0]));
    host.local_clients[SeatId::FIRST.index()] = None;
    advance(&mut host, &mut source);
    let peer = host.runtime.client_resources[other.0 as usize]
        .as_ref()
        .unwrap()
        .state
        .peer(0)
        .unwrap();
    assert!(peer.text.is_empty());
    assert_eq!(peer.metadata.user_id, Some(old.user_id));
    let replacement = host
        .runtime
        .connect_local(
            SeatId::FIRST,
            SpawnAnchor {
                position: Default::default(),
                angles: Default::default(),
                entity: 0,
                fixture_fallback: false,
            },
            ClientPolicy::select(Some(RuleSetId::Quake3), None, None, None, None).unwrap(),
            Protocol::QuakeWorld28,
            5,
        )
        .unwrap();
    assert_eq!(replacement, clients[0]);
    host.local_clients[SeatId::FIRST.index()] = Some(replacement);
    advance(&mut host, &mut source);
    host.runtime
        .server
        .replace_userinfo_raw(replacement, b"\\name\\Replacement", 2048)
        .unwrap();
    advance(&mut host, &mut source);
    let current = host
        .runtime
        .server
        .userinfo_record(replacement)
        .unwrap()
        .stamp;
    assert_ne!(current.lifetime, old.lifetime);
    assert_ne!(current.user_id, old.user_id);
    assert_eq!(
        host.runtime.snapshots[replacement.0 as usize]
            .as_ref()
            .unwrap()
            .native_client(),
        Some(5)
    );
    let peer = host.runtime.client_resources[other.0 as usize]
        .as_ref()
        .unwrap()
        .state
        .peer(0)
        .unwrap();
    assert_eq!(field(peer.text, b"name"), b"Replacement");
    assert_eq!(peer.metadata.user_id, Some(current.user_id));
}

#[test]
fn peer_namespace_reseed_preserves_an_earlier_native_score() {
    use qa_core::primitives::{ModuleId, NativeDelivery, NativeTarget, Vec3};
    let (mut host, clients) = pair([0, 1], None);
    let mut bytes = [0; 8];
    let length = qa_network::outputs::peer_update(
        Protocol::QuakeWorld28,
        qa_network::outputs::PeerUpdate::Frags {
            slot: 0,
            frags: 123,
        },
        &mut bytes,
    )
    .unwrap();
    host.runtime
        .server
        .events
        .native_message(
            NativeDelivery {
                module: ModuleId::default(),
                source: RuleSetId::QuakeWorld,
                target: NativeTarget::Client(clients[0]),
                origin: Vec3::default(),
                reliable: true,
                dupe_key: 0,
                component: 0,
                emission: None,
                check_portals: false,
            },
            &bytes[..length],
        )
        .unwrap();
    advance(&mut host, &mut Source::default());
    assert_eq!(
        host.runtime.client_resources[clients[0].0 as usize]
            .as_ref()
            .unwrap()
            .state
            .peer(0)
            .unwrap()
            .metadata
            .frags,
        123
    );
}

#[test]
fn canceled_tombstone_is_recovered_by_the_existing_local_namespace_resync() {
    use qa_core::primitives::PrintKind;
    let (mut host, clients) = pair([2, 7], Some(8));
    let mut source = Source::default();
    for _ in 0..4 {
        advance(&mut host, &mut source);
    }
    host.runtime
        .server
        .replace_userinfo_raw(clients[0], b"\\name\\Departed", 2048)
        .unwrap();
    advance(&mut host, &mut source);
    let healthy = clients[1];
    let peer = host.runtime.client_resources[healthy.0 as usize]
        .as_ref()
        .unwrap()
        .state
        .peer(0)
        .unwrap();
    assert_eq!(field(peer.text, b"name"), b"Departed");
    let old_output = host.runtime.server.clients[healthy.0 as usize]
        .output
        .unwrap();
    assert!(host.runtime.disconnect_client(clients[0]));
    host.local_clients[0] = None;
    // Actual ring overflow cancels the unsubmitted tombstone. The same local
    // recovery path must clear its obsolete native slot before reseeding peers.
    for _ in 0..16 {
        host.runtime
            .server
            .events
            .print(Some(healthy), PrintKind::Console, format_args!("resync"))
            .unwrap();
    }
    assert!(host.runtime.server.events.needs_resync(old_output));
    for _ in 0..8 {
        advance(&mut host, &mut source);
    }
    assert_ne!(
        host.runtime.server.clients[healthy.0 as usize].output,
        Some(old_output)
    );
    let state = &host.runtime.client_resources[healthy.0 as usize]
        .as_ref()
        .unwrap()
        .state;
    assert!(state.peer(0).unwrap().text.is_empty());
    assert!(!field(state.peer(7).unwrap().text, b"name").is_empty());
    assert!(host.runtime.server.clients[healthy.0 as usize].is_active());
}

#[test]
fn stalled_remote_peer_is_bounded_without_stopping_healthy_userinfo_or_server_ticks() {
    use qa_core::{loopback::Endpoint, primitives::ModuleId, sys_events::Peer};
    use qa_network::{
        channel::Channel,
        commands::connection::Commands,
        ingress::{Connection as NativeConnection, ConnectionPhase, Route},
    };
    use qa_session::clients::Connection;
    let (mut host, clients) = pair([2, 7], Some(2));
    let mut source = Source::default();
    for _ in 0..4 {
        advance(&mut host, &mut source);
    }
    let stalled = host
        .runtime
        .server
        .connect(Connection::Remote, ModuleId::default(), None)
        .unwrap();
    let mut channel =
        Channel::load(Protocol::QuakeWorld28.channel(), Endpoint::Server, 8192, 16).unwrap();
    channel.configure_snapshots(Protocol::QuakeWorld28).unwrap();
    channel.set_qport(2000);
    host.runtime
        .network
        .bind(
            stalled,
            Endpoint::Server,
            NativeConnection {
                phase: ConnectionPhase::Connected,
                route: Route {
                    socket: 0,
                    peer: Peer::Socket(([127, 0, 0, 1], 2000).into()),
                },
                channel,
                output: host.runtime.server.clients[stalled.0 as usize].output,
                pending_output: None,
                commands: Some(Commands::load(Protocol::QuakeWorld28, 8192)),
            },
        )
        .unwrap();
    let before = host.runtime.server.world_frame;
    host.runtime
        .server
        .replace_userinfo_raw(clients[0], b"\\name\\HealthyLatest", 2048)
        .unwrap();
    let mut disconnected = 0;
    for _ in 0..160 {
        source.time += 20;
        let result = host.frame(&mut source, true);
        assert_eq!(result.drains, 2);
        disconnected += result.output.native_disconnected;
        assert!(host.runtime.server.clients[clients[0].0 as usize].is_active());
        assert!(host.runtime.server.clients[clients[1].0 as usize].is_active());
    }
    assert_eq!(disconnected, 1);
    assert!(
        host.runtime.server.clients[stalled.0 as usize]
            .connection
            .is_none()
    );
    assert!(host.runtime.server.world_frame > before + 100);
    for (receiver, slot) in [(clients[0], 2), (clients[1], 0)] {
        let peer = host.runtime.client_resources[receiver.0 as usize]
            .as_ref()
            .unwrap()
            .state
            .peer(slot)
            .unwrap();
        assert_eq!(field(peer.text, b"name"), b"HealthyLatest");
    }
}
