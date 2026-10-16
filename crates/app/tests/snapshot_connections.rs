#[path = "model_assets/support.rs"]
#[allow(dead_code)]
mod assets_support;

#[path = "support/client_asset_frames.rs"]
mod asset_frames;

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
    presentation_paused: bool,
    reject_reset: bool,
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
    fn presentation_ready(&self) -> bool {
        !self.presentation_paused
    }
    fn reset_presentation(&mut self, _: ClientId, _: EntityId) -> bool {
        !std::mem::take(&mut self.reject_reset)
    }
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
            // This transport fixture loads no module model registrations.
            // model_publication.rs covers models readied by real config ACKs.
            assert_eq!(
                actor.appearance.models[0], None,
                "{protocol:?} {client:?}: unpublished model"
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
        assert!(host.runtime.send_snapshot(
            remote,
            time,
            None,
            &mut source,
            |_, _| None,
            |_, _, _, _| None
        ));
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
        assert!(!host.runtime.send_snapshot(
            remote,
            source.time,
            None,
            &mut source,
            |_, _| None,
            |_, _, _, _| None
        ));
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
            |_, _| None,
            |_, _, _, _| None
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
        assert!(host.runtime.send_snapshot(
            local,
            time,
            None,
            &mut source,
            |_, _| None,
            |_, _, _, _| None
        ));
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
    camera: (u32, Vec3),
    view: Option<u16>,
) {
    queue_nq_view_snapshot_with_prefix(host, client, seconds, origin, camera, view, &[]);
}

fn queue_nq_view_snapshot_with_prefix(
    host: &mut FrameHost,
    client: ClientId,
    seconds: f64,
    origin: Option<Vec3>,
    camera: (u32, Vec3),
    view: Option<u16>,
    prefix: &[u8],
) {
    use qa_network::{
        message::{Encoding, Writer},
        projection::{EntityContext, EntityProjection},
        snapshots::{Entity, Frame},
        states,
    };
    let projection = EntityProjection::load(Protocol::NetQuake15);
    let mut entities = Vec::new();
    let player_number = host.runtime.snapshots[client.0 as usize]
        .as_ref()
        .unwrap()
        .player_entity(None)
        .unwrap();
    for (number, position) in [(camera.0, Some(camera.1)), (player_number, origin)] {
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
            |model, _usage| Some(model.ordinal),
            |_| None,
            |_| None,
        ));
        entities.push(Entity { number, words });
    }
    entities.sort_by_key(|entity| entity.number);
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
    for &byte in prefix {
        writer.write_bits(u32::from(byte), 8).unwrap();
    }
    channel
        .write_snapshot_candidate(&mut writer, frame, None, 16)
        .unwrap();
    if let Some(entity) = view {
        writer.write_bits(5, 8).unwrap();
        writer.write_bits(u32::from(entity), 16).unwrap();
    }
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

