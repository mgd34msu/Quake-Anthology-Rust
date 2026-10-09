//! Native Q3 adaptive subdivision and mesh normals, tr_curve.c:46-61,112-214,
//! 262-283,360-624; stitching/group propagation follows tr_bsp.c:534-1186.
use super::{
    GeometryError, GeometryOptions, PatchStats, WorldGeometry, WorldVertex, push_triangle, span,
    vertex_bounds,
};
use qa_core::{
    math::{cross, normalized_or_zero},
    primitives::Vec3,
};

const MAX_CONTROL_SIZE: usize = 32;
const MAX_GRID_SIZE: usize = 65;
const COLLINEAR: f32 = 999.0;

pub(super) struct Tessellation {
    pub vertices: Vec<WorldVertex>,
    pub dimensions: [u16; 2],
    pub width_lod_error: Box<[f32]>,
    pub height_lod_error: Box<[f32]>,
}

#[expect(
    clippy::needless_range_loop,
    reason = "Preserve tr_curve.c direction/transposition passes and PutPointsOnCurve column-before-row updates."
)]
pub(super) fn subdivide(
    points: &[WorldVertex],
    dimensions: [i32; 2],
    tolerance: f32,
    id: u32,
) -> Result<Tessellation, GeometryError> {
    let [source_width, source_height] = dimensions;
    if source_width < 3
        || source_height < 3
        || source_width % 2 == 0
        || source_height % 2 == 0
        || source_width as usize > MAX_CONTROL_SIZE
        || source_height as usize > MAX_CONTROL_SIZE
        || source_width as usize * source_height as usize != points.len()
    {
        return Err(GeometryError::InvalidPatch(id));
    }
    let mut width = source_width as usize;
    let mut height = source_height as usize;
    let mut rows: Vec<Vec<WorldVertex>> = points.chunks_exact(width).map(|r| r.to_vec()).collect();
    let mut errors = [[0.0f32; MAX_GRID_SIZE]; 2];
    for axis in 0..2 {
        let mut column = 0;
        while column + 2 < width {
            let mut maximum_squared = 0.0f32;
            for row in &rows {
                let a = row[column].vertex.position;
                let b = row[column + 1].vertex.position;
                let c = row[column + 2].vertex.position;
                let midpoint = Vec3(std::array::from_fn(|i| {
                    (a.0[i] + b.0[i] * 2.0 + c.0[i]) * 0.25
                }));
                let offset = midpoint - a;
                let direction = normalized_or_zero(c - a);
                let distance = offset - (direction * offset.dot(direction));
                maximum_squared = maximum_squared.max(distance.dot(distance));
            }
            let deviation = maximum_squared.sqrt();
            if !deviation.is_finite() {
                return Err(GeometryError::NonFinite("patch curvature", id));
            }
            if deviation < 0.1 {
                errors[axis][column + 1] = COLLINEAR;
                column += 2;
                continue;
            }
            if width + 2 > MAX_GRID_SIZE || deviation <= tolerance {
                errors[axis][column + 1] = 1.0 / deviation;
                column += 2;
                continue;
            }
            errors[axis][column + 2] = 1.0 / deviation;
            for row in &mut rows {
                let previous = lerp(row[column], row[column + 1]);
                let next = lerp(row[column + 1], row[column + 2]);
                let middle = lerp(previous, next);
                row[column + 1] = previous;
                row.insert(column + 2, middle);
                row.insert(column + 3, next);
            }
            width += 2;
            // Native j -= 2 cancels its loop increment and rechecks this segment.
        }
        rows = transpose(&rows, width, height);
        std::mem::swap(&mut width, &mut height);
    }
    // PutPointsOnCurve first projects odd rows, then odd columns.
    for column in 0..width {
        for row in (1..height).step_by(2) {
            let previous = lerp(rows[row][column], rows[row + 1][column]);
            let next = lerp(rows[row][column], rows[row - 1][column]);
            rows[row][column] = lerp(previous, next);
        }
    }
    for row in &mut rows {
        for column in (1..width).step_by(2) {
            let previous = lerp(row[column], row[column + 1]);
            let next = lerp(row[column], row[column - 1]);
            row[column] = lerp(previous, next);
        }
    }
    let mut column = 1;
    while column < width - 1 {
        if errors[0][column] == COLLINEAR {
            for row in &mut rows {
                row.remove(column);
            }
            errors[0].copy_within(column + 1..width, column);
            width -= 1;
        }
        column += 1;
    }
    let mut row = 1;
    while row < height - 1 {
        if errors[1][row] == COLLINEAR {
            rows.remove(row);
            errors[1].copy_within(row + 1..height, row);
            height -= 1;
        }
        row += 1;
    }
    if height > width {
        rows = transpose(&rows, width, height);
        let old = errors;
        for i in 0..width {
            errors[1][i] = old[0][i];
        }
        for i in 0..height {
            errors[0][i] = old[1][height - 1 - i];
        }
        std::mem::swap(&mut width, &mut height);
        for row in &mut rows {
            row.reverse();
        }
    }
    mesh_normals(&mut rows, width, height);
    let vertices: Vec<_> = rows.into_iter().flatten().collect();
    if vertices.iter().any(|v| {
        v.vertex
            .position
            .0
            .iter()
            .chain(v.vertex.texcoord.iter())
            .chain(v.vertex.lightmap_coord.iter())
            .chain(v.normal.0.iter())
            .any(|n| !n.is_finite())
    }) {
        return Err(GeometryError::NonFinite("patch output", id));
    }
    Ok(Tessellation {
        vertices,
        dimensions: [width as u16, height as u16],
        width_lod_error: errors[0][..width].to_vec().into_boxed_slice(),
        height_lod_error: errors[1][..height].to_vec().into_boxed_slice(),
    })
}

