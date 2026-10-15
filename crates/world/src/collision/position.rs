//! Exact shape occupancy and certified sweeps over BSP or authored convex cells.
//! Native compiled hull traversal does not use this predicate.
mod integer;
mod sphere;

use self::integer::Int;
use super::{AllSolid, Contents, Trace, TraceQuery, brushes::Brush, hulls::HullModel};
use qa_core::{
    math,
    primitives::{Bounds, Capsule, ClipNode, IndexSpan, Plane, Vec3},
};
use std::cmp::Ordering;

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
    /// Contact-local active constraints; overflow rejects the query.
    pub contact_rows: usize,
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
            contact_rows: 256,
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
    slope: Int,
    strict: bool,
}

impl Row {
    fn reduce(&mut self) -> Result<(), Failure> {
        let shift = self
            .normal
            .iter()
            .chain([&self.distance, &self.slope])
            .map(Int::trailing_zeros)
            .min()
            .ok_or(Failure::Input)?;
        for value in self
            .normal
            .iter_mut()
            .chain([&mut self.distance, &mut self.slope])
        {
            *value = value.unshift(shift)?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Default)]
struct Cost {
    alpha: Int,
    beta: Int,
    strict: bool,
}

fn cost(rows: &[Row], indices: &[usize], weights: &[Int; 4]) -> Result<Cost, Failure> {
    let mut result = Cost::default();
    for (column, &index) in indices.iter().enumerate() {
        result.alpha = result
            .alpha
            .add(&weights[column].mul(&rows[index].distance)?)?;
        result.beta = result.beta.add(&weights[column].mul(&rows[index].slope)?)?;
        result.strict |= rows[index].strict && !weights[column].is_zero();
    }
    Ok(result)
}

#[derive(Clone, Copy)]
struct Fraction {
    numerator: Int,
    denominator: Int,
}

impl Fraction {
    const ZERO: Self = Self {
        numerator: Int::ZERO,
        denominator: Int::ONE,
    };
    const ONE: Self = Self {
        numerator: Int::ONE,
        denominator: Int::ONE,
    };

    fn new(mut numerator: Int, mut denominator: Int) -> Self {
        debug_assert!(!denominator.is_zero());
        if denominator.negative() {
            numerator = numerator.neg();
            denominator = denominator.neg();
        }
        Self {
            numerator,
            denominator,
        }
    }

    fn binary32(value: f32) -> Result<Self, Failure> {
        Ok(Self::new(Int::binary32(value)?, Int::ONE.shift(149)?))
    }

