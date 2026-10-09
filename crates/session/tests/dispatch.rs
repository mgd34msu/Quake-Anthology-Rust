use qa_core::primitives::*;
use qa_session::dispatch::*;
use qa_world::entities::{AllocationPolicy, EntityTable, EntityTime};

struct World {
    entities: EntityTable,
    calls: Vec<(u32, u16, u32, ThinkTime)>,
    remove: Option<EntityId>,
    reschedule: Option<ThinkTime>,
    replacement: Option<EntityId>,
    bindings: [ModuleBinding; 8],
}
impl ThinkWorld for World {
    fn entities(&mut self) -> &mut EntityTable {
        &mut self.entities
    }
}

fn world(capacity: usize) -> World {
    World {
        entities: EntityTable::new(capacity, 1).unwrap(),
        calls: Vec::new(),
        remove: None,
        reschedule: None,
        replacement: None,
        bindings: [ModuleBinding::default(); 8],
    }
}

fn entity(world: &mut World, module: u16) -> EntityId {
    world
        .entities
        .allocate(1.0, ModuleId(module), AllocationPolicy::EDICT)
        .unwrap()
        .id
}

fn schedule(world: &mut World, entity: EntityId, at: ThinkTime, callback: Option<CallbackId>) {
    world.entities.columns.next_think[entity.slot as usize] = Some(at);
    set_function(world, entity.slot as usize, callback);
}

fn set_function(world: &mut World, slot: usize, callback: Option<CallbackId>) {
    let module = world.entities.columns.owner[slot];
    world
        .entities
        .columns
        .set_think_function(slot, world.bindings[usize::from(module.0)].think(callback));
}

fn install(world: &mut World, table: &FunctionTable<World>) {
    world.bindings = std::array::from_fn(|module| table.module_binding(ModuleId(module as u16)));
    let mut start = 0;
    while let Some(id) = world.entities.next_active(start) {
        start = id.slot as usize + 1;
        let callback = world.entities.columns.think_function(id.slot as usize);
        assert!(table.bind_think(&mut world.entities, id, callback));
    }
}

fn frame(now: f64, step: f64) -> Option<ThinkFrame> {
    Some(ThinkFrame::Seconds { now, step })
}

fn record(world: &mut World, module: ModuleId, entry: u32, call: CallbackCall) -> bool {
    let CallbackCall::Think { entity, time } = call else {
        return false;
    };
    let slot = world.entities.resolve(entity).unwrap();
    assert!(world.entities.columns.next_think[slot].is_none());
    assert!(world.entities.columns.think_function(slot).is_some());
    world.calls.push((entity.slot, module.0, entry, time));
    if let Some(remove) = world.remove.take() {
        let now = match time {
            ThinkTime::Seconds(value) => EntityTime::Seconds(value),
            ThinkTime::Milliseconds(value) => EntityTime::Milliseconds(value),
        };
        assert!(world.entities.release(remove, now));
    }
    world.entities.columns.next_think[slot] = world.reschedule;
    true
}

fn table(timing: RuleSetId) -> FunctionTable<World> {
    FunctionTable::load([(
        ModuleId(1),
        timing,
        vec![FunctionBinding {
            entry: 5,
            call: record,
        }],
    )])
    .unwrap()
}

#[test]
fn slot_order_clear_before_callback_clamped_time_and_removed_lifetimes() {
    let mut world = world(16);
    let ids: Vec<_> = (1..=3).map(|module| entity(&mut world, module)).collect();
    for &id in &ids {
        schedule(&mut world, id, ThinkTime::Seconds(0.5), Some(CallbackId(0)));
    }
    world.remove = Some(ids[1]);
    world.reschedule = Some(ThinkTime::Seconds(1.0));
    let table = FunctionTable::load((1..=3).map(|module| {
        (
            ModuleId(module),
            RuleSetId::Quake,
            vec![FunctionBinding {
                entry: u32::from(module) * 10,
                call: record,
            }],
        )
    }))
    .unwrap();
    install(&mut world, &table);
    let stats = run_thinks(&mut world, &table, |_| frame(1.0, 0.01));
    assert_eq!((stats.called, stats.rejected), (2, 0));
    assert_eq!(
        world.calls,
        [
            (1, 1, 10, ThinkTime::Seconds(1.0)),
            (3, 3, 30, ThinkTime::Seconds(1.0)),
        ]
    );
    assert!(world.entities.resolve(ids[1]).is_none());
    assert!(
        world
            .entities
            .columns
            .think_function(ids[1].slot as usize)
            .is_none()
    );
    // NetQuake does not repeat a callback which reschedules into this frame.
    assert_eq!(world.entities.columns.next_think[1], world.reschedule);
    assert_eq!(
        world.entities.columns.think_function(1),
        Some(CallbackId(0))
    );
    assert_eq!(
        run_thinks(&mut world, &table, |_| frame(1.01, 0.01)).called,
        2
    );
}

