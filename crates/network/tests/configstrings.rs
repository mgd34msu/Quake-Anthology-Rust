use qa_network::{
    commands::packet::Protocol,
    outputs::{self, ConfigCommandReader, Error},
};

#[test]
fn connected_q2_channels_deliver_config_rows_and_continue_the_service_stream() {
    use qa_core::{loopback::Endpoint, sys_events::EventTime};
    use qa_network::{
        channel::{Channel, Delivery},
        commands::connection::Commands,
        ingress::Incoming,
    };
    for protocol in [Protocol::Quake2_34, Protocol::Quake2Repro1038] {
        let mut server = Channel::load(protocol.channel(), Endpoint::Server, 8192, 4).unwrap();
        let mut client = Channel::load(protocol.channel(), Endpoint::Client, 8192, 4).unwrap();
        let mut commands = Commands::load(protocol);
        let mut payload = [0; 64];
        let mut count = outputs::configstring(protocol, 62, b"model\xff", &mut payload).unwrap();
        count += outputs::configstring(protocol, 63, b"", &mut payload[count..]).unwrap();
        count += outputs::print(
            protocol,
            qa_core::primitives::PrintKind::Console,
            2,
            b"next",
            &mut payload[count..],
        )
        .unwrap();
        server.queue_reliable(&payload[..count]).unwrap();
        let packet = server
            .prepare(None, EventTime(1))
            .unwrap()
            .unwrap()
            .bytes
            .to_vec();
        server.submitted(EventTime(1)).unwrap();
        let received = client.receive(&packet, EventTime(2)).unwrap();
        let Delivery::Payload(bytes) = received.delivery else {
            panic!("payload")
        };
        let count = commands.stage(bytes).unwrap();
        let mut rows = Vec::new();
        let mut prints = Vec::new();
        commands
            .decode_output(
                count,
                received.header.sequence,
                &mut client,
                qa_core::primitives::ResourceNamespace::Client(qa_core::primitives::ClientId(0)),
                |incoming| match incoming {
                    Incoming::ConfigString(config) => {
                        rows.push((config.index, config.text.to_vec()))
                    }
                    Incoming::Print(print) => prints.push(print.text.to_vec()),
                    _ => {}
                },
            )
            .unwrap();
        assert_eq!(rows, [(62, b"model\xff".to_vec()), (63, vec![])]);
        assert_eq!(prints, [b"next".to_vec()]);
    }
}

#[test]
fn q2_live_configstrings_keep_u16_index_raw_bytes_and_message_boundaries() {
    for protocol in [Protocol::Quake2_34, Protocol::Quake2Repro1038] {
        for index in [0, 32, 62, 12447, u32::from(u16::MAX)] {
            let mut bytes = [0xcc; 24];
            let n = outputs::configstring(protocol, index, b"models/\x80\xff", &mut bytes).unwrap();
            assert_eq!(bytes[0], 13);
            assert_eq!(&bytes[1..3], &(index as u16).to_le_bytes());
            assert_eq!(&bytes[3..n], b"models/\x80\xff\0");
            assert_eq!(bytes[n], 0xcc);
            let (config, used) = outputs::read_configstring(protocol, &bytes[..n + 1]).unwrap();
            assert_eq!(
                (config.index, config.text, used),
                (index, &b"models/\x80\xff"[..], n)
            );
            assert_eq!(
                outputs::read_configstring(protocol, &bytes[..n - 1]),
                Err(Error::Truncated)
            );
        }
        let mut short = [0xcc; 4];
        assert!(outputs::configstring(protocol, 32, b"x", &mut short).is_err());
        assert_eq!(short, [0xcc; 4]);
        assert!(outputs::configstring(protocol, 65536, b"", &mut short).is_err());
        assert_eq!(short, [0xcc; 4]);
    }
    for protocol in [Protocol::NetQuake15, Protocol::QuakeWorld28] {
        assert_eq!(
            outputs::configstring(protocol, 0, b"", &mut [0; 8]),
            Err(Error::Unsupported)
        );
        assert_eq!(
            outputs::read_configstring(protocol, b"\r\0\0\0"),
            Err(Error::Unsupported)
        );
    }
}

#[test]
fn q3_config_chunks_follow_native_999_byte_boundaries_and_reassemble() {
    for size in [0, 1, 998, 999, 1000, 1998, 1999, 5000] {
        let text = (0..size)
            .map(|i| if i % 17 == 0 { 0xff } else { b'x' })
            .collect::<Vec<_>>();
        let mut reader = ConfigCommandReader::load();
        let mut bytes = [0xcc; 1024];
        let mut sent = 0;
        let mut fragments = 0;
        loop {
            let chunk = outputs::config_command(32, &text, sent, &mut bytes).unwrap();
            assert!(chunk.length <= 1024);
            let record = reader.read(&bytes[..chunk.length]).unwrap();
            if chunk.complete {
                let config = record.unwrap();
                assert_eq!(config.index, 32);
                assert_eq!(config.text, &text);
                break;
            }
            assert!(record.is_none());
            assert_eq!(chunk.next - sent, 999);
            assert!(bytes.starts_with(if fragments == 0 { b"bcs0" } else { b"bcs1" }));
            sent = chunk.next;
            fragments += 1;
        }
        assert_eq!(fragments, if size < 1000 { 0 } else { (size - 1) / 999 });
    }
}

#[test]
fn q3_config_reader_rejects_bad_sequence_and_bounds_without_partial_row() {
    let mut reader = ConfigCommandReader::load();
    assert!(reader.read(b"print \"x\"").unwrap().is_none());
    assert_eq!(reader.read(b"bcs1 32 \"x\""), Err(Error::Opcode));
    assert!(reader.read(b"bcs0 32 \"abc\"").unwrap().is_none());
    assert_eq!(reader.read(b"bcs2 33 \"x\""), Err(Error::Opcode));
    assert_eq!(reader.read(b"bcs2 32 \"x\""), Err(Error::Opcode));
    assert_eq!(reader.read(b"cs 1024 \"x\""), Err(Error::Opcode));
    assert_eq!(reader.read(b"cs 32 \"x"), Err(Error::Truncated));
    let (config, _) = outputs::read_configstring(Protocol::Quake2_34, b"\r\x20\0\0").unwrap();
    assert_eq!(config.text, b"");
    let config = reader.read(b"cs 32 \"\"\n").unwrap().unwrap();
    assert_eq!(config.text, b"");
    let mut bytes = [0; 1024];
    let text = vec![b'x'; 8191];
    assert!(outputs::config_command(32, &text, 0, &mut bytes).is_err());
    let mut malicious = b"bcs0 32 \"".to_vec();
    malicious.extend_from_slice(&text);
    malicious.push(b'"');
    assert!(reader.read(&malicious).is_err());
    assert_eq!(reader.read(b"bcs2 32 \"x\""), Err(Error::Opcode));
}
