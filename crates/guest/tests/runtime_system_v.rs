//! System V runtime tests: C++ streams, ELF lifecycle with real static
//! TLS, destructor ordering, and heap services.
//!
//! Donor: `tests/guest/runtime/system-v/runtime.test.ts` plus
//! `lifecycle-fixture.ts`. The real-Quake-Live suite needs supplied retail
//! archives outside the source boundary.

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use qa_guest::abi::runner::{GuestCallFailure, GuestCallRequest, GuestCallRunner};
use qa_guest::abi::GuestCpu;
use qa_guest::core::callbacks::{GuestHostCallback, HookState};
use qa_guest::core::contracts::{
    CallbackId, GuestAddress, GuestAllocationOptions, GuestArchitecture, GuestCallContext,
    GuestCallResult, GuestCallValue, GuestCallbackReference, GuestImage, GuestImport,
    GuestImportResolution, GuestPermissions, GuestStorage, GuestSymbolName, ModuleIdentity,
    NativeCallAbi,
};
use qa_guest::core::memory::SparseGuestMemory;
use qa_guest::core::registers::GuestProcessorState;
use qa_guest::core::registers::GuestProcessorInitialState;
use qa_guest::runtime::system_v::contracts::{symbol_key, system_v_signature, SystemVCapabilities, SystemVInitializeOptions, SystemVRuntimeOptions, SystemVStream};
use qa_guest::runtime::system_v::runtime::SystemVGuestRuntime;
use qa_guest::x64::cpu::X64Cpu;
use qa_guest::x86::cpu::I386Cpu;

use common::{map, test_module};

struct Setup {
    runner: GuestCallRunner<'static>,
    hooks: Rc<HookState>,
    runtime: SystemVGuestRuntime,
    context: GuestCallContext,
    module: ModuleIdentity,
    width: usize,
}

fn allocate(memory: &mut SparseGuestMemory, byte_length: usize) -> GuestAddress {
    memory
        .allocate(&GuestAllocationOptions {
            byte_length,
            alignment: 16,
            permissions: GuestPermissions::ReadWrite,
            label: "system-v-fixture".to_string(),
        })
        .unwrap()
}

fn output_name(stream: SystemVStream) -> &'static str {
    match stream {
        SystemVStream::Stdout => "stdout",
        SystemVStream::Stderr => "stderr",
    }
}

fn setup(width: usize, capabilities: SystemVCapabilities) -> Setup {
    let wide = width == 8;
    let module = test_module("system-v");
    let mut memory = SparseGuestMemory::new(module.clone(), width, 0x5000_0000).unwrap();
    map(&mut memory, 0x10000, 65536, GuestPermissions::ReadWrite, None);
    let sentinel = map(&mut memory, 0x30000, 4096, GuestPermissions::ReadExecute, Some(vec![0xcc]));
    let hooks = Rc::new(HookState::new());
    let runtime = SystemVGuestRuntime::new(
        Rc::clone(&hooks),
        &mut memory,
        SystemVRuntimeOptions { capabilities, argv: vec![], environment: vec![] },
    )
    .unwrap();
    let state = GuestProcessorState::create(GuestProcessorInitialState {
        architecture: if wide { GuestArchitecture::X86_64 } else { GuestArchitecture::I386 },
        instruction_pointer: sentinel.offset,
        stack_pointer: 0x20000,
        flags: 2,
        x87_control_word: 0x37f,
        mxcsr: 0x1f80,
        mxcsr_mask: 0xffff,
    })
    .unwrap();
    let boxed: Box<dyn GuestCpu> = if wide {
        Box::new(X64Cpu::new(state, memory).unwrap())
    } else {
        Box::new(I386Cpu::new(state, memory).unwrap())
    };
    let leaked: &'static mut dyn GuestCpu = Box::leak(boxed);
    let mut runner = GuestCallRunner::new(leaked, Rc::clone(&hooks), sentinel, None).unwrap();
    runtime.attach_runner(&mut runner).unwrap();
    let abi = if wide { NativeCallAbi::SystemVX86_64 } else { NativeCallAbi::SystemVI386 };
    let context = GuestCallContext {
        module: module.clone(),
        callback: GuestCallbackReference::NativeGuest { module: module.clone(), address: sentinel, abi },
        parent: None,
        itself: None,
        other: None,
    };
    Setup { runner, hooks, runtime, context, module, width }
}

