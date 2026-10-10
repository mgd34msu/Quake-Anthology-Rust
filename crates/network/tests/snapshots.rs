use qa_core::{
    loopback::Endpoint,
    primitives::{ClientId, PlayerState, RuleSetId, ThinkTime},
    sys_events::{EventTime, Peer},
};
use qa_network::{
    channel::{Channel, QUAKE3},
    commands::{
        connection::Commands,
        packet::{Error, Protocol},
    },
    ingress::{Connection, Connections, Incoming, Route},
    message::{Encoding, Reader, Writer},
    projection::{PlayerContext, PlayerProjection},
    snapshots::{self, Entity, Frame, Q3Ring, Ring},
    states::{self, ENTITY_WORDS, PLAYER_WORDS},
};

fn native_player_context() -> PlayerContext {
    PlayerContext {
        client_number: None,
        ground_number: None,
        weapon_number: None,
        weapon_model: None,
        gravity: 0.,
        speed: 0.,
        player_info_flags: 0,
        command_age_ms: 0,
        body_yaw: 0.,
    }
}

fn entity(number: u32, value: f32) -> Entity<ENTITY_WORDS> {
    let mut words = [0; ENTITY_WORDS];
    words[1] = value.to_bits();
    words[29] = number + 1;
    Entity { number, words }
}
fn store(ring: &mut Q3Ring, sequence: u32, entities: &[Entity<ENTITY_WORDS>]) -> Result<(), Error> {
    let mut player = [0; PLAYER_WORDS];
    player[0] = sequence * 50;
    player[1] = (sequence as f32 * 0.5).to_bits();
    player[48] = 100;
    ring.store(Frame {
        sequence,
        time: ThinkTime::Milliseconds(i64::from(sequence as i32 * 50)),
        command: sequence,
        flags: 4,
        areas: &[0xaa, 0x55],
        player: &player,
        entities,
    })
}
fn encode(
    ring: &Q3Ring,
    sequence: u32,
    from: Option<u32>,
    bytes: &mut [u8],
) -> Result<usize, Error> {
    let mut writer = Writer::new(bytes, Encoding::Q3);
    snapshots::write_q3(&mut writer, ring, sequence, from)?;
    writer.write_bits(8, 8)?;
    Ok(writer.size())
}
fn decode(ring: &mut Q3Ring, sequence: u32, bytes: &[u8]) -> Result<bool, Error> {
    let mut reader = Reader::new(bytes, Encoding::Q3);
    assert_eq!(reader.read_bits(8)?, 7);
    let accepted = snapshots::read_q3(&mut reader, ring, sequence, sequence)?;
    assert_eq!(reader.read_bits(8)?, 8);
    Ok(accepted)
}

#[test]
fn full_delta_baselines_insert_remove_and_implicit_unchanged() -> Result<(), Error> {
    let mut server = Q3Ring::load(16, 1024, 32, None)?;
    let mut client = Q3Ring::load(16, 1024, 32, Some(1920))?;
    let baseline = entity(7, 91.25);
    assert!(server.set_baseline(7, &baseline.words));
    assert!(client.set_baseline(7, &baseline.words));
    let old = [entity(1, 1.5), entity(3, -0.0), entity(5, 4.0)];
    store(&mut server, 1, &old)?;
    let mut bytes = [0; 1400];
    let n = encode(&server, 1, None, &mut bytes)?;
    assert!(decode(&mut client, 1, &bytes[..n])?);
    let new = [old[0], entity(3, 4.25), entity(7, 91.25)];
    store(&mut server, 2, &new)?;
    let n = encode(&server, 2, Some(1), &mut bytes)?;
    assert!(decode(&mut client, 2, &bytes[..n])?);
    let Some(frame) = client.frame(2) else {
        return Err(Error::Context);
    };
    assert_eq!(frame.entities, new);
    assert_eq!(frame.areas, [0xaa, 0x55]);
    assert_eq!(frame.player[0], 100);
    assert_eq!(frame.player[48], 100);
    assert_eq!(frame.command, 2);
    assert_eq!(frame.flags, 4);
    assert_eq!(client.counts().accepted, 2);
    Ok(())
}

#[test]
fn missing_delta_is_consumed_and_next_full_frame_recovers() -> Result<(), Error> {
    let mut server = Q3Ring::load(8, 1024, 32, None)?;
    let mut client = Q3Ring::load(8, 1024, 32, Some(1920))?;
    store(&mut server, 1, &[entity(3, 1.)])?;
    store(&mut server, 2, &[entity(3, 2.), entity(4, 3.)])?;
    let mut bytes = [0; 1400];
    let n = encode(&server, 2, Some(1), &mut bytes)?;
    assert!(!decode(&mut client, 2, &bytes[..n])?);
    assert!(client.frame(2).is_none());
    assert_eq!(client.counts().missing_base, 1);
    let n = encode(&server, 2, None, &mut bytes)?;
    assert!(decode(&mut client, 2, &bytes[..n])?);
    assert_eq!(client.frame(2).map(|f| f.entities.len()), Some(2));
    Ok(())
}

#[test]
fn invalid_delta_consumes_retained_slot_rows_and_ages_the_native_window() -> Result<(), Error> {
    let mut server = Q3Ring::load(8, 1024, 32, None)?;
    let mut client = Q3Ring::load(8, 1024, 32, Some(3))?;
    store(&mut client, 2, &[entity(1, 1.), entity(2, 2.)])?;
    store(&mut server, 34, &[entity(1, 1.)])?;
    store(&mut server, 35, &[entity(1, 1.)])?;
    let mut bytes = [0; 1400];
    let n = encode(&server, 35, Some(34), &mut bytes)?;
    assert!(!decode(&mut client, 35, &bytes[..n])?);
    assert!(client.frame(2).is_none());
    assert!(client.frame(35).is_none());
    assert_eq!(client.counts().missing_base, 1);
    Ok(())
}

#[test]
fn overflow_consumes_the_message_without_publishing_a_partial_frame() -> Result<(), Error> {
    let mut server = Q3Ring::load(8, 1024, 32, None)?;
    let mut client = Q3Ring::load(1, 1024, 32, None)?;
    store(&mut server, 1, &[entity(2, 1.), entity(3, 2.)])?;
    let mut bytes = [0; 1400];
    let n = encode(&server, 1, None, &mut bytes)?;
    assert!(!decode(&mut client, 1, &bytes[..n])?);
    assert_eq!(client.counts().overflow, 1);
    assert_eq!(client.counts().accepted, 0);
    Ok(())
}

#[test]
fn generic_ring_wrap_gaps_row_retention_and_connection_isolation() -> Result<(), Error> {
    let mut a = Ring::<2, 1>::load(4, 8, 2, Some(8))?;
    let b = Ring::<2, 1>::load(4, 8, 2, Some(8))?;
    let entities = [
        Entity {
            number: 1,
            words: [1],
        },
        Entity {
            number: 2,
            words: [2],
        },
    ];
    for sequence in [1, 2, 4, 34] {
        a.store(Frame {
            sequence,
            time: ThinkTime::Milliseconds(i64::from(0)),
            command: 0,
            flags: 0,
            areas: &[0],
            player: &[1, 2],
            entities: &entities,
        })?;
    }
    assert!(a.frame(1).is_none());
    assert!(a.frame(2).is_none());
    assert!(a.frame(3).is_none());
    assert!(a.frame(34).is_some());
    assert!(b.frame(34).is_none());
    for sequence in 35..40 {
        a.store(Frame {
            sequence,
            time: ThinkTime::Milliseconds(i64::from(0)),
            command: 0,
            flags: 0,
            areas: &[],
            player: &[0; 2],
            entities: &entities,
        })?;
    }
    assert!(a.frame(34).is_none());
    assert!(a.frame(39).is_some());
    assert!(!a.set_baseline(8, &[0]));
    Ok(())
}

#[test]
fn stale_native_delta_request_falls_back_to_zero_baseline() -> Result<(), Error> {
    let mut server = Q3Ring::load(8, 1024, 32, None)?;
    store(&mut server, 1, &[entity(1, 1.)])?;
    store(&mut server, 30, &[entity(1, 2.)])?;
    let mut bytes = [0; 1400];
    let n = encode(&server, 30, Some(1), &mut bytes)?;
    let mut client = Q3Ring::load(8, 1024, 32, None)?;
    assert!(decode(&mut client, 30, &bytes[..n])?);
    assert_eq!(client.frame(30).map(|f| f.entities[0]), Some(entity(1, 2.)));
    Ok(())
}

#[test]
fn native_snapshot_boundary_omits_reserved_numbers_and_extra_area_bits() -> Result<(), Error> {
    let mut server = Q3Ring::load(8, 2048, 64, None)?;
    let mut client = Q3Ring::load(8, 1024, 32, None)?;
    let rows = [entity(1, 2.), entity(1023, 3.), entity(1500, 4.)];
    server.store(Frame {
        sequence: 1,
        time: ThinkTime::Milliseconds(i64::from(0)),
        command: 0,
        flags: 0,
        areas: &[0x55; 64],
        player: &[0; PLAYER_WORDS],
        entities: &rows,
    })?;
    let mut bytes = [0; 1400];
    let n = encode(&server, 1, None, &mut bytes)?;
    assert!(decode(&mut client, 1, &bytes[..n])?);
    let Some(frame) = client.frame(1) else {
        return Err(Error::Context);
    };
    assert_eq!(frame.entities, [rows[0]]);
    assert_eq!(frame.areas, [0x55; 32]);
    assert!(!server.set_baseline(1, &[0; ENTITY_WORDS]));
    Ok(())
}

