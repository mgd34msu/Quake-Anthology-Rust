use qa_core::primitives::*;
use qa_session::dispatch::*;
use qa_world::entities::AllocationPolicy;
use qa_world::entities::EntityTable;

struct World {
    entities: EntityTable,
    calls: Vec<(u32, u16, u32, f64)>,
    remove: Option<EntityId>,
}
impl ThinkWorld for World {
    fn entities(&mut self) -> &mut EntityTable {
        &mut self.entities
    }
}

fn think(world: &mut World, module: ModuleId, entry: u32, call: CallbackCall) -> bool {
    let CallbackCall::Think { entity, time } = call else {
        return false;
    };
    assert!(world.entities.columns.next_think[entity.slot as usize].is_none());
    world.calls.push((entity.slot, module.0, entry, time));
    if let Some(remove) = world.remove.take() {
        assert!(world.entities.release(remove, time));
    }
    world.entities.columns.next_think[entity.slot as usize] = Some(Think {
        at: time,
        callback: CallbackId(0),
    });
    true
}

#[test]
fn slot_order_clear_before_callback_clamped_time_and_removed_lifetimes() {
    let mut world = World {
        entities: EntityTable::new(16, 1).unwrap(),
        calls: Vec::new(),
        remove: None,
    };
    let ids: Vec<_> = (1..=3)
        .map(|module| {
            world
                .entities
                .allocate(1.0, ModuleId(module), AllocationPolicy::EDICT)
                .unwrap()
                .id
        })
        .collect();
    for &id in &ids {
        world.entities.columns.next_think[id.slot as usize] = Some(Think {
            at: 0.5,
            callback: CallbackId(0),
        });
    }
    world.remove = Some(ids[1]);
    let table = FunctionTable::load((1..=3).map(|module| {
        (
            ModuleId(module),
            ThinkTiming::FrameEnd,
            vec![FunctionBinding {
                entry: u32::from(module) * 10,
                call: think,
            }],
        )
    }))
    .unwrap();
    let stats = run_thinks(&mut world, &table, 1.0, 1.01);
    assert_eq!((stats.called, stats.rejected), (2, 0));
    assert_eq!(world.calls, [(1, 1, 10, 1.0), (3, 3, 30, 1.0)]);
    assert!(world.entities.resolve(ids[1]).is_none());
    // A callback can reschedule itself, but cannot fire twice in one scan.
    assert_eq!(world.entities.columns.next_think[1].unwrap().at, 1.0);
    assert_eq!(run_thinks(&mut world, &table, 1.01, 1.02).called, 2);
}

#[test]
fn bad_module_or_callback_only_rejects_that_entity_and_due_boundary_is_inclusive() {
    let mut world = World {
        entities: EntityTable::new(16, 1).unwrap(),
        calls: Vec::new(),
        remove: None,
    };
    let first = world
        .entities
        .allocate(1.0, ModuleId(7), AllocationPolicy::EDICT)
        .unwrap()
        .id;
    let second = world
        .entities
        .allocate(1.0, ModuleId(1), AllocationPolicy::EDICT)
        .unwrap()
        .id;
    let third = world
        .entities
        .allocate(1.0, ModuleId(1), AllocationPolicy::EDICT)
        .unwrap()
        .id;
    for (id, callback, at) in [
        (first, CallbackId(0), 1.0),
        (second, CallbackId(4), 1.0),
        (third, CallbackId(0), 1.01),
    ] {
        world.entities.columns.next_think[id.slot as usize] = Some(Think { at, callback });
    }
    let table = FunctionTable::load([(
        ModuleId(1),
        ThinkTiming::FrameEnd,
        vec![FunctionBinding {
            entry: 5,
            call: think,
        }],
    )])
    .unwrap();
    let stats = run_thinks(&mut world, &table, 1.0, 1.01);
    assert_eq!((stats.called, stats.rejected), (1, 2));
    assert_eq!(world.calls, [(third.slot, 1, 5, 1.01)]);
    assert_eq!(
        table.invoke(
            &mut world,
            CallbackId(0),
            CallbackCall::Think {
                entity: EntityId {
                    generation: 0,
                    ..third
                },
                time: 1.0
            }
        ),
        Err(CallError::StaleEntity)
    );
}

