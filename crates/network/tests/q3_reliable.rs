use qa_core::{
    events::NativeReceipt, loopback::Endpoint, primitives::UserCmd, sys_events::EventTime,
};
use qa_network::{
    channel::{self, Channel, Delivery, TransmitError, commands::CommandContext},
    commands::{
        connection::Commands,
        packet::{self, Protocol},
    },
};

fn pair() -> (Channel, Channel) {
    pair_with_limit(packet::Q3_MESSAGE_BYTES)
}
fn pair_with_limit(maximum: usize) -> (Channel, Channel) {
    let context = CommandContext {
        server_id: 13,
        challenge: 0x8765_abcd,
        checksum_feed: 0x1234_7654,
    };
    let mut client = Channel::load(channel::QUAKE3, Endpoint::Client, maximum, 8).unwrap();
    let mut server = Channel::load(channel::QUAKE3, Endpoint::Server, maximum, 8).unwrap();
    for c in [&mut client, &mut server] {
        c.set_command_context(context).unwrap();
    }
    (client, server)
}

#[test]
fn encoded_command_window_drains_before_ack_and_refunds_only_after_ack() {
    for maximum in [16384, 32768] {
        let (mut client, mut server) = pair_with_limit(maximum);
        let text = [b'x'; 900];
        let mut accepted = 0;
        for _ in 0..64 {
            match server.queue_reliable(&text) {
                Ok(receipt) => {
                    accepted += 1;
                    assert_eq!(receipt, NativeReceipt(accepted));
                }
                Err(TransmitError::Full) => break,
                other => panic!("individually representable command: {other:?}"),
            }
        }
        assert!(accepted > 0 && accepted < 64);
        assert_eq!(server.queue_reliable(&text), Err(TransmitError::Full));
        assert_eq!(server.command_state().unwrap().queued, accepted as u32);
        assert!(server.acknowledge_commands(accepted as u32, 1).is_err());
        let received = outputs(&mut server, &mut client);
        assert_eq!(received.len(), accepted as usize);
        assert!(received.iter().all(|received| received == &text));
        assert_eq!(server.command_state().unwrap().submitted, accepted as u32);
        assert_eq!(server.queue_reliable(&text), Err(TransmitError::Full));
        let mut codec = Commands::load(Protocol::Quake3_68, maximum);
        move_message(&mut client, &mut server, &mut codec, 16);
        assert_eq!(server.reliable_receipts().len(), accepted as usize);
        assert_eq!(
            server.command_state().unwrap().acknowledged,
            accepted as u32
        );
        assert_eq!(
            server.queue_reliable(&text),
            Ok(NativeReceipt(accepted + 1))
        );
        assert_eq!(outputs(&mut server, &mut client), [text.to_vec()]);
        move_message(&mut client, &mut server, &mut codec, 32);
        assert_eq!(server.reliable_receipts(), &[NativeReceipt(accepted + 1)]);
    }
}
fn move_message(client: &mut Channel, server: &mut Channel, codec: &mut Commands, time: i32) {
    let command = UserCmd {
        server_time_ms: time,
        duration_ms: 16,
        movement: [127., -127., 0.],
        ..UserCmd::default()
    };
    let maximum = client.native_payload_capacity().unwrap();
    let mut bytes = vec![0; maximum];
    let encoder = Commands::load(Protocol::Quake3_68, maximum);
    let n = encoder.encode(&command, 0, client, &mut bytes).unwrap();
    let data = client
        .prepare_move(&bytes[..n], EventTime(time as u64), None, None)
        .unwrap()
        .unwrap()
        .bytes
        .to_vec();
    client.submitted(EventTime(time as u64)).unwrap();
    let received = server.receive(&data, EventTime(time as u64)).unwrap();
    let sequence = received.header.sequence;
    let Delivery::Payload(payload) = received.delivery else {
        panic!("complete move")
    };
    let n = codec.stage(payload).unwrap();
    let mut decoded = None;
    assert_eq!(
        codec
            .decode(n, sequence, time, server, None, |incoming| {
                if let qa_network::ingress::Incoming::Command {
                    command: cmd,
                    native_weapon: _,
                    observation: _,
                    ..
                } = incoming
                {
                    decoded = Some(cmd);
                }
                std::ops::ControlFlow::Continue(())
            })
            .unwrap(),
        1
    );
    let cmd = decoded.unwrap();
    assert_eq!(cmd.server_time_ms, time);
    assert_eq!(cmd.movement, command.movement);
}
fn outputs(server: &mut Channel, client: &mut Channel) -> Vec<Vec<u8>> {
    let mut texts = Vec::new();
    loop {
        let bytes = server
            .prepare_output(EventTime(1))
            .unwrap()
            .unwrap()
            .bytes
            .to_vec();
        server.submitted(EventTime(1)).unwrap();
        let received = client.receive(&bytes, EventTime(1)).unwrap();
        let seq = received.header.sequence;
        if let Delivery::Payload(body) = received.delivery {
            let mut body = body.to_vec();
            client
                .decode_server_output(&mut body, seq, |output| {
                    if let qa_network::channel::commands::ServerOutput::ReliableCommand {
                        text,
                        ..
                    } = output
                    {
                        texts.push(text.to_vec());
                    }
                })
                .unwrap();
            break;
        }
    }
    texts
}
#[test]
fn native_commands_and_actual_message_ack_retire_each_direction() {
    let (mut client, mut server) = pair();
    let receipt = client.queue_reliable(b"userinfo \"name native\"").unwrap();
    let mut codec = Commands::load(Protocol::Quake3_68, 8192);
    move_message(&mut client, &mut server, &mut codec, 16);
    assert_eq!(
        server.received_command().unwrap(),
        (1, &b"userinfo \"name native\""[..])
    );
    let first = server.queue_reliable(b"cp \"native one\"").unwrap();
    let second = server.queue_reliable(b"print \"native two\"").unwrap();
    assert_eq!(
        outputs(&mut server, &mut client),
        [
            b"cp \"native one\"".to_vec(),
            b"print \"native two\"".to_vec()
        ]
    );
    assert_eq!(client.reliable_receipts(), &[receipt]);
    assert_eq!(server.command_state().unwrap().submitted, 2);
    assert_eq!(server.command_state().unwrap().acknowledged, 0);
    move_message(&mut client, &mut server, &mut codec, 32);
    assert_eq!(server.reliable_receipts(), &[first, second]);
    assert_eq!(server.command_state().unwrap().acknowledged, 2);
    assert!(!server.has_output());
    assert_eq!(
        server
            .command_key(Some(packet::Acknowledgements {
                server_id: 13,
                message: 1,
                reliable: 1
            }))
            .unwrap()
            .server_command,
        b"cp \"native one\""
    );
    server.acknowledge_commands(2, 1).unwrap();
    assert!(server.reliable_receipts().is_empty());
}
#[test]
fn rejected_packet_and_partial_fragments_cannot_ack_queued_commands() {
    let (mut client, mut server) = pair();
    for _ in 0..64 {
        server.queue_reliable(&[b'x'; 160]).unwrap();
    }
    assert_eq!(server.queue_reliable(b"overflow"), Err(TransmitError::Full));
    assert!(server.acknowledge_commands(1, 0).is_err());
    let first = server
        .prepare_output(EventTime(0))
        .unwrap()
        .unwrap()
        .bytes
        .to_vec();
    assert!(matches!(
        server.prepare_output(EventTime(0)),
        Err(TransmitError::PendingPacket)
    ));
    assert_eq!(server.pending_packet().unwrap().bytes, first);
    assert_eq!(server.command_state().unwrap().submitted, 0);
    assert!(server.acknowledge_commands(1, 1).is_err());
    server.submitted(EventTime(1)).unwrap();
    let received = client.receive(&first, EventTime(1)).unwrap();
    assert_eq!(received.delivery, Delivery::Pending);
    assert_eq!(server.command_state().unwrap().submitted, 0);
    assert!(server.acknowledge_commands(64, 1).is_err());
    let mut texts = Vec::new();
    while server.send_state().fragment_bytes != 0 {
        let data = server
            .prepare_output(EventTime(2))
            .unwrap()
            .unwrap()
            .bytes
            .to_vec();
        server.submitted(EventTime(2)).unwrap();
        let received = client.receive(&data, EventTime(2)).unwrap();
        let seq = received.header.sequence;
        if let Delivery::Payload(body) = received.delivery {
            let mut bytes = body.to_vec();
            client
                .decode_server_output(&mut bytes, seq, |output| {
                    if let qa_network::channel::commands::ServerOutput::ReliableCommand {
                        text,
                        ..
                    } = output
                    {
                        texts.push(text.to_vec());
                    }
                })
                .unwrap();
        }
    }
    assert_eq!(texts.len(), 64);
    assert_eq!(server.command_state().unwrap().submitted, 64);
    let mut codec = Commands::load(Protocol::Quake3_68, 8192);
    move_message(&mut client, &mut server, &mut codec, 16);
    assert_eq!(server.reliable_receipts().len(), 64);
    assert_eq!(server.reliable_receipts()[0], NativeReceipt(1));
    assert_eq!(server.reliable_receipts()[63], NativeReceipt(64));
    assert_eq!(server.queue_reliable(b"next"), Ok(NativeReceipt(65)));
    assert_eq!(
        server
            .command_key(Some(packet::Acknowledgements {
                server_id: 13,
                message: 1,
                reliable: 1
            }))
            .unwrap()
            .server_command,
        b"next"
    );
}
#[test]
fn native_string_width_gaps_duplicates_and_future_ack_are_bounded() {
    let (mut client, mut server) = pair();
    assert!(server.receive_command(1, b"first").unwrap());
    assert!(server.receive_command(3, b"gap").is_err());
    assert_eq!(server.received_command().unwrap().0, 1);
    assert!(client.receive_command(3, b"server gap").unwrap());
    assert!(!client.receive_command(2, b"old").unwrap());
    let (mut client, mut server) = pair();
    assert_eq!(
        server.queue_reliable(&[b'y'; 1023]),
        Err(TransmitError::MessageTooLarge)
    );
    assert_eq!(
        server.queue_reliable(&[b'y'; 2000]),
        Err(TransmitError::MessageTooLarge)
    );
    assert_eq!(server.command_state().unwrap().queued, 0);
    let receipt = server.queue_reliable(&[b'y'; 1022]).unwrap();
    let mut text = Vec::new();
    loop {
        let data = server
            .prepare_output(EventTime(1))
            .unwrap()
            .unwrap()
            .bytes
            .to_vec();
        server.submitted(EventTime(1)).unwrap();
        let received = client.receive(&data, EventTime(1)).unwrap();
        let seq = received.header.sequence;
        if let Delivery::Payload(body) = received.delivery {
            let mut bytes = body.to_vec();
            client
                .decode_server_output(&mut bytes, seq, |output| {
                    if let qa_network::channel::commands::ServerOutput::ReliableCommand {
                        text: bytes,
                        ..
                    } = output
                    {
                        text.extend_from_slice(bytes);
                    }
                })
                .unwrap();
            break;
        }
    }
    assert_eq!(text, vec![b'y'; 1022]);
    assert_eq!(server.command_state().unwrap().submitted, 1);
    assert!(server.acknowledge_commands(2, 1).is_err());
    assert!(server.acknowledge_commands(1, 0).is_err());
    let mut codec = Commands::load(Protocol::Quake3_68, 8192);
    move_message(&mut client, &mut server, &mut codec, 16);
    assert_eq!(server.reliable_receipts(), &[receipt]);
}

