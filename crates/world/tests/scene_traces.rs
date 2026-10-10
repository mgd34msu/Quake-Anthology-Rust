use qa_core::primitives::ThinkTime;
use qa_core::primitives::{
    Body, Bounds, CollisionOwner, CollisionShape, CollisionTags, EntityId, GeometryId, ModuleId,
    NativeEntity, Plane, SurfaceFlags, Vec3,
};
use qa_world::{
    area::{AreaGrid, LinkFlags, LinkIntent, LinkOrder},
    collision::{
        CollisionStore, Contents, EntityTracePolicy, QuakeTraceKind, Trace, TraceQuery, TraceRules,
        WorldTrace,
        brushes::{Brush, BrushTree, CollisionLeaf, ModelRoot},
    },
    entities::{AllocationPolicy, EntityTable},
};

struct FixtureWorld {
    store: CollisionStore,
    geometry: GeometryId,
}
fn brush_world(planes: Vec<Plane>, brushes: Vec<Brush>) -> FixtureWorld {
    let count = brushes.len() as u32;
    let surfaces = vec![SurfaceFlags::default(); planes.len()];
    let mut store = CollisionStore::new();
    let geometry = store
        .load_brushes(
            planes,
            brushes,
            surfaces,
            BrushTree {
                planes: Vec::new(),
                nodes: Vec::new(),
                leaves: vec![CollisionLeaf {
                    stored_contents: None,
                    first_brush: 0,
                    brush_count: count,
                }],
                leaf_brushes: (0..count).collect(),
                models: vec![ModelRoot::Leaf(0)],
            },
            vec![Bounds {
                mins: Vec3([-1024.0; 3]),
                maxs: Vec3([1024.0; 3]),
            }],
        )
        .unwrap();
    FixtureWorld { store, geometry }
}
fn empty() -> FixtureWorld {
    brush_world(Vec::new(), Vec::new())
}
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
fn body(
    table: &mut EntityTable,
    area: &mut AreaGrid,
    origin: Vec3,
    mins: Vec3,
    maxs: Vec3,
) -> EntityId {
    let id = table
        .allocate(
            ThinkTime::Seconds(0.0),
            ModuleId(2),
            AllocationPolicy::EDICT,
        )
        .unwrap()
        .id;
    let slot = id.slot as usize;
    table.columns.set_body(
        slot,
        Body {
            position: origin,
            mins,
            maxs,
            ..Default::default()
        },
    );
    table.columns.native_entity[slot] = Some(NativeEntity {
        module: ModuleId(2),
        slot: slot as i32,
    });
    table.columns.collision_shape[slot] = CollisionShape::Box;
    table.columns.collision_contents[slot] = Contents::BODY.0;
    area.link(
        table,
        id,
        LinkFlags::SOLID,
        LinkOrder::Tail,
        LinkIntent::Explicit,
    );
    id
}
fn query(rules: EntityTracePolicy) -> TraceQuery<'static> {
    TraceQuery {
        mask: Contents::SOLID | Contents::BODY,
        ..TraceQuery::point(
            Vec3([-50.0, 0.0, 0.0]),
            Vec3([50.0, 0.0, 0.0]),
            if rules == qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake3).1
            {
                TraceRules::ARENA
            } else {
                TraceRules::LEGACY
            },
            rules,
        )
    }
}
fn trace(world: &FixtureWorld, table: &EntityTable, area: &AreaGrid, query: TraceQuery) -> Trace {
    let mut scratch = world.store.scratch();
    WorldTrace::new(
        &world.store,
        world.geometry,
        0,
        table,
        area,
        &mut scratch,
        None,
    )
    .trace(query)
}

#[test]
fn pass_exclusions_and_native_weak_owner_rules_cross_module_slots() {
    let world = empty();
    let (mut table, mut area) = table();
    let pass = body(
        &mut table,
        &mut area,
        Vec3([-50.0, 0.0, 0.0]),
        Vec3([-2.0; 3]),
        Vec3([2.0; 3]),
    );
    let target = body(
        &mut table,
        &mut area,
        Vec3::default(),
        Vec3([-5.0; 3]),
        Vec3([5.0; 3]),
    );
    table.columns.native_entity[pass.slot as usize] = Some(NativeEntity {
        module: ModuleId(8),
        slot: 103,
    });
    table.columns.collision_owner[target.slot as usize] = CollisionOwner::Native(NativeEntity {
        module: ModuleId(8),
        slot: 103,
    });
    for rules in [
        qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake).1,
        qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake2).1,
        qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake3).1,
    ] {
        assert_eq!(
            trace(
                &world,
                &table,
                &area,
                TraceQuery {
                    pass: Some(pass),
                    ..query(rules)
                }
            )
            .fraction,
            1.0
        );
    }
    table.columns.collision_owner[target.slot as usize] = CollisionOwner::None;
    table.columns.collision_owner[pass.slot as usize] =
        CollisionOwner::Native(table.columns.native_entity[target.slot as usize].unwrap());
    for rules in [
        qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake).1,
        qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake2).1,
    ] {
        assert_eq!(
            trace(
                &world,
                &table,
                &area,
                TraceQuery {
                    pass: Some(pass),
                    ..query(rules)
                }
            )
            .fraction,
            1.0
        );
    }
    assert_eq!(
        trace(
            &world,
            &table,
            &area,
            TraceQuery {
                pass: Some(pass),
                ..query(qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake3).1)
            }
        )
        .entity,
        Some(target)
    );
    table.columns.collision_owner[pass.slot as usize] = CollisionOwner::None;
    assert_eq!(
        trace(
            &world,
            &table,
            &area,
            TraceQuery {
                pass: Some(pass),
                excluded: &[target],
                ..query(qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake3).1)
            }
        )
        .fraction,
        1.0
    );
    let stale = EntityId {
        generation: target.generation + 1,
        ..target
    };
    assert_eq!(
        trace(
            &world,
            &table,
            &area,
            TraceQuery {
                pass: Some(pass),
                excluded: &[stale],
                ..query(qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake3).1)
            }
        )
        .entity,
        Some(target)
    );
}

