//! Ordered box-to-leaf collection over admitted BSP nodes.
use qa_core::primitives::{Axis, Bounds, Plane, Vec3};

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
        let mut top = 1;
        let mut result = BoxLeaves {
            count: 0,
            top_node: -1,
        };
        self.nodes[0] = root;
        while top != 0 && result.count < output.len() {
            top -= 1;
            let child = self.nodes[top];
            if child < 0 {
                output[result.count] = (-1i64 - i64::from(child)) as u32;
                result.count += 1;
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
                    *self.nodes.get_mut(top)? = children[side];
                    top += 1;
                }
            }
        }
        Some(result)
    }
}

fn box_sides(bounds: Bounds, plane: Plane) -> u8 {
    if let Some(axis) = plane.axis {
        let axis = match axis {
            Axis::X => 0,
            Axis::Y => 1,
            Axis::Z => 2,
        };
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
