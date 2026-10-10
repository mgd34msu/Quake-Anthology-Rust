use qa_core::primitives::{Bounds, Plane, Vec3};
use qa_formats::bsp::{
    Bsp, Face, IndexRange, Map, Shader, Surface, SurfaceKind, TextureInfo, Vertex,
};
use qa_render::world::geometry::{
    GeometryError, GeometryKind, GeometryOptions, LightEncoding, LightSource, PatchGrid,
    TextureCoordinates, WorldGeometry, WorldVertex, load_geometry, prepare_patches,
};

fn bsp_bytes(family: u8, lighting: &[u8]) -> Vec<u8> {
    let (magic, count, light_lump) = match family {
        1 => (29u32.to_le_bytes().to_vec(), 15, 8),
        2 => ([b"IBSP".as_slice(), &38u32.to_le_bytes()].concat(), 19, 7),
        _ => ([b"IBSP".as_slice(), &46u32.to_le_bytes()].concat(), 17, 14),
    };
    let header = magic.len() + count * 8;
    let mut bytes = vec![0; header];
    bytes[..magic.len()].copy_from_slice(&magic);
    for index in 0..count {
        let at = magic.len() + index * 8;
        bytes[at..at + 4].copy_from_slice(&(header as u32).to_le_bytes());
        if index == light_lump {
            bytes[at + 4..at + 8].copy_from_slice(&(lighting.len() as u32).to_le_bytes());
        }
    }
    bytes.extend_from_slice(lighting);
    bytes
}
fn empty_map(bytes: &[u8]) -> Map<'_> {
    Map {
        bsp: Bsp::parse(bytes).unwrap(),
        planes: vec![],
        vertices: vec![],
        nodes: vec![],
        leaves: vec![],
        edges: vec![],
        surface_edges: vec![],
        faces: vec![],
        leaf_faces: vec![],
        leaf_brushes: vec![],
        clipnodes: vec![],
        texture_info: vec![],
        textures: vec![],
        models: vec![],
        brushes: vec![],
        brush_sides: vec![],
        shaders: vec![],
        fogs: vec![],
        surfaces: vec![],
        indices: vec![],
        areas: vec![],
        area_portals: vec![],
        extensions: vec![],
    }
}
fn vertex(position: [f32; 3]) -> Vertex {
    Vertex {
        position: Vec3(position),
        texcoord: [0.0; 2],
        lightmap_coord: [0.0; 2],
        normal: Vec3([0.0, 0.0, 1.0]),
        color: [255; 4],
    }
}
fn legacy_map(bytes: &[u8]) -> Map<'_> {
    let mut map = empty_map(bytes);
    map.planes.push(Plane {
        encoding: None,
        normal: Vec3([0.0, 0.0, 1.0]),
        distance: 2.0,
        axis: None,
    });
    map.vertices = [
        [-1.0, 3.0, 2.0],
        [17.0, 3.0, 2.0],
        [17.0, 34.0, 2.0],
        [-1.0, 34.0, 2.0],
    ]
    .map(vertex)
    .to_vec();
    // Negative surfedges use endpoint one, including the closing polygon edge.
    map.edges = vec![[0, 1], [2, 1], [2, 3], [0, 3]];
    map.surface_edges = vec![0, -1, 2, -3];
    map.texture_info.push(TextureInfo {
        projection: [[1.0, 0.0, 0.0, -1.0], [0.0, 1.0, 0.0, 1.0]],
        flags: 0,
        texture: 3,
        value: 0,
        next: -1,
        name: &[],
    });
    map.faces.push(Face {
        plane: 0,
        flags: 1,
        edges: IndexRange { first: 0, count: 4 },
        texture_info: 0,
        styles: [0, 7, 255, 255],
        lighting_offset: 2,
    });
    map
}

