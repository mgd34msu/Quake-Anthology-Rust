use qa_core::{
    loopback::Endpoint,
    primitives::{EntityId, ModuleId, SnapshotObservation, ThinkTime},
    sys_events::EventTime,
};
use qa_network::{
    channel::{self, Channel, Delivery, Unreliable},
    commands::{
        connection::Commands,
        packet::{self, Key, Move, Protocol, ZERO_Q2_RR},
    },
    headers::{self, Direction, Header},
    snapshots::{Frame, ReceivedFrame},
    states,
};

fn source(frame: u32) -> SnapshotObservation {
    SnapshotObservation {
        module: ModuleId(7),
        module_lifetime: EntityId {
            slot: 91,
            generation: 4,
        },
        native_frame: frame,
        time: EventTime::from_milliseconds(2_300),
    }
}
fn channel() -> Channel {
    let mut channel = Channel::load(channel::q2_new(false), Endpoint::Server, 8192, 16).unwrap();
    channel
        .configure_snapshots(Protocol::Quake2Repro1038)
        .unwrap();
    channel
}
fn publish(channel: &mut Channel, sequence: u32) {
    channel
        .publish_snapshot(ReceivedFrame::Quake2Repro(Frame {
            sequence,
            time: ThinkTime::Milliseconds(i64::from(sequence) * 50),
            command: 0,
            flags: 0,
            areas: &[],
            player: &[0; states::Q2_REPRO_PLAYER_WORDS],
            entities: &[],
        }))
        .unwrap();
}
fn submit(channel: &mut Channel, sequence: u32, observation: SnapshotObservation) {
    publish(channel, sequence);
    channel
        .prepare_snapshot(sequence, Some(observation), b"snapshot", observation.time)
        .unwrap()
        .unwrap();
    channel.submitted(observation.time).unwrap();
}

#[test]
fn publication_and_rejected_submission_do_not_explain_an_ack() {
    let mut channel = channel();
    publish(&mut channel, 46);
    assert_eq!(channel.snapshot_observation(46), None);
    channel
        .prepare_snapshot(46, Some(source(4)), b"snapshot", EventTime(1))
        .unwrap()
        .unwrap();
    assert_eq!(channel.submit_with(EventTime(2), |_| false).unwrap(), None);
    assert_eq!(channel.snapshot_observation(46), None);
    assert_eq!(
        channel.submit_with(EventTime(3), |_| true).unwrap(),
        Some(Unreliable::Included)
    );
    assert_eq!(channel.snapshot_observation(46), Some(source(4)));
    assert_eq!(channel.snapshot_observation(47), None);
    // Reusing the same ring slot invalidates its old publication and source.
    publish(&mut channel, 46 + 32);
    assert_eq!(channel.snapshot_observation(46), None);
    assert_eq!(channel.snapshot_observation(46 + 32), None);
}

#[test]
fn fragments_retain_the_original_source_and_defer_other_snapshots() {
    let mut channel = channel();
    publish(&mut channel, 46);
    publish(&mut channel, 47);
    channel
        .prepare_snapshot(46, Some(source(4)), &[7; 2700], EventTime(1))
        .unwrap()
        .unwrap();
    channel.submitted(EventTime(1)).unwrap();
    assert!(channel.pending_fragments());
    assert_eq!(channel.snapshot_observation(46), None);
    assert_eq!(
        channel
            .prepare_snapshot(47, Some(source(8)), b"new", EventTime(2))
            .unwrap()
            .unwrap()
            .unreliable,
        Unreliable::Deferred
    );
    channel.submitted(EventTime(2)).unwrap();
    assert_eq!(channel.snapshot_observation(46), None);
    channel.prepare(None, EventTime(3)).unwrap().unwrap();
    assert_eq!(channel.submit_with(EventTime(3), |_| false).unwrap(), None);
    assert_eq!(channel.snapshot_observation(46), None);
    channel.submit_with(EventTime(4), |_| true).unwrap();
    assert!(!channel.pending_fragments());
    assert_eq!(channel.snapshot_observation(46), Some(source(4)));
    assert_eq!(channel.snapshot_observation(47), None);
}

