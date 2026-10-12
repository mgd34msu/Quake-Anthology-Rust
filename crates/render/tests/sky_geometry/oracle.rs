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
            model,
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
