#[path = "client_commands/input.rs"]
mod input;
#[path = "model_assets/support.rs"]
#[allow(dead_code)]
mod model_support;
#[path = "client_commands/music.rs"]
mod music;

#[path = "client_commands/collision.rs"]
mod collision;
#[path = "client_commands/drop_client.rs"]
mod drop_client;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "../../formats/tests/support/native_image.rs"]
#[allow(dead_code)]
mod native_image;
#[path = "client_commands/scene.rs"]
mod scene;
#[path = "client_commands/server.rs"]
mod server;
#[path = "support/service_program.rs"]
mod service_program;

use qa_app::{
    Runtime,
    client_policy::ClientPolicy,
    host::{FrameHost, FrameResult},
    map::SpawnAnchor,
    modules::{Export, ModuleRequest, ModuleResult, Phase, Program},
};
use qa_compat::{abi::Q3_CLIENT, services::CallContext};
use qa_console::{commands::Console, views::Context};
use qa_core::{loopback::Endpoint, primitives::*, sys_events::*};
use qa_formats::program::qvm::Opcode;
use qa_network::commands::packet::Protocol;
use qa_session::timing::TickRate;
use qa_world::{area::LinkOrder, entities::AllocationPolicy};
use std::time::Duration;

fn qvm() -> Program {
    use Opcode::*;
    let mut operations = vec![(Enter, 64)];
    let mut branches = Vec::new();
    for command in 0..17 {
        operations.extend([(Local, 72), (Load4, 0), (Const, command), (Eq, 0)]);
        branches.push(operations.len() - 1);
    }
    operations.extend([(Const, -1), (Leave, 64)]);
    for (command, branch) in branches.into_iter().enumerate() {
        operations[branch].1 = operations.len() as i32;
        if command == 15 {
            operations.extend([
                (Const, 240),
                (Arg, 8),
                (Const, 272),
                (Arg, 12),
                (Const, -36),
                (Call, 0),
            ]);
        } else if command == 16 {
            operations.extend([(Const, -70), (Call, 0)]);
        } else if command == 11 {
            operations.extend([(Const, -41), (Call, 0)]);
        } else if matches!(command, 12 | 13 | 14) {
            if command == 14 {
                operations.extend([(Local, 76), (Load4, 0)]);
            } else {
                operations.push((Const, 192));
            }
            operations.extend([
                (Arg, 8),
                (
                    Const,
                    if command == 14 {
                        0
                    } else {
                        64.0f32.to_bits() as i32
                    },
                ),
                (Arg, 12),
            ]);
            for (offset, value) in [(16, 1.0f32), (20, 0.5), (24, 0.25)] {
                operations.extend([(Const, value.to_bits() as i32), (Arg, offset)]);
            }
            operations.extend([(Const, if command == 13 { -86 } else { -44 }), (Call, 0)]);
        } else if command == 7 {
            operations.extend([(Const, -55), (Call, 0)]);
        } else if matches!(command, 8 | 10) {
            operations.extend([(Local, 76), (Load4, 0), (Arg, 8)]);
            if command == 8 {
                operations.push((Const, 128));
            } else {
                operations.extend([(Local, 80), (Load4, 0)]);
            }
            operations.extend([(Arg, 12), (Const, -56), (Call, 0)]);
        } else if command == 9 {
            operations.extend([(Const, 128), (Local, 76), (Load4, 0), (Add, 0), (Load4, 0)]);
        } else if command == 0 {
            operations.extend([(Local, 76), (Load4, 0), (Arg, 8), (Const, -54), (Call, 0)]);
        } else if matches!(command, 1 | 4) {
            if command == 4 {
                operations.extend([(Const, -3), (Call, 0), (Pop, 0)]);
            }
            operations.extend([(Const, -8), (Call, 0)]);
        } else {
            operations.extend([
                (Const, 128),
                (Const, 0),
                (Store4, 0),
                (Const, 132),
                (Const, 0),
                (Store4, 0),
            ]);
            if matches!(command, 2 | 6) {
                operations.extend([
                    (Local, 76),
                    (Load4, 0),
                    (Arg, 8),
                    (Const, 128),
                    (Arg, 12),
                    (Const, 8),
                    (Arg, 16),
                    (Const, -9),
                    (Call, 0),
                ]);
            } else {
                operations.extend([
                    (Const, 128),
                    (Arg, 8),
                    (Const, 8),
                    (Arg, 12),
                    (Const, -10),
                    (Call, 0),
                ]);
            }
            operations.extend([
                (Pop, 0),
                (Const, if command >= 5 { 132 } else { 128 }),
                (Load4, 0),
            ]);
        }
        operations.push((Leave, 64));
    }
    let mut data = [0; 320];
    for (axis, value) in [1.0f32, 2., 3.].into_iter().enumerate() {
        data[192 + axis * 4..196 + axis * 4].copy_from_slice(&value.to_bits().to_le_bytes());
    }
    data[240..256].copy_from_slice(b"music/intro.wav\0");
    data[272..289].copy_from_slice(b"music/repeat.wav\0");
    Program::Qvm {
        vm: Box::new(service_program::program(&operations, &data)),
        imports: &Q3_CLIENT,
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn native() -> Program {
    use native_image::*;
    use qa_compat::native::{NamedExport, Vm};
    use qa_formats::program::native::{Image, LoadRole};
    use qa_platform::native::NativeScalar;
    let mut file = pe_export_fixture(64);
    put(&mut file, 392 + 16, 4096, 4);
    file.resize(4608, 0);
    pe_rva(&mut file, 0x1140, 0x1800, 4);
    pe_text(&mut file, 0x1800, &[0xc3]);
    let image = Image::parse(&file, None, LoadRole::Library).unwrap();
    let base = image.base;
    let mut vm = Vm::map_image(
        image,
        &(0..17)
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
    let pointer = vm.import_callback();
    let buffer = base + 0x1e00;
    let mut code = Vec::new();
    for command in 0..17u8 {
        code.extend([0x83, 0xf9, command, 0x0f, 0x85]); // cmp ecx,command; jne next
        let branch = code.len();
        code.extend([0; 4]);
        code.extend([0x48, 0x83, 0xec, 0x38]);
        if command == 9 {
            code.extend([0x48, 0xb8]);
            code.extend(buffer.to_le_bytes());
            code.extend([0x8b, 0x04, 0x10]); // mov eax,[rax+rdx]
        } else {
            if matches!(command, 2 | 3 | 5 | 6) {
                code.extend([0x48, 0xb8]);
                code.extend(buffer.to_le_bytes());
                code.extend([0x48, 0xc7, 0x00, 0, 0, 0, 0]);
            }
            let ordinal = match command {
                0 => 53,
                1 | 4 => 7,
                2 | 6 => 8,
                7 => 54,
                8 | 10 => 55,
                11 => 40,
                15 => 35,
                16 => 69,
                12 | 14 => 43,
                13 => 85,
                _ => 9,
            };
            if command == 15 {
                code.extend([0x48, 0xba]);
                code.extend((base + 0x1d20).to_le_bytes());
                code.extend([0x49, 0xb8]);
                code.extend((base + 0x1d50).to_le_bytes());
            } else if matches!(command, 2 | 6) {
                code.extend([0x49, 0xb8]);
                code.extend(buffer.to_le_bytes());
                code.extend([0x41, 0xb9, 8, 0, 0, 0]);
            } else if matches!(command, 3 | 5) {
                code.extend([0x48, 0xba]);
                code.extend(buffer.to_le_bytes());
                code.extend([0x41, 0xb8, 8, 0, 0, 0]);
            } else if command == 8 {
                code.extend([0x49, 0xb8]);
                code.extend(buffer.to_le_bytes());
            } else if matches!(command, 12 | 13 | 14) {
                if command != 14 {
                    code.extend([0x48, 0xba]);
                    code.extend((base + 0x1d00).to_le_bytes());
                }
                code.extend([0x41, 0xb8]);
                code.extend(
                    if command == 14 {
                        0u32
                    } else {
                        64.0f32.to_bits()
                    }
                    .to_le_bytes(),
                );
                code.extend([0x41, 0xb9]);
                code.extend(1.0f32.to_bits().to_le_bytes());
                for (offset, value) in [(0x20, 0.5f32), (0x28, 0.25f32)] {
                    code.extend([0xc7, 0x44, 0x24, offset]);
                    code.extend(value.to_bits().to_le_bytes());
                }
            }
            let mut call = |ordinal: u8| {
                code.extend([0xb9, ordinal, 0, 0, 0, 0x48, 0xb8]);
                code.extend(pointer.to_le_bytes());
                code.extend([0xff, 0xd0]);
            };
            if command == 4 {
                call(2);
            }
            call(ordinal);
            if matches!(command, 2 | 3 | 5 | 6) {
                code.extend([0x48, 0xb8]);
                code.extend((buffer + u64::from(command >= 5) * 4).to_le_bytes());
                code.extend([0x8b, 0x00]);
            }
        }
        code.extend([0x48, 0x83, 0xc4, 0x38, 0xc3]);
        let next = code.len();
        code[branch..branch + 4].copy_from_slice(&((next - branch - 4) as i32).to_le_bytes());
    }
    code.extend([0x31, 0xc0, 0xc3]);
    assert!(code.len() < 0x500);
    vm.process.memory_mut().unwrap()[0x1800..0x1800 + code.len()].copy_from_slice(&code);
    for (axis, value) in [1.0f32, 2., 3.].into_iter().enumerate() {
        vm.process.memory_mut().unwrap()[0x1d00 + axis * 4..0x1d04 + axis * 4]
            .copy_from_slice(&value.to_bits().to_le_bytes());
    }
    vm.process.memory_mut().unwrap()[0x1d20..0x1d30].copy_from_slice(b"music/intro.wav\0");
    vm.process.memory_mut().unwrap()[0x1d50..0x1d61].copy_from_slice(b"music/repeat.wav\0");
    Program::Native {
        vm: Box::new(vm),
        imports: &Q3_CLIENT,
    }
}

fn runtime(clients: usize) -> (Runtime, Vec<ClientId>) {
    let mut runtime = Runtime::load(clients, []).unwrap();
    let ids = (0..clients)
        .map(|seat| {
            runtime
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
                    if seat == 0 { 0 } else { 13 },
                )
                .unwrap()
        })
        .collect();
    (runtime, ids)
}
fn request(
    runtime: &mut Runtime,
    id: ModuleId,
    client: ClientId,
    program: Program,
) -> ModuleRequest {
    let anchor = runtime
        .server
        .entities
        .allocate(ThinkTime::Milliseconds(0), id, AllocationPolicy::q3(0))
        .unwrap()
        .id;
    ModuleRequest {
        context: CallContext {
            module: id,
            clock: ThinkTime::Milliseconds(0),
            server_frame: 0,
            native_rules: RuleSetId::Quake3,
            trace_rules: RuleSetId::QuakeWorld,
            sound_policy: qa_gameplay::rules::sound_policy(RuleSetId::Quake2),
            console: Context {
                source: RuleSetId::Quake2,
                seat: SeatId::new(client.0 as u8).unwrap(),
                ..Context::default()
            },
            allocation: AllocationPolicy::q3(0),
            link_order: LinkOrder::Head,
        },
        phase: Phase::Client,
        timing_rules: RuleSetId::Quake3,
        rate: TickRate::FrameDriven,
        anchor,
        program,
        entries: (0..17).collect(),
        frame: Export {
            client: Some(client),
            ..Export::words(CallbackId(4), &[])
        },
        prepare: vec![],
        initialize: vec![],
        api: None,
        shutdown: vec![],
        instruction_budget: 1000,
        configstrings: 0,
        files: 0,
        entity_source: None,
    }
}
fn host(program: impl Fn() -> Program, clients: usize) -> (FrameHost, Vec<ClientId>) {
    host_capacity(program, clients, qa_core::command_history::DEFAULT_CAPACITY)
}
fn host_capacity(
    program: impl Fn() -> Program,
    clients: usize,
    capacity: usize,
) -> (FrameHost, Vec<ClientId>) {
    let (mut runtime, ids) = runtime(clients);
    let requests = ids
        .iter()
        .enumerate()
        .map(|(seat, &client)| request(&mut runtime, ModuleId(seat as u16 + 1), client, program()))
        .collect();
    let mut console = Console::new(Context::default()).unwrap();
    let history = console
        .cvars
        .bind("cl_commandHistory", Context::default())
        .unwrap();
    console.cvars.write(history, &capacity.to_string()).unwrap();
    let mut host =
        FrameHost::load_modules(console, runtime, TickRate::FrameDriven, requests).unwrap();
    for (seat, &client) in ids.iter().enumerate() {
        host.local_clients[seat] = Some(client);
    }
    (host, ids)
}
fn wire(host: &mut FrameHost, client: ClientId, texts: &[&[u8]]) {
    let connection = host
        .runtime
        .network
        .get_mut(client, Endpoint::Server)
        .unwrap();
    for text in texts {
        connection.channel.queue_reliable(text).unwrap();
    }
    loop {
        let Some(packet) = connection.channel.prepare_output(EventTime(1)).unwrap() else {
            break;
        };
        host.runtime
            .loopback
            .send(Endpoint::Server, client, packet.bytes)
            .unwrap();
        connection.channel.submitted(EventTime(1)).unwrap();
        if !connection.channel.pending_fragments() {
            break;
        }
    }
    host.time = EventTime(1);
    host.drain(&mut FrameResult::default());
}
fn probe(host: &mut FrameHost, seat: usize, command: u32, value: u64) -> u32 {
    probe_args(host, seat, command, &[value])
}
fn probe_args(host: &mut FrameHost, seat: usize, command: u32, values: &[u64]) -> u32 {
    let client = ClientId(seat as u32);
    let module = ModuleId(seat as u16 + 1);
    host.call_module_export(
        module,
        Export {
            client: Some(client),
            ..Export::words(CallbackId(command), values)
        },
        ThinkTime::Milliseconds(1),
    )
    .unwrap();
    match host.module_counts(module).unwrap().last_result.unwrap() {
        ModuleResult::Qvm(value) => value as u32,
        ModuleResult::Native(value) => value as u32,
        _ => panic!("word module"),
    }
}
fn word(text: &[u8]) -> u32 {
    u32::from_le_bytes(text.try_into().unwrap())
}
fn retained_usercmds_reach_the_selected_module(program: impl Fn() -> Program) {
    let capacity = 8193;
    let (mut host, ids) = host_capacity(program, 2, capacity);
    assert_eq!(host.runtime.network.history_capacity(), capacity);
    assert_eq!(probe(&mut host, 0, 7, 0), 0);
    // Native startup exposes zeroed commands without manufacturing recorded rows.
    assert_eq!(probe(&mut host, 0, 8, (-1i32) as u32 as u64), 1);
    for offset in (0..24).step_by(4) {
        assert_eq!(probe(&mut host, 0, 9, offset), 0);
    }
    let count = capacity + 129;
    for number in 1..=count {
        host.runtime.send_local_command(
            ids[0],
            &UserCmd {
                server_time_ms: number as i32,
                view_angles: Vec3([90., 180., 270.]),
                movement: [37., -19., 11.],
                units: qa_gameplay::rules::command_units(RuleSetId::Quake3),
                buttons: buttons::ATTACK,
                ..UserCmd::default()
            },
            EventTime::from_milliseconds(number as u64),
        );
    }
    // Back-pressure does not erase commands produced while a flight is pending.
    assert_eq!(probe(&mut host, 0, 7, 0), count as u32);
    assert_eq!(probe(&mut host, 1, 7, 0), 0);
    for number in [count, count - 100, count - capacity + 1] {
        assert_eq!(probe(&mut host, 0, 8, number as u64), 1);
        for (offset, expected) in [
            (0, number as u32),
            (4, 16384),
            (8, 32768),
            (12, 49152),
            (16, 1),
            (20, u32::from_le_bytes([0, 37, (-19i8) as u8, 11])),
        ] {
            assert_eq!(probe(&mut host, 0, 9, offset), expected);
        }
    }
    let previous = probe(&mut host, 0, 9, 0);
    let expired = (count - capacity) as u64;
    // An expired row returns false before dereferencing the caller's pointer.
    assert_eq!(probe_args(&mut host, 0, 10, &[expired, u64::MAX]), 0);
    assert_eq!(probe(&mut host, 0, 9, 0), previous);
    host.runtime.send_local_command(
        ids[1],
        &UserCmd {
            server_time_ms: 777,
            ..UserCmd::default()
        },
        EventTime(1),
    );
    assert_eq!(probe(&mut host, 1, 7, 0), 1);
    assert_eq!(probe(&mut host, 1, 8, 1), 1);
    assert_eq!(probe(&mut host, 1, 9, 0), 777);
    wire(&mut host, ids[0], &[b"map_restart"]);
    assert_eq!(probe(&mut host, 0, 0, 1), 1);
    assert_eq!(probe(&mut host, 0, 7, 0), count as u32);
    assert_eq!(probe(&mut host, 0, 8, count as u64), 1);
    for offset in (0..24).step_by(4) {
        assert_eq!(probe(&mut host, 0, 9, offset), 0);
    }
    assert_eq!(probe(&mut host, 1, 9, 0), 777);
    // Future requests reject only the connection that owns the module.
    assert!(
        host.call_module_export(
            ModuleId(1),
            Export {
                client: Some(ids[0]),
                ..Export::words(CallbackId(8), &[(count + 1) as u64])
            },
            ThinkTime::Milliseconds(1)
        )
        .is_err()
    );
    assert!(host.runtime.server.clients[0].connection.is_none());
    assert!(host.runtime.server.clients[1].connection.is_some());
}

fn native_scene_calls_use_the_session_frame(program: impl Fn() -> Program) {
    let (mut host, _) = host(program, 1);
    assert_eq!(probe(&mut host, 0, 11, 0), 0);
    assert_eq!(probe(&mut host, 0, 12, 0), 0);
    let limits = qa_render::Limits {
        lights: 2,
        ..Default::default()
    };
    host.runtime.scene = Some(
        qa_app::renderer::SceneComposer::load(limits, qa_render::Assets::load().unwrap(), vec![])
            .unwrap()
            .0,
    );
    assert!(host.runtime.scene.as_mut().unwrap().begin_frame());
    assert_eq!(probe(&mut host, 0, 12, 0), 0);
    assert_eq!(probe(&mut host, 0, 11, 0), 0);
    assert_eq!(probe(&mut host, 0, 13, 0), 0);
    assert_eq!(probe(&mut host, 0, 12, 0), 0); // bounded drop, not a module trap
    assert_eq!(probe(&mut host, 0, 14, u64::MAX), 0); // zero radius does not read origin
    let scene = host.runtime.scene.as_mut().unwrap();
    assert!(scene.frame_mut().unwrap().render_scene(
        qa_render::Refdef::default(),
        &[],
        &qa_render::Assets::load().unwrap()
    ));
    let packet = scene.finish_frame().unwrap();
    assert_eq!(packet.rejected, 1);
    let qa_render::scene::Command::View(view) = packet.commands()[1] else {
        panic!("native scene view")
    };
    assert_eq!(view.scene.lights, IndexSpan { first: 1, count: 1 });
    let light = packet.lights(view.scene.lights)[0];
    assert_eq!(light.origin, Vec3([1., 2., 3.]));
    assert_eq!(light.radius, 64.);
    assert_eq!(light.color, [1., 0.5, 0.25]);
    assert!(light.additive);
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 0);
}
fn selected_commands_use_one_session_service(program: impl Fn() -> Program) {
    let (mut host, ids) = host(program, 2);
    wire(
        &mut host,
        ids[0],
        &[
            b"print  \"hello world\"   tail // comment",
            b"quit; set sensitivity 999",
        ],
    );
    wire(
        &mut host,
        ids[1],
        &[b"chat \"other seat\"", b"cs 32 \"alpha beta\" extra"],
    );
    assert_eq!(
        host.runtime.client_resources[ids[1].0 as usize]
            .as_ref()
            .unwrap()
            .state
            .configs
            .config(32)
            .map(|row| row.text),
        Some(&b""[..])
    );
    assert_eq!(probe(&mut host, 0, 0, 1), 1);
    assert_eq!(probe(&mut host, 0, 1, 0), 3);
    assert_eq!(probe(&mut host, 0, 2, 1), word(b"hell"));
    assert_eq!(probe(&mut host, 0, 6, 1), word(b"o w\0"));
    assert_eq!(probe(&mut host, 0, 3, 0), word(b"hell"));
    assert_eq!(probe(&mut host, 0, 5, 0), word(b"o w\0"));
    assert_eq!(probe(&mut host, 0, 4, 0), 3);
    assert_eq!(probe(&mut host, 0, 2, u64::MAX), 0);
    assert_eq!(probe(&mut host, 1, 0, 1), 1);
    assert_eq!(probe(&mut host, 1, 3, 0), word(b"othe"));
    assert_eq!(probe(&mut host, 0, 2, 1), word(b"hell"));
    assert_eq!(probe(&mut host, 1, 0, 2), 1);
    assert_eq!(
        host.runtime.client_resources[ids[1].0 as usize]
            .as_ref()
            .unwrap()
            .state
            .configs
            .config(32)
            .map(|row| row.text),
        Some(&b"alpha beta extra"[..])
    );
    assert_eq!(probe(&mut host, 0, 0, 2), 1);
    host.console.execute_frame(&mut host.runtime);
    assert!(!host.runtime.quit);
    assert_ne!(
        host.console
            .cvars
            .value(host.console.cvars.find("sensitivity").unwrap()),
        999.0
    );
    assert_eq!(
        host.runtime
            .network
            .get(ids[0], Endpoint::Client)
            .unwrap()
            .channel
            .last_executed_server_command(),
        Ok(2)
    );
}
fn bcs_rescans_and_failed_cs_preserve_rows() {
    let (mut host, ids) = host(qvm, 1);
    wire(
        &mut host,
        ids[0],
        &[
            b"bcs0 0032 \"first \"",
            b"bcs1 ignored \"second \"",
            b"bcs2 different \"third\"",
        ],
    );
    assert_eq!(probe(&mut host, 0, 0, 1), 0);
    assert_eq!(probe(&mut host, 0, 0, 2), 0);
    assert_eq!(probe(&mut host, 0, 0, 3), 1);
    assert_eq!(probe(&mut host, 0, 2, 1), word(b"0032"));
    assert_eq!(
        host.runtime.client_resources[0]
            .as_ref()
            .unwrap()
            .state
            .configs
            .config(32)
            .map(|row| row.text),
        Some(&b"first second third"[..])
    );
    // A completed rescan uses native atoi-prefix semantics and one joined value.
    wire(&mut host, ids[0], &[b"cs 32junk \"kept\""]);
    assert_eq!(probe(&mut host, 0, 0, 4), 1);
    assert_eq!(
        host.runtime.client_resources[0]
            .as_ref()
            .unwrap()
            .state
            .configs
            .config(32)
            .map(|row| row.text),
        Some(&b"kept"[..])
    );
    wire(&mut host, ids[0], &[b"cs 1024 \"invalid\""]);
    assert!(
        host.call_module_export(
            ModuleId(1),
            Export {
                client: Some(ids[0]),
                ..Export::words(CallbackId(0), &[5])
            },
            ThinkTime::Milliseconds(1)
        )
        .is_err()
    );
    assert!(host.runtime.server.clients[0].connection.is_none());
}
fn selected_systeminfo_updates_only_the_native_server_id() {
    use qa_network::channel::commands::CommandContext;
    let (mut host, ids) = host(qvm, 1);
    let context = CommandContext {
        server_id: 7,
        challenge: 0x91f8_2345,
        checksum_feed: 0x6732_0981,
    };
    for endpoint in [Endpoint::Client, Endpoint::Server] {
        host.runtime
            .network
            .get_mut(ids[0], endpoint)
            .unwrap()
            .channel
            .set_command_context(context)
            .unwrap();
    }
    wire(
        &mut host,
        ids[0],
        &[b"cs 1 \"\\SV_SERVERID\\42tail\\sv_cheats\\0\""],
    );
    assert_eq!(
        host.runtime
            .network
            .get(ids[0], Endpoint::Client)
            .unwrap()
            .channel
            .command_context()
            .unwrap(),
        context,
    );
    assert_eq!(probe(&mut host, 0, 0, 1), 1);
    let channel = &host
        .runtime
        .network
        .get(ids[0], Endpoint::Client)
        .unwrap()
        .channel;
    assert_eq!(
        channel.command_context().unwrap(),
        CommandContext {
            server_id: 42,
            ..context
        },
    );
    let key = channel.command_key(None).unwrap();
    assert_eq!(key.acknowledgements.server_id, 42);
    assert_eq!(key.challenge, context.challenge);
    assert_eq!(key.checksum_feed, context.checksum_feed);
}

fn map_restart_clears_selected_notify_and_preserves_pending_server_command() {
    let (mut host, ids) = host(qvm, 2);
    let notify = host
        .runtime
        .server
        .events
        .texts
        .insert(b"selected notify")
        .unwrap();
    let notify_id = notify.id();
    let other = host
        .runtime
        .server
        .events
        .texts
        .insert(b"other notify")
        .unwrap();
    let other_id = other.id();
    let center = host
        .runtime
        .server
        .events
        .texts
        .insert(b"center stays")
        .unwrap();
    let center_id = center.id();
    host.runtime.server.clients[0].hud.notify[0] = Some(HudLine {
        text: notify,
        started_at: 0.0,
        until: 30.0,
    });
    host.runtime.server.clients[0].hud.centerprint = Some(HudLine {
        text: center,
        started_at: 0.0,
        until: 30.0,
    });
    host.runtime.server.clients[1].hud.notify[0] = Some(HudLine {
        text: other,
        started_at: 0.0,
        until: 30.0,
    });
    for &client in &ids {
        host.runtime.server.submit_command(
            client,
            UserCmd {
                movement: [12.0, 0.0, 0.0],
                duration_ms: 20,
                units: qa_gameplay::rules::command_units(RuleSetId::Quake3),
                ..UserCmd::default()
            },
            CommandExecution::PerCommand,
            None,
        );
    }
    wire(&mut host, ids[0], &[b"map_restart"]);
    assert_eq!(probe(&mut host, 0, 0, 1), 1);
    assert!(
        host.runtime.server.clients[0]
            .hud
            .notify
            .iter()
            .all(Option::is_none)
    );
    assert!(host.runtime.server.events.texts.get(notify_id).is_none());
    assert_eq!(
        host.runtime.server.events.texts.get(other_id),
        Some(&b"other notify"[..])
    );
    assert_eq!(
        host.runtime.server.events.texts.get(center_id),
        Some(&b"center stays"[..])
    );
    assert!(host.runtime.server.take_pending_command(ids[0]).is_some());
    let preserved = host.runtime.server.clients[0].command;
    assert_eq!(preserved.movement, [12.0, 0.0, 0.0]);
    assert_eq!(
        (
            preserved.duration_ms,
            preserved.duration_ns,
            preserved.server_time_ms,
            preserved.buttons
        ),
        (20, 0, 0, 0)
    );
    assert!(host.runtime.server.take_pending_command(ids[1]).is_some());
}

fn disconnect_and_slot_reuse_do_not_lend_a_new_connection() {
    let (mut host, ids) = host(qvm, 2);
    let old = host.runtime.server.clients[0].entity;
    wire(
        &mut host,
        ids[0],
        &[b"disconnect \"reason\"", b"cs 32 \"tail\""],
    );
    assert!(
        host.call_module_export(
            ModuleId(1),
            Export {
                client: Some(ids[0]),
                ..Export::words(CallbackId(0), &[1])
            },
            ThinkTime::Milliseconds(1)
        )
        .is_err()
    );
    assert!(host.runtime.server.clients[0].connection.is_some());
    assert!(
        host.runtime.snapshots[0]
            .as_ref()
            .unwrap()
            .presentation_closed()
    );
    assert!(host.runtime.network.get(ids[0], Endpoint::Client).is_some());
    assert!(host.runtime.server.clients[1].connection.is_some());
    // Audio retirement is completed by the real Presentation consumer.
    struct Sink;
    impl qa_app::host::FrameSource for Sink {
        fn begin_frame(&mut self) -> Duration {
            Duration::ZERO
        }
        fn poll_events(&mut self, _: &mut SysEventQueue) {}
        fn wait_time(&mut self, _: Duration) -> Duration {
            Duration::ZERO
        }
        fn elapsed(&self) -> Duration {
            Duration::ZERO
        }
        fn present(&mut self) {}
        fn sound(&mut self, _: SoundEvent) -> bool {
            true
        }
    }
    // Retain the transport until an ordinary frame submits its final control.
    host.frame(&mut Sink, true);
    assert!(host.runtime.server.clients[0].connection.is_none());
    assert!(host.runtime.server.clients[1].connection.is_some());
    qa_app::output::dispatch(
        &mut host.runtime,
        &mut Sink,
        &mut qa_console::presentation::Scrollback::default(),
        &host.local_clients,
        EventTime(0),
        3.0,
        2.0,
    );
    let new = host
        .runtime
        .connect_local(
            SeatId::FIRST,
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
                trace: RuleSetId::QuakeWorld,
            },
            Protocol::Quake3_68,
            0,
        )
        .unwrap();
    assert_eq!(new, ids[0]);
    assert_ne!(host.runtime.server.clients[0].entity, old);
    host.local_clients[0] = Some(new);
    wire(&mut host, new, &[b"cs 32 \"new peer\""]);
    assert!(
        host.call_module_export(
            ModuleId(1),
            Export {
                client: Some(new),
                ..Export::words(CallbackId(0), &[1])
            },
            ThinkTime::Milliseconds(1)
        )
        .is_err()
    );
    assert!(host.runtime.server.clients[0].connection.is_some());
    assert_eq!(
        host.runtime.client_resources[0]
            .as_ref()
            .unwrap()
            .state
            .configs
            .config(32)
            .map(|row| row.text),
        Some(&b""[..])
    );
    host.client_modules();
    assert_eq!(
        host.runtime.client_resources[0]
            .as_ref()
            .unwrap()
            .state
            .configs
            .config(32)
            .map(|row| row.text),
        Some(&b"new peer"[..])
    );
}