#[test]
fn dropped_snapshot_bytes_never_acquire_reliable_flight_provenance() {
    let mut channel = channel();
    publish(&mut channel, 46);
    channel.queue_reliable(&[3; 8000]).unwrap();
    assert_eq!(
        channel
            .prepare_snapshot(46, Some(source(4)), &[9; 600], EventTime(1))
            .unwrap()
            .unwrap()
            .unreliable,
        Unreliable::Dropped
    );
    channel.submitted(EventTime(1)).unwrap();
    while channel.pending_fragments() {
        channel.prepare(None, EventTime(2)).unwrap().unwrap();
        channel.submitted(EventTime(2)).unwrap();
    }
    assert_eq!(channel.snapshot_observation(46), None);
}

fn moves(
    codec: &mut Commands,
    channel: &mut Channel,
    sequence: u32,
    ack: i32,
    values: [f32; 3],
) -> Vec<(f32, Option<SnapshotObservation>)> {
    let movement = match codec.protocol {
        Protocol::Quake2Repro1038 => Move::Quake2Repro {
            last_frame: ack,
            commands: values.map(|value| qa_network::commands::Q2RrCmd {
                msec: 10,
                movement: [value, 0.],
                ..ZERO_Q2_RR
            }),
        },
        Protocol::Quake2_34 => Move::Quake2 {
            last_frame: ack,
            commands: values.map(|value| qa_network::commands::Q2Cmd {
                msec: 10,
                movement: [value as i16, 0, 0],
                ..packet::ZERO_Q2
            }),
        },
        _ => unreachable!(),
    };
    let mut body = [0; 1400];
    let length = packet::write(&mut body, &movement, sequence, Key::default()).unwrap();
    let mut bytes = [0; 1400];
    let length = headers::encode(
        if codec.protocol == Protocol::Quake2Repro1038 {
            headers::q2_new(false)
        } else {
            headers::QUAKE2
        },
        Direction::ToServer,
        Header {
            sequence,
            ..Default::default()
        },
        &body[..length],
        &mut bytes,
    )
    .unwrap();
    let received = channel.receive(&bytes[..length], EventTime(9)).unwrap();
    let Delivery::Payload(body) = received.delivery else {
        panic!("native payload");
    };
    let length = codec.stage(body).unwrap();
    let mut result = Vec::new();
    codec
        .decode(length, sequence, 9, channel, |command, observation| {
            result.push((command.movement[0], observation))
        })
        .unwrap();
    result
}

#[test]
fn packet_ack_observations_preserve_retained_backup_origin_without_invented_sample_frames() {
    let mut channel = channel();
    submit(&mut channel, 46, source(4));
    submit(&mut channel, 48, source(8));
    let mut commands = Commands::load(Protocol::Quake2Repro1038);
    assert_eq!(
        moves(&mut commands, &mut channel, 1, 46, [0., 0., 10.]),
        [(10., Some(source(4)))]
    );
    // Three missed packets repeat the previously admitted command, then two
    // packet backups and the newest command. The repeat retains its old ACK;
    // the packet's three commands share its ACK observation, not fake times.
    assert_eq!(
        moves(&mut commands, &mut channel, 5, 48, [20., 30., 40.]),
        [
            (10., Some(source(4))),
            (20., Some(source(8))),
            (30., Some(source(8))),
            (40., Some(source(8)))
        ]
    );
    assert_eq!(
        moves(&mut commands, &mut channel, 6, 99, [0., 0., 50.]),
        [(50., None)]
    );
    assert_eq!(
        moves(&mut commands, &mut channel, 7, -1, [0., 0., 60.]),
        [(60., None)]
    );
}

#[test]
fn native_frame_requires_the_same_module_and_anchor_lifetime() {
    let observation = source(4);
    assert_eq!(
        observation.frame_for(ModuleId(7), observation.module_lifetime),
        Some(4)
    );
    assert_eq!(
        observation.frame_for(ModuleId(8), observation.module_lifetime),
        None
    );
    assert_eq!(
        observation.frame_for(
            ModuleId(7),
            EntityId {
                generation: 5,
                ..observation.module_lifetime
            }
        ),
        None
    );
}

fn publish_protocol(channel: &mut Channel, protocol: Protocol, sequence: u32) {
    macro_rules! frame {
        ($variant:ident, $words:expr) => {
            ReceivedFrame::$variant(Frame {
                sequence,
                time: ThinkTime::Milliseconds(2300),
                command: 0,
                flags: 0,
                areas: &[],
                player: &[0; $words],
                entities: &[],
            })
        };
    }
    channel
        .publish_snapshot(match protocol {
            Protocol::QuakeWorld28 => frame!(QuakeWorld, 0),
            Protocol::Quake2_34 => frame!(Quake2, states::Q2_PLAYER_WORDS),
            Protocol::Quake3_68 => frame!(Quake3, states::PLAYER_WORDS),
            _ => unreachable!(),
        })
        .unwrap();
}

