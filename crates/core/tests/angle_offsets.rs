use qa_core::{
    math::{self, AngleShortForm},
    primitives::{AngleOffset, AngleOffsets, Vec3},
};

#[test]
fn native_integer_offsets_never_roundtrip_through_float_degrees() {
    for word in [
        i32::MIN,
        i32::MIN + 1,
        -65_537,
        -1,
        0,
        65_537,
        i32::MAX - 1,
        i32::MAX,
    ] {
        let offset = AngleOffset::Units(word);
        for form in [
            AngleShortForm::MultiplyDivide,
            AngleShortForm::Factored,
            AngleShortForm::Double,
        ] {
            assert_eq!(offset.units(form), word);
        }
        assert_eq!(
            offset.degrees().to_bits(),
            math::short_to_angle(word).to_bits()
        );
    }
    let word = i32::MAX - 1;
    assert_ne!(
        math::angle_to_short(math::short_to_angle(word), AngleShortForm::Factored),
        word
    );
}

#[test]
fn float_offsets_keep_bits_and_use_the_selected_native_conversion_order() {
    for bits in [0, 0x8000_0000, 0x3f80_0001, 0x42b4_0000, 0xffc0_1234] {
        let value = f32::from_bits(bits);
        let offset = AngleOffset::Degrees(value);
        assert_eq!(offset.degrees().to_bits(), bits);
        for form in [
            AngleShortForm::MultiplyDivide,
            AngleShortForm::Factored,
            AngleShortForm::Double,
        ] {
            assert_eq!(offset.units(form), math::angle_to_short(value, form));
        }
    }
    let degrees = Vec3([f32::from_bits(0x8000_0000), 3.125, 17.25]);
    assert_eq!(
        AngleOffsets::from(degrees).degrees().0.map(f32::to_bits),
        degrees.0.map(f32::to_bits)
    );
    assert_eq!(AngleOffsets::default().0, [AngleOffset::Degrees(0.0); 3]);
}
