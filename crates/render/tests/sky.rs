use qa_core::primitives::Vec3;
use qa_render::sky::{
    CLOUD_GRID_SIZE, CloudGrid, CloudSphere, CubeFace, FaceBounds, LAYER_SIZE, LayeredSphere,
    Rotation, SkyClip, cloud_uv, cube_sample, cube_vertex, layered_uv, split_layered_sky, unrotate,
};

fn close(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() <= 0.000002,
        "{actual} != {expected}"
    );
}

fn close_uv(actual: [f32; 2], expected: [f32; 2]) {
    close(actual[0], expected[0]);
    close(actual[1], expected[1]);
}

// WinQuake/gl_warp.c EmitSkyPolys: Z*3, 6*63/length, modulo-128 8/16 scrolling.
#[test]
fn layered_sphere_uses_native_projection_and_scroll_periods() {
    let sphere = LayeredSphere::NATIVE;
    assert!(sphere.valid());
    assert_eq!(sphere.flatten_z, 3.0);
    assert_eq!(sphere.projected_scale, 378.0);
    assert_eq!(sphere.texture_size, 128.0);
    assert_eq!(sphere.scroll_speeds, [8.0, 16.0]);
    let horizontal = Vec3([1.0, 0.0, 0.0]);
    assert_eq!(
        layered_uv(horizontal, 0.0, sphere, 0),
        Some([2.953125, 0.0])
    );
    assert_eq!(
        layered_uv(horizontal, 1.0, sphere, 0),
        Some([3.015625, 0.0625])
    );
    assert_eq!(
        layered_uv(horizontal, 1.0, sphere, 1),
        Some([3.078125, 0.125])
    );
    assert_eq!(
        layered_uv(horizontal, 16.0, sphere, 0),
        layered_uv(horizontal, 0.0, sphere, 0)
    );
    assert_eq!(
        layered_uv(horizontal, 8.0, sphere, 1),
        layered_uv(horizontal, 0.0, sphere, 1)
    );
    assert_eq!(
        layered_uv(Vec3([0.0, 0.0, 1.0]), 1.0, sphere, 0),
        Some([0.0625, 0.0625])
    );
    let diagonal = Vec3([1.0, 0.0, 1.0]);
    close_uv(
        layered_uv(diagonal, 0.0, sphere, 0).unwrap(),
        [378.0 / 10.0_f32.sqrt() / 128.0, 0.0],
    );
    close_uv(
        layered_uv(diagonal * 17.0, 0.0, sphere, 0).unwrap(),
        layered_uv(diagonal, 0.0, sphere, 0).unwrap(),
    );
}

#[test]
fn layered_scroll_preserves_native_negative_fractional_time() {
    // Native casts before masking: (int)-0.5 is 0; (int)-1.5 & ~127 is -128.
    let up = Vec3([0.0, 0.0, 1.0]);
    assert_eq!(
        layered_uv(up, -0.0625, LayeredSphere::NATIVE, 0),
        Some([-0.00390625; 2])
    );
    assert_eq!(
        layered_uv(up, -0.1875, LayeredSphere::NATIVE, 0),
        Some([0.98828125; 2])
    );
    assert!(layered_uv(Vec3([0.0; 3]), 0.0, LayeredSphere::NATIVE, 0).is_none());
    assert!(layered_uv(up, f32::NAN, LayeredSphere::NATIVE, 0).is_none());
    assert!(layered_uv(up, 0.0, LayeredSphere::NATIVE, 2).is_none());
    assert!(
        !LayeredSphere {
            texture_size: 0.0,
            ..LayeredSphere::NATIVE
        }
        .valid()
    );
}