fn fallback_host(clients: usize) -> (FrameHost, Vec<ClientId>) {
    let (runtime, ids) = runtime(clients);
    let mut host = FrameHost::load(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        vec![],
    )
    .unwrap();
    for (seat, &client) in ids.iter().enumerate() {
        host.local_clients[seat] = Some(client);
    }
    (host, ids)
}

fn presented_prints(host: &FrameHost) -> Vec<(u64, PrintEvent, Vec<u8>)> {
    use qa_core::events::FrameEvent;
    let ring = &host.runtime.server.events;
    let mut batch = ring.batch(host.runtime.server.presentation).unwrap();
    let mut prints = Vec::new();
    while let Some(record) = ring.next(&mut batch) {
        if let FrameEvent::Print(print) = record.event {
            prints.push((
                record.sequence,
                print,
                ring.texts.get(print.text).unwrap().to_vec(),
            ));
        }
    }
    prints
}

#[derive(Default)]
struct PresentationSink {
    packets: usize,
}
impl qa_app::host::FrameSource for PresentationSink {
    fn begin_frame(&mut self) -> Duration {
        Duration::ZERO
    }
    fn poll_events(&mut self, _: &mut SysEventQueue) {}
    fn wait_time(&mut self, _: Duration) -> Duration {
        Duration::ZERO
    }
    fn elapsed(&self) -> Duration {
        Duration::ZERO
    }
    fn present(&mut self) {}
    fn sound(&mut self, _: SoundEvent) -> bool {
        true
    }
    fn send_packet(&mut self, _: u16, _: std::net::SocketAddr, _: &[u8]) -> bool {
        self.packets += 1;
        false
    }
}
fn dispatch_fallback(host: &mut FrameHost) -> qa_app::output::OutputCounts {
    qa_app::output::dispatch(
        &mut host.runtime,
        &mut PresentationSink::default(),
        &mut host.console.scrollback,
        &host.local_clients,
        host.time,
        3.0,
        2.0,
    )
}
fn executed(host: &FrameHost, client: ClientId) -> u32 {
    host.runtime
        .network
        .get(client, Endpoint::Client)
        .unwrap()
        .channel
        .last_executed_server_command()
        .unwrap()
}
fn queued(host: &FrameHost, client: ClientId) -> u32 {
    host.runtime
        .network
        .get(client, Endpoint::Server)
        .unwrap()
        .channel
        .command_state()
        .unwrap()
        .queued
}

