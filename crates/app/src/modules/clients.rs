//! Client lifecycle exports use the same session table as provider callbacks.
use super::{Export, FrameHost, ModuleResult, Program, Q2Clients, State};
use qa_core::primitives::{ClientId, NativeEntity, ThinkTime, UserCmd};

impl FrameHost {
    /// Native client numbers are supplied by the connection boundary. They are
    /// unrelated to the common entity slot or the player's movement rules.
    pub fn connect_q2_client(
        &mut self,
        api: &Q2Clients,
        id: ClientId,
        userinfo: &[u8],
    ) -> Result<(), String> {
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            use qa_compat::memory::ModuleMemory;
            let native_client = self
                .runtime
                .local_snapshots
                .get(id.0 as usize)
                .and_then(Option::as_ref)
                .ok_or("native client needs connection metadata")?
                .native_client();
            let row = self.module_row(api.module).ok_or("missing client module")?;
            if row.state != State::Running || native_client >= 256 {
                return Err("client module unavailable or native slot out of range".into());
            }
            let time = row.request.context.clock.at(self.time);
            let context = row.request.context.console;
            let maxclients = row.storage.max_clients(&self.console.cvars, context.source);
            if native_client >= maxclients.max(0) as u32 {
                return Err("native client outside module maxclients".into());
            }
            let native = NativeEntity {
                module: api.module,
                slot: (native_client + 1) as i32,
            };
            let table = self
                .native_entities(api.module)
                .map_err(|e| format!("client table: {e:?}"))?;
            let address = qa_compat::entities::slot_address(table, native.slot as u32)
                .ok_or("native client outside published entity table")?;
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
                let base = vm.process.base();
                let mut memory = ModuleMemory::borrow(
                    base,
                    vm.process
                        .memory_mut()
                        .map_err(|e| format!("client memory: {e:?}"))?,
                )
                .map_err(|e| format!("client memory: {e:?}"))?;
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
                    .client_disconnect = Some(api.disconnect);
                accepted = true;
                self.call_module_export(api.module, Export::words(api.begin, &[address]), time)
                    .map_err(|e| format!("ClientBegin: {e:?}"))
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
    ) -> Result<(), String> {
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            use qa_compat::memory::ModuleMemory;
            let (native, address, time) = self.module_client(id)?;
            if native.module != api.module {
                return Err("command targets another native module".into());
            }
            let row = self
                .module_row_mut(native.module)
                .ok_or("missing client module")?;
            let Program::Native { vm, .. } = &mut row.request.program else {
                return Err("client needs a native module".into());
            };
            let base = vm.process.base();
            let mut memory = ModuleMemory::borrow(
                base,
                vm.process
                    .memory_mut()
                    .map_err(|e| format!("command memory: {e:?}"))?,
            )
            .map_err(|e| format!("command memory: {e:?}"))?;
            api.abi
                .write_command(&mut memory, api.usercmd, command, server_frame)
                .map_err(|e| format!("native usercmd: {e:?}"))?;
            self.call_module_export(
                api.module,
                Export::words(api.think, &[address, api.usercmd]),
                time,
            )
            .map_err(|e| format!("ClientThink: {e:?}"))
        }
        #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
        {
            let _ = (api, id, command, server_frame);
            Err("native child backend unsupported on this host".into())
        }
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn module_client(&mut self, id: ClientId) -> Result<(NativeEntity, u64, ThinkTime), String> {
        let client = self
            .runtime
            .server
            .clients
            .get(id.0 as usize)
            .filter(|client| client.connection.is_some())
            .ok_or("client is disconnected")?;
        let slot = self
            .runtime
            .server
            .entities
            .resolve(client.entity)
            .ok_or("stale client lifetime")?;
        let native = self.runtime.server.entities.columns.native_entity[slot]
            .filter(|native| native.module == client.module)
            .ok_or("client has no native binding")?;
        let row = self
            .module_row(native.module)
            .ok_or("missing client module")?;
        if row.state != State::Running || row.client_disconnect.is_none() {
            return Err("native client is not active".into());
        }
        let time = row.request.context.clock.at(self.time);
        let table = self
            .native_entities(native.module)
            .map_err(|e| format!("client table: {e:?}"))?;
        let slot = u32::try_from(native.slot).map_err(|_| "invalid native client slot")?;
        let address = qa_compat::entities::slot_address(table, slot)
            .ok_or("native client outside published entity table")?;
        Ok((native, address, time))
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
            .and_then(|row| row.client_disconnect)
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
