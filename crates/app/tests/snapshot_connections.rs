use qa_app::{
    Runtime,
    client_policy::ClientPolicy,
    host::{FrameHost, FrameSource},
    map::SpawnAnchor,
};
use qa_console::{commands::Console, views::Context};
use qa_core::{loopback::Endpoint, primitives::*, sys_events::*};
use qa_network::{
    channel::Channel,
    commands::{connection::Commands, packet::Protocol},
    ingress::{Connection as NativeConnection, Connections, Route},
    snapshots::ReceivedFrame,
};
use qa_session::{clients::Connection, timing::TickRate};
use std::{ops::ControlFlow, time::Duration};

const RESOURCES: ModuleId = ModuleId(7);
const ENTITIES: ModuleId = ModuleId(8);
const REMOTE: u16 = 24000;
const SERVER: u16 = 24001;

#[derive(Default)]
struct Source {
    time: EventTime,
    limit: Option<usize>,
    packets: Vec<Vec<u8>>,
}
impl FrameSource for Source {
    fn begin_frame(&mut self) -> Duration {
        Duration::from_nanos(self.time.0)
    }
    fn poll_events(&mut self, queue: &mut SysEventQueue) {
        queue
            .push(SysEvent {
                time: self.time,
                kind: EventKind::Time,
            })
            .unwrap();
    }
    fn wait_time(&mut self, _: Duration) -> Duration {
        Duration::from_nanos(self.time.0)
    }
    fn elapsed(&self) -> Duration {
        Duration::ZERO
    }
    fn present(&mut self) {}
    fn send_packet(&mut self, socket: u16, peer: std::net::SocketAddr, bytes: &[u8]) -> bool {
        assert_eq!(socket, 0);
        assert_eq!(peer.port(), REMOTE);
        if self.limit.is_some_and(|limit| self.packets.len() >= limit) {
            return false;
        }
        self.packets.push(bytes.to_vec());
        true
    }
}

struct RemoteReceiver {
    client: ClientId,
    connections: Connections,
    queue: SysEventQueue,
    prints: Vec<Vec<u8>>,
}
impl RemoteReceiver {
    fn load(client: ClientId, protocol: Protocol) -> Self {
        let mut channel = Channel::load(protocol.channel(), Endpoint::Client, 8192, 16).unwrap();
        channel.set_qport(REMOTE);
        let mut connections = Connections::load(2);
        connections
            .bind(
                client,
                Endpoint::Client,
                NativeConnection {
                    route: Route {
                        socket: 0,
                        peer: PeerAddress::server(),
                    },
                    channel,
                    output: None,
                    pending_output: None,
                    commands: Some(Commands::load(protocol, 8192)),
                },
            )
            .unwrap();
        Self {
            client,
            connections,
            queue: SysEventQueue::load(64, 128_000).unwrap(),
            prints: Vec::new(),
        }
    }
    fn queue_command(&mut self, host: &mut FrameHost, time: EventTime) {
        let connection = self
            .connections
            .get_mut(self.client, Endpoint::Client)
            .unwrap();
        let commands = connection.commands.as_ref().unwrap();
        let mut bytes = [0; 1400];
        let length = commands
            .encode(
                &UserCmd {
                    server_time_ms: time.milliseconds() as i32,
                    duration_ms: 16,
                    ..Default::default()
                },
                0,
                &connection.channel,
                &mut bytes,
            )
            .unwrap();
        let request = commands.snapshot_request(&connection.channel);
        let packet = connection
            .channel
            .prepare_move(&bytes[..length], time, request)
            .unwrap()
            .unwrap()
            .bytes
            .to_vec();
        connection.channel.submitted(time).unwrap();
        host.queue
            .push(SysEvent {
                time,
                kind: EventKind::Packet {
                    socket: 0,
                    from: PeerAddress::remote(),
                    bytes: &packet,
                },
            })
            .unwrap();
    }
    fn receive(&mut self, packets: &[Vec<u8>], time: EventTime) {
        for bytes in packets {
            self.queue
                .push(SysEvent {
                    time,
                    kind: EventKind::Packet {
                        socket: 0,
                        from: PeerAddress::server(),
                        bytes,
                    },
                })
                .unwrap();
        }
        while let Some(event) = self.queue.pop() {
            if let EventKind::Packet {
                socket,
                from,
                bytes,
            } = event.kind
            {
                self.connections
                    .receive(socket, from, bytes, event.time, |_, _, incoming| {
                        if let qa_network::ingress::Incoming::Print(print) = incoming {
                            self.prints.push(print.text.to_vec());
                        }
                        ControlFlow::Continue(())
                    });
            }
        }
        assert_eq!(self.connections.malformed, 0);
        assert_eq!(self.connections.command_errors, 0);
    }
    fn frame(&self) -> Option<ReceivedFrame<'_>> {
        self.connections
            .get(self.client, Endpoint::Client)
            .unwrap()
            .channel
            .current_snapshot()
    }
}

