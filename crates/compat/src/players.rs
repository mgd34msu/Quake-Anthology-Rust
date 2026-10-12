//! Native numeric stats enter the same player/value projection as snapshots.
use crate::{
    entities::{Entities, EntityProjection, native_address},
    memory::{ModuleMemory, Width},
    services::CallError,
};
use qa_core::primitives::{ClientId, IndexSpan, ModuleId, ValueBinding};
use qa_network::{
    commands::packet::Protocol,
    projection::{PlayerContext, PlayerProjection},
};
use qa_session::clients::Server;

#[derive(Clone, Copy)]
pub(crate) struct StatsLayout {
    pub client: usize,
    pub offset: usize,
    pub protocol: Protocol,
}
pub(crate) struct PlayerStats {
    client: &'static crate::q2_abi::ClientLayout,
    projection: Option<PlayerProjection>,
    words: Box<[u32]>,
    stats: IndexSpan,
}
impl PlayerStats {
    pub fn load(client: &'static crate::q2_abi::ClientLayout) -> Self {
        Self {
            client,
            projection: None,
            words: Box::default(),
            stats: qa_network::states::player_value_layout(client.stats.protocol).1[0],
        }
    }
    pub fn bind(&mut self, bindings: &[ValueBinding]) -> Result<(), CallError> {
        if bindings.len() != self.stats.count as usize {
            return Err(CallError::Capacity);
        }
        let projection = PlayerProjection::load_stat_values(self.client.stats.protocol, bindings);
        self.words = vec![0; projection.words()].into_boxed_slice();
        self.projection = Some(projection);
        Ok(())
    }
    /// The returned native call has stopped its child before this borrow.
    pub fn project(
        &mut self,
        memory: &ModuleMemory<'_>,
        table: Entities,
        entities: &EntityProjection,
        module: ModuleId,
        server: &mut Server,
        mut pmove: Option<&mut crate::pmove::Pmove>,
    ) -> Result<(), CallError> {
        if table.address == 0 {
            return Ok(());
        }
        for index in 0..server.clients.len() {
            let module_moves = server.module_moves(ClientId(index as u32)).is_some();
            let client = &mut server.clients[index];
            if client.connection.is_none() || client.module != module {
                continue;
            }
            let Some(edict) = native_address(table, module, &server.entities, client.entity) else {
                continue;
            };
            let slot = client.entity.slot as usize;
            let pointer = memory.read_u64(edict + self.client.stats.client as u64)?;
            if pointer == 0 {
                continue;
            }
            let address = pointer
                .checked_add(self.client.stats.offset as u64)
                .ok_or(CallError::Memory)?;
            if let Some(projection) = &self.projection {
                let words = &mut self.words[self.stats.range()];
                memory.read(address, words.len() * Width::Short.len())?;
                for (index, word) in words.iter_mut().enumerate() {
                    *word = memory
                        .read_field(address + (index * Width::Short.len()) as u64, Width::Short)?
                        as i16 as i32 as u32;
                }
                projection.apply(
                    &self.words,
                    &mut client.player,
                    &mut PlayerContext::default(),
                    qa_core::primitives::ResourceNamespace::Module(module),
                    |_| None,
                );
            }
            if module_moves && let Some(pmove) = pmove.as_deref_mut() {
                let (body, _) = entities.body(memory, edict)?;
                pmove.import_client(memory, pointer, module, &mut client.player)?;
                client.player.body.position = body.position;
                client.player.body.mins = body.mins;
                client.player.body.maxs = body.maxs;
                server.entities.columns.set_body(slot, client.player.body);
            } else {
                self.client
                    .import_presentation(memory, pointer, module, &mut client.player)?;
            }
        }
        Ok(())
    }
}
