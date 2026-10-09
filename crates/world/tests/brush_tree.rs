use qa_core::primitives::{Axis, Bounds, ClipNode, GeometryId, Plane, SurfaceFlags, Vec3};
use qa_world::collision::brushes::{Brush, BrushTree, CollisionLeaf, GeometryError, ModelRoot};
use qa_world::collision::{
    CollisionStore, Contents, EntityTraceRules, LeafGate, StoreError, TraceQuery, TraceRules,
};

struct Case {
    store: CollisionStore,
    geometry: GeometryId,
}

fn load_tree(
    planes: Vec<Plane>,
    brushes: Vec<Brush>,
    surfaces: Vec<SurfaceFlags>,
    tree: BrushTree,
) -> Result<Case, StoreError> {
    let bounds = vec![
        Bounds {
            mins: Vec3([-32768.0; 3]),
            maxs: Vec3([32768.0; 3])
        };
        tree.models.len()
    ];
    let mut store = CollisionStore::new();
    let geometry = store.load_brushes(planes, brushes, surfaces, tree, bounds)?;
    Ok(Case { store, geometry })
}

fn load(planes: Vec<Plane>, brushes: Vec<Brush>) -> Result<Case, StoreError> {
    let tree = BrushTree::direct(brushes.len()).map_err(StoreError::Brush)?;
    let surfaces = vec![SurfaceFlags::default(); planes.len()];
    load_tree(planes, brushes, surfaces, tree)
}

fn plane(normal: [f32; 3], distance: f32) -> Plane {
    Plane {
        normal: Vec3(normal),
        distance,
        axis: None,
    }
}

fn x_split(distance: f32) -> Plane {
    Plane {
        normal: Vec3([1.0, 0.0, 0.0]),
        distance,
        axis: Some(Axis::X),
    }
}

fn query(rules: TraceRules, start: [f32; 3], end: [f32; 3]) -> TraceQuery<'static> {
    TraceQuery::point(
        Vec3(start),
        Vec3(end),
        rules,
        match rules.leaf_gate {
            LeafGate::StoredContents => EntityTraceRules::QUAKE2,
            LeafGate::Brushes => EntityTraceRules::ARENA,
        },
    )
}

fn leaf(contents: Option<Contents>, first: u32, count: u32) -> CollisionLeaf {
    CollisionLeaf {
        stored_contents: contents,
        first_brush: first,
        brush_count: count,
    }
}

#[test]
fn native_point_contents_and_model_membership_are_caller_selected() {
    let map = load_tree(
        vec![
            plane([1.0, 0.0, 0.0], 10.0),
            plane([-1.0, 0.0, 0.0], 10.0),
            plane([0.0, 1.0, 0.0], 2.0),
        ],
        vec![
            Brush {
                first_plane: 0,
                plane_count: 1,
                contents: Contents::SOLID,
            },
            Brush {
                first_plane: 1,
                plane_count: 1,
                contents: Contents::WATER,
            },
            Brush {
                first_plane: 2,
                plane_count: 1,
                contents: Contents::PLAYER_CLIP,
            },
        ],
        vec![SurfaceFlags(11), SurfaceFlags(22), SurfaceFlags(33)],
        BrushTree {
            planes: vec![x_split(0.0)],
            nodes: vec![ClipNode {
                plane: 0,
                children: [-1, -2],
            }],
            leaves: vec![
                leaf(Some(Contents::SLIME), 0, 1),
                leaf(Some(Contents::LAVA), 1, 1),
                leaf(None, 2, 1),
            ],
            leaf_brushes: vec![1, 0, 2],
            models: vec![ModelRoot::Tree(0), ModelRoot::Leaf(2)],
        },
    )
    .unwrap();
    assert_eq!(
        map.store.point_contents_model(
            map.geometry,
            0,
            Vec3([1.0, 0.0, 0.0]),
            EntityTraceRules::QUAKE2
        ),
        Contents::SLIME
    );
    assert_eq!(
        map.store.point_contents_model(
            map.geometry,
            0,
            Vec3([1.0, 0.0, 0.0]),
            EntityTraceRules::ARENA
        ),
        Contents::WATER
    );
    assert_eq!(
        map.store.point_contents_model(
            map.geometry,
            0,
            Vec3([-1.0, 0.0, 0.0]),
            EntityTraceRules::QUAKE2
        ),
        Contents::LAVA
    );
    assert_eq!(
        map.store.point_contents_model(
            map.geometry,
            0,
            Vec3([-1.0, 0.0, 0.0]),
            EntityTraceRules::ARENA
        ),
        Contents::SOLID
    );
    // Points exactly on a native splitting plane select front0.
    assert_eq!(
        map.store
            .point_contents_model(map.geometry, 0, Vec3::default(), EntityTraceRules::QUAKE2),
        Contents::SLIME
    );
    assert_eq!(
        map.store.point_contents_model(
            map.geometry,
            1,
            Vec3([0.0, 1.0, 0.0]),
            EntityTraceRules::ARENA
        ),
        Contents::PLAYER_CLIP
    );
    assert_eq!(
        map.store.point_contents_model(
            map.geometry,
            1,
            Vec3([0.0, 3.0, 0.0]),
            EntityTraceRules::ARENA
        ),
        Contents::EMPTY
    );
    assert_eq!(
        map.store
            .point_contents_model(map.geometry, 99, Vec3::default(), EntityTraceRules::ARENA),
        Contents::EMPTY
    );
    let mut scratch = map.store.scratch();
    let query = TraceQuery {
        mask: Contents::PLAYER_CLIP,
        ..query(TraceRules::ARENA, [0.0, 10.0, 0.0], [0.0; 3])
    };
    assert_eq!(
        map.store
            .trace_model(map.geometry, 0, query, &mut scratch)
            .fraction,
        1.0
    );
    let inline = map.store.trace_model(map.geometry, 1, query, &mut scratch);
    assert_eq!(inline.fraction, (10.0 - 2.0 - 0.125) / 10.0);
    assert_eq!(inline.surface, SurfaceFlags(33));
    assert_eq!(inline.contents, Contents::PLAYER_CLIP);
}

