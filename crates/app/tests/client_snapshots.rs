#[path = "support/client_asset_frames.rs"]
mod asset_frames;
#[path = "support/frame_source.rs"]
mod frame_source;

#[path = "model_assets/support.rs"]
#[allow(dead_code)]
mod model_files;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "../../formats/tests/support/native_image.rs"]
#[allow(dead_code)]
mod native_image;
#[path = "support/service_program.rs"]
mod service_program;

use qa_app::{
    Runtime,
    client_policy::ClientPolicy,
    host::{FrameHost, FrameResult},
    map::SpawnAnchor,
    modules::{Argument, Export, ModuleRequest, ModuleResult, Phase, Program, State},
};
use qa_compat::{abi::Q3_CLIENT, services::CallContext};
use qa_console::{commands::Console, views::Context};
use qa_core::{
    loopback::Endpoint,
    primitives::*,
    sys_events::{EventTime, SeatId},
};
use qa_formats::program::qvm::Opcode;
use qa_network::{
    commands::packet::Protocol,
    snapshots::{Entity, Frame, ReceivedFrame},
    states,
};
use qa_session::timing::TickRate;
use qa_world::{area::LinkOrder, entities::AllocationPolicy};

const SNAPSHOT_BYTES: usize = 53772;
const QVM_OUTPUT: u64 = 1024;
const GET_GAME_STATE: u32 = 7;
const SELECT_COMMAND: u32 = 8;
const READ_BYTE: u32 = 9;

