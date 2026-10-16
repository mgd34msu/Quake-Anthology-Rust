#[path = "support/world_surfaces.rs"]
mod world_surfaces;

use qa_core::primitives::Vec3;
use qa_render::{
    Assets, Draw2d, FrontEnd, Limits, Refdef, ShaderBinding, ShaderBindingId, Vertex, Viewport,
    assets::{Cull, MaterialSettings, Sampler, Stage, StageTexture},
    cpu::CpuBackend,
    lightmap::{AtlasRegion, LightmapConversion},
    world::{SurfaceMaterial, geometry::LightSource},
};

fn stage(image: qa_render::ImageId) -> Stage {
    Stage {
        texture: StageTexture::Image(image),
        sampler: Sampler {
            mipmaps: false,
            ..Default::default()
        },
        ..Default::default()
    }
}

#[test]
fn nonparallel_binding_ids_select_distinct_lightmaps_from_one_material() {
    let mut assets = Assets::load_with_shader_capacity(8).unwrap();
    assert_eq!(assets.shader_binding_capacity(), 8);
    for name in ["unused first", "unused second"] {
        assets
            .register_material(name, &[stage(qa_render::ImageId(0))], Default::default())
            .unwrap();
    }
    let material = assets
        .register_material(
            "shared lightmap stages",
            &[Stage {
                texture: StageTexture::Lightmap,
                is_lightmap: true,
                ..stage(qa_render::ImageId(0))
            }],
            Default::default(),
        )
        .unwrap();
    let red = assets.register_image(1, 1, &[255, 0, 0, 255]).unwrap();
    let blue = assets.register_image(1, 1, &[0, 0, 255, 255]).unwrap();
    let first = ShaderBinding {
        lightmap: red,
        ..ShaderBinding::material(material, LightSource::Page(7))
    };
    let second = ShaderBinding {
        lightmap: blue,
        ..ShaderBinding::material(material, LightSource::Page(9))
    };
    let red_shader = assets.register_shader_binding(first).unwrap();
    let blue_shader = assets.register_shader_binding(second).unwrap();
    assert_ne!(red_shader, blue_shader);
    assert_ne!(red_shader.0, material.0);
    assert_ne!(blue_shader.0, material.0);
    assert_eq!(assets.register_shader_binding(first).unwrap(), red_shader);
    assert_eq!(
        assets.shader_binding(red_shader).unwrap().material,
        material
    );
    assert_eq!(
        assets.shader_binding(blue_shader).unwrap().material,
        material
    );
    let diffuse = assets
        .register_shader_binding(ShaderBinding::material(material, LightSource::None))
        .unwrap();
    let two_d = assets
        .register_shader_binding(ShaderBinding::material(
            material,
            LightSource::TwoDimensional,
        ))
        .unwrap();
    assert_ne!(diffuse, two_d);

    let mut cpu = CpuBackend::load_with_assets(4, 2, &assets).unwrap();
    let mut frontend = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = frontend.begin_frame([0; 4]).unwrap();
    for (x, shader) in [(0., red_shader), (2., blue_shader)] {
        assert!(frame.draw_2d(Draw2d {
            rect: [x, 0., 2., 2.],
            shader: shader.into(),
            color: [255; 4],
            ..Default::default()
        }));
    }
    let packet = frame.finish();
    assert_eq!(cpu.render(&packet, &assets).rejected, 0);
    for row in cpu.pixels().chunks_exact(4) {
        assert_eq!(row, &[0xff0000ff, 0xff0000ff, 0xffff0000, 0xffff0000]);
    }
}

