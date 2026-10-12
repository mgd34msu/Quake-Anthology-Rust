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
    pair_with_limit(16384)
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
        let mut codec = Commands::load(Protocol::Quake3_68);
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
    let mut bytes = [0; 8192];
    let encoder = Commands::load(Protocol::Quake3_68);
    let n = encoder.encode(&command, 0, client, &mut bytes).unwrap();
    let data = client
        .prepare_move(&bytes[..n], EventTime(time as u64), None)
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
                .decode_server_output(&mut body, seq, |_, text| texts.push(text.to_vec()))
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
    let mut codec = Commands::load(Protocol::Quake3_68);
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
                .decode_server_output(&mut bytes, seq, |_, text| texts.push(text.to_vec()))
                .unwrap();
        }
    }
    assert_eq!(texts.len(), 64);
    assert_eq!(server.command_state().unwrap().submitted, 64);
    let mut codec = Commands::load(Protocol::Quake3_68);
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
                .decode_server_output(&mut bytes, seq, |_, bytes| text.extend_from_slice(bytes))
                .unwrap();
            break;
        }
    }
    assert_eq!(text, vec![b'y'; 1022]);
    assert_eq!(server.command_state().unwrap().submitted, 1);
    assert!(server.acknowledge_commands(2, 1).is_err());
    assert!(server.acknowledge_commands(1, 0).is_err());
    let mut codec = Commands::load(Protocol::Quake3_68);
    move_message(&mut client, &mut server, &mut codec, 16);
    assert_eq!(server.reliable_receipts(), &[receipt]);
}
