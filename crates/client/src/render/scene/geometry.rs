//! Brush face reconstruction and water subdivision (donor
//! `src/render/scene/geometry.ts`, from `gl_rsurf.c`/`gl_warp.c`).
//!
//! Brush inputs are local owned structs rather than `qa-content` BSP shapes,
//! so scene code stays decoupled from the content decoders.

use qa_core::math::{dot3, scale3, vec2, vec3, Bounds, Plane, Vec2, Vec3, Vec4};

use crate::materials::geometry::{MaterialGeometry, MaterialVertex};
use crate::materials::lighting::{
    lightmap_atlas_coordinates, BspLighting, DecoupledLightmap, LightmapFace, LightmapProjection, TextureProjection,
};
use crate::render::error::RenderError;

/// Bounds over material vertex positions; empty input yields zero bounds.
#[must_use]
pub fn geometry_bounds(vertices: &[MaterialVertex]) -> Bounds {
    if vertices.is_empty() {
        return Bounds {
            min: vec3(0.0, 0.0, 0.0),
            max: vec3(0.0, 0.0, 0.0),
        };
    }
    let mut min = vec3(f32::INFINITY, f32::INFINITY, f32::INFINITY);
    let mut max = vec3(f32::NEG_INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY);
    for vertex in vertices {
        min.x = min.x.min(vertex.position.x);
        min.y = min.y.min(vertex.position.y);
        min.z = min.z.min(vertex.position.z);
        max.x = max.x.max(vertex.position.x);
        max.y = max.y.max(vertex.position.y);
        max.z = max.z.max(vertex.position.z);
    }
    Bounds { min, max }
}

/// Texture projection axes with offsets for one brush surface.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BrushTextureInfo {
    /// S axis plus offset in `w`.
    pub projection_s: Vec4,
    /// T axis plus offset in `w`.
    pub projection_t: Vec4,
}

/// One brush face referencing shared map arrays.
#[derive(Debug, Clone, PartialEq)]
pub struct BrushFace {
    /// Texture info index.
    pub texture_info: usize,
    /// Plane index.
    pub plane: usize,
    /// Whether the face uses the flipped plane.
    pub back: bool,
    /// First surface edge.
    pub first_edge: usize,
    /// Surface edge count.
    pub edge_count: usize,
    /// Lighting sample offset, if lit.
    pub lighting_offset: Option<i32>,
    /// Light styles.
    pub styles: Vec<u8>,
}

/// Shared brush arrays plus lighting and decoupled mappings.
#[derive(Debug, Clone, PartialEq)]
pub struct BrushMapData {
    /// Texture projections.
    pub texture_info: Vec<BrushTextureInfo>,
    /// Face planes.
    pub planes: Vec<Plane>,
    /// Signed edge indices forming face loops.
    pub surface_edges: Vec<i32>,
    /// Undirected edges as vertex index pairs.
    pub edges: Vec<[u32; 2]>,
    /// Shared vertices.
    pub vertices: Vec<Vec3>,
    /// Map lighting samples.
    pub lighting: Option<BspLighting>,
    /// Per-face decoupled lightmap mappings.
    pub decoupled: Vec<Option<DecoupledLightmap>>,
}

/// Reconstructed face geometry plus its lightmap description.
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedBrushFace {
    /// Triangulated face geometry.
    pub geometry: MaterialGeometry,
    /// Oriented face plane.
    pub plane: Plane,
    /// Lightmap samples for the face.
    pub lightmap: LightmapFace,
    /// Whether a decoupled mapping supplied the projection.
    pub decoupled: bool,
}

fn axis_coord(point: Vec3, axis: usize) -> f32 {
    match axis {
        0 => point.x,
        1 => point.y,
        _ => point.z,
    }
}

