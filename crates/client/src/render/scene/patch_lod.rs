//! Patch LOD stitching and selection (`tr_bsp.c`, `tr_surface.c`).
//!
//! Donor provenance: `src/render/scene/patch-lod.ts` (`createPatchGrid`,
//! `preparePatchGrids`, `selectPatchLod` with `stitch`/`merged`/`matches`/
//! `sameGroup`/`synchronize`). Stitching searches forward width, forward
//! height, reverse width, reverse height; shared-vertex LOD errors are
//! synchronized across curve groups in surface order.

use qa_core::math::{add3, dot3, length3, scale3, sub3, Vec3};

use super::patch::{insert_patch_strip, PatchDirection, PatchMesh};

/// Patch mesh with its shared curve-group LOD volume.
#[derive(Debug, Clone, PartialEq)]
pub struct PatchGrid {
    /// Tessellated mesh.
    pub mesh: PatchMesh,
    /// Shared LOD origin.
    pub lod_origin: Vec3,
    /// Shared LOD radius.
    pub lod_radius: f32,
}

struct Preparing {
    mesh: PatchMesh,
    lod_origin: Vec3,
    lod_radius: f32,
    lod_fixed: i32,
    lod_stitched: bool,
}

impl From<PatchGrid> for Preparing {
    fn from(grid: PatchGrid) -> Self {
        Self {
            mesh: grid.mesh,
            lod_origin: grid.lod_origin,
            lod_radius: grid.lod_radius,
            lod_fixed: 0,
            lod_stitched: false,
        }
    }
}

impl From<Preparing> for PatchGrid {
    fn from(preparing: Preparing) -> Self {
        Self {
            mesh: preparing.mesh,
            lod_origin: preparing.lod_origin,
            lod_radius: preparing.lod_radius,
        }
    }
}

/// Store the shared curve-group volume from the surface bounds.
#[must_use]
pub fn create_patch_grid(mesh: PatchMesh, bounds: [Vec3; 2]) -> PatchGrid {
    let lod_origin = scale3(add3(bounds[0], bounds[1]), 0.5);
    let lod_radius = length3(sub3(bounds[0], lod_origin));
    PatchGrid {
        mesh,
        lod_origin,
        lod_radius,
    }
}

struct Edge {
    direction: PatchDirection,
    boundary: usize,
    count: usize,
    offset: usize,
    stride: usize,
}

fn edges(mesh: &PatchMesh) -> [Edge; 4] {
    [
        Edge {
            direction: PatchDirection::Width,
            boundary: 0,
            count: mesh.width,
            offset: 0,
            stride: 1,
        },
        Edge {
            direction: PatchDirection::Width,
            boundary: mesh.height - 1,
            count: mesh.width,
            offset: (mesh.height - 1) * mesh.width,
            stride: 1,
        },
        Edge {
            direction: PatchDirection::Height,
            boundary: 0,
            count: mesh.height,
            offset: 0,
            stride: mesh.width,
        },
        Edge {
            direction: PatchDirection::Height,
            boundary: mesh.width - 1,
            count: mesh.height,
            offset: mesh.width - 1,
            stride: mesh.width,
        },
    ]
}

fn edge_errors<'a>(mesh: &'a PatchMesh, edge: &Edge) -> &'a [f32] {
    if edge.direction == PatchDirection::Width {
        &mesh.width_lod_error
    } else {
        &mesh.height_lod_error
    }
}

fn point(mesh: &PatchMesh, edge: &Edge, index: usize) -> Vec3 {
    mesh.vertices[edge.offset + edge.stride * index].position
}

fn matches(a: Vec3, b: Vec3) -> bool {
    (a.x - b.x).abs() <= 0.1 && (a.y - b.y).abs() <= 0.1 && (a.z - b.z).abs() <= 0.1
}

fn merged(mesh: &PatchMesh, edge: &Edge) -> bool {
    for i in 1..edge.count.saturating_sub(1) {
        for j in i + 1..edge.count.saturating_sub(1) {
            if matches(point(mesh, edge, i), point(mesh, edge, j)) {
                return true;
            }
        }
    }
    false
}

