use qa_core::primitives::ThinkTime;
use qa_core::primitives::{
    Axis, Body, Bounds, ClipNode, CollisionShape, EntityId, EntityPose, GeometryId, ModelRotation,
    ModelRules, ModuleId, Plane, RotatedLinkBounds, SurfaceFlags, Vec3,
};
use qa_world::{
    area::{AreaGrid, LinkFlags, LinkIntent, LinkOrder},
    collision::{
        CollisionStore, Contents, EntityTracePolicy, QuakeTraceKind, TraceQuery, TraceRules,
        WorldTrace,
        brushes::{Brush, BrushTree, CollisionLeaf, ModelRoot},
        hulls::HullModel,
    },
    entities::{AllocationPolicy, EntityTable},
};

fn table() -> (EntityTable, AreaGrid) {
    (
        EntityTable::new(16, 1).unwrap(),
        AreaGrid::load(
            16,
            Bounds {
                mins: Vec3([-1024.0; 3]),
                maxs: Vec3([1024.0; 3]),
            },
        )
        .unwrap(),
    )
}

fn empty(store: &mut CollisionStore) -> GeometryId {
    store
        .load_brushes(
            Vec::new(),
            Vec::new(),
            qa_world::collision::surfaces::SurfaceTable::flags(Vec::new()),
            BrushTree {
                planes: Vec::new(),
                nodes: Vec::new(),
                leaves: vec![CollisionLeaf {
                    stored_contents: Some(Contents::EMPTY),
                    first_brush: 0,
                    brush_count: 0,
                }],
                leaf_brushes: Vec::new(),
                models: vec![ModelRoot::Leaf(0)],
            },
            vec![Bounds {
                mins: Vec3([-512.0; 3]),
                maxs: Vec3([512.0; 3]),
            }],
        )
        .unwrap()
}

/// An empty world model and one inline convex box share immutable geometry.
fn brush_model(
    store: &mut CollisionStore,
    mins: Vec3,
    maxs: Vec3,
    contents: Contents,
) -> GeometryId {
    let planes = (0..6)
        .map(|side| {
            let axis = side / 2;
            let positive = side % 2 == 0;
            Plane {
                encoding: None,
                normal: Vec3(std::array::from_fn(|i| {
                    if i == axis {
                        if positive { 1.0 } else { -1.0 }
                    } else {
                        0.0
                    }
                })),
                distance: if positive {
                    maxs.0[axis]
                } else {
                    -mins.0[axis]
                },
                axis: positive.then_some([Axis::X, Axis::Y, Axis::Z][axis]),
            }
        })
        .collect();
    store
        .load_brushes(
            planes,
            vec![Brush {
                first_plane: 0,
                plane_count: 6,
                contents,
            }],
            qa_world::collision::surfaces::SurfaceTable::flags(vec![SurfaceFlags::default(); 6]),
            BrushTree {
                planes: Vec::new(),
                nodes: Vec::new(),
                leaves: vec![
                    CollisionLeaf {
                        stored_contents: Some(Contents::EMPTY),
                        first_brush: 0,
                        brush_count: 0,
                    },
                    CollisionLeaf {
                        // Analytic membership has no native BSP leaf contents;
                        // both caller families classify its convex sides.
                        stored_contents: None,
                        first_brush: 0,
                        brush_count: 1,
                    },
                ],
                leaf_brushes: vec![0],
                models: vec![ModelRoot::Leaf(0), ModelRoot::Leaf(1)],
            },
            vec![
                Bounds {
                    mins: Vec3([-512.0; 3]),
                    maxs: Vec3([512.0; 3]),
                },
                // Native map conversion, not the store or the link operation,
                // expands authored model bounds once (qsrc Q2 cmodel.c:159).
                Bounds {
                    mins: Vec3(mins.0.map(|v| v - 1.0)),
                    maxs: Vec3(maxs.0.map(|v| v + 1.0)),
                },
            ],
        )
        .unwrap()
}

