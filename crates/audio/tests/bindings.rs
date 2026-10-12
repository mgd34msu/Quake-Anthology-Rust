use qa_audio::{
    Bank, SoundBindingError, SoundBindings, native_sound_name, same_sound_codec, sound_path,
};
use qa_core::primitives::{Pcm, PcmChannels, SoundId};
use std::num::NonZeroU32;

fn bank(names: &[&str]) -> Bank {
    let rate = NonZeroU32::new(44100).unwrap();
    Bank::load(
        rate,
        names
            .iter()
            .map(|name| {
                (
                    name.to_string(),
                    Pcm {
                        rate,
                        channels: PcmChannels::Mono,
                        samples: vec![1],
                        loop_start: None,
                    },
                )
            })
            .collect(),
    )
    .unwrap()
}

#[test]
fn numeric_bindings_share_registration_and_choose_the_lowest_nonzero_ordinal() {
    let bank = bank(&["sound/a.wav", "sound/b.wav", "music/c.wav"]);
    let a = bank.find("sound/a.wav").unwrap();
    let b = bank.find("sound/b.wav").unwrap();
    let mut bindings = SoundBindings::load(5);
    bindings.attach(&bank);
    bindings
        .bind_name(
            Some(&bank),
            3,
            b"a.wav",
            qa_core::primitives::RuleSetId::Quake2,
        )
        .unwrap();
    bindings
        .bind_name(
            Some(&bank),
            1,
            b"A.WAV",
            qa_core::primitives::RuleSetId::Quake2,
        )
        .unwrap();
    assert_eq!(
        (bindings.sound(1), bindings.sound(3), bindings.ordinal(a)),
        (Some(a), Some(a), Some(1))
    );
    bindings
        .bind_name(
            Some(&bank),
            1,
            b"b.wav",
            qa_core::primitives::RuleSetId::Quake2,
        )
        .unwrap();
    assert_eq!(
        (bindings.ordinal(a), bindings.ordinal(b)),
        (Some(3), Some(1))
    );
    bindings
        .bind_name(
            Some(&bank),
            3,
            b"*player.wav",
            qa_core::primitives::RuleSetId::Quake2,
        )
        .unwrap();
    assert_eq!((bindings.sound(3), bindings.ordinal(a)), (None, None));
    bindings
        .bind_name(
            Some(&bank),
            2,
            b"#music/c.wav",
            qa_core::primitives::RuleSetId::Quake2,
        )
        .unwrap();
    assert_eq!(bindings.sound(2), bank.find("music/c.wav"));
    bindings.set(0, Some(a)).unwrap();
    assert_eq!(bindings.sound(0), None);
    assert_eq!(bindings.set(5, Some(a)), Err(SoundBindingError::Ordinal));
    assert_eq!(
        bindings.set(1, Some(SoundId(bank.len() as u32))),
        Err(SoundBindingError::Sound)
    );
    assert_eq!(bindings.sound(1), Some(b));
}

#[test]
fn bank_reattachment_clears_old_handles_even_when_numeric_ids_are_reused() {
    let first = bank(&["sound/a.wav"]);
    let second = bank(&["sound/b.wav"]);
    let first_sound = first.find("sound/a.wav").unwrap();
    let second_sound = second.find("sound/b.wav").unwrap();
    assert_eq!(first_sound, second_sound);
    let mut bindings = SoundBindings::load(2);
    bindings.attach(&first);
    bindings
        .bind_name(
            Some(&first),
            1,
            b"a.wav",
            qa_core::primitives::RuleSetId::Quake2,
        )
        .unwrap();
    assert_eq!(bindings.ordinal(first_sound), Some(1));
    bindings.attach(&second);
    assert_eq!(
        (bindings.sound(1), bindings.ordinal(first_sound)),
        (None, None)
    );
    bindings
        .bind_name(
            Some(&second),
            1,
            b"b.wav",
            qa_core::primitives::RuleSetId::Quake2,
        )
        .unwrap();
    assert_eq!(bindings.ordinal(second_sound), Some(1));
    assert!(
        sound_path(b"*player.wav", qa_core::primitives::RuleSetId::Quake2)
            .unwrap()
            .is_none()
    );
    assert!(
        sound_path(b"", qa_core::primitives::RuleSetId::Quake2)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        sound_path(b"a.wav", qa_core::primitives::RuleSetId::Quake2)
            .unwrap()
            .unwrap()
            .as_bytes(),
        b"sound/a.wav"
    );
    assert_eq!(
        sound_path(b"#music/c.wav", qa_core::primitives::RuleSetId::Quake2)
            .unwrap()
            .unwrap()
            .as_bytes(),
        b"music/c.wav"
    );
}

#[test]
fn decoded_paths_use_shared_bindings_and_keep_zero_reserved_after_reattachment() {
    let first = bank(&["sound/weapons/blastf1a.wav"]);
    let second = bank(&["sound/other.wav"]);
    let mut bindings = SoundBindings::load(2);
    bindings.attach(&first);
    let blaster = first.find("sound/weapons/blastf1a.wav").unwrap();
    bindings
        .bind_path(Some(&first), 1, b"sound/weapons/blastf1a.wav")
        .unwrap();
    assert_eq!(bindings.sound(1), Some(blaster));
    assert_eq!(bindings.ordinal(blaster), Some(1));
    bindings
        .bind_path(Some(&first), 0, b"sound/weapons/blastf1a.wav")
        .unwrap();
    assert_eq!(bindings.sound(0), None);
    assert_eq!(
        bindings.bind_path(Some(&first), 1, b"sound/\xff.wav"),
        Err(SoundBindingError::Text)
    );
    assert_eq!(bindings.sound(1), Some(blaster));
    bindings.attach(&second);
    bindings
        .bind_path(Some(&second), 1, b"sound/weapons/blastf1a.wav")
        .unwrap();
    assert_eq!(bindings.sound(1), None);
    assert_eq!(bindings.ordinal(blaster), None);
    bindings
        .bind_path(Some(&second), 1, b"sound/other.wav")
        .unwrap();
    assert_eq!(bindings.sound(1), second.find("sound/other.wav"));
}

