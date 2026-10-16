use qa_audio::{Bank, Listener, Mixer};
use qa_core::{primitives::*, sys_events::EventTime};
use qa_gameplay::rules::{loop_rules, native_sound_playback, native_sound_spatial};
use std::num::NonZeroU32;

const CLIENT: ClientId = ClientId(7);
const LIFE: EntityId = EntityId {
    slot: 3,
    generation: 9,
};
const OWNER: ModuleId = ModuleId(2);

fn mixer(voices: usize) -> (Bank, Mixer) {
    let rate = NonZeroU32::new(44_100).unwrap();
    let bank = Bank::load(
        rate,
        vec![
            (
                "sound/a.wav".into(),
                Pcm {
                    rate,
                    channels: PcmChannels::Mono,
                    samples: vec![1001; 128],
                    loop_start: Some(5),
                    loop_end: None,
                    loop_invalid: false,
                },
            ),
            (
                "sound/b.wav".into(),
                Pcm {
                    rate,
                    channels: PcmChannels::Mono,
                    samples: vec![-1001; 128],
                    loop_start: None,
                    loop_end: None,
                    loop_invalid: false,
                },
            ),
        ],
    )
    .unwrap();
    let mixer = Mixer::load(bank.rate, voices, 2);
    (bank, mixer)
}
fn emitter(slot: i32) -> SoundEmitter {
    SoundEmitter::Native {
        namespace: ResourceNamespace::Client(CLIENT),
        slot,
    }
}
fn event(bank: &Bank, slot: i32, ms: u64) -> SoundEvent {
    SoundEvent {
        sound: bank.find("sound/a.wav").unwrap(),
        source_ordinal: Some(1),
        playback: native_sound_playback(RuleSetId::Quake3),
        spatial: native_sound_spatial(RuleSetId::Quake3),
        entity: Some(emitter(slot)),
        queued_at: EventTime::from_milliseconds(ms),
        client: Some(CLIENT),
        client_lifetime: Some(LIFE),
        listener_owner: Some(OWNER),
        volume: 1.0,
        attenuation: 1.0,
        ..SoundEvent::default()
    }
}
fn control(action: SoundAction) -> SoundEvent {
    SoundEvent {
        action,
        client: Some(CLIENT),
        client_lifetime: Some(LIFE),
        listener_owner: Some(OWNER),
        ..SoundEvent::default()
    }
}
fn respatialize(bank: &Bank, m: &mut Mixer, origin: Vec3) -> bool {
    let mut e = control(SoundAction::Respatialize);
    e.listener = Some(Listener {
        client: CLIENT,
        entity: Some(emitter(1)),
        origin,
        right: Vec3([-1.0, 0.0, 0.0]),
        negate_dot: true,
    });
    m.sound(bank, e)
}
fn looping(bank: &Bank, slot: i32, persistent: bool) -> SoundEvent {
    SoundEvent {
        playback: None,
        looping: Some(loop_rules(RuleSetId::Quake3).unwrap().native(persistent)),
        action: SoundAction::StartLoop,
        ..event(bank, slot, 0)
    }
}
fn sample(bank: &Bank, m: &mut Mixer) -> [i16; 2] {
    let mut output = [0; 2];
    assert!(m.paint(bank, &mut output));
    output
}
// Original snd_mix applies device volume to the spatial gain before the
// sample product, then shifts the stereo transfer once more.
fn expected(sample: i16, gain: i64, volume: f32) -> i16 {
    (((i64::from(sample) * (gain * (volume * 255.0) as i64)) >> 8) >> 8)
        .clamp(i64::from(i16::MIN), i64::from(i16::MAX)) as i16
}

#[test]
fn local_ui_audio_is_audible_without_client_or_listener_and_keeps_native_shifts() {
    for volume in [1.0, 0.37, 0.001] {
        let (bank, mut m) = mixer(2);
        m.listen(&[], volume);
        let mut e = event(&bank, 1, 0);
        e.client = None;
        e.client_lifetime = None;
        e.entity = None;
        e.bus = SoundBus::Local;
        assert!(m.sound(&bank, e));
        assert_eq!(sample(&bank, &mut m), [expected(1001, 127, volume); 2]);
        e.sound = bank.find("sound/b.wav").unwrap();
        e.source_ordinal = Some(2);
        e.queued_at = EventTime::from_milliseconds(100);
        assert!(m.sound(&bank, e));
        assert_eq!(
            sample(&bank, &mut m),
            [expected(1001, 127, volume) + expected(-1001, 127, volume); 2]
        );
    }
}

