use qa_core::{
    loopback::Endpoint,
    primitives::{ClientId, NATIVE_WIRE_BYTES, ResourceNamespace},
};
use qa_network::{
    channel::Channel,
    commands::{
        connection::Commands,
        packet::{Error, Protocol},
    },
    downloads::{self, Chunk},
    ingress::Incoming,
    message::{Encoding, Writer},
};

const RR: Protocol = Protocol::Quake2Repro1038;

// Fixed raw-DEFLATE fixtures: 10000 svc_nop bytes followed by svc_print,
// exactly 65536/65537 svc_nop bytes, and 2000 repetitions of "abc".
const LARGE_SERVICES: &[u8] = &[
    0xed, 0xc1, 0xb1, 0x0d, 0x00, 0x20, 0x08, 0x00, 0x30, 0xe2, 0xc0, 0xe0, 0x9d, 0x32, 0xb0, 0xb0,
    0xf8, 0x7f, 0xe2, 0x1d, 0x26, 0x6d, 0x33, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0xe0, 0x7f, 0x7b, 0xf5, 0xdc, 0x3e, 0x15, 0x0f,
];
const LIMIT: &[u8] = &[
    0xed, 0xc1, 0x21, 0x01, 0x00, 0x00, 0x00, 0x80, 0xa0, 0xe6, 0xff, 0xc9, 0x1e, 0x01, 0x0a, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x6a,
];
const OVERFLOW: &[u8] = &[
    0xed, 0xc1, 0x31, 0x01, 0x00, 0x00, 0x00, 0xc2, 0xa0, 0x6f, 0xfd, 0x23, 0x6b, 0x10, 0xa0, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xe0, 0x06,
];
const DOWNLOAD: &[u8] = &[
    0xed, 0xc2, 0x41, 0x11, 0x00, 0x00, 0x0c, 0x02, 0xa0, 0xac, 0x6a, 0xff, 0x0e, 0x0b, 0xb1, 0x2f,
    0x1c, 0xe9, 0xa2, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xfe, 0x1e,
];

fn setup(protocol: Protocol) -> (Commands, Channel) {
    let mut channel =
        Channel::load(protocol.channel(), Endpoint::Client, NATIVE_WIRE_BYTES, 8).unwrap();
    channel.configure_snapshots(protocol).unwrap();
    (Commands::load(protocol, NATIVE_WIRE_BYTES), channel)
}

