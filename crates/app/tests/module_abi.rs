#[path = "support/service_program.rs"]
mod service_program;
use qa_app::Runtime;
use qa_compat::{
    abi::{Addresses, Invocation, Q3_CLIENT, Q3_SERVER, Q3_UI, QvmCalls, UnknownCalls},
    memory::ModuleMemory,
    services::{CallContext, CallError, ServiceStorage},
};
use qa_console::{commands::Console, views::Context};
use qa_core::primitives::ThinkTime;
use qa_core::{
    events::FrameEvent,
    primitives::{ModuleId, RuleSetId},
    sys_events::EventTime,
};
use qa_world::{area::LinkOrder, entities::AllocationPolicy};

fn context() -> CallContext {
    CallContext {
        server_frame: 0,
        module: ModuleId(1),
        clock: ThinkTime::Milliseconds(17),
        console: Context::default(),
        allocation: AllocationPolicy::EDICT,
        link_order: LinkOrder::Head,
    }
}

#[test]
fn native_syscall_memory_width_stays_separate_from_child_runtime_imports() {
    let mut runtime = Runtime::load(1, std::iter::empty()).unwrap();
    let mut console = Console::new(Context::default()).unwrap();
    let mut storage = ServiceStorage::load(&[(ModuleId(1), 0)], 0, &console.cvars).unwrap();
    let mut scratch = runtime.geometry.scratch();
    let mut unknown = UnknownCalls::load(1).unwrap();
    let base = 1u64 << 40;
    let mut memory = ModuleMemory::load(base, 128, &[]).unwrap();
    memory.write(base + 16, b"\xffa\0").unwrap();
    let mut services = runtime.engine_services(&mut console, &mut storage, &mut scratch);
    let mut call = Invocation {
        services: &mut services,
        memory: &mut memory,
        native_cvars: None,
        native_resources: None,
        native_entities: None,
        native_surfaces: None,
        native_command: None,
        native_configs: None,
        context: context(),
        platform_time: EventTime(0),
        command: &[],
        addresses: Addresses::Native,
        arguments: &[base + 32, base + 16, (1u64 << 32) + 1],
    };
    assert_eq!(Q3_SERVER.invoke(101, &mut call, &mut unknown), Ok(0));
    assert_eq!(call.memory.read(base + 32, 1).unwrap(), b"\xff");
    assert_eq!(
        Q3_SERVER.invoke(qa_platform::native::runtime::FIRST, &mut call, &mut unknown),
        Ok(0)
    );
    assert_eq!(unknown.calls, 1);
}

#[test]
fn original_float_syscalls_keep_their_declared_bits() {
    use qa_compat::abi::QUAKEC;
    let mut runtime = Runtime::load(1, std::iter::empty()).unwrap();
    let mut console = Console::new(Context::default()).unwrap();
    let mut storage = ServiceStorage::load(&[(ModuleId(1), 0)], 0, &console.cvars).unwrap();
    let mut scratch = runtime.geometry.scratch();
    let mut unknown = UnknownCalls::load(1).unwrap();
    let mut memory = ModuleMemory::load(0, 64, &[]).unwrap();
    let mut services = runtime.engine_services(&mut console, &mut storage, &mut scratch);
    let mut invoke = |table: &qa_compat::abi::CallTable, number, addresses, arguments: &[u64]| {
        let mut call = Invocation {
            services: &mut services,
            memory: &mut memory,
            native_cvars: None,
            native_resources: None,
            native_entities: None,
            native_surfaces: None,
            native_command: None,
            native_configs: None,
            context: context(),
            platform_time: EventTime(0),
            command: &[],
            addresses,
            arguments,
        };
        table.invoke(number, &mut call, &mut unknown)
    };
    let functions: [(u32, &qa_compat::abi::CallTable, u32, fn(f64, f64) -> f64); 7] = [
        (6, &Q3_SERVER, 103, |x, _| x.sin()),
        (7, &Q3_SERVER, 104, |x, _| x.cos()),
        (8, &Q3_SERVER, 105, f64::atan2),
        (9, &Q3_SERVER, 106, |x, _| x.sqrt()),
        (10, &Q3_SERVER, 110, |x, _| x.floor()),
        (11, &Q3_SERVER, 111, |x, _| x.ceil()),
        (12, &Q3_CLIENT, 111, |x, _| x.acos()),
    ];
    for x in [
        -0.0,
        0.0,
        -0.5,
        0.5,
        1.0,
        -12345.6789,
        1e30,
        f64::INFINITY,
        f64::NAN,
    ] {
        for &(_, table, original, operation) in &functions {
            let x = x as f32;
            let args = [
                0xaabbccdd00000000 | u64::from(x.to_bits()),
                u64::from((-2.25f32).to_bits()),
            ];

            let actual =
                invoke(table, original, Addresses::Qvm { mask: 63 }, &args).unwrap() as u32;
            let expected = operation(f64::from(x), -2.25) as f32;
            if expected.is_nan() {
                assert!(f32::from_bits(actual).is_nan());
            } else {
                assert_eq!(actual, expected.to_bits(), "original math {original}({x})");
            }
        }
    }
    for bits in [0x80000000u32, 0xbf000000, 0xffc12345] {
        let args = [u64::from(bits)];

        assert_eq!(
            invoke(&QUAKEC, 43, Addresses::Native, &args),
            Ok(u64::from(bits & 0x7fffffff))
        );
    }
    assert_eq!(unknown.calls, 0);
}

