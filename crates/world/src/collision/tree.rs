use super::brushes::{
    Brush, BrushMap, BrushWork, GeometryError, clip_brush, position_brush, valid_plane,
};
use super::{Contents, LeafGate, NonAxialOffset, Trace, TraceQuery};
use qa_core::primitives::{Axis, Bounds, ClipNode, Plane, Vec3};

#[derive(Clone, Copy, Debug)]
pub struct CollisionLeaf {
    pub stored_contents: Option<Contents>,
    pub first_brush: u32,
    pub brush_count: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelRoot {
    Tree(i32),
    Leaf(u32),
}

/// Cold interchange preserves nodes, leaves, brush IDs and ordered references.
/// Negative children are -1-leaf; direct model leaves own their membership.
pub struct BrushTree {
    pub planes: Vec<Plane>,
    pub nodes: Vec<ClipNode>,
    pub leaves: Vec<CollisionLeaf>,
    pub leaf_brushes: Vec<u32>,
    pub models: Vec<ModelRoot>,
}

pub(super) struct Topology {
    planes: Box<[Plane]>,
    nodes: Box<[ClipNode]>,
    leaves: Box<[CollisionLeaf]>,
    leaf_brushes: Box<[u32]>,
    leaf_contents: Box<[Contents]>,
    models: Box<[ModelRoot]>,
    max_depth: usize,
}

#[derive(Clone, Copy, Default)]
struct Frame {
    node: i32,
    p1f: f32,
    p2f: f32,
    p1: Vec3,
    p2: Vec3,
}

/// All mutable query state belongs to this caller, never to loaded geometry.
pub struct BrushScratch {
    frames: Box<[Frame]>,
    stamps: Box<[u32]>,
    generation: u32,
    position_leaves: Box<[u32]>,
}

impl BrushScratch {
    pub(super) fn load(map: &BrushMap, position_capacity: usize) -> Self {
        Self {
            frames: vec![Frame::default(); map.topology.max_depth].into_boxed_slice(),
            stamps: vec![0; map.brushes.len()].into_boxed_slice(),
            generation: 0,
            position_leaves: vec![0; position_capacity].into_boxed_slice(),
        }
    }

