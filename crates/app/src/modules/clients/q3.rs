//! Q3 client exports share the session table and canonical native namespace.
use super::*;

fn binding(
    host: &FrameHost,
    module: ModuleId,
    id: ClientId,
) -> Result<(u32, ThinkTime), ClientCommandError> {
    let row = host
        .module_row(module)
        .filter(|row| row.state == State::Running)
        .ok_or(ClientCommandError::Inactive)?;
    let info = row
        .client_lifecycle
        .ok_or(ClientCommandError::Inactive)?
        .info;
    let client = host
        .runtime
        .server
        .clients
        .get(id.0 as usize)
        .filter(|client| client.connection.is_some() && client.module == module)
        .ok_or(ClientCommandError::Inactive)?;
    let slot = host
        .runtime
        .server
        .entities
        .resolve(client.entity)
        .ok_or(ClientCommandError::Inactive)?;
    let native = host.runtime.server.entities.columns.native_entity[slot]
        .filter(|native| native.module == module)
        .ok_or(ClientCommandError::InvalidSlot)?;
    let number = u32::try_from(native.slot).map_err(|_| ClientCommandError::InvalidSlot)?;
    if number as usize >= info.clients {
        return Err(ClientCommandError::InvalidSlot);
    }
    Ok((number, row.time(host.time)))
}

fn prepare_callback<'a>(
    host: &mut FrameHost,
    module: ModuleId,
    id: ClientId,
    event: ClientCallback,
    argv: CommandArgs<'a>,
) -> Result<(CallbackId, CallbackCall<'a>), ClientCommandError> {
    let (number, time) = binding(host, module, id)?;
    // game/g_public.h GAME_CLIENT_* ordinals, shared by QVM and native vmMain.
    let export = match event {
        ClientCallback::Begin => 3,
        ClientCallback::UserinfoChanged => 4,
        ClientCallback::Disconnect => {
            host.module_row(module)
                .and_then(|row| row.client_lifecycle)
                .and_then(|table| table.disconnect_callback)
                .ok_or(ClientCommandError::Unsupported)?
                .0
        }
        ClientCallback::Command => 6,
        ClientCallback::BeforeMove => 7,
        _ => return Err(ClientCommandError::Unsupported),
    };
    host.client_call(id, event, time, &[u64::from(number)], argv)
        .map(|call| (CallbackId(export), call))
}
fn callback(
    host: &mut FrameHost,
    module: ModuleId,
    id: ClientId,
    event: ClientCallback,
    argv: CommandArgs<'_>,
    driver: &mut crate::modules::Driver<'_>,
) -> Result<(), ClientCommandError> {
    let (callback, call) = prepare_callback(host, module, id, event, argv)?;
    host.call_module_with_driver(callback, call, driver)
        .map_err(ClientCommandError::Dispatch)
}

fn denial(host: &mut FrameHost, module: ModuleId) -> Result<Option<String>, String> {
    let row = host.module_row_mut(module).ok_or("missing client module")?;
    let value = row
        .counts
        .last_result
        .ok_or("missing ClientConnect return")?;
    match (&mut row.request.program, value) {
        (Program::Qvm { vm, .. }, ModuleResult::Qvm(value)) => {
            if value == 0 {
                return Ok(None);
            }
            // NULL is tested before QVM address masking, per VM_ExplicitArgPtr.
            let text = vm
                .memory
                .cstring(vm.masked_address(value))
                .map_err(|e| format!("ClientConnect denial: {e:?}"))?;
            Ok(Some(
                String::from_utf8_lossy(&text[..text.len().min(1023)]).into_owned(),
            ))
        }
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        (Program::Native { vm, .. }, ModuleResult::Native(value)) => {
            if value == 0 {
                return Ok(None);
            }
            let memory = qa_compat::memory::process_memory(&mut vm.process)
                .map_err(|e| format!("ClientConnect denial memory: {e:?}"))?;
            let text = memory
                .cstring(value)
                .map_err(|e| format!("ClientConnect denial: {e:?}"))?;
            Ok(Some(
                String::from_utf8_lossy(&text[..text.len().min(1023)]).into_owned(),
            ))
        }
        _ => Err("invalid ClientConnect result ABI".into()),
    }
}