#[test]
fn draw_keys_keep_material_order_when_binding_admission_order_is_reversed() {
    let mut assets = Assets::load().unwrap();
    assets
        .register_material(
            "unused",
            &[stage(qa_render::ImageId(0))],
            Default::default(),
        )
        .unwrap();
    let low = assets
        .register_material("low", &[stage(qa_render::ImageId(0))], Default::default())
        .unwrap();
    let high = assets
        .register_material("high", &[stage(qa_render::ImageId(0))], Default::default())
        .unwrap();
    let high = assets
        .register_shader_binding(ShaderBinding::material(high, LightSource::None))
        .unwrap();
    let low = assets
        .register_shader_binding(ShaderBinding::material(low, LightSource::None))
        .unwrap();
    assert!(high.0 < low.0);
    let vertices = [[8., 8., 8.], [8., -8., 8.], [8., 0., -8.]].map(|position| Vertex {
        position: Vec3(position),
        ..Default::default()
    });
    let mut frontend = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = frontend.begin_frame([0; 4]).unwrap();
    assert!(frame.add_poly(high.into(), &vertices));
    assert!(frame.add_poly(low.into(), &vertices));
    assert!(frame.render_scene(
        Refdef {
            viewport: Viewport {
                width: 4,
                height: 4,
                ..Default::default()
            },
            ..Default::default()
        },
        &[],
        &assets,
        qa_render::shader::reference::ShaderView::direct()
    ));
    let packet = frame.finish();
    let qa_render::Command::View(view) = packet.commands()[1] else {
        panic!("expected view")
    };
    let ordered: Vec<_> = packet
        .draws(view.scene.draws)
        .iter()
        .map(|draw| packet.poly(draw.index).shader.source(&()).unwrap())
        .collect();
    assert_eq!(ordered, [low, high]);
}

#[test]
fn capacity_and_partial_world_failure_restore_binding_heads_and_styled_provenance() {
    let mut assets = Assets::load_with_shader_capacity(3).unwrap();
    let material = assets
        .register_material(
            "retained",
            &[stage(qa_render::ImageId(0))],
            MaterialSettings {
                cull: Cull::None,
                ..Default::default()
            },
        )
        .unwrap();
    let retained_binding = ShaderBinding::material(material, LightSource::TwoDimensional);
    let retained = assets.register_shader_binding(retained_binding).unwrap();
    let lightmap = assets.register_image(1, 1, &[128; 4]).unwrap();
    let prefix = assets.counts();
    let candidate = SurfaceMaterial {
        shader: ShaderBinding {
            lightmap,
            lightmap_region: Some(AtlasRegion {
                page: 0,
                page_size: 1,
                x: 0,
                y: 0,
                width: 1,
                height: 1,
            }),
            lightmap_conversion: Some(LightmapConversion::Rgb { modulate: 1. }),
            ..ShaderBinding::material(material, LightSource::Samples)
        },
        texture_scale: [1.; 2],
    };
    let (geometry, visibility) = world_surfaces::inputs(2);
    assert!(
        assets
            .register_world(
                geometry,
                visibility,
                &[qa_render::world::WorldPresentationInput::new(
                    &[candidate; 2]
                )]
            )
            .is_err()
    );
    assert_eq!(assets.counts(), prefix);
    assert!(
        assets
            .shader_binding(ShaderBindingId(prefix.shader_bindings as u32))
            .is_none()
    );
    assert_eq!(
        assets.register_shader_binding(retained_binding).unwrap(),
        retained
    );
    let retry = assets
        .register_shader_binding(ShaderBinding::material(material, LightSource::None))
        .unwrap();
    assert_eq!(retry.0 as usize, prefix.shader_bindings);
    let full = assets.counts();
    assert!(
        assets
            .register_shader_binding(ShaderBinding::material(material, LightSource::Vertex))
            .is_err()
    );
    assert_eq!(assets.counts(), full);
    assets.discard_registration_suffix(prefix).unwrap();
    assert_eq!(
        assets.register_shader_binding(retained_binding).unwrap(),
        retained
    );

    let (mut geometry, visibility) = world_surfaces::inputs(1);
    geometry.surfaces[0].light_source = LightSource::Samples;
    geometry.surfaces[0].lightmap_grid = [1; 2];
    geometry.surfaces[0].light_samples = qa_core::primitives::IndexSpan { first: 0, count: 1 };
    geometry.light_samples = vec![[128; 3]];
    let world = assets
        .register_world(
            geometry,
            visibility,
            &[qa_render::world::WorldPresentationInput::new(&[candidate])],
        )
        .unwrap();
    let presentation = assets.world(world).unwrap().presentations()[0];
    let shader = assets.surface_shader(presentation, 0).unwrap();
    assert_eq!(shader, retry);
    assert_eq!(
        assets.shader_binding(shader).unwrap().styled_surface,
        Some((presentation, 0))
    );
    assert_eq!(assets.shader_binding(retained), Some(&retained_binding));
    let mut cpu = CpuBackend::load_with_assets(4, 4, &assets).unwrap();
    assert_eq!(cpu.admitted_assets(), Some(assets.counts()));
    cpu.prepare_assets(&assets, None).unwrap();
}