/// Recursively split a water polygon on `size` boundaries with 8-unit margins.
fn split_water(points: &[Vec3], size: f32) -> Vec<Vec<Vec3>> {
    if points.is_empty() {
        return vec![Vec::new()];
    }
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for point in points {
        for axis in 0..3 {
            let value = axis_coord(*point, axis);
            min[axis] = min[axis].min(value);
            max[axis] = max[axis].max(value);
        }
    }
    for axis in 0..3 {
        let middle = size * ((min[axis] + max[axis]) * 0.5 / size + 0.5).floor();
        if max[axis] - middle < 8.0 || middle - min[axis] < 8.0 {
            continue;
        }
        let mut front: Vec<Vec3> = Vec::new();
        let mut back: Vec<Vec3> = Vec::new();
        for (i, point) in points.iter().enumerate() {
            let next = points[(i + 1) % points.len()];
            let distance = axis_coord(*point, axis) - middle;
            let next_distance = axis_coord(next, axis) - middle;
            if distance >= 0.0 {
                front.push(*point);
            }
            if distance <= 0.0 {
                back.push(*point);
            }
            if distance == 0.0 || next_distance == 0.0 || (distance > 0.0) == (next_distance > 0.0) {
                continue;
            }
            let fraction = distance / (distance - next_distance);
            let intersection = vec3(
                point.x + fraction * (next.x - point.x),
                point.y + fraction * (next.y - point.y),
                point.z + fraction * (next.z - point.z),
            );
            front.push(intersection);
            back.push(intersection);
        }
        let mut polygons = split_water(&front, size);
        polygons.extend(split_water(&back, size));
        return polygons;
    }
    vec![points.to_vec()]
}

