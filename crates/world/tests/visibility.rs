use qa_core::primitives::{Bounds, Plane, Vec3};
use qa_world::visibility::{
    PvsRows, SurfaceSpan, ViewVisibility, VisLeaf, VisNode, VisibilityError, VisibilityQueryError,
    VisibilityWorld,
};

fn bounds(mins: [f32; 3], maxs: [f32; 3]) -> Bounds {
    Bounds {
        mins: Vec3(mins),
        maxs: Vec3(maxs),
    }
}

fn whole() -> Bounds {
    bounds([-16.0; 3], [16.0; 3])
}

fn plane(axis: usize) -> Plane {
    let mut normal = [0.0; 3];
    normal[axis] = 1.0;
    Plane {
        encoding: None,
        normal: Vec3(normal),
        distance: 0.0,
        axis: None,
    }
}

fn span(first: u32, count: u32) -> SurfaceSpan {
    SurfaceSpan { first, count }
}

fn leaf(selector: Option<u32>, first: u32, count: u32) -> VisLeaf {
    VisLeaf {
        selector,
        area: None,
        solid: false,
        bounds: whole(),
        surfaces: span(first, count),
    }
}

fn two_leaves(pvs: PvsRows) -> VisibilityWorld {
    VisibilityWorld::load(
        vec![plane(0)],
        vec![VisNode {
            plane: 0,
            children: [-1, -2],
            bounds: whole(),
            surfaces: SurfaceSpan::default(),
        }],
        vec![leaf(Some(0), 0, 1), leaf(Some(1), 1, 1)],
        vec![0, 1],
        2,
        0,
        pvs,
    )
    .unwrap()
}

#[test]
fn box_queries_use_the_loaded_visibility_leaf_metadata() {
    let world = two_leaves(PvsRows::all_visible(2));
    let mut scratch = qa_world::leaves::LeafScratch::new(world.node_count() + 1);
    let mut leaves = [u32::MAX; 2];
    let result = world
        .box_leaves(whole(), &mut leaves, &mut scratch)
        .unwrap();
    assert_eq!((result.count, result.top_node), (2, 0));
    assert_eq!(leaves, [0, 1]);
    assert_eq!(world.leaf(leaves[0]).unwrap().selector, Some(0));
    assert_eq!(world.leaf(leaves[1]).unwrap().selector, Some(1));
}

#[test]
fn rows_use_native_zero_runs_and_missing_rows_are_all_visible() {
    let rows = PvsRows::load(vec![Some(0); 16], vec![0, 2]).unwrap();
    let mut destination = [99; 2];
    rows.read_into(Some(0), &mut destination).unwrap();
    assert_eq!(destination, [0, 0]);
    rows.read_into(None, &mut destination).unwrap();
    assert_eq!(destination, [255, 255]);

    let rows = PvsRows::load(vec![Some(0); 24], vec![7, 0, 1, 129]).unwrap();
    let mut mixed = [0; 3];
    rows.read_into(Some(0), &mut mixed).unwrap();
    assert_eq!(mixed, [7, 0, 129]);

    let rows = PvsRows::load(vec![None, Some(0)], vec![2]).unwrap();
    let mut destination = [0; 1];
    rows.read_into(Some(0), &mut destination).unwrap();
    assert_eq!(destination, [255]);
    rows.read_into(Some(1), &mut destination).unwrap();
    assert_eq!(destination, [2]);

    let malformed = [vec![0], vec![0, 0], vec![0, 2]];
    for encoded in malformed {
        assert!(PvsRows::load(vec![Some(0); 8], encoded).is_err());
    }
    assert!(PvsRows::load(vec![Some(4)], vec![1]).is_err());
}

