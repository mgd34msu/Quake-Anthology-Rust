use qa_core::{loopback::Endpoint, primitives::Vec3};
use qa_network::{
    channel::Channel,
    commands::{
        Q1Move,
        connection::Commands,
        packet::{self, Key, Move, Protocol, ZERO_Q2, ZERO_Q2_RR, ZERO_Q3, ZERO_QW},
    },
    ingress::Incoming,
};
use std::ops::ControlFlow;

#[derive(Debug, PartialEq)]
enum Seen {
    Text(Vec<u8>),
    Move(i32),
    Userinfo(Vec<u8>),
    Disconnect,
}

fn movement(protocol: Protocol, value: i16) -> Move {
    match protocol {
        Protocol::NetQuake15 => Move::NetQuake {
            timestamp: 0.,
            command: Q1Move {
                view_angles: Vec3::default(),
                movement: [value, 0, 0],
                buttons: 0,
                impulse: 0,
            },
        },
        Protocol::QuakeWorld28 => {
            let mut commands = [ZERO_QW; 3];
            commands[2].movement[0] = value;
            commands[2].msec = 8;
            Move::QuakeWorld {
                loss: 0,
                commands,
                delta_request: None,
            }
        }
        Protocol::Quake2_34 => {
            let mut commands = [ZERO_Q2; 3];
            commands[2].movement[0] = value;
            commands[2].msec = 8;
            Move::Quake2 {
                last_frame: -1,
                commands,
            }
        }
        Protocol::Quake2Repro1038 => {
            let mut commands = [ZERO_Q2_RR; 3];
            commands[2].movement[0] = f32::from(value);
            commands[2].msec = 8;
            Move::Quake2Repro {
                last_frame: -1,
                commands,
            }
        }
        Protocol::Quake3_68 => {
            let mut commands = [ZERO_Q3; 32];
            commands[0].movement[0] = value as i8;
            commands[0].server_time = 8;
            Move::Quake3 {
                commands,
                count: 1,
                delta: false,
            }
        }
    }
}

fn move_bytes(protocol: Protocol, value: i16) -> Vec<u8> {
    let mut bytes = [0; 1400];
    let length = packet::write(&mut bytes, &movement(protocol, value), 7, Key::default()).unwrap();
    bytes[..length].to_vec()
}
fn string(bytes: &mut Vec<u8>, text: &[u8]) {
    bytes.push(4);
    bytes.extend_from_slice(text);
    bytes.push(0);
}
fn seen(incoming: Incoming<'_>) -> Seen {
    match incoming {
        Incoming::CommandString { text, .. } => Seen::Text(text.to_vec()),
        Incoming::Command {
            command, execution, ..
        } => {
            if execution == packet::CommandExecution::FrameLatest {
                assert_eq!(command.duration_ms, 0);
                assert_eq!(command.duration_ns, 0);
            }
            Seen::Move(command.movement[0] as i32)
        }
        Incoming::Userinfo { text, .. } => Seen::Userinfo(text.to_vec()),
        Incoming::Disconnect { .. } => Seen::Disconnect,
        _ => panic!("unexpected client record"),
    }
}
fn deliver(protocol: Protocol, bytes: &[u8]) -> (Result<usize, packet::Error>, Vec<Seen>) {
    let mut channel = Channel::load(protocol.channel(), Endpoint::Server, 8192, 16).unwrap();
    let mut commands = Commands::load(protocol);
    let length = commands.stage(bytes).unwrap();
    let mut output = Vec::new();
    let result = commands.decode(length, 7, 16, &mut channel, None, |incoming| {
        output.push(seen(incoming));
        ControlFlow::Continue(())
    });
    (result, output)
}

