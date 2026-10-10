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
use qa_platform::native::{NativeAbi, NativeImport, NativeScalar};
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

pub fn run() {
    for functions in [false, true] {
        two_native_modules_use_session_rates_and_the_same_calltable_output_ring(functions);
    }
    checked_exports_preserve_names_and_native_command_arguments();
    elf_relro_protects_complete_pages_and_keeps_adjacent_pages_writable();
    native_files_use_the_qvm_role_policy_and_vfs_loader();
    runtime_binding_rejections_report_native_library_names_and_versions();
    scalar_native_export_results_survive_session_dispatch();
    native_math_imports_execute_in_owned_children_through_session_dispatch();
    println!("native session dispatch checks passed");
}
