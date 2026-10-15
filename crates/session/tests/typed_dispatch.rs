use qa_core::primitives::{
    CallbackCall, CallbackId, CommandArgs, EntityId, ModuleId, RuleSetId, ThinkTime,
};
use qa_session::dispatch::{
    CallError, CallOutcome, CallbackResolver, FunctionBinding, FunctionTable, ModuleBinding,
    ThinkFrame, ThinkWorld, run_think, run_think_with_rebind, run_thinks,
};
use qa_world::entities::{AllocationPolicy, EntityTable};
use std::sync::Arc;

struct World {
    entities: EntityTable,
    starts: u32,
    completions: u32,
    entries: Vec<u32>,
    bindings: [ModuleBinding; 3],
    replacement: Option<EntityId>,
    table: Option<Arc<FunctionTable<World, Outcome>>>,
    finish: fn(&mut World, ModuleId, u32, CallbackCall<'_>) -> bool,
}

impl ThinkWorld for World {
    fn entities(&mut self) -> &mut EntityTable {
        &mut self.entities
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    Rejected,
    Started { module: ModuleId, entry: u32 },
}

impl CallOutcome<World> for Outcome {
    fn rejected() -> Self {
        Self::Rejected
    }

    fn complete(self, world: &mut World, call: CallbackCall<'_>) -> bool {
        world.completions += 1;
        match self {
            Self::Rejected => false,
            Self::Started { module, entry } => {
                world.entries.push(entry);
                (world.finish)(world, module, entry, call)
            }
        }
    }
}

fn world(capacity: usize) -> World {
    World {
        entities: EntityTable::new(capacity, 1).unwrap(),
        starts: 0,
        completions: 0,
        entries: Vec::new(),
        bindings: [ModuleBinding::default(); 3],
        replacement: None,
        table: None,
        finish: |_, _, _, _| true,
    }
}

fn entity(world: &mut World, module: ModuleId) -> EntityId {
    world
        .entities
        .allocate(ThinkTime::Seconds(1.0), module, AllocationPolicy::EDICT)
        .unwrap()
        .id
}

fn start(world: &mut World, module: ModuleId, entry: u32, call: CallbackCall<'_>) -> Outcome {
    let slot = world.entities.resolve(call.entity()).unwrap();
    assert_eq!(world.entities.columns.owner[slot], module);
    if matches!(call, CallbackCall::Think { .. }) {
        assert!(world.entities.columns.next_think[slot].is_none());
    }
    world.starts += 1;
    Outcome::Started { module, entry }
}

fn reject(world: &mut World, _: ModuleId, _: u32, _: CallbackCall<'_>) -> Outcome {
    world.starts += 1;
    Outcome::Rejected
}

fn table(rules: RuleSetId, entries: &[u32]) -> FunctionTable<World, Outcome> {
    FunctionTable::load([(
        ModuleId(1),
        rules,
        entries
            .iter()
            .map(|&entry| FunctionBinding { entry, call: start })
            .collect(),
    )])
    .unwrap()
}

fn install(world: &mut World, table: &FunctionTable<World, Outcome>) {
    world.bindings = std::array::from_fn(|module| table.module_binding(ModuleId(module as u16)));
}

fn schedule(world: &mut World, id: EntityId, callback: CallbackId, time: f64) {
    let slot = world.entities.resolve(id).unwrap();
    world.entities.columns.next_think[slot] = Some(ThinkTime::Seconds(time));
    let module = world.entities.columns.owner[slot];
    world.entities.columns.set_think_function(
        slot,
        world.bindings[usize::from(module.0)].think(Some(callback)),
    );
}

fn use_call(entity: EntityId) -> CallbackCall<'static> {
    CallbackCall::Use {
        entity,
        activator: entity,
    }
}

#[test]
fn raw_dispatch_keeps_completion_and_borrowed_root_arguments_with_the_caller() {
    fn finish(_: &mut World, module: ModuleId, entry: u32, call: CallbackCall<'_>) -> bool {
        assert_eq!((module, entry), (ModuleId(1), 70_003));
        let CallbackCall::Export {
            time,
            arguments,
            command,
            ..
        } = call
        else {
            return false;
        };
        assert_eq!(time, ThinkTime::Milliseconds(9001));
        assert_eq!(arguments[0], 0xfedc_ba98_7654_3210);
        assert_eq!(command.argv, [b"root".as_slice(), &[0xff, b'x']]);
        assert_eq!(command.raw, b"root \xffx");
        true
    }

    let mut world = world(4);
    world.finish = finish;
    let id = entity(&mut world, ModuleId(1));
    let table = table(RuleSetId::Quake3, &[91, 70_003]);
    // These bytes belong to the synchronous caller, not the typed outcome.
    let raw = [b'r', b'o', b'o', b't', b' ', 0xff, b'x'];
    let argv = [&raw[..4], &raw[5..]];
    let mut arguments = [0; 9];
    arguments[0] = 0xfedc_ba98_7654_3210;
    let call = CallbackCall::Export {
        entity: id,
        time: ThinkTime::Milliseconds(9001),
        arguments,
        client: None,
        command: CommandArgs {
            argv: &argv,
            raw: &raw,
        },
    };

    let outcome = table.dispatch(&mut world, CallbackId(1), call).unwrap();
    assert_eq!((world.starts, world.completions), (1, 0));
    assert!(world.entries.is_empty());
    assert!(outcome.complete(&mut world, call));
    assert_eq!((world.starts, world.completions), (1, 1));

    let resolved = table
        .resolve_callback(&world.entities, ModuleId(1), CallbackId(1), id)
        .unwrap();
    assert_eq!(resolved.entry(), 70_003);
    let outcome = table.dispatch_resolved(&mut world, resolved, call).unwrap();
    assert_eq!((world.starts, world.completions), (2, 1));
    assert!(outcome.complete(&mut world, call));
    assert_eq!((world.starts, world.completions), (2, 2));
    assert_eq!(world.entries, [70_003, 70_003]);

    assert_eq!(table.invoke(&mut world, CallbackId(1), call), Ok(()));
    assert_eq!((world.starts, world.completions), (3, 3));
}

#[test]
fn invoke_completes_a_typed_rejection_once_without_accepting_a_start() {
    let mut world = world(4);
    let id = entity(&mut world, ModuleId(1));
    let table = FunctionTable::load([(
        ModuleId(1),
        RuleSetId::Quake3,
        vec![FunctionBinding {
            entry: 51,
            call: reject,
        }],
    )])
    .unwrap();
    let outcome = table
        .dispatch(&mut world, CallbackId(0), use_call(id))
        .unwrap();
    assert_eq!((world.starts, world.completions), (1, 0));
    assert!(!outcome.complete(&mut world, use_call(id)));
    assert_eq!((world.starts, world.completions), (1, 1));
    assert_eq!(
        table.invoke(&mut world, CallbackId(0), use_call(id)),
        Err(CallError::Rejected)
    );
    assert_eq!((world.starts, world.completions), (2, 2));
    assert!(world.entries.is_empty());
}

#[test]
fn quakeworld_completion_can_reenter_then_reschedule_before_the_next_entity() {
    fn finish(world: &mut World, _: ModuleId, entry: u32, call: CallbackCall<'_>) -> bool {
        let id = call.entity();
        match entry {
            10 => {
                let table = Arc::clone(world.table.as_ref().unwrap());
                assert_eq!(table.invoke(world, CallbackId(1), use_call(id)), Ok(()));
            }
            20 => schedule(world, id, CallbackId(2), 0.6),
            30 => {
                let CallbackCall::Think { time, .. } = call else {
                    return false;
                };
                assert_eq!(time, ThinkTime::Seconds(f64::from(0.6f32)));
            }
            40 => {}
            _ => return false,
        }
        true
    }

    let mut world = world(8);
    world.finish = finish;
    let first = entity(&mut world, ModuleId(1));
    let second = entity(&mut world, ModuleId(2));
    let table = Arc::new(
        FunctionTable::load([
            (
                ModuleId(1),
                RuleSetId::QuakeWorld,
                [10, 20, 30]
                    .into_iter()
                    .map(|entry| FunctionBinding { entry, call: start })
                    .collect(),
            ),
            (
                ModuleId(2),
                RuleSetId::Quake,
                vec![FunctionBinding {
                    entry: 40,
                    call: start,
                }],
            ),
        ])
        .unwrap(),
    );
    install(&mut world, &table);
    world.table = Some(Arc::clone(&table));
    schedule(&mut world, first, CallbackId(0), 0.5);
    schedule(&mut world, second, CallbackId(0), 0.5);
    let stats = run_thinks(&mut world, &table, |_| {
        Some(ThinkFrame::Seconds {
            now: 0.5,
            step: 0.25,
        })
    });
    assert_eq!((stats.called, stats.rejected), (3, 0));
    assert_eq!((world.starts, world.completions), (4, 4));
    assert_eq!(world.entries, [10, 20, 30, 40]);
    assert!(world.entities.columns.next_think[first.slot as usize].is_none());
    assert!(world.entities.columns.next_think[second.slot as usize].is_none());
    assert_eq!(
        world.entities.columns.think_function(first.slot as usize),
        Some(CallbackId(2))
    );
}

#[test]
fn typed_think_completion_rebinds_only_after_a_live_replacement_is_published() {
    fn finish(world: &mut World, module: ModuleId, entry: u32, call: CallbackCall<'_>) -> bool {
        if entry == 100 {
            let old = call.entity();
            assert!(world.entities.release(old, ThinkTime::Seconds(0.0)));
            let replacement = entity(world, module);
            assert_eq!(replacement.slot, old.slot);
            assert_ne!(replacement.generation, old.generation);
            schedule(world, replacement, CallbackId(1), 0.5);
            world.replacement = Some(replacement);
        }
        true
    }

    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let mut world = world(2);
        world.finish = finish;
        let old = entity(&mut world, ModuleId(1));
        let table = table(rules, &[100, 101]);
        install(&mut world, &table);
        schedule(&mut world, old, CallbackId(0), 0.5);
        let result = run_think_with_rebind(
            &mut world,
            &table,
            old,
            |_| {
                Some(ThinkFrame::Seconds {
                    now: 0.5,
                    step: 0.25,
                })
            },
            |world, current| {
                let next = world.replacement.unwrap_or(current);
                world.entities.resolve(next).map(|_| next)
            },
        );
        let expected = if rules == RuleSetId::QuakeWorld { 2 } else { 1 };
        assert_eq!((result.called, result.rejected), (expected, 0));
        assert_eq!((world.starts, world.completions), (expected, expected));
        assert_eq!(result.current_entity, world.replacement);
        assert!(world.entities.resolve(old).is_none());
        assert_eq!(world.entries[0], 100);
        if expected == 2 {
            assert_eq!(world.entries[1], 101);
        }
    }
}