fn fallback_commands_publish_once_to_the_selected_local_display_and_hud() {
    use qa_core::events::OutputTarget;
    let (mut host, ids) = fallback_host(2);
    let module = host
        .runtime
        .server
        .events
        .bind(OutputTarget::Module(ModuleId(99)))
        .unwrap();
    wire(
        &mut host,
        ids[0],
        &[
            b"print \"quit; set sensitivity 999 \x80%\" ignored",
            b"cp \"first center\" ignored",
        ],
    );
    wire(
        &mut host,
        ids[1],
        &[
            b"chat \"other\x19 seat\" ignored",
            b"cp \"second center\" ignored",
            b"print \"second console\" ignored",
        ],
    );
    let queued_before = [queued(&host, ids[0]), queued(&host, ids[1])];
    host.client_modules();
    assert_eq!((executed(&host, ids[0]), executed(&host, ids[1])), (2, 3));
    let prints = presented_prints(&host);
    assert_eq!(
        prints
            .iter()
            .map(|(_, print, text)| (print.client, print.kind, text.as_slice()))
            .collect::<Vec<_>>(),
        [
            (
                Some(ids[0]),
                PrintKind::Console,
                b"quit; set sensitivity 999 ..".as_slice()
            ),
            (Some(ids[0]), PrintKind::Center, b"first center".as_slice()),
            (Some(ids[1]), PrintKind::Chat, b"other seat\n".as_slice()),
            (Some(ids[1]), PrintKind::Center, b"second center".as_slice()),
            (
                Some(ids[1]),
                PrintKind::Console,
                b"second console".as_slice()
            ),
        ]
    );
    for (sequence, print, _) in &prints {
        let selected = print.client.unwrap();
        for &client in &ids {
            let consumer = host.runtime.server.clients[client.0 as usize]
                .output
                .unwrap();
            assert_eq!(
                host.runtime.server.events.retained_for(consumer, *sequence),
                client == selected
            );
        }
        assert!(!host.runtime.server.events.retained_for(module, *sequence));
    }
    let range = host.runtime.server.events.sequence_range();
    // A real native retransmission is decoded again but cannot reexecute or republish.
    wire(&mut host, ids[0], &[]);
    wire(&mut host, ids[1], &[]);
    host.client_modules();
    host.client_modules();
    assert_eq!(host.runtime.server.events.sequence_range(), range);
    assert_eq!(presented_prints(&host).len(), 5);
    let counts = dispatch_fallback(&mut host);
    assert_eq!(counts.stale_texts, 0);
    assert_eq!(
        [queued(&host, ids[0]), queued(&host, ids[1])],
        queued_before,
        "received text must never echo into SERVER transport"
    );
    assert!(host.runtime.server.events.is_empty());
    let texts = &host.runtime.server.events.texts;
    assert_eq!(
        texts.get(
            host.runtime.server.clients[0]
                .hud
                .centerprint
                .as_ref()
                .unwrap()
                .text
                .id()
        ),
        Some(b"first center".as_slice())
    );
    assert_eq!(
        texts.get(
            host.runtime.server.clients[1]
                .hud
                .centerprint
                .as_ref()
                .unwrap()
                .text
                .id()
        ),
        Some(b"second center".as_slice())
    );
    assert!(
        host.runtime.server.clients[0]
            .hud
            .notify
            .iter()
            .all(Option::is_none)
    );
    assert_eq!(
        texts.get(
            host.runtime.server.clients[1].hud.notify[3]
                .as_ref()
                .unwrap()
                .text
                .id()
        ),
        Some(b"other seat\n".as_slice())
    );
    assert_eq!(
        host.console.scrollback.chunks(texts).collect::<Vec<_>>(),
        [
            b"quit; set sensitivity 999 ..".as_slice(),
            b"other seat\n".as_slice(),
            b"second console".as_slice(),
        ]
    );
    let first_center = host.runtime.server.clients[0]
        .hud
        .centerprint
        .as_ref()
        .unwrap()
        .text
        .id();
    assert!(
        texts.get(first_center).is_some(),
        "HUD display lease survives ring retirement"
    );
    host.runtime.server.clients[0]
        .hud
        .clear_messages(&mut host.runtime.server.events.texts);
    assert!(host.runtime.server.events.texts.get(first_center).is_none());
    host.console.execute_frame(&mut host.runtime);
    assert!(!host.runtime.quit);
    assert_ne!(
        host.console
            .cvars
            .value(host.console.cvars.find("sensitivity").unwrap()),
        999.0
    );
    assert!(
        host.runtime
            .client_resources
            .iter()
            .flatten()
            .all(|resources| resources.unsupported_commands == 0)
    );
}

