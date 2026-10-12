use super::*;
use qa_core::primitives::{ClientId, ResourceNamespace, SoundEmitter, SoundEvent, Vec3};

fn send_payload(host: &mut FrameHost, client: ClientId, body: &[u8]) {
    use qa_core::loopback::Endpoint;
    let runtime = &mut host.runtime;
    let channel = &mut runtime
        .network
        .get_mut(client, Endpoint::Server)
        .unwrap()
        .channel;
    assert!(channel.prepare(Some(body), host.time).unwrap().is_some());
    assert!(
        channel
            .submit_with(host.time, |packet| runtime
                .loopback
                .send(Endpoint::Server, client, packet)
                .is_ok())
            .unwrap()
            .is_some()
    );
    host.drain(&mut Default::default());
}

fn send_cue(host: &mut FrameHost, client: ClientId, cue: qa_network::outputs::Cue) {
    let protocol = host.runtime.snapshots[client.0 as usize]
        .as_ref()
        .unwrap()
        .protocol;
    let mut body = [0; 64];
    let length = qa_network::outputs::write_cue(protocol, cue, &mut body).unwrap();
    send_payload(host, client, &body[..length]);
}

fn emitter(client: ClientId, slot: i32) -> SoundEmitter {
    SoundEmitter::Native {
        namespace: ResourceNamespace::Client(client),
        slot,
    }
}

fn client_position(host: &FrameHost, client: ClientId, slot: u32) -> Vec3 {
    use qa_core::loopback::Endpoint;
    use qa_network::{projection::EntityProjection, snapshots::Frame};
    fn position<const P: usize, const E: usize>(
        projection: &EntityProjection,
        frame: Frame<'_, P, E>,
        client: ClientId,
        slot: u32,
    ) -> Vec3 {
        let row = frame
            .entities
            .binary_search_by_key(&slot, |entity| entity.number)
            .unwrap();
        let mut record = qa_network::projection::EntityRecord::default();
        assert!(projection.apply(
            &frame.entities[row].words,
            &mut record,
            ResourceNamespace::Client(client)
        ));
        record.position
    }
    let projection = host.runtime.snapshots[client.0 as usize]
        .as_ref()
        .unwrap()
        .entity_projection();
    let current = host
        .runtime
        .network
        .get(client, Endpoint::Client)
        .unwrap()
        .channel
        .current_snapshot()
        .unwrap();
    match current {
        qa_network::snapshots::ReceivedFrame::Quake2(frame) => {
            position(projection, frame, client, slot)
        }
        qa_network::snapshots::ReceivedFrame::Quake2Repro(frame) => {
            position(projection, frame, client, slot)
        }
        _ => panic!("Q2 cue fixture"),
    }
}

fn accept_client_resources_and_pose(
    host: &mut FrameHost,
    client: ClientId,
    bank: &std::sync::Arc<qa_audio::Bank>,
) -> Vec3 {
    use qa_core::loopback::Endpoint;
    host.runtime.sound_bank = Some(std::sync::Arc::clone(bank));
    let binding = host.runtime.snapshots[client.0 as usize].as_ref().unwrap();
    let protocol = binding.protocol;
    let native_slot = binding.native_client() + 1;
    let resource = host.runtime.client_resources[client.0 as usize]
        .as_ref()
        .unwrap()
        .state
        .layout
        .resources[1]
        .get(1)
        .unwrap();
    let mut body = [0; 1400];
    let length =
        qa_network::outputs::configstring(protocol, resource as u32, b"_qa_cue.wav", &mut body)
            .unwrap();
    send_payload(host, client, &body[..length]);
    assert_eq!(
        host.runtime.client_resources[client.0 as usize]
            .as_ref()
            .unwrap()
            .state
            .sound(1),
        bank.find("sound/_qa_cue.wav")
    );
    let previous = host
        .runtime
        .network
        .get(client, Endpoint::Client)
        .unwrap()
        .channel
        .current_snapshot()
        .map_or(0, |frame| frame.time().milliseconds());
    let time = EventTime::from_milliseconds(u64::try_from(previous).unwrap() + 100);
    assert!(
        host.runtime
            .send_snapshot(client, time, None, &mut Source { time, polls: 0 })
    );
    host.queue
        .push(SysEvent {
            time,
            kind: EventKind::Time,
        })
        .unwrap();
    host.drain(&mut Default::default());
    client_position(host, client, native_slot)
}

