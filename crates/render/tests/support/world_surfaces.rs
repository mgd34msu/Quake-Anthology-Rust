use qa_core::primitives::{Bounds, Vec3};
use qa_render::{
    Vertex,
    scene::Span,
    world::geometry::{
        GeometryKind, GeometryPartition, LightEncoding, LightSource, PatchStats,
        TextureCoordinates, WorldGeometry, WorldSurface, WorldVertex,
    },
};
use qa_world::visibility::{PvsRows, SurfaceSpan, VisLeaf, VisibilityWorld};
pub fn inputs(count: usize) -> (WorldGeometry, VisibilityWorld) {
    let bounds = Bounds {
        mins: Vec3([1.0, -1.0, -1.0]),
        maxs: Vec3([1.0, 1.0, 1.0]),
    };
    let geometry = WorldGeometry {
        partition: GeometryPartition::SplitBsp,
        world_has_lightdata: true,
        vertices: [[1.0, -1.0, -1.0], [1.0, 1.0, -1.0], [1.0, 0.0, 1.0]]
            .map(|position| WorldVertex {
                vertex: Vertex {
                    normal: Vec3([-1.0, 0.0, 0.0]),
                    position: Vec3(position),
                    lightmap_coord: [0.25, 0.75],
                    ..Vertex::default()
                },
            })
            .into(),
        indices: vec![0, 1, 2],
        boundaries: vec![Span { first: 0, count: 3 }],
        surfaces: (0..count)
            .map(|index| WorldSurface {
                source_id: index as u32,
                kind: GeometryKind::Triangles,
                vertices: Span { first: 0, count: 3 },
                indices: Span { first: 0, count: 3 },
                boundaries: Span { first: 0, count: 1 },
                plane: None,
                bounds,
                texture_coordinates: TextureCoordinates::Normalized,
                texture_projection: [[0.0; 4]; 2],
                texture_minima: [0; 2],
                texture_extents: [16; 2],
                lightmap_projection: None,
                lightmap_grid: [2; 2],
                styles: [0, 255, 255, 255],
                light_source: LightSource::Page(0),
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
                source_lightmap: 0,
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
                count: count as u32,
            },
        }],
        (0..count as u32).collect(),
        count,
        -1,
        PvsRows::all_visible(0),
    )
    .unwrap();
    (geometry, visibility)
}
