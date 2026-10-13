//! QuakeC native client phases over the common lifetime and movement services.
use super::*;
use qa_compat::services::CallError as ServiceError;
use qa_core::primitives::{CallbackCall, EntityId, MovementMode};
use qa_session::dispatch::{ThinkFrame, run_think_with_rebind};
use std::sync::Arc;

fn edit<R>(
    host: &mut FrameHost,
    module: ModuleId,
    call: impl FnOnce(
        &mut qa_compat::quakec::Vm,
        &mut qa_compat::services::EngineServices<'_>,
        qa_compat::services::CallContext,
    ) -> Result<R, ServiceError>,
) -> Result<R, ClientCommandError> {
    let time = host.time;
    let (row, storage) = host
        .modules
        .as_mut()
        .and_then(|m| m.parts_mut(module))
        .ok_or(ClientCommandError::MissingModule)?;
    let clock = row.time(time);
    let Program::QuakeC(qc) = &mut row.request.program else {
        return Err(ClientCommandError::NeedsNativeModule);
    };
    let mut services = host
        .runtime
        .engine_services(&mut host.console, storage, &mut row.scratch);
    call(
        &mut qc.vm,
        &mut services,
        super::super::context(row.request.context, clock),
    )
    .map_err(ClientCommandError::Usercmd)
}
fn native(host: &FrameHost, module: ModuleId, id: ClientId) -> Result<u32, ClientCommandError> {
    let client = host
        .runtime
        .server
        .clients
        .get(id.0 as usize)
        .filter(|client| client.connection.is_some())
        .ok_or(ClientCommandError::Inactive)?;
    let row = host
        .module_row(module)
        .filter(|row| row.state == State::Running)
        .ok_or(ClientCommandError::Inactive)?;
    let Program::QuakeC(qc) = &row.request.program else {
        return Err(ClientCommandError::NeedsNativeModule);
    };
    super::super::native_self(&host.runtime, client.entity, module, &qc.vm)
        .ok_or(ClientCommandError::InvalidSlot)
}
fn callback(
    host: &mut FrameHost,
    module: ModuleId,
    id: ClientId,
    event: ClientCallback,
) -> Result<(), ClientCommandError> {
    let row = host
        .module_row(module)
        .filter(|row| row.state == State::Running)
        .ok_or(ClientCommandError::Inactive)?;
    let Program::QuakeC(qc) = &row.request.program else {
        return Err(ClientCommandError::NeedsNativeModule);
    };
    let callback = qc
        .vm
        .client_callback(event)
        .map_err(ClientCommandError::Usercmd)?;
    let time = row.time(host.time);
    host.invoke_client(id, callback, event, time, &[], CommandArgs::default())
}

