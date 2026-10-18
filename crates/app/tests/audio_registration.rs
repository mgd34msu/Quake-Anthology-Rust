#[path = "support/sound_registration.rs"]
mod support;

use qa_app::audio::SoundAssets;
use qa_audio::{Bank, SoundBindings};
use qa_compat::services::{CallError, SoundRegistrar};
use qa_content::vfs::Vfs;
use qa_core::primitives::RuleSetId;
use std::sync::atomic::{AtomicU64, Ordering};

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
    let mut loader = SoundAssets::load(&files.vfs, 4, 256).unwrap();
    let _ = loader.register(&files.vfs, Bank::DEFAULT_SOUND, RuleSetId::Quake3, None);
    assert_eq!(loader.reads, 1);
    assert_eq!(loader.decodes, 1);
    let default = loader.bank().find_bytes(Bank::DEFAULT_SOUND).unwrap();
    assert_eq!(
        loader.register(&files.vfs, Bank::DEFAULT_SOUND, RuleSetId::Quake3, None),
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
    let mut loader = SoundAssets::load(&files.vfs, 4, 256).unwrap();
    let _ = loader.register(&files.vfs, Bank::DEFAULT_SOUND, RuleSetId::Quake3, None);
    let handle = loader
        .register(&files.vfs, b"test.wav", RuleSetId::Quake, None)
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
        assert_eq!(loader.register(&files.vfs, path, source, None), Ok(handle));
    }
    let explicit = loader
        .register(
            &files.vfs,
            b"#sound\\test.wav",
            RuleSetId::Quake2Rerelease,
            None,
        )
        .unwrap();
    let backslash = loader
        .register(&files.vfs, b"SOUND\\TEST.WAV", RuleSetId::Quake3, None)
        .unwrap();
    let slash = loader
        .register(&files.vfs, b"sound/test.wav", RuleSetId::Quake3, None)
        .unwrap();
    assert_ne!(backslash, slash); // native Q_stricmp does not fold separators
    for handle in [explicit, backslash, slash] {
        assert_eq!(loader.bank().registered_sound(handle), Some(sound));
    }
    assert_eq!(loader.reads, reads);
    assert_eq!(files.vfs.take_lookup_count(), 0);
    loader.end_initial_registration();
    let bank = loader.bank();
    assert_eq!(bank.find("SOUND/TEST.WAV"), Some(sound));
    assert_eq!(bank.sample(sound), Some(&expected_pcm));
    assert_eq!(bank.registered_handle(b"sound/test.wav"), Some(slash));
    assert_eq!(bank.registered_handle(b"sound/late.wav"), None);
}

#[test]
fn missing_raw_name_retains_one_failed_attempt_and_native_handle_order() {
    let files = Files::ready();
    let mut loader = SoundAssets::load(&files.vfs, 4, 256).unwrap();
    let _ = loader.register(&files.vfs, Bank::DEFAULT_SOUND, RuleSetId::Quake3, None);
    files.vfs.take_lookup_count();
    let raw = b"sound/missing\x80.wav";
    assert_eq!(
        loader.register(&files.vfs, raw, RuleSetId::Quake3, None),
        Err(CallError::File)
    );
    let sound = loader.bank().attempted(raw).unwrap();
    let count = loader.bank().len();
    assert_eq!(loader.bank().name(sound), Some(raw.as_slice()));
    assert_eq!(files.vfs.take_lookup_count(), 1);
    for _ in 0..3 {
        assert_eq!(
            loader.register(&files.vfs, raw, RuleSetId::Quake3, None),
            Err(CallError::File)
        );
    }
    assert_eq!(files.vfs.take_lookup_count(), 0);
    assert_eq!(loader.bank().len(), count);
    assert_eq!(loader.bank().find_bytes(raw), None);
    assert_eq!(loader.bank().registered_handle(raw), Some(0));
    let next = loader
        .register(&files.vfs, b"sound/test.wav", RuleSetId::Quake3, None)
        .unwrap();
    assert_eq!(next, 2); // handle one belongs to the retained failed attempt
    loader.end_initial_registration();
    let bank = loader.bank();
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
    let mut loader = SoundAssets::load(&files.vfs, 4, 256).unwrap();
    let _ = loader.register(&files.vfs, Bank::DEFAULT_SOUND, RuleSetId::Quake3, None);
    assert_eq!(loader.bank().registered_sound(0), None);
    assert_eq!(
        loader.register(&files.vfs, Bank::DEFAULT_SOUND, RuleSetId::Quake3, None),
        Err(CallError::File)
    );
    assert!(
        loader
            .precache_path(
                &files.vfs,
                b"sound/missing.wav",
                RuleSetId::Quake,
                false,
                None
            )
            .unwrap()
            .is_some()
    );
    assert!(
        loader
            .precache_path(
                &files.vfs,
                b"SOUND/MISSING.WAV",
                RuleSetId::Quake,
                false,
                None
            )
            .unwrap()
            .is_none()
    );
    let reads = loader.reads;
    assert!(
        loader
            .precache_path(
                &files.vfs,
                b"sound/missing.wav",
                RuleSetId::Quake,
                true,
                None
            )
            .is_err()
    );
    assert_eq!(loader.reads, reads);
    assert!(
        loader
            .precache_path(&files.vfs, b"sound/bad.wav", RuleSetId::Quake, false, None)
            .is_err()
    );
    assert_eq!(
        loader.register(&files.vfs, b"sound/bad.wav", RuleSetId::Quake3, None),
        Err(CallError::File)
    );
}

