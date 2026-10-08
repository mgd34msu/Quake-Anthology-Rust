//! Native Q3 adaptive subdivision and mesh normals, tr_curve.c:46-61,112-214,
//! 262-283,360-515. Cross-patch stitching and distance LOD remain separate.
use super::{GeometryError, WorldVertex, add, length, scale, sub};
use qa_core::primitives::Vec3;

const MAX_CONTROL_SIZE: usize = 32;
const MAX_GRID_SIZE: usize = 65;
const COLLINEAR: f32 = 999.0;

pub(super) struct Tessellation {
    pub vertices: Vec<WorldVertex>,
    pub dimensions: [u16; 2],
    pub width_lod_error: Box<[f32]>,
    pub height_lod_error: Box<[f32]>,
}

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
                let offset = sub(midpoint, a);
                let direction = normalize(sub(c, a));
                let distance = sub(offset, scale(direction, offset.dot(direction)));
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
    out.vertex.position = scale(add(a.vertex.position, b.vertex.position), 0.5);
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
fn normalize(value: Vec3) -> Vec3 {
    let magnitude = length(value);
    if magnitude == 0.0 {
        Vec3::default()
    } else {
        scale(value, 1.0 / magnitude)
    }
}
fn cross(a: Vec3, b: Vec3) -> Vec3 {
    Vec3([
        a.0[1] * b.0[2] - a.0[2] * b.0[1],
        a.0[2] * b.0[0] - a.0[0] * b.0[2],
        a.0[0] * b.0[1] - a.0[1] * b.0[0],
    ])
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
        let delta = sub(row[0].vertex.position, row[width - 1].vertex.position);
        delta.dot(delta) <= 1.0
    });
    let wrap_height = (0..width).all(|column| {
        let delta = sub(
            rows[0][column].vertex.position,
            rows[height - 1][column].vertex.position,
        );
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
                        normalize(sub(rows[y as usize][x as usize].vertex.position, base));
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
                    sum = add(sum, normalize(cross(around[next], around[neighbor])));
                }
            }
            rows[row][column].normal = normalize(sum);
        }
    }
}
