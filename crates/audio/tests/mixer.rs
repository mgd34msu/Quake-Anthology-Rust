use qa_audio::{Bank, Listener, Mixer};
use qa_core::primitives::*;
use std::{num::NonZeroU32, sync::Arc};

fn mixer(pcm: Vec<i16>, loop_start: Option<usize>, voices: usize) -> Mixer {
    let rate = NonZeroU32::new(44100).unwrap();
    let bank = Bank::load(
        rate,
        vec![(
            "sound/test.wav".into(),
            Pcm {
                rate,
                channels: PcmChannels::Mono,
                samples: pcm,
                loop_start,
            },
        )],
    )
    .unwrap();
    Mixer::load(Arc::new(bank), voices, 4)
}
fn listener(client: u32, origin: Vec3) -> Option<Listener> {
    Some(Listener {
        client: ClientId(client),
        entity: Some(SoundEmitter::Entity(EntityId {
            slot: client,
            generation: 1,
        })),
        origin,
        right: Vec3([1.0, 0.0, 0.0]),
    })
}
fn event(m: &Mixer) -> SoundEvent {
    SoundEvent {
        sound: m.bank.find("SOUND\\TEST.WAV").unwrap(),
        spatial: qa_gameplay::rules::sound_policy(RuleSetId::Quake),
        entity: None,
        channel: 0,
        position: Vec3::default(),
        volume: 1.0,
        attenuation: 0.0,
        action: SoundAction::Play,
        ..qa_core::primitives::SoundEvent::default()
    }
}

#[test]
fn finite_samples_finish_once_and_keep_native_fixed_gain() {
    let mut m = mixer(vec![1000, -1000], None, 2);
    m.listen(&[listener(0, Vec3::default())], 1.0);
    assert!(m.sound(event(&m)));
    let mut pcm = [0; 6];
    assert!(m.paint(&mut pcm));
    assert_eq!(pcm, [996, 996, -997, -997, 0, 0]);
    assert_eq!(m.counts.started, 1);
    assert!(!m.paint(&mut [0; 3]));
}

#[test]
fn shared_native_spatial_policies_preserve_channel_side_and_full_volume_self() {
    for (rules, right) in [
        (RuleSetId::Quake, 1224),
        (RuleSetId::QuakeWorld, 1224),
        (RuleSetId::Quake2, 692),
        (RuleSetId::Quake2Rerelease, 692),
        (RuleSetId::Quake3, 740),
    ] {
        let mut m = mixer(vec![1024], None, 2);
        m.listen(&[listener(0, Vec3::default())], 1.0);
        let mut e = event(&m);
        e.spatial = qa_gameplay::rules::sound_policy(rules);
        e.position = Vec3([400.0, 0.0, 0.0]);
        e.attenuation = 1.0;
        assert!(m.sound(e));
        let mut pcm = [0; 2];
        m.paint(&mut pcm);
        assert_eq!(pcm[0], 0);
        assert_eq!(pcm[1], right, "{rules:?}");
        e.entity = Some(SoundEmitter::Entity(EntityId {
            slot: 0,
            generation: 1,
        }));
        assert!(m.sound(e));
        m.paint(&mut pcm);
        assert_eq!(pcm, [1020; 2]);
    }
}

#[test]
fn one_precached_pcm_supports_independent_spatial_policies_on_simultaneous_voices() {
    let mut m = mixer(vec![1024; 2], None, 2);
    m.listen(&[listener(0, Vec3::default())], 1.0);
    let mut e = event(&m);
    e.position = Vec3([400.0, 0.0, 0.0]);
    e.attenuation = 1.0;
    for rules in [RuleSetId::Quake, RuleSetId::Quake2] {
        e.spatial = qa_gameplay::rules::sound_policy(rules);
        assert!(m.sound(e));
    }
    let mut pcm = [0; 2];
    assert!(m.paint(&mut pcm));
    assert_eq!(pcm, [0, 1224 + 692]);
    assert_eq!(m.counts.started, 2);
    assert_eq!(m.counts.invalid, 0);
}

