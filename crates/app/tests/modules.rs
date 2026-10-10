#[path = "../../compat/tests/support/quakec_program.rs"]
mod quakec_program;
#[path = "support/service_program.rs"]
mod service_program;

use qa_app::{
    Runtime,
    host::{FrameHost, FrameSource},
    modules::{Export, ModuleRequest, ModuleResult, Phase, Program},
};
use qa_compat::{
    abi::Q3_SERVER,
    quakec::{Layout, Vm},
    services::CallContext,
};
use qa_console::{commands::Console, views::Context};
use qa_core::{events::FrameEvent, primitives::*, sys_events::*};
use qa_formats::program::quakec::{Image, Opcode::*};
use qa_session::{dispatch::CallError, timing::TickRate};
use qa_world::{area::LinkOrder, entities::AllocationPolicy};
use std::time::Duration;

#[test]
fn quakec_presets_are_explicit_and_do_not_inherit_map_or_player_rules() {
    use qa_app::modules::QuakeCSpec;
    let spec = QuakeCSpec::parse("qw:qwprogs.dat").unwrap();
    assert_eq!(spec.rules, RuleSetId::QuakeWorld);
    assert_eq!(spec.path, "qwprogs.dat");
    for input in ["progs.dat", "q3:progs.dat", "q1:", "other:progs.dat"] {
        assert!(QuakeCSpec::parse(input).is_err());
    }
}

#[test]
fn qvm_game_selection_does_not_schedule_client_exports_as_server_ticks() {
    use qa_app::modules::{Q3Role, Q3Spec};
    assert_eq!(
        Q3Spec::parse("game:vm/qagame.qvm").unwrap().path,
        "vm/qagame.qvm"
    );
    assert_eq!(Q3Spec::parse("ui:vm/ui.qvm").unwrap().role, Q3Role::Ui);
    assert_eq!(
        Q3Spec::parse("cgame:1:vm/cgame.qvm").unwrap().role,
        Q3Role::Cgame(SeatId::new(1).unwrap())
    );
    for input in [
        "vm/qagame.qvm",
        "game:",
        "client:vm/cgame.qvm",
        "cgame:4:file",
        "cgame:0:",
        "ui:",
    ] {
        assert!(Q3Spec::parse(input).is_err());
    }
}

fn lifecycle_host(budget: u64) -> FrameHost {
    lifecycle_host_in(budget, Phase::Server, None)
}
fn lifecycle_host_in(
    budget: u64,
    phase: Phase,
    api: Option<qa_app::modules::ApiCheck>,
) -> FrameHost {
    use qa_app::modules::{Argument, Export};
    use qa_formats::program::qvm::Opcode::*;
    let mut runtime = Runtime::load(1, std::iter::empty()).unwrap();
    let id = runtime
        .server
        .entities
        .allocate(
            ThinkTime::Milliseconds(0),
            ModuleId(1),
            AllocationPolicy::q3(0),
        )
        .unwrap()
        .id;
    // Return 1000*command + arg1 + arg2 + arg3, observing vmMain's stack ABI.
    let mut operations = vec![
        (Enter, 64),
        (Local, 72),
        (Load4, 0),
        (Const, 1000),
        (MulI, 0),
        (Local, 76),
        (Load4, 0),
        (Add, 0),
        (Local, 80),
        (Load4, 0),
        (Add, 0),
        (Local, 84),
        (Load4, 0),
        (Add, 0),
        (Leave, 64),
    ];
    let (initialize, frame, shutdown) = if api.is_some() {
        let body = operations.drain(1..).collect::<Vec<_>>();
        operations.extend([(Local, 72), (Load4, 0), (Const, 0), (Eq, 19)]);
        operations.extend(body);
        operations.extend([(Const, 6), (Leave, 64)]);
        (1, 5, 2)
    } else {
        (0, if phase == Phase::Client { 3 } else { 8 }, 1)
    };
    let vm = service_program::program(&operations, &[0; 16]);
    FrameHost::load_modules(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        vec![ModuleRequest {
            context: CallContext {
                module: ModuleId(1),
                clock: ThinkTime::Milliseconds(0),
                console: Context::default(),
                allocation: AllocationPolicy::q3(0),
                link_order: LinkOrder::Head,
            },
            timing_rules: RuleSetId::Quake3,
            phase,
            api,
            rate: TickRate::fixed(50).unwrap(),
            anchor: id,
            program: Program::Qvm {
                vm: Box::new(vm),
                imports: &Q3_SERVER,
            },
            entries: (0..=10).collect(),
            frame: Export::clocked(CallbackId(frame)),
            prepare: Vec::new(),
            initialize: Some(Export {
                callback: CallbackId(initialize),
                arguments: [
                    Argument::ClockMilliseconds,
                    Argument::PlatformMilliseconds,
                    Argument::Word(1),
                    Argument::Word(0),
                    Argument::Word(0),
                    Argument::Word(0),
                    Argument::Word(0),
                    Argument::Word(0),
                    Argument::Word(0),
                ],
            }),
            shutdown: vec![Export {
                callback: CallbackId(shutdown),
                arguments: [
                    Argument::Word(1),
                    Argument::Word(0),
                    Argument::Word(0),
                    Argument::Word(0),
                    Argument::Word(0),
                    Argument::Word(0),
                    Argument::Word(0),
                    Argument::Word(0),
                    Argument::Word(0),
                ],
            }],
            instruction_budget: budget,
            configstrings: 0,
            files: 0,
        }],
    )
    .unwrap()
}