struct Audio {
    mixer: qa_audio::Mixer,
    client: ClientId,
    entity: SoundEmitter,
    origin: Vec3,
    count: usize,
    sound_rules: RuleSetId,
}
impl FrameSource for Audio {
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
        assert_eq!(event.client, Some(self.client));
        assert_eq!(
            event.spatial,
            qa_gameplay::rules::sound_policy(self.sound_rules)
        );
        match self.count {
            0 => {
                assert_eq!(event.entity, Some(self.entity));
                assert_eq!(event.position, self.origin);
                assert_eq!(
                    (
                        event.channel,
                        event.volume,
                        event.attenuation,
                        event.time_offset
                    ),
                    (2, 0.8, 2.0, 0.125)
                );
            }
            1 => {
                assert_eq!(event.entity, None);
                assert_eq!(event.position, self.origin);
                assert_eq!(
                    (
                        event.channel,
                        event.volume,
                        event.attenuation,
                        event.time_offset
                    ),
                    (0, 0.8, 2.0, 0.0)
                );
            }
            2 => {
                assert_eq!(event.entity, Some(emitter(self.client, 8191)));
                assert_eq!(event.position, self.origin);
                assert_eq!(
                    (
                        event.channel,
                        event.volume,
                        event.attenuation,
                        event.time_offset
                    ),
                    (2, 1.0, 1.0, 0.0)
                );
            }
            3 => {
                assert_eq!(event.entity, None);
                assert_eq!(event.position, Vec3::default());
                assert_eq!(
                    (
                        event.channel,
                        event.volume,
                        event.attenuation,
                        event.time_offset
                    ),
                    (0, 1.0, 1.0, 0.0)
                );
            }
            _ => panic!("unexpected sound"),
        }
        self.count += 1;
        self.mixer.sound(event)
    }
}

