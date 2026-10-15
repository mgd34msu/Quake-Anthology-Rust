use qa_audio::{Bank, Listener, Mixer};
use qa_core::primitives::*;
use qa_gameplay::rules::sound_policy;
use std::{num::NonZeroU32, sync::Arc};

fn mixer(voices: usize) -> Mixer {
    let rate = NonZeroU32::new(44_100).unwrap();
    let bank = Bank::load(
        rate,
        [("sound/first.wav", 1024), ("sound/second.wav", -1024)]
            .into_iter()
            .map(|(name, sample)| {
                (
                    name.into(),
                    Pcm {
                        rate,
                        channels: PcmChannels::Mono,
                        samples: vec![sample; 8],
                        loop_start: None,
                        loop_end: None,
                        loop_invalid: false,
                    },
                )
            })
            .collect(),
    )
    .unwrap();
    let mut mixer = Mixer::load(Arc::new(bank), voices, 2);
    mixer.listen(
        &[Some(Listener {
            client: ClientId(0),
            entity: None,
            origin: Vec3::default(),
            right: Vec3([1.0, 0.0, 0.0]),
            negate_dot: false,
        })],
        1.0,
    );
    mixer
}
fn event(mixer: &Mixer, rules: RuleSetId, path: &str, delay: f32) -> SoundEvent {
    SoundEvent {
        sound: mixer.bank.find(path).unwrap(),
        spatial: sound_policy(rules),
        entity: Some(SoundEmitter::Native {
            namespace: ResourceNamespace::Client(ClientId(0)),
            slot: 1,
        }),
        client: Some(ClientId(0)),
        client_lifetime: Some(EntityId {
            slot: 1,
            generation: 2,
        }),
        channel: 1,
        volume: 1.0,
        attenuation: 0.0,
        action: SoundAction::Play,
        time_offset: delay,
        ..SoundEvent::default()
    }
}
fn pair(mixer: &mut Mixer, rules: RuleSetId, delay: f32) {
    for path in ["sound/first.wav", "sound/second.wav"] {
        assert!(mixer.sound(event(mixer, rules, path, delay)));
    }
}

#[test]
fn q2_equal_deadlines_issue_newest_first_so_first_publication_survives() {
    // Original Q2 snd_dma.c inserts before equal begin; snd_mix.c issues head
    // first, and S_PickChannel replaces the same entity/nonzero channel.
    for rules in [RuleSetId::Quake2, RuleSetId::Quake2Rerelease] {
        let mut mixer = mixer(4);
        pair(&mut mixer, rules, 2.0 / 44_100.0);
        assert_eq!(mixer.counts.replaced, 0);
        let mut output = [0; 8];
        assert!(mixer.paint(&mut output));
        assert_eq!(output, [0, 0, 0, 0, 1020, 1020, 1020, 1020], "{rules:?}");
        assert_eq!(mixer.counts.replaced, 1);
    }
}

#[test]
fn q2_zero_and_subsample_offsets_still_wait_for_native_issue_order() {
    for rules in [RuleSetId::Quake2, RuleSetId::Quake2Rerelease] {
        for delay in [0.0, 0.5 / 44_100.0] {
            let mut mixer = mixer(4);
            pair(&mut mixer, rules, delay);
            assert_eq!(mixer.counts.replaced, 0);
            let mut output = [0; 2];
            assert!(mixer.paint(&mut output));
            assert_eq!(output, [1020; 2], "{rules:?}, {delay}");
            assert_eq!(mixer.counts.replaced, 1);
        }
    }
}

#[test]
fn q1_qw_and_q3_retain_immediate_replacement_and_delayed_fifo() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld, RuleSetId::Quake3] {
        for delay in [0.0, 2.0 / 44_100.0] {
            let mut mixer = mixer(4);
            pair(&mut mixer, rules, delay);
            assert_eq!(mixer.counts.replaced, u64::from(delay == 0.0));
            let mut output = [0; 6];
            assert!(mixer.paint(&mut output));
            if delay != 0.0 {
                assert_eq!(&output[..4], &[0; 4]);
            }
            assert_eq!(&output[4..], &[-1020; 2], "{rules:?}, {delay}");
            assert_eq!(mixer.counts.replaced, 1);
        }
    }
}

