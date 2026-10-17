use qa_core::{
    loopback::Endpoint,
    primitives::{ClientId, PrintKind},
    sys_events::{EventTime, Peer},
};
use qa_network::{
    channel::Channel,
    commands::{connection::Commands, packet::Protocol},
    downloads::{self, Chunk, Error},
    ingress::{Connection, ConnectionPhase, Connections, Incoming, Route},
};
use std::ops::ControlFlow;

const PROTOCOLS: [Protocol; 5] = [
    Protocol::NetQuake15,
    Protocol::QuakeWorld28,
    Protocol::Quake2_34,
    Protocol::Quake2Repro1038,
    Protocol::Quake3_68,
];

fn setup(protocol: Protocol) -> (Connections, [Channel; 2]) {
    let mut connections = Connections::load(2);
    let servers = std::array::from_fn(|slot| {
        let mut server = Channel::load(protocol.channel(), Endpoint::Server, 8192, 8).unwrap();
        server.configure_snapshots(protocol).unwrap();
        connections
            .bind(
                ClientId(slot as u32),
                Endpoint::Client,
                Connection {
                    phase: ConnectionPhase::Connected,
                    route: Route {
                        socket: Endpoint::Client.socket(),
                        peer: Peer::Loopback(ClientId(slot as u32)),
                    },
                    channel: Channel::load(protocol.channel(), Endpoint::Client, 8192, 8).unwrap(),
                    output: None,
                    pending_output: None,
                    commands: Some(Commands::load(protocol, 8192)),
                },
            )
            .unwrap();
        server
    });
    (connections, servers)
}

fn packet(server: &mut Channel, protocol: Protocol, body: &[u8]) -> Vec<u8> {
    if protocol == Protocol::QuakeWorld28 {
        let sequence = server.send_state().sequence;
        super::support::admit_qw_reply(server, sequence);
    }
    let bytes = server
        .prepare_server_message(body, EventTime(1))
        .unwrap()
        .unwrap()
        .bytes
        .to_vec();
    server.submitted(EventTime(1)).unwrap();
    bytes
}

fn services(server: &mut Channel, protocol: Protocol, suffix: &[u8]) -> Vec<u8> {
    let mut body = [0; 1024];
    let length = if protocol == Protocol::Quake3_68 {
        server.queue_reliable(b"prefix").unwrap();
        server
            .encode_server_output(&mut body, |writer, _| {
                for &byte in suffix {
                    writer.write_bits(u32::from(byte), 8)?;
                }
                Ok(())
            })
            .unwrap()
    } else {
        let length =
            qa_network::outputs::print(protocol, PrintKind::Console, 2, b"prefix", None, &mut body)
                .unwrap();
        body[length..length + suffix.len()].copy_from_slice(suffix);
        length + suffix.len()
    };
    packet(server, protocol, &body[..length])
}

