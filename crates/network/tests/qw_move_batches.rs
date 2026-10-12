use qa_core::{loopback::Endpoint, sys_events::EventTime};
use qa_network::{
    channel::{Channel, Delivery},
    commands::{
        QwCmd,
        connection::Commands,
        packet::{self, Key, Move, Protocol, ZERO_QW},
    },
    headers::{self, Direction, Header},
    ingress::Incoming,
};
use std::ops::ControlFlow;

#[derive(Clone, Debug, PartialEq)]
enum Seen {
    Begin,
    Command(u16, u32),
    End,
    Text(Vec<u8>),
}

fn movement(sequence: u32, msec: [u8; 3]) -> Vec<u8> {
    let mut output = [0; 1400];
    let length = packet::write(
        &mut output,
        &Move::QuakeWorld {
            loss: 0,
            commands: msec.map(|msec| QwCmd {
                msec,
                buttons: 1,
                ..ZERO_QW
            }),
            delta_request: None,
        },
        sequence,
        Key::default(),
    )
    .unwrap();
    output[..length].to_vec()
}

fn deliver(
    channel: &mut Channel,
    commands: &mut Commands,
    sequence: u32,
    payload: &[u8],
    stop: Option<usize>,
) -> (Result<usize, packet::Error>, Vec<Seen>) {
    let mut bytes = [0; 1400];
    let length = headers::encode(
        headers::QUAKEWORLD,
        Direction::ToServer,
        Header {
            sequence,
            ..Default::default()
        },
        payload,
        &mut bytes,
    )
    .unwrap();
    let received = channel.receive(&bytes[..length], EventTime(1)).unwrap();
    let Delivery::Payload(payload) = received.delivery else {
        panic!("native move")
    };
    let length = commands.stage(payload).unwrap();
    let mut seen = Vec::new();
    let result = commands.decode(length, sequence, 0, channel, None, |incoming| {
        seen.push(match incoming {
            Incoming::MoveBegin => Seen::Begin,
            Incoming::Command { command, .. } => {
                Seen::Command(command.duration_ms, command.buttons)
            }
            Incoming::MoveEnd => Seen::End,
            Incoming::CommandString { text, .. } => Seen::Text(text.to_vec()),
            _ => panic!("unexpected command"),
        });
        if stop == Some(seen.len() - 1) {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    });
    (result, seen)
}

#[test]
fn qw_101_and_203_ms_recovery_share_one_checked_batch_envelope() {
    for dropped in [0, 1, 2, 3, 19, 20] {
        let mut channel =
            Channel::load(Protocol::QuakeWorld28.channel(), Endpoint::Server, 8192, 4).unwrap();
        let mut commands = Commands::load(Protocol::QuakeWorld28, 8192);
        assert_eq!(
            deliver(
                &mut channel,
                &mut commands,
                1,
                &movement(1, [0, 0, 7]),
                None
            ),
            (Ok(1), vec![Seen::Begin, Seen::Command(7, 1), Seen::End])
        );
        let sequence = dropped + 2;
        let mut payload = movement(sequence, [101, 203, 101]);
        payload.extend_from_slice(b"\x04after\0");
        let mut expected = vec![Seen::Begin];
        if dropped < 20 {
            for _ in 2..dropped {
                expected.push(Seen::Command(7, 0));
            }
            if dropped > 1 {
                expected.push(Seen::Command(101, 1));
            }
            if dropped > 0 {
                expected.push(Seen::Command(203, 1));
            }
        }
        expected.extend([
            Seen::Command(101, 1),
            Seen::End,
            Seen::Text(b"after".to_vec()),
        ]);
        let count = expected
            .iter()
            .filter(|seen| matches!(seen, Seen::Command(..)))
            .count();
        assert_eq!(
            deliver(&mut channel, &mut commands, sequence, &payload, None),
            (Ok(count), expected)
        );
        assert_eq!(channel.state().dropped, dropped);
    }
}

#[test]
fn invalid_crc_never_begins_and_consumer_stop_never_completes_a_batch() {
    let mut bad = movement(1, [0, 0, 101]);
    bad[1] ^= 1;
    let mut payload = b"\x04before\0".to_vec();
    payload.extend(bad);
    payload.extend_from_slice(b"\x04after\0");
    let mut channel =
        Channel::load(Protocol::QuakeWorld28.channel(), Endpoint::Server, 8192, 4).unwrap();
    let mut commands = Commands::load(Protocol::QuakeWorld28, 8192);
    assert_eq!(
        deliver(&mut channel, &mut commands, 1, &payload, None),
        (
            Err(packet::Error::Checksum),
            vec![Seen::Text(b"before".to_vec())]
        )
    );
    for (stop, count, expected) in [
        (0, 0, vec![Seen::Begin]),
        (1, 1, vec![Seen::Begin, Seen::Command(203, 1)]),
        (2, 1, vec![Seen::Begin, Seen::Command(203, 1), Seen::End]),
    ] {
        let mut channel =
            Channel::load(Protocol::QuakeWorld28.channel(), Endpoint::Server, 8192, 4).unwrap();
        let mut commands = Commands::load(Protocol::QuakeWorld28, 8192);
        let mut payload = movement(1, [0, 0, 203]);
        payload.extend_from_slice(b"\x04after\0");
        assert_eq!(
            deliver(&mut channel, &mut commands, 1, &payload, Some(stop)),
            (Ok(count), expected)
        );
    }
}

#[test]
fn a_forbidden_second_move_cannot_open_a_second_batch() {
    let mut channel =
        Channel::load(Protocol::QuakeWorld28.channel(), Endpoint::Server, 8192, 4).unwrap();
    let mut commands = Commands::load(Protocol::QuakeWorld28, 8192);
    let mut payload = movement(1, [0, 0, 101]);
    payload.extend_from_slice(b"\x04between\0\x03\xff\xff\xff\x04after\0");
    assert_eq!(
        deliver(&mut channel, &mut commands, 1, &payload, None),
        (
            Ok(1),
            vec![
                Seen::Begin,
                Seen::Command(101, 1),
                Seen::End,
                Seen::Text(b"between".to_vec()),
            ]
        )
    );
}