#[test]
fn ordered_leaf_references_preserve_ties_and_each_query_has_its_own_stamps() {
    let map = load_tree(
        vec![plane([1.0, 0.0, 0.0], 0.0); 2],
        vec![
            Brush {
                first_plane: 0,
                plane_count: 1,
                contents: Contents::SOLID,
            },
            Brush {
                first_plane: 1,
                plane_count: 1,
                contents: Contents::PLAYER_CLIP,
            },
        ],
        vec![SurfaceFlags(11), SurfaceFlags(22)],
        BrushTree {
            planes: vec![],
            nodes: vec![],
            leaves: vec![leaf(None, 0, 3)],
            leaf_brushes: vec![1, 1, 0],
            models: vec![ModelRoot::Leaf(0)],
        },
    )
    .unwrap();
    let mut first = map.store.scratch();
    let mut second = map.store.scratch();
    for rules in [TraceRules::LEGACY, TraceRules::ARENA, TraceRules::LEGACY] {
        let query = TraceQuery {
            mask: Contents::SOLID | Contents::PLAYER_CLIP,
            ..query(rules, [1.0, 0.0, 0.0], [-1.0, 0.0, 0.0])
        };
        for scratch in [&mut first, &mut second] {
            let trace = map.store.trace_model(map.geometry, 0, query, scratch);
            assert_eq!(trace.fraction, ((1.0 - rules.contact_epsilon) / 2.0) as f32);
            assert_eq!(trace.contents, Contents::PLAYER_CLIP);
            assert_eq!(trace.surface, SurfaceFlags(22));
            let solid = map.store.trace_model(
                map.geometry,
                0,
                TraceQuery {
                    mask: Contents::SOLID,
                    ..query
                },
                scratch,
            );
            assert_eq!(solid.contents, Contents::SOLID);
            assert_eq!(solid.surface, SurfaceFlags(11));
        }
    }
}

