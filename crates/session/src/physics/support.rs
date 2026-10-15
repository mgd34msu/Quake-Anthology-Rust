use qa_core::primitives::{Bounds, Vec3};
use qa_world::collision::Trace;

/// The caller selects contents semantics, point filtering, pass identity and
/// trace rules. Support checks never move or link the queried entity.
pub trait SupportQueries {
    type Error;
    fn solid(&mut self, point: Vec3) -> Result<bool, Self::Error>;
    fn trace(&mut self, start: Vec3, end: Vec3) -> Result<Trace, Self::Error>;
}

pub fn check_bottom<Q: SupportQueries>(
    queries: &mut Q,
    origin: Vec3,
    bounds: Bounds,
    step_height: f32,
) -> Result<bool, Q::Error> {
    let mins = origin + bounds.mins;
    let maxs = origin + bounds.maxs;
    let mut easy = true;
    'corners: for x in [mins.0[0], maxs.0[0]] {
        for y in [mins.0[1], maxs.0[1]] {
            if !queries.solid(Vec3([x, y, mins.0[2] - 1.0]))? {
                easy = false;
                break 'corners;
            }
        }
    }
    if easy {
        return Ok(true);
    }
    let midpoint = Vec3([
        (mins.0[0] + maxs.0[0]) * 0.5,
        (mins.0[1] + maxs.0[1]) * 0.5,
        mins.0[2],
    ]);
    let end_z = mins.0[2] - 2.0 * step_height;
    let middle = queries.trace(midpoint, Vec3([midpoint.0[0], midpoint.0[1], end_z]))?;
    if middle.fraction == 1.0 {
        return Ok(false);
    }
    for x in [mins.0[0], maxs.0[0]] {
        for y in [mins.0[1], maxs.0[1]] {
            let corner = queries.trace(Vec3([x, y, mins.0[2]]), Vec3([x, y, end_z]))?;
            if corner.fraction == 1.0 || middle.end.0[2] - corner.end.0[2] > step_height {
                return Ok(false);
            }
        }
    }
    Ok(true)
}