fn hull_model(store: &mut CollisionStore) -> GeometryId {
    let node = ClipNode {
        plane: 0,
        children: [-1, -2],
    };
    store
        .load_hulls(
            vec![Plane {
                encoding: None,
                normal: Vec3([1.0, 0.0, 0.0]),
                distance: 0.0,
                axis: Some(Axis::X),
            }],
            vec![node],
            vec![node],
            vec![HullModel { roots: [-1; 3] }, HullModel { roots: [0; 3] }],
            vec![
                Bounds {
                    mins: Vec3([-512.0; 3]),
                    maxs: Vec3([512.0; 3]),
                },
                Bounds {
                    mins: Vec3([-64.0; 3]),
                    maxs: Vec3([64.0; 3]),
                },
            ],
        )
        .unwrap()
}

fn model(
    store: &CollisionStore,
    table: &mut EntityTable,
    area: &mut AreaGrid,
    geometry: GeometryId,
    position: Vec3,
    angles: Vec3,
    rules: ModelRules,
) -> EntityId {
    let id = table
        .allocate(
            ThinkTime::Seconds(0.0),
            ModuleId(7),
            AllocationPolicy::EDICT,
        )
        .unwrap()
        .id;
    let bounds = store.model_bounds(geometry, 1).unwrap();
    let slot = id.slot as usize;
    table.columns.set_body(
        slot,
        Body {
            position,
            mins: bounds.mins,
            maxs: bounds.maxs,
            ..Default::default()
        },
    );
    table.columns.collision_shape[slot] = CollisionShape::Model { geometry, index: 1 };
    table.columns.collision_contents[slot] = u64::MAX;
    table.columns.angles[slot] = angles;
    table.columns.model_rules[slot] = rules;
    assert!(area.link(
        table,
        id,
        LinkFlags::SOLID,
        LinkOrder::Tail,
        LinkIntent::Explicit
    ));
    id
}

fn query(start: Vec3, end: Vec3, rules: EntityTracePolicy) -> TraceQuery<'static> {
    TraceQuery::point(
        start,
        end,
        if rules == qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake3).1 {
            TraceRules::ARENA
        } else {
            TraceRules::LEGACY
        },
        rules,
    )
}

#[test]
fn explicit_world_geometry_and_inline_ordinal_do_not_default_to_model_zero() {
    let mut store = CollisionStore::new();
    let first = brush_model(&mut store, Vec3([-2.0; 3]), Vec3([2.0; 3]), Contents::SOLID);
    let second = brush_model(
        &mut store,
        Vec3([18.0, -2.0, -2.0]),
        Vec3([22.0, 2.0, 2.0]),
        Contents::WATER,
    );
    let (table, area) = table();
    let mut scratch = store.scratch();
    let input = TraceQuery {
        mask: Contents::SOLID | Contents::WATER,
        ..query(
            Vec3([50.0, 0.0, 0.0]),
            Vec3([-50.0, 0.0, 0.0]),
            qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake2).1,
        )
    };
    let mut scene = WorldTrace::new(&store, second, 1, &table, &area, &mut scratch, None);
    let result = scene.trace(input);
    assert_eq!(result.contents, Contents::WATER);
    assert_eq!(result.entity, table.id_at(0));
    assert_eq!(result.end.0[0].to_bits(), (22.0f32 + 0.03125).to_bits());
    assert_eq!(
        scene.point_contents(
            Vec3([20.0, 0.0, 0.0]),
            qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake2).1,
            &[]
        ),
        Contents::WATER
    );
    assert_eq!(
        scene.point_contents(
            Vec3::default(),
            qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake2).1,
            &[]
        ),
        Contents::EMPTY
    );
    let mut scene = WorldTrace::new(&store, second, 0, &table, &area, &mut scratch, None);
    assert_eq!(scene.trace(input).fraction, 1.0);
    let mut scene = WorldTrace::new(&store, first, 1, &table, &area, &mut scratch, None);
    assert_eq!(scene.trace(input).contents, Contents::SOLID);
}

