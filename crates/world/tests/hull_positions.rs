use qa_core::primitives::{Bounds, GeometryId, ModelRotation, ModelRules, Plane, RuleSetId, Vec3};
use qa_world::collision::{
    CollisionStore, Contents, HullPositionLimits, HullShape, StoreError, TraceError, TraceQuery,
    TraceRules,
    hulls::{ClipNode, HullModel},
    trace_policy,
};

fn load_geometry(
    planes: Vec<Plane>,
    nodes: Vec<ClipNode>,
    roots: Vec<i32>,
) -> (CollisionStore, GeometryId) {
    let mut store = CollisionStore::new();
    let models: Vec<_> = roots
        .into_iter()
        .map(|root| HullModel { roots: [root; 3] })
        .collect();
    let bounds = vec![
        Bounds {
            mins: Vec3([-1024.0; 3]),
            maxs: Vec3([1024.0; 3])
        };
        models.len()
    ];
    let id = store
        .load_hulls(planes, nodes.clone(), nodes, models, bounds)
        .expect("native geometry");
    (store, id)
}

fn path_cell(planes: Vec<Plane>, front: bool) -> (CollisionStore, GeometryId) {
    let nodes = (0..planes.len())
        .map(|index| {
            let next = if index + 1 == planes.len() {
                -2
            } else {
                index as i32 + 1
            };
            ClipNode {
                plane: index as u32,
                children: if front { [next, -1] } else { [-1, next] },
            }
        })
        .collect();
    load_geometry(planes, nodes, vec![0])
}

fn convex(planes: Vec<Plane>) -> (CollisionStore, GeometryId) {
    path_cell(planes, false)
}

fn box_at(position: Vec3, mins: Vec3, maxs: Vec3) -> TraceQuery<'static> {
    TraceQuery {
        mins,
        maxs,
        rules: TraceRules {
            hull_shape: HullShape::ExactBox,
            ..TraceRules::ARENA
        },
        ..TraceQuery::point(
            position,
            position,
            TraceRules::ARENA,
            trace_policy(RuleSetId::Quake3).1,
        )
    }
}

fn radius(position: Vec3, radius: f32) -> TraceQuery<'static> {
    box_at(position, Vec3([-radius; 3]), Vec3([radius; 3]))
}

fn admit(store: &mut CollisionStore, geometry: GeometryId) {
    store
        .prepare_hull_positions(geometry, HullPositionLimits::default())
        .expect("exact position admission");
}

#[test]
fn tetrahedron_rejects_shifted_face_false_positive_and_accepts_edge_touch() {
    let (mut store, geometry) = convex(vec![
        Plane::oriented(Vec3([-1.0, 0.0, 0.0]), 0.0),
        Plane::oriented(Vec3([0.0, -1.0, 0.0]), 0.0),
        Plane::oriented(Vec3([0.0, 0.0, -1.0]), 0.0),
        Plane::oriented(Vec3([1.0; 3]), 1.0),
    ]);
    admit(&mut store, geometry);
    let mut scratch = store.scratch();
    let outside = radius(Vec3([1.0, 0.6, -0.25]), 0.25);
    let clear = store
        .trace_model(geometry, 0, outside, &mut scratch)
        .expect("exact position");
    // Shifted original planes accept this point. The real box/tetrahedron
    // intersection would require x+y <= 1.5, but this center gives 1.6.
    assert!(!clear.start_solid && !clear.all_solid);
    assert_eq!(clear.fraction, 1.0);
    assert_eq!(clear.end, outside.end);
    let touch = radius(Vec3([1.0, 0.5, -0.25]), 0.25);
    let blocked = store
        .trace_model(geometry, 0, touch, &mut scratch)
        .expect("edge touch");
    assert!(blocked.start_solid && blocked.all_solid);
    assert_eq!(blocked.fraction, 0.0);
    assert_eq!(blocked.contents, Contents::SOLID);
}

#[test]
fn strict_empty_bsp_path_never_contributes_a_closed_phantom_cell() {
    let (mut store, geometry) = load_geometry(
        vec![Plane::oriented(Vec3([1.0, 0.0, 0.0]), 0.0)],
        vec![
            ClipNode {
                plane: 0,
                children: [1, -1],
            },
            ClipNode {
                plane: 0,
                children: [-1, -2],
            },
        ],
        vec![0],
    );
    admit(&mut store, geometry);
    let query = radius(Vec3::default(), 100.0);
    let trace = store
        .trace_model(geometry, 0, query, &mut store.scratch())
        .expect("empty path");
    assert!(!trace.start_solid && !trace.all_solid);
}