#[test]
fn channel_zero_and_other_emitters_keep_independent_tied_starts() {
    for rules in [RuleSetId::Quake2, RuleSetId::Quake2Rerelease] {
        for different_entity in [false, true] {
            let mut mixer = mixer(4);
            let mut first = event(&mixer, rules, "sound/first.wav", 0.0);
            first.channel = if different_entity { 1 } else { 0 };
            let mut second = first;
            second.sound = mixer.bank.find("sound/second.wav").unwrap();
            if different_entity {
                second.entity = Some(SoundEmitter::Native {
                    namespace: ResourceNamespace::Client(ClientId(0)),
                    slot: 2,
                });
            }
            assert!(mixer.sound(first));
            assert!(mixer.sound(second));
            let mut output = [0; 2];
            assert!(mixer.paint(&mut output));
            assert_eq!(output, [0; 2]);
            assert_eq!(mixer.counts.replaced, 0);
        }
    }
}

#[test]
fn different_deadlines_and_stopping_pending_sounds_preserve_sample_order() {
    for rules in [RuleSetId::Quake2, RuleSetId::Quake2Rerelease] {
        let mut mixer = mixer(4);
        let first = event(&mixer, rules, "sound/first.wav", 1.0 / 44_100.0);
        let second = event(&mixer, rules, "sound/second.wav", 3.0 / 44_100.0);
        assert!(mixer.sound(second));
        assert!(mixer.sound(first));
        let mut output = [0; 8];
        assert!(mixer.paint(&mut output));
        assert_eq!(output, [0, 0, 1020, 1020, 1020, 1020, -1020, -1020]);
        assert!(mixer.sound(first));
        assert!(mixer.sound(second));
        assert!(mixer.sound(SoundEvent {
            action: SoundAction::Stop,
            ..first
        }));
        assert!(mixer.paint(&mut output));
        assert_eq!(output, [0; 8]);
        assert_eq!(mixer.counts.stopped, 3);
    }
}

#[test]
fn q2_ties_do_not_reverse_another_source_or_native_client_namespace() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld, RuleSetId::Quake3] {
        let mut mixer = mixer(8);
        for (source, emitter) in [(RuleSetId::Quake2, 1), (rules, 2)] {
            for path in ["sound/first.wav", "sound/second.wav"] {
                let mut event = event(&mixer, source, path, 2.0 / 44_100.0);
                event.entity = Some(SoundEmitter::Native {
                    namespace: ResourceNamespace::Client(ClientId(0)),
                    slot: emitter,
                });
                assert!(mixer.sound(event));
            }
        }
        let mut output = [0; 6];
        assert!(mixer.paint(&mut output));
        assert_eq!(output, [0; 6], "{rules:?}");
        assert_eq!(mixer.counts.replaced, 2);
    }
    let mut mixer = mixer(8);
    mixer.listen(
        &[
            Some(Listener {
                client: ClientId(0),
                entity: None,
                origin: Vec3::default(),
                right: Vec3([1.0, 0.0, 0.0]),
                negate_dot: false,
            }),
            Some(Listener {
                client: ClientId(1),
                entity: None,
                origin: Vec3::default(),
                right: Vec3([1.0, 0.0, 0.0]),
                negate_dot: false,
            }),
        ],
        1.0,
    );
    for (rules, client) in [(RuleSetId::Quake2, 0), (RuleSetId::Quake3, 1)] {
        for path in ["sound/first.wav", "sound/second.wav"] {
            let mut event = event(&mixer, rules, path, 0.0);
            event.client = Some(ClientId(client));
            event.entity = Some(SoundEmitter::Native {
                namespace: ResourceNamespace::Client(ClientId(client)),
                slot: 1,
            });
            assert!(mixer.sound(event));
        }
    }
    let mut output = [0; 2];
    assert!(mixer.paint(&mut output));
    assert_eq!(output, [0; 2]);
    assert_eq!(mixer.counts.replaced, 2);
}

#[test]
fn queued_zero_offset_reserves_the_existing_bounded_table_atomically() {
    let mut mixer = mixer(1);
    let first = event(&mixer, RuleSetId::Quake2, "sound/first.wav", 0.0);
    let second = event(&mixer, RuleSetId::Quake2, "sound/second.wav", 0.0);
    assert!(mixer.sound(first));
    assert!(!mixer.sound(second));
    assert_eq!(mixer.counts.full, 1);
    let mut output = [0; 2];
    assert!(mixer.paint(&mut output));
    assert_eq!(output, [1020; 2]);
}
