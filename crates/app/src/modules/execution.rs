//! Synchronous module calls use one load-reserved, iterative activation stack.
use super::{Module, ModuleResult, Program, State, contacts, context, motion, native_self};
use crate::host::FrameHost;
use qa_compat::{
    abi::{
        ArgumentSelection, CommandRole, HostRequest, ImportContinuation, ImportStep, QUAKEC,
        QcImportContinuation, QcImportStep, QuakeCCalls, QvmCalls,
    },
    quakec, qvm,
    services::{CallContext, CallError as ServiceError, ClientConnection, EngineServices},
};
use qa_console::{
    command_text::{Arguments, ParsedCommand},
    commands::{CommandReply, CommandRun, CommandStep},
    views::Context as ConsoleContext,
};
use qa_core::{
    primitives::{
        CallbackCall, CallbackId, ClientCallback, ClientId, CommandArgs, EntityId, ModuleId,
        ResourceNamespace, RuleSetId,
    },
    sys_events::EventTime,
};
use qa_input::{DispatchCursor, DispatchResume, DispatchStep, InputCapture};
use qa_session::dispatch::CallOutcome;
use std::sync::Arc;

mod connection;

pub(super) enum ModuleDispatch {
    Completed(ExecutionResult),
    Guest(GuestStart),
}

/// A backend borrow belongs to this synchronous execution scope, including
/// every nested guest and console activation entered through the same driver.
pub struct Driver<'a> {
    preparer: Option<&'a mut dyn crate::renderer::AssetPreparer>,
    connector: Option<&'a mut qa_platform::EventPump>,
}
impl<'a> Driver<'a> {
    pub fn new(preparer: Option<&'a mut dyn crate::renderer::AssetPreparer>) -> Self {
        Self {
            preparer,
            connector: None,
        }
    }
    pub fn with_connector(
        preparer: Option<&'a mut dyn crate::renderer::AssetPreparer>,
        connector: &'a mut qa_platform::EventPump,
    ) -> Self {
        Self {
            connector: Some(connector),
            ..Self::new(preparer)
        }
    }
    pub fn cache_only() -> Self {
        Self::new(None)
    }
    fn available(&self) -> bool {
        self.preparer.is_some()
    }
    pub(super) fn complete(
        &mut self,
        host: &mut FrameHost,
        outcome: ModuleDispatch,
        call: CallbackCall<'_>,
    ) -> bool {
        match outcome {
            ModuleDispatch::Completed(result) => result.is_ok(),
            ModuleDispatch::Guest(start) => {
                let command = match call {
                    CallbackCall::Export { command, .. } | CallbackCall::Client { command, .. } => {
                        command
                    }
                    _ => CommandArgs::default(),
                };
                drive(
                    host,
                    Frame::Guest(Activation::new(start, false)),
                    command,
                    self,
                )
                .is_ok()
            }
        }
    }
}
impl CallOutcome<FrameHost> for ModuleDispatch {
    fn rejected() -> Self {
        Self::Completed(Err(()))
    }
    fn complete(self, host: &mut FrameHost, call: CallbackCall<'_>) -> bool {
        Driver::cache_only().complete(host, self, call)
    }
}

#[derive(Clone, Copy)]
pub(super) enum EntryScope {
    Ordinary,
    Lifecycle,
}
#[derive(Clone, Copy)]
pub(super) struct GuestStart {
    scope: EntryScope,
    module: ModuleId,
    call: CallbackCall<'static>,
    anchor: EntityId,
    entry: u32,
    context: CallContext,
    platform_time: EventTime,
    connection: Option<(ClientId, EntityId)>,
}

impl GuestStart {
    fn child_scope(self, module: ModuleId) -> EntryScope {
        if module == self.module {
            self.scope
        } else {
            EntryScope::Ordinary
        }
    }
    fn lifecycle_module(self) -> Option<ModuleId> {
        matches!(self.scope, EntryScope::Lifecycle).then_some(self.module)
    }
    fn words(self) -> [u64; 9] {
        match self.call {
            CallbackCall::Think { time, .. } => {
                let mut words = [0; 9];
                words[0] = time.milliseconds() as u64;
                words
            }
            CallbackCall::Export { arguments, .. } | CallbackCall::Client { arguments, .. } => {
                arguments
            }
            _ => [0; 9],
        }
    }
    fn native_player(self) -> Option<ClientId> {
        match self.call {
            CallbackCall::Export { client, .. } => client,
            CallbackCall::Client { client, .. } => Some(client),
            _ => None,
        }
    }
}

enum Backend {
    QuakeC {
        run: Option<quakec::Run>,
        restore: Option<quakec::ClientCallRestore>,
        native: Option<u32>,
    },
    Qvm {
        run: qvm::Run,
        outer: bool,
    },
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    Native {
        run: qa_compat::native::Run,
        pending: Option<qa_compat::native::PendingImport>,
    },
}
enum ReplyPoint {
    QuakeC(QcImportContinuation),
    Qvm(ImportContinuation),
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    Native,
}
struct Pending {
    point: Option<ReplyPoint>,
    operation: Operation,
}
enum Operation {
    RegisterRenderer(Option<qa_compat::services::RendererRequest>),
    Callback(Option<qa_compat::abi::CallbackRequest>),
    Triggers {
        run: contacts::TriggerRun,
        touch: Option<contacts::PreparedTouch>,
    },
    Cleanup {
        result: Result<ModuleResult, ()>,
        failed: bool,
        next_client: usize,
        retiring: Option<(ClientId, EntityId)>,
        termination: Option<qa_compat::services::ClientTermination>,
    },
    Clear {
        cursor: Option<DispatchCursor<'static>>,
        resume: Option<DispatchResume<'static>>,
        context: qa_console::views::Context,
        time: EventTime,
        recipient: Option<ModuleId>,
    },
    Console {
        context: ConsoleContext,
        text: Option<u64>,
        waiting: bool,
    },
}
type ExecutionResult = Result<Option<ModuleResult>, ()>;

#[derive(Clone, Copy)]
enum ArgSource {
    Ambient,
    Console(usize),
}
pub(crate) struct ExecutionSlot {
    frame: Option<Frame>,
    command: ParsedCommand,
}
impl ExecutionSlot {
    pub(crate) fn reserve(capacity: usize) -> Result<Box<[Self]>, String> {
        capacity
            .checked_mul(std::mem::size_of::<Self>())
            .ok_or("execution byte capacity")?;
        let mut slots = Vec::new();
        slots
            .try_reserve_exact(capacity)
            .map_err(|_| "execution reservation")?;
        slots.resize_with(capacity, || Self {
            frame: None,
            command: ParsedCommand::default(),
        });
        Ok(slots.into_boxed_slice())
    }
}
/// Borrowed authority for foreign reads while the owned driver is suspended.
/// Current wins over parents; an active matching frame without a pending stop
/// cannot fall through to an older activation's controller capability.
#[derive(Clone, Copy)]
pub(crate) struct TraceAccess<'a> {
    parents: &'a [ExecutionSlot],
    current: Option<(ModuleId, &'a Backend)>,
}
impl<'a> TraceAccess<'a> {
    pub(crate) fn empty() -> Self {
        Self {
            parents: &[],
            current: None,
        }
    }
    fn parents(parents: &'a [ExecutionSlot]) -> Self {
        Self {
            parents,
            current: None,
        }
    }
    fn with_current(self, module: ModuleId, backend: Option<&'a Backend>) -> Self {
        Self {
            current: backend.map(|backend| (module, backend)),
            ..self
        }
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    pub(super) fn native(
        self,
        module: ModuleId,
    ) -> Option<(
        &'a qa_compat::native::Run,
        &'a qa_compat::native::PendingImport,
    )> {
        let stopped = |backend: &'a Backend| match backend {
            Backend::Native { run, pending } => pending.as_ref().map(|pending| (run, pending)),
            _ => None,
        };
        if let Some((current, backend @ Backend::Native { .. })) = self.current
            && current == module
        {
            return stopped(backend);
        }
        for slot in self.parents.iter().rev() {
            if let Some(Frame::Guest(frame)) = &slot.frame
                && frame.start.module == module
                && let Some(backend @ Backend::Native { .. }) = frame.backend.as_ref()
            {
                return stopped(backend);
            }
        }
        None
    }
}

