use qa_audio::{Bank, Listener, Mixer};
use qa_core::primitives::*;
use qa_gameplay::rules::{loop_rules, loop_spatial_policy};
use std::num::NonZeroU32;

const CLIENT: ClientId = ClientId(7);
const LIFE: EntityId = EntityId {
    slot: 3,
    generation: 9,
};

fn mixer(samples: &[i16], loop_start: Option<usize>, voices: usize) -> (Bank, Mixer) {
    let rate = NonZeroU32::new(44_100).unwrap();
    let bank = Bank::load(
        rate,
        vec![(
            "sound/loop.wav".into(),
            Pcm {
                rate,
                channels: PcmChannels::Mono,
                samples: samples.to_vec(),
                loop_start,
                loop_end: None,
                loop_invalid: false,
            },
        )],
    )
    .unwrap();
    let mut mixer = Mixer::load(bank.rate, voices, 2);
    listen(&mut mixer, 1.0);
    (bank, mixer)
}
fn listen(mixer: &mut Mixer, volume: f32) {
    mixer.listen(
        &[Some(Listener {
            client: CLIENT,
            entity: Some(SoundEmitter::Native {
                namespace: ResourceNamespace::Client(CLIENT),
                slot: 1,
            }),
            origin: Vec3::default(),
            right: Vec3([1.0, 0.0, 0.0]),
            negate_dot: false,
        })],
        volume,
    );
}
fn appearance() -> EntityAppearance {
    EntityAppearance {
        loop_sound: SoundRef::native(ResourceNamespace::Client(CLIENT), 23),
        ..EntityAppearance::default()
    }
}
fn native_loop(bank: &Bank, source: RuleSetId, slot: i32) -> SoundEvent {
    loop_with(bank, source, slot, &appearance(), source)
}
fn loop_with(
    bank: &Bank,
    source: RuleSetId,
    slot: i32,
    appearance: &EntityAppearance,
    presentation: RuleSetId,
) -> SoundEvent {
    let (playback, spatial) = loop_rules(source)
        .unwrap()
        .admit(appearance, loop_spatial_policy(presentation))
        .unwrap();
    SoundEvent {
        sound: bank.find("sound/loop.wav").unwrap(),
        source_ordinal: Some(23),
        looping: Some(playback),
        spatial,
        entity: Some(SoundEmitter::Native {
            namespace: ResourceNamespace::Client(CLIENT),
            slot,
        }),
        client: Some(CLIENT),
        client_lifetime: Some(LIFE),
        volume: 1.0,
        attenuation: 1.0,
        action: SoundAction::StartLoop,
        ..SoundEvent::default()
    }
}
// Original snd_mix multiplies grouped channel volume by snd_vol, then shifts
// the sample product, then shifts once more in the stereo device transfer.
fn integer_sample(sample: i16, gain: i64, volume: f32, scale: u16) -> i16 {
    let snd_vol = (volume * f32::from(scale)) as i64;
    (((i64::from(sample) * (gain * snd_vol)) >> 8) >> 8)
        .clamp(i64::from(i16::MIN), i64::from(i16::MAX)) as i16
}

#[test]
fn original_integer_loop_formulas_keep_both_shifts_and_source_volume_scale() {
    let samples = [1001, -1001, 32_767, -32_768, 13];
    for (source, gain, scale) in [(RuleSetId::Quake2, 127, 256), (RuleSetId::Quake3, 63, 255)] {
        for volume in [1.0, 0.8, 0.5, 0.001] {
            let (bank, mut mixer) = mixer(&samples, None, 2);
            listen(&mut mixer, volume);
            assert!(mixer.sound(&bank, native_loop(&bank, source, 1)));
            let mut output = [0; 10];
            assert!(mixer.paint(&bank, &mut output));
            for (frame, sample) in output.chunks_exact(2).zip(samples) {
                assert_eq!(
                    frame,
                    [integer_sample(sample, gain, volume, scale); 2],
                    "{source:?} {volume}"
                );
            }
        }
    }
}