#[test]
fn lifecycle_is_once_only_uses_native_exports_and_keeps_two_physical_intakes() {
    use qa_app::modules::State;
    let mut host = lifecycle_host(100);
    let mut source = Source {
        time: EventTime(123_000_000),
        polls: 0,
    };
    assert_eq!(host.module_state(ModuleId(1)), Some(State::Pending));
    host.frame(&mut source, true);
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, 1);
    assert_eq!(
        host.module_counts(ModuleId(1)).unwrap().last_result,
        Some(ModuleResult::Qvm(247))
    );
    assert_eq!(source.polls, 2);
    host.initialize_modules(Phase::Server);
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, 1);
    source.time = EventTime(173_000_000);
    host.frame(&mut source, true);
    assert_eq!(
        host.module_counts(ModuleId(1)).unwrap().last_result,
        Some(ModuleResult::Qvm(8173))
    );
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, 2);
    host.shutdown_modules();
    assert_eq!(host.module_state(ModuleId(1)), Some(State::Stopped));
    assert_eq!(
        host.module_counts(ModuleId(1)).unwrap().last_result,
        Some(ModuleResult::Qvm(1001))
    );
    host.shutdown_modules();
    source.time = EventTime(223_000_000);
    host.frame(&mut source, true);
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, 3);
    assert_eq!(source.polls, 6);
}

#[test]
fn failed_initialization_is_not_retried_or_advanced_as_a_running_module() {
    use qa_app::modules::State;
    let mut host = lifecycle_host(1);
    let mut source = Source {
        time: EventTime(0),
        polls: 0,
    };
    host.frame(&mut source, true);
    assert_eq!(host.module_state(ModuleId(1)), Some(State::Failed));
    source.time = EventTime(150_000_000);
    let result = host.frame(&mut source, true);
    assert!(result.server_ticks >= 3);
    let counts = host.module_counts(ModuleId(1)).unwrap();
    assert_eq!((counts.calls, counts.traps, counts.rejected), (1, 1, 1));
    host.shutdown_modules();
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, 1);
    assert_eq!(source.polls, 4);
}

#[test]
fn client_exports_run_once_per_client_frame_instead_of_catching_up_server_ticks() {
    let mut host = lifecycle_host_in(100, Phase::Client, None);
    let mut source = Source {
        time: EventTime(0),
        polls: 0,
    };
    host.frame(&mut source, true);
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, 2);
    source.time = EventTime(150_000_000);
    let frame = host.frame(&mut source, true);
    assert_eq!(frame.server_ticks, 1);
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, 3);
    assert_eq!(
        host.module_counts(ModuleId(1)).unwrap().last_result,
        Some(ModuleResult::Qvm(3150))
    );
    assert_eq!(source.polls, 4);
}

