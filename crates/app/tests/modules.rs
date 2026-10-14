#[path = "modules/q3_clients.rs"]
mod q3_clients;
#[path = "modules/q3_snapshot_metadata.rs"]
mod q3_snapshot_metadata;
#[path = "modules/quakec_map.rs"]
mod quakec_map;
#[path = "../../compat/tests/support/quakec_program.rs"]
mod quakec_program;
#[path = "modules/quakec_thinks.rs"]
mod quakec_thinks;
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
fn qvm_spawn_tokens_use_session_dispatch_and_explicit_module_syntax() {
    use qa_app::map::NativeEntityText;
    use qa_formats::{entities::EntitySyntax, program::qvm::Opcode::*};
    let mut runtime = Runtime::load(1, std::iter::empty()).unwrap();
    runtime.entity_sources.push(NativeEntityText {
        syntax: EntitySyntax::Quake,
        bytes: b"{ a:b raw }\0ignored".as_slice().into(),
    });
    let operations = [
        (Enter, 64),
        (Const, 128),
        (Arg, 8),
        (Const, 64),
        (Arg, 12),
        (Const, -38),
        (Call, 0),
        (Const, 128),
        (Arg, 8),
        (Const, -1),
        (Call, 0),
        (Pop, 0),
        (Leave, 64),
    ];
    let requests = (1..=2)
        .map(|module| {
            let module = ModuleId(module);
            let anchor = runtime
                .server
                .entities
                .allocate(ThinkTime::Milliseconds(0), module, AllocationPolicy::q3(0))
                .unwrap()
                .id;
            ModuleRequest {
                context: CallContext {
                    native_rules: qa_core::primitives::RuleSetId::Quake3,
                    trace_rules: qa_core::primitives::RuleSetId::Quake3,
                    sound_policy: qa_gameplay::rules::sound_policy(
                        qa_core::primitives::RuleSetId::Quake3,
                    ),
                    server_frame: 0,
                    module,
                    clock: ThinkTime::Milliseconds(0),
                    console: Context::default(),
                    allocation: AllocationPolicy::q3(0),
                    link_order: LinkOrder::Head,
                },
                timing_rules: RuleSetId::Quake3,
                phase: Phase::Server,
                rate: TickRate::fixed(50).unwrap(),
                anchor,
                program: Program::Qvm {
                    vm: Box::new(service_program::program(&operations, &[])),
                    imports: &Q3_SERVER,
                },
                entries: vec![0],
                frame: Export::clocked(CallbackId(0)),
                prepare: Vec::new(),
                initialize: Vec::new(),
                api: None,
                shutdown: Vec::new(),
                instruction_budget: 100,
                configstrings: 0,
                files: 0,
                entity_source: Some((0, EntitySyntax::Quake3)),
            }
        })
        .collect();
    let mut host = FrameHost::load_modules(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        requests,
    )
    .unwrap();
    for expected in [1, 1, 1, 1, 0, 0] {
        for module in [ModuleId(1), ModuleId(2)] {
            host.call_module_export(
                module,
                Export::clocked(CallbackId(0)),
                ThinkTime::Milliseconds(50),
            )
            .unwrap();
            assert_eq!(
                host.module_counts(module).unwrap().last_result,
                Some(ModuleResult::Qvm(expected))
            );
        }
    }
    let mut batch = host
        .runtime
        .server
        .events
        .batch(host.runtime.server.presentation)
        .unwrap();
    let mut prints = Vec::new();
    while let Some(record) = host.runtime.server.events.next(&mut batch) {
        if let FrameEvent::Print(print) = record.event {
            prints.push(
                host.runtime
                    .server
                    .events
                    .texts
                    .get(print.text)
                    .unwrap()
                    .to_vec(),
            );
        }
    }
    let expected = [b"{".as_slice(), b"a:b", b"raw", b"}", b"", b""]
        .into_iter()
        .flat_map(|token| [token.to_vec(), token.to_vec()])
        .collect::<Vec<_>>();
    assert_eq!(prints, expected);
}

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
fn q2_native_presets_do_not_inherit_map_or_player_rules() {
    use qa_app::modules::Q2Spec;
    for (input, rules, path) in [
        ("q2:gamex86.so", RuleSetId::Quake2, "gamex86.so"),
        (
            "q2rr:game_x64.dll",
            RuleSetId::Quake2Rerelease,
            "game_x64.dll",
        ),
    ] {
        let spec = Q2Spec::parse(input).unwrap();
        assert_eq!(spec.rules, rules);
        assert_eq!(spec.path, path);
    }
    for input in ["gamex86.so", "q1:game.dll", "q3:game.dll", "q2:", "q2rr:"] {
        assert!(Q2Spec::parse(input).is_err());
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
        Q3Spec::parse("game:vm/qagame.qvm").unwrap().player_stats,
        Q3StatLayout::BASE
    );
    assert_eq!(
        Q3Spec::parse("game-ta:vm/qagame.qvm").unwrap().player_stats,
        Q3StatLayout::TEAM_ARENA
    );
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
        "cgame-ta:1:vm/cgame.qvm",
        "ui-ta:vm/ui.qvm",
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
    lifecycle_host_for_modules(budget, phase, api, &[ModuleId(1)])
}
fn lifecycle_host_for_modules(
    budget: u64,
    phase: Phase,
    api: Option<qa_app::modules::ApiCheck>,
    modules: &[ModuleId],
) -> FrameHost {
    use qa_app::modules::{Argument, Export};
    use qa_formats::program::qvm::Opcode::*;
    let mut runtime = Runtime::load(1, std::iter::empty()).unwrap();
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
    let requests = modules
        .iter()
        .map(|&module| {
            let id = runtime
                .server
                .entities
                .allocate(ThinkTime::Milliseconds(0), module, AllocationPolicy::q3(0))
                .unwrap()
                .id;
            let vm = service_program::program(&operations, &[0; 16]);
            ModuleRequest {
                context: CallContext {
                    native_rules: qa_core::primitives::RuleSetId::Quake3,
                    trace_rules: qa_core::primitives::RuleSetId::Quake3,
                    sound_policy: qa_gameplay::rules::sound_policy(
                        qa_core::primitives::RuleSetId::Quake3,
                    ),
                    server_frame: 0,
                    module,
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
                initialize: vec![Export {
                    client: None,
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
                }],
                shutdown: vec![Export {
                    client: None,
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
                entity_source: None,
            }
        })
        .collect();
    FrameHost::load_modules(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        requests,
    )
    .unwrap()
}

#[test]
fn stopping_unobserved_server_module_preserves_independent_client_and_provider() {
    use qa_app::modules::State;
    use qa_session::clients::Connection;
    let mut host =
        lifecycle_host_for_modules(100, Phase::Server, None, &[ModuleId(1), ModuleId(2)]);
    host.initialize_modules(Phase::Server);
    let client = host
        .runtime
        .server
        .connect(Connection::Remote, ModuleId(2), None)
        .unwrap();
    let before = &host.runtime.server.clients[client.0 as usize];
    let lifetime = before.entity;
    let consumer = before.output.unwrap();
    let owned = linked_owned_entity(&mut host, ModuleId(1));
    let independent = linked_owned_entity(&mut host, ModuleId(2));
    assert!(host.runtime.server.entities.set_never_free(owned, true));
    assert!(
        !host
            .runtime
            .server
            .free_entity(owned, ThinkTime::Milliseconds(0))
    );
    let calls = host.module_counts(ModuleId(2)).unwrap().calls;
    assert!(host.shutdown_module(ModuleId(1)));
    assert!(host.runtime.server.entities.resolve(owned).is_none());
    assert!(host.runtime.server.area.bounds(owned).is_none());
    assert!(!host.runtime.server.navigation.contains(owned));
    assert!(host.runtime.server.entities.resolve(independent).is_some());
    assert!(host.runtime.server.area.bounds(independent).is_some());
    assert!(host.runtime.server.navigation.contains(independent));
    let after = &host.runtime.server.clients[client.0 as usize];
    assert_eq!(after.connection, Some(Connection::Remote));
    assert_eq!(after.entity, lifetime);
    assert_eq!(after.output, Some(consumer));
    assert!(after.audio_retirement.is_none());
    assert!(host.runtime.server.entities.resolve(lifetime).is_some());
    assert!(host.runtime.server.events.batch(consumer).is_some());
    assert_eq!(host.module_state(ModuleId(1)), Some(State::Stopped));
    assert_eq!(host.module_state(ModuleId(2)), Some(State::Running));
    host.frame(
        &mut Source {
            time: EventTime(50_000_000),
            polls: 0,
        },
        true,
    );
    host.frame(
        &mut Source {
            time: EventTime(100_000_000),
            polls: 0,
        },
        true,
    );
    assert!(host.module_counts(ModuleId(2)).unwrap().calls > calls);
    assert_eq!(
        host.runtime.server.clients[client.0 as usize].connection,
        Some(Connection::Remote)
    );
    // The owning module still retires its own clients.
    assert!(host.shutdown_module(ModuleId(2)));
    assert!(
        host.runtime.server.clients[client.0 as usize]
            .connection
            .is_none()
    );
}

fn linked_owned_entity(host: &mut FrameHost, module: ModuleId) -> EntityId {
    let server = &mut host.runtime.server;
    let entity = server
        .entities
        .allocate(ThinkTime::Milliseconds(0), module, AllocationPolicy::q3(0))
        .unwrap()
        .id;
    server.entities.columns.mins[entity.slot as usize] = Vec3([-1.; 3]);
    server.entities.columns.maxs[entity.slot as usize] = Vec3([1.; 3]);
    assert!(server.area.link(
        &server.entities,
        entity,
        qa_world::area::LinkFlags::SOLID,
        LinkOrder::Head,
        qa_world::area::LinkIntent::Explicit,
    ));
    assert!(server.navigation.register(entity));
    entity
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
    let entity = linked_owned_entity(&mut host, ModuleId(1));
    let mut source = Source {
        time: EventTime(0),
        polls: 0,
    };
    host.frame(&mut source, true);
    assert_eq!(host.module_state(ModuleId(1)), Some(State::Failed));
    assert!(host.runtime.server.entities.resolve(entity).is_none());
    assert!(host.runtime.server.area.bounds(entity).is_none());
    assert!(!host.runtime.server.navigation.contains(entity));
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
                client: None,
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
        fn begin_frame(&mut self) -> Duration {
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
        fn wait_time(&mut self, duration: Duration) -> Duration {
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
        client: None,
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
    fn begin_frame(&mut self) -> Duration {
        Duration::from_nanos(self.time.0)
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
    fn wait_time(&mut self, _: Duration) -> Duration {
        Duration::from_nanos(self.time.0)
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
            entities: 64,
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
                native_rules: qa_core::primitives::RuleSetId::Quake3,
                trace_rules: qa_core::primitives::RuleSetId::Quake3,
                sound_policy: qa_gameplay::rules::sound_policy(
                    qa_core::primitives::RuleSetId::Quake3,
                ),
                server_frame: 0,
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
            initialize: Vec::new(),
            shutdown: Vec::new(),
            instruction_budget: qvm_budget,
            configstrings: 0,
            files: 0,
            entity_source: None,
        },
        ModuleRequest {
            context: CallContext {
                native_rules: RuleSetId::Quake,
                trace_rules: RuleSetId::Quake,
                sound_policy: qa_gameplay::rules::sound_policy(RuleSetId::Quake),
                server_frame: 0,
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
            program: Program::quakec(qc, None),
            entries: vec![1, 5],
            frame: Export::clocked(CallbackId(0)),
            prepare: Vec::new(),
            initialize: Vec::new(),
            shutdown: Vec::new(),
            instruction_budget: 1000,
            configstrings: 0,
            files: 0,
            entity_source: None,
        },
    ];
    (
        FrameHost::load_modules(console, runtime, TickRate::FrameDriven, requests).unwrap(),
        ids,
    )
}

fn quakec_output_host(rules: RuleSetId, level: f32, texts: [&[u8]; 2]) -> FrameHost {
    let mut operations = vec![(StoreF, 54, 1, 0), (StoreF, 55, 2, 0), (StoreF, 56, 3, 0)];
    let parameter = if rules == RuleSetId::QuakeWorld {
        operations.push((StoreF, 53, 4, 0));
        7
    } else {
        4
    };
    operations.extend([
        (StoreS, 48, parameter, 0),
        (StoreS, 49, parameter + 3, 0),
        (
            if rules == RuleSetId::QuakeWorld {
                Call3
            } else {
                Call2
            },
            27,
            0,
            0,
        ),
        (StoreS, 50, 4, 0),
        (Call1, 28, 0, 0),
        (StoreS, 51, 4, 0),
        (Call1, 28, 0, 0),
        (Return, 1, 0, 0),
    ]);
    let bytes = quakec_program::program(&operations, &[0, 1, -23, -46]);
    let mut vm = Vm::load(
        Image::parse(&bytes, Some(5927)).unwrap(),
        Layout {
            entities: 64,
            header_bytes: 16,
            extra_string_bytes: 4096,
            state_step: 0.1,
        },
    )
    .unwrap();
    for (global, text) in [
        (48, texts[0]),
        (49, texts[1]),
        (50, b"fov "),
        (51, b"111\n"),
    ] {
        vm.image.globals[global] = vm.insert_string(text).unwrap() as u32;
    }
    vm.image.globals[27] = 2;
    vm.image.globals[28] = 3;
    vm.image.globals[53] = level.to_bits();
    vm.image.globals[54..57].copy_from_slice(&[0x12345678, 0x87654321, 0x11223344]);
    quakec_host(rules, vm)
}

fn quakec_host(rules: RuleSetId, vm: Vm) -> FrameHost {
    quakec_runtime_host(rules, vm, Runtime::load(1, []).unwrap())
}

fn quakec_runtime_host(rules: RuleSetId, vm: Vm, mut runtime: Runtime) -> FrameHost {
    let module = ModuleId(1);
    let anchor = runtime
        .server
        .entities
        .allocate(ThinkTime::Seconds(0.0), module, AllocationPolicy::EDICT)
        .unwrap()
        .id;
    runtime.server.entities.columns.native_entity[anchor.slot as usize] =
        Some(NativeEntity { module, slot: 0 });
    FrameHost::load_modules(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        vec![ModuleRequest {
            context: CallContext {
                native_rules: rules,
                trace_rules: rules,
                sound_policy: qa_gameplay::rules::sound_policy(rules),
                server_frame: 0,
                module,
                clock: ThinkTime::module_clock(rules),
                console: Context {
                    source: rules,
                    ..Context::default()
                },
                allocation: AllocationPolicy::EDICT,
                link_order: qa_gameplay::rules::link_order(rules),
            },
            timing_rules: rules,
            phase: Phase::Server,
            rate: TickRate::fixed(100).unwrap(),
            anchor,
            program: Program::quakec(vm, None),
            entries: vec![1],
            frame: Export::clocked(CallbackId(0)),
            prepare: vec![],
            initialize: vec![],
            shutdown: vec![],
            api: None,
            instruction_budget: 1000,
            configstrings: 0,
            files: 0,
            entity_source: None,
        }],
    )
    .unwrap()
}

fn quakec_math_program(builtin: i32) -> Vm {
    let vectors: &[(u16, &[u8])] = if builtin == 1 {
        &[(52, b"v_forward"), (55, b"v_right"), (58, b"v_up")]
    } else {
        &[]
    };
    quakec_math_program_with_vectors(builtin, vectors)
}

fn quakec_math_program_with_vectors(builtin: i32, vectors: &[(u16, &[u8])]) -> Vm {
    let bytes = quakec_program::program_with_vectors(
        &[
            (StoreV, 48, 4, 0),
            (Call1, 27, 0, 0),
            (Return, 1, 0, 0),
            (Call0, 27, 0, 0),
            (Return, 1, 0, 0),
            (Call2, 27, 0, 0),
            (Return, 1, 0, 0),
        ],
        &[0, 1, -builtin, 4, 6],
        vectors,
    );
    let mut vm = Vm::load(
        Image::parse(&bytes, Some(5927)).unwrap(),
        Layout {
            entities: 64,
            header_bytes: 16,
            extra_string_bytes: 64,
            state_step: 0.1,
        },
    )
    .unwrap();
    vm.image.globals[27] = 2;
    // The second parameter is unrelated to the vector in the first parameter.
    vm.image.globals[7..10].fill(f32::MAX.to_bits());
    vm.image.globals[1..4].copy_from_slice(&[0x12345678, 0x87654321, 0x11223344]);
    vm
}

#[test]
fn quakec_pointcontents_uses_module_rules_over_all_collision_topologies() {
    use qa_app::WorldCollision;
    use qa_world::collision::{
        Contents,
        brushes::{Brush, BrushTree, CollisionLeaf, ModelRoot},
        hulls::HullModel,
        surfaces::SurfaceTable,
    };
    let plane = Plane::oriented(Vec3([1.0, 0.0, 0.0]), 0.0);
    let bounds = Bounds {
        mins: Vec3([-512.0; 3]),
        maxs: Vec3([512.0; 3]),
    };
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        for native in [-1, -2, -3, -4, -5, -6, -9, -10, -11, -12, -13, -14] {
            for topology in 0..3 {
                for x in [-1.0f32, -0.0, 0.0, 1.0] {
                    let mut runtime = Runtime::load(1, []).unwrap();
                    let geometry = if topology == 0 {
                        let node = ClipNode {
                            plane: 0,
                            children: [-1, native],
                        };
                        runtime
                            .geometry
                            .load_hulls(
                                vec![plane],
                                vec![node],
                                vec![node],
                                vec![HullModel { roots: [0; 3] }],
                                vec![bounds],
                            )
                            .unwrap()
                    } else {
                        let contents = Contents::from_q1(native);
                        runtime
                            .geometry
                            .load_brushes(
                                vec![plane],
                                vec![Brush {
                                    sides: qa_core::primitives::IndexSpan { first: 0, count: 1 },
                                    contents,
                                }],
                                SurfaceTable::flags(vec![SurfaceFlags::default()]),
                                BrushTree {
                                    planes: vec![plane],
                                    nodes: vec![ClipNode {
                                        plane: 0,
                                        children: [child_from_leaf(0), child_from_leaf(1)],
                                    }],
                                    leaves: vec![
                                        CollisionLeaf {
                                            stored_contents: (topology == 1)
                                                .then_some(Contents::EMPTY),
                                            brushes: qa_core::primitives::IndexSpan {
                                                first: 0,
                                                count: 0,
                                            },
                                        },
                                        CollisionLeaf {
                                            stored_contents: (topology == 1).then_some(contents),
                                            brushes: qa_core::primitives::IndexSpan {
                                                first: 0,
                                                count: 1,
                                            },
                                        },
                                    ],
                                    leaf_brushes: vec![0],
                                    models: vec![ModelRoot::Tree(0)],
                                },
                                vec![bounds],
                            )
                            .unwrap()
                    };
                    runtime.collision = Some(WorldCollision::new(&runtime.geometry, geometry, 0));
                    let mut vm = quakec_math_program(41);
                    vm.image.globals[48..51].copy_from_slice(&[x, 7.0, -9.0].map(f32::to_bits));
                    let mut host = quakec_runtime_host(rules, vm, runtime);
                    host.call_module_export(
                        ModuleId(1),
                        Export::clocked(CallbackId(0)),
                        ThinkTime::module_clock(rules),
                    )
                    .unwrap();
                    let expected = if x >= 0.0 {
                        -1
                    } else if native <= -9 {
                        -3
                    } else {
                        native
                    };
                    let counts = host.module_counts(ModuleId(1)).unwrap();
                    assert_eq!(
                        counts.last_result,
                        Some(ModuleResult::QuakeC([
                            (expected as f32).to_bits(),
                            0x87654321,
                            0x11223344
                        ])),
                        "{rules:?} native={native} topology={topology} x={x}"
                    );
                    assert_eq!(counts.traps, 0);
                    assert!(host.runtime.server.events.is_empty());
                }
            }
        }
    }
}

#[test]
fn quakec_numeric_strings_reuse_one_temporary_slot_and_keep_inserted_strings() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        with_quakec_calls(rules, 1, |calls| {
            let mut vm = quakec_math_program(26);
            let persistent = vm.insert_string(b"persistent").unwrap();
            let extent = vm.strings.len();
            let temporary = (extent - qa_compat::quakec::TEMP_STRING_BYTES) as u32;
            assert_eq!(
                vm.string(temporary as i32),
                Err(qa_compat::quakec::Trap::String)
            );
            assert_eq!(
                vm.string(persistent + 12),
                Err(qa_compat::quakec::Trap::String)
            );
            vm.image.functions[2].first_statement = -25;
            vm.image.globals[48] = temporary;
            assert_eq!(
                vm.call(calls, 1, 100, false),
                Err(qa_compat::quakec::Trap::String)
            );
            assert!(calls.services.server.events.is_empty());
            vm.image.functions[2].first_statement = -26;
            for (value, expected) in [
                (0.0f32, "0"),
                (-0.0, "0"),
                (17.0, "17"),
                (-7.5, " -7.5"),
                (1.25, "  1.2"),
                (2.75, "  2.8"),
                (f32::NAN, "  nan"),
                (f32::from_bits(0xffc00000), " -nan"),
                (f32::INFINITY, "  inf"),
                (f32::NEG_INFINITY, " -inf"),
            ] {
                vm.image.globals[48] = value.to_bits();
                let result = vm.call(calls, 1, 100, false).unwrap();
                assert_eq!(result[0], temporary);
                assert_eq!(vm.string(temporary as i32).unwrap(), expected.as_bytes());
                assert_eq!(
                    vm.string(temporary as i32 + expected.len() as i32 + 1),
                    Err(qa_compat::quakec::Trap::String)
                );
                assert_eq!(vm.string(persistent).unwrap(), b"persistent");
                assert_eq!(vm.strings.len(), extent);
                assert_eq!(&result[1..], &[0x87654321, 0x11223344]);
            }
            vm.image.functions[2].first_statement = -27;
            vm.image.globals[48..51].copy_from_slice(&[-0.0f32, 1.25, -2.75].map(f32::to_bits));
            assert_eq!(vm.call(calls, 1, 100, false).unwrap()[0], temporary);
            assert_eq!(vm.string(temporary as i32).unwrap(), b"' -0.0   1.2  -2.8'");
            vm.image.globals[48..51].fill(f32::MAX.to_bits());
            assert_eq!(vm.call(calls, 1, 100, false).unwrap()[0], temporary);
            let prior_text = vm.string(temporary as i32).unwrap().to_vec();
            assert_eq!(prior_text.len(), 127);
            let before = vm.image.globals[1..4].to_vec();
            vm.image.globals[48..51].fill((-f32::MAX).to_bits());
            assert_eq!(
                vm.call(calls, 1, 100, false),
                Err(qa_compat::quakec::Trap::Builtin)
            );
            assert_eq!(&vm.image.globals[1..4], before);
            assert_eq!(vm.string(temporary as i32).unwrap(), prior_text);
            let second = vm.insert_string(b"second").unwrap();
            assert_eq!(second, persistent + 11);
            assert_eq!(vm.string(second).unwrap(), b"second");
            assert!(calls.services.server.events.is_empty());
        });
    }
}

#[test]
fn quakec_numeric_strings_reach_print_consumers_through_session_dispatch() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        for (builtin, values, expected) in [
            (26, [-7.5f32, 0.0, 0.0], b" -7.5".as_slice()),
            (
                27,
                [-0.0f32, 1.25, -2.75],
                b"' -0.0   1.2  -2.8'".as_slice(),
            ),
        ] {
            let bytes = quakec_program::program(
                &[
                    (StoreV, 48, 4, 0),
                    (Call1, 27, 0, 0),
                    (StoreS, 1, 52, 0),
                    (StoreS, 52, 4, 0),
                    (Call1, 28, 0, 0),
                    (Return, 52, 0, 0),
                ],
                &[0, 1, -builtin, -25],
            );
            let mut vm = Vm::load(
                Image::parse(&bytes, Some(5927)).unwrap(),
                Layout {
                    entities: 64,
                    header_bytes: 16,
                    extra_string_bytes: 0,
                    state_step: 0.1,
                },
            )
            .unwrap();
            let temporary = (vm.strings.len() - qa_compat::quakec::TEMP_STRING_BYTES) as u32;
            vm.image.globals[27] = 2;
            vm.image.globals[28] = 3;
            vm.image.globals[48..51].copy_from_slice(&values.map(f32::to_bits));
            let mut host = quakec_host(rules, vm);
            let developer = host.console.cvars.find("developer").unwrap();
            host.console.cvars.set_text(developer, "1").unwrap();
            host.call_module_export(
                ModuleId(1),
                Export::clocked(CallbackId(0)),
                ThinkTime::Seconds(0.1),
            )
            .unwrap();
            assert_eq!(
                host.module_counts(ModuleId(1))
                    .unwrap()
                    .last_result
                    .unwrap(),
                ModuleResult::QuakeC([temporary, 0, 0])
            );
            assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 0);
            let events = &mut host.runtime.server.events;
            let mut batch = events.batch(host.runtime.server.presentation).unwrap();
            let record = events.next(&mut batch).unwrap();
            let FrameEvent::Print(print) = record.event else {
                panic!()
            };
            assert_eq!(events.texts.get(print.text).unwrap(), expected);
            assert!(events.next(&mut batch).is_none());
        }
    }
}

