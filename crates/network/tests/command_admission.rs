use qa_core::{
    loopback::Endpoint,
    primitives::{ClientId, UserCmd, Vec3, buttons},
    sys_events::{EventTime, Peer},
};
use qa_network::{
    channel::{Channel, Delivery},
    commands::{
        Q1Move, Q2Cmd, Q2RrCmd, Q3Cmd, QwCmd,
        connection::Commands,
        packet::{
            self, CommandExecution, Key, Move, Protocol, ZERO_Q2, ZERO_Q2_RR, ZERO_Q3, ZERO_QW,
        },
    },
    headers::{self, Direction, Header},
    ingress::{Connection, Connections, Incoming, Route},
    message::{Encoding, Writer},
};

fn payload(movement: &Move, sequence: u32) -> Vec<u8> {
    let mut bytes = [0; 1400];
    let length = packet::write(&mut bytes, movement, sequence, Key::default()).unwrap();
    bytes[..length].to_vec()
}

fn framed(protocol: Protocol, sequence: u32, payload: &[u8]) -> Vec<u8> {
    let mut bytes = [0; 1400];
    let length = headers::encode(
        match protocol {
            Protocol::NetQuake15 => headers::NETQUAKE,
            Protocol::QuakeWorld28 => headers::QUAKEWORLD,
            Protocol::Quake2_34 => headers::QUAKE2,
            Protocol::Quake2Repro1038 => headers::q2_new(false),
            Protocol::Quake3_68 => headers::QUAKE3,
        },
        Direction::ToServer,
        Header {
            sequence,
            datagram_flags: headers::datagram::UNRELIABLE,
            ..Header::default()
        },
        payload,
        &mut bytes,
    )
    .unwrap();
    bytes[..length].to_vec()
}

fn admit(
    codec: &mut Commands,
    channel: &mut Channel,
    movement: &Move,
    sequence: u32,
) -> Vec<UserCmd> {
    admit_bytes(codec, channel, &payload(movement, sequence), sequence)
}

fn admit_bytes(
    codec: &mut Commands,
    channel: &mut Channel,
    body: &[u8],
    sequence: u32,
) -> Vec<UserCmd> {
    let bytes = framed(codec.protocol, sequence, body);
    let received = channel.receive(&bytes, EventTime(123)).unwrap();
    let Delivery::Payload(bytes) = received.delivery else {
        panic!("complete move");
    };
    let length = codec.stage(bytes).unwrap();
    let mut commands = Vec::new();
    let accepted = codec
        .decode(length, sequence, 123, channel, |command| {
            commands.push(command)
        })
        .unwrap();
    assert_eq!(accepted, commands.len());
    commands
}

/// Full timestamps exercise restart filtering. The native writer's short
/// delta form wraps backwards times into an unsigned byte.
fn full_q3_times(times: &[i32]) -> Vec<u8> {
    let mut bytes = [0; 1400];
    let mut writer = Writer::new(&mut bytes, Encoding::Q3);
    for _ in 0..3 {
        writer.write_bits(0, 32).unwrap();
    }
    writer.write_bits(2, 8).unwrap(); // clc_move, delta request zero.
    writer.write_bits(times.len() as u32, 8).unwrap();
    for &time in times {
        writer.write_bits(0, 1).unwrap(); // Full native timestamp.
        writer.write_bits(time as u32, 32).unwrap();
        writer.write_bits(0, 1).unwrap(); // Other fields stay zero.
    }
    writer.write_bits(5, 8).unwrap(); // clc_EOF.
    writer.bytes().to_vec()
}

fn native_move(protocol: Protocol, moves: [(u8, i16); 3]) -> Move {
    match protocol {
        Protocol::QuakeWorld28 => Move::QuakeWorld {
            loss: 99, // The client loss report does not choose SERVER recovery.
            commands: moves.map(|(msec, movement)| QwCmd {
                msec,
                movement: [movement, 0, 0],
                buttons: 1,
                impulse: 9,
                ..ZERO_QW
            }),
            delta_request: Some(3),
        },
        Protocol::Quake2_34 => Move::Quake2 {
            last_frame: 3,
            commands: moves.map(|(msec, movement)| Q2Cmd {
                msec,
                movement: [movement, 0, 0],
                buttons: 1,
                impulse: 9,
                light_level: 13,
                ..ZERO_Q2
            }),
        },
        Protocol::Quake2Repro1038 => Move::Quake2Repro {
            last_frame: 3,
            commands: moves.map(|(msec, movement)| Q2RrCmd {
                msec,
                movement: [f32::from(movement), 0.],
                buttons: 1,
                ..ZERO_Q2_RR
            }),
        },
        _ => panic!("backup protocol"),
    }
}

