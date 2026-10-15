//! Sphere distance to the shared cell extruded by a capsule's segment.
use super::{Cost, Failure, Fraction, Int, Interval, Row, Work, combinations, dot, solve};
use std::cmp::Ordering;

struct Feature {
    displacement: [Int; 3],
    slope: [Int; 3],
    denominator: Int,
    weights: [Cost; 4],
}

impl Feature {
    fn fit(
        rows: &[Row],
        indices: &[usize],
        start: &[Int; 3],
        delta: Option<&[Int; 3]>,
        work: &mut Work,
    ) -> Result<Option<Self>, Failure> {
        let mut result = Self {
            displacement: [Int::ZERO; 3],
            slope: [Int::ZERO; 3],
            denominator: Int::ONE,
            weights: [Cost::default(); 4],
        };
        if indices.is_empty() {
            return Ok(Some(result));
        }
        let mut gram = [[Int::ZERO; 4]; 4];
        let mut rhs = [Int::ZERO; 4];
        for (a, &index) in indices.iter().enumerate() {
            let row = &rows[index];
            rhs[a] = dot(&row.normal, start)?.add(&row.distance.neg())?;
            for (b, &other) in indices.iter().enumerate() {
                gram[a][b] = dot(&row.normal, &rows[other].normal)?;
            }
        }
        let Some(constant) = solve(&gram, &rhs, indices.len(), work)? else {
            return Ok(None);
        };
        result.denominator = constant.denominator;
        let slope = if let Some(delta) = delta {
            for (a, &index) in indices.iter().enumerate() {
                rhs[a] = dot(&rows[index].normal, delta)?;
            }
            solve(&gram, &rhs, indices.len(), work)?
                .ok_or(Failure::Arithmetic)?
                .numerator
        } else {
            [Int::ZERO; 4]
        };
        for (column, &index) in indices.iter().enumerate() {
            result.weights[column] = Cost {
                alpha: constant.numerator[column],
                beta: slope[column],
                strict: false,
            };
            for axis in 0..3 {
                result.displacement[axis] = result.displacement[axis]
                    .add(&constant.numerator[column].mul(&rows[index].normal[axis])?)?;
                result.slope[axis] =
                    result.slope[axis].add(&slope[column].mul(&rows[index].normal[axis])?)?;
            }
        }
        Ok(Some(result))
    }

    fn region(
        &self,
        rows: &[Row],
        indices: &[usize],
        start: &[Int; 3],
        delta: Option<&[Int; 3]>,
        strict: bool,
        work: &mut Work,
    ) -> Result<Option<Interval>, Failure> {
        let mut region = Interval {
            first: Fraction::ZERO,
            last: Fraction::ONE,
            isolated: false,
            first_closed: true,
            last_closed: true,
        };
        for &weight in &self.weights[..indices.len()] {
            if !clip(&mut region, weight)? {
                return Ok(None);
            }
        }
        for (index, row) in rows.iter().enumerate() {
            work.item()?;
            if indices.contains(&index) {
                if strict && row.strict {
                    return Ok(None);
                }
                continue;
            }
            let error = row.distance.add(&dot(&row.normal, start)?.neg())?;
            let alpha = error
                .mul(&self.denominator)?
                .add(&dot(&row.normal, &self.displacement)?)?;
            let beta = if let Some(delta) = delta {
                dot(&row.normal, &self.slope)?
                    .add(&dot(&row.normal, delta)?.mul(&self.denominator)?.neg())?
            } else {
                Int::ZERO
            };
            if !clip(
                &mut region,
                Cost {
                    alpha,
                    beta,
                    strict: strict && row.strict,
                },
            )? {
                return Ok(None);
            }
        }
        region.isolated = region.first.compare(region.last)? == Ordering::Equal;
        Ok(Some(region))
    }

    fn polynomial(&self, radius: Int) -> Result<Polynomial, Failure> {
        let bound = radius.mul(&self.denominator)?;
        Polynomial::new([
            dot(&self.displacement, &self.displacement)?.add(&bound.mul(&bound)?.neg())?,
            dot(&self.displacement, &self.slope)?.shift(1)?,
            dot(&self.slope, &self.slope)?,
        ])
    }
}

fn clip(interval: &mut Interval, cost: Cost) -> Result<bool, Failure> {
    if cost.beta.is_zero() {
        return Ok(!cost.alpha.negative() && !(cost.strict && cost.alpha.is_zero()));
    }
    let bound = Fraction::new(cost.alpha.neg(), cost.beta);
    if cost.beta.negative() {
        match bound.compare(interval.last)? {
            Ordering::Less => {
                interval.last = bound;
                interval.last_closed = !cost.strict;
            }
            Ordering::Equal => interval.last_closed &= !cost.strict,
            _ => {}
        }
    } else {
        match bound.compare(interval.first)? {
            Ordering::Greater => {
                interval.first = bound;
                interval.first_closed = !cost.strict;
            }
            Ordering::Equal => interval.first_closed &= !cost.strict,
            _ => {}
        }
    }
    let order = interval.first.compare(interval.last)?;
    Ok(order == Ordering::Less
        || order == Ordering::Equal && interval.first_closed && interval.last_closed)
}