fn lerp(a: WorldVertex, b: WorldVertex) -> WorldVertex {
    let mut out = a;
    out.vertex.position = (a.vertex.position + b.vertex.position) * 0.5;
    out.vertex.texcoord =
        std::array::from_fn(|i| 0.5 * (a.vertex.texcoord[i] + b.vertex.texcoord[i]));
    out.vertex.lightmap_coord =
        std::array::from_fn(|i| 0.5 * (a.vertex.lightmap_coord[i] + b.vertex.lightmap_coord[i]));
    out.vertex.color = std::array::from_fn(|i| {
        ((u16::from(a.vertex.color[i]) + u16::from(b.vertex.color[i])) >> 1) as u8
    });
    out
}
fn transpose(rows: &[Vec<WorldVertex>], width: usize, height: usize) -> Vec<Vec<WorldVertex>> {
    (0..width)
        .map(|column| (0..height).map(|row| rows[row][column]).collect())
        .collect()
}
fn mesh_normals(rows: &mut [Vec<WorldVertex>], width: usize, height: usize) {
    const NEIGHBORS: [[i32; 2]; 8] = [
        [0, 1],
        [1, 1],
        [1, 0],
        [1, -1],
        [0, -1],
        [-1, -1],
        [-1, 0],
        [-1, 1],
    ];
    let wrap_width = rows.iter().all(|row| {
        let delta = row[0].vertex.position - (row[width - 1].vertex.position);
        delta.dot(delta) <= 1.0
    });
    let wrap_height = (0..width).all(|column| {
        let delta = rows[0][column].vertex.position - (rows[height - 1][column].vertex.position);
        delta.dot(delta) <= 1.0
    });
    for column in 0..width {
        for row in 0..height {
            let base = rows[row][column].vertex.position;
            let mut around = [Vec3::default(); 8];
            let mut good = [false; 8];
            for neighbor in 0..8 {
                for distance in 1..=3 {
                    let mut x = column as i32 + NEIGHBORS[neighbor][0] * distance;
                    let mut y = row as i32 + NEIGHBORS[neighbor][1] * distance;
                    if wrap_width {
                        if x < 0 {
                            x += width as i32 - 1;
                        } else if x >= width as i32 {
                            x = 1 + x - width as i32;
                        }
                    }
                    if wrap_height {
                        if y < 0 {
                            y += height as i32 - 1;
                        } else if y >= height as i32 {
                            y = 1 + y - height as i32;
                        }
                    }
                    if x < 0 || y < 0 || x >= width as i32 || y >= height as i32 {
                        break;
                    }
                    let direction =
                        normalized_or_zero(rows[y as usize][x as usize].vertex.position - base);
                    if direction == Vec3::default() {
                        continue;
                    }
                    around[neighbor] = direction;
                    good[neighbor] = true;
                    break;
                }
            }
            let mut sum = Vec3::default();
            for neighbor in 0..8 {
                let next = (neighbor + 1) & 7;
                if good[neighbor] && good[next] {
                    sum = sum + normalized_or_zero(cross(around[next], around[neighbor]));
                }
            }
            rows[row][column].normal = normalized_or_zero(sum);
            rows[row][column].vertex.normal = rows[row][column].normal;
        }
    }
}

