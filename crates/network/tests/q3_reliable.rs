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
    let context = CommandContext {
        server_id: 13,
        challenge: 0x8765_abcd,
        checksum_feed: 0x1234_7654,
    };
    let mut client = Channel::load(channel::QUAKE3, Endpoint::Client, 16384, 8).unwrap();
    let mut server = Channel::load(channel::QUAKE3, Endpoint::Server, 16384, 8).unwrap();
    for c in [&mut client, &mut server] {
        c.set_command_context(context).unwrap();
    }
    (client, server)
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
    let n = encoder.encode(&command, client, &mut bytes).unwrap();
    let data = client
        .prepare_move(&bytes[..n], EventTime(time as u64))
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
    let cmd = codec
        .decode(n, sequence, time, 16_000_000, server)
        .unwrap()
        .unwrap();
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
                .decode_command_output(&mut body, seq, |_, text| texts.push(text.to_vec()))
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
                .decode_command_output(&mut bytes, seq, |_, text| texts.push(text.to_vec()))
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
    let receipt = server.queue_reliable(&[b'y'; 2000]).unwrap();
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
            assert_eq!(
                client.decode_command_output(&mut bytes, seq, |_, bytes| text
                    .extend_from_slice(bytes)),
                Err(packet::Error::Opcode)
            );
            break;
        }
    }
    assert_eq!(text, vec![b'y'; 1023]);
    assert_eq!(server.command_state().unwrap().submitted, 1);
    assert!(server.acknowledge_commands(2, 1).is_err());
    assert!(server.acknowledge_commands(1, 0).is_err());
    server.acknowledge_commands(1, 1).unwrap();
    assert_eq!(server.reliable_receipts(), &[receipt]);
}
