use qa_core::primitives::{
    Body, Bounds, Capsule, CollisionOwner, CollisionShape, EntityId, GeometryId, IndexSpan,
    ModelRotation, ModelRules, ModuleId, Plane, RuleSetId, SurfaceFlags, ThinkTime, Vec3,
};
use qa_world::collision::{
    CollisionStore, Contents, QuakeTraceKind, Trace, TraceQuery, WorldTrace,
    brushes::{Brush, BrushTree, CollisionLeaf, ModelRoot},
    hulls::{ClipNode, HullModel},
    store::BodyTraceMode,
    surfaces::SurfaceTable,
    trace_policy,
};
use qa_world::{
    area::{AreaGrid, LinkFlags, LinkIntent, LinkOrder},
    entities::{AllocationPolicy, EntityTable},
};

struct Fixture {
    store: CollisionStore,
    geometry: GeometryId,
}
fn interval(first: f32, last: f32) -> Bounds {
    Bounds {
        mins: Vec3([first, -2.0, -2.0]),
        maxs: Vec3([last, 2.0, 2.0]),
    }
}
fn brush_map(source: RuleSetId, volumes: &[(Bounds, Contents)]) -> Fixture {
    brush_members(source, volumes, None)
}
fn brush_parts(volumes: &[(Bounds, Contents)]) -> (Vec<Plane>, Vec<Brush>) {
    let mut planes = vec![];
    let brushes = volumes
        .iter()
        .map(|(bounds, contents)| {
            let first = planes.len() as u32;
            for axis in 0..3 {
                planes.push(Plane::oriented(Vec3::axis(axis, 1.0), bounds.maxs.0[axis]));
                planes.push(Plane::oriented(
                    Vec3::axis(axis, -1.0),
                    -bounds.mins.0[axis],
                ));
            }
            Brush {
                sides: IndexSpan { first, count: 6 },
                contents: *contents,
            }
        })
        .collect::<Vec<_>>();
    (planes, brushes)
}
fn brush_members(
    source: RuleSetId,
    volumes: &[(Bounds, Contents)],
    members: Option<Vec<u32>>,
) -> Fixture {
    let (planes, brushes) = brush_parts(volumes);
    let mut tree = BrushTree::direct(brushes.len()).unwrap();
    if let Some(members) = members {
        tree.leaves[0].brushes.count = members.len() as u32;
        tree.leaf_brushes = members;
    }
    if source == RuleSetId::Quake2 {
        tree.leaves[0].stored_contents = Some(
            volumes
                .iter()
                .fold(Contents::EMPTY, |all, (_, contents)| all | *contents),
        );
    }
    let surfaces = vec![SurfaceFlags::default(); planes.len()];
    let mut store = CollisionStore::new();
    let geometry = store
        .load_brushes(
            planes,
            brushes,
            vec![],
            SurfaceTable::flags(surfaces),
            tree,
            vec![Bounds {
                mins: Vec3([-64.0; 3]),
                maxs: Vec3([64.0; 3]),
            }],
        )
        .unwrap();
    Fixture { store, geometry }
}
fn authored_map(volumes: &[(Bounds, Contents)]) -> Fixture {
    let (planes, brushes) = brush_parts(volumes);
    let count = brushes.len() as u32;
    let mut store = CollisionStore::new();
    let geometry = store
        .load_hulls(
            planes,
            vec![],
            vec![],
            vec![HullModel {
                roots: [-2; 3],
                brushes: IndexSpan { first: 0, count },
            }],
            vec![Bounds {
                mins: Vec3([-64.0; 3]),
                maxs: Vec3([64.0; 3]),
            }],
            Some(brushes),
            Default::default(),
        )
        .unwrap();
    Fixture { store, geometry }
}
fn maps(volumes: &[(Bounds, Contents)]) -> [Fixture; 3] {
    [
        brush_map(RuleSetId::Quake2, volumes),
        brush_map(RuleSetId::Quake3, volumes),
        authored_map(volumes),
    ]
}
fn query(role: RuleSetId) -> TraceQuery<'static> {
    let (rules, entities) = trace_policy(role);
    TraceQuery::point(
        Vec3([0.0, 0.0, 0.0]),
        Vec3([10.0, 0.0, 0.0]),
        rules,
        entities,
    )
}
fn run(fixture: &Fixture, query: TraceQuery<'_>) -> Trace {
    let mut scratch = fixture.store.scratch();
    let result = fixture
        .store
        .trace_model(fixture.geometry, 0, query, &mut scratch)
        .unwrap();
    if query.rules.reached_media {
        let disabled = TraceQuery {
            rules: qa_world::collision::TraceRules {
                reached_media: false,
                ..query.rules
            },
            ..query
        };
        let baseline = fixture
            .store
            .trace_model(fixture.geometry, 0, disabled, &mut scratch)
            .unwrap();
        same_contact(result, baseline);
    }
    result
}
fn same_contact(actual: Trace, expected: Trace) {
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
    assert_eq!(actual.plane.axis, expected.plane.axis);
    let plane_bits = |plane: Plane| {
        (
            plane.normal.0.map(f32::to_bits),
            plane.distance.to_bits(),
            plane.encoding,
            plane.axis,
        )
    };
    assert_eq!(
        actual.secondary_plane.map(plane_bits),
        expected.secondary_plane.map(plane_bits)
    );
    assert_eq!(
        (
            actual.start_solid,
            actual.all_solid,
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

#[test]
fn q1_callers_observe_open_empty_brush_models_without_changing_q2_q3_roles() {
    for source in [RuleSetId::Quake2, RuleSetId::Quake3] {
        let fixture = brush_map(source, &[]);
        for role in [
            RuleSetId::Quake,
            RuleSetId::QuakeWorld,
            RuleSetId::Quake2,
            RuleSetId::Quake2Rerelease,
            RuleSetId::Quake3,
        ] {
            let hit = run(&fixture, query(role));
            assert_eq!(hit.fraction, 1.0);
            assert!(!hit.start_solid && !hit.all_solid);
            assert_eq!(
                (hit.in_open, hit.in_water),
                (
                    matches!(role, RuleSetId::Quake | RuleSetId::QuakeWorld),
                    false
                ),
                "source={source:?} role={role:?} {hit:?}"
            );
        }
    }
}

#[test]
fn q1_brush_media_stops_at_the_selected_wall_and_observes_water_before_it() {
    for source in [RuleSetId::Quake2, RuleSetId::Quake3] {
        for (water, expected) in [(interval(2.0, 4.0), true), (interval(8.0, 9.0), false)] {
            let fixture = brush_map(
                source,
                &[
                    (water, Contents::WATER),
                    (interval(6.0, 7.0), Contents::SOLID),
                ],
            );
            for role in [
                RuleSetId::Quake,
                RuleSetId::QuakeWorld,
                RuleSetId::Quake2,
                RuleSetId::Quake3,
            ] {
                let hit = run(&fixture, query(role));
                assert!(hit.fraction > 0.0 && hit.fraction < 0.6);
                assert_eq!(hit.contents, Contents::SOLID);
                let enabled = matches!(role, RuleSetId::Quake | RuleSetId::QuakeWorld);
                assert_eq!(
                    (hit.in_open, hit.in_water),
                    (enabled, enabled && expected),
                    "source={source:?} role={role:?} water={water:?} {hit:?}"
                );
            }
        }
    }
}

#[test]
fn allsolid_reached_brush_space_cannot_observe_overlapping_water_or_open_space() {
    for source in [RuleSetId::Quake2, RuleSetId::Quake3] {
        let fixture = brush_map(
            source,
            &[
                (interval(-1.0, 11.0), Contents::SOLID),
                (interval(2.0, 4.0), Contents::WATER),
            ],
        );
        let hit = run(&fixture, query(RuleSetId::Quake));
        assert!(hit.start_solid && hit.all_solid);
        assert_eq!(hit.fraction, 1.0);
        assert!(!hit.in_open && !hit.in_water);
    }
}

#[test]
fn overlapping_media_uses_union_coverage_and_solid_precedence_instead_of_any_brush() {
    let cases = [
        (vec![(interval(-1.0, 11.0), Contents::WATER)], (false, true)),
        (vec![(interval(2.0, 4.0), Contents::WATER)], (true, true)),
        (
            vec![
                (interval(-1.0, 5.0), Contents::WATER),
                (interval(5.0, 11.0), Contents::SLIME),
            ],
            (false, true),
        ),
        (
            vec![
                (interval(2.0, 6.0), Contents::WATER),
                (interval(4.0, 8.0), Contents::LAVA),
            ],
            (true, true),
        ),
        (
            vec![
                (interval(2.0, 4.0), Contents::SOLID),
                (interval(2.0, 4.0), Contents::WATER),
            ],
            (true, false),
        ),
        (
            vec![
                (interval(2.0, 4.0), Contents::SOLID),
                (interval(3.0, 5.0), Contents::WATER),
            ],
            (true, true),
        ),
        (
            vec![
                (interval(-1.0, 5.0), Contents::SOLID),
                (interval(5.0, 11.0), Contents::WATER),
            ],
            (false, true),
        ),
        (
            vec![(interval(-1.0, 11.0), Contents::SOLID | Contents::WATER)],
            (false, false),
        ),
    ];
    for (volumes, expected) in cases {
        for fixture in maps(&volumes) {
            // None of these volumes blocks this request. Media still observes
            // their geometry, including solids hidden by the collision mask.
            let hit = run(
                &fixture,
                TraceQuery {
                    mask: Contents::EMPTY,
                    ..query(RuleSetId::Quake)
                },
            );
            assert_eq!(hit.fraction, 1.0);
            assert!(!hit.start_solid && !hit.all_solid);
            assert_eq!((hit.in_open, hit.in_water), expected, "{volumes:?} {hit:?}");
        }
        let reversed = volumes.iter().rev().copied().collect::<Vec<_>>();
        let fixture = brush_map(RuleSetId::Quake3, &reversed);
        assert_eq!(
            {
                let hit = run(
                    &fixture,
                    TraceQuery {
                        mask: Contents::EMPTY,
                        ..query(RuleSetId::Quake)
                    },
                );
                (hit.in_open, hit.in_water)
            },
            expected
        );
        let duplicated = brush_members(
            RuleSetId::Quake2,
            &volumes,
            Some((0..volumes.len() as u32).flat_map(|id| [id, id]).collect()),
        );
        let hit = run(
            &duplicated,
            TraceQuery {
                mask: Contents::EMPTY,
                ..query(RuleSetId::Quake)
            },
        );
        assert_eq!((hit.in_open, hit.in_water), expected);
    }
}

#[test]
fn media_uses_the_published_fraction_and_does_not_reach_the_geometric_wall() {
    for (water, expected) in [(interval(5.95, 5.96), true), (interval(5.98, 5.99), false)] {
        for fixture in maps(&[
            (water, Contents::WATER),
            (interval(6.0, 7.0), Contents::SOLID),
        ]) {
            let hit = run(&fixture, query(RuleSetId::Quake));
            assert!((hit.end.0[0] - 5.96875).abs() < 2.0e-6);
            assert_eq!((hit.in_open, hit.in_water), (true, expected));
        }
    }
}

#[test]
fn stationary_points_and_closed_liquid_touches_keep_solid_precedence() {
    for (volumes, point, expected) in [
        (vec![], 3.0, (true, false)),
        (
            vec![(interval(2.0, 4.0), Contents::WATER)],
            3.0,
            (false, true),
        ),
        (
            vec![(interval(2.0, 4.0), Contents::WATER)],
            2.0,
            (false, true),
        ),
        (
            vec![
                (interval(2.0, 4.0), Contents::SOLID),
                (interval(2.0, 4.0), Contents::WATER),
            ],
            2.0,
            (false, false),
        ),
    ] {
        for fixture in maps(&volumes) {
            let point = Vec3([point, 0.0, 0.0]);
            let hit = run(
                &fixture,
                TraceQuery {
                    start: point,
                    end: point,
                    mask: Contents::EMPTY,
                    ..query(RuleSetId::Quake)
                },
            );
            assert_eq!((hit.in_open, hit.in_water), expected);
        }
    }
    // Reached range has positive length, but its closed endpoint alone is
    // liquid. A coincident solid point suppresses that water observation.
    for solid in [false, true] {
        let mut volumes = vec![(interval(2.0, 4.0), Contents::WATER)];
        if solid {
            volumes.push((interval(2.0, 4.0), Contents::SOLID));
        }
        for fixture in maps(&volumes) {
            let hit = run(
                &fixture,
                TraceQuery {
                    end: Vec3([2.0, 0.0, 0.0]),
                    mask: Contents::EMPTY,
                    ..query(RuleSetId::Quake)
                },
            );
            assert_eq!((hit.in_open, hit.in_water), (true, !solid));
        }
    }
}

#[test]
fn native_contents_projection_classifies_media_without_reusing_the_blocking_mask() {
    for (contents, expected) in [
        (Contents::WINDOW, (false, false)),
        (Contents::BODY, (false, false)),
        (Contents::CORPSE, (false, false)),
        (Contents::PLAYER, (false, false)),
        (Contents::PROJECTILE, (false, false)),
        (Contents::SKY, (false, true)),
        (Contents::from_q1(-9, Contents::EMPTY), (false, true)),
        (
            Contents::VOLUME_CLIP | Contents::LADDER | Contents::AUX,
            (true, false),
        ),
    ] {
        for fixture in maps(&[(interval(-1.0, 11.0), contents)]) {
            for mask in [Contents::EMPTY, Contents::SOLID, Contents::WATER] {
                let hit = run(
                    &fixture,
                    TraceQuery {
                        mask,
                        ..query(RuleSetId::Quake)
                    },
                );
                assert_eq!(
                    (hit.in_open, hit.in_water),
                    expected,
                    "{contents:?} {mask:?} {hit:?}"
                );
            }
        }
    }
}

#[test]
fn media_startsolid_exit_and_full_inside_use_the_actual_reached_span() {
    for fixture in maps(&[
        (interval(-1.0, 4.0), Contents::SOLID),
        (interval(6.0, 8.0), Contents::WATER),
    ]) {
        let exiting = run(&fixture, query(RuleSetId::Quake));
        assert!(exiting.start_solid && !exiting.all_solid);
        assert_eq!(exiting.fraction, 1.0);
        assert_eq!((exiting.in_open, exiting.in_water), (true, true));
        let inside = run(
            &fixture,
            TraceQuery {
                end: Vec3([2.0, 0.0, 0.0]),
                ..query(RuleSetId::Quake)
            },
        );
        assert!(inside.start_solid && inside.all_solid);
        assert_eq!((inside.in_open, inside.in_water), (false, false));
    }
}

#[test]
fn exact_bsp_cells_and_authored_empty_models_observe_media_without_changing_native_hulls() {
    let planes = [6.0, 4.0, 2.0]
        .map(|distance| Plane::oriented(Vec3::axis(0, 1.0), distance))
        .to_vec();
    let nodes = vec![
        ClipNode {
            plane: 0,
            children: [-2, 1],
        },
        ClipNode {
            plane: 1,
            children: [-1, 2],
        },
        ClipNode {
            plane: 2,
            children: [-3, -1],
        },
    ];
    let mut store = CollisionStore::new();
    let geometry = store
        .load_hulls(
            planes,
            nodes.clone(),
            nodes,
            vec![HullModel {
                roots: [0; 3],
                brushes: IndexSpan::default(),
            }],
            vec![Bounds {
                mins: Vec3([-64.0; 3]),
                maxs: Vec3([64.0; 3]),
            }],
            None,
            Default::default(),
        )
        .unwrap();
    let original = store
        .trace_model(geometry, 0, query(RuleSetId::Quake), &mut store.scratch())
        .unwrap();
    assert_eq!((original.in_open, original.in_water), (true, true));
    store
        .prepare_hull_positions(geometry, Default::default())
        .unwrap();
    let fixture = Fixture { store, geometry };
    let native = run(&fixture, query(RuleSetId::Quake));
    same_contact(native, original);
    assert_eq!(
        (native.in_open, native.in_water),
        (original.in_open, original.in_water)
    );
    let exact = TraceQuery {
        rules: qa_world::collision::TraceRules {
            hull_shape: qa_world::collision::HullShape::ExactBox,
            ..query(RuleSetId::Quake).rules
        },
        ..query(RuleSetId::Quake)
    };
    let hit = run(&fixture, exact);
    assert_eq!((hit.in_open, hit.in_water), (true, true));
    for query in [
        exact,
        TraceQuery {
            mins: Vec3([-0.25; 3]),
            maxs: Vec3([0.25; 3]),
            ..exact
        }
        .with_capsule(),
    ] {
        let hit = run(&fixture, query);
        assert!(hit.fraction < 0.6);
        assert_eq!((hit.in_open, hit.in_water), (true, true));
    }
    let empty = authored_map(&[]);
    let hit = run(&empty, query(RuleSetId::Quake));
    assert_eq!(hit.fraction, 1.0);
    assert_eq!((hit.in_open, hit.in_water), (true, false));
}

#[test]
fn asymmetric_and_capsule_queries_share_media_in_their_posed_inline_model() {
    let volumes = [
        (interval(2.0, 4.0), Contents::WATER),
        (interval(6.0, 7.0), Contents::SOLID),
    ];
    for fixture in maps(&volumes) {
        for local in [
            TraceQuery {
                mins: Vec3([-0.25, -0.5, -0.5]),
                maxs: Vec3([0.75, 0.5, 0.5]),
                ..query(RuleSetId::Quake)
            },
            TraceQuery {
                mins: Vec3([-0.5; 3]),
                maxs: Vec3([0.5; 3]),
                capsule: Some(Capsule {
                    radius: 0.25,
                    offset: Vec3([0.25, 0.0, 0.0]),
                }),
                ..query(RuleSetId::Quake)
            },
        ] {
            let direct = run(&fixture, local);
            assert_eq!((direct.in_open, direct.in_water), (true, true));
            let posed = TraceQuery {
                start: Vec3([10.0, 20.0, 0.0]),
                end: Vec3([10.0, 30.0, 0.0]),
                ..local
            };
            let rules = ModelRules {
                rotation: ModelRotation::TransposeBasis,
                ..Default::default()
            };
            let mut scratch = fixture.store.scratch();
            let hit = fixture
                .store
                .trace_transformed(
                    fixture.geometry,
                    0,
                    posed,
                    Vec3([10.0, 20.0, 0.0]),
                    Vec3([0.0, 90.0, 0.0]),
                    rules,
                    &mut scratch,
                )
                .unwrap();
            assert_eq!((hit.in_open, hit.in_water), (true, true));
            assert!(hit.fraction > 0.0 && hit.fraction < 0.6);
            let disabled = TraceQuery {
                rules: qa_world::collision::TraceRules {
                    reached_media: false,
                    ..posed.rules
                },
                ..posed
            };
            let baseline = fixture
                .store
                .trace_transformed(
                    fixture.geometry,
                    0,
                    disabled,
                    Vec3([10.0, 20.0, 0.0]),
                    Vec3([0.0, 90.0, 0.0]),
                    rules,
                    &mut scratch,
                )
                .unwrap();
            same_contact(hit, baseline);
        }
    }
}

fn body_trace(query: TraceQuery<'_>, shape: CollisionShape, body: Body) -> Trace {
    let store = CollisionStore::new();
    let mut scratch = store.scratch();
    let id = Some(EntityId {
        slot: 7,
        generation: 2,
    });
    let hit = store
        .trace_body(
            query,
            shape,
            &body,
            Vec3::default(),
            ModelRules::default(),
            BodyTraceMode::Transformed,
            id,
            &mut scratch,
        )
        .unwrap();
    let disabled = TraceQuery {
        rules: qa_world::collision::TraceRules {
            reached_media: false,
            ..query.rules
        },
        ..query
    };
    let baseline = store
        .trace_body(
            disabled,
            shape,
            &body,
            Vec3::default(),
            ModelRules::default(),
            BodyTraceMode::Transformed,
            id,
            &mut scratch,
        )
        .unwrap();
    same_contact(hit, baseline);
    hit
}

#[test]
fn all_winning_body_shapes_publish_their_own_reached_open_space() {
    let mut native_box = query(RuleSetId::Quake);
    native_box.entity_rules = native_box
        .entity_rules
        .with_quake_kind(Some(QuakeTraceKind::Normal));
    let volume = TraceQuery {
        mins: Vec3([-0.5; 3]),
        maxs: Vec3([0.5; 3]),
        ..query(RuleSetId::Quake)
    };
    for (shape, query) in [
        (CollisionShape::Box, native_box),
        (CollisionShape::Box, query(RuleSetId::Quake)),
        (CollisionShape::Box, volume.with_capsule()),
        (CollisionShape::Capsule, volume),
        (CollisionShape::Capsule, volume.with_capsule()),
        (
            CollisionShape::Capsule,
            TraceQuery {
                capsule: Some(Capsule {
                    radius: 0.25,
                    offset: Vec3([0.25, 0.0, 0.0]),
                }),
                ..volume
            },
        ),
    ] {
        let body = Body {
            position: Vec3([6.0, 0.0, 0.0]),
            mins: Vec3([-1.0, -1.0, -3.0]),
            maxs: Vec3([1.0, 1.0, 3.0]),
            ..Default::default()
        };
        let query = TraceQuery {
            mask: Contents::SOLID | Contents::BODY,
            ..query
        };
        let hit = body_trace(query, shape, body);
        assert!(
            hit.fraction > 0.0 && hit.fraction < 1.0,
            "{shape:?} {query:?} {hit:?}"
        );
        assert_eq!((hit.in_open, hit.in_water), (true, false));
        assert_eq!(
            hit.entity,
            Some(EntityId {
                slot: 7,
                generation: 2
            })
        );
        let inside = body_trace(
            TraceQuery {
                start: body.position + Vec3([0.0, 0.0, 2.0]),
                end: body.position + Vec3([0.0, 0.0, 2.0]),
                ..query
            },
            shape,
            body,
        );
        assert!(inside.start_solid && inside.all_solid);
        assert_eq!((inside.in_open, inside.in_water), (false, false));
    }
}

#[test]
fn inline_media_uses_only_the_selected_models_brush_membership() {
    let (planes, brushes) = brush_parts(&[
        (interval(-1.0, 11.0), Contents::SOLID),
        (interval(2.0, 4.0), Contents::WATER),
        (interval(6.0, 7.0), Contents::SOLID),
    ]);
    let surfaces = vec![SurfaceFlags::default(); planes.len()];
    let bounds = Bounds {
        mins: Vec3([-64.0; 3]),
        maxs: Vec3([64.0; 3]),
    };
    let tree = BrushTree {
        planes: vec![],
        nodes: vec![],
        leaves: vec![
            CollisionLeaf {
                stored_contents: None,
                brushes: IndexSpan { first: 0, count: 1 },
                patches: IndexSpan::default(),
            },
            CollisionLeaf {
                stored_contents: None,
                brushes: IndexSpan { first: 1, count: 2 },
                patches: IndexSpan::default(),
            },
        ],
        leaf_brushes: vec![0, 1, 2],
        leaf_patches: vec![],
        models: vec![ModelRoot::Leaf(0), ModelRoot::Leaf(1)],
    };
    let mut store = CollisionStore::new();
    let geometry = store
        .load_brushes(
            planes,
            brushes,
            vec![],
            SurfaceTable::flags(surfaces),
            tree,
            vec![bounds; 2],
        )
        .unwrap();
    let mut scratch = store.scratch();
    let world = store
        .trace_model(geometry, 0, query(RuleSetId::Quake), &mut scratch)
        .unwrap();
    assert!(world.all_solid);
    assert_eq!((world.in_open, world.in_water), (false, false));
    let hit = store
        .trace_model(geometry, 1, query(RuleSetId::Quake), &mut scratch)
        .unwrap();
    assert_eq!((hit.in_open, hit.in_water), (true, true));
    let posed = TraceQuery {
        start: Vec3([10.0, 20.0, 0.0]),
        end: Vec3([10.0, 30.0, 0.0]),
        ..query(RuleSetId::Quake)
    };
    let hit = store
        .trace_transformed(
            geometry,
            1,
            posed,
            Vec3([10.0, 20.0, 0.0]),
            Vec3([0.0, 90.0, 0.0]),
            ModelRules {
                rotation: ModelRotation::TransposeBasis,
                ..Default::default()
            },
            &mut scratch,
        )
        .unwrap();
    assert_eq!((hit.in_open, hit.in_water), (true, true));
    assert!(hit.fraction < 0.6);
}

fn linked(
    fixture: &Fixture,
    body: Body,
    shape: CollisionShape,
    query: TraceQuery<'_>,
) -> (Trace, EntityId, EntityId) {
    let mut entities = EntityTable::new(8, 1).unwrap();
    let world = entities.id_at(0).unwrap();
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
    entities.columns.collision_shape[slot] = shape;
    entities.columns.collision_contents[slot] = Contents::BODY.0;
    let mut area = AreaGrid::load(
        8,
        Bounds {
            mins: Vec3([-64.0; 3]),
            maxs: Vec3([64.0; 3]),
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
    let mut scratch = fixture.store.scratch();
    let hit = WorldTrace::new(
        &fixture.store,
        fixture.geometry,
        0,
        &entities,
        &area,
        &mut scratch,
        CollisionOwner::None,
    )
    .trace(query)
    .unwrap();
    assert_eq!(area.bounds(id), Some(bounds));
    (hit, id, world)
}

#[test]
fn linked_winners_keep_their_own_media_and_losing_records_do_not_or_flags() {
    let mut native = query(RuleSetId::Quake);
    native.mask |= Contents::BODY;
    native.entity_rules = native
        .entity_rules
        .with_quake_kind(Some(QuakeTraceKind::Normal));
    let near = Body {
        position: Vec3([6.0, 0.0, 0.0]),
        mins: Vec3([-1.0, -1.0, -3.0]),
        maxs: Vec3([1.0, 1.0, 3.0]),
        ..Default::default()
    };
    let fixture = brush_map(
        RuleSetId::Quake3,
        &[
            (interval(2.0, 4.0), Contents::WATER),
            (interval(8.0, 9.0), Contents::SOLID),
        ],
    );
    assert_eq!(
        {
            let hit = run(&fixture, native);
            (hit.in_open, hit.in_water)
        },
        (true, true)
    );
    for shape in [CollisionShape::Box, CollisionShape::Capsule] {
        let (hit, id, _) = linked(&fixture, near, shape, native);
        assert_eq!(hit.entity, Some(id));
        assert!(hit.fraction < 0.6);
        assert_eq!((hit.in_open, hit.in_water), (true, false));
    }
    // A nonwinning clear body reports open, but it cannot add that flag to
    // the retained entirely-submerged world record.
    let submerged = brush_map(
        RuleSetId::Quake3,
        &[(interval(-1.0, 11.0), Contents::WATER)],
    );
    let (hit, _, _) = linked(
        &submerged,
        Body {
            position: Vec3([20.0, 0.0, 0.0]),
            ..near
        },
        CollisionShape::Capsule,
        native,
    );
    assert_eq!(hit.fraction, 1.0);
    assert_eq!((hit.in_open, hit.in_water), (false, true));
    // End inside the box so the native hull does not first split its far
    // X face. Both targets then perform the same single entering division.
    let tied = brush_map(
        RuleSetId::Quake3,
        &[
            (interval(2.0, 4.0), Contents::WATER),
            (interval(6.0, 7.0), Contents::SOLID),
        ],
    );
    let tied_body = Body {
        position: Vec3([6.5, 0.0, 0.0]),
        mins: Vec3([-0.5, -2.0, -2.0]),
        maxs: Vec3([0.5, 2.0, 2.0]),
        ..Default::default()
    };
    let tied_query = TraceQuery {
        end: Vec3([6.5, 0.0, 0.0]),
        ..native
    };
    let world_hit = run(&tied, tied_query);
    let body_hit = body_trace(tied_query, CollisionShape::Box, tied_body);
    assert_eq!(world_hit.fraction.to_bits(), body_hit.fraction.to_bits());
    let (hit, _, world) = linked(&tied, tied_body, CollisionShape::Box, tied_query);
    assert_eq!(hit.entity, Some(world));
    assert_eq!((hit.in_open, hit.in_water), (true, true));
}

#[test]
fn exact_capsule_media_rejects_a_water_corner_that_shifted_faces_would_include() {
    let water = Bounds {
        mins: Vec3::default(),
        maxs: Vec3([1.0; 3]),
    };
    let fixture = authored_map(&[(water, Contents::WATER)]);
    for (xy, expected) in [(1.75, (true, false)), (1.5, (false, true))] {
        let query = TraceQuery {
            start: Vec3([xy, xy, 0.5]),
            end: Vec3([xy, xy, 0.6]),
            mins: Vec3([-1.0, -1.0, -1.25]),
            maxs: Vec3([1.0, 1.0, 1.25]),
            capsule: Some(Capsule {
                radius: 1.0,
                offset: Vec3([0.0, 0.0, 0.25]),
            }),
            ..query(RuleSetId::Quake)
        };
        let hit = run(&fixture, query);
        assert_eq!(hit.fraction, 1.0);
        assert_eq!((hit.in_open, hit.in_water), expected);
    }
}

#[test]
fn liquid_shader_patch_stops_collision_without_becoming_a_closed_liquid_volume() {
    let mut tree = BrushTree::direct(0).unwrap();
    tree.leaf_patches = vec![0];
    tree.leaves[0].patches = IndexSpan { first: 0, count: 1 };
    let patch = qa_world::collision::patches::PatchInput {
        dimensions: [3, 3],
        control_points: [-4.0, 0.0, 4.0]
            .into_iter()
            .flat_map(|y| [-4.0, 0.0, 4.0].map(move |x| Vec3([x, y, 6.0])))
            .collect(),
        contents: Contents::SOLID | Contents::WATER,
        surface: 0,
    };
    let mut store = CollisionStore::new();
    let geometry = store
        .load_brushes(
            vec![],
            vec![],
            vec![patch],
            SurfaceTable {
                records: vec![qa_world::collision::surfaces::SurfaceInput::unnamed(
                    SurfaceFlags::default(),
                )],
                sides: vec![],
            },
            tree,
            vec![Bounds {
                mins: Vec3([-64.0; 3]),
                maxs: Vec3([64.0; 3]),
            }],
        )
        .unwrap();
    let fixture = Fixture { store, geometry };
    let hit = run(
        &fixture,
        TraceQuery {
            start: Vec3([0.0, 0.0, 10.0]),
            end: Vec3::default(),
            ..query(RuleSetId::Quake)
        },
    );
    assert!(hit.fraction > 0.0 && hit.fraction < 0.4);
    assert_eq!(hit.contents, Contents::SOLID | Contents::WATER);
    assert_eq!((hit.in_open, hit.in_water), (true, false));
}

#[test]
fn repeated_media_queries_reuse_the_same_arenas_without_stamps_or_flags_leaking() {
    let volumes = [
        (interval(2.0, 4.0), Contents::WATER),
        (interval(6.0, 7.0), Contents::SOLID),
    ];
    for fixture in maps(&volumes) {
        let mut scratch = fixture.store.scratch();
        for _ in 0..4 {
            for (query, expected) in [
                (query(RuleSetId::Quake), (true, true)),
                (
                    TraceQuery {
                        end: Vec3([1.0, 0.0, 0.0]),
                        ..query(RuleSetId::Quake)
                    },
                    (true, false),
                ),
                (
                    TraceQuery {
                        start: Vec3([3.0, 0.0, 0.0]),
                        end: Vec3([3.0, 0.0, 0.0]),
                        ..query(RuleSetId::Quake)
                    },
                    (false, true),
                ),
                (
                    TraceQuery {
                        start: Vec3([6.5, 0.0, 0.0]),
                        end: Vec3([6.5, 0.0, 0.0]),
                        ..query(RuleSetId::Quake)
                    },
                    (false, false),
                ),
            ] {
                let hit = fixture
                    .store
                    .trace_model(fixture.geometry, 0, query, &mut scratch)
                    .unwrap();
                assert_eq!((hit.in_open, hit.in_water), expected);
            }
        }
    }
}

#[test]
fn exact_cell_media_keeps_strict_native_partition_boundaries_for_points_and_capsules() {
    let plane = Plane::oriented(Vec3::axis(0, 1.0), 0.0);
    let nodes = vec![ClipNode {
        plane: 0,
        children: [-1, -3],
    }];
    let mut store = CollisionStore::new();
    let geometry = store
        .load_hulls(
            vec![plane],
            nodes.clone(),
            nodes,
            vec![HullModel {
                roots: [0; 3],
                brushes: IndexSpan::default(),
            }],
            vec![Bounds {
                mins: Vec3([-64.0; 3]),
                maxs: Vec3([64.0; 3]),
            }],
            None,
            Default::default(),
        )
        .unwrap();
    let on_plane = TraceQuery {
        start: Vec3::default(),
        end: Vec3([0.0, 1.0, 0.0]),
        ..query(RuleSetId::Quake)
    };
    for native in [
        on_plane,
        TraceQuery {
            end: on_plane.start,
            ..on_plane
        },
    ] {
        let hit = store
            .trace_model(geometry, 0, native, &mut store.scratch())
            .unwrap();
        assert_eq!((hit.in_open, hit.in_water), (true, false));
    }
    store
        .prepare_hull_positions(geometry, Default::default())
        .unwrap();
    let fixture = Fixture { store, geometry };
    let exact = TraceQuery {
        rules: qa_world::collision::TraceRules {
            hull_shape: qa_world::collision::HullShape::ExactBox,
            ..on_plane.rules
        },
        ..on_plane
    };
    for moving in [true, false] {
        let point = TraceQuery {
            end: if moving { exact.end } else { exact.start },
            ..exact
        };
        let hit = run(&fixture, point);
        assert_eq!((hit.in_open, hit.in_water), (true, false));
        let penetrating = run(
            &fixture,
            TraceQuery {
                start: point.start + Vec3([-1.0, 0.0, 0.0]),
                end: point.end + Vec3([-1.0, 0.0, 0.0]),
                ..point
            },
        );
        assert_eq!((penetrating.in_open, penetrating.in_water), (false, true));
        for (x, expected) in [(1.0, (true, false)), (0.5, (false, true))] {
            let capsule = TraceQuery {
                start: point.start + Vec3([x, 0.0, 0.0]),
                end: point.end + Vec3([x, 0.0, 0.0]),
                mins: Vec3([-1.0, -1.0, -1.5]),
                maxs: Vec3([1.0, 1.0, 1.5]),
                capsule: Some(Capsule {
                    radius: 1.0,
                    offset: Vec3([0.0, 0.0, 0.5]),
                }),
                ..point
            };
            let hit = run(&fixture, capsule);
            assert_eq!(
                (hit.in_open, hit.in_water),
                expected,
                "x={x} moving={moving} {hit:?}"
            );
        }
    }
}

#[test]
fn capsule_extrusion_keeps_strictness_when_opposing_rows_generate_a_wedge_constraint() {
    let nodes = vec![
        ClipNode {
            plane: 0,
            children: [-1, 1],
        },
        ClipNode {
            plane: 1,
            children: [-3, -1],
        },
    ];
    let mut store = CollisionStore::new();
    let geometry = store
        .load_hulls(
            vec![
                Plane::oriented(Vec3::axis(0, 1.0), 0.0),
                Plane::oriented(Vec3::axis(1, 1.0), 0.0),
            ],
            nodes.clone(),
            nodes,
            vec![HullModel {
                roots: [0; 3],
                brushes: IndexSpan::default(),
            }],
            vec![Bounds {
                mins: Vec3([-64.0; 3]),
                maxs: Vec3([64.0; 3]),
            }],
            None,
            Default::default(),
        )
        .unwrap();
    store
        .prepare_hull_positions(geometry, Default::default())
        .unwrap();
    let fixture = Fixture { store, geometry };
    // WATER is x<0,y>=0. Every point on the zero-radius segment has x=y,
    // so even its origin cannot enter this strict wedge. A positive radius can.
    for (radius, expected) in [(0.0, (true, false)), (1.0 / 16.0, (false, true))] {
        let extent = Vec3([1.0 + radius, 1.0 + radius, radius]);
        let base = query(RuleSetId::Quake);
        let query = TraceQuery {
            start: Vec3::default(),
            end: Vec3::default(),
            mins: -extent,
            maxs: extent,
            mask: Contents::EMPTY,
            capsule: Some(Capsule {
                radius,
                offset: Vec3([1.0, 1.0, 0.0]),
            }),
            rules: qa_world::collision::TraceRules {
                hull_shape: qa_world::collision::HullShape::ExactBox,
                ..base.rules
            },
            ..base
        };
        let hit = run(&fixture, query);
        assert_eq!(hit.fraction, 1.0);
        assert!(!hit.start_solid && !hit.all_solid);
        assert_eq!(
            (hit.in_open, hit.in_water),
            expected,
            "radius={radius} {hit:?}"
        );
    }
}

#[test]
fn centered_media_tree_envelope_matches_direct_membership_after_native_rounding() {
    let water = Bounds {
        mins: Vec3([16_777_222.0, -2.0, -2.0]),
        maxs: Vec3([16_777_230.0, 2.0, 2.0]),
    };
    let (planes, brushes) = brush_parts(&[(water, Contents::WATER)]);
    let surfaces = vec![SurfaceFlags::default(); planes.len()];
    let tree = BrushTree {
        planes: vec![Plane::oriented(Vec3::axis(0, 1.0), 16_777_222.0)],
        nodes: vec![ClipNode {
            plane: 0,
            children: [-1, -2],
        }],
        leaves: vec![
            CollisionLeaf {
                stored_contents: Some(Contents::WATER),
                brushes: IndexSpan { first: 0, count: 1 },
                patches: IndexSpan::default(),
            },
            CollisionLeaf {
                stored_contents: Some(Contents::EMPTY),
                brushes: IndexSpan::default(),
                patches: IndexSpan::default(),
            },
        ],
        leaf_brushes: vec![0],
        leaf_patches: vec![],
        models: vec![ModelRoot::Tree(0), ModelRoot::Leaf(0)],
    };
    let bounds = Bounds {
        mins: Vec3([16_777_200.0, -64.0, -64.0]),
        maxs: Vec3([16_777_240.0, 64.0, 64.0]),
    };
    let mut store = CollisionStore::new();
    let geometry = store
        .load_brushes(
            planes,
            brushes,
            vec![],
            SurfaceTable::flags(surfaces),
            tree,
            vec![bounds; 2],
        )
        .unwrap();
    let base = query(RuleSetId::Quake);
    let query = TraceQuery {
        start: Vec3([16_777_218.0, 0.0, 0.0]),
        end: Vec3([16_777_218.0, 0.0, 0.0]),
        mins: Vec3([-1.0, 0.0, 0.0]),
        maxs: Vec3([3.0, 0.0, 0.0]),
        mask: Contents::EMPTY,
        rules: qa_world::collision::TraceRules {
            bounds_origin: qa_world::collision::BoundsOrigin::Centered,
            ..base.rules
        },
        ..base
    };
    // Native centering adds 1 to the origin, rounding it to 16_777_220,
    // and changes the X bounds to [-2,2]. That closed shape touches WATER.
    let mut scratch = store.scratch();
    let direct = store.trace_model(geometry, 1, query, &mut scratch).unwrap();
    assert_eq!((direct.in_open, direct.in_water), (false, true));
    let split = store.trace_model(geometry, 0, query, &mut scratch).unwrap();
    same_contact(split, direct);
    assert_eq!((split.in_open, split.in_water), (false, true));
}
