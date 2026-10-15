use qa_core::primitives::{
    Axis, Bounds, ClipNode, GeometryId, ModelRules, Plane, SurfaceFlags, Vec3,
};
use qa_world::collision::{
    CollisionStore, Contents, EntityTracePolicy, StoreError, TraceError, TraceQuery, TraceRules,
    brushes::{Brush, BrushTree, CollisionLeaf, GeometryError, ModelRoot},
    hulls::{HullError, HullModel},
    surfaces::{SurfaceInput, SurfaceTable},
};

#[test]
fn contact_rows_keep_exact_names_native_fields_and_resource_lifetimes() -> Result<(), StoreError> {
    let mut store = CollisionStore::new();
    let mut load = |name: &'static [u8], value| {
        store.load_brushes(
            vec![wall_plane()],
            vec![Brush {
                sides: qa_core::primitives::IndexSpan { first: 0, count: 1 },
                contents: Contents::SOLID,
            }],
            Vec::new(),
            SurfaceTable {
                records: vec![SurfaceInput {
                    name,
                    material: b"metal",
                    flags: SurfaceFlags::from_q2(0x80000083),
                    value,
                    source_index: 19,
                }],
                sides: vec![0],
            },
            BrushTree::direct(1).map_err(StoreError::Brush)?,
            vec![bounds()],
        )
    };
    let first = load(b"Wall_A", 37)?;
    let second = load(b"wall_a", -12)?;
    let mut scratch = store.scratch();
    let query = crossing(
        TraceRules::LEGACY,
        qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake2).1,
    );
    let hit = store
        .trace_model(first, 0, query, &mut scratch)
        .expect("valid collision query");
    assert_eq!(hit.fraction.to_bits(), ((1.0f32 - 0.03125) / 2.0).to_bits());
    let contact = hit.surface_id.expect("loaded contact");
    assert_eq!(contact.geometry, first);
    assert_eq!(contact.index, 0);
    let row = store.surface(contact).expect("surface row");
    assert_eq!(row.name, b"Wall_A");
    assert_eq!(row.material, b"metal");
    assert_eq!(row.value, 37);
    assert_eq!(row.source_index, 19);
    assert_eq!(row.flags.to_q2(), 0x80000083);
    let other = store
        .trace_model(second, 0, query, &mut scratch)
        .expect("valid collision query")
        .surface_id
        .expect("second contact");
    assert_eq!(store.surface(other).expect("other row").name, b"wall_a");
    assert_eq!(store.surface(other).expect("other row").value, -12);
    assert!(store.remove(first));
    assert!(store.surface(contact).is_none());
    assert_eq!(store.surface_count(first), None);
    assert!(store.surface(other).is_some());
    let replacement = add_brush(&mut store)?;
    assert_eq!(replacement.slot, first.slot);
    assert_ne!(replacement.generation, first.generation);
    assert!(store.surface(contact).is_none());
    Ok(())
}

#[test]
fn invalid_surface_indices_do_not_publish_geometry() -> Result<(), StoreError> {
    let mut store = CollisionStore::new();
    for sides in [vec![], vec![1]] {
        assert_eq!(
            store.load_brushes(
                vec![wall_plane()],
                Vec::new(),
                Vec::new(),
                SurfaceTable {
                    records: vec![SurfaceInput::unnamed(SurfaceFlags::default())],
                    sides
                },
                BrushTree::direct(0).map_err(StoreError::Brush)?,
                vec![bounds()],
            ),
            Err(StoreError::Brush(GeometryError::Surface))
        );
    }
    assert_eq!(add_brush(&mut store)?.slot, 0);
    Ok(())
}

fn wall_plane() -> Plane {
    Plane {
        encoding: None,
        normal: Vec3([1.0, 0.0, 0.0]),
        distance: 0.0,
        axis: Some(Axis::X),
    }
}

fn bounds() -> Bounds {
    // Authored bounds already carry their loader's native one-unit margin.
    Bounds {
        mins: Vec3([-17.0, -9.0, -5.0]),
        maxs: Vec3([33.0, 25.0, 13.0]),
    }
}

