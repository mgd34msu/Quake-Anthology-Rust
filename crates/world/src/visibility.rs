//! Shared BSP visibility; file readers normalize selectors and row encoding at load.

use qa_core::primitives::{Bounds, Plane, Vec3};

mod load;
mod query;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SurfaceSpan {
    pub first: u32,
    pub count: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct VisNode {
    pub plane: u32,
    /// Negative values encode leaf = -1 - child.
    pub children: [i32; 2],
    pub bounds: Bounds,
    /// Direct surface ids owned by this splitting node.
    pub surfaces: SurfaceSpan,
}

#[derive(Clone, Copy, Debug)]
pub struct VisLeaf {
    pub selector: Option<u32>,
    pub area: Option<u32>,
    pub solid: bool,
    pub bounds: Bounds,
    /// Range into the world's leaf-surface reference array.
    pub surfaces: SurfaceSpan,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VisibilityError {
    Size,
    Plane(usize),
    Bounds(usize),
    Child(usize),
    Root,
    Cycle,
    SurfaceRange(usize),
    SurfaceReference(usize),
    SurfaceOwnership(u32),
    Selector(usize),
    PvsOffset(u32),
    PvsEncoding(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VisibilityQueryError {
    Origin,
    Frustum,
    Selector(u32),
    ScratchSize,
}

/// The only compressed row representation: nonzero literals, or zero + run length.
#[derive(Debug)]
pub struct PvsRows {
    offsets: Box<[Option<u32>]>,
    encoded: Box<[u8]>,
}

impl PvsRows {
    pub fn load(offsets: Vec<Option<u32>>, encoded: Vec<u8>) -> Result<Self, VisibilityError> {
        if offsets.len() > u32::MAX as usize || encoded.len() > u32::MAX as usize {
            return Err(VisibilityError::Size);
        }
        let mut row = vec![0; offsets.len().div_ceil(8)];
        for &offset in offsets.iter().flatten() {
            decode_rle(&encoded, offset, &mut row)?;
        }
        Ok(Self {
            offsets: offsets.into_boxed_slice(),
            encoded: encoded.into_boxed_slice(),
        })
    }

    pub fn all_visible(selector_count: usize) -> Self {
        Self {
            offsets: vec![None; selector_count].into_boxed_slice(),
            encoded: Box::default(),
        }
    }

    pub fn selector_count(&self) -> usize {
        self.offsets.len()
    }

    pub fn row_bytes(&self) -> usize {
        self.offsets.len().div_ceil(8)
    }

    /// Missing rows follow qsrc Mod_DecompressVis's all-visible fallback.
    pub fn read_into(
        &self,
        selector: Option<u32>,
        destination: &mut [u8],
    ) -> Result<(), VisibilityQueryError> {
        if destination.len() != self.row_bytes() {
            return Err(VisibilityQueryError::ScratchSize);
        }
        let Some(selector) = selector else {
            destination.fill(255);
            return Ok(());
        };
        let Some(&offset) = self.offsets.get(selector as usize) else {
            return Err(VisibilityQueryError::Selector(selector));
        };
        if let Some(offset) = offset {
            decode_rle(&self.encoded, offset, destination)
                .map_err(|_| VisibilityQueryError::ScratchSize)
        } else {
            destination.fill(255);
            Ok(())
        }
    }
}

fn decode_rle(encoded: &[u8], offset: u32, destination: &mut [u8]) -> Result<(), VisibilityError> {
    let mut at = offset as usize;
    let mut written = 0;
    while written < destination.len() {
        let byte = *encoded.get(at).ok_or(VisibilityError::PvsOffset(offset))?;
        at += 1;
        if byte != 0 {
            destination[written] = byte;
            written += 1;
        } else {
            let count = usize::from(
                *encoded
                    .get(at)
                    .ok_or(VisibilityError::PvsEncoding(offset))?,
            );
            at += 1;
            if count == 0 || count > destination.len() - written {
                return Err(VisibilityError::PvsEncoding(offset));
            }
            destination[written..written + count].fill(0);
            written += count;
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug)]
struct ParentEdge {
    node: u32,
    next: u32,
}

#[derive(Debug)]
pub struct VisibilityWorld {
    planes: Box<[Plane]>,
    nodes: Box<[VisNode]>,
    leaves: Box<[VisLeaf]>,
    leaf_surfaces: Box<[u32]>,
    surface_owners: Box<[Option<u32>]>,
    parent_heads: Box<[u32]>,
    parent_edges: Box<[ParentEdge]>,
    root: i32,
    pvs: PvsRows,
}

impl VisibilityWorld {
    pub fn point_in_leaf(&self, point: Vec3) -> Option<u32> {
        if point.0.iter().any(|value| !value.is_finite()) {
            return None;
        }
        let mut child = self.root;
        // qsrc model.c Mod_PointInLeaf uses > 0, including the back-side tie.
        while child >= 0 {
            let node = &self.nodes[child as usize];
            let plane = self.planes[node.plane as usize];
            let side = usize::from(plane.normal.dot(point) <= plane.distance);
            child = node.children[side];
        }
        Some((-1 - i64::from(child)) as u32)
    }

    pub fn leaf(&self, index: u32) -> Option<&VisLeaf> {
        self.leaves.get(index as usize)
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn leaf_count(&self) -> usize {
        self.leaves.len()
    }

    pub fn surface_count(&self) -> usize {
        self.surface_owners.len()
    }

    pub fn pvs(&self) -> &PvsRows {
        &self.pvs
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VisibilityCounters {
    pub nodes_marked: usize,
    pub nodes_visited: usize,
    pub leaves_visited: usize,
    pub surfaces: usize,
    pub bounds_rejected: usize,
}

#[derive(Clone, Copy, Default)]
struct WalkStep {
    child: i32,
    planes: u8,
    emit_node: bool,
}

pub struct ViewVisibility {
    primary: Box<[u8]>,
    secondary: Box<[u8]>,
    node_marks: Box<[u32]>,
    leaf_marks: Box<[u32]>,
    surface_marks: Box<[u32]>,
    emitted_marks: Box<[u32]>,
    walk_marks: Box<[u32]>,
    ancestors: Box<[u32]>,
    walk: Box<[WalkStep]>,
    visible: Box<[u32]>,
    depth_keys: Box<[u32]>,
    visible_count: usize,
    generation: u32,
    counters: VisibilityCounters,
}

impl ViewVisibility {
    pub fn new(world: &VisibilityWorld) -> Self {
        let nodes = world.node_count();
        let leaves = world.leaf_count();
        let surfaces = world.surface_count();
        Self {
            primary: vec![0; world.pvs.row_bytes()].into_boxed_slice(),
            secondary: vec![0; world.pvs.row_bytes()].into_boxed_slice(),
            node_marks: vec![0; nodes].into_boxed_slice(),
            leaf_marks: vec![0; leaves].into_boxed_slice(),
            surface_marks: vec![0; surfaces].into_boxed_slice(),
            emitted_marks: vec![0; surfaces].into_boxed_slice(),
            walk_marks: vec![0; nodes + leaves].into_boxed_slice(),
            ancestors: vec![0; nodes].into_boxed_slice(),
            // Each descent adds a far child and an emit step; acyclic depth <= nodes.
            walk: vec![WalkStep::default(); nodes * 2 + 1].into_boxed_slice(),
            visible: vec![0; surfaces].into_boxed_slice(),
            depth_keys: vec![0; surfaces].into_boxed_slice(),
            visible_count: 0,
            generation: 0,
            counters: VisibilityCounters::default(),
        }
    }

    pub fn visible_surfaces(&self) -> &[u32] {
        &self.visible[..self.visible_count]
    }

    /// Parallel keys retain near-leaf / node-surface / far-leaf ordering.
    pub fn depth_keys(&self) -> &[u32] {
        &self.depth_keys[..self.visible_count]
    }

    pub fn counters(&self) -> VisibilityCounters {
        self.counters
    }

    fn next_generation(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        if self.generation == 0 {
            self.node_marks.fill(0);
            self.leaf_marks.fill(0);
            self.surface_marks.fill(0);
            self.emitted_marks.fill(0);
            self.walk_marks.fill(0);
            self.generation = 1;
        }
    }
}
