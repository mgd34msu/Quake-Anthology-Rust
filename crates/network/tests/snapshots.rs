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
mod support;

fn native_player_context() -> PlayerContext {
    PlayerContext {
        namespace: None,
        client_number: None,
        ground_number: None,
        appearance: None,
        weapon_number: None,

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
        qw_peers: 0,
        qw_dead: 0,
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
            qw_peers: 0,
            qw_dead: 0,
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
            qw_peers: 0,
            qw_dead: 0,
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
        qw_peers: 0,
        qw_dead: 0,
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
        server.encode_server_output(&mut payload, |w, _| snapshots::write_q3(w, &ring, 1, None))?;
    let mut connections = Connections::load(1);
    connections
        .bind(
            ClientId(0),
            Endpoint::Client,
            Connection {
                phase: qa_network::ingress::ConnectionPhase::Connected,
                route: Route {
                    socket: Endpoint::Client.socket(),
                    peer: Peer::Loopback(ClientId(0)),
                },
                channel: Channel::load(QUAKE3, Endpoint::Client, 8192, 16)
                    .map_err(|_| Error::Context)?,
                commands: Some(Commands::load(Protocol::Quake3_68, 8192)),
                output: None,
                pending_output: None,
            },
        )
        .map_err(|_| Error::Context)?;
    let Some(packet) = server
        .prepare_move(&payload[..n], EventTime(1), None, None)
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
                assert!(projection.apply(
                    frame.player,
                    &mut imported,
                    &mut native,
                    qa_core::primitives::ResourceNamespace::Client(qa_core::primitives::ClientId(
                        0
                    )),
                    |_| None
                ));
                assert_eq!(imported.body.position.0[0], 0.5);
                assert_eq!(imported.health, 100);
                assert_eq!(imported.movement.command_time_ms, 50);
                assert_eq!(imported.movement_rules, RuleSetId::Quake2);
                assert_eq!(imported.trace_rules, RuleSetId::Quake);
                snapshots += 1;
            } else if let Incoming::ReliableCommand { .. } = incoming {
                reliable += 1;
            };
            std::ops::ControlFlow::Continue(())
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
    let n = server.encode_server_output(&mut payload, |writer, _| {
        if let Some((ring, from)) = snapshot {
            snapshots::write_q3(writer, ring, sequence, from)?;
        }
        Ok(())
    })?;
    let mut bytes = [0; 1400];
    let packet = server
        .prepare_move(&payload[..n], EventTime(1), None, None)
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
        .decode_output(
            n,
            sequence,
            client,
            qa_core::primitives::ResourceNamespace::Client(qa_core::primitives::ClientId(0)),
            |_| {},
        )
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
    let n = client_commands.encode(&command, |_| 0, client, &mut bytes)?;
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
    let accepted = server_commands.decode(
        staged,
        client.send_state().sequence,
        time,
        server,
        None,
        |incoming| {
            if let qa_network::ingress::Incoming::Command { .. } = incoming {}
            std::ops::ControlFlow::Continue(())
        },
    )? > 0;
    Ok((delta, accepted))
}

