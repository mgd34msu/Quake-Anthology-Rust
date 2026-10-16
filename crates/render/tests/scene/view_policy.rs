use super::*;
use qa_core::primitives::Plane;
use qa_render::{
    Viewport,
    scene::{SceneParticle, ViewPolicy},
    shader::reference::ShaderView,
};
use qa_world::visibility::VisNode;

#[test]
fn pvs_and_area_controls_are_independent_and_nocull_keeps_the_portal_plane() {
    let mut assets = Assets::load().unwrap();
    let bounds = Bounds {
        mins: Vec3([-20., -1., -1.]),
        maxs: Vec3([16., 1., 1.]),
    };
    let leaf_bounds = [
        Bounds {
            mins: Vec3([1., -1., -1.]),
            maxs: Vec3([1., 1., 1.]),
        },
        Bounds {
            mins: Vec3([-5., -1., -1.]),
            maxs: Vec3([-5., 1., 1.]),
        },
        Bounds {
            mins: Vec3([-15., -1., -1.]),
            maxs: Vec3([-15., 1., 1.]),
        },
    ];
    let visibility = VisibilityWorld::load(
        vec![
            Plane::oriented(Vec3([1., 0., 0.]), 0.),
            Plane::oriented(Vec3([1., 0., 0.]), -10.),
        ],
        vec![
            VisNode {
                plane: 0,
                children: [-1, 1],
                bounds,
                surfaces: Span::default(),
            },
            VisNode {
                plane: 1,
                children: [-2, -3],
                bounds,
                surfaces: Span::default(),
            },
        ],
        leaf_bounds
            .into_iter()
            .enumerate()
            .map(|(index, bounds)| VisLeaf {
                selector: Some(index as u32),
                area: Some(index as u32),
                solid: false,
                bounds,
                surfaces: Span {
                    first: index as u32,
                    count: 1,
                },
            })
            .collect(),
        vec![0, 1, 2],
        3,
        0,
        PvsRows::load(
            vec![Some(0), Some(1), Some(2)],
            vec![0b011, 0b010, 0b100],
            None,
        )
        .unwrap(),
    )
    .unwrap();
    let id = world_variants_visibility(
        &mut assets,
        &[&[SurfaceMaterial::default(); 3]],
        |geometry| {
            geometry.vertices.clear();
            geometry.indices.clear();
            geometry.boundaries.clear();
            for (index, x) in [1., -5., -15.].into_iter().enumerate() {
                let first = index as u32 * 3;
                geometry
                    .vertices
                    .extend(
                        [[x, -1., -1.], [x, 1., -1.], [x, 0., 1.]].map(|point| WorldVertex {
                            vertex: Vertex {
                                position: Vec3(point),
                                ..Default::default()
                            },
                        }),
                    );
                geometry.indices.extend([first, first + 1, first + 2]);
                geometry.boundaries.push(Span { first, count: 3 });
                let surface = &mut geometry.surfaces[index];
                surface.vertices = Span { first, count: 3 };
                surface.indices = Span { first, count: 3 };
                surface.boundaries = Span {
                    first: index as u32,
                    count: 1,
                };
                surface.bounds = Bounds {
                    mins: Vec3([x, -1., -1.]),
                    maxs: Vec3([x, 1., 1.]),
                };
            }
        },
        Some(visibility),
    );
    let world = assets.world(id).unwrap();
    let mut scratch = qa_render::world::WorldView::load(world);
    let base = Refdef {
        viewport: Viewport {
            width: 16,
            height: 16,
            ..Default::default()
        },
        origin: Vec3([5., 0., 0.]),
        axes: [Vec3([-1., 0., 0.]), Vec3([0., -1., 0.]), Vec3([0., 0., 1.])],
        near: 0.25,
        far: 64.,
        fov: [90.; 2],
        ..Default::default()
    };
    let mut selected = |view, secondary| {
        let mut ids = scratch
            .query(world, view, secondary, &[0b010])
            .unwrap()
            .iter()
            .map(|surface| surface.surface)
            .collect::<Vec<_>>();
        ids.sort_unstable();
        ids
    };
    assert_eq!(selected(base, None), [0]);
    let all_pvs = Refdef {
        policy: ViewPolicy {
            ignore_pvs: true,
            ..Default::default()
        },
        ..base
    };
    assert_eq!(selected(all_pvs, Some(1)), [0, 2]);
    let all_areas = Refdef {
        policy: ViewPolicy {
            ignore_areas: true,
            ..all_pvs.policy
        },
        ..all_pvs
    };
    assert_eq!(selected(all_areas, None), [0, 1, 2]);
    let behind = Refdef {
        axes: Refdef::default().axes,
        policy: all_areas.policy,
        ..base
    };
    assert!(selected(behind, None).is_empty());
    let no_cull = Refdef {
        policy: ViewPolicy {
            cull_bounds: false,
            ..behind.policy
        },
        ..behind
    };
    assert_eq!(selected(no_cull, None), [0, 1, 2]);
    assert_eq!(
        selected(
            Refdef {
                clip_plane: Some(Plane::oriented(Vec3([1., 0., 0.]), -10.)),
                ..no_cull
            },
            None
        ),
        [0, 1]
    );
    assert!(
        selected(
            Refdef {
                policy: ViewPolicy {
                    draw_world: false,
                    ..no_cull.policy
                },
                ..no_cull
            },
            None
        )
        .is_empty()
    );
    assert_eq!(selected(base, None), [0]);
}