struct PeerAddress;
impl PeerAddress {
    fn remote() -> qa_core::sys_events::Peer {
        qa_core::sys_events::Peer::Socket(([127, 0, 0, 1], REMOTE).into())
    }
    fn server() -> qa_core::sys_events::Peer {
        qa_core::sys_events::Peer::Socket(([127, 0, 0, 1], SERVER).into())
    }
}

fn host(protocol: Protocol) -> (FrameHost, ClientId, ClientId, RemoteReceiver) {
    let mut runtime = Runtime::load(2, []).unwrap();
    let local = runtime
        .connect_local(
            SeatId::FIRST,
            SpawnAnchor {
                position: Vec3([10., 0., 0.]),
                angles: Vec3::default(),
                entity: 0,
                fixture_fallback: false,
            },
            ClientPolicy::select(Some(RuleSetId::Quake3), None, None, None).unwrap(),
            protocol,
            0,
        )
        .unwrap();
    assert!(runtime.server.bind_client_native(
        local,
        NativeEntity {
            module: ENTITIES,
            slot: if protocol == Protocol::Quake3_68 {
                0
            } else {
                1
            },
        }
    ));
    runtime.server.clients[local.0 as usize].module = RESOURCES;
    let remote = runtime
        .server
        .connect(Connection::Remote, RESOURCES, None)
        .unwrap();
    assert!(runtime.server.bind_client_native(
        remote,
        NativeEntity {
            module: ENTITIES,
            slot: if protocol == Protocol::Quake3_68 {
                1
            } else {
                2
            },
        }
    ));
    runtime.server.clients[remote.0 as usize].module = RESOURCES;
    runtime.server.clients[remote.0 as usize]
        .player
        .body
        .position = Vec3([20., 0., 0.]);
    let entity = runtime.server.clients[remote.0 as usize].entity;
    runtime.server.entities.columns.set_body(
        entity.slot as usize,
        runtime.server.clients[remote.0 as usize].player.body,
    );
    let mut channel = Channel::load(protocol.channel(), Endpoint::Server, 8192, 16).unwrap();
    channel.set_qport(REMOTE);
    runtime
        .bind_server_connection(
            remote,
            NativeConnection {
                route: Route {
                    socket: 0,
                    peer: PeerAddress::remote(),
                },
                channel,
                output: None,
                pending_output: None,
                commands: Some(Commands::load(protocol, 8192)),
            },
            1,
            None,
        )
        .unwrap();
    let peer = RemoteReceiver::load(remote, protocol);
    let mut host = FrameHost::load(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::fixed(25).unwrap(),
        vec![],
    )
    .unwrap();
    host.local_clients[SeatId::FIRST.index()] = Some(local);
    (host, local, remote, peer)
}

fn add(host: &mut FrameHost, protocol: Protocol, number: i32, ordinal: u32) -> EntityId {
    let entity = host
        .runtime
        .server
        .entities
        .allocate(
            ThinkTime::Seconds(0.),
            ENTITIES,
            qa_world::entities::AllocationPolicy::EDICT,
        )
        .unwrap()
        .id;
    let columns = &mut host.runtime.server.entities.columns;
    let slot = entity.slot as usize;
    columns.native_entity[slot] = Some(NativeEntity {
        module: ENTITIES,
        slot: number,
    });
    columns.position[slot] = Vec3([17., 19., 23.]);
    columns.mins[slot] = Vec3([-1.; 3]);
    columns.maxs[slot] = Vec3([1.; 3]);
    columns.appearance[slot] = EntityAppearance {
        source: protocol.rule_set(),
        models: [
            ModelRef::native(ResourceNamespace::Module(RESOURCES), ordinal),
            None,
            None,
            None,
        ],
        frame: 17,
        ..Default::default()
    };
    entity
}

