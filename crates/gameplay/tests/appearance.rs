use qa_core::{
    math,
    primitives::{
        ClientId, EntityAppearance, ModelRef, ModelUse, ResourceNamespace, RuleSetId, Vec3,
    },
};
use qa_gameplay::rules::appearance::{
    AliasRoll, ModelFeatures, ModelInput, model_input, model_presentation, policy,
};

#[test]
fn model_cell_meanings_require_the_admitted_source_and_actual_use() {
    use ModelInput::{LinkedTranslucent, Player, Resource};
    use ModelUse::{Entity, ViewWeapon};
    use RuleSetId::{Quake, Quake2, Quake2Rerelease, Quake3, QuakeWorld};
    for (source, usage, ordinal, expected) in [
        (Quake2, Some(Entity(1)), 0x80, LinkedTranslucent(0)),
        (Quake2, Some(Entity(1)), 0x87, LinkedTranslucent(7)),
        (Quake2, Some(Entity(1)), 0xfe, LinkedTranslucent(126)),
        (Quake2, Some(Entity(0)), 0x87, Resource(135)),
        (Quake2, Some(Entity(2)), 0x87, Resource(135)),
        (Quake2, Some(Entity(3)), 0xff, Resource(255)),
        (Quake2, Some(Entity(1)), 0x100, Resource(256)),
        (Quake2, Some(Entity(1)), 0x187, Resource(391)),
        (Quake2, Some(Entity(0)), 0xff, Player),
        (Quake2, Some(Entity(1)), 0xff, Player),
        (Quake2, Some(ViewWeapon), 0xff, Resource(255)),
        (Quake2, None, 0xff, Resource(255)),
        (Quake2, None, 0x87, Resource(135)),
        (Quake2Rerelease, Some(Entity(0)), 0xff, Player),
        (Quake2Rerelease, Some(Entity(1)), 0xff, Player),
        (Quake2Rerelease, Some(Entity(1)), 0x87, Resource(135)),
        (Quake2Rerelease, Some(ViewWeapon), 0xff, Resource(255)),
        (Quake2Rerelease, None, 0xff, Resource(255)),
        (Quake, Some(Entity(1)), 0xff, Resource(255)),
        (QuakeWorld, Some(Entity(1)), 0x87, Resource(135)),
        (Quake3, Some(Entity(0)), 0xff, Resource(255)),
    ] {
        assert_eq!(
            model_input(source, ordinal, usage),
            expected,
            "{source:?} {usage:?} {ordinal}"
        );
    }
}

#[test]
fn classic_linked_cells_reset_draw_properties_without_changing_common_pose_policy() {
    let namespace = ResourceNamespace::Client(ClientId(0));
    let mut appearance = EntityAppearance {
        source: RuleSetId::Quake2,
        models: [200, 0x87, 201, 202].map(|ordinal| ModelRef::native(namespace, ordinal)),
        skin: 0x302,
        scale: 2.0,
        frame: 91,
        ..Default::default()
    };
    appearance.native.render_flags = 2 | 4 | 8 | 16 | 32 | 64 | 0x8000;
    appearance.native.effects = 1 << 7; // EF_BFG gives the primary its own alpha.
    let rules = policy(appearance.source);
    let base = rules.project(&appearance);
    let references = appearance.models;
    let primary = rules
        .model_cell(&appearance, 0, base, ModelFeatures::default())
        .unwrap();
    assert_eq!(primary.model, ModelInput::Resource(200));
    assert_eq!(primary.skin, 0x302);
    assert_eq!(primary.capabilities, base);
    assert_eq!(primary.capabilities.alpha, 0.3);
    for (cell, expected, alpha) in [
        (1, ModelInput::LinkedTranslucent(7), 0.32),
        (2, ModelInput::Resource(201), 1.0),
        (3, ModelInput::Resource(202), 1.0),
    ] {
        let linked = rules
            .model_cell(&appearance, cell, base, ModelFeatures::default())
            .unwrap();
        assert_eq!(linked.model, expected);
        assert_eq!(linked.skin, 0);
        let caps = linked.capabilities;
        assert!(!caps.viewer_model && !caps.weapon_model && !caps.depth_hack);
        assert!(!caps.fullbright && !caps.infrared_visible && !caps.alias_motion);
        assert_eq!(caps.beam_radius, None);
        assert_eq!(caps.alpha, alpha);
        assert_eq!(
            caps.inline_opacity,
            if cell == 1 { Some(0.25) } else { None }
        );
        assert_eq!(caps.scale, base.scale);
        assert_eq!(caps.frame_lerp, base.frame_lerp);
        assert_eq!(caps.state_old_frame, base.state_old_frame);
    }
    assert_eq!(appearance.models, references);
    assert_eq!(appearance.frame, 91);
    assert_eq!(appearance.skin, 0x302);
    assert!(
        rules
            .model_cell(&appearance, 4, base, ModelFeatures::default())
            .is_none()
    );
    appearance.models[1] = None;
    assert!(
        rules
            .model_cell(&appearance, 1, base, ModelFeatures::default())
            .is_none()
    );
}