fn fallback_chat_preserves_native_limits_and_team_only_selection() {
    let (mut host, ids) = fallback_host(1);
    let team = host
        .console
        .cvars
        .bind(
            "cg_teamChatsOnly",
            Context {
                source: RuleSetId::Quake3,
                ..Context::default()
            },
        )
        .unwrap();
    host.console.cvars.force_write(team, "1").unwrap();
    wire(
        &mut host,
        ids[0],
        &[b"chat \"suppressed\"", b"tchat \"team\x19text\" ignored"],
    );
    host.client_modules();
    let prints = presented_prints(&host);
    assert_eq!(prints.len(), 1);
    assert_eq!(
        (prints[0].1.kind, prints[0].2.as_slice()),
        (PrintKind::Chat, b"teamtext\n".as_slice())
    );
    dispatch_fallback(&mut host);
    host.console.cvars.force_write(team, "0").unwrap();
    let mut text = [b'x'; 170];
    text[0] = 0x19;
    text[148] = 0x19;
    text[149] = b'z';
    let mut command = b"chat \"".to_vec();
    command.extend_from_slice(&text);
    command.extend_from_slice(b"\" ignored");
    wire(&mut host, ids[0], &[&command]);
    host.client_modules();
    let prints = presented_prints(&host);
    let mut expected = vec![b'x'; 147];
    expected.push(b'\n');
    assert_eq!(prints.len(), 1);
    assert_eq!(
        prints[0].2, expected,
        "149-byte native copy occurs before escape removal"
    );
    assert_eq!(executed(&host, ids[0]), 3);
    dispatch_fallback(&mut host);
    assert_eq!(
        host.runtime.client_resources[0]
            .as_ref()
            .unwrap()
            .unsupported_commands,
        0
    );
}

