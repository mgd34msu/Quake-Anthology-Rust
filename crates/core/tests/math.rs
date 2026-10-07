use qa_core::{math::*, primitives::Vec3};

#[test]
fn vector_operations_normalization_and_angle_wrapping() {
    let x = Vec3([1.0, 0.0, 0.0]);
    let y = Vec3([0.0, 1.0, 0.0]);
    assert_eq!(cross(x, y), Vec3([0.0, 0.0, 1.0]));
    assert_eq!((x + y) * 2.0 / 2.0 - x, y);
    let mut v = Vec3([3.0, 4.0, 0.0]);
    assert_eq!(normalize(&mut v), 5.0);
    assert_eq!(v, Vec3([0.6, 0.8, 0.0]));
    assert_eq!(normalize(&mut Vec3::default()), 0.0);
    assert_eq!(anglemod(-90.0), 270.0);
    assert_eq!(anglemod(360.0), 0.0);
    let axes = angle_vectors(Vec3::default());
    assert_eq!(axes.forward, x);
    assert_eq!(axes.right, -y);
    assert_eq!(axes.up, Vec3([0.0, 0.0, 1.0]));
}
