//! Load-generated thin facets share the brush resource, topology and reducer.
use super::brushes::{BrushWork, GeometryError, convex_interval};
use super::{Contents, Trace, surfaces::SurfaceRows};
use qa_core::{
    math,
    primitives::{Bounds, IndexSpan, Plane, Vec3},
};

pub struct PatchInput {
    pub dimensions: [u32; 2],
    pub control_points: Vec<Vec3>,
    pub contents: Contents,
    pub surface: u32,
}

const MAX_CONTROL_POINTS: usize = 65_536;
const MAX_GRID_DIMENSION: usize = 257;
const MAX_FACETS: usize = 131_072;
const MAX_PATCH_PLANES: usize = 262_144;
const MAX_WINDING_POINTS: usize = 128;
const MAP_BOUNDS: f32 = 65_535.0;

#[derive(Clone, Copy)]
struct Border {
    plane: u32,
    inward: bool,
}
struct Facet {
    surface: u32,
    borders: Box<[Border]>,
}
pub(super) struct Patch {
    planes: IndexSpan,
    facets: Box<[Facet]>,
    pub bounds: Bounds,
    pub contents: Contents,
}
#[derive(Clone, Copy, Default)]
pub(super) struct PointRelation {
    front: bool,
    intersection: f32,
}

fn push<T>(values: &mut Vec<T>, value: T, limit: usize) -> Result<(), GeometryError> {
    if values.len() == limit {
        return Err(GeometryError::Capacity);
    }
    values.try_reserve(1).map_err(|_| GeometryError::Capacity)?;
    values.push(value);
    Ok(())
}
fn close_points(first: Vec3, second: Vec3) -> bool {
    (0..3).all(|axis| (f64::from(first.0[axis] - second.0[axis])).abs() <= 0.1)
}
fn flipped(plane: Plane) -> Plane {
    Plane::oriented(-plane.normal, -plane.distance)
}
fn plane_equal(first: Plane, second: Plane) -> bool {
    (0..3).all(|axis| f64::from(first.normal.0[axis] - second.normal.0[axis]).abs() < 0.0001)
        && f64::from(first.distance - second.distance).abs() < 0.02
}
fn bounds(points: &[Vec3]) -> Bounds {
    let mut bounds = Bounds {
        mins: Vec3([MAP_BOUNDS; 3]),
        maxs: Vec3([-MAP_BOUNDS; 3]),
    };
    for point in points {
        for axis in 0..3 {
            bounds.mins.0[axis] = bounds.mins.0[axis].min(point.0[axis]);
            bounds.maxs.0[axis] = bounds.maxs.0[axis].max(point.0[axis]);
        }
    }
    bounds
}

struct Grid {
    columns: Vec<Vec<Vec3>>,
}
impl Grid {
    fn wrapped(&self) -> bool {
        self.columns[0]
            .iter()
            .zip(self.columns.last().unwrap())
            .all(|(&a, &b)| close_points(a, b))
    }
    fn subdivide(&mut self) -> Result<(), GeometryError> {
        let mut index = 0;
        while index + 2 < self.columns.len() {
            let needed = (0..self.columns[0].len()).any(|row| {
                let a = self.columns[index][row];
                let b = self.columns[index + 1][row];
                let c = self.columns[index + 2][row];
                let linear = math::midpoint(a, c);
                let curve = Vec3(std::array::from_fn(|axis| {
                    (0.5 * (0.5 * f64::from(a.0[axis] + b.0[axis])
                        + 0.5 * f64::from(b.0[axis] + c.0[axis]))) as f32
                }));
                math::length(curve - linear) >= 16.0
            });
            if !needed {
                self.columns.remove(index + 1);
                index += 1;
                continue;
            }
            if self.columns.len() + 2 > MAX_GRID_DIMENSION {
                return Err(GeometryError::Capacity);
            }
            let mut first = Vec::new();
            let mut middle = Vec::new();
            let mut last = Vec::new();
            for row in 0..self.columns[0].len() {
                let a = self.columns[index][row];
                let b = self.columns[index + 1][row];
                let c = self.columns[index + 2][row];
                let left = math::midpoint(a, b);
                let right = math::midpoint(b, c);
                push(&mut first, left, MAX_GRID_DIMENSION)?;
                push(&mut middle, math::midpoint(left, right), MAX_GRID_DIMENSION)?;
                push(&mut last, right, MAX_GRID_DIMENSION)?;
            }
            self.columns
                .try_reserve(2)
                .map_err(|_| GeometryError::Capacity)?;
            self.columns[index + 1] = first;
            self.columns.insert(index + 2, middle);
            self.columns.insert(index + 3, last);
        }
        let mut index = 0;
        while index + 1 < self.columns.len() {
            if self.columns[index]
                .iter()
                .zip(&self.columns[index + 1])
                .all(|(&a, &b)| close_points(a, b))
            {
                self.columns.remove(index + 1);
            } else {
                index += 1;
            }
        }
        Ok(())
    }
    fn transpose(&mut self) -> Result<(), GeometryError> {
        let mut columns = Vec::new();
        for row in 0..self.columns[0].len() {
            let mut column = Vec::new();
            for old in &self.columns {
                push(&mut column, old[row], MAX_GRID_DIMENSION)?;
            }
            push(&mut columns, column, MAX_GRID_DIMENSION)?;
        }
        self.columns = columns;
        Ok(())
    }
    fn point(&self, x: usize, y: usize) -> Vec3 {
        self.columns[x][y]
    }
}

