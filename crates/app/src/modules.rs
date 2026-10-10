//! Module entry points enter through session dispatch and the shared services.
use crate::{
    Runtime,
    host::{FrameHost, Provider},
};
use qa_compat::{
    abi::{CallTable, QUAKEC, QuakeCCalls, QvmCalls, UnknownCalls},
    quakec, qvm,
    services::{CallContext, ServiceStorage},
};
use qa_core::primitives::{CallbackCall, CallbackId, EntityId, ModuleId, RuleSetId, ThinkTime};
use qa_session::{
    dispatch::{CallError, FunctionBinding, FunctionTable, ThinkWorld},
    timing::{Tick, TickRate},
};
use qa_world::entities::EntityTime;
use std::sync::Arc;

mod load;
pub use load::{QuakeCSpec, QvmRole, QvmSpec, load_quakec, load_qvm};

pub enum Program {
    Qvm {
        vm: Box<qvm::Vm>,
        imports: &'static CallTable,
    },
    QuakeC(QuakeCProgram),
}
pub struct QuakeCProgram {
    vm: Box<quakec::Vm>,
    self_global: Option<usize>,
    time_global: Option<usize>,
}
impl Program {
    pub fn quakec(vm: quakec::Vm) -> Self {
        let self_global = vm.image.global(b"self");
        let time_global = vm.image.global(b"time");
        Self::QuakeC(QuakeCProgram {
            vm: Box::new(vm),
            self_global,
            time_global,
        })
    }
    fn entry(&self) -> fn(&mut FrameHost, ModuleId, u32, CallbackCall) -> bool {
        match self {
            Self::Qvm { .. } => qvm_entry,
            Self::QuakeC(_) => quakec_entry,
        }
    }
}
pub struct ModuleRequest {
    pub context: CallContext,
    pub phase: Phase,
    pub timing_rules: RuleSetId,
    pub rate: TickRate,
    pub anchor: EntityId,
    pub program: Program,
    pub entries: Vec<u32>,
    pub frame: CallbackId,
    pub initialize: Option<Export>,
    pub api: Option<VersionCheck>,
    pub shutdown: Option<Export>,
    pub instruction_budget: u64,
    pub configstrings: usize,
    pub files: usize,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Server,
    Client,
}
pub struct VersionCheck {
    pub export: Export,
    pub accepted: &'static [i32],
}
#[derive(Clone, Copy)]
pub enum Argument {
    Word(u64),
    ClockMilliseconds,
    PlatformMilliseconds,
}
#[derive(Clone, Copy)]
pub struct Export {
    pub callback: CallbackId,
    pub arguments: [Argument; 9],
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Pending,
    Running,
    Failed,
    Stopped,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModuleResult {
    Qvm(i32),
    QuakeC([u32; 3]),
}
#[derive(Default, Debug)]
pub struct Counts {
    pub calls: u64,
    pub rejected: u64,
    pub traps: u64,
    pub last_result: Option<ModuleResult>,
}
struct Module {
    request: ModuleRequest,
    storage: ServiceStorage,
    scratch: qa_world::collision::TraceScratch,
    unknown: UnknownCalls,
    counts: Counts,
    state: State,
}
pub struct Modules {
    functions: Arc<FunctionTable<FrameHost>>,
    rows: Box<[Option<Module>]>,
}
impl ThinkWorld for FrameHost {
    fn entities(&mut self) -> &mut qa_world::entities::EntityTable {
        &mut self.runtime.server.entities
    }
}

impl FrameHost {
    /// The caller supplies namespace, scheduling and import-table roles at load.
    pub fn load_modules(
        console: qa_console::commands::Console<Runtime>,
        runtime: Runtime,
        world_rate: TickRate,
        requests: Vec<ModuleRequest>,
    ) -> Result<Self, String> {
        let providers = requests
            .iter()
            .filter(|m| m.phase == Phase::Server)
            .map(|m| Provider {
                module: m.context.module,
                rate: m.rate,
                frame,
                output: None,
            })
            .collect();
        let mut host = Self::load(console, runtime, world_rate, providers)?;
        let functions = FunctionTable::load(requests.iter().map(|m| {
            (
                m.context.module,
                m.timing_rules,
                m.entries
                    .iter()
                    .map(|&entry| FunctionBinding {
                        entry,
                        call: m.program.entry(),
                    })
                    .collect(),
            )
        }))
        .map_err(|e| format!("module functions: {e:?}"))?;
        let capacity = requests
            .iter()
            .map(|m| usize::from(m.context.module.0) + 1)
            .max()
            .unwrap_or(0);
        let mut rows: Box<[Option<Module>]> =
            std::iter::repeat_with(|| None).take(capacity).collect();
        for request in requests {
            let index = usize::from(request.context.module.0);
            let entity = host
                .runtime
                .server
                .entities
                .resolve(request.anchor)
                .ok_or("stale module anchor")?;
            if host.runtime.server.entities.columns.owner[entity] != request.context.module
                || request.frame.0 as usize >= request.entries.len()
                || [
                    request.initialize,
                    request.shutdown,
                    request.api.as_ref().map(|api| api.export),
                ]
                .into_iter()
                .flatten()
                .any(|export| export.callback.0 as usize >= request.entries.len())
                || request.instruction_budget == 0
            {
                return Err("invalid module binding".into());
            }
            if let Program::QuakeC(qc) = &request.program {
                if request
                    .entries
                    .iter()
                    .any(|&entry| entry as usize >= qc.vm.image.functions.len())
                {
                    return Err("invalid QuakeC function".into());
                }
                if qc.self_global.is_some() {
                    native_self(
                        &host.runtime,
                        request.anchor,
                        request.context.module,
                        &qc.vm,
                    )
                    .ok_or("missing QuakeC entity namespace")?;
                }
            }
            let storage = ServiceStorage::load(
                &[(request.context.module, request.configstrings)],
                request.files,
            )
            .map_err(|e| format!("module services: {e:?}"))?;
            let state = if request.initialize.is_some() || request.api.is_some() {
                State::Pending
            } else {
                State::Running
            };
            rows[index] = Some(Module {
                request,
                storage,
                scratch: host.runtime.geometry.scratch(),
                unknown: UnknownCalls::load(256).map_err(|e| e.to_string())?,
                counts: Counts::default(),
                state,
            });
        }
        host.modules = Some(Modules {
            functions: Arc::new(functions),
            rows,
        });
        Ok(host)
    }
    pub fn module_counts(&self, module: ModuleId) -> Option<&Counts> {
        self.modules
            .as_ref()?
            .rows
            .get(module.0 as usize)?
            .as_ref()
            .map(|m| &m.counts)
    }
    pub fn module_state(&self, module: ModuleId) -> Option<State> {
        Some(
            self.modules
                .as_ref()?
                .rows
                .get(module.0 as usize)?
                .as_ref()?
                .state,
        )
    }
    pub fn call_module_export(
        &mut self,
        module: ModuleId,
        export: Export,
        time: ThinkTime,
    ) -> Result<(), CallError> {
        let row = self
            .modules
            .as_ref()
            .and_then(|m| m.rows.get(module.0 as usize))
            .and_then(Option::as_ref)
            .ok_or(CallError::MissingModule)?;
        let entity = row.request.anchor;
        let slot = self
            .runtime
            .server
            .entities
            .resolve(entity)
            .ok_or(CallError::StaleEntity)?;
        if self.runtime.server.entities.columns.owner[slot] != module {
            return Err(CallError::MissingModule);
        }
        let arguments = export.arguments.map(|argument| match argument {
            Argument::Word(word) => word,
            Argument::ClockMilliseconds => time.milliseconds() as u64,
            Argument::PlatformMilliseconds => self.time.milliseconds(),
        });
        self.call_module(
            export.callback,
            CallbackCall::Export {
                entity,
                time,
                arguments,
            },
        )
    }
    /// Startup follows its event/command phase, without another intake.
    pub fn initialize_modules(&mut self, phase: Phase) {
        self.module_lifecycle(Some(phase));
    }
    /// Called for ordinary quit and frame-limit exit; repeating it is a no-op.
    pub fn shutdown_modules(&mut self) {
        self.module_lifecycle(None);
    }
    fn module_lifecycle(&mut self, phase: Option<Phase>) {
        let shutdown = phase.is_none();
        let count = self.modules.as_ref().map_or(0, |m| m.rows.len());
        for index in 0..count {
            let Some(row) = self.modules.as_ref().and_then(|m| m.rows[index].as_ref()) else {
                continue;
            };
            if row.state
                != if shutdown {
                    State::Running
                } else {
                    State::Pending
                }
            {
                continue;
            }
            if phase.is_some_and(|phase| row.request.phase != phase) {
                continue;
            }
            let api = (!shutdown)
                .then_some(row.request.api.as_ref())
                .flatten()
                .map(|api| (api.export, api.accepted));
            let export = if shutdown {
                row.request.shutdown
            } else {
                row.request.initialize
            };
            let time = module_time(row.request.context.clock, self.time);
            let module = ModuleId(index as u16);
            let result = api
                .map_or(Ok(()), |(export, accepted)| {
                    self.call_module_export(module, export, time)?;
                    match self.module_counts(module).and_then(|c| c.last_result) {
                        Some(ModuleResult::Qvm(value)) if accepted.contains(&value) => Ok(()),
                        _ => Err(CallError::Rejected),
                    }
                })
                .and_then(|()| {
                    export.map_or(Ok(()), |export| {
                        self.call_module_export(module, export, time)
                    })
                });
            if let Some(row) = self.modules.as_mut().and_then(|m| m.rows[index].as_mut()) {
                if result.is_err() {
                    row.counts.rejected += 1;
                }
                row.state = if shutdown {
                    State::Stopped
                } else if result.is_ok() {
                    State::Running
                } else {
                    State::Failed
                };
            }
        }
    }
    pub fn client_modules(&mut self) {
        self.initialize_modules(Phase::Client);
        let count = self.modules.as_ref().map_or(0, |m| m.rows.len());
        for index in 0..count {
            let Some(row) = self.modules.as_ref().and_then(|m| m.rows[index].as_ref()) else {
                continue;
            };
            if row.request.phase != Phase::Client {
                continue;
            }
            let time = module_time(row.request.context.clock, self.time);
            invoke_frame(self, ModuleId(index as u16), time);
        }
    }
    /// Providers invoke this at their native phase; it adds no think scan.
    pub fn call_module(
        &mut self,
        callback: CallbackId,
        call: CallbackCall,
    ) -> Result<(), CallError> {
        let functions = Arc::clone(
            &self
                .modules
                .as_ref()
                .ok_or(CallError::MissingModule)?
                .functions,
        );
        functions.invoke(self, callback, call)
    }
}

fn frame(host: &mut FrameHost, tick: Tick) {
    let qa_session::timing::TickTarget::Provider(module) = tick.target else {
        return;
    };
    let Some(row) = host
        .modules
        .as_ref()
        .and_then(|m| m.rows.get(module.0 as usize))
        .and_then(Option::as_ref)
    else {
        return;
    };
    let time = module_time(row.request.context.clock, tick.end);
    invoke_frame(host, module, time);
}
fn invoke_frame(host: &mut FrameHost, module: ModuleId, time: ThinkTime) {
    let Some(row) = host
        .modules
        .as_ref()
        .and_then(|m| m.rows.get(module.0 as usize))
        .and_then(Option::as_ref)
    else {
        return;
    };
    if row.state != State::Running {
        return;
    }
    let entity = row.request.anchor;
    let callback = row.request.frame;
    if host
        .runtime
        .server
        .entities
        .resolve(entity)
        .is_none_or(|slot| host.runtime.server.entities.columns.owner[slot] != module)
    {
        if let Some(row) = host
            .modules
            .as_mut()
            .and_then(|m| m.rows[module.0 as usize].as_mut())
        {
            row.counts.rejected += 1;
        }
        return;
    }
    if host
        .call_module(callback, CallbackCall::Think { entity, time })
        .is_err()
        && let Some(row) = host
            .modules
            .as_mut()
            .and_then(|m| m.rows[module.0 as usize].as_mut())
    {
        row.counts.rejected += 1;
    }
}

fn module_time(clock: EntityTime, time: qa_core::sys_events::EventTime) -> ThinkTime {
    match clock {
        EntityTime::Seconds(_) => ThinkTime::Seconds(time.seconds()),
        EntityTime::Milliseconds(_) => ThinkTime::Milliseconds(time.milliseconds() as i64),
    }
}
fn context(mut context: CallContext, time: ThinkTime) -> CallContext {
    context.clock = match time {
        ThinkTime::Seconds(s) => EntityTime::Seconds(s),
        ThinkTime::Milliseconds(ms) => EntityTime::Milliseconds(ms),
    };
    context
}
fn qvm_entry(host: &mut FrameHost, module: ModuleId, entry: u32, call: CallbackCall) -> bool {
    let (time, words) = match call {
        CallbackCall::Think { time, .. } => {
            let mut arguments = [0; 9];
            arguments[0] = time.milliseconds() as u64;
            (time, arguments)
        }
        CallbackCall::Export {
            time, arguments, ..
        } => (time, arguments),
        _ => return false,
    };
    let Some(row) = host
        .modules
        .as_mut()
        .and_then(|m| m.rows.get_mut(module.0 as usize))
        .and_then(Option::as_mut)
    else {
        return false;
    };
    let Program::Qvm { vm, imports } = &mut row.request.program else {
        return false;
    };
    let mut services =
        host.runtime
            .engine_services(&mut host.console, &mut row.storage, &mut row.scratch);
    let mut calls = QvmCalls {
        services: &mut services,
        table: imports,
        context: context(row.request.context, time),
        platform_time: host.time,
        command: &[],
        unknown: &mut row.unknown,
    };
    let mut arguments = [0; 10];
    arguments[0] = entry as i32;
    for (output, word) in arguments[1..].iter_mut().zip(words) {
        *output = word as u32 as i32;
    }
    row.counts.calls += 1;
    match vm.call(&mut calls, arguments, row.request.instruction_budget, false) {
        Ok(result) => {
            row.counts.last_result = Some(ModuleResult::Qvm(result));
            true
        }
        Err(_) => {
            row.counts.traps += 1;
            false
        }
    }
}
fn native_self(
    runtime: &Runtime,
    entity: EntityId,
    module: ModuleId,
    vm: &quakec::Vm,
) -> Option<u32> {
    let index = runtime.server.entities.resolve(entity)?;
    let native = runtime.server.entities.columns.native_entity[index]?;
    if native.module != module {
        return None;
    }
    let offset = usize::try_from(native.slot).ok()?.checked_mul(vm.stride)?;
    if offset >= vm.entities.len() {
        return None;
    }
    u32::try_from(offset).ok()
}
fn quakec_entry(host: &mut FrameHost, module: ModuleId, entry: u32, call: CallbackCall) -> bool {
    let CallbackCall::Think { entity, time } = call else {
        return false;
    };
    let Some(row) = host
        .modules
        .as_mut()
        .and_then(|m| m.rows.get_mut(module.0 as usize))
        .and_then(Option::as_mut)
    else {
        return false;
    };
    let Program::QuakeC(qc) = &mut row.request.program else {
        return false;
    };
    if let Some(index) = qc.self_global {
        let Some(value) = native_self(&host.runtime, entity, module, &qc.vm) else {
            return false;
        };
        qc.vm.image.globals[index] = value;
    }
    if let Some(index) = qc.time_global {
        qc.vm.image.globals[index] = (time.seconds() as f32).to_bits();
    }
    let mut services =
        host.runtime
            .engine_services(&mut host.console, &mut row.storage, &mut row.scratch);
    let mut calls = QuakeCCalls {
        services: &mut services,
        table: &QUAKEC,
        context: context(row.request.context, time),
        platform_time: host.time,
        developer: host.developer,
        unknown: &mut row.unknown,
    };
    row.counts.calls += 1;
    match qc
        .vm
        .call(&mut calls, entry, row.request.instruction_budget, false)
    {
        Ok(result) => {
            row.counts.last_result = Some(ModuleResult::QuakeC(result));
            true
        }
        Err(_) => {
            row.counts.traps += 1;
            false
        }
    }
}