#[test]
fn truncated_payload_does_not_replace_an_accepted_snapshot() -> Result<(), Error> {
    let mut server = Q3Ring::load(8, 1024, 32, None)?;
    let mut client = Q3Ring::load(8, 1024, 32, None)?;
    store(&mut server, 1, &[entity(1, 1.)])?;
    let mut bytes = [0; 1400];
    let n = encode(&server, 1, None, &mut bytes)?;
    assert!(decode(&mut client, 1, &bytes[..n])?);
    store(&mut server, 2, &[entity(1, 2.)])?;
    let n = encode(&server, 2, Some(1), &mut bytes)?;
    // Leave enough of the header for the decoder to reach the field payload.
    for length in 1..n.saturating_sub(2) {
        let mut reader = Reader::new(&bytes[..length], Encoding::Q3);
        if reader.read_bits(8).is_ok() {
            assert!(snapshots::read_q3(&mut reader, &mut client, 2, 2).is_err());
        }
        assert!(client.frame(2).is_none());
        assert_eq!(client.frame(1).map(|f| f.entities[0]), Some(entity(1, 1.)));
    }
    Ok(())
}

#[test]
fn snapshots_share_reliable_command_xor_channel_and_packet_ingress() -> Result<(), Error> {
    let mut server =
        Channel::load(QUAKE3, Endpoint::Server, 8192, 16).map_err(|_| Error::Context)?;
    let mut ring = Q3Ring::load(8, 1024, 32, None)?;
    store(&mut ring, 1, &[entity(1, 7.5)])?;
    for _ in 0..64 {
        server
            .queue_reliable(b"print a")
            .map_err(|_| Error::Context)?;
    }
    let mut payload = [0; 1400];
    let n =
        server.encode_server_output(&mut payload, |w| snapshots::write_q3(w, &ring, 1, None))?;
    let mut connections = Connections::load(1);
    connections
        .bind(
            ClientId(0),
            Endpoint::Client,
            Connection {
                route: Route {
                    socket: Endpoint::Client.socket(),
                    peer: Peer::Loopback(ClientId(0)),
                },
                channel: Channel::load(QUAKE3, Endpoint::Client, 8192, 16)
                    .map_err(|_| Error::Context)?,
                commands: Some(Commands::load(Protocol::Quake3_68)),
                output: None,
            },
        )
        .map_err(|_| Error::Context)?;
    let Some(packet) = server
        .prepare_move(&payload[..n], EventTime(1), None)
        .map_err(|_| Error::Context)?
    else {
        return Err(Error::Context);
    };
    let mut snapshots = 0;
    let mut reliable = 0;
    let projection = PlayerProjection::load(Protocol::Quake3_68, &[]);
    let mut imported = PlayerState {
        movement_rules: RuleSetId::Quake2,
        trace_rules: RuleSetId::Quake,
        ..PlayerState::default()
    };
    let mut native = native_player_context();
    connections.receive(
        Endpoint::Client.socket(),
        Peer::Loopback(ClientId(0)),
        packet.bytes,
        EventTime(2),
        |id, endpoint, incoming| {
            if let Incoming::Snapshot(snapshots::ReceivedFrame::Quake3(frame)) = incoming {
                assert_eq!(id, ClientId(0));
                assert_eq!(endpoint, Endpoint::Client);
                assert_eq!(frame.entities, [entity(1, 7.5)]);
                assert_eq!(frame.sequence, 1);
                assert_eq!(frame.command, 64);
                assert!(projection.apply(frame.player, &mut imported, &mut native, |_| None));
                assert_eq!(imported.body.position.0[0], 0.5);
                assert_eq!(imported.health, 100);
                assert_eq!(imported.movement.command_time_ms, 50);
                assert_eq!(imported.movement_rules, RuleSetId::Quake2);
                assert_eq!(imported.trace_rules, RuleSetId::Quake);
                snapshots += 1;
            } else if let Incoming::ReliableCommand { .. } = incoming {
                reliable += 1;
            }
        },
    );
    assert_eq!(snapshots, 1);
    assert_eq!(reliable, 64);
    assert_eq!(connections.command_errors, 0);
    Ok(())
}

fn deliver_server_message(
    server: &mut Channel,
    client: &mut Channel,
    client_commands: &mut Commands,
    snapshot: Option<(&Q3Ring, Option<u32>)>,
) -> Result<(), Error> {
    let sequence = server.send_state().sequence;
    let mut payload = [0; 1400];
    let n = server.encode_server_output(&mut payload, |writer| {
        if let Some((ring, from)) = snapshot {
            snapshots::write_q3(writer, ring, sequence, from)?;
        }
        Ok(())
    })?;
    let mut bytes = [0; 1400];
    let packet = server
        .prepare_move(&payload[..n], EventTime(1), None)
        .map_err(|_| Error::Context)?
        .ok_or(Error::Context)?;
    let n = packet.bytes.len();
    bytes[..n].copy_from_slice(packet.bytes);
    server.submitted(EventTime(1)).map_err(|_| Error::Context)?;
    let received = client
        .receive(&bytes[..n], EventTime(2))
        .map_err(|_| Error::Context)?;
    let qa_network::channel::Delivery::Payload(payload) = received.delivery else {
        return Err(Error::Context);
    };
    let n = client_commands.stage(payload)?;
    client_commands
        .decode_output(n, sequence, client, |_| {})
        .map(|_| ())
}

fn deliver_client_move(
    client: &Channel,
    client_commands: &Commands,
    server: &mut Channel,
    server_commands: &mut Commands,
    time: i32,
) -> Result<(bool, bool), Error> {
    let mut bytes = [0; 1400];
    let command = qa_core::primitives::UserCmd {
        server_time_ms: time,
        ..Default::default()
    };
    let n = client_commands.encode(&command, client, &mut bytes)?;
    let staged = server_commands.stage(&bytes[..n])?;
    let ack = qa_network::commands::packet::acknowledgements(&bytes[..n])?;
    let key = server.command_key(Some(ack))?;
    let movement = qa_network::commands::packet::read(
        Protocol::Quake3_68,
        &mut bytes[..n],
        client.send_state().sequence,
        key,
    )?;
    let qa_network::commands::packet::Move::Quake3 { delta, .. } = movement else {
        return Err(Error::Context);
    };
    let accepted = server_commands
        .decode(staged, client.send_state().sequence, time, 0, server)?
        .is_some();
    Ok((delta, accepted))
}

#[test]
fn native_snapshot_request_tracks_message_ack_before_stale_command_filtering() -> Result<(), Error>
{
    let mut server =
        Channel::load(QUAKE3, Endpoint::Server, 8192, 16).map_err(|_| Error::Context)?;
    let mut client =
        Channel::load(QUAKE3, Endpoint::Client, 8192, 16).map_err(|_| Error::Context)?;
    let mut incoming = Commands::load(Protocol::Quake3_68);
    let outgoing = Commands::load(Protocol::Quake3_68);
    let mut client_output = Commands::load(Protocol::Quake3_68);
    let mut ring = Q3Ring::load(8, 1024, 32, None)?;
    assert_eq!(
        deliver_client_move(&client, &outgoing, &mut server, &mut incoming, 20)?,
        (false, true)
    );
    assert_eq!(incoming.delta_request(), None);
    store(&mut ring, 1, &[entity(1, 2.)])?;
    deliver_server_message(
        &mut server,
        &mut client,
        &mut client_output,
        Some((&ring, None)),
    )?;
    assert_eq!(
        deliver_client_move(&client, &outgoing, &mut server, &mut incoming, 40)?,
        (true, true)
    );
    assert_eq!(incoming.delta_request(), Some(1));
    // A newer native server message with no snapshot must request a full frame.
    deliver_server_message(&mut server, &mut client, &mut client_output, None)?;
    assert_eq!(
        deliver_client_move(&client, &outgoing, &mut server, &mut incoming, 40)?,
        (false, false)
    );
    assert_eq!(incoming.delta_request(), None);
    store(&mut ring, 3, &[entity(1, 3.)])?;
    deliver_server_message(
        &mut server,
        &mut client,
        &mut client_output,
        Some((&ring, Some(1))),
    )?;
    assert_eq!(
        deliver_client_move(&client, &outgoing, &mut server, &mut incoming, 40)?,
        (true, false)
    );
    assert_eq!(incoming.delta_request(), Some(3));
    Ok(())
}

#[test]
fn missing_snapshot_base_requests_full_without_acknowledging_an_invalid_frame() -> Result<(), Error>
{
    let mut server =
        Channel::load(QUAKE3, Endpoint::Server, 8192, 16).map_err(|_| Error::Context)?;
    let mut client =
        Channel::load(QUAKE3, Endpoint::Client, 8192, 16).map_err(|_| Error::Context)?;
    let mut incoming = Commands::load(Protocol::Quake3_68);
    let outgoing = Commands::load(Protocol::Quake3_68);
    let mut client_output = Commands::load(Protocol::Quake3_68);
    let mut ring = Q3Ring::load(8, 1024, 32, None)?;
    // First native message is consumed but contains no accepted snapshot.
    deliver_server_message(&mut server, &mut client, &mut client_output, None)?;
    store(&mut ring, 1, &[entity(1, 2.)])?;
    store(&mut ring, 2, &[entity(1, 3.)])?;
    deliver_server_message(
        &mut server,
        &mut client,
        &mut client_output,
        Some((&ring, Some(1))),
    )?;
    assert!(client.snapshot(2).is_none());
    assert_eq!(
        client.command_state().map(|s| s.message_acknowledged),
        Some(2)
    );
    assert_eq!(
        deliver_client_move(&client, &outgoing, &mut server, &mut incoming, 20)?,
        (false, true)
    );
    assert_eq!(incoming.delta_request(), None);
    Ok(())
}

fn q2_entity(number: u32, x: f32, event: u32) -> Entity<{ states::Q2_ENTITY_WORDS }> {
    let mut words = [0; states::Q2_ENTITY_WORDS];
    words[0] = 1;
    words[8] = x.to_bits();
    words[14] = 999.0f32.to_bits();
    words[18] = event;
    Entity { number, words }
}

fn q2_store(
    ring: &mut snapshots::Q2Ring,
    sequence: u32,
    entities: &[Entity<{ states::Q2_ENTITY_WORDS }>],
) -> Result<(), Error> {
    let mut player = [0; states::Q2_PLAYER_WORDS];
    player[1] = sequence;
    ring.store(Frame {
        sequence,
        time: ThinkTime::Milliseconds(i64::from(sequence as i32 * 100)),
        command: 0,
        flags: 7,
        player: &player,
        areas: &[0x81, 0x42],
        entities,
    })
}

