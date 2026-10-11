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