#[test]
fn repeated_scaled_path_planes_fit_bounded_admission_and_query_work() {
    let planes = (0..128)
        .map(|index| Plane::oriented(Vec3::axis(0, 2.0f32.powi(index % 16)), 0.0))
        .collect();
    let (mut store, geometry) = convex(planes);
    let native = TraceQuery::point(
        Vec3([2.0, 0.0, 0.0]),
        Vec3([-2.0, 0.0, 0.0]),
        TraceRules::LEGACY,
        trace_policy(RuleSetId::Quake).1,
    );
    let before = store
        .trace_model(geometry, 0, native, &mut store.scratch())
        .expect("native repeated path");
    store
        .prepare_hull_positions(
            geometry,
            HullPositionLimits {
                admission_work: 2_000,
                query_work: 10_000,
                ..Default::default()
            },
        )
        .expect("redundant path fits bounded admission");
    let after = store
        .trace_model(geometry, 0, native, &mut store.scratch())
        .expect("native after reduction");
    assert_eq!(before.fraction.to_bits(), after.fraction.to_bits());
    assert_eq!(
        before.end.0.map(f32::to_bits),
        after.end.0.map(f32::to_bits)
    );
    assert_eq!(
        before.plane.normal.0.map(f32::to_bits),
        after.plane.normal.0.map(f32::to_bits)
    );
    assert_eq!(
        before.plane.distance.to_bits(),
        after.plane.distance.to_bits()
    );
    let query = sweep(native.start, native.end);
    let trace = store
        .trace_model(geometry, 0, query, &mut store.scratch())
        .expect("persisted reduced path fits query work");
    assert_eq!(trace.fraction, 0.40625);
    assert_eq!(trace.plane.normal, Vec3([1.0, 0.0, 0.0]));
    assert_clear_endpoint(&store, geometry, query, &trace);
}

#[test]
fn positive_parallel_bounds_keep_the_tightest_negative_normal_source() {
    let (mut store, geometry) = convex(vec![
        Plane::oriented(Vec3([-2.0, 0.0, 0.0]), -8.0),
        Plane::oriented(Vec3([-1.0, 0.0, 0.0]), -6.0),
        Plane::oriented(Vec3([-4.0, 0.0, 0.0]), -16.0),
    ]);
    admit(&mut store, geometry);
    for (x, solid) in [(5.0, false), (6.0, true), (7.0, true)] {
        assert_eq!(
            store
                .trace_model(
                    geometry,
                    0,
                    radius(Vec3([x, 0.0, 0.0]), 0.0),
                    &mut store.scratch()
                )
                .expect("exact tightest bound")
                .all_solid,
            solid
        );
    }
    let query = sweep(Vec3::default(), Vec3([10.0, 0.0, 0.0]));
    let trace = store
        .trace_model(geometry, 0, query, &mut store.scratch())
        .expect("tightest source contact");
    assert_eq!(trace.fraction, 0.5625);
    assert_eq!(trace.plane.normal, Vec3([-1.0, 0.0, 0.0]));
    assert_eq!(trace.plane.distance, -6.0);
    assert_clear_endpoint(&store, geometry, query, &trace);
}

#[test]
fn equal_parallel_bounds_preserve_strict_ownership_but_loose_bounds_do_not() {
    for equal_strict in [false, true] {
        let mut planes = vec![
            Plane::oriented(Vec3([-2.0, 0.0, 0.0]), -2.0),
            Plane::oriented(Vec3([1.0, 0.0, 0.0]), 2.0),
        ];
        let mut nodes = vec![
            ClipNode {
                plane: 0,
                children: [1, -1],
            },
            ClipNode {
                plane: 1,
                children: [-1, 2],
            },
        ];
        if equal_strict {
            planes.push(Plane::oriented(Vec3([4.0, 0.0, 0.0]), 4.0));
            nodes.push(ClipNode {
                plane: 2,
                children: [-1, 3],
            });
        }
        planes.push(Plane::oriented(Vec3([1.0, 0.0, 0.0]), 1.0));
        nodes.push(ClipNode {
            plane: planes.len() as u32 - 1,
            children: [-2, -1],
        });
        let (mut store, geometry) = load_geometry(planes, nodes, vec![0]);
        admit(&mut store, geometry);
        let trace = store
            .trace_model(
                geometry,
                0,
                radius(Vec3([1.0, 0.0, 0.0]), 0.0),
                &mut store.scratch(),
            )
            .expect("mixed strict path");
        assert_eq!(trace.all_solid, !equal_strict);
    }
}

