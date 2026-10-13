use qa_audio::Bank;
use qa_core::{
    names::NameTable,
    primitives::{NameId, Pcm, PcmChannels, SoundId},
};
use std::{num::NonZeroU32, sync::Arc};

fn rate() -> NonZeroU32 {
    NonZeroU32::new(44100).unwrap()
}

fn pcm(samples: &[i16]) -> Pcm {
    Pcm {
        rate: rate(),
        channels: PcmChannels::Mono,
        samples: samples.to_vec(),
        loop_start: None,
        loop_end: None,
        loop_invalid: false,
    }
}

#[test]
fn native_zero_default_and_failed_attempts_keep_registration_order() {
    let mut bank = Bank::with_capacity(rate(), 3, 128).unwrap();
    let default = bank.attempted(Bank::DEFAULT_SOUND).unwrap();
    assert_eq!(bank.registered_handle(Bank::DEFAULT_SOUND), Some(0));
    assert_eq!(bank.registered_sound(0), None);
    bank.admit(default, pcm(&[12, 13])).unwrap();
    assert_eq!(bank.registered_sound(0), Some(default));
    let (z, first) = bank
        .attempt_registration(b"sound/z.wav", Some(b"sound/z.wav"))
        .unwrap();
    assert!(first);
    bank.admit(z, pcm(&[20])).unwrap();
    assert_eq!(bank.registered_handle(b"SOUND/Z.WAV"), Some(1));
    let (failed, _) = bank
        .attempt_registration(b"sound/missing.wav", Some(b"sound/missing.wav"))
        .unwrap();
    assert_eq!(bank.registered_handle(b"sound/missing.wav"), Some(0));
    assert_eq!(bank.registered_sound(2), None);
    let (a, _) = bank
        .attempt_registration(b"sound/a.wav", Some(b"sound/a.wav"))
        .unwrap();
    bank.admit(a, pcm(&[30])).unwrap();
    assert_eq!(bank.registered_handle(b"sound/a.wav"), Some(3));
    assert_eq!(bank.registered_sound(3), Some(a));
    let count = bank.len();
    for _ in 0..100 {
        assert_eq!(bank.attempt(b"SOUND\\MISSING.WAV").unwrap(), failed);
        assert_eq!(bank.attempt(b"sound/z.wav").unwrap(), z);
        assert_eq!(bank.registered_handle(b"sound/missing.wav"), Some(0));
        assert_eq!(bank.find_bytes(b"sound/missing.wav"), None);
        assert_eq!(bank.registered_sound(-1), None);
        assert_eq!(bank.registered_sound(i32::MAX), None);
    }
    assert_eq!(bank.len(), count);
    assert_eq!(bank.registered_handle(b"sound/unattempted.wav"), None);
    let bank = Arc::new(bank);
    assert_eq!(bank.registered_sound(0), Some(default));
    assert_eq!(bank.find_bytes(b"SOUND/Z.WAV"), Some(z));
    assert_eq!(bank.name(failed), Some(b"sound/missing.wav".as_slice()));
}

#[test]
fn bulk_load_retains_lexical_sound_ids_and_uses_source_order_for_native_handles() {
    let paths = ["sound/z.wav", "sound/a.wav", "sound/m.wav"];
    let old_names = NameTable::load(paths.iter().map(|path| path.as_bytes())).unwrap();
    let bank = Bank::load(
        rate(),
        paths
            .iter()
            .map(|path| (path.to_string(), pcm(&[1, 2])))
            .collect(),
    )
    .unwrap();
    for (index, path) in paths.into_iter().enumerate() {
        let name = old_names.find(path.as_bytes()).unwrap();
        let sound = bank.find(path).unwrap();
        assert_eq!(sound, SoundId(name.0));
        assert_eq!(
            bank.registered_handle(path.as_bytes()),
            Some(index as i32 + 1)
        );
        assert_eq!(bank.registered_sound(index as i32 + 1), Some(sound));
        assert_eq!(bank.sample(sound).unwrap().samples, [1, 2]);
    }
    assert_eq!(bank.name(SoundId(0)), Some(b"".as_slice()));
    assert_eq!(old_names.get(NameId(0)), bank.name(SoundId(0)));
    assert_eq!(bank.registered_sound(0), None);
}

