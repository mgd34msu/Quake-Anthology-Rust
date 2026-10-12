use qa_app::{
    Runtime,
    host::{FrameHost, FrameSource},
    modules::{Argument, Export, ModuleRequest, ModuleResult, Phase, Program, State},
};
use qa_compat::{
    abi::Q3_SERVER,
    native::{NamedExport, Vm},
    services::CallContext,
};
use qa_console::{commands::Console, views::Context};
use qa_core::primitives::ThinkTime;
use qa_core::{
    events::{FrameEvent, OutputTarget},
    primitives::{CallbackId, ModuleId, RuleSetId},
    sys_events::{EventKind, EventTime, SysEvent, SysEventQueue},
};
use qa_formats::program::native::{Encoding, Image, LoadRole};
use qa_platform::native::{NativeAbi, NativeImport, NativeScalar};
use qa_session::timing::TickRate;
use qa_world::entities::AllocationPolicy;
use std::time::Duration;

#[path = "../../../formats/tests/support/native_image.rs"]
#[allow(dead_code)]
mod native_image;
use native_image::*;
mod cues;
#[path = "../elf32_q3.rs"]
mod registered_q3;
mod sound_routes;
#[path = "../support/visibility_world.rs"]
mod visibility_world;

struct Files(std::path::PathBuf);
impl Drop for Files {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn function_file(encoding: Encoding, code: &[u8]) -> Vec<u8> {
    match encoding {
        Encoding::Elf => {
            let (mut file, tags) = elf_symbol_fixture(64);
            put(&mut file, 64 + 56 + 4, 7, 4);
            file[0x13c0..0x13c0 + code.len()].copy_from_slice(code);
            // Export dllEntry, storing its System V syscall pointer in the
            // shared slot used by vmMain. Its return is void in the native ABI.
            put(&mut file, 0x1400 + 48, elf_name(b"dllEntry"), 4);
            put(&mut file, 0x1400 + 48 + 4, 0x12, 1);
            file[0x1ac0..0x1ac8].fill(0);
            file[0x1300..0x1308].copy_from_slice(&[0x48, 0x89, 0x3d, 0xb9, 0x07, 0, 0, 0xc3]);
            elf_dynamic(&mut file, 64, &tags);
            file
        }
        Encoding::Pe => {
            let mut file = pe_export_fixture(64);
            file[640..640 + code.len()].copy_from_slice(code);
            pe_rva(&mut file, 0x1100 + 20, 2, 4);
            pe_rva(&mut file, 0x1100 + 24, 3, 4);
            pe_rva(&mut file, 0x1144, 0x10c0, 4);
            pe_rva(&mut file, 0x1158, 0x11b0, 4);
            pe_rva(&mut file, 0x1164, 1, 2);
            pe_text(&mut file, 0x11b0, b"dllEntry\0");
            file[704..712].copy_from_slice(&[0x48, 0x89, 0x0d, 0xb9, 0x06, 0, 0, 0xc3]);
            file
        }
    }
}

fn function_image(encoding: Encoding, code: &[u8]) -> Image {
    let file = function_file(encoding, code);
    Image::parse(
        &file,
        (encoding == Encoding::Elf).then_some(0x2000_0000),
        LoadRole::Library,
    )
    .unwrap()
}

fn runtime_file(encoding: Encoding, import: &[u8]) -> Vec<u8> {
    // Tail-call strlen through a native ELF PLT slot or PE IAT. The address
    // must come from the file binding, not a test-injected callback pointer.
    let mut code = [0x48, 0x8d, 0x3d, 9, 7, 0, 0, 0xff, 0x25, 0xf3, 6, 0, 0];
    if encoding == Encoding::Pe {
        code[2] = 0x0d;
    }
    let mut file = function_file(encoding, &code);
    match encoding {
        Encoding::Elf => {
            file[0x1300] = 0xc3; // dllEntry has no syscall state to initialize
            let at = 0x1280 + ELF_SYMBOL_NAMES.len();
            file[at..at + import.len()].copy_from_slice(import);
            put(&mut file, 0x1400 + 3 * 24, ELF_SYMBOL_NAMES.len() as u64, 4);
            put(&mut file, 0x1400 + 3 * 24 + 4, 0x12, 1); // required undefined function
            elf_relocation(&mut file, 64, 0x1b40, 0x3ac0, 7, 3, Some(0));
            let (_, mut tags) = elf_symbol_fixture(64);
            tags.iter_mut().find(|(tag, _)| *tag == 10).unwrap().1 =
                (ELF_SYMBOL_NAMES.len() + import.len()) as u64;
            tags.extend([(1, elf_name(b"libc.so.6")), (7, 0x3b40), (8, 24), (9, 24)]);
            elf_dynamic(&mut file, 64, &tags);
        }
        Encoding::Pe => {
            file[704] = 0xc3;
            put(&mut file, 392 + 16, 4096, 4);
            file.resize(4608, 0);
            pe_directory(&mut file, 64, 1, 0x1200, 40);
            pe_rva(&mut file, 0x1200, 0x1250, 4);
            pe_rva(&mut file, 0x120c, 0x1280, 4);
            pe_rva(&mut file, 0x1210, 0x1780, 4);
            pe_rva(&mut file, 0x1250, 0x12a0, 8);
            pe_rva(&mut file, 0x1780, 0x12a0, 8);
            pe_text(&mut file, 0x1280, b"MSVCRT.dll\0");
            pe_text(&mut file, 0x12a2, import);
        }
    }
    file
}

fn print_code(encoding: Encoding, numbered: bool) -> Vec<u8> {
    // Copy the shared message to a native local buffer, then call Print with
    // its stack pointer. This exercises the same lifetime as qsrc G_Printf.
    let (reserve, local, number, argument) = match encoding {
        Encoding::Elf => (24, 0, 0xbf, 0x74),
        Encoding::Pe => (56, 32, 0xb9, 0x54),
    };
    let mut code = vec![
        0x48,
        0x83,
        0xec,
        reserve, // sub rsp,reserve (including MS shadow space)
        0x48,
        0x8b,
        0x05,
        0xf5,
        0x06,
        0,
        0, // syscall slot at entry + 0x700
        0x4c,
        0x8d,
        0x15,
        0xfe,
        0x06,
        0,
        0, // text at entry + 0x710
        0x4d,
        0x8b,
        0x1a, // mov r11,[r10]
        0x4c,
        0x89,
        0x5c,
        0x24,
        local, // mov [rsp+local],r11
        0x4d,
        0x8b,
        0x5a,
        8, // mov r11,[r10+8]
        0x4c,
        0x89,
        0x5c,
        0x24,
        local + 8,
        number,
        0,
        0,
        0,
        0,
        0x48,
        0x8d,
        argument,
        0x24,
        local, // lea rsi/rdx,[rsp+local]
        0xff,
        0xd0,
        0x48,
        0x83,
        0xc4,
        reserve,
        0xc3,
    ];
    if !numbered {
        // A native function pointer takes the string as its first argument.
        // Remove the Q3 ordinal setup and select rdi/rcx for the local buffer.
        code[35..40].fill(0x90);
        code[42] = if encoding == Encoding::Elf {
            0x7c
        } else {
            0x4c
        };
    }
    code
}

fn module(
    runtime: &mut Runtime,
    id: ModuleId,
    rules: RuleSetId,
    rate: TickRate,
    text: &[u8],
    encoding: Encoding,
    imports: &[NativeImport<'_>],
) -> ModuleRequest {
    let code = print_code(encoding, imports.is_empty());
    let name = if encoding == Encoding::Elf {
        b"vmMain".as_slice()
    } else {
        b"GetGameAPI".as_slice()
    };
    let image = function_image(encoding, &code);
    let at = (image.symbol(name).unwrap().address - image.base) as usize + 0x700;
    let mut vm = Vm::map_image(
        image,
        &[
            NamedExport {
                parameters: &[qa_platform::native::NativeScalar::Word; 13],
                result: qa_platform::native::NativeScalar::Word,
                name,
                command: None,
            },
            NamedExport {
                parameters: &[qa_platform::native::NativeScalar::Word; 1],
                result: qa_platform::native::NativeScalar::Void,
                name: b"dllEntry",
                command: None,
            },
        ],
        imports,
        Duration::from_secs(3),
    )
    .unwrap();
    let pointer = if imports.is_empty() {
        vm.import_callback()
    } else {
        vm.process
            .import_pointer(0)
            .expect("registered native function")
    };
    let memory = vm.process.memory_mut().expect("parked native backing");
    assert_eq!(&memory[at..at + 8], &[0; 8]);
    memory[at + 16..at + 16 + text.len()].copy_from_slice(text);
    let mut request = request(runtime, id, rules, rate, vm);
    request.prepare = vec![Export {
        client: None,
        callback: CallbackId(1),
        arguments: [
            Argument::Word(pointer),
            Argument::Word(0),
            Argument::Word(0),
            Argument::Word(0),
            Argument::Word(0),
            Argument::Word(0),
            Argument::Word(0),
            Argument::Word(0),
            Argument::Word(0),
        ],
    }];
    request.entries.push(1);
    request
}

fn request(
    runtime: &mut Runtime,
    id: ModuleId,
    rules: RuleSetId,
    rate: TickRate,
    vm: Vm,
) -> ModuleRequest {
    let anchor = runtime
        .server
        .entities
        .allocate(ThinkTime::Milliseconds(0), id, AllocationPolicy::EDICT)
        .unwrap()
        .id;
    ModuleRequest {
        context: CallContext {
            native_rules: rules,
            trace_rules: rules,
            sound_rules: rules,
            server_frame: 0,
            module: id,
            clock: ThinkTime::Milliseconds(0),
            console: Context {
                source: rules,
                ..Context::default()
            },
            allocation: AllocationPolicy::EDICT,
            link_order: qa_gameplay::rules::link_order(rules),
        },
        phase: Phase::Server,
        timing_rules: rules,
        rate,
        anchor,
        program: Program::Native {
            vm: Box::new(vm),
            imports: &Q3_SERVER,
        },
        entries: vec![0],
        frame: Export::clocked(CallbackId(0)),
        prepare: Vec::new(),
        initialize: Vec::new(),
        api: None,
        shutdown: Vec::new(),
        instruction_budget: 100,
        configstrings: 0,
        files: 0,
        entity_source: None,
    }
}

fn checked_exports_preserve_names_and_native_command_arguments() {
    for (encoding, name, code) in [
        (
            Encoding::Elf,
            b"vmMain".as_slice(),
            [0x48, 0x89, 0xf8, 0x48, 0x01, 0xf0, 0xc3],
        ),
        (
            Encoding::Pe,
            b"GetGameAPI".as_slice(),
            [0x48, 0x89, 0xc8, 0x48, 0x01, 0xd0, 0xc3],
        ),
    ] {
        let named = [NamedExport {
            parameters: &[qa_platform::native::NativeScalar::Word; 2],
            result: qa_platform::native::NativeScalar::Word,
            name,
            command: Some(7),
        }];
        let vm = Vm::map_image(
            function_image(encoding, &code),
            &named,
            &[],
            Duration::from_secs(3),
        )
        .unwrap();
        let mut runtime = Runtime::load(1, std::iter::empty()).unwrap();
        let request = request(
            &mut runtime,
            ModuleId(1),
            RuleSetId::Quake3,
            TickRate::fixed(50).unwrap(),
            vm,
        );
        let mut host = FrameHost::load_modules(
            Console::new(Context::default()).unwrap(),
            runtime,
            TickRate::FrameDriven,
            vec![request],
        )
        .unwrap();
        let mut source = Source {
            time: EventTime(0),
            polls: 0,
        };
        host.frame(&mut source, true);
        source.time = EventTime(50_000_000);
        host.frame(&mut source, true);
        assert_eq!(
            host.module_counts(ModuleId(1)).unwrap().last_result,
            Some(ModuleResult::Native(57))
        );
        let missing = [NamedExport {
            parameters: &[qa_platform::native::NativeScalar::Word; 2],
            result: qa_platform::native::NativeScalar::Word,
            name: b"missing",
            command: None,
        }];
        assert!(matches!(
            Vm::map_image(
                function_image(encoding, &code),
                &missing,
                &[],
                Duration::from_secs(3)
            ),
            Err(qa_compat::native::Error::Export)
        ));
        let folded = name.to_ascii_lowercase();
        assert!(matches!(
            Vm::map_image(
                function_image(encoding, &code),
                &[NamedExport {
                    parameters: &[qa_platform::native::NativeScalar::Word; 2],
                    result: qa_platform::native::NativeScalar::Word,
                    name: &folded,
                    command: None
                }],
                &[],
                Duration::from_secs(3)
            ),
            Err(qa_compat::native::Error::Export)
        ));
    }
    let file = pe_export_fixture(32);
    let image = Image::parse(&file, None, LoadRole::Library).unwrap();
    assert!(matches!(
        Vm::map_image(
            image,
            &[NamedExport {
                parameters: &[qa_platform::native::NativeScalar::Word; 2],
                result: qa_platform::native::NativeScalar::Word,
                name: b"vmMain",
                command: None
            }],
            &[],
            Duration::from_secs(3)
        ),
        Err(qa_compat::native::Error::Process(
            qa_platform::native::NativeError::Unsupported
        ))
    ));
    let mut image = function_image(Encoding::Elf, &[0xc3]);
    image
        .symbols
        .iter_mut()
        .find(|s| s.name == image.names.find(b"vmMain"))
        .unwrap()
        .kind = 10;
    assert!(matches!(
        Vm::map_image(
            image,
            &[NamedExport {
                parameters: &[qa_platform::native::NativeScalar::Word; 2],
                result: qa_platform::native::NativeScalar::Word,
                name: b"vmMain",
                command: None
            }],
            &[],
            Duration::from_secs(3)
        ),
        Err(qa_compat::native::Error::Export)
    ));
}

fn native_usercmd_uses_session_dispatch_and_native_client_identity() {
    use qa_core::primitives::{NativeEntity, PlayerTail, UserCmd};
    use qa_session::clients::Connection;
    for encoding in [Encoding::Elf, Encoding::Pe] {
        // GetUsercmd(0, stack_buffer), then return stack_buffer.serverTime.
        // Microsoft uses shadow space; both callbacks receive owned-child memory.
        let microsoft = encoding == Encoding::Pe;
        let local = if microsoft { 32 } else { 16 };
        let code = [
            0x48,
            0x83,
            0xec,
            56,
            0x48,
            0x8b,
            0x05,
            0xf5,
            0x06,
            0,
            0,
            if microsoft { 0xb9 } else { 0xbf },
            36,
            0,
            0,
            0,
            0x31,
            if microsoft { 0xd2 } else { 0xf6 },
            if microsoft { 0x4c } else { 0x48 },
            0x8d,
            if microsoft { 0x44 } else { 0x54 },
            0x24,
            local,
            0xff,
            0xd0,
            0x8b,
            0x44,
            0x24,
            local,
            0x48,
            0x83,
            0xc4,
            56,
            0xc3,
        ];
        let image = function_image(encoding, &code);
        let name = if microsoft {
            b"GetGameAPI".as_slice()
        } else {
            b"vmMain".as_slice()
        };
        let at = (image.symbol(name).unwrap().address - image.base) as usize;
        let mut runtime = Runtime::load(2, []).unwrap();
        let clients = [ModuleId(8), ModuleId(7)].map(|module| {
            runtime
                .server
                .connect(
                    Connection::Bot,
                    module,
                    PlayerTail::None,
                    Some(NativeEntity { module, slot: 0 }),
                )
                .unwrap()
        });
        assert_eq!(clients[1].0, 1);
        runtime.server.clients[clients[0].0 as usize]
            .command
            .server_time_ms = 999;
        let mut request = module(
            &mut runtime,
            ModuleId(7),
            RuleSetId::Quake3,
            TickRate::fixed(50).unwrap(),
            b"unused\0",
            encoding,
            &[],
        );
        let Program::Native { vm, .. } = &mut request.program else {
            panic!()
        };
        vm.process.memory_mut().unwrap()[at..at + code.len()].copy_from_slice(&code);
        let mut host = FrameHost::load_modules(
            Console::new(Context::default()).unwrap(),
            runtime,
            TickRate::FrameDriven,
            vec![request],
        )
        .unwrap();
        host.initialize_modules(Phase::Server);
        assert_eq!(host.module_state(ModuleId(7)), Some(State::Running));
        for time in [1234, 5432] {
            host.runtime.server.clients[clients[1].0 as usize].command = UserCmd {
                server_time_ms: time,
                ..UserCmd::default()
            };
            host.call_module_export(
                ModuleId(7),
                Export::clocked(CallbackId(0)),
                ThinkTime::Milliseconds(0),
            )
            .unwrap();
            let counts = host.module_counts(ModuleId(7)).unwrap();
            assert_eq!(counts.last_result, Some(ModuleResult::Native(time as u64)));
            assert_eq!(counts.traps, 0);
        }
        assert!(host.runtime.server.events.is_empty());
    }
}

fn native_visibility_uses_compiled_pvs_through_session_dispatch() {
    use qa_world::visibility::PvsRows;
    for encoding in [Encoding::Elf, Encoding::Pe] {
        let microsoft = encoding == Encoding::Pe;
        // The export's first argument is the import ordinal. The two vectors
        // live above Microsoft's shadow space in the owned child's stack.
        let mut code = vec![0x48, 0x83, 0xec, 56, 0xc7, 0x44, 0x24, 32];
        code.extend(1.0f32.to_bits().to_le_bytes());
        code.extend([0x48, 0xc7, 0x44, 0x24, 36, 0, 0, 0, 0, 0xc7, 0x44, 0x24, 44]);
        code.extend((-1.0f32).to_bits().to_le_bytes());
        code.extend([0x48, 0xc7, 0x44, 0x24, 48, 0, 0, 0, 0]);
        let next = code.len() + 7;
        code.extend([0x48, 0x8b, 0x05]);
        code.extend((0x700i32 - next as i32).to_le_bytes());
        code.extend([0x48, 0x8d, if microsoft { 0x54 } else { 0x74 }, 0x24, 32]);
        code.extend([
            if microsoft { 0x4c } else { 0x48 },
            0x8d,
            if microsoft { 0x44 } else { 0x54 },
            0x24,
            44,
        ]);
        code.extend([0xff, 0xd0, 0x48, 0x83, 0xc4, 56, 0xc3]);
        let image = function_image(encoding, &code);
        let name = if microsoft {
            b"GetGameAPI".as_slice()
        } else {
            b"vmMain".as_slice()
        };
        let at = (image.symbol(name).unwrap().address - image.base) as usize;
        let mut runtime = Runtime::load(1, []).unwrap();
        runtime.collision = Some(visibility_world::collision(
            &runtime.geometry,
            [Some(0), Some(1)],
            PvsRows::load(vec![Some(0); 2], vec![3]).unwrap(),
        ));
        let mut request = module(
            &mut runtime,
            ModuleId(7),
            RuleSetId::Quake3,
            TickRate::fixed(50).unwrap(),
            b"unused\0",
            encoding,
            &[],
        );
        let Program::Native { vm, .. } = &mut request.program else {
            panic!()
        };
        vm.process.memory_mut().unwrap()[at..at + code.len()].copy_from_slice(&code);
        let mut host = FrameHost::load_modules(
            Console::new(Context::default()).unwrap(),
            runtime,
            TickRate::FrameDriven,
            vec![request],
        )
        .unwrap();
        host.initialize_modules(Phase::Server);
        assert_eq!(host.module_state(ModuleId(7)), Some(State::Running));
        let live_before = host.runtime.server.entities.len();
        for (open, ordinal, expected) in [
            (false, 26, 0),
            (false, 27, 1),
            (true, 26, 1),
            (false, 26, 0),
        ] {
            host.runtime
                .collision
                .as_mut()
                .unwrap()
                .portals
                .as_mut()
                .unwrap()
                .set(0, open)
                .unwrap();
            let mut export = Export::clocked(CallbackId(0));
            export.arguments[0] = Argument::Word(ordinal);
            host.call_module_export(ModuleId(7), export, ThinkTime::Milliseconds(0))
                .unwrap();
            let counts = host.module_counts(ModuleId(7)).unwrap();
            assert_eq!(counts.last_result, Some(ModuleResult::Native(expected)));
            assert_eq!(counts.traps, 0);
        }
        assert!(host.runtime.server.events.is_empty());
        assert_eq!(host.runtime.server.entities.len(), live_before);
    }
}

fn native_area_connection_uses_shared_topology_through_session_dispatch() {
    use qa_world::visibility::PvsRows;
    for encoding in [Encoding::Elf, Encoding::Pe] {
        let mut code = vec![0x48, 0x83, 0xec, 40];
        let next = code.len() + 7;
        code.extend([0x48, 0x8b, 0x05]);
        code.extend((0x700i32 - next as i32).to_le_bytes());
        // Forward vmMain's ordinal and two native integer arguments unchanged.
        code.extend([0xff, 0xd0, 0x48, 0x83, 0xc4, 40, 0xc3]);
        let image = function_image(encoding, &code);
        let name = if encoding == Encoding::Elf {
            b"vmMain".as_slice()
        } else {
            b"GetGameAPI".as_slice()
        };
        let at = (image.symbol(name).unwrap().address - image.base) as usize;
        let mut runtime = Runtime::load(1, []).unwrap();
        runtime.collision = Some(visibility_world::collision(
            &runtime.geometry,
            [Some(0), Some(1)],
            PvsRows::load(vec![Some(0); 2], vec![3]).unwrap(),
        ));
        let mut request = module(
            &mut runtime,
            ModuleId(7),
            RuleSetId::Quake3,
            TickRate::fixed(50).unwrap(),
            b"unused\0",
            encoding,
            &[],
        );
        let Program::Native { vm, .. } = &mut request.program else {
            panic!()
        };
        let cvar_name = image.base + 0x1700;
        let memory = vm.process.memory_mut().unwrap();
        memory[at..at + code.len()].copy_from_slice(&code);
        memory[0x1700..0x1712].copy_from_slice(b"_qa_module_number\0");
        let mut host = FrameHost::load_modules(
            Console::new(Context::default()).unwrap(),
            runtime,
            TickRate::FrameDriven,
            vec![request],
        )
        .unwrap();
        host.initialize_modules(Phase::Server);
        let no_areas = host
            .console
            .cvars
            .bind(
                "cm_noAreas",
                Context {
                    source: RuleSetId::Quake3,
                    ..Context::default()
                },
            )
            .unwrap();
        let number = host
            .console
            .cvars
            .register(
                "_qa_module_number",
                Some("1"),
                0,
                Context {
                    source: RuleSetId::Quake3,
                    ..Context::default()
                },
            )
            .unwrap();
        for (text, expected) in [
            ("-7.5", -7i32),
            ("2147483647", i32::MAX),
            ("-2147483648", i32::MIN),
            ("1e3", 1),
        ] {
            host.console.cvars.force_write(number, text).unwrap();
            let mut export = Export::clocked(CallbackId(0));
            export.arguments[0] = Argument::Word(6);
            export.arguments[1] = Argument::Word(cvar_name);
            host.call_module_export(ModuleId(7), export, ThinkTime::Milliseconds(0))
                .unwrap();
            assert_eq!(
                host.module_counts(ModuleId(7)).unwrap().last_result,
                Some(ModuleResult::Native(expected as i64 as u64))
            );
            assert_eq!(host.module_counts(ModuleId(7)).unwrap().traps, 0);
        }
        let live_before = host.runtime.server.entities.len();
        for (open, ignore, first, second, expected) in [
            (false, "0", 0, 1, 0),
            (true, "0", 0, 1, 1),
            (false, "0", 0, 0, 1),
            (false, "0", 1, 1, 1),
            (false, "0", u32::MAX as u64, 0, 0),
            (false, "0.5", 0, 1, 0),
            (false, "1.5", 2, 3, 1),
        ] {
            host.runtime
                .collision
                .as_mut()
                .unwrap()
                .portals
                .as_mut()
                .unwrap()
                .set(0, open)
                .unwrap();
            host.console.cvars.force_write(no_areas, ignore).unwrap();
            let mut export = Export::clocked(CallbackId(0));
            export.arguments[0] = Argument::Word(29);
            export.arguments[1] = Argument::Word(first);
            export.arguments[2] = Argument::Word(second);
            host.call_module_export(ModuleId(7), export, ThinkTime::Milliseconds(0))
                .unwrap();
            assert_eq!(
                host.module_counts(ModuleId(7)).unwrap().last_result,
                Some(ModuleResult::Native(expected))
            );
            assert_eq!(host.module_counts(ModuleId(7)).unwrap().traps, 0);
        }
        assert_eq!(host.runtime.server.entities.len(), live_before);
        assert!(host.runtime.server.events.is_empty());
        host.console.cvars.force_write(no_areas, "0").unwrap();
        let mut export = Export::clocked(CallbackId(0));
        export.arguments[0] = Argument::Word(29);
        export.arguments[1] = Argument::Word(2);
        export.arguments[2] = Argument::Word(0);
        assert!(
            host.call_module_export(ModuleId(7), export, ThinkTime::Milliseconds(0))
                .is_err()
        );
        assert_eq!(host.module_counts(ModuleId(7)).unwrap().traps, 1);
        let mut batch = host
            .runtime
            .server
            .events
            .batch(host.runtime.server.presentation)
            .unwrap();
        let record = host.runtime.server.events.next(&mut batch).unwrap();
        let FrameEvent::Print(print) = record.event else {
            panic!()
        };
        assert_eq!(print.kind, qa_core::primitives::PrintKind::Console);
        assert!(host.runtime.server.events.texts.get(print.text).is_some());
        assert!(host.runtime.server.events.next(&mut batch).is_none());
        assert_eq!(host.module_state(ModuleId(7)), Some(State::Failed));
    }
}

fn elf_relro_protects_complete_pages_and_keeps_adjacent_pages_writable() {
    use std::os::unix::process::ExitStatusExt;
    // mov [rdi],rsi; mov rax,rsi; ret
    let code = [0x48, 0x89, 0x37, 0x48, 0x89, 0xf0, 0xc3];
    let mut image = function_image(Encoding::Elf, &code);
    let entry = image.symbol(b"vmMain").unwrap().address;
    let base = image.base;
    image.relro = Box::new([(base + 0x4180, 0xe80)]);
    let mut vm = Vm::map_image(
        image,
        &[NamedExport {
            parameters: &[qa_platform::native::NativeScalar::Word; 2],
            result: qa_platform::native::NativeScalar::Word,
            name: b"vmMain",
            command: None,
        }],
        &[],
        Duration::from_secs(3),
    )
    .unwrap();
    let entry = vm
        .process
        .bind(
            entry,
            NativeAbi::SystemV,
            &[qa_platform::native::NativeScalar::Word; 2],
            qa_platform::native::NativeScalar::Word,
        )
        .unwrap();
    let mut words = [0; 13];
    words[0] = base + 0x3f00;
    words[1] = 123;
    assert_eq!(
        vm.process
            .invoke(entry, words, |_, _, _| Err(
                qa_platform::native::NativeError::Callback
            ))
            .unwrap(),
        123
    );
    words[0] = base + 0x4100;
    let result = vm.process.invoke(entry, words, |_, _, _| {
        Err(qa_platform::native::NativeError::Callback)
    });
    assert!(
        matches!(result, Err(qa_platform::native::NativeError::Exited(status)) if status.signal() == Some(11)),
        "{result:?}"
    );
    let mut image = function_image(Encoding::Elf, &code);
    image.relro = Box::new([(image.base + 0x3000, 0x1000)]);
    assert!(matches!(
        Vm::map_image(
            image,
            &[NamedExport {
                parameters: &[qa_platform::native::NativeScalar::Word; 2],
                result: qa_platform::native::NativeScalar::Word,
                name: b"vmMain",
                command: None
            }],
            &[],
            Duration::from_secs(3)
        ),
        Err(qa_compat::native::Error::Export)
    ));
}

fn native_files_use_the_qvm_role_policy_and_vfs_loader() {
    use qa_app::modules::{Q3Spec, load_q3};
    let files =
        Files(std::env::temp_dir().join(format!("qa-native-modules-{}", std::process::id())));
    std::fs::create_dir(&files.0).unwrap();
    let mut specs = Vec::new();
    for (encoding, file_name, message, library) in [
        (
            Encoding::Elf,
            "qagame.so",
            b"ELF cold\n\0".as_slice(),
            false,
        ),
        (Encoding::Pe, "qagame.dll", b"PE cold\n\0".as_slice(), false),
        (
            Encoding::Elf,
            "runtime.so",
            b"ELF runtime\0".as_slice(),
            true,
        ),
        (
            Encoding::Pe,
            "runtime.dll",
            b"PE runtime\0".as_slice(),
            true,
        ),
    ] {
        let code = print_code(encoding, true);
        let mut file = if library {
            runtime_file(encoding, b"strlen\0")
        } else {
            function_file(encoding, &code)
        };
        match encoding {
            Encoding::Elf => {
                put(&mut file, 64 + 3 * 56, 0, 4); // no native TLS dependency
                file[0x1ad0..0x1ad0 + message.len()].copy_from_slice(message);
            }
            Encoding::Pe => {
                put(&mut file, 168, 0, 4); // no DllMain/CRT initializer
                put(&mut file, 392 + 16, 4096, 4);
                file.resize(4608, 0);
                pe_text(&mut file, 0x1190, b"vmMain\0");
                pe_text(&mut file, 0x1790, message);
            }
        }
        std::fs::write(files.0.join(file_name), &file).unwrap();
        specs.push(Q3Spec::parse(&format!("game:{file_name}")).unwrap());
    }
    let mut runtime = Runtime::load(1, std::iter::empty()).unwrap();
    runtime.vfs.mount_directory(&files.0, 0).unwrap();
    let observer = runtime
        .server
        .events
        .bind(OutputTarget::Module(ModuleId(7)))
        .unwrap();
    let mut requests = Vec::new();
    load_q3(
        &mut runtime,
        &specs,
        &mut requests,
        TickRate::fixed(50).unwrap(),
        None,
    )
    .unwrap();
    assert_eq!(requests.len(), 4);
    for request in &requests {
        assert!(matches!(request.program, Program::Native { .. }));
        assert_eq!(request.timing_rules, RuleSetId::Quake3);
        assert_eq!(request.phase, Phase::Server);
        assert_eq!(request.prepare[0].callback, CallbackId(11));
    }
    let mut host = FrameHost::load_modules(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        requests,
    )
    .unwrap();
    let mut source = Source {
        time: EventTime(0),
        polls: 0,
    };
    let mut presented_prints = host.frame(&mut source, true).output.prints;
    source.time = EventTime(50_000_000);
    presented_prints += host.frame(&mut source, true).output.prints;
    assert_eq!(presented_prints, 4);
    host.shutdown_modules();
    for id in [ModuleId(1), ModuleId(2), ModuleId(3), ModuleId(4)] {
        assert_eq!(host.module_state(id), Some(State::Stopped));
        assert_eq!(
            (
                host.module_counts(id).unwrap().calls,
                host.module_counts(id).unwrap().traps
            ),
            (4, 0)
        );
        if id.0 >= 3 {
            assert_eq!(
                host.module_counts(id).unwrap().last_result,
                Some(ModuleResult::Native(if id == ModuleId(3) {
                    11
                } else {
                    10
                }))
            );
        }
    }
    let mut batch = host.runtime.server.events.batch(observer).unwrap();
    assert!(host.runtime.server.events.next(&mut batch).is_none());
    let mut batch = host
        .runtime
        .server
        .events
        .batch(host.runtime.server.presentation)
        .unwrap();
    let mut messages = Vec::new();
    while let Some(record) = host.runtime.server.events.next(&mut batch) {
        if let FrameEvent::Print(p) = record.event {
            messages.push(
                host.runtime
                    .server
                    .events
                    .texts
                    .get(p.text)
                    .unwrap()
                    .to_vec(),
            );
        }
    }
    assert_eq!(
        messages.iter().map(Vec::as_slice).collect::<Vec<_>>(),
        [b"ELF cold\n".as_slice(), b"PE cold\n".as_slice()]
    );
}

fn runtime_binding_respects_native_library_names_and_versions() {
    for encoding in [Encoding::Elf, Encoding::Pe] {
        for (versioned, wrong_provider) in [(false, false), (true, false), (false, true)] {
            let file = runtime_file(
                encoding,
                if versioned || wrong_provider {
                    b"strlen\0"
                } else {
                    b"missing\0"
                },
            );
            let mut image = Image::parse(
                &file,
                (encoding == Encoding::Elf).then_some(0x2000_0000),
                LoadRole::Library,
            )
            .unwrap();
            if versioned {
                if encoding == Encoding::Elf {
                    image.names = qa_core::names::NameTable::load_reserved(
                        (0..image.names.len()).map(|i| {
                            image
                                .names
                                .get(qa_core::primitives::NameId(i as u32))
                                .unwrap()
                        }),
                        1,
                        5,
                    )
                    .unwrap();
                    let version = image.names.intern(b"VER_1").unwrap();
                    image.symbols[3].version = Some(qa_formats::program::native::Version {
                        name: version,
                        library: image.names.find(b"libc.so.6"),
                        weak: false,
                    });
                } else {
                    // A named provider is required, even for a supported function.
                    image.imports[0].library = image.names.find(b"Alias");
                }
            }
            if wrong_provider {
                image.names = qa_core::names::NameTable::load_reserved(
                    (0..image.names.len()).map(|i| {
                        image
                            .names
                            .get(qa_core::primitives::NameId(i as u32))
                            .unwrap()
                    }),
                    2,
                    64,
                )
                .unwrap();
                let provider = image
                    .names
                    .intern(if encoding == Encoding::Elf {
                        b"libm.so.6".as_slice()
                    } else {
                        b"api-ms-win-crt-math-l1-1-0.dll"
                    })
                    .unwrap();
                if encoding == Encoding::Elf {
                    let version = image.names.intern(b"GLIBC_2.2.5").unwrap();
                    image.symbols[3].version = Some(qa_formats::program::native::Version {
                        name: version,
                        library: Some(provider),
                        weak: false,
                    });
                } else {
                    image.imports[0].library = Some(provider);
                }
            }
            let result = Vm::map_image(
                image,
                &[NamedExport {
                    name: if encoding == Encoding::Elf {
                        b"vmMain"
                    } else {
                        b"GetGameAPI"
                    },
                    command: None,
                    parameters: &[],
                    result: NativeScalar::Word,
                }],
                &[],
                Duration::from_secs(3),
            );
            if encoding == Encoding::Pe || (!versioned && !wrong_provider) {
                let vm = result.unwrap();
                let missing: Vec<_> = vm.unresolved_imports().collect();
                assert_eq!(missing.len(), 1);
                assert_eq!(missing[0].calls, 0);
                assert_eq!(
                    missing[0].name,
                    Some(if versioned || wrong_provider {
                        b"strlen".as_slice()
                    } else {
                        b"missing"
                    })
                );
                assert_eq!(
                    missing[0].library,
                    if encoding == Encoding::Elf {
                        None
                    } else {
                        Some(if wrong_provider {
                            b"api-ms-win-crt-math-l1-1-0.dll".as_slice()
                        } else if versioned {
                            b"Alias"
                        } else if encoding == Encoding::Pe {
                            b"MSVCRT.dll".as_slice()
                        } else {
                            b"libc.so.6"
                        })
                    }
                );
                continue;
            }
            let error = result.err().expect("unsupported runtime binding");
            let qa_compat::native::Error::Binding(message) = error else {
                panic!("unexpected binding error {error:?}");
            };
            assert!(
                message.contains(if wrong_provider {
                    if encoding == Encoding::Elf {
                        "libm.so.6:strlen"
                    } else {
                        "api-ms-win-crt-math-l1-1-0.dll:strlen"
                    }
                } else if versioned {
                    if encoding == Encoding::Elf {
                        "strlen@VER_1"
                    } else {
                        "Alias:strlen"
                    }
                } else {
                    "missing"
                }),
                "{message}"
            );
        }
    }
}

fn missing_native_imports_log_once_and_preserve_session_dispatch() {
    let mut runtime = Runtime::load(4, std::iter::empty()).unwrap();
    let observer = runtime
        .server
        .events
        .bind(OutputTarget::Module(ModuleId(4)))
        .unwrap();
    let mut requests = Vec::new();
    for (encoding, id) in [(Encoding::Elf, ModuleId(1)), (Encoding::Pe, ModuleId(2))] {
        let image = Image::parse(
            &runtime_file(encoding, b"missing\0"),
            (encoding == Encoding::Elf).then_some(0x2000_0000),
            LoadRole::Library,
        )
        .unwrap();
        let vm = Vm::map_image(
            image,
            &[NamedExport {
                name: if encoding == Encoding::Elf {
                    b"vmMain"
                } else {
                    b"GetGameAPI"
                },
                command: None,
                parameters: &[],
                result: NativeScalar::Word,
            }],
            &[],
            Duration::from_secs(3),
        )
        .unwrap();
        requests.push(request(
            &mut runtime,
            id,
            RuleSetId::Quake3,
            TickRate::fixed(50).unwrap(),
            vm,
        ));
    }
    let vm = Vm::map_image(
        function_image(Encoding::Elf, &[0xb8, 77, 0, 0, 0, 0xc3]),
        &[NamedExport {
            name: b"vmMain",
            command: None,
            parameters: &[],
            result: NativeScalar::Word,
        }],
        &[],
        Duration::from_secs(3),
    )
    .unwrap();
    requests.push(request(
        &mut runtime,
        ModuleId(3),
        RuleSetId::Quake3,
        TickRate::fixed(50).unwrap(),
        vm,
    ));
    let mut host = FrameHost::load_modules(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        requests,
    )
    .unwrap();
    host.initialize_modules(Phase::Server);
    for frame in 1..4 {
        for module in [ModuleId(1), ModuleId(2), ModuleId(3)] {
            let result = host.call_module_export(
                module,
                Export::clocked(CallbackId(0)),
                ThinkTime::Milliseconds(frame * 50),
            );
            assert_eq!(result.is_ok(), module == ModuleId(3));
        }
    }
    for id in [ModuleId(1), ModuleId(2)] {
        assert_eq!(host.module_state(id), Some(State::Running));
        let counts = host.module_counts(id).unwrap();
        assert_eq!((counts.calls, counts.traps), (3, 3));
    }
    assert_eq!(host.module_state(ModuleId(3)), Some(State::Running));
    let counts = host.module_counts(ModuleId(3)).unwrap();
    assert_eq!(
        (counts.calls, counts.traps, counts.last_result),
        (3, 0, Some(ModuleResult::Native(77)))
    );
    let mut batch = host.runtime.server.events.batch(observer).unwrap();
    assert!(host.runtime.server.events.next(&mut batch).is_none());
    let mut batch = host
        .runtime
        .server
        .events
        .batch(host.runtime.server.presentation)
        .unwrap();
    let mut messages = Vec::new();
    while let Some(record) = host.runtime.server.events.next(&mut batch) {
        if let FrameEvent::Print(p) = record.event {
            messages.push(
                host.runtime
                    .server
                    .events
                    .texts
                    .get(p.text)
                    .unwrap()
                    .to_vec(),
            );
        }
    }
    assert_eq!(messages.len(), 2);
    for (message, provider) in messages
        .iter()
        .zip([b"native import missing".as_slice(), b"MSVCRT.dll:missing"])
    {
        let message = std::str::from_utf8(message).unwrap();
        assert!(
            message.contains(std::str::from_utf8(provider).unwrap()),
            "{message}"
        );
        assert!(message.contains(" at 0x"), "{message}");
    }
    host.shutdown_modules();
}

fn native_math_imports_execute_in_owned_children_through_session_dispatch() {
    let operations: [(&[u8], fn(f64) -> f64); 8] = [
        (b"sin\0", f64::sin),
        (b"cos\0", f64::cos),
        (b"sqrt\0", f64::sqrt),
        (b"floor\0", f64::floor),
        (b"ceil\0", f64::ceil),
        (b"acos\0", f64::acos),
        (b"fabs\0", f64::abs),
        (b"atan2\0", |x| x.atan2(2.0)),
    ];
    for encoding in [Encoding::Elf, Encoding::Pe] {
        for &(import, operation) in &operations {
            let mut file = runtime_file(encoding, import);
            let code_at = if encoding == Encoding::Elf {
                0x13c0
            } else {
                640
            };
            // Convert the native tick word to double, scale it to 0.5 at tick
            // 50, then tail-call the file's PLT/IAT math import. XMM0 carries
            // the result all the way through the hardware gate and session.
            let mut code = vec![
                0xf2,
                0x48,
                0x0f,
                0x2a,
                if encoding == Encoding::Elf {
                    0xc7
                } else {
                    0xc1
                },
                0x48,
                0xb8,
            ];
            let scale = if import == b"fabs\0" {
                -0.01f64
            } else {
                0.01f64
            };
            code.extend_from_slice(&scale.to_bits().to_le_bytes());
            code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xd0, 0xf2, 0x0f, 0x59, 0xc2]);
            code.extend_from_slice(&[0x48, 0xb8]);
            code.extend_from_slice(&2.0f64.to_bits().to_le_bytes());
            code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc8]);
            let target = if encoding == Encoding::Elf {
                0x1ac0
            } else {
                512 + 0x780
            };
            let displacement = (target as i64 - (code_at + code.len() + 6) as i64) as i32;
            code.extend_from_slice(&[0xff, 0x25]);
            code.extend_from_slice(&displacement.to_le_bytes());
            file[code_at..code_at + code.len()].copy_from_slice(&code);
            if encoding == Encoding::Elf {
                let offset = ELF_SYMBOL_NAMES.len() + import.len();
                file[0x1280 + offset..0x1280 + offset + 10].copy_from_slice(b"libm.so.6\0");
                let (_, mut tags) = elf_symbol_fixture(64);
                tags.iter_mut().find(|(tag, _)| *tag == 10).unwrap().1 = (offset + 10) as u64;
                tags.extend([(1, offset as u64), (7, 0x3b40), (8, 24), (9, 24)]);
                elf_dynamic(&mut file, 64, &tags);
            }
            let mut image = Image::parse(
                &file,
                (encoding == Encoding::Elf).then_some(0x20000000),
                LoadRole::Library,
            )
            .unwrap();
            image.names = qa_core::names::NameTable::load_reserved(
                (0..image.names.len()).map(|i| {
                    image
                        .names
                        .get(qa_core::primitives::NameId(i as u32))
                        .unwrap()
                }),
                1,
                64,
            )
            .unwrap();
            let provider = image
                .names
                .intern(if encoding == Encoding::Elf {
                    b"GLIBC_2.2.5".as_slice()
                } else {
                    b"API-MS-WIN-CRT-MATH-L1-1-0.dll"
                })
                .unwrap();
            if encoding == Encoding::Elf {
                image.symbols[3].version = Some(qa_formats::program::native::Version {
                    name: provider,
                    library: image.names.find(b"libm.so.6"),
                    weak: false,
                });
            } else {
                // API-set names use the same core folded comparison as full CRTs.
                image.imports[0].library = Some(provider);
            }
            let vm = Vm::map_image(
                image,
                &[NamedExport {
                    name: if encoding == Encoding::Elf {
                        b"vmMain"
                    } else {
                        b"GetGameAPI"
                    },
                    command: None,
                    parameters: &[NativeScalar::Word; 13],
                    result: NativeScalar::Double,
                }],
                &[],
                Duration::from_secs(3),
            )
            .unwrap();
            let mut runtime = Runtime::load(1, std::iter::empty()).unwrap();
            let request = request(
                &mut runtime,
                ModuleId(1),
                RuleSetId::Quake3,
                TickRate::fixed(50).unwrap(),
                vm,
            );
            let mut host = FrameHost::load_modules(
                Console::new(Context::default()).unwrap(),
                runtime,
                TickRate::FrameDriven,
                vec![request],
            )
            .unwrap();
            let mut source = Source {
                time: EventTime(0),
                polls: 0,
            };
            host.frame(&mut source, true);
            source.time = EventTime(50_000_000);
            host.frame(&mut source, true);
            let counts = host.module_counts(ModuleId(1)).unwrap();
            assert_eq!(counts.traps, 0, "{encoding:?} {import:?}");
            assert_eq!(
                counts.last_result,
                Some(ModuleResult::Native(operation(50.0 * scale).to_bits())),
                "{encoding:?} {import:?}"
            );
        }
    }
}

