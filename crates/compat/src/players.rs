//! Native numeric arrays enter the same player/value projection as snapshots.
use crate::{
    entities::{Entities, EntityProjection, native_address},
    memory::{ModuleMemory, Width},
    services::CallError,
};
use qa_core::primitives::{ClientId, ModuleId, ValueBinding};
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
#[derive(Clone, Copy)]
enum PlayerAddress {
    EntityClient(&'static crate::q2_abi::ClientLayout),
    Registered,
}
#[derive(Clone, Copy)]
struct ArrayLayout {
    offset: usize,
    width: Width,
}
struct MemoryField {
    offset: u64,
    word: usize,
    width: Width,
}
pub(crate) struct PlayerValues {
    address: PlayerAddress,
    protocol: Protocol,
    arrays: [ArrayLayout; 2],
    fields: Box<[MemoryField]>,
    projection: Option<PlayerProjection>,
    words: Box<[u32]>,
}
impl PlayerValues {
    pub fn load_q2(client: &'static crate::q2_abi::ClientLayout) -> Self {
        Self::load(
            PlayerAddress::EntityClient(client),
            client.stats.protocol,
            [
                ArrayLayout {
                    offset: client.stats.offset,
                    width: Width::Short,
                },
                ArrayLayout {
                    offset: 0,
                    width: Width::Short,
                },
            ],
        )
    }
    pub fn load_q3() -> Self {
        // Original q_shared.h playerState_t: pointer-free signed32 arrays.
        Self::load(
            PlayerAddress::Registered,
            Protocol::Quake3_68,
            [
                ArrayLayout {
                    offset: 184,
                    width: Width::Word,
                },
                ArrayLayout {
                    offset: 248,
                    width: Width::Word,
                },
            ],
        )
    }
    fn load(address: PlayerAddress, protocol: Protocol, arrays: [ArrayLayout; 2]) -> Self {
        Self {
            address,
            protocol,
            arrays,
            fields: Box::default(),
            projection: None,
            words: Box::default(),
        }
    }
    pub fn bind(
        &mut self,
        stats: &[ValueBinding],
        persistent: &[ValueBinding],
    ) -> Result<(), CallError> {
        let spans = qa_network::states::player_value_layout(self.protocol).1;
        if spans
            .into_iter()
            .zip([stats, persistent])
            .any(|(span, bindings)| bindings.len() != span.count as usize)
        {
            return Err(CallError::Capacity);
        }
        let projection = PlayerProjection::load_stat_values(self.protocol, stats, persistent);
        self.fields = spans
            .into_iter()
            .zip(self.arrays)
            .flat_map(|(span, array)| {
                span.range()
                    .enumerate()
                    .map(move |(ordinal, word)| MemoryField {
                        offset: (array.offset + ordinal * array.width.len()) as u64,
                        word,
                        width: array.width,
                    })
            })
            .collect();
        self.words = vec![0; projection.words()].into_boxed_slice();
        self.projection = Some(projection);
        Ok(())
    }
    /// Native callers lend bytes only after their child is parked; QVM owns them.
    pub fn project(
        &mut self,
        memory: &ModuleMemory<'_>,
        table: Entities,
        entities: &EntityProjection,
        module: ModuleId,
        server: &mut Server,
        mut pmove: Option<&mut crate::pmove::Pmove>,
    ) -> Result<(), CallError> {
        if matches!(self.address, PlayerAddress::EntityClient(_)) && table.address == 0 {
            return Ok(());
        }
        for index in 0..server.clients.len() {
            let module_moves = server.module_moves(ClientId(index as u32)).is_some();
            let client = &mut server.clients[index];
            if client.connection.is_none() || client.module != module {
                continue;
            }
            let (pointer, edict) = match self.address {
                PlayerAddress::EntityClient(layout) => {
                    let Some(edict) =
                        native_address(table, module, &server.entities, client.entity)
                    else {
                        continue;
                    };
                    let pointer = memory.read_u64(edict + layout.stats.client as u64)?;
                    if pointer == 0 {
                        continue;
                    }
                    (pointer, Some(edict))
                }
                PlayerAddress::Registered => {
                    let Some(players) = entities.registered_players() else {
                        continue;
                    };
                    let Some(pointer) =
                        native_address(players, module, &server.entities, client.entity)
                    else {
                        continue;
                    };
                    (pointer, None)
                }
            };
            if let Some(projection) = &self.projection {
                for field in &self.fields {
                    let address = pointer.checked_add(field.offset).ok_or(CallError::Memory)?;
                    let value = memory.read_field(address, field.width)?;
                    self.words[field.word] = if matches!(field.width, Width::Short) {
                        value as i16 as i32 as u32
                    } else {
                        value
                    };
                }
                projection.apply(
                    &self.words,
                    &mut client.player,
                    &mut PlayerContext::default(),
                    qa_core::primitives::ResourceNamespace::Module(module),
                    |_| None,
                );
            }
            let PlayerAddress::EntityClient(layout) = self.address else {
                continue;
            };
            let edict = edict.ok_or(CallError::Entity)?;
            let slot = client.entity.slot as usize;
            if module_moves && let Some(pmove) = pmove.as_deref_mut() {
                let (body, _) = entities.body(memory, edict)?;
                pmove.import_client(memory, pointer, module, &mut client.player)?;
                client.player.body.position = body.position;
                client.player.body.mins = body.mins;
                client.player.body.maxs = body.maxs;
                server.entities.columns.set_body(slot, client.player.body);
            } else {
                layout.import_presentation(memory, pointer, module, &mut client.player)?;
            }
        }
        Ok(())
    }
}