fn service_key(library: &str, name: &str, version: Option<&str>) -> String {
    symbol_key(library, name, version)
}

fn call(setup: &mut Setup, library: &str, name: &str, version: Option<&str>, args: Vec<GuestCallValue>) -> GuestCallResult {
    let key = service_key(library, name, version);
    let address = setup
        .hooks
        .callbacks
        .borrow()
        .address(&CallbackId::new("system-v", &key))
        .unwrap_or_else(|| panic!("missing service {key}"));
    let signature = {
        let (_, memory) = setup.runner.cpu_parts();
        setup.hooks.callbacks.borrow_mut().handle(memory, address).unwrap().unwrap().signature
    };
    let request = GuestCallRequest {
        target: address,
        signature,
        arguments: args,
        context: setup.context.clone(),
        instruction_budget: 1000,
    };
    setup.runner.invoke(&request).unwrap_or_else(|failure| panic!("{library}!{name} failed: {failure:?}"))
}

fn call_failure(setup: &mut Setup, library: &str, name: &str, version: Option<&str>, args: Vec<GuestCallValue>) -> GuestCallFailure {
    let key = service_key(library, name, version);
    let address = setup
        .hooks
        .callbacks
        .borrow()
        .address(&CallbackId::new("system-v", &key))
        .unwrap_or_else(|| panic!("missing service {key}"));
    let signature = {
        let (_, memory) = setup.runner.cpu_parts();
        setup.hooks.callbacks.borrow_mut().handle(memory, address).unwrap().unwrap().signature
    };
    let request = GuestCallRequest {
        target: address,
        signature,
        arguments: args,
        context: setup.context.clone(),
        instruction_budget: 1000,
    };
    setup.runner.invoke(&request).unwrap_err()
}

fn int32(result: &GuestCallResult) -> i32 {
    match result {
        GuestCallResult::Value(GuestCallValue::Int32(value)) => *value,
        other => panic!("expected int32, got {other:?}"),
    }
}

fn ptr(value: Option<GuestAddress>) -> GuestCallValue {
    GuestCallValue::Pointer(value)
}