#[test]
fn quakec_vlen_uses_three_words_of_one_parameter_through_session_dispatch() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        for (vector, expected) in [
            ([3.0f32, 4.0, 12.0], 13.0f32),
            ([0.0, -0.0, -0.0], 0.0),
            ([f32::from_bits(1), 0.0, 0.0], 0.0),
            ([f32::MAX, 0.0, 0.0], f32::INFINITY),
        ] {
            let mut vm = quakec_math_program(12);
            vm.image.globals[48..51].copy_from_slice(&vector.map(f32::to_bits));
            let mut host = quakec_host(rules, vm);
            host.call_module_export(
                ModuleId(1),
                Export::clocked(CallbackId(0)),
                ThinkTime::Seconds(0.1),
            )
            .unwrap();
            assert_eq!(
                host.module_counts(ModuleId(1)).unwrap().last_result,
                Some(ModuleResult::QuakeC([
                    expected.to_bits(),
                    0x87654321,
                    0x11223344,
                ]))
            );
            assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 0);
            assert!(host.console.idle());
            assert!(host.runtime.server.events.is_empty());
        }
    }
}

fn with_quakec_calls(
    rules: RuleSetId,
    clients: usize,
    run: impl FnOnce(&mut qa_compat::abi::QuakeCCalls<'_, '_>),
) {
    use qa_compat::{
        abi::{QUAKEC, QuakeCCalls, UnknownCalls},
        services::ServiceStorage,
    };
    let mut runtime = Runtime::load(clients, []).unwrap();
    let mut console = Console::new(Context::default()).unwrap();
    let mut storage = ServiceStorage::load(&[], 0, &console.cvars).unwrap();
    let mut scratch = runtime.geometry.scratch();
    let developer = console.cvars.find("developer").unwrap();
    let mut unknown = UnknownCalls::load(1).unwrap();
    let mut services = runtime.engine_services(&mut console, &mut storage, &mut scratch);
    let mut calls = QuakeCCalls {
        services: &mut services,
        table: &QUAKEC,
        context: CallContext {
            native_rules: rules,
            trace_rules: rules,
            sound_policy: qa_gameplay::rules::sound_policy(rules),
            module: ModuleId(1),
            clock: ThinkTime::Seconds(0.0),
            server_frame: 0,
            console: Context {
                source: rules,
                ..Context::default()
            },
            allocation: AllocationPolicy::EDICT,
            link_order: qa_gameplay::rules::link_order(rules),
        },
        platform_time: EventTime(0),
        developer,
        unknown: &mut unknown,
    };
    run(&mut calls);
}