enum Frame {
    Guest(Activation),
    Console(ConsoleActivation),
}
impl Frame {
    fn child(&mut self, result: ExecutionResult) {
        match self {
            Self::Guest(frame) => frame.child = Some(result),
            Self::Console(frame) => frame.child = Some(result),
        }
    }
}
struct ConsoleActivation {
    run: CommandRun,
    fallback: Option<FallbackRun>,
    child: Option<ExecutionResult>,
    owner: Option<GuestStart>,
    connection: Option<connection::Change>,
}
impl ConsoleActivation {
    fn new(run: CommandRun) -> Self {
        Self {
            run,
            fallback: None,
            child: None,
            owner: None,
            connection: None,
        }
    }
}
struct FallbackRun {
    tier: u8,
    next: usize,
    client: Option<(ClientId, EntityId, bool)>,
}
pub(super) struct CommandBinding {
    role: CommandRole,
    callback: CallbackId,
}
impl CommandBinding {
    pub(super) fn load(program: &Program, entries: &[u32]) -> Result<Option<Self>, String> {
        let Some(export) = program.command_export() else {
            return Ok(None);
        };
        let index = entries
            .iter()
            .position(|&entry| entry == export.entry)
            .ok_or("missing command export")?;
        Ok(Some(Self {
            role: export.role,
            callback: CallbackId(u32::try_from(index).map_err(|_| "command callback capacity")?),
        }))
    }
}
pub(super) struct Activation {
    start: GuestStart,
    backend: Option<Backend>,
    counted: bool,
    pending: Option<Pending>,
    reply: Option<(ReplyPoint, u64)>,
    child: Option<ExecutionResult>,
    arguments: ArgSource,
    argument_selection: ArgumentSelection,
    input_callback: bool,
    termination: Option<qa_compat::services::ClientTermination>,
    admitted: Option<quakec::AdmittedEntity>,
}
impl Activation {
    fn new(start: GuestStart, input_callback: bool) -> Self {
        Self {
            start,
            backend: None,
            counted: false,
            pending: None,
            reply: None,
            child: None,
            arguments: ArgSource::Ambient,
            argument_selection: ArgumentSelection::RetainedClient,
            input_callback,
            termination: None,
            admitted: None,
        }
    }
}

enum GuestStep {
    Host(HostRequest, ReplyPoint),
    Returned(ModuleResult),
}
enum OperationStep {
    Continue,
    Call(ModuleDispatch),
    Reply(u64),
    Abort,
    FailedInput(ModuleId),
    Finished(Result<ModuleResult, ()>),
    Console {
        context: ConsoleContext,
        text: Option<u64>,
    },
}
enum FrameStep {
    Continue,
    Guest(Activation),
    Console {
        context: ConsoleContext,
        text: Option<u64>,
    },
    Finished(ExecutionResult),
}

pub(super) fn start(
    host: &mut FrameHost,
    module: ModuleId,
    entry: u32,
    call: CallbackCall<'_>,
) -> ModuleDispatch {
    start_scoped(host, module, entry, call, EntryScope::Ordinary)
}

pub(super) fn start_scoped(
    host: &mut FrameHost,
    module: ModuleId,
    entry: u32,
    call: CallbackCall<'_>,
    scope: EntryScope,
) -> ModuleDispatch {
    let Some(row) = host.module_row(module) else {
        return ModuleDispatch::Completed(Err(()));
    };
    if !row.state.can_invoke()
        || matches!(scope, EntryScope::Ordinary) && !host.module_generation_ready(module)
    {
        return ModuleDispatch::Completed(Err(()));
    }
    // Command tokens remain in complete()'s external lease. The activation owns
    // only the numeric callback metadata needed across stopped guest borrows.
    let (call, time) = match call {
        CallbackCall::Think {
            entity,
            time,
            prepared_native,
        } => (
            CallbackCall::Think {
                entity,
                time,
                prepared_native,
            },
            time,
        ),
        CallbackCall::Spawn { entity, time }
            if matches!(row.request.program, Program::QuakeC(_)) =>
        {
            (CallbackCall::Spawn { entity, time }, time)
        }
        CallbackCall::Touch {
            entity,
            other,
            time,
        } if matches!(row.request.program, Program::QuakeC(_)) => (
            CallbackCall::Touch {
                entity,
                other,
                time: if other.namespace == ResourceNamespace::Module(module) {
                    time
                } else {
                    Some(row.time(host.time))
                },
            },
            if other.namespace == ResourceNamespace::Module(module) {
                time.unwrap_or_else(|| row.time(host.time))
            } else {
                row.time(host.time)
            },
        ),
        CallbackCall::Blocked {
            entity,
            subject,
            other,
        } if matches!(row.request.program, Program::QuakeC(_)) => (
            CallbackCall::Blocked {
                entity,
                subject,
                other,
            },
            row.time(host.time),
        ),
        CallbackCall::Export {
            entity,
            time,
            arguments,
            client,
            ..
        } => (
            CallbackCall::Export {
                entity,
                time,
                arguments,
                client,
                command: CommandArgs::default(),
            },
            time,
        ),
        CallbackCall::Client {
            entity,
            client,
            event,
            time,
            arguments,
            ..
        } => (
            CallbackCall::Client {
                entity,
                client,
                event,
                time,
                arguments,
                command: CommandArgs::default(),
            },
            time,
        ),
        _ => return ModuleDispatch::Completed(Err(())),
    };
    let start = GuestStart {
        scope,
        module,
        call,
        anchor: row.request.anchor,
        entry,
        // ED_Alloc/ED_Free use the module frame clock. A think's QC global
        // time instead contains its selected nextthink time.
        context: if matches!(row.request.program, Program::QuakeC(_))
            && matches!(call, CallbackCall::Think { .. })
        {
            row.request.context
        } else {
            context(row.request.context, time)
        },
        platform_time: host.runtime.input_time,
        connection: row.client_connection,
    };
    if valid(host, start, true) {
        ModuleDispatch::Guest(start)
    } else {
        ModuleDispatch::Completed(Err(()))
    }
}

fn valid(host: &FrameHost, start: GuestStart, live_self: bool) -> bool {
    let Some(row) = host.module_row(start.module) else {
        return false;
    };
    if !row.state.can_invoke()
        || matches!(start.scope, EntryScope::Ordinary)
            && !host.module_generation_ready(start.module)
        || row.request.anchor != start.anchor
        || row.client_connection != start.connection
    {
        return false;
    }
    for entity in [Some(start.anchor), live_self.then_some(start.call.entity())]
        .into_iter()
        .flatten()
    {
        let Some(slot) = host.runtime.server.entities.resolve(entity) else {
            return false;
        };
        if host.runtime.server.entities.columns.owner[slot] != start.module {
            return false;
        }
    }
    if let CallbackCall::Client { entity, client, .. } = start.call
        && host
            .runtime
            .server
            .clients
            .get(client.0 as usize)
            .is_none_or(|client| client.connection.is_none() || client.entity != entity)
    {
        return false;
    }
    if let Some((id, lifetime)) = start.connection {
        if start
            .native_player()
            .is_some_and(|requested| requested != id)
            || host
                .runtime
                .server
                .clients
                .get(id.0 as usize)
                .is_none_or(|client| client.connection.is_none() || client.entity != lifetime)
            || host
                .runtime
                .snapshots
                .get(id.0 as usize)
                .and_then(Option::as_ref)
                .is_some_and(|binding| binding.presentation_closed())
            || host.runtime.server.entities.resolve(lifetime).is_none()
        {
            return false;
        }
    }
    true
}

