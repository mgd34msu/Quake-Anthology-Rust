use qa_network::headers::{self, Direction, Fragment, Header, QPort};

#[test]
fn native_datagrams_preserve_big_endian_flags_length_and_independent_sequence() {
    let mut out = [0; 64];
    let header = Header {
        sequence: 0x1234_5678,
        datagram_flags: headers::datagram::UNRELIABLE,
        ..Header::default()
    };
    let length = headers::encode(
        headers::NETQUAKE,
        Direction::ToServer,
        header,
        b"abc",
        &mut out,
    )
    .unwrap();
    assert_eq!(
        &out[..length],
        &[0, 0x10, 0, 11, 0x12, 0x34, 0x56, 0x78, b'a', b'b', b'c']
    );
    assert_eq!(
        headers::decode(headers::NETQUAKE, Direction::ToClient, &out[..length]).unwrap(),
        (header, b"abc".as_slice())
    );
    let ack = Header {
        sequence: u32::MAX,
        datagram_flags: headers::datagram::ACK,
        ..Header::default()
    };
    let length =
        headers::encode(headers::NETQUAKE, Direction::ToClient, ack, &[], &mut out).unwrap();
    assert_eq!(&out[..length], &[0, 2, 0, 8, 255, 255, 255, 255]);
    assert_eq!(
        headers::decode(headers::NETQUAKE, Direction::ToServer, &out[..length])
            .unwrap()
            .0,
        ack
    );
    // Native declared lengths ignore trailing transport bytes, never overread.
    out[length] = 0xab;
    assert_eq!(
        headers::decode(headers::NETQUAKE, Direction::ToServer, &out[..length + 1])
            .unwrap()
            .1,
        &[]
    );
}

#[test]
fn native_toggle_headers_project_qport_width_and_client_direction() {
    let header = Header {
        sequence: 7,
        acknowledgement: 5,
        reliable: true,
        reliable_ack: true,
        qport: 0x1234,
        ..Header::default()
    };
    let mut out = [0; 64];
    let length = headers::encode(
        headers::QUAKEWORLD,
        Direction::ToServer,
        header,
        b"a",
        &mut out,
    )
    .unwrap();
    assert_eq!(
        &out[..length],
        &[7, 0, 0, 128, 5, 0, 0, 128, 0x34, 0x12, b'a']
    );
    assert_eq!(
        headers::decode(headers::QUAKE2, Direction::ToServer, &out[..length])
            .unwrap()
            .0,
        header
    );
    for (port, size, expected) in [
        (QPort::None, 9, 0),
        (QPort::Byte, 10, 0x34),
        (QPort::Short, 11, 0x1234),
    ] {
        let format = headers::q2_old(port);
        let length = headers::encode(format, Direction::ToServer, header, b"a", &mut out).unwrap();
        assert_eq!(length, size);
        assert_eq!(
            headers::decode(format, Direction::ToServer, &out[..length])
                .unwrap()
                .0
                .qport,
            expected
        );
        let length = headers::encode(format, Direction::ToClient, header, b"a", &mut out).unwrap();
        assert_eq!(length, 9);
        assert_eq!(
            headers::decode(format, Direction::ToClient, &out[..length])
                .unwrap()
                .0
                .qport,
            0
        );
    }
    // QW peers can send their original 1450-byte payload plus the client header.
    let mut native = [0; 1460];
    let length = headers::encode(
        headers::QUAKEWORLD,
        Direction::ToServer,
        header,
        &[23; 1450],
        &mut native,
    )
    .unwrap();
    assert_eq!(
        headers::decode(headers::QUAKEWORLD, Direction::ToServer, &native[..length])
            .unwrap()
            .1
            .len(),
        1450
    );
}

#[test]
fn q3_and_q2pro_fragments_keep_their_distinct_native_end_markers() {
    let header = Header {
        sequence: 7,
        acknowledgement: 5,
        reliable: true,
        reliable_ack: true,
        qport: 0x1234,
        fragment: Some(Fragment {
            offset: 1300,
            more: true,
        }),
        ..Header::default()
    };
    let mut out = [0; 1400];
    let length = headers::encode(
        headers::QUAKE3,
        Direction::ToServer,
        header,
        b"abc",
        &mut out,
    )
    .unwrap();
    assert_eq!(
        &out[..length],
        &[7, 0, 0, 128, 0x34, 0x12, 0x14, 5, 3, 0, b'a', b'b', b'c']
    );
    let (read, payload) =
        headers::decode(headers::QUAKE3, Direction::ToServer, &out[..length]).unwrap();
    assert_eq!(read.acknowledgement, 0);
    assert!(!read.reliable && !read.reliable_ack);
    assert_eq!(
        read.fragment,
        Some(Fragment {
            offset: 1300,
            more: false
        })
    );
    assert_eq!(payload, b"abc");
    let length = headers::encode(
        headers::QUAKE3,
        Direction::ToClient,
        header,
        &[17; 1300],
        &mut out,
    )
    .unwrap();
    assert!(
        headers::decode(headers::QUAKE3, Direction::ToClient, &out[..length])
            .unwrap()
            .0
            .fragment
            .unwrap()
            .more
    );
    let length =
        headers::encode(headers::QUAKE3, Direction::ToClient, header, &[], &mut out).unwrap();
    assert_eq!(
        headers::decode(headers::QUAKE3, Direction::ToClient, &out[..length])
            .unwrap()
            .0
            .fragment,
        Some(Fragment {
            offset: 1300,
            more: false
        })
    );
    let length = headers::encode(
        headers::q2_new(true),
        Direction::ToServer,
        header,
        b"abc",
        &mut out,
    )
    .unwrap();
    assert_eq!(
        &out[..length],
        &[
            7, 0, 0, 192, 5, 0, 0, 128, 0x34, 0x14, 0x85, b'a', b'b', b'c'
        ]
    );
    let (read, payload) =
        headers::decode(headers::q2_new(true), Direction::ToServer, &out[..length]).unwrap();
    assert_eq!(read.fragment, header.fragment);
    assert!(read.reliable && read.reliable_ack);
    assert_eq!(payload, b"abc");
}

#[test]
fn truncated_or_connectionless_headers_do_not_become_connected_payloads() {
    let mut out = [0x55; 64];
    let previous = out;
    assert!(
        headers::encode(
            headers::QUAKE3,
            Direction::ToServer,
            Header::default(),
            &[0; 100],
            &mut out
        )
        .is_err()
    );
    assert_eq!(out, previous);
    for format in [
        headers::NETQUAKE,
        headers::QUAKEWORLD,
        headers::QUAKE3,
        headers::q2_new(true),
    ] {
        assert!(headers::decode(format, Direction::ToServer, &[]).is_err());
        assert_eq!(
            headers::decode(format, Direction::ToServer, &[255; 4]),
            Err(headers::Error::Connectionless)
        );
    }
    assert!(
        headers::decode(
            headers::NETQUAKE,
            Direction::ToServer,
            &[0, 1, 0, 100, 0, 0, 0, 0]
        )
        .is_err()
    );
    assert!(
        headers::decode(
            headers::QUAKE3,
            Direction::ToServer,
            &[1, 0, 0, 128, 0, 0, 0, 0, 2, 0, 0]
        )
        .is_err()
    );
    assert!(
        headers::encode(
            headers::QUAKEWORLD,
            Direction::ToServer,
            Header {
                fragment: Some(Fragment::default()),
                ..Header::default()
            },
            &[],
            &mut out
        )
        .is_err()
    );
}