pub(super) fn received_sounds_use_precache_and_the_presentation_ring(
    host: &mut FrameHost,
    client: ClientId,
    bank: std::sync::Arc<qa_audio::Bank>,
) {
    use qa_core::primitives::NativeEntity;
    use qa_network::outputs::{Cue, Sound};
    let protocol = host.runtime.snapshots[client.0 as usize]
        .as_ref()
        .unwrap()
        .protocol;
    let entity = host.runtime.server.clients[client.0 as usize].entity;
    let server_origin = host.runtime.server.entities.columns.position[entity.slot as usize];
    let server_player_origin = host.runtime.server.clients[client.0 as usize]
        .player
        .body
        .position;
    let mut precached = Vec::new();
    host.for_each_native_sound(|name, _| {
        precached.push(name.to_vec());
        Ok(())
    })
    .unwrap();
    assert_eq!(
        precached,
        [b"_qa_cue.wav".to_vec()],
        "{protocol:?} native precache"
    );
    let origin = accept_client_resources_and_pose(host, client, &bank);
    // A later SERVER mutation has not reached CLIENT. Sound coordinates and
    // voice identity must keep using the last accepted native snapshot.
    host.runtime.server.entities.columns.position[entity.slot as usize] =
        server_origin + Vec3([512., -256., 128.]);
    host.runtime.server.clients[client.0 as usize]
        .player
        .body
        .position = server_player_origin + Vec3([512., -256., 128.]);
    assert_eq!(client_position(host, client, 1), origin);
    let output = host.runtime.server.clients[client.0 as usize]
        .output
        .unwrap();
    let submitted = host
        .runtime
        .server
        .events
        .counters(output)
        .unwrap()
        .submissions;
    let prior_sound_policy = host.runtime.client_resources[client.0 as usize]
        .as_ref()
        .unwrap()
        .state
        .sound_policy();
    host.runtime.client_resources[client.0 as usize]
        .as_mut()
        .unwrap()
        .state
        .set_sound_policy(qa_gameplay::rules::sound_policy(RuleSetId::Quake3));
    let mut audio = Audio {
        mixer: qa_audio::Mixer::load(bank, 8, 1),
        client,
        entity: emitter(client, 1),
        origin,
        count: 0,
        sound_rules: RuleSetId::Quake3,
    };
    audio.mixer.listen(
        &[Some(qa_audio::Listener {
            client,
            entity: Some(emitter(client, 1)),
            origin,
            right: Vec3([0.0, 1.0, 0.0]),
        })],
        1.0,
    );
    // A native Init registered ordinal 1. Positioned world sounds need no
    // emitter. Entity-channel pairs belong to the receiving CLIENT namespace;
    // explicit positions preserve them even without a received entity pose.
    for (index, flags, native_entity, position, expected) in [
        (1, 1 | 2 | 8 | 16, 1, Vec3::default(), 1),
        (1, 1 | 2 | 4, 0, origin, 2),
        (2, 0, 0, Vec3::default(), 2), // no registered sound
        (1, 8, 2, Vec3::default(), 2), // no received pose
        (1, 4 | 8, 8191, origin, 3),   // explicit native identity, no pose required
        (1, 0, 0, Vec3::default(), 4), // default parameters, world origin
    ] {
        send_cue(
            host,
            client,
            Cue::Sound(Sound {
                flags,
                index,
                volume: 204,
                attenuation: 128,
                time_offset: 125,
                entity: native_entity,
                channel: 2,
                position,
            }),
        );
        assert_eq!(
            (
                host.runtime.network.command_errors,
                host.runtime.network.malformed
            ),
            (0, 0),
            "received cue"
        );
        let counts = qa_app::output::dispatch(
            &mut host.runtime,
            &mut audio,
            &mut host.console.scrollback,
            &[Some(client), None, None, None],
            host.time,
            3.0,
            2.0,
        );
        assert_eq!(counts.unhandled_sounds, 0);
        assert_eq!(audio.count, expected, "{protocol:?} flags {flags}");
        assert!(host.runtime.server.events.is_empty());
        assert_eq!(
            host.runtime
                .server
                .events
                .counters(output)
                .unwrap()
                .submissions,
            submitted
        );
    }
    let mut pcm = [0; 64];
    audio.mixer.paint(&mut pcm);
    assert!(pcm.iter().any(|&value| value != 0));
    assert_eq!(audio.mixer.counts.started, 4);
    assert_eq!(
        (
            host.runtime.network.command_errors,
            host.runtime.network.malformed
        ),
        (0, 0)
    );
    // This fixture used native client zero / edict one, never the common slot.
    assert_eq!(
        host.runtime.server.entities.columns.native_entity[entity.slot as usize],
        Some(NativeEntity {
            module: ModuleId(1),
            slot: 1
        })
    );
    host.runtime.client_resources[client.0 as usize]
        .as_mut()
        .unwrap()
        .state
        .set_sound_policy(prior_sound_policy);
    host.runtime.server.entities.columns.position[entity.slot as usize] = server_origin;
    host.runtime.server.clients[client.0 as usize]
        .player
        .body
        .position = server_player_origin;
}

struct Expected {
    choices: Vec<qa_core::primitives::SoundId>,
    channel: u16,
    volume: f32,
    attenuation: f32,
    delay: f32,
}
struct MuzzleAudio {
    audio: Audio,
    expected: Vec<Expected>,
}
impl FrameSource for MuzzleAudio {
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
        let expected = &self.expected[self.audio.count];
        assert!(expected.choices.contains(&event.sound));
        assert_eq!(event.client, Some(self.audio.client));
        assert_eq!(
            event.spatial,
            qa_gameplay::rules::sound_policy(self.audio.sound_rules)
        );
        assert_eq!(event.entity, Some(self.audio.entity));
        assert_eq!(event.position, self.audio.origin);
        assert_eq!(
            (
                event.channel,
                event.volume,
                event.attenuation,
                event.time_offset
            ),
            (
                expected.channel,
                expected.volume,
                expected.attenuation,
                expected.delay
            )
        );
        self.audio.count += 1;
        self.audio.mixer.sound(event)
    }
}