fn cgame_owns_retained_commands_while_a_ui_only_module_does_not() {
    let (mut cgame, ids) = host(qvm, 1);
    wire(&mut cgame, ids[0], &[b"print \"module owned\""]);
    cgame.client_modules();
    assert_eq!(executed(&cgame, ids[0]), 0);
    assert!(presented_prints(&cgame).is_empty());
    assert_eq!(probe(&mut cgame, 0, 0, 1), 1);
    assert_eq!(probe(&mut cgame, 0, 2, 1), word(b"modu"));
    assert!(
        presented_prints(&cgame).is_empty(),
        "GetServerCommand leaves display to cgame"
    );

    let (mut runtime, ids) = runtime(1);
    let program = Program::Qvm {
        vm: Box::new(service_program::program(
            &[(Opcode::Enter, 64), (Opcode::Const, 0), (Opcode::Leave, 64)],
            &[0; 256],
        )),
        imports: &qa_compat::abi::Q3_UI,
    };
    let request = request(&mut runtime, ModuleId(1), ids[0], program);
    let mut ui = FrameHost::load_modules(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        vec![request],
    )
    .unwrap();
    ui.local_clients[0] = Some(ids[0]);
    wire(&mut ui, ids[0], &[b"cp \"ui-only fallback\""]);
    ui.client_modules();
    assert_eq!(ui.module_counts(ModuleId(1)).unwrap().traps, 0);
    assert_eq!(executed(&ui, ids[0]), 1);
    let prints = presented_prints(&ui);
    assert_eq!(prints.len(), 1);
    assert_eq!(
        (prints[0].1.kind, prints[0].2.as_slice()),
        (PrintKind::Center, b"ui-only fallback".as_slice())
    );
    dispatch_fallback(&mut ui);
    assert_eq!(
        ui.runtime.server.events.texts.get(
            ui.runtime.server.clients[0]
                .hud
                .centerprint
                .as_ref()
                .unwrap()
                .text
                .id()
        ),
        Some(b"ui-only fallback".as_slice())
    );
}

