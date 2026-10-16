#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
use super::native_image;
use super::service_program;
use qa_app::{
    Runtime,
    host::FrameHost,
    modules::{Export, ModuleRequest, ModuleResult, Phase, Program},
};
use qa_compat::{abi::Q3_SERVER, services::CallContext};
use qa_console::{commands::Console, views::Context};
use qa_core::{
    events::FrameEvent,
    primitives::{
        CallbackId, ClientId, ModuleId, NativeEntity, Q3StatLayout, RuleSetId, ThinkTime,
    },
};
use qa_formats::program::qvm::Opcode;
use qa_session::{
    clients::{ClientPhase, Connection},
    timing::TickRate,
};
use qa_world::{area::LinkOrder, entities::AllocationPolicy};

const MODULE: ModuleId = ModuleId(21);
const OTHER_MODULE: ModuleId = ModuleId(22);
const TARGET_NATIVE: i32 = 3;
const FREE_NATIVE: i32 = 4;
const INVALID_NATIVE: i32 = 64;

const ENTITIES: i32 = 4096;
const ENTITY_COUNT: i32 = 8;
const ENTITY_STRIDE: i32 = 608;
const PLAYERS: i32 = 12_288;
const PLAYER_STRIDE: i32 = 468;
const CONNECTED_COUNT: i32 = 256;
const FREE_COUNT: i32 = 260;
const REASON: i32 = 512;
const FREE_REASON: i32 = 544;
const FRAME_OFFSET: i32 = 172;
const LINKED_OFFSET: i32 = 416;
const OUTER_FRAME: i32 = 111;
const CALLBACK_FRAME: i32 = 222;

fn qvm_store(operations: &mut Vec<(Opcode, i32)>, address: i32, value: i32) {
    operations.extend([
        (Opcode::Const, address),
        (Opcode::Const, value),
        (Opcode::Store4, 0),
    ]);
}

fn qvm_increment(operations: &mut Vec<(Opcode, i32)>, address: i32) {
    operations.extend([
        (Opcode::Const, address),
        (Opcode::Const, address),
        (Opcode::Load4, 0),
        (Opcode::Const, 1),
        (Opcode::Add, 0),
        (Opcode::Store4, 0),
    ]);
}

fn qvm_drop(operations: &mut Vec<(Opcode, i32)>, native: i32, reason: i32) {
    operations.extend([
        (Opcode::Const, native),
        (Opcode::Arg, 8),
        (Opcode::Const, reason),
        (Opcode::Arg, 12),
        // QVM negative call targets encode -(system-call ordinal + 1).
        (Opcode::Const, -17),
        (Opcode::Call, 0),
        (Opcode::Pop, 0),
    ]);
}