#[test]
fn native_loops_use_painted_time_and_full_sample_wrap_ignoring_wav_cues() {
    for source in [
        RuleSetId::Quake2,
        RuleSetId::Quake2Rerelease,
        RuleSetId::Quake3,
    ] {
        let samples = [101, 203, 307, 409, 503];
        let (bank, mut mixer) = mixer(&samples, Some(2), 1);
        assert!(mixer.paint(&bank, &mut [0; 4]));
        let event = native_loop(&bank, source, 4);
        assert!(mixer.sound(&bank, event));
        let mut output = [0; 8];
        assert!(mixer.paint(&bank, &mut output));
        let expected = |sample| match source {
            RuleSetId::Quake2 => integer_sample(sample, 127, 1.0, 256),
            RuleSetId::Quake3 => integer_sample(sample, 63, 1.0, 255),
            _ => (f32::from(sample) * 0.5) as i16,
        };
        assert_eq!(
            output,
            [
                expected(307),
                expected(307),
                expected(409),
                expected(409),
                expected(503),
                expected(503),
                expected(101),
                expected(101)
            ]
        );
        assert!(mixer.sound(&bank, event));
        assert!(mixer.paint(&bank, &mut output[..2]));
        assert_eq!(output[..2], [expected(203); 2]);
        assert_eq!(mixer.counts.started, 1);
    }
}

#[test]
fn shared_waveform_retains_independent_cue_and_full_painted_loop_periods() {
    let rate = NonZeroU32::new(44_100).unwrap();
    let bank = Bank::load(
        rate,
        vec![(
            "sound/loop.wav".into(),
            Pcm {
                rate,
                channels: PcmChannels::Mono,
                samples: vec![101, 203, 307, 409],
                loop_start: Some(1),
                loop_end: Some(3),
                loop_invalid: false,
            },
        )],
    )
    .unwrap();
    for (source, sequence, gain, scale) in [
        (RuleSetId::Quake2, [101, 203, 307, 101, 203, 307], 127, 256),
        (RuleSetId::Quake3, [101, 203, 307, 409, 101, 203], 63, 255),
    ] {
        let mut mixer = Mixer::load(bank.rate, 1, 1);
        listen(&mut mixer, 1.0);
        assert!(mixer.sound(&bank, native_loop(&bank, source, 4)));
        let mut output = [0; 12];
        assert!(mixer.paint(&bank, &mut output));
        let expected: Vec<_> = sequence
            .into_iter()
            .flat_map(|sample| [integer_sample(sample, gain, 1.0, scale); 2])
            .collect();
        assert_eq!(output.as_slice(), expected, "{source:?}");
    }
    assert_eq!(
        bank.sample(bank.find("sound/loop.wav").unwrap())
            .unwrap()
            .samples,
        [101, 203, 307, 409]
    );
    let mut mixed = Mixer::load(bank.rate, 2, 1);
    listen(&mut mixed, 1.0);
    let full = native_loop(&bank, RuleSetId::Quake3, 4);
    let mut cue = native_loop(&bank, RuleSetId::Quake3, 5);
    cue.looping.as_mut().unwrap().cue_extent = true;
    assert!(mixed.sound(&bank, full));
    assert!(mixed.sound(&bank, cue));
    let mut output = [0; 12];
    assert!(mixed.paint(&bank, &mut output));
    let first_shift = |sample: i16| (i64::from(sample) * (63 * 255)) >> 8;
    for (frame, (full, cue)) in output.chunks_exact(2).zip(
        [101, 203, 307, 409, 101, 203]
            .into_iter()
            .zip([101, 203, 307, 101, 203, 307]),
    ) {
        assert_eq!(
            frame,
            [((first_shift(full) + first_shift(cue)) >> 8) as i16; 2]
        );
    }
}

#[test]
fn cursor_loop_updates_keep_the_cursor_but_wrap_the_full_pcm() {
    let samples = [101, 203, 307, 409];
    let (bank, mut mixer) = mixer(&samples, Some(2), 1);
    assert!(mixer.paint(&bank, &mut [0; 4]));
    let mut event = native_loop(&bank, RuleSetId::Quake2, 4);
    event.looping.as_mut().unwrap().phase = LoopPhase::Cursor;
    assert!(mixer.sound(&bank, event));
    let mut output = [0; 2];
    for sample in samples.into_iter().chain([101, 203]) {
        assert!(mixer.sound(&bank, event));
        assert!(mixer.paint(&bank, &mut output));
        assert_eq!(output, [integer_sample(sample, 127, 1.0, 256); 2]);
    }
    assert_eq!(mixer.counts.started, 1);
}