#[test]
fn repeated_opposing_constraints_preserve_closed_rank_deficient_occupancy() {
    let planes = (0..128)
        .map(|index| {
            let scale = 2.0f32.powi(index % 16);
            Plane::oriented(Vec3::axis(0, if index < 64 { scale } else { -scale }), 0.0)
        })
        .collect();
    let (mut store, geometry) = path_cell(planes, true);
    store
        .prepare_hull_positions(
            geometry,
            HullPositionLimits {
                admission_work: 2_000,
                query_work: 10_000,
                ..Default::default()
            },
        )
        .expect("opposing rows remain bounded");
    for (x, solid) in [(0.0, true), (-0.25, false), (0.25, false)] {
        assert_eq!(
            store
                .trace_model(
                    geometry,
                    0,
                    radius(Vec3([x, 0.0, 0.0]), 0.0),
                    &mut store.scratch()
                )
                .expect("opposing rank constraints")
                .all_solid,
            solid
        );
    }
}

#[test]
fn closed_rank_deficient_path_and_zero_extent_box_are_supported() {
    let (mut store, geometry) = load_geometry(
        vec![
            Plane::oriented(Vec3([1.0, 0.0, 0.0]), 0.0),
            Plane::oriented(Vec3([-1.0, 0.0, 0.0]), 0.0),
        ],
        vec![
            ClipNode {
                plane: 0,
                children: [1, -1],
            },
            ClipNode {
                plane: 1,
                children: [-2, -1],
            },
        ],
        vec![0],
    );
    admit(&mut store, geometry);
    let mut scratch = store.scratch();
    for (position, blocked) in [(Vec3::default(), true), (Vec3([1.0, 0.0, 0.0]), false)] {
        let trace = store
            .trace_model(geometry, 0, radius(position, 0.0), &mut scratch)
            .expect("rank two occupancy");
        assert_eq!(trace.all_solid, blocked);
    }
}

#[test]
fn unbounded_and_fullspace_cells_do_not_use_model_bounds_as_caps() {
    let (mut halfspace, geometry) = convex(vec![Plane::oriented(Vec3([1.0, 0.0, 0.0]), 0.0)]);
    admit(&mut halfspace, geometry);
    let far = Vec3([-f32::MAX, f32::MAX, f32::MAX]);
    assert!(
        halfspace
            .trace_model(geometry, 0, radius(far, 0.0), &mut halfspace.scratch())
            .expect("unbounded position")
            .all_solid
    );
    let (mut fullspace, geometry) = load_geometry(Vec::new(), Vec::new(), vec![-2]);
    admit(&mut fullspace, geometry);
    assert!(
        fullspace
            .trace_model(geometry, 0, radius(far, f32::MAX), &mut fullspace.scratch())
            .expect("fullspace position")
            .all_solid
    );
}

#[test]
fn supplied_height_changes_stationary_occupancy_instead_of_compiled_width() {
    let (mut store, geometry) = convex(vec![Plane::oriented(Vec3([0.0, 0.0, -1.0]), -10.0)]);
    admit(&mut store, geometry);
    let mut scratch = store.scratch();
    let crouched = box_at(
        Vec3::default(),
        Vec3([-15.0, -15.0, -24.0]),
        Vec3([15.0, 15.0, 4.0]),
    );
    assert!(
        !store
            .trace_model(geometry, 0, crouched, &mut scratch)
            .expect("crouched exact position")
            .start_solid
    );
    let standing = TraceQuery {
        maxs: Vec3([15.0, 15.0, 32.0]),
        ..crouched
    };
    assert!(
        store
            .trace_model(geometry, 0, standing, &mut scratch)
            .expect("standing exact position")
            .start_solid
    );
}

