use qa_core::loopback::{Endpoint, Loopback, MESSAGES, PACKET_BYTES, SendError};
use qa_core::primitives::ClientId;

#[test]
fn local_clients_share_transport_without_losing_connection_identity() {
    let mut loopback = Loopback::load();
    for id in [1, 7, 63] {
        loopback
            .send(Endpoint::Client, ClientId(id), b"command")
            .unwrap();
        loopback
            .send(Endpoint::Server, ClientId(id), b"snapshot")
            .unwrap();
    }
    for to in [Endpoint::Client, Endpoint::Server] {
        for id in [1, 7, 63] {
            let packet = loopback.receive(to).unwrap();
            assert_eq!(packet.client, ClientId(id));
            assert_eq!(
                packet.bytes,
                if to == Endpoint::Server {
                    b"command".as_slice()
                } else {
                    b"snapshot".as_slice()
                }
            );
        }
    }
    assert_ne!(Endpoint::Client.socket(), Endpoint::Server.socket());
}

#[test]
fn independent_directions_fifo_overwrite_and_wrap_match_q3() {
    let mut loopback = Loopback::load();
    for id in 0..40u8 {
        loopback
            .send(Endpoint::Client, ClientId(4), &[id, 0, 255])
            .unwrap();
    }
    loopback
        .send(Endpoint::Server, ClientId(7), b"snapshot")
        .unwrap();
    assert_eq!(loopback.pending(Endpoint::Server), MESSAGES);
    assert_eq!(loopback.overwritten(Endpoint::Server), 24);
    assert_eq!(
        loopback
            .receive(Endpoint::Client)
            .map(|packet| packet.bytes),
        Some(b"snapshot".as_slice())
    );
    for id in 24..40u8 {
        assert_eq!(
            loopback
                .receive(Endpoint::Server)
                .map(|packet| packet.bytes),
            Some([id, 0, 255].as_slice())
        );
    }
    assert_eq!(
        loopback
            .receive(Endpoint::Server)
            .map(|packet| packet.bytes),
        None
    );
    for id in 0..1000u32 {
        let packet = id.to_ne_bytes();
        loopback
            .send(Endpoint::Client, ClientId(4), &packet)
            .unwrap();
        assert_eq!(
            loopback
                .receive(Endpoint::Server)
                .map(|packet| packet.bytes),
            Some(packet.as_slice())
        );
    }
}

#[test]
fn empty_full_binary_and_oversize_packets_preserve_owned_storage() {
    let mut loopback = Loopback::load();
    let mut packet = [0; PACKET_BYTES];
    for (index, byte) in packet.iter_mut().enumerate() {
        *byte = index as u8;
    }
    loopback
        .send(Endpoint::Server, ClientId(7), &packet)
        .unwrap();
    loopback.send(Endpoint::Server, ClientId(7), b"").unwrap();
    assert_eq!(
        loopback.send(Endpoint::Server, ClientId(7), &[0; PACKET_BYTES + 1]),
        Err(SendError::PacketTooLarge)
    );
    assert_eq!(loopback.pending(Endpoint::Client), 2);
    assert_eq!(
        loopback
            .receive(Endpoint::Client)
            .map(|packet| packet.bytes),
        Some(packet.as_slice())
    );
    assert_eq!(
        loopback
            .receive(Endpoint::Client)
            .map(|packet| packet.bytes),
        Some(b"".as_slice())
    );
    assert_eq!(
        loopback
            .receive(Endpoint::Client)
            .map(|packet| packet.bytes),
        None
    );
    loopback
        .send(Endpoint::Client, ClientId(4), b"old session")
        .unwrap();
    loopback.clear();
    assert_eq!(
        loopback
            .receive(Endpoint::Server)
            .map(|packet| packet.bytes),
        None
    );
    assert_eq!(loopback.overwritten(Endpoint::Server), 0);
}