#[test]
fn native_snapshot_request_tracks_message_ack_before_stale_command_filtering() -> Result<(), Error>
{
    let mut server =
        Channel::load(QUAKE3, Endpoint::Server, 8192, 16).map_err(|_| Error::Context)?;
    let mut client =
        Channel::load(QUAKE3, Endpoint::Client, 8192, 16).map_err(|_| Error::Context)?;
    let mut incoming = Commands::load(Protocol::Quake3_68, 8192);
    let outgoing = Commands::load(Protocol::Quake3_68, 8192);
    let mut client_output = Commands::load(Protocol::Quake3_68, 8192);
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
    let mut incoming = Commands::load(Protocol::Quake3_68, 8192);
    let outgoing = Commands::load(Protocol::Quake3_68, 8192);
    let mut client_output = Commands::load(Protocol::Quake3_68, 8192);
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
        qw_peers: 0,
        qw_dead: 0,
        player: &player,
        areas: &[!0x81, !0x42],
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
    assert_eq!(frame.areas, &[!0x81, !0x42]);
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
    mut consume: impl FnMut(ClientId, Endpoint, Incoming<'_>),
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
        .prepare_move(writer.bytes(), EventTime(1), None, None)
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
            |id, endpoint, incoming| {
                consume(id, endpoint, incoming);
                std::ops::ControlFlow::Continue(())
            },
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
    let n = codec.encode(&command, |_| 0, &client.channel, &mut bytes)?;
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
    receiver.decode(
        staged,
        client.channel.send_state().sequence,
        0,
        server,
        None,
        |incoming| {
            if let qa_network::ingress::Incoming::Command { .. } = incoming {}
            std::ops::ControlFlow::Continue(())
        },
    )?;
    let expected = (last_frame >= 0).then_some(last_frame as u32);
    assert_eq!(receiver.delta_request(), expected);
    Ok(expected)
}

#[test]
fn q2_connected_frames_use_payload_numbers_and_invalid_frames_request_full() -> Result<(), Error> {
    let policy = Protocol::Quake2_34.channel();
    let mut server =
        Channel::load(policy, Endpoint::Server, 8192, 16).map_err(|_| Error::Context)?;
    let mut commands = Commands::load(Protocol::Quake2_34, 8192);
    let mut connections = Connections::load(1);
    connections
        .bind(
            ClientId(0),
            Endpoint::Client,
            Connection {
                phase: qa_network::ingress::ConnectionPhase::Connected,
                route: Route {
                    socket: Endpoint::Client.socket(),
                    peer: Peer::Loopback(ClientId(0)),
                },
                channel: Channel::load(policy, Endpoint::Client, 8192, 16)
                    .map_err(|_| Error::Context)?,
                commands: Some(Commands::load(Protocol::Quake2_34, 8192)),
                output: None,
                pending_output: None,
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
                            qa_core::primitives::ResourceNamespace::Client(
                                qa_core::primitives::ClientId(0)
                            ),
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
                    Incoming::CommandProgress(_) => {}
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
    let mut commands = Commands::load(Protocol::Quake2_34, 8192);
    let mut ring = snapshots::Q2Ring::load(8, 1024, 32, None)?;
    q2_store(&mut ring, 41, &[q2_entity(30, 7., 0)])?;
    let mut payload = [0; 1400];
    let mut writer = Writer::new(&mut payload, Encoding::Bytes);
    snapshots::write_q2(&mut writer, &ring, 41, None, 16)?;
    let n = commands.stage(writer.bytes())?;
    assert_eq!(
        commands.decode_output(
            n,
            1,
            &mut client,
            qa_core::primitives::ResourceNamespace::Client(qa_core::primitives::ClientId(0)),
            |_| {}
        )?,
        Some(41)
    );
    q2_store(&mut ring, 42, &[q2_entity(30, 8., 0)])?;
    let mut writer = Writer::new(&mut payload, Encoding::Bytes);
    snapshots::write_q2(&mut writer, &ring, 42, Some(41), 16)?;
    let length = writer.size();
    for cut in 1..length {
        let n = commands.stage(&payload[..cut])?;
        assert!(
            commands
                .decode_output(
                    n,
                    2,
                    &mut client,
                    qa_core::primitives::ResourceNamespace::Client(qa_core::primitives::ClientId(
                        0
                    )),
                    |_| {}
                )
                .is_err()
        );
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
        assert!(
            commands
                .decode_output(
                    n,
                    3,
                    &mut client,
                    qa_core::primitives::ResourceNamespace::Client(qa_core::primitives::ClientId(
                        0
                    )),
                    |_| {}
                )
                .is_err()
        );
    }
    let n = commands.stage(&payload[..length])?;
    assert_eq!(
        commands.decode_output(
            n,
            4,
            &mut client,
            qa_core::primitives::ResourceNamespace::Client(qa_core::primitives::ClientId(0)),
            |_| {}
        )?,
        Some(42)
    );
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
        qw_peers: 0,
        qw_dead: 0,
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
        units: qa_gameplay::rules::command_units(qa_core::primitives::RuleSetId::QuakeWorld),
        ..Default::default()
    };
    let mut payload = [0; 1400];
    let length = codec.encode(&command, |_| 0, &client.channel, &mut payload)?;
    let request = codec.snapshot_request(&client.channel);
    let packet = if let Some(pending) = client.channel.pending_packet() {
        pending
    } else {
        client
            .channel
            .prepare_move(&payload[..length], EventTime(1), request, None)
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
        let mut command = None;
        commands.decode(length, sequence, 0, server, None, |incoming| {
            if let qa_network::ingress::Incoming::Command {
                command: current,
                native_weapon: _,
                observation: _,
                ..
            } = incoming
            {
                command = Some(current);
            }
            std::ops::ControlFlow::Continue(())
        })?;
        let command = command.ok_or(Error::Context)?;
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
    mut consume: impl FnMut(ClientId, Endpoint, Incoming<'_>),
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
        .prepare_move(&payload[..length], EventTime(2), None, None)
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
            |id, endpoint, incoming| {
                consume(id, endpoint, incoming);
                std::ops::ControlFlow::Continue(())
            },
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
                phase: qa_network::ingress::ConnectionPhase::Connected,
                route: Route {
                    socket: Endpoint::Client.socket(),
                    peer: Peer::Loopback(ClientId(0)),
                },
                channel: Channel::load(protocol.channel(), Endpoint::Client, 8192, 16)
                    .map_err(|_| Error::Context)?,
                output: None,
                pending_output: None,
                commands: Some(Commands::load(protocol, 8192)),
            },
        )
        .map_err(|_| Error::Context)?;
    let mut server = Channel::load(protocol.channel(), Endpoint::Server, 8192, 16)
        .map_err(|_| Error::Context)?;
    server.configure_snapshots(protocol)?;
    Ok((server, Commands::load(protocol, 8192), connections))
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
                    Incoming::CommandProgress(_) => {}
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
        units: qa_gameplay::rules::command_units(qa_core::primitives::RuleSetId::QuakeWorld),
        ..Default::default()
    };
    let mut payload = [0; 1400];
    let n = codec.encode(&command, |_| 0, &client.channel, &mut payload)?;
    let request = codec.snapshot_request(&client.channel);
    assert_eq!(request, Some(4));
    client
        .channel
        .prepare_move(&payload[..n], EventTime(4), request, None)
        .map_err(|_| Error::Context)?;
    // No transport admission yet: a delta response to that unsent request must
    // not borrow a conveniently retained base and invent an accepted frame.
    qw_store(&mut ring, 5, &[qw_entity(3, 5.)])?;
    assert!(
        server
            .prepare_output(EventTime(4))
            .map_err(|_| Error::Context)?
            .is_none()
    );
    // A misbehaving peer can still send an unsolicited native response. The
    // production SERVER refuses it; CLIENT must also reject its unsubmitted base.
    let mut response = [0; 1400];
    let mut writer = Writer::new(&mut response, Encoding::Bytes);
    snapshots::write_qw(&mut writer, &ring, 5, Some((4, 4)))?;
    let mut packet = [0; 1400];
    let length = qa_network::headers::encode(
        qa_network::headers::QUAKEWORLD,
        qa_network::headers::Direction::ToClient,
        qa_network::headers::Header {
            sequence: 5,
            ..Default::default()
        },
        writer.bytes(),
        &mut packet,
    )
    .map_err(|_| Error::Context)?;
    connections.receive(
        Endpoint::Client.socket(),
        Peer::Loopback(ClientId(0)),
        &packet[..length],
        EventTime(4),
        |_, _, incoming| {
            assert!(!matches!(incoming, Incoming::Snapshot(_)));
            std::ops::ControlFlow::Continue(())
        },
    );
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
    assert_eq!(server.send_state().sequence, 5);
    assert!(
        server
            .prepare_output(EventTime(5))
            .map_err(|_| Error::Context)?
            .is_some()
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
            };
            std::ops::ControlFlow::Continue(())
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
    mut consume: impl FnMut(ClientId, Endpoint, Incoming<'_>),
) -> Result<(), Error> {
    let packet = server
        .prepare_move(payload, EventTime(2), None, None)
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
        |id, endpoint, incoming| {
            consume(id, endpoint, incoming);
            std::ops::ControlFlow::Continue(())
        },
    );
    Ok(())
}

fn nq_reliable_payload(
    server: &mut Channel,
    connections: &mut Connections,
    payload: &[u8],
    mut consume: impl FnMut(ClientId, Endpoint, Incoming<'_>),
) -> Result<(), Error> {
    let receipt = server.queue_reliable(payload).map_err(|_| Error::Context)?;
    loop {
        let time = server.send_state().last_sent.0 + 4;
        let packet = server
            .prepare_output(EventTime(time))
            .map_err(|_| Error::Context)?
            .ok_or(Error::Context)?;
        let mut bytes = [0; 1400];
        let length = packet.bytes.len();
        bytes[..length].copy_from_slice(packet.bytes);
        server
            .submitted(EventTime(time))
            .map_err(|_| Error::Context)?;
        let remaining = server.send_state().reliable_bytes;
        connections.receive(
            Endpoint::Client.socket(),
            Peer::Loopback(ClientId(0)),
            &bytes[..length],
            EventTime(time + 1),
            |id, endpoint, incoming| {
                consume(id, endpoint, incoming);
                std::ops::ControlFlow::Continue(())
            },
        );
        assert!(!server.reliable_receipts().contains(&receipt));
        let client = connections
            .get_mut(ClientId(0), Endpoint::Client)
            .ok_or(Error::Context)?;
        let ack = client
            .channel
            .prepare_output(EventTime(time + 2))
            .map_err(|_| Error::Context)?
            .ok_or(Error::Context)?;
        let length = ack.bytes.len();
        bytes[..length].copy_from_slice(ack.bytes);
        client
            .channel
            .submitted(EventTime(time + 2))
            .map_err(|_| Error::Context)?;
        server
            .receive(&bytes[..length], EventTime(time + 3))
            .map_err(|_| Error::Context)?;
        assert!(server.send_state().reliable_bytes < remaining);
        if server.send_state().reliable_bytes == 0 {
            assert_eq!(server.reliable_receipts(), &[receipt]);
            return Ok(());
        }
    }
}

fn q1_service_payload(
    protocol: Protocol,
    server: &mut Channel,
    commands: &mut Commands,
    connections: &mut Connections,
    payload: &[u8],
    consume: impl FnMut(ClientId, Endpoint, Incoming<'_>),
) -> Result<(), Error> {
    if protocol == Protocol::QuakeWorld28 {
        qw_move(connections, server, commands, true)?;
    }
    server_payload(server, connections, payload, consume)
}

fn q1_frame_row<'a>(frame: snapshots::ReceivedFrame<'a>, number: u32) -> Result<&'a [u32], Error> {
    match frame {
        snapshots::ReceivedFrame::NetQuake(frame) => frame
            .entities
            .iter()
            .find(|entity| entity.number == number)
            .map(|entity| entity.words.as_slice())
            .ok_or(Error::Context),
        snapshots::ReceivedFrame::QuakeWorld(frame) => frame
            .entities
            .iter()
            .find(|entity| entity.number == number)
            .map(|entity| entity.words.as_slice())
            .ok_or(Error::Context),
        _ => Err(Error::Context),
    }
}

// Original baseline256: model7/frame3/colormap2/skin4, followed by the
// same signed coordinate/angle oracle as static_entities.rs's literal.
const Q1_BASELINE: [u8; 16] = [22, 0, 1, 7, 3, 2, 4, 247, 255, 255, 162, 0, 64, 1, 4, 128];
const NQ_BASELINE_UPDATE: &[u8] = b"\x07\0\0\x80\x3f\x81\x40\0\x01";
const QW_BASELINE_UPDATE: &[u8] = b"\x2f\0\x01\0\0";
const NQ_SERVERINFO: &[u8] = b"\x0b\x0f\0\0\0\x10\xfeLev\x80el\0maps/a.bsp\0*1\0\0misc/a.wav\0\0";

fn assert_q1_baseline_row(words: &[u32], model: u32) {
    assert_eq!(&words[..5], &[model, 3, 2, 4, 0]);
    assert_eq!(
        &words[5..11],
        &[-1.125f32, -1.40625, 20.25, 90., 128.125, -180.].map(f32::to_bits)
    );
    assert!(words[11..].iter().all(|&word| word == 0));
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
                            qa_core::primitives::ResourceNamespace::Client(
                                qa_core::primitives::ClientId(0)
                            ),
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
                    Incoming::CommandProgress(_) => {}
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
fn qw_accepted_playerinfo_shares_the_snapshot_ring_and_expires_without_a_new_record()
-> Result<(), Error> {
    use qa_core::primitives::{ResourceNamespace, Vec3};
    use qa_network::{
        commands::packet::ZERO_QW,
        projection::{EntityProjection, EntityRecord},
    };

    let (mut server, mut commands, mut connections) = byte_connection(Protocol::QuakeWorld28)?;
    let mut ring = snapshots::QwRing::load(64, 512, 0, None)?;
    let projection = EntityProjection::load(Protocol::QuakeWorld28);
    for sequence in 1..=2 {
        let request = qw_move(&mut connections, &mut server, &mut commands, true)?;
        assert_eq!(request, (sequence == 2).then_some(1));
        let packet_entity = qw_entity(40, sequence as f32);
        qw_store(&mut ring, sequence, &[packet_entity])?;
        let mut payload = [0; 1400];
        let mut writer = Writer::new(&mut payload, Encoding::Bytes);
        if sequence == 1 {
            let mut words = [0; states::QW_PLAYER_WORDS];
            words[0] = 48.0f32.to_bits();
            words[1] = 12.0f32.to_bits();
            words[2] = 24.0f32.to_bits();
            words[3] = 17.0f32.to_bits();
            words[5] = 25.0f32.to_bits();
            words[6] = 100.0f32.to_bits();
            words[7] = (-50.0f32).to_bits();
            words[8] = 5.0f32.to_bits();
            words[9] = 7.0f32.to_bits();
            words[10] = 2.0f32.to_bits();
            words[12] = (1 << 0)
                | (1 << 1)
                | (1 << 2)
                | (1 << 3)
                | (1 << 4)
                | (1 << 5)
                | (1 << 6)
                | (1 << 7);
            let command = qa_network::commands::QwCmd {
                view_angles: Vec3([90., 45., 0.]),
                ..ZERO_QW
            };
            assert!(states::write_qw_player(&mut writer, 1, &words, command)?);
        }
        snapshots::write_qw(
            &mut writer,
            &ring,
            sequence,
            request.map(|base| (base, base as u8)),
        )?;
        server_payload(&mut server, &mut connections, writer.bytes(), |_, _, _| {})?;
        let channel = &connections
            .get(ClientId(0), Endpoint::Client)
            .ok_or(Error::Context)?
            .channel;
        let Some(snapshots::ReceivedFrame::QuakeWorld(frame)) = channel.current_snapshot() else {
            return Err(Error::Context);
        };
        assert_eq!(frame.sequence, sequence);
        assert_eq!(frame.qw_peers, if sequence == 1 { 1 << 1 } else { 0 });
        assert_eq!(frame.qw_player(2), sequence == 1);
        assert!(!frame.qw_player(0));
        assert!(!frame.qw_player(1));
        assert!(!frame.qw_player(40));
        assert_eq!(
            frame
                .entities
                .iter()
                .map(|row| row.number)
                .collect::<Vec<_>>(),
            if sequence == 1 { vec![2, 40] } else { vec![40] }
        );
        assert_eq!(
            f32::from_bits(frame.entities.last().unwrap().words[5]),
            sequence as f32
        );
        if sequence == 1 {
            let mut record = EntityRecord::default();
            assert!(projection.apply(
                &frame.entities[0].words,
                &mut record,
                ResourceNamespace::Client(ClientId(0))
            ));
            assert_eq!(record.position, Vec3([48., 12., 24.]));
            assert_eq!(record.velocity, Vec3([25., 100., -50.]));
            assert_eq!(record.angles, Vec3([90., 45., 0.]));
            assert_eq!(
                (
                    record.appearance.frame,
                    record.appearance.skin,
                    record.appearance.native.effects
                ),
                (17, 7, 2)
            );
            assert_eq!(
                record.appearance.models[0],
                qa_core::primitives::ModelRef::native(ResourceNamespace::Client(ClientId(0)), 5)
            );
            assert_eq!(record.appearance.native.colormap, 2);
        } else {
            let Some(snapshots::ReceivedFrame::QuakeWorld(previous)) = channel.previous_snapshot()
            else {
                return Err(Error::Context);
            };
            assert_eq!(previous.sequence, 1);
            assert_eq!(previous.entities[0].number, 2);
            assert_eq!(previous.qw_peers, 1 << 1);
            assert!(previous.qw_player(2));
        }
    }
    assert_eq!(connections.command_errors, 0);
    Ok(())
}

#[test]
fn accepted_qw_dead_capability_tracks_current_and_previous_playerinfo_without_a_second_history()
-> Result<(), Error> {
    let (mut server, mut commands, mut connections) = byte_connection(Protocol::QuakeWorld28)?;
    let mut ring = snapshots::QwRing::load(64, 512, 0, None)?;
    for sequence in 1..=4 {
        let request = qw_move(&mut connections, &mut server, &mut commands, true)?;
        assert_eq!(request, (sequence > 1).then_some(sequence - 1));
        qw_store(&mut ring, sequence, &[qw_entity(40, sequence as f32)])?;
        let mut payload = [0; 1400];
        let mut writer = Writer::new(&mut payload, Encoding::Bytes);
        if sequence < 4 {
            for player in [0, 31] {
                let mut words = [0; states::QW_PLAYER_WORDS];
                words[0] = (32. + player as f32).to_bits();
                words[8] = 5.0f32.to_bits();
                words[12] = 1 << 5;
                if sequence == 2 && player == 31 {
                    words[12] |= u32::from(states::QW_PLAYER_DEAD);
                }
                assert!(states::write_qw_player(
                    &mut writer,
                    player,
                    &words,
                    qa_network::commands::packet::ZERO_QW
                )?);
            }
        }
        snapshots::write_qw(
            &mut writer,
            &ring,
            sequence,
            request.map(|base| (base, base as u8)),
        )?;
        server_payload(&mut server, &mut connections, writer.bytes(), |_, _, _| {})?;
        let channel = &connections
            .get(ClientId(0), Endpoint::Client)
            .ok_or(Error::Context)?
            .channel;
        let Some(snapshots::ReceivedFrame::QuakeWorld(frame)) = channel.current_snapshot() else {
            return Err(Error::Context);
        };
        assert_eq!(frame.sequence, sequence);
        assert_eq!(frame.qw_peers, if sequence < 4 { 1 | 1 << 31 } else { 0 });
        assert_eq!(frame.qw_dead, if sequence == 2 { 1 << 31 } else { 0 });
        assert_eq!(frame.qw_player(1), sequence < 4);
        assert_eq!(frame.qw_player(32), sequence < 4);
        assert_eq!(frame.qw_dead_player(32), sequence == 2);
        for number in [0, 1, 2, 33, 40, u32::MAX] {
            assert!(!frame.qw_dead_player(number));
        }
        assert_eq!(
            frame
                .entities
                .iter()
                .map(|row| row.number)
                .collect::<Vec<_>>(),
            if sequence < 4 {
                vec![1, 32, 40]
            } else {
                vec![40]
            }
        );
        if sequence > 1 {
            let Some(snapshots::ReceivedFrame::QuakeWorld(previous)) = channel.previous_snapshot()
            else {
                return Err(Error::Context);
            };
            assert_eq!(previous.sequence, sequence - 1);
            assert_eq!(previous.qw_dead_player(32), sequence == 3);
            assert_eq!(previous.qw_dead, if sequence == 3 { 1 << 31 } else { 0 });
        }
    }
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
                if matches!(incoming, Incoming::CommandProgress(_)) {
                    return;
                }
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
    let mut codec = Commands::load(Protocol::QuakeWorld28, 8192);
    for (channel, model) in [(&mut client.channel, 42), (&mut other, 88)] {
        let length = codec.stage(writer.bytes())?;
        let mut count = 0;
        assert_eq!(
            codec.decode_output(
                length,
                65,
                channel,
                qa_core::primitives::ResourceNamespace::Client(qa_core::primitives::ClientId(0)),
                |incoming| {
                    let Incoming::PlayerInfo(info) = incoming else {
                        panic!("player record")
                    };
                    assert_eq!(info.words[8], model);
                    assert_eq!(info.command, ZERO_QW);
                    count += 1;
                }
            )?,
            None
        );
        assert_eq!(count, 1);
    }
    Ok(())
}

#[test]
fn qw_modellist_rows_select_the_playerinfo_default_before_the_next_service() -> Result<(), Error> {
    use qa_network::commands::packet::ZERO_QW;
    let (mut server, mut commands, mut connections) = byte_connection(Protocol::QuakeWorld28)?;
    let chunks: &[(&[u8], u32, &[u32])] = &[
        // Matching is byte-exact, including case and the complete filename.
        (
            b"\x2d\0progs/Player.mdl\0progs/player.mdl.bak\0\0\0",
            0,
            &[1, 2],
        ),
        // Native chunk bases precede their first row; a continuation does not
        // delay publication of the model already accepted in this packet.
        (
            b"\x2d\0maps/start.bsp\0progs/player.mdl\0\0\x02",
            2,
            &[1, 2],
        ),
        (
            b"\x2d\x02progs/ogre.mdl\0progs/player.mdl\0\0\0",
            4,
            &[3, 4],
        ),
        // Sound and lightstyle text cannot select a player model.
        (b"\x2e\0progs/player.mdl\0\0\0", 4, &[257]),
        (b"\x0c\0progs/player.mdl\0", 4, &[512]),
        // Restarting a list alone retains the hint; an exact new row replaces
        // it, and repeated exact matches retain their authored last match.
        (b"\x2d\0maps/restart.bsp\0\0\0", 4, &[1]),
        (b"\x2d\0progs/player.mdl\0\0\0", 1, &[1]),
        (
            b"\x2d\x08progs/player.mdl\0progs/eyes.mdl\0progs/player.mdl\0\0\x0b",
            11,
            &[9, 10, 11],
        ),
    ];
    for &(chunk, expected, indices) in chunks {
        qw_move(&mut connections, &mut server, &mut commands, true)?;
        let mut bytes = [0; 1400];
        let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
        writer.write_data(chunk)?;
        assert!(states::write_qw_player(
            &mut writer,
            0,
            &[0; states::QW_PLAYER_WORDS],
            ZERO_QW
        )?);
        let mut explicit = [0; states::QW_PLAYER_WORDS];
        explicit[8] = 77.0f32.to_bits();
        explicit[12] = 1 << 5; // PF_MODEL overrides the default hint.
        assert!(states::write_qw_player(&mut writer, 1, &explicit, ZERO_QW)?);
        let mut rows = Vec::new();
        let mut models = Vec::new();
        server_payload(
            &mut server,
            &mut connections,
            writer.bytes(),
            |_, _, incoming| match incoming {
                Incoming::ConfigString(row) => rows.push(row.index),
                Incoming::PlayerInfo(player) => models.push((player.number, player.words[8])),
                _ => {}
            },
        )?;
        assert_eq!(rows, indices);
        assert_eq!(models, [(0, expected), (1, 77)]);
    }
    assert_eq!(connections.command_errors, 0);
    Ok(())
}

#[test]
fn qw_modellist_retains_each_admitted_model_prefix_when_its_chunk_is_truncated() -> Result<(), Error>
{
    use qa_network::commands::packet::ZERO_QW;
    let (mut server, mut commands, mut connections) = byte_connection(Protocol::QuakeWorld28)?;
    qw_move(&mut connections, &mut server, &mut commands, true)?;
    let mut rows = Vec::new();
    server_payload(
        &mut server,
        &mut connections,
        b"\x2d\x06progs/player.mdl\0",
        |_, _, incoming| {
            if let Incoming::ConfigString(row) = incoming {
                rows.push(row.index);
            }
        },
    )?;
    assert_eq!(rows, [7]);
    assert_eq!(connections.command_errors, 1);
    qw_move(&mut connections, &mut server, &mut commands, true)?;
    let mut bytes = [0; 1400];
    let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
    assert!(states::write_qw_player(
        &mut writer,
        0,
        &[0; states::QW_PLAYER_WORDS],
        ZERO_QW
    )?);
    let mut models = Vec::new();
    server_payload(
        &mut server,
        &mut connections,
        writer.bytes(),
        |_, _, incoming| {
            if let Incoming::PlayerInfo(player) = incoming {
                models.push(player.words[8]);
            }
        },
    )?;
    assert_eq!(models, [7]);
    assert_eq!(connections.command_errors, 1);
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
            qw_peers: 0,
            qw_dead: 0,
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
    support::admit_qw_reply(&mut server, 1);
    let entities = [qw_entity(3, 1.)];
    let frame = Frame {
        sequence: 1,
        time: ThinkTime::Milliseconds(i64::from(0)),
        command: 0,
        flags: 0,
        qw_peers: 0,
        qw_dead: 0,
        areas: &[],
        player: &[],
        entities: &entities,
    };
    server.publish_snapshot(snapshots::ReceivedFrame::QuakeWorld(frame))?;
    let mut bytes = [0; 1400];
    let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
    server.write_snapshot(&mut writer, 1, None, 0)?;
    let packet = server
        .prepare_move(writer.bytes(), EventTime(1), None, None)
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
        qw_peers: 0,
        qw_dead: 0,
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
fn publication_retains_snapshot_between_native_q3_fragments() -> Result<(), Error> {
    let mut server = Channel::load(Protocol::Quake3_68.channel(), Endpoint::Server, 8192, 16)
        .map_err(|_| Error::Context)?;
    server.configure_snapshots(Protocol::Quake3_68)?;
    let mut player = [0; states::PLAYER_WORDS];
    player[48] = 100;
    let frame = Frame {
        sequence: 1,
        time: ThinkTime::Milliseconds(1001),
        command: 0,
        flags: 0,
        qw_peers: 0,
        qw_dead: 0,
        areas: &[],
        player: &player,
        entities: &[],
    };
    server.publish_snapshot(snapshots::ReceivedFrame::Quake3(frame))?;
    server
        .prepare_move(&[0x55; 2600], EventTime(0), None, None)
        .map_err(|_| Error::Context)?
        .ok_or(Error::Context)?;
    assert_eq!(
        server
            .submit_with(EventTime(0), |_| false)
            .map_err(|_| Error::Context)?,
        None
    );
    assert_eq!(server.send_state().sequence, 1);
    assert!(
        server
            .submit_with(EventTime(0), |_| true)
            .map_err(|_| Error::Context)?
            .is_some()
    );
    assert!(server.pending_packet().is_none());
    assert!(server.pending_fragments());
    assert!(
        server
            .publish_snapshot(snapshots::ReceivedFrame::Quake3(frame))
            .is_err()
    );
    for _ in 0..2 {
        server
            .prepare_output(EventTime(1))
            .map_err(|_| Error::Context)?
            .ok_or(Error::Context)?;
        assert!(
            server
                .submit_with(EventTime(1), |_| true)
                .map_err(|_| Error::Context)?
                .is_some()
        );
    }
    assert!(!server.pending_fragments());
    server.publish_snapshot(snapshots::ReceivedFrame::Quake3(Frame {
        sequence: 2,
        ..frame
    }))?;
    let Some(snapshots::ReceivedFrame::Quake3(retained)) = server.snapshot(1) else {
        return Err(Error::Context);
    };
    assert_eq!(retained.player[48], 100);
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
                        qw_peers: 0,
                        qw_dead: 0,
                        areas: &[!0x81],
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
                        qw_peers: 0,
                        qw_dead: 0,
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
            let current = server.current_snapshot().ok_or(Error::Context)?;
            assert_eq!(current.sequence(), sequence);
            assert_eq!(current.time(), ThinkTime::Milliseconds(123));
        }
    }
    Ok(())
}

#[test]
fn q1_wire_baselines_replace_future_rows_without_mutating_retained_frames() -> Result<(), Error> {
    for (protocol, update) in [
        (Protocol::NetQuake15, NQ_BASELINE_UPDATE),
        (Protocol::QuakeWorld28, QW_BASELINE_UPDATE),
    ] {
        let (mut server, mut commands, mut connections) = byte_connection(protocol)?;
        let mut payload = Q1_BASELINE.to_vec();
        payload.extend_from_slice(update);
        let mut frames = 0;
        q1_service_payload(
            protocol,
            &mut server,
            &mut commands,
            &mut connections,
            &payload,
            |_, _, incoming| match incoming {
                Incoming::Snapshot(frame) => {
                    assert_q1_baseline_row(q1_frame_row(frame, 256).unwrap(), 7);
                    frames += 1;
                }
                Incoming::CommandProgress(_) | Incoming::NqEntityUpdate => {}
                _ => panic!("baseline/update published an unrelated event"),
            },
        )?;
        assert_eq!(frames, 1);
        let old = connections
            .get(ClientId(0), Endpoint::Client)
            .ok_or(Error::Context)?
            .channel
            .current_snapshot()
            .ok_or(Error::Context)?
            .sequence();
        // Complete duplicate records replace in service order, including after
        // a snapshot exists. Neither record manufactures another snapshot.
        let mut replacement = Q1_BASELINE;
        replacement[3] = 8;
        let mut payload = replacement.to_vec();
        replacement[3] = 9;
        payload.extend_from_slice(&replacement);
        q1_service_payload(
            protocol,
            &mut server,
            &mut commands,
            &mut connections,
            &payload,
            |_, _, _| panic!("baseline-only replacement published an event"),
        )?;
        {
            let channel = &mut connections
                .get_mut(ClientId(0), Endpoint::Client)
                .ok_or(Error::Context)?
                .channel;
            let frame = channel.current_snapshot().ok_or(Error::Context)?;
            assert_eq!(frame.sequence(), old);
            assert_q1_baseline_row(q1_frame_row(frame, 256)?, 7);
            let words = q1_frame_row(frame, 256)?.to_vec();
            assert!(!channel.set_snapshot_baseline(256, &words));
        }
        q1_service_payload(
            protocol,
            &mut server,
            &mut commands,
            &mut connections,
            update,
            |_, _, incoming| match incoming {
                Incoming::Snapshot(frame) => {
                    assert_q1_baseline_row(q1_frame_row(frame, 256).unwrap(), 9)
                }
                Incoming::CommandProgress(_) | Incoming::NqEntityUpdate => {}
                _ => panic!("baseline-derived update"),
            },
        )?;
        let channel = &connections
            .get(ClientId(0), Endpoint::Client)
            .ok_or(Error::Context)?
            .channel;
        let current = channel.current_snapshot().ok_or(Error::Context)?;
        assert_ne!(current.sequence(), old);
        assert_q1_baseline_row(q1_frame_row(current, 256)?, 9);
        assert_q1_baseline_row(
            q1_frame_row(channel.snapshot(old).ok_or(Error::Context)?, 256)?,
            7,
        );
        assert_eq!(connections.command_errors, 0);
    }
    Ok(())
}

#[test]
fn qw_late_baselines_leave_matched_delta_rows_on_the_requested_old_frame() -> Result<(), Error> {
    let protocol = Protocol::QuakeWorld28;
    let (mut server, mut commands, mut connections) = byte_connection(protocol)?;
    let mut payload = Q1_BASELINE.to_vec();
    payload.extend_from_slice(QW_BASELINE_UPDATE);
    q1_service_payload(
        protocol,
        &mut server,
        &mut commands,
        &mut connections,
        &payload,
        |_, _, _| {},
    )?;
    let old = connections
        .get(ClientId(0), Endpoint::Client)
        .ok_or(Error::Context)?
        .channel
        .current_snapshot()
        .ok_or(Error::Context)?
        .sequence();
    let mut replacement = Q1_BASELINE;
    replacement[3] = 9;
    let mut payload = replacement.to_vec();
    replacement[1..3].copy_from_slice(&257u16.to_le_bytes());
    replacement[3] = 11;
    payload.extend_from_slice(&replacement);
    q1_service_payload(
        protocol,
        &mut server,
        &mut commands,
        &mut connections,
        &payload,
        |_, _, _| panic!("baseline-only replacement published an event"),
    )?;
    // Matched entity256 uses the old row; new entity257 uses its new baseline.
    let delta = [48, old as u8, 0, 1, 1, 1, 0, 0];
    let mut frames = 0;
    q1_service_payload(
        protocol,
        &mut server,
        &mut commands,
        &mut connections,
        &delta,
        |_, _, incoming| match incoming {
            Incoming::Snapshot(frame) => {
                assert_q1_baseline_row(q1_frame_row(frame, 256).unwrap(), 7);
                assert_q1_baseline_row(q1_frame_row(frame, 257).unwrap(), 11);
                frames += 1;
            }
            Incoming::CommandProgress(_) => {}
            _ => panic!("matched delta update"),
        },
    )?;
    assert_eq!(frames, 1);
    q1_service_payload(
        protocol,
        &mut server,
        &mut commands,
        &mut connections,
        QW_BASELINE_UPDATE,
        |_, _, incoming| {
            if let Incoming::Snapshot(frame) = incoming {
                assert_q1_baseline_row(q1_frame_row(frame, 256).unwrap(), 9);
                frames += 1;
            }
        },
    )?;
    assert_eq!(frames, 2);
    assert_eq!(connections.command_errors, 0);
    Ok(())
}

#[test]
fn q1_baseline_replacements_are_atomic_per_record_and_keep_accepted_prefixes() -> Result<(), Error>
{
    for (protocol, update) in [
        (Protocol::NetQuake15, NQ_BASELINE_UPDATE),
        (Protocol::QuakeWorld28, QW_BASELINE_UPDATE),
    ] {
        let (mut server, mut commands, mut connections) = byte_connection(protocol)?;
        q1_service_payload(
            protocol,
            &mut server,
            &mut commands,
            &mut connections,
            &Q1_BASELINE,
            |_, _, _| panic!("baseline-only packet published an event"),
        )?;
        let mut replacement = Q1_BASELINE;
        replacement[3..7].fill(255);
        for cut in 1..replacement.len() {
            q1_service_payload(
                protocol,
                &mut server,
                &mut commands,
                &mut connections,
                &replacement[..cut],
                |_, _, _| panic!("truncated baseline published an event"),
            )?;
        }
        q1_service_payload(
            protocol,
            &mut server,
            &mut commands,
            &mut connections,
            update,
            |_, _, incoming| {
                if let Incoming::Snapshot(frame) = incoming {
                    assert_q1_baseline_row(q1_frame_row(frame, 256).unwrap(), 7);
                }
            },
        )?;
        for (model, tail) in [(11, &replacement[..8]), (13, &[0][..])] {
            let mut accepted = Q1_BASELINE;
            accepted[3] = model;
            let mut payload = accepted.to_vec();
            payload.extend_from_slice(tail);
            q1_service_payload(
                protocol,
                &mut server,
                &mut commands,
                &mut connections,
                &payload,
                |_, _, _| panic!("baseline prefix/tail published an event"),
            )?;
            q1_service_payload(
                protocol,
                &mut server,
                &mut commands,
                &mut connections,
                update,
                |_, _, incoming| {
                    if let Incoming::Snapshot(frame) = incoming {
                        assert_q1_baseline_row(q1_frame_row(frame, 256).unwrap(), u32::from(model));
                    }
                },
            )?;
            let channel = &connections
                .get(ClientId(0), Endpoint::Client)
                .ok_or(Error::Context)?
                .channel;
            assert_q1_baseline_row(
                q1_frame_row(channel.current_snapshot().ok_or(Error::Context)?, 256)?,
                u32::from(model),
            );
        }
        assert_eq!(connections.command_errors, 17);
    }
    Ok(())
}

#[test]
fn q1_baseline_native_capacities_do_not_widen_qw_packet_entity_numbers() -> Result<(), Error> {
    for (protocol, capacity) in [
        (Protocol::NetQuake15, 600u16),
        (Protocol::QuakeWorld28, 768),
    ] {
        let (mut server, mut commands, mut connections) = byte_connection(protocol)?;
        let mut baseline = Q1_BASELINE;
        baseline[3..7].fill(255);
        for number in [0, 256, capacity - 1] {
            baseline[1..3].copy_from_slice(&number.to_le_bytes());
            q1_service_payload(
                protocol,
                &mut server,
                &mut commands,
                &mut connections,
                &baseline,
                |_, _, _| panic!("baseline-only boundary record published an event"),
            )?;
        }
        assert_eq!(connections.command_errors, 0);
        for number in [capacity, u16::MAX] {
            baseline[1..3].copy_from_slice(&number.to_le_bytes());
            q1_service_payload(
                protocol,
                &mut server,
                &mut commands,
                &mut connections,
                &baseline,
                |_, _, _| panic!("rejected baseline published an event"),
            )?;
        }
        assert_eq!(connections.command_errors, 2);
        assert!(
            connections
                .get(ClientId(0), Endpoint::Client)
                .ok_or(Error::Context)?
                .channel
                .current_snapshot()
                .is_none()
        );
        let update = if protocol == Protocol::NetQuake15 {
            NQ_BASELINE_UPDATE
        } else {
            QW_BASELINE_UPDATE
        };
        q1_service_payload(
            protocol,
            &mut server,
            &mut commands,
            &mut connections,
            update,
            |_, _, incoming| {
                if let Incoming::Snapshot(frame) = incoming {
                    assert_eq!(
                        &q1_frame_row(frame, 256).unwrap()[..5],
                        &[255, 255, 255, 255, 0]
                    );
                }
            },
        )?;
        assert_eq!(connections.command_errors, 2);
        let frame = connections
            .get(ClientId(0), Endpoint::Client)
            .ok_or(Error::Context)?
            .channel
            .current_snapshot()
            .ok_or(Error::Context)?;
        assert_eq!(&q1_frame_row(frame, 256)?[..5], &[255, 255, 255, 255, 0]);
    }
    let limits = snapshots::snapshot_limits(Protocol::QuakeWorld28);
    assert_eq!((limits.native_numbers, limits.end_entity), (768, 512));
    let words = [0; states::QW_ENTITY_WORDS];
    let mut bytes = [0; 32];
    let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
    assert!(states::write_qw_entity(
        &mut writer,
        511,
        &words,
        Some(&words),
        true
    )?);
    assert!(!states::write_qw_entity(
        &mut writer,
        512,
        &words,
        Some(&words),
        true
    )?);
    assert!(!states::write_qw_entity(
        &mut writer,
        767,
        &words,
        Some(&words),
        true
    )?);
    Ok(())
}

#[test]
fn q1_wire_baselines_are_independent_between_connected_namespaces() -> Result<(), Error> {
    for (protocol, update) in [
        (Protocol::NetQuake15, NQ_BASELINE_UPDATE),
        (Protocol::QuakeWorld28, QW_BASELINE_UPDATE),
    ] {
        let (mut first_server, mut first_commands, mut first) = byte_connection(protocol)?;
        let (mut second_server, mut second_commands, mut second) = byte_connection(protocol)?;
        let mut baseline = Q1_BASELINE;
        q1_service_payload(
            protocol,
            &mut first_server,
            &mut first_commands,
            &mut first,
            &baseline,
            |_, _, _| {},
        )?;
        baseline[3] = 19;
        q1_service_payload(
            protocol,
            &mut second_server,
            &mut second_commands,
            &mut second,
            &baseline,
            |_, _, _| {},
        )?;
        q1_service_payload(
            protocol,
            &mut second_server,
            &mut second_commands,
            &mut second,
            update,
            |_, _, _| {},
        )?;
        q1_service_payload(
            protocol,
            &mut first_server,
            &mut first_commands,
            &mut first,
            update,
            |_, _, _| {},
        )?;
        for (connections, model) in [(&first, 7), (&second, 19)] {
            let frame = connections
                .get(ClientId(0), Endpoint::Client)
                .ok_or(Error::Context)?
                .channel
                .current_snapshot()
                .ok_or(Error::Context)?;
            assert_q1_baseline_row(q1_frame_row(frame, 256)?, model);
            assert_eq!(connections.command_errors, 0);
        }
    }
    Ok(())
}

#[test]
fn nq_serverinfo_borrows_header_and_resource_rows_before_following_controls() -> Result<(), Error> {
    let (mut server, _, mut connections) = byte_connection(Protocol::NetQuake15)?;
    let mut payload = NQ_SERVERINFO.to_vec();
    payload.extend_from_slice(&Q1_BASELINE);
    payload.extend_from_slice(b"\x05\x2a\0\x20\x03\x04\x08after\0");
    let mut order = Vec::new();
    server_payload(
        &mut server,
        &mut connections,
        &payload,
        |_, _, incoming| match incoming {
            Incoming::ServerInfo(info) => {
                assert_eq!((info.max_clients, info.game_type), (16, 254));
                assert_eq!(info.level_name, b"Lev\x80el");
                let mut rows = Vec::new();
                info.for_each_config(|row| rows.push((row.index, row.text)))
                    .unwrap();
                assert_eq!(
                    rows,
                    [
                        (1, b"maps/a.bsp".as_slice()),
                        (2, b"*1"),
                        (257, b"misc/a.wav")
                    ]
                );
                assert_eq!(
                    rows[0].1.as_ptr() as usize - info.level_name.as_ptr() as usize,
                    info.level_name.len() + 1
                );
                let mut repeated = Vec::new();
                info.clone()
                    .for_each_config(|row| repeated.push((row.index, row.text.as_ptr())))
                    .unwrap();
                assert_eq!(
                    repeated,
                    rows.iter()
                        .map(|&(index, text)| (index, text.as_ptr()))
                        .collect::<Vec<_>>()
                );
                order.push(11);
            }
            Incoming::SetView { entity } => {
                assert_eq!(entity, 42);
                order.push(5);
            }
            Incoming::CdTrack(track) => {
                assert_eq!((track.track, track.loop_track), (3, Some(4)));
                order.push(32);
            }
            Incoming::Print(print) => {
                assert_eq!(print.text, b"after");
                order.push(8);
            }
            _ => panic!("serverinfo/baseline/control packet fabricated another event"),
        },
    )?;
    assert_eq!(order, [11, 5, 32, 8]);
    assert!(
        connections
            .get(ClientId(0), Endpoint::Client)
            .ok_or(Error::Context)?
            .channel
            .current_snapshot()
            .is_none()
    );
    server_payload(
        &mut server,
        &mut connections,
        NQ_BASELINE_UPDATE,
        |_, _, incoming| {
            if matches!(incoming, Incoming::NqEntityUpdate) {
                return;
            }
            let Incoming::Snapshot(frame) = incoming else {
                panic!("following native snapshot")
            };
            assert_q1_baseline_row(q1_frame_row(frame, 256).unwrap(), 7);
        },
    )?;
    assert_eq!(connections.command_errors, 0);
    Ok(())
}

#[test]
fn nq_serverinfo_cancels_only_validated_unsubmitted_client_text_and_prepared_bytes()
-> Result<(), Error> {
    for prepare in [false, true] {
        let (mut server, _, mut connections) = byte_connection(Protocol::NetQuake15)?;
        let client = connections
            .get_mut(ClientId(0), Endpoint::Client)
            .ok_or(Error::Context)?;
        client
            .channel
            .queue_client_command(Protocol::NetQuake15, b"prespawn")
            .map_err(|_| Error::Context)?;
        let discarded = client
            .channel
            .queue_client_command(Protocol::NetQuake15, b"unrelated old command")
            .map_err(|_| Error::Context)?;
        let prepared = if prepare {
            client
                .channel
                .prepare_output(EventTime(0))
                .map_err(|_| Error::Context)?
                .ok_or(Error::Context)?;
            assert_eq!(
                client
                    .channel
                    .submit_with(EventTime(0), |_| false)
                    .map_err(|_| Error::Context)?,
                None
            );
            Some(
                client
                    .channel
                    .pending_packet()
                    .ok_or(Error::Context)?
                    .bytes
                    .to_vec(),
            )
        } else {
            None
        };
        let before = client.channel.send_state();
        server_payload(
            &mut server,
            &mut connections,
            &NQ_SERVERINFO[..NQ_SERVERINFO.len() - 1],
            |_, _, _| panic!("malformed replacement escaped validation"),
        )?;
        let client = connections
            .get(ClientId(0), Endpoint::Client)
            .ok_or(Error::Context)?;
        assert_eq!(client.channel.send_state(), before);
        assert_eq!(
            client.channel.pending_packet().map(|packet| packet.bytes),
            prepared.as_deref()
        );
        assert!(client.channel.has_output());
        let mut headers = 0;
        server_payload(
            &mut server,
            &mut connections,
            NQ_SERVERINFO,
            |_, _, incoming| match incoming {
                Incoming::ServerInfo(_) => headers += 1,
                _ => panic!("replacement header manufactured another event"),
            },
        )?;
        let client = connections
            .get_mut(ClientId(0), Endpoint::Client)
            .ok_or(Error::Context)?;
        let mut expected = before;
        expected.reliable_bytes = 0;
        assert_eq!(client.channel.send_state(), expected);
        assert!(client.channel.pending_packet().is_none());
        assert!(!client.channel.has_output());
        assert!(
            client
                .channel
                .prepare_output(EventTime(4))
                .map_err(|_| Error::Context)?
                .is_none()
        );
        let fresh = client
            .channel
            .queue_client_command(Protocol::NetQuake15, b"fresh")
            .map_err(|_| Error::Context)?;
        assert!(fresh.0 > discarded.0);
        let packet = client
            .channel
            .prepare_output(EventTime(5))
            .map_err(|_| Error::Context)?
            .ok_or(Error::Context)?;
        let (header, body) = qa_network::headers::decode(
            qa_network::headers::NETQUAKE,
            qa_network::headers::Direction::ToServer,
            packet.bytes,
        )
        .map_err(|_| Error::Context)?;
        assert_eq!(header.sequence, before.sequence);
        assert_eq!(body, b"\x04fresh\0");
        assert_eq!((headers, connections.command_errors), (1, 1));
    }
    Ok(())
}

#[test]
fn nq_serverinfo_regenerates_a_blocked_native_ack_without_old_client_text() -> Result<(), Error> {
    let (mut server, _, mut connections) = byte_connection(Protocol::NetQuake15)?;
    let receipt = server.queue_reliable(&[1]).map_err(|_| Error::Context)?;
    server_payload(&mut server, &mut connections, &[], |_, _, _| {
        panic!("native nop manufactured a presentation event")
    })?;
    let client = connections
        .get_mut(ClientId(0), Endpoint::Client)
        .ok_or(Error::Context)?;
    client
        .channel
        .queue_client_command(Protocol::NetQuake15, b"obsolete")
        .map_err(|_| Error::Context)?;
    let old_ack = client
        .channel
        .prepare_output(EventTime(4))
        .map_err(|_| Error::Context)?
        .ok_or(Error::Context)?
        .bytes
        .to_vec();
    assert_eq!(
        client
            .channel
            .submit_with(EventTime(4), |_| false)
            .map_err(|_| Error::Context)?,
        None
    );
    assert_eq!(client.channel.pending_controls(), 1);
    server_payload(
        &mut server,
        &mut connections,
        NQ_SERVERINFO,
        |_, _, incoming| assert!(matches!(incoming, Incoming::ServerInfo(_))),
    )?;
    let client = connections
        .get_mut(ClientId(0), Endpoint::Client)
        .ok_or(Error::Context)?;
    assert!(client.channel.pending_packet().is_none());
    assert_eq!(client.channel.pending_controls(), 1);
    let ack = client
        .channel
        .prepare_output(EventTime(5))
        .map_err(|_| Error::Context)?
        .ok_or(Error::Context)?
        .bytes
        .to_vec();
    assert_eq!(ack, old_ack);
    client
        .channel
        .submitted(EventTime(5))
        .map_err(|_| Error::Context)?;
    server
        .receive(&ack, EventTime(6))
        .map_err(|_| Error::Context)?;
    assert_eq!(server.reliable_receipts(), &[receipt]);
    assert!(!client.channel.has_output());
    assert_eq!(client.channel.send_state().sequence, 0);
    assert_eq!(connections.command_errors, 0);
    Ok(())
}

#[test]
fn nq_serverinfo_preserves_a_submitted_multifragment_flight_and_its_native_receipts()
-> Result<(), Error> {
    let (mut server, _, mut connections) = byte_connection(Protocol::NetQuake15)?;
    let client = connections
        .get_mut(ClientId(0), Endpoint::Client)
        .ok_or(Error::Context)?;
    let receipts = [
        client
            .channel
            .queue_reliable(&[b'a'; 1200])
            .map_err(|_| Error::Context)?,
        client
            .channel
            .queue_reliable(&[b'b'; 1300])
            .map_err(|_| Error::Context)?,
    ];
    let mut regenerated = None;
    let mut fresh = None;
    let mut acknowledged = Vec::new();
    for part in 0..3 {
        let time = 10 + part * 10;
        let client = connections
            .get_mut(ClientId(0), Endpoint::Client)
            .ok_or(Error::Context)?;
        let data = client
            .channel
            .prepare_output(EventTime(time))
            .map_err(|_| Error::Context)?
            .ok_or(Error::Context)?
            .bytes
            .to_vec();
        if part == 1 {
            assert_eq!(Some(data.as_slice()), regenerated.as_deref());
        }
        client
            .channel
            .submitted(EventTime(time))
            .map_err(|_| Error::Context)?;
        match server
            .receive(&data, EventTime(time + 1))
            .map_err(|_| Error::Context)?
            .delivery
        {
            qa_network::channel::Delivery::Pending if part < 2 => {}
            qa_network::channel::Delivery::Payload(body) if part == 2 => {
                assert_eq!(&body[..1200], &[b'a'; 1200]);
                assert_eq!(&body[1200..], &[b'b'; 1300]);
            }
            _ => panic!("replacement corrupted a submitted reliable flight"),
        }
        let ack = server
            .prepare_output(EventTime(time + 2))
            .map_err(|_| Error::Context)?
            .ok_or(Error::Context)?
            .bytes
            .to_vec();
        server
            .submitted(EventTime(time + 2))
            .map_err(|_| Error::Context)?;
        connections.receive(
            Endpoint::Client.socket(),
            Peer::Loopback(ClientId(0)),
            &ack,
            EventTime(time + 3),
            |_, _, incoming| {
                match incoming {
                    Incoming::Acknowledged { receipt, .. } => acknowledged.push(receipt),
                    _ => panic!("CLIENT ACK manufactured a presentation event"),
                }
                std::ops::ControlFlow::Continue(())
            },
        );
        let client = connections
            .get_mut(ClientId(0), Endpoint::Client)
            .ok_or(Error::Context)?;
        if part == 0 {
            assert!(client.channel.reliable_receipts().is_empty());
            let discarded = client
                .channel
                .queue_client_command(Protocol::NetQuake15, b"obsolete suffix")
                .map_err(|_| Error::Context)?;
            regenerated = Some(
                client
                    .channel
                    .prepare_output(EventTime(time + 4))
                    .map_err(|_| Error::Context)?
                    .ok_or(Error::Context)?
                    .bytes
                    .to_vec(),
            );
            assert_eq!(
                client
                    .channel
                    .submit_with(EventTime(time + 4), |_| false)
                    .map_err(|_| Error::Context)?,
                None
            );
            let before = client.channel.send_state();
            let info = server
                .prepare_move(NQ_SERVERINFO, EventTime(time + 5), None, None)
                .map_err(|_| Error::Context)?
                .ok_or(Error::Context)?
                .bytes
                .to_vec();
            server
                .submitted(EventTime(time + 5))
                .map_err(|_| Error::Context)?;
            connections.receive(
                Endpoint::Client.socket(),
                Peer::Loopback(ClientId(0)),
                &info,
                EventTime(time + 6),
                |_, _, incoming| {
                    assert!(matches!(incoming, Incoming::ServerInfo(_)));
                    std::ops::ControlFlow::Continue(())
                },
            );
            let client = connections
                .get_mut(ClientId(0), Endpoint::Client)
                .ok_or(Error::Context)?;
            assert_eq!(client.channel.send_state(), before);
            assert!(client.channel.pending_packet().is_none());
            let receipt = client
                .channel
                .queue_client_command(Protocol::NetQuake15, b"fresh")
                .map_err(|_| Error::Context)?;
            assert!(receipt.0 > discarded.0);
            fresh = Some(receipt);
        } else if part == 2 {
            assert_eq!(client.channel.reliable_receipts(), &receipts);
        } else {
            assert!(client.channel.reliable_receipts().is_empty());
        }
    }
    assert_eq!(acknowledged, receipts);
    let client = connections
        .get_mut(ClientId(0), Endpoint::Client)
        .ok_or(Error::Context)?;
    let data = client
        .channel
        .prepare_output(EventTime(50))
        .map_err(|_| Error::Context)?
        .ok_or(Error::Context)?
        .bytes
        .to_vec();
    client
        .channel
        .submitted(EventTime(50))
        .map_err(|_| Error::Context)?;
    let delivery = server
        .receive(&data, EventTime(51))
        .map_err(|_| Error::Context)?
        .delivery;
    assert_eq!(
        delivery,
        qa_network::channel::Delivery::Payload(b"\x04fresh\0")
    );
    let ack = server
        .prepare_output(EventTime(52))
        .map_err(|_| Error::Context)?
        .ok_or(Error::Context)?
        .bytes
        .to_vec();
    server
        .submitted(EventTime(52))
        .map_err(|_| Error::Context)?;
    client
        .channel
        .receive(&ack, EventTime(53))
        .map_err(|_| Error::Context)?;
    assert_eq!(
        client.channel.reliable_receipts(),
        &[fresh.ok_or(Error::Context)?]
    );
    assert_eq!(client.channel.send_state().reliable_bytes, 0);
    assert_eq!(connections.command_errors, 0);
    Ok(())
}

#[test]
fn nq_serverinfo_replacement_resets_only_received_state_and_the_in_progress_packet()
-> Result<(), Error> {
    let (mut server, _, mut connections) = byte_connection(Protocol::NetQuake15)?;
    let mut bytes = [0; 1400];
    let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
    writer.write_data(&Q1_BASELINE)?;
    writer.write_data(NQ_BASELINE_UPDATE)?;
    let mut player = [0; states::NQ_PLAYER_WORDS];
    player[12] = 57.0f32.to_bits();
    states::write_nq_player(&mut writer, &player)?;
    server_payload(&mut server, &mut connections, writer.bytes(), |_, _, _| {})?;
    let client = connections
        .get_mut(ClientId(0), Endpoint::Client)
        .ok_or(Error::Context)?;
    let old = client
        .channel
        .current_snapshot()
        .ok_or(Error::Context)?
        .sequence();
    assert_eq!(
        client
            .channel
            .current_snapshot()
            .ok_or(Error::Context)?
            .player_words()
            .ok_or(Error::Context)?[12],
        57
    );
    client.channel.set_nq_weapon_mask(true)?;
    client
        .commands
        .as_mut()
        .ok_or(Error::Context)?
        .record(qa_core::primitives::UserCmd::default())
        .map_err(|_| Error::Context)?;
    client
        .channel
        .queue_client_command(Protocol::NetQuake15, b"retained")
        .map_err(|_| Error::Context)?;
    assert!(
        client
            .channel
            .prepare_output(EventTime(4))
            .map_err(|_| Error::Context)?
            .is_some()
    );
    client
        .channel
        .submitted(EventTime(4))
        .map_err(|_| Error::Context)?;
    let send = client.channel.send_state();
    assert!(send.reliable_bytes > 0);
    let datagram = client.channel.state().next_datagram;
    // These changed time/player/entity values precede a fully accepted new
    // serverinfo in the SAME packet and must not escape into its new state.
    let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
    writer.write_bits(7, 8)?;
    writer.write_bits(99.0f32.to_bits(), 32)?;
    writer.write_data(b"\x81\x40\0\x01")?;
    player[12] = 99.0f32.to_bits();
    states::write_nq_player(&mut writer, &player)?;
    writer.write_data(NQ_SERVERINFO)?;
    writer.write_data(b"\x05\x03\0")?;
    let mut headers = 0;
    server_payload(
        &mut server,
        &mut connections,
        writer.bytes(),
        |_, _, incoming| match incoming {
            Incoming::ServerInfo(_) => headers += 1,
            Incoming::SetView { entity: 3 } => {}
            Incoming::NqEntityUpdate => {}
            _ => panic!("new serverinfo leaked its preceding in-progress snapshot"),
        },
    )?;
    assert_eq!(headers, 1);
    let client = connections
        .get(ClientId(0), Endpoint::Client)
        .ok_or(Error::Context)?;
    assert!(client.channel.current_snapshot().is_none());
    assert!(client.channel.snapshot(old).is_none());
    assert_eq!(client.channel.send_state(), send);
    assert!(client.channel.state().next_datagram > datagram);
    assert_eq!(
        client
            .commands
            .as_ref()
            .ok_or(Error::Context)?
            .history()
            .ok_or(Error::Context)?
            .len(),
        1
    );
    server_payload(
        &mut server,
        &mut connections,
        b"\x81\x40\0\x01",
        |_, _, incoming| {
            if matches!(incoming, Incoming::NqEntityUpdate) {
                return;
            }
            let Incoming::Snapshot(frame) = incoming else {
                panic!("new-state entity update")
            };
            assert_eq!(frame.time(), ThinkTime::Seconds(0.0));
            assert!(frame.player_words().unwrap().iter().all(|&word| word == 0));
            assert!(
                q1_frame_row(frame, 256)
                    .unwrap()
                    .iter()
                    .all(|&word| word == 0)
            );
        },
    )?;
    let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
    player[18] = 3;
    states::write_nq_player(&mut writer, &player)?;
    server_payload(
        &mut server,
        &mut connections,
        writer.bytes(),
        |_, _, incoming| {
            let Incoming::Snapshot(frame) = incoming else {
                panic!("explicit weapon-mask policy")
            };
            assert_eq!(frame.player_words().unwrap()[18], 8);
        },
    )?;
    assert_eq!(connections.command_errors, 0);
    Ok(())
}

#[test]
fn nq_serverinfo_truncation_or_invalid_header_keeps_the_prior_ring_and_baselines()
-> Result<(), Error> {
    let (mut server, _, mut connections) = byte_connection(Protocol::NetQuake15)?;
    let mut payload = Q1_BASELINE.to_vec();
    payload.extend_from_slice(NQ_BASELINE_UPDATE);
    server_payload(&mut server, &mut connections, &payload, |_, _, _| {})?;
    let old = connections
        .get(ClientId(0), Endpoint::Client)
        .ok_or(Error::Context)?
        .channel
        .current_snapshot()
        .ok_or(Error::Context)?
        .sequence();
    let mut rejected = 0;
    for cut in 1..NQ_SERVERINFO.len() {
        server_payload(
            &mut server,
            &mut connections,
            &NQ_SERVERINFO[..cut],
            |_, _, _| panic!("partially validated header escaped"),
        )?;
        rejected += 1;
        let channel = &connections
            .get(ClientId(0), Endpoint::Client)
            .ok_or(Error::Context)?
            .channel;
        let frame = channel.current_snapshot().ok_or(Error::Context)?;
        assert_eq!(frame.sequence(), old);
        assert_q1_baseline_row(q1_frame_row(frame, 256)?, 7);
    }
    for (offset, value) in [(1, 28), (5, 0), (5, 17)] {
        let mut invalid = NQ_SERVERINFO.to_vec();
        invalid[offset] = value;
        server_payload(&mut server, &mut connections, &invalid, |_, _, _| {
            panic!("invalid header escaped")
        })?;
        rejected += 1;
    }
    assert_eq!(connections.command_errors, rejected);
    assert!(
        connections
            .get(ClientId(0), Endpoint::Client)
            .ok_or(Error::Context)?
            .channel
            .snapshot(old)
            .is_some()
    );
    // If any rejected header reset the old baseline, this omitted-field
    // update would decode to zeros instead of the admitted model and pose.
    server_payload(
        &mut server,
        &mut connections,
        NQ_BASELINE_UPDATE,
        |_, _, incoming| {
            if matches!(incoming, Incoming::NqEntityUpdate) {
                return;
            }
            let Incoming::Snapshot(frame) = incoming else {
                panic!("old-state update")
            };
            assert_q1_baseline_row(q1_frame_row(frame, 256).unwrap(), 7);
        },
    )?;
    Ok(())
}

#[test]
fn nq_serverinfo_resource_lists_keep_native_ordinals_counts_and_path_widths() -> Result<(), Error> {
    let (mut server, _, mut connections) = byte_connection(Protocol::NetQuake15)?;
    // Models/sounds both reserve zero and permit 255 admitted names. Empty
    // lists are syntactically valid; world readiness is a later operation.
    for (models, sounds, model_bytes, sound_bytes, valid) in [
        (0, 0, 1, 1, true),
        (255, 255, 1, 1, true),
        (256, 1, 1, 1, false),
        (1, 256, 1, 1, false),
        (1, 1, 63, 63, true),
        (1, 1, 64, 1, false),
        (1, 1, 1, 64, false),
    ] {
        let mut payload = b"\x0b\x0f\0\0\0\x01\xff".to_vec();
        let level = vec![b'L'; 127];
        payload.extend_from_slice(&level);
        payload.push(0);
        for (count, path_bytes) in [(models, model_bytes), (sounds, sound_bytes)] {
            let path = vec![b'x'; path_bytes];
            for _ in 0..count {
                payload.extend_from_slice(&path);
                payload.push(0);
            }
            payload.push(0);
        }
        let before = connections.command_errors;
        let mut admitted = 0;
        nq_reliable_payload(&mut server, &mut connections, &payload, |_, _, incoming| {
            let Incoming::ServerInfo(info) = incoming else {
                panic!("resource-only header fabricated an event")
            };
            assert!(valid, "invalid resource list escaped validation");
            assert_eq!((info.max_clients, info.game_type), (1, 255));
            assert_eq!(info.level_name, level.as_slice());
            let mut indices = Vec::new();
            info.for_each_config(|row| {
                let path_bytes = if row.index < 256 {
                    model_bytes
                } else {
                    sound_bytes
                };
                assert_eq!(row.text.len(), path_bytes);
                assert!(row.text.iter().all(|&byte| byte == b'x'));
                indices.push(row.index);
            })
            .unwrap();
            let expected = (1..=models)
                .map(|n| n as u32)
                .chain((1..=sounds).map(|n| 256 + n as u32))
                .collect::<Vec<_>>();
            assert_eq!(indices, expected);
            admitted += 1;
        })?;
        assert_eq!(admitted, usize::from(valid));
        assert_eq!(connections.command_errors - before, u64::from(!valid));
        assert!(
            connections
                .get(ClientId(0), Endpoint::Client)
                .ok_or(Error::Context)?
                .channel
                .current_snapshot()
                .is_none()
        );
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
    client.channel.set_nq_weapon_mask(true)?;
    server_payload(
        &mut server,
        &mut connections,
        &[22, 0, 1, 7, 0, 1, 0, 98, 0, 0, 0, 0, 0, 0, 0, 0],
        |_, _, _| panic!("baseline-only packet created a presentation event"),
    )?;
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
    source
        .view
        .accept_offset(qa_core::primitives::Vec3([0., 0., 22.]), None);
    let mut context = native_player_context();
    let mut player = [0; states::NQ_PLAYER_WORDS];
    assert!(projection.reduce(
        &source,
        &context,
        &mut player,
        |model, _| Some(model.ordinal),
        |sound| Some(sound.ordinal),
    ));
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
        qw_peers: 0,
        qw_dead: 0,
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
            7,
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
                assert!(projection.apply(
                    frame.player,
                    &mut imported,
                    &mut context,
                    qa_core::primitives::ResourceNamespace::Client(qa_core::primitives::ClientId(
                        0
                    )),
                    |_| None
                ));
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
            Incoming::NqEntityUpdate => {}
            _ => panic!("NQ snapshot/print stream"),
        },
    )?;
    assert_eq!((snapshots, prints, connections.command_errors), (1, 1, 0));
    assert!(!server.set_snapshot_baseline(256, &baseline));
    Ok(())
}

