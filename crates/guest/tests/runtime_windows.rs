//! Windows runtime service tests: heap/TLS/files, CRT memory and float
//! services, unsupported-import accounting, dynamic libraries, and PE
//! export lookup.
//!
//! Donor: `tests/guest/runtime/windows/services.test.ts`. The MSVC and
//! installed-DLL suites need user-supplied retail binaries outside the
//! source boundary.

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use qa_core::identity::ProviderId;
use qa_guest::abi::runner::{GuestCallFailure, GuestCallRequest, GuestCallRunner};
use qa_guest::abi::GuestCpu;
use qa_guest::core::callbacks::{GuestHostCallback, HookState};
use qa_guest::core::contracts::{
    CallbackId, GuestAddress, GuestAllocationOptions, GuestArchitecture, GuestCallContext, GuestCallResult,
    GuestCallValue, GuestCallbackReference, GuestImage, GuestImport, GuestImportResolution, GuestPermissions,
    GuestStorage, GuestSymbolName, ModuleIdentity, NativeAbi,
};
use qa_guest::core::memory::SparseGuestMemory;
use qa_guest::core::registers::{GuestProcessorInitialState, GuestProcessorState};
use qa_guest::pe::loader::{map_pe_image, MapPeImageOptions};
use qa_guest::runtime::windows::contracts::{
    windows_signature, WindowsCapabilities, WindowsFile, WindowsOpenOptions, WindowsRuntimeOptions,
};
use qa_guest::runtime::windows::runtime::WindowsGuestRuntime;
use qa_guest::x64::cpu::X64Cpu;
use qa_guest::x86::cpu::I386Cpu;

use common::{pe_fixture, test_module};

struct Setup {
    runner: GuestCallRunner<'static>,
    hooks: Rc<HookState>,
    runtime: WindowsGuestRuntime,
    context: GuestCallContext,
    module: ModuleIdentity,
    width: usize,
}

fn allocate(memory: &mut SparseGuestMemory, byte_length: usize, alignment: u64) -> GuestAddress {
    memory
        .allocate(&GuestAllocationOptions {
            byte_length,
            alignment,
            permissions: GuestPermissions::ReadWrite,
            label: "windows-fixture".to_string(),
        })
        .unwrap()
}

