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

#[test]
fn explicit_native_clients_use_protocol_limits_independently_of_common_capacity() {
    for protocol in [
        Protocol::NetQuake15,
        Protocol::QuakeWorld28,
        Protocol::Quake2_34,
        Protocol::Quake2Repro1038,
        Protocol::Quake3_68,
    ] {
        let capacity = qa_network::snapshots::snapshot_limits(protocol).client_capacity;
        for (number, admitted) in [(capacity - 1, true), (capacity, false)] {
            let mut runtime = Runtime::load(1, []).unwrap();
            let result = runtime.connect_local(
                SeatId::FIRST,
                SpawnAnchor {
                    position: Vec3::default(),
                    angles: Vec3::default(),
                    entity: 0,
                    fixture_fallback: false,
                },
                ClientPolicy::select(Some(RuleSetId::Quake3), None, None, None, None).unwrap(),
                protocol,
                number,
            );
            assert_eq!(
                result.is_ok(),
                admitted,
                "{protocol:?} native client {number}"
            );
            if admitted {
                assert_eq!(runtime.native_numbers[0].native_clients(), capacity);
            } else {
                assert!(runtime.server.clients[0].connection.is_none());
            }
        }
    }
}

const RESOURCES: ModuleId = ModuleId(7);
const ENTITIES: ModuleId = ModuleId(8);
const REMOTE: u16 = 24000;
const SERVER: u16 = 24001;

