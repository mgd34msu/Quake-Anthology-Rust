use qa_core::{
    loopback::Endpoint,
    primitives::{ClientId, UserCmd},
    sys_events::{EventTime, Peer},
};
use qa_network::{
    channel::{self, Channel},
    commands::{
        connection::Commands,
        packet::Protocol,
        startup::{
            Q2_CONNECT_USERINFO_BYTES, Q2ConnectAccepted, Q2Connectionless, read_q2_connectionless,
            write_q2_connect, write_q2_getchallenge,
        },
    },
    headers::{self, Direction, Header, QPort},
    ingress::{
        BindError, ConnectParameters, Connection, ConnectionPhase, Connections, Incoming,
        NativeConnect, Route,
    },
};
use std::{net::SocketAddr, ops::ControlFlow};

const PROTOCOLS: [Protocol; 2] = [Protocol::Quake2_34, Protocol::Quake2Repro1038];
const GET: &[u8] = b"\xff\xff\xff\xffgetchallenge\n";
const REMOTE: &str = "127.0.0.1:27910";
const CHALLENGE: &[u8] = b"\xff\xff\xff\xffchallenge -17 p=34,1038\n";

#[test]
fn native_disconnect_payload_decodes_before_the_classic_final_eof() {
    use qa_network::commands::packet::{self, Error, Key, Record};
    for protocol in PROTOCOLS {
        let mut bytes = [0xcc; 32];
        let length = protocol.write_disconnect(&mut bytes).unwrap();
        let expected = if protocol == Protocol::Quake2_34 {
            &b"\x04disconnect"[..]
        } else {
            &b"\x04disconnect\0"[..]
        };
        assert_eq!(&bytes[..length], expected);
        let mut strings = 0;
        packet::read_with_commands(
            protocol,
            &mut bytes[..length],
            1,
            Key::default(),
            |record| {
                let Record::String { text, .. } = record else {
                    panic!("disconnect string command");
                };
                assert_eq!(text, b"disconnect");
                strings += 1;
                Ok(ControlFlow::Break(()))
            },
        )
        .unwrap();
        assert_eq!(strings, 1);
        let mut strings = 0;
        let result = packet::read_with_commands(
            protocol,
            &mut bytes[..length],
            1,
            Key::default(),
            |record| {
                if let Record::String { text, .. } = record {
                    assert_eq!(text, b"disconnect");
                    strings += 1;
                }
                Ok(ControlFlow::Continue(()))
            },
        );
        assert_eq!(strings, 1);
        if protocol == Protocol::Quake2_34 {
            assert!(matches!(result, Err(Error::Message(error))
                if error.kind == qa_network::message::ErrorKind::Truncated));
        } else {
            result.unwrap();
        }
    }
}

#[test]
fn disconnect_payload_does_not_wait_for_an_older_reliable_flight() {
    use qa_network::channel::Delivery;
    for protocol in PROTOCOLS {
        let mut client = Channel::load(
            protocol.client_channel(19),
            Endpoint::Client,
            protocol.message_bytes(),
            4,
        )
        .unwrap();
        let mut server = Channel::load(
            protocol.client_channel(19),
            Endpoint::Server,
            protocol.message_bytes(),
            4,
        )
        .unwrap();
        client.configure_snapshots(protocol).unwrap();
        server.configure_snapshots(protocol).unwrap();
        client.set_qport(19);
        client.queue_reliable(b"\x04older\0").unwrap();
        let packet = client
            .prepare(None, EventTime(1))
            .unwrap()
            .unwrap()
            .bytes
            .to_vec();
        client.submitted(EventTime(1)).unwrap();
        let received = server.receive(&packet, EventTime(1)).unwrap();
        assert!(matches!(
            received.delivery,
            Delivery::Payload(b"\x04older\0")
        ));
        client.queue_reliable(b"\x04queued\0").unwrap();
        let mut body = [0; 32];
        let length = protocol.write_disconnect(&mut body).unwrap();
        let packet = client
            .prepare_move(&body[..length], EventTime(2), None, None)
            .unwrap()
            .unwrap()
            .bytes
            .to_vec();
        client.submitted(EventTime(2)).unwrap();
        assert!(
            client.send_state().reliable_bytes > 0,
            "older flight remains unacknowledged"
        );
        let received = server.receive(&packet, EventTime(2)).unwrap();
        let sequence = received.header.sequence;
        let Delivery::Payload(payload) = received.delivery else {
            panic!("disconnect datagram payload");
        };
        assert_eq!(payload, &body[..length]);
        let mut commands = Commands::load(protocol, protocol.message_bytes());
        let length = commands.stage(payload).unwrap();
        let mut strings = 0;
        commands
            .decode(length, sequence, 0, &mut server, None, |incoming| {
                if let Incoming::CommandString {
                    protocol: actual,
                    text,
                    ..
                } = incoming
                {
                    assert_eq!(actual, protocol);
                    assert_eq!(text, b"disconnect");
                    strings += 1;
                    return ControlFlow::Break(());
                }
                ControlFlow::Continue(())
            })
            .unwrap();
        assert_eq!(strings, 1);
    }
}