#[test]
fn legacy_text_and_moves_keep_wire_order_and_non_utf8_bytes() {
    for protocol in [
        Protocol::NetQuake15,
        Protocol::QuakeWorld28,
        Protocol::Quake2_34,
        Protocol::Quake2Repro1038,
    ] {
        let mut bytes = vec![1];
        string(&mut bytes, b"before \x80");
        bytes.extend(move_bytes(protocol, 11));
        string(&mut bytes, b"after \x81");
        let (result, output) = deliver(protocol, &bytes);
        assert_eq!(result, Ok(1), "{protocol:?}");
        assert_eq!(
            output,
            [
                Seen::Text(b"before \x80".to_vec()),
                Seen::Move(11),
                Seen::Text(b"after \x81".to_vec())
            ]
        );
    }
}

#[test]
fn string_only_packets_are_delivered_without_a_move() {
    for protocol in [
        Protocol::NetQuake15,
        Protocol::QuakeWorld28,
        Protocol::Quake2_34,
        Protocol::Quake2Repro1038,
    ] {
        let mut bytes = Vec::new();
        string(&mut bytes, b"say hello");
        assert_eq!(
            deliver(protocol, &bytes),
            (Ok(0), vec![Seen::Text(b"say hello".to_vec())])
        );
    }
}

#[test]
fn bad_move_crc_keeps_prefix_text_and_rejects_only_the_move_and_tail() {
    for protocol in [Protocol::QuakeWorld28, Protocol::Quake2_34] {
        let mut bytes = Vec::new();
        string(&mut bytes, b"before");
        let mut movement = move_bytes(protocol, 11);
        movement[1] ^= 1;
        bytes.extend(movement);
        string(&mut bytes, b"after");
        let (result, output) = deliver(protocol, &bytes);
        assert_eq!(result, Err(packet::Error::Checksum));
        assert_eq!(output, [Seen::Text(b"before".to_vec())]);
    }
}

#[test]
fn second_move_stops_without_retracting_prior_native_effects() {
    for protocol in [
        Protocol::QuakeWorld28,
        Protocol::Quake2_34,
        Protocol::Quake2Repro1038,
    ] {
        let mut bytes = move_bytes(protocol, 11);
        string(&mut bytes, b"between");
        bytes.push(if protocol == Protocol::QuakeWorld28 {
            3
        } else {
            2
        });
        // The second move body is deliberately invalid and never read natively.
        bytes.extend([255, 255, 255]);
        string(&mut bytes, b"after");
        let (result, output) = deliver(protocol, &bytes);
        assert_eq!(result, Ok(1));
        assert_eq!(output, [Seen::Move(11), Seen::Text(b"between".to_vec())]);
    }
}

#[test]
fn q2_native_string_limits_skip_excess_text_and_preserve_later_move() {
    for (protocol, maximum) in [(Protocol::Quake2_34, 7), (Protocol::Quake2Repro1038, 8)] {
        let mut bytes = Vec::new();
        for i in 0..10 {
            string(&mut bytes, &[b'a' + i]);
        }
        bytes.extend(move_bytes(protocol, 11));
        let (result, output) = deliver(protocol, &bytes);
        assert_eq!(result, Ok(1));
        let expected: Vec<_> = (0..maximum)
            .map(|i| Seen::Text(vec![b'a' + i]))
            .chain([Seen::Move(11)])
            .collect();
        assert_eq!(output, expected);
    }
}

#[test]
fn nq_keeps_each_move_at_its_position_for_latest_per_frame_consumption() {
    let mut bytes = move_bytes(Protocol::NetQuake15, 11);
    string(&mut bytes, b"between");
    bytes.extend(move_bytes(Protocol::NetQuake15, 22));
    assert_eq!(
        deliver(Protocol::NetQuake15, &bytes),
        (
            Ok(2),
            vec![
                Seen::Move(11),
                Seen::Text(b"between".to_vec()),
                Seen::Move(22)
            ]
        )
    );
}

