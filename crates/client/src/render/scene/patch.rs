//! Quadratic patch tessellation (`tr_curve.c`).
//!
//! Donor provenance: `src/render/scene/patch.ts` (`tessellatePatch`,
//! `insertPatchStrip`, `createMesh`). Adaptive quadratic subdivision with
//! linear-column removal, wrapped-edge normal rebuild, and stitch-time
//! strip insertion. Rust ownership replaces the donor's zone-memory machinery
//! (`TemporaryPatchMesh`/`PatchAllocator`); the tessellation behavior itself
//! is ported in full.

use qa_core::math::{add3, cross3, dot3, length3, normalize3, normalize3_or_zero, scale3, sub3, vec2, Vec3};

use crate::materials::geometry::MaterialVertex;
use crate::render::error::RenderError;

/// Tessellated quadratic patch grid.
#[derive(Debug, Clone, PartialEq)]
pub struct PatchMesh {
    /// Grid width in vertices.
    pub width: usize,
    /// Grid height in vertices.
    pub height: usize,
    /// Row-major vertices.
    pub vertices: Vec<MaterialVertex>,
    /// Triangle indices.
    pub indices: Vec<u32>,
    /// Per-column LOD error.
    pub width_lod_error: Vec<f32>,
    /// Per-row LOD error.
    pub height_lod_error: Vec<f32>,
}

/// Strip insertion direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatchDirection {
    /// Insert a column.
    Width,
    /// Insert a row.
    Height,
}

const MAX_GRID: usize = 65;
const MAX_INPUT: usize = 1024;

fn midpoint(a: &MaterialVertex, b: &MaterialVertex) -> MaterialVertex {
    MaterialVertex {
        position: scale3(add3(a.position, b.position), 0.5),
        normal: normalize3(add3(a.normal, b.normal)),
        tex_coord: vec2(
            (a.tex_coord.x + b.tex_coord.x) * 0.5,
            (a.tex_coord.y + b.tex_coord.y) * 0.5,
        ),
        lightmap_coord: vec2(
            (a.lightmap_coord.x + b.lightmap_coord.x) * 0.5,
            (a.lightmap_coord.y + b.lightmap_coord.y) * 0.5,
        ),
        color: [
            (((a.color[0] as u16 + b.color[0] as u16) >> 1) as u8),
            (((a.color[1] as u16 + b.color[1] as u16) >> 1) as u8),
            (((a.color[2] as u16 + b.color[2] as u16) >> 1) as u8),
            (((a.color[3] as u16 + b.color[3] as u16) >> 1) as u8),
        ],
    }
}

fn transpose(rows: Vec<Vec<MaterialVertex>>) -> Vec<Vec<MaterialVertex>> {
    let width = rows.first().map_or(0, Vec::len);
    (0..width).map(|x| rows.iter().map(|row| row[x]).collect()).collect()
}

fn vertex_at(vertices: &[MaterialVertex], index: usize) -> Result<MaterialVertex, RenderError> {
    vertices.get(index).copied().ok_or(RenderError::BadBatch {
        index,
        detail: format!("patch vertex {index} outside {}", vertices.len()),
    })
}