fn native_heap_imports_share_owned_memory_through_session_dispatch() {
    // calloc zeros, realloc preserves a marker, and freeing all allocations
    // allows the next malloc to reuse the first address on each native tick.
    const SYSV: &[u8] = &[
        0x53, 0x41, 0x54, 0x41, 0x55, 0x48, 0xc7, 0xc7, 0x20, 0x00, 0x00, 0x00, 0xff, 0x15, 0xa6,
        0x00, 0x00, 0x00, 0x48, 0x85, 0xc0, 0x0f, 0x84, 0x8f, 0x00, 0x00, 0x00, 0x48, 0x89, 0xc3,
        0x48, 0xb8, 0xef, 0xcd, 0xab, 0x89, 0x67, 0x45, 0x23, 0x01, 0x48, 0x89, 0x03, 0x48, 0xc7,
        0xc7, 0x08, 0x00, 0x00, 0x00, 0x48, 0xc7, 0xc6, 0x08, 0x00, 0x00, 0x00, 0xff, 0x15, 0x81,
        0x00, 0x00, 0x00, 0x48, 0x85, 0xc0, 0x74, 0x66, 0x49, 0x89, 0xc4, 0x49, 0x83, 0x3c, 0x24,
        0x00, 0x75, 0x5c, 0x48, 0x89, 0xdf, 0x48, 0xc7, 0xc6, 0x00, 0x10, 0x00, 0x00, 0xff, 0x15,
        0x6a, 0x00, 0x00, 0x00, 0x48, 0x85, 0xc0, 0x74, 0x47, 0x49, 0x89, 0xc5, 0x48, 0xb8, 0xef,
        0xcd, 0xab, 0x89, 0x67, 0x45, 0x23, 0x01, 0x49, 0x39, 0x45, 0x00, 0x75, 0x34, 0x4c, 0x89,
        0xef, 0xff, 0x15, 0x51, 0x00, 0x00, 0x00, 0x4c, 0x89, 0xe7, 0xff, 0x15, 0x48, 0x00, 0x00,
        0x00, 0x48, 0xc7, 0xc7, 0x10, 0x00, 0x00, 0x00, 0xff, 0x15, 0x23, 0x00, 0x00, 0x00, 0x48,
        0x39, 0xd8, 0x75, 0x10, 0x48, 0x89, 0xc7, 0xff, 0x15, 0x2d, 0x00, 0x00, 0x00, 0xb8, 0x2a,
        0x00, 0x00, 0x00, 0xeb, 0x07, 0x48, 0xc7, 0xc0, 0xff, 0xff, 0xff, 0xff, 0x41, 0x5d, 0x41,
        0x5c, 0x5b, 0xc3, 0x90, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];
    const MS: &[u8] = &[
        0x53, 0x41, 0x54, 0x41, 0x55, 0x48, 0x83, 0xec, 0x20, 0x48, 0xc7, 0xc1, 0x20, 0x00, 0x00,
        0x00, 0xff, 0x15, 0xaa, 0x00, 0x00, 0x00, 0x48, 0x85, 0xc0, 0x0f, 0x84, 0x8f, 0x00, 0x00,
        0x00, 0x48, 0x89, 0xc3, 0x48, 0xb8, 0xef, 0xcd, 0xab, 0x89, 0x67, 0x45, 0x23, 0x01, 0x48,
        0x89, 0x03, 0x48, 0xc7, 0xc1, 0x08, 0x00, 0x00, 0x00, 0x48, 0xc7, 0xc2, 0x08, 0x00, 0x00,
        0x00, 0xff, 0x15, 0x85, 0x00, 0x00, 0x00, 0x48, 0x85, 0xc0, 0x74, 0x66, 0x49, 0x89, 0xc4,
        0x49, 0x83, 0x3c, 0x24, 0x00, 0x75, 0x5c, 0x48, 0x89, 0xd9, 0x48, 0xc7, 0xc2, 0x00, 0x10,
        0x00, 0x00, 0xff, 0x15, 0x6e, 0x00, 0x00, 0x00, 0x48, 0x85, 0xc0, 0x74, 0x47, 0x49, 0x89,
        0xc5, 0x48, 0xb8, 0xef, 0xcd, 0xab, 0x89, 0x67, 0x45, 0x23, 0x01, 0x49, 0x39, 0x45, 0x00,
        0x75, 0x34, 0x4c, 0x89, 0xe9, 0xff, 0x15, 0x55, 0x00, 0x00, 0x00, 0x4c, 0x89, 0xe1, 0xff,
        0x15, 0x4c, 0x00, 0x00, 0x00, 0x48, 0xc7, 0xc1, 0x10, 0x00, 0x00, 0x00, 0xff, 0x15, 0x27,
        0x00, 0x00, 0x00, 0x48, 0x39, 0xd8, 0x75, 0x10, 0x48, 0x89, 0xc1, 0xff, 0x15, 0x31, 0x00,
        0x00, 0x00, 0xb8, 0x2a, 0x00, 0x00, 0x00, 0xeb, 0x07, 0x48, 0xc7, 0xc0, 0xff, 0xff, 0xff,
        0xff, 0x48, 0x83, 0xc4, 0x20, 0x41, 0x5d, 0x41, 0x5c, 0x5b, 0xc3, 0x90, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];
    for encoding in [Encoding::Elf, Encoding::Pe] {
        let code = if encoding == Encoding::Elf { SYSV } else { MS };
        let imports = [b"malloc\0".as_slice(), b"calloc\0", b"realloc\0", b"free\0"];
        let mut file = function_file(encoding, &[0xc3]);
        let slot_offset = code.len() - 32;
        match encoding {
            Encoding::Elf => {
                file[0x1600..0x1600 + code.len()].copy_from_slice(code);
                put(&mut file, 0x1400 + 24 + 8, 0x3600, 8);
                let mut offset = ELF_SYMBOL_NAMES.len();
                for (index, import) in imports.iter().enumerate() {
                    file[0x1280 + offset..0x1280 + offset + import.len()].copy_from_slice(import);
                    let symbol = 0x1400 + (index + 3) * 24;
                    file[symbol..symbol + 24].fill(0);
                    put(&mut file, symbol, offset as u64, 4);
                    put(&mut file, symbol + 4, 0x12, 1);
                    elf_relocation(
                        &mut file,
                        64,
                        0x1b40 + index * 24,
                        (0x3600 + slot_offset + index * 8) as u64,
                        7,
                        (index + 3) as u32,
                        Some(0),
                    );
                    offset += import.len();
                }
                let (_, mut tags) = elf_symbol_fixture(64);
                tags.iter_mut().find(|(tag, _)| *tag == 10).unwrap().1 = offset as u64;
                tags.iter_mut().find(|(tag, _)| *tag == 39).unwrap().1 = 7 * 24;
                tags.extend([(1, elf_name(b"libc.so.6")), (7, 0x3b40), (8, 96), (9, 24)]);
                elf_dynamic(&mut file, 64, &tags);
            }
            Encoding::Pe => {
                put(&mut file, 392 + 16, 4096, 4);
                file.resize(4608, 0);
                file[1536..1536 + code.len()].copy_from_slice(code);
                pe_rva(&mut file, 0x1140, 0x1400, 4);
                pe_directory(&mut file, 64, 1, 0x1200, 40);
                pe_rva(&mut file, 0x1200, 0x1250, 4);
                pe_rva(&mut file, 0x120c, 0x12e0, 4);
                pe_rva(&mut file, 0x1210, (0x1400 + slot_offset) as u64, 4);
                pe_text(&mut file, 0x12e0, b"API-MS-WIN-CRT-HEAP-L1-1-0.dll\0");
                let mut offset = 0x12a0;
                for (index, import) in imports.iter().enumerate() {
                    pe_rva(&mut file, 0x1250 + index * 8, offset as u64, 8);
                    pe_rva(
                        &mut file,
                        0x1400 + slot_offset + index * 8,
                        offset as u64,
                        8,
                    );
                    pe_text(&mut file, offset + 2, import);
                    offset += import.len() + 2;
                }
                pe_rva(&mut file, 0x1250 + 32, 0, 8);
                // The IAT's zero terminator is adjacent to the four slot words.
                pe_rva(&mut file, 0x1400 + code.len(), 0, 8);
            }
        }
        let image = Image::parse(
            &file,
            (encoding == Encoding::Elf).then_some(0x20000000),
            LoadRole::Library,
        )
        .unwrap();
        let vm = Vm::map_image(
            image,
            &[NamedExport {
                name: if encoding == Encoding::Elf {
                    b"vmMain"
                } else {
                    b"GetGameAPI"
                },
                command: None,
                parameters: &[NativeScalar::Word; 13],
                result: NativeScalar::Word,
            }],
            &[],
            Duration::from_secs(3),
        )
        .unwrap();
        let mut runtime = Runtime::load(1, std::iter::empty()).unwrap();
        let request = request(
            &mut runtime,
            ModuleId(1),
            RuleSetId::Quake3,
            TickRate::fixed(50).unwrap(),
            vm,
        );
        let mut host = FrameHost::load_modules(
            Console::new(Context::default()).unwrap(),
            runtime,
            TickRate::FrameDriven,
            vec![request],
        )
        .unwrap();
        let mut source = Source {
            time: EventTime(0),
            polls: 0,
        };
        for tick in [0, 50, 100, 150] {
            source.time = EventTime(tick * 1_000_000);
            host.frame(&mut source, true);
            let counts = host.module_counts(ModuleId(1)).unwrap();
            assert_eq!(counts.traps, 0, "{encoding:?}");
            if tick != 0 {
                assert_eq!(
                    counts.last_result,
                    Some(ModuleResult::Native(42)),
                    "{encoding:?}"
                );
            }
        }
    }
}

