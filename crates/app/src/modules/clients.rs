//! Client lifecycle exports use the same session table as provider callbacks.
use super::{FrameHost, ModuleResult, Program, Q2Clients, State};
use qa_core::primitives::{
    CallbackCall, CallbackId, ClientCallback, ClientId, CommandArgs, ModuleId, NativeEntity,
    RuleSetId, SnapshotObservation, ThinkTime, UserCmd, Vec3,
};
use std::fmt;

mod q3;
pub(super) mod quakec;

/// Cold ABI metadata, independent of a client's wire, movement and trace roles.
#[derive(Clone, Copy, Debug)]
pub struct ClientLifecycleInfo {
    pub module: ModuleId,
    pub rules: RuleSetId,
    pub userinfo_bytes: usize,
    pub clients: usize,
    pub separate_values: bool,
}
#[derive(Clone, Copy)]
pub(super) struct ClientLifecycle {
    pub(super) info: ClientLifecycleInfo,
    pub(super) disconnect_callback: Option<CallbackId>,
    frame_commands: bool,
    q2: Option<Q2Clients>,
    connect: fn(&mut FrameHost, ModuleId, ClientId, &[u8]) -> Result<u32, String>,
    command:
        fn(&mut FrameHost, ModuleId, ClientId, &UserCmd, u32) -> Result<(), ClientCommandError>,
    finish: fn(&mut FrameHost, ModuleId, ClientId) -> Result<(), ClientCommandError>,
    prepare_disconnect: fn(
        &mut FrameHost,
        ModuleId,
        ClientId,
    ) -> Result<(CallbackId, CallbackCall<'static>), ClientCommandError>,
    text: fn(&mut FrameHost, ModuleId, ClientId, CommandArgs<'_>) -> Result<(), ClientCommandError>,
    userinfo: fn(&mut FrameHost, ModuleId, ClientId, &[u8]) -> Result<(), ClientCommandError>,
    publish: fn(&mut FrameHost, ModuleId, ClientId) -> Result<(), ClientCommandError>,
}
impl ClientLifecycle {
    pub(super) fn q3(module: ModuleId, clients: usize) -> Self {
        Self {
            info: ClientLifecycleInfo {
                module,
                rules: RuleSetId::Quake3,
                userinfo_bytes: qa_compat::services::userinfo_rules(RuleSetId::Quake3).bytes,
                clients,
                separate_values: false,
            },
            disconnect_callback: Some(CallbackId(5)),
            frame_commands: false,
            q2: None,
            connect: q3::connect,
            command: q3::command,
            finish: no_finish,
            prepare_disconnect: q3::prepare_disconnect,
            text: q3::text,
            userinfo: q3::userinfo,
            publish: no_finish,
        }
    }
    pub(super) fn quakec(module: ModuleId, rules: RuleSetId, clients: usize) -> Self {
        let frame_client = rules == RuleSetId::Quake;
        Self {
            info: ClientLifecycleInfo {
                module,
                rules,
                userinfo_bytes: qa_compat::quakec::QW_USERINFO_BYTES,
                clients,
                separate_values: true,
            },
            disconnect_callback: None,
            frame_commands: frame_client,
            q2: None,
            connect: quakec::connect,
            command: if frame_client {
                quakec::receive
            } else {
                quakec::command
            },
            finish: if frame_client {
                no_finish
            } else {
                quakec::finish
            },
            prepare_disconnect: quakec::prepare_disconnect,
            text: quakec::text,
            userinfo: quakec::userinfo,
            publish: quakec::publish,
        }
    }
    fn q2(api: Q2Clients, clients: usize) -> Self {
        Self {
            info: ClientLifecycleInfo {
                module: api.module,
                rules: api.rules,
                userinfo_bytes: api.abi.userinfo_bytes,
                clients,
                separate_values: false,
            },
            disconnect_callback: Some(api.disconnect),
            frame_commands: false,
            q2: Some(api),
            connect: q2_connect,
            command: q2_command,
            finish: no_finish,
            prepare_disconnect: q2_disconnect,
            text: q2_text,
            userinfo: q2_userinfo,
            publish: no_finish,
        }
    }
}
fn no_finish(_: &mut FrameHost, _: ModuleId, _: ClientId) -> Result<(), ClientCommandError> {
    Ok(())
}
fn q2_api(host: &FrameHost, module: ModuleId) -> Result<Q2Clients, ClientCommandError> {
    host.module_row(module)
        .and_then(|row| row.client_lifecycle)
        .and_then(|table| table.q2)
        .ok_or(ClientCommandError::Inactive)
}
fn q2_connect(
    host: &mut FrameHost,
    module: ModuleId,
    id: ClientId,
    text: &[u8],
) -> Result<u32, String> {
    let api = q2_api(host, module).map_err(|e| e.to_string())?;
    host.connect_q2_data(&api, id, text)
}
fn q2_command(
    host: &mut FrameHost,
    module: ModuleId,
    id: ClientId,
    command: &UserCmd,
    frame: u32,
) -> Result<(), ClientCommandError> {
    host.think_q2_client(&q2_api(host, module)?, id, command, frame)
}
fn q2_disconnect(
    host: &mut FrameHost,
    module: ModuleId,
    id: ClientId,
) -> Result<(CallbackId, CallbackCall<'static>), ClientCommandError> {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        let api = q2_api(host, module)?;
        let (_, address, time) = host.module_client(id)?;
        host.client_call(
            id,
            ClientCallback::Disconnect,
            time,
            &[address],
            CommandArgs::default(),
        )
        .map(|call| (api.disconnect, call))
    }
    #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
    {
        let _ = (host, module, id);
        Err(ClientCommandError::Unsupported)
    }
}
fn q2_text(
    host: &mut FrameHost,
    module: ModuleId,
    id: ClientId,
    command: CommandArgs<'_>,
) -> Result<(), ClientCommandError> {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        let api = q2_api(host, module)?;
        let (_, address, time) = host.module_client(id)?;
        host.invoke_client(
            id,
            api.command,
            ClientCallback::Command,
            time,
            &[address],
            command,
        )
    }
    #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
    {
        let _ = (host, module, id, command);
        Err(ClientCommandError::Unsupported)
    }
}
fn q2_userinfo(
    host: &mut FrameHost,
    _: ModuleId,
    id: ClientId,
    text: &[u8],
) -> Result<(), ClientCommandError> {
    host.userinfo_q2_client(id, text)
}

