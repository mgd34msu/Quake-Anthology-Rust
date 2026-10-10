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
        frame: CallbackId(0),
        prepare: Vec::new(),
        initialize: None,
        api: None,
        shutdown: Vec::new(),
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
    let (file, _) = elf_symbol_fixture(32);
    let image = Image::parse(&file, Some(0x2000_0000), LoadRole::Library).unwrap();
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
    host.frame(&mut source, true);
    source.time = EventTime(50_000_000);
    host.frame(&mut source, true);
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

fn runtime_binding_rejections_report_native_library_names_and_versions() {
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
            let error = Vm::map_image(
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
            .err()
            .expect("unsupported runtime binding");
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
            request.frame = CallbackId(first + 2);
            let export = |callback| Export {
                callback: CallbackId(callback),
                arguments: [Argument::Word(0); 9],
            };
            request.api = Some(ApiCheck::NativeTable { export: export(0) });
            request.initialize = Some(export(first));
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

fn two_native_modules_use_session_rates_and_the_same_calltable_output_ring(functions: bool) {
    let system_v = [NativeImport {
        number: 0,
        abi: NativeAbi::SystemV,
        parameters: &[NativeScalar::Word],
        result: NativeScalar::Word,
    }];
    let microsoft = [NativeImport {
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

pub fn run() {
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
    runtime_binding_rejections_report_native_library_names_and_versions();
    scalar_native_export_results_survive_session_dispatch();
    native_math_imports_execute_in_owned_children_through_session_dispatch();
    native_heap_imports_share_owned_memory_through_session_dispatch();
    elf_lifecycle_uses_session_order_once_and_a_bad_constructor_stops_only_its_module();
    elf_lifecycle_rejects_malformed_arrays_and_non_executable_targets();
    returned_native_tables_bind_once_and_isolate_bad_apis();
    println!("native session dispatch checks passed");
}