#[test]
fn missing_modules_do_not_inherit_quake_timing_and_bad_functions_are_scoped() {
    let mut world = world(16);
    let missing = entity(&mut world, 7);
    let bad_function = entity(&mut world, 1);
    let valid = entity(&mut world, 1);
    schedule(
        &mut world,
        missing,
        ThinkTime::Seconds(10.0),
        Some(CallbackId(0)),
    );
    schedule(
        &mut world,
        bad_function,
        ThinkTime::Seconds(1.0),
        Some(CallbackId(4)),
    );
    schedule(
        &mut world,
        valid,
        ThinkTime::Seconds(1.0),
        Some(CallbackId(0)),
    );
    let table = table(RuleSetId::Quake);
    install(&mut world, &table);
    let stats = run_thinks(&mut world, &table, |_| frame(1.0, 0.0));
    assert_eq!((stats.called, stats.rejected), (1, 2));
    assert_eq!(world.calls, [(valid.slot, 1, 5, ThinkTime::Seconds(1.0))]);
    assert_eq!(
        world.entities.columns.next_think[missing.slot as usize],
        Some(ThinkTime::Seconds(10.0))
    );
    assert!(world.entities.columns.next_think[bad_function.slot as usize].is_none());
    assert_eq!(
        world
            .entities
            .columns
            .think_function(bad_function.slot as usize),
        Some(CallbackId(4))
    );
    assert_eq!(
        table.invoke(
            &mut world,
            CallbackId(0),
            CallbackCall::Think {
                entity: EntityId {
                    generation: 0,
                    ..valid
                },
                time: ThinkTime::Seconds(1.0),
            }
        ),
        Err(CallError::StaleEntity)
    );
}

#[test]
fn null_callback_clears_only_the_due_deadline() {
    let mut world = world(4);
    let id = entity(&mut world, 1);
    let think = Think {
        at: Some(ThinkTime::Seconds(1.0)),
        callback: None,
    };
    world.entities.columns.next_think[id.slot as usize] = think.at;
    set_function(&mut world, id.slot as usize, think.callback);
    let table = table(RuleSetId::Quake2);
    install(&mut world, &table);
    let result = run_think(&mut world, &table, id, |_| frame(1.0, 0.0));
    assert_eq!(
        result,
        ThinkResult {
            called: 0,
            rejected: 1,
            current_lifetime: true,
        }
    );
    assert!(world.entities.columns.next_think[id.slot as usize].is_none());
    assert!(
        world
            .entities
            .columns
            .think_function(id.slot as usize)
            .is_none()
    );
    assert!(world.calls.is_empty());
}

#[test]
fn missing_clock_and_wrong_units_do_not_consume_a_deadline() {
    let mut world = world(4);
    let id = entity(&mut world, 1);
    schedule(&mut world, id, ThinkTime::Seconds(1.0), Some(CallbackId(0)));
    let table = table(RuleSetId::Quake);
    install(&mut world, &table);
    let missing = run_think(&mut world, &table, id, |_| None);
    assert_eq!(
        (missing.called, missing.rejected, missing.current_lifetime),
        (0, 1, true)
    );
    let wrong = run_think(&mut world, &table, id, |_| {
        Some(ThinkFrame::Milliseconds { now: 1000 })
    });
    assert_eq!((wrong.called, wrong.rejected), (0, 1));
    assert_eq!(
        world.entities.columns.next_think[id.slot as usize],
        Some(ThinkTime::Seconds(1.0))
    );
    assert_eq!(
        world.entities.columns.think_function(id.slot as usize),
        Some(CallbackId(0))
    );
    assert!(world.calls.is_empty());
}

