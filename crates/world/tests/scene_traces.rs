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
            Vec::new(),
            qa_world::collision::surfaces::SurfaceTable::flags(surfaces),
            BrushTree {
                leaf_patches: Default::default(),
                planes: Vec::new(),
                nodes: Vec::new(),
                leaves: vec![CollisionLeaf {
                    patches: Default::default(),
                    stored_contents: None,
                    brushes: qa_core::primitives::IndexSpan {
                        first: 0,
                        count: count,
                    },
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
        CollisionOwner::None,
    )
    .trace(query)
    .expect("valid collision query")
}

#[test]
fn world_scope_keeps_world_clipping_and_skips_linked_bodies_for_each_trace_policy() {
    let world = brush_world(
        vec![Plane::oriented(Vec3([-1.0, 0.0, 0.0]), -24.0)],
        vec![Brush {
            sides: qa_core::primitives::IndexSpan { first: 0, count: 1 },
            contents: Contents::SOLID,
        }],
    );
    let (mut table, mut area) = table();
    let target = body(
        &mut table,
        &mut area,
        Vec3::default(),
        Vec3([-5.0; 3]),
        Vec3([5.0; 3]),
    );
    for rules in qa_core::primitives::RuleSetId::ALL {
        let (trace_rules, entity_rules) = qa_world::collision::trace_policy(rules);
        let mut input = query(entity_rules);
        input.rules = trace_rules;
        let scene = trace(&world, &table, &area, input);
        input.scope = qa_world::collision::TraceScope::World;
        let world_only = trace(&world, &table, &area, input);
        assert_eq!(scene.entity, Some(target));
        assert!(world_only.fraction < 1.0);
        assert!(world_only.end.0[0] > 20.0);
        assert!(scene.fraction < world_only.fraction);
        assert_eq!(world_only.entity, table.id_at(0));
    }
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
                    pass: CollisionOwner::Lifetime(pass),
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
                    pass: CollisionOwner::Lifetime(pass),
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
                pass: CollisionOwner::Lifetime(pass),
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
                pass: CollisionOwner::Lifetime(pass),
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
                pass: CollisionOwner::Lifetime(pass),
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
                pass: CollisionOwner::Lifetime(pass),
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
                pass: CollisionOwner::Lifetime(pass),
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
                pass: CollisionOwner::Lifetime(pass),
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
                pass: CollisionOwner::Lifetime(pass),
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
                pass: CollisionOwner::Lifetime(pass),
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
                pass: CollisionOwner::Lifetime(pass),
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
        CollisionOwner::None,
    );
    assert_eq!(
        service
            .point_contents(
                Vec3::default(),
                qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake).1,
                &[]
            )
            .expect("valid collision query"),
        Contents::EMPTY
    );
    assert_eq!(
        service
            .point_contents(
                Vec3::default(),
                qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake2).1,
                &[]
            )
            .expect("valid collision query"),
        Contents::BODY
    );
    assert_eq!(
        service
            .point_contents(
                Vec3([1.0, 0.0, 0.0]),
                qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake2).1,
                &[]
            )
            .expect("valid collision query"),
        Contents::EMPTY
    );
    assert_eq!(
        service
            .point_contents(
                Vec3([-1.0, 0.0, 0.0]),
                qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake2).1,
                &[]
            )
            .expect("valid collision query"),
        Contents::BODY
    );
    assert_eq!(
        service
            .point_contents(
                Vec3([1.0, 0.0, 0.0]),
                qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake3).1,
                &[]
            )
            .expect("valid collision query"),
        Contents::BODY
    );
    assert_eq!(
        service
            .point_contents(
                Vec3::default(),
                qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake3).1,
                &[target]
            )
            .expect("valid collision query"),
        Contents::EMPTY
    );
    let pass_service = WorldTrace::new(
        &world.store,
        world.geometry,
        0,
        &table,
        &area,
        &mut scratch,
        CollisionOwner::Lifetime(target),
    );
    assert_eq!(
        pass_service
            .point_contents(
                Vec3::default(),
                qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake2).1,
                &[]
            )
            .expect("valid collision query"),
        Contents::BODY
    );
    assert_eq!(
        pass_service
            .point_contents(
                Vec3::default(),
                qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake3).1,
                &[]
            )
            .expect("valid collision query"),
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
            encoding: None,
            normal: Vec3([1.0, 0.0, 0.0]),
            distance: 0.0,
            axis: None,
        }],
        vec![Brush {
            sides: qa_core::primitives::IndexSpan { first: 0, count: 1 },
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

#[test]
fn weak_native_pass_filtering_does_not_require_a_live_common_slot() {
    use qa_core::primitives::RuleSetId;
    let world = empty();
    let (mut table, mut area) = table();
    let target = body(
        &mut table,
        &mut area,
        Vec3::default(),
        Vec3([-5.0; 3]),
        Vec3([5.0; 3]),
    );
    let weak = CollisionOwner::Native(NativeEntity {
        module: ModuleId(8),
        slot: 103,
    });
    table.columns.collision_owner[target.slot as usize] = weak;
    let mut q = query(qa_world::collision::trace_policy(RuleSetId::Quake2).1);
    q.pass = weak;
    assert_eq!(trace(&world, &table, &area, q).fraction, 1.0);
    q.pass = CollisionOwner::Native(NativeEntity {
        module: ModuleId(9),
        slot: 103,
    });
    assert_eq!(trace(&world, &table, &area, q).entity, Some(target));
    // A stale strong pass remains absent; it cannot suppress other contacts or
    // become a weak reference to the replacement at that slot.
    q.pass = CollisionOwner::Lifetime(EntityId {
        slot: u32::MAX,
        generation: 5,
    });
    assert_eq!(trace(&world, &table, &area, q).entity, Some(target));
}

#[test]
fn rerelease_entity_class_filters_are_independent_of_geometry_and_other_callers() {
    use qa_core::primitives::RuleSetId;
    let world = empty();
    let (mut table, mut area) = table();
    let target = body(
        &mut table,
        &mut area,
        Vec3::default(),
        Vec3([-5.0; 3]),
        Vec3([5.0; 3]),
    );
    for (tag, content) in [
        (CollisionTags::PLAYER, Contents::PLAYER),
        (CollisionTags::PROJECTILE, Contents::PROJECTILE),
    ] {
        table.columns.collision_tags[target.slot as usize] = tag;
        for rules in [
            RuleSetId::Quake,
            RuleSetId::Quake2,
            RuleSetId::Quake2Rerelease,
            RuleSetId::Quake3,
        ] {
            let (clipping, entities) = qa_world::collision::trace_policy(rules);
            let mut q = query(entities);
            q.rules = clipping;
            assert_eq!(
                trace(&world, &table, &area, q).fraction == 1.0,
                rules == RuleSetId::Quake2Rerelease
            );
            q.mask |= content;
            assert_eq!(trace(&world, &table, &area, q).entity, Some(target));
        }
    }
}

#[test]
fn live_native_flags_keep_common_players_and_replace_stale_native_classes() {
    use qa_core::primitives::RuleSetId;
    use qa_world::collision::{
        NativeModelNames, NativeTraceEntities, NativeTraceFields, NativeTraceTarget,
    };
    struct LiveClasses;
    impl NativeTraceEntities for LiveClasses {
        fn supports(&self, module: ModuleId) -> bool {
            module == ModuleId(2)
        }
        fn owner(&self, _: NativeEntity) -> Option<CollisionOwner> {
            Some(CollisionOwner::None)
        }
        fn fields(&self, native: NativeEntity, _: bool) -> Option<NativeTraceFields> {
            Some(NativeTraceFields {
                tags: if native.slot == 0 {
                    CollisionTags::default()
                } else {
                    CollisionTags::MONSTER
                },
                enabled: true,
                box_target: true,
                contents: Contents::BODY,
            })
        }
        fn bounds(&self, _: NativeEntity, cached: Bounds) -> Option<Bounds> {
            Some(cached)
        }
        fn target(
            &self,
            _: NativeEntity,
            cached: NativeTraceTarget,
            _: bool,
            _: &CollisionStore,
            _: Option<&dyn NativeModelNames>,
        ) -> Option<NativeTraceTarget> {
            Some(cached)
        }
    }
    let world = empty();
    let (_, mut area) = table();
    let mut table = EntityTable::new(16, 2).unwrap();
    let client = table.id_at(1).unwrap();
    let slot = client.slot as usize;
    table.columns.set_body(
        slot,
        Body {
            mins: Vec3([-5.0; 3]),
            maxs: Vec3([5.0; 3]),
            ..Default::default()
        },
    );
    table.columns.native_entity[slot] = Some(NativeEntity {
        module: ModuleId(2),
        slot: 0,
    });
    table.columns.collision_shape[slot] = CollisionShape::Box;
    table.columns.collision_contents[slot] = Contents::BODY.0;
    assert!(area.link(
        &table,
        client,
        LinkFlags::SOLID,
        LinkOrder::Tail,
        LinkIntent::Explicit
    ));
    let monster = body(
        &mut table,
        &mut area,
        Vec3([30.0, 0.0, 0.0]),
        Vec3([-5.0; 3]),
        Vec3([5.0; 3]),
    );
    // Cached flags are stale. Native reads must replace these flags while the
    // reserved common client lifetime remains a player even at native slot0.
    table.columns.collision_tags[monster.slot as usize] = CollisionTags::PLAYER;
    let mut scratch = world.store.scratch();
    let mut tracer = WorldTrace::new(
        &world.store,
        world.geometry,
        0,
        &table,
        &area,
        &mut scratch,
        CollisionOwner::None,
    )
    .with_native_entities(Some(&LiveClasses));
    let mut q = query(qa_world::collision::trace_policy(RuleSetId::Quake2Rerelease).1);
    assert_eq!(tracer.trace(q).unwrap().entity, Some(monster));
    q.mask |= Contents::PLAYER;
    assert_eq!(tracer.trace(q).unwrap().entity, Some(client));
}

#[test]
fn attachment_queries_keep_transport_and_live_native_shape_and_published_offset() {
    use qa_core::primitives::{BodyAttachment, BodyFollow, EntityPose, RuleSetId};
    use qa_world::collision::{
        NativeModelNames, NativeTraceEntities, NativeTraceFields, NativeTraceTarget,
    };
    struct View;
    impl NativeTraceEntities for View {
        fn supports(&self, module: ModuleId) -> bool {
            module == ModuleId(2)
        }
        fn owner(&self, _: NativeEntity) -> Option<CollisionOwner> {
            Some(CollisionOwner::None)
        }
        fn fields(&self, _: NativeEntity, _: bool) -> Option<NativeTraceFields> {
            Some(NativeTraceFields {
                enabled: true,
                box_target: true,
                contents: Contents::BODY,
                ..Default::default()
            })
        }
        fn bounds(&self, _: NativeEntity, _: Bounds) -> Option<Bounds> {
            Some(Bounds {
                mins: Vec3([-6.0; 3]),
                maxs: Vec3([6.0; 3]),
            })
        }
        fn target(
            &self,
            _: NativeEntity,
            mut cached: NativeTraceTarget,
            point_contents: bool,
            _: &CollisionStore,
            _: Option<&dyn NativeModelNames>,
        ) -> Option<NativeTraceTarget> {
            // The module has not run since the anchor moved. Its distinct
            // public contents origin is three units ahead of its physical one.
            cached.body.position = if point_contents {
                Vec3([3.0, 0.0, 0.0])
            } else {
                Vec3::default()
            };
            cached.body.mins = Vec3([-6.0; 3]);
            cached.body.maxs = Vec3([6.0; 3]);
            cached.contents_pose = point_contents.then_some(EntityPose {
                position: Vec3([3.0, 0.0, 0.0]),
                angles: Vec3::default(),
            });
            Some(cached)
        }
    }
    let world = empty();
    let (mut table, mut area) = table();
    let target = body(
        &mut table,
        &mut area,
        Vec3::default(),
        Vec3([-8.0; 3]),
        Vec3([8.0; 3]),
    );
    let anchor = table
        .allocate(
            ThinkTime::Milliseconds(0),
            ModuleId(3),
            AllocationPolicy::EDICT,
        )
        .unwrap()
        .id;
    table.columns.position[anchor.slot as usize] = Vec3([40.0, 0.0, 0.0]);
    table.columns.point_contents_pose[target.slot as usize] = Some(EntityPose {
        position: Vec3([3.0, 0.0, 0.0]),
        angles: Vec3::default(),
    });
    let mut publication = qa_core::primitives::EntityPresentation::new(
        qa_core::primitives::ResourceNamespace::Module(ModuleId(2)),
    );
    publication.pose.position = Vec3([3.0, 0.0, 0.0]);
    publication.trajectories.position.base = Vec3([10.0, 0.0, 0.0]);
    publication.trajectories.position.delta = Vec3([5.0, 6.0, 7.0]);
    publication.trajectories.position.start = ThinkTime::Milliseconds(1234);
    publication.secondary_pose.position = Vec3([99.0; 3]);
    table.columns.appearance[target.slot as usize].presentation = Some(publication);
    table
        .attach(
            target,
            BodyAttachment {
                anchor,
                follow: BodyFollow::Translation,
                offset: Vec3::default(),
            },
        )
        .unwrap();
    assert_eq!(area.transport_attachments(&mut table).moved, 1);
    let link = area.bounds(target);
    for rules in RuleSetId::ALL {
        for foreign in [false, true] {
            let mut scratch = world.store.scratch();
            let mut scene = WorldTrace::new(
                &world.store,
                world.geometry,
                0,
                &table,
                &area,
                &mut scratch,
                CollisionOwner::None,
            );
            scene = if foreign {
                scene.with_foreign_entities(Some(&View))
            } else {
                scene.with_native_entities(Some(&View))
            };
            let (clipping, entities) = qa_world::collision::trace_policy(rules);
            let hit = scene
                .trace(TraceQuery {
                    mask: Contents::SOLID | Contents::BODY,
                    ..TraceQuery::point(
                        Vec3([20.0, 0.0, 0.0]),
                        Vec3([60.0, 0.0, 0.0]),
                        clipping,
                        entities,
                    )
                })
                .unwrap();
            assert_eq!(hit.entity, Some(target), "{rules:?}, foreign={foreign}");
            assert!(hit.end.0[0] > 33.0 && hit.end.0[0] < 34.0);
            if matches!(
                rules,
                RuleSetId::Quake2 | RuleSetId::Quake2Rerelease | RuleSetId::Quake3
            ) {
                assert_eq!(
                    scene
                        .point_contents(Vec3([47.0, 0.0, 0.0]), entities, &[])
                        .unwrap(),
                    Contents::BODY
                );
                assert_eq!(
                    scene
                        .point_contents(Vec3([33.0, 0.0, 0.0]), entities, &[])
                        .unwrap(),
                    Contents::EMPTY
                );
            }
        }
    }
    assert_eq!(area.bounds(target), link);
    let transported = table.columns.appearance[target.slot as usize]
        .presentation
        .unwrap();
    assert_eq!(transported.pose.position, Vec3([43.0, 0.0, 0.0]));
    assert_eq!(
        transported.trajectories.position.base,
        Vec3([50.0, 0.0, 0.0])
    );
    assert_eq!(
        transported.trajectories.position.delta,
        publication.trajectories.position.delta
    );
    assert_eq!(
        transported.trajectories.position.start,
        publication.trajectories.position.start
    );
    assert_eq!(transported.secondary_pose, publication.secondary_pose);
    assert_eq!(
        table.columns.point_contents_pose[target.slot as usize]
            .unwrap()
            .position,
        Vec3([43.0, 0.0, 0.0])
    );
    let mut scratch = world.store.scratch();
    assert_eq!(
        WorldTrace::new(
            &world.store,
            world.geometry,
            0,
            &table,
            &area,
            &mut scratch,
            CollisionOwner::None,
        )
        .point_contents(
            Vec3([47.0, 0.0, 0.0]),
            qa_world::collision::trace_policy(RuleSetId::Quake3).1,
            &[],
        )
        .unwrap(),
        Contents::BODY,
    );
    table.detach(target);
    let mut scratch = world.store.scratch();
    let mut scene = WorldTrace::new(
        &world.store,
        world.geometry,
        0,
        &table,
        &area,
        &mut scratch,
        CollisionOwner::None,
    )
    .with_native_entities(Some(&View));
    let (clipping, entities) = qa_world::collision::trace_policy(RuleSetId::Quake3);
    let hit = scene
        .trace(TraceQuery {
            mask: Contents::SOLID | Contents::BODY,
            ..TraceQuery::point(
                Vec3([-20.0, 0.0, 0.0]),
                Vec3([60.0, 0.0, 0.0]),
                clipping,
                entities,
            )
        })
        .unwrap();
    assert_eq!(hit.entity, Some(target));
    assert!(hit.end.0[0] < 0.0);
}

#[test]
fn owner_views_select_each_namespace_and_never_fall_back_after_a_supported_read() {
    use qa_core::primitives::RuleSetId;
    use qa_world::collision::{
        NativeModelNames, NativeTraceEntities, NativeTraceFields, NativeTraceTarget,
    };
    use std::cell::Cell;

    struct View {
        module: ModuleId,
        enabled: Option<bool>,
        anchor: Option<EntityId>,
        reads: Cell<usize>,
    }
    impl NativeTraceEntities for View {
        fn failed(&self) -> bool {
            self.enabled.is_none() && self.reads.get() != 0
        }
        fn supports(&self, module: ModuleId) -> bool {
            module == self.module
        }
        fn anchor(&self, _: ModuleId) -> Option<EntityId> {
            self.anchor
        }
        fn owner(&self, native: NativeEntity) -> Option<CollisionOwner> {
            assert_eq!(native.module, self.module);
            Some(CollisionOwner::None)
        }
        fn fields(&self, native: NativeEntity, _: bool) -> Option<NativeTraceFields> {
            assert_eq!(native.module, self.module);
            self.reads.set(self.reads.get() + 1);
            Some(NativeTraceFields {
                enabled: self.enabled?,
                box_target: true,
                contents: Contents::BODY,
                ..Default::default()
            })
        }
        fn bounds(&self, native: NativeEntity, cached: Bounds) -> Option<Bounds> {
            assert_eq!(native.module, self.module);
            Some(cached)
        }
        fn target(
            &self,
            native: NativeEntity,
            cached: NativeTraceTarget,
            _: bool,
            _: &CollisionStore,
            _: Option<&dyn NativeModelNames>,
        ) -> Option<NativeTraceTarget> {
            assert_eq!(native.module, self.module);
            Some(cached)
        }
    }

    let world = empty();
    let (mut table, mut area) = table();
    let local_body = body(
        &mut table,
        &mut area,
        Vec3([-20.0, 0.0, 0.0]),
        Vec3([-2.0; 3]),
        Vec3([2.0; 3]),
    );
    let foreign_body = body(
        &mut table,
        &mut area,
        Vec3([20.0, 0.0, 0.0]),
        Vec3([-2.0; 3]),
        Vec3([2.0; 3]),
    );
    table.columns.owner[foreign_body.slot as usize] = ModuleId(3);
    table.columns.native_entity[foreign_body.slot as usize]
        .as_mut()
        .unwrap()
        .module = ModuleId(3);
    let local = View {
        module: ModuleId(2),
        enabled: Some(false),
        anchor: None,
        reads: Cell::new(0),
    };
    let mut foreign = View {
        module: ModuleId(3),
        enabled: Some(true),
        anchor: None,
        reads: Cell::new(0),
    };
    let rules = qa_world::collision::trace_policy(RuleSetId::Quake2).1;
    let mut scratch = world.store.scratch();
    let trace = WorldTrace::new(
        &world.store,
        world.geometry,
        0,
        &table,
        &area,
        &mut scratch,
        CollisionOwner::None,
    )
    .with_native_entities(Some(&local))
    .with_foreign_entities(Some(&foreign))
    .trace(query(rules))
    .unwrap();
    assert_eq!(trace.entity, Some(foreign_body));
    assert!(local.reads.get() > 0);
    assert!(foreign.reads.get() > 0);

    let candidates = [local_body];
    foreign.module = ModuleId(2);
    foreign.reads.set(0);
    for enabled in [Some(false), None] {
        let local = View {
            module: ModuleId(2),
            enabled,
            anchor: None,
            reads: Cell::new(0),
        };
        let trace = WorldTrace::new(
            &world.store,
            world.geometry,
            0,
            &table,
            &area,
            &mut scratch,
            CollisionOwner::None,
        )
        .with_native_entities(Some(&local))
        .with_foreign_entities(Some(&foreign))
        .with_candidates(&candidates)
        .trace(query(rules));
        if enabled.is_none() {
            assert!(matches!(
                trace,
                Err(qa_world::collision::TraceError::Entity)
            ));
            let invalid_world = WorldTrace::new(
                &world.store,
                world.geometry,
                u32::MAX,
                &table,
                &area,
                &mut scratch,
                CollisionOwner::None,
            )
            .with_native_entities(Some(&local));
            assert_eq!(
                invalid_world.point_contents(Vec3::default(), rules, &[]),
                Err(qa_world::collision::TraceError::Entity)
            );
            let mut invalid_world = invalid_world;
            assert!(matches!(
                invalid_world.trace(query(rules)),
                Err(qa_world::collision::TraceError::Entity)
            ));
            let foreign_failure = View {
                module: ModuleId(2),
                enabled: None,
                anchor: None,
                reads: Cell::new(0),
            };
            let mut foreign_trace = WorldTrace::new(
                &world.store,
                world.geometry,
                0,
                &table,
                &area,
                &mut scratch,
                CollisionOwner::None,
            )
            .with_foreign_entities(Some(&foreign_failure))
            .with_candidates(&candidates);
            assert!(matches!(
                foreign_trace.trace(query(rules)),
                Err(qa_world::collision::TraceError::Entity)
            ));
            assert_eq!(
                foreign_trace.point_contents(Vec3([-20.0, 0.0, 0.0]), rules, &[]),
                Err(qa_world::collision::TraceError::Entity)
            );
        } else {
            let trace = trace.unwrap();
            assert_eq!(trace.fraction, 1.0);
            assert_eq!(trace.entity, table.id_at(0));
        }
        assert_eq!(foreign.reads.get(), 0);
    }

    // An invalid module lifetime is ineligible before any native field read.
    foreign.anchor = Some(EntityId {
        generation: u32::MAX,
        ..local_body
    });
    let trace = WorldTrace::new(
        &world.store,
        world.geometry,
        0,
        &table,
        &area,
        &mut scratch,
        CollisionOwner::None,
    )
    .with_foreign_entities(Some(&foreign))
    .with_candidates(&candidates)
    .trace(query(rules))
    .unwrap();
    assert_eq!(trace.entity, Some(local_body));
    assert_eq!(foreign.reads.get(), 0);
    foreign.anchor = Some(foreign_body);
    let trace = WorldTrace::new(
        &world.store,
        world.geometry,
        0,
        &table,
        &area,
        &mut scratch,
        CollisionOwner::None,
    )
    .with_foreign_entities(Some(&foreign))
    .with_candidates(&candidates)
    .trace(query(rules))
    .unwrap();
    assert_eq!(trace.entity, Some(local_body));
    assert_eq!(foreign.reads.get(), 0);
}

#[test]
fn stale_linked_geometry_rejects_the_whole_query_instead_of_a_partial_hit() {
    let mut world = brush_world(
        vec![Plane::oriented(Vec3([-1.0, 0.0, 0.0]), -24.0)],
        vec![Brush {
            sides: qa_core::primitives::IndexSpan { first: 0, count: 1 },
            contents: Contents::SOLID,
        }],
    );
    let (mut table, mut area) = table();
    let id = body(
        &mut table,
        &mut area,
        Vec3::default(),
        Vec3([-2.0; 3]),
        Vec3([2.0; 3]),
    );
    let stale = world
        .store
        .load_brushes(
            Vec::new(),
            Vec::new(),
            Vec::new(),
            qa_world::collision::surfaces::SurfaceTable::flags(Vec::new()),
            BrushTree::direct(0).unwrap(),
            vec![Bounds {
                mins: Vec3([-4.0; 3]),
                maxs: Vec3([4.0; 3]),
            }],
        )
        .unwrap();
    table.columns.collision_shape[id.slot as usize] = CollisionShape::Model {
        geometry: stale,
        index: 0,
    };
    let mut scratch = world.store.scratch();
    assert!(world.store.remove(stale));
    let rules = qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake2).1;
    let input = query(rules);
    let native_world = world
        .store
        .trace_model(world.geometry, 0, input, &mut scratch)
        .expect("valid world");
    assert!(native_world.fraction < 1.0 && native_world.fraction > 0.0);
    let mut trace = WorldTrace::new(
        &world.store,
        world.geometry,
        0,
        &table,
        &area,
        &mut scratch,
        CollisionOwner::None,
    );
    assert_eq!(
        trace.trace(input).unwrap_err(),
        qa_world::collision::TraceError::Geometry {
            geometry: stale,
            model: 0
        }
    );
    assert_eq!(
        trace.point_contents(Vec3::default(), rules, &[]),
        Err(qa_world::collision::TraceError::Geometry {
            geometry: stale,
            model: 0
        })
    );
}
