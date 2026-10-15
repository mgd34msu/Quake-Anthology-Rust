use qa_core::primitives::{
    Body, Bounds, Capsule, CollisionOwner, CollisionShape, EntityId, ModelRotation, ModelRules,
    ModuleId, Plane, RotatedLinkBounds, RuleSetId, SurfaceFlags, ThinkTime, Vec3,
};
use qa_world::{
    area::{AreaGrid, LinkFlags, LinkIntent, LinkOrder},
    collision::{
        CollisionStore, Contents, Trace, TraceError, TraceQuery, TraceScratch, WorldTrace,
        brushes::{Brush, BrushTree},
        store::BodyTraceMode,
        surfaces::SurfaceTable,
        trace_policy,
    },
    entities::{AllocationPolicy, EntityTable},
};

const TARGET: EntityId = EntityId {
    slot: 7,
    generation: 4,
};

fn target() -> Body {
    Body {
        mins: Vec3([-1.0, -1.0, -11.0]),
        maxs: Vec3([1.0, 1.0, 11.0]),
        ..Default::default()
    }
}
fn moving(start: Vec3, end: Vec3, offset: Vec3) -> TraceQuery<'static> {
    capsule_query(start, end, 1.0, offset)
}
fn capsule_query(start: Vec3, end: Vec3, radius: f32, offset: Vec3) -> TraceQuery<'static> {
    let (rules, entity_rules) = trace_policy(RuleSetId::Quake3);
    let extent = Vec3(offset.0.map(|v| v.abs() + radius));
    TraceQuery {
        mask: Contents::BODY,
        mins: -extent,
        maxs: extent,
        capsule: Some(Capsule { radius, offset }),
        ..TraceQuery::point(start, end, rules, entity_rules)
    }
}
fn trace(query: TraceQuery<'_>, body: Body) -> Trace {
    let store = CollisionStore::new();
    run(&store, &mut store.scratch(), query, body).unwrap()
}
fn run(
    store: &CollisionStore,
    scratch: &mut TraceScratch,
    query: TraceQuery<'_>,
    body: Body,
) -> Result<Trace, TraceError> {
    store.trace_body(
        query,
        CollisionShape::Capsule,
        &body,
        Vec3::default(),
        Default::default(),
        BodyTraceMode::Direct,
        Some(TARGET),
        scratch,
    )
}
fn near(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() < 2.0e-5,
        "{actual:?} != {expected:?}"
    );
}
fn same_trace(actual: Trace, expected: Trace) {
    assert_eq!(actual.fraction.to_bits(), expected.fraction.to_bits());
    assert_eq!(
        actual.end.0.map(f32::to_bits),
        expected.end.0.map(f32::to_bits)
    );
    assert_eq!(
        actual.plane.normal.0.map(f32::to_bits),
        expected.plane.normal.0.map(f32::to_bits)
    );
    assert_eq!(
        actual.plane.distance.to_bits(),
        expected.plane.distance.to_bits()
    );
    assert_eq!(actual.plane.encoding, expected.plane.encoding);
    assert_eq!(
        (
            actual.start_solid,
            actual.all_solid,
            actual.in_open,
            actual.in_water,
            actual.contents,
            actual.entity,
            actual.surface,
            actual.surface_id,
            actual.secondary_plane,
            actual.secondary_surface_id,
            actual.brush_solid
        ),
        (
            expected.start_solid,
            expected.all_solid,
            expected.in_open,
            expected.in_water,
            expected.contents,
            expected.entity,
            expected.surface,
            expected.surface_id,
            expected.secondary_plane,
            expected.secondary_surface_id,
            expected.brush_solid
        )
    );
}
fn clear_endpoint(query: TraceQuery<'_>, body: Body, hit: Trace) {
    let endpoint = trace(
        TraceQuery {
            start: hit.end,
            end: hit.end,
            ..query
        },
        body,
    );
    assert!(
        !endpoint.start_solid && !endpoint.all_solid,
        "{hit:?} -> {endpoint:?}"
    );
}
#[test]
fn crossing_segments_overlap_without_any_endpoint_pair_overlap() {
    let query = moving(Vec3::default(), Vec3::default(), Vec3([10.0, 0.0, 0.0]));
    let hit = trace(query, target());
    assert!(hit.start_solid && hit.all_solid);
    assert_eq!(hit.fraction, 0.0);
    assert_eq!(
        hit.entity,
        Some(EntityId {
            slot: 7,
            generation: 4
        })
    );
    assert!(!hit.brush_solid);
}
#[test]
fn tilted_capsule_vertical_sweep_contacts_the_target_endpoint() {
    let query = moving(
        Vec3([0.0, 0.0, -20.0]),
        Vec3([0.0, 0.0, 20.0]),
        Vec3([10.0, 0.0, 0.0]),
    );
    let hit = trace(query, target());
    assert!(hit.fraction > 0.0 && hit.fraction < 0.2, "{hit:?}");
    assert!(hit.end.0[2] < -12.0 && hit.end.0[2] > -13.0, "{hit:?}");
    near(hit.fraction, 0.196875);
    near(hit.end.0[2], -12.125);
    assert_eq!(hit.plane.normal, Vec3([0.0, 0.0, -1.0]));
    near(hit.plane.distance, 11.0);
    assert_eq!(hit.contents, Contents::BODY);
    assert!(!hit.start_solid && !hit.all_solid);
    let at_end = trace(
        TraceQuery {
            start: hit.end,
            end: hit.end,
            ..query
        },
        target(),
    );
    assert!(
        !at_end.start_solid,
        "published exterior endpoint must be clear"
    );
    let reversed = trace(
        TraceQuery {
            capsule: Some(Capsule {
                radius: 1.0,
                offset: Vec3([-10.0, 0.0, 0.0]),
            }),
            ..query
        },
        target(),
    );
    assert_eq!(
        (
            hit.fraction.to_bits(),
            hit.end,
            hit.plane,
            hit.start_solid,
            hit.all_solid,
            hit.contents,
            hit.entity,
            hit.brush_solid
        ),
        (
            reversed.fraction.to_bits(),
            reversed.end,
            reversed.plane,
            reversed.start_solid,
            reversed.all_solid,
            reversed.contents,
            reversed.entity,
            reversed.brush_solid
        )
    );
}
#[test]
fn overlapping_capsule_aabbs_do_not_replace_segment_distance() {
    let query = moving(
        Vec3([10.0, -10.0, 0.0]),
        Vec3([10.0, -10.0, 0.0]),
        Vec3([10.0, 10.0, 0.0]),
    );
    let hit = trace(query, target());
    assert!(!hit.start_solid && !hit.all_solid);
    assert_eq!(hit.fraction, 1.0);
}
#[test]
fn tilted_capsule_retains_the_shared_inside_component_contract() {
    let offset = Vec3([10.0, 0.0, 0.0]);
    let inside = trace(
        moving(Vec3::default(), Vec3([0.0, 0.0, 1.0]), offset),
        target(),
    );
    assert!(inside.start_solid && inside.all_solid);
    assert_eq!(inside.fraction, 0.0);
    let exiting = trace(
        moving(Vec3::default(), Vec3([0.0, 0.0, 20.0]), offset),
        target(),
    );
    assert!(exiting.start_solid && !exiting.all_solid);
    assert_eq!(exiting.fraction, 1.0);
    assert_eq!(exiting.end, Vec3([0.0, 0.0, 20.0]));
}

