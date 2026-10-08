use qa_core::primitives::{Bounds, Vec3};
use qa_formats::bsp::IndexRange;
use qa_render::assets::{MaterialSettings, Stage};
use qa_render::scene::DrawKind;
use qa_render::shader::{AlphaGen, BlendFactor, RgbGen, StageBlend};
use qa_render::world::geometry::*;
use qa_render::world::{SurfaceMaterial, VisibleSurface, WorldId};
use qa_render::{Assets, Command, FrontEnd, Limits, MaterialId, Refdef, SceneEntity, Vertex};
use qa_world::visibility::{PvsRows, SurfaceSpan, VisLeaf, VisibilityWorld};

fn material(assets: &mut Assets, name: &str, sort: f32) -> MaterialId {
    assets
        .register_material(
            name,
            &[Stage::default()],
            MaterialSettings {
                sort,
                ..MaterialSettings::default()
            },
        )
        .unwrap()
}

fn alpha_material(assets: &mut Assets, name: &str, sort: f32) -> MaterialId {
    assets
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
        .unwrap()
}

fn poly_order(assets: &Assets, materials: &[MaterialId]) -> Vec<u32> {
    let mut front = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = front.begin_frame([0; 4]).unwrap();
    for (index, &material) in materials.iter().enumerate() {
        let vertices = [Vertex {
            color: [index as u8, 0, 0, 128],
            ..Vertex::default()
        }; 3];
        assert!(frame.add_poly(material, &vertices));
    }
    assert!(frame.render_scene(Refdef::default(), &[], assets));
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
    let bounds = Bounds {
        mins: Vec3([1.0, -1.0, -1.0]),
        maxs: Vec3([1.0, 1.0, 1.0]),
    };
    let geometry = WorldGeometry {
        partition: GeometryPartition::SplitBsp,
        world_has_lightdata: false,
        vertices: [[1.0, -1.0, -1.0], [1.0, 1.0, -1.0], [1.0, 0.0, 1.0]]
            .map(|position| WorldVertex {
                vertex: Vertex {
                    position: Vec3(position),
                    ..Vertex::default()
                },
                normal: Vec3([-1.0, 0.0, 0.0]),
            })
            .into(),
        indices: vec![0, 1, 2],
        boundaries: vec![IndexRange { first: 0, count: 3 }],
        surfaces: (0..materials.len())
            .map(|source| WorldSurface {
                source_id: source as u32,
                kind: GeometryKind::Triangles,
                vertices: IndexRange { first: 0, count: 3 },
                indices: IndexRange { first: 0, count: 3 },
                boundaries: IndexRange { first: 0, count: 1 },
                plane: None,
                bounds,
                texture_coordinates: TextureCoordinates::Normalized,
                texture_projection: [[0.0; 4]; 2],
                texture_minima: [0; 2],
                texture_extents: [0; 2],
                lightmap_grid: [0; 2],
                styles: [255; 4],
                light_source: LightSource::None,
                light_encoding: LightEncoding::Rgb,
                light_samples: IndexRange::default(),
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
    let visibility = VisibilityWorld::load(
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
    .unwrap();
    assets
        .register_world_with_bindings(geometry, visibility, materials)
        .unwrap()
}

#[test]
fn world_submissions_copy_surface_ids_and_keep_prior_views_on_overflow() {
    let mut assets = Assets::load();
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
    assert!(frame.add_world(first_world, &visible));
    visible[0].surface = 1;
    assert!(frame.render_scene(Refdef::default(), &[], &assets));
    assert!(!frame.add_world(WorldId(8), &[visible[0]; 2]));
    assert!(frame.add_world(second_world, &visible));
    assert!(frame.render_scene(Refdef::default(), &[], &assets));
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
    let assets = Assets::load();
    let mut front = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = front.begin_frame([0; 4]).unwrap();
    let first = SceneEntity {
        frame: 7,
        ..SceneEntity::default()
    };
    assert!(frame.add_entity(first));
    let mut vertices = [Vertex::default(); 3];
    assert!(frame.add_poly(MaterialId(0), &vertices));
    let mut areas = [0x55u8, 0xaa];
    assert!(frame.render_scene(Refdef::default(), &areas, &assets));
    vertices[0].color = [0; 4];
    assert_eq!(vertices[0].color, [0; 4]);
    areas.fill(0);
    frame.clear_scene();
    assert!(frame.add_entity(SceneEntity {
        frame: 11,
        ..SceneEntity::default()
    }));
    assert!(frame.render_scene(Refdef::default(), &[], &assets));
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

#[test]
fn rejected_poly_and_view_leave_existing_payloads_intact() {
    let assets = Assets::load();
    let limits = Limits {
        commands: 3,
        entities: 1,
        polys: 1,
        vertices: 3,
        lights: 1,
        area_bytes: 2,
        surfaces: 1,
        draws: 0,
    };
    let mut front = FrontEnd::load(limits).unwrap();
    let mut frame = front.begin_frame([0; 4]).unwrap();
    assert!(!frame.add_poly(MaterialId(0), &[Vertex::default(); 4]));
    assert!(frame.add_poly(MaterialId(0), &[Vertex::default(); 3]));
    assert!(!frame.render_scene(Refdef::default(), &[0; 3], &assets));
    assert!(frame.render_scene(Refdef::default(), &[0; 2], &assets));
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
    let mut assets = Assets::load();
    let translucent = alpha_material(&mut assets, "translucent", 8.25);
    let opaque = material(&mut assets, "opaque", 3.0);
    let middle = material(&mut assets, "middle", 5.0);
    let world = world(
        &mut assets,
        &[SurfaceMaterial {
            material: translucent,
            ..SurfaceMaterial::default()
        }],
    );
    let mut front = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = front.begin_frame([0; 4]).unwrap();
    assert!(frame.add_world(
        world,
        &[VisibleSurface {
            surface: 0,
            depth_key: 7
        }]
    ));
    assert!(frame.add_entity(SceneEntity {
        material: Some(opaque),
        ..SceneEntity::default()
    }));
    assert!(frame.add_poly(middle, &[Vertex::default(); 3]));
    assert!(frame.render_scene(Refdef::default(), &[], &assets));
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
    assert_eq!(packet.entity(draws[0].index).material, Some(opaque));
    assert_eq!(packet.poly(draws[1].index).material, middle);
}

#[test]
fn shared_draw_order_retains_fractional_sorts_and_numeric_material_ties() {
    let mut assets = Assets::load();
    let later = material(&mut assets, "later", 3.75);
    let earlier = material(&mut assets, "earlier", 3.25);
    let tied_first = material(&mut assets, "tied-first", 3.25);
    let tied_second = material(&mut assets, "tied-second", 3.25);
    let mut front = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = front.begin_frame([0; 4]).unwrap();
    for material in [later, tied_second, earlier, tied_first] {
        assert!(frame.add_poly(material, &[Vertex::default(); 3]));
    }
    assert!(frame.render_scene(Refdef::default(), &[], &assets));
    let packet = frame.finish();
    let Command::View(view) = packet.commands()[1] else {
        panic!()
    };
    assert_eq!(
        packet
            .draws(view.scene.draws)
            .iter()
            .map(|draw| packet.poly(draw.index).material)
            .collect::<Vec<_>>(),
        [earlier, tied_first, tied_second, later]
    );
}

#[test]
fn equal_native_keys_keep_native_cross_kind_shortsort_swaps() {
    let mut assets = Assets::load();
    let material = alpha_material(&mut assets, "same-alpha", 8.0);
    let world = world(
        &mut assets,
        &[SurfaceMaterial {
            material,
            ..SurfaceMaterial::default()
        }],
    );
    let mut front = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = front.begin_frame([0; 4]).unwrap();
    assert!(frame.add_poly(material, &[Vertex::default(); 3]));
    assert!(frame.add_world(
        world,
        &[VisibleSurface {
            surface: 0,
            depth_key: 1
        }]
    ));
    assert!(frame.add_poly(material, &[Vertex::default(); 3]));
    assert!(frame.render_scene(Refdef::default(), &[], &assets));
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
    let mut assets = Assets::load();
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
    let mut assets = Assets::load();
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
    let mut assets = Assets::load();
    let material = material(&mut assets, "lightmapped", 3.0);
    let image = assets.register_image(1, 1, &[128; 4]).unwrap();
    let world = world(
        &mut assets,
        &[
            SurfaceMaterial {
                material,
                lightmap: image,
                ..SurfaceMaterial::default()
            },
            SurfaceMaterial {
                material,
                ..SurfaceMaterial::default()
            },
        ],
    );
    let mut front = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = front.begin_frame([0; 4]).unwrap();
    assert!(frame.add_world(
        world,
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
    assert!(frame.render_scene(Refdef::default(), &[], &assets));
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
    let mut assets = Assets::load();
    let material = material(&mut assets, "coincident", 3.0);
    let world = world(
        &mut assets,
        &[SurfaceMaterial {
            material,
            ..SurfaceMaterial::default()
        }; 3],
    );
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
    assert!(frame.add_world(world, &surfaces));
    assert!(frame.render_scene(Refdef::default(), &[], &assets));
    assert!(frame.add_world(world, &surfaces[..2]));
    assert!(frame.render_scene(Refdef::default(), &[], &assets));
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
    let mut assets = Assets::load();
    let late = material(&mut assets, "late", 7.0);
    let early = material(&mut assets, "early", 2.0);
    let mut front = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = front.begin_frame([0; 4]).unwrap();
    let vertices = [Vertex::default(); 3];
    for material in [late, early] {
        assert!(frame.add_poly(material, &vertices));
    }
    assert!(frame.render_scene(Refdef::default(), &[1], &assets));
    frame.clear_scene();
    for material in [late, early] {
        assert!(frame.add_poly(material, &vertices));
    }
    assert!(frame.render_scene(Refdef::default(), &[2], &assets));
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
    let assets = Assets::load();
    let mut front = FrontEnd::load(Limits {
        draws: 2,
        ..Limits::default()
    })
    .unwrap();
    let mut frame = front.begin_frame([0; 4]).unwrap();
    assert!(frame.add_entity(SceneEntity::default()));
    assert!(!frame.add_world(WorldId(99), &[VisibleSurface::default(); 2]));
    assert!(frame.add_poly(MaterialId(0), &[Vertex::default(); 3]));
    assert!(!frame.add_entity(SceneEntity::default()));
    assert!(frame.render_scene(Refdef::default(), &[], &assets));
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
    let assets = Assets::load();
    let mut front = FrontEnd::load(Limits {
        commands: 3,
        area_bytes: 1,
        ..Limits::default()
    })
    .unwrap();
    let mut frame = front.begin_frame([0; 4]).unwrap();
    assert!(frame.add_poly(MaterialId(0), &[Vertex::default(); 3]));
    assert!(!frame.render_scene(Refdef::default(), &[0; 2], &assets));
    assert!(frame.add_poly(MaterialId(0), &[Vertex::default(); 3]));
    assert!(frame.render_scene(Refdef::default(), &[7], &assets));
    assert!(frame.add_poly(MaterialId(0), &[Vertex::default(); 3]));
    assert!(!frame.render_scene(Refdef::default(), &[8], &assets));
    assert!(frame.render_scene(Refdef::default(), &[], &assets));
    assert!(!frame.render_scene(Refdef::default(), &[], &assets));
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
    let assets = Assets::load();
    let mut front = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = front.begin_frame([0; 4]).unwrap();
    assert!(frame.add_poly(MaterialId(999), &[Vertex::default(); 3]));
    assert!(frame.add_entity(SceneEntity {
        model: qa_render::ModelId(999),
        ..SceneEntity::default()
    }));
    assert!(frame.add_world(WorldId(999), &[VisibleSurface::default()]));
    assert!(frame.add_poly(MaterialId(0), &[Vertex::default(); 3]));
    assert!(frame.render_scene(Refdef::default(), &[], &assets));
    assert!(frame.add_poly(MaterialId(0), &[Vertex::default(); 3]));
    assert!(frame.render_scene(Refdef::default(), &[], &assets));
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
    let mut assets = Assets::load();
    let positive = material(&mut assets, "positive-zero", 0.0);
    let negative = material(&mut assets, "negative-zero", -0.0);
    let mut front = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = front.begin_frame([0; 4]).unwrap();
    assert!(frame.add_poly(negative, &[Vertex::default(); 3]));
    assert!(frame.add_poly(positive, &[Vertex::default(); 3]));
    assert!(frame.render_scene(Refdef::default(), &[], &assets));
    let packet = frame.finish();
    let Command::View(view) = packet.commands()[1] else {
        panic!()
    };
    assert_eq!(
        packet
            .draws(view.scene.draws)
            .iter()
            .map(|draw| packet.poly(draw.index).material)
            .collect::<Vec<_>>(),
        [positive, negative]
    );
}

#[test]
fn automatic_draw_capacity_is_checked_before_allocation() {
    assert!(FrontEnd::load(Limits {
        entities: u32::MAX as usize,
        ..Limits::default()
    })
    .is_err());
    assert!(FrontEnd::load(Limits {
        surfaces: usize::MAX,
        ..Limits::default()
    })
    .is_err());
}
