//! Synchronous module calls use one load-reserved, iterative activation stack.
use super::{Module, ModuleResult, Program, State, context};
use crate::host::FrameHost;
use qa_compat::{
    abi::{HostRequest, ImportContinuation, ImportStep, QvmCalls},
    qvm,
    services::{CallContext, CallError as ServiceError, ClientConnection},
};
use qa_core::{
    primitives::{CallbackCall, ClientId, CommandArgs, EntityId, ModuleId},
    sys_events::EventTime,
};
use qa_input::{DispatchCursor, DispatchResume, DispatchStep, InputCapture};
use qa_session::dispatch::CallOutcome;
use std::sync::Arc;

pub(super) enum ModuleDispatch {
    Completed(bool),
    Guest(GuestStart),
}
impl CallOutcome<FrameHost> for ModuleDispatch {
    fn rejected() -> Self {
        Self::Completed(false)
    }
    fn complete(self, host: &mut FrameHost, call: CallbackCall<'_>) -> bool {
        match self {
            Self::Completed(accepted) => accepted,
            Self::Guest(start) => {
                let command = match call {
                    CallbackCall::Export { command, .. } | CallbackCall::Client { command, .. } => {
                        command
                    }
                    _ => CommandArgs::default(),
                };
                drive(host, start, command)
            }
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct GuestStart {
    module: ModuleId,
    entity: EntityId,
    anchor: EntityId,
    entry: u32,
    words: [u64; 9],
    context: CallContext,
    platform_time: EventTime,
    native_player: Option<ClientId>,
    connection: Option<(ClientId, EntityId)>,
}

enum Backend {
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
    Qvm(ImportContinuation),
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    Native,
}
struct Pending {
    point: Option<ReplyPoint>,
    operation: Operation,
}
enum Operation {
    Callback(Option<qa_compat::abi::CallbackRequest>),
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
}
pub(super) struct Activation {
    start: GuestStart,
    backend: Option<Backend>,
    counted: bool,
    pending: Option<Pending>,
    reply: Option<(ReplyPoint, u64)>,
    child: Option<bool>,
    input_callback: bool,
    termination: Option<qa_compat::services::ClientTermination>,
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
            input_callback,
            termination: None,
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
    Reply,
    Abort,
    FailedInput(ModuleId),
    Finished(Result<ModuleResult, ()>),
}

pub(super) fn start(
    host: &mut FrameHost,
    module: ModuleId,
    entry: u32,
    call: CallbackCall<'_>,
) -> ModuleDispatch {
    let (time, words, native_player) = match call {
        CallbackCall::Think { time, .. } => {
            let mut words = [0; 9];
            words[0] = time.milliseconds() as u64;
            (time, words, None)
        }
        CallbackCall::Export {
            time,
            arguments,
            client,
            ..
        } => (time, arguments, client),
        CallbackCall::Client {
            time,
            arguments,
            client,
            ..
        } => (time, arguments, Some(client)),
        _ => return ModuleDispatch::Completed(false),
    };
    let Some(row) = host.module_row(module) else {
        return ModuleDispatch::Completed(false);
    };
    if !row.state.can_invoke() || matches!(row.request.program, Program::QuakeC(_)) {
        return ModuleDispatch::Completed(false);
    }
    let start = GuestStart {
        module,
        entity: call.entity(),
        anchor: row.request.anchor,
        entry,
        words,
        context: context(row.request.context, time),
        platform_time: host.runtime.input_time,
        native_player,
        connection: row.client_connection,
    };
    if valid(host, start) {
        ModuleDispatch::Guest(start)
    } else {
        ModuleDispatch::Completed(false)
    }
}

fn valid(host: &FrameHost, start: GuestStart) -> bool {
    let Some(row) = host.module_row(start.module) else {
        return false;
    };
    if !row.state.can_invoke()
        || row.request.anchor != start.anchor
        || row.client_connection != start.connection
    {
        return false;
    }
    for entity in [start.anchor, start.entity] {
        let Some(slot) = host.runtime.server.entities.resolve(entity) else {
            return false;
        };
        if host.runtime.server.entities.columns.owner[slot] != start.module {
            return false;
        }
    }
    if let Some((id, lifetime)) = start.connection {
        if start.native_player.is_some_and(|requested| requested != id)
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

fn drive(host: &mut FrameHost, start: GuestStart, command: CommandArgs<'_>) -> bool {
    let Some(modules) = host.modules.as_mut() else {
        return false;
    };
    let mut arena = std::mem::take(&mut modules.execution);
    if arena.is_empty() {
        modules.execution = arena;
        return false;
    }
    arena[0] = Some(Activation::new(start, false));
    let mut depth = 1;
    let result = loop {
        let Some(mut frame) = arena[depth - 1].take() else {
            break false;
        };
        let cleanup = frame
            .pending
            .as_ref()
            .is_some_and(|pending| matches!(pending.operation, Operation::Cleanup { .. }));
        let finished = if !cleanup && !valid(host, frame.start) {
            cancel(host, &mut frame, command);
            completing(host, &mut frame, Err(()));
            None
        } else if frame.pending.is_some() {
            match operation(host, &mut frame) {
                OperationStep::Continue => None,
                OperationStep::Reply => {
                    if let Some(Pending {
                        point: Some(point), ..
                    }) = frame.pending.take()
                    {
                        frame.reply = Some((point, 0));
                        None
                    } else {
                        cancel(host, &mut frame, command);
                        completing(host, &mut frame, Err(()));
                        None
                    }
                }
                OperationStep::Call(ModuleDispatch::Completed(accepted)) => {
                    frame.child = Some(accepted);
                    None
                }
                OperationStep::Call(ModuleDispatch::Guest(start)) => {
                    if depth == arena.len() {
                        // The reserved envelope follows actual backend stack limits.
                        // A missing slot is never another arbitrary guest depth cap.
                        frame.child = Some(false);
                    } else {
                        let input = frame.pending.as_ref().is_some_and(|pending| {
                            matches!(pending.operation, Operation::Clear { .. })
                        });
                        arena[depth - 1] = Some(frame);
                        arena[depth] = Some(Activation::new(start, input));
                        depth += 1;
                        continue;
                    }
                    None
                }
                OperationStep::Abort => {
                    cancel(host, &mut frame, command);
                    completing(host, &mut frame, Err(()));
                    None
                }
                OperationStep::FailedInput(module) => {
                    if let Some(row) = host.module_row(module).filter(|row| row.state.can_invoke())
                    {
                        let start = GuestStart {
                            module,
                            entity: row.request.anchor,
                            anchor: row.request.anchor,
                            entry: 0,
                            words: [0; 9],
                            context: row.request.context,
                            platform_time: host.runtime.input_time,
                            native_player: None,
                            connection: row.client_connection,
                        };
                        let mut cleanup = Activation::new(start, true);
                        completing(host, &mut cleanup, Err(()));
                        if depth < arena.len() {
                            arena[depth - 1] = Some(frame);
                            arena[depth] = Some(cleanup);
                            depth += 1;
                            continue;
                        }
                    }
                    frame.child = Some(false);
                    None
                }
                OperationStep::Finished(result) => Some(finish(host, &frame, result)),
            }
        } else {
            match advance(host, &mut frame, command) {
                Ok(GuestStep::Host(request, point)) => {
                    frame.pending = Some(Pending {
                        point: Some(point),
                        operation: match request {
                            HostRequest::Callback(request) => Operation::Callback(Some(request)),
                            HostRequest::ClearKeys {
                                seat,
                                context,
                                time,
                            } => Operation::Clear {
                                cursor: Some(DispatchCursor::clear_seat(seat, time)),
                                resume: None,
                                context,
                                time,
                                recipient: None,
                            },
                        },
                    });
                    None
                }
                Ok(GuestStep::Returned(value)) => {
                    completing(host, &mut frame, Ok(value));
                    None
                }
                Err(()) => {
                    cancel(host, &mut frame, command);
                    completing(host, &mut frame, Err(()));
                    None
                }
            }
        };
        if let Some(accepted) = finished {
            depth -= 1;
            if depth == 0 {
                break accepted;
            }
            if let Some(parent) = arena[depth - 1].as_mut() {
                parent.child = Some(accepted);
            }
        } else {
            arena[depth - 1] = Some(frame);
        }
    };
    // Every exceptional exit drains owned tokens in LIFO order before returning
    // the one reserved arena. Ordinary errors take the same completion path.
    while depth != 0 {
        depth -= 1;
        if let Some(mut frame) = arena[depth].take() {
            cancel(host, &mut frame, command);
            finish(host, &frame, Err(()));
        }
    }
    if let Some(modules) = host.modules.as_mut() {
        modules.execution = arena;
    }
    result
}

fn completing(host: &mut FrameHost, frame: &mut Activation, result: Result<ModuleResult, ()>) {
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
) -> Option<OperationStep> {
    let (callback, call) = host.begin_client_disconnect(id, lifetime)?;
    let Some(modules) = host.modules.as_ref() else {
        host.finish_client_disconnect(id, lifetime);
        return None;
    };
    let functions = Arc::clone(&modules.functions);
    *retiring = Some((id, lifetime));
    Some(OperationStep::Call(
        functions
            .dispatch(host, callback, call)
            .unwrap_or(ModuleDispatch::Completed(false)),
    ))
}

fn operation(host: &mut FrameHost, frame: &mut Activation) -> OperationStep {
    let Some(pending) = frame.pending.as_mut() else {
        return OperationStep::Abort;
    };
    match &mut pending.operation {
        Operation::Callback(request) => {
            if let Some(request) = request.take() {
                let Some(modules) = host.modules.as_ref() else {
                    return OperationStep::Abort;
                };
                let functions = Arc::clone(&modules.functions);
                OperationStep::Call(
                    functions
                        .dispatch_resolved(host, request.target, request.call)
                        .unwrap_or(ModuleDispatch::Completed(false)),
                )
            } else if frame.child.take() == Some(true) {
                return OperationStep::Reply;
            } else {
                return OperationStep::Abort;
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
                frame.child.take();
                host.finish_client_disconnect(id, lifetime);
            }
            if let Some(termination) = termination.take()
                && let Some((id, lifetime)) = frame.start.connection
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
                        if let Some(step) = disconnect(host, id, lifetime, retiring) {
                            return step;
                        }
                    }
                }
            }
            if *failed {
                while *next_client < host.runtime.server.clients.len() {
                    let id = ClientId(*next_client as u32);
                    *next_client += 1;
                    if !host.module_recipient(frame.start.module, id) {
                        continue;
                    }
                    let lifetime = host.runtime.server.clients[id.0 as usize].entity;
                    if let Some(step) = disconnect(host, id, lifetime, retiring) {
                        return step;
                    }
                }
                // Keep native client identities live through their disconnect
                // callbacks. The sole failed-module owner retires rows afterward.
                host.retire_module_state(frame.start.module);
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
            if let Some(accepted) = frame.child.take() {
                if !accepted && let Some(module) = recipient.take() {
                    return OperationStep::FailedInput(module);
                }
                *recipient = None;
                if !valid(host, frame.start) {
                    return OperationStep::Abort;
                }
            }
            if let Some(resume) = resume.take() {
                match input_resume(host, frame.start.module, resume, *context) {
                    Some(next) => *cursor = Some(next),
                    None => return OperationStep::Abort,
                }
            }
            let Some(current) = cursor.take() else {
                return OperationStep::Abort;
            };
            match input_advance(host, frame.start.module, current, *context) {
                Some(DispatchStep::Done) => return OperationStep::Reply,
                Some(DispatchStep::Atom { atom, resume: next }) => {
                    *resume = Some(next);
                    host.console
                        .sync_editor_input(&mut host.runtime.input, *time);
                    if let Some((module, callback, call)) = host.input_call(atom, true) {
                        *recipient = Some(module);
                        let Some(modules) = host.modules.as_ref() else {
                            return OperationStep::Abort;
                        };
                        let functions = Arc::clone(&modules.functions);
                        OperationStep::Call(
                            functions
                                .dispatch(host, callback, call)
                                .unwrap_or(ModuleDispatch::Completed(false)),
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
    command: CommandArgs<'_>,
) -> Result<GuestStep, ()> {
    let start = frame.start;
    let Some(functions) = host
        .modules
        .as_ref()
        .map(|modules| Arc::clone(&modules.functions))
    else {
        return Err(());
    };
    let (row, storage) = host
        .modules
        .as_mut()
        .and_then(|modules| modules.parts_mut(start.module))
        .ok_or(())?;
    let entering = frame.backend.is_none();
    let (mut services, mut client_view, mut composer, models) = host.runtime.engine_service_parts(
        &mut host.console,
        storage,
        &mut row.scratch,
        start.connection,
    );
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
    services.callbacks = Some(functions.as_ref());
    services.client_disconnect = row
        .client_lifecycle
        .and_then(|table| table.disconnect_callback);
    services.scene = Some(&mut scene);
    services.client_collision = Some(&mut row.collision_role);
    services.client_connection = client_view
        .as_mut()
        .map(|view| view as &mut dyn ClientConnection);
    if entering {
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
                command: command.argv,
                unknown: &mut row.unknown,
            };
            if entering {
                let outer = !vm.is_active();
                if outer {
                    qvm::SystemCalls::begin_call(&mut calls, vm).map_err(|_| ())?;
                }
                let mut words = [0; 10];
                words[0] = start.entry as i32;
                for (to, word) in words[1..].iter_mut().zip(start.words) {
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
                #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
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
                command: command.argv,
                raw_args: command.raw,
                native_player: start.native_player,
                unknown: &mut row.unknown,
            };
            let result = (|| {
                if entering {
                    frame.backend = Some(Backend::Native {
                        run: vm.begin(&mut calls, start.entry, &start.words)?,
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
        Program::QuakeC(_) => Err(()),
    })();
    let termination = services
        .client_connection
        .as_deref()
        .and_then(|view| view.disconnected());
    frame.termination = frame.termination.or(termination);
    result
}

fn cancel(host: &mut FrameHost, frame: &mut Activation, command: CommandArgs<'_>) {
    if let Some(Pending {
        operation:
            Operation::Clear {
                mut cursor,
                mut resume,
                context,
                ..
            },
        ..
    }) = frame.pending.take()
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
    let Some((row, storage)) = host
        .modules
        .as_mut()
        .and_then(|modules| modules.parts_mut(frame.start.module))
    else {
        return;
    };
    let mut services = host
        .runtime
        .engine_services(&mut host.console, storage, &mut row.scratch);
    services.storage.abort_client_callbacks(frame.start.module);
    match (&mut row.request.program, frame.backend.as_mut()) {
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
                command: command.argv,
                raw_args: command.raw,
                native_player: frame.start.native_player,
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

fn finish(host: &mut FrameHost, frame: &Activation, result: Result<ModuleResult, ()>) -> bool {
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
    accepted
}
