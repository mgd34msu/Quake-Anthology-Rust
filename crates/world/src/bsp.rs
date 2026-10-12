//! BSP operations shared by collision and visibility.
use qa_core::primitives::{ClipNode, Plane, Vec3};

pub(crate) trait SplitNode {
    fn plane(&self) -> u32;
    fn children(&self) -> [i32; 2];
}
impl SplitNode for ClipNode {
    fn plane(&self) -> u32 {
        self.plane
    }
    fn children(&self) -> [i32; 2] {
        self.children
    }
}
impl SplitNode for crate::visibility::VisNode {
    fn plane(&self) -> u32 {
        self.plane
    }
    fn children(&self) -> [i32; 2] {
        self.children
    }
}
pub(crate) fn node_view<'a, N: SplitNode>(
    planes: &'a [Plane],
    nodes: &'a [N],
) -> impl Fn(i32) -> (Plane, [i32; 2]) + 'a {
    move |child| {
        let node = &nodes[child as usize];
        (planes[node.plane() as usize], node.children())
    }
}

pub(crate) fn descend(
    mut child: i32,
    point: Vec3,
    back_on_tie: bool,
    node: impl Fn(i32) -> (Plane, [i32; 2]),
) -> i32 {
    while child >= 0 {
        let (plane, children) = node(child);
        let distance = plane.signed_distance(point);
        let back = distance < 0.0 || (back_on_tie && distance == 0.0);
        child = children[usize::from(back)];
    }
    child
}

#[cfg(test)]
#[path = "../tests/bsp/oracle.rs"]
mod tests;