fn same_group(origin: Vec3, radius: f32, grid: &Preparing) -> bool {
    radius == grid.lod_radius
        && origin.x == grid.lod_origin.x
        && origin.y == grid.lod_origin.y
        && origin.z == grid.lod_origin.z
}

/// `R_StitchPatches`: find one strip insertion of `source` detail into
/// `target`, or `None` when they already join.
fn stitch(source: &PatchMesh, target: &PatchMesh) -> Option<PatchMesh> {
    let target_edges = edges(target);
    for reversed in [false, true] {
        for source_edge in edges(source) {
            if merged(source, &source_edge) {
                continue;
            }
            let mut k: isize = if reversed { source_edge.count as isize - 1 } else { 0 };
            while if reversed {
                k > 1
            } else {
                k < source_edge.count as isize - 2
            } {
                let next = (k + if reversed { -2 } else { 2 }) as usize;
                let middle = (k + if reversed { -1 } else { 1 }) as usize;
                let kusize = k as usize;
                for target_edge in &target_edges {
                    if target_edge.count >= 65 {
                        continue;
                    }
                    for l in 0..target_edge.count.saturating_sub(1) {
                        let a = point(target, target_edge, l);
                        let b = point(target, target_edge, l + 1);
                        if !matches(point(source, &source_edge, kusize), a)
                            || !matches(point(source, &source_edge, next), b)
                        {
                            continue;
                        }
                        if (a.x - b.x).abs() < 0.01 && (a.y - b.y).abs() < 0.01 && (a.z - b.z).abs() < 0.01 {
                            continue;
                        }
                        // Compatibility repair for native UB only: the first
                        // reverse segment reads k+1 outside its allocation.
                        // Use the matched triple's actual midpoint there.
                        let error_index = if reversed && k + 1 == source_edge.count as isize {
                            middle
                        } else {
                            kusize + 1
                        };
                        let error = edge_errors(source, &source_edge)[error_index];
                        return insert_patch_strip(
                            target,
                            target_edge.direction,
                            l + 1,
                            target_edge.boundary,
                            point(source, &source_edge, middle),
                            error,
                        )
                        .ok();
                    }
                }
                k += if reversed { -2 } else { 2 };
            }
        }
    }
    None
}

/// `R_StitchAllPatches` then `R_FixSharedVertexLodError`, in surface order.
pub fn prepare_patch_grids(
    input: Vec<PatchGrid>,
    on_stitched: &mut dyn FnMut(u32),
    on_replaced: &mut dyn FnMut(usize, &PatchGrid),
) -> Vec<PatchGrid> {
    let mut grids: Vec<Preparing> = input.into_iter().map(Preparing::from).collect();
    let mut stitch_count: u32 = 0;
    loop {
        let mut visited = false;
        for i in 0..grids.len() {
            if grids[i].lod_stitched {
                continue;
            }
            grids[i].lod_stitched = true;
            visited = true;
            let group_origin = grids[i].lod_origin;
            let group_radius = grids[i].lod_radius;
            for j in 0..grids.len() {
                if !same_group(group_origin, group_radius, &grids[j]) {
                    continue;
                }
                loop {
                    let stitched = stitch(&grids[i].mesh, &grids[j].mesh);
                    let Some(mesh) = stitched else {
                        break;
                    };
                    grids[j] = Preparing {
                        mesh,
                        lod_origin: grids[j].lod_origin,
                        lod_radius: grids[j].lod_radius,
                        lod_fixed: 0,
                        lod_stitched: false,
                    };
                    let grid = PatchGrid {
                        mesh: grids[j].mesh.clone(),
                        lod_origin: grids[j].lod_origin,
                        lod_radius: grids[j].lod_radius,
                    };
                    on_replaced(j, &grid);
                    stitch_count = stitch_count.wrapping_add(1);
                }
            }
        }
        if !visited {
            break;
        }
    }
    on_stitched(stitch_count);

    synchronize_all(&mut grids);
    grids.into_iter().map(PatchGrid::from).collect()
}

