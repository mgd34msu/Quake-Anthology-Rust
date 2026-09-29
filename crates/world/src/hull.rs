//! Quake I hull traversal ported from `src/world/collision/q1/hull.ts`
//! (WinQuake `world.c`). Recursive clip-walk with the `1/32` epsilon and
//! the `0.1` backoff loop.

use qa_core::math::{vec3, Bounds, Plane, Vec3};
use qa_core::numeric::NumericOps;

use crate::collision::q1;
use crate::WorldError;

/// Hull plane with its axial type and sign bits.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BspPlane {
    /// Plane normal.
    pub normal: Vec3,
    /// Plane distance.
    pub distance: f32,
    /// Axial type (`0/1/2` axial, `3` general).
    pub plane_type: u8,
    /// Sign bits.
    pub signbits: u8,
}

/// Clip-tree child: another node or leaf contents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipChild {
    /// Clip node index.
    Node(usize),
    /// Leaf contents.
    Contents(i32),
}

/// One clip node.
#[derive(Debug, Clone, PartialEq)]
pub struct ClipNode {
    /// Plane index.
    pub plane: usize,
    /// Front and back children.
    pub children: [ClipChild; 2],
}

/// Collision hull: planes plus a clip tree.
#[derive(Debug, Clone, PartialEq)]
pub struct Hull {
    /// Planes.
    pub planes: Vec<BspPlane>,
    /// Clip nodes.
    pub clipnodes: Vec<ClipNode>,
    /// First clip node (or root contents when negative).
    pub first: i32,
    /// Last clip node.
    pub last: i32,
}

/// Hull trace result.
#[derive(Debug, Clone, PartialEq)]
pub struct HullTrace {
    /// Reached fraction.
    pub fraction: f64,
    /// Reached endpoint.
    pub end: Vec3,
    /// Started inside solid.
    pub start_solid: bool,
    /// Entirely inside solid.
    pub all_solid: bool,
    /// Reached open space.
    pub in_open: bool,
    /// Reached liquid.
    pub in_water: bool,
    /// Impact plane.
    pub plane: Plane,
    /// Impact contents.
    pub contents: i32,
}

/// Six-plane axis box hull, matching the actor-body helper in the donor.
#[must_use]
pub fn axis_box_hull(bounds: &Bounds) -> Hull {
    let zero = vec3(0.0, 0.0, 0.0);
    let planes = vec![
        (vec3(1.0, 0.0, 0.0), bounds.max.x, 0),
        (vec3(1.0, 0.0, 0.0), bounds.min.x, 0),
        (vec3(0.0, 1.0, 0.0), bounds.max.y, 1),
        (vec3(0.0, 1.0, 0.0), bounds.min.y, 1),
        (vec3(0.0, 0.0, 1.0), bounds.max.z, 2),
        (vec3(0.0, 0.0, 1.0), bounds.min.z, 2),
    ]
    .into_iter()
    .map(|(normal, distance, plane_type)| BspPlane {
        normal,
        distance,
        plane_type,
        signbits: 0,
    })
    .collect::<Vec<_>>();
    let _ = zero;
    let clipnodes = (0..6)
        .map(|index| {
            let empty = ClipChild::Contents(q1::CONTENTS_EMPTY);
            let next = if index == 5 {
                ClipChild::Contents(q1::CONTENTS_SOLID)
            } else {
                ClipChild::Node(index + 1)
            };
            let children = if index % 2 == 0 { [empty, next] } else { [next, empty] };
            ClipNode { plane: index, children }
        })
        .collect::<Vec<_>>();
    Hull {
        planes,
        clipnodes,
        first: 0,
        last: 5,
    }
}

fn child_number(child: ClipChild) -> i32 {
    match child {
        ClipChild::Node(index) => index as i32,
        ClipChild::Contents(value) => value,
    }
}

fn node_at(hull: &Hull, node: i32) -> Result<&ClipNode, WorldError> {
    if node < hull.first || node > hull.last {
        return Err(WorldError::Hull(format!("Invalid Quake hull node {node}")));
    }
    hull.clipnodes
        .get(node as usize)
        .ok_or_else(|| WorldError::Hull(format!("Missing Quake hull node {node}")))
}

fn plane_at(hull: &Hull, plane: usize) -> Result<&BspPlane, WorldError> {
    hull.planes
        .get(plane)
        .ok_or_else(|| WorldError::Hull("Invalid Quake hull plane".to_string()))
}