#[test]
fn near_first_and_front_first_position_order_keep_the_native_first_contact() {
    let map = load_tree(
        vec![plane([1.0, 0.0, 0.0], 0.0); 2],
        vec![
            Brush {
                first_plane: 0,
                plane_count: 1,
                contents: Contents::SOLID,
            },
            Brush {
                first_plane: 1,
                plane_count: 1,
                contents: Contents::PLAYER_CLIP,
            },
        ],
        vec![SurfaceFlags(11), SurfaceFlags(22)],
        BrushTree {
            planes: vec![x_split(0.0)],
            nodes: vec![ClipNode {
                plane: 0,
                children: [-1, -2],
            }],
            leaves: vec![
                leaf(Some(Contents::PLAYER_CLIP), 0, 1),
                leaf(Some(Contents::SOLID), 1, 1),
            ],
            leaf_brushes: vec![1, 0],
            models: vec![ModelRoot::Tree(0)],
        },
    )
    .unwrap();
    let mut scratch = map.store.scratch();
    for rules in [TraceRules::LEGACY, TraceRules::ARENA] {
        let moving = TraceQuery {
            mask: Contents::SOLID | Contents::PLAYER_CLIP,
            ..query(rules, [0.015625, 0.0, 0.0], [-0.0625, 0.0, 0.0])
        };
        let trace = map.store.trace_model(map.geometry, 0, moving, &mut scratch);
        assert_eq!(trace.fraction, 0.0);
        assert_eq!(trace.contents, Contents::PLAYER_CLIP);
        assert_eq!(trace.surface, SurfaceFlags(22));
        let position = map.store.trace_model(
            map.geometry,
            0,
            TraceQuery {
                start: Vec3::default(),
                end: Vec3::default(),
                ..moving
            },
            &mut scratch,
        );
        assert!(position.start_solid && position.all_solid);
        assert_eq!(position.contents, Contents::PLAYER_CLIP);
    }
}

#[test]
fn brush_clipping_uses_the_original_full_segment_after_tree_splitting() {
    let map = load_tree(
        vec![plane([1.0, 0.0, 0.0], 5.0)],
        vec![Brush {
            first_plane: 0,
            plane_count: 1,
            contents: Contents::SOLID,
        }],
        vec![SurfaceFlags(7)],
        BrushTree {
            planes: vec![x_split(0.0)],
            nodes: vec![ClipNode {
                plane: 0,
                children: [-1, -2],
            }],
            leaves: vec![
                leaf(Some(Contents::SOLID), 0, 1),
                leaf(Some(Contents::EMPTY), 1, 0),
            ],
            leaf_brushes: vec![0],
            models: vec![ModelRoot::Tree(0)],
        },
    )
    .unwrap();
    let mut scratch = map.store.scratch();
    for rules in [TraceRules::LEGACY, TraceRules::ARENA] {
        let trace = map.store.trace_model(
            map.geometry,
            0,
            query(rules, [10.0, 0.0, 0.0], [-10.0, 0.0, 0.0]),
            &mut scratch,
        );
        assert_eq!(
            trace.fraction,
            ((10.0 - 5.0 - rules.contact_epsilon) / 20.0) as f32
        );
        assert_eq!(trace.end.0[0], 10.0 + trace.fraction * -20.0);
    }
}

#[test]
fn q2_leaf_gate_precedes_stamp_and_q3_ignores_that_gate_on_the_same_tree() {
    let map = load_tree(
        vec![plane([1.0, 0.0, 0.0], 0.0)],
        vec![Brush {
            first_plane: 0,
            plane_count: 1,
            contents: Contents::SOLID,
        }],
        vec![SurfaceFlags(7)],
        BrushTree {
            planes: vec![x_split(0.0)],
            nodes: vec![ClipNode {
                plane: 0,
                children: [-1, -2],
            }],
            leaves: vec![
                leaf(Some(Contents::EMPTY), 0, 1),
                leaf(Some(Contents::SOLID), 1, 1),
            ],
            leaf_brushes: vec![0, 0],
            models: vec![ModelRoot::Tree(0), ModelRoot::Tree(-1)],
        },
    )
    .unwrap();
    let mut scratch = map.store.scratch();
    let legacy = query(TraceRules::LEGACY, [10.0, 0.0, 0.0], [-10.0, 0.0, 0.0]);
    assert_eq!(
        map.store
            .trace_model(map.geometry, 0, legacy, &mut scratch)
            .fraction,
        (10.0 - 0.03125) / 20.0
    );
    assert_eq!(
        map.store
            .trace_model(map.geometry, 1, legacy, &mut scratch)
            .fraction,
        1.0
    );
    let arena = query(TraceRules::ARENA, [10.0, 0.0, 0.0], [-10.0, 0.0, 0.0]);
    assert_eq!(
        map.store
            .trace_model(map.geometry, 1, arena, &mut scratch)
            .fraction,
        (10.0 - 0.125) / 20.0
    );
}

