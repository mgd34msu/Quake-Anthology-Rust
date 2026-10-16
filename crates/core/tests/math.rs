use qa_core::{
    math::*,
    primitives::{Axis, Bounds, Plane, Vec3},
};

#[test]
fn plane_clip_component_keeps_strict_stop_endpoints_and_native_zero_sign() {
    let epsilon = 0.1_f32;
    let inside = f32::from_bits(epsilon.to_bits() - 1);
    for value in [epsilon, -epsilon] {
        assert_eq!(
            clip_component(value, 0.0, 1.0, Some(epsilon)).to_bits(),
            value.to_bits()
        );
    }
    for value in [inside, -inside, -0.0, f32::from_bits(1)] {
        assert_eq!(
            clip_component(value, 0.0, 1.0, Some(epsilon)).to_bits(),
            0.0_f32.to_bits()
        );
    }
    assert_eq!(
        clip_component(-0.0, 0.0, 1.0, None).to_bits(),
        (-0.0_f32).to_bits()
    );
    assert_eq!(
        clip_component(inside, 0.0, 1.0, None).to_bits(),
        inside.to_bits()
    );
    for value in [f32::INFINITY, f32::NEG_INFINITY] {
        assert_eq!(
            clip_component(value, 0.0, 1.0, Some(epsilon)).to_bits(),
            value.to_bits()
        );
    }
    assert!(clip_component(f32::NAN, 0.0, 1.0, Some(epsilon)).is_nan());
    assert!(clip_component(1.0, f32::INFINITY, 0.0, Some(epsilon)).is_nan());
}

#[test]
fn plane_clip_component_rounds_product_before_subtraction_without_fusion() {
    let normal = f32::from_bits(0x3f80_0001); // 1 + 2^-23
    let backoff = f32::from_bits(0x3f7f_fffe); // 1 - 2^-23
    // The exact product is 1 - 2^-46 and rounds to one before subtraction.
    assert_eq!(clip_component(1.0, normal, backoff, None).to_bits(), 0);
    assert_eq!((-normal).mul_add(backoff, 1.0).to_bits(), 0x2880_0000);
    assert_eq!(clip_component(5.0, 0.5, -4.0, None), 7.0);
}

#[test]
fn local_plane_retains_the_world_half_space_for_rotated_scaled_coordinates() {
    let plane = Plane::oriented(Vec3([1., -2., 3.]), 19.);
    let origin = Vec3([4., 8., -2.]);
    let axes = [Vec3([0., 2., 0.]), Vec3([-3., 0., 0.]), Vec3([0., 0., 4.])];
    let local = plane.to_local(origin, axes);
    for point in [Vec3::default(), Vec3([2., 3., -5.]), Vec3([-7., 1., 9.])] {
        assert_eq!(
            local.signed_distance(point),
            plane.signed_distance(transform_point(origin, axes, point))
        );
    }
}

#[test]
fn plane_from_points_keeps_clockwise_normals_and_rejects_degenerate_triangles() {
    let points = [
        Vec3([1., -1., -1.]),
        Vec3([1., 1., -1.]),
        Vec3([1., 0., 1.]),
    ];
    let plane = Plane::from_points(points[0], points[1], points[2]).unwrap();
    assert_eq!(plane.normal, Vec3([-1., 0., 0.]));
    assert_eq!(plane.distance, -1.);
    for point in points {
        assert_eq!(plane.signed_distance(point), 0.);
    }
    assert_eq!(
        Plane::from_points(points[0], points[2], points[1])
            .unwrap()
            .normal,
        Vec3([1., 0., 0.])
    );
    assert!(Plane::from_points(points[0], points[0], points[1]).is_none());
}