#[test]
fn native_one_shots_ignore_wav_cues_and_finish() {
    let (bank, mut m) = mixer(1);
    let mut e = event(&bank, 1, 0);
    e.bus = SoundBus::Local;
    assert!(m.sound(&bank, e));
    assert!(m.paint(&bank, &mut [0; 256]));
    assert_eq!(sample(&bank, &mut m), [0; 2]);
}

#[test]
fn native_loops_latch_only_at_respatialize_including_stop_and_sound_replacement() {
    let (bank, mut m) = mixer(1);
    let mut e = looping(&bank, 4, false);
    assert!(m.sound(&bank, e));
    assert_eq!(sample(&bank, &mut m), [0; 2]);
    assert!(respatialize(&bank, &mut m, Vec3::default()));
    assert_eq!(sample(&bank, &mut m), [expected(1001, 63, 1.0); 2]);
    e.sound = bank.find("sound/b.wav").unwrap();
    e.source_ordinal = Some(2);
    assert!(m.sound(&bank, e));
    assert_eq!(sample(&bank, &mut m), [expected(1001, 63, 1.0); 2]);
    assert!(respatialize(&bank, &mut m, Vec3::default()));
    assert_eq!(sample(&bank, &mut m), [expected(-1001, 63, 1.0); 2]);
    e.action = SoundAction::StopLoop;
    assert!(m.sound(&bank, e));
    assert_eq!(sample(&bank, &mut m), [expected(-1001, 63, 1.0); 2]);
    assert!(respatialize(&bank, &mut m, Vec3::default()));
    assert_eq!(sample(&bank, &mut m), [0; 2]);
}

#[test]
fn clear_false_mutes_every_latched_loop_but_real_registration_survives() {
    let (bank, mut m) = mixer(3);
    assert!(m.sound(&bank, looping(&bank, 4, false)));
    assert!(m.sound(&bank, looping(&bank, 5, true)));
    assert!(respatialize(&bank, &mut m, Vec3::default()));
    assert_eq!(sample(&bank, &mut m), [expected(1001, 63 + 45, 1.0); 2]);
    assert!(m.sound(&bank, control(SoundAction::ClearTemporaryLoops)));
    assert_eq!(sample(&bank, &mut m), [0; 2]);
    assert!(respatialize(&bank, &mut m, Vec3::default()));
    assert_eq!(sample(&bank, &mut m), [expected(1001, 45, 1.0); 2]);
    assert!(m.sound(&bank, looping(&bank, 4, false)));
    assert_eq!(sample(&bank, &mut m), [expected(1001, 45, 1.0); 2]);
    assert!(respatialize(&bank, &mut m, Vec3::default()));
    assert_eq!(sample(&bank, &mut m), [expected(1001, 108, 1.0); 2]);
    assert!(m.sound(&bank, control(SoundAction::ClearClientLoops)));
    assert!(respatialize(&bank, &mut m, Vec3::default()));
    assert_eq!(sample(&bank, &mut m), [0; 2]);
}

#[test]
fn entity_position_updates_dynamic_channels_only_and_waits_for_respatialize() {
    let (bank, mut m) = mixer(2);
    let mut e = event(&bank, 4, 0);
    e.position = Vec3([400.0, 0.0, 0.0]);
    assert!(m.sound(&bank, e));
    assert_eq!(sample(&bank, &mut m), [expected(1001, 127, 1.0); 2]);
    assert!(respatialize(&bank, &mut m, Vec3::default()));
    assert_eq!(sample(&bank, &mut m), [0, expected(1001, 94, 1.0)]);
    let mut update = control(SoundAction::UpdateEmitterPosition);
    update.entity = Some(emitter(4));
    assert!(m.sound(&bank, update));
    assert_eq!(sample(&bank, &mut m), [0, expected(1001, 94, 1.0)]);
    assert!(respatialize(&bank, &mut m, Vec3::default()));
    assert_eq!(sample(&bank, &mut m), [expected(1001, 63, 1.0); 2]);
    e.positioned = true;
    e.queued_at = EventTime::from_milliseconds(100);
    e.source_ordinal = Some(2);
    assert!(m.sound(&bank, e));
    assert!(m.sound(&bank, update));
    assert!(respatialize(&bank, &mut m, Vec3::default()));
    assert_eq!(
        sample(&bank, &mut m),
        [
            expected(1001, 63, 1.0),
            expected(1001, 63, 1.0) + expected(1001, 94, 1.0)
        ]
    );
}

