use qa_app::{
    Runtime,
    client_policy::ClientPolicy,
    host::{FrameHost, FrameSource},
    map::SpawnAnchor,
};
use qa_console::{commands::Console, views::Context};
use qa_core::{
    loopback::{Endpoint, Loopback, LoopbackLimits},
    primitives::*,
    sys_events::*,
};
use qa_network::{
    channel::{Channel, Delivery},
    commands::{
        connection::Commands,
        packet::{self, Protocol},
    },
    ingress::{Connection as NativeConnection, Incoming, Route},
};
use qa_session::{
    clients::{ClientPhase, Connection},
    timing::TickRate,
};
use std::time::Duration;

#[derive(Default)]
struct Source {
    now: EventTime,
    reject: Option<u16>,
    packets: Vec<(u16, Vec<u8>)>,
}
impl FrameSource for Source {
    fn begin_frame(&mut self) -> Duration {
        Duration::from_nanos(self.now.0)
    }
    fn wait_time(&mut self, _: Duration) -> Duration {
        Duration::from_nanos(self.now.0)
    }
    fn poll_events(&mut self, queue: &mut SysEventQueue) {
        queue
            .push(SysEvent {
                time: self.now,
                kind: EventKind::Time,
            })
            .unwrap();
    }
    fn elapsed(&self) -> Duration {
        Duration::ZERO
    }
    fn present(&mut self) {}
    fn send_packet(&mut self, _: u16, to: std::net::SocketAddr, bytes: &[u8]) -> bool {
        if self.reject == Some(to.port()) {
            return false;
        }
        self.packets.push((to.port(), bytes.to_vec()));
        true
    }
}

fn remote(runtime: &mut Runtime, port: u16) -> ClientId {
    remote_protocol(runtime, Protocol::Quake3_68, port)
}

fn remote_protocol(runtime: &mut Runtime, protocol: Protocol, port: u16) -> ClientId {
    let id = runtime
        .server
        .connect(Connection::Remote, ModuleId(7), None)
        .unwrap();
    let mut channel = Channel::load(protocol.channel(), Endpoint::Server, 8192, 16).unwrap();
    channel.set_qport(port);
    runtime
        .network
        .bind(
            id,
            Endpoint::Server,
            NativeConnection {
                route: Route {
                    socket: 0,
                    peer: Peer::Socket(([127, 0, 0, 1], port).into()),
                },
                channel,
                output: runtime.server.clients[id.0 as usize].output,
                pending_output: None,
                commands: Some(Commands::load(protocol, 8192)),
            },
        )
        .unwrap();
    id
}

fn host(runtime: Runtime) -> FrameHost {
    let mut host = FrameHost::load(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::fixed(20).unwrap(),
        vec![],
    )
    .unwrap();
    // The first frame seeds the timeline instead of accruing startup debt.
    host.frame(&mut Source::default(), true);
    host
}

fn begin_close(
    host: &mut FrameHost,
    id: ClientId,
) -> (EntityId, qa_core::events::OutputConsumerId, u64) {
    let client = &host.runtime.server.clients[id.0 as usize];
    let lifetime = client.entity;
    let output = client.output.unwrap();
    assert!(
        host.runtime
            .server
            .begin_close(id, lifetime, EventTime::default())
    );
    let sequence = host
        .runtime
        .server
        .events
        .server_command(
            Some(id),
            ModuleId(7),
            RuleSetId::Quake3,
            b"disconnect \"closed\"",
        )
        .unwrap();
    assert!(host.runtime.server.finish_close(id, lifetime, sequence));
    (lifetime, output, sequence)
}

fn queue(host: &mut FrameHost, port: u16, bytes: &[u8], time: EventTime) {
    host.queue
        .push(SysEvent {
            time,
            kind: EventKind::Packet {
                socket: 0,
                from: Peer::Socket(([127, 0, 0, 1], port).into()),
                bytes,
            },
        })
        .unwrap();
}

