use qa_core::primitives::{Bounds, Vec3};
use qa_render::SceneModel;
use qa_render::assets::{MaterialSettings, Stage};
use qa_render::scene::DrawKind;
use qa_render::scene::Span;
use qa_render::shader::{AlphaGen, BlendFactor, RgbGen, StageBlend};
use qa_render::world::geometry::*;
use qa_render::world::{SurfaceMaterial, VisibleSurface, WorldId};
use qa_render::{Assets, Command, FrontEnd, Limits, Refdef, SceneEntity, ShaderBindingId, Vertex};
use qa_world::visibility::{PvsRows, SurfaceSpan, VisLeaf, VisibilityWorld};

#[path = "scene/mdl_groups.rs"]
mod mdl_groups;
#[path = "scene/view_policy.rs"]
mod view_policy;

#[test]
fn background_commands_precede_native_views_and_ui_with_owned_ranges() {
    let assets = Assets::load().unwrap();
    let mut front = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = front.begin_frame([0; 4]).unwrap();
    assert!(frame.add_poly(
        ShaderBindingId(0).into(),
        &[Vertex {
            color: [11; 4],
            ..Default::default()
        }; 3]
    ));
    assert!(frame.render_scene(
        Refdef {
            time_ms: -12,
            ..Default::default()
        },
        &[],
        &assets,
        qa_render::shader::reference::ShaderView::direct()
    ));
    assert!(frame.draw_2d(qa_render::scene::Draw2d {
        rect: [1., 2., 3., 4.],
        ..Default::default()
    }));
    frame.background(|frame| {
        frame.clear_scene();
        assert!(frame.add_poly(
            ShaderBindingId(0).into(),
            &[Vertex {
                color: [22; 4],
                ..Default::default()
            }; 3]
        ));
        assert!(frame.render_scene(
            Refdef {
                time_ms: 34,
                ..Default::default()
            },
            &[],
            &assets,
            qa_render::shader::reference::ShaderView::direct()
        ));
    });
    let packet = frame.finish();
    let [
        Command::Clear(_),
        Command::View(background),
        Command::View(native),
        Command::Draw2d(ui),
    ] = packet.commands()
    else {
        panic!("scene order")
    };
    assert_eq!(background.refdef.time_ms, 34);
    assert_eq!(native.refdef.time_ms, -12);
    assert_eq!(
        packet.vertices(packet.polys(background.scene.polys)[0].vertices)[0].color,
        [22; 4]
    );
    assert_eq!(
        packet.vertices(packet.polys(native.scene.polys)[0].vertices)[0].color,
        [11; 4]
    );
    assert_eq!(ui.rect, [1., 2., 3., 4.]);
    assert!(front.recycle(packet).is_ok());
}

#[test]
fn picture_clipping_keeps_texture_reversal_and_seat_bounds() {
    let viewport = qa_render::Viewport {
        x: 100,
        y: 20,
        width: 40,
        height: 30,
    };
    let draw = qa_render::scene::Draw2d {
        rect: [150., 15., -60., 40.],
        texcoords: [0., 0., 1., 1.],
        ..Default::default()
    }
    .clipped(viewport)
    .unwrap();
    assert_eq!(draw.rect, [100., 20., 40., 30.]);
    assert_eq!(draw.texcoords, [5. / 6., 0.125, 1. / 6., 0.875]);
    assert!(
        qa_render::scene::Draw2d {
            rect: [0., 0., 2., 2.],
            ..Default::default()
        }
        .clipped(viewport)
        .is_none()
    );
}

#[test]
fn hyperspace_clears_only_the_native_viewport_on_cpu() {
    let assets = Assets::load().unwrap();
    let mut frontend = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = frontend.begin_frame([1, 2, 3, 4]).unwrap();
    let viewport = qa_render::Viewport {
        x: 2,
        y: 1,
        width: 7,
        height: 4,
    };
    assert!(frame.render_scene(
        Refdef {
            viewport,
            time_ms: -17,
            hyperspace: true,
            ..Default::default()
        },
        &[],
        &assets,
        qa_render::shader::reference::ShaderView::direct()
    ));
    let packet = frame.finish();
    let mut cpu = qa_render::cpu::CpuBackend::load(12, 6).unwrap();
    let stats = cpu.render(&packet, &assets);
    assert_eq!(stats.views, 1);
    assert_eq!(stats.rejected, 0);
    for y in 0..6 {
        for x in 0..12 {
            let expected = if (2..9).contains(&x) && (1..5).contains(&y) {
                [239, 239, 239, 255]
            } else {
                [1, 2, 3, 4]
            };
            assert_eq!(cpu.pixels()[y * 12 + x], u32::from_le_bytes(expected));
        }
    }
}

fn material(assets: &mut Assets, name: &str, sort: f32) -> ShaderBindingId {
    {
        let material = assets
            .register_material(
                name,
                &[Stage::default()],
                MaterialSettings {
                    sort,
                    ..MaterialSettings::default()
                },
            )
            .unwrap();
        assets
            .register_shader_binding(qa_render::ShaderBinding::material(
                material,
                qa_render::world::geometry::LightSource::TwoDimensional,
            ))
            .unwrap()
    }
}

fn alpha_material(assets: &mut Assets, name: &str, sort: f32) -> ShaderBindingId {
    {
        let material = assets
            .register_material(
                name,
                &[Stage {
                    blend: Some(StageBlend {
                        source: BlendFactor::SourceAlpha,
                        destination: BlendFactor::OneMinusSourceAlpha,
                    }),
                    rgb_gen: RgbGen::ExactVertex,
                    alpha_gen: AlphaGen::Vertex,
                    depth_write: false,
                    ..Stage::default()
                }],
                MaterialSettings {
                    sort,
                    ..MaterialSettings::default()
                },
            )
            .unwrap();
        assets
            .register_shader_binding(qa_render::ShaderBinding::material(
                material,
                qa_render::world::geometry::LightSource::TwoDimensional,
            ))
            .unwrap()
    }
}

fn poly_order(assets: &Assets, materials: &[ShaderBindingId]) -> Vec<u32> {
    let mut front = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = front.begin_frame([0; 4]).unwrap();
    for (index, &material) in materials.iter().enumerate() {
        let vertices = [Vertex {
            color: [index as u8, 0, 0, 128],
            ..Vertex::default()
        }; 3];
        assert!(frame.add_poly(material.into(), &vertices));
    }
    assert!(frame.render_scene(
        Refdef::default(),
        &[],
        assets,
        qa_render::shader::reference::ShaderView::direct()
    ));
    let packet = frame.finish();
    let Command::View(view) = packet.commands()[1] else {
        panic!()
    };
    packet
        .draws(view.scene.draws)
        .iter()
        .map(|draw| {
            let poly = packet.poly(draw.index);
            assert_eq!(packet.vertices(poly.vertices)[0].color[0], draw.index as u8);
            draw.index
        })
        .collect()
}

fn world(assets: &mut Assets, materials: &[SurfaceMaterial]) -> WorldId {
    world_variants(assets, &[materials])
}

fn world_variants(assets: &mut Assets, variants: &[&[SurfaceMaterial]]) -> WorldId {
    world_variants_with(assets, variants, |_| {})
}

