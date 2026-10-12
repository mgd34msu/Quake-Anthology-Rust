use qa_core::{
    configstrings::{ConfigError, ConfigLayout, ConfigStrings},
    primitives::RuleSetId,
};

#[test]
fn rows_keep_raw_bytes_cached_integers_and_stable_load_storage() {
    let mut strings = ConfigStrings::load([4, 24, 0]).unwrap();
    let address = strings.config(1).unwrap().text.as_ptr();
    assert!(strings.set(0, b"a\x80\xff").unwrap());
    assert!(strings.set(1, b" -214748364999suffix").unwrap());
    assert_eq!(strings.config(1).unwrap().integer, i32::MIN);
    assert_eq!(strings.config(1).unwrap().text.as_ptr(), address);
    let first = strings.config(0).unwrap();
    assert_eq!(first.text, b"a\x80\xff");
    assert_eq!(first.revision, 1);
    assert!(!strings.set(0, b"a\x80\xff").unwrap());
    assert_eq!(strings.config(0).unwrap().revision, 1);
    assert_eq!(strings.set(0, b"12345"), Err(ConfigError::Text));
    assert_eq!(strings.set(0, b"x\0y"), Err(ConfigError::Text));
    assert_eq!(strings.config(0).unwrap().text, b"a\x80\xff");
    assert_eq!(strings.config(0).unwrap().revision, 1);
    assert_eq!(strings.set(3, b""), Err(ConfigError::Index));
    assert!(!strings.set(2, b"").unwrap());
    assert_eq!(strings.set(2, b"x"), Err(ConfigError::Text));
    strings.reset();
    assert_eq!(strings.config(0).unwrap().revision, 2);
    assert_eq!(strings.config(1).unwrap().integer, 0);
    assert_eq!(strings.config(1).unwrap().text.as_ptr(), address);
    strings.reset();
    assert_eq!(strings.config(0).unwrap().revision, 2);
    assert_eq!(strings.len(), 3);
    assert!(!strings.is_empty());
}

#[test]
fn native_capacities_match_cs_size_and_resources_project_by_ordinal() {
    let classic = ConfigLayout::QUAKE2;
    let rerelease = ConfigLayout::QUAKE2_RERELEASE;
    let q3 = ConfigLayout::QUAKE3;
    assert_eq!(
        (classic.count, rerelease.count, q3.count),
        (2080, 12448, 1024)
    );
    assert_eq!(classic.capacity(0), Some(63));
    assert_eq!(classic.capacity(5), Some(24 * 64 - 1));
    assert_eq!(rerelease.capacity(5), Some(54 * 96 - 1));
    assert_eq!(rerelease.capacity(59), Some(95));
    assert_eq!(rerelease.capacity(11838), Some(8192));
    assert_eq!(
        rerelease.capacity(12349),
        Some(99 * 96 - 1).map(|n| n.min(8192))
    );
    assert_eq!(rerelease.capacity(12350), Some(95));
    assert_eq!(rerelease.capacity(12448), None);
    assert_eq!(q3.capacity(0), Some(8191));
    for rules in [
        RuleSetId::Quake,
        RuleSetId::QuakeWorld,
        RuleSetId::Quake2,
        RuleSetId::Quake2Rerelease,
        RuleSetId::Quake3,
    ] {
        let layout = ConfigLayout::for_rules(rules);
        for index in 0..layout.count {
            assert_eq!(layout.project_index(layout, index), Some(index));
        }
        assert_eq!(layout.project_index(layout, layout.count), None);
    }
    for ordinal in 0..256 {
        assert_eq!(
            classic.project_index(rerelease, 62 + ordinal),
            Some(32 + ordinal)
        );
        assert_eq!(
            rerelease.project_index(classic, 288 + ordinal),
            Some(8254 + ordinal)
        );
        assert_eq!(
            q3.project_index(rerelease, 62 + ordinal),
            Some(32 + ordinal)
        );
    }
    assert_eq!(classic.project_index(rerelease, 62 + 256), None);
    assert_eq!(q3.project_index(rerelease, 10302), None);
    assert_eq!(classic.project_index(rerelease, 59), Some(29));
    assert_eq!(rerelease.project_index(classic, 31), Some(61));
    assert_eq!(classic.project_index(rerelease, 10814 + 17), Some(800 + 17));
    assert_eq!(q3.project_index(rerelease, 11582 + 63), Some(544 + 63));
    assert_eq!(q3.project_index(rerelease, 11582 + 64), None);
    assert_eq!(q3.project_index(rerelease, 0), None);
}