fn synchronize_all(grids: &mut [Preparing]) {
    for i in 0..grids.len() {
        if grids[i].lod_fixed != 0 {
            continue;
        }
        grids[i].lod_fixed = 2;
        synchronize(grids, i + 1, i);
    }
}

fn synchronize(grids: &mut [Preparing], start: usize, source_index: usize) {
    let source_mesh = grids[source_index].mesh.clone();
    let source_origin = grids[source_index].lod_origin;
    let source_radius = grids[source_index].lod_radius;
    for j in start..grids.len() {
        if grids[j].lod_fixed == 2 || !same_group(source_origin, source_radius, &grids[j]) {
            continue;
        }
        let mut touch = false;
        for source_edge in edges(&source_mesh) {
            if merged(&source_mesh, &source_edge) {
                continue;
            }
            for k in 1..source_edge.count.saturating_sub(1) {
                for target_edge in edges(&grids[j].mesh) {
                    if merged(&grids[j].mesh, &target_edge) {
                        continue;
                    }
                    for l in 1..target_edge.count.saturating_sub(1) {
                        if !matches(
                            point(&source_mesh, &source_edge, k),
                            point(&grids[j].mesh, &target_edge, l),
                        ) {
                            continue;
                        }
                        let value = edge_errors(&source_mesh, &source_edge)[k];
                        let errors = if target_edge.direction == PatchDirection::Width {
                            &mut grids[j].mesh.width_lod_error
                        } else {
                            &mut grids[j].mesh.height_lod_error
                        };
                        errors[l] = value;
                        touch = true;
                    }
                }
            }
        }
        if touch {
            grids[j].lod_fixed = 2;
            synchronize(grids, start, j);
        }
    }
}

/// `LodErrorForVolume` and `RB_SurfaceGrid` row/column selection.
#[must_use]
pub fn select_patch_lod(
    grid: &PatchGrid,
    world_lod_origin: Vec3,
    view_origin: Vec3,
    view_forward: Vec3,
    lod_curve_error: f32,
) -> PatchMesh {
    let mut distance = dot3(sub3(world_lod_origin, view_origin), view_forward);
    if distance < 0.0 {
        distance = -distance;
    }
    distance -= grid.lod_radius;
    if distance < 1.0 {
        distance = 1.0;
    }
    let error = if lod_curve_error < 0.0 {
        0.0
    } else {
        lod_curve_error / distance
    };
    let mesh = &grid.mesh;
    let selected = |errors: &[f32]| {
        let mut result = vec![0usize];
        for (i, candidate) in errors.iter().enumerate().take(errors.len() - 1).skip(1) {
            if *candidate <= error {
                result.push(i);
            }
        }
        result.push(errors.len() - 1);
        result
    };
    let columns = selected(&mesh.width_lod_error);
    let rows = selected(&mesh.height_lod_error);
    if columns.len() == mesh.width && rows.len() == mesh.height {
        return mesh.clone();
    }
    let mut vertices = Vec::with_capacity(rows.len() * columns.len());
    for y in &rows {
        for x in &columns {
            vertices.push(mesh.vertices[y * mesh.width + x]);
        }
    }
    let mut indices = Vec::new();
    for y in 0..rows.len() - 1 {
        for x in 0..columns.len() - 1 {
            let a = (y * columns.len() + x) as u32;
            let b = a + columns.len() as u32;
            indices.extend([a, b, a + 1, a + 1, b, b + 1]);
        }
    }
    PatchMesh {
        width: columns.len(),
        height: rows.len(),
        vertices,
        indices,
        width_lod_error: columns.iter().map(|x| mesh.width_lod_error[*x]).collect(),
        height_lod_error: rows.iter().map(|y| mesh.height_lod_error[*y]).collect(),
    }
}

#[cfg(test)]
mod tests {
    use qa_core::math::{vec2, vec3};

    use crate::materials::geometry::MaterialVertex;

    use super::*;

