use qa_core::loopback::Endpoint;
use qa_core::primitives::{ClientId, PrintKind, ResourceNamespace};
use qa_network::{
    channel::Channel,
    commands::{connection::Commands, packet::Protocol},
    downloads::{self, Chunk, Error},
    ingress::Incoming,
    message::{Encoding, ErrorKind, Writer},
};

const SEQUENTIAL: [Protocol; 3] = [
    Protocol::QuakeWorld28,
    Protocol::Quake2_34,
    Protocol::Quake2Repro1038,
];

#[test]
fn native_sequential_golden_blocks_preserve_signed_size_progress_and_raw_bytes() {
    for protocol in SEQUENTIAL {
        let opcode = if protocol == Protocol::QuakeWorld28 {
            41
        } else {
            16
        };
        for (chunk, expected) in [
            (
                Chunk {
                    percent: 40,
                    bytes: Some(&[0, 128, 255]),
                },
                vec![opcode, 3, 0, 40, 0, 128, 255],
            ),
            (
                Chunk {
                    percent: 0,
                    bytes: None,
                },
                vec![opcode, 255, 255, 0],
            ),
            (
                Chunk {
                    percent: 100,
                    bytes: Some(&[]),
                },
                vec![opcode, 0, 0, 100],
            ),
        ] {
            let mut wire = [0xcc; 16];
            let length = downloads::write_chunk(protocol, chunk, &mut wire).unwrap();
            assert_eq!(&wire[..length], expected);
            assert!(wire[length..].iter().all(|&byte| byte == 0xcc));
            let (decoded, consumed) = downloads::read_chunk(protocol, &wire[..length]).unwrap();
            assert_eq!((decoded, consumed), (chunk, length));
            if let Some(data) = decoded.bytes {
                assert_eq!(
                    data.as_ptr(),
                    wire[4..].as_ptr(),
                    "decoder borrows packet storage"
                );
            }
        }
    }
    assert!(downloads::sequential(Protocol::NetQuake15).is_none());
    assert!(downloads::sequential(Protocol::Quake3_68).is_none());
    assert!(
        !downloads::sequential(Protocol::QuakeWorld28)
            .unwrap()
            .empty_completes()
    );
    assert!(
        downloads::sequential(Protocol::Quake2_34)
            .unwrap()
            .empty_completes()
    );
}

#[test]
fn malformed_or_truncated_blocks_never_borrow_beyond_the_native_service() {
    for protocol in SEQUENTIAL {
        let mut wire = [0; 2048];
        let policy = downloads::sequential(protocol).unwrap();
        let data = vec![71; policy.block_bytes()];
        let length = downloads::write_chunk(
            protocol,
            Chunk {
                percent: 50,
                bytes: Some(&data),
            },
            &mut wire,
        )
        .unwrap();
        for end in 0..length {
            assert!(
                matches!(downloads::read_chunk(protocol, &wire[..end]), Err(Error::Message(error)) if error.kind == ErrorKind::Truncated)
            );
        }
        wire[1..3].copy_from_slice(&(-2i16).to_le_bytes());
        assert_eq!(
            downloads::read_chunk(protocol, &wire[..length]),
            Err(Error::Size)
        );
        wire[1..3].copy_from_slice(&0i16.to_le_bytes());
        wire[3] = 101;
        assert_eq!(
            downloads::read_chunk(protocol, &wire[..length]),
            Err(Error::Percent)
        );
        assert_eq!(
            downloads::write_chunk(
                protocol,
                Chunk {
                    percent: 1,
                    bytes: Some(&vec![0; policy.block_bytes() + 1])
                },
                &mut wire
            ),
            Err(Error::Size)
        );
    }
    let data = vec![15; 1200];
    let mut raw = vec![0; data.len() + 4];
    let mut writer = Writer::new(&mut raw, Encoding::Bytes);
    writer.write_bits(41, 8).unwrap();
    writer.write_bits(data.len() as u32, 16).unwrap();
    writer.write_bits(99, 8).unwrap();
    writer.write_data(&data).unwrap();
    let (chunk, consumed) = downloads::read_chunk(Protocol::QuakeWorld28, &raw).unwrap();
    assert_eq!(chunk.bytes, Some(data.as_slice()));
    assert_eq!(
        consumed,
        raw.len(),
        "receive supports positive native sizes above the SERVER chunk preference"
    );
}

#[test]
fn connection_parser_delivers_blocks_and_following_services_in_native_order() {
    for protocol in SEQUENTIAL {
        let mut wire = [0; 256];
        let mut length = 0;
        for chunk in [
            Chunk {
                percent: 35,
                bytes: Some(b"first"),
            },
            Chunk {
                percent: 100,
                bytes: Some(b"last"),
            },
        ] {
            length += downloads::write_chunk(protocol, chunk, &mut wire[length..]).unwrap();
        }
        length += qa_network::outputs::print(
            protocol,
            PrintKind::Console,
            2,
            b"after blocks",
            None,
            &mut wire[length..],
        )
        .unwrap();
        let mut channel = Channel::load(protocol.channel(), Endpoint::Client, 8192, 16).unwrap();
        let mut commands = Commands::load(protocol, 8192);
        let staged = commands.stage(&wire[..length]).unwrap();
        let mut received = Vec::new();
        commands
            .decode_output(
                staged,
                1,
                &mut channel,
                ResourceNamespace::Client(ClientId(0)),
                |event| match event {
                    Incoming::Download(Ok(chunk)) => {
                        received.push((chunk.percent, chunk.bytes.unwrap().to_vec()))
                    }
                    Incoming::Print(print) => {
                        assert_eq!(received, [(35, b"first".to_vec()), (100, b"last".to_vec())]);
                        assert_eq!(print.text, b"after blocks");
                        received.push((255, print.text.to_vec()));
                    }
                    _ => panic!("unexpected service"),
                },
            )
            .unwrap();
        assert_eq!(received.len(), 3);

        let mut malformed = wire[..length].to_vec();
        malformed[1..3].copy_from_slice(&(-2i16).to_le_bytes());
        let staged = commands.stage(&malformed).unwrap();
        let mut rejected = 0;
        assert!(
            commands
                .decode_output(
                    staged,
                    2,
                    &mut channel,
                    ResourceNamespace::Client(ClientId(0)),
                    |event| match event {
                        Incoming::Download(Err(Error::Size)) => rejected += 1,
                        _ => panic!("invalid prefix must deliver only its scoped rejection"),
                    },
                )
                .is_err()
        );
        assert_eq!(rejected, 1);
    }
}
