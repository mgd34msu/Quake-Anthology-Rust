use qa_render::shader::*;

fn actions(shader: &ShaderDef) -> Vec<ShaderAction> {
    shader.ops.iter().map(|op| op.action.clone()).collect()
}

fn compile(catalog: &ShaderCatalog, name: &str) -> (qa_render::Assets, qa_render::MaterialId) {
    use qa_core::primitives::RuleSetId;
    use qa_render::material::{
        resources::{ImageSettings, Images},
        world_load::{NamedShaderLighting, compile_named_shader},
    };
    let vfs = qa_content::vfs::Vfs::default();
    let mut assets = qa_render::Assets::load().unwrap();
    let material = {
        let mut images = Images::new(
            &vfs,
            &mut assets,
            RuleSetId::Quake3,
            RuleSetId::Quake3,
            ImageSettings::native(RuleSetId::Quake3),
            None,
            None,
        )
        .unwrap();
        let registration = compile_named_shader(
            catalog,
            name.as_bytes(),
            true,
            NamedShaderLighting {
                source: qa_render::world::geometry::LightSource::TwoDimensional,
                lightmap: qa_render::ImageId(0),
            },
            qa_render::material::ShaderCompilePolicy::default(),
            &mut images,
        )
        .unwrap();
        assert_eq!(registration.defaulted, None);
        images
            .assets
            .shader_binding(registration.shader)
            .unwrap()
            .material
    };
    (assets, material)
}

fn parse(text: &[u8]) -> ShaderCatalog {
    parse_sources(&[ShaderSource {
        name: "scripts/native.shader",
        bytes: text,
    }])
}

#[test]
fn sun_angles_keep_native_division_and_double_trig_products() {
    // Captured from tr_shader.c:1460-1468 with its float stores, double M_PI
    // and double trig products (cc -O2 -ffp-contract=off, no fast-math).
    for (azimuth, elevation, expected) in [
        (0., 0., [0x3f80_0000, 0, 0]),
        (1., 0., [0x3f7f_f605, 0x3c8e_f859, 0]),
        (10., 0., [0x3f7c_1c5c, 0x3e31_d0d4, 0]),
        (45., 60., [0x3eb5_04f2, 0x3eb5_04f3, 0x3f5d_b3d8]),
        (-45., -60., [0x3eb5_04f2, 0xbeb5_04f3, 0xbf5d_b3d8]),
        (225., 17., [0xbf2d_1c13, 0xbf2d_1c11, 0x3e95_b1bf]),
        (0.1, -0.1, [0x3f7f_ffcd, 0x3ae4_c36a, 0xbae4_c380]),
        (359., 89., [0x3c8e_f2fa, 0xb99f_b07d, 0x3f7f_f605]),
    ] {
        let sun = SunParms {
            color: [1.; 3],
            intensity: 200.,
            azimuth_degrees: azimuth,
            elevation_degrees: elevation,
        };
        assert_eq!(
            sun.direction().0.map(f32::to_bits),
            expected,
            "native sun angles ({azimuth}, {elevation})"
        );
    }
}

#[test]
fn native_masked_lightmap_state_and_load_names() {
    let catalog = parse(
        br#"
        "Textures\Gothic/grate" {
            surfaceparm metalsteps
            cull none
            { map textures/gothic/grate.tga
              alphaFunc GE128
              rgbGen identity
              depthWrite
            }
            { map $lightmap
              blendFunc filter
              depthFunc equal
            }
        }
    "#,
    );
    assert!(catalog.diagnostics.is_empty());
    let shader = catalog.find_canonical("textures/gothic/grate").unwrap();
    assert!(shader.is_complete());
    assert_eq!(
        actions(shader),
        [
            ShaderAction::Header(HeaderOp::SurfaceParm {
                surface: 0x1000,
                contents: 0
            }),
            ShaderAction::Header(HeaderOp::Cull(Cull::None)),
            ShaderAction::BeginStage,
            ShaderAction::Stage(StageOp::Map(ShaderMap::Image {
                name: "textures/gothic/grate.tga".into(),
                clamp: false,
            })),
            ShaderAction::Stage(StageOp::AlphaFunc(AlphaFunc::AtLeastHalf)),
            ShaderAction::Stage(StageOp::RgbGen(RgbGen::Identity)),
            ShaderAction::Stage(StageOp::DepthWrite),
            ShaderAction::EndStage,
            ShaderAction::BeginStage,
            ShaderAction::Stage(StageOp::Map(ShaderMap::Lightmap)),
            ShaderAction::Stage(StageOp::BlendSource(BlendFactor::DestinationColor)),
            ShaderAction::Stage(StageOp::BlendDestination(BlendFactor::Zero)),
            ShaderAction::Stage(StageOp::BlendComplete),
            ShaderAction::Stage(StageOp::DepthFunc(DepthFunc::Equal)),
            ShaderAction::EndStage,
            ShaderAction::EndShader,
        ]
    );
}

