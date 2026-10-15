//! QuakeC native client phases over the common lifetime and movement services.
use super::*;
use qa_compat::services::CallError as ServiceError;
use qa_core::primitives::{Motion, MovementMode};
use qa_session::dispatch::{ThinkFrame, run_think_with_rebind};
use std::sync::Arc;

use super::super::edit_quakec as edit;

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
fn prepare_callback(
    host: &mut FrameHost,
    module: ModuleId,
    id: ClientId,
    event: ClientCallback,
) -> Result<(CallbackId, CallbackCall<'static>), ClientCommandError> {
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
    host.client_call(id, event, time, &[], CommandArgs::default())
        .map(|call| (callback, call))
}
fn callback(
    host: &mut FrameHost,
    module: ModuleId,
    id: ClientId,
    event: ClientCallback,
) -> Result<(), ClientCommandError> {
    let (callback, call) = prepare_callback(host, module, id, event)?;
    host.call_module(callback, call)
        .map_err(ClientCommandError::Dispatch)
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
                .and_then(|client| {
                    let entities = &host.runtime.server.entities;
                    let slot = entities.resolve(client.entity)?;
                    let native = entities.columns.native_entity[slot]
                        .filter(|native| native.module == module)?;
                    u32::try_from(native.slot).ok()?.checked_sub(1)
                })
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
        host.runtime.native_numbers[id.0 as usize].set_native_clients(info.clients as u32, 1);
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

/// Resolve movement-role tuning for the duration of one native phase.
fn with_tuning<R>(
    host: &mut FrameHost,
    id: ClientId,
    call: impl FnOnce(&mut FrameHost) -> Result<R, ClientCommandError>,
) -> Result<R, ClientCommandError> {
    let player = &mut host.runtime.server.clients[id.0 as usize].player;
    let base = player.movement.tuning;
    player.movement.tuning =
        host.movement_handles
            .resolve(&host.console.cvars, player.movement_rules, base);
    let result = call(host);
    host.runtime.server.clients[id.0 as usize]
        .player
        .movement
        .tuning = base;
    result
}

fn move_frame_client(
    host: &mut FrameHost,
    module: ModuleId,
    id: ClientId,
    offset: u32,
    options: qa_movement::MoveOptions,
) -> Result<qa_movement::MovementResult, ClientCommandError> {
    let captured = host.runtime.server.clients[id.0 as usize].entity;
    let command = host.runtime.server.clients[id.0 as usize].command;
    with_tuning(host, id, |host| {
        edit(host, module, |vm, services, context| {
            vm.read_frame_client(
                &services.server.entities,
                context,
                offset,
                captured,
                &mut services.server.clients[id.0 as usize].player,
            )?;
            let (geometry, index) = services.world.ok_or(ServiceError::Geometry)?;
            services.server.move_client(
                id,
                command,
                services.geometry,
                geometry,
                index,
                services.scratch,
                options,
                |entities, player, write| {
                    vm.write_frame_scalar(entities, context, offset, captured, player, write)
                },
            )
        })
    })
}

/// Native control uses the retained command, in edict order, before StartFrame.
pub(super) fn controls(host: &mut FrameHost, module: ModuleId, seconds: f64) {
    let Some(info) = host.client_info(module) else {
        return;
    };
    for ordinal in 1..=info.clients {
        let Ok(slot) = i32::try_from(ordinal) else {
            break;
        };
        let binding = NativeEntity { module, slot };
        let Some(id) = host.runtime.server.client_for_native(binding) else {
            continue;
        };
        if host.runtime.server.module_moves(id) != Some(binding) {
            continue;
        }
        let result = (|| {
            let offset = native(host, module, id)?;
            let options = edit(host, module, |vm, services, context| {
                vm.prepare_control(services, context, offset, seconds)
            })?;
            if let Some(options) = options {
                move_frame_client(host, module, id, offset, options)?;
            }
            Ok::<(), ClientCommandError>(())
        })();
        if let Err(error) = result {
            let unsupported = matches!(
                error,
                ClientCommandError::Usercmd(ServiceError::Unsupported)
            );
            if let Some(row) = host.module_row_mut(module) {
                if unsupported {
                    row.counts.unsupported_physics += 1;
                } else {
                    row.counts.rejected += 1;
                }
            }
            if !unsupported {
                host.disconnect_client(id);
            }
        }
    }
}

/// Reserved NQ clients execute at their native ordinal in the provider pass.
/// NONE and exact NOCLIP are admitted; other motion remains counted.
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
    match phase {
        Motion::None => {}
        Motion::Noclip => {
            let admitted = edit(host, module, |vm, _, context| {
                vm.client_noclip(context, offset)
            });
            if matches!(
                admitted,
                Err(ClientCommandError::Usercmd(ServiceError::Unsupported))
            ) {
                return Ok(false);
            }
            admitted?;
        }
        _ => return Ok(false),
    }
    think(host, module, id, time, step)?;
    let offset = native(host, module, id)?;
    if phase == Motion::Noclip {
        move_frame_client(
            host,
            module,
            id,
            offset,
            qa_movement::MoveOptions {
                phase: qa_movement::MovePhase::Integrate { seconds: step },
                ..Default::default()
            },
        )?;
    }
    edit(host, module, |vm, services, context| {
        vm.relink_entity(services, context, offset)
    })?;
    let entity = host.runtime.server.clients[id.0 as usize].entity;
    super::super::contacts::triggers(host, module, entity, time)?;
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
        let player = &host.runtime.server.clients[id.0 as usize].player;
        let spectator = player.movement.mode == MovementMode::Spectator;
        // Q3's shared executor consumes an absolute command clock. A QW leaf
        // retains native duration while progressing that clock explicitly.
        if player.movement_rules == RuleSetId::Quake3 {
            command.server_time_ms = player
                .movement
                .command_time_ms
                .wrapping_add(i32::from(command.duration_ms));
        }
        let moved = with_tuning(host, id, |host| {
            let world = host
                .runtime
                .collision
                .as_mut()
                .ok_or(ClientCommandError::Usercmd(ServiceError::Geometry))?;
            host.runtime
                .server
                .move_client(
                    id,
                    command,
                    &host.runtime.geometry,
                    world.geometry,
                    world.index,
                    &mut world.scratch,
                    Default::default(),
                    qa_session::physics::no_body_boundary,
                )
                .map_err(|_| ClientCommandError::Usercmd(ServiceError::Geometry))
        })?;
        edit(host, module, |vm, services, context| {
            vm.write_client(services, context, offset, origin_offset)
        })?;
        if !spectator {
            let entity = host.runtime.server.clients[id.0 as usize].entity;
            super::super::contacts::triggers(host, module, entity, time)?;
            let other_ref = super::super::contacts::reference(host, entity)?;
            for contact in moved.contacts[..moved.contact_count].iter().flatten() {
                if let Some(other) = contact.entity {
                    super::super::contacts::touch(
                        host, module, other, other_ref, None, true, false,
                    )?;
                }
            }
        }
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
pub(super) fn prepare_disconnect(
    host: &mut FrameHost,
    module: ModuleId,
    id: ClientId,
) -> Result<(CallbackId, CallbackCall<'static>), ClientCommandError> {
    prepare_callback(host, module, id, ClientCallback::Disconnect)
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
