use qa_core::{
    loopback::Endpoint,
    primitives::ClientId,
    sys_events::{EventTime, Peer},
};
use qa_network::{
    channel::{self, Channel},
    headers::{self, Direction, Header},
    ingress::{BindError, Connection, Connections, Incoming, Route},
};

#[test]
fn q2_native_ack_survives_a_rejected_fragment_body() -> Result<(), String> {
    let mut peers = Connections::load(1);
    let client = ClientId(0);
    let mut channel =
        Channel::load(channel::q2_new(true), Endpoint::Server, 32, 4).map_err(|e| e.to_string())?;
    let receipt = channel
        .queue_reliable(b"native")
        .map_err(|e| e.to_string())?;
    assert!(
        channel
            .prepare(None, EventTime(1))
            .map_err(|e| e.to_string())?
            .is_some()
    );
    channel.submitted(EventTime(1)).map_err(|e| e.to_string())?;
    peers
        .bind(
            client,
            Endpoint::Server,
            Connection {
                route: Route {
                    socket: Endpoint::Server.socket(),
                    peer: Peer::Loopback(client),
                },
                channel,
                output: None,
                commands: None,
            },
        )
        .map_err(|e| format!("bind {e:?}"))?;
    let mut bytes = [0; 1400];
    let n = headers::encode(
        headers::q2_new(true),
        Direction::ToServer,
        Header {
            sequence: 1,
            acknowledgement: 1,
            reliable_ack: true,
            fragment: Some(headers::Fragment {
                offset: 0,
                more: true,
            }),
            ..Header::default()
        },
        &[137; 64],
        &mut bytes,
    )
    .map_err(|e| e.to_string())?;
    let mut acknowledgements = 0;
    peers.receive(
        Endpoint::Server.socket(),
        Peer::Loopback(client),
        &bytes[..n],
        EventTime(2),
        |id, endpoint, incoming| {
            assert_eq!((id, endpoint), (client, Endpoint::Server));
            if let Incoming::Acknowledged {
                receipt: actual, ..
            } = incoming
            {
                assert_eq!(actual, receipt);
                acknowledgements += 1;
            }
        },
    );
    assert_eq!(
        (peers.malformed, peers.delivered, acknowledgements),
        (1, 0, 1)
    );
    Ok(())
}