#[test]
fn newlines_bound_animation_and_texmods_but_block_comments_do_not() {
    let catalog = parse(
        br#"
        effects/fire {
            { animMap 8 a.tga b.tga /* native compression removes
                  this internal newline */ c.tga d.tga e.tga f.tga g.tga h.tga i.tga
              blendFunc add
              tcMod transform 1 2 3 4 5 6
              tcMod stretch triangle .5 .25 0 2
              tcMod turb 0 .1 .2 .3
              tcMod entityTranslate
              rgbGen wave sin .8 .2 0 1
              alphaGen const .7
            }
        }
    "#,
    );
    let shader = &catalog.definitions[0];
    assert!(shader.is_complete());
    let ops = actions(shader);
    assert_eq!(ops[1], ShaderAction::Stage(StageOp::AnimationSpeed(8.)));
    let images: Vec<_> = shader
        .ops
        .iter()
        .filter_map(|op| match &op.action {
            ShaderAction::Stage(StageOp::AnimationFrame(name)) => Some(name.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(images.len(), MAX_ANIMATIONS);
    assert_eq!(images[7], "h.tga");
    let modifiers: Vec<_> = shader
        .ops
        .iter()
        .filter_map(|op| match &op.action {
            ShaderAction::Stage(StageOp::TexMod(value)) => Some(*value),
            _ => None,
        })
        .collect();
    assert_eq!(modifiers.len(), MAX_TEXMODS);
    assert_eq!(
        modifiers[0],
        Some(TexMod::Transform {
            matrix: [[1.0, 2.0], [3.0, 4.0]],
            translate: [5.0, 6.0]
        })
    );
    assert_eq!(modifiers[3], Some(TexMod::EntityTranslate));
    assert!(
        ops.contains(&ShaderAction::Stage(StageOp::RgbWave(WaveUpdate {
            function: Some(WaveFunction::Sin),
            values: [Some(0.8), Some(0.2), Some(0.), Some(1.)],
        })))
    );
    assert!(
        ops.contains(&ShaderAction::Stage(StageOp::AlphaGen(AlphaGen::Const(
            0.7
        ))))
    );
    assert!(catalog.diagnostics.iter().any(|diagnostic| {
        diagnostic.kind == DiagnosticKind::LimitExceeded(LimitKind::AnimationFrames)
            && diagnostic.severity == Severity::Warning
    }));
}

#[test]
fn global_native_data_and_unconverted_runtime_declarations_survive() {
    let catalog = parse(
        br#"
        textures/fog {
            qer_editorimage textures/editor/fog.tga
            surfaceparm fog
            surfaceparm nonsolid
            fogParms ( .1 .2 .3 ) 512
        }
        textures/sky {
            surfaceparm sky
            skyParms env/night 0 -
            q3map_sun 1 .5 .25 200 45 60
            polygonOffset
            noMipMaps
            entityMergable
            clampTime 3
            deformVertexes wave 100 sin 0 1 0 1
            { map $whiteimage }
        }
    "#,
    );
    let fog = catalog.find_canonical("textures/fog").unwrap();
    assert!(fog.is_complete());
    assert!(!fog.has_unsupported_runtime());
    assert_eq!(
        actions(fog)
            .into_iter()
            .filter(|op| !matches!(op, ShaderAction::Header(HeaderOp::Unsupported(_))))
            .collect::<Vec<_>>(),
        [
            ShaderAction::Header(HeaderOp::SurfaceParm {
                surface: 0,
                contents: 64
            }),
            ShaderAction::Header(HeaderOp::SurfaceParm {
                surface: 0x4000,
                contents: 0
            }),
            ShaderAction::Header(HeaderOp::FogColor([Some(0.1), Some(0.2), Some(0.3)])),
            ShaderAction::Header(HeaderOp::FogDepth(512.)),
            ShaderAction::EndShader,
        ]
    );
    let (assets, material) = compile(&catalog, "textures/fog");
    let material = assets.material(material).unwrap();
    assert_eq!(material.settings.sort, 7.);
    assert_eq!(material.settings.content_flags, 64);
    assert_eq!(material.settings.surface_flags, 0x4000);
    assert_eq!(
        material.settings.fog,
        Some(FogParms {
            color: [0.1, 0.2, 0.3],
            depth_opaque: 512.
        })
    );
    let sky = catalog.find_canonical("textures/sky").unwrap();
    assert!(sky.is_complete() && !sky.has_unsupported_runtime());
    let ops = actions(sky);
    assert!(ops.contains(&ShaderAction::Header(HeaderOp::SkyOuter(Some(
        "env/night".into()
    )))));
    assert!(ops.contains(&ShaderAction::Header(HeaderOp::SkyHeight(512.))));
    assert!(ops.contains(&ShaderAction::Header(HeaderOp::SkyInner(None))));
    assert!(ops.contains(&ShaderAction::Header(HeaderOp::Sun(SunParms {
        color: [1., 0.5, 0.25],
        intensity: 200.,
        azimuth_degrees: 45.,
        elevation_degrees: 60.,
    }))));
    assert!(ops.contains(&ShaderAction::Header(HeaderOp::NoMipmaps)));
    assert!(ops.contains(&ShaderAction::Header(HeaderOp::PolygonOffset)));
    assert!(ops.contains(&ShaderAction::Header(HeaderOp::EntityMergable)));
    assert!(ops.contains(&ShaderAction::Header(HeaderOp::ClampTime(3.))));
    assert_eq!(
        sky.ops
            .iter()
            .filter_map(|op| match &op.action {
                ShaderAction::Header(HeaderOp::Deform(value)) => Some(*value),
                _ => None,
            })
            .collect::<Vec<_>>(),
        [Some(Deform::Wave {
            spread: 0.01,
            wave: Waveform {
                function: WaveFunction::Sin,
                base: 0.0,
                amplitude: 1.0,
                phase: 0.0,
                frequency: 1.0
            },
        })]
    );
}

#[test]
fn native_stage_defaults_and_explicit_depth_write_order() {
    let catalog = parse(
        br#"
        native/defaults {
            { map $whiteimage blendFunc filter }
            { map $whiteimage depthWrite blendFunc blend }
            { map $whiteimage blendFunc GL_ONE GL_ZERO }
            { map $whiteimage alphaGen identity rgbGen vertex }
            { map $whiteimage rgbGen identity alphaGen entity }
            { map $whiteimage tcGen vector ( 1 0 0 ) ( 0 1 0 ) alphaFunc LT128 }
        }
    "#,
    );
    let shader = &catalog.definitions[0];
    assert!(shader.is_complete());
    let (assets, material) = compile(&catalog, "native/defaults");
    let material = assets.material(material).unwrap();
    assert_eq!(material.settings.sort, 9.0);
    assert_eq!(material.stages[0].rgb_gen, RgbGen::Identity);
    assert!(!material.stages[0].depth_write);
    assert!(material.stages[1].depth_write);
    assert!(material.stages[2].blend.is_none() && material.stages[2].depth_write);
    assert_eq!(material.stages[3].alpha_gen, AlphaGen::Vertex);
    // ParseStage's original cross-enum comparison: CGEN_IDENTITY == AGEN_ENTITY.
    assert_eq!(material.stages[4].alpha_gen, AlphaGen::Skip);
    assert_eq!(material.stages[5].alpha_test, AlphaFunc::LessThanHalf);
    assert_eq!(
        material.stages[5].texgen,
        TexCoordGen::Vector([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]])
    );
}

#[test]
fn sorted_catalog_retains_first_duplicate_and_reports_invalid_definitions() {
    let catalog = parse_sources(&[
        ShaderSource {
            name: "scripts/first.shader",
            bytes: br#"
            Z { { map first.tga } }
            textures/missing { { map
                rgbGen identity } }
            textures/unknown { extension mystery
                { map $whiteimage } }
        "#,
        },
        ShaderSource {
            name: "scripts/second.shader",
            bytes: br#"
            z { { map second.tga } }
            a { { map $whiteimage } }
        "#,
        },
    ]);
    assert_eq!(
        catalog
            .definitions
            .iter()
            .map(|definition| definition.name.as_str())
            .collect::<Vec<_>>(),
        ["a", "textures/missing", "textures/unknown", "z"]
    );
    assert_eq!(
        catalog.find_canonical("z").unwrap().source,
        "scripts/first.shader"
    );
    assert_eq!(
        actions(catalog.find_canonical("textures/missing").unwrap()),
        [
            ShaderAction::BeginStage,
            ShaderAction::Abort(ShaderFailure::Native)
        ]
    );
    let unknown = catalog.find_canonical("textures/unknown").unwrap();
    assert_eq!(
        actions(unknown),
        [ShaderAction::Abort(ShaderFailure::Native)]
    );
    assert!(catalog.diagnostics.iter().any(|diagnostic| {
        diagnostic.shader.as_deref() == Some("textures/unknown")
            && diagnostic.kind == DiagnosticKind::Unsupported("extension".into())
    }));
    assert_eq!(
        actions(catalog.find_canonical("z").unwrap()),
        [
            ShaderAction::BeginStage,
            ShaderAction::Stage(StageOp::Map(ShaderMap::Image {
                name: "first.tga".into(),
                clamp: false
            })),
            ShaderAction::EndStage,
            ShaderAction::EndShader,
        ]
    );
    assert!(
        catalog
            .diagnostics
            .iter()
            .any(|diagnostic| matches!(diagnostic.kind, DiagnosticKind::Duplicate { .. }))
    );
}

#[test]
fn bounded_native_stages_texmods_and_lexical_failures_are_visible() {
    let too_many_stages = format!("overflow {{ {} }}", "{ map $whiteimage } ".repeat(9));
    let catalog = parse(too_many_stages.as_bytes());
    assert!(!catalog.definitions[0].is_complete());
    assert_eq!(
        catalog.definitions[0]
            .ops
            .iter()
            .filter(|op| matches!(op.action, ShaderAction::BeginStage))
            .count(),
        MAX_STAGES
    );
    assert_eq!(
        catalog.definitions[0].ops.last().unwrap().action,
        ShaderAction::Abort(ShaderFailure::Limit(LimitKind::Stages))
    );
    assert!(
        catalog
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.kind == DiagnosticKind::LimitExceeded(LimitKind::Stages))
    );
    let catalog = parse(b"overflow { { map $whiteimage\n tcMod scale 1 1\n tcMod scale 1 1\n tcMod scale 1 1\n tcMod scale 1 1\n tcMod scale 1 1\n } }");
    assert!(!catalog.definitions[0].is_complete());
    assert_eq!(
        catalog.definitions[0]
            .ops
            .iter()
            .filter(|op| matches!(op.action, ShaderAction::Stage(StageOp::TexMod(_))))
            .count(),
        MAX_TEXMODS
    );
    assert_eq!(
        catalog.definitions[0].ops.last().unwrap().action,
        ShaderAction::Abort(ShaderFailure::Limit(LimitKind::TexMods))
    );
    for (source, definitions) in [
        (b"name { /*".as_slice(), 1),
        (b"name { { map \"unterminated".as_slice(), 1),
        (b"name\0ignored".as_slice(), 0),
    ] {
        let catalog = parse(source);
        assert_eq!(catalog.definitions.len(), definitions);
        if let Some(shader) = catalog.definitions.first() {
            assert!(!shader.is_complete());
            assert_eq!(
                shader.ops.last().unwrap().action,
                ShaderAction::Abort(ShaderFailure::Native)
            );
        }
        assert!(
            catalog
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.severity == Severity::Error)
        );
    }
}