#[test]
fn draw_filters_preserve_inline_ownership_and_independent_scene_inputs() {
    let mut assets = Assets::load().unwrap();
    assets
        .register_palette(
            qa_render::surface_cache::PaletteLighting::load(&[255; 768], &[0; 64 * 256], None, 256)
                .unwrap(),
        )
        .unwrap();
    let id = world(&mut assets, &[SurfaceMaterial::default()]);
    let presentation = presentation(&assets, id);
    let mesh = assets
        .register_model(&[Vertex::default(); 3], &[0, 1, 2], ShaderBindingId(0))
        .unwrap();
    let mut front = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = front.begin_frame([0; 4]).unwrap();
    for (draw_world, draw_entities) in [(true, true), (false, true), (true, false), (false, false)]
    {
        frame.clear_scene();
        assert!(frame.add_world(
            id,
            presentation,
            &[VisibleSurface {
                surface: 0,
                depth_key: 0
            }]
        ));
        assert!(frame.add_entity(SceneEntity {
            model: SceneModel::World {
                world: id,
                presentation,
                surfaces: Span { first: 0, count: 1 }
            },
            ..Default::default()
        }));
        assert!(frame.add_entity(SceneEntity {
            model: SceneModel::Sprite {
                radius: 1.,
                rotation: 0.
            },
            shader: Some(ShaderBindingId(0).into()),
            ..Default::default()
        }));
        assert!(frame.add_entity(SceneEntity {
            model: SceneModel::Mesh(mesh),
            ..Default::default()
        }));
        assert!(frame.add_entity(SceneEntity {
            model: SceneModel::Portal {
                camera: Vec3([1., 0., 0.]),
                rotation: 0
            },
            ..Default::default()
        }));
        assert!(frame.add_poly(ShaderBindingId(0).into(), &[Vertex::default(); 3]));
        assert!(frame.add_particle(SceneParticle::default()));
        assert!(frame.render_scene(
            Refdef {
                policy: ViewPolicy {
                    draw_world,
                    draw_entities,
                    ..Default::default()
                },
                ..Default::default()
            },
            &[],
            &assets,
            ShaderView::direct()
        ));
        assert!(frame.draw_2d(Default::default()));
    }
    let packet = frame.finish();
    let views = packet
        .commands()
        .iter()
        .filter_map(|command| match command {
            Command::View(view) => Some(view),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(views.len(), 4);
    for (index, view) in views.iter().enumerate() {
        let (world, entities) = [(true, true), (false, true), (true, false), (false, false)][index];
        let draws = packet.draws(view.scene.draws);
        assert_eq!(
            draws
                .iter()
                .filter(|draw| draw.kind == DrawKind::Surface
                    && packet.surface(draw.index).entity.is_none())
                .count(),
            usize::from(world)
        );
        assert_eq!(
            draws
                .iter()
                .filter(|draw| draw.kind == DrawKind::Surface
                    && packet.surface(draw.index).entity.is_some())
                .count(),
            usize::from(entities)
        );
        assert_eq!(
            draws
                .iter()
                .filter(
                    |draw| draw.kind == DrawKind::Poly && packet.poly(draw.index).entity.is_some()
                )
                .count(),
            if entities { 2 } else { 0 }
        );
        assert_eq!(
            draws
                .iter()
                .filter(
                    |draw| draw.kind == DrawKind::Poly && packet.poly(draw.index).entity.is_none()
                )
                .count(),
            1
        );
        assert_eq!(
            draws
                .iter()
                .filter(|draw| draw.kind == DrawKind::Entity)
                .count(),
            usize::from(entities)
        );
        assert_eq!(view.scene.entities.count, 4);
        assert!(matches!(
            packet.entities(view.scene.entities)[3].model,
            SceneModel::Portal { .. }
        ));
        assert_eq!(view.scene.surfaces.count, 2);
        assert_eq!(view.scene.particles.count, 1);
    }
    assert_eq!(
        packet
            .commands()
            .iter()
            .filter(|command| matches!(command, Command::Draw2d(_)))
            .count(),
        4
    );
    assert_eq!(packet.rejected, 0);
}

#[test]
fn nocull_does_not_disable_cpu_near_or_portal_clipping_and_views_keep_their_planes() {
    let mut assets = Assets::load().unwrap();
    let material = assets
        .register_material(
            "clip witness",
            &[Stage::default()],
            MaterialSettings {
                cull: qa_render::shader::Cull::None,
                ..Default::default()
            },
        )
        .unwrap();
    let shader = assets
        .register_shader_binding(qa_render::ShaderBinding::material(
            material,
            LightSource::None,
        ))
        .unwrap();
    let mut front = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = front.begin_frame([0, 0, 0, 255]).unwrap();
    for (index, near, clip_plane) in [
        (0, 1., None),
        (1, 3., None),
        (2, 1., Some(Plane::oriented(Vec3([1., 0., 0.]), 3.))),
    ] {
        assert!(frame.add_poly(
            shader.into(),
            &[[2., -1., -1.], [2., 1., -1.], [2., 0., 1.]].map(|point| Vertex {
                position: Vec3(point),
                ..Default::default()
            })
        ));
        assert!(frame.render_scene(
            Refdef {
                viewport: Viewport {
                    x: index * 8,
                    y: 0,
                    width: 8,
                    height: 8
                },
                near,
                clip_plane,
                policy: ViewPolicy {
                    cull_bounds: false,
                    ..Default::default()
                },
                fov: [90.; 2],
                ..Default::default()
            },
            &[],
            &assets,
            ShaderView::direct()
        ));
    }
    let packet = frame.finish();
    let mut cpu = qa_render::cpu::CpuBackend::load_with_assets(24, 8, &assets).unwrap();
    let stats = cpu.render(&packet, &assets);
    assert_eq!(stats.views, 3);
    assert_eq!(stats.rejected, 0);
    let clear = u32::from_le_bytes([0, 0, 0, 255]);
    assert!(
        cpu.pixels()
            .iter()
            .enumerate()
            .any(|(index, &pixel)| index % 24 < 8 && pixel != clear)
    );
    assert!(
        cpu.pixels()
            .iter()
            .enumerate()
            .all(|(index, &pixel)| index % 24 < 8 || pixel == clear)
    );
    let views = packet
        .commands()
        .iter()
        .filter_map(|command| match command {
            Command::View(view) => Some(view),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        [
            views[0].refdef.near,
            views[1].refdef.near,
            views[2].refdef.near
        ],
        [1., 3., 1.]
    );
    assert!(views[0].refdef.clip_plane.is_none());
    assert!(views[2].refdef.clip_plane.is_some());
}