fn peer(port: u16) -> (Channel, Commands) {
    let mut channel =
        Channel::load(Protocol::Quake3_68.channel(), Endpoint::Client, 8192, 16).unwrap();
    channel.set_qport(port);
    (channel, Commands::load(Protocol::Quake3_68, 8192))
}

fn receive(channel: &mut Channel, commands: &mut Commands, bytes: &[u8]) -> Vec<Vec<u8>> {
    let received = channel
        .receive(bytes, EventTime::from_milliseconds(100))
        .unwrap();
    let Delivery::Payload(payload) = received.delivery else {
        return Vec::new();
    };
    let sequence = received.header.sequence;
    let length = commands.stage(payload).unwrap();
    let mut records = Vec::new();
    commands
        .decode_output(
            length,
            sequence,
            channel,
            ResourceNamespace::Client(ClientId(0)),
            |incoming| {
                if let Incoming::ReliableCommand { text, .. } = incoming {
                    records.push(text.to_vec());
                }
            },
        )
        .unwrap();
    records
}

fn control(channel: &mut Channel, commands: &Commands, future_ack: bool) -> Vec<u8> {
    let mut bytes = [0; 8192];
    let length = if future_ack {
        let mut key = channel.command_key(None).unwrap();
        key.acknowledgements.reliable += 100;
        packet::write_control(&mut bytes, Protocol::Quake3_68, key, |_| Ok(())).unwrap()
    } else {
        commands.encode_control(channel, &mut bytes).unwrap()
    };
    channel
        .prepare_move(
            &bytes[..length],
            EventTime::from_milliseconds(100),
            None,
            None,
        )
        .unwrap()
        .unwrap();
    let mut packet = Vec::new();
    channel
        .submit_with(EventTime::from_milliseconds(100), |bytes| {
            packet.extend_from_slice(bytes);
            true
        })
        .unwrap();
    packet
}

#[test]
fn submitted_disconnect_waits_for_exact_native_ack_while_a_healthy_peer_and_ticks_continue() {
    let mut runtime = Runtime::load(2, []).unwrap();
    let closing = remote(&mut runtime, 2000);
    let healthy = remote(&mut runtime, 2001);
    let mut host = host(runtime);
    let (lifetime, output, sequence) = begin_close(&mut host, closing);
    host.runtime
        .server
        .events
        .server_command(
            Some(healthy),
            ModuleId(7),
            RuleSetId::Quake3,
            b"print \"healthy\"",
        )
        .unwrap();
    let mut source = Source {
        now: EventTime::from_milliseconds(100),
        reject: Some(2000),
        ..Default::default()
    };
    assert!(host.frame(&mut source, true).server_ticks > 0);
    assert!(source.packets.iter().any(|(port, _)| *port == 2001));
    assert!(host.runtime.server.events.retained_for(output, sequence));
    assert!(matches!(
        host.runtime.server.clients[closing.0 as usize].phase,
        ClientPhase::Closing(_)
    ));
    source.reject = None;
    source.now = EventTime::from_milliseconds(120);
    assert!(host.frame(&mut source, true).server_ticks > 0);
    let (mut client, mut commands) = peer(2000);
    let records: Vec<_> = source
        .packets
        .iter()
        .filter(|(port, _)| *port == 2000)
        .flat_map(|(_, packet)| receive(&mut client, &mut commands, packet))
        .collect();
    assert_eq!(records, [b"disconnect \"closed\"".to_vec()]);
    assert!(host.runtime.server.events.retained_for(output, sequence));
    assert!(host.runtime.server.entities.resolve(lifetime).is_some());
    assert_eq!(host.close_acknowledged, 0);

    let forged = control(&mut client, &commands, true);
    queue(&mut host, 2000, &forged, EventTime::from_milliseconds(140));
    source.now = EventTime::from_milliseconds(140);
    host.frame(&mut source, true);
    assert!(host.runtime.server.events.retained_for(output, sequence));
    assert!(
        host.runtime.server.clients[closing.0 as usize]
            .connection
            .is_some()
    );
    assert_eq!(host.close_acknowledged, 0);

    let ack = control(&mut client, &commands, false);
    queue(&mut host, 2000, &ack, EventTime::from_milliseconds(160));
    source.now = EventTime::from_milliseconds(160);
    assert!(host.frame(&mut source, true).server_ticks > 0);
    assert!(
        host.runtime.server.clients[closing.0 as usize]
            .connection
            .is_none()
    );
    assert!(host.runtime.server.entities.resolve(lifetime).is_none());
    assert!(
        host.runtime
            .network
            .get(closing, Endpoint::Server)
            .is_none()
    );
    assert_eq!(host.close_acknowledged, 1);
    assert_eq!((host.close_timed_out, host.close_aborted), (0, 0));
    assert!(host.runtime.server.clients[healthy.0 as usize].is_active());
    queue(&mut host, 2000, &ack, EventTime::from_milliseconds(180));
    source.now = EventTime::from_milliseconds(180);
    host.frame(&mut source, true);
    assert_eq!(host.close_acknowledged, 1);
}

