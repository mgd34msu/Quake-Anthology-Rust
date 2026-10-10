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
use qa_core::{
    events::{FrameEvent, OutputTarget},
    primitives::{CallbackId, ModuleId, RuleSetId},
    sys_events::{EventKind, EventTime, SysEvent, SysEventQueue},
};
use qa_formats::program::native::{Encoding, Image, LoadRole};
use qa_platform::native::NativeAbi;
use qa_session::timing::TickRate;
use qa_world::entities::{AllocationPolicy, EntityTime};
use std::time::Duration;

#[path = "../../../formats/tests/support/native_image.rs"]
#[allow(dead_code)]
mod native_image;
use native_image::*;

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

fn print_code(encoding: Encoding) -> Vec<u8> {
    // Copy the shared message to a native local buffer, then call Print with
    // its stack pointer. This exercises the same lifetime as qsrc G_Printf.
    let (reserve, local, number, argument) = match encoding {
        Encoding::Elf => (24, 0, 0xbf, 0x74),
        Encoding::Pe => (56, 32, 0xb9, 0x54),
    };
    vec![
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
    ]
}

fn module(
    runtime: &mut Runtime,
    id: ModuleId,
    rules: RuleSetId,
    rate: TickRate,
    text: &[u8],
    encoding: Encoding,
) -> ModuleRequest {
    let code = print_code(encoding);
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
                name,
                command: None,
            },
            NamedExport {
                name: b"dllEntry",
                command: None,
            },
        ],
        Duration::from_secs(3),
    )
    .unwrap();
    let pointer = vm.import_callback();
    let memory = vm.process.memory_mut().expect("parked native backing");
    assert_eq!(&memory[at..at + 8], &[0; 8]);
    memory[at + 16..at + 16 + text.len()].copy_from_slice(text);
    let mut request = request(runtime, id, rules, rate, vm);
    request.prepare = Some(Export {
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
    });
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
        .allocate(EntityTime::Milliseconds(0), id, AllocationPolicy::EDICT)
        .unwrap()
        .id;
    ModuleRequest {
        context: CallContext {
            module: id,
            clock: EntityTime::Milliseconds(0),
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
        frame: CallbackId(0),
        prepare: None,
        initialize: None,
        api: None,
        shutdown: None,
        instruction_budget: 100,
        configstrings: 0,
        files: 0,
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
            name,
            command: Some(7),
        }];
        let vm = Vm::map_image(
            function_image(encoding, &code),
            &named,
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
            name: b"missing",
            command: None,
        }];
        assert!(matches!(
            Vm::map_image(
                function_image(encoding, &code),
                &missing,
                Duration::from_secs(3)
            ),
            Err(qa_compat::native::Error::Export)
        ));
        let folded = name.to_ascii_lowercase();
        assert!(matches!(
            Vm::map_image(
                function_image(encoding, &code),
                &[NamedExport {
                    name: &folded,
                    command: None
                }],
                Duration::from_secs(3)
            ),
            Err(qa_compat::native::Error::Export)
        ));
    }
    let (file, _) = elf_symbol_fixture(32);
    let image = Image::parse(&file, Some(0x2000_0000), LoadRole::Library).unwrap();
    assert!(matches!(
        Vm::map_image(
            image,
            &[NamedExport {
                name: b"vmMain",
                command: None
            }],
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
                name: b"vmMain",
                command: None
            }],
            Duration::from_secs(3)
        ),
        Err(qa_compat::native::Error::Export)
    ));
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
            name: b"vmMain",
            command: None,
        }],
        Duration::from_secs(3),
    )
    .unwrap();
    let mut words = [0; 13];
    words[0] = base + 0x3f00;
    words[1] = 123;
    assert_eq!(
        vm.process
            .invoke(entry, NativeAbi::SystemV, words, |_, _, _| Err(
                qa_platform::native::NativeError::Callback
            ))
            .unwrap(),
        123
    );
    words[0] = base + 0x4100;
    let result = vm
        .process
        .invoke(entry, NativeAbi::SystemV, words, |_, _, _| {
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
                name: b"vmMain",
                command: None
            }],
            Duration::from_secs(3)
        ),
        Err(qa_compat::native::Error::Export)
    ));
}

