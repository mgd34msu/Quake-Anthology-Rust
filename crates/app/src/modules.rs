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
use qa_core::primitives::{
    CallbackCall, CallbackId, ClientId, EntityId, ModuleId, RuleSetId, ThinkTime,
};
use qa_session::{
    dispatch::{
        CallError, FunctionBinding, FunctionTable, ThinkFrame, ThinkWorld, run_think_with_rebind,
    },
    timing::{Tick, TickRate},
};

use std::sync::Arc;

mod load;
pub use load::{Q2Clients, Q2Spec, Q3Role, Q3Spec, QuakeCSpec, load_q2, load_q3, load_quakec};
mod clients;
pub use clients::ClientCommandError;
mod cues;
mod map_start;
mod resources;

pub enum Program {
    Qvm {
        vm: Box<qvm::Vm>,
        imports: &'static CallTable,
    },
    QuakeC(QuakeCProgram),
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    Native {
        vm: Box<qa_compat::native::Vm>,
        imports: &'static CallTable,
    },
}
pub struct QuakeCProgram {
    vm: Box<quakec::Vm>,
    physics: Option<QwPhysics>,
    map: Option<QuakeCMap>,
    duration: Option<(qa_gameplay::rules::FrameDuration, qa_console::cvars::View)>,
}
/// A loaded world and its retained native entity source are paired explicitly.
pub struct QuakeCMap {
    pub world_path: String,
    pub entity_source: usize,
}
struct QwPhysics {
    minimum: qa_core::primitives::CvarHandle,
    maximum: qa_core::primitives::CvarHandle,
    last: Option<ThinkTime>,
}
impl Program {
    pub fn quakec(vm: quakec::Vm, map: Option<QuakeCMap>) -> Self {
        Self::QuakeC(QuakeCProgram {
            vm: Box::new(vm),
            physics: None,
            map,
            duration: None,
        })
    }
    fn entry(&self) -> fn(&mut FrameHost, ModuleId, u32, CallbackCall) -> bool {
        match self {
            Self::Qvm { .. } => word_entry,
            #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
            Self::Native { .. } => word_entry,
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
    pub frame: Export,
    /// Load-selected callbacks, before version checks and game initialization.
    pub prepare: Vec<Export>,
    /// Ordered callbacks after the checked API has been bound.
    pub initialize: Vec<Export>,
    pub api: Option<ApiCheck>,
    pub shutdown: Vec<Export>,
    pub instruction_budget: u64,
    pub configstrings: usize,
    pub files: usize,
    /// Physical spawn source and caller-selected token syntax, bound at load.
    pub entity_source: Option<(usize, qa_formats::entities::EntitySyntax)>,
}
impl ModuleRequest {
    fn sequence(&self, sequence: LifecycleSequence) -> &[Export] {
        match sequence {
            LifecycleSequence::Prepare => &self.prepare,
            LifecycleSequence::Initialize => &self.initialize,
            LifecycleSequence::Shutdown => &self.shutdown,
        }
    }
}
#[derive(Clone, Copy)]
enum LifecycleSequence {
    Prepare,
    Initialize,
    Shutdown,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Server,
    Client,
}
#[derive(Clone, Copy)]
pub enum ApiCheck {
    Version {
        export: Export,
        accepted: &'static [i32],
    },
    NativeTable {
        export: Export,
    },
}
impl ApiCheck {
    fn export(self) -> Export {
        match self {
            Self::Version { export, .. } | Self::NativeTable { export } => export,
        }
    }
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
    pub client: Option<ClientId>,
}
impl Export {
    pub fn words(callback: CallbackId, words: &[u64]) -> Self {
        let mut arguments = [Argument::Word(0); 9];
        for (to, &from) in arguments.iter_mut().zip(words) {
            *to = Argument::Word(from);
        }
        Self {
            callback,
            arguments,
            client: None,
        }
    }

    pub const fn clocked(callback: CallbackId) -> Self {
        let mut arguments = [Argument::Word(0); 9];
        arguments[0] = Argument::ClockMilliseconds;
        Self {
            callback,
            arguments,
            client: None,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Pending,
    Running,
    Failed,
    Stopped,
}
impl State {
    fn can_invoke(self) -> bool {
        matches!(self, Self::Pending | Self::Running)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModuleResult {
    Qvm(i32),
    QuakeC([u32; 3]),
    Native(u64),
}
#[derive(Default, Debug)]
pub struct Counts {
    pub calls: u64,
    pub rejected: u64,
    pub traps: u64,
    pub last_result: Option<ModuleResult>,
    /// Native QC physics rows whose client/movetype phase is not yet adopted.
    /// They never pass through a blanket think scan.
    pub unsupported_physics: u64,
}
impl Counts {
    fn record(&mut self, result: Result<ModuleResult, ()>) -> bool {
        match result {
            Ok(value) => {
                self.last_result = Some(value);
                true
            }
            Err(()) => {
                self.traps += 1;
                false
            }
        }
    }
}
struct Module {
    request: ModuleRequest,
    storage: ServiceStorage,
    scratch: qa_world::collision::TraceScratch,
    unknown: UnknownCalls,
    counts: Counts,
    state: State,
    client_api: Option<Q2Clients>,
}
impl Module {
    fn time(&self, queued: qa_core::sys_events::EventTime) -> ThinkTime {
        if matches!(self.request.program, Program::QuakeC(_)) {
            self.request.context.clock
        } else {
            self.request.context.clock.at(queued)
        }
    }
    fn sound_resources(&self) -> Option<qa_core::primitives::IndexSpan> {
        self.storage
            .resources(self.request.context.module)
            .map(|ranges| ranges[1])
    }
}
pub struct Modules {
    functions: Arc<FunctionTable<FrameHost>>,
    rows: Box<[Option<Module>]>,
}
impl Modules {
    fn row(&self, id: ModuleId) -> Option<&Module> {
        self.rows.get(id.0 as usize)?.as_ref()
    }
    fn row_mut(&mut self, id: ModuleId) -> Option<&mut Module> {
        self.rows.get_mut(id.0 as usize)?.as_mut()
    }
}
impl FrameHost {
    fn module_row(&self, id: ModuleId) -> Option<&Module> {
        self.modules.as_ref()?.row(id)
    }
    fn module_row_mut(&mut self, id: ModuleId) -> Option<&mut Module> {
        self.modules.as_mut()?.row_mut(id)
    }
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
        let mut functions = FunctionTable::load(requests.iter().map(|m| {
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
        for mut request in requests {
            let index = usize::from(request.context.module.0);
            let entity = host
                .runtime
                .server
                .entities
                .resolve(request.anchor)
                .ok_or("stale module anchor")?;
            if host.runtime.server.entities.columns.owner[entity] != request.context.module
                || request.frame.callback.0 as usize >= request.entries.len()
                || request
                    .prepare
                    .iter()
                    .chain(&request.initialize)
                    .chain(&request.shutdown)
                    .copied()
                    .chain(request.api.map(ApiCheck::export))
                    .any(|export| export.callback.0 as usize >= request.entries.len())
                || request.instruction_budget == 0
            {
                return Err("invalid module binding".into());
            }
            if let Program::QuakeC(qc) = &mut request.program {
                if request
                    .entries
                    .iter()
                    .any(|&entry| entry as usize >= qc.vm.image.functions.len())
                {
                    return Err("invalid QuakeC function".into());
                }
                if qc.vm.needs_self() || qc.vm.needs_world() {
                    let native = native_self(
                        &host.runtime,
                        request.anchor,
                        request.context.module,
                        &qc.vm,
                    )
                    .ok_or("missing QuakeC entity namespace")?;
                    if qc.vm.needs_world() && native != 0 {
                        return Err("missing QuakeC world namespace".into());
                    }
                }
                qc.vm.bind_thinks(
                    functions.module_binding(request.context.module),
                    &request.entries,
                );
                functions.bind_missing(
                    request.context.module,
                    FunctionBinding {
                        entry: u32::MAX,
                        call: quakec_entry,
                    },
                );
                if request.timing_rules == RuleSetId::QuakeWorld {
                    qc.physics = Some(QwPhysics {
                        minimum: host
                            .console
                            .cvars
                            .find("sv_mintic")
                            .ok_or("missing QW minimum tick cvar")?,
                        maximum: host
                            .console
                            .cvars
                            .find("sv_maxtic")
                            .ok_or("missing QW maximum tick cvar")?,
                        last: None,
                    });
                }
                if request.rate == TickRate::FrameDriven
                    && let Some(policy) = qa_gameplay::rules::frame_duration(request.timing_rules)
                {
                    qc.duration = Some((
                        policy,
                        host.console
                            .cvars
                            .bind(
                                policy.override_cvar,
                                qa_console::views::Context {
                                    source: request.timing_rules,
                                    role: qa_console::views::Role::Engine,
                                    side: qa_console::catalog::Scope::Server,
                                    ..qa_console::views::Context::default()
                                },
                            )
                            .ok_or("missing module frame duration cvar")?,
                    ));
                }
                if let Some(map) = &qc.map {
                    let source = host
                        .runtime
                        .entity_sources
                        .get(map.entity_source)
                        .ok_or("missing QuakeC map entity source")?;
                    if request.entity_source != Some((map.entity_source, source.syntax)) {
                        return Err("unpaired QuakeC map entity source".into());
                    }
                    request.context.clock = ThinkTime::Seconds(1.0);
                }
            }
            #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
            if let Program::Native { vm, .. } = &request.program
                && request
                    .entries
                    .iter()
                    .any(|&entry| !vm.declares_export(entry))
            {
                return Err("invalid native export".into());
            }
            let mut storage = ServiceStorage::load_native(
                &[(
                    request.context.module,
                    request.configstrings,
                    request.context.native_rules,
                )],
                request.files,
                &host.console.cvars,
            )
            .map_err(|e| format!("module services: {e:?}"))?;
            if let Program::QuakeC(qc) = &mut request.program {
                qc.vm
                    .seed_entities(
                        &host.runtime.server.entities,
                        request.context.module,
                        request.context.native_rules,
                        storage.max_clients(&host.console.cvars, request.context.native_rules),
                    )
                    .map_err(|error| format!("QuakeC entity bindings: {error:?}"))?;
            }
            storage
                .bind_lifetime(request.context.module, request.anchor)
                .map_err(|error| format!("module lifetime: {error:?}"))?;
            if let Some((index, syntax)) = request.entity_source {
                let source = host
                    .runtime
                    .entity_sources
                    .get(index)
                    .ok_or("missing module entity source")?;
                storage
                    .bind_entity_text(request.context.module, source.bytes.clone(), syntax)
                    .map_err(|e| format!("module entity source: {e:?}"))?;
            }
            let values = host
                .runtime
                .catalog
                .hud
                .values
                .native(request.context.native_rules);
            match &mut request.program {
                Program::Qvm { vm, .. } => vm
                    .bind_player_values(&values.stats, &values.persistent)
                    .map_err(|e| format!("module player values: {e:?}"))?,
                #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
                Program::Native { vm, .. } => {
                    vm.rebind_configstrings();
                    vm.bind_player_values(&values.stats, &values.persistent)
                        .map_err(|e| format!("module player values: {e:?}"))?;
                }
                Program::QuakeC(_) => {}
            }
            let state = if !request.prepare.is_empty()
                || !request.initialize.is_empty()
                || request.api.is_some()
                || matches!(&request.program, Program::QuakeC(qc) if qc.map.is_some())
            {
                State::Pending
            } else {
                State::Running
            };
            if let Program::QuakeC(qc) = &mut request.program {
                qc.vm.active = state == State::Running;
            }
            rows[index] = Some(Module {
                request,
                storage,
                scratch: host.runtime.geometry.scratch(),
                unknown: UnknownCalls::load(256).map_err(|e| e.to_string())?,
                counts: Counts::default(),
                state,
                client_api: None,
            });
        }
        host.modules = Some(Modules {
            functions: Arc::new(functions),
            rows,
        });
        host.configure_output_resources();
        Ok(host)
    }
    pub fn module_counts(&self, module: ModuleId) -> Option<&Counts> {
        Some(&self.module_row(module)?.counts)
    }
    pub fn last_unknown_call(&self, module: ModuleId) -> Option<u32> {
        self.module_row(module)?.unknown.last
    }
    pub fn module_state(&self, module: ModuleId) -> Option<State> {
        Some(self.module_row(module)?.state)
    }
    /// Resource ordinals are populated by native Init/SpawnEntities at load.
    pub fn for_each_native_sound(
        &self,
        mut visit: impl FnMut(&[u8], RuleSetId) -> Result<(), String>,
    ) -> Result<(), String> {
        if let Some(modules) = &self.modules {
            for row in modules.rows.iter().flatten() {
                if let Some(sounds) = row.sound_resources() {
                    for ordinal in sounds.range().skip(1) {
                        let (name, _) = row
                            .storage
                            .configstring(row.request.context.module, ordinal)
                            .map_err(|e| format!("native sound precache: {e:?}"))?;
                        if !name.is_empty() {
                            visit(name, row.request.context.native_rules)?;
                        }
                    }
                }
            }
        }
        Ok(())
    }
    /// Resolve native ordinals and client cue resources after PCM loading.
    pub fn bind_sounds(&mut self, bank: &Arc<qa_audio::Bank>) -> Result<(), String> {
        self.muzzle_sounds.bind(bank);
        for resources in self.runtime.client_resources.iter_mut().flatten() {
            if !resources.state.bind_sounds(bank) {
                return Err("client sound binding failed".into());
            }
        }
        if let Some(modules) = &mut self.modules {
            for row in modules.rows.iter_mut().flatten() {
                if let Some(sounds) = row.sound_resources() {
                    row.storage
                        .bind_sound_bank(row.request.context.module, sounds, Arc::clone(bank))
                        .map_err(|e| format!("native sound binding: {e:?}"))?;
                }
            }
        }
        self.bind_output_sounds(bank);
        self.publish_module_ambients();
        Ok(())
    }
    pub(crate) fn publish_module_ambients(&mut self) {
        let Some(modules) = &mut self.modules else {
            return;
        };
        for row in modules.rows.iter_mut().flatten() {
            if row.state != State::Running {
                continue;
            }
            let mut context = row.request.context;
            context.clock = row.time(self.time);
            if row
                .storage
                .publish_initial_ambients(&mut self.runtime.server, context)
                .is_err()
            {
                row.counts.rejected += 1;
                row.state = State::Failed;
            }
        }
    }
    /// Read the existing native entity view only at its stopped-child boundary.
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    pub fn native_entities(
        &mut self,
        module: ModuleId,
    ) -> Result<qa_compat::entities::Entities, CallError> {
        let row = self
            .module_row_mut(module)
            .ok_or(CallError::MissingModule)?;
        if row.state != State::Running {
            return Err(CallError::Rejected);
        }
        match &mut row.request.program {
            Program::Native { vm, .. } => vm.entities().map_err(|_| CallError::Rejected),
            _ => Err(CallError::Rejected),
        }
    }

    pub fn call_module_export(
        &mut self,
        module: ModuleId,
        export: Export,
        time: ThinkTime,
    ) -> Result<(), CallError> {
        self.call_module_command(module, export, time, Default::default())
    }

    fn call_module_command(
        &mut self,
        module: ModuleId,
        export: Export,
        time: ThinkTime,
        command: qa_core::primitives::CommandArgs<'_>,
    ) -> Result<(), CallError> {
        let row = self.module_row(module).ok_or(CallError::MissingModule)?;
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
                client: export.client,
                command,
            },
        )
    }
    /// Load-time startup or the frame's command phase uses the same entries.
    pub fn initialize_modules(&mut self, phase: Phase) {
        self.module_lifecycle(Some(phase));
    }
    /// Called for ordinary quit and frame-limit exit; repeating it is a no-op.
    pub fn shutdown_modules(&mut self) {
        self.disconnect_module_clients();
        self.module_lifecycle(None);
    }
    fn module_lifecycle(&mut self, phase: Option<Phase>) {
        let shutdown = phase.is_none();
        let count = self.modules.as_ref().map_or(0, |m| m.rows.len());
        for index in 0..count {
            let Some(row) = self.module_row(ModuleId(index as u16)) else {
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
            let api = (!shutdown).then_some(row.request.api).flatten();
            let time = row.time(self.time);
            let module = ModuleId(index as u16);
            let sequence = if shutdown {
                LifecycleSequence::Shutdown
            } else {
                LifecycleSequence::Prepare
            };
            let result = self.module_sequence(module, sequence, time).and_then(|()| {
                if shutdown {
                    return Ok(());
                }
                api.map_or(Ok(()), |api| self.check_module_api(module, api, time))
                    .and_then(|()| {
                        self.module_sequence(module, LifecycleSequence::Initialize, time)
                    })
                    .and_then(|()| self.initialize_quakec_map(module))
            });
            if let Some(row) = self.module_row_mut(ModuleId(index as u16)) {
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
                if let Program::QuakeC(qc) = &mut row.request.program {
                    qc.vm.active = row.state == State::Running;
                }
            }
        }
    }
    fn check_module_api(
        &mut self,
        module: ModuleId,
        api: ApiCheck,
        time: ThinkTime,
    ) -> Result<(), CallError> {
        self.call_module_export(module, api.export(), time)?;
        let row = self
            .module_row_mut(module)
            .ok_or(CallError::MissingModule)?;
        match api {
            ApiCheck::Version { accepted, .. } => {
                let value = match row.counts.last_result {
                    Some(ModuleResult::Qvm(value)) => Some(value),
                    Some(ModuleResult::Native(value)) => Some(value as u32 as i32),
                    _ => None,
                };
                value
                    .filter(|value| accepted.contains(value))
                    .map(|_| ())
                    .ok_or(CallError::Rejected)
            }
            ApiCheck::NativeTable { .. } => {
                #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
                if let (Program::Native { vm, .. }, Some(ModuleResult::Native(address))) =
                    (&mut row.request.program, row.counts.last_result)
                {
                    return vm.bind_table(address).map_err(|_| CallError::Rejected);
                }
                Err(CallError::Rejected)
            }
        }
    }
    fn module_sequence(
        &mut self,
        module: ModuleId,
        sequence: LifecycleSequence,
        time: ThinkTime,
    ) -> Result<(), CallError> {
        let count = self
            .module_row(module)
            .map_or(0, |row| row.request.sequence(sequence).len());
        for index in 0..count {
            let export = self
                .module_row(module)
                .and_then(|row| row.request.sequence(sequence).get(index))
                .copied()
                .ok_or(CallError::MissingModule)?;
            self.call_module_export(module, export, time)?;
        }
        Ok(())
    }
    pub fn client_modules(&mut self) {
        self.initialize_modules(Phase::Client);
        let count = self.modules.as_ref().map_or(0, |m| m.rows.len());
        for index in 0..count {
            let Some(row) = self.module_row(ModuleId(index as u16)) else {
                continue;
            };
            if row.request.phase != Phase::Client {
                continue;
            }
            let time = row.time(self.time);
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
    let (Some(modules), cvars) = (host.modules.as_mut(), &host.console.cvars) else {
        return;
    };
    let Some(row) = modules.row_mut(module) else {
        return;
    };
    // qsrc advances the SERVER frame number after RunFrame. Loading calls
    // retain zero; completed ticks are visible to later imports and shutdown.
    let _ = row.storage.reset_message(module);
    row.request.context.server_frame = tick.index - 1;
    let quakec = matches!(row.request.program, Program::QuakeC(_));
    let mut step = qa_core::sys_events::EventTime(tick.end.since(tick.start)).seconds();
    if let Program::QuakeC(qc) = &row.request.program
        && let Some((policy, view)) = qc.duration
    {
        let forced = f64::from(cvars.numeric(view).unwrap_or(0.0));
        step = if forced > 0.0 {
            forced
        } else {
            step.clamp(policy.bounds[0], policy.bounds[1])
        };
    }
    // Native sv.time is an accumulator, not the platform's absolute epoch.
    // QW SV_Frame takes a float duration; NQ advances its double after physics.
    let previous = row.request.context.clock;
    let advance = if row.request.timing_rules == RuleSetId::QuakeWorld {
        f64::from(step as f32)
    } else {
        step
    };
    let next = ThinkTime::Seconds(previous.seconds() + advance);
    let time = if quakec {
        if row.request.timing_rules == RuleSetId::QuakeWorld {
            row.request.context.clock = next;
            next
        } else {
            previous
        }
    } else {
        row.request.context.clock.at(tick.end)
    };
    let mut physics_due = true;
    if let Program::QuakeC(qc) = &mut row.request.program
        && let Some(physics) = &mut qc.physics
    {
        let last = *physics.last.get_or_insert(previous);
        step = time.seconds() - last.seconds();
        let minimum = cvars.value_in(physics.minimum, RuleSetId::QuakeWorld);
        let maximum = cvars.value_in(physics.maximum, RuleSetId::QuakeWorld);
        if step < f64::from(minimum) {
            physics_due = false;
        } else {
            if step > f64::from(maximum) {
                step = f64::from(maximum);
            }
            physics.last = Some(time);
        }
    }
    if physics_due && let Program::QuakeC(qc) = &mut row.request.program {
        qc.vm.frame_time(step);
    }
    if physics_due && invoke_frame(host, module, time) && quakec {
        quakec_entity_phase(host, module, time, step);
    }
    if let Some(row) = host.module_row_mut(module) {
        row.request.context.server_frame = tick.index;
        if quakec {
            row.request.context.clock = next;
        }
    }
}
fn invoke_frame(host: &mut FrameHost, module: ModuleId, time: ThinkTime) -> bool {
    let Some(row) = host.module_row(module) else {
        return false;
    };
    if row.state != State::Running {
        return false;
    }
    let export = row.request.frame;
    if host.call_module_export(module, export, time).is_err() {
        if let Some(row) = host.module_row_mut(module) {
            row.counts.rejected += 1;
        }
        false
    } else {
        true
    }
}

/// NONE and NOCLIP belong after StartFrame in the native edict-order pass.
/// Step, toss and pusher thinking belongs at their own motion phase positions,
/// and is explicitly counted as unsupported rather than run prematurely.
fn quakec_entity_phase(host: &mut FrameHost, module: ModuleId, time: ThinkTime, step: f64) {
    let Some(modules) = host.modules.as_ref() else {
        return;
    };
    let functions = Arc::clone(&modules.functions);
    // Native VectorMA takes float host_frametime, while think timing retains
    // its existing double comparison and native float deadline narrowing.
    let motion_step = step as f32;
    let mut cursor = 0;
    loop {
        let Some(row) = host.module_row(module) else {
            return;
        };
        let Program::QuakeC(qc) = &row.request.program else {
            return;
        };
        let next = qc.vm.physics_entity(
            &host.runtime.server.entities,
            &mut cursor,
            row.request.context.native_rules,
        );
        match next {
            Ok(Some(entity)) if entity.phase != quakec::PhysicsPhase::Unsupported => {
                let result = run_think_with_rebind(
                    host,
                    &functions,
                    entity.entity,
                    |owner| {
                        (owner == module).then_some(ThinkFrame::Seconds {
                            now: time.seconds(),
                            step,
                        })
                    },
                    |host, _| {
                        let row = host.module_row(module)?;
                        let Program::QuakeC(qc) = &row.request.program else {
                            return None;
                        };
                        qc.vm
                            .bound_entity(&host.runtime.server.entities, entity.native)
                            .ok()
                            .flatten()
                    },
                );
                if let Some(row) = host.module_row_mut(module) {
                    row.counts.rejected += u64::from(result.rejected);
                }
                if entity.phase == quakec::PhysicsPhase::Noclip
                    && result.rejected == 0
                    && let Some(current) = result.current_entity
                    && quakec_noclip(host, module, current, motion_step).is_err()
                    && let Some(row) = host.module_row_mut(module)
                {
                    row.counts.rejected += 1;
                }
            }
            Ok(Some(_)) => {
                if let Some(row) = host.module_row_mut(module) {
                    row.counts.unsupported_physics += 1;
                }
            }
            Ok(None) => return,
            Err(_) => {
                if let Some(row) = host.module_row_mut(module) {
                    row.counts.rejected += 1;
                }
                // A malformed native row rejects that entity. The monotonic
                // cursor already advanced, so other lifetimes keep running.
            }
        }
    }
}

fn quakec_noclip(
    host: &mut FrameHost,
    module: ModuleId,
    entity: EntityId,
    step: f32,
) -> Result<(), qa_compat::services::CallError> {
    let row = host
        .modules
        .as_mut()
        .and_then(|modules| modules.row_mut(module))
        .ok_or(qa_compat::services::CallError::Entity)?;
    let Program::QuakeC(qc) = &mut row.request.program else {
        return Err(qa_compat::services::CallError::Entity);
    };
    let native = native_self(&host.runtime, entity, module, &qc.vm)
        .ok_or(qa_compat::services::CallError::Entity)?;
    let mut services =
        host.runtime
            .engine_services(&mut host.console, &mut row.storage, &mut row.scratch);
    qc.vm
        .advance_noclip(&mut services, row.request.context, native, step)
}

fn context(mut context: CallContext, time: ThinkTime) -> CallContext {
    context.clock = time;
    context
}
fn word_entry(host: &mut FrameHost, module: ModuleId, entry: u32, call: CallbackCall<'_>) -> bool {
    let (time, words, native_player, command) = match call {
        CallbackCall::Think { time, .. } => {
            let mut arguments = [0; 9];
            arguments[0] = time.milliseconds() as u64;
            (time, arguments, None, Default::default())
        }
        CallbackCall::Export {
            time,
            arguments,
            client,
            command,
            ..
        } => (time, arguments, client, command),
        _ => return false,
    };
    let Some(row) = host
        .modules
        .as_mut()
        .and_then(|modules| modules.row_mut(module))
    else {
        return false;
    };
    if !row.state.can_invoke() {
        return false;
    }
    if matches!(row.request.program, Program::QuakeC(_)) {
        return false;
    }
    let mut services =
        host.runtime
            .engine_services(&mut host.console, &mut row.storage, &mut row.scratch);
    row.counts.calls += 1;
    let call_context = context(row.request.context, time);
    let result = match &mut row.request.program {
        Program::Qvm { vm, imports } => {
            let mut calls = QvmCalls {
                services: &mut services,
                table: imports,
                context: call_context,
                platform_time: host.time,
                command: command.argv,
                unknown: &mut row.unknown,
            };
            let mut arguments = [0; 10];
            arguments[0] = entry as i32;
            for (output, word) in arguments[1..].iter_mut().zip(words) {
                *output = word as u32 as i32;
            }
            let result = vm
                .call(&mut calls, arguments, row.request.instruction_budget, false)
                .map_err(|_| ());
            result.and_then(|value| {
                vm.publish_entities(calls.services, call_context)
                    .map(|()| ModuleResult::Qvm(value))
                    .map_err(|_| ())
            })
        }
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        Program::Native { vm, imports } => {
            let mut calls = qa_compat::native::NativeCalls {
                services: &mut services,
                table: imports,
                context: call_context,
                platform_time: host.time,
                command: command.argv,
                raw_args: command.raw,
                native_player,
                unknown: &mut row.unknown,
            };
            vm.call(&mut calls, entry, &words)
                .map(ModuleResult::Native)
                .map_err(|error| {
                    if !error.recoverable() {
                        row.state = State::Failed;
                        if row.counts.traps == 0 {
                            let _ = (qa_compat::services::ENGINE_CALLS.console_formatted)(
                                calls.services,
                                format_args!(
                                    "module {}: native export {entry}: {error:?}\n",
                                    module.0
                                ),
                            );
                        }
                    }
                })
        }
        Program::QuakeC(_) => return false,
    };
    row.counts.record(result)
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
    let phase = match call {
        CallbackCall::Think { .. } => quakec::CallPhase::Think,
        CallbackCall::Spawn { .. } => quakec::CallPhase::Spawn,
        _ => quakec::CallPhase::Export,
    };
    let think = phase == quakec::CallPhase::Think;
    let (CallbackCall::Think { entity, time }
    | CallbackCall::Spawn { entity, time }
    | CallbackCall::Export { entity, time, .. }) = call
    else {
        return false;
    };
    let Some(row) = host
        .modules
        .as_mut()
        .and_then(|modules| modules.row_mut(module))
    else {
        return false;
    };
    if !row.state.can_invoke() {
        return false;
    }
    let Program::QuakeC(qc) = &mut row.request.program else {
        return false;
    };
    let native = native_self(&host.runtime, entity, module, &qc.vm);
    let world = native_self(&host.runtime, row.request.anchor, module, &qc.vm);
    if qc.vm.prepare_call(native, world, time, phase).is_err() {
        return false;
    }
    let mut services =
        host.runtime
            .engine_services(&mut host.console, &mut row.storage, &mut row.scratch);
    let mut calls = QuakeCCalls {
        services: &mut services,
        table: &QUAKEC,
        // ED_Alloc/ED_Free use sv.time, while QC global time is the chosen
        // think time and can lie ahead within this native physics frame.
        context: if think {
            row.request.context
        } else {
            context(row.request.context, time)
        },
        platform_time: host.time,
        developer: host.developer,
        unknown: &mut row.unknown,
    };
    row.counts.calls += 1;
    let execution = qc
        .vm
        .call(&mut calls, entry, row.request.instruction_budget, false);
    // VM-owned entvars are visible after the call, including admitted writes
    // preceding a call-scoped trap. Importing pose never implicitly relinks.
    let projection = if think {
        qc.vm.project_dirty_entities(calls.services, calls.context)
    } else {
        qc.vm.project_entities(calls.services, calls.context)
    };
    let result = execution.map_err(|_| ()).and_then(|value| {
        projection
            .map(|()| ModuleResult::QuakeC(value))
            .map_err(|_| ())
    });
    row.counts.record(result)
}
