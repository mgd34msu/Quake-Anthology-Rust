#[path = "support/frame_source.rs"]
mod frame_source;

use qa_app::{Runtime, client_policy::ClientPolicy, map::SpawnAnchor};
use qa_core::{
    events::NativeReceipt,
    loopback::{Endpoint, Loopback, LoopbackLimits},
    primitives::{ClientId, RuleSetId, UserCmd},
    sys_events::{EventKind, EventTime, SeatId, SysEventQueue},
};
use qa_network::{
    channel::TransmitError,
    commands::packet::{self, Protocol},
    headers::{self, Direction},
    ingress::Incoming,
};
use std::ops::ControlFlow;

fn socket_routes(runtime: &mut Runtime, clients: [ClientId; 2]) -> [std::net::SocketAddr; 2] {
    std::array::from_fn(|index| {
        let server = ([127, 0, 0, 1], 31000 + index as u16).into();
        let receiver = ([127, 0, 0, 1], 32000 + index as u16).into();
        runtime
            .network
            .get_mut(clients[index], Endpoint::Client)
            .unwrap()
            .route = qa_network::ingress::Route {
            socket: 70 + index as u16,
            peer: qa_core::sys_events::Peer::Socket(server),
        };
        runtime
            .network
            .get_mut(clients[index], Endpoint::Server)
            .unwrap()
            .route = qa_network::ingress::Route {
            socket: Endpoint::Server.socket(),
            peer: qa_core::sys_events::Peer::Socket(receiver),
        };
        server
    })
}

#[derive(Debug, PartialEq)]
enum Record {
    Reliable {
        client: ClientId,
        sequence: u32,
        text: Vec<u8>,
    },
    Move {
        client: ClientId,
        time: i32,
        movement: [f32; 3],
        weapon: u8,
    },
    Ack {
        client: ClientId,
        receipt: NativeReceipt,
    },
}

fn runtime_for(protocol: Protocol) -> (Runtime, [ClientId; 2]) {
    let mut runtime = Runtime::load(2, []).unwrap();
    let clients = std::array::from_fn(|seat| {
        runtime
            .connect_local(
                SeatId::new(seat as u8).unwrap(),
                SpawnAnchor {
                    position: Default::default(),
                    angles: Default::default(),
                    entity: 0,
                    fixture_fallback: false,
                },
                ClientPolicy {
                    input: RuleSetId::Quake3,
                    client: RuleSetId::Quake3,
                    movement: RuleSetId::Quake3,
                    trace: RuleSetId::Quake3,
                },
                protocol,
                seat as u32,
            )
            .unwrap()
    });
    (runtime, clients)
}

fn runtime() -> (Runtime, [ClientId; 2]) {
    let (mut runtime, clients) = runtime_for(Protocol::Quake3_68);
    assert_eq!(
        runtime
            .network
            .get(clients[0], Endpoint::Client)
            .unwrap()
            .channel
            .native_payload_capacity()
            .unwrap(),
        packet::Q3_MESSAGE_BYTES
    );
    runtime.loopback = Loopback::load([
        LoopbackLimits {
            maximum_message: 1400,
            payload_bytes: 1400,
            messages: 1,
        },
        LoopbackLimits {
            maximum_message: 1400,
            payload_bytes: 5600,
            messages: 4,
        },
    ])
    .unwrap();
    (runtime, clients)
}

fn movement(time: i32) -> UserCmd {
    UserCmd {
        server_time_ms: time,
        duration_ms: 16,
        movement: [127., -127., 0.],
        units: qa_gameplay::rules::command_units(RuleSetId::Quake3),
        ..UserCmd::default()
    }
}