#[test]
fn same_pcm_integer_groups_cap_after_emitter_gains_and_merge_q3_lifetimes() {
    for (source, count, gain, scale) in [
        (RuleSetId::Quake2, 2, 254, 256),
        (RuleSetId::Quake2, 3, 255, 256),
        (RuleSetId::Quake3, 3, 189, 255),
        (RuleSetId::Quake3, 5, 255, 255),
    ] {
        let (bank, mut mixer) = mixer(&[1001], None, 8);
        for slot in (1..=count).rev() {
            assert!(mixer.sound(&bank, native_loop(&bank, source, slot)));
        }
        let mut output = [0; 2];
        assert!(mixer.paint(&bank, &mut output));
        assert_eq!(output, [integer_sample(1001, gain, 1.0, scale); 2]);
    }
    let (bank, mut mixer) = mixer(&[1001], None, 2);
    let mut speaker = appearance();
    speaker.native.entity_type = 7;
    let speaker = loop_with(&bank, RuleSetId::Quake3, 8, &speaker, RuleSetId::Quake3);
    assert_eq!(speaker.looping.unwrap().lifetime, LoopLifetime::Persistent);
    assert!(mixer.sound(&bank, speaker));
    assert!(mixer.sound(&bank, native_loop(&bank, RuleSetId::Quake3, 1)));
    let mut output = [0; 2];
    assert!(mixer.paint(&bank, &mut output));
    assert_eq!(output, [integer_sample(1001, 45 + 63, 1.0, 255); 2]);
}

#[test]
fn rerelease_float_groups_cap_before_global_volume_and_truncate_at_transfer() {
    for count in [1, 2, 3] {
        for volume in [1.0, 0.37] {
            let (bank, mut mixer) = mixer(&[1001, -1001], None, 3);
            listen(&mut mixer, volume);
            for slot in (1..=count).rev() {
                assert!(mixer.sound(&bank, native_loop(&bank, RuleSetId::Quake2Rerelease, slot)));
            }
            let gain = (0.5f32 * count as f32).min(1.0) * volume;
            let mut output = [0; 4];
            assert!(mixer.paint(&bank, &mut output));
            let positive = (1001.0 * gain) as i16;
            let negative = (-1001.0 * gain) as i16;
            assert_eq!(output, [positive, positive, negative, negative]);
        }
    }
}

#[test]
fn loop_geometry_does_not_apply_the_one_shot_self_volume_shortcut() {
    for (source, right) in [
        (RuleSetId::Quake2, 39),
        (RuleSetId::Quake2Rerelease, 40),
        (RuleSetId::Quake3, 366),
    ] {
        let (bank, mut mixer) = mixer(&[1001], None, 1);
        let mut event = native_loop(&bank, source, 1);
        event.position = Vec3([400.0, 0.0, 0.0]);
        assert!(mixer.sound(&bank, event));
        let mut output = [0; 2];
        assert!(mixer.paint(&bank, &mut output));
        assert_eq!(output, [0, right], "{source:?}");
    }
}

#[test]
fn independent_presentation_and_rerelease_overrides_are_cached_capability_data() {
    let mut appearance = appearance();
    let source = loop_rules(RuleSetId::Quake2Rerelease).unwrap();
    let target = loop_spatial_policy(RuleSetId::Quake3);
    let (_, spatial) = source.admit(&appearance, target).unwrap();
    assert_eq!(spatial, target);
    for (attenuation, distance) in [
        (-1.0, DistanceAttenuation::Linear(0.0)),
        (0.0, target.attenuation),
        (3.0, target.attenuation),
        (2.0, DistanceAttenuation::Linear(2.0 * 0.0006)),
    ] {
        appearance.native.loop_attenuation = attenuation;
        let (_, spatial) = source.admit(&appearance, target).unwrap();
        assert_eq!(spatial.attenuation, distance);
    }
    appearance.native.loop_volume = 0.0;
    assert_eq!(
        source.admit(&appearance, target).unwrap().0.master,
        LoopGain::Float(1.0)
    );
    appearance.native.loop_volume = 0.25;
    appearance.native.render_flags = 4;
    let (playback, _) = source.admit(&appearance, target).unwrap();
    assert_eq!(playback.master, LoopGain::Float(0.25));
    assert!(!playback.panning);
    assert_eq!(playback.origin, LoopOrigin::Presented);
    assert!(loop_rules(RuleSetId::Quake).is_none());
    assert!(loop_rules(RuleSetId::QuakeWorld).is_none());
}