fn entity(
    frame: ReceivedFrame<'_>,
    protocol: Protocol,
    client: ClientId,
    number: u32,
) -> EntityRecord {
    let projection = qa_network::projection::EntityProjection::load(protocol);
    let mut record = EntityRecord::default();
    macro_rules! decode {
        ($frame:expr) => {{
            let row = $frame
                .entities
                .iter()
                .find(|row| row.number == number)
                .unwrap();
            assert!(projection.apply(&row.words, &mut record, ResourceNamespace::Client(client)));
        }};
    }
    match frame {
        ReceivedFrame::NetQuake(frame) => decode!(frame),
        ReceivedFrame::QuakeWorld(frame) => decode!(frame),
        ReceivedFrame::Quake2(frame) => decode!(frame),
        ReceivedFrame::Quake2Repro(frame) => decode!(frame),
        ReceivedFrame::Quake3(frame) => decode!(frame),
    }
    record
}

#[test]
fn queued_local_and_remote_connections_share_snapshot_projection_for_every_protocol() {
    for protocol in [
        Protocol::NetQuake15,
        Protocol::QuakeWorld28,
        Protocol::Quake2_34,
        Protocol::Quake2Repro1038,
        Protocol::Quake3_68,
    ] {
        let (mut host, local, remote, mut peer) = host(protocol);
        let actor = add(&mut host, protocol, 42, 5);
        let foreign = add(&mut host, protocol, 43, 5);
        host.runtime.server.entities.columns.appearance[foreign.slot as usize].models[0] =
            ModelRef::native(ResourceNamespace::Module(ENTITIES), 5);
        assert!(host.runtime.client_resources[remote.0 as usize].is_none());
        assert_eq!(
            host.runtime.snapshots[remote.0 as usize]
                .as_ref()
                .unwrap()
                .seat(),
            None
        );
        assert_eq!(
            host.runtime.snapshots[remote.0 as usize]
                .as_ref()
                .unwrap()
                .interpolation(),
            None
        );
        let time = EventTime::from_milliseconds(100);
        assert!(host.runtime.send_local_command(
            local,
            &UserCmd {
                duration_ms: 16,
                server_time_ms: 100,
                ..Default::default()
            },
            time
        ));
        peer.queue_command(&mut host, time);
        let mut source = Source {
            time,
            ..Default::default()
        };
        host.frame(&mut source, true);
        peer.receive(&source.packets, time);
        let local_frame = host
            .runtime
            .network
            .get(local, Endpoint::Client)
            .unwrap()
            .channel
            .current_snapshot()
            .unwrap();
        let remote_frame = peer.frame().unwrap();
        for (client, frame) in [(local, local_frame), (remote, remote_frame)] {
            let actor = entity(frame, protocol, client, 42);
            assert_eq!(actor.position, Vec3([17., 19., 23.]), "{protocol:?}");
            assert_eq!(
                actor.appearance.models[0],
                ModelRef::native(ResourceNamespace::Client(client), 5),
                "{protocol:?} {client:?}: connection resource namespace"
            );
            assert_eq!(
                entity(frame, protocol, client, 43).appearance.models[0],
                None
            );
        }
        assert_eq!(
            host.runtime.native_numbers[remote.0 as usize].number(actor),
            Some(42)
        );
        assert_eq!(host.runtime.network.command_errors, 0);
        assert!(
            host.runtime.snapshots[remote.0 as usize]
                .as_ref()
                .unwrap()
                .pending
                .is_none()
        );
    }
}

#[test]
fn queued_qw_requests_grant_one_reply_while_other_frames_keep_running() {
    let (mut host, local, remote, mut peer) = host(Protocol::QuakeWorld28);
    let mut bytes = [0; 64];
    let length = qa_network::outputs::print(
        Protocol::QuakeWorld28,
        PrintKind::Console,
        2,
        b"first request",
        None,
        &mut bytes,
    )
    .unwrap();
    host.runtime
        .network
        .get_mut(remote, Endpoint::Server)
        .unwrap()
        .channel
        .queue_reliable(&bytes[..length])
        .unwrap();
    let mut source = Source::default();
    host.frame(&mut source, true);
    assert!(source.packets.is_empty());
    for milliseconds in [100, 150] {
        source.time = EventTime::from_milliseconds(milliseconds);
        peer.queue_command(&mut host, source.time);
        host.frame(&mut source, true);
        assert_eq!(source.packets.len(), 1);
        peer.receive(&source.packets, source.time);
        source.packets.clear();
        source.time = EventTime::from_milliseconds(milliseconds + 25);
        let result = host.frame(&mut source, true);
        assert_eq!(result.server_ticks, 1);
        assert!(source.packets.is_empty());
        assert!(
            host.runtime
                .network
                .get(local, Endpoint::Client)
                .unwrap()
                .channel
                .current_snapshot()
                .is_some()
        );
    }
    assert_eq!(peer.prints, [b"first request".to_vec()]);
    assert_eq!(peer.frame().unwrap().sequence(), 2);
    assert_eq!(host.runtime.network.command_errors, 0);
}

