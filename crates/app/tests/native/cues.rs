use super::*;
use qa_core::primitives::{ClientId, EntityId, SoundEvent, Vec3};

struct Audio {
    mixer: qa_audio::Mixer,
    client: ClientId,
    entity: EntityId,
    origin: Vec3,
    count: usize,
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
    use qa_core::{loopback::Endpoint, primitives::NativeEntity};
    use qa_network::outputs::{Cue, Sound};
    let protocol = host.runtime.local_snapshots[client.0 as usize]
        .as_ref()
        .unwrap()
        .protocol;
    let entity = host.runtime.server.clients[client.0 as usize].entity;
    let origin = host.runtime.server.entities.columns.position[entity.slot as usize];
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
    let mut audio = Audio {
        mixer: qa_audio::Mixer::load(bank, 8, 1),
        client,
        entity,
        origin,
        count: 0,
    };
    audio.mixer.listen(
        &[Some(qa_audio::Listener {
            client,
            entity: Some(entity),
            origin,
            right: Vec3([0.0, 1.0, 0.0]),
        })],
        1.0,
    );
    let mut body = [0; 64];
    // A native Init registered ordinal 1. Positioned world sounds need no
    // emitter. Entity-channel pairs retain the exact mapped lifetime.
    for (index, flags, native_entity, position, expected) in [
        (1, 1 | 2 | 8 | 16, 1, Vec3::default(), 1),
        (1, 1 | 2 | 4, 0, origin, 2),
        (2, 0, 0, Vec3::default(), 2), // no registered sound
        (1, 8, 2, Vec3::default(), 2), // no bound entity
        (1, 4 | 8, 8191, origin, 2),   // no channel identity, even with position
        (1, 0, 0, Vec3::default(), 3), // default parameters, world origin
    ] {
        let n = qa_network::outputs::write_cue(
            protocol,
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
            &mut body,
        )
        .unwrap();
        let channel = &mut host
            .runtime
            .network
            .get_mut(client, Endpoint::Server)
            .unwrap()
            .channel;
        let packet = channel
            .prepare(Some(&body[..n]), host.time)
            .unwrap()
            .unwrap()
            .bytes
            .to_vec();
        host.runtime
            .loopback
            .send(Endpoint::Server, client, &packet)
            .unwrap();
        channel.submitted(host.time).unwrap();
        host.drain(&mut Default::default());
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
    assert_eq!(audio.mixer.counts.started, 3);
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
}
