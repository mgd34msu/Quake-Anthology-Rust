use super::*;

const FIELDS: &[(u16, u16, &[u8])] = &[
    (3, 0, b"origin"),
    (3, 3, b"velocity"),
    (3, 6, b"angles"),
    (3, 9, b"mins"),
    (3, 12, b"maxs"),
    (3, 15, b"size"),
    (3, 18, b"absmin"),
    (3, 21, b"absmax"),
    (2, 24, b"solid"),
    (2, 25, b"flags"),
    (4, 26, b"owner"),
    (1, 27, b"model"),
    (2, 28, b"modelindex"),
    (2, 29, b"frame"),
    (2, 30, b"skin"),
    (2, 31, b"effects"),
    (2, 32, b"takedamage"),
    (2, 33, b"colormap"),
    (2, 34, b"nextthink"),
    (6, 35, b"think"),
    (2, 36, b"movetype"),
];
const GLOBALS: &[(u16, u16, &[u8])] = &[
    (4, 34, b"self"),
    (2, 35, b"time"),
    (4, 36, b"other"),
    (2, 37, b"frametime"),
];

struct Code {
    operations: Vec<(qa_formats::program::quakec::Opcode, i16, i16, i16)>,
    functions: Vec<i32>,
}
impl Code {
    fn new() -> Self {
        Self {
            operations: vec![],
            functions: vec![0],
        }
    }
    fn function(&mut self, body: &[(qa_formats::program::quakec::Opcode, i16, i16, i16)]) -> u32 {
        let id = self.functions.len() as u32;
        self.functions.push(self.operations.len() as i32 + 1);
        self.operations.extend_from_slice(body);
        id
    }
    fn builtin(&mut self, number: i32) -> u32 {
        let id = self.functions.len() as u32;
        self.functions.push(-number);
        id
    }
    fn vm(&self) -> Vm {
        self.vm_fields(FIELDS)
    }
    fn vm_fields(&self, fields: &[(u16, u16, &[u8])]) -> Vm {
        let bytes = quakec_program::program_with_definitions(
            &self.operations,
            &self.functions,
            GLOBALS,
            fields,
            37,
            128,
        );
        let mut vm = Vm::load(
            Image::parse(&bytes, Some(5927)).unwrap(),
            Layout {
                entities: 64,
                header_bytes: 16,
                extra_string_bytes: 128,
                state_step: 0.1,
            },
        )
        .unwrap();
        vm.image.globals[48] = 1.0f32.to_bits();
        vm.image.globals[50] = 9.0f32.to_bits();
        vm.image.globals[53] = 0; // origin field
        vm.image.globals[54] = 34; // nextthink field
        vm.image.globals[55] = 35; // think field
        vm.image.globals[56] = 29; // frame field
        vm.image.globals[57] = 36; // movetype field
        vm
    }
}
fn write(vm: &mut Vm, native: i32, field: &[u8], word: u32) {
    let value = native as u32 * vm.stride as u32;
    let address = vm
        .field_address(value, vm.image.field(field).unwrap() as u32, 1)
        .unwrap();
    vm.entities.write_word(address, word as i32).unwrap();
}
fn request(
    runtime: &mut Runtime,
    module: ModuleId,
    rules: RuleSetId,
    vm: Vm,
    entries: Vec<u32>,
    frame: u32,
    rate: u32,
    rows: &[(i32, f32, f32, u32)],
) -> (ModuleRequest, Vec<EntityId>) {
    let anchor = runtime
        .server
        .entities
        .allocate(ThinkTime::Seconds(0.0), module, AllocationPolicy::EDICT)
        .unwrap()
        .id;
    runtime.server.entities.columns.native_entity[anchor.slot as usize] =
        Some(NativeEntity { module, slot: 0 });
    let mut vm = vm;
    let mut ids = Vec::new();
    for &(native, movetype, nextthink, think) in rows {
        let entity = runtime
            .server
            .entities
            .allocate(ThinkTime::Seconds(0.0), module, AllocationPolicy::EDICT)
            .unwrap()
            .id;
        runtime.server.entities.columns.native_entity[entity.slot as usize] = Some(NativeEntity {
            module,
            slot: native,
        });
        for (field, value) in [
            (b"movetype".as_slice(), movetype.to_bits()),
            (b"nextthink", nextthink.to_bits()),
            (b"think", think),
        ] {
            if vm.image.field(field).is_some() {
                write(&mut vm, native, field, value);
            }
        }
        ids.push(entity);
    }
    let callback = entries.iter().position(|&entry| entry == frame).unwrap() as u32;
    (
        ModuleRequest {
            context: CallContext {
                native_rules: rules,
                trace_rules: RuleSetId::Quake3,
                sound_rules: RuleSetId::Quake2,
                server_frame: 0,
                module,
                clock: ThinkTime::module_clock(rules),
                console: Context {
                    source: RuleSetId::Quake3,
                    ..Context::default()
                },
                allocation: AllocationPolicy::for_rules(rules, 0),
                link_order: qa_gameplay::rules::link_order(rules),
            },
            phase: Phase::Server,
            timing_rules: rules,
            rate: TickRate::fixed(rate).unwrap(),
            anchor,
            program: Program::quakec(vm),
            entries,
            frame: Export::clocked(CallbackId(callback)),
            prepare: vec![],
            initialize: vec![],
            api: None,
            shutdown: vec![],
            instruction_budget: 1000,
            configstrings: 0,
            files: 0,
            entity_source: None,
        },
        ids,
    )
}
fn load(mut runtime: Runtime, requests: Vec<ModuleRequest>) -> FrameHost {
    let mut console = Console::new(Context::default()).unwrap();
    console
        .append("sv_maxclients 1\nsv_maxtic 1\n", Context::default())
        .unwrap();
    console.execute_frame(&mut runtime);
    FrameHost::load_modules(console, runtime, TickRate::FrameDriven, requests).unwrap()
}
fn tick(host: &mut FrameHost, milliseconds: u64) {
    host.frame(
        &mut Source {
            time: EventTime::from_milliseconds(milliseconds),
            polls: 0,
        },
        true,
    );
}
fn native_entity(host: &FrameHost, module: ModuleId, native: i32) -> EntityId {
    host.runtime
        .server
        .entities
        .active()
        .find(|entity| {
            host.runtime.server.entities.columns.native_entity[entity.slot as usize]
                == Some(NativeEntity {
                    module,
                    slot: native,
                })
        })
        .unwrap()
}