fn base_winding(plane: Plane) -> Vec<Vec3> {
    let mut axis = 0;
    for candidate in 1..3 {
        if plane.normal.0[candidate].abs() > plane.normal.0[axis].abs() {
            axis = candidate;
        }
    }
    let mut up = Vec3::axis(if axis == 2 { 0 } else { 2 }, 1.0);
    up = up + plane.normal * -up.dot(plane.normal);
    math::normalize(&mut up);
    let origin = plane.normal * plane.distance;
    let right = math::cross(up, plane.normal) * MAP_BOUNDS;
    up = up * MAP_BOUNDS;
    vec![
        (origin - right) + up,
        (origin + right) + up,
        (origin + right) - up,
        (origin - right) - up,
    ]
}
fn chop(winding: &mut Vec<Vec3>, plane: Plane) -> Result<(), GeometryError> {
    let mut sides = [0i8; MAX_WINDING_POINTS + 1];
    let mut distances = [0.0f32; MAX_WINDING_POINTS + 1];
    let mut front = false;
    let mut back = false;
    if winding.is_empty() {
        return Ok(());
    }
    for (index, &point) in winding.iter().enumerate() {
        let distance = point.dot(plane.normal) - plane.distance;
        distances[index] = distance;
        sides[index] = if distance > 0.1 {
            front = true;
            1
        } else if distance < -0.1 {
            back = true;
            -1
        } else {
            0
        };
    }
    if !front {
        winding.clear();
        return Ok(());
    }
    if !back {
        return Ok(());
    }
    sides[winding.len()] = sides[0];
    distances[winding.len()] = distances[0];
    let mut output = Vec::new();
    for (index, &first) in winding.iter().enumerate() {
        if sides[index] >= 0 {
            push(&mut output, first, MAX_WINDING_POINTS)?;
        }
        if sides[index] == 0 || sides[index + 1] == 0 || sides[index] == sides[index + 1] {
            continue;
        }
        let second = winding[(index + 1) % winding.len()];
        let fraction = distances[index] / (distances[index] - distances[index + 1]);
        let point = Vec3(std::array::from_fn(|axis| {
            if plane.normal.0[axis] == 1.0 {
                plane.distance
            } else if plane.normal.0[axis] == -1.0 {
                -plane.distance
            } else {
                first.0[axis] + fraction * (second.0[axis] - first.0[axis])
            }
        }));
        push(&mut output, point, MAX_WINDING_POINTS)?;
    }
    *winding = output;
    Ok(())
}