fn qvm() -> (Program, u64) {
    qvm_with_renderer(&Q3_CLIENT, 49)
}
fn qvm_ui_renderer() -> (Program, u64) {
    qvm_with_renderer(&qa_compat::abi::Q3_UI, 43)
}
fn qvm_with_renderer(imports: &'static qa_compat::abi::CallTable, renderer: i32) -> (Program, u64) {
    use Opcode::*;
    let mut operations = vec![(Enter, 64)];
    let mut branches = Vec::new();
    for command in 0..18 {
        operations.extend([(Local, 72), (Load4, 0), (Const, command), (Eq, 0)]);
        branches.push(operations.len() - 1);
    }
    operations.extend([(Const, -1), (Leave, 64)]);
    for (command, branch) in branches.into_iter().enumerate() {
        operations[branch].1 = operations.len() as i32;
        match command {
            0 | 1 => {
                operations.extend([
                    (Const, QVM_OUTPUT as i32),
                    (Arg, 8),
                    (Const, QVM_OUTPUT as i32 + if command == 0 { 4 } else { 0 }),
                    (Arg, 12),
                    (Const, -52),
                    (Call, 0),
                    (Pop, 0),
                    (Const, QVM_OUTPUT as i32),
                    (Load4, 0),
                ]);
            }
            2 | 5 => {
                operations.extend([
                    (Local, 76),
                    (Load4, 0),
                    (Arg, 8),
                    (Local, 80),
                    (Load4, 0),
                    (Arg, 12),
                    (Const, if command == 2 { -53 } else { -52 }),
                    (Call, 0),
                ]);
            }
            3 => operations.extend([(Local, 76), (Load4, 0), (Load4, 0)]),
            4 => operations.extend([
                (Local, 76),
                (Load4, 0),
                (Arg, 8),
                (Local, 80),
                (Load4, 0),
                (Arg, 12),
                (Local, 84),
                (Load4, 0),
                (Arg, 16),
                (Const, -101),
                (Call, 0),
            ]),
            6 => operations.extend([(Local, 76), (Load4, 0)]),
            7 | 8 => operations.extend([
                (Local, 76),
                (Load4, 0),
                (Arg, 8),
                (Const, if command == 7 { -51 } else { -54 }),
                (Call, 0),
            ]),
            9 => operations.extend([(Local, 76), (Load4, 0), (Load1, 0)]),
            10 => operations.extend([
                (Local, 76),
                (Load4, 0),
                (Arg, 8),
                (Const, -renderer - 1),
                (Call, 0),
            ]),
            11 => operations.extend([(Const, 128), (Arg, 8), (Const, -40), (Call, 0)]),
            12 | 13 => {
                operations.extend([(Local, 76), (Load4, 0), (Arg, 8)]);
                if command == 13 {
                    operations.extend([
                        (Local, 80),
                        (Load4, 0),
                        (Arg, 12),
                        (Local, 84),
                        (Load4, 0),
                        (Arg, 16),
                    ]);
                }
                let ordinal = match (renderer, command) {
                    (49, 12) => 37,
                    (49, _) => 47,
                    (_, 12) => 18,
                    _ => 56,
                };
                operations.extend([(Const, -ordinal - 1), (Call, 0)]);
            }
            14 => {
                for argument in 0..6 {
                    operations.extend([
                        (Local, 76 + argument * 4),
                        (Load4, 0),
                        (Arg, 8 + argument * 4),
                    ]);
                }
                let ordinal = if renderer == 49 { 48 } else { 29 };
                operations.extend([(Const, -ordinal - 1), (Call, 0)]);
            }
            15 => {
                for ordinal in [18, 36] {
                    operations.extend([
                        (Const, 512),
                        (Arg, 8),
                        (Const, -ordinal - 1),
                        (Call, 0),
                        (Pop, 0),
                    ]);
                }
                operations.extend([
                    (Const, QVM_OUTPUT as i32 + 64),
                    (Const, -20),
                    (Call, 0),
                    (Store4, 0),
                    (Const, QVM_OUTPUT as i32 + 64),
                    (Load4, 0),
                ]);
            }
            16 => {
                for (argument, value) in [QVM_OUTPUT as i32, 544, 560, 576, 576, 0, 1]
                    .into_iter()
                    .enumerate()
                {
                    operations.extend([(Const, value), (Arg, 8 + argument as i32 * 4)]);
                }
                operations.extend([(Const, -26), (Call, 0)]);
            }
            17 => operations.extend([(Const, 608), (Arg, 8), (Const, -45), (Call, 0)]),
            _ => unreachable!(),
        }
        operations.push((Leave, 64));
    }
    let mut data = [0; 1024];
    data[128..140].copy_from_slice(b"ui/cold.tga\0");
    data[192..208].copy_from_slice(b"models/test.md2\0");
    data[224..240].copy_from_slice(b"models/test.md3\0");
    data[240..251].copy_from_slice(b"tag_weapon\0");
    data[512..1024].copy_from_slice(&world_import_data());
    (
        Program::Qvm {
            vm: Box::new(service_program::program(&operations, &data)),
            imports,
        },
        QVM_OUTPUT,
    )
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn native() -> (Program, u64) {
    native_with_renderer(&Q3_CLIENT, 49)
}
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn native_ui_renderer() -> (Program, u64) {
    native_with_renderer(&qa_compat::abi::Q3_UI, 43)
}
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn native_with_renderer(
    imports: &'static qa_compat::abi::CallTable,
    renderer: u8,
) -> (Program, u64) {
    use native_image::*;
    use qa_compat::native::{NamedExport, Vm};
    use qa_formats::program::native::{Image, LoadRole};
    use qa_platform::native::NativeScalar;
    let mut file = pe_export_fixture(64);
    put(&mut file, 392 + 8, 65536, 4);
    put(&mut file, 392 + 16, 65536, 4);
    put(&mut file, 208, 69632, 4);
    file.resize(65536 + 512, 0);
    pe_rva(&mut file, 0x1140, 0x1800, 4);
    pe_text(&mut file, 0x1800, &[0xc3]);
    let mut vm = Vm::map_image(
        Image::parse(&file, None, LoadRole::Library).unwrap(),
        &(0..18)
            .map(|command| NamedExport {
                name: b"GetGameAPI",
                command: Some(command),
                parameters: &[NativeScalar::Word; 13],
                result: NativeScalar::Word,
            })
            .collect::<Vec<_>>(),
        &[],
        std::time::Duration::from_secs(3),
    )
    .unwrap();
    let output = vm.process.base() + 0x2000;
    let callback = vm.import_callback();
    let mut code = Vec::new();
    for command in 0..18u8 {
        code.extend([0x83, 0xf9, command, 0x0f, 0x85]);
        let branch = code.len();
        code.extend([0; 4]);
        if command == 3 {
            code.extend([0x8b, 0x02, 0xc3]); // mov eax,[rdx]; ret
        } else if command == 6 {
            code.extend([0x89, 0xd0, 0xc3]); // mov eax,edx; ret
        } else if command == 9 {
            code.extend([0x0f, 0xb6, 0x02, 0xc3]); // movzx eax,byte [rdx]; ret
        } else if command == 15 {
            code.extend([0x48, 0x83, 0xec, 0x28]);
            for ordinal in [18, 36, 19] {
                code.extend([0x48, 0xba]);
                code.extend((vm.process.base() + 0x5100).to_le_bytes());
                code.extend([0xb9, ordinal, 0, 0, 0, 0x48, 0xb8]);
                code.extend(callback.to_le_bytes());
                code.extend([0xff, 0xd0]);
            }
            code.extend([0x49, 0xbb]);
            code.extend((output + 64).to_le_bytes());
            code.extend([0x41, 0x89, 0x03, 0x48, 0x83, 0xc4, 0x28, 0xc3]);
        } else if command == 16 {
            code.extend([0x48, 0x83, 0xec, 0x48]);
            for (opcode, address) in [
                ([0x48, 0xba], output),
                ([0x49, 0xb8], vm.process.base() + 0x5120),
                ([0x49, 0xb9], vm.process.base() + 0x5130),
            ] {
                code.extend(opcode);
                code.extend(address.to_le_bytes());
            }
            code.extend([0x48, 0xb8]);
            code.extend((vm.process.base() + 0x5140).to_le_bytes());
            code.extend([0x48, 0x89, 0x44, 0x24, 0x20, 0x48, 0x89, 0x44, 0x24, 0x28]);
            code.extend([0x31, 0xc0, 0x48, 0x89, 0x44, 0x24, 0x30]);
            code.extend([0xff, 0xc0, 0x48, 0x89, 0x44, 0x24, 0x38]);
            code.extend([0xb9, 25, 0, 0, 0, 0x48, 0xb8]);
            code.extend(callback.to_le_bytes());
            code.extend([0xff, 0xd0, 0x48, 0x83, 0xc4, 0x48, 0xc3]);
        } else {
            let stack = if command == 14 { 0x38 } else { 0x28 };
            code.extend([0x48, 0x83, 0xec, stack]);
            if command == 14 {
                for (from, to) in [(0x60, 0x20), (0x68, 0x28), (0x70, 0x30)] {
                    code.extend([0x48, 0x8b, 0x44, 0x24, from, 0x48, 0x89, 0x44, 0x24, to]);
                }
            }
            if matches!(command, 0 | 1) {
                code.extend([0x48, 0xba]);
                code.extend(output.to_le_bytes());
                code.extend([0x49, 0xb8]);
                code.extend((output + if command == 0 { 4 } else { 0 }).to_le_bytes());
            }
            if matches!(command, 11 | 17) {
                code.extend([0x48, 0xba]);
                code.extend(
                    (vm.process.base() + if command == 11 { 0x4000 } else { 0x5160 }).to_le_bytes(),
                );
            }
            let ordinal = match command {
                2 => 52,
                4 => 100,
                7 => 50,
                8 => 53,
                10 => renderer,
                11 => 39,
                17 => 44,
                12 => {
                    if renderer == 49 {
                        37
                    } else {
                        18
                    }
                }
                13 => {
                    if renderer == 49 {
                        47
                    } else {
                        56
                    }
                }
                14 => {
                    if renderer == 49 {
                        48
                    } else {
                        29
                    }
                }
                _ => 51,
            };
            code.extend([0xb9, ordinal, 0, 0, 0, 0x48, 0xb8]);
            code.extend(callback.to_le_bytes());
            code.extend([0xff, 0xd0]);
            if matches!(command, 0 | 1) {
                code.extend([0x48, 0xb8]);
                code.extend(output.to_le_bytes());
                code.extend([0x8b, 0x00]);
            }
            code.extend([0x48, 0x83, 0xc4, stack, 0xc3]);
        }
        let next = code.len();
        code[branch..branch + 4].copy_from_slice(&((next - branch - 4) as i32).to_le_bytes());
    }
    code.extend([0x31, 0xc0, 0xc3]);
    assert!(code.len() < 0x800);
    vm.process.memory_mut().unwrap()[0x1800..0x1800 + code.len()].copy_from_slice(&code);
    vm.process.memory_mut().unwrap()[0x4000..0x400c].copy_from_slice(b"ui/cold.tga\0");
    vm.process.memory_mut().unwrap()[0x5000..0x5010].copy_from_slice(b"models/test.md2\0");
    vm.process.memory_mut().unwrap()[0x5020..0x5030].copy_from_slice(b"models/test.md3\0");
    vm.process.memory_mut().unwrap()[0x5040..0x504b].copy_from_slice(b"tag_weapon\0");
    vm.process.memory_mut().unwrap()[0x5100..0x5300].copy_from_slice(&world_import_data());
    (
        Program::Native {
            vm: Box::new(vm),
            imports,
        },
        output,
    )
}

// One pointer-free payload is shared by the existing QVM and owned-child factories.
fn world_import_data() -> [u8; 512] {
    let mut bytes = [0; 512];
    bytes[..16].copy_from_slice(b"maps/unseen.bsp\0");
    for (at, value) in [
        (32, -2.0f32),
        (36, 0.5),
        (40, 0.5),
        (48, 2.0),
        (52, 0.5),
        (56, 0.5),
    ] {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    let refdef = &mut bytes[96..96 + qa_compat::renderer::Q3_REFDEF_BYTES];
    refdef[8..12].copy_from_slice(&64i32.to_le_bytes());
    refdef[12..16].copy_from_slice(&48i32.to_le_bytes());
    for (at, value) in [
        (16, 90.0f32),
        (20, 75.0),
        (24, 32.0),
        (36, -1.0),
        (52, -1.0),
        (68, 1.0),
    ] {
        refdef[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    bytes
}

fn host(factory: fn() -> (Program, u64), clients: usize) -> (FrameHost, Vec<ClientId>, u64) {
    host_with_startup(factory, clients, None)
}

fn host_with_startup(
    factory: fn() -> (Program, u64),
    clients: usize,
    startup: Option<Argument>,
) -> (FrameHost, Vec<ClientId>, u64) {
    host_with_server(factory, clients, startup, false)
}

fn host_with_server(
    factory: fn() -> (Program, u64),
    clients: usize,
    startup: Option<Argument>,
    server: bool,
) -> (FrameHost, Vec<ClientId>, u64) {
    host_with_initializer(factory, clients, startup, server, CallbackId(6))
}
fn host_with_initializer(
    factory: fn() -> (Program, u64),
    clients: usize,
    startup: Option<Argument>,
    server: bool,
    initializer: CallbackId,
) -> (FrameHost, Vec<ClientId>, u64) {
    let mut runtime = Runtime::load(clients, []).unwrap();
    let mut requests = Vec::new();
    let mut ids = Vec::new();
    let mut output = 0;
    for seat in 0..clients {
        let client = runtime
            .connect_local(
                SeatId::new(seat as u8).unwrap(),
                SpawnAnchor {
                    position: Vec3::default(),
                    angles: Vec3::default(),
                    entity: 0,
                    fixture_fallback: false,
                },
                ClientPolicy {
                    input: RuleSetId::Quake3,
                    client: RuleSetId::Quake3,
                    movement: RuleSetId::Quake,
                    trace: RuleSetId::QuakeWorld,
                },
                Protocol::Quake3_68,
                seat as u32,
            )
            .unwrap();
        let module = ModuleId(seat as u16 + 1);
        let anchor = runtime
            .server
            .entities
            .allocate(ThinkTime::Milliseconds(0), module, AllocationPolicy::q3(0))
            .unwrap()
            .id;
        let (program, address) = factory();
        if output != 0 {
            assert_eq!(address, output);
        }
        output = address;
        requests.push(ModuleRequest {
            q3_image: None,
            context: CallContext {
                module,
                clock: ThinkTime::Milliseconds(9000),
                server_frame: 0,
                native_rules: RuleSetId::Quake3,
                trace_rules: RuleSetId::Quake2,
                sound_policy: qa_gameplay::rules::sound_policy(RuleSetId::QuakeWorld),
                console: Context {
                    source: RuleSetId::Quake2,
                    seat: SeatId::new(seat as u8).unwrap(),
                    ..Context::default()
                },
                allocation: AllocationPolicy::q3(0),
                link_order: LinkOrder::Head,
            },
            phase: Phase::Client,
            timing_rules: RuleSetId::QuakeWorld,
            rate: TickRate::FrameDriven,
            anchor,
            program,
            entries: (0..18).collect(),
            frame: Export {
                client: Some(client),
                ..Export::clocked(CallbackId(6))
            },
            prepare: vec![],
            initialize: startup
                .into_iter()
                .map(|argument| {
                    let mut export = Export::words(initializer, &[]);
                    export.client = Some(client);
                    export.arguments[0] = argument;
                    export
                })
                .collect(),
            api: None,
            shutdown: vec![],
            instruction_budget: 10000,
            configstrings: 0,
            files: 0,
            entity_source: None,
        });
        ids.push(client);
    }
    if server {
        use Opcode::*;
        let module = ModuleId(clients as u16 + 1);
        let mut context = requests[0].context;
        context.module = module;
        context.console.source = RuleSetId::Quake3;
        context.console.side = qa_console::catalog::Scope::Server;
        let anchor = runtime
            .server
            .entities
            .allocate(ThinkTime::Milliseconds(0), module, context.allocation)
            .unwrap()
            .id;
        let mut data = vec![0; 16384];
        data[16..25].copy_from_slice(b"BASEQ3-1\0");
        data[64..80].copy_from_slice(b"models/test.md3\0");
        // This row forces native fragmentation and preserves high raw bytes.
        data[1024..1024 + 8190].fill(0xff);
        let mut operations = vec![(Enter, 64)];
        for (index, address) in [(20, 16), (32, 64), (900, 1024)] {
            operations.extend([
                (Const, index),
                (Arg, 8),
                (Const, address),
                (Arg, 12),
                (Const, -19),
                (Call, 0),
                (Pop, 0),
            ]);
        }
        operations.extend([(Const, 0), (Leave, 64)]);
        requests.push(ModuleRequest {
            q3_image: None,
            context,
            phase: Phase::Server,
            timing_rules: RuleSetId::Quake3,
            rate: TickRate::FrameDriven,
            anchor,
            program: Program::Qvm {
                vm: Box::new(service_program::program(&operations, &data)),
                imports: &qa_compat::abi::Q3_SERVER,
            },
            entries: vec![0],
            frame: Export::clocked(CallbackId(0)),
            prepare: vec![],
            initialize: vec![Export::clocked(CallbackId(0))],
            api: None,
            shutdown: vec![],
            instruction_budget: 10000,
            configstrings: 1024,
            files: 0,
            entity_source: None,
        });
        for &id in &ids {
            assert!(runtime.server.bind_client_native(
                id,
                NativeEntity {
                    module,
                    slot: id.0 as i32
                }
            ));
        }
    }
    let mut host = FrameHost::load_modules(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        requests,
    )
    .unwrap();
    for (seat, &id) in ids.iter().enumerate() {
        host.local_clients[seat] = Some(id);
    }
    (host, ids, output)
}

fn call(host: &mut FrameHost, seat: usize, command: u32, words: &[u64]) -> Result<u32, ()> {
    let module = ModuleId(seat as u16 + 1);
    host.call_module_export(
        module,
        Export {
            client: Some(ClientId(seat as u32)),
            ..Export::words(CallbackId(command), words)
        },
        ThinkTime::Milliseconds(9000),
    )
    .map_err(|_| ())?;
    Ok(
        match host.module_counts(module).unwrap().last_result.unwrap() {
            ModuleResult::Qvm(value) => value as u32,
            ModuleResult::Native(value) => value as u32,
            _ => unreachable!(),
        },
    )
}
fn word(host: &mut FrameHost, seat: usize, address: u64) -> u32 {
    call(host, seat, 3, &[address]).unwrap()
}
fn fill(host: &mut FrameHost, seat: usize, address: u64) {
    call(host, seat, 4, &[address, 0xa5, SNAPSHOT_BYTES as u64]).unwrap();
}

fn config_commands(host: &mut FrameHost, client: ClientId, rows: &[(u32, &[u8])]) -> (u32, u32) {
    let channel = &mut host
        .runtime
        .network
        .get_mut(client, Endpoint::Server)
        .unwrap()
        .channel;
    let first = channel.command_state().unwrap().queued + 1;
    for &(index, text) in rows {
        let mut sent = 0;
        loop {
            let mut bytes = [0; 1024];
            let chunk = qa_network::outputs::config_command(index, text, sent, &mut bytes).unwrap();
            channel.queue_reliable(&bytes[..chunk.length]).unwrap();
            if chunk.complete {
                break;
            }
            sent = chunk.next;
        }
    }
    let last = channel.command_state().unwrap().queued;
    let time = EventTime::from_milliseconds(100);
    loop {
        let Some(packet) = channel.prepare_output(time).unwrap() else {
            break;
        };
        host.runtime
            .loopback
            .send(Endpoint::Server, client, packet.bytes)
            .unwrap();
        channel.submitted(time).unwrap();
        if !channel.pending_fragments() {
            break;
        }
    }
    host.time = time;
    host.drain(&mut FrameResult::default());
    (first, last)
}

fn select_configs(host: &mut FrameHost, seat: usize, range: (u32, u32)) {
    for sequence in range.0..=range.1 {
        call(host, seat, SELECT_COMMAND, &[u64::from(sequence)]).unwrap();
    }
}

fn assert_game_state(host: &mut FrameHost, seat: usize, output: u64, rows: &[(u32, &[u8])]) {
    use qa_compat::configstrings::{
        Q3_GAME_STATE_BYTES, Q3_GAME_STATE_COUNT_OFFSET, Q3_GAME_STATE_DATA_OFFSET,
        Q3_GAME_STATE_ROWS,
    };
    fill(host, seat, output);
    assert_eq!(call(host, seat, GET_GAME_STATE, &[output]), Ok(0));
    let mut expected = vec![0; Q3_GAME_STATE_BYTES];
    let mut position = 1usize;
    let mut checks = vec![
        0,
        8,
        19 * 4,
        22 * 4,
        31 * 4,
        33 * 4,
        511 * 4,
        1023 * 4,
        Q3_GAME_STATE_DATA_OFFSET,
        Q3_GAME_STATE_COUNT_OFFSET - 4,
        Q3_GAME_STATE_COUNT_OFFSET,
    ];
    for index in 0..Q3_GAME_STATE_ROWS {
        let Some((_, text)) = rows.iter().find(|(row, _)| *row as usize == index) else {
            continue;
        };
        if text.is_empty() {
            continue;
        }
        expected[index * 4..index * 4 + 4].copy_from_slice(&(position as i32).to_le_bytes());
        checks.push(index * 4);
        let first = Q3_GAME_STATE_DATA_OFFSET + position;
        expected[first..first + text.len()].copy_from_slice(text);
        checks.extend((first..first + text.len() + 1).step_by(4));
        position += text.len() + 1;
    }
    expected[Q3_GAME_STATE_COUNT_OFFSET..].copy_from_slice(&(position as i32).to_le_bytes());
    for offset in checks {
        let bytes = &expected[offset..offset + 4];
        assert_eq!(
            word(host, seat, output + offset as u64),
            u32::from_le_bytes(bytes.try_into().unwrap()),
            "gameState byte {offset}"
        );
    }
    assert_eq!(
        word(host, seat, output + Q3_GAME_STATE_BYTES as u64),
        0xa5a5_a5a5
    );
}

fn accepted_client_game_state(factory: fn() -> (Program, u64)) {
    let (mut host, ids, output) = host(factory, 2);
    assert_game_state(&mut host, 0, output, &[]);
    let rows: [(u32, &[u8]); 5] = [
        (32, b"models/raw\x80%.md3"),
        (20, b"baseq3-1"),
        (0, b"\\mapname\\q3dm1"),
        (21, b"1000"),
        (1, b"\\sv_serverid\\42"),
    ];
    let accepted: [(u32, &[u8]); 5] = [
        (32, b"models/raw...md3"),
        rows[1],
        rows[2],
        rows[3],
        rows[4],
    ];
    let commands = config_commands(&mut host, ids[0], &rows);
    // Receipt retains commands; selecting them owns config mutation.
    assert_game_state(&mut host, 0, output, &[]);
    select_configs(&mut host, 0, commands);
    assert_game_state(&mut host, 0, output, &accepted);
    let commands = config_commands(&mut host, ids[1], &[(20, b"other seat")]);
    select_configs(&mut host, 1, commands);
    assert_game_state(&mut host, 1, output, &[(20, b"other seat")]);
    assert_game_state(&mut host, 0, output, &accepted);

    let revision = host.runtime.client_resources[0]
        .as_ref()
        .unwrap()
        .state
        .configs
        .config(32)
        .unwrap()
        .revision;
    let commands = config_commands(&mut host, ids[0], &[(32, rows[0].1)]);
    select_configs(&mut host, 0, commands);
    assert_eq!(
        host.runtime.client_resources[0]
            .as_ref()
            .unwrap()
            .state
            .configs
            .config(32)
            .unwrap()
            .revision,
        revision
    );
    let commands = config_commands(&mut host, ids[0], &[(20, b"replacement"), (32, b"")]);
    assert_game_state(&mut host, 0, output, &accepted);
    select_configs(&mut host, 0, commands);
    let changed: [(u32, &[u8]); 4] = [
        (0, rows[2].1),
        (1, rows[4].1),
        (20, b"replacement"),
        (21, rows[3].1),
    ];
    assert_game_state(&mut host, 0, output, &changed);
    // Packing itself preserves accepted raw bytes. Q3 wire sanitation above
    // is a separate boundary and must not be repeated by the guest getter.
    assert!(host.runtime.client_resources[0].as_mut().unwrap().apply(
        8,
        b"local\x80\xff%",
        None,
        None
    ));
    let mut with_raw = changed.to_vec();
    with_raw.push((8, b"local\x80\xff%"));
    assert_game_state(&mut host, 0, output, &with_raw);
    assert_eq!(call(&mut host, 0, GET_GAME_STATE, &[0]), Err(()));
    assert_game_state(&mut host, 1, output, &[(20, b"other seat")]);
}

fn game_state_pointer_and_aggregate_limits() {
    use qa_compat::configstrings::*;
    let (mut host, ids, output) = host(qvm, 1);
    fill(&mut host, 0, output);
    assert_eq!(call(&mut host, 0, GET_GAME_STATE, &[65000]), Err(()));
    assert_eq!(word(&mut host, 0, output), 0xa5a5_a5a5);
    assert_eq!(call(&mut host, 0, GET_GAME_STATE, &[65536]), Ok(0));
    assert_eq!(word(&mut host, 0, Q3_GAME_STATE_COUNT_OFFSET as u64), 1);
    assert_eq!(
        call(&mut host, 0, READ_BYTE, &[Q3_GAME_STATE_DATA_OFFSET as u64]),
        Ok(0)
    );

    let first = vec![b'e'; 7998];
    let second = vec![b'h'; 7999];
    let commands = config_commands(&mut host, ids[0], &[(700, &first), (701, &second)]);
    select_configs(&mut host, 0, commands);
    assert_eq!(call(&mut host, 0, GET_GAME_STATE, &[output]), Ok(0));
    assert_eq!(
        word(&mut host, 0, output + Q3_GAME_STATE_COUNT_OFFSET as u64),
        Q3_GAME_STATE_DATA_BYTES as u32
    );
    assert_eq!(word(&mut host, 0, output + 700 * 4), 1);
    assert_eq!(word(&mut host, 0, output + 701 * 4), 8000);
    assert_eq!(
        call(
            &mut host,
            0,
            READ_BYTE,
            &[output + Q3_GAME_STATE_COUNT_OFFSET as u64 - 1]
        ),
        Ok(0)
    );
    let revision = host.runtime.client_resources[0]
        .as_ref()
        .unwrap()
        .state
        .configs
        .config(700)
        .unwrap()
        .revision;
    // Retire the first native command flight through a real queued CLIENT ACK.
    assert!(host.runtime.send_client_command(
        ids[0],
        &UserCmd::default(),
        host.time,
        &mut frame_source::Sink::default()
    ));
    host.drain(&mut FrameResult::default());
    let commands = config_commands(&mut host, ids[0], &[(700, &first)]);
    select_configs(&mut host, 0, commands);
    assert_eq!(
        host.runtime.client_resources[0]
            .as_ref()
            .unwrap()
            .state
            .configs
            .config(700)
            .unwrap()
            .revision,
        revision
    );

    let commands = config_commands(&mut host, ids[0], &[(702, b"x")]);
    // The same row admission checks the complete native extent before changing
    // the accepted text or its revision. The queued command is consumed below.
    assert!(
        !host.runtime.client_resources[0]
            .as_mut()
            .unwrap()
            .apply(702, b"x", None, None)
    );
    let configs = &host.runtime.client_resources[0]
        .as_ref()
        .unwrap()
        .state
        .configs;
    assert_eq!(configs.config(702).unwrap().text, b"");
    assert_eq!(configs.config(702).unwrap().revision, 0);
    assert_eq!(configs.config(700).unwrap().revision, revision);
    assert_eq!(q3_game_state_data_count(configs, None), Ok(16000));

    // Even an invalid trusted table must fail the guest-copy boundary atomically.
    host.runtime.client_resources[0]
        .as_mut()
        .unwrap()
        .state
        .configs
        .set(702, b"x")
        .unwrap();
    fill(&mut host, 0, output);
    assert_eq!(call(&mut host, 0, GET_GAME_STATE, &[output]), Err(()));
    for offset in [0, 4096, Q3_GAME_STATE_COUNT_OFFSET] {
        assert_eq!(word(&mut host, 0, output + offset as u64), 0xa5a5_a5a5);
    }
    host.runtime.client_resources[0]
        .as_mut()
        .unwrap()
        .state
        .configs
        .set(702, b"")
        .unwrap();
    assert_eq!(
        call(&mut host, 0, SELECT_COMMAND, &[u64::from(commands.0)]),
        Err(())
    );
    assert!(host.runtime.server.clients[0].connection.is_none());
}

fn snapshot(
    host: &mut FrameHost,
    client: ClientId,
    time: i32,
    count: usize,
    queued_command: bool,
) -> u32 {
    let channel = &mut host
        .runtime
        .network
        .get_mut(client, Endpoint::Server)
        .unwrap()
        .channel;
    if queued_command {
        channel
            .queue_reliable(b"print \"snapshot command\"")
            .unwrap();
    }
    let sequence = channel.send_state().sequence;
    let mut player = [0; states::PLAYER_WORDS];
    player[0] = 1234;
    player[1] = 17.25f32.to_bits();
    player[3] = 231;
    player[13] = 0x4567;
    player[24] = 777;
    player[35] = 65534;
    player[43] = (-0.0f32).to_bits();
    player[46] = 1022;
    player[47] = 513;
    player[48] = (-13i32) as u32;
    player[80] = (-1i32) as u32;
    player[96] = 0x8123_4567;
    let entities = (0..count)
        .map(|index| {
            let mut words = [0; states::ENTITY_WORDS];
            words[0] = 0x8000_1234;
            words[1] = (index as f32 + 0.25).to_bits();
            words[11] = 7;
            words[18] = 17;
            words[29] = 63;
            words[32] = 133;
            words[38] = 0x8123_4567;
            Entity {
                number: index as u32,
                words,
            }
        })
        .collect::<Vec<_>>();
    let frame = ReceivedFrame::Quake3(Frame {
        sequence,
        time: ThinkTime::Milliseconds(i64::from(time)),
        command: 0,
        flags: 4,
        qw_peers: 0,
        qw_dead: 0,
        areas: &[0x81, 0x42],
        player: &player,
        entities: &entities,
    });
    let mut bytes = [0; 16384];
    let length = channel
        .encode_server_output(&mut bytes, |writer, channel| {
            channel.write_snapshot_candidate(writer, frame, None, 64)
        })
        .unwrap();
    channel.publish_written_snapshot(frame, None).unwrap();
    let now = EventTime::from_milliseconds(sequence as u64 * 100);
    channel
        .prepare_snapshot(sequence, None, &bytes[..length], now)
        .unwrap()
        .unwrap();
    loop {
        assert!(
            channel
                .submit_with(now, |packet| host
                    .runtime
                    .loopback
                    .send(Endpoint::Server, client, packet)
                    .is_ok())
                .unwrap()
                .is_some()
        );
        if !channel.pending_fragments() {
            break;
        }
        channel.prepare_output(now).unwrap().unwrap();
    }
    host.time = now;
    host.drain(&mut FrameResult::default());
    sequence
}

fn accepted_connection_snapshot_bytes(factory: fn() -> (Program, u64)) {
    let (mut host, ids, output) = host(factory, 2);
    assert_eq!(call(&mut host, 0, 0, &[]), Ok(0));
    assert_eq!(word(&mut host, 0, output + 4), 0);
    fill(&mut host, 0, output);
    assert_eq!(call(&mut host, 0, 2, &[0, 0]), Ok(0));
    assert_eq!(word(&mut host, 0, output), 0xa5a5_a5a5);
    let sequence = snapshot(&mut host, ids[0], -12345, 2, true);
    let other_sequence = snapshot(&mut host, ids[1], 45678, 1, false);
    host.runtime.server.clients[0].player.body.position = Vec3([999., 888., 777.]);
    assert_eq!(call(&mut host, 0, 0, &[]), Ok(sequence));
    assert_eq!(word(&mut host, 0, output + 4), (-12345i32) as u32);
    assert_eq!(call(&mut host, 1, 0, &[]), Ok(other_sequence));
    assert_eq!(word(&mut host, 1, output + 4), 45678);
    host.time = EventTime::from_milliseconds(500000);
    for (seat, &id) in ids.iter().enumerate() {
        let channel = &host
            .runtime
            .network
            .get(id, Endpoint::Client)
            .unwrap()
            .channel;
        let advanced = host.runtime.snapshots[id.0 as usize]
            .as_mut()
            .unwrap()
            .advance_presentation(channel, host.time, Default::default());
        if seat == 0 {
            // The ABI preserves signed snapshot time even when that value is
            // not a legal first running CL_SetCGameTime sample.
            assert_eq!(advanced, Err(qa_app::snapshots::ClockError::TimeRegression));
            assert!(
                host.runtime.snapshots[id.0 as usize]
                    .as_ref()
                    .unwrap()
                    .interpolation()
                    .is_none()
            );
        } else {
            advanced.unwrap();
        }
    }
    let inactive_calls = host.module_counts(ModuleId(1)).unwrap().calls;
    host.client_modules();
    assert_eq!(
        host.module_counts(ModuleId(1)).unwrap().calls,
        inactive_calls
    );
    let actual = match host
        .module_counts(ModuleId(2))
        .unwrap()
        .last_result
        .unwrap()
    {
        ModuleResult::Qvm(value) => value as u32,
        ModuleResult::Native(value) => value as u32,
        _ => unreachable!(),
    };
    assert_eq!(
        actual, 45678,
        "healthy CLIENT uses its accepted native clock"
    );
    assert_eq!(call(&mut host, 0, 1, &[]), Ok((-12345i32) as u32));
    fill(&mut host, 0, output);
    assert_eq!(call(&mut host, 0, 2, &[sequence as u64, output]), Ok(1));
    for (offset, value) in [
        (0, 4),
        // Native zero-initialized outgoing metadata matches the nonnegative echo.
        (4, sequence * 100),
        (8, (-12345i32) as u32),
        (12, 0x0000_4281),
        (44, 1234),
        (44 + 8, 231),
        (44 + 20, 17.25f32.to_bits()),
        (44 + 48, 777),
        (44 + 56, 65534),
        (44 + 108, 0x4567),
        (44 + 136, 0),
        (44 + 184, (-13i32) as u32),
        (44 + 376, u32::MAX),
        (44 + 312, 0x8123_4567),
        (44 + 444, 513),
        (44 + 448, 1022),
        (44 + 452, 0),
        (44 + 464, 0),
        (512, 2),
        (516, 0),
        (516 + 4, 7),
        (516 + 16, 0x8000_1234),
        (516 + 24, 0.25f32.to_bits()),
        (516 + 84, 0x8123_4567),
        (516 + 140, 17),
        (516 + 160, 63),
        (516 + 204, 133),
        (516 + 208, 1),
        (516 + 208 + 24, 1.25f32.to_bits()),
        (516 + 2 * 208, 0xa5a5_a5a5),
        (53764, 0xa5a5_a5a5),
        (53768, 1),
    ] {
        assert_eq!(
            word(&mut host, 0, output + offset),
            value,
            "offset {offset}"
        );
    }
    fill(&mut host, 0, output);
    assert_eq!(call(&mut host, 0, 2, &[u32::MAX as u64, 0]), Ok(0));
    assert_eq!(word(&mut host, 0, output), 0xa5a5_a5a5);
    assert_eq!(
        call(&mut host, 0, 2, &[(sequence + 1) as u64, output]),
        Err(())
    );
    assert!(host.runtime.server.clients[0].connection.is_none());
    assert!(host.runtime.server.clients[1].connection.is_some());
    assert_eq!(
        call(&mut host, 1, 2, &[other_sequence as u64, output]),
        Ok(1)
    );
    assert_eq!(word(&mut host, 1, output + 8), 45678);
}

fn complete_presentation(host: &mut FrameHost) {
    // A slot remains reserved until its retirement reaches Presentation.
    qa_app::output::dispatch(
        &mut host.runtime,
        &mut frame_source::Sink::default(),
        &mut host.console.scrollback,
        &host.local_clients,
        host.time,
        3.0,
        2.0,
    );
}

fn qvm_pointer_checks_and_retained_window() {
    let (mut host, ids, output) = host(qvm, 1);
    let sequence = snapshot(&mut host, ids[0], 2345, 1, false);
    fill(&mut host, 0, output);
    assert_eq!(call(&mut host, 0, 5, &[output, 65535]), Err(()));
    assert_eq!(word(&mut host, 0, output), 0xa5a5_a5a5);
    assert_eq!(call(&mut host, 0, 2, &[sequence as u64, 65000]), Err(()));
    assert_eq!(word(&mut host, 0, output), 0xa5a5_a5a5);
    assert_eq!(call(&mut host, 0, 5, &[65536, output + 4]), Ok(0));
    assert_eq!(word(&mut host, 0, 0), sequence);
    assert_eq!(word(&mut host, 0, output + 4), 2345);
    assert_eq!(call(&mut host, 0, 2, &[sequence as u64, 65536]), Ok(1));
    assert_eq!(word(&mut host, 0, 8), 2345);
    assert_eq!(call(&mut host, 0, 2, &[sequence as u64, 0]), Err(()));
    for _ in 0..32 {
        snapshot(&mut host, ids[0], 2345, 0, false);
    }
    assert_eq!(call(&mut host, 0, 2, &[sequence as u64, 0]), Ok(0));
    let lifetime = host.runtime.server.clients[0].entity;
    assert!(host.runtime.disconnect_client(ids[0]));
    complete_presentation(&mut host);
    let replacement = host
        .runtime
        .connect_local(
            SeatId::new(0).unwrap(),
            SpawnAnchor {
                position: Vec3::default(),
                angles: Vec3::default(),
                entity: 0,
                fixture_fallback: false,
            },
            ClientPolicy {
                input: RuleSetId::Quake3,
                client: RuleSetId::Quake3,
                movement: RuleSetId::Quake2,
                trace: RuleSetId::Quake,
            },
            Protocol::Quake3_68,
            0,
        )
        .unwrap();
    assert_eq!(replacement, ids[0]);
    assert_ne!(host.runtime.server.clients[0].entity, lifetime);
    let calls = host.module_counts(ModuleId(1)).unwrap().calls;
    assert_eq!(call(&mut host, 0, 0, &[]), Err(()));
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, calls);
    assert_eq!(call(&mut host, 0, GET_GAME_STATE, &[output]), Err(()));
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, calls);
}

fn snapshot_entity_count_is_truncated_to_the_native_public_array() {
    let (mut host, ids, output) = host(qvm, 1);
    let sequence = snapshot(&mut host, ids[0], 7777, 300, false);
    fill(&mut host, 0, output);
    assert_eq!(call(&mut host, 0, 2, &[sequence as u64, output]), Ok(1));
    assert_eq!(word(&mut host, 0, output + 512), 256);
    assert_eq!(word(&mut host, 0, output + 516 + 255 * 208), 255);
    assert_eq!(word(&mut host, 0, output + 53764), 0xa5a5_a5a5);
    assert_eq!(word(&mut host, 0, output + 53768), 0);
}

fn ui_keeps_its_queued_clock() {
    fn ui() -> (Program, u64) {
        let (mut program, output) = qvm();
        let Program::Qvm { imports, .. } = &mut program else {
            unreachable!()
        };
        *imports = &qa_compat::abi::Q3_UI;
        (program, output)
    }
    let (mut host, ids, _) = host(ui, 1);
    snapshot(&mut host, ids[0], 1234, 0, false);
    host.time = EventTime::from_milliseconds(500000);
    let channel = &host
        .runtime
        .network
        .get(ids[0], Endpoint::Client)
        .unwrap()
        .channel;
    host.runtime.snapshots[0]
        .as_mut()
        .unwrap()
        .advance_presentation(channel, host.time, Default::default())
        .unwrap();
    host.client_modules();
    assert_eq!(
        host.module_counts(ModuleId(1)).unwrap().last_result,
        Some(ModuleResult::Qvm(500000))
    );
}

fn gamestate_packets(
    host: &mut FrameHost,
    client: ClientId,
    native_client: i32,
    rows: &[(u32, &[u8])],
) -> Vec<Vec<u8>> {
    let channel = &mut host
        .runtime
        .network
        .get_mut(client, Endpoint::Server)
        .unwrap()
        .channel;
    let command_sequence = channel.command_state().unwrap().queued;
    let mut bytes = [0; 32768];
    let length = channel
        .encode_server_output(&mut bytes, |writer, _| {
            writer.write_bits(2, 8)?;
            writer.write_bits(command_sequence, 32)?;
            for &(index, text) in rows {
                writer.write_bits(3, 8)?;
                writer.write_bits(index, 16)?;
                // Incoming reference bytes exercise ReadBigString's native
                // high-byte preservation and percent conversion separately.
                for &byte in text {
                    writer.write_bits(u32::from(byte), 8)?;
                }
                writer.write_bits(0, 8)?;
            }
            let mut baseline = [0; states::ENTITY_WORDS];
            baseline[0] = 7;
            baseline[1] = 12.5f32.to_bits();
            writer.write_bits(4, 8)?;
            assert!(states::write_q3_entity(
                writer,
                7,
                &[0; states::ENTITY_WORDS],
                Some(&baseline),
                true,
            )?);
            writer.write_bits(8, 8)?;
            writer.write_bits(native_client as u32, 32)?;
            writer.write_bits(0, 32)?;
            Ok(())
        })
        .unwrap();
    let time = EventTime::from_milliseconds(100);
    channel
        .prepare_server_message(&bytes[..length], time)
        .unwrap()
        .unwrap();
    let mut packets = Vec::new();
    loop {
        packets.push(channel.pending_packet().unwrap().bytes.to_vec());
        channel.submitted(time).unwrap();
        if !channel.pending_fragments() {
            break;
        }
        channel.prepare_output(time).unwrap().unwrap();
    }
    packets
}

fn receive_gamestate_packets(host: &mut FrameHost, client: ClientId, packets: &[Vec<u8>]) {
    for packet in packets {
        host.runtime
            .loopback
            .send(Endpoint::Server, client, packet)
            .unwrap();
    }
    host.time = EventTime::from_milliseconds(100);
    host.drain(&mut FrameResult::default());
}

fn queued_gamestate_precedes_first_snapshot(factory: fn() -> (Program, u64)) {
    let (mut host, ids, output) = host(factory, 2);
    for (seat, &client) in ids.iter().enumerate() {
        let commands = host
            .runtime
            .network
            .get_mut(client, Endpoint::Client)
            .unwrap()
            .commands
            .as_mut()
            .unwrap();
        for number in 1..=seat + 3 {
            commands
                .record(UserCmd {
                    server_time_ms: number as i32,
                    ..UserCmd::default()
                })
                .unwrap();
        }
    }
    let packets = gamestate_packets(
        &mut host,
        ids[0],
        13,
        &[(20, b"BASEQ3-1"), (32, b"first model"), (200, b"\x80raw%")],
    );
    let channel = &host
        .runtime
        .network
        .get(ids[0], Endpoint::Client)
        .unwrap()
        .channel;
    assert!(channel.q3_startup().is_none());
    assert!(channel.current_snapshot().is_none());
    assert_eq!(
        host.runtime.client_resources[0]
            .as_ref()
            .unwrap()
            .state
            .configs
            .config(20)
            .unwrap()
            .text,
        b"",
    );
    receive_gamestate_packets(&mut host, ids[0], &packets);
    for (seat, &client) in ids.iter().enumerate() {
        let history = host
            .runtime
            .network
            .get(client, Endpoint::Client)
            .unwrap()
            .commands
            .as_ref()
            .unwrap()
            .history()
            .unwrap();
        assert_eq!(
            (history.number(), history.len()),
            if seat == 0 { (0, 0) } else { (4, 4) }
        );
    }
    let channel = &host
        .runtime
        .network
        .get(ids[0], Endpoint::Client)
        .unwrap()
        .channel;
    let startup = channel.q3_startup().unwrap();
    assert_eq!(startup.client_num, 13);
    assert_eq!(startup.last_executed_server_command, 0);
    assert!(channel.current_snapshot().is_none());
    assert_eq!(
        host.runtime.snapshots[0].as_ref().unwrap().native_client(),
        Some(13)
    );
    assert_eq!(call(&mut host, 0, GET_GAME_STATE, &[output]), Ok(0));
    let offset = word(&mut host, 0, output + 20 * 4);
    assert_eq!(
        word(&mut host, 0, output + 4096 + u64::from(offset)),
        word_bytes(b"BASE")
    );
    assert_eq!(
        host.runtime.client_resources[0]
            .as_ref()
            .unwrap()
            .state
            .configs
            .config(200)
            .unwrap()
            .text,
        b"\x80raw."
    );
    assert!(
        host.runtime
            .network
            .get(ids[1], Endpoint::Client)
            .unwrap()
            .channel
            .q3_startup()
            .is_none()
    );
    assert_eq!(
        host.runtime.client_resources[1]
            .as_ref()
            .unwrap()
            .state
            .configs
            .config(20)
            .unwrap()
            .text,
        b""
    );

    let packets = gamestate_packets(&mut host, ids[1], 57, &[(20, b"other seat")]);
    receive_gamestate_packets(&mut host, ids[1], &packets);
    assert_eq!(
        host.runtime.snapshots[1].as_ref().unwrap().native_client(),
        Some(57)
    );
    assert_eq!(call(&mut host, 1, GET_GAME_STATE, &[output]), Ok(0));
    let offset = word(&mut host, 1, output + 20 * 4);
    assert_eq!(
        word(&mut host, 1, output + 4096 + u64::from(offset)),
        word_bytes(b"othe")
    );
    assert_eq!(
        host.runtime.prediction[0].player.movement_rules,
        RuleSetId::Quake
    );
    assert_eq!(
        host.runtime.prediction[1].player.trace_rules,
        RuleSetId::QuakeWorld
    );
}

fn word_bytes(bytes: &[u8]) -> u32 {
    u32::from_le_bytes(bytes.try_into().unwrap())
}

fn rejected_gamestate_preserves_accepted_client_and_reuse_starts_empty() {
    let (mut host, ids, _) = host(qvm, 1);
    let packets = gamestate_packets(&mut host, ids[0], 13, &[(20, b"accepted"), (32, b"old")]);
    receive_gamestate_packets(&mut host, ids[0], &packets);
    let previous = host
        .runtime
        .network
        .get(ids[0], Endpoint::Client)
        .unwrap()
        .channel
        .q3_startup();
    let errors = host.runtime.network.command_errors;
    let packets = gamestate_packets(&mut host, ids[0], 64, &[(20, b"rejected")]);
    receive_gamestate_packets(&mut host, ids[0], &packets);
    assert_eq!(host.runtime.network.command_errors, errors + 1);
    assert_eq!(
        host.runtime
            .network
            .get(ids[0], Endpoint::Client)
            .unwrap()
            .channel
            .q3_startup(),
        previous
    );
    assert_eq!(
        host.runtime.client_resources[0]
            .as_ref()
            .unwrap()
            .state
            .configs
            .config(20)
            .unwrap()
            .text,
        b"accepted"
    );
    assert_eq!(
        host.runtime.snapshots[0].as_ref().unwrap().native_client(),
        Some(13)
    );

    let packets = gamestate_packets(&mut host, ids[0], 15, &[(20, b"replacement")]);
    receive_gamestate_packets(&mut host, ids[0], &packets);
    assert_eq!(
        host.runtime.client_resources[0]
            .as_ref()
            .unwrap()
            .state
            .configs
            .config(32)
            .unwrap()
            .text,
        b""
    );
    assert_eq!(
        host.runtime.snapshots[0].as_ref().unwrap().native_client(),
        Some(15)
    );
    assert!(host.runtime.disconnect_client(ids[0]));
    assert!(host.runtime.client_resources[0].is_none());
    assert!(host.runtime.network.get(ids[0], Endpoint::Client).is_none());
    complete_presentation(&mut host);
    assert_eq!(
        host.runtime
            .connect_local(
                SeatId::new(0).unwrap(),
                SpawnAnchor {
                    position: Vec3::default(),
                    angles: Vec3::default(),
                    entity: 0,
                    fixture_fallback: false
                },
                ClientPolicy {
                    input: RuleSetId::Quake3,
                    client: RuleSetId::Quake3,
                    movement: RuleSetId::Quake2,
                    trace: RuleSetId::Quake
                },
                Protocol::Quake3_68,
                0,
            )
            .unwrap(),
        ids[0]
    );
    assert!(
        host.runtime
            .network
            .get(ids[0], Endpoint::Client)
            .unwrap()
            .channel
            .q3_startup()
            .is_none()
    );
    assert_eq!(
        host.runtime.client_resources[0]
            .as_ref()
            .unwrap()
            .state
            .configs
            .config(20)
            .unwrap()
            .text,
        b""
    );
}

fn cgame_startup_waits_for_queued_state_and_resolves_current_arguments(
    factory: fn() -> (Program, u64),
) {
    for (argument, expected) in [
        (Argument::ClientMessageSequence, 2),
        (Argument::ClientExecutedCommand, 2),
        (Argument::ClientNativeNumber, 57),
    ] {
        let (mut host, ids, _) = host_with_startup(factory, 1, Some(argument));
        let module = ModuleId(1);
        assert_eq!(host.module_state(module), Some(State::Pending));
        host.initialize_modules(Phase::Client);
        assert_eq!(host.module_state(module), Some(State::Pending));
        assert_eq!(host.module_counts(module).unwrap().calls, 0);
        let packets = gamestate_packets(&mut host, ids[0], 64, &[(20, b"bad state")]);
        receive_gamestate_packets(&mut host, ids[0], &packets);
        host.initialize_modules(Phase::Client);
        assert_eq!(host.module_state(module), Some(State::Pending));
        assert_eq!(host.module_counts(module).unwrap().traps, 0);
        {
            let channel = &mut host
                .runtime
                .network
                .get_mut(ids[0], Endpoint::Server)
                .unwrap()
                .channel;
            channel.queue_reliable(b"print before").unwrap();
            channel.queue_reliable(b"print second").unwrap();
        }
        let packets = gamestate_packets(&mut host, ids[0], 57, &[(20, b"BASEQ3-1")]);
        receive_gamestate_packets(&mut host, ids[0], &packets);
        host.runtime
            .network
            .get_mut(ids[0], Endpoint::Client)
            .unwrap()
            .channel
            .executed_server_command(2)
            .unwrap();
        assert!(
            host.runtime
                .network
                .get(ids[0], Endpoint::Client)
                .unwrap()
                .channel
                .current_snapshot()
                .is_none()
        );
        host.initialize_modules(Phase::Client);
        assert_eq!(host.module_state(module), Some(State::Running));
        assert_eq!(host.module_counts(module).unwrap().calls, 1);
        assert_eq!(host.module_counts(module).unwrap().traps, 0);
        let actual = match host.module_counts(module).unwrap().last_result.unwrap() {
            ModuleResult::Qvm(value) => value as u64,
            ModuleResult::Native(value) => value,
            ModuleResult::QuakeC(_) => panic!("word module result"),
        };
        assert_eq!(actual, expected);
        host.initialize_modules(Phase::Client);
        assert_eq!(host.module_counts(module).unwrap().calls, 1);
    }
}

fn install_cgame_world_assets(
    host: &mut FrameHost,
    client: ClientId,
    files: &mut model_files::Fixture,
) -> asset_frames::Source {
    use qa_app::{
        model_assets::{ModelAssets, ModelRequest},
        renderer::SceneComposer,
    };
    host.runtime.vfs = std::mem::take(&mut files.vfs);
    let mut assets = qa_render::Assets::load().unwrap();
    let palette = model_files::palette(&mut assets);
    let (scene, _) =
        SceneComposer::load(qa_render::Limits::default(), &mut assets, vec![]).unwrap();
    let profile = ModelRequest::native("", RuleSetId::Quake3, None).profile();
    let mut owner = ModelAssets::new(assets, [profile], [], Default::default()).unwrap();
    owner
        .adopt_palettes(
            qa_app::render_settings::PaletteRegistrations::from_registered([
                Some(palette),
                None,
                None,
                None,
                None,
            ]),
        )
        .unwrap();
    let source = asset_frames::Source::load(owner.assets());
    host.runtime.model_assets = Some(owner);
    host.runtime.scene = Some(scene);
    host.publish_model_admission(source.cpu.admitted_assets().unwrap())
        .unwrap();
    host.bind_client_model_assets([(client, profile, None)]);
    let mut defaults = [None; 5];
    defaults[RuleSetId::Quake3 as usize] = Some(profile);
    host.configure_native_scenes(&defaults, &[[None; 5]; SeatId::COUNT], 64, 48)
        .unwrap();
    source
}

fn queued_uncached_world_is_admitted_before_cgame_map_imports(factory: fn() -> (Program, u64)) {
    use qa_compat::services::ModelLookup;
    use qa_render::scene::Command;

    let (mut host, ids, output) = host_with_initializer(
        factory,
        1,
        Some(Argument::ClientNativeNumber),
        false,
        CallbackId(15),
    );
    let client = ids[0];
    let module = ModuleId(1);
    let mut files = model_files::Fixture::new(&[("maps/unseen.bsp", model_files::world_map(8.0))]);
    let mut source = install_cgame_world_assets(&mut host, client, &mut files);
    let original = host
        .runtime
        .model_assets
        .as_ref()
        .unwrap()
        .assets()
        .counts();
    let lifetime = host.runtime.server.clients[client.0 as usize].entity;
    let physical_position = host.runtime.server.clients[client.0 as usize]
        .player
        .body
        .position;
    let packets = gamestate_packets(&mut host, client, 57, &[(0, b"\\mapname\\unseen")]);
    receive_gamestate_packets(&mut host, client, &packets);
    assert!(
        host.runtime
            .network
            .get(client, Endpoint::Client)
            .unwrap()
            .channel
            .q3_startup()
            .is_some()
    );
    assert!(
        host.runtime
            .model_assets
            .as_ref()
            .unwrap()
            .catalog()
            .loaded_world(b"maps/unseen.bsp")
            .is_none()
    );
    host.initialize_modules(Phase::Client);
    assert_eq!(host.module_state(module), Some(State::Pending));
    assert_eq!(host.module_counts(module).unwrap().calls, 0);

    source.enabled = false;
    host.runtime.vfs.take_lookup_count();
    source.step(&mut host);
    assert_eq!(host.runtime.vfs.take_lookup_count(), 0);
    assert_eq!(source.attempts, 0);
    assert_eq!(host.module_state(module), Some(State::Pending));
    assert_eq!(host.module_counts(module).unwrap().calls, 0);
    assert_eq!(host.local_worlds[0], None);
    assert_eq!(
        host.runtime
            .model_assets
            .as_ref()
            .unwrap()
            .assets()
            .counts(),
        original
    );

    source.enabled = true;
    source.step(&mut host);
    assert!(host.runtime.vfs.take_lookup_count() > 0);
    assert!(source.attempts > 0);
    assert_eq!(host.module_state(module), Some(State::Running));
    assert_eq!(host.module_counts(module).unwrap().traps, 0);
    // CG_INIT executed both native map imports before querying their inline namespace.
    assert_eq!(word(&mut host, 0, output + 64), 2);
    let loaded = host
        .runtime
        .model_assets
        .as_ref()
        .unwrap()
        .catalog()
        .loaded_world(b"maps/unseen.bsp")
        .unwrap();
    let (world, presentation) = host.local_worlds[0].unwrap();
    assert_eq!(loaded.render, world);
    let geometry = loaded.collision.unwrap();
    assert_eq!(host.runtime.geometry.model_count(geometry), Some(2));
    assert_eq!(
        host.runtime
            .model_assets
            .as_ref()
            .unwrap()
            .assets()
            .counts()
            .worlds,
        original.worlds + 1
    );
    assert_eq!(
        source.cpu.admitted_assets(),
        Some(
            host.runtime
                .model_assets
                .as_ref()
                .unwrap()
                .assets()
                .counts()
        )
    );
    assert!(
        host.runtime.collision.is_none(),
        "CLIENT admission must not install a SERVER world"
    );
    assert_eq!(
        host.runtime.server.clients[client.0 as usize].entity,
        lifetime
    );
    assert_eq!(
        host.runtime.server.clients[client.0 as usize]
            .player
            .body
            .position,
        physical_position
    );
    assert_eq!(
        (
            host.runtime.prediction[0].player.movement_rules,
            host.runtime.prediction[0].player.trace_rules
        ),
        (RuleSetId::Quake, RuleSetId::QuakeWorld)
    );

    // CM_BoxTrace uses model zero selected by CM_LoadMap. The BSP is the shared
    // authored Q1 fixture while this module explicitly uses Q2 trace rules.
    assert_eq!(call(&mut host, 0, 16, &[]), Ok(0));
    assert_eq!(word(&mut host, 0, output), 0);
    assert_eq!(word(&mut host, 0, output + 4), 0);
    let fraction = f32::from_bits(word(&mut host, 0, output + 8));
    assert!(fraction > 0.0 && fraction < 1.0);
    assert_eq!(word(&mut host, 0, output + 24), (-1.0f32).to_bits());
    assert_eq!(word(&mut host, 0, output + 48), 1);

    // R_RenderScene must resolve R_LoadWorldMap's selected world and presentation,
    // rather than drawing an empty native scene or borrowing a SERVER world.
    assert!(host.runtime.scene.as_mut().unwrap().begin_frame());
    assert_eq!(call(&mut host, 0, 17, &[]), Ok(0));
    let scene = host.runtime.scene.as_mut().unwrap();
    let packet = scene.finish_frame().unwrap();
    let views: Vec<_> = packet
        .commands()
        .iter()
        .filter_map(|command| match command {
            Command::View(view) => Some(view),
            _ => None,
        })
        .collect();
    assert_eq!(views.len(), 1);
    let surfaces = packet.surfaces(views[0].scene.surfaces);
    assert!(!surfaces.is_empty());
    assert!(
        surfaces
            .iter()
            .all(|surface| surface.world == world && surface.presentation == presentation)
    );
    assert!(scene.recycle(packet).is_ok());
    let calls = host.module_counts(module).unwrap().calls;
    host.initialize_modules(Phase::Client);
    assert_eq!(host.module_counts(module).unwrap().calls, calls);
    assert_eq!(host.module_counts(module).unwrap().traps, 0);
}

fn server_publication_reaches_cgame_through_the_ring_channel_and_queue(
    factory: fn() -> (Program, u64),
) {
    use qa_app::host::FrameSource;
    use qa_core::sys_events::{EventKind, SysEvent, SysEventQueue};
    use std::time::Duration;
    struct Source {
        time: EventTime,
        polls: usize,
        backend: asset_frames::Source,
    }
    impl FrameSource for Source {
        fn asset_preparer(&mut self) -> Option<&mut (dyn qa_app::renderer::AssetPreparer + '_)> {
            Some(&mut self.backend)
        }
        fn begin_frame(&mut self) -> Duration {
            Duration::from_nanos(self.time.0)
        }
        fn poll_events(&mut self, queue: &mut SysEventQueue) {
            self.polls += 1;
            queue
                .push(SysEvent {
                    time: self.time,
                    kind: EventKind::Time,
                })
                .unwrap();
        }
        fn wait_time(&mut self, _: Duration) -> Duration {
            self.begin_frame()
        }
        fn elapsed(&self) -> Duration {
            Duration::ZERO
        }
        fn present(&mut self) {}
    }
    let (mut host, ids, output) =
        host_with_server(factory, 1, Some(Argument::ClientNativeNumber), true);
    let client = ids[0];
    let mut files =
        model_files::Fixture::new(&[("maps/native_fixture.bsp", model_files::world_map(8.0))]);
    let backend = install_cgame_world_assets(&mut host, client, &mut files);
    let owner = host.runtime.server.clients[client.0 as usize]
        .output
        .unwrap();
    let view = host
        .console
        .cvars
        .bind(
            "mapname",
            Context {
                source: RuleSetId::Quake3,
                side: qa_console::catalog::Scope::Server,
                ..Context::default()
            },
        )
        .unwrap();
    host.console
        .cvars
        .force_write(view, "native_fixture")
        .unwrap();
    host.initialize_modules(Phase::Client);
    assert_eq!(host.module_state(ModuleId(1)), Some(State::Pending));
    let mut source = Source {
        time: EventTime::from_milliseconds(100),
        polls: 0,
        backend,
    };
    host.frame(&mut source, true);
    assert_eq!(source.polls, 2);
    assert_eq!(host.module_state(ModuleId(2)), Some(State::Running));
    assert_eq!(host.module_state(ModuleId(1)), Some(State::Pending));
    assert!(
        host.runtime
            .network
            .get(client, Endpoint::Client)
            .unwrap()
            .channel
            .q3_startup()
            .is_none()
    );
    let sequence = host.runtime.server.events.sequence_range().start;
    assert!(host.runtime.server.events.retained_for(owner, sequence));
    assert_eq!(
        host.runtime
            .server
            .events
            .counters(owner)
            .unwrap()
            .acknowledgements,
        0
    );
    let check_native_retirement = |host: &FrameHost| {
        let retained = host.runtime.server.events.retained_for(owner, sequence);
        let counters = host.runtime.server.events.counters(owner).unwrap();
        // This sole reliable gamestate retires only after an actual channel ACK,
        // even if cold world admission keeps cgame Pending for another frame.
        assert_eq!(counters.acknowledgements, u64::from(!retained));
        assert_eq!(counters.acknowledged_records, u64::from(!retained));
        assert_eq!(counters.retired_on_resync, 0);
        assert_eq!(counters.overflow, 0);
    };
    let mut frames = 1;
    while host.module_state(ModuleId(1)) == Some(State::Pending) && frames < 32 {
        check_native_retirement(&host);
        let channel = &host
            .runtime
            .network
            .get(client, Endpoint::Client)
            .unwrap()
            .channel;
        if channel.q3_startup().is_none() {
            assert!(channel.current_snapshot().is_none());
        }
        assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, 0);
        frames += 1;
        source.time = EventTime::from_milliseconds(frames as u64 * 100);
        host.frame(&mut source, true);
        assert_eq!(source.polls, frames * 2);
    }
    assert!(
        frames > 2,
        "the large state must pass through native fragments"
    );
    assert_eq!(host.module_state(ModuleId(1)), Some(State::Running));
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 0);
    let channel = &host
        .runtime
        .network
        .get(client, Endpoint::Client)
        .unwrap()
        .channel;
    let startup = channel.q3_startup().unwrap();
    assert_eq!(startup.client_num, 0);
    check_native_retirement(&host);
    use qa_compat::services::ModelLookup;
    let owner_assets = host.runtime.model_assets.as_ref().unwrap();
    assert!(
        owner_assets
            .catalog()
            .loaded_world(b"maps/native_fixture.bsp")
            .is_some()
    );
    assert!(host.local_worlds[0].is_some());
    assert_eq!(
        source.backend.cpu.admitted_assets(),
        Some(owner_assets.assets().counts())
    );
    assert!(source.backend.attempts > 0);
    let configs = &host.runtime.client_resources[0]
        .as_ref()
        .unwrap()
        .state
        .configs;
    let info = configs.config(0).unwrap().text;
    let map = qa_core::text::info_value(info, b"mapname", true).unwrap();
    assert_eq!(&info[map], b"native_fixture");
    assert_eq!(configs.config(20).unwrap().text, b"BASEQ3-1");
    assert_eq!(configs.config(32).unwrap().text, b"models/test.md3");
    assert_eq!(configs.config(900).unwrap().text, vec![b'.'; 8190]);
    let version_offset =
        1 + info.len() as u32 + 1 + configs.config(1).unwrap().text.len() as u32 + 1;
    assert_eq!(call(&mut host, 0, GET_GAME_STATE, &[output]), Ok(0));
    assert_eq!(word(&mut host, 0, output + 20 * 4), version_offset);
    frames += 1;
    source.time = EventTime::from_milliseconds(frames as u64 * 100);
    host.frame(&mut source, true);
    assert_eq!(source.polls, frames * 2);
    check_native_retirement(&host);
    assert!(!host.runtime.server.events.retained_for(owner, sequence));
    assert!(
        host.runtime
            .network
            .get(client, Endpoint::Client)
            .unwrap()
            .channel
            .current_snapshot()
            .is_some()
    );
    assert_eq!(
        host.runtime
            .server
            .events
            .counters(owner)
            .unwrap()
            .acknowledgements,
        1
    );
}

