use qa_content::vfs::Vfs;
use qa_core::primitives::{RuleSetId, Vec3};
use qa_render::{
    Assets, Command, FrontEnd, Limits, Refdef, ShaderRegistration, Vertex, Viewport,
    assets::ImageId,
    cpu::CpuBackend,
    material::{
        resources::{ImageSettings, Images},
        world_load::{NamedShaderLighting, compile_named_shader},
    },
    shader::{
        ShaderSource,
        context::ShaderContext,
        parse_sources,
        reference::{
            NativeShaderRef, NativeShaders, ResolvedShader, ShaderContextId, ShaderRef, ShaderView,
        },
    },
    world::geometry::LightSource,
};

struct Reader {
    context: ShaderContext,
    generation: u64,
}
impl NativeShaders for Reader {
    fn source(&self, reference: NativeShaderRef) -> Option<&ShaderRegistration> {
        if reference.context
            != (ShaderContextId {
                slot: 0,
                generation: self.generation,
            })
        {
            return None;
        }
        self.context.node(reference.node)
    }
    fn resolve(&self, reference: NativeShaderRef, assets: &Assets) -> Option<ResolvedShader> {
        let binding = self.source(reference)?.shader;
        let offset = assets
            .material(assets.shader_binding(binding)?.material)?
            .settings
            .time_offset;
        ResolvedShader::select(assets, binding, binding, offset)
    }
}
fn fixture() -> (Assets, Reader, [ShaderRef; 3]) {
    let catalog = parse_sources(&[ShaderSource {
        name: "references.shader",
        bytes: br#"
            b { cull none
                { map $whiteimage }
            }
            c { cull none
                { map $whiteimage
                  rgbGen exactVertex }
            }
        "#,
    }]);
    let mut assets = Assets::load().unwrap();
    let vfs = Vfs::default();
    let mut images = Images::new(
        &vfs,
        &mut assets,
        RuleSetId::Quake3,
        RuleSetId::Quake3,
        ImageSettings::native(RuleSetId::Quake3),
        None,
    )
    .unwrap();
    let mut compile = |name| {
        compile_named_shader(
            &catalog,
            name,
            true,
            NamedShaderLighting {
                source: LightSource::Page(0),
                lightmap: ImageId(0),
            },
            &mut images,
        )
        .unwrap()
    };
    let b = compile(b"b");
    let c = compile(b"c");
    drop(compile);
    drop(images);
    let mut context = ShaderContext::load(assets.name_capacity(), 8).unwrap();
    let mut append = context.append();
    let nodes = [
        append.admit(b).unwrap(),
        append.admit(c).unwrap(),
        append.admit(b).unwrap(),
    ];
    append.commit();
    assert_ne!(nodes[0], nodes[2]);
    assert_eq!(
        context.node(nodes[0]).unwrap().shader,
        context.node(nodes[2]).unwrap().shader
    );
    let reader = Reader {
        context,
        generation: 1,
    };
    let refs = nodes.map(|node| {
        ShaderRef::Native(NativeShaderRef {
            context: ShaderContextId {
                slot: 0,
                generation: 1,
            },
            node,
        })
    });
    (assets, reader, refs)
}
fn view() -> Refdef {
    Refdef {
        viewport: Viewport {
            width: 8,
            height: 8,
            ..Default::default()
        },
        fov: [90.; 2],
        ..Default::default()
    }
}
fn quad() -> [Vertex; 4] {
    [
        Vec3([8., 8., 8.]),
        Vec3([8., -8., 8.]),
        Vec3([8., -8., -8.]),
        Vec3([8., 8., -8.]),
    ]
    .map(|position| Vertex {
        position,
        ..Default::default()
    })
}
#[test]
fn sort_keeps_exact_logical_creation_order_when_physical_bindings_repeat() {
    let (assets, reader, refs) = fixture();
    let mut front = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = front.begin_frame([0; 4]).unwrap();
    for index in [2, 1, 0] {
        assert!(frame.add_poly(refs[index], &quad()));
    }
    assert!(frame.render_scene(
        view(),
        &[],
        &assets,
        ShaderView {
            native: &reader,
            context: Some(ShaderContextId {
                slot: 0,
                generation: 1
            })
        }
    ));
    let packet = frame.finish();
    let Command::View(view) = packet.commands()[1] else {
        panic!("view")
    };
    let captured: Vec<_> = packet
        .draws(view.scene.draws)
        .iter()
        .map(|draw| draw.shader.unwrap())
        .collect();
    assert_eq!(captured, refs);
    assert_eq!(
        packet
            .draws(view.scene.draws)
            .iter()
            .map(|draw| draw.index)
            .collect::<Vec<_>>(),
        [2, 1, 0]
    );
}
#[test]
fn sealed_packet_owns_selection_after_context_generation_changes() {
    let (assets, mut reader, refs) = fixture();
    let mut front = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = front.begin_frame([0, 0, 0, 255]).unwrap();
    assert!(frame.add_poly(refs[0], &quad()));
    assert!(frame.render_scene(
        view(),
        &[],
        &assets,
        ShaderView {
            native: &reader,
            context: Some(ShaderContextId {
                slot: 0,
                generation: 1
            })
        }
    ));
    let mut packet = frame.finish();
    packet.resolve_shaders(&assets, &reader);
    reader.generation += 1;
    reader.context.reset();
    let mut cpu = CpuBackend::load(8, 8).unwrap();
    let stats = cpu.render(&packet, &assets);
    assert_eq!(stats.rejected, 0);
    assert!(
        cpu.pixels()
            .iter()
            .any(|pixel| *pixel != u32::from_le_bytes([0, 0, 0, 255]))
    );
}
#[test]
fn stale_generation_cannot_fall_back_to_entity_or_polygon_binding() {
    let (assets, mut reader, refs) = fixture();
    let mut front = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = front.begin_frame([0, 0, 0, 255]).unwrap();
    assert!(frame.add_poly(refs[0], &quad()));
    assert!(frame.render_scene(
        view(),
        &[],
        &assets,
        ShaderView {
            native: &reader,
            context: Some(ShaderContextId {
                slot: 0,
                generation: 1
            })
        }
    ));
    assert!(frame.draw_2d(qa_render::Draw2d {
        rect: [0., 0., 8., 8.],
        shader: refs[0],
        ..Default::default()
    }));
    let mut packet = frame.finish();
    reader.generation += 1;
    packet.resolve_shaders(&assets, &reader);
    let mut cpu = CpuBackend::load(8, 8).unwrap();
    assert!(cpu.render(&packet, &assets).rejected >= 2);
    assert!(
        cpu.pixels()
            .iter()
            .all(|pixel| *pixel == u32::from_le_bytes([0, 0, 0, 255]))
    );
}
#[path = "support/world_surfaces.rs"]
mod world_surfaces;