fn world_variants_with(
    assets: &mut Assets,
    variants: &[&[SurfaceMaterial]],
    mutate: impl FnOnce(&mut WorldGeometry),
) -> WorldId {
    world_variants_visibility(assets, variants, mutate, None)
}

fn world_variants_visibility(
    assets: &mut Assets,
    variants: &[&[SurfaceMaterial]],
    mutate: impl FnOnce(&mut WorldGeometry),
    visibility: Option<VisibilityWorld>,
) -> WorldId {
    let materials = variants[0];
    let bounds = Bounds {
        mins: Vec3([1.0, -1.0, -1.0]),
        maxs: Vec3([1.0, 1.0, 1.0]),
    };
    let mut geometry = WorldGeometry {
        partition: GeometryPartition::SplitBsp,
        world_has_lightdata: false,
        vertices: [[1.0, -1.0, -1.0], [1.0, 1.0, -1.0], [1.0, 0.0, 1.0]]
            .map(|position| WorldVertex {
                vertex: Vertex {
                    normal: Vec3([-1.0, 0.0, 0.0]),
                    position: Vec3(position),
                    ..Vertex::default()
                },
            })
            .into(),
        indices: vec![0, 1, 2],
        boundaries: vec![Span { first: 0, count: 3 }],
        surfaces: (0..materials.len())
            .map(|source| WorldSurface {
                source_id: source as u32,
                kind: GeometryKind::Triangles,
                vertices: Span { first: 0, count: 3 },
                indices: Span { first: 0, count: 3 },
                boundaries: Span { first: 0, count: 1 },
                plane: None,
                bounds,
                texture_coordinates: TextureCoordinates::Normalized,
                texture_projection: [[0.0; 4]; 2],
                texture_minima: [0; 2],
                texture_extents: [0; 2],
                lightmap_projection: None,
                lightmap_grid: [0; 2],
                styles: [255; 4],
                light_source: LightSource::None,
                light_encoding: LightEncoding::Rgb,
                light_samples: Span::default(),
                source_texture: None,
                source_texture_info: None,
                source_shader: None,
                source_flags: 0,
                source_contents: 0,
                no_draw: false,
                source_fog: -1,
                source_brush_side: -1,
                source_lightmap: -1,
                lightmap_rect: [0; 4],
                lightmap_origin: Vec3::default(),
                lightmap_vectors: [Vec3::default(); 3],
                patch: None,
            })
            .collect(),
        light_samples: vec![],
        models: vec![],
        patch_stats: PatchStats::default(),
    };
    mutate(&mut geometry);
    let visibility = visibility.unwrap_or_else(|| {
        VisibilityWorld::load(
            vec![],
            vec![],
            vec![VisLeaf {
                selector: None,
                area: None,
                solid: false,
                bounds,
                surfaces: SurfaceSpan {
                    first: 0,
                    count: materials.len() as u32,
                },
            }],
            (0..materials.len() as u32).collect(),
            materials.len(),
            -1,
            PvsRows::all_visible(0),
        )
        .unwrap()
    });
    assets
        .register_world(
            geometry,
            visibility,
            &(variants)
                .iter()
                .map(|surfaces| qa_render::world::WorldPresentationInput::new(surfaces))
                .collect::<Vec<_>>(),
        )
        .unwrap()
}

fn presentation(assets: &Assets, world: WorldId) -> qa_render::WorldPresentationId {
    assets.world(world).unwrap().presentations()[0]
}

#[test]
fn portal_views_share_packet_storage_and_capacity_failures_preserve_the_main_view() {
    use qa_render::scene::{Light, SceneModel, Viewport};
    let mut assets = Assets::load().unwrap();
    let portal_material = {
        let material = assets
            .register_material(
                "portal",
                &[Stage {
                    alpha_gen: AlphaGen::Const(0.),
                    blend: Some(StageBlend::ALPHA),
                    depth_write: false,
                    ..Default::default()
                }],
                MaterialSettings {
                    sort: 1.,
                    portal: true,
                    cull: qa_render::shader::Cull::None,
                    ..Default::default()
                },
            )
            .unwrap();
        assets
            .register_shader_binding(qa_render::ShaderBinding::material(
                material,
                qa_render::world::geometry::LightSource::TwoDimensional,
            ))
            .unwrap()
    };
    let red = assets.register_image(1, 1, &[200, 40, 20, 255]).unwrap();
    let red = {
        let material = assets
            .register_material(
                "portal target",
                &[Stage {
                    texture: qa_render::assets::StageTexture::Image(red),
                    ..Default::default()
                }],
                MaterialSettings {
                    cull: qa_render::shader::Cull::None,
                    ..Default::default()
                },
            )
            .unwrap();
        assets
            .register_shader_binding(qa_render::ShaderBinding::material(
                material,
                qa_render::world::geometry::LightSource::TwoDimensional,
            ))
            .unwrap()
    };
    let world = {
        let shader_input_0 = *assets.shader_binding(portal_material).unwrap();
        world(
            &mut assets,
            &[SurfaceMaterial {
                shader: shader_input_0,
                ..Default::default()
            }],
        )
    };
    let presentation = presentation(&assets, world);
    let view = Refdef {
        viewport: Viewport {
            width: 8,
            height: 8,
            ..Default::default()
        },
        near: 0.25,
        far: 64.,
        fov: [90.; 2],
        ..Default::default()
    };
    let mut scratch = qa_render::world::WorldView::load(assets.world(world).unwrap());
    let mut cpu = qa_render::cpu::CpuBackend::load_with_assets(8, 8, &assets).unwrap();
    for (limits, expected_views, policy) in [
        (Limits::default(), 2),
        (
            Limits {
                entities: 2,
                ..Default::default()
            },
            1,
        ),
        (
            Limits {
                commands: 2,
                ..Default::default()
            },
            1,
        ),
        (
            Limits {
                vertices: 10,
                ..Default::default()
            },
            1,
        ),
    ]
    .into_iter()
    .map(|(limits, count)| (limits, count, qa_render::scene::ViewPolicy::default()))
    .chain(std::iter::once((
        Limits::default(),
        2,
        qa_render::scene::ViewPolicy {
            ignore_pvs: true,
            ignore_areas: true,
            cull_bounds: false,
            draw_entities: false,
            ..Default::default()
        },
    ))) {
        let mut front = FrontEnd::load(limits).unwrap();
        let mut frame = front.begin_frame([0, 0, 0, 255]).unwrap();
        assert!(frame.add_entity(SceneEntity {
            origin: Vec3([1., 0., 0.]),
            model: SceneModel::Portal {
                camera: Vec3([1., 0., 0.]),
                rotation: 0
            },
            ..Default::default()
        }));
        assert!(frame.add_entity(SceneEntity {
            origin: Vec3([0.5, 100., 0.]),
            model: SceneModel::Sprite {
                radius: 0.1,
                rotation: 0.
            },
            shader: Some(red.into()),
            ..Default::default()
        }));
        assert!(frame.add_light(Light {
            radius: 1.,
            ..Default::default()
        }));
        assert!(
            frame.add_poly(
                red.into(),
                &[
                    Vec3([-1., -3., -3.]),
                    Vec3([-1., 3., -3.]),
                    Vec3([-1., 3., 3.]),
                    Vec3([-1., -3., 3.]),
                ]
                .map(|position| Vertex {
                    position,
                    ..Default::default()
                })
            )
        );
        assert!(frame.add_world(
            world,
            presentation,
            &[VisibleSurface {
                surface: 0,
                depth_key: 1
            }]
        ));
        assert!(frame.render_world_scene(
            world,
            presentation,
            Refdef { policy, ..view },
            &[],
            &mut scratch,
            &assets,
            qa_render::shader::reference::ShaderView::direct()
        ));
        let packet = frame.finish();
        let views: Vec<_> = packet
            .commands()
            .iter()
            .filter_map(|command| match command {
                Command::View(view) => Some(view),
                _ => None,
            })
            .collect();
        assert_eq!(views.len(), expected_views);
        assert!(!views.last().unwrap().refdef.portal);
        for view in &views {
            assert_eq!(view.refdef.policy, policy);
            assert_eq!(view.refdef.near, 0.25);
            assert_eq!(view.scene.entities.count, 2);
            assert_eq!(view.scene.lights.count, 1);
            assert_eq!(
                packet.entities(view.scene.entities)[0].origin,
                Vec3([1., 0., 0.])
            );
        }
        let stats = cpu.render(&packet, &assets);
        assert_eq!(stats.views, expected_views as u32);
        assert_eq!(stats.rejected as u64, packet.rejected);
        if expected_views == 2 {
            assert!(views[0].refdef.portal && views[0].refdef.reflected_view);
            assert!(
                cpu.pixels()
                    .iter()
                    .all(|&pixel| pixel == u32::from_le_bytes([200, 40, 20, 255])),
                "portal pixels {:?}, stats {stats:?}",
                cpu.pixels()
            );
            assert_eq!(packet.rejected, 0);
        } else {
            assert!(packet.rejected > 0);
            assert!(
                cpu.pixels()
                    .iter()
                    .all(|&pixel| pixel == u32::from_le_bytes([0, 0, 0, 255]))
            );
        }
    }
}