// Q2 gl_warp.c and Q3 tr_sky.c st_to_vec/vec_to_st and sky_texorder.
#[test]
fn cube_faces_keep_native_direction_texture_orientation() {
    let directions = [
        Vec3([2.0, -0.5, -1.0]),
        Vec3([-2.0, 0.5, -1.0]),
        Vec3([0.5, 2.0, -1.0]),
        Vec3([-0.5, -2.0, -1.0]),
        Vec3([1.0, -0.5, 2.0]),
        Vec3([-1.0, -0.5, -2.0]),
    ];
    let suffixes = ["rt", "lf", "bk", "ft", "up", "dn"];
    for face in CubeFace::ALL {
        let vertex = cube_vertex(face, [0.25, -0.5], 2.0, [0.0, 1.0]);
        assert_eq!(vertex.direction, directions[face.index()]);
        assert_eq!(vertex.uv, [0.625, 0.75]);
        let sample = cube_sample(vertex.direction, [0.0, 1.0]).unwrap();
        assert_eq!(sample.face, face);
        assert_eq!(sample.uv, vertex.uv);
        assert_eq!(face.suffix(), suffixes[face.index()]);
        assert_eq!(
            cube_sample(
                cube_vertex(face, [0.0; 2], 1.0, [0.0, 1.0]).direction,
                [0.0, 1.0]
            )
            .unwrap()
            .uv,
            [0.5; 2]
        );
    }
}

#[test]
fn cube_seams_and_exact_edge_rays_have_bounded_coordinates() {
    let seam = [1.0 / 512.0, 511.0 / 512.0];
    let vertex = cube_vertex(CubeFace::PositiveX, [-1.0, 1.0], 1.0, seam);
    assert_eq!(vertex.uv, [1.0 / 512.0; 2]);
    assert_eq!(
        cube_vertex(CubeFace::PositiveX, [-1.0, 1.0], 1.0, [0.0, 1.0]).uv,
        [0.0; 2]
    );
    let corner = cube_sample(Vec3([1.0, 1.0, 0.0]), [0.0, 1.0]).unwrap();
    assert_eq!(corner.face, CubeFace::PositiveX);
    assert_eq!(corner.uv, [0.0, 0.5]);
    assert!(cube_sample(Vec3([0.0; 3]), [0.0, 1.0]).is_none());
    assert!(cube_sample(Vec3([f32::INFINITY, 0.0, 1.0]), [0.0, 1.0]).is_none());
    assert!(cube_sample(Vec3([1.0, 0.0, 0.0]), [1.0, 0.0]).is_none());
}

// Q3 R_InitSkyTexCoords intersects a cloud sphere around (0,0,-4096).
#[test]
fn cloud_sphere_matches_native_axis_intersections_and_metadata() {
    let sphere = CloudSphere::native(256.0);
    assert_eq!(sphere.radius, 4096.0);
    assert_eq!(sphere.height, 256.0);
    let half_pi = std::f32::consts::FRAC_PI_2;
    close_uv(
        cloud_uv(Vec3([0.0, 0.0, 1.0]), sphere).unwrap(),
        [half_pi; 2],
    );
    close_uv(
        cloud_uv(Vec3([0.0, 0.0, -1.0]), sphere).unwrap(),
        [half_pi; 2],
    );
    let x = (2.0 * sphere.radius * sphere.height + sphere.height * sphere.height).sqrt()
        / (sphere.radius + sphere.height);
    close_uv(
        cloud_uv(Vec3([1.0, 0.0, 0.0]), sphere).unwrap(),
        [x.acos(), half_pi],
    );
    close_uv(
        cloud_uv(Vec3([-1.0, 0.0, 0.0]), sphere).unwrap(),
        [std::f32::consts::PI - x.acos(), half_pi],
    );
    close_uv(
        cloud_uv(Vec3([5.0, 3.0, 7.0]), sphere).unwrap(),
        cloud_uv(Vec3([15.0, 9.0, 21.0]), sphere).unwrap(),
    );
    assert!(cloud_uv(Vec3([0.0; 3]), sphere).is_none());
    assert!(cloud_uv(Vec3([1.0, 0.0, 0.0]), CloudSphere::native(0.0)).is_none());
}