#[derive(Debug)]
pub enum ClientCommandError {
    NoMovementBinding,
    MissingModule,
    Inactive,
    InvalidSlot,
    DifferentModule,
    NeedsNativeModule,
    SlotOutsideTable,
    Unsupported,
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    Memory(qa_compat::memory::ProcessMemoryError),
    Usercmd(qa_compat::services::CallError),
    Table(qa_session::dispatch::CallError),
    Dispatch(qa_session::dispatch::CallError),
}
impl fmt::Display for ClientCommandError {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoMovementBinding => {
                output.write_str("client has no active native movement binding")
            }
            Self::MissingModule => output.write_str("missing client module"),
            Self::Inactive => output.write_str("native client is not active"),
            Self::InvalidSlot => output.write_str("invalid native client slot"),
            Self::DifferentModule => output.write_str("command targets another native module"),
            Self::NeedsNativeModule => output.write_str("client needs a native module"),
            Self::SlotOutsideTable => {
                output.write_str("native client outside published entity table")
            }
            Self::Unsupported => output.write_str("native child backend unsupported on this host"),
            #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
            Self::Memory(error) => write!(output, "native memory: {error:?}"),
            Self::Usercmd(error) => write!(output, "native usercmd: {error:?}"),
            Self::Table(error) => write!(output, "client table: {error:?}"),
            Self::Dispatch(error) => write!(output, "ClientThink: {error:?}"),
        }
    }
}
impl std::error::Error for ClientCommandError {}