/// Reconstruct one brush face: oriented plane, edge loop, lightmap, triangulation.
///
/// `q1_lighting_scale` multiplies sample offsets (3 for Q1 RGB lighting, else 1).
/// Warped faces subdivide on `subdivision` boundaries and keep raw texture
/// coordinates; other faces normalize by `image_size`.
pub fn prepare_brush_face(
    map: &BrushMapData,
    face: &BrushFace,
    face_index: usize,
    image_size: Vec2,
    warp: bool,
    subdivision: f32,
    q1_lighting_scale: i32,
) -> Result<PreparedBrushFace, RenderError> {
    let bad = |index: usize, len: usize| RenderError::BadBatch {
        index,
        detail: format!("Brush surface index {index} outside {len}"),
    };
    let info = map
        .texture_info
        .get(face.texture_info)
        .ok_or_else(|| bad(face.texture_info, map.texture_info.len()))?;
    let raw_plane = map
        .planes
        .get(face.plane)
        .ok_or_else(|| bad(face.plane, map.planes.len()))?;
    let plane = if face.back {
        Plane {
            normal: scale3(raw_plane.normal, -1.0),
            distance: -raw_plane.distance,
        }
    } else {
        *raw_plane
    };
    let mut points: Vec<Vec3> = Vec::with_capacity(face.edge_count);
    for edge_index in 0..face.edge_count {
        let at = face.first_edge + edge_index;
        let signed = *map
            .surface_edges
            .get(at)
            .ok_or_else(|| bad(at, map.surface_edges.len()))?;
        let edge_at = signed.unsigned_abs() as usize;
        let edge = *map.edges.get(edge_at).ok_or_else(|| bad(edge_at, map.edges.len()))?;
        let vertex_at = edge[if signed >= 0 { 0 } else { 1 }] as usize;
        points.push(
            *map.vertices
                .get(vertex_at)
                .ok_or_else(|| bad(vertex_at, map.vertices.len()))?,
        );
    }
    let (axis_s, axis_t) = (info.projection_s, info.projection_t);
    let texture = |position: Vec3| {
        vec2(
            dot3(position, vec3(axis_s.x, axis_s.y, axis_s.z)) + axis_s.w,
            dot3(position, vec3(axis_t.x, axis_t.y, axis_t.z)) + axis_t.w,
        )
    };
    let (mut min_s, mut min_t) = (f32::INFINITY, f32::INFINITY);
    let (mut max_s, mut max_t) = (f32::NEG_INFINITY, f32::NEG_INFINITY);
    for point in &points {
        let uv = texture(*point);
        min_s = min_s.min(uv.x);
        min_t = min_t.min(uv.y);
        max_s = max_s.max(uv.x);
        max_t = max_t.max(uv.y);
    }
    let mins = vec2((min_s / 16.0).floor() * 16.0, (min_t / 16.0).floor() * 16.0);
    let mapping = map.decoupled.get(face_index).copied().flatten();
    let projection = match mapping {
        None => LightmapProjection::Classic {
            texture: TextureProjection { s: axis_s, t: axis_t },
            texture_mins: mins,
        },
        Some(mapping) => LightmapProjection::Decoupled { mapping },
    };
    // The shared decoupled mapping carries axes only, so extents always derive
    // from the classic 16-unit projection in both paths.
    let width = ((max_s / 16.0).ceil() as i32 - (min_s / 16.0).floor() as i32 + 1).max(1) as usize;
    let height = ((max_t / 16.0).ceil() as i32 - (min_t / 16.0).floor() as i32 + 1).max(1) as usize;
    let offset = face
        .lighting_offset
        .map(|sample| sample.saturating_mul(q1_lighting_scale).max(0) as usize);
    let lightmap = LightmapFace {
        width,
        height,
        lighting: if offset.is_none() { None } else { map.lighting.clone() },
        offset: offset.unwrap_or(0),
        styles: face.styles.clone(),
        plane,
        projection,
    };
    let polygons = if warp {
        split_water(&points, subdivision)
    } else {
        vec![points]
    };
    let mut vertices: Vec<MaterialVertex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    for polygon in &polygons {
        let first = vertices.len() as u32;
        for position in polygon {
            let uv = texture(*position);
            vertices.push(MaterialVertex::new(
                *position,
                plane.normal,
                if warp {
                    uv
                } else {
                    vec2(uv.x / image_size.x, uv.y / image_size.y)
                },
                lightmap_atlas_coordinates(*position, &projection, vec2(0.0, 0.0), width as f32, height as f32),
                [255, 255, 255, 255],
            ));
        }
        for index in 2..polygon.len() {
            indices.extend([first, first + index as u32 - 1, first + index as u32]);
        }
    }
    Ok(PreparedBrushFace {
        geometry: MaterialGeometry { vertices, indices },
        plane,
        lightmap,
        decoupled: mapping.is_some(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quad_map() -> (BrushMapData, BrushFace) {
        let map = BrushMapData {
            texture_info: vec![BrushTextureInfo {
                projection_s: Vec4 {
                    x: 1.0,
                    y: 0.0,
                    z: 0.0,
                    w: 0.0,
                },
                projection_t: Vec4 {
                    x: 0.0,
                    y: 1.0,
                    z: 0.0,
                    w: 0.0,
                },
            }],
            planes: vec![Plane {
                normal: vec3(0.0, 0.0, 1.0),
                distance: 0.0,
            }],
            surface_edges: vec![0, 1, 2, 3],
            edges: vec![[0, 1], [1, 2], [2, 3], [3, 0]],
            vertices: vec![
                vec3(0.0, 0.0, 0.0),
                vec3(32.0, 0.0, 0.0),
                vec3(32.0, 32.0, 0.0),
                vec3(0.0, 32.0, 0.0),
            ],
            lighting: Some(BspLighting::Luminance8 { samples: vec![128; 64] }),
            decoupled: vec![None],
        };
        let face = BrushFace {
            texture_info: 0,
            plane: 0,
            back: false,
            first_edge: 0,
            edge_count: 4,
            lighting_offset: Some(0),
            styles: vec![0],
        };
        (map, face)
    }

    #[test]
    fn quad_face_counts_uvs_and_lightmap() {
        let (map, face) = quad_map();
        let prepared =
            prepare_brush_face(&map, &face, 0, vec2(64.0, 64.0), false, 64.0, 1).expect("quad face prepares");
        assert_eq!(prepared.geometry.vertices.len(), 4);
        assert_eq!(prepared.geometry.indices, vec![0, 1, 2, 0, 2, 3]);
        assert_eq!(prepared.geometry.vertices[2].tex_coord, vec2(0.5, 0.5));
        assert_eq!((prepared.lightmap.width, prepared.lightmap.height), (3, 3));
        assert!(!prepared.decoupled);
        assert_eq!(prepared.plane.normal, vec3(0.0, 0.0, 1.0));
        assert!(prepared.lightmap.lighting.is_some());
    }

    #[test]
    fn back_face_flips_the_plane() {
        let (map, mut face) = quad_map();
        face.back = true;
        let prepared =
            prepare_brush_face(&map, &face, 0, vec2(64.0, 64.0), false, 64.0, 1).expect("back face prepares");
        assert_eq!(prepared.plane.normal, vec3(0.0, 0.0, -1.0));
        assert_eq!(prepared.geometry.vertices[0].normal, vec3(0.0, 0.0, -1.0));
    }

    #[test]
    fn warp_subdivision_grows_vertices_and_keeps_raw_uvs() {
        let (mut map, face) = quad_map();
        map.vertices = vec![
            vec3(0.0, 0.0, 0.0),
            vec3(128.0, 0.0, 0.0),
            vec3(128.0, 128.0, 0.0),
            vec3(0.0, 128.0, 0.0),
        ];
        let flat = prepare_brush_face(&map, &face, 0, vec2(64.0, 64.0), false, 64.0, 1).expect("flat face prepares");
        let warped = prepare_brush_face(&map, &face, 0, vec2(64.0, 64.0), true, 16.0, 1).expect("warped face prepares");
        assert!(warped.geometry.vertices.len() > flat.geometry.vertices.len());
        let max_uv = warped.geometry.vertices.iter().fold(0.0f32, |max, vertex| {
            max.max(vertex.tex_coord.x).max(vertex.tex_coord.y)
        });
        assert_eq!(max_uv, 128.0);
        assert!(flat.geometry.vertices.iter().all(|vertex| {
            vertex.tex_coord.x >= 0.0
                && vertex.tex_coord.x <= 2.0
                && vertex.tex_coord.y >= 0.0
                && vertex.tex_coord.y <= 2.0
        }));
    }

    #[test]
    fn decoupled_mapping_selects_the_decoupled_path() {
        let (mut map, face) = quad_map();
        map.decoupled = vec![Some(DecoupledLightmap {
            axes: [vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0)],
            offset: vec2(0.0, 0.0),
        })];
        let prepared =
            prepare_brush_face(&map, &face, 0, vec2(64.0, 64.0), false, 64.0, 3).expect("decoupled face prepares");
        assert!(prepared.decoupled);
        assert!(matches!(
            prepared.lightmap.projection,
            LightmapProjection::Decoupled { .. }
        ));
    }

    #[test]
    fn bad_indices_report_bad_batch() {
        let (map, mut face) = quad_map();
        face.texture_info = 9;
        assert!(matches!(
            prepare_brush_face(&map, &face, 0, vec2(64.0, 64.0), false, 64.0, 1),
            Err(RenderError::BadBatch { .. })
        ));
        let (map, mut face) = quad_map();
        face.first_edge = 99;
        assert!(matches!(
            prepare_brush_face(&map, &face, 0, vec2(64.0, 64.0), false, 64.0, 1),
            Err(RenderError::BadBatch { .. })
        ));
    }

    #[test]
    fn bounds_cover_vertices_and_empty() {
        let (map, face) = quad_map();
        let prepared =
            prepare_brush_face(&map, &face, 0, vec2(64.0, 64.0), false, 64.0, 1).expect("quad face prepares");
        let bounds = geometry_bounds(&prepared.geometry.vertices);
        assert_eq!(bounds.min, vec3(0.0, 0.0, 0.0));
        assert_eq!(bounds.max, vec3(32.0, 32.0, 0.0));
        let empty = geometry_bounds(&[]);
        assert_eq!(empty.min, vec3(0.0, 0.0, 0.0));
        assert_eq!(empty.max, vec3(0.0, 0.0, 0.0));
    }
}