fn append_trace_digit(file: &mut [u8], at: usize, trace: usize, digit: u8) {
    let mut code = vec![0x8b, 0x05];
    code.extend((trace as i32 - (at + 6) as i32).to_le_bytes());
    code.extend([0x6b, 0xc0, 10, 0x83, 0xc0, digit, 0x89, 0x05]);
    code.extend((trace as i32 - (at + 18) as i32).to_le_bytes());
    code.push(0xc3);
    file[at..at + code.len()].copy_from_slice(&code);
}

fn elf_lifecycle_file() -> Vec<u8> {
    let mut file = function_file(Encoding::Elf, &[0xc3]);
    put(&mut file, 64 + 3 * 56, 0, 4); // no TLS dependency
    put(&mut file, 0x1400 + 24 + 8, 0x3600, 8); // game export code
    file[0x1a80..0x1a90].fill(0); // trace and initialization count
    for (at, digit) in [
        (0x1700, 1),
        (0x1740, 2),
        (0x1780, 3),
        (0x17c0, 6),
        (0x1800, 7),
        (0x1840, 8),
    ] {
        append_trace_digit(&mut file, at, 0x1a80, digit);
    }
    // The final native callback traps unless the whole reached order is exact.
    // This checks finalizer order without exposing raw module memory to app.
    file[0x1852..0x185c].copy_from_slice(&[0x3d, 0xa8, 0x61, 0xbc, 0, 0x74, 2, 0x0f, 0x0b, 0xc3]);
    // DT_INIT sees a native argc and separate valid, empty argv/envp vectors.
    let checks = [
        0x85, 0xff, 0x75, 16, 0x48, 0x83, 0x3e, 0, 0x75, 10, 0x48, 0x83, 0x3a, 0, 0x75, 4, 0xeb, 4,
        0x0f, 0x0b, 0x0f, 0x0b,
    ];
    file[0x1700..0x1700 + checks.len()].copy_from_slice(&checks);
    append_trace_digit(&mut file, 0x1700 + checks.len(), 0x1a80, 1);
    // dllEntry stores the shared syscall callback and appends digit 4.
    file[0x1300..0x1307].copy_from_slice(&[0x48, 0x89, 0x3d, 0xb9, 7, 0, 0]);
    append_trace_digit(&mut file, 0x1307, 0x1a80, 4);
    // Game command 1 is shutdown; all other commands return the reached trace.
    file[0x1600..0x1605].copy_from_slice(&[0x83, 0xff, 1, 0x74, 7]);
    file[0x1605..0x1607].copy_from_slice(&[0x8b, 0x05]);
    put(&mut file, 0x1607, (0x1a80 - 0x160b) as u64, 4);
    file[0x160b] = 0xc3;
    append_trace_digit(&mut file, 0x160c, 0x1a80, 5);
    for (at, value) in [
        (0x1a00, 0x3740),
        (0x1a08, 0x3780),
        (0x1a20, 0x37c0),
        (0x1a28, 0x3800),
    ] {
        put(&mut file, at, value, 8);
    }
    for (index, (at, value)) in [
        (0x3a00, 0x3740),
        (0x3a08, 0x3780),
        (0x3a20, 0x37c0),
        (0x3a28, 0x3800),
    ]
    .into_iter()
    .enumerate()
    {
        elf_relocation(&mut file, 64, 0x1b40 + index * 24, at, 8, 0, Some(value));
    }
    let (_, mut tags) = elf_symbol_fixture(64);
    tags.extend([
        (12, 0x3700),
        (25, 0x3a00),
        (27, 16),
        (26, 0x3a20),
        (28, 16),
        (13, 0x3840),
        (7, 0x3b40),
        (8, 96),
        (9, 24),
        (32, u64::MAX),
        (33, 8),
    ]); // ignored shared-object preinit
    elf_dynamic(&mut file, 64, &tags);
    file
}

fn elf_lifecycle_uses_session_order_once_and_a_bad_constructor_stops_only_its_module() {
    use qa_app::modules::{Q3Spec, load_q3};
    let files =
        Files(std::env::temp_dir().join(format!("qa-native-lifecycle-{}", std::process::id())));
    std::fs::create_dir(&files.0).unwrap();
    let file = elf_lifecycle_file();
    std::fs::write(files.0.join("good.so"), &file).unwrap();
    let mut bad = file;
    bad[0x1700..0x1702].copy_from_slice(&[0x0f, 0x0b]);
    std::fs::write(files.0.join("bad.so"), &bad).unwrap();
    let mut runtime = Runtime::load(1, std::iter::empty()).unwrap();
    runtime.vfs.mount_directory(&files.0, 0).unwrap();
    let mut requests = Vec::new();
    load_q3(
        &mut runtime,
        &[
            Q3Spec::parse("game:bad.so").unwrap(),
            Q3Spec::parse("game:good.so").unwrap(),
        ],
        &mut requests,
        TickRate::fixed(50).unwrap(),
        None,
    )
    .unwrap();
    for request in &requests {
        assert_eq!(request.prepare.len(), 4);
        assert_eq!(request.shutdown.len(), 4);
    }
    let mut host = FrameHost::load_modules(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        requests,
    )
    .unwrap();
    let mut source = Source {
        time: EventTime(0),
        polls: 0,
    };
    for tick in [0, 50, 100] {
        source.time = EventTime(tick * 1_000_000);
        host.frame(&mut source, true);
        assert_eq!(host.module_state(ModuleId(1)), Some(State::Failed));
        assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, 1);
        assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 1);
        assert_eq!(
            host.module_counts(ModuleId(2)).unwrap().last_result,
            Some(ModuleResult::Native(1234))
        );
    }
    let calls = host.module_counts(ModuleId(2)).unwrap().calls;
    host.shutdown_modules();
    assert_eq!(host.module_state(ModuleId(2)), Some(State::Stopped));
    assert_eq!(host.module_counts(ModuleId(2)).unwrap().calls, calls + 4);
    assert_eq!(host.module_counts(ModuleId(2)).unwrap().traps, 0);
    // Finalizers return void; the final callback asserts the shared trace is
    // 12345768, otherwise the child traps and the count above fails.
    host.shutdown_modules();
    assert_eq!(host.module_counts(ModuleId(2)).unwrap().calls, calls + 4);
}

fn elf_lifecycle_rejects_malformed_arrays_and_non_executable_targets() {
    for kind in 0..4 {
        let mut image =
            Image::parse(&elf_lifecycle_file(), Some(0x20000000), LoadRole::Library).unwrap();
        match kind {
            0 => {
                image
                    .dynamic
                    .iter_mut()
                    .find(|(tag, _)| *tag == 27)
                    .unwrap()
                    .1 = 7
            }
            1 => {
                image.dynamic = image
                    .dynamic
                    .iter()
                    .copied()
                    .filter(|&(tag, _)| tag != 27)
                    .collect()
            }
            2 => {
                image
                    .dynamic
                    .iter_mut()
                    .find(|(tag, _)| *tag == 25)
                    .unwrap()
                    .1 = 0x100000
            }
            _ => {
                image
                    .dynamic
                    .iter_mut()
                    .find(|(tag, _)| *tag == 12)
                    .unwrap()
                    .1 = 0x100000
            }
        }
        assert!(
            Vm::map_image(
                image,
                &[NamedExport {
                    name: b"vmMain",
                    command: None,
                    parameters: &[],
                    result: NativeScalar::Word
                }],
                &[],
                Duration::from_secs(3)
            )
            .is_err()
        );
    }
}

fn pe_lifecycle_file() -> Vec<u8> {
    let mut file = function_file(Encoding::Pe, &[0xc3]);
    put(&mut file, 392 + 16, 4096, 4);
    file.resize(4608, 0);
    pe_text(&mut file, 0x1190, b"vmMain\0");
    put(&mut file, 168, 0x1400, 4);
    let base = Image::parse(&file, None, LoadRole::Library).unwrap().base;
    pe_directory(&mut file, 64, 9, 0x1a00, 40);
    for (at, value) in [
        (0x1a00, base + 0x1a80),
        (0x1a08, base + 0x1a84),
        (0x1a10, base + 0x1a90),
        (0x1a18, base + 0x1aa0),
        (0x1aa0, base + 0x1500),
        (0x1aa8, base + 0x1580),
    ] {
        pe_rva(&mut file, at, value, 8);
    }
    pe_rva(&mut file, 0x1a20, 16, 4);
    pe_rva(&mut file, 0x1a24, 5 << 20, 4); // sixteen-byte native alignment
    pe_rva(&mut file, 0x1a80, 0x11223344, 4);
    for (at, attach, detach) in [(0x600, 3, 6), (0x700, 1, 7), (0x780, 2, 8)] {
        let mut prefix = vec![0x48, 0xb8];
        prefix.extend(base.to_le_bytes());
        prefix.extend([0x48, 0x39, 0xc1, 0x74, 2, 0x0f, 0x0b]); // correct HMODULE
        prefix.extend([0x4d, 0x85, 0xc0, 0x74, 2, 0x0f, 0x0b]); // null reserved pointer
        prefix.extend([0x65, 0x48, 0x8b, 0x04, 0x25, 0x58, 0, 0, 0, 0x48, 0x8b, 0]);
        prefix.extend([0x81, 0x38, 0x44, 0x33, 0x22, 0x11, 0x74, 2, 0x0f, 0x0b]);
        prefix.extend([0x85, 0xd2, 0x74, 19]); // detach skips one complete append
        file[at..at + prefix.len()].copy_from_slice(&prefix);
        append_trace_digit(&mut file, at + prefix.len(), 0xd00, attach);
        append_trace_digit(&mut file, at + prefix.len() + 19, 0xd00, detach);
        if detach == 8 {
            let end = at + prefix.len() + 19 + 18;
            file[end..end + 10]
                .copy_from_slice(&[0x3d, 0x4e, 0x61, 0xbc, 0, 0x74, 2, 0x0f, 0x0b, 0xc3]);
        }
    }
    // dllEntry follows TLS callbacks and DllMain, then game init/frame run.
    file[704..711].copy_from_slice(&[0x48, 0x89, 0x0d, 0xb9, 6, 0, 0]);
    append_trace_digit(&mut file, 711, 0xd00, 4);
    file[640..645].copy_from_slice(&[0x83, 0xf9, 1, 0x74, 7]);
    file[645..647].copy_from_slice(&[0x8b, 0x05]);
    put(&mut file, 647, (0xd00 - 651) as u64, 4);
    file[651] = 0xc3;
    append_trace_digit(&mut file, 652, 0xd00, 5);
    file
}

fn pe_lifecycle_uses_session_order_and_rejects_false_attach() {
    use qa_app::modules::{Q3Spec, load_q3};
    let files = Files(std::env::temp_dir().join(format!("qa-pe-lifecycle-{}", std::process::id())));
    std::fs::create_dir(&files.0).unwrap();
    let file = pe_lifecycle_file();
    std::fs::write(files.0.join("good.dll"), &file).unwrap();
    let mut bad = file;
    bad[0x600..0x603].copy_from_slice(&[0x31, 0xc0, 0xc3]);
    std::fs::write(files.0.join("bad.dll"), &bad).unwrap();
    let mut runtime = Runtime::load(1, std::iter::empty()).unwrap();
    runtime.vfs.mount_directory(&files.0, 0).unwrap();
    let mut requests = Vec::new();
    load_q3(
        &mut runtime,
        &[
            Q3Spec::parse("game:bad.dll").unwrap(),
            Q3Spec::parse("game:good.dll").unwrap(),
        ],
        &mut requests,
        TickRate::fixed(50).unwrap(),
        None,
    )
    .unwrap();
    for request in &requests {
        assert_eq!(request.prepare.len(), 4);
        assert_eq!(request.shutdown.len(), 4);
    }
    let mut host = FrameHost::load_modules(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        requests,
    )
    .unwrap();
    let mut source = Source {
        time: EventTime(0),
        polls: 0,
    };
    for tick in [0, 50, 100] {
        source.time = EventTime(tick * 1_000_000);
        host.frame(&mut source, true);
        assert_eq!(host.module_state(ModuleId(1)), Some(State::Failed));
        assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, 3);
        assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 1);
        assert_eq!(
            host.module_counts(ModuleId(2)).unwrap().last_result,
            Some(ModuleResult::Native(1234))
        );
    }
    let calls = host.module_counts(ModuleId(2)).unwrap().calls;
    host.shutdown_modules();
    assert_eq!(host.module_counts(ModuleId(2)).unwrap().calls, calls + 4);
    assert_eq!(host.module_counts(ModuleId(2)).unwrap().traps, 0);
    host.shutdown_modules();
    assert_eq!(host.module_counts(ModuleId(2)).unwrap().calls, calls + 4);
}

fn pe_lifecycle_rejects_unmapped_callbacks_and_read_only_tls_indices() {
    for kind in 0..3 {
        let mut image = Image::parse(&pe_lifecycle_file(), None, LoadRole::Library).unwrap();
        match kind {
            0 => image.initializers[0] = image.base + 80,
            1 => image.entry = image.base + image.bytes.len() as u64 + 0x1000,
            _ => {
                for region in &mut image.regions {
                    region.write = false;
                }
            }
        }
        let error = Vm::map_image(
            image,
            &[NamedExport {
                name: b"vmMain",
                command: Some(0),
                parameters: &[NativeScalar::Word; 13],
                result: NativeScalar::Word,
            }],
            &[],
            Duration::from_secs(3),
        )
        .err()
        .expect("invalid PE lifecycle");
        assert!(
            matches!(
                error,
                qa_compat::native::Error::Export | qa_compat::native::Error::Binding(_)
            ),
            "{error:?}"
        );
    }
}

fn scalar_native_export_results_survive_session_dispatch() {
    use qa_platform::native::NativeScalar;
    for encoding in [Encoding::Elf, Encoding::Pe] {
        // double entry(word clock, double a, float b, ...word padding):
        // convert b and clock to double, add to a and return through XMM0.
        let mut code = [
            0xf3, 0x0f, 0x5a, 0xd9, 0xf2, 0x0f, 0x58, 0xc3, 0xf2, 0x48, 0x0f, 0x2a, 0xe7, 0xf2,
            0x0f, 0x58, 0xc4, 0x66, 0x0f, 0x28, 0xc0, 0xc3,
        ];
        if encoding == Encoding::Pe {
            code[3] = 0xda;
            code[7] = 0xcb;
            code[12] = 0xe1;
            code[16] = 0xcc;
            code[20] = 0xc1;
        }
        let mut kinds = [NativeScalar::Word; 13];
        kinds[1] = NativeScalar::Double;
        kinds[2] = NativeScalar::Float;
        let vm = Vm::map_image(
            function_image(encoding, &code),
            &[NamedExport {
                name: if encoding == Encoding::Elf {
                    b"vmMain"
                } else {
                    b"GetGameAPI"
                },
                command: None,
                parameters: &kinds,
                result: NativeScalar::Double,
            }],
            &[],
            Duration::from_secs(3),
        )
        .unwrap();
        let mut runtime = Runtime::load(1, std::iter::empty()).unwrap();
        let request = request(
            &mut runtime,
            ModuleId(1),
            RuleSetId::Quake3,
            TickRate::fixed(50).unwrap(),
            vm,
        );
        let mut host = FrameHost::load_modules(
            Console::new(Context::default()).unwrap(),
            runtime,
            TickRate::FrameDriven,
            vec![request],
        )
        .unwrap();
        let mut source = Source {
            time: EventTime(0),
            polls: 0,
        };
        host.frame(&mut source, true);
        source.time = EventTime(50_000_000);
        host.frame(&mut source, true);
        let counts = host.module_counts(ModuleId(1)).unwrap();
        assert_eq!(counts.traps, 0);
        assert_eq!(
            counts.last_result,
            Some(ModuleResult::Native(50f64.to_bits()))
        );
    }
}

fn q2_table_file() -> Vec<u8> {
    // API 3 x86-64 layout from quake-2/game/game.h: version at 0,
    // Init/Shutdown at 8/16, RunFrame at 112, edict metadata through 148.
    let mut file = function_file(Encoding::Pe, &[0x48, 0x8d, 0x05, 0x79, 7, 0, 0, 0xc3]);
    put(&mut file, 168, 0, 4); // this API-table fixture has no DLL lifecycle
    file.resize(4608, 0);
    put(&mut file, 392 + 16, 4096, 4);
    pe_rva(&mut file, 0x1800, 3, 4);
    for (offset, target) in [(8, 0x1400), (16, 0x1440), (112, 0x1480)] {
        pe_rva(&mut file, 0x1800 + offset, 0x180000000 + target, 8);
    }
    // Init writes 1; RunFrame adds 2 and clears its original table pointer.
    // The second frame must still use the cached checked entry.
    pe_text(
        &mut file,
        0x1400,
        &[0xc7, 0x05, 0xf6, 4, 0, 0, 1, 0, 0, 0, 0xc3],
    );
    pe_text(
        &mut file,
        0x1480,
        &[
            0x83, 0x05, 0x79, 4, 0, 0, 2, 0x48, 0xc7, 0x05, 0xde, 3, 0, 0, 0, 0, 0, 0, 0xc3,
        ],
    );
    // Shutdown requires Init + two frames = 5, otherwise UD2 traps.
    pe_text(
        &mut file,
        0x1440,
        &[0x83, 0x3d, 0xb9, 4, 0, 0, 5, 0x74, 2, 0x0f, 0x0b, 0xc3],
    );
    file
}

fn q2_normal_loader_selects_native_roles_and_copies_spawn_strings() {
    use qa_app::modules::{Q2Spec, load_q2};
    use qa_formats::entities::EntitySyntax;
    let files = Files(std::env::temp_dir().join(format!("qa-q2-loader-{}", std::process::id())));
    std::fs::create_dir(&files.0).unwrap();
    let mut runtime = Runtime::load(1, std::iter::empty()).unwrap();
    runtime.entity_sources.push(qa_app::map::NativeEntityText {
        syntax: EntitySyntax::Quake3,
        bytes: b"{\"classname\" \"worldspawn\"}\0".as_slice().into(),
    });
    for rr in [false, true] {
        let mut file = q2_table_file();
        pe_rva(&mut file, 0x1800, if rr { 2023 } else { 3 }, 4);
        pe_text(&mut file, 0x1500, &[0xc3]);
        // Both loaded modules retain their real GetGameAPI import pointer.
        let mut api = vec![0x48, 0x89, 0x0d];
        api.extend((0x1b20i32 - 0x1087).to_le_bytes());
        api.extend([0x48, 0x8d, 0x05]);
        api.extend((0x1800i32 - 0x108e).to_le_bytes());
        api.push(0xc3);
        pe_text(&mut file, 0x1080, &api);
        // Init precaches one sound through the actual native soundindex import.
        let mut sound = vec![0x48, 0x83, 0xec, 0x28, 0x48, 0x8b, 0x05];
        sound.extend((0x1b20i32 - 0x160b).to_le_bytes());
        sound.extend([0x48, 0x8d, 0x0d]);
        sound.extend((0x1bd0i32 - 0x1612).to_le_bytes());
        sound.extend([0x48, 0x8b, 0x80]);
        sound.extend((if rr { 16i32 + 11 * 8 } else { 9 * 8 }).to_le_bytes());
        sound.extend([0xff, 0xd0, 0x48, 0x83, 0xc4, 0x28, 0xc3]);
        pe_text(&mut file, 0x1600, &sound);
        pe_text(&mut file, 0x1bd0, b"_qa_native_loader.wav\0");
        if rr {
            // Verify ServerFrame against native RunFrame calls.
            let mut check = vec![0x48, 0x83, 0xec, 0x28, 0x48, 0x8b, 0x05];
            check.extend((0x1b20i32 - 0x174b).to_le_bytes());
            check.extend([0x48, 0x8b, 0x80]);
            check.extend((16i32 + 65 * 8).to_le_bytes());
            check.extend([0xff, 0xd0, 0x3b, 0x05]);
            check.extend((0x1b30i32 - 0x175a).to_le_bytes());
            check.extend([0x74, 2, 0x0f, 0x0b, 0x48, 0x83, 0xc4, 0x28, 0xc3]);
            pe_text(&mut file, 0x1740, &check);
        }
        for index in 0..if rr { 29 } else { 15 } {
            let offset = if !rr || index < 19 {
                8 + index * 8
            } else {
                192 + (index - 19) * 8
            };
            pe_rva(
                &mut file,
                0x1800 + offset,
                if rr && offset == 192 {
                    0x180001710
                } else {
                    0x180001500
                },
                8,
            );
            if rr && offset == 192 {
                pe_text(&mut file, 0x1710, &[0xc3, 0x90, 0x90, 0x90, 0x90]);
            }
        }
        let mut calls = vec![(if rr { 16 } else { 8 }, 0x1400, u8::from(rr))];
        if rr {
            calls.push((8, 0x1360, 0));
        }
        calls.push((if rr { 32 } else { 24 }, 0x1440, if rr { 2 } else { 1 }));
        calls.push((if rr { 24 } else { 16 }, 0x14c0, if rr { 13 } else { 6 }));
        for (table_offset, rva, expected) in calls {
            pe_rva(
                &mut file,
                0x1800 + table_offset,
                0x180000000 + rva as u64,
                8,
            );
            let mut code = Vec::new();
            if rr && (rva == 0x1400 || rva == 0x14c0) {
                code.extend([0x48, 0x83, 0xec, 0x28, 0xe8]);
                code.extend((0x1740i32 - (rva + 9) as i32).to_le_bytes());
                code.extend([0x48, 0x83, 0xc4, 0x28]);
            }
            if rva == 0x1400 {
                code.extend([0x48, 0x83, 0xec, 0x28, 0xe8]);
                code.extend((0x1600i32 - (rva + code.len() + 4) as i32).to_le_bytes());
                code.extend([0x48, 0x83, 0xc4, 0x28]);
            }
            if rva == 0x1440 {
                // SpawnEntities sees owned native pointers to the basename,
                // unchanged source text, and an empty spawn point.
                for prefix in [
                    &[0x80, 0x39, b'f'][..],
                    &[0x80, 0x3a, b'{'],
                    &[0x41, 0x80, 0x38, 0],
                ] {
                    code.extend_from_slice(prefix);
                    code.extend_from_slice(&[0x74, 2, 0x0f, 0x0b]);
                }
            }
            let start = rva + code.len();
            code.extend_from_slice(&[0x83, 0x3d]);
            code.extend_from_slice(&((0x1b00 - (start + 7)) as i32).to_le_bytes());
            code.extend_from_slice(&[expected, 0x75, 11, 0xc7, 0x05]);
            code.extend_from_slice(&((0x1b00 - (start + 19)) as i32).to_le_bytes());
            code.extend_from_slice(&(u32::from(expected) + 1).to_le_bytes());
            code.extend_from_slice(&[0xc3, 0x0f, 0x0b]);
            pe_text(&mut file, rva, &code);
        }
        pe_rva(
            &mut file,
            0x1800 + if rr { 136 } else { 112 },
            0x180001480,
            8,
        );
        let mut frame = vec![0x83, 0x05, 0x79, 6, 0, 0, 1];
        if rr {
            // Loading's two RunFrame(false) calls leave ServerFrame at zero.
            // Ordinary RunFrame(true) sees the old index, then advances it.
            frame.extend([0x48, 0x83, 0xec, 0x28, 0x88, 0x4c, 0x24, 0x20, 0xe8]);
            frame.extend((0x1740i32 - 0x1494).to_le_bytes());
            frame.extend([0x80, 0x7c, 0x24, 0x20, 0, 0x74, 7, 0x83, 0x05]);
            frame.extend((0x1b30i32 - 0x14a2).to_le_bytes());
            frame.push(1);
            frame.extend([0x48, 0x83, 0xc4, 0x28]);
        }
        frame.push(0xc3);
        pe_text(&mut file, 0x1480, &frame);
        std::fs::write(
            files.0.join(if rr { "rr.dll" } else { "classic.dll" }),
            file,
        )
        .unwrap();
    }
    runtime.vfs.mount_directory(&files.0, 0).unwrap();
    let mut requests = Vec::new();
    load_q2(
        &mut runtime,
        &[
            Q2Spec::parse("q2:classic.dll").unwrap(),
            Q2Spec::parse("q2rr:rr.dll").unwrap(),
        ],
        &mut requests,
        "maps/fixture.bsp",
        0,
    )
    .unwrap();
    for (request, rules, interval, steps) in [
        (&requests[0], RuleSetId::Quake2, 100, 4),
        (&requests[1], RuleSetId::Quake2Rerelease, 25, 5),
    ] {
        assert_eq!(request.timing_rules, rules);
        assert_eq!(request.context.console.source, rules);
        assert_eq!(request.rate, TickRate::fixed(interval).unwrap());
        assert_eq!(request.initialize.len(), steps);
        assert_eq!(
            request.context.link_order,
            qa_gameplay::rules::link_order(rules)
        );
        assert!(
            matches!(request.frame.arguments[0], Argument::Word(value) if value == u64::from(rules == RuleSetId::Quake2Rerelease))
        );
    }
    for (index, request) in requests.iter_mut().enumerate() {
        request.context.console.source = if index == 0 {
            RuleSetId::Quake3
        } else {
            RuleSetId::QuakeWorld
        };
        request.context.sound_rules = RuleSetId::Quake;
    }
    let mut host = FrameHost::load_modules(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        requests,
    )
    .unwrap();
    let mut source = Source {
        time: EventTime(0),
        polls: 0,
    };
    host.frame(&mut source, true);
    let mut precached = Vec::new();
    host.for_each_native_sound(|name, source| {
        precached.push((name.to_vec(), source));
        Ok(())
    })
    .unwrap();
    assert_eq!(
        precached,
        [
            (b"_qa_native_loader.wav".to_vec(), RuleSetId::Quake2),
            (
                b"_qa_native_loader.wav".to_vec(),
                RuleSetId::Quake2Rerelease
            ),
        ]
    );
    for (id, calls) in [(ModuleId(1), 5), (ModuleId(2), 6)] {
        assert_eq!(
            host.module_state(id),
            Some(State::Running),
            "module {id:?}: {:?}",
            host.module_counts(id)
        );
        assert_eq!(host.module_counts(id).unwrap().calls, calls);
        assert_eq!(host.module_counts(id).unwrap().traps, 0);
    }
    // Irregular client frames make the world index differ from the provider;
    // the 75 -> 125 jump also requires two separate native 25 ms ticks.
    for time in [5, 15, 25, 35, 50, 75, 125, 150, 175, 200] {
        source.time = EventTime(time * 1_000_000);
        host.frame(&mut source, true);
    }
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, 7);
    assert_eq!(host.module_counts(ModuleId(2)).unwrap().calls, 14);
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 0);
    assert_eq!(host.module_counts(ModuleId(2)).unwrap().traps, 0);
    host.shutdown_modules();
    for (id, calls) in [(ModuleId(1), 8), (ModuleId(2), 15)] {
        assert_eq!(host.module_state(id), Some(State::Stopped));
        assert_eq!(host.module_counts(id).unwrap().calls, calls);
        assert_eq!(host.module_counts(id).unwrap().traps, 0);
    }
    host.shutdown_modules();
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, 8);
    assert_eq!(host.module_counts(ModuleId(2)).unwrap().calls, 15);
}