#[test]
fn native_addresses_role_ordinals_cvar_conversion_and_byte_strings_use_existing_services() {
    let mut runtime = Runtime::load(1, std::iter::empty()).unwrap();
    let mut console = Console::new(Context::default()).unwrap();
    let mut storage = ServiceStorage::load(&[(ModuleId(1), 8)], 0, &console.cvars).unwrap();
    let mut scratch = runtime.geometry.scratch();
    let mut unknown = UnknownCalls::load(1).unwrap();
    let base = 1u64 << 40;
    let mut memory = ModuleMemory::load(base, 512, b"\0").unwrap();
    memory.write(base + 16, b"cg_fov\0").unwrap();
    memory.write(base + 32, b"1e3\0").unwrap();
    memory.write(base + 48, b"\x82native\n\0").unwrap();
    {
        let mut services = runtime.engine_services(&mut console, &mut storage, &mut scratch);
        let mut call = Invocation {
            services: &mut services,
            memory: &mut memory,
            native_cvars: None,
            native_resources: None,
            native_entities: None,
            native_surfaces: None,
            native_command: None,
            native_configs: None,
            context: context(),
            platform_time: EventTime((u64::from(u32::MAX) + 8) * 1_000_000),
            command: &[b"native", b"arg"],
            addresses: Addresses::Native,
            arguments: &[base + 16, base + 32],
        };
        assert_eq!(Q3_SERVER.invoke(5, &mut call, &mut unknown), Ok(0));
        let args1 = [base + 16];
        call.arguments = &args1;
        // atoi is not a float-to-integer cast: the same text has value 1000.
        assert_eq!(Q3_SERVER.invoke(6, &mut call, &mut unknown), Ok(1));
        assert_eq!(
            Q3_UI.invoke(4, &mut call, &mut unknown),
            Ok(1000.0f32.to_bits() as u64)
        );
        assert_eq!(Q3_SERVER.invoke(2, &mut call, &mut unknown), Ok(7));
        let args2 = [3, base + 48];
        call.arguments = &args2;
        assert_eq!(Q3_SERVER.invoke(18, &mut call, &mut unknown), Ok(0));
        let args3 = [3, base + 80, 4];
        call.arguments = &args3;
        assert_eq!(Q3_SERVER.invoke(19, &mut call, &mut unknown), Ok(0));
        assert_eq!(call.memory.read(base + 80, 4).unwrap(), b"\x82na\0");
        let args4 = [u32::MAX as u64, base + 80, 4];
        call.arguments = &args4;
        assert_eq!(Q3_UI.invoke(11, &mut call, &mut unknown), Ok(0));
        assert_eq!(call.memory.read(base + 80, 1).unwrap(), b"\0");
        let args5 = [base + 48];
        call.arguments = &args5;
        assert_eq!(Q3_UI.invoke(1, &mut call, &mut unknown), Ok(0));
        assert_eq!(
            Q3_UI.invoke(0, &mut call, &mut unknown),
            Err(CallError::Aborted)
        );
        assert_eq!(Q3_SERVER.invoke(1234, &mut call, &mut unknown), Ok(0));
        assert_eq!(Q3_SERVER.invoke(1234, &mut call, &mut unknown), Ok(0));
        assert_eq!(Q3_SERVER.invoke(1235, &mut call, &mut unknown), Ok(0));
        assert_eq!((unknown.calls, unknown.capacity_drops), (3, 1));
        let mut batch = services
            .server
            .events
            .batch(services.server.presentation)
            .unwrap();
        let mut prints = Vec::new();
        while let Some(record) = services.server.events.next(&mut batch) {
            if let FrameEvent::Print(p) = record.event {
                prints.push(services.server.events.texts.get(p.text).unwrap().to_vec());
            }
        }
        assert_eq!(
            prints,
            vec![
                b"\x82native\n".to_vec(),
                b"\x82native\n".to_vec(),
                b"module 1: unknown system call 1234\n".to_vec()
            ]
        );
    }
}