fn valid_frame(host: &FrameHost, start: GuestStart, backend: Option<&Backend>) -> bool {
    // QC keeps native self as a weak edict offset. A trigger may free it while
    // the suspended function remains executable. Its module anchor and selected
    // client lifetime still authorize every resume; entry validation was strong.
    let live_self = !matches!(backend, Some(Backend::QuakeC { .. }))
        || matches!(start.call, CallbackCall::Client { .. });
    valid(host, start, live_self)
}

impl FrameHost {
    /// Both Com_Frame command phases and startup use the owned module driver.
    pub fn execute_commands(&mut self) {
        self.execute_commands_with_driver(&mut Driver::cache_only());
    }
    pub fn execute_commands_with_driver(&mut self, driver: &mut Driver<'_>) {
        let _ = drive(
            self,
            Frame::Console(ConsoleActivation::new(CommandRun::drain())),
            CommandArgs::default(),
            driver,
        );
    }
}

fn arguments<'a>(
    arena: &'a [ExecutionSlot],
    source: ArgSource,
    root: CommandArgs<'a>,
) -> Option<Arguments<'a>> {
    match source {
        ArgSource::Ambient => Some(Arguments::from_command(root)),
        ArgSource::Console(index) => Some(arena.get(index)?.command.arguments()),
    }
}

fn drive(
    host: &mut FrameHost,
    frame: Frame,
    command: CommandArgs<'_>,
    driver: &mut Driver<'_>,
) -> ExecutionResult {
    let mut arena = std::mem::take(&mut host.execution);
    if arena.is_empty() {
        host.execution = arena;
        return Err(());
    }
    arena[0].frame = Some(frame);
    let mut depth = 1;
    let result = loop {
        let index = depth - 1;
        let Some(mut frame) = arena[index].frame.take() else {
            break Err(());
        };
        let step = match &mut frame {
            Frame::Guest(guest) => match arguments(&arena, guest.arguments, command) {
                Some(command) => guest_step(
                    host,
                    guest,
                    command,
                    TraceAccess::parents(&arena[..index]),
                    driver,
                ),
                None => {
                    cancel(
                        host,
                        guest,
                        Arguments::default(),
                        TraceAccess::parents(&arena[..index]),
                    );
                    completing(host, guest, Err(()));
                    FrameStep::Continue
                }
            },
            Frame::Console(console) => {
                let (parents, current) = arena.split_at_mut(index);
                console_step(
                    host,
                    console,
                    &mut current[0].command,
                    index,
                    TraceAccess::parents(parents),
                    driver,
                )
            }
        };
        match step {
            FrameStep::Continue => arena[index].frame = Some(frame),
            FrameStep::Guest(guest) => {
                if depth == arena.len() {
                    frame.child(Err(()));
                    arena[index].frame = Some(frame);
                } else {
                    arena[index].frame = Some(frame);
                    arena[depth].frame = Some(Frame::Guest(guest));
                    depth += 1;
                }
            }
            FrameStep::Console { context, text } => {
                if depth == arena.len() {
                    frame.child(Err(()));
                    arena[index].frame = Some(frame);
                    continue;
                }
                let captured = match &mut frame {
                    Frame::Guest(guest) => {
                        capture_command(host, guest, text, &mut arena[depth].command)
                    }
                    Frame::Console(_) => Err(()),
                };
                match captured {
                    Ok(nonempty) => {
                        let run = if nonempty {
                            CommandRun::single(context)
                        } else {
                            CommandRun::drain()
                        };
                        arena[index].frame = Some(frame);
                        let mut console = ConsoleActivation::new(run);
                        if let Some(Frame::Guest(guest)) = arena[index].frame.as_ref() {
                            console.owner = Some(guest.start);
                        }
                        arena[depth].frame = Some(Frame::Console(console));
                        depth += 1;
                    }
                    Err(()) => {
                        frame.child(Err(()));
                        arena[index].frame = Some(frame);
                    }
                }
            }
            FrameStep::Finished(result) => {
                arena[index].command.clear();
                depth -= 1;
                if depth == 0 {
                    break result;
                }
                let Some(parent) = arena[depth - 1].frame.as_mut() else {
                    break Err(());
                };
                parent.child(result);
            }
        }
    };
    // Drain all owned guest tokens before returning this same arena. Console
    // scratch stays in its slot until every borrowing child has unwound.
    while depth != 0 {
        depth -= 1;
        if let Some(Frame::Guest(mut frame)) = arena[depth].frame.take() {
            let command = arguments(&arena, frame.arguments, command).unwrap_or_default();
            cancel(
                host,
                &mut frame,
                command,
                TraceAccess::parents(&arena[..depth]),
            );
            let _ = finish(host, &frame, Err(()));
        }
        arena[depth].command.clear();
    }
    host.execution = arena;
    result
}

fn guest_step(
    host: &mut FrameHost,
    frame: &mut Activation,
    command: Arguments<'_>,
    access: TraceAccess<'_>,
    driver: &mut Driver<'_>,
) -> FrameStep {
    let cleanup = frame
        .pending
        .as_ref()
        .is_some_and(|pending| matches!(pending.operation, Operation::Cleanup { .. }));
    if !cleanup && !valid_frame(host, frame.start, frame.backend.as_ref()) {
        cancel(host, frame, command, access);
        completing(host, frame, Err(()));
        return FrameStep::Continue;
    }
    if frame.pending.is_some() {
        return match operation(host, frame, access, driver) {
            OperationStep::Continue => FrameStep::Continue,
            OperationStep::Reply(value) => {
                if let Some(Pending {
                    point: Some(point), ..
                }) = frame.pending.take()
                {
                    frame.reply = Some((point, value));
                } else {
                    cancel(host, frame, command, access);
                    completing(host, frame, Err(()));
                }
                FrameStep::Continue
            }
            OperationStep::Call(ModuleDispatch::Completed(result)) => {
                frame.child = Some(result);
                FrameStep::Continue
            }
            OperationStep::Call(ModuleDispatch::Guest(start)) => {
                let input = frame
                    .pending
                    .as_ref()
                    .is_some_and(|pending| matches!(pending.operation, Operation::Clear { .. }));
                let mut child = Activation::new(start, input);
                child.arguments = frame.arguments;
                child.argument_selection = match frame.arguments {
                    ArgSource::Ambient => ArgumentSelection::RetainedClient,
                    ArgSource::Console(_) => ArgumentSelection::Caller,
                };
                FrameStep::Guest(child)
            }
            OperationStep::Abort => {
                cancel(host, frame, command, access);
                completing(host, frame, Err(()));
                FrameStep::Continue
            }
            OperationStep::FailedInput(module) => {
                if let Some(row) = host.module_row(module).filter(|row| row.state.can_invoke()) {
                    let start = GuestStart {
                        scope: frame.start.child_scope(module),
                        module,
                        call: CallbackCall::Think {
                            entity: row.request.anchor,
                            time: row.request.context.clock,
                            prepared_native: None,
                        },
                        anchor: row.request.anchor,
                        entry: 0,
                        context: row.request.context,
                        platform_time: host.runtime.input_time,
                        connection: row.client_connection,
                    };
                    let mut cleanup = Activation::new(start, true);
                    cleanup.arguments = frame.arguments;
                    completing(host, &mut cleanup, Err(()));
                    return FrameStep::Guest(cleanup);
                }
                frame.child = Some(Err(()));
                FrameStep::Continue
            }
            OperationStep::Finished(result) => FrameStep::Finished(finish(host, frame, result)),
            OperationStep::Console { context, text } => FrameStep::Console { context, text },
        };
    }
    match advance(host, frame, command, access, driver) {
        Ok(GuestStep::Host(request, point)) => {
            let operation = match request {
                HostRequest::RegisterRenderer(request) => {
                    Ok(Operation::RegisterRenderer(Some(request)))
                }
                HostRequest::Callback(request) => Ok(Operation::Callback(Some(request))),
                HostRequest::TouchTriggers { mover, time }
                    if mover.namespace == ResourceNamespace::Module(frame.start.module) =>
                {
                    contacts::TriggerRun::begin(host, mover, time)
                        .map(|run| Operation::Triggers { run, touch: None })
                        .map_err(|_| ())
                }
                HostRequest::TouchTriggers { .. } => Err(()),
                HostRequest::ClearKeys {
                    seat,
                    context,
                    time,
                } => Ok(Operation::Clear {
                    cursor: Some(DispatchCursor::clear_seat(seat, time)),
                    resume: None,
                    context,
                    time,
                    recipient: None,
                }),
                HostRequest::ExecuteNow { context, text } => Ok(Operation::Console {
                    context,
                    text,
                    waiting: false,
                }),
            };
            if let Ok(operation) = operation {
                frame.pending = Some(Pending {
                    point: Some(point),
                    operation,
                });
            } else {
                frame.reply = Some((point, 0));
                cancel(host, frame, command, access);
                completing(host, frame, Err(()));
            }
        }
        Ok(GuestStep::Returned(value)) => completing(host, frame, Ok(value)),
        Err(()) => {
            cancel(host, frame, command, access);
            completing(host, frame, Err(()));
        }
    }
    FrameStep::Continue
}

