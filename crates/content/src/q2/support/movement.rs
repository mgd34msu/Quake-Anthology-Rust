//! Rerelease unstuck placement (`src/movement/q2/rerelease.ts`, `types.ts`).
//!
//! Only the surface `spawn-placement.ts` uses is modeled: the
//! `G_FixStuckObject_Generic` search plus the stuck-result type. The full
//! player-movement machine stays with its owning lane.

use std::cmp::Ordering;

use qa_core::numeric::NumericOps;

/// Movement-local vector: three donor numbers (`movement/q2/types.ts`).
pub type MovementVector = [f64; 3];

/// Unstuck search outcome (`StuckResultT`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StuckResult {
    /// The origin is already free.
    GoodPosition,
    /// The origin was moved to a free position.
    Fixed,
    /// No free position was found.
    NoGoodPosition,
}

/// Trace record consumed by the unstuck search.
///
/// `G_FixStuckObject_Generic` only reads solidity and the trace end; the
/// remaining `TraceT` fields never reach this search.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StuckTrace {
    /// Trace started inside solid.
    pub start_solid: bool,
    /// Trace end position.
    pub endpos: MovementVector,
}

/// Side checks from `G_FixStuckObject_Generic`.
const SIDE_CHECKS: [([f64; 3], [f64; 3], [f64; 3]); 6] = [
    ([0.0, 0.0, 1.0], [-1.0, -1.0, 0.0], [1.0, 1.0, 0.0]),
    ([0.0, 0.0, -1.0], [-1.0, -1.0, 0.0], [1.0, 1.0, 0.0]),
    ([1.0, 0.0, 0.0], [0.0, -1.0, -1.0], [0.0, 1.0, 1.0]),
    ([-1.0, 0.0, 0.0], [0.0, -1.0, -1.0], [0.0, 1.0, 1.0]),
    ([0.0, 1.0, 0.0], [-1.0, 0.0, -1.0], [1.0, 0.0, 1.0]),
    ([0.0, -1.0, 0.0], [-1.0, 0.0, -1.0], [1.0, 0.0, 1.0]),
];

fn store(ops: &NumericOps, value: f64) -> f64 {
    f64::from(ops.store(value))
}

fn dot(ops: &NumericOps, a: &MovementVector, b: &MovementVector) -> f64 {
    ops.add(
        ops.add(ops.mul(a[0], b[0]), ops.mul(a[1], b[1])),
        ops.mul(a[2], b[2]),
    )
}