#[test]
fn arena_siblings_and_pass_owner_none_keep_native_signed_comparison() {
    let world = empty();
    let (mut table, mut area) = table();
    let pass = body(
        &mut table,
        &mut area,
        Vec3([-50.0, 0.0, 0.0]),
        Vec3([-2.0; 3]),
        Vec3([2.0; 3]),
    );
    let target = body(
        &mut table,
        &mut area,
        Vec3::default(),
        Vec3([-5.0; 3]),
        Vec3([5.0; 3]),
    );
    let owner = NativeEntity {
        module: ModuleId(2),
        slot: 71,
    };
    table.columns.collision_owner[pass.slot as usize] = CollisionOwner::Native(owner);
    table.columns.collision_owner[target.slot as usize] = CollisionOwner::Native(owner);
    assert_eq!(
        trace(
            &world,
            &table,
            &area,
            TraceQuery {
                pass: Some(pass),
                ..query(qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake3).1)
            }
        )
        .fraction,
        1.0
    );
    assert_eq!(
        trace(
            &world,
            &table,
            &area,
            TraceQuery {
                pass: Some(pass),
                ..query(qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake2).1)
            }
        )
        .entity,
        Some(target)
    );
    table.columns.collision_owner[pass.slot as usize] = CollisionOwner::Native(NativeEntity {
        module: ModuleId(2),
        slot: 1023,
    });
    table.columns.collision_owner[target.slot as usize] = CollisionOwner::Native(NativeEntity {
        module: ModuleId(2),
        slot: 1023,
    });
    assert_eq!(
        trace(
            &world,
            &table,
            &area,
            TraceQuery {
                pass: Some(pass),
                ..query(qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake3).1)
            }
        )
        .entity,
        Some(target)
    );
    table.columns.collision_owner[target.slot as usize] = CollisionOwner::Native(NativeEntity {
        module: ModuleId(2),
        slot: -1,
    });
    assert_eq!(
        trace(
            &world,
            &table,
            &area,
            TraceQuery {
                pass: Some(pass),
                ..query(qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake3).1)
            }
        )
        .fraction,
        1.0
    );
}

#[test]
fn caller_selects_missile_monster_bounds_no_monsters_and_point_pass_filter() {
    let world = empty();
    let (mut table, mut area) = table();
    let monster = body(
        &mut table,
        &mut area,
        Vec3([0.0, 12.0, 0.0]),
        Vec3([-1.0; 3]),
        Vec3([1.0; 3]),
    );
    table.columns.collision_tags[monster.slot as usize] = CollisionTags::MONSTER;
    assert_eq!(
        trace(
            &world,
            &table,
            &area,
            query(qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake).1)
        )
        .fraction,
        1.0
    );
    let missile = qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake)
        .1
        .with_quake_kind(Some(QuakeTraceKind::Missile));
    assert_eq!(
        trace(&world, &table, &area, query(missile)).entity,
        Some(monster)
    );
    assert_eq!(
        trace(
            &world,
            &table,
            &area,
            query(
                qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake)
                    .1
                    .with_quake_kind(Some(QuakeTraceKind::IgnoreBoxes))
            )
        )
        .fraction,
        1.0
    );
    let pass = body(
        &mut table,
        &mut area,
        Vec3([-50.0, 0.0, 0.0]),
        Vec3([-2.0; 3]),
        Vec3([2.0; 3]),
    );
    let point = body(
        &mut table,
        &mut area,
        Vec3::default(),
        Vec3::default(),
        Vec3::default(),
    );
    assert_eq!(
        trace(
            &world,
            &table,
            &area,
            TraceQuery {
                pass: Some(pass),
                mins: Vec3([-1.0; 3]),
                maxs: Vec3([1.0; 3]),
                ..query(qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake).1)
            }
        )
        .fraction,
        1.0
    );
    table.columns.maxs[pass.slot as usize].0[0] = table.columns.mins[pass.slot as usize].0[0];
    assert_eq!(
        trace(
            &world,
            &table,
            &area,
            TraceQuery {
                pass: Some(pass),
                mins: Vec3([-1.0; 3]),
                maxs: Vec3([1.0; 3]),
                ..query(qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake).1)
            }
        )
        .entity,
        Some(point)
    );
}