fn connection(socket: u16, peer: Peer, endpoint: Endpoint) -> Result<Connection, String> {
    Ok(Connection {
        route: Route { socket, peer },
        channel: Channel::load(channel::QUAKEWORLD, endpoint, 1450, 16)
            .map_err(|e| e.to_string())?,
        output: None,
        commands: None,
    })
}
#[test]
fn socket_and_local_packets_route_to_only_the_bound_endpoint() -> Result<(), String> {
    let mut peers = Connections::load(4);
    let remote = Peer::Socket(
        "127.0.0.1:27910"
            .parse()
            .map_err(|e| format!("address {e}"))?,
    );
    peers
        .bind(
            ClientId(3),
            Endpoint::Server,
            connection(1, remote, Endpoint::Server)?,
        )
        .map_err(|e| format!("bind {e:?}"))?;
    peers
        .bind(
            ClientId(1),
            Endpoint::Server,
            connection(
                Endpoint::Server.socket(),
                Peer::Loopback(ClientId(1)),
                Endpoint::Server,
            )?,
        )
        .map_err(|e| format!("bind {e:?}"))?;
    let mut bytes = [0; 1400];
    let n = headers::encode(
        headers::QUAKEWORLD,
        Direction::ToServer,
        Header {
            sequence: 1,
            ..Header::default()
        },
        b"native",
        &mut bytes,
    )
    .map_err(|e| e.to_string())?;
    let mut calls = 0;
    for (socket, peer, client) in [
        (1, remote, ClientId(3)),
        (
            Endpoint::Server.socket(),
            Peer::Loopback(ClientId(1)),
            ClientId(1),
        ),
    ] {
        peers.receive(
            socket,
            peer,
            &bytes[..n],
            EventTime(5),
            |id, endpoint, incoming| {
                assert_eq!((id, endpoint), (client, Endpoint::Server));
                if let Incoming::Payload(payload) = incoming {
                    assert_eq!(payload, b"native");
                    calls += 1;
                }
            },
        );
    }
    for (socket, peer) in [
        (2, remote),
        (Endpoint::Client.socket(), Peer::Loopback(ClientId(1))),
        (Endpoint::Server.socket(), Peer::Loopback(ClientId(99))),
    ] {
        peers.receive(socket, peer, &bytes[..n], EventTime(6), |_, _, _| {
            calls += 100
        });
    }
    assert_eq!((calls, peers.delivered, peers.unrouted), (2, 2, 3));
    assert_eq!(
        peers
            .get(ClientId(3), Endpoint::Server)
            .ok_or("channel")?
            .channel
            .state()
            .sequence,
        1
    );
    peers.unbind(ClientId(3));
    peers.receive(1, remote, &bytes[..n], EventTime(7), |_, _, _| calls += 100);
    assert_eq!(peers.unrouted, 4);
    Ok(())
}
#[test]
fn binding_rejects_ambiguous_routes_and_mismatched_endpoints() -> Result<(), String> {
    let mut peers = Connections::load(2);
    let remote = Peer::Socket(
        "127.0.0.1:27910"
            .parse()
            .map_err(|e| format!("address {e}"))?,
    );
    peers
        .bind(
            ClientId(0),
            Endpoint::Server,
            connection(1, remote, Endpoint::Server)?,
        )
        .map_err(|e| format!("bind {e:?}"))?;
    assert_eq!(
        peers.bind(
            ClientId(1),
            Endpoint::Server,
            connection(1, remote, Endpoint::Server)?
        ),
        Err(BindError::Route)
    );
    assert_eq!(
        peers.bind(
            ClientId(1),
            Endpoint::Client,
            connection(2, remote, Endpoint::Server)?
        ),
        Err(BindError::Route)
    );
    assert_eq!(
        peers.bind(
            ClientId(0),
            Endpoint::Server,
            connection(2, remote, Endpoint::Server)?
        ),
        Err(BindError::Occupied)
    );
    assert_eq!(
        peers.bind(
            ClientId(2),
            Endpoint::Server,
            connection(2, remote, Endpoint::Server)?
        ),
        Err(BindError::Client)
    );
    assert_eq!(
        peers.bind(
            ClientId(1),
            Endpoint::Server,
            connection(
                Endpoint::Client.socket(),
                Peer::Loopback(ClientId(1)),
                Endpoint::Server
            )?
        ),
        Err(BindError::Route)
    );
    assert_eq!(
        peers.bind(
            ClientId(1),
            Endpoint::Server,
            connection(
                Endpoint::Server.socket(),
                Peer::Loopback(ClientId(0)),
                Endpoint::Server
            )?
        ),
        Err(BindError::Route)
    );
    Ok(())
}

#[test]
fn negotiated_repro_bind_uses_channel_compatibility() -> Result<(), String> {
    use qa_network::commands::{connection::Commands, packet::Protocol};
    for qport in [0, 37] {
        let mut peers = Connections::load(1);
        let mut channel = Channel::load(channel::q2_new(qport != 0), Endpoint::Server, 8192, 16)
            .map_err(|e| e.to_string())?;
        channel.set_qport(qport);
        peers
            .bind(
                ClientId(0),
                Endpoint::Server,
                Connection {
                    route: Route {
                        socket: 7,
                        peer: Peer::Socket("127.0.0.1:27910".parse().map_err(|e| format!("{e}"))?),
                    },
                    channel,
                    output: None,
                    commands: Some(Commands::load(Protocol::Quake2Repro1038)),
                },
            )
            .map_err(|e| format!("{e:?}"))?;
        assert_eq!(
            peers
                .get(ClientId(0), Endpoint::Server)
                .ok_or("connection")?
                .channel
                .q2_repro_frame_ms(),
            Some(25)
        );
    }
    Ok(())
}