fn native_import_code(
    pointer: u64,
    a: &[u64],
    returns_value: bool,
    float_positions: &[usize],
    output: u64,
) -> Vec<u8> {
    let stack = if a.len() > 4 { 0x48 } else { 0x28 };
    let mut code = vec![0x48, 0x83, 0xec, stack];
    for &index in float_positions.iter().filter(|&&i| i < 4) {
        code.push(0xb8);
        code.extend((a[index] as u32).to_le_bytes());
        code.extend([0x66, 0x0f, 0x6e, 0xc0 + index as u8 * 8]);
    }
    for (arg, register) in
        a.iter()
            .take(4)
            .zip([[0x48, 0xb9], [0x48, 0xba], [0x49, 0xb8], [0x49, 0xb9]])
    {
        code.extend(register);
        code.extend(arg.to_le_bytes());
    }
    for (index, arg) in a.iter().skip(4).enumerate() {
        code.extend([0x48, 0xb8]);
        code.extend(arg.to_le_bytes());
        code.extend([0x48, 0x89, 0x44, 0x24, (32 + index * 8) as u8]);
    }
    code.extend([0x48, 0xb8]);
    code.extend(pointer.to_le_bytes());
    code.extend([0xff, 0xd0]);
    if returns_value {
        code.extend([0x48, 0xb9]);
        code.extend(output.to_le_bytes());
        code.extend([0x48, 0x89, 1]);
    }
    code.extend([0x48, 0x83, 0xc4, stack, 0xc3]);
    code
}

fn q2_clients_bind_after_initialization_and_disconnect_before_shutdown() {
    use qa_app::modules::{ApiCheck, Q2Clients};
    use qa_compat::native::q2::Game;
    use qa_core::primitives::{NativeEntity, PlayerTail};
    use qa_session::clients::Connection;
    fn error_text(error: impl std::fmt::Display) -> qa_core::text::FixedText<256> {
        use std::fmt::Write;
        let mut text = qa_core::text::FixedText::default();
        write!(&mut text, "{error}").unwrap();
        text
    }
    for rr in [false, true] {
        for (accepted, independent_console) in [(false, false), (true, false), (true, true)] {
            if rr && independent_console {
                continue;
            }
            let rules = if rr {
                RuleSetId::Quake2Rerelease
            } else {
                RuleSetId::Quake2
            };
            let admitted = accepted && !independent_console;
            let stride = if rr { 1472 } else { 280 };
            let target = 0x180003000 + stride * 2;
            let mut file = q2_table_file();
            pe_rva(&mut file, 0x1800, if rr { 2023 } else { 3 }, 4);
            pe_text(&mut file, 0x1700, &[0xc3]);
            for index in 0..if rr { 29 } else { 15 } {
                let offset = if !rr || index < 19 {
                    8 + index * 8
                } else {
                    192 + (index - 19) * 8
                };
                pe_rva(
                    &mut file,
                    0x1800 + offset,
                    if rr && offset == 192 {
                        0x180001710
                    } else {
                        0x180001700
                    },
                    8,
                );
                if rr && offset == 192 {
                    pe_text(&mut file, 0x1710, &[0xc3, 0x90, 0x90, 0x90, 0x90]);
                }
            }
            let metadata = 0x1800 + if rr { 160 } else { 128 };
            pe_rva(&mut file, metadata, 0x180003000, 8);
            pe_rva(&mut file, metadata + 8, stride, if rr { 8 } else { 4 });
            pe_rva(&mut file, metadata + if rr { 16 } else { 12 }, 3, 4);
            pe_rva(&mut file, metadata + if rr { 20 } else { 16 }, 3, 4);
            for (offset, at, expected, result) in [
                (
                    if rr { 88 } else { 64 },
                    0x1500,
                    0,
                    if accepted { if rr { 1 } else { 2 } } else { 0 },
                ),
                (if rr { 96 } else { 72 }, 0x1560, 1, 0),
                (if rr { 112 } else { 88 }, 0x15c0, 2, 0),
                (
                    if rr { 24 } else { 16 },
                    0x1620,
                    if admitted {
                        3
                    } else if independent_console {
                        0
                    } else {
                        1
                    },
                    0,
                ),
            ] {
                pe_rva(&mut file, 0x1800 + offset, 0x180000000 + at, 8);
                let mut code = Vec::new();
                if at != 0x1620 {
                    code.extend([0x48, 0xb8]);
                    code.extend(target.to_le_bytes());
                    code.extend([0x48, 0x39, 0xc1, 0x74, 2, 0x0f, 0x0b]);
                }
                if at == 0x1500 {
                    code.extend([0x80, 0x3a, b'\\', 0x74, 2, 0x0f, 0x0b]);
                    if rr {
                        code.extend([0x41, 0x80, 0x38, 0, 0x74, 2, 0x0f, 0x0b]);
                        code.extend([0x45, 0x84, 0xc9, 0x74, 2, 0x0f, 0x0b]);
                    }
                }
                let start = at as usize + code.len();
                code.extend([0x83, 0x3d]);
                code.extend((0x1b00i32 - (start + 7) as i32).to_le_bytes());
                code.extend([expected, 0x74, 2, 0x0f, 0x0b]);
                let start = at as usize + code.len();
                code.extend([0x83, 0x05]);
                code.extend((0x1b00i32 - (start + 7) as i32).to_le_bytes());
                code.extend([1, 0xb8]);
                code.extend((result as u32).to_le_bytes());
                code.push(0xc3);
                pe_text(&mut file, at as usize, &code);
            }
            pe_rva(
                &mut file,
                0x1800 + if rr { 128 } else { 104 },
                0x180001200,
                8,
            );
            let mut think = vec![0x48, 0xb8];
            think.extend(target.to_le_bytes());
            think.extend([0x48, 0x39, 0xc1, 0x74, 2, 0x0f, 0x0b]);
            for offset in (0..if rr { 28 } else { 16 }).step_by(4) {
                think.extend([0x8b, 0x42, offset]);
                think.extend([0x48, 0xb9]);
                think.extend((0x180004400u64 + u64::from(offset)).to_le_bytes());
                think.extend([0x3b, 0x01, 0x74, 2, 0x0f, 0x0b]);
                if !rr || offset < 24 {
                    think.extend([0xc7, 0x01, 0, 0, 0, 0]);
                }
            }
            // The module changes a native stat; the ordinary returned-call
            // boundary must import it into the shared player/value bank.
            think.extend([0x48, 0xb9]);
            think.extend((0x180006000u64 + if rr { 166 } else { 120 } + 2).to_le_bytes());
            think.extend([0x66, 0x83, 0x01, 1]);
            think.push(0xc3);
            assert!(think.len() <= 0x200);
            pe_text(&mut file, 0x1200, &think);
            let mut image = Image::parse(&file, None, LoadRole::Library).unwrap();
            let mut bytes = std::mem::take(&mut image.bytes).into_vec();
            bytes.resize(0x8000, 0);
            let pointer = target as usize - 0x180000000 + if rr { 120 } else { 88 };
            bytes[pointer..pointer + 8].copy_from_slice(&0x180006000u64.to_le_bytes());
            let stats_at = 0x6000 + if rr { 166 } else { 120 };
            let native_stats: Vec<i16> = (0..if rr { 64 } else { 32 })
                .map(|index| match index {
                    0 => i16::MIN,
                    1 => 90,
                    5 => 41,
                    14 => -7,
                    _ => index as i16,
                })
                .collect();
            for (index, value) in native_stats.iter().enumerate() {
                bytes[stats_at + index * 2..stats_at + index * 2 + 2]
                    .copy_from_slice(&value.to_le_bytes());
            }
            let expected: Vec<u8> = if rr {
                [
                    vec![255, 139, 0, 0],
                    [90.0f32, -90.0, 180.0, 123.75, -456.5]
                        .into_iter()
                        .flat_map(f32::to_le_bytes)
                        .collect(),
                    0xfedcba98u32.to_le_bytes().to_vec(),
                ]
                .concat()
            } else {
                vec![
                    255, 131, 0, 64, 0, 192, 0, 128, 123, 0, 56, 254, 200, 0, 17, 42,
                ]
            };
            bytes[0x4400..0x4400 + expected.len()].copy_from_slice(&expected);
            image.bytes = bytes.into_boxed_slice();
            let mut regions = std::mem::take(&mut image.regions).into_vec();
            regions.push(qa_formats::program::native::Region {
                offset: 0x3000,
                length: 0x5000,
                read: true,
                write: true,
                execute: false,
            });
            image.regions = regions.into_boxed_slice();
            let mut runtime = Runtime::load(2, []).unwrap();
            let id = runtime
                .server
                .connect(Connection::Local, ModuleId(0), PlayerTail::None, None)
                .unwrap();
            runtime.server.clients[id.0 as usize].player.movement_rules = RuleSetId::Quake3;
            runtime.server.clients[id.0 as usize].player.trace_rules = RuleSetId::Quake;
            runtime.local_snapshots[id.0 as usize] = Some(qa_app::snapshots::LocalSnapshot::load(
                id,
                qa_core::sys_events::SeatId::FIRST,
                runtime.server.clients[id.0 as usize].output.unwrap(),
                qa_network::commands::packet::Protocol::Quake2_34,
                1,
                &runtime.catalog.hud.values,
            ));
            let game =
                Game::map(image, rules, 25, &runtime.geometry, Duration::from_secs(3)).unwrap();
            let api = Q2Clients {
                module: ModuleId(1),
                rules,
                connect: CallbackId(game.entry(b"ClientConnect").unwrap()),
                begin: CallbackId(game.entry(b"ClientBegin").unwrap()),
                disconnect: CallbackId(game.entry(b"ClientDisconnect").unwrap()),
                userinfo: 0x180005000,
                usercmd: 0x180005c00,
                abi: qa_compat::q2_abi::client_layout(rules).unwrap(),
                think: CallbackId(game.entry(b"ClientThink").unwrap()),
                command: CallbackId(game.entry(b"ClientCommand").unwrap()),
                userinfo_changed: CallbackId(game.entry(b"ClientUserinfoChanged").unwrap()),
                social: 0x180005800,
            };
            let imports = game.imports;
            let imports_address = game.imports_address;
            let count = game.vm.export_count();
            let shutdown = Export::words(CallbackId(game.entry(b"Shutdown").unwrap()), &[]);
            let mut request = request(
                &mut runtime,
                ModuleId(1),
                rules,
                TickRate::FrameDriven,
                game.vm,
            );
            if let Program::Native { imports: table, .. } = &mut request.program {
                *table = imports;
            }
            request.entries = (0..count as u32).collect();
            request.api = Some(ApiCheck::NativeTable {
                export: Export::words(CallbackId(0), &[imports_address]),
            });
            request.shutdown.push(shutdown);
            if independent_console {
                request.context.console.source = RuleSetId::Quake3;
            }
            let context = request.context.console;
            let mut host = FrameHost::load_modules(
                Console::new(Context::default()).unwrap(),
                runtime,
                TickRate::FrameDriven,
                vec![request],
            )
            .unwrap();
            let maxclients = host.console.cvars.bind("sv_maxclients", context).unwrap();
            if independent_console {
                let native_limit = host.console.cvars.integer_in(maxclients.canonical(), rules);
                let console_limit = host
                    .console
                    .cvars
                    .integer_in(maxclients.canonical(), context.source);
                assert_eq!(native_limit, 1);
                assert!(console_limit > 1);
            } else {
                host.console.cvars.force_write(maxclients, "2").unwrap();
            }
            host.initialize_modules(Phase::Server);
            let result = host.connect_q2_client(&api, id, b"\\name\\NativeClient");
            assert_eq!(
                result.is_ok(),
                admitted,
                "{rules:?}, independent console {independent_console}: {result:?}"
            );
            if admitted {
                assert_eq!(result.unwrap(), 1);
                let client = &host.runtime.server.clients[id.0 as usize];
                assert_eq!(
                    host.runtime.server.entities.columns.native_entity[client.entity.slot as usize],
                    Some(NativeEntity {
                        module: ModuleId(1),
                        slot: 2
                    })
                );
                assert_eq!(client.player.movement_rules, RuleSetId::Quake3);
                assert_eq!(client.player.trace_rules, RuleSetId::Quake);
                assert_eq!(
                    (
                        client.player.health,
                        client.player.armor,
                        client.player.frags
                    ),
                    (90, 41, -7)
                );
                let stats = &host.runtime.catalog.hud.values.native(rules).stats;
                for (binding, &value) in stats.iter().zip(&native_stats) {
                    assert_eq!(
                        binding.export(&client.player.values),
                        Some(u32::from(value as u16))
                    );
                    assert_eq!(
                        client
                            .player
                            .values
                            .get(binding.id)
                            .map(|value| value.as_integer()),
                        Some(i32::from(value))
                    );
                }
                let body = client.player.body;
                let before = host.module_counts(api.module).unwrap().calls;
                let mut invalid_api = api;
                invalid_api.module = ModuleId(2);
                let error = host
                    .think_q2_client(&invalid_api, id, &Default::default(), 0)
                    .unwrap_err();
                assert_eq!(
                    error_text(error).as_str(),
                    "command targets another native module"
                );
                let mut invalid_api = api;
                invalid_api.usercmd = u64::MAX;
                let error = host
                    .think_q2_client(&invalid_api, id, &Default::default(), 0)
                    .unwrap_err();
                assert_eq!(error_text(error).as_str(), "native usercmd: Memory");
                let mut invalid_api = api;
                invalid_api.think = CallbackId(u32::MAX);
                let error = host
                    .think_q2_client(&invalid_api, id, &Default::default(), 0)
                    .unwrap_err();
                assert_eq!(error_text(error).as_str(), "ClientThink: MissingFunction");
                assert_eq!(host.module_counts(api.module).unwrap().calls, before);
                for command in [
                    qa_core::primitives::UserCmd {
                        duration_ms: 300,
                        view_angles: qa_core::primitives::Vec3([90.0, -90.0, 180.0]),
                        movement: [123.75, -456.5, 200.0],
                        buttons: qa_core::primitives::buttons::ATTACK
                            | qa_core::primitives::buttons::USE
                            | qa_core::primitives::buttons::ANY,
                        impulse: 17,
                        light_level: 42,
                        ..Default::default()
                    },
                    qa_core::primitives::UserCmd::default(),
                ]
                .into_iter()
                .enumerate()
                {
                    let (index, command) = command;
                    host.think_q2_client(&api, id, &command, 0xfedcba98)
                        .unwrap();
                    let client = &host.runtime.server.clients[id.0 as usize];
                    assert_eq!(client.player.body.position, body.position);
                    assert_eq!(client.player.body.velocity, body.velocity);
                    assert_eq!(client.player.body.mins, body.mins);
                    assert_eq!(client.player.body.maxs, body.maxs);
                    assert_eq!(client.player.movement_rules, RuleSetId::Quake3);
                    assert_eq!(client.player.trace_rules, RuleSetId::Quake);
                    assert_eq!(client.player.health, 91 + index as i32);
                    assert_eq!((client.player.armor, client.player.frags), (41, -7));
                    let binding = host.runtime.catalog.hud.values.native(rules).stats[1];
                    assert_eq!(
                        binding.export(&client.player.values),
                        Some((91 + index) as u32)
                    );
                }
            } else if independent_console {
                assert_eq!(
                    result.unwrap_err(),
                    "native client outside module maxclients"
                );
                let client = &host.runtime.server.clients[id.0 as usize];
                assert!(client.connection.is_some());
                assert_eq!(
                    host.runtime.server.entities.columns.native_entity[client.entity.slot as usize],
                    None
                );
                assert!(host.runtime.server.module_moves(id).is_none());
                assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, 1);
            } else {
                assert!(
                    host.runtime.server.clients[id.0 as usize]
                        .connection
                        .is_none()
                );
                assert_eq!(host.module_state(ModuleId(1)), Some(State::Running));
            }
            host.shutdown_modules();
            let calls = if admitted {
                7
            } else if independent_console {
                2
            } else {
                3
            };
            assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, calls);
            assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 0);
            host.shutdown_modules();
            assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, calls);
            let error = host
                .think_q2_client(&api, id, &Default::default(), 0)
                .unwrap_err();
            assert_eq!(
                error_text(error).as_str(),
                "client has no active native movement binding"
            );
            assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, calls);
        }
    }
}

fn native_snap_imports_execute_through_session_dispatch() {
    use qa_compat::abi::Q3_CLIENT;
    use qa_console::views::Role;
    for (imports, ordinal, phase, role) in [
        (&Q3_SERVER, 42, Phase::Server, Role::Game),
        (&Q3_CLIENT, 71, Phase::Client, Role::Cgame),
    ] {
        for (input, expected) in [
            ([0.5, 1.5, 2.5], [0.0f32, 2.0, 2.0]),
            ([-0.5, -1.5, -2.5], [0.0, -2.0, -2.0]),
            ([-0.0, f32::from_bits(1), -f32::from_bits(1)], [0.0; 3]),
            (
                [f32::NAN, f32::INFINITY, f32::NEG_INFINITY],
                [i32::MIN as f32; 3],
            ),
        ] {
            let image = function_image(Encoding::Pe, &[0xc3]);
            let base = image.base;
            let entry = image.symbol(b"GetGameAPI").unwrap().address;
            let vector = base + 0x1700;
            let mut vm = Vm::map_image(
                image,
                &[NamedExport {
                    name: b"GetGameAPI",
                    command: None,
                    parameters: &[NativeScalar::Word; 13],
                    result: NativeScalar::Word,
                }],
                &[],
                Duration::from_secs(3),
            )
            .unwrap();
            let mut code =
                native_import_code(vm.import_callback(), &[ordinal, vector], false, &[], 0);
            assert_eq!(code.pop(), Some(0xc3));
            code.extend([0x48, 0xb8]); // mov rax,vector
            code.extend(vector.to_le_bytes());
            let mut jumps = Vec::new();
            for (axis, value) in expected.into_iter().enumerate() {
                code.extend([0x81, 0x78, axis as u8 * 4]); // cmp dword [rax+offset],bits
                code.extend(value.to_bits().to_le_bytes());
                code.extend([0x75, 0]); // jne mismatch
                jumps.push(code.len() - 1);
            }
            code.extend([0x31, 0xc0, 0xc3]); // return zero when all components match
            let mismatch = code.len();
            code.extend([0xb8, 1, 0, 0, 0, 0xc3]);
            for jump in jumps {
                code[jump] = u8::try_from(mismatch - jump - 1).unwrap();
            }
            let mut memory =
                qa_compat::memory::ModuleMemory::borrow(base, vm.process.memory_mut().unwrap())
                    .unwrap();
            memory.write(entry, &code).unwrap();
            memory
                .write_vec3(vector, qa_core::primitives::Vec3(input))
                .unwrap();
            let mut runtime = Runtime::load(1, []).unwrap();
            let mut request = request(
                &mut runtime,
                ModuleId(1),
                RuleSetId::Quake3,
                TickRate::fixed(50).unwrap(),
                vm,
            );
            request.phase = phase;
            request.context.console.role = role;
            if let Program::Native { imports: table, .. } = &mut request.program {
                *table = imports;
            }
            let mut host = FrameHost::load_modules(
                Console::new(Context::default()).unwrap(),
                runtime,
                TickRate::FrameDriven,
                vec![request],
            )
            .unwrap();
            host.call_module_export(
                ModuleId(1),
                Export::words(CallbackId(0), &[]),
                ThinkTime::Milliseconds(0),
            )
            .unwrap();
            let counts = host.module_counts(ModuleId(1)).unwrap();
            assert_eq!(counts.last_result, Some(ModuleResult::Native(0)));
            assert_eq!(counts.traps, 0);
        }
    }
}