fn qvm_program(drop_from_client_think: bool) -> Program {
    use Opcode::*;

    let mut operations = vec![(Enter, 96)];
    let mut dispatch = Vec::new();
    for command in 0..8 {
        operations.extend([(Local, 104), (Load4, 0), (Const, command), (Eq, 0)]);
        dispatch.push(operations.len() - 1);
    }
    operations.extend([(Const, 0), (Leave, 96)]);

    let connected_row = ENTITIES + TARGET_NATIVE * ENTITY_STRIDE;
    let mut drop_handler = None;
    for (command, branch) in dispatch.into_iter().enumerate() {
        if command == 7 && drop_from_client_think {
            operations[branch].1 = drop_handler.expect("drop handler");
            continue;
        }
        operations[branch].1 = operations.len() as i32;
        match command {
            0 => {
                drop_handler = Some(operations.len() as i32);
                for (offset, value) in [
                    (8, ENTITIES),
                    (12, ENTITY_COUNT),
                    (16, ENTITY_STRIDE),
                    (20, PLAYERS),
                    (24, PLAYER_STRIDE),
                ] {
                    operations.extend([(Const, value), (Arg, offset)]);
                }
                operations.extend([(Const, -16), (Call, 0), (Pop, 0)]);
                qvm_store(&mut operations, connected_row, TARGET_NATIVE);
                qvm_store(&mut operations, connected_row + LINKED_OFFSET, 1);
                qvm_store(&mut operations, connected_row + FRAME_OFFSET, OUTER_FRAME);
                qvm_drop(&mut operations, TARGET_NATIVE, REASON);
                operations.extend([
                    (Const, connected_row + FRAME_OFFSET),
                    (Load4, 0),
                    (Leave, 96),
                ]);
            }
            1 => {
                qvm_drop(&mut operations, -1, 0);
                qvm_drop(&mut operations, INVALID_NATIVE, 0);
                operations.extend([(Const, 0), (Leave, 96)]);
            }
            2 => {
                qvm_drop(&mut operations, FREE_NATIVE, FREE_REASON);
                operations.extend([(Const, FREE_COUNT), (Load4, 0), (Leave, 96)]);
            }
            5 => {
                operations.extend([(Local, 108), (Load4, 0), (Const, TARGET_NATIVE), (Eq, 0)]);
                let connected = operations.len() - 1;
                operations.extend([(Local, 108), (Load4, 0), (Const, FREE_NATIVE), (Eq, 0)]);
                let free = operations.len() - 1;
                operations.extend([(Const, -1), (Leave, 96)]);

                operations[connected].1 = operations.len() as i32;
                qvm_increment(&mut operations, CONNECTED_COUNT);
                operations.extend([
                    (Const, connected_row + FRAME_OFFSET),
                    (Load4, 0),
                    (Const, OUTER_FRAME),
                    (Eq, 0),
                ]);
                let marker_matches = operations.len() - 1;
                operations.extend([(Const, 0), (Leave, 96)]);
                operations[marker_matches].1 = operations.len() as i32;
                qvm_store(
                    &mut operations,
                    connected_row + FRAME_OFFSET,
                    CALLBACK_FRAME,
                );
                qvm_store(&mut operations, REASON, i32::from_le_bytes(*b"chan"));
                qvm_store(&mut operations, REASON + 4, i32::from_le_bytes(*b"ged\0"));
                // Re-entering the same native ordinal must return before the
                // null reason is dereferenced or another callback is made.
                qvm_drop(&mut operations, TARGET_NATIVE, 0);
                operations.extend([(Const, 0), (Leave, 96)]);

                operations[free].1 = operations.len() as i32;
                qvm_increment(&mut operations, FREE_COUNT);
                qvm_drop(&mut operations, FREE_NATIVE, 0);
                operations.extend([(Const, 0), (Leave, 96)]);
            }
            6 => operations.extend([(Const, CONNECTED_COUNT), (Load4, 0), (Leave, 96)]),
            7 => operations.extend([(Const, FREE_COUNT), (Load4, 0), (Leave, 96)]),
            _ => operations.extend([(Const, 0), (Leave, 96)]),
        }
    }

    let mut data = vec![0; 16_384];
    data[REASON as usize..REASON as usize + 9].copy_from_slice(b"original\0");
    data[FREE_REASON as usize..FREE_REASON as usize + 5].copy_from_slice(b"free\0");
    let mut vm = service_program::program(&operations, &data);
    vm.configure_q3_game(Q3StatLayout::BASE);
    Program::Qvm {
        vm: Box::new(vm),
        imports: &Q3_SERVER,
    }
}

fn context() -> CallContext {
    CallContext {
        module: MODULE,
        clock: ThinkTime::Milliseconds(0),
        server_frame: 0,
        native_rules: RuleSetId::Quake3,
        trace_rules: RuleSetId::Quake3,
        sound_policy: qa_gameplay::rules::sound_policy(RuleSetId::Quake3),
        console: Context {
            source: RuleSetId::Quake3,
            ..Context::default()
        },
        allocation: AllocationPolicy::q3(0),
        link_order: LinkOrder::Head,
    }
}