#[test]
fn secondary_rows_union_and_failed_queries_clear_previous_output() {
    let world = two_leaves(PvsRows::load(vec![Some(0), Some(1)], vec![1, 2]).unwrap());
    let mut view = ViewVisibility::new(&world);
    view.query(&world, Vec3([1.0; 3]), Some(0), None, &[], &[])
        .unwrap();
    assert_eq!(view.visible_surfaces(), &[0]);
    view.query(&world, Vec3([1.0; 3]), Some(0), Some(1), &[], &[])
        .unwrap();
    assert_eq!(view.visible_surfaces(), &[0, 1]);
    assert_eq!(
        view.query(&world, Vec3([1.0; 3]), Some(2), None, &[], &[]),
        Err(VisibilityQueryError::Selector(2))
    );
    assert!(view.visible_surfaces().is_empty());
}

#[test]
fn all_incoming_dag_parents_are_marked_without_duplicate_surfaces() {
    let empty = SurfaceSpan::default();
    let nodes = vec![
        VisNode {
            plane: 0,
            children: [1, 2],
            bounds: whole(),
            surfaces: empty,
        },
        VisNode {
            plane: 1,
            children: [3, -2],
            bounds: whole(),
            surfaces: empty,
        },
        VisNode {
            plane: 1,
            children: [3, -2],
            bounds: whole(),
            surfaces: empty,
        },
        VisNode {
            plane: 2,
            children: [-1, -2],
            bounds: whole(),
            surfaces: empty,
        },
    ];
    let mut solid = leaf(None, 1, 0);
    solid.solid = true;
    let world = VisibilityWorld::load(
        vec![plane(0), plane(1), plane(2)],
        nodes,
        vec![leaf(Some(0), 0, 1), solid],
        vec![0],
        1,
        0,
        PvsRows::load(vec![Some(0)], vec![1]).unwrap(),
    )
    .unwrap();
    let mut view = ViewVisibility::new(&world);
    view.query(&world, Vec3([1.0; 3]), Some(0), None, &[], &[])
        .unwrap();
    assert_eq!(view.visible_surfaces(), &[0]);
    assert_eq!(view.counters().nodes_marked, 4);
    assert_eq!(view.counters().nodes_visited, 4);
    assert_eq!(view.counters().leaves_visited, 1);
}

#[test]
fn node_owned_surfaces_keep_native_near_node_far_order() {
    let world = VisibilityWorld::load(
        vec![plane(0), plane(1)],
        vec![
            VisNode {
                plane: 0,
                children: [1, -3],
                bounds: whole(),
                surfaces: span(1, 1),
            },
            VisNode {
                plane: 1,
                children: [-1, -2],
                bounds: whole(),
                surfaces: span(0, 1),
            },
        ],
        vec![leaf(None, 0, 3), leaf(None, 3, 3), leaf(None, 6, 2)],
        vec![2, 0, 1, 3, 0, 1, 4, 1],
        5,
        0,
        PvsRows::all_visible(0),
    )
    .unwrap();
    let mut view = ViewVisibility::new(&world);
    view.query(&world, Vec3([1.0; 3]), None, None, &[], &[])
        .unwrap();
    assert_eq!(view.visible_surfaces(), &[2, 0, 3, 1, 4]);
    assert_eq!(view.depth_keys(), &[0, 1, 2, 3, 4]);
    view.query(&world, Vec3([-1.0; 3]), None, None, &[], &[])
        .unwrap();
    assert_eq!(view.visible_surfaces(), &[4, 1, 3, 0, 2]);
    assert_eq!(world.point_in_leaf(Vec3([1.0; 3])), Some(0));
    assert_eq!(world.point_in_leaf(Vec3([0.0; 3])), Some(2));
    assert_eq!(world.point_in_leaf(Vec3([f32::NAN, 0.0, 0.0])), None);
}

