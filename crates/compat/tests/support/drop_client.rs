use super::*;
use qa_core::primitives::{CallbackId, EntityId};
use qa_session::dispatch::{FunctionBinding, FunctionTable};

const REASON: u64 = 256;
const COUNT: u64 = 128;
const OBSERVED: u64 = 132;
const TEXT: u64 = 512;
const FAIL: u64 = 136;
const LIVE: i32 = 777.0f32.to_bits() as i32;

fn callbacks(disconnect: u32) -> FunctionTable<Server> {
    FunctionTable::load([(
        MODULE,
        RuleSetId::Quake3,
        (0..6)
            .map(|ordinal| FunctionBinding {
                entry: if ordinal == 5 { disconnect } else { ordinal },
                call: |_, _, _, _| panic!("active adapter must consume the resolved callback"),
            })
            .collect(),
    )])
    .unwrap()
}
fn anchor(fixture: &mut Fixture) -> EntityId {
    let entity = fixture
        .server
        .entities
        .allocate(context().clock, MODULE, AllocationPolicy::EDICT)
        .unwrap()
        .id;
    fixture.storage.bind_lifetime(MODULE, entity).unwrap();
    fixture
        .server
        .replace_userinfo_raw(ClientId(1), b"\\name\\Target", 1024)
        .unwrap();
    entity
}
fn observed_commands(f: &Fixture, id: ClientId) -> Vec<Vec<u8>> {
    let output = f.server.clients[id.0 as usize].output.unwrap();
    let mut batch = f.server.events.batch(output).unwrap();
    let mut commands = Vec::new();
    while let Some(record) = f.server.events.next(&mut batch) {
        if let qa_core::events::FrameEvent::ServerCommand { text, .. } = record.event {
            commands.push(f.server.events.texts.get(text).unwrap().to_vec());
        }
    }
    commands
}
fn assert_close(f: &Fixture) {
    let dropped = &f.server.clients[1];
    assert!(matches!(
        dropped.phase,
        qa_session::clients::ClientPhase::Closing(_)
    ));
    assert!(dropped.connection.is_some());
    assert_eq!(f.server.userinfo(ClientId(1)), Some(b"".as_slice()));
    assert!(f.server.clients[0].is_active());
    let target = observed_commands(f, ClientId(1));
    assert_eq!(
        target,
        [
            b"print \"Target^7 before\n\"".to_vec(),
            b"disconnect \"after\"".to_vec()
        ]
    );
    let healthy = observed_commands(f, ClientId(0));
    assert!(healthy.contains(&b"print \"Target^7 before\n\"".to_vec()));
    assert_eq!(
        healthy
            .iter()
            .filter(|text| text.as_slice() == b"print \"nested\"")
            .count(),
        2
    );
    assert!(!healthy.iter().any(|text| text.starts_with(b"disconnect")));
}