#[test]
fn staged_and_bulk_loading_share_pcm_preparation_and_name_semantics() {
    let make = || Pcm {
        rate: NonZeroU32::new(22050).unwrap(),
        channels: PcmChannels::Stereo,
        samples: vec![1, -1, 3, -3, 5, -5],
        loop_start: Some(1),
        loop_end: None,
        loop_invalid: false,
    };
    let path = b"Sound\\TEST.WAV";
    let mut staged = Bank::with_capacity(rate(), 1, path.len()).unwrap();
    let sound = staged.attempt(path).unwrap();
    staged.admit(sound, make()).unwrap();
    let bulk = Bank::load(rate(), vec![("sound/test.wav".into(), make())]).unwrap();
    let a = staged.sample(sound).unwrap();
    let b = bulk.sample(bulk.find("SOUND\\TEST.WAV").unwrap()).unwrap();
    assert_eq!(a.rate, b.rate);
    assert_eq!(a.channels, b.channels);
    assert_eq!(a.samples, b.samples);
    assert_eq!(a.loop_start, b.loop_start);
    assert_eq!(
        staged.name(sound),
        bulk.name(bulk.find("sound/test.wav").unwrap())
    );
}

#[test]
fn rejected_pcm_or_capacity_keeps_prior_attempts_and_samples_intact() {
    let mut bank = Bank::with_capacity(rate(), 2, 11).unwrap();
    let (good, _) = bank
        .attempt_registration(b"sound/a.wav", Some(b"sound/a.wav"))
        .unwrap();
    bank.admit(good, pcm(&[77])).unwrap();
    let (bad, _) = bank
        .attempt_registration(b"sound/b.wav", Some(b"sound/b.wav"))
        .unwrap();
    assert_eq!(bank.admit(bad, pcm(&[])).unwrap_err(), "empty sound");
    assert!(
        bank.admit(
            bad,
            Pcm {
                rate: rate(),
                channels: PcmChannels::Stereo,
                samples: vec![3],
                loop_start: None,
                loop_end: None,
                loop_invalid: false,
            }
        )
        .is_err()
    );
    assert_eq!(bank.registered_handle(b"sound/b.wav"), Some(0));
    assert_eq!(bank.find_bytes(b"sound/b.wav"), None);
    let count = bank.len();
    assert!(bank.attempt(b"sound/c.wav").is_err());
    assert_eq!(bank.len(), count);
    assert_eq!(bank.attempted(b"sound/c.wav"), None);
    assert_eq!(bank.sample(good).unwrap().samples, [77]);
    assert_eq!(bank.registered_handle(b"sound/a.wav"), Some(1));
    assert_eq!(bank.registered_sound(1), Some(good));
    assert_eq!(bank.attempt(b"sound/b.wav").unwrap(), bad);
    assert!(bank.admit(good, pcm(&[99])).is_err());
    assert_eq!(bank.sample(good).unwrap().samples, [77]);
    assert!(bank.admit(SoundId(u32::MAX), pcm(&[1])).is_err());
    assert!(Bank::with_capacity(rate(), usize::MAX, 0).is_err());
}

#[test]
fn native_default_remains_zero_even_if_bulk_source_lists_it_last() {
    let bank = Bank::load(
        rate(),
        vec![
            ("sound/a.wav".into(), pcm(&[1])),
            (
                String::from_utf8(Bank::DEFAULT_SOUND.to_vec()).unwrap(),
                pcm(&[2]),
            ),
            ("sound/b.wav".into(), pcm(&[3])),
        ],
    )
    .unwrap();
    assert_eq!(bank.registered_handle(b"sound/a.wav"), Some(1));
    assert_eq!(bank.registered_handle(Bank::DEFAULT_SOUND), Some(0));
    assert_eq!(bank.registered_handle(b"sound/b.wav"), Some(2));
    let default = bank.find_bytes(Bank::DEFAULT_SOUND).unwrap();
    assert_eq!(bank.registered_sound(0), Some(default));
    assert_eq!(bank.sample(default).unwrap().samples, [2]);
}

#[test]
fn bulk_duplicate_canonical_names_keep_the_existing_rejection() {
    let error = Bank::load(
        rate(),
        vec![
            ("Sound\\TEST.WAV".into(), pcm(&[1])),
            ("sound/test.wav".into(), pcm(&[2])),
        ],
    )
    .err()
    .expect("duplicate canonical registration rejects the load");
    assert_eq!(error, "duplicate sound registration");
}