fn host(program: Program) -> (FrameHost, ClientId, ClientId) {
    let mut runtime = Runtime::load(2, []).unwrap();
    let other = runtime
        .server
        .connect(
            Connection::Remote,
            OTHER_MODULE,
            Some(NativeEntity {
                module: OTHER_MODULE,
                slot: TARGET_NATIVE,
            }),
        )
        .unwrap();
    let target = runtime
        .server
        .connect(
            Connection::Remote,
            MODULE,
            Some(NativeEntity {
                module: MODULE,
                slot: TARGET_NATIVE,
            }),
        )
        .unwrap();
    assert_ne!(target.0, TARGET_NATIVE as u32);
    runtime
        .server
        .replace_userinfo(
            target,
            b"\\name\\Dropper",
            qa_compat::services::userinfo_rules(RuleSetId::Quake3),
        )
        .unwrap();
    let anchor = runtime
        .server
        .entities
        .allocate(ThinkTime::Milliseconds(0), MODULE, AllocationPolicy::q3(0))
        .unwrap()
        .id;
    let request = ModuleRequest {
        q3_image: None,
        context: context(),
        phase: Phase::Server,
        timing_rules: RuleSetId::Quake3,
        rate: TickRate::FrameDriven,
        anchor,
        program,
        entries: (0..8).collect(),
        frame: Export::clocked(CallbackId(7)),
        prepare: Vec::new(),
        initialize: Vec::new(),
        api: None,
        shutdown: Vec::new(),
        instruction_budget: 5_000,
        configstrings: 0,
        files: 0,
        entity_source: None,
    };
    let mut console = Console::new(Context::default()).unwrap();
    let max_clients = console
        .cvars
        .bind("sv_maxclients", context().console)
        .unwrap();
    console.cvars.force_write(max_clients, "8").unwrap();
    (
        FrameHost::load_modules(console, runtime, TickRate::FrameDriven, vec![request]).unwrap(),
        other,
        target,
    )
}

fn invoke(host: &mut FrameHost, callback: u32) -> u64 {
    host.call_module_export(
        MODULE,
        Export::words(CallbackId(callback), &[]),
        ThinkTime::Milliseconds(17),
    )
    .unwrap();
    match host.module_counts(MODULE).unwrap().last_result.unwrap() {
        ModuleResult::Qvm(value) => u64::from(value as u32),
        ModuleResult::Native(value) => value,
        ModuleResult::QuakeC(_) => panic!("word module"),
    }
}

fn commands(host: &FrameHost, client: ClientId) -> Vec<(u64, Vec<u8>)> {
    let output = host.runtime.server.clients[client.0 as usize]
        .output
        .unwrap();
    let events = &host.runtime.server.events;
    let mut batch = events.batch(output).unwrap();
    let mut commands = Vec::new();
    while let Some(record) = events.next(&mut batch) {
        if let FrameEvent::ServerCommand { text, .. } = record.event {
            commands.push((record.sequence, events.texts.get(text).unwrap().to_vec()));
        }
    }
    commands
}