fn row(protocol: Protocol, qport: u16) -> Connection {
    let mut channel = Channel::load(
        protocol.client_channel(qport),
        Endpoint::Client,
        protocol.message_bytes(),
        16,
    )
    .unwrap();
    channel.set_qport(qport);
    Connection {
        phase: ConnectionPhase::Connecting(NativeConnect::new()),
        route: Route {
            socket: 7,
            peer: Peer::Socket(REMOTE.parse().unwrap()),
        },
        channel,
        output: None,
        pending_output: None,
        commands: Some(Commands::load(protocol, protocol.message_bytes())),
    }
}

fn pending(protocol: Protocol, qport: u16) -> Connections {
    let mut peers = Connections::load(1);
    peers
        .bind(ClientId(0), Endpoint::Client, row(protocol, qport))
        .unwrap();
    peers
}

fn send(
    peers: &mut Connections,
    time: u64,
    info: Option<&[u8]>,
    packet_length: i32,
    admit: bool,
) -> (Vec<Vec<u8>>, usize) {
    let mut packets = Vec::new();
    let mut reads = 0;
    peers.send_connect_requests(
        EventTime::from_milliseconds(time),
        |id, protocol, route, output| {
            assert_eq!(id, ClientId(0));
            assert!(PROTOCOLS.contains(&protocol));
            assert_eq!(route.socket, 7);
            assert_eq!(route.peer, Peer::Socket(REMOTE.parse().unwrap()));
            assert_eq!(output.len(), Q2_CONNECT_USERINFO_BYTES);
            reads += 1;
            let info = info?;
            output[..info.len()].copy_from_slice(info);
            Some(ConnectParameters {
                userinfo_length: info.len(),
                packet_length,
            })
        },
        |route, bytes| {
            assert_eq!(route.socket, 7);
            packets.push(bytes.to_vec());
            admit
        },
        |_, _, _| panic!("Q2 has no three-attempt timeout"),
    );
    (packets, reads)
}