fn qvm_program() -> Vec<(Opcode, i32)> {
    use Opcode::*;
    let mut code = vec![
        (Enter, 64),
        (Local, 72),
        (Load4, 0),
        (Const, 5),
        (Eq, 0),
        (Const, (ENTITIES + 488) as i32),
        (Const, LIVE),
        (Store4, 0),
        (Const, 0),
        (Arg, 8),
        (Const, REASON as i32),
        (Arg, 12),
        (Const, -17),
        (Call, 0),
        (Pop, 0),
        (Const, COUNT as i32),
        (Load4, 0),
        (Leave, 64),
    ];
    code[4].1 = code.len() as i32;
    code.extend([
        (Const, OBSERVED as i32),
        (Const, (ENTITIES + 488) as i32),
        (Load4, 0),
        (Store4, 0),
        (Const, COUNT as i32),
        (Const, COUNT as i32),
        (Load4, 0),
        (Const, 1),
        (Add, 0),
        (Store4, 0),
        // Recursion on the same connected or FREE ordinal must skip its bad pointer.
        (Local, 76),
        (Load4, 0),
        (Arg, 8),
        (Const, -1),
        (Arg, 12),
        (Const, -17),
        (Call, 0),
        (Pop, 0),
        (Local, 76),
        (Load4, 0),
        (Const, 0),
        (Ne, 0),
    ]);
    let skip = code.len() - 1;
    code.extend([
        (Const, 1),
        (Arg, 8),
        (Const, REASON as i32),
        (Arg, 12),
        (Const, -17),
        (Call, 0),
        (Pop, 0),
    ]);
    code[skip].1 = code.len() as i32;
    code.extend([
        (Const, REASON as i32),
        (Const, i32::from_le_bytes(*b"afte")),
        (Store4, 0),
        (Const, REASON as i32 + 4),
        (Const, i32::from_le_bytes([b'r', 0, 0, 0])),
        (Store4, 0),
        (Const, -1),
        (Arg, 8),
        (Const, TEXT as i32),
        (Arg, 12),
        (Const, -18),
        (Call, 0),
        (Pop, 0),
        (Const, 0),
        (Leave, 64),
    ]);
    code
}
fn qvm_synchronous_drop_preserves_live_fields_and_recurses_through_the_same_table() {
    let mut f = Fixture::new();
    let _anchor = anchor(&mut f);
    let resolver = callbacks(5);
    let mut vm = qvm(&qvm_program(), &f.registry);
    entity(&mut vm.memory, ENTITIES);
    f.register(&mut vm, ENTITIES, PLAYERS, PLAYER_STRIDE);
    vm.memory.write(REASON, b"before\0").unwrap();
    vm.memory.write(TEXT, b"print \"nested\"\0").unwrap();
    let mut unknown = UnknownCalls::load(2).unwrap();
    {
        let mut engine = f.engine();
        engine.callbacks = Some(&resolver);
        engine.client_disconnect = Some(CallbackId(5));
        let mut calls = QvmCalls {
            services: &mut engine,
            table: &Q3_SERVER,
            context: context(),
            platform_time: EventTime(123),
            command: qa_console::command_text::Arguments::from_command(
                qa_core::primitives::CommandArgs {
                    argv: &[b"outer", b"raw\x80"],
                    raw: &[],
                },
            ),
            argument_selection: &mut qa_compat::abi::ArgumentSelection::RetainedClient,
            unknown: &mut unknown,
        };
        assert_eq!(vm.call(&mut calls, [0; 10], 10_000, false), Ok(2));
        assert_eq!(calls.context.clock, context().clock);
        assert_eq!(calls.command.get_bytes(1), b"raw\x80");
        // Invalid and already-Closing pointers are never read, even after return.
        for number in [-1, 2, 0, 1] {
            assert_eq!(calls.call(&mut vm, 16, &[0, number, -1]), Ok(0));
        }
    }
    assert_eq!(vm.memory.read_word(OBSERVED).unwrap(), LIVE);
    assert_eq!(vm.memory.read_word(ENTITIES + 488).unwrap(), LIVE);
    assert_eq!(vm.memory.cstring(REASON).unwrap(), b"after");
    assert_eq!(vm.memory.read_word(COUNT).unwrap(), 2);
    assert_eq!(unknown.calls, 0);
    assert_close(&f);
}