#[test]
fn add_loop_updates_shared_emitter_origin_without_touching_fixed_origin_or_gain_latch() {
    let (bank, mut m) = mixer(3);
    let mut e = event(&bank, 4, 0);
    e.position = Vec3([400.0, 0.0, 0.0]);
    assert!(m.sound(&bank, e));
    assert!(m.sound(
        &bank,
        SoundEvent {
            positioned: true,
            source_ordinal: Some(2),
            ..e
        }
    ));
    assert!(respatialize(&bank, &mut m, Vec3::default()));
    assert_eq!(sample(&bank, &mut m), [0, expected(1001, 188, 1.0)]);
    assert!(m.sound(&bank, looping(&bank, 4, false)));
    assert_eq!(sample(&bank, &mut m), [0, expected(1001, 188, 1.0)]);
    assert!(respatialize(&bank, &mut m, Vec3::default()));
    let product = |gain: i64| (1001i64 * (gain * 255)) >> 8;
    assert_eq!(
        sample(&bank, &mut m),
        [
            ((product(63) * 2) >> 8) as i16,
            ((product(63) * 2 + product(94)) >> 8) as i16
        ]
    );
}

#[test]
fn owned_listener_survives_host_none_and_wrong_owner_until_exact_release() {
    let (bank, mut m) = mixer(2);
    assert!(respatialize(&bank, &mut m, Vec3::default()));
    m.listen(&[None, None], 1.0);
    let mut e = event(&bank, 4, 0);
    e.playback = None;
    e.position = Vec3([400.0, 0.0, 0.0]);
    assert!(m.sound(&bank, e));
    assert_eq!(sample(&bank, &mut m), [0, ((1001i64 * 189) >> 8) as i16]);
    let mut release = control(SoundAction::ReleaseListener);
    release.listener_owner = Some(ModuleId(99));
    assert!(!m.sound(&bank, release));
    release = control(SoundAction::ReleaseListener);
    release.client_lifetime = Some(EntityId {
        generation: LIFE.generation + 1,
        ..LIFE
    });
    assert!(!m.sound(&bank, release));
    assert!(m.sound(&bank, control(SoundAction::ReleaseListener)));
    assert!(m.sound(&bank, control(SoundAction::ReleaseListener)));
    m.listen(
        &[Some(Listener {
            client: CLIENT,
            entity: Some(emitter(1)),
            origin: Vec3([400.0, 0.0, 0.0]),
            right: Vec3([1.0, 0.0, 0.0]),
            negate_dot: false,
        })],
        1.0,
    );
    assert_eq!(sample(&bank, &mut m), [((1001i64 * 127) >> 8) as i16; 2]);
}

#[test]
fn duplicate_suppression_is_queued_time_and_limits_allow_the_native_fifth_channel() {
    let (bank, mut m) = mixer(16);
    assert!(m.sound(&bank, event(&bank, 4, 0)));
    assert!(m.sound(&bank, event(&bank, 4, 49)));
    assert_eq!((m.counts.started, m.counts.suppressed), (1, 1));
    for ms in [50, 100, 150, 200] {
        assert!(m.sound(&bank, event(&bank, 4, ms)));
    }
    assert!(m.sound(&bank, event(&bank, 4, 250)));
    assert_eq!((m.counts.started, m.counts.suppressed), (5, 2));
    assert!(respatialize(&bank, &mut m, Vec3::default()));
    for ms in (0..10).map(|i| i * 50) {
        assert!(m.sound(&bank, event(&bank, 1, ms)));
    }
    assert_eq!((m.counts.started, m.counts.suppressed), (14, 3));
}

