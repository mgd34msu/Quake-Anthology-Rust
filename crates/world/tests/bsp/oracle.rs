#[test]
fn point_descent_keeps_each_callers_tie_and_signed_zero_rule() {
    use qa_core::primitives::{Plane, Vec3};
    let node = |_| (Plane::oriented(Vec3([1.0, 0.0, 0.0]), 0.0), [-1, -2]);
    for x in [-1.0, -0.0, 0.0, 1.0] {
        let point = Vec3([x, 0.0, 0.0]);
        assert_eq!(
            super::descend(0, point, false, node),
            if x < 0.0 { -2 } else { -1 }
        );
        assert_eq!(
            super::descend(0, point, true, node),
            if x <= 0.0 { -2 } else { -1 }
        );
    }
    assert_eq!(
        super::descend(-3, Vec3::default(), true, |_| panic!("leaf")),
        -3
    );
}