#[test]
fn stationary_leaf_truncation_is_compatibility_data_and_can_be_extended_cold() {
    let mut nodes = vec![ClipNode {
        plane: 0,
        children: [1, -2],
    }];
    for index in 1..=10 {
        let child = if index == 10 { -1 } else { index + 1 };
        nodes.push(ClipNode {
            plane: 0,
            children: [child, child],
        });
    }
    // The shared front subtree yields1024 empty leaves before the solid back.
    let map = load_tree(
        vec![plane([1.0, 0.0, 0.0], 1.0)],
        vec![Brush {
            first_plane: 0,
            plane_count: 1,
            contents: Contents::SOLID,
        }],
        vec![SurfaceFlags::default()],
        BrushTree {
            planes: vec![x_split(0.0)],
            nodes,
            leaves: vec![
                leaf(Some(Contents::EMPTY), 0, 0),
                leaf(Some(Contents::SOLID), 0, 1),
            ],
            leaf_brushes: vec![0],
            models: vec![ModelRoot::Tree(0), ModelRoot::Leaf(1)],
        },
    )
    .unwrap();
    let mut native = map.store.scratch();
    let mut extended = map.store.scratch_with_position_capacity(1025).unwrap();
    for rules in [TraceRules::LEGACY, TraceRules::ARENA] {
        assert_eq!(
            map.store
                .trace_model(
                    map.geometry,
                    0,
                    query(rules, [0.0; 3], [0.0; 3]),
                    &mut native
                )
                .fraction,
            1.0
        );
        let custom = TraceRules {
            position_leaf_limit: 1025,
            ..rules
        };
        let blocked = map.store.trace_model(
            map.geometry,
            0,
            query(custom, [0.0; 3], [0.0; 3]),
            &mut extended,
        );
        assert_eq!(blocked.fraction, 0.0);
        assert!(blocked.start_solid && blocked.all_solid);
        assert_eq!(blocked.contents, Contents::SOLID);
    }
    let mut no_collection = map.store.scratch_with_position_capacity(0).unwrap();
    let zero_limit = TraceRules {
        position_leaf_limit: 0,
        ..TraceRules::ARENA
    };
    assert_eq!(
        map.store
            .trace_model(
                map.geometry,
                0,
                query(zero_limit, [0.0; 3], [0.0; 3]),
                &mut no_collection
            )
            .fraction,
        1.0
    );
    // Direct membership never enters the BSP position collector.
    assert_eq!(
        map.store
            .trace_model(
                map.geometry,
                1,
                query(zero_limit, [0.0; 3], [0.0; 3]),
                &mut no_collection
            )
            .fraction,
        0.0
    );
    assert_eq!(
        map.store
            .trace_model(
                map.geometry,
                1,
                query(TraceRules::ARENA, [0.0; 3], [0.0; 3]),
                &mut no_collection
            )
            .fraction,
        0.0
    );
}

#[test]
fn stationary_native_endpoint_preserves_q2_start_signed_zero() {
    let map = load(vec![], vec![]).unwrap();
    let mut scratch = map.store.scratch();
    for (rules, expected) in [(TraceRules::LEGACY, -0.0f32), (TraceRules::ARENA, 0.0f32)] {
        let trace = map.store.trace_model(
            map.geometry,
            0,
            query(rules, [-0.0, 0.0, 0.0], [0.0; 3]),
            &mut scratch,
        );
        assert_eq!(trace.fraction, 1.0);
        assert_eq!(trace.end.0[0].to_bits(), expected.to_bits());
    }
}

#[test]
fn axial_position_bounds_accept_both_native_prefix_orders_and_foreign_sides_fall_back() {
    for positive_first in [false, true] {
        let mins = [16_777_218.0, -10.0, -10.0];
        let maxs = [16_777_220.0, 10.0, 10.0];
        let mut planes = Vec::new();
        for axis in 0..3 {
            let mut negative = [0.0; 3];
            negative[axis] = -1.0;
            let mut positive = [0.0; 3];
            positive[axis] = 1.0;
            let mut pair = [plane(negative, -mins[axis]), plane(positive, maxs[axis])];
            if positive_first {
                pair.reverse();
            }
            planes.extend(pair);
        }
        let map = load(
            planes,
            vec![Brush {
                first_plane: 0,
                plane_count: 6,
                contents: Contents::SOLID,
            }],
        )
        .unwrap();
        let mut scratch = map.store.scratch();
        let position = TraceQuery {
            mins: Vec3([-1.0; 3]),
            maxs: Vec3([1.0; 3]),
            ..query(
                TraceRules::ARENA,
                [16_777_216.0, 0.0, 0.0],
                [16_777_216.0, 0.0, 0.0],
            )
        };
        assert!(
            !map.store
                .trace_model(map.geometry, 0, position, &mut scratch)
                .all_solid
        );
        let legacy = map.store.trace_model(
            map.geometry,
            0,
            TraceQuery {
                rules: TraceRules::LEGACY,
                entity_rules: EntityTraceRules::QUAKE2,
                ..position
            },
            &mut scratch,
        );
        assert!(legacy.all_solid);
    }
    // This is foreign convex geometry, not a Q3 assumed six-plane prefix.
    let map = load(
        vec![plane([1.0, 1.0, 0.0], -1.0); 6],
        vec![Brush {
            first_plane: 0,
            plane_count: 6,
            contents: Contents::SOLID,
        }],
    )
    .unwrap();
    assert_eq!(
        map.store
            .trace_model(
                map.geometry,
                0,
                query(TraceRules::ARENA, [0.0; 3], [0.0; 3]),
                &mut map.store.scratch()
            )
            .fraction,
        1.0
    );
}

