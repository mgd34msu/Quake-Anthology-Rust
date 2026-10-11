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
fn q3_module_setters_create_cvars_and_force_readonly_and_latched_values() {
    for (base, addresses) in [
        (0, Addresses::Qvm { mask: 511 }),
        (
            1u64 << 40,
            Addresses::Native {
                abi: qa_platform::native::NativeAbi::SystemV,
            },
        ),
    ] {
        for (table, ordinal) in [(&Q3_SERVER, 5), (&Q3_CLIENT, 5), (&Q3_UI, 3)] {
            let mut runtime = Runtime::load(1, std::iter::empty()).unwrap();
            let mut console = Console::new(Context::default()).unwrap();
            let mut call_context = context();
            call_context.console.source = RuleSetId::Quake3;
            let ctx = call_context.console;
            let gamma = console.cvars.bind("gamma", ctx).unwrap();
            console.cvars.set_text(gamma.canonical(), "0").unwrap();
            assert!(console.cvars.read(gamma).is_err());
            let readonly = console
                .cvars
                .register("_qa_module_rom", Some("1"), 64, ctx)
                .unwrap();
            let latched = console
                .cvars
                .register("_qa_module_latch", Some("1"), 32, ctx)
                .unwrap();
            assert!(console.cvars.write(readonly, "2").is_err());
            console.cvars.set_latch_active(latched.canonical(), true);
            console.cvars.write(latched, "2").unwrap();
            assert_eq!(
                console.cvars.latched(latched).unwrap().unwrap().as_str(),
                "2"
            );
            let mut storage = ServiceStorage::load(&[(ModuleId(1), 0)], 0, &console.cvars).unwrap();
            let mut scratch = runtime.geometry.scratch();
            let mut unknown = UnknownCalls::load(1).unwrap();
            let mut memory = ModuleMemory::load(base, 512, &[]).unwrap();
            let mut services = runtime.engine_services(&mut console, &mut storage, &mut scratch);
            for (name, value) in [
                ("_qa_module_new", "17"),
                ("_qa_module_rom", "4"),
                ("_qa_module_latch", "3"),
                ("_qa_module_new", "18"),
                ("gamma", "1.000000"),
            ] {
                memory.write_string(base + 16, 64, name.as_bytes()).unwrap();
                memory
                    .write_string(base + 96, 64, value.as_bytes())
                    .unwrap();
                let arguments = [base + 16, base + 96];
                let mut call = Invocation {
                    services: &mut services,
                    memory: &mut memory,
                    native_cvars: None,
                    native_resources: None,
                    native_entities: None,
                    native_surfaces: None,
                    native_command: None,
                    native_configs: None,
                    context: call_context,
                    platform_time: EventTime(0),
                    command: &[],
                    addresses,
                    arguments: &arguments,
                };
                assert_eq!(table.invoke(ordinal, &mut call, &mut unknown), Ok(0));
                let view = services.cvars.bind(name, ctx).unwrap();
                assert_eq!(services.cvars.read(view).unwrap().as_str(), value);
            }
            assert!(services.cvars.latched(latched).unwrap().is_none());
            assert_eq!(services.cvars.flags(readonly) & 64, 64);
            assert_eq!(unknown.calls, 0);
        }
    }
}

