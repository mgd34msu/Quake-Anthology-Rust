use qa_core::primitives::{Bounds, GeometryId, Plane, Vec3};
use qa_world::{
    collision::CollisionStore,
    portals::{AreaPortals, Portal},
    visibility::{PvsRows, SurfaceSpan, VisLeaf, VisNode, VisibilityWorld},
};

pub fn collision(
    store: &CollisionStore,
    selectors: [Option<u32>; 2],
    rows: PvsRows,
) -> qa_app::WorldCollision {
    let bounds = Bounds {
        mins: Vec3([-10.0; 3]),
        maxs: Vec3([10.0; 3]),
    };
    let world = VisibilityWorld::load(
        vec![Plane::oriented(Vec3([1.0, 0.0, 0.0]), 0.0)],
        vec![VisNode {
            plane: 0,
            children: [-1, -2],
            bounds,
            surfaces: SurfaceSpan::default(),
        }],
        selectors
            .into_iter()
            .enumerate()
            .map(|(area, selector)| VisLeaf {
                selector,
                area: Some(area as u32),
                solid: false,
                bounds,
                surfaces: SurfaceSpan::default(),
            })
            .collect(),
        vec![],
        0,
        0,
        rows,
    )
    .unwrap();
    qa_app::WorldCollision::new(
        store,
        GeometryId {
            slot: 0,
            generation: 1,
        },
        0,
    )
    .with_visibility(std::sync::Arc::new(world))
    .with_portals(Some(
        AreaPortals::load(
            2,
            1,
            vec![Portal {
                number: 0,
                first: 0,
                second: 1,
            }],
        )
        .unwrap(),
    ))
}