fn capture_command(
    host: &mut FrameHost,
    frame: &mut Activation,
    text: Option<u64>,
    scratch: &mut ParsedCommand,
) -> Result<bool, ()> {
    scratch.clear();
    let Some(pointer) = text else {
        return Ok(false);
    };
    let row = host.module_row_mut(frame.start.module).ok_or(())?;
    match (
        &mut row.request.program,
        frame.backend.as_ref(),
        frame
            .pending
            .as_ref()
            .and_then(|pending| pending.point.as_ref()),
    ) {
        (Program::Qvm { vm, .. }, Some(Backend::Qvm { .. }), Some(ReplyPoint::Qvm(_))) => {
            let text = vm
                .memory
                .cstring(vm.masked_address(pointer as i32))
                .map_err(|_| ())?;
            scratch.set_raw(text).map_err(|_| ())?;
            Ok(!text.is_empty())
        }
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        (
            Program::Native { vm, .. },
            Some(Backend::Native {
                run,
                pending: Some(pending),
            }),
            Some(ReplyPoint::Native),
        ) => vm
            .copy_import_command(run, pending, pointer, scratch)
            .map_err(|_| ()),
        _ => Err(()),
    }
}

impl FallbackRun {
    fn new(host: &FrameHost, context: ConsoleContext) -> Self {
        let client = host.local_clients[context.seat.index()].and_then(|id| {
            let client = host.runtime.server.clients.get(id.0 as usize)?;
            Some((id, client.entity, client.is_active()))
        });
        Self {
            tier: 0,
            next: 0,
            client,
        }
    }
    fn current(&self, host: &FrameHost, context: ConsoleContext) -> bool {
        host.local_clients[context.seat.index()] == self.client.map(|(id, _, _)| id)
            && self.client.is_none_or(|(id, lifetime, active)| {
                host.runtime
                    .server
                    .clients
                    .get(id.0 as usize)
                    .is_some_and(|client| client.is_active() == active && client.entity == lifetime)
                    && (!active
                        || (host.runtime.server.entities.resolve(lifetime).is_some()
                            && host
                                .runtime
                                .snapshots
                                .get(id.0 as usize)
                                .and_then(Option::as_ref)
                                .is_none_or(|snapshot| !snapshot.presentation_closed())))
            })
    }
    fn next(
        &mut self,
        host: &mut FrameHost,
        context: ConsoleContext,
        owner: Option<GuestStart>,
    ) -> Option<ModuleDispatch> {
        use qa_core::sys_events::key_catchers;
        if !self.current(host, context) {
            return None;
        }
        loop {
            let role = match self.tier {
                0 => CommandRole::Cgame,
                1 => CommandRole::Server,
                2 => CommandRole::Ui,
                _ => return None,
            };
            let candidate = if role == CommandRole::Server {
                let modules = host.modules.as_ref()?;
                let selected = self
                    .client
                    .filter(|(_, _, active)| *active)
                    .and_then(|(id, _, _)| host.runtime.server.clients.get(id.0 as usize))
                    .map(|client| client.module);
                let mut selected_row = None;
                while self.next < modules.rows.len() {
                    let index = self.next;
                    self.next += 1;
                    let Some(row) = modules.rows[index].as_ref() else {
                        continue;
                    };
                    let module = row.request.context.module;
                    if selected.is_some_and(|selected| selected != module)
                        || row.request.phase != super::Phase::Server
                        || !row.ready_for_callback(true)
                        || row
                            .command
                            .as_ref()
                            .is_none_or(|command| command.role != role)
                    {
                        continue;
                    }
                    selected_row = Some((module, row.time(host.time)));
                    break;
                }
                selected_row
            } else {
                let catcher = if role == CommandRole::Ui {
                    key_catchers::UI
                } else {
                    key_catchers::CGAME
                };
                let candidates = host.client_module_candidates(context.seat, catcher);
                let mut selected = None;
                while self.next < candidates.len() {
                    let index = self.next;
                    self.next += 1;
                    let Some(module) = candidates[index] else {
                        continue;
                    };
                    if candidates[..index].contains(&Some(module)) {
                        continue;
                    }
                    let Some(row) = host.module_row(module) else {
                        continue;
                    };
                    if row
                        .command
                        .as_ref()
                        .is_none_or(|command| command.role != role)
                    {
                        continue;
                    }
                    if let Some(time) = host.client_module_time_scoped(
                        module,
                        context.seat,
                        true,
                        owner.and_then(GuestStart::lifecycle_module),
                    ) {
                        selected = Some((module, time));
                        // One selected CLIENT owner per native tier.
                        self.next = candidates.len();
                        break;
                    }
                }
                selected
            };
            if let Some((module, time)) = candidate {
                let row = host.module_row(module)?;
                let callback = row.command.as_ref()?.callback;
                let mut arguments = [0; 9];
                if role == CommandRole::Ui {
                    arguments[0] = host.time.milliseconds();
                }
                let call = CallbackCall::Export {
                    entity: row.request.anchor,
                    time,
                    arguments,
                    client: None,
                    command: CommandArgs::default(),
                };
                let functions = Arc::clone(&host.modules.as_ref()?.functions);
                let dispatched = if let Some(owner) = owner {
                    functions
                        .resolve(&host.runtime.server.entities, callback, call.entity())
                        .and_then(|target| dispatch_child(host, owner, target, call))
                } else {
                    functions.dispatch(host, callback, call)
                };
                return Some(dispatched.unwrap_or(ModuleDispatch::Completed(Err(()))));
            }
            self.tier += 1;
            self.next = 0;
        }
    }
}

fn forward_command(
    host: &mut FrameHost,
    fallback: &FallbackRun,
    context: ConsoleContext,
    command: &ParsedCommand,
) -> bool {
    let arguments = command.arguments();
    let text = if context.source == RuleSetId::Quake3 {
        // CL_ForwardCommandToServer applies console semantics before selecting
        // the connection's independently negotiated native string encoding.
        match arguments.get_bytes(0).first() {
            Some(b'-') => return true,
            Some(b'+') => return false,
            _ => {}
        }
        if arguments.len() == 1 {
            arguments.get_bytes(0)
        } else {
            command.raw_bytes()
        }
    } else {
        command.raw_bytes()
    };
    if !fallback.current(host, context) {
        return false;
    }
    let Some((id, _, true)) = fallback.client else {
        return false;
    };
    let Some(connection) = host
        .runtime
        .network
        .get_mut(id, qa_core::loopback::Endpoint::Client)
    else {
        return false;
    };
    if !connection.phase.is_connected() {
        return false;
    }
    let Some(protocol) = connection
        .commands
        .as_ref()
        .map(|commands| commands.protocol)
    else {
        return false;
    };
    connection
        .channel
        .queue_client_command(protocol, text)
        .is_ok()
}

