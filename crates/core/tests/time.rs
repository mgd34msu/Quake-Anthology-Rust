use qa_core::primitives::ThinkTime;

#[test]
fn native_integer_time_truncates_fractional_milliseconds_towards_zero() {
    for (seconds, milliseconds) in [
        (0.0009, 0),
        (-0.0009, 0),
        (0.9999, 999),
        (-0.9999, -999),
        (1.0009, 1000),
        (-1.0009, -1000),
    ] {
        assert_eq!(ThinkTime::Seconds(seconds).milliseconds(), milliseconds);
        assert_eq!(
            ThinkTime::Seconds(seconds).seconds().to_bits(),
            seconds.to_bits()
        );
    }
    for milliseconds in [i64::MIN, -1, 0, 1, i64::MAX] {
        assert_eq!(
            ThinkTime::Milliseconds(milliseconds).milliseconds(),
            milliseconds
        );
    }
}