#[derive(Debug, PartialEq)]
enum ClientRecord {
    Reliable(u32, Vec<u8>),
    Move {
        time: i32,
        movement: [f32; 3],
        weapon: u8,
    },
}

fn client_records(
    server: &mut Channel,
    codec: &mut Commands,
    packet_bytes: &[u8],
    records: &mut Vec<ClientRecord>,
) -> bool {
    let received = server.receive(packet_bytes, EventTime(1)).unwrap();
    let sequence = received.header.sequence;
    let Delivery::Payload(payload) = received.delivery else {
        assert_eq!(received.delivery, Delivery::Pending);
        return false;
    };
    let length = codec.stage(payload).unwrap();
    assert_eq!(
        codec
            .decode(length, sequence, 100, server, None, |incoming| {
                match incoming {
                    qa_network::ingress::Incoming::CommandString { sequence, text, .. } => {
                        records.push(ClientRecord::Reliable(sequence.unwrap(), text.to_vec()));
                    }
                    qa_network::ingress::Incoming::Command {
                        command,
                        native_weapon,
                        ..
                    } => {
                        records.push(ClientRecord::Move {
                            time: command.server_time_ms,
                            movement: command.movement,
                            weapon: native_weapon.unwrap(),
                        });
                    }
                    _ => panic!("unexpected CLIENT record"),
                }
                std::ops::ControlFlow::Continue(())
            })
            .unwrap(),
        1
    );
    true
}

