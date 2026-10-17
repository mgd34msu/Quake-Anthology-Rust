use qa_core::{
    loopback::Endpoint,
    primitives::{ClientId, MovementMagnitude, ResourceNamespace, UserCmd, Vec3},
    sys_events::EventTime,
};
use qa_network::{
    channel::Channel,
    commands::{
        connection::{Commands, OutputError},
        packet::{Protocol, ZERO_QW},
    },
    ingress::Incoming,
    message::{Encoding, Writer},
    outputs::ResourceListProgress,
    states,
};

const MOVEVARS: [f32; 10] = [800.0, 100.0, 320.0, 500.0, 10.0, 0.75, 5.5, 4.0, 1.25, -0.0];

fn serverdata(version: u32, slot: u8, directory: &[u8], values: [f32; 10]) -> Vec<u8> {
    let mut bytes = [0; 256];
    let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
    writer.write_bits(11, 8).unwrap();
    writer.write_bits(version, 32).unwrap();
    writer.write_bits(i32::MIN as u32, 32).unwrap();
    writer
        .write_string(Protocol::QuakeWorld28.string_policy(), directory)
        .unwrap();
    writer.write_bits(u32::from(slot), 8).unwrap();
    writer
        .write_string(Protocol::QuakeWorld28.string_policy(), b"level\x80")
        .unwrap();
    for value in values {
        writer.write_bits(value.to_bits(), 32).unwrap();
    }
    writer.bytes().to_vec()
}

fn qw_channel(endpoint: Endpoint) -> Channel {
    let protocol = Protocol::QuakeWorld28;
    let mut channel = Channel::load(protocol.channel(), endpoint, 8192, 4).unwrap();
    channel.configure_snapshots(protocol).unwrap();
    channel
}

fn accept_request(server: &mut Channel, packet: &[u8], expected: &[u8]) {
    let received = server.receive(packet, EventTime(3)).unwrap();
    let sequence = received.header.sequence;
    let qa_network::channel::Delivery::Payload(body) = received.delivery else {
        panic!("native CLIENT request");
    };
    assert_eq!(body.first(), Some(&4));
    assert_eq!(body.last(), Some(&0));
    assert_eq!(&body[1..body.len() - 1], expected);
    let mut commands = Commands::load(Protocol::QuakeWorld28, 8192);
    let length = commands.stage(body).unwrap();
    let mut delivered = 0;
    commands
        .decode(length, sequence, 0, server, None, |incoming| {
            let Incoming::CommandString { protocol, text, .. } = incoming else {
                panic!("native string command");
            };
            assert_eq!(protocol, Protocol::QuakeWorld28);
            assert_eq!(text, expected);
            delivered += 1;
            std::ops::ControlFlow::Continue(())
        })
        .unwrap();
    assert_eq!(delivered, 1);
}