#[test]
fn native_disconnect_and_callback_stop_leave_the_tail_unexecuted() {
    let mut nq = vec![2];
    nq.extend(move_bytes(Protocol::NetQuake15, 11));
    assert_eq!(
        deliver(Protocol::NetQuake15, &nq),
        (Ok(0), vec![Seen::Disconnect])
    );
    for protocol in [
        Protocol::QuakeWorld28,
        Protocol::Quake2_34,
        Protocol::Quake2Repro1038,
    ] {
        let mut bytes = Vec::new();
        string(&mut bytes, b"disconnect");
        bytes.extend(move_bytes(protocol, 11));
        let mut channel = Channel::load(protocol.channel(), Endpoint::Server, 8192, 16).unwrap();
        let mut commands = Commands::load(protocol);
        let length = commands.stage(&bytes).unwrap();
        let mut output = Vec::new();
        assert_eq!(
            commands.decode(length, 7, 16, &mut channel, None, |incoming| {
                output.push(seen(incoming));
                ControlFlow::Break(())
            }),
            Ok(0)
        );
        assert_eq!(output, [Seen::Text(b"disconnect".to_vec())]);
    }
}

fn q3_text_move(strings: &[(u32, &[u8])]) -> Vec<u8> {
    let mut bytes = [0; 1400];
    let length = packet::write_with_commands(
        &mut bytes,
        &movement(Protocol::Quake3_68, 11),
        7,
        Key::default(),
        |writer| {
            for &(sequence, text) in strings {
                writer.write_bits(4, 8)?;
                writer.write_bits(sequence, 32)?;
                for &byte in text {
                    writer.write_bits(u32::from(byte), 8)?;
                }
                writer.write_bits(0, 8)?;
            }
            Ok(())
        },
    )
    .unwrap();
    bytes[..length].to_vec()
}

#[test]
fn q3_new_reliable_strings_precede_moves_and_duplicates_never_execute_twice() {
    let bytes = q3_text_move(&[(1, b"say \x80%")]);
    let mut channel =
        Channel::load(Protocol::Quake3_68.channel(), Endpoint::Server, 8192, 16).unwrap();
    let mut commands = Commands::load(Protocol::Quake3_68);
    for (expected_count, expected) in [
        (1, vec![Seen::Text(b"say ..".to_vec()), Seen::Move(11)]),
        (0, vec![]),
    ] {
        let length = commands.stage(&bytes).unwrap();
        let mut output = Vec::new();
        assert_eq!(
            commands.decode(length, 7, 16, &mut channel, None, |incoming| {
                output.push(seen(incoming));
                ControlFlow::Continue(())
            }),
            Ok(expected_count)
        );
        assert_eq!(output, expected);
    }
    let bytes = q3_text_move(&[(2, b"accepted"), (4, b"lost")]);
    let length = commands.stage(&bytes).unwrap();
    let mut output = Vec::new();
    assert_eq!(
        commands.decode(length, 7, 16, &mut channel, None, |incoming| {
            output.push(seen(incoming));
            ControlFlow::Continue(())
        }),
        Err(packet::Error::Context)
    );
    assert_eq!(output, [Seen::Text(b"accepted".to_vec())]);
}

#[test]
fn q3_duplicate_record_count_is_not_an_extra_per_packet_limit() {
    let strings = vec![(0, b"".as_slice()); 70];
    let bytes = q3_text_move(&strings);
    assert_eq!(
        deliver(Protocol::Quake3_68, &bytes),
        (Ok(1), vec![Seen::Move(11)])
    );
}

#[test]
fn q2_userinfo_is_a_distinct_byte_preserving_record_with_rerelease_length_limit() {
    for protocol in [Protocol::Quake2_34, Protocol::Quake2Repro1038] {
        let mut bytes = vec![3];
        bytes.extend(b"\\name\\\x80\0");
        bytes.extend(move_bytes(protocol, 11));
        assert_eq!(
            deliver(protocol, &bytes),
            (
                Ok(1),
                vec![Seen::Userinfo(b"\\name\\\x80".to_vec()), Seen::Move(11)]
            )
        );
    }
    let mut bytes = vec![3];
    bytes.extend([b'a'; 1024]);
    bytes.push(0);
    assert_eq!(
        deliver(Protocol::Quake2Repro1038, &bytes),
        (Err(packet::Error::Count), vec![])
    );
}
