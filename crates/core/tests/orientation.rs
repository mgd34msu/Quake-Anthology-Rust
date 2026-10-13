use qa_core::primitives::{Orientation, Vec3};

fn pose(words: &[u32]) -> Orientation {
    let vector = |offset| {
        Vec3(std::array::from_fn(|axis| {
            f32::from_bits(words[offset + axis])
        }))
    };
    Orientation {
        origin: vector(0),
        axes: std::array::from_fn(|axis| vector(3 + axis * 3)),
    }
}

#[test]
fn weighted_tag_interpolation_matches_original_q3_bits() {
    // Regenerate from unchanged R_LerpTag and VectorNormalize with
    // tools/check_model_tags.py; includes extrapolation and zero-length axes.
    let mut rows = 0;
    for line in include_str!("fixtures/q3_tags.txt").lines() {
        let words = line
            .split_whitespace()
            .map(|word| u32::from_str_radix(word, 16).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(words.len(), 37);
        let result =
            pose(&words[..12]).interpolate(pose(&words[12..24]), f32::from_bits(words[24]));
        let actual = result
            .origin
            .0
            .into_iter()
            .chain(result.axes.into_iter().flat_map(|axis| axis.0))
            .map(f32::to_bits)
            .collect::<Vec<_>>();
        assert_eq!(actual, words[25..], "native tag row {rows}");
        rows += 1;
    }
    assert_eq!(rows, 64);
}

#[test]
fn absent_tag_orientation_has_native_identity_axes_and_zero_origin() {
    let identity = Orientation::default();
    assert_eq!(identity.origin, Vec3::default());
    for axis in 0..3 {
        assert_eq!(identity.axes[axis], Vec3::axis(axis, 1.0));
    }
}