    fn compare(self, other: Self) -> Result<Ordering, Failure> {
        Ok(self
            .numerator
            .mul(&other.denominator)?
            .compare(&other.numerator.mul(&self.denominator)?))
    }
}

impl Cost {
    fn at(self, parameter: Fraction) -> Result<Int, Failure> {
        if self.beta.is_zero() {
            // Constant, all-source certificates may have greater degree; do
            // not unnecessarily multiply them by an unrelated denominator.
            return Ok(self.alpha);
        }
        self.alpha
            .mul(&parameter.denominator)?
            .add(&self.beta.mul(&parameter.numerator)?)
    }
}

#[derive(Clone, Copy)]
struct Interval<T = Fraction> {
    first: T,
    last: T,
    isolated: bool,
}

trait Parameter: Copy {
    const ZERO: Self;
    const ONE: Self;
    fn order(self, other: Self, work: &mut Work) -> Result<Ordering, Failure>;
}

impl Parameter for Fraction {
    const ZERO: Self = Self::ZERO;
    const ONE: Self = Self::ONE;
    fn order(self, other: Self, _: &mut Work) -> Result<Ordering, Failure> {
        self.compare(other)
    }
}

impl<T: Parameter> Interval<T> {
    fn contains(self, parameter: T, work: &mut Work) -> Result<bool, Failure> {
        Ok(self.first.order(parameter, work)? != Ordering::Greater
            && self.last.order(parameter, work)? != Ordering::Less)
    }
}

struct Component<T> {
    start_contents: Option<Contents>,
    reach: T,
    hit: Option<(T, Contents)>,
    all_solid: bool,
}

impl<T> Component<T> {
    fn solid_trace(&self, query: TraceQuery) -> Option<Trace> {
        if !self.all_solid {
            return None;
        }
        let mut trace = Trace::clear(query.end);
        trace.start_solid = true;
        trace.all_solid = true;
        trace.contents = self.start_contents?;
        if query.rules.all_solid == AllSolid::BlockAtStart {
            trace.fraction = 0.0;
            trace.end = query.start;
        }
        Some(trace)
    }
}

/// One interval-union reducer serves both polyhedral and rounded shapes.
/// Re-enumeration stays bounded by cells and work, without a query allocation.
fn component<T: Parameter>(
    cells: u32,
    include_isolated: bool,
    work: &mut Work,
    mut intervals: impl FnMut(
        &mut Work,
        &mut dyn FnMut(Interval<T>, Contents, &mut Work) -> Result<(), Failure>,
    ) -> Result<(), Failure>,
) -> Result<Component<T>, Failure> {
    let mut result = Component {
        start_contents: None,
        reach: T::ZERO,
        hit: None,
        all_solid: false,
    };
    intervals(work, &mut |interval, contents, work| {
        if interval.contains(T::ZERO, work)? && result.start_contents.is_none() {
            result.start_contents = Some(contents);
        }
        if interval.contains(result.reach, work)?
            && interval.last.order(result.reach, work)? == Ordering::Greater
        {
            result.reach = interval.last;
        }
        if (!interval.isolated || include_isolated)
            && (result.hit.is_none()
                || interval
                    .first
                    .order(result.hit.map_or(T::ONE, |hit| hit.0), work)?
                    == Ordering::Less)
        {
            result.hit = Some((interval.first, contents));
        }
        Ok(())
    })?;
    if result.start_contents.is_none() {
        return Ok(result);
    }
    for _ in 0..=cells {
        if result.reach.order(T::ONE, work)? != Ordering::Less {
            break;
        }
        let old = result.reach;
        intervals(work, &mut |interval, _, work| {
            if interval.contains(result.reach, work)?
                && interval.last.order(result.reach, work)? == Ordering::Greater
            {
                result.reach = interval.last;
            }
            Ok(())
        })?;
        if result.reach.order(old, work)? == Ordering::Equal {
            break;
        }
    }
    result.all_solid = result.reach.order(T::ONE, work)? != Ordering::Less;
    if result.all_solid {
        return Ok(result);
    }
    result.hit = None;
    intervals(work, &mut |interval, contents, work| {
        if interval.first.order(result.reach, work)? == Ordering::Greater
            && (!interval.isolated || include_isolated)
            && (result.hit.is_none()
                || interval
                    .first
                    .order(result.hit.map_or(T::ONE, |hit| hit.0), work)?
                    == Ordering::Less)
        {
            result.hit = Some((interval.first, contents));
        }
        Ok(())
    })?;
    Ok(result)
}

/// The existing certificate stream eliminates the point variables once. Its
/// affine costs clip parameter space, including unbounded/degenerate cells.
fn interval(rows: &[Row], work: &mut Work) -> Result<Option<Interval>, Failure> {
    let (mut lower, mut upper) = (None::<Fraction>, None::<Fraction>);
    let mut empty = false;
    visit_certificates(rows, work, |indices, weights, _| {
        let cost = cost(rows, indices, weights)?;
        if cost.beta.is_zero() {
            empty = cost.alpha.negative();
        } else {
            let bound = Fraction::new(cost.alpha.neg(), cost.beta);
            if cost.beta.negative() {
                upper = Some(match upper {
                    Some(old) if bound.compare(old)? != Ordering::Less => old,
                    _ => bound,
                });
            } else {
                lower = Some(match lower {
                    Some(old) if bound.compare(old)? != Ordering::Greater => old,
                    _ => bound,
                });
            }
            if let (Some(lower), Some(upper)) = (lower, upper) {
                empty = lower.compare(upper)? == Ordering::Greater;
            }
        }
        Ok(!empty)
    })?;
    if empty {
        return Ok(None);
    }
    let isolated = match (lower, upper) {
        (Some(lower), Some(upper)) => lower.compare(upper)? == Ordering::Equal,
        _ => false,
    };
    let first = match lower {
        Some(lower) if lower.compare(Fraction::ZERO)? == Ordering::Greater => lower,
        _ => Fraction::ZERO,
    };
    let last = match upper {
        Some(upper) if upper.compare(Fraction::ONE)? == Ordering::Less => upper,
        _ => Fraction::ONE,
    };
    Ok(
        (first.compare(last)? != Ordering::Greater).then_some(Interval {
            first,
            last,
            isolated,
        }),
    )
}

fn dot(a: &[Int; 3], b: &[Int; 3]) -> Result<Int, Failure> {
    let mut result = Int::ZERO;
    for axis in 0..3 {
        result = result.add(&a[axis].mul(&b[axis])?)?;
    }
    Ok(result)
}

fn cross(a: &[Int; 3], b: &[Int; 3]) -> Result<[Int; 3], Failure> {
    Ok([
        a[1].mul(&b[2])?.add(&a[2].mul(&b[1])?.neg())?,
        a[2].mul(&b[0])?.add(&a[0].mul(&b[2])?.neg())?,
        a[0].mul(&b[1])?.add(&a[1].mul(&b[0])?.neg())?,
    ])
}

fn zero(vector: &[Int; 3]) -> bool {
    vector.iter().all(|component| component.is_zero())
}

fn parallel(a: &[Int; 3], b: &[Int; 3]) -> Result<bool, Failure> {
    Ok(zero(&cross(a, b)?) && !dot(a, b)?.negative())
}

#[derive(Clone, Copy, Default)]
struct Contact {
    cell: u32,
    normal: [Int; 3],
    source_distance: Int,
    cost: Cost,
    relative: bool,
    affine: bool,
}

/// A constraint is an affine equality when its opposite is in the positive
/// cone of the cell's active normals. The existing exact certificate supplies
/// that proof, including equalities implied by three or four authored faces.
fn affine_constraint(cell: &[Contact], index: usize, work: &mut Work) -> Result<bool, Failure> {
    let normal = cell[index].normal;
    let opposite = normal.map(Int::neg);
    for other in cell {
        work.item()?;
        if parallel(&opposite, &other.normal)? {
            return Ok(true);
        }
    }
    let mut answer = false;
    for count in 3..=4 {
        combinations(cell.len() - 1, count - 1, |indices| {
            work.item()?;
            let mut columns = [[Int::ZERO; 4]; 4];
            columns[0][..3].copy_from_slice(&normal);
            columns[0][3] = Int::ONE;
            for (column, &other) in indices.iter().enumerate() {
                let other = other + usize::from(other >= index);
                columns[column + 1][..3].copy_from_slice(&cell[other].normal);
                columns[column + 1][3] = Int::ONE;
            }
            answer =
                certificate(&columns, count, work)?.is_some_and(|weights| !weights[0].is_zero());
            Ok(!answer)
        })?;
        if answer {
            break;
        }
    }
    Ok(answer)
}

impl Contact {
    fn load(
        cell: usize,
        rows: &[Row],
        source_count: usize,
        indices: &[usize],
        weights: &[Int; 4],
    ) -> Result<Self, Failure> {
        let mut result = Self {
            cell: cell as u32,
            cost: cost(rows, indices, weights)?,
            ..Self::default()
        };
        for (column, &index) in indices.iter().enumerate() {
            if index < source_count {
                for axis in 0..3 {
                    result.normal[axis] = result.normal[axis]
                        .add(&weights[column].mul(&rows[index].normal[axis])?)?;
                }
                result.source_distance = result
                    .source_distance
                    .add(&weights[column].mul(&rows[index].distance)?)?;
            }
        }
        Ok(result)
    }