#[test]
fn skew_side_side_and_side_end_contacts_use_the_actual_segments() {
    // The interior of the (8,0,6) segment meets the target's interior.
    // Separation is solely along Y, so the padded center stops at 2+.125.
    let side_side = moving(
        Vec3([4.0, 6.0, 0.0]),
        Vec3([4.0, -6.0, 0.0]),
        Vec3([8.0, 0.0, 6.0]),
    );
    let side_end = moving(
        Vec3([5.0, 0.0, 20.0]),
        Vec3([5.0, 0.0, 0.0]),
        Vec3([10.0, 0.0, 0.0]),
    );
    for (query, expected_end, normal, distance) in [
        (
            side_side,
            Vec3([4.0, 2.125, 0.0]),
            Vec3([0.0, 1.0, 0.0]),
            1.0,
        ),
        (
            side_end,
            Vec3([5.0, 0.0, 12.125]),
            Vec3([0.0, 0.0, 1.0]),
            11.0,
        ),
    ] {
        let hit = trace(query, target());
        assert!(!hit.start_solid && !hit.all_solid);
        for axis in 0..3 {
            near(hit.end.0[axis], expected_end.0[axis]);
            near(hit.plane.normal.0[axis], normal.0[axis]);
        }
        near(hit.plane.distance, distance);
        assert_eq!(hit.contents, Contents::BODY);
        assert_eq!(hit.entity, Some(TARGET));
        clear_endpoint(query, target(), hit);
        let reversed = TraceQuery {
            capsule: query.capsule.map(|capsule| Capsule {
                offset: -capsule.offset,
                ..capsule
            }),
            ..query
        };
        same_trace(trace(reversed, target()), hit);
    }
}

