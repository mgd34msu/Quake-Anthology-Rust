use super::{mount, resources};
use qa_core::primitives::RuleSetId;
use qa_render::{
    Assets, ShaderRegistration,
    assets::{ImageId, StageTexture, TcGen, Wrap},
    material::{
        MaterialFinishPolicy, ShaderCompilePolicy,
        resources::{ImageSettings, Images},
        world_load::{NamedShaderLighting, compile_named_shader},
    },
    shader::{Cull, RgbGen, ShaderSource, parse_sources},
    world::geometry::LightSource,
};

struct Compiled {
    assets: Assets,
    rows: Vec<ShaderRegistration>,
    image_first: usize,
    conflicts: usize,
}
fn compile(
    label: &str,
    source: &[u8],
    names: &[&[u8]],
    lighting: LightSource,
    settings: ImageSettings,
    policy: ShaderCompilePolicy,
) -> Compiled {
    let root = resources(label).unwrap();
    let mut second = std::fs::read(root.join("textures/modern.tga")).unwrap();
    for pixel in second[18..].chunks_exact_mut(3) {
        pixel.copy_from_slice(&[11, 29, 71]);
    }
    std::fs::write(root.join("textures/second.tga"), second).unwrap();
    for suffix in ["rt", "bk", "lf", "ft", "up", "dn"] {
        std::fs::copy(
            root.join("textures/modern.tga"),
            root.join(format!("textures/sky_{suffix}.tga")),
        )
        .unwrap();
    }
    let vfs = mount(&root).unwrap();
    let catalog = parse_sources(&[ShaderSource {
        name: "ordered.shader",
        bytes: source,
    }]);
    let mut assets = Assets::load().unwrap();
    let image_first = assets.images().len();
    let mut images = Images::new(
        &vfs,
        &mut assets,
        RuleSetId::Quake3,
        RuleSetId::Quake3,
        settings,
        None,
    )
    .unwrap();
    let rows = names
        .iter()
        .map(|name| {
            compile_named_shader(
                &catalog,
                name,
                true,
                NamedShaderLighting {
                    source: lighting,
                    lightmap: ImageId(0),
                },
                policy,
                &mut images,
            )
            .unwrap()
        })
        .collect();
    let conflicts = images.conflicts().len();
    drop(images);
    std::fs::remove_dir_all(root).unwrap();
    Compiled {
        assets,
        rows,
        image_first,
        conflicts,
    }
}
fn material(compiled: &Compiled, row: usize) -> &qa_render::assets::Material {
    let binding = compiled
        .assets
        .shader_binding(compiled.rows[row].shader)
        .unwrap();
    compiled.assets.material(binding.material).unwrap()
}
fn native() -> ImageSettings {
    ImageSettings::native(RuleSetId::Quake3)
}

#[test]
fn detail_removal_shifts_one_slot_and_does_not_refinish_or_filter_the_tail() {
    let result = compile(
        "finish-detail-slots",
        br#"
        shifted {
            { map textures/modern.tga
              detail
              tcGen texture
            }
            { map $lightmap
              map textures/second.tga
              tcGen environment
            }
            { map $whiteimage
              tcGen texture
            }
        }
        last {
            { map $whiteimage tcGen texture }
            { map $whiteimage tcGen texture }
            { map $whiteimage tcGen texture }
            { map $whiteimage tcGen texture }
            { map $whiteimage tcGen texture }
            { map $whiteimage tcGen texture }
            { map $whiteimage tcGen texture }
            { map $whiteimage tcGen texture detail }
        }
        "#,
        &[b"shifted", b"last"],
        LightSource::Page(0),
        native(),
        ShaderCompilePolicy {
            finish: MaterialFinishPolicy {
                detail_textures: false,
                ..Default::default()
            },
            ..Default::default()
        },
    );
    assert!(result.rows.iter().all(|row| row.defaulted.is_none()));
    let shifted = material(&result, 0);
    assert_eq!(shifted.stages.len(), 1);
    assert_eq!(
        shifted.stages[0].texture,
        StageTexture::Image(ImageId((result.image_first + 1) as u32))
    );
    assert_eq!(shifted.stages[0].texgen, TcGen::Environment);
    assert!(shifted.stages[0].is_lightmap);
    assert!(!shifted.finish_has_lightmap());
    assert_eq!(result.rows[0].finished, LightSource::None);
    let last = material(&result, 1);
    assert_eq!(last.stages.len(), 8);
    assert!(last.stages[7].detail);
}