#[test]
fn local_close_retries_refused_control_without_creating_usercmd_and_retires_after_queued_ack() {
    let mut runtime = Runtime::load(2, []).unwrap();
    let local = runtime
        .connect_local(
            SeatId::new(0).unwrap(),
            SpawnAnchor {
                position: Vec3::default(),
                angles: Vec3::default(),
                entity: 0,
                fixture_fallback: false,
            },
            ClientPolicy {
                input: RuleSetId::Quake3,
                client: RuleSetId::Quake3,
                movement: RuleSetId::Quake,
                trace: RuleSetId::Quake2,
            },
            Protocol::Quake3_68,
            0,
        )
        .unwrap();
    let healthy = remote(&mut runtime, 2001);
    let mut host = host(runtime);
    host.local_clients[0] = Some(local);
    host.runtime
        .network
        .get_mut(local, Endpoint::Client)
        .unwrap()
        .commands
        .as_mut()
        .unwrap()
        .record(UserCmd::default())
        .unwrap();
    let (_, output, sequence) = begin_close(&mut host, local);
    let mut source = Source {
        now: EventTime::from_milliseconds(100),
        ..Default::default()
    };
    host.frame(&mut source, true);
    // Transfer the real SERVER packet to the existing system-event queue, then
    // refuse CLIENT control admission while retaining the channel's packet.
    assert!(host.runtime.loopback.enqueue(&mut host.queue, source.now) > 0);
    host.runtime.loopback = Loopback::load([
        LoopbackLimits {
            maximum_message: 1,
            payload_bytes: 1,
            messages: 1,
        },
        LoopbackLimits {
            maximum_message: 1400,
            payload_bytes: 2800,
            messages: 2,
        },
    ])
    .unwrap();
    let number = host
        .runtime
        .network
        .get(local, Endpoint::Client)
        .unwrap()
        .commands
        .as_ref()
        .unwrap()
        .history()
        .unwrap()
        .number();
    source.now = EventTime::from_milliseconds(120);
    assert!(host.frame(&mut source, true).server_ticks > 0);
    let connection = host.runtime.network.get(local, Endpoint::Client).unwrap();
    assert!(connection.channel.pending_packet().is_some());
    assert_eq!(
        connection
            .commands
            .as_ref()
            .unwrap()
            .history()
            .unwrap()
            .number(),
        number
    );
    assert!(
        host.runtime.snapshots[local.0 as usize]
            .as_ref()
            .unwrap()
            .presentation_closed()
    );
    assert!(host.runtime.server.events.retained_for(output, sequence));
    assert_eq!(host.close_acknowledged, 0);

    host.runtime.loopback = Loopback::load(std::iter::repeat_n(
        LoopbackLimits {
            maximum_message: 1400,
            payload_bytes: 5600,
            messages: 4,
        },
        2,
    ))
    .unwrap();
    source.now = EventTime::from_milliseconds(140);
    host.frame(&mut source, true);
    assert!(
        host.runtime
            .network
            .get(local, Endpoint::Client)
            .unwrap()
            .channel
            .pending_packet()
            .is_none()
    );
    assert_eq!(
        host.runtime
            .network
            .get(local, Endpoint::Client)
            .unwrap()
            .commands
            .as_ref()
            .unwrap()
            .history()
            .unwrap()
            .number(),
        number
    );
    assert!(
        host.runtime
            .loopback
            .pending_client(Endpoint::Server, local)
            .unwrap()
            > 0
    );
    assert!(host.runtime.server.events.retained_for(output, sequence));
    assert!(
        host.runtime.server.clients[local.0 as usize]
            .connection
            .is_some()
    );
    source.now = EventTime::from_milliseconds(160);
    assert!(host.frame(&mut source, true).server_ticks > 0);
    assert!(
        host.runtime.server.clients[local.0 as usize]
            .connection
            .is_none()
    );
    assert!(host.runtime.network.get(local, Endpoint::Client).is_none());
    assert_eq!(host.close_acknowledged, 1);
    assert_eq!((host.close_timed_out, host.close_aborted), (0, 0));
    assert!(host.runtime.server.clients[healthy.0 as usize].is_active());
}