impl FrameHost {
    /// Install a native table once after load. This caches the frozen native limit.
    pub fn register_q2_client_api(&mut self, api: Q2Clients) -> Result<(), String> {
        let modules = self.modules.as_mut().ok_or("missing client module")?;
        let limit = modules
            .storage
            .max_clients(&self.console.cvars, api.rules)
            .clamp(0, 256) as usize;
        modules
            .storage
            .bind_client_capacity(api.module, limit)
            .map_err(|e| format!("module client capacity: {e:?}"))?;
        let row = modules.row_mut(api.module).ok_or("missing client module")?;
        if let Some(existing) = row.client_lifecycle {
            let unchanged = existing.q2.is_some_and(|previous| {
                previous.module == api.module
                    && previous.rules == api.rules
                    && previous.usercmd == api.usercmd
                    && previous.userinfo == api.userinfo
                    && previous.connect == api.connect
                    && previous.begin == api.begin
                    && previous.disconnect == api.disconnect
                    && previous.think == api.think
                    && previous.command == api.command
                    && previous.userinfo_changed == api.userinfo_changed
                    && previous.social == api.social
                    && std::ptr::eq(previous.abi, api.abi)
            });
            return if unchanged {
                Ok(())
            } else {
                Err("client lifecycle already registered differently".into())
            };
        }
        row.client_lifecycle = Some(ClientLifecycle::q2(api, limit));
        Ok(())
    }
    pub fn client_info(&self, module: ModuleId) -> Option<ClientLifecycleInfo> {
        Some(self.module_row(module)?.client_lifecycle?.info)
    }
    pub fn connect_module_client(
        &mut self,
        module: ModuleId,
        id: ClientId,
        userinfo: &[u8],
    ) -> Result<u32, String> {
        let table = self
            .module_row(module)
            .and_then(|row| row.client_lifecycle)
            .ok_or("module has no supported client lifecycle")?;
        self.runtime
            .server
            .replace_userinfo(
                id,
                userinfo,
                qa_compat::services::userinfo_rules(table.info.rules),
            )
            .map_err(|e| format!("client userinfo: {e:?}"))?;
        (table.connect)(self, module, id, userinfo)
    }
    pub fn run_module_client_command(
        &mut self,
        module: ModuleId,
        id: ClientId,
        command: &UserCmd,
        frame: u32,
    ) -> Result<(), ClientCommandError> {
        let native = self
            .runtime
            .server
            .module_moves(id)
            .ok_or(ClientCommandError::NoMovementBinding)?;
        if native.module != module {
            return Err(ClientCommandError::DifferentModule);
        }
        let table = self
            .module_row(module)
            .and_then(|row| row.client_lifecycle)
            .ok_or(ClientCommandError::Inactive)?;
        let implicit = self
            .module_row(module)
            .is_some_and(|row| row.client_batch.is_none());
        if implicit {
            self.begin_command_batch(id);
        }
        let result = (table.command)(self, module, id, command, frame);
        if implicit {
            if result.is_ok() {
                self.end_command_batch(id);
            } else if let Some(row) = self.module_row_mut(module) {
                row.client_batch = None;
            }
        }
        result
    }
    pub(crate) fn begin_command_batch(&mut self, id: ClientId) {
        let Some(native) = self.runtime.server.module_moves(id) else {
            return;
        };
        let Some(row) = self.module_row_mut(native.module) else {
            return;
        };
        if row.client_batch == Some(id) {
            return;
        }
        if row.client_batch.is_some() {
            row.counts.rejected += 1;
            return;
        }
        row.client_contacts.begin();
        row.client_batch = Some(id);
    }
    pub(crate) fn end_command_batch(&mut self, id: ClientId) {
        let Some(native) = self.runtime.server.module_moves(id) else {
            return;
        };
        let table = self
            .module_row(native.module)
            .filter(|row| row.client_batch == Some(id))
            .and_then(|row| row.client_lifecycle);
        if let Some(table) = table {
            if (table.finish)(self, native.module, id).is_err() {
                self.disconnect_client(id);
            }
            if let Some(row) = self.module_row_mut(native.module) {
                row.client_batch = None;
            }
        }
    }
    pub(crate) fn publish_module_client_state(&mut self) {
        for slot in 0..self.runtime.server.clients.len() {
            let id = ClientId(slot as u32);
            let Some(native) = self.runtime.server.module_moves(id) else {
                continue;
            };
            let Some(table) = self
                .module_row(native.module)
                .filter(|row| row.state == State::Running)
                .and_then(|row| row.client_lifecycle)
            else {
                continue;
            };
            if (table.publish)(self, native.module, id).is_err() {
                if let Some(row) = self.module_row_mut(native.module) {
                    row.counts.rejected += 1;
                }
            }
        }
    }
    fn invoke_client(
        &mut self,
        id: ClientId,
        callback: CallbackId,
        event: ClientCallback,
        time: ThinkTime,
        words: &[u64],
        command: CommandArgs<'_>,
    ) -> Result<(), ClientCommandError> {
        let call = self.client_call(id, event, time, words, command)?;
        self.call_module(callback, call)
            .map_err(ClientCommandError::Dispatch)
    }
    fn client_call<'a>(
        &self,
        id: ClientId,
        event: ClientCallback,
        time: ThinkTime,
        words: &[u64],
        command: CommandArgs<'a>,
    ) -> Result<CallbackCall<'a>, ClientCommandError> {
        let client = self
            .runtime
            .server
            .clients
            .get(id.0 as usize)
            .filter(|client| {
                client.connection.is_some()
                    && (client.is_active() || event == ClientCallback::Disconnect)
            })
            .ok_or(ClientCommandError::Inactive)?;
        let mut arguments = [0; 9];
        for (to, from) in arguments.iter_mut().zip(words) {
            *to = *from;
        }
        Ok(CallbackCall::Client {
            entity: client.entity,
            client: id,
            event,
            time,
            arguments,
            command,
        })
    }
    /// Called once for every command admitted by the native channel. Modules
    /// never share the engine's deferred movement pass with their ClientThink.
    pub(crate) fn received_command(
        &mut self,
        id: ClientId,
        command: UserCmd,
        execution: qa_network::commands::packet::CommandExecution,
        observation: Option<SnapshotObservation>,
    ) {
        let rules = self.runtime.server.clients[id.0 as usize]
            .player
            .movement_rules;
        let command = if execution == qa_network::commands::packet::CommandExecution::PerCommand
            && self.runtime.server.module_moves(id).is_none()
        {
            qa_movement::prepare_command(rules, command)
        } else {
            command
        };
        self.runtime
            .server
            .submit_command(id, command, execution, observation);
        if execution == qa_network::commands::packet::CommandExecution::PerCommand {
            self.consume_pending_command(id);
            self.runtime.server.commit_attachments();
        }
    }

    /// Frame-driven native clients admit their queued fields before StartFrame,
    /// including catch-up provider ticks that precede the current world tick.
    pub(super) fn prepare_module_client_commands(&mut self, module: ModuleId, seconds: f64) {
        if !self
            .module_row(module)
            .and_then(|row| row.client_lifecycle)
            .is_some_and(|table| table.frame_commands)
        {
            return;
        }
        for slot in 0..self.runtime.server.clients.len() {
            let id = ClientId(slot as u32);
            if self
                .runtime
                .server
                .module_moves(id)
                .is_some_and(|native| native.module == module)
            {
                self.consume_pending_command(id);
            }
        }
        quakec::controls(self, module, seconds);
    }

    /// The host calls this ordered batch for frame-latest commands and bots.
    /// Packet-command protocols already consumed their commands during ingress.
    pub(crate) fn run_pending_commands(&mut self, span: qa_core::sys_events::EventTime) {
        for slot in 0..self.runtime.server.clients.len() {
            let id = ClientId(slot as u32);
            self.runtime
                .server
                .resolve_frame_command(id, span, self.time);
            self.consume_pending_command(id);
        }
        self.runtime.server.commit_attachments();
    }

    fn consume_pending_command(&mut self, id: ClientId) {
        if let Some(native) = self.runtime.server.module_moves(id) {
            let Some((command, observation)) = self.runtime.server.take_pending_command(id) else {
                return;
            };
            let bound = self.module_row(native.module).map(|row| {
                observation
                    .filter(|_| {
                        self.runtime
                            .server
                            .entities
                            .resolve(row.request.anchor)
                            .is_some()
                    })
                    .and_then(|source| source.frame_for(native.module, row.request.anchor))
                    .unwrap_or(0)
            });
            if let Some(frame) = bound {
                let result = self.run_module_client_command(native.module, id, &command, frame);
                if let Err(error) = result {
                    let _ = self
                        .runtime
                        .server
                        .events
                        .console_formatted(format_args!("ClientThink: {error}\n"));
                    self.disconnect_client(id);
                }
            }
        } else if let Some(world) = &mut self.runtime.collision {
            let player = &mut self.runtime.server.clients[id.0 as usize].player;
            let base = player.movement.tuning;
            player.movement.tuning =
                self.movement_handles
                    .resolve(&self.console.cvars, player.movement_rules, base);
            let result = self.runtime.server.move_pending_client(
                id,
                &self.runtime.geometry,
                world.geometry,
                world.index,
                &mut world.scratch,
            );
            self.runtime.server.clients[id.0 as usize]
                .player
                .movement
                .tuning = base;
            if let Err(error) = result {
                let _ = self
                    .runtime
                    .server
                    .events
                    .console_formatted(format_args!("Movement client {}: {error:?}\n", id.0));
            }
        }
    }
    /// Capture the owning native provider's completed frame at publication,
    /// independently of the negotiated snapshot sequence/time scale.
    pub(crate) fn snapshot_observation(
        &self,
        id: ClientId,
        time: qa_core::sys_events::EventTime,
    ) -> Option<SnapshotObservation> {
        let native = self.runtime.server.module_moves(id)?;
        let row = self.module_row(native.module)?;
        row.client_lifecycle?;
        if row.state != State::Running
            || self
                .runtime
                .server
                .entities
                .resolve(row.request.anchor)
                .is_none()
        {
            return None;
        }
        Some(SnapshotObservation {
            module: native.module,
            module_lifetime: row.request.anchor,
            native_frame: row.request.context.server_frame as u32,
            time,
        })
    }
    /// Native client numbers are supplied by the connection boundary. They are
    /// unrelated to the common entity slot or the player's movement rules.
    fn connect_q2_data(
        &mut self,
        api: &Q2Clients,
        id: ClientId,
        userinfo: &[u8],
    ) -> Result<u32, String> {
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            let snapshot = self
                .runtime
                .snapshots
                .get(id.0 as usize)
                .and_then(Option::as_ref)
                .ok_or("native client needs connection metadata")?;
            let native_client = snapshot.native_client();
            let seat = snapshot.seat();
            let row = self.module_row(api.module).ok_or("missing client module")?;
            if row.state != State::Running || native_client >= 256 {
                return Err("client module unavailable or native slot out of range".into());
            }
            let maxclients = self
                .client_info(api.module)
                .ok_or("missing client lifecycle")?
                .clients;
            if native_client as usize >= maxclients {
                return Err("native client outside module maxclients".into());
            }
            self.runtime.native_numbers[id.0 as usize].set_native_clients(maxclients as u32, 1);
            let native = NativeEntity {
                module: api.module,
                slot: (native_client + 1) as i32,
            };
            let (address, time) = self
                .native_slot(api.module, native.slot as u32)
                .map_err(|error| error.to_string())?;
            if !self.runtime.server.bind_client_native(id, native) {
                return Err("native client namespace already occupied".into());
            }
            let result = (|| {
                let entity = self.runtime.server.clients[id.0 as usize].entity;
                let row = self
                    .modules
                    .as_mut()
                    .and_then(|m| m.row_mut(api.module))
                    .ok_or("missing client module")?;
                let Program::Native { vm, .. } = &mut row.request.program else {
                    return Err("client needs a native module".to_string());
                };
                vm.bind_client(&self.runtime.server.entities, native, entity)
                    .map_err(|e| format!("native client binding: {e:?}"))?;
                let mut memory = self
                    .native_memory(api.module)
                    .map_err(|error| error.to_string())?;
                memory
                    .write_string(api.userinfo, api.abi.userinfo_bytes, userinfo)
                    .map_err(|e| format!("native userinfo: {e:?}"))?;
                self.invoke_client(
                    id,
                    api.connect,
                    ClientCallback::Connect,
                    time,
                    &[address, api.userinfo, api.social, 0],
                    CommandArgs::default(),
                )
                .map_err(|e| format!("ClientConnect: {e:?}"))?;
                if !self
                    .module_counts(api.module)
                    .and_then(|counts| counts.last_result)
                    .is_some_and(
                        |result| matches!(result, ModuleResult::Native(value) if value != 0),
                    )
                {
                    return Err("native ClientConnect rejected this client".into());
                }
                self.refresh_q2_userinfo(id, api)
                    .map_err(|e| format!("native userinfo publication: {e:?}"))?;
                // The successful ClientBegin return publishes native pmove and
                // edict bounds through the existing owned-client projection.
                // Failure below disconnects and clears this admission.
                self.runtime.server.clients[id.0 as usize].module_movement = true;
                self.invoke_client(
                    id,
                    api.begin,
                    ClientCallback::Begin,
                    time,
                    &[address],
                    CommandArgs::default(),
                )
                .map_err(|e| format!("ClientBegin: {e:?}"))?;
                // Q2 ClientBegin owns spawn facing through ps.pmove.delta_angles.
                // Like CL_ClearState, begin with zero command angles rather
                // than adding the walk-through host's map-facing seed again.
                if let Some(seat) = seat {
                    self.runtime.input.set_view_angles(seat, Vec3::default());
                }
                Ok(native_client)
            })();
            if result.is_err() {
                self.disconnect_client(id);
            }
            result
        }
        #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
        {
            let _ = (api, id, userinfo);
            Err("native child backend unsupported on this host".into())
        }
    }
    /// Encode a received common command into the load-reserved game.h buffer.
    /// `server_frame` is the owning native frame retained with the client's
    /// acknowledged submitted snapshot, never its raw protocol frame number.
    /// Dispatch does not also run the host's authoritative movement entry.
    fn think_q2_client(
        &mut self,
        api: &Q2Clients,
        id: ClientId,
        command: &UserCmd,
        server_frame: u32,
    ) -> Result<(), ClientCommandError> {
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            let (native, address, time) = self.module_client(id)?;
            if native.module != api.module {
                return Err(ClientCommandError::DifferentModule);
            }
            let mut memory = self.native_memory(native.module)?;
            api.abi
                .write_command(&mut memory, api.usercmd, command, server_frame)
                .map_err(ClientCommandError::Usercmd)?;
            self.invoke_client(
                id,
                api.think,
                ClientCallback::BeforeMove,
                time,
                &[address, api.usercmd],
                CommandArgs::default(),
            )
        }
        #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
        {
            let _ = (api, id, command, server_frame);
            Err(ClientCommandError::Unsupported)
        }
    }

    /// Packet text is client input, never an unrestricted local-console command.
    /// The module's load-selected tokenizer exposes native argv through the
    /// same session function table and owned-child import path as ClientThink.
    pub fn client_string_command(
        &mut self,
        id: ClientId,
        rules: qa_core::primitives::RuleSetId,
        bytes: &[u8],
    ) -> Result<std::ops::ControlFlow<()>, ClientCommandError> {
        let api = self
            .runtime
            .server
            .module_moves(id)
            .and_then(|native| self.module_row(native.module))
            .and_then(|row| {
                row.client_lifecycle
                    .map(|table| (table, row.request.context.console))
            });
        let context = api.map_or(
            qa_console::views::Context {
                source: rules,
                side: qa_console::catalog::Scope::Server,
                ..Default::default()
            },
            |(_, mut context)| {
                context.source = rules;
                context
            },
        );
        let mut text = qa_core::text::FixedText::<{ qa_console::text::MAX_TEXT }>::default();
        text.append_bytes(bytes)
            .map_err(|_| ClientCommandError::Usercmd(qa_compat::services::CallError::Text))?;
        // Classic Q2 expands cvars in server client commands. Rerelease passes
        // macroExpand=false; syntax belongs to the explicit protocol boundary.
        let mut tokens = std::mem::take(&mut self.command_tokens);
        let result = (|| {
            if rules == qa_core::primitives::RuleSetId::Quake2 {
                qa_console::command_text::expand_bytes(&mut text, &mut tokens, |name| {
                    self.console.cvars.macro_value(name, context)
                })
                .map_err(|_| ClientCommandError::Usercmd(qa_compat::services::CallError::Text))?;
            }
            let args =
                qa_console::command_text::tokenize_bytes(text.as_bytes(), rules, &mut tokens)
                    .map_err(|_| {
                        ClientCommandError::Usercmd(qa_compat::services::CallError::Text)
                    })?;
            if args.is_empty() {
                return Ok(std::ops::ControlFlow::Continue(()));
            }
            if args.get_bytes(0) == b"disconnect" {
                self.disconnect_client(id);
                return Ok(std::ops::ControlFlow::Break(()));
            }
            let Some((api, _)) = api else {
                return Ok(std::ops::ControlFlow::Continue(()));
            };
            let mut argv = [&[][..]; 1024];
            for (to, from) in argv.iter_mut().zip(args.iter_bytes()) {
                *to = from;
            }
            (api.text)(
                self,
                api.info.module,
                id,
                CommandArgs {
                    argv: &argv[..args.len()],
                    raw: args.raw_bytes(),
                },
            )?;
            Ok(std::ops::ControlFlow::Continue(()))
        })();
        self.command_tokens = tokens;
        result
    }

    pub fn userinfo_module_client(
        &mut self,
        id: ClientId,
        text: &[u8],
    ) -> Result<(), ClientCommandError> {
        let native = self
            .runtime
            .server
            .module_moves(id)
            .ok_or(ClientCommandError::NoMovementBinding)?;
        let table = self
            .module_row(native.module)
            .and_then(|row| row.client_lifecycle)
            .ok_or(ClientCommandError::Inactive)?;
        self.runtime
            .server
            .replace_userinfo(
                id,
                text,
                qa_compat::services::userinfo_rules(table.info.rules),
            )
            .map_err(|_| ClientCommandError::Usercmd(qa_compat::services::CallError::Text))?;
        (table.userinfo)(self, native.module, id, text)
    }
    fn userinfo_q2_client(&mut self, id: ClientId, text: &[u8]) -> Result<(), ClientCommandError> {
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            let (native, address, time) = self.module_client(id)?;
            let api = self
                .module_row(native.module)
                .and_then(|row| row.client_lifecycle)
                .and_then(|table| table.q2)
                .ok_or(ClientCommandError::Inactive)?;
            self.native_memory(native.module)?
                .write_string(api.userinfo, api.abi.userinfo_bytes, text)
                .map_err(|error| ClientCommandError::Usercmd(error.into()))?;
            self.invoke_client(
                id,
                api.userinfo_changed,
                ClientCallback::UserinfoChanged,
                time,
                &[address, api.userinfo],
                CommandArgs::default(),
            )?;
            self.refresh_q2_userinfo(id, &api)
        }
        #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
        {
            let _ = (id, text);
            Err(ClientCommandError::Unsupported)
        }
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn refresh_q2_userinfo(
        &mut self,
        id: ClientId,
        api: &Q2Clients,
    ) -> Result<(), ClientCommandError> {
        let row = self
            .modules
            .as_mut()
            .and_then(|modules| modules.row_mut(api.module))
            .ok_or(ClientCommandError::MissingModule)?;
        let Program::Native { vm, .. } = &mut row.request.program else {
            return Err(ClientCommandError::NeedsNativeModule);
        };
        let memory = qa_compat::memory::process_memory(&mut vm.process)
            .map_err(ClientCommandError::Memory)?;
        let text = memory
            .read(api.userinfo, api.abi.userinfo_bytes)
            .map_err(|e| ClientCommandError::Usercmd(e.into()))?;
        let text = qa_core::text::terminated(text).ok_or(ClientCommandError::Usercmd(
            qa_compat::services::CallError::Memory,
        ))?;
        self.runtime
            .server
            .replace_userinfo_raw(id, text, api.abi.userinfo_bytes)
            .map_err(|_| ClientCommandError::Usercmd(qa_compat::services::CallError::Text))
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn module_client(
        &mut self,
        id: ClientId,
    ) -> Result<(NativeEntity, u64, ThinkTime), ClientCommandError> {
        let native = self
            .runtime
            .server
            .module_moves(id)
            .ok_or(ClientCommandError::NoMovementBinding)?;
        let row = self
            .module_row(native.module)
            .ok_or(ClientCommandError::MissingModule)?;
        if row.state != State::Running || row.client_lifecycle.is_none() {
            return Err(ClientCommandError::Inactive);
        }
        let slot = u32::try_from(native.slot).map_err(|_| ClientCommandError::InvalidSlot)?;
        let (address, time) = self.native_slot(native.module, slot)?;
        Ok((native, address, time))
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn native_memory(
        &mut self,
        module: ModuleId,
    ) -> Result<qa_compat::memory::ModuleMemory<'_>, ClientCommandError> {
        let row = self
            .module_row_mut(module)
            .ok_or(ClientCommandError::MissingModule)?;
        let Program::Native { vm, .. } = &mut row.request.program else {
            return Err(ClientCommandError::NeedsNativeModule);
        };
        qa_compat::memory::process_memory(&mut vm.process).map_err(ClientCommandError::Memory)
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn native_slot(
        &mut self,
        module: ModuleId,
        slot: u32,
    ) -> Result<(u64, ThinkTime), ClientCommandError> {
        let row = self
            .module_row(module)
            .ok_or(ClientCommandError::MissingModule)?;
        let time = row.time(self.time);
        let table = self
            .native_entities(module)
            .map_err(ClientCommandError::Table)?;
        let address = qa_compat::entities::slot_address(table, slot)
            .ok_or(ClientCommandError::SlotOutsideTable)?;
        Ok((address, time))
    }
    pub(super) fn disconnect_module_clients(&mut self) {
        for index in 0..self.runtime.server.clients.len() {
            if self.runtime.server.clients[index].connection.is_some() {
                self.disconnect_client(ClientId(index as u32));
            }
        }
    }
    /// Original signon protocols cannot selectively remove one module's loops
    /// or static models. Drop recipients which have observed that module.
    pub(super) fn module_recipient(&self, module: ModuleId, id: ClientId) -> bool {
        let Some(client) = self.runtime.server.clients.get(id.0 as usize) else {
            return false;
        };
        client.connection.is_some()
            && (client.module == module
                || self.modules.as_ref().is_some_and(|modules| {
                    modules.row(module).is_some_and(|row| {
                        modules.signon[row.signon.range()]
                            .get(id.0 as usize)
                            .is_some_and(|cursor| {
                                cursor.has_publication_for(module, id, client.entity)
                            })
                    })
                }))
    }
    pub(super) fn disconnect_module_recipients(&mut self, module: ModuleId) {
        for slot in 0..self.runtime.server.clients.len() {
            let id = ClientId(slot as u32);
            if self.module_recipient(module, id) {
                self.disconnect_client(id);
            }
        }
    }
    pub(super) fn client_disconnect_call(
        &mut self,
        id: ClientId,
    ) -> Option<(CallbackId, CallbackCall<'static>)> {
        let native = self.runtime.server.module_moves(id)?;
        let prepare = self
            .module_row(native.module)
            .filter(|row| row.state == State::Running)?
            .client_lifecycle?
            .prepare_disconnect;
        prepare(self, native.module, id).ok()
    }
    /// Claim one synchronous disconnect without disturbing a healthy in-flight
    /// owner. Failed owners cannot resume their earlier Closing continuation.
    pub(super) fn begin_client_disconnect(
        &mut self,
        id: ClientId,
        lifetime: qa_core::primitives::EntityId,
    ) -> Option<(CallbackId, CallbackCall<'static>)> {
        let prepared = self.client_disconnect_call(id);
        if !self
            .runtime
            .server
            .begin_close(id, lifetime, self.runtime.input_time)
        {
            let failed = self
                .runtime
                .server
                .clients
                .get(id.0 as usize)
                .filter(|client| client.connection.is_some() && client.entity == lifetime)
                .is_some_and(|client| self.module_state(client.module) == Some(State::Failed));
            if failed {
                self.finish_client_disconnect(id, lifetime);
            }
            return None;
        }
        if prepared.is_none() {
            self.finish_client_disconnect(id, lifetime);
        }
        prepared
    }
    pub(super) fn finish_client_disconnect(
        &mut self,
        id: ClientId,
        lifetime: qa_core::primitives::EntityId,
    ) {
        if self
            .runtime
            .server
            .clients
            .get(id.0 as usize)
            .is_some_and(|client| client.entity == lifetime)
        {
            self.retire_client(id);
        }
    }
    pub(crate) fn disconnect_client(&mut self, id: ClientId) {
        let Some(lifetime) = self
            .runtime
            .server
            .clients
            .get(id.0 as usize)
            .map(|client| client.entity)
        else {
            return;
        };
        if let Some((callback, call)) = self.begin_client_disconnect(id, lifetime) {
            let _ = self.call_module(callback, call);
            self.finish_client_disconnect(id, lifetime);
        }
    }
    pub(crate) fn finish_client_termination(
        &mut self,
        id: ClientId,
        termination: qa_compat::services::ClientTermination,
    ) {
        match termination {
            qa_compat::services::ClientTermination::ServerDisconnect => {
                self.received_client_disconnect(id)
            }
            qa_compat::services::ClientTermination::Rejected => self.disconnect_client(id),
        }
    }
    pub(crate) fn retire_client(&mut self, id: ClientId) {
        let module = self
            .runtime
            .server
            .clients
            .get(id.0 as usize)
            .map(|client| client.module);
        if quakec::retain_body(self, id).is_err()
            && let Some(row) = module.and_then(|module| self.module_row_mut(module))
        {
            row.counts.rejected += 1;
        }
        self.runtime.disconnect_client(id);
        if let Some(modules) = self.modules.as_mut() {
            for row in modules.rows.iter_mut().flatten() {
                super::Module::retain_active_batch(&mut row.client_batch, &self.runtime.server);
            }
        }
        for local in &mut self.local_clients {
            if *local == Some(id) {
                *local = None;
            }
        }
    }
}
