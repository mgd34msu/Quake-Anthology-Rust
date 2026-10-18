//! Contact traversal owns its cursor and releases spatial borrows per callback.
use super::{FrameHost, Program, clients::ClientCommandError, execution::TraceAccess};
use qa_compat::{abi::CallbackRequest, quakec::AdmittedEntity};
use qa_core::primitives::{
    CallbackCall, EntityId, ModuleId, NativeEntityRef, ResourceNamespace, ThinkTime,
};
use qa_session::dispatch::CallError;
use qa_world::area::{AreaCursor, LinkFlags};
use qa_world::entities::EntityReference;
use std::sync::Arc;

pub(super) fn reference(
    host: &FrameHost,
    entity: EntityId,
) -> Result<NativeEntityRef, ClientCommandError> {
    capture(host, entity)?
        .reference()
        .ok_or(ClientCommandError::InvalidSlot)
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
    let native = qa_core::primitives::NativeEntity {
        module,
        slot: i32::try_from(reference.ordinal).map_err(|_| ClientCommandError::InvalidSlot)?,
    };
    Ok(host.runtime.server.entities.find_native(native, None))
}

pub(super) fn release(
    host: &mut FrameHost,
    admitted: AdmittedEntity,
) -> Result<(), ClientCommandError> {
    let ResourceNamespace::Module(module) = admitted.reference().namespace else {
        return Err(ClientCommandError::DifferentModule);
    };
    let row = host
        .module_row_mut(module)
        .ok_or(ClientCommandError::MissingModule)?;
    let Program::QuakeC(qc) = &mut row.request.program else {
        return Err(ClientCommandError::NeedsNativeModule);
    };
    qc.vm
        .release_entity(admitted)
        .map_err(ClientCommandError::Usercmd)
}

pub(super) fn capture(
    host: &FrameHost,
    entity: EntityId,
) -> Result<EntityReference, ClientCommandError> {
    host.runtime
        .server
        .entities
        .capture_reference(entity)
        .ok_or(ClientCommandError::InvalidSlot)
}