#[derive(Default)]
struct Source {
    time: EventTime,
    limit: Option<usize>,
    packets: Vec<Vec<u8>>,
    polls: usize,
}
impl FrameSource for Source {
    fn begin_frame(&mut self) -> Duration {
        Duration::from_nanos(self.time.0)
    }
    fn poll_events(&mut self, queue: &mut SysEventQueue) {
        self.polls += 1;
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
                |_| 0,
                &connection.channel,
                &mut bytes,
            )
            .unwrap();
        let request = commands.snapshot_request(&connection.channel);
        let packet = connection
            .channel
            .prepare_move(&bytes[..length], time, request, None)
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
            ClientPolicy::select(Some(RuleSetId::Quake3), None, None, None, None).unwrap(),
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
fn native_server_disconnect_retires_only_its_client_and_healthy_ticks_continue() {
    use qa_network::headers::{self, Direction, Header};
    for (protocol, format) in [
        (Protocol::NetQuake15, headers::NETQUAKE),
        (Protocol::QuakeWorld28, headers::QUAKEWORLD),
        (Protocol::Quake2_34, headers::QUAKE2),
        (Protocol::Quake2Repro1038, headers::q2_new(false)),
    ] {
        let (mut host, local, healthy, mut peer) = host(protocol);
        let mut source = Source::default();
        host.frame(&mut source, true);
        source.packets.clear();
        source.polls = 0;
        let lifetime = host.runtime.server.clients[healthy.0 as usize].entity;
        let time = EventTime::from_milliseconds(100);
        let mut body = [255; 2];
        assert_eq!(qa_network::outputs::disconnect(protocol, &mut body), Ok(1));
        // The byte after disconnect is deliberately not a supported service.
        // A terminal command cannot parse it or publish a later snapshot.
        let mut bytes = [0; 1400];
        let length = headers::encode(
            format,
            Direction::ToClient,
            Header {
                sequence: 100,
                datagram_flags: headers::datagram::UNRELIABLE,
                ..Header::default()
            },
            &body,
            &mut bytes,
        )
        .unwrap();
        host.queue
            .push(SysEvent {
                time,
                kind: EventKind::Packet {
                    socket: Endpoint::Client.socket(),
                    from: Peer::Loopback(local),
                    bytes: &bytes[..length],
                },
            })
            .unwrap();
        peer.queue_command(&mut host, time);
        source.time = time;
        let result = host.frame(&mut source, true);
        assert_eq!(source.polls, 2);
        assert!(result.server_ticks > 0);
        assert_eq!(
            host.runtime.server.clients[local.0 as usize].connection,
            None
        );
        assert_eq!(host.local_clients[SeatId::FIRST.index()], None);
        assert!(host.runtime.snapshots[local.0 as usize].is_none());
        assert!(host.runtime.client_resources[local.0 as usize].is_none());
        for endpoint in [Endpoint::Client, Endpoint::Server] {
            assert!(host.runtime.network.get(local, endpoint).is_none());
        }
        assert_eq!(host.runtime.network.malformed, 0, "{protocol:?}");
        assert_eq!(
            host.runtime.server.clients[healthy.0 as usize].entity,
            lifetime
        );
        assert_eq!(
            host.runtime.server.clients[healthy.0 as usize].connection,
            Some(Connection::Remote)
        );
        peer.receive(&source.packets, time);
        assert!(peer.frame().is_some(), "healthy {protocol:?} snapshot");
        source.time = EventTime::from_milliseconds(125);
        source.packets.clear();
        peer.queue_command(&mut host, source.time);
        assert!(host.frame(&mut source, true).server_ticks > 0);
        assert_eq!(source.polls, 4);
        peer.receive(&source.packets, source.time);
        assert_eq!(
            host.runtime.server.clients[healthy.0 as usize].entity,
            lifetime
        );
    }
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
        assert!(
            host.runtime
                .send_snapshot(remote, time, None, &mut source, |_, _| None)
        );
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
                .send_snapshot(remote, source.time, None, &mut source, |_, _| None)
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
        assert!(!host.runtime.send_snapshot(
            remote,
            time,
            Some(observation),
            &mut source,
            |_, _| None
        ));
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
        assert!(
            host.runtime
                .send_snapshot(local, time, None, &mut source, |_, _| None)
        );
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

struct NoPredictionQueries;

fn queue_nq_view_snapshot(
    host: &mut FrameHost,
    client: ClientId,
    seconds: f64,
    origin: Option<Vec3>,
) {
    use qa_network::{
        message::{Encoding, Writer},
        projection::{EntityContext, EntityProjection},
        snapshots::{Entity, Frame},
        states,
    };
    let projection = EntityProjection::load(Protocol::NetQuake15);
    let mut entities = Vec::new();
    for (number, position) in [(1, Some(Vec3([-200., -300., -400.]))), (6, origin)] {
        let Some(position) = position else {
            continue;
        };
        let record = EntityRecord {
            position,
            angles: Vec3([88., 99., 111.]),
            appearance: EntityAppearance {
                source: RuleSetId::Quake,
                models: [
                    ModelRef::native(ResourceNamespace::Module(ENTITIES), 9),
                    None,
                    None,
                    None,
                ],
                ..Default::default()
            },
            ..Default::default()
        };
        let mut words = [0; states::NQ_ENTITY_WORDS];
        assert!(projection.reduce(
            &record,
            &EntityContext::default(),
            &mut words,
            |model| Some(model.ordinal),
            |_| None,
            |_| None,
        ));
        entities.push(Entity { number, words });
    }
    let mut player = [0; states::NQ_PLAYER_WORDS];
    player[0] = 22.0f32.to_bits();
    player[3] = 32.0f32.to_bits();
    player[5] = (-16.0f32).to_bits();
    player[7] = 48.0f32.to_bits();
    player[11] = 2;
    player[12] = 87.0f32.to_bits();
    let channel = &mut host
        .runtime
        .network
        .get_mut(client, Endpoint::Server)
        .unwrap()
        .channel;
    let sequence = channel.send_state().sequence;
    let frame = ReceivedFrame::NetQuake(Frame {
        sequence,
        time: ThinkTime::Seconds(seconds),
        command: 0,
        flags: 0,
        qw_peers: 0,
        qw_dead: 0,
        areas: &[],
        player: &player,
        entities: &entities,
    });
    let mut bytes = [0; 1400];
    let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
    channel
        .write_snapshot_candidate(&mut writer, frame, None, 16)
        .unwrap();
    let length = writer.size();
    channel.publish_written_snapshot(frame, None).unwrap();
    let now = EventTime::from_milliseconds(u64::from(sequence) * 100 + 100);
    channel
        .prepare_snapshot(sequence, None, &bytes[..length], now)
        .unwrap()
        .unwrap();
    assert!(
        channel
            .submit_with(now, |packet| host
                .runtime
                .loopback
                .send(Endpoint::Server, client, packet)
                .is_ok())
            .unwrap()
            .is_some()
    );
    assert!(!channel.pending_fragments());
    host.time = now;
}

#[test]
fn queued_netquake_view_entity_updates_only_client_origin_across_native_frames() {
    let spawn = Vec3([10., 20., 30.]);
    let angles = Vec3([13., 27., 39.]);
    let mut runtime = Runtime::load(1, []).unwrap();
    let client = runtime
        .connect_local(
            SeatId::FIRST,
            SpawnAnchor {
                position: spawn,
                angles,
                entity: 0,
                fixture_fallback: false,
            },
            ClientPolicy::select(
                Some(RuleSetId::QuakeWorld),
                None,
                Some(RuleSetId::Quake3),
                Some(RuleSetId::Quake2),
                None,
            )
            .unwrap(),
            Protocol::NetQuake15,
            5,
        )
        .unwrap();
    runtime.server.clients[client.0 as usize].module = RESOURCES;
    let server_entity = runtime.server.clients[client.0 as usize].entity;
    assert_ne!(server_entity.slot, 6);
    let server_body = runtime.server.clients[client.0 as usize].player.body;
    let mut host = FrameHost::load(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::fixed(25).unwrap(),
        vec![],
    )
    .unwrap();
    host.local_clients[0] = Some(client);
    host.local_worlds[0] = Some((
        qa_render::world::WorldId(0),
        qa_render::world::WorldPresentationId(0),
    ));
    let (mins, maxs) = {
        let body = host.runtime.prediction[0].player.body;
        (body.mins, body.maxs)
    };
    let mut accepted = spawn;
    for (index, (seconds, origin)) in [
        (1.0, Some(Vec3([120.375, -24.625, 48.125]))),
        (1.0, Some(Vec3([128.5, -32.25, 56.75]))),
        (2.0, Some(Vec3([-1024.125, 2048.25, 512.5]))),
        (3.0, None),
    ]
    .into_iter()
    .enumerate()
    {
        queue_nq_view_snapshot(&mut host, client, seconds, origin);
        assert_eq!(host.runtime.prediction[0].player.body.position, accepted);
        host.drain(&mut qa_app::host::FrameResult::default());
        accepted = origin.unwrap_or(accepted);
        let player = &host.runtime.prediction[0].player;
        assert_eq!(player.body.position, accepted, "frame {index}");
        assert_eq!(player.body.velocity, Vec3([32., -16., 48.]));
        assert_eq!(player.view_angles, angles);
        assert_eq!((player.body.mins, player.body.maxs), (mins, maxs));
        assert_eq!(
            (player.movement_rules, player.trace_rules),
            (RuleSetId::Quake3, RuleSetId::Quake2)
        );
        assert_eq!(
            player.view_weapon.model,
            ModelRef::native(ResourceNamespace::Client(client), 2)
        );
        assert_eq!(player.health, 87);
        assert_eq!(
            player.native_clock.unwrap().native,
            ThinkTime::Seconds(seconds)
        );
        let binding = host.runtime.snapshots[client.0 as usize].as_ref().unwrap();
        assert_eq!((binding.applied, binding.rejected), (index as u64 + 1, 0));
        assert_eq!(binding.native_client(), 5);
        let frame = host
            .runtime
            .network
            .get(client, Endpoint::Client)
            .unwrap()
            .channel
            .current_snapshot()
            .unwrap();
        assert_eq!(frame.time(), ThinkTime::Seconds(seconds));
        assert_eq!(binding.view_entity(Some(frame)), Some(6));
        assert_eq!(
            host.client_views()[0].unwrap().origin,
            accepted + Vec3([0., 0., 22.])
        );
        let server = &host.runtime.server.clients[client.0 as usize];
        assert_eq!(server.module, RESOURCES);
        for body in [
            server.player.body,
            host.runtime
                .server
                .entities
                .columns
                .body(server_entity.slot as usize),
        ] {
            assert_eq!(body.position, server_body.position);
            assert_eq!(body.velocity, server_body.velocity);
            assert_eq!(body.mins, server_body.mins);
            assert_eq!(body.maxs, server_body.maxs);
        }
        assert_eq!(host.runtime.network.command_errors, 0);
        assert_eq!(host.runtime.network.malformed, 0);
    }
}
impl qa_movement::TraceServices for NoPredictionQueries {
    fn trace(
        &mut self,
        _: qa_world::collision::TraceQuery,
    ) -> Result<qa_world::collision::Trace, qa_world::collision::TraceError> {
        panic!("queued nonpositional or noclip prediction must not trace")
    }
    fn point_contents(
        &self,
        _: Vec3,
        _: qa_world::collision::EntityTracePolicy,
    ) -> Result<qa_world::collision::Contents, qa_world::collision::TraceError> {
        panic!("queued nonpositional or noclip prediction must not sample contents")
    }
}

#[test]
fn queued_native_flags_select_position_and_angle_permissions_independently_of_movement() {
    for (protocol, flags, expected) in [
        (
            Protocol::Quake2_34,
            64,
            PredictionPolicy {
                position: false,
                angles: true,
            },
        ),
        (
            Protocol::Quake2Repro1038,
            64,
            PredictionPolicy {
                position: false,
                angles: true,
            },
        ),
        (
            Protocol::Quake2Repro1038,
            256,
            PredictionPolicy {
                position: true,
                angles: false,
            },
        ),
        (Protocol::Quake2Repro1038, 320, PredictionPolicy::DISABLED),
    ] {
        let (mut host, local, _, _) = host(protocol);
        let slot = local.0 as usize;
        let player = &mut host.runtime.server.clients[slot].player;
        player.movement_rules = RuleSetId::Quake3;
        player.trace_rules = RuleSetId::QuakeWorld;
        player.movement.mode = MovementMode::Noclip;
        player.movement.delta_angles = AngleOffsets([AngleOffset::Degrees(45.0); 3]);
        player.view_angles = Vec3([-0.0, 90.0, 0.0]);
        let mut native =
            NativePlayerTail::new(protocol.rule_set(), ResourceNamespace::Module(ENTITIES));
        native.flags = flags;
        player.native = Some(native);
        // Capability identities are explicit CLIENT choices, not wire fields.
        host.runtime.prediction[0].player.trace_rules = RuleSetId::QuakeWorld;
        let time = EventTime::from_milliseconds(100);
        let mut source = Source {
            time,
            ..Default::default()
        };
        assert!(
            host.runtime
                .send_snapshot(local, time, None, &mut source, |_, _| None)
        );

        // Deliberately diverge SERVER after submission. CLIENT permissions and
        // angles must come from the accepted packet, not the live server row.
        let server = &mut host.runtime.server.clients[slot].player;
        server.native.as_mut().unwrap().flags = 0;
        server.view_angles = Vec3([999.0; 3]);
        server.body.position = Vec3([999.0; 3]);
        host.queue
            .push(SysEvent {
                time,
                kind: EventKind::Time,
            })
            .unwrap();
        host.drain(&mut Default::default());
        let prediction = &mut host.runtime.prediction[0];
        assert_eq!(
            prediction.player.prediction, expected,
            "{protocol:?} {flags}"
        );
        assert_eq!(prediction.player.native.unwrap().flags & 320, flags);
        assert_eq!(
            (
                prediction.player.movement_rules,
                prediction.player.trace_rules
            ),
            (RuleSetId::Quake3, RuleSetId::QuakeWorld)
        );
        assert_eq!(prediction.player.view_angles.0[1], 90.0);
        let before = prediction.player.body;
        let accepted_view = prediction.player.view_angles;
        let movement = prediction.player.movement;
        let command = UserCmd {
            duration_ms: 16,
            server_time_ms: 116,
            movement: [127.0, 0.0, 0.0],
            units: qa_gameplay::rules::command_units(RuleSetId::Quake3),
            view_angles: Vec3([179.0, 0.0, -19.0]),
            ..Default::default()
        };
        prediction
            .advance(command, &mut NoPredictionQueries)
            .unwrap();
        if expected.position {
            assert!(prediction.player.body.position.0[1] > before.position.0[1]);
        } else {
            for (actual, original) in [
                (prediction.player.body.position, before.position),
                (prediction.player.body.velocity, before.velocity),
                (prediction.player.body.mins, before.mins),
                (prediction.player.body.maxs, before.maxs),
            ] {
                assert_eq!(actual.0.map(f32::to_bits), original.0.map(f32::to_bits));
            }
            assert_eq!(
                prediction.player.movement.command_time_ms,
                movement.command_time_ms
            );
            assert_eq!(
                prediction.player.movement.remaining_ms,
                movement.remaining_ms
            );
            assert_eq!(prediction.player.movement.timer, movement.timer);
        }
        let view = if expected.angles {
            // Foreign protocol flags select permissions, while this player
            // retains the Q3 movement role's native angle kernel.
            Vec3(std::array::from_fn(|axis| {
                let short = qa_core::math::angle_to_short(
                    command.view_angles.0[axis],
                    qa_core::math::AngleShortForm::Factored,
                )
                .wrapping_add(
                    movement.delta_angles.0[axis].units(qa_core::math::AngleShortForm::Factored),
                ) as i16;
                qa_core::math::short_to_angle(i32::from(if axis == 0 {
                    short.clamp(-16000, 16000)
                } else {
                    short
                }))
            }))
        } else {
            accepted_view
        };
        assert_eq!(
            prediction.player.view_angles.0.map(f32::to_bits),
            view.0.map(f32::to_bits)
        );
        assert_eq!(
            host.runtime.server.clients[slot].player.body.position,
            Vec3([999.0; 3])
        );
        assert_eq!(
            host.runtime.server.clients[slot]
                .player
                .native
                .unwrap()
                .flags,
            0
        );
    }
}