fn add_hull(store: &mut CollisionStore) -> Result<GeometryId, StoreError> {
    let node = ClipNode {
        plane: 0,
        children: [-1, -2],
    };
    store.load_hulls(
        vec![wall_plane()],
        vec![node],
        vec![node],
        vec![HullModel {
            roots: [0; 3],
            brushes: Default::default(),
        }],
        vec![bounds()],
        None,
        Default::default(),
    )
}

fn add_brush(store: &mut CollisionStore) -> Result<GeometryId, StoreError> {
    store.load_brushes(
        vec![wall_plane()],
        vec![Brush {
            sides: qa_core::primitives::IndexSpan { first: 0, count: 1 },
            contents: Contents::SOLID,
        }],
        Vec::new(),
        qa_world::collision::surfaces::SurfaceTable::flags(vec![SurfaceFlags(77)]),
        BrushTree {
            leaf_patches: Default::default(),
            planes: Vec::new(),
            nodes: Vec::new(),
            leaves: vec![
                CollisionLeaf {
                    patches: Default::default(),
                    stored_contents: None,
                    brushes: qa_core::primitives::IndexSpan { first: 0, count: 0 },
                },
                CollisionLeaf {
                    patches: Default::default(),
                    stored_contents: None,
                    brushes: qa_core::primitives::IndexSpan { first: 0, count: 1 },
                },
            ],
            leaf_brushes: vec![0],
            models: vec![ModelRoot::Leaf(0), ModelRoot::Leaf(1)],
        },
        vec![bounds(); 2],
    )
}

fn crossing(rules: TraceRules, entities: EntityTracePolicy) -> TraceQuery<'static> {
    TraceQuery::point(
        Vec3([1.0, 0.0, 0.0]),
        Vec3([-1.0, 0.0, 0.0]),
        rules,
        entities,
    )
}

#[test]
fn mixed_topologies_keep_native_models_and_caller_rules_independent() -> Result<(), StoreError> {
    let mut store = CollisionStore::new();
    let hull = add_hull(&mut store)?;
    let brush = add_brush(&mut store)?;
    assert_eq!(store.model_count(hull), Some(1));
    assert_eq!(store.model_count(brush), Some(2));
    assert_eq!(store.model_bounds(hull, 0), Some(bounds()));
    assert_eq!(store.model_bounds(brush, 1), Some(bounds()));
    store.prepare_traces(TraceRules::ARENA, Default::default())?;
    let mut first = store.scratch();
    let mut second = store.scratch();
    for (rules, entities, fraction) in [
        (
            TraceRules::LEGACY,
            qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake2).1,
            0.484375,
        ),
        (
            TraceRules::ARENA,
            qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake3).1,
            0.4375,
        ),
    ] {
        let query = crossing(rules, entities);
        let hull_hit = store
            .trace_model(hull, 0, query, &mut first)
            .expect("valid collision query");
        let brush_hit = store
            .trace_model(brush, 1, query, &mut second)
            .expect("valid collision query");
        assert_eq!(hull_hit.fraction, fraction);
        assert_eq!(brush_hit.fraction, fraction);
        assert_eq!(brush_hit.surface, SurfaceFlags(77));
        assert_eq!(
            store
                .trace_model(brush, 0, query, &mut first)
                .expect("valid collision query")
                .fraction,
            1.0
        );
        assert_eq!(
            store
                .trace_model(brush, 1, query, &mut first)
                .expect("valid collision query")
                .fraction
                .to_bits(),
            brush_hit.fraction.to_bits()
        );
        assert_eq!(
            store
                .trace_model(hull, 0, query, &mut second)
                .expect("valid collision query")
                .end
                .0
                .map(f32::to_bits),
            hull_hit.end.0.map(f32::to_bits)
        );
        assert_eq!(
            store
                .point_contents_model(brush, 0, query.end, entities)
                .expect("valid collision query"),
            Contents::EMPTY
        );
        assert_eq!(
            store
                .point_contents_model(brush, 1, query.end, entities)
                .expect("valid collision query"),
            Contents::SOLID
        );
        assert_eq!(
            store
                .point_contents_model(hull, 0, query.end, entities)
                .expect("valid collision query"),
            Contents::SOLID
        );
    }
    Ok(())
}