#[test]
fn quakec_client_prints_resolve_native_namespaces_and_keep_native_payloads() {
    use qa_core::events::OutputSubmission;
    use qa_network::commands::packet::Protocol;
    use qa_session::clients::Connection;
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        for (builtin, level) in [(24, 1.75f32), (24, 2.0), (24, 3.0), (73, 2.0)] {
            with_quakec_calls(rules, 3, |calls| {
                let identities = [
                    NativeEntity {
                        module: ModuleId(2),
                        slot: 3,
                    },
                    NativeEntity {
                        module: ModuleId(1),
                        slot: 3,
                    },
                    NativeEntity {
                        module: ModuleId(1),
                        slot: 1,
                    },
                ];
                let clients = identities.map(|native| {
                    calls
                        .services
                        .server
                        .connect(Connection::Remote, native.module, Some(native))
                        .unwrap()
                });
                let target = clients[1];
                assert_ne!(target.0, identities[1].slot as u32);
                assert_ne!(
                    calls.services.server.clients[target.0 as usize].entity.slot,
                    3
                );
                let qw_level = rules == RuleSetId::QuakeWorld && builtin == 24;
                let operations = [
                    (StoreF, 48, 4, 0),
                    (StoreF, 49, 7, 0),
                    (StoreF, 50, 10, 0),
                    (StoreF, 51, 13, 0),
                    (if qw_level { Call4 } else { Call3 }, 27, 0, 0),
                    (Return, 1, 0, 0),
                ];
                let mut vm = Vm::load(
                    Image::parse(
                        &quakec_program::program(&operations, &[0, 1, -builtin]),
                        Some(5927),
                    )
                    .unwrap(),
                    Layout {
                        entities: 64,
                        header_bytes: 16,
                        extra_string_bytes: 2048,
                        state_step: 0.1,
                    },
                )
                .unwrap();
                let first = vm.insert_string(b"\x82seat").unwrap() as u32;
                let second = vm.insert_string(b"\n").unwrap() as u32;
                vm.image.globals[27] = 2;
                vm.image.globals[48] = (vm.stride * 3) as u32;
                let arguments = if qw_level {
                    [level.to_bits(), first, second]
                } else {
                    [first, second, 0]
                };
                vm.image.globals[49..52].copy_from_slice(&arguments);
                let returns = [0x12345678, 0x87654321, 0x11223344];
                vm.image.globals[1..4].copy_from_slice(&returns);
                assert_eq!(vm.call(calls, 1, 100, false), Ok(returns));
                let presentation = calls.services.server.presentation;
                let events = &mut calls.services.server.events;
                let mut batch = events.batch(presentation).unwrap();
                let record = events.next(&mut batch).unwrap();
                let FrameEvent::Print(print) = record.event else {
                    panic!()
                };
                assert_eq!(print.client, Some(target));
                let wire_level = if qw_level { level as u8 } else { 2 };
                let kind = if builtin == 73 {
                    PrintKind::Center
                } else {
                    PrintKind::from_level(wire_level)
                };
                assert_eq!((print.kind, print.level), (kind, wire_level));
                let text = events.texts.get(print.text).unwrap();
                assert_eq!(text, b"\x82seat\n");
                for protocol in [
                    Protocol::NetQuake15,
                    Protocol::QuakeWorld28,
                    Protocol::Quake2_34,
                    Protocol::Quake2Repro1038,
                    Protocol::Quake3_68,
                ] {
                    let mut output = [0; 64];
                    let count = qa_network::outputs::print(
                        protocol,
                        kind,
                        wire_level,
                        text,
                        None,
                        &mut output,
                    )
                    .unwrap();
                    let mut expected = match protocol {
                        Protocol::NetQuake15 | Protocol::QuakeWorld28 => {
                            vec![if builtin == 73 { 26 } else { 8 }]
                        }
                        Protocol::Quake2_34 | Protocol::Quake2Repro1038 => {
                            vec![if builtin == 73 { 15 } else { 10 }]
                        }
                        Protocol::Quake3_68 => match kind {
                            PrintKind::Center => b"cp \"".to_vec(),
                            PrintKind::Chat => b"chat \"".to_vec(),
                            _ => b"print \"".to_vec(),
                        },
                    };
                    if builtin != 73
                        && matches!(
                            protocol,
                            Protocol::QuakeWorld28
                                | Protocol::Quake2_34
                                | Protocol::Quake2Repro1038
                        )
                    {
                        expected.push(wire_level);
                    }
                    if protocol == Protocol::NetQuake15 && kind == PrintKind::Chat {
                        expected.push(1);
                    }
                    expected.extend(text);
                    if protocol == Protocol::Quake3_68 {
                        expected.push(b'"');
                    }
                    expected.push(0);
                    assert_eq!(
                        &output[..count],
                        expected,
                        "{rules:?} builtin {builtin} {protocol:?}"
                    );
                }
                assert!(events.submit(presentation, record.sequence, OutputSubmission::BestEffort));
                assert!(!events.is_empty());
                for client in clients {
                    let consumer = calls.services.server.clients[client.0 as usize]
                        .output
                        .unwrap();
                    let events = &mut calls.services.server.events;
                    let mut batch = events.batch(consumer).unwrap();
                    if client == target {
                        assert_eq!(events.next(&mut batch).unwrap().sequence, record.sequence);
                        assert!(events.submit(
                            consumer,
                            record.sequence,
                            OutputSubmission::BestEffort
                        ));
                    } else {
                        assert!(events.next(&mut batch).is_none());
                    }
                }
                assert!(calls.services.server.events.is_empty());
                // A misaligned or out-of-range edict offset traps only on use.
                for entity in [1, vm.entities.len() as u32] {
                    vm.image.globals[48] = entity;
                    assert_eq!(
                        vm.call(calls, 1, 100, false),
                        Err(qa_compat::quakec::Trap::Memory)
                    );
                    assert_eq!(&vm.image.globals[1..4], &returns);
                    assert!(calls.services.server.events.is_empty());
                }
                // Removing the binding cannot redirect an old offset to the
                // other module's client with the same native ordinal.
                assert!(calls.services.server.disconnect(target));
                vm.image.globals[48] = (vm.stride * 3) as u32;
                assert_eq!(vm.call(calls, 1, 100, false), Ok(returns));
                let events = &mut calls.services.server.events;
                let mut batch = events.batch(presentation).unwrap();
                let record = events.next(&mut batch).unwrap();
                let FrameEvent::Print(print) = record.event else {
                    panic!()
                };
                assert_eq!((print.client, print.kind), (None, PrintKind::Console));
                assert_eq!(
                    events.texts.get(print.text).unwrap(),
                    b"tried to sprint to a non-client\n"
                );
                for client in [clients[0], clients[2]] {
                    let mut batch = events
                        .batch(
                            calls.services.server.clients[client.0 as usize]
                                .output
                                .unwrap(),
                        )
                        .unwrap();
                    assert!(events.next(&mut batch).is_none());
                }
                assert!(events.submit(presentation, record.sequence, OutputSubmission::BestEffort));
                assert!(events.is_empty());
                assert_eq!(calls.unknown.calls, 0);
                let mut host = quakec_host(rules, vm);
                let target = host
                    .runtime
                    .server
                    .connect(Connection::Remote, ModuleId(1), Some(identities[1]))
                    .unwrap();
                host.call_module_export(
                    ModuleId(1),
                    Export::clocked(CallbackId(0)),
                    ThinkTime::module_clock(rules),
                )
                .unwrap();
                assert_eq!(
                    host.module_counts(ModuleId(1)).unwrap().last_result,
                    Some(ModuleResult::QuakeC(returns))
                );
                assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 0);
                let mut batch = host
                    .runtime
                    .server
                    .events
                    .batch(host.runtime.server.presentation)
                    .unwrap();
                let FrameEvent::Print(print) =
                    host.runtime.server.events.next(&mut batch).unwrap().event
                else {
                    panic!()
                };
                assert_eq!(
                    (print.client, print.kind, print.level),
                    (Some(target), kind, wire_level)
                );
                assert_eq!(
                    host.runtime.server.events.texts.get(print.text).unwrap(),
                    b"\x82seat\n"
                );
            });
        }
    }
}

