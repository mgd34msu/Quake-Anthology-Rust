//! Native numeric arrays enter the same player/value projection as snapshots.
use crate::{
    entities::{Entities, EntityProjection, native_address},
    memory::{ModuleMemory, Width},
    services::CallError,
};
use qa_core::primitives::{ClientId, ModuleId, NativeEntity, ResourceNamespace, ValueBinding};
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
// Original q_shared.h playerState_t offsets, mapped into the one player
// projection's numeric words. Both native widths have this pointer-free ABI.
const Q3_PLAYER_FIELDS: &[(usize, u64)] = &[
    (0, 0),
    (1, 20),
    (2, 24),
    (4, 32),
    (5, 36),
    (6, 156),
    (7, 152),
    (8, 44),
    (9, 28),
    (10, 40),
    (12, 16),
    (19, 12),
    (20, 68),
    (24, 48),
    (25, 52),
    (26, 60),
    (28, 164),
    (34, 4),
    (35, 56),
    (36, 64),
    (40, 140),
    (41, 144),
    (42, 160),
];
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
        let mut values = Self::load(
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
        );
        // Body/view/movement publication is useful even without HUD bank rows.
        values.bind_fields(&[], &[]);
        values
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
        self.bind_fields(stats, persistent);
        Ok(())
    }
    fn bind_fields(&mut self, stats: &[ValueBinding], persistent: &[ValueBinding]) {
        let (projection, native_fields) = match self.address {
            PlayerAddress::Registered => (
                PlayerProjection::load_native_q3(stats, persistent),
                Q3_PLAYER_FIELDS,
            ),
            PlayerAddress::EntityClient(_) => (
                PlayerProjection::load_stat_values(self.protocol, stats, persistent),
                &[][..],
            ),
        };
        let spans = qa_network::states::player_value_layout(self.protocol).1;
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
            .chain(native_fields.iter().map(|&(word, offset)| MemoryField {
                offset,
                word,
                width: Width::Word,
            }))
            .collect();
        self.words = vec![0; projection.words()].into_boxed_slice();
        self.projection = Some(projection);
    }
    /// Native callers lend bytes only after their child is parked; QVM owns them.
    pub fn project(
        &mut self,
        memory: &ModuleMemory<'_>,
        table: Entities,
        entities: &EntityProjection,
        module: ModuleId,
        registry: &qa_gameplay::registry::Registry,
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
            let Some(slot) = server.entities.resolve(client.entity) else {
                continue;
            };
            if server.entities.columns.owner[slot] != module {
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
                    let Some(native) = server.entities.columns.native_entity[slot]
                        .filter(|native| native.module == module)
                    else {
                        continue;
                    };
                    if entities.binding(&server.entities, native)? != Some(client.entity) {
                        continue;
                    }
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
                let mut context = PlayerContext::default();
                let namespace = ResourceNamespace::Module(module);
                let imported = if module_moves {
                    projection.apply(
                        &self.words,
                        &mut client.player,
                        &mut context,
                        namespace,
                        |number| {
                            entities
                                .binding(
                                    &server.entities,
                                    NativeEntity {
                                        module,
                                        slot: number as i32,
                                    },
                                )
                                .ok()
                                .flatten()
                                .filter(|entity| {
                                    server.entities.columns.owner[entity.slot as usize] == module
                                })
                        },
                    )
                } else {
                    projection.apply_stats(&self.words, &mut client.player, namespace)
                };
                if !imported {
                    return Err(CallError::Capacity);
                }
                if module_moves && matches!(self.address, PlayerAddress::Registered) {
                    registry.project_player_weapon(&mut client.player);
                    if native_address(table, module, &server.entities, client.entity).is_some() {
                        // playerState origin remains authoritative for unlinked
                        // spectators; sharedEntity contributes only body bounds.
                        client.player.body.mins = server.entities.columns.mins[slot];
                        client.player.body.maxs = server.entities.columns.maxs[slot];
                    }
                }
            }
            let PlayerAddress::EntityClient(layout) = self.address else {
                continue;
            };
            let edict = edict.ok_or(CallError::Entity)?;
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
