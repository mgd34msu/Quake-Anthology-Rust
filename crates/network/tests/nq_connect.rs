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
        startup::{NqControl, read_nq_control, write_nq_control},
    },
    headers::{self, Direction, Header},
    ingress::{
        BindError, Connection, ConnectionPhase, Connections, Incoming, NativeConnect, Route,
    },
};
use std::{net::SocketAddr, ops::ControlFlow};

const REQUEST: &[u8] = b"\x80\0\0\x0c\x01QUAKE\0\x03";
const ACCEPT: &[u8] = b"\x80\0\0\x09\x81\x91\x65\0\0";

fn pending(socket: u16, address: &str) -> Connection {
    Connection {
        phase: ConnectionPhase::Connecting(NativeConnect::new()),
        route: Route {
            socket,
            peer: Peer::Socket(address.parse().unwrap()),
        },
        channel: Channel::load(channel::NETQUAKE, Endpoint::Client, 8192, 16).unwrap(),
        output: None,
        pending_output: None,
        commands: Some(Commands::load(Protocol::NetQuake15, 8192)),
    }
}

fn start(peers: &mut Connections, time: u64) -> usize {
    let mut submitted = 0;
    peers.send_connect_requests(
        EventTime::from_milliseconds(time),
        |_, _, _, _| panic!("NQ has no userinfo request"),
        |_, bytes| {
            assert_eq!(bytes, REQUEST);
            submitted += 1;
            true
        },
        |_, _, _| panic!("unexpected timeout"),
    );
    submitted
}

#[test]
fn control_codec_matches_native_bytes_and_checks_complete_fields() {
    let mut output = [0; 128];
    let length = write_nq_control(NqControl::Connect, &mut output).unwrap();
    assert_eq!(&output[..length], REQUEST);
    assert!(matches!(read_nq_control(REQUEST), Ok(NqControl::Connect)));
    let length = write_nq_control(NqControl::Accept { port: 26001 }, &mut output).unwrap();
    assert_eq!(&output[..length], ACCEPT);
    assert!(matches!(
        read_nq_control(ACCEPT),
        Ok(NqControl::Accept { port: 26001 })
    ));
    let reject = b"\x80\0\0\x0a\x82bad\xff\0";
    let length = write_nq_control(NqControl::Reject { reason: b"bad\xff" }, &mut output).unwrap();
    assert_eq!(&output[..length], reject);
    assert!(matches!(
        read_nq_control(reject),
        Ok(NqControl::Reject { reason: b"bad\xff" })
    ));
    for port in [1, 256, 65535] {
        let length = write_nq_control(NqControl::Accept { port }, &mut output).unwrap();
        assert!(matches!(
            read_nq_control(&output[..length]),
            Ok(NqControl::Accept { port: actual }) if actual == port
        ));
    }
    assert!(write_nq_control(NqControl::Accept { port: 0 }, &mut output).is_err());
    assert!(write_nq_control(NqControl::Connect, &mut [0; 11]).is_err());
    assert!(write_nq_control(NqControl::Reject { reason: b"a\0b" }, &mut output).is_err());

    for original in [REQUEST, ACCEPT, reject.as_slice()] {
        for length in 0..original.len() {
            assert!(read_nq_control(&original[..length]).is_err());
        }
        let mut extra = original.to_vec();
        extra.push(0);
        extra[3] += 1;
        assert!(read_nq_control(&extra).is_err());
    }
    for (index, byte) in [(0, 0xff), (1, 1), (3, 11), (4, 2), (5, b'q'), (11, 15)] {
        let mut bad = REQUEST.to_vec();
        bad[index] = byte;
        assert!(read_nq_control(&bad).is_err());
    }
    for port in [0i32, -1, 65536, i32::MAX, i32::MIN] {
        let mut bad = ACCEPT.to_vec();
        bad[5..].copy_from_slice(&port.to_le_bytes());
        assert!(read_nq_control(&bad).is_err());
    }
    let mut maximum = vec![0; 65536];
    let reason = vec![b'x'; 65529];
    let length = write_nq_control(NqControl::Reject { reason: &reason }, &mut maximum).unwrap();
    assert_eq!(length, 65535);
    assert!(matches!(
        read_nq_control(&maximum[..length]),
        Ok(NqControl::Reject { reason: actual }) if actual == reason.as_slice()
    ));
    assert!(read_nq_control(&maximum).is_err());
    assert!(
        write_nq_control(
            NqControl::Reject {
                reason: &vec![b'x'; 65530]
            },
            &mut maximum,
        )
        .is_err()
    );
}

