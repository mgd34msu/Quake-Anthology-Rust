use qa_core::primitives::{Capsule, Vec3};

#[test]
fn capsule_validation_preserves_finite_nonnegative_radius_and_arbitrary_segment() {
    for radius in [0.0, -0.0, 1.0, f32::MAX] {
        for offset in [
            Vec3::default(),
            Vec3([10.0, -20.0, 30.0]),
            Vec3([f32::MAX; 3]),
        ] {
            assert!(Capsule { radius, offset }.is_valid());
        }
    }
    for radius in [
        -f32::MIN_POSITIVE,
        -1.0,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NAN,
    ] {
        assert!(
            !Capsule {
                radius,
                offset: Vec3::default()
            }
            .is_valid()
        );
    }
    for axis in 0..3 {
        for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut offset = Vec3::default();
            offset.0[axis] = value;
            assert!(
                !Capsule {
                    radius: 1.0,
                    offset
                }
                .is_valid()
            );
        }
    }
}