#[test]
fn quake_deadline_and_past_time_clamp_use_native_float_width() {
    let mut world = world(4);
    let id = entity(&mut world, 1);
    let table = table(RuleSetId::Quake);
    install(&mut world, &table);
    schedule(
        &mut world,
        id,
        ThinkTime::Seconds(1.00000005),
        Some(CallbackId(0)),
    );
    assert_eq!(
        run_think(&mut world, &table, id, |_| frame(1.0, 0.0)).called,
        1
    );
    assert_eq!(world.calls[0].3, ThinkTime::Seconds(1.0));

    let above = f64::from(f32::from_bits(1.0f32.to_bits() + 1));
    schedule(
        &mut world,
        id,
        ThinkTime::Seconds(above),
        Some(CallbackId(0)),
    );
    assert_eq!(
        run_think(&mut world, &table, id, |_| frame(1.0, (above - 1.0) * 0.5)).called,
        0
    );
    assert_eq!(
        run_think(&mut world, &table, id, |_| frame(1.0, above - 1.0)).called,
        1
    );
    assert_eq!(world.calls[1].3, ThinkTime::Seconds(above));

    schedule(&mut world, id, ThinkTime::Seconds(0.5), Some(CallbackId(0)));
    assert_eq!(
        run_think(&mut world, &table, id, |_| frame(1.00000009, 0.0)).called,
        1
    );
    // The assignment back to float rounds above the double current time.
    assert_eq!(world.calls[2].3, ThinkTime::Seconds(above));
    assert!(world.entities.columns.next_think[id.slot as usize].is_none());
    assert_eq!(
        world.entities.columns.think_function(id.slot as usize),
        Some(CallbackId(0))
    );
}

#[test]
fn quake2_tolerance_is_double_after_native_float_clock_narrowing() {
    let mut world = world(4);
    let id = entity(&mut world, 1);
    let table = table(RuleSetId::Quake2);
    install(&mut world, &table);
    schedule(
        &mut world,
        id,
        ThinkTime::Seconds(1.001),
        Some(CallbackId(0)),
    );
    // float(1.001) is above double(1.0f)+0.001, despite the decimal label.
    assert_eq!(
        run_think(&mut world, &table, id, |_| frame(1.0, 99.0)).called,
        0
    );
    let below = f64::from(f32::from_bits(1.001f32.to_bits() - 1));
    schedule(
        &mut world,
        id,
        ThinkTime::Seconds(below),
        Some(CallbackId(0)),
    );
    assert_eq!(
        run_think(&mut world, &table, id, |_| frame(1.0, 0.0)).called,
        1
    );
    assert_eq!(world.calls[0].3, ThinkTime::Seconds(1.0));

    schedule(&mut world, id, ThinkTime::Seconds(0.5), Some(CallbackId(0)));
    assert_eq!(
        run_think(&mut world, &table, id, |_| frame(1.00000006, 0.0)).called,
        1
    );
    assert_eq!(
        world.calls[1].3,
        ThinkTime::Seconds(f64::from(f32::from_bits(1.0f32.to_bits() + 1)))
    );
}

#[test]
fn rerelease_deadlines_keep_all_integer_milliseconds() {
    let mut world = world(4);
    let id = entity(&mut world, 1);
    let table = table(RuleSetId::Quake2Rerelease);
    install(&mut world, &table);
    let due = 9_007_199_254_740_993;
    schedule(
        &mut world,
        id,
        ThinkTime::Milliseconds(due),
        Some(CallbackId(0)),
    );
    assert_eq!(
        run_think(&mut world, &table, id, |_| Some(ThinkFrame::Milliseconds {
            now: due - 1
        }))
        .called,
        0
    );
    assert_eq!(
        world.entities.columns.next_think[id.slot as usize],
        Some(ThinkTime::Milliseconds(due))
    );
    assert_eq!(
        run_think(&mut world, &table, id, |_| Some(ThinkFrame::Milliseconds {
            now: due
        }))
        .called,
        1
    );
    assert_eq!(world.calls[0].3, ThinkTime::Milliseconds(due));
}

#[test]
fn quake3_due_comparison_rounds_to_float_but_callback_time_stays_integer() {
    let mut world = world(4);
    let id = entity(&mut world, 1);
    let table = table(RuleSetId::Quake3);
    install(&mut world, &table);
    let boundary = 16_777_216;
    schedule(
        &mut world,
        id,
        ThinkTime::Milliseconds(boundary + 1),
        Some(CallbackId(0)),
    );
    assert_eq!(
        run_think(&mut world, &table, id, |_| Some(ThinkFrame::Milliseconds {
            now: boundary
        }))
        .called,
        1
    );
    assert_eq!(world.calls[0].3, ThinkTime::Milliseconds(boundary));
    schedule(
        &mut world,
        id,
        ThinkTime::Milliseconds(boundary),
        Some(CallbackId(0)),
    );
    assert_eq!(
        run_think(&mut world, &table, id, |_| Some(ThinkFrame::Milliseconds {
            now: boundary + 1
        }))
        .called,
        1
    );
    assert_eq!(world.calls[1].3, ThinkTime::Milliseconds(boundary + 1));
}