#[test]
fn cloud_lookup_is_the_native_six_face_nine_by_nine_grid() {
    let sphere = CloudSphere::native(512.0);
    let grid = CloudGrid::generate(sphere).unwrap();
    assert_eq!(CLOUD_GRID_SIZE, 9);
    assert_eq!(grid.sphere, sphere);
    let distance = 1024.0 / 1.75;
    for face in CubeFace::ALL {
        for t in 0..CLOUD_GRID_SIZE {
            for s in 0..CLOUD_GRID_SIZE {
                let st = [(s as f32 - 4.0) / 4.0, (t as f32 - 4.0) / 4.0];
                let direction = cube_vertex(face, st, distance, [0.0, 1.0]).direction;
                close_uv(
                    grid.uv[face.index()][t][s],
                    cloud_uv(direction, sphere).unwrap(),
                );
                assert!(grid.intersection[face.index()][t][s].is_finite());
                assert!(grid.intersection[face.index()][t][s] > 0.0);
            }
        }
    }
    close(
        grid.intersection[CubeFace::PositiveZ.index()][4][4],
        sphere.height / distance,
    );
    close(
        grid.intersection[CubeFace::NegativeZ.index()][4][4],
        (2.0 * sphere.radius + sphere.height) / distance,
    );
    assert!(CloudGrid::generate(CloudSphere::native(0.0)).is_err());
}

#[test]
fn sky_rotation_uses_inverse_normalized_axis_for_ray_sampling() {
    let rotation = Rotation {
        axis: Vec3([0.0, 0.0, 1.0]),
        degrees_per_second: 90.0,
    };
    let direction = unrotate(Vec3([1.0, 0.0, 0.0]), rotation, 1.0);
    close(direction.0[0], 0.0);
    close(direction.0[1], -1.0);
    close(direction.0[2], 0.0);
    assert_eq!(
        unrotate(Vec3([1.0, 2.0, 3.0]), rotation, 0.0),
        Vec3([1.0, 2.0, 3.0])
    );
}

// WinQuake R_InitSky splits the 256x128 image: right solid, left index-zero mask.
#[test]
fn layered_image_split_keeps_indices_and_native_mask_fringe_rgb() {
    let palette = std::array::from_fn(|index| {
        [
            index as u8,
            (index / 2) as u8,
            (255 - index) as u8,
            if index == 255 { 0 } else { 255 },
        ]
    });
    let mut indices = vec![0_u8; LAYER_SIZE * LAYER_SIZE * 2];
    for y in 0..LAYER_SIZE {
        for x in 0..LAYER_SIZE {
            indices[y * LAYER_SIZE * 2 + x + LAYER_SIZE] = if x % 2 == 0 { 2 } else { 4 };
        }
    }
    indices[0] = 7;
    let images = split_layered_sky(&indices, &palette).unwrap();
    assert_eq!(images.average_rgb, [3, 1, 252]);
    assert_eq!(&images.opaque_indices[..2], &[2, 4]);
    assert_eq!(&images.masked_indices[..2], &[7, 0]);
    assert_eq!(&images.opaque_rgba[..8], &[2, 1, 253, 255, 4, 2, 251, 255]);
    assert_eq!(&images.masked_rgba[..8], &[7, 3, 248, 255, 3, 1, 252, 0]);
    assert_eq!(images.opaque_indices.len(), LAYER_SIZE * LAYER_SIZE);
    assert_eq!(images.masked_rgba.len(), LAYER_SIZE * LAYER_SIZE * 4);
    assert!(split_layered_sky(&indices[..indices.len() - 1], &palette).is_err());
}

#[test]
fn sky_clip_bounds_are_view_relative_and_use_native_face_projection() {
    let vertices = [
        Vec3([10.0, -5.0, -5.0]),
        Vec3([10.0, 5.0, -5.0]),
        Vec3([10.0, 5.0, 5.0]),
        Vec3([10.0, -5.0, 5.0]),
    ];
    let mut clip = SkyClip::new();
    assert!(clip.add_polygon(&vertices, Vec3([0.0; 3])));
    assert_eq!(
        clip.bounds()[CubeFace::PositiveX.index()],
        FaceBounds {
            mins: [-0.5; 2],
            maxs: [0.5; 2]
        }
    );
    assert_eq!(
        clip.bounds()
            .iter()
            .filter(|bounds| bounds.visible())
            .count(),
        1
    );
    let origin = Vec3([23.0, -7.0, 19.0]);
    let translated = vertices.map(|vertex| vertex + origin);
    let mut translated_clip = SkyClip::new();
    assert!(translated_clip.add_polygon(&translated, origin));
    assert_eq!(translated_clip.bounds(), clip.bounds());
}