#[test]
fn linked_hull_and_brush_models_share_filters_and_stale_handles_are_scoped() {
    let mut store = CollisionStore::new();
    let world = empty(&mut store);
    let hull = hull_model(&mut store);
    let brush = brush_model(&mut store, Vec3([-2.0; 3]), Vec3([2.0; 3]), Contents::SOLID);
    let (mut table, mut area) = table();
    let hull_entity = model(
        &store,
        &mut table,
        &mut area,
        hull,
        Vec3([10.0, 0.0, 0.0]),
        Vec3([35.0, 70.0, 20.0]),
        ModelRules::default(),
    );
    let brush_entity = model(
        &store,
        &mut table,
        &mut area,
        brush,
        Vec3([-10.0, 0.0, 0.0]),
        Vec3::default(),
        ModelRules::default(),
    );
    let box_entity = table
        .allocate(
            ThinkTime::Seconds(0.0),
            ModuleId(9),
            AllocationPolicy::EDICT,
        )
        .unwrap()
        .id;
    table.columns.set_body(
        box_entity.slot as usize,
        Body {
            position: Vec3([30.0, 0.0, 0.0]),
            mins: Vec3([-2.0; 3]),
            maxs: Vec3([2.0; 3]),
            ..Default::default()
        },
    );
    table.columns.collision_shape[box_entity.slot as usize] = CollisionShape::Box;
    table.columns.collision_contents[box_entity.slot as usize] = Contents::BODY.0;
    assert!(area.link(
        &table,
        box_entity,
        LinkFlags::SOLID,
        LinkOrder::Tail,
        LinkIntent::Explicit
    ));
    let mut scratch = store.scratch();
    let input = TraceQuery {
        mask: Contents::SOLID | Contents::BODY,
        ..query(
            Vec3([50.0, 0.0, 0.0]),
            Vec3([-50.0, 0.0, 0.0]),
            qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake)
                .1
                .with_quake_kind(Some(QuakeTraceKind::IgnoreBoxes)),
        )
    };
    let result = WorldTrace::new(&store, world, 0, &table, &area, &mut scratch, None).trace(input);
    assert_eq!(result.entity, Some(hull_entity));
    assert!(result.brush_solid);
    assert_eq!(result.end.0[0].to_bits(), (10.0f32 + 0.03125).to_bits());
    let excluded = [hull_entity];
    let result =
        WorldTrace::new(&store, world, 0, &table, &area, &mut scratch, None).trace(TraceQuery {
            excluded: &excluded,
            ..input
        });
    assert_eq!(result.entity, Some(brush_entity));
    assert!(store.remove(hull));
    let replacement = hull_model(&mut store);
    assert_ne!(replacement, hull);
    assert!(store.model_bounds(hull, 1).is_none());
    // The stale candidate cannot silently bind to the replacement geometry.
    let result = WorldTrace::new(&store, world, 0, &table, &area, &mut scratch, None).trace(input);
    assert_eq!(result.entity, Some(brush_entity));
    table.columns.collision_shape[brush_entity.slot as usize] = CollisionShape::Model {
        geometry: brush,
        index: u32::MAX,
    };
    let result = WorldTrace::new(&store, world, 0, &table, &area, &mut scratch, None).trace(input);
    assert_eq!(result.fraction, 1.0);
}

