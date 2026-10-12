use super::*;

#[test]
fn shared_cloud_triangles_keep_native_face_cell_and_vertex_order() {
    let grid = CloudGrid::generate(CloudSphere {
        height: 512.0,
        radius: 4096.0,
    })
    .unwrap();
    let bounds = [FaceBounds {
        mins: [-0.37, -0.23],
        maxs: [0.41, 0.87],
    }; 6];
    let origin = Vec3([37.0, -17.0, 23.0]);
    let mut previous = Vec::new();
    for face in CubeFace::ALL {
        if face == CubeFace::NegativeZ {
            continue;
        }
        let Some([mins, maxs]) = bounds[face.index()].grid_bounds() else {
            continue;
        };
        for t in mins[1]..maxs[1] {
            for s in mins[0]..maxs[0] {
                for [s, t] in [
                    [s, t],
                    [s, t + 1],
                    [s + 1, t],
                    [s, t + 1],
                    [s + 1, t + 1],
                    [s + 1, t],
                ] {
                    let direction = cube_vertex(
                        face,
                        [(s as f32 - 4.0) / 4.0, (t as f32 - 4.0) / 4.0],
                        1024.0,
                        [0.0, 1.0],
                    )
                    .direction;
                    previous.push((
                        (origin + direction).0.map(f32::to_bits),
                        grid.uv[face.index()][t][s].map(f32::to_bits),
                    ));
                }
            }
        }
    }
    let actual = cloud_triangles(bounds, &grid, 1024.0, origin)
        .flatten()
        .map(|v| (v.position.0.map(f32::to_bits), v.texcoord.map(f32::to_bits)))
        .collect::<Vec<_>>();
    assert_eq!(cloud_triangle_count(bounds) * 3, actual.len());
    assert_eq!(cloud_triangle_count([FaceBounds::EMPTY; 6]), 0);
    assert_eq!(actual, previous);
}

#[test]
fn rotating_bounds_keep_native_full_cube_and_zero_rotation_visibility() {
    let bounds = [FaceBounds::EMPTY; 6];
    let rotation = Rotation {
        axis: Vec3([0.0, 0.0, 1.0]),
        degrees_per_second: 0.0,
    };
    assert_eq!(draw_bounds(bounds, Some(rotation)), bounds);
    assert_eq!(
        draw_bounds(
            bounds,
            Some(Rotation {
                degrees_per_second: 1.0,
                ..rotation
            })
        ),
        [FaceBounds {
            mins: [-1.0; 2],
            maxs: [1.0; 2]
        }; 6]
    );
}

#[test]
fn sky_batches_draw_once_per_view_and_discard_untouched_previous_bounds() {
    let mut batches = SkyBatches::new(2);
    let triangle = [
        Vec3([100.0, -10.0, -10.0]),
        Vec3([100.0, 10.0, -10.0]),
        Vec3([100.0, 0.0, 10.0]),
    ];
    batches.begin_frame();
    assert!(
        batches
            .clip_mut(0)
            .unwrap()
            .add_polygon(&triangle, Vec3::default())
    );
    assert!(batches.take_for_draw(0).is_some());
    assert!(batches.take_for_draw(0).is_none());
    assert!(batches.take_for_draw(1).is_none());
    assert!(batches.clip_mut(2).is_none());
    batches.begin_frame();
    assert!(batches.take_for_draw(0).is_none());
    assert!(
        batches
            .clip_mut(0)
            .unwrap()
            .bounds()
            .iter()
            .all(|bound| !bound.visible())
    );
    assert!(
        batches
            .clip_mut(0)
            .unwrap()
            .add_polygon(&triangle, Vec3::default())
    );
    assert!(batches.take_for_draw(0).is_some());
}