fn console_step(
    host: &mut FrameHost,
    frame: &mut ConsoleActivation,
    scratch: &mut ParsedCommand,
    index: usize,
    access: TraceAccess<'_>,
    driver: &mut Driver<'_>,
) -> FrameStep {
    if let Some(change) = frame.connection.as_mut() {
        return match change.advance(
            host,
            &mut frame.child,
            scratch,
            frame.owner,
            frame.run.context(),
            access,
            driver,
        ) {
            Some(step) => step,
            None => {
                frame.connection = None;
                frame.run.reply(CommandReply::Handled);
                FrameStep::Continue
            }
        };
    }
    if frame.owner.is_some_and(|owner| !valid(host, owner, true)) {
        return FrameStep::Finished(Err(()));
    }
    if host.runtime.quit {
        return FrameStep::Finished(Ok(None));
    }
    if frame.fallback.is_some()
        && frame.run.context().producer.is_some_and(|producer| {
            !qa_console::commands::Host::command_producer_live(&host.runtime, producer)
        })
    {
        frame.fallback = None;
        frame.child = None;
    }
    if frame
        .fallback
        .as_ref()
        .is_some_and(|fallback| !fallback.current(host, frame.run.context()))
    {
        frame.fallback = None;
        frame.child = None;
        frame.run.reply(CommandReply::Unhandled);
    }
    if let Some(result) = frame.child.take() {
        if result
            .ok()
            .flatten()
            .is_some_and(ModuleResult::claims_command)
        {
            frame.fallback = None;
            frame.run.reply(CommandReply::Handled);
        }
    }
    if let Some(fallback) = frame.fallback.as_mut() {
        if let Some(outcome) = fallback.next(host, frame.run.context(), frame.owner) {
            return match outcome {
                ModuleDispatch::Completed(result) => {
                    frame.child = Some(result);
                    FrameStep::Continue
                }
                ModuleDispatch::Guest(start) => {
                    let mut guest = Activation::new(start, false);
                    guest.arguments = ArgSource::Console(index);
                    guest.argument_selection = ArgumentSelection::Caller;
                    FrameStep::Guest(guest)
                }
            };
        }
        let forwarded = forward_command(host, fallback, frame.run.context(), scratch);
        frame.fallback = None;
        frame.run.reply(if forwarded {
            CommandReply::Handled
        } else {
            CommandReply::Unhandled
        });
    }
    match host
        .console
        .step(&mut host.runtime, &mut frame.run, scratch)
    {
        CommandStep::Continue => FrameStep::Continue,
        CommandStep::Done => FrameStep::Finished(Ok(None)),
        CommandStep::Fallback => {
            frame.fallback = Some(FallbackRun::new(host, frame.run.context()));
            FrameStep::Continue
        }
        CommandStep::External(command) => {
            frame.connection =
                connection::begin(host, command, scratch, frame.run.context(), driver);
            if frame.connection.is_none() {
                frame.run.reply(CommandReply::Handled);
            }
            FrameStep::Continue
        }
    }
}

fn completing(host: &mut FrameHost, frame: &mut Activation, mut result: Result<ModuleResult, ()>) {
    if let Some(admitted) = frame.admitted.take()
        && contacts::release(host, admitted).is_err()
    {
        result = Err(());
    }
    frame.backend.take();
    if result.is_err()
        && frame.input_callback
        && let Some(row) = host.module_row_mut(frame.start.module)
        && row.state.can_invoke()
    {
        row.state = State::Failed;
    }
    let failed = host.claim_failed_module(frame.start.module);
    frame.pending = Some(Pending {
        point: None,
        operation: Operation::Cleanup {
            result,
            failed,
            next_client: 0,
            retiring: None,
            termination: frame.termination.take(),
        },
    });
}

fn disconnect(
    host: &mut FrameHost,
    id: ClientId,
    lifetime: EntityId,
    retiring: &mut Option<(ClientId, EntityId)>,
    access: TraceAccess<'_>,
) -> Option<OperationStep> {
    let (callback, call) = host.begin_client_disconnect(id, lifetime, access)?;
    let Some(modules) = host.modules.as_ref() else {
        host.finish_client_disconnect(id, lifetime, access);
        return None;
    };
    let functions = Arc::clone(&modules.functions);
    *retiring = Some((id, lifetime));
    Some(OperationStep::Call(
        functions
            .dispatch(host, callback, call)
            .unwrap_or(ModuleDispatch::Completed(Err(()))),
    ))
}

fn dispatch_child(
    host: &mut FrameHost,
    parent: GuestStart,
    target: qa_session::dispatch::ResolvedCall,
    call: CallbackCall<'_>,
) -> Result<ModuleDispatch, qa_session::dispatch::CallError> {
    let functions = Arc::clone(
        &host
            .modules
            .as_ref()
            .ok_or(qa_session::dispatch::CallError::MissingModule)?
            .functions,
    );
    let target = functions.revalidate(&host.runtime.server.entities, target, call.entity())?;
    if functions.is_missing(target) {
        return functions.dispatch_resolved(host, target, call);
    }
    Ok(start_scoped(
        host,
        target.module(),
        target.entry(),
        call,
        parent.child_scope(target.module()),
    ))
}

