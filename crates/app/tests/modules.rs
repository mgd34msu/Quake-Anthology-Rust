#[path = "../../compat/tests/support/quakec_program.rs"]
mod quakec_program;
#[path = "support/service_program.rs"]
mod service_program;

use qa_app::{
    Runtime,
    host::{FrameHost, FrameSource},
    modules::{ModuleRequest, ModuleResult, Program},
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
use qa_world::{
    area::LinkOrder,
    entities::{AllocationPolicy, EntityTime},
};
use std::time::Duration;

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
            .allocate(EntityTime::Seconds(0.0), id, AllocationPolicy::EDICT)
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
                clock: EntityTime::Milliseconds(0),
                console: Context::default(),
                allocation: AllocationPolicy::EDICT,
                link_order: LinkOrder::Head,
            },
            timing_rules: RuleSetId::Quake3,
            rate: TickRate::fixed(50).unwrap(),
            anchor: ids[0],
            program: Program::Qvm {
                vm: Box::new(service_program::qvm()),
                imports: &Q3_SERVER,
            },
            entries: vec![8],
            frame: CallbackId(0),
            instruction_budget: qvm_budget,
            configstrings: 0,
            files: 0,
        },
        ModuleRequest {
            context: CallContext {
                module: ModuleId(2),
                clock: EntityTime::Seconds(0.0),
                console: Context {
                    source: RuleSetId::Quake,
                    ..Context::default()
                },
                allocation: AllocationPolicy::EDICT,
                link_order: LinkOrder::Tail,
            },
            timing_rules: RuleSetId::Quake,
            rate: TickRate::fixed(100).unwrap(),
            anchor: ids[1],
            program: Program::quakec(qc),
            entries: vec![1, 5],
            frame: CallbackId(0),
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
            .release(ids[0], EntityTime::Milliseconds(150))
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