#[test]
fn native_atof_accepts_retail_turb_and_numeric_prefixes_with_visible_fallbacks() {
    let catalog = parse(
        br#"
        native/atof {
            { map $whiteimage
              tcMod turb sin 0 1 0
              tcMod scale 1.5suffix -.25tail
              tcMod transform 0x1p+1 0 0 0X1p-1 0 0
              rgbGen const ( nan inf 1e999 )
              alphaGen const 1e+suffix
            }
        }
    "#,
    );
    let shader = &catalog.definitions[0];
    assert!(shader.is_complete());
    let modifiers: Vec<_> = shader
        .ops
        .iter()
        .filter_map(|op| match &op.action {
            ShaderAction::Stage(StageOp::TexMod(value)) => Some(*value),
            _ => None,
        })
        .collect();
    assert_eq!(
        modifiers[0],
        Some(TexMod::Turbulent {
            base: 0.0,
            amplitude: 0.0,
            phase: 1.0,
            frequency: 0.0
        })
    );
    assert_eq!(modifiers[1], Some(TexMod::Scale([1.5, -0.25])));
    assert_eq!(
        modifiers[2],
        Some(TexMod::Transform {
            matrix: [[2.0, 0.0], [0.0, 0.5]],
            translate: [0.0, 0.0]
        })
    );
    let ops = actions(shader);
    assert!(
        ops.contains(&ShaderAction::Stage(StageOp::RgbGen(RgbGen::Const(
            [0.; 3]
        ))))
    );
    assert!(ops.contains(&ShaderAction::Stage(StageOp::AlphaGen(AlphaGen::Const(1.)))));
    assert!(catalog.diagnostics.iter().any(|diagnostic| matches!(
        &diagnostic.kind, DiagnosticKind::NativeFallback { value, .. } if value == "sin"
    )));
    assert!(
        catalog
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.severity == Severity::Warning)
    );
    let incomplete_modifier = parse(b"missing { { map $whiteimage\n tcMod scale 1\n } }");
    let shader = &incomplete_modifier.definitions[0];
    assert!(shader.is_complete());
    assert!(actions(shader).contains(&ShaderAction::Stage(StageOp::TexMod(None))));
    assert!(incomplete_modifier.diagnostics.iter().any(|diagnostic| matches!(
        &diagnostic.kind, DiagnosticKind::MissingArgument(keyword) if keyword == "tcMod scale"
    )));
    let catalog = parse(b"missing { { map $whiteimage\n rgbGen const 1 2 3\n } }");
    assert!(!catalog.definitions[0].is_complete());
    assert!(
        catalog
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == Severity::Error)
    );
}