#[test]
fn rotated_box_intersection_uses_original_world_axes_not_local_aabb() {
    let (mut store, geometry) = convex(vec![
        Plane::oriented(Vec3([-1.0, 0.0, 0.0]), 0.0),
        Plane::oriented(Vec3([0.0, -1.0, 0.0]), 0.0),
    ]);
    admit(&mut store, geometry);
    let mut scratch = store.scratch();
    let rules = ModelRules {
        rotation: ModelRotation::TransposeBasis,
        ..Default::default()
    };
    let origin = Vec3([10.0, 20.0, 0.0]);
    let angles = Vec3([0.0, 45.0, 0.0]);
    let clear = radius(Vec3([10.0, 19.6, 0.0]), 0.25);
    // The transformed box's local AABB reaches both positive local axes,
    // but no actual box point reaches the rotated quadrant y>=|x|.
    assert!(
        !store
            .trace_transformed(geometry, 0, clear, origin, angles, rules, &mut scratch)
            .expect("posed exact position")
            .all_solid
    );
    let touch = radius(Vec3([10.0, 19.75, 0.0]), 0.25);
    assert!(
        store
            .trace_transformed(geometry, 0, touch, origin, angles, rules, &mut scratch)
            .expect("posed contact")
            .all_solid
    );
}

#[test]
fn asymmetric_world_box_is_not_rounded_by_native_centering() {
    let (mut store, geometry) = convex(vec![Plane::oriented(Vec3([-1.0, 0.0, 0.0]), -1.0)]);
    admit(&mut store, geometry);
    let position = Vec3([16_777_216.0, 0.0, 0.0]);
    let query = box_at(position, Vec3::default(), Vec3([1.0, 0.0, 0.0]));
    let trace = store
        .trace_transformed(
            geometry,
            0,
            query,
            position,
            Vec3::default(),
            ModelRules {
                rotation: ModelRotation::TransposeBasis,
                ..Default::default()
            },
            &mut store.scratch(),
        )
        .expect("unrounded world bounds");
    assert!(trace.all_solid);
    assert_eq!(trace.end.0.map(f32::to_bits), position.0.map(f32::to_bits));
}

#[test]
fn source_contents_masks_are_selected_at_query_time() {
    let (mut store, geometry) = load_geometry(Vec::new(), Vec::new(), vec![-3]);
    admit(&mut store, geometry);
    let mut scratch = store.scratch();
    let query = radius(Vec3::default(), 1.0);
    assert!(
        !store
            .trace_model(geometry, 0, query, &mut scratch)
            .expect("solid mask")
            .all_solid
    );
    let water = store
        .trace_model(
            geometry,
            0,
            TraceQuery {
                mask: Contents::WATER,
                ..query
            },
            &mut scratch,
        )
        .expect("water mask");
    assert!(water.all_solid);
    assert_eq!(water.contents, Contents::WATER);
}

#[test]
fn admission_rejection_keeps_native_queries_and_publishes_no_partial_models() {
    let (mut store, geometry) = load_geometry(
        vec![Plane::oriented(Vec3([1.0, 0.0, 0.0]), 0.0)],
        vec![ClipNode {
            plane: 0,
            children: [-1, -2],
        }],
        vec![0, 0],
    );
    let native = TraceQuery::point(
        Vec3([1.0, 0.0, 0.0]),
        Vec3([-1.0, 0.0, 0.0]),
        TraceRules::LEGACY,
        trace_policy(RuleSetId::Quake).1,
    );
    let before = store
        .trace_model(geometry, 0, native, &mut store.scratch())
        .expect("native before admission");
    for limits in [
        HullPositionLimits {
            cells: 1,
            ..Default::default()
        },
        HullPositionLimits {
            bytes: 0,
            ..Default::default()
        },
        HullPositionLimits {
            admission_work: 0,
            ..Default::default()
        },
        // Counting and the first model's feasibility finish before the
        // second model exhausts this budget. The first model must stay private.
        HullPositionLimits {
            admission_work: 15,
            ..Default::default()
        },
    ] {
        assert_eq!(
            store.prepare_hull_positions(geometry, limits),
            Err(StoreError::Capacity)
        );
        for model in 0..2 {
            assert_eq!(
                store
                    .trace_model(
                        geometry,
                        model,
                        radius(Vec3::default(), 1.0),
                        &mut store.scratch()
                    )
                    .unwrap_err(),
                TraceError::Unsupported { geometry, model }
            );
        }
        let after = store
            .trace_model(geometry, 0, native, &mut store.scratch())
            .expect("native after rejected admission");
        assert_eq!(after.fraction.to_bits(), before.fraction.to_bits());
        assert_eq!(
            after.end.0.map(f32::to_bits),
            before.end.0.map(f32::to_bits)
        );
        assert_eq!(
            after.plane.normal.0.map(f32::to_bits),
            before.plane.normal.0.map(f32::to_bits)
        );
    }
    admit(&mut store, geometry);
    for model in 0..2 {
        assert!(
            store
                .trace_model(
                    geometry,
                    model,
                    radius(Vec3::default(), 1.0),
                    &mut store.scratch()
                )
                .expect("atomic successful admission")
                .all_solid
        );
    }
}

