use qa_core::{
    configstrings::{Q3_GAME_STATE_DATA_BYTES, Q3_GAME_STATE_ROWS},
    loopback::Endpoint,
    primitives::ThinkTime,
    sys_events::EventTime,
};
use qa_network::{
    channel::{
        Channel, Delivery,
        commands::{CommandContext, ServerOutput},
    },
    commands::{
        connection::Commands,
        packet::{self, Protocol},
    },
    ingress::Incoming,
    message::Writer,
    snapshots::{Entity, Frame, ReceivedFrame},
    states,
};

const MESSAGE_BYTES: usize = 32768;

fn pair() -> (Channel, Channel) {
    let context = CommandContext {
        server_id: 13,
        challenge: 0x8765_abcd,
        checksum_feed: 0x1234_7654,
    };
    let mut client = Channel::load(
        Protocol::Quake3_68.channel(),
        Endpoint::Client,
        MESSAGE_BYTES,
        8,
    )
    .unwrap();
    let mut server = Channel::load(
        Protocol::Quake3_68.channel(),
        Endpoint::Server,
        MESSAGE_BYTES,
        8,
    )
    .unwrap();
    for channel in [&mut client, &mut server] {
        channel.set_command_context(context).unwrap();
        channel.configure_snapshots(Protocol::Quake3_68).unwrap();
    }
    (client, server)
}

