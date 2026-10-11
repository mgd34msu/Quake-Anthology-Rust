//! Native numeric stats enter the same player/value projection as snapshots.
use crate::{
    entities::{Entities, slot_address},
    memory::ModuleMemory,
    services::CallError,
};
use qa_core::primitives::{ModuleId, ValueBinding};
use qa_network::{
    commands::packet::Protocol,
    projection::{PlayerContext, PlayerProjection},
};
use qa_session::clients::Server;

#[derive(Clone, Copy)]
pub(crate) struct StatsLayout {
    pub client: usize,
    pub offset: usize,
    pub first_word: usize,
    pub protocol: Protocol,
}
pub(crate) struct PlayerStats {
    layout: StatsLayout,
    projection: Option<PlayerProjection>,
    words: Box<[u32]>,
}
impl PlayerStats {
    pub fn load(layout: StatsLayout) -> Self {
        Self {
            layout,
            projection: None,
            words: Box::default(),
        }
    }
    pub fn bind(&mut self, bindings: &[ValueBinding]) -> Result<(), CallError> {
        let bindings: Vec<_> = bindings
            .iter()
            .enumerate()
            .map(|(index, &binding)| (self.layout.first_word + index, binding))
            .collect();
        let projection = PlayerProjection::load_stats(self.layout.protocol, &bindings);
        if bindings.len() != projection.words() - self.layout.first_word {
            return Err(CallError::Capacity);
        }
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
    ) -> Result<(), CallError> {
        let Some(projection) = &self.projection else {
            return Ok(());
        };
        if table.address == 0 {
            return Ok(());
        }
        for client in &mut server.clients {
            if client.connection.is_none() || client.module != module {
                continue;
            }
            let Some(slot) = server.entities.resolve(client.entity) else {
                continue;
            };
            let Some(native) = server.entities.columns.native_entity[slot]
                .filter(|native| native.module == module)
            else {
                continue;
            };
            let address = slot_address(
                table,
                u32::try_from(native.slot).map_err(|_| CallError::Entity)?,
            )
            .ok_or(CallError::Entity)?;
            let pointer = memory.read_u64(address + self.layout.client as u64)?;
            if pointer == 0 {
                continue;
            }
            let address = pointer
                .checked_add(self.layout.offset as u64)
                .ok_or(CallError::Memory)?;
            let words = &mut self.words[self.layout.first_word..];
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
        }
        Ok(())
    }
}