fn bounded_fallback_print_failure_counts_once_without_disconnect_or_reexecution() {
    let (mut host, ids) = fallback_host(1);
    let mut held = Vec::new();
    while let Some(lease) = host.runtime.server.events.texts.insert(b"held") {
        held.push(lease);
    }
    let rejected = host.runtime.server.events.rejected_publications();
    wire(&mut host, ids[0], &[b"print \"one rejected publication\""]);
    host.client_modules();
    assert_eq!(
        host.runtime.server.events.rejected_publications(),
        rejected + 1
    );
    assert_eq!(executed(&host, ids[0]), 1);
    assert!(host.runtime.server.clients[0].connection.is_some());
    assert!(presented_prints(&host).is_empty());
    for lease in held {
        host.runtime.server.events.texts.release(lease);
    }
    let queued_before = queued(&host, ids[0]);
    wire(&mut host, ids[0], &[]);
    host.client_modules();
    assert_eq!(
        host.runtime.server.events.rejected_publications(),
        rejected + 1
    );
    assert!(
        presented_prints(&host).is_empty(),
        "an already executed rejected command is not replayed"
    );
    assert_eq!(queued(&host, ids[0]), queued_before);
    wire(&mut host, ids[0], &[b"print \"healthy next\""]);
    host.client_modules();
    assert_eq!(executed(&host, ids[0]), 2);
    assert!(host.runtime.server.clients[0].connection.is_some());
    assert_eq!(presented_prints(&host)[0].2, b"healthy next");
    dispatch_fallback(&mut host);
}

