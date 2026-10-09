use qa_core::{
    loopback::{Endpoint, Loopback, LoopbackLimits as Limits, SendError},
    primitives::ClientId,
    sys_events::{EventKind, EventTime, Peer, QueueError, SysEvent, SysEventQueue},
};

fn limits(maximum_message: usize, payload_bytes: usize, messages: usize) -> Limits {
    Limits {
        maximum_message,
        payload_bytes,
        messages,
    }
}
fn packet(queue: &mut SysEventQueue, to: Endpoint, client: u32, expected: &[u8]) {
    let event = queue.pop().unwrap();
    assert_eq!(event.time, EventTime(17));
    assert_eq!(
        event.kind,
        EventKind::Packet {
            socket: to.socket(),
            from: Peer::Loopback(ClientId(client)),
            bytes: expected,
        }
    );
}

#[test]
fn negotiated_large_messages_and_independent_seats_preserve_send_order() {
    let mut loopback = Loopback::load([
        limits(8000, 8192, 8),
        limits(64000, 128000, 8),
        limits(1400, 2800, 8),
    ])
    .unwrap();
    let mut queue = SysEventQueue::load(16, 256000).unwrap();
    let nq = vec![137; 8000];
    let qss = vec![255; 64000];
    // Both endpoint and send order survive interleaved client identities.
    for (client, bytes) in [
        (1, qss.as_slice()),
        (0, nq.as_slice()),
        (2, b"q3".as_slice()),
    ] {
        loopback
            .send(Endpoint::Client, ClientId(client), bytes)
            .unwrap();
        loopback
            .send(Endpoint::Server, ClientId(client), bytes)
            .unwrap();
    }
    assert_eq!(loopback.enqueue(&mut queue, EventTime(17)), 6);
    for to in [Endpoint::Client, Endpoint::Server] {
        packet(&mut queue, to, 1, &qss);
        packet(&mut queue, to, 0, &nq);
        packet(&mut queue, to, 2, b"q3");
        assert_eq!(loopback.pending(to), 0);
    }
    assert!(queue.is_empty());
}

#[test]
fn full_never_overwrites_or_affects_another_client_and_is_counted() {
    let mut loopback = Loopback::load([limits(8, 8, 2); 2]).unwrap();
    let mut queue = SysEventQueue::load(16, 64).unwrap();
    loopback
        .send(Endpoint::Client, ClientId(0), b"abcdefgh")
        .unwrap();
    assert_eq!(
        loopback.send(Endpoint::Client, ClientId(0), b"x"),
        Err(SendError::Full)
    );
    assert_eq!(loopback.full(Endpoint::Server, ClientId(0)), Some(1));
    loopback
        .send(Endpoint::Client, ClientId(1), b"other")
        .unwrap();
    // Header capacity also bounds zero-length messages.
    loopback.send(Endpoint::Server, ClientId(0), b"").unwrap();
    loopback.send(Endpoint::Server, ClientId(0), b"").unwrap();
    assert_eq!(
        loopback.send(Endpoint::Server, ClientId(0), b""),
        Err(SendError::Full)
    );
    assert_eq!(loopback.full(Endpoint::Client, ClientId(0)), Some(1));
    assert_eq!(
        loopback.send(Endpoint::Client, ClientId(0), b"oversized"),
        Err(SendError::PacketTooLarge)
    );
    assert_eq!(
        loopback.send(Endpoint::Client, ClientId(2), b"missing"),
        Err(SendError::Client)
    );
    assert_eq!(loopback.enqueue(&mut queue, EventTime(17)), 4);
    packet(&mut queue, Endpoint::Client, 0, b"");
    packet(&mut queue, Endpoint::Client, 0, b"");
    packet(&mut queue, Endpoint::Server, 0, b"abcdefgh");
    packet(&mut queue, Endpoint::Server, 1, b"other");
    assert_eq!(loopback.full(Endpoint::Server, ClientId(1)), Some(0));
}

