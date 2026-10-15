use qa_core::{
    loopback::Endpoint,
    primitives::{UserCmd, Vec3, WeaponId},
    sys_events::EventTime,
};
use qa_network::{
    channel::{Channel, MoveCommand},
    commands::{
        connection::Commands,
        packet::{self, Key, Move, Protocol},
    },
};

fn command(number: i32) -> UserCmd {
    UserCmd {
        server_time_ms: number * 16,
        duration_ms: 16,
        view_angles: Vec3([(number % 16) as f32 * 11.25, -45., 90.]),
        movement: [number as f32, -17., 0.],
        units: qa_gameplay::rules::command_units(qa_core::primitives::RuleSetId::Quake),
        weapon: Some(WeaponId(1000 + number as u32)),
        ..UserCmd::default()
    }
}
fn weapon(id: Option<WeaponId>) -> u8 {
    (id.unwrap().0 - 1000) as u8
}

#[test]
fn legacy_packets_project_three_actual_commands_in_native_order() {
    for protocol in [
        Protocol::QuakeWorld28,
        Protocol::Quake2_34,
        Protocol::Quake2Repro1038,
    ] {
        let channel = Channel::load(protocol.channel(), Endpoint::Client, 8192, 8).unwrap();
        let mut codec = Commands::load(protocol, 8192);
        codec.configure_history(8193).unwrap();
        for number in 1..=101 {
            codec.record(command(number)).unwrap();
        }
        let mut bytes = [0; 1400];
        let n = codec
            .encode(&command(101), weapon, &channel, &mut bytes)
            .unwrap();
        let decoded = packet::read(
            protocol,
            &mut bytes[..n],
            channel.send_state().sequence,
            Key::default(),
        )
        .unwrap();
        let expected = [command(99), command(100), command(101)];
        match decoded {
            Move::QuakeWorld { commands, .. } => assert_eq!(
                commands,
                expected.map(|command| qa_network::commands::to_qw_usercmd(&command))
            ),
            Move::Quake2 { commands, .. } => assert_eq!(
                commands,
                expected.map(|command| qa_network::commands::to_q2_usercmd(&command))
            ),
            Move::Quake2Repro { commands, .. } => assert_eq!(
                commands,
                expected.map(|command| qa_network::commands::to_q2_rr_usercmd(&command, 0))
            ),
            _ => panic!("legacy movement"),
        }
    }
}

fn q3_encode(codec: &Commands, channel: &Channel, latest: i32) -> Vec<u8> {
    let mut bytes = [0; packet::Q3_MESSAGE_BYTES];
    let n = codec
        .encode(&command(latest), weapon, channel, &mut bytes)
        .unwrap();
    bytes[..n].to_vec()
}
fn q3_check(bytes: &[u8], sequence: u32, numbers: std::ops::RangeInclusive<i32>) {
    let mut bytes = bytes.to_vec();
    let Move::Quake3 {
        commands, count, ..
    } = packet::read(Protocol::Quake3_68, &mut bytes, sequence, Key::default()).unwrap()
    else {
        panic!("Q3 movement")
    };
    let numbers = numbers.collect::<Vec<_>>();
    assert_eq!(usize::from(count), numbers.len());
    for (native, &number) in commands[..usize::from(count)].iter().zip(&numbers) {
        assert_eq!(
            *native,
            qa_network::commands::to_q3_usercmd(&command(number), number as u8)
        );
    }
}
fn submit(channel: &mut Channel, bytes: &[u8], number: u64) {
    channel
        .prepare_move(
            bytes,
            EventTime(number),
            None,
            Some(MoveCommand {
                number,
                server_time_ms: number as i32 * 16,
            }),
        )
        .unwrap()
        .unwrap();
    loop {
        channel.submitted(EventTime(number)).unwrap();
        if !channel.pending_fragments() {
            break;
        }
        channel.prepare_output(EventTime(number)).unwrap().unwrap();
    }
}

#[test]
fn q3_packet_duplicates_backlog_and_weapon_changes_use_one_history() {
    for (setting, duplicate) in [(-99, 0), (0, 0), (1, 1), (5, 5), (99, 5)] {
        let mut channel = Channel::load(
            Protocol::Quake3_68.channel(),
            Endpoint::Client,
            packet::Q3_MESSAGE_BYTES,
            8,
        )
        .unwrap();
        let mut codec = Commands::load(Protocol::Quake3_68, packet::Q3_MESSAGE_BYTES);
        codec.configure_history(8193).unwrap();
        codec.set_packet_dup(setting);
        for number in 1..=8 {
            codec.record(command(number)).unwrap();
            let bytes = q3_encode(&codec, &channel, number);
            q3_check(
                &bytes,
                channel.send_state().sequence,
                (number - duplicate).max(1)..=number,
            );
            submit(&mut channel, &bytes, number as u64);
        }
        // Many commands can be produced between packets; the native packet
        // limit does not shrink or replace the much larger common history.
        for number in 9..=80 {
            codec.record(command(number)).unwrap();
        }
        let bytes = q3_encode(&codec, &channel, 80);
        q3_check(&bytes, channel.send_state().sequence, 49..=80);
        assert_eq!(
            (
                codec.history().unwrap().capacity(),
                codec.history().unwrap().len()
            ),
            (8193, 80)
        );
    }
}

