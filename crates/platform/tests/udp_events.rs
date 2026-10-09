use qa_core::sys_events::{EventKind, SysEventQueue};
use qa_platform::EventPump;
use std::net::UdpSocket;

#[test]
fn nonblocking_udp_produces_owned_ordered_events_and_drops_overruns() {
    let mut pump = EventPump::new();
    let (socket, address) = pump.bind_udp("127.0.0.1:0".parse().unwrap()).unwrap();
    let sender = UdpSocket::bind("127.0.0.1:0").unwrap();
    sender.send_to(b"first", address).unwrap();
    sender.send_to(b"second", address).unwrap();
    let mut queue = SysEventQueue::load(8, 64).unwrap();
    pump.poll_network(&mut queue);
    let mut last = None;
    for expected in [b"first".as_slice(), b"second".as_slice()] {
        let event = queue.pop().unwrap();
        if let Some(time) = last {
            assert!(event.time >= time);
        }
        last = Some(event.time);
        assert_eq!(
            event.kind,
            EventKind::Packet {
                socket,
                from: sender.local_addr().unwrap().into(),
                bytes: expected
            }
        );
    }
    assert!(queue.is_empty());
    // A full payload arena drops a datagram without invalidating its predecessor.
    let mut queue = SysEventQueue::load(8, 5).unwrap();
    sender.send_to(b"first", address).unwrap();
    sender.send_to(b"second", address).unwrap();
    pump.poll_network(&mut queue);
    assert_eq!(queue.len(), 1);
    assert_eq!(pump.dropped_packets(), 1);
    assert_eq!(pump.socket_errors(), 0);
    assert!(matches!(
        queue.pop().unwrap().kind,
        EventKind::Packet {
            bytes: b"first",
            ..
        }
    ));
    pump.poll_network(&mut queue);
    assert!(queue.is_empty());
}
