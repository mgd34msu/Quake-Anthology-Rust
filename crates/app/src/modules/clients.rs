//! Client lifecycle exports use the same session table as provider callbacks.
use super::{Export, FrameHost, ModuleResult, Program, Q2Clients, State};
use qa_core::primitives::{ClientId, NativeEntity};

impl FrameHost {
    /// Native client numbers are supplied by the connection boundary. They are
    /// unrelated to the common entity slot or the player's movement rules.
    pub fn connect_q2_client(
        &mut self,
        api: &Q2Clients,
        id: ClientId,
        native_client: u32,
        userinfo: &[u8],
    ) -> Result<(), String> {
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            use qa_compat::memory::ModuleMemory;
            let row = self.module_row(api.module).ok_or("missing client module")?;
            if row.state != State::Running || native_client >= 256 {
                return Err("client module unavailable or native slot out of range".into());
            }
            let time = row.request.context.clock.at(self.time);
            let context = row.request.context.console;
            let maxclients = self
                .console
                .cvars
                .bind("sv_maxclients", context)
                .and_then(|view| self.console.cvars.integer_view(view).ok())
                .ok_or("missing native maxclients")?;
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
            if native.slot as u32 >= table.count {
                return Err("native client outside published entity table".into());
            }
            let address = table.address + table.stride * native.slot as u64;
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
                    .write_string(api.userinfo, api.userinfo_bytes, userinfo)
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
            let _ = (api, id, native_client, userinfo);
            Err("native child backend unsupported on this host".into())
        }
    }
    pub(super) fn disconnect_module_clients(&mut self) {
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        for index in 0..self.runtime.server.clients.len() {
            self.disconnect_module_client(ClientId(index as u32));
        }
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn disconnect_module_client(&mut self, id: ClientId) {
        let client = &self.runtime.server.clients[id.0 as usize];
        if client.connection.is_none() {
            return;
        }
        let module = client.module;
        let Some(native) =
            self.runtime.server.entities.columns.native_entity[client.entity.slot as usize]
        else {
            return;
        };
        let Some(row) = self.module_row(module) else {
            return;
        };
        let Some(callback) = row.client_disconnect else {
            return;
        };
        if row.state == State::Running {
            let time = row.request.context.clock.at(self.time);
            if let Ok(table) = self.native_entities(module) {
                let address = table.address + table.stride * native.slot as u64;
                let _ = self.call_module_export(module, Export::words(callback, &[address]), time);
            }
        }
        self.runtime.server.disconnect(id);
    }
}