#[test]
fn rejected_and_partial_flights_do_not_publish_new_command_numbers() {
    let mut channel = Channel::load(
        Protocol::Quake3_68.channel(),
        Endpoint::Client,
        packet::Q3_MESSAGE_BYTES,
        8,
    )
    .unwrap();
    let mut codec = Commands::load(Protocol::Quake3_68, packet::Q3_MESSAGE_BYTES);
    codec.configure_history(4096).unwrap();
    codec.record(command(1)).unwrap();
    for _ in 0..6 {
        channel.queue_reliable(&[b'x'; 900]).unwrap();
    }
    let bytes = q3_encode(&codec, &channel, 1);
    let sequence = channel.send_state().sequence;
    channel
        .prepare_move(
            &bytes,
            EventTime(1),
            None,
            Some(MoveCommand {
                number: 1,
                server_time_ms: 16,
            }),
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        channel
            .submitted_command(sequence)
            .map(|command| command.number),
        None
    );
    assert!(
        channel
            .submit_with(EventTime(1), |_| false)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        channel
            .submitted_command(sequence)
            .map(|command| command.number),
        None
    );
    let mut fragments = 0;
    loop {
        channel.submitted(EventTime(2)).unwrap();
        fragments += 1;
        if !channel.pending_fragments() {
            break;
        }
        assert_eq!(
            channel
                .submitted_command(sequence)
                .map(|command| command.number),
            None
        );
        channel.prepare_output(EventTime(2)).unwrap().unwrap();
    }
    assert!(fragments > 1);
    assert_eq!(
        channel
            .submitted_command(sequence)
            .map(|command| command.number),
        Some(1)
    );
    codec.record(command(2)).unwrap();
    codec.set_packet_dup(0);
    q3_check(
        &q3_encode(&codec, &channel, 2),
        channel.send_state().sequence,
        2..=2,
    );
}

#[test]
fn native_receive_recovers_lost_moves_from_the_retained_commands() {
    for protocol in [
        Protocol::QuakeWorld28,
        Protocol::Quake2_34,
        Protocol::Quake2Repro1038,
        Protocol::Quake3_68,
    ] {
        let mut client = Channel::load(
            protocol.channel(),
            Endpoint::Client,
            packet::Q3_MESSAGE_BYTES,
            8,
        )
        .unwrap();
        let mut server = Channel::load(
            protocol.channel(),
            Endpoint::Server,
            packet::Q3_MESSAGE_BYTES,
            8,
        )
        .unwrap();
        let mut outgoing = Commands::load(protocol, packet::Q3_MESSAGE_BYTES);
        outgoing.configure_history(4096).unwrap();
        outgoing.set_packet_dup(5);
        let mut incoming = Commands::load(protocol, packet::Q3_MESSAGE_BYTES);
        let mut moves = Vec::new();
        for number in 1..=4 {
            let command = UserCmd {
                units: qa_gameplay::rules::command_units(protocol.rule_set()),
                ..command(number)
            };
            outgoing.record(command).unwrap();
            let mut bytes = [0; packet::Q3_MESSAGE_BYTES];
            let length = outgoing
                .encode(&command, weapon, &client, &mut bytes)
                .unwrap();
            let time = EventTime::from_milliseconds(number as u64 * 16);
            let packet = client
                .prepare_move(
                    &bytes[..length],
                    time,
                    None,
                    Some(MoveCommand {
                        number: number as u64,
                        server_time_ms: number * 16,
                    }),
                )
                .unwrap()
                .unwrap()
                .bytes
                .to_vec();
            client.submitted(time).unwrap();
            if matches!(number, 2 | 3) {
                continue;
            }
            let received = server.receive(&packet, time).unwrap();
            let qa_network::channel::Delivery::Payload(bytes) = received.delivery else {
                panic!("connected movement")
            };
            let length = incoming.stage(bytes).unwrap();
            incoming
                .decode(
                    length,
                    received.header.sequence,
                    number * 16,
                    &mut server,
                    None,
                    |record| {
                        if let qa_network::ingress::Incoming::Command { command, .. } = record {
                            moves.push(command.movement[0]);
                        }
                        std::ops::ControlFlow::Continue(())
                    },
                )
                .unwrap();
        }
        assert_eq!(moves, [1., 2., 3., 4.], "{protocol:?}");
    }
}

#[test]
fn submitted_command_metadata_is_shared_and_old_packet_aliases_expire() {
    for protocol in [
        Protocol::NetQuake15,
        Protocol::QuakeWorld28,
        Protocol::Quake2_34,
        Protocol::Quake2Repro1038,
        Protocol::Quake3_68,
    ] {
        let mut channel = Channel::load(
            protocol.channel(),
            Endpoint::Client,
            packet::Q3_MESSAGE_BYTES,
            8,
        )
        .unwrap();
        channel.configure_snapshots(protocol).unwrap();
        let mut codec = Commands::load(protocol, packet::Q3_MESSAGE_BYTES);
        codec.configure_history(4096).unwrap();
        codec.set_packet_dup(0);
        let first = channel.send_state().sequence;
        for number in 1..=34 {
            codec.record(command(number)).unwrap();
            let mut bytes = [0; packet::Q3_MESSAGE_BYTES];
            let length = codec
                .encode(&command(number), weapon, &channel, &mut bytes)
                .unwrap();
            let sequence = channel.send_state().sequence;
            submit(&mut channel, &bytes[..length], number as u64);
            let metadata = channel.submitted_command(sequence);
            if protocol == Protocol::NetQuake15 {
                assert_eq!(metadata, None);
            } else {
                assert_eq!(
                    metadata,
                    Some(MoveCommand {
                        number: number as u64,
                        server_time_ms: number * 16
                    })
                );
            }
        }
        assert_eq!(channel.submitted_command(first), None, "{protocol:?}");
    }
}