#[test]
fn load_reserved_name_capacity_rejects_before_another_vfs_read() {
    let files = Files::ready();
    let mut loader = SoundAssets::load(&files.vfs, 0, 0).unwrap();
    loader
        .register(&files.vfs, Bank::DEFAULT_SOUND, RuleSetId::Quake3, None)
        .unwrap();
    files.vfs.take_lookup_count();
    assert_eq!(
        loader.register(&files.vfs, b"sound/test.wav", RuleSetId::Quake3, None),
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
    let mut loader = SoundAssets::load(&files.vfs, 4, 256).unwrap();
    let _ = loader.register(&files.vfs, Bank::DEFAULT_SOUND, RuleSetId::Quake3, None);
    files.vfs.take_lookup_count();
    for _ in 0..3 {
        assert_eq!(
            loader.register(&files.vfs, b"*death1.wav", RuleSetId::Quake3, None),
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
        loader.register(&files.vfs, b"sound/test.wav", RuleSetId::Quake3, None),
        Ok(2)
    );
    let before = loader.bank().len();
    // Q2's configstring placeholder is not a Q3 engine registration slot.
    assert_eq!(
        loader.register(&files.vfs, b"*pain.wav", RuleSetId::Quake2, None),
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
    let mut loader = SoundAssets::load(&files.vfs, 4, 256).unwrap();
    let _ = loader.register(&files.vfs, Bank::DEFAULT_SOUND, RuleSetId::Quake3, None);
    // The shared PCM capability can admit stereo before native registration.
    loader
        .precache_path(
            &files.vfs,
            b"sound/stereo.wav",
            RuleSetId::Quake2,
            true,
            None,
        )
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
            loader.register(&files.vfs, b"sound/stereo.wav", RuleSetId::Quake3, None),
            Err(CallError::File)
        );
    }
    assert_eq!(loader.reads, reads);
    assert_eq!(files.vfs.take_lookup_count(), 0);
    assert_eq!(
        loader.register(&files.vfs, b"sound/test.wav", RuleSetId::Quake3, None),
        Ok(2)
    );
    loader.end_initial_registration();
    let bank = loader.bank();
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
    let mut loader = SoundAssets::load(&files.vfs, 2, 128).unwrap();
    assert_eq!(loader.reads, 0);
    assert!(loader.bank().registered_sound(0).is_none());
    loader
        .precache_path(
            &files.vfs,
            Bank::DEFAULT_SOUND,
            RuleSetId::Quake,
            true,
            None,
        )
        .unwrap();
    let sound = loader.bank().find_bytes(Bank::DEFAULT_SOUND).unwrap();
    assert_eq!(loader.bank().sample(sound).unwrap().loop_start, Some(1));
    assert_eq!(loader.reads, 1);
    loader
        .precache_path(
            &files.vfs,
            Bank::DEFAULT_SOUND,
            RuleSetId::Quake,
            true,
            None,
        )
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
    let mut loader = SoundAssets::load(&files.vfs, 2, 128).unwrap();
    assert_eq!(
        loader.register(&files.vfs, b"sound/test.wav", RuleSetId::Quake3, None),
        Ok(1)
    );
    let default = loader.bank().find_bytes(Bank::DEFAULT_SOUND).unwrap();
    assert_eq!(
        loader.bank().sample(default).unwrap().channels,
        qa_core::primitives::PcmChannels::Stereo
    );
    assert_eq!(loader.bank().registered_sound(0), None);
    assert_eq!(
        loader.register(&files.vfs, Bank::DEFAULT_SOUND, RuleSetId::Quake3, None),
        Err(CallError::File)
    );
    assert_eq!(loader.reads, 2);
}

#[test]
fn frozen_default_rejects_foreign_stereo_without_reading_or_discarding_pcm() {
    let files = Files::new(&[("sound/feedback/hit.wav", support::wav_channels(2))]);
    let mut loader = SoundAssets::load(&files.vfs, 2, 128).unwrap();
    loader
        .precache_path(
            &files.vfs,
            Bank::DEFAULT_SOUND,
            RuleSetId::Quake,
            true,
            None,
        )
        .unwrap();
    let sound = loader.bank().find_bytes(Bank::DEFAULT_SOUND).unwrap();
    assert_eq!(loader.reads, 1);
    files.vfs.take_lookup_count();
    loader.end_initial_registration();
    let bank = loader.bank();
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
    let mut loader = SoundAssets::load(&files.vfs, 4, 256).unwrap();
    let q2 = loader
        .precache(&files.vfs, b"a.wav", RuleSetId::Quake2, None)
        .unwrap()
        .unwrap();
    assert_eq!(loader.bank().name(q2), Some(b"sound/a.wav".as_slice()));
    assert_eq!(loader.bank().registered_handle(b"a.wav"), None);
    let q3_handle = loader
        .register(&files.vfs, b"a.wav", RuleSetId::Quake3, None)
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
        let mut loader = SoundAssets::load(&files.vfs, 4, 256).unwrap();
        let (q1, native) = if q3_first {
            let native = loader
                .register(&files.vfs, b"sound/shared.wav", RuleSetId::Quake3, None)
                .unwrap();
            let q1 = loader
                .precache(&files.vfs, b"shared.wav", RuleSetId::Quake, None)
                .unwrap()
                .unwrap();
            (q1, native)
        } else {
            let q1 = loader
                .precache(&files.vfs, b"shared.wav", RuleSetId::Quake, None)
                .unwrap()
                .unwrap();
            let native = loader
                .register(&files.vfs, b"sound/shared.wav", RuleSetId::Quake3, None)
                .unwrap();
            (q1, native)
        };
        assert_eq!(loader.bank().registered_sound(native), Some(q1));
        assert_eq!((loader.reads, loader.decodes), (2, 2));
        files.vfs.take_lookup_count();
        assert_eq!(
            loader
                .precache(&files.vfs, b"shared.wav", RuleSetId::Quake2, None)
                .unwrap(),
            Some(q1)
        );
        assert_eq!(
            loader.register(&files.vfs, b"sound/shared.wav", RuleSetId::Quake3, None),
            Ok(native)
        );
        assert_eq!(files.vfs.take_lookup_count(), 0);
        loader.end_initial_registration();
        let bank = loader.bank();
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

        let mut q3 = Mixer::load(bank.rate, 1, 1);
        assert!(q3.sound(
            bank,
            SoundEvent {
                sound: q1,
                volume: 1.0,
                bus: SoundBus::Local,
                playback: qa_gameplay::rules::native_sound_playback(RuleSetId::Quake3),
                ..SoundEvent::default()
            }
        ));
        let mut output = [0; 12];
        assert!(q3.paint(bank, &mut output));
        let native_sample = |sample: i16| (((i64::from(sample) * (127 * 255)) >> 8) >> 8) as i16;
        let expected: Vec<_> = [1024, -2048, 4096, 7777, 0, 0]
            .into_iter()
            .flat_map(|sample| [native_sample(sample); 2])
            .collect();
        assert_eq!(output.as_slice(), expected, "Q3 first={q3_first}");
        let mut q1_mixer = Mixer::load(bank.rate, 1, 1);
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
        assert!(q1_mixer.sound(
            bank,
            SoundEvent {
                sound: q1,
                volume: 1.0,
                ..SoundEvent::default()
            }
        ));
        assert!(q1_mixer.paint(bank, &mut output));
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
            let mut loader = SoundAssets::load(&files.vfs, 4, 256).unwrap();
            let native = if q3_first {
                let native = loader
                    .register(&files.vfs, b"sound/shared.wav", RuleSetId::Quake3, None)
                    .unwrap();
                assert_eq!(
                    loader.precache(&files.vfs, b"shared.wav", RuleSetId::Quake, None),
                    Err(CallError::File)
                );
                native
            } else {
                assert_eq!(
                    loader.precache(&files.vfs, b"shared.wav", RuleSetId::Quake, None),
                    Err(CallError::File)
                );
                loader
                    .register(&files.vfs, b"sound/shared.wav", RuleSetId::Quake3, None)
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
                    .precache_path(
                        &files.vfs,
                        b"sound/shared.wav",
                        RuleSetId::Quake,
                        true,
                        None
                    )
                    .is_err()
            );
            files.vfs.take_lookup_count();
            assert_eq!(
                loader.register(&files.vfs, b"sound/shared.wav", RuleSetId::Quake3, None),
                Ok(native)
            );
            assert_eq!(files.vfs.take_lookup_count(), 0);
            assert_eq!((loader.reads, loader.decodes), (2, 2));
            loader.end_initial_registration();
            let bank = loader.bank();
            let mut mixer = qa_audio::Mixer::load(bank.rate, 1, 0);
            assert!(!mixer.sound(
                bank,
                SoundEvent {
                    sound,
                    volume: 1.0,
                    ..SoundEvent::default()
                }
            ));
            assert!(mixer.sound(
                bank,
                SoundEvent {
                    sound,
                    volume: 1.0,
                    bus: SoundBus::Local,
                    playback: qa_gameplay::rules::native_sound_playback(RuleSetId::Quake3),
                    ..SoundEvent::default()
                }
            ));
            let mut output = [0; 12];
            assert!(mixer.paint(bank, &mut output));
            assert_ne!(&output[6..8], &[0, 0]);
            assert_eq!(&output[8..], &[0; 4]);
        }
    }
}

