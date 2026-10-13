use qa_app::{
    Runtime,
    client_policy::ClientPolicy,
    host::{FrameHost, FrameResult, FrameSource},
    map::SpawnAnchor,
};
use qa_audio::{Bank, Listener, Mixer};
use qa_console::{commands::Console, views::Context};
use qa_core::{events::*, loopback::Endpoint, primitives::*, sys_events::*};
use qa_network::{
    commands::packet::Protocol,
    message::{Encoding, Writer},
    projection::EntityProjection,
    snapshots::{Entity, Frame, ReceivedFrame},
    states,
};
use qa_session::timing::TickRate;
use std::{num::NonZeroU32, sync::Arc, time::Duration};

struct Source {
    mixer: Mixer,
    observations: Vec<(
        SoundAction,
        SoundId,
        Option<SoundEmitter>,
        Option<LoopPlayback>,
    )>,
}
impl FrameSource for Source {
    fn begin_frame(&mut self) -> Duration {
        Duration::ZERO
    }
    fn poll_events(&mut self, _: &mut SysEventQueue) {}
    fn wait_time(&mut self, _: Duration) -> Duration {
        Duration::ZERO
    }
    fn elapsed(&self) -> Duration {
        Duration::ZERO
    }
    fn present(&mut self) {}
    fn sound(&mut self, event: SoundEvent) -> bool {
        self.observations
            .push((event.action, event.sound, event.entity, event.looping));
        self.mixer.sound(event)
    }
}
fn fixture(
    protocol: Protocol,
    clients: usize,
    capacity: usize,
    voices: usize,
) -> (FrameHost, Source, Vec<ClientId>) {
    let rate = NonZeroU32::new(44100).unwrap();
    let bank = Arc::new(
        Bank::load(
            rate,
            vec![
                (
                    "sound/first.wav".into(),
                    Pcm {
                        rate,
                        channels: PcmChannels::Mono,
                        samples: vec![1000, -500, 2000, 300, -1700],
                        loop_start: Some(2),
                        loop_end: None,
                        loop_invalid: false,
                    },
                ),
                (
                    "sound/second.wav".into(),
                    Pcm {
                        rate,
                        channels: PcmChannels::Mono,
                        samples: vec![8000, -4000, 6000, 2400],
                        loop_start: None,
                        loop_end: None,
                        loop_invalid: false,
                    },
                ),
            ],
        )
        .unwrap(),
    );
    let mut runtime = Runtime::load(clients, []).unwrap();
    runtime.server.events = EventRing::load(capacity, clients + 1, capacity + 1, 8192).unwrap();
    runtime.server.presentation = runtime
        .server
        .events
        .bind(OutputTarget::Presentation)
        .unwrap();
    let ids = (0..clients)
        .map(|seat| {
            runtime
                .connect_local(
                    SeatId::new(seat as u8).unwrap(),
                    SpawnAnchor {
                        position: Vec3::default(),
                        angles: Vec3::default(),
                        entity: 0,
                        fixture_fallback: false,
                    },
                    ClientPolicy {
                        client: protocol.rule_set(),
                        movement: RuleSetId::QuakeWorld,
                        trace: RuleSetId::Quake,
                    },
                    protocol,
                    seat as u32,
                )
                .unwrap()
        })
        .collect::<Vec<_>>();
    let mut host = FrameHost::load(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        vec![],
    )
    .unwrap();
    for (seat, &client) in ids.iter().enumerate() {
        host.local_clients[seat] = Some(client);
    }
    host.runtime.sound_bank = Some(Arc::clone(&bank));
    host.bind_sounds(&bank).unwrap();
    let mut mixer = Mixer::load(bank, voices, clients);
    mixer.listen(
        &ids.iter()
            .map(|&client| {
                Some(Listener {
                    negate_dot: false,
                    client,
                    entity: None,
                    origin: Vec3::default(),
                    right: Vec3([1.0, 0.0, 0.0]),
                })
            })
            .collect::<Vec<_>>(),
        1.0,
    );
    (
        host,
        Source {
            mixer,
            observations: Vec::new(),
        },
        ids,
    )
}
fn drain(host: &mut FrameHost, time: EventTime) {
    host.queue
        .push(SysEvent {
            time,
            kind: EventKind::Time,
        })
        .unwrap();
    host.drain(&mut FrameResult::default());
    host.client_modules();
}
fn config(host: &mut FrameHost, client: ClientId, protocol: Protocol, ordinal: u32, name: &[u8]) {
    let index = protocol.config_layout().unwrap().resources[1]
        .get(ordinal as usize)
        .unwrap();
    let mut bytes = [0; 8196];
    let length = if protocol == Protocol::Quake3_68 {
        qa_network::outputs::config_command(index as u32, name, 0, &mut bytes)
            .unwrap()
            .length
    } else {
        qa_network::outputs::configstring(protocol, index as u32, name, &mut bytes).unwrap()
    };
    let channel = &mut host
        .runtime
        .network
        .get_mut(client, Endpoint::Server)
        .unwrap()
        .channel;
    channel.queue_reliable(&bytes[..length]).unwrap();
    let time = EventTime::from_milliseconds(10);
    loop {
        let packet = channel.prepare_output(time).unwrap().unwrap();
        host.runtime
            .loopback
            .send(Endpoint::Server, client, packet.bytes)
            .unwrap();
        channel.submitted(time).unwrap();
        if !channel.pending_fragments() {
            break;
        }
    }
    drain(host, time);
    ack_client(host, client, time);
}
fn ack_client(host: &mut FrameHost, client: ClientId, time: EventTime) {
    assert!(
        host.runtime
            .send_local_command(client, &UserCmd::default(), time)
    );
    drain(host, time);
}
fn record(protocol: Protocol, client: ClientId, sound: u32, entity_type: u32) -> EntityRecord {
    EntityRecord {
        position: Vec3([120.0, 0.0, 0.0]),
        appearance: EntityAppearance {
            source: protocol.rule_set(),
            loop_sound: SoundRef::native(ResourceNamespace::Client(client), sound),
            native: NativeAppearanceTail {
                entity_type,
                loop_volume: 0.0,
                loop_attenuation: 0.0,
                ..Default::default()
            },
            ..Default::default()
        },
        ..Default::default()
    }
}
fn snapshot(
    host: &mut FrameHost,
    client: ClientId,
    protocol: Protocol,
    frame_number: u32,
    records: &[(u32, EntityRecord)],
) {
    snapshot_with_player(host, client, protocol, frame_number, records, None);
}
fn snapshot_with_player(
    host: &mut FrameHost,
    client: ClientId,
    protocol: Protocol,
    frame_number: u32,
    records: &[(u32, EntityRecord)],
    player_words: Option<&[u32; states::PLAYER_WORDS]>,
) {
    let channel = &mut host
        .runtime
        .network
        .get_mut(client, Endpoint::Server)
        .unwrap()
        .channel;
    let sequence = if protocol == Protocol::Quake3_68 {
        channel.send_state().sequence
    } else {
        frame_number
    };
    let projection = EntityProjection::load(protocol);
    let time = EventTime::from_milliseconds(u64::from(frame_number) * 100);
    let mut bytes = [0; 16384];
    macro_rules! write {
        ($kind:ident, $player:expr, $entity:expr) => {{
            let mut player = [0; $player];
            if let Some(words) = player_words {
                player.copy_from_slice(&words[..$player]);
            }
            let entities = records
                .iter()
                .map(|(number, record)| {
                    let mut words = [0; $entity];
                    assert!(projection.reduce(
                        record,
                        &mut words,
                        |_| None,
                        |sound| {
                            (sound.namespace == ResourceNamespace::Client(client))
                                .then_some(sound.ordinal)
                        }
                    ));
                    Entity {
                        number: *number,
                        words,
                    }
                })
                .collect::<Vec<_>>();
            let frame = ReceivedFrame::$kind(Frame {
                sequence,
                time: ThinkTime::Milliseconds(time.milliseconds() as i64),
                command: 0,
                flags: 0,
                qw_peers: 0,
                areas: &[],
                player: &player,
                entities: &entities,
            });
            let length = if protocol == Protocol::Quake3_68 {
                channel
                    .encode_server_output(&mut bytes, |writer, channel| {
                        channel.write_snapshot_candidate(writer, frame, None, 64)
                    })
                    .unwrap()
            } else {
                let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
                channel
                    .write_snapshot_candidate(&mut writer, frame, None, 64)
                    .unwrap();
                writer.size()
            };
            channel.publish_written_snapshot(frame, None).unwrap();
            length
        }};
    }
    let length = match protocol {
        Protocol::Quake2_34 => write!(Quake2, states::Q2_PLAYER_WORDS, states::Q2_ENTITY_WORDS),
        Protocol::Quake2Repro1038 => write!(
            Quake2Repro,
            states::Q2_REPRO_PLAYER_WORDS,
            states::Q2_RERELEASE_ENTITY_WORDS
        ),
        Protocol::Quake3_68 => write!(Quake3, states::PLAYER_WORDS, states::ENTITY_WORDS),
        _ => panic!("loop-capable native protocol"),
    };
    channel
        .prepare_snapshot(sequence, None, &bytes[..length], time)
        .unwrap()
        .unwrap();
    assert!(
        channel
            .submit_with(time, |packet| host
                .runtime
                .loopback
                .send(Endpoint::Server, client, packet)
                .is_ok())
            .unwrap()
            .is_some()
    );
    while channel.pending_fragments() {
        channel.prepare_output(time).unwrap().unwrap();
        assert!(
            channel
                .submit_with(time, |packet| host
                    .runtime
                    .loopback
                    .send(Endpoint::Server, client, packet)
                    .is_ok())
                .unwrap()
                .is_some()
        );
    }
    drain(host, time);
    ack_client(host, client, time);
}
fn dispatch(host: &mut FrameHost, source: &mut Source) {
    qa_app::output::dispatch(
        &mut host.runtime,
        source,
        &mut host.console.scrollback,
        &host.local_clients,
        host.time,
        3.0,
        2.0,
    );
}
fn paint(host: &FrameHost, source: &mut Source) -> [i16; 10] {
    qa_app::audio::respatialize(&mut source.mixer, &host.client_entities_inputs());
    let mut pcm = [0; 10];
    assert!(source.mixer.paint(&mut pcm));
    pcm
}
fn starts(source: &Source) -> usize {
    source
        .observations
        .iter()
        .filter(|row| row.0 == SoundAction::StartLoop)
        .count()
}
fn accepted_snapshots_and_numeric_rewrites(protocol: Protocol) {
    let (mut host, mut source, ids) = fixture(protocol, 1, 32, 8);
    let client = ids[0];
    let name = if protocol == Protocol::Quake3_68 {
        &b"sound/first.wav"[..]
    } else {
        &b"first.wav"[..]
    };
    config(&mut host, client, protocol, 1, name);
    let number = if protocol == Protocol::Quake3_68 {
        0
    } else {
        2
    };
    let current = record(protocol, client, 1, 0);
    snapshot(&mut host, client, protocol, 1, &[(number, current)]);
    // SERVER is irrelevant after acceptance, including poisoned common poses.
    host.runtime
        .server
        .entities
        .columns
        .position
        .fill(Vec3([f32::NAN; 3]));
    host.publish_client_loops();
    dispatch(&mut host, &mut source);
    assert_eq!(starts(&source), 1);
    let emission = source
        .observations
        .iter()
        .find(|row| row.0 == SoundAction::StartLoop)
        .unwrap();
    assert_eq!(
        emission.2,
        Some(SoundEmitter::Native {
            namespace: ResourceNamespace::Client(client),
            slot: number as i32
        })
    );
    assert_eq!(
        emission.3.unwrap().origin,
        if protocol == Protocol::Quake2_34 {
            LoopOrigin::Accepted
        } else {
            LoopOrigin::Presented
        }
    );
    assert!(paint(&host, &mut source).iter().any(|sample| *sample != 0));
    for _ in 0..3 {
        host.publish_client_loops();
        dispatch(&mut host, &mut source);
    }
    assert_eq!(
        starts(&source),
        1,
        "unchanged accepted state republishes nothing"
    );
    let name = if protocol == Protocol::Quake3_68 {
        &b"sound/second.wav"[..]
    } else {
        &b"second.wav"[..]
    };
    config(&mut host, client, protocol, 1, name);
    host.publish_client_loops();
    dispatch(&mut host, &mut source);
    assert_eq!(
        starts(&source),
        2,
        "numeric config rewrite is seen without a new snapshot"
    );
    assert_eq!(
        source.observations.last().unwrap().1,
        source.mixer.bank.find("sound/second.wav").unwrap()
    );
    // An ordinary channel-zero voice survives the admitted-loop disappearance.
    host.runtime
        .server
        .events
        .push_to(
            OutputTarget::Presentation,
            FrameEvent::Sound(SoundEvent {
                sound: source.mixer.bank.find("sound/first.wav").unwrap(),
                volume: 1.0,
                attenuation: 0.0,
                client: Some(client),
                client_lifetime: Some(host.runtime.server.clients[0].entity),
                entity: Some(SoundEmitter::Native {
                    namespace: ResourceNamespace::Client(client),
                    slot: number as i32,
                }),
                ..SoundEvent::default()
            }),
        )
        .unwrap();
    dispatch(&mut host, &mut source);
    snapshot(&mut host, client, protocol, 2, &[]);
    host.publish_client_loops();
    dispatch(&mut host, &mut source);
    assert_eq!(source.observations.last().unwrap().0, SoundAction::StopLoop);
    assert!(
        paint(&host, &mut source).iter().any(|sample| *sample != 0),
        "one-shot channel zero survived"
    );
}
fn fair_pressure_reset_and_voice_rejection() {
    let protocol = Protocol::Quake2_34;
    let (mut host, mut source, ids) = fixture(protocol, 2, 4, 1);
    for &client in &ids {
        config(&mut host, client, protocol, 1, b"first.wav");
        snapshot(
            &mut host,
            client,
            protocol,
            1,
            &[(2, record(protocol, client, 1, 0))],
        );
    }
    let lifetime = host.runtime.server.clients[0].entity;
    host.runtime
        .server
        .events
        .push_to(
            OutputTarget::Presentation,
            FrameEvent::Sound(SoundEvent {
                sound: source.mixer.bank.find("sound/first.wav").unwrap(),
                client: Some(ids[0]),
                client_lifetime: Some(lifetime),
                volume: 1.0,
                attenuation: 0.0,
                channel: 7,
                action: SoundAction::StartLoop,
                ..SoundEvent::default()
            }),
        )
        .unwrap();
    dispatch(&mut host, &mut source);
    for _ in 0..4 {
        host.publish_client_loops();
        dispatch(&mut host, &mut source);
    }
    assert!(
        source.mixer.counts.full > 0,
        "actual bounded mixer rejected a loop"
    );
    assert!(
        source
            .observations
            .iter()
            .any(|row| row.0 == SoundAction::ResetClientLoops)
    );
    assert!(
        starts(&source) >= 3,
        "rejected starts were not permanently settled"
    );
    host.runtime
        .server
        .events
        .push_to(
            OutputTarget::Presentation,
            FrameEvent::Sound(SoundEvent {
                client: Some(ids[0]),
                client_lifetime: Some(lifetime),
                channel: 7,
                action: SoundAction::Stop,
                ..SoundEvent::default()
            }),
        )
        .unwrap();
    dispatch(&mut host, &mut source);
    for _ in 0..6 {
        host.publish_client_loops();
        dispatch(&mut host, &mut source);
    }
    assert!(paint(&host, &mut source).iter().any(|sample| *sample != 0));
    // Cancel retained publications twice; reset completion must be consumed,
    // not inferred from successful ring admission or its disappearing token.
    for _ in 0..2 {
        for _ in 0..5 {
            host.runtime
                .server
                .events
                .push_to(
                    OutputTarget::Presentation,
                    FrameEvent::Effect(EffectEvent {
                        effect: EffectId(0),
                        position: Vec3::default(),
                        direction: Vec3::default(),
                        count: 1,
                        ..EffectEvent::default()
                    }),
                )
                .unwrap();
        }
        host.publish_client_loops();
        dispatch(&mut host, &mut source);
    }
    for _ in 0..8 {
        host.publish_client_loops();
        dispatch(&mut host, &mut source);
    }
    assert!(
        host.runtime
            .server
            .events
            .counters(host.runtime.server.presentation)
            .unwrap()
            .overflow
            > 0
    );
    assert!(
        source
            .observations
            .iter()
            .filter(|row| row.0 == SoundAction::ResetClientLoops)
            .count()
            >= 2
    );
}
fn q3_real_and_native_stop_event() {
    let protocol = Protocol::Quake3_68;
    let (mut host, mut source, ids) = fixture(protocol, 1, 32, 8);
    let client = ids[0];
    config(&mut host, client, protocol, 1, b"sound/first.wav");
    let real = record(protocol, client, 1, 7);
    snapshot(&mut host, client, protocol, 1, &[(0, real)]);
    host.publish_client_loops();
    dispatch(&mut host, &mut source);
    assert_eq!(
        source.observations.last().unwrap().3.unwrap().lifetime,
        LoopLifetime::Persistent
    );
    snapshot(&mut host, client, protocol, 2, &[]);
    host.publish_client_loops();
    dispatch(&mut host, &mut source);
    assert_eq!(
        source.observations.len(),
        1,
        "native real loop survives a missing row"
    );
    let mut stopped = real;
    stopped.appearance.native.event = 75 | 0x100;
    snapshot(&mut host, client, protocol, 3, &[(0, stopped)]);
    host.publish_client_loops();
    dispatch(&mut host, &mut source);
    assert_eq!(source.observations.last().unwrap().0, SoundAction::StopLoop);
    let count = source.observations.len();
    host.publish_client_loops();
    dispatch(&mut host, &mut source);
    assert_eq!(
        source.observations.len(),
        count,
        "same accepted event is not replayed"
    );
    assert!(paint(&host, &mut source).iter().all(|sample| *sample == 0));
}
fn independent_clients_and_reused_lifetime() {
    let protocol = Protocol::Quake2_34;
    let (mut host, mut source, ids) = fixture(protocol, 2, 32, 8);
    for (index, &client) in ids.iter().enumerate() {
        let name = if index == 0 {
            &b"first.wav"[..]
        } else {
            &b"second.wav"[..]
        };
        config(&mut host, client, protocol, 1, name);
        snapshot(
            &mut host,
            client,
            protocol,
            1,
            &[(2, record(protocol, client, 1, 0))],
        );
    }
    host.publish_client_loops();
    dispatch(&mut host, &mut source);
    assert_eq!(starts(&source), 2);
    for (index, &client) in ids.iter().enumerate() {
        let name = if index == 0 {
            "sound/first.wav"
        } else {
            "sound/second.wav"
        };
        assert!(
            source
                .observations
                .iter()
                .any(|row| row.0 == SoundAction::StartLoop
                    && row.1 == source.mixer.bank.find(name).unwrap()
                    && row.2
                        == Some(SoundEmitter::Native {
                            namespace: ResourceNamespace::Client(client),
                            slot: 2
                        }))
        );
    }
    let old = host.runtime.server.clients[0].entity;
    assert!(host.runtime.disconnect_client(ids[0]));
    dispatch(&mut host, &mut source);
    let client = host
        .runtime
        .connect_local(
            SeatId::FIRST,
            SpawnAnchor {
                position: Vec3::default(),
                angles: Vec3::default(),
                entity: 0,
                fixture_fallback: false,
            },
            ClientPolicy {
                client: RuleSetId::Quake2,
                movement: RuleSetId::QuakeWorld,
                trace: RuleSetId::Quake,
            },
            protocol,
            0,
        )
        .unwrap();
    assert_eq!(client, ids[0]);
    assert_ne!(host.runtime.server.clients[0].entity, old);
    config(&mut host, client, protocol, 1, b"first.wav");
    snapshot(
        &mut host,
        client,
        protocol,
        1,
        &[(2, record(protocol, client, 1, 0))],
    );
    host.publish_client_loops();
    dispatch(&mut host, &mut source);
    assert_eq!(
        starts(&source),
        3,
        "new CLIENT lifetime publishes its loop independently"
    );
    host.runtime
        .server
        .events
        .push_to(
            OutputTarget::Presentation,
            FrameEvent::Sound(SoundEvent::reset_client_loops(client, old)),
        )
        .unwrap();
    dispatch(&mut host, &mut source);
    assert!(
        paint(&host, &mut source).iter().any(|sample| *sample != 0),
        "old reset cannot retire new CLIENT voices"
    );
    snapshot(&mut host, ids[1], protocol, 2, &[]);
    host.publish_client_loops();
    dispatch(&mut host, &mut source);
    assert!(
        paint(&host, &mut source).iter().any(|sample| *sample != 0),
        "one CLIENT disappearance leaves the other namespace audible"
    );
}
fn q3_standalone_stop_is_once_until_disappearance() {
    let protocol = Protocol::Quake3_68;
    let (mut host, mut source, ids) = fixture(protocol, 1, 4, 8);
    let client = ids[0];
    config(&mut host, client, protocol, 1, b"sound/first.wav");
    let real = record(protocol, client, 1, 7);
    snapshot(&mut host, client, protocol, 1, &[(0, real)]);
    host.publish_client_loops();
    dispatch(&mut host, &mut source);
    let mut event = record(protocol, client, 0, 13 + 75);
    event.appearance.native.render_flags = 0x10;
    event.appearance.native.other_entity = 0;
    snapshot(&mut host, client, protocol, 2, &[(0, real), (3, event)]);
    // The one-record budget publishes the stop, with no underflow/start in
    // the same pass. It targets native zero rather than the event row.
    host.publish_client_loops();
    dispatch(&mut host, &mut source);
    assert_eq!(source.observations.last().unwrap().0, SoundAction::StopLoop);
    assert_eq!(
        source.observations.last().unwrap().2,
        Some(SoundEmitter::Native {
            namespace: ResourceNamespace::Client(client),
            slot: 0
        })
    );
    snapshot(&mut host, client, protocol, 3, &[(0, real), (3, event)]);
    host.publish_client_loops();
    dispatch(&mut host, &mut source);
    assert_eq!(
        source.observations.last().unwrap().0,
        SoundAction::StartLoop
    );
    let stops = source
        .observations
        .iter()
        .filter(|row| row.0 == SoundAction::StopLoop)
        .count();
    event.appearance.native.entity_type += 0x100;
    snapshot(&mut host, client, protocol, 4, &[(0, real), (3, event)]);
    for _ in 0..3 {
        host.publish_client_loops();
        dispatch(&mut host, &mut source);
    }
    assert_eq!(
        source
            .observations
            .iter()
            .filter(|row| row.0 == SoundAction::StopLoop)
            .count(),
        stops,
        "standalone event rows ignore toggle changes until actual disappearance"
    );
    snapshot(&mut host, client, protocol, 5, &[(0, real)]);
    for _ in 0..2 {
        host.publish_client_loops();
        dispatch(&mut host, &mut source);
    }
    event.appearance.native.entity_type = 13 + 75;
    snapshot(&mut host, client, protocol, 6, &[(0, real), (3, event)]);
    for _ in 0..2 {
        host.publish_client_loops();
        dispatch(&mut host, &mut source);
    }
    assert_eq!(
        source
            .observations
            .iter()
            .filter(|row| row.0 == SoundAction::StopLoop)
            .count(),
        stops + 1
    );
}
fn q3_cross_target_stop_retires_the_event_rows_old_loop() {
    let protocol = Protocol::Quake3_68;
    let (mut host, mut source, ids) = fixture(protocol, 1, 4, 8);
    let client = ids[0];
    config(&mut host, client, protocol, 1, b"sound/first.wav");
    let real = record(protocol, client, 1, 7);
    let ordinary = record(protocol, client, 1, 0);
    snapshot(&mut host, client, protocol, 1, &[(0, real), (42, ordinary)]);
    for _ in 0..2 {
        host.publish_client_loops();
        dispatch(&mut host, &mut source);
    }
    assert_eq!(starts(&source), 2);
    let mut stop = record(protocol, client, 0, 13 + 75);
    stop.appearance.native.render_flags = 0x10;
    stop.appearance.native.other_entity = 0;
    snapshot(&mut host, client, protocol, 2, &[(0, real), (42, stop)]);
    for number in [0, 42] {
        let before = source.observations.len();
        host.publish_client_loops();
        dispatch(&mut host, &mut source);
        assert_eq!(
            source.observations.len(),
            before + 1,
            "one-record publication budget"
        );
        let event = source.observations.last().unwrap();
        assert_eq!(event.0, SoundAction::StopLoop);
        assert_eq!(
            event.2,
            Some(SoundEmitter::Native {
                namespace: ResourceNamespace::Client(client),
                slot: number
            })
        );
    }
    assert!(paint(&host, &mut source).iter().all(|sample| *sample == 0));
}
fn q3_player_loop_uses_the_existing_emitter_without_a_packet_self_row() {
    let protocol = Protocol::Quake3_68;
    let (mut host, mut source, ids) = fixture(protocol, 1, 32, 8);
    let client = ids[0];
    config(&mut host, client, protocol, 1, b"sound/first.wav");
    config(&mut host, client, protocol, 2, b"sound/second.wav");
    let mut player = [0; states::PLAYER_WORDS];
    player[1] = 64.0f32.to_bits();
    player[47] = 1;
    snapshot_with_player(&mut host, client, protocol, 1, &[], Some(&player));
    assert_eq!(
        host.runtime.prediction[0].player.loop_sound,
        SoundRef::native(ResourceNamespace::Client(client), 1),
    );
    let ReceivedFrame::Quake3(frame) = host
        .runtime
        .network
        .get(client, Endpoint::Client)
        .unwrap()
        .channel
        .current_snapshot()
        .unwrap()
    else {
        panic!("expected Q3 native frame")
    };
    assert!(frame.entities.is_empty());
    // No native guest or SERVER pose can supply the missing CLIENT emitter.
    host.runtime
        .server
        .entities
        .columns
        .position
        .fill(Vec3([f32::NAN; 3]));
    host.publish_client_loops();
    dispatch(&mut host, &mut source);
    assert_eq!(starts(&source), 1);
    assert_eq!(
        source.observations.last().unwrap().2,
        Some(SoundEmitter::Native {
            namespace: ResourceNamespace::Client(client),
            slot: 0,
        }),
    );
    assert_eq!(
        host.client_entities_inputs()[0]
            .unwrap()
            .sound_origin(0, Some(LoopOrigin::Presented)),
        Some(Vec3([64.0, 0.0, 0.0])),
    );
    assert!(paint(&host, &mut source).iter().any(|sample| *sample != 0));
    for _ in 0..4 {
        host.publish_client_loops();
        dispatch(&mut host, &mut source);
    }
    assert_eq!(starts(&source), 1);
    player[47] = 0;
    snapshot_with_player(&mut host, client, protocol, 2, &[], Some(&player));
    host.publish_client_loops();
    dispatch(&mut host, &mut source);
    assert_eq!(source.observations.last().unwrap().0, SoundAction::StopLoop);
    assert!(paint(&host, &mut source).iter().all(|sample| *sample == 0));

    // FOLLOW chooses a native player number independently of this connection.
    player[40] = 1;
    player[47] = 1;
    player[1] = 32.0f32.to_bits();
    snapshot_with_player(&mut host, client, protocol, 3, &[], Some(&player));
    host.publish_client_loops();
    dispatch(&mut host, &mut source);
    assert_eq!(starts(&source), 2);
    assert_eq!(
        source.observations.last().unwrap().2,
        Some(SoundEmitter::Native {
            namespace: ResourceNamespace::Client(client),
            slot: 1,
        })
    );
    // A zero packet sound retains PS sound and updates the same emitter pose.
    let mut packet = record(protocol, client, 0, 1);
    packet.position = Vec3([96.0, 0.0, 0.0]);
    snapshot_with_player(
        &mut host,
        client,
        protocol,
        4,
        &[(1, packet)],
        Some(&player),
    );
    host.publish_client_loops();
    dispatch(&mut host, &mut source);
    assert_eq!(starts(&source), 2);
    assert_eq!(
        host.client_entities_inputs()[0]
            .unwrap()
            .sound_origin(1, Some(LoopOrigin::Accepted)),
        Some(packet.position)
    );
    packet.appearance.loop_sound = SoundRef::native(ResourceNamespace::Client(client), 2);
    snapshot_with_player(
        &mut host,
        client,
        protocol,
        5,
        &[(1, packet)],
        Some(&player),
    );
    host.publish_client_loops();
    dispatch(&mut host, &mut source);
    assert_eq!(starts(&source), 3);
    assert_eq!(source.observations.last().unwrap().1, SoundId(2));
    assert!(paint(&host, &mut source).iter().any(|sample| *sample != 0));
}
fn main() {
    if let Some(status) = qa_platform::native::native_child_bootstrap() {
        std::process::exit(status);
    }
    for protocol in [
        Protocol::Quake2_34,
        Protocol::Quake2Repro1038,
        Protocol::Quake3_68,
    ] {
        accepted_snapshots_and_numeric_rewrites(protocol);
    }
    fair_pressure_reset_and_voice_rejection();
    q3_real_and_native_stop_event();
    independent_clients_and_reused_lifetime();
    q3_standalone_stop_is_once_until_disappearance();
    q3_cross_target_stop_retires_the_event_rows_old_loop();
    q3_player_loop_uses_the_existing_emitter_without_a_packet_self_row();
    println!("accepted CLIENT loop fixtures passed");
}