#[test]
fn reliable_output_flushes_between_native_snapshot_boundaries() {
    for protocol in [Protocol::Quake2_34, Protocol::Quake2Repro1038] {
        let (mut host, _, remote, mut peer) = host(protocol);
        let time = EventTime::from_milliseconds(100);
        let mut source = Source {
            time,
            ..Default::default()
        };
        assert!(host.runtime.send_snapshot(remote, time, None, &mut source));
        peer.receive(&source.packets, time);
        let sequence = peer.frame().unwrap().sequence();
        let mut bytes = [0; 64];
        let length = qa_network::outputs::print(
            protocol,
            PrintKind::Console,
            2,
            b"between frames",
            None,
            &mut bytes,
        )
        .unwrap();
        host.runtime
            .network
            .get_mut(remote, Endpoint::Server)
            .unwrap()
            .channel
            .queue_reliable(&bytes[..length])
            .unwrap();
        let previous = source.packets.len();
        source.time = EventTime::from_milliseconds(101);
        assert!(
            !host
                .runtime
                .send_snapshot(remote, source.time, None, &mut source)
        );
        assert!(source.packets.len() > previous);
        peer.receive(&source.packets[previous..], source.time);
        assert_eq!(peer.prints, [b"between frames".to_vec()]);
        assert_eq!(peer.frame().unwrap().sequence(), sequence);
        assert!(
            host.runtime
                .network
                .get(remote, Endpoint::Server)
                .unwrap()
                .channel
                .send_state()
                .reliable_bytes
                > 0
        );
    }
}

#[test]
fn rejected_snapshot_fragment_keeps_provenance_pending_while_local_client_and_ticks_continue() {
    for protocol in [Protocol::Quake2Repro1038, Protocol::Quake3_68] {
        let (mut host, local, remote, mut peer) = host(protocol);
        for number in 42..300 {
            add(&mut host, protocol, number, 5);
        }
        let time = EventTime::from_milliseconds(100);
        let observation = SnapshotObservation {
            module: ENTITIES,
            module_lifetime: host.runtime.server.clients[remote.0 as usize].entity,
            native_frame: 73,
            time,
        };
        let mut source = Source {
            time,
            limit: Some(1),
            ..Default::default()
        };
        assert!(
            !host
                .runtime
                .send_snapshot(remote, time, Some(observation), &mut source)
        );
        let sequence = host.runtime.snapshots[remote.0 as usize]
            .as_ref()
            .unwrap()
            .pending
            .unwrap();
        let channel = &host
            .runtime
            .network
            .get(remote, Endpoint::Server)
            .unwrap()
            .channel;
        assert!(channel.pending_fragments());
        assert!(channel.snapshot_observation(sequence).is_none());
        peer.receive(&source.packets, time);
        assert!(peer.frame().is_none());
        assert!(host.runtime.send_snapshot(local, time, None, &mut source));
        host.frame(&mut source, true);
        assert!(
            host.runtime
                .network
                .get(local, Endpoint::Client)
                .unwrap()
                .channel
                .current_snapshot()
                .is_some()
        );
        assert_eq!(
            host.runtime.snapshots[remote.0 as usize]
                .as_ref()
                .unwrap()
                .pending,
            Some(sequence)
        );
        let accepted = source.packets.len();
        source.limit = None;
        source.time = EventTime::from_milliseconds(125);
        let result = host.frame(&mut source, true);
        assert_eq!(result.server_ticks, 1);
        assert!(
            host.runtime.snapshots[remote.0 as usize]
                .as_ref()
                .unwrap()
                .pending
                .is_none()
        );
        let channel = &host
            .runtime
            .network
            .get(remote, Endpoint::Server)
            .unwrap()
            .channel;
        assert!(!channel.pending_fragments());
        assert_eq!(channel.snapshot_observation(sequence), Some(observation));
        peer.receive(&source.packets[accepted..], source.time);
        assert_eq!(peer.frame().unwrap().sequence(), sequence);
        assert_eq!(host.runtime.network.command_errors, 0);
    }
}
