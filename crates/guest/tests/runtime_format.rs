//! Runtime `printf`/`scanf` service tests over both guest runtimes.
//!
//! Donor: `tests/guest/runtime/format/format.test.ts` and
//! `tests/guest/runtime/format/scan.test.ts`. The native-scan suite needs
//! user-supplied retail binaries outside the source boundary.

mod common;

use std::rc::Rc;

use qa_core::identity::ProviderId;
use qa_guest::abi::runner::{GuestCallRequest, GuestCallRunner};
use qa_guest::abi::GuestCpu;
use qa_guest::core::callbacks::HookState;
use qa_guest::core::contracts::{
    CallbackId, GuestAddress, GuestAllocationOptions, GuestArchitecture, GuestCallContext, GuestCallResult,
    GuestCallValue, GuestCallbackReference, GuestPermissions,
};
use qa_guest::core::memory::SparseGuestMemory;
use qa_guest::core::registers::{GuestProcessorInitialState, GuestProcessorState};
use qa_guest::floating_point::binary::{decode_binary, BigInt, BinaryWidth, Rounding};
use qa_guest::runtime::common::format::arguments::FormatDialect;
use qa_guest::runtime::common::format::float::{format_float, FormatRounding};
use qa_guest::runtime::common::format::{format_guest_buffer, FormatTermination, GuestFormatRequest};
use qa_guest::runtime::common::memory::{read_string, string_bytes};
use qa_guest::runtime::system_v::contracts::SystemVRuntimeOptions;
use qa_guest::runtime::system_v::runtime::SystemVGuestRuntime;
use qa_guest::runtime::windows::contracts::WindowsRuntimeOptions;
use qa_guest::runtime::windows::runtime::WindowsGuestRuntime;
use qa_guest::x64::cpu::X64Cpu;
use qa_guest::x86::cpu::I386Cpu;

use common::test_module;

struct Fixture {
    runner: GuestCallRunner<'static>,
    hooks: Rc<HookState>,
    context: GuestCallContext,
    width: usize,
}

fn allocate(
    memory: &mut SparseGuestMemory,
    byte_length: usize,
    permissions: GuestPermissions,
    alignment: u64,
) -> GuestAddress {
    memory
        .allocate(&GuestAllocationOptions {
            byte_length,
            alignment,
            permissions,
            label: "format-fixture".to_string(),
        })
        .unwrap()
}

fn fixture(width: usize) -> Fixture {
    let wide = width == 8;
    let module = test_module("guest-format");
    let mut memory = SparseGuestMemory::new(module.clone(), width, 0x10000).unwrap();
    let stack = allocate(&mut memory, 65536, GuestPermissions::ReadWrite, 16);
    let sentinel = allocate(&mut memory, 16, GuestPermissions::ReadExecute, 16);
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
    let hooks = Rc::new(HookState::new());
    let boxed: Box<dyn GuestCpu> = if wide {
        Box::new(X64Cpu::new(state, memory).unwrap())
    } else {
        Box::new(I386Cpu::new(state, memory).unwrap())
    };
    let leaked: &'static mut dyn GuestCpu = Box::leak(boxed);
    let runner = GuestCallRunner::new(leaked, Rc::clone(&hooks), sentinel, None).unwrap();
    let context = GuestCallContext {
        module,
        callback: GuestCallbackReference::TypeScript {
            provider: ProviderId::new("test", "format"),
            callback: CallbackId::new("test", "entry"),
        },
        parent: None,
        itself: None,
        other: None,
    };
    Fixture {
        runner,
        hooks,
        context,
        width,
    }
}

fn text(fixture: &mut Fixture, value: &str) -> GuestAddress {
    let bytes = string_bytes(value, false);
    let address = {
        let (_, memory) = fixture.runner.cpu_parts();
        allocate(memory, bytes.len(), GuestPermissions::ReadWrite, 16)
    };
    let (_, memory) = fixture.runner.cpu_parts();
    memory.write(address, &bytes).unwrap();
    address
}

fn service_address(fixture: &Fixture, namespace: &str, key: &str) -> GuestAddress {
    fixture
        .hooks
        .callbacks
        .borrow()
        .address(&CallbackId::new(namespace, key))
        .unwrap_or_else(|| panic!("missing service {namespace}:{key}"))
}