fn verify(program: Program) {
    let (mut host, other, target) = host(program);

    // Both invalid native ordinals return before evaluating the null reason.
    assert_eq!(invoke(&mut host, 1), 0);
    assert!(commands(&host, other).is_empty());
    assert!(commands(&host, target).is_empty());

    // Callback 5 runs below the suspended import. It observes the outer
    // public-entity write, changes it, mutates the reason, and recursively
    // attempts the same drop with a null pointer. The outer call sees its
    // marker before returning and the one publication imports it afterward.
    assert_eq!(invoke(&mut host, 0), CALLBACK_FRAME as u64);
    assert_eq!(invoke(&mut host, 6), 1);
    let lifetime = host.runtime.server.clients[target.0 as usize].entity;
    assert_eq!(
        host.runtime.server.entities.columns.appearance[lifetime.slot as usize].frame,
        CALLBACK_FRAME,
    );
    assert_eq!(host.runtime.server.userinfo(target), Some(&b""[..]));

    let target_commands = commands(&host, target);
    assert_eq!(
        target_commands
            .iter()
            .map(|(_, text)| text.as_slice())
            .collect::<Vec<_>>(),
        [
            b"print \"Dropper^7 original\n\"".as_slice(),
            b"disconnect \"changed\"".as_slice(),
        ],
    );
    let (final_sequence, _) = target_commands[1];
    let output = host.runtime.server.clients[target.0 as usize]
        .output
        .unwrap();
    assert!(matches!(
        host.runtime.server.clients[target.0 as usize].phase,
        ClientPhase::Closing(closing)
            if closing.final_output == Some((output, final_sequence))
                && !closing.acknowledged
    ));
    assert_eq!(
        commands(&host, other)
            .iter()
            .map(|(_, text)| text.as_slice())
            .collect::<Vec<_>>(),
        [b"print \"Dropper^7 original\n\"".as_slice()],
    );

    // A valid FREE native row has no canonical client lifetime. Callback
    // resolution therefore has to use the module anchor, and its recursive
    // same-row drop must still be suppressed.
    assert_eq!(invoke(&mut host, 2), 1);
    assert_eq!(invoke(&mut host, 7), 1);
    assert_eq!(
        commands(&host, other)
            .iter()
            .map(|(_, text)| text.as_slice())
            .collect::<Vec<_>>(),
        [
            b"print \"Dropper^7 original\n\"".as_slice(),
            b"print \"^7 free\n\"".as_slice(),
        ],
    );
    assert_eq!(host.module_counts(MODULE).unwrap().traps, 0);
}

pub fn qvm_drop_client_is_synchronous_and_lifetime_checked() {
    verify(qvm_program(false));
}