pub(super) fn connect(
    host: &mut FrameHost,
    module: ModuleId,
    id: ClientId,
    userinfo: &[u8],
) -> Result<u32, String> {
    let info = host.client_info(module).ok_or("missing client lifecycle")?;
    if info.rules == RuleSetId::Quake
        && !matches!(
            host.runtime
                .server
                .clients
                .get(id.0 as usize)
                .and_then(|client| client.connection),
            Some(qa_session::clients::Connection::Local | qa_session::clients::Connection::Bot)
        )
    {
        return Err("NetQuake remote signon is not yet admitted".into());
    }
    let row = host
        .module_row(module)
        .filter(|row| row.state == State::Running)
        .ok_or("client module unavailable")?;
    let Program::QuakeC(qc) = &row.request.program else {
        return Err("client needs a QuakeC module".into());
    };
    let native_client = host
        .runtime
        .snapshots
        .get(id.0 as usize)
        .and_then(Option::as_ref)
        .map(|binding| binding.native_client())
        .or_else(|| {
            host.runtime
                .server
                .clients
                .get(id.0 as usize)
                .filter(|c| c.connection == Some(qa_session::clients::Connection::Bot))
                .map(|_| id.0)
        })
        .ok_or("native client needs connection metadata")?;
    if native_client as usize >= info.clients {
        return Err("native client outside module maxclients".into());
    }
    let ordinal = native_client
        .checked_add(1)
        .ok_or("invalid native client slot")?;
    let offset = (ordinal as usize)
        .checked_mul(qc.vm.stride)
        .and_then(|v| u32::try_from(v).ok())
        .ok_or("invalid native client slot")?;
    let native = NativeEntity {
        module,
        slot: ordinal as i32,
    };
    if !host.runtime.server.bind_client_native(id, native) {
        return Err("native client namespace already occupied".into());
    }
    let entity = host.runtime.server.clients[id.0 as usize].entity;
    let result = (|| -> Result<(), ClientCommandError> {
        edit(host, module, |vm, services, _| {
            vm.bind_client(&services.server.entities, native, entity)
        })?;
        callback(host, module, id, ClientCallback::NewParameters)?;
        let parms = edit(host, module, |vm, _, _| vm.read_spawn_parms())?;
        host.runtime.server.clients[id.0 as usize].module_spawn_parms = parms;
        let color = |key| {
            let text = qa_core::text::info_value(userinfo, key, false)
                .map_or(&b""[..], |range| &userinfo[range]);
            (qa_core::text::integer_prefix(text).0 as u8 & 15).min(13)
        };
        let colors = color(b"topcolor") << 4 | color(b"bottomcolor");
        edit(host, module, |vm, services, context| {
            vm.prepare_client_spawn(services, context, offset, userinfo, colors)?;
            vm.write_spawn_parms(parms)
        })?;
        callback(host, module, id, ClientCallback::Connect)?;
        callback(host, module, id, ClientCallback::Begin)?;
        // The native module becomes movement owner only after Begin succeeds.
        host.runtime.server.clients[id.0 as usize].module_movement = true;
        host.runtime.native_numbers[id.0 as usize].set_native_clients(info.clients as u32);
        Ok(())
    })();
    if let Err(error) = result {
        host.disconnect_client(id);
        return Err(error.to_string());
    }
    Ok(native_client)
}

/// Frame-latest NQ commands update the existing native fields once at ingress.
/// The provider owns their client phase; no packet-time engine movement runs.
pub(super) fn receive(
    host: &mut FrameHost,
    module: ModuleId,
    id: ClientId,
    command: &UserCmd,
    _: u32,
) -> Result<(), ClientCommandError> {
    let offset = native(host, module, id)?;
    edit(host, module, |vm, _, context| {
        vm.receive_client_command(context, offset, command)
    })
}

fn think(
    host: &mut FrameHost,
    module: ModuleId,
    id: ClientId,
    time: ThinkTime,
    step: f64,
) -> Result<(), ClientCommandError> {
    let entity = host.runtime.server.clients[id.0 as usize].entity;
    let functions = Arc::clone(
        &host
            .modules
            .as_ref()
            .ok_or(ClientCommandError::MissingModule)?
            .functions,
    );
    let result = run_think_with_rebind(
        host,
        &functions,
        entity,
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
            let current = host.runtime.server.clients.get(id.0 as usize)?;
            let slot = host.runtime.server.entities.resolve(current.entity)?;
            let native = host.runtime.server.entities.columns.native_entity[slot]?;
            qc.vm
                .bound_entity(&host.runtime.server.entities, native)
                .ok()
                .flatten()
        },
    );
    if result.rejected != 0 || result.current_entity != Some(entity) {
        return Err(ClientCommandError::Inactive);
    }
    Ok(())
}