pub(super) fn muzzle_packets_publish_product_sounds(
    host: &mut FrameHost,
    client: ClientId,
    bank: std::sync::Arc<qa_audio::Bank>,
) {
    use qa_network::{
        commands::packet::Protocol,
        outputs::{Cue, MuzzleFlash, MuzzleFormat},
    };
    let rr = host.runtime.snapshots[client.0 as usize]
        .as_ref()
        .unwrap()
        .protocol
        == Protocol::Quake2Repro1038;
    let entity = host.runtime.server.clients[client.0 as usize].entity;
    let server_origin = host.runtime.server.entities.columns.position[entity.slot as usize];
    let server_player_origin = host.runtime.server.clients[client.0 as usize]
        .player
        .body
        .position;
    let origin = accept_client_resources_and_pose(host, client, &bank);
    host.runtime.server.entities.columns.position[entity.slot as usize] =
        server_origin + Vec3([-1024., 512., 256.]);
    host.runtime.server.clients[client.0 as usize]
        .player
        .body
        .position = server_player_origin + Vec3([-1024., 512., 256.]);
    assert_eq!(client_position(host, client, 1), origin);
    let output = host.runtime.server.clients[client.0 as usize]
        .output
        .unwrap();
    let before = host
        .runtime
        .server
        .events
        .counters(output)
        .unwrap()
        .submissions;
    // Independently selected movement and HUD cannot change native cue semantics.
    host.runtime.server.clients[client.0 as usize]
        .player
        .movement_rules = RuleSetId::Quake3;
    host.runtime.server.clients[client.0 as usize].client_rules = RuleSetId::Quake;
    let prior_sound_policy = host.runtime.client_resources[client.0 as usize]
        .as_ref()
        .unwrap()
        .state
        .sound_policy();
    host.runtime.client_resources[client.0 as usize]
        .as_mut()
        .unwrap()
        .state
        .set_sound_policy(qa_gameplay::rules::sound_policy(RuleSetId::Quake3));
    let mut audio = MuzzleAudio {
        audio: Audio {
            mixer: qa_audio::Mixer::load(std::sync::Arc::clone(&bank), 64, 1),
            client,
            entity: emitter(client, 1),
            origin,
            count: 0,
            sound_rules: RuleSetId::Quake3,
        },
        expected: Vec::new(),
    };
    audio.audio.mixer.listen(
        &[Some(qa_audio::Listener {
            client,
            entity: Some(emitter(client, 1)),
            origin,
            right: Vec3([0.0, 1.0, 0.0]),
        })],
        1.0,
    );
    let machinegun = ["machgf1b", "machgf2b", "machgf3b", "machgf4b", "machgf5b"];
    let mut check = |format, flash, silenced, slot, expected: &[(&[&str], u16, f32, f32, f32)]| {
        for &(paths, channel, volume, attenuation, delay) in expected {
            audio.expected.push(Expected {
                choices: paths
                    .iter()
                    .map(|path| bank.find(&format!("sound/{path}.wav")).unwrap())
                    .collect(),
                channel,
                volume,
                attenuation,
                delay,
            });
        }
        send_cue(
            host,
            client,
            Cue::MuzzleFlash(MuzzleFlash {
                format,
                entity: slot,
                flash,
                silenced,
            }),
        );
        let counts = qa_app::output::dispatch(
            &mut host.runtime,
            &mut audio,
            &mut host.console.scrollback,
            &[Some(client), None, None, None],
            host.time,
            3.0,
            2.0,
        );
        assert_eq!(counts.unhandled_sounds, 0);
        assert_eq!(audio.audio.count, audio.expected.len(), "flash {flash}");
        assert!(host.runtime.server.events.is_empty());
        assert_eq!(
            host.runtime
                .server
                .events
                .counters(output)
                .unwrap()
                .submissions,
            before
        );
    };
    let player = MuzzleFormat::Player;
    let monster = if rr {
        MuzzleFormat::MonsterWide
    } else {
        MuzzleFormat::Monster
    };
    check(
        player,
        0,
        false,
        1,
        &[(&["weapons/blastf1a"], 1, 1.0, 1.0, 0.0)],
    );
    check(
        player,
        0,
        true,
        1,
        &[(
            &["weapons/blastf1a"],
            1,
            if rr { 1.0 - 0.8 } else { 0.2 },
            1.0,
            0.0,
        )],
    );
    check(
        player,
        2,
        true,
        1,
        &[
            (
                &["weapons/shotgf1b"],
                1,
                if rr { 1.0 - 0.8 } else { 0.2 },
                1.0,
                0.0,
            ),
            (
                &["weapons/shotgr1b"],
                0,
                if rr { 1.0 - 0.8 } else { 0.2 },
                1.0,
                if rr { 0.35 } else { 0.1 },
            ),
        ],
    );
    let mg = machinegun.map(|name| format!("weapons/{name}"));
    let mg: Vec<_> = mg.iter().map(String::as_str).collect();
    check(
        player,
        4,
        false,
        1,
        &[(&mg, 1, 1.0, 1.0, 0.0), (&mg, 1, 1.0, 1.0, 0.05)],
    );
    check(
        player,
        5,
        false,
        1,
        &[
            (&mg, 1, 1.0, 1.0, 0.0),
            (&mg, 1, 1.0, 1.0, 0.033),
            (&mg, 1, 1.0, 1.0, 0.066),
        ],
    );
    let mut rail = vec![(&["weapons/railgf1a"][..], 1, 1.0, 1.0, 0.0)];
    if rr {
        rail.push((&["weapons/railgr1b"], 7, 1.0, 1.0, 0.4));
    }
    check(player, 6, false, 1, &rail);
    check(
        player,
        9,
        true,
        1,
        &[(&["weapons/grenlf1a"], 1, 1.0, 1.0, 0.0)],
    );
    check(
        monster,
        26,
        false,
        1,
        &[(&["infantry/infatck1"], 1, 1.0, 1.0, 0.0)],
    );
    check(
        monster,
        74,
        false,
        1,
        &[(
            &[if rr {
                "flyer/flyatck3"
            } else {
                "infantry/infatck1"
            }],
            1,
            1.0,
            0.0,
            0.0,
        )],
    );
    check(
        monster,
        134,
        false,
        1,
        if rr {
            &[(&["flyer/flyatck3"], 1, 1.0, 0.0, 0.0)]
        } else {
            &[]
        },
    );
    if rr {
        check(
            monster,
            260,
            false,
            1,
            &[(&["infantry/infatck1"], 1, 1.0, 1.0, 0.0)],
        );
    }
    check(player, 0, false, 2, &[]); // No accepted pose for this native ordinal.
    check(player, 127, false, 1, &[]);
    check(monster, 101, false, 1, &[]); // Makron BFG has no native sound.
    let mut pcm = [0; 64];
    audio.audio.mixer.paint(&mut pcm);
    assert!(pcm.iter().any(|&sample| sample != 0));
    assert_eq!(
        audio.audio.mixer.counts.started as usize,
        audio.expected.len()
    );
    assert_eq!(
        (
            audio.audio.mixer.counts.full,
            audio.audio.mixer.counts.invalid
        ),
        (0, 0)
    );
    assert_eq!(
        (
            host.runtime.network.command_errors,
            host.runtime.network.malformed
        ),
        (0, 0)
    );
    host.runtime.client_resources[client.0 as usize]
        .as_mut()
        .unwrap()
        .state
        .set_sound_policy(prior_sound_policy);
    host.runtime.server.entities.columns.position[entity.slot as usize] = server_origin;
    host.runtime.server.clients[client.0 as usize]
        .player
        .body
        .position = server_player_origin;
}