#[test]
fn glibcxx_streams_share_file_bytes_virtuals_and_final_flushing() {
    for width in [4usize, 8usize] {
        let output: Rc<RefCell<Vec<(String, String)>>> = Rc::new(RefCell::new(Vec::new()));
        let flushes: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
        let input: Rc<RefCell<Vec<u8>>> = Rc::new(RefCell::new(vec![104, 105]));
        let capabilities = SystemVCapabilities {
            now_seconds: Some(Rc::new(|| 123456)),
            standard_input: Some({
                let input = Rc::clone(&input);
                Rc::new(move |maximum: usize| {
                    let mut pending = input.borrow_mut();
                    let take = pending.len().min(maximum);
                    pending.drain(..take).collect()
                })
            }),
            standard_output: Some({
                let output = Rc::clone(&output);
                Rc::new(move |stream: SystemVStream, bytes: &[u8]| {
                    output.borrow_mut().push((output_name(stream).to_string(), String::from_utf8_lossy(bytes).into_owned()));
                    bytes.len()
                })
            }),
            standard_flush: Some({
                let flushes = Rc::clone(&flushes);
                Rc::new(move |stream: SystemVStream| {
                    flushes.borrow_mut().push(output_name(stream).to_string());
                    0
                })
            }),
            output_is_terminal: false,
        };
        let mut setup = setup(width, capabilities);
        let object = {
            let (_, memory) = setup.runner.cpu_parts();
            allocate(memory, 1)
        };
        call(&mut setup, "libstdc++.so.6", "_ZNSt8ios_base4InitC1Ev", Some("GLIBCXX_3.4"), vec![ptr(Some(object))]);
        let refcount = setup.runtime.iostreams.refcount;
        let (_, memory) = setup.runner.cpu_parts();
        assert_eq!(memory.read_i32(refcount).unwrap(), 2);
        let stream_address = |setup: &mut Setup, name: &str| {
            let found = setup.runtime.iostreams.streams.iter().find(|stream| stream.name == name).unwrap_or_else(|| panic!("missing {name}"));
            (found.address, found.ios, found.wide)
        };
        let (cout_address, cout_ios, _) = stream_address(&mut setup, "cout");
        let (_, cin_ios, _) = stream_address(&mut setup, "cin");
        let (_, cerr_ios, _) = stream_address(&mut setup, "cerr");
        let (_, clog_ios, _) = stream_address(&mut setup, "clog");
        let (wcerr_address, wcerr_ios, wcerr_wide) = stream_address(&mut setup, "wcerr");
        let (tie, flags, buffer_offset) = if width == 4 { (112i64, 12i64, 120i64) } else { (216i64, 24i64, 232i64) };
        let (_, memory) = setup.runner.cpu_parts();
        let slot = memory.offset(cin_ios, tie).unwrap();
        assert_eq!(memory.read_pointer(slot).unwrap(), Some(cout_address));
        let slot = memory.offset(cerr_ios, tie).unwrap();
        assert_eq!(memory.read_pointer(slot).unwrap(), Some(cout_address));
        let slot = memory.offset(cerr_ios, flags).unwrap();
        assert_eq!(memory.read_u32(slot).unwrap(), 0x3002);
        let slot = memory.offset(cout_ios, flags).unwrap();
        assert_eq!(memory.read_u32(slot).unwrap(), 0x1002);
        let cerr_buffer = memory.offset(cerr_ios, buffer_offset).unwrap();
        let clog_buffer = memory.offset(clog_ios, buffer_offset).unwrap();
        assert_eq!(memory.read_pointer(cerr_buffer).unwrap(), memory.read_pointer(clog_buffer).unwrap());
        let locale = setup.runtime.iostreams.locale().clone();
        let (_, memory) = setup.runner.cpu_parts();
        assert_eq!(memory.read_i32(locale.implementation).unwrap(), 16);

        let invoke_virtual = |setup: &mut Setup, object: GuestAddress, slot: usize, args: Vec<GuestCallValue>| {
            let (table, target) = {
                let (_, memory) = setup.runner.cpu_parts();
                let table = memory.read_pointer(object).unwrap().expect("missing vtable");
                let slot_address = memory.offset(table, (slot * width) as i64).unwrap();
                (table, memory.read_pointer(slot_address).unwrap().expect("missing virtual method"))
            };
            let _ = table;
            let signature = {
                let (_, memory) = setup.runner.cpu_parts();
                setup.hooks.callbacks.borrow_mut().handle(memory, target).unwrap().unwrap().signature
            };
            let mut arguments = vec![ptr(Some(object))];
            arguments.extend(args);
            let request = GuestCallRequest {
                target,
                signature,
                arguments,
                context: setup.context.clone(),
                instruction_budget: 1000,
            };
            setup.runner.invoke(&request).unwrap_or_else(|failure| panic!("virtual call failed: {failure:?}"))
        };
        let upper = {
            let (_, memory) = setup.runner.cpu_parts();
            let slot = memory.offset(locale.ctype[0], (4 * width) as i64).unwrap();
            memory.read_pointer(slot).unwrap().expect("missing ctype uppercase table")
        };
        let result = invoke_virtual(&mut setup, locale.ctype[0], 2, vec![GuestCallValue::Int32(97)]);
        assert_eq!(int32(&result), 65);
        {
            let (_, memory) = setup.runner.cpu_parts();
            memory.write_i32(memory.offset(upper, 97 * 4).unwrap(), 90).unwrap();
        }
        let result = invoke_virtual(&mut setup, locale.ctype[0], 2, vec![GuestCallValue::Int32(97)]);
        assert_eq!(int32(&result), 90);
        {
            let (_, memory) = setup.runner.cpu_parts();
            memory.write_i32(memory.offset(upper, 97 * 4).unwrap(), 65).unwrap();
        }
        let result = invoke_virtual(&mut setup, locale.ctype[1], 2, vec![GuestCallValue::Uint32(0x2000), GuestCallValue::Uint32(32)]);
        assert_eq!(int32(&result), 1);
        let result = invoke_virtual(&mut setup, locale.ctype[1], 2, vec![GuestCallValue::Uint32(0x2000), GuestCallValue::Uint32(65)]);
        assert_eq!(int32(&result), 0);
        let widen = if width == 4 { 144 } else { 156 } + 65 * 4;
        {
            let (_, memory) = setup.runner.cpu_parts();
            memory.write_u32(memory.offset(locale.ctype[1], widen).unwrap(), 66).unwrap();
        }
        let result = invoke_virtual(&mut setup, locale.ctype[1], 10, vec![GuestCallValue::Int32(65)]);
        assert!(matches!(result, GuestCallResult::Value(GuestCallValue::Uint32(66))), "{result:?}");
        {
            let (_, memory) = setup.runner.cpu_parts();
            memory.write_u32(memory.offset(locale.ctype[1], widen).unwrap(), 65).unwrap();
        }
        let bytes = {
            let (_, memory) = setup.runner.cpu_parts();
            let bytes = allocate(memory, 2);
            memory.write(bytes, &[65, 66]).unwrap();
            bytes
        };
        let count = if width == 4 { GuestCallValue::Int32(2) } else { GuestCallValue::Int64(2) };
        // Borrow-safe virtual dispatch via a macro to avoid closure captures.
        macro_rules! vcall {
            ($setup:expr, $name:expr, $slot:expr, $args:expr) => {{
                let found = $setup.runtime.iostreams.streams.iter().find(|stream| stream.name == $name).unwrap();
                let (ios, wide_flag) = (found.ios, found.wide);
                let offset = if width == 4 { if wide_flag { 124 } else { 120 } } else { 232 };
                let buffer = {
                    let (_, memory) = $setup.runner.cpu_parts();
                    memory.read_pointer(memory.offset(ios, offset).unwrap()).unwrap().expect("missing stream buffer")
                };
                invoke_virtual($setup, buffer, $slot, $args)
            }};
        }
        vcall!(&mut setup, "cout", 12, vec![ptr(Some(bytes)), count]);
        assert!(output.borrow().is_empty());
        let stdout = setup.runtime.iostreams.stdio.stdout;
        call(&mut setup, "libc.so.6", "fputc", None, vec![GuestCallValue::Int32(67), ptr(Some(stdout))]);
        call(&mut setup, "libstdc++.so.6", "_ZNSo5flushEv", Some("GLIBCXX_3.4"), vec![ptr(Some(cout_address))]);
        assert_eq!(*output.borrow(), [("stdout".to_string(), "ABC".to_string())]);
        let result = vcall!(&mut setup, "cin", 9, vec![]);
        assert_eq!(int32(&result), 104);
        let result = vcall!(&mut setup, "cin", 10, vec![]);
        assert_eq!(int32(&result), 104);
        let result = vcall!(&mut setup, "cin", 10, vec![]);
        assert_eq!(int32(&result), 105);
        let result = vcall!(&mut setup, "cin", 11, vec![GuestCallValue::Int32(-1)]);
        assert_eq!(int32(&result), 105);
        let result = vcall!(&mut setup, "cin", 10, vec![]);
        assert_eq!(int32(&result), 105);
        let result = vcall!(&mut setup, "wcerr", 13, vec![GuestCallValue::Uint32(90)]);
        assert!(matches!(result, GuestCallResult::Value(GuestCallValue::Uint32(90))), "{result:?}");
        assert_eq!(output.borrow()[1], ("stderr".to_string(), "Z".to_string()));
        let preserved = {
            let (_, memory) = setup.runner.cpu_parts();
            memory.read_pointer(wcerr_address).unwrap()
        };
        call(&mut setup, "libstdc++.so.6", "_ZNSt8ios_base4InitC1Ev", Some("GLIBCXX_3.4"), vec![ptr(Some(object))]);
        let (_, memory) = setup.runner.cpu_parts();
        assert_eq!(memory.read_pointer(wcerr_address).unwrap(), preserved);
        assert_eq!(memory.read_i32(locale.implementation).unwrap(), 16);
        let _ = (wcerr_ios, wcerr_wide);
        let before = flushes.borrow().len();
        call(&mut setup, "libstdc++.so.6", "_ZNSt8ios_base4InitD1Ev", Some("GLIBCXX_3.4"), vec![ptr(Some(object))]);
        assert_eq!(flushes.borrow().len(), before);
        call(&mut setup, "libstdc++.so.6", "_ZNSt8ios_base4InitD1Ev", Some("GLIBCXX_3.4"), vec![ptr(Some(object))]);
        assert_eq!(
            flushes.borrow()[before..],
            ["stdout", "stdout", "stderr", "stderr", "stderr", "stdout", "stdout", "stderr", "stderr", "stderr"]
        );
        let (_, memory) = setup.runner.cpu_parts();
        assert_eq!(memory.read_i32(refcount).unwrap(), 1);
    }
}

