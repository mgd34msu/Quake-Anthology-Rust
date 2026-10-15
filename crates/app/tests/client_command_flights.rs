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

fn runtime() -> (Runtime, [ClientId; 2]) {
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
                Protocol::Quake3_68,
                seat as u32,
            )
            .unwrap()
    });
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
    assert!(runtime.send_local_command(client, &movement(16), time));
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
    assert!(!runtime.send_local_command(stalled, &movement(16), time));
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
    assert!(runtime.send_local_command(healthy, &movement(48), time));
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
    assert!(!runtime.send_local_command(stalled, &movement(32), time));
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

    assert!(runtime.send_local_command(stalled, &movement(32), time));
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
    assert!(runtime.send_local_command(client, &movement(16), time));
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