fn main() {
    if let Some(status) = qa_platform::native::native_child_bootstrap() {
        std::process::exit(status);
    }
    input::queued_native_input(false);
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    input::queued_native_input(true);
    music::wire_music_reaches_shared_paint();
    music::console_cd_reaches_music_and_ordered_console_responses();
    music::native_music_reaches_shared_paint(qvm, false);
    music::native_music_reaches_shared_paint(qvm, true);
    selected_commands_use_one_session_service(qvm);
    retained_usercmds_reach_the_selected_module(qvm);
    native_scene_calls_use_the_session_frame(qvm);
    scene::submissions(false, false);
    scene::submissions(false, true);
    scene::ownership();
    scene::inline_entities(false);
    scene::entities(false, false);
    scene::entities(false, true);
    collision::selection(false);
    drop_client::qvm_drop_client_is_synchronous_and_lifetime_checked();
    drop_client::drop_during_client_think_releases_the_implicit_command_batch();
    server::qvm_import_uses_module_native_clients_and_source_string_storage();
    server::broadcast_backpressure_uses_each_real_ack_and_drops_foreign_protocols();
    server::command_width_and_reconnect_boundaries_do_not_stall_later_commands();
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        music::native_music_reaches_shared_paint(native, false);
        music::native_music_reaches_shared_paint(native, true);
        selected_commands_use_one_session_service(native);
        retained_usercmds_reach_the_selected_module(native);
        native_scene_calls_use_the_session_frame(native);
        scene::submissions(true, false);
        scene::submissions(true, true);
        scene::inline_entities(true);
        scene::entities(true, false);
        scene::entities(true, true);
        collision::selection(true);
        drop_client::owned_native_drop_client_uses_the_same_nested_callback_path();
        server::owned_native_import_uses_the_same_service_and_integer_width();
    }
    bcs_rescans_and_failed_cs_preserve_rows();
    selected_systeminfo_updates_only_the_native_server_id();
    map_restart_clears_selected_notify_and_preserves_pending_server_command();
    disconnect_and_slot_reuse_do_not_lend_a_new_connection();
    fallback_commands_publish_once_to_the_selected_local_display_and_hud();
    fallback_chat_preserves_native_limits_and_team_only_selection();
    cgame_owns_retained_commands_while_a_ui_only_module_does_not();
    bounded_fallback_print_failure_counts_once_without_disconnect_or_reexecution();
    println!("CLIENT command session fixtures passed");
}
