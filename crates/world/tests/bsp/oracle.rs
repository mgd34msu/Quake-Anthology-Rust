#[test]
fn point_descent_keeps_each_callers_tie_and_signed_zero_rule() {
    use qa_core::primitives::{Plane, PointLeafRules, Vec3};
    let node = |_| (Plane::oriented(Vec3([1.0, 0.0, 0.0]), 0.0), [-1, -2]);
    for x in [-1.0, -0.0, 0.0, 1.0] {
        let point = Vec3([x, 0.0, 0.0]);
        assert_eq!(
            super::descend(0, point, PointLeafRules::CLIP, node),
            if x < 0.0 { -2 } else { -1 }
        );
        assert_eq!(
            super::descend(0, point, PointLeafRules::RENDER, node),
            if x <= 0.0 { -2 } else { -1 }
        );
    }
    assert_eq!(
        super::descend(-3, Vec3::default(), PointLeafRules::RENDER, |_| panic!(
            "leaf"
        )),
        -3
    );
}

#[test]
fn raw_negative_axial_planes_keep_native_clip_arithmetic_separate_from_render_geometry() {
    use qa_core::primitives::{Axis, Plane, PointLeafRules, Vec3};
    let plane = Plane {
        normal: Vec3([-1.0, 0.0, 0.0]),
        distance: 0.0,
        axis: Some(Axis::X),
        encoding: Some([0, 1]),
    };
    for x in [-1.0, -0.0, 0.0, 1.0] {
        let point = Vec3([x, 0.0, 0.0]);
        assert_eq!(
            super::descend(0, point, PointLeafRules::CLIP, |_| (plane, [-1, -2])),
            if x < 0.0 { -2 } else { -1 },
        );
        assert_eq!(
            super::descend(0, point, PointLeafRules::RENDER, |_| (plane, [-1, -2])),
            if x >= 0.0 { -2 } else { -1 },
        );
    }
    assert_eq!(plane.type_sign(), [0, 1]);
    let point = Vec3([1.0, 0.0, 0.0]);
    assert_eq!(plane.signed_distance(point), 1.0);
    assert_eq!(plane.geometric_distance(point), -1.0);
}