#[test]
fn repeated_client_binding_preserves_a_completed_source_rejection() {
    use qa_app::{Runtime, client_resources::ClientResources, host::FrameHost};
    use qa_console::{commands::Console, views::Context};
    use qa_core::{configstrings::ConfigLayout, primitives::*};
    use qa_network::commands::packet::Protocol;
    use qa_session::timing::TickRate;
    let rate = std::num::NonZeroU32::new(44100).unwrap();
    let bank = Bank::load(
        rate,
        vec![(
            "sound/invalid-loop.wav".into(),
            Pcm {
                rate,
                channels: PcmChannels::Mono,
                samples: vec![1000; 4],
                loop_start: None,
                loop_end: None,
                loop_invalid: true,
            },
        )],
    )
    .unwrap();
    let sound = bank.find("sound/invalid-loop.wav").unwrap();
    let mut runtime = Runtime::load(1, []).unwrap();
    runtime.sound_assets = Some(Box::new(SoundAssets::from_bank(&runtime.vfs, bank)));
    let mut resources = ClientResources::load(
        ClientId(0),
        Protocol::Quake2_34,
        RuleSetId::Quake3,
        &runtime.vfs,
        None,
    )
    .unwrap()
    .unwrap();
    assert!(resources.apply(
        ConfigLayout::QUAKE2.resources[1].get(1).unwrap(),
        b"invalid-loop.wav",
        None,
        None
    ));
    runtime.client_resources[0] = Some(resources);
    let mut host = FrameHost::load(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        vec![],
    )
    .unwrap();
    host.bind_sounds().unwrap();
    assert_eq!(
        host.runtime.client_resources[0]
            .as_ref()
            .unwrap()
            .state
            .sound(1),
        None
    );
    // Q3 may use canonical PCM whose loop metadata the Q2 source rejected.
    assert_eq!(
        host.runtime
            .sound_bank()
            .unwrap()
            .find("sound/invalid-loop.wav"),
        Some(sound)
    );
    host.bind_sounds().unwrap();
    assert_eq!(
        host.runtime.client_resources[0]
            .as_ref()
            .unwrap()
            .state
            .sound(1),
        None
    );
    assert_eq!(host.runtime.sound_assets.as_ref().unwrap().reads, 0);
}