fn encode(
    server: &mut Channel,
    body: impl FnOnce(&mut Writer<'_>) -> Result<(), packet::Error>,
) -> Vec<u8> {
    let mut bytes = vec![0; MESSAGE_BYTES];
    let n = server
        .encode_server_output(&mut bytes, |writer, _| body(writer))
        .unwrap();
    bytes.truncate(n);
    bytes
}

fn deliver(
    server: &mut Channel,
    client: &mut Channel,
    bytes: &[u8],
    mut consume: impl FnMut(ServerOutput<'_>),
) -> Result<(), packet::Error> {
    let submitted = server.command_state().unwrap().submitted;
    server
        .prepare_server_message(bytes, EventTime(0))
        .unwrap()
        .unwrap();
    loop {
        let packet = server.pending_packet().unwrap().bytes.to_vec();
        server.submitted(EventTime(0)).unwrap();
        if server.pending_fragments() {
            assert_eq!(server.command_state().unwrap().submitted, submitted);
        }
        let received = client.receive(&packet, EventTime(0)).unwrap();
        let sequence = received.header.sequence;
        if let Delivery::Payload(body) = received.delivery {
            let mut body = body.to_vec();
            client.decode_server_output(&mut body, sequence, &mut consume)?;
        }
        if !server.pending_fragments() {
            return Ok(());
        }
        server.prepare_output(EventTime(0)).unwrap().unwrap();
    }
}

fn config(writer: &mut Writer<'_>, index: i32, text: &[u8]) -> Result<(), packet::Error> {
    writer.write_bits(3, 8)?;
    writer.write_bits(index as u32, 16)?;
    for &byte in text {
        writer.write_bits(u32::from(byte), 8)?;
    }
    writer.write_bits(0, 8)?;
    Ok(())
}

fn gamestate(
    writer: &mut Writer<'_>,
    watermark: u32,
    client: i32,
    checksum: u32,
    records: impl FnOnce(&mut Writer<'_>) -> Result<(), packet::Error>,
) -> Result<(), packet::Error> {
    writer.write_bits(2, 8)?;
    writer.write_bits(watermark, 32)?;
    records(writer)?;
    writer.write_bits(8, 8)?;
    writer.write_bits(client as u32, 32)?;
    writer.write_bits(checksum, 32)?;
    Ok(())
}

fn baseline(
    writer: &mut Writer<'_>,
    number: u32,
    words: &[u32; states::ENTITY_WORDS],
) -> Result<(), packet::Error> {
    writer.write_bits(4, 8)?;
    assert!(states::write_q3_entity(
        writer,
        number,
        &[0; states::ENTITY_WORDS],
        Some(words),
        true
    )?);
    Ok(())
}

fn model_words(model: u32) -> [u32; states::ENTITY_WORDS] {
    let mut words = [0; states::ENTITY_WORDS];
    words[29] = model;
    words[5] = 3.25f32.to_bits();
    words
}

fn received_entity(client: &Channel, sequence: u32) -> [u32; states::ENTITY_WORDS] {
    let ReceivedFrame::Quake3(frame) = client.snapshot(sequence).unwrap() else {
        panic!("Q3 native snapshot");
    };
    frame.entities[0].words
}

fn snapshot(
    server: &mut Channel,
    client: &mut Channel,
    words: [u32; states::ENTITY_WORDS],
    time: i64,
) {
    let sequence = server.send_state().sequence;
    let player = [0; states::PLAYER_WORDS];
    let entities = [Entity { number: 7, words }];
    server
        .publish_snapshot(ReceivedFrame::Quake3(Frame {
            sequence,
            time: ThinkTime::Milliseconds(time),
            command: 0,
            flags: 0,
            qw_peers: 0,
            qw_dead: 0,
            areas: &[],
            player: &player,
            entities: &entities,
        }))
        .unwrap();
    let mut bytes = vec![0; MESSAGE_BYTES];
    let n = server
        .encode_server_output(&mut bytes, |writer, channel| {
            channel.write_snapshot(writer, sequence, None, 0)
        })
        .unwrap();
    deliver(server, client, &bytes[..n], |_| {}).unwrap();
}

#[test]
fn native_gamestate_preserves_reserved_client_zero_bytes_and_header_context() {
    for client_num in [0, 63] {
        let (mut client, mut server) = pair();
        assert!(client.q3_startup().is_none());
        let challenge = client.command_context().unwrap().challenge;
        let bytes = encode(&mut server, |writer| {
            gamestate(writer, 0, client_num, 0xfedc_ba98, |writer| {
                config(writer, 0, b"\\mapname\\q3dm1")?;
                config(writer, 1, b"\\SV_SERVERID\\42")?;
                config(writer, 20, b"old")?;
                config(writer, 20, b"raw\x80\xff%")?;
                config(writer, 21, b"")
            })
        });
        let mut rows = Vec::new();
        let mut callbacks = 0;
        deliver(&mut server, &mut client, &bytes, |output| {
            let ServerOutput::Gamestate(gamestate) = output else {
                panic!("gamestate record");
            };
            assert_eq!(gamestate.client_num, client_num as u32);
            gamestate
                .for_each_config(|row| rows.push((row.index, row.text.to_vec())))
                .unwrap();
            callbacks += 1;
        })
        .unwrap();
        assert_eq!(callbacks, 1);
        assert_eq!(
            rows,
            [
                (0, b"\\mapname\\q3dm1".to_vec()),
                (1, b"\\SV_SERVERID\\42".to_vec()),
                (20, b"old".to_vec()),
                (20, b"raw\x80\xff.".to_vec()),
                (21, Vec::new())
            ]
        );
        let startup = client.q3_startup().unwrap();
        assert_eq!(startup.message_sequence, 1);
        assert_eq!(startup.client_num, client_num as u32);
        assert_eq!(startup.checksum_feed, 0xfedc_ba98);
        let context = client.command_context().unwrap();
        assert_eq!(context.server_id, 42);
        assert_eq!(context.checksum_feed, 0xfedc_ba98);
        assert_eq!(context.challenge, challenge);
        assert_eq!(
            client.q3_snapshot_number(),
            Some((0, ThinkTime::Milliseconds(0)))
        );
    }
}

#[test]
fn gamestate_retains_unexecuted_native_commands_and_does_not_jump_execution() {
    let (mut client, mut server) = pair();
    server.queue_reliable(b"cp \"before gamestate\"").unwrap();
    server.queue_reliable(b"print \"still pending\"").unwrap();
    let bytes = encode(&mut server, |writer| {
        gamestate(writer, 2, 0, 123, |_| Ok(()))
    });
    let mut commands = Vec::new();
    deliver(&mut server, &mut client, &bytes, |output| {
        if let ServerOutput::ReliableCommand { sequence, text } = output {
            commands.push((sequence, text.to_vec()));
        }
    })
    .unwrap();
    assert_eq!(commands.len(), 2);
    assert_eq!(server.command_state().unwrap().submitted, 2);
    assert_eq!(
        client.server_command(1).unwrap(),
        b"cp \"before gamestate\""
    );
    assert_eq!(
        client.server_command(2).unwrap(),
        b"print \"still pending\""
    );
    assert_eq!(client.last_executed_server_command().unwrap(), 0);
    client.executed_server_command(1).unwrap();
    let bytes = encode(&mut server, |writer| {
        gamestate(writer, 2, 0, 456, |_| Ok(()))
    });
    deliver(&mut server, &mut client, &bytes, |_| {}).unwrap();
    let startup = client.q3_startup().unwrap();
    assert_eq!(startup.server_command_sequence, 2);
    assert_eq!(startup.last_executed_server_command, 1);
    assert_eq!(
        client.server_command(2).unwrap(),
        b"print \"still pending\""
    );
    assert_eq!(client.command_context().unwrap().server_id, 0); // Absent systeminfo clears the old server ID.
}

#[test]
fn native_baseline_is_used_by_snapshot_and_replaced_without_a_second_ring() {
    let (mut client, mut server) = pair();
    let old = model_words(11);
    assert!(server.set_snapshot_baseline(7, &old));
    let bytes = encode(&mut server, |writer| {
        gamestate(writer, 0, 0, 1, |writer| baseline(writer, 7, &old))
    });
    deliver(&mut server, &mut client, &bytes, |_| {}).unwrap();
    snapshot(&mut server, &mut client, old, 500);
    let sequence = client.q3_snapshot_number().unwrap().0;
    assert_eq!(received_entity(&client, sequence), old);
    let new = model_words(22);
    let bytes = encode(&mut server, |writer| {
        gamestate(writer, 0, 0, 2, |writer| baseline(writer, 7, &new))
    });
    deliver(&mut server, &mut client, &bytes, |_| {}).unwrap();
    assert!(client.snapshot(sequence).is_none());
    assert_eq!(
        client.q3_snapshot_number(),
        Some((0, ThinkTime::Milliseconds(0)))
    );
    // The server sends an unchanged delta against its old baseline: the receiver
    // must now use the newly admitted native baseline, proving baseline replacement.
    snapshot(&mut server, &mut client, old, 700);
    let sequence = client.q3_snapshot_number().unwrap().0;
    assert_eq!(received_entity(&client, sequence), new);
}

#[test]
fn malformed_bodies_leave_previous_rows_baseline_snapshot_and_startup_untouched() {
    let (mut client, mut server) = pair();
    let old = model_words(11);
    assert!(server.set_snapshot_baseline(7, &old));
    let bytes = encode(&mut server, |writer| {
        gamestate(writer, 0, 0, 1, |writer| {
            config(writer, 20, b"accepted")?;
            baseline(writer, 7, &old)
        })
    });
    let mut rows = Vec::new();
    deliver(&mut server, &mut client, &bytes, |output| {
        if let ServerOutput::Gamestate(gs) = output {
            gs.for_each_config(|row| rows.push(row.text.to_vec()))
                .unwrap();
        }
    })
    .unwrap();
    snapshot(&mut server, &mut client, old, 500);
    let startup = client.q3_startup().unwrap();
    let sequence = client.q3_snapshot_number().unwrap().0;
    let context = client.command_context().unwrap();
    for poison in 0..7 {
        let bytes = encode(&mut server, |writer| {
            match poison {
                0 => gamestate(writer, 0, 0, 2, |writer| config(writer, -1, b"bad")),
                1 => gamestate(writer, 0, 0, 2, |writer| {
                    config(writer, Q3_GAME_STATE_ROWS as i32, b"bad")
                }),
                2 => gamestate(writer, 0, 64, 2, |writer| {
                    config(writer, 20, b"replacement")?;
                    baseline(writer, 7, &model_words(22))
                }),
                3 => gamestate(writer, 0, -1, 2, |_| Ok(())),
                4 => gamestate(writer, 0, 0, 2, |writer| {
                    writer.write_bits(4, 8)?;
                    writer.write_bits(7, 10)?;
                    writer.write_bits(0, 1)?;
                    writer.write_bits(1, 1)?;
                    writer.write_bits(255, 8)?;
                    Ok(()) // Invalid entity field count.
                }),
                5 => gamestate(writer, 0, 0, 2, |writer| {
                    for _ in 0..4 {
                        config(writer, 20, &[b'a'; 4000])?;
                    }
                    Ok(()) // Duplicate replacements still exceed native dataCount.
                }),
                _ => {
                    writer.write_bits(2, 8)?;
                    writer.write_bits(0, 32)?;
                    writer.write_bits(8, 8)?;
                    writer.write_bits(0, 32)?;
                    Ok(()) // Missing checksumFeed before the outer EOF.
                }
            }
        });
        let mut callbacks = 0;
        assert!(deliver(&mut server, &mut client, &bytes, |_| callbacks += 1).is_err());
        assert_eq!(callbacks, 0);
        assert_eq!(rows, [b"accepted".to_vec()]);
        assert_eq!(client.q3_startup(), Some(startup));
        assert_eq!(client.command_context().unwrap(), context);
        assert_eq!(received_entity(&client, sequence), old);
    }
    snapshot(&mut server, &mut client, old, 900);
    let sequence = client.q3_snapshot_number().unwrap().0;
    assert_eq!(received_entity(&client, sequence), old);
}

#[test]
fn replacement_body_with_bad_outer_tail_revokes_old_readiness() {
    let (mut client, mut server) = pair();
    let bytes = encode(&mut server, |writer| gamestate(writer, 0, 0, 1, |_| Ok(())));
    deliver(&mut server, &mut client, &bytes, |_| {}).unwrap();
    assert!(client.q3_startup().is_some());
    let bytes = encode(&mut server, |writer| {
        gamestate(writer, 0, 0, 2, |writer| {
            config(writer, 20, b"body accepted")
        })?;
        writer.write_bits(6, 8)?; // Unsupported trailing download; no outer commit.
        Ok(())
    });
    let mut rows = Vec::new();
    assert_eq!(
        deliver(&mut server, &mut client, &bytes, |output| {
            if let ServerOutput::Gamestate(gs) = output {
                gs.for_each_config(|row| rows.push(row.text.to_vec()))
                    .unwrap();
            }
        }),
        Err(packet::Error::Opcode)
    );
    assert_eq!(rows, [b"body accepted".to_vec()]);
    assert!(client.q3_startup().is_none());
    let bytes = encode(&mut server, |writer| gamestate(writer, 0, 0, 3, |_| Ok(())));
    deliver(&mut server, &mut client, &bytes, |_| {}).unwrap();
    assert_eq!(client.q3_startup().unwrap().checksum_feed, 3);
}

#[test]
fn aggregate_limit_includes_reserved_zero_and_every_empty_or_duplicate_record() {
    assert_eq!(Q3_GAME_STATE_DATA_BYTES, 16000);
    let (mut client, mut server) = pair();
    let bytes = encode(&mut server, |writer| {
        gamestate(writer, 0, 0, 1, |writer| {
            config(writer, 20, &[b'a'; 7998])?;
            config(writer, 20, &[b'b'; 7998])?;
            config(writer, 21, b"")?; // 1 + 7999 + 7999 + 1 == 16000.
            Ok(())
        })
    });
    deliver(&mut server, &mut client, &bytes, |_| {}).unwrap();
    let startup = client.q3_startup();
    let bytes = encode(&mut server, |writer| {
        gamestate(writer, 0, 0, 2, |writer| {
            config(writer, 20, &[b'a'; 7998])?;
            config(writer, 20, &[b'b'; 7998])?;
            config(writer, 21, b"")?;
            config(writer, 21, b"")?;
            Ok(())
        })
    });
    let mut callbacks = 0;
    assert_eq!(
        deliver(&mut server, &mut client, &bytes, |_| callbacks += 1),
        Err(packet::Error::Count)
    );
    assert_eq!(callbacks, 0);
    assert_eq!(client.q3_startup(), startup);
}

#[test]
fn command_adapter_exposes_only_a_fully_checked_borrowed_gamestate() {
    let (mut client, mut server) = pair();
    let mut codec = Commands::load(Protocol::Quake3_68, MESSAGE_BYTES);
    let bytes = encode(&mut server, |writer| {
        gamestate(writer, 0, 17, 99, |writer| config(writer, 20, b"adapter"))
    });
    let sequence = server.send_state().sequence;
    let length = codec.stage(&bytes).unwrap();
    let mut rows = Vec::new();
    assert_eq!(
        codec
            .decode_output(
                length,
                sequence,
                &mut client,
                qa_core::primitives::ResourceNamespace::Client(qa_core::primitives::ClientId(2)),
                |incoming| {
                    let Incoming::Gamestate(gs) = incoming else {
                        panic!("typed gamestate adapter");
                    };
                    assert_eq!(gs.client_num, 17); // Independent of the common connection index.
                    gs.for_each_config(|row| rows.push((row.index, row.text.to_vec())))
                        .unwrap();
                }
            )
            .unwrap(),
        None
    );
    assert_eq!(rows, [(20, b"adapter".to_vec())]);
    assert_eq!(client.q3_startup().unwrap().client_num, 17);
}