fn invoke(fixture: &mut Fixture, address: GuestAddress, args: Vec<GuestCallValue>) -> GuestCallResult {
    let signature = {
        let (_, memory) = fixture.runner.cpu_parts();
        fixture
            .hooks
            .callbacks
            .borrow_mut()
            .handle(memory, address)
            .unwrap()
            .unwrap_or_else(|| panic!("missing callback at 0x{:x}", address.offset))
            .signature
    };
    let request = GuestCallRequest {
        target: address,
        signature,
        arguments: args,
        context: fixture.context.clone(),
        instruction_budget: 1000,
    };
    fixture
        .runner
        .invoke(&request)
        .unwrap_or_else(|failure| panic!("service failed: {failure:?}"))
}

fn invoke_result(fixture: &mut Fixture, address: GuestAddress, args: Vec<GuestCallValue>) -> i32 {
    match invoke(fixture, address, args) {
        GuestCallResult::Value(GuestCallValue::Int32(value)) => value,
        other => panic!("expected int32, got {other:?}"),
    }
}

fn ptr(value: Option<GuestAddress>) -> GuestCallValue {
    GuestCallValue::Pointer(value)
}

fn uint(width: usize, value: u64) -> GuestCallValue {
    if width == 4 {
        #[allow(clippy::cast_possible_truncation)]
        GuestCallValue::Uint32(value as u32)
    } else {
        GuestCallValue::Uint64(value)
    }
}

fn offset_of(fixture: &mut Fixture, address: GuestAddress, displacement: i64) -> GuestAddress {
    let (_, memory) = fixture.runner.cpu_parts();
    memory.offset(address, displacement).unwrap()
}

#[test]
fn ucrt_numeric_save_arguments_and_crt_buffer_options() {
    for width in [4usize, 8usize] {
        let mut fixture = fixture(width);
        let hooks = Rc::clone(&fixture.hooks);
        let runtime = {
            let (_, memory) = fixture.runner.cpu_parts();
            WindowsGuestRuntime::new(
                hooks,
                memory,
                WindowsRuntimeOptions {
                    capabilities: Default::default(),
                    process_id: None,
                    thread_id: None,
                },
            )
            .unwrap()
        };
        runtime.attach_runner(&mut fixture.runner).unwrap();
        let service = service_address(
            &fixture,
            "windows",
            "api-ms-win-crt-stdio-l1-1-0.dll!__stdio_common_vsprintf",
        );
        let (buffer, args) = {
            let (_, memory) = fixture.runner.cpu_parts();
            (
                allocate(memory, 64, GuestPermissions::ReadWrite, 16),
                allocate(memory, 32, GuestPermissions::ReadWrite, 16),
            )
        };
        {
            let (_, memory) = fixture.runner.cpu_parts();
            memory.write_i32(args, 17).unwrap();
            let slot = memory.offset(args, width as i64).unwrap();
            memory.write_f64(slot, 16.0).unwrap();
        }
        let format = text(&mut fixture, "%.*g");
        let result = invoke_result(
            &mut fixture,
            service,
            vec![
                GuestCallValue::Uint64(0x26),
                ptr(Some(buffer)),
                uint(width, 36),
                ptr(Some(format)),
                ptr(None),
                ptr(Some(args)),
            ],
        );
        assert_eq!(result, 2);
        let (_, memory) = fixture.runner.cpu_parts();
        assert_eq!(read_string(memory, buffer, false).unwrap(), "16");

        let literal = text(&mut fixture, "abcde");
        {
            let (_, memory) = fixture.runner.cpu_parts();
            memory.write(buffer, &[0x7f; 64]).unwrap();
        }
        let call = |fixture: &mut Fixture,
                    options: u64,
                    capacity: u64,
                    format: GuestAddress,
                    list: Option<GuestAddress>,
                    destination: Option<GuestAddress>| {
            invoke_result(
                fixture,
                service,
                vec![
                    GuestCallValue::Uint64(options),
                    ptr(destination),
                    uint(width, capacity),
                    ptr(Some(format)),
                    ptr(None),
                    ptr(list),
                ],
            )
        };
        assert_eq!(call(&mut fixture, 2, 4, literal, None, Some(buffer)), 5);
        let (_, memory) = fixture.runner.cpu_parts();
        assert_eq!(memory.copy(buffer, 6).unwrap(), [97, 98, 99, 0, 127, 127]);
        assert_eq!(call(&mut fixture, 2, 0, literal, None, None), 5);
        {
            let (_, memory) = fixture.runner.cpu_parts();
            memory.write(buffer, &[0x7f; 64]).unwrap();
        }
        assert_eq!(call(&mut fixture, 1, 5, literal, None, Some(buffer)), 5);
        let (_, memory) = fixture.runner.cpu_parts();
        assert_eq!(memory.copy(buffer, 6).unwrap(), [97, 98, 99, 100, 101, 127]);
        assert_eq!(call(&mut fixture, 1, 4, literal, None, Some(buffer)), -1);
        assert_eq!(call(&mut fixture, 3, 4, literal, None, Some(buffer)), -1);
        assert_eq!(call(&mut fixture, 0, 5, literal, None, Some(buffer)), -2);
        let (_, memory) = fixture.runner.cpu_parts();
        assert_eq!(read_string(memory, buffer, false).unwrap(), "abcd");
        let empty = text(&mut fixture, "");
        assert_eq!(call(&mut fixture, 0, 0, empty, None, Some(buffer)), -1);
        {
            let (_, memory) = fixture.runner.cpu_parts();
            memory.write_pointer(args, Some(buffer)).unwrap();
        }
        let percent_n = text(&mut fixture, "abc%n");
        let result = invoke_result(
            &mut fixture,
            service,
            vec![
                GuestCallValue::Uint64(2),
                ptr(Some(buffer)),
                uint(width, 64),
                ptr(Some(percent_n)),
                ptr(None),
                ptr(Some(args)),
            ],
        );
        assert_eq!(result, -1);
        let errno_service = service_address(&fixture, "windows", "ucrtbase.dll!_errno");
        let errno_value = invoke(&mut fixture, errno_service, vec![]);
        let GuestCallResult::Value(GuestCallValue::Pointer(Some(errno_address))) = errno_value else {
            panic!("missing CRT errno: {errno_value:?}");
        };
        let (_, memory) = fixture.runner.cpu_parts();
        assert_eq!(memory.read_i32(errno_address).unwrap(), 22);
    }
}