#[test]
fn ui_api_version_is_checked_before_initialization_and_failure_stops_client_calls() {
    use qa_app::modules::{ApiCheck, Argument, Export, State};
    for (accepted, running, expected_calls) in [(&[4, 6][..], true, 3), (&[4][..], false, 1)] {
        let api = ApiCheck::Version {
            export: Export {
                callback: CallbackId(0),
                arguments: [Argument::Word(0); 9],
            },
            accepted,
        };
        let mut host = lifecycle_host_in(100, Phase::Client, Some(api));
        let mut source = Source {
            time: EventTime(0),
            polls: 0,
        };
        host.frame(&mut source, true);
        assert_eq!(
            host.module_counts(ModuleId(1)).unwrap().calls,
            expected_calls
        );
        assert_eq!(
            host.module_state(ModuleId(1)),
            Some(if running {
                State::Running
            } else {
                State::Failed
            })
        );
        source.time = EventTime(250_000_000);
        host.frame(&mut source, true);
        assert_eq!(
            host.module_counts(ModuleId(1)).unwrap().calls,
            expected_calls + u64::from(running)
        );
        assert_eq!(source.polls, 4);
    }
}

#[test]
fn quit_in_second_command_phase_prevents_client_module_startup() {
    use qa_app::modules::State;
    struct QuitAtSecond(Source);
    impl FrameSource for QuitAtSecond {
        fn begin_frame(&mut self) -> EventTime {
            self.0.begin_frame()
        }
        fn poll_events(&mut self, queue: &mut SysEventQueue) {
            self.0.poll_events(queue);
            if self.0.polls == 2 {
                queue
                    .push(SysEvent {
                        time: self.0.time,
                        kind: EventKind::ConsoleLine("quit"),
                    })
                    .unwrap();
            }
        }
        fn wait_time(&mut self, duration: Duration) -> EventTime {
            self.0.wait_time(duration)
        }
        fn elapsed(&self) -> Duration {
            self.0.elapsed()
        }
        fn present(&mut self) {
            self.0.present();
        }
    }
    let mut host = lifecycle_host_in(100, Phase::Client, None);
    let mut source = QuitAtSecond(Source {
        time: EventTime(0),
        polls: 0,
    });
    host.frame(&mut source, true);
    assert!(host.runtime.quit);
    assert_eq!(source.0.polls, 2);
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, 0);
    assert_eq!(host.module_state(ModuleId(1)), Some(State::Pending));
}

#[test]
fn export_words_narrow_only_at_the_qvm_boundary_and_owner_changes_are_rejected() {
    use qa_app::modules::{Argument, Export};
    let mut host = lifecycle_host(100);
    let mut arguments = [Argument::Word(0); 9];
    arguments[0] = Argument::Word(u64::MAX);
    arguments[1] = Argument::Word(0x1_8000_0000);
    arguments[2] = Argument::Word(0x1_0000_0007);
    let export = Export {
        callback: CallbackId(2),
        arguments,
    };
    assert_eq!(
        host.call_module_export(ModuleId(1), export, ThinkTime::Milliseconds(-123)),
        Ok(())
    );
    assert_eq!(
        host.module_counts(ModuleId(1)).unwrap().last_result,
        Some(ModuleResult::Qvm(i32::MIN.wrapping_add(2006)))
    );
    let slot = host
        .runtime
        .server
        .entities
        .next_active(0)
        .into_iter()
        .flat_map(|first| {
            std::iter::successors(Some(first), |id| {
                host.runtime
                    .server
                    .entities
                    .next_active(id.slot as usize + 1)
            })
        })
        .map(|id| id.slot as usize)
        .find(|&slot| host.runtime.server.entities.columns.owner[slot] == ModuleId(1))
        .unwrap();
    host.runtime.server.entities.columns.owner[slot] = ModuleId(2);
    assert_eq!(
        host.call_module_export(ModuleId(1), export, ThinkTime::Milliseconds(0)),
        Err(CallError::MissingModule)
    );
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, 1);
}