#[test]
fn socket_rejection_retains_exact_client_flight_and_does_not_stall_a_peer() {
    let (mut runtime, clients @ [stalled, healthy]) = runtime();
    let servers = socket_routes(&mut runtime, clients);
    let time = EventTime::from_milliseconds(100);
    let mut source = frame_source::Sink {
        reject: Some(servers[0]),
        ..Default::default()
    };
    assert!(!runtime.send_client_command(stalled, &movement(16), time, &mut source));
    let connection = runtime.network.get(stalled, Endpoint::Client).unwrap();
    let pending = connection.channel.pending_packet().unwrap().bytes.to_vec();
    let sequence = connection.channel.send_state().sequence;
    assert_eq!(
        connection
            .commands
            .as_ref()
            .unwrap()
            .history()
            .unwrap()
            .number(),
        1
    );
    assert!(connection.channel.submitted_command(sequence).is_none());
    assert!(runtime.send_client_command(healthy, &movement(48), time, &mut source));
    assert!(!runtime.send_client_command(stalled, &movement(32), time, &mut source));
    let connection = runtime.network.get(stalled, Endpoint::Client).unwrap();
    let channel = &connection.channel;
    assert_eq!(channel.send_state().sequence, sequence);
    assert_eq!(channel.pending_packet().unwrap().bytes, pending);
    assert_eq!(
        connection
            .commands
            .as_ref()
            .unwrap()
            .history()
            .unwrap()
            .number(),
        2
    );
    assert!(channel.submitted_command(sequence).is_none());
    source.reject = None;
    // Finishing the retained flight must not claim this third produced command.
    assert!(!runtime.send_client_command(stalled, &movement(64), time, &mut source));
    assert_eq!(source.packets.len(), 4);
    for index in [0, 2, 3] {
        let packet = &source.packets[index];
        assert_eq!((packet.socket, packet.to), (70, servers[0]));
        assert_eq!(packet.bytes, pending);
        assert_eq!(packet.admitted, index == 3);
    }
    assert_eq!(
        (source.packets[1].socket, source.packets[1].to),
        (71, servers[1])
    );
    assert!(source.packets[1].admitted);
    let connection = runtime.network.get(stalled, Endpoint::Client).unwrap();
    assert_eq!(
        connection
            .commands
            .as_ref()
            .unwrap()
            .history()
            .unwrap()
            .number(),
        3
    );
    assert_eq!(
        connection.channel.submitted_command(sequence),
        Some(qa_network::channel::MoveCommand {
            number: 1,
            server_time_ms: 16
        })
    );
    assert!(
        connection
            .channel
            .submitted_command(sequence.wrapping_add(1))
            .is_none()
    );
    assert!(runtime.send_client_command(stalled, &movement(80), time, &mut source));
    assert_eq!(
        runtime
            .network
            .get(stalled, Endpoint::Client)
            .unwrap()
            .channel
            .submitted_command(sequence.wrapping_add(1)),
        Some(qa_network::channel::MoveCommand {
            number: 4,
            server_time_ms: 80
        })
    );

    // Captured socket datagrams enter the same queued SERVER decoder. No
    // loopback packet was emitted as a fallback for either socket connection.
    let mut queue = SysEventQueue::load(16, 8192).unwrap();
    for packet in source.packets.iter().filter(|packet| packet.admitted) {
        let index = servers
            .iter()
            .position(|server| *server == packet.to)
            .unwrap();
        queue
            .push(qa_core::sys_events::SysEvent {
                time,
                kind: EventKind::Packet {
                    socket: Endpoint::Server.socket(),
                    from: qa_core::sys_events::Peer::Socket(
                        ([127, 0, 0, 1], 32000 + index as u16).into(),
                    ),
                    bytes: &packet.bytes,
                },
            })
            .unwrap();
    }
    let mut records = Vec::new();
    assert_eq!(drain(&mut runtime, &mut queue, &mut records), 0);
    let times: Vec<_> = records
        .iter()
        .map(|record| match record {
            Record::Move { client, time, .. } => (*client, *time),
            _ => panic!("only native moves were submitted"),
        })
        .collect();
    assert_eq!(
        times,
        [
            (healthy, 48),
            (stalled, 16),
            (stalled, 32),
            (stalled, 64),
            (stalled, 80)
        ]
    );
}

fn queue_nq_service(
    host: &mut qa_app::host::FrameHost,
    client: ClientId,
    sequence: u32,
    body: &[u8],
    time: EventTime,
) {
    let route = host
        .runtime
        .network
        .get(client, Endpoint::Client)
        .unwrap()
        .route;
    let mut bytes = [0; 1400];
    let length = headers::encode(
        headers::NETQUAKE,
        Direction::ToClient,
        headers::Header {
            sequence,
            datagram_flags: headers::datagram::DATA | headers::datagram::EOM,
            ..Default::default()
        },
        body,
        &mut bytes,
    )
    .unwrap();
    host.queue
        .push(qa_core::sys_events::SysEvent {
            time,
            kind: EventKind::Packet {
                socket: route.socket,
                from: route.peer,
                bytes: &bytes[..length],
            },
        })
        .unwrap();
    host.drain(&mut Default::default());
}