#[test]
fn unacknowledged_close_times_out_as_abort_not_ack_and_preserves_replacement_lifetime() {
    let mut runtime = Runtime::load(2, []).unwrap();
    let closing = remote(&mut runtime, 2000);
    let healthy = remote(&mut runtime, 2001);
    let mut host = host(runtime);
    let (old_lifetime, old_output, old_sequence) = begin_close(&mut host, closing);
    let mut source = Source {
        now: EventTime::from_milliseconds(100),
        ..Default::default()
    };
    host.frame(&mut source, true);
    let (mut client, mut commands) = peer(2000);
    for (_, packet) in source.packets.iter().filter(|(port, _)| *port == 2000) {
        receive(&mut client, &mut commands, packet);
    }
    let old_ack = control(&mut client, &commands, false);
    source.now = EventTime::from_milliseconds(2100);
    host.frame(&mut source, true);
    assert_eq!((host.close_acknowledged, host.close_timed_out), (0, 1));
    assert!(
        host.runtime.server.clients[closing.0 as usize]
            .connection
            .is_none()
    );
    let replacement = remote(&mut host.runtime, 2002);
    assert_eq!(replacement, closing);
    let new_lifetime = host.runtime.server.clients[replacement.0 as usize].entity;
    assert_ne!(new_lifetime, old_lifetime);
    assert!(
        !host
            .runtime
            .server
            .begin_close(replacement, old_lifetime, source.now)
    );
    assert!(
        !host
            .runtime
            .server
            .acknowledge_close(replacement, old_output, old_sequence)
    );
    queue(
        &mut host,
        2000,
        &old_ack,
        EventTime::from_milliseconds(2120),
    );
    source.now = EventTime::from_milliseconds(2120);
    host.frame(&mut source, true);
    assert_eq!(
        host.runtime.server.clients[replacement.0 as usize].entity,
        new_lifetime
    );
    assert!(host.runtime.server.clients[replacement.0 as usize].is_active());
    assert!(host.runtime.server.clients[healthy.0 as usize].is_active());
    assert_eq!(host.close_acknowledged, 0);
}