#[test]
fn mixed_modules_resolve_independent_seconds_and_millisecond_frames() {
    let mut world = world(8);
    let specs = [
        (
            RuleSetId::Quake,
            ThinkTime::Seconds(1.5),
            ThinkFrame::Seconds {
                now: 1.0,
                step: 0.5,
            },
            ThinkTime::Seconds(1.5),
        ),
        (
            RuleSetId::QuakeWorld,
            ThinkTime::Seconds(2.0),
            ThinkFrame::Seconds {
                now: 2.0,
                step: 0.01,
            },
            ThinkTime::Seconds(2.0),
        ),
        (
            RuleSetId::Quake2,
            ThinkTime::Seconds(10.0009),
            ThinkFrame::Seconds {
                now: 10.0,
                step: 100.0,
            },
            ThinkTime::Seconds(10.0),
        ),
        (
            RuleSetId::Quake2Rerelease,
            ThinkTime::Milliseconds(5000),
            ThinkFrame::Milliseconds { now: 5000 },
            ThinkTime::Milliseconds(5000),
        ),
        (
            RuleSetId::Quake3,
            ThinkTime::Milliseconds(23000),
            ThinkFrame::Milliseconds { now: 23000 },
            ThinkTime::Milliseconds(23000),
        ),
    ];
    let ids: Vec<_> = specs
        .iter()
        .enumerate()
        .map(|(index, (_, at, _, _))| {
            let id = entity(&mut world, index as u16 + 1);
            schedule(&mut world, id, *at, Some(CallbackId(0)));
            id
        })
        .collect();
    let table = FunctionTable::load(specs.iter().enumerate().map(|(index, (timing, _, _, _))| {
        (
            ModuleId(index as u16 + 1),
            *timing,
            vec![FunctionBinding {
                entry: index as u32,
                call: record,
            }],
        )
    }))
    .unwrap();
    install(&mut world, &table);
    let mut clocks = Vec::new();
    let stats = run_thinks(&mut world, &table, |module| {
        clocks.push(module);
        Some(specs[module.0 as usize - 1].2)
    });
    assert_eq!((stats.called, stats.rejected), (5, 0));
    assert_eq!(
        clocks,
        [
            ModuleId(1),
            ModuleId(2),
            ModuleId(3),
            ModuleId(4),
            ModuleId(5)
        ]
    );
    for (index, id) in ids.into_iter().enumerate() {
        assert_eq!(
            world.calls[index],
            (id.slot, index as u16 + 1, index as u32, specs[index].3)
        );
        assert!(world.entities.columns.next_think[id.slot as usize].is_none());
        assert_eq!(
            world.entities.columns.think_function(id.slot as usize),
            Some(CallbackId(0))
        );
    }
}

fn chain(world: &mut World, module: ModuleId, entry: u32, call: CallbackCall) -> bool {
    assert!(record(world, module, entry, call));
    let slot = call.entity().slot as usize;
    match entry {
        10 => {
            world.entities.columns.next_think[slot] = Some(ThinkTime::Seconds(0.6));
            set_function(world, slot, Some(CallbackId(1)));
        }
        20 => {
            world.entities.columns.next_think[slot] = Some(ThinkTime::Seconds(0.7));
            set_function(world, slot, Some(CallbackId(2)));
        }
        _ => {}
    }
    true
}

