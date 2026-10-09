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
