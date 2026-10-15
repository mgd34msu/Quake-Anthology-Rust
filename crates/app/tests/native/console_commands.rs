use super::*;

const ENTRY: usize = 0x1600;
const SEGMENT_BIAS: usize = 0x2000;
const SYSCALL: usize = 0x1ac0;
const TEXT: usize = 0x1b00;
const NAME: usize = 0x1b50;
const NESTED: usize = 0x1b80;
const ARGUMENT: usize = 0x1be0;

fn rip(code: &mut Vec<u8>, operation: &[u8], target: usize) {
    code.extend_from_slice(operation);
    code.extend((target as i32 - (ENTRY + code.len() + 4) as i32).to_le_bytes());
}
fn number(code: &mut Vec<u8>, value: u32) {
    code.push(0xbf); // mov edi,number
    code.extend(value.to_le_bytes());
}
fn syscall(code: &mut Vec<u8>) {
    code.extend([0x31, 0xc0]); // native varargs has no vector arguments
    rip(code, &[0xff, 0x15], SYSCALL);
}
fn now(code: &mut Vec<u8>, target: usize) {
    number(code, 14);
    code.extend([0x31, 0xf6]);
    rip(code, &[0x48, 0x8d, 0x15], target);
    syscall(code);
}
fn finish(code: &mut Vec<u8>) {
    code.extend([0x48, 0x83, 0xc4, 8, 0xc3]);
}
fn fixture(nested: bool) -> FrameHost {
    let mut code = vec![0x48, 0x83, 0xec, 8];
    code.extend([0x83, 0xff, 9, 0x0f, 0x84]); // cmp edi,9; je callback
    let callback_jump = code.len();
    code.extend([0; 4]);
    now(&mut code, TEXT);
    number(&mut code, 6);
    rip(&mut code, &[0x48, 0x8d, 0x35], NAME);
    syscall(&mut code);
    finish(&mut code);
    let callback = code.len();
    code[callback_jump..callback_jump + 4]
        .copy_from_slice(&(callback as i32 - callback_jump as i32 - 4).to_le_bytes());
    // Read raw Other argv through the same common copy ABI used by stock Q3.
    number(&mut code, 9);
    code.extend([0xbe, 1, 0, 0, 0]);
    rip(&mut code, &[0x48, 0x8d, 0x15], ARGUMENT);
    code.extend([0xb9, 16, 0, 0, 0]);
    syscall(&mut code);
    rip(&mut code, &[0x80, 0x3d], ARGUMENT);
    // cmp byte [rip+displacement],0xff has an additional immediate byte.
    let displacement = code.len() - 4;
    let relative = ARGUMENT as i32 - (ENTRY + code.len() + 1) as i32;
    code[displacement..displacement + 4].copy_from_slice(&relative.to_le_bytes());
    code.extend([0xff, 0x0f, 0x85]);
    let bad_byte = code.len();
    code.extend([0; 4]);
    now(&mut code, NESTED);
    number(&mut code, 8);
    syscall(&mut code);
    code.extend([0x83, 0xf8, 2, 0x0f, 0x85]); // outer callback argv survives inner EXEC_NOW
    let bad_count = code.len();
    code.extend([0; 4]);
    code.extend([0xb8, 1, 0, 0, 0]);
    finish(&mut code);
    let failed = code.len();
    code.extend([0x31, 0xc0]);
    finish(&mut code);
    for jump in [bad_byte, bad_count] {
        code[jump..jump + 4].copy_from_slice(&(failed as i32 - jump as i32 - 4).to_le_bytes());
    }
    assert!(code.len() <= 0x140);
    // Keep the builder's 48-byte symbol entry clear of its adjacent symbol table.
    let mut entry = vec![0xe9];
    entry.extend((ENTRY as i32 - 0x13c0 - 5).to_le_bytes());
    let mut image = function_image(Encoding::Elf, &entry);
    image.bytes[ENTRY + SEGMENT_BIAS..ENTRY + SEGMENT_BIAS + code.len()].copy_from_slice(&code);
    let exports = [
        NamedExport {
            parameters: &[NativeScalar::Word; 13],
            result: NativeScalar::Word,
            name: b"vmMain",
            command: Some(10),
        },
        NamedExport {
            parameters: &[NativeScalar::Word],
            result: NativeScalar::Void,
            name: b"dllEntry",
            command: None,
        },
        NamedExport {
            parameters: &[NativeScalar::Word; 13],
            result: NativeScalar::Word,
            name: b"vmMain",
            command: Some(9),
        },
    ];
    let mut vm = Vm::map_image(image, &exports, &[], Duration::from_secs(3)).unwrap();
    assert_eq!(vm.export_for_command(9), Some(2));
    assert!(!vm.declares_export(9));
    let pointer = vm.import_callback();
    let memory = vm.process.memory_mut().unwrap();
    for (at, bytes) in [
        (
            TEXT,
            if nested {
                b"native_callback \xff\0".as_slice()
            } else {
                b"set _qa_native_now 17\0".as_slice()
            },
        ),
        (
            NAME,
            if nested {
                b"_qa_native_nested\0".as_slice()
            } else {
                b"_qa_native_now\0".as_slice()
            },
        ),
        (NESTED, b"set _qa_native_nested 23\0".as_slice()),
    ] {
        memory[at + SEGMENT_BIAS..at + SEGMENT_BIAS + bytes.len()].copy_from_slice(bytes);
    }
    let mut runtime = Runtime::load(1, []).unwrap();
    let mut request = super::request(
        &mut runtime,
        ModuleId(1),
        RuleSetId::Quake3,
        TickRate::FrameDriven,
        vm,
    );
    request.entries = vec![0, 1];
    request.prepare = vec![Export::words(CallbackId(1), &[pointer])];
    let mut host = FrameHost::load_modules(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        vec![request],
    )
    .unwrap();
    host.initialize_modules(Phase::Server);
    assert_eq!(host.module_state(ModuleId(1)), Some(State::Running));
    host
}