#[test]
fn selected_sources_retain_distinct_pcm_native_defaults_and_failed_names() {
    let mut files = Files::ready();
    let mut changed = support::wav();
    changed[44..46].copy_from_slice(&777i16.to_le_bytes());
    let high_files = Files::new(&[
        ("sound/test.wav", changed.clone()),
        ("sound/feedback/hit.wav", changed),
    ]);
    let low_mount = files.vfs.mounts().next().unwrap().id;
    let high_mount = files.vfs.mount_directory(&high_files.root, 10).unwrap();
    let low = files.vfs.search_view([low_mount]).unwrap();
    let high = files.vfs.search_view([high_mount]).unwrap();
    let empty = files.vfs.search_view([]).unwrap();
    let mut loader = SoundAssets::load(&files.vfs, 32, 4096).unwrap();
    assert_eq!(
        loader.source(files.vfs.read_view(Some(&low))).unwrap(),
        None
    );
    let mut sounds = Vec::new();
    for (view, first) in [(Some(&low), 1024), (Some(&high), 777), (None, 777)] {
        assert_eq!(
            loader.register(&files.vfs, b"sound/test.wav", RuleSetId::Quake3, view),
            Ok(1)
        );
        let source = loader.source(files.vfs.read_view(view)).unwrap().unwrap();
        let sound = loader.bank().registered_sound_in(source, 1).unwrap();
        let default = loader.bank().registered_sound_in(source, 0).unwrap();
        assert_eq!(loader.bank().sample(sound).unwrap().samples[0], first);
        assert_eq!(loader.bank().sample(default).unwrap().samples[0], first);
        let reads = loader.reads;
        files.vfs.take_lookup_count();
        assert_eq!(
            loader.register(&files.vfs, b"SOUND\\TEST.WAV", RuleSetId::Quake3, view),
            Ok(2)
        );
        assert_eq!(
            loader
                .precache(&files.vfs, b"test.wav", RuleSetId::Quake2, view)
                .unwrap(),
            Some(sound)
        );
        assert_eq!(loader.bank().registered_sound_in(source, 2), Some(sound));
        assert_eq!(loader.reads, reads);
        assert_eq!(files.vfs.take_lookup_count(), 0);
        sounds.push(sound);
    }
    assert_ne!(sounds[0], sounds[1]);
    assert_ne!(sounds[1], sounds[2]);
    assert_eq!(loader.bank().name(sounds[0]), loader.bank().name(sounds[1]));
    assert_eq!(
        loader.register(
            &files.vfs,
            b"sound/test.wav",
            RuleSetId::Quake3,
            Some(&empty)
        ),
        Err(CallError::File)
    );
    let empty_source = loader
        .source(files.vfs.read_view(Some(&empty)))
        .unwrap()
        .unwrap();
    assert_eq!(loader.bank().registered_sound_in(empty_source, 0), None);
    assert_eq!(loader.bank().registered_sound_in(empty_source, 1), None);
    let reads = loader.reads;
    assert_eq!(
        loader.register(
            &files.vfs,
            b"sound/test.wav",
            RuleSetId::Quake3,
            Some(&empty)
        ),
        Err(CallError::File)
    );
    assert_eq!(loader.reads, reads);
    let foreign = Vfs::default();
    let foreign_view = foreign.default_view();
    for (vfs, view) in [
        (&files.vfs, Some(&foreign_view)),
        (&foreign, Some(&low)),
        (&foreign, None),
    ] {
        assert_eq!(
            loader.register(vfs, b"sound/test.wav", RuleSetId::Quake3, view),
            Err(CallError::File)
        );
        assert!(loader.source(vfs.read_view(view)).is_err());
    }
    assert_eq!(loader.reads, reads);
    assert_eq!(
        loader.register(&files.vfs, b"sound/test.wav", RuleSetId::Quake3, Some(&low)),
        Ok(1)
    );
}

