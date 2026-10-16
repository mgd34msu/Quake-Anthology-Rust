use qa_core::primitives::Vec3;
use qa_render::assets::{
    Assets, Flow, ImageId, MaterialSettings, Stage, StageTexture, TcMod, UvWarp, Vertex,
};
use qa_render::shader::{
    AlphaFunc, AlphaGen, BlendFactor, Deform, RgbGen, StageBlend, TexCoordGen, TexMod,
    WaveFunction, Waveform,
};
use qa_render::stage::{DrawInputs, StageError, StageEvaluator, alpha_pass, blend_pixel};

fn shade(
    evaluator: &StageEvaluator,
    stage: Stage,
    inputs: DrawInputs,
    vertex: Vertex,
) -> qa_render::stage::EvaluatedVertex {
    let prepared = evaluator
        .prepare(&stage, MaterialSettings::default(), inputs)
        .unwrap();
    evaluator.evaluate(&prepared, &vertex)
}
fn wave(function: WaveFunction) -> Waveform {
    Waveform {
        function,
        base: 0.0,
        amplitude: 1.0,
        phase: 0.0,
        frequency: 1.0,
    }
}

#[test]
fn noise_retains_original_four_axis_weighted_interpolation_bits() {
    // The original tr_noise.c R_NoiseGet4f generated these bits with the same
    // injected tables (cc -O2 -ffp-contract=off), independent of Rust helpers.
    let evaluator = StageEvaluator::with_noise(
        std::array::from_fn(|i| (((i * 73) % 257) as i32 - 128) as f32 / 128.0),
        std::array::from_fn(|i| ((i * 97 + 31) & 255) as u8),
    );
    for (point, expected) in [
        ([0.125, 0.375, 0.625, 0.875], 0xbe86_b3c0),
        ([-0.25, 13.137, -7.25, 0.33333334], 0x3df4_4ccd),
        ([255.9, -256.125, 17.03125, 2.7182817], 0xbe9f_f91a),
        ([0.1, 0.2, 0.3, 0.4], 0xbe4e_0d1d),
    ] {
        assert_eq!(evaluator.noise(point).to_bits(), expected, "{point:?}");
    }
}

#[test]
fn stage_inputs_transform_only_model_clip_planes_and_clear_them_for_2d() {
    use qa_core::primitives::Plane;
    use qa_render::{Refdef, SceneEntity, SceneModel};
    let plane = Plane::oriented(Vec3([0., 1., 0.]), 3.);
    let view = DrawInputs::for_view(&Refdef {
        clip_plane: Some(plane),
        ..Default::default()
    });
    let entity = SceneEntity {
        origin: Vec3([1., 2., 3.]),
        axes: [Vec3([0., 1., 0.]), Vec3([-1., 0., 0.]), Vec3([0., 0., 1.])],
        ..Default::default()
    };
    assert_eq!(
        view.for_entity(&entity).clip_plane.unwrap(),
        Plane::oriented(Vec3([1., 0., 0.]), 1.)
    );
    assert_eq!(
        view.for_entity(&SceneEntity {
            model: SceneModel::Sprite {
                radius: 1.,
                rotation: 0.
            },
            ..entity
        })
        .clip_plane,
        Some(plane)
    );
    assert!(DrawInputs::default().clip_plane.is_none());
}

#[test]
fn reflected_views_use_one_shared_culling_policy_for_all_backends() {
    use qa_render::{Refdef, shader::Cull};
    let input = DrawInputs::for_view(&Refdef {
        reflected_view: true,
        ..Default::default()
    });
    assert!(input.reflected_view);
    for (ordinary, reflected) in [
        (Cull::Front, Cull::Back),
        (Cull::Back, Cull::Front),
        (Cull::None, Cull::None),
    ] {
        assert_eq!(ordinary.for_view(false), ordinary);
        assert_eq!(ordinary.for_view(input.reflected_view), reflected);
    }
}