fn q2_api_layouts_bind_the_full_table_and_name_missing_engine_services() {
    use qa_compat::native::{NativeCalls, q2::Game};
    use qa_compat::{abi::UnknownCalls, services::ServiceStorage};
    for rules in [RuleSetId::Quake2, RuleSetId::Quake2Rerelease] {
        let rr = rules == RuleSetId::Quake2Rerelease;
        let mut file = q2_table_file();
        let version = if rr { 2023 } else { 3 };
        pe_rva(&mut file, 0x1800, version, 4);
        pe_text(&mut file, 0x1500, &[0xc3]);
        let count = if rr { 29 } else { 15 };
        for slot in 0..count {
            let offset = if !rr || slot < 19 {
                8 + slot * 8
            } else {
                192 + (slot - 19) * 8
            };
            pe_rva(
                &mut file,
                0x1800 + offset,
                if rr && offset == 192 {
                    0x180001710
                } else {
                    0x180001500
                },
                8,
            );
            if rr && offset == 192 {
                pe_text(&mut file, 0x1710, &[0xc3, 0x90, 0x90, 0x90, 0x90]);
            }
        }
        let entity_offset = if rr { 160 } else { 128 };
        let stride = if rr { 1472 } else { 280 };
        pe_rva(&mut file, 0x1800 + entity_offset, 0x180003000, 8);
        pe_rva(
            &mut file,
            0x1800 + entity_offset + 8,
            stride,
            if rr { 8 } else { 4 },
        );
        let size = if rr { 16 } else { 12 };
        pe_rva(&mut file, 0x1800 + entity_offset + size, 2, 4);
        pe_rva(&mut file, 0x1800 + entity_offset + size + 4, 4, 4);
        if rr {
            pe_rva(&mut file, 0x1800 + entity_offset + 24, 5, 4);
        }
        let mut image = Image::parse(&file, None, LoadRole::Library).unwrap();
        let entity_bytes = (stride as usize * 4).div_ceil(4096) * 4096;
        let mut bytes = std::mem::take(&mut image.bytes).into_vec();
        bytes.resize(0x3000 + entity_bytes, 0);
        image.bytes = bytes.into_boxed_slice();
        let mut regions = std::mem::take(&mut image.regions).into_vec();
        regions.push(qa_formats::program::native::Region {
            offset: 0x3000,
            length: entity_bytes,
            read: true,
            write: true,
            execute: false,
        });
        image.regions = regions.into_boxed_slice();
        let mut runtime = Runtime::load(1, std::iter::empty()).unwrap();
        let sound_observer = runtime
            .server
            .events
            .bind(OutputTarget::Module(ModuleId(90)))
            .unwrap();
        let bounds = qa_core::primitives::Bounds {
            mins: qa_core::primitives::Vec3([-1000.0; 3]),
            maxs: qa_core::primitives::Vec3([1000.0; 3]),
        };
        let geometry = runtime
            .geometry
            .load_brushes(
                vec![],
                vec![],
                qa_world::collision::surfaces::SurfaceTable::flags(vec![]),
                qa_world::collision::brushes::BrushTree {
                    planes: vec![],
                    nodes: vec![],
                    leaves: vec![qa_world::collision::brushes::CollisionLeaf {
                        stored_contents: None,
                        brushes: qa_core::primitives::IndexSpan { first: 0, count: 0 },
                    }],
                    leaf_brushes: vec![],
                    models: vec![qa_world::collision::brushes::ModelRoot::Leaf(0); 2],
                },
                vec![bounds; 2],
            )
            .unwrap();
        let visibility = qa_world::visibility::VisibilityWorld::load(
            vec![qa_core::primitives::Plane::oriented(
                qa_core::primitives::Vec3([1.0, 0.0, 0.0]),
                0.0,
            )],
            vec![qa_world::visibility::VisNode {
                plane: 0,
                children: [-1, -2],
                bounds,
                surfaces: qa_world::visibility::SurfaceSpan::default(),
            }],
            [7, 9]
                .map(|area| qa_world::visibility::VisLeaf {
                    selector: Some(5),
                    area: Some(area),
                    solid: false,
                    bounds,
                    surfaces: qa_world::visibility::SurfaceSpan::default(),
                })
                .to_vec(),
            vec![],
            0,
            0,
            qa_world::visibility::PvsRows::load(vec![Some(0); 6], vec![0, 1, 32])
                .unwrap()
                .with_hearing(vec![Some(2); 6])
                .unwrap(),
        )
        .unwrap();
        runtime.collision = Some(
            qa_app::WorldCollision::new(&runtime.geometry, geometry, 0)
                .with_visibility(std::sync::Arc::new(visibility))
                .with_portals(Some(
                    qa_world::portals::AreaPortals::load(
                        10,
                        3,
                        vec![qa_world::portals::Portal {
                            number: 2,
                            first: 7,
                            second: 9,
                        }],
                    )
                    .unwrap(),
                )),
        );
        let mut game =
            Game::map(image, rules, 25, &runtime.geometry, Duration::from_secs(3)).unwrap();
        assert!(game.entities().is_err());
        let imports = game.imports_address;
        let base = game.vm.process.base();
        let first = (imports - base) as usize;
        let header = if rr { 16 } else { 0 };
        let service_count = if rr { 70 } else { 44 };
        let pointers = (0..service_count)
            .map(|n| game.vm.process.import_pointer(n).unwrap())
            .collect::<Vec<_>>();
        let write_slots = [
            b"WriteChar".as_slice(),
            b"WriteByte",
            b"WriteShort",
            b"WriteLong",
            b"WriteFloat",
            b"WriteString",
        ]
        .map(|name| game.import_ordinal(name).unwrap());
        assert_eq!(
            write_slots,
            if rr {
                [26, 27, 28, 29, 30, 31]
            } else {
                [24, 25, 26, 27, 28, 29]
            }
        );
        let vector_slots = [b"WritePosition".as_slice(), b"WriteDir", b"WriteAngle"]
            .map(|name| game.import_ordinal(name).unwrap());
        assert_eq!(vector_slots, if rr { [32, 33, 34] } else { [30, 31, 32] });
        let entity_slot = rr.then(|| game.import_ordinal(b"WriteEntity").unwrap());
        assert_eq!(entity_slot, rr.then_some(35));
        let unicast_slot = game.import_ordinal(b"unicast").unwrap();
        assert_eq!(unicast_slot, if rr { 25 } else { 23 });
        assert!(
            !game
                .vm
                .unresolved_imports()
                .any(|i| { matches!(i.name, Some(b"unicast" | b"multicast")) })
        );
        let memory = game.vm.process.memory_mut().unwrap();
        if rr {
            assert_eq!(&memory[first..first + 4], &40u32.to_le_bytes());
            assert_eq!(&memory[first + 4..first + 8], &0.025f32.to_le_bytes());
            assert_eq!(&memory[first + 8..first + 12], &25u32.to_le_bytes());
            assert_eq!(&memory[first + 12..first + 16], &[0; 4]);
        }
        for (slot, pointer) in pointers.iter().enumerate() {
            let at = first + header + slot * 8;
            assert_eq!(&memory[at..at + 8], &pointer.to_le_bytes());
        }
        let bound = runtime
            .server
            .entities
            .allocate(
                ThinkTime::Milliseconds(0),
                ModuleId(1),
                AllocationPolicy::EDICT,
            )
            .unwrap()
            .id;
        runtime.server.entities.columns.native_entity[bound.slot as usize] =
            Some(qa_core::primitives::NativeEntity {
                module: ModuleId(1),
                slot: 1,
            });
        let order = qa_gameplay::rules::link_order(rules);
        assert!(runtime.server.area.link(
            &runtime.server.entities,
            bound,
            qa_world::area::LinkFlags::SOLID,
            order,
            qa_world::area::LinkIntent::Explicit
        ));
        if rr {
            assert!(runtime.server.navigation.register(bound));
        }
        let observer = runtime
            .server
            .events
            .bind(OutputTarget::Module(ModuleId(7)))
            .unwrap();
        let mut console = Console::new(Context::default()).unwrap();
        let mut storage = ServiceStorage::load(
            &[(ModuleId(1), if rr { 10814 } else { 800 })],
            0,
            &console.cvars,
        )
        .unwrap();
        let mut scratch = runtime.geometry.scratch();
        let mut unknown = UnknownCalls::load(8).unwrap();
        let context = CallContext {
            native_rules: rules,
            trace_rules: rules,
            sound_rules: rules,
            server_frame: 0x1_1234_5678,
            module: ModuleId(1),
            clock: ThinkTime::Milliseconds(0),
            console: Context {
                source: rules,
                role: qa_console::views::Role::Game,
                ..Context::default()
            },
            allocation: AllocationPolicy::EDICT,
            link_order: qa_gameplay::rules::link_order(rules),
        };
        // GetGameAPI may use world services before publishing its edict
        // table. A missing native view must not reject that valid query.
        let mut factory = native_import_code(
            pointers[if rr { 16 } else { 13 }],
            &[base + 0x1df0],
            true,
            &[],
            base + 0x1de0,
        );
        factory.pop();
        factory.extend([0x48, 0xb8]);
        factory.extend((base + 0x1800).to_le_bytes());
        factory.push(0xc3);
        let factory_at = 0x1080;
        game.vm.process.memory_mut().unwrap()[factory_at..factory_at + factory.len()]
            .copy_from_slice(&factory);
        let returned = {
            let mut services = runtime.engine_services(&mut console, &mut storage, &mut scratch);
            game.vm
                .call(
                    &mut NativeCalls {
                        native_player: None,
                        services: &mut services,
                        table: game.imports,
                        context,
                        platform_time: EventTime(0),
                        command: &[],
                        raw_args: b"",
                        unknown: &mut unknown,
                    },
                    0,
                    &[imports],
                )
                .unwrap()
        };
        assert_eq!(
            &game.vm.process.memory_mut().unwrap()[0x1de0..0x1de8],
            &[0; 8]
        );
        let target = 0x180001600u64;
        let frame_offset = if rr { 136 } else { 112 };
        game.vm.process.memory_mut().unwrap()[0x1800 + frame_offset..0x1808 + frame_offset]
            .copy_from_slice(&target.to_le_bytes());
        game.vm.bind_table(returned).unwrap();
        let entities = game.entities().unwrap();
        assert_eq!(entities.address, base + 0x3000);
        assert_eq!(
            (
                entities.stride,
                entities.count,
                entities.capacity,
                entities.server_flags
            ),
            (stride, 2, 4, if rr { 5 } else { 0 })
        );
        let table_at = (returned - base) as usize + entity_offset as usize;
        for (at, value) in [
            (table_at + size as usize, 5u32),
            (table_at + size as usize + 4, u32::MAX),
        ] {
            let saved = game.vm.process.memory_mut().unwrap()[at..at + 4].to_vec();
            game.vm.process.memory_mut().unwrap()[at..at + 4].copy_from_slice(&value.to_le_bytes());
            assert!(game.entities().is_err());
            game.vm.process.memory_mut().unwrap()[at..at + 4].copy_from_slice(&saved);
        }
        let run = game.entry(b"RunFrame").unwrap();
        assert_eq!(run, if rr { 17 } else { 14 });
        assert_eq!(game.entry(b"Pmove").is_some(), rr);
        assert_eq!(game.entry(b"unknown"), None);
        // Force the real loaded API frame to reach a named engine trap. No
        // guessed argument signature is read, and instructions after it stop.
        let target = 0x180001600u64;
        let trap_name = if rr {
            b"GetPathToGoal".as_slice()
        } else {
            b"DebugGraph".as_slice()
        };
        let pointer = pointers[if rr { 52 } else { 43 }];
        let code_at = (target - base) as usize;
        let mut code = vec![0x48, 0x83, 0xec, 0x28];
        code.extend([0x48, 0xb9]);
        code.extend((base + 0x1700).to_le_bytes());
        if !rr {
            // Classic dprintf is variadic, unlike API2023's plain string.
            code.extend([0xba, 7, 0, 0, 0]);
        }
        code.extend([0x48, 0xb8]);
        code.extend(pointers[1].to_le_bytes());
        code.extend([0xff, 0xd0]);
        let text = if rr {
            b"API 2023 print\0".as_slice()
        } else {
            b"API classic print %d\xff\0"
        };
        game.vm.process.memory_mut().unwrap()[0x1700..0x1700 + text.len()].copy_from_slice(text);
        code.extend([0x48, 0xb8]);
        code.extend(pointer.to_le_bytes());
        code.extend([0xff, 0xd0, 0x0f, 0x0b]);
        game.vm.process.memory_mut().unwrap()[code_at..code_at + code.len()].copy_from_slice(&code);
        for _ in 0..3 {
            let mut services = runtime.engine_services(&mut console, &mut storage, &mut scratch);
            let error = game
                .vm
                .call(
                    &mut NativeCalls {
                        native_player: None,
                        services: &mut services,
                        table: game.imports,
                        context,
                        platform_time: EventTime(0),
                        command: &[],
                        raw_args: b"",
                        unknown: &mut unknown,
                    },
                    run,
                    &[1],
                )
                .unwrap_err();
            assert!(error.recoverable());
        }
        let import = game
            .vm
            .unresolved_imports()
            .find(|i| i.name == Some(trap_name))
            .unwrap();
        assert_eq!(import.calls, 3);
        let mut batch = runtime.server.events.batch(observer).unwrap();
        assert!(runtime.server.events.next(&mut batch).is_none());
        let mut batch = runtime
            .server
            .events
            .batch(runtime.server.presentation)
            .unwrap();
        let mut logs = 0;
        let mut prints = 0;
        while let Some(record) = runtime.server.events.next(&mut batch) {
            if let FrameEvent::Print(print) = record.event {
                let text = runtime.server.events.texts.get(print.text).unwrap();
                if text
                    == if rr {
                        b"API 2023 print".as_slice()
                    } else {
                        b"API classic print 7\xff"
                    }
                {
                    prints += 1;
                } else {
                    assert!(text.windows(trap_name.len()).any(|s| s == trap_name));
                    logs += 1;
                }
            }
        }
        assert_eq!(logs, 1);
        assert_eq!(prints, 3);
        // Reach the actual declared cvar import through the owned child.
        let name = b"_qa_child_cvar\0";
        let default = b"3\0";
        game.vm.process.memory_mut().unwrap()[0x1720..0x1720 + name.len()].copy_from_slice(name);
        game.vm.process.memory_mut().unwrap()[0x1760..0x1760 + default.len()]
            .copy_from_slice(default);
        let mut code = vec![0x48, 0x83, 0xec, 0x28, 0x48, 0xb9];
        code.extend((base + 0x1720).to_le_bytes());
        code.extend([0x48, 0xba]);
        code.extend((base + 0x1760).to_le_bytes());
        code.extend([0x41, 0xb8]);
        code.extend(20u32.to_le_bytes());
        code.extend([0x48, 0xb8]);
        code.extend(pointers[if rr { 39 } else { 36 }].to_le_bytes());
        code.extend([0xff, 0xd0, 0x48, 0xb9]);
        code.extend((base + 0x1c00).to_le_bytes());
        code.extend([0x48, 0x89, 1, 0x48, 0x83, 0xc4, 0x28, 0xc3]);
        game.vm.process.memory_mut().unwrap()[code_at..code_at + code.len()].copy_from_slice(&code);
        let mut saved_pointer = None;
        for text in ["3", "7"] {
            if text == "7" {
                let view = console
                    .cvars
                    .bind("_qa_child_cvar", context.console)
                    .unwrap();
                console.cvars.force_write(view, text).unwrap();
            }
            let mut services = runtime.engine_services(&mut console, &mut storage, &mut scratch);
            game.vm
                .call(
                    &mut NativeCalls {
                        native_player: None,
                        services: &mut services,
                        table: game.imports,
                        context,
                        platform_time: EventTime(0),
                        command: &[],
                        raw_args: b"",
                        unknown: &mut unknown,
                    },
                    run,
                    &[1],
                )
                .unwrap();
            let memory = qa_compat::memory::ModuleMemory::borrow(
                base,
                game.vm.process.memory_mut().unwrap(),
            )
            .unwrap();
            let address =
                u64::from_le_bytes(memory.read(base + 0x1c00, 8).unwrap().try_into().unwrap());
            assert_eq!(*saved_pointer.get_or_insert(address), address);
            let string =
                u64::from_le_bytes(memory.read(address + 8, 8).unwrap().try_into().unwrap());
            assert_eq!(memory.cstring(string).unwrap(), text.as_bytes());
            assert_eq!(memory.read_word(address + 24).unwrap(), 20);
            assert_eq!(
                memory.read_word(address + 28).unwrap(),
                if rr && text == "7" { 2 } else { 1 }
            );
            assert_eq!(
                memory.read_word(address + 32).unwrap() as u32,
                text.parse::<f32>().unwrap().to_bits()
            );
            if rr {
                assert_eq!(
                    memory.read_word(address + 48).unwrap(),
                    text.parse::<i32>().unwrap()
                );
            }
        }
        let malloc_slot = if rr { 36 } else { 33 };
        let link_slot = if rr { 21 } else { 18 };
        let set_model_slot = if rr { 13 } else { 11 };
        let unlink_slot = if rr { 22 } else { 19 };
        let entity_address = base + 0x3000 + stride;
        let origin = qa_core::primitives::Vec3([3.0, 5.0, 7.0]);
        let mut memory =
            qa_compat::memory::ModuleMemory::borrow(base, game.vm.process.memory_mut().unwrap())
                .unwrap();
        memory.write_vec3(entity_address + 4, origin).unwrap();
        memory
            .write_vec3(
                entity_address + if rr { 1396 } else { 204 },
                qa_core::primitives::Vec3([-16.0, -16.0, -24.0]),
            )
            .unwrap();
        memory
            .write_vec3(
                entity_address + if rr { 1408 } else { 216 },
                qa_core::primitives::Vec3([16.0, 16.0, 32.0]),
            )
            .unwrap();
        if rr {
            memory.write(entity_address + 1376, &[1]).unwrap();
            memory.write(entity_address + 1456, &[2]).unwrap();
        } else {
            memory.write_word(entity_address + 96, 1).unwrap();
            memory.write_word(entity_address + 264, 2).unwrap();
        }
        drop(memory);
        if rr {
            game.vm.process.memory_mut().unwrap()[(entity_address - base) as usize + 1377] = 1;
        }
        let index_slot = if rr { 10 } else { 8 };
        let mut replacement = None;
        let mut observer_entity = None;
        let mut owned_entity = None;
        let trace_slot = if rr { 14 } else { 12 };
        let area_slot = if rr { 23 } else { 20 };
        let contents_slot = if rr { 16 } else { 13 };
        let portal_slot = if rr { 19 } else { 16 };
        let connected_slot = if rr { 20 } else { 17 };
        let argc_slot = game.import_ordinal(b"argc").unwrap();
        let argv_slot = game.import_ordinal(b"argv").unwrap();
        let args_slot = game.import_ordinal(b"args").unwrap();
        let append_slot = game.import_ordinal(b"AddCommandString").unwrap();
        assert_eq!(
            (argc_slot, argv_slot, args_slot, append_slot),
            if rr {
                (42, 43, 44, 45)
            } else {
                (39, 40, 41, 42)
            }
        );
        // These writes execute in the owned child immediately before the
        // import, after the previous export projection and without LinkEntity.
        let live_edits = std::cell::RefCell::new(Vec::<(u64, Vec<u8>)>::new());
        let mut invoke_import = |game: &mut Game, slot: usize, a: &[u64]| {
            let returns_value = slot == malloc_slot
                || slot == trace_slot
                || (rr && (slot == 8 || slot == 15))
                || slot == area_slot
                || slot == contents_slot
                || slot == connected_slot
                || slot == argc_slot
                || slot == argv_slot
                || slot == args_slot
                || (if rr { 17..=18 } else { 14..=15 }).contains(&slot)
                || (rr && (67..=69).contains(&slot))
                || (rr && slot == 65)
                || (index_slot..index_slot + 3).contains(&slot);
            let mut code = native_import_code(
                pointers[slot],
                a,
                returns_value,
                if slot == write_slots[4] || slot == vector_slots[2] {
                    &[0]
                } else {
                    &[]
                },
                base + 0x1c30,
            );
            let mut prefix = Vec::new();
            for (address, bytes) in live_edits.borrow_mut().drain(..) {
                for (index, bytes) in bytes.chunks(4).enumerate() {
                    prefix.extend([0x48, 0xb9]);
                    prefix.extend((address + (index * 4) as u64).to_le_bytes());
                    if bytes.len() == 4 {
                        prefix.extend([0xc7, 0x01]);
                        prefix.extend(bytes);
                    } else {
                        assert_eq!(bytes.len(), 1);
                        prefix.extend([0xc6, 0x01, bytes[0]]);
                    }
                }
            }
            prefix.append(&mut code);
            code = prefix;
            if slot == argv_slot || slot == args_slot || (rr && slot == 8) {
                // Dereference the returned C string in the child as well as inspecting
                // the authoritative shared bytes in the parent.
                let mut read = vec![0x0f, 0xb6, 0x00, 0x48, 0xb9];
                read.extend((base + 0x1c38).to_le_bytes());
                read.extend([0x88, 1]);
                let at = code.len() - 5;
                code.splice(at..at, read);
            }
            game.vm.process.memory_mut().unwrap()[code_at..code_at + code.len()]
                .copy_from_slice(&code);
            let mut services = runtime.engine_services(&mut console, &mut storage, &mut scratch);
            let before_spatial = (slot == trace_slot || slot == contents_slot)
                .then(|| services.server.area.bounds(bound));
            let before_clip = (rr && slot == 15).then(|| {
                (
                    services.server.entities.len(),
                    services.server.entities.columns.position[bound.slot as usize],
                    services.server.area.bounds(bound),
                )
            });
            let before_config = (slot == if rr { 7 } else { 6 } && a[0] == 3).then(|| {
                let (value, revision) = services.storage.configstring(ModuleId(1), 3).unwrap();
                (value.to_vec(), revision)
            });
            game.vm
                .call(
                    &mut NativeCalls {
                        native_player: None,
                        services: &mut services,
                        table: game.imports,
                        context,
                        platform_time: EventTime(0),
                        command: &[b"_qa_native_command", b"\x80", b"third"],
                        raw_args: if rr {
                            b"\"\x80\"   third ".as_slice()
                        } else {
                            b"\"\x80\"   third"
                        },
                        unknown: &mut unknown,
                    },
                    run,
                    &[1],
                )
                .unwrap();
            if let Some(bounds) = before_spatial {
                assert_eq!(services.server.area.bounds(bound), bounds);
            }
            if let Some((count, position, bounds)) = before_clip {
                assert_eq!(services.server.entities.len(), count);
                assert_eq!(
                    services.server.entities.columns.position[bound.slot as usize],
                    position
                );
                assert_eq!(services.server.area.bounds(bound), bounds);
            }
            if slot == link_slot && a[0] == entity_address {
                let bounds = services.server.area.bounds(bound).unwrap();
                assert_eq!(
                    bounds.mins,
                    qa_core::primitives::Vec3([-14.0, -12.0, -18.0])
                );
                assert_eq!(bounds.maxs, qa_core::primitives::Vec3([20.0, 22.0, 40.0]));
                assert_eq!(
                    services.server.entities.columns.position[bound.slot as usize],
                    origin
                );
                assert_eq!(
                    services.server.entities.columns.collision_shape[bound.slot as usize],
                    qa_core::primitives::CollisionShape::Box
                );
                assert_eq!(
                    services.server.entities.columns.collision_contents[bound.slot as usize],
                    qa_world::collision::Contents::BODY.0
                );
                let memory = qa_compat::memory::ModuleMemory::borrow(
                    base,
                    game.vm.process.memory_mut().unwrap(),
                )
                .unwrap();
                assert_eq!(
                    memory
                        .read_vec3(entity_address + if rr { 1420 } else { 228 })
                        .unwrap(),
                    bounds.mins
                );
                assert_eq!(
                    memory
                        .read_vec3(entity_address + if rr { 1432 } else { 240 })
                        .unwrap(),
                    bounds.maxs
                );
                assert_eq!(
                    memory
                        .read_vec3(entity_address + if rr { 1444 } else { 252 })
                        .unwrap(),
                    qa_core::primitives::Vec3([32.0, 32.0, 56.0])
                );
                assert_eq!(memory.read_vec3(entity_address + 28).unwrap(), origin);
                assert_eq!(
                    memory
                        .read_word(entity_address + if rr { 1384 } else { 192 })
                        .unwrap(),
                    7
                );
                assert_eq!(
                    memory
                        .read_word(entity_address + if rr { 1388 } else { 196 })
                        .unwrap(),
                    9
                );
                assert_eq!(
                    memory
                        .read_word(entity_address + if rr { 76 } else { 72 })
                        .unwrap() as u32,
                    if rr { 0x40181010 } else { 2 | 3 << 5 | 8 << 10 }
                );
                if rr {
                    assert_eq!(memory.read(entity_address + 1377, 1).unwrap(), &[1]);
                } else {
                    assert_eq!(memory.read_word(entity_address + 120).unwrap(), 1);
                    assert_eq!(memory.read_word(entity_address + 124).unwrap(), 5);
                }
            }
            if slot == link_slot && a[0] == entity_address + stride {
                let entity = services
                    .server
                    .entities
                    .active()
                    .find(|entity| {
                        services.server.entities.columns.native_entity[entity.slot as usize]
                            == Some(qa_core::primitives::NativeEntity {
                                module: ModuleId(1),
                                slot: 2,
                            })
                    })
                    .unwrap();
                if a[1] == 1 {
                    let old = owned_entity.unwrap();
                    assert_ne!(old, entity);
                    assert!(services.server.entities.resolve(old).is_none());
                    assert!(!services.server.navigation.contains(old));
                } else if let Some(old) = owned_entity {
                    assert_eq!(old, entity);
                }
                owned_entity = Some(entity);
            }
            if slot == link_slot && a[0] == entity_address + stride * 2 {
                let entity = services
                    .server
                    .entities
                    .active()
                    .find(|entity| {
                        services.server.entities.columns.native_entity[entity.slot as usize]
                            == Some(qa_core::primitives::NativeEntity {
                                module: ModuleId(1),
                                slot: 3,
                            })
                    })
                    .unwrap();
                assert_eq!(
                    services.server.entities.columns.collision_shape[entity.slot as usize],
                    qa_core::primitives::CollisionShape::Model { geometry, index: 1 }
                );
                let absolute = services.server.area.bounds(entity).unwrap();
                assert_eq!(
                    absolute.mins,
                    qa_core::primitives::Vec3([-30.0, -28.0, -26.0])
                );
                assert_eq!(absolute.maxs, qa_core::primitives::Vec3([36.0, 38.0, 40.0]));
                let memory = qa_compat::memory::ModuleMemory::borrow(
                    base,
                    game.vm.process.memory_mut().unwrap(),
                )
                .unwrap();
                let address = entity_address + stride * 2;
                assert_eq!(
                    memory
                        .read_word(address + if rr { 76 } else { 72 })
                        .unwrap(),
                    31
                );
                assert_eq!(
                    memory.read_vec3(address + 28).unwrap(),
                    if rr {
                        qa_core::primitives::Vec3([99.0; 3])
                    } else {
                        origin
                    }
                );
            }
            if slot == set_model_slot {
                let memory = qa_compat::memory::ModuleMemory::borrow(
                    base,
                    game.vm.process.memory_mut().unwrap(),
                )
                .unwrap();
                assert_eq!(
                    memory.read_word(a[0] + 40).unwrap(),
                    if a[1] == base + 0x17a0 { 2 } else { 1 }
                );
                if a[1] == base + 0x17a0 {
                    assert_eq!(
                        memory
                            .read_vec3(a[0] + if rr { 1396 } else { 204 })
                            .unwrap(),
                        bounds.mins
                    );
                    assert_eq!(
                        memory
                            .read_vec3(a[0] + if rr { 1408 } else { 216 })
                            .unwrap(),
                        bounds.maxs
                    );
                    let entity = services
                        .server
                        .entities
                        .active()
                        .find(|entity| {
                            services.server.entities.columns.native_entity[entity.slot as usize]
                                == Some(qa_core::primitives::NativeEntity {
                                    module: ModuleId(1),
                                    slot: 3,
                                })
                        })
                        .unwrap();
                    assert_eq!(
                        services.server.entities.columns.collision_shape[entity.slot as usize],
                        qa_core::primitives::CollisionShape::Model { geometry, index: 1 }
                    );
                    let absolute = services.server.area.bounds(entity).unwrap();
                    assert_eq!(
                        absolute.mins,
                        qa_core::primitives::Vec3([-998.0, -996.0, -994.0])
                    );
                    assert_eq!(
                        absolute.maxs,
                        qa_core::primitives::Vec3([1004.0, 1006.0, 1008.0])
                    );
                    assert_eq!(
                        memory
                            .read_word(a[0] + if rr { 1380 } else { 100 })
                            .unwrap(),
                        3
                    );
                } else {
                    // Non-inline registration changes only the published model.
                    assert_eq!(
                        memory
                            .read_word(a[0] + if rr { 1380 } else { 100 })
                            .unwrap(),
                        2
                    );
                }
            }
            if let Some((before, revision)) = before_config {
                let memory = qa_compat::memory::ModuleMemory::borrow(
                    base,
                    game.vm.process.memory_mut().unwrap(),
                )
                .unwrap();
                let value = if a[1] == 0 {
                    &[][..]
                } else {
                    memory.cstring(a[1]).unwrap()
                };
                let value = &value[..value.len().min(if rr { 95 } else { 8192 })];
                assert_eq!(
                    services.storage.configstring(ModuleId(1), 3).unwrap(),
                    (value, revision + u64::from(before != value))
                );
            }
            if rr && slot == 49 {
                assert!(!services.server.navigation.contains(bound));
                if let Some(entity) = observer_entity {
                    assert!(!services.server.navigation.contains(entity));
                }
            }
            if rr && slot == 48 {
                let entity = services
                    .server
                    .entities
                    .active()
                    .find(|entity| {
                        services.server.entities.columns.native_entity[entity.slot as usize]
                            == Some(qa_core::primitives::NativeEntity {
                                module: ModuleId(1),
                                slot: 2,
                            })
                    })
                    .unwrap();
                assert_eq!(*observer_entity.get_or_insert(entity), entity);
                assert!(services.server.navigation.contains(entity));
                assert_eq!(
                    services.server.entities.columns.owner[entity.slot as usize],
                    ModuleId(1)
                );
            }
            if slot == unlink_slot && a[1] == 1 {
                assert!(!services.server.area.unlink(bound));
                assert!(
                    services
                        .server
                        .entities
                        .release(bound, ThinkTime::Seconds(3.0))
                );
                let next = services
                    .server
                    .entities
                    .allocate(
                        ThinkTime::Seconds(4.0),
                        ModuleId(2),
                        AllocationPolicy::EDICT,
                    )
                    .unwrap()
                    .id;
                assert_eq!(next.slot, bound.slot);
                assert_ne!(next.generation, bound.generation);
                services.server.entities.columns.native_entity[next.slot as usize] =
                    Some(qa_core::primitives::NativeEntity {
                        module: ModuleId(2),
                        slot: 1,
                    });
                assert!(services.server.area.link(
                    &services.server.entities,
                    next,
                    qa_world::area::LinkFlags::SOLID,
                    order,
                    qa_world::area::LinkIntent::Explicit
                ));
                replacement = Some(next);
            }
            if returns_value {
                u64::from_le_bytes(
                    game.vm.process.memory_mut().unwrap()[0x1c30..0x1c38]
                        .try_into()
                        .unwrap(),
                )
            } else {
                0
            }
        };
        // API3 admits only the low 32 size bits; the rerelease keeps size_t.
        let bytes = if rr { 16 } else { (1u64 << 32) + 16 };
        let first = invoke_import(&mut game, malloc_slot, &[bytes, 765]);
        let first_at = (first - base) as usize;
        assert_eq!(
            &game.vm.process.memory_mut().unwrap()[first_at..first_at + 16],
            &[0; 16]
        );
        game.vm.process.memory_mut().unwrap()[first_at..first_at + 16].fill(77);
        let other = invoke_import(&mut game, malloc_slot, &[16, 766]);
        let other_at = (other - base) as usize;
        game.vm.process.memory_mut().unwrap()[other_at..other_at + 16].fill(88);
        invoke_import(&mut game, malloc_slot + 2, &[765, 0]);
        let next = invoke_import(&mut game, malloc_slot, &[16, 765]);
        assert_eq!(next, first);
        assert_eq!(
            &game.vm.process.memory_mut().unwrap()[first_at..first_at + 16],
            &[0; 16]
        );
        assert_eq!(
            &game.vm.process.memory_mut().unwrap()[other_at..other_at + 16],
            &[88; 16]
        );
        invoke_import(&mut game, malloc_slot + 1, &[other, 0]);
        invoke_import(&mut game, malloc_slot + 2, &[765, 0]);
        let whole = invoke_import(&mut game, malloc_slot, &[32, 0]);
        assert_eq!(whole, first);
        invoke_import(&mut game, malloc_slot + 2, &[0, 0]);
        let config_slot = if rr { 7 } else { 6 };
        if rr {
            assert_eq!(invoke_import(&mut game, 65, &[]), 0x1234_5678);
            let info = b"\\name\\Mike\\empty\\\\name\\later\\binary\\\xff\x80\0";
            for (key, capacity, expected, length) in [
                (b"name\0".as_slice(), 8, b"Mike\0".as_slice(), 4),
                (b"name\0", 3, b"Mi\0", 4),
                (b"name\0", 1, b"\0", 4),
                (b"name\0", 0, b"", 4),
                (b"NAME\0", 8, b"\0", 0),
                (b"empty\0", 8, b"\0", 0),
                (b"binary\0", 8, b"\xff\x80\0", 2),
            ] {
                let memory = game.vm.process.memory_mut().unwrap();
                memory[0x1e00..0x1e00 + info.len()].copy_from_slice(info);
                memory[0x1e40..0x1e40 + key.len()].copy_from_slice(key);
                memory[0x1e80..0x1e88].fill(0xcc);
                assert_eq!(
                    invoke_import(
                        &mut game,
                        67,
                        &[
                            base + 0x1e00,
                            base + 0x1e40,
                            if capacity == 0 { 0 } else { base + 0x1e80 },
                            capacity,
                        ]
                    ),
                    length
                );
                let memory = game.vm.process.memory().unwrap();
                assert_eq!(&memory[0x1e80..0x1e80 + expected.len()], expected);
                assert!(
                    memory[0x1e80 + expected.len()..0x1e88]
                        .iter()
                        .all(|&b| b == 0xcc)
                );
                assert_eq!(&memory[0x1e00..0x1e00 + info.len()], info);
            }
            let info_address = base + 0x2000;
            let info_bytes = qa_compat::q2_abi::client_layout(rules)
                .unwrap()
                .userinfo_bytes;
            let key_address = info_address + info_bytes as u64 + 64;
            let value_address = key_address + 128;
            let mut mutate_info = |game: &mut Game,
                                   set: bool,
                                   info: &[u8],
                                   key: &[u8],
                                   value: &[u8],
                                   accepted: u64,
                                   expected: &[u8]| {
                let mut memory = qa_compat::memory::ModuleMemory::borrow(
                    base,
                    game.vm.process.memory_mut().unwrap(),
                )
                .unwrap();
                memory.write_string(info_address, info_bytes, info).unwrap();
                memory.write_string(key_address, 128, key).unwrap();
                memory.write_string(value_address, 257, value).unwrap();
                drop(memory);
                let result = if set {
                    invoke_import(game, 69, &[info_address, key_address, value_address])
                } else {
                    invoke_import(game, 68, &[info_address, key_address])
                };
                assert_eq!(result, accepted);
                let memory = qa_compat::memory::ModuleMemory::borrow(
                    base,
                    game.vm.process.memory_mut().unwrap(),
                )
                .unwrap();
                assert_eq!(memory.cstring(info_address).unwrap(), expected);
            };
            mutate_info(
                &mut game,
                false,
                b"\\name\\Mike\\name\\later\\NAME\\raw\x80",
                b"name",
                b"",
                1,
                b"\\NAME\\raw\x80",
            );
            mutate_info(
                &mut game,
                false,
                b"\\name\\Mike",
                b"NAME",
                b"",
                0,
                b"\\name\\Mike",
            );
            mutate_info(
                &mut game,
                true,
                b"\\name\\Mike\\name\\later\\n\\raw\x80",
                b"name",
                b"new",
                1,
                b"\\n\\raw\x80\\name\\new",
            );
            mutate_info(
                &mut game,
                true,
                b"\\name\\Mike\\name\\later",
                b"name",
                b"",
                1,
                b"",
            );
            mutate_info(
                &mut game,
                true,
                b"\\name\\Mike",
                b"name",
                b"new;value",
                0,
                b"\\name\\Mike",
            );
            mutate_info(
                &mut game,
                true,
                b"\\name\\Mike",
                b"name",
                b"\xdc",
                0,
                b"\\name\\Mike",
            );
            mutate_info(
                &mut game,
                true,
                b"\\name\\Mike",
                b"name",
                b"\xe1\x01\xff\x80b",
                1,
                b"\\name\\ab",
            );
            mutate_info(
                &mut game,
                true,
                b"\\keep\\yes",
                b"\xe1",
                b"value",
                1,
                b"\\keep\\yes\\a\\value",
            );
            mutate_info(
                &mut game,
                true,
                b"\\name\\Mike",
                b"name",
                &[b'a'; 256],
                0,
                b"\\name\\Mike",
            );
            mutate_info(
                &mut game,
                true,
                b"",
                &[b'k'; 63],
                &[b'v'; 255],
                1,
                &[b"\\".as_slice(), &[b'k'; 63], b"\\", &[b'v'; 255]].concat(),
            );
            mutate_info(
                &mut game,
                true,
                b"\\name\\Mike",
                &[b'k'; 64],
                b"value",
                0,
                b"\\name\\Mike",
            );
            let mut large = b"\\name\\old\\keep\\".to_vec();
            large.extend(std::iter::repeat_n(b'x', info_bytes - 16));
            let mut remaining = b"\\keep\\".to_vec();
            remaining.extend(std::iter::repeat_n(b'x', info_bytes - 16));
            mutate_info(
                &mut game,
                true,
                &large,
                b"name",
                b"replacement",
                0,
                &remaining,
            );
            let mut exact = b"\\keep\\".to_vec();
            exact.extend(std::iter::repeat_n(b'x', info_bytes - 11));
            let mut expected = exact.clone();
            expected.extend(b"\\k\\v");
            mutate_info(&mut game, true, &exact, b"k", b"v", 1, &expected);
            exact.push(b'x');
            mutate_info(&mut game, true, &exact, b"k", b"v", 0, &exact);
            // Invalid fields reject before inspecting even a null target.
            let mut memory = qa_compat::memory::ModuleMemory::borrow(
                base,
                game.vm.process.memory_mut().unwrap(),
            )
            .unwrap();
            memory.write_string(key_address, 128, b"bad;key").unwrap();
            memory.write_string(value_address, 128, b"value").unwrap();
            drop(memory);
            assert_eq!(
                invoke_import(&mut game, 69, &[0, key_address, value_address]),
                0
            );
        }
        invoke_import(&mut game, config_slot, &[3, base + 0x1720]);
        invoke_import(&mut game, config_slot, &[3, base + 0x1720]);
        if rr {
            let read = |game: &mut Game, pointer: u64| {
                qa_compat::memory::ModuleMemory::borrow(base, game.vm.process.memory_mut().unwrap())
                    .unwrap()
                    .cstring(pointer)
                    .unwrap()
                    .to_vec()
            };
            let pointer = invoke_import(&mut game, 8, &[3]);
            assert_ne!(pointer, 0);
            assert_eq!(read(&mut game, pointer), b"_qa_child_cvar");
            assert_eq!(invoke_import(&mut game, 8, &[3]), pointer);
            // The child retains a pointer while the common store is changed by
            // another import. It is refreshed before the child resumes.
            game.vm.process.memory_mut().unwrap()[0x1b00..0x1b05].copy_from_slice(b"\x80raw\0");
            invoke_import(&mut game, config_slot, &[3, base + 0x1b00]);
            assert_eq!(read(&mut game, pointer), b"\x80raw");
            let other = invoke_import(&mut game, 8, &[4]);
            assert_ne!(other, pointer);
            assert_eq!(read(&mut game, other), b"");
            invoke_import(&mut game, config_slot, &[3, 0]);
            assert_eq!(read(&mut game, pointer), b"");
            assert_eq!(invoke_import(&mut game, 8, &[3]), pointer);
            assert_eq!(read(&mut game, other), b"");
            // RR normal strings truncate at 95; statusbar retains its extended
            // native capacity. Each returned pointer has independent storage.
            game.vm.process.memory_mut().unwrap()[0x1b00..0x1b81].fill(b'x');
            game.vm.process.memory_mut().unwrap()[0x1b80] = 0;
            invoke_import(&mut game, config_slot, &[3, base + 0x1b00]);
            assert_eq!(read(&mut game, pointer), vec![b'x'; 95]);
            invoke_import(&mut game, config_slot, &[5, base + 0x1b00]);
            let statusbar = invoke_import(&mut game, 8, &[5]);
            assert_eq!(read(&mut game, statusbar), vec![b'x'; 128]);
            assert_eq!(read(&mut game, pointer), vec![b'x'; 95]);
        }
        invoke_import(&mut game, config_slot, &[3, 0]);
        for slot in index_slot..index_slot + 3 {
            assert_eq!(invoke_import(&mut game, slot, &[base + 0x1720, 0]), 1);
            assert_eq!(invoke_import(&mut game, slot, &[base + 0x1720, 0]), 1);
            assert_eq!(invoke_import(&mut game, slot, &[0, 0]), 0);
        }
        if rr {
            invoke_import(&mut game, 49, &[entity_address, 0]);
            invoke_import(&mut game, 49, &[entity_address, 0]);
            invoke_import(&mut game, 49, &[0, 0]);
            invoke_import(&mut game, 49, &[entity_address + stride, 0]);
            // Registration observes a real native lifetime once. It neither
            // derives its common slot from the edict ordinal nor duplicates it.
            game.vm.process.memory_mut().unwrap()
                [table_at + size as usize..table_at + size as usize + 4]
                .copy_from_slice(&3u32.to_le_bytes());
            game.vm.process.memory_mut().unwrap()
                [(entity_address + stride - base) as usize + 1376] = 1;
            invoke_import(&mut game, 48, &[entity_address + stride, 0]);
            invoke_import(&mut game, 48, &[entity_address + stride, 0]);
            invoke_import(&mut game, 49, &[entity_address + stride, 0]);
            invoke_import(&mut game, 48, &[entity_address + stride, 0]);
        }
        game.vm.process.memory_mut().unwrap()
            [table_at + size as usize..table_at + size as usize + 4]
            .copy_from_slice(&3u32.to_le_bytes());
        if !rr {
            qa_compat::memory::ModuleMemory::borrow(base, game.vm.process.memory_mut().unwrap())
                .unwrap()
                .write_word(entity_address + stride + 96, 1)
                .unwrap();
        }
        // Exercise the actual child ABI, including the hidden result pointer,
        // register arguments and three stack arguments. Edict3 is inactive and
        // unbound; owner comparisons still use its original weak slot identity.
        let output = base + 0x1d00;
        let start = base + 0x1d80;
        let end = base + 0x1d8c;
        let inactive = entity_address + stride * 2;
        let set_fields = |game: &mut Game, owner: u64, passed_owner: u64, flags: u32| {
            let mut memory = qa_compat::memory::ModuleMemory::borrow(
                base,
                game.vm.process.memory_mut().unwrap(),
            )
            .unwrap();
            memory
                .write_vec3(start, qa_core::primitives::Vec3([-50.0, 5.0, 7.0]))
                .unwrap();
            memory
                .write_vec3(end, qa_core::primitives::Vec3([50.0, 5.0, 7.0]))
                .unwrap();
            memory
                .write(
                    entity_address + if rr { 1464 } else { 272 },
                    &owner.to_le_bytes(),
                )
                .unwrap();
            memory
                .write(
                    inactive + if rr { 1464 } else { 272 },
                    &passed_owner.to_le_bytes(),
                )
                .unwrap();
            memory
                .write_word(entity_address + if rr { 1392 } else { 200 }, flags as i32)
                .unwrap();
        };
        let result_entity = |game: &mut Game| {
            let memory = qa_compat::memory::ModuleMemory::borrow(
                base,
                game.vm.process.memory_mut().unwrap(),
            )
            .unwrap();
            u64::from_le_bytes(
                memory
                    .read(output + if rr { 56 } else { 64 }, 8)
                    .unwrap()
                    .try_into()
                    .unwrap(),
            )
        };
        let body_mask = qa_world::collision::Contents::BODY.to_q2() as u64;
        invoke_import(&mut game, link_slot, &[entity_address, 0]);
        for (owner, passed_owner, flags, pass, mask, expected) in [
            (0, 0, 0, 0, body_mask, entity_address),
            (0, 0, 0, entity_address, body_mask, base + 0x3000),
            (inactive, 0, 0, inactive, body_mask, base + 0x3000),
            (0, entity_address, 0, inactive, body_mask, base + 0x3000),
            (
                0,
                0,
                8,
                0,
                body_mask,
                if rr { base + 0x3000 } else { entity_address },
            ),
            (0, 0, 8, 0, body_mask | (1 << 30), entity_address),
            (
                0,
                0,
                128,
                0,
                body_mask,
                if rr { base + 0x3000 } else { entity_address },
            ),
            (0, 0, 128, 0, body_mask | (1 << 31), entity_address),
        ] {
            set_fields(&mut game, owner, passed_owner, flags);
            assert_eq!(
                invoke_import(
                    &mut game,
                    trace_slot,
                    &[output, start, 0, 0, end, pass, mask]
                ),
                output
            );
            assert_eq!(result_entity(&mut game), expected);
        }
        set_fields(&mut game, 0, 0, 0);
        let fraction = |game: &mut Game| {
            qa_compat::memory::ModuleMemory::borrow(base, game.vm.process.memory_mut().unwrap())
                .unwrap()
                .read_word(output + if rr { 4 } else { 8 })
                .unwrap() as u32
        };
        let vec_bytes = |v: [f32; 3]| {
            v.into_iter()
                .flat_map(|v| v.to_le_bytes())
                .collect::<Vec<_>>()
        };
        for (address, bytes, expected) in [
            (
                entity_address + 4,
                vec_bytes([8.0, 5.0, 7.0]),
                (42.0f32 - 0.03125) / 100.0,
            ),
            (
                entity_address + if rr { 1396 } else { 204 },
                vec_bytes([-4.0, -16.0, -24.0]),
                (54.0f32 - 0.03125) / 100.0,
            ),
        ] {
            live_edits.borrow_mut().push((address, bytes));
            invoke_import(
                &mut game,
                trace_slot,
                &[output, start, 0, 0, end, 0, body_mask],
            );
            assert_eq!(fraction(&mut game), expected.to_bits());
        }
        // Current SOLID_NOT disables an old solid-chain member immediately.
        live_edits.borrow_mut().push((
            entity_address + if rr { 1456 } else { 264 },
            if rr {
                vec![0]
            } else {
                0u32.to_le_bytes().to_vec()
            },
        ));
        invoke_import(
            &mut game,
            trace_slot,
            &[output, start, 0, 0, end, 0, body_mask],
        );
        assert_eq!(fraction(&mut game), 1.0f32.to_bits());
        // PointContents walks the old AREA_SOLID membership even after the
        // current SOLID_NOT value disables traces, using the live box hull.
        let live_point = base + 0x1d98;
        qa_compat::memory::ModuleMemory::borrow(base, game.vm.process.memory_mut().unwrap())
            .unwrap()
            .write_vec3(live_point, qa_core::primitives::Vec3([8.0, 5.0, 7.0]))
            .unwrap();
        assert_eq!(
            invoke_import(&mut game, contents_slot, &[live_point]),
            body_mask
        );
        // Restore ABI fields without relinking; a contents query reads the
        // live origin/bounds while the same stored area membership remains.
        live_edits.borrow_mut().extend([
            (
                entity_address + if rr { 1456 } else { 264 },
                if rr {
                    vec![2]
                } else {
                    2u32.to_le_bytes().to_vec()
                },
            ),
            (entity_address + 4, vec_bytes([3.0, 5.0, 7.0])),
            (
                entity_address + if rr { 1396 } else { 204 },
                vec_bytes([-16.0, -16.0, -24.0]),
            ),
        ]);
        invoke_import(
            &mut game,
            trace_slot,
            &[output, start, 0, 0, end, 0, body_mask],
        );
        // Current model selection must use the existing config revision/cache
        // even when the old linked common shape was a box. Model1 is empty.
        let model_row = game.vm.resource_ranges().unwrap()[0].get(1).unwrap();
        game.vm.process.memory_mut().unwrap()[0x1b00..0x1b03].copy_from_slice(b"*1\0");
        invoke_import(
            &mut game,
            if rr { 7 } else { 6 },
            &[model_row as u64, base + 0x1b00],
        );
        live_edits.borrow_mut().extend([
            (
                entity_address + if rr { 1456 } else { 264 },
                if rr {
                    vec![3]
                } else {
                    3u32.to_le_bytes().to_vec()
                },
            ),
            (entity_address + 40, 1u32.to_le_bytes().to_vec()),
        ]);
        invoke_import(
            &mut game,
            trace_slot,
            &[output, start, 0, 0, end, 0, body_mask],
        );
        assert_eq!(fraction(&mut game), 1.0f32.to_bits());
        live_edits.borrow_mut().push((
            entity_address + if rr { 1456 } else { 264 },
            if rr {
                vec![2]
            } else {
                2u32.to_le_bytes().to_vec()
            },
        ));
        invoke_import(
            &mut game,
            trace_slot,
            &[output, start, 0, 0, end, 0, body_mask],
        );
        invoke_import(
            &mut game,
            if rr { 7 } else { 6 },
            &[model_row as u64, base + 0x1720],
        );
        let point = base + 0x1d98;
        let mut memory =
            qa_compat::memory::ModuleMemory::borrow(base, game.vm.process.memory_mut().unwrap())
                .unwrap();
        memory
            .write_vec3(point, qa_core::primitives::Vec3([-13.0, 5.0, 7.0]))
            .unwrap();
        drop(memory);
        assert_eq!(invoke_import(&mut game, contents_slot, &[point]), body_mask);
        live_edits
            .borrow_mut()
            .push((entity_address + 4, vec_bytes([4.0, 5.0, 7.0])));
        assert_eq!(invoke_import(&mut game, contents_slot, &[point]), 0);
        live_edits
            .borrow_mut()
            .push((entity_address + 4, vec_bytes([3.0, 5.0, 7.0])));
        invoke_import(
            &mut game,
            trace_slot,
            &[output, start, 0, 0, end, 0, body_mask],
        );
        if rr {
            let fraction = |game: &mut Game| {
                qa_compat::memory::ModuleMemory::borrow(base, game.vm.process.memory_mut().unwrap())
                    .unwrap()
                    .read_word(output + 4)
                    .unwrap() as u32
            };
            // A direct world clip ignores the linked body intersecting this segment.
            assert_eq!(
                invoke_import(
                    &mut game,
                    15,
                    &[output, base + 0x3000, start, 0, 0, end, body_mask]
                ),
                output
            );
            assert_eq!(fraction(&mut game), 1.0f32.to_bits());
            assert_eq!(result_entity(&mut game), base + 0x3000);
            // A miss still returns the specified target, unlike an ordinary trace.
            invoke_import(
                &mut game,
                15,
                &[output, entity_address, start, 0, 0, end, 1],
            );
            assert_eq!(fraction(&mut game), 1.0f32.to_bits());
            assert_eq!(result_entity(&mut game), entity_address);
            // This native slot has no common lifetime, is not in use, and is unlinked.
            // NOT, TRIGGER without an inline model, and BBOX all use its temporary box.
            for solid in [0, 1, 2] {
                let mut memory = qa_compat::memory::ModuleMemory::borrow(
                    base,
                    game.vm.process.memory_mut().unwrap(),
                )
                .unwrap();
                memory.write_vec3(inactive + 4, origin).unwrap();
                memory
                    .write_vec3(inactive + 16, qa_core::primitives::Vec3([0.0, 90.0, 0.0]))
                    .unwrap();
                memory
                    .write_vec3(
                        inactive + 1396,
                        qa_core::primitives::Vec3([-16.0, -16.0, -24.0]),
                    )
                    .unwrap();
                memory
                    .write_vec3(
                        inactive + 1408,
                        qa_core::primitives::Vec3([16.0, 16.0, 32.0]),
                    )
                    .unwrap();
                memory.write(inactive + 1456, &[solid]).unwrap();
                drop(memory);
                invoke_import(
                    &mut game,
                    15,
                    &[output, inactive, start, 0, 0, end, body_mask],
                );
                assert_eq!(fraction(&mut game), ((37.0f32 - 0.03125) / 100.0).to_bits());
                assert_eq!(result_entity(&mut game), inactive);
            }
            // Raw target motion takes effect without relinking and never mutates the
            // authoritative pose or area link of the previously linked client.
            qa_compat::memory::ModuleMemory::borrow(base, game.vm.process.memory_mut().unwrap())
                .unwrap()
                .write_vec3(inactive + 4, qa_core::primitives::Vec3([90.0, 5.0, 7.0]))
                .unwrap();
            invoke_import(
                &mut game,
                15,
                &[output, inactive, start, 0, 0, end, body_mask],
            );
            assert_eq!(fraction(&mut game), 1.0f32.to_bits());
            assert_eq!(result_entity(&mut game), inactive);
        }
        let list = base + 0x1e00;
        let predicate = base + 0x1780;
        for (limit, role, callback, decision, expected) in [
            (3, 1, 0, 0, 1),
            (1, 1, 0, 0, 1),
            (3, 2, 0, 0, 0),
            (0, 1, 0, 0, if rr { 1 } else { 0 }),
            (3, 1, predicate, 64, 1),
            (3, 1, predicate, 1, 0),
            (0, 1, predicate, 64, 1),
        ] {
            if !rr && callback != 0 {
                continue;
            }
            game.vm.process.memory_mut().unwrap()[0x1e00..0x1e18].fill(0xcc);
            let mut code = vec![0xb8];
            code.extend((decision as u32).to_le_bytes());
            code.push(0xc3);
            game.vm.process.memory_mut().unwrap()[0x1780..0x1780 + code.len()]
                .copy_from_slice(&code);
            let words = [
                start,
                end,
                if limit == 0 { 0 } else { list },
                limit,
                role,
                callback,
                0,
            ];
            assert_eq!(
                invoke_import(&mut game, area_slot, &words[..if rr { 7 } else { 5 }]),
                expected
            );
            let memory = game.vm.process.memory().unwrap();
            if expected != 0 && limit != 0 {
                assert_eq!(&memory[0x1e00..0x1e08], &entity_address.to_le_bytes());
                assert_eq!(&memory[0x1e08..0x1e18], &[0xcc; 16]);
            } else {
                assert_eq!(&memory[0x1e00..0x1e18], &[0xcc; 24]);
            }
        }
        for (point, expected) in [
            (origin, body_mask),
            (qa_core::primitives::Vec3([-13.0, -11.0, -17.0]), body_mask),
            (qa_core::primitives::Vec3([19.0, 21.0, 39.0]), 0),
            (qa_core::primitives::Vec3([50.0, 5.0, 7.0]), 0),
        ] {
            qa_compat::memory::ModuleMemory::borrow(base, game.vm.process.memory_mut().unwrap())
                .unwrap()
                .write_vec3(start, point)
                .unwrap();
            assert_eq!(invoke_import(&mut game, contents_slot, &[start]), expected);
        }
        assert_eq!(invoke_import(&mut game, connected_slot, &[7, 9]), 0);
        let sight_slot = if rr { 17 } else { 14 };
        let hearing_slot = sight_slot + 1;
        for (point, from) in [(start, [1.0, 0.0, 0.0]), (end, [-1.0, 0.0, 0.0])] {
            qa_compat::memory::ModuleMemory::borrow(base, game.vm.process.memory_mut().unwrap())
                .unwrap()
                .write_vec3(point, qa_core::primitives::Vec3(from))
                .unwrap();
        }
        for slot in [sight_slot, hearing_slot] {
            assert_eq!(invoke_import(&mut game, slot, &[start, end, 1]), 0);
            if rr {
                assert_eq!(
                    invoke_import(&mut game, slot, &[start, end, 0]),
                    u64::from(slot == hearing_slot)
                );
            }
        }
        invoke_import(&mut game, portal_slot, &[2, 1]);
        assert_eq!(invoke_import(&mut game, connected_slot, &[7, 9]), 1);
        for slot in [sight_slot, hearing_slot] {
            assert_eq!(
                invoke_import(&mut game, slot, &[start, end, 1]),
                u64::from(slot == hearing_slot)
            );
        }
        invoke_import(&mut game, portal_slot, &[2, 1]);
        invoke_import(&mut game, portal_slot, &[2, 0]);
        assert_eq!(invoke_import(&mut game, connected_slot, &[7, 9]), 0);
        assert_eq!(
            invoke_import(&mut game, connected_slot, &[0, 0]),
            u64::from(!rr)
        );
        assert_eq!(invoke_import(&mut game, connected_slot, &[10, 9]), 0);
        if rr {
            invoke_import(&mut game, portal_slot, &[99, 1]);
            invoke_import(&mut game, portal_slot, &[u64::MAX, 1]);
        }
        let memory = game.vm.process.memory_mut().unwrap();
        memory[0x1e40..0x1e4c].copy_from_slice(b"map_noareas\0");
        memory[0x1e80..0x1e82].copy_from_slice(b"1\0");
        memory[0x1e84..0x1e86].copy_from_slice(b"0\0");
        memory[0x1e88..0x1e8c].copy_from_slice(b"0.5\0");
        let force_slot = if rr { 41 } else { 38 };
        invoke_import(&mut game, force_slot, &[base + 0x1e40, base + 0x1e80]);
        assert_eq!(invoke_import(&mut game, sight_slot, &[start, end, 1]), 0);
        assert_eq!(invoke_import(&mut game, hearing_slot, &[start, end, 1]), 1);
        assert_eq!(invoke_import(&mut game, connected_slot, &[0, 0]), 1);
        assert_eq!(invoke_import(&mut game, connected_slot, &[7, 9]), 1);
        invoke_import(&mut game, force_slot, &[base + 0x1e40, base + 0x1e84]);
        assert_eq!(invoke_import(&mut game, connected_slot, &[7, 9]), 0);
        invoke_import(&mut game, force_slot, &[base + 0x1e40, base + 0x1e88]);
        assert_eq!(
            invoke_import(&mut game, hearing_slot, &[start, end, 1]),
            u64::from(!rr)
        );
        assert_eq!(
            invoke_import(&mut game, connected_slot, &[7, 9]),
            u64::from(!rr)
        );
        invoke_import(&mut game, force_slot, &[base + 0x1e40, base + 0x1e84]);
        invoke_import(&mut game, link_slot, &[entity_address + stride, 0]);
        invoke_import(&mut game, link_slot, &[entity_address + stride, 0]);
        qa_compat::memory::ModuleMemory::borrow(base, game.vm.process.memory_mut().unwrap())
            .unwrap()
            .write_word(entity_address + stride + if rr { 1380 } else { 100 }, 0)
            .unwrap();
        invoke_import(&mut game, link_slot, &[entity_address + stride, 1]);
        let address = entity_address + stride * 2;
        let mut memory =
            qa_compat::memory::ModuleMemory::borrow(base, game.vm.process.memory_mut().unwrap())
                .unwrap();
        memory
            .write_word(base + table_at as u64 + size as u64, 4)
            .unwrap();
        memory.write_vec3(address + 4, origin).unwrap();
        memory
            .write_vec3(address + 16, qa_core::primitives::Vec3([0.0, 90.0, 0.0]))
            .unwrap();
        memory
            .write_vec3(address + 28, qa_core::primitives::Vec3([99.0; 3]))
            .unwrap();
        memory.write_word(address + 40, 2).unwrap();
        memory
            .write_vec3(
                address + if rr { 1396 } else { 204 },
                qa_core::primitives::Vec3([-16.0, -16.0, -24.0]),
            )
            .unwrap();
        memory
            .write_vec3(
                address + if rr { 1408 } else { 216 },
                qa_core::primitives::Vec3([16.0, 16.0, 32.0]),
            )
            .unwrap();
        if rr {
            memory.write(address + 1376, &[1]).unwrap();
            memory.write(address + 1456, &[3]).unwrap();
            memory.write_word(address + 72, 128).unwrap();
        } else {
            memory.write_word(address + 96, 1).unwrap();
            memory.write_word(address + 264, 3).unwrap();
        }
        memory.write(base + 0x17a0, b"*1\0").unwrap();
        drop(memory);
        invoke_import(
            &mut game,
            config_slot,
            &[if rr { 64 } else { 34 }, base + 0x17a0],
        );
        if rr {
            // A trigger's valid inline model is used even while unlinked. The
            // fixture's empty inline model clears where its temporary box hits.
            for solid in [1, 3] {
                qa_compat::memory::ModuleMemory::borrow(
                    base,
                    game.vm.process.memory_mut().unwrap(),
                )
                .unwrap()
                .write(address + 1456, &[solid])
                .unwrap();
                invoke_import(
                    &mut game,
                    15,
                    &[output, address, start, 0, 0, end, body_mask],
                );
                assert_eq!(result_entity(&mut game), address);
                let memory = qa_compat::memory::ModuleMemory::borrow(
                    base,
                    game.vm.process.memory_mut().unwrap(),
                )
                .unwrap();
                assert_eq!(
                    memory.read_word(output + 4).unwrap() as u32,
                    1.0f32.to_bits()
                );
            }
        }
        invoke_import(&mut game, link_slot, &[address, 0]);
        invoke_import(&mut game, link_slot, &[address, 0]);
        invoke_import(&mut game, link_slot, &[entity_address, 0]);
        invoke_import(&mut game, link_slot, &[base + 0x3000, 0]);
        invoke_import(&mut game, set_model_slot, &[entity_address, base + 0x1720]);
        invoke_import(&mut game, set_model_slot, &[address, base + 0x17a0]);
        invoke_import(&mut game, unlink_slot, &[entity_address, 0]);
        invoke_import(&mut game, unlink_slot, &[entity_address, 1]);
        // A cached old lifetime cannot unlink a replacement owned by another module.
        invoke_import(&mut game, unlink_slot, &[entity_address, 0]);
        // Unlinking a never-published native slot must not create a common entity.
        invoke_import(&mut game, unlink_slot, &[entity_address + stride, 0]);
        invoke_import(&mut game, unlink_slot, &[base + 0x3000, 0]);
        for (offset, word) in [
            (0, 0x1234_ffff),
            (1, 0x80),
            (2, (-321i32) as u32 as u64),
            (3, 0x1234_5678),
            (4, 0x7fa1_2345),
            (5, base + 0x1720),
            (5, 0),
        ] {
            invoke_import(&mut game, write_slots[offset], &[word]);
        }
        assert_eq!(invoke_import(&mut game, argc_slot, &[]), 3);
        for (index, expected) in [
            (0, b"_qa_native_command".as_slice()),
            (1, b"\x80"),
            (2, b"third"),
            (3, b""),
            (u32::MAX, b""),
        ] {
            let address = invoke_import(&mut game, argv_slot, &[u64::from(index)]);
            assert_ne!(address, 0);
            assert_eq!(
                qa_compat::memory::ModuleMemory::borrow(
                    base,
                    game.vm.process.memory_mut().unwrap()
                )
                .unwrap()
                .cstring(address)
                .unwrap(),
                expected
            );
            assert_eq!(
                game.vm.process.memory().unwrap()[0x1c38],
                expected.first().copied().unwrap_or(0)
            );
        }
        let address = invoke_import(&mut game, args_slot, &[]);
        assert_eq!(
            qa_compat::memory::ModuleMemory::borrow(base, game.vm.process.memory_mut().unwrap())
                .unwrap()
                .cstring(address)
                .unwrap(),
            if rr {
                b"\"\x80\"   third ".as_slice()
            } else {
                b"\"\x80\"   third"
            }
        );
        assert_eq!(game.vm.process.memory().unwrap()[0x1c38], b'"');
        for (at, text) in [
            (0x1e00, b"sensitivity 7\0".as_slice()),
            (0x1e40, b" extra\n\0"),
        ] {
            game.vm.process.memory_mut().unwrap()[at..at + text.len()].copy_from_slice(text);
            invoke_import(&mut game, append_slot, &[base + at as u64]);
        }
        for (at, vector) in [
            (0x1e80, [1.0625, -2.125, 4096.0]),
            (0x1ec0, [0.0, 0.0, 1.0]),
        ] {
            qa_compat::memory::ModuleMemory::borrow(base, game.vm.process.memory_mut().unwrap())
                .unwrap()
                .write_vec3(base + at, qa_core::primitives::Vec3(vector))
                .unwrap();
        }
        invoke_import(&mut game, vector_slots[0], &[base + 0x1e80]);
        invoke_import(&mut game, vector_slots[1], &[base + 0x1ec0]);
        invoke_import(&mut game, vector_slots[1], &[0]);
        invoke_import(
            &mut game,
            vector_slots[2],
            &[u64::from((-270.0f32).to_bits())],
        );
        if let Some(slot) = entity_slot {
            invoke_import(&mut game, slot, &[entity_address + stride]);
            invoke_import(&mut game, slot, &[base + 0x3000]);
        }
        invoke_import(
            &mut game,
            if rr { 24 } else { 22 },
            &[0, if rr { 0 } else { 3 }, 1],
        );
        invoke_import(&mut game, write_slots[0], &[0xaa]);
        invoke_import(
            &mut game,
            if rr { 25 } else { 23 },
            &[0, 1, u64::from(u32::MAX)],
        );
        invoke_import(&mut game, if rr { 24 } else { 22 }, &[base + 0x1e80, 2, 0]);
        for to in 0..if rr { 3 } else { 6 } {
            invoke_import(&mut game, write_slots[0], &[0xbb + to]);
            invoke_import(&mut game, if rr { 24 } else { 22 }, &[base + 0x1e80, to, 1]);
        }
        drop(invoke_import);
        console.execute_frame(&mut runtime);
        let rate = std::num::NonZeroU32::new(44100).unwrap();
        runtime.sound_bank = Some(std::sync::Arc::new(
            qa_audio::Bank::load(
                rate,
                vec![(
                    "sound/_qa_child_cvar".into(),
                    qa_core::primitives::Pcm {
                        rate,
                        channels: qa_core::primitives::PcmChannels::Mono,
                        samples: vec![1024],
                        loop_start: None,
                    },
                )],
            )
            .unwrap(),
        ));
        storage
            .bind_sound_bank(
                ModuleId(1),
                game.vm.resource_ranges().unwrap()[1],
                std::sync::Arc::clone(runtime.sound_bank.as_ref().unwrap()),
            )
            .unwrap();
        let source = {
            let memory = qa_compat::memory::ModuleMemory::borrow(
                base,
                game.vm.process.memory_mut().unwrap(),
            )
            .unwrap();
            let position = memory.read_vec3(entity_address + 4).unwrap();
            if memory
                .read_word(entity_address + if rr { 1380 } else { 100 })
                .unwrap()
                == 3
            {
                qa_core::primitives::Body {
                    position,
                    mins: memory
                        .read_vec3(entity_address + if rr { 1396 } else { 204 })
                        .unwrap(),
                    maxs: memory
                        .read_vec3(entity_address + if rr { 1408 } else { 216 })
                        .unwrap(),
                    ..Default::default()
                }
                .center()
            } else {
                position
            }
        };
        let mut emit_sound =
            |game: &mut Game, slot: usize, arguments: &[u64], sound_rules: RuleSetId| {
                let mut sound_context = context;
                sound_context.console.source = RuleSetId::Quake3;
                sound_context.sound_rules = sound_rules;
                let code = native_import_code(
                    pointers[slot],
                    arguments,
                    false,
                    if slot == 4 { &[3] } else { &[] },
                    0,
                );
                game.vm.process.memory_mut().unwrap()[code_at..code_at + code.len()]
                    .copy_from_slice(&code);
                let mut services =
                    runtime.engine_services(&mut console, &mut storage, &mut scratch);
                game.vm.call(
                    &mut NativeCalls {
                        native_player: None,
                        services: &mut services,
                        table: game.imports,
                        context: sound_context,
                        platform_time: EventTime(0),
                        command: &[],
                        raw_args: b"",
                        unknown: &mut unknown,
                    },
                    run,
                    &[1],
                )
            };
        let volume = u64::from(0.5f32.to_bits());
        let attenuation = u64::from(1.0f32.to_bits());
        let offset = u64::from(0.125f32.to_bits());
        emit_sound(
            &mut game,
            4,
            &[entity_address, 18, 1, volume, attenuation, offset],
            RuleSetId::Quake,
        )
        .unwrap();
        emit_sound(
            &mut game,
            5,
            &[
                base + 0x1e80,
                entity_address,
                3,
                1,
                volume,
                attenuation,
                offset,
            ],
            RuleSetId::Quake3,
        )
        .unwrap();
        emit_sound(
            &mut game,
            4,
            &[0, 18, 1, volume, attenuation, offset],
            RuleSetId::Quake,
        )
        .unwrap();
        emit_sound(
            &mut game,
            5,
            &[0, entity_address, 3, 1, volume, attenuation, offset],
            RuleSetId::Quake,
        )
        .unwrap();
        emit_sound(
            &mut game,
            4,
            &[entity_address, 24, 1, volume, attenuation, offset],
            RuleSetId::Quake,
        )
        .unwrap();
        emit_sound(
            &mut game,
            4,
            &[entity_address, 2, 1, volume, 0, offset],
            RuleSetId::Quake,
        )
        .unwrap();
        drop(emit_sound);
        let mut batch = runtime.server.events.batch(sound_observer).unwrap();
        let mut sounds = Vec::new();
        let mut sound_deliveries = Vec::new();
        while let Some(record) = runtime.server.events.next(&mut batch) {
            if let FrameEvent::Sound(sound) = record.event {
                sounds.push(sound);
                sound_deliveries.push(record.delivery.unwrap());
            }
        }
        assert_eq!(sounds.len(), 5);
        assert_eq!(
            sounds
                .iter()
                .map(|sound| sound.spatial_rules)
                .collect::<Vec<_>>(),
            [
                RuleSetId::Quake,
                RuleSetId::Quake3,
                RuleSetId::Quake,
                RuleSetId::Quake,
                RuleSetId::Quake
            ]
        );
        assert_eq!(sounds[0].position, source);
        assert_eq!(sounds[2].position, source);
        assert_eq!(
            sounds[1].position,
            qa_core::primitives::Vec3([1.0625, -2.125, 4096.0])
        );
        assert_eq!((sounds[0].channel, sound_deliveries[0].reliable), (2, true));
        assert_eq!(
            (sounds[1].channel, sound_deliveries[1].reliable),
            (3, false)
        );
        assert_eq!(
            sound_deliveries[3].target,
            qa_core::primitives::NativeTarget::All
        );
        assert_eq!(sound_deliveries[3].reliable, rr);
        assert_eq!(
            sound_deliveries[4].target,
            qa_core::primitives::NativeTarget::All
        );
        for sound in sounds.into_iter().take(4) {
            assert_eq!(
                (
                    sound.volume,
                    sound.attenuation,
                    sound.time_offset,
                    sound.client
                ),
                (0.5, 1.0, 0.125, None)
            );
        }
        let view = console.cvars.bind("sensitivity", context.console).unwrap();
        assert_eq!(console.cvars.read(view).unwrap().as_str(), "7");
        let mut expected = vec![
            0xff, 0x80, 0xbf, 0xfe, 0x78, 0x56, 0x34, 0x12, 0x45, 0x23, 0xa1, 0x7f,
        ];
        expected.extend(b"_qa_child_cvar\0\0");
        if rr {
            for value in [1.0625f32, -2.125, 4096.0] {
                expected.extend(value.to_bits().to_le_bytes());
            }
        } else {
            for value in [8i16, -17, i16::MIN] {
                expected.extend(value.to_le_bytes());
            }
        }
        expected.extend([5, 0, 64]);
        if rr {
            expected.extend([2, 0, 0, 0]);
        }
        let mut native = Vec::new();
        let mut batch = runtime.server.events.batch(sound_observer).unwrap();
        let mut output = vec![0; 0x8000];
        while let Some(record) = runtime.server.events.next(&mut batch) {
            if let FrameEvent::Native(message) = record.event {
                let bytes = runtime
                    .server
                    .events
                    .texts
                    .copy_payload(message, &mut output)
                    .unwrap();
                native.push((record.delivery.unwrap(), bytes.to_vec()));
            }
            runtime.server.events.submit(
                sound_observer,
                record.sequence,
                qa_core::events::OutputSubmission::BestEffort,
            );
        }
        assert_eq!(native[0].1, expected);
        assert!(native[0].0.reliable);
        assert_eq!(
            native[0].0.source,
            if rr {
                RuleSetId::Quake2Rerelease
            } else {
                RuleSetId::Quake2
            }
        );
        let start = if rr {
            1
        } else {
            assert_eq!(native[1].1, [0xaa]);
            assert_eq!(native[1].0.target, qa_core::primitives::NativeTarget::Sight);
            assert_eq!(
                native[1].0.origin,
                qa_core::primitives::Vec3([1.0625, -2.125, 4096.0])
            );
            2
        };
        for (to, (delivery, bytes)) in native[start..].iter().enumerate() {
            use qa_core::primitives::NativeTarget;
            assert_eq!(*bytes, [0xbb + to as u8]);
            assert_eq!(delivery.reliable, rr || to >= 3);
            assert_eq!(
                delivery.target,
                [
                    NativeTarget::All,
                    NativeTarget::Hearing,
                    NativeTarget::Sight
                ][to % 3]
            );
        }
        if let Some(entity) = owned_entity {
            let mut services = runtime.engine_services(&mut console, &mut storage, &mut scratch);
            (qa_compat::services::ENGINE_CALLS.free)(&mut services, context, entity).unwrap();
            assert!(!services.server.navigation.contains(entity));
            assert!(services.server.entities.resolve(entity).is_none());
        }
        for first in if rr {
            [62, 8254, 10302]
        } else {
            [32, 288, 544]
        } {
            assert_eq!(
                storage.configstring(ModuleId(1), first + 1).unwrap(),
                (
                    &b"_qa_child_cvar"[..],
                    if first + 1 == model_row { 3 } else { 1 }
                )
            );
        }
        assert!(!runtime.server.area.unlink(bound));
        assert!(runtime.server.entities.resolve(bound).is_none());
        assert!(runtime.server.area.unlink(replacement.unwrap()));
        if rr {
            assert_eq!(
                game.vm.process.memory().unwrap()[(entity_address - base) as usize + 1377],
                0
            );
            use qa_core::primitives::{NativeEntity, PlayerTail, PrintKind};
            let native = NativeEntity {
                module: ModuleId(1),
                slot: 2,
            };
            assert!(!runtime.server.entities.active().any(|entity| {
                runtime.server.entities.columns.native_entity[entity.slot as usize] == Some(native)
            }));
            let client = runtime
                .server
                .connect(
                    qa_session::clients::Connection::Local,
                    ModuleId(1),
                    PlayerTail::None,
                    Some(native),
                )
                .unwrap();
            // Common client zero/reserved entity one targets native edict two.
            assert_eq!(client.0, 0);
            assert_ne!(runtime.server.clients[0].entity.slot as i32, native.slot);
            let target = entity_address + stride;
            let observer = runtime
                .server
                .events
                .bind(OutputTarget::Module(ModuleId(91)))
                .unwrap();
            let text = b"\x80RR native print";
            let message = base + 0x1e00;
            let bytes = game.vm.process.memory_mut().unwrap();
            bytes[0x1e00..0x1e00 + text.len()].copy_from_slice(text);
            bytes[0x1e00 + text.len()] = 0;
            let slots = [
                b"Broadcast_Print".as_slice(),
                b"Client_Print",
                b"Center_Print",
            ]
            .map(|name| game.import_ordinal(name).unwrap());
            assert_eq!(slots, [0, 2, 3]);
            let mut emit = |game: &mut Game, slot: usize, arguments: &[u64]| {
                let code = native_import_code(
                    pointers[slot],
                    arguments,
                    false,
                    if slot == 4 { &[3] } else { &[] },
                    0,
                );
                game.vm.process.memory_mut().unwrap()[code_at..code_at + code.len()]
                    .copy_from_slice(&code);
                let mut services =
                    runtime.engine_services(&mut console, &mut storage, &mut scratch);
                game.vm.call(
                    &mut NativeCalls {
                        native_player: None,
                        services: &mut services,
                        table: game.imports,
                        context,
                        platform_time: EventTime(0),
                        command: &[],
                        raw_args: b"",
                        unknown: &mut unknown,
                    },
                    run,
                    &[1],
                )
            };
            emit(&mut game, slots[0], &[0, message]).unwrap();
            emit(&mut game, slots[0], &[2, message]).unwrap();
            emit(&mut game, slots[1], &[target, 1, message]).unwrap();
            emit(&mut game, slots[1], &[target, 2, message]).unwrap();
            emit(&mut game, slots[1], &[target, 3, message]).unwrap();
            emit(&mut game, slots[2], &[target, message]).unwrap();
            emit(&mut game, slots[1], &[0, 2, message]).unwrap();
            // Null center and unconnected targets cannot read bad text or broadcast.
            emit(&mut game, slots[2], &[0, 1]).unwrap();
            emit(&mut game, slots[1], &[entity_address, 2, 1]).unwrap();
            emit(&mut game, slots[1], &[target, 0, message]).unwrap();
            emit(&mut game, write_slots[0], &[0x9a]).unwrap();
            emit(&mut game, unicast_slot, &[target, 1, 0xabcd_ef01_1234_5678]).unwrap();
            // The retail help marker addresses one client but emits from the
            // world at a supplied position, independently of that listener.
            emit(
                &mut game,
                6,
                &[
                    target,
                    base + 0x1e80,
                    base + 0x3000,
                    18,
                    1,
                    u64::from(1.0f32.to_bits()),
                    0,
                    0,
                    0xdead_beef,
                ],
            )
            .unwrap();
            emit(
                &mut game,
                6,
                &[
                    0,
                    1,
                    1,
                    18,
                    1,
                    u64::from(1.0f32.to_bits()),
                    0,
                    0,
                    0xdead_beef,
                ],
            )
            .unwrap();
            drop(emit);
            let mut presentation = runtime
                .server
                .events
                .batch(runtime.server.presentation)
                .unwrap();
            let mut local_prints = Vec::new();
            let mut local_sounds = Vec::new();
            while let Some(record) = runtime.server.events.next(&mut presentation) {
                if let FrameEvent::Sound(sound) = record.event {
                    local_sounds.push(sound);
                }
                if let FrameEvent::Print(print) = record.event
                    && runtime.server.events.texts.get(print.text) == Some(text.as_slice())
                {
                    local_prints.push((print.client, print.kind, print.level));
                }
            }
            assert!(local_sounds.is_empty());
            assert_eq!(
                local_prints,
                [
                    (None, PrintKind::Notify, 0),
                    (None, PrintKind::Console, 2),
                    (Some(client), PrintKind::Notify, 1),
                    (Some(client), PrintKind::Console, 2),
                    (Some(client), PrintKind::Chat, 3),
                    (Some(client), PrintKind::Center, 2),
                    (None, PrintKind::Console, 2),
                    (Some(client), PrintKind::Notify, 0),
                ]
            );
            let mut batch = runtime.server.events.batch(observer).unwrap();
            let mut prints = Vec::new();
            let mut unicasts = Vec::new();
            while let Some(record) = runtime.server.events.next(&mut batch) {
                if let FrameEvent::Sound(sound) = record.event {
                    let delivery = record.delivery.unwrap();
                    assert_eq!(sound.client, None);
                    assert_eq!(sound.channel, 2);
                    assert_eq!(
                        sound.entity,
                        Some(qa_core::primitives::SoundEmitter::Native {
                            namespace: qa_core::primitives::ResourceNamespace::Module(ModuleId(1)),
                            slot: 0,
                        })
                    );
                    assert_eq!(
                        sound.position,
                        qa_core::primitives::Vec3([1.0625, -2.125, 4096.0])
                    );
                    assert!(sound.positioned);
                    assert_eq!(
                        delivery.target,
                        qa_core::primitives::NativeTarget::Client(client)
                    );
                    assert!(delivery.reliable);
                    assert_eq!(delivery.dupe_key, 0xdead_beef);
                    local_sounds.push(sound);
                }
                if let FrameEvent::Native(message) = record.event {
                    let mut bytes = [0; 32];
                    assert_eq!(
                        runtime
                            .server
                            .events
                            .texts
                            .copy_payload(message, &mut bytes)
                            .unwrap(),
                        [0x9a]
                    );
                    unicasts.push(record.delivery.unwrap());
                }
                if let FrameEvent::Print(print) = record.event
                    && runtime.server.events.texts.get(print.text) == Some(text.as_slice())
                {
                    prints.push((print.client, print.kind, print.level));
                }
            }
            assert_eq!(local_sounds.len(), 1);
            assert_eq!(
                prints,
                [
                    (None, PrintKind::Notify, 0),
                    (None, PrintKind::Console, 2),
                    (Some(client), PrintKind::Notify, 1),
                    (Some(client), PrintKind::Console, 2),
                    (Some(client), PrintKind::Chat, 3),
                    (Some(client), PrintKind::Center, 2),
                    (Some(client), PrintKind::Notify, 0),
                ]
            );
            assert_eq!(unicasts.len(), 1);
            assert_eq!(
                unicasts[0].target,
                qa_core::primitives::NativeTarget::Client(client)
            );
            assert!(unicasts[0].reliable);
            assert_eq!(unicasts[0].dupe_key, 0x1234_5678);
            runtime.server.events.unbind(observer);
        }
        // Reserved capacity may precede row publication during initialization.
        let table_address = game.vm.table_address().unwrap() + entity_offset as u64;
        let saved = {
            let mut memory = qa_compat::memory::ModuleMemory::borrow(
                base,
                game.vm.process.memory_mut().unwrap(),
            )
            .unwrap();
            let saved = memory
                .read(table_address, size as usize + 8)
                .unwrap()
                .to_vec();
            memory.write_u64(table_address, 0).unwrap();
            memory.write_word(table_address + size as u64, 0).unwrap();
            memory
                .write_word(table_address + size as u64 + 4, 1)
                .unwrap();
            saved
        };
        let unpublished = game.entities().unwrap();
        assert_eq!(unpublished.count, 0);
        assert_eq!(unpublished.capacity, 1);
        {
            let mut memory = qa_compat::memory::ModuleMemory::borrow(
                base,
                game.vm.process.memory_mut().unwrap(),
            )
            .unwrap();
            memory.write_word(table_address + size as u64, 1).unwrap();
        }
        assert!(game.entities().is_err());
        {
            let mut memory = qa_compat::memory::ModuleMemory::borrow(
                base,
                game.vm.process.memory_mut().unwrap(),
            )
            .unwrap();
            memory.write(table_address, &saved).unwrap();
        }
        let mut frame_code = if rr {
            vec![0x80, 0xf9, 1, 0x74, 2, 0x0f, 0x0b]
        } else {
            Vec::new()
        };
        // Keep the established healthy-frame proof, then execute the actual
        // NULL LinkEntity import as the final call. A service rejection stops
        // that child/module; it is not a resumable native export result.
        let counter = base + 0x1c38;
        game.vm.process.memory_mut().unwrap()[0x1c38..0x1c3c].fill(0);
        frame_code.extend([0x48, 0xb8]);
        frame_code.extend(counter.to_le_bytes());
        frame_code.extend([
            0xff,
            0x00,
            0x83,
            0x38,
            if rr { 5 } else { 2 },
            0x73,
            1,
            0xc3,
        ]);
        frame_code.extend(native_import_code(pointers[link_slot], &[0], false, &[], 0));
        game.vm.process.memory_mut().unwrap()[code_at..code_at + frame_code.len()]
            .copy_from_slice(&frame_code);
        // A session-owned frame is still callable after the trap.
        let imports_table = game.imports;
        let mut request = request(
            &mut runtime,
            ModuleId(1),
            rules,
            TickRate::fixed(if rr { 25 } else { 100 }).unwrap(),
            game.vm,
        );
        request.configstrings = if rr { 10814 } else { 800 };
        request.entries = (0..1 + count as u32).collect();
        request.frame = Export {
            client: None,
            callback: CallbackId(run),
            arguments: {
                let mut arguments = [Argument::Word(0); 9];
                if rr {
                    arguments[0] = Argument::Word(1);
                }
                arguments
            },
        };
        if let Program::Native { imports, .. } = &mut request.program {
            *imports = imports_table;
        }
        let mut host =
            FrameHost::load_modules(console, runtime, TickRate::FrameDriven, vec![request])
                .unwrap();
        let mut source = Source {
            time: EventTime(0),
            polls: 0,
        };
        host.frame(&mut source, true);
        source.time = EventTime(100_000_000);
        host.frame(&mut source, true);
        assert_eq!(host.module_state(ModuleId(1)), Some(State::Running));
        assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 0);
        assert!(host.module_counts(ModuleId(1)).unwrap().calls > 0);
        let active_before = host.runtime.server.entities.active().count();
        source.time = EventTime(200_000_000);
        host.frame(&mut source, true);
        assert_eq!(host.module_state(ModuleId(1)), Some(State::Failed));
        assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 1);
        assert_eq!(host.runtime.server.entities.active().count(), active_before);
    }
}