#[test]
fn filtered_inline_surface_cannot_prepare_the_previous_views_draw_zero_shader() {
    use qa_core::primitives::Bounds;
    use qa_render::{
        SceneEntity, SceneModel, ShaderBinding,
        assets::{Cull, MaterialSettings, Stage, StageTexture},
        scene::{EntityVisibility, Span},
        world::{SurfaceMaterial, WorldPresentationInput},
    };

    let mut assets = Assets::load().unwrap();
    let shaders = [
        ("prior view red", [255, 0, 0, 255]),
        ("hidden inline green", [0, 255, 0, 255]),
        ("current view blue", [0, 0, 255, 255]),
    ]
    .map(|(name, color)| {
        let image = assets.register_image(1, 1, &color).unwrap();
        let material = assets
            .register_material(
                name,
                &[Stage {
                    texture: StageTexture::Image(image),
                    ..Default::default()
                }],
                MaterialSettings {
                    cull: Cull::None,
                    ..Default::default()
                },
            )
            .unwrap();
        assets
            .register_shader_binding(ShaderBinding::material(material, LightSource::None))
            .unwrap()
    });
    let (mut geometry, visibility) = world_surfaces::inputs(1);
    for vertex in &mut geometry.vertices {
        vertex.vertex.position = vertex.vertex.position * 6.;
    }
    geometry.surfaces[0].bounds = Bounds {
        mins: Vec3([6., -6., -6.]),
        maxs: Vec3([6., 6., 6.]),
    };
    let world = assets
        .register_world(
            geometry,
            visibility,
            &[WorldPresentationInput::new(&[SurfaceMaterial {
                shader: *assets.shader_binding(shaders[1]).unwrap(),
                texture_scale: [1.; 2],
            }])],
        )
        .unwrap();
    let presentation = assets.world(world).unwrap().presentations()[0];
    let mut cpu = CpuBackend::load_with_assets(8, 8, &assets).unwrap();
    let mut front = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = front.begin_frame([0, 0, 0, 255]).unwrap();

    // A real earlier view leaves its red selection at absolute draw zero.
    assert!(frame.add_poly(shaders[0].into(), &quad()));
    assert!(frame.render_scene(view(), &[], &assets, ShaderView::direct()));
    assert!(frame.add_entity(SceneEntity {
        model: SceneModel::World {
            world,
            presentation,
            surfaces: Span { first: 0, count: 1 },
        },
        visibility: EntityVisibility::Hidden,
        ..Default::default()
    }));
    assert!(frame.add_poly(shaders[2].into(), &quad()));
    assert!(frame.render_scene(view(), &[], &assets, ShaderView::direct()));
    let mut packet = frame.finish();
    packet.resolve_shaders(&assets, &());

    let stats = cpu.render(&packet, &assets);
    assert_eq!(stats.views, 2);
    assert_eq!(stats.rejected, 0);
    assert_eq!(stats.surfaces, 0);
    assert_eq!(cpu.world_stats().polygons, 0);
    for y in 3..5 {
        for x in 3..5 {
            assert_eq!(
                cpu.pixels()[y * 8 + x],
                u32::from_le_bytes([0, 0, 255, 255])
            );
        }
    }
}

