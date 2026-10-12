use qa_core::{
    math,
    primitives::{ClientId, EntityAppearance, ModelRef, ResourceNamespace, RuleSetId, Vec3},
};
use qa_gameplay::rules::appearance::{alias_light_direction, can_interpolate, project};

#[test]
fn source_flags_do_not_inherit_another_games_render_flag_semantics() {
    let mut appearance = EntityAppearance {
        source: RuleSetId::Quake2,
        ..Default::default()
    };
    appearance.native.render_flags = 2 | 4 | 8 | 16 | 64;
    let q2 = project(&appearance);
    assert!(q2.viewer_model && q2.weapon_model && q2.fullbright && q2.depth_hack && q2.frame_lerp);
    assert_eq!(q2.unsupported, 0);
    appearance.source = RuleSetId::Quake3;
    let q3 = project(&appearance);
    assert!(
        !q3.viewer_model && !q3.weapon_model && !q3.fullbright && !q3.depth_hack && !q3.frame_lerp
    );
    assert_eq!(q3.unsupported, u64::from(appearance.native.render_flags));
    assert!(q3.drawable);
}

#[test]
fn rerelease_default_scalars_are_effective_values_without_changing_native_sentinels() {
    let mut appearance = EntityAppearance {
        source: RuleSetId::Quake2Rerelease,
        alpha: -0.0,
        scale: 0.0,
        ..Default::default()
    };
    appearance.native.render_flags = 32;
    let native_alpha = appearance.alpha.to_bits();
    let native_scale = appearance.scale.to_bits();
    let effective = project(&appearance);
    assert_eq!(effective.alpha, 0.7);
    assert_eq!(effective.scale, 1.0);
    assert_eq!(appearance.alpha.to_bits(), native_alpha);
    assert_eq!(appearance.scale.to_bits(), native_scale);
    appearance.alpha = 0.25;
    appearance.scale = 2.0;
    appearance.native.render_flags = 0;
    let explicit = project(&appearance);
    assert_eq!(explicit.alpha, 0.25);
    assert_eq!(explicit.scale, 2.0);
}

#[test]
fn legacy_translucency_defaults_keep_the_original_effect_override_order() {
    let mut appearance = EntityAppearance {
        source: RuleSetId::Quake2Rerelease,
        alpha: 0.0,
        ..Default::default()
    };
    for (effects, expected) in [
        (1 << 7, 0.3),
        ((1 << 7) | (1 << 24), 0.6),
        ((1 << 24) | (1 << 28), 0.3),
        ((1 << 28) | (1 << 31), 0.6),
    ] {
        appearance.native.effects = effects;
        let projected = project(&appearance);
        assert_eq!(projected.alpha, expected);
        assert_ne!(projected.unsupported, 0); // Scalar decoding supplies no particles.
    }
    appearance.source = RuleSetId::Quake2;
    appearance.native.effects = 0;
    appearance.native.render_flags = 32 | 8;
    assert_eq!(project(&appearance).alpha, 0.0);
    appearance.native.render_flags = 32;
    assert_eq!(project(&appearance).alpha, 0.7);
}

#[test]
fn unsupported_beams_do_not_turn_into_ordinary_models() {
    let mut appearance = EntityAppearance {
        source: RuleSetId::Quake2,
        ..Default::default()
    };
    appearance.native.render_flags = 128;
    let q2 = project(&appearance);
    assert!(!q2.drawable && q2.unsupported != 0);
    appearance.source = RuleSetId::Quake3;
    assert!(project(&appearance).drawable);
    assert_ne!(project(&appearance).unsupported, 0);
}

#[test]
fn rerelease_keeps_old_frame_selection_separate_from_discrete_origin_selection() {
    let mut appearance = EntityAppearance {
        source: RuleSetId::Quake2Rerelease,
        ..Default::default()
    };
    appearance.native.render_flags = 1 << 22;
    let old_frame = project(&appearance);
    assert!(old_frame.state_old_frame && !old_frame.frame_lerp);
    appearance.native.render_flags = 64;
    let discrete = project(&appearance);
    assert!(!discrete.state_old_frame && discrete.frame_lerp);
}

#[test]
fn software_alias_light_preserves_native_dot_then_negation_order() {
    for angles in [Vec3::default(), Vec3([23.0, 45.0, 67.0])] {
        let light = Vec3([-1.0, 0.0, 0.0]);
        let basis = math::angle_vectors(angles);
        let expected = [
            light.dot(basis.forward),
            -light.dot(basis.right),
            light.dot(basis.up),
        ];
        for source in [RuleSetId::Quake2, RuleSetId::Quake2Rerelease] {
            let actual = alias_light_direction(source, angles).unwrap();
            assert_eq!(actual.0.map(f32::to_bits), expected.map(f32::to_bits));
        }
    }
    assert_eq!(
        alias_light_direction(RuleSetId::Quake3, Vec3::default()),
        None
    );
}

#[test]
fn q2_native_discontinuities_reset_interpolation_without_cross_namespace_aliasing() {
    let previous = EntityAppearance {
        source: RuleSetId::Quake2,
        ..Default::default()
    };
    let mut current = previous;
    assert!(can_interpolate(
        &previous,
        &current,
        Vec3::default(),
        Vec3([512.875, 0.0, 0.0])
    ));
    assert!(!can_interpolate(
        &previous,
        &current,
        Vec3::default(),
        Vec3([-513.0, 0.0, 0.0])
    ));
    current.models[0] = ModelRef::native(ResourceNamespace::Client(ClientId(1)), 2);
    assert!(!can_interpolate(
        &previous,
        &current,
        Vec3::default(),
        Vec3::default()
    ));
    current = previous;
    for event in [6, 7] {
        current.native.event = event;
        assert!(!can_interpolate(
            &previous,
            &current,
            Vec3::default(),
            Vec3::default()
        ));
    }
    current.native.event = 0;
    current.source = RuleSetId::Quake3;
    assert!(!can_interpolate(
        &previous,
        &current,
        Vec3::default(),
        Vec3::default()
    ));
    let previous = current;
    current.native.event = 6;
    assert!(can_interpolate(
        &previous,
        &current,
        Vec3::default(),
        Vec3::default()
    ));
}