#[test]
fn insufficient_old_scratch_and_degenerate_moving_queries_are_errors() {
    let (mut store, geometry) = convex(vec![Plane::oriented(Vec3([1.0, 0.0, 0.0]), 0.0)]);
    let mut old = store.scratch();
    let query = radius(Vec3::default(), 1.0);
    assert_eq!(
        store.trace_model(geometry, 0, query, &mut old).unwrap_err(),
        TraceError::Unsupported { geometry, model: 0 }
    );
    admit(&mut store, geometry);
    assert_eq!(
        store.trace_model(geometry, 0, query, &mut old).unwrap_err(),
        TraceError::Scratch
    );
    let moving = TraceQuery {
        end: Vec3([10.0, 0.0, 0.0]),
        mins: Vec3::default(),
        maxs: Vec3::default(),
        ..query
    };
    let mut scratch = store.scratch();
    assert_eq!(
        store
            .trace_model(geometry, 0, moving, &mut scratch)
            .unwrap_err(),
        TraceError::Unsupported { geometry, model: 0 }
    );
    assert_eq!(
        store
            .trace_transformed(
                geometry,
                0,
                moving,
                Vec3::default(),
                Vec3::default(),
                ModelRules {
                    rotation: ModelRotation::TransposeBasis,
                    ..Default::default()
                },
                &mut scratch
            )
            .unwrap_err(),
        TraceError::Unsupported { geometry, model: 0 }
    );
    assert!(
        store
            .trace_model(geometry, 0, query, &mut scratch)
            .expect("stationary after rejected queries")
            .all_solid
    );
}

#[test]
fn query_budget_and_nonfinite_bounds_never_report_clear() {
    let (mut store, geometry) = convex(vec![Plane::oriented(Vec3([1.0, 0.0, 0.0]), 0.0)]);
    store
        .prepare_hull_positions(
            geometry,
            HullPositionLimits {
                query_work: 1,
                ..Default::default()
            },
        )
        .expect("bounded admission");
    let query = radius(Vec3([1.0, 0.0, 0.0]), 0.1);
    assert_eq!(
        store
            .trace_model(geometry, 0, query, &mut store.scratch())
            .unwrap_err(),
        TraceError::Unsupported { geometry, model: 0 }
    );
    let invalid = TraceQuery {
        maxs: Vec3([f32::INFINITY, 1.0, 1.0]),
        ..query
    };
    assert_eq!(
        store
            .trace_model(geometry, 0, invalid, &mut store.scratch())
            .unwrap_err(),
        TraceError::Unsupported { geometry, model: 0 }
    );
}

#[test]
fn extreme_finite_coefficients_and_world_box_sums_remain_exact() {
    let planes = (0..3)
        .flat_map(|axis| {
            [-f32::MAX, f32::MAX].map(|normal| Plane::oriented(Vec3::axis(axis, normal), f32::MAX))
        })
        .collect();
    let (mut store, geometry) = convex(planes);
    admit(&mut store, geometry);
    let origin = Vec3([f32::MAX; 3]);
    let query = radius(origin, f32::MAX);
    // The six original bounds are finite, but their exact world upper sums
    // exceed f32::MAX. The model's cube still lies within this supplied box.
    let trace = store
        .trace_transformed(
            geometry,
            0,
            query,
            origin,
            Vec3::default(),
            ModelRules::default(),
            &mut store.scratch(),
        )
        .expect("finite exact predicate coefficients");
    assert!(trace.start_solid && trace.all_solid);
    assert_eq!(trace.end.0.map(f32::to_bits), origin.0.map(f32::to_bits));
}

