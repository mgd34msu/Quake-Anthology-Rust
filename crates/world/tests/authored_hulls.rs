use qa_core::primitives::{
    Bounds, Capsule, GeometryId, IndexSpan, ModelRotation, ModelRules, Plane, RuleSetId, Vec3,
};
use qa_world::collision::{
    CollisionStore, Contents, HullPositionLimits, StoreError, TraceError, TraceQuery,
    brushes::Brush,
    hulls::{HullError, HullModel},
    trace_policy,
};

fn bounds(mins: [f32; 3], maxs: [f32; 3]) -> Bounds {
    Bounds {
        mins: Vec3(mins),
        maxs: Vec3(maxs),
    }
}

fn append_brush(planes: &mut Vec<Plane>, bounds: Bounds, contents: Contents) -> Brush {
    let first = planes.len() as u32;
    for axis in 0..3 {
        planes.push(Plane::oriented(Vec3::axis(axis, 1.0), bounds.maxs.0[axis]));
    }
    for axis in 0..3 {
        planes.push(Plane::oriented(
            Vec3::axis(axis, -1.0),
            -bounds.mins.0[axis],
        ));
    }
    Brush {
        sides: IndexSpan { first, count: 6 },
        contents,
    }
}

fn load(brush_bounds: &[(Bounds, Contents)]) -> (CollisionStore, GeometryId) {
    let mut store = CollisionStore::new();
    let mut planes = Vec::new();
    let brushes = brush_bounds
        .iter()
        .map(|(bounds, contents)| append_brush(&mut planes, *bounds, *contents))
        .collect::<Vec<_>>();
    let geometry = store
        .load_hulls(
            planes,
            Vec::new(),
            Vec::new(),
            vec![HullModel {
                // All compiled hulls deliberately claim solid everywhere.
                roots: [-2; 3],
                brushes: IndexSpan {
                    first: 0,
                    count: brushes.len() as u32,
                },
            }],
            vec![bounds([-1024.0; 3], [1024.0; 3])],
            Some(brushes),
            HullPositionLimits::default(),
        )
        .expect("authored geometry admission");
    (store, geometry)
}

fn query(start: Vec3, end: Vec3, rules: RuleSetId) -> TraceQuery<'static> {
    let (trace, entity) = trace_policy(rules);
    TraceQuery::point(start, end, trace, entity)
}

#[test]
fn quake_contents_share_known_values_and_explicit_unknown_policy() {
    for (raw, expected) in [
        (-1, Contents::EMPTY),
        (-15, Contents::EMPTY),
        (-8, Contents::VOLUME_CLIP),
        (-16, Contents::LADDER),
        (-17, Contents::MONSTER_CLIP),
        (-18, Contents::PLAYER_CLIP),
        (-19, Contents::CORPSE),
    ] {
        for unknown in [Contents::EMPTY, Contents::SOLID] {
            assert_eq!(Contents::from_q1(raw, unknown), expected);
        }
    }
    for raw in [-32768, -20, -7, 0, 32767] {
        assert_eq!(Contents::from_q1(raw, Contents::EMPTY), Contents::EMPTY);
        assert_eq!(Contents::from_q1(raw, Contents::SOLID), Contents::SOLID);
    }
    for (raw, mask, excluded) in [
        (-17, Contents::MONSTER_CLIP, Contents::PLAYER_CLIP),
        (-18, Contents::PLAYER_CLIP, Contents::MONSTER_CLIP),
        (-19, Contents::CORPSE, Contents::SOLID),
        (-20, Contents::SOLID, Contents::LADDER),
    ] {
        let contents = Contents::from_q1(raw, Contents::SOLID);
        let (store, geometry) = load(&[(bounds([-1.0; 3], [1.0; 3]), contents)]);
        let point = query(Vec3::default(), Vec3::default(), RuleSetId::Quake);
        assert_eq!(
            store
                .point_contents_model(geometry, 0, Vec3::default(), point.entity_rules)
                .unwrap(),
            contents
        );
        let hit = store
            .trace_model(
                geometry,
                0,
                TraceQuery { mask, ..point },
                &mut store.scratch(),
            )
            .unwrap();
        assert!(hit.start_solid && hit.all_solid, "raw {raw}");
        assert_eq!(hit.contents, contents);
        assert_eq!(
            store
                .trace_model(
                    geometry,
                    0,
                    TraceQuery {
                        mask: excluded,
                        ..point
                    },
                    &mut store.scratch()
                )
                .unwrap()
                .fraction,
            1.0
        );
    }
}