#[test]
fn native_typed_deforms_retain_normal_order_zero_spread_and_three_slot_limit() {
    let catalog = parse(
        br#"
        native/deforms {
            deformVertexes wave 0 triangle 1 2 3 4
            deformVertexes normal .4 2
            deformVertexes move 1 2 3 sin 4 5 6 7
            { map $whiteimage }
        }
        native/sprites {
            deformVertexes bulge 1 2 3
            deformVertexes autosprite
            deformVertexes autosprite2
            deformVertexes projectionShadow
            { map $whiteimage }
        }
        native/text {
            deformVertexes projectionShadow
            deformVertexes text7
            deformVertexes text9
            { map $whiteimage }
        }
    "#,
    );
    let deforms = catalog.find_canonical("native/deforms").unwrap();
    assert!(deforms.is_complete());
    let deforms: Vec<_> = deforms
        .ops
        .iter()
        .filter_map(|op| match &op.action {
            ShaderAction::Header(HeaderOp::Deform(value)) => Some(*value),
            _ => None,
        })
        .collect();
    assert_eq!(
        deforms[0],
        Some(Deform::Wave {
            spread: 100.0,
            wave: Waveform {
                function: WaveFunction::Triangle,
                base: 1.0,
                amplitude: 2.0,
                phase: 3.0,
                frequency: 4.0
            }
        })
    );
    assert_eq!(
        deforms[1],
        Some(Deform::Normal {
            amplitude: 0.4,
            frequency: 2.0
        })
    );
    assert_eq!(
        deforms[2],
        Some(Deform::Move {
            vector: [1.0, 2.0, 3.0],
            wave: Waveform {
                function: WaveFunction::Sin,
                base: 4.0,
                amplitude: 5.0,
                phase: 6.0,
                frequency: 7.0
            }
        })
    );
    let sprites = catalog.find_canonical("native/sprites").unwrap();
    assert!(sprites.is_complete());
    let sprites: Vec<_> = sprites
        .ops
        .iter()
        .filter_map(|op| match &op.action {
            ShaderAction::Header(HeaderOp::Deform(value)) => Some(*value),
            _ => None,
        })
        .collect();
    assert_eq!(sprites.len(), MAX_DEFORMS);
    assert_eq!(sprites[1], Some(Deform::AutoSprite));
    assert_eq!(sprites[2], Some(Deform::AutoSprite2));
    let text = catalog.find_canonical("native/text").unwrap();
    assert_eq!(
        text.ops
            .iter()
            .filter_map(|op| match &op.action {
                ShaderAction::Header(HeaderOp::Deform(value)) => Some(*value),
                _ => None,
            })
            .collect::<Vec<_>>(),
        [
            Some(Deform::ProjectionShadow),
            Some(Deform::Text(7)),
            Some(Deform::Text(0))
        ]
    );
    assert!(
        catalog
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.kind == DiagnosticKind::LimitExceeded(LimitKind::Deforms))
    );
}

