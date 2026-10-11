//! Native numeric stats enter the same player/value projection as snapshots.
use crate::{
    entities::{Entities, native_address},
    memory::ModuleMemory,
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
    layout: StatsLayout,
    projection: Option<PlayerProjection>,
    words: Box<[u32]>,
    stats: IndexSpan,
}
impl PlayerStats {
    pub fn load(layout: StatsLayout) -> Self {
        Self {
            layout,
            projection: None,
            words: Box::default(),
            stats: qa_network::states::player_value_layout(layout.protocol).1[0],
        }
    }
    pub fn bind(&mut self, bindings: &[ValueBinding]) -> Result<(), CallError> {
        if bindings.len() != self.stats.count as usize {
            return Err(CallError::Capacity);
        }
        let projection = PlayerProjection::load_stat_values(self.layout.protocol, bindings);
        self.words = vec![0; projection.words()].into_boxed_slice();
        self.projection = Some(projection);
        Ok(())
    }
    /// The returned native call has stopped its child before this borrow.
    pub fn project(
        &mut self,
        memory: &ModuleMemory<'_>,
        table: Entities,
        module: ModuleId,
        server: &mut Server,
        mut pmove: Option<&mut crate::pmove::Pmove>,
    ) -> Result<(), CallError> {
        let Some(projection) = &self.projection else {
            return Ok(());
        };
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
            let pointer = memory.read_u64(edict + self.layout.client as u64)?;
            if pointer == 0 {
                continue;
            }
            let address = pointer
                .checked_add(self.layout.offset as u64)
                .ok_or(CallError::Memory)?;
            let words = &mut self.words[self.stats.range()];
            let bytes = memory.read(address, words.len() * 2)?;
            for (word, bytes) in words.iter_mut().zip(bytes.chunks_exact(2)) {
                *word = i16::from_le_bytes([bytes[0], bytes[1]]) as i32 as u32;
            }
            projection.apply(
                &self.words,
                &mut client.player,
                &mut PlayerContext::default(),
                |_| None,
            );
            if module_moves && let Some(pmove) = pmove.as_deref_mut() {
                pmove.import_client(memory, pointer, edict, &mut client.player)?;
                let body = server.entities.columns.body(slot);
                client.player.body.mins = body.mins;
                client.player.body.maxs = body.maxs;
                server.entities.columns.set_body(slot, client.player.body);
            }
        }
        Ok(())
    }
}
