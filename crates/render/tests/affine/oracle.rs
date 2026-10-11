use super::{determinant, fit3, widest_fan_basis};

#[test]
fn sliver_culling_retains_f32_fan_rounding_independently_of_winding() {
    let points = [[0.0, 0.0], [4096.5, 4096.0], [4096.0, 4095.5]];
    assert_eq!(super::cull_area(3, |index| points[index]), 0.0);
    assert_eq!(
        super::signed_area(3, |index| points[index].map(f64::from)),
        -0.25
    );
}

// Coefficient arithmetic before consolidation, with each caller's prepared points.
fn previous(points: [[f64; 2]; 3], values: [f64; 3], area: f64) -> [f64; 3] {
    let [a, b, c] = points;
    let ab = [b[0] - a[0], b[1] - a[1]];
    let ac = [c[0] - a[0], c[1] - a[1]];
    let vb = values[1] - values[0];
    let vc = values[2] - values[0];
    let x = (vb * ac[1] - vc * ab[1]) / area;
    let y = (ab[0] * vc - ac[0] * vb) / area;
    [x, y, values[0] - x * a[0] - y * a[1]]
}

#[test]
fn fits_keep_bits_with_native_half_pixel_and_depth_coordinates() {
    let mut state = 0x1234_5678u32;
    let mut sample = || {
        state = state.wrapping_mul(1664525).wrapping_add(1013904223);
        (state as i32 as f32) / 32768.0
    };
    for _ in 0..4096 {
        let points: [[f32; 2]; 3] = std::array::from_fn(|_| std::array::from_fn(|_| sample()));
        let values = [sample(), sample(), sample()].map(f64::from);
        for offset in [false, true] {
            let points = points.map(|xy| xy.map(|v| f64::from(if offset { v - 0.5 } else { v })));
            let area = determinant(points);
            if area == 0.0 {
                continue;
            }
            assert_eq!(
                fit3(points, values, area).map(f64::to_bits),
                previous(points, values, area).map(f64::to_bits)
            );
        }
    }
}

#[test]
fn fan_selection_keeps_first_equal_area_and_no_thin_geometry_epsilon() {
    let square = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
    assert_eq!(
        widest_fan_basis(square.len(), |i| square[i]),
        Some(([0, 1, 2], 1.0))
    );
    let thin = [[0.0, 0.0], [1.0, 0.0], [1.0, f64::from(f32::from_bits(1))]];
    assert!(widest_fan_basis(thin.len(), |i| thin[i]).is_some());
    assert_eq!(widest_fan_basis(2, |i| square[i]), None);
    assert_eq!(widest_fan_basis(3, |i| [i as f64, 0.0]), None);
}

#[test]
fn fan_iterator_keeps_polygon_anchor_and_order() {
    for count in [0, 1, 2] {
        assert!(super::fan(count).next().is_none());
    }
    assert_eq!(super::fan(3).collect::<Vec<_>>(), [[0, 1, 2]]);
    assert_eq!(
        super::fan(5).collect::<Vec<_>>(),
        [[0, 1, 2], [0, 2, 3], [0, 3, 4]]
    );
}