#[test]
fn q3_owned_closing_record_uses_each_connections_native_disconnect_and_real_ack() {
    for protocol in [
        Protocol::NetQuake15,
        Protocol::QuakeWorld28,
        Protocol::Quake2_34,
        Protocol::Quake2Repro1038,
        Protocol::Quake3_68,
    ] {
        let mut runtime = Runtime::load(2, []).unwrap();
        let closing = remote_protocol(&mut runtime, protocol, 2000);
        let healthy = remote(&mut runtime, 2001);
        let mut host = host(runtime);
        let (_, output, sequence) = begin_close(&mut host, closing);
        let mut peer = Channel::load(protocol.channel(), Endpoint::Client, 8192, 16).unwrap();
        peer.configure_snapshots(protocol).unwrap();
        peer.set_qport(2000);
        let mut commands = Commands::load(protocol, 8192);
        if protocol == Protocol::QuakeWorld28 {
            // QW only replies after an admitted native sequenced request.
            peer.prepare(None, EventTime::default()).unwrap().unwrap();
            let mut request = Vec::new();
            peer.submit_with(EventTime::default(), |bytes| {
                request.extend_from_slice(bytes);
                true
            })
            .unwrap();
            queue(&mut host, 2000, &request, EventTime::from_milliseconds(100));
        }
        let mut source = Source {
            now: EventTime::from_milliseconds(100),
            ..Default::default()
        };
        assert!(host.frame(&mut source, true).server_ticks > 0);
        let mut disconnects = 0;
        for (_, bytes) in source.packets.iter().filter(|(port, _)| *port == 2000) {
            let received = peer.receive(bytes, source.now).unwrap();
            let sequence = received.header.sequence;
            let Delivery::Payload(payload) = received.delivery else {
                continue;
            };
            if protocol != Protocol::Quake3_68 {
                let opcode = if matches!(protocol, Protocol::NetQuake15 | Protocol::QuakeWorld28) {
                    2
                } else {
                    7
                };
                assert_eq!(payload, [opcode], "{protocol:?}");
            }
            let length = commands.stage(payload).unwrap();
            commands
                .decode_output(
                    length,
                    sequence,
                    &mut peer,
                    ResourceNamespace::Client(closing),
                    |incoming| match incoming {
                        Incoming::Disconnect { .. } => disconnects += 1,
                        Incoming::ReliableCommand { text, .. }
                            if text == b"disconnect \"closed\"" =>
                        {
                            disconnects += 1
                        }
                        _ => {}
                    },
                )
                .unwrap_or_else(|error| panic!("{protocol:?} disconnect decode: {error:?}"));
        }
        assert_eq!(disconnects, 1, "{protocol:?}");
        assert!(
            host.runtime.server.events.retained_for(output, sequence),
            "{protocol:?}"
        );
        assert_eq!(host.close_acknowledged, 0, "{protocol:?}");

        let mut control_bytes = [0; 8192];
        let length = commands.encode_control(&peer, &mut control_bytes).unwrap();
        peer.prepare_move(
            &control_bytes[..length],
            EventTime::from_milliseconds(120),
            None,
            None,
        )
        .unwrap()
        .unwrap();
        let mut ack = Vec::new();
        peer.submit_with(EventTime::from_milliseconds(120), |bytes| {
            ack.extend_from_slice(bytes);
            true
        })
        .unwrap();
        // Native transport submission alone does not complete SERVER retention.
        assert!(host.runtime.server.events.retained_for(output, sequence));
        queue(&mut host, 2000, &ack, EventTime::from_milliseconds(120));
        source.now = EventTime::from_milliseconds(120);
        assert!(host.frame(&mut source, true).server_ticks > 0);
        assert!(
            host.runtime.server.clients[closing.0 as usize]
                .connection
                .is_none(),
            "{protocol:?}"
        );
        assert_eq!(host.close_acknowledged, 1, "{protocol:?}");
        assert_eq!(
            (host.close_aborted, host.close_timed_out),
            (0, 0),
            "{protocol:?}"
        );
        assert!(host.runtime.server.clients[healthy.0 as usize].is_active());
    }
}