    fn begin(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        if self.generation == 0 {
            self.stamps.fill(0);
            self.generation = 1;
        }
    }
}

fn axis_index(axis: Axis) -> usize {
    match axis {
        Axis::X => 0,
        Axis::Y => 1,
        Axis::Z => 2,
    }
}

fn leaf_index(child: i32) -> usize {
    (-1i64 - i64::from(child)) as usize
}

fn root_child(root: ModelRoot) -> i32 {
    match root {
        ModelRoot::Tree(child) => child,
        ModelRoot::Leaf(leaf) => (-1i64 - i64::from(leaf)) as i32,
    }
}

impl Topology {
    pub(super) fn load(tree: BrushTree, brushes: &[Brush]) -> Result<Self, GeometryError> {
        if tree.nodes.len() > i32::MAX as usize
            || tree.leaves.len() > i32::MAX as usize + 1
            || brushes.len() > u32::MAX as usize
        {
            return Err(GeometryError::Capacity);
        }
        if tree.planes.iter().any(|plane| {
            if !valid_plane(*plane) {
                return true;
            }
            if let Some(axis) = plane.axis {
                let mut normal = [0.0; 3];
                // Q2 PlaneTypeForNormal classifies either unit sign as axial.
                normal[axis_index(axis)] = plane.normal.0[axis_index(axis)].signum();
                return plane.normal != Vec3(normal);
            }
            false
        }) {
            return Err(GeometryError::TreePlane);
        }
        let valid_child = |child: i32| {
            if child < 0 {
                leaf_index(child) < tree.leaves.len()
            } else {
                (child as usize) < tree.nodes.len()
            }
        };
        if tree.models.is_empty()
            || tree.nodes.iter().any(|node| {
                node.plane as usize >= tree.planes.len()
                    || node.children.iter().any(|&child| !valid_child(child))
            })
            || tree.models.iter().any(|root| match *root {
                ModelRoot::Tree(child) => !valid_child(child),
                ModelRoot::Leaf(leaf) => leaf as usize >= tree.leaves.len(),
            })
            || tree.leaves.iter().any(|leaf| {
                (leaf.first_brush as usize)
                    .checked_add(leaf.brush_count as usize)
                    .is_none_or(|end| end > tree.leaf_brushes.len())
            })
            || tree
                .leaf_brushes
                .iter()
                .any(|&id| id as usize >= brushes.len())
        {
            return Err(GeometryError::TreeRange);
        }

        // Memoized depth admits shared children while an active-node mark
        // rejects cycles. Validate disconnected nodes as well as model roots.
        let mut colors = vec![0u8; tree.nodes.len()];
        let mut depths = vec![0usize; tree.nodes.len()];
        let mut stack = Vec::with_capacity(tree.nodes.len());
        for root in 0..tree.nodes.len() {
            if colors[root] == 2 {
                continue;
            }
            colors[root] = 1;
            stack.push((root, 0usize));
            while let Some(&mut (node, ref mut next)) = stack.last_mut() {
                if *next < 2 {
                    let child = tree.nodes[node].children[*next];
                    *next += 1;
                    if child < 0 {
                        continue;
                    }
                    let child = child as usize;
                    if colors[child] == 1 {
                        return Err(GeometryError::TreeCycle);
                    }
                    if colors[child] == 0 {
                        colors[child] = 1;
                        stack.push((child, 0));
                    }
                } else {
                    let children = tree.nodes[node].children;
                    let depth = children
                        .into_iter()
                        .map(|child| if child < 0 { 1 } else { depths[child as usize] })
                        .max()
                        .unwrap_or(1);
                    depths[node] = depth.checked_add(1).ok_or(GeometryError::Capacity)?;
                    colors[node] = 2;
                    stack.pop();
                }
            }
        }
        let max_depth = tree
            .models
            .iter()
            .map(|root| {
                let child = root_child(*root);
                if child < 0 { 1 } else { depths[child as usize] }
            })
            .max()
            .unwrap_or(1);
        if max_depth > isize::MAX as usize / size_of::<Frame>() {
            return Err(GeometryError::Capacity);
        }
        let leaf_contents = tree
            .leaves
            .iter()
            .map(|leaf| {
                leaf.stored_contents.unwrap_or_else(|| {
                    tree.leaf_brushes[leaf.first_brush as usize
                        ..leaf.first_brush as usize + leaf.brush_count as usize]
                        .iter()
                        .fold(Contents::EMPTY, |contents, &id| {
                            contents | brushes[id as usize].contents
                        })
                })
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Ok(Self {
            planes: tree.planes.into_boxed_slice(),
            nodes: tree.nodes.into_boxed_slice(),
            leaves: tree.leaves.into_boxed_slice(),
            leaf_brushes: tree.leaf_brushes.into_boxed_slice(),
            leaf_contents,
            models: tree.models.into_boxed_slice(),
            max_depth,
        })
    }

    pub(super) fn members(&self, leaf: &CollisionLeaf) -> &[u32] {
        &self.leaf_brushes
            [leaf.first_brush as usize..leaf.first_brush as usize + leaf.brush_count as usize]
    }

    pub(super) fn point_leaf(&self, model: usize, point: Vec3) -> Option<&CollisionLeaf> {
        let mut child = root_child(*self.models.get(model)?);
        while child >= 0 {
            let node = self.nodes[child as usize];
            let distance = self.planes[node.plane as usize].signed_distance(point);
            child = node.children[usize::from(distance < 0.0)];
        }
        Some(&self.leaves[leaf_index(child)])
    }

    pub(super) fn trace(
        &self,
        map: &BrushMap,
        model: usize,
        query: TraceQuery,
        scratch: &mut BrushScratch,
    ) -> Trace {
        let mut trace = Trace::clear(query.end);
        let Some(&root) = self.models.get(model) else {
            return trace;
        };
        if scratch.frames.len() < self.max_depth || scratch.stamps.len() < map.brushes.len() {
            return trace;
        }
        let work = BrushWork::new(query);
        scratch.begin();
        if query.start == query.end {
            match root {
                ModelRoot::Leaf(leaf) => {
                    self.visit_leaf(map, leaf, &work, true, &mut trace, scratch);
                }
                ModelRoot::Tree(child) => {
                    let limit = query.rules.position_leaf_limit as usize;
                    if limit > scratch.position_leaves.len() {
                        return trace;
                    }
                    let bounds = Bounds {
                        mins: Vec3(std::array::from_fn(|axis| {
                            work.start.0[axis] + work.mins.0[axis] - 1.0
                        })),
                        maxs: Vec3(std::array::from_fn(|axis| {
                            work.start.0[axis] + work.maxs.0[axis] + 1.0
                        })),
                    };
                    let count = self.position_leaves(child, bounds, limit, scratch);
                    for index in 0..count {
                        self.visit_leaf(
                            map,
                            scratch.position_leaves[index],
                            &work,
                            true,
                            &mut trace,
                            scratch,
                        );
                        if trace.all_solid {
                            break;
                        }
                    }
                }
            }
        } else {
            self.sweep(map, root_child(root), &work, &mut trace, scratch);
        }
        trace.end = work.end_position(trace.fraction);
        trace
    }

    fn visit_leaf(
        &self,
        map: &BrushMap,
        leaf: u32,
        work: &BrushWork,
        position: bool,
        trace: &mut Trace,
        scratch: &mut BrushScratch,
    ) {
        if work.query.rules.leaf_gate == LeafGate::StoredContents
            && !self.leaf_contents[leaf as usize].intersects(work.query.mask)
        {
            return;
        }
        for &id in self.members(&self.leaves[leaf as usize]) {
            let index = id as usize;
            if scratch.stamps[index] == scratch.generation {
                continue;
            }
            scratch.stamps[index] = scratch.generation;
            let brush = map.brushes[index];
            if !brush.contents.intersects(work.query.mask) {
                continue;
            }
            if position {
                position_brush(work, &map.planes, brush, map.axial_bounds[index], trace);
            } else {
                clip_brush(work, &map.planes, brush, &map.surfaces, trace);
            }
            if trace.fraction == 0.0 {
                return;
            }
        }
    }

    fn position_leaves(
        &self,
        root: i32,
        bounds: Bounds,
        limit: usize,
        scratch: &mut BrushScratch,
    ) -> usize {
        let mut top = 1;
        let mut count = 0;
        scratch.frames[0].node = root;
        while top != 0 && count < limit {
            top -= 1;
            let child = scratch.frames[top].node;
            if child < 0 {
                scratch.position_leaves[count] = leaf_index(child) as u32;
                count += 1;
                continue;
            }
            let node = self.nodes[child as usize];
            let sides = box_sides(bounds, self.planes[node.plane as usize]);
            // Native position collection visits front0 before back1.
            if sides & 2 != 0 {
                scratch.frames[top].node = node.children[1];
                top += 1;
            }
            if sides & 1 != 0 {
                scratch.frames[top].node = node.children[0];
                top += 1;
            }
        }
        count
    }

    fn sweep(
        &self,
        map: &BrushMap,
        root: i32,
        work: &BrushWork,
        trace: &mut Trace,
        scratch: &mut BrushScratch,
    ) {
        let mut top = 1;
        scratch.frames[0] = Frame {
            node: root,
            p1f: 0.0,
            p2f: 1.0,
            p1: work.start,
            p2: work.end,
        };
        while top != 0 {
            top -= 1;
            let frame = scratch.frames[top];
            if trace.fraction <= frame.p1f {
                continue;
            }
            if frame.node < 0 {
                self.visit_leaf(
                    map,
                    leaf_index(frame.node) as u32,
                    work,
                    false,
                    trace,
                    scratch,
                );
                continue;
            }
            let node = self.nodes[frame.node as usize];
            let plane = self.planes[node.plane as usize];
            let t1 = plane.signed_distance(frame.p1);
            let t2 = plane.signed_distance(frame.p2);
            let offset = if let Some(axis) = plane.axis {
                work.extents.0[axis_index(axis)]
            } else if work.tree_point {
                0.0
            } else {
                match work.query.rules.nonaxial_offset {
                    NonAxialOffset::Fixed(offset) => offset,
                    NonAxialOffset::ProjectedExtents => {
                        // Native fabs promotes each f32 product to double;
                        // the sum rounds once when assigned to float offset.
                        (f64::from(work.extents.0[0] * plane.normal.0[0]).abs()
                            + f64::from(work.extents.0[1] * plane.normal.0[1]).abs()
                            + f64::from(work.extents.0[2] * plane.normal.0[2]).abs())
                            as f32
                    }
                }
            };
            let margin = work.query.rules.tree_margin;
            let only_side = if t1 >= offset + margin && t2 >= offset + margin {
                Some(0)
            } else if t1 < -offset - margin && t2 < -offset - margin {
                Some(1)
            } else {
                None
            };
            if let Some(side) = only_side {
                scratch.frames[top] = Frame {
                    node: node.children[side],
                    ..frame
                };
                top += 1;
                continue;
            }
            let epsilon = work.query.rules.contact_epsilon;
            let (side, near, far) = if t1 < t2 {
                let inverse = (1.0f64 / f64::from(t1 - t2)) as f32;
                (
                    1,
                    ((f64::from(t1 - offset) + epsilon) * f64::from(inverse)) as f32,
                    ((f64::from(t1 + offset) + epsilon) * f64::from(inverse)) as f32,
                )
            } else if t1 > t2 {
                let inverse = (1.0f64 / f64::from(t1 - t2)) as f32;
                (
                    0,
                    ((f64::from(t1 + offset) + epsilon) * f64::from(inverse)) as f32,
                    ((f64::from(t1 - offset) - epsilon) * f64::from(inverse)) as f32,
                )
            } else {
                (0, 1.0, 0.0)
            };
            let near = unit_fraction(near);
            let far = unit_fraction(far);
            let near_frame = Frame {
                node: node.children[side],
                p1f: frame.p1f,
                p2f: frame.p1f + (frame.p2f - frame.p1f) * near,
                p1: frame.p1,
                p2: frame.p1.lerp(frame.p2, near),
            };
            let far_frame = Frame {
                node: node.children[side ^ 1],
                p1f: frame.p1f + (frame.p2f - frame.p1f) * far,
                p2f: frame.p2f,
                p1: frame.p1.lerp(frame.p2, far),
                p2: frame.p2,
            };
            scratch.frames[top] = far_frame;
            scratch.frames[top + 1] = near_frame;
            top += 2;
        }
    }
}

fn unit_fraction(mut fraction: f32) -> f32 {
    if fraction < 0.0 {
        fraction = 0.0;
    }
    if fraction > 1.0 {
        fraction = 1.0;
    }
    fraction
}

fn box_sides(bounds: Bounds, plane: Plane) -> u8 {
    if let Some(axis) = plane.axis {
        let axis = axis_index(axis);
        if plane.distance <= bounds.mins.0[axis] {
            return 1;
        }
        if plane.distance >= bounds.maxs.0[axis] {
            return 2;
        }
        return 3;
    }
    let far = Vec3(std::array::from_fn(|axis| {
        if plane.normal.0[axis] < 0.0 {
            bounds.mins.0[axis]
        } else {
            bounds.maxs.0[axis]
        }
    }));
    let near = Vec3(std::array::from_fn(|axis| {
        if plane.normal.0[axis] < 0.0 {
            bounds.maxs.0[axis]
        } else {
            bounds.mins.0[axis]
        }
    }));
    u8::from(plane.normal.dot(far) >= plane.distance)
        | (u8::from(plane.normal.dot(near) < plane.distance) << 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collision::{EntityTraceRules, TraceRules};
    use qa_core::primitives::SurfaceFlags;

    #[test]
    fn stamp_rollover_cannot_skip_a_brush_with_a_stale_future_epoch() -> Result<(), GeometryError> {
        let map = BrushMap::load(
            vec![Plane {
                normal: Vec3([1.0, 0.0, 0.0]),
                distance: 0.0,
                axis: None,
            }],
            vec![Brush {
                first_plane: 0,
                plane_count: 1,
                contents: Contents::SOLID,
            }],
        )?;
        let mut scratch = map.scratch();
        scratch.generation = u32::MAX;
        scratch.stamps.fill(1);
        let query = TraceQuery::point(
            Vec3([1.0, 0.0, 0.0]),
            Vec3([-1.0, 0.0, 0.0]),
            TraceRules::ARENA,
            EntityTraceRules::ARENA,
        );
        let trace = map.trace_model(0, query, &mut scratch);
        assert_eq!(trace.fraction, (1.0 - 0.125) / 2.0);
        assert_eq!(trace.contents, Contents::SOLID);
        assert_eq!(trace.surface, SurfaceFlags::default());
        Ok(())
    }
}