/// Reserved NQ clients execute at their native ordinal in the provider pass.
/// This slice admits NONE; other native motion remains explicitly unsupported.
pub(in crate::modules) fn frame(
    host: &mut FrameHost,
    module: ModuleId,
    id: ClientId,
    time: ThinkTime,
    step: f64,
) -> Result<bool, ClientCommandError> {
    callback(host, module, id, ClientCallback::BeforeMove)?;
    let offset = native(host, module, id)?;
    let phase = edit(host, module, |vm, _, context| {
        vm.client_phase(context, offset)
    })?;
    if phase != qa_core::primitives::Motion::None {
        return Ok(false);
    }
    think(host, module, id, time, step)?;
    let offset = native(host, module, id)?;
    edit(host, module, |vm, services, context| {
        vm.relink_entity(services, context, offset)
    })?;
    triggers(host, module, id, time)?;
    native(host, module, id)?;
    callback(host, module, id, ClientCallback::AfterMove)?;
    Ok(true)
}

pub(super) fn command(
    host: &mut FrameHost,
    module: ModuleId,
    id: ClientId,
    command: &UserCmd,
    _frame: u32,
) -> Result<(), ClientCommandError> {
    for mut command in qa_movement::quakeworld_steps(*command) {
        let offset = native(host, module, id)?;
        let step = edit(host, module, |vm, services, context| {
            vm.prepare_client_command(services, context, offset, &command)
        })?;
        callback(host, module, id, ClientCallback::BeforeMove)?;
        let time = host
            .module_row(module)
            .ok_or(ClientCommandError::MissingModule)?
            .time(host.time);
        think(host, module, id, time, step)?;
        let offset = native(host, module, id)?;
        let origin_offset = edit(host, module, |vm, services, context| {
            vm.read_client(services, context, offset)
        })?;
        if host.runtime.collision.is_none() {
            return Err(ClientCommandError::Usercmd(ServiceError::Geometry));
        }
        let player = &mut host.runtime.server.clients[id.0 as usize].player;
        let spectator = player.movement.mode == MovementMode::Spectator;
        let base = player.movement.tuning;
        player.movement.tuning =
            host.movement_handles
                .resolve(&host.console.cvars, player.movement_rules, base);
        // Q3's shared executor consumes an absolute command clock. A QW leaf
        // retains native duration while progressing that clock explicitly.
        if player.movement_rules == RuleSetId::Quake3 {
            command.server_time_ms = player
                .movement
                .command_time_ms
                .wrapping_add(i32::from(command.duration_ms));
        }
        let moved = if let Some(world) = &mut host.runtime.collision {
            host.runtime.server.move_client(
                id,
                command,
                &host.runtime.geometry,
                world.geometry,
                world.index,
                &mut world.scratch,
            )
        } else {
            return Err(ClientCommandError::Usercmd(ServiceError::Geometry));
        };
        host.runtime.server.clients[id.0 as usize]
            .player
            .movement
            .tuning = base;
        let moved = moved.map_err(|_| ClientCommandError::Usercmd(ServiceError::Geometry))?;
        edit(host, module, |vm, services, context| {
            vm.write_client(services, context, offset, origin_offset)
        })?;
        if !spectator {
            triggers(host, module, id, time)?;
            for contact in moved.contacts[..moved.contact_count].iter().flatten() {
                if let Some(other) = contact.entity {
                    touch(host, module, id, other, None, true)?;
                }
            }
        }
    }
    Ok(())
}

