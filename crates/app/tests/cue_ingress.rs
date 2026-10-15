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
use std::{num::NonZeroU32, sync::Arc, time::Duration};

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
    fn sound(&mut self, event: SoundEvent) -> bool {
        if event.action == SoundAction::Play {
            self.plays += 1;
            self.last = Some(event);
        }
        self.mixer.sound(event)
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
        assert!(
            host.runtime
                .send_local_command(client, &UserCmd::default(), time)
        );
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
        let bank = Arc::new(
            Bank::load(
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
            .unwrap(),
        );
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
        host.runtime.sound_bank = Some(Arc::clone(&bank));
        host.bind_sounds(&bank).unwrap();
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
                    .apply(index, b"_qa_dynamic.wav", None, Some(&bank))
            );
        }
        let sound = bank.find("sound/_qa_dynamic.wav").unwrap();
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
            .set_sound_policy(spatial);
        let observer = host
            .runtime
            .server
            .events
            .bind(OutputTarget::Module(ModuleId(29)))
            .unwrap();
        let mut audio = Audio {
            mixer: Mixer::load(Arc::clone(&bank), 8, 1),
            plays: 0,
            last: None,
        };
        audio.mixer.listen(
            &[Some(Listener {
                client,
                entity: None,
                origin: Vec3::default(),
                right: Vec3([0.0, 1.0, 0.0]),
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
        assert!(audio.mixer.paint(&mut pcm));
        assert!(pcm.iter().any(|&sample| sample != 0));
        assert_eq!(audio.mixer.counts.started, 1);
        assert_eq!(dispatch(&mut host, &mut audio).sounds, 0);
        assert_eq!(audio.plays, 1);
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