struct Generator {
    planes: Vec<Plane>,
    facets: Vec<Facet>,
}
impl Generator {
    fn plane(&mut self, a: Vec3, b: Vec3, c: Vec3) -> Result<Option<u32>, GeometryError> {
        let Some(plane) = Plane::from_points(a, b, c) else {
            return Ok(None);
        };
        for (index, &existing) in self.planes.iter().enumerate() {
            if plane.normal.dot(existing.normal) >= 0.0
                && [a, b, c].iter().all(|point| {
                    f64::from(point.dot(existing.normal) - existing.distance).abs() <= 0.1
                })
            {
                return Ok(Some(index as u32));
            }
        }
        let index = self.planes.len() as u32;
        push(&mut self.planes, plane, MAX_PATCH_PLANES)?;
        Ok(Some(index))
    }
    fn bevel_plane(&mut self, plane: Plane) -> Result<Border, GeometryError> {
        for (index, &existing) in self.planes.iter().enumerate() {
            if plane_equal(existing, plane) {
                return Ok(Border {
                    plane: index as u32,
                    inward: false,
                });
            }
            if plane_equal(existing, flipped(plane)) {
                return Ok(Border {
                    plane: index as u32,
                    inward: true,
                });
            }
        }
        let index = self.planes.len() as u32;
        push(&mut self.planes, plane, MAX_PATCH_PLANES)?;
        Ok(Border {
            plane: index,
            inward: false,
        })
    }
    fn duplicate(&self, surface: u32, borders: &[Border], plane: Plane) -> bool {
        std::iter::once(surface)
            .chain(borders.iter().map(|border| border.plane))
            .any(|index| {
                plane_equal(self.planes[index as usize], plane)
                    || plane_equal(self.planes[index as usize], flipped(plane))
            })
    }
    fn facet(
        &mut self,
        surface: Option<u32>,
        raw: &[Option<u32>],
        points: &[Vec3],
    ) -> Result<(), GeometryError> {
        let Some(surface) = surface else {
            return Ok(());
        };
        let mut borders = Vec::new();
        for &index in raw {
            let Some(index) = index else { return Ok(()) };
            let plane = self.planes[index as usize];
            let front = points
                .iter()
                .any(|point| f64::from(point.dot(plane.normal) - plane.distance) > 0.1);
            let back = points
                .iter()
                .any(|point| f64::from(point.dot(plane.normal) - plane.distance) < -0.1);
            if !front && !back {
                return Ok(());
            }
            push(
                &mut borders,
                Border {
                    plane: index,
                    inward: front && !back,
                },
                MAX_WINDING_POINTS,
            )?;
        }
        let mut winding = base_winding(self.planes[surface as usize]);
        for border in &borders {
            let plane = self.planes[border.plane as usize];
            chop(
                &mut winding,
                if border.inward { plane } else { flipped(plane) },
            )?;
        }
        if winding.is_empty() {
            return Ok(());
        }
        let extent = bounds(&winding);
        if (0..3).any(|axis| {
            extent.maxs.0[axis] - extent.mins.0[axis] > MAP_BOUNDS
                || extent.mins.0[axis] >= MAP_BOUNDS
                || extent.maxs.0[axis] <= -MAP_BOUNDS
        }) {
            return Ok(());
        }
        // CM_AddFacetBevels independently recreates its winding after validation.
        winding = base_winding(self.planes[surface as usize]);
        for border in &borders {
            if border.plane == surface {
                continue;
            }
            let plane = self.planes[border.plane as usize];
            chop(
                &mut winding,
                if border.inward { plane } else { flipped(plane) },
            )?;
        }
        if !winding.is_empty() {
            let extent = bounds(&winding);
            for axis in 0..3 {
                for direction in [-1.0, 1.0] {
                    let plane = Plane::oriented(
                        Vec3::axis(axis, direction),
                        if direction == 1.0 {
                            extent.maxs.0[axis]
                        } else {
                            -extent.mins.0[axis]
                        },
                    );
                    if !self.duplicate(surface, &borders, plane) {
                        let border = self.bevel_plane(plane)?;
                        push(&mut borders, border, MAX_WINDING_POINTS)?;
                    }
                }
            }
            for index in 0..winding.len() {
                let mut edge = winding[index] - winding[(index + 1) % winding.len()];
                if math::normalize(&mut edge) < 0.5 {
                    continue;
                }
                for axis in 0..3 {
                    if f64::from(edge.0[axis] - 1.0).abs() < 0.0001 {
                        edge = Vec3::axis(axis, 1.0);
                        break;
                    }
                    if f64::from(edge.0[axis] + 1.0).abs() < 0.0001 {
                        edge = Vec3::axis(axis, -1.0);
                        break;
                    }
                }
                if edge.0.iter().any(|value| value.abs() == 1.0) {
                    continue;
                }
                for axis in 0..3 {
                    for direction in [-1.0, 1.0] {
                        let mut normal = math::cross(edge, Vec3::axis(axis, direction));
                        if math::normalize(&mut normal) < 0.5 {
                            continue;
                        }
                        let plane = Plane::oriented(normal, winding[index].dot(normal));
                        if winding
                            .iter()
                            .any(|point| f64::from(point.dot(normal) - plane.distance) > 0.1)
                            || self.duplicate(surface, &borders, plane)
                        {
                            continue;
                        }
                        let border = self.bevel_plane(plane)?;
                        let plane = self.planes[border.plane as usize];
                        let mut clipped = winding.clone();
                        chop(
                            &mut clipped,
                            if border.inward { plane } else { flipped(plane) },
                        )?;
                        if !clipped.is_empty() {
                            push(&mut borders, border, MAX_WINDING_POINTS)?;
                        }
                    }
                }
            }
            push(
                &mut borders,
                Border {
                    plane: surface,
                    inward: true,
                },
                MAX_WINDING_POINTS,
            )?;
        }
        push(
            &mut self.facets,
            Facet {
                surface,
                borders: borders.into_boxed_slice(),
            },
            MAX_FACETS,
        )
    }
    fn edge(
        &mut self,
        grid: &Grid,
        cells: &[[Option<u32>; 2]],
        x: usize,
        y: usize,
        edge: usize,
    ) -> Result<Option<u32>, GeometryError> {
        let (a, b, base, triangle) = match edge {
            0 => (grid.point(x, y), grid.point(x + 1, y), grid.point(x, y), 0),
            1 => (
                grid.point(x + 1, y),
                grid.point(x + 1, y + 1),
                grid.point(x + 1, y),
                0,
            ),
            2 => (
                grid.point(x + 1, y + 1),
                grid.point(x, y + 1),
                grid.point(x, y + 1),
                1,
            ),
            3 => (grid.point(x, y + 1), grid.point(x, y), grid.point(x, y), 1),
            4 => (
                grid.point(x + 1, y + 1),
                grid.point(x, y),
                grid.point(x + 1, y + 1),
                0,
            ),
            5 => (
                grid.point(x, y),
                grid.point(x + 1, y + 1),
                grid.point(x, y),
                1,
            ),
            _ => unreachable!(),
        };
        let cell = cells[x * (grid.columns[0].len() - 1) + y];
        let Some(plane) = cell[triangle].or(cell[1 - triangle]) else {
            return Ok(None);
        };
        self.plane(a, b, base + self.planes[plane as usize].normal * 4.0)
    }
}

