use qa_compat::{
    renderer::{Q3_REFDEF_BYTES, read_q3_view},
    services::CallError,
};
use qa_render::stage::DrawInputs;

#[test]
fn complete_ref_entity_decodes_to_owned_pose_and_renderer_handles() {
    use qa_compat::renderer::{EntityKind, Q3_REF_ENTITY_BYTES, read_q3_entity};
    let mut bytes = [0; Q3_REF_ENTITY_BYTES];
    for (offset, word) in [
        (0, 0i32),
        (4, 1 | 4 | 8 | 128 | 256 | 512),
        (8, 41),
        (64, 1),
        (80, -7),
        (96, 18),
        (104, 3),
        (108, 11),
        (112, 19),
    ] {
        bytes[offset..offset + 4].copy_from_slice(&word.to_le_bytes());
    }
    for (offset, value) in [
        (12, 1.5f32),
        (16, -2.25),
        (20, 3.75),
        (24, -20.),
        (28, 2.),
        (44, 3.),
        (60, 4.),
        (68, 8.),
        (72, 9.),
        (76, 10.),
        (84, 4.),
        (88, 5.),
        (92, 6.),
        (100, 0.25),
        (120, 0.125),
        (124, 0.75),
        (128, -3.5),
        (132, 2.),
        (136, -30.),
    ] {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    bytes[116..120].copy_from_slice(&[1, 2, 3, 4]);
    let entity = read_q3_entity(&bytes).unwrap();
    assert_eq!(entity.kind, EntityKind::Model);
    assert_eq!(
        (
            entity.model,
            entity.skin,
            entity.custom_skin,
            entity.custom_shader
        ),
        (41, 3, 11, 19)
    );
    assert_eq!((entity.frame, entity.old_frame), (-7, 18));
    assert_eq!(
        entity.pose.axes.map(|v| v.0),
        [[2., 0., 0.], [0., 3., 0.], [0., 0., 4.]]
    );
    assert_eq!(entity.pose.origin.0, [8., 9., 10.]);
    assert_eq!(entity.old_origin.0, [4., 5., 6.]);
    assert_eq!(entity.pose.color, [1, 2, 3, 4]);
    assert_eq!(entity.pose.lighting_origin.unwrap().0, [1.5, -2.25, 3.75]);
    assert_eq!(entity.pose.shadow_plane, Some(-20.));
    assert!(
        entity.pose.minimum_light
            && entity.pose.casts_shadow
            && entity.pose.depth_hack
            && entity.pose.non_normalized_axes
            && entity.wrap_frames
    );
    assert_eq!(
        entity.pose.visibility,
        qa_render::scene::EntityVisibility::Direct
    );
    assert_eq!(
        (entity.radius, entity.rotation, entity.pose.shader_time),
        (2., -30., -3.5)
    );
    for (code, kind) in [
        (0i32, EntityKind::Model),
        (1, EntityKind::PolyMarker),
        (2, EntityKind::Sprite),
        (3, EntityKind::Beam),
        (4, EntityKind::RailCore),
        (5, EntityKind::RailRings),
        (6, EntityKind::Lightning),
        (7, EntityKind::Portal),
    ] {
        bytes[..4].copy_from_slice(&code.to_le_bytes());
        assert_eq!(read_q3_entity(&bytes).unwrap().kind, kind);
    }
    bytes[..4].copy_from_slice(&8i32.to_le_bytes());
    assert_eq!(read_q3_entity(&bytes).unwrap_err(), CallError::Geometry);
    assert_eq!(
        read_q3_entity(&bytes[..139]).unwrap_err(),
        CallError::Memory
    );
}

#[test]
fn signed_clock_and_complete_text_rows_survive_native_refdef_decode() {
    for time in [i32::MIN, -12345, 0, 16777217, i32::MAX] {
        let mut bytes = [0; Q3_REFDEF_BYTES];
        bytes[8..12].copy_from_slice(&320i32.to_le_bytes());
        bytes[12..16].copy_from_slice(&200i32.to_le_bytes());
        bytes[72..76].copy_from_slice(&time.to_le_bytes());
        bytes[76..80].copy_from_slice(&5i32.to_le_bytes());
        bytes[80..112].fill(0xf1);
        bytes[113] = 0xff;
        let view = read_q3_view(&bytes).unwrap();
        assert_eq!(view.refdef.time_ms, i64::from(time));
        assert_eq!(
            view.refdef.time_seconds().to_bits(),
            ((time as f32) * 0.001f32).to_bits()
        );
        assert_eq!(
            DrawInputs::for_view(&view.refdef).time_seconds().to_bits(),
            view.refdef.time_seconds().to_bits()
        );
        assert_eq!(view.hidden_areas, [0xf1; 32]);
        assert_eq!(view.refdef.text[0][0], 0);
        assert_eq!(view.refdef.text[0][1], 0xff);
        assert!(view.refdef.hyperspace);
        assert!(!view.world);
        bytes[112..144].fill(1);
        assert_eq!(read_q3_view(&bytes).unwrap_err(), CallError::Text);
        assert_eq!(
            read_q3_view(&bytes[..Q3_REFDEF_BYTES - 1]).unwrap_err(),
            CallError::Memory
        );
    }
}