#[test]
fn nq_signon_stage_is_ordered_control_with_app_owned_progression() -> Result<(), Error> {
    let (mut server, _, mut connections) = byte_connection(Protocol::NetQuake15)?;
    let mut order = Vec::new();
    server_payload(
        &mut server,
        &mut connections,
        b"\x19\x01\x08after\0\x19\x03\x19\x02\x19\x02\x19\x04",
        |id, endpoint, incoming| {
            assert_eq!((id, endpoint), (ClientId(0), Endpoint::Client));
            match incoming {
                Incoming::SignonStage { stage } => order.push((25, stage)),
                Incoming::Print(print) => {
                    assert_eq!(print.text, b"after");
                    order.push((8, 0));
                }
                _ => panic!("signon controls manufactured snapshot or command progress"),
            }
        },
    )?;
    // This boundary checks the byte, while the one app owner decides whether
    // a repeated, decreasing or skipped stage is acceptable for its connection.
    assert_eq!(order, [(25, 1), (8, 0), (25, 3), (25, 2), (25, 2), (25, 4)]);
    assert!(
        connections
            .get(ClientId(0), Endpoint::Client)
            .ok_or(Error::Context)?
            .channel
            .current_snapshot()
            .is_none()
    );
    assert_eq!(connections.command_errors, 0);
    Ok(())
}