#[test]
fn updating_loop_spatial_policy_preserves_its_pcm_cursor() {
    let mut m = mixer(vec![1024, 2048, 3072], None, 1);
    m.listen(&[listener(0, Vec3::default())], 1.0);
    let mut e = event(&m);
    e.entity = Some(SoundEmitter::Entity(EntityId {
        slot: 9,
        generation: 1,
    }));
    e.channel = 2;
    e.position = Vec3([400.0, 0.0, 0.0]);
    e.attenuation = 1.0;
    e.action = SoundAction::StartLoop;
    let mut pcm = [0; 2];
    assert!(m.sound(e));
    assert!(m.paint(&mut pcm));
    assert_eq!(pcm, [0, 1224]);
    e.spatial = qa_gameplay::rules::sound_policy(RuleSetId::Quake2);
    assert!(m.sound(e));
    assert!(m.paint(&mut pcm));
    assert_eq!(pcm, [0, 2 * 692]);
    assert_eq!(m.counts.started, 1);
    assert_eq!(m.counts.replaced, 0);
}

#[test]
fn spatialization_uses_capability_data_without_a_game_selector() {
    let mut m = mixer(vec![1024], None, 1);
    m.listen(&[listener(0, Vec3::default())], 1.0);
    let mut e = event(&m);
    e.spatial = SpatialPolicy {
        distance_offset: 80.0,
        distance_scale: 0.001,
        separation: 0.25,
    };
    e.position = Vec3([400.0, 0.0, 0.0]);
    e.attenuation = 1.0;
    assert!(m.sound(e));
    let mut pcm = [0; 2];
    assert!(m.paint(&mut pcm));
    assert_eq!(pcm, [0, 344]);
}

#[test]
fn listeners_are_independent_and_duplicate_views_do_not_increase_gain() {
    let mut m = mixer(vec![1024], None, 2);
    let e = event(&m);
    m.listen(
        &[listener(0, Vec3::default()), listener(0, Vec3::default())],
        1.0,
    );
    m.sound(e);
    let mut a = [0; 2];
    m.paint(&mut a);
    m.listen(
        &[listener(0, Vec3::default()), listener(1, Vec3::default())],
        1.0,
    );
    m.sound(e);
    let mut b = [0; 2];
    m.paint(&mut b);
    assert_eq!(a, b);
    m.listen(&[], 1.0);
    m.sound(e);
    m.paint(&mut b);
    assert_eq!(b, [0; 2]);
}

#[test]
fn looping_replacement_stop_generation_and_full_voice_table() {
    let mut m = mixer(vec![256, 512, 768], Some(1), 1);
    m.listen(&[listener(0, Vec3::default())], 1.0);
    let mut e = event(&m);
    e.entity = Some(SoundEmitter::Entity(EntityId {
        slot: 9,
        generation: 1,
    }));
    e.channel = 2;
    assert!(m.sound(e));
    assert!(m.sound(e));
    assert_eq!(m.counts.replaced, 1);
    let mut pcm = [0; 10];
    m.paint(&mut pcm);
    assert_eq!(pcm, [255, 255, 510, 510, 765, 765, 510, 510, 765, 765]);
    let mut other = e;
    other.entity = Some(SoundEmitter::Entity(EntityId {
        slot: 9,
        generation: 2,
    }));
    assert!(!m.sound(other));
    assert_eq!(m.counts.full, 1);
    other.action = SoundAction::Stop;
    m.sound(other);
    m.paint(&mut pcm);
    assert!(pcm.iter().any(|v| *v != 0));
    e.action = SoundAction::Stop;
    m.sound(e);
    m.paint(&mut pcm);
    assert_eq!(pcm, [0; 10]);
    assert_eq!(m.counts.stopped, 1);
}