fn operation(
    host: &mut FrameHost,
    frame: &mut Activation,
    parents: TraceAccess<'_>,
    driver: &mut Driver<'_>,
) -> OperationStep {
    let Activation {
        start,
        backend,
        pending,
        child,
        ..
    } = frame;
    let access = parents.with_current(start.module, backend.as_ref());
    let Some(pending) = pending.as_mut() else {
        return OperationStep::Abort;
    };
    match &mut pending.operation {
        Operation::RegisterRenderer(request) => {
            let Some(request) = request.take() else {
                return OperationStep::Abort;
            };
            let result = (|| {
                let row = host
                    .module_row(start.module)
                    .ok_or_else(|| "missing renderer module".to_owned())?;
                let profile = row
                    .scene_role
                    .profile()
                    .ok_or_else(|| "missing renderer presentation".to_owned())?;
                if matches!(
                    request.kind,
                    qa_compat::services::RendererRegistration::Model
                ) && request.name.as_bytes().starts_with(b"*")
                    && !request.world.is_some_and(|world| {
                        row.scene_role.registration_world_available(
                            host.runtime.scene.as_ref(),
                            host.runtime.model_assets.as_ref().map(|owner| owner.view()),
                            world,
                        )
                    })
                {
                    return Err("missing inline model presentation".to_owned());
                }
                let preparer = driver
                    .preparer
                    .as_mut()
                    .ok_or_else(|| "renderer registration has no backend".to_owned())?;
                let owner = host
                    .runtime
                    .model_assets
                    .as_mut()
                    .ok_or_else(|| "missing model assets".to_owned())?;
                owner.prepare_registration(&host.runtime.vfs, &request, profile, |assets| {
                    (**preparer).prepare_assets(assets, None)
                })
            })();
            let value = match result {
                Ok((handle, _)) => {
                    host.refresh_model_consumers();
                    handle as u32 as u64
                }
                Err(error) => {
                    let _ = host
                        .runtime
                        .server
                        .events
                        .console_formatted(format_args!("renderer registration: {error}\n"));
                    0
                }
            };
            OperationStep::Reply(value)
        }
        Operation::Console {
            context,
            text,
            waiting,
        } => {
            if !*waiting {
                *waiting = true;
                OperationStep::Console {
                    context: *context,
                    text: *text,
                }
            } else if matches!(child.take(), Some(Ok(_))) {
                OperationStep::Reply(0)
            } else {
                OperationStep::Abort
            }
        }
        Operation::Callback(request) => {
            if let Some(request) = request.take() {
                OperationStep::Call(
                    dispatch_child(host, *start, request.target, request.call)
                        .unwrap_or(ModuleDispatch::Completed(Err(()))),
                )
            } else if matches!(child.take(), Some(Ok(_))) {
                return OperationStep::Reply(0);
            } else {
                return OperationStep::Abort;
            }
        }
        Operation::Triggers { run, touch } => {
            if let Some(accepted) = child.take() {
                let Some(completed) = touch.take() else {
                    return OperationStep::Abort;
                };
                if completed.complete(host, accepted.is_ok()).is_err() {
                    return OperationStep::Abort;
                }
            }
            match run.next(host, access) {
                Ok(Some(prepared)) => {
                    let request = prepared.request();
                    *touch = Some(prepared);
                    match dispatch_child(host, *start, request.target, request.call) {
                        Ok(outcome) => OperationStep::Call(outcome),
                        Err(_) => OperationStep::Abort,
                    }
                }
                Ok(None) => OperationStep::Reply(0),
                Err(_) => OperationStep::Abort,
            }
        }
        Operation::Cleanup {
            result,
            failed,
            next_client,
            retiring,
            termination,
        } => {
            if let Some((id, lifetime)) = retiring.take() {
                child.take();
                host.finish_client_disconnect(id, lifetime, access);
            }
            if let Some(termination) = termination.take()
                && let Some((id, lifetime)) = start.connection
                && host
                    .runtime
                    .server
                    .clients
                    .get(id.0 as usize)
                    .is_some_and(|client| client.entity == lifetime)
            {
                match termination {
                    qa_compat::services::ClientTermination::ServerDisconnect => {
                        host.received_client_disconnect(id)
                    }
                    qa_compat::services::ClientTermination::Rejected => {
                        if let Some(step) = disconnect(host, id, lifetime, retiring, access) {
                            return step;
                        }
                    }
                }
            }
            if *failed {
                while *next_client < host.runtime.server.clients.len() {
                    let id = ClientId(*next_client as u32);
                    *next_client += 1;
                    if !host.module_recipient(start.module, id) {
                        continue;
                    }
                    let lifetime = host.runtime.server.clients[id.0 as usize].entity;
                    if let Some(step) = disconnect(host, id, lifetime, retiring, access) {
                        return step;
                    }
                }
                // Keep native client identities live through their disconnect
                // callbacks. The sole failed-module owner retires rows afterward.
                host.retire_module_state(start.module);
                *failed = false;
            }
            OperationStep::Finished(*result)
        }
        Operation::Clear {
            cursor,
            resume,
            context,
            time,
            recipient,
        } => {
            if let Some(accepted) = child.take() {
                if accepted.is_err()
                    && let Some(module) = recipient.take()
                {
                    return OperationStep::FailedInput(module);
                }
                *recipient = None;
                if !valid_frame(host, *start, backend.as_ref()) {
                    return OperationStep::Abort;
                }
            }
            if let Some(resume) = resume.take() {
                match input_resume(host, start.module, resume, *context) {
                    Some(next) => *cursor = Some(next),
                    None => return OperationStep::Abort,
                }
            }
            let Some(current) = cursor.take() else {
                return OperationStep::Abort;
            };
            match input_advance(host, start.module, current, *context) {
                Some(DispatchStep::Done) => return OperationStep::Reply(0),
                Some(DispatchStep::Atom { atom, resume: next }) => {
                    *resume = Some(next);
                    host.console
                        .sync_editor_input(&mut host.runtime.input, *time);
                    if let Some((module, callback, call)) =
                        host.input_call(atom, true, start.lifecycle_module())
                    {
                        *recipient = Some(module);
                        let Some(modules) = host.modules.as_ref() else {
                            return OperationStep::Abort;
                        };
                        let functions = Arc::clone(&modules.functions);
                        OperationStep::Call(
                            functions
                                .resolve(&host.runtime.server.entities, callback, call.entity())
                                .and_then(|target| dispatch_child(host, *start, target, call))
                                .unwrap_or(ModuleDispatch::Completed(Err(()))),
                        )
                    } else {
                        OperationStep::Continue
                    }
                }
                None => return OperationStep::Abort,
            }
        }
    }
}

fn input_advance(
    host: &mut FrameHost,
    module: ModuleId,
    cursor: DispatchCursor<'static>,
    context: qa_console::views::Context,
) -> Option<DispatchStep<'static>> {
    let (row, storage) = host.modules.as_mut()?.parts_mut(module)?;
    let mut services = host
        .runtime
        .engine_services(&mut host.console, storage, &mut row.scratch);
    let mut output = qa_console::commands::BindOutput::new(services.commands, context);
    let step = services.input.advance(cursor, &mut output);
    let outcome = output.outcome();
    services.input_outcome(outcome);
    Some(step)
}
fn input_resume(
    host: &mut FrameHost,
    module: ModuleId,
    resume: DispatchResume<'static>,
    context: qa_console::views::Context,
) -> Option<DispatchCursor<'static>> {
    let (row, storage) = host.modules.as_mut()?.parts_mut(module)?;
    let mut services = host
        .runtime
        .engine_services(&mut host.console, storage, &mut row.scratch);
    let mut output = qa_console::commands::BindOutput::new(services.commands, context);
    let cursor = services
        .input
        .resume(resume, InputCapture::Caught, &mut output);
    let outcome = output.outcome();
    services.input_outcome(outcome);
    Some(cursor)
}

