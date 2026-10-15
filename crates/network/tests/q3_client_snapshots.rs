use qa_core::{
    loopback::Endpoint,
    primitives::{ThinkTime, UserCmd},
    sys_events::EventTime,
};
use qa_network::{
    channel::{Channel, Delivery, Unreliable, commands::CommandContext},
    commands::{
        connection::Commands,
        packet::{Error, Protocol},
    },
    headers::{self, Direction},
    snapshots::{Entity, Frame, ReceivedFrame},
    states,
};
use std::ops::ControlFlow;

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

fn entity(number: u32) -> Entity<{ states::ENTITY_WORDS }> {
    let mut words = [0; states::ENTITY_WORDS];
    words[23] = 0x8123_4567; // Native apos.trType narrows to eight wire bits.
    words[29] = 1;
    words[40] = 0xfedc_ba98; // Native time is another 32-bit cell.
    Entity { number, words }
}

fn prepare_snapshot(
    server: &mut Channel,
    time: i64,
    command_time: i32,
    entities: &[Entity<{ states::ENTITY_WORDS }>],
    delta: Option<u32>,
) -> u32 {
    let sequence = server.send_state().sequence;
    let mut player = [0; states::PLAYER_WORDS];
    player[0] = command_time as u32;
    player[1] = 123.5f32.to_bits();
    player[96] = 0x8123_4567;
    player[111] = 0xfedc_ba98;
    server
        .publish_snapshot(ReceivedFrame::Quake3(Frame {
            sequence,
            time: ThinkTime::Milliseconds(time),
            command: 0,
            flags: 5,
            qw_peers: 0,
            qw_dead: 0,
            areas: &[0x55, 0xaa],
            player: &player,
            entities,
        }))
        .unwrap();
    let mut bytes = vec![0; MESSAGE_BYTES];
    let length = server
        .encode_server_output(&mut bytes, |writer, channel| {
            channel.write_snapshot(writer, sequence, delta, 0)
        })
        .unwrap();
    let prepared = server
        .prepare_snapshot(sequence, None, &bytes[..length], EventTime(0))
        .unwrap()
        .unwrap();
    assert_eq!(prepared.unreliable, Unreliable::Included);
    sequence
}

fn submit_snapshot(
    server: &mut Channel,
    client: &mut Channel,
    received_at: EventTime,
    deliver: bool,
) {
    loop {
        let bytes = server.pending_packet().unwrap().bytes.to_vec();
        server.submitted(EventTime(0)).unwrap();
        if deliver {
            let received = client.receive(&bytes, received_at).unwrap();
            let sequence = received.header.sequence;
            if let Delivery::Payload(bytes) = received.delivery {
                let mut bytes = bytes.to_vec();
                client
                    .decode_server_output(&mut bytes, sequence, |_| {})
                    .unwrap();
            }
        }
        if !server.pending_fragments() {
            break;
        }
        server.prepare_output(EventTime(0)).unwrap().unwrap();
    }
}

fn snapshot(
    server: &mut Channel,
    client: &mut Channel,
    time: i64,
    command_time: i32,
    entities: &[Entity<{ states::ENTITY_WORDS }>],
    delta: Option<u32>,
    received_at: EventTime,
) -> u32 {
    let sequence = prepare_snapshot(server, time, command_time, entities, delta);
    submit_snapshot(server, client, received_at, true);
    sequence
}

fn prepare_move(client: &mut Channel, command_time: i32, prepared_at: EventTime) {
    let command = UserCmd {
        server_time_ms: command_time,
        ..Default::default()
    };
    let codec = Commands::load(Protocol::Quake3_68, MESSAGE_BYTES);
    let mut bytes = vec![0; MESSAGE_BYTES];
    let length = codec.encode(&command, |_| 0, client, &mut bytes).unwrap();
    assert_eq!(
        client
            .prepare_move(
                &bytes[..length],
                prepared_at,
                None,
                Some(qa_network::channel::MoveCommand {
                    number: 0,
                    server_time_ms: command_time
                })
            )
            .unwrap()
            .unwrap()
            .unreliable,
        Unreliable::Included
    );
}