#[test]
fn repeated_loop_updates_keep_the_sample_clock() {
    let mut m = mixer(vec![256, 512, 768], None, 1);
    m.listen(&[listener(0, Vec3::default())], 1.0);
    let mut e = event(&m);
    e.action = SoundAction::StartLoop;
    let mut pcm = [0; 2];
    assert!(m.sound(e));
    m.paint(&mut pcm);
    assert_eq!(pcm, [255; 2]);
    assert!(m.sound(e));
    m.paint(&mut pcm);
    assert_eq!(pcm, [510; 2]);
    assert_eq!(m.counts.started, 1);
}

#[test]
fn invalid_event_does_not_replace_live_voice_or_grow_storage() {
    let mut m = mixer(vec![1024], None, 1);
    m.listen(&[listener(0, Vec3::default())], 1.0);
    let e = event(&m);
    assert!(m.sound(e));
    for volume in [-1.0, 2.0, f32::NAN] {
        let mut bad = e;
        bad.volume = volume;
        assert!(!m.sound(bad));
    }
    let mut pcm = [0; 2];
    m.paint(&mut pcm);
    assert_eq!(pcm, [1020; 2]);
    assert_eq!(m.counts.invalid, 3);
}

#[test]
fn native_delay_and_private_client_gain_use_the_event() {
    let mut m = mixer(vec![1024], None, 2);
    m.listen(
        &[listener(0, Vec3::default()), listener(1, Vec3::default())],
        1.0,
    );
    let mut e = event(&m);
    e.client = Some(ClientId(1));
    e.time_offset = 2.0 / 44100.0;
    assert!(m.sound(e));
    let mut pcm = [0; 6];
    m.paint(&mut pcm);
    assert_eq!(pcm, [0, 0, 0, 0, 1020, 1020]);
    e.client = Some(ClientId(9));
    e.time_offset = 0.0;
    assert!(m.sound(e));
    m.paint(&mut pcm);
    assert_eq!(pcm, [0; 6]);
}

#[test]
fn private_channels_do_not_replace_another_clients_voice() {
    let mut m = mixer(vec![1024], None, 2);
    m.listen(
        &[listener(0, Vec3::default()), listener(1, Vec3::default())],
        1.0,
    );
    let mut e = event(&m);
    e.entity = Some(SoundEmitter::Entity(EntityId {
        slot: 3,
        generation: 1,
    }));
    e.channel = 1;
    for client in [0, 1] {
        e.client = Some(ClientId(client));
        assert!(m.sound(e));
    }
    assert_eq!(m.counts.replaced, 0);
    let mut pcm = [0; 2];
    m.paint(&mut pcm);
    assert_eq!(pcm, [2040; 2]);
}

#[test]
fn delayed_weapon_bursts_replace_at_their_sample_time() {
    let mut m = mixer(vec![256, 512, 768, 1024, 1280], None, 4);
    m.listen(&[listener(0, Vec3::default())], 1.0);
    let mut e = event(&m);
    e.entity = Some(SoundEmitter::Entity(EntityId {
        slot: 0,
        generation: 1,
    }));
    e.channel = 1;
    assert!(m.sound(e));
    e.time_offset = 2.0 / 44100.0;
    assert!(m.sound(e));
    e.time_offset = 4.0 / 44100.0;
    assert!(m.sound(e));
    assert_eq!(m.counts.replaced, 0);
    let mut first = [0; 6];
    let mut second = [0; 8];
    m.paint(&mut first);
    m.paint(&mut second);
    assert_eq!(first, [255, 255, 510, 510, 255, 255]);
    assert_eq!(second, [510, 510, 255, 255, 510, 510, 765, 765]);
    assert_eq!(m.counts.replaced, 2);
}