struct Grid {
    surface: usize,
    partition: u32,
    width: usize,
    height: usize,
    vertices: Vec<WorldVertex>,
    width_errors: Vec<f32>,
    height_errors: Vec<f32>,
    origin: Vec3,
    radius: f32,
    stitched: bool,
    fixed: bool,
    changed: bool,
}

#[derive(Clone, Copy)]
struct Edge {
    count: usize,
    offset: usize,
    stride: usize,
    boundary: usize,
    vertical: bool,
}
impl Grid {
    fn edge(&self, index: usize) -> Edge {
        if index < 2 {
            Edge {
                count: self.width,
                offset: if index == 0 {
                    0
                } else {
                    (self.height - 1) * self.width
                },
                stride: 1,
                boundary: if index == 0 { 0 } else { self.height - 1 },
                vertical: false,
            }
        } else {
            Edge {
                count: self.height,
                offset: if index == 2 { 0 } else { self.width - 1 },
                stride: self.width,
                boundary: if index == 2 { 0 } else { self.width - 1 },
                vertical: true,
            }
        }
    }
    fn point(&self, edge: Edge, at: usize) -> Vec3 {
        self.vertices[edge.offset + at * edge.stride]
            .vertex
            .position
    }
    fn error(&self, edge: Edge, at: usize) -> f32 {
        if edge.vertical {
            self.height_errors[at]
        } else {
            self.width_errors[at]
        }
    }
    fn set_error(&mut self, edge: Edge, at: usize, error: f32) {
        if edge.vertical {
            self.height_errors[at] = error;
        } else {
            self.width_errors[at] = error;
        }
    }
    fn merged(&self, edge: Edge) -> bool {
        for i in 1..edge.count - 1 {
            for j in i + 1..edge.count - 1 {
                if matches(self.point(edge, i), self.point(edge, j)) {
                    return true;
                }
            }
        }
        false
    }
    fn same_group(&self, other: &Self) -> bool {
        self.partition == other.partition
            && self.radius == other.radius
            && self.origin == other.origin
    }
}