/// Adaptive quadratic subdivision, including linear-column removal and
/// wrapped normals.
pub fn tessellate_patch(
    points: &[MaterialVertex],
    width: usize,
    height: usize,
    subdivisions: f32,
) -> Result<PatchMesh, RenderError> {
    if width < 3
        || height < 3
        || width > MAX_GRID
        || height > MAX_GRID
        || width * height > MAX_INPUT
        || width % 2 != 1
        || height % 2 != 1
        || points.len() != width * height
        || !subdivisions.is_finite()
    {
        return Err(RenderError::Backend(
            "invalid quadratic patch dimensions or subdivision threshold".to_string(),
        ));
    }
    let mut rows: Vec<Vec<MaterialVertex>> = (0..height)
        .map(|y| points[y * width..(y + 1) * width].to_vec())
        .collect();
    let mut errors: Vec<Vec<f32>> = Vec::with_capacity(2);
    for _ in 0..2 {
        let mut error = vec![0.0f32; MAX_GRID];
        let mut x = 0usize;
        while x + 2 < rows[0].len() {
            let mut maximum = 0.0f32;
            for row in &rows {
                let a = row[x].position;
                let b = row[x + 1].position;
                let c = row[x + 2].position;
                let curve = sub3(scale3(add3(add3(a, scale3(b, 2.0)), c), 0.25), a);
                let line = normalize3(sub3(c, a));
                let distance = sub3(curve, scale3(line, dot3(curve, line)));
                maximum = maximum.max(dot3(distance, distance));
            }
            maximum = f64::from(maximum).sqrt() as f32;
            if maximum < 0.1 {
                error[x + 1] = 999.0;
                x += 2;
                continue;
            }
            if rows[0].len() + 2 > MAX_GRID || maximum <= subdivisions {
                error[x + 1] = 1.0 / maximum;
                x += 2;
                continue;
            }
            error[x + 2] = 1.0 / maximum;
            for row in rows.iter_mut() {
                let previous = midpoint(&row[x], &row[x + 1]);
                let next = midpoint(&row[x + 1], &row[x + 2]);
                let middle = midpoint(&previous, &next);
                row.splice(x + 1..x + 2, [previous, middle, next]);
            }
        }
        errors.push(error);
        rows = transpose(rows);
    }
    let mut width = rows[0].len();
    let mut height = rows.len();
    // Column smoothing reads neighboring rows while writing the current one.
    #[allow(clippy::needless_range_loop)]
    for x in 0..width {
        let mut y = 1usize;
        while y < height {
            let point = rows[y][x];
            let above = rows[y + 1][x];
            let below = rows[y - 1][x];
            let first = midpoint(&point, &above);
            let second = midpoint(&point, &below);
            rows[y][x] = midpoint(&first, &second);
            y += 2;
        }
    }
    for row in rows.iter_mut() {
        let mut x = 1usize;
        while x < width {
            let point = row[x];
            let next = row[x + 1];
            let previous = row[x - 1];
            let first = midpoint(&point, &next);
            let second = midpoint(&point, &previous);
            row[x] = midpoint(&first, &second);
            x += 2;
        }
    }
    let mut width_error = errors[0].clone();
    let mut height_error = errors[1].clone();
    let mut x = 1usize;
    while x < width.saturating_sub(1) {
        if width_error[x] == 999.0 {
            for row in rows.iter_mut() {
                row.remove(x);
            }
            width_error.remove(x);
            width -= 1;
        }
        x += 1;
    }
    let mut y = 1usize;
    while y < height.saturating_sub(1) {
        if height_error[y] == 999.0 {
            rows.remove(y);
            height_error.remove(y);
            height -= 1;
        }
        y += 1;
    }
    if height > width {
        rows = transpose(rows)
            .into_iter()
            .map(|mut row| {
                row.reverse();
                row
            })
            .collect();
        let old_width = width;
        let old_width_error = width_error;
        width = height;
        height = old_width;
        width_error = height_error[..width].iter().rev().copied().collect();
        height_error = old_width_error[..height].to_vec();
    }
    Ok(create_mesh(&rows, &width_error[..width], &height_error[..height]))
}

/// `R_GridInsertColumn` / `R_GridInsertRow`: interpolate a strip, anchor
/// only its matched edge, rebuild normals.
pub fn insert_patch_strip(
    mesh: &PatchMesh,
    direction: PatchDirection,
    index: usize,
    edge: usize,
    position: Vec3,
    lod_error: f32,
) -> Result<PatchMesh, RenderError> {
    let width = mesh.width + usize::from(direction == PatchDirection::Width);
    let height = mesh.height + usize::from(direction == PatchDirection::Height);
    let old_size = if direction == PatchDirection::Width {
        mesh.width
    } else {
        mesh.height
    };
    let edge_size = if direction == PatchDirection::Width {
        mesh.height
    } else {
        mesh.width
    };
    if width > MAX_GRID || height > MAX_GRID || index < 1 || index >= old_size || edge >= edge_size {
        return Err(RenderError::Backend("invalid patch insertion".to_string()));
    }
    let mut rows = Vec::with_capacity(height);
    for y in 0..height {
        let mut row = Vec::with_capacity(width);
        for x in 0..width {
            let coordinate = if direction == PatchDirection::Width { x } else { y };
            if coordinate != index {
                let old_x = x - usize::from(direction == PatchDirection::Width && x > index);
                let old_y = y - usize::from(direction == PatchDirection::Height && y > index);
                row.push(vertex_at(&mesh.vertices, old_y * mesh.width + old_x)?);
                continue;
            }
            let next = y * mesh.width + x;
            let previous = next
                - if direction == PatchDirection::Width {
                    1
                } else {
                    mesh.width
                };
            let mut point = midpoint(&vertex_at(&mesh.vertices, previous)?, &vertex_at(&mesh.vertices, next)?);
            if (if direction == PatchDirection::Width { y } else { x }) == edge {
                point.position = position;
            }
            row.push(point);
        }
        rows.push(row);
    }
    let mut width_error = mesh.width_lod_error.clone();
    let mut height_error = mesh.height_lod_error.clone();
    if direction == PatchDirection::Width {
        width_error.insert(index, lod_error);
    } else {
        height_error.insert(index, lod_error);
    }
    Ok(create_mesh(&rows, &width_error, &height_error))
}