#[test]
fn removal_compacts_private_rows_and_reuse_rejects_stale_handles() -> Result<(), StoreError> {
    let mut store = CollisionStore::new();
    let old_hull = add_hull(&mut store)?;
    let brush = add_brush(&mut store)?;
    let mut scratch = store.scratch();
    let query = crossing(
        TraceRules::ARENA,
        qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake3).1,
    );
    let before = store
        .trace_model(brush, 1, query, &mut scratch)
        .expect("valid collision query");
    assert!(store.remove(old_hull));
    assert!(!store.remove(old_hull));
    assert_eq!(store.model_count(old_hull), None);
    assert_eq!(store.model_bounds(old_hull, 0), None);
    assert_eq!(
        store
            .trace_model(old_hull, 0, query, &mut scratch)
            .unwrap_err(),
        TraceError::Geometry {
            geometry: old_hull,
            model: 0
        }
    );
    assert_eq!(
        store.point_contents_model(
            old_hull,
            0,
            query.end,
            qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake3).1
        ),
        Err(TraceError::Geometry {
            geometry: old_hull,
            model: 0
        })
    );
    let after = store
        .trace_model(brush, 1, query, &mut scratch)
        .expect("valid collision query");
    assert_eq!(after.fraction.to_bits(), before.fraction.to_bits());
    assert_eq!(
        after.end.0.map(f32::to_bits),
        before.end.0.map(f32::to_bits)
    );
    assert_eq!(after.surface, before.surface);
    assert_eq!(store.model_bounds(brush, 1), Some(bounds()));
    let new_hull = add_hull(&mut store)?;
    store.prepare_traces(TraceRules::ARENA, Default::default())?;
    scratch = store.scratch();
    assert_eq!(new_hull.slot, old_hull.slot);
    assert_eq!(new_hull.generation, old_hull.generation + 1);
    assert_eq!(
        store
            .trace_model(old_hull, 0, query, &mut scratch)
            .unwrap_err(),
        TraceError::Geometry {
            geometry: old_hull,
            model: 0
        }
    );
    assert_eq!(
        store
            .trace_model(new_hull, 0, query, &mut scratch)
            .expect("valid collision query")
            .fraction,
        0.4375
    );
    assert_eq!(
        store
            .trace_model(brush, 2, query, &mut scratch)
            .unwrap_err(),
        TraceError::Geometry {
            geometry: brush,
            model: 2
        }
    );
    assert_eq!(store.model_bounds(brush, 2), None);
    assert!(store.remove(brush));
    assert_eq!(
        store
            .trace_model(new_hull, 0, query, &mut scratch)
            .expect("valid collision query")
            .fraction,
        0.4375
    );
    Ok(())
}

#[test]
fn removing_the_middle_resource_preserves_both_neighbors() -> Result<(), StoreError> {
    let mut store = CollisionStore::new();
    let first = add_hull(&mut store)?;
    let middle = add_brush(&mut store)?;
    let last = add_hull(&mut store)?;
    let mut scratch = store.scratch();
    let query = crossing(
        TraceRules::LEGACY,
        qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake).1,
    );
    let before = [first, last].map(|geometry| {
        store
            .trace_model(geometry, 0, query, &mut scratch)
            .expect("valid collision query")
    });
    assert!(store.remove(middle));
    for (geometry, before) in [first, last].into_iter().zip(before) {
        let after = store
            .trace_model(geometry, 0, query, &mut scratch)
            .expect("valid collision query");
        assert_eq!(after.fraction.to_bits(), before.fraction.to_bits());
        assert_eq!(
            after.end.0.map(f32::to_bits),
            before.end.0.map(f32::to_bits)
        );
        assert_eq!(store.model_bounds(geometry, 0), Some(bounds()));
    }
    assert_eq!(store.model_count(middle), None);
    Ok(())
}