/// Free a stuck monster origin (`G_FixStuckObject_Generic`).
///
/// On [`StuckResult::Fixed`] the origin is overwritten with the closest
/// free position. Like the donor, only the candidates before the last one
/// are distance-sorted.
pub fn fix_stuck_object(
    ops: &NumericOps,
    origin: &mut MovementVector,
    own_mins: &MovementVector,
    own_maxs: &MovementVector,
    trace: &mut dyn FnMut(&MovementVector, &MovementVector, &MovementVector, &MovementVector) -> StuckTrace,
) -> StuckResult {
    if !trace(origin, own_mins, own_maxs, origin).start_solid {
        return StuckResult::GoodPosition;
    }
    let mut good_positions: Vec<(f64, MovementVector)> = Vec::new();
    for (sn, (normal, side_mins, side_maxs)) in SIDE_CHECKS.iter().enumerate() {
        let mut start = *origin;
        let mut mins = [0.0, 0.0, 0.0];
        let mut maxs = [0.0, 0.0, 0.0];
        for n in 0..3 {
            if normal[n] < 0.0 {
                start[n] = store(ops, ops.add(start[n], own_mins[n]));
            } else if normal[n] > 0.0 {
                start[n] = store(ops, ops.add(start[n], own_maxs[n]));
            }
            if side_mins[n] == -1.0 {
                mins[n] = store(ops, own_mins[n]);
            } else if side_mins[n] == 1.0 {
                mins[n] = store(ops, own_maxs[n]);
            }
            if side_maxs[n] == -1.0 {
                maxs[n] = store(ops, own_mins[n]);
            } else if side_maxs[n] == 1.0 {
                maxs[n] = store(ops, own_maxs[n]);
            }
        }
        let mut hit = trace(&start, &mins, &maxs, &start);
        let mut needed_epsilon_fix: i32 = -1;
        let mut needed_epsilon_dir = 0.0;
        if hit.start_solid {
            for e in 0..3 {
                if normal[e] != 0.0 {
                    continue;
                }
                let mut ep_start = start;
                ep_start[e] = store(ops, ops.add(ep_start[e], 1.0));
                hit = trace(&ep_start, &mins, &maxs, &ep_start);
                if !hit.start_solid {
                    start = ep_start;
                    needed_epsilon_fix = e as i32;
                    needed_epsilon_dir = 1.0;
                    break;
                }
                ep_start[e] = store(ops, ops.sub(ep_start[e], 2.0));
                hit = trace(&ep_start, &mins, &maxs, &ep_start);
                if !hit.start_solid {
                    start = ep_start;
                    needed_epsilon_fix = e as i32;
                    needed_epsilon_dir = -1.0;
                    break;
                }
            }
        }
        if hit.start_solid {
            continue;
        }
        let mut opposite_start = *origin;
        let (other_normal, _, _) = SIDE_CHECKS[sn ^ 1];
        for n in 0..3 {
            if other_normal[n] < 0.0 {
                opposite_start[n] = store(ops, ops.add(opposite_start[n], own_mins[n]));
            } else if other_normal[n] > 0.0 {
                opposite_start[n] = store(ops, ops.add(opposite_start[n], own_maxs[n]));
            }
        }
        if needed_epsilon_fix >= 0 {
            let e = needed_epsilon_fix as usize;
            opposite_start[e] = store(ops, ops.add(opposite_start[e], needed_epsilon_dir));
        }
        hit = trace(&start, &mins, &maxs, &opposite_start);
        if hit.start_solid {
            continue;
        }
        let mut end = hit.endpos;
        let epsilon = store(ops, 0.125);
        for n in 0..3 {
            end[n] = store(ops, ops.add(end[n], ops.mul(normal[n], epsilon)));
        }
        let mut delta = [0.0, 0.0, 0.0];
        for n in 0..3 {
            delta[n] = store(ops, ops.sub(end[n], opposite_start[n]));
        }
        let mut new_origin = [0.0, 0.0, 0.0];
        for n in 0..3 {
            new_origin[n] = store(ops, ops.add(origin[n], delta[n]));
        }
        if needed_epsilon_fix >= 0 {
            let e = needed_epsilon_fix as usize;
            new_origin[e] = store(ops, ops.add(new_origin[e], needed_epsilon_dir));
        }
        hit = trace(&new_origin, own_mins, own_maxs, &new_origin);
        if hit.start_solid {
            continue;
        }
        good_positions.push((dot(ops, &delta, &delta), new_origin));
    }
    if good_positions.is_empty() {
        return StuckResult::NoGoodPosition;
    }
    if good_positions.len() > 1 {
        let sortable = good_positions.len() - 1;
        good_positions[..sortable]
            .sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(Ordering::Equal));
    }
    *origin = good_positions[0].1;
    StuckResult::Fixed
}

#[cfg(test)]
mod tests {
    use super::*;

    use qa_core::numeric::Q3_BINARY32_PROFILE;

    fn ops() -> NumericOps {
        NumericOps::select(Q3_BINARY32_PROFILE).expect("profile")
    }

    #[test]
    fn free_origin_reports_good_position() {
        let ops = ops();
        let mut origin = [0.0, 0.0, 0.0];
        let mins = [-16.0, -16.0, -24.0];
        let maxs = [16.0, 16.0, 32.0];
        let result = fix_stuck_object(&ops, &mut origin, &mins, &maxs, &mut |_, _, _, _| StuckTrace {
            start_solid: false,
            endpos: [0.0, 0.0, 0.0],
        });
        assert_eq!(result, StuckResult::GoodPosition);
    }

    #[test]
    fn solid_everywhere_reports_no_good_position() {
        let ops = ops();
        let mut origin = [0.0, 0.0, 0.0];
        let mins = [-16.0, -16.0, -24.0];
        let maxs = [16.0, 16.0, 32.0];
        let result = fix_stuck_object(&ops, &mut origin, &mins, &maxs, &mut |_, _, _, _| StuckTrace {
            start_solid: true,
            endpos: [0.0, 0.0, 0.0],
        });
        assert_eq!(result, StuckResult::NoGoodPosition);
    }
}