#[test]
fn equal_time_pending_starts_keep_publication_order_and_stop_cancels_them() {
    let mut m = mixer(vec![1024; 8], None, 4);
    m.listen(&[listener(0, Vec3::default())], 1.0);
    let mut e = event(&m);
    e.entity = Some(SoundEmitter::Entity(EntityId {
        slot: 0,
        generation: 1,
    }));
    e.channel = 1;
    e.time_offset = 2.0 / 44100.0;
    assert!(m.sound(e));
    e.volume = 0.5;
    assert!(m.sound(e));
    let mut pcm = [0; 6];
    m.paint(&mut pcm);
    assert_eq!(pcm, [0, 0, 0, 0, 508, 508]);
    assert_eq!(m.counts.replaced, 1);
    e.time_offset = 4.0 / 44100.0;
    assert!(m.sound(e));
    e.action = SoundAction::Stop;
    m.sound(e);
    m.paint(&mut [0; 16]);
    m.paint(&mut pcm);
    assert_eq!(pcm, [0; 6]);
    assert_eq!(m.counts.stopped, 2);
}

fn native_emitter(namespace: ResourceNamespace, slot: i32) -> SoundEmitter {
    SoundEmitter::Native { namespace, slot }
}

#[test]
fn native_channels_keep_client_namespaces_slots_and_private_recipients_distinct() {
    let mut m = mixer(vec![1024; 2], Some(0), 6);
    m.listen(&[listener(0, Vec3::default())], 1.0);
    let emitters = [
        native_emitter(ResourceNamespace::Client(ClientId(0)), 1),
        native_emitter(ResourceNamespace::Client(ClientId(1)), 1),
        native_emitter(ResourceNamespace::Module(ModuleId(0)), 1),
        native_emitter(ResourceNamespace::Client(ClientId(0)), 2),
    ];
    let mut e = event(&m);
    e.action = SoundAction::StartLoop;
    e.channel = 1;
    for emitter in emitters {
        e.entity = Some(emitter);
        assert!(m.sound(e));
    }
    e.entity = Some(emitters[0]);
    for client in [0, 1] {
        e.client = Some(ClientId(client));
        assert!(m.sound(e));
    }
    assert_eq!(m.counts.started, 6);
    assert_eq!(m.counts.replaced, 0);
    assert!(m.sound(e)); // The same native voice updates without restarting.
    assert_eq!(m.counts.started, 6);
    e.action = SoundAction::Stop;
    assert!(m.sound(e));
    assert_eq!(m.counts.stopped, 1);
    e.client = Some(ClientId(2));
    assert!(m.sound(e));
    assert_eq!(m.counts.stopped, 1);
    e.client = None;
    assert!(m.sound(e));
    assert_eq!(m.counts.stopped, 2);
    e.entity = Some(emitters[1]);
    assert!(m.sound(e));
    assert_eq!(m.counts.stopped, 3);
}

#[test]
fn native_self_relative_gain_uses_the_client_voice_namespace() {
    let mut m = mixer(vec![1024], None, 1);
    let own = native_emitter(ResourceNamespace::Client(ClientId(3)), 0);
    m.listen(
        &[Some(Listener {
            client: ClientId(3),
            entity: Some(own),
            origin: Vec3::default(),
            right: Vec3([1.0, 0.0, 0.0]),
        })],
        1.0,
    );
    let mut e = event(&m);
    e.entity = Some(own);
    e.spatial = qa_gameplay::rules::sound_policy(RuleSetId::Quake2);
    e.position = Vec3([400.0, 0.0, 0.0]);
    e.attenuation = 1.0;
    let mut pcm = [0; 2];
    assert!(m.sound(e));
    assert!(m.paint(&mut pcm));
    assert_eq!(pcm, [1020; 2]);
    for emitter in [
        native_emitter(ResourceNamespace::Client(ClientId(4)), 0),
        native_emitter(ResourceNamespace::Module(ModuleId(3)), 0),
        native_emitter(ResourceNamespace::Client(ClientId(3)), 1),
    ] {
        e.entity = Some(emitter);
        assert!(m.sound(e));
        assert!(m.paint(&mut pcm));
        assert_eq!(pcm, [0, 692]);
    }
}

