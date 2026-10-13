#[path = "support/sound_registration.rs"]
mod support;

use qa_app::audio::SoundLoader;
use qa_audio::{Bank, SoundBindings};
use qa_compat::services::{CallError, SoundRegistrar};
use qa_content::vfs::Vfs;
use qa_core::primitives::RuleSetId;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Files {
    root: std::path::PathBuf,
    vfs: Vfs,
}
impl Files {
    fn new(entries: &[(&str, Vec<u8>)]) -> Self {
        let root = std::env::temp_dir().join(format!(
            "qa-rust-audio-registration-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        qa_platform::files::create_dir_all(&root).unwrap();
        for (path, bytes) in entries {
            let path = root.join(path);
            qa_platform::files::create_dir_all(path.parent().unwrap()).unwrap();
            qa_platform::files::write(path, bytes).unwrap();
        }
        let mut vfs = Vfs::default();
        vfs.mount_directory(&root, 0).unwrap();
        Self { root, vfs }
    }
    fn ready() -> Self {
        Self::new(&[
            ("sound/feedback/hit.wav", support::wav()),
            ("sound/test.wav", support::wav()),
            ("outside.wav", support::wav()),
        ])
    }
    fn overwrite(&self, path: &str, bytes: &[u8]) {
        qa_platform::files::write(self.root.join(path), bytes).unwrap();
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        let _ = qa_platform::files::remove_dir_all(&self.root);
    }
}

#[test]
fn native_handle_zero_is_ready_default_but_configstring_zero_is_none() {
    let files = Files::ready();
    let mut loader = SoundLoader::load(4, 256).unwrap();
    let _ = loader.register(&files.vfs, Bank::DEFAULT_SOUND, RuleSetId::Quake3);
    assert_eq!(loader.reads, 1);
    assert_eq!(loader.decodes, 1);
    let default = loader.bank().find_bytes(Bank::DEFAULT_SOUND).unwrap();
    assert_eq!(
        loader.register(&files.vfs, Bank::DEFAULT_SOUND, RuleSetId::Quake3),
        Ok(0)
    );
    assert_eq!(loader.bank().registered_sound(0), Some(default));
    let mut config = SoundBindings::load(4);
    config.attach(loader.bank());
    config.set(0, Some(default)).unwrap();
    config.set(1, Some(default)).unwrap();
    assert_eq!(config.sound(0), None);
    assert_eq!(config.sound(1), Some(default));
    assert_eq!(loader.reads, 1);
}

#[test]
fn shared_native_path_policies_and_case_hit_one_pcm_without_rereading() {
    let files = Files::ready();
    let mut loader = SoundLoader::load(4, 256).unwrap();
    let _ = loader.register(&files.vfs, Bank::DEFAULT_SOUND, RuleSetId::Quake3);
    let handle = loader
        .register(&files.vfs, b"test.wav", RuleSetId::Quake)
        .unwrap();
    let sound = loader.bank().find("sound/test.wav").unwrap();
    assert_eq!(loader.bank().registered_sound(handle), Some(sound));
    let expected_pcm = loader.bank().sample(sound).unwrap().clone();
    let reads = loader.reads;
    files.vfs.take_lookup_count();
    files.overwrite("sound/test.wav", b"malformed after the first admission");
    for (path, source) in [
        (b"TEST.WAV".as_slice(), RuleSetId::QuakeWorld),
        (b"test.wav".as_slice(), RuleSetId::Quake2),
    ] {
        assert_eq!(loader.register(&files.vfs, path, source), Ok(handle));
    }
    let explicit = loader
        .register(&files.vfs, b"#sound\\test.wav", RuleSetId::Quake2Rerelease)
        .unwrap();
    let backslash = loader
        .register(&files.vfs, b"SOUND\\TEST.WAV", RuleSetId::Quake3)
        .unwrap();
    let slash = loader
        .register(&files.vfs, b"sound/test.wav", RuleSetId::Quake3)
        .unwrap();
    assert_ne!(backslash, slash); // native Q_stricmp does not fold separators
    for handle in [explicit, backslash, slash] {
        assert_eq!(loader.bank().registered_sound(handle), Some(sound));
    }
    assert_eq!(loader.reads, reads);
    assert_eq!(files.vfs.take_lookup_count(), 0);
    let bank = loader.finish();
    assert_eq!(bank.find("SOUND/TEST.WAV"), Some(sound));
    assert_eq!(bank.sample(sound), Some(&expected_pcm));
    assert_eq!(bank.registered_handle(b"sound/test.wav"), Some(slash));
    assert_eq!(bank.registered_handle(b"sound/late.wav"), None);
}

#[test]
fn missing_raw_name_retains_one_failed_attempt_and_native_handle_order() {
    let files = Files::ready();
    let mut loader = SoundLoader::load(4, 256).unwrap();
    let _ = loader.register(&files.vfs, Bank::DEFAULT_SOUND, RuleSetId::Quake3);
    files.vfs.take_lookup_count();
    let raw = b"sound/missing\x80.wav";
    assert_eq!(
        loader.register(&files.vfs, raw, RuleSetId::Quake3),
        Err(CallError::File)
    );
    let sound = loader.bank().attempted(raw).unwrap();
    let count = loader.bank().len();
    assert_eq!(loader.bank().name(sound), Some(raw.as_slice()));
    assert_eq!(files.vfs.take_lookup_count(), 1);
    for _ in 0..3 {
        assert_eq!(
            loader.register(&files.vfs, raw, RuleSetId::Quake3),
            Err(CallError::File)
        );
    }
    assert_eq!(files.vfs.take_lookup_count(), 0);
    assert_eq!(loader.bank().len(), count);
    assert_eq!(loader.bank().find_bytes(raw), None);
    assert_eq!(loader.bank().registered_handle(raw), Some(0));
    let next = loader
        .register(&files.vfs, b"sound/test.wav", RuleSetId::Quake3)
        .unwrap();
    assert_eq!(next, 2); // handle one belongs to the retained failed attempt
    let bank = loader.finish();
    assert_eq!(bank.attempted(raw), Some(sound));
    assert_eq!(bank.registered_handle(raw), Some(0));
    assert_eq!(
        bank.registered_sound(0),
        bank.find_bytes(Bank::DEFAULT_SOUND)
    );
}

#[test]
fn optional_missing_warns_once_but_required_and_malformed_stay_errors() {
    let files = Files::new(&[("sound/bad.wav", b"not a WAV".to_vec())]);
    let mut loader = SoundLoader::load(4, 256).unwrap();
    let _ = loader.register(&files.vfs, Bank::DEFAULT_SOUND, RuleSetId::Quake3);
    assert_eq!(loader.bank().registered_sound(0), None);
    assert_eq!(
        loader.register(&files.vfs, Bank::DEFAULT_SOUND, RuleSetId::Quake3),
        Err(CallError::File)
    );
    assert!(
        loader
            .precache_path(&files.vfs, b"sound/missing.wav", RuleSetId::Quake, false)
            .unwrap()
            .is_some()
    );
    assert!(
        loader
            .precache_path(&files.vfs, b"SOUND/MISSING.WAV", RuleSetId::Quake, false)
            .unwrap()
            .is_none()
    );
    let reads = loader.reads;
    assert!(
        loader
            .precache_path(&files.vfs, b"sound/missing.wav", RuleSetId::Quake, true)
            .is_err()
    );
    assert_eq!(loader.reads, reads);
    assert!(
        loader
            .precache_path(&files.vfs, b"sound/bad.wav", RuleSetId::Quake, false)
            .is_err()
    );
    assert_eq!(
        loader.register(&files.vfs, b"sound/bad.wav", RuleSetId::Quake3),
        Err(CallError::File)
    );
}

#[test]
fn load_reserved_name_capacity_rejects_before_another_vfs_read() {
    let files = Files::ready();
    let mut loader = SoundLoader::load(0, 0).unwrap();
    loader
        .register(&files.vfs, Bank::DEFAULT_SOUND, RuleSetId::Quake3)
        .unwrap();
    files.vfs.take_lookup_count();
    assert_eq!(
        loader.register(&files.vfs, b"sound/test.wav", RuleSetId::Quake3),
        Err(CallError::Capacity)
    );
    assert_eq!(files.vfs.take_lookup_count(), 0);
    assert_eq!(loader.reads, 1);
    assert_eq!(
        loader.bank().registered_sound(0),
        loader.bank().find_bytes(Bank::DEFAULT_SOUND)
    );
}

#[test]
fn q3_player_specific_registration_retains_handle_without_reading_a_file() {
    let files = Files::new(&[
        ("sound/feedback/hit.wav", support::wav()),
        ("sound/test.wav", support::wav()),
        ("*death1.wav", support::wav()),
    ]);
    let mut loader = SoundLoader::load(4, 256).unwrap();
    let _ = loader.register(&files.vfs, Bank::DEFAULT_SOUND, RuleSetId::Quake3);
    files.vfs.take_lookup_count();
    for _ in 0..3 {
        assert_eq!(
            loader.register(&files.vfs, b"*death1.wav", RuleSetId::Quake3),
            Err(CallError::File)
        );
    }
    assert_eq!(files.vfs.take_lookup_count(), 0);
    assert_eq!(loader.reads, 1);
    assert_eq!(loader.bank().attempted(b"*death1.wav"), None);
    assert_eq!(loader.bank().registered_handle(b"*death1.wav"), Some(0));
    assert_eq!(loader.bank().registered_sound(1), None);
    assert_eq!(loader.bank().find_bytes(b"*death1.wav"), None);
    assert_eq!(
        loader.register(&files.vfs, b"sound/test.wav", RuleSetId::Quake3),
        Ok(2)
    );
    let before = loader.bank().len();
    // Q2's configstring placeholder is not a Q3 engine registration slot.
    assert_eq!(
        loader.register(&files.vfs, b"*pain.wav", RuleSetId::Quake2),
        Ok(0)
    );
    assert_eq!(loader.bank().len(), before);
}

#[test]
fn native_q3_stereo_rejection_keeps_the_shared_pcm_and_failed_handle() {
    let stereo = support::wav_channels(2);
    let files = Files::new(&[
        ("sound/feedback/hit.wav", support::wav()),
        ("sound/stereo.wav", stereo),
        ("sound/test.wav", support::wav()),
    ]);
    let mut loader = SoundLoader::load(4, 256).unwrap();
    let _ = loader.register(&files.vfs, Bank::DEFAULT_SOUND, RuleSetId::Quake3);
    // The shared PCM capability can admit stereo before native registration.
    loader
        .precache_path(&files.vfs, b"sound/stereo.wav", RuleSetId::Quake2, true)
        .unwrap();
    let sound = loader.bank().find("sound/stereo.wav").unwrap();
    assert_eq!(
        loader.bank().sample(sound).unwrap().channels,
        qa_core::primitives::PcmChannels::Stereo
    );
    let reads = loader.reads;
    files.vfs.take_lookup_count();
    for _ in 0..2 {
        assert_eq!(
            loader.register(&files.vfs, b"sound/stereo.wav", RuleSetId::Quake3),
            Err(CallError::File)
        );
    }
    assert_eq!(loader.reads, reads);
    assert_eq!(files.vfs.take_lookup_count(), 0);
    assert_eq!(
        loader.register(&files.vfs, b"sound/test.wav", RuleSetId::Quake3),
        Ok(2)
    );
    let bank = loader.finish();
    assert_eq!(bank.find("sound/stereo.wav"), Some(sound));
    assert_eq!(bank.registered_handle(b"sound/stereo.wav"), Some(0));
    assert_eq!(bank.registered_sound(1), None);
}

#[test]
fn lazy_default_preserves_the_explicit_q1_wav_policy() {
    let mut looped = support::wav();
    looped.extend(b"cue ");
    looped.extend(28u32.to_le_bytes());
    looped.extend(1u32.to_le_bytes());
    looped.extend([0; 20]);
    looped.extend(1u32.to_le_bytes());
    let riff = (looped.len() as u32 - 8).to_le_bytes();
    looped[4..8].copy_from_slice(&riff);
    let files = Files::new(&[("sound/feedback/hit.wav", looped)]);
    let mut loader = SoundLoader::load(2, 128).unwrap();
    assert_eq!(loader.reads, 0);
    assert!(loader.bank().registered_sound(0).is_none());
    loader
        .precache_path(&files.vfs, Bank::DEFAULT_SOUND, RuleSetId::Quake, true)
        .unwrap();
    let sound = loader.bank().find_bytes(Bank::DEFAULT_SOUND).unwrap();
    assert_eq!(loader.bank().sample(sound).unwrap().loop_start, Some(1));
    assert_eq!(loader.reads, 1);
    loader
        .precache_path(&files.vfs, Bank::DEFAULT_SOUND, RuleSetId::Quake, true)
        .unwrap();
    assert_eq!(loader.reads, 1);
}

#[test]
fn lazy_native_default_applies_the_same_q3_mono_boundary() {
    let stereo = support::wav_channels(2);
    let files = Files::new(&[
        ("sound/feedback/hit.wav", stereo),
        ("sound/test.wav", support::wav()),
    ]);
    let mut loader = SoundLoader::load(2, 128).unwrap();
    assert_eq!(
        loader.register(&files.vfs, b"sound/test.wav", RuleSetId::Quake3),
        Ok(1)
    );
    let default = loader.bank().find_bytes(Bank::DEFAULT_SOUND).unwrap();
    assert_eq!(
        loader.bank().sample(default).unwrap().channels,
        qa_core::primitives::PcmChannels::Stereo
    );
    assert_eq!(loader.bank().registered_sound(0), None);
    assert_eq!(
        loader.register(&files.vfs, Bank::DEFAULT_SOUND, RuleSetId::Quake3),
        Err(CallError::File)
    );
    assert_eq!(loader.reads, 2);
}

#[test]
fn frozen_default_rejects_foreign_stereo_without_reading_or_discarding_pcm() {
    let files = Files::new(&[("sound/feedback/hit.wav", support::wav_channels(2))]);
    let mut loader = SoundLoader::load(2, 128).unwrap();
    loader
        .precache_path(&files.vfs, Bank::DEFAULT_SOUND, RuleSetId::Quake, true)
        .unwrap();
    let sound = loader.bank().find_bytes(Bank::DEFAULT_SOUND).unwrap();
    assert_eq!(loader.reads, 1);
    files.vfs.take_lookup_count();
    let bank = loader.finish();
    assert_eq!(files.vfs.take_lookup_count(), 0);
    assert_eq!(bank.registered_handle(Bank::DEFAULT_SOUND), Some(0));
    assert_eq!(bank.registered_sound(0), None);
    assert_eq!(bank.find_bytes(Bank::DEFAULT_SOUND), Some(sound));
    assert_eq!(
        bank.sample(sound).unwrap().channels,
        qa_core::primitives::PcmChannels::Stereo
    );
}

#[test]
fn native_config_precache_does_not_claim_a_client_engine_handle() {
    let files = Files::new(&[
        ("sound/feedback/hit.wav", support::wav()),
        ("sound/a.wav", support::wav()),
        ("a.wav", support::wav()),
    ]);
    let mut loader = SoundLoader::load(4, 256).unwrap();
    let q2 = loader
        .precache(&files.vfs, b"a.wav", RuleSetId::Quake2)
        .unwrap()
        .unwrap();
    assert_eq!(loader.bank().name(q2), Some(b"sound/a.wav".as_slice()));
    assert_eq!(loader.bank().registered_handle(b"a.wav"), None);
    let q3_handle = loader
        .register(&files.vfs, b"a.wav", RuleSetId::Quake3)
        .unwrap();
    assert_eq!(q3_handle, 1);
    let q3 = loader.bank().registered_sound(q3_handle).unwrap();
    assert_eq!(loader.bank().name(q3), Some(b"a.wav".as_slice()));
    assert_ne!(q2, q3);
    assert_eq!(loader.reads, 3);
}

#[test]
fn first_source_never_discards_another_roles_waveform_or_native_loop_interval() {
    use qa_audio::{Listener, Mixer};
    use qa_core::primitives::*;
    for q3_first in [false, true] {
        let files = Files::new(&[
            ("sound/feedback/hit.wav", support::wav()),
            ("sound/shared.wav", support::cue_wav()),
        ]);
        let mut loader = SoundLoader::load(4, 256).unwrap();
        let (q1, native) = if q3_first {
            let native = loader
                .register(&files.vfs, b"sound/shared.wav", RuleSetId::Quake3)
                .unwrap();
            let q1 = loader
                .precache(&files.vfs, b"shared.wav", RuleSetId::Quake)
                .unwrap()
                .unwrap();
            (q1, native)
        } else {
            let q1 = loader
                .precache(&files.vfs, b"shared.wav", RuleSetId::Quake)
                .unwrap()
                .unwrap();
            let native = loader
                .register(&files.vfs, b"sound/shared.wav", RuleSetId::Quake3)
                .unwrap();
            (q1, native)
        };
        assert_eq!(loader.bank().registered_sound(native), Some(q1));
        assert_eq!((loader.reads, loader.decodes), (2, 2));
        files.vfs.take_lookup_count();
        assert_eq!(
            loader
                .precache(&files.vfs, b"shared.wav", RuleSetId::Quake2)
                .unwrap(),
            Some(q1)
        );
        assert_eq!(
            loader.register(&files.vfs, b"sound/shared.wav", RuleSetId::Quake3),
            Ok(native)
        );
        assert_eq!(files.vfs.take_lookup_count(), 0);
        let bank = loader.finish();
        let pcm = bank.sample(q1).unwrap();
        assert_eq!(pcm.samples, [1024, -2048, 4096, 7777]);
        assert_eq!(
            (
                pcm.frames(),
                pcm.cue_frames(),
                pcm.loop_start,
                pcm.loop_end,
                pcm.loop_invalid
            ),
            (4, 3, Some(1), Some(3), false)
        );

        let mut q3 = Mixer::load(Arc::clone(&bank), 1, 1);
        assert!(q3.sound(SoundEvent {
            sound: q1,
            volume: 1.0,
            bus: SoundBus::Local,
            playback: qa_gameplay::rules::native_sound_playback(RuleSetId::Quake3),
            ..SoundEvent::default()
        }));
        let mut output = [0; 12];
        assert!(q3.paint(&mut output));
        let native_sample = |sample: i16| (((i64::from(sample) * (127 * 255)) >> 8) >> 8) as i16;
        let expected: Vec<_> = [1024, -2048, 4096, 7777, 0, 0]
            .into_iter()
            .flat_map(|sample| [native_sample(sample); 2])
            .collect();
        assert_eq!(output.as_slice(), expected, "Q3 first={q3_first}");
        let mut q1_mixer = Mixer::load(Arc::clone(&bank), 1, 1);
        q1_mixer.listen(
            &[Some(Listener {
                client: ClientId(0),
                entity: None,
                origin: Vec3::default(),
                right: Vec3([1.0, 0.0, 0.0]),
                negate_dot: false,
            })],
            1.0,
        );
        assert!(q1_mixer.sound(SoundEvent {
            sound: q1,
            volume: 1.0,
            ..SoundEvent::default()
        }));
        assert!(q1_mixer.paint(&mut output));
        let expected: Vec<_> = [1024, -2048, 4096, -2048, 4096, -2048]
            .into_iter()
            .flat_map(|sample| [((i64::from(sample) * 255) >> 8) as i16; 2])
            .collect();
        assert_eq!(output.as_slice(), expected, "Q1 first={q3_first}");
    }
}

#[test]
fn malformed_optional_cues_reject_quake_without_poisoning_q3_in_either_order() {
    use qa_core::primitives::*;
    for q3_first in [false, true] {
        for malformed in 0..3 {
            let mut wav = support::cue_wav();
            let cue = wav.windows(4).position(|word| word == b"cue ").unwrap();
            match malformed {
                0 => wav[cue + 32..cue + 36].copy_from_slice(&999u32.to_le_bytes()),
                1 => wav[cue + 8..cue + 12].copy_from_slice(&2u32.to_le_bytes()),
                _ => {
                    wav.extend(b"LIST");
                    wav.extend(200u32.to_le_bytes());
                    wav.extend(b"mark");
                }
            }
            let length = wav.len() as u32 - 8;
            wav[4..8].copy_from_slice(&length.to_le_bytes());
            let files = Files::new(&[
                ("sound/feedback/hit.wav", support::wav()),
                ("sound/shared.wav", wav),
            ]);
            let mut loader = SoundLoader::load(4, 256).unwrap();
            let native = if q3_first {
                let native = loader
                    .register(&files.vfs, b"sound/shared.wav", RuleSetId::Quake3)
                    .unwrap();
                assert_eq!(
                    loader.precache(&files.vfs, b"shared.wav", RuleSetId::Quake),
                    Err(CallError::File)
                );
                native
            } else {
                assert_eq!(
                    loader.precache(&files.vfs, b"shared.wav", RuleSetId::Quake),
                    Err(CallError::File)
                );
                loader
                    .register(&files.vfs, b"sound/shared.wav", RuleSetId::Quake3)
                    .unwrap()
            };
            let sound = loader.bank().registered_sound(native).unwrap();
            let pcm = loader.bank().sample(sound).unwrap();
            assert_eq!(pcm.samples, [1024, -2048, 4096, 7777]);
            assert_eq!(
                (pcm.loop_start, pcm.loop_end, pcm.loop_invalid),
                (None, None, true)
            );
            assert!(
                loader
                    .precache_path(&files.vfs, b"sound/shared.wav", RuleSetId::Quake, true)
                    .is_err()
            );
            files.vfs.take_lookup_count();
            assert_eq!(
                loader.register(&files.vfs, b"sound/shared.wav", RuleSetId::Quake3),
                Ok(native)
            );
            assert_eq!(files.vfs.take_lookup_count(), 0);
            assert_eq!((loader.reads, loader.decodes), (2, 2));
            let bank = loader.finish();
            let mut mixer = qa_audio::Mixer::load(bank, 1, 0);
            assert!(!mixer.sound(SoundEvent {
                sound,
                volume: 1.0,
                ..SoundEvent::default()
            }));
            assert!(mixer.sound(SoundEvent {
                sound,
                volume: 1.0,
                bus: SoundBus::Local,
                playback: qa_gameplay::rules::native_sound_playback(RuleSetId::Quake3),
                ..SoundEvent::default()
            }));
            let mut output = [0; 12];
            assert!(mixer.paint(&mut output));
            assert_ne!(&output[6..8], &[0, 0]);
            assert_eq!(&output[8..], &[0; 4]);
        }
    }
}