#[test]
fn native_config_sound_rebinds_after_source_change_without_rewriting_its_configstring() {
    use qa_compat::services::{ENGINE_CALLS, ServiceStorage};
    use qa_console::{commands::Console, views::Context};
    use qa_core::{configstrings::ConfigLayout, primitives::ModuleId};
    let mut low = Files::ready();
    let mut changed = support::wav();
    changed[44..46].copy_from_slice(&777i16.to_le_bytes());
    let high = Files::new(&[("sound/test.wav", changed)]);
    let low_mount = low.vfs.mounts().next().unwrap().id;
    let high_mount = low.vfs.mount_directory(&high.root, 10).unwrap();
    let selected = [
        low.vfs.search_view([low_mount]).unwrap(),
        low.vfs.search_view([high_mount]).unwrap(),
    ];
    let mut runtime = qa_app::Runtime::load(1, []).unwrap();
    runtime.vfs = std::mem::take(&mut low.vfs);
    runtime.sound_assets = Some(Box::new(SoundAssets::load(&runtime.vfs, 16, 4096).unwrap()));
    let mut console = Console::new(Context::default()).unwrap();
    let module = ModuleId(1);
    let layout = ConfigLayout::for_rules(RuleSetId::Quake2);
    let mut storage = ServiceStorage::load_native(
        &[(module, layout.count, RuleSetId::Quake2)],
        4,
        &console.cvars,
    )
    .unwrap();
    storage.prepare_files(&runtime.vfs).unwrap();
    storage
        .set_file_search(module, Some(selected[0].clone()), &runtime.vfs)
        .unwrap();
    let mut scratch = runtime.geometry.scratch();
    let mut services = runtime.engine_services(&mut console, &mut storage, &mut scratch);
    let sounds = layout.resources[1];
    assert_eq!(
        (ENGINE_CALLS.resource_index)(&mut services, module, sounds, b"test.wav").unwrap(),
        1
    );
    let first = (ENGINE_CALLS.sound_id)(&mut services, module, sounds, 1)
        .unwrap()
        .unwrap();
    let revision = services.storage.config_revision(module).unwrap();
    services
        .storage
        .set_file_search(module, Some(selected[1].clone()), services.vfs)
        .unwrap();
    services.prepare_sound_bindings(module).unwrap();
    let second = (ENGINE_CALLS.sound_id)(&mut services, module, sounds, 1)
        .unwrap()
        .unwrap();
    assert_ne!(first, second);
    assert_eq!(services.storage.config_revision(module).unwrap(), revision);
    let bank = services.sound_loader.as_deref().unwrap().bank();
    assert_eq!(bank.sample(first).unwrap().samples[0], 1024);
    assert_eq!(bank.sample(second).unwrap().samples[0], 777);
    assert_eq!(bank.name(first), bank.name(second));
    services
        .storage
        .set_file_search(module, Some(selected[0].clone()), services.vfs)
        .unwrap();
    services.prepare_sound_bindings(module).unwrap();
    assert_eq!(
        (ENGINE_CALLS.sound_id)(&mut services, module, sounds, 1).unwrap(),
        Some(first)
    );
}