#[test]
fn nq_invalid_or_truncated_signon_stage_preserves_the_admitted_control_prefix() -> Result<(), Error>
{
    let (mut server, _, mut connections) = byte_connection(Protocol::NetQuake15)?;
    let mut stage = None;
    server_payload(
        &mut server,
        &mut connections,
        &[25, 1],
        |_, _, incoming| match incoming {
            Incoming::SignonStage { stage: value } => stage = Some(value),
            _ => panic!("stage-only packet"),
        },
    )?;
    for invalid in [b"\x19".as_slice(), b"\x19\0", b"\x19\x05", b"\x19\xff"] {
        server_payload(&mut server, &mut connections, invalid, |_, _, _| {
            panic!("invalid signon operand escaped")
        })?;
        assert_eq!(stage, Some(1));
    }
    let mut admitted = 0;
    server_payload(
        &mut server,
        &mut connections,
        &[25, 2, 25],
        |_, _, incoming| match incoming {
            Incoming::SignonStage { stage: value } => {
                admitted += 1;
                stage = Some(value);
            }
            _ => panic!("invalid signon tail published another event"),
        },
    )?;
    assert_eq!(
        (admitted, stage, connections.command_errors),
        (1, Some(2), 5)
    );
    assert!(
        connections
            .get(ClientId(0), Endpoint::Client)
            .ok_or(Error::Context)?
            .channel
            .current_snapshot()
            .is_none()
    );
    Ok(())
}

