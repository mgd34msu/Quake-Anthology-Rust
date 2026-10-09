use qa_core::primitives::{
    Axis, Bounds, ClipNode, GeometryId, ModelRules, Plane, SurfaceFlags, Vec3,
};
use qa_world::collision::{
    CollisionStore, Contents, EntityTracePolicy, StoreError, TraceQuery, TraceRules,
    brushes::{Brush, BrushTree, CollisionLeaf, GeometryError, ModelRoot},
    hulls::{HullError, HullModel},
};

fn wall_plane() -> Plane {
    Plane {
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
        vec![HullModel { roots: [0; 3] }],
        vec![bounds()],
    )
}

fn add_brush(store: &mut CollisionStore) -> Result<GeometryId, StoreError> {
    store.load_brushes(
        vec![wall_plane()],
        vec![Brush {
            first_plane: 0,
            plane_count: 1,
            contents: Contents::SOLID,
        }],
        vec![SurfaceFlags(77)],
        BrushTree {
            planes: Vec::new(),
            nodes: Vec::new(),
            leaves: vec![
                CollisionLeaf {
                    stored_contents: None,
                    first_brush: 0,
                    brush_count: 0,
                },
                CollisionLeaf {
                    stored_contents: None,
                    first_brush: 0,
                    brush_count: 1,
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
        let hull_hit = store.trace_model(hull, 0, query, &mut first);
        let brush_hit = store.trace_model(brush, 1, query, &mut second);
        assert_eq!(hull_hit.fraction, fraction);
        assert_eq!(brush_hit.fraction, fraction);
        assert_eq!(brush_hit.surface, SurfaceFlags(77));
        assert_eq!(store.trace_model(brush, 0, query, &mut first).fraction, 1.0);
        assert_eq!(
            store
                .trace_model(brush, 1, query, &mut first)
                .fraction
                .to_bits(),
            brush_hit.fraction.to_bits()
        );
        assert_eq!(
            store
                .trace_model(hull, 0, query, &mut second)
                .end
                .0
                .map(f32::to_bits),
            hull_hit.end.0.map(f32::to_bits)
        );
        assert_eq!(
            store.point_contents_model(brush, 0, query.end, entities),
            Contents::EMPTY
        );
        assert_eq!(
            store.point_contents_model(brush, 1, query.end, entities),
            Contents::SOLID
        );
        assert_eq!(
            store.point_contents_model(hull, 0, query.end, entities),
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
    let before = store.trace_model(brush, 1, query, &mut scratch);
    assert!(store.remove(old_hull));
    assert!(!store.remove(old_hull));
    assert_eq!(store.model_count(old_hull), None);
    assert_eq!(store.model_bounds(old_hull, 0), None);
    assert_eq!(
        store.trace_model(old_hull, 0, query, &mut scratch).fraction,
        1.0
    );
    assert_eq!(
        store.point_contents_model(
            old_hull,
            0,
            query.end,
            qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake3).1
        ),
        Contents::EMPTY
    );
    let after = store.trace_model(brush, 1, query, &mut scratch);
    assert_eq!(after.fraction.to_bits(), before.fraction.to_bits());
    assert_eq!(
        after.end.0.map(f32::to_bits),
        before.end.0.map(f32::to_bits)
    );
    assert_eq!(after.surface, before.surface);
    assert_eq!(store.model_bounds(brush, 1), Some(bounds()));
    let new_hull = add_hull(&mut store)?;
    assert_eq!(new_hull.slot, old_hull.slot);
    assert_eq!(new_hull.generation, old_hull.generation + 1);
    assert_eq!(
        store.trace_model(old_hull, 0, query, &mut scratch).fraction,
        1.0
    );
    assert_eq!(
        store.trace_model(new_hull, 0, query, &mut scratch).fraction,
        0.4375
    );
    assert_eq!(
        store.trace_model(brush, 2, query, &mut scratch).fraction,
        1.0
    );
    assert_eq!(store.model_bounds(brush, 2), None);
    assert!(store.remove(brush));
    assert_eq!(
        store.trace_model(new_hull, 0, query, &mut scratch).fraction,
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
    let before = [first, last].map(|geometry| store.trace_model(geometry, 0, query, &mut scratch));
    assert!(store.remove(middle));
    for (geometry, before) in [first, last].into_iter().zip(before) {
        let after = store.trace_model(geometry, 0, query, &mut scratch);
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
            BrushTree::direct(0).map_err(StoreError::Brush)?,
            Vec::new()
        ),
        Err(StoreError::Models)
    ));
    let mut bad_tree = BrushTree::direct(0).map_err(StoreError::Brush)?;
    bad_tree.models[0] = ModelRoot::Leaf(1);
    assert!(matches!(
        store.load_brushes(Vec::new(), Vec::new(), Vec::new(), bad_tree, vec![bounds()]),
        Err(StoreError::Brush(GeometryError::TreeRange))
    ));
    assert!(matches!(
        store.load_hulls(
            Vec::new(),
            Vec::new(),
            Vec::new(),
            vec![HullModel { roots: [0; 3] }],
            vec![bounds()]
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
            vec![HullModel { roots: [0; 3] }],
            vec![bounds()]
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
            HullModel { roots: [-1; 3] },
            HullModel { roots: [-2; 3] },
            HullModel { roots: [-3; 3] },
        ],
        vec![bounds(); 3],
    )?;
    let query = crossing(
        TraceRules::LEGACY,
        qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake).1,
    );
    assert_eq!(
        store
            .trace_model(terminal, 1, query, &mut old_scratch)
            .fraction,
        1.0
    );
    assert!(
        !store
            .trace_model(terminal, 1, query, &mut old_scratch)
            .start_solid
    );
    let mut scratch = store.scratch();
    let empty = store.trace_model(terminal, 0, query, &mut scratch);
    assert_eq!(empty.fraction, 1.0);
    assert!(empty.in_open && !empty.all_solid);
    let solid = store.trace_model(terminal, 1, query, &mut scratch);
    assert!(solid.start_solid && solid.all_solid);
    assert_eq!(solid.fraction, 1.0);
    assert_eq!(
        store.point_contents_model(
            terminal,
            2,
            Vec3::default(),
            qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake3).1
        ),
        Contents::WATER
    );
    let brush = add_brush(&mut store)?;
    // The previously provisioned workspace cannot gain stamp/DFS storage while
    // tracing a subsequently loaded resource. The caller provisions it cold.
    let clear = store.trace_model(brush, 1, query, &mut scratch);
    assert_eq!(clear.fraction, 1.0);
    assert_eq!(clear.end.0.map(f32::to_bits), query.end.0.map(f32::to_bits));
    let mut refreshed = store.scratch();
    assert_eq!(
        store.trace_model(brush, 1, query, &mut refreshed).fraction,
        0.484375
    );
    assert!(
        store
            .trace_model(terminal, 1, query, &mut refreshed)
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
    let hit = store.trace_transformed(
        geometry,
        0,
        query,
        Vec3([-16_777_215.0, 0.0, 0.0]),
        Vec3::default(),
        ModelRules::default(),
        &mut scratch,
    );
    assert_eq!(hit.fraction, 0.484375);
    assert_eq!(hit.end.0[0], 1.03125);
    let miss = TraceQuery {
        end: Vec3([4.0, -0.0, 0.0]),
        ..query
    };
    let clear = store.trace_transformed(
        geometry,
        0,
        miss,
        Vec3([-16_777_215.0, 0.0, 0.0]),
        Vec3::default(),
        ModelRules::default(),
        &mut scratch,
    );
    assert_eq!(clear.fraction, 1.0);
    assert_eq!(clear.end.0.map(f32::to_bits), miss.end.0.map(f32::to_bits));
    Ok(())
}