#[test]
fn plane_projection_and_axis_rotation_preserve_the_native_basis() {
    let direction = Vec3([0.0, 0.0, 1.0]);
    assert_eq!(
        project_on_plane(Vec3([3.0, -4.0, 5.0]), direction),
        Vec3([3.0, -4.0, 0.0])
    );
    assert_eq!(perpendicular_vector(direction), Vec3([1.0, 0.0, 0.0]));
    let input = Vec3([3.0, 4.0, 5.0]);
    for (degrees, expected) in [
        (0.0, input),
        (90.0, Vec3([-4.0, 3.0, 5.0])),
        (180.0, Vec3([-3.0, -4.0, 5.0])),
        (-90.0, Vec3([4.0, -3.0, 5.0])),
    ] {
        let rotated = rotate_about_axis(direction, input, degrees);
        for (actual, expected) in rotated.0.into_iter().zip(expected.0) {
            assert!((actual - expected).abs() < 0.000002);
        }
    }
    for direction in [
        Vec3([1.0, 0.0, 0.0]),
        normalized(Vec3([1.0, 2.0, 3.0])),
        normalized(Vec3([-4.0, 2.0, 7.0])),
    ] {
        let perpendicular = perpendicular_vector(direction);
        assert!(perpendicular.dot(direction).abs() < 0.000001);
        assert!((length(perpendicular) - 1.0).abs() < 0.000001);
    }
}

#[test]
fn native_sine_table_and_reflection_keep_original_float_operations() {
    for (index, &value) in sine_table().iter().enumerate() {
        let degrees = index as f32 * 360.0 / 1023.0;
        let radians = degrees * std::f32::consts::PI / 180.0;
        assert_eq!(value.to_bits(), (f64::from(radians).sin() as f32).to_bits());
    }
    let mut seed = 0x3367_u32;
    let mut component = || {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        seed as i32 as f32 / 65536.0
    };
    for _ in 0..4096 {
        let normal = Vec3(std::array::from_fn(|_| component()));
        let direction = Vec3(std::array::from_fn(|_| component()));
        let old = normal * (2.0 * normal.dot(direction)) - direction;
        assert_eq!(
            reflect(normal, direction).0.map(f32::to_bits),
            old.0.map(f32::to_bits)
        );
    }
}

#[test]
fn inverse_projection_preserves_subtraction_dot_order_and_axis_signs() {
    let mut seed = 0x3366_u32;
    let mut value = || {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        (seed as i32) as f32 / 65536.0
    };
    for _ in 0..65536 {
        let origin = Vec3(std::array::from_fn(|_| value()));
        let axes = std::array::from_fn(|_| Vec3(std::array::from_fn(|_| value())));
        let point = Vec3(std::array::from_fn(|_| value()));
        let delta = point - origin;
        let original = axes
            .map(|axis| delta.0[0] * axis.0[0] + delta.0[1] * axis.0[1] + delta.0[2] * axis.0[2]);
        assert_eq!(
            inverse_transform_point(origin, axes, point)
                .0
                .map(f32::to_bits),
            original.map(f32::to_bits)
        );
    }
    let point = Vec3([1.0, 1.0, 0.0]);
    let axis = Vec3([1.0, -1.0, 0.0]);
    let projected = inverse_transform_point(Vec3::default(), [axis; 3], point);
    let negated_axis = inverse_transform_point(Vec3::default(), [-axis; 3], point);
    assert_eq!(projected.0[0].to_bits(), 0.0_f32.to_bits());
    assert_eq!((-projected.0[0]).to_bits(), (-0.0_f32).to_bits());
    assert_eq!(negated_axis.0[0].to_bits(), 0.0_f32.to_bits());
}

#[test]
fn eighth_unit_halves_keep_native_narrowing_and_sign_extension() {
    let mut seed = 0x3366_u32;
    for _ in 0..65536 {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        let value = f32::from_bits(seed);
        let original = (value * 8.0) as i32 as i16;
        assert_eq!(eighth_to_short(value), original);
        assert_eq!(
            narrow_eighth(value).to_bits(),
            (f32::from(original) * 0.125).to_bits()
        );
        assert_eq!(
            eighth_to_short(value) as i32 as u32,
            (narrow_eighth(value) * 8.0) as i32 as u32
        );
    }
    for value in i16::MIN..=i16::MAX {
        assert_eq!(
            short_to_eighth(value).to_bits(),
            (f32::from(value) * 0.125).to_bits()
        );
    }
}

