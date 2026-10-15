#[path = "support/scalar_import.rs"]
mod scalar_import;
use qa_app::Runtime;
use qa_compat::{
    abi::{Addresses, CallTable, Invocation, Q3_CLIENT, Q3_SERVER, Q3_UI, UnknownCalls},
    memory::ModuleMemory,
    services::{CallContext, CallError, ServiceStorage},
};
use qa_console::{
    commands::Console,
    views::{Context, Role},
};
use qa_core::{
    primitives::{ModuleId, ThinkTime},
    sys_events::EventTime,
};
use qa_world::{area::LinkOrder, entities::AllocationPolicy};

#[test]
fn q3_vm_cvars_share_registry_and_match_native_layout_and_reference_rows() {
    for (base, addresses) in [
        (0, Addresses::Qvm { mask: 4095 }),
        (
            1 << 40,
            Addresses::Native {
                pointer_bytes: 8,
                abi: qa_platform::native::NativeAbi::SystemV,
            },
        ),
        (
            1 << 40,
            Addresses::Native {
                pointer_bytes: 8,
                abi: qa_platform::native::NativeAbi::Microsoft,
            },
        ),
    ] {
        for (table, register, update, role) in [
            (&Q3_SERVER, 3, 4, Role::Game),
            (&Q3_CLIENT, 3, 4, Role::Cgame),
            (&Q3_UI, 50, 51, Role::Engine),
        ] {
            let mut runtime = Runtime::load(1, std::iter::empty()).unwrap();
            let mut console = Console::new(Context::default()).unwrap();
            let mut storage = ServiceStorage::load(&[], 0, &console.cvars).unwrap();
            let mut scratch = runtime.geometry.scratch();
            let mut services = runtime.engine_services(&mut console, &mut storage, &mut scratch);
            let context = CallContext {
                native_rules: qa_core::primitives::RuleSetId::Quake3,
                trace_rules: qa_core::primitives::RuleSetId::Quake3,
                sound_policy: qa_gameplay::rules::sound_policy(
                    qa_core::primitives::RuleSetId::Quake3,
                ),
                server_frame: 0,
                module: ModuleId(1),
                clock: ThinkTime::Milliseconds(0),
                console: Context {
                    role,
                    ..Context::default()
                },
                allocation: AllocationPolicy::EDICT,
                link_order: LinkOrder::Head,
            };
            let mut memory = ModuleMemory::load(base, 4096, &[]).unwrap();
            let mut unknown = UnknownCalls::load(1).unwrap();
            let mut call = Invocation {
                services: &mut services,
                memory: &mut memory,
                native_cvars: None,
                native_resources: None,
                native_entities: None,
                native_surfaces: None,
                native_command: None,
                native_configs: None,
                native_pmove: None,
                native_player: None,
                context,
                platform_time: EventTime(0),
                command: Default::default(),
                argument_selection: &mut qa_compat::abi::ArgumentSelection::RetainedClient,
                addresses,
                arguments: &[],
            };
            call.memory
                .write_string(base + 16, 64, b"_qa_vm_cvar")
                .unwrap();
            call.memory.write_string(base + 80, 64, b"1e3").unwrap();
            call.memory.read_mut(base + 256, 272).unwrap().fill(0xcc);
            invoke(
                table,
                &mut call,
                &mut unknown,
                register,
                &[base + 256, base + 16, base + 80, 0xffff_ffff_0000_0001],
            )
            .unwrap();
            let handle = call.memory.read_word(base + 256).unwrap();
            assert_eq!(call.memory.read_word(base + 260).unwrap(), 1);
            assert_eq!(
                call.memory.read_word(base + 264).unwrap() as u32,
                1000f32.to_bits()
            );
            assert_eq!(call.memory.read_word(base + 268).unwrap(), 1);
            assert_eq!(call.memory.cstring(base + 272).unwrap(), b"1e3");
            assert!(
                call.memory
                    .read(base + 276, 252)
                    .unwrap()
                    .iter()
                    .all(|&b| b == 0)
            );
            let view = call
                .services
                .cvars
                .bind("_qa_vm_cvar", context.console)
                .unwrap();
            assert_eq!(call.services.cvars.flags(view) & 1, 1);
            compare_reference_rows(table, &mut call, &mut unknown, register, update, base, view);
            // Repeated registration reuses the view and keeps the current value.
            call.memory
                .write_string(base + 16, 64, b"_QA_VM_CVAR")
                .unwrap();
            call.memory.write_string(base + 80, 64, b"99").unwrap();
            invoke(
                table,
                &mut call,
                &mut unknown,
                register,
                &[base + 576, base + 16, base + 80, 0],
            )
            .unwrap();
            assert_eq!(call.memory.read_word(base + 576).unwrap(), handle);
            assert_eq!(call.memory.cstring(base + 592).unwrap(), b"1e3");
            // Native Cvar_Update deliberately trusts an equal modificationCount.
            call.memory
                .write_string(base + 272, 256, b"retained")
                .unwrap();
            invoke(table, &mut call, &mut unknown, update, &[base + 256]).unwrap();
            assert_eq!(call.memory.cstring(base + 272).unwrap(), b"retained");
            let prior_count = call.memory.read_word(base + 260).unwrap();
            call.services.cvars.force_write(view, "17.25").unwrap();
            call.services.cvars.force_write(view, "19.5").unwrap();
            invoke(table, &mut call, &mut unknown, update, &[base + 256]).unwrap();
            assert_eq!(call.memory.read_word(base + 260).unwrap(), prior_count + 2);
            assert_eq!(
                call.memory.read_word(base + 264).unwrap() as u32,
                19.5f32.to_bits()
            );
            assert_eq!(call.memory.read_word(base + 268).unwrap(), 19);
            assert_eq!(call.memory.cstring(base + 272).unwrap(), b"19.5");
            for (text, integer) in [
                ("0xFF", 0),
                ("214748364999", i32::MAX),
                ("-214748364999", i32::MIN),
            ] {
                call.services.cvars.force_write(view, text).unwrap();
                invoke(table, &mut call, &mut unknown, update, &[base + 256]).unwrap();
                assert_eq!(call.memory.read_word(base + 268).unwrap(), integer);
                assert_eq!(call.memory.cstring(base + 272).unwrap(), text.as_bytes());
            }
            // Invalid native handles and extents reject before touching the row.
            call.memory.write_word(base + 256, -1).unwrap();
            let before = call.memory.read(base + 256, 272).unwrap().to_vec();
            assert_eq!(
                invoke(table, &mut call, &mut unknown, update, &[base + 256]),
                Err(CallError::Cvar)
            );
            assert_eq!(call.memory.read(base + 256, 272).unwrap(), before);
            assert_eq!(
                invoke(table, &mut call, &mut unknown, update, &[base + 4095]),
                Err(CallError::Memory)
            );
            assert_eq!(
                invoke(table, &mut call, &mut unknown, update, &[0]),
                Err(CallError::Memory)
            );
            call.memory.write_word(base + 256, handle).unwrap();
            // The original limit includes the trailing NUL: 255 succeeds, 256 traps.
            call.services
                .cvars
                .force_write(view, &"x".repeat(255))
                .unwrap();
            invoke(table, &mut call, &mut unknown, update, &[base + 256]).unwrap();
            assert_eq!(call.memory.cstring(base + 272).unwrap().len(), 255);
            call.services
                .cvars
                .force_write(view, &"y".repeat(256))
                .unwrap();
            assert_eq!(
                invoke(table, &mut call, &mut unknown, update, &[base + 256]),
                Err(CallError::Text)
            );
            // NULL registration still creates a cvar without writing a vmCvar.
            call.memory
                .write_string(base + 16, 64, b"_qa_vm_null")
                .unwrap();
            let first = call.memory.read(base, 16).unwrap().to_vec();
            invoke(
                table,
                &mut call,
                &mut unknown,
                register,
                &[0, base + 16, base + 80, 0],
            )
            .unwrap();
            assert!(
                call.services
                    .cvars
                    .bind("_qa_vm_null", context.console)
                    .is_some()
            );
            assert_eq!(call.memory.read(base, 16).unwrap(), first);
            if base == 0 {
                // A nonzero QVM argument wrapping to zero is actual data.
                invoke(table, &mut call, &mut unknown, register, &[4096, 16, 80, 0]).unwrap();
                assert_eq!(call.memory.read_word(4).unwrap(), 1);
                assert_eq!(call.memory.cstring(16).unwrap(), b"99");
            }
            assert_eq!(unknown.calls, 0);
        }
    }
}

