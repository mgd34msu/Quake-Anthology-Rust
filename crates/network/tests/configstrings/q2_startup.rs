use qa_core::{
    loopback::Endpoint,
    primitives::{ClientId, ResourceNamespace, ThinkTime, UserCmd},
    sys_events::EventTime,
};
use qa_network::{
    channel::{Channel, Delivery},
    commands::{
        connection::Commands,
        packet::{Error, Protocol},
        startup::{Q2ReproServerData, ResourceStream},
    },
    ingress::Incoming,
    message::{Encoding, Writer},
    outputs,
    snapshots::{self, Entity, Frame, Q2EntityPolicy, Q2ReproRing, Q2Ring, ReceivedFrame},
    states,
};
use std::ops::ControlFlow;

fn channel(protocol: Protocol, endpoint: Endpoint) -> Channel {
    let mut channel = Channel::load(protocol.channel(), endpoint, 8192, 8).unwrap();
    channel.configure_snapshots(protocol).unwrap();
    channel
}

fn serverdata(protocol: Protocol, client: i16, attract: bool, flags: u16, fps: u8) -> Vec<u8> {
    let mut bytes = [0; 256];
    let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
    writer.write_bits(12, 8).unwrap();
    writer.write_bits(protocol.number(), 32).unwrap();
    writer.write_bits(i32::MIN as u32, 32).unwrap();
    writer.write_bits(u32::from(attract), 8).unwrap();
    writer
        .write_string(protocol.string_policy(), b"baseq2")
        .unwrap();
    writer.write_bits(u32::from(client as u16), 16).unwrap();
    writer
        .write_string(protocol.string_policy(), b"level\x80")
        .unwrap();
    if protocol == Protocol::Quake2Repro1038 {
        writer.write_bits(1024, 16).unwrap();
        writer.write_bits(2, 8).unwrap();
        writer.write_bits(u32::from(flags), 16).unwrap();
        writer.write_bits(u32::from(fps), 8).unwrap();
    }
    writer.bytes().to_vec()
}