#[test]
fn different_protocol_ack_fields_project_the_same_native_provider_observation() {
    use packet::{Acknowledgements, ZERO_Q2, ZERO_Q3, ZERO_QW};
    for (protocol, sequence, ack) in [
        (Protocol::Quake2_34, 46, 46),
        (Protocol::QuakeWorld28, 302, 46),
        (Protocol::Quake3_68, 1, 1),
    ] {
        let mut channel = Channel::load(protocol.channel(), Endpoint::Server, 8192, 16).unwrap();
        channel.configure_snapshots(protocol).unwrap();
        publish_protocol(&mut channel, protocol, sequence);
        channel
            .prepare_snapshot(sequence, Some(source(4)), b"snapshot", EventTime(1))
            .unwrap()
            .unwrap();
        channel.submitted(EventTime(1)).unwrap();
        assert_eq!(channel.snapshot_observation(ack), Some(source(4)));
        let movement = match protocol {
            Protocol::Quake2_34 => Move::Quake2 {
                last_frame: ack as i32,
                commands: [ZERO_Q2; 3],
            },
            Protocol::QuakeWorld28 => Move::QuakeWorld {
                loss: 0,
                commands: [ZERO_QW; 3],
                delta_request: Some(ack as u8),
            },
            Protocol::Quake3_68 => {
                let mut commands = [ZERO_Q3; 32];
                commands[0].server_time = 10;
                Move::Quake3 {
                    commands,
                    count: 1,
                    delta: false,
                }
            }
            _ => unreachable!(),
        };
        let mut bytes = [0; 1400];
        let key = Key {
            acknowledgements: Acknowledgements {
                message: ack as i32,
                ..Default::default()
            },
            ..Default::default()
        };
        let length = packet::write(&mut bytes, &movement, 1, key).unwrap();
        let mut commands = Commands::load(protocol);
        let length = commands.stage(&bytes[..length]).unwrap();
        let mut observed = Vec::new();
        commands
            .decode(length, 1, 10, &mut channel, |_, observation| {
                observed.push(observation)
            })
            .unwrap();
        assert_eq!(observed, [Some(source(4))], "{protocol:?}");
        if protocol == Protocol::QuakeWorld28 {
            assert_eq!(channel.snapshot_observation(ack + 32), None);
        }
    }
}

#[test]
fn rerelease_wire_frame_wrap_resolves_only_the_exact_retained_slot() {
    use qa_network::snapshots::Q2Header;
    let mut channel = channel();
    submit(&mut channel, 46, source(4));
    let wrapped = Q2Header::REPRO_FRAME_MASK + 1 + 46;
    submit(&mut channel, wrapped, source(8));
    assert_eq!(channel.snapshot_observation(46), Some(source(8)));
    assert_eq!(channel.snapshot_observation(46 + 32), None);
    assert_eq!(channel.snapshot_observation(wrapped), None);
    // Publishing a new generation in that slot removes the older submitted
    // metadata even before the new payload is admitted.
    publish(&mut channel, wrapped + 32);
    assert_eq!(channel.snapshot_observation(46), None);
}

#[test]
fn classic_q2_repeats_keep_their_original_snapshot_observation() {
    let mut channel =
        Channel::load(Protocol::Quake2_34.channel(), Endpoint::Server, 8192, 16).unwrap();
    channel.configure_snapshots(Protocol::Quake2_34).unwrap();
    for (sequence, observation) in [(46, source(4)), (48, source(8))] {
        publish_protocol(&mut channel, Protocol::Quake2_34, sequence);
        channel
            .prepare_snapshot(sequence, Some(observation), b"snapshot", EventTime(1))
            .unwrap()
            .unwrap();
        channel.submitted(EventTime(1)).unwrap();
    }
    let mut commands = Commands::load(Protocol::Quake2_34);
    assert_eq!(
        moves(&mut commands, &mut channel, 1, 46, [0., 0., 10.]),
        [(10., Some(source(4)))]
    );
    assert_eq!(
        moves(&mut commands, &mut channel, 5, 48, [20., 30., 40.]),
        [
            (10., Some(source(4))),
            (20., Some(source(8))),
            (30., Some(source(8))),
            (40., Some(source(8)))
        ]
    );
}