#[test]
fn native_identity_does_not_apply_entity_tint_and_vertex_colors_quantize_first() {
    let evaluator = StageEvaluator::load();
    let inputs = DrawInputs {
        entity_color: [30, 60, 90, 120],
        identity_light: 0.5,
        ..DrawInputs::default()
    };
    let vertex = Vertex {
        color: [101, 151, 201, 57],
        ..Vertex::default()
    };
    assert_eq!(
        shade(&evaluator, Stage::default(), inputs, vertex).color,
        [255; 4]
    );
    let stage = Stage {
        rgb_gen: RgbGen::Vertex,
        alpha_gen: AlphaGen::Vertex,
        ..Stage::default()
    };
    assert_eq!(
        shade(&evaluator, stage, inputs, vertex).color,
        [50, 75, 100, 57]
    );
    let identity_alpha = Stage {
        alpha_gen: AlphaGen::Identity,
        ..stage
    };
    assert_eq!(
        shade(&evaluator, identity_alpha, inputs, vertex).color[3],
        255
    );
    assert_eq!(
        shade(
            &evaluator,
            identity_alpha,
            DrawInputs {
                identity_light: 1.0,
                ..inputs
            },
            vertex
        )
        .color,
        vertex.color
    );
    let entity = Stage {
        rgb_gen: RgbGen::Entity,
        alpha_gen: AlphaGen::Entity,
        ..Stage::default()
    };
    assert_eq!(
        shade(&evaluator, entity, inputs, vertex).color,
        inputs.entity_color
    );
}

#[test]
fn native_table_waves_and_animation_share_fixed_phase() {
    let evaluator = StageEvaluator::load();
    assert_eq!(
        evaluator.wave(wave(WaveFunction::Sawtooth), 0.25).unwrap(),
        0.25
    );
    assert_eq!(
        evaluator
            .wave(wave(WaveFunction::InverseSawtooth), 0.25)
            .unwrap(),
        0.75
    );
    assert_eq!(
        evaluator.wave(wave(WaveFunction::Triangle), 0.75).unwrap(),
        -1.0
    );
    // Native tr_init uses 1023 as the sine denominator; phase .25 is not
    // analytic sin(pi/2), and the same table goes to the GL texture buffer.
    let angle = (256.0_f32 * 360.0 / 1023.0) * std::f32::consts::PI / 180.0;
    assert_eq!(
        evaluator.wave(wave(WaveFunction::Sin), 0.25).unwrap(),
        (angle as f64).sin() as f32
    );
    let stage = Stage {
        texture: StageTexture::Animation {
            images: std::array::from_fn(|i| ImageId(i as u32 + 1)),
            count: 8,
            frequency: 10.0,
        },
        ..Stage::default()
    };
    let prepared = evaluator
        .prepare(
            &stage,
            MaterialSettings::default(),
            DrawInputs {
                time_ms: 150,
                ..DrawInputs::default()
            },
        )
        .unwrap();
    assert_eq!(prepared.image, ImageId(2));
    assert_eq!(
        evaluator
            .prepare(
                &stage,
                MaterialSettings::default(),
                DrawInputs {
                    entity_shader_time: 0.5,
                    ..DrawInputs::default()
                }
            )
            .unwrap()
            .image,
        ImageId(1)
    );
    let shifted = evaluator
        .prepare(
            &stage,
            MaterialSettings {
                time_offset: 0.1,
                clamp_time: Some(0.2),
                ..MaterialSettings::default()
            },
            DrawInputs {
                time_ms: 1000,
                ..DrawInputs::default()
            },
        )
        .unwrap();
    assert_eq!(shifted.shader_time, 0.2);
    assert_eq!(shifted.image, ImageId(3));
}

