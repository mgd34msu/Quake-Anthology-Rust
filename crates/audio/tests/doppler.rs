use qa_audio::{Bank, Listener, Mixer};
use qa_core::primitives::*;
use std::{num::NonZeroU32, sync::Arc};

fn fixture(samples: Vec<i16>, channels: PcmChannels) -> Mixer {
    let rate = NonZeroU32::new(44100).unwrap();
    let bank = Bank::load(
        rate,
        vec![(
            "sound/rate.wav".into(),
            Pcm {
                rate,
                channels,
                samples,
                loop_start: None,
                loop_end: None,
                loop_invalid: false,
            },
        )],
    )
    .unwrap();
    let mut mixer = Mixer::load(Arc::new(bank), 4, 1);
    mixer.listen(
        &[Some(Listener {
            client: ClientId(1),
            entity: None,
            origin: Vec3::default(),
            right: Vec3([1., 0., 0.]),
            negate_dot: false,
        })],
        1.,
    );
    mixer
}
fn emission(mixer: &Mixer, slot: i32, rate: Option<f32>) -> SoundEvent {
    let playback = qa_gameplay::rules::loop_rules(RuleSetId::Quake3)
        .unwrap()
        .admit(
            &EntityAppearance {
                loop_sound: SoundRef::native(ResourceNamespace::Client(ClientId(1)), 7),
                ..Default::default()
            },
            qa_gameplay::rules::loop_spatial_policy(RuleSetId::Quake3),
        )
        .unwrap()
        .0;
    SoundEvent {
        sound: mixer.bank.find("sound/rate.wav").unwrap(),
        looping: Some(playback),
        spatial: qa_gameplay::rules::loop_spatial_policy(RuleSetId::Quake3),
        doppler: rate.map(|scale| LoopDoppler {
            scale,
            previous_scale: 1.,
        }),
        entity: Some(SoundEmitter::Native {
            namespace: ResourceNamespace::Client(ClientId(1)),
            slot,
        }),
        client: Some(ClientId(1)),
        client_lifetime: Some(EntityId {
            slot: 0,
            generation: 1,
        }),
        volume: 1.,
        attenuation: 1.,
        action: SoundAction::StartLoop,
        ..Default::default()
    }
}
fn paint_sample(sample: f32, master: i64) -> i16 {
    // Q3 float channel contribution narrows before the final device shift.
    (((sample * (master * 255) as f32) / 256.) as i32 >> 8) as i16
}

#[test]
fn native_rate_two_averages_samples_and_preserves_negative_narrowing() {
    let mut mixer = fixture(vec![1000, 3000, -1000, -3000], PcmChannels::Mono);
    assert!(mixer.sound(emission(&mixer, 2, Some(2.))));
    let mut pcm = [0; 8];
    assert!(mixer.paint(&mut pcm));
    assert_eq!(
        pcm,
        [
            paint_sample(2000., 63),
            paint_sample(2000., 63),
            paint_sample(-2000., 63),
            paint_sample(-2000., 63),
            0,
            0,
            0,
            0
        ]
    );
}

#[test]
fn doppler_voices_stay_separate_from_same_pcm_plain_groups() {
    let mut mixer = fixture(vec![1000, 3000, 5000, 7000], PcmChannels::Mono);
    assert!(mixer.sound(emission(&mixer, 2, Some(2.))));
    assert!(mixer.sound(emission(&mixer, 3, None)));
    let mut pcm = [0; 8];
    assert!(mixer.paint(&mut pcm));
    let expected = [
        paint_sample(3000., 63),
        paint_sample(9000., 63),
        paint_sample(5000., 63),
        paint_sample(7000., 63),
    ];
    for (frame, expected) in pcm.as_chunks::<2>().0.iter().zip(expected) {
        assert_eq!(frame, &[expected; 2]);
    }
}

#[test]
fn wide_rates_use_the_padded_period_without_a_rate_cap() {
    for scale in [1025., 8192., 1_048_576., f32::MAX] {
        let mut mixer = fixture(vec![8192; 1024], PcmChannels::Mono);
        assert!(mixer.sound(emission(&mixer, 2, Some(scale))));
        let mut pcm = [0; 12];
        assert!(mixer.paint(&mut pcm));
        assert!(pcm.iter().all(|&value| value == paint_sample(8192., 63)));
    }
    let mut mixer = fixture(vec![8192; 512], PcmChannels::Mono);
    assert!(mixer.sound(emission(&mixer, 2, Some(2048.))));
    let mut pcm = [0; 12];
    assert!(mixer.paint(&mut pcm));
    assert!(pcm.iter().all(|&value| value == paint_sample(4096., 63)));
}

#[test]
fn wide_stereo_rates_average_each_source_channel_independently() {
    let mut mixer = fixture([8192, -4096].repeat(1024), PcmChannels::Stereo);
    assert!(mixer.sound(emission(&mixer, 2, Some(2048.))));
    let mut pcm = [0; 8];
    assert!(mixer.paint(&mut pcm));
    for frame in pcm.as_chunks::<2>().0 {
        assert_eq!(frame, &[paint_sample(8192., 63), paint_sample(-4096., 63)]);
    }
}

#[test]
fn invalid_rates_and_velocity_do_not_replace_a_healthy_voice() {
    let mut mixer = fixture(vec![8192; 1024], PcmChannels::Mono);
    assert!(mixer.sound(emission(&mixer, 2, Some(2.))));
    for scale in [f32::NAN, f32::INFINITY, 0., -1., 1.] {
        assert!(!mixer.sound(emission(&mixer, 2, Some(scale))));
    }
    assert!(!mixer.sound(SoundEvent {
        velocity: Vec3([f32::NAN, 0., 0.]),
        ..emission(&mixer, 2, None)
    }));
    let mut pcm = [0; 4];
    assert!(mixer.paint(&mut pcm));
    assert!(pcm.iter().all(|&value| value == paint_sample(8192., 63)));
}