#[test]
fn real_qvm_calls_write_cvars_print_and_append_into_the_existing_console() {
    let mut vm = service_program::qvm();
    let mut runtime = Runtime::load(1, std::iter::empty()).unwrap();
    let mut console = Console::new(Context::default()).unwrap();
    let mut storage = ServiceStorage::load(&[(ModuleId(1), 0)], 0, &console.cvars).unwrap();
    let mut scratch = runtime.geometry.scratch();
    let mut unknown = UnknownCalls::load(4).unwrap();
    {
        let mut services = runtime.engine_services(&mut console, &mut storage, &mut scratch);
        let mut calls = QvmCalls {
            services: &mut services,
            table: &Q3_SERVER,
            context: context(),
            platform_time: EventTime(90_000_000),
            command: &[],
            unknown: &mut unknown,
        };
        assert_eq!(vm.call(&mut calls, [0; 10], 1000, false), Ok(105));
        assert_eq!(unknown.calls, 0);
    }
    console.execute_frame(&mut runtime);
    assert_eq!(
        console
            .cvars
            .numeric(
                console
                    .cvars
                    .bind("sensitivity", Context::default())
                    .unwrap()
            )
            .unwrap(),
        7.0
    );
    assert_eq!(
        console
            .cvars
            .value_in(console.cvars.find("cg_fov").unwrap(), RuleSetId::Quake3),
        105.0
    );
}

#[test]
fn server_and_client_console_imports_keep_their_different_arguments() {
    let mut runtime = Runtime::load(1, std::iter::empty()).unwrap();
    let mut console = Console::new(Context::default()).unwrap();
    let mut storage = ServiceStorage::load(&[], 0, &console.cvars).unwrap();
    let mut scratch = runtime.geometry.scratch();
    let mut unknown = UnknownCalls::load(4).unwrap();
    let mut memory = ModuleMemory::load(0, 128, b"\0sensitivity 6\n\0").unwrap();
    {
        let mut services = runtime.engine_services(&mut console, &mut storage, &mut scratch);
        let mut call = Invocation {
            services: &mut services,
            memory: &mut memory,
            native_cvars: None,
            native_resources: None,
            native_entities: None,
            native_surfaces: None,
            native_command: None,
            native_configs: None,
            context: context(),
            platform_time: EventTime(0),
            command: &[],
            addresses: Addresses::Qvm { mask: 127 },
            arguments: &[1],
        };
        assert_eq!(Q3_CLIENT.invoke(14, &mut call, &mut unknown), Ok(0));
        let args6 = [2, 1];
        call.arguments = &args6;
        assert_eq!(Q3_SERVER.invoke(14, &mut call, &mut unknown), Ok(0));
        let args7 = [0, 1];
        call.arguments = &args7;
        assert_eq!(
            Q3_SERVER.invoke(14, &mut call, &mut unknown),
            Err(CallError::Text)
        );
        let args8 = [1.25f32.to_bits() as u64];
        call.arguments = &args8;
        assert_eq!(
            Q3_SERVER.invoke(110, &mut call, &mut unknown),
            Ok(1.0f32.to_bits() as u64)
        );
        assert_eq!(
            Q3_CLIENT.invoke(107, &mut call, &mut unknown),
            Ok(1.0f32.to_bits() as u64)
        );
    }
}