#[test]
fn plane_encoding_keeps_native_fields_independent_of_transformed_normals() {
    assert_eq!(size_of::<Plane>(), 20);
    for axis in 0..3 {
        let mut normal = [0.0; 3];
        normal[axis] = 1.0;
        let positive = Plane::oriented(Vec3(normal), 7.0);
        let axis_id = Axis::from_index(axis).unwrap();
        assert_eq!(axis_id.index(), axis);
        assert_eq!(positive.axis, Some(axis_id));
        let mut point = [f32::NAN; 3];
        point[axis] = -0.0;
        assert_eq!(
            positive.signed_distance(Vec3(point)).to_bits(),
            (-7.0f32).to_bits()
        );
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
    assert_eq!(Axis::from_index(3), None);
    assert_eq!(Axis::from_index(usize::MAX), None);
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
fn native_angle_lerp_keeps_shortest_path_strict_ties_and_operation_order() {
    for (from, to, fraction, expected) in [
        (350.0, 10.0, 0.5, 360.0),
        (10.0, 350.0, 0.5, 0.0),
        (0.0, 180.0, 0.5, 90.0),
        (0.0, -180.0, 0.5, -90.0),
        (40.0, 50.0, 0.0, 40.0),
        (40.0, 50.0, 1.0, 50.0),
    ] {
        assert_eq!(lerp_angle(from, to, fraction), expected);
    }
    let mut seed = 0x0012_u32;
    for _ in 0..65536 {
        let mut next = || {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            seed as i32 as f32 / 65536.0
        };
        let from = next();
        let mut to = next();
        let fraction = next() / 32768.0;
        let actual = lerp_angle(from, to, fraction);
        if to - from > 180.0 {
            to -= 360.0;
        }
        if to - from < -180.0 {
            to += 360.0;
        }
        assert_eq!(actual.to_bits(), (from + fraction * (to - from)).to_bits());
    }
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

#[test]
fn bounds_corners_cover_octants_and_zero_normal_components() {
    let bounds = Bounds {
        mins: Vec3([-8.0, -0.0, -32.0]),
        maxs: Vec3([16.0, 0.0, 64.0]),
    };
    for signs in 0..8 {
        let normal = Vec3(std::array::from_fn(|axis| {
            if signs & (1 << axis) == 0 { 1.0 } else { -1.0 }
        }));
        let far = bounds.corner(normal, true);
        let near = bounds.corner(normal, false);
        for axis in 0..3 {
            let (far_value, near_value) = if signs & (1 << axis) == 0 {
                (bounds.maxs.0[axis], bounds.mins.0[axis])
            } else {
                (bounds.mins.0[axis], bounds.maxs.0[axis])
            };
            assert_eq!(far.0[axis].to_bits(), far_value.to_bits());
            assert_eq!(near.0[axis].to_bits(), near_value.to_bits());
        }
    }
    for normal in [Vec3([0.0; 3]), Vec3([-0.0; 3])] {
        assert_eq!(
            bounds.corner(normal, true).0.map(f32::to_bits),
            bounds.maxs.0.map(f32::to_bits)
        );
        assert_eq!(
            bounds.corner(normal, false).0.map(f32::to_bits),
            bounds.mins.0.map(f32::to_bits)
        );
    }
}

#[test]
fn leaf_encoding_plane_validity_and_midpoint_keep_native_values() {
    use qa_core::primitives::{child_from_leaf, leaf_from_child};
    for leaf in [0, 1, 63, 65535, i32::MAX as u32] {
        let child = child_from_leaf(leaf);
        assert_eq!(child, (-1i64 - i64::from(leaf)) as i32);
        assert_eq!(leaf_from_child(child), leaf);
    }
    let tiny = Vec3([f32::from_bits(1), 0.0, 0.0]);
    assert_eq!(tiny.dot(tiny), 0.0);
    assert!(Plane::oriented(tiny, 0.0).is_valid());
    for (normal, distance) in [
        (Vec3::default(), 0.0),
        (Vec3([-0.0; 3]), 0.0),
        (Vec3([f32::NAN, 0.0, 1.0]), 0.0),
        (Vec3([1.0, 0.0, 0.0]), f32::INFINITY),
    ] {
        assert!(!Plane::oriented(normal, distance).is_valid());
    }
    for a in [Vec3([0.0, -0.0, 1.0]), Vec3([-97.375, 1234.625, f32::MAX])] {
        for b in [Vec3([-0.0, 0.0, -1.0]), Vec3([73.125, -177.75, f32::MAX])] {
            let old = Vec3(std::array::from_fn(|i| (a.0[i] + b.0[i]) * 0.5));
            assert_eq!(midpoint(a, b).0.map(f32::to_bits), old.0.map(f32::to_bits));
        }
    }
}

#[test]
fn scalar_vector_ma_reads_aliases_after_each_native_store() {
    let mut words = [2.0_f32, 3.0, 4.0, 5.0];
    let mut order = Vec::new();
    vector_ma(
        &mut words,
        2.0,
        |words, axis| Ok::<_, ()>((words[axis + 1], words[axis])),
        |words, axis, value| {
            words[axis + 1] = value;
            order.push(axis);
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(words, [2.0, 7.0, 18.0, 41.0]);
    assert_eq!(order, [0, 1, 2]);
}

#[test]
fn scalar_vector_ma_keeps_separate_float_operations_and_failure_prefix() {
    let base = [-0.0_f32, 1234.5679, -9.875];
    let rate = [-0.0_f32, -23.125, 0.125];
    let mut result = base;
    vector_ma(
        &mut result,
        0.333,
        |result, axis| Ok::<_, ()>((result[axis], rate[axis])),
        |result, axis, value| {
            result[axis] = value;
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(
        result.map(f32::to_bits),
        std::array::from_fn::<_, 3, _>(|axis| (base[axis] + 0.333f32 * rate[axis]).to_bits())
    );
    let mut writes = 0;
    let mut result = base;
    assert_eq!(
        vector_ma(
            &mut result,
            0.333,
            |result, axis| Ok((result[axis], rate[axis])),
            |result, axis, value| {
                writes += 1;
                if axis == 1 {
                    return Err("boundary");
                }
                result[axis] = value;
                Ok(())
            },
        ),
        Err("boundary")
    );
    assert_eq!(writes, 2);
    assert_eq!(
        result[0].to_bits(),
        (base[0] + 0.333f32 * rate[0]).to_bits()
    );
    assert_eq!(result[1..], base[1..]);
}

#[test]
fn model_radius_uses_farthest_axis_extents_about_origin() {
    let bounds = Bounds {
        mins: Vec3([-3.0, -2.0, -1.0]),
        maxs: Vec3([1.0, 4.0, 12.0]),
    };
    assert_eq!(bounds.radius(), 13.0);
    assert_eq!(Bounds::default().radius().to_bits(), 0.0_f32.to_bits());
}

#[test]
fn quaternion_slerp_endpoints_preserve_raw_nonunit_components_and_zero_signs() {
    let previous = [-0.0, 2.0, -3.0, 4.0];
    let current = [5.0, -6.0, 7.0, -0.0];
    for back in [-1.0, -0.0, 0.0] {
        assert_eq!(
            slerp_quaternion(previous, current, back).map(f32::to_bits),
            current.map(f32::to_bits)
        );
    }
    for back in [1.0, 2.0] {
        assert_eq!(
            slerp_quaternion(previous, current, back).map(f32::to_bits),
            previous.map(f32::to_bits)
        );
    }
}

#[test]
fn quaternion_slerp_chooses_the_short_arc_without_normalizing_antipodal_inputs() {
    for back in [0.25, 0.5, 0.75] {
        assert_eq!(
            slerp_quaternion([0.0, 0.0, 0.0, 2.0], [0.0, 0.0, 0.0, -2.0], back),
            [0.0, 0.0, 0.0, 2.0]
        );
    }

    let half = std::f32::consts::FRAC_1_SQRT_2;
    let midpoint = slerp_quaternion([0.0, 0.0, 0.0, 1.0], [0.0, 0.0, -half, -half], 0.5);
    // The current endpoint represents +90° Z despite its negative quaternion
    // sign. The shortest midpoint is +45°, with quaternion half-angle 22.5°.
    for (actual, expected) in midpoint
        .into_iter()
        .zip([0.0, 0.0, 0.382_683_43, 0.923_879_5])
    {
        assert!((actual - expected).abs() < 1e-6, "{actual} != {expected}");
    }
    let rotated = quaternion_rotation_rows(midpoint).map(|row| row.dot(Vec3([1.0, 0.0, 0.0])));
    for (actual, expected) in rotated.into_iter().zip([half, half, 0.0]) {
        assert!((actual - expected).abs() < 1e-6, "{actual} != {expected}");
    }
}

#[test]
fn close_quaternions_use_the_unnormalized_linear_branch() {
    // Dot=4095/4096 exceeds the native .9995 threshold. These binary fractions
    // make a 1/4-old, 3/4-new blend exact and expose any extra normalization.
    assert_eq!(
        slerp_quaternion(
            [0.0, 0.0, 0.0, 1.0],
            [1.0 / 64.0, 0.0, 0.0, 4095.0 / 4096.0],
            0.25,
        ),
        [3.0 / 256.0, 0.0, 0.0, 16_381.0 / 16_384.0]
    );
}

#[test]
fn quaternion_rows_project_a_quarter_turn_and_preserve_native_nonunit_axes() {
    let half = std::f32::consts::FRAC_1_SQRT_2;
    let rows = quaternion_rotation_rows([0.0, 0.0, half, half]);
    for (actual, expected) in rows.into_iter().zip([
        Vec3([0.0, -1.0, 0.0]),
        Vec3([1.0, 0.0, 0.0]),
        Vec3([0.0, 0.0, 1.0]),
    ]) {
        for (actual, expected) in actual.0.into_iter().zip(expected.0) {
            assert!((actual - expected).abs() < 2e-7, "{actual} != {expected}");
        }
    }
    for (actual, expected) in rows
        .map(|row| row.dot(Vec3([2.0, 3.0, 5.0])))
        .into_iter()
        .zip([-3.0, 2.0, 5.0])
    {
        assert!((actual - expected).abs() < 1e-6, "{actual} != {expected}");
    }

    let nonunit = [1.0, 2.0, 3.0, 4.0];
    let rows = quaternion_rotation_rows(nonunit);
    assert_eq!(
        rows,
        [
            Vec3([33.0, -20.0, 22.0]),
            Vec3([28.0, 39.0, 4.0]),
            Vec3([-10.0, 20.0, 49.0]),
        ]
    );
    let point = Vec3([2.0, -1.0, 0.5]);
    assert_eq!(rows.map(|row| row.dot(point)), [97.0, 19.0, -15.5]);
    assert_eq!(rotate_quaternion(nonunit, point), Vec3([39.0, 48.0, -30.0]));

    let zero = [-0.0, 0.0, -0.0, 0.0];
    assert_eq!(
        normalize_quaternion(zero).map(f32::to_bits),
        zero.map(f32::to_bits)
    );
    assert_eq!(
        quaternion_rotation_rows(zero),
        [
            Vec3([-1.0, 0.0, 0.0]),
            Vec3([0.0, -1.0, 0.0]),
            Vec3([0.0, 0.0, -1.0]),
        ]
    );
}