#[test]
fn presentation_variants_select_sort_materials_for_world_and_inline_and_reject_wrong_world() {
    let mut assets = Assets::load().unwrap();
    let low = material(&mut assets, "variant-low", 1.0);
    let middle = material(&mut assets, "variant-middle", 5.0);
    let high = material(&mut assets, "variant-high", 9.0);
    let low_bindings = [SurfaceMaterial {
        shader: *assets.shader_binding(low).unwrap(),
        ..Default::default()
    }];
    let high_bindings = [SurfaceMaterial {
        shader: *assets.shader_binding(high).unwrap(),
        ..Default::default()
    }];
    let world = world_variants(&mut assets, &[&low_bindings, &high_bindings]);
    let foreign = world_variants(&mut assets, &[&low_bindings]);
    let first = presentation(&assets, world);
    let second = qa_render::WorldPresentationId(first.0 + 1);
    assert_eq!(assets.worlds().len(), 2);
    assert_eq!(assets.models().len(), 3);
    assert_eq!(
        assets.surface_binding(first, 0).unwrap().mesh_indices,
        assets.surface_binding(second, 0).unwrap().mesh_indices
    );
    let mut front = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = front.begin_frame([0; 4]).unwrap();
    assert!(frame.add_world(world, first, &[VisibleSurface::default()]));
    assert!(frame.add_poly(middle.into(), &[Vertex::default(); 3]));
    assert!(frame.render_scene(
        Refdef::default(),
        &[],
        &assets,
        qa_render::shader::reference::ShaderView::direct()
    ));
    assert!(frame.add_entity(SceneEntity {
        model: SceneModel::World {
            world,
            presentation: second,
            surfaces: Span { first: 0, count: 1 }
        },
        ..Default::default()
    }));
    assert!(frame.add_poly(middle.into(), &[Vertex::default(); 3]));
    assert!(frame.render_scene(
        Refdef::default(),
        &[],
        &assets,
        qa_render::shader::reference::ShaderView::direct()
    ));
    assert!(frame.add_world(
        world,
        presentation(&assets, foreign),
        &[VisibleSurface::default()]
    ));
    assert!(frame.render_scene(
        Refdef::default(),
        &[],
        &assets,
        qa_render::shader::reference::ShaderView::direct()
    ));
    let packet = frame.finish();
    let views: Vec<_> = packet
        .commands()
        .iter()
        .filter_map(|command| {
            if let Command::View(view) = command {
                Some(view)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        packet
            .draws(views[0].scene.draws)
            .iter()
            .map(|draw| draw.kind)
            .collect::<Vec<_>>(),
        [DrawKind::Surface, DrawKind::Poly]
    );
    assert_eq!(
        packet
            .draws(views[1].scene.draws)
            .iter()
            .map(|draw| draw.kind)
            .collect::<Vec<_>>(),
        [DrawKind::Poly, DrawKind::Surface]
    );
    assert_eq!(packet.surface(0).presentation, first);
    assert_eq!(packet.surface(1).presentation, second);
    assert_eq!(views[2].scene.draws.count, 0);
    assert_eq!(packet.rejected, 1);
}

#[test]
fn world_submissions_copy_surface_ids_and_keep_prior_views_on_overflow() {
    let mut assets = Assets::load().unwrap();
    let first_world = world(&mut assets, &[SurfaceMaterial::default(); 2]);
    let second_world = world(&mut assets, &[SurfaceMaterial::default(); 2]);
    let mut front = FrontEnd::load(Limits {
        surfaces: 2,
        ..Limits::default()
    })
    .unwrap();
    let mut frame = front.begin_frame([0; 4]).unwrap();
    let mut visible = [VisibleSurface {
        surface: 0,
        depth_key: 7,
    }];
    assert!(frame.add_world(first_world, presentation(&assets, first_world), &visible));
    visible[0].surface = 1;
    assert!(frame.render_scene(
        Refdef::default(),
        &[],
        &assets,
        qa_render::shader::reference::ShaderView::direct()
    ));
    assert!(!frame.add_world(
        WorldId(8),
        qa_render::WorldPresentationId(u32::MAX),
        &[visible[0]; 2]
    ));
    assert!(frame.add_world(second_world, presentation(&assets, second_world), &visible));
    assert!(frame.render_scene(
        Refdef::default(),
        &[],
        &assets,
        qa_render::shader::reference::ShaderView::direct()
    ));
    let packet = frame.finish();
    let Command::View(first) = packet.commands()[1] else {
        panic!()
    };
    let Command::View(second) = packet.commands()[2] else {
        panic!()
    };
    let first = &packet.surfaces(first.scene.surfaces)[0];
    assert_eq!(
        (first.world, first.surface, first.depth_key),
        (first_world, 0, 7)
    );
    let second = &packet.surfaces(second.scene.surfaces)[0];
    assert_eq!((second.world, second.surface), (second_world, 1));
    assert_eq!(packet.rejected, 1);
}

// qsrc Q3 tr_scene.c ClearScene/RenderScene preserve earlier scene payloads.
#[test]
fn scenes_copy_payloads_and_advance_ranges() {
    let assets = Assets::load().unwrap();
    let mut front = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = front.begin_frame([0; 4]).unwrap();
    let first = SceneEntity {
        frame: 7,
        ..SceneEntity::default()
    };
    assert!(frame.add_entity(first));
    let mut vertices = [Vertex::default(); 3];
    assert!(frame.add_poly(ShaderBindingId(0).into(), &vertices));
    let mut areas = [0x55u8, 0xaa];
    assert!(frame.render_scene(
        Refdef::default(),
        &areas,
        &assets,
        qa_render::shader::reference::ShaderView::direct()
    ));
    vertices[0].color = [0; 4];
    assert_eq!(vertices[0].color, [0; 4]);
    areas.fill(0);
    frame.clear_scene();
    assert!(frame.add_entity(SceneEntity {
        frame: 11,
        ..SceneEntity::default()
    }));
    assert!(frame.render_scene(
        Refdef::default(),
        &[],
        &assets,
        qa_render::shader::reference::ShaderView::direct()
    ));
    let packet = frame.finish();
    let Command::View(view0) = packet.commands()[1] else {
        panic!()
    };
    let Command::View(view1) = packet.commands()[2] else {
        panic!()
    };
    assert_eq!(packet.entities(view0.scene.entities)[0].frame, 7);
    assert_eq!(packet.entities(view1.scene.entities)[0].frame, 11);
    assert_eq!(packet.hidden_areas(view0.hidden_areas), &[0x55, 0xaa]);
    assert_eq!(
        packet.vertices(packet.polys(view0.scene.polys)[0].vertices)[0].color,
        [255; 4]
    );
    assert_eq!(view1.scene.polys.count, 0);
    assert!(front.recycle(packet).is_ok());
}

#[test]
fn two_outstanding_packets_prevent_reuse_until_returned() {
    let mut front = FrontEnd::load(Limits::default()).unwrap();
    let first = front.begin_frame([1; 4]).unwrap().finish();
    let second = front.begin_frame([2; 4]).unwrap().finish();
    assert!(front.begin_frame([3; 4]).is_none());
    assert_eq!(first.frame, 1);
    assert_eq!(second.frame, 2);
    assert!(front.recycle(first).is_ok());
    let third = front.begin_frame([3; 4]).unwrap().finish();
    assert_eq!(third.frame, 3);
    let Command::Clear(color) = second.commands()[0] else {
        panic!()
    };
    assert_eq!(color, [2; 4]);
    assert!(front.recycle(second).is_ok());
    assert!(front.recycle(third).is_ok());
}

fn cold_limits() -> Limits {
    Limits {
        commands: 4,
        entities: 1,
        lod_choices: 0,
        polys: 1,
        vertices: 3,
        lights: 1,
        particles: 1,
        area_bytes: 1,
        surfaces: 1,
        draws: 0,
    }
}

#[test]
fn cold_capacity_stage_requires_both_owned_packets_returned() {
    let limits = cold_limits();
    let mut front = FrontEnd::load(limits).unwrap();
    let first = front.begin_frame([1; 4]).unwrap().finish();
    assert!(front.stage_capacity(limits).is_err());
    let second = front.begin_frame([2; 4]).unwrap().finish();
    assert!(front.stage_capacity(limits).is_err());
    assert!(front.recycle(first).is_ok());
    assert!(front.stage_capacity(limits).is_err());
    assert!(matches!(second.commands(), [Command::Clear([2, 2, 2, 2])]));
    assert!(front.recycle(second).is_ok());
    front.stage_capacity(limits).unwrap().commit();
    let third = front.begin_frame([3; 4]).unwrap().finish();
    assert_eq!(third.frame, 3);
    assert!(front.recycle(third).is_ok());
}

#[test]
fn dropped_capacity_stage_preserves_arenas_and_commit_grows_the_same_owner() {
    let mut assets = Assets::load().unwrap();
    let world = world(&mut assets, &[SurfaceMaterial::default(); 3]);
    let presentation = presentation(&assets, world);
    let mut front = FrontEnd::load(cold_limits()).unwrap();
    let required = Limits {
        entities: 3,
        polys: 2,
        vertices: 6,
        surfaces: 3,
        area_bytes: 2,
        ..cold_limits()
    };
    drop(front.stage_capacity(required).unwrap());
    assert_eq!(front.limits().entities, 1);
    let mut old = front.begin_frame([0; 4]).unwrap();
    assert!(old.add_entity(SceneEntity::default()));
    assert!(!old.add_entity(SceneEntity::default()));
    assert!(front.recycle(old.finish()).is_ok());

    front.stage_capacity(required).unwrap().commit();
    assert_eq!(front.limits().draws, 8);
    let mut grown = front.begin_frame([0; 4]).unwrap();
    for _ in 0..3 {
        assert!(grown.add_entity(SceneEntity::default()));
    }
    assert!(!grown.add_entity(SceneEntity::default()));
    for _ in 0..2 {
        assert!(grown.add_poly(ShaderBindingId(0).into(), &[Vertex::default(); 3]));
    }
    assert!(
        grown.add_world(
            world,
            presentation,
            &(0..3)
                .map(|surface| VisibleSurface {
                    surface,
                    depth_key: 1,
                })
                .collect::<Vec<_>>()
        )
    );
    assert!(grown.render_scene(
        Refdef::default(),
        &[1, 2],
        &assets,
        qa_render::shader::reference::ShaderView::direct()
    ));
    let packet = grown.finish();
    assert_eq!(packet.frame, 2);
    let Command::View(view) = packet.commands()[1] else {
        panic!("grown scene")
    };
    assert_eq!(view.scene.entities.count, 3);
    assert_eq!(view.scene.polys.count, 2);
    assert_eq!(view.scene.surfaces.count, 3);
    assert_eq!(view.scene.draws.count, 8);
    assert_eq!(packet.hidden_areas(view.hidden_areas), &[1, 2]);
    assert!(front.recycle(packet).is_ok());
    front.stage_capacity(cold_limits()).unwrap().commit();
    assert_eq!(front.limits().entities, 3);
    assert_eq!(front.limits().surfaces, 3);
}

#[test]
fn rejected_capacity_stage_preserves_pool_and_explicit_draw_budget() {
    let mut front = FrontEnd::load(cold_limits()).unwrap();
    assert!(
        front
            .stage_capacity(Limits {
                entities: usize::MAX,
                ..cold_limits()
            })
            .is_err()
    );
    let packet = front.begin_frame([0; 4]).unwrap().finish();
    assert!(front.recycle(packet).is_ok());
    let mut front = FrontEnd::load(Limits {
        draws: 2,
        ..cold_limits()
    })
    .unwrap();
    front
        .stage_capacity(Limits {
            entities: 3,
            surfaces: 3,
            ..cold_limits()
        })
        .unwrap()
        .commit();
    assert_eq!(front.limits().draws, 2);
    let mut frame = front.begin_frame([0; 4]).unwrap();
    assert!(frame.add_entity(SceneEntity::default()));
    assert!(frame.add_entity(SceneEntity::default()));
    assert!(!frame.add_entity(SceneEntity::default()));
    assert!(front.recycle(frame.finish()).is_ok());
}

#[test]
fn rejected_poly_and_view_leave_existing_payloads_intact() {
    let assets = Assets::load().unwrap();
    let limits = Limits {
        commands: 3,
        entities: 1,
        lod_choices: 0,
        polys: 1,
        vertices: 3,
        lights: 1,
        particles: 1,
        area_bytes: 2,
        surfaces: 1,
        draws: 0,
    };
    let mut front = FrontEnd::load(limits).unwrap();
    let mut frame = front.begin_frame([0; 4]).unwrap();
    assert!(!frame.add_poly(ShaderBindingId(0).into(), &[Vertex::default(); 4]));
    assert!(frame.add_poly(ShaderBindingId(0).into(), &[Vertex::default(); 3]));
    assert!(!frame.render_scene(
        Refdef::default(),
        &[0; 3],
        &assets,
        qa_render::shader::reference::ShaderView::direct()
    ));
    assert!(frame.render_scene(
        Refdef::default(),
        &[0; 2],
        &assets,
        qa_render::shader::reference::ShaderView::direct()
    ));
    let packet = frame.finish();
    assert_eq!(packet.rejected, 2);
    let Command::View(view) = packet.commands()[1] else {
        panic!()
    };
    assert_eq!(view.scene.polys.count, 1);
    assert_eq!(packet.polys(view.scene.polys)[0].vertices.count, 3);
}

// Q3 SortNewShader preserves floating shader sort, then R_AddDrawSurf uses
// the sorted shader/instance key. Surface submission time cannot override it.
#[test]
fn shared_draw_order_places_translucent_surfaces_after_opaque_entities() {
    let mut assets = Assets::load().unwrap();
    let translucent = alpha_material(&mut assets, "translucent", 8.25);
    let opaque = material(&mut assets, "opaque", 3.0);
    let middle = material(&mut assets, "middle", 5.0);
    let world = {
        let shader_input_0 = *assets.shader_binding(translucent).unwrap();
        world(
            &mut assets,
            &[SurfaceMaterial {
                shader: shader_input_0,
                ..SurfaceMaterial::default()
            }],
        )
    };
    let mut front = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = front.begin_frame([0; 4]).unwrap();
    assert!(frame.add_world(
        world,
        presentation(&assets, world),
        &[VisibleSurface {
            surface: 0,
            depth_key: 7
        }]
    ));
    assert!(frame.add_entity(SceneEntity {
        shader: Some(opaque.into()),
        ..SceneEntity::default()
    }));
    assert!(frame.add_poly(middle.into(), &[Vertex::default(); 3]));
    assert!(frame.render_scene(
        Refdef::default(),
        &[],
        &assets,
        qa_render::shader::reference::ShaderView::direct()
    ));
    let packet = frame.finish();
    let Command::View(view) = packet.commands()[1] else {
        panic!()
    };
    let draws = packet.draws(view.scene.draws);
    assert_eq!(
        draws
            .iter()
            .map(|draw| (draw.kind, draw.index))
            .collect::<Vec<_>>(),
        [
            (DrawKind::Entity, 0),
            (DrawKind::Poly, 0),
            (DrawKind::Surface, 0)
        ]
    );
    assert_eq!(packet.surface(draws[2].index).depth_key, 7);
    assert_eq!(
        packet
            .entity(draws[0].index)
            .shader
            .map(|shader| shader.source(&()).unwrap()),
        Some(opaque)
    );
    assert_eq!(
        packet.poly(draws[1].index).shader.source(&()).unwrap(),
        middle
    );
}

#[test]
fn shared_draw_order_retains_fractional_sorts_and_numeric_material_ties() {
    let mut assets = Assets::load().unwrap();
    let later = material(&mut assets, "later", 3.75);
    let earlier = material(&mut assets, "earlier", 3.25);
    let tied_first = material(&mut assets, "tied-first", 3.25);
    let tied_second = material(&mut assets, "tied-second", 3.25);
    let mut front = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = front.begin_frame([0; 4]).unwrap();
    for material in [later, tied_second, earlier, tied_first] {
        assert!(frame.add_poly(material.into(), &[Vertex::default(); 3]));
    }
    assert!(frame.render_scene(
        Refdef::default(),
        &[],
        &assets,
        qa_render::shader::reference::ShaderView::direct()
    ));
    let packet = frame.finish();
    let Command::View(view) = packet.commands()[1] else {
        panic!()
    };
    assert_eq!(
        packet
            .draws(view.scene.draws)
            .iter()
            .map(|draw| packet.poly(draw.index).shader.source(&()).unwrap())
            .collect::<Vec<_>>(),
        [earlier, tied_first, tied_second, later]
    );
}

#[test]
fn equal_native_keys_keep_native_cross_kind_shortsort_swaps() {
    let mut assets = Assets::load().unwrap();
    let material = alpha_material(&mut assets, "same-alpha", 8.0);
    let world = {
        let shader_input_0 = *assets.shader_binding(material).unwrap();
        world(
            &mut assets,
            &[SurfaceMaterial {
                shader: shader_input_0,
                ..SurfaceMaterial::default()
            }],
        )
    };
    let mut front = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = front.begin_frame([0; 4]).unwrap();
    assert!(frame.add_poly(material.into(), &[Vertex::default(); 3]));
    assert!(frame.add_world(
        world,
        presentation(&assets, world),
        &[VisibleSurface {
            surface: 0,
            depth_key: 1
        }]
    ));
    assert!(frame.add_poly(material.into(), &[Vertex::default(); 3]));
    assert!(frame.render_scene(
        Refdef::default(),
        &[],
        &assets,
        qa_render::shader::reference::ShaderView::direct()
    ));
    let packet = frame.finish();
    let Command::View(view) = packet.commands()[1] else {
        panic!()
    };
    assert_eq!(
        packet
            .draws(view.scene.draws)
            .iter()
            .map(|draw| (draw.kind, draw.index))
            .collect::<Vec<_>>(),
        [
            (DrawKind::Surface, 0),
            (DrawKind::Poly, 1),
            (DrawKind::Poly, 0)
        ]
    );
    assert_eq!(packet.surface(0).draw_rank, 0);
}

// tr_main.c shortsort rotates an all-equal short array once; qsortFast instead
// swaps the middle pivot to the front before its <=/>= scans skip an equal run.
#[test]
fn native_equal_alpha_order_matches_three_eight_and_nine_plus_draws() {
    let mut assets = Assets::load().unwrap();
    let material = alpha_material(&mut assets, "equal-alpha", 8.0);
    for (count, expected) in [
        (3, vec![1, 2, 0]),
        (8, vec![1, 2, 3, 4, 5, 6, 7, 0]),
        (9, vec![4, 1, 2, 3, 0, 5, 6, 7, 8]),
        (
            17,
            vec![8, 1, 2, 3, 4, 5, 6, 7, 0, 9, 10, 11, 12, 13, 14, 15, 16],
        ),
    ] {
        assert_eq!(poly_order(&assets, &vec![material; count]), expected);
    }
}

#[test]
fn native_mixed_partition_keys_keep_original_equal_group_swaps() {
    let mut assets = Assets::load().unwrap();
    let lower = alpha_material(&mut assets, "lower-alpha", 7.0);
    let upper = alpha_material(&mut assets, "upper-alpha", 8.0);
    let cases = [
        ([2, 1, 2, 1, 2, 1, 2, 1, 2], [1, 5, 3, 7, 0, 6, 2, 4, 8]),
        ([2, 1, 2, 1, 1, 2, 1, 2, 2], [4, 1, 3, 6, 5, 0, 7, 8, 2]),
    ];
    for (keys, expected) in cases {
        let materials = keys.map(|key| if key == 1 { lower } else { upper });
        assert_eq!(poly_order(&assets, &materials), expected);
    }
}

#[test]
fn draw_keys_group_lightmaps_without_reordering_raw_depth_surfaces() {
    let mut assets = Assets::load().unwrap();
    let material = material(&mut assets, "lightmapped", 3.0);
    let image = assets.register_image(1, 1, &[128; 4]).unwrap();
    let world = {
        let shader_input_0 = *assets.shader_binding(material).unwrap();
        let shader_input_1 = *assets.shader_binding(material).unwrap();
        world(
            &mut assets,
            &[
                SurfaceMaterial {
                    shader: qa_render::ShaderBinding {
                        lightmap: image,
                        ..shader_input_0
                    },
                    ..SurfaceMaterial::default()
                },
                SurfaceMaterial {
                    shader: shader_input_1,
                    ..SurfaceMaterial::default()
                },
            ],
        )
    };
    let mut front = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = front.begin_frame([0; 4]).unwrap();
    assert!(frame.add_world(
        world,
        presentation(&assets, world),
        &[
            VisibleSurface {
                surface: 0,
                depth_key: 2
            },
            VisibleSurface {
                surface: 1,
                depth_key: 9
            },
        ]
    ));
    assert!(frame.render_scene(
        Refdef::default(),
        &[],
        &assets,
        qa_render::shader::reference::ShaderView::direct()
    ));
    let packet = frame.finish();
    let Command::View(view) = packet.commands()[1] else {
        panic!()
    };
    assert_eq!(
        packet
            .draws(view.scene.draws)
            .iter()
            .map(|draw| draw.index)
            .collect::<Vec<_>>(),
        [1, 0]
    );
    assert_eq!(
        packet
            .surfaces(view.scene.surfaces)
            .iter()
            .map(|surface| (surface.surface, surface.depth_key))
            .collect::<Vec<_>>(),
        [(0, 2), (1, 9)]
    );
    assert_eq!(packet.surface(0).draw_rank, 1);
    assert_eq!(packet.surface(1).draw_rank, 0);
}

#[test]
fn surface_draw_ranks_are_native_view_local_and_preserve_bsp_depth_keys() {
    let mut assets = Assets::load().unwrap();
    let material = material(&mut assets, "coincident", 3.0);
    let world = {
        let shader_input_0 = *assets.shader_binding(material).unwrap();
        world(
            &mut assets,
            &[SurfaceMaterial {
                shader: shader_input_0,
                ..SurfaceMaterial::default()
            }; 3],
        )
    };
    let mut front = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = front.begin_frame([0; 4]).unwrap();
    let surfaces = [
        VisibleSurface {
            surface: 0,
            depth_key: 21,
        },
        VisibleSurface {
            surface: 1,
            depth_key: 7,
        },
        VisibleSurface {
            surface: 2,
            depth_key: 99,
        },
    ];
    assert!(frame.add_world(world, presentation(&assets, world), &surfaces));
    assert!(frame.render_scene(
        Refdef::default(),
        &[],
        &assets,
        qa_render::shader::reference::ShaderView::direct()
    ));
    assert!(frame.add_world(world, presentation(&assets, world), &surfaces[..2]));
    assert!(frame.render_scene(
        Refdef::default(),
        &[],
        &assets,
        qa_render::shader::reference::ShaderView::direct()
    ));
    let packet = frame.finish();
    let Command::View(first) = packet.commands()[1] else {
        panic!()
    };
    let Command::View(second) = packet.commands()[2] else {
        panic!()
    };
    assert_eq!(
        packet
            .draws(first.scene.draws)
            .iter()
            .map(|draw| draw.index)
            .collect::<Vec<_>>(),
        [1, 2, 0]
    );
    assert_eq!(
        packet
            .draws(second.scene.draws)
            .iter()
            .map(|draw| draw.index)
            .collect::<Vec<_>>(),
        [4, 3]
    );
    assert_eq!(
        packet
            .surfaces(first.scene.surfaces)
            .iter()
            .map(|surface| (surface.surface, surface.depth_key, surface.draw_rank))
            .collect::<Vec<_>>(),
        [(0, 21, 2), (1, 7, 0), (2, 99, 1)]
    );
    assert_eq!(
        packet
            .surfaces(second.scene.surfaces)
            .iter()
            .map(|surface| (surface.surface, surface.depth_key, surface.draw_rank))
            .collect::<Vec<_>>(),
        [(0, 21, 1), (1, 7, 0)]
    );
}

#[test]
fn separately_sorted_view_ranges_keep_absolute_payload_indices() {
    let mut assets = Assets::load().unwrap();
    let late = material(&mut assets, "late", 7.0);
    let early = material(&mut assets, "early", 2.0);
    let mut front = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = front.begin_frame([0; 4]).unwrap();
    let vertices = [Vertex::default(); 3];
    for material in [late, early] {
        assert!(frame.add_poly(material.into(), &vertices));
    }
    assert!(frame.render_scene(
        Refdef::default(),
        &[1],
        &assets,
        qa_render::shader::reference::ShaderView::direct()
    ));
    frame.clear_scene();
    for material in [late, early] {
        assert!(frame.add_poly(material.into(), &vertices));
    }
    assert!(frame.render_scene(
        Refdef::default(),
        &[2],
        &assets,
        qa_render::shader::reference::ShaderView::direct()
    ));
    let packet = frame.finish();
    let Command::View(first) = packet.commands()[1] else {
        panic!()
    };
    let Command::View(second) = packet.commands()[2] else {
        panic!()
    };
    assert_eq!(
        packet
            .draws(first.scene.draws)
            .iter()
            .map(|draw| draw.index)
            .collect::<Vec<_>>(),
        [1, 0]
    );
    assert_eq!(
        packet
            .draws(second.scene.draws)
            .iter()
            .map(|draw| draw.index)
            .collect::<Vec<_>>(),
        [3, 2]
    );
    assert_eq!((first.scene.draws.first, first.scene.draws.count), (0, 2));
    assert_eq!((second.scene.draws.first, second.scene.draws.count), (2, 2));
    assert_eq!(packet.hidden_areas(first.hidden_areas), &[1]);
    assert_eq!(packet.hidden_areas(second.hidden_areas), &[2]);
}

#[test]
fn draw_capacity_drops_whole_payload_submissions_atomically() {
    let assets = Assets::load().unwrap();
    let mut front = FrontEnd::load(Limits {
        draws: 2,
        ..Limits::default()
    })
    .unwrap();
    let mut frame = front.begin_frame([0; 4]).unwrap();
    assert!(frame.add_entity(SceneEntity::default()));
    assert!(!frame.add_world(
        WorldId(99),
        qa_render::WorldPresentationId(u32::MAX),
        &[VisibleSurface::default(); 2]
    ));
    assert!(frame.add_poly(ShaderBindingId(0).into(), &[Vertex::default(); 3]));
    assert!(!frame.add_entity(SceneEntity::default()));
    assert!(frame.render_scene(
        Refdef::default(),
        &[],
        &assets,
        qa_render::shader::reference::ShaderView::direct()
    ));
    let packet = frame.finish();
    let Command::View(view) = packet.commands()[1] else {
        panic!()
    };
    assert_eq!(view.scene.entities.count, 1);
    assert_eq!(view.scene.polys.count, 1);
    assert_eq!(view.scene.surfaces.count, 0);
    assert_eq!(view.scene.draws.count, 2);
    assert_eq!(packet.rejected, 2);
}

#[test]
fn failed_view_preserves_previous_draws_and_native_pending_order() {
    let assets = Assets::load().unwrap();
    let mut front = FrontEnd::load(Limits {
        commands: 3,
        area_bytes: 1,
        ..Limits::default()
    })
    .unwrap();
    let mut frame = front.begin_frame([0; 4]).unwrap();
    assert!(frame.add_poly(ShaderBindingId(0).into(), &[Vertex::default(); 3]));
    assert!(!frame.render_scene(
        Refdef::default(),
        &[0; 2],
        &assets,
        qa_render::shader::reference::ShaderView::direct()
    ));
    assert!(frame.add_poly(ShaderBindingId(0).into(), &[Vertex::default(); 3]));
    assert!(frame.render_scene(
        Refdef::default(),
        &[7],
        &assets,
        qa_render::shader::reference::ShaderView::direct()
    ));
    assert!(frame.add_poly(ShaderBindingId(0).into(), &[Vertex::default(); 3]));
    assert!(!frame.render_scene(
        Refdef::default(),
        &[8],
        &assets,
        qa_render::shader::reference::ShaderView::direct()
    ));
    assert!(frame.render_scene(
        Refdef::default(),
        &[],
        &assets,
        qa_render::shader::reference::ShaderView::direct()
    ));
    assert!(!frame.render_scene(
        Refdef::default(),
        &[],
        &assets,
        qa_render::shader::reference::ShaderView::direct()
    ));
    let packet = frame.finish();
    let Command::View(first) = packet.commands()[1] else {
        panic!()
    };
    let Command::View(second) = packet.commands()[2] else {
        panic!()
    };
    assert_eq!(
        packet
            .draws(first.scene.draws)
            .iter()
            .map(|draw| draw.index)
            .collect::<Vec<_>>(),
        [1, 0]
    );
    assert_eq!(
        packet
            .draws(second.scene.draws)
            .iter()
            .map(|draw| draw.index)
            .collect::<Vec<_>>(),
        [2]
    );
    assert_eq!(packet.hidden_areas(first.hidden_areas), &[7]);
    assert_eq!(packet.rejected, 3);
}

#[test]
fn invalid_frozen_handles_only_drop_the_affected_draw() {
    let assets = Assets::load().unwrap();
    let mut front = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = front.begin_frame([0; 4]).unwrap();
    assert!(frame.add_poly(ShaderBindingId(999).into(), &[Vertex::default(); 3]));
    assert!(frame.add_entity(SceneEntity {
        model: qa_render::ModelId(999).into(),
        ..SceneEntity::default()
    }));
    assert!(frame.add_world(
        WorldId(999),
        qa_render::WorldPresentationId(u32::MAX),
        &[VisibleSurface::default()]
    ));
    assert!(frame.add_poly(ShaderBindingId(0).into(), &[Vertex::default(); 3]));
    assert!(frame.render_scene(
        Refdef::default(),
        &[],
        &assets,
        qa_render::shader::reference::ShaderView::direct()
    ));
    assert!(frame.add_poly(ShaderBindingId(0).into(), &[Vertex::default(); 3]));
    assert!(frame.render_scene(
        Refdef::default(),
        &[],
        &assets,
        qa_render::shader::reference::ShaderView::direct()
    ));
    let packet = frame.finish();
    let Command::View(first) = packet.commands()[1] else {
        panic!()
    };
    let Command::View(second) = packet.commands()[2] else {
        panic!()
    };
    assert_eq!(
        packet
            .draws(first.scene.draws)
            .iter()
            .map(|draw| (draw.kind, draw.index))
            .collect::<Vec<_>>(),
        [(DrawKind::Poly, 1)]
    );
    assert_eq!(
        packet
            .draws(second.scene.draws)
            .iter()
            .map(|draw| (draw.kind, draw.index))
            .collect::<Vec<_>>(),
        [(DrawKind::Poly, 2)]
    );
    assert_eq!((first.scene.draws.first, second.scene.draws.first), (0, 1));
    assert_eq!(packet.rejected, 3);
}

#[test]
fn native_signed_zero_sort_keeps_numeric_material_registration_order() {
    let mut assets = Assets::load().unwrap();
    let positive = material(&mut assets, "positive-zero", 0.0);
    let negative = material(&mut assets, "negative-zero", -0.0);
    let mut front = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = front.begin_frame([0; 4]).unwrap();
    assert!(frame.add_poly(negative.into(), &[Vertex::default(); 3]));
    assert!(frame.add_poly(positive.into(), &[Vertex::default(); 3]));
    assert!(frame.render_scene(
        Refdef::default(),
        &[],
        &assets,
        qa_render::shader::reference::ShaderView::direct()
    ));
    let packet = frame.finish();
    let Command::View(view) = packet.commands()[1] else {
        panic!()
    };
    assert_eq!(
        packet
            .draws(view.scene.draws)
            .iter()
            .map(|draw| packet.poly(draw.index).shader.source(&()).unwrap())
            .collect::<Vec<_>>(),
        [positive, negative]
    );
}

#[test]
fn automatic_draw_capacity_is_checked_before_allocation() {
    assert!(
        FrontEnd::load(Limits {
            entities: u32::MAX as usize,
            ..Limits::default()
        })
        .is_err()
    );
    assert!(
        FrontEnd::load(Limits {
            surfaces: usize::MAX,
            ..Limits::default()
        })
        .is_err()
    );
}

#[test]
fn inline_surface_submission_owns_one_pose_and_keeps_per_face_bindings() {
    let mut assets = Assets::load().unwrap();
    let low = material(&mut assets, "inline/low", 2.0);
    let high = material(&mut assets, "inline/high", 7.0);
    let id = {
        let shader_input_0 = *assets.shader_binding(high).unwrap();
        let shader_input_1 = *assets.shader_binding(low).unwrap();
        world(
            &mut assets,
            &[
                SurfaceMaterial {
                    shader: shader_input_0,
                    ..Default::default()
                },
                SurfaceMaterial {
                    shader: shader_input_1,
                    ..Default::default()
                },
            ],
        )
    };
    let mut front = FrontEnd::load(Limits {
        entities: 2,
        surfaces: 5,
        ..Limits::default()
    })
    .unwrap();
    let mut frame = front.begin_frame([0; 4]).unwrap();
    let mut entity = SceneEntity {
        model: SceneModel::World {
            presentation: presentation(&assets, id),
            world: id,
            surfaces: Span { first: 0, count: 2 },
        },
        origin: Vec3([13.0, 21.0, 34.0]),
        color: [1, 2, 3, 4],
        shader_time: 0.25,
        ..SceneEntity::default()
    };
    assert!(frame.add_entity(entity));
    entity.origin = Vec3([100.0; 3]);
    assert!(frame.add_world(
        id,
        presentation(&assets, id),
        &[VisibleSurface {
            surface: 1,
            depth_key: 19
        }]
    ));
    assert!(frame.render_scene(
        Refdef::default(),
        &[],
        &assets,
        qa_render::shader::reference::ShaderView::direct()
    ));
    frame.clear_scene();
    entity.shader = Some(high.into());
    assert!(frame.add_entity(entity));
    assert!(frame.render_scene(
        Refdef::default(),
        &[],
        &assets,
        qa_render::shader::reference::ShaderView::direct()
    ));
    let packet = frame.finish();
    let Command::View(first) = packet.commands()[1] else {
        panic!()
    };
    let Command::View(second) = packet.commands()[2] else {
        panic!()
    };
    assert_eq!(first.scene.entities.count, 1);
    assert_eq!(packet.entity(0).origin, Vec3([13.0, 21.0, 34.0]));
    assert_eq!(packet.entity(0).color, [1, 2, 3, 4]);
    assert_eq!(packet.entity(1).origin, Vec3([100.0; 3]));
    assert_eq!(
        packet
            .surfaces(first.scene.surfaces)
            .iter()
            .map(|s| s.entity)
            .collect::<Vec<_>>(),
        [Some(0), Some(0), None]
    );
    assert_eq!(
        packet
            .surfaces(second.scene.surfaces)
            .iter()
            .map(|s| s.entity)
            .collect::<Vec<_>>(),
        [Some(1), Some(1)]
    );
    let order: Vec<_> = packet
        .draws(first.scene.draws)
        .iter()
        .map(|draw| packet.surface(draw.index).surface)
        .collect();
    assert_eq!(order, [1, 1, 0]);
    assert!(front.recycle(packet).is_ok());
}

#[test]
fn inline_surface_capacity_failure_leaves_entity_and_draw_arenas_unchanged() {
    let mut assets = Assets::load().unwrap();
    let id = world(&mut assets, &[SurfaceMaterial::default(); 2]);
    for limits in [
        Limits {
            surfaces: 1,
            ..Limits::default()
        },
        Limits {
            draws: 1,
            ..Limits::default()
        },
    ] {
        let mut front = FrontEnd::load(limits).unwrap();
        let mut frame = front.begin_frame([0; 4]).unwrap();
        let instance = SceneEntity {
            model: SceneModel::World {
                presentation: presentation(&assets, id),
                world: id,
                surfaces: Span { first: 0, count: 2 },
            },
            ..SceneEntity::default()
        };
        assert!(!frame.add_entity(instance));
        assert!(frame.add_entity(SceneEntity {
            model: SceneModel::World {
                presentation: presentation(&assets, id),
                world: id,
                surfaces: Span { first: 1, count: 1 }
            },
            ..instance
        }));
        assert!(frame.render_scene(
            Refdef::default(),
            &[],
            &assets,
            qa_render::shader::reference::ShaderView::direct()
        ));
        let packet = frame.finish();
        let Command::View(view) = packet.commands()[1] else {
            panic!()
        };
        assert_eq!(
            (
                view.scene.entities.count,
                view.scene.surfaces.count,
                view.scene.draws.count
            ),
            (1, 1, 1)
        );
        assert_eq!(packet.surface(0).entity, Some(0));
        assert_eq!(packet.rejected, 1);
    }
}

#[test]
fn bounded_patch_views_keep_prior_selection_after_failed_inline_submission() {
    let mut assets = Assets::load().unwrap();
    let materials = [SurfaceMaterial::default(); 2];
    let id = world_variants_with(&mut assets, &[&materials], |geometry| {
        geometry.vertices = [
            [1.0, -1.0, -1.0],
            [1.0, 0.0, -1.0],
            [1.0, 1.0, -1.0],
            [1.0, -1.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [1.0, -1.0, 1.0],
            [1.0, 0.0, 1.0],
            [1.0, 1.0, 1.0],
        ]
        .map(|position| WorldVertex {
            vertex: Vertex {
                position: Vec3(position),
                ..Default::default()
            },
        })
        .into();
        geometry.indices = vec![
            0, 3, 1, 1, 3, 4, 1, 4, 2, 2, 4, 5, 3, 6, 4, 4, 6, 7, 4, 7, 5, 5, 7, 8,
        ];
        geometry.boundaries = (0..8)
            .map(|triangle| Span {
                first: triangle * 3,
                count: 3,
            })
            .collect();
        for surface in &mut geometry.surfaces {
            surface.kind = GeometryKind::Patch;
            surface.vertices = Span { first: 0, count: 9 };
            surface.indices = Span {
                first: 0,
                count: 24,
            };
            surface.boundaries = Span { first: 0, count: 8 };
            surface.patch = Some(PatchGrid {
                control_vertices: surface.vertices,
                control_dimensions: [3, 3],
                dimensions: [3, 3],
                width_lod_error: vec![0.0, 4.0, 0.0].into_boxed_slice(),
                height_lod_error: vec![0.0, 4.0, 0.0].into_boxed_slice(),
                lod_origin: Vec3([1.0, 0.0, 0.0]),
                lod_radius: 0.0,
            });
        }
    });
    let mut frontend = FrontEnd::load(Limits {
        commands: 3,
        entities: 1,
        surfaces: 2,
        draws: 2,
        ..Default::default()
    })
    .unwrap();
    let mut frame = frontend.begin_frame([0; 4]).unwrap();
    assert!(frame.add_world(
        id,
        presentation(&assets, id),
        &[VisibleSurface {
            surface: 0,
            depth_key: 0
        }]
    ));
    assert!(frame.render_scene(
        Refdef {
            curve_error: 4.0,
            ..Default::default()
        },
        &[],
        &assets,
        qa_render::shader::reference::ShaderView::direct()
    ));
    assert!(!frame.add_entity(SceneEntity {
        model: SceneModel::World {
            world: id,
            presentation: presentation(&assets, id),
            surfaces: Span { first: 0, count: 2 }
        },
        ..Default::default()
    }));
    assert!(frame.add_world(
        id,
        presentation(&assets, id),
        &[VisibleSurface {
            surface: 1,
            depth_key: 0
        }]
    ));
    assert!(frame.render_scene(
        Refdef {
            curve_error: 0.0,
            ..Default::default()
        },
        &[],
        &assets,
        qa_render::shader::reference::ShaderView::direct()
    ));
    let packet = frame.finish();
    let mut counts = Vec::new();
    for command in packet.commands() {
        if let Command::View(view) = command {
            assert_eq!(view.scene.entities.count, 0);
            assert_eq!(view.scene.surfaces.count, 1);
            assert_eq!(view.scene.draws.count, 1);
            let reference = packet.surfaces(view.scene.surfaces)[0];
            let grid = assets.world(id).unwrap().geometry().surfaces[reference.surface as usize]
                .patch
                .as_ref()
                .unwrap();
            counts.push(reference.patch_lod.triangle_count(grid));
        }
    }
    assert_eq!(counts, [8, 2]);
    assert_eq!(packet.rejected, 1);
}