#[test]
fn ordered_modifiers_preserve_native_matrix_columns() {
    let evaluator = StageEvaluator::load();
    let stage = Stage {
        tcmods: [
            Some(TcMod::Script(TexMod::Scroll([0.5, 0.0]))),
            Some(TcMod::Script(TexMod::Scale([2.0, 3.0]))),
            Some(TcMod::Script(TexMod::Transform {
                matrix: [[1.0, 2.0], [3.0, 4.0]],
                translate: [5.0, 6.0],
            })),
            None,
        ],
        ..Stage::default()
    };
    let vertex = Vertex {
        texcoord: [0.5, 0.25],
        ..Vertex::default()
    };
    let out = shade(
        &evaluator,
        stage,
        DrawInputs {
            time_ms: 500,
            ..DrawInputs::default()
        },
        vertex,
    );
    assert_eq!(out.texcoord, [8.75, 12.0]);
    let reversed = Stage {
        tcmods: [stage.tcmods[1], stage.tcmods[0], stage.tcmods[2], None],
        ..stage
    };
    assert_ne!(
        shade(
            &evaluator,
            reversed,
            DrawInputs {
                time_ms: 500,
                ..DrawInputs::default()
            },
            vertex
        )
        .texcoord,
        out.texcoord
    );
}

#[test]
fn liquid_lookup_and_flow_preserve_quantized_native_coordinates() {
    let evaluator = StageEvaluator::load();
    let stage = Stage {
        tcmods: [
            Some(TcMod::Warp(UvWarp {
                texel_scale: [64.0; 2],
                amplitude: [0.125; 2],
                frequency: 0.125,
                time_scale: 1.0,
            })),
            None,
            None,
            None,
        ],
        ..Stage::default()
    };
    let inputs = DrawInputs {
        texture_scale: [1.0 / 64.0; 2],
        ..DrawInputs::default()
    };
    let vertex = Vertex {
        texcoord: [0.0, 1.0],
        ..Vertex::default()
    };
    let out = shade(&evaluator, stage, inputs, vertex);
    assert_eq!(out.texcoord, [0.979285 / 64.0, 1.0 / 64.0]);
    let flow = Stage {
        tcmods: [
            Some(TcMod::Flow(Flow {
                speed: 1.0 / 40.0,
                amplitude: [-64.0, 0.0],
                cycle_start: [-64.0, 0.0],
            })),
            None,
            None,
            None,
        ],
        ..Stage::default()
    };
    assert_eq!(
        shade(&evaluator, flow, DrawInputs::default(), Vertex::default()).texcoord,
        [-64.0, 0.0]
    );
    assert_eq!(
        shade(
            &evaluator,
            flow,
            DrawInputs {
                time_ms: 20000,
                ..DrawInputs::default()
            },
            Vertex::default()
        )
        .texcoord,
        [-32.0, 0.0]
    );
    assert_eq!(
        shade(
            &evaluator,
            flow,
            DrawInputs {
                time_ms: 40000,
                ..DrawInputs::default()
            },
            Vertex::default()
        )
        .texcoord,
        [-64.0, 0.0]
    );
}

#[test]
fn native_turb_ignores_base_and_uses_spatial_coordinates() {
    let evaluator = StageEvaluator::load();
    let stage = Stage {
        tcmods: [
            Some(TcMod::Script(TexMod::Turbulent {
                base: 10.0,
                amplitude: 0.25,
                phase: 0.0,
                frequency: 0.0,
            })),
            None,
            None,
            None,
        ],
        ..Stage::default()
    };
    let vertex = Vertex {
        position: Vec3([128.0, 256.0, 128.0]),
        ..Vertex::default()
    };
    let out = shade(&evaluator, stage, DrawInputs::default(), vertex);
    let expected = evaluator.wave(wave(WaveFunction::Sin), 0.25).unwrap() * 0.25;
    assert_eq!(out.texcoord, [expected; 2]);
    let vector = Stage {
        texgen: TexCoordGen::Vector([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]),
        ..Stage::default()
    };
    assert_eq!(
        shade(
            &evaluator,
            vector,
            DrawInputs {
                texture_scale: [0.01; 2],
                ..DrawInputs::default()
            },
            vertex
        )
        .texcoord,
        [128.0, 256.0]
    );
}