#[test]
fn native_client_voice_tracks_without_restarting_and_retains_missing_pose() {
    let mut m = mixer(vec![1024; 4], None, 1);
    m.listen(&[listener(7, Vec3::default())], 1.0);
    let mut e = event(&m);
    e.spatial = qa_gameplay::rules::sound_policy(RuleSetId::Quake2);
    e.entity = Some(SoundEmitter::Native {
        namespace: ResourceNamespace::Client(ClientId(7)),
        slot: 19,
    });
    e.client = Some(ClientId(7));
    e.position = Vec3([400.0, 0.0, 0.0]);
    e.attenuation = 1.0;
    assert!(m.sound(e));
    let mut pcm = [0; 2];
    assert!(m.paint(&mut pcm));
    assert_eq!(pcm, [0, 692]);
    m.respatialize(|client, number, origin| {
        assert_eq!(origin, None);
        assert_eq!((client, number), (ClientId(7), 19));
        Some(Vec3([-400.0, 0.0, 0.0]))
    });
    assert!(m.paint(&mut pcm));
    assert_eq!(pcm, [692, 0]);
    assert_eq!(m.counts.started, 1);
    assert_eq!(m.counts.respatialized, 1);
    m.respatialize(|_, _, _| None);
    assert!(m.paint(&mut pcm));
    assert_eq!(pcm, [692, 0]);
    assert_eq!(m.counts.missing_origins, 1);
    m.respatialize(|_, _, _| Some(Vec3([f32::NAN, 0.0, 0.0])));
    assert!(m.paint(&mut pcm));
    assert_eq!(pcm, [692, 0]);
    assert_eq!(m.counts.missing_origins, 2);
}

#[test]
fn fixed_and_non_client_emitters_never_resolve_client_poses() {
    for (positioned, entity) in [
        (
            true,
            Some(SoundEmitter::Native {
                namespace: ResourceNamespace::Client(ClientId(7)),
                slot: 19,
            }),
        ),
        (
            false,
            Some(SoundEmitter::Native {
                namespace: ResourceNamespace::Module(ModuleId(7)),
                slot: 19,
            }),
        ),
        (
            false,
            Some(SoundEmitter::Entity(EntityId {
                slot: 19,
                generation: 1,
            })),
        ),
        (false, None),
    ] {
        let mut m = mixer(vec![1024; 2], None, 1);
        m.listen(&[listener(7, Vec3::default())], 1.0);
        let mut e = event(&m);
        e.spatial = qa_gameplay::rules::sound_policy(RuleSetId::Quake2);
        e.entity = entity;
        e.position = Vec3([400.0, 0.0, 0.0]);
        e.positioned = positioned;
        e.attenuation = 1.0;
        assert!(m.sound(e));
        m.respatialize(|_, _, _| panic!("fixed/non-client origin queried"));
        let mut pcm = [0; 2];
        assert!(m.paint(&mut pcm));
        assert_eq!(pcm, [0, 692]);
        assert_eq!(m.counts.respatialized, 0);
        assert_eq!(m.counts.missing_origins, 0);
    }
}

#[test]
fn moving_voices_keep_native_connection_namespace() {
    let mut m = mixer(vec![1024], None, 2);
    m.listen(
        &[listener(7, Vec3::default()), listener(9, Vec3::default())],
        1.0,
    );
    for client in [ClientId(7), ClientId(9)] {
        let mut e = event(&m);
        e.spatial = qa_gameplay::rules::sound_policy(RuleSetId::Quake2);
        e.entity = Some(SoundEmitter::Native {
            namespace: ResourceNamespace::Client(client),
            slot: 19,
        });
        e.client = Some(client);
        e.attenuation = 1.0;
        assert!(m.sound(e));
    }
    let mut queried = [false; 2];
    m.respatialize(|client, number, origin| {
        assert_eq!(origin, None);
        assert_eq!(number, 19);
        let index = match client.0 {
            7 => 0,
            9 => 1,
            _ => panic!("wrong CLIENT namespace"),
        };
        queried[index] = true;
        Some(Vec3([if index == 0 { 400.0 } else { -400.0 }, 0.0, 0.0]))
    });
    assert_eq!(queried, [true; 2]);
    let mut pcm = [0; 2];
    assert!(m.paint(&mut pcm));
    assert_eq!(pcm, [692, 692]);
    assert_eq!(m.counts.respatialized, 2);
}