fn free_drop_program() -> Vec<(Opcode, i32)> {
    use Opcode::*;
    let mut instructions = vec![
        (Enter, 64),
        (Local, 72),
        (Load4, 0),
        (Const, 5),
        (Eq, 0),
        (Const, 1),
        (Arg, 8),
        (Local, 76),
        (Load4, 0),
        (Arg, 12),
        (Const, -17),
        (Call, 0),
        (Pop, 0),
        (Const, COUNT as i32),
        (Load4, 0),
        (Leave, 64),
    ];
    instructions[4].1 = instructions.len() as i32;
    instructions.extend([
        (Const, FAIL as i32),
        (Load4, 0),
        (Const, 0),
        (Eq, 0),
        (Const, 1),
        (Const, 0),
        (DivI, 0),
        (Pop, 0),
    ]);
    let skip_trap = instructions.len() - 5;
    instructions[skip_trap].1 = instructions.len() as i32;
    instructions.extend([
        (Const, 1),
        (Arg, 8),
        (Const, -1),
        (Arg, 12),
        (Const, -17),
        (Call, 0),
        (Pop, 0),
        (Const, COUNT as i32),
        (Const, COUNT as i32),
        (Load4, 0),
        (Const, 1),
        (Add, 0),
        (Store4, 0),
        (Const, 0),
        (Leave, 64),
    ]);
    instructions
}
fn qvm_free_close_guard_expires_and_admission_resets_it() {
    let mut f = Fixture::new();
    anchor(&mut f);
    let resolver = callbacks(5);
    let instructions = free_drop_program();
    let mut vm = qvm(&instructions, &f.registry);
    vm.memory.write(REASON, b"reason\0").unwrap();
    let mut unknown = UnknownCalls::load(1).unwrap();
    let mut engine = f.engine();
    engine.callbacks = Some(&resolver);
    engine.client_disconnect = Some(CallbackId(5));
    let mut calls = QvmCalls {
        services: &mut engine,
        table: &Q3_SERVER,
        context: context(),
        platform_time: EventTime::from_milliseconds(5_000),
        command: Default::default(),
        argument_selection: &mut qa_compat::abi::ArgumentSelection::RetainedClient,
        unknown: &mut unknown,
    };
    let mut arguments = [0; 10];
    arguments[1] = REASON as i32;
    assert_eq!(vm.call(&mut calls, arguments, 1_000, false), Ok(1));
    arguments[1] = -1;
    for now in [4_000, 6_999, 7_000] {
        calls.platform_time = EventTime::from_milliseconds(now);
        assert_eq!(vm.call(&mut calls, arguments, 1_000, false), Ok(1));
    }
    calls.platform_time = EventTime::from_milliseconds(7_001);
    arguments[1] = REASON as i32;
    assert_eq!(vm.call(&mut calls, arguments, 1_000, false), Ok(2));
    calls
        .services
        .storage
        .reset_native_close(NativeEntity {
            module: MODULE,
            slot: 1,
        })
        .unwrap();
    calls.platform_time = EventTime::from_milliseconds(7_002);
    assert_eq!(vm.call(&mut calls, arguments, 1_000, false), Ok(3));
    assert_eq!(unknown.calls, 0);
}
fn qvm_negative_free_timeout_keeps_recursive_disconnect_synchronous() {
    let mut f = Fixture::new();
    anchor(&mut f);
    let resolver = callbacks(5);
    let zombie = f
        .cvars
        .bind(
            "sv_zombietime",
            Context {
                source: RuleSetId::Quake3,
                ..Context::default()
            },
        )
        .unwrap();
    f.cvars.force_write(zombie, "-1").unwrap();
    let mut vm = qvm(&free_drop_program(), &f.registry);
    vm.memory.write(REASON, b"reason\0").unwrap();
    let mut unknown = UnknownCalls::load(1).unwrap();
    let mut engine = f.engine();
    engine.callbacks = Some(&resolver);
    engine.client_disconnect = Some(CallbackId(5));
    let mut calls = QvmCalls {
        services: &mut engine,
        table: &Q3_SERVER,
        context: context(),
        platform_time: EventTime::from_milliseconds(5_000),
        command: Default::default(),
        argument_selection: &mut qa_compat::abi::ArgumentSelection::RetainedClient,
        unknown: &mut unknown,
    };
    let mut arguments = [0; 10];
    arguments[1] = REASON as i32;
    assert_eq!(vm.call(&mut calls, arguments, 1_000, false), Ok(1));
    // The previous callback finished; the negative timeout now admits a new
    // independent Drop at the same timestamp, whose recursion still skips.
    assert_eq!(vm.call(&mut calls, arguments, 1_000, false), Ok(2));
    arguments[1] = -1;
    calls.platform_time = EventTime::from_milliseconds(4_999);
    assert_eq!(vm.call(&mut calls, arguments, 1_000, false), Ok(2));
    assert_eq!(unknown.calls, 0);
}