fn netquake_camera_host(native_clients: &[u32]) -> (FrameHost, Vec<ClientId>) {
    let mut runtime = Runtime::load(native_clients.len(), []).unwrap();
    let clients = native_clients
        .iter()
        .enumerate()
        .map(|(seat, &native_client)| {
            let client = runtime
                .connect_local(
                    SeatId::new(seat as u8).unwrap(),
                    SpawnAnchor {
                        position: Vec3([10., 20., 30.]),
                        angles: Vec3([13., 27., 39.]),
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
                    native_client,
                )
                .unwrap();
            runtime.server.clients[client.0 as usize].module = RESOURCES;
            client
        })
        .collect::<Vec<_>>();
    let mut host = FrameHost::load(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::fixed(25).unwrap(),
        vec![],
    )
    .unwrap();
    for (seat, &client) in clients.iter().enumerate() {
        host.local_clients[seat] = Some(client);
        host.local_worlds[seat] = Some((
            qa_render::world::WorldId(0),
            qa_render::world::WorldPresentationId(0),
        ));
    }
    (host, clients)
}

#[test]
fn queued_netquake_view_entity_updates_only_client_origin_across_native_frames() {
    let spawn = Vec3([10., 20., 30.]);
    let angles = Vec3([13., 27., 39.]);
    let (mut host, clients) = netquake_camera_host(&[5]);
    let client = clients[0];
    let server_entity = host.runtime.server.clients[client.0 as usize].entity;
    assert_ne!(server_entity.slot, 6);
    let server_body = host.runtime.server.clients[client.0 as usize].player.body;
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
        queue_nq_view_snapshot(
            &mut host,
            client,
            seconds,
            origin,
            (1, Vec3([-200., -300., -400.])),
            None,
        );
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
fn queue_netquake_control(host: &mut FrameHost, client: ClientId, bytes: &[u8]) {
    let time = EventTime(host.time.0 + 1);
    let channel = &mut host
        .runtime
        .network
        .get_mut(client, Endpoint::Server)
        .unwrap()
        .channel;
    channel
        .prepare_server_message(bytes, time)
        .unwrap()
        .unwrap();
    assert!(
        channel
            .submit_with(time, |packet| host
                .runtime
                .loopback
                .send(Endpoint::Server, client, packet)
                .is_ok())
            .unwrap()
            .is_some()
    );
    host.time = time;
}

#[test]
fn queued_netquake_setview_selects_retained_camera_without_reassigning_the_physical_player() {
    let (mut host, clients) = netquake_camera_host(&[5, 9]);
    let first = clients[0];
    let second = clients[1];
    let common = host.runtime.server.clients[first.0 as usize].entity;
    assert_ne!(common.slot, 6);
    assert_ne!(common.slot, 42);
    let server_bodies = host
        .runtime
        .server
        .clients
        .iter()
        .map(|client| {
            (
                client.entity,
                client.player.body.position,
                client.player.body.velocity,
                client.player.view_angles,
                client.player.movement,
                client.player.movement_rules,
                client.player.trace_rules,
            )
        })
        .collect::<Vec<_>>();
    queue_nq_view_snapshot(
        &mut host,
        first,
        1.0,
        Some(Vec3([120., 24., 48.])),
        (42, Vec3([-200., -300., -400.])),
        Some(42),
    );
    queue_nq_view_snapshot(
        &mut host,
        second,
        1.0,
        Some(Vec3([30., 40., 50.])),
        (42, Vec3([400., 500., 600.])),
        None,
    );
    host.drain(&mut qa_app::host::FrameResult::default());
    assert_eq!(
        host.runtime.prediction[0].player.body.position,
        Vec3([120., 24., 48.])
    );
    assert_eq!(
        host.client_views()[0].unwrap().origin,
        Vec3([-200., -300., -378.])
    );
    assert_eq!(
        host.client_views()[1].unwrap().origin,
        Vec3([30., 40., 72.])
    );
    let binding = host.runtime.snapshots[first.0 as usize].as_ref().unwrap();
    let frame = host
        .runtime
        .network
        .get(first, Endpoint::Client)
        .unwrap()
        .channel
        .current_snapshot()
        .unwrap();
    assert_eq!(binding.player_entity(Some(frame)), Some(6));
    assert_eq!(binding.view_entity(Some(frame)), Some(42));
    assert_eq!(
        (binding.native_client(), binding.applied, binding.rejected),
        (5, 1, 0)
    );
    assert_eq!(
        host.client_entities_inputs()[0]
            .unwrap()
            .get(42)
            .unwrap()
            .origin,
        Vec3([-200., -300., -400.])
    );
    let sequence = frame.sequence();
    // At this magnitude physical-eye subtraction would round the offset to32.
    // The camera must apply native22 directly to the target's retained origin.
    host.runtime.prediction[0].player.body.position.0[2] = 268_435_456.;
    let player = &host.runtime.prediction[0].player;
    let predicted_eye = player.eye();
    assert_eq!(predicted_eye.0[2], 268_435_488.);
    let physical = (
        player.body.position,
        player.body.velocity,
        player.body.mins,
        player.body.maxs,
        player.view_angles,
        player.movement,
        player.movement_rules,
        player.trace_rules,
        player.native_clock,
    );
    let acknowledged = host.runtime.prediction[0].packet_acknowledged();
    for (target, camera) in [
        (6u16, predicted_eye),
        (42, Vec3([-200., -300., -378.])),
        (u16::MAX, predicted_eye),
        (0, predicted_eye),
        (43, predicted_eye),
    ] {
        let [low, high] = target.to_le_bytes();
        queue_netquake_control(&mut host, first, &[5, low, high]);
        host.drain(&mut qa_app::host::FrameResult::default());
        let player = &host.runtime.prediction[0].player;
        assert_eq!(
            (
                player.body.position,
                player.body.velocity,
                player.body.mins,
                player.body.maxs,
                player.view_angles,
                player.movement,
                player.movement_rules,
                player.trace_rules,
                player.native_clock
            ),
            physical
        );
        assert_eq!(
            host.runtime.prediction[0].packet_acknowledged(),
            acknowledged
        );
        let binding = host.runtime.snapshots[first.0 as usize].as_ref().unwrap();
        let frame = host
            .runtime
            .network
            .get(first, Endpoint::Client)
            .unwrap()
            .channel
            .current_snapshot()
            .unwrap();
        assert_eq!(
            (binding.native_client(), binding.applied, binding.rejected),
            (5, 1, 0)
        );
        assert_eq!(frame.sequence(), sequence);
        assert_eq!(binding.player_entity(Some(frame)), Some(6));
        assert_eq!(binding.view_entity(Some(frame)), Some(u32::from(target)));
        assert_eq!(
            host.client_views()[0].unwrap().origin,
            camera,
            "target {target}"
        );
        assert_eq!(
            host.client_views()[1].unwrap().origin,
            Vec3([30., 40., 72.])
        );
        let second_binding = host.runtime.snapshots[second.0 as usize].as_ref().unwrap();
        assert_eq!(
            (
                second_binding.view_entity(None),
                second_binding.native_client(),
                second_binding.applied
            ),
            (Some(10), 9, 1)
        );
    }
    // An absent target keeps its metadata and resolves when a later accepted row appears.
    queue_nq_view_snapshot(
        &mut host,
        first,
        2.0,
        Some(Vec3([121., 25., 49.])),
        (43, Vec3([50., 60., 70.])),
        None,
    );
    host.drain(&mut qa_app::host::FrameResult::default());
    assert_eq!(
        host.client_views()[0].unwrap().origin,
        Vec3([50., 60., 92.])
    );
    assert_eq!(
        host.runtime.prediction[0].player.body.position,
        Vec3([121., 25., 49.])
    );
    let before = (
        host.runtime.network.command_errors,
        host.runtime.network.malformed,
    );
    queue_netquake_control(&mut host, first, &[5, 42]);
    host.drain(&mut qa_app::host::FrameResult::default());
    let binding = host.runtime.snapshots[first.0 as usize].as_ref().unwrap();
    assert_eq!(
        (
            binding.view_entity(None),
            binding.player_entity(None),
            binding.applied
        ),
        (Some(43), Some(6), 2)
    );
    assert_eq!(
        host.client_views()[0].unwrap().origin,
        Vec3([50., 60., 92.])
    );
    assert_eq!(
        (
            host.runtime.network.command_errors,
            host.runtime.network.malformed
        ),
        (before.0 + 1, before.1)
    );
    assert_eq!(
        host.runtime
            .server
            .clients
            .iter()
            .map(|client| {
                (
                    client.entity,
                    client.player.body.position,
                    client.player.body.velocity,
                    client.player.view_angles,
                    client.player.movement,
                    client.player.movement_rules,
                    client.player.trace_rules,
                )
            })
            .collect::<Vec<_>>(),
        server_bodies
    );
    let body = host
        .runtime
        .server
        .entities
        .columns
        .body(common.slot as usize);
    assert_eq!(body.position, server_bodies[0].1);
    assert_eq!(body.velocity, server_bodies[0].2);
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
        assert!(host.runtime.send_snapshot(
            local,
            time,
            None,
            &mut source,
            |_, _| None,
            |_, _, _, _| None
        ));

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

fn nq_server_info(level: &[u8], models: &[&[u8]], sounds: &[&[u8]]) -> Vec<u8> {
    use qa_network::message::{Encoding, Writer};
    let mut bytes = vec![0; 8192];
    let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
    writer.write_bits(11, 8).unwrap();
    writer.write_bits(15, 32).unwrap();
    writer.write_bits(16, 8).unwrap();
    writer.write_bits(1, 8).unwrap();
    writer
        .write_string(Protocol::NetQuake15.string_policy(), level)
        .unwrap();
    for list in [models, sounds] {
        for name in list {
            writer
                .write_string(Protocol::NetQuake15.string_policy(), name)
                .unwrap();
        }
        writer
            .write_string(Protocol::NetQuake15.string_policy(), b"")
            .unwrap();
    }
    let length = writer.size();
    bytes.truncate(length);
    bytes
}

fn bind_nq_preloaded_world(host: &mut FrameHost, clients: &[ClientId]) -> GeometryId {
    use qa_app::model_assets::{ModelAssets, ModelLimits, ModelRequest};
    use qa_world::collision::{
        brushes::{BrushTree, CollisionLeaf, ModelRoot},
        surfaces::SurfaceTable,
    };
    let bounds = Bounds {
        mins: Vec3([-1024.; 3]),
        maxs: Vec3([1024.; 3]),
    };
    let geometry = host
        .runtime
        .geometry
        .load_brushes(
            vec![],
            vec![],
            vec![],
            SurfaceTable::flags(vec![]),
            BrushTree {
                planes: vec![],
                nodes: vec![],
                leaves: vec![CollisionLeaf {
                    stored_contents: None,
                    brushes: IndexSpan::default(),
                    patches: IndexSpan::default(),
                }],
                leaf_brushes: vec![],
                leaf_patches: vec![],
                models: vec![ModelRoot::Leaf(0), ModelRoot::Leaf(0)],
            },
            vec![bounds; 2],
        )
        .unwrap();
    let mut assets = qa_render::Assets::load().unwrap();
    let world = assets_support::world(&mut assets, 0.);
    let presentation =
        qa_render::WorldPresentationId(assets.world(world).unwrap().presentations().first);
    let profile = ModelRequest::native("", RuleSetId::QuakeWorld, None).profile();
    let mut owner = ModelAssets::new(assets, [profile], [world], ModelLimits::default()).unwrap();
    owner
        .bind_loaded_world("maps/ready.bsp", world, Some(geometry), bounds)
        .unwrap();
    host.runtime.model_assets = Some(owner);
    for (seat, &client) in clients.iter().enumerate() {
        host.local_worlds[seat] = Some((world, presentation));
        let state = &mut host.runtime.client_resources[client.0 as usize]
            .as_mut()
            .unwrap()
            .state;
        let index = state.layout.resources[0].get(1).unwrap();
        state.configs.set(index, b"maps/ready.bsp").unwrap();
    }
    host.bind_client_model_assets(
        clients
            .iter()
            .copied()
            .map(|client| (client, profile, Some((world, presentation)))),
    );
    geometry
}

#[test]
fn queued_netquake_serverinfo_replaces_client_resources_without_reassigning_the_player_or_peer() {
    let (mut host, clients) = netquake_camera_host(&[5, 9]);
    let first = clients[0];
    let second = clients[1];
    let geometry = bind_nq_preloaded_world(&mut host, &clients);
    queue_nq_view_snapshot(
        &mut host,
        first,
        1.,
        Some(Vec3([120., 24., 48.])),
        (42, Vec3([1., 2., 3.])),
        Some(42),
    );
    queue_nq_view_snapshot(
        &mut host,
        second,
        1.,
        Some(Vec3([30., 40., 50.])),
        (42, Vec3([4., 5., 6.])),
        Some(42),
    );
    host.drain(&mut Default::default());
    let server = host
        .runtime
        .server
        .clients
        .iter()
        .map(|row| {
            (
                row.entity,
                row.player.body.position,
                row.player.body.velocity,
                row.player.view_angles,
                row.player.movement_rules,
                row.player.trace_rules,
            )
        })
        .collect::<Vec<_>>();
    let peer_frame = host
        .runtime
        .network
        .get(second, Endpoint::Client)
        .unwrap()
        .channel
        .current_snapshot()
        .unwrap()
        .sequence();
    let peer_configs = host.runtime.client_resources[second.0 as usize]
        .as_ref()
        .unwrap()
        .state
        .configs
        .config(1)
        .unwrap()
        .revision;
    let old_world = host.local_worlds[0];
    let model = ModelRef::native(ResourceNamespace::Client(first), 1);
    {
        let resources = host.runtime.client_resources[first.0 as usize]
            .as_mut()
            .unwrap();
        resources.state.receive_static_entity(
            EntityRecord {
                appearance: EntityAppearance {
                    models: [model, None, None, None],
                    ..Default::default()
                },
                ..Default::default()
            },
            Some(host.runtime.model_assets.as_ref().unwrap().catalog()),
            old_world,
        );
        resources
            .state
            .configs
            .set(3, b"old-removed-model.mdl")
            .unwrap();
        assert_eq!(resources.state.static_entities().len(), 1);
    }
    host.runtime.prediction[0].player.inventory.collectibles = 7;
    host.runtime.prediction[0].player.health = 87;
    let level = vec![b'L'; 100];
    let header = nq_server_info(&level, &[b"maps/ready.bsp", b"*1"], &[]);
    queue_netquake_control(&mut host, first, &header);
    host.drain(&mut Default::default());
    let resources = &host.runtime.client_resources[first.0 as usize]
        .as_ref()
        .unwrap()
        .state;
    let info = resources.server_info().unwrap();
    assert_eq!((info.max_clients, info.game_type), (16, 1));
    assert_eq!(info.level_name.as_bytes(), level);
    assert_eq!(resources.max_clients(), 16);
    assert_eq!(resources.configs.config(1).unwrap().text, b"maps/ready.bsp");
    assert_eq!(resources.configs.config(2).unwrap().text, b"*1");
    assert_eq!(resources.configs.config(3).unwrap().text, b"");
    assert!(resources.static_entities().is_empty());
    assert_eq!(resources.models.collision_world(), Some((geometry, 0)));
    assert_eq!(host.local_worlds[0], old_world);
    assert!(
        host.runtime
            .network
            .get(first, Endpoint::Client)
            .unwrap()
            .channel
            .current_snapshot()
            .is_none()
    );
    let entities = host.client_entities_inputs()[0].unwrap();
    assert!(entities.current.is_none());
    assert!(entities.previous.is_none());
    assert!(entities.get(42).is_none());
    let binding = host.runtime.snapshots[first.0 as usize].as_ref().unwrap();
    assert_eq!(
        (
            binding.native_client(),
            binding.player_entity(None),
            binding.view_entity(None)
        ),
        (5, Some(6), Some(6))
    );
    assert_eq!(host.runtime.prediction[0].player.health, 0);
    assert_eq!(host.runtime.prediction[0].player.inventory.collectibles, 0);
    assert_eq!(
        (
            host.runtime.prediction[0].player.movement_rules,
            host.runtime.prediction[0].player.trace_rules
        ),
        (RuleSetId::Quake3, RuleSetId::Quake2)
    );
    assert_eq!(
        host.runtime
            .network
            .get(second, Endpoint::Client)
            .unwrap()
            .channel
            .current_snapshot()
            .unwrap()
            .sequence(),
        peer_frame
    );
    assert_eq!(
        host.runtime.client_resources[second.0 as usize]
            .as_ref()
            .unwrap()
            .state
            .configs
            .config(1)
            .unwrap()
            .revision,
        peer_configs
    );
    assert_eq!(
        host.runtime.snapshots[second.0 as usize]
            .as_ref()
            .unwrap()
            .view_entity(None),
        Some(42)
    );
    assert_eq!(
        host.runtime
            .model_assets
            .as_ref()
            .unwrap()
            .catalog()
            .collision_world(old_world.unwrap().0),
        Some(geometry)
    );

    // A truncated replacement cannot clear any admitted row or camera target.
    queue_netquake_control(&mut host, first, &[5, 42, 0]);
    host.drain(&mut Default::default());
    let revision = host.runtime.client_resources[first.0 as usize]
        .as_ref()
        .unwrap()
        .state
        .configs
        .config(1)
        .unwrap()
        .revision;
    let errors = host.runtime.network.command_errors;
    queue_netquake_control(&mut host, first, &header[..header.len() - 1]);
    host.drain(&mut Default::default());
    assert_eq!(host.runtime.network.command_errors, errors + 1);
    assert_eq!(
        host.runtime.client_resources[first.0 as usize]
            .as_ref()
            .unwrap()
            .state
            .configs
            .config(1)
            .unwrap()
            .revision,
        revision
    );
    assert_eq!(
        host.runtime.snapshots[first.0 as usize]
            .as_ref()
            .unwrap()
            .view_entity(None),
        Some(42)
    );

    // Checked header, replacement rows, SetView and native update share the
    // ordinary packet decoder and become visible in that admitted order.
    let mut prefix = nq_server_info(b"same packet", &[b"maps/ready.bsp", b"*1"], &[]);
    prefix.extend([5, 43, 0]);
    queue_nq_view_snapshot_with_prefix(
        &mut host,
        first,
        2.,
        Some(Vec3([64., 32., 16.])),
        (43, Vec3([200., 300., 400.])),
        None,
        &prefix,
    );
    host.drain(&mut Default::default());
    let binding = host.runtime.snapshots[first.0 as usize].as_ref().unwrap();
    assert_eq!(
        (binding.player_entity(None), binding.view_entity(None)),
        (Some(6), Some(43))
    );
    assert_eq!(
        host.runtime.prediction[0].player.body.position,
        Vec3([64., 32., 16.])
    );
    assert_eq!(
        host.client_views()[0].unwrap().origin,
        Vec3([200., 300., 422.])
    );
    assert_eq!(
        host.runtime.client_resources[first.0 as usize]
            .as_ref()
            .unwrap()
            .state
            .server_info()
            .unwrap()
            .level_name
            .as_bytes(),
        b"same packet"
    );
    assert_eq!(
        host.runtime
            .server
            .clients
            .iter()
            .map(|row| (
                row.entity,
                row.player.body.position,
                row.player.body.velocity,
                row.player.view_angles,
                row.player.movement_rules,
                row.player.trace_rules,
            ))
            .collect::<Vec<_>>(),
        server
    );

    host.runtime.server.clients[first.0 as usize].hud.health = 999;
    host.runtime.server.clients[first.0 as usize]
        .hud
        .inventory
        .collectibles = 7;
    let output = qa_app::output::dispatch(
        &mut host.runtime,
        &mut Source::default(),
        &mut host.console.scrollback,
        &host.local_clients,
        host.time,
        3.,
        2.,
    );
    assert_eq!(output.presentation_resets, 1);
    assert_eq!(host.runtime.server.clients[first.0 as usize].hud.health, 87);
    assert_eq!(
        host.runtime.server.clients[first.0 as usize]
            .hud
            .inventory
            .collectibles,
        0
    );

    // An unknown map does not silently reuse the previous world or collision.
    queue_netquake_control(
        &mut host,
        first,
        &nq_server_info(b"pending map", &[b"maps/not-preloaded.bsp"], &[]),
    );
    host.drain(&mut Default::default());
    assert_eq!(host.local_worlds[0], None);
    assert_eq!(
        host.runtime.client_resources[first.0 as usize]
            .as_ref()
            .unwrap()
            .state
            .models
            .collision_world(),
        None
    );
    assert!(host.client_views()[0].is_none());
    assert!(host.local_worlds[1].is_some());
}

struct ServerInfoAudio {
    mixer: qa_audio::Mixer,
    order: Vec<(bool, ClientId, Option<u32>)>,
    actions: Vec<SoundAction>,
    reject_reset: bool,
}
impl FrameSource for ServerInfoAudio {
    fn begin_frame(&mut self) -> Duration {
        Duration::ZERO
    }
    fn poll_events(&mut self, _: &mut SysEventQueue) {}
    fn wait_time(&mut self, _: Duration) -> Duration {
        Duration::ZERO
    }
    fn elapsed(&self) -> Duration {
        Duration::ZERO
    }
    fn present(&mut self) {}
    fn reset_presentation(&mut self, client: ClientId, lifetime: EntityId) -> bool {
        self.order.push((true, client, None));
        if std::mem::take(&mut self.reject_reset) {
            return false;
        }
        self.mixer.reset_client(client, lifetime, false);
        true
    }
    fn sound(&mut self, bank: Option<&qa_audio::Bank>, event: SoundEvent) -> bool {
        self.actions.push(event.action);
        self.order.push((
            false,
            event.client.unwrap(),
            match event.entity {
                Some(SoundEmitter::Static { slot, .. }) => Some(slot),
                _ => None,
            },
        ));
        self.mixer.sound(bank.unwrap(), event)
    }
}
fn nq_ambient() -> Vec<u8> {
    let mut bytes = [0; 64];
    let length = qa_network::outputs::write_cue(
        Protocol::NetQuake15,
        qa_network::outputs::Cue::StaticSound(qa_network::outputs::Sound {
            flags: 1 | 2 | 4,
            index: 1,
            volume: 255,
            attenuation: 0,
            time_offset: 0,
            entity: 0,
            channel: 0,
            position: Vec3::default(),
        }),
        &mut bytes,
    )
    .unwrap();
    bytes[..length].to_vec()
}
fn dispatch_server_info_audio(
    host: &mut FrameHost,
    source: &mut ServerInfoAudio,
) -> qa_app::output::OutputCounts {
    qa_app::output::dispatch(
        &mut host.runtime,
        source,
        &mut host.console.scrollback,
        &host.local_clients,
        host.time,
        3.,
        2.,
    )
}

#[test]
fn queued_netquake_serverinfo_reset_fence_retires_old_ambient_before_new_sound_without_retiring_listener_or_peer()
 {
    use std::num::NonZeroU32;
    let (mut host, clients) = netquake_camera_host(&[5, 9]);
    bind_nq_preloaded_world(&mut host, &clients);
    let rate = NonZeroU32::new(44100).unwrap();
    let pcm = || Pcm {
        rate,
        channels: PcmChannels::Mono,
        samples: vec![1200; 32],
        loop_start: Some(0),
        loop_end: None,
        loop_invalid: false,
    };
    let bank = qa_audio::Bank::load(
        rate,
        vec![
            ("sound/old.wav".into(), pcm()),
            ("sound/new.wav".into(), pcm()),
        ],
    )
    .unwrap();
    let old = bank.find("sound/old.wav").unwrap();
    let new = bank.find("sound/new.wav").unwrap();
    host.runtime.sound_assets = Some(Box::new(qa_app::audio::SoundAssets::from_bank(bank)));
    host.bind_sounds().unwrap();
    let mut source = ServerInfoAudio {
        mixer: qa_audio::Mixer::load(rate, 8, 2),
        order: vec![],
        actions: vec![],
        reject_reset: false,
    };
    source.mixer.listen(
        &clients
            .iter()
            .copied()
            .map(|client| {
                Some(qa_audio::Listener {
                    client,
                    entity: None,
                    origin: Vec3::default(),
                    right: Vec3([1., 0., 0.]),
                    negate_dot: false,
                })
            })
            .collect::<Vec<_>>(),
        1.,
    );
    for &client in &clients {
        let mut bytes = nq_server_info(b"old", &[b"maps/ready.bsp"], &[b"old.wav"]);
        bytes.extend(nq_ambient());
        queue_netquake_control(&mut host, client, &bytes);
    }
    host.drain(&mut Default::default());
    let first = dispatch_server_info_audio(&mut host, &mut source);
    assert_eq!(
        (
            first.presentation_resets,
            first.sounds,
            first.unhandled_sounds
        ),
        (2, 2, 0)
    );
    assert_eq!(
        source.order,
        vec![
            (true, clients[0], None),
            (false, clients[0], Some(0)),
            (true, clients[1], None),
            (false, clients[1], Some(0))
        ]
    );
    assert_eq!(
        (source.mixer.counts.started, source.mixer.counts.invalid),
        (2, 0)
    );
    let owner = host.runtime.sound_assets.as_ref().unwrap();
    let retained = (owner.bank().len(), owner.reads, owner.decodes);
    let peer_sound = host.runtime.client_resources[clients[1].0 as usize]
        .as_ref()
        .unwrap()
        .state
        .sound(1);
    assert_eq!(peer_sound, Some(old));

    source.order.clear();
    let mut bytes = nq_server_info(b"new", &[b"maps/ready.bsp"], &[b"new.wav"]);
    bytes.extend(nq_ambient());
    queue_netquake_control(&mut host, clients[0], &bytes);
    host.drain(&mut Default::default());
    let output = dispatch_server_info_audio(&mut host, &mut source);
    assert_eq!(
        (
            output.presentation_resets,
            output.unhandled_presentation_resets,
            output.sounds,
            output.unhandled_sounds
        ),
        (1, 0, 1, 0)
    );
    assert_eq!(
        source.order,
        vec![(true, clients[0], None), (false, clients[0], Some(0))]
    );
    assert_eq!(source.mixer.counts.stopped, 1);
    assert_eq!(
        (source.mixer.counts.started, source.mixer.counts.invalid),
        (3, 0)
    );
    assert_eq!(
        host.runtime.client_resources[clients[0].0 as usize]
            .as_ref()
            .unwrap()
            .state
            .sound(1),
        Some(new)
    );
    assert_eq!(
        host.runtime.client_resources[clients[1].0 as usize]
            .as_ref()
            .unwrap()
            .state
            .sound(1),
        Some(old)
    );
    let owner = host.runtime.sound_assets.as_ref().unwrap();
    assert_eq!((owner.bank().len(), owner.reads, owner.decodes), retained);
    let mut pcm = [0; 8];
    assert!(source.mixer.paint(owner.bank(), &mut pcm));
    assert!(pcm.iter().any(|&sample| sample != 0));
    source.order.clear();
    let output = dispatch_server_info_audio(&mut host, &mut source);
    assert_eq!(output.presentation_resets, 0);
    assert!(source.order.is_empty());
    assert_eq!(source.mixer.counts.stopped, 1);

    // A failed sink reset drops its following cue rather than playing it into
    // the prior level. The same retained owner retries once on the next pass.
    source.reject_reset = true;
    queue_netquake_control(&mut host, clients[0], &bytes);
    host.drain(&mut Default::default());
    let output = dispatch_server_info_audio(&mut host, &mut source);
    assert_eq!(
        (
            output.unhandled_presentation_resets,
            output.unhandled_sounds
        ),
        (1, 1)
    );
    assert_eq!(source.order, vec![(true, clients[0], None)]);
    source.order.clear();
    let output = dispatch_server_info_audio(&mut host, &mut source);
    assert_eq!(
        (
            output.presentation_resets,
            output.unhandled_presentation_resets
        ),
        (1, 0)
    );
    assert_eq!(source.order, vec![(true, clients[0], None)]);
    queue_netquake_control(&mut host, clients[0], &nq_ambient());
    host.drain(&mut Default::default());
    let output = dispatch_server_info_audio(&mut host, &mut source);
    assert_eq!((output.sounds, output.unhandled_sounds), (1, 0));
    assert_eq!(source.mixer.counts.invalid, 0);

    // Native listener ownership must retire even when a preceding map reset
    // rejects. Otherwise a later successful map reset deliberately preserves
    // the old owner and blocks the same client slot's next connection.
    let first = clients[0];
    let peer = clients[1];
    let lifetime = host.runtime.server.clients[first.0 as usize].entity;
    let listener_event = |lifetime| SoundEvent {
        action: SoundAction::Respatialize,
        client: Some(first),
        client_lifetime: Some(lifetime),
        listener_owner: Some(RESOURCES),
        listener: Some(qa_audio::Listener {
            client: first,
            entity: None,
            origin: Vec3::default(),
            right: Vec3([1., 0., 0.]),
            negate_dot: false,
        }),
        volume: 1.,
        ..Default::default()
    };
    assert!(
        source
            .mixer
            .sound(host.runtime.sound_bank().unwrap(), listener_event(lifetime))
    );
    source.order.clear();
    source.actions.clear();
    source.reject_reset = true;
    let mut close = nq_server_info(b"closing level", &[b"maps/ready.bsp"], &[b"new.wav"]);
    close.push(2); // Native svc_disconnect follows the checked replacement.
    queue_netquake_control(&mut host, first, &close);
    queue_netquake_control(&mut host, peer, &nq_ambient());
    host.drain(&mut Default::default());
    assert!(
        host.runtime.snapshots[first.0 as usize]
            .as_ref()
            .unwrap()
            .presentation_closed()
    );
    assert!(
        host.runtime.server.clients[first.0 as usize]
            .audio_retirement
            .is_some()
    );
    let output = dispatch_server_info_audio(&mut host, &mut source);
    assert_eq!(
        (
            output.unhandled_presentation_resets,
            output.unhandled_sounds
        ),
        (1, 0)
    );
    assert_eq!(
        source.actions,
        vec![SoundAction::StartLoop, SoundAction::RetireClient]
    );
    assert_eq!(
        source.order,
        vec![
            (true, first, None),
            (false, peer, Some(1)),
            (false, first, None)
        ]
    );
    assert!(
        host.runtime.server.clients[first.0 as usize]
            .audio_retirement
            .is_none()
    );
    source.order.clear();
    source.actions.clear();
    let output = dispatch_server_info_audio(&mut host, &mut source);
    assert_eq!(
        (
            output.presentation_resets,
            output.unhandled_presentation_resets
        ),
        (1, 0)
    );
    assert_eq!(source.order, vec![(true, first, None)]);
    assert!(source.actions.is_empty());
    assert!(host.runtime.disconnect_client(first));
    let replacement = host
        .runtime
        .connect_local(
            SeatId::FIRST,
            SpawnAnchor {
                position: Vec3::default(),
                angles: Vec3::default(),
                entity: 0,
                fixture_fallback: false,
            },
            ClientPolicy {
                input: RuleSetId::Quake,
                client: RuleSetId::QuakeWorld,
                movement: RuleSetId::Quake3,
                trace: RuleSetId::Quake2,
            },
            Protocol::NetQuake15,
            5,
        )
        .unwrap();
    assert_eq!(replacement, first);
    let replacement_lifetime = host.runtime.server.clients[first.0 as usize].entity;
    assert_ne!(replacement_lifetime, lifetime);
    assert!(source.mixer.sound(
        host.runtime.sound_bank().unwrap(),
        listener_event(replacement_lifetime)
    ));
    host.bind_sounds().unwrap();
    source.order.clear();
    let mut replacement = nq_server_info(b"reconnected", &[b"maps/ready.bsp"], &[b"new.wav"]);
    replacement.extend(nq_ambient());
    queue_netquake_control(&mut host, first, &replacement);
    host.drain(&mut Default::default());
    let output = dispatch_server_info_audio(&mut host, &mut source);
    assert_eq!(
        (
            output.presentation_resets,
            output.sounds,
            output.unhandled_sounds
        ),
        (1, 1, 0)
    );
    assert_eq!(
        source.order,
        vec![(true, first, None), (false, first, Some(0))]
    );
    assert_eq!(source.mixer.counts.invalid, 0);
    assert_eq!(
        host.runtime.client_resources[peer.0 as usize]
            .as_ref()
            .unwrap()
            .state
            .sound(1),
        Some(old)
    );
    let owner = host.runtime.sound_assets.as_ref().unwrap();
    assert_eq!((owner.bank().len(), owner.reads, owner.decodes), retained);
    let mut pcm = [0; 8];
    assert!(source.mixer.paint(owner.bank(), &mut pcm));
    assert!(pcm.iter().any(|&sample| sample != 0));
}

#[test]
fn queued_netquake_reset_keeps_new_display_records_pending_without_stalling_a_healthy_peer() {
    let (mut host, clients) = netquake_camera_host(&[5, 9]);
    bind_nq_preloaded_world(&mut host, &clients);
    let first = clients[0];
    let second = clients[1];
    let print = |text: &[u8]| {
        let mut bytes = [0; 128];
        let length = qa_network::outputs::print(
            Protocol::NetQuake15,
            PrintKind::Center,
            0,
            text,
            None,
            &mut bytes,
        )
        .unwrap();
        bytes[..length].to_vec()
    };
    let mut bytes = nq_server_info(b"new level", &[b"maps/ready.bsp"], &[]);
    bytes.extend(print(b"new level message"));
    queue_netquake_control(&mut host, first, &bytes);
    queue_netquake_control(&mut host, second, &print(b"healthy peer"));
    host.drain(&mut Default::default());
    let first_owner = host.runtime.server.clients[first.0 as usize]
        .output
        .unwrap();
    let mut source = Source {
        presentation_paused: true,
        ..Default::default()
    };
    let output = qa_app::output::dispatch(
        &mut host.runtime,
        &mut source,
        &mut host.console.scrollback,
        &host.local_clients,
        host.time,
        3.,
        2.,
    );
    assert_eq!(output.presentation_resets, 0);
    assert!(
        host.runtime.server.clients[first.0 as usize]
            .hud
            .centerprint
            .is_none()
    );
    let peer = host.runtime.server.clients[second.0 as usize]
        .hud
        .centerprint
        .as_ref()
        .unwrap()
        .text
        .id();
    assert_eq!(
        host.runtime.server.events.texts.get(peer),
        Some(b"healthy peer".as_slice())
    );
    assert!(host.runtime.server.events.sequence_range().any(|sequence| {
        host.runtime
            .server
            .events
            .retained_for(first_owner, sequence)
    }));
    source.presentation_paused = false;
    let output = qa_app::output::dispatch(
        &mut host.runtime,
        &mut source,
        &mut host.console.scrollback,
        &host.local_clients,
        host.time,
        3.,
        2.,
    );
    assert_eq!(output.presentation_resets, 1);
    let first_text = host.runtime.server.clients[first.0 as usize]
        .hud
        .centerprint
        .as_ref()
        .unwrap()
        .text
        .id();
    assert_eq!(
        host.runtime.server.events.texts.get(first_text),
        Some(b"new level message".as_slice())
    );
    assert_eq!(
        host.runtime.server.events.texts.get(peer),
        Some(b"healthy peer".as_slice())
    );
}

fn dispatch_nq_startup_reset(host: &mut FrameHost) {
    qa_app::output::dispatch(
        &mut host.runtime,
        &mut Source::default(),
        &mut host.console.scrollback,
        &host.local_clients,
        host.time,
        3.,
        2.,
    );
}

// Observe the actual existing loopback/channel decoder at SERVER. This fixture
// consumes commands instead of dispatching gameplay, then returns native ACKs.
fn flush_nq_client_commands(host: &mut FrameHost, rounds: usize) -> (Vec<Vec<u8>>, usize) {
    let mut commands = Vec::new();
    let mut moves = 0;
    let mut queue = SysEventQueue::load(256, 256 * 1024).unwrap();
    for _ in 0..rounds {
        for client in host.local_clients.into_iter().flatten() {
            host.runtime.send_local_command(
                client,
                &UserCmd {
                    duration_ms: 16,
                    movement: [100., 0., 0.],
                    ..Default::default()
                },
                host.time,
            );
        }
        host.runtime.loopback.enqueue(&mut queue, host.time);
        while let Some(event) = queue.pop() {
            let EventKind::Packet {
                socket,
                from,
                bytes,
            } = event.kind
            else {
                continue;
            };
            assert_eq!(socket, Endpoint::Server.socket());
            host.runtime
                .network
                .receive(socket, from, bytes, event.time, |_, _, incoming| {
                    match incoming {
                        qa_network::ingress::Incoming::CommandString { text, .. } => {
                            commands.push(text.to_vec())
                        }
                        qa_network::ingress::Incoming::Command { .. } => moves += 1,
                        _ => {}
                    }
                    ControlFlow::Continue(())
                });
        }
        for client in host.local_clients.into_iter().flatten() {
            let connection = host
                .runtime
                .network
                .get_mut(client, Endpoint::Server)
                .unwrap();
            while connection.channel.pending_controls() != 0 {
                connection
                    .channel
                    .prepare(None, host.time)
                    .unwrap()
                    .unwrap();
                connection
                    .channel
                    .submit_with(host.time, |packet| {
                        host.runtime
                            .loopback
                            .send(Endpoint::Server, client, packet)
                            .is_ok()
                    })
                    .unwrap();
            }
        }
        host.drain(&mut Default::default());
    }
    assert_eq!(
        (
            host.runtime.network.malformed,
            host.runtime.network.command_errors
        ),
        (0, 0)
    );
    (commands, moves)
}

#[test]
fn queued_netquake_signon_replies_preserve_native_text_retry_progress_and_stage_order() {
    let (mut host, clients) = netquake_camera_host(&[5]);
    let client = clients[0];
    bind_nq_preloaded_world(&mut host, &clients);
    let context = Context {
        source: RuleSetId::Quake,
        seat: SeatId::FIRST,
        ..Default::default()
    };
    host.console
        .cvars
        .set_user("_cl_name", "Pilot", context, None, 0)
        .unwrap();
    host.console
        .cvars
        .set_user("_cl_color", "0x34", context, None, 0)
        .unwrap();
    let resources = host.runtime.client_resources[client.0 as usize]
        .as_mut()
        .unwrap();
    resources.set_nq_spawn_parameters(b"1 2 3 ").unwrap();
    for rejected in [
        b"hidden\0suffix".as_slice(),
        b"hidden\xffsuffix",
        &vec![b'x'; 2048],
    ] {
        assert!(resources.set_nq_spawn_parameters(rejected).is_err());
    }
    let mut header = nq_server_info(b"signon", &[b"maps/ready.bsp", b"*1"], &[]);
    header.extend([25, 1]);
    queue_netquake_control(&mut host, client, &header);
    host.drain(&mut Default::default());
    assert_eq!(
        host.runtime.client_resources[client.0 as usize]
            .as_ref()
            .unwrap()
            .nq_signon_stage(),
        Some(1)
    );
    assert_eq!(flush_nq_client_commands(&mut host, 1), (vec![], 0));
    dispatch_nq_startup_reset(&mut host);
    // An ordinary admitted row while stage1 keeps it at1 and retries readiness.
    queue_nq_view_snapshot(&mut host, client, 1., None, (42, Vec3::default()), None);
    host.drain(&mut Default::default());
    assert_eq!(
        flush_nq_client_commands(&mut host, 2),
        (vec![b"prespawn".to_vec()], 0)
    );
    assert_eq!(
        host.runtime
            .network
            .get(client, Endpoint::Client)
            .unwrap()
            .commands
            .as_ref()
            .unwrap()
            .history()
            .unwrap()
            .number(),
        0
    );

    // Discover the bounded record capacity through real queue admission and
    // drain its native coalesced flight. Then refill all but one record slot:
    // stage2 can admit name, while color/spawn must wait without duplicating it.
    let mut filled = 0;
    loop {
        let result = host
            .runtime
            .network
            .get_mut(client, Endpoint::Client)
            .unwrap()
            .channel
            .queue_client_command(Protocol::NetQuake15, b"busy");
        match result {
            Ok(_) => filled += 1,
            Err(qa_network::channel::TransmitError::Full) => break,
            Err(error) => panic!("unexpected native queue error: {error}"),
        }
    }
    assert!(filled > 1);
    assert_eq!(
        flush_nq_client_commands(&mut host, filled + 2),
        (vec![b"busy".to_vec(); filled], 0)
    );
    for _ in 0..filled - 1 {
        host.runtime
            .network
            .get_mut(client, Endpoint::Client)
            .unwrap()
            .channel
            .queue_client_command(Protocol::NetQuake15, b"busy")
            .unwrap();
    }
    queue_netquake_control(&mut host, client, &[25, 2]);
    host.drain(&mut Default::default());
    let (queued, moves) = flush_nq_client_commands(&mut host, filled + 2);
    assert_eq!(moves, 0);
    assert_eq!(
        queued
            .iter()
            .filter(|text| text.as_slice() == b"busy")
            .count(),
        filled - 1
    );
    assert_eq!(
        queued
            .iter()
            .filter(|text| text.as_slice() != b"busy")
            .cloned()
            .collect::<Vec<_>>(),
        vec![b"name \"Pilot\"\n".to_vec()]
    );
    queue_nq_view_snapshot(&mut host, client, 2., None, (42, Vec3::default()), None);
    host.drain(&mut Default::default());
    assert_eq!(
        flush_nq_client_commands(&mut host, 3),
        (vec![b"color 3 4\n".to_vec(), b"spawn 1 2 3 ".to_vec()], 0)
    );
    assert_eq!(
        host.runtime
            .network
            .get(client, Endpoint::Client)
            .unwrap()
            .commands
            .as_ref()
            .unwrap()
            .history()
            .unwrap()
            .number(),
        0
    );

    queue_netquake_control(&mut host, client, &[25, 3]);
    host.drain(&mut Default::default());
    assert_eq!(
        flush_nq_client_commands(&mut host, 2),
        (vec![b"begin".to_vec()], 0)
    );
    assert_eq!(
        host.runtime.client_resources[client.0 as usize]
            .as_ref()
            .unwrap()
            .nq_signon_stage(),
        Some(3)
    );
    // A truncated fast-update body cannot admit the native transition.
    queue_netquake_control(&mut host, client, &[0x80]);
    host.drain(&mut Default::default());
    assert_eq!(
        host.runtime.client_resources[client.0 as usize]
            .as_ref()
            .unwrap()
            .nq_signon_stage(),
        Some(3)
    );
    assert_eq!(host.runtime.network.command_errors, 1);
    queue_nq_view_snapshot(&mut host, client, 3., None, (42, Vec3::default()), None);
    host.drain(&mut Default::default());
    assert_eq!(
        host.runtime.client_resources[client.0 as usize]
            .as_ref()
            .unwrap()
            .nq_signon_stage(),
        Some(4)
    );
    assert!(host.runtime.send_local_command(
        client,
        &UserCmd {
            duration_ms: 16,
            ..Default::default()
        },
        host.time
    ));
    assert_eq!(
        host.runtime
            .network
            .get(client, Endpoint::Client)
            .unwrap()
            .commands
            .as_ref()
            .unwrap()
            .history()
            .unwrap()
            .number(),
        1
    );
}

#[test]
fn queued_netquake_stage_jumps_require_resources_and_reset_before_movement_and_reject_only_the_bad_client()
 {
    let (mut host, clients) = netquake_camera_host(&[5, 9]);
    bind_nq_preloaded_world(&mut host, &clients);
    let first = clients[0];
    let peer = clients[1];
    let mut bytes = nq_server_info(b"unseen", &[b"maps/unseen.bsp"], &[]);
    bytes.extend([25, 2, 25, 3]);
    queue_nq_view_snapshot_with_prefix(
        &mut host,
        first,
        1.,
        None,
        (42, Vec3::default()),
        None,
        &bytes,
    );
    host.drain(&mut Default::default());
    assert_eq!(
        host.runtime.client_resources[first.0 as usize]
            .as_ref()
            .unwrap()
            .nq_signon_stage(),
        Some(4)
    );
    let number = host
        .runtime
        .network
        .get(first, Endpoint::Client)
        .unwrap()
        .commands
        .as_ref()
        .unwrap()
        .history()
        .unwrap()
        .number();
    assert!(!host.runtime.send_local_command(
        first,
        &UserCmd {
            duration_ms: 100,
            movement: [200., 0., 0.],
            ..Default::default()
        },
        host.time
    ));
    assert_eq!(
        host.runtime
            .network
            .get(first, Endpoint::Client)
            .unwrap()
            .commands
            .as_ref()
            .unwrap()
            .history()
            .unwrap()
            .number(),
        number
    );
    assert_eq!(
        host.runtime
            .network
            .get(first, Endpoint::Client)
            .unwrap()
            .channel
            .send_state()
            .reliable_bytes,
        0
    );
    // Explicit wire stage4 has the same resource gate as an entity-completed4.
    let mut bytes = nq_server_info(b"unseen again", &[b"maps/unseen.bsp"], &[]);
    bytes.extend([25, 4]);
    queue_netquake_control(&mut host, first, &bytes);
    host.drain(&mut Default::default());
    assert!(!host.runtime.send_local_command(
        first,
        &UserCmd {
            duration_ms: 100,
            ..Default::default()
        },
        host.time
    ));
    assert_eq!(
        host.runtime
            .network
            .get(first, Endpoint::Client)
            .unwrap()
            .commands
            .as_ref()
            .unwrap()
            .history()
            .unwrap()
            .number(),
        number
    );

    // Another connection admits a jump directly to3 and sends only begin.
    let mut bytes = nq_server_info(b"healthy", &[b"maps/ready.bsp"], &[]);
    bytes.extend([25, 3]);
    queue_netquake_control(&mut host, peer, &bytes);
    host.drain(&mut Default::default());
    dispatch_nq_startup_reset(&mut host);
    queue_netquake_control(&mut host, peer, &[1]);
    host.drain(&mut Default::default());
    // A row completes3→4, but resources must first be admitted at the same
    // restored-network boundary; no name/color/spawn/prespawn is fabricated.
    queue_nq_view_snapshot(&mut host, peer, 2., None, (42, Vec3::default()), None);
    host.drain(&mut Default::default());
    assert_eq!(
        host.runtime.client_resources[peer.0 as usize]
            .as_ref()
            .unwrap()
            .nq_signon_stage(),
        Some(4)
    );
    assert!(
        !host.runtime.snapshots[peer.0 as usize]
            .as_ref()
            .unwrap()
            .presentation_closed()
    );

    let (replies, _) = flush_nq_client_commands(&mut host, 2);
    assert_eq!(replies, vec![b"begin".to_vec()]);

    queue_netquake_control(&mut host, first, &[25, 4]); // duplicate after4
    host.drain(&mut Default::default());
    assert!(
        host.runtime.snapshots[first.0 as usize]
            .as_ref()
            .unwrap()
            .presentation_closed()
    );
    assert!(
        !host.runtime.snapshots[peer.0 as usize]
            .as_ref()
            .unwrap()
            .presentation_closed()
    );
    assert_eq!(
        host.runtime.snapshots[peer.0 as usize]
            .as_ref()
            .unwrap()
            .native_client(),
        9
    );
}

#[test]
fn queued_netquake_stage4_waits_for_rejected_reset_without_predicting_or_advancing_command_history()
{
    let (mut host, clients) = netquake_camera_host(&[5, 9]);
    bind_nq_preloaded_world(&mut host, &clients);
    let first = clients[0];
    let peer = clients[1];
    let old_angles = host.runtime.prediction[0].player.view_angles;
    let mut bytes = nq_server_info(b"pending reset", &[b"maps/ready.bsp"], &[]);
    bytes.extend([25, 4]);
    queue_netquake_control(&mut host, first, &bytes);
    host.drain(&mut Default::default());
    host.runtime
        .input
        .set_view_angles(SeatId::FIRST, Vec3([70., 80., 90.]));
    let mut source = Source {
        time: EventTime::from_milliseconds(10),
        reject_reset: true,
        ..Default::default()
    };
    let first_frame = host.frame(&mut source, true);
    assert_eq!(first_frame.output.unhandled_presentation_resets, 1);
    assert_eq!(
        (
            first_frame.commands[0].duration_ms,
            first_frame.commands[0].movement[0]
        ),
        (0, 0.)
    );
    assert_eq!(host.runtime.prediction[0].player.view_angles, old_angles);
    assert_eq!(
        host.runtime
            .network
            .get(first, Endpoint::Client)
            .unwrap()
            .commands
            .as_ref()
            .unwrap()
            .history()
            .unwrap()
            .number(),
        0
    );
    assert!(
        host.runtime
            .network
            .get(peer, Endpoint::Client)
            .unwrap()
            .commands
            .as_ref()
            .unwrap()
            .history()
            .unwrap()
            .number()
            > 0
    );
    source.time = EventTime::from_milliseconds(20);
    let retry = host.frame(&mut source, true);
    assert_eq!(retry.output.presentation_resets, 1);
    assert_eq!(
        (retry.commands[0].duration_ms, retry.commands[0].movement[0]),
        (0, 0.)
    );
    assert_eq!(
        host.runtime
            .network
            .get(first, Endpoint::Client)
            .unwrap()
            .commands
            .as_ref()
            .unwrap()
            .history()
            .unwrap()
            .number(),
        0
    );
    source.time = EventTime::from_milliseconds(30);
    host.frame(&mut source, true);
    assert!(
        host.runtime
            .network
            .get(first, Endpoint::Client)
            .unwrap()
            .commands
            .as_ref()
            .unwrap()
            .history()
            .unwrap()
            .number()
            > 0
    );
    assert_eq!(
        (
            host.runtime.prediction[0].player.movement_rules,
            host.runtime.prediction[0].player.trace_rules
        ),
        (RuleSetId::Quake3, RuleSetId::Quake2)
    );
}

#[test]
fn queued_netquake_required_model_waits_without_backend_then_closes_on_real_cold_failure() {
    let (mut host, clients) = netquake_camera_host(&[5, 9]);
    bind_nq_preloaded_world(&mut host, &clients);
    let first = clients[0];
    let peer = clients[1];
    let mut backend =
        asset_frames::Source::load(host.runtime.model_assets.as_ref().unwrap().assets());
    host.publish_model_admission(backend.cpu.admitted_assets().unwrap())
        .unwrap();
    let mut bytes = nq_server_info(
        b"missing model",
        &[b"maps/ready.bsp", b"progs/not-installed.mdl"],
        &[],
    );
    bytes.extend([25, 1]);
    queue_netquake_control(&mut host, first, &bytes);
    host.drain(&mut Default::default());
    dispatch_nq_startup_reset(&mut host);
    host.runtime.vfs.take_lookup_count();
    host.register_client_model_assets(None);
    assert_eq!(host.runtime.vfs.take_lookup_count(), 0);
    queue_nq_view_snapshot(&mut host, first, 1., None, (42, Vec3::default()), None);
    host.drain(&mut Default::default());
    assert!(
        !host.runtime.snapshots[first.0 as usize]
            .as_ref()
            .unwrap()
            .presentation_closed()
    );
    assert_eq!(
        host.runtime
            .network
            .get(first, Endpoint::Client)
            .unwrap()
            .channel
            .send_state()
            .reliable_bytes,
        0
    );
    host.register_client_model_assets(Some(&mut backend));
    assert!(host.runtime.vfs.take_lookup_count() > 0);
    host.register_client_model_assets(Some(&mut backend));
    assert_eq!(host.runtime.vfs.take_lookup_count(), 0);
    queue_nq_view_snapshot(&mut host, first, 2., None, (42, Vec3::default()), None);
    host.drain(&mut Default::default());
    assert!(
        host.runtime.snapshots[first.0 as usize]
            .as_ref()
            .unwrap()
            .presentation_closed()
    );
    assert!(
        !host.runtime.snapshots[peer.0 as usize]
            .as_ref()
            .unwrap()
            .presentation_closed()
    );
    assert_eq!(
        host.runtime
            .network
            .get(first, Endpoint::Client)
            .unwrap()
            .commands
            .as_ref()
            .unwrap()
            .history()
            .unwrap()
            .number(),
        0
    );
}