#[test]
fn quakec_developer_prints_stay_local_while_broadcasts_reach_client_consumers() {
    use qa_core::events::{OutputSubmission, OutputTarget};
    use qa_core::primitives::ClientId;
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        for (builtin, enabled) in [(23, true), (25, true), (25, false)] {
            with_quakec_calls(rules, 1, |calls| {
                let context = calls.context.console;
                let developer = calls.services.cvars.bind("developer", context).unwrap();
                (qa_compat::services::ENGINE_CALLS.cvar_force)(
                    calls.services,
                    developer,
                    if enabled { "1" } else { "0" },
                )
                .unwrap();
                let remote = calls
                    .services
                    .server
                    .events
                    .bind(OutputTarget::Client(ClientId(0)))
                    .unwrap();
                let mut vm = quakec_math_program(builtin);
                let string = vm.insert_string(b"\x82local probe").unwrap() as u32;
                let function = if builtin == 23 && rules == RuleSetId::QuakeWorld {
                    vm.image.globals[4] = 2.0f32.to_bits();
                    vm.image.globals[7] = string;
                    4
                } else {
                    vm.image.globals[48] = string;
                    1
                };
                assert_eq!(
                    vm.call(calls, function, 100, false),
                    Ok([0x12345678, 0x87654321, 0x11223344])
                );
                let events = &mut calls.services.server.events;
                let presentation = calls.services.server.presentation;
                let mut client = events.batch(remote).unwrap();
                assert_eq!(
                    events.next(&mut client).is_some(),
                    builtin == 23,
                    "{rules:?} builtin {builtin} developer={enabled}"
                );
                let mut local = events.batch(presentation).unwrap();
                if enabled || rules == RuleSetId::QuakeWorld {
                    let record = events.next(&mut local).unwrap();
                    let FrameEvent::Print(print) = record.event else {
                        panic!()
                    };
                    assert_eq!(print.kind, PrintKind::Console);
                    assert_eq!(events.texts.get(print.text).unwrap(), b"\x82local probe");
                    assert!(events.submit(
                        presentation,
                        record.sequence,
                        OutputSubmission::BestEffort
                    ));
                    if builtin == 25 {
                        assert!(events.is_empty());
                    }
                } else {
                    assert!(events.next(&mut local).is_none());
                    assert!(events.is_empty());
                }
            });
        }
    }
}