fn submit_move_packet(client: &mut Channel, server: &mut Channel, submitted_at: EventTime) -> bool {
    let bytes = client.pending_packet().unwrap().bytes.to_vec();
    client.submitted(submitted_at).unwrap();
    let received = server.receive(&bytes, submitted_at).unwrap();
    let sequence = received.header.sequence;
    let Delivery::Payload(bytes) = received.delivery else {
        return false;
    };
    let mut codec = Commands::load(Protocol::Quake3_68, MESSAGE_BYTES);
    let length = codec.stage(bytes).unwrap();
    let mut commands = 0;
    assert_eq!(
        codec
            .decode(length, sequence, 0, server, None, |incoming| {
                if matches!(incoming, qa_network::ingress::Incoming::Command { .. }) {
                    commands += 1;
                }
                ControlFlow::Continue(())
            })
            .unwrap(),
        1
    );
    assert_eq!(commands, 1);
    true
}

fn move_command(
    client: &mut Channel,
    server: &mut Channel,
    command_time: i32,
    submitted_at: EventTime,
) {
    prepare_move(client, command_time, submitted_at);
    assert!(submit_move_packet(client, server, submitted_at));
}

#[test]
fn public_snapshot_view_is_client_only_and_retains_native_cells() {
    let (mut client, mut server) = pair();
    assert_eq!(
        client.q3_snapshot_number(),
        Some((0, ThinkTime::Milliseconds(0)))
    );
    assert!(client.q3_client_snapshot(0).unwrap().is_none());
    assert!(client.q3_client_snapshot(-1).unwrap().is_none());
    assert!(matches!(client.q3_client_snapshot(1), Err(Error::Context)));
    assert_eq!(server.q3_snapshot_number(), None);
    assert!(matches!(server.q3_client_snapshot(0), Err(Error::Context)));
    let other = Channel::load(
        Protocol::Quake2_34.channel(),
        Endpoint::Client,
        MESSAGE_BYTES,
        8,
    )
    .unwrap();
    assert_eq!(other.q3_snapshot_number(), None);
    assert!(matches!(other.q3_client_snapshot(0), Err(Error::Context)));

    let row = entity(7);
    let number = snapshot(
        &mut server,
        &mut client,
        0x8000_0010,
        0,
        &[row],
        None,
        EventTime::from_milliseconds(137),
    );
    assert_eq!(
        client.q3_snapshot_number(),
        Some((number, ThinkTime::Milliseconds(-2_147_483_632)))
    );
    let public = client.q3_client_snapshot(number as i32).unwrap().unwrap();
    assert_eq!(public.frame.sequence, number);
    assert_eq!(public.frame.time, ThinkTime::Milliseconds(-2_147_483_632));
    assert_eq!(public.frame.flags, 5);
    assert_eq!(public.frame.areas, [0x55, 0xaa]);
    assert_eq!(public.frame.player[1], 123.5f32.to_bits());
    assert_eq!(public.frame.player[96], 0x8123_4567);
    assert_eq!(public.frame.player[111], 0xfedc_ba98);
    let mut expected = row;
    expected.words[23] = u32::from(row.words[23] as u8);
    assert_eq!(public.frame.entities, [expected]);
    assert_eq!(public.ping, 137); // Native initialized outPackets contain zero.
    assert!(client.q3_client_snapshot(0).unwrap().is_none());
    assert!(matches!(
        client.q3_client_snapshot(number as i32 + 1),
        Err(Error::Context)
    ));
}