fn apply(
    commands: &mut Commands,
    channel: &mut Channel,
    bytes: &[u8],
    sequence: u32,
    consume: impl FnMut(Incoming<'_>),
) -> Result<Option<u32>, OutputError> {
    let length = commands.stage(bytes)?;
    commands.decode_output(
        length,
        sequence,
        channel,
        ResourceNamespace::Client(ClientId(0)),
        consume,
    )
}

#[test]
fn qw_startup_borrows_native_header_and_reports_list_control_after_its_rows() {
    let protocol = Protocol::QuakeWorld28;
    let mut commands = Commands::load(protocol, 8192);
    let mut channel = qw_channel(Endpoint::Client);
    let mut bytes = serverdata(28, 128 | 31, b"qw", MOVEVARS);
    // Native bundle: CD, fullserverinfo stufftext, paged resources, then print.
    bytes.extend_from_slice(b"\x20\x02\x09fullserverinfo \"\\hostname\\server\"\n\0\x2e\0a.wav\0b.wav\0\0\x02\x2d\x02progs/player.mdl\0\0\0\x08\x02after\0");
    bytes.push(49);
    bytes.extend_from_slice(&(-0.0f32).to_bits().to_le_bytes());
    bytes.push(50);
    bytes.extend_from_slice(&0x7fc0_4321u32.to_le_bytes());
    let mut order = Vec::new();
    apply(
        &mut commands,
        &mut channel,
        &bytes,
        1,
        |incoming| match incoming {
            Incoming::QwServerData(data) => {
                order.push("data");
                assert_eq!(
                    (data.server_count, data.player_slot, data.spectator),
                    (i32::MIN, 31, true)
                );
                assert_eq!(data.game_directory, b"qw");
                assert_eq!(data.level_name, b"level\x80");
                assert_eq!(
                    data.level_name.as_ptr() as usize,
                    data.game_directory.as_ptr() as usize + 4
                );
                let MovementMagnitude::Base(gravity) = data.tuning.gravity.unwrap() else {
                    panic!("native base gravity");
                };
                let MovementMagnitude::Base(speed) = data.tuning.max_speed.unwrap() else {
                    panic!("native base speed");
                };
                let actual = [
                    gravity,
                    data.tuning.stop_speed.unwrap(),
                    speed,
                    data.tuning.spectator_max_speed.unwrap(),
                    data.tuning.accelerate.unwrap(),
                    data.tuning.air_accelerate.unwrap(),
                    data.tuning.water_accelerate.unwrap(),
                    data.tuning.friction.unwrap(),
                    data.tuning.water_friction.unwrap(),
                    data.tuning.gravity_multiplier,
                ];
                assert_eq!(actual.map(f32::to_bits), MOVEVARS.map(f32::to_bits));
                assert_eq!(data.tuning.edge_friction, None);
                assert_eq!(data.tuning.speed_multiplier, 1.0);
            }
            Incoming::CdTrack(_) => order.push("cd"),
            Incoming::StuffText(text) => {
                assert_eq!(text, b"fullserverinfo \"\\hostname\\server\"\n");
                order.push("stuff");
            }
            Incoming::ConfigString(row) => {
                assert!(matches!(row.index, 257 | 258 | 3));
                order.push("row");
            }
            Incoming::ResourceList(progress) => {
                if progress.range == protocol.resource_layout().resources[1] {
                    assert_eq!(
                        progress,
                        ResourceListProgress {
                            range: protocol.resource_layout().resources[1],
                            first: 0,
                            end: 2,
                            next: 2
                        }
                    );
                    order.push("sounds");
                } else {
                    assert_eq!(
                        progress,
                        ResourceListProgress {
                            range: protocol.resource_layout().resources[0],
                            first: 2,
                            end: 3,
                            next: 0
                        }
                    );
                    order.push("models");
                }
            }
            Incoming::MaxSpeed(value) => {
                assert_eq!(value.to_bits(), (-0.0f32).to_bits());
                order.push("maxspeed");
            }
            Incoming::EntityGravity(value) => {
                assert_eq!(value.to_bits(), 0x7fc0_4321);
                order.push("entgravity");
            }
            Incoming::Print(print) => {
                assert_eq!(print.text, b"after");
                order.push("print");
            }
            _ => panic!("unexpected startup service"),
        },
    )
    .unwrap();
    assert_eq!(
        order,
        [
            "data",
            "cd",
            "stuff",
            "row",
            "row",
            "sounds",
            "row",
            "models",
            "print",
            "maxspeed",
            "entgravity"
        ]
    );
}

#[test]
fn qw_startup_rejects_incomplete_headers_without_delivering_them_and_keeps_native_prefix_effects() {
    let protocol = Protocol::QuakeWorld28;
    let mut commands = Commands::load(protocol, 8192);
    let mut channel = qw_channel(Endpoint::Client);
    let bytes = serverdata(28, 31, b"qw", MOVEVARS);
    for end in 1..bytes.len() {
        let mut delivered = 0;
        assert!(
            apply(&mut commands, &mut channel, &bytes[..end], 1, |_| {
                delivered += 1
            })
            .is_err(),
            "prefix {end}"
        );
        assert_eq!(delivered, 0);
    }
    for invalid in [
        serverdata(29, 0, b"qw", MOVEVARS),
        serverdata(28, 32, b"qw", MOVEVARS),
        serverdata(28, 128 | 32, b"qw", MOVEVARS),
        serverdata(28, 0, &[b'x'; 128], MOVEVARS),
    ] {
        let mut delivered = 0;
        assert!(apply(&mut commands, &mut channel, &invalid, 1, |_| delivered += 1).is_err());
        assert_eq!(delivered, 0);
    }
    apply(
        &mut commands,
        &mut channel,
        &serverdata(28, 0, &[b'x'; 64], MOVEVARS),
        1,
        |incoming| {
            let Incoming::QwServerData(data) = incoming else {
                panic!("data");
            };
            assert_eq!(data.game_directory, &[b'x'; 64]);
        },
    )
    .unwrap();
    for opcode in [49, 50] {
        for bytes in 1..5 {
            let raw = [opcode, 0, 0, 0, 0];
            assert!(
                apply(&mut commands, &mut channel, &raw[..bytes], 1, |_| panic!(
                    "incomplete movement update"
                ))
                .is_err()
            );
        }
    }
    let mut values = MOVEVARS;
    values[0] = f32::from_bits(0x7fc0_1234);
    apply(
        &mut commands,
        &mut channel,
        &serverdata(28, 0, b"", values),
        1,
        |incoming| {
            let Incoming::QwServerData(data) = incoming else {
                panic!("data");
            };
            let Some(MovementMagnitude::Base(value)) = data.tuning.gravity else {
                panic!("gravity");
            };
            assert_eq!(
                value.to_bits(),
                0x7fc0_1234,
                "native float bits are not an invented range validator"
            );
            assert!(!data.spectator);
        },
    )
    .unwrap();
    let mut order = Vec::new();
    assert!(
        apply(
            &mut commands,
            &mut channel,
            b"\x09quit\n\0\x2e\0first.wav\0\0",
            1,
            |incoming| match incoming {
                Incoming::StuffText(text) => {
                    assert_eq!(text, b"quit\n");
                    order.push("stuff");
                }
                Incoming::ConfigString(row) => {
                    assert_eq!(row.text, b"first.wav");
                    order.push("row");
                }
                Incoming::ResourceList(_) => panic!("no progress before the continuation byte"),
                _ => panic!("unexpected prefix"),
            }
        )
        .is_err()
    );
    assert_eq!(order, ["stuff", "row"]);
    assert!(
        apply(
            &mut commands,
            &mut channel,
            b"\x09unterminated",
            1,
            |_| panic!("unfinished stufftext")
        )
        .is_err()
    );
}

#[test]
fn qw_serverdata_clears_map_frames_commands_and_prediction_but_preserves_native_reliable_flight() {
    let protocol = Protocol::QuakeWorld28;
    let mut commands = Commands::load(protocol, 8192);
    commands.configure_history(4).unwrap();
    commands
        .record(UserCmd {
            server_time_ms: 10,
            ..Default::default()
        })
        .unwrap();
    commands
        .record(UserCmd {
            server_time_ms: 20,
            ..Default::default()
        })
        .unwrap();
    let mut channel = qw_channel(Endpoint::Client);
    channel.set_qw_player_model(7).unwrap();
    let receipt = channel.queue_client_command(protocol, b"retained").unwrap();
    let flight = channel
        .prepare(None, EventTime(1))
        .unwrap()
        .unwrap()
        .bytes
        .to_vec();
    channel.submitted(EventTime(1)).unwrap();
    let mut bytes = [0; 128];
    let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
    let mut words = [0; states::QW_PLAYER_WORDS];
    words[12] = 1 << 1;
    states::write_qw_player(
        &mut writer,
        0,
        &words,
        qa_network::commands::QwCmd {
            view_angles: Vec3([45.0, 90.0, 0.0]),
            ..ZERO_QW
        },
    )
    .unwrap();
    writer.write_data(b"\x2f\0\0").unwrap();
    apply(&mut commands, &mut channel, writer.bytes(), 1, |_| {}).unwrap();
    assert!(channel.current_snapshot().is_some());
    assert_eq!(commands.delta_request(), Some(1));
    channel.queue_client_command(protocol, b"nextdl").unwrap();
    let transport = (channel.state(), channel.send_state());
    let mut truncated = serverdata(28, 0, b"qw", MOVEVARS);
    truncated.pop();
    assert!(
        apply(&mut commands, &mut channel, &truncated, 2, |_| panic!(
            "truncated header"
        ))
        .is_err()
    );
    assert!(
        channel.current_snapshot().is_some(),
        "bad header cannot reset the prior accepted map"
    );
    assert_eq!(commands.history().unwrap().len(), 2);
    apply(
        &mut commands,
        &mut channel,
        &serverdata(28, 0, b"qw", MOVEVARS),
        2,
        |_| {},
    )
    .unwrap();
    assert!(channel.current_snapshot().is_none());
    assert!(channel.previous_snapshot().is_none());
    assert_eq!(commands.delta_request(), None);
    assert_eq!(
        (
            commands.history().unwrap().number(),
            commands.history().unwrap().len()
        ),
        (2, 0)
    );
    assert_eq!((channel.state(), channel.send_state()), transport);
    assert!(channel.has_output());
    assert_eq!(commands.record(UserCmd::default()).unwrap(), 3);
    // Sequence65 revisits the exact omitted-command slot populated by sequence1.
    let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
    words[12] = 0;
    states::write_qw_player(&mut writer, 0, &words, ZERO_QW).unwrap();
    apply(
        &mut commands,
        &mut channel,
        writer.bytes(),
        65,
        |incoming| {
            let Incoming::PlayerInfo(player) = incoming else {
                panic!("player");
            };
            assert_eq!(player.command, ZERO_QW);
            assert_eq!(player.words[8], 0, "old map player.mdl ordinal was reset");
        },
    )
    .unwrap();
    // Only a real native channel ACK retires the command held across serverdata.
    let mut server = qw_channel(Endpoint::Server);
    accept_request(&mut server, &flight, b"retained");
    let ack = server
        .prepare(Some(&[]), EventTime(4))
        .unwrap()
        .unwrap()
        .bytes
        .to_vec();
    server.submitted(EventTime(4)).unwrap();
    channel.receive(&ack, EventTime(5)).unwrap();
    assert_eq!(channel.reliable_receipts(), &[receipt]);
    assert!(
        !channel.has_output(),
        "unsent old nextdl was canceled, not transmitted after the surviving flight"
    );
}

#[test]
fn qw_new_serverdata_cancels_prepared_unsent_startup_without_losing_the_next_reliable_bit() {
    let protocol = Protocol::QuakeWorld28;
    let mut commands = Commands::load(protocol, 8192);
    let mut client = qw_channel(Endpoint::Client);
    let canceled = client.queue_client_command(protocol, b"nextdl").unwrap();
    let original = client.send_state();
    assert!(client.prepare(None, EventTime(1)).unwrap().is_some());
    assert_ne!(
        client.send_state().reliable_sequence,
        original.reliable_sequence
    );
    apply(
        &mut commands,
        &mut client,
        &serverdata(28, 0, b"qw", MOVEVARS),
        1,
        |_| {},
    )
    .unwrap();
    assert_eq!(client.send_state(), original);
    assert!(!client.has_output());
    let fresh = client
        .queue_client_command(protocol, b"soundlist -2147483648 0")
        .unwrap();
    assert_ne!(fresh, canceled);
    let flight = client
        .prepare(None, EventTime(2))
        .unwrap()
        .unwrap()
        .bytes
        .to_vec();
    client.submitted(EventTime(2)).unwrap();
    let mut server = qw_channel(Endpoint::Server);
    accept_request(&mut server, &flight, b"soundlist -2147483648 0");
    let ack = server
        .prepare(Some(&[]), EventTime(4))
        .unwrap()
        .unwrap()
        .bytes
        .to_vec();
    server.submitted(EventTime(4)).unwrap();
    client.receive(&ack, EventTime(5)).unwrap();
    assert_eq!(client.reliable_receipts(), &[fresh]);
    assert!(!client.has_output());
}