#[test]
fn socket_startup_and_closing_controls_retry_without_movement_or_peer_teardown() {
    use qa_app::host::FrameHost;
    use qa_console::{commands::Console, views::Context};
    use qa_session::timing::TickRate;
    let (mut runtime, clients @ [client, healthy]) = runtime_for(Protocol::NetQuake15);
    let servers = socket_routes(&mut runtime, clients);
    let mut host = FrameHost::load(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        vec![],
    )
    .unwrap();
    host.local_clients[..clients.len()].copy_from_slice(&clients.map(Some));
    let time = EventTime::from_milliseconds(100);
    // Actual serverinfo plus stage1 establishes managed startup. This fixture
    // has no loaded world, so readiness must not activate movement.
    let mut info = vec![11];
    info.extend(15u32.to_le_bytes());
    info.extend([2, 0]);
    info.extend(b"transport\0\0\0");
    info.extend([25, 1]);
    queue_nq_service(&mut host, client, 0, &info, time);
    assert_eq!(
        host.runtime.client_resources[client.0 as usize]
            .as_ref()
            .unwrap()
            .nq_signon_stage(),
        Some(1)
    );
    let mut source = frame_source::Sink {
        reject: Some(servers[0]),
        time,
        ..Default::default()
    };
    assert!(
        !host
            .runtime
            .send_client_command(client, &movement(16), time, &mut source)
    );
    let pending = host
        .runtime
        .network
        .get(client, Endpoint::Client)
        .unwrap()
        .channel
        .pending_packet()
        .unwrap()
        .bytes
        .to_vec();
    let (header, body) = headers::decode(headers::NETQUAKE, Direction::ToServer, &pending).unwrap();
    assert_eq!(header.datagram_flags, headers::datagram::ACK);
    assert!(body.is_empty());
    let commands = host
        .runtime
        .network
        .get(client, Endpoint::Client)
        .unwrap()
        .commands
        .as_ref()
        .unwrap();
    assert_eq!(commands.history().unwrap().number(), 0);
    source.reject = None;
    assert!(
        !host
            .runtime
            .send_client_command(client, &movement(32), time, &mut source)
    );
    assert_eq!(source.packets.last().unwrap().bytes, pending);
    assert!(source.packets.last().unwrap().admitted);
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

    // The next real native service closes CLIENT presentation. Its final ACK
    // must retain the connection on rejection, then reach the socket before
    // teardown. The other seat continues ordinary command submission.
    queue_nq_service(&mut host, client, 1, &[2], time);
    assert!(
        host.runtime.snapshots[client.0 as usize]
            .as_ref()
            .unwrap()
            .presentation_closed()
    );
    source.reject = Some(servers[0]);
    for milliseconds in [100, 120, 140] {
        source.time = EventTime::from_milliseconds(milliseconds);
        host.queue
            .push(qa_core::sys_events::SysEvent {
                time: source.time,
                kind: EventKind::Time,
            })
            .unwrap();
        host.frame(&mut source, true);
    }
    let connection = host.runtime.network.get(client, Endpoint::Client).unwrap();
    let final_ack = connection.channel.pending_packet().unwrap().bytes.to_vec();
    let (header, body) =
        headers::decode(headers::NETQUAKE, Direction::ToServer, &final_ack).unwrap();
    assert_eq!(
        (header.datagram_flags, header.sequence),
        (headers::datagram::ACK, 1)
    );
    assert!(body.is_empty());
    assert!(
        host.runtime.server.clients[client.0 as usize]
            .connection
            .is_some()
    );
    assert!(
        source
            .packets
            .iter()
            .any(|packet| packet.to == servers[1] && packet.admitted)
    );
    assert!(
        host.runtime
            .network
            .get(healthy, Endpoint::Client)
            .unwrap()
            .commands
            .as_ref()
            .unwrap()
            .history()
            .unwrap()
            .number()
            > 0
    );
    source.reject = None;
    source.time = EventTime::from_milliseconds(160);
    host.queue
        .push(qa_core::sys_events::SysEvent {
            time: source.time,
            kind: EventKind::Time,
        })
        .unwrap();
    host.frame(&mut source, true);
    assert!(
        source
            .packets
            .iter()
            .any(|packet| packet.to == servers[0] && packet.admitted && packet.bytes == final_ack)
    );
    assert!(host.runtime.network.get(client, Endpoint::Client).is_none());
    assert!(
        host.runtime.server.clients[client.0 as usize]
            .connection
            .is_none()
    );
    assert!(
        host.runtime
            .network
            .get(healthy, Endpoint::Client)
            .is_some()
    );
    assert!(
        host.runtime.server.clients[healthy.0 as usize]
            .connection
            .is_some()
    );
    assert_eq!(host.runtime.network.malformed, 0);
    assert_eq!(host.runtime.network.command_errors, 0);
}