#[test]
fn native_deforms_execute_in_order_and_reject_geometry_only_requirements() {
    let evaluator = StageEvaluator::load();
    let settings = MaterialSettings {
        deforms: [
            Some(Deform::Wave {
                spread: 20.0,
                wave: Waveform {
                    base: 2.0,
                    frequency: 0.0,
                    ..wave(WaveFunction::Sin)
                },
            }),
            Some(Deform::Move {
                vector: [1.0, 0.0, 0.0],
                wave: Waveform {
                    base: 3.0,
                    frequency: 0.0,
                    ..wave(WaveFunction::Sin)
                },
            }),
            None,
        ],
        ..MaterialSettings::default()
    };
    let vertex = Vertex {
        position: Vec3([100.0, 200.0, 300.0]),
        ..Vertex::default()
    };
    assert_eq!(
        evaluator
            .deform_vertex(&settings, &DrawInputs::default(), vertex)
            .unwrap()
            .position,
        Vec3([103.0, 200.0, 302.0])
    );
    let unsupported = MaterialSettings {
        deforms: [Some(Deform::AutoSprite), None, None],
        ..settings
    };
    assert_eq!(
        evaluator.deform_vertex(&unsupported, &DrawInputs::default(), vertex),
        Err(StageError::GeometryDeform)
    );
    let diffuse = Stage {
        rgb_gen: RgbGen::LightingDiffuse,
        ..Stage::default()
    };
    assert!(matches!(
        evaluator.prepare(&diffuse, settings, DrawInputs::default()),
        Err(StageError::MissingLighting)
    ));
}

#[test]
fn native_general_blend_factors_and_alpha_thresholds() {
    let blend = Some(StageBlend {
        source: BlendFactor::DestinationColor,
        destination: BlendFactor::OneMinusDestinationAlpha,
    });
    let out = blend_pixel(blend, [0.4, 0.6, 0.8, 1.0], [0.25, 0.5, 0.75, 0.2]);
    for (value, expected) in out.into_iter().zip([0.3, 0.7, 1.0, 0.36]) {
        assert!((value - expected).abs() < 0.00001);
    }
    assert!(!alpha_pass(AlphaFunc::GreaterZero, 0.0));
    assert!(alpha_pass(AlphaFunc::LessThanHalf, 127.0 / 255.0));
    assert!(!alpha_pass(AlphaFunc::LessThanHalf, 128.0 / 255.0));
    assert!(alpha_pass(AlphaFunc::AtLeastHalf, 128.0 / 255.0));
    assert_eq!(
        blend_pixel(None, [0.1, 0.2, 0.3, 0.4], [0.5; 4]),
        [0.1, 0.2, 0.3, 0.4]
    );
}

#[test]
fn native_specular_uses_fixed_light_without_diffuse_sampling() {
    let evaluator = StageEvaluator::load();
    let inputs = DrawInputs {
        view_origin: Vec3([-960.0, 1980.0, 96.0]),
        ..DrawInputs::default()
    };
    let vertex = Vertex {
        position: Vec3([-960.0, 1980.0, 88.0]),
        normal: Vec3([0.0, 0.0, 1.0]),
        ..Vertex::default()
    };
    let stage = Stage {
        alpha_gen: AlphaGen::LightingSpecular,
        ..Stage::default()
    };
    // Original Q_rsqrt single iteration makes two normalized axial vectors
    // slightly shorter than one; RB_CalcSpecularAlpha truncates their l^4.
    assert_eq!(
        shade(&evaluator, stage, inputs, vertex).color,
        [255, 255, 255, 251]
    );
}

