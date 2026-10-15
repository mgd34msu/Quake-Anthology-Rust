use qa_core::math::{fits_native_integer, native_integer};

#[test]
fn native_signed_conversion_preserves_truncation_and_overflow_sentinel() {
    for (value, expected) in [
        (0.0, 0),
        (-0.0, 0),
        (10.9, 10),
        (-10.9, -10),
        (-2147483648.0, i32::MIN),
        (2147483647.999, i32::MAX),
    ] {
        assert!(fits_native_integer(value));
        assert_eq!(native_integer(value), expected);
    }
    for value in [
        2147483648.0,
        -2147483648.5,
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
    ] {
        assert!(!fits_native_integer(value));
        assert_eq!(native_integer(value), i32::MIN);
    }
    assert_eq!(native_integer(f32::from_bits(0x4eff_ffff)), 2147483520);
    assert_eq!(native_integer(f32::from_bits(0x4f00_0000)), i32::MIN);
}