#[test]
fn signed_edges_plane_side_extents_and_full_style_spans_follow_qsrc() {
    let lighting: Vec<_> = [vec![250, 251], (0..24).collect()].concat();
    let bytes = bsp_bytes(1, &lighting);
    let geometry = load_geometry(&legacy_map(&bytes), GeometryOptions::default()).unwrap();
    let face = &geometry.surfaces[0];
    assert_eq!(face.source_id, 0);
    assert_eq!(face.kind, GeometryKind::Polygon);
    assert_eq!(face.texture_coordinates, TextureCoordinates::Texels);
    assert_eq!(face.texture_minima, [-16, 0]);
    assert_eq!(face.texture_extents, [32, 48]);
    assert_eq!(face.lightmap_grid, [3, 4]);
    assert_eq!(face.styles, [0, 7, 255, 255]);
    assert_eq!(face.plane.unwrap().normal, Vec3([0.0, 0.0, -1.0]));
    assert_eq!(face.plane.unwrap().distance, -2.0);
    assert_eq!(face.source_texture, Some(3));
    assert_eq!(face.light_encoding, LightEncoding::Luminance);
    assert_eq!(face.light_source, LightSource::Samples);
    assert_eq!(face.light_samples.count, 24);
    assert_eq!(
        geometry.light_samples,
        (0..24).map(|v| [v; 3]).collect::<Vec<_>>()
    );
    assert_eq!(
        &geometry.indices[face.indices.indices()],
        &[0, 1, 2, 0, 2, 3]
    );
    let boundary = geometry.boundaries[face.boundaries.first as usize];
    assert_eq!(&geometry.indices[boundary.indices()], &[0, 1, 2, 3]);
    assert_eq!(geometry.vertices[1].vertex.position, Vec3([17.0, 3.0, 2.0]));
    assert_eq!(
        face.bounds,
        Bounds {
            mins: Vec3([-1.0, 3.0, 2.0]),
            maxs: Vec3([17.0, 34.0, 2.0])
        }
    );
}

#[test]
fn q2_rgb_is_retained_and_last_style_cannot_read_past_lighting() {
    let lighting = [vec![250, 251], (0..72).collect()].concat();
    let bytes = bsp_bytes(2, &lighting);
    let geometry = load_geometry(&legacy_map(&bytes), GeometryOptions::default()).unwrap();
    assert_eq!(geometry.surfaces[0].light_encoding, LightEncoding::Rgb);
    assert_eq!(geometry.light_samples[0], [0, 1, 2]);
    assert_eq!(geometry.light_samples[23], [69, 70, 71]);
    let truncated = bsp_bytes(2, &lighting[..lighting.len() - 1]);
    assert!(matches!(
        load_geometry(&legacy_map(&truncated), GeometryOptions::default()),
        Err(GeometryError::LightSpan(0))
    ));
    let truncated = bsp_bytes(1, &[0; 25]);
    assert!(matches!(
        load_geometry(&legacy_map(&truncated), GeometryOptions::default()),
        Err(GeometryError::LightSpan(0))
    ));
}

fn q3_surface(kind: SurfaceKind, first: u32, count: u32) -> Surface<'static> {
    Surface {
        kind,
        shader: Some(0),
        shader_name: &[],
        brush_side: -1,
        fog: -1,
        vertices: IndexRange { first, count },
        indices: IndexRange::default(),
        lightmap: -3,
        lightmap_rect: [0; 4],
        lightmap_origin: Vec3::default(),
        lightmap_vectors: [
            Vec3::default(),
            Vec3([128.0, 128.0, 64.0]),
            Vec3([0.0, 0.0, 1.0]),
        ],
        patch: [0; 2],
        triangle_fan: false,
    }
}
fn q3_map(bytes: &[u8]) -> Map<'_> {
    let mut map = empty_map(bytes);
    map.shaders.push(Shader {
        name: b"textures/test",
        surface_flags: 0,
        content_flags: 0,
    });
    map
}