fn apply(
    commands: &mut Commands,
    channel: &mut Channel,
    bytes: &[u8],
    consume: impl FnMut(Incoming<'_>),
) -> Result<Option<u32>, Error> {
    let length = commands.stage(bytes)?;
    commands.decode_output(
        length,
        1,
        channel,
        ResourceNamespace::Client(ClientId(0)),
        consume,
    )
}

fn baseline(protocol: Protocol, stream: bool, model: u32) -> Vec<u8> {
    let mut bytes = [0; 256];
    let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
    if !stream {
        writer.write_bits(14, 8).unwrap();
    }
    if protocol == Protocol::Quake2_34 {
        let mut words = [0; states::Q2_ENTITY_WORDS];
        words[0] = model;
        words[4] = 300;
        words[8] = 64f32.to_bits();
        states::write_q2_entity(
            &mut writer,
            7,
            &[0; states::Q2_ENTITY_WORDS],
            Some(&words),
            true,
            true,
        )
        .unwrap();
    } else {
        let mut words = [0; states::Q2_RERELEASE_ENTITY_WORDS];
        words[0] = model;
        words[4] = 300;
        words[8] = 64f32.to_bits();
        states::write_q2_repro_entity(
            &mut writer,
            7,
            &[0; states::Q2_RERELEASE_ENTITY_WORDS],
            Some(&words),
            true,
        )
        .unwrap();
    }
    writer.bytes().to_vec()
}

fn frame(protocol: Protocol, model: u32) -> Vec<u8> {
    let mut bytes = [0; 1024];
    let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
    if protocol == Protocol::Quake2_34 {
        let mut ring = Q2Ring::load(8, 1024, 32, None).unwrap();
        let mut words = [0; states::Q2_ENTITY_WORDS];
        words[0] = model;
        words[4] = 300;
        words[8] = 64f32.to_bits();
        assert!(ring.set_baseline(7, &words));
        ring.store(Frame {
            sequence: 1,
            time: ThinkTime::Milliseconds(100),
            command: 0,
            flags: 0,
            qw_peers: 0,
            qw_dead: 0,
            areas: &[],
            player: &[0; states::Q2_PLAYER_WORDS],
            entities: &[Entity { number: 7, words }],
        })
        .unwrap();
        snapshots::write_q2(&mut writer, &ring, 1, None, 0).unwrap();
    } else {
        let mut ring = Q2ReproRing::load(8, 8192, 255, None).unwrap();
        let mut words = [0; states::Q2_RERELEASE_ENTITY_WORDS];
        words[0] = model;
        words[4] = 300;
        words[8] = 64f32.to_bits();
        assert!(ring.set_baseline(7, &words));
        ring.store(Frame {
            sequence: 1,
            time: ThinkTime::Milliseconds(25),
            command: 0,
            flags: 0,
            qw_peers: 0,
            qw_dead: 0,
            areas: &[],
            player: &[0; states::Q2_REPRO_PLAYER_WORDS],
            entities: &[Entity { number: 7, words }],
        })
        .unwrap();
        snapshots::write_q2_repro(&mut writer, &mut ring, 1, None, Q2EntityPolicy::default())
            .unwrap();
    }
    writer.bytes().to_vec()
}

fn assert_frame(channel: &Channel, expected: u32, milliseconds: i64) {
    let frame = channel.current_snapshot().unwrap();
    assert_eq!(frame.time(), ThinkTime::Milliseconds(milliseconds));
    let words = match frame {
        ReceivedFrame::Quake2(frame) => &frame.entities[0].words[..],
        ReceivedFrame::Quake2Repro(frame) => &frame.entities[0].words[..],
        _ => panic!("Q2 snapshot"),
    };
    assert_eq!(words[0], expected);
    assert_eq!(words[4], 300);
    assert_eq!(f32::from_bits(words[8]), 64.0);
}

#[test]
fn complete_headers_keep_signed_native_fields_and_service_order() {
    for protocol in [Protocol::Quake2_34, Protocol::Quake2Repro1038] {
        let mut commands = Commands::load(protocol, 8192);
        let mut channel = channel(protocol, Endpoint::Client);
        let mut bytes = serverdata(protocol, -1, true, 0x8008, 59);
        let header_len = bytes.len();
        bytes.extend_from_slice(b"\x0bcmd configstrings -2147483648 0\n\0");
        let mut row = [0; 64];
        let length = outputs::configstring(protocol, 0, b"title", &mut row).unwrap();
        bytes.extend_from_slice(&row[..length]);
        bytes.extend_from_slice(b"\x08\x0a\x02after\0");
        let mut order = Vec::new();
        apply(
            &mut commands,
            &mut channel,
            &bytes,
            |incoming| match incoming {
                Incoming::Q2ServerData(data) => {
                    assert_eq!(data.server_count, i32::MIN);
                    assert!(data.attract_loop);
                    assert_eq!(data.client_number, -1);
                    assert_eq!(data.game_directory, b"baseq2");
                    assert_eq!(data.level_name, b"level\x80");
                    assert_eq!(
                        data.level_name.as_ptr() as usize,
                        data.game_directory.as_ptr() as usize + 9
                    );
                    if protocol == Protocol::Quake2Repro1038 {
                        assert_eq!(
                            data.rerelease,
                            Some(Q2ReproServerData {
                                revision: 1024,
                                server_state: 2,
                                flags: 0x8008,
                                server_fps: 59
                            })
                        );
                        assert_eq!(data.resource_layout().count, 13630);
                        assert_eq!(data.frame_milliseconds(), 16);
                    } else {
                        assert!(data.rerelease.is_none());
                        assert_eq!(data.frame_milliseconds(), 100);
                    }
                    order.push("header");
                }
                Incoming::StuffText(text) => {
                    assert_eq!(text, b"cmd configstrings -2147483648 0\n");
                    order.push("stuff");
                }
                Incoming::ConfigString(row) => {
                    assert_eq!((row.index, row.text), (0, b"title".as_slice()));
                    order.push("config");
                }
                Incoming::Reconnect => order.push("reconnect"),
                Incoming::Print(print) => {
                    assert_eq!(print.text, b"after");
                    order.push("print");
                }
                _ => panic!("startup service"),
            },
        )
        .unwrap();
        assert!(header_len < bytes.len());
        assert_eq!(order, ["header", "stuff", "config", "reconnect", "print"]);
    }
}

#[test]
fn cinematic_header_preserves_player_field_and_native_rr_server_state() {
    // q2repro server.h: ss_game2, ss_pic3, ss_broadcast4, ss_cinematic5.
    for (protocol, client, state, expected) in [
        (Protocol::Quake2_34, -1, 2, true),
        (Protocol::Quake2_34, 0, 2, false),
        (Protocol::Quake2Repro1038, -1, 2, true),
        (Protocol::Quake2Repro1038, 7, 3, true),
        (Protocol::Quake2Repro1038, 7, 5, true),
        (Protocol::Quake2Repro1038, 7, 2, false),
        (Protocol::Quake2Repro1038, 7, 4, false),
    ] {
        let mut bytes = serverdata(protocol, client, false, 0, 40);
        if protocol == Protocol::Quake2Repro1038 {
            // Tail is revision:u16, state:u8, flags:u16, FPS:u8.
            let at = bytes.len() - 4;
            bytes[at] = state;
        }
        let mut commands = Commands::load(protocol, 8192);
        let mut channel = channel(protocol, Endpoint::Client);
        let mut delivered = 0;
        apply(&mut commands, &mut channel, &bytes, |incoming| {
            let Incoming::Q2ServerData(data) = incoming else {
                panic!("serverdata")
            };
            assert_eq!(data.client_number, client);
            assert_eq!(data.cinematic(), expected);
            if let Some(repro) = data.rerelease {
                assert_eq!(repro.server_state, state);
            }
            delivered += 1;
        })
        .unwrap();
        assert_eq!(delivered, 1);
    }
}

#[test]
fn rr_streams_share_row_and_baseline_readers_and_terminate_before_following_services() {
    let protocol = Protocol::Quake2Repro1038;
    for (opcode, flags, limit) in [(36, 0, 12448u16), (38, 0x8000, 2080), (38, 0x8010, 13630)] {
        let mut commands = Commands::load(protocol, 8192);
        let mut channel = channel(protocol, Endpoint::Client);
        let mut bytes = serverdata(protocol, 255, false, flags, 59);
        bytes.push(opcode);
        bytes.extend_from_slice(&33u16.to_le_bytes());
        bytes.extend_from_slice(b"maps/test.bsp\0");
        bytes.extend_from_slice(&limit.to_le_bytes());
        if opcode == 38 {
            bytes.push(39);
        }
        bytes.extend_from_slice(&baseline(protocol, true, 7));
        bytes.extend_from_slice(&[0, 0]);
        bytes.extend_from_slice(b"\x0bprecache -2147483648\n\0");
        bytes.extend_from_slice(&frame(protocol, 7));
        let mut order = Vec::new();
        apply(
            &mut commands,
            &mut channel,
            &bytes,
            |incoming| match incoming {
                Incoming::Q2ServerData(data) => {
                    assert_eq!(data.client_number, 255);
                    assert_eq!(data.resource_layout().count, usize::from(limit));
                    order.push("header");
                }
                Incoming::ConfigString(row) => {
                    assert_eq!((row.index, row.text), (33, b"maps/test.bsp".as_slice()));
                    order.push("config");
                }
                Incoming::ResourceStream(ResourceStream::ConfigStrings) => {
                    order.push("configs_done")
                }
                Incoming::ResourceStream(ResourceStream::Baselines) => order.push("baselines_done"),
                Incoming::StuffText(text) => {
                    assert_eq!(text, b"precache -2147483648\n");
                    order.push("precache");
                }
                _ => panic!("stream service"),
            },
        )
        .unwrap();
        assert_eq!(
            order,
            [
                "header",
                "config",
                "configs_done",
                "baselines_done",
                "precache"
            ]
        );
        assert_frame(&channel, 7, 20);
    }
    // Prefix rows survive, but no completion is published before its terminator.
    let mut commands = Commands::load(protocol, 8192);
    let mut channel = channel(protocol, Endpoint::Client);
    let bytes = b"\x26\x01\0first\0\xa0";
    let mut rows = 0;
    assert!(
        apply(&mut commands, &mut channel, bytes, |incoming| {
            let Incoming::ConfigString(row) = incoming else {
                panic!("unfinished stream completion")
            };
            assert_eq!(row.text, b"first");
            rows += 1;
        })
        .is_err()
    );
    assert_eq!(rows, 1);
}

#[test]
fn rr_failed_raw_stream_releases_framing_without_retracting_accepted_prefixes() {
    let protocol = Protocol::Quake2Repro1038;
    let mut commands = Commands::load(protocol, 8192);
    let mut channel = channel(protocol, Endpoint::Client);
    let mut seen = Vec::new();
    for broken in [
        b"\x26\x01\0first\0\x02\0unterminated".as_slice(),
        b"\x26\xff\xff",
    ] {
        assert!(
            apply(&mut commands, &mut channel, broken, |incoming| {
                let Incoming::ConfigString(row) = incoming else {
                    panic!("no completion for a malformed stream")
                };
                seen.push((row.index, row.text.to_vec()));
            })
            .is_err()
        );
        let mut prints = 0;
        apply(
            &mut commands,
            &mut channel,
            b"\x0a\x02recovered\0",
            |incoming| {
                let Incoming::Print(print) = incoming else {
                    panic!("next packet starts with an ordinary service")
                };
                assert_eq!(print.text, b"recovered");
                prints += 1;
            },
        )
        .unwrap();
        assert_eq!(prints, 1);
    }
    assert_eq!(seen, [(1, b"first".to_vec())]);

    // A valid stream may pause between complete records and retain its mode.
    apply(
        &mut commands,
        &mut channel,
        b"\x26\x02\0second\0",
        |incoming| {
            let Incoming::ConfigString(row) = incoming else {
                panic!("complete row")
            };
            seen.push((row.index, row.text.to_vec()));
        },
    )
    .unwrap();
    let mut completions = 0;
    apply(
        &mut commands,
        &mut channel,
        b"\x03\0third\0\xa0\x30",
        |incoming| match incoming {
            Incoming::ConfigString(row) => seen.push((row.index, row.text.to_vec())),
            Incoming::ResourceStream(ResourceStream::ConfigStrings) => completions += 1,
            _ => panic!("raw continuation"),
        },
    )
    .unwrap();
    assert_eq!(completions, 1);
    assert_eq!(
        seen,
        [
            (1, b"first".to_vec()),
            (2, b"second".to_vec()),
            (3, b"third".to_vec())
        ]
    );

    let mut broken = vec![39];
    broken.extend_from_slice(&baseline(protocol, true, 7));
    let mut unfinished = baseline(protocol, true, 9);
    unfinished.pop();
    broken.extend_from_slice(&unfinished);
    assert!(
        apply(&mut commands, &mut channel, &broken, |_| panic!(
            "no baseline completion"
        ))
        .is_err()
    );
    // The complete baseline7 survives; the truncated replacement9 never commits.
    apply(&mut commands, &mut channel, &frame(protocol, 7), |_| {
        panic!("snapshot returned at packet completion")
    })
    .unwrap();
    assert_frame(&channel, 7, 25);
}

#[test]
fn malformed_headers_cannot_reset_history_snapshots_or_native_flight() {
    for protocol in [Protocol::Quake2_34, Protocol::Quake2Repro1038] {
        let mut commands = Commands::load(protocol, 8192);
        commands.configure_history(4).unwrap();
        commands.record(UserCmd::default()).unwrap();
        let mut client = channel(protocol, Endpoint::Client);
        let initial = serverdata(protocol, 0, false, 0, 40);
        apply(&mut commands, &mut client, &initial, |incoming| {
            assert!(matches!(incoming, Incoming::Q2ServerData(_)))
        })
        .unwrap();
        commands.record(UserCmd::default()).unwrap();
        apply(
            &mut commands,
            &mut client,
            &baseline(protocol, false, 7),
            |_| panic!("baseline is decoder state"),
        )
        .unwrap();
        apply(&mut commands, &mut client, &frame(protocol, 7), |_| {
            panic!("snapshot returned at packet completion")
        })
        .unwrap();
        assert_frame(
            &client,
            7,
            if protocol == Protocol::Quake2_34 {
                100
            } else {
                25
            },
        );
        let receipt = client.queue_client_command(protocol, b"retained").unwrap();
        let flight = client
            .prepare(None, EventTime(1))
            .unwrap()
            .unwrap()
            .bytes
            .to_vec();
        client.submitted(EventTime(1)).unwrap();
        client.queue_client_command(protocol, b"nextdl").unwrap();
        let transport = (client.state(), client.send_state());
        let replacement = serverdata(protocol, -1, false, 0, 59);
        let mut malformed = Vec::new();
        for cut in 1..replacement.len() {
            malformed.push(replacement[..cut].to_vec());
        }
        let mut wrong = replacement.clone();
        wrong[1..5].copy_from_slice(&999u32.to_le_bytes());
        malformed.push(wrong);
        malformed.push(serverdata(protocol, -2, false, 0, 40));
        malformed.push(serverdata(protocol, 256, false, 0, 40));
        for bytes in malformed {
            assert!(
                apply(&mut commands, &mut client, &bytes, |_| panic!(
                    "unfinished header"
                ))
                .is_err()
            );
            assert!(client.current_snapshot().is_some());
            assert_eq!(commands.history().unwrap().len(), 1);
            assert_eq!((client.state(), client.send_state()), transport);
        }
        apply(&mut commands, &mut client, &replacement, |incoming| {
            assert!(matches!(incoming, Incoming::Q2ServerData(_)))
        })
        .unwrap();
        assert!(client.current_snapshot().is_none());
        assert_eq!(
            (
                commands.history().unwrap().number(),
                commands.history().unwrap().len()
            ),
            (2, 0)
        );
        assert_eq!((client.state(), client.send_state()), transport);
        assert_eq!(commands.record(UserCmd::default()).unwrap(), 3);

        let mut server = channel(protocol, Endpoint::Server);
        let received = server.receive(&flight, EventTime(2)).unwrap();
        let sequence = received.header.sequence;
        let Delivery::Payload(body) = received.delivery else {
            panic!("reliable CLIENT command")
        };
        let mut server_commands = Commands::load(protocol, 8192);
        let length = server_commands.stage(body).unwrap();
        let mut delivered = 0;
        server_commands
            .decode(length, sequence, 0, &mut server, None, |incoming| {
                let Incoming::CommandString { text, .. } = incoming else {
                    panic!("CLIENT command")
                };
                assert_eq!(text, b"retained");
                delivered += 1;
                ControlFlow::Continue(())
            })
            .unwrap();
        assert_eq!(delivered, 1);
        let ack = server
            .prepare(Some(&[]), EventTime(3))
            .unwrap()
            .unwrap()
            .bytes
            .to_vec();
        server.submitted(EventTime(3)).unwrap();
        client.receive(&ack, EventTime(4)).unwrap();
        assert_eq!(client.reliable_receipts(), &[receipt]);
        assert!(
            !client.has_output(),
            "old unsent nextdl was canceled, submitted flight waited for real ACK"
        );
    }
}