fn apply(
    commands: &mut Commands,
    channel: &mut Channel,
    wire: &[u8],
    consume: impl FnMut(Incoming<'_>),
) -> Result<Option<u32>, Error> {
    let length = commands.stage(wire)?;
    commands.decode_output(
        length,
        1,
        channel,
        ResourceNamespace::Client(ClientId(0)),
        consume,
    )
}

fn packet(deflate: &[u8], advisory: u16) -> Vec<u8> {
    let mut wire = vec![34];
    wire.extend_from_slice(&(deflate.len() as u16).to_le_bytes());
    wire.extend_from_slice(&advisory.to_le_bytes());
    wire.extend_from_slice(deflate);
    wire
}

fn block(deflate: &[u8], percent: u8) -> Vec<u8> {
    let mut wire = vec![35];
    wire.extend_from_slice(&(deflate.len() as i16).to_le_bytes());
    wire.push(percent);
    wire.extend_from_slice(deflate);
    wire
}

// An authored RFC1951 final stored block suffices for small service fixtures.
fn stored(bytes: &[u8]) -> Vec<u8> {
    let length = u16::try_from(bytes.len()).unwrap();
    let mut wire = vec![1];
    wire.extend_from_slice(&length.to_le_bytes());
    wire.extend_from_slice(&(!length).to_le_bytes());
    wire.extend_from_slice(bytes);
    wire
}

#[test]
fn expanded_services_share_the_native_loop_and_resume_raw_services_in_order() {
    let (mut commands, mut channel) = setup(RR);
    let mut wire = b"\x0a\x02before\0".to_vec();
    wire.extend(packet(LARGE_SERVICES, 0)); // Native ignores this advisory.
    wire.extend_from_slice(b"\x0a\x02between\0");
    wire.extend(packet(&stored(b"\x0a\x02second\0"), 1));
    wire.extend_from_slice(b"\x0a\x02after\0");
    let mut prints = Vec::new();
    apply(&mut commands, &mut channel, &wire, |event| match event {
        Incoming::Print(print) => prints.push(print.text.to_vec()),
        _ => panic!("unexpected service"),
    })
    .unwrap();
    assert_eq!(
        prints,
        [
            b"before".as_slice(),
            b"inside",
            b"between",
            b"second",
            b"after"
        ]
    );

    // A large ordinary payload uses the native wire bound independently of
    // the expansion arena; it does not inherit an 8192-byte constructor.
    let mut raw = vec![6; 20000];
    raw.extend_from_slice(b"\x0a\x02large raw\0");
    apply(&mut commands, &mut channel, &raw, |event| match event {
        Incoming::Print(print) => assert_eq!(print.text, b"large raw"),
        _ => panic!("unexpected service"),
    })
    .unwrap();
    assert_eq!(
        commands.stage(&vec![6; NATIVE_WIRE_BYTES + 1]),
        Err(Error::Count)
    );
}

#[test]
fn nonfinal_complete_services_accept_native_sync_flush_and_each_envelope_starts_fresh() {
    let (mut commands, mut channel) = setup(RR);
    let mut nonfinal = stored(b"\x06\x0a\x02inside nonfinal block\0");
    nonfinal[0] = 0; // Complete stored block with BFINAL0, no stream-end marker.
    let mut wire = packet(&nonfinal, 0);
    wire.extend_from_slice(b"\x0a\x02raw after nonfinal\0");
    wire.extend(packet(&stored(b"\x0a\x02fresh envelope\0"), 1));
    let mut prints = Vec::new();
    apply(&mut commands, &mut channel, &wire, |event| match event {
        Incoming::Print(print) => prints.push(print.text.to_vec()),
        _ => panic!("unexpected service"),
    })
    .unwrap();
    assert_eq!(
        prints,
        [
            b"inside nonfinal block".to_vec(),
            b"raw after nonfinal".to_vec(),
            b"fresh envelope".to_vec()
        ]
    );
}

#[test]
fn expanded_size_is_bounded_by_actual_output_and_classic34_does_not_negotiate_it() {
    let (mut commands, mut channel) = setup(RR);
    apply(&mut commands, &mut channel, &packet(LIMIT, 0), |_| {
        panic!("only nops")
    })
    .unwrap();
    assert!(
        apply(
            &mut commands,
            &mut channel,
            &packet(OVERFLOW, 0),
            |_| panic!("only nops")
        )
        .is_err()
    );
    apply(
        &mut commands,
        &mut channel,
        b"\x0a\x02healthy\0",
        |event| match event {
            Incoming::Print(print) => assert_eq!(print.text, b"healthy"),
            _ => panic!("unexpected service"),
        },
    )
    .unwrap();

    let (mut commands, mut channel) = setup(Protocol::Quake2_34);
    for wire in [packet(LARGE_SERVICES, 10009), block(DOWNLOAD, 100)] {
        assert!(
            apply(&mut commands, &mut channel, &wire, |_| panic!(
                "unnegotiated service"
            ))
            .is_err()
        );
    }
}

#[test]
fn truncated_corrupt_and_nested_envelopes_preserve_accepted_prefixes_and_recover() {
    let (mut commands, mut channel) = setup(RR);
    let complete = packet(LARGE_SERVICES, 10009);
    let mut malformed = (1..complete.len())
        .map(|end| complete[..end].to_vec())
        .collect::<Vec<_>>();
    malformed.push(packet(&[7], 0)); // Reserved DEFLATE block type.
    // A nonfinal block is legal; its expanded bytes must still be native services.
    malformed.push(packet(b"\x00\x03\x00\xfc\xffabc", 3)); // Unknown svc 'a'.
    let mut nested = b"\x0a\x02inner prefix\0".to_vec();
    nested.extend(packet(&stored(&[6]), 1));
    malformed.push(packet(&stored(&nested), 0));
    for (index, envelope) in malformed.into_iter().enumerate() {
        let mut wire = b"\x0a\x02raw prefix\0".to_vec();
        wire.extend(envelope);
        let mut prints = Vec::new();
        assert!(
            apply(&mut commands, &mut channel, &wire, |event| match event {
                Incoming::Print(print) => prints.push(print.text.to_vec()),
                _ => panic!("unexpected service"),
            })
            .is_err(),
            "malformed envelope {index}"
        );
        assert_eq!(prints.first().unwrap(), b"raw prefix");
        apply(
            &mut commands,
            &mut channel,
            b"\x0a\x02healthy\0",
            |event| match event {
                Incoming::Print(print) => assert_eq!(print.text, b"healthy"),
                _ => panic!("unexpected service"),
            },
        )
        .unwrap();
    }
    let mut wire = packet(&stored(&nested), 0);
    wire.extend_from_slice(b"\x0a\x02must not follow\0");
    let mut prints = Vec::new();
    assert!(
        apply(&mut commands, &mut channel, &wire, |event| match event {
            Incoming::Print(print) => prints.push(print.text.to_vec()),
            _ => panic!("unexpected service"),
        })
        .is_err()
    );
    assert_eq!(prints, [b"inner prefix".to_vec()]);
}

#[test]
fn compressed_download_retains_history_and_empty_progress_across_packets() {
    let (mut commands, mut channel) = setup(RR);
    let mut decoded = Vec::new();
    let mut lengths = Vec::new();
    let mut percentages = Vec::new();
    for (bytes, percent) in [
        (&DOWNLOAD[..1], 10),
        (&DOWNLOAD[1..18], 40),
        (&DOWNLOAD[18..], 100),
    ] {
        let mut wire = block(bytes, percent);
        wire.extend_from_slice(b"\x0a\x02after block\0");
        apply(&mut commands, &mut channel, &wire, |event| match event {
            Incoming::Download(Ok(chunk)) => {
                assert!(chunk.compressed);
                let bytes = chunk.bytes.unwrap();
                decoded.extend_from_slice(bytes);
                lengths.push(bytes.len());
                percentages.push(chunk.percent);
            }
            Incoming::Print(print) => assert_eq!(print.text, b"after block"),
            _ => panic!("unexpected service"),
        })
        .unwrap();
        // svc34 owns a distinct stream while the file stream remains active.
        apply(
            &mut commands,
            &mut channel,
            &packet(&stored(&[6]), 1),
            |_| panic!("only nops"),
        )
        .unwrap();
    }
    assert_eq!(lengths[0], 0);
    assert_eq!(percentages, [10, 40, 100]);
    assert_eq!(decoded, b"abc".repeat(2000));
    // StreamEnd starts the next transfer with the same retained decoder.
    apply(
        &mut commands,
        &mut channel,
        &block(DOWNLOAD, 100),
        |event| match event {
            Incoming::Download(Ok(chunk)) => assert_eq!(chunk.bytes.unwrap(), b"abc".repeat(2000)),
            _ => panic!("unexpected service"),
        },
    )
    .unwrap();
}

#[test]
fn compressed_download_expansion_error_is_scoped_and_allows_a_fresh_retry() {
    let (mut commands, mut channel) = setup(RR);
    for malformed in [
        block(OVERFLOW, 20),
        block(&[7], 20),
        vec![35, 255, 255, 0],
        vec![35, 1, 0, 101, 0],
        vec![35, 3, 0, 20, 0],
    ] {
        let mut rejected = 0;
        assert!(
            apply(
                &mut commands,
                &mut channel,
                &malformed,
                |event| match event {
                    Incoming::Download(Err(_)) => rejected += 1,
                    _ => panic!("invalid block must not publish partial file bytes"),
                }
            )
            .is_err()
        );
        assert_eq!(rejected, 1);
        apply(
            &mut commands,
            &mut channel,
            &block(DOWNLOAD, 100),
            |event| match event {
                Incoming::Download(Ok(chunk)) => {
                    assert_eq!(chunk.bytes.unwrap(), b"abc".repeat(2000))
                }
                _ => panic!("unexpected service"),
            },
        )
        .unwrap();
    }
    let mut exact = 0;
    apply(
        &mut commands,
        &mut channel,
        &block(LIMIT, 100),
        |event| match event {
            Incoming::Download(Ok(chunk)) => {
                assert!(chunk.compressed);
                let bytes = chunk.bytes.unwrap();
                exact = bytes.len();
                assert!(bytes.iter().all(|&value| value == 6));
            }
            _ => panic!("unexpected service"),
        },
    )
    .unwrap();
    assert_eq!(exact, 65536);
}

fn header() -> Vec<u8> {
    let mut bytes = [0; 64];
    let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
    writer.write_bits(12, 8).unwrap();
    writer.write_bits(RR.number(), 32).unwrap();
    writer.write_bits(17, 32).unwrap();
    writer.write_bits(0, 8).unwrap();
    writer.write_string(RR.string_policy(), b"baseq2").unwrap();
    writer.write_bits(0, 16).unwrap();
    writer.write_string(RR.string_policy(), b"map").unwrap();
    writer.write_bits(1024, 16).unwrap();
    writer.write_bits(2, 8).unwrap();
    writer.write_bits(0, 16).unwrap();
    writer.write_bits(40, 8).unwrap();
    writer.bytes().to_vec()
}

#[test]
fn download_owner_resets_on_explicit_retirement_and_validated_native_lifecycle() {
    for reset in 0..4 {
        let (mut commands, mut channel) = setup(RR);
        apply(
            &mut commands,
            &mut channel,
            &block(&DOWNLOAD[..1], 10),
            |event| match event {
                Incoming::Download(Ok(chunk)) => assert!(chunk.bytes.unwrap().is_empty()),
                _ => panic!("unexpected service"),
            },
        )
        .unwrap();
        match reset {
            0 => commands.reset_download_stream(),
            1 => {
                apply(&mut commands, &mut channel, &header(), |event| {
                    assert!(matches!(event, Incoming::Q2ServerData(_)))
                })
                .unwrap();
            }
            2 => {
                apply(&mut commands, &mut channel, &[8], |event| {
                    assert!(matches!(event, Incoming::Reconnect))
                })
                .unwrap();
            }
            _ => {
                apply(&mut commands, &mut channel, &[7], |event| {
                    assert!(matches!(event, Incoming::Disconnect { .. }))
                })
                .unwrap();
            }
        }
        apply(
            &mut commands,
            &mut channel,
            &block(DOWNLOAD, 100),
            |event| match event {
                Incoming::Download(Ok(chunk)) => {
                    assert_eq!(chunk.bytes.unwrap(), b"abc".repeat(2000))
                }
                _ => panic!("unexpected service"),
            },
        )
        .unwrap();
    }

    // A truncated map header is not an accepted map replacement and must
    // leave a partially consumed download decoder ready for its continuation.
    let (mut commands, mut channel) = setup(RR);
    apply(
        &mut commands,
        &mut channel,
        &block(&DOWNLOAD[..1], 10),
        |_| {},
    )
    .unwrap();
    let header = header();
    assert!(
        apply(
            &mut commands,
            &mut channel,
            &header[..header.len() - 1],
            |_| panic!("unvalidated header")
        )
        .is_err()
    );
    apply(
        &mut commands,
        &mut channel,
        &block(&DOWNLOAD[1..], 100),
        |event| match event {
            Incoming::Download(Ok(chunk)) => assert_eq!(chunk.bytes.unwrap(), b"abc".repeat(2000)),
            _ => panic!("unexpected service"),
        },
    )
    .unwrap();
}

#[test]
fn forwarding_a_decoded_compressed_chunk_writes_only_the_ordinary_service() {
    let mut wire = [0xcc; 32];
    let length = downloads::write_chunk(
        RR,
        Chunk {
            percent: 100,
            bytes: Some(b"decoded"),
            compressed: true,
        },
        &mut wire,
    )
    .unwrap();
    assert_eq!(&wire[..length], b"\x10\x07\x00\x64decoded");
    let (decoded, consumed) = downloads::read_chunk(RR, &wire[..length]).unwrap();
    assert_eq!(consumed, length);
    assert!(!decoded.compressed);
    assert_eq!(decoded.bytes, Some(b"decoded".as_slice()));
}