#[test]
fn retries_wait_for_admission_and_three_complete_native_reply_windows() {
    let mut peers = Connections::load(1);
    peers
        .bind(ClientId(0), Endpoint::Client, pending(7, "127.0.0.1:26000"))
        .unwrap();
    for time in [0, 1, 9000] {
        let mut calls = 0;
        peers.send_connect_requests(
            EventTime::from_milliseconds(time),
            |_, _, _, _| panic!("NQ has no userinfo request"),
            |route, bytes| {
                assert_eq!(route.socket, 7);
                assert_eq!(bytes, REQUEST);
                calls += 1;
                false
            },
            |_, _, _| panic!("unsubmitted request cannot time out"),
        );
        assert_eq!(calls, 1);
    }
    assert_eq!(start(&mut peers, 9001), 1);
    for time in [0, 9000, 9001, 11500] {
        assert_eq!(start(&mut peers, time), 0);
    }
    assert_eq!(start(&mut peers, 11501), 1);
    assert_eq!(start(&mut peers, 14000), 0);
    assert_eq!(start(&mut peers, 14001), 1);
    for time in [0, 14000, 14001, 16500] {
        assert_eq!(start(&mut peers, time), 0);
    }
    let mut rejected = 0;
    for time in [16501, 20000] {
        peers.send_connect_requests(
            EventTime::from_milliseconds(time),
            |_, _, _, _| panic!("NQ has no userinfo request"),
            |_, _| panic!("at most three requests"),
            |id, endpoint, incoming| {
                assert_eq!((id, endpoint), (ClientId(0), Endpoint::Client));
                assert!(matches!(
                    incoming,
                    Incoming::ConnectRejected {
                        reason: b"No Response"
                    }
                ));
                rejected += 1;
                ControlFlow::Continue(())
            },
        );
    }
    assert_eq!(rejected, 1);
    peers.receive(
        7,
        "127.0.0.1:26000".parse::<SocketAddr>().unwrap(),
        ACCEPT,
        EventTime::from_milliseconds(20001),
        |_, _, _| panic!("terminal timeout"),
    );
    assert_eq!(
        peers.get(ClientId(0), Endpoint::Client).unwrap().phase,
        ConnectionPhase::Rejected
    );
}