#[test]
fn native_alias_handles_share_pcm_without_double_start_suppression_or_loop_merge() {
    let (bank, mut m) = mixer(6);
    let e = event(&bank, 4, 0);
    assert!(m.sound(&bank, e));
    assert!(m.sound(
        &bank,
        SoundEvent {
            source_ordinal: Some(2),
            ..e
        }
    ));
    assert_eq!((m.counts.started, m.counts.suppressed), (2, 0));
    assert!(m.sound(&bank, SoundEvent::retire_client(CLIENT, LIFE)));
    for slot in 4..10 {
        let e = looping(&bank, slot, false);
        assert!(m.sound(
            &bank,
            SoundEvent {
                source_ordinal: Some(if slot < 7 { 1 } else { 2 }),
                ..e
            }
        ));
    }
    assert!(respatialize(&bank, &mut m, Vec3::default()));
    assert_eq!(sample(&bank, &mut m), [expected(1001, 189, 1.0) * 2; 2]);
}

#[test]
fn strict_oldest_steal_preserves_announcer_and_listener_channels() {
    let (bank, mut m) = mixer(3);
    assert!(respatialize(&bank, &mut m, Vec3::default()));
    assert!(m.sound(&bank, event(&bank, 1, 0)));
    assert!(m.sound(
        &bank,
        SoundEvent {
            channel: 7,
            ..event(&bank, 4, 0)
        }
    ));
    assert!(m.sound(&bank, event(&bank, 5, 0)));
    assert!(!m.sound(&bank, event(&bank, 6, 0)));
    let mut incoming = event(&bank, 6, 1);
    incoming.sound = bank.find("sound/b.wav").unwrap();
    incoming.source_ordinal = Some(2);
    assert!(m.sound(&bank, incoming));
    assert_eq!(m.counts.replaced, 1);
    assert!(respatialize(&bank, &mut m, Vec3::default()));
    let product = |sample: i16, gain: i64| (i64::from(sample) * (gain * 255)) >> 8;
    assert_eq!(
        sample(&bank, &mut m),
        [((product(1001, 127) + product(1001, 63) + product(-1001, 63)) >> 8) as i16; 2]
    );
    // The announcer survives ordinary stealing and can still be stopped by
    // its original emitter/channel identity.
    assert!(m.sound(
        &bank,
        SoundEvent {
            action: SoundAction::Stop,
            channel: 7,
            ..event(&bank, 4, 0)
        }
    ));
    assert_eq!(
        sample(&bank, &mut m),
        [((product(1001, 127) + product(-1001, 63)) >> 8) as i16; 2]
    );

    let (bank, mut m) = mixer(2);
    assert!(respatialize(&bank, &mut m, Vec3::default()));
    assert!(m.sound(&bank, event(&bank, 1, 0)));
    assert!(m.sound(
        &bank,
        SoundEvent {
            channel: 7,
            ..event(&bank, 4, 20)
        }
    ));
    // Neither protected channel is available to a non-listener.
    assert!(!m.sound(&bank, event(&bank, 6, 100)));
    // A listener start reaches the bounded final pass and replaces the
    // strictly oldest listener channel, leaving the newer announcer intact.
    let incoming = SoundEvent {
        sound: bank.find("sound/b.wav").unwrap(),
        source_ordinal: Some(2),
        ..event(&bank, 1, 100)
    };
    assert!(m.sound(&bank, incoming));
    assert_eq!(m.counts.replaced, 1);
    assert!(respatialize(&bank, &mut m, Vec3::default()));
    assert_eq!(
        sample(&bank, &mut m),
        [((product(-1001, 127) + product(1001, 63)) >> 8) as i16; 2]
    );
    assert!(m.sound(
        &bank,
        SoundEvent {
            action: SoundAction::Stop,
            channel: 7,
            ..event(&bank, 4, 20)
        }
    ));
    assert_eq!(sample(&bank, &mut m), [expected(-1001, 127, 1.0); 2]);
}