#[test]
fn target_role_selects_rotated_collision_and_contents_under_foreign_callers() {
    let mut store = CollisionStore::new();
    let world = empty(&mut store);
    let geometry = brush_model(
        &mut store,
        Vec3([-5.0, -1.0, -2.0]),
        Vec3([5.0, 1.0, 2.0]),
        Contents::WATER,
    );
    for (rotation, bounds) in [
        (ModelRotation::TranslationOnly, RotatedLinkBounds::Unrotated),
        (ModelRotation::NegativeEuler, RotatedLinkBounds::MaxAbsCube),
        (ModelRotation::TransposeBasis, RotatedLinkBounds::RadiusCube),
    ] {
        let (mut table, mut area) = table();
        let target = model(
            &store,
            &mut table,
            &mut area,
            geometry,
            Vec3([30.0, 40.0, 50.0]),
            Vec3([0.0, 90.0, 0.0]),
            ModelRules {
                rotation,
                link_bounds: bounds,
            },
        );
        let mut scratch = store.scratch();
        for caller in [
            qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake).1,
            qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake2).1,
            qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake3).1,
        ] {
            let mut scene = WorldTrace::new(&store, world, 0, &table, &area, &mut scratch, None);
            let result = scene.trace(TraceQuery {
                mask: Contents::WATER,
                ..query(Vec3([40.0, 40.0, 50.0]), Vec3([20.0, 40.0, 50.0]), caller)
            });
            assert_eq!(result.entity, Some(target));
            assert_eq!(result.contents, Contents::WATER);
            let edge = if rotation == ModelRotation::TranslationOnly {
                35.0
            } else {
                31.0
            };
            assert!(
                (result.end.0[0]
                    - (edge
                        + if caller
                            == qa_world::collision::trace_policy(
                                qa_core::primitives::RuleSetId::Quake3
                            )
                            .1
                        {
                            0.125
                        } else {
                            0.03125
                        }))
                .abs()
                    < 0.00001
            );
            assert!((result.plane.normal.0[0] - 1.0).abs() < 0.00001);
            assert_eq!(
                result.plane.distance,
                if rotation == ModelRotation::TranslationOnly {
                    5.0
                } else {
                    1.0
                }
            );
            if caller == qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake).1
            {
                // Native SV_PointContents never adds linked model contents.
                assert_eq!(
                    scene.point_contents(Vec3([30.0, 44.0, 50.0]), caller, &[]),
                    Contents::EMPTY
                );
            } else {
                let (inside, outside) = if rotation == ModelRotation::TranslationOnly {
                    (Vec3([34.0, 40.0, 50.0]), Vec3([30.0, 44.0, 50.0]))
                } else {
                    (Vec3([30.0, 44.0, 50.0]), Vec3([34.0, 40.0, 50.0]))
                };
                assert_eq!(scene.point_contents(inside, caller, &[]), Contents::WATER);
                assert_eq!(scene.point_contents(outside, caller, &[]), Contents::EMPTY);
                assert_eq!(
                    scene.point_contents(inside, caller, &[target]),
                    Contents::EMPTY
                );
            }
        }
    }
}

fn candidates(area: &AreaGrid, table: &EntityTable, point: Vec3) -> Vec<EntityId> {
    area.query(
        table,
        Bounds {
            mins: point,
            maxs: point,
        },
        LinkFlags::SOLID,
    )
    .map(|linked| linked.id)
    .collect()
}

#[test]
fn rotated_link_bounds_keep_native_cube_radius_and_separate_load_margin() {
    let mut store = CollisionStore::new();
    let geometry = brush_model(
        &mut store,
        Vec3([-2.0, -3.0, -11.0]),
        Vec3([2.0, 3.0, 11.0]),
        Contents::SOLID,
    );
    assert_eq!(
        store.model_bounds(geometry, 1).unwrap().mins,
        Vec3([-3.0, -4.0, -12.0])
    );
    for (policy, radius) in [
        (RotatedLinkBounds::Unrotated, 3.0),
        (RotatedLinkBounds::MaxAbsCube, 12.0),
        (RotatedLinkBounds::RadiusCube, 13.0),
    ] {
        let (mut table, mut area) = table();
        let target = model(
            &store,
            &mut table,
            &mut area,
            geometry,
            Vec3([100.0; 3]),
            Vec3([0.0, 90.0, 0.0]),
            ModelRules {
                rotation: ModelRotation::NegativeEuler,
                link_bounds: policy,
            },
        );
        assert_eq!(
            candidates(&area, &table, Vec3([100.0 + radius + 1.0, 100.25, 100.25])),
            vec![target]
        );
        assert!(
            candidates(
                &area,
                &table,
                Vec3([100.0 + radius + 1.125, 100.25, 100.25])
            )
            .is_empty()
        );
        let relinks = area.relinks;
        assert!(!area.link(
            &table,
            target,
            LinkFlags::SOLID,
            LinkOrder::Tail,
            LinkIntent::Commit
        ));
        assert_eq!(area.relinks, relinks);
        assert!(area.link(
            &table,
            target,
            LinkFlags::SOLID,
            LinkOrder::Tail,
            LinkIntent::Explicit
        ));
        assert_eq!(area.relinks, relinks + 1);
        // Module setsize remains independent of the stored model bounds.
        table.columns.mins[target.slot as usize] = Vec3([-1.0; 3]);
        table.columns.maxs[target.slot as usize] = Vec3([1.0; 3]);
        assert!(area.link(
            &table,
            target,
            LinkFlags::SOLID,
            LinkOrder::Tail,
            LinkIntent::Commit
        ));
        assert!(candidates(&area, &table, Vec3([110.0, 100.25, 100.25])).is_empty());
    }
}