#[test]
fn sky_entity_triangles_use_the_same_selected_frame_pair_as_mesh_draws() {
    use crate::scene::{Command, FrontEnd, Limits, Refdef, SceneEntity, Viewport};

    let mut assets = Assets::load().unwrap();
    let old = [
        Vec3([8.0, -2.0, 1.0]),
        Vec3([8.0, 2.0, 1.0]),
        Vec3([8.0, 0.0, -2.0]),
    ];
    let current = old.map(|point| point + Vec3([4.0, 3.0, 2.0]));
    let vertices: Vec<_> = old
        .into_iter()
        .chain(current)
        .map(|position| Vertex {
            position,
            ..Vertex::default()
        })
        .collect();
    let model = assets
        .register_animated_model(&vertices, 3, &[0, 1, 2], MaterialId(0))
        .unwrap();
    let mut frontend = FrontEnd::load(Limits::default()).unwrap();
    for (frame, old_frame, back, valid) in [(1, 0, 0.25, true), (2, 0, 0.0, false)] {
        let entity = SceneEntity {
            model: model.into(),
            frame,
            old_frame,
            back_lerp: back,
            origin: Vec3([1.0, -2.0, 3.0]),
            ..SceneEntity::default()
        };
        let mut submission = frontend.begin_frame([0; 4]).unwrap();
        assert!(submission.add_entity(entity));
        assert!(submission.render_scene(
            Refdef {
                viewport: Viewport {
                    width: 32,
                    height: 24,
                    ..Viewport::default()
                },
                ..Refdef::default()
            },
            &[],
            &assets
        ));
        let list = submission.finish();
        let draws = list
            .commands()
            .iter()
            .find_map(|command| match command {
                Command::View(view) => Some(view.scene.draws),
                _ => None,
            })
            .unwrap();
        let mut triangles = Vec::new();
        let mut missing = 0;
        for_each_sky_triangle(&list, draws, &assets, |_, geometry| {
            match geometry {
                SkySubmission::Begin(_) => {}
                SkySubmission::Triangle(points) => triangles.push(points),
                SkySubmission::MissingSurface => missing += 1,
            }
            true
        });
        if valid {
            let expected = [0, 1, 2].map(|index| {
                math::transform_point(
                    entity.origin,
                    entity.axes,
                    old[index] * back + current[index] * (1.0 - back),
                )
            });
            assert_eq!(triangles.len(), 1);
            assert_eq!(
                triangles[0].map(|point| point.0.map(f32::to_bits)),
                expected.map(|point| point.0.map(f32::to_bits))
            );
            assert_eq!(missing, 0);
        } else {
            assert!(triangles.is_empty());
            assert_eq!(missing, 1);
        }
        assert!(frontend.recycle(list).is_ok());
    }
}

fn two_face_world(
    assets: &mut Assets,
    materials: &[crate::world::SurfaceMaterial; 2],
) -> (crate::world::WorldId, [[Vec3; 3]; 2]) {
    use crate::world::geometry::{
        GeometryKind, GeometryPartition, LightEncoding, LightSource, PatchStats,
        TextureCoordinates, WorldGeometry, WorldSurface, WorldVertex,
    };
    use qa_core::primitives::Bounds;
    use qa_world::visibility::{PvsRows, VisLeaf, VisibilityWorld};

    let points = [
        [
            Vec3([8.0, -2.0, -1.0]),
            Vec3([8.0, 2.0, -1.0]),
            Vec3([8.0, 0.0, 2.0]),
        ],
        [
            Vec3([16.0, -1.0, -1.0]),
            Vec3([16.0, 1.0, -1.0]),
            Vec3([16.0, 0.0, 1.0]),
        ],
    ];
    let bounds = points.map(|points| {
        let mut bounds = Bounds::empty();
        for point in points {
            bounds.add_point(point);
        }
        bounds
    });
    let mut world_bounds = Bounds::empty();
    for bounds in bounds {
        world_bounds.add_bounds(bounds);
    }
    let geometry = WorldGeometry {
        partition: GeometryPartition::SplitBsp,
        world_has_lightdata: true,
        vertices: points
            .into_iter()
            .flatten()
            .map(|position| WorldVertex {
                vertex: Vertex {
                    position,
                    ..Vertex::default()
                },
            })
            .collect(),
        indices: vec![0, 1, 2, 3, 4, 5],
        boundaries: vec![Span { first: 0, count: 3 }, Span { first: 3, count: 3 }],
        surfaces: (0..2)
            .map(|index| WorldSurface {
                source_id: index as u32,
                kind: GeometryKind::Triangles,
                vertices: Span {
                    first: index as u32 * 3,
                    count: 3,
                },
                indices: Span {
                    first: index as u32 * 3,
                    count: 3,
                },
                boundaries: Span {
                    first: index as u32,
                    count: 1,
                },
                plane: None,
                bounds: bounds[index],
                texture_coordinates: TextureCoordinates::Normalized,
                texture_projection: [[0.0; 4]; 2],
                texture_minima: [0; 2],
                texture_extents: [0; 2],
                lightmap_projection: None,
                lightmap_grid: [0; 2],
                styles: [255; 4],
                light_source: LightSource::Page(index as u32),
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
                source_lightmap: index as i32,
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
            bounds: world_bounds,
            surfaces: Span { first: 0, count: 2 },
        }],
        vec![0, 1],
        2,
        -1,
        PvsRows::all_visible(0),
    )
    .unwrap();
    (
        assets
            .register_world_with_bindings(geometry, visibility, materials)
            .unwrap(),
        points,
    )
}