fn setup(width: usize, capabilities: WindowsCapabilities) -> Setup {
    let wide = width == 8;
    let module = test_module("windows-services");
    let mut memory = SparseGuestMemory::new(module.clone(), width, 0x10000).unwrap();
    let hooks = Rc::new(HookState::new());
    let runtime = WindowsGuestRuntime::new(
        Rc::clone(&hooks),
        &mut memory,
        WindowsRuntimeOptions {
            capabilities,
            process_id: None,
            thread_id: None,
        },
    )
    .unwrap();
    let stack = allocate(&mut memory, 65536, 16);
    let sentinel = memory
        .allocate(&GuestAllocationOptions {
            byte_length: 16,
            alignment: 16,
            permissions: GuestPermissions::ReadExecute,
            label: "sentinel".to_string(),
        })
        .unwrap();
    let state = GuestProcessorState::create(GuestProcessorInitialState {
        architecture: if wide {
            GuestArchitecture::X86_64
        } else {
            GuestArchitecture::I386
        },
        instruction_pointer: sentinel.offset,
        stack_pointer: stack.offset + 65536,
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
    let context = GuestCallContext {
        module: module.clone(),
        callback: GuestCallbackReference::TypeScript {
            provider: ProviderId::new("test", "windows-services"),
            callback: CallbackId::new("test", "entry"),
        },
        parent: None,
        itself: None,
        other: None,
    };
    Setup {
        runner,
        hooks,
        runtime,
        context,
        module,
        width,
    }
}

fn call(setup: &mut Setup, library: &str, name: &str, args: Vec<GuestCallValue>) -> GuestCallResult {
    let address = setup
        .runtime
        .resolve_address(library, name)
        .unwrap_or_else(|| panic!("missing service {library}!{name}"));
    let signature = {
        let (_, memory) = setup.runner.cpu_parts();
        setup
            .hooks
            .callbacks
            .borrow_mut()
            .handle(memory, address)
            .unwrap()
            .unwrap()
            .signature
    };
    let request = GuestCallRequest {
        target: address,
        signature,
        arguments: args,
        context: setup.context.clone(),
        instruction_budget: 1000,
    };
    setup
        .runner
        .invoke(&request)
        .unwrap_or_else(|failure| panic!("{library}!{name} failed: {failure:?}"))
}

fn call_failure(setup: &mut Setup, address: GuestAddress, args: Vec<GuestCallValue>) -> GuestCallFailure {
    let signature = {
        let (_, memory) = setup.runner.cpu_parts();
        setup
            .hooks
            .callbacks
            .borrow_mut()
            .handle(memory, address)
            .unwrap()
            .unwrap()
            .signature
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

fn ptr(value: Option<GuestAddress>) -> GuestCallValue {
    GuestCallValue::Pointer(value)
}

fn result_pointer(result: &GuestCallResult) -> GuestAddress {
    match result {
        GuestCallResult::Value(GuestCallValue::Pointer(Some(address))) => *address,
        other => panic!("expected nonnull pointer, got {other:?}"),
    }
}

struct FixtureFile {
    bytes: Rc<RefCell<Vec<u8>>>,
    closed: Rc<RefCell<bool>>,
}

impl WindowsFile for FixtureFile {
    fn read(&mut self, offset: usize, length: usize) -> Vec<u8> {
        let bytes = self.bytes.borrow();
        bytes[offset..(offset + length).min(bytes.len())].to_vec()
    }
    fn write(&mut self, offset: usize, bytes: &[u8]) -> usize {
        let mut current = self.bytes.borrow_mut();
        if current.len() < offset + bytes.len() {
            current.resize(offset + bytes.len(), 0);
        }
        current[offset..offset + bytes.len()].copy_from_slice(bytes);
        bytes.len()
    }
    fn size(&mut self) -> usize {
        self.bytes.borrow().len()
    }
    fn truncate(&mut self, length: usize) {
        self.bytes.borrow_mut().truncate(length);
    }
    fn flush(&mut self) {}
    fn close(&mut self) {
        *self.closed.borrow_mut() = true;
    }
}

#[test]
fn windows_heap_tls_and_file_services_mutate_guest_memory() {
    let bytes = Rc::new(RefCell::new(vec![10u8, 20, 30]));
    let closed = Rc::new(RefCell::new(false));
    let capabilities = WindowsCapabilities {
        open_file: Some({
            let bytes = Rc::clone(&bytes);
            let closed = Rc::clone(&closed);
            Rc::new(move |path: &str, _options: WindowsOpenOptions| {
                (path == "save.sav").then(|| {
                    Box::new(FixtureFile {
                        bytes: Rc::clone(&bytes),
                        closed: Rc::clone(&closed),
                    }) as Box<dyn WindowsFile>
                })
            })
        }),
        ..Default::default()
    };
    let mut setup = setup(4, capabilities);
    let encoded_null = call(&mut setup, "kernel32.dll", "EncodePointer", vec![ptr(None)]);
    assert_ne!(result_pointer(&encoded_null).offset, 0);
    let decoded = call(
        &mut setup,
        "kernel32.dll",
        "DecodePointer",
        vec![ptr(Some(result_pointer(&encoded_null)))],
    );
    assert!(
        matches!(decoded, GuestCallResult::Value(GuestCallValue::Pointer(None))),
        "{decoded:?}"
    );
    let original = {
        let (_, memory) = setup.runner.cpu_parts();
        memory.pointer(0x1234_5678).unwrap().unwrap()
    };
    // 0x12345678 is unmapped; EncodePointer only obscures the value.
    let encoded = call(&mut setup, "kernel32.dll", "EncodePointer", vec![ptr(Some(original))]);
    assert_ne!(encoded, GuestCallResult::Value(ptr(Some(original))));
    let repeat = call(&mut setup, "kernel32.dll", "EncodePointer", vec![ptr(Some(original))]);
    assert_eq!(repeat, encoded);
    let decoded = call(
        &mut setup,
        "kernel32.dll",
        "DecodePointer",
        vec![ptr(Some(result_pointer(&encoded)))],
    );
    assert_eq!(decoded, GuestCallResult::Value(ptr(Some(original))));

    let heap = result_pointer(&call(
        &mut setup,
        "kernel32.dll",
        "HeapCreate",
        vec![
            GuestCallValue::Uint32(0),
            GuestCallValue::Uint32(0),
            GuestCallValue::Uint32(0),
        ],
    ));
    let allocation = result_pointer(&call(
        &mut setup,
        "kernel32.dll",
        "HeapAlloc",
        vec![ptr(Some(heap)), GuestCallValue::Uint32(8), GuestCallValue::Uint32(16)],
    ));
    {
        let (_, memory) = setup.runner.cpu_parts();
        memory.write_u32(allocation, 0x1234_5678).unwrap();
    }
    let odd = result_pointer(&call(
        &mut setup,
        "kernel32.dll",
        "HeapAlloc",
        vec![ptr(Some(heap)), GuestCallValue::Uint32(8), GuestCallValue::Uint32(6)],
    ));
    assert_eq!(setup.runtime.allocation_size(odd, heap.offset), Some(6));
    let (_, memory) = setup.runner.cpu_parts();
    assert_eq!(memory.copy(odd, 16).unwrap(), vec![0; 16]);
    let freed = call(
        &mut setup,
        "kernel32.dll",
        "HeapFree",
        vec![ptr(Some(heap)), GuestCallValue::Uint32(0), ptr(Some(odd))],
    );
    assert_eq!(freed, GuestCallResult::Value(GuestCallValue::Int32(1)));
    let (_, memory) = setup.runner.cpu_parts();
    assert!(memory.copy(memory.offset(odd, 4095).unwrap(), 1).is_err());
    let enlarged = result_pointer(&call(
        &mut setup,
        "kernel32.dll",
        "HeapReAlloc",
        vec![
            ptr(Some(heap)),
            GuestCallValue::Uint32(8),
            ptr(Some(allocation)),
            GuestCallValue::Uint32(40),
        ],
    ));
    let (_, memory) = setup.runner.cpu_parts();
    assert_eq!(memory.read_u32(enlarged).unwrap(), 0x1234_5678);
    assert_eq!(memory.read_u32(memory.offset(enlarged, 20).unwrap()).unwrap(), 0);
    assert!(memory.read_u8(allocation).is_err());

    let index = call(&mut setup, "kernel32.dll", "TlsAlloc", vec![]);
    let GuestCallResult::Value(GuestCallValue::Uint32(index)) = index else {
        panic!("TLS index required");
    };
    let set = call(
        &mut setup,
        "kernel32.dll",
        "TlsSetValue",
        vec![GuestCallValue::Uint32(index), ptr(Some(enlarged))],
    );
    assert_eq!(set, GuestCallResult::Value(GuestCallValue::Int32(1)));
    let teb = setup.runtime.teb();
    let (_, memory) = setup.runner.cpu_parts();
    let slot = memory.offset(teb, 0xe10 + i64::from(index) * 4).unwrap();
    assert_eq!(
        memory.read_pointer(slot).unwrap().map(|address| address.offset),
        Some(enlarged.offset)
    );
    {
        let (_, memory) = setup.runner.cpu_parts();
        memory.write_pointer(slot, Some(heap)).unwrap();
        setup.runtime.set_last_error(memory, 99).unwrap();
    }
    let value = call(
        &mut setup,
        "kernel32.dll",
        "TlsGetValue",
        vec![GuestCallValue::Uint32(index)],
    );
    assert_eq!(value, GuestCallResult::Value(ptr(Some(heap))));
    let (_, memory) = setup.runner.cpu_parts();
    assert_eq!(setup.runtime.last_error(memory).unwrap(), 0);
    let freed = call(
        &mut setup,
        "kernel32.dll",
        "TlsFree",
        vec![GuestCallValue::Uint32(index)],
    );
    assert_eq!(freed, GuestCallResult::Value(GuestCallValue::Int32(1)));
    let value = call(
        &mut setup,
        "kernel32.dll",
        "TlsGetValue",
        vec![GuestCallValue::Uint32(index)],
    );
    assert_eq!(value, GuestCallResult::Value(ptr(None)));
    let (_, memory) = setup.runner.cpu_parts();
    assert_eq!(setup.runtime.last_error(memory).unwrap(), 87);

    let path = {
        let (_, memory) = setup.runner.cpu_parts();
        let path = allocate(memory, 16, 16);
        memory.write(path, b"save.sav\0").unwrap();
        path
    };
    let handle = result_pointer(&call(
        &mut setup,
        "kernel32.dll",
        "CreateFileA",
        vec![
            ptr(Some(path)),
            GuestCallValue::Uint32(0xc000_0000),
            GuestCallValue::Uint32(0),
            ptr(None),
            GuestCallValue::Uint32(3),
            GuestCallValue::Uint32(0),
            ptr(None),
        ],
    ));
    let (buffer, transferred) = {
        let (_, memory) = setup.runner.cpu_parts();
        (allocate(memory, 8, 16), allocate(memory, 4, 16))
    };
    let read = call(
        &mut setup,
        "kernel32.dll",
        "ReadFile",
        vec![
            ptr(Some(handle)),
            ptr(Some(buffer)),
            GuestCallValue::Uint32(2),
            ptr(Some(transferred)),
            ptr(None),
        ],
    );
    assert_eq!(read, GuestCallResult::Value(GuestCallValue::Int32(1)));
    let (_, memory) = setup.runner.cpu_parts();
    assert_eq!(memory.copy(buffer, 2).unwrap(), [10, 20]);
    assert_eq!(memory.read_u32(transferred).unwrap(), 2);
    let position = call(
        &mut setup,
        "kernel32.dll",
        "SetFilePointer",
        vec![
            ptr(Some(handle)),
            GuestCallValue::Int32(1),
            ptr(None),
            GuestCallValue::Uint32(0),
        ],
    );
    assert_eq!(position, GuestCallResult::Value(GuestCallValue::Uint32(1)));
    {
        let (_, memory) = setup.runner.cpu_parts();
        memory.write(buffer, &[99, 88]).unwrap();
    }
    call(
        &mut setup,
        "kernel32.dll",
        "WriteFile",
        vec![
            ptr(Some(handle)),
            ptr(Some(buffer)),
            GuestCallValue::Uint32(2),
            ptr(Some(transferred)),
            ptr(None),
        ],
    );
    assert_eq!(*bytes.borrow(), [10, 99, 88]);
    call(&mut setup, "kernel32.dll", "CloseHandle", vec![ptr(Some(handle))]);
    assert!(*closed.borrow());
    let destroyed = call(&mut setup, "kernel32.dll", "HeapDestroy", vec![ptr(Some(heap))]);
    assert_eq!(destroyed, GuestCallResult::Value(GuestCallValue::Int32(1)));
    let (_, memory) = setup.runner.cpu_parts();
    assert!(memory.read_u8(enlarged).is_err());
}

#[test]
fn crt_memory_and_floating_services_preserve_overlap_sign_and_adjacent_values() {
    let mut setup = setup(
        8,
        WindowsCapabilities {
            now_milliseconds: Some(Rc::new(|| 1_700_000_000_123)),
            ..Default::default()
        },
    );
    let buffer = {
        let (_, memory) = setup.runner.cpu_parts();
        let buffer = allocate(memory, 32, 16);
        memory.write(buffer, &[1, 2, 3, 4, 5, 6]).unwrap();
        buffer
    };
    let (destination, source) = {
        let (_, memory) = setup.runner.cpu_parts();
        (memory.offset(buffer, 1).unwrap(), buffer)
    };
    call(
        &mut setup,
        "ucrtbase.dll",
        "memmove",
        vec![ptr(Some(destination)), ptr(Some(source)), GuestCallValue::Uint64(5)],
    );
    let (_, memory) = setup.runner.cpu_parts();
    assert_eq!(memory.copy(buffer, 6).unwrap(), [1, 1, 2, 3, 4, 5]);
    let up = call(
        &mut setup,
        "ucrtbase.dll",
        "nextafterf",
        vec![GuestCallValue::Float32(1.0), GuestCallValue::Float32(2.0)],
    );
    assert_eq!(
        up,
        GuestCallResult::Value(GuestCallValue::Float32(1.0 + 2f32.powi(-23)))
    );
    let down = call(
        &mut setup,
        "ucrtbase.dll",
        "nextafterf",
        vec![GuestCallValue::Float32(0.0), GuestCallValue::Float32(-1.0)],
    );
    assert_eq!(
        down,
        GuestCallResult::Value(GuestCallValue::Float32(f32::from_bits(0x8000_0001)))
    );
    let fraction = call(
        &mut setup,
        "ucrtbase.dll",
        "modf",
        vec![GuestCallValue::Float64(-2.0), ptr(Some(buffer))],
    );
    match fraction {
        GuestCallResult::Value(GuestCallValue::Float64(value)) => assert!(value == 0.0 && value.is_sign_negative()),
        other => panic!("expected negative zero, got {other:?}"),
    }
    let (_, memory) = setup.runner.cpu_parts();
    assert_eq!(memory.read_f64(buffer).unwrap(), -2.0);
    let sign = call(
        &mut setup,
        "ucrtbase.dll",
        "_dsign",
        vec![GuestCallValue::Float64(-0.0)],
    );
    assert_eq!(sign, GuestCallResult::Value(GuestCallValue::Int32(-32768)));
    let ticks = setup
        .runtime
        .resolve_address("msvcp140.dll", "_Xtime_get_ticks")
        .expect("MSVC clock binding missing");
    let signature = {
        let (_, memory) = setup.runner.cpu_parts();
        setup
            .hooks
            .callbacks
            .borrow_mut()
            .handle(memory, ticks)
            .unwrap()
            .unwrap()
            .signature
    };
    let request = GuestCallRequest {
        target: ticks,
        signature,
        arguments: vec![],
        context: setup.context.clone(),
        instruction_budget: 1000,
    };
    let clock = setup.runner.invoke(&request).unwrap();
    assert_eq!(
        clock,
        GuestCallResult::Value(GuestCallValue::Int64(17_000_000_001_230_000))
    );

    let (text, end) = {
        let (_, memory) = setup.runner.cpu_parts();
        (allocate(memory, 32, 16), allocate(memory, 8, 16))
    };
    {
        let (_, memory) = setup.runner.cpu_parts();
        memory.write(text, b"-0x10!\0").unwrap();
    }
    let parsed = call(
        &mut setup,
        "ucrtbase.dll",
        "strtoul",
        vec![ptr(Some(text)), ptr(Some(end)), GuestCallValue::Int32(0)],
    );
    assert_eq!(parsed, GuestCallResult::Value(GuestCallValue::Uint32(0xffff_fff0)));
    let (_, memory) = setup.runner.cpu_parts();
    assert_eq!(
        memory.read_pointer(end).unwrap().map(|address| address.offset),
        Some(text.offset + 5)
    );
    {
        let (_, memory) = setup.runner.cpu_parts();
        memory.write(text, b"4294967296x\0").unwrap();
    }
    let parsed = call(
        &mut setup,
        "ucrtbase.dll",
        "strtoul",
        vec![ptr(Some(text)), ptr(Some(end)), GuestCallValue::Int32(10)],
    );
    assert_eq!(parsed, GuestCallResult::Value(GuestCallValue::Uint32(0xffff_ffff)));
    let errno = call(&mut setup, "ucrtbase.dll", "_errno", vec![]);
    let (_, memory) = setup.runner.cpu_parts();
    assert_eq!(memory.read_i32(result_pointer(&errno)).unwrap(), 34);

    let comparisons = Rc::new(RefCell::new(0u32));
    let probe = Rc::clone(&comparisons);
    let signature = windows_signature(
        8,
        "ucrtbase.dll",
        &[GuestStorage::Pointer, GuestStorage::Pointer],
        Some(GuestStorage::Int32),
    );
    let comparator = {
        let (_, memory) = setup.runner.cpu_parts();
        setup
            .hooks
            .callbacks
            .borrow_mut()
            .bind(
                memory,
                GuestHostCallback {
                    id: CallbackId::new("test", "sort-compare"),
                    signature,
                    invoke: Rc::new(move |ctx, _, args| {
                        let (Some(left), Some(right)) = (pointer_value(args, 0), pointer_value(args, 1)) else {
                            return Err(qa_guest::error::GuestError::invalid("Comparator pointers missing"));
                        };
                        *probe.borrow_mut() += 1;
                        let memory = ctx.memory();
                        Ok(GuestCallResult::Value(GuestCallValue::Int32(
                            memory.read_i32(left)? - memory.read_i32(right)?,
                        )))
                    }),
                },
            )
            .unwrap()
    };
    {
        let (_, memory) = setup.runner.cpu_parts();
        for (index, value) in [9, -3, 9, 0, 2].iter().enumerate() {
            memory
                .write_i32(memory.offset(buffer, (index * 4) as i64).unwrap(), *value)
                .unwrap();
        }
    }
    call(
        &mut setup,
        "ucrtbase.dll",
        "qsort",
        vec![
            ptr(Some(buffer)),
            GuestCallValue::Uint64(5),
            GuestCallValue::Uint64(4),
            ptr(Some(comparator)),
        ],
    );
    let (_, memory) = setup.runner.cpu_parts();
    let sorted: Vec<i32> = (0..5)
        .map(|index| memory.read_i32(memory.offset(buffer, index * 4).unwrap()).unwrap())
        .collect();
    assert_eq!(sorted, [-3, 0, 2, 9, 9]);
    assert!(comparisons.take() > 0);
}

#[test]
fn unimplemented_imports_retain_explicit_reached_failure() {
    let mut setup = setup(4, WindowsCapabilities::default());
    let image = {
        let (_, memory) = setup.runner.cpu_parts();
        map_pe_image(MapPeImageOptions {
            bytes: &pe_fixture(4),
            memory,
            module: None,
            base: None,
            maximum_image_bytes: None,
        })
        .unwrap()
    };
    let import = GuestImport {
        library: "unavailable.dll".to_string(),
        symbol: GuestSymbolName::Name {
            name: "Missing".to_string(),
            version: None,
        },
        slot: image.image.base,
        weak: false,
    };
    let resolution = {
        let Setup { runner, runtime, .. } = &mut setup;
        let (_, memory) = runner.cpu_parts();
        <WindowsGuestRuntime as qa_guest::core::contracts::GuestImportResolver>::resolve(
            runtime,
            memory,
            &import,
            &image.image,
        )
    };
    let GuestImportResolution::Host { address, .. } = resolution else {
        panic!("expected explicit unsupported host trap: {resolution:?}");
    };
    let failure = call_failure(&mut setup, address, vec![]);
    assert!(
        format!("{failure:?}").contains("Unsupported Windows guest import"),
        "{failure:?}"
    );
    let entry = setup
        .runtime
        .coverage()
        .into_iter()
        .find(|entry| entry.name == "Missing")
        .expect("coverage entry");
    assert!(!entry.supported);
    assert_eq!(entry.reached, 1);
    assert_eq!(entry.failed, 1);
}

#[test]
fn dynamic_libraries_spin_locks_and_fiber_slots_retain_service_state() {
    let mut setup = setup(8, WindowsCapabilities::default());
    let wide = {
        let (_, memory) = setup.runner.cpu_parts();
        let wide = allocate(memory, 64, 16);
        for (index, letter) in "kernel32.dll".chars().enumerate() {
            memory
                .write_u16(memory.offset(wide, (index * 2) as i64).unwrap(), letter as u16)
                .unwrap();
        }
        wide
    };
    let library = result_pointer(&call(
        &mut setup,
        "kernel32.dll",
        "LoadLibraryExW",
        vec![ptr(Some(wide)), ptr(None), GuestCallValue::Uint32(0x800)],
    ));
    let handle = call(&mut setup, "kernel32.dll", "GetModuleHandleW", vec![ptr(Some(wide))]);
    assert_eq!(handle, GuestCallResult::Value(ptr(Some(library))));
    let name = {
        let (_, memory) = setup.runner.cpu_parts();
        allocate(memory, 64, 16)
    };
    {
        let (_, memory) = setup.runner.cpu_parts();
        memory.write(name, b"GetLastError\0").unwrap();
    }
    let expected = setup.runtime.resolve_address("kernel32.dll", "GetLastError");
    let proc = call(
        &mut setup,
        "kernel32.dll",
        "GetProcAddress",
        vec![ptr(Some(library)), ptr(Some(name))],
    );
    assert_eq!(proc, GuestCallResult::Value(ptr(expected)));
    {
        let (_, memory) = setup.runner.cpu_parts();
        memory.write(name, b"UnsupportedDynamic\0").unwrap();
    }
    let proc = call(
        &mut setup,
        "kernel32.dll",
        "GetProcAddress",
        vec![ptr(Some(library)), ptr(Some(name))],
    );
    assert_eq!(proc, GuestCallResult::Value(ptr(None)));
    let (_, memory) = setup.runner.cpu_parts();
    assert_eq!(setup.runtime.last_error(memory).unwrap(), 127);
    let freed = call(&mut setup, "kernel32.dll", "FreeLibrary", vec![ptr(Some(library))]);
    assert_eq!(freed, GuestCallResult::Value(GuestCallValue::Int32(1)));
    assert_eq!(setup.runtime.library_handle("kernel32.dll"), Some(library));
    let freed = call(&mut setup, "kernel32.dll", "FreeLibrary", vec![ptr(Some(library))]);
    assert_eq!(freed, GuestCallResult::Value(GuestCallValue::Int32(0)));

    let lock = {
        let (_, memory) = setup.runner.cpu_parts();
        allocate(memory, 40, 16)
    };
    let initialized = call(
        &mut setup,
        "kernel32.dll",
        "InitializeCriticalSectionAndSpinCount",
        vec![ptr(Some(lock)), GuestCallValue::Uint32(4000)],
    );
    assert_eq!(initialized, GuestCallResult::Value(GuestCallValue::Int32(1)));
    let (_, memory) = setup.runner.cpu_parts();
    assert_eq!(memory.read_u64(memory.offset(lock, 32).unwrap()).unwrap(), 4000);
    call(
        &mut setup,
        "kernel32.dll",
        "EnterCriticalSection",
        vec![ptr(Some(lock))],
    );
    call(
        &mut setup,
        "kernel32.dll",
        "EnterCriticalSection",
        vec![ptr(Some(lock))],
    );
    let (_, memory) = setup.runner.cpu_parts();
    assert_eq!(memory.read_u32(memory.offset(lock, 12).unwrap()).unwrap(), 2);
    call(
        &mut setup,
        "kernel32.dll",
        "LeaveCriticalSection",
        vec![ptr(Some(lock))],
    );
    call(
        &mut setup,
        "kernel32.dll",
        "LeaveCriticalSection",
        vec![ptr(Some(lock))],
    );
    let (_, memory) = setup.runner.cpu_parts();
    assert_eq!(memory.read_u32(memory.offset(lock, 8).unwrap()).unwrap(), 0xffff_ffff);

    let (list, entry) = {
        let (_, memory) = setup.runner.cpu_parts();
        (allocate(memory, 16, 16), allocate(memory, 16, 16))
    };
    call(&mut setup, "kernel32.dll", "InitializeSListHead", vec![ptr(Some(list))]);
    let flushed = call(
        &mut setup,
        "kernel32.dll",
        "InterlockedFlushSList",
        vec![ptr(Some(list))],
    );
    assert_eq!(flushed, GuestCallResult::Value(ptr(None)));
    {
        let (_, memory) = setup.runner.cpu_parts();
        memory.write_u64(list, 0x70001).unwrap();
        memory.write_u64(memory.offset(list, 8).unwrap(), entry.offset).unwrap();
    }
    let flushed = call(
        &mut setup,
        "kernel32.dll",
        "InterlockedFlushSList",
        vec![ptr(Some(list))],
    );
    assert_eq!(flushed, GuestCallResult::Value(ptr(Some(entry))));
    let (_, memory) = setup.runner.cpu_parts();
    assert_eq!(memory.read_u16(list).unwrap(), 0);
    assert_eq!(memory.read_u64(memory.offset(list, 8).unwrap()).unwrap(), 0);

    let released: Rc<RefCell<Vec<GuestCallValue>>> = Rc::new(RefCell::new(Vec::new()));
    let probe = Rc::clone(&released);
    let signature = windows_signature(8, "kernel32.dll", &[GuestStorage::Pointer], None);
    let cleanup = {
        let (_, memory) = setup.runner.cpu_parts();
        setup
            .hooks
            .callbacks
            .borrow_mut()
            .bind(
                memory,
                GuestHostCallback {
                    id: CallbackId::new("test", "fls-cleanup"),
                    signature,
                    invoke: Rc::new(move |_, _, args| {
                        probe.borrow_mut().extend(args.iter().cloned());
                        Ok(GuestCallResult::Void)
                    }),
                },
            )
            .unwrap()
    };
    let index = call(&mut setup, "kernel32.dll", "FlsAlloc", vec![ptr(Some(cleanup))]);
    let GuestCallResult::Value(GuestCallValue::Uint32(index)) = index else {
        panic!("Missing FLS index");
    };
    let value = call(
        &mut setup,
        "kernel32.dll",
        "FlsGetValue",
        vec![GuestCallValue::Uint32(index)],
    );
    assert_eq!(value, GuestCallResult::Value(ptr(None)));
    call(
        &mut setup,
        "kernel32.dll",
        "FlsSetValue",
        vec![GuestCallValue::Uint32(index), ptr(Some(lock))],
    );
    let value = call(
        &mut setup,
        "kernel32.dll",
        "FlsGetValue",
        vec![GuestCallValue::Uint32(index)],
    );
    assert_eq!(value, GuestCallResult::Value(ptr(Some(lock))));
    let freed = call(
        &mut setup,
        "kernel32.dll",
        "FlsFree",
        vec![GuestCallValue::Uint32(index)],
    );
    assert_eq!(freed, GuestCallResult::Value(GuestCallValue::Int32(1)));
    let value = call(
        &mut setup,
        "kernel32.dll",
        "FlsGetValue",
        vec![GuestCallValue::Uint32(index)],
    );
    assert_eq!(value, GuestCallResult::Value(ptr(None)));
    let (_, memory) = setup.runner.cpu_parts();
    assert_eq!(setup.runtime.last_error(memory).unwrap(), 87);
    assert_eq!(*released.borrow(), [ptr(Some(lock))]);
}

#[test]
fn dynamic_lookup_resolves_prepared_pe_exports_without_cfg() {
    let mut setup = setup(8, WindowsCapabilities::default());
    let image = {
        let (_, memory) = setup.runner.cpu_parts();
        map_pe_image(MapPeImageOptions {
            bytes: &pe_fixture(8),
            memory,
            module: None,
            base: None,
            maximum_image_bytes: None,
        })
        .unwrap()
    };
    let config = image
        .load_configuration
        .clone()
        .expect("fixture lacks load configuration");
    let (check, dispatch) = (
        config.guard_check_slot.expect("guard check"),
        config.guard_dispatch_slot.expect("guard dispatch"),
    );
    let original = {
        let (_, memory) = setup.runner.cpu_parts();
        let original = memory.offset(image.image.base, 0x1010).unwrap();
        memory.write_pointer(check, Some(original)).unwrap();
        memory.write_pointer(dispatch, Some(original)).unwrap();
        original
    };
    assert_eq!(config.guard_flags, 0x100);
    assert_eq!(image.pe.dll_characteristics & 0x4000, 0);
    {
        let Setup { runner, runtime, .. } = &mut setup;
        let (_, memory) = runner.cpu_parts();
        runtime.prepare_image(memory, &image).unwrap();
    }
    let (_, memory) = setup.runner.cpu_parts();
    assert_eq!(memory.read_pointer(check).unwrap(), Some(original));
    assert_eq!(memory.read_pointer(dispatch).unwrap(), Some(original));
    let artifact = setup.module.artifact_path.clone();
    let (_, memory) = setup.runner.cpu_parts();
    assert_eq!(
        setup.runtime.load_library(memory, &artifact).unwrap(),
        Some(image.image.base)
    );
    assert_eq!(setup.runtime.resolve_address(&artifact, "GetGameAPI"), Some(original));
    assert_eq!(setup.runtime.resolve_address(&artifact, "missing"), None);
    assert_eq!(setup.runtime.resolve_address(&artifact, "Forward"), None);
    let (_, memory) = setup.runner.cpu_parts();
    assert!(setup.runtime.free_library(memory, image.image.base).unwrap());
    assert_eq!(setup.runtime.library_handle(&artifact), Some(image.image.base));
}

fn pointer_value(args: &[GuestCallValue], index: usize) -> Option<GuestAddress> {
    match args.get(index) {
        Some(GuestCallValue::Pointer(value)) => *value,
        _ => None,
    }
}

#[allow(dead_code)]
fn synthetic_image(module: ModuleIdentity, base: GuestAddress) -> GuestImage {
    GuestImage {
        module,
        abi: NativeAbi::WindowsX86_64,
        base,
        preferred_base: base.offset,
        byte_length: 0,
        entry_point: None,
        mappings: vec![],
        imports: vec![],
        exports: vec![],
        tls: None,
        initializers: vec![],
        finalizers: vec![],
        unwind: vec![],
    }
}