#[test]
fn boxes_ignore_model_rotation_and_item_link_expansion_stays_native() {
    let mut store = CollisionStore::new();
    let geometry = brush_model(
        &mut store,
        Vec3([-2.0, -3.0, -11.0]),
        Vec3([2.0, 3.0, 11.0]),
        Contents::SOLID,
    );
    let (mut table, mut area) = table();
    let target = model(
        &store,
        &mut table,
        &mut area,
        geometry,
        Vec3([100.0; 3]),
        Vec3([0.0, 90.0, 0.0]),
        ModelRules {
            rotation: ModelRotation::TransposeBasis,
            link_bounds: RotatedLinkBounds::RadiusCube,
        },
    );
    table.columns.collision_shape[target.slot as usize] = CollisionShape::Box;
    assert!(area.link(
        &table,
        target,
        LinkFlags::SOLID,
        LinkOrder::Tail,
        LinkIntent::Explicit
    ));
    assert!(candidates(&area, &table, Vec3([110.0, 100.25, 100.25])).is_empty());
    assert!(area.link(
        &table,
        target,
        LinkFlags(LinkFlags::SOLID.0 | LinkFlags::ITEM),
        LinkOrder::Tail,
        LinkIntent::Explicit
    ));
    assert_eq!(
        candidates(&area, &table, Vec3([118.0, 100.25, 100.25])),
        vec![target]
    );
    assert!(candidates(&area, &table, Vec3([118.125, 100.25, 100.25])).is_empty());
    assert_eq!(
        candidates(&area, &table, Vec3([100.25, 100.25, 112.0])),
        vec![target]
    );
    assert!(candidates(&area, &table, Vec3([100.25, 100.25, 112.125])).is_empty());
}

#[test]
fn unchanged_rotated_link_cube_does_not_freeze_the_narrow_phase_pose() {
    let mut store = CollisionStore::new();
    let world = empty(&mut store);
    let geometry = brush_model(
        &mut store,
        Vec3([-5.0, -1.0, -2.0]),
        Vec3([5.0, 1.0, 2.0]),
        Contents::SOLID,
    );
    let (mut table, mut area) = table();
    let target = model(
        &store,
        &mut table,
        &mut area,
        geometry,
        Vec3([30.0, 40.0, 50.0]),
        Vec3([0.0, 90.0, 0.0]),
        ModelRules {
            rotation: ModelRotation::NegativeEuler,
            link_bounds: RotatedLinkBounds::MaxAbsCube,
        },
    );
    let mut scratch = store.scratch();
    let point = Vec3([30.0, 44.0, 50.0]);
    assert_eq!(
        WorldTrace::new(&store, world, 0, &table, &area, &mut scratch, None).point_contents(
            point,
            qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake2).1,
            &[]
        ),
        Contents::SOLID
    );
    table.columns.angles[target.slot as usize] = Vec3([0.0, 180.0, 0.0]);
    assert!(!area.link(
        &table,
        target,
        LinkFlags::SOLID,
        LinkOrder::Tail,
        LinkIntent::Commit
    ));
    assert_eq!(
        WorldTrace::new(&store, world, 0, &table, &area, &mut scratch, None).point_contents(
            point,
            qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake2).1,
            &[]
        ),
        Contents::EMPTY
    );
}

