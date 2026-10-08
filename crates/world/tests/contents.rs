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
    assert_eq!(Contents::from_q2(0x40000000), Contents::BODY);
}