fn returned_native_tables_bind_once_and_isolate_bad_apis() {
    use qa_app::modules::ApiCheck;
    use qa_compat::native::{ReturnedTable, TableFunction};
    for bad in 0..5 {
        let mut runtime = Runtime::load(1, std::iter::empty()).unwrap();
        let mut requests = Vec::new();
        for id in [ModuleId(1), ModuleId(2)] {
            let mut file = q2_table_file();
            if id == ModuleId(1) {
                match bad {
                    0 => pe_rva(&mut file, 0x1800, 99, 4),
                    1 => pe_rva(&mut file, 0x1800 + 112, 0, 8),
                    2 => pe_rva(&mut file, 0x1800 + 112, 0x180000080, 8),
                    // Return a table that extends beyond the image, or an
                    // unaligned address. Both reject before initialization.
                    3 => pe_rva(&mut file, 0x1083, 0xf69, 4),
                    _ => pe_rva(&mut file, 0x1083, 0x77a, 4),
                }
            }
            let image = Image::parse(&file, None, LoadRole::Library).unwrap();
            let mut vm = Vm::map_image(
                image,
                &[NamedExport {
                    name: b"GetGameAPI",
                    command: None,
                    parameters: &[NativeScalar::Word],
                    result: NativeScalar::Word,
                }],
                &[],
                Duration::from_secs(3),
            )
            .unwrap();
            let first = vm
                .declare_table(ReturnedTable {
                    version: 3,
                    bytes: 152,
                    functions: [8, 16, 112]
                        .map(|offset| TableFunction {
                            offset,
                            parameters: &[],
                            result: NativeScalar::Void,
                        })
                        .into(),
                })
                .unwrap();
            assert_eq!(first, 1);
            assert_eq!(vm.table_address(), None);
            let mut request = request(
                &mut runtime,
                id,
                RuleSetId::Quake2,
                TickRate::fixed(100).unwrap(),
                vm,
            );
            request.entries = (0..4).collect();
            request.context.clock = ThinkTime::Seconds(0.0);
            request.frame = Export::clocked(CallbackId(first + 2));
            let export = |callback| Export {
                client: None,
                callback: CallbackId(callback),
                arguments: [Argument::Word(0); 9],
            };
            request.api = Some(ApiCheck::NativeTable { export: export(0) });
            request.initialize = vec![export(first)];
            request.shutdown = vec![export(first + 1)];
            requests.push(request);
        }
        let mut host = FrameHost::load_modules(
            Console::new(Context::default()).unwrap(),
            runtime,
            TickRate::FrameDriven,
            requests,
        )
        .unwrap();
        let mut source = Source {
            time: EventTime(0),
            polls: 0,
        };
        for (tick, calls) in [(0, 2), (100, 3), (200, 4)] {
            source.time = EventTime(tick * 1_000_000);
            host.frame(&mut source, true);
            assert_eq!(host.module_state(ModuleId(1)), Some(State::Failed));
            assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, 1);
            assert_eq!(host.module_state(ModuleId(2)), Some(State::Running));
            assert_eq!(host.module_counts(ModuleId(2)).unwrap().calls, calls);
            assert_eq!(host.module_counts(ModuleId(2)).unwrap().traps, 0);
        }
        host.shutdown_modules();
        assert_eq!(host.module_state(ModuleId(2)), Some(State::Stopped));
        assert_eq!(host.module_counts(ModuleId(2)).unwrap().calls, 5);
        assert_eq!(host.module_counts(ModuleId(2)).unwrap().traps, 0);
        host.shutdown_modules();
        assert_eq!(host.module_counts(ModuleId(2)).unwrap().calls, 5);
    }
}