#[test]
fn q3_preserves_source_surface_ids_page_coordinates_colors_and_flare_metadata() {
    let mut page = vec![0; 128 * 128 * 3];
    page[..3].copy_from_slice(&[5, 7, 9]);
    let bytes = bsp_bytes(3, &page);
    let mut map = q3_map(&bytes);
    map.vertices = [[0.0, 0.0, 0.0], [32.0, 0.0, 0.0], [0.0, 32.0, 0.0]]
        .map(vertex)
        .to_vec();
    map.vertices[0].texcoord = [0.2, 0.3];
    map.vertices[0].lightmap_coord = [0.1, 0.8];
    map.vertices[0].color = [5, 6, 7, 8];
    map.indices = vec![2, 0, 1];
    let mut face = q3_surface(SurfaceKind::Planar, 0, 3);
    face.indices = IndexRange { first: 0, count: 3 };
    face.lightmap = 0;
    face.lightmap_rect = [10, 20, 30, 40];
    let mut flare = q3_surface(SurfaceKind::Flare, 0, 0);
    flare.lightmap_origin = Vec3([-2.0, 3.0, 4.0]);
    map.surfaces = vec![face, flare];
    let geometry = load_geometry(&map, GeometryOptions::default()).unwrap();
    assert_eq!(
        geometry
            .surfaces
            .iter()
            .map(|s| s.source_id)
            .collect::<Vec<_>>(),
        vec![0, 1]
    );
    let face = &geometry.surfaces[0];
    assert_eq!(face.light_source, LightSource::Page(0));
    assert_eq!(face.light_samples.count, 128 * 128);
    assert_eq!(face.lightmap_rect, [10, 20, 30, 40]);
    assert_eq!(geometry.light_samples[0], [5, 7, 9]);
    assert_eq!(geometry.vertices[0].vertex.texcoord, [0.2, 0.3]);
    assert_eq!(geometry.vertices[0].vertex.lightmap_coord, [0.1, 0.8]);
    assert_eq!(geometry.vertices[0].vertex.color, [5, 6, 7, 8]);
    assert_eq!(&geometry.indices[face.indices.indices()], &[2, 0, 1]);
    assert_eq!(geometry.surfaces[1].kind, GeometryKind::Flare);
    assert_eq!(geometry.surfaces[1].indices.count, 0);
    assert_eq!(geometry.surfaces[1].bounds.mins, Vec3([-2.0, 3.0, 4.0]));
}

fn curved_patch(bytes: &[u8], height: f32) -> Map<'_> {
    let mut map = q3_map(bytes);
    for row in 0..3 {
        for column in 0..3 {
            let mut v = vertex([
                column as f32 * 64.0,
                row as f32 * 64.0,
                if column == 1 { height } else { 0.0 },
            ]);
            v.texcoord = [column as f32 * 0.5, row as f32 * 0.5];
            v.lightmap_coord = v.texcoord;
            map.vertices.push(v);
        }
    }
    let mut surface = q3_surface(SurfaceKind::Patch, 0, 9);
    surface.patch = [3, 3];
    map.surfaces.push(surface);
    map
}

#[test]
fn q3_patch_uses_native_curve_deviation_collinear_pruning_and_triangle_order() {
    // tr_curve.c's distance-from-chord test repeatedly subdivides this arch.
    // PutPointsOnCurve gives the exact binary-fraction samples below.
    let bytes = bsp_bytes(3, &[]);
    let map = curved_patch(&bytes, 64.0);
    let geometry = load_geometry(&map, GeometryOptions::default()).unwrap();
    let face = &geometry.surfaces[0];
    let grid = face.patch.as_ref().unwrap();
    assert_eq!(grid.dimensions, [9, 2]);
    assert_eq!(grid.control_dimensions, [3, 3]);
    assert_eq!(grid.control_vertices, IndexRange { first: 0, count: 9 });
    assert_eq!(face.vertices.count, 18);
    assert_eq!(face.indices.count, 48);
    let vertices = &geometry.vertices[face.vertices.indices()];
    let expected = [0.0, 14.0, 24.0, 30.0, 32.0, 30.0, 24.0, 14.0, 0.0];
    for column in 0..9 {
        assert_eq!(
            vertices[column].vertex.position,
            Vec3([column as f32 * 16.0, 0.0, expected[column]])
        );
        assert_eq!(
            vertices[column].vertex.texcoord,
            [column as f32 * 0.125, 0.0]
        );
        assert_eq!(vertices[column + 9].vertex.position.0[1], 128.0);
        assert!(vertices[column].normal.0[2] > 0.5);
    }
    let base = face.vertices.first;
    assert_eq!(
        &geometry.indices[face.indices.first as usize..face.indices.first as usize + 6],
        &[base, base + 9, base + 1, base + 1, base + 9, base + 10]
    );
    let coarse = load_geometry(
        &map,
        GeometryOptions {
            patch_subdivisions: 100.0,
            ..GeometryOptions::default()
        },
    )
    .unwrap();
    assert_eq!(
        coarse.surfaces[0].patch.as_ref().unwrap().dimensions,
        [3, 2]
    );
    let flat = load_geometry(&curved_patch(&bytes, 0.0), GeometryOptions::default()).unwrap();
    assert_eq!(flat.surfaces[0].patch.as_ref().unwrap().dimensions, [2, 2]);
}