#[test]
fn authored_source_overrides_compiled_hulls_for_every_caller_policy() {
    let (store, geometry) = load(&[(bounds([0.0, -4.0, -4.0], [4.0; 3]), Contents::SOLID)]);
    let mut scratch = store.scratch();
    for rules in RuleSetId::ALL {
        let outside = query(Vec3([-4.0, 0.0, 0.0]), Vec3([-3.0, 0.0, 0.0]), rules);
        let clear = store
            .trace_model(geometry, 0, outside, &mut scratch)
            .unwrap();
        assert_eq!(clear.fraction, 1.0, "{rules:?}");
        assert!(!clear.start_solid && !clear.all_solid);
        assert_eq!(
            store
                .point_contents_model(geometry, 0, outside.start, outside.entity_rules)
                .unwrap(),
            Contents::EMPTY
        );
        let crossing = TraceQuery {
            mins: Vec3([-1.0; 3]),
            maxs: Vec3([1.0; 3]),
            ..query(Vec3([-4.0, 0.0, 0.0]), Vec3([8.0, 0.0, 0.0]), rules)
        };
        let hit = store
            .trace_model(geometry, 0, crossing, &mut scratch)
            .unwrap();
        assert!(
            hit.fraction > 0.23 && hit.fraction < 0.25,
            "{rules:?}: {hit:?}"
        );
        assert_eq!(hit.plane.normal, Vec3::axis(0, -1.0));
        assert_eq!(hit.contents, Contents::SOLID);
        assert!(hit.surface_id.is_none());
        assert_eq!(
            store
                .point_contents_model(geometry, 0, Vec3([2.0, 0.0, 0.0]), outside.entity_rules)
                .unwrap(),
            Contents::SOLID
        );
    }
    let mut compiled = CollisionStore::new();
    let id = compiled
        .load_hulls(
            Vec::new(),
            Vec::new(),
            Vec::new(),
            vec![HullModel {
                roots: [-2; 3],
                brushes: IndexSpan::default(),
            }],
            vec![bounds([-1.0; 3], [1.0; 3])],
            None,
            HullPositionLimits::default(),
        )
        .unwrap();
    let native = query(
        Vec3([-4.0, 0.0, 0.0]),
        Vec3([-3.0, 0.0, 0.0]),
        RuleSetId::Quake,
    );
    assert!(
        compiled
            .trace_model(id, 0, native, &mut compiled.scratch())
            .unwrap()
            .all_solid
    );
}

#[test]
fn authoritative_empty_and_unnamed_models_never_fall_back() {
    let mut store = CollisionStore::new();
    let geometry = store
        .load_hulls(
            Vec::new(),
            Vec::new(),
            Vec::new(),
            vec![
                HullModel {
                    roots: [-2; 3],
                    brushes: IndexSpan::default()
                };
                2
            ],
            vec![bounds([-1.0; 3], [1.0; 3]); 2],
            Some(Vec::new()),
            HullPositionLimits::default(),
        )
        .unwrap();
    let mut scratch = store.scratch();
    for model in 0..2 {
        for rules in RuleSetId::ALL {
            let point = query(Vec3::default(), Vec3([1.0; 3]), rules);
            for capsule in [
                None,
                Some(Capsule {
                    radius: 1.0,
                    offset: Vec3::axis(2, 2.0),
                }),
            ] {
                let trace = store
                    .trace_model(
                        geometry,
                        model,
                        TraceQuery { capsule, ..point },
                        &mut scratch,
                    )
                    .unwrap();
                assert_eq!(trace.fraction, 1.0);
                assert!(!trace.start_solid && !trace.all_solid);
            }
            assert_eq!(
                store
                    .point_contents_model(geometry, model, Vec3::default(), point.entity_rules)
                    .unwrap(),
                Contents::EMPTY
            );
        }
    }
}