#[test]
fn nq_fast_update_notifications_follow_complete_records_and_interleaved_signon() -> Result<(), Error>
{
    let (mut server, _, mut connections) = byte_connection(Protocol::NetQuake15)?;
    let mut stage = 0;
    let mut completed = 0;
    let mut order = Vec::new();
    // Literal unchanged fast updates still carry real native entity records.
    // An update before stage 3 cannot consume the update after that service.
    server_payload(
        &mut server,
        &mut connections,
        b"\x80\x01\x19\x03\x80\x02\x80\x03\x08after\0",
        |_, _, incoming| match incoming {
            Incoming::NqEntityUpdate => {
                if stage == 3 {
                    stage = 4;
                    completed += 1;
                }
                order.push((128, stage));
            }
            Incoming::SignonStage { stage: value } => {
                stage = value;
                order.push((25, stage));
            }
            Incoming::Print(print) => {
                assert_eq!(print.text, b"after");
                order.push((8, stage));
            }
            Incoming::Snapshot(snapshots::ReceivedFrame::NetQuake(frame)) => {
                assert_eq!(
                    frame
                        .entities
                        .iter()
                        .map(|row| row.number)
                        .collect::<Vec<_>>(),
                    [1, 2, 3]
                );
                order.push((7, stage));
            }
            _ => panic!("fast-update/signon stream"),
        },
    )?;
    assert_eq!(completed, 1);
    assert_eq!(
        order,
        [(128, 0), (25, 3), (128, 4), (128, 4), (8, 4), (7, 4)]
    );
    assert_eq!(connections.command_errors, 0);
    Ok(())
}