#[test]
fn state_reaches_provider_none_phase_with_native_clock_clear_and_world_context() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let mut code = Code::new();
        let frame = code.function(&[
            (StoreEnt, 64, 34, 0),
            (State, 50, 51, 0),
            (Return, 73, 0, 0),
        ]);
        let think = code.function(&[
            (LoadF, 34, 54, 73),
            (StoreF, 35, 74, 0),
            (StoreEnt, 36, 75, 0),
            (Return, 73, 0, 0),
        ]);
        let mut vm = code.vm();
        vm.image.globals[51] = think;
        vm.image.globals[64] = 40 * vm.stride as u32;
        let mut runtime = Runtime::load(1, []).unwrap();
        // Callback order deliberately differs from the native function table.
        let (request, entities) = request(
            &mut runtime,
            ModuleId(1),
            rules,
            vm,
            vec![think, 0, frame],
            frame,
            125,
            &[(40, 0.0, 0.0, 0)],
        );
        let mut host = load(runtime, vec![request]);
        tick(&mut host, 0);
        tick(&mut host, 125);
        let now = if rules == RuleSetId::Quake {
            0.0f32
        } else {
            0.125f32
        };
        let due = (f64::from(now) + 0.1) as f32;
        assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, 2);
        assert_eq!(
            host.module_counts(ModuleId(1)).unwrap().last_result,
            Some(ModuleResult::QuakeC([0, due.to_bits(), 0]))
        );
        let slot = entities[0].slot as usize;
        assert_eq!(
            host.runtime.server.entities.columns.appearance[slot].frame,
            9
        );
        assert_eq!(host.runtime.server.entities.columns.next_think[slot], None);
        assert_eq!(
            host.runtime.server.entities.columns.think_function(slot),
            Some(CallbackId(0))
        );
    }
}