#[test]
fn q3_color_shift_precedes_interpolation_and_nodraw_keeps_its_source_id() {
    let bytes = bsp_bytes(3, &[]);
    let mut map = curved_patch(&bytes, 64.0);
    for (index, v) in map.vertices.iter_mut().enumerate() {
        v.color = if index % 3 == 1 {
            [200, 100, 50, 255]
        } else {
            [0, 0, 0, 255]
        };
        v.lightmap_coord = [f32::NAN; 2];
    }
    let geometry = load_geometry(
        &map,
        GeometryOptions {
            vertex_color_shift: 1,
            ..GeometryOptions::default()
        },
    )
    .unwrap();
    assert_eq!(geometry.vertices[1].vertex.color, [255, 127, 63, 255]);
    let center = geometry.surfaces[0].vertices.first as usize + 4;
    assert_eq!(geometry.vertices[center].vertex.color, [127, 63, 31, 255]);
    assert_eq!(geometry.vertices[center].vertex.lightmap_coord, [0.0; 2]);
    map.shaders[0].surface_flags = 0x80;
    let no_draw = load_geometry(&map, GeometryOptions::default()).unwrap();
    assert_eq!(no_draw.surfaces.len(), 1);
    assert_eq!(no_draw.surfaces[0].source_id, 0);
    assert!(no_draw.surfaces[0].no_draw);
    assert_eq!(no_draw.surfaces[0].indices.count, 0);
    assert!(no_draw.surfaces[0].patch.is_some());
}

#[test]
fn geometry_rejects_invalid_load_dimensions_and_nonfinite_projection() {
    let bytes = bsp_bytes(1, &[]);
    let mut map = legacy_map(&bytes);
    assert!(matches!(
        load_geometry(
            &map,
            GeometryOptions {
                max_surface_vertices: 3,
                ..GeometryOptions::default()
            }
        ),
        Err(GeometryError::TooManySurfaceVertices(0))
    ));
    map.texture_info[0].projection[0][3] = f32::INFINITY;
    assert!(matches!(
        load_geometry(&map, GeometryOptions::default()),
        Err(GeometryError::NonFinite(_, 0))
    ));
    let bytes = bsp_bytes(3, &[]);
    let mut map = curved_patch(&bytes, 64.0);
    map.surfaces[0].patch = [4, 3];
    assert!(matches!(
        load_geometry(&map, GeometryOptions::default()),
        Err(GeometryError::InvalidPatch(0))
    ));
}

