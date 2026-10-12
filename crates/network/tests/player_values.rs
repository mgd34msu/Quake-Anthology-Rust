use qa_core::{
    loopback::Endpoint,
    primitives::{ClientId, PrintKind, ResourceNamespace, Vec3},
    sys_events::EventTime,
};
use qa_network::{
    channel::{Channel, Delivery},
    commands::{connection::Commands, packet::Protocol},
    ingress::Incoming,
    outputs::{self, Error},
};
use std::ops::ControlFlow;

#[test]
fn native_stat_literals_keep_signed_width_and_qw_byte_selection() {
    for (protocol, value, expected) in [
        (Protocol::NetQuake15, 0, &b"\x03\x1f\x00\x00\x00\x00"[..]),
        (Protocol::NetQuake15, 255, &b"\x03\x1f\xff\x00\x00\x00"[..]),
        (Protocol::NetQuake15, -1, &b"\x03\x1f\xff\xff\xff\xff"[..]),
        (
            Protocol::NetQuake15,
            i32::MIN,
            &b"\x03\x1f\x00\x00\x00\x80"[..],
        ),
        (
            Protocol::NetQuake15,
            i32::MAX,
            &b"\x03\x1f\xff\xff\xff\x7f"[..],
        ),
        (Protocol::QuakeWorld28, 0, &b"\x03\x1f\x00"[..]),
        (Protocol::QuakeWorld28, 255, &b"\x03\x1f\xff"[..]),
        (
            Protocol::QuakeWorld28,
            256,
            &b"\x26\x1f\x00\x01\x00\x00"[..],
        ),
        (Protocol::QuakeWorld28, -1, &b"\x26\x1f\xff\xff\xff\xff"[..]),
        (
            Protocol::QuakeWorld28,
            i32::MIN,
            &b"\x26\x1f\x00\x00\x00\x80"[..],
        ),
        (
            Protocol::QuakeWorld28,
            i32::MAX,
            &b"\x26\x1f\xff\xff\xff\x7f"[..],
        ),
    ] {
        let mut output = [0xcc; 8];
        let length = outputs::player_value(protocol, 31, value, &mut output).unwrap();
        assert_eq!(&output[..length], expected);
        assert_eq!(output[length], 0xcc);
        assert_eq!(
            outputs::read_player_value(protocol, expected),
            Ok(((31, value), expected.len()))
        );
        for cut in 0..expected.len() {
            assert_eq!(
                outputs::read_player_value(protocol, &expected[..cut]),
                Err(Error::Truncated)
            );
        }
        let mut short = [0xcc; 6];
        assert!(outputs::player_value(protocol, 31, value, &mut short[..length - 1]).is_err());
        assert_eq!(short, [0xcc; 6]);
    }
    // The retail receiver also accepts a small value carried by the long service.
    assert_eq!(
        outputs::read_player_value(Protocol::QuakeWorld28, &[38, 0, 7, 0, 0, 0, 1]),
        Ok(((0, 7), 6))
    );
    for protocol in [Protocol::NetQuake15, Protocol::QuakeWorld28] {
        for index in [32, 255] {
            let mut output = [0xcc; 6];
            assert_eq!(
                outputs::player_value(protocol, index, 0, &mut output),
                Err(Error::Unsupported)
            );
            assert_eq!(output, [0xcc; 6]);
            assert_eq!(
                outputs::read_player_value(protocol, &[3, index, 0, 0, 0, 0]),
                Err(Error::Unsupported)
            );
        }
    }
    assert_eq!(
        outputs::read_player_value(Protocol::NetQuake15, &[38, 0, 0, 0, 0, 0]),
        Err(Error::Opcode)
    );
}

#[test]
fn native_setangle_literals_keep_distinct_rounding_and_signed_byte_reading() {
    for (protocol, expected, angles) in [
        (Protocol::NetQuake15, [10, 0, 0, 192], Vec3([0., 0., -90.])),
        (
            Protocol::QuakeWorld28,
            [10, 1, 255, 192],
            Vec3([1.40625, -1.40625, -90.]),
        ),
    ] {
        let mut output = [0xcc; 5];
        assert_eq!(
            outputs::set_angles(protocol, Vec3([1.9, -1.9, 270.]), &mut output),
            Ok(4)
        );
        assert_eq!(&output[..4], &expected);
        assert_eq!(output[4], 0xcc);
        assert_eq!(outputs::read_set_angles(protocol, &output), Ok((angles, 4)));
        for cut in 0..4 {
            assert_eq!(
                outputs::read_set_angles(protocol, &expected[..cut]),
                Err(Error::Truncated)
            );
        }
        let mut short = [0xcc; 3];
        assert!(outputs::set_angles(protocol, Vec3::default(), &mut short).is_err());
        assert_eq!(short, [0xcc; 3]);
    }
    for protocol in [
        Protocol::Quake2_34,
        Protocol::Quake2Repro1038,
        Protocol::Quake3_68,
    ] {
        let mut output = [0xcc; 6];
        assert_eq!(
            outputs::player_value(protocol, 0, 0, &mut output),
            Err(Error::Unsupported)
        );
        assert_eq!(
            outputs::read_player_value(protocol, &[3, 0, 0]),
            Err(Error::Unsupported)
        );
        assert_eq!(
            outputs::set_angles(protocol, Vec3::default(), &mut output),
            Err(Error::Unsupported)
        );
        assert_eq!(
            outputs::read_set_angles(protocol, &[10, 0, 0, 0]),
            Err(Error::Unsupported)
        );
        assert_eq!(output, [0xcc; 6]);
    }
}