pub(super) fn generate(
    input: PatchInput,
    planes: &mut Vec<Plane>,
    side_surfaces: &mut Vec<u32>,
) -> Result<Patch, GeometryError> {
    let [width, height] = input.dimensions.map(|value| value as usize);
    if width < 3
        || height < 3
        || width % 2 == 0
        || height % 2 == 0
        || width > MAX_GRID_DIMENSION
        || height > MAX_GRID_DIMENSION
        || width.checked_mul(height) != Some(input.control_points.len())
        || input.control_points.len() > MAX_CONTROL_POINTS
        || input.control_points.iter().any(|point| !point.is_finite())
    {
        return Err(GeometryError::PatchGrid);
    }
    let mut grid = Grid {
        columns: Vec::new(),
    };
    for x in 0..width {
        let mut column = Vec::new();
        for y in 0..height {
            push(
                &mut column,
                input.control_points[y * width + x],
                MAX_GRID_DIMENSION,
            )?;
        }
        push(&mut grid.columns, column, MAX_GRID_DIMENSION)?;
    }
    let wrap_height = grid.wrapped();
    grid.subdivide()?;
    grid.transpose()?;
    let wrap_width = grid.wrapped();
    grid.subdivide()?;
    let width = grid.columns.len();
    let height = grid.columns[0].len();
    let mut generator = Generator {
        planes: Vec::new(),
        facets: Vec::new(),
    };
    let mut cells = Vec::new();
    for x in 0..width - 1 {
        for y in 0..height - 1 {
            let first = generator.plane(
                grid.point(x, y),
                grid.point(x + 1, y),
                grid.point(x + 1, y + 1),
            )?;
            let second = generator.plane(
                grid.point(x + 1, y + 1),
                grid.point(x, y + 1),
                grid.point(x, y),
            )?;
            push(&mut cells, [first, second], MAX_FACETS)?;
        }
    }
    let at = |x: usize, y: usize, triangle: usize| cells[x * (height - 1) + y][triangle];
    for x in 0..width - 1 {
        for y in 0..height - 1 {
            let first = at(x, y, 0);
            let second = at(x, y, 1);
            let mut top = if y > 0 {
                at(x, y - 1, 1)
            } else if wrap_height {
                at(x, height - 2, 1)
            } else {
                None
            };
            let mut bottom = if y + 2 < height {
                at(x, y + 1, 0)
            } else if wrap_height {
                at(x, 0, 0)
            } else {
                None
            };
            let mut left = if x > 0 {
                at(x - 1, y, 0)
            } else if wrap_width {
                at(width - 2, y, 0)
            } else {
                None
            };
            let mut right = if x + 2 < width {
                at(x + 1, y, 1)
            } else if wrap_width {
                at(0, y, 1)
            } else {
                None
            };
            if top.is_none() || top == first {
                top = generator.edge(&grid, &cells, x, y, 0)?;
            }
            if bottom.is_none() || bottom == second {
                bottom = generator.edge(&grid, &cells, x, y, 2)?;
            }
            if left.is_none() || left == second {
                left = generator.edge(&grid, &cells, x, y, 3)?;
            }
            if right.is_none() || right == first {
                right = generator.edge(&grid, &cells, x, y, 1)?;
            }
            if first == second {
                generator.facet(
                    first,
                    &[top, right, bottom, left],
                    &[
                        grid.point(x, y),
                        grid.point(x + 1, y),
                        grid.point(x + 1, y + 1),
                        grid.point(x, y + 1),
                    ],
                )?;
            } else {
                let mut diagonal = second.or(bottom);
                if diagonal.is_none() {
                    diagonal = generator.edge(&grid, &cells, x, y, 4)?;
                }
                generator.facet(
                    first,
                    &[top, right, diagonal],
                    &[
                        grid.point(x, y),
                        grid.point(x + 1, y),
                        grid.point(x + 1, y + 1),
                    ],
                )?;
                let mut diagonal = first.or(top);
                if diagonal.is_none() {
                    diagonal = generator.edge(&grid, &cells, x, y, 5)?;
                }
                generator.facet(
                    second,
                    &[bottom, left, diagonal],
                    &[
                        grid.point(x + 1, y + 1),
                        grid.point(x, y + 1),
                        grid.point(x, y),
                    ],
                )?;
            }
        }
    }
    let mut extent = Bounds {
        mins: Vec3([f32::INFINITY; 3]),
        maxs: Vec3([f32::NEG_INFINITY; 3]),
    };
    for column in &grid.columns {
        for point in column {
            for axis in 0..3 {
                extent.mins.0[axis] = extent.mins.0[axis].min(point.0[axis]);
                extent.maxs.0[axis] = extent.maxs.0[axis].max(point.0[axis]);
            }
        }
    }
    extent.mins = extent.mins - Vec3([1.0; 3]);
    extent.maxs = extent.maxs + Vec3([1.0; 3]);
    let base = u32::try_from(planes.len()).map_err(|_| GeometryError::Capacity)?;
    let count = u32::try_from(generator.planes.len()).map_err(|_| GeometryError::Capacity)?;
    base.checked_add(count).ok_or(GeometryError::Capacity)?;
    for facet in &mut generator.facets {
        facet.surface += base;
        for border in &mut facet.borders {
            border.plane += base;
        }
    }
    planes
        .try_reserve(generator.planes.len())
        .map_err(|_| GeometryError::Capacity)?;
    side_surfaces
        .try_reserve(generator.planes.len())
        .map_err(|_| GeometryError::Capacity)?;
    side_surfaces.extend(std::iter::repeat_n(input.surface, generator.planes.len()));
    planes.extend(generator.planes);
    Ok(Patch {
        planes: IndexSpan { first: base, count },
        facets: generator.facets.into_boxed_slice(),
        bounds: extent,
        contents: input.contents,
    })
}