fn plane_distance(hull: &Hull, plane: usize, point: Vec3, ops: &NumericOps) -> Result<f64, WorldError> {
    let plane = plane_at(hull, plane)?;
    let distance = f64::from(plane.distance);
    match plane.plane_type {
        0 => Ok(ops.sub(f64::from(point.x), distance)),
        1 => Ok(ops.sub(f64::from(point.y), distance)),
        2 => Ok(ops.sub(f64::from(point.z), distance)),
        _ => {
            let normal = plane.normal;
            let dot = ops.add(
                ops.add(
                    ops.mul(f64::from(point.x), f64::from(normal.x)),
                    ops.mul(f64::from(point.y), f64::from(normal.y)),
                ),
                ops.mul(f64::from(point.z), f64::from(normal.z)),
            );
            Ok(ops.sub(dot, distance))
        }
    }
}

fn contents_at(hull: &Hull, point: Vec3, mut node: i32, ops: &NumericOps) -> Result<i32, WorldError> {
    let mut visits = 0;
    while node >= 0 {
        visits += 1;
        if visits > hull.clipnodes.len() {
            return Err(WorldError::HullCycle);
        }
        let clip = node_at(hull, node)?;
        let distance = plane_distance(hull, clip.plane, point, ops)?;
        let child = clip.children[usize::from(distance < 0.0)];
        node = child_number(child);
    }
    Ok(node)
}

/// Leaf contents at a point.
pub fn hull_point_contents(hull: &Hull, point: Vec3, ops: &NumericOps) -> Result<i32, WorldError> {
    contents_at(hull, point, hull.first, ops)
}

fn interpolate(start: Vec3, end: Vec3, fraction: f64, ops: &NumericOps) -> Vec3 {
    vec3(
        ops.store(ops.add(
            f64::from(start.x),
            ops.mul(fraction, ops.sub(f64::from(end.x), f64::from(start.x))),
        )),
        ops.store(ops.add(
            f64::from(start.y),
            ops.mul(fraction, ops.sub(f64::from(end.y), f64::from(start.y))),
        )),
        ops.store(ops.add(
            f64::from(start.z),
            ops.mul(fraction, ops.sub(f64::from(end.z), f64::from(start.z))),
        )),
    )
}

struct Walker<'a> {
    hull: &'a Hull,
    ops: &'a NumericOps,
    blocks: &'a dyn Fn(i32) -> bool,
    trace: HullTrace,
}

impl Walker<'_> {
    fn walk(&mut self, node: i32, p1f: f64, p2f: f64, p1: Vec3, p2: Vec3, depth: usize) -> Result<bool, WorldError> {
        if depth > self.hull.clipnodes.len() {
            return Err(WorldError::HullCycle);
        }
        if node < 0 {
            if (self.blocks)(node) {
                self.trace.start_solid = true;
                self.trace.contents = node;
            } else {
                self.trace.all_solid = false;
                if node == q1::CONTENTS_EMPTY {
                    self.trace.in_open = true;
                } else {
                    self.trace.in_water = true;
                }
            }
            return Ok(true);
        }
        let clip = node_at(self.hull, node)?.clone();
        let t1 = plane_distance(self.hull, clip.plane, p1, self.ops)?;
        let t2 = plane_distance(self.hull, clip.plane, p2, self.ops)?;
        if t1 >= 0.0 && t2 >= 0.0 {
            return self.walk(child_number(clip.children[0]), p1f, p2f, p1, p2, depth + 1);
        }
        if t1 < 0.0 && t2 < 0.0 {
            return self.walk(child_number(clip.children[1]), p1f, p2f, p1, p2, depth + 1);
        }
        let offset = if t1 < 0.0 {
            q1::DISTANCE_EPSILON
        } else {
            -q1::DISTANCE_EPSILON
        };
        let mut fraction = self
            .ops
            .div(self.ops.add(t1, offset), self.ops.sub(t1, t2))
            .clamp(0.0, 1.0);
        let mut midf = self.ops.add(p1f, self.ops.mul(self.ops.sub(p2f, p1f), fraction));
        let mut mid = interpolate(p1, p2, fraction, self.ops);
        let near = clip.children[usize::from(t1 < 0.0)];
        let far = clip.children[usize::from(t1 >= 0.0)];
        if !self.walk(child_number(near), p1f, midf, p1, mid, depth + 1)? {
            return Ok(false);
        }
        let far_contents = contents_at(self.hull, mid, child_number(far), self.ops)?;
        if !(self.blocks)(far_contents) {
            return self.walk(child_number(far), midf, p2f, mid, p2, depth + 1);
        }
        if self.trace.all_solid {
            return Ok(false);
        }
        let plane = *plane_at(self.hull, clip.plane)?;
        self.trace.plane = if t1 < 0.0 {
            Plane {
                normal: vec3(-plane.normal.x, -plane.normal.y, -plane.normal.z),
                distance: -plane.distance,
            }
        } else {
            Plane {
                normal: plane.normal,
                distance: plane.distance,
            }
        };
        self.trace.contents = far_contents;
        while (self.blocks)(contents_at(self.hull, mid, self.hull.first, self.ops)?) {
            fraction = self.ops.sub(fraction, 0.1);
            if fraction < 0.0 {
                self.trace.fraction = midf;
                self.trace.end = mid;
                return Ok(false);
            }
            midf = self.ops.add(p1f, self.ops.mul(self.ops.sub(p2f, p1f), fraction));
            mid = interpolate(p1, p2, fraction, self.ops);
        }
        self.trace.fraction = midf;
        self.trace.end = mid;
        self.trace.contents = far_contents;
        Ok(false)
    }
}