fn queued_startup_prepares_world_and_retains_unready_output(factory: fn() -> (Program, u64)) {
    use qa_app::host::FrameSource;
    use qa_core::{
        events::{FrameEvent, OutputTarget},
        sys_events::{EventKind, SysEvent, SysEventQueue},
    };
    use std::time::Duration;
    struct Source {
        time: EventTime,
        ready: bool,
        sounds: usize,
        backend: asset_frames::Source,
    }
    impl FrameSource for Source {
        fn asset_preparer(&mut self) -> Option<&mut (dyn qa_app::renderer::AssetPreparer + '_)> {
            if self.backend.enabled {
                Some(&mut self.backend)
            } else {
                None
            }
        }
        fn begin_frame(&mut self) -> Duration {
            self.time = EventTime::from_milliseconds(self.time.milliseconds() + 100);
            Duration::from_nanos(self.time.0)
        }
        fn poll_events(&mut self, queue: &mut SysEventQueue) {
            queue
                .push(SysEvent {
                    time: self.time,
                    kind: EventKind::Time,
                })
                .unwrap();
        }
        fn wait_time(&mut self, _: Duration) -> Duration {
            self.begin_frame()
        }
        fn elapsed(&self) -> Duration {
            Duration::ZERO
        }
        fn present(&mut self) {}
        fn presentation_ready(&self) -> bool {
            self.ready
        }
        fn sound(&mut self, _bank: Option<&qa_audio::Bank>, event: SoundEvent) -> bool {
            self.sounds += usize::from(event.sound == SoundId(3));
            true
        }
    }
    let (mut host, ids, _) = host_with_initializer(
        factory,
        1,
        Some(Argument::ClientNativeNumber),
        false,
        CallbackId(11),
    );
    let client = ids[0];
    let mut files = model_files::Fixture::new(&[
        ("maps/native_fixture.bsp", model_files::world_map(8.0)),
        (
            "ui/cold.tga",
            model_files::tga(2, 2, &[[70, 80, 90, 255]; 4]),
        ),
    ]);
    let mut backend = install_cgame_world_assets(&mut host, client, &mut files);
    backend.enabled = false;
    let mut source = Source {
        time: EventTime::default(),
        ready: true,
        sounds: 0,
        backend,
    };
    assert_eq!(host.registration_pending(Phase::Client), Ok(true));
    host.runtime.vfs.take_lookup_count();
    let packets = gamestate_packets(&mut host, client, 57, &[(0, b"\\mapname\\native_fixture")]);
    receive_gamestate_packets(&mut host, client, &packets);
    assert!(
        host.runtime
            .network
            .get(client, Endpoint::Client)
            .unwrap()
            .channel
            .q3_startup()
            .is_some()
    );
    host.initialize_modules(Phase::Client);
    assert_eq!(host.module_state(ModuleId(1)), Some(State::Pending));
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, 0);
    let mut reset = FrameResult::default();
    host.dispatch_output(&mut source, &mut reset);
    assert!(reset.output.presentation_resets > 0);
    assert_eq!(reset.output.unhandled_presentation_resets, 0);
    assert_eq!(host.runtime.vfs.take_lookup_count(), 0);
    assert_eq!(source.backend.attempts, 0);
    // Observe the accepted header's reset receipt before withholding the later
    // sound. Gamestate receipt alone does not complete the presentation fence.
    let sound_sequence = host
        .runtime
        .server
        .events
        .push_to(
            OutputTarget::Presentation,
            FrameEvent::Sound(SoundEvent {
                sound: SoundId(3),
                ..Default::default()
            }),
        )
        .unwrap();
    source.ready = false;
    host.frame(&mut source, true);
    assert_eq!(host.runtime.vfs.take_lookup_count(), 0);
    assert_eq!(source.backend.attempts, 0);
    assert_eq!(host.module_state(ModuleId(1)), Some(State::Pending));
    assert_eq!(source.sounds, 0);
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, 0);
    assert!(
        host.runtime
            .server
            .events
            .sequence_range()
            .contains(&sound_sequence)
    );

    source.backend.enabled = true;
    host.frame(&mut source, true);
    assert_eq!(host.registration_pending(Phase::Client), Ok(false));
    assert_eq!(host.module_state(ModuleId(1)), Some(State::Running));
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 0);
    let owner = host.runtime.model_assets.as_ref().unwrap();
    assert_eq!(
        source.backend.cpu.admitted_assets(),
        Some(owner.assets().counts())
    );
    assert!(source.backend.attempts > 0);
    use qa_compat::services::ModelLookup;
    assert!(
        owner
            .catalog()
            .loaded_world(b"maps/native_fixture.bsp")
            .is_some()
    );
    assert_eq!(
        owner
            .catalog()
            .shader_handle(b"ui/cold.tga", RuleSetId::Quake3),
        Some(1)
    );
    assert!(host.local_worlds[0].is_some());
    assert_eq!(source.sounds, 0);
    assert!(
        host.runtime
            .server
            .events
            .sequence_range()
            .contains(&sound_sequence)
    );

    source.ready = true;
    host.frame(&mut source, true);
    assert_eq!(source.sounds, 1);
    assert!(
        !host
            .runtime
            .server
            .events
            .sequence_range()
            .contains(&sound_sequence)
    );
    host.frame(&mut source, true);
    assert_eq!(source.sounds, 1);
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 0);
    assert_eq!(host.registration_pending(Phase::Client), Ok(false));
}