#[test]
fn rerelease_linked_cells_keep_explicit_alpha_and_full_ordinals() {
    let namespace = ResourceNamespace::Client(ClientId(0));
    for (alpha, expected) in [(0.0, 1.0), (-0.0, 1.0), (0.4, 0.4), (1.0, 1.0)] {
        let mut appearance = EntityAppearance {
            source: RuleSetId::Quake2Rerelease,
            models: [ModelRef::native(namespace, 0x87); 4],
            skin: 0x302,
            alpha,
            scale: 2.0,
            ..Default::default()
        };
        appearance.native.render_flags = 8 | 16 | 32 | 64 | 0x8000 | (1 << 22);
        let rules = policy(appearance.source);
        let base = rules.project(&appearance);
        assert_eq!(
            rules
                .model_cell(&appearance, 0, base, ModelFeatures::default())
                .unwrap()
                .capabilities,
            base
        );
        for cell in 1..4 {
            let linked = rules
                .model_cell(&appearance, cell, base, ModelFeatures::default())
                .unwrap();
            assert_eq!(linked.model, ModelInput::Resource(135));
            assert_eq!(linked.skin, 0);
            assert_eq!(linked.capabilities.alpha, expected);
            assert_eq!(
                linked.capabilities.inline_opacity,
                if expected == 1.0 {
                    None
                } else {
                    Some(expected)
                }
            );
            assert!(!linked.capabilities.fullbright && !linked.capabilities.depth_hack);
            assert!(!linked.capabilities.infrared_visible);
            assert_eq!(linked.capabilities.scale, base.scale);
            assert_eq!(linked.capabilities.frame_lerp, base.frame_lerp);
            assert_eq!(linked.capabilities.state_old_frame, base.state_old_frame);
        }
    }
}

#[test]
fn configured_player_cells_use_skin_zero_without_resetting_other_sources() {
    let namespace = ResourceNamespace::Client(ClientId(0));
    for source in [RuleSetId::Quake2, RuleSetId::Quake2Rerelease] {
        let appearance = EntityAppearance {
            source,
            models: [ModelRef::native(namespace, 255); 4],
            skin: 0x302,
            ..Default::default()
        };
        let rules = policy(source);
        let base = rules.project(&appearance);
        for cell in 0..2 {
            let model = rules
                .model_cell(&appearance, cell, base, ModelFeatures::default())
                .unwrap();
            assert_eq!(model.model, ModelInput::Player);
            assert_eq!(model.skin, 0);
        }
    }
    for source in [RuleSetId::Quake, RuleSetId::QuakeWorld, RuleSetId::Quake3] {
        let appearance = EntityAppearance {
            source,
            models: [ModelRef::native(namespace, 255); 4],
            skin: 17,
            alpha: 0.45,
            ..Default::default()
        };
        let rules = policy(source);
        let base = rules.project(&appearance);
        for cell in 0..4 {
            let model = rules
                .model_cell(&appearance, cell, base, ModelFeatures::default())
                .unwrap();
            assert_eq!(model.model, ModelInput::Resource(255));
            assert_eq!(model.skin, 17);
            assert_eq!(model.capabilities, base);
        }
    }
}