#[test]
fn native_qports_isolate_shared_addresses_and_update_only_the_selected_peer() -> Result<(), String>
{
    let address: std::net::SocketAddr = "127.0.0.1:27910".parse().map_err(|e| format!("{e}"))?;
    for (policy, format) in [
        (channel::QUAKEWORLD, headers::QUAKEWORLD),
        (channel::QUAKE2, headers::QUAKE2),
        (channel::QUAKE3, headers::QUAKE3),
        (channel::q2_new(true), headers::q2_new(true)),
    ] {
        let mut peers = Connections::load(3);
        for (slot, qport) in [17, 23].into_iter().enumerate() {
            let mut channel =
                Channel::load(policy, Endpoint::Server, 8192, 16).map_err(|e| e.to_string())?;
            channel.set_qport(qport);
            peers
                .bind(
                    ClientId(slot as u32),
                    Endpoint::Server,
                    Connection {
                        route: Route {
                            socket: 7,
                            peer: Peer::Socket(address),
                        },
                        channel,
                        output: None,
                        commands: None,
                    },
                )
                .map_err(|e| format!("{e:?}"))?;
        }
        let mut repeated = connection(
            7,
            Peer::Socket("127.0.0.1:28000".parse().map_err(|e| format!("{e}"))?),
            Endpoint::Server,
        )?;
        repeated.channel =
            Channel::load(policy, Endpoint::Server, 8192, 16).map_err(|e| e.to_string())?;
        repeated.channel.set_qport(23);
        assert_eq!(
            peers.bind(ClientId(2), Endpoint::Server, repeated),
            Err(BindError::Route)
        );
        let mut bytes = [0; 1400];
        let n = headers::encode(
            format,
            Direction::ToServer,
            Header {
                sequence: 1,
                qport: 23,
                ..Default::default()
            },
            b"native",
            &mut bytes,
        )
        .map_err(|e| e.to_string())?;
        let changed = Peer::Socket("127.0.0.1:28000".parse().map_err(|e| format!("{e}"))?);
        let mut count = 0;
        peers.receive(
            7,
            changed,
            &bytes[..n],
            EventTime(1),
            |id, endpoint, incoming| {
                assert_eq!((id, endpoint), (ClientId(1), Endpoint::Server));
                if let Incoming::Payload(payload) = incoming {
                    assert_eq!(payload, b"native");
                    count += 1;
                }
            },
        );
        assert_eq!(count, 1);
        assert_eq!(
            peers
                .get(ClientId(0), Endpoint::Server)
                .ok_or("first")?
                .route
                .peer,
            Peer::Socket(address)
        );
        assert_eq!(
            peers
                .get(ClientId(1), Endpoint::Server)
                .ok_or("second")?
                .route
                .peer,
            changed
        );
        for (socket, from, packet) in [
            (8, changed, &bytes[..n]),
            (
                7,
                Peer::Socket("127.0.0.2:28000".parse().map_err(|e| format!("{e}"))?),
                &bytes[..n],
            ),
            (
                7,
                Peer::Socket(
                    "[::ffff:127.0.0.1]:28000"
                        .parse()
                        .map_err(|e| format!("{e}"))?,
                ),
                &bytes[..n],
            ),
            (7, changed, &bytes[..4]),
            (7, changed, &[255; 4][..]),
        ] {
            peers.receive(socket, from, packet, EventTime(2), |_, _, _| count += 100);
        }
        assert_eq!(count, 1);
        assert_eq!(peers.unrouted, 5);
    }
    Ok(())
}