#[test]
fn published_contents_pose_is_independent_with_physical_broadphase_and_reset() {
    let mut store = CollisionStore::new();
    let world = empty(&mut store);
    let geometry = brush_model(
        &mut store,
        Vec3([-5.0, -1.0, -2.0]),
        Vec3([5.0, 1.0, 2.0]),
        Contents::WATER,
    );
    let (mut table, mut area) = table();
    let target = model(
        &store,
        &mut table,
        &mut area,
        geometry,
        Vec3([30.0, 40.0, 50.0]),
        Vec3([0.0, 90.0, 0.0]),
        ModelRules {
            rotation: ModelRotation::TransposeBasis,
            link_bounds: RotatedLinkBounds::RadiusCube,
        },
    );
    let mut scratch = store.scratch();
    let point = Vec3([36.0, 40.0, 50.0]);
    assert_eq!(
        WorldTrace::new(&store, world, 0, &table, &area, &mut scratch, None).point_contents(
            point,
            qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake3).1,
            &[]
        ),
        Contents::EMPTY
    );
    table.columns.point_contents_pose[target.slot as usize] = Some(EntityPose {
        position: Vec3([32.0, 40.0, 50.0]),
        angles: Vec3::default(),
    });
    // qsrc Q3 sv_world.c:678-683 uses the published s.origin/s.angles after
    // querying the physical r.currentOrigin/currentAngles area bounds.
    let mut scene = WorldTrace::new(&store, world, 0, &table, &area, &mut scratch, None);
    assert_eq!(
        scene.point_contents(
            point,
            qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake3).1,
            &[]
        ),
        Contents::WATER
    );
    let result = scene.trace(TraceQuery {
        mask: Contents::WATER,
        ..query(
            Vec3([40.0, 40.0, 50.0]),
            Vec3([20.0, 40.0, 50.0]),
            qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake3).1,
        )
    });
    assert_eq!(result.entity, Some(target));
    assert!((result.end.0[0] - 31.125).abs() < 0.00001);
    // A published pose beyond the physical link bounds does not introduce a
    // second area index or bypass the native broadphase.
    table.columns.point_contents_pose[target.slot as usize] = Some(EntityPose {
        position: Vec3([300.0, 40.0, 50.0]),
        angles: Vec3::default(),
    });
    assert_eq!(
        WorldTrace::new(&store, world, 0, &table, &area, &mut scratch, None).point_contents(
            Vec3([300.0, 40.0, 50.0]),
            qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake3).1,
            &[]
        ),
        Contents::EMPTY
    );
    area.unlink(target);
    assert!(table.release(target, ThinkTime::Seconds(1.0)));
    let replacement = table
        .allocate(
            ThinkTime::Seconds(1.1),
            ModuleId(8),
            AllocationPolicy::EDICT,
        )
        .unwrap()
        .id;
    assert_eq!(replacement.slot, target.slot);
    assert_ne!(replacement.generation, target.generation);
    assert_eq!(
        table.columns.point_contents_pose[replacement.slot as usize],
        None
    );
    assert_eq!(
        table.columns.model_rules[replacement.slot as usize],
        ModelRules::default()
    );
}

#[test]
fn box_contents_uses_published_origin_and_ignores_published_angles() {
    let mut store = CollisionStore::new();
    let world = empty(&mut store);
    let (mut table, mut area) = table();
    let target = table
        .allocate(
            ThinkTime::Seconds(0.0),
            ModuleId(7),
            AllocationPolicy::EDICT,
        )
        .unwrap()
        .id;
    let slot = target.slot as usize;
    table.columns.set_body(
        slot,
        Body {
            position: Vec3([30.0, 40.0, 50.0]),
            mins: Vec3([-6.0; 3]),
            maxs: Vec3([6.0; 3]),
            ..Default::default()
        },
    );
    table.columns.collision_shape[slot] = CollisionShape::Box;
    assert!(area.link(
        &table,
        target,
        LinkFlags::SOLID,
        LinkOrder::Tail,
        LinkIntent::Explicit
    ));
    let mut scratch = store.scratch();
    let point = Vec3([24.0, 40.0, 50.0]);
    assert_eq!(
        WorldTrace::new(&store, world, 0, &table, &area, &mut scratch, None).point_contents(
            point,
            qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake3).1,
            &[]
        ),
        Contents::BODY
    );
    table.columns.point_contents_pose[slot] = Some(EntityPose {
        position: Vec3([33.0, 40.0, 50.0]),
        angles: Vec3([75.0, 90.0, 65.0]),
    });
    let scene = WorldTrace::new(&store, world, 0, &table, &area, &mut scratch, None);
    assert_eq!(
        scene.point_contents(
            point,
            qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake3).1,
            &[]
        ),
        Contents::EMPTY
    );
    assert_eq!(
        scene.point_contents(
            Vec3([36.0, 40.0, 50.0]),
            qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake3).1,
            &[]
        ),
        Contents::BODY
    );
}