#[test]
fn authored_admission_does_not_require_unavailable_compiled_roots() {
    let mut store = CollisionStore::new();
    let models = vec![HullModel {
        roots: [-1, 500, 700],
        brushes: IndexSpan::default(),
    }];
    let model_bounds = vec![bounds([-1.0; 3], [1.0; 3])];
    assert_eq!(
        store.load_hulls(
            Vec::new(),
            Vec::new(),
            Vec::new(),
            models.clone(),
            model_bounds.clone(),
            None,
            HullPositionLimits::default()
        ),
        Err(StoreError::Hull(HullError::Root))
    );
    let geometry = store
        .load_hulls(
            Vec::new(),
            Vec::new(),
            Vec::new(),
            models,
            model_bounds,
            Some(Vec::new()),
            HullPositionLimits::default(),
        )
        .unwrap();
    assert_eq!(geometry.slot, 0);
    assert_eq!(
        store
            .trace_model(
                geometry,
                0,
                TraceQuery {
                    mins: Vec3([-16.0, -16.0, -24.0]),
                    maxs: Vec3([16.0, 16.0, 32.0]),
                    ..query(Vec3::default(), Vec3([10.0, 0.0, 0.0]), RuleSetId::Quake)
                },
                &mut store.scratch()
            )
            .unwrap()
            .fraction,
        1.0
    );
}

#[test]
fn clip_uses_executed_size_and_preserves_explicit_point_masks_and_observation() {
    let (store, geometry) = load(&[(
        bounds([-1.0; 3], [1.0; 3]),
        Contents::from_q1(-8, Contents::SOLID),
    )]);
    let mut scratch = store.scratch();
    for rules in RuleSetId::ALL {
        for moving in [false, true] {
            let point = if moving {
                query(Vec3([-4.0, 0.0, 0.0]), Vec3([4.0, 0.0, 0.0]), rules)
            } else {
                query(Vec3::default(), Vec3::default(), rules)
            };
            let clear = store.trace_model(geometry, 0, point, &mut scratch).unwrap();
            assert_eq!(clear.fraction, 1.0);
            assert!(!clear.start_solid && !clear.all_solid);
            let explicit = store
                .trace_model(
                    geometry,
                    0,
                    TraceQuery {
                        mask: Contents::VOLUME_CLIP,
                        ..point
                    },
                    &mut scratch,
                )
                .unwrap();
            assert!(explicit.fraction < 1.0);
            assert_eq!(explicit.contents, Contents::VOLUME_CLIP);
            // Constructed as a point, then given asymmetric bounds: no cached
            // constructor classification may hide its executed shape.
            let sized = store
                .trace_model(
                    geometry,
                    0,
                    TraceQuery {
                        mins: Vec3([0.0; 3]),
                        maxs: Vec3([0.5; 3]),
                        ..point
                    },
                    &mut scratch,
                )
                .unwrap();
            assert!(sized.fraction < 1.0);
            assert_eq!(sized.contents, Contents::VOLUME_CLIP);
            let degenerate = store
                .trace_model(
                    geometry,
                    0,
                    TraceQuery {
                        mins: Vec3([0.25; 3]),
                        maxs: Vec3([0.25; 3]),
                        ..point
                    },
                    &mut scratch,
                )
                .unwrap();
            assert_eq!(degenerate.fraction, 1.0);
        }
    }
    assert_eq!(
        store
            .point_contents_model(
                geometry,
                0,
                Vec3::default(),
                trace_policy(RuleSetId::Quake).1
            )
            .unwrap(),
        Contents::VOLUME_CLIP
    );
    assert_eq!(Contents::VOLUME_CLIP.to_q1(), -1);
    assert_eq!(Contents::VOLUME_CLIP.to_q2(), 0x30000);
    assert_eq!(Contents::VOLUME_CLIP.to_q3(), 0x30000);
    assert!(!Contents::from_q2(u32::MAX).intersects(Contents::VOLUME_CLIP));
}