#[test]
fn same_named_samples_keep_each_faces_lighting_when_the_logical_node_is_reused() {
    use qa_core::primitives::Bounds;
    use qa_render::{
        ShaderBinding, WorldPresentationId,
        assets::{Cull, MaterialSettings, Stage, StageTexture},
        lightmap::AtlasRegion,
        scene::Span,
        shader::{TexCoordGen, context::ShaderNodeId},
        world::{PreparedShader, SurfaceMaterial, VisibleSurface, WorldPresentationInput},
    };

    struct WorldReader {
        reader: Reader,
        presentation: WorldPresentationId,
        nodes: [ShaderNodeId; 2],
    }
    impl NativeShaders for WorldReader {
        fn source(&self, reference: NativeShaderRef) -> Option<&ShaderRegistration> {
            self.reader.source(reference)
        }
        fn resolve(&self, reference: NativeShaderRef, assets: &Assets) -> Option<ResolvedShader> {
            self.reader.resolve(reference, assets)
        }
        fn resource_nodes(
            &self,
            context: ShaderContextId,
            resource: qa_render::shader::registration::ShaderResource,
        ) -> Option<&[ShaderNodeId]> {
            (context
                == (ShaderContextId {
                    slot: 0,
                    generation: self.reader.generation,
                })
                && resource
                    == qa_render::shader::registration::ShaderResource::World(self.presentation.0))
            .then_some(&self.nodes)
        }
    }

    let mut assets = Assets::load().unwrap();
    let lightmap = assets
        .register_image(
            2,
            2,
            &[
                255, 0, 0, 255, 0, 0, 255, 255, 255, 0, 0, 255, 0, 0, 255, 255,
            ],
        )
        .unwrap();
    let material = assets
        .register_material(
            "shared Samples shader",
            &[Stage {
                texture: StageTexture::Lightmap,
                is_lightmap: true,
                texgen: TexCoordGen::Lightmap,
                ..Default::default()
            }],
            MaterialSettings {
                cull: Cull::None,
                ..Default::default()
            },
        )
        .unwrap();
    let bindings = [0, 1].map(|x| ShaderBinding {
        lightmap,
        lightmap_region: Some(AtlasRegion {
            page: 0,
            page_size: 2,
            x,
            y: 0,
            width: 1,
            height: 1,
        }),
        ..ShaderBinding::material(material, LightSource::Samples)
    });
    let registrations = bindings.map(|binding| {
        let shader = assets.register_shader_binding(binding).unwrap();
        assets.shader_registration(b"same/stone", shader).unwrap()
    });
    let (mut geometry, visibility) = world_surfaces::inputs(2);
    let vertex = geometry.vertices[0];
    geometry.vertices = [
        [8., 8., -8.],
        [8., 0., -8.],
        [8., 4., 8.],
        [8., 0., -8.],
        [8., -8., -8.],
        [8., -4., 8.],
    ]
    .into_iter()
    .enumerate()
    .map(
        |(index, position)| qa_render::world::geometry::WorldVertex {
            vertex: Vertex {
                position: Vec3(position),
                lightmap_coord: [if index < 3 { 0.25 } else { 0.75 }, 0.25],
                ..vertex.vertex
            },
        },
    )
    .collect();
    geometry.indices = (0..6).collect();
    geometry.boundaries = vec![Span { first: 0, count: 3 }, Span { first: 3, count: 3 }];
    for (index, surface) in geometry.surfaces.iter_mut().enumerate() {
        surface.vertices = Span {
            first: index as u32 * 3,
            count: 3,
        };
        surface.indices = surface.vertices;
        surface.boundaries = Span {
            first: index as u32,
            count: 1,
        };
        surface.bounds = Bounds {
            mins: Vec3([8., -8., -8.]),
            maxs: Vec3([8., 8., 8.]),
        };
        surface.light_source = LightSource::Samples;
    }
    let world = assets
        .register_world(
            geometry,
            visibility,
            &[WorldPresentationInput {
                surfaces: &bindings.map(|shader| SurfaceMaterial {
                    shader,
                    texture_scale: [1.; 2],
                }),
                registrations: &std::array::from_fn::<_, 2, _>(|index| {
                    registrations[index].map_shader(|_| PreparedShader::Surface(index as u32))
                }),
                native_page0: None,
                native_default: None,
            }],
        )
        .unwrap();
    let presentation = assets.world(world).unwrap().presentations()[0];
    let mut context = ShaderContext::load(assets.name_capacity(), 8).unwrap();
    let mut append = context.append();
    let nodes = std::array::from_fn(|index| {
        append
            .admit(
                assets
                    .world_presentation(presentation)
                    .unwrap()
                    .shader_registrations()[index],
            )
            .unwrap()
    });
    append.commit();
    assert_eq!(nodes[0], nodes[1]);
    let reader = WorldReader {
        reader: Reader {
            context,
            generation: 1,
        },
        presentation,
        nodes,
    };
    let mut colors = [[0; 2]; 2];
    for (index, native) in [false, true].into_iter().enumerate() {
        let shaders = if native {
            ShaderView {
                native: &reader,
                context: Some(ShaderContextId {
                    slot: 0,
                    generation: 1,
                }),
            }
        } else {
            ShaderView::direct()
        };
        let mut cpu = CpuBackend::load_with_assets(8, 8, &assets).unwrap();
        let mut front = FrontEnd::load(Limits::default()).unwrap();
        let mut frame = front.begin_frame([0, 0, 0, 255]).unwrap();
        assert!(frame.add_world(
            world,
            presentation,
            &[
                VisibleSurface {
                    surface: 0,
                    depth_key: 0
                },
                VisibleSurface {
                    surface: 1,
                    depth_key: 0
                },
            ]
        ));
        assert!(frame.render_scene(view(), &[], &assets, shaders));
        let mut packet = frame.finish();
        packet.resolve_shaders(&assets, &reader);
        let stats = cpu.render(&packet, &assets);
        assert_eq!(stats.rejected, 0);
        assert_eq!(stats.surfaces, 2);
        colors[index] = [cpu.pixels()[5 * 8 + 2], cpu.pixels()[5 * 8 + 5]];
    }
    assert_eq!(colors[0], [0xff0000ff, 0xffff0000]);
    assert_eq!(colors[1], colors[0]);
}