#[test]
fn quakec_cvar_signature_reads_both_strings_and_preserves_void_returns() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        with_quakec_calls(rules, 1, |calls| {
            let mut vm = quakec_math_program(72);
            vm.image.globals[4] = vm.insert_string(b"fov").unwrap() as u32;
            vm.image.globals[7] = vm.insert_string(b"83").unwrap() as u32;
            let returns = [0x12345678, 0x87654321, 0x11223344];
            assert_eq!(vm.call(calls, 4, 100, false), Ok(returns));
            let fov = calls.services.cvars.find("fov").unwrap();
            assert_eq!(calls.services.cvars.value_in(fov, rules), 83.0);
            for function in [1, 3] {
                assert_eq!(
                    vm.call(calls, function, 100, false),
                    Err(qa_compat::quakec::Trap::Builtin)
                );
                assert_eq!(&vm.image.globals[1..4], &returns);
                assert_eq!(calls.services.cvars.value_in(fov, rules), 83.0);
            }
            vm.image.globals[7] = u32::MAX;
            assert_eq!(
                vm.call(calls, 4, 100, false),
                Err(qa_compat::quakec::Trap::String)
            );
            assert_eq!(&vm.image.globals[1..4], &returns);
            assert_eq!(calls.services.cvars.value_in(fov, rules), 83.0);
            // QuakeC offset zero reads the string; it is not Q3's NULL reset.
            vm.image.globals[4] = vm.insert_string(b"sensitivity").unwrap() as u32;
            vm.image.globals[7] = 0;
            vm.strings.write(0, b"7\0").unwrap();
            let sensitivity = calls
                .services
                .cvars
                .bind("sensitivity", calls.context.console)
                .unwrap();
            calls.services.cvars.force_write(sensitivity, "9").unwrap();
            assert_eq!(vm.call(calls, 4, 100, false), Ok(returns));
            assert_eq!(
                calls.services.cvars.read(sensitivity).unwrap().as_str(),
                "7"
            );
        });
    }
}