#[test]
fn rerelease_shell_features_are_cached_with_exact_folded_path_semantics() {
    let rerelease = policy(RuleSetId::Quake2Rerelease);
    for path in [
        &b"models/items/shell/tris.md2"[..],
        &b"MODELS/ITEMS/SHELL/TRIS.MD2"[..],
    ] {
        assert!(rerelease.model_features(path).linked_shell);
        for source in [
            RuleSetId::Quake,
            RuleSetId::QuakeWorld,
            RuleSetId::Quake2,
            RuleSetId::Quake3,
        ] {
            assert!(!policy(source).model_features(path).linked_shell);
        }
    }
    for path in [
        &b"models/items/shell/tris.md2.bak"[..],
        &b"models/items/shell/tris.md"[..],
        &b"models\\items\\shell\\tris.md2"[..],
        &b"./models/items/shell/tris.md2"[..],
        &b""[..],
    ] {
        assert_eq!(rerelease.model_features(path), ModelFeatures::default());
    }
}

#[test]
fn rerelease_shell_alpha_applies_only_to_the_selected_secondary_cell() {
    let namespace = ResourceNamespace::Client(ClientId(0));
    for (alpha, expected) in [(0.0, 0.32), (0.5, 0.16)] {
        let appearance = EntityAppearance {
            source: RuleSetId::Quake2Rerelease,
            models: [ModelRef::native(namespace, 0x87); 4],
            alpha,
            scale: 2.0,
            ..Default::default()
        };
        let rules = policy(appearance.source);
        let base = rules.project(&appearance);
        let features = rules.model_features(b"MODELS/ITEMS/SHELL/TRIS.MD2");
        for cell in 0..4 {
            let selected = rules.model_cell(&appearance, cell, base, features).unwrap();
            assert_eq!(selected.model, ModelInput::Resource(135));
            assert_eq!(selected.capabilities.scale, base.scale);
            if cell == 1 {
                assert_eq!(selected.capabilities.alpha, expected);
                assert_eq!(selected.capabilities.inline_opacity, Some(expected));
            } else if cell == 0 {
                assert_eq!(selected.capabilities, base);
            } else {
                assert_eq!(
                    selected.capabilities.alpha,
                    if alpha == 0.0 { 1.0 } else { alpha }
                );
            }
        }
    }
}

#[test]
fn player_model_rules_keep_source_limits_fallbacks_and_packed_selection_separate() {
    for source in [RuleSetId::Quake, RuleSetId::QuakeWorld, RuleSetId::Quake3] {
        assert!(policy(source).player_models().is_none());
    }
    let classic = policy(RuleSetId::Quake2).player_models().unwrap();
    let rerelease = policy(RuleSetId::Quake2Rerelease).player_models().unwrap();
    assert_eq!((classic.weapon_limit, classic.weapon_start), (20, 1));
    assert!(classic.require_icon);
    assert!(!classic.partial_base);
    assert!(!classic.female_skin_fallback && !classic.restricted_paths);
    assert!(!classic.skip_player_model_gap && !classic.folded_weapon_fallback);
    assert_eq!((rerelease.weapon_limit, rerelease.weapon_start), (256, 2));
    assert!(!rerelease.require_icon);
    assert!(rerelease.partial_base);
    assert!(rerelease.female_skin_fallback && rerelease.restricted_paths);
    assert!(rerelease.skip_player_model_gap && rerelease.folded_weapon_fallback);
    assert_eq!(classic.player_selection(0x0502), (2, 5));
    assert_eq!(rerelease.player_selection(0x0502), (2, 5));
    // RR's signed height byte and team/POI byte cannot become a weapon index.
    assert_eq!(classic.player_selection(0x1234_0502), (2, 0x123405));
    assert_eq!(rerelease.player_selection(0x1234_0502), (2, 5));
    assert_eq!(rerelease.player_selection(-1), (255, 255));
    let (client, weapon) = classic.player_selection(-1);
    assert_eq!(client, 255);
    assert!(weapon >= classic.weapon_limit);
}

fn pose(
    presentation: RuleSetId,
    angles: Vec3,
    velocity: Vec3,
    command_angles: bool,
    software: bool,
    roll: AliasRoll,
) -> Vec3 {
    let source = if command_angles {
        RuleSetId::QuakeWorld
    } else {
        presentation
    };
    let body = policy(source).body_angles(angles, velocity, command_angles, roll);
    model_presentation(presentation).pose.angles(body, software)
}

