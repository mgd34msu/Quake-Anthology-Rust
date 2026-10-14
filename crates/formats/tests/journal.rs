use qa_core::{primitives::ClientId, sys_events::*};
use qa_formats::{FormatError, journal::*};
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV6};

#[test]
fn every_event_round_trips_with_full_time_device_and_peer_widths() {
    let keyboard = DeviceId::Keyboard(u32::MAX);
    let mouse = DeviceId::Mouse(0x9abc_1234);
    let controller = DeviceId::Controller(i32::MIN);
    let kinds = [
        EventKind::Time,
        EventKind::Key {
            device: keyboard,
            code: u16::MAX,
            symbol: i32::MIN,
            down: true,
            repeat: false,
        },
        EventKind::Key {
            device: keyboard,
            code: 0,
            symbol: i32::MAX,
            down: false,
            repeat: true,
        },
        EventKind::Text {
            device: keyboard,
            text: "é🦀\0",
        },
        EventKind::Mouse {
            device: mouse,
            dx: i32::MIN,
            dy: i32::MAX,
        },
        EventKind::MouseButton {
            device: mouse,
            button: u8::MAX,
            down: true,
        },
        EventKind::MouseWheel {
            device: mouse,
            x: i32::MAX,
            y: i32::MIN,
        },
        EventKind::ControllerAxis {
            device: controller,
            axis: 255,
            value: i16::MIN,
        },
        EventKind::ControllerButton {
            device: controller,
            button: 0,
            down: false,
        },
        EventKind::DeviceRemoved(controller),
        EventKind::Focus(true),
        EventKind::MouseCapture(false),
        EventKind::Quit,
        EventKind::ConsoleLine("set name \"all games\"; echo done\n"),
        EventKind::Packet {
            socket: u16::MAX,
            from: Peer::Loopback(ClientId(u32::MAX)),
            bytes: &[0, 255, 128, 0],
        },
        EventKind::Packet {
            socket: 7,
            from: Peer::Socket(SocketAddr::new(Ipv4Addr::new(127, 2, 3, 4).into(), 65535)),
            bytes: &[],
        },
        EventKind::Packet {
            socket: 0,
            from: Peer::Socket(SocketAddr::V6(SocketAddrV6::new(
                Ipv6Addr::LOCALHOST,
                27960,
                u32::MAX,
                17,
            ))),
            bytes: b"native bytes",
        },
    ];
    check_file_header(&FILE_HEADER).unwrap();
    for (index, kind) in kinds.into_iter().enumerate() {
        let event = SysEvent {
            time: EventTime(u64::MAX - index as u64),
            kind,
        };
        let header = encode_header(event).unwrap();
        assert_eq!(payload_length(&header), kind.payload().len());
        assert_eq!(decode(&header, kind.payload()).unwrap(), event);
        let mut queue = SysEventQueue::load(4, 512).unwrap();
        queue
            .push(decode(&header, kind.payload()).unwrap())
            .unwrap();
        assert_eq!(queue.pop().unwrap(), event);
    }
}

#[test]
fn record_bytes_are_canonical_little_endian_and_do_not_truncate_payload_lengths() {
    let text = "x".repeat(70_000);
    let event = SysEvent {
        time: EventTime(0x1020_3040_5060_7080),
        kind: EventKind::ConsoleLine(&text),
    };
    let header = encode_header(event).unwrap();
    assert_eq!(&header[..4], &70_000_u32.to_le_bytes());
    assert_eq!(
        &header[4..12],
        &[0x80, 0x70, 0x60, 0x50, 0x40, 0x30, 0x20, 0x10]
    );
    assert_eq!(header[12], 12);
    assert!(header[13..].iter().all(|&byte| byte == 0));
    assert_eq!(decode(&header, text.as_bytes()).unwrap(), event);
}

#[test]
fn invalid_tags_booleans_reserved_bytes_and_payloads_fail_at_the_reader_boundary() {
    let event = SysEvent {
        time: EventTime(1),
        kind: EventKind::Focus(true),
    };
    let original = encode_header(event).unwrap();
    for (index, value) in [(12, 14), (13, 2), (47, 1)] {
        let mut header = original;
        header[index] = value;
        assert_eq!(decode(&header, &[]), Err(FormatError::InvalidValue));
    }
    assert_eq!(
        decode(&original, b"surplus"),
        Err(FormatError::InvalidRange)
    );
    let mut header = original;
    header[..4].copy_from_slice(&1_u32.to_le_bytes());
    assert_eq!(decode(&header, b"x"), Err(FormatError::InvalidRange));
    let text = encode_header(SysEvent {
        time: EventTime(1),
        kind: EventKind::ConsoleLine("x"),
    })
    .unwrap();
    assert_eq!(decode(&text, &[255]), Err(FormatError::InvalidValue));
    assert_eq!(decode(&text, &[]), Err(FormatError::InvalidRange));
    let mut version = FILE_HEADER;
    version[4] = 2;
    assert_eq!(check_file_header(&version), Err(FormatError::Unsupported));
}