fn triggers(
    host: &mut FrameHost,
    module: ModuleId,
    id: ClientId,
    time: ThinkTime,
) -> Result<(), ClientCommandError> {
    let entity = host.runtime.server.clients[id.0 as usize].entity;
    let bounds = host
        .runtime
        .server
        .area
        .bounds(entity)
        .ok_or(ClientCommandError::Usercmd(ServiceError::Entity))?;
    // Copy only bounded generation handles; no module bytes or event payloads.
    let mut candidates = std::mem::take(
        &mut host
            .module_row_mut(module)
            .ok_or(ClientCommandError::MissingModule)?
            .client_triggers,
    );
    candidates.clear();
    for linked in host.runtime.server.area.query(
        &host.runtime.server.entities,
        bounds,
        qa_world::area::LinkFlags::TRIGGER,
    ) {
        if linked.id != entity && candidates.len() < candidates.capacity() {
            candidates.push(linked.id);
        }
    }
    let result = (|| {
        for &other in &candidates {
            touch(host, module, id, other, Some(time), false)?;
        }
        Ok(())
    })();
    if let Some(row) = host.module_row_mut(module) {
        row.client_triggers = candidates;
    }
    result
}
fn touch(
    host: &mut FrameHost,
    module: ModuleId,
    id: ClientId,
    other: EntityId,
    time: Option<ThinkTime>,
    deduplicate: bool,
) -> Result<(), ClientCommandError> {
    let entity = host.runtime.server.clients[id.0 as usize].entity;
    let Some(slot) = host.runtime.server.entities.resolve(other) else {
        return Ok(());
    };
    let Some(native) = host.runtime.server.entities.columns.native_entity[slot]
        .filter(|native| native.module == module)
    else {
        return Ok(());
    };
    let index = usize::try_from(native.slot).map_err(|_| ClientCommandError::InvalidSlot)?;
    let row = host
        .module_row(module)
        .ok_or(ClientCommandError::MissingModule)?;
    if index >= row.client_contacts.len() {
        return Err(ClientCommandError::InvalidSlot);
    }
    if deduplicate && row.client_contacts.contains(index) {
        return Ok(());
    }
    let Program::QuakeC(qc) = &row.request.program else {
        return Err(ClientCommandError::NeedsNativeModule);
    };
    let offset = index
        .checked_mul(qc.vm.stride)
        .and_then(|v| u32::try_from(v).ok())
        .ok_or(ClientCommandError::InvalidSlot)?;
    let Some(callback) = qc
        .vm
        .touch_callback(offset)
        .map_err(ClientCommandError::Usercmd)?
    else {
        return Ok(());
    };
    host.call_module(
        callback,
        CallbackCall::Touch {
            entity: other,
            other: entity,
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
pub(super) fn finish(
    host: &mut FrameHost,
    module: ModuleId,
    id: ClientId,
) -> Result<(), ClientCommandError> {
    callback(host, module, id, ClientCallback::AfterMove)
}
pub(super) fn disconnect(
    host: &mut FrameHost,
    module: ModuleId,
    id: ClientId,
) -> Result<(), ClientCommandError> {
    callback(host, module, id, ClientCallback::Disconnect)
}
pub(super) fn text(
    _: &mut FrameHost,
    _: ModuleId,
    _: ClientId,
    _: CommandArgs<'_>,
) -> Result<(), ClientCommandError> {
    Err(ClientCommandError::Unsupported)
}
pub(super) fn userinfo(
    _: &mut FrameHost,
    _: ModuleId,
    _: ClientId,
    _: &[u8],
) -> Result<(), ClientCommandError> {
    Err(ClientCommandError::Unsupported)
}

pub(super) fn publish(
    host: &mut FrameHost,
    module: ModuleId,
    id: ClientId,
) -> Result<(), ClientCommandError> {
    let rules = host
        .client_info(module)
        .ok_or(ClientCommandError::MissingModule)?
        .rules;
    host.publish_client_values(id, rules, module);
    let offset = native(host, module, id)?;
    let angles = edit(host, module, |vm, _, _| vm.client_set_angles(offset))?;
    if let Some(angles) = angles {
        let delivery = qa_core::primitives::NativeDelivery {
            module,
            source: rules,
            target: qa_core::primitives::NativeTarget::Client(id),
            origin: host.runtime.server.clients[id.0 as usize]
                .player
                .body
                .position,
            reliable: false,
            dupe_key: 0,
            component: 0,
            emission: None,
            check_portals: false,
        };
        if host
            .runtime
            .server
            .events
            .push_delivered(delivery, qa_core::events::FrameEvent::SetAngles(angles))
            .is_ok()
        {
            edit(host, module, |vm, _, _| vm.clear_client_set_angles(offset))?;
        }
    }
    Ok(())
}