#[test]
fn repeated_maps_and_late_mipmap_flags_keep_their_source_order() {
    let catalog = parse(
        br#"
        ordered {
            { map first.tga
              clampmap second.tga
              map $lightmap
              map $whiteimage
            }
            noMipMaps
            { map third.tga }
        }
    "#,
    );
    let shader = catalog.find_canonical("ordered").unwrap();
    assert_eq!(
        actions(shader),
        [
            ShaderAction::BeginStage,
            ShaderAction::Stage(StageOp::Map(ShaderMap::Image {
                name: "first.tga".into(),
                clamp: false
            })),
            ShaderAction::Stage(StageOp::Map(ShaderMap::Image {
                name: "second.tga".into(),
                clamp: true
            })),
            ShaderAction::Stage(StageOp::Map(ShaderMap::Lightmap)),
            ShaderAction::Stage(StageOp::Map(ShaderMap::White)),
            ShaderAction::EndStage,
            ShaderAction::Header(HeaderOp::NoMipmaps),
            ShaderAction::BeginStage,
            ShaderAction::Stage(StageOp::Map(ShaderMap::Image {
                name: "third.tga".into(),
                clamp: false
            })),
            ShaderAction::EndStage,
            ShaderAction::EndShader,
        ]
    );
}

#[test]
fn repeated_animation_directives_keep_frames_speed_and_later_map_writes() {
    let catalog = parse(
        br#"
        animations {
            { animMap 2 a.tga b.tga
              map replacement.tga
              animMap 4 c.tga d.tga e.tga f.tga g.tga h.tga i.tga
            }
        }
    "#,
    );
    let shader = catalog.find_canonical("animations").unwrap();
    assert_eq!(
        actions(shader),
        [
            ShaderAction::BeginStage,
            ShaderAction::Stage(StageOp::AnimationSpeed(2.)),
            ShaderAction::Stage(StageOp::AnimationFrame("a.tga".into())),
            ShaderAction::Stage(StageOp::AnimationFrame("b.tga".into())),
            ShaderAction::Stage(StageOp::Map(ShaderMap::Image {
                name: "replacement.tga".into(),
                clamp: false
            })),
            ShaderAction::Stage(StageOp::AnimationSpeed(4.)),
            ShaderAction::Stage(StageOp::AnimationFrame("c.tga".into())),
            ShaderAction::Stage(StageOp::AnimationFrame("d.tga".into())),
            ShaderAction::Stage(StageOp::AnimationFrame("e.tga".into())),
            ShaderAction::Stage(StageOp::AnimationFrame("f.tga".into())),
            ShaderAction::Stage(StageOp::AnimationFrame("g.tga".into())),
            ShaderAction::Stage(StageOp::AnimationFrame("h.tga".into())),
            ShaderAction::EndStage,
            ShaderAction::EndShader,
        ]
    );
    assert!(
        catalog.diagnostics.iter().any(|diagnostic| diagnostic.kind
            == DiagnosticKind::LimitExceeded(LimitKind::AnimationFrames))
    );
}