#[test]
fn full_bank_rejects_a_new_source_without_publishing_a_default_alias() {
    let files = Files::ready();
    let mount = files.vfs.mounts().next().unwrap().id;
    let selected = files.vfs.search_view([mount]).unwrap();
    let mut loader = SoundAssets::load(&files.vfs, 0, 0).unwrap();
    assert_eq!(
        loader.precache(&files.vfs, b"", RuleSetId::Quake2, Some(&selected)),
        Ok(None)
    );
    assert_eq!(
        loader.source(files.vfs.read_view(Some(&selected))).unwrap(),
        None
    );
    assert_eq!(
        loader.register(
            &files.vfs,
            Bank::DEFAULT_SOUND,
            RuleSetId::Quake3,
            Some(&selected)
        ),
        Err(CallError::Capacity)
    );
    assert_eq!(
        loader.source(files.vfs.read_view(Some(&selected))).unwrap(),
        None
    );
    assert_eq!(loader.reads, 0);
    assert_eq!(
        loader.register(&files.vfs, Bank::DEFAULT_SOUND, RuleSetId::Quake3, None),
        Ok(0)
    );
    assert_eq!(loader.reads, 1);
}

#[test]
fn pure_read_policy_cannot_reuse_a_warmed_unrestricted_sound_alias() {
    let files = Files::ready();
    let order = files
        .vfs
        .search_view(files.vfs.mounts().map(|mount| mount.id))
        .unwrap();
    let restricted = files
        .vfs
        .q3_search_view(Some(&order), &[123], false)
        .unwrap();
    let mut loader = SoundAssets::load(&files.vfs, 12, 1024).unwrap();
    let handle = loader
        .register(
            &files.vfs,
            b"sound/test.wav",
            RuleSetId::Quake3,
            Some(&order),
        )
        .unwrap();
    let source = loader
        .source(files.vfs.read_view(Some(&order)))
        .unwrap()
        .unwrap();
    let pcm = loader.bank().registered_sound_in(source, handle).unwrap();
    let decoded = loader.decodes;
    assert!(
        files
            .vfs
            .query_with(b"sound/test.wav", Some(&restricted))
            .is_some()
    );
    assert_eq!(
        loader.register(
            &files.vfs,
            b"sound/test.wav",
            RuleSetId::Quake3,
            Some(&restricted)
        ),
        Err(CallError::File)
    );
    assert_eq!(loader.decodes, decoded);
    let denied = loader
        .source(files.vfs.read_view(Some(&restricted)))
        .unwrap()
        .unwrap();
    assert_ne!(source, denied);
    assert_eq!(loader.bank().find_in(denied, b"sound/test.wav"), None);
    assert_eq!(loader.bank().registered_sound_in(source, handle), Some(pcm));
    assert_eq!(
        loader.register(
            &files.vfs,
            b"sound/test.wav",
            RuleSetId::Quake3,
            Some(&order)
        ),
        Ok(handle)
    );
}