#[test]
fn quakeworld_rereads_function_and_finishes_reschedules_before_next_entity() {
    let mut world = world(8);
    let first = entity(&mut world, 1);
    let second = entity(&mut world, 2);
    schedule(
        &mut world,
        first,
        ThinkTime::Seconds(0.5),
        Some(CallbackId(0)),
    );
    schedule(
        &mut world,
        second,
        ThinkTime::Seconds(0.5),
        Some(CallbackId(0)),
    );
    let table = FunctionTable::load([
        (
            ModuleId(1),
            RuleSetId::QuakeWorld,
            [10, 20, 30]
                .into_iter()
                .map(|entry| FunctionBinding { entry, call: chain })
                .collect(),
        ),
        (
            ModuleId(2),
            RuleSetId::Quake,
            vec![FunctionBinding {
                entry: 40,
                call: record,
            }],
        ),
    ])
    .unwrap();
    install(&mut world, &table);
    let stats = run_thinks(&mut world, &table, |_| frame(0.5, 0.25));
    assert_eq!((stats.called, stats.rejected), (4, 0));
    assert_eq!(
        world.calls,
        [
            (first.slot, 1, 10, ThinkTime::Seconds(0.5)),
            (first.slot, 1, 20, ThinkTime::Seconds(f64::from(0.6f32))),
            (first.slot, 1, 30, ThinkTime::Seconds(f64::from(0.7f32))),
            (second.slot, 2, 40, ThinkTime::Seconds(0.5)),
        ]
    );
    assert_eq!(
        world.entities.columns.think_function(first.slot as usize),
        Some(CallbackId(2))
    );
    assert!(world.entities.columns.next_think[first.slot as usize].is_none());
}

#[test]
fn quakeworld_re_resolves_owner_and_its_clock_after_a_callback() {
    fn transfer(world: &mut World, module: ModuleId, entry: u32, call: CallbackCall) -> bool {
        assert!(record(world, module, entry, call));
        let slot = call.entity().slot as usize;
        world.entities.columns.owner[slot] = ModuleId(2);
        world.entities.columns.next_think[slot] = Some(ThinkTime::Milliseconds(42));
        set_function(world, slot, Some(CallbackId(0)));
        true
    }
    let mut world = world(4);
    let id = entity(&mut world, 1);
    schedule(&mut world, id, ThinkTime::Seconds(0.5), Some(CallbackId(0)));
    let table = FunctionTable::load([
        (
            ModuleId(1),
            RuleSetId::QuakeWorld,
            vec![FunctionBinding {
                entry: 10,
                call: transfer,
            }],
        ),
        (
            ModuleId(2),
            RuleSetId::Quake2Rerelease,
            vec![FunctionBinding {
                entry: 20,
                call: record,
            }],
        ),
    ])
    .unwrap();
    install(&mut world, &table);
    let result = run_think(&mut world, &table, id, |module| match module.0 {
        1 => frame(0.5, 0.25),
        2 => Some(ThinkFrame::Milliseconds { now: 42 }),
        _ => None,
    });
    assert_eq!(
        (result.called, result.rejected, result.current_lifetime),
        (2, 0, true)
    );
    assert_eq!(
        world.calls,
        [
            (id.slot, 1, 10, ThinkTime::Seconds(0.5)),
            (id.slot, 2, 20, ThinkTime::Milliseconds(42)),
        ]
    );
}

#[test]
fn quakeworld_rejection_stops_rescheduling_and_preserves_the_callback() {
    fn reject(world: &mut World, module: ModuleId, entry: u32, call: CallbackCall) -> bool {
        assert!(record(world, module, entry, call));
        world.entities.columns.next_think[call.entity().slot as usize] =
            Some(ThinkTime::Seconds(0.5));
        false
    }
    let mut world = world(4);
    let id = entity(&mut world, 1);
    schedule(&mut world, id, ThinkTime::Seconds(0.5), Some(CallbackId(0)));
    let table = FunctionTable::load([(
        ModuleId(1),
        RuleSetId::QuakeWorld,
        vec![FunctionBinding {
            entry: 10,
            call: reject,
        }],
    )])
    .unwrap();
    install(&mut world, &table);
    let result = run_think(&mut world, &table, id, |_| frame(0.5, 0.25));
    assert_eq!(
        (result.called, result.rejected, result.current_lifetime),
        (0, 1, true)
    );
    assert_eq!(world.calls.len(), 1);
    assert_eq!(
        world.entities.columns.next_think[id.slot as usize],
        Some(ThinkTime::Seconds(0.5))
    );
    assert_eq!(
        world.entities.columns.think_function(id.slot as usize),
        Some(CallbackId(0))
    );
}

