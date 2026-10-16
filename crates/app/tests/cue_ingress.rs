#[path = "support/frame_source.rs"]
mod frame_source;

use qa_app::{
    Runtime,
    client_policy::ClientPolicy,
    host::{FrameHost, FrameResult, FrameSource},
    map::SpawnAnchor,
};
use qa_audio::{Bank, Listener, Mixer};
use qa_console::{commands::Console, views::Context};
use qa_core::{
    events::{FrameEvent, OutputTarget},
    loopback::Endpoint,
    primitives::*,
    sys_events::*,
};
use qa_network::{commands::packet::Protocol, outputs};
use qa_session::timing::TickRate;
use std::{num::NonZeroU32, time::Duration};

struct Audio {
    mixer: Mixer,
    plays: usize,
    last: Option<SoundEvent>,
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
    fn sound(&mut self, _bank: Option<&qa_audio::Bank>, event: SoundEvent) -> bool {
        if event.action == SoundAction::Play {
            self.plays += 1;
            self.last = Some(event);
        }
        self.mixer.sound(_bank.unwrap(), event)
    }
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

fn send(host: &mut FrameHost, client: ClientId, payload: &[u8], time: EventTime) {
    if host.runtime.snapshots[client.0 as usize]
        .as_ref()
        .unwrap()
        .protocol
        == Protocol::QuakeWorld28
    {
        assert!(host.runtime.send_client_command(
            client,
            &UserCmd::default(),
            time,
            &mut frame_source::Sink::default()
        ));
        drain(host, time);
    }
    let runtime = &mut host.runtime;
    let channel = &mut runtime
        .network
        .get_mut(client, Endpoint::Server)
        .unwrap()
        .channel;
    assert!(channel.prepare(Some(payload), time).unwrap().is_some());
    assert!(
        channel
            .submit_with(time, |bytes| runtime
                .loopback
                .send(Endpoint::Server, client, bytes)
                .is_ok())
            .unwrap()
            .is_some()
    );
    assert!(
        runtime
            .loopback
            .pending_client(Endpoint::Client, client)
            .unwrap()
            != 0
    );
    drain(host, time);
}

fn dispatch(host: &mut FrameHost, source: &mut Audio) -> qa_app::output::OutputCounts {
    qa_app::output::dispatch(
        &mut host.runtime,
        source,
        &mut host.console.scrollback,
        &host.local_clients,
        host.time,
        3.0,
        2.0,
    )
}

#[test]
fn native_dynamic_sound_packets_reach_the_one_ring_and_real_pcm_for_nq_and_qw() {
    let rate = NonZeroU32::new(44100).unwrap();
    for protocol in [Protocol::NetQuake15, Protocol::QuakeWorld28] {
        let bank = Bank::load(
            rate,
            vec![(
                "sound/_qa_dynamic.wav".into(),
                Pcm {
                    rate,
                    channels: PcmChannels::Mono,
                    samples: [6000, -3000, 9000, -7000].repeat(64),
                    loop_start: None,
                    loop_end: None,
                    loop_invalid: false,
                },
            )],
        )
        .unwrap();
        let mut runtime = Runtime::load(1, []).unwrap();
        let client = runtime
            .connect_local(
                SeatId::FIRST,
                SpawnAnchor {
                    position: Vec3::default(),
                    angles: Vec3::default(),
                    entity: 0,
                    fixture_fallback: false,
                },
                ClientPolicy {
                    input: protocol.rule_set(),
                    client: protocol.rule_set(),
                    movement: RuleSetId::Quake3,
                    trace: RuleSetId::QuakeWorld,
                },
                protocol,
                0,
            )
            .unwrap();
        let lifetime = runtime.server.clients[client.0 as usize].entity;
        let mut host = FrameHost::load(
            Console::new(Context::default()).unwrap(),
            runtime,
            TickRate::FrameDriven,
            vec![],
        )
        .unwrap();
        host.local_clients[0] = Some(client);
        host.runtime.sound_assets = Some(Box::new(qa_app::audio::SoundAssets::from_bank(bank)));
        host.bind_sounds().unwrap();
        let index = host.runtime.client_resources[client.0 as usize]
            .as_ref()
            .unwrap()
            .state
            .layout
            .resources[1]
            .get(1)
            .unwrap();
        let time = EventTime::from_milliseconds(137);
        if protocol == Protocol::QuakeWorld28 {
            let mut config = [0; 128];
            let count =
                outputs::configstring(protocol, index as u32, b"_qa_dynamic.wav", &mut config)
                    .unwrap();
            send(&mut host, client, &config[..count], time);
        } else {
            // This slice starts after NQ signon's precache admission. Its
            // serverinfo reader remains separate; use the real cold binding.
            assert!(
                host.runtime.client_resources[client.0 as usize]
                    .as_mut()
                    .unwrap()
                    .apply(
                        index,
                        b"_qa_dynamic.wav",
                        None,
                        host.runtime
                            .sound_assets
                            .as_deref()
                            .map(qa_app::audio::SoundAssets::bank)
                    )
            );
        }
        let sound = host
            .runtime
            .sound_bank()
            .unwrap()
            .find("sound/_qa_dynamic.wav")
            .unwrap();
        assert_eq!(
            host.runtime.client_resources[client.0 as usize]
                .as_ref()
                .unwrap()
                .state
                .sound(1),
            Some(sound)
        );
        let spatial = qa_gameplay::rules::sound_policy(RuleSetId::Quake2Rerelease);
        host.runtime.client_resources[client.0 as usize]
            .as_mut()
            .unwrap()
            .state
            .set_sound_policy(
                spatial,
                qa_gameplay::rules::loop_spatial_policy(RuleSetId::Quake2Rerelease),
            );
        let observer = host
            .runtime
            .server
            .events
            .bind(OutputTarget::Module(ModuleId(29)))
            .unwrap();
        let mut audio = Audio {
            mixer: Mixer::load(rate, 8, 1),
            plays: 0,
            last: None,
        };
        audio.mixer.listen(
            &[Some(Listener {
                client,
                entity: None,
                origin: Vec3::default(),
                right: Vec3([1.0, 0.0, 0.0]),
                negate_dot: false,
            })],
            1.0,
        );
        let body: &[u8] = if protocol == Protocol::NetQuake15 {
            &[6, 3, 204, 128, 2, 0, 1, 247, 255, 162, 0, 1, 4]
        } else {
            &[6, 2, 192, 204, 128, 1, 247, 255, 162, 0, 1, 4]
        };
        send(&mut host, client, body, time);
        assert_eq!(audio.plays, 0);
        let presentation = host.runtime.server.presentation;
        let mut batch = host.runtime.server.events.batch(presentation).unwrap();
        let mut queued = 0;
        while let Some(record) = host.runtime.server.events.next(&mut batch) {
            if let FrameEvent::Sound(event) = record.event
                && event.action == SoundAction::Play
            {
                queued += 1;
                assert_eq!(event.sound, sound);
                assert_eq!(event.client_lifetime, Some(lifetime));
                assert!(record.delivery.is_none());
            }
        }
        assert_eq!(queued, 1);
        assert!(
            host.runtime
                .server
                .events
                .next(&mut host.runtime.server.events.batch(observer).unwrap())
                .is_none()
        );
        let submitted = dispatch(&mut host, &mut audio);
        assert_eq!((submitted.sounds, submitted.unhandled_sounds), (1, 0));
        assert_eq!(audio.plays, 1);
        let event = audio.last.unwrap();
        assert_eq!(
            event.entity,
            Some(SoundEmitter::Native {
                namespace: ResourceNamespace::Client(client),
                slot: 0
            })
        );
        assert_eq!(
            (event.client, event.source_ordinal, event.channel),
            (Some(client), Some(1), 2)
        );
        assert_eq!(event.position, Vec3([-1.125, 20.25, 128.125]));
        assert_eq!(
            (
                event.volume,
                event.attenuation,
                event.time_offset,
                event.positioned
            ),
            (0.8, 2.0, 0.0, true)
        );
        assert_eq!(event.spatial, spatial);
        let mut pcm = [0; 64];
        assert!(
            audio
                .mixer
                .paint(host.runtime.sound_bank().unwrap(), &mut pcm)
        );
        assert!(pcm.iter().any(|&sample| sample != 0));
        assert_eq!(audio.mixer.counts.started, 1);
        assert_eq!(dispatch(&mut host, &mut audio).sounds, 0);
        assert_eq!(audio.plays, 1);
        for presentation in [
            RuleSetId::Quake,
            RuleSetId::Quake2,
            RuleSetId::Quake2Rerelease,
        ] {
            let ordinary = qa_gameplay::rules::sound_policy(presentation);
            let entity_loop = qa_gameplay::rules::loop_spatial_policy(presentation);
            let resources = &mut host.runtime.client_resources[client.0 as usize]
                .as_mut()
                .unwrap()
                .state;
            resources.set_sound_policy(ordinary, entity_loop);
            assert_eq!(resources.loop_spatial_policy(), entity_loop);
            for attenuation_byte in [64, 192] {
                // Explicit volume, attenuation and world0: source at [400,0,0].
                let bytes = if protocol == Protocol::NetQuake15 {
                    [6, 3, 255, attenuation_byte, 2, 0, 1, 128, 12, 0, 0, 0, 0]
                } else {
                    [6, 2, 192, 255, attenuation_byte, 1, 128, 12, 0, 0, 0, 0, 0]
                };
                let body = &bytes[..if protocol == Protocol::NetQuake15 {
                    13
                } else {
                    12
                }];
                send(&mut host, client, body, time);
                let submitted = dispatch(&mut host, &mut audio);
                assert_eq!((submitted.sounds, submitted.unhandled_sounds), (1, 0));
                let event = audio.last.unwrap();
                assert_eq!(event.spatial, ordinary);
                assert_eq!(event.native_fields.unwrap().attenuation, attenuation_byte);
                let mut encoded = [0; 16];
                let length =
                    outputs::sound(protocol, event, 1, Some(0), true, &mut encoded).unwrap();
                assert_eq!(&encoded[..length], body);
                let attenuation = f32::from(attenuation_byte) / 64.0;
                // Source equations, independent of DistanceAttenuation::apply.
                let distance = match presentation {
                    RuleSetId::Quake => 400.0 * attenuation * 0.001,
                    RuleSetId::Quake2 => {
                        let coefficient = if attenuation_byte == 192 {
                            (f64::from(attenuation) * 0.001) as f32
                        } else {
                            (f64::from(attenuation) * 0.0005) as f32
                        };
                        320.0 * coefficient
                    }
                    _ => {
                        let coefficient = if attenuation_byte == 192 {
                            attenuation * 0.001f32
                        } else {
                            attenuation * 0.0005f32
                        };
                        320.0 * coefficient
                    }
                };
                let gain = (255.0 * (1.0 - distance) * 2.0 * ordinary.separation).max(0.0) as i64;
                let mut pcm = [0; 2];
                assert!(
                    audio
                        .mixer
                        .paint(host.runtime.sound_bank().unwrap(), &mut pcm)
                );
                assert_eq!(
                    pcm,
                    [0, ((6000 * gain) >> 8) as i16],
                    "{protocol:?} {presentation:?} {attenuation_byte}"
                );
            }
        }
        assert_eq!(audio.plays, 7);
        assert_eq!(
            (
                host.runtime.network.command_errors,
                host.runtime.network.malformed
            ),
            (0, 0)
        );
        assert_eq!(
            (
                host.runtime.prediction[0].player.movement_rules,
                host.runtime.prediction[0].player.trace_rules
            ),
            (RuleSetId::Quake3, RuleSetId::QuakeWorld)
        );
        host.runtime.server.events.unbind(observer);
    }
}