pub fn drop_during_client_think_releases_the_implicit_command_batch() {
    let (mut host, healthy, target) = host(qvm_program(true));
    assert!(host.runtime.server.bind_client_native(
        healthy,
        NativeEntity {
            module: MODULE,
            slot: 2,
        },
    ));
    for client in [healthy, target] {
        host.runtime.server.clients[client.0 as usize].module_movement = true;
    }

    let rejected = host.module_counts(MODULE).unwrap().rejected;
    host.run_module_client_command(MODULE, target, &Default::default(), 1)
        .unwrap();
    let calls_after_drop = host.module_counts(MODULE).unwrap().calls;
    let output = host.runtime.server.clients[target.0 as usize]
        .output
        .unwrap();
    let ClientPhase::Closing(closing) = host.runtime.server.clients[target.0 as usize].phase else {
        panic!("DropClient did not retain the target in its closing lifetime")
    };
    assert_eq!(closing.final_output.map(|(owner, _)| owner), Some(output));
    assert!(!closing.acknowledged);
    assert!(host.runtime.server.clients[healthy.0 as usize].is_active());

    for completed in 1..=2 {
        host.run_module_client_command(MODULE, healthy, &Default::default(), completed + 1)
            .unwrap();
        assert_eq!(
            host.module_counts(MODULE).unwrap().calls,
            calls_after_drop + completed as u64,
        );
        assert_eq!(host.module_counts(MODULE).unwrap().rejected, rejected);
        assert!(host.runtime.server.clients[healthy.0 as usize].is_active());
        assert!(matches!(
            host.runtime.server.clients[target.0 as usize].phase,
            ClientPhase::Closing(current) if current == closing
        ));
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn patch_rel32(code: &mut [u8], displacement_at: usize, target: usize) {
    let displacement = i32::try_from(target as isize - (displacement_at + 4) as isize).unwrap();
    code[displacement_at..displacement_at + 4].copy_from_slice(&displacement.to_le_bytes());
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn jump_equal(code: &mut Vec<u8>) -> usize {
    code.extend([0x0f, 0x84]);
    let at = code.len();
    code.extend([0; 4]);
    at
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn mov_rax(code: &mut Vec<u8>, value: u64) {
    code.extend([0x48, 0xb8]);
    code.extend(value.to_le_bytes());
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn native_call(code: &mut Vec<u8>, callback: u64, native: i32, reason: u64) {
    code.extend([0xb9]);
    code.extend(16u32.to_le_bytes());
    code.extend([0xba]);
    code.extend((native as u32).to_le_bytes());
    code.extend([0x49, 0xb8]);
    code.extend(reason.to_le_bytes());
    mov_rax(code, callback);
    code.extend([0xff, 0xd0]);
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn native_program() -> Program {
    use native_image::*;
    use qa_compat::{
        memory::process_memory,
        native::{NamedExport, Vm},
    };
    use qa_formats::program::native::{Image, LoadRole};
    use qa_platform::native::NativeScalar;
    use std::time::Duration;

    let mut file = pe_export_fixture(64);
    put(&mut file, 208, 0x8000, 4);
    put(&mut file, 392 + 8, 0x7000, 4);
    put(&mut file, 392 + 16, 0x7000, 4);
    pe_rva(&mut file, 0x1140, 0x1200, 4);
    file.resize(512 + 0x7000, 0);
    pe_text(&mut file, 0x1200, &[0x31, 0xc0, 0xc3]);
    let image = Image::parse(&file, None, LoadRole::Library).unwrap();
    let base = image.base;
    let mut vm = Vm::map_image(
        image,
        &(0..8)
            .map(|command| NamedExport {
                name: b"GetGameAPI",
                command: Some(command),
                parameters: &[NativeScalar::Word; 13],
                result: NativeScalar::Word,
            })
            .collect::<Vec<_>>(),
        &[],
        Duration::from_secs(3),
    )
    .unwrap();
    vm.configure_q3_game(Q3StatLayout::BASE);

    let callback = vm.import_callback();
    let entities = base + 0x2000;
    let players = base + 0x4000;
    let connected_count = base + 0x5000;
    let free_count = base + 0x5004;
    let reason = base + 0x5100;
    let free_reason = base + 0x5120;
    let connected_row = entities + TARGET_NATIVE as u64 * ENTITY_STRIDE as u64;

    let mut code = Vec::new();
    let mut dispatch = Vec::new();
    for command in 0..8u8 {
        code.extend([0x83, 0xf9, command]);
        dispatch.push(jump_equal(&mut code));
    }
    code.extend([0x31, 0xc0, 0xc3]);
    let mut handlers = [0usize; 8];

    handlers[0] = code.len();
    code.extend([0x48, 0x83, 0xec, 0x38]);
    code.extend([0xb9]);
    code.extend(15u32.to_le_bytes());
    code.extend([0x48, 0xba]);
    code.extend(entities.to_le_bytes());
    code.extend([0x41, 0xb8]);
    code.extend((ENTITY_COUNT as u32).to_le_bytes());
    code.extend([0x41, 0xb9]);
    code.extend((ENTITY_STRIDE as u32).to_le_bytes());
    mov_rax(&mut code, players);
    code.extend([0x48, 0x89, 0x44, 0x24, 0x20]);
    code.extend([0xc7, 0x44, 0x24, 0x28]);
    code.extend((PLAYER_STRIDE as u32).to_le_bytes());
    mov_rax(&mut code, callback);
    code.extend([0xff, 0xd0]);
    mov_rax(&mut code, connected_row);
    for (offset, value) in [
        (0u32, TARGET_NATIVE),
        (LINKED_OFFSET as u32, 1),
        (FRAME_OFFSET as u32, OUTER_FRAME),
    ] {
        code.extend([0xc7, 0x80]);
        code.extend(offset.to_le_bytes());
        code.extend((value as u32).to_le_bytes());
    }
    native_call(&mut code, callback, TARGET_NATIVE, reason);
    mov_rax(&mut code, connected_row);
    code.extend([0x8b, 0x80]);
    code.extend((FRAME_OFFSET as u32).to_le_bytes());
    code.extend([0x48, 0x83, 0xc4, 0x38, 0xc3]);

    handlers[1] = code.len();
    code.extend([0x48, 0x83, 0xec, 0x28]);
    native_call(&mut code, callback, -1, 0);
    native_call(&mut code, callback, INVALID_NATIVE, 0);
    code.extend([0x31, 0xc0, 0x48, 0x83, 0xc4, 0x28, 0xc3]);

    handlers[2] = code.len();
    code.extend([0x48, 0x83, 0xec, 0x28]);
    native_call(&mut code, callback, FREE_NATIVE, free_reason);
    mov_rax(&mut code, free_count);
    code.extend([0x8b, 0x00, 0x48, 0x83, 0xc4, 0x28, 0xc3]);

    for handler in &mut handlers[3..5] {
        *handler = code.len();
        code.extend([0x31, 0xc0, 0xc3]);
    }

    handlers[5] = code.len();
    code.extend([0x83, 0xfa, TARGET_NATIVE as u8]);
    let connected = jump_equal(&mut code);
    code.extend([0x83, 0xfa, FREE_NATIVE as u8]);
    let free = jump_equal(&mut code);
    code.extend([0x31, 0xc0, 0xc3]);

    let connected_handler = code.len();
    code.extend([0x48, 0x83, 0xec, 0x28]);
    mov_rax(&mut code, connected_count);
    code.extend([0x83, 0x00, 1]);
    mov_rax(&mut code, connected_row);
    code.extend([0x83, 0xb8]);
    code.extend((FRAME_OFFSET as u32).to_le_bytes());
    code.push(OUTER_FRAME as u8);
    let marker_matches = jump_equal(&mut code);
    code.extend([0x31, 0xc0, 0x48, 0x83, 0xc4, 0x28, 0xc3]);
    let marker_handler = code.len();
    mov_rax(&mut code, connected_row);
    code.extend([0xc7, 0x80]);
    code.extend((FRAME_OFFSET as u32).to_le_bytes());
    code.extend((CALLBACK_FRAME as u32).to_le_bytes());
    mov_rax(&mut code, reason);
    code.extend([0xc7, 0x00]);
    code.extend(u32::from_le_bytes(*b"chan").to_le_bytes());
    code.extend([0xc7, 0x40, 4]);
    code.extend(u32::from_le_bytes(*b"ged\0").to_le_bytes());
    native_call(&mut code, callback, TARGET_NATIVE, 0);
    code.extend([0x31, 0xc0, 0x48, 0x83, 0xc4, 0x28, 0xc3]);

    let free_handler = code.len();
    code.extend([0x48, 0x83, 0xec, 0x28]);
    mov_rax(&mut code, free_count);
    code.extend([0x83, 0x00, 1]);
    native_call(&mut code, callback, FREE_NATIVE, 0);
    code.extend([0x31, 0xc0, 0x48, 0x83, 0xc4, 0x28, 0xc3]);

    handlers[6] = code.len();
    mov_rax(&mut code, connected_count);
    code.extend([0x8b, 0x00, 0xc3]);
    handlers[7] = code.len();
    mov_rax(&mut code, free_count);
    code.extend([0x8b, 0x00, 0xc3]);

    for (branch, handler) in dispatch.into_iter().zip(handlers) {
        patch_rel32(&mut code, branch, handler);
    }
    patch_rel32(&mut code, connected, connected_handler);
    patch_rel32(&mut code, free, free_handler);
    patch_rel32(&mut code, marker_matches, marker_handler);
    assert!(code.len() < 0xe00);
    {
        let mut memory = process_memory(&mut vm.process).unwrap();
        memory.write(base + 0x1200, &code).unwrap();
        memory.write(reason, b"original\0").unwrap();
        memory.write(free_reason, b"free\0").unwrap();
    }

    Program::Native {
        vm: Box::new(vm),
        imports: &Q3_SERVER,
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub fn owned_native_drop_client_uses_the_same_nested_callback_path() {
    verify(native_program());
}