pub struct Source {
    pub time: EventTime,
    pub polls: u64,
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
pub fn host(qvm_budget: u64, developer: bool) -> (FrameHost, [EntityId; 2]) {
    let mut runtime = Runtime::load(1, std::iter::empty()).unwrap();
    let mut console = Console::new(Context::default()).unwrap();
    if developer {
        console.append("developer 1\n", Context::default()).unwrap();
        console.execute_frame(&mut runtime);
    }
    let ids = [ModuleId(1), ModuleId(2)].map(|id| {
        runtime
            .server
            .entities
            .allocate(ThinkTime::Seconds(0.0), id, AllocationPolicy::EDICT)
            .unwrap()
            .id
    });
    runtime.server.entities.columns.native_entity[ids[1].slot as usize] = Some(NativeEntity {
        module: ModuleId(2),
        slot: 3,
    });
    let bytes = quakec_program::program(
        &[
            (StoreF, 40, 4, 0),
            (Call1, 32, 0, 0),
            (StoreF, 1, 36, 0),
            (StoreF, 34, 37, 0),
            (StoreF, 35, 38, 0),
            (StoreS, 30, 4, 0),
            (StoreS, 31, 7, 0),
            (Call2, 33, 0, 0),
            (StoreS, 29, 4, 0),
            (Call1, 28, 0, 0),
            (Return, 36, 0, 0),
            (Call0, 27, 0, 0),
            (Return, 36, 0, 0),
        ],
        &[0, 1, -37, -72, -25, 12, -999],
    );
    let mut qc = Vm::load(
        Image::parse(&bytes, Some(5927)).unwrap(),
        Layout {
            entities: 4,
            header_bytes: 16,
            extra_string_bytes: 128,
            state_step: 0.1,
        },
    )
    .unwrap();
    for (global, text) in [(30, &b"fov"[..]), (31, b"108"), (29, b"\x82qc\n")] {
        qc.image.globals[global] = qc.insert_string(text).unwrap() as u32;
    }
    for (global, value) in [
        (32, 2),
        (33, 3),
        (28, 4),
        (27, 6),
        (40, (-1.25f32).to_bits()),
    ] {
        qc.image.globals[global] = value;
    }
    let requests = vec![
        ModuleRequest {
            context: CallContext {
                module: ModuleId(1),
                clock: ThinkTime::Milliseconds(0),
                console: Context::default(),
                allocation: AllocationPolicy::EDICT,
                link_order: LinkOrder::Head,
            },
            timing_rules: RuleSetId::Quake3,
            phase: Phase::Server,
            api: None,
            rate: TickRate::fixed(50).unwrap(),
            anchor: ids[0],
            program: Program::Qvm {
                vm: Box::new(service_program::qvm()),
                imports: &Q3_SERVER,
            },
            entries: vec![8],
            frame: Export::clocked(CallbackId(0)),
            prepare: Vec::new(),
            initialize: None,
            shutdown: Vec::new(),
            instruction_budget: qvm_budget,
            configstrings: 0,
            files: 0,
        },
        ModuleRequest {
            context: CallContext {
                module: ModuleId(2),
                clock: ThinkTime::Seconds(0.0),
                console: Context {
                    source: RuleSetId::Quake,
                    ..Context::default()
                },
                allocation: AllocationPolicy::EDICT,
                link_order: LinkOrder::Tail,
            },
            timing_rules: RuleSetId::Quake,
            phase: Phase::Server,
            api: None,
            rate: TickRate::fixed(100).unwrap(),
            anchor: ids[1],
            program: Program::quakec(qc),
            entries: vec![1, 5],
            frame: Export::clocked(CallbackId(0)),
            prepare: Vec::new(),
            initialize: None,
            shutdown: Vec::new(),
            instruction_budget: 1000,
            configstrings: 0,
            files: 0,
        },
    ];
    (
        FrameHost::load_modules(console, runtime, TickRate::FrameDriven, requests).unwrap(),
        ids,
    )
}
#[test]
fn module_entries_share_services_keep_native_namespace_and_preserve_raw_print_bytes() {
    let (mut host, ids) = host(1000, true);
    assert_eq!(
        host.call_module(
            CallbackId(0),
            CallbackCall::Think {
                entity: ids[0],
                time: ThinkTime::Milliseconds(50)
            }
        ),
        Ok(())
    );
    assert_eq!(
        host.call_module(
            CallbackId(0),
            CallbackCall::Think {
                entity: ids[1],
                time: ThinkTime::Seconds(0.1)
            }
        ),
        Ok(())
    );
    assert_eq!(
        host.module_counts(ModuleId(1)).unwrap().last_result,
        Some(ModuleResult::Qvm(105))
    );
    assert_eq!(
        host.module_counts(ModuleId(2)).unwrap().last_result,
        Some(ModuleResult::QuakeC([
            (-2.0f32).to_bits(),
            144,
            0.1f32.to_bits()
        ]))
    );
    let mut batch = host
        .runtime
        .server
        .events
        .batch(host.runtime.server.presentation)
        .unwrap();
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
    assert!(prints.contains(&b"module print\n".to_vec()));
    assert!(prints.contains(&b"\x82qc\n".to_vec()));
    assert_eq!(
        host.console
            .cvars
            .value_in(host.console.cvars.find("cg_fov").unwrap(), RuleSetId::Quake),
        108.0
    );
}
#[test]
fn host_ticks_both_program_types_at_independent_rates_and_drains_the_shared_commands() {
    let (mut host, _) = host(1000, true);
    let mut source = Source {
        time: EventTime(0),
        polls: 0,
    };
    host.frame(&mut source, true);
    source.time = EventTime(150_000_000);
    host.frame(&mut source, true);
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, 3);
    assert_eq!(host.module_counts(ModuleId(2)).unwrap().calls, 1);
    assert_eq!(
        host.module_counts(ModuleId(2)).unwrap().last_result,
        Some(ModuleResult::QuakeC([
            (-2.0f32).to_bits(),
            144,
            0.1f32.to_bits()
        ]))
    );
    assert_eq!(
        host.console
            .cvars
            .numeric(
                host.console
                    .cvars
                    .bind("sensitivity", Context::default())
                    .unwrap()
            )
            .unwrap(),
        7.0
    );
    assert_eq!(source.polls, 4);
}
#[test]
fn an_executed_unsupported_quakec_builtin_traps_without_poisoning_the_next_entry() {
    let (mut host, ids) = host(1000, false);
    let call = CallbackCall::Think {
        entity: ids[1],
        time: ThinkTime::Seconds(0.2),
    };
    assert_eq!(
        host.call_module(CallbackId(1), call),
        Err(CallError::Rejected)
    );
    assert_eq!(host.module_counts(ModuleId(2)).unwrap().traps, 1);
    assert_eq!(host.call_module(CallbackId(0), call), Ok(()));
    let mut batch = host
        .runtime
        .server
        .events
        .batch(host.runtime.server.presentation)
        .unwrap();
    while let Some(record) = host.runtime.server.events.next(&mut batch) {
        if let FrameEvent::Print(p) = record.event {
            assert_ne!(
                host.runtime.server.events.texts.get(p.text).unwrap(),
                b"\x82qc\n"
            );
        }
    }
}
#[test]
fn a_budget_trap_and_a_stale_anchor_do_not_stop_other_providers() {
    let (mut host, ids) = host(1, true);
    let mut source = Source {
        time: EventTime(0),
        polls: 0,
    };
    host.frame(&mut source, true);
    source.time = EventTime(150_000_000);
    host.frame(&mut source, true);
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 3);
    assert_eq!(host.module_counts(ModuleId(2)).unwrap().calls, 1);
    assert!(
        host.runtime
            .server
            .entities
            .release(ids[0], ThinkTime::Milliseconds(150))
    );
    source.time = EventTime(250_000_000);
    host.frame(&mut source, true);
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, 3);
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().rejected, 5);
    assert_eq!(host.module_counts(ModuleId(2)).unwrap().calls, 2);
}

#[test]
fn a_provider_anchor_transferred_to_another_module_cannot_call_its_new_owner() {
    let (mut host, ids) = host(1000, false);
    let mut source = Source {
        time: EventTime(0),
        polls: 0,
    };
    host.frame(&mut source, true);
    host.runtime.server.entities.columns.owner[ids[0].slot as usize] = ModuleId(2);
    source.time = EventTime(150_000_000);
    host.frame(&mut source, true);
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, 0);
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().rejected, 3);
    assert_eq!(host.module_counts(ModuleId(2)).unwrap().calls, 1);
}
