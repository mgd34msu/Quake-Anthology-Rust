use qa_render::{
    edges::{DepthPolicy, Edge, Edges, ProjectedVertex, polygon_winding, prepare_polygon},
    scene::Viewport,
};

pub fn add_polygon(
    edges: &mut Edges,
    surface: u32,
    key: u32,
    rank: u32,
    vertices: &[ProjectedVertex],
) -> bool {
    let (viewport, policy) = edges.projection();
    let mut storage = vec![Edge::default(); vertices.len()];
    let prepared = prepare_polygon(
        vertices,
        viewport,
        policy,
        polygon_winding(vertices).ok(),
        &mut storage,
    );
    let (count, winding) = match prepared {
        Ok((count, winding)) => (count, Some(winding)),
        Err(()) => (0, None),
    };
    edges.add_polygon(surface, key, rank, &storage[..count], winding)
}

#[allow(dead_code)]
pub fn certified_rows(
    vertices: &[ProjectedVertex],
    viewport: Viewport,
    policy: DepthPolicy,
) -> Result<std::ops::Range<u32>, ()> {
    let mut storage = vec![Edge::default(); vertices.len()];
    let (count, winding) = prepare_polygon(
        vertices,
        viewport,
        policy,
        polygon_winding(vertices).ok(),
        &mut storage,
    )?;
    qa_render::edges::certified_rows(&storage[..count], viewport, policy, Some(winding))
}