#[test]
fn malformed_cold_resources_preserve_registered_geometry() -> Result<(), StoreError> {
    let mut store = CollisionStore::new();
    let valid = add_brush(&mut store)?;
    for invalid in [
        Bounds {
            mins: Vec3([f32::NAN, 0.0, 0.0]),
            maxs: Vec3([1.0; 3]),
        },
        Bounds {
            mins: Vec3([2.0, 0.0, 0.0]),
            maxs: Vec3([1.0; 3]),
        },
        Bounds {
            mins: Vec3::default(),
            maxs: Vec3([f32::INFINITY; 3]),
        },
    ] {
        assert!(matches!(
            store.load_brushes(
                Vec::new(),
                Vec::new(),
                Vec::new(),
                qa_world::collision::surfaces::SurfaceTable::flags(Vec::new()),
                BrushTree::direct(0).map_err(StoreError::Brush)?,
                vec![invalid]
            ),
            Err(StoreError::Bounds)
        ));
        assert_eq!(store.model_count(valid), Some(2));
        assert_eq!(store.model_bounds(valid, 1), Some(bounds()));
    }
    assert!(matches!(
        store.load_brushes(
            Vec::new(),
            Vec::new(),
            Vec::new(),
            qa_world::collision::surfaces::SurfaceTable::flags(Vec::new()),
            BrushTree::direct(0).map_err(StoreError::Brush)?,
            Vec::new()
        ),
        Err(StoreError::Models)
    ));
    let mut bad_tree = BrushTree::direct(0).map_err(StoreError::Brush)?;
    bad_tree.models[0] = ModelRoot::Leaf(1);
    assert!(matches!(
        store.load_brushes(
            Vec::new(),
            Vec::new(),
            Vec::new(),
            qa_world::collision::surfaces::SurfaceTable::flags(Vec::new()),
            bad_tree,
            vec![bounds()]
        ),
        Err(StoreError::Brush(GeometryError::TreeRange))
    ));
    assert!(matches!(
        store.load_hulls(
            Vec::new(),
            Vec::new(),
            Vec::new(),
            vec![HullModel {
                roots: [0; 3],
                brushes: Default::default(),
            }],
            vec![bounds()],
            None,
            Default::default(),
        ),
        Err(StoreError::Hull(HullError::Root))
    ));
    let cycle = vec![ClipNode {
        plane: 0,
        children: [0, -1],
    }];
    assert!(matches!(
        store.load_hulls(
            vec![wall_plane()],
            cycle.clone(),
            cycle,
            vec![HullModel {
                roots: [0; 3],
                brushes: Default::default(),
            }],
            vec![bounds()],
            None,
            Default::default(),
        ),
        Err(StoreError::Hull(HullError::Cycle))
    ));
    let next = add_hull(&mut store)?;
    assert_eq!(next.slot, valid.slot + 1);
    assert_eq!(next.generation, 1);
    let mut scratch = store.scratch();
    assert_eq!(
        store
            .trace_model(
                valid,
                1,
                crossing(
                    TraceRules::LEGACY,
                    qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake2).1
                ),
                &mut scratch
            )
            .expect("valid collision query")
            .fraction,
        0.484375
    );
    Ok(())
}

