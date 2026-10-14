use qa_core::sys_events::{EventKind, EventTime, QueueError, SysEvent, SysEventQueue};

#[test]
fn closed_default_queue_and_exhausted_loaded_queue_pop_none() {
    let mut queue = SysEventQueue::default();
    assert_eq!(queue.pop(), None);
    assert_eq!(queue.pop(), None);
    queue = SysEventQueue::load(2, 8).unwrap();
    let event = SysEvent {
        time: EventTime(42),
        kind: EventKind::Time,
    };
    queue.push(event).unwrap();
    assert_eq!(queue.pop(), Some(event));
    assert_eq!(queue.pop(), None);
    let mut drained = std::mem::take(&mut queue);
    assert_eq!(queue.pop(), None);
    assert_eq!(drained.pop(), None);
}

#[test]
fn physical_id_removal_and_capture_transitions_use_the_same_fifo() {
    use qa_core::sys_events::DeviceId;
    let mut queue = SysEventQueue::load(4, 1).unwrap();
    let events = [
        EventKind::DeviceRemoved(DeviceId::Keyboard(17)),
        EventKind::MouseCapture(false),
        EventKind::DeviceRemoved(DeviceId::Mouse(28)),
        EventKind::Time,
    ];
    for kind in events {
        queue
            .push(SysEvent {
                time: EventTime(42),
                kind,
            })
            .unwrap();
    }
    for kind in events {
        assert_eq!(
            queue.pop(),
            Some(SysEvent {
                time: EventTime(42),
                kind
            })
        );
    }
    assert!(queue.is_empty());
}

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

#[test]
fn millisecond_construction_keeps_console_saturation_and_think_units() {
    use qa_core::primitives::ThinkTime;
    for ms in [
        0,
        1,
        100,
        u64::from(u32::MAX),
        u64::MAX / 1_000_000,
        u64::MAX,
    ] {
        let time = EventTime::from_milliseconds(ms);
        assert_eq!(time.0, ms.saturating_mul(1_000_000));
        assert_eq!(
            ThinkTime::Milliseconds(-1).at(time),
            ThinkTime::Milliseconds((time.0 / 1_000_000) as i64)
        );
        assert_eq!(
            ThinkTime::Seconds(-0.0).at(time).seconds().to_bits(),
            (time.0 as f64 / 1e9).to_bits()
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
    assert!(queue.has_ordinary_room());
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
    assert!(!queue.has_ordinary_room());
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
    assert!(queue.has_ordinary_room());
    assert!(SysEventQueue::load(1, 20).is_err());
}

#[test]
fn text_payload_is_owned_atomic_and_keeps_the_final_time_slot() {
    use qa_core::sys_events::DeviceId;
    let mut queue = SysEventQueue::load(2, 8).unwrap();
    let mut text = String::from("λ🦀");
    fn kind(text: &str) -> EventKind<'_> {
        EventKind::Text {
            device: DeviceId::Keyboard(0),
            text,
        }
    }
    queue
        .push(SysEvent {
            time: EventTime(1),
            kind: kind(&text),
        })
        .unwrap();
    text.clear();
    assert_eq!(
        queue.push(SysEvent {
            time: EventTime(2),
            kind: kind("x")
        }),
        Err(QueueError::Full)
    );
    queue
        .push(SysEvent {
            time: EventTime(3),
            kind: EventKind::Time,
        })
        .unwrap();
    assert_eq!(
        queue.pop(),
        Some(SysEvent {
            time: EventTime(1),
            kind: kind("λ🦀")
        })
    );
    assert_eq!(
        queue.pop(),
        Some(SysEvent {
            time: EventTime(3),
            kind: EventKind::Time
        })
    );
    assert_eq!(
        queue.push(SysEvent {
            time: EventTime(4),
            kind: kind("123456789")
        }),
        Err(QueueError::PayloadFull)
    );
    assert!(queue.is_empty());
    queue
        .push(SysEvent {
            time: EventTime(5),
            kind: kind("ok"),
        })
        .unwrap();
    assert_eq!(
        queue.pop(),
        Some(SysEvent {
            time: EventTime(5),
            kind: kind("ok")
        })
    );
    assert_eq!(queue.rejected(), 2);
}