#[test]
fn opaque_vertex_collapse_copies_only_the_winning_bundle_and_retains_slot_zero_state() {
    let source = br#"
        copied { sort opaque
            { map textures/modern.tga
              tcGen environment
              tcMod scroll 1 2
              rgbGen const ( 0.25 0.5 0.75 )
              alphaGen const 0.25
              alphaFunc GE128
              depthFunc equal
              blendFunc blend
            }
            { clampmap textures/second.tga
              tcGen texture
              rgbGen identity
              alphaFunc GT0
              depthFunc lequal
            }
        }
        sticky { sort opaque
            { map $lightmap
              map textures/modern.tga
              tcGen environment
              rgbGen vertex
            }
            { map $lightmap
              clampmap textures/second.tga
              tcGen texture
              rgbGen identity
            }
        }
    "#;
    for lighting in [LightSource::Page(0), LightSource::None] {
        let result = compile(
            "finish-opaque-bundle",
            source,
            &[b"copied", b"sticky"],
            lighting,
            native(),
            ShaderCompilePolicy {
                finish: MaterialFinishPolicy {
                    vertex_light: true,
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        assert!(result.rows.iter().all(|row| row.defaulted.is_none()));
        let copied = material(&result, 0);
        assert_eq!(copied.settings.sort, 3.0);
        assert_eq!(copied.stages.len(), 1);
        let stage = copied.stages[0];
        assert_eq!(
            stage.texture,
            StageTexture::Image(ImageId((result.image_first + 1) as u32))
        );
        assert_eq!(stage.sampler.wrap, Wrap::Clamp);
        assert_eq!(stage.texgen, TcGen::Texture);
        assert!(stage.tcmods.iter().all(Option::is_none));
        assert_eq!(stage.alpha_test, qa_render::shader::AlphaFunc::AtLeastHalf);
        assert_eq!(stage.depth_func, qa_render::assets::DepthFunc::Equal);
        assert!(stage.depth_write);
        assert!(stage.blend.is_none());
        assert_eq!(stage.alpha_gen, qa_render::shader::AlphaGen::Skip);
        assert_eq!(
            stage.rgb_gen,
            if lighting == LightSource::None {
                RgbGen::LightingDiffuse
            } else {
                RgbGen::ExactVertex
            }
        );
        let sticky = material(&result, 1);
        assert_eq!(sticky.stages.len(), 1);
        assert_eq!(sticky.stages[0].texture, stage.texture);
        assert!(sticky.stages[0].is_lightmap);
        assert!(!sticky.finish_has_lightmap());
        assert_eq!(result.rows[1].finished, LightSource::None);
    }
}

#[test]
fn declared_noop_texmod_still_penalizes_the_first_bundle_in_vertex_ranking() {
    let result = compile(
        "finish-noop-rank",
        br#"
        ranked { sort opaque
            { map textures/modern.tga
              tcGen texture
              rgbGen identity
              tcMod bogus
            }
            { map textures/second.tga
              tcGen texture
              rgbGen identity
            }
        }
        "#,
        &[b"ranked"],
        LightSource::Page(0),
        native(),
        ShaderCompilePolicy {
            finish: MaterialFinishPolicy {
                vertex_light: true,
                ..Default::default()
            },
            ..Default::default()
        },
    );
    assert!(result.rows[0].defaulted.is_none());
    let stages = &material(&result, 0).stages;
    assert_eq!(stages.len(), 1);
    assert_eq!(
        stages[0].texture,
        StageTexture::Image(ImageId((result.image_first + 1) as u32))
    );
    assert!(stages[0].tcmods.iter().all(Option::is_none));
    assert_eq!(stages[0].rgb_gen, RgbGen::ExactVertex);
}

#[test]
fn nonopaque_vertex_collapse_keeps_the_first_pass_and_native_crossfade_rules() {
    let result = compile(
        "finish-nonopaque",
        br#"
        entity { sort additive
            { map textures/modern.tga rgbGen oneMinusEntity blendFunc add }
            { map textures/second.tga rgbGen entity blendFunc add }
        }
        saw { sort additive
            { map textures/modern.tga rgbGen wave sawtooth 0 1 0 1 blendFunc add }
            { map textures/second.tga rgbGen wave inverseSawtooth 0 1 0 1 blendFunc add }
        }
        inverse { sort additive
            { map textures/modern.tga rgbGen wave inverseSawtooth 0 1 0 1 blendFunc add }
            { map textures/second.tga rgbGen wave sawtooth 0 1 0 1 blendFunc add }
        }
        skiplightmap { sort additive
            { map $lightmap tcGen lightmap }
            { map textures/second.tga
              tcGen texture
              rgbGen entity
              alphaGen const 0.375
              alphaFunc GE128
              depthFunc equal
              blendFunc blend
              depthWrite
            }
        }
        "#,
        &[b"entity", b"saw", b"inverse", b"skiplightmap"],
        LightSource::Page(0),
        native(),
        ShaderCompilePolicy {
            finish: MaterialFinishPolicy {
                vertex_light: true,
                ..Default::default()
            },
            ..Default::default()
        },
    );
    assert!(result.rows.iter().all(|row| row.defaulted.is_none()));
    for row in 0..3 {
        let body = material(&result, row);
        assert_eq!(body.settings.sort, 10.0);
        assert_eq!(body.stages.len(), 1);
        assert_eq!(
            body.stages[0].texture,
            StageTexture::Image(ImageId(result.image_first as u32))
        );
        assert_eq!(body.stages[0].rgb_gen, RgbGen::IdentityLighting);
        assert_eq!(
            body.stages[0].blend,
            Some(qa_render::shader::StageBlend {
                source: qa_render::shader::BlendFactor::One,
                destination: qa_render::shader::BlendFactor::One,
            })
        );
        assert!(!body.stages[0].depth_write);
    }
    let body = material(&result, 3);
    assert_eq!(body.stages.len(), 1);
    let stage = body.stages[0];
    assert_eq!(
        stage.texture,
        StageTexture::Image(ImageId((result.image_first + 1) as u32))
    );
    assert_eq!(stage.rgb_gen, RgbGen::Entity);
    assert_eq!(stage.alpha_gen, qa_render::shader::AlphaGen::Const(0.375));
    assert_eq!(stage.alpha_test, qa_render::shader::AlphaFunc::AtLeastHalf);
    assert_eq!(stage.depth_func, qa_render::assets::DepthFunc::Equal);
    assert!(stage.depth_write);
    assert_eq!(stage.blend, Some(qa_render::shader::StageBlend::ALPHA));
    assert_eq!(result.rows[3].finished, LightSource::None);
}

#[test]
fn implicit_lightmapped_material_uses_the_same_vertex_finish_and_fullscreen_guard() {
    for (vertex_light, ui_fullscreen, passes, finished) in [
        (false, false, 2, LightSource::Page(0)),
        (true, false, 1, LightSource::None),
        (true, true, 2, LightSource::Page(0)),
    ] {
        let result = compile(
            "finish-implicit",
            b"",
            &[b"textures/modern.tga"],
            LightSource::Page(0),
            native(),
            ShaderCompilePolicy {
                finish: MaterialFinishPolicy {
                    vertex_light,
                    ..Default::default()
                },
                ui_fullscreen,
            },
        );
        assert!(result.rows[0].defaulted.is_none());
        assert_eq!(result.rows[0].requested, LightSource::Page(0));
        assert_eq!(result.rows[0].finished, finished);
        let body = material(&result, 0);
        assert_eq!(body.stages.len(), passes);
        assert_eq!(body.finish_has_lightmap(), passes == 2);
        if passes == 1 {
            assert_eq!(
                body.stages[0].texture,
                StageTexture::Image(ImageId(result.image_first as u32))
            );
            assert_eq!(body.stages[0].rgb_gen, RgbGen::ExactVertex);
            assert_eq!(body.stages[0].alpha_gen, qa_render::shader::AlphaGen::Skip);
        }
    }
    for (lighting, rgb) in [
        (LightSource::None, RgbGen::LightingDiffuse),
        (LightSource::TwoDimensional, RgbGen::Vertex),
        (LightSource::Vertex, RgbGen::ExactVertex),
        (LightSource::White, RgbGen::ExactVertex),
    ] {
        let result = compile(
            "finish-implicit-sentinels",
            b"",
            &[b"textures/modern.tga", b"textures/not-present.tga"],
            lighting,
            native(),
            ShaderCompilePolicy {
                finish: MaterialFinishPolicy {
                    vertex_light: true,
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        assert!(result.rows[0].defaulted.is_none());
        assert_eq!(result.rows[0].requested, lighting);
        assert_eq!(result.rows[0].finished, lighting);
        let body = material(&result, 0);
        assert_eq!(body.stages.len(), 1);
        assert_eq!(body.stages[0].rgb_gen, rgb);
        assert!(!body.finish_has_lightmap());
        assert!(result.rows[1].defaulted.is_some());
        assert_eq!(result.rows[1].requested, lighting);
        assert_eq!(result.rows[1].finished, lighting);
        assert!(material(&result, 1).stages.is_empty());
        assert_eq!(material(&result, 1).settings.sort, 7.0);
    }
}

#[test]
fn repeated_maps_load_in_order_and_preserve_lightmap_classification() {
    let result = compile(
        "recipe-repeat",
        br#"
        repeated { { map textures/modern.tga
                     clampmap textures/second.tga
        } }
    "#,
        &[b"repeated"],
        LightSource::Page(0),
        native(),
        ShaderCompilePolicy::default(),
    );
    assert!(result.rows[0].defaulted.is_none());
    assert_eq!(result.rows[0].finished, LightSource::None);
    assert_eq!(result.assets.images().len(), result.image_first + 2);
    let stage = &material(&result, 0).stages[0];
    assert_eq!(
        stage.texture,
        StageTexture::Image(ImageId((result.image_first + 1) as u32))
    );
    assert_eq!(stage.sampler.wrap, Wrap::Clamp);
    assert_ne!(
        result.assets.images()[result.image_first].rgba,
        result.assets.images()[result.image_first + 1].rgba
    );

    let sticky = compile(
        "recipe-sticky-lightmap",
        br#"
        sticky { { map $lightmap
                   map textures/modern.tga
        } }
    "#,
        &[b"sticky"],
        LightSource::Page(0),
        native(),
        ShaderCompilePolicy::default(),
    );
    assert_eq!(sticky.rows[0].finished, LightSource::Page(0));
    let stage = &material(&sticky, 0).stages[0];
    assert!(stage.is_lightmap);
    assert_eq!(stage.texgen, TcGen::Lightmap);
    assert_eq!(
        stage.texture,
        StageTexture::Image(ImageId(sticky.image_first as u32))
    );
}

#[test]
fn mip_flags_only_affect_later_image_requests_and_keep_first_registration() {
    let mut settings = native();
    settings.picmip = 1;
    let distinct = compile(
        "recipe-mip-order",
        br#"
        ordered { { map textures/modern.tga
                  }
                  noMipMaps
                  { map textures/second.tga
                  }
        }
    "#,
        &[b"ordered"],
        LightSource::None,
        settings,
        ShaderCompilePolicy::default(),
    );
    assert!(distinct.rows[0].defaulted.is_none());
    let first = &distinct.assets.images()[distinct.image_first];
    let second = &distinct.assets.images()[distinct.image_first + 1];
    assert!(first.native_sampler.unwrap().mipmaps);
    assert!(!second.native_sampler.unwrap().mipmaps);
    assert_eq!(first.prepared.as_ref().unwrap().levels[0].width, 8);
    assert_eq!(second.prepared.as_ref().unwrap().levels[0].width, 16);
    assert!(first.prepared.as_ref().unwrap().levels.len() > 1);
    assert_eq!(second.prepared.as_ref().unwrap().levels.len(), 1);
    let reused = compile(
        "recipe-mip-first",
        br#"
        ordered { { map textures/modern.tga
                  }
                  noMipMaps
                  { map textures/modern.tga
                  }
        }
    "#,
        &[b"ordered"],
        LightSource::None,
        settings,
        ShaderCompilePolicy::default(),
    );
    assert_eq!(reused.assets.images().len(), reused.image_first + 1);
    assert_eq!(reused.conflicts, 1);
    assert!(
        reused.assets.images()[reused.image_first]
            .native_sampler
            .unwrap()
            .mipmaps
    );
}

#[test]
fn header_abort_keeps_committed_stage_without_loading_or_applying_later_directives() {
    let result = compile(
        "recipe-header-abort",
        br#"
        failed { { map textures/modern.tga
                 }
                 bogus
                 cull none
                 sort additive
                 { map textures/second.tga
                 }
        }
    "#,
        &[b"failed"],
        LightSource::None,
        native(),
        ShaderCompilePolicy::default(),
    );
    assert!(result.rows[0].defaulted.is_some());
    assert_eq!(result.assets.images().len(), result.image_first + 1);
    let body = material(&result, 0);
    assert_eq!(body.stages.len(), 1);
    assert_eq!(body.settings.cull, Cull::Front);
    assert_eq!(body.settings.sort, 3.0);
    assert!(body.stages[0].depth_write);
}

#[test]
fn stage_abort_retains_immediate_writes_without_committing_pipeline_state() {
    let result = compile(
        "recipe-stage-abort",
        br#"
        failed { { map textures/modern.tga
                   blendFunc add
                   rgbGen vertex
                   bogus
                 }
                 cull none
        }
    "#,
        &[b"failed"],
        LightSource::None,
        native(),
        ShaderCompilePolicy::default(),
    );
    assert!(result.rows[0].defaulted.is_some());
    let body = material(&result, 0);
    assert_eq!(body.settings.cull, Cull::Front);
    assert_eq!(body.settings.sort, 3.0);
    assert_eq!(body.stages.len(), 1);
    assert_eq!(body.stages[0].rgb_gen, RgbGen::Vertex);
    assert!(body.stages[0].blend.is_none());
    assert!(!body.stages[0].depth_write);
}

#[test]
fn missing_image_stops_at_its_request_and_finish_retains_native_slot_count() {
    let result = compile(
        "recipe-image-abort",
        br#"
        failed { { map textures/modern.tga
                 }
                 { map textures/not-present.tga
                 }
                 { map textures/second.tga
                 }
        }
        absent { { map textures/not-present.tga
                 } }
    "#,
        &[b"failed", b"absent", b"implicit-absent"],
        LightSource::Page(0),
        native(),
        ShaderCompilePolicy::default(),
    );
    assert!(result.rows.iter().all(|row| row.defaulted.is_some()));
    assert_eq!(result.assets.images().len(), result.image_first + 1);
    assert_eq!(material(&result, 0).stages.len(), 1);
    assert_eq!(material(&result, 0).settings.sort, 3.0);
    assert!(material(&result, 1).stages.is_empty());
    assert_eq!(material(&result, 1).settings.sort, 3.0);
    assert!(material(&result, 2).stages.is_empty());
    assert_eq!(material(&result, 2).settings.sort, 7.0);
}

#[test]
fn empty_committed_prefix_keeps_lightmap_finish_truth_without_fabricating_a_default() {
    let result = compile(
        "recipe-empty-prefix",
        br#"
        empty { { } }
        laterlightmap { { } { map $lightmap
                           } }
    "#,
        &[b"empty", b"laterlightmap"],
        LightSource::Page(0),
        native(),
        ShaderCompilePolicy::default(),
    );
    assert!(result.rows.iter().all(|row| row.defaulted.is_none()));
    assert!(material(&result, 0).stages.is_empty());
    assert!(material(&result, 1).stages.is_empty());
    assert_eq!(material(&result, 0).settings.sort, 3.0);
    assert_eq!(material(&result, 1).settings.sort, 3.0);
    assert_eq!(result.rows[0].finished, LightSource::None);
    assert_eq!(result.rows[1].finished, LightSource::Page(0));
    assert_ne!(result.rows[0].shader, result.rows[1].shader);
}

#[test]
fn trailing_lexical_failure_retains_good_definition_and_failed_committed_prefix() {
    let result = compile(
        "recipe-lexical-prefix",
        br#"
        good { { map textures/modern.tga
               } }
        bad { { map textures/second.tga
              } /* unterminated
    "#,
        &[b"good", b"bad"],
        LightSource::None,
        native(),
        ShaderCompilePolicy::default(),
    );
    assert!(result.rows[0].defaulted.is_none());
    assert!(result.rows[1].defaulted.is_some());
    assert_eq!(result.assets.images().len(), result.image_first + 2);
    assert_eq!(material(&result, 0).stages.len(), 1);
    assert_eq!(material(&result, 1).stages.len(), 1);
    assert_eq!(material(&result, 1).settings.sort, 3.0);
}

#[test]
fn repeated_animation_appends_frames_and_later_map_replaces_only_frame_zero() {
    let result = compile(
        "recipe-animation-order",
        br#"
        animation { { animMap 2 textures/modern.tga
                      animMap 3 textures/second.tga
                      map $whiteimage
                    } }
    "#,
        &[b"animation"],
        LightSource::None,
        native(),
        ShaderCompilePolicy::default(),
    );
    assert!(result.rows[0].defaulted.is_none());
    assert_eq!(result.assets.images().len(), result.image_first + 2);
    let StageTexture::Animation {
        images,
        count,
        frequency,
    } = material(&result, 0).stages[0].texture
    else {
        panic!("native animation count survives the later map");
    };
    assert_eq!(count, 2);
    assert_eq!(frequency, 3.0);
    assert_eq!(images[0], ImageId(0));
    assert_eq!(images[1], ImageId((result.image_first + 1) as u32));
}

#[test]
fn sun_receipt_follows_only_reached_prefix_even_for_native_defaults() {
    let result = compile(
        "recipe-sun-prefix",
        br#"
        before { q3map_sun 1 1 1 100 90 0
                 { map textures/modern.tga
                 }
                 q3map_sun 1 2 3 200 0 0
                 bogus
        }
        after { bogus
                q3map_sun 1 1 1 100 0 0
        }
        imagefirst { { map textures/not-present.tga
                     }
                     q3map_sun 1 1 1 100 0 0
        }
    "#,
        &[b"before", b"after", b"imagefirst"],
        LightSource::None,
        native(),
        ShaderCompilePolicy::default(),
    );
    assert!(result.rows.iter().all(|row| row.defaulted.is_some()));
    let reached = result.rows[0].sun.unwrap();
    assert_eq!(reached.color, [1., 2., 3.]);
    assert_eq!(reached.intensity, 200.);
    assert_eq!(reached.direction(), qa_core::primitives::Vec3([1., 0., 0.]));
    assert!(result.rows[1].sun.is_none());
    assert!(result.rows[2].sun.is_none());
}

#[test]
fn quote_at_eof_loads_its_returned_image_before_stage_failure() {
    let result = compile(
        "recipe-quote-eof",
        b"quoted { { map \"textures/modern.tga",
        &[b"quoted"],
        LightSource::None,
        native(),
        ShaderCompilePolicy::default(),
    );
    assert!(result.rows[0].defaulted.is_some());
    assert_eq!(result.assets.images().len(), result.image_first + 1);
    let body = material(&result, 0);
    assert_eq!(body.stages.len(), 1);
    assert_eq!(
        body.stages[0].texture,
        StageTexture::Image(ImageId(result.image_first as u32))
    );
    assert!(!body.stages[0].depth_write);
}

#[test]
fn initialized_constant_vector_with_bad_closing_token_continues_to_its_map() {
    let result = compile(
        "recipe-vector-close",
        br#"
        vector { { rgbGen const ( .5 .25 .75 wrong
                   map textures/modern.tga
                 } }
    "#,
        &[b"vector"],
        LightSource::None,
        native(),
        ShaderCompilePolicy::default(),
    );
    assert!(result.rows[0].defaulted.is_none());
    assert_eq!(result.assets.images().len(), result.image_first + 1);
    let body = material(&result, 0);
    assert_eq!(body.stages.len(), 1);
    assert_eq!(body.stages[0].rgb_gen, RgbGen::Const([0.5, 0.25, 0.75]));
    assert!(body.stages[0].depth_write);
}

#[test]
fn repeated_line_sensitive_reads_cross_a_newline_once_without_losing_its_token() {
    let vectors = compile(
        "recipe-vector-newline",
        br#"
        vector { { tcGen vector
                   ( 1 2 3 )
                   map textures/modern.tga
                 } }
    "#,
        &[b"vector"],
        LightSource::None,
        native(),
        ShaderCompilePolicy::default(),
    );
    assert!(vectors.rows[0].defaulted.is_none());
    assert_eq!(vectors.assets.images().len(), vectors.image_first + 1);
    assert_eq!(
        material(&vectors, 0).stages[0].texgen,
        TcGen::Vector([[0.; 3], [1., 2., 3.]])
    );

    let sun = compile(
        "recipe-sun-newline",
        br#"
        sun { q3map_sun
              1 2 3 4 5
              { map textures/modern.tga
              }
        }
    "#,
        &[b"sun"],
        LightSource::None,
        native(),
        ShaderCompilePolicy::default(),
    );
    assert!(sun.rows[0].defaulted.is_none());
    assert_eq!(sun.assets.images().len(), sun.image_first + 1);
    let receipt = sun.rows[0].sun.unwrap();
    assert_eq!(receipt.color, [0., 1., 2.]);
    assert_eq!(receipt.intensity, 3.);
    assert_eq!(receipt.azimuth_degrees, 4.);
    assert_eq!(receipt.elevation_degrees, 5.);
}

#[test]
fn incomplete_stretch_keeps_its_noop_slot_and_stops_later_coordinate_modifiers() {
    let result = compile(
        "recipe-incomplete-stretch",
        br#"
        stretch { { map textures/modern.tga
                    tcMod stretch sin 2
                    tcMod scale 2 3
                  } }
    "#,
        &[b"stretch"],
        LightSource::None,
        native(),
        ShaderCompilePolicy::default(),
    );
    assert!(result.rows[0].defaulted.is_none());
    let body = material(&result, 0);
    assert_eq!(body.stages[0].tcmods[0], None);
    assert_eq!(
        body.stages[0].tcmods[1],
        Some(qa_render::assets::TcMod::Script(
            qa_render::shader::TexMod::Scale([2., 3.])
        ))
    );
    let evaluator = qa_render::stage::StageEvaluator::load();
    let prepared = evaluator
        .prepare(&body.stages[0], body.settings, Default::default())
        .unwrap();
    let vertex = qa_render::Vertex {
        texcoord: [0.25, 0.5],
        ..Default::default()
    };
    assert_eq!(
        evaluator.evaluate(&prepared, &vertex).texcoord,
        vertex.texcoord
    );
}

#[test]
fn repeated_sky_dash_retains_faces_and_missing_faces_use_the_shared_native_bitmap() {
    let retained = compile(
        "recipe-sky-retain",
        br#"
        sky { skyParms textures/sky 512 textures/sky
              skyParms - 256 -
              { map $whiteimage
              }
        }
    "#,
        &[b"sky"],
        LightSource::None,
        native(),
        ShaderCompilePolicy::default(),
    );
    assert!(retained.rows[0].defaulted.is_none());
    let qa_render::assets::Sky::Cube {
        outer_box,
        inner_box,
        clouds,
        ..
    } = material(&retained, 0).settings.sky.unwrap()
    else {
        panic!("native cube sky");
    };
    assert!(outer_box.is_some());
    assert_eq!(outer_box, inner_box);
    assert_eq!(clouds.height, 256.);
    assert_eq!(retained.assets.images().len(), retained.image_first + 6);

    let missing = compile(
        "recipe-sky-missing",
        br#"
        sky { skyParms textures/absent-sky 512 -
              { map $whiteimage
              }
        }
    "#,
        &[b"sky"],
        LightSource::None,
        native(),
        ShaderCompilePolicy::default(),
    );
    assert!(missing.rows[0].defaulted.is_none());
    let qa_render::assets::Sky::Cube {
        outer_box: Some(faces),
        inner_box: None,
        ..
    } = material(&missing, 0).settings.sky.unwrap()
    else {
        panic!("missing faces keep the native sky definition");
    };
    assert!(faces.iter().all(|&face| face == faces[0]));
    assert_eq!(missing.assets.images().len(), missing.image_first + 1);
    let bitmap = missing.assets.image(faces[0]).unwrap();
    assert_eq!((bitmap.width, bitmap.height), (16, 16));
    assert_eq!(&bitmap.rgba[..4], &[255; 4]);
    assert_eq!(&bitmap.rgba[(16 + 1) * 4..(16 + 2) * 4], &[32, 32, 32, 32]);
}

#[test]
fn empty_quoted_blend_operand_is_consumed_without_swallowing_later_directives() {
    let result = compile(
        "recipe-empty-operand",
        br#"
        empty { { map $whiteimage
                  blendFunc "" rgbGen vertex
                } }
    "#,
        &[b"empty"],
        LightSource::None,
        native(),
        ShaderCompilePolicy::default(),
    );
    assert!(result.rows[0].defaulted.is_none());
    let body = material(&result, 0);
    assert_eq!(body.stages.len(), 1);
    assert_eq!(body.stages[0].rgb_gen, RgbGen::Vertex);
    assert!(body.stages[0].blend.is_none());
    assert!(body.stages[0].depth_write);
}