#[derive(Clone)]
struct FixtureGrid {
    width: usize,
    height: usize,
    vertices: Vec<WorldVertex>,
    width_errors: Vec<f32>,
    height_errors: Vec<f32>,
    origin: Vec3,
    radius: f32,
}
fn fixture_grid(
    width: usize,
    height: usize,
    point: impl Fn(usize, usize) -> [f32; 3],
) -> FixtureGrid {
    FixtureGrid {
        width,
        height,
        vertices: (0..height)
            .flat_map(|y| (0..width).map(move |x| (x, y)))
            .map(|(x, y)| WorldVertex {
                vertex: qa_render::assets::Vertex {
                    position: Vec3(point(x, y)),
                    normal: Vec3([0.0, 0.0, 1.0]),
                    texcoord: [x as f32 * 0.25, y as f32 * 0.125],
                    lightmap_coord: [x as f32 * 0.0625, y as f32 * 0.03125],
                    color: [(x * 31 + y * 17) as u8, (x * 19 + y * 7) as u8, 123, 255],
                },
                normal: Vec3([0.0, 0.0, 1.0]),
            })
            .collect(),
        width_errors: (0..width).map(|x| x as f32 * 0.125).collect(),
        height_errors: (0..height).map(|y| y as f32 * 0.25).collect(),
        origin: Vec3::default(),
        radius: 100.0,
    }
}
fn fixture_geometry(grids: &[FixtureGrid], curved: bool, tolerance: f32) -> WorldGeometry {
    let bytes = bsp_bytes(3, &[]);
    let mut map = q3_map(&bytes);
    for grid in grids {
        let first = map.vertices.len() as u32;
        map.vertices.extend(grid.vertices.iter().map(|v| Vertex {
            position: v.vertex.position,
            texcoord: v.vertex.texcoord,
            lightmap_coord: v.vertex.lightmap_coord,
            normal: v.normal,
            color: v.vertex.color,
        }));
        let mut surface = q3_surface(
            if curved {
                SurfaceKind::Patch
            } else {
                SurfaceKind::Planar
            },
            first,
            grid.vertices.len() as u32,
        );
        surface.patch = [grid.width as i32, grid.height as i32];
        surface.lightmap_vectors[0] = Vec3([-100.0, 0.0, 0.0]);
        surface.lightmap_vectors[1] = Vec3([100.0, 0.0, 0.0]);
        map.surfaces.push(surface);
    }
    let mut geometry = load_geometry(
        &map,
        GeometryOptions {
            patch_subdivisions: tolerance,
            ..GeometryOptions::default()
        },
    )
    .unwrap();
    if !curved {
        for (surface, grid) in geometry.surfaces.iter_mut().zip(grids) {
            surface.kind = GeometryKind::Patch;
            surface.patch = Some(PatchGrid {
                control_vertices: surface.vertices,
                control_dimensions: [grid.width as u16, grid.height as u16],
                dimensions: [grid.width as u16, grid.height as u16],
                width_lod_error: grid.width_errors.clone().into_boxed_slice(),
                height_lod_error: grid.height_errors.clone().into_boxed_slice(),
                lod_origin: grid.origin,
                lod_radius: grid.radius,
            });
        }
        geometry.patch_stats = prepare_patches(&mut geometry, GeometryOptions::default()).unwrap();
    }
    geometry
}
fn seam_grids(reverse: bool, rotated: bool) -> Vec<FixtureGrid> {
    let fine = fixture_grid(3, 2, |x, y| {
        [
            (if reverse { 2 - x } else { x }) as f32 * 32.0,
            y as f32 * 16.0,
            if x == 1 && y == 0 { 8.0 } else { 0.0 },
        ]
    });
    let coarse = if rotated {
        fixture_grid(2, 2, |x, y| [y as f32 * 64.0, -(x as f32) * 16.0, 0.0])
    } else {
        fixture_grid(2, 2, |x, y| [x as f32 * 64.0, -(y as f32) * 16.0, 0.0])
    };
    vec![fine, coarse]
}