#[test]
fn native_recovery_obeys_packet_gaps_and_retains_only_one_previous_command() {
    for protocol in [
        Protocol::QuakeWorld28,
        Protocol::Quake2_34,
        Protocol::Quake2Repro1038,
    ] {
        assert_eq!(protocol.command_execution(), CommandExecution::PerCommand);
        for dropped in [0, 1, 2, 3, 19, 20] {
            let mut channel =
                Channel::load(protocol.channel(), Endpoint::Server, 8192, 16).unwrap();
            let mut codec = Commands::load(protocol);
            let first = admit(
                &mut codec,
                &mut channel,
                &native_move(protocol, [(0, 0), (0, 0), (7, 42)]),
                1,
            );
            assert_eq!(first.len(), 1);
            let commands = admit(
                &mut codec,
                &mut channel,
                &native_move(protocol, [(11, 1), (13, 2), (17, 3)]),
                dropped + 2,
            );
            assert_eq!(channel.state().dropped, dropped);
            let expected: Vec<(u16, i16)> = match dropped {
                0 | 20 => vec![(17, 3)],
                1 => vec![(13, 2), (17, 3)],
                2 => vec![(11, 1), (13, 2), (17, 3)],
                3 => vec![(7, 42), (11, 1), (13, 2), (17, 3)],
                19 => {
                    let mut expected = vec![(7, 42); 17];
                    expected.extend([(11, 1), (13, 2), (17, 3)]);
                    expected
                }
                _ => unreachable!(),
            };
            let mut time = 7;
            assert_eq!(commands.len(), expected.len());
            for (command, (msec, movement)) in commands.iter().zip(expected) {
                time += i32::from(msec);
                assert_eq!(command.server_time_ms, time);
                assert_eq!(command.duration_ms, msec);
                assert_eq!(command.movement[0], f32::from(movement));
                assert_eq!(
                    command.buttons & buttons::ATTACK,
                    if movement == 42 && protocol == Protocol::QuakeWorld28 {
                        0
                    } else {
                        buttons::ATTACK
                    }
                );
                if protocol != Protocol::Quake2Repro1038 {
                    assert_eq!(command.impulse, 9);
                }
            }
            // The next gap repeats this packet's new command, not older backup entries.
            let next = admit(
                &mut codec,
                &mut channel,
                &native_move(protocol, [(19, 4), (23, 5), (29, 6)]),
                dropped + 6,
            );
            assert_eq!(
                next.iter()
                    .map(|command| command.movement[0])
                    .collect::<Vec<_>>(),
                [3., 4., 5., 6.]
            );
            assert_eq!(
                next[0].buttons & buttons::ATTACK,
                if protocol == Protocol::QuakeWorld28 {
                    0
                } else {
                    buttons::ATTACK
                }
            );
        }
    }
}

fn q3_move(times: &[i32], delta: bool) -> Move {
    let mut commands = [ZERO_Q3; 32];
    for (index, &time) in times.iter().enumerate() {
        commands[index] = Q3Cmd {
            server_time: time,
            movement: [index as i8 + 1, 0, 0],
            buttons: if index & 1 == 0 { 1 } else { 4 },
            ..ZERO_Q3
        };
    }
    Move::Quake3 {
        commands,
        count: times.len() as u8,
        delta,
    }
}

#[test]
fn q3_admits_every_new_command_in_order_with_native_timestamp_filters() {
    let protocol = Protocol::Quake3_68;
    let mut channel = Channel::load(protocol.channel(), Endpoint::Server, 8192, 16).unwrap();
    let mut codec = Commands::load(protocol);
    assert_eq!(
        admit(&mut codec, &mut channel, &q3_move(&[100], false), 1).len(),
        1
    );
    let commands = admit(
        &mut codec,
        &mut channel,
        &q3_move(&[90, 110, 110, 130], true),
        2,
    );
    assert_eq!(
        commands
            .iter()
            .map(|command| command.server_time_ms)
            .collect::<Vec<_>>(),
        [110, 130]
    );
    assert_eq!(
        commands
            .iter()
            .map(|command| command.duration_ms)
            .collect::<Vec<_>>(),
        [10, 20]
    );
    assert_eq!(
        commands
            .iter()
            .map(|command| command.movement[0])
            .collect::<Vec<_>>(),
        [2., 4.]
    );
    assert_eq!(codec.delta_request(), Some(0));
    // A pre-map-restart command newer than the packet's final command is skipped.
    let commands = admit_bytes(&mut codec, &mut channel, &full_q3_times(&[150, 140]), 3);
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].server_time_ms, 140);
    assert!(admit(&mut codec, &mut channel, &q3_move(&[130, 140], false), 4).is_empty());
    assert_eq!(codec.delta_request(), None); // Updated even when every command is old.
    let commands = admit_bytes(
        &mut codec,
        &mut channel,
        &full_q3_times(&[130, 145, 142, 148]),
        5,
    );
    assert_eq!(
        commands
            .iter()
            .map(|command| command.server_time_ms)
            .collect::<Vec<_>>(),
        [145, 148]
    );
    assert_eq!(
        commands
            .iter()
            .map(|command| command.duration_ms)
            .collect::<Vec<_>>(),
        [5, 3]
    );
    let times: Vec<_> = (0..32).map(|index| 150 + index * 10).collect();
    let commands = admit(&mut codec, &mut channel, &q3_move(&times, true), 25);
    assert_eq!(commands.len(), 32); // A channel gap does not repeat Q3 commands.
    assert_eq!(
        commands
            .iter()
            .map(|command| command.server_time_ms)
            .collect::<Vec<_>>(),
        times
    );
}