pub(super) fn connect(
    host: &mut FrameHost,
    module: ModuleId,
    id: ClientId,
    _: &[u8],
    driver: &mut crate::modules::Driver<'_>,
) -> Result<u32, String> {
    let info = host.client_info(module).ok_or("missing client lifecycle")?;
    let row = host
        .module_row(module)
        .filter(|row| row.state == State::Running)
        .ok_or("client module unavailable")?;
    let client = host
        .runtime
        .server
        .clients
        .get(id.0 as usize)
        .filter(|client| client.connection.is_some())
        .ok_or("inactive client")?;
    let is_bot = client.connection == Some(qa_session::clients::Connection::Bot);
    let number = host
        .runtime
        .snapshots
        .get(id.0 as usize)
        .and_then(Option::as_ref)
        .and_then(|binding| binding.native_client())
        .or_else(|| {
            // Bots must supply their native namespace at the connection boundary.
            is_bot
                .then(|| {
                    host.runtime.server.entities.columns.native_entity[client.entity.slot as usize]
                })
                .flatten()
                .filter(|native| native.module == module)
                .and_then(|native| u32::try_from(native.slot).ok())
        })
        .ok_or("native client needs connection metadata")?;
    if number as usize >= info.clients {
        return Err("native client outside module maxclients".into());
    }
    let time = row.time(host.time);
    let native = NativeEntity {
        module,
        slot: number as i32,
    };
    if !host.runtime.server.bind_client_native(id, native) {
        return Err("native client namespace already occupied".into());
    }
    let entity = host.runtime.server.clients[id.0 as usize].entity;
    let result = (|| {
        host.modules
            .as_mut()
            .ok_or("missing client module")?
            .storage
            .reset_native_close(native)
            .map_err(|e| format!("native client close reset: {e:?}"))?;
        let row = host
            .modules
            .as_mut()
            .and_then(|modules| modules.row_mut(module))
            .ok_or("missing client module")?;
        let player_stats = match &mut row.request.program {
            Program::Qvm { vm, .. } => {
                vm.bind_client(&host.runtime.server.entities, native, entity)
                    .map_err(|e| format!("QVM client binding: {e:?}"))?;
                vm.player_stat_layout()
            }
            #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
            Program::Native { vm, .. } => {
                vm.bind_client(&host.runtime.server.entities, native, entity)
                    .map_err(|e| format!("native client binding: {e:?}"))?;
                vm.player_stat_layout()
            }
            _ => return Err("client needs a Q3 module".into()),
        };
        if let Some(binding) = host.runtime.snapshots[id.0 as usize].as_mut()
            && !binding.set_player_stat_layout(player_stats, &host.runtime.catalog.hud.values)
        {
            return Err("native stat layout must be selected before the first snapshot".into());
        }
        host.invoke_client(
            id,
            CallbackId(2),
            ClientCallback::Connect,
            time,
            &[u64::from(number), 1, u64::from(is_bot)],
            CommandArgs::default(),
            driver,
        )
        .map_err(|e| format!("ClientConnect: {e:?}"))?;
        if let Some(text) = denial(host, module)? {
            return Err(format!("ClientConnect rejected: {text}"));
        }
        // Stock Q3 owns Pmove inside its module. The host must never add a
        // second move, including synchronous/bot Think deferred to RunFrame.
        host.runtime.server.clients[id.0 as usize].module_movement = true;
        callback(
            host,
            module,
            id,
            ClientCallback::Begin,
            CommandArgs::default(),
            driver,
        )
        .map_err(|e| format!("ClientBegin: {e:?}"))?;
        host.runtime.native_numbers[id.0 as usize].set_native_clients(info.clients as u32, 0);
        if let Some(seat) = host.runtime.snapshots[id.0 as usize]
            .as_ref()
            .and_then(|b| b.seat())
        {
            host.runtime.input.set_view_angles(seat, Vec3::default());
        }
        Ok(number)
    })();
    if result.is_err() {
        host.disconnect_client_with_driver(id, driver);
    }
    result
}