#[test]
fn quakeworld_removal_and_slot_reuse_stop_the_old_generation() {
    fn replace(world: &mut World, module: ModuleId, entry: u32, call: CallbackCall) -> bool {
        assert!(record(world, module, entry, call));
        let id = call.entity();
        assert!(world.entities.release(id, 1.0));
        let replacement = world
            .entities
            .allocate(2.0, module, AllocationPolicy::EDICT)
            .unwrap()
            .id;
        assert_eq!(replacement.slot, id.slot);
        assert!(
            world
                .entities
                .columns
                .think_function(replacement.slot as usize)
                .is_none()
        );
        schedule(
            world,
            replacement,
            ThinkTime::Seconds(0.5),
            Some(CallbackId(0)),
        );
        world.replacement = Some(replacement);
        true
    }
    let mut world = world(2);
    let id = entity(&mut world, 1);
    schedule(&mut world, id, ThinkTime::Seconds(0.5), Some(CallbackId(0)));
    let table = FunctionTable::load([(
        ModuleId(1),
        RuleSetId::QuakeWorld,
        vec![FunctionBinding {
            entry: 10,
            call: replace,
        }],
    )])
    .unwrap();
    install(&mut world, &table);
    let result = run_think(&mut world, &table, id, |_| frame(0.5, 0.25));
    assert_eq!(
        (result.called, result.rejected, result.current_lifetime),
        (1, 0, false)
    );
    assert_eq!(world.calls.len(), 1);
    assert!(world.entities.resolve(id).is_none());
    assert!(world.entities.resolve(world.replacement.unwrap()).is_some());
    let stale = run_think(&mut world, &table, id, |_| frame(0.5, 0.25));
    assert_eq!(stale, ThinkResult::default());
}

#[test]
fn all_entity_reactions_share_one_numeric_function_table() {
    let mut world = world(4);
    let id = entity(&mut world, 2);
    fn reaction(world: &mut World, module: ModuleId, entry: u32, call: CallbackCall) -> bool {
        world
            .calls
            .push((call.entity().slot, module.0, entry, ThinkTime::Seconds(0.0)));
        true
    }
    let table = FunctionTable::load([(
        ModuleId(2),
        RuleSetId::Quake2,
        vec![FunctionBinding {
            entry: 77,
            call: reaction,
        }],
    )])
    .unwrap();
    install(&mut world, &table);
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
    assert_eq!(world.calls, [(id.slot, 2, 77, ThinkTime::Seconds(0.0)); 5]);
    world.entities.columns.pain[id.slot as usize] = Some(CallbackId(0));
    world.entities.columns.die[id.slot as usize] = Some(CallbackId(0));
    assert!(world.entities.release(id, 1.0));
    assert!(world.entities.columns.pain[id.slot as usize].is_none());
    assert!(world.entities.columns.die[id.slot as usize].is_none());
}

#[test]
fn function_handles_preserve_native_indices_above_sixteen_bits() {
    // qsrc pr_comp.h func_t and dprograms_t.numfunctions are signed int,
    // not a 16-bit wire field. The shared table must not truncate an index.
    let mut world = world(4);
    let id = entity(&mut world, 1);
    let index = 65_536;
    let table = FunctionTable::load([(
        ModuleId(1),
        RuleSetId::Quake,
        (0..=index)
            .map(|entry| FunctionBinding {
                entry,
                call: record,
            })
            .collect(),
    )])
    .unwrap();
    install(&mut world, &table);
    schedule(
        &mut world,
        id,
        ThinkTime::Seconds(1.0),
        Some(CallbackId(index)),
    );
    let result = run_think(&mut world, &table, id, |_| frame(1.0, 0.01));
    assert_eq!((result.called, result.rejected), (1, 0));
    assert_eq!(world.calls[0].2, index);
    assert_eq!(
        world.entities.columns.think_function(id.slot as usize),
        Some(CallbackId(index))
    );
}

#[test]
fn stale_binding_cannot_change_a_reused_lifetime() {
    let mut world = world(2);
    let table = table(RuleSetId::Quake2);
    install(&mut world, &table);
    let old = entity(&mut world, 1);
    assert!(table.bind_think(&mut world.entities, old, Some(CallbackId(0))));
    assert!(world.entities.release(old, 0.0));
    let current = entity(&mut world, 1);
    assert_eq!(old.slot, current.slot);
    assert_ne!(old.generation, current.generation);
    assert_eq!(
        world.entities.columns.think_binding(current.slot as usize),
        ThinkBinding::default()
    );
    assert!(!table.bind_think(&mut world.entities, old, Some(CallbackId(0))));
    schedule(
        &mut world,
        current,
        ThinkTime::Seconds(1.0),
        Some(CallbackId(0)),
    );
    let result = run_think(&mut world, &table, current, |_| frame(1.0, 0.0));
    assert_eq!((result.called, result.rejected), (1, 0));
}

