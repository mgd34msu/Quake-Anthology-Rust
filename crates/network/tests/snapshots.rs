use qa_core::{
    loopback::Endpoint,
    primitives::ClientId,
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
    snapshots::{self, Entity, Frame, Q3Ring, Ring},
    states::{ENTITY_WORDS, PLAYER_WORDS},
};

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
        time: sequence as i32 * 50,
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
            time: 0,
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
            time: 0,
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
        time: 0,
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
        .prepare_move(&payload[..n], EventTime(1))
        .map_err(|_| Error::Context)?
    else {
        return Err(Error::Context);
    };
    let mut snapshots = 0;
    let mut reliable = 0;
    connections.receive(
        Endpoint::Client.socket(),
        Peer::Loopback(ClientId(0)),
        packet.bytes,
        EventTime(2),
        |id, endpoint, incoming| {
            if let Incoming::Snapshot(frame) = incoming {
                assert_eq!(id, ClientId(0));
                assert_eq!(endpoint, Endpoint::Client);
                assert_eq!(frame.entities, [entity(1, 7.5)]);
                assert_eq!(frame.sequence, 1);
                assert_eq!(frame.command, 64);
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
        .prepare_move(&payload[..n], EventTime(1))
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
    client_commands.decode_output(n, sequence, client, |_, _| {})
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