#[test]
fn omitted_qport_and_client_receivers_keep_exact_socket_routes() -> Result<(), String> {
    for (policy, format, endpoint) in [
        (channel::NETQUAKE, headers::NETQUAKE, Endpoint::Server),
        (
            channel::q2_new(false),
            headers::q2_new(false),
            Endpoint::Server,
        ),
        (channel::QUAKEWORLD, headers::QUAKEWORLD, Endpoint::Client),
        (channel::QUAKE3, headers::QUAKE3, Endpoint::Client),
    ] {
        let mut peers = Connections::load(1);
        let original = Peer::Socket("127.0.0.1:27910".parse().map_err(|e| format!("{e}"))?);
        peers
            .bind(
                ClientId(0),
                endpoint,
                Connection {
                    route: Route {
                        socket: 7,
                        peer: original,
                    },
                    channel: Channel::load(policy, endpoint, 8192, 16)
                        .map_err(|e| e.to_string())?,
                    output: None,
                    commands: None,
                },
            )
            .map_err(|e| format!("{e:?}"))?;
        let mut bytes = [0; 1400];
        let n = headers::encode(
            format,
            if endpoint == Endpoint::Server {
                Direction::ToServer
            } else {
                Direction::ToClient
            },
            Header {
                sequence: 1,
                datagram_flags: headers::datagram::UNRELIABLE,
                ..Default::default()
            },
            b"native",
            &mut bytes,
        )
        .map_err(|e| e.to_string())?;
        let translated = Peer::Socket("127.0.0.1:28000".parse().map_err(|e| format!("{e}"))?);
        let mut count = 0;
        peers.receive(7, translated, &bytes[..n], EventTime(1), |_, _, _| {
            count += 100
        });
        peers.receive(7, original, &bytes[..n], EventTime(2), |_, _, incoming| {
            if matches!(incoming, Incoming::Payload(_)) {
                count += 1
            }
        });
        assert_eq!(count, 1);
        assert_eq!(
            peers
                .get(ClientId(0), endpoint)
                .ok_or("connection")?
                .route
                .peer,
            original
        );
    }
    Ok(())
}

#[test]
fn qport_routing_keeps_native_ack_receipts_with_the_selected_peer() -> Result<(), String> {
    for (policy, format) in [
        (channel::QUAKEWORLD, headers::QUAKEWORLD),
        (channel::QUAKE2, headers::QUAKE2),
        (channel::q2_new(true), headers::q2_new(true)),
    ] {
        let mut peers = Connections::load(2);
        let address = Peer::Socket("127.0.0.1:27910".parse().map_err(|e| format!("{e}"))?);
        for (slot, qport) in [17, 23].into_iter().enumerate() {
            let mut channel =
                Channel::load(policy, Endpoint::Server, 8192, 16).map_err(|e| e.to_string())?;
            channel.set_qport(qport);
            channel
                .queue_reliable(b"retained")
                .map_err(|e| e.to_string())?;
            assert!(
                channel
                    .prepare(None, EventTime(0))
                    .map_err(|e| e.to_string())?
                    .is_some()
            );
            channel.submitted(EventTime(0)).map_err(|e| e.to_string())?;
            peers
                .bind(
                    ClientId(slot as u32),
                    Endpoint::Server,
                    Connection {
                        route: Route {
                            socket: 7,
                            peer: address,
                        },
                        channel,
                        output: None,
                        commands: None,
                    },
                )
                .map_err(|e| format!("{e:?}"))?;
        }
        let translated = Peer::Socket("127.0.0.1:28000".parse().map_err(|e| format!("{e}"))?);
        let mut bytes = [0; 1400];
        let mut receipts = 0;
        for qport in [99, 23] {
            let n = headers::encode(
                format,
                Direction::ToServer,
                Header {
                    sequence: 1,
                    acknowledgement: 1,
                    reliable_ack: true,
                    qport,
                    ..Default::default()
                },
                &[],
                &mut bytes,
            )
            .map_err(|e| e.to_string())?;
            peers.receive(
                7,
                translated,
                &bytes[..n],
                EventTime(1),
                |id, _, incoming| {
                    if matches!(incoming, Incoming::Acknowledged { .. }) {
                        assert_eq!(id, ClientId(1));
                        receipts += 1;
                    }
                },
            );
            assert_eq!(receipts, u64::from(qport == 23));
        }
        assert!(
            peers
                .get(ClientId(0), Endpoint::Server)
                .ok_or("first")?
                .channel
                .send_state()
                .reliable_bytes
                > 0
        );
        assert_eq!(
            peers
                .get(ClientId(1), Endpoint::Server)
                .ok_or("second")?
                .channel
                .send_state()
                .reliable_bytes,
            0
        );
    }
    Ok(())
}