#[test]
fn nq_signon_completion_requires_an_entire_fast_update_body() -> Result<(), Error> {
    let (mut server, _, mut connections) = byte_connection(Protocol::NetQuake15)?;
    let mut bytes = [0; 128];
    let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
    let mut entity = [0; states::NQ_ENTITY_WORDS];
    for (index, value) in entity[..11].iter_mut().enumerate() {
        *value = (index as f32 + 1.0).to_bits();
    }
    assert!(states::write_nq_entity(
        &mut writer,
        256,
        &[0; states::NQ_ENTITY_WORDS],
        &entity,
        true,
    )?);
    let update = writer.bytes().to_vec();
    for cut in 1..update.len() {
        let mut payload = vec![25, 3];
        payload.extend_from_slice(&update[..cut]);
        let mut stage = None;
        server_payload(
            &mut server,
            &mut connections,
            &payload,
            |_, _, incoming| match incoming {
                Incoming::SignonStage { stage: value } => stage = Some(value),
                _ => panic!("truncated fast body completed signon or published a frame"),
            },
        )?;
        assert_eq!(stage, Some(3));
    }
    assert_eq!(connections.command_errors, (update.len() - 1) as u64);
    let mut payload = update.clone();
    payload.extend_from_slice(&[128]);
    let mut updates = 0;
    server_payload(
        &mut server,
        &mut connections,
        &payload,
        |_, _, incoming| match incoming {
            Incoming::NqEntityUpdate => updates += 1,
            _ => panic!("malformed tail published more than its complete update prefix"),
        },
    )?;
    assert_eq!(updates, 1);
    assert!(
        connections
            .get(ClientId(0), Endpoint::Client)
            .ok_or(Error::Context)?
            .channel
            .current_snapshot()
            .is_none()
    );
    // Time and clientdata may legitimately publish a frame without any update.
    for clientdata in [false, true] {
        let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
        writer.write_data(&[25, 3])?;
        if clientdata {
            states::write_nq_player(&mut writer, &[0; states::NQ_PLAYER_WORDS])?;
        } else {
            writer.write_bits(7, 8)?;
            writer.write_bits(1.25f32.to_bits(), 32)?;
        }
        let mut frames = 0;
        server_payload(
            &mut server,
            &mut connections,
            writer.bytes(),
            |_, _, incoming| match incoming {
                Incoming::SignonStage { stage: 3 } => {}
                Incoming::Snapshot(_) => frames += 1,
                _ => panic!("time or clientdata manufactured a fast update"),
            },
        )?;
        assert_eq!(frames, 1);
    }
    Ok(())
}