#[test]
fn quakec_makevectors_matches_original_rows_and_preserves_void_returns() {
    let rows =
        std::env::var_os("QA_QC_MAKEVECTORS_REFERENCE").map(|path| std::fs::read(path).unwrap());
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        with_quakec_calls(rules, 1, |calls| {
            let mut vm = quakec_math_program(1);
            for definition in vm.image.globaldefs.iter_mut() {
                if definition.value_type == 3 {
                    definition.offset = 63;
                }
            }
            let returns = [0x12345678, 0x87654321, 0x11223344];
            if let Some(rows) = &rows {
                assert!(!rows.is_empty());
                assert_eq!(rows.len() % 48, 0);
                for (index, row) in rows.chunks_exact(48).enumerate() {
                    let words: [u32; 12] = std::array::from_fn(|i| {
                        u32::from_le_bytes(row[i * 4..i * 4 + 4].try_into().unwrap())
                    });
                    vm.image.globals[48..51].copy_from_slice(&words[..3]);
                    assert_eq!(vm.call(calls, 1, 100, false), Ok(returns));
                    assert_eq!(
                        &vm.image.globals[52..61],
                        &words[3..],
                        "{rules:?} row {index}"
                    );
                }
            }
            vm.image.globals[48..51].fill(0);
            vm.call(calls, 1, 100, false).unwrap();
            let expected = [1.0f32, 0.0, -0.0, 0.0, -1.0, -0.0, 0.0, 0.0, 1.0];
            assert_eq!(&vm.image.globals[52..61], &expected.map(f32::to_bits));
            let before = vm.image.globals[52..61].to_vec();
            for function in [3, 4] {
                assert_eq!(
                    vm.call(calls, function, 100, false),
                    Err(qa_compat::quakec::Trap::Builtin)
                );
                assert_eq!(&vm.image.globals[1..4], &returns);
                assert_eq!(&vm.image.globals[52..61], &before);
            }
            let mut missing = quakec_math_program_with_vectors(1, &[]);
            assert_eq!(
                missing.call(calls, 1, 100, false),
                Err(qa_compat::quakec::Trap::Memory)
            );
            assert_eq!(&missing.image.globals[1..4], &returns);
            assert_eq!(calls.unknown.calls, 0);
        });
        let mut host = quakec_host(rules, quakec_math_program(1));
        host.call_module_export(
            ModuleId(1),
            Export::clocked(CallbackId(0)),
            ThinkTime::Seconds(0.1),
        )
        .unwrap();
        assert_eq!(
            host.module_counts(ModuleId(1)).unwrap().last_result,
            Some(ModuleResult::QuakeC([0x12345678, 0x87654321, 0x11223344]))
        );
        assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 0);
    }
}