fn qvm_callback_failure_keeps_the_free_timeout_without_an_inflight_leak() {
    let mut f = Fixture::new();
    anchor(&mut f);
    let resolver = callbacks(5);
    let mut vm = qvm(&free_drop_program(), &f.registry);
    vm.memory.write(REASON, b"reason\0").unwrap();
    vm.memory.write_word(FAIL, 1).unwrap();
    let mut unknown = UnknownCalls::load(1).unwrap();
    let mut engine = f.engine();
    engine.callbacks = Some(&resolver);
    engine.client_disconnect = Some(CallbackId(5));
    let mut calls = QvmCalls {
        services: &mut engine,
        table: &Q3_SERVER,
        context: context(),
        platform_time: EventTime::from_milliseconds(5_000),
        command: Default::default(),
        argument_selection: &mut qa_compat::abi::ArgumentSelection::RetainedClient,
        unknown: &mut unknown,
    };
    let mut arguments = [0; 10];
    arguments[1] = REASON as i32;
    assert_eq!(
        vm.call(&mut calls, arguments, 1_000, false),
        Err(qa_compat::qvm::Trap::Division)
    );
    calls.platform_time = EventTime::from_milliseconds(6_000);
    arguments[1] = -1;
    assert_eq!(vm.call(&mut calls, arguments, 1_000, false), Ok(0));
    vm.memory.write_word(FAIL, 0).unwrap();
    calls.platform_time = EventTime::from_milliseconds(7_001);
    arguments[1] = REASON as i32;
    assert_eq!(vm.call(&mut calls, arguments, 1_000, false), Ok(1));
    assert_eq!(unknown.calls, 0);
}

fn qvm_failed_reason_reread_unwinds_the_free_callback_guard() {
    use Opcode::*;
    let mut f = Fixture::new();
    anchor(&mut f);
    let resolver = callbacks(5);
    let mut instructions = free_drop_program();
    instructions.truncate(instructions[4].1 as usize);
    instructions.extend([
        (Const, COUNT as i32),
        (Const, COUNT as i32),
        (Load4, 0),
        (Const, 1),
        (Add, 0),
        (Store4, 0),
        // Host arg9 provides the last NUL; the callback removes it without
        // touching any return PC. Rereading the same address must now fail.
        (Const, 65_535),
        (Const, 120),
        (Store1, 0),
        (Const, 0),
        (Leave, 64),
    ]);
    let mut vm = qvm(&instructions, &f.registry);
    assert_eq!(vm.memory.len(), 65_536);
    let mut unknown = UnknownCalls::load(1).unwrap();
    let mut engine = f.engine();
    engine.callbacks = Some(&resolver);
    engine.client_disconnect = Some(CallbackId(5));
    let mut calls = QvmCalls {
        services: &mut engine,
        table: &Q3_SERVER,
        context: context(),
        platform_time: EventTime::from_milliseconds(5_000),
        command: Default::default(),
        argument_selection: &mut qa_compat::abi::ArgumentSelection::RetainedClient,
        unknown: &mut unknown,
    };
    let mut arguments = [0; 10];
    arguments[1] = 65_535;
    assert_eq!(
        vm.call(&mut calls, arguments, 1_000, false),
        Err(qa_compat::qvm::Trap::Syscall)
    );
    assert_eq!(vm.memory.read_word(COUNT).unwrap(), 1);
    arguments[1] = -1;
    calls.platform_time = EventTime::from_milliseconds(6_000);
    assert_eq!(vm.call(&mut calls, arguments, 1_000, false), Ok(1));
    arguments[1] = 65_535;
    calls.platform_time = EventTime::from_milliseconds(7_001);
    assert_eq!(
        vm.call(&mut calls, arguments, 1_000, false),
        Err(qa_compat::qvm::Trap::Syscall)
    );
    assert_eq!(vm.memory.read_word(COUNT).unwrap(), 2);
    assert_eq!(unknown.calls, 0);
}