fn reply(
    peers: &mut Connections,
    bytes: &[u8],
    consume: impl FnMut(ClientId, Endpoint, Incoming<'_>) -> ControlFlow<()>,
) {
    peers.receive(
        7,
        REMOTE.parse::<SocketAddr>().unwrap(),
        bytes,
        EventTime(1),
        consume,
    );
}

fn begin(peers: &mut Connections) {
    assert_eq!(send(peers, 0, None, 1390, true), (vec![GET.to_vec()], 0));
    reply(peers, CHALLENGE, |_, _, _| {
        panic!("challenge is state only")
    });
}

#[test]
fn codecs_preserve_native_connect_grammar_widths_and_packet_options() {
    let mut wire = [0xcc; 1024];
    let length = write_q2_getchallenge(&mut wire).unwrap();
    assert_eq!(&wire[..length], GET);
    assert_eq!(wire[length], 0xcc);
    for (protocol, expected) in [
        (
            Protocol::Quake2_34,
            b"\xff\xff\xff\xffconnect 34 65535 -2147483648 \"\\name\\native\"\n".as_slice(),
        ),
        (
            Protocol::Quake2Repro1038,
            b"\xff\xff\xff\xffconnect 1038 255 -2147483648 \"\\name\\native\" 1390 1\n",
        ),
    ] {
        assert_eq!(protocol.default_connect_port(), Some(27910));
        assert_eq!(protocol.userinfo_bytes(), Some(512));
        let length = write_q2_connect(
            protocol,
            65535,
            i32::MIN,
            b"\\name\\native",
            1390,
            &mut wire,
        )
        .unwrap();
        assert_eq!(&wire[..length], expected);
        for invalid in [b"a\0b".as_slice(), b"a\"b", b"a\rb", b"a\nb", &[b'a'; 512]] {
            let old = wire;
            assert!(write_q2_connect(protocol, 0, 0, invalid, 1390, &mut wire).is_err());
            assert_eq!(wire, old);
        }
        let mut short = [0xcc; 8];
        assert!(write_q2_connect(protocol, 0, 0, b"", 1390, &mut short).is_err());
        assert_eq!(short, [0xcc; 8]);
    }
    for (raw, expected) in [
        (0, 0),
        (-1, 512),
        (1, 512),
        (511, 512),
        (512, 512),
        (4086, 4086),
        (i32::MAX, 4086),
    ] {
        let length =
            write_q2_connect(Protocol::Quake2Repro1038, 256, 3, b"", raw, &mut wire).unwrap();
        let mut wanted = vec![255; 4];
        wanted.extend_from_slice(format!("connect 1038 0 3 \"\" {expected} 1\n").as_bytes());
        assert_eq!(&wire[..length], wanted);
    }
    let maximum = [b'a'; 511];
    assert!(write_q2_connect(Protocol::Quake2Repro1038, 0, 0, &maximum, 1390, &mut wire).is_ok());
    assert!(write_q2_connect(Protocol::Quake2_34, 0, 0, &[255], 1390, &mut wire).is_err());
    assert!(write_q2_connect(Protocol::Quake2Repro1038, 0, 0, &[255], 1390, &mut wire).is_ok());
}

#[test]
fn borrowed_replies_keep_native_offers_print_body_and_acceptance_options() {
    assert_eq!(
        read_q2_connectionless(Protocol::Quake2_34, b"\xff\xff\xff\xffchallenge 42"),
        Ok(Q2Connectionless::Challenge(42))
    );
    assert!(
        read_q2_connectionless(Protocol::Quake2Repro1038, b"\xff\xff\xff\xffchallenge 42").is_err()
    );
    for protocol in PROTOCOLS {
        assert_eq!(
            read_q2_connectionless(protocol, CHALLENGE),
            Ok(Q2Connectionless::Challenge(-17))
        );
        assert_eq!(
            read_q2_connectionless(
                protocol,
                b"\xff\xff\xff\xffchallenge +42tail p=35,unknown p=1038,34"
            ),
            Ok(Q2Connectionless::Challenge(42))
        );
        assert!(
            read_q2_connectionless(protocol, b"\xff\xff\xff\xffchallenge 42 p=35,36,2023").is_err()
        );
        let print = b"\xff\xff\xff\xffprint\nBad password.\n\x80\0ignored";
        let Q2Connectionless::Print(text) = read_q2_connectionless(protocol, print).unwrap() else {
            panic!("print")
        };
        assert_eq!(text, b"Bad password.\n\x80");
        assert_eq!(text.as_ptr(), print[10..].as_ptr());
        for malformed in [
            b"".as_slice(),
            b"\xff\xff\xff",
            b"\0\xff\xff\xffclient_connect",
            b"\xff\xff\xff\xffcmd execute",
            b"\xff\xff\xff\xffclient_connect nc=2",
        ] {
            assert!(read_q2_connectionless(protocol, malformed).is_err());
        }
    }
    assert_eq!(
        read_q2_connectionless(Protocol::Quake2_34, b"\xff\xff\xff\xffclient_connect"),
        Ok(Q2Connectionless::Accepted(Q2ConnectAccepted::default()))
    );
    let accepted = b"\xff\xff\xff\xffclient_connect nc=1 ac=2 map=base1 dlserver=http://assets/ dlserver=ignored";
    assert_eq!(
        read_q2_connectionless(Protocol::Quake2Repro1038, accepted),
        Ok(Q2Connectionless::Accepted(Q2ConnectAccepted {
            map: Some(b"base1"),
            download_server: Some(b"http://assets/"),
            anticheat: 2,
        }))
    );
    assert!(
        read_q2_connectionless(
            Protocol::Quake2Repro1038,
            b"\xff\xff\xff\xffclient_connect nc=0"
        )
        .is_err()
    );
    let mut oversized = b"\xff\xff\xff\xffclient_connect ".to_vec();
    oversized.extend_from_slice(&[b'x'; 1024]);
    assert!(read_q2_connectionless(Protocol::Quake2Repro1038, &oversized).is_err());
    let mut long_print = b"\xff\xff\xff\xffprint\n".to_vec();
    long_print.extend_from_slice(&[b'x'; 2100]);
    assert!(
        matches!(read_q2_connectionless(Protocol::Quake2_34, &long_print), Ok(Q2Connectionless::Print(text)) if text.len()==2047)
    );
}

#[test]
fn wire_userinfo_rules_keep_native_field_limits_filters_and_total_capacity() {
    use qa_core::text::{InfoBytes, InfoError, info_set};
    assert!(Protocol::NetQuake15.userinfo_rules().is_none());
    assert_eq!(Protocol::NetQuake15.userinfo_bytes(), None);
    for (protocol, bytes, fields, filter) in [
        (Protocol::QuakeWorld28, 196, 64, InfoBytes::Exact),
        (Protocol::Quake2_34, 512, 64, InfoBytes::Ascii7Printable),
        (
            Protocol::Quake2Repro1038,
            512,
            64,
            InfoBytes::Ascii7Printable,
        ),
        (Protocol::Quake3_68, 1024, 1024, InfoBytes::Exact),
    ] {
        let rules = protocol.userinfo_rules().unwrap();
        assert_eq!(
            (
                rules.bytes,
                rules.key_bytes,
                rules.value_bytes,
                rules.filter
            ),
            (bytes, fields, fields, filter)
        );
        assert_eq!(protocol.userinfo_bytes(), Some(rules.bytes));
    }
    for protocol in PROTOCOLS {
        let rules = protocol.userinfo_rules().unwrap();
        let mut output = [0; 512];
        let mut length = 0;
        info_set(&mut output, &mut length, b"name", &[b'x'; 63], rules).unwrap();
        let previous = output;
        let old_length = length;
        assert_eq!(
            info_set(&mut output, &mut length, b"name", &[b'x'; 64], rules),
            Err(InfoError::Invalid)
        );
        assert_eq!((output, length), (previous, old_length));
        length = 0;
        info_set(&mut output, &mut length, &[b'k'; 63], b"x", rules).unwrap();
        assert_eq!(
            info_set(&mut output, &mut length, &[b'k'; 64], b"x", rules),
            Err(InfoError::Invalid)
        );

        length = 0;
        info_set(
            &mut output,
            &mut length,
            b"name",
            b"\xe1\x80\x1f\xffZ",
            rules,
        )
        .unwrap();
        assert_eq!(&output[..length], b"\\name\\aZ");
        length = 0;
        for key in [b"k0", b"k1", b"k2", b"k3", b"k4", b"k5", b"k6"] {
            info_set(&mut output, &mut length, key, &[b'x'; 63], rules).unwrap();
        }
        info_set(&mut output, &mut length, b"z", &[b'x'; 39], rules).unwrap();
        assert_eq!(length, 511);
        assert_eq!(output[length], 0);
        assert_eq!(
            info_set(&mut output, &mut length, b"q", b"x", rules),
            Err(InfoError::Capacity)
        );
        assert_eq!(length, 511);
    }
    for protocol in [Protocol::QuakeWorld28, Protocol::Quake3_68] {
        let mut output = [0; 1024];
        let mut length = 0;
        info_set(
            &mut output,
            &mut length,
            b"name",
            b"\xe1",
            protocol.userinfo_rules().unwrap(),
        )
        .unwrap();
        assert_eq!(&output[..length], b"\\name\\\xe1");
    }
    let mut output = [0; 1024];
    let mut length = 0;
    info_set(
        &mut output,
        &mut length,
        b"name",
        &[b'x'; 64],
        Protocol::Quake3_68.userinfo_rules().unwrap(),
    )
    .unwrap();
    assert_eq!(length, 70);
}

#[test]
fn acceptance_uses_the_last_valid_netchan_assignment_and_ignores_empty_tokens() {
    for (protocol, options) in [
        (Protocol::Quake2Repro1038, "nc=0 nc=1"),
        (Protocol::Quake2Repro1038, "nc=garbage nc=+1tail"),
        (Protocol::Quake2Repro1038, "nc=-0 nc=+1 nc="),
        (Protocol::Quake2Repro1038, "nc="),
        (Protocol::Quake2_34, "nc=1 nc=0"),
        (Protocol::Quake2_34, "nc=garbage"),
        (Protocol::Quake2_34, "nc="),
    ] {
        let mut wire = vec![255; 4];
        wire.extend_from_slice(format!("client_connect {options}").as_bytes());
        assert_eq!(
            read_q2_connectionless(protocol, &wire),
            Ok(Q2Connectionless::Accepted(Q2ConnectAccepted::default())),
            "{protocol:?}: {options}"
        );
    }
    for (protocol, options) in [
        (Protocol::Quake2Repro1038, "nc=1 nc=0"),
        (Protocol::Quake2Repro1038, "nc=0 nc="),
        (Protocol::Quake2_34, "nc=0 nc=1"),
        (Protocol::Quake2Repro1038, "nc=2 nc=1"),
        (Protocol::Quake2Repro1038, "nc=-1 nc=1"),
        (Protocol::Quake2_34, "nc=2 nc=0"),
    ] {
        let mut wire = vec![255; 4];
        wire.extend_from_slice(format!("client_connect {options}").as_bytes());
        assert!(
            read_q2_connectionless(protocol, &wire).is_err(),
            "{protocol:?}: {options}"
        );
    }
}

#[test]
fn classic_rechallenges_but_rr_retries_current_connect_every_three_seconds() {
    for protocol in PROTOCOLS {
        let mut peers = pending(protocol, 17);
        begin(&mut peers);
        assert_eq!(send(&mut peers, 1, None, 1390, true), (vec![], 1));
        assert_eq!(send(&mut peers, 2, Some(b"\\name\\first"), 1390, true).1, 1);
        assert_eq!(send(&mut peers, 3001, None, 1390, true), (vec![], 0));
        for attempt in 1..=12 {
            let (packets, reads) = send(
                &mut peers,
                2 + 3000 * attempt,
                Some(b"\\name\\current"),
                700,
                true,
            );
            if protocol == Protocol::Quake2_34 {
                assert_eq!((packets, reads), (vec![GET.to_vec()], 0));
            } else {
                assert_eq!(reads, 1);
                assert_eq!(
                    packets,
                    [b"\xff\xff\xff\xffconnect 1038 17 -17 \"\\name\\current\" 700 1\n".to_vec()]
                );
            }
        }
        if protocol == Protocol::Quake2_34 {
            reply(&mut peers, CHALLENGE, |_, _, _| panic!("state only"));
            assert_eq!(
                send(&mut peers, 36003, Some(b"\\name\\current"), 0, true).1,
                1
            );
        } else {
            // A duplicate challenge cannot replace the in-use native token.
            reply(
                &mut peers,
                b"\xff\xff\xff\xffchallenge 999 p=1038",
                |_, _, _| panic!("state only"),
            );
            assert_eq!(send(&mut peers, 36003, None, 1390, true), (vec![], 0));
        }
        reply(
            &mut peers,
            b"\xff\xff\xff\xffclient_connect",
            |_, _, incoming| {
                assert!(matches!(
                    incoming,
                    Incoming::ConnectAccepted {
                        port: 27910,
                        userinfo: Some(b"\\name\\current"),
                        ..
                    }
                ));
                ControlFlow::Continue(())
            },
        );
        assert!(
            peers
                .get(ClientId(0), Endpoint::Client)
                .unwrap()
                .phase
                .is_connected()
        );
    }
}

#[test]
fn transport_rejection_cannot_admit_connect_or_replace_accepted_userinfo() {
    for protocol in PROTOCOLS {
        let mut peers = pending(protocol, 17);
        reply(&mut peers, CHALLENGE, |_, _, _| panic!("unsolicited reply"));
        assert_eq!(
            send(&mut peers, 0, None, 1390, false),
            (vec![GET.to_vec()], 0)
        );
        reply(&mut peers, CHALLENGE, |_, _, _| panic!("unsubmitted reply"));
        assert_eq!(
            send(&mut peers, 1, None, 1390, true),
            (vec![GET.to_vec()], 0)
        );
        reply(&mut peers, b"\xff\xff\xff\xffclient_connect", |_, _, _| {
            panic!("no connect submitted")
        });
        reply(&mut peers, CHALLENGE, |_, _, _| panic!("state only"));
        assert_eq!(
            send(&mut peers, 2, Some(&[b'x'; 512]), 1390, true),
            (vec![], 1)
        );
        reply(&mut peers, b"\xff\xff\xff\xffclient_connect", |_, _, _| {
            panic!("out-of-range userinfo has no admitted request")
        });
        assert_eq!(
            send(&mut peers, 2, Some(b"\\name\\refused"), 1390, false).1,
            1
        );
        reply(&mut peers, b"\xff\xff\xff\xffclient_connect", |_, _, _| {
            panic!("refused connect")
        });
        assert_eq!(
            send(&mut peers, 3, Some(b"\\name\\admitted"), 1390, true).1,
            1
        );
        if protocol == Protocol::Quake2_34 {
            assert_eq!(send(&mut peers, 3003, None, 1390, true).0, [GET.to_vec()]);
            reply(&mut peers, CHALLENGE, |_, _, _| panic!("state only"));
        }
        assert_eq!(
            send(&mut peers, 3004, Some(b"\\name\\new-refused"), 1390, false).1,
            1
        );
        reply(
            &mut peers,
            b"\xff\xff\xff\xffclient_connect",
            |_, _, incoming| {
                assert!(matches!(
                    incoming,
                    Incoming::ConnectAccepted {
                        userinfo: Some(b"\\name\\admitted"),
                        ..
                    }
                ));
                ControlFlow::Break(())
            },
        );
    }
}

#[test]
fn native_refusal_prints_remain_nonterminal_and_rr_restarts_its_challenge_cycle() {
    for protocol in PROTOCOLS {
        let mut peers = pending(protocol, 0);
        begin(&mut peers);
        assert_eq!(
            send(&mut peers, 1, Some(b"\\name\\native"), 1390, true).1,
            1
        );
        let mut printed = 0;
        reply(
            &mut peers,
            b"\xff\xff\xff\xffprint\nBad challenge.\n",
            |_, _, incoming| {
                let Incoming::ConnectionMessage(text) = incoming else {
                    panic!("connection notice")
                };
                assert_eq!(text, b"Bad challenge.\n");
                printed += 1;
                ControlFlow::Continue(())
            },
        );
        assert_eq!(printed, 1);
        assert!(matches!(
            peers.get(ClientId(0), Endpoint::Client).unwrap().phase,
            ConnectionPhase::Connecting(_)
        ));
        assert_eq!(send(&mut peers, 3000, None, 1390, true), (vec![], 0));
        assert_eq!(
            send(&mut peers, 3001, None, 1390, true),
            (vec![GET.to_vec()], 0)
        );
        if protocol == Protocol::Quake2Repro1038 {
            reply(&mut peers, b"\xff\xff\xff\xffclient_connect", |_, _, _| {
                panic!("print retired the old admitted connect")
            });
        }
        reply(&mut peers, CHALLENGE, |_, _, _| panic!("state only"));
        assert_eq!(
            send(&mut peers, 3002, Some(b"\\name\\retry"), 1390, true).1,
            1
        );
        reply(
            &mut peers,
            b"\xff\xff\xff\xffclient_connect",
            |_, _, incoming| {
                assert!(matches!(
                    incoming,
                    Incoming::ConnectAccepted {
                        userinfo: Some(b"\\name\\retry"),
                        ..
                    }
                ));
                ControlFlow::Continue(())
            },
        );
    }
}

#[test]
fn exact_peer_and_socket_gate_oob_then_the_existing_channel_owns_new_and_packets() {
    for (protocol, qport, format, width) in [
        (Protocol::Quake2_34, 65535, headers::q2_old(QPort::Short), 2),
        (Protocol::Quake2Repro1038, 255, headers::q2_new(true), 1),
        (Protocol::Quake2Repro1038, 0, headers::q2_new(false), 0),
    ] {
        let mut peers = pending(protocol, qport);
        let (state, sent, number) = {
            let row = peers.get_mut(ClientId(0), Endpoint::Client).unwrap();
            let number = row
                .commands
                .as_mut()
                .unwrap()
                .record(UserCmd::default())
                .unwrap();
            (row.channel.state(), row.channel.send_state(), number)
        };
        assert_eq!(send(&mut peers, 0, None, 1390, true).0, [GET.to_vec()]);
        for (socket, address) in [(8, REMOTE), (7, "127.0.0.2:27910"), (7, "127.0.0.1:27911")] {
            for bytes in [
                CHALLENGE,
                b"\xff\xff\xff\xffclient_connect",
                b"\xff\xff\xff\xffprint\nspoofed",
            ] {
                peers.receive(
                    socket,
                    address.parse::<SocketAddr>().unwrap(),
                    bytes,
                    EventTime(1),
                    |_, _, _| panic!("wrong socket or full peer"),
                );
            }
        }
        assert_eq!(
            send(&mut peers, 1, Some(b"\\name\\unchallenged"), 1390, true),
            (vec![], 0)
        );
        reply(&mut peers, CHALLENGE, |_, _, _| panic!("state only"));
        let maximum = [b'x'; 511];
        assert_eq!(send(&mut peers, 2, Some(&maximum), 1390, true).1, 1);
        reply(
            &mut peers,
            b"\xff\xff\xff\xffclient_connect",
            |_, _, incoming| {
                assert!(
                    matches!(incoming, Incoming::ConnectAccepted { userinfo: Some(info), .. } if info==maximum)
                );
                ControlFlow::Continue(())
            },
        );
        let malformed = peers.malformed;
        for bytes in [
            CHALLENGE,
            b"\xff\xff\xff\xffclient_connect",
            b"\xff\xff\xff\xffprint\nlate",
        ] {
            reply(&mut peers, bytes, |_, _, _| panic!("late OOB"));
        }
        assert_eq!(peers.malformed, malformed);
        let row = peers.get_mut(ClientId(0), Endpoint::Client).unwrap();
        assert_eq!(
            (row.channel.state(), row.channel.send_state()),
            (state, sent)
        );
        assert_eq!(row.channel.qport(), qport);
        assert_eq!(
            row.commands
                .as_mut()
                .unwrap()
                .record(UserCmd::default())
                .unwrap(),
            number + 1
        );
        row.channel.queue_client_command(protocol, b"new").unwrap();
        let packet = row
            .channel
            .prepare(None, EventTime(2))
            .unwrap()
            .unwrap()
            .bytes
            .to_vec();
        let (header, payload) = headers::decode(format, Direction::ToServer, &packet).unwrap();
        assert_eq!(header.qport, qport);
        assert_eq!(payload, b"\x04new\0");
        assert_eq!(packet.len(), 8 + width + payload.len());
        let mut packet = [0; 64];
        let length = headers::encode(
            format,
            Direction::ToClient,
            Header {
                sequence: 1,
                ..Default::default()
            },
            b"\x0a\x02sequenced\0",
            &mut packet,
        )
        .unwrap();
        reply(&mut peers, &packet[..length], |_, _, incoming| {
            let Incoming::Print(print) = incoming else {
                panic!("sequenced print")
            };
            assert_eq!(print.text, b"sequenced");
            ControlFlow::Continue(())
        });
    }
}

#[test]
fn pending_q2_connection_requires_its_actual_protocol_channel() {
    let mut peers = Connections::load(1);
    let mut bad = row(Protocol::Quake2Repro1038, 1);
    bad.channel = Channel::load(channel::q2_new(false), Endpoint::Client, 32768, 16).unwrap();
    bad.channel.set_qport(1);
    assert_eq!(
        peers.bind(ClientId(0), Endpoint::Client, bad),
        Err(BindError::Route)
    );
    let mut bad = row(Protocol::Quake2_34, 0);
    bad.channel = Channel::load(channel::q2_new(false), Endpoint::Client, 32768, 16).unwrap();
    assert_eq!(
        peers.bind(ClientId(0), Endpoint::Client, bad),
        Err(BindError::Route)
    );
    peers
        .bind(
            ClientId(0),
            Endpoint::Client,
            row(Protocol::Quake2Repro1038, 0),
        )
        .unwrap();
    begin(&mut peers);
    assert_eq!(send(&mut peers, 1, Some(b"old"), 1390, true).1, 1);
    peers.unbind(ClientId(0));
    peers
        .bind(
            ClientId(0),
            Endpoint::Client,
            row(Protocol::Quake2Repro1038, 0),
        )
        .unwrap();
    reply(&mut peers, b"\xff\xff\xff\xffclient_connect", |_, _, _| {
        panic!("reused slot has no admitted request")
    });
    assert!(
        !peers
            .get(ClientId(0), Endpoint::Client)
            .unwrap()
            .phase
            .is_connected()
    );
}