#[test]
fn quakec_math_matches_original_rows_and_rejects_wrong_parameter_counts() {
    for (builtin, key) in [
        (9, "QA_QC_NORMALIZE_REFERENCE"),
        (12, "QA_QC_VLEN_REFERENCE"),
        (13, "QA_QC_VECTOYAW_REFERENCE"),
        (36, "QA_QC_RINT_REFERENCE"),
        (51, "QA_QC_VECTOANGLES_REFERENCE"),
    ] {
        let rows = std::env::var_os(key).map(|path| std::fs::read(path).unwrap());
        for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
            with_quakec_calls(rules, 1, |calls| {
                let mut vm = quakec_math_program(builtin);
                if let Some(rows) = &rows {
                    assert!(!rows.is_empty());
                    assert_eq!(rows.len() % 24, 0);
                    for (index, row) in rows.chunks_exact(24).enumerate() {
                        let words: [u32; 6] = std::array::from_fn(|i| {
                            u32::from_le_bytes(row[i * 4..i * 4 + 4].try_into().unwrap())
                        });
                        vm.image.globals[48..51].copy_from_slice(&words[..3]);
                        assert_eq!(
                            vm.call(calls, 1, 100, false),
                            Ok(words[3..].try_into().unwrap()),
                            "builtin {builtin}, {rules:?} row {index}"
                        );
                    }
                }
                for vector in [
                    [f32::NAN, 1.0, 0.0],
                    [f32::INFINITY, f32::NEG_INFINITY, 0.0],
                ] {
                    vm.image.globals[48..51].copy_from_slice(&vector.map(f32::to_bits));
                    let result = vm.call(calls, 1, 100, false).unwrap();
                    if builtin == 9 {
                        if vector[0].is_nan() {
                            assert!(result.into_iter().all(|word| f32::from_bits(word).is_nan()));
                        } else {
                            assert!(f32::from_bits(result[0]).is_nan());
                            assert!(f32::from_bits(result[1]).is_nan());
                            assert_eq!(result[2], 0.0f32.to_bits());
                        }
                    } else if matches!(builtin, 13 | 51) {
                        let yaw = if vector[0].is_nan() {
                            i32::MIN as f32 + 360.0
                        } else {
                            315.0
                        };
                        if builtin == 13 {
                            assert_eq!(result[0], yaw.to_bits());
                        } else {
                            let pitch = if vector[0].is_nan() { yaw } else { 0.0 };
                            assert_eq!(result, [pitch, yaw, 0.0].map(f32::to_bits));
                        }
                    } else if builtin == 36 {
                        assert_eq!(result[0], (i32::MIN as f32).to_bits());
                    } else if vector[0].is_nan() {
                        assert!(f32::from_bits(result[0]).is_nan());
                    } else {
                        assert_eq!(result[0], f32::INFINITY.to_bits());
                    }
                    if !matches!(builtin, 9 | 51) {
                        assert_eq!(&result[1..], &[0x87654321, 0x11223344]);
                    }
                }
                let before: [u32; 3] = vm.image.globals[1..4].try_into().unwrap();
                for function in [3, 4] {
                    assert_eq!(
                        vm.call(calls, function, 100, false),
                        Err(qa_compat::quakec::Trap::Builtin)
                    );
                    assert_eq!(&vm.image.globals[1..4], &before);
                }
                assert_eq!(calls.unknown.calls, 0);
            });
        }
    }
}

#[test]
fn quakec_vector_angles_keep_native_axes_and_integer_angles_through_session_dispatch() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        for builtin in [13, 51] {
            for (vector, angles) in [
                ([0.0f32, -0.0, 0.0], [270.0f32, 0.0, 0.0]),
                ([0.0, 0.0, 1.0], [90.0, 0.0, 0.0]),
                ([0.0, 0.0, -1.0], [270.0, 0.0, 0.0]),
                ([1.0, 0.0, 0.0], [0.0, 0.0, 0.0]),
                ([0.0, 1.0, 0.0], [0.0, 90.0, 0.0]),
                ([-1.0, -0.0, 0.0], [0.0, 180.0, 0.0]),
                ([0.0, -1.0, 0.0], [0.0, 270.0, 0.0]),
                ([1.0, 1.0, 1.0], [35.0, 45.0, 0.0]),
                ([1.0, -1.0, -1.0], [325.0, 315.0, 0.0]),
                ([f32::MAX, 0.0, 1.0], [0.0, 0.0, 0.0]),
            ] {
                let mut vm = quakec_math_program(builtin);
                vm.image.globals[48..51].copy_from_slice(&vector.map(f32::to_bits));
                let mut host = quakec_host(rules, vm);
                host.call_module_export(
                    ModuleId(1),
                    Export::clocked(CallbackId(0)),
                    ThinkTime::Seconds(0.1),
                )
                .unwrap();
                let expected = if builtin == 13 {
                    [angles[1].to_bits(), 0x87654321, 0x11223344]
                } else {
                    angles.map(f32::to_bits)
                };
                assert_eq!(
                    host.module_counts(ModuleId(1)).unwrap().last_result,
                    Some(ModuleResult::QuakeC(expected)),
                    "{rules:?} builtin {builtin}: {vector:?}"
                );
                assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 0);
                assert!(host.console.idle());
                assert!(host.runtime.server.events.is_empty());
            }
        }
    }
}

#[test]
fn quakec_normalize_returns_all_three_words_through_session_dispatch() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        for (vector, expected) in [
            ([3.0f32, 4.0, 0.0], [0.6f32, 0.8, 0.0]),
            ([1.0, 2.0, 2.0], [1.0 / 3.0, 2.0 / 3.0, 2.0 / 3.0]),
            ([-0.0, 0.0, -0.0], [0.0, 0.0, 0.0]),
            ([f32::from_bits(1), 0.0, -0.0], [0.0, 0.0, 0.0]),
            ([-f32::MAX, -0.0, 1.0], [-0.0, -0.0, 0.0]),
        ] {
            let mut vm = quakec_math_program(9);
            vm.image.globals[48..51].copy_from_slice(&vector.map(f32::to_bits));
            let mut host = quakec_host(rules, vm);
            host.call_module_export(
                ModuleId(1),
                Export::clocked(CallbackId(0)),
                ThinkTime::Seconds(0.1),
            )
            .unwrap();
            assert_eq!(
                host.module_counts(ModuleId(1)).unwrap().last_result,
                Some(ModuleResult::QuakeC(expected.map(f32::to_bits))),
                "{rules:?}: {vector:?}"
            );
            assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 0);
            assert!(host.console.idle());
            assert!(host.runtime.server.events.is_empty());
        }
    }
}

#[test]
fn quakec_rint_keeps_double_bias_and_native_integer_width_through_session_dispatch() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        for (value, expected) in [
            (f32::from_bits(0x3effffff), 0.0f32),
            (0.5, 1.0),
            (f32::from_bits(0xbeffffff), 0.0),
            (-0.5, -1.0),
            (1.5, 2.0),
            (-1.5, -2.0),
            (-0.0, 0.0),
            (16777215.0, 16777215.0),
            (-16777215.0, -16777215.0),
            (2147483520.0, 2147483520.0),
            (2147483648.0, i32::MIN as f32),
        ] {
            let mut vm = quakec_math_program(36);
            vm.image.globals[48..51].copy_from_slice(&[value.to_bits(), 0, 0]);
            let mut host = quakec_host(rules, vm);
            host.call_module_export(
                ModuleId(1),
                Export::clocked(CallbackId(0)),
                ThinkTime::Seconds(0.1),
            )
            .unwrap();
            assert_eq!(
                host.module_counts(ModuleId(1)).unwrap().last_result,
                Some(ModuleResult::QuakeC([
                    expected.to_bits(),
                    0x87654321,
                    0x11223344,
                ])),
                "{rules:?}: {value}"
            );
            assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 0);
            assert!(host.console.idle());
            assert!(host.runtime.server.events.is_empty());
        }
    }
}