#[test]
fn resolved_dispatch_rechecks_lifetime_owner_and_the_call_entity_before_starting() {
    let mut world = world(4);
    let old = entity(&mut world, ModuleId(1));
    let other = entity(&mut world, ModuleId(1));
    let table = table(RuleSetId::Quake3, &[70_003]);
    let resolved = table.resolve(&world.entities, CallbackId(0), old).unwrap();
    assert_eq!(
        table.dispatch_resolved(&mut world, resolved, use_call(other)),
        Err(CallError::StaleEntity)
    );
    world.entities.columns.owner[old.slot as usize] = ModuleId(2);
    assert_eq!(
        table.dispatch_resolved(&mut world, resolved, use_call(old)),
        Err(CallError::MissingModule)
    );
    world.entities.columns.owner[old.slot as usize] = ModuleId(1);
    assert!(world.entities.release(old, ThinkTime::Seconds(0.0)));
    let replacement = entity(&mut world, ModuleId(1));
    assert_eq!(replacement.slot, old.slot);
    assert_ne!(replacement.generation, old.generation);
    assert_eq!(
        table.dispatch_resolved(&mut world, resolved, use_call(old)),
        Err(CallError::StaleEntity)
    );
    assert_eq!(
        table.dispatch_resolved(&mut world, resolved, use_call(replacement)),
        Err(CallError::StaleEntity)
    );
    assert_eq!((world.starts, world.completions), (0, 0));
    assert!(world.entries.is_empty());
}