#[test]
fn nq_runs_once_and_qw_repeats_reschedules_through_the_same_dispatcher() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let mut code = Code::new();
        let frame = code.function(&[
            (StoreEnt, 64, 34, 0),
            (State, 50, 51, 0),
            (Return, 73, 0, 0),
        ]);
        let first = code.function(&[(State, 50, 52, 0), (StoreF, 35, 74, 0), (Return, 73, 0, 0)]);
        let second = code.function(&[
            (LoadF, 34, 54, 73),
            (StoreF, 35, 74, 0),
            (StoreEnt, 36, 75, 0),
            (Return, 73, 0, 0),
        ]);
        let mut vm = code.vm();
        vm.image.globals[51] = first;
        vm.image.globals[52] = second;
        vm.image.globals[64] = 40 * vm.stride as u32;
        let mut runtime = Runtime::load(1, []).unwrap();
        let (request, entities) = request(
            &mut runtime,
            ModuleId(1),
            rules,
            vm,
            vec![second, frame, first, 0],
            frame,
            250,
            &[(40, 0.0, 0.0, 0)],
        );
        let mut host = load(runtime, vec![request]);
        tick(&mut host, 0);
        tick(&mut host, 250);
        let row = host.module_counts(ModuleId(1)).unwrap();
        assert_eq!(row.calls, if rules == RuleSetId::Quake { 2 } else { 3 });
        let deadline = host.runtime.server.entities.columns.next_think[entities[0].slot as usize];
        if rules == RuleSetId::Quake {
            let due = (f64::from(0.1f32) + 0.1) as f32;
            assert_eq!(deadline, Some(ThinkTime::Seconds(f64::from(due))));
        } else {
            assert_eq!(deadline, None);
            let first_due = (f64::from(0.25f32) + 0.1) as f32;
            let second_due = (f64::from(first_due) + 0.1) as f32;
            assert_eq!(
                row.last_result,
                Some(ModuleResult::QuakeC([0, second_due.to_bits(), 0]))
            );
        }
    }
}

#[test]
fn native_edict_order_is_independent_of_common_slots_and_other_physics_is_not_scanned() {
    let mut code = Code::new();
    let frame = code.function(&[(Return, 73, 0, 0)]);
    let think = code.function(&[
        (AddF, 70, 48, 70),
        (Address, 34, 53, 61),
        (StorePF, 70, 61, 0),
        (Return, 70, 0, 0),
    ]);
    let mut runtime = Runtime::load(1, []).unwrap();
    let (request, entities) = request(
        &mut runtime,
        ModuleId(1),
        RuleSetId::Quake,
        code.vm(),
        vec![think, frame, 0],
        frame,
        125,
        &[
            (41, 0.0, 0.05, think),
            (40, 0.0, 0.05, think),
            (42, 4.0, 0.05, think), // STEP: think belongs after movement.
            (43, 6.0, 0.05, think), // TOSS: think precedes its own movement.
            (44, 7.0, 0.05, think), // PUSH: ltime controls the due phase.
        ],
    );
    let mut host = load(runtime, vec![request]);
    tick(&mut host, 0);
    tick(&mut host, 125);
    assert_eq!(
        host.runtime.server.entities.columns.position[entities[1].slot as usize].0[0],
        1.0
    );
    assert_eq!(
        host.runtime.server.entities.columns.position[entities[0].slot as usize].0[0],
        2.0
    );
    for entity in &entities[2..] {
        assert_eq!(
            host.runtime.server.entities.columns.position[entity.slot as usize].0[0],
            0.0
        );
        assert_eq!(
            host.runtime.server.entities.columns.next_think[entity.slot as usize],
            Some(ThinkTime::Seconds(f64::from(0.05f32)))
        );
    }
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, 3);
    assert_eq!(
        host.module_counts(ModuleId(1)).unwrap().unsupported_physics,
        3
    );
}