/// KKT distance projection. Admission proves that every retained cell is
/// nonempty, and conic Caratheodory supplies an independent basis of <=3 rows.
pub(super) struct Projection {
    pub displacement: [Int; 3],
    pub denominator: Int,
}

pub(super) fn closest(
    rows: &[Row],
    center: &[Int; 3],
    work: &mut Work,
) -> Result<Projection, Failure> {
    let mut answer = None;
    for count in 0..=3 {
        let mut visit = |indices: &[usize]| {
            work.item()?;
            let Some(feature) = Feature::fit(rows, indices, center, None, work)? else {
                return Ok(true);
            };
            if feature
                .region(rows, indices, center, None, false, work)?
                .is_none()
            {
                return Ok(true);
            }
            answer = Some(Projection {
                displacement: feature.displacement,
                denominator: feature.denominator,
            });
            Ok(false)
        };
        if count == 0 {
            visit(&[])?;
        } else {
            combinations(rows.len(), count, visit)?;
        }
        if let Some(answer) = answer {
            return Ok(answer);
        }
    }
    Err(Failure::Arithmetic)
}

pub(super) fn occupancy(
    rows: &[Row],
    center: &[Int; 3],
    radius: Int,
    work: &mut Work,
) -> Result<bool, Failure> {
    if separated(rows, center, None, radius, work)? {
        return Ok(false);
    }
    let closest = closest(rows, center, work)?;
    let bound = radius.mul(&closest.denominator)?;
    Ok(
        dot(&closest.displacement, &closest.displacement)?.compare(&bound.mul(&bound)?)
            != Ordering::Greater,
    )
}

/// Exact supporting-plane rejection avoids fitting features of distant cells.
/// Squared distance proves separation without rounding a plane length/root.
fn separated(
    rows: &[Row],
    start: &[Int; 3],
    delta: Option<&[Int; 3]>,
    radius: Int,
    work: &mut Work,
) -> Result<bool, Failure> {
    let squared_radius = radius.mul(&radius)?;
    for row in rows {
        work.item()?;
        let mut minimum = dot(&row.normal, start)?.add(&row.distance.neg())?;
        if let Some(delta) = delta {
            let slope = dot(&row.normal, delta)?;
            if slope.negative() {
                minimum = minimum.add(&slope)?;
            }
        }
        if !minimum.negative()
            && !minimum.is_zero()
            && minimum
                .mul(&minimum)?
                .compare(&squared_radius.mul(&dot(&row.normal, &row.normal)?)?)
                == Ordering::Greater
        {
            return Ok(true);
        }
    }
    Ok(false)
}

#[derive(Clone, Copy)]
pub(super) struct Polynomial {
    // constant + linear*t + quadratic*t*t, with nonnegative quadratic term.
    coefficients: [Int; 3],
}

impl Polynomial {
    fn new(mut coefficients: [Int; 3]) -> Result<Self, Failure> {
        let shift = coefficients
            .iter()
            .map(Int::trailing_zeros)
            .min()
            .ok_or(Failure::Input)?;
        for value in &mut coefficients {
            *value = value.unshift(shift)?;
        }
        Ok(Self { coefficients })
    }

    fn at(self, t: Fraction) -> Result<Int, Failure> {
        let [constant, linear, quadratic] = self.coefficients;
        constant
            .mul(&t.denominator)?
            .mul(&t.denominator)?
            .add(&linear.mul(&t.numerator)?.mul(&t.denominator)?)?
            .add(&quadratic.mul(&t.numerator)?.mul(&t.numerator)?)
    }

    fn vertex(self) -> Result<Fraction, Failure> {
        Ok(Fraction::new(
            self.coefficients[1].neg(),
            self.coefficients[2].shift(1)?,
        ))
    }

