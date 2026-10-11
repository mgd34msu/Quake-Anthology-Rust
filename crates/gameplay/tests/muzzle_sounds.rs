use qa_core::primitives::RuleSetId;
use qa_gameplay::rules::q2::sounds::{PATHS, monster, player};

#[test]
fn player_product_differences_and_bursts() {
    let q2 = RuleSetId::Quake2;
    let rr = RuleSetId::Quake2Rerelease;
    for (flash, classic, rerelease) in [(2, 0.1, 0.35), (7, 0.1, 0.15), (8, 0.1, 0.15)] {
        assert_eq!(player(q2, flash)[1].delay, classic);
        assert_eq!(player(rr, flash)[1].delay, rerelease);
        assert_eq!(player(q2, flash)[1].channel, 0);
    }
    assert_eq!(player(q2, 6).len(), 1);
    assert_eq!(player(rr, 6)[1].channel, 7);
    assert_eq!(player(rr, 6)[1].delay, 0.4);
    assert_eq!(
        PATHS[player(q2, 32)[0].choices.first as usize],
        "sound/weapons/shotg2.wav"
    );
    assert_eq!(
        PATHS[player(rr, 32)[0].choices.first as usize],
        "sound/weapons/nail1.wav"
    );
    assert!(player(q2, 31).is_empty());
    assert_eq!(player(rr, 31)[1].delay, 0.15);
    for (flash, delays) in [
        (3, &[0.0][..]),
        (4, &[0.0, 0.05][..]),
        (5, &[0.0, 0.033, 0.066][..]),
    ] {
        let sounds = player(q2, flash);
        assert_eq!(sounds.len(), delays.len());
        for (sound, delay) in sounds.iter().zip(delays) {
            assert_eq!(sound.delay, *delay);
            assert_eq!(sound.choices.count, 5);
            assert_eq!(sound.channel, 1);
        }
    }
    for flash in 9..=11 {
        assert!(player(q2, flash)[0].full_volume);
    }
    assert!(!player(q2, 0)[0].full_volume);
    assert_eq!(
        player(q2, 0)[0].volume(q2, true).to_bits(),
        0.2f32.to_bits()
    );
    assert_eq!(
        player(rr, 0)[0].volume(rr, true).to_bits(),
        (1.0f32 - 0.8).to_bits()
    );
    assert_eq!(player(rr, 9)[0].volume(rr, true), 1.0);
    for flash in [15, 19, 20, 33, 36, 37, 38, 39, 127] {
        assert!(player(rr, flash).is_empty(), "silent flash {flash}");
    }
}

#[test]
fn monster_sound_groups_and_silent_second_barrels() {
    let q2 = RuleSetId::Quake2;
    let rr = RuleSetId::Quake2Rerelease;
    for (flash, path) in [
        (1, "tank/tnkatck3.wav"),
        (26, "infantry/infatck1.wav"),
        (39, "soldier/solatck2.wav"),
        (41, "soldier/solatck1.wav"),
        (43, "soldier/solatck3.wav"),
        (45, "gunner/gunatck2.wav"),
        (53, "gunner/gunatck3.wav"),
        (57, "chick/chkatck2.wav"),
        (58, "flyer/flyatck3.wav"),
        (60, "medic/medatck1.wav"),
        (62, "hover/hovatck1.wav"),
        (70, "tank/rocket.wav"),
        (82, "floater/fltatck1.wav"),
        (102, "makron/blaster.wav"),
        (120, "boss3/xfire.wav"),
        (148, "weapons/disint2.wav"),
        (211, "weapons/rippfire.wav"),
        (219, "weapons/hyprbf1a.wav"),
        (240, "guncmdr/gcdratck2.wav"),
        (242, "guncmdr/gcdratck3.wav"),
    ] {
        let sound = monster(rr, flash)[0];
        assert_eq!(
            PATHS[sound.choices.first as usize].strip_prefix("sound/"),
            Some(path)
        );
        assert_eq!(sound.channel, 1);
        assert!(sound.full_volume);
    }
    assert_eq!(monster(q2, 4)[0].choices.count, 5);
    assert!(monster(q2, 211).is_empty());
    for flash in [0, 61, 101, 119, 126, 133, 195, 289, 2047] {
        assert!(
            monster(rr, flash).is_empty(),
            "silent monster flash {flash}"
        );
    }
    for rules in [q2, rr] {
        assert_eq!(monster(rules, 73)[0].attenuation, 0.0);
    }
    assert_eq!(
        PATHS[monster(q2, 74)[0].choices.first as usize],
        "sound/infantry/infatck1.wav"
    );
    assert_eq!(
        PATHS[monster(rr, 74)[0].choices.first as usize],
        "sound/flyer/flyatck3.wav"
    );
    assert!(monster(q2, 134).is_empty());
    assert_eq!(monster(rr, 134)[0].attenuation, 0.0);
}

#[test]
fn all_recipe_ranges_are_load_bindable_and_other_games_have_no_q2_table() {
    for rules in RuleSetId::ALL {
        for flash in 0..=2047 {
            for sounds in [player(rules, flash), monster(rules, flash)] {
                for sound in sounds {
                    assert!(sound.choices.count > 0);
                    assert!(sound.choices.range().end <= PATHS.len());
                    assert_eq!(sound.choices.range().len(), sound.choices.count as usize);
                }
                if !matches!(rules, RuleSetId::Quake2 | RuleSetId::Quake2Rerelease) {
                    assert!(sounds.is_empty());
                }
            }
        }
    }
}