#[test]
fn later_spawn_runs_but_earlier_reused_native_ordinal_waits_for_next_pass() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let mut code = Code::new();
        let frame = code.function(&[
            (If, 64, 6, 0),
            (Call0, 58, 0, 0),
            (StoreEnt, 1, 64, 0),
            (StoreEnt, 1, 34, 0),
            (State, 50, 51, 0),
            (StoreEnt, 49, 34, 0),
            (Return, 70, 0, 0),
        ]);
        let first = code.function(&[
            (AddF, 70, 48, 70),
            (Call0, 58, 0, 0),
            (StoreEnt, 1, 34, 0),
            (State, 50, 52, 0),
            (Return, 70, 0, 0),
        ]);
        let second = code.function(&[
            (AddF, 70, 48, 70),
            (StoreEnt, 64, 4, 0),
            (Call1, 59, 0, 0),
            (Call0, 58, 0, 0),
            (StoreEnt, 1, 34, 0),
            (State, 50, 60, 0),
            (Return, 70, 0, 0),
        ]);
        let reused = code.function(&[(AddF, 70, 48, 70), (Return, 70, 0, 0)]);
        let spawn = code.builtin(14);
        let remove = code.builtin(15);
        let mut vm = code.vm();
        for (global, value) in [
            (51, first),
            (52, second),
            (60, reused),
            (58, spawn),
            (59, remove),
        ] {
            vm.image.globals[global] = value;
        }
        let entries = (0..code.functions.len() as u32).rev().collect();
        let mut runtime = Runtime::load(1, []).unwrap();
        let (request, _) = request(
            &mut runtime,
            ModuleId(1),
            rules,
            vm,
            entries,
            frame,
            500,
            &[],
        );
        let mut host = load(runtime, vec![request]);
        tick(&mut host, 0);
        tick(&mut host, 500);
        let ordinal = if rules == RuleSetId::Quake { 2 } else { 33 };
        let earlier = native_entity(&host, ModuleId(1), ordinal);
        assert!(earlier.generation > 1);
        assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, 3);
        assert_eq!(
            host.module_counts(ModuleId(1)).unwrap().last_result,
            Some(ModuleResult::QuakeC([2.0f32.to_bits(), 0, 0]))
        );
        assert!(host.runtime.server.entities.columns.next_think[earlier.slot as usize].is_some());
        tick(&mut host, 1000);
        assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, 5);
        assert_eq!(
            host.module_counts(ModuleId(1)).unwrap().last_result,
            Some(ModuleResult::QuakeC([3.0f32.to_bits(), 0, 0]))
        );
    }
}

#[test]
fn independent_provider_rates_supply_native_start_end_time_and_qw_frametime() {
    let mut code = Code::new();
    let frame = code.function(&[
        (StoreF, 35, 73, 0),
        (StoreF, 37, 74, 0),
        (StoreEnt, 36, 75, 0),
        (Return, 73, 0, 0),
    ]);
    let mut runtime = Runtime::load(1, []).unwrap();
    let (nq, _) = request(
        &mut runtime,
        ModuleId(1),
        RuleSetId::Quake,
        code.vm(),
        vec![frame, 0],
        frame,
        100,
        &[],
    );
    let (qw, _) = request(
        &mut runtime,
        ModuleId(2),
        RuleSetId::QuakeWorld,
        code.vm(),
        vec![frame, 0],
        frame,
        50,
        &[],
    );
    let mut host = load(runtime, vec![nq, qw]);
    tick(&mut host, 1000);
    tick(&mut host, 1250);
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, 2);
    assert_eq!(host.module_counts(ModuleId(2)).unwrap().calls, 5);
    assert_eq!(
        host.module_counts(ModuleId(1)).unwrap().last_result,
        Some(ModuleResult::QuakeC([1.1f32.to_bits(), 0, 0]))
    );
    assert_eq!(
        host.module_counts(ModuleId(2)).unwrap().last_result,
        Some(ModuleResult::QuakeC([
            1.25f32.to_bits(),
            0.05f32.to_bits(),
            0
        ]))
    );
}

#[test]
fn full_width_invalid_native_function_rejects_only_its_due_entity() {
    let mut code = Code::new();
    let frame = code.function(&[(Return, 70, 0, 0)]);
    let think = code.function(&[(AddF, 70, 48, 70), (Return, 70, 0, 0)]);
    let mut runtime = Runtime::load(1, []).unwrap();
    let (request, ids) = request(
        &mut runtime,
        ModuleId(1),
        RuleSetId::Quake,
        code.vm(),
        vec![frame, think, 0],
        frame,
        125,
        &[(40, 0.0, 0.05, 0x0100_0001), (41, 0.0, 0.05, think)],
    );
    let mut host = load(runtime, vec![request]);
    tick(&mut host, 0);
    tick(&mut host, 125);
    assert_eq!(
        host.runtime
            .server
            .entities
            .columns
            .think_function(ids[0].slot as usize),
        Some(CallbackId(0x0100_0001))
    );
    assert_eq!(
        host.runtime.server.entities.columns.next_think[ids[0].slot as usize],
        None
    );
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().rejected, 1);
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, 3);
    assert_eq!(
        host.module_counts(ModuleId(1)).unwrap().last_result,
        Some(ModuleResult::QuakeC([1.0f32.to_bits(), 0, 0]))
    );
    tick(&mut host, 250);
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().rejected, 1);
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 1);
    assert_eq!(
        host.runtime.server.entities.columns.next_think[ids[0].slot as usize],
        Some(ThinkTime::Seconds(0.0))
    );
}

