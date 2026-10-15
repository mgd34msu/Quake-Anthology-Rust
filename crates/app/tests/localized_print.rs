#[path = "support/scalar_import.rs"]
mod scalar_import;
use qa_app::Runtime;
use qa_compat::{
    abi::{Addresses, Invocation, Q2_RERELEASE, UnknownCalls},
    memory::ModuleMemory,
    services::{CallContext, CallError, ServiceStorage},
};
use qa_console::{commands::Console, views::Context};
use qa_core::{
    events::{FrameEvent, OutputRecord, OutputSubmission, TextStore, dispatch_consumer},
    primitives::{ModuleId, PrintKind, RuleSetId, ThinkTime},
    sys_events::EventTime,
};
use qa_platform::native::NativeAbi;
use qa_world::{area::LinkOrder, entities::AllocationPolicy};

#[test]
fn localized_native_prints_share_ring_routing_and_checked_argument_memory() {
    for abi in [NativeAbi::SystemV, NativeAbi::Microsoft] {
        let mut runtime = Runtime::load(1, []).unwrap();
        runtime.localization = qa_content::localization::Catalog::load(
            br#"hello = "Welcome {}, raw!" name = "Ranger""#,
        )
        .unwrap();
        let mut console = Console::new(Context::default()).unwrap();
        let mut storage = ServiceStorage::load(&[], 0, &console.cvars).unwrap();
        let mut scratch = runtime.geometry.scratch();
        let base = 1 << 40;
        let mut memory = ModuleMemory::load(base, 256, &[]).unwrap();
        memory.write_string(base + 16, 32, b"$hello").unwrap();
        memory.write_string(base + 64, 32, b"$name").unwrap();
        memory.write_u64(base + 96, base + 64).unwrap();
        let mut unknown = UnknownCalls::load(1).unwrap();
        for (flags, kind, level, valid) in [
            (1, PrintKind::Console, 1, true),
            (8 | 1, PrintKind::Notify, 1, true),
            (8 | 4, PrintKind::Chat, 3, true),
            (8 | 6 | 16, PrintKind::Console, 6, true),
            (8 | 1, PrintKind::Notify, 1, false),
        ] {
            let args = [0, flags, base + 16, base + if valid { 96 } else { 255 }, 1];
            let result = {
                let mut services =
                    runtime.engine_services(&mut console, &mut storage, &mut scratch);
                Q2_RERELEASE
                    .invoke(
                        53,
                        &mut Invocation {
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
                            context: CallContext {
                                native_rules: RuleSetId::Quake2Rerelease,
                                trace_rules: RuleSetId::Quake2Rerelease,
                                sound_policy: qa_gameplay::rules::sound_policy(
                                    RuleSetId::Quake2Rerelease,
                                ),
                                server_frame: 0,
                                module: ModuleId(1),
                                clock: ThinkTime::Milliseconds(0),
                                console: Context {
                                    source: RuleSetId::Quake2Rerelease,
                                    ..Context::default()
                                },
                                allocation: AllocationPolicy::EDICT,
                                link_order: LinkOrder::Head,
                            },
                            platform_time: EventTime(0),
                            command: &[],
                            addresses: Addresses::Native {
                                pointer_bytes: 8,
                                abi,
                            },
                            arguments: &args,
                        },
                        &mut unknown,
                    )
                    .map(scalar_import::word)
            };
            if !valid {
                assert_eq!(result, Err(CallError::Memory));
                assert!(runtime.server.events.is_empty());
                continue;
            }
            assert_eq!(result, Ok(0));
            let mut seen = 0;
            dispatch_consumer(
                &mut runtime.server.events,
                runtime.server.presentation,
                &mut |record: OutputRecord, texts: &mut TextStore| {
                    let FrameEvent::Print(print) = record.event else {
                        panic!("expected print")
                    };
                    assert_eq!((print.kind, print.level, print.client), (kind, level, None));
                    assert_eq!(texts.get(print.text), Some(&b"Welcome Ranger, raw!"[..]));
                    seen += 1;
                    OutputSubmission::BestEffort
                },
            );
            assert_eq!(seen, 1);
        }
        assert_eq!(unknown.calls, 0);
    }
}