#[test]
fn all_existing_policy_presets_retain_compiled_width_compatibility() {
    for rules in [
        RuleSetId::Quake,
        RuleSetId::QuakeWorld,
        RuleSetId::Quake2,
        RuleSetId::Quake2Rerelease,
        RuleSetId::Quake3,
    ] {
        assert_eq!(trace_policy(rules).0.hull_shape, HullShape::CompiledWidth);
    }
}

fn sweep(start: Vec3, end: Vec3) -> TraceQuery<'static> {
    TraceQuery {
        end,
        ..radius(start, 0.25)
    }
}

fn assert_clear_endpoint(
    store: &CollisionStore,
    geometry: GeometryId,
    query: TraceQuery,
    trace: &qa_world::collision::Trace,
) {
    assert!(trace.fraction < 1.0 && !trace.all_solid);
    assert_eq!(
        trace.end.0.map(f32::to_bits),
        query
            .start
            .lerp(query.end, trace.fraction)
            .0
            .map(f32::to_bits)
    );
    let position = TraceQuery {
        start: trace.end,
        end: trace.end,
        ..query
    };
    assert!(
        !store
            .trace_model(geometry, 0, position, &mut store.scratch())
            .expect("published endpoint occupancy")
            .start_solid
    );
}

#[test]
fn moving_halfspace_has_native_epsilon_and_unshifted_contact_plane() {
    let (mut store, geometry) = convex(vec![Plane::oriented(Vec3([1.0, 0.0, 0.0]), 0.0)]);
    admit(&mut store, geometry);
    let query = sweep(Vec3([2.0, 0.0, 0.0]), Vec3([-2.0, 0.0, 0.0]));
    let trace = store
        .trace_model(geometry, 0, query, &mut store.scratch())
        .expect("moving halfspace");
    assert_eq!(trace.fraction, 0.40625);
    assert_eq!(trace.end, Vec3([0.375, 0.0, 0.0]));
    assert_eq!(trace.plane.normal, Vec3([1.0, 0.0, 0.0]));
    assert_eq!(trace.plane.distance, 0.0);
    assert_clear_endpoint(&store, geometry, query, &trace);
}

#[test]
fn moving_tetrahedron_uses_the_box_edge_bevel() {
    let (mut store, geometry) = convex(vec![
        Plane::oriented(Vec3([-1.0, 0.0, 0.0]), 0.0),
        Plane::oriented(Vec3([0.0, -1.0, 0.0]), 0.0),
        Plane::oriented(Vec3([0.0, 0.0, -1.0]), 0.0),
        Plane::oriented(Vec3([1.0; 3]), 1.0),
    ]);
    admit(&mut store, geometry);
    let mut query = sweep(Vec3([1.0, 0.6, -0.25]), Vec3([1.0, 0.4, -0.25]));
    query.rules.contact_epsilon = 0.03125;
    let trace = store
        .trace_model(geometry, 0, query, &mut store.scratch())
        .expect("complete box bevel");
    assert!(trace.fraction > 0.0 && trace.fraction < 0.5);
    assert!(trace.plane.normal.0[0] > 0.7 && trace.plane.normal.0[1] > 0.7);
    assert_eq!(trace.plane.normal.0[2], 0.0);
    assert_clear_endpoint(&store, geometry, query, &trace);
}

#[test]
fn moving_union_does_not_publish_an_internal_partition_normal() {
    let (mut store, geometry) = load_geometry(
        vec![
            Plane::oriented(Vec3([1.0, 0.0, 0.0]), 0.0),
            Plane::oriented(Vec3([0.0, 1.0, 0.0]), 0.0),
        ],
        vec![
            ClipNode {
                plane: 0,
                children: [1, 1],
            },
            ClipNode {
                plane: 1,
                children: [-1, -2],
            },
        ],
        vec![0],
    );
    admit(&mut store, geometry);
    let query = sweep(Vec3([2.0, 2.0, 0.0]), Vec3([-2.0, -2.0, 0.0]));
    let trace = store
        .trace_model(geometry, 0, query, &mut store.scratch())
        .expect("union exterior");
    assert_eq!(trace.plane.normal, Vec3([0.0, 1.0, 0.0]));
    assert_clear_endpoint(&store, geometry, query, &trace);
}