#[test]
fn native_registration_keys_and_styled_requests_share_atomic_canonical_rows() {
    use qa_render::{
        ShaderRegistration,
        world::{PreparedShader, WorldPresentationInput},
    };
    let mut assets = Assets::load_with_shader_capacity(3).unwrap();
    let material = assets
        .register_material(
            "compiled/path",
            &[stage(qa_render::ImageId(0))],
            Default::default(),
        )
        .unwrap();
    let base = assets
        .register_shader_binding(ShaderBinding::material(material, LightSource::None))
        .unwrap();
    let slash = assets
        .shader_registration(b"Textures/Stone.diffuse.tga", base)
        .unwrap();
    let folded = assets
        .shader_registration(b"textures/stone.alternative", base)
        .unwrap();
    let backslash = assets
        .shader_registration(b"Textures\\Stone.diffuse.tga", base)
        .unwrap();
    assert_eq!(slash, folded);
    assert_ne!(slash.key, backslash.key);
    assert_eq!(
        assets.name(slash.key).unwrap().to_ascii_lowercase(),
        "textures/stone"
    );
    assert_eq!(qa_render::shader::registration_key(b"a.b/c.tga"), b"a");
    let lightmap = assets.register_image(1, 1, &[128; 4]).unwrap();
    let prefix = assets.counts();
    let candidate = SurfaceMaterial {
        shader: ShaderBinding {
            lightmap,
            lightmap_region: Some(AtlasRegion {
                page: 0,
                page_size: 1,
                x: 0,
                y: 0,
                width: 1,
                height: 1,
            }),
            lightmap_conversion: Some(LightmapConversion::Rgb { modulate: 1. }),
            ..ShaderBinding::material(material, LightSource::Samples)
        },
        texture_scale: [1.; 2],
    };
    let registrations = [
        ShaderRegistration {
            requested: LightSource::Samples,
            finished: LightSource::None,
            ..slash.map_shader(|_| PreparedShader::Surface(0))
        },
        ShaderRegistration {
            requested: LightSource::Vertex,
            finished: LightSource::Vertex,
            ..backslash.map_shader(|_| PreparedShader::Binding {
                shader: ShaderBinding::material(material, LightSource::Vertex),
                surface: None,
            })
        },
    ];
    let (geometry, visibility) = world_surfaces::inputs(1);
    assert!(
        assets
            .register_world(
                geometry,
                visibility,
                &[WorldPresentationInput {
                    surfaces: &[candidate],
                    registrations: &registrations,
                    native_page0: Some(lightmap),
                    native_default: None,
                }]
            )
            .is_err()
    );
    assert_eq!(assets.counts(), prefix);
    assert_eq!(
        assets
            .register_shader_binding(ShaderBinding::material(material, LightSource::None))
            .unwrap(),
        base
    );
    let (geometry, visibility) = world_surfaces::inputs(1);
    let world = assets
        .register_world(
            geometry,
            visibility,
            &[WorldPresentationInput {
                surfaces: &[candidate],
                registrations: &registrations[..1],
                native_page0: Some(lightmap),
                native_default: None,
            }],
        )
        .unwrap();
    let id = assets.world(world).unwrap().presentations()[0];
    let presentation = assets.world_presentation(id).unwrap();
    assert_eq!(
        presentation.shader_registrations(),
        &[registrations[0].map_shader(|_| presentation.surface_shader(0).unwrap())]
    );
    let styled = presentation.surface_shader(0).unwrap();
    assert_eq!(styled.0 as usize, prefix.shader_bindings);
    assert_eq!(
        assets.counts().shader_bindings,
        prefix.shader_bindings + 1,
        "no unstyled intermediate binding is admitted"
    );
    assert_eq!(
        assets.shader_binding(styled).unwrap().styled_surface,
        Some((id, 0))
    );
    assert_eq!(presentation.native_page0(), Some(lightmap));
}

