use qa_core::{
    configstrings::{ConfigStrings, Q3_GAME_STATE_DATA_BYTES},
    events::NativeReceipt,
    loopback::Endpoint,
    primitives::ThinkTime,
    sys_events::EventTime,
};
use qa_network::{
    channel::{
        self, Channel, Delivery, TransmitError,
        commands::{CommandContext, ServerOutput, write_gamestate},
    },
    commands::{
        connection::Commands,
        packet::{self, Protocol},
    },
    message::{Encoding, Reader, StringPolicy, Writer},
    snapshots::{Entity, Frame, ReceivedFrame},
    states,
};
use std::ops::ControlFlow;

const MAXIMUM: usize = packet::Q3_MESSAGE_BYTES;
const CONTEXT: CommandContext = CommandContext {
    server_id: 13,
    challenge: 0x8765_abcd,
    checksum_feed: 0x1234_7654,
};

fn pair() -> (Channel, Channel) {
    let mut client = Channel::load(channel::QUAKE3, Endpoint::Client, MAXIMUM, 8).unwrap();
    let mut server = Channel::load(channel::QUAKE3, Endpoint::Server, MAXIMUM, 8).unwrap();
    for channel in [&mut client, &mut server] {
        channel.set_command_context(CONTEXT).unwrap();
    }
    (client, server)
}

fn encode_state(server: &Channel, text: &[u8]) -> (Vec<u8>, u32, usize) {
    let commands = server.command_state().unwrap().queued;
    let mut bytes = vec![0; MAXIMUM];
    let mut writer = Writer::new(&mut bytes, Encoding::Q3);
    write_gamestate(
        &mut writer,
        commands,
        0,
        CONTEXT.checksum_feed,
        [(1, &b"\\sv_serverid\\13"[..]), (2, text)],
        [],
    )
    .unwrap();
    let bits = writer.bit_position();
    let length = writer.size();
    bytes.truncate(length);
    (bytes, commands, bits)
}

fn prepare_client_message(
    client: &mut Channel,
    acknowledgements: Option<packet::Acknowledgements>,
) -> Vec<u8> {
    let mut key = client.command_key(None).unwrap();
    if let Some(ack) = acknowledgements {
        key.acknowledgements = ack;
        key.server_command = client.server_command(ack.reliable as u32).unwrap_or(b"");
    }
    let mut commands = [packet::ZERO_Q3; packet::Q3_COMMANDS];
    commands[0].server_time = client.send_state().sequence as i32;
    let mut bytes = vec![0; MAXIMUM];
    let length = packet::write_with_commands(
        &mut bytes,
        &packet::Move::Quake3 {
            commands,
            count: 1,
            delta: false,
        },
        client.send_state().sequence,
        key,
        |writer| client.write_command_records(writer),
    )
    .unwrap();
    let packet = client
        .prepare_move(&bytes[..length], EventTime(1), None, None)
        .unwrap()
        .unwrap()
        .bytes
        .to_vec();
    client.submitted(EventTime(1)).unwrap();
    packet
}

fn accept_client_message(server: &mut Channel, packet: &[u8]) -> Result<(), packet::Error> {
    let received = server.receive(packet, EventTime(1)).unwrap();
    let sequence = received.header.sequence;
    let Delivery::Payload(payload) = received.delivery else {
        panic!("complete CLIENT message")
    };
    let mut codec = Commands::load(Protocol::Quake3_68, MAXIMUM);
    let length = codec.stage(payload).unwrap();
    codec.decode(length, sequence, 0, server, None, |_| {
        ControlFlow::Continue(())
    })?;
    Ok(())
}

fn client_message(
    client: &mut Channel,
    server: &mut Channel,
    acknowledgements: Option<packet::Acknowledgements>,
) -> Result<(), packet::Error> {
    accept_client_message(server, &prepare_client_message(client, acknowledgements))
}

fn ack(message: i32, reliable: i32) -> packet::Acknowledgements {
    packet::Acknowledgements {
        server_id: CONTEXT.server_id,
        message,
        reliable,
    }
}

