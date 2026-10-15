//! Contact traversal owns its cursor and releases spatial borrows per callback.
use super::{FrameHost, Program, clients::ClientCommandError};
use qa_compat::abi::CallbackRequest;
use qa_core::primitives::{
    CallbackCall, EntityId, ModuleId, NativeEntityRef, ResourceNamespace, ThinkTime,
};
use qa_session::dispatch::{CallError, CallOutcome};
use qa_world::area::{AreaCursor, LinkFlags};
use std::sync::Arc;

pub(super) fn reference(
    host: &FrameHost,
    entity: EntityId,
) -> Result<NativeEntityRef, ClientCommandError> {
    let entities = &host.runtime.server.entities;
    let slot = entities
        .resolve(entity)
        .ok_or(ClientCommandError::Inactive)?;
    let native = entities.columns.native_entity[slot].ok_or(ClientCommandError::InvalidSlot)?;
    let ordinal = u32::try_from(native.slot).map_err(|_| ClientCommandError::InvalidSlot)?;
    Ok(NativeEntityRef::entity(
        ResourceNamespace::Module(native.module),
        ordinal,
    ))
}

/// Rebind only at a native callback boundary. The preceding callback can
/// remove and reuse a row while retaining its original native pointer.
pub(super) fn current(
    host: &FrameHost,
    reference: NativeEntityRef,
) -> Result<Option<EntityId>, ClientCommandError> {
    let ResourceNamespace::Module(module) = reference.namespace else {
        return Err(ClientCommandError::DifferentModule);
    };
    let row = host
        .module_row(module)
        .ok_or(ClientCommandError::MissingModule)?;
    let Program::QuakeC(qc) = &row.request.program else {
        return Err(ClientCommandError::NeedsNativeModule);
    };
    let native = qa_core::primitives::NativeEntity {
        module,
        slot: i32::try_from(reference.ordinal).map_err(|_| ClientCommandError::InvalidSlot)?,
    };
    qc.vm
        .bound_entity(&host.runtime.server.entities, native)
        .map_err(ClientCommandError::Usercmd)
}

/// One owned traversal serves both ordinary provider calls and a stopped QC
/// builtin. No link or entity borrow survives a returned callback request.
pub(super) struct TriggerRun {
    mover: NativeEntityRef,
    time: ThinkTime,
    cursor: Option<AreaCursor>,
}
impl TriggerRun {
    pub(super) fn begin(
        host: &FrameHost,
        mover: NativeEntityRef,
        time: ThinkTime,
    ) -> Result<Self, ClientCommandError> {
        let cursor = current(host, mover)?
            .and_then(|entity| host.runtime.server.area.bounds(entity))
            .map(|bounds| {
                host.runtime
                    .server
                    .area
                    .query_cursor(bounds, LinkFlags::TRIGGER)
            });
        Ok(Self {
            mover,
            time,
            cursor,
        })
    }

    pub(super) fn next(
        &mut self,
        host: &mut FrameHost,
    ) -> Result<Option<PreparedTouch>, ClientCommandError> {
        let ResourceNamespace::Module(module) = self.mover.namespace else {
            return Err(ClientCommandError::DifferentModule);
        };
        while let Some(cursor) = self.cursor.as_mut() {
            // The previous callback can free/reuse or teleport the native row.
            // Refresh both its common lifetime and bounds before taking next.
            let Some(entity) = current(host, self.mover)? else {
                break;
            };
            let Some(bounds) = host.runtime.server.area.bounds(entity) else {
                break;
            };
            cursor.set_bounds(bounds);
            let Some(linked) =
                cursor.next(&host.runtime.server.area, &host.runtime.server.entities)
            else {
                break;
            };
            let target = linked.id;
            if target != entity
                && let Some(touch) = prepare_touch(
                    host,
                    module,
                    target,
                    self.mover,
                    Some(self.time),
                    false,
                    true,
                )?
            {
                return Ok(Some(touch));
            }
        }
        if let Some(cursor) = self.cursor.take()
            && cursor.interrupted()
            && let Some(row) = host.module_row_mut(module)
        {
            row.counts.rejected += 1;
        }
        Ok(None)
    }
}

pub(super) fn triggers(
    host: &mut FrameHost,
    module: ModuleId,
    entity: EntityId,
    time: ThinkTime,
) -> Result<(), ClientCommandError> {
    let mover = reference(host, entity)?;
    if mover.namespace != ResourceNamespace::Module(module) {
        return Err(ClientCommandError::DifferentModule);
    }
    let mut run = TriggerRun::begin(host, mover, time)?;
    while let Some(touch) = run.next(host)? {
        touch.invoke(host)?;
    }
    Ok(())
}