#[test]
fn all_entity_reactions_share_one_numeric_function_table() {
    let mut world = World {
        entities: EntityTable::new(4, 1).unwrap(),
        calls: Vec::new(),
        remove: None,
    };
    let id = world
        .entities
        .allocate(1.0, ModuleId(2), AllocationPolicy::EDICT)
        .unwrap()
        .id;
    fn reaction(world: &mut World, module: ModuleId, entry: u32, call: CallbackCall) -> bool {
        world.calls.push((call.entity().slot, module.0, entry, 0.0));
        true
    }
    let table = FunctionTable::load([(
        ModuleId(2),
        ThinkTiming::Current { tolerance: 0.001 },
        vec![FunctionBinding {
            entry: 77,
            call: reaction,
        }],
    )])
    .unwrap();
    let event = DamageEvent {
        target: id,
        attacker: None,
        inflictor: None,
        amount: 10.0,
        knockback: 0,
        direction: None,
        point: Vec3::default(),
        flags: DamageFlags::default(),
    };
    for call in [
        CallbackCall::Touch {
            entity: id,
            other: id,
        },
        CallbackCall::Use {
            entity: id,
            activator: id,
        },
        CallbackCall::Blocked {
            entity: id,
            other: id,
        },
        CallbackCall::Pain {
            event,
            taken: 10,
            knockback: 0,
        },
        CallbackCall::Die { event, taken: 10 },
    ] {
        table.invoke(&mut world, CallbackId(0), call).unwrap();
    }
    assert_eq!(world.calls, [(id.slot, 2, 77, 0.0); 5]);
    world.entities.columns.pain[id.slot as usize] = Some(CallbackId(0));
    world.entities.columns.die[id.slot as usize] = Some(CallbackId(0));
    assert!(world.entities.release(id, 1.0));
    assert!(world.entities.columns.pain[id.slot as usize].is_none());
    assert!(world.entities.columns.die[id.slot as usize].is_none());
}

#[test]
fn mixed_entities_preserve_q1_lookahead_q2_tolerance_and_q3_current_time() {
    let mut world = World {
        entities: EntityTable::new(8, 1).unwrap(),
        calls: Vec::new(),
        remove: None,
    };
    for module in 1..=3 {
        let id = world
            .entities
            .allocate(1.0, ModuleId(module), AllocationPolicy::EDICT)
            .unwrap()
            .id;
        world.entities.columns.next_think[id.slot as usize] = Some(Think {
            at: 1.0009,
            callback: CallbackId(0),
        });
    }
    let table = FunctionTable::load([
        (
            ModuleId(1),
            ThinkTiming::FrameEnd,
            vec![FunctionBinding {
                entry: 1,
                call: think,
            }],
        ),
        (
            ModuleId(2),
            ThinkTiming::Current { tolerance: 0.001 },
            vec![FunctionBinding {
                entry: 2,
                call: think,
            }],
        ),
        (
            ModuleId(3),
            ThinkTiming::Current { tolerance: 0.0 },
            vec![FunctionBinding {
                entry: 3,
                call: think,
            }],
        ),
    ])
    .unwrap();
    assert_eq!(run_thinks(&mut world, &table, 1.0, 1.01).called, 2);
    assert_eq!(world.calls, [(1, 1, 1, 1.0009), (2, 2, 2, 1.0)]);
    world.entities.columns.next_think[1] = None;
    world.entities.columns.next_think[2] = Some(Think {
        at: 1.0011,
        callback: CallbackId(0),
    });
    assert_eq!(run_thinks(&mut world, &table, 1.0, 1.01).called, 0);
    assert_eq!(run_thinks(&mut world, &table, 1.001, 1.01).called, 2);
}
