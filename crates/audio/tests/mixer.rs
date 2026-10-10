use qa_audio::{Bank, Listener, Mixer};
use qa_core::primitives::*;
use std::{num::NonZeroU32, sync::Arc};

fn mixer(rules: RuleSetId, pcm: Vec<i16>, loop_start: Option<usize>, voices: usize) -> Mixer {
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
            rules,
        )],
    )
    .unwrap();
    Mixer::load(Arc::new(bank), voices, 4)
}
fn listener(client: u32, origin: Vec3) -> Option<Listener> {
    Some(Listener {
        client: ClientId(client),
        entity: Some(EntityId {
            slot: client,
            generation: 1,
        }),
        origin,
        right: Vec3([1.0, 0.0, 0.0]),
    })
}
fn event(m: &Mixer) -> SoundEvent {
    SoundEvent {
        sound: m.bank.find("SOUND\\TEST.WAV").unwrap(),
        entity: None,
        channel: 0,
        position: Vec3::default(),
        volume: 1.0,
        attenuation: 0.0,
        action: SoundAction::Play,
    }
}

#[test]
fn finite_samples_finish_once_and_keep_native_fixed_gain() {
    let mut m = mixer(RuleSetId::Quake, vec![1000, -1000], None, 2);
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
        (RuleSetId::Quake2, 692),
        (RuleSetId::Quake3, 740),
    ] {
        let mut m = mixer(rules, vec![1024], None, 2);
        m.listen(&[listener(0, Vec3::default())], 1.0);
        let mut e = event(&m);
        e.position = Vec3([400.0, 0.0, 0.0]);
        e.attenuation = 1.0;
        assert!(m.sound(e));
        let mut pcm = [0; 2];
        m.paint(&mut pcm);
        assert_eq!(pcm[0], 0);
        assert_eq!(pcm[1], right, "{rules:?}");
        e.entity = Some(EntityId {
            slot: 0,
            generation: 1,
        });
        assert!(m.sound(e));
        m.paint(&mut pcm);
        assert_eq!(pcm, [1020; 2]);
    }
}

#[test]
fn listeners_are_independent_and_duplicate_views_do_not_increase_gain() {
    let mut m = mixer(RuleSetId::Quake, vec![1024], None, 2);
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
    let mut m = mixer(RuleSetId::Quake, vec![256, 512, 768], Some(1), 1);
    m.listen(&[listener(0, Vec3::default())], 1.0);
    let mut e = event(&m);
    e.entity = Some(EntityId {
        slot: 9,
        generation: 1,
    });
    e.channel = 2;
    assert!(m.sound(e));
    assert!(m.sound(e));
    assert_eq!(m.counts.replaced, 1);
    let mut pcm = [0; 10];
    m.paint(&mut pcm);
    assert_eq!(pcm, [255, 255, 510, 510, 765, 765, 510, 510, 765, 765]);
    let mut other = e;
    other.entity = Some(EntityId {
        slot: 9,
        generation: 2,
    });
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
    let mut m = mixer(RuleSetId::Quake3, vec![256, 512, 768], None, 1);
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
    let mut m = mixer(RuleSetId::Quake, vec![1024], None, 1);
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
