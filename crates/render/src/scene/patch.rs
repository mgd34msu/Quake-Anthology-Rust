use crate::world::geometry::PatchGrid;
use qa_core::primitives::Vec3;

/// Interior row/column picks; the two endpoints are always implicit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PatchSelection {
    columns: u64,
    rows: u64,
}
impl Default for PatchSelection {
    fn default() -> Self {
        Self {
            columns: u64::MAX,
            rows: u64::MAX,
        }
    }
}
impl PatchSelection {
    /// Select a grid validated by the cold world asset boundary.
    pub fn select(
        grid: &PatchGrid,
        pose: Option<(Vec3, [Vec3; 3])>,
        view_origin: Vec3,
        view_forward: Vec3,
        curve_error: f32,
    ) -> Self {
        let error = if curve_error < 0.0 {
            0.0
        } else {
            let world = pose.map_or(grid.lod_origin, |(origin, axes)| {
                ((axes[0] * grid.lod_origin.0[0] + axes[1] * grid.lod_origin.0[1])
                    + axes[2] * grid.lod_origin.0[2])
                    + origin
            });
            let mut distance = (world - view_origin).dot(view_forward);
            if distance < 0.0 {
                distance = -distance;
            }
            distance -= grid.lod_radius;
            if distance < 1.0 {
                distance = 1.0;
            }
            curve_error / distance
        };
        let selected = |errors: &[f32]| {
            errors
                .iter()
                .enumerate()
                .skip(1)
                .take(errors.len() - 2)
                .fold(0u64, |mask, (index, &candidate)| {
                    mask | if candidate <= error {
                        1 << (index - 1)
                    } else {
                        0
                    }
                })
        };
        Self {
            columns: selected(&grid.width_lod_error),
            rows: selected(&grid.height_lod_error),
        }
    }
    pub fn is_full(self, grid: &PatchGrid) -> bool {
        let [width, height] = grid.dimensions;
        let columns = interior(width);
        let rows = interior(height);
        self.columns & columns == columns && self.rows & rows == rows
    }
    pub fn triangle_count(self, grid: &PatchGrid) -> usize {
        let [width, height] = grid.dimensions;
        let columns = (self.columns & interior(width)).count_ones() as usize + 2;
        let rows = (self.rows & interior(height)).count_ones() as usize + 2;
        (columns - 1) * (rows - 1) * 2
    }
    pub fn triangles(self, grid: &PatchGrid, first_vertex: u32) -> PatchTriangles {
        let [width, height] = grid.dimensions.map(u32::from);
        PatchTriangles {
            selection: self,
            first_vertex,
            width,
            last_row: height - 1,
            row: 0,
            next_row: Some(next(0, self.rows, height - 1)),
            column: 0,
            next_column: next(0, self.columns, width - 1),
            second: false,
        }
    }
}
fn interior(dimension: u16) -> u64 {
    (1u64 << (dimension - 2)) - 1
}
fn next(current: u32, mask: u64, last: u32) -> u32 {
    let remaining = mask & (u64::MAX << current);
    if remaining == 0 {
        last
    } else {
        (remaining.trailing_zeros() + 1).min(last)
    }
}
pub struct PatchTriangles {
    selection: PatchSelection,
    first_vertex: u32,
    width: u32,
    last_row: u32,
    row: u32,
    next_row: Option<u32>,
    column: u32,
    next_column: u32,
    second: bool,
}
impl Iterator for PatchTriangles {
    type Item = [u32; 3];
    fn next(&mut self) -> Option<Self::Item> {
        let row = self.next_row?;
        let a = self.first_vertex + self.row * self.width + self.column;
        let b = self.first_vertex + row * self.width + self.column;
        let c = self.first_vertex + self.row * self.width + self.next_column;
        let d = self.first_vertex + row * self.width + self.next_column;
        if !self.second {
            self.second = true;
            return Some([a, b, c]);
        }
        self.second = false;
        if self.next_column == self.width - 1 {
            self.column = 0;
            self.next_column = next(0, self.selection.columns, self.width - 1);
            self.row = row;
            self.next_row =
                (row < self.last_row).then(|| next(row, self.selection.rows, self.last_row));
        } else {
            self.column = self.next_column;
            self.next_column = next(self.column, self.selection.columns, self.width - 1);
        }
        Some([c, b, d])
    }
}