#[test]
fn caller_side_margin_and_nonaxial_offset_select_visitation_on_one_graph() {
    for (split, start, end, mins, maxs, side_distance, expected_fraction) in [
        (
            x_split(0.0),
            [0.75, 0.0, 0.0],
            [0.0625, 0.0, 0.0],
            [0.0; 3],
            [0.0; 3],
            0.0,
            10.0 / 11.0,
        ),
        (
            plane([1.0, 0.0, 0.0], 0.0),
            [3.0, 0.0, 0.0],
            [2.0, 0.0, 0.0],
            [-1.0; 3],
            [1.0; 3],
            2.5,
            0.0,
        ),
    ] {
        let map = load_tree(
            vec![plane([1.0, 0.0, 0.0], side_distance)],
            vec![Brush {
                first_plane: 0,
                plane_count: 1,
                contents: Contents::SOLID,
            }],
            vec![SurfaceFlags::default()],
            BrushTree {
                planes: vec![split],
                nodes: vec![ClipNode {
                    plane: 0,
                    children: [-1, -2],
                }],
                leaves: vec![
                    leaf(Some(Contents::EMPTY), 0, 0),
                    leaf(Some(Contents::SOLID), 0, 1),
                ],
                leaf_brushes: vec![0],
                models: vec![ModelRoot::Tree(0)],
            },
        )
        .unwrap();
        let mut scratch = map.store.scratch();
        let legacy = TraceQuery {
            mins: Vec3(mins),
            maxs: Vec3(maxs),
            ..query(TraceRules::LEGACY, start, end)
        };
        assert_eq!(
            map.store
                .trace_model(map.geometry, 0, legacy, &mut scratch)
                .fraction,
            1.0
        );
        let arena = map.store.trace_model(
            map.geometry,
            0,
            TraceQuery {
                rules: TraceRules::ARENA,
                entity_rules: EntityTraceRules::ARENA,
                ..legacy
            },
            &mut scratch,
        );
        // The point endpoint lies inside the1/8 contact band, so the native
        // far split is10/11 rather than clamping to1 and pruning that leaf.
        // The nonaxial volume case is enclosed after the2048-offset visit.
        assert_eq!(arena.fraction, expected_fraction);
        assert_eq!(arena.contents, Contents::SOLID);
    }
}

#[test]
fn centered_min_only_tree_point_classification_does_not_remove_brush_support() {
    let origin = 16_777_216.0;
    let map = load_tree(
        vec![plane([-1.0, 0.0, 0.0], -(origin + 2.0))],
        vec![Brush {
            first_plane: 0,
            plane_count: 1,
            contents: Contents::SOLID,
        }],
        vec![SurfaceFlags::default()],
        BrushTree {
            planes: vec![plane([1.0, 0.0, 0.0], origin - 8.0)],
            nodes: vec![ClipNode {
                plane: 0,
                children: [-1, -2],
            }],
            leaves: vec![leaf(None, 0, 0), leaf(None, 0, 1)],
            leaf_brushes: vec![0],
            models: vec![ModelRoot::Tree(0), ModelRoot::Leaf(1)],
        },
    )
    .unwrap();
    let mut scratch = map.store.scratch();
    let query = TraceQuery {
        mins: Vec3([origin, 0.0, 0.0]),
        maxs: Vec3([origin + 2.0, 0.0, 0.0]),
        ..query(TraceRules::ARENA, [0.0; 3], [-4.0, 0.0, 0.0])
    };
    // Center rounds to origin: size0 becomes0 but size1 remains2. Q3's
    // nonaxial tree offset is0; the direct brush still has that2-unit support.
    let world = map.store.trace_model(map.geometry, 0, query, &mut scratch);
    assert!(!world.start_solid);
    let direct = map.store.trace_model(map.geometry, 1, query, &mut scratch);
    assert!(direct.start_solid);
    assert!(!direct.all_solid);
    assert_eq!(direct.fraction, 1.0);
}