#[test]
fn acceptance_requires_the_requested_peer_and_preserves_the_same_native_channel() {
    let mut peers = Connections::load(1);
    peers
        .bind(ClientId(0), Endpoint::Client, pending(7, "127.0.0.1:26000"))
        .unwrap();
    let requested: SocketAddr = "127.0.0.1:26000".parse().unwrap();
    peers.receive(7, requested, ACCEPT, EventTime(1), |_, _, _| {
        panic!("unsolicited accept")
    });
    assert!(
        !peers
            .get(ClientId(0), Endpoint::Client)
            .unwrap()
            .phase
            .is_connected()
    );
    let (before_receive, before_send, command_number) = {
        let row = peers.get_mut(ClientId(0), Endpoint::Client).unwrap();
        row.channel.queue_reliable(b"unchanged").unwrap();
        let number = row
            .commands
            .as_mut()
            .unwrap()
            .record(UserCmd::default())
            .unwrap();
        (row.channel.state(), row.channel.send_state(), number)
    };
    assert_eq!(start(&mut peers, 10), 1);
    for (socket, address) in [
        (8, requested),
        (7, "127.0.0.2:26000".parse().unwrap()),
        (7, "127.0.0.1:26001".parse().unwrap()),
    ] {
        peers.receive(
            socket,
            address,
            ACCEPT,
            EventTime::from_milliseconds(11),
            |_, _, _| panic!("spoofed accept"),
        );
    }
    let mut data = [0; 1400];
    let length = headers::encode(
        headers::NETQUAKE,
        Direction::ToClient,
        Header {
            datagram_flags: headers::datagram::UNRELIABLE,
            ..Header::default()
        },
        b"\x08ready\0",
        &mut data,
    )
    .unwrap();
    peers.receive(
        7,
        requested,
        &data[..length],
        EventTime::from_milliseconds(12),
        |_, _, _| panic!("connected data before accept"),
    );
    let mut accepted = 0;
    peers.receive(
        7,
        requested,
        ACCEPT,
        EventTime::from_milliseconds(13),
        |id, endpoint, incoming| {
            assert_eq!((id, endpoint), (ClientId(0), Endpoint::Client));
            assert!(matches!(
                incoming,
                Incoming::ConnectAccepted {
                    port: 26001,
                    userinfo: None
                }
            ));
            accepted += 1;
            ControlFlow::Continue(())
        },
    );
    assert_eq!(accepted, 1);
    let row = peers.get_mut(ClientId(0), Endpoint::Client).unwrap();
    assert_eq!(row.phase, ConnectionPhase::Connected);
    assert_eq!(
        row.route.peer,
        Peer::Socket("127.0.0.1:26001".parse().unwrap())
    );
    assert_eq!(row.channel.state(), before_receive);
    assert_eq!(row.channel.send_state(), before_send);
    assert_eq!(row.commands.as_ref().unwrap().history().unwrap().len(), 1);
    assert_eq!(
        row.commands
            .as_mut()
            .unwrap()
            .record(UserCmd::default())
            .unwrap(),
        command_number + 1
    );
    let prepared = row
        .channel
        .prepare(None, EventTime::from_milliseconds(14))
        .unwrap()
        .unwrap();
    let (_, payload) =
        headers::decode(headers::NETQUAKE, Direction::ToServer, prepared.bytes).unwrap();
    assert_eq!(payload, b"unchanged");

    peers.receive(
        7,
        requested,
        &data[..length],
        EventTime::from_milliseconds(15),
        |_, _, _| panic!("old control port is not game peer"),
    );
    let mut prints = 0;
    peers.receive(
        7,
        "127.0.0.1:26001".parse::<SocketAddr>().unwrap(),
        &data[..length],
        EventTime::from_milliseconds(16),
        |_, _, incoming| {
            assert!(matches!(incoming, Incoming::Print(print) if print.text == b"ready"));
            prints += 1;
            ControlFlow::Continue(())
        },
    );
    assert_eq!(prints, 1);
}

#[test]
fn malformed_replies_are_atomic_and_complete_rejection_is_terminal() {
    let mut peers = Connections::load(2);
    for slot in 0..2 {
        peers
            .bind(
                ClientId(slot),
                Endpoint::Client,
                pending(slot as u16 + 7, "127.0.0.1:26000"),
            )
            .unwrap();
    }
    assert_eq!(start(&mut peers, 0), 2);
    let route = peers.get(ClientId(0), Endpoint::Client).unwrap().route;
    let phase = peers
        .get(ClientId(0), Endpoint::Client)
        .unwrap()
        .phase
        .clone();
    for packet in [
        b"\x80\0\0\x08\x81\x91\x65\0".as_slice(),
        b"\x80\0\0\x09\x81\0\0\x01\0",
        b"\x80\0\0\x09\x81\xff\xff\xff\xff",
        b"\x80\0\0\x08\x82bad",
        b"\x80\0\0\x09\x82bad\xff",
        b"\x80\0\0\x0a\x81\x91\x65\0\0\0",
        b"\x80\x01\0\x09\x81\x91\x65\0\0",
        b"\xff\xff\xff\xff\x81\x91\x65\0\0",
    ] {
        peers.receive(route.socket, route.peer, packet, EventTime(1), |_, _, _| {
            panic!("malformed reply delivered")
        });
        let row = peers.get(ClientId(0), Endpoint::Client).unwrap();
        assert_eq!((row.route, &row.phase), (route, &phase));
    }
    let mut rejections = 0;
    peers.receive(
        route.socket,
        route.peer,
        b"\x80\0\0\x0a\x82bad\xff\0",
        EventTime(2),
        |id, endpoint, incoming| {
            assert_eq!((id, endpoint), (ClientId(0), Endpoint::Client));
            assert!(matches!(
                incoming,
                Incoming::ConnectRejected { reason: b"bad\xff" }
            ));
            rejections += 1;
            ControlFlow::Continue(())
        },
    );
    assert_eq!(rejections, 1);
    peers.receive(route.socket, route.peer, ACCEPT, EventTime(3), |_, _, _| {
        panic!("rejected connection revived")
    });
    assert_eq!(
        peers.get(ClientId(0), Endpoint::Client).unwrap().phase,
        ConnectionPhase::Rejected
    );
    assert_eq!(start(&mut peers, 2500), 1);
    peers.receive(
        8,
        route.peer,
        b"\x80\0\0\x09\x81\xff\xff\0\0",
        EventTime(4),
        |id, _, incoming| {
            assert_eq!(id, ClientId(1));
            assert!(matches!(
                incoming,
                Incoming::ConnectAccepted {
                    port: 65535,
                    userinfo: None
                }
            ));
            ControlFlow::Continue(())
        },
    );
    assert!(
        peers
            .get(ClientId(1), Endpoint::Client)
            .unwrap()
            .phase
            .is_connected()
    );
}