#[test]
fn skew_end_end_contact_retains_its_round_normal_instead_of_aabb_faces() {
    // Moving endpoint (x,y,z)-(4,3,0) meets the target endpoint (0,0,10).
    // At the padded radius 2.125 their delta is (1.5,1.125,1), a 12:9:8 triangle.
    let query = moving(
        Vec3([12.0, 9.0, 11.0]),
        Vec3([0.0, 0.0, 11.0]),
        Vec3([4.0, 3.0, 0.0]),
    );
    let hit = trace(query, target());
    near(hit.fraction, 13.0 / 24.0);
    for (actual, expected) in hit.end.0.into_iter().zip([5.5, 4.125, 11.0]) {
        near(actual, expected);
    }
    for (actual, expected) in
        hit.plane
            .normal
            .0
            .into_iter()
            .zip([12.0 / 17.0, 9.0 / 17.0, 8.0 / 17.0])
    {
        near(actual, expected);
    }
    near(hit.plane.distance, 97.0 / 17.0);
    assert_eq!(hit.contents, Contents::BODY);
    assert!(!hit.start_solid && !hit.all_solid);
    clear_endpoint(query, target(), hit);
    let reversed = TraceQuery {
        capsule: Some(Capsule {
            radius: 1.0,
            offset: Vec3([-4.0, -3.0, 0.0]),
        }),
        ..query
    };
    same_trace(trace(reversed, target()), hit);
}

#[test]
fn nearly_parallel_segments_keep_the_common_margin_without_a_vertical_tolerance() {
    let query = moving(
        Vec3([6.0, 0.0, 0.0]),
        Vec3([-6.0, 0.0, 0.0]),
        Vec3([1.0 / 1_048_576.0, 0.0, 10.0]),
    );
    let hit = trace(query, target());
    near(hit.end.0[0], 2.125);
    near(hit.plane.normal.0[0], 1.0);
    assert!(!hit.start_solid && !hit.all_solid);
    clear_endpoint(query, target(), hit);
    let reversed = TraceQuery {
        capsule: query.capsule.map(|capsule| Capsule {
            offset: -capsule.offset,
            ..capsule
        }),
        ..query
    };
    same_trace(trace(reversed, target()), hit);
    for (x, inside) in [(1.75, true), (2.25, false)] {
        let point = Vec3([x, 0.0, 0.0]);
        let result = trace(
            TraceQuery {
                start: point,
                end: point,
                ..query
            },
            target(),
        );
        assert_eq!((result.start_solid, result.all_solid), (inside, inside));
    }
}