#[test]
fn native_control_submission_does_not_complete_an_unsent_move() {
    let mut runtime = Runtime::load(1, []).unwrap();
    let client = runtime
        .connect_local(
            SeatId::new(0).unwrap(),
            SpawnAnchor {
                position: Default::default(),
                angles: Default::default(),
                entity: 0,
                fixture_fallback: false,
            },
            ClientPolicy {
                input: RuleSetId::Quake,
                client: RuleSetId::Quake,
                movement: RuleSetId::Quake,
                trace: RuleSetId::Quake,
            },
            Protocol::NetQuake15,
            1,
        )
        .unwrap();
    let time = EventTime::from_milliseconds(100);
    let mut native = [0; 32];
    let length = headers::encode(
        headers::NETQUAKE,
        Direction::ToClient,
        headers::Header {
            datagram_flags: headers::datagram::DATA | headers::datagram::EOM,
            ..Default::default()
        },
        &[1],
        &mut native,
    )
    .unwrap();
    let channel = &mut runtime
        .network
        .get_mut(client, Endpoint::Client)
        .unwrap()
        .channel;
    channel.receive(&native[..length], time).unwrap();
    assert_eq!(channel.pending_controls(), 1);
    assert!(runtime.send_client_command(
        client,
        &movement(16),
        time,
        &mut frame_source::Sink::default()
    ));
    let mut queue = SysEventQueue::load(8, 4096).unwrap();
    assert_eq!(runtime.loopback.enqueue(&mut queue, time), 2);
    let mut flags = Vec::new();
    while let Some(event) = queue.pop() {
        let EventKind::Packet { bytes, .. } = event.kind else {
            panic!("expected native packet");
        };
        let (header, body) =
            headers::decode(headers::NETQUAKE, Direction::ToServer, bytes).unwrap();
        flags.push(header.datagram_flags);
        if header.datagram_flags == headers::datagram::UNRELIABLE {
            let mut body = body.to_vec();
            let packet::Move::NetQuake { command, .. } = packet::read(
                Protocol::NetQuake15,
                &mut body,
                header.sequence,
                packet::Key::default(),
            )
            .unwrap() else {
                panic!("expected command after control");
            };
            assert_eq!(
                command.movement,
                qa_network::commands::to_q1_move(&movement(16)).movement
            );
        }
    }
    assert_eq!(
        flags,
        [headers::datagram::ACK, headers::datagram::UNRELIABLE]
    );
}

/// Both directions enter the actual system queue before native routing. This
/// observer retains only fixture results, never a production input history.
fn drain(runtime: &mut Runtime, queue: &mut SysEventQueue, records: &mut Vec<Record>) -> usize {
    let time = EventTime::from_milliseconds(100);
    let queued = runtime.loopback.enqueue(queue, time);
    while let Some(event) = queue.pop() {
        let EventKind::Packet {
            socket,
            from,
            bytes,
        } = event.kind
        else {
            panic!("expected queued Packet");
        };
        assert!(bytes.len() <= 1400);
        let direction = if socket == Endpoint::Server.socket() {
            Direction::ToServer
        } else {
            assert_eq!(socket, Endpoint::Client.socket());
            Direction::ToClient
        };
        headers::decode(headers::QUAKE3, direction, bytes).unwrap();
        runtime.network.receive(
            socket,
            from,
            bytes,
            event.time,
            |client, endpoint, incoming| {
                match incoming {
                    Incoming::CommandString {
                        protocol,
                        sequence,
                        text,
                        ..
                    } => {
                        assert_eq!(endpoint, Endpoint::Server);
                        assert_eq!(protocol, Protocol::Quake3_68);
                        records.push(Record::Reliable {
                            client,
                            sequence: sequence.unwrap(),
                            text: text.to_vec(),
                        });
                    }
                    Incoming::Command {
                        command,
                        native_weapon,
                        ..
                    } => {
                        assert_eq!(endpoint, Endpoint::Server);
                        records.push(Record::Move {
                            client,
                            time: command.server_time_ms,
                            movement: command.movement,
                            weapon: native_weapon.unwrap(),
                        });
                    }
                    Incoming::Acknowledged { receipt, .. } => {
                        records.push(Record::Ack { client, receipt });
                    }
                    _ => panic!("unexpected queued Q3 record"),
                }
                ControlFlow::Continue(())
            },
        );
    }
    assert_eq!(runtime.network.malformed, 0);
    assert_eq!(runtime.network.command_errors, 0);
    queued
}