pub(super) fn finish_world_patches(
    geometry: &mut WorldGeometry,
    options: GeometryOptions,
    partitions: &[u32],
) -> Result<PatchStats, GeometryError> {
    let mut grids = Vec::new();
    for (index, surface) in geometry.surfaces.iter().enumerate() {
        let Some(patch) = &surface.patch else {
            continue;
        };
        // Native ParseMesh returns SF_SKIP before building a no-draw patch.
        if surface.no_draw {
            continue;
        }
        let width = patch.dimensions[0] as usize;
        let height = patch.dimensions[1] as usize;
        if !(2..=MAX_GRID_SIZE).contains(&width)
            || !(2..=MAX_GRID_SIZE).contains(&height)
            || width * height != surface.vertices.count as usize
            || patch.width_lod_error.len() != width
            || patch.height_lod_error.len() != height
        {
            return Err(GeometryError::InvalidPatch(surface.source_id));
        }
        let vertices = super::section(
            &geometry.vertices,
            surface.vertices,
            "patch vertices",
            surface.source_id,
        )?
        .to_vec();
        let partition = *partitions.get(index).ok_or(GeometryError::SizeLimit)?;
        grids.push(Grid {
            surface: index,
            partition,
            width,
            height,
            vertices,
            width_errors: patch.width_lod_error.to_vec(),
            height_errors: patch.height_lod_error.to_vec(),
            origin: patch.lod_origin,
            radius: patch.lod_radius,
            stitched: false,
            fixed: false,
            changed: false,
        });
    }
    let mut stats = PatchStats::default();
    loop {
        let mut visited = false;
        for source in 0..grids.len() {
            if grids[source].stitched {
                continue;
            }
            grids[source].stitched = true;
            visited = true;
            for target in 0..grids.len() {
                if !grids[source].same_group(&grids[target]) {
                    continue;
                }
                while let Some(insertion) = find_stitch(&grids[source], &grids[target]) {
                    if insertion.reverse_endpoint_fix {
                        stats.reverse_endpoint_fixes += 1;
                    }
                    insert(&mut grids[target], insertion, options.max_surface_vertices)?;
                    stats.insertions += 1;
                }
            }
        }
        if !visited {
            break;
        }
    }
    // Explicit DFS retains native recursion/first-surface ordering without
    // risking the process stack on a large connected patch group.
    let mut stack = Vec::with_capacity(grids.len());
    for root in 0..grids.len() {
        if grids[root].fixed {
            continue;
        }
        grids[root].fixed = true;
        stack.push((root, root + 1));
        while let Some((source, next)) = stack.last_mut() {
            if *next == grids.len() {
                stack.pop();
                continue;
            }
            let target = *next;
            *next += 1;
            let source = *source;
            if grids[target].fixed || !grids[source].same_group(&grids[target]) {
                continue;
            }
            let copies = synchronize(&mut grids, source, target);
            stats.lod_copies += copies;
            if copies > 0 {
                grids[target].fixed = true;
                stack.push((target, root + 1));
            }
        }
    }
    for grid in grids {
        let mut vertices = geometry.surfaces[grid.surface].vertices;
        let mut indices = geometry.surfaces[grid.surface].indices;
        let mut boundaries = geometry.surfaces[grid.surface].boundaries;
        let mut bounds = geometry.surfaces[grid.surface].bounds;
        if grid.changed {
            stats.changed_grids += 1;
            vertices = span(geometry.vertices.len(), grid.vertices.len())?;
            bounds = vertex_bounds(&grid.vertices)?;
            geometry.vertices.extend_from_slice(&grid.vertices);
            let first_index = geometry.indices.len();
            let first_boundary = geometry.boundaries.len();
            for row in 0..grid.height - 1 {
                for column in 0..grid.width - 1 {
                    let a = vertices.first + (row * grid.width + column) as u32;
                    let b = a + grid.width as u32;
                    push_triangle(geometry, [a, b, a + 1])?;
                    push_triangle(geometry, [a + 1, b, b + 1])?;
                }
            }
            indices = span(first_index, geometry.indices.len() - first_index)?;
            boundaries = span(first_boundary, geometry.boundaries.len() - first_boundary)?;
        }
        let surface = &mut geometry.surfaces[grid.surface];
        surface.vertices = vertices;
        surface.indices = indices;
        surface.boundaries = boundaries;
        surface.bounds = bounds;
        let Some(patch) = &mut surface.patch else {
            return Err(GeometryError::SizeLimit);
        };
        patch.dimensions = [grid.width as u16, grid.height as u16];
        patch.width_lod_error = grid.width_errors.into_boxed_slice();
        patch.height_lod_error = grid.height_errors.into_boxed_slice();
    }
    Ok(stats)
}