#[test]
fn sky_clip_splits_one_polygon_across_two_cube_faces() {
    let vertices = [
        Vec3([10.0, 0.0, -2.0]),
        Vec3([10.0, 20.0, -2.0]),
        Vec3([10.0, 20.0, 2.0]),
        Vec3([10.0, 0.0, 2.0]),
    ];
    let mut clip = SkyClip::new();
    assert!(clip.add_polygon(&vertices, Vec3([0.0; 3])));
    let x = clip.bounds()[CubeFace::PositiveX.index()];
    close_uv(x.mins, [-1.0, -0.2]);
    close_uv(x.maxs, [0.0, 0.2]);
    let y = clip.bounds()[CubeFace::PositiveY.index()];
    close_uv(y.mins, [0.5, -0.2]);
    close_uv(y.maxs, [1.0, 0.2]);
    assert_eq!(
        clip.bounds()
            .iter()
            .filter(|bounds| bounds.visible())
            .count(),
        2
    );
}

#[test]
fn sky_clip_overflow_is_atomic_and_clear_reuses_fixed_scratch() {
    let vertices = [
        Vec3([10.0, -2.0, -2.0]),
        Vec3([10.0, 2.0, -2.0]),
        Vec3([10.0, 0.0, 2.0]),
    ];
    let mut clip = SkyClip::new();
    assert!(clip.add_polygon(&vertices, Vec3([0.0; 3])));
    let before = *clip.bounds();
    let oversized = [Vec3([10.0, 0.0, 0.0]); 63];
    assert!(!clip.add_polygon(&oversized, Vec3([0.0; 3])));
    assert_eq!(*clip.bounds(), before);
    let mut invalid = vertices;
    invalid[0].0[0] = f32::NAN;
    assert!(!clip.add_polygon(&invalid, Vec3([0.0; 3])));
    assert_eq!(*clip.bounds(), before);
    assert_eq!(clip.rejected, 2);
    clip.clear();
    assert_eq!(*clip.bounds(), [FaceBounds::EMPTY; 6]);
    assert_eq!(clip.rejected, 0);
    assert!(clip.add_polygon(&vertices, Vec3([0.0; 3])));
    assert_eq!(*clip.bounds(), before);
}

#[test]
fn face_grid_bounds_round_outward_and_clamp_to_native_subdivisions() {
    assert_eq!(
        FaceBounds {
            mins: [-0.31, -0.01],
            maxs: [0.26, 0.51]
        }
        .grid_bounds(),
        Some([[2, 3], [6, 7]])
    );
    assert_eq!(
        FaceBounds {
            mins: [-2.0; 2],
            maxs: [2.0; 2]
        }
        .grid_bounds(),
        Some([[0; 2], [8; 2]])
    );
    assert_eq!(FaceBounds::EMPTY.grid_bounds(), None);
    assert_eq!(
        FaceBounds {
            mins: [0.0; 2],
            maxs: [0.0; 2]
        }
        .grid_bounds(),
        None
    );
}

#[test]
fn layered_foreground_preserves_gl_palette_alpha_without_changing_cpu_indices() {
    let mut indices = vec![1; 256 * 128];
    indices[0] = 255;
    indices[1] = 0;
    let raw = [[10, 20, 30, 255]; 256];
    let cpu = split_layered_sky(&indices, &raw).unwrap();
    let mut gl_palette = raw;
    gl_palette[255][3] = 0;
    let gl = split_layered_sky(&indices, &gl_palette).unwrap();
    assert_eq!(&cpu.masked_rgba[..8], &[10, 20, 30, 255, 10, 20, 30, 0]);
    assert_eq!(&gl.masked_rgba[..8], &[10, 20, 30, 0, 10, 20, 30, 0]);
    assert_eq!(cpu.masked_indices, gl.masked_indices);
    assert_eq!(cpu.opaque_indices, gl.opaque_indices);
    assert_eq!(cpu.opaque_rgba, gl.opaque_rgba);
}
