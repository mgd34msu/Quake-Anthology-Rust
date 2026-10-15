use super::position::{Failure, Interval, Parameter, Work};
use super::{Contents, Trace};
use std::cmp::Ordering;

impl Parameter for f32 {
    const ZERO: Self = 0.0;
    const ONE: Self = 1.0;
    fn order(self, other: Self, work: &mut Work) -> Result<Ordering, Failure> {
        work.item()?;
        self.partial_cmp(&other).ok_or(Failure::Input)
    }
}

pub(super) type Frontier<T> = Option<(T, bool)>;

pub(super) fn extend<T: Parameter>(
    span: Interval<T>,
    reach: &mut Frontier<T>,
    interval: Interval<T>,
    work: &mut Work,
) -> Result<(), Failure> {
    let (frontier, closed) = reach.unwrap_or((span.first, !span.first_closed));
    let first = interval.first.order(frontier, work)?;
    if first == Ordering::Greater {
        return Ok(());
    }
    let last = interval.last.order(frontier, work)?;
    if last != Ordering::Less && (first != Ordering::Equal || closed || interval.first_closed) {
        *reach = Some(if last == Ordering::Greater {
            (interval.last, interval.last_closed)
        } else {
            (frontier, closed || interval.last_closed)
        });
    }
    Ok(())
}

pub(super) fn covers<T: Parameter>(
    reach: Frontier<T>,
    span: Interval<T>,
    work: &mut Work,
) -> Result<bool, Failure> {
    match reach {
        None => Ok(false),
        Some((last, closed)) => Ok(match last.order(span.last, work)? {
            Ordering::Greater => true,
            Ordering::Equal => closed || !span.last_closed,
            Ordering::Less => false,
        }),
    }
}

pub(super) fn union_reach<T: Parameter>(
    span: Interval<T>,
    count: u32,
    mut reach: Frontier<T>,
    work: &mut Work,
    intervals: &mut impl FnMut(
        &mut Work,
        &mut dyn FnMut(Interval<T>, Contents, &mut Work) -> Result<(), Failure>,
    ) -> Result<(), Failure>,
) -> Result<Frontier<T>, Failure> {
    for _ in 0..=count {
        if covers(reach, span, work)? {
            return Ok(reach);
        }
        let old = reach;
        intervals(work, &mut |interval, _, work| {
            extend(span, &mut reach, interval, work)
        })?;
        let unchanged = match (old, reach) {
            (None, None) => true,
            (Some((old, a)), Some((new, b))) => a == b && old.order(new, work)? == Ordering::Equal,
            _ => false,
        };
        if unchanged {
            return Ok(reach);
        }
    }
    Err(Failure::Budget)
}

fn clip<T: Parameter>(
    mut interval: Interval<T>,
    upper: T,
    work: &mut Work,
) -> Result<Option<Interval<T>>, Failure> {
    if interval.first.order(T::ZERO, work)? == Ordering::Less {
        interval.first = T::ZERO;
        interval.first_closed = true;
    }
    if interval.last.order(upper, work)? == Ordering::Greater {
        interval.last = upper;
        interval.last_closed = true;
    }
    let order = interval.first.order(interval.last, work)?;
    Ok((order == Ordering::Less
        || order == Ordering::Equal && interval.first_closed && interval.last_closed)
        .then_some(interval))
}

pub(super) struct TraceMedia {
    in_open: bool,
    in_water: bool,
}
impl TraceMedia {
    pub(super) fn apply(self, trace: &mut Trace) {
        trace.in_open = self.in_open;
        trace.in_water = self.in_water;
    }
}

pub(super) fn reached<T: Parameter>(
    upper: T,
    count: u32,
    work: &mut Work,
    mut intervals: impl FnMut(
        &mut Work,
        &mut dyn FnMut(Interval<T>, Contents, &mut Work) -> Result<(), Failure>,
    ) -> Result<(), Failure>,
) -> Result<TraceMedia, Failure> {
    if upper.order(T::ZERO, work)? == Ordering::Less
        || upper.order(T::ONE, work)? == Ordering::Greater
    {
        return Err(Failure::Input);
    }
    let span = Interval {
        first: T::ZERO,
        last: upper,
        isolated: false,
        first_closed: true,
        last_closed: true,
    };
    let reach = union_reach(span, count, None, work, &mut |work, visit| {
        intervals(work, &mut |interval, contents, work| {
            if contents.to_q1() != -1
                && let Some(interval) = clip(interval, upper, work)?
            {
                visit(interval, contents, work)?;
            }
            Ok(())
        })
    })?;
    let in_open = !covers(reach, span, work)?;
    for candidate in 0..count {
        let (mut next, mut selected) = (0, None);
        intervals(work, &mut |interval, contents, work| {
            let native = contents.to_q1();
            if native != -1
                && native != -2
                && let Some(interval) = clip(interval, upper, work)?
            {
                if next == candidate {
                    selected = Some(interval);
                }
                next += 1;
            }
            Ok(())
        })?;
        let Some(span) = selected else {
            break;
        };
        let reach = union_reach(span, count, None, work, &mut |work, visit| {
            intervals(work, &mut |interval, contents, work| {
                if contents.to_q1() == -2 {
                    visit(interval, contents, work)?;
                }
                Ok(())
            })
        })?;
        if !covers(reach, span, work)? {
            return Ok(TraceMedia {
                in_open,
                in_water: true,
            });
        }
    }
    Ok(TraceMedia {
        in_open,
        in_water: false,
    })
}
