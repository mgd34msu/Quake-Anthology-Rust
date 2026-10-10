use qa_app::{
    Runtime,
    host::{FrameHost, FrameSource},
    modules::{ModuleRequest, ModuleResult, Phase, Program, State},
};
use qa_compat::{
    abi::Q3_SERVER,
    native::{Export, Vm},
    services::CallContext,
};
use qa_console::{commands::Console, views::Context};
use qa_core::{
    events::{FrameEvent, OutputTarget},
    primitives::{CallbackId, ModuleId, RuleSetId},
    sys_events::{EventKind, EventTime, SysEvent, SysEventQueue},
};
use qa_platform::native::{NativeAbi, NativeImage, NativeProcess, NativeRegion};
use qa_session::timing::TickRate;
use qa_world::entities::{AllocationPolicy, EntityTime};
use std::time::Duration;

const BASE: u64 = 0x2000_0000;
fn standard(code: &[u8]) -> NativeProcess {
    let mut bytes = vec![0; 8192];
    bytes[..code.len()].copy_from_slice(code);
    NativeProcess::load(NativeImage {
        base: BASE,
        pointer_bytes: 8,
        bytes: &bytes,
        regions: &[
            NativeRegion {
                offset: 0,
                length: 4096,
                permissions: 5,
            },
            NativeRegion {
                offset: 4096,
                length: 4096,
                permissions: 3,
            },
        ],
        timeout: Duration::from_secs(3),
    })
    .expect("owned native child")
}

fn module(
    runtime: &mut Runtime,
    id: ModuleId,
    rules: RuleSetId,
    rate: TickRate,
    text: &[u8],
) -> ModuleRequest {
    // Load syscall pointer from shared RAM, print its shared text, then return.
    // sub rsp,8; mov rax,[rip+4085]; mov edi,0; lea rsi,[rip+4089]; call rax; add rsp,8; ret
    let code = [
        0x48, 0x83, 0xec, 8, 0x48, 0x8b, 0x05, 0xf5, 0x0f, 0, 0, 0xbf, 0, 0, 0, 0, 0x48, 0x8d,
        0x35, 0xf9, 0x0f, 0, 0, 0xff, 0xd0, 0x48, 0x83, 0xc4, 8, 0xc3,
    ];
    let mut process = standard(&code);
    let pointer = process.callback(NativeAbi::SystemV);
    let memory = process.memory_mut().expect("parked native backing");
    memory[4096..4104].copy_from_slice(&pointer.to_le_bytes());
    memory[4112..4112 + text.len()].copy_from_slice(text);
    let anchor = runtime
        .server
        .entities
        .allocate(EntityTime::Milliseconds(0), id, AllocationPolicy::EDICT)
        .unwrap()
        .id;
    ModuleRequest {
        context: CallContext {
            module: id,
            clock: EntityTime::Milliseconds(0),
            console: Context {
                source: rules,
                ..Context::default()
            },
            allocation: AllocationPolicy::EDICT,
            link_order: qa_gameplay::rules::link_order(rules),
        },
        phase: Phase::Server,
        timing_rules: rules,
        rate,
        anchor,
        program: Program::Native {
            vm: Box::new(
                Vm::load(
                    process,
                    NativeAbi::SystemV,
                    Box::new([Export {
                        address: BASE,
                        command: None,
                    }]),
                )
                .unwrap(),
            ),
            imports: &Q3_SERVER,
        },
        entries: vec![0],
        frame: CallbackId(0),
        initialize: None,
        api: None,
        shutdown: None,
        instruction_budget: 100,
        configstrings: 0,
        files: 0,
    }
}
struct Source {
    time: EventTime,
    polls: u32,
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

fn two_native_modules_use_session_rates_and_the_same_calltable_output_ring() {
    let mut runtime = Runtime::load(1, std::iter::empty()).unwrap();
    let observer = runtime
        .server
        .events
        .bind(OutputTarget::Module(ModuleId(7)))
        .unwrap();
    let q3 = module(
        &mut runtime,
        ModuleId(1),
        RuleSetId::Quake3,
        TickRate::fixed(50).unwrap(),
        b"q3 native\n\0",
    );
    let q2 = module(
        &mut runtime,
        ModuleId(2),
        RuleSetId::Quake2,
        TickRate::fixed(100).unwrap(),
        b"q2 native\n\0",
    );
    let mut host = FrameHost::load_modules(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        vec![q3, q2],
    )
    .unwrap();
    let mut source = Source {
        time: EventTime(0),
        polls: 0,
    };
    for time in [0, 50, 100, 150] {
        source.time = EventTime(time * 1_000_000);
        host.frame(&mut source, true);
    }
    assert_eq!(source.polls, 8);
    for (id, count) in [(ModuleId(1), 3), (ModuleId(2), 1)] {
        assert_eq!(host.module_state(id), Some(State::Running));
        let counters = host.module_counts(id).unwrap();
        assert_eq!((counters.calls, counters.traps), (count, 0));
        assert_eq!(counters.last_result, Some(ModuleResult::Native(0)));
    }
    let mut batch = host.runtime.server.events.batch(observer).unwrap();
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
    assert_eq!(
        prints,
        vec![
            b"q3 native\n".to_vec(),
            b"q3 native\n".to_vec(),
            b"q2 native\n".to_vec(),
            b"q3 native\n".to_vec()
        ]
    );
}

pub fn run() {
    two_native_modules_use_session_rates_and_the_same_calltable_output_ring();
    println!("native session dispatch checks passed");
}