#[test]
fn resolved_dispatch_rejects_another_tables_native_entry_or_internal_path() {
    let mut world = world(4);
    let id = entity(&mut world, ModuleId(1));
    let current = table(RuleSetId::Quake3, &[70_003]);
    let changed_entry = table(RuleSetId::Quake3, &[70_004]);
    let resolved = changed_entry
        .resolve(&world.entities, CallbackId(0), id)
        .unwrap();
    assert_eq!(
        current.dispatch_resolved(&mut world, resolved, use_call(id)),
        Err(CallError::MissingFunction)
    );
    let changed_path = FunctionTable::load([
        (
            ModuleId(2),
            RuleSetId::Quake3,
            vec![FunctionBinding {
                entry: 1,
                call: start,
            }],
        ),
        (
            ModuleId(1),
            RuleSetId::Quake3,
            vec![FunctionBinding {
                entry: 70_003,
                call: start,
            }],
        ),
    ])
    .unwrap();
    let resolved = changed_path
        .resolve(&world.entities, CallbackId(0), id)
        .unwrap();
    assert_eq!(
        current.dispatch_resolved(&mut world, resolved, use_call(id)),
        Err(CallError::MissingFunction)
    );
    assert_eq!(
        current.dispatch(&mut world, CallbackId(9), use_call(id)),
        Err(CallError::MissingFunction)
    );
    assert_eq!((world.starts, world.completions), (0, 0));
    assert!(world.entries.is_empty());
}

#[test]
fn due_missing_functions_complete_the_typed_rejection_after_clearing_only_the_deadline() {
    let mut world = world(4);
    let id = entity(&mut world, ModuleId(1));
    let table = table(RuleSetId::Quake, &[70_003]);
    install(&mut world, &table);
    schedule(&mut world, id, CallbackId(99), 0.75);
    let early = run_think(&mut world, &table, id, |_| {
        Some(ThinkFrame::Seconds {
            now: 0.5,
            step: 0.1,
        })
    });
    assert_eq!((early.called, early.rejected), (0, 0));
    assert_eq!((world.starts, world.completions), (0, 0));
    let due = run_think(&mut world, &table, id, |_| {
        Some(ThinkFrame::Seconds {
            now: 0.5,
            step: 0.25,
        })
    });
    assert_eq!((due.called, due.rejected), (0, 1));
    assert_eq!((world.starts, world.completions), (0, 1));
    assert!(world.entities.columns.next_think[id.slot as usize].is_none());
    assert_eq!(
        world.entities.columns.think_function(id.slot as usize),
        Some(CallbackId(99))
    );
}