fn q2_send(
    server: &snapshots::Q2Ring,
    client: &mut snapshots::Q2Ring,
    sequence: u32,
    request: Option<u32>,
) -> Result<bool, Error> {
    let mut bytes = [0; 8192];
    let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
    snapshots::write_q2(&mut writer, server, sequence, request, 16)?;
    writer.write_bits(6, 8)?;
    let mut reader = Reader::new(writer.bytes(), Encoding::Bytes);
    assert_eq!(reader.read_bits(8)?, 20);
    let accepted = snapshots::read_q2(&mut reader, client)?;
    assert_eq!(reader.read_bits(8)?, 6);
    assert_eq!(reader.byte_position(), writer.size());
    Ok(accepted)
}

#[test]
fn q2_frames_share_the_ring_and_reset_unchanged_rows() -> Result<(), Error> {
    let mut server = snapshots::Q2Ring::load(8, 1024, 32, None)?;
    let mut client = snapshots::Q2Ring::load(8, 1024, 32, Some(896))?;
    q2_store(
        &mut server,
        1,
        &[q2_entity(30, 12., 9), q2_entity(31, 25., 9)],
    )?;
    assert!(q2_send(&server, &mut client, 1, None)?);
    q2_store(
        &mut server,
        2,
        &[
            q2_entity(30, 12., 0),
            q2_entity(31, 25., 0),
            q2_entity(260, 33., 0),
        ],
    )?;
    assert!(q2_send(&server, &mut client, 2, Some(1))?);
    let frame = client.frame(2).ok_or(Error::Context)?;
    assert_eq!(frame.time, ThinkTime::Milliseconds(200));
    assert_eq!(frame.flags, 7);
    assert_eq!(frame.areas, &[0x81, 0x42]);
    assert_eq!(frame.player[1], 2);
    assert_eq!(
        frame.entities.iter().map(|e| e.number).collect::<Vec<_>>(),
        [30, 31, 260]
    );
    assert_eq!(frame.entities[0].words[18], 0);
    assert_eq!(frame.entities[0].words[14], 12.0f32.to_bits());
    // The unchanged trailing row also takes the native reset policy.
    q2_store(
        &mut server,
        3,
        &[q2_entity(31, 25., 0), q2_entity(260, 33., 0)],
    )?;
    assert!(q2_send(&server, &mut client, 3, Some(2))?);
    let frame = client.frame(3).ok_or(Error::Context)?;
    assert_eq!(frame.entities.len(), 2);
    assert_eq!(frame.entities[0].number, 31);
    assert_eq!(frame.entities[0].words[14], 25.0f32.to_bits());
    assert_eq!(client.counts().accepted, 3);
    Ok(())
}

#[test]
fn q2_missing_base_is_consumed_and_retained_without_publishing() -> Result<(), Error> {
    let mut server = snapshots::Q2Ring::load(8, 1024, 32, None)?;
    let mut client = snapshots::Q2Ring::load(8, 1024, 32, Some(896))?;
    q2_store(&mut server, 1, &[q2_entity(30, 12., 0)])?;
    q2_store(&mut server, 2, &[q2_entity(30, 25., 0)])?;
    assert!(!q2_send(&server, &mut client, 2, Some(1))?);
    assert!(client.frame(2).is_none());
    assert_eq!(client.counts().missing_base, 1);
    // Original CL_ParseFrame's next delta validates sequence/row age even
    // when the saved addressed frame was invalid. Normal clients request full.
    q2_store(&mut server, 3, &[q2_entity(30, 33., 0)])?;
    assert!(q2_send(&server, &mut client, 3, Some(2))?);
    assert_eq!(
        client.frame(3).ok_or(Error::Context)?.entities[0].words[8],
        33.0f32.to_bits()
    );
    Ok(())
}

#[test]
fn q2_frame_overflow_and_area_limits_do_not_publish_partial_data() -> Result<(), Error> {
    let mut server = snapshots::Q2Ring::load(8, 1024, 32, None)?;
    let mut client = snapshots::Q2Ring::load(1, 1024, 32, Some(896))?;
    q2_store(
        &mut server,
        1,
        &[q2_entity(30, 12., 0), q2_entity(31, 25., 0)],
    )?;
    assert!(!q2_send(&server, &mut client, 1, None)?);
    assert!(client.frame(1).is_none());
    assert_eq!(client.counts().overflow, 1);
    q2_store(&mut server, 2, &[q2_entity(30, 12., 0)])?;
    assert!(q2_send(&server, &mut client, 2, None)?);
    let mut bytes = [0; 16];
    let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
    for (value, width) in [(3, 32), (u32::MAX, 32), (0, 8), (33, 8)] {
        writer.write_bits(value, width)?;
    }
    let mut reader = Reader::new(writer.bytes(), Encoding::Bytes);
    assert_eq!(
        snapshots::read_q2(&mut reader, &mut client),
        Err(Error::Count)
    );
    assert!(client.frame(3).is_none());
    Ok(())
}