const FORTIFY_MARKER: &str = "smaller than maxlen";
const FORTIFY_WRITABLE_MARKER: &str = "%n in writable format";

#[test]
fn glibc_varargs_stack_overflow_fortify_and_positional_conversions() {
    for width in [4usize, 8usize] {
        let mut fixture = fixture(width);
        let hooks = Rc::clone(&fixture.hooks);
        let runtime = {
            let (_, memory) = fixture.runner.cpu_parts();
            SystemVGuestRuntime::new(
                hooks,
                memory,
                SystemVRuntimeOptions {
                    capabilities: Default::default(),
                    argv: vec![],
                    environment: vec![],
                },
            )
            .unwrap()
        };
        runtime.attach_runner(&mut fixture.runner).unwrap();
        let service = service_address(&fixture, "system-v", "libc.so.6:__vsnprintf_chk@GLIBC_2.3.4");
        let (buffer, registers, stack, descriptor) = {
            let (_, memory) = fixture.runner.cpu_parts();
            (
                allocate(memory, 256, GuestPermissions::ReadWrite, 16),
                allocate(memory, 176, GuestPermissions::ReadWrite, 16),
                allocate(memory, 64, GuestPermissions::ReadWrite, 16),
                allocate(memory, 24, GuestPermissions::ReadWrite, 16),
            )
        };
        let label = text(&mut fixture, "guest");
        let count = {
            let (_, memory) = fixture.runner.cpu_parts();
            allocate(memory, 8, GuestPermissions::ReadWrite, 16)
        };
        let arguments = if width == 8 { descriptor } else { stack };
        let reset = |fixture: &mut Fixture| {
            if width == 8 {
                let (_, memory) = fixture.runner.cpu_parts();
                memory.write_u32(descriptor, 40).unwrap();
                let gp = memory.offset(descriptor, 4).unwrap();
                memory.write_u32(gp, 160).unwrap();
                let overflow = memory.offset(descriptor, 8).unwrap();
                memory.write_pointer(overflow, Some(stack)).unwrap();
                let save = memory.offset(descriptor, 16).unwrap();
                memory.write_pointer(save, Some(registers)).unwrap();
                let slot = memory.offset(registers, 40).unwrap();
                memory.write_i64(slot, -42).unwrap();
                let slot = memory.offset(registers, 160).unwrap();
                memory.write_f64(slot, 2.5).unwrap();
                memory.write_pointer(stack, Some(label)).unwrap();
                let slot = memory.offset(stack, 8).unwrap();
                memory.write_u64(slot, u64::MAX).unwrap();
                let slot = memory.offset(stack, 16).unwrap();
                memory.write_f64(slot, 1.25).unwrap();
                let slot = memory.offset(stack, 24).unwrap();
                memory.write_pointer(slot, Some(count)).unwrap();
            } else {
                let (_, memory) = fixture.runner.cpu_parts();
                memory.write_i32(stack, -42).unwrap();
                let slot = memory.offset(stack, 4).unwrap();
                memory.write_f64(slot, 2.5).unwrap();
                let slot = memory.offset(stack, 12).unwrap();
                memory.write_pointer(slot, Some(label)).unwrap();
                let slot = memory.offset(stack, 16).unwrap();
                memory.write_u64(slot, u64::MAX).unwrap();
                let slot = memory.offset(stack, 24).unwrap();
                memory.write_f64(slot, 1.25).unwrap();
                let slot = memory.offset(stack, 32).unwrap();
                memory.write_pointer(slot, Some(count)).unwrap();
            }
        };
        let literal = text(&mut fixture, "------- Game Initialization -------\n");
        let call = |fixture: &mut Fixture, format: GuestAddress, capacity: u64, size: u64, flag: i32| {
            invoke_result(
                fixture,
                service,
                vec![
                    ptr(Some(buffer)),
                    uint(width, capacity),
                    GuestCallValue::Int32(flag),
                    uint(width, size),
                    ptr(Some(format)),
                    ptr(Some(arguments)),
                ],
            )
        };
        assert_eq!(call(&mut fixture, literal, 256, 256, 0), 36);
        let (_, memory) = fixture.runner.cpu_parts();
        assert_eq!(
            read_string(memory, buffer, false).unwrap(),
            "------- Game Initialization -------\n"
        );
        reset(&mut fixture);
        let format = text(&mut fixture, "%+06d|%.0f|%.3s|%#llx|%.2f%n");
        let expected = "-00042|2|gue|0xffffffffffffffff|1.25";
        assert_eq!(call(&mut fixture, format, 256, 256, 0), expected.len() as i32);
        let (_, memory) = fixture.runner.cpu_parts();
        assert_eq!(read_string(memory, buffer, false).unwrap(), expected);
        assert_eq!(memory.read_i32(count).unwrap(), expected.len() as i32);
        if width == 8 {
            let (_, memory) = fixture.runner.cpu_parts();
            assert_eq!(memory.read_u32(descriptor).unwrap(), 48);
            let slot = memory.offset(descriptor, 4).unwrap();
            assert_eq!(memory.read_u32(slot).unwrap(), 176);
            let slot = memory.offset(descriptor, 8).unwrap();
            let expected_stack = memory.offset(stack, 32).unwrap();
            assert_eq!(memory.read_pointer(slot).unwrap(), Some(expected_stack));
        }
        reset(&mut fixture);
        assert_eq!(call(&mut fixture, format, 5, 256, 0), expected.len() as i32);
        let (_, memory) = fixture.runner.cpu_parts();
        assert_eq!(read_string(memory, buffer, false).unwrap(), "-000");
        reset(&mut fixture);
        let signature = {
            let (_, memory) = fixture.runner.cpu_parts();
            fixture
                .hooks
                .callbacks
                .borrow_mut()
                .handle(memory, service)
                .unwrap()
                .unwrap()
                .signature
        };
        let failing = GuestCallRequest {
            target: service,
            signature: signature.clone(),
            arguments: vec![
                ptr(Some(buffer)),
                uint(width, 257),
                GuestCallValue::Int32(0),
                uint(width, 256),
                ptr(Some(format)),
                ptr(Some(arguments)),
            ],
            context: fixture.context.clone(),
            instruction_budget: 1000,
        };
        assert!(format!("{:?}", fixture.runner.invoke(&failing)).contains(FORTIFY_MARKER));
        let failing = GuestCallRequest {
            target: service,
            signature,
            arguments: vec![
                ptr(Some(buffer)),
                uint(width, 256),
                GuestCallValue::Int32(1),
                uint(width, 256),
                ptr(Some(format)),
                ptr(Some(arguments)),
            ],
            context: fixture.context.clone(),
            instruction_budget: 1000,
        };
        assert!(format!("{:?}", fixture.runner.invoke(&failing)).contains(FORTIFY_WRITABLE_MARKER));
        reset(&mut fixture);
        {
            let (_, memory) = fixture.runner.cpu_parts();
            let format_bytes = string_bytes("%+06d|%.0f|%.3s|%#llx|%.2f%n", false);
            memory
                .protect(format, format_bytes.len(), GuestPermissions::Read)
                .unwrap();
        }
        assert_eq!(call(&mut fixture, format, 256, 256, 1), expected.len() as i32);
        reset(&mut fixture);
        let positional = text(&mut fixture, "%2$.1f/%1$d/%2$.0f");
        assert_eq!(call(&mut fixture, positional, 256, 256, 0), 9);
        let (_, memory) = fixture.runner.cpu_parts();
        assert_eq!(read_string(memory, buffer, false).unwrap(), "2.5/-42/2");
    }
}