#[test]
fn missing_delta_ages_entities_without_replacing_last_good_header() {
    let (mut client, mut server) = pair();
    let first = snapshot(
        &mut server,
        &mut client,
        50,
        0,
        &[entity(1)],
        None,
        EventTime::from_milliseconds(80),
    );
    let mut last_missing = None;
    for (count, parsed) in [(959, 960), (960, 1920), (127, 2047), (1, 2048)] {
        let missing = prepare_snapshot(&mut server, 100, 0, &[], None);
        submit_snapshot(&mut server, &mut client, EventTime(0), false);
        last_missing = Some(missing);
        let rows: Vec<_> = (1..=count).map(entity).collect();
        let rejected = snapshot(
            &mut server,
            &mut client,
            150,
            0,
            &rows,
            Some(missing),
            EventTime::from_milliseconds(180),
        );
        assert_eq!(
            client.q3_snapshot_number(),
            Some((first, ThinkTime::Milliseconds(50)))
        );
        assert!(client.q3_client_snapshot(rejected as i32).is_err());
        assert_eq!(client.snapshot(first).is_some(), parsed <= 1920);
        assert_eq!(
            client.q3_client_snapshot(first as i32).unwrap().is_some(),
            parsed < 2048
        );
    }
    assert_eq!(
        client.q3_snapshot_number(),
        Some((first, ThinkTime::Milliseconds(50)))
    );
    let recovered = snapshot(
        &mut server,
        &mut client,
        300,
        0,
        &[entity(9)],
        None,
        EventTime::from_milliseconds(330),
    );
    assert_eq!(
        client.q3_snapshot_number(),
        Some((recovered, ThinkTime::Milliseconds(300)))
    );
    assert!(
        client
            .q3_client_snapshot(recovered as i32)
            .unwrap()
            .is_some()
    );
    assert!(
        client
            .q3_client_snapshot(last_missing.unwrap() as i32)
            .unwrap()
            .is_none()
    );
}

#[test]
fn public_snapshot_has_the_native_packet_backup_window() {
    let (mut client, mut server) = pair();
    let first = snapshot(&mut server, &mut client, 50, 0, &[], None, EventTime(0));
    for index in 2..=32 {
        snapshot(
            &mut server,
            &mut client,
            index * 50,
            0,
            &[],
            None,
            EventTime(0),
        );
    }
    assert!(client.q3_client_snapshot(first as i32).unwrap().is_some());
    let last = snapshot(&mut server, &mut client, 1650, 0, &[], None, EventTime(0));
    assert_eq!(last - first, 32);
    assert!(client.q3_client_snapshot(first as i32).unwrap().is_none());
    assert!(client.q3_client_snapshot(31).unwrap().is_some());
    assert!(client.q3_client_snapshot(-1).unwrap().is_none());
    assert!(matches!(
        client.q3_client_snapshot(last as i32 + 1),
        Err(Error::Context)
    ));
}

#[test]
fn ping_uses_submitted_moves_and_native_signed_milliseconds() {
    let (mut client, mut server) = pair();
    move_command(
        &mut client,
        &mut server,
        100,
        EventTime::from_milliseconds(1000),
    );
    move_command(
        &mut client,
        &mut server,
        200,
        EventTime::from_milliseconds(1020),
    );
    let number = snapshot(
        &mut server,
        &mut client,
        150,
        150,
        &[],
        None,
        EventTime::from_milliseconds(1080),
    );
    assert_eq!(
        client
            .q3_client_snapshot(number as i32)
            .unwrap()
            .unwrap()
            .ping,
        80
    );
    let number = snapshot(
        &mut server,
        &mut client,
        200,
        200,
        &[],
        None,
        EventTime::from_milliseconds(1100),
    );
    assert_eq!(
        client
            .q3_client_snapshot(number as i32)
            .unwrap()
            .unwrap()
            .ping,
        80
    );

    let (mut client, mut server) = pair();
    let sent = u32::MAX as u64 - 9;
    move_command(
        &mut client,
        &mut server,
        100,
        EventTime::from_milliseconds(sent),
    );
    let number = snapshot(
        &mut server,
        &mut client,
        -100,
        100,
        &[],
        None,
        EventTime::from_milliseconds(sent + 25),
    );
    assert_eq!(
        client
            .q3_client_snapshot(number as i32)
            .unwrap()
            .unwrap()
            .ping,
        25
    );
    let (mut client, mut server) = pair();
    let number = snapshot(
        &mut server,
        &mut client,
        0,
        -1,
        &[],
        None,
        EventTime::from_milliseconds(25),
    );
    assert_eq!(
        client
            .q3_client_snapshot(number as i32)
            .unwrap()
            .unwrap()
            .ping,
        999
    );
}

