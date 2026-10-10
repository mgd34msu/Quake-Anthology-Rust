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

fn function_image(encoding: Encoding, code: &[u8]) -> Image {
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
            Image::parse(&file, Some(0x2000_0000), LoadRole::Library).unwrap()
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
            Image::parse(&file, None, LoadRole::Library).unwrap()
        }
    }
}

fn module(
    runtime: &mut Runtime,
    id: ModuleId,
    rules: RuleSetId,
    rate: TickRate,
    text: &[u8],
    encoding: Encoding,
) -> ModuleRequest {
    // Load syscall pointer from shared RAM, print its shared text, then return.
    // SysV: sub rsp,8; mov rax,[rip+1781]; mov edi,0; lea rsi,[rip+1785]; call rax; add rsp,8; ret
    // Microsoft: reserve the 32-byte shadow area too, and use rcx/rdx.
    let mut code = [
        0x48, 0x83, 0xec, 8, 0x48, 0x8b, 0x05, 0xf5, 0x06, 0, 0, 0xbf, 0, 0, 0, 0, 0x48, 0x8d,
        0x35, 0xf9, 0x06, 0, 0, 0xff, 0xd0, 0x48, 0x83, 0xc4, 8, 0xc3,
    ];
    let (name, abi) = match encoding {
        Encoding::Elf => (b"vmMain".as_slice(), NativeAbi::SystemV),
        Encoding::Pe => {
            code[3] = 40;
            code[11] = 0xb9;
            code[18] = 0x15;
            code[28] = 40;
            (b"GetGameAPI".as_slice(), NativeAbi::Microsoft)
        }
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
    let pointer = vm.process.callback(abi);
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
    println!("native session dispatch checks passed");
}