/// Trace a hull from `start` to `end`.
pub fn trace_hull(
    hull: &Hull,
    start: Vec3,
    end: Vec3,
    ops: &NumericOps,
    blocks: &dyn Fn(i32) -> bool,
) -> Result<HullTrace, WorldError> {
    let mut walker = Walker {
        hull,
        ops,
        blocks,
        trace: HullTrace {
            fraction: 1.0,
            end,
            start_solid: false,
            all_solid: true,
            in_open: false,
            in_water: false,
            plane: Plane {
                normal: vec3(0.0, 0.0, 0.0),
                distance: 0.0,
            },
            contents: q1::CONTENTS_EMPTY,
        },
    };
    walker.walk(hull.first, 0.0, 1.0, start, end, 0)?;
    Ok(walker.trace)
}

/// Trace with the default solid-blocking rule.
pub fn trace_hull_solid(hull: &Hull, start: Vec3, end: Vec3, ops: &NumericOps) -> Result<HullTrace, WorldError> {
    trace_hull(hull, start, end, ops, &|contents| contents == q1::CONTENTS_SOLID)
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::numeric::Q1_DONOR_PROFILE;

    fn ops() -> NumericOps {
        NumericOps::select(Q1_DONOR_PROFILE).unwrap()
    }

    fn unit_box() -> Hull {
        axis_box_hull(&Bounds {
            min: vec3(-16.0, -16.0, -16.0),
            max: vec3(16.0, 16.0, 16.0),
        })
    }

    #[test]
    fn open_trace_reaches_the_end() {
        let hull = unit_box();
        let trace = trace_hull_solid(&hull, vec3(64.0, 0.0, 0.0), vec3(128.0, 0.0, 0.0), &ops()).unwrap();
        assert_eq!(trace.fraction, 1.0);
        assert!(!trace.start_solid);
        assert!(!trace.all_solid);
        assert!(trace.in_open);
    }

    #[test]
    fn crossing_trace_hits_the_near_face() {
        let hull = unit_box();
        let trace = trace_hull_solid(&hull, vec3(-64.0, 0.0, 0.0), vec3(64.0, 0.0, 0.0), &ops()).unwrap();
        assert!(!trace.start_solid);
        assert!(trace.fraction > 0.3 && trace.fraction < 0.45);
        assert!((trace.end.x + 16.0).abs() < 1.0);
        assert_eq!(trace.plane.normal, vec3(-1.0, 0.0, 0.0));
    }

    #[test]
    fn inside_start_reports_solid() {
        let hull = unit_box();
        let trace = trace_hull_solid(&hull, vec3(0.0, 0.0, 0.0), vec3(64.0, 0.0, 0.0), &ops()).unwrap();
        assert!(trace.start_solid);
        assert_eq!(
            hull_point_contents(&hull, vec3(0.0, 0.0, 0.0), &ops()).unwrap(),
            q1::CONTENTS_SOLID
        );
        assert_eq!(
            hull_point_contents(&hull, vec3(64.0, 0.0, 0.0), &ops()).unwrap(),
            q1::CONTENTS_EMPTY
        );
    }
}