pub(super) fn admit_captured(
    host: &mut FrameHost,
    recipient: ModuleId,
    access: TraceAccess<'_>,
    captured: EntityReference,
) -> Result<AdmittedEntity, ClientCommandError> {
    super::edit_quakec(host, recipient, access, |vm, services, context| {
        vm.admit_captured_reference(&services.server.entities, context, captured)
    })
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
        access: TraceAccess<'_>,
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
            let other = capture(host, entity)?;
            let target_capture = capture(host, target)?;
            if target != entity
                && let Some(touch) = prepare_touch(
                    host,
                    module,
                    access,
                    target_capture,
                    other,
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
    access: TraceAccess<'_>,
    entity: EntityId,
    time: ThinkTime,
    driver: &mut crate::modules::Driver<'_>,
) -> Result<(), ClientCommandError> {
    let mover = reference(host, entity)?;
    if mover.namespace != ResourceNamespace::Module(module) {
        return Err(ClientCommandError::DifferentModule);
    }
    let mut run = TriggerRun::begin(host, mover, time)?;
    while let Some(touch) = run.next(host, access)? {
        touch.invoke(host, driver)?;
    }
    Ok(())
}

/// The checked callback and its one optional post-acceptance stamp. The weak
/// other survives removal; callback-self is revalidated by FunctionTable.
pub(super) struct PreparedTouch {
    request: CallbackRequest,
    stamp: Option<(ModuleId, usize)>,
    admitted: AdmittedEntity,
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
        release(host, self.admitted)?;
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

    pub(super) fn cancel(self, host: &mut FrameHost) -> Result<(), ClientCommandError> {
        release(host, self.admitted)
    }

    fn invoke(
        self,
        host: &mut FrameHost,
        driver: &mut crate::modules::Driver<'_>,
    ) -> Result<(), ClientCommandError> {
        let request = self.request;
        let result = (|| {
            let functions = host
                .modules
                .as_ref()
                .map(|modules| Arc::clone(&modules.functions))
                .ok_or(ClientCommandError::MissingModule)?;
            let outcome = functions
                .dispatch_resolved(host, request.target, request.call)
                .map_err(ClientCommandError::Dispatch)?;
            Ok(driver.complete(host, outcome, request.call))
        })();
        let completed = self.complete(host, result.as_ref().is_ok_and(|accepted| *accepted));
        result.map(|_| ()).and(completed)
    }
}

fn prepare_touch(
    host: &mut FrameHost,
    module: ModuleId,
    access: TraceAccess<'_>,
    target: EntityReference,
    other: EntityReference,
    time: Option<ThinkTime>,
    deduplicate: bool,
    trigger_only: bool,
) -> Result<Option<PreparedTouch>, ClientCommandError> {
    prepare_admitted_touch(
        host,
        module,
        access,
        target,
        other,
        None,
        time,
        deduplicate,
        trigger_only,
    )
}

fn resolve_touch(
    host: &mut FrameHost,
    module: ModuleId,
    entity: EntityId,
    time: Option<ThinkTime>,
    stamp: Option<usize>,
    trigger_only: bool,
) -> Result<
    Option<(
        ModuleId,
        qa_session::dispatch::ResolvedCall,
        Option<ThinkTime>,
    )>,
    ClientCommandError,
> {
    let entities = &host.runtime.server.entities;
    let Some(slot) = entities.resolve(entity) else {
        return Ok(None);
    };
    let owner = entities.columns.owner[slot];
    let native = entities.columns.native_entity[slot];
    let published = entities.columns.touch[slot];
    if native.is_none() && published.is_none() {
        return Ok(None);
    }
    if let Some(native) = native
        && native.module != owner
    {
        return Err(ClientCommandError::DifferentModule);
    }
    if native.is_none() {
        if let Some(callback) = published {
            host.modules
                .as_ref()
                .ok_or(ClientCommandError::MissingModule)?
                .functions
                .resolve(entities, callback, entity)
                .map_err(ClientCommandError::Dispatch)?;
            host.module_row_mut(module)
                .ok_or(ClientCommandError::MissingModule)?
                .counts
                .unsupported_contacts += 1;
        }
        return Ok(None);
    }
    let native = native.ok_or(ClientCommandError::InvalidSlot)?;
    let row = host
        .module_row(owner)
        .ok_or(ClientCommandError::MissingModule)?;
    if let Some(index) = stamp {
        let batch = host
            .module_row(module)
            .ok_or(ClientCommandError::MissingModule)?;
        if index >= batch.client_contacts.len() {
            return Err(ClientCommandError::InvalidSlot);
        }
        if batch.client_contacts.contains(index) {
            return Ok(None);
        }
    }
    let Program::QuakeC(qc) = &row.request.program else {
        host.module_row_mut(module)
            .ok_or(ClientCommandError::MissingModule)?
            .counts
            .unsupported_contacts += 1;
        return Ok(None);
    };
    let offset = usize::try_from(native.slot)
        .map_err(|_| ClientCommandError::InvalidSlot)?
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
    let time = if owner == module {
        time
    } else {
        Some(row.time(host.time))
    };
    Ok(Some((owner, target, time)))
}

fn prepare_admitted_touch(
    host: &mut FrameHost,
    module: ModuleId,
    access: TraceAccess<'_>,
    target_capture: EntityReference,
    other: EntityReference,
    mut admitted: Option<AdmittedEntity>,
    time: Option<ThinkTime>,
    deduplicate: bool,
    trigger_only: bool,
) -> Result<Option<PreparedTouch>, ClientCommandError> {
    let Some(entity) = host
        .runtime
        .server
        .entities
        .resolve_reference(target_capture)
    else {
        if let Some(admitted) = admitted.take() {
            release(host, admitted)?;
        }
        return Ok(None);
    };
    let stamp = deduplicate.then_some(target_capture.entity().slot as usize);
    let prepared = resolve_touch(host, module, entity, time, stamp, trigger_only);
    let prepared = match prepared {
        Ok(Some(prepared)) => prepared,
        result => {
            if let Some(admitted) = admitted.take() {
                release(host, admitted)?;
            }
            return result.map(|_| None);
        }
    };
    let (owner, target, time) = prepared;
    let admitted = match admitted {
        Some(admitted) => admitted,
        None => admit_captured(host, owner, access, other)?,
    };
    Ok(Some(PreparedTouch {
        request: CallbackRequest {
            target,
            call: CallbackCall::Touch {
                entity,
                other: admitted.reference(),
                time,
            },
        },
        stamp: stamp.map(|index| (module, index)),
        admitted,
    }))
}

pub(super) fn touch(
    host: &mut FrameHost,
    module: ModuleId,
    access: TraceAccess<'_>,
    target: EntityReference,
    other: EntityReference,
    time: Option<ThinkTime>,
    deduplicate: bool,
    trigger_only: bool,
    driver: &mut crate::modules::Driver<'_>,
) -> Result<(), ClientCommandError> {
    if let Some(touch) = prepare_touch(
        host,
        module,
        access,
        target,
        other,
        time,
        deduplicate,
        trigger_only,
    )? {
        touch.invoke(host, driver)?;
    }
    Ok(())
}

pub(super) fn impact(
    host: &mut FrameHost,
    module: ModuleId,
    access: TraceAccess<'_>,
    mover_entity: Option<EntityId>,
    mover: EntityReference,
    obstacle: EntityReference,
    time: ThinkTime,
    driver: &mut crate::modules::Driver<'_>,
) -> Result<(), ClientCommandError> {
    let quakec = host
        .module_row(module)
        .is_some_and(|row| matches!(&row.request.program, Program::QuakeC(_)));
    let restore = if quakec {
        Some(super::edit_quakec(host, module, access, |vm, _, _| {
            vm.prepare_impact(time)
        })?)
    } else {
        None
    };
    let mut first = None;
    let mut second = None;
    let execution = (|| {
        let original_mover = host.runtime.server.entities.resolve_reference(mover);
        let first_runs = if let Some(entity) = mover_entity {
            resolve_touch(host, module, entity, None, None, false)?.is_some()
        } else {
            false
        };
        let obstacle_entity = host.runtime.server.entities.resolve_reference(obstacle);
        if first_runs {
            first = Some(admit_captured(host, module, access, obstacle)?);
        }
        if let Some(entity) = obstacle_entity {
            let owner = host.runtime.server.entities.columns.owner[entity.slot as usize];
            if let Some(row) = host.module_row(owner)
                && let Program::QuakeC(qc) = &row.request.program
                && let Some(native) = obstacle.reference()
                && qc
                    .vm
                    .weak_entity_offset(owner, native)
                    .and_then(|offset| qc.vm.touch_pending(offset))
                    .map_err(ClientCommandError::Usercmd)?
            {
                second = Some(admit_captured(host, owner, access, mover)?);
            }
        }
        // Both values are pinned before callback one can retire either source.
        // Re-resolve callback-self and its touch word after each immediate call.
        if first_runs
            && mover_entity.is_some()
            && let Some(touch) = prepare_admitted_touch(
                host,
                module,
                access,
                mover,
                obstacle,
                first.take(),
                None,
                false,
                false,
            )?
        {
            touch.invoke(host, driver)?;
        }
        if let Some(entity) = host.runtime.server.entities.resolve_reference(obstacle) {
            let replacement = host.runtime.server.entities.resolve_reference(mover);
            if second.is_some() && replacement.is_some() && replacement != original_mover {
                if let Some(admitted) = second.take() {
                    release(host, admitted)?;
                }
                let owner = host.runtime.server.entities.columns.owner[entity.slot as usize];
                second = Some(admit_captured(host, owner, access, mover)?);
            }
            if let Some(touch) = prepare_admitted_touch(
                host,
                module,
                access,
                obstacle,
                mover,
                second.take(),
                None,
                false,
                false,
            )? {
                touch.invoke(host, driver)?;
            }
        }
        Ok(())
    })();
    let released_first = first
        .take()
        .map(|admitted| release(host, admitted))
        .transpose();
    let released_second = second
        .take()
        .map(|admitted| release(host, admitted))
        .transpose();
    let restored = if let Some(restore) = restore {
        super::edit_quakec(host, module, access, |vm, _, _| {
            vm.finish_client_call(restore)
        })
    } else {
        Ok(())
    };
    execution
        .and(released_first.map(|_| ()))
        .and(released_second.map(|_| ()))
        .and(restored)
}

#[cfg(test)]
#[path = "../../tests/modules/foreign_contacts.rs"]
pub(super) mod tests;