fn reply(runtime: &mut Runtime, client: ClientId) {
    let time = EventTime::from_milliseconds(100);
    let channel = &mut runtime
        .network
        .get_mut(client, Endpoint::Server)
        .unwrap()
        .channel;
    channel.prepare_output(time).unwrap().unwrap();
    assert!(
        channel
            .submit_with(time, |bytes| {
                runtime
                    .loopback
                    .send(Endpoint::Server, client, bytes)
                    .is_ok()
            })
            .unwrap()
            .is_some()
    );
}

#[test]
fn queued_client_fragments_do_not_claim_a_current_command_while_an_older_flight_drains() {
    let (mut runtime, [stalled, healthy]) = runtime();
    let a = [b'x'; 900];
    let b = b"later command";
    let channel = &mut runtime
        .network
        .get_mut(stalled, Endpoint::Client)
        .unwrap()
        .channel;
    assert_eq!(channel.queue_reliable(&a), Ok(NativeReceipt(1)));
    assert_eq!(channel.queue_reliable(&a), Ok(NativeReceipt(2)));
    let time = EventTime::from_milliseconds(100);
    assert!(!runtime.send_client_command(
        stalled,
        &movement(16),
        time,
        &mut frame_source::Sink::default()
    ));
    let channel = &runtime
        .network
        .get(stalled, Endpoint::Client)
        .unwrap()
        .channel;
    assert!(channel.pending_fragments());
    assert!(channel.pending_packet().is_some());
    assert_eq!(channel.command_state().unwrap().submitted, 0);
    let pending = channel.pending_packet().unwrap().bytes.to_vec();
    let (header, _) = headers::decode(headers::QUAKE3, Direction::ToServer, &pending).unwrap();
    assert!(
        !header.fragment.unwrap().more,
        "the blocked packet is A's final fragment"
    );
    runtime
        .network
        .get_mut(stalled, Endpoint::Client)
        .unwrap()
        .channel
        .queue_reliable(b)
        .unwrap();

    // Another connection uses the same Runtime scratch while A is retained.
    assert!(runtime.send_client_command(
        healthy,
        &movement(48),
        time,
        &mut frame_source::Sink::default()
    ));
    let mut queue = SysEventQueue::load(64, packet::Q3_MESSAGE_BYTES * 2).unwrap();
    let mut records = Vec::new();
    assert_eq!(drain(&mut runtime, &mut queue, &mut records), 2);
    assert_eq!(
        records,
        [Record::Move {
            client: healthy,
            time: 48,
            movement: [127., -127., 0.],
            weapon: 0
        }]
    );

    // This call can finish only A. It must return false for the unsent current
    // movement, and B must not inherit A's real message submission watermark.
    assert!(!runtime.send_client_command(
        stalled,
        &movement(32),
        time,
        &mut frame_source::Sink::default()
    ));
    let channel = &mut runtime
        .network
        .get_mut(stalled, Endpoint::Client)
        .unwrap()
        .channel;
    assert!(!channel.pending_fragments());
    assert_eq!(channel.command_state().unwrap().queued, 3);
    assert_eq!(channel.command_state().unwrap().submitted, 2);
    assert_eq!(channel.command_state().unwrap().acknowledged, 0);
    assert!(channel.acknowledge_commands(3, header.sequence).is_err());
    assert_eq!(drain(&mut runtime, &mut queue, &mut records), 1);
    assert_eq!(
        &records[1..],
        &[
            Record::Reliable {
                client: stalled,
                sequence: 1,
                text: a.to_vec()
            },
            Record::Reliable {
                client: stalled,
                sequence: 2,
                text: a.to_vec()
            },
            Record::Move {
                client: stalled,
                time: 16,
                movement: [127., -127., 0.],
                weapon: 0
            },
        ]
    );

    reply(&mut runtime, stalled);
    assert_eq!(drain(&mut runtime, &mut queue, &mut records), 1);
    assert_eq!(
        &records[4..],
        &[
            Record::Ack {
                client: stalled,
                receipt: NativeReceipt(1)
            },
            Record::Ack {
                client: stalled,
                receipt: NativeReceipt(2)
            },
        ]
    );
    assert_eq!(
        runtime
            .network
            .get(stalled, Endpoint::Client)
            .unwrap()
            .channel
            .command_state()
            .unwrap()
            .acknowledged,
        2
    );

    assert!(runtime.send_client_command(
        stalled,
        &movement(32),
        time,
        &mut frame_source::Sink::default()
    ));
    assert_eq!(drain(&mut runtime, &mut queue, &mut records), 1);
    assert_eq!(
        &records[6..],
        &[
            Record::Reliable {
                client: stalled,
                sequence: 3,
                text: b.to_vec()
            },
            Record::Move {
                client: stalled,
                time: 32,
                movement: [127., -127., 0.],
                weapon: 0
            },
        ]
    );
    reply(&mut runtime, stalled);
    drain(&mut runtime, &mut queue, &mut records);
    assert_eq!(
        records.last(),
        Some(&Record::Ack {
            client: stalled,
            receipt: NativeReceipt(3)
        })
    );
    assert_eq!(runtime.network.commands, 3);
    assert_eq!(
        runtime
            .network
            .get(stalled, Endpoint::Client)
            .unwrap()
            .channel
            .command_state()
            .unwrap()
            .acknowledged,
        3
    );
}

