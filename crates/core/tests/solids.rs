use qa_core::{
    primitives::{Bounds, RuleSetId, Vec3},
    solids::{PackedSolid, SolidCodec},
};

const Q2: SolidCodec = SolidCodec::QUAKE2;
const RR: SolidCodec = SolidCodec::QUAKE2_RERELEASE;
const Q3: SolidCodec = SolidCodec::QUAKE3;

fn bounds() -> Bounds {
    Bounds {
        mins: Vec3([-16., -24., -24.]),
        maxs: Vec3([16., 24., 32.]),
    }
}

#[test]
fn packed_solid_capabilities_are_selected_independently_from_native_source() {
    assert_eq!(SolidCodec::load(RuleSetId::Quake), None);
    assert_eq!(SolidCodec::load(RuleSetId::QuakeWorld), None);
    assert_eq!(SolidCodec::load(RuleSetId::Quake2), Some(Q2));
    assert_eq!(SolidCodec::load(RuleSetId::Quake2Rerelease), Some(RR));
    assert_eq!(SolidCodec::load(RuleSetId::Quake3), Some(Q3));
}

#[test]
fn original_link_encodings_and_inverse_keep_native_extent_units() {
    for (codec, word, y) in [
        (Q2, 0x2062, 16.),
        (RR, 0x4018_1810, 24.),
        (Q3, 0x0040_1810, 16.),
    ] {
        assert_eq!(codec.pack_bounds(bounds()), word);
        assert_eq!(
            codec.decode(word),
            PackedSolid::Box(Bounds {
                mins: Vec3([-16., -y, -24.]),
                maxs: Vec3([16., y, 32.]),
            })
        );
    }
}

#[test]
fn zero_brush_and_empty_bounds_keep_their_native_meanings() {
    for codec in [Q2, RR, Q3] {
        assert_eq!(codec.decode(0), PackedSolid::None);
        assert_eq!(codec.decode(codec.bsp()), PackedSolid::Brush);
    }
    assert_eq!(Q2.bsp(), 31);
    assert_eq!(RR.bsp(), 31);
    assert_eq!(Q3.bsp(), 0x00ff_ffff);
    assert_eq!(Q2.pack_bounds(Bounds::default()), 0x1021);
    assert_eq!(RR.pack_bounds(Bounds::default()), 0);
    assert_eq!(Q3.pack_bounds(Bounds::default()), 0x0020_0101);
}

#[test]
fn original_float_division_truncation_and_asymmetric_clamps_are_preserved() {
    let fractional = Bounds {
        mins: Vec3([-15.999, -31.999, -24.9]),
        maxs: Vec3([15.999, 31.999, -16.5]),
    };
    assert_eq!(Q2.pack_bounds(fractional), 0x0461);
    assert_eq!(RR.pack_bounds(fractional), 0x0f18_1f0f);
    assert_eq!(Q3.pack_bounds(fractional), 0x000f_180f);
    let tiny = Bounds {
        mins: Vec3([-0.1, -0.2, -0.1]),
        maxs: Vec3([0.1, 0.2, -0.05]),
    };
    assert_eq!(Q2.pack_bounds(tiny), 0x0c21);
    assert_eq!(RR.pack_bounds(tiny), 0x1f00_0101);
    assert_eq!(Q3.pack_bounds(tiny), 0x001f_0101);
    let large = Bounds {
        mins: Vec3([-999.; 3]),
        maxs: Vec3([999.; 3]),
    };
    assert_eq!(Q2.pack_bounds(large), 0xffff);
    assert_eq!(RR.pack_bounds(large), 0xffff_ffff);
    assert_eq!(Q3.pack_bounds(large), 0x00ff_ffff);
}

#[test]
fn supported_cross_layout_mappings_do_not_copy_incompatible_raw_cells() {
    for (source, source_word, expected) in [
        (Q2, 0x2062, [0x2062, 0x4018_1010, 0x0040_1810]),
        (RR, 0x4018_1810, [0x2062, 0x4018_1810, 0x0040_1810]),
        (Q3, 0x0040_1810, [0x2062, 0x4018_1010, 0x0040_1810]),
    ] {
        for (target, expected) in [Q2, RR, Q3].into_iter().zip(expected) {
            assert_eq!(target.transcode(source, source_word), Some(expected));
            assert_eq!(target.transcode(source, 0), Some(0));
            assert_eq!(target.transcode(source, source.bsp()), Some(target.bsp()));
        }
    }
}

#[test]
fn native_words_stay_exact_and_classic_signed_decode_uses_low_extent_fields() {
    for codec in [Q2, RR, Q3] {
        for word in [0, 31, 0x00ff_ffff, 0x8123_4567, u32::MAX] {
            assert_eq!(codec.transcode(codec, word), Some(word));
        }
    }
    let expected = PackedSolid::Box(Bounds {
        mins: Vec3([-248.; 3]),
        maxs: Vec3([248., 248., 472.]),
    });
    assert_eq!(Q2.decode(0xffff), expected);
    assert_eq!(Q2.decode(u32::MAX), expected);
    assert!(matches!(Q3.decode(0x01ff_ffff), PackedSolid::Box(_)));
}

#[test]
fn foreign_invalid_boxes_and_target_brush_collisions_are_not_fabricated_models() {
    // RR box with upper Z=-32 and lower Z=0 cannot form valid target bounds.
    let invalid = 0x0000_0101;
    assert!(matches!(RR.decode(invalid), PackedSolid::Box(_)));
    assert_eq!(Q2.transcode(RR, invalid), None);
    assert_eq!(Q3.transcode(RR, invalid), None);
    assert_eq!(RR.transcode(RR, invalid), Some(invalid));
    // All byte extents255 is a real RR box, but a Q3 brush sentinel.
    assert!(matches!(RR.decode(u32::MAX), PackedSolid::Box(_)));
    assert_eq!(Q3.transcode(RR, u32::MAX), None);
    assert_eq!(Q3.transcode(Q3, Q3.bsp()), Some(Q3.bsp()));
}