#[test]
fn failed_stage_keeps_prefix_without_an_end_stage_and_recovers_next_definition() {
    let catalog = parse(
        br#"
        broken {
            { map $whiteimage
              blendFunc add
              rgbGen vertex
              bogus
              tcGen environment
            }
            cull none
        }
        after { { map $whiteimage } }
    "#,
    );
    assert_eq!(
        actions(catalog.find_canonical("broken").unwrap()),
        [
            ShaderAction::BeginStage,
            ShaderAction::Stage(StageOp::Map(ShaderMap::White)),
            ShaderAction::Stage(StageOp::BlendSource(BlendFactor::One)),
            ShaderAction::Stage(StageOp::BlendDestination(BlendFactor::One)),
            ShaderAction::Stage(StageOp::BlendComplete),
            ShaderAction::Stage(StageOp::RgbGen(RgbGen::Vertex)),
            ShaderAction::Abort(ShaderFailure::Native),
        ]
    );
    assert_eq!(
        actions(catalog.find_canonical("after").unwrap()),
        [
            ShaderAction::BeginStage,
            ShaderAction::Stage(StageOp::Map(ShaderMap::White)),
            ShaderAction::EndStage,
            ShaderAction::EndShader,
        ]
    );
}

#[test]
fn header_abort_keeps_completed_stage_and_reached_sun_without_later_headers() {
    let catalog = parse(
        br#"
        broken {
            { map $whiteimage }
            q3map_sun 1 0 0 100 90 0
            bogus
            q3map_sun 0 1 0 50 0 0
            cull none
            sort additive
        }
        after { cull back }
    "#,
    );
    assert_eq!(
        actions(catalog.find_canonical("broken").unwrap()),
        [
            ShaderAction::BeginStage,
            ShaderAction::Stage(StageOp::Map(ShaderMap::White)),
            ShaderAction::EndStage,
            ShaderAction::Header(HeaderOp::Sun(SunParms {
                color: [1., 0., 0.],
                intensity: 100.,
                azimuth_degrees: 90.,
                elevation_degrees: 0.,
            })),
            ShaderAction::Abort(ShaderFailure::Native),
        ]
    );
    // Syntax completion is separate from the compiler's native no-stage check.
    assert_eq!(
        actions(catalog.find_canonical("after").unwrap()),
        [
            ShaderAction::Header(HeaderOp::Cull(Cull::Back)),
            ShaderAction::EndShader,
        ]
    );
}