#[test]
fn q3_null_setters_reset_existing_views_and_leave_missing_names_absent() {
    for (base, addresses) in [
        (0, Addresses::Qvm { mask: 511 }),
        (
            1u64 << 40,
            Addresses::Native {
                abi: qa_platform::native::NativeAbi::SystemV,
            },
        ),
    ] {
        for (table, ordinal) in [(&Q3_SERVER, 5), (&Q3_CLIENT, 5), (&Q3_UI, 3)] {
            let mut runtime = Runtime::load(1, std::iter::empty()).unwrap();
            let mut console = Console::new(Context::default()).unwrap();
            let mut call_context = context();
            call_context.console.source = RuleSetId::Quake3;
            let ctx = call_context.console;
            let readonly = console
                .cvars
                .register("_qa_reset_rom", Some("1"), 64, ctx)
                .unwrap();
            let latched = console
                .cvars
                .register("_qa_reset_latch", Some("1"), 32, ctx)
                .unwrap();
            console.cvars.force_write(readonly, "2").unwrap();
            console.cvars.force_write(latched, "2").unwrap();
            console.cvars.set_latch_active(latched.canonical(), true);
            console.cvars.write(latched, "3").unwrap();
            let sensitivity = console.cvars.bind("sensitivity", ctx).unwrap();
            console.cvars.force_write(sensitivity, "9").unwrap();
            let mut storage = ServiceStorage::load(&[(ModuleId(1), 0)], 0, &console.cvars).unwrap();
            let mut scratch = runtime.geometry.scratch();
            let mut unknown = UnknownCalls::load(1).unwrap();
            let mut memory = ModuleMemory::load(base, 512, &[]).unwrap();
            // Zero in a QVM is NULL even when its data at offset zero is not empty.
            memory.write(base, b"7\0").unwrap();
            let mut services = runtime.engine_services(&mut console, &mut storage, &mut scratch);
            let before = services.cvars.entries().count();
            for (name, expected) in [
                ("_qa_reset_missing", None),
                ("_qa_reset_rom", Some("1")),
                ("_qa_reset_latch", Some("1")),
                ("sensitivity", Some("5")),
            ] {
                memory.write_string(base + 32, 64, name.as_bytes()).unwrap();
                let arguments = [base + 32, 0];
                let mut call = Invocation {
                    services: &mut services,
                    memory: &mut memory,
                    native_cvars: None,
                    native_resources: None,
                    native_entities: None,
                    native_surfaces: None,
                    native_command: None,
                    native_configs: None,
                    context: call_context,
                    platform_time: EventTime(0),
                    command: &[],
                    addresses,
                    arguments: &arguments,
                };
                assert_eq!(table.invoke(ordinal, &mut call, &mut unknown), Ok(0));
                let actual = services
                    .cvars
                    .bind(name, ctx)
                    .map(|view| services.cvars.read(view).unwrap().as_str().to_owned());
                assert_eq!(actual.as_deref(), expected);
            }
            assert_eq!(services.cvars.entries().count(), before);
            assert!(services.cvars.latched(latched).unwrap().is_none());
            assert_eq!(services.cvars.flags(readonly) & 64, 64);
            assert_eq!(unknown.calls, 0);
            // The same native value leaves an outstanding latch untouched.
            services.cvars.write(latched, "2").unwrap();
            memory
                .write_string(base + 32, 64, b"_qa_reset_latch")
                .unwrap();
            memory.write_string(base + 96, 64, b"1").unwrap();
            let arguments = [base + 32, base + 96];
            let mut call = Invocation {
                services: &mut services,
                memory: &mut memory,
                native_cvars: None,
                native_resources: None,
                native_entities: None,
                native_surfaces: None,
                native_command: None,
                native_configs: None,
                context: call_context,
                platform_time: EventTime(0),
                command: &[],
                addresses,
                arguments: &arguments,
            };
            assert_eq!(table.invoke(ordinal, &mut call, &mut unknown), Ok(0));
            assert_eq!(
                services.cvars.latched(latched).unwrap().unwrap().as_str(),
                "2"
            );
            if base == 0 {
                // QuakeC string offset zero is a string, not Q3's NULL reset.
                memory.write_string(32, 64, b"sensitivity").unwrap();
                let arguments = [32, 512];
                let mut call = Invocation {
                    services: &mut services,
                    memory: &mut memory,
                    native_cvars: None,
                    native_resources: None,
                    native_entities: None,
                    native_surfaces: None,
                    native_command: None,
                    native_configs: None,
                    context: call_context,
                    platform_time: EventTime(0),
                    command: &[],
                    addresses,
                    arguments: &arguments,
                };
                assert_eq!(table.invoke(ordinal, &mut call, &mut unknown), Ok(0));
                assert_eq!(services.cvars.read(sensitivity).unwrap().as_str(), "7");
                services.cvars.force_write(sensitivity, "9").unwrap();
                let arguments = [32, 0];
                let mut call = Invocation {
                    services: &mut services,
                    memory: &mut memory,
                    native_cvars: None,
                    native_resources: None,
                    native_entities: None,
                    native_surfaces: None,
                    native_command: None,
                    native_configs: None,
                    context: call_context,
                    platform_time: EventTime(0),
                    command: &[],
                    addresses: Addresses::Native {
                        abi: qa_platform::native::NativeAbi::SystemV,
                    },
                    arguments: &arguments,
                };
                assert_eq!(
                    qa_compat::abi::QUAKEC.invoke(72, &mut call, &mut unknown),
                    Ok(0)
                );
                assert_eq!(services.cvars.read(sensitivity).unwrap().as_str(), "7");
            }
        }
    }
}

