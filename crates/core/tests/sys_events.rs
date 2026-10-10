use qa_core::sys_events::{EventKind, EventTime, QueueError, SysEvent, SysEventQueue};

#[test]
fn seconds_preserve_the_native_duration_division() {
    for ns in [
        0,
        1,
        999_999_999,
        (1 << 53) - 1,
        1 << 53,
        (1 << 53) + 1,
        u64::MAX,
    ] {
        assert_eq!(
            EventTime(ns).seconds().to_bits(),
            (ns as f64 / 1e9).to_bits()
        );
    }
    let mut ns = 0x7f00_a715_39ce_4b8d_u64;
    for _ in 0..65_536 {
        ns = ns.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        assert_eq!(
            EventTime(ns).seconds().to_bits(),
            (ns as f64 / 1e9).to_bits()
        );
    }
}

fn line(time: u64, text: &str) -> SysEvent<'_> {
    SysEvent {
        time: EventTime(time),
        kind: EventKind::ConsoleLine(text),
    }
}

#[test]
fn payload_wrap_is_fifo_and_rejected_writes_leave_existing_bytes_intact() {
    let mut queue = SysEventQueue::load(8, 16).unwrap();
    queue.push(line(1, "abcdefgh")).unwrap();
    queue.push(line(2, "ijkl")).unwrap();
    assert_eq!(queue.pop(), Some(line(1, "abcdefgh")));
    queue.push(line(3, "mnopqr")).unwrap(); // consumes four bytes of wrap padding
    assert_eq!(queue.push(line(4, "xxx")), Err(QueueError::PayloadFull));
    assert_eq!(queue.rejected(), 1);
    assert_eq!(queue.pop(), Some(line(2, "ijkl")));
    queue.push(line(5, "stuv")).unwrap();
    assert_eq!(queue.pop(), Some(line(3, "mnopqr")));
    assert_eq!(queue.pop(), Some(line(5, "stuv")));
    assert!(queue.is_empty());
    queue.push(line(6, "0123456789abcdef")).unwrap();
    assert_eq!(queue.pop(), Some(line(6, "0123456789abcdef")));
}

#[test]
fn packet_and_utf8_console_bytes_are_owned_and_capacity_is_scoped() {
    let from = "127.0.0.1:27960"
        .parse::<std::net::SocketAddr>()
        .unwrap()
        .into();
    let mut queue = SysEventQueue::load(3, 64).unwrap();
    let mut packet = [1, 2, 3];
    queue
        .push(SysEvent {
            time: EventTime(19),
            kind: EventKind::Packet {
                socket: 2,
                from,
                bytes: &packet,
            },
        })
        .unwrap();
    packet.fill(0);
    queue.push(line(20, "echo λ🦀")).unwrap();
    assert_eq!(queue.push(line(21, "overflow")), Err(QueueError::Full));
    queue
        .push(SysEvent {
            time: EventTime(22),
            kind: EventKind::Time,
        })
        .unwrap();
    assert_eq!(
        queue.pop(),
        Some(SysEvent {
            time: EventTime(19),
            kind: EventKind::Packet {
                socket: 2,
                from,
                bytes: &[1, 2, 3]
            }
        })
    );
    assert_eq!(queue.pop(), Some(line(20, "echo λ🦀")));
    assert_eq!(
        queue.pop(),
        Some(SysEvent {
            time: EventTime(22),
            kind: EventKind::Time
        })
    );
    assert!(queue.pop().is_none());
    assert!(SysEventQueue::load(1, 20).is_err());
}
