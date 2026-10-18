use qa_core::primitives::{Bounds, GeometryId, IndexSpan, Plane, SurfaceFlags, Vec3};
use qa_world::{
    collision::{
        CollisionStore, Contents,
        brushes::{Brush, BrushTree, CollisionLeaf, ModelRoot},
        surfaces::SurfaceTable,
    },
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

pub fn inline_box(runtime: &mut qa_app::Runtime, size: f32) -> qa_core::primitives::GeometryId {
    let planes = [
        Vec3([1.0, 0.0, 0.0]),
        Vec3([-1.0, 0.0, 0.0]),
        Vec3([0.0, 1.0, 0.0]),
        Vec3([0.0, -1.0, 0.0]),
        Vec3([0.0, 0.0, 1.0]),
        Vec3([0.0, 0.0, -1.0]),
    ]
    .map(|normal| Plane::oriented(normal, size))
    .to_vec();
    runtime
        .geometry
        .load_brushes(
            planes,
            vec![Brush {
                sides: IndexSpan { first: 0, count: 6 },
                contents: Contents::SOLID,
            }],
            Vec::new(),
            SurfaceTable::flags(vec![SurfaceFlags::default(); 6]),
            BrushTree {
                planes: vec![],
                nodes: vec![],
                leaves: vec![
                    CollisionLeaf {
                        stored_contents: None,
                        brushes: IndexSpan::default(),
                        patches: Default::default(),
                    },
                    CollisionLeaf {
                        stored_contents: None,
                        brushes: IndexSpan { first: 0, count: 1 },
                        patches: Default::default(),
                    },
                ],
                leaf_brushes: vec![0],
                models: vec![ModelRoot::Leaf(0), ModelRoot::Leaf(1)],
                leaf_patches: Vec::new(),
            },
            vec![
                Bounds {
                    mins: Vec3([-512.0; 3]),
                    maxs: Vec3([512.0; 3]),
                },
                Bounds {
                    mins: Vec3([-size; 3]),
                    maxs: Vec3([size; 3]),
                },
            ],
        )
        .unwrap()
}