#[test]
fn no_stereo_still_attenuates_and_attenuation_none_does_not_pan() {
    for (attenuation, flags, expected) in [(0.0, 4, 40), (-1.0, 0, 1001)] {
        let (bank, mut mixer) = mixer(&[1001], None, 1);
        let mut appearance = appearance();
        appearance.native.render_flags = flags;
        appearance.native.loop_attenuation = attenuation;
        let mut event = loop_with(
            &bank,
            RuleSetId::Quake2Rerelease,
            2,
            &appearance,
            RuleSetId::Quake2Rerelease,
        );
        event.position = Vec3([400.0, 0.0, 0.0]);
        assert!(mixer.sound(&bank, event));
        let mut output = [0; 2];
        assert!(mixer.paint(&bank, &mut output));
        assert_eq!(output, [expected; 2]);
    }
}

#[test]
fn loop_only_stop_reset_and_replacement_preserve_channel_zero_and_static_cues() {
    let (bank, mut mixer) = mixer(&[1001], Some(0), 5);
    let loop_event = native_loop(&bank, RuleSetId::Quake2, 1);
    let mut one_shot = loop_event;
    one_shot.looping = None;
    one_shot.action = SoundAction::Play;
    one_shot.attenuation = 0.0;
    let mut static_cue = one_shot;
    static_cue.entity = Some(SoundEmitter::Static {
        namespace: ResourceNamespace::Client(CLIENT),
        lifetime: LIFE,
        slot: 4,
    });
    static_cue.action = SoundAction::StartLoop;
    assert!(mixer.sound(&bank, loop_event));
    assert!(mixer.sound(&bank, one_shot));
    assert!(mixer.sound(&bank, static_cue));
    let mut stop = loop_event;
    stop.looping = None;
    stop.action = SoundAction::StopLoop;
    assert!(mixer.sound(&bank, stop));
    assert_eq!(mixer.counts.stopped, 1);
    let mut output = [0; 2];
    assert!(mixer.paint(&bank, &mut output));
    assert_eq!(output, [1994; 2]);
    assert!(mixer.sound(&bank, loop_event));
    let mut next = loop_event;
    next.client_lifetime = Some(EntityId {
        generation: LIFE.generation + 1,
        ..LIFE
    });
    assert!(mixer.sound(&bank, next));
    assert!(mixer.sound(&bank, SoundEvent::reset_client_loops(CLIENT, LIFE)));
    assert_eq!(mixer.counts.stopped, 2);
    assert!(mixer.paint(&bank, &mut output));
    assert_eq!(output, [1994 + integer_sample(1001, 127, 1.0, 256); 2]);
    assert!(mixer.sound(&bank, SoundEvent::retire_client(CLIENT, LIFE)));
    assert_eq!(mixer.counts.stopped, 4);
    assert!(mixer.paint(&bank, &mut output));
    assert_eq!(output, [integer_sample(1001, 127, 1.0, 256); 2]);
}

#[test]
fn changing_a_native_loop_pcm_reuses_only_its_emitter_voice() {
    let rate = NonZeroU32::new(44_100).unwrap();
    let bank = Bank::load(
        rate,
        [("sound/loop.wav", 1001), ("sound/other.wav", -2003)]
            .map(|(path, value)| {
                (
                    path.into(),
                    Pcm {
                        rate,
                        channels: PcmChannels::Mono,
                        samples: vec![value],
                        loop_start: None,
                        loop_end: None,
                        loop_invalid: false,
                    },
                )
            })
            .to_vec(),
    )
    .unwrap();
    let mut mixer = Mixer::load(bank.rate, 2, 1);
    listen(&mut mixer, 1.0);
    let event = native_loop(&bank, RuleSetId::Quake2, 1);
    let mut one_shot = event;
    one_shot.looping = None;
    one_shot.action = SoundAction::Play;
    one_shot.attenuation = 0.0;
    assert!(mixer.sound(&bank, one_shot));
    assert!(mixer.sound(&bank, event));
    let mut changed = event;
    changed.sound = bank.find("sound/other.wav").unwrap();
    assert!(mixer.sound(&bank, changed));
    assert_eq!(mixer.counts.replaced, 1);
    let mut output = [0; 2];
    assert!(mixer.paint(&bank, &mut output));
    assert_eq!(output, [997 + integer_sample(-2003, 127, 1.0, 256); 2]);
}

