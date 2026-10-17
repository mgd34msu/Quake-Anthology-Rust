use qa_core::loopback::Endpoint;
use qa_core::primitives::{ClientId, PrintKind, ResourceNamespace};
use qa_network::{
    channel::Channel,
    commands::{
        connection::{Commands, OutputError},
        packet::Protocol,
    },
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
                Chunk::Sequential {
                    compressed: false,
                    percent: 40,
                    bytes: Some(&[0, 128, 255]),
                },
                vec![opcode, 3, 0, 40, 0, 128, 255],
            ),
            (
                Chunk::Sequential {
                    compressed: false,
                    percent: 0,
                    bytes: None,
                },
                vec![opcode, 255, 255, 0],
            ),
            (
                Chunk::Sequential {
                    compressed: false,
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
            if let Chunk::Sequential {
                bytes: Some(data), ..
            } = decoded
            {
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
            Chunk::Sequential {
                compressed: false,
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
                Chunk::Sequential {
                    compressed: false,
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
    assert_eq!(
        chunk,
        Chunk::Sequential {
            percent: 99,
            bytes: Some(data.as_slice()),
            compressed: false
        }
    );
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
            Chunk::Sequential {
                compressed: false,
                percent: 35,
                bytes: Some(b"first"),
            },
            Chunk::Sequential {
                compressed: false,
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
                    Incoming::Download(Ok(Chunk::Sequential { percent, bytes, .. })) => {
                        received.push((percent, bytes.unwrap().to_vec()))
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
        assert_eq!(
            commands.decode_output(
                staged,
                2,
                &mut channel,
                ResourceNamespace::Client(ClientId(0)),
                |event| match event {
                    Incoming::Download(Err(Error::Size)) => rejected += 1,
                    _ => panic!("invalid prefix must deliver only its scoped rejection"),
                },
            ),
            Err(OutputError::MalformedDownload(Error::Size))
        );
        assert_eq!(rejected, 1);
    }
}

#[test]
fn shared_download_paths_and_categories_keep_native_content_boundaries() {
    use downloads::AssetCategory::*;
    for name in [
        b"skins/name.pcx".as_slice(),
        b"models/a+b_1-2.md2",
        b"maps/name.bsp",
    ] {
        assert_eq!(downloads::validate_path(name), Ok(()));
    }
    for name in [
        b"/maps/a".as_slice(),
        b"maps/../a",
        b"maps/a..b",
        b"maps//a",
        b"maps/./a",
        b"maps/a/",
        b"maps\\a",
        b"maps/a:b",
        b"maps/a\0b",
        b"a",
        b".maps/a",
    ] {
        assert_eq!(downloads::validate_path(name), Err(Error::Path));
    }
    let mut longest = [b'a'; 64];
    longest[4] = b'/';
    assert_eq!(downloads::validate_path(&longest[..63]), Ok(()));
    assert_eq!(downloads::validate_path(&longest), Err(Error::Path));
    for protocol in SEQUENTIAL {
        assert_eq!(downloads::category(protocol, b"maps/a.pk3"), Package);
        assert_eq!(downloads::content_category(protocol, b"maps/a.pk3"), Map);
        for (name, expected) in [
            (b"maps/a.bsp".as_slice(), Map),
            (b"env/a.tga", Texture),
            (b"textures/a.wal", Texture),
            (b"skins/a.pcx", Skin),
            (b"players/a/b.pcx", Player),
            (b"progs/a.mdl", Model),
            (b"models/a.md2", Model),
            (b"sound/a.wav", Sound),
            (b"pics/a.pcx", Picture),
            (b"other/a.txt", Other),
            (b"packs/a.pk3", Package),
        ] {
            assert_eq!(downloads::category(protocol, name), expected);
        }
    }
    assert_eq!(
        downloads::category(Protocol::Quake2Repro1038, b"sprites/a.sp2"),
        Model
    );
    assert_eq!(
        downloads::category(Protocol::Quake2_34, b"sprites/a.sp2"),
        Other
    );
}

#[test]
fn q3_downloads_share_huffman_commands_fragmentation_and_service_order() {
    use qa_core::sys_events::EventTime;
    use qa_network::channel::Delivery;
    let protocol = Protocol::Quake3_68;
    let mut client = Channel::load(
        protocol.channel(),
        Endpoint::Client,
        protocol.message_bytes(),
        16,
    )
    .unwrap();
    let mut server = Channel::load(
        protocol.channel(),
        Endpoint::Server,
        protocol.message_bytes(),
        16,
    )
    .unwrap();
    for channel in [&mut client, &mut server] {
        channel.configure_snapshots(protocol).unwrap();
        channel
            .set_command_context(qa_network::channel::commands::CommandContext {
                challenge: 0x12345678,
                ..Default::default()
            })
            .unwrap();
    }
    let mut commands = Commands::load(protocol, protocol.message_bytes());
    server.queue_reliable(b"print before").unwrap();
    let binary: Vec<u8> = (0..6144).map(|index| (index * 17) as u8).collect();
    let mut payload = vec![0; protocol.message_bytes()];
    let size = server
        .encode_server_output(&mut payload, |writer, _| {
            // Native SV_WriteDownloadToClient layout, independent of write_service.
            for (number, data) in [(0, binary.as_slice()), (1, &[])] {
                writer.write_bits(6, 8)?;
                writer.write_bits(number, 16)?;
                if number == 0 {
                    writer.write_bits(binary.len() as u32, 32)?;
                }
                writer.write_bits(data.len() as u32, 16)?;
                writer.write_data(data)?;
            }
            Ok(())
        })
        .unwrap();
    server
        .prepare_server_message(&payload[..size], EventTime(1))
        .unwrap()
        .unwrap();
    let mut packets = 0;
    let mut received = Vec::new();
    loop {
        let wire = server.pending_packet().unwrap().bytes.to_vec();
        server.submitted(EventTime(1)).unwrap();
        packets += 1;
        let result = client.receive(&wire, EventTime(1)).unwrap();
        if let Delivery::Payload(body) = result.delivery {
            let staged = commands.stage(body).unwrap();
            commands
                .decode_output(
                    staged,
                    result.header.sequence,
                    &mut client,
                    ResourceNamespace::Client(ClientId(0)),
                    |incoming| match incoming {
                        Incoming::ReliableCommand { sequence, text } => {
                            assert_eq!((sequence, text), (1, b"print before".as_slice()));
                            assert!(received.is_empty());
                        }
                        Incoming::Download(Ok(Chunk::Block {
                            number,
                            total,
                            bytes,
                        })) => {
                            assert_eq!(number as usize, received.len());
                            assert_eq!(total, (number == 0).then_some(binary.len() as u32));
                            received.push(bytes.to_vec());
                        }
                        _ => panic!("unexpected Q3 service"),
                    },
                )
                .unwrap();
        }
        if !server.pending_fragments() {
            break;
        }
        server.prepare_output(EventTime(1)).unwrap().unwrap();
    }
    assert!(packets > 1);
    assert_eq!(received, [binary, vec![]]);
}

#[test]
fn q3_download_rejections_sizes_and_following_services_keep_the_native_boundary() {
    use qa_network::channel::commands::ServerOutput;
    let protocol = Protocol::Quake3_68;
    let data = [0, 255, 128, 1];
    for chunk in [
        Chunk::Block {
            number: 0,
            total: Some(4),
            bytes: &data,
        },
        Chunk::Block {
            number: i16::MIN,
            total: None,
            bytes: &data,
        },
        Chunk::Block {
            number: 17,
            total: None,
            bytes: &[],
        },
        Chunk::Rejected {
            size: -1,
            message: b"disabled by server",
        },
    ] {
        let mut client = Channel::load(
            protocol.channel(),
            Endpoint::Client,
            protocol.message_bytes(),
            8,
        )
        .unwrap();
        let mut server = Channel::load(
            protocol.channel(),
            Endpoint::Server,
            protocol.message_bytes(),
            8,
        )
        .unwrap();
        let mut wire = vec![0; protocol.message_bytes()];
        let n = server
            .encode_server_output(&mut wire, |writer, _| {
                downloads::write_service(protocol, chunk, writer).unwrap();
                writer.write_bits(1, 8)?; // following svc_nop
                Ok(())
            })
            .unwrap();
        let mut decoded = 0;
        client
            .decode_server_output(&mut wire[..n], 1, |output| match output {
                ServerOutput::Download(Ok(actual)) => {
                    assert_eq!(actual, chunk);
                    decoded += 1;
                }
                _ => panic!("download must remain in the shared MSG stream"),
            })
            .unwrap();
        assert_eq!(decoded, 1);
    }
    for length in [-1i16, 16385, 32767, 2] {
        let mut client = Channel::load(
            protocol.channel(),
            Endpoint::Client,
            protocol.message_bytes(),
            8,
        )
        .unwrap();
        let mut server = Channel::load(
            protocol.channel(),
            Endpoint::Server,
            protocol.message_bytes(),
            8,
        )
        .unwrap();
        let mut wire = [0; 128];
        let n = server
            .encode_server_output(&mut wire, |writer, _| {
                writer.write_bits(6, 8)?;
                writer.write_bits(1, 16)?;
                writer.write_bits(u32::from(length as u16), 16)?;
                // Positive two-byte size is truncated, without an outer EOF byte.
                Ok(())
            })
            .unwrap();
        let mut rejected = 0;
        assert!(
            client
                .decode_server_output(&mut wire[..n], 1, |output| match output {
                    ServerOutput::Download(Err(_)) => rejected += 1,
                    _ => panic!("malformed chunk cannot reach another service"),
                })
                .is_err()
        );
        assert_eq!(rejected, 1);
    }
}