fn receive(
    connections: &mut Connections,
    slot: u32,
    bytes: &[u8],
    mut consume: impl FnMut(Incoming<'_>),
) {
    connections.receive(
        Endpoint::Client.socket(),
        Peer::Loopback(ClientId(slot)),
        bytes,
        EventTime(2),
        |client, endpoint, incoming| {
            assert_eq!((client, endpoint), (ClientId(slot), Endpoint::Client));
            consume(incoming);
            ControlFlow::Continue(())
        },
    );
}

#[test]
fn fatal_native_services_keep_the_accepted_prefix_and_disconnect_only_their_client() {
    for protocol in PROTOCOLS {
        let truncated: &[u8] = match protocol {
            Protocol::NetQuake15 => &[10, 64],
            Protocol::QuakeWorld28 => &[49, 0],
            Protocol::Quake2_34 | Protocol::Quake2Repro1038 => &[13, 0],
            Protocol::Quake3_68 => &[7],
        };
        for suffix in [b"\0".as_slice(), truncated] {
            let (mut connections, mut servers) = setup(protocol);
            let bytes = services(&mut servers[0], protocol, suffix);
            let mut order = Vec::new();
            receive(&mut connections, 0, &bytes, |incoming| match incoming {
                Incoming::Print(print) => {
                    assert_eq!(print.text, b"prefix");
                    order.push("prefix");
                }
                Incoming::ReliableCommand { text, .. } => {
                    assert_eq!(text, b"prefix");
                    order.push("prefix");
                }
                Incoming::Disconnect { .. } => order.push("disconnect"),
                Incoming::PacketTiming { .. } => {}
                _ => panic!("unexpected service: {protocol:?}"),
            });
            assert_eq!(order, ["prefix", "disconnect"], "{protocol:?}");
            assert_eq!(connections.command_errors, 1);
            connections.unbind(ClientId(0));
            assert!(connections.get(ClientId(0), Endpoint::Client).is_none());

            let bytes = services(&mut servers[1], protocol, &[]);
            let mut healthy = 0;
            receive(&mut connections, 1, &bytes, |incoming| match incoming {
                Incoming::Print(_) | Incoming::ReliableCommand { .. } => healthy += 1,
                Incoming::PacketTiming { .. } => {}
                _ => panic!("healthy peer was affected: {protocol:?}"),
            });
            assert_eq!(healthy, 1, "{protocol:?}");
            assert_eq!(connections.command_errors, 1);
            assert!(connections.get(ClientId(1), Endpoint::Client).is_some());
        }
    }
}

#[test]
fn download_percentage_rejects_only_the_transfer_but_framing_and_later_services_are_fatal() {
    for protocol in &PROTOCOLS[1..4] {
        let protocol = *protocol;
        let (mut connections, mut servers) = setup(protocol);
        let mut body = [0; 32];
        let length = downloads::write_chunk(
            protocol,
            Chunk::Sequential {
                percent: 100,
                bytes: Some(b"file"),
                compressed: false,
            },
            &mut body,
        )
        .unwrap();
        body[3] = 101;
        let bytes = packet(&mut servers[0], protocol, &body[..length]);
        let mut errors = Vec::new();
        receive(&mut connections, 0, &bytes, |incoming| match incoming {
            Incoming::Download(Err(error)) => errors.push(error),
            Incoming::PacketTiming { .. } => {}
            _ => panic!("percentage rejection must keep the connection"),
        });
        assert_eq!(errors, [Error::Percent]);
        assert_eq!(connections.command_errors, 1);

        body[3] = 100;
        let bytes = packet(&mut servers[0], protocol, &body[..length]);
        let mut accepted = 0;
        receive(&mut connections, 0, &bytes, |incoming| match incoming {
            Incoming::Download(Ok(chunk)) => {
                assert!(matches!(
                    chunk,
                    qa_network::downloads::Chunk::Sequential {
                        bytes: Some(b"file"),
                        ..
                    }
                ));
                accepted += 1;
            }
            Incoming::PacketTiming { .. } => {}
            _ => panic!("fresh transfer must succeed"),
        });
        assert_eq!(accepted, 1);
        assert_eq!(connections.command_errors, 1);

        // A successful download cannot exempt a malformed following service.
        body[length] = 0;
        let bytes = packet(&mut servers[0], protocol, &body[..length + 1]);
        let mut order = Vec::new();
        receive(&mut connections, 0, &bytes, |incoming| match incoming {
            Incoming::Download(Ok(_)) => order.push("download"),
            Incoming::Disconnect { .. } => order.push("disconnect"),
            Incoming::PacketTiming { .. } => {}
            _ => panic!("unexpected download service"),
        });
        assert_eq!(order, ["download", "disconnect"]);
        assert_eq!(connections.command_errors, 2);

        let mut malformed = vec![vec![body[0], 254, 255, 20], vec![body[0], 4, 0, 101, b'x']];
        if protocol == Protocol::Quake2Repro1038 {
            malformed.push(vec![35, 1, 0, 20, 7]);
        }
        for malformed in malformed {
            let (mut connections, mut servers) = setup(protocol);
            let bytes = packet(&mut servers[0], protocol, &malformed);
            let mut order = Vec::new();
            receive(&mut connections, 0, &bytes, |incoming| match incoming {
                Incoming::Download(Err(error)) => {
                    assert_ne!(error, Error::Percent);
                    order.push("download rejection");
                }
                Incoming::Disconnect { .. } => order.push("disconnect"),
                Incoming::PacketTiming { .. } => {}
                _ => panic!("invalid framing cannot deliver file bytes"),
            });
            assert_eq!(order, ["download rejection", "disconnect"]);
            assert_eq!(connections.command_errors, 1);
        }
    }
}
