use qa_audio::{Bank, Listener, Mixer};
use qa_core::primitives::*;
use std::num::NonZeroU32;

fn fixture(samples: Vec<i16>, channels: PcmChannels) -> (Bank, Mixer) {
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
    let mut mixer = Mixer::load(bank.rate, 4, 1);
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
    (bank, mixer)
}
fn emission(bank: &Bank, slot: i32, rate: Option<f32>) -> SoundEvent {
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
        sound: bank.find("sound/rate.wav").unwrap(),
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
    let (bank, mut mixer) = fixture(vec![1000, 3000, -1000, -3000], PcmChannels::Mono);
    assert!(mixer.sound(&bank, emission(&bank, 2, Some(2.))));
    let mut pcm = [0; 8];
    assert!(mixer.paint(&bank, &mut pcm));
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
fn native_doppler_grouping_observes_leader_and_candidate_order() {
    for doppler_first in [true, false] {
        let (bank, mut mixer) = fixture(vec![1000, 3000, 5000, 7000], PcmChannels::Mono);
        assert!(mixer.sound(
            &bank,
            emission(&bank, if doppler_first { 2 } else { 3 }, Some(2.))
        ));
        assert!(mixer.sound(
            &bank,
            emission(&bank, if doppler_first { 3 } else { 2 }, None)
        ));
        let mut pcm = [0; 8];
        assert!(mixer.paint(&bank, &mut pcm));
        let expected = if doppler_first {
            [paint_sample(2000., 126), paint_sample(6000., 126), 0, 0]
        } else {
            [
                paint_sample(3000., 63),
                paint_sample(9000., 63),
                paint_sample(5000., 63),
                paint_sample(7000., 63),
            ]
        };
        for (frame, expected) in pcm.as_chunks::<2>().0.iter().zip(expected) {
            assert_eq!(frame, &[expected; 2]);
        }
    }
}

#[test]
fn wide_rates_use_the_padded_period_without_a_rate_cap() {
    for scale in [1025., 8192., 1_048_576., f32::MAX] {
        let (bank, mut mixer) = fixture(vec![8192; 1024], PcmChannels::Mono);
        assert!(mixer.sound(&bank, emission(&bank, 2, Some(scale))));
        let mut pcm = [0; 12];
        assert!(mixer.paint(&bank, &mut pcm));
        assert!(pcm.iter().all(|&value| value == paint_sample(8192., 63)));
    }
    let (bank, mut mixer) = fixture(vec![8192; 512], PcmChannels::Mono);
    assert!(mixer.sound(&bank, emission(&bank, 2, Some(2048.))));
    let mut pcm = [0; 12];
    assert!(mixer.paint(&bank, &mut pcm));
    assert!(pcm.iter().all(|&value| value == paint_sample(4096., 63)));
}

#[test]
fn wide_stereo_rates_average_each_source_channel_independently() {
    let (bank, mut mixer) = fixture([8192, -4096].repeat(1024), PcmChannels::Stereo);
    assert!(mixer.sound(&bank, emission(&bank, 2, Some(2048.))));
    let mut pcm = [0; 8];
    assert!(mixer.paint(&bank, &mut pcm));
    for frame in pcm.as_chunks::<2>().0 {
        assert_eq!(frame, &[paint_sample(8192., 63), paint_sample(-4096., 63)]);
    }
}

#[test]
fn invalid_rates_and_velocity_do_not_replace_a_healthy_voice() {
    let (bank, mut mixer) = fixture(vec![8192; 1024], PcmChannels::Mono);
    assert!(mixer.sound(&bank, emission(&bank, 2, Some(2.))));
    for scale in [f32::NAN, f32::INFINITY, 0., -1., 1.] {
        assert!(!mixer.sound(&bank, emission(&bank, 2, Some(scale))));
    }
    assert!(!mixer.sound(
        &bank,
        SoundEvent {
            velocity: Vec3([f32::NAN, 0., 0.]),
            ..emission(&bank, 2, None)
        }
    ));
    let mut pcm = [0; 4];
    assert!(mixer.paint(&bank, &mut pcm));
    assert!(pcm.iter().all(|&value| value == paint_sample(8192., 63)));
}