#[test]
fn q3_ui_create_and_reset_preserve_native_flags_protection_and_latches() {
    for (base, addresses) in [
        (0, Addresses::Qvm { mask: 511 }),
        (
            1u64 << 40,
            Addresses::Native {
                abi: qa_platform::native::NativeAbi::SystemV,
            },
        ),
    ] {
        let mut runtime = Runtime::load(1, std::iter::empty()).unwrap();
        let mut console = Console::new(Context::default()).unwrap();
        let ctx = context();
        let mut storage = ServiceStorage::load(&[(ModuleId(1), 0)], 0, &console.cvars).unwrap();
        let mut scratch = runtime.geometry.scratch();
        let mut unknown = UnknownCalls::load(1).unwrap();
        let mut memory = ModuleMemory::load(base, 512, &[]).unwrap();
        let mut services = runtime.engine_services(&mut console, &mut storage, &mut scratch);
        let mut invoke = |services: &mut qa_compat::services::EngineServices<'_>,
                          ordinal,
                          name: &str,
                          value: Option<&str>,
                          flags: u32| {
            memory
                .write_string(base + 32, 128, name.as_bytes())
                .unwrap();
            let value_pointer = if let Some(value) = value {
                memory
                    .write_string(base + 192, 128, value.as_bytes())
                    .unwrap();
                base + 192
            } else {
                0
            };
            let arguments = [base + 32, value_pointer, u64::from(flags)];
            let mut call = Invocation {
                services,
                memory: &mut memory,
                native_cvars: None,
                native_resources: None,
                native_entities: None,
                native_surfaces: None,
                native_command: None,
                native_configs: None,
                context: ctx,
                platform_time: EventTime(0),
                command: &[],
                addresses,
                arguments: &arguments,
            };
            assert_eq!(Q3_UI.invoke(ordinal, &mut call, &mut unknown), Ok(0));
        };
        let before = services.cvars.entries().count();
        invoke(&mut services, 7, "_qa_ui_missing", None, 0);
        assert_eq!(services.cvars.entries().count(), before);
        for (name, flags) in [
            ("_qa_ui_rom", 64),
            ("_qa_ui_init", 16),
            ("_qa_ui_cheat", 512),
        ] {
            invoke(&mut services, 8, name, Some("1"), flags);
            invoke(&mut services, 3, name, Some("2"), 0);
            let view = services.cvars.bind(name, ctx.console).unwrap();
            services.cvars.initialized = true;
            services.cvars.cheats = false;
            invoke(&mut services, 7, name, None, 0);
            assert_eq!(services.cvars.read(view).unwrap().as_str(), "2");
            assert_eq!(services.cvars.flags(view) & flags, flags);
            invoke(&mut services, 3, name, None, 0);
            assert_eq!(services.cvars.read(view).unwrap().as_str(), "1");
        }
        invoke(&mut services, 8, "_qa_ui_latch", Some("1"), 32);
        invoke(&mut services, 3, "_qa_ui_latch", Some("2"), 0);
        let view = services.cvars.bind("_qa_ui_latch", ctx.console).unwrap();
        services.cvars.set_latch_active(view.canonical(), true);
        invoke(&mut services, 7, "_qa_ui_latch", None, 0);
        assert_eq!(services.cvars.read(view).unwrap().as_str(), "2");
        assert_eq!(services.cvars.latched(view).unwrap().unwrap().as_str(), "1");
        invoke(&mut services, 3, "_qa_ui_latch", None, 0);
        assert_eq!(services.cvars.read(view).unwrap().as_str(), "1");
        assert!(services.cvars.latched(view).unwrap().is_none());
        invoke(&mut services, 8, "_qa_ui_existing", Some("5"), 0);
        invoke(&mut services, 3, "_qa_ui_existing", Some("8"), 0);
        let view = services.cvars.bind("_qa_ui_existing", ctx.console).unwrap();
        let entries = services.cvars.entries().count();
        invoke(&mut services, 8, "_QA_UI_EXISTING", Some("99"), 64);
        assert_eq!(services.cvars.entries().count(), entries);
        assert_eq!(services.cvars.read(view).unwrap().as_str(), "8");
        assert_eq!(services.cvars.flags(view) & 64, 64);
        invoke(&mut services, 3, "_qa_ui_existing", None, 0);
        assert_eq!(services.cvars.read(view).unwrap().as_str(), "5");
        assert_eq!(unknown.calls, 0);
        assert_eq!(services.server.events.len(), 3);
    }
}