#[test]
fn native_function_write_refreshes_only_the_bound_path() {
    let mut world = world(4);
    let id = entity(&mut world, 1);
    let table = table(RuleSetId::Quake2);
    install(&mut world, &table);
    let slot = id.slot as usize;
    world.entities.columns.next_think[slot] = Some(ThinkTime::Seconds(2.0));
    assert!(table.bind_think(&mut world.entities, id, Some(CallbackId(u32::MAX))));
    assert_eq!(
        world.entities.columns.think_function(slot),
        Some(CallbackId(u32::MAX))
    );
    // Invalid native functions are rejected only when due, after clearing the
    // deadline, just like null native callbacks. No invented function limit.
    assert_eq!(
        run_think(&mut world, &table, id, |_| frame(1.0, 0.0)).rejected,
        0
    );
    assert_eq!(
        world.entities.columns.next_think[slot],
        Some(ThinkTime::Seconds(2.0))
    );
    assert_eq!(
        run_think(&mut world, &table, id, |_| frame(2.0, 0.0)).rejected,
        1
    );
    assert!(world.entities.columns.next_think[slot].is_none());
    assert!(table.bind_think(&mut world.entities, id, Some(CallbackId(0))));
    world.entities.columns.next_think[slot] = Some(ThinkTime::Seconds(2.0));
    assert_eq!(
        run_think(&mut world, &table, id, |_| frame(2.0, 0.0)).called,
        1
    );
    assert_eq!(world.calls, [(id.slot, 1, 5, ThinkTime::Seconds(2.0))]);
}

enum ActiveMutation {
    RemoveFuture(EntityId),
    SpawnLower,
    OverwriteLast,
}

struct ActiveMutationWorld {
    entities: EntityTable,
    calls: Vec<EntityId>,
    mutation: Option<ActiveMutation>,
    spawned: Option<EntityId>,
    binding: ModuleBinding,
}

impl ThinkWorld for ActiveMutationWorld {
    fn entities(&mut self) -> &mut EntityTable {
        &mut self.entities
    }
}

fn mutate_active_slots(
    world: &mut ActiveMutationWorld,
    module: ModuleId,
    _entry: u32,
    call: CallbackCall,
) -> bool {
    let CallbackCall::Think { entity, .. } = call else {
        return false;
    };
    world.calls.push(entity);
    let Some(mutation) = world.mutation.take() else {
        return true;
    };
    let (now, policy) = match mutation {
        ActiveMutation::RemoveFuture(future) => {
            if !world.entities.release(future, 10.0) {
                return false;
            }
            // The native freetime delay keeps the deleted future slot empty;
            // allocating here extends the live higher source-slot range.
            (10.0, AllocationPolicy::EDICT)
        }
        ActiveMutation::SpawnLower => (0.0, AllocationPolicy::EDICT),
        ActiveMutation::OverwriteLast => (0.0, AllocationPolicy::QUAKEWORLD),
    };
    let Some(allocation) = world.entities.allocate(now, module, policy) else {
        return false;
    };
    let slot = allocation.id.slot as usize;
    world.entities.columns.next_think[slot] = Some(ThinkTime::Seconds(0.5));
    world
        .entities
        .columns
        .set_think_function(slot, world.binding.think(Some(CallbackId(0))));
    world.spawned = Some(allocation.id);
    true
}

fn mutation_table(timing: RuleSetId) -> Result<FunctionTable<ActiveMutationWorld>, &'static str> {
    FunctionTable::load([(
        ModuleId(1),
        timing,
        vec![FunctionBinding {
            entry: 1,
            call: mutate_active_slots,
        }],
    )])
    .map_err(|_| "functions")
}

fn install_mutation(world: &mut ActiveMutationWorld, table: &FunctionTable<ActiveMutationWorld>) {
    world.binding = table.module_binding(ModuleId(1));
    let mut start = 0;
    while let Some(id) = world.entities.next_active(start) {
        start = id.slot as usize + 1;
        assert!(table.bind_think(&mut world.entities, id, Some(CallbackId(0))));
    }
}