#[test]
fn target_spheres_and_zero_radii_keep_closed_tangency_separate_from_clear_space() {
    for (moving_radius, target_radius, segment) in [
        (1.0, 1.0, 0.0),
        (0.0, 1.0, 10.0),
        (1.0, 0.0, 10.0),
        (0.0, 0.0, 10.0),
    ] {
        let body = Body {
            mins: Vec3([-target_radius, -target_radius, -target_radius - segment]),
            maxs: Vec3([target_radius, target_radius, target_radius + segment]),
            ..Body::default()
        };
        let tangent = moving_radius + target_radius;
        for (y, inside) in [(tangent, true), (tangent + 0.001, false)] {
            let point = Vec3([0.0, y, 0.0]);
            let result = trace(
                capsule_query(point, point, moving_radius, Vec3([10.0, 0.0, 0.0])),
                body,
            );
            assert_eq!(
                (result.start_solid, result.all_solid),
                (inside, inside),
                "radii={moving_radius}/{target_radius} segment={segment} y={y} {result:?}"
            );
            assert_eq!(result.fraction, if inside { 0.0 } else { 1.0 });
            if inside {
                assert_eq!(result.contents, Contents::BODY);
                assert_eq!(result.entity, Some(TARGET));
            }
        }
    }
    let sphere = Body {
        mins: Vec3([-1.0; 3]),
        maxs: Vec3([1.0; 3]),
        ..Body::default()
    };
    let query = capsule_query(
        Vec3([0.0, 4.0, 0.0]),
        Vec3([0.0, -4.0, 0.0]),
        0.0,
        Vec3([10.0, 0.0, 0.0]),
    );
    let hit = trace(query, sphere);
    near(hit.end.0[1], 1.125);
    near(hit.plane.normal.0[1], 1.0);
    near(hit.plane.distance, 1.0);
    clear_endpoint(query, sphere, hit);
}

fn posed_case() -> (TraceQuery<'static>, Body, Vec3, ModelRules) {
    let body = Body {
        position: Vec3([10.0, 20.0, 30.0]),
        ..target()
    };
    let query = moving(
        body.position + Vec3([0.0, 0.0, -20.0]),
        body.position + Vec3([0.0, 0.0, 20.0]),
        Vec3([0.0, 0.0, 10.0]),
    );
    let rules = ModelRules {
        rotation: ModelRotation::TransposeBasis,
        link_bounds: RotatedLinkBounds::MaxAbsCube,
    };
    (query, body, Vec3([90.0, 0.0, 0.0]), rules)
}

#[test]
fn transformed_world_vertical_query_matches_its_tilted_local_query_and_restores_contact() {
    use qa_core::math::{angle_vectors_radians, inverse_transform_point, radians_from_degrees_f32};
    let store = CollisionStore::new();
    let mut scratch = store.scratch();
    let (query, body, angles, rules) = posed_case();
    let axes = angle_vectors_radians(radians_from_degrees_f32(angles)).axes();
    let local_query = TraceQuery {
        start: inverse_transform_point(body.position, axes, query.start),
        end: inverse_transform_point(body.position, axes, query.end),
        capsule: query
            .capsule
            .map(|capsule| capsule.to_local([axes[0], -axes[1], axes[2]])),
        ..query
    };
    assert_ne!(local_query.capsule.unwrap().offset.0[0], 0.0);
    let local = run(&store, &mut scratch, local_query, target()).unwrap();
    let hit = store
        .trace_body(
            query,
            CollisionShape::Capsule,
            &body,
            angles,
            rules,
            BodyTraceMode::Transformed,
            Some(TARGET),
            &mut scratch,
        )
        .unwrap();
    assert!(hit.fraction > 0.0 && hit.fraction < 1.0);
    assert_eq!(hit.fraction.to_bits(), local.fraction.to_bits());
    assert_eq!(hit.end, query.start.lerp(query.end, hit.fraction));
    let local_endpoint = inverse_transform_point(body.position, axes, hit.end);
    for axis in 0..3 {
        near(local_endpoint.0[axis], local.end.0[axis]);
    }
    near(hit.end.0[2], body.position.0[2] - 12.125);
    near(hit.plane.normal.0[0], 0.0);
    near(hit.plane.normal.0[1], 0.0);
    near(hit.plane.normal.0[2], -1.0);
    near(local.plane.distance, 1.0);
    let target_support = body.position + Vec3([0.0, 0.0, -1.0]);
    near(hit.plane.distance, hit.plane.normal.dot(target_support));
    near(hit.plane.distance, -29.0);
    near(hit.plane.signed_distance(target_support), 0.0);
    near(
        hit.plane.signed_distance(target_support + hit.plane.normal),
        1.0,
    );
    assert_eq!(hit.entity, Some(TARGET));
    assert_eq!(hit.contents, Contents::BODY);
    assert!(!hit.brush_solid);
    let endpoint = store
        .trace_body(
            TraceQuery {
                start: hit.end,
                end: hit.end,
                ..query
            },
            CollisionShape::Capsule,
            &body,
            angles,
            rules,
            BodyTraceMode::Transformed,
            Some(TARGET),
            &mut scratch,
        )
        .unwrap();
    assert!(!endpoint.start_solid && !endpoint.all_solid);
}

