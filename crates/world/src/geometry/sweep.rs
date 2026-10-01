//! Convex separating-axis sweeps retaining BSP cell edges for foreign box
//! sizes. Capsule sweeps use segment-to-polyhedron distance.
//!
//! Donor provenance: `/home/buzzkill/Projects/quake-typescript/src/world/geometry/q1-solid/sweep.ts`.

use crate::WorldError;

use super::polyhedron::{add, box_separating_planes, cross, dot, length, lerp, scale, sub, ConvexCell, DPlane, DVec3};

/// Swept shape against a cell.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CellShape {
    /// Oriented box with axes and half extents.
    Box {
        /// Shape axes.
        axes: [DVec3; 3],
        /// Half extents.
        extents: DVec3,
    },
    /// Capsule with axis, radius, and half segment length.
    Capsule {
        /// Segment axis (unit).
        axis: DVec3,
        /// Radius.
        radius: f64,
        /// Half segment length.
        half_segment: f64,
    },
}

/// Sweep hit interval.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SweepInterval {
    /// Entry fraction.
    pub enter: f64,
    /// Exit fraction.
    pub exit: f64,
    /// Contact plane.
    pub plane: DPlane,
    /// Contact fraction with epsilon backoff.
    pub contact: f64,
}

/// Support distance of a shape along a normal.
#[must_use]
pub fn shape_support(shape: CellShape, normal: DVec3) -> f64 {
    match shape {
        CellShape::Capsule {
            axis,
            radius,
            half_segment,
        } => radius + dot(normal, axis).abs() * half_segment,
        CellShape::Box { axes, extents } => {
            dot(normal, axes[0]).abs() * extents.x
                + dot(normal, axes[1]).abs() * extents.y
                + dot(normal, axes[2]).abs() * extents.z
        }
    }
}

/// Sweep an axis-aligned box cell from `start` to `end`.
#[must_use]
pub fn sweep_box_cell(
    cell: &ConvexCell,
    start: DVec3,
    end: DVec3,
    axes: [DVec3; 3],
    extents: DVec3,
    epsilon: f64,
) -> Option<SweepInterval> {
    let shape = CellShape::Box { axes, extents };
    let mut enter = f64::NEG_INFINITY;
    let mut exit = f64::INFINITY;
    let mut contact = f64::NEG_INFINITY;
    let mut plane = DPlane {
        normal: DVec3 { x: 0.0, y: 0.0, z: 0.0 },
        distance: 0.0,
    };
    for candidate in box_separating_planes(cell, &axes) {
        let distance = candidate.distance + shape_support(shape, candidate.normal);
        let a = dot(start, candidate.normal) - distance;
        let b = dot(end, candidate.normal) - distance;
        if a > 0.0 && b > 0.0 {
            return None;
        }
        if a <= 0.0 && b <= 0.0 {
            continue;
        }
        let fraction = a / (a - b);
        if a > b {
            if fraction > enter {
                enter = fraction;
                plane = candidate;
            }
            contact = contact.max((a - epsilon) / (a - b));
        } else {
            exit = exit.min(fraction);
        }
        if enter > exit {
            return None;
        }
    }
    if enter <= 1.0 && exit >= 0.0 {
        Some(SweepInterval {
            enter,
            exit,
            plane,
            contact,
        })
    } else {
        None
    }
}

#[derive(Debug, Clone, Copy)]
struct Closest {
    a: DVec3,
    b: DVec3,
    squared: f64,
}

fn pair(a: DVec3, b: DVec3) -> Closest {
    let d = sub(a, b);
    Closest {
        a,
        b,
        squared: dot(d, d),
    }
}

fn nearest(first: Closest, second: Closest) -> Closest {
    if first.squared <= second.squared {
        first
    } else {
        second
    }
}

