//! Synchronous entity contacts release all spatial and module borrows per call.
use super::{FrameHost, Program, clients::ClientCommandError};
use qa_core::primitives::{
    CallbackCall, EntityId, ModuleId, NativeEntityRef, ResourceNamespace, ThinkTime,
};
use qa_world::area::LinkFlags;

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

pub(super) fn triggers(
    host: &mut FrameHost,
    module: ModuleId,
    mut entity: EntityId,
    time: ThinkTime,
) -> Result<(), ClientCommandError> {
    let other = reference(host, entity)?;
    let Some(bounds) = host.runtime.server.area.bounds(entity) else {
        return Ok(());
    };
    let mut cursor = host
        .runtime
        .server
        .area
        .query_cursor(bounds, LinkFlags::TRIGGER);
    loop {
        let Some(bound) = current(host, other)? else {
            break;
        };
        entity = bound;
        let entities = &host.runtime.server.entities;
        let Some(bounds) = host.runtime.server.area.bounds(entity) else {
            break;
        };
        cursor.set_bounds(bounds);
        let Some(linked) = cursor.next(&host.runtime.server.area, entities) else {
            break;
        };
        let target = linked.id;
        if target != entity {
            touch(host, module, target, other, Some(time), false, true)?;
        }
    }
    if cursor.interrupted()
        && let Some(row) = host.module_row_mut(module)
    {
        row.counts.rejected += 1;
    }
    Ok(())
}

/// Callback-self is generation checked. Other is a weak native identity so
/// removing the mover in its touch cannot erase the obstacle's other argument.
pub(super) fn touch(
    host: &mut FrameHost,
    module: ModuleId,
    entity: EntityId,
    other: NativeEntityRef,
    time: Option<ThinkTime>,
    deduplicate: bool,
    trigger_only: bool,
) -> Result<(), ClientCommandError> {
    let Some(slot) = host.runtime.server.entities.resolve(entity) else {
        return Ok(());
    };
    let native = host.runtime.server.entities.columns.native_entity[slot]
        .ok_or(ClientCommandError::InvalidSlot)?;
    if native.module != module {
        return Err(ClientCommandError::DifferentModule);
    }
    let index = usize::try_from(native.slot).map_err(|_| ClientCommandError::InvalidSlot)?;
    let row = host
        .module_row(module)
        .ok_or(ClientCommandError::MissingModule)?;
    if deduplicate {
        if index >= row.client_contacts.len() {
            return Err(ClientCommandError::InvalidSlot);
        }
        if row.client_contacts.contains(index) {
            return Ok(());
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
        return Ok(());
    };
    host.call_module(
        callback,
        CallbackCall::Touch {
            entity,
            other,
            time,
        },
    )
    .map_err(ClientCommandError::Dispatch)?;
    if deduplicate {
        host.module_row_mut(module)
            .ok_or(ClientCommandError::MissingModule)?
            .client_contacts
            .mark(index);
    }
    Ok(())
}

pub(super) fn impact(
    host: &mut FrameHost,
    module: ModuleId,
    mover: NativeEntityRef,
    obstacle: NativeEntityRef,
    time: ThinkTime,
) -> Result<(), ClientCommandError> {
    let restore = super::edit_quakec(host, module, |vm, _, _| vm.prepare_impact(time))?;
    let execution = (|| {
        if let Some(entity) = current(host, mover)? {
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
