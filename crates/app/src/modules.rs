//! Module entry points enter through session dispatch and the shared services.
use crate::{
    Runtime,
    host::{FrameHost, Provider},
};
use qa_compat::{
    abi::{CallTable, UnknownCalls},
    quakec, qvm,
    services::{CallContext, ServiceStorage, SignonCursor},
};
use qa_core::primitives::{
    CallbackCall, CallbackId, ClientId, EntityId, IndexSpan, ModuleId, Motion, NativeEntityRef,
    RuleSetId, ThinkTime,
};
use qa_session::{
    dispatch::{
        CallError, FunctionBinding, FunctionTable, PreparedNativeThink, ThinkFrame, ThinkWorld,
        run_think_with_rebind,
    },
    timing::{Tick, TickRate},
};

use std::sync::Arc;

mod load;
pub use load::{Q2Clients, Q2Spec, Q3Role, Q3Spec, QuakeCSpec, load_q2, load_q3, load_quakec};
mod clients;
mod contacts;
use clients::ClientLifecycle;
pub use clients::{ClientCommandError, ClientLifecycleInfo};
mod cues;
mod execution;
mod input;
use execution::ModuleDispatch;
pub(crate) use execution::{ExecutionSlot, TraceAccess};
mod map_start;
mod motion;
mod resources;
mod trace;

fn host_clear_feedback(runtime: &mut Runtime, client: ClientId) {
    if let Some(client) = runtime.server.clients.get_mut(client.0 as usize) {
        client.hud.view_feedback = None;
    }
}

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
    map: Option<QuakeCMap>,
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
            map,
        })
    }
    fn imports(&self) -> Option<&'static CallTable> {
        match self {
            Self::Qvm { imports, .. } => Some(imports),
            #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
            Self::Native { imports, .. } => Some(imports),
            Self::QuakeC(_) => None,
        }
    }
    fn command_export(&self) -> Option<qa_compat::abi::CommandExport> {
        let export = self.imports()?.command_export()?;
        match self {
            #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
            Self::Native { vm, .. } => Some(qa_compat::abi::CommandExport {
                entry: vm.export_for_command(export.entry)?,
                ..export
            }),
            _ => Some(export),
        }
    }
    fn consumes_server_commands(&self) -> bool {
        self.imports()
            .is_some_and(CallTable::consumes_server_commands)
    }
    fn produces_client_audio(&self) -> bool {
        self.imports().is_some_and(CallTable::produces_client_audio)
    }
    fn produces_client_view(&self) -> bool {
        self.imports().is_some_and(CallTable::produces_client_view)
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
    ClientMessageSequence,
    ClientExecutedCommand,
    ClientNativeNumber,
}
impl Argument {
    fn requires_startup(self) -> bool {
        matches!(
            self,
            Self::ClientMessageSequence | Self::ClientExecutedCommand | Self::ClientNativeNumber
        )
    }
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
    /// Contacts whose target module has not published a callable native entry.
    pub unsupported_contacts: u64,
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
impl ModuleResult {
    fn claims_command(self) -> bool {
        match self {
            Self::Qvm(value) => value != 0,
            Self::Native(value) => value != 0,
            Self::QuakeC(_) => false,
        }
    }
}
struct Module {
    request: ModuleRequest,
    geometry: Option<qa_core::primitives::GeometryId>,
    scratch: qa_world::collision::TraceScratch,
    unknown: UnknownCalls,
    counts: Counts,
    state: State,
    failure_cleanup: bool,
    client_lifecycle: Option<ClientLifecycle>,
    client_batch: Option<ClientId>,
    client_contacts: qa_core::stamps::StampSet,
    clock: qa_gameplay::rules::FrameClock,
    physics: Option<QwPhysics>,
    toss: Option<qa_gameplay::rules::motion::TossRules>,
    step: Option<(
        qa_gameplay::rules::motion::StepRules,
        qa_gameplay::rules::sweep::LegacySweepRules,
    )>,
    push: Option<qa_gameplay::rules::motion::PushRules>,
    push_undo: Option<qa_session::physics::PushUndo>,
    duration: Option<(qa_gameplay::rules::FrameDuration, qa_console::cvars::View)>,
    signon: IndexSpan,
    client_connection: Option<(ClientId, EntityId)>,
    requires_startup: bool,
    consumes_server_commands: bool,
    produces_client_audio: bool,
    produces_client_view: bool,
    input: Option<qa_compat::abi::ClientInputExports>,
    command: Option<execution::CommandBinding>,
    scene_role: crate::renderer::native::SceneRole,
    collision_role: qa_compat::services::ClientCollision,
}
impl Module {
    fn ready_for_callback(&self, active: bool) -> bool {
        self.state == State::Running
            || (active
                && self.state == State::Pending
                && match &self.request.program {
                    Program::Qvm { vm, .. } => vm.is_active(),
                    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
                    Program::Native { vm, .. } => vm.is_active(),
                    Program::QuakeC(_) => false,
                })
    }
    fn retain_active_batch(batch: &mut Option<ClientId>, server: &qa_session::clients::Server) {
        *batch = batch.filter(|id| {
            server
                .clients
                .get(id.0 as usize)
                .is_some_and(|client| client.is_active())
        });
    }
    fn time(&self, queued: qa_core::sys_events::EventTime) -> ThinkTime {
        if self.request.phase == Phase::Server && self.clock.accumulate {
            self.request.context.clock
        } else {
            self.request.context.clock.at(queued)
        }
    }
}
pub struct Modules {
    functions: Arc<FunctionTable<FrameHost, ModuleDispatch>>,
    rows: Box<[Option<Module>]>,
    storage: ServiceStorage,
    signon: Box<[SignonCursor]>,
    input: [input::Recipients; qa_core::sys_events::SeatId::COUNT],
}
impl Modules {
    pub(crate) fn has_module(&self, module: ModuleId) -> bool {
        self.row(module).is_some()
    }
    pub(crate) fn has_running_server(&self) -> bool {
        self.rows
            .iter()
            .flatten()
            .any(|row| row.state == State::Running && row.request.phase == Phase::Server)
    }
    /// Cold precaching follows loaded producer roles, independently of the map.
    pub(crate) fn has_sound_source(&self, rules: RuleSetId) -> bool {
        self.rows
            .iter()
            .flatten()
            .any(|row| row.state.can_invoke() && row.request.context.native_rules == rules)
    }
    pub(crate) fn bound_sound(
        &self,
        module: ModuleId,
        ordinal: u32,
    ) -> Option<qa_core::primitives::SoundId> {
        (self.row(module)?.state == State::Running)
            .then(|| self.storage.bound_sound(module, ordinal))
            .flatten()
    }
    fn row(&self, id: ModuleId) -> Option<&Module> {
        self.rows.get(id.0 as usize)?.as_ref()
    }
    fn row_mut(&mut self, id: ModuleId) -> Option<&mut Module> {
        self.rows.get_mut(id.0 as usize)?.as_mut()
    }
    fn parts_mut(&mut self, id: ModuleId) -> Option<(&mut Module, &mut ServiceStorage)> {
        let row = self.rows.get_mut(id.0 as usize)?.as_mut()?;
        Some((row, &mut self.storage))
    }
    fn trace_parts_mut<'a>(
        &'a mut self,
        id: ModuleId,
        access: execution::TraceAccess<'a>,
    ) -> Option<(&'a mut Module, &'a mut ServiceStorage, trace::TraceView<'a>)> {
        let (before, current) = self.rows.split_at_mut_checked(id.0 as usize)?;
        let (current, after) = current.split_first_mut()?;
        Some((
            current.as_mut()?,
            &mut self.storage,
            trace::TraceView::rows(before, after, access),
        ))
    }
}
impl FrameHost {
    pub(crate) fn retire_stale_modules(&mut self) {
        let count = self
            .modules
            .as_ref()
            .map_or(0, |modules| modules.rows.len());
        for index in 0..count {
            let module = ModuleId(index as u16);
            let Some(row) = self.module_row(module).filter(|row| row.state.can_invoke()) else {
                continue;
            };
            let producer = qa_core::primitives::CommandProducer {
                module,
                anchor: row.request.anchor,
            };
            if qa_console::commands::Host::command_producer_live(&self.runtime, producer) {
                continue;
            }
            if let Some(row) = self.module_row_mut(module) {
                row.state = State::Failed;
                row.counts.rejected += 1;
            }
            self.cleanup_failed_module(module);
        }
    }

