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

#[test]
fn native_zero_normalization_distinguishes_in_place_and_output_results() {
    let input = Vec3([-0.0, 0.0, -0.0]);
    let mut in_place = input;
    assert_eq!(normalize(&mut in_place), 0.0);
    assert_eq!(in_place.0.map(f32::to_bits), input.0.map(f32::to_bits));
    assert_eq!(normalized_or_zero(input).0.map(f32::to_bits), [0; 3]);
    let underflow = Vec3([f32::from_bits(1), -f32::from_bits(1), 0.0]);
    assert_eq!(normalized_or_zero(underflow).0.map(f32::to_bits), [0; 3]);
    assert_eq!(
        normalized_or_zero(Vec3([3.0, 4.0, 0.0])),
        normalized(Vec3([3.0, 4.0, 0.0]))
    );
    assert_eq!(
        normalized_fast(input).0.map(f32::to_bits),
        input.0.map(f32::to_bits)
    );
}
