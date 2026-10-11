//! Client lifecycle exports use the same session table as provider callbacks.
use super::{Export, FrameHost, ModuleResult, Program, Q2Clients, State};
use qa_core::primitives::{ClientId, ModuleId, NativeEntity, ThinkTime, UserCmd, Vec3};
use std::fmt;

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
    /// Called once for every command admitted by the native channel. Modules
    /// never share the engine's deferred movement pass with their ClientThink.
    pub(crate) fn received_command(
        &mut self,
        id: ClientId,
        command: UserCmd,
        execution: qa_network::commands::packet::CommandExecution,
    ) {
        let rules = self.runtime.server.clients[id.0 as usize]
            .player
            .movement_rules;
        let command = if execution == qa_network::commands::packet::CommandExecution::PerCommand {
            qa_movement::prepare_command(rules, command)
        } else {
            command
        };
        self.runtime.server.submit_command(id, command, execution);
        if execution == qa_network::commands::packet::CommandExecution::PerCommand {
            self.consume_pending_command(id);
            self.runtime.server.commit_attachments();
        }
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
            let Some(command) = self.runtime.server.take_pending_command(id) else {
                return;
            };
            if let Some(api) = self
                .module_row(native.module)
                .and_then(|row| row.client_api)
                && let Err(error) = self.think_q2_client(&api, id, &command, 0)
            {
                let _ = self
                    .runtime
                    .server
                    .events
                    .console_formatted(format_args!("ClientThink: {error}\n"));
            }
        } else if let Some(world) = &mut self.runtime.collision {
            self.runtime.server.move_pending_client(
                id,
                &self.runtime.geometry,
                world.geometry,
                world.index,
                &mut world.scratch,
            );
        }
    }
    /// Native client numbers are supplied by the connection boundary. They are
    /// unrelated to the common entity slot or the player's movement rules.
    pub fn connect_q2_client(
        &mut self,
        api: &Q2Clients,
        id: ClientId,
        userinfo: &[u8],
    ) -> Result<u32, String> {
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            let snapshot = self
                .runtime
                .local_snapshots
                .get(id.0 as usize)
                .and_then(Option::as_ref)
                .ok_or("native client needs connection metadata")?;
            let native_client = snapshot.native_client();
            let seat = snapshot.seat;
            let row = self.module_row(api.module).ok_or("missing client module")?;
            if row.state != State::Running || native_client >= 256 {
                return Err("client module unavailable or native slot out of range".into());
            }
            let context = row.request.context.console;
            let maxclients = row.storage.max_clients(&self.console.cvars, context.source);
            if native_client >= maxclients.max(0) as u32 {
                return Err("native client outside module maxclients".into());
            }
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
            let mut accepted = false;
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
                self.call_module_export(
                    api.module,
                    Export::words(api.connect, &[address, api.userinfo, api.social, 0]),
                    time,
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
                self.module_row_mut(api.module)
                    .ok_or("missing client module")?
                    .client_api = Some(*api);
                self.runtime.server.clients[id.0 as usize].module_movement = true;
                accepted = true;
                self.call_module_export(api.module, Export::words(api.begin, &[address]), time)
                    .map_err(|e| format!("ClientBegin: {e:?}"))?;
                // Q2 ClientBegin owns spawn facing through ps.pmove.delta_angles.
                // Like CL_ClearState, begin with zero command angles rather
                // than adding the walk-through host's map-facing seed again.
                self.runtime.input.set_view_angles(seat, Vec3::default());
                Ok(native_client)
            })();
            if result.is_err() {
                if accepted {
                    self.disconnect_module_client(id);
                } else {
                    self.runtime.server.disconnect(id);
                }
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
    /// `server_frame` is the client's reported snapshot, not the provider clock.
    /// Dispatch does not also run the host's authoritative movement entry.
    pub fn think_q2_client(
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
            let mut export = Export::words(api.think, &[address, api.usercmd]);
            export.client = Some(id);
            self.call_module_export(api.module, export, time)
                .map_err(ClientCommandError::Dispatch)
        }
        #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
        {
            let _ = (api, id, command, server_frame);
            Err(ClientCommandError::Unsupported)
        }
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
        if row.state != State::Running || row.client_api.is_none() {
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
        let time = row.request.context.clock.at(self.time);
        let table = self
            .native_entities(module)
            .map_err(ClientCommandError::Table)?;
        let address = qa_compat::entities::slot_address(table, slot)
            .ok_or(ClientCommandError::SlotOutsideTable)?;
        Ok((address, time))
    }
    pub(super) fn disconnect_module_clients(&mut self) {
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        for index in 0..self.runtime.server.clients.len() {
            self.disconnect_module_client(ClientId(index as u32));
        }
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn disconnect_module_client(&mut self, id: ClientId) {
        // Only clients admitted through this lifecycle have a disconnect entry.
        // Failed modules still release those clients without re-entering them.
        let Some(callback) = self
            .runtime
            .server
            .clients
            .get(id.0 as usize)
            .and_then(|client| self.module_row(client.module))
            .and_then(|row| row.client_api.map(|api| api.disconnect))
        else {
            return;
        };
        if let Ok((native, address, time)) = self.module_client(id) {
            let _ =
                self.call_module_export(native.module, Export::words(callback, &[address]), time);
        }
        self.runtime.server.disconnect(id);
    }
}
