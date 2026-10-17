use qa_core::{loopback::Endpoint, sys_events::EventTime};
use qa_network::{
    channel::{Channel, Delivery, TransmitError},
    commands::{connection::Commands, packet::Protocol},
    ingress::Incoming,
};
use std::ops::ControlFlow;

const PROTOCOLS: [Protocol; 5] = [
    Protocol::NetQuake15,
    Protocol::QuakeWorld28,
    Protocol::Quake2_34,
    Protocol::Quake2Repro1038,
    Protocol::Quake3_68,
];

fn transmit(protocol: Protocol, client: &mut Channel) -> Vec<u8> {
    let mut control = [0; 1400];
    let length = Commands::load(protocol, 8192)
        .encode_control(client, &mut control)
        .unwrap();
    let body = (protocol == Protocol::Quake3_68).then_some(&control[..length]);
    let bytes = client
        .prepare(body, EventTime(1))
        .unwrap()
        .unwrap()
        .bytes
        .to_vec();
    client.submitted(EventTime(1)).unwrap();
    bytes
}

fn decode(
    protocol: Protocol,
    bytes: &[u8],
    consume: impl FnMut(Incoming<'_>) -> ControlFlow<()>,
) -> Vec<u8> {
    let mut server = Channel::load(protocol.channel(), Endpoint::Server, 8192, 8).unwrap();
    let mut commands = Commands::load(protocol, 8192);
    let received = server.receive(bytes, EventTime(2)).unwrap();
    let sequence = received.header.sequence;
    let Delivery::Payload(payload) = received.delivery else {
        panic!("complete command payload");
    };
    let payload = payload.to_vec();
    let length = commands.stage(&payload).unwrap();
    assert_eq!(
        commands
            .decode(length, sequence, 0, &mut server, None, consume)
            .unwrap(),
        0
    );
    payload
}

fn receive(protocol: Protocol, bytes: &[u8]) -> (Vec<u8>, Vec<Vec<u8>>) {
    let mut text = Vec::new();
    let payload = decode(protocol, bytes, |record| {
        if let Incoming::CommandString { text: bytes, .. } = record {
            text.push(bytes.to_vec());
        } else {
            panic!("console command must not create a usercmd");
        }
        ControlFlow::Continue(())
    });
    (payload, text)
}

#[test]
fn console_commands_use_each_protocols_existing_reliable_channel() {
    for protocol in PROTOCOLS {
        let mut client = Channel::load(protocol.channel(), Endpoint::Client, 8192, 8).unwrap();
        let text = b"native command\x80";
        client.queue_client_command(protocol, text).unwrap();
        assert!(client.reliable_receipts().is_empty());
        let bytes = transmit(protocol, &mut client);
        let (payload, records) = receive(protocol, &bytes);
        let expected = if protocol == Protocol::Quake3_68 {
            b"native command.".as_slice()
        } else {
            text
        };
        assert_eq!(records, [expected], "{protocol:?}");
        if protocol != Protocol::Quake3_68 {
            let mut expected = vec![4];
            expected.extend_from_slice(text);
            expected.push(0);
            assert_eq!(payload, expected, "{protocol:?}");
        }
        // Submission is not acknowledgement for any reliable native command.
        assert!(client.reliable_receipts().is_empty());
    }
}

#[test]
fn console_command_strings_keep_native_terminators_and_rerelease_high_bytes() {
    for protocol in PROTOCOLS {
        let mut client = Channel::load(protocol.channel(), Endpoint::Client, 8192, 8).unwrap();
        client
            .queue_client_command(protocol, b"command \xfftail\0ignored")
            .unwrap();
        let (_, records) = receive(protocol, &transmit(protocol, &mut client));
        let expected = match protocol {
            Protocol::Quake2Repro1038 => b"command \xfftail".as_slice(),
            Protocol::Quake3_68 => b"command .tail",
            _ => b"command ",
        };
        assert_eq!(records, [expected], "{protocol:?}");
    }
}

#[test]
fn console_command_admission_preserves_prepared_packets_and_restores_scratch_after_rejection() {
    let protocol = Protocol::Quake2_34;
    let mut client = Channel::load(protocol.channel(), Endpoint::Client, 8192, 8).unwrap();
    assert_eq!(
        client.queue_client_command(protocol, &[b'x'; 2046]),
        Err(TransmitError::MessageTooLarge)
    );
    client.queue_client_command(protocol, b"first").unwrap();
    let prepared = client
        .prepare(None, EventTime(1))
        .unwrap()
        .unwrap()
        .bytes
        .to_vec();
    client.queue_client_command(protocol, b"second").unwrap();
    assert_eq!(client.pending_packet().unwrap().bytes, prepared);
    client.submitted(EventTime(1)).unwrap();
    let (_, records) = receive(protocol, &prepared);
    assert_eq!(records, [b"first".as_slice()]);
}

#[test]
fn console_command_queue_rejects_server_endpoints_and_mismatched_protocols() {
    for protocol in PROTOCOLS {
        let mut server = Channel::load(protocol.channel(), Endpoint::Server, 8192, 8).unwrap();
        assert_eq!(
            server.queue_client_command(protocol, b"test"),
            Err(TransmitError::PayloadReliability)
        );
        let mut client = Channel::load(protocol.channel(), Endpoint::Client, 8192, 8).unwrap();
        let other = if protocol == Protocol::Quake3_68 {
            Protocol::NetQuake15
        } else {
            Protocol::Quake3_68
        };
        assert_eq!(
            client.queue_client_command(other, b"test"),
            Err(TransmitError::PayloadReliability)
        );
        client.queue_client_command(protocol, b"test").unwrap();
        let (_, records) = receive(protocol, &transmit(protocol, &mut client));
        assert_eq!(records, [b"test".as_slice()]);
    }
}

#[test]
fn userinfo_shares_reliable_flight_with_console_text_in_native_record_order() {
    for protocol in [
        Protocol::Quake2_34,
        Protocol::Quake2Repro1038,
        Protocol::Quake3_68,
    ] {
        let mut client = Channel::load(protocol.channel(), Endpoint::Client, 8192, 8).unwrap();
        let text = b"\\name\\native\x80\\fov\\110";
        client.queue_userinfo(protocol, text).unwrap();
        client
            .queue_client_command(protocol, b"nextserver")
            .unwrap();
        let bytes = transmit(protocol, &mut client);
        let mut records = Vec::new();
        let payload = decode(protocol, &bytes, |record| {
            match record {
                Incoming::Userinfo {
                    protocol: actual,
                    text,
                    ..
                } => {
                    assert_eq!(actual, protocol);
                    records.push((3, text.to_vec()));
                }
                Incoming::CommandString { text, .. } => records.push((4, text.to_vec())),
                _ => panic!("no movement from a userinfo change"),
            }
            ControlFlow::Continue(())
        });
        if protocol == Protocol::Quake3_68 {
            assert_eq!(
                records,
                [
                    (4, b"userinfo \"\\name\\native.\\fov\\110\"".to_vec()),
                    (4, b"nextserver".to_vec())
                ]
            );
            assert!(client.reliable_receipts().is_empty());
            continue;
        }
        assert_eq!(records, [(3, text.to_vec()), (4, b"nextserver".to_vec())]);
        let mut expected = vec![3];
        expected.extend(text);
        expected.extend(b"\0\x04nextserver\0");
        assert_eq!(payload, expected, "{protocol:?}");
        assert!(client.reliable_receipts().is_empty());
    }
}

#[test]
fn userinfo_rejects_foreign_framing_and_preserves_prepared_packets() {
    for protocol in PROTOCOLS {
        let mut server = Channel::load(protocol.channel(), Endpoint::Server, 8192, 8).unwrap();
        assert_eq!(
            server.queue_userinfo(protocol, b"\\fov\\110"),
            Err(TransmitError::PayloadReliability)
        );
        let mut client = Channel::load(protocol.channel(), Endpoint::Client, 8192, 8).unwrap();
        if !matches!(
            protocol,
            Protocol::Quake2_34 | Protocol::Quake2Repro1038 | Protocol::Quake3_68
        ) {
            assert_eq!(
                client.queue_userinfo(protocol, b"\\fov\\110"),
                Err(TransmitError::PayloadReliability)
            );
            continue;
        }
        let foreign = if protocol == Protocol::Quake3_68 {
            Protocol::Quake2_34
        } else {
            Protocol::Quake3_68
        };
        assert_eq!(
            client.queue_userinfo(foreign, b"\\fov\\110"),
            Err(TransmitError::PayloadReliability)
        );
        assert_eq!(
            client.queue_userinfo(protocol, &[b'x'; 2046]),
            Err(TransmitError::MessageTooLarge)
        );
        client.queue_client_command(protocol, b"first").unwrap();
        let mut control = [0; 1400];
        let length = Commands::load(protocol, 8192)
            .encode_control(&client, &mut control)
            .unwrap();
        let body = (protocol == Protocol::Quake3_68).then_some(&control[..length]);
        let pending = client
            .prepare(body, EventTime(1))
            .unwrap()
            .unwrap()
            .bytes
            .to_vec();
        client.queue_userinfo(protocol, b"\\fov\\110").unwrap();
        assert_eq!(client.pending_packet().unwrap().bytes, pending);
        client.submitted(EventTime(1)).unwrap();
        assert_eq!(receive(protocol, &pending).1, [b"first".to_vec()]);
    }
}
