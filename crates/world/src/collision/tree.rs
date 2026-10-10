use super::brushes::{
    Brush, BrushMap, BrushWork, GeometryError, clip_brush, position_brush, valid_plane,
};
use super::{Contents, LeafGate, NonAxialOffset, Trace, TraceQuery};
use qa_core::primitives::{Axis, Bounds, ClipNode, Plane, Vec3};
use qa_core::stamps::StampSet;

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

impl BrushTree {
    /// Ordered direct membership for analytic brush resources, including empty
    /// ones. Model queries still supply the resource and ordinal explicitly.
    pub fn direct(brush_count: usize) -> Result<Self, GeometryError> {
        let count = u32::try_from(brush_count).map_err(|_| GeometryError::Capacity)?;
        Ok(Self {
            planes: Vec::new(),
            nodes: Vec::new(),
            leaves: vec![CollisionLeaf {
                stored_contents: None,
                first_brush: 0,
                brush_count: count,
            }],
            leaf_brushes: (0..count).collect(),
            models: vec![ModelRoot::Leaf(0)],
        })
    }
}

pub(super) struct Topology {
    planes: Box<[Plane]>,
    nodes: Box<[ClipNode]>,
    leaves: Box<[CollisionLeaf]>,
    leaf_brushes: Box<[u32]>,
    leaf_contents: Box<[Contents]>,
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
pub(super) struct BrushScratch {
    frames: Box<[Frame]>,
    leaf_walk: crate::leaves::LeafScratch,
    stamps: StampSet,
    position_leaves: Box<[u32]>,
}

impl BrushScratch {
    pub(crate) fn new(depth: usize, brushes: usize, position_capacity: usize) -> Self {
        Self {
            frames: vec![Frame::default(); depth].into_boxed_slice(),
            leaf_walk: crate::leaves::LeafScratch::new(depth),
            stamps: StampSet::new(brushes),
            position_leaves: vec![0; position_capacity].into_boxed_slice(),
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
    pub(super) fn load(
        tree: BrushTree,
        brushes: &[Brush],
    ) -> Result<(Self, Vec<ModelRoot>), GeometryError> {
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
        Ok((
            Self {
                planes: tree.planes.into_boxed_slice(),
                nodes: tree.nodes.into_boxed_slice(),
                leaves: tree.leaves.into_boxed_slice(),
                leaf_brushes: tree.leaf_brushes.into_boxed_slice(),
                leaf_contents,
                max_depth,
            },
            tree.models,
        ))
    }

    pub(super) fn members(&self, leaf: &CollisionLeaf) -> &[u32] {
        &self.leaf_brushes
            [leaf.first_brush as usize..leaf.first_brush as usize + leaf.brush_count as usize]
    }

    pub(super) fn scratch_capacity(&self) -> usize {
        self.max_depth
    }

    pub(super) fn point_leaf(&self, root: ModelRoot, point: Vec3) -> &CollisionLeaf {
        let mut child = root_child(root);
        while child >= 0 {
            let node = self.nodes[child as usize];
            let distance = self.planes[node.plane as usize].signed_distance(point);
            child = node.children[usize::from(distance < 0.0)];
        }
        &self.leaves[leaf_index(child)]
    }

    pub(super) fn trace(
        &self,
        map: &BrushMap,
        root: ModelRoot,
        query: TraceQuery,
        scratch: &mut BrushScratch,
    ) -> Trace {
        let mut trace = Trace::clear(query.end);
        if scratch.frames.len() < self.max_depth || scratch.stamps.len() < map.brushes.len() {
            return trace;
        }
        let work = BrushWork::new(query);
        scratch.stamps.begin();
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
            if scratch.stamps.test_and_set(index) {
                continue;
            }
            let brush = map.brushes[index];
            if !brush.contents.intersects(work.query.mask) {
                continue;
            }
            if position {
                position_brush(work, &map.planes, brush, map.axial_bounds[index], trace);
            } else {
                clip_brush(work, &map.planes, brush, map.surfaces.borrow(), trace);
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
        scratch
            .leaf_walk
            .query(
                root,
                bounds,
                &mut scratch.position_leaves[..limit],
                |child| {
                    let node = self.nodes[child as usize];
                    (self.planes[node.plane as usize], node.children)
                },
            )
            .map_or(0, |result| result.count)
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

#[expect(
    clippy::manual_clamp,
    reason = "Retain the original hull trace's ordered float comparisons"
)]
fn unit_fraction(mut fraction: f32) -> f32 {
    if fraction < 0.0 {
        fraction = 0.0;
    }
    if fraction > 1.0 {
        fraction = 1.0;
    }
    fraction
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collision::{CollisionStore, StoreError, TraceRules};
    use qa_core::primitives::SurfaceFlags;

    #[test]
    fn successive_queries_keep_the_original_brush_contact() -> Result<(), StoreError> {
        let mut map = CollisionStore::new();
        let geometry = map.load_brushes(
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
            crate::collision::surfaces::SurfaceTable::flags(vec![SurfaceFlags::default()]),
            BrushTree::direct(1).map_err(StoreError::Brush)?,
            vec![Bounds {
                mins: Vec3([-2.0; 3]),
                maxs: Vec3([2.0; 3]),
            }],
        )?;
        let mut scratch = map.scratch();
        let query = TraceQuery::point(
            Vec3([1.0, 0.0, 0.0]),
            Vec3([-1.0, 0.0, 0.0]),
            TraceRules::ARENA,
            crate::collision::trace_policy(qa_core::primitives::RuleSetId::Quake3).1,
        );
        for _ in 0..3 {
            let trace = map.trace_model(geometry, 0, query, &mut scratch);
            assert_eq!(trace.fraction, (1.0 - 0.125) / 2.0);
            assert_eq!(trace.contents, Contents::SOLID);
            assert_eq!(trace.surface, SurfaceFlags::default());
        }
        Ok(())
    }
}