fn q2_deliver_frame(
    server: &mut Channel,
    connections: &mut Connections,
    ring: &snapshots::Q2Ring,
    frame: u32,
    delta: Option<u32>,
    deliver: bool,
    consume: impl FnMut(ClientId, Endpoint, Incoming<'_>),
) -> Result<(), Error> {
    let mut payload = [0; 1400];
    let mut writer = Writer::new(&mut payload, Encoding::Bytes);
    writer.write_bits(10, 8)?;
    writer.write_bits(3, 8)?;
    writer.write_data(b"before\0")?;
    snapshots::write_q2(&mut writer, ring, frame, delta, 16)?;
    writer.write_bits(6, 8)?;
    writer.write_bits(4, 8)?;
    writer.write_data(b"after\0")?;
    let mut bytes = [0; 1400];
    let packet = server
        .prepare_move(writer.bytes(), EventTime(1), None)
        .map_err(|_| Error::Context)?
        .ok_or(Error::Context)?;
    let n = packet.bytes.len();
    bytes[..n].copy_from_slice(packet.bytes);
    server.submitted(EventTime(1)).map_err(|_| Error::Context)?;
    if deliver {
        connections.receive(
            Endpoint::Client.socket(),
            Peer::Loopback(ClientId(0)),
            &bytes[..n],
            EventTime(2),
            consume,
        );
    }
    Ok(())
}

fn q2_request(
    connections: &Connections,
    receiver: &mut Commands,
    server: &mut Channel,
) -> Result<Option<u32>, Error> {
    let client = connections
        .get(ClientId(0), Endpoint::Client)
        .ok_or(Error::Context)?;
    let codec = client.commands.as_ref().ok_or(Error::Context)?;
    let command = qa_core::primitives::UserCmd {
        duration_ms: 20,
        ..Default::default()
    };
    let mut bytes = [0; 1400];
    let n = codec.encode(&command, &client.channel, &mut bytes)?;
    let staged = receiver.stage(&bytes[..n])?;
    let movement = qa_network::commands::packet::read(
        Protocol::Quake2_34,
        &mut bytes[..n],
        client.channel.send_state().sequence,
        qa_network::commands::packet::Key::default(),
    )?;
    let qa_network::commands::packet::Move::Quake2 { last_frame, .. } = movement else {
        return Err(Error::Context);
    };
    receiver.decode(staged, client.channel.send_state().sequence, 0, 0, server)?;
    let expected = (last_frame >= 0).then_some(last_frame as u32);
    assert_eq!(receiver.delta_request(), expected);
    Ok(expected)
}

#[test]
fn q2_connected_frames_use_payload_numbers_and_invalid_frames_request_full() -> Result<(), Error> {
    let policy = Protocol::Quake2_34.channel();
    let mut server =
        Channel::load(policy, Endpoint::Server, 8192, 16).map_err(|_| Error::Context)?;
    let mut commands = Commands::load(Protocol::Quake2_34);
    let mut connections = Connections::load(1);
    connections
        .bind(
            ClientId(0),
            Endpoint::Client,
            Connection {
                route: Route {
                    socket: Endpoint::Client.socket(),
                    peer: Peer::Loopback(ClientId(0)),
                },
                channel: Channel::load(policy, Endpoint::Client, 8192, 16)
                    .map_err(|_| Error::Context)?,
                commands: Some(Commands::load(Protocol::Quake2_34)),
                output: None,
            },
        )
        .map_err(|_| Error::Context)?;
    let mut ring = snapshots::Q2Ring::load(8, 1024, 32, None)?;
    let mut frames = 0;
    let mut prints = 0;
    let projection = PlayerProjection::load(Protocol::Quake2_34, &[]);
    let mut imported = PlayerState {
        movement_rules: RuleSetId::Quake3,
        trace_rules: RuleSetId::QuakeWorld,
        ..PlayerState::default()
    };
    let mut native = native_player_context();
    assert_eq!(q2_request(&connections, &mut commands, &mut server)?, None);
    for (frame, delta, deliver, valid) in [
        (41, None, true, true),
        (42, Some(41), true, true),
        (43, Some(42), false, false),
        (44, Some(43), true, false),
        (45, None, true, true),
    ] {
        q2_store(&mut ring, frame, &[q2_entity(30, frame as f32, 0)])?;
        q2_deliver_frame(
            &mut server,
            &mut connections,
            &ring,
            frame,
            delta,
            deliver,
            |id, endpoint, incoming| {
                assert_eq!(id, ClientId(0));
                assert_eq!(endpoint, Endpoint::Client);
                match incoming {
                    Incoming::Snapshot(snapshots::ReceivedFrame::Quake2(snapshot)) => {
                        assert!(valid);
                        assert_eq!(snapshot.sequence, frame);
                        assert_eq!(
                            snapshot.time,
                            ThinkTime::Milliseconds(i64::from(frame as i32 * 100))
                        );
                        assert!(projection.apply(
                            snapshot.player,
                            &mut imported,
                            &mut native,
                            |_| None,
                        ));
                        assert_eq!(imported.body.position.0[0], frame as f32 * 0.125);
                        assert_eq!(imported.movement_rules, RuleSetId::Quake3);
                        assert_eq!(imported.trace_rules, RuleSetId::QuakeWorld);
                        let mut expected = q2_entity(30, frame as f32, 0);
                        if let Some(previous) = delta {
                            expected.words[14] = (previous as f32).to_bits();
                        }
                        assert_eq!(snapshot.entities, [expected]);
                        frames += 1;
                    }
                    Incoming::Print(print) => {
                        if prints % 2 == 0 {
                            assert_eq!(print.text, b"before");
                            assert_eq!(print.level, Some(3));
                            assert_eq!(print.kind, qa_core::primitives::PrintKind::Chat);
                        } else {
                            assert_eq!(print.text, b"after");
                            assert_eq!(print.kind, qa_core::primitives::PrintKind::Layout);
                        }
                        prints += 1;
                    }
                    _ => unreachable!(),
                }
            },
        )?;
        if deliver {
            assert_eq!(
                q2_request(&connections, &mut commands, &mut server)?,
                valid.then_some(frame)
            );
        }
    }
    assert_eq!((frames, prints), (3, 8));
    assert_eq!(connections.command_errors, 0);
    // Frame 41 arrived in channel packet 1, so using its channel sequence as
    // clc_move's lastframe would have failed the native requests above.
    Ok(())
}

#[test]
fn q2_stream_rejection_is_bounded_and_the_existing_frame_is_not_replaced() -> Result<(), Error> {
    let policy = Protocol::Quake2_34.channel();
    let mut client =
        Channel::load(policy, Endpoint::Client, 8192, 16).map_err(|_| Error::Context)?;
    client.configure_snapshots(Protocol::Quake2_34)?;
    let mut commands = Commands::load(Protocol::Quake2_34);
    let mut ring = snapshots::Q2Ring::load(8, 1024, 32, None)?;
    q2_store(&mut ring, 41, &[q2_entity(30, 7., 0)])?;
    let mut payload = [0; 1400];
    let mut writer = Writer::new(&mut payload, Encoding::Bytes);
    snapshots::write_q2(&mut writer, &ring, 41, None, 16)?;
    let n = commands.stage(writer.bytes())?;
    assert_eq!(commands.decode_output(n, 1, &mut client, |_| {})?, Some(41));
    q2_store(&mut ring, 42, &[q2_entity(30, 8., 0)])?;
    let mut writer = Writer::new(&mut payload, Encoding::Bytes);
    snapshots::write_q2(&mut writer, &ring, 42, Some(41), 16)?;
    let length = writer.size();
    for cut in 1..length {
        let n = commands.stage(&payload[..cut])?;
        assert!(commands.decode_output(n, 2, &mut client, |_| {}).is_err());
        assert!(client.snapshot(42).is_none());
        assert!(client.snapshot(41).is_some());
        assert_eq!(commands.delta_request(), None);
    }
    for bytes in [
        &[17][..],
        &[18][..],
        &[19][..],
        &[9][..],
        &[10, 2, b'x'][..],
    ] {
        let n = commands.stage(bytes)?;
        assert!(commands.decode_output(n, 3, &mut client, |_| {}).is_err());
    }
    let n = commands.stage(&payload[..length])?;
    assert_eq!(commands.decode_output(n, 4, &mut client, |_| {})?, Some(42));
    // Idempotent binding preserves received frames; a different native payload
    // cannot reuse their storage just because its channel header is identical.
    client.configure_snapshots(Protocol::Quake2_34)?;
    assert!(client.configure_snapshots(Protocol::QuakeWorld28).is_err());
    assert!(client.snapshot(42).is_some());
    Ok(())
}

fn qw_entity(number: u32, x: f32) -> Entity<{ states::QW_ENTITY_WORDS }> {
    let mut words = [0; states::QW_ENTITY_WORDS];
    words[0] = 1;
    words[5] = x.to_bits();
    Entity { number, words }
}
fn qw_store(
    ring: &mut snapshots::QwRing,
    sequence: u32,
    entities: &[Entity<{ states::QW_ENTITY_WORDS }>],
) -> Result<(), Error> {
    ring.store(Frame {
        sequence,
        time: ThinkTime::Milliseconds(i64::from(0)),
        command: 0,
        flags: 0,
        areas: &[],
        player: &[],
        entities,
    })
}
fn qw_encode(
    ring: &snapshots::QwRing,
    sequence: u32,
    request: Option<u32>,
    bytes: &mut [u8],
) -> Result<usize, Error> {
    let mut writer = Writer::new(bytes, Encoding::Bytes);
    snapshots::write_qw(
        &mut writer,
        ring,
        sequence,
        request.map(|base| (base, base as u8)),
    )?;
    writer.write_bits(6, 8)?;
    Ok(writer.size())
}
fn qw_decode(
    ring: &mut snapshots::QwRing,
    sequence: u32,
    request: Option<u32>,
    outgoing: u32,
    bytes: &[u8],
) -> Result<bool, Error> {
    let mut reader = Reader::new(bytes, Encoding::Bytes);
    let opcode = reader.read_bits(8)?;
    assert!(matches!(opcode, 47 | 48));
    let accepted =
        snapshots::read_qw(&mut reader, ring, sequence, opcode == 48, request, outgoing)?;
    assert_eq!(reader.read_bits(8)?, 6);
    assert_eq!(reader.byte_position(), bytes.len());
    Ok(accepted)
}

#[test]
fn qw_packet_frames_merge_baselines_removals_and_ignore_advisory_from() -> Result<(), Error> {
    let mut server = snapshots::QwRing::load(64, 512, 0, None)?;
    let mut client = snapshots::QwRing::load(64, 512, 0, None)?;
    let baseline = qw_entity(7, 40.);
    assert!(server.set_baseline(7, &baseline.words));
    assert!(client.set_baseline(7, &baseline.words));
    let old = [qw_entity(1, 1.), qw_entity(3, 3.), qw_entity(5, 5.)];
    qw_store(&mut server, 256, &old)?;
    let mut bytes = [0; 1400];
    let n = qw_encode(&server, 256, None, &mut bytes)?;
    assert!(qw_decode(&mut client, 256, None, 257, &bytes[..n])?);
    let new = [old[0], qw_entity(3, 9.125), baseline];
    qw_store(&mut server, 257, &new)?;
    let n = qw_encode(&server, 257, Some(256), &mut bytes)?;
    assert_eq!(&bytes[..2], &[48, 0]);
    bytes[1] = 255; // qsrc only warns; the caller's actual request still selects 256
    assert!(qw_decode(&mut client, 257, Some(256), 258, &bytes[..n])?);
    let frame = client.frame(257).ok_or(Error::Context)?;
    assert_eq!(
        frame.entities.iter().map(|e| e.number).collect::<Vec<_>>(),
        [1, 3, 7]
    );
    for (actual, expected) in frame.entities.iter().zip(&new) {
        assert_eq!(actual.words[..11], expected.words[..11]);
        assert_eq!(actual.words[11] & 0xffff0000, 0); // packet header is unsigned in qsrc
    }
    // Native QW's request-age check uses outgoing sequence, not incoming time.
    assert!(!qw_decode(&mut client, 258, Some(256), 319, &bytes[..n])?);
    assert!(client.current().is_none());
    assert!(client.frame(256).is_some());
    assert_eq!(client.counts().missing_base, 1);
    qw_store(&mut server, 259, &new)?;
    let n = qw_encode(&server, 259, None, &mut bytes)?;
    assert!(qw_decode(&mut client, 259, None, 320, &bytes[..n])?);
    Ok(())
}

#[test]
fn qw_packet_frames_bound_missing_truncated_and_native_capacity() -> Result<(), Error> {
    let mut server = snapshots::QwRing::load(128, 1024, 0, None)?;
    let mut client = snapshots::QwRing::load(64, 512, 0, None)?;
    let old = [qw_entity(1, 1.)];
    qw_store(&mut server, 1, &old)?;
    let mut bytes = [0; 8192];
    let n = qw_encode(&server, 1, None, &mut bytes)?;
    assert!(qw_decode(&mut client, 1, None, 2, &bytes[..n])?);
    qw_store(&mut server, 2, &[qw_entity(1, 2.), qw_entity(3, 3.)])?;
    let n = qw_encode(&server, 2, Some(1), &mut bytes)?;
    for cut in 1..n - 1 {
        assert!(qw_decode(&mut client, 2, Some(1), 3, &bytes[..cut]).is_err());
        assert!(client.frame(2).is_none());
        assert!(client.frame(1).is_some());
    }
    let mut missing = snapshots::QwRing::load(64, 512, 0, None)?;
    assert!(!qw_decode(&mut missing, 2, Some(1), 3, &bytes[..n])?);
    assert!(missing.current().is_none());
    assert_eq!(missing.counts().missing_base, 1);
    // The engine's retained 32 slots choose a native full update after wrap.
    qw_store(&mut server, 33, &old)?;
    let n = qw_encode(&server, 33, Some(1), &mut bytes)?;
    assert_eq!(bytes[0], 47);
    assert!(qw_decode(&mut client, 33, Some(1), 34, &bytes[..n])?);
    let rows = (1..=65).map(|n| qw_entity(n, n as f32)).collect::<Vec<_>>();
    qw_store(&mut server, 34, &rows)?;
    let n = qw_encode(&server, 34, None, &mut bytes)?;
    assert!(qw_decode(&mut client, 34, None, 35, &bytes[..n])?);
    assert_eq!(client.frame(34).ok_or(Error::Context)?.entities.len(), 64);
    let mut bounded = snapshots::QwRing::load(4, 512, 0, None)?;
    assert!(!qw_decode(&mut bounded, 34, None, 35, &bytes[..n])?);
    assert_eq!(bounded.counts().overflow, 1);
    // Native REMOVE is not valid in a full packet; do not publish it.
    let mut reader = Reader::new(&[1, 64, 0, 0], Encoding::Bytes);
    assert!(snapshots::read_qw(&mut reader, &mut client, 35, false, None, 36).is_err());
    assert!(client.frame(35).is_none());
    assert!(client.frame(34).is_some());
    Ok(())
}

fn qw_move(
    connections: &mut Connections,
    server: &mut Channel,
    commands: &mut Commands,
    deliver: bool,
) -> Result<Option<u32>, Error> {
    let client = connections
        .get_mut(ClientId(0), Endpoint::Client)
        .ok_or(Error::Context)?;
    let codec = client.commands.as_ref().ok_or(Error::Context)?;
    let command = qa_core::primitives::UserCmd {
        duration_ms: 20,
        movement: [300., 0., 0.],
        ..Default::default()
    };
    let mut payload = [0; 1400];
    let length = codec.encode(&command, &client.channel, &mut payload)?;
    let request = codec.snapshot_request(&client.channel);
    let packet = if let Some(pending) = client.channel.pending_packet() {
        pending
    } else {
        client
            .channel
            .prepare_move(&payload[..length], EventTime(1), request)
            .map_err(|_| Error::Context)?
            .ok_or(Error::Context)?
    };
    let mut bytes = [0; 1400];
    let length = packet.bytes.len();
    bytes[..length].copy_from_slice(packet.bytes);
    client
        .channel
        .submitted(EventTime(1))
        .map_err(|_| Error::Context)?;
    if deliver {
        let received = server
            .receive(&bytes[..length], EventTime(2))
            .map_err(|_| Error::Context)?;
        let sequence = received.header.sequence;
        let qa_network::channel::Delivery::Payload(payload) = received.delivery else {
            return Err(Error::Context);
        };
        let length = commands.stage(payload)?;
        let command = commands
            .decode(length, sequence, 0, 0, server)?
            .ok_or(Error::Context)?;
        assert_eq!(command.movement[0], 300.);
        assert_eq!(command.duration_ms, 20);
    }
    Ok(commands.delta_request())
}

fn qw_reply(
    server: &mut Channel,
    connections: &mut Connections,
    ring: &snapshots::QwRing,
    sequence: u32,
    request: Option<(u32, u8)>,
    deliver: bool,
    consume: impl FnMut(ClientId, Endpoint, Incoming<'_>),
) -> Result<(), Error> {
    assert_eq!(server.send_state().sequence, sequence);
    server.publish_snapshot(snapshots::ReceivedFrame::QuakeWorld(
        ring.frame(sequence).ok_or(Error::Context)?,
    ))?;
    let mut payload = [0; 1400];
    let mut writer = Writer::new(&mut payload, Encoding::Bytes);
    writer.write_bits(8, 8)?;
    writer.write_bits(2, 8)?;
    writer.write_data(b"before\0")?;
    let start = writer.size();
    server.write_snapshot(
        &mut writer,
        sequence,
        request.map(|(base, _)| u32::from(base as u8)),
        0,
    )?;
    writer.write_bits(1, 8)?;
    writer.write_bits(26, 8)?;
    writer.write_data(b"after\0")?;
    let length = writer.size();
    if let Some((_, advisory)) = request {
        payload[start + 1] = advisory;
    }
    let packet = server
        .prepare_move(&payload[..length], EventTime(2), None)
        .map_err(|_| Error::Context)?
        .ok_or(Error::Context)?;
    let mut bytes = [0; 1400];
    let length = packet.bytes.len();
    bytes[..length].copy_from_slice(packet.bytes);
    server.submitted(EventTime(2)).map_err(|_| Error::Context)?;
    if deliver {
        connections.receive(
            Endpoint::Client.socket(),
            Peer::Loopback(ClientId(0)),
            &bytes[..length],
            EventTime(3),
            consume,
        );
    }
    Ok(())
}

fn byte_connection(protocol: Protocol) -> Result<(Channel, Commands, Connections), Error> {
    let mut connections = Connections::load(1);
    connections
        .bind(
            ClientId(0),
            Endpoint::Client,
            Connection {
                route: Route {
                    socket: Endpoint::Client.socket(),
                    peer: Peer::Loopback(ClientId(0)),
                },
                channel: Channel::load(protocol.channel(), Endpoint::Client, 8192, 16)
                    .map_err(|_| Error::Context)?,
                output: None,
                commands: Some(Commands::load(protocol)),
            },
        )
        .map_err(|_| Error::Context)?;
    let mut server = Channel::load(protocol.channel(), Endpoint::Server, 8192, 16)
        .map_err(|_| Error::Context)?;
    server.configure_snapshots(protocol)?;
    Ok((server, Commands::load(protocol), connections))
}

#[test]
fn qw_connected_frames_use_submitted_requests_and_recover_after_slipped_reply() -> Result<(), Error>
{
    let (mut server, mut commands, mut connections) = byte_connection(Protocol::QuakeWorld28)?;
    let mut ring = snapshots::QwRing::load(64, 512, 0, None)?;
    let mut frames = Vec::new();
    let mut prints = Vec::new();
    for sequence in 1..=4 {
        let request = qw_move(&mut connections, &mut server, &mut commands, true)?;
        assert_eq!(
            request,
            match sequence {
                1 => None,
                2 => Some(1),
                _ => Some(2),
            }
        );
        qw_store(&mut ring, sequence, &[qw_entity(3, sequence as f32)])?;
        qw_reply(
            &mut server,
            &mut connections,
            &ring,
            sequence,
            request.map(|n| (n, 255)),
            sequence != 3,
            |_, endpoint, incoming| {
                assert_eq!(endpoint, Endpoint::Client);
                match incoming {
                    Incoming::Snapshot(snapshots::ReceivedFrame::QuakeWorld(frame)) => {
                        assert_eq!(
                            f32::from_bits(frame.entities[0].words[5]),
                            frame.sequence as f32
                        );
                        frames.push(frame.sequence);
                    }
                    Incoming::Print(print) => prints.push(print.text.to_vec()),
                    _ => panic!("QW native frame/print"),
                }
            },
        )?;
    }
    assert_eq!(frames, [1, 2, 4]);
    assert_eq!(prints.len(), 6);
    for pair in prints.as_chunks::<2>().0 {
        assert_eq!(pair[0], b"before");
        assert_eq!(pair[1], b"after");
    }
    let client = connections
        .get_mut(ClientId(0), Endpoint::Client)
        .ok_or(Error::Context)?;
    assert!(client.channel.snapshot(3).is_none());
    let codec = client.commands.as_ref().ok_or(Error::Context)?;
    let command = qa_core::primitives::UserCmd {
        duration_ms: 20,
        movement: [300., 0., 0.],
        ..Default::default()
    };
    let mut payload = [0; 1400];
    let n = codec.encode(&command, &client.channel, &mut payload)?;
    let request = codec.snapshot_request(&client.channel);
    assert_eq!(request, Some(4));
    client
        .channel
        .prepare_move(&payload[..n], EventTime(4), request)
        .map_err(|_| Error::Context)?;
    // No transport admission yet: a delta response to that unsent request must
    // not borrow a conveniently retained base and invent an accepted frame.
    qw_store(&mut ring, 5, &[qw_entity(3, 5.)])?;
    qw_reply(
        &mut server,
        &mut connections,
        &ring,
        5,
        Some((4, 4)),
        true,
        |_, _, incoming| {
            assert!(!matches!(incoming, Incoming::Snapshot(_)));
        },
    )?;
    let client = connections
        .get(ClientId(0), Endpoint::Client)
        .ok_or(Error::Context)?;
    assert!(client.channel.snapshot(5).is_none());
    assert!(client.channel.snapshot(4).is_some());
    assert_eq!(
        client
            .commands
            .as_ref()
            .ok_or(Error::Context)?
            .delta_request(),
        None
    );
    // Retry the exact pending move: its request is still four despite current
    // CLIENT validity changing. SERVER processes it even though reply five slipped.
    assert_eq!(
        qw_move(&mut connections, &mut server, &mut commands, true)?,
        Some(4)
    );
    assert_eq!(server.send_state().sequence, 6);
    assert!(
        server
            .prepare_output(EventTime(5))
            .map_err(|_| Error::Context)?
            .is_none()
    );
    assert_eq!(
        qw_move(&mut connections, &mut server, &mut commands, true)?,
        None
    );
    qw_store(&mut ring, 6, &[qw_entity(3, 6.)])?;
    let mut recovered = false;
    qw_reply(
        &mut server,
        &mut connections,
        &ring,
        6,
        None,
        true,
        |_, _, incoming| {
            recovered |= matches!(incoming, Incoming::Snapshot(snapshots::ReceivedFrame::QuakeWorld(frame)) if frame.sequence == 6);
        },
    )?;
    assert!(recovered);
    assert_eq!(connections.command_errors, 0);
    Ok(())
}

#[test]
fn qw_connected_request_byte_zero_retains_full_sequence_after_a_gap() -> Result<(), Error> {
    let (mut server, mut commands, mut connections) = byte_connection(Protocol::QuakeWorld28)?;
    for _ in 1..256 {
        qw_move(&mut connections, &mut server, &mut commands, false)?;
    }
    assert_eq!(
        qw_move(&mut connections, &mut server, &mut commands, true)?,
        None
    );
    assert_eq!(server.send_state().sequence, 256);
    let mut ring = snapshots::QwRing::load(64, 512, 0, None)?;
    qw_store(&mut ring, 256, &[qw_entity(3, 10.)])?;
    qw_reply(
        &mut server,
        &mut connections,
        &ring,
        256,
        None,
        true,
        |_, _, _| {},
    )?;
    assert_eq!(
        qw_move(&mut connections, &mut server, &mut commands, true)?,
        Some(0)
    );
    qw_store(&mut ring, 257, &[qw_entity(3, 11.)])?;
    let mut accepted = false;
    qw_reply(
        &mut server,
        &mut connections,
        &ring,
        257,
        Some((256, 255)),
        true,
        |_, _, incoming| {
            accepted |= matches!(incoming, Incoming::Snapshot(snapshots::ReceivedFrame::QuakeWorld(frame)) if frame.sequence == 257 && frame.entities[0].words[5] == 11.0f32.to_bits());
        },
    )?;
    assert!(accepted);
    assert_eq!(connections.command_errors, 0);
    Ok(())
}

#[test]
fn qw_reply_alignment_rebuilds_unsent_payload_without_retiring_reliable_data() -> Result<(), Error>
{
    let (mut server, mut commands, mut connections) = byte_connection(Protocol::QuakeWorld28)?;
    qw_move(&mut connections, &mut server, &mut commands, true)?;
    let print = b"\x08\x02retained\0";
    let receipt = server.queue_reliable(print).map_err(|_| Error::Context)?;
    assert!(
        server
            .prepare_output(EventTime(1))
            .map_err(|_| Error::Context)?
            .is_some()
    );
    let selected = server.send_state();
    assert_eq!(selected.sequence, 1);
    assert_eq!(selected.reliable_bytes, print.len());
    assert_eq!(selected.packets, 0);
    qw_move(&mut connections, &mut server, &mut commands, false)?;
    qw_move(&mut connections, &mut server, &mut commands, false)?;
    qw_move(&mut connections, &mut server, &mut commands, true)?;
    assert!(server.pending_packet().is_none());
    assert_eq!(server.send_state().sequence, 4);
    assert_eq!(
        server.send_state().reliable_sequence,
        selected.reliable_sequence
    );
    assert_eq!(server.send_state().reliable_bytes, print.len());
    assert!(server.reliable_receipts().is_empty());
    let packet = server
        .prepare_output(EventTime(2))
        .map_err(|_| Error::Context)?
        .ok_or(Error::Context)?;
    let mut bytes = [0; 1400];
    let length = packet.bytes.len();
    bytes[..length].copy_from_slice(packet.bytes);
    assert_eq!(&bytes[length - print.len()..length], print);
    server.submitted(EventTime(2)).map_err(|_| Error::Context)?;
    assert_eq!(server.send_state().last_reliable_sequence, 5);
    assert!(server.reliable_receipts().is_empty());
    let mut delivered = 0;
    connections.receive(
        Endpoint::Client.socket(),
        Peer::Loopback(ClientId(0)),
        &bytes[..length],
        EventTime(3),
        |_, _, incoming| {
            if let Incoming::Print(print) = incoming {
                assert_eq!(print.text, b"retained");
                delivered += 1;
            }
        },
    );
    assert_eq!(delivered, 1);
    qw_move(&mut connections, &mut server, &mut commands, true)?;
    assert_eq!(server.reliable_receipts(), &[receipt]);
    assert_eq!(server.send_state().reliable_bytes, 0);
    assert_eq!(connections.command_errors, 0);
    Ok(())
}

fn server_payload(
    server: &mut Channel,
    connections: &mut Connections,
    payload: &[u8],
    consume: impl FnMut(ClientId, Endpoint, Incoming<'_>),
) -> Result<(), Error> {
    let packet = server
        .prepare_move(payload, EventTime(2), None)
        .map_err(|_| Error::Context)?
        .ok_or(Error::Context)?;
    let mut bytes = [0; 1400];
    let length = packet.bytes.len();
    bytes[..length].copy_from_slice(packet.bytes);
    server.submitted(EventTime(2)).map_err(|_| Error::Context)?;
    connections.receive(
        Endpoint::Client.socket(),
        Peer::Loopback(ClientId(0)),
        &bytes[..length],
        EventTime(3),
        consume,
    );
    Ok(())
}

#[test]
fn qw_connected_playerinfo_retains_native_slot_commands_and_imports_common_players()
-> Result<(), Error> {
    use qa_core::primitives::Vec3;
    use qa_network::commands::{QwCmd, packet::ZERO_QW};
    let (mut server, mut commands, mut connections) = byte_connection(Protocol::QuakeWorld28)?;
    connections
        .get_mut(ClientId(0), Endpoint::Client)
        .ok_or(Error::Context)?
        .channel
        .set_qw_player_model(42)?;
    let first = QwCmd {
        msec: 20,
        view_angles: Vec3([90., 45., -90.]),
        movement: [300, -20, 0],
        ..ZERO_QW
    };
    let later = QwCmd {
        view_angles: Vec3([-180., 90., 45.]),
        movement: [-17, 18, 19],
        ..first
    };
    let projection = PlayerProjection::load(Protocol::QuakeWorld28, &[]);
    let mut players: [PlayerState; 2] = std::array::from_fn(|_| PlayerState {
        movement_rules: RuleSetId::Quake3,
        trace_rules: RuleSetId::Quake2,
        view_angles: Vec3([17., 18., 19.]),
        health: 123,
        ..Default::default()
    });
    let mut seen = 0;
    for sequence in 1..=65 {
        qw_move(&mut connections, &mut server, &mut commands, true)?;
        if ![1, 2, 33, 65].contains(&sequence) {
            continue;
        }
        let mut payload = [0; 1400];
        let mut writer = Writer::new(&mut payload, Encoding::Bytes);
        let mut words = [0; states::QW_PLAYER_WORDS];
        words[0] = (sequence as f32 * 0.125).to_bits();
        words[3] = 9.0f32.to_bits();
        if sequence == 1 {
            words[4] = 17;
            words[5] = 300.0f32.to_bits();
            words[8] = 99.0f32.to_bits();
            words[9] = 7.0f32.to_bits();
            words[10] = 8.0f32.to_bits();
            words[11] = 9.0f32.to_bits();
            words[12] = 0x1ff;
        } else if sequence == 33 {
            words[12] = 2;
        }
        assert!(states::write_qw_player(
            &mut writer,
            0,
            &words,
            if sequence == 33 { later } else { first },
        )?);
        if matches!(sequence, 1 | 65) {
            let mut other = words;
            other[12] = if sequence == 1 { 2 } else { 0 };
            assert!(states::write_qw_player(&mut writer, 31, &other, later)?);
        }
        writer.write_bits(26, 8)?;
        writer.write_data(b"playerinfo\0")?;
        let mut records = 0;
        server_payload(
            &mut server,
            &mut connections,
            writer.bytes(),
            |_, _, incoming| {
                match incoming {
                    Incoming::PlayerInfo(info) => {
                        let expected = match (sequence, info.number) {
                            (2, 0) => ZERO_QW,
                            (33, 0) | (_, 31) => later,
                            _ => first,
                        };
                        assert_eq!(info.command, expected);
                        assert_eq!(
                            info.words[8],
                            if sequence == 1 && info.number == 0 {
                                99
                            } else {
                                42
                            }
                        );
                        if sequence != 1 || info.number == 31 {
                            assert_eq!(&info.words[5..8], &[0; 3]);
                            assert_eq!(&info.words[9..12], &[0; 3]);
                        }
                        // An explicit connection namespace maps native players to
                        // the common array; native 31 is not common ClientId(31).
                        let slot = match info.number {
                            0 => 1,
                            31 => 0,
                            _ => panic!("native player namespace"),
                        };
                        let mut context = native_player_context();
                        assert!(projection.apply(
                            &info.words,
                            &mut players[slot],
                            &mut context,
                            |_| None
                        ));
                        assert_eq!(players[slot].body.position.0[0], sequence as f32 * 0.125);
                        assert_eq!(players[slot].movement_rules, RuleSetId::Quake3);
                        assert_eq!(players[slot].trace_rules, RuleSetId::Quake2);
                        assert_eq!(players[slot].view_angles, Vec3([17., 18., 19.]));
                        assert_eq!(players[slot].health, 123);
                        records += 1;
                        seen += 1;
                    }
                    Incoming::Print(print) => assert_eq!(print.text, b"playerinfo"),
                    _ => panic!("QW playerinfo/print stream"),
                }
            },
        )?;
        assert_eq!(records, if matches!(sequence, 1 | 65) { 2 } else { 1 });
    }
    assert_eq!(seen, 6);
    assert_eq!(connections.command_errors, 0);
    Ok(())
}

#[test]
fn qw_playerinfo_rejects_bad_numbers_and_does_not_commit_truncated_commands() -> Result<(), Error> {
    use qa_network::commands::{QwCmd, packet::ZERO_QW};
    let (mut server, mut commands, mut connections) = byte_connection(Protocol::QuakeWorld28)?;
    let mut seen = 0;
    for sequence in 1..=67 {
        qw_move(&mut connections, &mut server, &mut commands, true)?;
        if ![1, 2, 3, 67].contains(&sequence) {
            continue;
        }
        let mut payload = [0; 1400];
        let length = match sequence {
            1 => {
                payload[0] = 42;
                1
            }
            2 => {
                payload[..2].copy_from_slice(&[42, 32]);
                2
            }
            _ => {
                let mut writer = Writer::new(&mut payload, Encoding::Bytes);
                let mut words = [0; states::QW_PLAYER_WORDS];
                words[12] = if sequence == 3 { 2 | (1 << 8) } else { 0 };
                assert!(states::write_qw_player(
                    &mut writer,
                    0,
                    &words,
                    QwCmd {
                        msec: 99,
                        ..ZERO_QW
                    }
                )?);
                writer.size() - usize::from(sequence == 3)
            }
        };
        server_payload(
            &mut server,
            &mut connections,
            &payload[..length],
            |_, _, incoming| {
                let Incoming::PlayerInfo(info) = incoming else {
                    panic!("player record")
                };
                assert_eq!(sequence, 67);
                assert_eq!(info.command, ZERO_QW);
                seen += 1;
            },
        )?;
    }
    assert_eq!(seen, 1);
    assert_eq!(connections.command_errors, 3);
    Ok(())
}

#[test]
fn qw_playerinfo_default_model_and_omitted_command_context_are_per_connection() -> Result<(), Error>
{
    use qa_network::commands::packet::ZERO_QW;
    let (mut server, _, mut connections) = byte_connection(Protocol::QuakeWorld28)?;
    assert!(server.set_qw_player_model(7).is_err());
    let mut other = Channel::load(Protocol::QuakeWorld28.channel(), Endpoint::Client, 8192, 16)
        .map_err(|_| Error::Context)?;
    other.configure_snapshots(Protocol::QuakeWorld28)?;
    let client = connections
        .get_mut(ClientId(0), Endpoint::Client)
        .ok_or(Error::Context)?;
    client.channel.set_qw_player_model(42)?;
    other.set_qw_player_model(88)?;
    let mut bytes = [0; 1400];
    let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
    assert!(states::write_qw_player(
        &mut writer,
        31,
        &[0; states::QW_PLAYER_WORDS],
        ZERO_QW
    )?);
    let mut codec = Commands::load(Protocol::QuakeWorld28);
    for (channel, model) in [(&mut client.channel, 42), (&mut other, 88)] {
        let length = codec.stage(writer.bytes())?;
        let mut count = 0;
        assert_eq!(
            codec.decode_output(length, 65, channel, |incoming| {
                let Incoming::PlayerInfo(info) = incoming else {
                    panic!("player record")
                };
                assert_eq!(info.words[8], model);
                assert_eq!(info.command, ZERO_QW);
                count += 1;
            })?,
            None
        );
        assert_eq!(count, 1);
    }
    Ok(())
}

#[test]
fn channel_qw_server_selects_native_slots_and_preserves_the_request_byte() -> Result<(), Error> {
    let mut server = Channel::load(Protocol::QuakeWorld28.channel(), Endpoint::Server, 8192, 16)
        .map_err(|_| Error::Context)?;
    server.configure_snapshots(Protocol::QuakeWorld28)?;
    let mut bytes = [0; 1400];
    for (sequence, requests) in [
        (256, &[][..]),
        (
            257,
            &[
                (Some(0), Some(0)),
                (Some(64), Some(64)),
                (Some(192), Some(192)),
                (Some(32), None),
                (None, None),
            ][..],
        ),
        (288, &[][..]),
        (289, &[(Some(0), None), (Some(32), Some(32))][..]),
    ] {
        server.publish_snapshot(snapshots::ReceivedFrame::QuakeWorld(Frame {
            sequence,
            time: ThinkTime::Milliseconds(i64::from(0)),
            command: 0,
            flags: 0,
            areas: &[],
            player: &[],
            entities: &[qw_entity(3, sequence as f32)],
        }))?;
        for &(request, advisory) in requests {
            let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
            server.write_snapshot(&mut writer, sequence, request, 0)?;
            assert_eq!(writer.bytes()[0], if advisory.is_some() { 48 } else { 47 });
            if let Some(advisory) = advisory {
                assert_eq!(writer.bytes()[1], advisory);
            }
        }
    }
    Ok(())
}

#[test]
fn channel_snapshot_publication_retains_pending_frames_and_rejects_wrong_protocols()
-> Result<(), Error> {
    let mut server = Channel::load(Protocol::QuakeWorld28.channel(), Endpoint::Server, 8192, 16)
        .map_err(|_| Error::Context)?;
    server.configure_snapshots(Protocol::QuakeWorld28)?;
    let entities = [qw_entity(3, 1.)];
    let frame = Frame {
        sequence: 1,
        time: ThinkTime::Milliseconds(i64::from(0)),
        command: 0,
        flags: 0,
        areas: &[],
        player: &[],
        entities: &entities,
    };
    server.publish_snapshot(snapshots::ReceivedFrame::QuakeWorld(frame))?;
    let mut bytes = [0; 1400];
    let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
    server.write_snapshot(&mut writer, 1, None, 0)?;
    let packet = server
        .prepare_move(writer.bytes(), EventTime(1), None)
        .map_err(|_| Error::Context)?
        .ok_or(Error::Context)?;
    let pending = packet.bytes.to_vec();
    let changed = [qw_entity(3, 999.)];
    assert!(
        server
            .publish_snapshot(snapshots::ReceivedFrame::QuakeWorld(Frame {
                entities: &changed,
                ..frame
            }))
            .is_err()
    );
    let Some(snapshots::ReceivedFrame::QuakeWorld(retained)) = server.snapshot(1) else {
        panic!("retained frame")
    };
    assert_eq!(retained.entities, &entities);
    assert_eq!(
        server.pending_packet().ok_or(Error::Context)?.bytes,
        pending
    );
    server.submitted(EventTime(2)).map_err(|_| Error::Context)?;
    server.publish_snapshot(snapshots::ReceivedFrame::QuakeWorld(Frame {
        sequence: 2,
        entities: &changed,
        ..frame
    }))?;
    let foreign = snapshots::ReceivedFrame::Quake2(Frame {
        sequence: 3,
        time: ThinkTime::Milliseconds(i64::from(0)),
        command: 0,
        flags: 0,
        areas: &[],
        player: &[0; states::Q2_PLAYER_WORDS],
        entities: &[],
    });
    assert!(server.publish_snapshot(foreign).is_err());
    assert!(server.snapshot(3).is_none());
    assert!(server.configure_snapshots(Protocol::Quake2_34).is_err());
    let mut client = Channel::load(Protocol::QuakeWorld28.channel(), Endpoint::Client, 8192, 16)
        .map_err(|_| Error::Context)?;
    client.configure_snapshots(Protocol::QuakeWorld28)?;
    assert!(
        client
            .publish_snapshot(snapshots::ReceivedFrame::QuakeWorld(frame))
            .is_err()
    );
    assert!(client.write_snapshot(&mut writer, 1, None, 0).is_err());
    assert!(client.snapshot(1).is_none());
    Ok(())
}

#[test]
fn channel_server_publication_uses_the_existing_q2_and_q3_writers() -> Result<(), Error> {
    for protocol in [Protocol::Quake2_34, Protocol::Quake3_68] {
        let mut server = Channel::load(protocol.channel(), Endpoint::Server, 8192, 16)
            .map_err(|_| Error::Context)?;
        server.configure_snapshots(protocol)?;
        let q2_player = [0; states::Q2_PLAYER_WORDS];
        let q3_player = [0; PLAYER_WORDS];
        let mut q2 = snapshots::Q2Ring::load(8, 1024, 32, None)?;
        let mut q3 = snapshots::Q3Ring::load(8, 1024, 32, None)?;
        for sequence in [1, 2] {
            let frame = match protocol {
                Protocol::Quake2_34 => {
                    let frame = Frame {
                        sequence,
                        time: ThinkTime::Milliseconds(i64::from(123)),
                        command: 0,
                        flags: 1,
                        areas: &[0x81],
                        player: &q2_player,
                        entities: &[],
                    };
                    q2.store(frame)?;
                    snapshots::ReceivedFrame::Quake2(frame)
                }
                Protocol::Quake3_68 => {
                    let frame = Frame {
                        sequence,
                        time: ThinkTime::Milliseconds(i64::from(123)),
                        command: 0,
                        flags: 1,
                        areas: &[0x81],
                        player: &q3_player,
                        entities: &[],
                    };
                    q3.store(frame)?;
                    snapshots::ReceivedFrame::Quake3(frame)
                }
                _ => unreachable!("Q2/Q3 fixture"),
            };
            server.publish_snapshot(frame)?;
            let encoding = if protocol == Protocol::Quake3_68 {
                Encoding::Q3
            } else {
                Encoding::Bytes
            };
            let mut actual = [0; 1400];
            let mut expected = [0; 1400];
            let mut writer = Writer::new(&mut actual, encoding);
            let mut reference = Writer::new(&mut expected, encoding);
            let request = (sequence == 2).then_some(1);
            server.write_snapshot(&mut writer, sequence, request, 16)?;
            match protocol {
                Protocol::Quake2_34 => {
                    snapshots::write_q2(&mut reference, &q2, sequence, request, 16)?
                }
                Protocol::Quake3_68 => snapshots::write_q3(&mut reference, &q3, sequence, request)?,
                _ => unreachable!("Q2/Q3 fixture"),
            }
            assert_eq!(writer.bit_position(), reference.bit_position());
            assert_eq!(writer.bytes(), reference.bytes());
            assert_eq!(
                server.snapshot(sequence).ok_or(Error::Context)?.sequence(),
                sequence
            );
        }
    }
    Ok(())
}

#[test]
fn nq_connected_snapshots_preserve_fractional_time_baselines_and_common_player_import()
-> Result<(), Error> {
    let (mut server, _, mut connections) = byte_connection(Protocol::NetQuake15)?;
    let mut baseline = [0; states::NQ_ENTITY_WORDS];
    baseline[0] = 7;
    baseline[2] = 1;
    baseline[5] = 12.25f32.to_bits();
    assert!(server.set_snapshot_baseline(256, &baseline));
    let client = connections
        .get_mut(ClientId(0), Endpoint::Client)
        .ok_or(Error::Context)?;
    assert!(client.channel.set_snapshot_baseline(256, &baseline));
    client.channel.set_nq_weapon_mask(true)?;
    let projection = PlayerProjection::load(Protocol::NetQuake15, &[]);
    let mut imported = PlayerState {
        movement_rules: RuleSetId::Quake3,
        trace_rules: RuleSetId::Quake2,
        ..Default::default()
    };
    let mut source = PlayerState {
        health: -17,
        armor: 55,
        ..Default::default()
    };
    source.body.velocity.0[0] = -32.;
    source.view_offset.0[2] = 22.;
    let mut context = native_player_context();
    let mut player = [0; states::NQ_PLAYER_WORDS];
    assert!(projection.reduce(&source, &context, &mut player));
    player[18] = 3;
    let mut entities = [snapshots::Entity {
        number: 256,
        words: baseline,
    }];
    entities[0].words[0] = 7.0f32.to_bits();
    entities[0].words[2] = 1.0f32.to_bits();
    entities[0].words[11] = 1;
    let seconds = 1.234567f32;
    let sequence = server.send_state().datagram_sequence;
    server.publish_snapshot(snapshots::ReceivedFrame::NetQuake(Frame {
        sequence,
        time: ThinkTime::Seconds(f64::from(seconds)),
        command: 0,
        flags: 0,
        areas: &[],
        player: &player,
        entities: &entities,
    }))?;
    let mut payload = [0; 1400];
    let mut writer = Writer::new(&mut payload, Encoding::Bytes);
    server.write_snapshot(&mut writer, sequence, None, 0)?;
    writer.write_bits(26, 8)?;
    writer.write_data(b"NetQuake\0")?;
    assert_eq!(
        &writer.bytes()[..5],
        &[
            4,
            seconds.to_bits() as u8,
            (seconds.to_bits() >> 8) as u8,
            (seconds.to_bits() >> 16) as u8,
            (seconds.to_bits() >> 24) as u8
        ]
    );
    let mut snapshots = 0;
    let mut prints = 0;
    server_payload(
        &mut server,
        &mut connections,
        writer.bytes(),
        |_, _, incoming| match incoming {
            Incoming::Snapshot(snapshots::ReceivedFrame::NetQuake(frame)) => {
                assert_eq!(frame.sequence, sequence);
                assert_eq!(frame.time, ThinkTime::Seconds(f64::from(seconds)));
                assert_eq!(frame.entities.len(), 1);
                assert_eq!(frame.entities[0].number, 256);
                assert_eq!(frame.entities[0].words[0], 7);
                assert_eq!(frame.entities[0].words[2], 1);
                assert_eq!(frame.entities[0].words[5], baseline[5]);
                assert_eq!(frame.entities[0].words[11], 1);
                assert_eq!(frame.player[18], 8);
                assert!(projection.apply(frame.player, &mut imported, &mut context, |_| None));
                assert_eq!((imported.health, imported.armor), (-17, 55));
                assert_eq!(imported.body.velocity.0[0], -32.);
                assert_eq!(imported.movement_rules, RuleSetId::Quake3);
                assert_eq!(imported.trace_rules, RuleSetId::Quake2);
                snapshots += 1;
            }
            Incoming::Print(print) => {
                assert_eq!(print.text, b"NetQuake");
                prints += 1;
            }
            _ => panic!("NQ snapshot/print stream"),
        },
    )?;
    assert_eq!((snapshots, prints, connections.command_errors), (1, 1, 0));
    assert!(!server.set_snapshot_baseline(256, &baseline));
    Ok(())
}

#[test]
fn nq_reliable_service_stream_waits_for_native_ack_after_client_publication() -> Result<(), Error> {
    let (mut server, _, mut connections) = byte_connection(Protocol::NetQuake15)?;
    let mut payload = [0; 1400];
    let mut writer = Writer::new(&mut payload, Encoding::Bytes);
    writer.write_bits(4, 8)?;
    writer.write_bits(1.0625f32.to_bits(), 32)?;
    let mut player = [0; states::NQ_PLAYER_WORDS];
    player[0] = 22.0f32.to_bits();
    player[12] = (-17.0f32).to_bits();
    states::write_nq_player(&mut writer, &player)?;
    writer.write_bits(8, 8)?;
    writer.write_data(b"reliable\0")?;
    let receipt = server
        .queue_reliable(writer.bytes())
        .map_err(|_| Error::Context)?;
    let packet = server
        .prepare_output(EventTime(1))
        .map_err(|_| Error::Context)?
        .ok_or(Error::Context)?;
    let mut bytes = [0; 1400];
    let length = packet.bytes.len();
    bytes[..length].copy_from_slice(packet.bytes);
    server.submitted(EventTime(1)).map_err(|_| Error::Context)?;
    assert!(server.reliable_receipts().is_empty());
    let mut seen = 0;
    connections.receive(
        Endpoint::Client.socket(),
        Peer::Loopback(ClientId(0)),
        &bytes[..length],
        EventTime(2),
        |_, _, incoming| match incoming {
            Incoming::Snapshot(snapshots::ReceivedFrame::NetQuake(frame)) => {
                assert_eq!(frame.time, ThinkTime::Seconds(1.0625));
                assert_eq!(frame.player[12] as i32, -17);
                seen += 1;
            }
            Incoming::Print(print) => {
                assert_eq!(print.text, b"reliable");
                seen += 1;
            }
            _ => panic!("NQ reliable service stream"),
        },
    );
    assert_eq!((seen, connections.command_errors), (2, 0));
    assert!(server.reliable_receipts().is_empty());
    let client = connections
        .get_mut(ClientId(0), Endpoint::Client)
        .ok_or(Error::Context)?;
    let ack = client
        .channel
        .prepare_output(EventTime(3))
        .map_err(|_| Error::Context)?
        .ok_or(Error::Context)?;
    let length = ack.bytes.len();
    bytes[..length].copy_from_slice(ack.bytes);
    client
        .channel
        .submitted(EventTime(3))
        .map_err(|_| Error::Context)?;
    server
        .receive(&bytes[..length], EventTime(4))
        .map_err(|_| Error::Context)?;
    assert_eq!(server.reliable_receipts(), &[receipt]);
    assert_eq!(server.send_state().reliable_bytes, 0);
    Ok(())
}

#[test]
fn nq_native_entity_order_duplicates_and_message_times_use_the_shared_stamp_set()
-> Result<(), Error> {
    let (mut server, _, mut connections) = byte_connection(Protocol::NetQuake15)?;
    let mut entity = [0; states::NQ_ENTITY_WORDS];
    entity[0] = 1.0f32.to_bits();
    let zero = [0; states::NQ_ENTITY_WORDS];
    for packet in 0..4 {
        let mut payload = [0; 1400];
        let mut writer = Writer::new(&mut payload, Encoding::Bytes);
        if packet == 0 {
            writer.write_bits(4, 8)?;
            writer.write_bits(1.5f32.to_bits(), 32)?;
            entity[5] = 10.0f32.to_bits();
            states::write_nq_entity(&mut writer, 5, &zero, &entity, false)?;
            states::write_nq_entity(&mut writer, 2, &zero, &entity, false)?;
            entity[5] = 20.0f32.to_bits();
            states::write_nq_entity(&mut writer, 5, &zero, &entity, false)?;
        } else if packet == 1 {
            writer.write_bits(1, 8)?;
            // Without a changed native time, unmentioned visible entities persist.
            states::write_nq_entity(&mut writer, 2, &zero, &entity, false)?;
        } else if packet == 2 {
            // Updates preceding a new svc_time still have the old msgtime.
            states::write_nq_entity(&mut writer, 5, &zero, &entity, false)?;
            writer.write_bits(4, 8)?;
            writer.write_bits(2.25f32.to_bits(), 32)?;
            states::write_nq_entity(&mut writer, 7, &zero, &entity, true)?;
        } else {
            writer.write_bits(26, 8)?;
            writer.write_data(b"print only\0")?;
        }
        let mut seen = 0;
        server_payload(
            &mut server,
            &mut connections,
            writer.bytes(),
            |_, _, incoming| {
                if let Incoming::Snapshot(snapshots::ReceivedFrame::NetQuake(frame)) = incoming {
                    if packet < 2 {
                        assert_eq!(
                            frame.entities.iter().map(|e| e.number).collect::<Vec<_>>(),
                            [2, 5]
                        );
                        assert_eq!(frame.entities[1].words[5], 20.0f32.to_bits());
                        assert_eq!(frame.time, ThinkTime::Seconds(1.5));
                    } else {
                        assert_eq!(frame.entities.len(), 1);
                        assert_eq!(frame.entities[0].number, 7);
                        assert_eq!(frame.entities[0].words[11], 1);
                        assert_eq!(frame.time, ThinkTime::Seconds(2.25));
                    }
                    seen += 1;
                }
            },
        )?;
        assert_eq!(seen, usize::from(packet != 3));
    }
    assert_eq!(connections.command_errors, 0);
    Ok(())
}

#[test]
fn nq_truncated_and_invalid_streams_do_not_publish_or_replace_an_accepted_frame()
-> Result<(), Error> {
    let (mut server, _, mut connections) = byte_connection(Protocol::NetQuake15)?;
    let mut bytes = [0; 1400];
    let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
    writer.write_bits(4, 8)?;
    writer.write_bits(1.25f32.to_bits(), 32)?;
    let mut entity = [0; states::NQ_ENTITY_WORDS];
    entity[0] = 1.0f32.to_bits();
    entity[5] = 12.0f32.to_bits();
    states::write_nq_entity(
        &mut writer,
        1,
        &[0; states::NQ_ENTITY_WORDS],
        &entity,
        false,
    )?;
    let valid = writer.bytes().to_vec();
    server_payload(&mut server, &mut connections, &valid, |_, _, _| {})?;
    let mut rejected = 0;
    for end in 1..valid.len() {
        if end == 5 {
            continue;
        } // A complete standalone svc_time is native-valid.
        server_payload(
            &mut server,
            &mut connections,
            &valid[..end],
            |_, _, incoming| {
                assert!(!matches!(incoming, Incoming::Snapshot(_)));
            },
        )?;
        rejected += 1;
    }
    let mut invalid = [0; 1400];
    let mut writer = Writer::new(&mut invalid, Encoding::Bytes);
    states::write_nq_entity(
        &mut writer,
        600,
        &[0; states::NQ_ENTITY_WORDS],
        &entity,
        false,
    )?;
    server_payload(
        &mut server,
        &mut connections,
        writer.bytes(),
        |_, _, incoming| {
            assert!(!matches!(incoming, Incoming::Snapshot(_)));
        },
    )?;
    assert_eq!(connections.command_errors, rejected + 1);
    let client = connections
        .get(ClientId(0), Endpoint::Client)
        .ok_or(Error::Context)?;
    let Some(snapshots::ReceivedFrame::NetQuake(frame)) = client.channel.snapshot(0) else {
        panic!("retained NQ frame")
    };
    assert_eq!(frame.time, ThinkTime::Seconds(1.25));
    assert_eq!(frame.entities[0].words[5], 12.0f32.to_bits());
    Ok(())
}