#[test]
fn q3_full_paths_and_q2_bare_paths_bind_the_same_loaded_sample() {
    use qa_core::primitives::RuleSetId;
    let bank = bank(&["sound/a.wav", "music/c.wav"]);
    let a = bank.find("sound/a.wav").unwrap();
    let mut bindings = SoundBindings::load(3);
    bindings.attach(&bank);
    bindings
        .bind_name(Some(&bank), 1, b"sound/a.wav", RuleSetId::Quake3)
        .unwrap();
    bindings
        .bind_name(Some(&bank), 2, b"a.wav", RuleSetId::Quake2)
        .unwrap();
    assert_eq!((bindings.sound(1), bindings.sound(2)), (Some(a), Some(a)));
    assert_eq!(
        sound_path(b"sound/a.wav", RuleSetId::Quake3)
            .unwrap()
            .unwrap()
            .as_bytes(),
        b"sound/a.wav"
    );
    assert!(
        sound_path(b"*player.wav", RuleSetId::Quake3)
            .unwrap()
            .is_none()
    );
    bindings.prefer(2);
    assert_eq!(bindings.ordinal(a), Some(2));
    bindings.clear();
    assert_eq!(
        (bindings.sound(1), bindings.sound(2), bindings.ordinal(a)),
        (None, None, None)
    );
}

#[test]
fn quake_filename_prefixes_are_literal_while_q2_resources_keep_native_markers() {
    use qa_core::primitives::RuleSetId;
    let bank = bank(&["sound/#foo.wav", "sound/*foo.wav", "foo.wav"]);
    let mut bindings = SoundBindings::load(4);
    bindings.attach(&bank);
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        bindings
            .bind_name(Some(&bank), 1, b"#foo.wav", rules)
            .unwrap();
        bindings
            .bind_name(Some(&bank), 2, b"*foo.wav", rules)
            .unwrap();
        assert_eq!(bindings.sound(1), bank.find("sound/#foo.wav"));
        assert_eq!(bindings.sound(2), bank.find("sound/*foo.wav"));
    }
    for rules in [RuleSetId::Quake2, RuleSetId::Quake2Rerelease] {
        bindings
            .bind_name(Some(&bank), 1, b"#foo.wav", rules)
            .unwrap();
        bindings
            .bind_name(Some(&bank), 2, b"*foo.wav", rules)
            .unwrap();
        assert_eq!(bindings.sound(1), bank.find("foo.wav"));
        assert_eq!(bindings.sound(2), None);
        bindings
            .bind_name(Some(&bank), 3, b"#sound/*foo.wav", rules)
            .unwrap();
        assert_eq!(bindings.sound(3), bank.find("sound/*foo.wav"));
    }
}

#[test]
fn forward_and_inverse_filename_codecs_preserve_literal_and_escaped_paths() {
    use qa_core::primitives::RuleSetId;
    let bank = bank(&[
        "sound/#foo.wav",
        "sound/*foo.wav",
        "sound/a.wav",
        "music/a.wav",
    ]);
    let rules = [
        RuleSetId::Quake,
        RuleSetId::QuakeWorld,
        RuleSetId::Quake2,
        RuleSetId::Quake2Rerelease,
        RuleSetId::Quake3,
    ];
    for source in rules {
        for target in rules {
            assert_eq!(
                same_sound_codec(source, target),
                source == target
                    || matches!(
                        (source, target),
                        (RuleSetId::Quake, RuleSetId::QuakeWorld)
                            | (RuleSetId::QuakeWorld, RuleSetId::Quake)
                            | (RuleSetId::Quake2, RuleSetId::Quake2Rerelease)
                            | (RuleSetId::Quake2Rerelease, RuleSetId::Quake2)
                    )
            );
        }
    }
    for target in rules {
        for path in [
            "sound/#foo.wav",
            "sound/*foo.wav",
            "sound/a.wav",
            "music/a.wav",
        ] {
            let sound = bank.find(path).unwrap();
            let name = native_sound_name(target, &bank, sound);
            if matches!(target, RuleSetId::Quake | RuleSetId::QuakeWorld)
                && path.starts_with("music/")
            {
                assert!(name.is_none());
            } else {
                let name = name.unwrap();
                assert_eq!(
                    sound_path(name.as_bytes(), target)
                        .unwrap()
                        .unwrap()
                        .as_bytes(),
                    path.as_bytes()
                );
            }
        }
    }
    let literal = bank.find("sound/#foo.wav").unwrap();
    assert_eq!(
        native_sound_name(RuleSetId::Quake, &bank, literal)
            .unwrap()
            .as_bytes(),
        b"#foo.wav"
    );
    assert_eq!(
        native_sound_name(RuleSetId::Quake2, &bank, literal)
            .unwrap()
            .as_bytes(),
        b"#sound/#foo.wav"
    );
    assert!(
        sound_path(b"*player.wav", RuleSetId::Quake2)
            .unwrap()
            .is_none()
    );
}
