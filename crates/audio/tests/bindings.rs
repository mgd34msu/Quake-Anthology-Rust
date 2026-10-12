use qa_audio::{Bank, SoundBindingError, SoundBindings, sound_path};
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
    bindings.bind_name(Some(&bank), 3, b"a.wav").unwrap();
    bindings.bind_name(Some(&bank), 1, b"A.WAV").unwrap();
    assert_eq!(
        (bindings.sound(1), bindings.sound(3), bindings.ordinal(a)),
        (Some(a), Some(a), Some(1))
    );
    bindings.bind_name(Some(&bank), 1, b"b.wav").unwrap();
    assert_eq!(
        (bindings.ordinal(a), bindings.ordinal(b)),
        (Some(3), Some(1))
    );
    bindings.bind_name(Some(&bank), 3, b"*player.wav").unwrap();
    assert_eq!((bindings.sound(3), bindings.ordinal(a)), (None, None));
    bindings.bind_name(Some(&bank), 2, b"#music/c.wav").unwrap();
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
    bindings.bind_name(Some(&first), 1, b"a.wav").unwrap();
    assert_eq!(bindings.ordinal(first_sound), Some(1));
    bindings.attach(&second);
    assert_eq!(
        (bindings.sound(1), bindings.ordinal(first_sound)),
        (None, None)
    );
    bindings.bind_name(Some(&second), 1, b"b.wav").unwrap();
    assert_eq!(bindings.ordinal(second_sound), Some(1));
    assert!(sound_path(b"*player.wav").unwrap().is_none());
    assert!(sound_path(b"").unwrap().is_none());
    assert_eq!(
        sound_path(b"a.wav").unwrap().unwrap().as_bytes(),
        b"sound/a.wav"
    );
    assert_eq!(
        sound_path(b"#music/c.wav").unwrap().unwrap().as_bytes(),
        b"music/c.wav"
    );
}