#[test]
fn cached_source_and_model_policies_remain_independent_capabilities() {
    use qa_gameplay::rules::appearance::{WeaponInterpolation, model_source};
    let peer = policy(RuleSetId::QuakeWorld);
    let model = model_presentation(RuleSetId::Quake2);
    let angles = Vec3([90.0, 0.0, 37.0]);
    let body = peer.body_angles(
        angles,
        Vec3([0.0, 100.0, 0.0]),
        true,
        AliasRoll {
            angle: 2.0,
            speed: 200.0,
        },
    );
    assert_eq!(body, Vec3([-30.0, 0.0, -4.0]));
    assert_eq!(model.pose.angles(body, true), body);
    assert_eq!(model.pose.angles(body, false), Vec3([-30.0, 0.0, 4.0]));
    assert_eq!(angles, Vec3([90.0, 0.0, 37.0]));
    assert!(peer.adjacent(10, 1));
    assert_eq!(peer.weapon_interpolation(), WeaponInterpolation::None);
    let classic = policy(RuleSetId::Quake2);
    assert!(classic.adjacent(0, u32::MAX));
    assert!(!classic.adjacent(10, 1));
    assert_eq!(
        classic.weapon_interpolation(),
        WeaponInterpolation::Snapshot
    );
    let enhanced = policy(RuleSetId::Quake2Rerelease);
    assert_eq!(enhanced.weapon_interpolation(), WeaponInterpolation::Rate);
    assert!(enhanced.player_event_check());
    for (source, zero, pcx, software) in [
        (RuleSetId::Quake, true, false, false),
        (RuleSetId::QuakeWorld, true, false, false),
        (RuleSetId::Quake2, true, true, true),
        (RuleSetId::Quake2Rerelease, true, true, true),
        (RuleSetId::Quake3, false, false, false),
    ] {
        assert_eq!(policy(source).world_zero(), zero);
        assert!(!policy(source).is_interpolation_event(&EntityAppearance::default()));
        assert_eq!(model_source(source).indexed_external_pcx, pcx);
        assert_eq!(model_presentation(source).software_alias, software);
    }
}

#[test]
fn native_alias_axes_preserve_each_renderers_pitch_and_roll_conventions() {
    let angles = Vec3([30.0, 20.0, 40.0]);
    for source in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        for software in [false, true] {
            assert_eq!(
                pose(
                    source,
                    angles,
                    Vec3::default(),
                    false,
                    software,
                    AliasRoll::default()
                ),
                Vec3([-30.0, 20.0, 40.0])
            );
        }
    }
    for source in [RuleSetId::Quake2, RuleSetId::Quake2Rerelease] {
        assert_eq!(
            pose(
                source,
                angles,
                Vec3::default(),
                false,
                true,
                AliasRoll::default()
            ),
            angles
        );
        assert_eq!(
            pose(
                source,
                angles,
                Vec3::default(),
                false,
                false,
                AliasRoll::default()
            ),
            Vec3([30.0, 20.0, -40.0])
        );
    }
    let q1 = pose(
        RuleSetId::Quake,
        Vec3([30.0, 0.0, 0.0]),
        Vec3::default(),
        false,
        true,
        AliasRoll::default(),
    );
    assert_eq!(
        math::angle_vectors(q1).axes()[0].0[2].to_bits(),
        0.5f32.to_bits()
    );
    let q2_gl = pose(
        RuleSetId::Quake2,
        Vec3([0.0, 0.0, 30.0]),
        Vec3::default(),
        false,
        false,
        AliasRoll::default(),
    );
    assert_eq!(
        math::angle_vectors(q2_gl).axes()[1].0[2].to_bits(),
        (-0.5f32).to_bits()
    );
}