#[test]
fn named_zero_pass_body_is_safe_as_a_generic_world_draw_without_cpu_rejection() {
    use qa_content::vfs::Vfs;
    use qa_core::primitives::RuleSetId;
    use qa_render::{
        NativeDefaultKind,
        material::{
            resources::{ImageSettings, Images},
            world_load::{NamedShaderLighting, compile_named_shader},
        },
        shader::parse_sources,
        world::{VisibleSurface, WorldPresentationInput},
    };
    let vfs = Vfs::default();
    let mut assets = Assets::load().unwrap();
    let registration = {
        let mut images = Images::new(
            &vfs,
            &mut assets,
            RuleSetId::Quake3,
            RuleSetId::Quake3,
            ImageSettings::native(RuleSetId::Quake3),
            None,
        )
        .unwrap();
        compile_named_shader(
            &parse_sources(&[]),
            b"named/no_image",
            true,
            NamedShaderLighting {
                source: LightSource::None,
                lightmap: qa_render::ImageId(0),
            },
            qa_render::material::ShaderCompilePolicy::default(),
            &mut images,
        )
        .unwrap()
    };
    assert_eq!(
        registration.defaulted,
        Some(NativeDefaultKind::MissingImage)
    );
    let candidate = SurfaceMaterial {
        shader: *assets.shader_binding(registration.shader).unwrap(),
        texture_scale: [1.0; 2],
    };
    let (geometry, visibility) = world_surfaces::inputs(1);
    let world = assets
        .register_world(
            geometry,
            visibility,
            &[WorldPresentationInput::new(&[candidate])],
        )
        .unwrap();
    let presentation = assets.world(world).unwrap().presentations()[0];
    assert_eq!(
        assets.surface_shader(presentation, 0).unwrap(),
        registration.shader
    );
    let mut cpu = CpuBackend::load_with_assets(8, 8, &assets).unwrap();
    let mut frontend = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = frontend.begin_frame([31, 43, 59, 255]).unwrap();
    assert!(frame.add_world(world, presentation, &[VisibleSurface::default()]));
    assert!(frame.render_scene(
        Refdef {
            viewport: Viewport {
                width: 8,
                height: 8,
                ..Default::default()
            },
            ..Default::default()
        },
        &[],
        &assets,
        qa_render::shader::reference::ShaderView::direct()
    ));
    let packet = frame.finish();
    let qa_render::Command::View(view) = packet.commands()[1] else {
        panic!("world view")
    };
    assert_eq!(packet.draws(view.scene.draws).len(), 1);
    let stats = cpu.render(&packet, &assets);
    assert_eq!(stats.rejected, 0);
    assert!(
        cpu.pixels()
            .iter()
            .all(|&pixel| pixel == u32::from_le_bytes([31, 43, 59, 255]))
    );
}

#[test]
fn world_registration_rejects_native_metadata_that_disagrees_with_its_actual_body() {
    use qa_render::{
        NativeDefaultKind,
        world::{PreparedShader, WorldPresentationInput},
    };
    let mut assets = Assets::load().unwrap();
    let material = assets
        .register_material(
            "metadata/body",
            &[stage(qa_render::ImageId(0))],
            Default::default(),
        )
        .unwrap();
    let body = ShaderBinding::material(material, LightSource::None);
    let shader = assets.register_shader_binding(body).unwrap();
    let registration = assets
        .shader_registration(b"metadata/raw.tga", shader)
        .unwrap();
    let surface = SurfaceMaterial {
        shader: body,
        texture_scale: [1.0; 2],
    };
    let counts = assets.counts();
    for payload in [
        PreparedShader::Surface(0),
        PreparedShader::Binding {
            shader: body,
            surface: None,
        },
    ] {
        let valid = registration.map_shader(|_| payload);
        for invalid in [
            qa_render::ShaderRegistration {
                requested: LightSource::Page(7),
                ..valid
            },
            qa_render::ShaderRegistration {
                finished: LightSource::Vertex,
                ..valid
            },
            qa_render::ShaderRegistration {
                defaulted: Some(NativeDefaultKind::MissingImage),
                ..valid
            },
        ] {
            let (geometry, visibility) = world_surfaces::inputs(1);
            assert!(
                assets
                    .register_world(
                        geometry,
                        visibility,
                        &[WorldPresentationInput {
                            surfaces: &[surface],
                            registrations: &[invalid],
                            native_page0: None,
                            native_default: None,
                        }]
                    )
                    .is_err()
            );
            assert_eq!(assets.counts(), counts);
        }
    }
    let (geometry, visibility) = world_surfaces::inputs(1);
    let world = assets
        .register_world(
            geometry,
            visibility,
            &[WorldPresentationInput {
                surfaces: &[surface],
                registrations: &[registration.map_shader(|_| PreparedShader::Surface(0))],
                native_page0: None,
                native_default: None,
            }],
        )
        .unwrap();
    let presentation = assets.world(world).unwrap().presentations()[0];
    assert_eq!(
        assets
            .world_presentation(presentation)
            .unwrap()
            .shader_registrations(),
        &[registration]
    );
}