#[test]
fn translated_ipv6_port_preserves_registered_scope_and_flow() -> Result<(), String> {
    let address = "fe80::17".parse().map_err(|e| format!("{e}"))?;
    let bound = std::net::SocketAddrV6::new(address, 27910, 11, 7);
    let from = std::net::SocketAddrV6::new(address, 28000, 13, 9);
    let mut peers = Connections::load(1);
    let mut channel = Channel::load(channel::q2_new(true), Endpoint::Server, 8192, 16)
        .map_err(|e| e.to_string())?;
    channel.set_qport(17);
    peers
        .bind(
            ClientId(0),
            Endpoint::Server,
            Connection {
                route: Route {
                    socket: 7,
                    peer: Peer::Socket(bound.into()),
                },
                channel,
                output: None,
                commands: None,
            },
        )
        .map_err(|e| format!("{e:?}"))?;
    let mut packet = [0; 1400];
    let n = headers::encode(
        headers::q2_new(true),
        Direction::ToServer,
        Header {
            sequence: 1,
            qport: 17,
            ..Default::default()
        },
        b"native",
        &mut packet,
    )
    .map_err(|e| e.to_string())?;
    let mut count = 0;
    peers.receive(
        7,
        std::net::SocketAddr::V6(from),
        &packet[..n],
        EventTime(1),
        |_, _, incoming| {
            if matches!(incoming, Incoming::Payload(_)) {
                count += 1
            }
        },
    );
    let mut expected = bound;
    expected.set_port(from.port());
    assert_eq!(
        peers
            .get(ClientId(0), Endpoint::Server)
            .ok_or("peer")?
            .route
            .peer,
        Peer::Socket(expected.into())
    );
    assert_eq!(count, 1);
    Ok(())
}

#[test]
fn mixed_header_offsets_cannot_route_an_exact_peer_to_another_protocol() -> Result<(), String> {
    let mut peers = Connections::load(2);
    for (slot, policy, port, qport) in [
        (0, channel::QUAKEWORLD, 27910, 17),
        (1, channel::QUAKE3, 28000, 23),
    ] {
        let mut channel =
            Channel::load(policy, Endpoint::Server, 8192, 16).map_err(|e| e.to_string())?;
        channel.set_qport(qport);
        peers
            .bind(
                ClientId(slot),
                Endpoint::Server,
                Connection {
                    route: Route {
                        socket: 7,
                        peer: Peer::Socket(std::net::SocketAddr::from(([127, 0, 0, 1], port))),
                    },
                    channel,
                    output: None,
                    commands: None,
                },
            )
            .map_err(|e| format!("{e:?}"))?;
    }
    let mut packet = [0; 1400];
    // This native Q3 header's body overlaps the QW header's qport position.
    let body = [0, 0, 17, 0, 0, 0];
    let n = headers::encode(
        headers::QUAKE3,
        Direction::ToServer,
        Header {
            sequence: 1,
            qport: 23,
            ..Default::default()
        },
        &body,
        &mut packet,
    )
    .map_err(|e| e.to_string())?;
    let mut selected = None;
    peers.receive(
        7,
        std::net::SocketAddr::from(([127, 0, 0, 1], 28000)),
        &packet[..n],
        EventTime(1),
        |client, _, incoming| {
            if matches!(incoming, Incoming::Payload(_)) {
                selected = Some(client);
            }
        },
    );
    assert_eq!(selected, Some(ClientId(1)));
    assert_eq!(
        peers
            .get(ClientId(0), Endpoint::Server)
            .ok_or("first")?
            .channel
            .state()
            .sequence,
        0
    );
    // With neither port registered, both native interpretations match. Drop
    // without advancing either channel or redirecting either peer.
    peers.receive(
        7,
        std::net::SocketAddr::from(([127, 0, 0, 1], 28100)),
        &packet[..n],
        EventTime(2),
        |_, _, _| panic!("ambiguous delivery"),
    );
    assert_eq!(peers.ambiguous_routes, 1);
    assert_eq!(
        peers
            .get(ClientId(0), Endpoint::Server)
            .ok_or("first")?
            .channel
            .state()
            .sequence,
        0
    );
    assert_eq!(
        peers
            .get(ClientId(0), Endpoint::Server)
            .ok_or("first")?
            .route
            .peer,
        Peer::Socket(std::net::SocketAddr::from(([127, 0, 0, 1], 27910)))
    );
    assert_eq!(
        peers
            .get(ClientId(1), Endpoint::Server)
            .ok_or("second")?
            .route
            .peer,
        Peer::Socket(std::net::SocketAddr::from(([127, 0, 0, 1], 28000)))
    );
    Ok(())
}