#[test]
fn q3_ui_numeric_setter_preserves_native_formatting_and_forced_writes() {
    // Cvar_SetValue's %i/%f branches and val[32], checked against libc snprintf.
    let values: [(u32, &str); 15] = [
        (0x00000000, "0"),
        (0x80000000, "0"),
        (0x3f800000, "1"),
        (0xc0000000, "-2"),
        (0x3f000000, "0.500000"),
        (0xbfa00000, "-1.250000"),
        (0x3f9e0651, "1.234568"),
        (0x4effffff, "2147483520"),
        (0x4f000000, "2147483648.000000"),
        (0xcf000000, "-2147483648"),
        (0x7149f2ca, "1000000015047466219876688855040"),
        (0x7f800000, "inf"),
        (0xff800000, "-inf"),
        (0x7fc00000, "nan"),
        (0xffc00000, "-nan"),
    ];
    for (base, addresses) in [
        (0, Addresses::Qvm { mask: 511 }),
        (
            1u64 << 40,
            Addresses::Native {
                abi: qa_platform::native::NativeAbi::SystemV,
            },
        ),
    ] {
        let mut runtime = Runtime::load(1, std::iter::empty()).unwrap();
        let mut console = Console::new(Context::default()).unwrap();
        let ctx = context();
        let rom = console
            .cvars
            .register("_qa_ui_number_rom", Some("1"), 64, ctx.console)
            .unwrap();
        let latch = console
            .cvars
            .register("_qa_ui_number_latch", Some("1"), 32, ctx.console)
            .unwrap();
        console.cvars.set_latch_active(latch.canonical(), true);
        console.cvars.write(latch, "3").unwrap();
        let mut storage = ServiceStorage::load(&[(ModuleId(1), 0)], 0, &console.cvars).unwrap();
        let mut scratch = runtime.geometry.scratch();
        let mut unknown = UnknownCalls::load(1).unwrap();
        let mut memory = ModuleMemory::load(base, 512, &[]).unwrap();
        let mut services = runtime.engine_services(&mut console, &mut storage, &mut scratch);
        for (name, bits, expected) in values
            .iter()
            .map(|&(bits, text)| ("_qa_ui_number", bits, text))
            .chain([
                ("_qa_ui_number_rom", 0x40000000, "2"),
                ("_qa_ui_number_latch", 0x40000000, "2"),
            ])
        {
            memory
                .write_string(base + 32, 128, name.as_bytes())
                .unwrap();
            let arguments = [base + 32, 0xfedcba9800000000 | u64::from(bits)];
            let mut call = Invocation {
                services: &mut services,
                memory: &mut memory,
                native_cvars: None,
                native_resources: None,
                native_entities: None,
                native_surfaces: None,
                native_command: None,
                native_configs: None,
                context: ctx,
                platform_time: EventTime(0),
                command: &[],
                addresses,
                arguments: &arguments,
            };
            assert_eq!(Q3_UI.invoke(6, &mut call, &mut unknown), Ok(0));
            let view = services.cvars.bind(name, ctx.console).unwrap();
            assert_eq!(services.cvars.read(view).unwrap().as_str(), expected);
        }
        assert_eq!(services.cvars.flags(rom) & 64, 64);
        assert!(services.cvars.latched(latch).unwrap().is_none());
        assert_eq!(services.server.events.len(), 1);
        assert_eq!(unknown.calls, 0);
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
        addresses: Addresses::Native {
            abi: qa_platform::native::NativeAbi::SystemV,
        },
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
            invoke(
                &QUAKEC,
                43,
                Addresses::Native {
                    abi: qa_platform::native::NativeAbi::SystemV
                },
                &args
            ),
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
            addresses: Addresses::Native {
                abi: qa_platform::native::NativeAbi::SystemV,
            },
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