    fn claim_failed_module(&mut self, module: ModuleId) -> bool {
        let Some(row) = self.module_row_mut(module) else {
            return false;
        };
        if row.state != State::Failed || row.failure_cleanup {
            return false;
        }
        row.failure_cleanup = true;
        true
    }
    fn cleanup_failed_module(&mut self, module: ModuleId) {
        if self.claim_failed_module(module) {
            self.disconnect_module_recipients(module);
            self.retire_module_state(module);
        }
    }

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

    fn prepare_native_think(
        &mut self,
        entity: EntityId,
        subject: Option<NativeEntityRef>,
        time: ThinkTime,
    ) -> Result<Option<PreparedNativeThink>, CallError> {
        let slot = self
            .runtime
            .server
            .entities
            .resolve(entity)
            .ok_or(CallError::StaleEntity)?;
        let module = self.runtime.server.entities.columns.owner[slot];
        let row = self.module_row(module).ok_or(CallError::MissingModule)?;
        if !matches!(&row.request.program, Program::QuakeC(_)) {
            return Ok(None);
        }
        let subject = subject
            .map_or_else(|| contacts::reference(self, entity), Ok)
            .map_err(|_| CallError::Rejected)?;
        edit_quakec(
            self,
            module,
            crate::modules::execution::TraceAccess::empty(),
            |vm, services, context| {
                if !vm.is_running() {
                    motion::sync_attachments(vm, services, context)?;
                }
                let native = vm.weak_entity_offset(context.module, subject)?;
                let binding = vm.prepare_think(native, time);
                let published = vm.project_entity(services, context, native);
                Ok(PreparedNativeThink {
                    subject,
                    binding: binding.and_then(|binding| published.map(|()| binding))?,
                })
            },
        )
        .map(Some)
        .map_err(|_| CallError::Rejected)
    }
}