#[test]
fn float_conversion_rounds_without_host_semantics() {
    let number = |value: f64| decode_binary(&BigInt::from_u64(value.to_bits()), BinaryWidth::W64);
    let format = |value: f64, code: char, precision: Option<usize>, alternate: bool| {
        format_float(
            &number(value),
            code,
            precision,
            alternate,
            FormatRounding::Ieee(Rounding::Nearest),
            false,
            false,
            2,
        )
    };
    assert_eq!(format(2.5, 'f', Some(0), false), "2");
    assert_eq!(format(3.5, 'f', Some(0), false), "4");
    assert_eq!(format(2.25, 'f', Some(1), false), "2.2");
    assert_eq!(
        format_float(
            &number(2.25),
            'f',
            Some(1),
            false,
            FormatRounding::LegacyNearest,
            true,
            false,
            2
        ),
        "2.3"
    );
    assert_eq!(format(1e21, 'f', Some(1), false), "1000000000000000000000.0");
    assert_eq!(format(0.1, 'f', Some(20), false), "0.10000000000000000555");
    assert_eq!(format(999.5, 'g', Some(3), false), "1e+03");
    assert_eq!(format(0.00009999, 'g', Some(3), false), "0.0001");
    assert_eq!(format(16.0, 'g', Some(17), false), "16");
    assert_eq!(format(16.0, 'g', Some(4), true), "16.00");
    assert_eq!(format(1.5, 'a', None, false), "0x1.8p+0");
    assert_eq!(format(1.5, 'a', Some(0), false), "0x2p+0");
    assert_eq!(format(f64::from_bits(1), 'a', None, false), "0x0.0000000000001p-1022");
    assert_eq!(
        format_float(
            &number(-2.25),
            'f',
            Some(1),
            false,
            FormatRounding::Ieee(Rounding::Down),
            false,
            false,
            2
        ),
        "2.3"
    );
    let extended = decode_binary(&BigInt::from_u128(0x3fff_8000_0000_0000_0001), BinaryWidth::W80);
    assert_eq!(
        format_float(
            &extended,
            'g',
            Some(21),
            false,
            FormatRounding::Ieee(Rounding::Nearest),
            false,
            true,
            2
        ),
        "1.00000000000000000011"
    );
    assert_eq!(
        format_float(
            &extended,
            'a',
            None,
            false,
            FormatRounding::Ieee(Rounding::Nearest),
            false,
            true,
            2
        ),
        "0x8.000000000000001p-3"
    );
}