#[test]
fn valid_native_function_above_u16_binds_to_its_actual_session_callback() {
    let mut code = Code::new();
    let frame = code.function(&[(Return, 70, 0, 0)]);
    code.functions.resize(65_536, 0);
    let think = code.function(&[(AddF, 70, 48, 70), (Return, 70, 0, 0)]);
    assert_eq!(think, 65_536);
    let mut runtime = Runtime::load(1, []).unwrap();
    let (request, ids) = request(
        &mut runtime,
        ModuleId(1),
        RuleSetId::Quake,
        code.vm(),
        vec![frame, think],
        frame,
        125,
        &[(40, 0.0, 0.05, think)],
    );
    let mut host = load(runtime, vec![request]);
    tick(&mut host, 0);
    tick(&mut host, 125);
    assert_eq!(
        host.runtime
            .server
            .entities
            .columns
            .think_function(ids[0].slot as usize),
        Some(CallbackId(1))
    );
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, 2);
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().rejected, 0);
    assert_eq!(
        host.module_counts(ModuleId(1)).unwrap().last_result,
        Some(ModuleResult::QuakeC([1.0f32.to_bits(), 0, 0]))
    );
}

#[test]
fn state_with_missing_typed_think_field_traps_only_when_executed() {
    let mut code = Code::new();
    let frame = code.function(&[(Return, 70, 0, 0)]);
    let state = code.function(&[(State, 50, 51, 0), (Return, 70, 0, 0)]);
    let fields = FIELDS
        .iter()
        .copied()
        .filter(|(_, _, name)| *name != b"think")
        .collect::<Vec<_>>();
    let mut runtime = Runtime::load(1, []).unwrap();
    let (request, _) = request(
        &mut runtime,
        ModuleId(1),
        RuleSetId::Quake,
        code.vm_fields(&fields),
        vec![frame, state, 0],
        frame,
        125,
        &[],
    );
    let mut host = load(runtime, vec![request]);
    tick(&mut host, 0);
    tick(&mut host, 125);
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, 1);
    assert_eq!(
        host.call_module_export(
            ModuleId(1),
            Export::clocked(CallbackId(1)),
            ThinkTime::Seconds(0.125)
        ),
        Err(CallError::Rejected)
    );
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 1);
}

#[test]
fn absent_think_dormant_and_future_deadlines_do_not_execute_a_rejection() {
    let mut code = Code::new();
    let frame = code.function(&[(Return, 70, 0, 0)]);
    let fields = FIELDS
        .iter()
        .copied()
        .filter(|(_, _, name)| *name != b"think")
        .collect::<Vec<_>>();
    let mut runtime = Runtime::load(1, []).unwrap();
    let (request, ids) = request(
        &mut runtime,
        ModuleId(1),
        RuleSetId::Quake,
        code.vm_fields(&fields),
        vec![frame, 0],
        frame,
        125,
        &[(40, 0.0, 0.0, 0), (41, 0.0, 0.5, 0)],
    );
    let mut host = load(runtime, vec![request]);
    tick(&mut host, 0);
    tick(&mut host, 125);
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().rejected, 0);
    tick(&mut host, 500);
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().rejected, 1);
    tick(&mut host, 625);
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().rejected, 1);
    // The next export imports native zero after the rejected scheduled call.
    assert_eq!(
        host.runtime.server.entities.columns.next_think[ids[1].slot as usize],
        Some(ThinkTime::Seconds(0.0))
    );
}