#[test]
fn common_contents_observes_overlapping_ladder_fluid_and_clip() {
    let shape = bounds([-1.0; 3], [1.0; 3]);
    let (store, geometry) = load(&[
        (shape, Contents::from_q1(-16, Contents::SOLID)),
        (shape, Contents::WATER),
        (shape, Contents::VOLUME_CLIP),
    ]);
    let point = query(Vec3::default(), Vec3::default(), RuleSetId::Quake);
    let contents = store
        .point_contents_model(geometry, 0, Vec3::default(), point.entity_rules)
        .unwrap();
    assert_eq!(
        contents,
        Contents::LADDER | Contents::WATER | Contents::VOLUME_CLIP
    );
    assert_eq!(contents.to_q1(), -3);
    assert_eq!(contents.to_q2(), 0x20030020);
    assert_eq!(contents.to_q3(), 0x30020);
    let ladder = store
        .trace_model(
            geometry,
            0,
            TraceQuery {
                mask: Contents::LADDER,
                ..point
            },
            &mut store.scratch(),
        )
        .unwrap();
    assert!(ladder.all_solid);
    assert_eq!(ladder.contents, Contents::LADDER);
    assert_eq!(
        store
            .trace_model(geometry, 0, point, &mut store.scratch())
            .unwrap()
            .fraction,
        1.0
    );
}

#[test]
fn authored_capsule_retains_true_corner_distance_and_tilted_segment() {
    let (store, geometry) = load(&[(bounds([-1.0; 3], [1.0; 3]), Contents::SOLID)]);
    let mut scratch = store.scratch();
    for (center, offset, radius, solid) in [
        (Vec3([1.75, 1.75, 0.0]), Vec3::default(), 1.0, false),
        (Vec3([1.5, 1.5, 0.0]), Vec3::default(), 1.0, true),
        (Vec3([3.0, 3.0, 0.0]), Vec3([2.0, 2.0, 0.0]), 0.25, true),
    ] {
        let query = TraceQuery {
            capsule: Some(Capsule { radius, offset }),
            ..query(center, center, RuleSetId::Quake)
        };
        assert_eq!(
            store
                .trace_model(geometry, 0, query, &mut scratch)
                .unwrap()
                .all_solid,
            solid
        );
    }
    let sweep = TraceQuery {
        capsule: Some(Capsule {
            radius: 1.0,
            offset: Vec3::axis(2, 0.5),
        }),
        ..query(
            Vec3([-4.0, 0.0, 0.0]),
            Vec3([4.0, 0.0, 0.0]),
            RuleSetId::Quake3,
        )
    };
    let hit = store.trace_model(geometry, 0, sweep, &mut scratch).unwrap();
    assert!(hit.fraction > 0.2 && hit.fraction < 0.25);
    assert_eq!(hit.plane.normal, Vec3::axis(0, -1.0));
}