#[test]
fn qw_playerinfo_converts_command_pitch_and_velocity_roll_without_changing_packet_entities() {
    let roll = AliasRoll {
        angle: 2.0,
        speed: 200.0,
    };
    let angles = Vec3([30.0, 0.0, 37.0]);
    for (velocity, expected_roll) in [(100.0, -4.0), (200.0, -8.0), (-100.0, 4.0)] {
        assert_eq!(
            pose(
                RuleSetId::QuakeWorld,
                angles,
                Vec3([0.0, velocity, 0.0]),
                true,
                true,
                roll
            ),
            Vec3([10.0, 0.0, expected_roll])
        );
    }
    assert_eq!(
        pose(
            RuleSetId::QuakeWorld,
            angles,
            Vec3([0.0, 100.0, 0.0]),
            false,
            true,
            roll
        ),
        Vec3([-30.0, 0.0, 37.0])
    );
    assert_eq!(
        pose(
            RuleSetId::Quake2,
            angles,
            Vec3([0.0, 25.0, 0.0]),
            true,
            true,
            AliasRoll {
                angle: 2.5,
                speed: 50.0
            }
        ),
        Vec3([-10.0, 0.0, -5.0])
    );
    // Native V_CalcRoll takes its capped branch when speed is zero.
    assert_eq!(
        pose(
            RuleSetId::QuakeWorld,
            angles,
            Vec3::default(),
            true,
            true,
            AliasRoll {
                angle: 2.0,
                speed: 0.0
            }
        ),
        Vec3([10.0, 0.0, 8.0])
    );
}

#[test]
fn source_flags_do_not_inherit_another_games_render_flag_semantics() {
    let mut appearance = EntityAppearance {
        source: RuleSetId::Quake2,
        ..Default::default()
    };
    appearance.native.render_flags = 2 | 4 | 8 | 16 | 64;
    let q2 = policy(appearance.source).project(&appearance);
    assert!(q2.viewer_model && q2.weapon_model && q2.fullbright && q2.depth_hack && q2.frame_lerp);
    assert_eq!(q2.unsupported, 0);
    appearance.source = RuleSetId::Quake3;
    let q3 = policy(appearance.source).project(&appearance);
    assert!(
        !q3.viewer_model && !q3.weapon_model && !q3.fullbright && !q3.depth_hack && !q3.frame_lerp
    );
    assert_eq!(q3.unsupported, u64::from(appearance.native.render_flags));
    assert!(q3.beam_radius.is_none());
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
    let effective = policy(appearance.source).project(&appearance);
    assert_eq!(effective.alpha, 0.7);
    assert_eq!(effective.scale, 1.0);
    assert_eq!(appearance.alpha.to_bits(), native_alpha);
    assert_eq!(appearance.scale.to_bits(), native_scale);
    appearance.alpha = 0.25;
    appearance.scale = 2.0;
    appearance.native.render_flags = 0;
    let explicit = policy(appearance.source).project(&appearance);
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
        let projected = policy(appearance.source).project(&appearance);
        assert_eq!(projected.alpha, expected);
        assert_ne!(projected.unsupported, 0); // Scalar decoding supplies no particles.
    }
    appearance.source = RuleSetId::Quake2;
    appearance.native.effects = 0;
    appearance.native.render_flags = 32 | 8;
    assert_eq!(policy(appearance.source).project(&appearance).alpha, 0.0);
    appearance.native.render_flags = 32;
    assert_eq!(policy(appearance.source).project(&appearance).alpha, 0.7);
}

#[test]
fn native_beams_select_radius_and_motion_without_becoming_ordinary_models() {
    let mut appearance = EntityAppearance {
        source: RuleSetId::Quake2,
        ..Default::default()
    };
    appearance.native.render_flags = 128;
    appearance.frame = 7;
    let q2 = policy(appearance.source).project(&appearance);
    assert_eq!(q2.unsupported, 0);
    assert_eq!(q2.beam_radius, Some(3.0));
    assert!(!q2.beam_interpolate);
    appearance.source = RuleSetId::Quake2Rerelease;
    let rerelease = policy(appearance.source).project(&appearance);
    assert_eq!(rerelease.beam_radius, Some(3.5));
    assert!(rerelease.beam_interpolate);
    appearance.source = RuleSetId::Quake3;
    assert!(
        policy(appearance.source)
            .project(&appearance)
            .beam_radius
            .is_none()
    );
    assert_ne!(
        policy(appearance.source).project(&appearance).unsupported,
        0
    );
}