    fn plane(self) -> Result<Plane, Failure> {
        let bits = self
            .normal
            .iter()
            .map(Int::bit_length)
            .max()
            .ok_or(Failure::Input)?;
        let mut normal = Vec3(self.normal.map(|value| value.scaled_f64(bits) as f32));
        let length = math::normalize(&mut normal);
        let distance = (self.source_distance.scaled_f64(bits + 149) / f64::from(length)) as f32;
        if length == 0.0 || !length.is_finite() || !normal.is_finite() || !distance.is_finite() {
            return Err(Failure::Input);
        }
        Ok(Plane::oriented(normal, distance))
    }
}

/// Only contact-local active normals are retained. On a candidate facet,
/// pairwise cross rays delimit the open tangent sectors; their positive sums
/// witness sector interiors. Affine equalities constrain relative sectors,
/// including zero-dimensional facets. The lexicographic tangent/normal
/// perturbation must leave every cell, excluding partitions and redundant
/// vertex-only supports from either kind of facet.
fn exterior(
    candidate: Contact,
    active: &[Contact],
    tangent: &[Int; 3],
    work: &mut Work,
) -> Result<bool, Failure> {
    if zero(tangent) && !candidate.relative {
        return Ok(false);
    }
    let mut equality = false;
    for row in active.iter().filter(|row| row.cell == candidate.cell) {
        work.item()?;
        if row.affine {
            equality = true;
            // An affine-hull equality is retained in both orientations. A
            // relative tangent must stay on it rather than enter a 3D interior.
            if !dot(&row.normal, tangent)?.is_zero() {
                return Ok(false);
            }
        } else if !parallel(&candidate.normal, &row.normal)?
            && !dot(&row.normal, tangent)?.negative()
        {
            return Ok(false);
        }
    }
    // A relative facet can be zero-dimensional. Ordinary cells still require
    // an open facet sector, so a vertex-only support cannot become a surface.
    if zero(tangent) && !equality {
        return Ok(false);
    }
    for cell in active.chunk_by(|a, b| a.cell == b.cell) {
        let mut outside = false;
        for row in cell {
            work.item()?;
            let along = dot(&row.normal, tangent)?;
            let sign = if along.is_zero() {
                dot(&row.normal, &candidate.normal)?
            } else {
                along
            };
            if !sign.negative() && !sign.is_zero() {
                outside = true;
                break;
            }
        }
        if !outside {
            return Ok(false);
        }
    }
    Ok(true)
}

fn exterior_contact(active: &[Contact], work: &mut Work) -> Result<Contact, Failure> {
    for &candidate in active {
        if candidate.cost.beta.negative() || candidate.cost.beta.is_zero() {
            continue;
        }
        if candidate.relative && exterior(candidate, active, &[Int::ZERO; 3], work)? {
            return Ok(candidate);
        }
        for axis in 0..3 {
            let mut unit = [Int::ZERO; 3];
            unit[axis] = Int::ONE;
            let ray = cross(&candidate.normal, &unit)?;
            if exterior(candidate, active, &ray, work)?
                || exterior(candidate, active, &ray.map(Int::neg), work)?
            {
                return Ok(candidate);
            }
        }
        for (index, first) in active.iter().enumerate() {
            work.item()?;
            let a = cross(&candidate.normal, &first.normal)?;
            if zero(&a) {
                continue;
            }
            if exterior(candidate, active, &a, work)?
                || exterior(candidate, active, &a.map(Int::neg), work)?
            {
                return Ok(candidate);
            }
            for second in &active[index + 1..] {
                work.item()?;
                let b = cross(&candidate.normal, &second.normal)?;
                if zero(&b) {
                    continue;
                }
                for negative_a in [false, true] {
                    for negative_b in [false, true] {
                        let mut tangent = [Int::ZERO; 3];
                        for axis in 0..3 {
                            let a = if negative_a { a[axis].neg() } else { a[axis] };
                            let b = if negative_b { b[axis].neg() } else { b[axis] };
                            tangent[axis] = a.add(&b)?;
                        }
                        if exterior(candidate, active, &tangent, work)? {
                            return Ok(candidate);
                        }
                    }
                }
            }
        }
    }
    Err(Failure::Input)
}

struct Pose {
    origin: [Int; 3],
    axes: [[Int; 3]; 3],
}

struct BoxPath {
    start: [Int; 3],
    delta: [Int; 3],
    bounds: [[Int; 3]; 2],
}

impl BoxPath {
    fn full_volume(&self) -> bool {
        self.bounds[0]
            .iter()
            .zip(&self.bounds[1])
            .all(|(min, max)| min.compare(max) == Ordering::Less)
    }
}

fn backoff(contact: Contact, epsilon: Option<usize>, work: &mut Work) -> Result<f32, Failure> {
    let normal_squared = dot(&contact.normal, &contact.normal)?;
    // At a binary32 parameter the cost has denominator S=2^149; center
    // coordinates contribute another S. Squaring physical separation gives
    // |normal|^2*S^4. Native dyadic epsilons compare without a square root.
    let required = normal_squared.shift(596)?;
    let mut permitted = |bits: u32| -> Result<bool, Failure> {
        work.item()?;
        let residual = contact
            .cost
            .at(Fraction::binary32(f32::from_bits(bits))?)?
            .neg();
        if residual.negative() || residual.is_zero() {
            return Ok(false);
        }
        match epsilon {
            Some(power) => Ok(residual
                .mul(&residual)?
                .shift(power * 2)?
                .compare(&required)
                != Ordering::Less),
            None => Ok(true),
        }
    };
    // Native per-plane clamp can stop at a clear start closer than epsilon.
    if !permitted(0)? {
        return Ok(0.0);
    }
    let (mut lower, mut upper) = (0u32, 1.0f32.to_bits() + 1);
    while lower + 1 < upper {
        let middle = lower + (upper - lower) / 2;
        if permitted(middle)? {
            lower = middle;
        } else {
            upper = middle;
        }
    }
    Ok(f32::from_bits(lower))
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
    full_dim: bool,
}

pub(super) struct Data {
    cells: Vec<Cell>,
    sides: Vec<Side>,
    rows: usize,
    contacts: usize,
    query_work: usize,
}

pub(super) struct Prepared {
    pub data: Data,
    pub models: Vec<IndexSpan>,
}

pub(super) struct Scratch {
    rows: Box<[Row]>,
    contacts: Box<[Contact]>,
}

impl Scratch {
    pub fn new(rows: usize, contacts: usize) -> Self {
        Self {
            rows: vec![Row::default(); rows].into_boxed_slice(),
            contacts: vec![Contact::default(); contacts].into_boxed_slice(),
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

struct Solution {
    numerator: [Int; 4],
    denominator: Int,
}

fn solve(
    matrix: &[[Int; 4]; 4],
    rhs: &[Int; 4],
    count: usize,
    work: &mut Work,
) -> Result<Option<Solution>, Failure> {
    let mut denominator = determinant(matrix, count, work)?;
    if denominator.is_zero() {
        return Ok(None);
    }
    let mut numerator = [Int::ZERO; 4];
    for column in 0..count {
        let mut replaced = *matrix;
        for row in 0..count {
            replaced[row][column] = rhs[row];
        }
        numerator[column] = determinant(&replaced, count, work)?;
    }
    if denominator.negative() {
        denominator = denominator.neg();
        for value in &mut numerator[..count] {
            *value = value.neg();
        }
    }
    Ok(Some(Solution {
        numerator,
        denominator,
    }))
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
        let mut rhs = [Int::ZERO; 4];
        for (row, &source) in selected.iter().enumerate() {
            rhs[row] = if source == 3 { Int::ONE } else { Int::ZERO };
        }
        let Some(Solution {
            numerator: weights,
            denominator,
        }) = solve(&matrix, &rhs, count, work)?
        else {
            return Ok(true);
        };
        if weights[..count].iter().any(|weight| weight.negative()) {
            return Ok(false);
        }
        for row in 0..4 {
            // Cramer's rule already proves selected rows exactly. Only omitted
            // rows need the full-system check; this also avoids an unnecessary
            // fourth-degree product for a complete four-row basis.
            if selected.contains(&row) {
                continue;
            }
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

fn query_rows(sides: usize) -> Result<usize, Failure> {
    // Eliminating one centered segment parameter adds one row for each pair
    // of opposite slope signs. Its largest partition has floor(N*N/4) pairs.
    let capsule = sides
        .checked_mul(sides)
        .and_then(|pairs| sides.checked_add(pairs / 4))
        .ok_or(Failure::Budget)?;
    Ok(capsule.max(sides.checked_add(6).ok_or(Failure::Budget)?))
}

/// The Motzkin alternative handles cold mixed-strict paths and hot closed
/// intersections. A certificate needs at most four rows in three dimensions.
/// For posed binary32 rows, |normal|<3*M^2 and |distance|<10*M^3, M=2^277.
/// Every stationary certificate distance is <6480*M^9 (2506 bits). Moving
/// poses restrict |T|<=2; certificates with affine costs contain a box row,
/// hence at most three source rows. Ratio comparisons stay below 3898 bits;
/// active-normal cross/ray-dot predicates stay below 3875 bits. All operations
/// remain checked against the same 64 limbs, including unusual finite data.
fn visit_certificates(
    rows: &[Row],
    work: &mut Work,
    mut visit: impl FnMut(&[usize], &[Int; 4], &mut Work) -> Result<bool, Failure>,
) -> Result<(), Failure> {
    let mut keep = true;
    for count in 1..=4 {
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
            keep = visit(indices, &weights, work)?;
            Ok(keep)
        })?;
        if !keep {
            break;
        }
    }
    Ok(())
}

/// A primal witness is accepted only by the same exact row arithmetic. Source
/// cells use physical coordinates (scale 0); posed/query rows use P=2^149*x.
fn witness_violation(
    rows: &[Row],
    candidate: Vec3,
    coordinate_scale: usize,
    work: &mut Work,
) -> Result<Option<usize>, Failure> {
    let candidate = vector(candidate)?;
    let shift = 149usize
        .checked_sub(coordinate_scale)
        .ok_or(Failure::Input)?;
    for (index, row) in rows.iter().enumerate() {
        work.item()?;
        let order = dot(&row.normal, &candidate)?.compare(&row.distance.shift(shift)?);
        if order == Ordering::Greater || row.strict && order == Ordering::Equal {
            return Ok(Some(index));
        }
    }
    Ok(None)
}

/// Floating arithmetic proposes a point only. Underflow, rounding and failed
/// projections cannot establish occupancy or reject a source cell.
fn project_witness(
    row: &Row,
    candidate: Vec3,
    coordinate_scale: usize,
    force: bool,
) -> Option<Vec3> {
    let shift = row
        .normal
        .iter()
        .map(Int::bit_length)
        .max()
        .filter(|shift| *shift != 0)?;
    let normal = Vec3(row.normal.map(|value| value.scaled_f64(shift) as f32));
    let distance = row
        .distance
        .scaled_f64(shift.checked_add(coordinate_scale)?) as f32;
    let error = normal.dot(candidate) - distance;
    if !normal.is_finite() || !distance.is_finite() || !error.is_finite() {
        return None;
    }
    if !force && (error < 0.0 || error == 0.0 && !row.strict) {
        return Some(candidate);
    }
    // Max-bit scaling leaves at least one component in [0.5, 1], so this
    // finite squared length is nonzero even if smaller components underflow.
    let mut projected = candidate - normal * (error.max(0.0) / normal.dot(normal));
    for axis in 0..3 {
        projected.0[axis] = if normal.0[axis] > 0.0 {
            projected.0[axis].next_down()
        } else if normal.0[axis] < 0.0 {
            projected.0[axis].next_up()
        } else {
            projected.0[axis]
        };
    }
    projected.is_finite().then_some(projected)
}

fn feasible(rows: &[Row], coordinate_scale: usize, work: &mut Work) -> Result<bool, Failure> {
    let mut candidate = Vec3::default();
    let Some(mut violated) = witness_violation(rows, candidate, coordinate_scale, work)? else {
        return Ok(true);
    };
    // Bound proposal work independently of convergence or cell geometry. A
    // missed narrow/rank-deficient witness retains the exact certificate path.
    for _ in 0..8 {
        let before = candidate.0.map(f32::to_bits);
        let mut projected = true;
        for (index, row) in rows.iter().enumerate() {
            work.item()?;
            let Some(next) = project_witness(row, candidate, coordinate_scale, index == violated)
            else {
                projected = false;
                break;
            };
            candidate = next;
        }
        if !projected {
            break;
        }
        let Some(next_violated) = witness_violation(rows, candidate, coordinate_scale, work)?
        else {
            return Ok(true);
        };
        violated = next_violated;
        if candidate.0.map(f32::to_bits) == before {
            break;
        }
    }
    let mut answer = true;
    visit_certificates(rows, work, |indices, weights, _| {
        let cost = cost(rows, indices, weights)?;
        answer = !(cost.alpha.negative() || cost.alpha.is_zero() && cost.strict);
        Ok(answer)
    })?;
    Ok(answer)
}

fn vector(value: Vec3) -> Result<[Int; 3], Failure> {
    Ok([
        Int::binary32(value.0[0])?,
        Int::binary32(value.0[1])?,
        Int::binary32(value.0[2])?,
    ])
}

fn difference(start: &[Int; 3], end: &[Int; 3]) -> Result<[Int; 3], Failure> {
    let mut delta = [Int::ZERO; 3];
    for axis in 0..3 {
        delta[axis] = end[axis].add(&start[axis].neg())?;
    }
    Ok(delta)
}

struct CapsulePath {
    start: [Int; 3],
    delta: [Int; 3],
    offset: [Int; 3],
    radius: Int,
}

impl CapsulePath {
    fn new(query: TraceQuery, capsule: Capsule) -> Result<Self, Failure> {
        let center = math::midpoint(query.mins, query.maxs);
        let start = vector(query.start + center)?;
        let end = vector(query.end + center)?;
        Ok(Self {
            start,
            delta: difference(&start, &end)?,
            offset: vector(capsule.offset)?,
            radius: Int::binary32(capsule.radius)?,
        })
    }
}

fn source_row(planes: &[Plane], side: Side) -> Result<Row, Failure> {
    let plane = planes[side.plane as usize];
    let mut row = Row {
        normal: vector(plane.normal)?,
        distance: Int::binary32(plane.distance)?,
        strict: side.strict,
        slope: Int::ZERO,
    };
    if side.inverted {
        row.normal = row.normal.map(Int::neg);
        row.distance = row.distance.neg();
    }
    Ok(row)
}

/// Reduce only positively parallel path inequalities. Their tightest bound
/// represents the same halfspace intersection; equal bounds keep the first
/// source plane and combine strict ownership. Opposing normals stay separate.
/// The unpublished tail of `sides` is the workspace for this path's provenance.
fn reduce_path(
    planes: &[Plane],
    path: &[Side],
    rows: &mut [Row],
    sides: &mut Vec<Side>,
    work: &mut Work,
) -> Result<usize, Failure> {
    let first = sides.len();
    let mut count = 0;
    'source: for &side in path {
        work.item()?;
        let row = source_row(planes, side)?;
        for index in 0..count {
            work.item()?;
            let old = &mut rows[index];
            if !parallel(&old.normal, &row.normal)? {
                continue;
            }
            let axis = old
                .normal
                .iter()
                .position(|component| !component.is_zero())
                .ok_or(Failure::Input)?;
            let old_scale = if old.normal[axis].negative() {
                old.normal[axis].neg()
            } else {
                old.normal[axis]
            };
            let new_scale = if row.normal[axis].negative() {
                row.normal[axis].neg()
            } else {
                row.normal[axis]
            };
            match row
                .distance
                .mul(&old_scale)?
                .compare(&old.distance.mul(&new_scale)?)
            {
                Ordering::Less => {
                    *old = row;
                    sides[first + index] = side;
                }
                Ordering::Equal => {
                    old.strict |= row.strict;
                    sides[first + index].strict = old.strict;
                }
                Ordering::Greater => {}
            }
            continue 'source;
        }
        let target = rows.get_mut(count).ok_or(Failure::Scratch)?;
        *target = row;
        if sides.len() == sides.capacity() {
            return Err(Failure::Budget);
        }
        sides.push(side);
        count += 1;
    }
    Ok(count)
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
            let contents = Contents::from_q1(frame.node, Contents::EMPTY);
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

enum CellSource<'a> {
    Bsp {
        nodes: &'a [ClipNode],
        root: &'a dyn Fn(usize) -> i32,
    },
    Authored {
        brushes: &'a [Brush],
        models: &'a [HullModel],
    },
}

impl CellSource<'_> {
    fn visit(
        &self,
        model: usize,
        path: &mut Vec<Side>,
        pending: &mut Vec<Pending>,
        work: &mut Work,
        mut leaf: impl FnMut(&[Side], Contents, &mut Work) -> Result<(), Failure>,
    ) -> Result<(), Failure> {
        match self {
            Self::Bsp { nodes, root } => paths(nodes, root(model), path, pending, work, leaf),
            Self::Authored { brushes, models } => {
                for brush in &brushes[models[model].brushes.range()] {
                    work.item()?;
                    if brush.contents == Contents::EMPTY {
                        continue;
                    }
                    path.clear();
                    for plane in brush.sides.range() {
                        path.push(Side {
                            plane: plane as u32,
                            inverted: false,
                            strict: false,
                        });
                    }
                    leaf(path, brush.contents, work)?;
                }
                Ok(())
            }
        }
    }
}

pub(super) fn prepare(
    planes: &[Plane],
    nodes: &[ClipNode],
    model_count: usize,
    root: impl Fn(usize) -> i32,
    depth: usize,
    limits: HullPositionLimits,
) -> Result<Prepared, Failure> {
    prepare_source(
        planes,
        CellSource::Bsp { nodes, root: &root },
        model_count,
        depth,
        limits,
    )
}

pub(super) fn prepare_brushes(
    planes: &[Plane],
    brushes: &[Brush],
    models: &[HullModel],
    limits: HullPositionLimits,
) -> Result<Prepared, Failure> {
    if planes.iter().any(|plane| !plane.is_valid())
        || brushes.iter().any(|brush| !brush.sides.fits(planes.len()))
        || models
            .iter()
            .any(|model| !model.brushes.fits(brushes.len()))
    {
        return Err(Failure::Input);
    }
    let depth = brushes
        .iter()
        .map(|brush| brush.sides.count as usize)
        .max()
        .unwrap_or(0);
    prepare_source(
        planes,
        CellSource::Authored { brushes, models },
        models.len(),
        depth,
        limits,
    )
}

fn prepare_source(
    planes: &[Plane],
    source: CellSource<'_>,
    model_count: usize,
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
        source.visit(model, &mut path, &mut pending, &mut work, |path, _, _| {
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
        })?;
    }
    let rows = greatest.checked_add(6).ok_or(Failure::Budget)?;
    let mut workspace = reserve::<Row>(rows, &mut held, limits.bytes)?;
    workspace.resize(rows, Row::default());
    let mut retained_cells = reserve::<Cell>(cells, &mut held, limits.bytes)?;
    let mut retained_sides = reserve::<Side>(sides, &mut held, limits.bytes)?;
    let mut models = reserve::<IndexSpan>(model_count, &mut held, limits.bytes)?;
    // Account the future caller's typed contact arena while all builder arrays
    // are owned. This reservation is not another published geometry table.
    let _contacts = reserve::<Contact>(limits.contact_rows, &mut held, limits.bytes)?;
    let mut retained_greatest = 0;
    for model in 0..model_count {
        let first = retained_cells.len();
        source.visit(
            model,
            &mut path,
            &mut pending,
            &mut work,
            |path, contents, work| {
                let first = retained_sides.len();
                let count = reduce_path(planes, path, &mut workspace, &mut retained_sides, work)?;
                // All source normals are nonzero. Strict satisfaction of every
                // retained inequality is exactly a 3D-interior certificate,
                // and also proves the original mixed-strict path nonempty.
                for row in &mut workspace[..count] {
                    row.strict = true;
                }
                let full_dim = feasible(&workspace[..count], 0, work)?;
                let keep = if full_dim {
                    true
                } else {
                    for (row, side) in workspace[..count].iter_mut().zip(&retained_sides[first..]) {
                        row.strict = side.strict;
                    }
                    feasible(&workspace[..count], 0, work)?
                };
                if keep {
                    retained_greatest = retained_greatest.max(count);
                    retained_cells.push(Cell {
                        sides: IndexSpan {
                            first: first as u32,
                            count: count as u32,
                        },
                        contents,
                        full_dim,
                    });
                } else {
                    retained_sides.truncate(first);
                }
                Ok(())
            },
        )?;
        models.push(IndexSpan {
            first: first as u32,
            count: (retained_cells.len() - first) as u32,
        });
    }
    let query_rows = query_rows(retained_greatest)?;
    // Include the eventual query workspace in the admission budget while
    // builder and retained arrays are still owned. It reuses the same row
    // arena for boxes and capsule extrusion, rather than another geometry.
    let _extra_rows = reserve::<Row>(query_rows.saturating_sub(rows), &mut held, limits.bytes)?;
    Ok(Prepared {
        data: Data {
            cells: retained_cells,
            sides: retained_sides,
            rows: query_rows,
            contacts: limits.contact_rows,
            query_work: limits.query_work,
        },
        models,
    })
}

impl Data {
    pub fn point_contents(
        &self,
        planes: &[Plane],
        cells: IndexSpan,
        point: Vec3,
    ) -> Result<Contents, Failure> {
        let point = vector(point)?;
        let mut contents = Contents::EMPTY;
        let mut work = Work {
            remaining: self.query_work,
        };
        for cell in &self.cells[cells.range()] {
            work.item()?;
            let mut inside = true;
            for &side in &self.sides[cell.sides.range()] {
                work.item()?;
                let row = source_row(planes, side)?;
                let distance = dot(&row.normal, &point)?.add(&row.distance.shift(149)?.neg())?;
                if !distance.negative() && (!distance.is_zero() || row.strict) {
                    inside = false;
                    break;
                }
            }
            if inside {
                contents |= cell.contents;
            }
        }
        Ok(contents)
    }