fn registration_wait_rejects_unreceived_state_instead_of_freezing_it() {
    use qa_app::host::FrameSource;
    use std::time::Duration;
    struct Source;
    impl FrameSource for Source {
        fn begin_frame(&mut self) -> Duration {
            panic!("timeout must not poll")
        }
        fn poll_events(&mut self, _: &mut qa_core::sys_events::SysEventQueue) {
            panic!("timeout must not poll")
        }
        fn wait_time(&mut self, _: Duration) -> Duration {
            unreachable!()
        }
        fn elapsed(&self) -> Duration {
            Duration::ZERO
        }
        fn present(&mut self) {}
    }
    let (mut host, _, _) = host_with_startup(qvm, 1, Some(Argument::ClientNativeNumber));
    assert!(
        host.register_client_modules(&mut Source, Duration::ZERO)
            .is_err()
    );
    assert_eq!(host.module_state(ModuleId(1)), Some(State::Pending));
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, 0);
}

fn model_registration_uses_shared_services(factory: fn() -> (Program, u64)) {
    use qa_app::model_assets::{ModelAssets, ModelProfile};
    use qa_compat::services::ModelLookup;
    let bytes = model_files::md2_skin(b"textures/body.tga");
    let expected = qa_formats::model::Model::parse(&bytes).unwrap().bounds;
    let mut files = model_files::Fixture::new(&[
        ("models/test.md2", bytes),
        (
            "textures/body.tga",
            model_files::tga(2, 2, &[[50, 60, 70, 255]; 4]),
        ),
    ]);
    let (mut host, _, output) = host(factory, 1);
    host.runtime.vfs = std::mem::take(&mut files.vfs);
    let profile = ModelProfile {
        source_rules: RuleSetId::Quake3,
        rules: RuleSetId::Quake3,
        source_palette: None,
        palette: None,
        image_settings: qa_render::material::resources::ImageSettings::native(RuleSetId::Quake3),
    };
    host.runtime.model_assets = Some(
        ModelAssets::new(
            qa_render::Assets::load().unwrap(),
            [profile],
            [],
            Default::default(),
        )
        .unwrap(),
    );
    let path = if output == QVM_OUTPUT {
        192
    } else {
        output + 0x3000
    };
    let handle = call(&mut host, 0, 12, &[path]).unwrap();
    assert!(handle > 0);
    files.remove("models/test.md2");
    files.remove("textures/body.tga");
    assert_eq!(call(&mut host, 0, 12, &[path]), Ok(handle));
    let catalog = host.runtime.model_assets.as_ref().unwrap().catalog();
    assert_eq!(
        catalog.model_handle(b"models/test.md2", RuleSetId::Quake3),
        Some(handle as i32)
    );
    assert!(catalog.registration("models/test.md2", profile).is_some());
    host.bind_client_model_assets([]);
    // No backend or scene profile admitted this logical registration.
    assert_eq!(call(&mut host, 0, 12, &[path]), Ok(0));
    // The two checked destinations retain native mins-then-maxs write order.
    for (mins, maxs) in [(output + 16, output + 32), (output + 48, output + 48)] {
        assert_eq!(call(&mut host, 0, 13, &[handle as u64, mins, maxs]), Ok(0));
        for axis in 0..3 {
            let mins_value = if mins == maxs {
                expected.maxs
            } else {
                expected.mins
            };
            assert_eq!(
                word(&mut host, 0, mins + axis * 4),
                mins_value.0[axis as usize].to_bits()
            );
            assert_eq!(
                word(&mut host, 0, maxs + axis * 4),
                expected.maxs.0[axis as usize].to_bits()
            );
        }
    }
    for bad in [0, u32::MAX, i32::MAX as u32] {
        assert_eq!(
            call(&mut host, 0, 13, &[bad as u64, output + 16, output + 32]),
            Ok(0)
        );
        for axis in 0..3 {
            assert_eq!(word(&mut host, 0, output + 16 + axis * 4), 0);
            assert_eq!(word(&mut host, 0, output + 32 + axis * 4), 0);
        }
    }
    assert_eq!(call(&mut host, 0, 12, &[0]), Ok(0));
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 0);
}