#[test]
fn an_inaudible_float_loop_preserves_ordinary_negative_transfer_rounding() {
    let (bank, mut mixer) = mixer(&[-1001], None, 2);
    let mut native = native_loop(&bank, RuleSetId::Quake2Rerelease, 1);
    native.position = Vec3([10_000.0, 0.0, 0.0]);
    assert!(mixer.sound(&bank, native));
    let mut one_shot = native;
    one_shot.looping = None;
    one_shot.action = SoundAction::Play;
    one_shot.attenuation = 0.0;
    assert!(mixer.sound(&bank, one_shot));
    let mut output = [0; 2];
    assert!(mixer.paint(&bank, &mut output));
    assert_eq!(output, [-998; 2]);
}

#[test]
fn invalid_loop_gain_or_scope_does_not_replace_a_retained_voice() {
    let (bank, mut mixer) = mixer(&[1001], None, 1);
    let event = native_loop(&bank, RuleSetId::Quake2Rerelease, 1);
    assert!(mixer.sound(&bank, event));
    for master in [f32::NAN, f32::INFINITY, -1.0, 1.1] {
        let mut invalid = event;
        invalid.looping.as_mut().unwrap().master = LoopGain::Float(master);
        assert!(!mixer.sound(&bank, invalid));
    }
    let mut invalid = event;
    invalid.client_lifetime = None;
    assert!(!mixer.sound(&bank, invalid));
    invalid = event;
    invalid.entity = Some(SoundEmitter::Native {
        namespace: ResourceNamespace::Module(ModuleId(7)),
        slot: 1,
    });
    assert!(!mixer.sound(&bank, invalid));
    invalid = event;
    invalid.time_offset = 0.5;
    assert!(!mixer.sound(&bank, invalid));
    assert_eq!(mixer.counts.invalid, 7);
    let mut output = [0; 2];
    assert!(mixer.paint(&bank, &mut output));
    assert_eq!(output, [500; 2]);
    assert_eq!(mixer.counts.started, 1);
}

#[test]
fn moving_loops_select_the_native_pose_capability_separately_from_channels() {
    let (bank, mut mixer) = mixer(&[1001], None, 3);
    for (source, slot) in [(RuleSetId::Quake2, 1), (RuleSetId::Quake2Rerelease, 2)] {
        assert!(mixer.sound(&bank, native_loop(&bank, source, slot)));
    }
    let mut ordinary = native_loop(&bank, RuleSetId::Quake3, 3);
    ordinary.looping = None;
    ordinary.action = SoundAction::Play;
    assert!(mixer.sound(&bank, ordinary));
    let mut calls = [false; 3];
    mixer.respatialize(|client, slot, origin| {
        assert_eq!(client, CLIENT);
        assert_eq!(
            origin,
            [
                Some(LoopOrigin::Accepted),
                Some(LoopOrigin::Presented),
                None
            ][slot as usize - 1]
        );
        calls[slot as usize - 1] = true;
        Some(Vec3([400.0, 0.0, 0.0]))
    });
    assert_eq!(calls, [true; 3]);
    assert_eq!(mixer.counts.respatialized, 3);
}

#[test]
fn q3_stops_keep_toggle_keys_and_standalone_event_player_targets() {
    let rules = loop_rules(RuleSetId::Quake3).unwrap();
    let mut appearance = appearance();
    for event in [75, 75 | 0x100, 75 | 0x200, 75 | 0x300] {
        appearance.native.event = event;
        assert_eq!(rules.stop(&appearance.native), Some(event));
        assert_eq!(rules.stop_emitter(&appearance, 9), Some(9));
        assert!(!rules.event_once(&appearance.native));
    }
    appearance.native.event = 0;
    appearance.native.entity_type = 13 + 75;
    appearance.native.render_flags = 0x10;
    appearance.native.other_entity = Some(qa_core::primitives::NativeEntityRef::entity(
        qa_core::primitives::ResourceNamespace::Module(qa_core::primitives::ModuleId(1)),
        0,
    ));
    assert!(rules.event_once(&appearance.native));
    assert_eq!(
        rules.stop(&appearance.native),
        Some(0x8000_0000 | (13 + 75))
    );
    assert_eq!(rules.stop_emitter(&appearance, 53), Some(0));
    assert_eq!(
        rules.stop_target(&appearance.native),
        appearance.native.other_entity
    );
    assert!(
        rules
            .admit(&appearance, loop_spatial_policy(RuleSetId::Quake3))
            .is_none()
    );
    appearance.native.entity_type = 13 + 1;
    assert!(rules.event_once(&appearance.native));
    assert_eq!(rules.stop(&appearance.native), None);
    assert_eq!(rules.stop_emitter(&appearance, 53), None);
    assert_eq!(rules.stop_target(&appearance.native), None);
}