#[test]
fn native_separator_names_have_distinct_handles_but_one_pcm_attempt() {
    let mut bank = Bank::with_capacity(rate(), 2, 64).unwrap();
    let (sound, first) = bank
        .attempt_registration(b"sound\\a.wav", Some(b"sound/a.wav"))
        .unwrap();
    assert!(first);
    bank.admit(sound, pcm(&[12, 13])).unwrap();
    assert_eq!(bank.registered_handle(b"SOUND\\A.WAV"), Some(1));
    assert_eq!(bank.registered_handle(b"sound/a.wav"), None);
    let before = bank.len();
    let (case, first) = bank
        .attempt_registration(b"SOUND\\A.WAV", Some(b"sound/a.wav"))
        .unwrap();
    assert_eq!(case, sound);
    assert!(!first);
    assert_eq!(bank.len(), before);
    let (slash, first) = bank
        .attempt_registration(b"sound/a.wav", Some(b"sound/a.wav"))
        .unwrap();
    assert_eq!(slash, sound);
    assert!(!first);
    assert_eq!(bank.registered_handle(b"sound/a.wav"), Some(2));
    assert_eq!(bank.registered_sound(1), Some(sound));
    assert_eq!(bank.registered_sound(2), Some(sound));
    assert_eq!(bank.find_bytes(b"SOUND\\A.WAV"), Some(sound));
    assert_eq!(bank.sample(sound).unwrap().samples, [12, 13]);
}

#[test]
fn pcm_precache_does_not_consume_native_handles_and_native_failures_do() {
    let mut bank = Bank::with_capacity(rate(), 4, 128).unwrap();
    let preloaded = bank.attempt(b"sound/a.wav").unwrap();
    bank.admit(preloaded, pcm(&[1])).unwrap();
    assert_eq!(bank.registered_handle(b"sound/a.wav"), None);
    let (dummy, first) = bank.attempt_registration(b"*player.wav", None).unwrap();
    assert_eq!(dummy, SoundId(0));
    assert!(!first);
    assert_eq!(bank.registered_handle(b"*PLAYER.WAV"), Some(0));
    assert_eq!(bank.registered_sound(1), None);
    assert_eq!(
        bank.attempt_registration(b"*PLAYER.WAV", None).unwrap(),
        (SoundId(0), false)
    );
    let (sound, first) = bank
        .attempt_registration(b"sound/a.wav", Some(b"sound/a.wav"))
        .unwrap();
    assert_eq!(sound, preloaded);
    assert!(!first);
    assert_eq!(bank.registered_handle(b"sound/a.wav"), Some(2));
    let literal = bank.attempt(b"*player.wav").unwrap();
    bank.admit(literal, pcm(&[9])).unwrap();
    assert_eq!(bank.find_bytes(b"*PLAYER.WAV"), Some(literal));
    assert_eq!(bank.registered_sound(1), None);
    assert_eq!(bank.registered_handle(b"*player.wav"), Some(0));
}

#[test]
fn rejecting_native_stereo_keeps_ready_shared_pcm_and_failed_handle_order() {
    let mut bank = Bank::with_capacity(rate(), 2, 64).unwrap();
    let (sound, _) = bank
        .attempt_registration(b"sound/stereo.wav", Some(b"sound/stereo.wav"))
        .unwrap();
    bank.admit(
        sound,
        Pcm {
            rate: rate(),
            channels: PcmChannels::Stereo,
            samples: vec![1, -1],
            loop_start: None,
            loop_end: None,
            loop_invalid: false,
        },
    )
    .unwrap();
    assert_eq!(bank.registered_handle(b"sound/stereo.wav"), Some(1));
    bank.reject_registration(b"SOUND/STEREO.WAV").unwrap();
    assert_eq!(bank.registered_handle(b"sound/stereo.wav"), Some(0));
    assert_eq!(bank.registered_sound(1), None);
    assert_eq!(bank.find_bytes(b"sound/stereo.wav"), Some(sound));
    assert_eq!(bank.sample(sound).unwrap().samples, [1, -1]);
    assert_eq!(
        bank.attempt_registration(b"sound/stereo.wav", Some(b"sound/stereo.wav"))
            .unwrap(),
        (sound, false)
    );
    assert_eq!(bank.registered_sound(1), None);
    let (later, _) = bank
        .attempt_registration(b"sound/later.wav", Some(b"sound/later.wav"))
        .unwrap();
    bank.admit(later, pcm(&[7])).unwrap();
    assert_eq!(bank.registered_handle(b"sound/later.wav"), Some(2));
    assert!(bank.reject_registration(b"sound/unseen.wav").is_err());
}