#[test]
fn converted_surface_keeps_its_body_when_the_named_history_reuses_another_variant() {
    use qa_render::shader::reference::{ShaderChoice, ShaderDefault, ShaderOccurrence};
    let (assets, reader, refs) = fixture();
    let ShaderRef::Native(reference) = refs[0] else {
        panic!("native source")
    };
    let body = refs[1].source(&reader).unwrap();
    let shader = ShaderView {
        native: &reader,
        context: Some(reference.context),
    }
    .choice(
        ShaderChoice::Converted {
            binding: body,
            occurrence: ShaderOccurrence(0),
        },
        &[],
        Some(&[reference.node]),
        ShaderDefault::Named,
    )
    .unwrap();
    let vertices = quad().map(|vertex| Vertex {
        color: [0, 128, 255, 255],
        ..vertex
    });
    let mut pixels = Vec::new();
    for selected in [ShaderRef::Direct(body), shader] {
        let mut front = FrontEnd::load(Limits::default()).unwrap();
        let mut frame = front.begin_frame([0, 0, 0, 255]).unwrap();
        assert!(frame.add_poly(selected, &vertices));
        assert!(frame.render_scene(
            view(),
            &[],
            &assets,
            ShaderView {
                native: &reader,
                context: Some(reference.context)
            }
        ));
        let mut packet = frame.finish();
        packet.resolve_shaders(&assets, &reader);
        let mut cpu = CpuBackend::load(8, 8).unwrap();
        assert_eq!(cpu.render(&packet, &assets).rejected, 0);
        pixels.push(cpu.pixels()[4 * 8 + 4]);
    }
    assert_eq!(pixels[0], u32::from_le_bytes([0, 128, 255, 255]));
    assert_eq!(pixels[1], pixels[0]);
}