fn native_initialization_sequences_follow_api_binding_and_isolate_failure() {
    use qa_app::modules::ApiCheck;
    use qa_compat::native::{ReturnedTable, TableFunction};
    let mut runtime = Runtime::load(1, std::iter::empty()).unwrap();
    let mut requests = Vec::new();
    for id in [ModuleId(1), ModuleId(2)] {
        let mut file = q2_table_file();
        for (index, offset) in [8, 24, 32, 112, 16].into_iter().enumerate() {
            let rva = 0x1400 + index * 0x40;
            pe_rva(&mut file, 0x1800 + offset, 0x180000000 + rva as u64, 8);
            let mut code = vec![0x83, 0x3d]; // cmp counter, expected
            code.extend_from_slice(&((0x1900 - (rva + 7)) as i32).to_le_bytes());
            code.extend_from_slice(&[index as u8, 0x75, 11, 0xc7, 0x05]);
            code.extend_from_slice(&((0x1900 - (rva + 19)) as i32).to_le_bytes());
            code.extend_from_slice(&((index + 1) as u32).to_le_bytes());
            code.extend_from_slice(&[0xc3, 0x0f, 0x0b]);
            if id == ModuleId(1) && index == 1 {
                code[..2].copy_from_slice(&[0x0f, 0x0b]);
            }
            pe_text(&mut file, rva, &code);
        }
        let mut vm = Vm::map_image(
            Image::parse(&file, None, LoadRole::Library).unwrap(),
            &[NamedExport {
                name: b"GetGameAPI",
                command: None,
                parameters: &[NativeScalar::Word],
                result: NativeScalar::Word,
            }],
            &[],
            Duration::from_secs(3),
        )
        .unwrap();
        let first = vm
            .declare_table(ReturnedTable {
                version: 3,
                bytes: 152,
                functions: [8, 24, 32, 112, 16]
                    .map(|offset| TableFunction {
                        offset,
                        parameters: &[],
                        result: NativeScalar::Void,
                    })
                    .into(),
            })
            .unwrap();
        let mut request = request(
            &mut runtime,
            id,
            RuleSetId::Quake2,
            TickRate::fixed(100).unwrap(),
            vm,
        );
        let export = |callback| Export {
            client: None,
            callback: CallbackId(callback),
            arguments: [Argument::Word(0); 9],
        };
        request.entries = (0..first + 5).collect();
        request.api = Some(ApiCheck::NativeTable { export: export(0) });
        request.initialize = (first..first + 3).map(export).collect();
        request.frame = export(first + 3);
        request.shutdown = vec![export(first + 4)];
        requests.push(request);
    }
    let mut host = FrameHost::load_modules(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        requests,
    )
    .unwrap();
    host.initialize_modules(Phase::Server);
    assert_eq!(host.module_state(ModuleId(1)), Some(State::Failed));
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, 3);
    assert_eq!(host.module_state(ModuleId(2)), Some(State::Running));
    assert_eq!(host.module_counts(ModuleId(2)).unwrap().calls, 4);
    assert_eq!(host.module_counts(ModuleId(2)).unwrap().traps, 0);
    host.initialize_modules(Phase::Server);
    let mut source = Source {
        time: EventTime(0),
        polls: 0,
    };
    host.frame(&mut source, true);
    source.time = EventTime(100_000_000);
    host.frame(&mut source, true);
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, 3);
    assert_eq!(host.module_counts(ModuleId(2)).unwrap().calls, 5);
    host.shutdown_modules();
    assert_eq!(host.module_state(ModuleId(2)), Some(State::Stopped));
    assert_eq!(host.module_counts(ModuleId(2)).unwrap().calls, 6);
    assert_eq!(host.module_counts(ModuleId(2)).unwrap().traps, 0);
    host.shutdown_modules();
    assert_eq!(host.module_counts(ModuleId(2)).unwrap().calls, 6);
}
struct Source {
    time: EventTime,
    polls: u32,
}
impl FrameSource for Source {
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
        Duration::from_nanos(self.time.0)
    }
    fn elapsed(&self) -> Duration {
        Duration::ZERO
    }
    fn present(&mut self) {}
}