fn model_tags_use_shared_services(factory: fn() -> (Program, u64), returns_result: bool) {
    use qa_app::model_assets::{ModelAssets, ModelRequest};
    let mut files = model_files::Fixture::surfaces();
    let bytes = model_files::md3_surfaces();
    let model = qa_formats::model::Model::parse(&bytes).unwrap();
    let (mut host, _, output) = host(factory, 1);
    host.runtime.vfs = std::mem::take(&mut files.vfs);
    let profile = ModelRequest::native("", RuleSetId::Quake3, None).profile();
    host.runtime.model_assets = Some(
        ModelAssets::new(
            qa_render::Assets::load().unwrap(),
            [profile],
            [],
            Default::default(),
        )
        .unwrap(),
    );
    let (path, name) = if output == QVM_OUTPUT {
        (224, 240)
    } else {
        (output + 0x3020, output + 0x3040)
    };
    let handle = call(&mut host, 0, 12, &[path]).unwrap();
    assert!(handle > 0);
    let destination = output + 16;
    for frozen in [false, true] {
        if frozen {
            files.remove("models/test.md3");
            host.bind_client_model_assets([]);
        }
        for (start, end, fraction) in [
            (0, 1, 0.25f32),
            (1, 0, 0.75),
            (99, i32::MAX, 0.5),
            (0, 1, -0.25),
            (0, 1, 1.25),
        ] {
            assert_eq!(
                call(
                    &mut host,
                    0,
                    14,
                    &[
                        destination,
                        handle as u64,
                        start as u64,
                        end as u64,
                        fraction.to_bits() as u64,
                        name
                    ]
                ),
                Ok(u32::from(returns_result))
            );
            let expected = model.tags[(start as usize).min(1)]
                .orientation
                .interpolate(model.tags[(end as usize).min(1)].orientation, fraction);
            for (index, value) in expected
                .origin
                .0
                .into_iter()
                .chain(expected.axes.into_iter().flat_map(|axis| axis.0))
                .enumerate()
            {
                assert_eq!(
                    word(&mut host, 0, destination + index as u64 * 4),
                    value.to_bits()
                );
            }
        }
        for (invalid_handle, start, name) in [
            (0, 0, name),
            (u32::MAX as u64, 0, name),
            (handle as u64, u32::MAX as u64, name),
            (handle as u64, 0, path),
        ] {
            assert_eq!(
                call(
                    &mut host,
                    0,
                    14,
                    &[
                        destination,
                        invalid_handle,
                        start,
                        1,
                        0.5f32.to_bits() as u64,
                        name
                    ]
                ),
                Ok(0)
            );
            let identity = Orientation::default();
            for (index, value) in identity
                .origin
                .0
                .into_iter()
                .chain(identity.axes.into_iter().flat_map(|axis| axis.0))
                .enumerate()
            {
                assert_eq!(
                    word(&mut host, 0, destination + index as u64 * 4),
                    value.to_bits()
                );
            }
        }
    }
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 0);
    assert_eq!(
        call(
            &mut host,
            0,
            14,
            &[0, handle as u64, 0, 1, 0.5f32.to_bits() as u64, name]
        ),
        Err(())
    );
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 1);
}