fn eword(bytes: &mut [u8], wide: bool, offset: usize, value: u64) {
    if wide {
        bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    } else {
        #[allow(clippy::cast_possible_truncation)]
        bytes[offset..offset + 4].copy_from_slice(&(value as u32).to_le_bytes());
    }
}

fn ehalf(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn ele32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn eseg(bytes: &mut [u8], wide: bool, index: usize, kind: u32, offset: u64, size: u64, memory_size: u64, alignment: u64) {
    let start = 64 + index * if wide { 56 } else { 32 };
    ele32(bytes, start, kind);
    ele32(bytes, start + if wide { 4 } else { 24 }, 7);
    eword(bytes, wide, start + if wide { 8 } else { 4 }, offset);
    eword(bytes, wide, start + if wide { 16 } else { 8 }, offset);
    eword(bytes, wide, start + if wide { 32 } else { 16 }, size);
    eword(bytes, wide, start + if wide { 40 } else { 20 }, memory_size);
    eword(bytes, wide, start + if wide { 48 } else { 28 }, alignment);
}

fn lifecycle_elf(width: usize, bias: u64, callback: GuestAddress, counter: GuestAddress) -> Vec<u8> {
    let wide = width == 8;
    let mut bytes = vec![0u8; 4096];
    ele32(&mut bytes, 0, 0x464c_457f);
    bytes[4] = if wide { 2 } else { 1 };
    bytes[5] = 1;
    bytes[6] = 1;
    ehalf(&mut bytes, 16, 3);
    ehalf(&mut bytes, 18, if wide { 62 } else { 3 });
    ele32(&mut bytes, 20, 1);
    eword(&mut bytes, wide, if wide { 32 } else { 28 }, 64);
    ehalf(&mut bytes, if wide { 52 } else { 40 }, if wide { 64 } else { 52 });
    ehalf(&mut bytes, if wide { 54 } else { 42 }, if wide { 56 } else { 32 });
    ehalf(&mut bytes, if wide { 56 } else { 44 }, 3);
    let tags: &[(u64, u64)] =
        &[(12, 0x800), (13, 0x980), (25, 0x600), (27, (width * 2) as u64), (26, 0x640), (28, (width * 2) as u64), (0, 0)];
    eseg(&mut bytes, wide, 0, 1, 0, 4096, 4096, 4096);
    eseg(&mut bytes, wide, 1, 2, 0x200, (tags.len() * width * 2) as u64, (tags.len() * width * 2) as u64, width as u64);
    eseg(&mut bytes, wide, 2, 7, 0x700, 4, 32, 32);
    for (index, (tag, value)) in tags.iter().enumerate() {
        eword(&mut bytes, wide, 0x200 + index * width * 2, *tag);
        eword(&mut bytes, wide, 0x200 + index * width * 2 + width, *value);
    }
    eword(&mut bytes, wide, 0x600, bias + 0x880);
    eword(&mut bytes, wide, 0x600 + width, bias + 0x900);
    eword(&mut bytes, wide, 0x640, bias + 0xa00);
    eword(&mut bytes, wide, 0x640 + width, bias + 0xa80);
    ele32(&mut bytes, 0x700, 77);
    let immediate = |value: u64| (0..width).map(|index| ((value >> (index * 8)) & 255) as u8).collect::<Vec<_>>();
    for (offset, digit) in [(0x800usize, 1u8), (0x880, 2), (0x900, 3), (0xa00, 4), (0xa80, 5), (0x980, 6)] {
        let code: Vec<u8> = if wide {
            [vec![0x48, 0x83, 0xec, 8, 0x48, 0xbf], immediate(counter.offset), vec![0xbe, digit, 0, 0, 0, 0x48, 0xb8], immediate(callback.offset), vec![0xff, 0xd0, 0x48, 0x83, 0xc4, 8, 0xc3]].concat()
        } else {
            [vec![0x83, 0xec, 4, 0x6a, digit, 0x68], immediate(counter.offset), vec![0xb8], immediate(callback.offset), vec![0xff, 0xd0, 0x83, 0xc4, 12, 0xc3]].concat()
        };
        bytes[offset..offset + code.len()].copy_from_slice(&code);
    }
    bytes
}

fn default_capabilities() -> SystemVCapabilities {
    SystemVCapabilities {
        now_seconds: Some(Rc::new(|| 123456)),
        standard_input: None,
        standard_output: None,
        standard_flush: None,
        output_is_terminal: false,
    }
}

#[test]
fn elf_lifecycle_executes_ordered_callbacks_and_installs_static_tls() {
    for width in [4usize, 8usize] {
        let mut setup = setup(width, default_capabilities());
        let counter = {
            let (_, memory) = setup.runner.cpu_parts();
            allocate(memory, 4)
        };
        let signature = system_v_signature(width, &[GuestStorage::Pointer, GuestStorage::Int32], None);
        let callback = {
            let (_, memory) = setup.runner.cpu_parts();
            setup
                .hooks
                .callbacks
                .borrow_mut()
                .bind(
                    memory,
                    GuestHostCallback {
                        id: CallbackId::new("test", "record"),
                        signature,
                        invoke: Rc::new(move |ctx, _, args| {
                            let (Some(address), GuestCallValue::Int32(digit)) = (pointer_value(args, 0), &args[1]) else {
                                return Err(qa_guest::error::GuestError::invalid("Incorrect lifecycle callback arguments"));
                            };
                            let memory = ctx.memory();
                            let current = memory.read_i32(address)?;
                            memory.write_i32(address, current * 10 + digit)?;
                            Ok(GuestCallResult::Void)
                        }),
                    },
                )
                .unwrap()
        };
        let bias = 0x1000_0000;
        let module = setup.module.clone();
        let bytes = lifecycle_elf(width, bias, callback, counter);
        let image = {
            let Setup { runner, runtime, .. } = &mut setup;
            let (_, memory) = runner.cpu_parts();
            runtime.load(memory, &bytes, module.clone(), bias).unwrap()
        };
        // Version-mismatched imports resolve to the unsupported trap.
        let mismatch = {
            let Setup { runner, runtime, .. } = &mut setup;
            let (_, memory) = runner.cpu_parts();
            let import = GuestImport {
                library: "libc.so.6".to_string(),
                symbol: GuestSymbolName::Name { name: "malloc".to_string(), version: Some("GLIBC_UNIMPLEMENTED".to_string()) },
                slot: counter,
                weak: false,
            };
            <SystemVGuestRuntime as qa_guest::core::contracts::GuestImportResolver>::resolve(runtime, memory, &import, &image.image)
        };
        assert!(matches!(mismatch, GuestImportResolution::Host { .. }), "{mismatch:?}");
        let failure = call_failure(&mut setup, "libc.so.6", "malloc", Some("GLIBC_UNIMPLEMENTED"), vec![]);
        assert!(format!("{failure:?}").contains("exact symbol version"), "{failure:?}");
        let tls = {
            let Setup { runner, runtime, .. } = &mut setup;
            let (_, memory) = runner.cpu_parts();
            runtime.tls_address(memory, 1, 0).unwrap()
        };
        let (_, memory) = setup.runner.cpu_parts();
        assert_eq!(memory.read_u32(tls).unwrap(), 77);
        assert_eq!(tls.offset % 32, 0);
        assert_eq!(memory.copy(memory.offset(tls, 4).unwrap(), 28).unwrap(), vec![0; 28]);
        let (state, _) = setup.runner.cpu_parts();
        let segment = if width == 4 { state.segments[GuestProcessorState::GS].base } else { state.segments[GuestProcessorState::FS].base };
        assert_eq!(segment, setup.runtime.thread_pointer().offset);
        if width == 8 {
            let index = {
                let (_, memory) = setup.runner.cpu_parts();
                let index = allocate(memory, 16);
                memory.write_u64(index, 1).unwrap();
                memory.write_u64(memory.offset(index, 8).unwrap(), 4).unwrap();
                index
            };
            let result = call(&mut setup, "ld-linux-x86-64.so.2", "__tls_get_addr", Some("GLIBC_2.3"), vec![ptr(Some(index))]);
            let expected = {
                let (_, memory) = setup.runner.cpu_parts();
                memory.offset(tls, 4).unwrap()
            };
            assert!(matches!(result, GuestCallResult::Value(GuestCallValue::Pointer(Some(address))) if address == expected), "{result:?}");
        }
        let options = SystemVInitializeOptions { context: setup.context.clone(), instruction_budget: 1000 };
        {
            let Setup { runner, runtime, .. } = &mut setup;
            runtime.initialize(runner, &module, &options).unwrap();
        }
        let (_, memory) = setup.runner.cpu_parts();
        assert_eq!(memory.read_i32(counter).unwrap(), 123);
        {
            let Setup { runner, runtime, .. } = &mut setup;
            runtime.initialize(runner, &module, &options).unwrap();
        }
        let (_, memory) = setup.runner.cpu_parts();
        assert_eq!(memory.read_i32(counter).unwrap(), 123);
        {
            let Setup { runner, runtime, .. } = &mut setup;
            runtime.finalize(runner, &module, &options).unwrap();
        }
        let (_, memory) = setup.runner.cpu_parts();
        assert_eq!(memory.read_i32(counter).unwrap(), 123546);
        {
            let Setup { runner, runtime, .. } = &mut setup;
            runtime.finalize(runner, &module, &options).unwrap();
        }
        let trace: Vec<u64> = setup.runtime.lifecycle_trace().iter().map(|entry| entry.target.offset - bias).collect();
        assert_eq!(trace, [0x800, 0x880, 0x900, 0xa80, 0xa00, 0x980]);
    }
}

#[test]
fn cxa_finalize_invokes_destructors_once_in_reverse_registration_order() {
    let mut setup = setup(8, default_capabilities());
    let (counter, other_dso) = {
        let (_, memory) = setup.runner.cpu_parts();
        (allocate(memory, 4), allocate(memory, 1))
    };
    let signature = system_v_signature(8, &[GuestStorage::Pointer], None);
    let target = {
        let (_, memory) = setup.runner.cpu_parts();
        setup
            .hooks
            .callbacks
            .borrow_mut()
            .bind(
                memory,
                GuestHostCallback {
                    id: CallbackId::new("test", "destructor"),
                    signature,
                    invoke: Rc::new(move |ctx, _, args| {
                        let Some(value) = pointer_value(args, 0) else {
                            return Err(qa_guest::error::GuestError::invalid("Destructor argument missing"));
                        };
                        let memory = ctx.memory();
                        let digit = memory.read_i32(value)?;
                        let current = memory.read_i32(counter)?;
                        memory.write_i32(counter, current * 10 + digit)?;
                        Ok(GuestCallResult::Void)
                    }),
                },
            )
            .unwrap()
    };
    for digit in [1, 2, 3] {
        let argument = {
            let (_, memory) = setup.runner.cpu_parts();
            let argument = allocate(memory, 4);
            memory.write_i32(argument, digit).unwrap();
            argument
        };
        let dso = if digit == 2 { other_dso } else { counter };
        call(
            &mut setup,
            "libc.so.6",
            "__cxa_atexit",
            None,
            vec![ptr(Some(target)), ptr(Some(argument)), ptr(Some(dso))],
        );
    }
    call(&mut setup, "libc.so.6", "__cxa_finalize", None, vec![ptr(Some(counter))]);
    let (_, memory) = setup.runner.cpu_parts();
    assert_eq!(memory.read_i32(counter).unwrap(), 31);
    {
        let Setup { runner, runtime, context, .. } = &mut setup;
        runtime.finalize_destructors(runner, context, None).unwrap();
    }
    let (_, memory) = setup.runner.cpu_parts();
    assert_eq!(memory.read_i32(counter).unwrap(), 312);
    {
        let Setup { runner, runtime, context, .. } = &mut setup;
        runtime.finalize_destructors(runner, context, None).unwrap();
    }
    let (_, memory) = setup.runner.cpu_parts();
    assert_eq!(memory.read_i32(counter).unwrap(), 312);
}

#[test]
fn system_v_heap_checked_copying_guards_and_nested_qsort() {
    for width in [4usize, 8usize] {
        let mut setup = setup(width, default_capabilities());
        let size = |value: u64| {
            if width == 4 {
                #[allow(clippy::cast_possible_truncation)]
                GuestCallValue::Uint32(value as u32)
            } else {
                GuestCallValue::Uint64(value)
            }
        };
        let result = call(&mut setup, "libc.so.6", "malloc", None, vec![size(16)]);
        let GuestCallResult::Value(GuestCallValue::Pointer(Some(allocation))) = result else {
            panic!("malloc returned no guest allocation: {result:?}");
        };
        {
            let (_, memory) = setup.runner.cpu_parts();
            memory.write(allocation, &[7, 0, 0, 0, 2, 0, 0, 0, 9, 0, 0, 0, 1, 0, 0, 0]).unwrap();
        }
        let code: Vec<u8> = if width == 8 {
            vec![0x8b, 0x07, 0x2b, 0x06, 0xc3]
        } else {
            vec![0x8b, 0x44, 0x24, 4, 0x8b, 0x00, 0x8b, 0x54, 0x24, 8, 0x2b, 0x02, 0xc3]
        };
        {
            let (_, memory) = setup.runner.cpu_parts();
            map(memory, 0x40000, 4096, GuestPermissions::ReadExecute, Some(code));
        }
        let comparator = {
            let (_, memory) = setup.runner.cpu_parts();
            memory.pointer(0x40000).unwrap().unwrap()
        };
        call(
            &mut setup,
            "libc.so.6",
            "qsort",
            None,
            vec![ptr(Some(allocation)), size(4), size(4), ptr(Some(comparator))],
        );
        let (_, memory) = setup.runner.cpu_parts();
        let sorted: Vec<i32> = [0, 4, 8, 12].iter().map(|offset| memory.read_i32(memory.offset(allocation, *offset).unwrap()).unwrap()).collect();
        assert_eq!(sorted, [1, 2, 7, 9]);
        let reached = setup
            .runtime
            .coverage()
            .iter()
            .find(|entry| entry.name == "qsort" && entry.version.is_none())
            .map(|entry| entry.reached);
        assert_eq!(reached, Some(1));
        let failure = call_failure(
            &mut setup,
            "libc.so.6",
            "__memcpy_chk",
            None,
            vec![ptr(Some(allocation)), ptr(Some(allocation)), size(16), size(8)],
        );
        assert!(format!("{failure:?}").contains("buffer overflow"), "{failure:?}");
        let guard = {
            let (_, memory) = setup.runner.cpu_parts();
            allocate(memory, 8)
        };
        let result = call(&mut setup, "libstdc++.so.6", "__cxa_guard_acquire", None, vec![ptr(Some(guard))]);
        assert_eq!(int32(&result), 1);
        let failure = call_failure(&mut setup, "libstdc++.so.6", "__cxa_guard_acquire", None, vec![ptr(Some(guard))]);
        assert!(format!("{failure:?}").contains("recursive local static"), "{failure:?}");
        call(&mut setup, "libstdc++.so.6", "__cxa_guard_release", None, vec![ptr(Some(guard))]);
        let (_, memory) = setup.runner.cpu_parts();
        assert_eq!(memory.read_u8(guard).unwrap(), 1);
        let result = call(&mut setup, "libstdc++.so.6", "__cxa_guard_acquire", None, vec![ptr(Some(guard))]);
        assert_eq!(int32(&result), 0);
        call(&mut setup, "libc.so.6", "free", None, vec![ptr(Some(allocation))]);
        let (_, memory) = setup.runner.cpu_parts();
        assert!(memory.copy(allocation, 1).is_err());
    }
}

fn pointer_value(args: &[GuestCallValue], index: usize) -> Option<GuestAddress> {
    match args.get(index) {
        Some(GuestCallValue::Pointer(value)) => *value,
        _ => None,
    }
}