#[test]
fn concave_union_exterior_uses_a_facet_tangent_witness() {
    let (mut store, geometry) = load_geometry(
        vec![
            Plane::oriented(Vec3([1.0, 0.0, 0.0]), 0.0),
            Plane::oriented(Vec3([0.0, 1.0, 0.0]), 0.0),
        ],
        vec![
            ClipNode {
                plane: 0,
                children: [1, -2],
            },
            ClipNode {
                plane: 1,
                children: [-1, -2],
            },
        ],
        vec![0],
    );
    admit(&mut store, geometry);
    let query = sweep(Vec3([2.0, 2.0, 0.0]), Vec3([-2.0, -2.0, 0.0]));
    let trace = store
        .trace_model(geometry, 0, query, &mut store.scratch())
        .expect("concave exterior");
    assert!([Vec3([1.0, 0.0, 0.0]), Vec3([0.0, 1.0, 0.0])].contains(&trace.plane.normal));
    assert_clear_endpoint(&store, geometry, query, &trace);
}

#[test]
fn redundant_vertex_plane_is_not_reported_as_a_surface() {
    let mut planes = vec![Plane::oriented(Vec3([1.0; 3]), 3.0)];
    for axis in 0..3 {
        planes.push(Plane::oriented(Vec3::axis(axis, 1.0), 1.0));
        planes.push(Plane::oriented(Vec3::axis(axis, -1.0), 1.0));
    }
    let (mut store, geometry) = convex(planes);
    store
        .prepare_hull_positions(
            geometry,
            HullPositionLimits {
                query_work: 16_777_216,
                ..Default::default()
            },
        )
        .expect("cube admission");
    let query = sweep(Vec3([2.0; 3]), Vec3::default());
    let trace = store
        .trace_model(geometry, 0, query, &mut store.scratch())
        .expect("facet instead of redundant vertex normal");
    assert_eq!(
        trace
            .plane
            .normal
            .0
            .iter()
            .filter(|&&value| value != 0.0)
            .count(),
        1
    );
    assert!(trace.plane.normal.0.contains(&1.0));
    assert_clear_endpoint(&store, geometry, query, &trace);
}

#[test]
fn posed_asymmetric_sweep_retains_world_box_axes() {
    let (mut store, geometry) = convex(vec![
        Plane::oriented(Vec3([-1.0, 0.0, 0.0]), 0.0),
        Plane::oriented(Vec3([0.0, -1.0, 0.0]), 0.0),
    ]);
    admit(&mut store, geometry);
    let query = TraceQuery {
        end: Vec3([10.0, 20.3, 0.0]),
        mins: Vec3([-0.125, -0.25, -0.5]),
        maxs: Vec3([0.375, 0.25, 0.5]),
        rules: TraceRules {
            contact_epsilon: 0.03125,
            hull_shape: HullShape::ExactBox,
            ..TraceRules::ARENA
        },
        ..radius(Vec3([10.0, 19.6, 0.0]), 0.25)
    };
    let origin = Vec3([10.0, 20.0, 0.0]);
    let angles = Vec3([0.0, 45.0, 0.0]);
    let rules = ModelRules {
        rotation: ModelRotation::TransposeBasis,
        ..Default::default()
    };
    let mut scratch = store.scratch();
    let trace = store
        .trace_transformed(geometry, 0, query, origin, angles, rules, &mut scratch)
        .expect("posed moving original bounds");
    assert!(trace.fraction > 0.0 && trace.end.0[1] < 19.75);
    assert_eq!(trace.plane.normal, Vec3([0.0, -1.0, 0.0]));
    let endpoint = TraceQuery {
        start: trace.end,
        end: trace.end,
        ..query
    };
    assert!(
        !store
            .trace_transformed(geometry, 0, endpoint, origin, angles, rules, &mut scratch)
            .expect("posed endpoint certification")
            .start_solid
    );
}