fn renderer_configuration_uses_shared_services(factory: fn() -> (Program, u64)) {
    use qa_core::presentation::{DisplayConfiguration, RendererConfiguration};
    let (mut host, _, output) = host(factory, 1);
    let mut configuration = RendererConfiguration {
        display: DisplayConfiguration {
            width: 1234,
            height: 567,
            frequency: 143,
            fullscreen: true,
            color_bits: 30,
            depth_bits: 24,
            stencil_bits: 8,
            stereo: false,
        },
        max_texture_size: 16384,
        max_active_textures: 32,
        texture_env_add: true,
        ..Default::default()
    };
    configuration.renderer.set("fixture backend").unwrap();
    configuration.vendor.set("fixture vendor").unwrap();
    configuration.version.set("fixture version").unwrap();
    configuration
        .extensions
        .set("GL_fixture_extension")
        .unwrap();
    host.runtime.renderer_configuration = Some(std::sync::Arc::new(configuration));
    fill(&mut host, 0, output);
    assert_eq!(call(&mut host, 0, 10, &[output]), Ok(0));
    assert_eq!(word(&mut host, 0, output), u32::from_le_bytes(*b"fixt"));
    assert_eq!(
        word(&mut host, 0, output + 1024),
        u32::from_le_bytes(*b"fixt")
    );
    assert_eq!(word(&mut host, 0, output + 1020), 0);
    assert_eq!(word(&mut host, 0, output + 11260), 0);
    let values = [
        16384,
        32,
        30,
        24,
        8,
        0,
        0,
        0,
        0,
        1,
        1234,
        567,
        (1234.0f32 / 567.0).to_bits(),
        143,
        1,
        0,
        0,
    ];
    for (index, value) in values.into_iter().enumerate() {
        assert_eq!(word(&mut host, 0, output + 11264 + index as u64 * 4), value);
    }
    assert_eq!(word(&mut host, 0, output + 11332), 0xa5a5a5a5);
    host.runtime.renderer_configuration = None;
    assert_eq!(call(&mut host, 0, 10, &[output]), Err(()));
}