#[test]
fn retirement_removes_only_the_old_client_lifetime_including_delayed_voices() {
    let mut m = mixer(vec![1000; 4], Some(0), 8);
    let old = EntityId {
        slot: 1,
        generation: 4,
    };
    let replacement = EntityId {
        generation: 5,
        ..old
    };
    m.listen(
        &[listener(0, Vec3::default()), listener(1, Vec3::default())],
        1.0,
    );
    let mut old_event = event(&m);
    old_event.client = Some(ClientId(0));
    old_event.client_lifetime = Some(old);
    old_event.action = SoundAction::StartLoop;
    assert!(m.sound(old_event));
    old_event.channel += 1;
    old_event.time_offset = 1.0;
    assert!(m.sound(old_event));
    let mut fresh = event(&m);
    fresh.client = Some(ClientId(0));
    fresh.client_lifetime = Some(replacement);
    fresh.action = SoundAction::StartLoop;
    assert!(m.sound(fresh));
    let mut other = fresh;
    other.client = Some(ClientId(1));
    assert!(m.sound(other));
    let mut global = fresh;
    global.client = None;
    global.client_lifetime = None;
    assert!(m.sound(global));
    assert!(m.sound(SoundEvent::retire_client(ClientId(0), old)));
    assert_eq!(m.counts.stopped, 2);
    let mut pcm = [0; 4];
    assert!(m.paint(&mut pcm));
    assert!(pcm.iter().all(|sample| *sample > 0));
    assert!(m.sound(SoundEvent::retire_client(ClientId(0), old)));
    assert_eq!(m.counts.stopped, 2);
    assert!(m.sound(SoundEvent::retire_client(ClientId(0), replacement)));
    assert!(m.sound(SoundEvent::retire_client(ClientId(1), replacement)));
    assert_eq!(m.counts.stopped, 4);
    assert!(m.paint(&mut pcm));
    assert!(
        pcm.iter().all(|sample| *sample > 0),
        "global console play survives"
    );
    global.action = SoundAction::Stop;
    assert!(m.sound(global));
    assert!(m.paint(&mut pcm));
    assert_eq!(pcm, [0; 4]);
}

#[test]
fn reused_native_channel_does_not_replace_a_different_client_lifetime() {
    let mut m = mixer(vec![1000; 4], Some(0), 4);
    m.listen(&[listener(0, Vec3::default())], 1.0);
    let old = EntityId {
        slot: 8,
        generation: 1,
    };
    let fresh = EntityId {
        generation: 2,
        ..old
    };
    let mut sound = event(&m);
    sound.client = Some(ClientId(0));
    sound.client_lifetime = Some(old);
    sound.entity = Some(SoundEmitter::Native {
        namespace: ResourceNamespace::Client(ClientId(0)),
        slot: 1,
    });
    sound.channel = 2;
    sound.action = SoundAction::StartLoop;
    assert!(m.sound(sound));
    sound.client_lifetime = Some(fresh);
    assert!(m.sound(sound));
    assert_eq!((m.counts.started, m.counts.replaced), (2, 0));
    assert!(m.sound(SoundEvent::retire_client(ClientId(0), old)));
    assert_eq!(m.counts.stopped, 1);
    let mut pcm = [0; 4];
    assert!(m.paint(&mut pcm));
    assert!(pcm.iter().all(|sample| *sample > 0));
    assert!(m.sound(SoundEvent::retire_client(ClientId(0), fresh)));
    assert!(m.paint(&mut pcm));
    assert_eq!(pcm, [0; 4]);
}