fn brush_store(occupied: bool) -> (CollisionStore, qa_core::primitives::GeometryId) {
    let planes = if occupied {
        vec![Plane::oriented(Vec3::axis(0, 1.0), -100.0)]
    } else {
        vec![]
    };
    let brushes = if occupied {
        vec![Brush {
            sides: qa_core::primitives::IndexSpan { first: 0, count: 1 },
            contents: Contents::SOLID,
        }]
    } else {
        vec![]
    };
    let surfaces = vec![SurfaceFlags::default(); planes.len()];
    let tree = BrushTree::direct(brushes.len()).unwrap();
    let mut store = CollisionStore::new();
    let geometry = store
        .load_brushes(
            planes,
            brushes,
            vec![],
            SurfaceTable::flags(surfaces),
            tree,
            vec![Bounds {
                mins: Vec3([-128.0; 3]),
                maxs: Vec3([128.0; 3]),
            }],
        )
        .unwrap();
    (store, geometry)
}

#[test]
fn linked_world_trace_adopts_the_same_tilted_capsule_contact_without_relinking() {
    let (store, geometry) = brush_store(false);
    let (query, body, angles, rules) = posed_case();
    let mut entities = EntityTable::new(8, 1).unwrap();
    let id = entities
        .allocate(
            ThinkTime::Seconds(0.0),
            ModuleId(1),
            AllocationPolicy::EDICT,
        )
        .unwrap()
        .id;
    let slot = id.slot as usize;
    entities.columns.set_body(slot, body);
    entities.columns.set_angles(slot, angles);
    entities.columns.model_rules[slot] = rules;
    entities.columns.collision_shape[slot] = CollisionShape::Capsule;
    entities.columns.collision_contents[slot] = Contents::BODY.0;
    let mut area = AreaGrid::load(
        8,
        Bounds {
            mins: Vec3([-128.0; 3]),
            maxs: Vec3([128.0; 3]),
        },
    )
    .unwrap();
    assert!(area.link(
        &entities,
        id,
        LinkFlags::SOLID,
        LinkOrder::Tail,
        LinkIntent::Explicit
    ));
    let bounds = area.bounds(id).unwrap();
    let mut scratch = store.scratch();
    let direct = store
        .trace_body(
            query,
            CollisionShape::Capsule,
            &body,
            angles,
            rules,
            BodyTraceMode::Transformed,
            Some(id),
            &mut scratch,
        )
        .unwrap();
    let linked = WorldTrace::new(
        &store,
        geometry,
        0,
        &entities,
        &area,
        &mut scratch,
        CollisionOwner::None,
    )
    .trace(query)
    .unwrap();
    assert!(linked.fraction < 1.0);
    same_trace(linked, direct);
    assert_eq!(linked.entity, Some(id));
    assert_eq!(area.bounds(id), Some(bounds));
    assert_eq!(entities.columns.body(slot).position, body.position);
}