#[test]
fn partial_wave_and_vector_writes_survive_recoverable_operand_failures() {
    let catalog = parse(
        br#"
        partial {
            { map $whiteimage
              rgbGen wave triangle .5
              alphaGen wave sin .25 .75
              tcGen vector ( 1 2 3 ) ( 4 5
              tcGen base
            }
        }
    "#,
    );
    let shader = catalog.find_canonical("partial").unwrap();
    assert_eq!(
        actions(shader),
        [
            ShaderAction::BeginStage,
            ShaderAction::Stage(StageOp::Map(ShaderMap::White)),
            ShaderAction::Stage(StageOp::RgbWave(WaveUpdate {
                function: Some(WaveFunction::Triangle),
                values: [Some(0.5), None, None, None],
            })),
            ShaderAction::Stage(StageOp::AlphaWave(WaveUpdate {
                function: Some(WaveFunction::Sin),
                values: [Some(0.25), Some(0.75), None, None],
            })),
            ShaderAction::Stage(StageOp::TcVector([
                [Some(1.), Some(2.), Some(3.)],
                [Some(4.), Some(5.), None]
            ])),
            ShaderAction::Stage(StageOp::TcGen(TexCoordGen::Texture)),
            ShaderAction::EndStage,
            ShaderAction::EndShader,
        ]
    );
    assert!(
        catalog
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == Severity::Error)
    );
}