fn qvm_drop_shares_the_outer_instruction_budget() {
    let mut f = Fixture::new();
    anchor(&mut f);
    let resolver = callbacks(5);
    let mut vm = qvm(&qvm_program(), &f.registry);
    entity(&mut vm.memory, ENTITIES);
    f.register(&mut vm, ENTITIES, PLAYERS, PLAYER_STRIDE);
    vm.memory.write(REASON, b"before\0").unwrap();
    vm.memory.write(TEXT, b"print \"nested\"\0").unwrap();
    let mut unknown = UnknownCalls::load(1).unwrap();
    let mut engine = f.engine();
    engine.callbacks = Some(&resolver);
    engine.client_disconnect = Some(CallbackId(5));
    let mut calls = QvmCalls {
        services: &mut engine,
        table: &Q3_SERVER,
        context: context(),
        platform_time: EventTime(123),
        command: Default::default(),
        argument_selection: &mut qa_compat::abi::ArgumentSelection::RetainedClient,
        unknown: &mut unknown,
    };
    assert_eq!(
        vm.call(&mut calls, [0; 10], 30, false),
        Err(qa_compat::qvm::Trap::Budget)
    );
    assert_eq!(calls.context.clock, context().clock);
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn owned_pe64_synchronous_drop(trap: bool) {
    use super::native_image::*;
    use qa_compat::native::{NamedExport, NativeCalls, Vm as NativeVm};
    use qa_formats::program::native::{Image, LoadRole};
    use qa_platform::native::NativeScalar;
    fn mov_rax(code: &mut Vec<u8>, address: u64) {
        code.extend([0x48, 0xb8]);
        code.extend(address.to_le_bytes());
    }
    fn store(code: &mut Vec<u8>, address: u64, value: u32) {
        mov_rax(code, address);
        code.extend([0xc7, 0x00]);
        code.extend(value.to_le_bytes());
    }
    fn call(code: &mut Vec<u8>, callback: u64, number: u32, target: u32, text: u64) {
        code.push(0xb9);
        code.extend(number.to_le_bytes());
        code.push(0xba);
        code.extend(target.to_le_bytes());
        code.extend([0x49, 0xb8]);
        code.extend(text.to_le_bytes());
        mov_rax(code, callback);
        code.extend([0xff, 0xd0]);
    }
    fn patch(code: &mut [u8], at: usize, to: usize) {
        code[at..at + 4].copy_from_slice(&((to as i32) - (at as i32 + 4)).to_le_bytes());
    }
    let mut file = pe_export_fixture(64);
    put(&mut file, 208, 12288, 4);
    put(&mut file, 392 + 8, 8192, 4);
    put(&mut file, 392 + 16, 8192, 4);
    pe_rva(&mut file, 0x1140, 0x1200, 4);
    file.resize(8704, 0);
    pe_text(&mut file, 0x1080, &[0xb8, 1, 0, 0, 0, 0xc3]);
    pe_text(&mut file, 0x1200, &[0x31, 0xc0, 0xc3]);
    let named = [0, 5, 15].map(|command| NamedExport {
        name: b"GetGameAPI",
        command: Some(command),
        parameters: &[NativeScalar::I32; 10],
        result: NativeScalar::Word,
    });
    let mut vm = NativeVm::map_image(
        Image::parse(&file, None, LoadRole::Library).unwrap(),
        &named,
        &[],
        std::time::Duration::from_secs(3),
    )
    .unwrap();
    let base = vm.process.base();
    let entities = base + 0x1800;
    let players = base + 0x2400;
    let reason = base + 0x1d00;
    let count = base + 0x1d40;
    let observed = base + 0x1d44;
    let text = base + 0x1d80;
    let callback = vm.import_callback();
    let mut code = vec![
        0x48, 0x83, 0xec, 0x48, 0x89, 0x54, 0x24, 0x40, 0x83, 0xf9, 5, 0x0f, 0x84,
    ];
    let disconnect = code.len();
    code.extend([0; 4]);
    code.extend([0x83, 0xf9, 15, 0x0f, 0x84]);
    let register = code.len();
    code.extend([0; 4]);
    store(&mut code, entities + 488, LIVE as u32);
    call(&mut code, callback, 16, 0, reason);
    mov_rax(&mut code, count);
    code.extend([0x8b, 0x00, 0x48, 0x83, 0xc4, 0x48, 0xc3]);
    let at = code.len();
    patch(&mut code, disconnect, at);
    mov_rax(&mut code, entities + 488);
    code.extend([0x8b, 0x10]);
    mov_rax(&mut code, observed);
    code.extend([0x89, 0x10]);
    mov_rax(&mut code, count);
    code.extend([0x83, 0x00, 1]);
    if trap {
        code.extend([0x83, 0x7c, 0x24, 0x40, 1, 0x0f, 0x85]);
        let skip_abort = code.len();
        code.extend([0; 4]);
        code.extend([0xb9, 1, 0, 0, 0, 0x48, 0xba]);
        code.extend(reason.to_le_bytes());
        mov_rax(&mut code, callback);
        code.extend([0xff, 0xd0]);
        let at = code.len();
        patch(&mut code, skip_abort, at);
    }
    // Recursive Drop uses the suspended native ordinal and an unreadable pointer.
    code.extend([0xb9, 16, 0, 0, 0, 0x8b, 0x54, 0x24, 0x40, 0x49, 0xb8]);
    code.extend(u64::MAX.to_le_bytes());
    mov_rax(&mut code, callback);
    code.extend([0xff, 0xd0]);
    code.extend([0x83, 0x7c, 0x24, 0x40, 0, 0x0f, 0x85]);
    let skip = code.len();
    code.extend([0; 4]);
    call(&mut code, callback, 16, 1, reason);
    let at = code.len();
    patch(&mut code, skip, at);
    store(&mut code, reason, u32::from_le_bytes(*b"afte"));
    store(&mut code, reason + 4, u32::from_le_bytes([b'r', 0, 0, 0]));
    call(&mut code, callback, 17, u32::MAX, text);
    code.extend([0x31, 0xc0, 0x48, 0x83, 0xc4, 0x48, 0xc3]);
    let at = code.len();
    patch(&mut code, register, at);
    // Same LocateGameData handler as production, with two load-reserved players.
    code.extend([0xb9, 15, 0, 0, 0, 0x48, 0xba]);
    code.extend(entities.to_le_bytes());
    code.extend([0x41, 0xb8, 1, 0, 0, 0, 0x41, 0xb9]);
    code.extend((ENTITY_STRIDE as u32).to_le_bytes());
    mov_rax(&mut code, players);
    code.extend([0x48, 0x89, 0x44, 0x24, 32, 0x48, 0xc7, 0x44, 0x24, 40]);
    code.extend((PLAYER_STRIDE as u32).to_le_bytes());
    mov_rax(&mut code, callback);
    code.extend([0xff, 0xd0, 0x48, 0x83, 0xc4, 0x48, 0xc3]);
    let mut f = Fixture::new();
    anchor(&mut f);
    let resolver = callbacks(1);
    vm.configure_q3_game(Q3StatLayout::BASE);
    vm.bind_player_values(value_bindings(&player_bindings()), &f.registry)
        .unwrap();
    {
        let mut memory = qa_compat::memory::process_memory(&mut vm.process).unwrap();
        memory.write(base + 0x1200, &code).unwrap();
        entity(&mut memory, entities);
        memory.write(reason, b"before\0").unwrap();
        memory.write(text, b"print \"nested\"\0").unwrap();
    }
    let mut unknown = UnknownCalls::load(2).unwrap();
    {
        let mut engine = f.engine();
        engine.callbacks = Some(&resolver);
        engine.client_disconnect = Some(CallbackId(5));
        let mut calls = NativeCalls {
            services: &mut engine,
            table: &Q3_SERVER,
            context: context(),
            platform_time: EventTime(123),
            command: qa_console::command_text::Arguments::from_command(
                qa_core::primitives::CommandArgs {
                    argv: &[b"outer"],
                    raw: b"outer",
                },
            ),
            argument_selection: &mut qa_compat::abi::ArgumentSelection::RetainedClient,
            unknown: &mut unknown,
            native_player: None,
        };
        assert_eq!(vm.call(&mut calls, 2, &[]).unwrap(), 0);
    }
    vm.bind_client(&f.server.entities, NATIVE, f.server.clients[1].entity)
        .unwrap();
    {
        let mut engine = f.engine();
        engine.callbacks = Some(&resolver);
        engine.client_disconnect = Some(CallbackId(5));
        let mut calls = NativeCalls {
            services: &mut engine,
            table: &Q3_SERVER,
            context: context(),
            platform_time: EventTime(123),
            command: qa_console::command_text::Arguments::from_command(
                qa_core::primitives::CommandArgs {
                    argv: &[b"outer"],
                    raw: b"outer",
                },
            ),
            argument_selection: &mut qa_compat::abi::ArgumentSelection::RetainedClient,
            unknown: &mut unknown,
            native_player: None,
        };
        let result = vm.call(&mut calls, 0, &[]);
        assert_eq!(calls.command.len(), 1);
        assert_eq!(calls.command.get_bytes(0), b"outer");
        if trap {
            assert!(matches!(
                result,
                Err(qa_compat::native::Error::Service(CallError::Aborted))
            ));
            let free = NativeEntity {
                module: MODULE,
                slot: 1,
            };
            assert!(
                (qa_compat::services::ENGINE_CALLS.drop_client_target)(
                    calls.services,
                    free,
                    EventTime::from_milliseconds(1_000)
                )
                .unwrap()
                .is_none()
            );
            assert!(
                (qa_compat::services::ENGINE_CALLS.drop_client_target)(
                    calls.services,
                    free,
                    EventTime::from_milliseconds(2_001)
                )
                .unwrap()
                .is_some()
            );
            return;
        }
        assert_eq!(result.unwrap(), 2);
    }
    {
        let memory = qa_compat::memory::process_memory(&mut vm.process).unwrap();
        assert_eq!(memory.read_word(observed).unwrap(), LIVE);
        assert_eq!(memory.read_word(entities + 488).unwrap(), LIVE);
        assert_eq!(memory.read_word(count).unwrap(), 2);
        assert_eq!(memory.cstring(reason).unwrap(), b"after");
    }
    assert_eq!(unknown.calls, 0);
    assert_close(&f);
}

pub(super) fn run() {
    qvm_synchronous_drop_preserves_live_fields_and_recurses_through_the_same_table();
    qvm_free_close_guard_expires_and_admission_resets_it();
    qvm_negative_free_timeout_keeps_recursive_disconnect_synchronous();
    qvm_drop_shares_the_outer_instruction_budget();
    qvm_callback_failure_keeps_the_free_timeout_without_an_inflight_leak();
    qvm_failed_reason_reread_unwinds_the_free_callback_guard();
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    owned_pe64_synchronous_drop(false);
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    owned_pe64_synchronous_drop(true);
}