fn two_native_modules_use_session_rates_and_the_same_calltable_output_ring(functions: bool) {
    let system_v = [NativeImport {
        format: None,
        filter: None,
        trap: false,
        number: 0,
        abi: NativeAbi::SystemV,
        parameters: &[NativeScalar::Word],
        result: NativeScalar::Word,
    }];
    let microsoft = [NativeImport {
        format: None,
        filter: None,
        trap: false,
        number: 0,
        abi: NativeAbi::Microsoft,
        parameters: &[NativeScalar::Word],
        result: NativeScalar::Word,
    }];
    let mut runtime = Runtime::load(1, std::iter::empty()).unwrap();
    let observer = runtime
        .server
        .events
        .bind(OutputTarget::Module(ModuleId(7)))
        .unwrap();
    let q3 = module(
        &mut runtime,
        ModuleId(1),
        RuleSetId::Quake3,
        TickRate::fixed(50).unwrap(),
        b"q3 native\n\0",
        Encoding::Elf,
        if functions { &system_v } else { &[] },
    );
    let q2 = module(
        &mut runtime,
        ModuleId(2),
        RuleSetId::Quake2,
        TickRate::fixed(100).unwrap(),
        b"q2 native\n\0",
        Encoding::Pe,
        if functions { &microsoft } else { &[] },
    );
    let mut host = FrameHost::load_modules(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        vec![q3, q2],
    )
    .unwrap();
    let mut source = Source {
        time: EventTime(0),
        polls: 0,
    };
    let mut presented_prints = 0;
    for time in [0, 50, 100, 150] {
        source.time = EventTime(time * 1_000_000);
        presented_prints += host.frame(&mut source, true).output.prints;
    }
    assert_eq!(source.polls, 8);
    assert_eq!(presented_prints, 4);
    for (id, count) in [(ModuleId(1), 3), (ModuleId(2), 1)] {
        assert_eq!(host.module_state(id), Some(State::Running));
        let counters = host.module_counts(id).unwrap();
        assert_eq!((counters.calls, counters.traps), (count + 1, 0));
        assert_eq!(counters.last_result, Some(ModuleResult::Native(0)));
    }
    let mut batch = host.runtime.server.events.batch(observer).unwrap();
    assert!(host.runtime.server.events.next(&mut batch).is_none());
    assert!(host.runtime.server.events.is_empty());
    // Direct session exports retain the actual owned-child print bytes before
    // the next frame's presentation pass, for both native calling conventions.
    for module in [ModuleId(1), ModuleId(2)] {
        host.call_module_export(
            module,
            Export::words(CallbackId(0), &[]),
            ThinkTime::Milliseconds(150),
        )
        .unwrap();
    }
    let consumer = host.runtime.server.presentation;
    let mut batch = host.runtime.server.events.batch(consumer).unwrap();
    let mut prints = Vec::new();
    while let Some(record) = host.runtime.server.events.next(&mut batch) {
        if let FrameEvent::Print(p) = record.event {
            prints.push(
                host.runtime
                    .server
                    .events
                    .texts
                    .get(p.text)
                    .unwrap()
                    .to_vec(),
            );
        }
        assert!(host.runtime.server.events.submit(
            consumer,
            record.sequence,
            qa_core::events::OutputSubmission::BestEffort,
        ));
    }
    assert_eq!(
        prints,
        vec![b"q3 native\n".to_vec(), b"q2 native\n".to_vec()]
    );
    assert!(host.runtime.server.events.is_empty());
}

fn native_windows_counter_uses_host_event_time_in_session_dispatch() {
    let mut file = runtime_file(Encoding::Pe, b"QueryPerformanceCounter\0");
    pe_text(&mut file, 0x1280, b"KERNEL32.dll\0");
    let mut code = vec![0x48, 0x83, 0xec, 40, 0x48, 0x8d, 0x0d];
    code.extend(((512 + 0x790) as i32 - (640 + code.len() + 4) as i32).to_le_bytes());
    code.extend([0xff, 0x15]);
    code.extend(((512 + 0x780) as i32 - (640 + code.len() + 4) as i32).to_le_bytes());
    code.extend([0x48, 0x8b, 0x05]);
    code.extend(((512 + 0x790) as i32 - (640 + code.len() + 4) as i32).to_le_bytes());
    code.extend([0x48, 0x83, 0xc4, 40, 0xc3]);
    file[640..640 + code.len()].copy_from_slice(&code);
    let image = Image::parse(&file, None, LoadRole::Library).unwrap();
    let vm = Vm::map_image(
        image,
        &[NamedExport {
            name: b"GetGameAPI",
            command: None,
            parameters: &[NativeScalar::Word; 13],
            result: NativeScalar::Word,
        }],
        &[],
        Duration::from_secs(3),
    )
    .unwrap();
    let mut runtime = Runtime::load(1, std::iter::empty()).unwrap();
    let request = request(
        &mut runtime,
        ModuleId(1),
        RuleSetId::Quake3,
        TickRate::fixed(50).unwrap(),
        vm,
    );
    let mut host = FrameHost::load_modules(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        vec![request],
    )
    .unwrap();
    let mut source = Source {
        time: EventTime(0),
        polls: 0,
    };
    host.frame(&mut source, true);
    for time in [50_000_000, 125_678_999, 200_123_456] {
        source.time = EventTime(time);
        host.frame(&mut source, true);
        let counts = host.module_counts(ModuleId(1)).unwrap();
        assert_eq!(counts.traps, 0);
        assert_eq!(counts.last_result, Some(ModuleResult::Native(time)));
    }
}

fn pe_static_tls_template_is_owned_and_aligned_before_child_start() {
    use qa_formats::program::native::Tls;
    let code = [
        0x65, 0x48, 0x8b, 0x04, 0x25, 0x58, 0, 0, 0, 0x48, 0x8b, 0, 0x48, 0x8b, 0, 0xc3,
    ];
    for (file_bytes, zero_bytes, alignment) in [(8, 16, 4), (8, 32, 16384), (0, 0, 1)] {
        let mut image = function_image(Encoding::Pe, &code);
        let base = image.base;
        image.bytes[0x1700..0x1708].copy_from_slice(b"TLS data");
        image.bytes[0x1740..0x1744].copy_from_slice(&99u32.to_le_bytes());
        image.tls = Some(Tls {
            address: if file_bytes == 0 { 0 } else { base + 0x1700 },
            file_bytes,
            zero_bytes,
            index: Some(base + 0x1740),
            alignment,
        });
        let mut vm = Vm::map_image(
            image,
            &[NamedExport {
                name: b"GetGameAPI",
                parameters: &[],
                result: NativeScalar::Word,
                command: None,
            }],
            &[],
            Duration::from_secs(3),
        )
        .unwrap();
        let memory = vm.process.memory().unwrap();
        assert_eq!(&memory[0x1740..0x1744], &[0; 4]);
        // The CRT page precedes the TEB; static TLS slot zero points at the
        // aligned copy, not at the original module's template.
        let teb = base + 0x3000;
        let vector = (teb - base) as usize + qa_platform::native::runtime::STATIC_TLS_OFFSET;
        let tls = u64::from_le_bytes(memory[vector..vector + 8].try_into().unwrap());
        assert_eq!(tls as usize % alignment.max(16), 0);
        let at = (tls - base) as usize;
        assert_eq!(&memory[at..at + file_bytes], &b"TLS data"[..file_bytes]);
        assert!(
            memory[at + file_bytes..at + file_bytes + zero_bytes]
                .iter()
                .all(|&b| b == 0)
        );
        let entry = vm
            .process
            .bind(base + 0x1080, NativeAbi::Microsoft, &[], NativeScalar::Word)
            .unwrap();
        assert_eq!(
            vm.process
                .invoke(entry, [0; 13], |_, _, _| panic!("no engine import"))
                .unwrap(),
            if file_bytes == 0 {
                0
            } else {
                u64::from_le_bytes(*b"TLS data")
            }
        );
    }
}

mod commands;
#[path = "observations.rs"]
mod observations;
#[path = "pmove.rs"]
mod pmove;

#[path = "appearance.rs"]
mod appearance;

pub fn run() {
    if std::env::args().any(|arg| arg == "--q3-game-data") {
        for (variable, bits) in [
            ("QA_NATIVE_Q3_ELF32_FIXTURE", 32),
            ("QA_NATIVE_Q3_ELF64_FIXTURE", 64),
        ] {
            let path = std::path::PathBuf::from(
                std::env::var_os(variable).expect("explicit compiled original Q3 fixture"),
            );
            registered_q3::original_native_q3_exports_use_the_shared_session_and_call_table(
                &path, bits,
            );
        }
        println!("native Q3 registered-game-data checks passed for 32/64");
        return;
    }
    if std::env::args().any(|arg| arg == "--q2-sound-routes") {
        sound_routes::server_sounds_wait_for_resource_ack_without_stalling_other_clients();
        sound_routes::unsupported_resource_names_drop_only_the_unrepresentable_client_delivery();
        sound_routes::world_edict_sounds_keep_the_native_channel_replacement_key();
        sound_routes::remote_native_emitters_keep_the_module_entity_channel();
        sound_routes::secondary_modules_share_the_connection_sound_namespace();
        sound_routes::native_sound_ordinals_and_retained_captures_survive_resource_rewrites();
        sound_routes::split_seat_native_dupes_wait_for_ack_and_play_once();
        println!("native Q2 sound route checks passed");
        return;
    }
    if std::env::args().any(|arg| arg == "--q2-appearance") {
        appearance::native_appearance_uses_common_lifetimes_and_player_state();
        println!("native Q2 appearance checks passed");
        return;
    }
    if std::env::args().any(|arg| arg == "--q2-command-stream") {
        commands::packet_commands_keep_native_order_and_borrowed_arguments();
        println!("native Q2 packet command stream checks passed");
        return;
    }
    if std::env::args().any(|arg| arg == "--q2-api") {
        q2_api_layouts_bind_the_full_table_and_name_missing_engine_services();
        println!("native Q2 API checks passed");
        return;
    }
    if std::env::args().any(|arg| arg == "--snapshot-observations") {
        observations::snapshot_frames_follow_native_provider_lifetimes();
        println!("native snapshot observation checks passed");
        return;
    }
    if std::env::args().any(|arg| arg == "--q2-client-commands") {
        q2_clients_bind_after_initialization_and_disconnect_before_shutdown();
        pmove::commands_move_a_native_client_once();
        println!("native Q2 client command checks passed");
        return;
    }
    appearance::native_appearance_uses_common_lifetimes_and_player_state();
    pmove::commands_move_a_native_client_once();
    commands::packet_commands_keep_native_order_and_borrowed_arguments();
    observations::snapshot_frames_follow_native_provider_lifetimes();
    native_area_connection_uses_shared_topology_through_session_dispatch();
    native_visibility_uses_compiled_pvs_through_session_dispatch();
    native_usercmd_uses_session_dispatch_and_native_client_identity();
    pe_lifecycle_uses_session_order_and_rejects_false_attach();
    pe_lifecycle_rejects_unmapped_callbacks_and_read_only_tls_indices();
    native_windows_counter_uses_host_event_time_in_session_dispatch();
    pe_static_tls_template_is_owned_and_aligned_before_child_start();
    for functions in [false, true] {
        two_native_modules_use_session_rates_and_the_same_calltable_output_ring(functions);
    }
    checked_exports_preserve_names_and_native_command_arguments();
    elf_relro_protects_complete_pages_and_keeps_adjacent_pages_writable();
    native_files_use_the_qvm_role_policy_and_vfs_loader();
    runtime_binding_respects_native_library_names_and_versions();
    missing_native_imports_log_once_and_preserve_session_dispatch();
    scalar_native_export_results_survive_session_dispatch();
    native_math_imports_execute_in_owned_children_through_session_dispatch();
    native_snap_imports_execute_through_session_dispatch();
    native_heap_imports_share_owned_memory_through_session_dispatch();
    elf_lifecycle_uses_session_order_once_and_a_bad_constructor_stops_only_its_module();
    elf_lifecycle_rejects_malformed_arrays_and_non_executable_targets();
    returned_native_tables_bind_once_and_isolate_bad_apis();
    native_initialization_sequences_follow_api_binding_and_isolate_failure();
    q2_api_layouts_bind_the_full_table_and_name_missing_engine_services();
    sound_routes::server_sounds_wait_for_resource_ack_without_stalling_other_clients();
    sound_routes::unsupported_resource_names_drop_only_the_unrepresentable_client_delivery();
    sound_routes::world_edict_sounds_keep_the_native_channel_replacement_key();
    sound_routes::remote_native_emitters_keep_the_module_entity_channel();
    sound_routes::secondary_modules_share_the_connection_sound_namespace();
    sound_routes::native_sound_ordinals_and_retained_captures_survive_resource_rewrites();
    sound_routes::split_seat_native_dupes_wait_for_ack_and_play_once();
    q2_normal_loader_selects_native_roles_and_copies_spawn_strings();
    q2_clients_bind_after_initialization_and_disconnect_before_shutdown();
    println!("native session dispatch checks passed");
}