#[test]
fn nq_signon_controls_are_connection_local_and_not_admitted_for_qw() -> Result<(), Error> {
    let (mut first_server, _, mut first) = byte_connection(Protocol::NetQuake15)?;
    let (mut second_server, _, mut second) = byte_connection(Protocol::NetQuake15)?;
    let mut stages = [0, 0];
    let mut updates = [0, 0];
    for (index, server, connections, stage) in [
        (0, &mut first_server, &mut first, 3),
        (1, &mut second_server, &mut second, 1),
    ] {
        server_payload(
            server,
            connections,
            &[25, stage, 128, 1],
            |_, _, incoming| match incoming {
                Incoming::SignonStage { stage } => stages[index] = stage,
                Incoming::NqEntityUpdate => updates[index] += 1,
                Incoming::Snapshot(_) => {}
                _ => panic!("connection-local signon controls"),
            },
        )?;
        assert_eq!(stages[1 - index], if index == 0 { 0 } else { 3 });
    }
    assert_eq!((stages, updates), ([3, 1], [1, 1]));
    assert_eq!((first.command_errors, second.command_errors), (0, 0));
    let (mut qw_server, mut qw_commands, mut qw) = byte_connection(Protocol::QuakeWorld28)?;
    for payload in [b"\x19\x03".as_slice(), b"\x80\x01"] {
        qw_move(&mut qw, &mut qw_server, &mut qw_commands, true)?;
        server_payload(&mut qw_server, &mut qw, payload, |_, _, incoming| {
            assert!(matches!(incoming, Incoming::CommandProgress(_)));
        })?;
    }
    assert_eq!(qw.command_errors, 2);
    Ok(())
}

#[test]
fn nq_setview_is_unsigned_ordered_control_without_snapshot_progress() -> Result<(), Error> {
    let (mut server, _, mut connections) = byte_connection(Protocol::NetQuake15)?;
    for (view, expected) in [
        ([5, 0, 0], 0),
        ([5, 1, 0], 1),
        ([5, 255, 0], 255),
        ([5, 0, 1], 256),
        ([5, 0x34, 0x12], 0x1234),
        ([5, 0, 0x80], 0x8000),
        ([5, 255, 255], u16::MAX),
    ] {
        let bytes = [view[0], view[1], view[2], 8, b'a', 0];
        let mut order = Vec::new();
        server_payload(
            &mut server,
            &mut connections,
            &bytes,
            |id, endpoint, incoming| {
                assert_eq!(id, ClientId(0));
                assert_eq!(endpoint, Endpoint::Client);
                match incoming {
                    Incoming::SetView { entity } => order.push((5, entity)),
                    Incoming::Print(print) => {
                        assert_eq!(print.text, b"a");
                        order.push((8, 0));
                    }
                    _ => {
                        panic!("standalone setview must not fabricate player or snapshot progress")
                    }
                }
            },
        )?;
        assert_eq!(order, [(5, expected), (8, 0)]);
        assert!(
            connections
                .get(ClientId(0), Endpoint::Client)
                .ok_or(Error::Context)?
                .channel
                .current_snapshot()
                .is_none()
        );
    }
    assert_eq!(connections.command_errors, 0);
    Ok(())
}

#[test]
fn nq_setview_in_normal_service_stream_commits_the_native_snapshot() -> Result<(), Error> {
    let (mut server, _, mut connections) = byte_connection(Protocol::NetQuake15)?;
    let mut bytes = [0; 1400];
    let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
    writer.write_data(b"\x05\x34\x12\x07")?;
    writer.write_bits(1.25f32.to_bits(), 32)?;
    let mut player = [0; states::NQ_PLAYER_WORDS];
    player[0] = 22.0f32.to_bits();
    player[12] = 37.0f32.to_bits();
    states::write_nq_player(&mut writer, &player)?;
    writer.write_data(b"\x08after\0\x05\xff\xff")?;
    let mut order = Vec::new();
    server_payload(
        &mut server,
        &mut connections,
        writer.bytes(),
        |_, _, incoming| match incoming {
            Incoming::SetView { entity } => order.push((5, u32::from(entity))),
            Incoming::Print(print) => {
                assert_eq!(print.text, b"after");
                order.push((8, 0));
            }
            Incoming::Snapshot(snapshots::ReceivedFrame::NetQuake(frame)) => {
                assert_eq!(frame.time, ThinkTime::Seconds(1.25));
                assert_eq!(frame.player[0], 22.0f32.to_bits());
                assert_eq!(frame.player[12], 37);
                order.push((7, 0));
            }
            _ => panic!("NQ setview/clientdata/print stream"),
        },
    )?;
    assert_eq!(order, [(5, 0x1234), (8, 0), (5, 65535), (7, 0)]);
    assert_eq!(connections.command_errors, 0);
    assert_eq!(
        connections
            .get(ClientId(0), Endpoint::Client)
            .ok_or(Error::Context)?
            .channel
            .current_snapshot()
            .ok_or(Error::Context)?
            .time(),
        ThinkTime::Seconds(1.25)
    );
    Ok(())
}

#[test]
fn nq_truncated_setview_keeps_prior_controls_and_accepted_frame() -> Result<(), Error> {
    let (mut server, _, mut connections) = byte_connection(Protocol::NetQuake15)?;
    let mut target = None;
    server_payload(
        &mut server,
        &mut connections,
        b"\x07\0\0\xa0\x3f\x05\x34\x12",
        |_, _, incoming| match incoming {
            Incoming::SetView { entity } => target = Some(entity),
            Incoming::Snapshot(_) => {}
            _ => panic!("initial time/setview stream"),
        },
    )?;
    assert_eq!(target, Some(0x1234));
    let accepted = connections
        .get(ClientId(0), Endpoint::Client)
        .ok_or(Error::Context)?
        .channel
        .current_snapshot()
        .ok_or(Error::Context)?
        .sequence();
    for truncated in [b"\x05".as_slice(), b"\x05\xef"] {
        server_payload(&mut server, &mut connections, truncated, |_, _, _| {
            panic!("truncated setview delivered an event")
        })?;
        assert_eq!(target, Some(0x1234));
    }
    // A later malformed operand does not undo a previously complete service.
    server_payload(
        &mut server,
        &mut connections,
        b"\x05\x22\x11\x05",
        |_, _, incoming| match incoming {
            Incoming::SetView { entity } => target = Some(entity),
            _ => panic!("malformed tail published more than its valid control prefix"),
        },
    )?;
    assert_eq!(target, Some(0x1122));
    assert_eq!(connections.command_errors, 3);
    let frame = connections
        .get(ClientId(0), Endpoint::Client)
        .ok_or(Error::Context)?
        .channel
        .current_snapshot()
        .ok_or(Error::Context)?;
    assert_eq!(frame.sequence(), accepted);
    assert_eq!(frame.time(), ThinkTime::Seconds(1.25));
    Ok(())
}