#[test]
fn native_contents_filter_and_point_contents_face_semantics() {
    let world = empty();
    let (mut table, mut area) = table();
    let target = body(
        &mut table,
        &mut area,
        Vec3::default(),
        Vec3([-1.0; 3]),
        Vec3([1.0; 3]),
    );
    table.columns.collision_tags[target.slot as usize] = CollisionTags::DEAD_MONSTER;
    assert_eq!(
        trace(
            &world,
            &table,
            &area,
            query(qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake2).1)
        )
        .entity,
        table.id_at(0)
    );
    assert_eq!(
        trace(
            &world,
            &table,
            &area,
            TraceQuery {
                mask: Contents::BODY | Contents::CORPSE,
                ..query(qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake2).1)
            }
        )
        .entity,
        Some(target)
    );
    table.columns.collision_contents[target.slot as usize] = Contents::WATER.0;
    assert_eq!(
        trace(
            &world,
            &table,
            &area,
            query(qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake3).1)
        )
        .fraction,
        1.0
    );
    assert_eq!(
        trace(
            &world,
            &table,
            &area,
            TraceQuery {
                mask: Contents::WATER,
                ..query(qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake3).1)
            }
        )
        .fraction,
        1.0
    );
    assert_eq!(
        trace(
            &world,
            &table,
            &area,
            TraceQuery {
                mask: Contents::BODY | Contents::WATER,
                ..query(qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake3).1)
            }
        )
        .entity,
        Some(target)
    );
    let mut scratch = world.store.scratch();
    let service = WorldTrace::new(
        &world.store,
        world.geometry,
        0,
        &table,
        &area,
        &mut scratch,
        None,
    );
    assert_eq!(
        service.point_contents(
            Vec3::default(),
            qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake).1,
            &[]
        ),
        Contents::EMPTY
    );
    assert_eq!(
        service.point_contents(
            Vec3::default(),
            qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake2).1,
            &[]
        ),
        Contents::BODY
    );
    assert_eq!(
        service.point_contents(
            Vec3([1.0, 0.0, 0.0]),
            qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake2).1,
            &[]
        ),
        Contents::EMPTY
    );
    assert_eq!(
        service.point_contents(
            Vec3([-1.0, 0.0, 0.0]),
            qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake2).1,
            &[]
        ),
        Contents::BODY
    );
    assert_eq!(
        service.point_contents(
            Vec3([1.0, 0.0, 0.0]),
            qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake3).1,
            &[]
        ),
        Contents::BODY
    );
    assert_eq!(
        service.point_contents(
            Vec3::default(),
            qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake3).1,
            &[target]
        ),
        Contents::EMPTY
    );
    let pass_service = WorldTrace::new(
        &world.store,
        world.geometry,
        0,
        &table,
        &area,
        &mut scratch,
        Some(target),
    );
    assert_eq!(
        pass_service.point_contents(
            Vec3::default(),
            qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake2).1,
            &[]
        ),
        Contents::BODY
    );
    assert_eq!(
        pass_service.point_contents(
            Vec3::default(),
            qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake3).1,
            &[]
        ),
        Contents::EMPTY
    );
    area.link(
        &table,
        target,
        LinkFlags::LINKED,
        LinkOrder::Head,
        LinkIntent::Explicit,
    );
    table.columns.collision_contents[target.slot as usize] = Contents::BODY.0;
    assert_eq!(
        trace(
            &world,
            &table,
            &area,
            query(qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake3).1)
        )
        .entity,
        Some(target)
    );
    assert_eq!(
        trace(
            &world,
            &table,
            &area,
            query(qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake2).1)
        )
        .fraction,
        1.0
    );
}

#[test]
fn native_world_zero_fraction_returns_before_linked_body_merging() {
    let world = brush_world(
        vec![Plane {
            normal: Vec3([1.0, 0.0, 0.0]),
            distance: 0.0,
            axis: None,
        }],
        vec![Brush {
            first_plane: 0,
            plane_count: 1,
            contents: Contents::SOLID,
        }],
    );
    let (mut table, mut area) = table();
    body(
        &mut table,
        &mut area,
        Vec3([0.03125, 0.0, 0.0]),
        Vec3([-5.0; 3]),
        Vec3([5.0; 3]),
    );
    for rules in [
        qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake2).1,
        qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake3).1,
    ] {
        let result = trace(
            &world,
            &table,
            &area,
            TraceQuery {
                start: Vec3([0.03125, 0.0, 0.0]),
                end: Vec3([-50.0, 0.0, 0.0]),
                ..query(rules)
            },
        );
        assert_eq!(result.fraction, 0.0);
        assert_eq!(result.entity, table.id_at(0));
        assert!(!result.start_solid);
        assert_eq!(result.contents, Contents::SOLID);
    }
}