fn expanded(work: &BrushWork<'_>, original: Plane, border: Option<bool>) -> (Plane, Vec3, Vec3) {
    let mut plane = if border == Some(true) {
        flipped(original)
    } else {
        original
    };
    let (start, end) = if let Some(capsule) = work.query.capsule {
        plane.distance += capsule.radius;
        capsule.closest_segment(work.start, work.end, plane.normal)
    } else {
        // Inward borders flip the normal, but native corner selection still
        // uses the original plane's sign bits and its actual centered bounds.
        let corner = Bounds {
            mins: work.mins,
            maxs: work.maxs,
        }
        .corner(original.normal, false);
        let offset = corner.dot(plane.normal);
        if border.is_some() {
            plane.distance = (f64::from(plane.distance) + f64::from(offset).abs()) as f32;
        } else {
            plane.distance -= offset;
        }
        (work.start, work.end)
    };
    (plane, start, end)
}

fn set_plane(trace: &mut Trace, plane: Plane) {
    // CM_TraceThroughPatchCollide writes normal/dist, retaining cplane type and
    // sign bytes from the trace's previous contact (or its zero initializer).
    let encoding = trace.plane.type_sign();
    trace.plane = Plane {
        encoding: Some(encoding),
        ..plane
    };
}

impl Patch {
    pub(super) fn plane_count(&self) -> usize {
        self.planes.count as usize
    }
    pub(super) fn trace(
        &self,
        work: &BrushWork<'_>,
        planes: &[Plane],
        surfaces: SurfaceRows<'_>,
        position: bool,
        trace: &mut Trace,
        relationships: &mut [PointRelation],
    ) {
        if position {
            for facet in &self.facets {
                let sides = std::iter::once((facet.surface, None)).chain(
                    facet
                        .borders
                        .iter()
                        .map(|border| (border.plane, Some(border.inward))),
                );
                if sides.into_iter().all(|(index, border)| {
                    let (plane, start, _) = expanded(work, planes[index as usize], border);
                    start.dot(plane.normal) - plane.distance <= 0.0
                }) {
                    trace.start_solid = true;
                    trace.all_solid = true;
                    trace.fraction = 0.0;
                    trace.contents = self.contents;
                    return;
                }
            }
            return;
        }
        let before = trace.fraction;
        if work.tree_point {
            if !work.query.rules.patches.points() {
                return;
            }
            self.trace_point(work, planes, trace, relationships);
        } else {
            for facet in &self.facets {
                let side = |index: usize| {
                    if index == 0 {
                        (facet.surface, None)
                    } else {
                        let border = facet.borders[index - 1];
                        (border.plane, Some(border.inward))
                    }
                };
                let Some(interval) =
                    convex_interval(work.query.rules, facet.borders.len() + 1, |index| {
                        let (index, border) = side(index);
                        let (plane, start, end) = expanded(work, planes[index as usize], border);
                        (
                            start.dot(plane.normal) - plane.distance,
                            end.dot(plane.normal) - plane.distance,
                        )
                    })
                else {
                    continue;
                };
                if interval.contact == facet.borders.len()
                    || interval.enter >= interval.leave
                    || interval.enter < 0.0
                    || interval.enter >= trace.fraction
                {
                    continue;
                }
                let (index, border) = side(interval.contact);
                let (plane, _, _) = expanded(work, planes[index as usize], border);
                trace.fraction = interval.enter;
                set_plane(trace, plane);
            }
        }
        if trace.fraction < before {
            trace.contents = self.contents;
            // All generated planes of this patch refer to the same authored
            // shader record through the existing side/surface mapping.
            let (flags, id) = surfaces.contact(self.planes.first as usize);
            trace.surface = flags;
            trace.surface_id = id;
        }
    }
    fn trace_point(
        &self,
        work: &BrushWork<'_>,
        planes: &[Plane],
        trace: &mut Trace,
        relationships: &mut [PointRelation],
    ) {
        for (index, &plane) in planes[self.planes.range()].iter().enumerate() {
            let corner = Bounds {
                mins: work.mins,
                maxs: work.maxs,
            }
            .corner(plane.normal, false);
            let offset = corner.dot(plane.normal);
            let first = (work.start.dot(plane.normal) - plane.distance) + offset;
            let last = (work.end.dot(plane.normal) - plane.distance) + offset;
            let mut intersection = if first == last {
                99999.0
            } else {
                first / (first - last)
            };
            if intersection <= 0.0 {
                intersection = 99999.0;
            }
            relationships[index] = PointRelation {
                front: first > 0.0,
                intersection,
            };
        }
        for facet in &self.facets {
            let surface = relationships[(facet.surface - self.planes.first) as usize];
            if !surface.front || surface.intersection < 0.0 || surface.intersection > trace.fraction
            {
                continue;
            }
            if !facet.borders.iter().all(|border| {
                let relation = relationships[(border.plane - self.planes.first) as usize];
                if relation.front ^ border.inward {
                    relation.intersection <= surface.intersection
                } else {
                    relation.intersection >= surface.intersection
                }
            }) {
                continue;
            }
            let plane = planes[facet.surface as usize];
            let corner = Bounds {
                mins: work.mins,
                maxs: work.maxs,
            }
            .corner(plane.normal, false);
            let offset = corner.dot(plane.normal);
            let first = (work.start.dot(plane.normal) - plane.distance) + offset;
            let last = (work.end.dot(plane.normal) - plane.distance) + offset;
            trace.fraction = work
                .query
                .rules
                .contact_fraction(first, last, true)
                .max(0.0);
            set_plane(trace, plane);
        }
    }
}
