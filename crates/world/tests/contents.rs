use qa_world::collision::Contents;

#[test]
fn auxiliary_contents_survives_native_q2_conversion_and_masks() {
    assert_eq!(Contents::from_q2(4), Contents::AUX);
    assert_eq!(Contents::from_q2(4 | 32), Contents::AUX | Contents::WATER);
    assert!(Contents::from_q2(4 | 32).intersects(Contents::AUX));
    assert!(!Contents::from_q2(32).intersects(Contents::AUX));
}

#[test]
fn native_contents_convert_into_distinct_engine_bits() {
    assert_eq!(Contents::from_q1(-2), Contents::SOLID);
    assert_eq!(Contents::from_q1(-3), Contents::from_q2(32));
    assert_eq!(Contents::from_q1(-4), Contents::from_q3(16));
    assert_eq!(Contents::from_q1(-5), Contents::LAVA);
    assert_eq!(Contents::from_q1(-6), Contents::SKY);
    assert_ne!(Contents::SKY, Contents::SOLID);
    assert_eq!(Contents::from_q1(-9), Contents::from_q2(32 | 0x40000));
    assert_eq!(Contents::from_q2(0x20000000), Contents::LADDER);
    assert_eq!(Contents::from_q3(0x20000000), Contents(0x20000000));
    assert_eq!(Contents::from_q2(2), Contents::WINDOW);
    assert_eq!(Contents::from_q2(0x40000000), Contents::PLAYER);
    assert_eq!(Contents::from_q2(0x80000000), Contents::PROJECTILE);
    assert!(!Contents::PLAYER.intersects(Contents::BODY | Contents::PROJECTILE));
}

#[test]
fn q2_contents_roundtrip_all_bits_and_mixed_words_without_aliases() {
    for bit in 0..32 {
        let native = 1u32 << bit;
        assert_eq!(Contents::from_q2(native).to_q2(), native);
        for other in 0..32 {
            if bit != other {
                assert!(!Contents::from_q2(native).intersects(Contents::from_q2(1 << other)));
            }
        }
    }
    let mut seed = 0x3169u32;
    for _ in 0..65536 {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        assert_eq!(Contents::from_q2(seed).to_q2(), seed);
    }
    assert_eq!(Contents::from_q2(u32::MAX).to_q2(), u32::MAX);
    for token in -14..=-9 {
        assert_eq!(
            Contents::from_q1(token).to_q2(),
            32 | (1 << (18 + (-9 - token)))
        );
    }
    assert_eq!(Contents::SKY.to_q2(), 1);
    // Q3's NODROP is not the rerelease's projectile identity.
    assert_eq!(Contents::from_q3(0x80000000).to_q2(), 0);
}
