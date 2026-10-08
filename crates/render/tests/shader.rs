use qa_render::shader::*;

fn parse(text: &[u8]) -> ShaderCatalog {
    parse_sources(&[ShaderSource {
        name: "scripts/native.shader",
        bytes: text,
    }])
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
    assert!(shader.valid);
    assert_eq!(shader.cull, Cull::None);
    assert_eq!(shader.surface_flags, 0x1000);
    assert_eq!(shader.sort, 3.0);
    assert_eq!(shader.stages[0].alpha_func, AlphaFunc::AtLeastHalf);
    assert!(shader.stages[0].depth_write);
    assert_eq!(shader.stages[1].tc_gen, TexCoordGen::Lightmap);
    assert_eq!(shader.stages[1].depth_func, DepthFunc::Equal);
    assert!(!shader.stages[1].depth_write);
    assert_eq!(
        shader.stages[1].blend,
        Some(StageBlend {
            source: BlendFactor::DestinationColor,
            destination: BlendFactor::Zero,
        })
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
    assert!(shader.valid);
    let stage = &shader.stages[0];
    let Some(TextureMap::Animation { frequency, images }) = &stage.map else {
        panic!()
    };
    assert_eq!(*frequency, 8.0);
    assert_eq!(images.len(), MAX_ANIMATIONS);
    assert_eq!(images[7], "h.tga");
    assert_eq!(stage.tc_mods.len(), MAX_TEXMODS);
    assert_eq!(
        stage.tc_mods[0],
        TexMod::Transform {
            matrix: [[1.0, 2.0], [3.0, 4.0]],
            translate: [5.0, 6.0]
        }
    );
    assert_eq!(stage.tc_mods[3], TexMod::EntityTranslate);
    assert_eq!(stage.alpha_gen, AlphaGen::Const(0.7));
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
    assert!(fog.valid);
    assert_eq!(fog.content_flags, 64);
    assert_eq!(fog.surface_flags, 0x4000);
    assert_eq!(fog.sort, 7.0);
    assert!(!fog.has_unsupported_runtime());
    assert_eq!(
        fog.fog,
        Some(FogParms {
            color: [0.1, 0.2, 0.3],
            depth_opaque: 512.0
        })
    );
    let sky = catalog.find_canonical("textures/sky").unwrap();
    assert!(sky.valid && !sky.has_unsupported_runtime());
    assert_eq!(sky.sort, 2.0);
    assert_eq!(sky.sky.as_ref().unwrap().cloud_height, 512.0);
    assert!(sky.no_mipmaps && sky.no_picmip && sky.polygon_offset && sky.entity_mergable);
    assert_eq!(sky.clamp_time, Some(3.0));
    assert_eq!(
        sky.deforms.as_ref(),
        &[Deform::Wave {
            spread: 0.01,
            wave: Waveform {
                function: WaveFunction::Sin,
                base: 0.0,
                amplitude: 1.0,
                phase: 0.0,
                frequency: 1.0
            },
        }][..]
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
    assert!(shader.valid);
    assert_eq!(shader.sort, 9.0);
    assert_eq!(shader.stages[0].rgb_gen, RgbGen::Identity);
    assert!(!shader.stages[0].depth_write);
    assert!(shader.stages[1].depth_write);
    assert!(shader.stages[2].blend.is_none() && shader.stages[2].depth_write);
    assert_eq!(shader.stages[3].alpha_gen, AlphaGen::Vertex);
    // ParseStage's original cross-enum comparison: CGEN_IDENTITY == AGEN_ENTITY.
    assert_eq!(shader.stages[4].alpha_gen, AlphaGen::Skip);
    assert_eq!(shader.stages[5].alpha_func, AlphaFunc::LessThanHalf);
    assert_eq!(
        shader.stages[5].tc_gen,
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
    assert!(!catalog.find_canonical("textures/missing").unwrap().valid);
    let unknown = catalog.find_canonical("textures/unknown").unwrap();
    assert!(!unknown.valid);
    assert_eq!(unknown.unsupported[0].arguments.as_ref(), ["mystery"]);
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
    assert!(!catalog.definitions[0].valid);
    assert_eq!(catalog.definitions[0].stages.len(), MAX_STAGES);
    assert!(
        catalog
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.kind == DiagnosticKind::LimitExceeded(LimitKind::Stages))
    );
    let catalog = parse(b"overflow { { map $whiteimage\n tcMod scale 1 1\n tcMod scale 1 1\n tcMod scale 1 1\n tcMod scale 1 1\n tcMod scale 1 1\n } }");
    assert!(!catalog.definitions[0].valid);
    assert_eq!(catalog.definitions[0].stages[0].tc_mods.len(), MAX_TEXMODS);
    for source in [
        b"name { /*".as_slice(),
        b"name { { map \"unterminated".as_slice(),
        b"name\0ignored".as_slice(),
    ] {
        let catalog = parse(source);
        assert!(catalog.definitions.is_empty());
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
    assert!(shader.valid);
    let stage = &shader.stages[0];
    assert_eq!(
        stage.tc_mods[0],
        TexMod::Turbulent {
            base: 0.0,
            amplitude: 0.0,
            phase: 1.0,
            frequency: 0.0
        }
    );
    assert_eq!(stage.tc_mods[1], TexMod::Scale([1.5, -0.25]));
    assert_eq!(
        stage.tc_mods[2],
        TexMod::Transform {
            matrix: [[2.0, 0.0], [0.0, 0.5]],
            translate: [0.0, 0.0]
        }
    );
    assert_eq!(stage.rgb_gen, RgbGen::Const([0.0; 3]));
    assert_eq!(stage.alpha_gen, AlphaGen::Const(1.0));
    assert!(catalog.diagnostics.iter().any(|diagnostic| matches!(
        &diagnostic.kind, DiagnosticKind::NativeFallback { value, .. } if value == "sin"
    )));
    assert!(
        catalog
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.severity == Severity::Warning)
    );
    for source in [
        b"missing { { map $whiteimage\n tcMod scale 1\n } }".as_slice(),
        b"missing { { map $whiteimage\n rgbGen const 1 2 3\n } }".as_slice(),
        b"missing { { map $whiteimage\n rgbGen const ( 1 2 3\n } }".as_slice(),
    ] {
        assert!(!parse(source).definitions[0].valid);
    }
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
    assert!(deforms.valid);
    assert_eq!(
        deforms.deforms[0],
        Deform::Wave {
            spread: 100.0,
            wave: Waveform {
                function: WaveFunction::Triangle,
                base: 1.0,
                amplitude: 2.0,
                phase: 3.0,
                frequency: 4.0
            }
        }
    );
    assert_eq!(
        deforms.deforms[1],
        Deform::Normal {
            amplitude: 0.4,
            frequency: 2.0
        }
    );
    assert_eq!(
        deforms.deforms[2],
        Deform::Move {
            vector: [1.0, 2.0, 3.0],
            wave: Waveform {
                function: WaveFunction::Sin,
                base: 4.0,
                amplitude: 5.0,
                phase: 6.0,
                frequency: 7.0
            }
        }
    );
    let sprites = catalog.find_canonical("native/sprites").unwrap();
    assert!(sprites.valid);
    assert_eq!(sprites.deforms.len(), MAX_DEFORMS);
    assert_eq!(sprites.deforms[1], Deform::AutoSprite);
    assert_eq!(sprites.deforms[2], Deform::AutoSprite2);
    let text = catalog.find_canonical("native/text").unwrap();
    assert_eq!(
        text.deforms.as_ref(),
        [Deform::ProjectionShadow, Deform::Text(7), Deform::Text(0)]
    );
    assert!(
        catalog
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.kind == DiagnosticKind::LimitExceeded(LimitKind::Deforms))
    );
}