#[test]
fn integer_lengths_dynamic_width_pointer_dialect_and_byte_strings() {
    let mut fixture = fixture(8);
    let (buffer, args) = {
        let (_, memory) = fixture.runner.cpu_parts();
        (
            allocate(memory, 256, GuestPermissions::ReadWrite, 16),
            allocate(memory, 64, GuestPermissions::ReadWrite, 16),
        )
    };
    let latin = text(&mut fixture, "ÿabc");
    {
        let (_, memory) = fixture.runner.cpu_parts();
        memory.write_i32(args, -8).unwrap();
        memory.write_i32(memory.offset(args, 8).unwrap(), 3).unwrap();
        memory.write_i32(memory.offset(args, 16).unwrap(), -7).unwrap();
        memory
            .write_u64(memory.offset(args, 24).unwrap(), 0x1234_5678_ffff_ffff)
            .unwrap();
        memory.write_u64(memory.offset(args, 32).unwrap(), 255).unwrap();
        memory
            .write_pointer(memory.offset(args, 40).unwrap(), Some(latin))
            .unwrap();
        memory.write_u64(memory.offset(args, 48).unwrap(), 0x1234).unwrap();
    }
    let format = text(&mut fixture, "%*.*d|%ld|%hhd|%.2s|%p");
    let outcome = {
        let (_, memory) = fixture.runner.cpu_parts();
        let mut request = GuestFormatRequest {
            memory,
            dialect: FormatDialect::Windows,
            format: Some(format),
            arguments: Some(args),
            buffer: Some(buffer),
            capacity: 256,
            termination: FormatTermination::C99,
            continue_count: false,
            rounding: FormatRounding::LegacyNearest,
            exponent_digits: 2,
            fortify: false,
        };
        format_guest_buffer(&mut request).unwrap()
    };
    assert_eq!(outcome.result, 34);
    assert_eq!(outcome.errno, None);
    let (_, memory) = fixture.runner.cpu_parts();
    assert_eq!(
        read_string(memory, buffer, false).unwrap(),
        "-007    |-1|-1|ÿa|0000000000001234"
    );
    let huge = text(&mut fixture, "%2147483647d!");
    let outcome = {
        let (_, memory) = fixture.runner.cpu_parts();
        let mut request = GuestFormatRequest {
            memory,
            dialect: FormatDialect::Windows,
            format: Some(huge),
            arguments: Some(args),
            buffer: None,
            capacity: 0,
            termination: FormatTermination::C99,
            continue_count: false,
            rounding: FormatRounding::LegacyNearest,
            exponent_digits: 2,
            fortify: false,
        };
        format_guest_buffer(&mut request).unwrap()
    };
    assert_eq!(outcome.result, -1);
    assert_eq!(outcome.errno, Some(132));
}