fn create_mesh(rows: &[Vec<MaterialVertex>], width_error: &[f32], height_error: &[f32]) -> PatchMesh {
    let width = rows[0].len();
    let height = rows.len();
    let wrap_width = rows.iter().all(|row| {
        let delta = sub3(row[0].position, row[width - 1].position);
        dot3(delta, delta) <= 1.0
    });
    let wrap_height = (0..width).all(|x| {
        let delta = sub3(rows[0][x].position, rows[height - 1][x].position);
        dot3(delta, delta) <= 1.0
    });
    const NEIGHBORS: [(i32, i32); 8] = [(0, 1), (1, 1), (1, 0), (1, -1), (0, -1), (-1, -1), (-1, 0), (-1, 1)];
    let mut vertices = Vec::with_capacity(width * height);
    for (y, row) in rows.iter().enumerate() {
        for (x, point) in row.iter().enumerate() {
            let mut around = [None; 8];
            for (k, (dx, dy)) in NEIGHBORS.iter().enumerate() {
                let mut found = None;
                for distance in 1..=3 {
                    let mut nx = x as i32 + dx * distance;
                    let mut ny = y as i32 + dy * distance;
                    if wrap_width {
                        if nx < 0 {
                            nx += width as i32 - 1;
                        } else if nx >= width as i32 {
                            nx = 1 + nx - width as i32;
                        }
                    }
                    if wrap_height {
                        if ny < 0 {
                            ny += height as i32 - 1;
                        } else if ny >= height as i32 {
                            ny = 1 + ny - height as i32;
                        }
                    }
                    if nx < 0 || nx >= width as i32 || ny < 0 || ny >= height as i32 {
                        break;
                    }
                    let delta = sub3(rows[ny as usize][nx as usize].position, point.position);
                    if length3(delta) != 0.0 {
                        found = Some(normalize3_or_zero(delta));
                        break;
                    }
                }
                around[k] = found;
            }
            let mut normal = Vec3 { x: 0.0, y: 0.0, z: 0.0 };
            for k in 0..8 {
                if let (Some(a), Some(b)) = (around[k], around[(k + 1) & 7]) {
                    let cross = cross3(b, a);
                    if length3(cross) != 0.0 {
                        normal = add3(normal, normalize3_or_zero(cross));
                    }
                }
            }
            vertices.push(MaterialVertex {
                normal: normalize3_or_zero(normal),
                ..*point
            });
        }
    }
    let mut indices = Vec::with_capacity((width - 1) * (height - 1) * 6);
    for y in 0..height - 1 {
        for x in 0..width - 1 {
            let a = (y * width + x) as u32;
            let b = a + width as u32;
            indices.extend([a, b, a + 1, a + 1, b, b + 1]);
        }
    }
    PatchMesh {
        width,
        height,
        vertices,
        indices,
        width_lod_error: width_error.to_vec(),
        height_lod_error: height_error.to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use qa_core::math::{vec2, vec3};

    use super::*;

    fn flat_vertex(x: f32, y: f32) -> MaterialVertex {
        MaterialVertex::new(
            vec3(x, y, 0.0),
            vec3(0.0, 0.0, 1.0),
            vec2(x / 2.0, y / 2.0),
            vec2(0.0, 0.0),
            [200, 100, 50, 255],
        )
    }

    fn flat_points(width: usize, height: usize) -> Vec<MaterialVertex> {
        (0..height)
            .flat_map(|y| (0..width).map(move |x| flat_vertex(x as f32, y as f32)))
            .collect()
    }

    #[test]
    fn flat_3x3_removes_linear_interior() {
        let mesh = tessellate_patch(&flat_points(3, 3), 3, 3, 4.0).unwrap();
        assert_eq!((mesh.width, mesh.height), (2, 2));
        assert_eq!(mesh.vertices.len(), 4);
        assert_eq!(mesh.indices, vec![0, 2, 1, 1, 2, 3]);
        assert_eq!(mesh.width_lod_error.len(), 2);
        assert_eq!(mesh.height_lod_error.len(), 2);
        let positions: Vec<Vec3> = mesh.vertices.iter().map(|vertex| vertex.position).collect();
        assert_eq!(
            positions,
            vec![
                vec3(0.0, 0.0, 0.0),
                vec3(2.0, 0.0, 0.0),
                vec3(0.0, 2.0, 0.0),
                vec3(2.0, 2.0, 0.0),
            ]
        );
    }

    #[test]
    fn curved_patch_subdivides() {
        let mut points = flat_points(3, 3);
        points[4].position.z = 8.0;
        let mesh = tessellate_patch(&points, 3, 3, 0.5).unwrap();
        assert!(mesh.width >= 3 && mesh.height >= 3);
        assert!(mesh.width > 3 || mesh.height > 3);
        assert_eq!(mesh.vertices.len(), mesh.width * mesh.height);
        assert_eq!(mesh.width_lod_error.len(), mesh.width);
        assert_eq!(mesh.height_lod_error.len(), mesh.height);
    }

    #[test]
    fn dimension_violations_error() {
        let points = flat_points(3, 3);
        assert!(matches!(
            tessellate_patch(&points, 2, 3, 4.0),
            Err(RenderError::Backend(_))
        ));
        assert!(matches!(
            tessellate_patch(&points, 4, 3, 4.0),
            Err(RenderError::Backend(_))
        ));
        assert!(matches!(
            tessellate_patch(&points, 3, 3, f32::NAN),
            Err(RenderError::Backend(_))
        ));
        assert!(matches!(
            tessellate_patch(&points[..8], 3, 3, 4.0),
            Err(RenderError::Backend(_))
        ));
    }

    #[test]
    fn insert_strip_grows_dimension_and_aligns_errors() {
        let mesh = tessellate_patch(&flat_points(3, 3), 3, 3, 4.0).unwrap();
        assert_eq!((mesh.width, mesh.height), (2, 2));
        let grown = insert_patch_strip(&mesh, PatchDirection::Width, 1, 0, vec3(0.5, 0.0, 0.0), 0.25).unwrap();
        assert_eq!((grown.width, grown.height), (3, 2));
        assert_eq!(grown.vertices.len(), 6);
        assert_eq!(grown.width_lod_error.len(), 3);
        assert_eq!(grown.width_lod_error[1], 0.25);
        assert_eq!(grown.height_lod_error, mesh.height_lod_error);
        assert_eq!(grown.vertices[1].position, vec3(0.5, 0.0, 0.0));

        let grown_row = insert_patch_strip(&mesh, PatchDirection::Height, 1, 1, vec3(2.0, 1.5, 0.0), 0.5).unwrap();
        assert_eq!((grown_row.width, grown_row.height), (2, 3));
        assert_eq!(grown_row.height_lod_error[1], 0.5);
        assert_eq!(grown_row.vertices[1 * 2 + 1].position, vec3(2.0, 1.5, 0.0));
    }

    #[test]
    fn insert_strip_rejects_bad_index_and_edge() {
        let mesh = tessellate_patch(&flat_points(3, 3), 3, 3, 4.0).unwrap();
        assert!(matches!(
            insert_patch_strip(&mesh, PatchDirection::Width, 0, 0, vec3(0.0, 0.0, 0.0), 0.0),
            Err(RenderError::Backend(_))
        ));
        assert!(matches!(
            insert_patch_strip(&mesh, PatchDirection::Width, 3, 0, vec3(0.0, 0.0, 0.0), 0.0),
            Err(RenderError::Backend(_))
        ));
        assert!(matches!(
            insert_patch_strip(&mesh, PatchDirection::Width, 1, 3, vec3(0.0, 0.0, 0.0), 0.0),
            Err(RenderError::Backend(_))
        ));
    }

    #[test]
    fn midpoint_color_averages_channels() {
        let mut a = flat_vertex(0.0, 0.0);
        let mut b = flat_vertex(1.0, 0.0);
        a.color = [200, 101, 0, 255];
        b.color = [100, 100, 255, 0];
        assert_eq!(midpoint(&a, &b).color, [150, 100, 127, 127]);
    }
}