#[test]
fn refused_prepared_moves_never_claim_ping_metadata() {
    let (mut client, mut server) = pair();
    prepare_move(&mut client, 700, EventTime::from_milliseconds(10));
    let bytes = client.pending_packet().unwrap().bytes.to_vec();
    assert_eq!(
        client
            .submit_with(EventTime::from_milliseconds(10), |_| false)
            .unwrap(),
        None
    );
    assert_eq!(client.pending_packet().unwrap().bytes, bytes);
    let number = snapshot(
        &mut server,
        &mut client,
        700,
        700,
        &[],
        None,
        EventTime::from_milliseconds(100),
    );
    assert_eq!(
        client
            .q3_client_snapshot(number as i32)
            .unwrap()
            .unwrap()
            .ping,
        100
    );
    assert!(submit_move_packet(
        &mut client,
        &mut server,
        EventTime::from_milliseconds(120)
    ));
    let number = snapshot(
        &mut server,
        &mut client,
        700,
        700,
        &[],
        None,
        EventTime::from_milliseconds(150),
    );
    assert_eq!(
        client
            .q3_client_snapshot(number as i32)
            .unwrap()
            .unwrap()
            .ping,
        30
    );
}

#[test]
fn fragmented_move_retains_first_submission_time_until_final_admission() {
    let (mut client, mut server) = pair();
    for _ in 0..6 {
        client.queue_reliable(&[b'x'; 900]).unwrap();
    }
    prepare_move(&mut client, 400, EventTime::from_milliseconds(10));
    assert!(!submit_move_packet(
        &mut client,
        &mut server,
        EventTime::from_milliseconds(20)
    ));
    assert!(client.pending_fragments());
    let number = snapshot(
        &mut server,
        &mut client,
        400,
        400,
        &[],
        None,
        EventTime::from_milliseconds(80),
    );
    assert_eq!(
        client
            .q3_client_snapshot(number as i32)
            .unwrap()
            .unwrap()
            .ping,
        80
    );
    assert_eq!(
        client
            .prepare_move(
                b"deferred",
                EventTime::from_milliseconds(90),
                None,
                Some(qa_network::channel::MoveCommand {
                    number: 0,
                    server_time_ms: 900
                })
            )
            .unwrap()
            .unwrap()
            .unreliable,
        Unreliable::Deferred
    );
    assert_eq!(
        client
            .submit_with(EventTime::from_milliseconds(90), |_| false)
            .unwrap(),
        None
    );
    let number = snapshot(
        &mut server,
        &mut client,
        400,
        400,
        &[],
        None,
        EventTime::from_milliseconds(100),
    );
    assert_eq!(
        client
            .q3_client_snapshot(number as i32)
            .unwrap()
            .unwrap()
            .ping,
        100
    );
    assert!(!submit_move_packet(
        &mut client,
        &mut server,
        EventTime::from_milliseconds(110)
    ));
    let mut submitted = 110;
    let mut rejected_final = false;
    while client.pending_fragments() {
        client
            .prepare_output(EventTime::from_milliseconds(submitted))
            .unwrap()
            .unwrap();
        submitted += 10;
        let (header, _) = headers::decode(
            headers::QUAKE3,
            Direction::ToServer,
            client.pending_packet().unwrap().bytes,
        )
        .unwrap();
        if !header.fragment.unwrap().more {
            rejected_final = true;
            assert_eq!(
                client
                    .submit_with(EventTime::from_milliseconds(submitted), |_| false)
                    .unwrap(),
                None
            );
            let number = snapshot(
                &mut server,
                &mut client,
                400,
                400,
                &[],
                None,
                EventTime::from_milliseconds(240),
            );
            assert_eq!(
                client
                    .q3_client_snapshot(number as i32)
                    .unwrap()
                    .unwrap()
                    .ping,
                240
            );
            submitted = 260;
        }
        let complete = submit_move_packet(
            &mut client,
            &mut server,
            EventTime::from_milliseconds(submitted),
        );
        assert_eq!(complete, !client.pending_fragments());
    }
    assert!(rejected_final);
    assert_eq!(submitted, 260);
    let number = snapshot(
        &mut server,
        &mut client,
        400,
        400,
        &[],
        None,
        EventTime::from_milliseconds(300),
    );
    assert_eq!(
        client
            .q3_client_snapshot(number as i32)
            .unwrap()
            .unwrap()
            .ping,
        280
    );
}