#[test]
fn native_seam_insertion_retains_controls_ids_and_triangle_boundaries() {
    for (reverse, rotated) in [(false, false), (true, false), (false, true), (true, true)] {
        let geometry = fixture_geometry(&seam_grids(reverse, rotated), false, 4.0);
        assert_eq!(geometry.patch_stats.insertions, 1);
        assert_eq!(
            geometry.patch_stats.reverse_endpoint_fixes,
            usize::from(reverse)
        );
        let surface = &geometry.surfaces[1];
        assert_eq!(surface.source_id, 1);
        let patch = surface.patch.as_ref().unwrap();
        assert_eq!(patch.control_dimensions, [2, 2]);
        assert_eq!(patch.control_vertices, IndexRange { first: 6, count: 4 });
        assert_eq!(patch.dimensions, if rotated { [2, 3] } else { [3, 2] });
        assert_eq!(surface.indices.count, 12);
        assert_eq!(surface.boundaries.count, 4);
        let midpoint =
            &geometry.vertices[(surface.vertices.first + if rotated { 2 } else { 1 }) as usize];
        assert_eq!(midpoint.vertex.position, Vec3([32.0, 0.0, 8.0]));
        assert_eq!(
            midpoint.vertex.texcoord,
            if rotated { [0.0, 0.0625] } else { [0.125, 0.0] }
        );
    }
}

#[test]
fn patch_lod_groups_exclude_different_bounds_merged_edges_and_inline_owners() {
    let grids = seam_grids(false, false);
    let mut separate = grids.clone();
    separate[1].radius += 1.0;
    assert_eq!(
        fixture_geometry(&separate, false, 4.0)
            .patch_stats
            .insertions,
        0
    );
    let mut geometry = fixture_geometry(&separate, false, 4.0);
    geometry.surfaces[1].patch.as_mut().unwrap().lod_radius = 100.0;
    geometry.models = vec![
        qa_render::world::geometry::WorldModel {
            bounds: geometry.surfaces[0].bounds,
            origin: Vec3::default(),
            surfaces: IndexRange { first: 0, count: 1 },
        },
        qa_render::world::geometry::WorldModel {
            bounds: geometry.surfaces[1].bounds,
            origin: Vec3::default(),
            surfaces: IndexRange { first: 1, count: 1 },
        },
    ];
    assert_eq!(
        prepare_patches(&mut geometry, GeometryOptions::default())
            .unwrap()
            .insertions,
        0
    );
    let mut merged = fixture_grid(5, 2, |x, y| {
        [[0.0, 32.0, 32.0, 48.0, 64.0][x], y as f32 * 16.0, 0.0]
    });
    merged.width_errors.fill(0.5);
    let mut shared = merged.clone();
    shared.width_errors.fill(0.75);
    let geometry = fixture_geometry(&[merged, shared], false, 4.0);
    assert_eq!(geometry.patch_stats.lod_copies, 0);
    assert_eq!(
        &*geometry.surfaces[1].patch.as_ref().unwrap().width_lod_error,
        &[0.75; 5]
    );
}

