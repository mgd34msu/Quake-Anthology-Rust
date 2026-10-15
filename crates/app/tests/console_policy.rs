#[path = "support/service_program.rs"]
#[allow(dead_code)]
mod service_program;

use qa_app::{
    Runtime,
    host::{FrameHost, FrameSource},
    modules::{Export, ModuleRequest, Phase, Program},
};
use qa_compat::{abi::Q3_SERVER, services::CallContext};
use qa_console::{commands::Console, cvars::RuntimePolicy, views::Context};
use qa_core::{
    primitives::{CallbackId, ModuleId, RuleSetId, ThinkTime},
    sys_events::{EventKind, EventTime, SysEvent, SysEventQueue},
};
use qa_formats::program::qvm::Opcode;
use qa_session::timing::TickRate;
use qa_world::{area::LinkOrder, entities::AllocationPolicy};
use std::time::Duration;

fn host(source: RuleSetId) -> FrameHost {
    let mut runtime = Runtime::load(1, []).unwrap();
    let module = ModuleId(1);
    let allocation = AllocationPolicy::q3(0);
    let clock = ThinkTime::module_clock(source);
    let anchor = runtime
        .server
        .entities
        .allocate(clock, module, allocation)
        .unwrap()
        .id;
    let context = Context {
        source,
        ..Default::default()
    };
    let vm = service_program::program(
        &[(Opcode::Enter, 0), (Opcode::Const, 0), (Opcode::Leave, 0)],
        &[],
    );
    let mut console = Console::new(context).unwrap();
    console
        .cvars
        .register("_policy_init", Some("1"), 16, context)
        .unwrap();
    console
        .cvars
        .register("_policy_latch", Some("1"), 32, context)
        .unwrap();
    FrameHost::load_modules(
        console,
        runtime,
        TickRate::FrameDriven,
        vec![ModuleRequest {
            context: CallContext {
                module,
                native_rules: source,
                trace_rules: source,
                sound_policy: qa_gameplay::rules::sound_policy(source),
                clock,
                server_frame: 0,
                console: context,
                allocation,
                link_order: LinkOrder::Head,
            },
            phase: Phase::Server,
            timing_rules: source,
            rate: TickRate::fixed(100).unwrap(),
            anchor,
            program: Program::Qvm {
                vm: Box::new(vm),
                imports: &Q3_SERVER,
            },
            entries: vec![0],
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

#[derive(Default)]
struct Source {
    time: u64,
    polls: usize,
}
impl FrameSource for Source {
    fn begin_frame(&mut self) -> Duration {
        Duration::from_millis(self.time)
    }
    fn poll_events(&mut self, queue: &mut SysEventQueue) {
        self.polls += 1;
        queue
            .push(SysEvent {
                time: EventTime::from_milliseconds(self.time),
                kind: EventKind::Time,
            })
            .unwrap();
    }
    fn wait_time(&mut self, _: Duration) -> Duration {
        Duration::from_millis(self.time)
    }
    fn elapsed(&self) -> Duration {
        Duration::ZERO
    }
    fn present(&mut self) {}
}

#[test]
fn live_host_protects_init_and_latches_only_while_the_q2_provider_runs() {
    let mut host = host(RuleSetId::Quake2);
    let context = host.console.cvars.context();
    assert_eq!(
        host.console.cvars.runtime_policy(),
        RuntimePolicy {
            initialized: false,
            server_active: true
        }
    );
    // Startup config still runs in the eagerly registered catalog's setup phase.
    host.console.append("_policy_init 2\n", context).unwrap();
    host.console.execute_frame(&mut host.runtime);
    let init = host.console.cvars.bind("_policy_init", context).unwrap();
    let latch = host.console.cvars.bind("_policy_latch", context).unwrap();
    assert_eq!(host.console.cvars.read(init).unwrap().as_str(), "2");
    host.console
        .append("_policy_init 3; _policy_latch 2\n", context)
        .unwrap();
    let mut source = Source::default();
    host.frame(&mut source, true);
    assert_eq!(source.polls, 2);
    assert_eq!(host.console.cvars.read(init).unwrap().as_str(), "2");
    assert_eq!(host.console.cvars.read(latch).unwrap().as_str(), "1");
    assert_eq!(
        host.console.cvars.latched(latch).unwrap().unwrap().as_str(),
        "2"
    );
    assert!(host.shutdown_module(ModuleId(1)));
    assert!(!host.console.cvars.runtime_policy().server_active);
    host.console.append("_policy_latch 3\n", context).unwrap();
    source.time = 1;
    host.frame(&mut source, true);
    assert_eq!(host.console.cvars.read(latch).unwrap().as_str(), "3");
    assert!(host.console.cvars.latched(latch).unwrap().is_none());
    assert_eq!(source.polls, 4);
}

#[test]
fn live_q3_host_keeps_the_native_latch_rule_after_provider_shutdown() {
    let mut host = host(RuleSetId::Quake3);
    let context = host.console.cvars.context();
    let latch = host.console.cvars.bind("_policy_latch", context).unwrap();
    let mut source = Source::default();
    host.frame(&mut source, true);
    assert!(host.shutdown_module(ModuleId(1)));
    host.console.append("_policy_latch 2\n", context).unwrap();
    source.time = 1;
    host.frame(&mut source, true);
    assert_eq!(host.console.cvars.read(latch).unwrap().as_str(), "1");
    assert_eq!(
        host.console.cvars.latched(latch).unwrap().unwrap().as_str(),
        "2"
    );
    assert_eq!(
        host.console.cvars.runtime_policy(),
        RuntimePolicy {
            initialized: true,
            server_active: false
        }
    );
}
