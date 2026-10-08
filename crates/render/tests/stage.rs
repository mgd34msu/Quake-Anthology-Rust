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
    let mut assets = Assets::load();
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
