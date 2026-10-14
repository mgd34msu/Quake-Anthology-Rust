//! Ordered surface probes over the already validated common BSP node table.
use super::{VisibilityQueryError, VisibilityWorld};
use qa_core::primitives::{Plane, Vec3};

#[derive(Clone, Copy)]
enum Task {
    Segment { node: i32, start: Vec3, end: Vec3 },
    Surfaces { node: u32, point: Vec3 },
}
pub struct SurfaceRayScratch {
    stack: Box<[Task]>,
}
impl SurfaceRayScratch {
    pub fn load(world: &VisibilityWorld) -> Self {
        Self {
            stack: vec![
                Task::Segment {
                    node: -1,
                    start: Vec3::default(),
                    end: Vec3::default()
                };
                world.segment_capacity
            ]
            .into_boxed_slice(),
        }
    }
}
impl VisibilityWorld {
    /// Visit near subtree, this node's surfaces, then far subtree, matching
    /// native light probes. This reads the same tree as visibility; it never
    /// constructs collision geometry or changes physical trace rules.
    pub fn probe_segment<T>(
        &self,
        start: Vec3,
        end: Vec3,
        scratch: &mut SurfaceRayScratch,
        mut probe: impl FnMut(u32, Plane, Vec3) -> Option<T>,
    ) -> Result<Option<T>, VisibilityQueryError> {
        if !start.is_finite() || !end.is_finite() {
            return Err(VisibilityQueryError::Origin);
        }
        if scratch.stack.len() < self.segment_capacity {
            return Err(VisibilityQueryError::ScratchSize);
        }
        scratch.stack[0] = Task::Segment {
            node: self.root,
            start,
            end,
        };
        let mut count = 1;
        while count != 0 {
            count -= 1;
            match scratch.stack[count] {
                Task::Surfaces { node, point } => {
                    let node = &self.nodes[node as usize];
                    let plane = self.planes[node.plane as usize];
                    for surface in node.surfaces.range() {
                        if let Some(value) = probe(surface as u32, plane, point) {
                            return Ok(Some(value));
                        }
                    }
                }
                Task::Segment {
                    mut node,
                    start,
                    end,
                } => {
                    while node >= 0 {
                        let split = &self.nodes[node as usize];
                        let plane = self.planes[split.plane as usize];
                        let front = plane.signed_distance(start);
                        let back = plane.signed_distance(end);
                        let side = usize::from(front < 0.);
                        if usize::from(back < 0.) == side {
                            node = split.children[side];
                            continue;
                        }
                        let fraction = front / (front - back);
                        let middle = start.lerp(end, fraction);
                        scratch.stack[count] = Task::Segment {
                            node: split.children[1 - side],
                            start: middle,
                            end,
                        };
                        scratch.stack[count + 1] = Task::Surfaces {
                            node: node as u32,
                            point: middle,
                        };
                        count += 2;
                        node = split.children[side];
                        // The near segment has a new endpoint; resume through
                        // the same bounded stack instead of recursive calls.
                        scratch.stack[count] = Task::Segment {
                            node,
                            start,
                            end: middle,
                        };
                        count += 1;
                        break;
                    }
                }
            }
        }
        Ok(None)
    }
}