fn minimal_tree() -> BrushTree {
    BrushTree {
        planes: vec![x_split(0.0)],
        nodes: vec![ClipNode {
            plane: 0,
            children: [-1, -1],
        }],
        leaves: vec![leaf(None, 0, 0)],
        leaf_brushes: vec![],
        models: vec![ModelRoot::Tree(0)],
    }
}

#[test]
fn load_rejects_bad_ranges_cycles_nonfinite_and_inconsistent_axis_metadata() {
    let mut cycle = minimal_tree();
    cycle.nodes[0].children[0] = 0;
    assert!(matches!(
        load_tree(vec![], vec![], vec![], cycle),
        Err(StoreError::Brush(GeometryError::TreeCycle))
    ));
    let mut bad_child = minimal_tree();
    bad_child.nodes[0].children[0] = -2;
    assert!(matches!(
        load_tree(vec![], vec![], vec![], bad_child),
        Err(StoreError::Brush(GeometryError::TreeRange))
    ));
    let mut bad_plane = minimal_tree();
    bad_plane.nodes[0].plane = 1;
    assert!(matches!(
        load_tree(vec![], vec![], vec![], bad_plane),
        Err(StoreError::Brush(GeometryError::TreeRange))
    ));
    let mut bad_model = minimal_tree();
    bad_model.models[0] = ModelRoot::Leaf(1);
    assert!(matches!(
        load_tree(vec![], vec![], vec![], bad_model),
        Err(StoreError::Brush(GeometryError::TreeRange))
    ));
    let mut bad_members = minimal_tree();
    bad_members.leaf_brushes.push(0);
    assert!(matches!(
        load_tree(vec![], vec![], vec![], bad_members),
        Err(StoreError::Brush(GeometryError::TreeRange))
    ));
    let mut bad_range = minimal_tree();
    bad_range.leaves[0].brush_count = 1;
    assert!(matches!(
        load_tree(vec![], vec![], vec![], bad_range),
        Err(StoreError::Brush(GeometryError::TreeRange))
    ));
    let mut nonfinite = minimal_tree();
    nonfinite.planes[0].distance = f32::NAN;
    assert!(matches!(
        load_tree(vec![], vec![], vec![], nonfinite),
        Err(StoreError::Brush(GeometryError::TreePlane))
    ));
    let mut bad_axis = minimal_tree();
    bad_axis.planes[0].normal = Vec3([-0.5, 0.0, 0.0]);
    assert!(matches!(
        load_tree(vec![], vec![], vec![], bad_axis),
        Err(StoreError::Brush(GeometryError::TreePlane))
    ));
    // An unused cycle is still malformed load data; shared acyclic children
    // are accepted by the1024-prefix fixture above.
    let mut disconnected = minimal_tree();
    disconnected.nodes.push(ClipNode {
        plane: 0,
        children: [1, -1],
    });
    assert!(matches!(
        load_tree(vec![], vec![], vec![], disconnected),
        Err(StoreError::Brush(GeometryError::TreeCycle))
    ));
}

#[test]
fn load_preserves_both_axial_unit_signs_and_rejects_malformed_normals() {
    for (component, axis) in [(0, Axis::X), (1, Axis::Y), (2, Axis::Z)] {
        for sign in [-1.0, 1.0] {
            let mut tree = minimal_tree();
            let mut normal = [0.0; 3];
            normal[component] = sign;
            tree.planes[0] = Plane {
                normal: Vec3(normal),
                distance: -17.0,
                axis: Some(axis),
            };
            assert!(load_tree(vec![], vec![], vec![], tree).is_ok());
        }
    }
    for normal in [
        [0.0, 0.0, 0.0],
        [0.5, 0.0, 0.0],
        [-2.0, 0.0, 0.0],
        [1.0, 0.25, 0.0],
        [0.0, -1.0, 0.0],
        [f32::NAN, 0.0, 0.0],
        [f32::INFINITY, 0.0, 0.0],
    ] {
        let mut tree = minimal_tree();
        tree.planes[0].normal = Vec3(normal);
        assert!(matches!(
            load_tree(vec![], vec![], vec![], tree),
            Err(StoreError::Brush(GeometryError::TreePlane))
        ));
    }
}