#[test]
fn authored_inline_uses_selected_pose_and_asymmetric_bounds() {
    let (store, geometry) = load(&[(
        bounds([-4.0, -0.25, -0.25], [4.0, 0.25, 0.25]),
        Contents::SOLID,
    )]);
    let origin = Vec3([10.0, 20.0, 30.0]);
    let angles = Vec3([0.0, 90.0, 0.0]);
    let model = ModelRules {
        rotation: ModelRotation::NegativeEuler,
        ..Default::default()
    };
    let mut scratch = store.scratch();
    for (offset, solid) in [
        (Vec3([0.0, 3.0, 0.0]), true),
        (Vec3([3.0, 0.0, 0.0]), false),
    ] {
        let point = origin + offset;
        let query = TraceQuery {
            mins: Vec3([0.0; 3]),
            maxs: Vec3([0.1; 3]),
            ..query(point, point, RuleSetId::Quake2)
        };
        assert_eq!(
            store
                .trace_transformed(geometry, 0, query, origin, angles, model, &mut scratch)
                .unwrap()
                .all_solid,
            solid
        );
        assert_eq!(
            store
                .point_contents_transformed(
                    geometry,
                    0,
                    point,
                    origin,
                    angles,
                    model,
                    query.entity_rules
                )
                .unwrap(),
            if solid {
                Contents::SOLID
            } else {
                Contents::EMPTY
            }
        );
    }
}

#[test]
fn authored_union_covers_touching_brushes_without_internal_contacts() {
    let (store, geometry) = load(&[
        (
            bounds([-3.0, -1.0, -1.0], [-1.0, 1.0, 1.0]),
            Contents::SOLID,
        ),
        (bounds([-1.0; 3], [2.0, 1.0, 1.0]), Contents::SOLID),
    ]);
    let query = query(
        Vec3([-2.0, 0.0, 0.0]),
        Vec3([1.0, 0.0, 0.0]),
        RuleSetId::Quake3,
    );
    let trace = store
        .trace_model(geometry, 0, query, &mut store.scratch())
        .unwrap();
    assert!(trace.start_solid && trace.all_solid);
    assert_eq!(trace.fraction, 0.0);
}

#[test]
fn rejected_authored_admission_publishes_no_resource_or_model_rows() {
    let mut store = CollisionStore::new();
    let mut planes = Vec::new();
    let brush = append_brush(&mut planes, bounds([-1.0; 3], [1.0; 3]), Contents::SOLID);
    for limits in [
        HullPositionLimits {
            cells: 0,
            ..Default::default()
        },
        HullPositionLimits {
            sides: 0,
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
    ] {
        assert_eq!(
            store.load_hulls(
                planes.clone(),
                Vec::new(),
                Vec::new(),
                vec![HullModel {
                    roots: [-1; 3],
                    brushes: IndexSpan { first: 0, count: 1 }
                }],
                vec![bounds([-1.0; 3], [1.0; 3])],
                Some(vec![brush]),
                limits
            ),
            Err(StoreError::Capacity)
        );
    }
    let id = store
        .load_hulls(
            planes,
            Vec::new(),
            Vec::new(),
            vec![HullModel {
                roots: [-1; 3],
                brushes: IndexSpan { first: 0, count: 1 },
            }],
            vec![bounds([-1.0; 3], [1.0; 3])],
            Some(vec![brush]),
            HullPositionLimits::default(),
        )
        .unwrap();
    assert_eq!(id.slot, 0);
    assert_eq!(store.model_count(id), Some(1));
}

#[test]
fn authored_trace_requires_scratch_admitted_after_geometry() {
    let mut store = CollisionStore::new();
    let mut old = store.scratch();
    let mut planes = Vec::new();
    let brush = append_brush(&mut planes, bounds([-1.0; 3], [1.0; 3]), Contents::SOLID);
    let id = store
        .load_hulls(
            planes,
            Vec::new(),
            Vec::new(),
            vec![HullModel {
                roots: [-1; 3],
                brushes: IndexSpan { first: 0, count: 1 },
            }],
            vec![bounds([-1.0; 3], [1.0; 3])],
            Some(vec![brush]),
            HullPositionLimits::default(),
        )
        .unwrap();
    assert!(matches!(
        store.trace_model(
            id,
            0,
            query(Vec3::default(), Vec3::default(), RuleSetId::Quake),
            &mut old
        ),
        Err(TraceError::Scratch)
    ));
}
