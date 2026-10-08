use qa_core::primitives::{Bounds, Plane, Vec3};
use qa_formats::bsp::{
    Bsp, Face, IndexRange, Map, Shader, Surface, SurfaceKind, TextureInfo, Vertex,
};
use qa_render::world::geometry::{
    GeometryError, GeometryKind, GeometryOptions, LightEncoding, LightSource, TextureCoordinates,
    load_geometry,
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
    assert_eq!(face.light_source, LightSource::Samples);
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