#[test]
fn initially_solid_adjacent_cells_cover_the_whole_sweep() {
    let (mut store, geometry) = load_geometry(
        vec![
            Plane::oriented(Vec3([1.0, 0.0, 0.0]), 0.0),
            Plane::oriented(Vec3([1.0, 0.0, 0.0]), 2.0),
            Plane::oriented(Vec3([-1.0, 0.0, 0.0]), 2.0),
        ],
        vec![
            ClipNode {
                plane: 0,
                children: [1, 2],
            },
            ClipNode {
                plane: 1,
                children: [-1, -2],
            },
            ClipNode {
                plane: 2,
                children: [-1, -2],
            },
        ],
        vec![0],
    );
    admit(&mut store, geometry);
    let query = sweep(Vec3([-1.0, 0.0, 0.0]), Vec3([1.0, 0.0, 0.0]));
    let trace = store
        .trace_model(geometry, 0, query, &mut store.scratch())
        .expect("connected initial union");
    assert!(trace.start_solid && trace.all_solid);
    assert_eq!(trace.fraction, 0.0);
    assert_eq!(trace.end, query.start);
}

fn disjoint_halfspaces(left: f32, right: f32) -> (CollisionStore, GeometryId) {
    load_geometry(
        vec![
            Plane::oriented(Vec3([1.0, 0.0, 0.0]), left),
            Plane::oriented(Vec3([-1.0, 0.0, 0.0]), -right),
        ],
        vec![
            ClipNode {
                plane: 0,
                children: [1, -2],
            },
            ClipNode {
                plane: 1,
                children: [-1, -2],
            },
        ],
        vec![0],
    )
}

#[test]
fn initial_solid_exit_and_reentry_stops_in_the_real_gap() {
    let (mut store, geometry) = disjoint_halfspaces(0.0, 2.0);
    admit(&mut store, geometry);
    let query = sweep(Vec3([-1.0, 0.0, 0.0]), Vec3([3.0, 0.0, 0.0]));
    let trace = store
        .trace_model(geometry, 0, query, &mut store.scratch())
        .expect("union reentry");
    assert!(trace.start_solid && !trace.all_solid);
    assert_eq!(trace.plane.normal, Vec3([-1.0, 0.0, 0.0]));
    assert!(trace.end.0[0] > 0.25 && trace.end.0[0] < 1.75);
    assert_clear_endpoint(&store, geometry, query, &trace);
}

#[test]
fn no_representable_clear_endpoint_returns_an_error_instead_of_false_clear() {
    let (mut store, geometry) = disjoint_halfspaces(16_777_216.0, 16_777_218.0);
    store
        .prepare_hull_positions(
            geometry,
            HullPositionLimits {
                query_work: 50_000,
                ..Default::default()
            },
        )
        .expect("bounded large-coordinate admission");
    // The mathematical gap for a radius 1/4 center is
    // (16777216.25, 16777217.75). Adjacent f32 values are 16777216/18;
    // neither can represent a clear center, whatever parameter is returned.
    for x in [16_777_216.0, 16_777_218.0] {
        assert!(
            store
                .trace_model(
                    geometry,
                    0,
                    radius(Vec3([x, 0.0, 0.0]), 0.25),
                    &mut store.scratch()
                )
                .expect("adjacent float occupancy")
                .start_solid
        );
    }
    let query = sweep(
        Vec3([16_777_216.0, 0.0, 0.0]),
        Vec3([16_777_220.0, 0.0, 0.0]),
    );
    assert_eq!(
        store
            .trace_model(geometry, 0, query, &mut store.scratch())
            .unwrap_err(),
        TraceError::Unsupported { geometry, model: 0 }
    );
}

#[test]
fn bounded_contact_overflow_never_publishes_a_clear_trace() {
    let (mut store, geometry) = convex(vec![Plane::oriented(Vec3([1.0, 0.0, 0.0]), 0.0)]);
    store
        .prepare_hull_positions(
            geometry,
            HullPositionLimits {
                contact_rows: 0,
                ..Default::default()
            },
        )
        .expect("zero contact capacity admission");
    let query = sweep(Vec3([2.0, 0.0, 0.0]), Vec3([-2.0, 0.0, 0.0]));
    assert_eq!(
        store
            .trace_model(geometry, 0, query, &mut store.scratch())
            .unwrap_err(),
        TraceError::Unsupported { geometry, model: 0 }
    );
}