#[test]
fn material_registration_validates_at_load_and_handles_remain_numeric() {
    let mut assets = Assets::load().unwrap();
    let invalid = Stage {
        texture: StageTexture::Animation {
            images: [ImageId(0); 8],
            count: 9,
            frequency: 1.0,
        },
        ..Stage::default()
    };
    assert!(
        assets
            .register_material("bad", &[invalid], MaterialSettings::default())
            .is_err()
    );
    let invalid = Stage {
        rgb_gen: RgbGen::Const([f32::NAN, 1.0, 1.0]),
        ..Stage::default()
    };
    assert!(
        assets
            .register_material("bad", &[invalid], MaterialSettings::default())
            .is_err()
    );
    let shared = Stage {
        texture: StageTexture::Lightmap,
        is_lightmap: true,
        ..Stage::default()
    };
    let id = assets
        .register_material("lightmap", &[shared], MaterialSettings::default())
        .unwrap();
    assert_eq!(
        assets
            .register_material("lightmap", &[shared], MaterialSettings::default())
            .unwrap(),
        id
    );
}

#[test]
fn native_entity_view_uses_submitted_axis_scale_policy() {
    use qa_core::primitives::Vec3;
    use qa_render::{SceneEntity, stage::entity_view_origin};
    let mut entity = SceneEntity {
        origin: Vec3([4.0, 5.0, 6.0]),
        ..SceneEntity::default()
    };
    let view = Vec3([68.0, 37.0, 22.0]);
    assert_eq!(entity_view_origin(view, &entity), Vec3([64.0, 32.0, 16.0]));
    entity.axes = entity.axes.map(|axis| Vec3(axis.0.map(|v| v * 2.0)));
    assert_eq!(entity_view_origin(view, &entity), Vec3([128.0, 64.0, 32.0]));
    entity.non_normalized_axes = true;
    assert_eq!(entity_view_origin(view, &entity), Vec3([64.0, 32.0, 16.0]));
    entity.axes[0] = Vec3::default();
    assert_eq!(entity_view_origin(view, &entity), Vec3::default());
}

