use qa_core::random::Random;

#[test]
fn mt19937_reference_sequence_and_twist_boundary() {
    let mut random = Random::new(5489);
    for expected in [
        3499211612, 581869302, 3890346734, 3586334585, 545404204, 4161255391, 3922919429,
        949333985, 2715962298, 1323567403,
    ] {
        assert_eq!(random.next_u32(), expected);
    }
    // Independent state retains its seed through multiple twists.
    let mut a = Random::new(1);
    let mut b = Random::new(1);
    let mut c = Random::new(2);
    for _ in 0..2000 {
        let value = a.next_u32();
        assert_eq!(value, b.next_u32());
        assert_ne!(value, c.next_u32());
    }
}
