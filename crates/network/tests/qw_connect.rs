use qa_core::{
    loopback::Endpoint,
    primitives::{ClientId, PrintKind, UserCmd},
    sys_events::{EventTime, Peer},
};
use qa_network::{
    channel::Channel,
    commands::{
        connection::Commands,
        packet::Protocol,
        startup::{
            QW_CONNECT_USERINFO_BYTES, QwConnectionless, read_qw_connectionless, write_qw_connect,
            write_qw_getchallenge,
        },
    },
    headers::{self, Direction, Header},
    ingress::{
        ConnectParameters, Connection, ConnectionPhase, Connections, Incoming, NativeConnect, Route,
    },
};
use std::{net::SocketAddr, ops::ControlFlow};

const GET: &[u8] = b"\xff\xff\xff\xffgetchallenge\n";
const REMOTE: &str = "127.0.0.1:27500";
const QPORT: u16 = 49123;

fn pending() -> Connections {
    let protocol = Protocol::QuakeWorld28;
    let mut channel = Channel::load(protocol.channel(), Endpoint::Client, 8192, 16).unwrap();
    channel.set_qport(QPORT);
    let mut peers = Connections::load(1);
    peers
        .bind(
            ClientId(0),
            Endpoint::Client,
            Connection {
                phase: ConnectionPhase::Connecting(NativeConnect::new()),
                route: Route {
                    socket: 7,
                    peer: Peer::Socket(REMOTE.parse().unwrap()),
                },
                channel,
                output: None,
                pending_output: None,
                commands: Some(Commands::load(protocol, 8192)),
            },
        )
        .unwrap();
    peers
}