#[test]
fn explicit_native_default_reason_cannot_be_transplanted_to_another_material() {
    use qa_content::vfs::Vfs;
    use qa_core::primitives::RuleSetId;
    use qa_render::{
        material::{
            resources::{ImageSettings, Images},
            world_load::{NamedShaderLighting, compile_named_shader},
        },
        shader::{ShaderSource, parse_sources},
        world::{PreparedShader, WorldPresentationInput},
    };
    let vfs = Vfs::default();
    let catalog = parse_sources(&[ShaderSource {
        name: "default-proof.shader",
        bytes: b"failed { { map $whiteimage\n } bogus\n }",
    }]);
    let mut assets = Assets::load().unwrap();
    let original = {
        let mut images = Images::new(
            &vfs,
            &mut assets,
            RuleSetId::Quake3,
            RuleSetId::Quake3,
            ImageSettings::native(RuleSetId::Quake3),
            None,
        )
        .unwrap();
        compile_named_shader(
            &catalog,
            b"failed",
            true,
            NamedShaderLighting {
                source: LightSource::None,
                lightmap: qa_render::ImageId(0),
            },
            qa_render::material::ShaderCompilePolicy::default(),
            &mut images,
        )
        .unwrap()
    };
    assert!(original.defaulted.is_some());
    let original_binding = *assets.shader_binding(original.shader).unwrap();
    let material = assets.material(original_binding.material).unwrap();
    let stages = material.stages.to_vec();
    let settings = material.settings;
    assert_eq!(stages.len(), 1);
    let other_material = assets
        .register_material("another/body", &stages, settings)
        .unwrap();
    assert_ne!(other_material, original_binding.material);
    let replacement = ShaderBinding::material(other_material, LightSource::None);
    let surface = SurfaceMaterial {
        shader: replacement,
        texture_scale: [1.0; 2],
    };
    let prefix = assets.counts();
    for payload in [
        PreparedShader::Binding {
            shader: replacement,
            surface: None,
        },
        PreparedShader::Surface(0),
    ] {
        let forged = original.map_shader(|_| payload);
        let (geometry, visibility) = world_surfaces::inputs(1);
        assert!(
            assets
                .register_world(
                    geometry,
                    visibility,
                    &[WorldPresentationInput {
                        surfaces: &[surface],
                        registrations: &[forged],
                        native_page0: None,
                        native_default: None,
                    }],
                )
                .is_err()
        );
        assert_eq!(assets.counts(), prefix);
    }
    let (geometry, visibility) = world_surfaces::inputs(1);
    let world = assets
        .register_world(
            geometry,
            visibility,
            &[WorldPresentationInput {
                surfaces: &[SurfaceMaterial {
                    shader: original_binding,
                    texture_scale: [1.0; 2],
                }],
                registrations: &[original.map_shader(|_| PreparedShader::Surface(0))],
                native_page0: None,
                native_default: None,
            }],
        )
        .unwrap();
    let presentation = assets.world(world).unwrap().presentations()[0];
    assert_eq!(
        assets
            .world_presentation(presentation)
            .unwrap()
            .shader_registrations(),
        &[original]
    );
}