#[test]
fn vector_operand_newline_is_consumed_before_the_second_vector_read() {
    let catalog = parse(b"s { { tcGen vector\n ( 1 0 0 ) map A } }");
    assert_eq!(
        actions(catalog.find_canonical("s").unwrap()),
        [
            ShaderAction::BeginStage,
            ShaderAction::Stage(StageOp::TcVector([
                [None; 3],
                [Some(1.), Some(0.), Some(0.)],
            ])),
            ShaderAction::Stage(StageOp::Map(ShaderMap::Image {
                name: "a".into(),
                clamp: false
            })),
            ShaderAction::EndStage,
            ShaderAction::EndShader,
        ]
    );
    assert!(catalog.diagnostics.iter().any(|diagnostic| matches!(
        &diagnostic.kind, DiagnosticKind::MissingArgument(keyword) if keyword == "tcgen"
    )));
}

#[test]
fn lexical_suffix_failures_preserve_complete_and_partial_definition_prefixes() {
    for (suffix, kind) in [
        (
            b"/* unterminated".as_slice(),
            DiagnosticKind::UnterminatedComment,
        ),
        (
            b"\"unterminated".as_slice(),
            DiagnosticKind::UnterminatedQuote,
        ),
        (b"\0ignored".as_slice(), DiagnosticKind::EmbeddedNul),
        (b"\xff".as_slice(), DiagnosticKind::InvalidUtf8),
    ] {
        let mut source = b"good { { map $whiteimage } }\nbad { { map $whiteimage } ".to_vec();
        source.extend_from_slice(suffix);
        let catalog = parse(&source);
        assert_eq!(
            actions(catalog.find_canonical("good").unwrap()),
            [
                ShaderAction::BeginStage,
                ShaderAction::Stage(StageOp::Map(ShaderMap::White)),
                ShaderAction::EndStage,
                ShaderAction::EndShader,
            ]
        );
        assert_eq!(
            actions(catalog.find_canonical("bad").unwrap()),
            [
                ShaderAction::BeginStage,
                ShaderAction::Stage(StageOp::Map(ShaderMap::White)),
                ShaderAction::EndStage,
                ShaderAction::Abort(ShaderFailure::Native),
            ]
        );
        assert!(
            catalog
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.kind == kind)
        );
    }
}

#[test]
fn initialized_const_vector_survives_wrong_or_missing_closing_delimiter() {
    for text in [
        b"s { { rgbGen const ( 1 0 0 wrong\n map A\n } }".as_slice(),
        b"s { { rgbGen const ( 1 0 0\n map A\n } }".as_slice(),
    ] {
        let catalog = parse(text);
        assert_eq!(
            actions(catalog.find_canonical("s").unwrap()),
            [
                ShaderAction::BeginStage,
                ShaderAction::Stage(StageOp::RgbGen(RgbGen::Const([1., 0., 0.]))),
                ShaderAction::Stage(StageOp::Map(ShaderMap::Image {
                    name: "a".into(),
                    clamp: false
                })),
                ShaderAction::EndStage,
                ShaderAction::EndShader,
            ]
        );
        assert!(catalog.diagnostics.iter().any(|diagnostic| matches!(
            diagnostic.kind,
            DiagnosticKind::Expected { expected: ")", .. }
        )));
    }
}

#[test]
fn unterminated_quoted_map_retains_its_operand_before_eof_abort() {
    let catalog = parse(b"s { { map \"A");
    assert_eq!(
        actions(catalog.find_canonical("s").unwrap()),
        [
            ShaderAction::BeginStage,
            ShaderAction::Stage(StageOp::Map(ShaderMap::Image {
                name: "a".into(),
                clamp: false
            })),
            ShaderAction::Abort(ShaderFailure::Native),
        ]
    );
    assert!(
        catalog
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.kind == DiagnosticKind::UnterminatedQuote)
    );
}

#[test]
fn aborted_definition_recovery_skips_empty_tokens_without_admitting_nested_names() {
    let catalog = parse(
        br#"
        bad { bogus
              ""
              ghost { { map textures/modern.tga } }
        }
        good { { map $whiteimage } }
    "#,
    );
    assert_eq!(
        catalog
            .definitions
            .iter()
            .map(|definition| definition.name.as_str())
            .collect::<Vec<_>>(),
        ["bad", "good"]
    );
    assert!(!catalog.find_canonical("bad").unwrap().is_complete());
    assert!(catalog.find_canonical("good").unwrap().is_complete());
}