#[test]
fn areas_and_frustum_reject_leaves_and_shared_surfaces_emit_once() {
    let mut front = leaf(Some(0), 0, 2);
    front.bounds = bounds([0.0, -16.0, -16.0], [16.0; 3]);
    let mut back = leaf(Some(1), 2, 2);
    back.bounds = bounds([-16.0; 3], [-1.0, 16.0, 16.0]);
    back.area = Some(1);
    let world = VisibilityWorld::load(
        vec![plane(0)],
        vec![VisNode {
            plane: 0,
            children: [-1, -2],
            bounds: whole(),
            surfaces: SurfaceSpan::default(),
        }],
        vec![front, back],
        vec![0, 1, 1, 2],
        3,
        0,
        PvsRows::all_visible(2),
    )
    .unwrap();
    let mut view = ViewVisibility::new(&world);
    view.query(&world, Vec3([1.0; 3]), None, None, &[], &[])
        .unwrap();
    assert_eq!(view.visible_surfaces(), &[0, 1, 2]);
    view.query(&world, Vec3([1.0; 3]), None, None, &[2], &[])
        .unwrap();
    assert_eq!(view.visible_surfaces(), &[0, 1]);
    view.query(&world, Vec3([1.0; 3]), None, None, &[], &[plane(0)])
        .unwrap();
    assert_eq!(view.visible_surfaces(), &[0, 1]);
    assert_eq!(view.counters().bounds_rejected, 1);
}

#[test]
fn missing_primary_or_secondary_row_includes_unclustered_nonsolid_leaf() {
    let world = VisibilityWorld::load(
        vec![],
        vec![],
        vec![leaf(None, 0, 1)],
        vec![0],
        1,
        -1,
        PvsRows::load(vec![None, Some(0)], vec![0, 1]).unwrap(),
    )
    .unwrap();
    let mut view = ViewVisibility::new(&world);
    view.query(&world, Vec3::default(), Some(1), None, &[], &[])
        .unwrap();
    assert!(view.visible_surfaces().is_empty());
    view.query(&world, Vec3::default(), Some(0), None, &[], &[])
        .unwrap();
    assert_eq!(view.visible_surfaces(), &[0]);
    view.query(&world, Vec3::default(), Some(1), Some(0), &[], &[])
        .unwrap();
    assert_eq!(view.visible_surfaces(), &[0]);
}

#[test]
fn load_rejects_bad_topology_ownership_and_external_ranges() {
    let node = VisNode {
        plane: 0,
        children: [0, -1],
        bounds: whole(),
        surfaces: span(0, 1),
    };
    assert_eq!(
        VisibilityWorld::load(
            vec![plane(0)],
            vec![node],
            vec![leaf(None, 0, 1)],
            vec![0],
            1,
            0,
            PvsRows::all_visible(0)
        )
        .unwrap_err(),
        VisibilityError::Cycle
    );
    let first = VisNode {
        children: [1, -1],
        ..node
    };
    let second = VisNode {
        children: [-1, -1],
        ..node
    };
    assert_eq!(
        VisibilityWorld::load(
            vec![plane(0)],
            vec![first, second],
            vec![leaf(None, 0, 1)],
            vec![0],
            1,
            0,
            PvsRows::all_visible(0)
        )
        .unwrap_err(),
        VisibilityError::SurfaceOwnership(0)
    );
    assert_eq!(
        VisibilityWorld::load(
            vec![],
            vec![],
            vec![leaf(None, 0, 1)],
            vec![1],
            1,
            -1,
            PvsRows::all_visible(0)
        )
        .unwrap_err(),
        VisibilityError::SurfaceReference(0)
    );
    assert_eq!(
        VisibilityWorld::load(
            vec![],
            vec![],
            vec![leaf(Some(1), 0, 0)],
            vec![],
            0,
            -1,
            PvsRows::all_visible(1)
        )
        .unwrap_err(),
        VisibilityError::Selector(0)
    );
    let mut bad_bounds = leaf(None, 0, 0);
    bad_bounds.bounds.maxs.0[0] = f32::NAN;
    assert_eq!(
        VisibilityWorld::load(
            vec![],
            vec![],
            vec![bad_bounds],
            vec![],
            0,
            -1,
            PvsRows::all_visible(0)
        )
        .unwrap_err(),
        VisibilityError::Bounds(0)
    );
}
