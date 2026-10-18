#[path = "support/scalar_import.rs"]
mod scalar_import;
use qa_app::Runtime;
use qa_compat::{
    abi::{Addresses, Invocation, Q3_CLIENT, Q3_SERVER, UnknownCalls},
    memory::ModuleMemory,
    services::{CallContext, CallError, ServiceStorage},
};
use qa_console::{
    commands::Console,
    views::{Context, Role},
};
use qa_core::{
    primitives::{ModuleId, ThinkTime, Vec3},
    sys_events::EventTime,
};
use qa_world::{area::LinkOrder, entities::AllocationPolicy};

#[test]
fn snap_imports_keep_x87_ties_zero_sign_and_integer_indefinite_results() {
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
        for (table, ordinal, role) in [(&Q3_SERVER, 42, Role::Game), (&Q3_CLIENT, 71, Role::Cgame)]
        {
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
            let cases = [
                ([0.5, 1.5, 2.5], [0.0, 2.0, 2.0]),
                ([-0.5, -1.5, -2.5], [0.0, -2.0, -2.0]),
                ([-0.0, f32::from_bits(1), -f32::from_bits(1)], [0.0; 3]),
                (
                    [f32::NAN, f32::INFINITY, f32::NEG_INFINITY],
                    [i32::MIN as f32; 3],
                ),
                (
                    [2147483648.0, -2147483648.0, f32::MAX],
                    [i32::MIN as f32; 3],
                ),
            ];
            let rows = std::env::var("QA_Q3_SNAP_REFERENCE")
                .ok()
                .map(|path| std::fs::read(path).unwrap());
            let vectors = rows.as_ref().map_or_else(
                || cases.to_vec(),
                |bytes| {
                    assert_eq!(bytes.len() % 24, 0);
                    bytes
                        .chunks_exact(24)
                        .map(|row| {
                            let vector = |start| {
                                std::array::from_fn(|axis| {
                                    let at = start + axis * 4;
                                    f32::from_bits(u32::from_le_bytes(
                                        row[at..at + 4].try_into().unwrap(),
                                    ))
                                })
                            };
                            (vector(0), vector(12))
                        })
                        .collect()
                },
            );
            for (input, expected) in vectors {
                memory.write_vec3(base + 16, Vec3(input)).unwrap();
                assert_eq!(
                    invoke(
                        table,
                        ordinal,
                        &[base + 16],
                        &mut services,
                        &mut memory,
                        context,
                        addresses,
                        &mut unknown
                    ),
                    Ok(0)
                );
                assert_eq!(
                    memory.read_vec3(base + 16).unwrap().0.map(f32::to_bits),
                    expected.map(f32::to_bits),
                    "{input:?}"
                );
            }
            memory.write_vec3(base, Vec3([-0.5, 1.5, -2.5])).unwrap();
            let before = memory.read(base, 12).unwrap().to_vec();
            assert_eq!(
                invoke(
                    table,
                    ordinal,
                    &[0],
                    &mut services,
                    &mut memory,
                    context,
                    addresses,
                    &mut unknown
                ),
                Err(CallError::Memory)
            );
            assert_eq!(memory.read(base, 12).unwrap(), before);
            assert_eq!(
                invoke(
                    table,
                    ordinal,
                    &[base + 4095],
                    &mut services,
                    &mut memory,
                    context,
                    addresses,
                    &mut unknown
                ),
                Err(CallError::Memory)
            );
            if base == 0 {
                assert_eq!(
                    invoke(
                        table,
                        ordinal,
                        &[4096],
                        &mut services,
                        &mut memory,
                        context,
                        addresses,
                        &mut unknown
                    ),
                    Ok(0)
                );
                assert_eq!(
                    memory.read_vec3(0).unwrap().0.map(f32::to_bits),
                    [0.0f32, 2.0, -2.0].map(f32::to_bits)
                );
            }
            assert_eq!(unknown.calls, 0);
        }
    }
}

fn invoke(
    table: &qa_compat::abi::CallTable,
    ordinal: u32,
    arguments: &[u64],
    services: &mut qa_compat::services::EngineServices<'_>,
    memory: &mut ModuleMemory<'_>,
    context: CallContext,
    addresses: Addresses,
    unknown: &mut UnknownCalls,
) -> Result<u64, CallError> {
    table
        .invoke(
            ordinal,
            &mut Invocation {
                services,
                memory,
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
                arguments,
            },
            unknown,
        )
        .map(scalar_import::word)
}

#[path = "support/service_program.rs"]
#[allow(dead_code)]
mod service_program;

#[test]
fn session_dispatch_calls_snap_import_from_qvm_game_and_cgame() {
    use qa_app::{
        host::FrameHost,
        modules::{Export, ModuleRequest, ModuleResult, Phase, Program},
    };
    use qa_core::primitives::{CallbackId, RuleSetId};
    use qa_formats::program::qvm::Opcode::*;
    use qa_session::timing::TickRate;
    for (table, ordinal, role, phase) in [
        (&Q3_SERVER, 42, Role::Game, Phase::Server),
        (&Q3_CLIENT, 71, Role::Cgame, Phase::Client),
    ] {
        let operations = [
            (Enter, 64),
            (Const, 16),
            (Arg, 8),
            (Const, -ordinal - 1),
            (Call, 0),
            (Pop, 0),
            (Const, 16),
            (Load4, 0),
            (Leave, 64),
        ];
        let mut data = [0; 32];
        for (at, value) in [1.5f32, -0.5, -2.5].into_iter().enumerate() {
            data[16 + at * 4..20 + at * 4].copy_from_slice(&value.to_bits().to_le_bytes());
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
            search: None,
            source: None,
            q3_image: None,
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
            phase,
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
        host.call_module_export(
            ModuleId(1),
            Export::clocked(CallbackId(0)),
            ThinkTime::Milliseconds(50),
        )
        .unwrap();
        assert_eq!(
            host.module_counts(ModuleId(1)).unwrap().last_result,
            Some(ModuleResult::Qvm(2.0f32.to_bits() as i32))
        );
        assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 0);
    }
}
