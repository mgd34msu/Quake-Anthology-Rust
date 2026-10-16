//! Ordered box-to-leaf collection over admitted BSP nodes.
use qa_core::primitives::{Bounds, Plane, leaf_from_child};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BoxLeaves {
    pub count: usize,
    /// First node split by the box, or -1 if no split occurred.
    pub top_node: i32,
}

/// Query-local stack; loaded topology supplies its validated maximum depth.
pub struct LeafScratch {
    nodes: Box<[i32]>,
}
impl LeafScratch {
    pub fn new(depth: usize) -> Self {
        Self {
            nodes: vec![0; depth.max(1)].into_boxed_slice(),
        }
    }
    pub fn query(
        &mut self,
        root: i32,
        bounds: Bounds,
        output: &mut [u32],
        node: impl Fn(i32) -> (Plane, [i32; 2]),
    ) -> Option<BoxLeaves> {
        if output.is_empty() {
            return Some(BoxLeaves {
                count: 0,
                top_node: -1,
            });
        }
        let mut count = 0;
        let result = self.visit(root, bounds, node, |leaf| {
            output[count] = leaf;
            count += 1;
            Ok::<_, std::convert::Infallible>(count < output.len())
        });
        match result {
            Ok(result) => result,
            Err(never) => match never {},
        }
    }

    /// Visit in native order without collecting or truncating. Returning false
    /// stops after the current leaf; None reports insufficient traversal stack.
    pub fn visit<E>(
        &mut self,
        root: i32,
        bounds: Bounds,
        node: impl Fn(i32) -> (Plane, [i32; 2]),
        mut leaf: impl FnMut(u32) -> Result<bool, E>,
    ) -> Result<Option<BoxLeaves>, E> {
        let mut top = 1;
        let mut result = BoxLeaves {
            count: 0,
            top_node: -1,
        };
        self.nodes[0] = root;
        while top != 0 {
            top -= 1;
            let child = self.nodes[top];
            if child < 0 {
                result.count = match result.count.checked_add(1) {
                    Some(count) => count,
                    None => return Ok(None),
                };
                if !leaf(leaf_from_child(child))? {
                    return Ok(Some(result));
                }
                continue;
            }
            let (plane, children) = node(child);
            let sides = box_sides(bounds, plane);
            if sides == 3 && result.top_node == -1 {
                result.top_node = child;
            }
            // Front zero precedes back one, including shared leaf references.
            for side in [1, 0] {
                if sides & (1 << side) != 0 {
                    let Some(slot) = self.nodes.get_mut(top) else {
                        return Ok(None);
                    };
                    *slot = children[side];
                    top += 1;
                }
            }
        }
        Ok(Some(result))
    }
}

/// Native box/plane side bits shared by BSP admission and world queries.
pub fn box_sides(bounds: Bounds, plane: Plane) -> u8 {
    if let Some(axis) = plane.axis {
        let axis = axis.index();
        if plane.distance <= bounds.mins.0[axis] {
            return 1;
        }
        if plane.distance >= bounds.maxs.0[axis] {
            return 2;
        }
        return 3;
    }
    let far = bounds.corner(plane.normal, true);
    let near = bounds.corner(plane.normal, false);
    u8::from(plane.signed_distance(far) >= 0.0) | (u8::from(plane.signed_distance(near) < 0.0) << 1)
}