#[allow(clippy::too_many_lines)]
fn point_triangle(p: DVec3, a: DVec3, b: DVec3, c: DVec3) -> Closest {
    let ab = sub(b, a);
    let ac = sub(c, a);
    let ap = sub(p, a);
    let d1 = dot(ab, ap);
    let d2 = dot(ac, ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return pair(p, a);
    }
    let bp = sub(p, b);
    let d3 = dot(ab, bp);
    let d4 = dot(ac, bp);
    if d3 >= 0.0 && d4 <= d3 {
        return pair(p, b);
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return pair(p, add(a, scale(ab, d1 / (d1 - d3))));
    }
    let cp = sub(p, c);
    let d5 = dot(ab, cp);
    let d6 = dot(ac, cp);
    if d6 >= 0.0 && d5 <= d6 {
        return pair(p, c);
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return pair(p, add(a, scale(ac, d2 / (d2 - d6))));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && d4 - d3 >= 0.0 && d5 - d6 >= 0.0 {
        return pair(p, lerp(b, c, (d4 - d3) / (d4 - d3 + d5 - d6)));
    }
    let sum = va + vb + vc;
    if sum.abs() < 1e-25 {
        return nearest(nearest(pair(p, a), pair(p, b)), pair(p, c));
    }
    pair(p, add(a, add(scale(ab, vb / sum), scale(ac, vc / sum))))
}

fn clamp01(f: f64) -> f64 {
    f.clamp(0.0, 1.0)
}

fn segments(p: DVec3, q: DVec3, a: DVec3, b: DVec3) -> Closest {
    let u = sub(q, p);
    let v = sub(b, a);
    let w = sub(p, a);
    let uu = dot(u, u);
    let uv = dot(u, v);
    let vv = dot(v, v);
    let uw = dot(u, w);
    let vw = dot(v, w);
    let denom = uu * vv - uv * uv;
    let mut s = if uu == 0.0 {
        0.0
    } else {
        clamp01((uv * vw - vv * uw) / (if denom == 0.0 { 1.0 } else { denom }))
    };
    let mut t = if vv == 0.0 { 0.0 } else { (uv * s + vw) / vv };
    if t < 0.0 {
        t = 0.0;
        s = if uu == 0.0 { 0.0 } else { clamp01(-uw / uu) };
    } else if t > 1.0 {
        t = 1.0;
        s = if uu == 0.0 { 0.0 } else { clamp01((uv - uw) / uu) };
    }
    pair(lerp(p, q, s), lerp(a, b, t))
}

fn segment_cell(cell: &ConvexCell, p: DVec3, q: DVec3) -> Closest {
    let mut enter = 0.0_f64;
    let mut exit = 1.0_f64;
    for face in &cell.faces {
        let plane = face.plane;
        let a = dot(p, plane.normal) - plane.distance;
        let b = dot(q, plane.normal) - plane.distance;
        if a > 0.0 && b > 0.0 {
            enter = f64::INFINITY;
            break;
        }
        if a > b && a > 0.0 {
            enter = enter.max(a / (a - b));
        }
        if a < b && b > 0.0 {
            exit = exit.min(a / (a - b));
        }
    }
    if enter <= exit {
        let point = lerp(p, q, enter);
        return pair(point, point);
    }
    let mut result = Closest {
        a: p,
        b: p,
        squared: f64::INFINITY,
    };
    for face in &cell.faces {
        let Some(a) = face.vertices.first().copied() else {
            continue;
        };
        for i in 1..face.vertices.len().saturating_sub(1) {
            let (Some(b), Some(c)) = (face.vertices.get(i).copied(), face.vertices.get(i + 1).copied()) else {
                continue;
            };
            if length(cross(sub(b, a), sub(c, a))) < 1e-12 {
                continue;
            }
            result = nearest(result, point_triangle(p, a, b, c));
            result = nearest(result, point_triangle(q, a, b, c));
            result = nearest(result, segments(p, q, a, b));
            result = nearest(result, segments(p, q, b, c));
            result = nearest(result, segments(p, q, c, a));
        }
    }
    result
}

struct CapsuleEntrance {
    fraction: f64,
    plane: DPlane,
}

fn capsule_entrance(
    cell: &ConvexCell,
    start: DVec3,
    end: DVec3,
    axis: DVec3,
    radius: f64,
    half_segment: f64,
    margin: f64,
) -> Result<Option<CapsuleEntrance>, WorldError> {
    let offset = scale(axis, half_segment);
    let movement = sub(end, start);
    let mut fraction = 0.0;
    for _ in 0..96 {
        let center = lerp(start, end, fraction);
        let closest = segment_cell(cell, sub(center, offset), add(center, offset));
        let distance = closest.squared.sqrt();
        let separation = distance - radius - margin;
        let normal = if distance > 1e-12 {
            scale(sub(closest.a, closest.b), 1.0 / distance)
        } else {
            DVec3 { x: 0.0, y: 0.0, z: 0.0 }
        };
        if separation <= 1e-7 {
            return Ok(Some(CapsuleEntrance {
                fraction,
                plane: DPlane {
                    normal,
                    distance: dot(normal, closest.b),
                },
            }));
        }
        let closing = -dot(movement, normal);
        if closing <= 0.0 {
            return Ok(None);
        }
        let step = separation / closing;
        if fraction + step > 1.0 {
            return Ok(None);
        }
        fraction += step;
    }
    Err(WorldError::SweepDiverged)
}

/// Sweep a capsule cell from `start` to `end`.
pub fn sweep_capsule_cell(
    cell: &ConvexCell,
    start: DVec3,
    end: DVec3,
    axis: DVec3,
    radius: f64,
    half_segment: f64,
    epsilon: f64,
) -> Result<Option<SweepInterval>, WorldError> {
    let entrance = capsule_entrance(cell, start, end, axis, radius, half_segment, 0.0)?;
    let Some(entrance) = entrance else {
        return Ok(None);
    };
    let reverse = capsule_entrance(cell, end, start, axis, radius, half_segment, 0.0)?;
    let contact = capsule_entrance(cell, start, end, axis, radius, half_segment, epsilon)?;
    Ok(Some(SweepInterval {
        enter: entrance.fraction,
        exit: reverse.map_or(entrance.fraction, |reverse| 1.0 - reverse.fraction),
        plane: entrance.plane,
        contact: contact.map_or(entrance.fraction, |contact| contact.fraction),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::polyhedron::{box_cell, dvec3, DBounds, AXES};

    fn unit_cell() -> ConvexCell {
        box_cell(&DBounds {
            min: dvec3(-8.0, -8.0, -8.0),
            max: dvec3(8.0, 8.0, 8.0),
        })
    }

    #[test]
    fn support_matches_box_extents() {
        let shape = CellShape::Box {
            axes: AXES,
            extents: dvec3(1.0, 2.0, 4.0),
        };
        assert!((shape_support(shape, AXES[0]) - 1.0).abs() < 1e-12);
        assert!((shape_support(shape, AXES[2]) - 4.0).abs() < 1e-12);
        let capsule = CellShape::Capsule {
            axis: AXES[2],
            radius: 2.0,
            half_segment: 3.0,
        };
        assert!((shape_support(capsule, AXES[2]) - 5.0).abs() < 1e-12);
        assert!((shape_support(capsule, AXES[0]) - 2.0).abs() < 1e-12);
    }

    #[test]
    fn box_sweep_hits_and_misses() {
        let cell = unit_cell();
        let hit = sweep_box_cell(
            &cell,
            dvec3(-32.0, 0.0, 0.0),
            dvec3(32.0, 0.0, 0.0),
            AXES,
            dvec3(1.0, 1.0, 1.0),
            0.125,
        )
        .expect("hits");
        assert!(hit.enter >= 0.0 && hit.enter <= 1.0);
        assert!(hit.exit >= hit.enter);
        let miss = sweep_box_cell(
            &cell,
            dvec3(-32.0, 64.0, 0.0),
            dvec3(32.0, 64.0, 0.0),
            AXES,
            dvec3(1.0, 1.0, 1.0),
            0.125,
        );
        assert!(miss.is_none());
    }

    #[test]
    fn capsule_sweep_finds_entrance() {
        let cell = unit_cell();
        let hit = sweep_capsule_cell(
            &cell,
            dvec3(-32.0, 0.0, 0.0),
            dvec3(32.0, 0.0, 0.0),
            AXES[2],
            2.0,
            4.0,
            0.125,
        )
        .unwrap()
        .expect("hits");
        assert!(hit.enter >= 0.0 && hit.enter <= 1.0);
        assert!(hit.exit >= hit.enter);
        let miss = sweep_capsule_cell(
            &cell,
            dvec3(-32.0, 64.0, 0.0),
            dvec3(32.0, 64.0, 0.0),
            AXES[2],
            2.0,
            4.0,
            0.125,
        )
        .unwrap();
        assert!(miss.is_none());
    }
}