#[test]
fn client_config_sounds_keep_each_connections_selected_pcm() {
    use qa_app::{Runtime, client_resources::ClientResources, host::FrameHost};
    use qa_console::{commands::Console, views::Context};
    use qa_core::primitives::ClientId;
    use qa_network::commands::packet::Protocol;
    use qa_session::timing::TickRate;
    let mut low = Files::ready();
    let mut changed = support::wav();
    changed[44..46].copy_from_slice(&777i16.to_le_bytes());
    let high = Files::new(&[("sound/test.wav", changed)]);
    let low_mount = low.vfs.mounts().next().unwrap().id;
    let high_mount = low.vfs.mount_directory(&high.root, 10).unwrap();
    let views = [
        low.vfs.search_view([low_mount]).unwrap(),
        low.vfs.search_view([high_mount]).unwrap(),
    ];
    let mut runtime = Runtime::load(4, []).unwrap();
    runtime.vfs = std::mem::take(&mut low.vfs);
    for (slot, protocol) in [Protocol::Quake2_34, Protocol::Quake3_68]
        .into_iter()
        .enumerate()
    {
        for (source, view) in views.iter().enumerate() {
            let index = slot * 2 + source;
            let mut resources = ClientResources::load(
                ClientId(index as u32),
                protocol,
                protocol.rule_set(),
                &runtime.vfs,
                Some(view),
            )
            .unwrap()
            .unwrap();
            assert!(resources.apply(
                resources.state.layout.resources[1].get(1).unwrap(),
                if protocol == Protocol::Quake3_68 {
                    b"sound/test.wav"
                } else {
                    b"test.wav"
                },
                None,
                None,
            ));
            runtime.client_resources[index] = Some(resources);
        }
    }
    let mut host = FrameHost::load(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        vec![],
    )
    .unwrap();
    // Warm the unrestricted winner first: it must never supply the low client.
    qa_app::audio::load_bank(&mut host, &["sound/test.wav".into()], RuleSetId::Quake3).unwrap();
    let before = host.runtime.sound_assets.as_ref().unwrap().reads;
    let ids = (0..4)
        .map(|slot| {
            let sound = host.runtime.client_resources[slot]
                .as_ref()
                .unwrap()
                .state
                .sound(1)
                .unwrap();
            assert_eq!(
                host.runtime
                    .sound_bank()
                    .unwrap()
                    .sample(sound)
                    .unwrap()
                    .samples[0],
                if slot % 2 == 0 { 1024 } else { 777 }
            );
            sound
        })
        .collect::<Vec<_>>();
    assert_ne!(ids[0], ids[1]);
    assert_eq!(ids[0], ids[2]);
    assert_eq!(ids[1], ids[3]);
    host.bind_sounds().unwrap();
    assert_eq!(host.runtime.sound_assets.as_ref().unwrap().reads, before);
}