#[test]
fn quakec_broadcast_and_localcmd_share_output_and_deferred_commands() {
    use qa_network::commands::packet::Protocol;
    for (rules, level, kind, wire_level) in [
        (RuleSetId::Quake, 3.0, PrintKind::Console, 2),
        (RuleSetId::QuakeWorld, 3.0, PrintKind::Chat, 3),
        (RuleSetId::QuakeWorld, 1.75, PrintKind::Notify, 1),
    ] {
        let mut host = quakec_output_host(rules, level, [b"\x82broadcast", b"\n"]);
        let fov = host.console.cvars.find("cg_fov").unwrap();
        let before = host.console.cvars.value_in(fov, rules);
        host.console
            .append(
                "fov 77\n",
                Context {
                    source: rules,
                    ..Context::default()
                },
            )
            .unwrap();
        host.call_module_export(
            ModuleId(1),
            Export::clocked(CallbackId(0)),
            ThinkTime::Seconds(0.1),
        )
        .unwrap();
        assert_eq!(
            host.module_counts(ModuleId(1)).unwrap().last_result,
            Some(ModuleResult::QuakeC([0x12345678, 0x87654321, 0x11223344]))
        );
        assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 0);
        assert_eq!(host.console.cvars.value_in(fov, rules), before);
        assert!(!host.console.idle());
        let events = &host.runtime.server.events;
        let mut batch = events.batch(host.runtime.server.presentation).unwrap();
        let FrameEvent::Print(print) = events.next(&mut batch).unwrap().event else {
            panic!()
        };
        assert_eq!(print.client, None);
        assert_eq!(print.kind, kind);
        assert_eq!(print.level, wire_level);
        let text = events.texts.get(print.text).unwrap();
        assert_eq!(text, b"\x82broadcast\n");
        assert!(events.next(&mut batch).is_none());
        let protocol = if rules == RuleSetId::QuakeWorld {
            Protocol::QuakeWorld28
        } else {
            Protocol::NetQuake15
        };
        let mut wire = [0; 64];
        let size =
            qa_network::outputs::print(protocol, kind, wire_level, text, None, &mut wire).unwrap();
        let mut expected = vec![8];
        if protocol == Protocol::QuakeWorld28 {
            expected.push(wire_level);
        }
        expected.extend(text);
        expected.push(0);
        assert_eq!(&wire[..size], expected);
        host.console.execute_frame(&mut host.runtime);
        assert_eq!(host.console.cvars.value_in(fov, rules), 111.0);
        assert!(host.console.idle());
    }
}

#[test]
fn oversized_quakec_broadcast_traps_before_publishing_or_queuing_commands() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let mut host = quakec_output_host(rules, 2.0, [&[b'x'; 1024], b"x"]);
        assert_eq!(
            host.call_module_export(
                ModuleId(1),
                Export::clocked(CallbackId(0)),
                ThinkTime::Seconds(0.1)
            ),
            Err(CallError::Rejected)
        );
        assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 1);
        assert!(host.console.idle());
        let events = &host.runtime.server.events;
        let mut batch = events.batch(host.runtime.server.presentation).unwrap();
        assert!(events.next(&mut batch).is_none());
    }
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
            0.0f32.to_bits()
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
fn scheduling_role_selects_clock_arithmetic_and_phase_independently_of_program_format() {
    use qa_formats::program::qvm::Opcode;
    let mut runtime = Runtime::load(4, []).unwrap();
    let mut console = Console::new(Context::default()).unwrap();
    let qw_view = console
        .cvars
        .bind(
            "sv_mintic",
            Context {
                source: RuleSetId::QuakeWorld,
                ..Context::default()
            },
        )
        .unwrap();
    console.cvars.force_write(qw_view, "0.01").unwrap();
    let requests = [RuleSetId::Quake, RuleSetId::QuakeWorld, RuleSetId::Quake3]
        .into_iter()
        .enumerate()
        .map(|(index, timing_rules)| {
            let module = ModuleId(index as u16 + 1);
            let anchor = runtime
                .server
                .entities
                .allocate(ThinkTime::Seconds(1.0), module, AllocationPolicy::q3(0))
                .unwrap()
                .id;
            let (native_rules, program, entries) = if index == 2 {
                let bytes = quakec_program::program_with_definitions(
                    &[(Return, 35, 0, 0)],
                    &[0, 1],
                    &[(2, 35, b"time")],
                    &[],
                    1,
                    64,
                );
                let vm = Vm::load(
                    Image::parse(&bytes, Some(5927)).unwrap(),
                    Layout {
                        entities: 64,
                        header_bytes: 16,
                        extra_string_bytes: 0,
                        state_step: 0.1,
                    },
                )
                .unwrap();
                (RuleSetId::Quake, Program::quakec(vm, None), vec![1])
            } else {
                // vmMain's first supplied argument is the native clock in ms.
                let vm = service_program::program(
                    &[
                        (Opcode::Enter, 64),
                        (Opcode::Local, 76),
                        (Opcode::Load4, 0),
                        (Opcode::Leave, 64),
                    ],
                    &[0; 16],
                );
                (
                    RuleSetId::Quake3,
                    Program::Qvm {
                        vm: Box::new(vm),
                        imports: &Q3_SERVER,
                    },
                    vec![0],
                )
            };
            ModuleRequest {
                context: CallContext {
                    module,
                    clock: ThinkTime::Seconds(1.0),
                    server_frame: 0,
                    native_rules,
                    trace_rules: RuleSetId::Quake2,
                    sound_policy: qa_gameplay::rules::sound_policy(RuleSetId::Quake2),
                    console: Context::default(),
                    allocation: AllocationPolicy::q3(0),
                    link_order: LinkOrder::Head,
                },
                phase: Phase::Server,
                timing_rules,
                rate: TickRate::FrameDriven,
                anchor,
                program,
                entries,
                frame: Export::clocked(CallbackId(0)),
                prepare: Vec::new(),
                initialize: Vec::new(),
                api: None,
                shutdown: Vec::new(),
                instruction_budget: 100,
                configstrings: 0,
                files: 0,
                entity_source: None,
            }
        })
        .collect();
    let mut host =
        FrameHost::load_modules(console, runtime, TickRate::FrameDriven, requests).unwrap();
    let mut source = Source {
        time: EventTime(0),
        polls: 0,
    };
    host.frame(&mut source, true);
    for (time, nq, qw) in [(12_345_678, 1000, 1012), (23_000_300, 1012, 1023)] {
        source.time = EventTime(time);
        host.frame(&mut source, true);
        assert_eq!(
            host.module_counts(ModuleId(1)).unwrap().last_result,
            Some(ModuleResult::Qvm(nq))
        );
        assert_eq!(
            host.module_counts(ModuleId(2)).unwrap().last_result,
            Some(ModuleResult::Qvm(qw))
        );
        assert_eq!(
            host.module_counts(ModuleId(3)).unwrap().last_result,
            Some(ModuleResult::QuakeC([
                (EventTime(time).seconds() as f32).to_bits(),
                0,
                0,
            ]))
        );
    }
    for module in [ModuleId(1), ModuleId(2), ModuleId(3)] {
        let counts = host.module_counts(module).unwrap();
        assert_eq!((counts.calls, counts.traps, counts.rejected), (2, 0, 0));
    }
    assert_eq!(source.polls, 6);
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

#[test]
fn missing_module_lookups_are_bounded() {
    let mut host = FrameHost::load_modules(
        Console::new(Context::default()).unwrap(),
        Runtime::load(1, std::iter::empty()).unwrap(),
        TickRate::FrameDriven,
        vec![],
    )
    .unwrap();
    for module in [ModuleId(0), ModuleId(u16::MAX)] {
        assert!(host.module_counts(module).is_none());
        assert!(host.module_state(module).is_none());
        let export = Export {
            client: None,
            callback: CallbackId(0),
            arguments: [qa_app::modules::Argument::Word(0); 9],
        };
        assert_eq!(
            host.call_module_export(module, export, ThinkTime::Milliseconds(0)),
            Err(CallError::MissingModule)
        );
    }
}