#[derive(Clone, Copy)]
struct Insertion {
    edge: Edge,
    index: usize,
    anchor: Vec3,
    error: f32,
    reverse_endpoint_fix: bool,
}
fn find_stitch(source: &Grid, target: &Grid) -> Option<Insertion> {
    // Native order is forward width/height edges, then reverse width/height.
    for reversed in [false, true] {
        for source_edge in 0..4 {
            let a = source.edge(source_edge);
            if source.merged(a) {
                continue;
            }
            let mut k = if reversed { a.count as i32 - 1 } else { 0 };
            while if reversed {
                k > 1
            } else {
                k + 2 < a.count as i32
            } {
                let next = (k + if reversed { -2 } else { 2 }) as usize;
                let middle = (k + if reversed { -1 } else { 1 }) as usize;
                for target_edge in 0..4 {
                    let b = target.edge(target_edge);
                    if b.count >= MAX_GRID_SIZE {
                        continue;
                    }
                    for l in 0..b.count - 1 {
                        let first = target.point(b, l);
                        let last = target.point(b, l + 1);
                        if !matches(source.point(a, k as usize), first)
                            || !matches(source.point(a, next), last)
                            || (0..3).all(|i| ((first.0[i] - last.0[i]).abs() as f64) < 0.01)
                        {
                            continue;
                        }
                        // Preserve native in-range k+1. The C port fixes only
                        // the endpoint OOB by selecting its real midpoint.
                        let endpoint_fix = reversed && k as usize + 1 == a.count;
                        let error_index = if endpoint_fix { middle } else { k as usize + 1 };
                        return Some(Insertion {
                            edge: b,
                            index: l + 1,
                            anchor: source.point(a, middle),
                            error: source.error(a, error_index),
                            reverse_endpoint_fix: endpoint_fix,
                        });
                    }
                }
                k += if reversed { -2 } else { 2 };
            }
        }
    }
    None
}
fn insert(grid: &mut Grid, insertion: Insertion, limit: usize) -> Result<(), GeometryError> {
    let new_width = grid.width + usize::from(!insertion.edge.vertical);
    let new_height = grid.height + usize::from(insertion.edge.vertical);
    if new_width * new_height > limit {
        return Err(GeometryError::TooManySurfaceVertices(grid.surface as u32));
    }
    let mut rows: Vec<Vec<WorldVertex>> = grid
        .vertices
        .chunks_exact(grid.width)
        .map(|row| row.to_vec())
        .collect();
    if insertion.edge.vertical {
        let mut row: Vec<_> = (0..grid.width)
            .map(|column| {
                lerp(
                    rows[insertion.index - 1][column],
                    rows[insertion.index][column],
                )
            })
            .collect();
        row[insertion.edge.boundary].vertex.position = insertion.anchor;
        rows.insert(insertion.index, row);
        grid.height_errors.insert(insertion.index, insertion.error);
    } else {
        for (index, row) in rows.iter_mut().enumerate() {
            let mut point = lerp(row[insertion.index - 1], row[insertion.index]);
            if index == insertion.edge.boundary {
                point.vertex.position = insertion.anchor;
            }
            row.insert(insertion.index, point);
        }
        grid.width_errors.insert(insertion.index, insertion.error);
    }
    mesh_normals(&mut rows, new_width, new_height);
    grid.vertices = rows.into_iter().flatten().collect();
    grid.width = new_width;
    grid.height = new_height;
    grid.stitched = false;
    grid.fixed = false;
    grid.changed = true;
    Ok(())
}

fn synchronize(grids: &mut [Grid], source: usize, target: usize) -> usize {
    let (a, b) = if source < target {
        let (left, right) = grids.split_at_mut(target);
        (&left[source], &mut right[0])
    } else {
        let (left, right) = grids.split_at_mut(source);
        (&right[0], &mut left[target])
    };
    let mut copies = 0;
    for source_edge in 0..4 {
        let edge_a = a.edge(source_edge);
        if a.merged(edge_a) {
            continue;
        }
        for k in 1..edge_a.count - 1 {
            for target_edge in 0..4 {
                let edge_b = b.edge(target_edge);
                if b.merged(edge_b) {
                    continue;
                }
                for l in 1..edge_b.count - 1 {
                    if matches(a.point(edge_a, k), b.point(edge_b, l)) {
                        b.set_error(edge_b, l, a.error(edge_a, k));
                        copies += 1;
                    }
                }
            }
        }
    }
    copies
}
fn matches(a: Vec3, b: Vec3) -> bool {
    // Original .1/.01 literals are double; promote the f32 difference so the
    // exact f32(0.1) boundary retains the native comparison behavior.
    (0..3).all(|axis| ((a.0[axis] - b.0[axis]).abs() as f64) <= 0.1)
}