fn submit_output(
    server: &mut Channel,
    client: &mut Channel,
    deliver: bool,
    mut consume: impl FnMut(ServerOutput<'_>),
) -> u32 {
    let sequence = server.send_state().sequence;
    if server.pending_packet().is_none() {
        server.prepare_output(EventTime(1)).unwrap().unwrap();
    }
    loop {
        let bytes = server.pending_packet().unwrap().bytes.to_vec();
        assert!(
            server
                .submit_with(EventTime(1), |_| true)
                .unwrap()
                .is_some()
        );
        if deliver {
            let received = client.receive(&bytes, EventTime(1)).unwrap();
            let sequence = received.header.sequence;
            if let Delivery::Payload(body) = received.delivery {
                let mut body = body.to_vec();
                client
                    .decode_server_output(&mut body, sequence, &mut consume)
                    .unwrap();
            }
        }
        if !server.pending_fragments() {
            return sequence;
        }
        server.prepare_output(EventTime(1)).unwrap().unwrap();
    }
}

#[test]
fn gamestate_receipt_requires_native_message_ack_and_preserves_later_commands() {
    let (mut client, mut server) = pair();
    let first = server.queue_reliable(b"print first").unwrap();
    let (body, watermark, encoded) = encode_state(&mut server, b"captured state");
    let state = server
        .queue_server_state(&body, encoded, watermark)
        .unwrap();
    let later = server.queue_reliable(b"print later").unwrap();
    assert_eq!(
        server.queue_server_state(&body, encoded, watermark),
        Err(TransmitError::Full)
    );
    let mut commands = 0;
    let mut gamestates = 0;
    let sequence = submit_output(&mut server, &mut client, true, |output| match output {
        ServerOutput::ReliableCommand { sequence, text } => {
            assert_eq!(sequence, 1);
            assert_eq!(text, b"print first");
            commands += 1;
        }
        ServerOutput::Download(_) => panic!("unexpected download"),
        ServerOutput::Gamestate(state) => {
            assert_eq!(state.server_command_sequence, 1);
            assert_eq!(state.client_num, 0);
            gamestates += 1;
        }
    });
    assert_eq!((commands, gamestates), (1, 1));
    assert_eq!(server.command_state().unwrap().submitted, 1);
    assert!(server.reliable_receipts().is_empty());
    server.acknowledge_commands(1, sequence).unwrap();
    assert_eq!(server.reliable_receipts(), &[first]);
    assert!(server.has_output());
    client_message(&mut client, &mut server, None).unwrap();
    assert_eq!(server.reliable_receipts(), &[state]);
    let mut received_later = false;
    submit_output(&mut server, &mut client, true, |output| {
        let ServerOutput::ReliableCommand { sequence, text } = output else {
            panic!("the acknowledged gamestate is no longer emitted")
        };
        assert_eq!((sequence, text), (2, &b"print later"[..]));
        received_later = true;
    });
    assert!(received_later);
    client_message(&mut client, &mut server, None).unwrap();
    assert_eq!(server.reliable_receipts(), &[later]);
    assert!(!server.has_output());
}

#[test]
fn one_native_ack_retires_all_sixty_four_commands_and_one_state_receipt() {
    let (mut client, mut server) = pair();
    for ordinal in 1..=64 {
        assert_eq!(server.queue_reliable(b"a"), Ok(NativeReceipt(ordinal)));
    }
    let (bytes, watermark, encoded) = encode_state(&mut server, b"full reliable window");
    assert_eq!(watermark, 64);
    assert_eq!(
        server.queue_server_state(&bytes, encoded, watermark),
        Ok(NativeReceipt(65))
    );
    submit_output(&mut server, &mut client, true, |_| {});
    assert_eq!(server.command_state().unwrap().submitted, 64);
    client_message(&mut client, &mut server, None).unwrap();
    assert_eq!(server.reliable_receipts().len(), 65);
    for (index, receipt) in server.reliable_receipts().iter().enumerate() {
        assert_eq!(*receipt, NativeReceipt(index as u64 + 1));
    }
    assert!(!server.has_output());
    client_message(&mut client, &mut server, None).unwrap();
    assert!(server.reliable_receipts().is_empty());
}

#[test]
fn blocked_partial_and_lost_messages_retain_state_and_rekey_the_same_body() {
    let (mut client, mut server) = pair();
    // A real CLIENT reliable string participates in the captured SERVER XOR key.
    client.queue_reliable(b"userinfo name%native").unwrap();
    client_message(&mut client, &mut server, None).unwrap();
    let text = vec![b'z'; 7000];
    let (bytes, watermark, encoded) = encode_state(&mut server, &text);
    let receipt = server
        .queue_server_state(&bytes, encoded, watermark)
        .unwrap();
    let first = server
        .prepare_output(EventTime(2))
        .unwrap()
        .unwrap()
        .bytes
        .to_vec();
    assert_eq!(server.submit_with(EventTime(2), |_| false), Ok(None));
    assert_eq!(server.pending_packet().unwrap().bytes, first);
    assert_eq!(server.send_state().sequence, 1);
    assert_eq!(
        client_message(&mut client, &mut server, Some(ack(1, 0))),
        Err(packet::Error::Context)
    );
    assert!(server.reliable_receipts().is_empty());
    assert_eq!(
        client_message(&mut client, &mut server, Some(ack(-1, 0))),
        Err(packet::Error::Context)
    );
    server.submitted(EventTime(2)).unwrap();
    assert!(server.pending_fragments());
    assert_eq!(
        client.receive(&first, EventTime(2)).unwrap().delivery,
        Delivery::Pending
    );
    client_message(&mut client, &mut server, Some(ack(0, 0))).unwrap();
    assert!(server.reliable_receipts().is_empty());
    assert_eq!(
        client_message(&mut client, &mut server, Some(ack(1, 0))),
        Err(packet::Error::Context)
    );
    // Finish the first native message, losing the remaining fragments in transit.
    server.prepare_output(EventTime(2)).unwrap().unwrap();
    submit_output(&mut server, &mut client, false, |_| {});
    assert_eq!(server.send_state().sequence, 2);
    client_message(&mut client, &mut server, Some(ack(0, 0))).unwrap();
    assert!(server.reliable_receipts().is_empty());
    server.acknowledge_commands(0, 1).unwrap();
    assert!(server.has_output());
    let mut received = false;
    let retried = submit_output(&mut server, &mut client, true, |output| {
        let ServerOutput::Gamestate(state) = output else {
            panic!("retained state")
        };
        state
            .for_each_config(|row| {
                if row.index == 2 {
                    assert_eq!(row.text, text);
                    received = true;
                }
            })
            .unwrap();
    });
    assert_eq!(retried, 2);
    assert!(received);
    assert_eq!(server.send_state().resends, 1);
    client_message(&mut client, &mut server, None).unwrap();
    assert_eq!(server.reliable_receipts(), &[receipt]);
    assert!(!server.has_output());
}

#[test]
fn old_state_ack_during_retry_drains_its_existing_fragments_before_next_state() {
    let (mut client, mut server) = pair();
    let (bytes, watermark, encoded) = encode_state(&mut server, &[b'x'; 7000]);
    let first = server
        .queue_server_state(&bytes, encoded, watermark)
        .unwrap();
    submit_output(&mut server, &mut client, true, |_| {});
    server.prepare_output(EventTime(2)).unwrap().unwrap();
    server.submitted(EventTime(2)).unwrap();
    assert!(server.pending_fragments());
    client_message(&mut client, &mut server, None).unwrap();
    assert_eq!(server.reliable_receipts(), &[first]);
    let (next, watermark, encoded) = encode_state(&mut server, b"next");
    assert_eq!(
        server.queue_server_state(&next, encoded, watermark),
        Err(TransmitError::PendingPacket)
    );
    server.prepare_output(EventTime(2)).unwrap().unwrap();
    submit_output(&mut server, &mut client, false, |_| {});
    let (next, watermark, encoded) = encode_state(&mut server, b"next");
    let second = server
        .queue_server_state(&next, encoded, watermark)
        .unwrap();
    assert_ne!(first, second);
    submit_output(&mut server, &mut client, true, |_| {});
    client_message(&mut client, &mut server, None).unwrap();
    assert_eq!(server.reliable_receipts(), &[second]);
}

#[test]
fn producer_sequence_survives_an_earlier_message_before_ring_admission() {
    let (mut client, mut server) = pair();
    let (state, watermark, encoded) = encode_state(&mut server, b"queued producer body");
    assert_eq!(server.send_state().sequence, 1);
    let mut earlier = vec![0; MAXIMUM];
    let length = server
        .encode_server_output(&mut earlier, |writer, _| {
            writer.write_bits(1, 8)?; // svc_nop from an earlier output record
            Ok(())
        })
        .unwrap();
    server
        .prepare_server_message(&earlier[..length], EventTime(0))
        .unwrap()
        .unwrap();
    submit_output(&mut server, &mut client, true, |_| {
        panic!("nop has no callback")
    });
    assert_eq!(server.send_state().sequence, 2);
    let receipt = server
        .queue_server_state(&state, encoded, watermark)
        .unwrap();
    let mut accepted = false;
    assert_eq!(
        submit_output(&mut server, &mut client, true, |output| {
            let ServerOutput::Gamestate(state) = output else {
                panic!("state")
            };
            state
                .for_each_config(|row| {
                    if row.index == 2 {
                        assert_eq!(row.text, b"queued producer body");
                        accepted = true;
                    }
                })
                .unwrap();
        }),
        2
    );
    assert!(accepted);
    client_message(&mut client, &mut server, None).unwrap();
    assert_eq!(server.reliable_receipts(), &[receipt]);
}

#[test]
fn retained_body_rebuilds_current_ack_and_xor_after_client_command_slot_reuse() {
    let (mut client, mut server) = pair();
    client.queue_reliable(b"old-key").unwrap();
    client_message(&mut client, &mut server, None).unwrap();
    let (body, watermark, bits) = encode_state(&server, b"retained % bytes");
    let receipt = server.queue_server_state(&body, bits, watermark).unwrap();
    for _ in 2..=64 {
        client.queue_reliable(b"new-key").unwrap();
    }
    // This native CLIENT message was emitted before receiving the gamestate;
    // it carries message ACK0 even when its delayed delivery is later admitted.
    let delayed = prepare_client_message(&mut client, None);
    submit_output(&mut server, &mut client, true, |_| {});
    assert_eq!(client.command_state().unwrap().acknowledged, 1);
    client.queue_reliable(b"overwrites-old-key-slot").unwrap();
    assert_eq!(client.command_state().unwrap().queued, 65);
    accept_client_message(&mut server, &delayed).unwrap();
    assert_eq!(server.received_command(), Some((64, &b"new-key"[..])));
    assert!(server.reliable_receipts().is_empty());
    let mut retried = false;
    submit_output(&mut server, &mut client, true, |output| {
        let ServerOutput::Gamestate(state) = output else {
            panic!("retained body")
        };
        state
            .for_each_config(|row| {
                if row.index == 2 {
                    assert_eq!(row.text, b"retained . bytes");
                    retried = true;
                }
            })
            .unwrap();
    });
    assert!(retried);
    assert_eq!(client.command_state().unwrap().acknowledged, 64);
    client_message(&mut client, &mut server, None).unwrap();
    assert_eq!(server.reliable_receipts(), &[receipt]);
    assert_eq!(
        server.received_command(),
        Some((65, &b"overwrites-old-key-slot"[..]))
    );
}

#[test]
fn checked_encoded_append_preserves_bit_alignment_and_native_percent_wire_bytes() {
    let mut body = [0; 64];
    let mut standalone = Writer::new(&mut body, Encoding::Q3);
    standalone.write_bits(5, 3).unwrap();
    standalone
        .write_string(StringPolicy::Q3_BIG, b"percent%\x80")
        .unwrap();
    let bits = standalone.bit_position();
    let size = standalone.size();
    let mut bytes = [0; 128];
    let mut writer = Writer::new(&mut bytes, Encoding::Q3);
    writer.write_bits(11, 5).unwrap();
    writer.append_encoded_q3(&body[..size], bits).unwrap();
    writer.write_bits(0x8123_4567, 32).unwrap();
    let size = writer.size();
    let mut reader = Reader::new(&bytes[..size], Encoding::Q3);
    assert_eq!(reader.read_bits(5).unwrap(), 11);
    assert_eq!(reader.read_bits(3).unwrap(), 5);
    for &byte in b"percent%.\0" {
        assert_eq!(reader.read_bits(8).unwrap(), u32::from(byte));
    }
    assert_eq!(reader.read_bits(32).unwrap(), 0x8123_4567);
    let mut short = [0xa5; 1];
    let mut writer = Writer::new(&mut short, Encoding::Q3);
    assert!(
        writer
            .append_encoded_q3(&body[..size.min(body.len())], bits)
            .is_err()
    );
    assert_eq!(writer.bit_position(), 0);
    let mut bytes = [0xa5; 128];
    let mut writer = Writer::new(&mut bytes, Encoding::Q3);
    assert!(writer.append_encoded_q3(&body[..1], 9).is_err());
    assert_eq!(writer.bit_position(), 0);
    let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
    assert!(writer.append_encoded_q3(&body, bits).is_err());
    assert_eq!(writer.bit_position(), 0);
}

#[test]
fn stalled_peer_does_not_block_independent_output_and_state_does_not_claim_snapshot_provenance() {
    let (stalled_client, mut stalled_server) = pair();
    let (bytes, watermark, encoded) = encode_state(&mut stalled_server, b"stalled");
    stalled_server
        .queue_server_state(&bytes, encoded, watermark)
        .unwrap();
    stalled_server
        .prepare_output(EventTime(0))
        .unwrap()
        .unwrap();
    assert_eq!(
        stalled_server.submit_with(EventTime(0), |_| false),
        Ok(None)
    );
    let (mut client, mut server) = pair();
    let (bytes, watermark, encoded) = encode_state(&mut server, b"healthy");
    let receipt = server
        .queue_server_state(&bytes, encoded, watermark)
        .unwrap();
    submit_output(&mut server, &mut client, true, |_| {});
    client_message(&mut client, &mut server, None).unwrap();
    assert_eq!(server.reliable_receipts(), &[receipt]);
    assert!(stalled_server.pending_packet().is_some());
    server.configure_snapshots(Protocol::Quake3_68).unwrap();
    let sequence = server.send_state().sequence;
    let player = [0; states::PLAYER_WORDS];
    server
        .publish_snapshot(ReceivedFrame::Quake3(Frame {
            sequence,
            time: ThinkTime::Milliseconds(700),
            command: 0,
            flags: 0,
            qw_peers: 0,
            qw_dead: 0,
            areas: &[],
            player: &player,
            entities: &[],
        }))
        .unwrap();
    let (bytes, watermark, encoded) = encode_state(&mut server, b"second state");
    let state = server
        .queue_server_state(&bytes, encoded, watermark)
        .unwrap();
    submit_output(&mut server, &mut client, true, |_| {});
    assert!(server.snapshot_observation(sequence).is_none());
    client_message(&mut client, &mut server, None).unwrap();
    assert_eq!(server.reliable_receipts(), &[state]);
    // Reconstructing a connection releases its channel-owned retained message.
    drop(stalled_server);
    let mut fresh = Channel::load(channel::QUAKE3, Endpoint::Server, MAXIMUM, 8).unwrap();
    fresh.set_command_context(CONTEXT).unwrap();
    let (bytes, watermark, encoded) = encode_state(&mut fresh, b"fresh lifetime");
    assert_eq!(
        fresh.queue_server_state(&bytes, encoded, watermark),
        Ok(NativeReceipt(1))
    );
    assert!(stalled_client.q3_startup().is_none());
}

#[test]
fn server_state_admission_is_scoped_bounded_and_preserves_failed_admission() {
    let (mut client, mut server) = pair();
    let mut q2 = Channel::load(channel::QUAKE2, Endpoint::Server, MAXIMUM, 8).unwrap();
    assert_eq!(
        client.queue_server_state(b"body", 31, 0),
        Err(TransmitError::ServerContext)
    );
    assert_eq!(
        q2.queue_server_state(b"body", 31, 0),
        Err(TransmitError::ServerContext)
    );
    assert_eq!(
        server.queue_server_state(&[], 0, 0),
        Err(TransmitError::MessageTooLarge)
    );
    assert_eq!(
        server.queue_server_state(&vec![0; MAXIMUM + 1], MAXIMUM * 8 + 7, 0),
        Err(TransmitError::MessageTooLarge)
    );
    let (bytes, watermark, encoded) = encode_state(&mut server, b"bounded");
    assert_eq!(
        server.queue_server_state(&bytes, 0, watermark),
        Err(TransmitError::MessageTooLarge)
    );
    assert_eq!(
        server.queue_server_state(&bytes, encoded + 1, watermark),
        Err(TransmitError::MessageTooLarge)
    );
    assert_eq!(
        server.queue_server_state(&bytes, encoded, watermark + 1),
        Err(TransmitError::MessageTooLarge)
    );
    assert_eq!(
        server.queue_server_state(&bytes, encoded, watermark),
        Ok(NativeReceipt(1))
    );
    assert_eq!(server.send_state().reliable_bytes, bytes.len());
}

#[test]
fn sole_gamestate_writer_uses_native_strings_and_forced_null_baselines() {
    let mut configs = ConfigStrings::load([64; 4]).unwrap();
    configs.set(1, b"native%\x80").unwrap();
    let mut words = [0; states::ENTITY_WORDS];
    words[29] = 7;
    words[5] = 3.25f32.to_bits();
    let mut bytes = vec![0; MAXIMUM];
    let mut writer = Writer::new(&mut bytes, Encoding::Q3);
    write_gamestate(
        &mut writer,
        17,
        63,
        0x8123_4567,
        (0..configs.len()).map(|index| (index, configs.config(index).unwrap().text)),
        [Entity { number: 0, words }, Entity { number: 7, words }],
    )
    .unwrap();
    let length = writer.size();
    let mut reader = Reader::new(&bytes[..length], Encoding::Q3);
    assert_eq!(reader.read_bits(8).unwrap(), 2);
    assert_eq!(reader.read_bits(32).unwrap(), 17);
    assert_eq!(reader.read_bits(8).unwrap(), 3);
    assert_eq!(reader.read_signed(16).unwrap(), 1);
    let mut text = [0; 8192];
    assert_eq!(
        reader
            .read_string_into(StringPolicy::Q3_BIG, &mut text)
            .unwrap(),
        b"native.."
    );
    assert_eq!(reader.read_bits(8).unwrap(), 4);
    let baseline = states::read_q3_entity(&mut reader, &[0; states::ENTITY_WORDS]).unwrap();
    assert_eq!(baseline.number, 7);
    assert_eq!(baseline.words, Some(words));
    assert_eq!(reader.read_bits(8).unwrap(), 8);
    assert_eq!(reader.read_bits(32).unwrap(), 63);
    assert_eq!(reader.read_bits(32).unwrap(), 0x8123_4567);
}

#[test]
fn gamestate_writer_validates_all_rows_and_baselines_before_changing_output() {
    let zero = [0; states::ENTITY_WORDS];
    for (rows, baselines, command, client) in [
        (vec![(1024, &b"x"[..])], vec![], 0, 0),
        (vec![(2, &b"x"[..]), (1, &b"y"[..])], vec![], 0, 0),
        (vec![(1, &b"x"[..]), (1, &b"y"[..])], vec![], 0, 0),
        (vec![(1, &b"x\0y"[..])], vec![], 0, 0),
        (
            vec![(1, &b"x"[..])],
            vec![Entity {
                number: 1024,
                words: zero,
            }],
            0,
            0,
        ),
        (vec![], vec![], 0x8000_0000, 0),
        (vec![], vec![], 0, 64),
    ] {
        let mut bytes = vec![0xa5; MAXIMUM];
        let mut writer = Writer::new(&mut bytes, Encoding::Q3);
        writer.write_bits(99, 8).unwrap();
        let before = writer.bytes().to_vec();
        let bit = writer.bit_position();
        assert!(write_gamestate(&mut writer, command, client, 0, rows, baselines).is_err());
        assert_eq!(writer.bit_position(), bit);
        assert_eq!(writer.bytes(), before);
    }
    let first = [b'a'; 8191];
    let second = [b'b'; Q3_GAME_STATE_DATA_BYTES - 1 - 8192 - 1];
    // The native character limit is independent of the Huffman wire size.
    let mut bytes = vec![0; qa_core::primitives::NATIVE_WIRE_BYTES];
    let mut writer = Writer::new(&mut bytes, Encoding::Q3);
    write_gamestate(
        &mut writer,
        0,
        0,
        0,
        [(1, &first[..]), (2, &second[..])],
        [],
    )
    .unwrap();
    let oversized = [b'b'; Q3_GAME_STATE_DATA_BYTES - 1 - 8192];
    let mut bytes = vec![0xa5; MAXIMUM];
    let mut writer = Writer::new(&mut bytes, Encoding::Q3);
    assert_eq!(
        write_gamestate(
            &mut writer,
            0,
            0,
            0,
            [(1, &first[..]), (2, &oversized[..])],
            []
        ),
        Err(packet::Error::Count)
    );
    assert_eq!(writer.bit_position(), 0);
    let too_long = [b'x'; 8192];
    assert_eq!(
        write_gamestate(&mut writer, 0, 0, 0, [(1, &too_long[..])], []),
        Err(packet::Error::Count)
    );
    assert_eq!(writer.bit_position(), 0);
}