fn movement(time: i32) -> UserCmd {
    UserCmd {
        server_time_ms: time,
        duration_ms: 16,
        movement: [127., -127., 0.],
        ..UserCmd::default()
    }
}

#[test]
fn client_encoded_windows_retain_rejected_fragments_and_refund_only_native_acks() {
    for maximum in [8192, packet::Q3_MESSAGE_BYTES, 32768] {
        let (mut client, mut server) = pair_with_limit(maximum);
        let mut texts = Vec::new();
        for sequence in 1..=64 {
            let mut text = format!("command {sequence} ").into_bytes();
            text.resize(900, b'x');
            match client.queue_reliable(&text) {
                Ok(receipt) => {
                    assert_eq!(receipt, NativeReceipt(sequence));
                    texts.push(text);
                }
                Err(TransmitError::Full) => break,
                other => panic!("individually representable CLIENT command: {other:?}"),
            }
        }
        let accepted = texts.len() as u32;
        assert!(accepted > 2 && accepted < 64);
        assert_eq!(client.queue_reliable(&texts[0]), Err(TransmitError::Full));
        assert_eq!(client.command_state().unwrap().submitted, 0);
        assert!(client.acknowledge_commands(accepted, 1).is_err());
        let encoder = Commands::load(Protocol::Quake3_68, maximum);
        let mut codec = Commands::load(Protocol::Quake3_68, maximum);
        let mut encoded = vec![0; maximum];
        let length = encoder
            .encode(&movement(16), 7, &client, &mut encoded)
            .unwrap();
        assert!(length > 1400 && length <= maximum);
        client
            .prepare_move(&encoded[..length], EventTime(1), None, None)
            .unwrap()
            .unwrap();
        let mut records = Vec::new();
        let mut fragment = 0;
        let mut rejected_middle = false;
        let mut rejected_final = false;
        loop {
            if client.pending_packet().is_none() {
                client.prepare(None, EventTime(1)).unwrap().unwrap();
            }
            let packet_bytes = client.pending_packet().unwrap().bytes.to_vec();
            assert!(packet_bytes.len() <= 1400);
            let (header, _) = qa_network::headers::decode(
                qa_network::headers::QUAKE3,
                qa_network::headers::Direction::ToServer,
                &packet_bytes,
            )
            .unwrap();
            let final_fragment = !header.fragment.unwrap().more;
            if fragment == 1 || final_fragment {
                let before = client.send_state();
                assert_eq!(
                    client
                        .submit_with(EventTime(1), |offered| {
                            assert_eq!(offered, packet_bytes);
                            false
                        })
                        .unwrap(),
                    None
                );
                assert_eq!(client.send_state(), before);
                assert_eq!(client.pending_packet().unwrap().bytes, packet_bytes);
                assert_eq!(client.command_state().unwrap().submitted, 0);
                assert!(
                    client
                        .acknowledge_commands(accepted, header.sequence)
                        .is_err()
                );
                assert_eq!(client.queue_reliable(&texts[0]), Err(TransmitError::Full));
                rejected_middle |= fragment == 1;
                rejected_final |= final_fragment;
            }
            assert!(
                client
                    .submit_with(EventTime(1), |offered| {
                        assert_eq!(offered, packet_bytes);
                        true
                    })
                    .unwrap()
                    .is_some()
            );
            let completed = client_records(&mut server, &mut codec, &packet_bytes, &mut records);
            assert_eq!(completed, final_fragment);
            if completed {
                break;
            }
            assert!(records.is_empty());
            assert_eq!(client.command_state().unwrap().submitted, 0);
            fragment += 1;
        }
        assert!(rejected_middle && rejected_final);
        assert!(!client.pending_fragments());
        assert_eq!(client.command_state().unwrap().submitted, accepted);
        assert_eq!(client.command_state().unwrap().acknowledged, 0);
        let mut expected: Vec<_> = texts
            .iter()
            .enumerate()
            .map(|(index, text)| ClientRecord::Reliable(index as u32 + 1, text.clone()))
            .collect();
        expected.push(ClientRecord::Move {
            time: 16,
            movement: [127., -127., 0.],
            weapon: 7,
        });
        assert_eq!(records, expected);
        assert_eq!(client.queue_reliable(&texts[0]), Err(TransmitError::Full));

        // The SERVER's ordinary native response carries the actual reliable
        // acknowledgement; submission alone has not reclaimed this budget.
        assert!(outputs(&mut server, &mut client).is_empty());
        assert_eq!(client.command_state().unwrap().acknowledged, accepted);
        assert_eq!(client.reliable_receipts().len(), accepted as usize);
        for (index, receipt) in client.reliable_receipts().iter().enumerate() {
            assert_eq!(*receipt, NativeReceipt(index as u64 + 1));
        }
        let receipt = client.queue_reliable(&texts[0]).unwrap();
        assert_eq!(receipt, NativeReceipt(u64::from(accepted) + 1));
    }
}