#[test]
fn terminal_hull_models_and_stale_scratch_remain_scoped() -> Result<(), StoreError> {
    let mut store = CollisionStore::new();
    let mut old_scratch = store.scratch();
    let terminal = store.load_hulls(
        Vec::new(),
        Vec::new(),
        Vec::new(),
        vec![
            HullModel {
                roots: [-1; 3],
                brushes: Default::default(),
            },
            HullModel {
                roots: [-2; 3],
                brushes: Default::default(),
            },
            HullModel {
                roots: [-3; 3],
                brushes: Default::default(),
            },
        ],
        vec![bounds(); 3],
        None,
        Default::default(),
    )?;
    let query = crossing(
        TraceRules::LEGACY,
        qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake).1,
    );
    assert_eq!(
        store
            .trace_model(terminal, 1, query, &mut old_scratch)
            .unwrap_err(),
        TraceError::Scratch
    );
    let mut scratch = store.scratch();
    let empty = store
        .trace_model(terminal, 0, query, &mut scratch)
        .expect("valid collision query");
    assert_eq!(empty.fraction, 1.0);
    assert!(empty.in_open && !empty.all_solid);
    let solid = store
        .trace_model(terminal, 1, query, &mut scratch)
        .expect("valid collision query");
    assert!(solid.start_solid && solid.all_solid);
    assert_eq!(solid.fraction, 1.0);
    assert_eq!(
        store
            .point_contents_model(
                terminal,
                2,
                Vec3::default(),
                qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake3).1
            )
            .expect("valid collision query"),
        Contents::WATER
    );
    let brush = add_brush(&mut store)?;
    // The previously provisioned workspace cannot gain stamp/DFS storage while
    // tracing a subsequently loaded resource. The caller provisions it cold.
    assert_eq!(
        store
            .trace_model(brush, 1, query, &mut scratch)
            .unwrap_err(),
        TraceError::Scratch
    );
    let mut refreshed = store.scratch();
    assert_eq!(
        store
            .trace_model(brush, 1, query, &mut refreshed)
            .expect("valid collision query")
            .fraction,
        0.484375
    );
    assert!(
        store
            .trace_model(terminal, 1, query, &mut refreshed)
            .expect("valid collision query")
            .all_solid
    );
    Ok(())
}

#[test]
fn translated_hull_forms_the_whole_native_offset_before_subtraction() -> Result<(), StoreError> {
    let mut store = CollisionStore::new();
    let geometry = add_hull(&mut store)?;
    let mut scratch = store.scratch();
    let query = TraceQuery {
        mins: Vec3([-16_777_216.0, 0.0, 0.0]),
        maxs: Vec3([-16_777_214.0, 0.0, 0.0]),
        ..TraceQuery::point(
            Vec3([2.0, 0.0, 0.0]),
            Vec3::default(),
            TraceRules::LEGACY,
            qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake).1,
        )
    };
    // world.c forms (clipmins - mins) + origin = 1 first. The resulting
    // local segment is [1,-1]; subtracting origin and clipmins separately
    // loses that one-unit translation through f32 cancellation.
    let hit = store
        .trace_transformed(
            geometry,
            0,
            query,
            Vec3([-16_777_215.0, 0.0, 0.0]),
            Vec3::default(),
            ModelRules::default(),
            &mut scratch,
        )
        .expect("valid collision query");
    assert_eq!(hit.fraction, 0.484375);
    assert_eq!(hit.end.0[0], 1.03125);
    let miss = TraceQuery {
        end: Vec3([4.0, -0.0, 0.0]),
        ..query
    };
    let clear = store
        .trace_transformed(
            geometry,
            0,
            miss,
            Vec3([-16_777_215.0, 0.0, 0.0]),
            Vec3::default(),
            ModelRules::default(),
            &mut scratch,
        )
        .expect("valid collision query");
    assert_eq!(clear.fraction, 1.0);
    assert_eq!(clear.end.0.map(f32::to_bits), miss.end.0.map(f32::to_bits));
    Ok(())
}

