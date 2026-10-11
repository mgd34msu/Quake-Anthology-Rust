use qa_render::{
    edges::{DepthPolicy, Edges, ProjectedVertex, polygon_winding},
    scene::Viewport,
};

pub fn add_polygon(
    edges: &mut Edges,
    surface: u32,
    key: u32,
    rank: u32,
    vertices: &[ProjectedVertex],
) -> bool {
    edges.add_polygon(surface, key, rank, vertices, polygon_winding(vertices).ok())
}

#[allow(dead_code)]
pub fn certified_rows(
    vertices: &[ProjectedVertex],
    viewport: Viewport,
    policy: DepthPolicy,
) -> Result<std::ops::Range<u32>, ()> {
    qa_render::edges::certified_rows(vertices, viewport, policy, polygon_winding(vertices).ok())
}