#[test]
fn returned_port_conflict_is_rejected_before_changing_the_pending_route() {
    let mut peers = Connections::load(2);
    let mut existing = pending(7, "127.0.0.1:26001");
    existing.phase = ConnectionPhase::Connected;
    peers.bind(ClientId(0), Endpoint::Client, existing).unwrap();
    peers
        .bind(ClientId(1), Endpoint::Client, pending(7, "127.0.0.1:26000"))
        .unwrap();
    assert_eq!(start(&mut peers, 0), 1);
    let route = peers.get(ClientId(1), Endpoint::Client).unwrap().route;
    let phase = peers
        .get(ClientId(1), Endpoint::Client)
        .unwrap()
        .phase
        .clone();
    peers.receive(7, route.peer, ACCEPT, EventTime(1), |_, _, _| {
        panic!("ambiguous accepted route")
    });
    assert_eq!(
        peers.get(ClientId(1), Endpoint::Client).unwrap().route,
        route
    );
    assert_eq!(
        peers.get(ClientId(1), Endpoint::Client).unwrap().phase,
        phase
    );
    assert_eq!(peers.malformed, 1);
    peers.unbind(ClientId(0));
    peers.receive(7, route.peer, ACCEPT, EventTime(2), |id, _, incoming| {
        assert_eq!(id, ClientId(1));
        assert!(matches!(
            incoming,
            Incoming::ConnectAccepted {
                port: 26001,
                userinfo: None
            }
        ));
        ControlFlow::Break(())
    });
    assert!(
        peers
            .get(ClientId(1), Endpoint::Client)
            .unwrap()
            .phase
            .is_connected()
    );
}

#[test]
fn connecting_phase_requires_native_client_socket_admission() {
    let mut peers = Connections::load(1);
    let mut row = pending(7, "127.0.0.1:26000");
    row.route = Route {
        socket: Endpoint::Client.socket(),
        peer: Peer::Loopback(ClientId(0)),
    };
    assert_eq!(
        peers.bind(ClientId(0), Endpoint::Client, row),
        Err(BindError::Route)
    );
    let mut row = pending(7, "127.0.0.1:26000");
    row.channel = Channel::load(channel::NETQUAKE, Endpoint::Server, 8192, 16).unwrap();
    assert_eq!(
        peers.bind(ClientId(0), Endpoint::Server, row),
        Err(BindError::Route)
    );
    let mut row = pending(7, "127.0.0.1:26000");
    row.commands = None;
    assert_eq!(
        peers.bind(ClientId(0), Endpoint::Client, row),
        Err(BindError::Route)
    );
    for protocol in [
        Protocol::Quake2_34,
        Protocol::Quake2Repro1038,
        Protocol::Quake3_68,
    ] {
        let mut row = pending(7, "127.0.0.1:26000");
        row.channel = Channel::load(protocol.channel(), Endpoint::Client, 8192, 16).unwrap();
        row.commands = Some(Commands::load(protocol, 8192));
        assert_eq!(
            peers.bind(ClientId(0), Endpoint::Client, row),
            Err(BindError::Route)
        );
    }
    let mut row = pending(7, "127.0.0.1:26000");
    row.channel = Channel::load(channel::QUAKEWORLD, Endpoint::Client, 8192, 16).unwrap();
    assert_eq!(
        peers.bind(ClientId(0), Endpoint::Client, row),
        Err(BindError::Route)
    );
}