    fn intersect(self, region: Interval) -> Result<Option<SweepInterval>, Failure> {
        if self.coefficients[2].is_zero() {
            return Ok(
                (self.coefficients[0].compare(&Int::ZERO) != Ordering::Greater)
                    .then_some(SweepInterval::rational(region)),
            );
        }
        let vertex = self.vertex()?;
        let minimum = if vertex.compare(region.first)? == Ordering::Less {
            region.first
        } else if vertex.compare(region.last)? == Ordering::Greater {
            region.last
        } else {
            vertex
        };
        match self.at(minimum)?.compare(&Int::ZERO) {
            Ordering::Greater => Ok(None),
            Ordering::Equal => Ok(Some(SweepInterval::rational(Interval {
                first: minimum,
                last: minimum,
                isolated: true,
                first_closed: true,
                last_closed: true,
            }))),
            Ordering::Less => Ok(Some(SweepInterval {
                first: if self.at(region.first)?.compare(&Int::ZERO) == Ordering::Greater {
                    Bound::Root {
                        polynomial: self,
                        upper: false,
                    }
                } else {
                    Bound::Rational(region.first)
                },
                last: if self.at(region.last)?.compare(&Int::ZERO) == Ordering::Greater {
                    Bound::Root {
                        polynomial: self,
                        upper: true,
                    }
                } else {
                    Bound::Rational(region.last)
                },
                isolated: false,
                first_closed: self.at(region.first)?.compare(&Int::ZERO) == Ordering::Greater
                    || region.first_closed,
                last_closed: self.at(region.last)?.compare(&Int::ZERO) == Ordering::Greater
                    || region.last_closed,
            })),
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum Bound {
    Rational(Fraction),
    Root { polynomial: Polynomial, upper: bool },
}

impl Bound {
    pub const ZERO: Self = Self::Rational(Fraction::ZERO);
    pub const ONE: Self = Self::Rational(Fraction::ONE);

    fn compare_rational(self, value: Fraction) -> Result<Ordering, Failure> {
        match self {
            Self::Rational(this) => this.compare(value),
            Self::Root { polynomial, upper } => {
                let vertex = polynomial.vertex()?;
                if upper && value.compare(vertex)? != Ordering::Greater {
                    return Ok(Ordering::Greater);
                }
                if !upper && value.compare(vertex)? != Ordering::Less {
                    return Ok(Ordering::Less);
                }
                let sign = polynomial.at(value)?.compare(&Int::ZERO);
                Ok(if upper { sign.reverse() } else { sign })
            }
        }
    }

    pub fn compare(self, other: Self, work: &mut Work) -> Result<Ordering, Failure> {
        match (self, other) {
            (_, Self::Rational(value)) => return self.compare_rational(value),
            (Self::Rational(value), _) => return Ok(other.compare_rational(value)?.reverse()),
            (
                Self::Root {
                    polynomial: a,
                    upper: ua,
                },
                Self::Root {
                    polynomial: b,
                    upper: ub,
                },
            ) => {
                // Eliminate the quadratic term. Distinct quadratics sharing a
                // root share this rational zero; proportional ones share both.
                let linear = b.coefficients[2]
                    .mul(&a.coefficients[1])?
                    .add(&a.coefficients[2].mul(&b.coefficients[1])?.neg())?;
                let constant = b.coefficients[2]
                    .mul(&a.coefficients[0])?
                    .add(&a.coefficients[2].mul(&b.coefficients[0])?.neg())?;
                if linear.is_zero() && constant.is_zero() {
                    return Ok(ua.cmp(&ub));
                }
                if !linear.is_zero() {
                    let root = Fraction::new(constant.neg(), linear);
                    if self.compare_rational(root)? == Ordering::Equal
                        && other.compare_rational(root)? == Ordering::Equal
                    {
                        return Ok(Ordering::Equal);
                    }
                }
            }
        }
        // Both roots lie within [0,1]. Refine one common dyadic bracket until
        // their ordering is proved. Checked arithmetic/work exhaustion rejects
        // a query; a floating proposal never decides geometric ordering.
        let mut lower = Int::ZERO;
        let mut bits = 0usize;
        loop {
            work.item()?;
            bits = bits.checked_add(1).ok_or(Failure::Arithmetic)?;
            let middle = lower.shift(1)?.add(&Int::ONE)?;
            let fraction = Fraction::new(middle, Int::ONE.shift(bits)?);
            let a = self.compare_rational(fraction)?;
            let b = other.compare_rational(fraction)?;
            if a != b {
                return Ok(a.cmp(&b));
            }
            if a == Ordering::Equal {
                return Ok(Ordering::Equal);
            }
            lower = if a == Ordering::Greater {
                middle
            } else {
                lower.shift(1)?
            };
        }
    }

    pub fn floor(self, work: &mut Work) -> Result<f32, Failure> {
        let mut lower = 0u32;
        let mut upper = 1.0f32.to_bits();
        while lower < upper {
            work.item()?;
            let middle = lower + (upper - lower).div_ceil(2);
            if self.compare_rational(Fraction::binary32(f32::from_bits(middle))?)? != Ordering::Less
            {
                lower = middle;
            } else {
                upper = middle - 1;
            }
        }
        Ok(f32::from_bits(lower))
    }
}

pub(super) type SweepInterval = Interval<Bound>;

impl SweepInterval {
    fn rational(interval: Interval) -> Self {
        Self {
            first: Bound::Rational(interval.first),
            last: Bound::Rational(interval.last),
            isolated: interval.isolated,
            first_closed: interval.first_closed,
            last_closed: interval.last_closed,
        }
    }
}

impl super::Parameter for Bound {
    const ZERO: Self = Self::ZERO;
    const ONE: Self = Self::ONE;
    fn order(self, other: Self, work: &mut Work) -> Result<Ordering, Failure> {
        self.compare(other, work)
    }
}

pub(super) fn interval(
    rows: &[Row],
    start: &[Int; 3],
    delta: &[Int; 3],
    radius: Int,
    work: &mut Work,
) -> Result<Option<SweepInterval>, Failure> {
    interval_mode(rows, start, delta, radius, false, work)
}

pub(super) fn media_interval(
    rows: &[Row],
    start: &[Int; 3],
    delta: &[Int; 3],
    radius: Int,
    work: &mut Work,
) -> Result<Option<SweepInterval>, Failure> {
    interval_mode(rows, start, delta, radius, true, work)
}

fn strict_piece(
    feature: &Feature,
    rows: &[Row],
    indices: &[usize],
    start: &[Int; 3],
    delta: &[Int; 3],
    polynomial: Polynomial,
    mut piece: SweepInterval,
    work: &mut Work,
) -> Result<Option<SweepInterval>, Failure> {
    let region = feature.region(rows, indices, start, Some(delta), true, work)?;
    if polynomial
        .coefficients
        .iter()
        .all(|coefficient| coefficient.is_zero())
    {
        return match region {
            None => Ok(None),
            Some(region) => polynomial.intersect(region),
        };
    }
    let mut included = |bound: Bound| -> Result<bool, Failure> {
        if let Bound::Rational(parameter) = bound
            && polynomial.at(parameter)?.negative()
        {
            return Ok(true);
        }
        let Some(region) = region else {
            return Ok(false);
        };
        let first = bound.compare(Bound::Rational(region.first), work)?;
        let last = bound.compare(Bound::Rational(region.last), work)?;
        Ok(
            (first == Ordering::Greater || first == Ordering::Equal && region.first_closed)
                && (last == Ordering::Less || last == Ordering::Equal && region.last_closed),
        )
    };
    piece.first_closed &= included(piece.first)?;
    piece.last_closed &= included(piece.last)?;
    let equal = piece.first.compare(piece.last, work)? == Ordering::Equal;
    Ok((!equal || piece.first_closed && piece.last_closed).then_some(piece))
}

fn interval_mode(
    rows: &[Row],
    start: &[Int; 3],
    delta: &[Int; 3],
    radius: Int,
    strict: bool,
    work: &mut Work,
) -> Result<Option<SweepInterval>, Failure> {
    if separated(rows, start, Some(delta), radius, work)? {
        return Ok(None);
    }
    let mut answer: Option<SweepInterval> = None;
    for count in 0..=3 {
        let mut visit = |indices: &[usize]| {
            work.item()?;
            let Some(feature) = Feature::fit(rows, indices, start, Some(delta), work)? else {
                return Ok(true);
            };
            let Some(region) = feature.region(rows, indices, start, Some(delta), false, work)?
            else {
                return Ok(true);
            };
            let polynomial = feature.polynomial(radius)?;
            let Some(mut piece) = polynomial.intersect(region)? else {
                return Ok(true);
            };
            if strict {
                let Some(included) = strict_piece(
                    &feature, rows, indices, start, delta, polynomial, piece, work,
                )?
                else {
                    return Ok(true);
                };
                piece = included;
            }
            match &mut answer {
                None => answer = Some(piece),
                Some(old) => {
                    match piece.first.compare(old.first, work)? {
                        Ordering::Less => {
                            old.first = piece.first;
                            old.first_closed = piece.first_closed;
                        }
                        Ordering::Equal => old.first_closed |= piece.first_closed,
                        _ => {}
                    }
                    match piece.last.compare(old.last, work)? {
                        Ordering::Greater => {
                            old.last = piece.last;
                            old.last_closed = piece.last_closed;
                        }
                        Ordering::Equal => old.last_closed |= piece.last_closed,
                        _ => {}
                    }
                    old.isolated = old.first.compare(old.last, work)? == Ordering::Equal;
                }
            }
            // A convex cell's sphere inflation is convex, so its contact set
            // is one interval; feature pieces cannot contain a hidden gap.
            Ok(true)
        };
        if count == 0 {
            visit(&[])?;
        } else {
            combinations(rows.len(), count, visit)?;
        }
        if let Some(interval) = answer
            && interval.first.compare(Bound::ZERO, work)? == Ordering::Equal
            && interval.last.compare(Bound::ONE, work)? == Ordering::Equal
            && interval.first_closed
            && interval.last_closed
        {
            return Ok(answer);
        }
    }
    Ok(answer)
}