// tools/check_patch.py asks this test to export exactly the same control/raw
// grids and resulting meshes for extracted original Q3 functions to consume.
#[test]
fn original_patch_comparison_fixtures() {
    use std::fmt::Write;
    let mut cases = vec![];
    for (name, reverse, rotated) in [
        ("forward_width", false, false),
        ("reverse_width", true, false),
        ("forward_height", false, true),
        ("reverse_height", true, true),
    ] {
        cases.push((name.to_owned(), false, 4.0, seam_grids(reverse, rotated)));
    }
    let mut separated = seam_grids(false, false);
    separated[1].radius = 101.0;
    cases.push(("different_group".to_owned(), false, 4.0, separated));
    let chain: Vec<_> = (0..4)
        .map(|i| {
            let mut grid = fixture_grid(3, 3, |x, y| {
                [x as f32 * 32.0, (y + i * 2) as f32 * 32.0, 0.0]
            });
            grid.width_errors.fill((i + 1) as f32 * 0.125);
            grid
        })
        .collect();
    cases.push(("recursive_group".to_owned(), false, 4.0, chain));
    let merged = fixture_grid(5, 2, |x, y| {
        [[0.0, 32.0, 32.0, 48.0, 64.0][x], y as f32 * 16.0, 0.0]
    });
    cases.push((
        "merged_edge".to_owned(),
        false,
        4.0,
        vec![merged.clone(), merged],
    ));
    for (name, amount, tolerance) in [
        ("flat", 0.0, 4.0),
        ("arch", 64.0, 4.0),
        ("coarse_arch", 64.0, 100.0),
    ] {
        cases.push((
            name.to_owned(),
            true,
            tolerance,
            vec![fixture_grid(3, 3, |x, y| {
                [
                    x as f32 * 64.0,
                    y as f32 * 64.0,
                    if x == 1 { amount } else { 0.0 },
                ]
            })],
        ));
    }
    for seed in 0..48u32 {
        let width = 3 + (seed % 3) as usize * 2;
        let height = 3 + ((seed / 3) % 3) as usize * 2;
        let mut state = seed + 1;
        let offsets: Vec<_> = (0..width * height)
            .map(|_| {
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                ((state >> 24) as i32 - 128) as f32 * 0.5
            })
            .collect();
        cases.push((
            format!("seed_{seed}"),
            true,
            4.0,
            vec![fixture_grid(width, height, |x, y| {
                [x as f32 * 32.0, y as f32 * 32.0, offsets[y * width + x]]
            })],
        ));
    }
    let mut input = String::new();
    let mut output = String::new();
    for (name, curved, tolerance, grids) in cases {
        writeln!(
            input,
            "CASE {name} {} {tolerance} {}",
            u8::from(curved),
            grids.len()
        )
        .unwrap();
        for grid in &grids {
            writeln!(
                input,
                "GRID {} {} {} {} {} {}",
                grid.width,
                grid.height,
                grid.origin.0[0],
                grid.origin.0[1],
                grid.origin.0[2],
                grid.radius
            )
            .unwrap();
            for errors in [&grid.width_errors, &grid.height_errors] {
                write!(input, "E").unwrap();
                for error in errors {
                    write!(input, " {error}").unwrap();
                }
                input.push('\n');
            }
            for v in &grid.vertices {
                write!(input, "V").unwrap();
                for value in v
                    .vertex
                    .position
                    .0
                    .into_iter()
                    .chain(v.normal.0)
                    .chain(v.vertex.texcoord)
                    .chain(v.vertex.lightmap_coord)
                {
                    write!(input, " {value}").unwrap();
                }
                for value in v.vertex.color {
                    write!(input, " {value}").unwrap();
                }
                input.push('\n');
            }
        }
        let geometry = fixture_geometry(&grids, curved, tolerance);
        writeln!(output, "CASE {name} {}", geometry.surfaces.len()).unwrap();
        writeln!(
            output,
            "FIX {}",
            geometry.patch_stats.reverse_endpoint_fixes
        )
        .unwrap();
        for surface in &geometry.surfaces {
            let patch = surface.patch.as_ref().unwrap();
            writeln!(
                output,
                "GRID {} {} {}",
                surface.source_id, patch.dimensions[0], patch.dimensions[1]
            )
            .unwrap();
            for errors in [&patch.width_lod_error, &patch.height_lod_error] {
                write!(output, "E").unwrap();
                for error in errors.iter() {
                    write!(output, " {:08x}", error.to_bits()).unwrap();
                }
                output.push('\n');
            }
            for v in &geometry.vertices[surface.vertices.indices()] {
                write!(output, "V").unwrap();
                for value in v
                    .vertex
                    .position
                    .0
                    .into_iter()
                    .chain(v.normal.0)
                    .chain(v.vertex.texcoord)
                    .chain(v.vertex.lightmap_coord)
                {
                    write!(output, " {:08x}", value.to_bits()).unwrap();
                }
                for value in v.vertex.color {
                    write!(output, " {value}").unwrap();
                }
                output.push('\n');
            }
        }
    }
    if let Ok(path) = std::env::var("QA_PATCH_INPUT") {
        std::fs::write(path, input).unwrap();
    }
    if let Ok(path) = std::env::var("QA_PATCH_OUTPUT") {
        std::fs::write(path, output).unwrap();
    }
}