#[test]
fn stale_particle_choice_drops_without_hiding_a_healthy_point() {
    use qa_render::{SceneParticle, surface_cache::PaletteLighting};
    let (mut assets, reader, refs) = fixture();
    let rgb: Vec<_> = (0..256)
        .flat_map(|index| match index {
            1 => [0, 255, 0],
            2 => [255, 0, 0],
            _ => [0, 0, 0],
        })
        .collect();
    let map: Vec<_> = (0..64).flat_map(|_| 0..=255_u8).collect();
    let palette = assets
        .register_palette(PaletteLighting::load(&rgb, &map, None, 256).unwrap())
        .unwrap();
    let ShaderRef::Native(mut stale) = refs[0] else {
        panic!("native shader")
    };
    stale.context.generation = 0;
    let mut front = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = front.begin_frame([0, 0, 0, 255]).unwrap();
    for (position, color, shader) in [
        ([16., 16., 0.], 1, refs[0]),
        ([16., -16., 0.], 2, ShaderRef::Native(stale)),
    ] {
        assert!(frame.add_particle(SceneParticle {
            position: Vec3(position),
            color,
            palette,
            shader,
            ..Default::default()
        }));
    }
    assert!(frame.render_scene(
        view(),
        &[],
        &assets,
        ShaderView {
            native: &reader,
            context: Some(stale.context)
        }
    ));
    let mut packet = frame.finish();
    packet.resolve_shaders(&assets, &reader);
    let mut cpu = CpuBackend::load(8, 8).unwrap();
    assert_eq!(cpu.render(&packet, &assets).rejected, 1);
    assert!(cpu.pixels().contains(&u32::from_le_bytes([0, 255, 0, 255])));
    assert!(!cpu.pixels().contains(&u32::from_le_bytes([255, 0, 0, 255])));
    packet.resolve_shaders(&assets, &reader);
    assert_eq!(packet.rejected, 1);
}