fn send(
    peers: &mut Connections,
    milliseconds: u64,
    info: Option<&[u8]>,
    admitted: bool,
) -> (Vec<Vec<u8>>, usize) {
    let mut packets = Vec::new();
    let mut info_reads = 0;
    peers.send_connect_requests(
        EventTime::from_milliseconds(milliseconds),
        |id, protocol, route, scratch| {
            assert_eq!((id, protocol), (ClientId(0), Protocol::QuakeWorld28));
            assert_eq!(route.socket, 7);
            assert_eq!(route.peer, Peer::Socket(REMOTE.parse().unwrap()));
            assert_eq!(scratch.len(), QW_CONNECT_USERINFO_BYTES);
            info_reads += 1;
            let info = info?;
            scratch[..info.len()].copy_from_slice(info);
            Some(ConnectParameters::new(info.len()))
        },
        |route, bytes| {
            assert_eq!(route.socket, 7);
            packets.push(bytes.to_vec());
            admitted
        },
        |_, _, _| panic!("QW has no three-attempt timeout"),
    );
    (packets, info_reads)
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

#[test]
fn client_codec_keeps_native_oob_bytes_signed_challenge_and_legacy_tail() {
    assert_eq!(Protocol::QuakeWorld28.default_connect_port(), Some(27500));
    let mut output = [0xcc; 256];
    let n = write_qw_getchallenge(&mut output).unwrap();
    assert_eq!(&output[..n], GET);
    assert_eq!(output[n], 0xcc);
    let n = write_qw_connect(65535, i32::MIN, b"\\name\\native\x80", &mut output).unwrap();
    assert_eq!(
        &output[..n],
        b"\xff\xff\xff\xffconnect 28 65535 -2147483648 \"\\name\\native\x80\"\n"
    );
    assert_eq!(
        read_qw_connectionless(b"\xff\xff\xff\xffc -42suffix"),
        Ok(QwConnectionless::Challenge(-42))
    );
    assert_eq!(
        read_qw_connectionless(b"\xff\xff\xff\xffc+17\xff99"),
        Ok(QwConnectionless::Challenge(17))
    );
    assert_eq!(
        read_qw_connectionless(b"\xff\xff\xff\xffc"),
        Ok(QwConnectionless::Challenge(0))
    );
    assert_eq!(
        read_qw_connectionless(b"\xff\xff\xff\xffj"),
        Ok(QwConnectionless::Accepted)
    );
    let print = b"\xff\xff\xff\xffnpassword required\xffnot text";
    let QwConnectionless::Print(text) = read_qw_connectionless(print).unwrap() else {
        panic!("print")
    };
    assert_eq!(text, b"password required");
    assert_eq!(text.as_ptr(), print[5..].as_ptr());
    assert_eq!(
        read_qw_connectionless(b"\xff\xff\xff\xffnraw\x80\0ignored"),
        Ok(QwConnectionless::Print(b"raw\x80"))
    );
    let mut long = vec![255; 4];
    long.push(b'n');
    long.extend_from_slice(&[b'a'; 2100]);
    assert!(
        matches!(read_qw_connectionless(&long), Ok(QwConnectionless::Print(text)) if text.len()==2047)
    );
    for packet in [
        b"".as_slice(),
        b"\xff\xff\xff\xff",
        b"\0\xff\xff\xffj",
        b"\xff\xff\xff\xffrcommand",
    ] {
        assert!(read_qw_connectionless(packet).is_err());
    }
    for info in [
        b"a\"b".as_slice(),
        b"a\0b",
        b"a\xffb",
        b"a\rb",
        b"a\nb",
        &[b'a'; 196],
    ] {
        let before = output;
        assert!(write_qw_connect(0, 0, info, &mut output).is_err());
        assert_eq!(output, before);
    }
    assert!(write_qw_connect(0, 0, &[b'a'; 195], &mut output).is_ok());
    let mut short = [0xcc; 8];
    assert!(write_qw_connect(0, 0, b"", &mut short).is_err());
    assert_eq!(short, [0xcc; 8]);
}

#[test]
fn retry_uses_five_second_challenges_and_only_admitted_current_info_is_accepted() {
    let mut peers = pending();
    reply(&mut peers, b"\xff\xff\xff\xffc17", |_, _, _| {
        panic!("unsolicited challenge")
    });
    assert_eq!(send(&mut peers, 0, None, false), (vec![GET.to_vec()], 0));
    reply(&mut peers, b"\xff\xff\xff\xffc17", |_, _, _| {
        panic!("unsubmitted challenge")
    });
    assert_eq!(send(&mut peers, 1, None, true), (vec![GET.to_vec()], 0));
    reply(&mut peers, b"\xff\xff\xff\xffj", |_, _, _| {
        panic!("no admitted connect")
    });
    reply(&mut peers, b"\xff\xff\xff\xffc-17", |_, _, _| {
        panic!("challenge is state only")
    });
    // The current provider may be unavailable; the challenge stays pending.
    assert_eq!(send(&mut peers, 2, None, true), (vec![], 1));
    let first = send(&mut peers, 2, Some(b"\\name\\refused"), false);
    assert_eq!(
        first,
        (
            vec![b"\xff\xff\xff\xffconnect 28 49123 -17 \"\\name\\refused\"\n".to_vec()],
            1
        )
    );
    reply(&mut peers, b"\xff\xff\xff\xffj", |_, _, _| {
        panic!("refused connect has no receipt")
    });
    let second = send(&mut peers, 3, Some(b"\\name\\admitted"), true);
    assert_eq!(
        second,
        (
            vec![b"\xff\xff\xff\xffconnect 28 49123 -17 \"\\name\\admitted\"\n".to_vec()],
            1
        )
    );
    assert_eq!(send(&mut peers, 5002, None, true), (vec![], 0));
    // Lost connect replies cause a fresh challenge, never a stale connect replay.
    for attempt in 1..=12 {
        assert_eq!(
            send(&mut peers, 3 + attempt * 5000, None, true),
            (vec![GET.to_vec()], 0)
        );
    }
    let mut prints = 0;
    reply(
        &mut peers,
        b"\xff\xff\xff\xffnBad password.\xffignored",
        |id, endpoint, incoming| {
            assert_eq!((id, endpoint), (ClientId(0), Endpoint::Client));
            let Incoming::Print(print) = incoming else {
                panic!("console print")
            };
            assert_eq!(
                (print.kind, print.level, print.text),
                (PrintKind::Console, None, b"Bad password.".as_slice())
            );
            prints += 1;
            ControlFlow::Continue(())
        },
    );
    assert_eq!(prints, 1);
    assert!(matches!(
        peers.get(ClientId(0), Endpoint::Client).unwrap().phase,
        ConnectionPhase::Connecting(_)
    ));
    reply(&mut peers, b"\xff\xff\xff\xffc23", |_, _, _| {
        panic!("state only")
    });
    assert_eq!(
        send(&mut peers, 60004, Some(b"\\name\\new-but-refused"), false).1,
        1
    );
    let mut accepted = 0;
    reply(
        &mut peers,
        b"\xff\xff\xff\xffj",
        |id, endpoint, incoming| {
            assert_eq!((id, endpoint), (ClientId(0), Endpoint::Client));
            assert!(matches!(
                incoming,
                Incoming::ConnectAccepted {
                    port: 27500,
                    userinfo: Some(b"\\name\\admitted")
                }
            ));
            accepted += 1;
            ControlFlow::Break(())
        },
    );
    assert_eq!(accepted, 1);
    assert!(
        peers
            .get(ClientId(0), Endpoint::Client)
            .unwrap()
            .phase
            .is_connected()
    );
}

#[test]
fn full_peer_binding_and_late_oob_preserve_the_same_channel_history_and_qport() {
    let mut peers = pending();
    let (state, send_state, number) = {
        let row = peers.get_mut(ClientId(0), Endpoint::Client).unwrap();
        let number = row
            .commands
            .as_mut()
            .unwrap()
            .record(UserCmd::default())
            .unwrap();
        (row.channel.state(), row.channel.send_state(), number)
    };
    assert_eq!(send(&mut peers, 0, None, true).0, vec![GET.to_vec()]);
    for (socket, address) in [(8, REMOTE), (7, "127.0.0.2:27500"), (7, "127.0.0.1:27501")] {
        for bytes in [
            b"\xff\xff\xff\xffc100".as_slice(),
            b"\xff\xff\xff\xffj",
            b"\xff\xff\xff\xffnspoofed",
        ] {
            peers.receive(
                socket,
                address.parse::<SocketAddr>().unwrap(),
                bytes,
                EventTime(1),
                |_, _, _| panic!("wrong socket or fullpeer"),
            );
        }
    }
    assert_eq!(
        send(&mut peers, 1, Some(b"\\name\\unchallenged"), true),
        (vec![], 0)
    );
    reply(&mut peers, b"\xff\xff\xff\xffc100", |_, _, _| {
        panic!("state only")
    });
    assert_eq!(send(&mut peers, 1, Some(b"\\name\\accepted"), true).1, 1);
    reply(&mut peers, b"\xff\xff\xff\xffj", |_, _, incoming| {
        assert!(matches!(
            incoming,
            Incoming::ConnectAccepted {
                port: 27500,
                userinfo: Some(b"\\name\\accepted")
            }
        ));
        ControlFlow::Continue(())
    });
    let malformed = peers.malformed;
    for bytes in [
        b"\xff\xff\xff\xffc200".as_slice(),
        b"\xff\xff\xff\xffj",
        b"\xff\xff\xff\xffnlate",
        b"\xff\xff\xff\xff",
    ] {
        reply(&mut peers, bytes, |_, _, _| panic!("connected OOB ignored"));
    }
    assert_eq!(peers.malformed, malformed);
    let row = peers.get_mut(ClientId(0), Endpoint::Client).unwrap();
    assert_eq!(
        (row.channel.state(), row.channel.send_state()),
        (state, send_state)
    );
    assert_eq!(row.channel.qport(), QPORT);
    assert_eq!(
        row.commands
            .as_mut()
            .unwrap()
            .record(UserCmd::default())
            .unwrap(),
        number + 1
    );
    row.channel
        .queue_client_command(Protocol::QuakeWorld28, b"new")
        .unwrap();
    let packet = row
        .channel
        .prepare(None, EventTime(2))
        .unwrap()
        .unwrap()
        .bytes
        .to_vec();
    let (header, payload) =
        headers::decode(headers::QUAKEWORLD, Direction::ToServer, &packet).unwrap();
    assert_eq!(header.qport, QPORT);
    assert_eq!(payload, b"\x04new\0");
    let mut packet = [0; 64];
    let length = headers::encode(
        headers::QUAKEWORLD,
        Direction::ToClient,
        Header {
            sequence: 1,
            ..Default::default()
        },
        b"\x08\x02sequenced\0",
        &mut packet,
    )
    .unwrap();
    let mut order = Vec::new();
    reply(&mut peers, &packet[..length], |_, _, incoming| {
        match incoming {
            Incoming::PacketTiming {
                acknowledged_sent: None,
            } => order.push("timing"),
            Incoming::Print(print) => {
                assert_eq!(print.text, b"sequenced");
                order.push("print");
            }
            _ => panic!("sequenced service"),
        }
        ControlFlow::Continue(())
    });
    assert_eq!(order, ["timing", "print"]);
}

#[test]
fn invalid_provider_payload_does_not_admit_or_consume_the_pending_challenge() {
    let mut peers = pending();
    send(&mut peers, 0, None, true);
    reply(&mut peers, b"\xff\xff\xff\xffc5", |_, _, _| {
        panic!("state only")
    });
    for invalid in [b"\\name\\bad\ncommand".as_slice(), &[b'x'; 196]] {
        assert_eq!(send(&mut peers, 1, Some(invalid), true), (vec![], 1));
    }
    reply(&mut peers, b"\xff\xff\xff\xffj", |_, _, _| {
        panic!("invalid info never sent")
    });
    let valid = [b'x'; 195];
    assert_eq!(send(&mut peers, 1, Some(&valid), true).1, 1);
    reply(&mut peers, b"\xff\xff\xff\xffj", |_, _, incoming| {
        let Incoming::ConnectAccepted {
            userinfo: Some(info),
            ..
        } = incoming
        else {
            panic!("accepted receipt")
        };
        assert_eq!(info, valid);
        ControlFlow::Continue(())
    });
}

#[test]
fn disconnect_is_native_drop_string_and_decodes_through_the_same_server_channel() {
    let protocol = Protocol::QuakeWorld28;
    let mut body = [0xcc; 16];
    let length = protocol.write_disconnect(&mut body).unwrap();
    assert_eq!(&body[..length], b"\x04drop\0");
    assert_eq!(body[length], 0xcc);
    let mut nq = [0xcc; 2];
    assert_eq!(Protocol::NetQuake15.write_disconnect(&mut nq).unwrap(), 1);
    assert_eq!(nq, [2, 0xcc]);
    for protocol in [
        Protocol::Quake2_34,
        Protocol::Quake2Repro1038,
        Protocol::Quake3_68,
    ] {
        assert!(protocol.write_disconnect(&mut nq).is_err());
    }
    let mut client = Channel::load(protocol.channel(), Endpoint::Client, 8192, 4).unwrap();
    let mut server = Channel::load(protocol.channel(), Endpoint::Server, 8192, 4).unwrap();
    server.configure_snapshots(protocol).unwrap();
    client.configure_snapshots(protocol).unwrap();
    client.set_qport(QPORT);
    let packet = client
        .prepare_move(&body[..length], EventTime(1), None, None)
        .unwrap()
        .unwrap()
        .bytes
        .to_vec();
    client.submitted(EventTime(1)).unwrap();
    let received = server.receive(&packet, EventTime(2)).unwrap();
    assert_eq!(received.header.qport, QPORT);
    let sequence = received.header.sequence;
    let qa_network::channel::Delivery::Payload(payload) = received.delivery else {
        panic!("native CLIENT payload")
    };
    let mut commands = Commands::load(protocol, 8192);
    let length = commands.stage(payload).unwrap();
    let mut strings = 0;
    commands
        .decode(length, sequence, 0, &mut server, None, |incoming| {
            let Incoming::CommandString {
                protocol: actual,
                text,
                ..
            } = incoming
            else {
                panic!("native string command")
            };
            assert_eq!(actual, protocol);
            assert_eq!(text, b"drop");
            strings += 1;
            ControlFlow::Continue(())
        })
        .unwrap();
    assert_eq!(strings, 1);
}