pub(super) fn run() {
    suspended_producer_is_live_after_its_nested_export_returns();
    for nested in [false, true] {
        let mut host = fixture(nested);
        host.console
            .append("set _qa_native_queued 5\n", Context::default())
            .unwrap();
        host.call_module_export(
            ModuleId(1),
            Export::words(CallbackId(0), &[]),
            ThinkTime::Milliseconds(0),
        )
        .unwrap();
        assert_eq!(
            host.module_counts(ModuleId(1)).unwrap().last_result,
            Some(ModuleResult::Native(if nested { 23 } else { 17 }))
        );
        assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 0);
        assert!(host.console.cvars.find("_qa_native_queued").is_none());
        host.execute_commands();
        assert_eq!(
            host.console
                .cvars
                .value(host.console.cvars.find("_qa_native_queued").unwrap()),
            5.0
        );
        host.shutdown_modules();
    }
}

fn suspended_producer_is_live_after_its_nested_export_returns() {
    use qa_core::primitives::{Bounds, NativeEntity, Q3StatLayout, Vec3};
    use qa_world::collision::{brushes::BrushTree, surfaces::SurfaceTable};

    const ENTITY: usize = 0x1800;
    const PLAYERS: usize = 0x1d00;
    const BODY: u32 = 0x0200_0000;
    fn leave(code: &mut Vec<u8>, value: Option<u32>) {
        if let Some(value) = value {
            code.push(0xb8);
            code.extend(value.to_le_bytes());
        }
        code.extend([0x48, 0x83, 0xc4, 24, 0xc3]);
    }
    fn store(code: &mut Vec<u8>, target: usize, value: u32) {
        rip(code, &[0xc7, 0x05], target);
        // The memory operation also consumes its four-byte immediate.
        let at = code.len() - 4;
        let relative = i32::from_le_bytes(code[at..at + 4].try_into().unwrap()) - 4;
        code[at..at + 4].copy_from_slice(&relative.to_le_bytes());
        code.extend(value.to_le_bytes());
    }
    fn jump(code: &mut Vec<u8>, command: u8) -> usize {
        code.extend([0x83, 0xff, command, 0x0f, 0x84]);
        let at = code.len();
        code.extend([0; 4]);
        at
    }
    fn bind(code: &[u8], commands: &[u32]) -> Vm {
        assert!(code.len() < ENTITY - ENTRY);
        let mut entry = vec![0xe9];
        entry.extend((ENTRY as i32 - 0x13c0 - 5).to_le_bytes());
        let mut image = function_image(Encoding::Elf, &entry);
        image.bytes[ENTRY + SEGMENT_BIAS..ENTRY + SEGMENT_BIAS + code.len()].copy_from_slice(code);
        let mut exports: Vec<_> = commands
            .iter()
            .map(|&command| NamedExport {
                parameters: &[NativeScalar::Word; 13],
                result: NativeScalar::Word,
                name: b"vmMain",
                command: Some(command),
            })
            .collect();
        exports.insert(
            1,
            NamedExport {
                parameters: &[NativeScalar::Word],
                result: NativeScalar::Void,
                name: b"dllEntry",
                command: None,
            },
        );
        Vm::map_image(image, &exports, &[], Duration::from_secs(3)).unwrap()
    }
    let mut producer = vec![0x48, 0x83, 0xec, 24];
    let initialize = jump(&mut producer, 0);
    let nested = jump(&mut producer, 9);
    let inspect = jump(&mut producer, 11);
    store(&mut producer, ENTITY + 488, 64.0f32.to_bits());
    store(&mut producer, ENTITY + 460, BODY);
    now(&mut producer, TEXT);
    leave(&mut producer, Some(7));
    let init_at = producer.len();
    number(&mut producer, 15);
    rip(&mut producer, &[0x48, 0x8d, 0x35], ENTITY);
    producer.extend([0xba, 1, 0, 0, 0, 0xb9]);
    producer.extend(608u32.to_le_bytes());
    rip(&mut producer, &[0x4c, 0x8d, 0x05], PLAYERS);
    producer.extend([0x41, 0xb9]);
    producer.extend(640u32.to_le_bytes());
    syscall(&mut producer);
    number(&mut producer, 30);
    rip(&mut producer, &[0x48, 0x8d, 0x35], ENTITY);
    syscall(&mut producer);
    leave(&mut producer, Some(0));
    let nested_at = producer.len();
    store(&mut producer, ENTITY + 488, 96.0f32.to_bits());
    leave(&mut producer, Some(0));
    let inspect_at = producer.len();
    rip(&mut producer, &[0x8b, 0x05], ENTITY + 420);
    leave(&mut producer, None);
    for (jump, target) in [
        (initialize, init_at),
        (nested, nested_at),
        (inspect, inspect_at),
    ] {
        producer[jump..jump + 4].copy_from_slice(&(target as i32 - jump as i32 - 4).to_le_bytes());
    }
    let mut producer = bind(&producer, &[10, 0, 11, 9]);
    producer.configure_q3_game(Q3StatLayout::BASE);
    {
        let base = producer.process.base();
        let mut memory = qa_compat::memory::process_memory(&mut producer.process).unwrap();
        let entity = base + (ENTITY + SEGMENT_BIAS) as u64;
        memory.read_mut(entity, 608).unwrap().fill(0);
        memory.write_word(entity + 512, 1023).unwrap();
        memory.write_word(entity + 460, 1).unwrap();
        memory.write_vec3(entity + 436, Vec3([-1.0; 3])).unwrap();
        memory.write_vec3(entity + 448, Vec3([1.0; 3])).unwrap();
        let text = base + (TEXT + SEGMENT_BIAS) as u64;
        memory.write_string(text, 32, b"foreign_trace").unwrap();
    }
    let mut consumer = vec![0x48, 0x83, 0xec, 24];
    number(&mut consumer, 24);
    rip(&mut consumer, &[0x48, 0x8d, 0x35], PLAYERS);
    rip(&mut consumer, &[0x48, 0x8d, 0x15], TEXT);
    consumer.extend([0x31, 0xc9, 0x45, 0x31, 0xc0]);
    rip(&mut consumer, &[0x4c, 0x8d, 0x0d], TEXT + 16);
    consumer.extend([0x48, 0xc7, 0x04, 0x24]);
    consumer.extend(1023u32.to_le_bytes());
    consumer.extend([0x48, 0xc7, 0x44, 0x24, 8]);
    consumer.extend(BODY.to_le_bytes());
    syscall(&mut consumer);
    rip(&mut consumer, &[0x8b, 0x05], PLAYERS + 8);
    leave(&mut consumer, None);
    let mut consumer = bind(&consumer, &[9]);
    {
        let base = consumer.process.base();
        let mut memory = qa_compat::memory::process_memory(&mut consumer.process).unwrap();
        memory
            .write_vec3(base + (TEXT + SEGMENT_BIAS) as u64, Vec3([-16.0, 0.0, 0.0]))
            .unwrap();
        memory
            .write_vec3(
                base + (TEXT + SEGMENT_BIAS + 16) as u64,
                Vec3([128.0, 0.0, 0.0]),
            )
            .unwrap();
    }
    let mut runtime = Runtime::load(1, []).unwrap();
    let geometry = runtime
        .geometry
        .load_brushes(
            vec![],
            vec![],
            Vec::new(),
            SurfaceTable::flags(vec![]),
            BrushTree::direct(0).unwrap(),
            vec![Bounds {
                mins: Vec3([-4096.0; 3]),
                maxs: Vec3([4096.0; 3]),
            }],
        )
        .unwrap();
    let collision = visibility_world::collision(
        &runtime.geometry,
        [Some(0), Some(0)],
        qa_world::visibility::PvsRows::all_visible(1),
    );
    assert_eq!(collision.geometry, geometry);
    runtime.collision = Some(collision);
    let producer_pointer = producer.import_callback();
    let consumer_pointer = consumer.import_callback();
    let mut first = request(
        &mut runtime,
        ModuleId(1),
        RuleSetId::Quake3,
        TickRate::FrameDriven,
        producer,
    );
    first.entries = vec![0, 1, 2, 3];
    first.prepare = vec![Export::words(CallbackId(1), &[producer_pointer])];
    first.initialize = vec![Export::words(CallbackId(2), &[])];
    let mut second = request(
        &mut runtime,
        ModuleId(2),
        RuleSetId::Quake3,
        TickRate::FrameDriven,
        consumer,
    );
    second.entries = vec![0, 1];
    second.prepare = vec![Export::words(CallbackId(1), &[consumer_pointer])];
    let mut console = Console::new(Context::default()).unwrap();
    let max_clients = console
        .cvars
        .bind(
            "sv_maxclients",
            Context {
                source: RuleSetId::Quake3,
                ..Context::default()
            },
        )
        .unwrap();
    console.cvars.force_write(max_clients, "1").unwrap();
    let mut host =
        FrameHost::load_modules(console, runtime, TickRate::FrameDriven, vec![first, second])
            .unwrap();
    host.initialize_modules(Phase::Server);
    if host.module_state(ModuleId(1)) != Some(State::Running) {
        let mut batch = host
            .runtime
            .server
            .events
            .batch(host.runtime.server.presentation)
            .unwrap();
        while let Some(record) = host.runtime.server.events.next(&mut batch) {
            if let FrameEvent::Print(print) = record.event {
                eprintln!(
                    "{}",
                    String::from_utf8_lossy(
                        host.runtime.server.events.texts.get(print.text).unwrap()
                    )
                );
            }
        }
    }
    assert_eq!(host.module_state(ModuleId(1)), Some(State::Running));
    assert_eq!(host.module_state(ModuleId(2)), Some(State::Running));
    let entity = host
        .runtime
        .server
        .entities
        .find_native(
            NativeEntity {
                module: ModuleId(1),
                slot: 0,
            },
            None,
        )
        .unwrap();
    let before = host.runtime.server.area.bounds(entity).unwrap();
    let relinks = host.runtime.server.area.relinks;
    host.call_module_export(
        ModuleId(1),
        Export::words(CallbackId(0), &[]),
        ThinkTime::Milliseconds(0),
    )
    .unwrap();
    assert_eq!(
        host.module_counts(ModuleId(1)).unwrap().last_result,
        Some(ModuleResult::Native(7))
    );
    let Some(ModuleResult::Native(bits)) = host.module_counts(ModuleId(2)).unwrap().last_result
    else {
        panic!("trace reply")
    };
    let fraction = f32::from_bits(bits as u32);
    assert!(
        fraction > 0.75 && fraction < 0.8,
        "live foreign collision fraction {fraction}"
    );
    assert_eq!(
        host.runtime.server.entities.columns.position[entity.slot as usize],
        Vec3([96.0, 0.0, 0.0])
    );
    assert_eq!(host.runtime.server.area.bounds(entity), Some(before));
    assert_eq!(host.runtime.server.area.relinks, relinks);
    host.call_module_export(
        ModuleId(1),
        Export::words(CallbackId(3), &[]),
        ThinkTime::Milliseconds(0),
    )
    .unwrap();
    assert_eq!(
        host.module_counts(ModuleId(1)).unwrap().last_result,
        Some(ModuleResult::Native(1))
    );
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 0);
    assert_eq!(host.module_counts(ModuleId(2)).unwrap().traps, 0);
    host.shutdown_modules();
}
