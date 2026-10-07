use super::{Contents, Trace};
use qa_core::primitives::{Axis, Plane, Vec3};

#[derive(Clone, Copy, Debug)]
pub struct ClipNode {
    pub plane: u32,
    pub children: [i32; 2],
}

#[derive(Clone, Copy, Debug)]
pub struct HullModel {
    pub roots: [i32; 3],
}

#[derive(Debug, PartialEq, Eq)]
pub enum HullError {
    Plane,
    Node,
    Cycle,
    Root,
    EmptyModels,
}

#[derive(Clone, Copy, Default)]
pub(crate) struct Frame {
    node: i32,
    far: i32,
    p1f: f32,
    p2f: f32,
    p1: Vec3,
    p2: Vec3,
    mid: Vec3,
    midf: f32,
    fraction: f32,
    t1: f32,
    plane: u32,
    awaiting_near: bool,
}

pub(crate) struct Hull<'a> {
    pub nodes: &'a [ClipNode],
    pub planes: &'a [Plane],
    pub root: i32,
}

impl Hull<'_> {
    fn point_contents(&self, point: Vec3, mut node: i32) -> i32 {
        while node >= 0 {
            let clip = self.nodes[node as usize];
            let side = usize::from(self.planes[clip.plane as usize].signed_distance(point) < 0.0);
            node = clip.children[side];
        }
        node
    }

    fn blocked(&self, point: Vec3, node: i32, mask: Contents) -> bool {
        Contents::from_q1(self.point_contents(point, node)).0 & mask.0 != 0
    }

    pub(crate) fn trace(
        &self,
        start: Vec3,
        end: Vec3,
        mask: Contents,
        stack: &mut [Frame],
    ) -> Trace {
        let mut trace = Trace::clear(end);
        trace.all_solid = true;
        stack[0] = Frame {
            node: self.root,
            p1f: 0.0,
            p2f: 1.0,
            p1: start,
            p2: end,
            ..Frame::default()
        };
        let mut count = 1;
        while count != 0 {
            let index = count - 1;
            let mut frame = stack[index];
            if !frame.awaiting_near {
                if frame.node < 0 {
                    if Contents::from_q1(frame.node).0 & mask.0 != 0 {
                        trace.start_solid = true;
                    } else {
                        trace.all_solid = false;
                        if frame.node == -1 {
                            trace.in_open = true;
                        } else {
                            trace.in_water = true;
                        }
                    }
                    count -= 1;
                    continue;
                }
                let clip = self.nodes[frame.node as usize];
                let plane = self.planes[clip.plane as usize];
                let t1 = plane.signed_distance(frame.p1);
                let t2 = plane.signed_distance(frame.p2);
                if t1 >= 0.0 && t2 >= 0.0 {
                    stack[index].node = clip.children[0];
                    continue;
                }
                if t1 < 0.0 && t2 < 0.0 {
                    stack[index].node = clip.children[1];
                    continue;
                }
                let side = usize::from(t1 < 0.0);
                // world.c's unsuffixed DIST_EPSILON promotes only this expression.
                // Stored distances, midpoint arithmetic and results remain f32.
                let numerator = f64::from(t1) + if side == 1 { 0.03125 } else { -0.03125 };
                let fraction = (numerator / f64::from(t1 - t2)) as f32;
                frame.fraction = fraction.clamp(0.0, 1.0);
                frame.midf = frame.p1f + (frame.p2f - frame.p1f) * frame.fraction;
                frame.mid = frame.p1.lerp(frame.p2, frame.fraction);
                frame.t1 = t1;
                frame.plane = clip.plane;
                frame.far = clip.children[side ^ 1];
                frame.awaiting_near = true;
                stack[index] = frame;
                stack[count] = Frame {
                    node: clip.children[side],
                    p1f: frame.p1f,
                    p2f: frame.midf,
                    p1: frame.p1,
                    p2: frame.mid,
                    ..Frame::default()
                };
                count += 1;
                continue;
            }
            if !self.blocked(frame.mid, frame.far, mask) {
                stack[index] = Frame {
                    node: frame.far,
                    p1f: frame.midf,
                    p2f: frame.p2f,
                    p1: frame.mid,
                    p2: frame.p2,
                    ..Frame::default()
                };
                continue;
            }
            if trace.all_solid {
                return trace;
            }
            trace.plane = self.planes[frame.plane as usize];
            if frame.t1 < 0.0 {
                trace.plane.normal = Vec3(trace.plane.normal.0.map(|value| 0.0 - value));
                trace.plane.distance = -trace.plane.distance;
                trace.plane.axis = None;
            }
            trace.contents = Contents::from_q1(self.point_contents(frame.mid, frame.far));
            while self.blocked(frame.mid, self.root, mask) {
                // The original compound assignment also uses an unsuffixed double.
                frame.fraction = (f64::from(frame.fraction) - 0.1) as f32;
                if frame.fraction < 0.0 {
                    break;
                }
                frame.midf = frame.p1f + (frame.p2f - frame.p1f) * frame.fraction;
                frame.mid = frame.p1.lerp(frame.p2, frame.fraction);
            }
            trace.fraction = frame.midf;
            trace.end = frame.mid;
            return trace;
        }
        trace
    }
}