#[derive(Debug, PartialEq)]
enum Seen {
    Value(u8, i32),
    Angles(Vec3),
    Print(Vec<u8>),
}

#[test]
fn framed_stat_setangle_and_following_print_keep_native_ack_and_service_order() {
    for protocol in [Protocol::NetQuake15, Protocol::QuakeWorld28] {
        let mut server = Channel::load(protocol.channel(), Endpoint::Server, 8192, 4).unwrap();
        let mut client = Channel::load(protocol.channel(), Endpoint::Client, 8192, 4).unwrap();
        server.configure_snapshots(protocol).unwrap();
        client.configure_snapshots(protocol).unwrap();
        let mut server_commands = Commands::load(protocol, 8192);
        let mut client_commands = Commands::load(protocol, 8192);
        if protocol == Protocol::QuakeWorld28 {
            let request = client
                .prepare(None, EventTime(1))
                .unwrap()
                .unwrap()
                .bytes
                .to_vec();
            client.submitted(EventTime(1)).unwrap();
            let received = server.receive(&request, EventTime(2)).unwrap();
            let Delivery::Payload(bytes) = received.delivery else {
                panic!("native request")
            };
            let length = server_commands.stage(bytes).unwrap();
            assert_eq!(
                server_commands.decode(
                    length,
                    received.header.sequence,
                    0,
                    &mut server,
                    None,
                    |_| ControlFlow::Continue(())
                ),
                Ok(0)
            );
        }
        // Literal retail services, followed immediately by a native print.
        let mut body = if protocol == Protocol::NetQuake15 {
            vec![3, 0, 255, 255, 255, 255, 10, 64, 128, 255, 8]
        } else {
            vec![3, 0, 255, 38, 31, 0, 0, 0, 128, 10, 64, 128, 255, 8, 2]
        };
        body.extend_from_slice(b"after\0");
        let receipt = server.queue_reliable(&body).unwrap();
        let bytes = server
            .prepare(None, EventTime(3))
            .unwrap()
            .unwrap()
            .bytes
            .to_vec();
        server.submitted(EventTime(3)).unwrap();
        assert!(server.reliable_receipts().is_empty());
        let received = client.receive(&bytes, EventTime(4)).unwrap();
        let Delivery::Payload(bytes) = received.delivery else {
            panic!("native services")
        };
        let length = client_commands.stage(bytes).unwrap();
        let mut seen = Vec::new();
        assert_eq!(
            client_commands.decode_output(
                length,
                received.header.sequence,
                &mut client,
                ResourceNamespace::Client(ClientId(0)),
                |incoming| {
                    match incoming {
                        Incoming::PlayerValue { index, value } => {
                            seen.push(Seen::Value(index, value))
                        }
                        Incoming::SetAngles(angles) => seen.push(Seen::Angles(angles)),
                        Incoming::Print(print) => {
                            assert_eq!(print.kind, PrintKind::Console);
                            seen.push(Seen::Print(print.text.to_vec()));
                        }
                        _ => panic!("unexpected service"),
                    }
                }
            ),
            Ok(None)
        );
        let expected = if protocol == Protocol::NetQuake15 {
            vec![
                Seen::Value(0, -1),
                Seen::Angles(Vec3([90., -180., -1.40625])),
                Seen::Print(b"after".to_vec()),
            ]
        } else {
            vec![
                Seen::Value(0, 255),
                Seen::Value(31, i32::MIN),
                Seen::Angles(Vec3([90., -180., -1.40625])),
                Seen::Print(b"after".to_vec()),
            ]
        };
        assert_eq!(seen, expected);
        let ack = client
            .prepare(None, EventTime(5))
            .unwrap()
            .unwrap()
            .bytes
            .to_vec();
        client.submitted(EventTime(5)).unwrap();
        server.receive(&ack, EventTime(6)).unwrap();
        assert_eq!(server.reliable_receipts(), &[receipt]);
    }
}

#[test]
fn a_truncated_following_service_keeps_only_accepted_stat_prefix_effects() {
    for protocol in [Protocol::NetQuake15, Protocol::QuakeWorld28] {
        let mut channel = Channel::load(protocol.channel(), Endpoint::Client, 8192, 4).unwrap();
        channel.configure_snapshots(protocol).unwrap();
        let mut commands = Commands::load(protocol, 8192);
        let body: &[u8] = if protocol == Protocol::NetQuake15 {
            &[3, 0, 7, 0, 0, 0, 10, 64, 128]
        } else {
            &[3, 0, 7, 10, 64, 128]
        };
        let length = commands.stage(body).unwrap();
        let mut seen = Vec::new();
        assert!(
            commands
                .decode_output(
                    length,
                    1,
                    &mut channel,
                    ResourceNamespace::Client(ClientId(0)),
                    |incoming| match incoming {
                        Incoming::PlayerValue { index, value } => {
                            seen.push(Seen::Value(index, value))
                        }
                        _ => panic!("truncated service must not be delivered"),
                    },
                )
                .is_err()
        );
        assert_eq!(seen, [Seen::Value(0, 7)]);
        assert!(channel.current_snapshot().is_none());
    }
}
