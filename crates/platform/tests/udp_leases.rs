use qa_core::sys_events::{EventKind, SysEventQueue};
use qa_platform::EventPump;
use std::{io, net::UdpSocket, time::Duration};

fn receiver(address: &str) -> io::Result<UdpSocket> {
    let socket = UdpSocket::bind(address)?;
    socket.set_read_timeout(Some(Duration::from_secs(1)))?;
    Ok(socket)
}

#[test]
fn dropping_a_client_lease_closes_its_socket_and_fences_queued_packets() {
    let remote = receiver("127.0.0.1:0").unwrap();
    let peer = remote.local_addr().unwrap();
    let mut pump = EventPump::new();
    let (lease, resolved) = pump.open_udp_client(&peer.to_string(), 26000).unwrap();
    assert_eq!(resolved, peer);
    let old_token = lease.socket();
    pump.send_udp(old_token, peer, b"old request").unwrap();
    let mut bytes = [0; 64];
    let (length, old_address) = remote.recv_from(&mut bytes).unwrap();
    assert_eq!(&bytes[..length], b"old request");
    assert!(UdpSocket::bind(old_address).is_err());

    remote.send_to(b"queued old response", old_address).unwrap();
    let mut queue = SysEventQueue::load(8, 128).unwrap();
    pump.poll_network(&mut queue);
    assert_eq!(queue.len(), 1);
    drop(lease);
    assert_eq!(
        pump.send_udp(old_token, peer, b"stale").unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
    // The weak event-source record does not keep the OS socket alive.
    let released = UdpSocket::bind(old_address).unwrap();
    let (replacement, _) = pump.open_udp_client(&peer.to_string(), 26000).unwrap();
    assert_ne!(replacement.socket(), old_token);
    assert!(
        matches!(queue.pop().unwrap().kind, EventKind::Packet { socket, from, bytes: b"queued old response" } if socket == old_token && from == peer.into())
    );
    assert_eq!(
        pump.send_udp(old_token, peer, b"stale").unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
    pump.send_udp(replacement.socket(), peer, b"new request")
        .unwrap();
    let (length, new_address) = remote.recv_from(&mut bytes).unwrap();
    assert_eq!(&bytes[..length], b"new request");
    assert_ne!(new_address, old_address);
    remote.send_to(b"new response", new_address).unwrap();
    pump.poll_network(&mut queue);
    assert!(
        matches!(queue.pop().unwrap().kind, EventKind::Packet { socket, bytes: b"new response", .. } if socket == replacement.socket())
    );
    drop(released);
}

#[test]
fn released_client_slots_are_reusable_and_bound_listeners_remain_owned() {
    let mut pump = EventPump::new();
    let (bound, local) = pump.bind_udp("127.0.0.1:0".parse().unwrap()).unwrap();
    let remote = receiver("127.0.0.1:0").unwrap();
    let peer = remote.local_addr().unwrap();
    let mut previous = bound;
    for _ in 0..32 {
        assert!(pump.open_udp_client("", 26000).is_err());
        assert!(pump.open_udp_client("127.0.0.1:0", 26000).is_err());
        let (lease, _) = pump.open_udp_client(&peer.to_string(), 26000).unwrap();
        assert!(lease.socket() > previous);
        previous = lease.socket();
        drop(lease);
    }
    let mut clients = Vec::new();
    for _ in 0..7 {
        clients.push(pump.open_udp_client(&peer.to_string(), 26000).unwrap().0);
    }
    assert_eq!(
        pump.open_udp_client(&peer.to_string(), 26000)
            .err()
            .unwrap()
            .kind(),
        io::ErrorKind::OutOfMemory
    );
    let old_token = clients.last().unwrap().socket();
    drop(clients.pop());
    let (replacement, _) = pump.open_udp_client(&peer.to_string(), 26000).unwrap();
    assert!(replacement.socket() > old_token);
    assert_eq!(
        pump.send_udp(old_token, peer, b"stale").unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );

    pump.send_udp(bound, peer, b"listener survived").unwrap();
    let mut bytes = [0; 64];
    let (length, from) = remote.recv_from(&mut bytes).unwrap();
    assert_eq!(&bytes[..length], b"listener survived");
    assert_eq!(from, local);
    remote.send_to(b"listener input", local).unwrap();
    let mut queue = SysEventQueue::load(8, 128).unwrap();
    pump.poll_network(&mut queue);
    assert!(
        matches!(queue.pop().unwrap().kind, EventKind::Packet { socket, bytes: b"listener input", .. } if socket == bound)
    );
}

#[test]
fn client_targets_use_standard_ipv4_ipv6_and_hostname_port_resolution() {
    let mut pump = EventPump::new();
    let remote = receiver("127.0.0.1:0").unwrap();
    let peer = remote.local_addr().unwrap();
    for (target, default_port) in [("127.0.0.1".to_owned(), peer.port()), (peer.to_string(), 1)] {
        let (lease, resolved) = pump.open_udp_client(&target, default_port).unwrap();
        assert_eq!(resolved, peer);
        pump.send_udp(lease.socket(), resolved, b"resolved IPv4")
            .unwrap();
        let mut bytes = [0; 32];
        let (length, _) = remote.recv_from(&mut bytes).unwrap();
        assert_eq!(&bytes[..length], b"resolved IPv4");
    }
    for (target, default_port) in [
        ("localhost".to_owned(), peer.port()),
        (format!("localhost:{}", peer.port()), 1),
    ] {
        let (lease, resolved) = pump.open_udp_client(&target, default_port).unwrap();
        assert!(resolved.ip().is_loopback());
        assert_eq!(resolved.port(), peer.port());
        drop(lease);
    }
    if let Ok(remote) = receiver("[::1]:0") {
        let peer = remote.local_addr().unwrap();
        for (target, default_port) in [
            ("::1".to_owned(), peer.port()),
            ("[::1]".to_owned(), peer.port()),
            (peer.to_string(), 1),
        ] {
            let (lease, resolved) = pump.open_udp_client(&target, default_port).unwrap();
            assert_eq!(resolved, peer);
            pump.send_udp(lease.socket(), resolved, b"resolved IPv6")
                .unwrap();
            let mut bytes = [0; 32];
            let (length, _) = remote.recv_from(&mut bytes).unwrap();
            assert_eq!(&bytes[..length], b"resolved IPv6");
        }
    }
    for target in [
        "",
        "127.0.0.1:0",
        "[::1]:0",
        "[::1]:65536",
        "127.0.0.1:65536",
    ] {
        assert!(pump.open_udp_client(target, 26000).is_err());
    }
    assert!(pump.open_udp_client("127.0.0.1", 0).is_err());
}

#[test]
fn dropping_the_event_source_does_not_invalidate_a_retained_lease() {
    let remote = receiver("127.0.0.1:0").unwrap();
    let mut pump = EventPump::new();
    let (lease, _) = pump
        .open_udp_client(&remote.local_addr().unwrap().to_string(), 26000)
        .unwrap();
    pump.send_udp(lease.socket(), remote.local_addr().unwrap(), b"owned")
        .unwrap();
    let mut bytes = [0; 32];
    let (_, local) = remote.recv_from(&mut bytes).unwrap();
    drop(pump);
    assert!(UdpSocket::bind(local).is_err());
    drop(lease);
    let released = UdpSocket::bind(local).unwrap();
    assert_eq!(released.local_addr().unwrap(), local);
}

#[test]
fn client_leases_cache_their_distinct_native_ports_at_the_actual_socket_boundary() {
    let remote = receiver("127.0.0.1:0").unwrap();
    let peer = remote.local_addr().unwrap();
    let mut pump = EventPump::new();
    let first = pump.open_udp_client(&peer.to_string(), 27500).unwrap().0;
    let second = pump.open_udp_client(&peer.to_string(), 27500).unwrap().0;
    assert_ne!(first.socket(), second.socket());
    assert_ne!(first.local_port(), 0);
    assert_ne!(second.local_port(), 0);
    assert_ne!(first.local_port(), second.local_port());
    let ports = [first.local_port(), second.local_port()];
    for (index, lease) in [&first, &second].into_iter().enumerate() {
        pump.send_udp(lease.socket(), peer, b"native qport")
            .unwrap();
        let mut bytes = [0; 32];
        let (length, source) = remote.recv_from(&mut bytes).unwrap();
        assert_eq!(&bytes[..length], b"native qport");
        assert_eq!(source.port(), ports[index]);
        assert_eq!(lease.local_port(), ports[index]);
    }
    drop(pump);
    assert_eq!([first.local_port(), second.local_port()], ports);
}