#[test]
fn admitted_native_client_window_larger_than_a_packet_reaches_queued_ingress() {
    let (mut runtime, [client, _]) = runtime();
    // Use the existing connection transport with enough owned ring slots for
    // every fragment. The old1400-byte encode buffer could not admit even two
    // of these commands, independent of transport capacity.
    runtime.loopback = Loopback::load(std::iter::repeat_n(
        LoopbackLimits {
            maximum_message: 1400,
            payload_bytes: packet::Q3_MESSAGE_BYTES * 2,
            messages: 32,
        },
        2,
    ))
    .unwrap();
    let text = [b'x'; 900];
    let mut accepted = 0;
    loop {
        match runtime
            .network
            .get_mut(client, Endpoint::Client)
            .unwrap()
            .channel
            .queue_reliable(&text)
        {
            Ok(receipt) => {
                accepted += 1;
                assert_eq!(receipt, NativeReceipt(accepted));
            }
            Err(TransmitError::Full) => break,
            other => panic!("individually valid command: {other:?}"),
        }
    }
    assert!(accepted > 2 && accepted < 64);
    let time = EventTime::from_milliseconds(100);
    assert!(runtime.send_client_command(
        client,
        &movement(16),
        time,
        &mut frame_source::Sink::default()
    ));
    let state = runtime
        .network
        .get(client, Endpoint::Client)
        .unwrap()
        .channel
        .command_state()
        .unwrap();
    assert_eq!(state.submitted, accepted as u32);
    assert_eq!(state.acknowledged, 0);
    assert_eq!(
        runtime
            .network
            .get_mut(client, Endpoint::Client)
            .unwrap()
            .channel
            .queue_reliable(&text),
        Err(TransmitError::Full)
    );
    let mut queue = SysEventQueue::load(64, packet::Q3_MESSAGE_BYTES * 2).unwrap();
    let mut records = Vec::new();
    assert!(drain(&mut runtime, &mut queue, &mut records) > 1);
    for (index, record) in records[..accepted as usize].iter().enumerate() {
        assert_eq!(
            *record,
            Record::Reliable {
                client,
                sequence: index as u32 + 1,
                text: text.to_vec()
            }
        );
    }
    assert_eq!(
        records.last(),
        Some(&Record::Move {
            client,
            time: 16,
            movement: [127., -127., 0.],
            weapon: 0
        })
    );
    reply(&mut runtime, client);
    drain(&mut runtime, &mut queue, &mut records);
    assert_eq!(
        runtime
            .network
            .get(client, Endpoint::Client)
            .unwrap()
            .channel
            .command_state()
            .unwrap()
            .acknowledged,
        accepted as u32
    );
    assert_eq!(
        runtime
            .network
            .get_mut(client, Endpoint::Client)
            .unwrap()
            .channel
            .queue_reliable(&text),
        Ok(NativeReceipt(accepted + 1))
    );
}