#[test]
fn event_backpressure_retains_packets_until_successful_queue_admission() {
    let mut loopback = Loopback::load([limits(8, 16, 4)]).unwrap();
    let mut queue = SysEventQueue::load(2, 8).unwrap();
    loopback
        .send(Endpoint::Server, ClientId(0), b"first")
        .unwrap();
    loopback
        .send(Endpoint::Server, ClientId(0), b"second")
        .unwrap();
    queue
        .push(SysEvent {
            time: EventTime(1),
            kind: EventKind::ConsoleLine("occupied"),
        })
        .unwrap();
    assert_eq!(loopback.enqueue(&mut queue, EventTime(17)), 0);
    assert_eq!(
        loopback.pending_client(Endpoint::Client, ClientId(0)),
        Some(2)
    );
    assert_eq!(
        queue.pop().unwrap().kind,
        EventKind::ConsoleLine("occupied")
    );
    assert_eq!(loopback.enqueue(&mut queue, EventTime(17)), 1);
    packet(&mut queue, Endpoint::Client, 0, b"first");
    assert_eq!(loopback.enqueue(&mut queue, EventTime(17)), 1);
    packet(&mut queue, Endpoint::Client, 0, b"second");
    assert_eq!(loopback.full(Endpoint::Client, ClientId(0)), Some(0));
    // A destination that cannot ever hold this payload also retains it.
    loopback
        .send(Endpoint::Server, ClientId(0), b"12345678")
        .unwrap();
    let mut too_small = SysEventQueue::load(8, 7).unwrap();
    assert_eq!(loopback.enqueue(&mut too_small, EventTime(17)), 0);
    assert_eq!(loopback.pending(Endpoint::Client), 1);
    assert_eq!(too_small.rejected(), 1);
    assert_eq!(loopback.enqueue(&mut queue, EventTime(17)), 1);
    packet(&mut queue, Endpoint::Client, 0, b"12345678");
}

#[test]
fn wrap_rejections_and_client_reset_keep_other_admitted_bytes_intact() {
    let mut loopback = Loopback::load([limits(8, 16, 4); 2]).unwrap();
    let mut queue = SysEventQueue::load(2, 16).unwrap();
    loopback
        .send(Endpoint::Client, ClientId(0), b"abcdefgh")
        .unwrap();
    loopback
        .send(Endpoint::Client, ClientId(0), b"ijkl")
        .unwrap();
    assert_eq!(loopback.enqueue(&mut queue, EventTime(17)), 1);
    packet(&mut queue, Endpoint::Server, 0, b"abcdefgh");
    loopback
        .send(Endpoint::Client, ClientId(0), b"mnopqr")
        .unwrap();
    assert_eq!(
        loopback.send(Endpoint::Client, ClientId(0), b"xxx"),
        Err(SendError::Full)
    );
    assert_eq!(loopback.enqueue(&mut queue, EventTime(17)), 1);
    packet(&mut queue, Endpoint::Server, 0, b"ijkl");
    loopback
        .send(Endpoint::Client, ClientId(0), b"stuv")
        .unwrap();
    for bytes in [b"mnopqr".as_slice(), b"stuv".as_slice()] {
        assert_eq!(loopback.enqueue(&mut queue, EventTime(17)), 1);
        packet(&mut queue, Endpoint::Server, 0, bytes);
    }
    for id in 0..1000u32 {
        let bytes = id.to_ne_bytes();
        loopback
            .send(Endpoint::Client, ClientId(0), &bytes)
            .unwrap();
        assert_eq!(loopback.enqueue(&mut queue, EventTime(17)), 1);
        packet(&mut queue, Endpoint::Server, 0, &bytes);
    }
    loopback
        .send(Endpoint::Client, ClientId(0), b"old")
        .unwrap();
    loopback
        .send(Endpoint::Client, ClientId(1), b"keep")
        .unwrap();
    loopback.clear_client(ClientId(0));
    assert_eq!(loopback.enqueue(&mut queue, EventTime(17)), 1);
    packet(&mut queue, Endpoint::Server, 1, b"keep");
    loopback.clear();
    assert_eq!(loopback.full(Endpoint::Server, ClientId(0)), Some(0));
    assert_eq!(loopback.pending(Endpoint::Server), 0);
    assert!(Loopback::load([limits(9, 8, 2)]).is_err());
    assert!(Loopback::load([limits(8, 8, 0)]).is_err());
    assert_eq!(SysEventQueue::load(1, 8).err(), Some(QueueError::Capacity));
}