#[test]
fn empty_and_brush_only_store_scratch_reuses_the_same_rounded_body_query() {
    let query = moving(
        Vec3([0.0, 0.0, -20.0]),
        Vec3([0.0, 0.0, 20.0]),
        Vec3([10.0, 0.0, 0.0]),
    );
    let expected = trace(query, target());
    assert!(expected.fraction < 1.0);
    for store in [CollisionStore::new(), brush_store(true).0] {
        let mut scratch = store.scratch();
        for _ in 0..8 {
            same_trace(
                run(&store, &mut scratch, query, target()).unwrap(),
                expected,
            );
            let clear = moving(
                Vec3([10.0, -10.0, 0.0]),
                Vec3([10.0, -10.0, 0.0]),
                Vec3([10.0, 10.0, 0.0]),
            );
            assert_eq!(
                run(&store, &mut scratch, clear, target()).unwrap().fraction,
                1.0
            );
        }
    }
}

#[test]
fn malformed_tilted_queries_reject_instead_of_returning_clear_and_scratch_recovers() {
    let store = CollisionStore::new();
    let mut scratch = store.scratch();
    let valid = moving(
        Vec3([0.0, 0.0, -20.0]),
        Vec3([0.0, 0.0, 20.0]),
        Vec3([10.0, 0.0, 0.0]),
    );
    let mut invalid = vec![];
    for radius in [-1.0, f32::INFINITY, f32::NAN] {
        invalid.push(TraceQuery {
            capsule: Some(Capsule {
                radius,
                offset: Vec3([10.0, 0.0, 0.0]),
            }),
            ..valid
        });
    }
    invalid.push(TraceQuery {
        capsule: Some(Capsule {
            radius: 1.0,
            offset: Vec3([f32::NAN, 0.0, 0.0]),
        }),
        ..valid
    });
    invalid.push(TraceQuery {
        capsule: Some(Capsule {
            radius: 1.0,
            offset: Vec3([10.0, f32::INFINITY, 0.0]),
        }),
        ..valid
    });
    invalid.push(TraceQuery {
        start: Vec3([f32::NAN, 0.0, -20.0]),
        ..valid
    });
    invalid.push(TraceQuery {
        end: Vec3([0.0, 0.0, f32::INFINITY]),
        ..valid
    });
    invalid.push(TraceQuery {
        mins: Vec3([100.0, -1.0, -1.0]),
        maxs: Vec3([-100.0, 1.0, 1.0]),
        ..valid
    });
    invalid.push(TraceQuery {
        mins: Vec3([f32::NEG_INFINITY, -1.0, -1.0]),
        ..valid
    });
    for query in invalid {
        assert_eq!(
            run(&store, &mut scratch, query, target()).unwrap_err(),
            TraceError::Query
        );
    }
    for body in [
        Body {
            mins: Vec3([2.0, -1.0, -11.0]),
            maxs: Vec3([1.0, 1.0, 11.0]),
            ..target()
        },
        Body {
            mins: Vec3([-1.0, 2.0, -11.0]),
            maxs: Vec3([1.0, 1.0, 11.0]),
            ..target()
        },
        Body {
            mins: Vec3([-1.0, f32::NAN, -11.0]),
            ..target()
        },
        Body {
            maxs: Vec3([1.0, 1.0, f32::INFINITY]),
            ..target()
        },
    ] {
        assert_eq!(
            run(&store, &mut scratch, valid, body).unwrap_err(),
            TraceError::Query
        );
    }
    same_trace(
        run(&store, &mut scratch, valid, target()).unwrap(),
        trace(valid, target()),
    );
}