#[test]
fn future_callback_time_does_not_change_native_reuse_grace_or_strict_delay() {
    for (provider_now, due) in [(1.99f64, 2.01f32), (3.0, 3.01)] {
        let mut code = Code::new();
        let frame = code.function(&[
            (Call0, 58, 0, 0),
            (StoreEnt, 1, 34, 0),
            (Address, 34, 54, 61),
            (StorePF, 66, 61, 0),
            (Address, 34, 55, 61),
            (StorePF, 51, 61, 0),
            (Return, 73, 0, 0),
        ]);
        let think = code.function(&[
            (StoreEnt, 34, 4, 0),
            (Call1, 59, 0, 0),
            (Call0, 58, 0, 0),
            (StoreEnt, 1, 73, 0),
            (StoreF, 35, 74, 0),
            (Return, 73, 0, 0),
        ]);
        let spawn_export = code.function(&[(Call0, 58, 0, 0), (Return, 1, 0, 0)]);
        let spawn = code.builtin(14);
        let remove = code.builtin(15);
        let mut vm = code.vm();
        let stride = vm.stride as u32;
        for (global, value) in [(51, think), (58, spawn), (59, remove), (66, due.to_bits())] {
            vm.image.globals[global] = value;
        }
        let mut runtime = Runtime::load(1, []).unwrap();
        let (request, _) = request(
            &mut runtime,
            ModuleId(1),
            RuleSetId::Quake,
            vm,
            (0..code.functions.len() as u32).collect(),
            frame,
            40,
            &[],
        );
        let mut host = load(runtime, vec![request]);
        let start_ms = (provider_now * 1000.0) as u64;
        tick(&mut host, start_ms);
        tick(&mut host, start_ms + 40);
        let expected_ordinal = if provider_now < 2.0 { 2 } else { 3 };
        assert_eq!(
            host.module_counts(ModuleId(1)).unwrap().last_result,
            Some(ModuleResult::QuakeC([
                expected_ordinal * stride,
                due.to_bits(),
                0
            ]))
        );
        if provider_now > 2.0 {
            host.call_module_export(
                ModuleId(1),
                Export::clocked(CallbackId(spawn_export)),
                ThinkTime::Seconds(3.5),
            )
            .unwrap();
            assert_eq!(
                host.module_counts(ModuleId(1)).unwrap().last_result,
                Some(ModuleResult::QuakeC([4 * stride, due.to_bits(), 0]))
            );
            host.call_module_export(
                ModuleId(1),
                Export::clocked(CallbackId(spawn_export)),
                ThinkTime::Seconds(3.500_001),
            )
            .unwrap();
            assert_eq!(
                host.module_counts(ModuleId(1)).unwrap().last_result,
                Some(ModuleResult::QuakeC([2 * stride, due.to_bits(), 0]))
            );
        }
    }
}

#[test]
fn omitted_native_entry_clears_only_its_due_deadline_before_rejection() {
    let mut code = Code::new();
    let frame = code.function(&[(Return, 70, 0, 0)]);
    let omitted = code.function(&[(AddF, 70, 48, 70), (Return, 70, 0, 0)]);
    let mut runtime = Runtime::load(1, []).unwrap();
    let (request, ids) = request(
        &mut runtime,
        ModuleId(1),
        RuleSetId::Quake,
        code.vm(),
        vec![frame],
        frame,
        125,
        &[(40, 0.0, 0.05, omitted)],
    );
    let mut host = load(runtime, vec![request]);
    tick(&mut host, 0);
    tick(&mut host, 125);
    assert_eq!(
        host.runtime
            .server
            .entities
            .columns
            .think_function(ids[0].slot as usize),
        Some(CallbackId(omitted))
    );
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().rejected, 1);
    tick(&mut host, 250);
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().rejected, 1);
    assert_eq!(
        host.runtime.server.entities.columns.next_think[ids[0].slot as usize],
        Some(ThinkTime::Seconds(0.0))
    );
}

#[test]
fn native_qw_minimum_accumulates_and_maximum_clamps_physics_without_stopping_clock() {
    let mut code = Code::new();
    let frame = code.function(&[(StoreF, 35, 73, 0), (StoreF, 37, 74, 0), (Return, 73, 0, 0)]);
    let mut runtime = Runtime::load(1, []).unwrap();
    let (mut request, _) = request(
        &mut runtime,
        ModuleId(1),
        RuleSetId::QuakeWorld,
        code.vm(),
        vec![frame, 0],
        frame,
        1,
        &[],
    );
    request.rate = TickRate::FrameDriven;
    let mut host = load(runtime, vec![request]);
    host.console
        .append("sv_maxtic 0.1\n", Context::default())
        .unwrap();
    host.console.execute_frame(&mut host.runtime);
    tick(&mut host, 0);
    tick(&mut host, 17);
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, 0);
    tick(&mut host, 34);
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, 1);
    assert_eq!(
        host.module_counts(ModuleId(1)).unwrap().last_result,
        Some(ModuleResult::QuakeC([
            0.034f32.to_bits(),
            0.034f32.to_bits(),
            0
        ]))
    );
    tick(&mut host, 284);
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, 2);
    assert_eq!(
        host.module_counts(ModuleId(1)).unwrap().last_result,
        Some(ModuleResult::QuakeC([
            0.284f32.to_bits(),
            0.1f32.to_bits(),
            0
        ]))
    );
}