fn compare_reference_rows(
    table: &CallTable,
    call: &mut Invocation<'_, '_, '_>,
    unknown: &mut UnknownCalls,
    register: u32,
    update: u32,
    base: u64,
    view: qa_console::cvars::View,
) {
    // qsrc Cvar_Get/Set2/Register/Update: includes two writes between updates,
    // unchanged values, changed/repeated latches, applying a latch and reset.
    let mut actual = Vec::new();
    let row =
        |call: &mut Invocation<'_, '_, '_>, unknown: &mut UnknownCalls, actual: &mut Vec<u8>| {
            invoke(table, call, unknown, update, &[base + 256]).unwrap();
            actual.extend_from_slice(call.memory.read(base + 256, 272).unwrap());
        };
    row(call, unknown, &mut actual);
    call.services.cvars.force_write(view, "17.25").unwrap();
    row(call, unknown, &mut actual);
    call.services.cvars.force_write(view, "19.5").unwrap();
    call.services.cvars.force_write(view, "3.5").unwrap();
    row(call, unknown, &mut actual);
    call.services.cvars.force_write(view, "3.5").unwrap();
    row(call, unknown, &mut actual);
    call.services
        .cvars
        .register("_qa_vm_cvar", Some("ignored"), 32, call.context.console)
        .unwrap();
    call.services.cvars.set_latch_active(view.canonical(), true);
    call.services.cvars.write(view, "7").unwrap();
    row(call, unknown, &mut actual);
    call.services.cvars.write(view, "7").unwrap();
    row(call, unknown, &mut actual);
    call.services.cvars.write(view, "9").unwrap();
    row(call, unknown, &mut actual);
    invoke(
        table,
        call,
        unknown,
        register,
        &[base + 256, base + 16, base + 80, 32],
    )
    .unwrap();
    row(call, unknown, &mut actual);
    call.services.cvars.reset_view(view, false).unwrap();
    row(call, unknown, &mut actual);
    invoke(
        table,
        call,
        unknown,
        register,
        &[base + 256, base + 16, base + 80, 0],
    )
    .unwrap();
    row(call, unknown, &mut actual);
    call.memory.read_mut(base + 272, 256).unwrap().fill(0);
    call.memory
        .write_string(base + 272, 256, b"retained")
        .unwrap();
    row(call, unknown, &mut actual);
    // Checked original-C rows are also portable fixtures for ordinary CI.
    let rows: [(i32, f32, i32, &[u8]); 11] = [
        (1, 1000.0, 1, b"1e3"),
        (2, 17.25, 17, b"17.25"),
        (4, 3.5, 3, b"3.5"),
        (4, 3.5, 3, b"3.5"),
        (5, 3.5, 3, b"3.5"),
        (5, 3.5, 3, b"3.5"),
        (6, 3.5, 3, b"3.5"),
        (7, 9.0, 9, b"9"),
        (8, 9.0, 9, b"9"),
        (9, 1000.0, 1, b"1e3"),
        (9, 1000.0, 1, b"retained"),
    ];
    let expected: Vec<_> = rows
        .into_iter()
        .flat_map(|(count, value, integer, text)| {
            let mut row = [0u8; 272];
            row[4..8].copy_from_slice(&count.to_le_bytes());
            row[8..12].copy_from_slice(&value.to_bits().to_le_bytes());
            row[12..16].copy_from_slice(&integer.to_le_bytes());
            row[16..16 + text.len()].copy_from_slice(text);
            row
        })
        .collect();
    assert_eq!(actual, expected);
    if let Ok(path) = std::env::var("QA_Q3_VMCVAR_REFERENCE") {
        assert_eq!(actual, std::fs::read(path).unwrap());
    }
}