impl FrameHost {
    /// The caller supplies namespace, scheduling and import-table roles at load.
    pub fn load_modules(
        console: qa_console::commands::Console<Runtime>,
        mut runtime: Runtime,
        world_rate: TickRate,
        requests: Vec<ModuleRequest>,
    ) -> Result<Self, String> {
        let mut requests = requests;
        for request in &mut requests {
            if std::iter::once(request.frame)
                .chain(request.prepare.iter().copied())
                .chain(request.initialize.iter().copied())
                .chain(request.shutdown.iter().copied())
                .chain(request.api.map(ApiCheck::export))
                .any(|export| export.callback.0 as usize >= request.entries.len())
            {
                return Err("invalid module binding".into());
            }
            if let Some(export) = request.program.command_export()
                && !request.entries.contains(&export.entry)
            {
                request
                    .entries
                    .try_reserve(1)
                    .map_err(|_| "module command entry reservation")?;
                request.entries.push(export.entry);
            }
        }
        runtime.prepare_traces(requests.iter().map(|request| request.context.trace_rules))?;
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
        let (names, bytes) =
            requests
                .iter()
                .try_fold((0usize, 0usize), |(names, bytes), request| {
                    let Program::QuakeC(qc) = &request.program else {
                        return Ok((names, bytes));
                    };
                    let (additional_names, additional_bytes) = qc
                        .vm
                        .name_history_reservation()
                        .map_err(|error| format!("QuakeC name reservation: {error:?}"))?;
                    Ok::<_, String>((
                        names
                            .checked_add(additional_names)
                            .ok_or("QuakeC name capacity")?,
                        bytes
                            .checked_add(additional_bytes)
                            .ok_or("QuakeC name byte capacity")?,
                    ))
                })?;
        if names != 0 || bytes != 0 {
            host.runtime
                .catalog
                .names
                .reserve(names, bytes)
                .map_err(|error| format!("QuakeC name reservation: {error}"))?;
        }
        let execution_capacity = requests.iter().try_fold(0usize, |total, request| {
            let count = match &request.program {
                Program::Qvm { vm, .. } => vm.activation_capacity(),
                #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
                Program::Native { vm, .. } => vm.activation_capacity(),
                Program::QuakeC(qc) => qc.vm.activation_capacity(),
            };
            total.checked_add(count).ok_or("module execution capacity")
        })?;
        // A completed backend may wait for one claimed client-lifetime or
        // failed-module cleanup. These owners cannot be claimed twice.
        let execution_capacity = execution_capacity
            .checked_mul(2)
            .and_then(|capacity| capacity.checked_add(host.runtime.server.clients.len()))
            .and_then(|capacity| capacity.checked_add(requests.len()))
            .and_then(|capacity| capacity.checked_add(1))
            .ok_or("module cleanup execution capacity")?;
        let execution = execution::ExecutionSlot::reserve(execution_capacity)?;
        let mut functions = FunctionTable::load(requests.iter().map(|m| {
            (
                m.context.module,
                m.timing_rules,
                m.entries
                    .iter()
                    .map(|&entry| FunctionBinding {
                        entry,
                        call: execution::start,
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
        let client_capacity = host.runtime.server.clients.len();
        let signon_capacity = requests
            .len()
            .checked_mul(client_capacity)
            .filter(|&capacity| capacity <= u32::MAX as usize)
            .ok_or("module signon capacity")?;
        let signon = std::iter::repeat_with(SignonCursor::default)
            .take(signon_capacity)
            .collect();
        let services: Vec<_> = requests
            .iter()
            .map(|request| {
                (
                    request.context.module,
                    request.configstrings,
                    request.context.native_rules,
                    request.files,
                )
            })
            .collect();
        let mut storage = ServiceStorage::load_modules(&services, &host.console.cvars)
            .map_err(|e| format!("module services: {e:?}"))?;
        for (ordinal, mut request) in requests.into_iter().enumerate() {
            let index = usize::from(request.context.module.0);
            let entity = host
                .runtime
                .server
                .entities
                .resolve(request.anchor)
                .ok_or("stale module anchor")?;
            if host.runtime.server.entities.columns.owner[entity] != request.context.module
                || request.instruction_budget == 0
            {
                return Err("invalid module binding".into());
            }
            request.context.console.producer = Some(qa_console::views::CommandProducer {
                module: request.context.module,
                anchor: request.anchor,
            });
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
                        call: execution::start,
                    },
                );
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
            let clock = qa_gameplay::rules::frame_clock(request.timing_rules);
            let physics = if request.timing_rules == RuleSetId::QuakeWorld {
                Some(QwPhysics {
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
                })
            } else {
                None
            };
            let duration = if request.rate == TickRate::FrameDriven {
                qa_gameplay::rules::frame_duration(request.timing_rules)
                    .map(|policy| {
                        let view = host
                            .console
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
                            .ok_or("missing module frame duration cvar")?;
                        Ok::<_, &str>((policy, view))
                    })
                    .transpose()?
            } else {
                None
            };
            #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
            if let Program::Native { vm, .. } = &request.program
                && request
                    .entries
                    .iter()
                    .any(|&entry| !vm.declares_export(entry))
            {
                return Err("invalid native export".into());
            }
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
                    .bind_player_values(values.bindings(), &host.runtime.catalog.registry)
                    .map_err(|e| format!("module player values: {e:?}"))?,
                #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
                Program::Native { vm, .. } => {
                    vm.rebind_configstrings();
                    vm.bind_player_values(values.bindings(), &host.runtime.catalog.registry)
                        .map_err(|e| format!("module player values: {e:?}"))?;
                }
                Program::QuakeC(qc) => {
                    qc.vm
                        .bind_player_values(&values.stats, values.items)
                        .map_err(|e| format!("module player values: {e:?}"))?;
                    qc.vm
                        .bind_client_settings(&host.console.cvars, &host.runtime.catalog.registry)
                        .map_err(|e| format!("module client settings: {e:?}"))?;
                }
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
            let client_connection = if request.phase == Phase::Client {
                request
                    .frame
                    .client
                    .map(|id| {
                        let client = host
                            .runtime
                            .server
                            .clients
                            .get(id.0 as usize)
                            .filter(|client| client.connection.is_some())
                            .ok_or("missing CLIENT module connection")?;
                        if host
                            .runtime
                            .server
                            .entities
                            .resolve(client.entity)
                            .is_none()
                        {
                            return Err("stale CLIENT module connection");
                        }
                        Ok((id, client.entity))
                    })
                    .transpose()?
            } else {
                None
            };
            let requires_startup = request.initialize.iter().any(|export| {
                export
                    .arguments
                    .iter()
                    .any(|&argument| argument.requires_startup())
            });
            if requires_startup && client_connection.is_none() {
                return Err("CLIENT startup arguments require a bound connection".into());
            }
            let consumes_server_commands = request.program.consumes_server_commands();
            let produces_client_audio = request.program.produces_client_audio();
            let produces_client_view = request.program.produces_client_view();
            let command = execution::CommandBinding::load(&request)?;
            let input = request
                .program
                .imports()
                .and_then(|table| table.input_exports());
            let q3_clients = request
                .program
                .imports()
                .filter(|imports| std::ptr::eq(*imports, &qa_compat::abi::Q3_SERVER))
                .map(|_| {
                    storage
                        .max_clients(&host.console.cvars, RuleSetId::Quake3)
                        .clamp(0, 64) as usize
                });
            if let Some(capacity) = q3_clients {
                storage
                    .bind_client_capacity(request.context.module, capacity)
                    .map_err(|e| format!("module client capacity: {e:?}"))?;
            }
            let client_lifecycle = match &request.program {
                Program::QuakeC(qc)
                    if request.phase == Phase::Server
                        && matches!(
                            request.context.native_rules,
                            RuleSetId::Quake | RuleSetId::QuakeWorld
                        ) =>
                {
                    Some(ClientLifecycle::quakec(
                        request.context.module,
                        request.context.native_rules,
                        qc.vm
                            .client_count()
                            .map_err(|e| format!("module client reservation: {e:?}"))?,
                    ))
                }
                _ if request.phase == Phase::Server
                    && request.context.native_rules == RuleSetId::Quake3
                    && q3_clients.is_some()
                    && (2..=7).all(|export| request.entries.contains(&export)) =>
                {
                    Some(ClientLifecycle::q3(
                        request.context.module,
                        q3_clients.ok_or("missing Q3 client capacity")?,
                    ))
                }
                _ => None,
            };
            if let Some(lifecycle) = client_lifecycle {
                storage
                    .bind_client_capacity(request.context.module, lifecycle.info.clients)
                    .map_err(|e| format!("module client capacity: {e:?}"))?;
            }
            let toss = qa_gameplay::rules::motion::toss_rules(request.context.native_rules);
            let step = qa_gameplay::rules::motion::step_rules(request.context.native_rules).zip(
                qa_gameplay::rules::sweep::entity_sweep(request.context.native_rules),
            );
            let push = matches!(&request.program, Program::QuakeC(_))
                .then(|| qa_gameplay::rules::motion::push_rules(request.context.native_rules))
                .flatten();
            let push_undo = if let (Some(_), Program::QuakeC(qc)) = (push, &request.program) {
                let maximum_candidates = (qc.vm.entities.len() / qc.vm.stride)
                    .checked_add(host.runtime.server.entities.capacity())
                    .ok_or("push undo capacity")?;
                Some(qa_session::physics::PushUndo::load(maximum_candidates))
            } else {
                None
            };
            rows[index] = Some(Module {
                request,
                geometry: host.runtime.collision.as_ref().map(|world| world.geometry),
                scratch: host.runtime.geometry.scratch(),
                unknown: UnknownCalls::load(256).map_err(|e| e.to_string())?,
                counts: Counts::default(),
                state,
                failure_cleanup: false,
                client_lifecycle,
                client_batch: None,
                client_contacts: qa_core::stamps::StampSet::new(
                    host.runtime.server.entities.capacity(),
                ),
                client_connection,
                requires_startup,
                consumes_server_commands,
                produces_client_audio,
                produces_client_view,
                input,
                command,
                scene_role: Default::default(),
                collision_role: Default::default(),
                clock,
                physics,
                toss,
                step,
                push,
                push_undo,
                duration,
                signon: IndexSpan {
                    first: (ordinal * client_capacity) as u32,
                    count: client_capacity as u32,
                },
            });
        }
        let input = input::recipients(&rows);
        host.retain_command_producers(|producer| {
            rows.get(usize::from(producer.module.0))
                .and_then(Option::as_ref)
                .is_some_and(|row| row.request.anchor == producer.anchor)
        });
        host.modules = Some(Modules {
            functions: Arc::new(functions),
            rows,
            storage,
            signon,
            input,
        });
        host.execution = execution;
        host.bind_client_presentation()?;
        host.configure_output_resources();
        host.update_console_policy(false);
        Ok(host)
    }
    fn bind_client_presentation(&mut self) -> Result<(), String> {
        let Some(modules) = self.modules.as_ref() else {
            return Ok(());
        };
        for row in modules.rows.iter().flatten() {
            if !row.produces_client_audio && !row.produces_client_view {
                continue;
            }
            let Some((client, lifetime)) = row.client_connection else {
                continue;
            };
            let binding = self
                .runtime
                .snapshots
                .get_mut(client.0 as usize)
                .and_then(Option::as_mut)
                .ok_or("missing CLIENT audio publisher")?;
            if row.produces_client_audio {
                binding
                    .attach_client_audio(
                        row.request.context.module,
                        lifetime,
                        row.request.context.native_rules,
                        &mut self.runtime.server.events,
                        self.runtime.server.presentation,
                    )
                    .map_err(|error| format!("CLIENT audio binding: {error:?}"))?;
            }
            if row.produces_client_view {
                binding.attach_client_view(row.request.context.module, lifetime);
                host_clear_feedback(&mut self.runtime, client);
            }
        }
        Ok(())
    }
    fn retire_module_state(&mut self, module: ModuleId) {
        self.retire_module_input(module);
        let Some(row) = self.module_row(module) else {
            return;
        };
        let (audio, view, connection, time, producer) = (
            row.produces_client_audio,
            row.produces_client_view,
            row.client_connection,
            row.time(self.time),
            qa_console::views::CommandProducer {
                module,
                anchor: row.request.anchor,
            },
        );
        self.retain_command_producers(|current| current != producer);
        if let Some(row) = self.module_row_mut(module) {
            row.collision_role.reset();
        }
        self.runtime.server.retire_module_entities(module, time);
        if let Some(modules) = &mut self.modules {
            let _ = modules
                .storage
                .retire_music(module, &mut self.runtime.server.events);
        }
        self.update_console_policy(false);
        let Some((client, lifetime)) = connection else {
            return;
        };
        if let Some(binding) = self
            .runtime
            .snapshots
            .get_mut(client.0 as usize)
            .and_then(Option::as_mut)
        {
            if audio {
                binding.detach_client_audio(
                    module,
                    lifetime,
                    &mut self.runtime.server.events,
                    self.runtime.server.presentation,
                );
            }
            if view {
                binding.detach_client_view(module, lifetime);
                host_clear_feedback(&mut self.runtime, client);
            }
        }
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
                if let Some(sounds) = modules
                    .storage
                    .resources(row.request.context.module)
                    .map(|ranges| ranges[1])
                {
                    for ordinal in sounds.range().skip(1) {
                        let (name, _) = modules
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
        self.muzzle_sounds
            .bind(bank)
            .map_err(|e| format!("muzzle sound binding: {e:?}"))?;
        for resources in self.runtime.client_resources.iter_mut().flatten() {
            if !resources.state.bind_sounds(bank) {
                return Err("client sound binding failed".into());
            }
        }
        if let Some(modules) = &mut self.modules {
            for row in modules.rows.iter_mut().flatten() {
                if let Some(sounds) = modules
                    .storage
                    .resources(row.request.context.module)
                    .map(|ranges| ranges[1])
                {
                    modules
                        .storage
                        .bind_sound_bank(row.request.context.module, sounds, Arc::clone(bank))
                        .map_err(|e| format!("native sound binding: {e:?}"))?;
                }
            }
        }
        self.bind_output_sounds(bank);
        self.publish_module_signon();
        Ok(())
    }
    pub(crate) fn publish_module_signon(&mut self) {
        let Some(modules) = &mut self.modules else {
            return;
        };
        for row in modules.rows.iter_mut().flatten() {
            if row.state != State::Running {
                continue;
            }
            let mut context = row.request.context;
            context.clock = row.time(self.time);
            for (index, cursor) in modules.signon[row.signon.range()].iter_mut().enumerate() {
                let id = ClientId(index as u32);
                let client = &self.runtime.server.clients[index];
                let Some(output) = client.output else {
                    continue;
                };
                if !client.is_active()
                    || client.connection == Some(qa_session::clients::Connection::Bot)
                    || self.runtime.server.events.needs_resync(output)
                    || self
                        .runtime
                        .network
                        .get(id, qa_core::loopback::Endpoint::Server)
                        .is_none_or(|connection| connection.output != Some(output))
                {
                    continue;
                }
                if modules
                    .storage
                    .publish_signon(&mut self.runtime.server, context, id, cursor)
                    .is_err()
                {
                    row.counts.rejected += 1;
                }
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

    fn client_startup(
        &self,
        connection: Option<(ClientId, EntityId)>,
    ) -> Option<qa_network::channel::commands::Q3Startup> {
        let (id, lifetime) = connection?;
        let client = self.runtime.server.clients.get(id.0 as usize)?;
        if client.connection.is_none()
            || client.entity != lifetime
            || self.runtime.snapshots[id.0 as usize]
                .as_ref()
                .is_some_and(|binding| binding.presentation_closed())
            || self.runtime.server.entities.resolve(lifetime).is_none()
        {
            return None;
        }
        self.runtime
            .network
            .get(id, qa_core::loopback::Endpoint::Client)?
            .channel
            .q3_startup()
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
        let mut cached_startup = None;
        let mut startup = || {
            (*cached_startup.get_or_insert_with(|| self.client_startup(row.client_connection)))
                .ok_or(CallError::Rejected)
        };
        let mut arguments = [0; 9];
        for (word, argument) in arguments.iter_mut().zip(export.arguments) {
            *word = match argument {
                Argument::Word(word) => word,
                Argument::ClockMilliseconds => time.milliseconds() as u64,
                Argument::PlatformMilliseconds => self.time.milliseconds(),
                Argument::ClientMessageSequence => u64::from(startup()?.message_sequence),
                Argument::ClientExecutedCommand => {
                    u64::from(startup()?.last_executed_server_command)
                }
                Argument::ClientNativeNumber => u64::from(startup()?.client_num),
            };
        }
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
    /// Pending queued startup is not a completed registration boundary.
    pub fn registration_pending(&self, phase: Phase) -> Result<bool, String> {
        let Some(modules) = &self.modules else {
            return Ok(false);
        };
        let mut pending = false;
        for row in modules
            .rows
            .iter()
            .flatten()
            .filter(|row| row.request.phase == phase)
        {
            if row.state == State::Failed {
                let module = row.request.context.module;
                let detail = self
                    .last_unknown_call(module)
                    .map_or_else(String::new, |number| {
                        format!("; last unsupported import {number}")
                    });
                return Err(format!("module {} failed initialization{detail}", module.0));
            }
            pending |= row.state == State::Pending;
        }
        Ok(pending)
    }
    /// Keep the cold asset owners live while the ordinary Com_Frame packet
    /// path delivers startup. This never adds a physical intake point.
    pub fn register_client_modules(
        &mut self,
        source: &mut impl crate::host::FrameSource,
        timeout: std::time::Duration,
    ) -> Result<(), String> {
        self.initialize_modules(Phase::Client);
        let started = qa_platform::Stopwatch::start();
        while self.registration_pending(Phase::Client)? && !self.runtime.quit {
            if started.elapsed() >= timeout {
                return Err("client modules are still waiting for queued startup state".into());
            }
            self.frame(source, false);
        }
        Ok(())
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
            if !shutdown
                && row.requires_startup
                && self.client_startup(row.client_connection).is_none()
            {
                // Await the receiving channel's completed gamestate, without
                // trapping or claiming resources from SERVER state.
                continue;
            }
            self.module_lifecycle_row(ModuleId(index as u16), shutdown);
        }
        self.update_console_policy(false);
    }
    /// Stop one module without stopping the shared frame loop or other providers.
    pub fn shutdown_module(&mut self, module: ModuleId) -> bool {
        if self.module_state(module) != Some(State::Running) {
            return false;
        }
        self.module_lifecycle_row(module, true);
        true
    }
    fn module_lifecycle_row(&mut self, module: ModuleId, shutdown: bool) {
        if shutdown {
            self.disconnect_module_recipients(module);
        }
        let Some(row) = self.module_row(module) else {
            return;
        };
        let api = (!shutdown).then_some(row.request.api).flatten();
        let time = row.time(self.time);
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
                .and_then(|()| self.module_sequence(module, LifecycleSequence::Initialize, time))
                .and_then(|()| self.initialize_quakec_map(module))
        });
        if let Some(row) = self.module_row_mut(module) {
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
        if shutdown {
            self.retire_module_state(module);
        } else if result.is_err() {
            self.cleanup_failed_module(module);
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
        self.client_configurations();
        // Init registers into these owners. Draw exports may use only the
        // published catalog and scene consumer, after registration completes.
        if self
            .runtime
            .model_assets
            .as_ref()
            .is_some_and(|owner| owner.initial_registration())
            || self.runtime.sound_loader.is_some()
        {
            return;
        }
        let count = self.modules.as_ref().map_or(0, |m| m.rows.len());
        // Display overlays follow every native client view, independently of
        // request registration order. Built-in seats are placed behind these
        // commands by the same scene packet's background composition.
        for view_producer in [true, false] {
            for index in 0..count {
                let Some(row) = self.module_row(ModuleId(index as u16)) else {
                    continue;
                };
                if row.request.phase != Phase::Client || row.produces_client_view != view_producer {
                    continue;
                }
                let time = if let Some((id, lifetime)) =
                    row.client_connection.filter(|_| row.produces_client_view)
                {
                    let Some(_) = self
                        .runtime
                        .server
                        .clients
                        .get(id.0 as usize)
                        .filter(|client| {
                            client.connection.is_some()
                                && !self.runtime.snapshots[id.0 as usize]
                                    .as_ref()
                                    .is_some_and(|binding| binding.presentation_closed())
                                && client.entity == lifetime
                                && self.runtime.server.entities.resolve(lifetime).is_some()
                        })
                    else {
                        continue;
                    };
                    let Some(sample) = self.runtime.snapshots[id.0 as usize]
                        .as_ref()
                        .and_then(|snapshot| snapshot.interpolation())
                    else {
                        // Bound world views wait for an active native sample;
                        // queued uptime belongs to unbound UI/overlay clocks.
                        continue;
                    };
                    sample.time
                } else {
                    row.time(self.time)
                };
                invoke_frame(self, ModuleId(index as u16), time);
            }
        }
    }
    fn client_configurations(&mut self) {
        let team_chats_only = self
            .console
            .cvars
            .integer_in(self.team_chats_only, RuleSetId::Quake3)
            != 0;
        for index in 0..self.runtime.server.clients.len() {
            let client = &self.runtime.server.clients[index];
            if client.connection.is_none() {
                continue;
            }
            let id = ClientId(index as u32);
            let lifetime = client.entity;
            let bound = self.modules.as_ref().is_some_and(|modules| {
                modules.rows.iter().flatten().any(|row| {
                    row.state.can_invoke()
                        && row.consumes_server_commands
                        && row.client_connection == Some((id, lifetime))
                })
            });
            if bound
                || self
                    .runtime
                    .network
                    .get(id, qa_core::loopback::Endpoint::Client)
                    .is_none_or(|connection| connection.channel.command_state().is_none())
            {
                continue;
            }
            let disconnected = if let Some((mut view, server)) =
                self.runtime.client_connection_parts(id, lifetime)
            {
                let _ = view.consume_configuration(server, team_chats_only);
                view.termination
            } else {
                None
            };
            if let Some(termination) = disconnected {
                self.finish_client_termination(id, termination);
            }
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
        let module = self
            .runtime
            .server
            .entities
            .resolve(call.entity())
            .map(|slot| self.runtime.server.entities.columns.owner[slot]);
        let result = functions.invoke(self, callback, call);
        if let Some(module) = module {
            self.cleanup_failed_module(module);
        }
        result
    }
}

fn frame(host: &mut FrameHost, tick: Tick) {
    let qa_session::timing::TickTarget::Provider(module) = tick.target else {
        return;
    };
    let (Some(modules), cvars) = (host.modules.as_mut(), &host.console.cvars) else {
        return;
    };
    let Some((row, storage)) = modules.parts_mut(module) else {
        return;
    };
    // qsrc advances the SERVER frame number after RunFrame. Loading calls
    // retain zero; completed ticks are visible to later imports and shutdown.
    let _ = storage.reset_message(module);
    row.request.context.server_frame = tick.index - 1;
    let quakec = matches!(row.request.program, Program::QuakeC(_));
    let mut step = qa_core::sys_events::EventTime(tick.end.since(tick.start)).seconds();
    if let Some((policy, view)) = row.duration {
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
    let advance = if row.clock.narrow_span {
        f64::from(step as f32)
    } else {
        step
    };
    let next = ThinkTime::Seconds(previous.seconds() + advance);
    let time = if row.clock.accumulate {
        if row.clock.advance_before {
            row.request.context.clock = next;
            next
        } else {
            previous
        }
    } else {
        row.request.context.clock.at(tick.end)
    };
    let mut physics_due = true;
    if let Some(physics) = &mut row.physics {
        let last = *physics.last.get_or_insert(previous);
        step = time.seconds() - last.seconds();
        let minimum = cvars.value_in(physics.minimum, row.request.timing_rules);
        let maximum = cvars.value_in(physics.maximum, row.request.timing_rules);
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
    if physics_due {
        host.prepare_module_client_commands(module, step);
    }
    if physics_due && invoke_frame(host, module, time) && quakec {
        quakec_entity_phase(host, module, time, step);
    }
    if let Some(row) = host.module_row_mut(module) {
        row.request.context.server_frame = tick.index;
        if row.clock.accumulate {
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

/// Native motion belongs after StartFrame in the native edict-order pass.
/// Each motion phase places the shared think dispatcher at its native position.
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
        let next = qc
            .vm
            .physics_entity(&host.runtime.server.entities, &mut cursor);
        if let Ok(Some(entity)) = &next
            && let Some(id) = host.runtime.server.client_for_native(entity.native)
            && host.runtime.server.module_moves(id) == Some(entity.native)
        {
            match clients::quakec::frame(host, module, id, time, step) {
                Ok(true) => {}
                Ok(false) => {
                    if let Some(row) = host.module_row_mut(module) {
                        row.counts.unsupported_physics += 1;
                    }
                }
                Err(_) => {
                    if let Some(row) = host.module_row_mut(module) {
                        row.counts.rejected += 1;
                    }
                }
            }
            continue;
        }
        match next {
            Ok(Some(entity)) if entity.phase == Motion::Push => {
                if motion::push(host, module, entity.entity, time, step).is_err()
                    && let Some(row) = host.module_row_mut(module)
                {
                    row.counts.rejected += 1;
                }
            }
            Ok(Some(entity)) if entity.phase == Motion::Step => {
                let execution = motion::step_fall(host, module, entity.entity, time, step)
                    .and_then(|current| {
                        if let Some(current) = current
                            && let Some(current) =
                                entity_think(host, module, &functions, &entity, current, time, step)
                        {
                            motion::water_transition(host, module, current, time)?;
                        }
                        Ok(())
                    });
                if execution.is_err()
                    && let Some(row) = host.module_row_mut(module)
                {
                    row.counts.rejected += 1;
                }
            }
            Ok(Some(entity))
                if matches!(
                    entity.phase,
                    Motion::None
                        | Motion::Noclip
                        | Motion::Toss
                        | Motion::Bounce
                        | Motion::Fly
                        | Motion::FlyMissile
                ) =>
            {
                if let Some(current) =
                    entity_think(host, module, &functions, &entity, entity.entity, time, step)
                {
                    let execution = match entity.phase {
                        Motion::Noclip => motion::noclip(host, module, current, motion_step),
                        Motion::Toss | Motion::Bounce | Motion::Fly | Motion::FlyMissile => {
                            motion::toss(host, module, current, time, step)
                        }
                        _ => Ok(()),
                    };
                    if execution.is_err()
                        && let Some(row) = host.module_row_mut(module)
                    {
                        row.counts.rejected += 1;
                    }
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

fn entity_think(
    host: &mut FrameHost,
    module: ModuleId,
    functions: &FunctionTable<FrameHost, ModuleDispatch>,
    source: &quakec::PhysicsEntity,
    current: EntityId,
    time: ThinkTime,
    step: f64,
) -> Option<EntityId> {
    let result = run_think_with_rebind(
        host,
        functions,
        current,
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
                .bound_entity(&host.runtime.server.entities, source.native)
                .ok()
                .flatten()
        },
    );
    if let Some(row) = host.module_row_mut(module) {
        row.counts.rejected += u64::from(result.rejected);
    }
    (result.rejected == 0)
        .then_some(result.current_entity)
        .flatten()
}

fn edit_quakec<R>(
    host: &mut FrameHost,
    module: ModuleId,
    access: execution::TraceAccess<'_>,
    call: impl FnOnce(
        &mut qa_compat::quakec::Vm,
        &mut qa_compat::services::EngineServices<'_>,
        qa_compat::services::CallContext,
    ) -> Result<R, qa_compat::services::CallError>,
) -> Result<R, ClientCommandError> {
    let time = host.time;
    let (row, storage, traces) = host
        .modules
        .as_mut()
        .and_then(|m| m.trace_parts_mut(module, access))
        .ok_or(ClientCommandError::MissingModule)?;
    let clock = row.time(time);
    let Program::QuakeC(qc) = &mut row.request.program else {
        return Err(ClientCommandError::NeedsNativeModule);
    };
    let mut services = host
        .runtime
        .engine_services(&mut host.console, storage, &mut row.scratch);
    services.native_geometry = row.geometry;
    services.foreign_trace = Some(traces.borrow());
    call(
        &mut qc.vm,
        &mut services,
        context(row.request.context, clock),
    )
    .map_err(ClientCommandError::Usercmd)
}
fn context(mut context: CallContext, time: ThinkTime) -> CallContext {
    context.clock = time;
    context
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
    let reference = qa_core::primitives::NativeEntityRef::entity(
        qa_core::primitives::ResourceNamespace::Module(module),
        u32::try_from(native.slot).ok()?,
    );
    vm.weak_entity_offset(module, reference).ok()
}