#[test]
fn malformed_or_bad_crc_moves_never_reach_the_command_callback() {
    for protocol in [
        Protocol::QuakeWorld28,
        Protocol::Quake2_34,
        Protocol::Quake2Repro1038,
        Protocol::Quake3_68,
    ] {
        let mut channel = Channel::load(protocol.channel(), Endpoint::Server, 8192, 16).unwrap();
        let mut codec = Commands::load(protocol);
        let movement = if protocol == Protocol::Quake3_68 {
            q3_move(&[10, 20], false)
        } else {
            native_move(protocol, [(11, 1), (13, 2), (17, 3)])
        };
        let bytes = payload(&movement, 1);
        let length = codec.stage(&bytes[..bytes.len() - 1]).unwrap();
        let mut accepted = 0;
        assert!(
            codec
                .decode(length, 1, 0, &mut channel, |_| accepted += 1)
                .is_err()
        );
        assert_eq!(accepted, 0);
        if matches!(protocol, Protocol::QuakeWorld28 | Protocol::Quake2_34) {
            let mut bytes = bytes;
            bytes[1] ^= 1; // Protected checksum byte.
            let length = codec.stage(&bytes).unwrap();
            assert_eq!(
                codec.decode(length, 1, 0, &mut channel, |_| accepted += 1),
                Err(packet::Error::Checksum)
            );
            assert_eq!(accepted, 0);
        }
    }
}

#[test]
fn ingress_dispatches_command_batches_synchronously_and_preserves_nq_frame_policy() {
    for protocol in [Protocol::NetQuake15, Protocol::Quake3_68] {
        let client = ClientId(0);
        let mut connections = Connections::load(1);
        connections
            .bind(
                client,
                Endpoint::Server,
                Connection {
                    route: Route {
                        socket: Endpoint::Server.socket(),
                        peer: Peer::Loopback(client),
                    },
                    channel: Channel::load(protocol.channel(), Endpoint::Server, 8192, 16).unwrap(),
                    output: None,
                    pending_output: None,
                    commands: Some(Commands::load(protocol)),
                },
            )
            .unwrap();
        let mut received = Vec::new();
        for sequence in [1, 2] {
            let movement = if protocol == Protocol::NetQuake15 {
                Move::NetQuake {
                    timestamp: 9876.,
                    command: Q1Move {
                        view_angles: Vec3([0.; 3]),
                        movement: [sequence as i16, 0, 0],
                        buttons: 0,
                        impulse: 0,
                    },
                }
            } else {
                q3_move(
                    &[
                        sequence as i32 * 30 - 20,
                        sequence as i32 * 30 - 10,
                        sequence as i32 * 30,
                    ],
                    false,
                )
            };
            let bytes = framed(protocol, sequence, &payload(&movement, sequence));
            connections.receive(
                Endpoint::Server.socket(),
                Peer::Loopback(client),
                &bytes,
                EventTime(123),
                |id, endpoint, incoming| {
                    assert_eq!((id, endpoint), (client, Endpoint::Server));
                    if let Incoming::Command {
                        command, execution, ..
                    } = incoming
                    {
                        assert_eq!(execution, protocol.command_execution());
                        received.push(command);
                    }
                },
            );
            assert_eq!(connections.commands as usize, received.len());
            // The accepted channel suppresses the duplicate before decoding.
            connections.receive(
                Endpoint::Server.socket(),
                Peer::Loopback(client),
                &bytes,
                EventTime(123),
                |_, _, _| panic!("duplicate command"),
            );
        }
        if protocol == Protocol::NetQuake15 {
            assert_eq!(received.len(), 2);
            assert_eq!(protocol.command_execution(), CommandExecution::FrameLatest);
            assert_eq!(
                received
                    .iter()
                    .map(|command| command.movement[0])
                    .collect::<Vec<_>>(),
                [1., 2.]
            );
            assert!(
                received
                    .iter()
                    .all(|command| command.duration_ns == 0 && command.duration_ms == 0)
            );
        } else {
            assert_eq!(
                received
                    .iter()
                    .map(|command| command.server_time_ms)
                    .collect::<Vec<_>>(),
                [10, 20, 30, 40, 50, 60]
            );
        }
    }
}