#[test]
fn default_noise_tables_preserve_all_seed1001_native_glibc_values() {
    // Independent glibc 2.44 srand/rand, with native f32 conversion at every
    // assignment. Each tuple is the noise bits and the following permutation.
    let expected: [(u32, u8); 256] = [
        (0xbf1018e9, 246),
        (0xbe166bc8, 230),
        (0xbdfe4958, 131),
        (0xbe95c46c, 73),
        (0xbeedd54a, 57),
        (0xbde5b380, 7),
        (0xbf2508e4, 201),
        (0x3e086a48, 143),
        (0xbef5e02e, 0),
        (0x3f561668, 251),
        (0x3f4c9320, 146),
        (0x3ce92a00, 115),
        (0xbe55fff0, 203),
        (0x3f5aec46, 72),
        (0xbf383e5a, 23),
        (0x3ef3e22c, 91),
        (0xbf6197ef, 42),
        (0xbef22c64, 126),
        (0x3eb9edb8, 157),
        (0x3f13064c, 242),
        (0x3f2f56da, 58),
        (0x3f754b66, 5),
        (0xbf746b78, 139),
        (0x3e279458, 72),
        (0x3dbcf750, 127),
        (0xbeee01dc, 113),
        (0xbf597d8d, 199),
        (0x3f4baf48, 120),
        (0x3e1fa9d8, 210),
        (0x3f037888, 183),
        (0x3f5585ee, 126),
        (0xbf58ab26, 248),
        (0x3ea37f80, 86),
        (0xbd5731a0, 87),
        (0x3f6a4642, 66),
        (0xbed67a5c, 204),
        (0xbc839e60, 68),
        (0x3f23eb10, 131),
        (0x3f21742e, 102),
        (0x3f181c06, 92),
        (0x3f4dcba2, 16),
        (0x3f1d4522, 249),
        (0x3f316378, 179),
        (0xbdd57ba8, 108),
        (0x3d78ff80, 52),
        (0xbf3705f4, 114),
        (0x3ecc6560, 55),
        (0xbe1bf060, 91),
        (0x3df46c80, 228),
        (0x3ecd5214, 131),
        (0xbf2edc01, 252),
        (0xbebb1e26, 165),
        (0xbef50bd8, 35),
        (0xbf2b815f, 19),
        (0x3da16110, 245),
        (0xbdfff528, 112),
        (0xbf724a42, 62),
        (0xbe26e8a8, 222),
        (0x3f663ae8, 220),
        (0xbecf31b6, 122),
        (0xbf5ba08c, 112),
        (0x3f5ae0c8, 196),
        (0x3ea36030, 89),
        (0xbf3d7b6e, 55),
        (0xbf008a82, 211),
        (0x3ef222f4, 103),
        (0x3f24997a, 13),
        (0xbf625f77, 20),
        (0xbf1e219c, 56),
        (0xbf2efe2f, 186),
        (0xbf214630, 151),
        (0xbf27fdb8, 54),
        (0x3f2e49a8, 150),
        (0xbf54773c, 201),
        (0xbdb10c00, 97),
        (0xbeeb611e, 134),
        (0x3f25511c, 50),
        (0xbece5fd2, 123),
        (0x3dc8a740, 109),
        (0x3ece7db8, 203),
        (0xbef6fb76, 111),
        (0xbf16dc1d, 20),
        (0xbc85a840, 67),
        (0xbf2cd0b2, 174),
        (0xbce38d60, 81),
        (0xbe311160, 171),
        (0x3f55344a, 149),
        (0x3f440cf4, 192),
        (0xbf25f9f2, 246),
        (0x3dc01410, 161),
        (0xbe98f4c0, 208),
        (0xbf2ddd7e, 44),
        (0xbf787dd4, 117),
        (0x3e9fafe0, 143),
        (0x3f464740, 91),
        (0xbe8dbd0a, 37),
        (0x3f176722, 144),
        (0xbf0c1aba, 73),
        (0x3f2909ca, 99),
        (0x3f70eb02, 80),
        (0x3ed68b64, 97),
        (0x3f798ac4, 159),
        (0x3f70649a, 221),
        (0xbe70204c, 37),
        (0x3f2d2d6e, 237),
        (0x3f0e4f36, 48),
        (0x3efc98c8, 239),
        (0xbe8cec6c, 194),
        (0xbe526ec4, 4),
        (0xbeb334b8, 72),
        (0xbe7e1df0, 175),
        (0xbe0e2ba4, 43),
        (0xbefc6706, 167),
        (0xbda733d0, 21),
        (0xbf689111, 109),
        (0xbe4cf168, 192),
        (0x3f20ab32, 98),
        (0xbe734594, 199),
        (0xbef833bc, 194),
        (0x3f5b7a74, 24),
        (0x3ec641f0, 180),
        (0xbedc72c0, 112),
        (0x3e9476c0, 165),
        (0xbf18c416, 10),
        (0x3ea96444, 134),
        (0xbeb16980, 10),
        (0xbf12d752, 193),
        (0xbf12f5b2, 119),
        (0xbe2f9538, 171),
        (0x3dce5b90, 117),
        (0xbf4bd7a3, 242),
        (0xbf11c1bf, 233),
        (0xbea644fc, 152),
        (0x3ecbc6b8, 151),
        (0xbe8ed936, 160),
        (0x3ec3ae6c, 13),
        (0xbea9b10c, 248),
        (0xbc3791c0, 249),
        (0x3e7bb8f8, 177),
        (0xbf74fd77, 73),
        (0xbf0e00cf, 88),
        (0xbeae69ee, 111),
        (0xbf49f1e1, 138),
        (0x3f4e1d08, 132),
        (0xbf1206d6, 115),
        (0x3f75b272, 80),
        (0xbe4621b8, 49),
        (0xbf090ae8, 188),
        (0x3f155d90, 237),
        (0xbea7ce8e, 38),
        (0x3df1b220, 6),
        (0xbf165aaf, 227),
        (0xbf7e723e, 178),
        (0x3f3ec098, 159),
        (0xbe536d4c, 227),
        (0x3f541c62, 157),
        (0xbf03ef17, 62),
        (0xbf63b444, 88),
        (0x3f12bcc0, 244),
        (0x3f3c47d8, 0),
        (0xbe36bb00, 216),
        (0xbeba511a, 207),
        (0xbf6a1502, 140),
        (0x3de0ef30, 212),
        (0xbd15b020, 227),
        (0x3f793982, 10),
        (0x3f56268e, 49),
        (0x3f5e3900, 234),
        (0x3f49a5dc, 205),
        (0x3dc18280, 74),
        (0x3ecd7970, 117),
        (0x3f51ee66, 240),
        (0x3ed26838, 246),
        (0xbed626b2, 125),
        (0x3f58a2e8, 40),
        (0xbc564dc0, 85),
        (0xbf7cd135, 206),
        (0xbf344306, 12),
        (0xbe8d6a74, 179),
        (0x3f446256, 215),
        (0x3e404600, 221),
        (0x3f44cb06, 130),
        (0xbf5f8556, 208),
        (0xbe084e8c, 244),
        (0x3e7ff670, 250),
        (0xbf0114f1, 83),
        (0xbdecb508, 40),
        (0xbeecf018, 37),
        (0xbf40ec67, 142),
        (0x3e8f1cb4, 11),
        (0x3edf5238, 33),
        (0xbe755558, 184),
        (0x3f63b35e, 134),
        (0x3f0bf1ae, 78),
        (0xbf09bd7b, 167),
        (0xbf331219, 210),
        (0x3d5efdc0, 8),
        (0xbea6e7da, 150),
        (0x3f34bcf2, 196),
        (0x3dc45860, 121),
        (0x3f00951c, 202),
        (0x3f1b74e2, 49),
        (0x3f684f90, 18),
        (0xbea2c810, 19),
        (0x3e86e2e4, 250),
        (0xbf40e656, 89),
        (0xbf45a08d, 128),
        (0xbf5a0d8e, 14),
        (0xbf6fd408, 216),
        (0xbe89de1a, 66),
        (0x3c0e0900, 131),
        (0xbf533d7e, 8),
        (0x3dcdbe30, 108),
        (0x3e7a4a18, 102),
        (0xbf1c38fc, 43),
        (0x3f432f48, 241),
        (0x3f6e93ac, 174),
        (0xbf38a563, 234),
        (0x3f03a102, 122),
        (0x3f7e4666, 99),
        (0xbd9ef8d0, 30),
        (0x3ef45a00, 146),
        (0x3e7c9190, 207),
        (0x3e892650, 167),
        (0x3ea40728, 254),
        (0x3f55bed8, 41),
        (0x3cf45ec0, 1),
        (0xbf1b9c48, 16),
        (0xbe10c040, 208),
        (0xbd818458, 159),
        (0x3f7a13d0, 89),
        (0x3e0fcc58, 243),
        (0xbf6e5d17, 180),
        (0x3f3f8ab8, 201),
        (0xbf1ebbb8, 222),
        (0xbf23a9a2, 166),
        (0x3f7b312c, 234),
        (0xbf0c1a7a, 156),
        (0x3eed45a4, 219),
        (0xbeecb426, 99),
        (0x3f381cca, 47),
        (0x3de710f0, 95),
        (0xbf1e19df, 191),
        (0xbdf72c28, 158),
        (0x3e10f050, 231),
        (0xbf02b2fa, 142),
        (0xbef82ea2, 207),
        (0x3cdeebc0, 74),
        (0x3d48dce0, 98),
        (0xbf55307e, 182),
        (0xbef7599c, 67),
        (0xbe899178, 63),
        (0xbf2256c0, 150),
        (0x3f39d9f0, 233),
        (0xbdc35440, 33),
        (0xbec61b12, 79),
    ];
    let evaluator = StageEvaluator::load();
    let tables = evaluator.gpu_tables();
    for (index, (noise, permutation)) in expected.into_iter().enumerate() {
        assert_eq!(
            tables[qa_render::stage::NOISE_TABLE + index].to_bits(),
            noise,
            "noise {index}"
        );
        assert_eq!(
            tables[qa_render::stage::NOISE_PERM + index],
            f32::from(permutation),
            "permutation {index}"
        );
    }
}
