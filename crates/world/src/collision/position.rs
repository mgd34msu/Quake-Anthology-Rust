//! Exact stationary box occupancy over admitted hull-zero path cells.
//! Native hull traversal and moving exact boxes do not use this predicate.
mod integer;

use self::integer::Int;
use super::{Contents, Trace, TraceQuery};
use qa_core::primitives::{Bounds, ClipNode, IndexSpan, Plane, Vec3};

/// Optional admission bounds. Bytes include retained arrays and one query
/// workspace, plus all concurrently owned builder arrays. Work counts graph
/// visits, basis candidates and determinants, not elapsed time.
#[derive(Clone, Copy, Debug)]
pub struct HullPositionLimits {
    pub cells: usize,
    pub sides: usize,
    pub path_planes: usize,
    pub bytes: usize,
    pub admission_work: usize,
    pub query_work: usize,
}

impl Default for HullPositionLimits {
    fn default() -> Self {
        Self {
            cells: 65_536,
            sides: 1_048_576,
            path_planes: 512,
            bytes: 64 * 1024 * 1024,
            admission_work: 16_777_216,
            query_work: 1_048_576,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Failure {
    Input,
    Budget,
    Arithmetic,
    Scratch,
}

#[derive(Clone, Copy, Default)]
struct Row {
    normal: [Int; 3],
    distance: Int,
    strict: bool,
}

#[derive(Clone, Copy)]
struct Side {
    plane: u32,
    inverted: bool,
    strict: bool,
}

#[derive(Clone, Copy)]
struct Cell {
    sides: IndexSpan,
    contents: Contents,
}

pub(super) struct Data {
    cells: Vec<Cell>,
    sides: Vec<Side>,
    rows: usize,
    query_work: usize,
}

pub(super) struct Prepared {
    pub data: Data,
    pub models: Vec<IndexSpan>,
}

pub(super) struct Scratch {
    rows: Box<[Row]>,
}

impl Scratch {
    pub fn new(rows: usize) -> Self {
        Self {
            rows: vec![Row::default(); rows].into_boxed_slice(),
        }
    }
}

struct Work {
    remaining: usize,
}

impl Work {
    fn item(&mut self) -> Result<(), Failure> {
        self.remaining = self.remaining.checked_sub(1).ok_or(Failure::Budget)?;
        Ok(())
    }
}

/// Both passes and both feasibility modes use these fixed-size combinations.
fn combinations(
    len: usize,
    count: usize,
    mut visit: impl FnMut(&[usize]) -> Result<bool, Failure>,
) -> Result<(), Failure> {
    if count == 0 || count > 4 || count > len {
        return Ok(());
    }
    let mut indices = [0usize; 4];
    for (index, slot) in indices[..count].iter_mut().enumerate() {
        *slot = index;
    }
    loop {
        if !visit(&indices[..count])? {
            return Ok(());
        }
        let mut next = count;
        while next != 0 && indices[next - 1] == len - count + next - 1 {
            next -= 1;
        }
        if next == 0 {
            return Ok(());
        }
        indices[next - 1] += 1;
        for index in next..count {
            indices[index] = indices[index - 1] + 1;
        }
    }
}

fn determinant(matrix: &[[Int; 4]; 4], count: usize, work: &mut Work) -> Result<Int, Failure> {
    work.item()?;
    fn terms(
        matrix: &[[Int; 4]; 4],
        count: usize,
        row: usize,
        columns: &mut [usize; 4],
        used: u8,
        total: &mut Int,
    ) -> Result<(), Failure> {
        if row == count {
            let mut product = Int::ONE;
            let mut negative = false;
            for index in 0..count {
                product = product.mul(&matrix[index][columns[index]])?;
                for later in index + 1..count {
                    negative ^= columns[index] > columns[later];
                }
            }
            if negative {
                product = product.neg();
            }
            *total = total.add(&product)?;
            return Ok(());
        }
        for column in 0..count {
            if used & (1 << column) == 0 {
                columns[row] = column;
                terms(matrix, count, row + 1, columns, used | (1 << column), total)?;
            }
        }
        Ok(())
    }
    let mut result = Int::ZERO;
    terms(matrix, count, 0, &mut [0; 4], 0, &mut result)?;
    Ok(result)
}

/// Return nonnegative Cramer numerators for N*lambda=0, sum(lambda)=1.
/// An independent minor has one unique candidate. A failed full-system check
/// cannot be repaired by trying another minor of the same columns.
fn certificate(
    columns: &[[Int; 4]; 4],
    count: usize,
    work: &mut Work,
) -> Result<Option<[Int; 4]>, Failure> {
    let mut answer = None;
    combinations(4, count, |selected| {
        let mut matrix = [[Int::ZERO; 4]; 4];
        for (row, &source) in selected.iter().enumerate() {
            for column in 0..count {
                matrix[row][column] = columns[column][source];
            }
        }
        let mut denominator = determinant(&matrix, count, work)?;
        if denominator.is_zero() {
            return Ok(true);
        }
        let mut weights = [Int::ZERO; 4];
        for column in 0..count {
            let mut replaced = matrix;
            for (row, &source) in selected.iter().enumerate() {
                replaced[row][column] = if source == 3 { Int::ONE } else { Int::ZERO };
            }
            weights[column] = determinant(&replaced, count, work)?;
        }
        if denominator.negative() {
            denominator = denominator.neg();
            for weight in &mut weights[..count] {
                *weight = weight.neg();
            }
        }
        if weights[..count].iter().any(|weight| weight.negative()) {
            return Ok(false);
        }
        for row in 0..4 {
            let mut projected = Int::ZERO;
            for column in 0..count {
                projected = projected.add(&columns[column][row].mul(&weights[column])?)?;
            }
            let expected = if row == 3 { denominator } else { Int::ZERO };
            if projected != expected {
                return Ok(false);
            }
        }
        answer = Some(weights);
        Ok(false)
    })?;
    Ok(answer)
}

/// The Motzkin alternative handles cold mixed-strict paths and hot closed
/// intersections. A certificate needs at most four rows in three dimensions.
/// For posed binary32 rows, |normal|<3*M^2 and |distance|<10*M^3, M=2^277.
/// Every certificate distance is <6480*M^9 (2506 bits), below 64 limbs.
fn feasible(rows: &[Row], work: &mut Work) -> Result<bool, Failure> {
    for count in 1..=4 {
        let mut empty = false;
        combinations(rows.len(), count, |indices| {
            work.item()?;
            let mut columns = [[Int::ZERO; 4]; 4];
            for (column, &index) in indices.iter().enumerate() {
                columns[column][..3].copy_from_slice(&rows[index].normal);
                columns[column][3] = Int::ONE;
            }
            let Some(weights) = certificate(&columns, count, work)? else {
                return Ok(true);
            };
            let mut distance = Int::ZERO;
            let mut strict = false;
            for (column, &index) in indices.iter().enumerate() {
                distance = distance.add(&weights[column].mul(&rows[index].distance)?)?;
                strict |= rows[index].strict && !weights[column].is_zero();
            }
            empty = distance.negative() || distance.is_zero() && strict;
            Ok(!empty)
        })?;
        if empty {
            return Ok(false);
        }
    }
    Ok(true)
}

fn vector(value: Vec3) -> Result<[Int; 3], Failure> {
    Ok([
        Int::binary32(value.0[0])?,
        Int::binary32(value.0[1])?,
        Int::binary32(value.0[2])?,
    ])
}

fn source_row(planes: &[Plane], side: Side) -> Result<Row, Failure> {
    let plane = planes[side.plane as usize];
    let mut row = Row {
        normal: vector(plane.normal)?,
        distance: Int::binary32(plane.distance)?,
        strict: side.strict,
    };
    if side.inverted {
        row.normal = row.normal.map(Int::neg);
        row.distance = row.distance.neg();
    }
    Ok(row)
}

fn reserve<T>(count: usize, held: &mut usize, limit: usize) -> Result<Vec<T>, Failure> {
    let expected = count.checked_mul(size_of::<T>()).ok_or(Failure::Budget)?;
    if held.checked_add(expected).is_none_or(|total| total > limit) {
        return Err(Failure::Budget);
    }
    let mut values = Vec::new();
    values
        .try_reserve_exact(count)
        .map_err(|_| Failure::Budget)?;
    let actual = values
        .capacity()
        .checked_mul(size_of::<T>())
        .ok_or(Failure::Budget)?;
    *held = held
        .checked_add(actual)
        .filter(|total| *total <= limit)
        .ok_or(Failure::Budget)?;
    Ok(values)
}

#[derive(Clone, Copy)]
struct Pending {
    node: i32,
    prefix: usize,
    side: Option<Side>,
}

fn paths(
    nodes: &[ClipNode],
    root: i32,
    path: &mut Vec<Side>,
    pending: &mut Vec<Pending>,
    work: &mut Work,
    mut leaf: impl FnMut(&[Side], Contents, &mut Work) -> Result<(), Failure>,
) -> Result<(), Failure> {
    path.clear();
    pending.clear();
    pending.push(Pending {
        node: root,
        prefix: 0,
        side: None,
    });
    while let Some(frame) = pending.pop() {
        work.item()?;
        path.truncate(frame.prefix);
        if let Some(side) = frame.side {
            if path.len() == path.capacity() {
                return Err(Failure::Budget);
            }
            path.push(side);
        }
        if frame.node < 0 {
            let contents = Contents::from_q1(frame.node);
            if contents != Contents::EMPTY {
                leaf(path, contents, work)?;
            }
        } else {
            if pending
                .len()
                .checked_add(2)
                .is_none_or(|len| len > pending.capacity())
            {
                return Err(Failure::Budget);
            }
            let node = nodes[frame.node as usize];
            pending.push(Pending {
                node: node.children[1],
                prefix: path.len(),
                side: Some(Side {
                    plane: node.plane,
                    inverted: false,
                    strict: true,
                }),
            });
            pending.push(Pending {
                node: node.children[0],
                prefix: path.len(),
                side: Some(Side {
                    plane: node.plane,
                    inverted: true,
                    strict: false,
                }),
            });
        }
    }
    Ok(())
}

pub(super) fn prepare(
    planes: &[Plane],
    nodes: &[ClipNode],
    model_count: usize,
    root: impl Fn(usize) -> i32,
    depth: usize,
    limits: HullPositionLimits,
) -> Result<Prepared, Failure> {
    if depth > limits.path_planes || model_count > u32::MAX as usize {
        return Err(Failure::Budget);
    }
    let mut held = 0;
    let mut path = reserve::<Side>(depth, &mut held, limits.bytes)?;
    let mut pending = reserve::<Pending>(
        depth.checked_add(1).ok_or(Failure::Budget)?,
        &mut held,
        limits.bytes,
    )?;
    let mut work = Work {
        remaining: limits.admission_work,
    };
    let (mut cells, mut sides, mut greatest) = (0usize, 0usize, 0usize);
    // Count first, then reserve once. The validated graph may share children;
    // cell paths, rather than node count, determine retained storage.
    for model in 0..model_count {
        paths(
            nodes,
            root(model),
            &mut path,
            &mut pending,
            &mut work,
            |path, _, _| {
                cells = cells
                    .checked_add(1)
                    .filter(|n| *n <= limits.cells && *n <= u32::MAX as usize)
                    .ok_or(Failure::Budget)?;
                sides = sides
                    .checked_add(path.len())
                    .filter(|n| *n <= limits.sides && *n <= u32::MAX as usize)
                    .ok_or(Failure::Budget)?;
                greatest = greatest.max(path.len());
                Ok(())
            },
        )?;
    }
    let rows = greatest.checked_add(6).ok_or(Failure::Budget)?;
    let mut workspace = reserve::<Row>(rows, &mut held, limits.bytes)?;
    workspace.resize(rows, Row::default());
    let mut retained_cells = reserve::<Cell>(cells, &mut held, limits.bytes)?;
    let mut retained_sides = reserve::<Side>(sides, &mut held, limits.bytes)?;
    let mut models = reserve::<IndexSpan>(model_count, &mut held, limits.bytes)?;
    for model in 0..model_count {
        let first = retained_cells.len();
        paths(
            nodes,
            root(model),
            &mut path,
            &mut pending,
            &mut work,
            |path, contents, work| {
                for (row, &side) in workspace.iter_mut().zip(path) {
                    *row = source_row(planes, side)?;
                }
                if feasible(&workspace[..path.len()], work)? {
                    let first = retained_sides.len();
                    retained_sides.extend_from_slice(path);
                    retained_cells.push(Cell {
                        sides: IndexSpan {
                            first: first as u32,
                            count: path.len() as u32,
                        },
                        contents,
                    });
                }
                Ok(())
            },
        )?;
        models.push(IndexSpan {
            first: first as u32,
            count: (retained_cells.len() - first) as u32,
        });
    }
    Ok(Prepared {
        data: Data {
            cells: retained_cells,
            sides: retained_sides,
            rows,
            query_work: limits.query_work,
        },
        models,
    })
}

impl Data {
    pub fn scratch_rows(&self) -> usize {
        self.rows
    }

    /// Query original world-space bounds. T is the selected native pose's
    /// finite world-to-local matrix; no endpoint or AABB is rounded locally.
    pub fn trace(
        &self,
        planes: &[Plane],
        cells: IndexSpan,
        query: TraceQuery,
        origin: Vec3,
        axes: [Vec3; 3],
        scratch: &mut Scratch,
    ) -> Result<Trace, Failure> {
        if query.start != query.end
            || !query.start.is_finite()
            || !(Bounds {
                mins: query.mins,
                maxs: query.maxs,
            })
            .is_valid()
        {
            return Err(Failure::Input);
        }
        if scratch.rows.len() < self.rows {
            return Err(Failure::Scratch);
        }
        let origin = vector(origin)?;
        let center = vector(query.start)?;
        let bounds = [vector(query.mins)?, vector(query.maxs)?];
        let axes = [vector(axes[0])?, vector(axes[1])?, vector(axes[2])?];
        let mut work = Work {
            remaining: self.query_work,
        };
        let mut matrix = [[Int::ZERO; 4]; 4];
        for axis in 0..3 {
            matrix[axis][..3].copy_from_slice(&axes[axis]);
        }
        if determinant(&matrix, 3, &mut work)?.is_zero() {
            return Err(Failure::Input);
        }
        for cell in &self.cells[cells.range()] {
            work.item()?;
            if !cell.contents.intersects(query.mask) {
                continue;
            }
            let rows = &mut scratch.rows[..cell.sides.count as usize + 6];
            for (row, &side) in rows.iter_mut().zip(&self.sides[cell.sides.range()]) {
                let source = source_row(planes, side)?;
                let mut transformed = Row {
                    distance: source.distance.shift(298)?,
                    ..Row::default()
                };
                for component in 0..3 {
                    for local in 0..3 {
                        transformed.normal[component] = transformed.normal[component]
                            .add(&source.normal[local].mul(&axes[local][component])?)?;
                    }
                    transformed.distance = transformed
                        .distance
                        .add(&transformed.normal[component].mul(&origin[component])?)?;
                }
                *row = transformed;
            }
            for axis in 0..3 {
                for side in 0..2 {
                    let mut row = Row::default();
                    row.normal[axis] = if side == 0 { Int::ONE.neg() } else { Int::ONE };
                    row.distance = center[axis].add(&bounds[side][axis])?;
                    if side == 0 {
                        row.distance = row.distance.neg();
                    }
                    rows[cell.sides.count as usize + axis * 2 + side] = row;
                }
            }
            if feasible(rows, &mut work)? {
                let mut trace = Trace::clear(query.end);
                trace.start_solid = true;
                trace.all_solid = true;
                trace.fraction = 0.0;
                trace.contents = cell.contents;
                return Ok(trace);
            }
        }
        Ok(Trace::clear(query.end))
    }
}