pub(super) fn command(
    host: &mut FrameHost,
    module: ModuleId,
    id: ClientId,
    _: &UserCmd,
    _: u32,
    driver: &mut crate::modules::Driver<'_>,
) -> Result<(), ClientCommandError> {
    callback(
        host,
        module,
        id,
        ClientCallback::BeforeMove,
        CommandArgs::default(),
        driver,
    )
}
pub(super) fn prepare_disconnect(
    host: &mut FrameHost,
    module: ModuleId,
    id: ClientId,
) -> Result<(CallbackId, CallbackCall<'static>), ClientCommandError> {
    prepare_callback(
        host,
        module,
        id,
        ClientCallback::Disconnect,
        CommandArgs::default(),
    )
}
pub(super) fn text(
    host: &mut FrameHost,
    module: ModuleId,
    id: ClientId,
    argv: CommandArgs<'_>,
    driver: &mut crate::modules::Driver<'_>,
) -> Result<(), ClientCommandError> {
    callback(host, module, id, ClientCallback::Command, argv, driver)
}
pub(super) fn userinfo(
    host: &mut FrameHost,
    module: ModuleId,
    id: ClientId,
    _: &[u8],
    driver: &mut crate::modules::Driver<'_>,
) -> Result<(), ClientCommandError> {
    callback(
        host,
        module,
        id,
        ClientCallback::UserinfoChanged,
        CommandArgs::default(),
        driver,
    )
}

/// The existing parser supplies cp/vdr argv; they never reach GAME_CLIENT_COMMAND.
pub(super) fn pure_command(
    host: &mut FrameHost,
    id: ClientId,
    args: qa_console::command_text::Arguments<'_>,
    driver: &mut crate::modules::Driver<'_>,
) -> Result<bool, ClientCommandError> {
    let decision = match args.get_bytes(0) {
        b"vdr" => {
            host.runtime.config_publications[id.0 as usize].pure.reset();
            return Ok(true);
        }
        b"cp" => args.with_command_args(|command| {
            host.runtime.config_publications[id.0 as usize]
                .pure
                .verify(command)
        }),
        _ => return Ok(false),
    };
    if decision == qa_network::pure::Decision::Rejected {
        reject_pure_client(host, id, driver)?;
    }
    Ok(true)
}

fn drop_services<T>(
    host: &mut FrameHost,
    module: ModuleId,
    call: impl FnOnce(
        &mut qa_compat::services::EngineServices<'_>,
    ) -> Result<T, qa_compat::services::CallError>,
) -> Result<T, ClientCommandError> {
    let (row, storage, _) = host
        .modules
        .as_mut()
        .and_then(|modules| modules.trace_parts_mut(module, crate::modules::TraceAccess::empty()))
        .ok_or(ClientCommandError::MissingModule)?;
    call(
        &mut host
            .runtime
            .engine_services(&mut host.console, storage, &mut row.scratch),
    )
    .map_err(ClientCommandError::Usercmd)
}
fn reject_pure_client(
    host: &mut FrameHost,
    id: ClientId,
    driver: &mut crate::modules::Driver<'_>,
) -> Result<(), ClientCommandError> {
    use qa_compat::services::ENGINE_CALLS;
    let native = host
        .runtime
        .server
        .module_moves(id)
        .ok_or(ClientCommandError::InvalidSlot)?;
    let prepared = host.client_disconnect_call(id);
    let time = host.time;
    let target = drop_services(host, native.module, |services| {
        let target = (ENGINE_CALLS.drop_client_target)(services, native, time)?;
        if let Some(target) = target {
            (ENGINE_CALLS.begin_drop_client)(services, target, qa_network::pure::REJECTION, time)?;
        }
        Ok(target)
    })?;
    let Some(target) = target else {
        return Ok(());
    };
    if let Some((callback, call)) = prepared {
        let _ = host.call_module_with_driver(callback, call, driver);
    }
    drop_services(host, native.module, |services| {
        (ENGINE_CALLS.finish_drop_client)(services, target, qa_network::pure::REJECTION)
    })
}