fn invoke(
    table: &CallTable,
    call: &mut Invocation<'_, '_, '_>,
    unknown: &mut UnknownCalls,
    ordinal: u32,
    arguments: &[u64],
) -> Result<u64, CallError> {
    // Invocation arguments are borrowed only for this boundary call.
    let mut invocation = Invocation {
        services: call.services,
        memory: call.memory,
        native_cvars: None,
        native_resources: None,
        native_entities: None,
        native_surfaces: None,
        native_command: None,
        native_configs: None,
        native_pmove: None,
        native_player: None,
        context: call.context,
        platform_time: call.platform_time,
        command: Default::default(),
        argument_selection: &mut qa_compat::abi::ArgumentSelection::RetainedClient,
        addresses: call.addresses,
        arguments,
    };
    table
        .invoke(ordinal, &mut invocation, unknown)
        .map(scalar_import::word)
}
#[path = "support/service_program.rs"]
#[allow(dead_code)]
mod service_program;

#[test]
fn session_function_dispatch_drives_vm_cvar_registration_and_updates() {
    use qa_app::{
        host::FrameHost,
        modules::{Export, ModuleRequest, ModuleResult, Phase, Program},
    };
    use qa_core::primitives::{CallbackId, RuleSetId};
    use qa_formats::program::qvm::Opcode::*;
    use qa_session::timing::TickRate;
    for (table, register, update, setter, role) in [
        (&Q3_SERVER, 3, 4, 5, Role::Game),
        (&Q3_CLIENT, 3, 4, 5, Role::Cgame),
        (&Q3_UI, 50, 51, 3, Role::Engine),
    ] {
        let mut operations = vec![(Enter, 128)];
        for (ordinal, args) in [
            (register, &[256, 16, 48, 0][..]),
            (setter, &[16, 64][..]),
            (setter, &[16, 80][..]),
            (update, &[256][..]),
        ] {
            for (index, &value) in args.iter().enumerate() {
                operations.extend([(Const, value), (Arg, (8 + index * 4) as i32)]);
            }
            operations.extend([(Const, -ordinal - 1), (Call, 0), (Pop, 0)]);
        }
        operations.extend([(Const, 260), (Load4, 0), (Leave, 128)]);
        let mut data = [0; 96];
        for (offset, text) in [
            (16, b"_qa_session_cvar\0".as_slice()),
            (48, b"1\0"),
            (64, b"2\0"),
            (80, b"3\0"),
        ] {
            data[offset..offset + text.len()].copy_from_slice(text);
        }
        let mut runtime = Runtime::load(1, std::iter::empty()).unwrap();
        let anchor = runtime
            .server
            .entities
            .allocate(
                ThinkTime::Milliseconds(0),
                ModuleId(1),
                AllocationPolicy::q3(0),
            )
            .unwrap()
            .id;
        let request = ModuleRequest {
            context: CallContext {
                native_rules: qa_core::primitives::RuleSetId::Quake3,
                trace_rules: qa_core::primitives::RuleSetId::Quake3,
                sound_policy: qa_gameplay::rules::sound_policy(
                    qa_core::primitives::RuleSetId::Quake3,
                ),
                server_frame: 0,
                module: ModuleId(1),
                clock: ThinkTime::Milliseconds(0),
                console: Context {
                    role,
                    ..Context::default()
                },
                allocation: AllocationPolicy::q3(0),
                link_order: LinkOrder::Head,
            },
            phase: Phase::Server,
            timing_rules: RuleSetId::Quake3,
            rate: TickRate::fixed(50).unwrap(),
            anchor,
            program: Program::Qvm {
                vm: Box::new(service_program::program(&operations, &data)),
                imports: table,
            },
            entries: vec![0],
            frame: Export::clocked(CallbackId(0)),
            prepare: vec![],
            initialize: vec![],
            api: None,
            shutdown: vec![],
            instruction_budget: 1000,
            configstrings: 0,
            files: 0,
            entity_source: None,
        };
        let mut host = FrameHost::load_modules(
            Console::new(Context::default()).unwrap(),
            runtime,
            TickRate::FrameDriven,
            vec![request],
        )
        .unwrap();
        for count in [3, 5] {
            host.call_module_export(
                ModuleId(1),
                Export::clocked(CallbackId(0)),
                ThinkTime::Milliseconds(50),
            )
            .unwrap();
            assert_eq!(
                host.module_counts(ModuleId(1)).unwrap().last_result,
                Some(ModuleResult::Qvm(count))
            );
        }
    }
}