fn advance(
    host: &mut FrameHost,
    frame: &mut Activation,
    command: Arguments<'_>,
    access: TraceAccess<'_>,
    driver: &mut Driver<'_>,
) -> Result<GuestStep, ()> {
    let start = frame.start;
    let Some(functions) = host
        .modules
        .as_ref()
        .map(|modules| Arc::clone(&modules.functions))
    else {
        return Err(());
    };
    let (row, storage, traces) = host
        .modules
        .as_mut()
        .and_then(|modules| modules.trace_parts_mut(start.module, access))
        .ok_or(())?;
    let entering = frame.backend.is_none();
    if entering && let Program::QuakeC(qc) = &mut row.request.program {
        // A nested callback must observe its suspended parent's native stores,
        // including attached poses not yet published to the common columns.
        if !qc.vm.is_running()
            && !matches!(
                start.call,
                CallbackCall::Think {
                    prepared_native: Some(_),
                    ..
                }
            )
        {
            let mut services =
                host.runtime
                    .engine_services(&mut host.console, storage, &mut row.scratch);
            services.native_geometry = row.geometry;
            services.foreign_trace = Some(traces.borrow());
            if motion::sync_attachments(&mut qc.vm, &mut services, row.request.context).is_err() {
                row.counts.rejected += 1;
                return Err(());
            }
        }
        let native = match start.call {
            CallbackCall::Think {
                prepared_native: Some(subject),
                ..
            }
            | CallbackCall::Blocked { subject, .. } => Some(
                qc.vm
                    .weak_entity_offset(start.module, subject)
                    .map_err(|_| ())?,
            ),
            _ => native_self(&host.runtime, start.call.entity(), start.module, &qc.vm),
        };
        let world = native_self(&host.runtime, start.anchor, start.module, &qc.vm);
        let prepared = match start.call {
            CallbackCall::Client { event, time, .. } => native
                .ok_or(ServiceError::Entity)
                .and_then(|native| qc.vm.prepare_client_call(event, native, time)),
            CallbackCall::Touch { other, time, .. } => {
                native.ok_or(ServiceError::Entity).and_then(|native| {
                    let other = if other.namespace == ResourceNamespace::Module(start.module) {
                        qc.vm.weak_entity_offset(start.module, other)?
                    } else {
                        let admitted = qc.vm.admit_reference(
                            &host.runtime.server.entities,
                            start.context,
                            other,
                        )?;
                        let offset = admitted.offset();
                        frame.admitted = Some(admitted);
                        offset
                    };
                    qc.vm.prepare_touch_call(native, other, time)
                })
            }
            CallbackCall::Blocked { other, .. } => {
                native.ok_or(ServiceError::Entity).and_then(|native| {
                    let other = if other.namespace == ResourceNamespace::Module(start.module) {
                        qc.vm.weak_entity_offset(start.module, other)?
                    } else {
                        let admitted = qc.vm.admit_reference(
                            &host.runtime.server.entities,
                            start.context,
                            other,
                        )?;
                        let offset = admitted.offset();
                        frame.admitted = Some(admitted);
                        offset
                    };
                    qc.vm.prepare_blocked(native, other)?;
                    Ok(quakec::ClientCallRestore::default())
                })
            }
            CallbackCall::Think {
                prepared_native: Some(_),
                ..
            } => Ok(quakec::ClientCallRestore::default()),
            CallbackCall::Think { time, .. }
            | CallbackCall::Spawn { time, .. }
            | CallbackCall::Export { time, .. } => qc
                .vm
                .prepare_call(
                    native,
                    world,
                    time,
                    match start.call {
                        CallbackCall::Think { .. } => quakec::CallPhase::Think,
                        CallbackCall::Spawn { .. } => quakec::CallPhase::Spawn,
                        _ => quakec::CallPhase::Export,
                    },
                )
                .map(|()| quakec::ClientCallRestore::default())
                .map_err(|_| ServiceError::Memory),
            _ => return Err(()),
        };
        frame.backend = Some(Backend::QuakeC {
            run: None,
            restore: Some(prepared.map_err(|_| ())?),
            native,
        });
        row.counts.calls += 1;
        frame.counted = true;
    }
    let (mut services, mut client_view, mut composer, models) = host.runtime.engine_service_parts(
        &mut host.console,
        storage,
        &mut row.scratch,
        start.connection,
    );
    services.native_geometry = row.geometry;
    services.foreign_trace = Some(traces.borrow());
    crate::renderer::native::select_publisher(composer.as_deref_mut(), start.module, entering);
    let mut scene = crate::renderer::native::SceneBorrow::new(
        composer,
        models,
        &mut row.scene_role,
        services.cvars,
        start
            .connection
            .map(|(client, lifetime)| qa_render::effects::EffectClient { client, lifetime }),
    );
    scene.set_registration_available(driver.available());
    services.callbacks = Some(functions.as_ref());
    services.client_disconnect = row
        .client_lifecycle
        .and_then(|table| table.disconnect_callback);
    services.scene = Some(&mut scene);
    services.client_collision = Some(&mut row.collision_role);
    services.client_connection = client_view
        .as_mut()
        .map(|view| view as &mut dyn ClientConnection);
    if entering && !matches!(row.request.program, Program::QuakeC(_)) {
        row.counts.calls += 1;
        frame.counted = true;
    }
    let result = (|| match &mut row.request.program {
        Program::Qvm { vm, imports } => {
            let mut calls = QvmCalls {
                services: &mut services,
                table: imports,
                context: start.context,
                platform_time: start.platform_time,
                command,
                argument_selection: &mut frame.argument_selection,
                unknown: &mut row.unknown,
            };
            if entering {
                let outer = !vm.is_active();
                if outer {
                    qvm::SystemCalls::begin_call(&mut calls, vm).map_err(|_| ())?;
                }
                let mut words = [0; 10];
                words[0] = start.entry as i32;
                for (to, word) in words[1..].iter_mut().zip(start.words()) {
                    *to = word as u32 as i32;
                }
                let run = vm
                    .begin_callback(words, row.request.instruction_budget, false)
                    .map_err(|_| ())?;
                frame.backend = Some(Backend::Qvm { run, outer });
            }
            let Some(Backend::Qvm { run, outer }) = frame.backend.as_mut() else {
                return Err(());
            };
            let mut imported = match frame.reply.take() {
                Some((ReplyPoint::Qvm(continuation), value)) => Some(
                    calls
                        .resume_import(vm, continuation, value)
                        .map_err(|_| ())?,
                ),
                Some(_) => return Err(()),
                None => None,
            };
            let mut step = None;
            loop {
                if let Some(import) = imported.take() {
                    match import {
                        ImportStep::Return(value) => {
                            step = Some(vm.resume(run, value as i32).map_err(|_| ())?)
                        }
                        ImportStep::Host {
                            request,
                            continuation,
                        } => return Ok(GuestStep::Host(request, ReplyPoint::Qvm(continuation))),
                    }
                }
                match step
                    .take()
                    .map_or_else(|| vm.advance(run), Ok)
                    .map_err(|_| ())?
                {
                    qvm::Step::Syscall(syscall) => {
                        imported = Some(calls.import(vm, &syscall).map_err(|_| ())?)
                    }
                    qvm::Step::Returned(value) => {
                        if *outer {
                            vm.publish_entities(
                                calls.services,
                                start.context,
                                start.platform_time,
                                Some(start.entry),
                            )
                            .map_err(|_| ())?;
                        }
                        return Ok(GuestStep::Returned(ModuleResult::Qvm(value)));
                    }
                }
            }
        }
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        Program::Native { vm, imports } => {
            let mut calls = qa_compat::native::NativeCalls {
                services: &mut services,
                table: imports,
                context: start.context,
                platform_time: start.platform_time,
                command,
                argument_selection: &mut frame.argument_selection,
                native_player: start.native_player(),
                unknown: &mut row.unknown,
            };
            let result = (|| {
                if entering {
                    frame.backend = Some(Backend::Native {
                        run: vm.begin(&mut calls, start.entry, &start.words())?,
                        pending: None,
                    });
                }
                let Some(Backend::Native { run, pending }) = frame.backend.as_mut() else {
                    return Err(qa_compat::native::Error::Export);
                };
                match frame.reply.take() {
                    Some((ReplyPoint::Native, value)) => vm.resume(&mut calls, run, pending, value),
                    Some(_) => Err(qa_compat::native::Error::Export),
                    None => vm.advance(&mut calls, run),
                }
            })();
            match result {
                Ok(qa_compat::native::Step::Returned(value)) => {
                    Ok(GuestStep::Returned(ModuleResult::Native(value)))
                }
                Ok(qa_compat::native::Step::Host {
                    request,
                    pending: stopped,
                }) => {
                    let Some(Backend::Native { pending, .. }) = frame.backend.as_mut() else {
                        return Err(());
                    };
                    *pending = Some(stopped);
                    Ok(GuestStep::Host(request, ReplyPoint::Native))
                }
                Err(error) => {
                    if !error.recoverable() {
                        row.state = State::Failed;
                        if row.counts.traps == 0 {
                            let _ = (qa_compat::services::ENGINE_CALLS.console_formatted)(
                                calls.services,
                                format_args!(
                                    "module {}: native export {}: {error:?}\n",
                                    start.module.0, start.entry
                                ),
                            );
                        }
                    }
                    Err(())
                }
            }
        }
        Program::QuakeC(qc) => {
            let mut calls = QuakeCCalls {
                services: &mut services,
                table: &QUAKEC,
                context: start.context,
                platform_time: start.platform_time,
                developer: host.developer,
                unknown: &mut row.unknown,
            };
            if entering {
                let run = if qc.vm.is_running() {
                    qc.vm
                        .begin_callback(start.entry, row.request.instruction_budget, false)
                } else {
                    qc.vm
                        .begin_root(start.entry, row.request.instruction_budget, false)
                }
                .map_err(|_| ())?;
                let Some(Backend::QuakeC { run: saved, .. }) = frame.backend.as_mut() else {
                    return Err(());
                };
                *saved = Some(run);
            }
            let Some(Backend::QuakeC { run: Some(run), .. }) = frame.backend.as_mut() else {
                return Err(());
            };
            let mut imported = match frame.reply.take() {
                Some((ReplyPoint::QuakeC(continuation), value)) => Some(
                    calls
                        .resume_import(&mut qc.vm, continuation, value)
                        .map_err(|_| ())?,
                ),
                Some(_) => return Err(()),
                None => None,
            };
            let mut step = None;
            loop {
                if let Some(import) = imported.take() {
                    match import {
                        // The ABI wrote only the return cells in this builtin's
                        // signature. Void imports and return-vector tails survive.
                        QcImportStep::Returned => {
                            let access = quakec::EntityAccess::engine(calls.services);
                            step = Some(qc.vm.resume(run, &access).map_err(|_| ())?);
                        }
                        QcImportStep::Host {
                            request,
                            continuation,
                        } => {
                            // A nested host operation sees every admitted producer
                            // store, while explicit area links keep their old bounds.
                            if qc
                                .vm
                                .project_dirty_entities(calls.services, start.context)
                                .is_err()
                            {
                                // The continuation has not entered pending/reply yet,
                                // so cancellation cannot restore its saved globals.
                                calls
                                    .abort_import(&mut qc.vm, continuation)
                                    .map_err(|_| ())?;
                                return Err(());
                            }
                            return Ok(GuestStep::Host(request, ReplyPoint::QuakeC(continuation)));
                        }
                    }
                }
                let next = if let Some(step) = step.take() {
                    step
                } else {
                    let access = quakec::EntityAccess::engine(calls.services);
                    qc.vm.advance(run, &access).map_err(|_| ())?
                };
                match next {
                    quakec::Step::Builtin(builtin) => {
                        imported = Some(calls.import(&mut qc.vm, &builtin).map_err(|_| ())?);
                    }
                    quakec::Step::Returned(value) => {
                        finish_quakec(&mut qc.vm, calls.services, frame, true)?;
                        return Ok(GuestStep::Returned(ModuleResult::QuakeC(value)));
                    }
                }
            }
        }
    })();
    let termination = services
        .client_connection
        .as_deref()
        .and_then(|view| view.disconnected());
    frame.termination = frame.termination.or(termination);
    result
}