#[test]
fn rerelease_keeps_old_frame_selection_separate_from_discrete_origin_selection() {
    let mut appearance = EntityAppearance {
        source: RuleSetId::Quake2Rerelease,
        ..Default::default()
    };
    appearance.native.render_flags = 1 << 22;
    let old_frame = policy(appearance.source).project(&appearance);
    assert!(old_frame.state_old_frame && !old_frame.frame_lerp);
    appearance.native.render_flags = 64;
    let discrete = policy(appearance.source).project(&appearance);
    assert!(!discrete.state_old_frame && discrete.frame_lerp);
    assert!(!discrete.alias_motion);
    appearance.source = RuleSetId::Quake2;
    let classic = policy(appearance.source).project(&appearance);
    assert!(classic.frame_lerp && classic.alias_motion);
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
            let actual = model_presentation(source)
                .pose
                .light_direction(angles)
                .unwrap();
            assert_eq!(actual.0.map(f32::to_bits), expected.map(f32::to_bits));
        }
    }
    assert_eq!(
        model_presentation(RuleSetId::Quake3)
            .pose
            .light_direction(Vec3::default()),
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
    assert!(policy(current.source).can_interpolate(
        &previous,
        &current,
        Vec3::default(),
        Vec3([512.875, 0.0, 0.0])
    ));
    assert!(!policy(current.source).can_interpolate(
        &previous,
        &current,
        Vec3::default(),
        Vec3([-513.0, 0.0, 0.0])
    ));
    current.models[0] = ModelRef::native(ResourceNamespace::Client(ClientId(1)), 2);
    assert!(!policy(current.source).can_interpolate(
        &previous,
        &current,
        Vec3::default(),
        Vec3::default()
    ));
    current = previous;
    for event in [6, 7] {
        current.native.event = event;
        assert!(!policy(current.source).can_interpolate(
            &previous,
            &current,
            Vec3::default(),
            Vec3::default()
        ));
    }
    current.native.event = 0;
    current.source = RuleSetId::Quake3;
    assert!(!policy(current.source).can_interpolate(
        &previous,
        &current,
        Vec3::default(),
        Vec3::default()
    ));
    let previous = current;
    current.native.event = 6;
    assert!(policy(current.source).can_interpolate(
        &previous,
        &current,
        Vec3::default(),
        Vec3::default()
    ));
}

#[test]
fn inline_opacity_uses_native_brush_rules_independently_of_alias_alpha() {
    use qa_core::primitives::{EntityAppearance, RuleSetId};
    let mut record = EntityAppearance::default();
    record.native.render_flags = 32;
    record.alpha = 0.5;
    let classic = policy(RuleSetId::Quake2).project(&record);
    assert_eq!(classic.inline_opacity, Some(0.25));
    assert_eq!(classic.alpha, 0.7);
    let rerelease = policy(RuleSetId::Quake2Rerelease).project(&record);
    assert_eq!(rerelease.inline_opacity, Some(0.5));
    assert_eq!(rerelease.alpha, 0.5);
    record.native.render_flags = 0;
    assert_eq!(
        policy(RuleSetId::Quake2).project(&record).inline_opacity,
        None
    );
}

#[test]
fn infrared_eligibility_and_native_palette_are_selected_at_the_appearance_boundary() {
    for source in [RuleSetId::Quake2, RuleSetId::Quake2Rerelease] {
        let mut appearance = EntityAppearance::default();
        appearance.native.render_flags = 0x8000;
        assert!(policy(source).project(&appearance).infrared_visible);
        appearance.native.render_flags = 0;
        assert!(!policy(source).project(&appearance).infrared_visible);
        let map = model_presentation(source).infrared_map.unwrap();
        assert_eq!((map[0], map[15], map[255]), (79, 64, 68));
    }
    for source in [RuleSetId::Quake, RuleSetId::QuakeWorld, RuleSetId::Quake3] {
        assert!(model_presentation(source).infrared_map.is_none());
        let mut appearance = EntityAppearance::default();
        appearance.native.render_flags = 0x8000;
        assert!(!policy(source).project(&appearance).infrared_visible);
    }
}
