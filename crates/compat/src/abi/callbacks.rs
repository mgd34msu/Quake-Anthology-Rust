use super::{CallError, CallValue, ENGINE_CALLS, Invocation};
use qa_core::primitives::{CallbackCall, CommandArgs};
use qa_session::dispatch::ResolvedCall;

#[derive(Clone, Copy, Debug)]
pub struct CallbackRequest {
    pub target: ResolvedCall,
    pub call: CallbackCall<'static>,
}

#[derive(Clone, Copy, Debug)]
pub enum HostRequest {
    Callback(CallbackRequest),
    ClearKeys {
        seat: qa_core::sys_events::SeatId,
        context: qa_console::views::Context,
        time: qa_core::sys_events::EventTime,
    },
}

#[derive(Debug)]
pub enum ImportStep {
    Return(u64),
    Host {
        request: HostRequest,
        continuation: ImportContinuation,
    },
}

#[derive(Debug)]
pub enum ImportContinuation {
    Return(u64),
    DropClient {
        target: crate::services::DropClientTarget,
        reason_address: u64,
    },
}
impl ImportContinuation {
    pub fn resume(
        self,
        invocation: &mut Invocation<'_, '_, '_>,
        _returned: u64,
    ) -> Result<ImportStep, CallError> {
        match self {
            Self::Return(value) => Ok(ImportStep::Return(value)),
            Self::DropClient {
                target,
                reason_address,
            } => {
                let reason = invocation.memory.cstring(reason_address)?;
                (ENGINE_CALLS.finish_drop_client)(invocation.services, target, reason)?;
                Ok(ImportStep::Return(0))
            }
        }
    }
}

pub(super) fn drop_client(c: &mut Invocation<'_, '_, '_>) -> Result<CallValue, CallError> {
    let native = qa_core::primitives::NativeEntity {
        module: c.context.module,
        slot: c.arg(0)? as u32 as i32,
    };
    // Invalid and already-closing ordinals do not execute the pointer operand.
    let Some(target) = (ENGINE_CALLS.drop_client_target)(c.services, native, c.platform_time)?
    else {
        return Ok(CallValue::Word(0));
    };
    let entity = target
        .client
        .map(|(_, lifetime)| lifetime)
        .or_else(|| c.services.storage.module_lifetime(native.module))
        .ok_or(CallError::Entity)?;
    let callback = c.services.client_disconnect.ok_or(CallError::Unsupported)?;
    let resolved = c
        .services
        .callbacks
        .ok_or(CallError::Unsupported)?
        .resolve_callback(&c.services.server.entities, native.module, callback, entity)
        .map_err(|_| CallError::Entity)?;
    let reason_address = c.required_pointer(1)?;
    let reason = c.memory.cstring(reason_address)?;
    (ENGINE_CALLS.begin_drop_client)(c.services, target, reason, c.platform_time)?;
    let mut arguments = [0; 9];
    arguments[0] = native.slot as u32 as u64;
    let call = if let Some((client, _)) = target.client {
        CallbackCall::Client {
            entity,
            client,
            event: qa_core::primitives::ClientCallback::Disconnect,
            time: c.context.clock,
            arguments,
            command: CommandArgs::default(),
        }
    } else {
        CallbackCall::Export {
            entity,
            time: c.context.clock,
            arguments,
            client: None,
            command: CommandArgs::default(),
        }
    };
    Ok(CallValue::Host(
        HostRequest::Callback(CallbackRequest {
            target: resolved,
            call,
        }),
        ImportContinuation::DropClient {
            target,
            reason_address,
        },
    ))
}

impl CallbackRequest {
    pub(crate) fn validate(
        &self,
        services: &crate::services::EngineServices<'_>,
        module: qa_core::primitives::ModuleId,
    ) -> Result<(), CallError> {
        if self.target.module() != module || self.call.entity() != self.target.entity() {
            return Err(CallError::Entity);
        }
        let slot = services
            .server
            .entities
            .resolve(self.target.entity())
            .ok_or(CallError::Entity)?;
        if services.server.entities.columns.owner[slot] != module {
            return Err(CallError::Entity);
        }
        Ok(())
    }
    pub(crate) fn words(
        &self,
    ) -> Result<
        (
            qa_core::primitives::ThinkTime,
            [u64; 9],
            Option<qa_core::primitives::ClientId>,
        ),
        CallError,
    > {
        match self.call {
            CallbackCall::Client {
                time,
                arguments,
                client,
                ..
            } => Ok((time, arguments, Some(client))),
            CallbackCall::Export {
                time,
                arguments,
                client,
                ..
            } => Ok((time, arguments, client)),
            _ => Err(CallError::Unsupported),
        }
    }
}