#[test]
fn world_sky_faces_use_captured_instance_pose_and_keep_each_face_binding() {
    use crate::assets::MaterialSettings;
    use crate::scene::{Command, FrontEnd, Limits, Refdef, SceneEntity, SceneModel, Viewport};
    use crate::world::{SurfaceMaterial, VisibleSurface};

    let mut assets = Assets::load().unwrap();
    let materials = ["first-sky-face", "second-sky-face"].map(|name| {
        assets
            .register_material(name, &[Stage::default()], MaterialSettings::default())
            .unwrap()
    });
    let lightmaps = [[17, 0, 0, 255], [0, 0, 231, 255]]
        .map(|color| assets.register_image(1, 1, &color).unwrap());
    let bindings = std::array::from_fn(|index| SurfaceMaterial {
        material: materials[index],
        lightmap: lightmaps[index],
        texture_scale: [1.0 / (32 << index) as f32, 1.0 / (64 << index) as f32],
        ..SurfaceMaterial::default()
    });
    let (world, points) = two_face_world(&mut assets, &bindings);
    let mut frontend = FrontEnd::load(Limits::default()).unwrap();
    for material in [None, Some(materials[1])] {
        let mut submission = frontend.begin_frame([0; 4]).unwrap();
        let first_entity = SceneEntity {
            model: SceneModel::World {
                world,
                surfaces: Span { first: 1, count: 1 },
            },
            origin: Vec3([1.0, 0.0, 0.0]),
            ..SceneEntity::default()
        };
        let refdef = Refdef {
            viewport: Viewport {
                width: 32,
                height: 24,
                ..Viewport::default()
            },
            ..Refdef::default()
        };
        assert!(submission.add_entity(first_entity));
        assert!(submission.render_scene(refdef, &[], &assets));
        assert!(submission.add_world(world, &[VisibleSurface::default()]));
        let second_entity = SceneEntity {
            model: SceneModel::World {
                world,
                surfaces: Span { first: 0, count: 2 },
            },
            origin: Vec3([-5.0, 7.0, -2.0]),
            axes: [
                Vec3([0.0, 1.0, 0.0]),
                Vec3([-1.0, 0.0, 0.0]),
                Vec3([0.0, 0.0, 1.0]),
            ],
            material,
            ..SceneEntity::default()
        };
        assert!(submission.add_entity(second_entity));
        assert!(submission.render_scene(refdef, &[], &assets));
        let list = submission.finish();
        let Command::View(view) = list.commands()[2] else {
            panic!()
        };
        assert_eq!(view.scene.entities.first, 1);
        assert!(
            list.surfaces(view.scene.surfaces)
                .iter()
                .filter_map(|surface| surface.entity)
                .all(|index| index == 1)
        );
        let mut actual = Vec::new();
        for_each_sky_triangle(&list, view.scene.draws, &assets, |material, geometry| {
            match geometry {
                SkySubmission::Begin(_) => {}
                SkySubmission::Triangle(points) => {
                    actual.push((material.0, points.map(|point| point.0.map(f32::to_bits))))
                }
                SkySubmission::MissingSurface => {
                    panic!("registered surfaces must remain available")
                }
            }
            true
        });
        let mut expected = vec![(
            materials[0].0,
            points[0].map(|point| point.0.map(f32::to_bits)),
        )];
        for (index, points) in points.iter().enumerate() {
            expected.push((
                material.unwrap_or(materials[index]).0,
                points.map(|point| {
                    math::transform_point(second_entity.origin, second_entity.axes, point)
                        .0
                        .map(f32::to_bits)
                }),
            ));
            let binding = assets.surface_binding(world, index as u32).unwrap();
            assert_eq!(binding.material, bindings[index].material);
            assert_eq!(binding.lightmap, bindings[index].lightmap);
            assert_eq!(binding.texture_scale, bindings[index].texture_scale);
        }
        actual.sort();
        expected.sort();
        assert_eq!(actual, expected);
        assert!(frontend.recycle(list).is_ok());
    }
}
