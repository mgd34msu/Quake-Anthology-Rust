use qa_core::primitives::{Bounds, Capsule, Vec3};

#[test]
fn supplied_bounds_define_one_vertical_segment_and_sphere() {
    let capsule = Capsule::from_bounds(Bounds {
        mins: Vec3([-15.0, -1.0, -24.0]),
        maxs: Vec3([15.0, 1.0, 32.0]),
    });
    assert_eq!(capsule.radius, 15.0);
    assert_eq!(capsule.offset, Vec3([0.0, 0.0, 13.0]));
    assert_eq!(
        capsule.swept_bounds(Vec3([10.0, 20.0, 4.0]), Vec3([30.0, 10.0, 4.0]), 1.0),
        Bounds {
            mins: Vec3([-6.0, -6.0, -25.0]),
            maxs: Vec3([46.0, 36.0, 33.0]),
        }
    );
    let sphere = Capsule::from_bounds(Bounds {
        mins: Vec3([-32.0, -1.0, -4.0]),
        maxs: Vec3([32.0, 1.0, 4.0]),
    });
    assert_eq!(sphere.radius, 4.0);
    assert_eq!(sphere.offset, Vec3::default());
}

#[test]
fn segment_projection_preserves_native_column_and_support_selection() {
    let capsule = Capsule {
        radius: 3.0,
        offset: Vec3([0.0, 0.0, 5.0]),
    };
    let rotated = capsule.to_local([
        Vec3([0.0, 0.0, 1.0]),
        Vec3([0.0, 1.0, -0.0]),
        Vec3([-1.0, 0.0, 0.0]),
    ]);
    assert_eq!(
        rotated.offset.0.map(f32::to_bits),
        [5.0f32.to_bits(), (-0.0f32).to_bits(), 0]
    );
    assert_eq!(
        rotated.closest_segment(
            Vec3([7.0, 0.0, 0.0]),
            Vec3([9.0, 0.0, 0.0]),
            Vec3::axis(0, 1.0)
        ),
        (Vec3([2.0, 0.0, 0.0]), Vec3([4.0, 0.0, 0.0]))
    );
    assert_eq!(
        rotated.closest_segment(
            Vec3([7.0, 0.0, 0.0]),
            Vec3([9.0, 0.0, 0.0]),
            Vec3::axis(0, -1.0)
        ),
        (Vec3([12.0, 0.0, 0.0]), Vec3([14.0, 0.0, 0.0]))
    );
}

#[test]
fn capsule_bounds_preserve_native_double_fabs_before_float_storage() {
    let capsule = Capsule {
        radius: 0.5,
        offset: Vec3([0.0, 0.0, 0.5]),
    };
    let bounds = capsule.swept_bounds(
        Vec3([0.0, 0.0, 16_777_216.0]),
        Vec3([0.0, 0.0, 16_777_216.0]),
        0.0,
    );
    assert_eq!(bounds.mins.0[2].to_bits(), 16_777_215.0f32.to_bits());
    assert_eq!(bounds.maxs.0[2].to_bits(), 16_777_216.0f32.to_bits());
}
