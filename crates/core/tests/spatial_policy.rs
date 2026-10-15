use qa_core::primitives::{AttenuationPrecision, DistanceAttenuation, SpatialPolicy};

#[test]
fn channel_coefficients_keep_native_double_and_float_product_rounding() {
    let classic = DistanceAttenuation::Product {
        ordinary: 0.0005,
        match_value: 3.0,
        matched: 0.001,
        precision: AttenuationPrecision::F64,
    };
    let rerelease = DistanceAttenuation::Product {
        ordinary: 0.0005,
        match_value: 3.0,
        matched: 0.001,
        precision: AttenuationPrecision::F32,
    };
    let mut different_coefficients = 0;
    for byte in 0..=u8::MAX {
        let attenuation = f32::from(byte) / 64.0;
        // Original S_IssuePlaysound stores these products in ch->dist_mult.
        let classic_coefficient = if byte == 192 {
            (f64::from(attenuation) * 0.001) as f32
        } else {
            (f64::from(attenuation) * 0.0005) as f32
        };
        let rerelease_coefficient = if byte == 192 {
            attenuation * 0.001f32
        } else {
            attenuation * 0.0005f32
        };
        different_coefficients +=
            usize::from(classic_coefficient.to_bits() != rerelease_coefficient.to_bits());
        for distance in [1.0f32, 0.125, 319.875, 1234.567] {
            for (policy, coefficient) in [
                (classic, classic_coefficient),
                (rerelease, rerelease_coefficient),
            ] {
                let (actual, no_pan) = policy.apply(distance, attenuation);
                assert_eq!(actual.to_bits(), (distance * coefficient).to_bits());
                assert_eq!(no_pan, byte == 0);
            }
        }
    }
    assert!(different_coefficients > 0);
    assert_eq!(classic.apply(320.0, 1.0).0.to_bits(), 0x3e23_d70b);
    assert_eq!(
        classic.apply(320.0, 3.0).0.to_bits(),
        (320.0f32 * 0.003).to_bits()
    );
}

#[test]
fn linear_policies_preserve_distance_first_order_and_zero_scale_panning() {
    for distance in [0.0f32, -0.0, 0.125, 319.875, 1234.567] {
        for attenuation in [0.0f32, -0.0, 0.015625, 1.0, 3.0, 255.0 / 64.0] {
            for scale in [0.0f32, 0.001, 0.003, 0.0008, 1.0 / 1170.0] {
                let (actual, no_pan) =
                    DistanceAttenuation::Linear(scale).apply(distance, attenuation);
                assert_eq!(actual.to_bits(), (distance * attenuation * scale).to_bits());
                assert_eq!(no_pan, scale == 0.0);
            }
        }
    }
    assert_eq!(
        SpatialPolicy::default().attenuation,
        DistanceAttenuation::Linear(0.0)
    );
}