/// The checked callback and its one optional post-acceptance stamp. The weak
/// other survives removal; callback-self is revalidated by FunctionTable.
pub(super) struct PreparedTouch {
    request: CallbackRequest,
    stamp: Option<(ModuleId, usize)>,
}
impl PreparedTouch {
    pub(super) fn request(&self) -> CallbackRequest {
        self.request
    }

    pub(super) fn complete(
        self,
        host: &mut FrameHost,
        accepted: bool,
    ) -> Result<(), ClientCommandError> {
        if !accepted {
            return Err(ClientCommandError::Dispatch(CallError::Rejected));
        }
        if let Some((module, index)) = self.stamp {
            host.module_row_mut(module)
                .ok_or(ClientCommandError::MissingModule)?
                .client_contacts
                .mark(index);
        }
        Ok(())
    }

    fn invoke(self, host: &mut FrameHost) -> Result<(), ClientCommandError> {
        let functions = host
            .modules
            .as_ref()
            .map(|modules| Arc::clone(&modules.functions))
            .ok_or(ClientCommandError::MissingModule)?;
        let request = self.request;
        let accepted = functions
            .dispatch_resolved(host, request.target, request.call)
            .map_err(ClientCommandError::Dispatch)?
            .complete(host, request.call);
        self.complete(host, accepted)
    }
}

fn prepare_touch(
    host: &FrameHost,
    module: ModuleId,
    entity: EntityId,
    other: NativeEntityRef,
    time: Option<ThinkTime>,
    deduplicate: bool,
    trigger_only: bool,
) -> Result<Option<PreparedTouch>, ClientCommandError> {
    let entities = &host.runtime.server.entities;
    let Some(slot) = entities.resolve(entity) else {
        return Ok(None);
    };
    let owner = entities.columns.owner[slot];
    let native = entities.columns.native_entity[slot].ok_or(ClientCommandError::InvalidSlot)?;
    // Foreign QC other values still require their own native namespace adapter.
    // Keep the existing same-module admission rather than inventing world zero.
    if native.module != module || native.module != owner {
        return Err(ClientCommandError::DifferentModule);
    }
    let index = usize::try_from(native.slot).map_err(|_| ClientCommandError::InvalidSlot)?;
    let row = host
        .module_row(owner)
        .ok_or(ClientCommandError::MissingModule)?;
    if deduplicate {
        if index >= row.client_contacts.len() {
            return Err(ClientCommandError::InvalidSlot);
        }
        if row.client_contacts.contains(index) {
            return Ok(None);
        }
    }
    let Program::QuakeC(qc) = &row.request.program else {
        return Err(ClientCommandError::NeedsNativeModule);
    };
    let offset = index
        .checked_mul(qc.vm.stride)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or(ClientCommandError::InvalidSlot)?;
    let Some(callback) = qc
        .vm
        .touch_callback(offset, trigger_only)
        .map_err(ClientCommandError::Usercmd)?
    else {
        return Ok(None);
    };
    let functions = host
        .modules
        .as_ref()
        .map(|modules| &modules.functions)
        .ok_or(ClientCommandError::MissingModule)?;
    let target = functions
        .resolve(entities, callback, entity)
        .map_err(ClientCommandError::Dispatch)?;
    Ok(Some(PreparedTouch {
        request: CallbackRequest {
            target,
            call: CallbackCall::Touch {
                entity,
                other,
                time,
            },
        },
        stamp: deduplicate.then_some((module, index)),
    }))
}

pub(super) fn touch(
    host: &mut FrameHost,
    module: ModuleId,
    entity: EntityId,
    other: NativeEntityRef,
    time: Option<ThinkTime>,
    deduplicate: bool,
    trigger_only: bool,
) -> Result<(), ClientCommandError> {
    if let Some(touch) =
        prepare_touch(host, module, entity, other, time, deduplicate, trigger_only)?
    {
        touch.invoke(host)?;
    }
    Ok(())
}

pub(super) fn impact(
    host: &mut FrameHost,
    module: ModuleId,
    mover_entity: Option<EntityId>,
    mover: NativeEntityRef,
    obstacle: NativeEntityRef,
    time: ThinkTime,
) -> Result<(), ClientCommandError> {
    let restore = super::edit_quakec(host, module, |vm, _, _| vm.prepare_impact(time))?;
    let execution = (|| {
        if let Some(entity) = mover_entity {
            touch(host, module, entity, obstacle, None, false, false)?;
        }
        if let Some(entity) = current(host, obstacle)? {
            touch(host, module, entity, mover, None, false, false)?;
        }
        Ok(())
    })();
    let restored = super::edit_quakec(host, module, |vm, _, _| vm.finish_client_call(restore));
    execution.and(restored)
}