#[test]
fn quake_storage_keeps_resource_limits_and_bounds_native_lightstyles() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let layout = ConfigLayout::for_rules(rules);
        assert_eq!(layout.count, 576);
        assert_eq!(layout.resources[0].range(), 0..256);
        assert_eq!(layout.resources[1].range(), 256..512);
        assert!(layout.resources[2].range().is_empty());
        assert_eq!(layout.lights.range(), 512..576);
        for index in 0..512 {
            assert_eq!(layout.capacity(index), Some(MAX_RESOURCE_BYTES));
        }
        for ordinal in 0..64 {
            assert_eq!(layout.style_index(ordinal), Some(512 + ordinal));
            assert_eq!(layout.capacity(512 + ordinal), Some(63));
        }
        assert_eq!(layout.style_index(64), None);
        assert_eq!(layout.style_index(usize::MAX), None);
        assert_eq!(layout.capacity(576), None);
        let mut rows =
            ConfigStrings::load((0..layout.count).map(|index| layout.capacity(index).unwrap()))
                .unwrap();
        rows.set(1, &vec![b'x'; MAX_RESOURCE_BYTES]).unwrap();
        rows.set(512, b"a\x80\xff").unwrap();
        let old = rows.config(512).unwrap();
        let revision = old.revision;
        assert_eq!(rows.set(512, &[b'm'; 64]), Err(ConfigError::Text));
        assert_eq!(rows.config(512).unwrap().text, b"a\x80\xff");
        assert_eq!(rows.config(512).unwrap().revision, revision);
        rows.set(575, &[b'm'; 63]).unwrap();
        rows.set(512, b"").unwrap();
        assert!(rows.config(512).unwrap().text.is_empty());
        assert_eq!(rows.config(1).unwrap().text.len(), MAX_RESOURCE_BYTES);
    }
}

const MAX_RESOURCE_BYTES: usize = qa_core::configstrings::MAX_TEXT;

#[test]
fn cross_source_lightstyles_share_ordinals_without_scalar_collisions() {
    let nq = ConfigLayout::QUAKE;
    let qw = ConfigLayout::QUAKEWORLD;
    let q2 = ConfigLayout::QUAKE2;
    let rr = ConfigLayout::QUAKE2_RERELEASE;
    let q3 = ConfigLayout::QUAKE3;
    for source in [nq, qw] {
        for ordinal in 0..64 {
            let index = source.style_index(ordinal).unwrap();
            assert_eq!(q2.project_index(source, index), q2.style_index(ordinal));
            assert_eq!(rr.project_index(source, index), rr.style_index(ordinal));
            assert_eq!(q3.project_index(source, index), None);
            assert_eq!(qw.project_index(source, index), Some(512 + ordinal));
        }
        assert_eq!(q2.project_index(source, 1), Some(33));
        assert_eq!(q3.project_index(source, 257), Some(289));
        for legacy in [q2, rr] {
            for index in [0, 1, 2, 3, 4] {
                assert_eq!(source.project_index(legacy, index), None);
            }
            let scalars = if legacy == q2 {
                [29, 30, 31]
            } else {
                [59, 60, 61]
            };
            for index in scalars {
                assert_eq!(source.project_index(legacy, index), None);
            }
            assert_eq!(
                source.project_index(legacy, legacy.style_index(63).unwrap()),
                Some(575)
            );
            assert_eq!(
                source.project_index(legacy, legacy.style_index(64).unwrap()),
                None
            );
        }
    }
    assert_eq!(q3.style_index(0), None);
}