fn mutation_entity(world: &mut ActiveMutationWorld) -> Result<EntityId, &'static str> {
    let id = world
        .entities
        .allocate(0.0, ModuleId(1), AllocationPolicy::EDICT)
        .ok_or("entity")?
        .id;
    world.entities.columns.next_think[id.slot as usize] = Some(ThinkTime::Seconds(0.5));
    world
        .entities
        .columns
        .set_think_function(id.slot as usize, world.binding.think(Some(CallbackId(0))));
    Ok(id)
}

#[test]
fn ascending_thinks_skip_deleted_slots_and_visit_new_higher_slots() -> Result<(), &'static str> {
    // Native Q2 g_main.c and Q3 g_main.c walk live source slots dynamically,
    // rather than snapshotting entity handles before invoking game callbacks.
    let mut world = ActiveMutationWorld {
        entities: EntityTable::new(130, 64).map_err(|_| "table")?,
        calls: Vec::new(),
        mutation: None,
        spawned: None,
        binding: ModuleBinding::default(),
    };
    let first = mutation_entity(&mut world)?;
    let removed = mutation_entity(&mut world)?;
    let kept = mutation_entity(&mut world)?;
    world.mutation = Some(ActiveMutation::RemoveFuture(removed));
    let table = mutation_table(RuleSetId::Quake2)?;
    install_mutation(&mut world, &table);
    let stats = run_thinks(&mut world, &table, |_| frame(10.0, 0.0));
    let spawned = world.spawned.ok_or("spawned")?;
    assert_eq!(spawned.slot, kept.slot + 1);
    assert_eq!((stats.called, stats.rejected), (3, 0));
    assert_eq!(world.calls, [first, kept, spawned]);
    assert!(world.entities.resolve(removed).is_none());
    assert!(world.entities.columns.next_think[spawned.slot as usize].is_none());
    Ok(())
}

#[test]
fn ascending_thinks_do_not_revisit_new_lower_slots() -> Result<(), &'static str> {
    let mut world = ActiveMutationWorld {
        entities: EntityTable::new(8, 1).map_err(|_| "table")?,
        calls: Vec::new(),
        mutation: None,
        spawned: None,
        binding: ModuleBinding::default(),
    };
    let lower = mutation_entity(&mut world)?;
    let first = mutation_entity(&mut world)?;
    let kept = mutation_entity(&mut world)?;
    assert!(world.entities.release(lower, 0.0));
    world.mutation = Some(ActiveMutation::SpawnLower);
    let table = mutation_table(RuleSetId::Quake)?;
    install_mutation(&mut world, &table);
    let stats = run_thinks(&mut world, &table, |_| frame(1.0, 0.0));
    let spawned = world.spawned.ok_or("spawned")?;
    assert_eq!(spawned.slot, lower.slot);
    assert_ne!(spawned.generation, lower.generation);
    assert_eq!((stats.called, stats.rejected), (2, 0));
    assert_eq!(world.calls, [first, kept]);
    assert_eq!(
        world.entities.columns.next_think[spawned.slot as usize],
        Some(ThinkTime::Seconds(0.5))
    );
    let next = run_thinks(&mut world, &table, |_| frame(1.0, 0.0));
    assert_eq!((next.called, next.rejected), (1, 0));
    assert_eq!(world.calls, [first, kept, spawned]);
    Ok(())
}

#[test]
fn ascending_thinks_resolve_replaced_future_generations() -> Result<(), &'static str> {
    let mut world = ActiveMutationWorld {
        entities: EntityTable::new(4, 1).map_err(|_| "table")?,
        calls: Vec::new(),
        mutation: None,
        spawned: None,
        binding: ModuleBinding::default(),
    };
    let first = mutation_entity(&mut world)?;
    let middle = mutation_entity(&mut world)?;
    let displaced = mutation_entity(&mut world)?;
    world.mutation = Some(ActiveMutation::OverwriteLast);
    let table = mutation_table(RuleSetId::QuakeWorld)?;
    install_mutation(&mut world, &table);
    let stats = run_thinks(&mut world, &table, |_| frame(1.0, 0.0));
    let replacement = world.spawned.ok_or("replacement")?;
    assert_eq!(replacement.slot, displaced.slot);
    assert_ne!(replacement.generation, displaced.generation);
    assert_eq!((stats.called, stats.rejected), (3, 0));
    assert_eq!(world.calls, [first, middle, replacement]);
    assert!(world.entities.resolve(displaced).is_none());
    Ok(())
}