#[test]
fn newer_client_command_cannot_inherit_an_older_fragment_flights_submission() {
    let maximum = packet::Q3_MESSAGE_BYTES;
    let (mut client, mut server) = pair_with_limit(maximum);
    let a = [b'x'; 900];
    let b = b"later command";
    client.queue_reliable(&a).unwrap();
    client.queue_reliable(&a).unwrap();
    let encoder = Commands::load(Protocol::Quake3_68, maximum);
    let mut codec = Commands::load(Protocol::Quake3_68, maximum);
    let mut encoded = vec![0; maximum];
    let length = encoder
        .encode(&movement(16), 7, &client, &mut encoded)
        .unwrap();
    let first = client
        .prepare_move(&encoded[..length], EventTime(1), None, None)
        .unwrap()
        .unwrap()
        .bytes
        .to_vec();
    client.submit_with(EventTime(1), |_| true).unwrap().unwrap();
    let mut records = Vec::new();
    assert!(!client_records(
        &mut server,
        &mut codec,
        &first,
        &mut records
    ));
    assert_eq!(client.command_state().unwrap().submitted, 0);
    client.queue_reliable(b).unwrap();
    let next = encoder
        .encode(&movement(32), 9, &client, &mut encoded)
        .unwrap();
    while client.pending_fragments() {
        // Even this direct caller supplies a newer body and watermark. The
        // channel must keep both attached to their original encoded flight.
        let packet_bytes = client
            .prepare_move(&encoded[..next], EventTime(2), None, None)
            .unwrap()
            .unwrap()
            .bytes
            .to_vec();
        client.submit_with(EventTime(2), |_| true).unwrap().unwrap();
        client_records(&mut server, &mut codec, &packet_bytes, &mut records);
    }
    assert_eq!(client.command_state().unwrap().queued, 3);
    assert_eq!(client.command_state().unwrap().submitted, 2);
    assert!(client.acknowledge_commands(3, 1).is_err());
    assert_eq!(
        records,
        [
            ClientRecord::Reliable(1, a.to_vec()),
            ClientRecord::Reliable(2, a.to_vec()),
            ClientRecord::Move {
                time: 16,
                movement: [127., -127., 0.],
                weapon: 7
            },
        ]
    );
    assert!(outputs(&mut server, &mut client).is_empty());
    assert_eq!(
        client.reliable_receipts(),
        &[NativeReceipt(1), NativeReceipt(2)]
    );

    // Only a fresh native message can submit B, after A's real ACK.
    let length = encoder
        .encode(&movement(32), 9, &client, &mut encoded)
        .unwrap();
    let packet_bytes = client
        .prepare_move(&encoded[..length], EventTime(3), None, None)
        .unwrap()
        .unwrap()
        .bytes
        .to_vec();
    client.submit_with(EventTime(3), |_| true).unwrap().unwrap();
    assert!(client_records(
        &mut server,
        &mut codec,
        &packet_bytes,
        &mut records
    ));
    assert_eq!(
        &records[3..],
        &[
            ClientRecord::Reliable(3, b.to_vec()),
            ClientRecord::Move {
                time: 32,
                movement: [127., -127., 0.],
                weapon: 9
            },
        ]
    );
    assert_eq!(client.command_state().unwrap().submitted, 3);
    assert!(outputs(&mut server, &mut client).is_empty());
    assert_eq!(client.reliable_receipts(), &[NativeReceipt(3)]);
}