#[test]
fn ucrt_scanner_abi_destination_precision_and_assignment_counts() {
    for width in [4usize, 8usize] {
        let mut fixture = fixture(width);
        let hooks = Rc::clone(&fixture.hooks);
        let runtime = {
            let (_, memory) = fixture.runner.cpu_parts();
            WindowsGuestRuntime::new(
                hooks,
                memory,
                WindowsRuntimeOptions {
                    capabilities: Default::default(),
                    process_id: None,
                    thread_id: None,
                },
            )
            .unwrap()
        };
        runtime.attach_runner(&mut fixture.runner).unwrap();
        let service = service_address(
            &fixture,
            "windows",
            "api-ms-win-crt-stdio-l1-1-0.dll!__stdio_common_vsscanf",
        );
        let (destinations, args) = {
            let (_, memory) = fixture.runner.cpu_parts();
            (
                allocate(memory, 32, GuestPermissions::ReadWrite, 16),
                allocate(memory, width * 3, GuestPermissions::ReadWrite, 16),
            )
        };
        for index in 0..3 {
            let slot = offset_of(&mut fixture, args, (index * width) as i64);
            let target = offset_of(&mut fixture, destinations, (index * 8) as i64);
            let (_, memory) = fixture.runner.cpu_parts();
            memory.write_pointer(slot, Some(target)).unwrap();
        }
        let scan = |fixture: &mut Fixture,
                    input: &str,
                    format: &str,
                    capacity: u64,
                    options: u64,
                    locale: Option<GuestAddress>| {
            let input_address = text(fixture, input);
            let format_address = text(fixture, format);
            invoke_result(
                fixture,
                service,
                vec![
                    GuestCallValue::Uint64(options),
                    ptr(Some(input_address)),
                    uint(width, capacity),
                    ptr(Some(format_address)),
                    ptr(locale),
                    ptr(Some(args)),
                ],
            )
        };
        let max = if width == 8 { u64::MAX } else { u32::MAX as u64 };
        assert_eq!(scan(&mut fixture, "-1768.0", "%lf", max, 2, None), 1);
        let (_, memory) = fixture.runner.cpu_parts();
        assert_eq!(memory.read_f64(destinations).unwrap(), -1768.0);
        assert_eq!(scan(&mut fixture, " -12.5 010", "%f %i", max, 2, None), 2);
        let (_, memory) = fixture.runner.cpu_parts();
        assert_eq!(memory.read_f32(destinations).unwrap(), -12.5);
        assert_eq!(memory.read_i32(memory.offset(destinations, 8).unwrap()).unwrap(), 8);
        assert_eq!(scan(&mut fixture, "+0x2a", "%i", max, 2, None), 1);
        let (_, memory) = fixture.runner.cpu_parts();
        assert_eq!(memory.read_i32(destinations).unwrap(), 42);
        assert_eq!(scan(&mut fixture, "010", "%d", max, 2, None), 1);
        let (_, memory) = fixture.runner.cpu_parts();
        assert_eq!(memory.read_i32(destinations).unwrap(), 10);
        assert_eq!(scan(&mut fixture, "( 12.75%, -20)", "( %*5lf%%, %d)", max, 2, None), 1);
        let (_, memory) = fixture.runner.cpu_parts();
        assert_eq!(memory.read_i32(destinations).unwrap(), -20);
        assert_eq!(scan(&mut fixture, "12345", "%3d%d", max, 2, None), 2);
        let (_, memory) = fixture.runner.cpu_parts();
        assert_eq!(memory.read_i32(destinations).unwrap(), 123);
        assert_eq!(memory.read_i32(memory.offset(destinations, 8).unwrap()).unwrap(), 45);
        assert_eq!(scan(&mut fixture, "12.75", "%lf", 2, 2, None), 1);
        let (_, memory) = fixture.runner.cpu_parts();
        assert_eq!(memory.read_f64(destinations).unwrap(), 12.0);
        let unterminated = {
            let (_, memory) = fixture.runner.cpu_parts();
            allocate(memory, 3, GuestPermissions::ReadWrite, 16)
        };
        {
            let (_, memory) = fixture.runner.cpu_parts();
            memory.write(unterminated, &[49, 46, 53]).unwrap();
        }
        let format = text(&mut fixture, "%lf");
        let result = invoke_result(
            &mut fixture,
            service,
            vec![
                GuestCallValue::Uint64(2),
                ptr(Some(unterminated)),
                uint(width, 3),
                ptr(Some(format)),
                ptr(None),
                ptr(Some(args)),
            ],
        );
        assert_eq!(result, 1);
        let (_, memory) = fixture.runner.cpu_parts();
        assert_eq!(memory.read_f64(destinations).unwrap(), 1.5);
        {
            let (_, memory) = fixture.runner.cpu_parts();
            memory.write(destinations, &[0x7f; 32]).unwrap();
        }
        for input in ["", " \t\n"] {
            assert_eq!(scan(&mut fixture, input, "%lf", max, 2, None), -1);
        }
        for input in ["x", "invalid", "no", "+", ".", "1e+"] {
            assert_eq!(scan(&mut fixture, input, "%lf", max, 2, None), 0);
        }
        let (_, memory) = fixture.runner.cpu_parts();
        assert_eq!(memory.copy(destinations, 32).unwrap(), vec![0x7f; 32]);
        assert_eq!(scan(&mut fixture, "1", "%*d %d", max, 2, None), 0);
        assert_eq!(scan(&mut fixture, "1", "%d %d", max, 2, None), 1);
        assert_eq!(scan(&mut fixture, "a", "b%lf", max, 2, None), 0);
        assert_eq!(scan(&mut fixture, "", "a%lf", max, 2, None), -1);
        assert_eq!(scan(&mut fixture, "", " ", max, 2, None), 0);
        for format in ["%s", "%n", "%lld", "%[a]", "%0f", "%Lf", "%a"] {
            let input_address = text(&mut fixture, "1");
            let format_address = text(&mut fixture, format);
            let signature = {
                let (_, memory) = fixture.runner.cpu_parts();
                fixture
                    .hooks
                    .callbacks
                    .borrow_mut()
                    .handle(memory, service)
                    .unwrap()
                    .unwrap()
                    .signature
            };
            let request = GuestCallRequest {
                target: service,
                signature,
                arguments: vec![
                    GuestCallValue::Uint64(2),
                    ptr(Some(input_address)),
                    uint(width, max),
                    ptr(Some(format_address)),
                    ptr(None),
                    ptr(Some(args)),
                ],
                context: fixture.context.clone(),
                instruction_budget: 1000,
            };
            let failure = fixture.runner.invoke(&request).unwrap_err();
            assert!(format!("{failure:?}").contains("supported"), "{format}: {failure:?}");
        }
        // Bad options, explicit locale, and hex/inf floats are unsupported.
        for (input, format, capacity, options, locale) in [
            ("1", "%lf", 1, 1, None),
            ("0x1p0", "%lf", max, 2, None),
            ("inf", "%lf", max, 2, None),
        ] {
            let input_address = text(&mut fixture, input);
            let format_address = text(&mut fixture, format);
            let signature = {
                let (_, memory) = fixture.runner.cpu_parts();
                fixture
                    .hooks
                    .callbacks
                    .borrow_mut()
                    .handle(memory, service)
                    .unwrap()
                    .unwrap()
                    .signature
            };
            let request = GuestCallRequest {
                target: service,
                signature,
                arguments: vec![
                    GuestCallValue::Uint64(options),
                    ptr(Some(input_address)),
                    uint(width, capacity),
                    ptr(Some(format_address)),
                    ptr(locale),
                    ptr(Some(args)),
                ],
                context: fixture.context.clone(),
                instruction_budget: 1000,
            };
            let failure = fixture.runner.invoke(&request).unwrap_err();
            assert!(
                format!("{failure:?}").contains("supported"),
                "{input} {format}: {failure:?}"
            );
        }
        {
            let destinations_copy = destinations;
            let input_address = text(&mut fixture, "1");
            let format_address = text(&mut fixture, "%lf");
            let signature = {
                let (_, memory) = fixture.runner.cpu_parts();
                fixture
                    .hooks
                    .callbacks
                    .borrow_mut()
                    .handle(memory, service)
                    .unwrap()
                    .unwrap()
                    .signature
            };
            let request = GuestCallRequest {
                target: service,
                signature,
                arguments: vec![
                    GuestCallValue::Uint64(2),
                    ptr(Some(input_address)),
                    uint(width, 1),
                    ptr(Some(format_address)),
                    ptr(Some(destinations_copy)),
                    ptr(Some(args)),
                ],
                context: fixture.context.clone(),
                instruction_budget: 1000,
            };
            let failure = fixture.runner.invoke(&request).unwrap_err();
            assert!(format!("{failure:?}").contains("supported"), "{failure:?}");
        }
        assert_eq!(scan(&mut fixture, "-0", "%lf", max, 2, None), 1);
        let (_, memory) = fixture.runner.cpu_parts();
        assert_eq!(memory.read_u64(destinations).unwrap(), 0x8000_0000_0000_0000);
        assert_eq!(
            scan(
                &mut fixture,
                "1.00000005960464477539062500000000001",
                "%f",
                max,
                2,
                None
            ),
            1
        );
        let (_, memory) = fixture.runner.cpu_parts();
        assert_eq!(memory.read_u32(destinations).unwrap(), 0x3f80_0001);
        assert_eq!(scan(&mut fixture, "1.000000059604644775390625", "%f", max, 2, None), 1);
        let (_, memory) = fixture.runner.cpu_parts();
        assert_eq!(memory.read_u32(destinations).unwrap(), 0x3f80_0000);
        assert_eq!(scan(&mut fixture, "4.9406564584124654e-324", "%lf", max, 2, None), 1);
        let (_, memory) = fixture.runner.cpu_parts();
        assert_eq!(memory.read_u64(destinations).unwrap(), 1);
        assert_eq!(scan(&mut fixture, "1e9999", "%lf", max, 2, None), 1);
        let (_, memory) = fixture.runner.cpu_parts();
        assert_eq!(memory.read_f64(destinations).unwrap(), f64::INFINITY);
        assert_eq!(scan(&mut fixture, "-1e-9999", "%lf", max, 2, None), 1);
        let (_, memory) = fixture.runner.cpu_parts();
        assert_eq!(memory.read_u64(destinations).unwrap(), 0x8000_0000_0000_0000);
    }
}