fn main() {
    if let Some(status) = qa_platform::native::native_child_bootstrap() {
        std::process::exit(status);
    }
    renderer_configuration_uses_shared_services(qvm);
    renderer_configuration_uses_shared_services(qvm_ui_renderer);
    model_registration_uses_shared_services(qvm);
    model_registration_uses_shared_services(qvm_ui_renderer);
    model_tags_use_shared_services(qvm, true);
    model_tags_use_shared_services(qvm_ui_renderer, false);
    queued_startup_prepares_world_and_retains_unready_output(qvm);
    registration_wait_rejects_unreceived_state_instead_of_freezing_it();
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        renderer_configuration_uses_shared_services(native);
        renderer_configuration_uses_shared_services(native_ui_renderer);
        model_registration_uses_shared_services(native);
        model_registration_uses_shared_services(native_ui_renderer);
        model_tags_use_shared_services(native, true);
        model_tags_use_shared_services(native_ui_renderer, false);
        queued_startup_prepares_world_and_retains_unready_output(native);
    }
    accepted_connection_snapshot_bytes(qvm);
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    accepted_connection_snapshot_bytes(native);
    accepted_client_game_state(qvm);
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    accepted_client_game_state(native);
    game_state_pointer_and_aggregate_limits();
    qvm_pointer_checks_and_retained_window();
    snapshot_entity_count_is_truncated_to_the_native_public_array();
    ui_keeps_its_queued_clock();
    queued_gamestate_precedes_first_snapshot(qvm);
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    queued_gamestate_precedes_first_snapshot(native);
    rejected_gamestate_preserves_accepted_client_and_reuse_starts_empty();
    cgame_startup_waits_for_queued_state_and_resolves_current_arguments(qvm);
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    cgame_startup_waits_for_queued_state_and_resolves_current_arguments(native);
    queued_uncached_world_is_admitted_before_cgame_map_imports(qvm);
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    queued_uncached_world_is_admitted_before_cgame_map_imports(native);
    server_publication_reaches_cgame_through_the_ring_channel_and_queue(qvm);
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    server_publication_reaches_cgame_through_the_ring_channel_and_queue(native);
    println!("CLIENT snapshot getter fixtures passed");
}