#[test]
fn nq_setview_intake_is_independent_between_connections_and_not_admitted_for_qw()
-> Result<(), Error> {
    let (mut first_server, _, mut first) = byte_connection(Protocol::NetQuake15)?;
    let (mut second_server, _, mut second) = byte_connection(Protocol::NetQuake15)?;
    let mut targets = [None, None];
    server_payload(
        &mut first_server,
        &mut first,
        b"\x05\x34\x12",
        |_, _, incoming| {
            let Incoming::SetView { entity } = incoming else {
                panic!("first connection setview")
            };
            targets[0] = Some(entity);
        },
    )?;
    assert_eq!(targets, [Some(0x1234), None]);
    server_payload(
        &mut second_server,
        &mut second,
        b"\x05\0\x80",
        |_, _, incoming| {
            let Incoming::SetView { entity } = incoming else {
                panic!("second connection setview")
            };
            targets[1] = Some(entity);
        },
    )?;
    assert_eq!(targets, [Some(0x1234), Some(0x8000)]);
    assert_eq!((first.command_errors, second.command_errors), (0, 0));
    let (mut qw_server, mut qw_commands, mut qw) = byte_connection(Protocol::QuakeWorld28)?;
    qw_move(&mut qw, &mut qw_server, &mut qw_commands, true)?;
    server_payload(
        &mut qw_server,
        &mut qw,
        b"\x05\x34\x12",
        |_, _, incoming| {
            assert!(
                matches!(incoming, Incoming::CommandProgress(_)),
                "NQ setview support leaked into QW"
            );
        },
    )?;
    assert_eq!(qw.command_errors, 1);
    Ok(())
}

#[test]
fn nq_original_literal_time_and_version_services_have_distinct_native_meanings() -> Result<(), Error>
{
    let (mut server, _, mut connections) = byte_connection(Protocol::NetQuake15)?;
    // Original protocol.h: svc_version4 / LE protocol15 is not a float clock.
    server_payload(
        &mut server,
        &mut connections,
        b"\x04\x0f\0\0\0",
        |_, _, incoming| {
            assert!(!matches!(incoming, Incoming::Snapshot(_)));
        },
    )?;
    assert!(
        connections
            .get(ClientId(0), Endpoint::Client)
            .ok_or(Error::Context)?
            .channel
            .current_snapshot()
            .is_none()
    );
    // Literal MSG_WriteByte(svc_time7), MSG_WriteFloat(1.25), then svc_print8.
    let mut times = Vec::new();
    let mut prints = Vec::new();
    server_payload(
        &mut server,
        &mut connections,
        b"\x04\x0f\0\0\0\x07\0\0\xa0\x3f\x08after\0",
        |_, _, incoming| match incoming {
            Incoming::Snapshot(frame) => times.push(frame.time()),
            Incoming::Print(print) => prints.push(print.text.to_vec()),
            _ => panic!("version/time/print service stream"),
        },
    )?;
    assert_eq!(times, [ThinkTime::Seconds(1.25)]);
    assert_eq!(prints, [b"after".to_vec()]);
    server_payload(
        &mut server,
        &mut connections,
        b"\x04\x0f\0\0\0",
        |_, _, incoming| {
            assert!(!matches!(incoming, Incoming::Snapshot(_)));
        },
    )?;
    let accepted = connections
        .get(ClientId(0), Endpoint::Client)
        .ok_or(Error::Context)?
        .channel
        .current_snapshot()
        .ok_or(Error::Context)?
        .sequence();
    let mut failures = 0;
    for bytes in [
        &b"\x04\x1c\0\0\0"[..],   // protocol28
        &b"\x04\x9a\x02\0\0"[..], // protocol666 was not negotiated
        &b"\x04\xe7\x03\0\0"[..], // protocol999 was not negotiated
        &b"\x04\0\0\x80\x3f"[..], // old bug reinterpreted this as1.0seconds
    ] {
        server_payload(&mut server, &mut connections, bytes, |_, _, incoming| {
            assert!(!matches!(incoming, Incoming::Snapshot(_)))
        })?;
        failures += 1;
    }
    for bytes in [&b"\x04\x0f\0\0\0"[..], &b"\x07\0\0\xa0\x3f"[..]] {
        for cut in 1..bytes.len() {
            server_payload(
                &mut server,
                &mut connections,
                &bytes[..cut],
                |_, _, incoming| assert!(!matches!(incoming, Incoming::Snapshot(_))),
            )?;
            failures += 1;
        }
    }
    // Native parsing keeps prior print effects when a later version is bad.
    prints.clear();
    server_payload(
        &mut server,
        &mut connections,
        b"\x08prefix\0\x04\x1c\0\0\0",
        |_, _, incoming| match incoming {
            Incoming::Print(print) => prints.push(print.text.to_vec()),
            _ => panic!("rejected version must not publish a clock"),
        },
    )?;
    failures += 1;
    assert_eq!(prints, [b"prefix".to_vec()]);
    assert_eq!(connections.command_errors, failures);
    let frame = connections
        .get(ClientId(0), Endpoint::Client)
        .ok_or(Error::Context)?
        .channel
        .current_snapshot()
        .ok_or(Error::Context)?;
    assert_eq!(frame.sequence(), accepted);
    assert_eq!(frame.time(), ThinkTime::Seconds(1.25));
    Ok(())
}

#[test]
fn nq_move_timestamp_uses_admitted_current_native_time_without_selecting_duration()
-> Result<(), Error> {
    use qa_core::primitives::UserCmd;
    use qa_network::commands::packet::{self, Key, Move};
    fn check(connections: &Connections, expected: f32) -> Result<(), Error> {
        let client = connections
            .get(ClientId(0), Endpoint::Client)
            .ok_or(Error::Context)?;
        let commands = client.commands.as_ref().ok_or(Error::Context)?;
        let mut bytes = [0; 1400];
        let length = commands.encode(
            &UserCmd {
                duration_ms: 16,
                duration_ns: 16_000_123,
                ..Default::default()
            },
            |_| 0,
            &client.channel,
            &mut bytes,
        )?;
        assert_eq!(bytes[0], 3); // clc_move
        assert_eq!(&bytes[1..5], &expected.to_bits().to_le_bytes());
        let Move::NetQuake { timestamp, .. } = packet::read(
            Protocol::NetQuake15,
            &mut bytes[..length],
            0,
            Key::default(),
        )?
        else {
            return Err(Error::Context);
        };
        assert_eq!(timestamp.to_bits(), expected.to_bits());
        let mut server_commands = Commands::load(Protocol::NetQuake15, 8192);
        let mut server_channel =
            Channel::load(Protocol::NetQuake15.channel(), Endpoint::Server, 8192, 16)
                .map_err(|_| Error::Context)?;
        let length = server_commands.stage(&bytes[..length])?;
        let mut decoded = None;
        assert_eq!(
            server_commands.decode(length, 0, 4321, &mut server_channel, None, |incoming| {
                if let qa_network::ingress::Incoming::Command {
                    command,
                    native_weapon: _,
                    observation: _,
                    ..
                } = incoming
                {
                    decoded = Some(command);
                }
                std::ops::ControlFlow::Continue(())
            })?,
            1
        );
        let decoded = decoded.ok_or(Error::Context)?;
        assert_eq!(decoded.duration_ns, 0);
        assert_eq!(decoded.duration_ms, 0);
        assert_eq!(decoded.server_time_ms, 4321);
        assert_eq!(
            Protocol::NetQuake15.command_execution(),
            qa_core::primitives::CommandExecution::FrameLatest,
        );
        Ok(())
    }
    let (mut server, _, mut connections) = byte_connection(Protocol::NetQuake15)?;
    check(&connections, 0.)?;
    let mut bytes = [0; 64];
    let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
    writer.write_bits(7, 8)?;
    writer.write_bits(1.234567f32.to_bits(), 32)?;
    server_payload(&mut server, &mut connections, writer.bytes(), |_, _, _| {})?;
    check(&connections, 1.234567)?;
    server_payload(&mut server, &mut connections, b"\x1aprint\0", |_, _, _| {})?;
    check(&connections, 1.234567)?;
    server_payload(&mut server, &mut connections, &[7, 0, 0, 0], |_, _, _| {})?;
    check(&connections, 1.234567)?;
    assert_eq!(connections.command_errors, 1);
    let channel = &connections
        .get(ClientId(0), Endpoint::Client)
        .ok_or(Error::Context)?
        .channel;
    assert_eq!(
        channel.current_snapshot().ok_or(Error::Context)?.sequence(),
        0
    );
    assert_eq!(channel.state().next_datagram, 3);
    let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
    writer.write_bits(7, 8)?;
    writer.write_bits((-0.0f32).to_bits(), 32)?;
    server
        .queue_reliable(writer.bytes())
        .map_err(|_| Error::Context)?;
    server_payload(&mut server, &mut connections, &[], |_, _, _| {})?;
    check(&connections, -0.)?;
    server_payload(&mut server, &mut connections, b"\x1aprint\0", |_, _, _| {})?;
    check(&connections, -0.)?;
    Ok(())
}

#[test]
fn nq_reliable_service_stream_waits_for_native_ack_after_client_publication() -> Result<(), Error> {
    let (mut server, _, mut connections) = byte_connection(Protocol::NetQuake15)?;
    let mut payload = [0; 1400];
    let mut writer = Writer::new(&mut payload, Encoding::Bytes);
    writer.write_bits(7, 8)?;
    writer.write_bits(1.0625f32.to_bits(), 32)?;
    let mut player = [0; states::NQ_PLAYER_WORDS];
    player[0] = 22.0f32.to_bits();
    player[12] = (-17.0f32).to_bits();
    states::write_nq_player(&mut writer, &player)?;
    writer.write_bits(8, 8)?;
    writer.write_data(b"reliable\0")?;
    let mut seen = 0;
    nq_reliable_payload(
        &mut server,
        &mut connections,
        writer.bytes(),
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
    )?;
    assert_eq!((seen, connections.command_errors), (2, 0));
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
            writer.write_bits(7, 8)?;
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
            writer.write_bits(7, 8)?;
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
    writer.write_bits(7, 8)?;
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

#[test]
fn previous_snapshot_borrows_only_the_prior_accepted_ring_slot() -> Result<(), Error> {
    let mut server = snapshots::Q2Ring::load(8, 1024, 32, None)?;
    let mut client = snapshots::Q2Ring::load(8, 1024, 32, Some(896))?;
    for sequence in 1..=5 {
        q2_store(&mut server, sequence, &[q2_entity(30, sequence as f32, 0)])?;
    }
    assert!(q2_send(&server, &mut client, 1, None)?);
    assert!(client.previous().is_none());
    assert!(!q2_send(&server, &mut client, 3, Some(2))?);
    assert!(client.current().is_none());
    assert_eq!(client.previous().map(|frame| frame.sequence), Some(1));
    assert!(!q2_send(&server, &mut client, 4, Some(2))?);
    assert_eq!(client.previous().map(|frame| frame.sequence), Some(1));
    assert!(q2_send(&server, &mut client, 5, None)?);
    assert_eq!(client.current().map(|frame| frame.sequence), Some(5));
    assert_eq!(client.previous().map(|frame| frame.sequence), Some(1));
    // Replacing the same accepted sequence must not turn current into previous.
    assert!(q2_send(&server, &mut client, 5, None)?);
    assert_eq!(client.previous().map(|frame| frame.sequence), Some(1));
    for sequence in [6, 38] {
        q2_store(&mut server, sequence, &[q2_entity(30, sequence as f32, 0)])?;
        assert!(q2_send(&server, &mut client, sequence, None)?);
    }
    // Frame38 overwrites frame6's slot; no copied fallback entity state exists.
    assert_eq!(client.current().map(|frame| frame.sequence), Some(38));
    assert!(client.previous().is_none());
    Ok(())
}