fn native_files_use_the_qvm_role_policy_and_vfs_loader() {
    use qa_app::modules::{Q3Spec, load_q3};
    struct Files(std::path::PathBuf);
    impl Drop for Files {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let files =
        Files(std::env::temp_dir().join(format!("qa-native-modules-{}", std::process::id())));
    std::fs::create_dir(&files.0).unwrap();
    let mut specs = Vec::new();
    for (encoding, file_name, message) in [
        (Encoding::Elf, "qagame.so", b"ELF cold\n\0".as_slice()),
        (Encoding::Pe, "qagame.dll", b"PE cold\n\0".as_slice()),
    ] {
        let code = print_code(encoding);
        let mut file = function_file(encoding, &code);
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
    )
    .unwrap();
    assert_eq!(requests.len(), 2);
    for request in &requests {
        assert!(matches!(request.program, Program::Native { .. }));
        assert_eq!(request.timing_rules, RuleSetId::Quake3);
        assert_eq!(request.phase, Phase::Server);
        assert_eq!(request.prepare.unwrap().callback, CallbackId(11));
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
    source.time = EventTime(50_000_000);
    host.frame(&mut source, true);
    host.shutdown_modules();
    for id in [ModuleId(1), ModuleId(2)] {
        assert_eq!(host.module_state(id), Some(State::Stopped));
        assert_eq!(
            (
                host.module_counts(id).unwrap().calls,
                host.module_counts(id).unwrap().traps
            ),
            (4, 0)
        );
    }
    let mut batch = host.runtime.server.events.batch(observer).unwrap();
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
        [b"ELF cold\n".as_slice(), b"PE cold\n".as_slice()].repeat(3)
    );
}
struct Source {
    time: EventTime,
    polls: u32,
}
impl FrameSource for Source {
    fn begin_frame(&mut self) -> EventTime {
        self.time
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
    fn wait_time(&mut self, _: Duration) -> EventTime {
        self.time
    }
    fn elapsed(&self) -> Duration {
        Duration::ZERO
    }
    fn present(&mut self) {}
}

fn two_native_modules_use_session_rates_and_the_same_calltable_output_ring() {
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
    );
    let q2 = module(
        &mut runtime,
        ModuleId(2),
        RuleSetId::Quake2,
        TickRate::fixed(100).unwrap(),
        b"q2 native\n\0",
        Encoding::Pe,
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
    for time in [0, 50, 100, 150] {
        source.time = EventTime(time * 1_000_000);
        host.frame(&mut source, true);
    }
    assert_eq!(source.polls, 8);
    for (id, count) in [(ModuleId(1), 3), (ModuleId(2), 1)] {
        assert_eq!(host.module_state(id), Some(State::Running));
        let counters = host.module_counts(id).unwrap();
        assert_eq!((counters.calls, counters.traps), (count + 1, 0));
        assert_eq!(counters.last_result, Some(ModuleResult::Native(0)));
    }
    let mut batch = host.runtime.server.events.batch(observer).unwrap();
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
    }
    assert_eq!(
        prints,
        vec![
            b"q3 native\n".to_vec(),
            b"q3 native\n".to_vec(),
            b"q2 native\n".to_vec(),
            b"q3 native\n".to_vec()
        ]
    );
}

pub fn run() {
    two_native_modules_use_session_rates_and_the_same_calltable_output_ring();
    checked_exports_preserve_names_and_native_command_arguments();
    elf_relro_protects_complete_pages_and_keeps_adjacent_pages_writable();
    native_files_use_the_qvm_role_policy_and_vfs_loader();
    println!("native session dispatch checks passed");
}