#[test]
fn intentional_none_and_box_shapes_do_not_require_geometry_or_scratch() {
    use qa_core::primitives::{Body, CollisionShape};
    let store = CollisionStore::new();
    let mut scratch = store.scratch();
    let query = crossing(
        TraceRules::LEGACY,
        qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake2).1,
    );
    let body = Body {
        mins: Vec3([-0.5; 3]),
        maxs: Vec3([0.5; 3]),
        ..Default::default()
    };
    let clear = store
        .trace_body(
            query,
            CollisionShape::None,
            &body,
            Vec3::default(),
            ModelRules::default(),
            qa_world::collision::store::BodyTraceMode::Transformed,
            None,
            &mut scratch,
        )
        .expect("intentional no geometry");
    assert_eq!(clear.fraction.to_bits(), 1.0f32.to_bits());
    assert_eq!(clear.end.0.map(f32::to_bits), query.end.0.map(f32::to_bits));
    let expected = qa_world::collision::boxes::trace_box(
        query,
        &body,
        None,
        qa_world::collision::store::BodyTraceMode::Transformed,
    );
    let boxed = store
        .trace_body(
            query,
            CollisionShape::Box,
            &body,
            Vec3::default(),
            ModelRules::default(),
            qa_world::collision::store::BodyTraceMode::Transformed,
            None,
            &mut scratch,
        )
        .expect("temporary box");
    assert_eq!(boxed.fraction.to_bits(), expected.fraction.to_bits());
    assert_eq!(
        boxed.end.0.map(f32::to_bits),
        expected.end.0.map(f32::to_bits)
    );
    let geometry = GeometryId {
        slot: 0,
        generation: 1,
    };
    assert_eq!(
        store
            .trace_body(
                query,
                CollisionShape::Model { geometry, index: 3 },
                &body,
                Vec3::default(),
                ModelRules::default(),
                qa_world::collision::store::BodyTraceMode::Transformed,
                None,
                &mut scratch
            )
            .unwrap_err(),
        TraceError::Geometry { geometry, model: 3 }
    );
    assert_eq!(
        store.point_contents_transformed(
            geometry,
            3,
            Vec3::default(),
            Vec3::default(),
            Vec3::default(),
            ModelRules::default(),
            query.entity_rules
        ),
        Err(TraceError::Geometry { geometry, model: 3 })
    );
}

#[test]
fn direct_body_model_queries_keep_the_selected_native_model_namespace() -> Result<(), StoreError> {
    use qa_core::primitives::{Body, CollisionShape, RuleSetId};
    use qa_world::collision::store::BodyTraceMode;
    let mut store = CollisionStore::new();
    let geometry = add_brush(&mut store)?;
    let hull = add_hull(&mut store)?;
    assert!(!store.has_bsp_nodes(geometry).unwrap());
    assert!(store.has_bsp_nodes(hull).unwrap());
    let mut scratch = store.scratch();
    let (rules, caller) = qa_world::collision::trace_policy(RuleSetId::Quake3);
    let query = crossing(rules, caller);
    let body = Body {
        position: Vec3([10.0, 0.0, 0.0]),
        ..Body::default()
    };
    // Model zero is intentionally empty; one owns the native wall brush.
    let shape = CollisionShape::Model { geometry, index: 1 };
    let direct = store
        .trace_body(
            query,
            shape,
            &body,
            Vec3([90.0; 3]),
            ModelRules::default(),
            BodyTraceMode::Direct,
            None,
            &mut scratch,
        )
        .unwrap();
    assert_eq!(direct.fraction.to_bits(), (7.0f32 / 16.0).to_bits());
    assert_eq!(direct.surface_id.unwrap().geometry, geometry);
    let transformed = store
        .trace_body(
            query,
            shape,
            &body,
            Vec3::default(),
            ModelRules::default(),
            BodyTraceMode::Transformed,
            None,
            &mut scratch,
        )
        .unwrap();
    assert!(transformed.start_solid && transformed.all_solid);
    for (mode, expected) in [
        (BodyTraceMode::Direct, Contents::EMPTY),
        (BodyTraceMode::Transformed, Contents::SOLID),
    ] {
        assert_eq!(
            store
                .point_contents_body(
                    Vec3([1.0, 0.0, 0.0]),
                    shape,
                    &body,
                    Vec3::default(),
                    ModelRules::default(),
                    caller,
                    mode,
                )
                .unwrap(),
            expected,
        );
    }
    assert!(store.remove(geometry));
    assert_eq!(
        store.has_bsp_nodes(geometry),
        Err(TraceError::Geometry { geometry, model: 0 }),
    );
    assert_eq!(
        store.point_contents_body(
            Vec3::default(),
            shape,
            &body,
            Vec3::default(),
            ModelRules::default(),
            caller,
            BodyTraceMode::Direct,
        ),
        Err(TraceError::Geometry { geometry, model: 1 }),
    );
    Ok(())
}