fn finish_quakec(
    vm: &mut quakec::Vm,
    services: &mut EngineServices<'_>,
    frame: &mut Activation,
    success: bool,
) -> Result<(), ()> {
    let Some(Backend::QuakeC {
        restore, native, ..
    }) = frame.backend.as_mut()
    else {
        return Err(());
    };
    let Some(restore) = restore.take() else {
        return Ok(());
    };
    let start = frame.start;
    let restored = vm.finish_client_call(restore);
    let velocity = if success
        && restored.is_ok()
        && start.context.native_rules == RuleSetId::Quake
        && matches!(
            start.call,
            CallbackCall::Client {
                event: ClientCallback::BeforeMove,
                ..
            }
        ) {
        (|| {
            let native = native.ok_or(ServiceError::Entity)?;
            let maximum = vm.max_velocity(services.cvars)?;
            motion::check_velocity(
                vm,
                services,
                start.context,
                native,
                start.call.entity(),
                maximum,
            )
        })()
    } else {
        Ok(())
    };
    // Publish admitted writes even after a trap or cancellation. Nested QC
    // callbacks finish this boundary before the suspended host operation resumes.
    let projection = if matches!(start.call, CallbackCall::Think { .. }) {
        vm.project_dirty_entities(services, start.context)
    } else {
        vm.project_entities(services, start.context)
    };
    restored.and(velocity).and(projection).map_err(|_| ())
}

fn cancel(
    host: &mut FrameHost,
    frame: &mut Activation,
    command: Arguments<'_>,
    access: TraceAccess<'_>,
) {
    let point = frame
        .pending
        .as_mut()
        .and_then(|pending| pending.point.take());
    let reply = frame.reply.take().map(|(point, _)| point);
    // A host stop resides in pending or reply, never both. Take ownership before
    // dropping the traversal so native saved globals cannot be lost on abort.
    let point = point.or(reply);
    let pending = match frame.pending.take() {
        Some(Pending {
            operation: Operation::Triggers {
                touch: Some(touch), ..
            },
            ..
        }) => {
            if touch.cancel(host).is_err()
                && let Some(row) = host.module_row_mut(frame.start.module)
            {
                row.counts.rejected += 1;
            }
            None
        }
        pending => pending,
    };
    if let Some(Pending {
        operation:
            Operation::Clear {
                mut cursor,
                mut resume,
                context,
                ..
            },
        ..
    }) = pending
    {
        loop {
            if let Some(next) = resume.take() {
                cursor = input_resume(host, frame.start.module, next, context);
            }
            let Some(next) = cursor.take() else {
                break;
            };
            match input_advance(host, frame.start.module, next, context) {
                Some(DispatchStep::Atom { resume: next, .. }) => resume = Some(next),
                _ => break,
            }
        }
    }
    if let Some(admitted) = frame.admitted.take()
        && contacts::release(host, admitted).is_err()
        && let Some(row) = host.module_row_mut(frame.start.module)
    {
        row.counts.rejected += 1;
    }
    let Some((row, storage, traces)) = host
        .modules
        .as_mut()
        .and_then(|modules| modules.trace_parts_mut(frame.start.module, access))
    else {
        return;
    };
    let mut services = host
        .runtime
        .engine_services(&mut host.console, storage, &mut row.scratch);
    services.native_geometry = row.geometry;
    services.foreign_trace = Some(traces.borrow());
    services.storage.abort_client_callbacks(frame.start.module);
    match (&mut row.request.program, frame.backend.as_mut()) {
        (Program::QuakeC(qc), Some(Backend::QuakeC { run, .. })) => {
            if let Some(run) = run {
                let _ = qc.vm.abort(run);
            }
            if let Some(ReplyPoint::QuakeC(continuation)) = point {
                let mut calls = QuakeCCalls {
                    services: &mut services,
                    table: &QUAKEC,
                    context: frame.start.context,
                    platform_time: frame.start.platform_time,
                    developer: host.developer,
                    unknown: &mut row.unknown,
                };
                let _ = calls.abort_import(&mut qc.vm, continuation);
            }
            let _ = finish_quakec(&mut qc.vm, &mut services, frame, false);
        }
        (Program::Qvm { vm, .. }, Some(Backend::Qvm { run, .. })) => {
            let _ = vm.abort(run);
        }
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        (Program::Native { vm, imports }, Some(Backend::Native { run, .. })) => {
            let mut calls = qa_compat::native::NativeCalls {
                services: &mut services,
                table: imports,
                context: frame.start.context,
                platform_time: frame.start.platform_time,
                command,
                argument_selection: &mut frame.argument_selection,
                native_player: frame.start.native_player(),
                unknown: &mut row.unknown,
            };
            if vm.is_active() {
                let error = vm.abort(&mut calls, run, ServiceError::Entity);
                if !error.recoverable() {
                    row.state = State::Failed;
                }
            }
        }
        _ => {}
    }
}

fn finish(
    host: &mut FrameHost,
    frame: &Activation,
    result: Result<ModuleResult, ()>,
) -> ExecutionResult {
    let accepted = if let Some(row) = host.module_row_mut(frame.start.module) {
        if frame.counted {
            row.counts.record(result)
        } else {
            false
        }
    } else {
        false
    };
    if let Some((row, _)) = host
        .modules
        .as_mut()
        .and_then(|modules| modules.parts_mut(frame.start.module))
    {
        Module::retain_active_batch(&mut row.client_batch, &host.runtime.server);
    }
    if accepted { result.map(Some) } else { Err(()) }
}

#[cfg(test)]
#[path = "../../tests/modules/quakec_yield.rs"]
mod tests;