    pub fn scratch_rows(&self) -> usize {
        self.rows
    }

    pub fn scratch_contacts(&self) -> usize {
        self.contacts
    }

    fn fill_source_rows<'a>(
        &self,
        planes: &[Plane],
        cell: Cell,
        pose: &Pose,
        rows: &'a mut [Row],
    ) -> Result<&'a mut [Row], Failure> {
        let rows = &mut rows[..cell.sides.count as usize];
        for (row, &side) in rows.iter_mut().zip(&self.sides[cell.sides.range()]) {
            let source = source_row(planes, side)?;
            let mut transformed = Row {
                distance: source.distance.shift(298)?,
                ..Row::default()
            };
            for component in 0..3 {
                for local in 0..3 {
                    transformed.normal[component] = transformed.normal[component]
                        .add(&source.normal[local].mul(&pose.axes[local][component])?)?;
                }
                transformed.distance = transformed
                    .distance
                    .add(&transformed.normal[component].mul(&pose.origin[component])?)?;
            }
            *row = transformed;
        }
        Ok(rows)
    }

    fn fill_rows<'a>(
        &self,
        planes: &[Plane],
        cell: Cell,
        path: &BoxPath,
        pose: &Pose,
        rows: &'a mut [Row],
    ) -> Result<&'a [Row], Failure> {
        self.fill_source_rows(planes, cell, pose, rows)?;
        let rows = &mut rows[..cell.sides.count as usize + 6];
        for axis in 0..3 {
            for side in 0..2 {
                let mut row = Row::default();
                row.normal[axis] = if side == 0 { Int::ONE.neg() } else { Int::ONE };
                row.distance = path.start[axis].add(&path.bounds[side][axis])?;
                row.slope = path.delta[axis];
                if side == 0 {
                    row.distance = row.distance.neg();
                    row.slope = row.slope.neg();
                }
                rows[cell.sides.count as usize + axis * 2 + side] = row;
            }
        }
        Ok(rows)
    }

    fn fill_capsule_rows<'a>(
        &self,
        planes: &[Plane],
        cell: Cell,
        offset: &[Int; 3],
        pose: &Pose,
        rows: &'a mut [Row],
        work: &mut Work,
    ) -> Result<&'a [Row], Failure> {
        let source_count = cell.sides.count as usize;
        for row in self.fill_source_rows(planes, cell, pose, rows)? {
            work.item()?;
            row.reduce()?;
            row.slope = dot(&row.normal, offset)?;
        }
        let mut count = source_count;
        // Exact Fourier-Motzkin elimination of u in [-1,1] from
        // n.(p-u*offset) <= distance. Sphere inflation then tests the distance
        // to this extruded cell, including edges and corners.
        for i in 0..source_count {
            if rows[i].slope.is_zero() || rows[i].slope.negative() {
                continue;
            }
            for j in 0..source_count {
                if !rows[j].slope.negative() {
                    continue;
                }
                work.item()?;
                let positive = rows[i].slope;
                let negative = rows[j].slope.neg();
                let mut row = Row {
                    distance: negative
                        .mul(&rows[i].distance)?
                        .add(&positive.mul(&rows[j].distance)?)?,
                    ..Row::default()
                };
                for axis in 0..3 {
                    row.normal[axis] = negative
                        .mul(&rows[i].normal[axis])?
                        .add(&positive.mul(&rows[j].normal[axis])?)?;
                }
                if zero(&row.normal) {
                    if row.distance.negative() {
                        return Err(Failure::Arithmetic);
                    }
                    continue;
                }
                row.reduce()?;
                *rows.get_mut(count).ok_or(Failure::Scratch)? = row;
                count += 1;
            }
        }
        for row in &mut rows[..source_count] {
            let slope = if row.slope.negative() {
                row.slope.neg()
            } else {
                row.slope
            };
            row.distance = row.distance.add(&slope)?;
            row.slope = Int::ZERO;
            row.reduce()?;
        }
        Ok(&rows[..count])
    }

    fn capsule_occupancy(
        &self,
        planes: &[Plane],
        cells: IndexSpan,
        query: TraceQuery,
        capsule: Capsule,
        pose: &Pose,
        rows: &mut [Row],
        work: &mut Work,
    ) -> Result<Option<Contents>, Failure> {
        let center = vector(query.start + math::midpoint(query.mins, query.maxs))?;
        let offset = vector(capsule.offset)?;
        let radius = Int::binary32(capsule.radius)?;
        for &cell in &self.cells[cells.range()] {
            work.item()?;
            if !cell.contents.intersects(query.mask) {
                continue;
            }
            let rows = self.fill_capsule_rows(planes, cell, &offset, pose, rows, work)?;
            if sphere::occupancy(rows, &center, radius, work)? {
                return Ok(Some(cell.contents));
            }
        }
        Ok(None)
    }

    fn capsule_intervals(
        &self,
        planes: &[Plane],
        cells: IndexSpan,
        mask: Contents,
        path: &CapsulePath,
        pose: &Pose,
        rows: &mut [Row],
        work: &mut Work,
        mut visit: impl FnMut(sphere::SweepInterval, Contents, usize, &mut Work) -> Result<(), Failure>,
    ) -> Result<(), Failure> {
        for index in cells.range() {
            work.item()?;
            let cell = self.cells[index];
            if !cell.contents.intersects(mask) {
                continue;
            }
            let rows = self.fill_capsule_rows(planes, cell, &path.offset, pose, rows, work)?;
            if let Some(interval) =
                sphere::interval(rows, &path.start, &path.delta, path.radius, work)?
            {
                visit(interval, cell.contents, index, work)?;
            }
        }
        Ok(())
    }

    fn moving_capsule(
        &self,
        planes: &[Plane],
        cells: IndexSpan,
        query: TraceQuery,
        capsule: Capsule,
        pose: &Pose,
        scratch: &mut Scratch,
        work: &mut Work,
    ) -> Result<Trace, Failure> {
        if !query.rules.contact_epsilon.is_finite() || query.rules.contact_epsilon < 0.0 {
            return Err(Failure::Input);
        }
        let path = CapsulePath::new(query, capsule)?;
        let component = component(
            cells.count,
            query.rules.contact_epsilon != 0.0,
            work,
            |work, visit| {
                self.capsule_intervals(
                    planes,
                    cells,
                    query.mask,
                    &path,
                    pose,
                    &mut scratch.rows,
                    work,
                    |interval, contents, _, work| visit(interval, contents, work),
                )
            },
        )?;
        if let Some(trace) = component.solid_trace(query) {
            return Ok(trace);
        }
        let start_solid = component.start_contents.is_some();
        let Some((parameter, contents)) = component.hit else {
            let endpoint = TraceQuery {
                start: query.end,
                ..query
            };
            if self
                .capsule_occupancy(
                    planes,
                    cells,
                    endpoint,
                    capsule,
                    pose,
                    &mut scratch.rows,
                    work,
                )?
                .is_some()
            {
                return Err(Failure::Input);
            }
            let mut trace = Trace::clear(query.end);
            trace.start_solid = start_solid;
            return Ok(trace);
        };
        let mut selected = None;
        self.capsule_intervals(
            planes,
            cells,
            query.mask,
            &path,
            pose,
            &mut scratch.rows,
            work,
            |interval, _, index, work| {
                if selected.is_none() && interval.first.compare(parameter, work)? == Ordering::Equal
                {
                    selected = Some(index);
                }
                Ok(())
            },
        )?;
        let cell = self.cells[selected.ok_or(Failure::Arithmetic)?];
        let rows =
            self.fill_capsule_rows(planes, cell, &path.offset, pose, &mut scratch.rows, work)?;
        let padded = (f64::from(capsule.radius) + query.rules.contact_epsilon) as f32;
        if !padded.is_finite() {
            return Err(Failure::Input);
        }
        let padded =
            sphere::interval(rows, &path.start, &path.delta, Int::binary32(padded)?, work)?
                .ok_or(Failure::Arithmetic)?;
        let mut fraction = padded.first.floor(work)?;
        loop {
            work.item()?;
            let value = sphere::Bound::Rational(Fraction::binary32(fraction)?);
            if value.compare(parameter, work)? != Ordering::Less {
                if fraction == 0.0 {
                    return Err(Failure::Input);
                }
                fraction = fraction.next_down();
                continue;
            }
            if start_solid && value.compare(component.reach, work)? != Ordering::Greater {
                return Err(Failure::Input);
            }
            let end = query.start.lerp(query.end, fraction);
            let endpoint = TraceQuery {
                start: end,
                end,
                ..query
            };
            if self
                .capsule_occupancy(
                    planes,
                    cells,
                    endpoint,
                    capsule,
                    pose,
                    &mut scratch.rows,
                    work,
                )?
                .is_none()
            {
                let center = end + math::midpoint(query.mins, query.maxs);
                let rows = self.fill_capsule_rows(
                    planes,
                    cell,
                    &path.offset,
                    pose,
                    &mut scratch.rows,
                    work,
                )?;
                let nearest = sphere::closest(rows, &vector(center)?, work)?;
                let shift = nearest.denominator.bit_length();
                let denominator = nearest.denominator.scaled_f64(shift);
                let displacement = Vec3(
                    nearest
                        .displacement
                        .map(|value| (value.scaled_f64(shift + 149) / denominator) as f32),
                );
                // Scale by the largest component using division before the
                // shared normalize. A truly axial projection stays exactly
                // +/-1 instead of losing a bit to length's reciprocal product.
                let largest = displacement
                    .0
                    .iter()
                    .map(|value| value.abs())
                    .fold(0.0f32, f32::max);
                let mut normal = displacement / largest;
                let distance = math::length(displacement);
                math::normalize(&mut normal);
                if distance == 0.0 || !distance.is_finite() || !normal.is_finite() {
                    return Err(Failure::Input);
                }
                let closing = dot(&vector(normal)?, &path.delta)?;
                if closing.is_zero() || !closing.negative() {
                    return Err(Failure::Input);
                }
                let plane = Plane::oriented(
                    normal,
                    normal.dot(center) - distance - normal.dot(capsule.offset).abs(),
                );
                if !plane.is_valid() {
                    return Err(Failure::Input);
                }
                let mut trace = Trace::clear(end);
                trace.fraction = fraction;
                trace.plane = plane;
                trace.start_solid = start_solid;
                trace.contents = contents;
                return Ok(trace);
            }
            if fraction == 0.0 {
                return Err(Failure::Input);
            }
            fraction = fraction.next_down();
        }
    }

    fn occupancy(
        &self,
        planes: &[Plane],
        cells: IndexSpan,
        mask: Contents,
        path: &BoxPath,
        pose: &Pose,
        rows: &mut [Row],
        work: &mut Work,
    ) -> Result<Option<Contents>, Failure> {
        for &cell in &self.cells[cells.range()] {
            work.item()?;
            if !cell.contents.intersects(mask) {
                continue;
            }
            let rows = self.fill_rows(planes, cell, path, pose, rows)?;
            if feasible(rows, 149, work)? {
                return Ok(Some(cell.contents));
            }
        }
        Ok(None)
    }

    fn intervals(
        &self,
        planes: &[Plane],
        cells: IndexSpan,
        mask: Contents,
        path: &BoxPath,
        pose: &Pose,
        rows: &mut [Row],
        work: &mut Work,
        mut visit: impl FnMut(Interval, Contents, &mut Work) -> Result<(), Failure>,
    ) -> Result<(), Failure> {
        for &cell in &self.cells[cells.range()] {
            work.item()?;
            if !cell.contents.intersects(mask) {
                continue;
            }
            let rows = self.fill_rows(planes, cell, path, pose, rows)?;
            if let Some(interval) = interval(rows, work)? {
                visit(interval, cell.contents, work)?;
            }
        }
        Ok(())
    }

    fn active_contacts<'a>(
        &self,
        planes: &[Plane],
        cells: IndexSpan,
        mask: Contents,
        path: &BoxPath,
        pose: &Pose,
        parameter: Fraction,
        scratch: &'a mut Scratch,
        work: &mut Work,
    ) -> Result<&'a [Contact], Failure> {
        let mut count = 0;
        for index in cells.range() {
            work.item()?;
            let cell = self.cells[index];
            if !cell.contents.intersects(mask) {
                continue;
            }
            let rows = self.fill_rows(planes, cell, path, pose, &mut scratch.rows)?;
            let Some(interval) = interval(rows, work)? else {
                continue;
            };
            if !interval.contains(parameter, work)? {
                continue;
            }
            let relative = !cell.full_dim && !path.full_volume();
            visit_certificates(rows, work, |indices, weights, work| {
                let mut contact =
                    Contact::load(index, rows, cell.sides.count as usize, indices, weights)?;
                contact.relative = relative;
                if zero(&contact.normal) || !contact.cost.at(parameter)?.is_zero() {
                    return Ok(true);
                }
                for old in scratch.contacts[..count]
                    .iter()
                    .filter(|old| old.cell == contact.cell)
                {
                    work.item()?;
                    if parallel(&old.normal, &contact.normal)? {
                        return Ok(true);
                    }
                }
                let target = scratch.contacts.get_mut(count).ok_or(Failure::Budget)?;
                *target = contact;
                count += 1;
                Ok(true)
            })?;
        }
        let mut first = 0;
        while first < count {
            let cell = scratch.contacts[first].cell;
            let end =
                first + scratch.contacts[first..count].partition_point(|row| row.cell == cell);
            if scratch.contacts[first].relative {
                for index in first..end {
                    let affine =
                        affine_constraint(&scratch.contacts[first..end], index - first, work)?;
                    scratch.contacts[index].affine = affine;
                }
            }
            first = end;
        }
        Ok(&scratch.contacts[..count])
    }

    fn moving(
        &self,
        planes: &[Plane],
        cells: IndexSpan,
        query: TraceQuery,
        path: &BoxPath,
        pose: &Pose,
        scratch: &mut Scratch,
        work: &mut Work,
    ) -> Result<Trace, Failure> {
        let epsilon = match query.rules.contact_epsilon {
            0.0 => None,
            0.03125 => Some(5),
            0.125 => Some(3),
            _ => return Err(Failure::Input),
        };
        let component = component(cells.count, epsilon.is_some(), work, |work, visit| {
            self.intervals(
                planes,
                cells,
                query.mask,
                path,
                pose,
                &mut scratch.rows,
                work,
                visit,
            )
        })?;
        if let Some(trace) = component.solid_trace(query) {
            return Ok(trace);
        }
        let start_solid = component.start_contents.is_some();
        let reach = component.reach;
        let hit = component.hit;
        let Some((parameter, contents)) = hit else {
            let endpoint = BoxPath {
                start: vector(query.end)?,
                delta: [Int::ZERO; 3],
                bounds: path.bounds,
            };
            if self
                .occupancy(
                    planes,
                    cells,
                    query.mask,
                    &endpoint,
                    pose,
                    &mut scratch.rows,
                    work,
                )?
                .is_some()
            {
                return Err(Failure::Input);
            }
            let mut trace = Trace::clear(query.end);
            trace.start_solid = start_solid;
            return Ok(trace);
        };
        let active = self.active_contacts(
            planes, cells, query.mask, path, pose, parameter, scratch, work,
        )?;
        let contact = exterior_contact(active, work)?;
        let mut fraction = backoff(contact, epsilon, work)?;
        loop {
            work.item()?;
            let value = Fraction::binary32(fraction)?;
            if value.compare(parameter)? != Ordering::Less
                || start_solid && value.compare(reach)? != Ordering::Greater
            {
                return Err(Failure::Input);
            }
            let end = query.start.lerp(query.end, fraction);
            if !end.is_finite() {
                return Err(Failure::Input);
            }
            let endpoint = BoxPath {
                start: vector(end)?,
                delta: [Int::ZERO; 3],
                bounds: path.bounds,
            };
            if self
                .occupancy(
                    planes,
                    cells,
                    query.mask,
                    &endpoint,
                    pose,
                    &mut scratch.rows,
                    work,
                )?
                .is_none()
            {
                let plane = contact.plane()?;
                // Finite endpoints can have a nonrepresentable f32
                // difference. Check the published normal against the exact
                // difference already owned by this path instead.
                let closing = dot(&vector(plane.normal)?, &path.delta)?;
                if closing.is_zero() || !closing.negative() {
                    return Err(Failure::Input);
                }
                let mut trace = Trace::clear(end);
                trace.fraction = fraction;
                trace.plane = plane;
                trace.start_solid = start_solid;
                trace.contents = contents;
                return Ok(trace);
            }
            if fraction == 0.0 {
                return Err(Failure::Input);
            }
            fraction = f32::from_bits(fraction.to_bits() - 1);
        }
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
        let query = TraceQuery {
            mask: query.effective_mask(),
            ..query
        };
        if !query.start.is_finite()
            || !query.end.is_finite()
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
        let moving = query.start != query.end;
        if moving
            && axes
                .iter()
                .any(|axis| axis.0.iter().any(|value| value.abs() > 2.0))
        {
            return Err(Failure::Input);
        }
        if moving && scratch.contacts.len() < self.contacts {
            return Err(Failure::Scratch);
        }
        let pose = Pose {
            origin: vector(origin)?,
            axes: [vector(axes[0])?, vector(axes[1])?, vector(axes[2])?],
        };
        let start = vector(query.start)?;
        let end = vector(query.end)?;
        let delta = difference(&start, &end)?;
        let path = BoxPath {
            start,
            delta,
            bounds: [vector(query.mins)?, vector(query.maxs)?],
        };
        let mut work = Work {
            remaining: self.query_work,
        };
        let mut matrix = [[Int::ZERO; 4]; 4];
        for axis in 0..3 {
            matrix[axis][..3].copy_from_slice(&pose.axes[axis]);
        }
        if determinant(&matrix, 3, &mut work)?.is_zero() {
            return Err(Failure::Input);
        }
        if let Some(capsule) = query.capsule {
            if !capsule.radius.is_finite() || capsule.radius < 0.0 || !capsule.offset.is_finite() {
                return Err(Failure::Input);
            }
            if moving {
                return self
                    .moving_capsule(planes, cells, query, capsule, &pose, scratch, &mut work);
            }
            let contents = self.capsule_occupancy(
                planes,
                cells,
                query,
                capsule,
                &pose,
                &mut scratch.rows,
                &mut work,
            )?;
            let mut trace = Trace::clear(query.end);
            if let Some(contents) = contents {
                trace.start_solid = true;
                trace.all_solid = true;
                trace.fraction = 0.0;
                trace.contents = contents;
            }
            return Ok(trace);
        }
        if moving {
            return self.moving(planes, cells, query, &path, &pose, scratch, &mut work);
        }
        if let Some(contents) = self.occupancy(
            planes,
            cells,
            query.mask,
            &path,
            &pose,
            &mut scratch.rows,
            &mut work,
        )? {
            let mut trace = Trace::clear(query.end);
            trace.start_solid = true;
            trace.all_solid = true;
            trace.fraction = 0.0;
            trace.contents = contents;
            return Ok(trace);
        }
        Ok(Trace::clear(query.end))
    }
}