fn validate(nodes: &[ClipNode], plane_count: usize) -> Result<Vec<usize>, HullError> {
    if nodes.len() > i32::MAX as usize {
        return Err(HullError::Node);
    }
    for node in nodes {
        if node.plane as usize >= plane_count
            || node
                .children
                .iter()
                .any(|child| *child >= 0 && *child as usize >= nodes.len())
        {
            return Err(HullError::Node);
        }
    }
    let mut colors = vec![0u8; nodes.len()];
    let mut depths = vec![0usize; nodes.len()];
    let mut pending = Vec::new();
    for root in 0..nodes.len() {
        if colors[root] == 2 {
            continue;
        }
        pending.push((root, false));
        while let Some((index, finish)) = pending.pop() {
            if finish {
                depths[index] = 1 + nodes[index]
                    .children
                    .iter()
                    .filter(|child| **child >= 0)
                    .map(|child| depths[*child as usize])
                    .max()
                    .unwrap_or(0);
                colors[index] = 2;
                continue;
            }
            if colors[index] == 2 {
                continue;
            }
            if colors[index] == 1 {
                return Err(HullError::Cycle);
            }
            colors[index] = 1;
            pending.push((index, true));
            for child in nodes[index]
                .children
                .iter()
                .rev()
                .filter(|child| **child >= 0)
            {
                pending.push((*child as usize, false));
            }
        }
    }
    Ok(depths)
}

pub struct Q1Hulls {
    planes: Box<[Plane]>,
    drawing: Box<[ClipNode]>,
    clips: Box<[ClipNode]>,
    models: Box<[HullModel]>,
    stack: Box<[Frame]>,
}

impl Q1Hulls {
    pub fn load(
        planes: Vec<Plane>,
        drawing: Vec<ClipNode>,
        clips: Vec<ClipNode>,
        models: Vec<HullModel>,
    ) -> Result<Self, HullError> {
        if models.is_empty() {
            return Err(HullError::EmptyModels);
        }
        for plane in &planes {
            if !plane.distance.is_finite()
                || !plane.normal.0.iter().all(|value| value.is_finite())
                || plane.normal.dot(plane.normal) == 0.0
            {
                return Err(HullError::Plane);
            }
            let axis = match plane.axis {
                Some(Axis::X) => Some(0),
                Some(Axis::Y) => Some(1),
                Some(Axis::Z) => Some(2),
                None => None,
            };
            if let Some(axis) = axis
                && plane.normal.0
                    != std::array::from_fn(|index| if index == axis { 1.0 } else { 0.0 })
            {
                return Err(HullError::Plane);
            }
        }
        let drawing_depth = validate(&drawing, planes.len())?;
        let clip_depth = validate(&clips, planes.len())?;
        let mut depth = 0;
        for model in &models {
            for (index, root) in model.roots.iter().enumerate() {
                let depths = if index == 0 {
                    &drawing_depth
                } else {
                    &clip_depth
                };
                if *root >= 0 {
                    depth = depth.max(*depths.get(*root as usize).ok_or(HullError::Root)?);
                }
            }
        }
        Ok(Self {
            planes: planes.into_boxed_slice(),
            drawing: drawing.into_boxed_slice(),
            clips: clips.into_boxed_slice(),
            models: models.into_boxed_slice(),
            stack: vec![Frame::default(); depth + 1].into_boxed_slice(),
        })
    }

    pub fn point_contents(&self, point: Vec3) -> Contents {
        Contents::from_q1(
            Hull {
                nodes: &self.drawing,
                planes: &self.planes,
                root: self.models[0].roots[0],
            }
            .point_contents(point, self.models[0].roots[0]),
        )
    }

    pub fn trace(
        &mut self,
        start: Vec3,
        end: Vec3,
        mins: Vec3,
        maxs: Vec3,
        mask: Contents,
    ) -> Trace {
        let width = maxs.0[0] - mins.0[0];
        let index = if width < 3.0 {
            0
        } else if width <= 32.0 {
            1
        } else {
            2
        };
        let hull_mins = [[0.0; 3], [-16.0, -16.0, -24.0], [-32.0, -32.0, -24.0]][index];
        let offset = Vec3(std::array::from_fn(|axis| hull_mins[axis] - mins.0[axis]));
        let local = |point: Vec3| Vec3(std::array::from_fn(|axis| point.0[axis] - offset.0[axis]));
        let mut trace = Hull {
            nodes: if index == 0 {
                &self.drawing
            } else {
                &self.clips
            },
            planes: &self.planes,
            root: self.models[0].roots[index],
        }
        .trace(local(start), local(end), mask, &mut self.stack);
        trace.end = if trace.fraction == 1.0 {
            end
        } else {
            Vec3(std::array::from_fn(|axis| {
                trace.end.0[axis] + offset.0[axis]
            }))
        };
        trace
    }
}