    fn vertex(position: Vec3) -> MaterialVertex {
        MaterialVertex::new(
            position,
            vec3(0.0, 0.0, 1.0),
            vec2(0.0, 0.0),
            vec2(0.0, 0.0),
            [255, 255, 255, 255],
        )
    }

    fn grid_mesh(
        width: usize,
        height: usize,
        origin: Vec3,
        dx: f32,
        dy: f32,
        width_errors: Vec<f32>,
        height_errors: Vec<f32>,
    ) -> PatchMesh {
        let vertices: Vec<MaterialVertex> = (0..height)
            .flat_map(|y| {
                (0..width).map(move |x| vertex(vec3(origin.x + x as f32 * dx, origin.y + y as f32 * dy, origin.z)))
            })
            .collect();
        let mut indices = Vec::new();
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
            width_lod_error: width_errors,
            height_lod_error: height_errors,
        }
    }

    fn adjacent_pair() -> (PatchGrid, PatchGrid) {
        let bounds = [vec3(0.0, 0.0, 0.0), vec3(4.0, 2.0, 0.0)];
        let fine = create_patch_grid(
            grid_mesh(
                3,
                5,
                vec3(0.0, 0.0, 0.0),
                1.0,
                0.5,
                vec![0.0, 0.1, 0.0],
                vec![0.0, 0.5, 0.6, 0.7, 0.0],
            ),
            bounds,
        );
        let coarse = create_patch_grid(
            grid_mesh(
                3,
                3,
                vec3(2.0, 0.0, 0.0),
                1.0,
                1.0,
                vec![0.0, 0.2, 0.0],
                vec![0.0, 0.0, 0.0],
            ),
            bounds,
        );
        (fine, coarse)
    }

    #[test]
    fn lod_near_keeps_full_resolution() {
        let (fine, _) = adjacent_pair();
        let full = select_patch_lod(&fine, fine.lod_origin, vec3(2.0, 1.0, -1.0), vec3(0.0, 0.0, 1.0), 100.0);
        assert_eq!((full.width, full.height), (3, 5));
    }

    #[test]
    fn lod_far_reduces_resolution() {
        let (fine, _) = adjacent_pair();
        let reduced = select_patch_lod(
            &fine,
            fine.lod_origin,
            vec3(2.0, 1.0, -1000.0),
            vec3(0.0, 0.0, 1.0),
            0.05,
        );
        assert!(reduced.width < 3 || reduced.height < 5);
        assert_eq!(reduced.vertices.len(), reduced.width * reduced.height);
        assert_eq!(reduced.width_lod_error.len(), reduced.width);
        assert_eq!(reduced.height_lod_error.len(), reduced.height);
    }

    #[test]
    fn negative_curve_error_drops_positive_interior() {
        let (fine, _) = adjacent_pair();
        let dropped = select_patch_lod(&fine, fine.lod_origin, fine.lod_origin, vec3(0.0, 0.0, 1.0), -1.0);
        assert_eq!((dropped.width, dropped.height), (2, 2));
    }

    #[test]
    fn stitch_joins_adjacent_grids() {
        let (fine, coarse) = adjacent_pair();
        let mut stitched = 0u32;
        let mut replaced = Vec::new();
        let grids = prepare_patch_grids(vec![fine, coarse], &mut |count| stitched = count, &mut |index, _| {
            replaced.push(index)
        });
        assert_eq!(stitched, 2);
        assert_eq!(replaced, vec![1, 1]);
        assert_eq!((grids[0].mesh.width, grids[0].mesh.height), (3, 5));
        assert_eq!((grids[1].mesh.width, grids[1].mesh.height), (3, 5));
    }

    #[test]
    fn shared_vertex_errors_synchronize() {
        let (fine, coarse) = adjacent_pair();
        let mut stitched = 0u32;
        let grids = prepare_patch_grids(vec![fine, coarse], &mut |count| stitched = count, &mut |_, _| {});
        assert_eq!(stitched, 2);
        assert_eq!(grids[1].mesh.height_lod_error, vec![0.0, 0.5, 0.6, 0.7, 0.0]);
    }
}
