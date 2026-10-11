use qa_core::{
    math::*,
    primitives::{Bounds, Plane, Vec3},
};

#[test]
fn plane_encoding_keeps_native_fields_independent_of_transformed_normals() {
    assert_eq!(size_of::<Plane>(), 20);
    for axis in 0..3 {
        let mut normal = [0.0; 3];
        normal[axis] = 1.0;
        let positive = Plane::oriented(Vec3(normal), 7.0);
        assert_eq!(positive.type_sign(), [axis as u8, 0]);
        normal[axis] = -1.0;
        let negative = Plane::oriented(Vec3(normal), 7.0);
        assert_eq!(negative.type_sign(), [3, 1 << axis]);
        let mut encoded = Plane {
            encoding: Some([3 + axis as u8, 0]),
            ..negative
        };
        encoded.normal = Vec3([0.2, -0.6, -0.8]);
        assert_eq!(encoded.type_sign(), [3 + axis as u8, 0]);
        encoded.encoding = None;
        assert_eq!(encoded.type_sign(), [3, 6]);
    }
}

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

#[test]
fn short_angles_preserve_native_operation_orders_and_signed_widths() {
    let mut seed = 0x3176_u32;
    for _ in 0..65536 {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        let angle = f32::from_bits(seed);
        assert_eq!(
            angle_to_short(angle, AngleShortForm::MultiplyDivide),
            (angle * 65536.0 / 360.0) as i32
        );
        assert_eq!(
            angle_to_short(angle, AngleShortForm::Factored),
            (angle * (65536.0 / 360.0)) as i32
        );
        let short = (f64::from(angle) * (65536.0 / 360.0)) as i32;
        assert_eq!(angle_to_short(angle, AngleShortForm::Double), short);
        assert_eq!(
            anglemod(angle).to_bits(),
            (((360.0 / 65536.0) * f64::from(short & 65535)) as f32).to_bits()
        );
    }
    for short in i16::MIN..=i16::MAX {
        assert_eq!(
            short_to_angle(i32::from(short)).to_bits(),
            (f32::from(short) * (360.0 / 65536.0)).to_bits()
        );
    }
}

#[test]
fn byte_angles_keep_native_float_and_integral_forms() {
    let mut seed = 0x3169_u32;
    for _ in 0..65536 {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        let angle = f32::from_bits(seed);
        assert_eq!(
            angle_to_byte(angle, AngleByteForm::Float),
            (angle * 256.0 / 360.0) as i32
        );
        assert_eq!(
            angle_to_byte(angle, AngleByteForm::Integral),
            (angle as i32).wrapping_mul(256) / 360
        );
    }
    for byte in i8::MIN..=i8::MAX {
        assert_eq!(
            byte_to_angle(i32::from(byte)).to_bits(),
            (f32::from(byte) * (360.0 / 256.0)).to_bits()
        );
    }
}

#[test]
fn direction_encoding_preserves_native_indices_and_zero() {
    for (index, normal) in DIRECTIONS.iter().enumerate() {
        assert_eq!(direction_to_byte(*normal), index as u8);
    }
    assert_eq!(direction_to_byte(Vec3::default()), 0);
    assert_eq!(direction_to_byte(Vec3([-0.0; 3])), 0);
    assert_eq!(direction_to_byte(Vec3([f32::NAN; 3])), 0);
    assert_eq!(direction_to_byte(Vec3([0.0, 0.0, 1.0])), 5);
}

#[test]
fn transforms_and_clip_interpolation_keep_each_component_order() {
    let mut seed = 0x3176_u32;
    let mut next = || {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        (seed as i32 as f32) / 65536.0
    };
    for case in 0..16384 {
        let origin = Vec3(std::array::from_fn(|_| next()));
        let axes = std::array::from_fn(|_| Vec3(std::array::from_fn(|_| next())));
        let local = if case % 8 == 0 {
            Vec3([-0.0, 0.0, -0.0])
        } else {
            Vec3(std::array::from_fn(|_| next()))
        };
        let expected = std::array::from_fn(|i| {
            origin.0[i]
                + axes[0].0[i] * local.0[0]
                + axes[1].0[i] * local.0[1]
                + axes[2].0[i] * local.0[2]
        });
        assert_eq!(
            transform_point(origin, axes, local).0.map(f32::to_bits),
            expected.map(f32::to_bits)
        );
        let fraction = next();
        let expected = std::array::from_fn(|i| origin.0[i] + fraction * (local.0[i] - origin.0[i]));
        assert_eq!(
            origin.lerp(local, fraction).0.map(f32::to_bits),
            expected.map(f32::to_bits)
        );
    }
}

#[test]
fn bounds_union_retains_point_order_and_signed_zero() {
    let points = [
        Vec3([-0.0, 0.0, -0.0]),
        Vec3([0.0, -0.0, 0.0]),
        Vec3([-32.0, 1.0, 8.0]),
        Vec3([32.0, -1.0, -8.0]),
    ];
    let mut accumulated = Bounds::empty();
    let mut expected = Bounds::empty();
    for point in points {
        accumulated.add_point(point);
        for i in 0..3 {
            expected.mins.0[i] = expected.mins.0[i].min(point.0[i]);
            expected.maxs.0[i] = expected.maxs.0[i].max(point.0[i]);
        }
        assert_eq!(
            accumulated.mins.0.map(f32::to_bits),
            expected.mins.0.map(f32::to_bits)
        );
        assert_eq!(
            accumulated.maxs.0.map(f32::to_bits),
            expected.maxs.0.map(f32::to_bits)
        );
    }
    let mut union = Bounds::empty();
    union.add_bounds(accumulated);
    assert_eq!(union, accumulated);
}
