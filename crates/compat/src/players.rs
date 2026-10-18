//! Native numeric arrays enter the same player/value projection as snapshots.
use crate::{
    entities::{Entities, EntityProjection, native_address},
    memory::{ModuleMemory, Width},
    services::CallError,
};
use qa_core::primitives::{
    ClientId, ClockObservation, EntityId, ModuleId, NativeEntity, PlayerValueBindings,
    Q3StatLayout, ResourceNamespace, ThinkTime, ValueBinding, Vec3,
};
use qa_network::{
    commands::packet::Protocol,
    projection::{PlayerContext, PlayerProjection},
};
use qa_session::clients::Server;
use qa_world::entities::EntityTable;

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
#[derive(Clone, Copy)]
struct MemoryField {
    offset: u64,
    word: usize,
    width: Width,
}
pub const Q3_PLAYER_STATE_BYTES: usize = 468;
pub const Q3_ENTITY_STATE_BYTES: usize = 208;

const Q3_PLAYER_ARRAYS: [ArrayLayout; 4] = [
    ArrayLayout {
        offset: 184,
        width: Width::Word,
    },
    ArrayLayout {
        offset: 248,
        width: Width::Word,
    },
    ArrayLayout {
        offset: 376,
        width: Width::Word,
    },
    ArrayLayout {
        offset: 312,
        width: Width::Word,
    },
];
// Original q_shared.h playerState_t offsets, mapped into the one player
// projection's numeric words. Both native widths have this pointer-free ABI.
const Q3_PLAYER_FIELDS: &[(usize, u64)] = &[
    (0, 0),
    (1, 20),
    (2, 24),
    (3, 8),
    (4, 32),
    (5, 36),
    (6, 156),
    (7, 152),
    (8, 44),
    (9, 28),
    (10, 40),
    (11, 72),
    (12, 16),
    (13, 108),
    (14, 84),
    (15, 88),
    (16, 112),
    (17, 76),
    (18, 116),
    (19, 12),
    (20, 68),
    (21, 148),
    (22, 104),
    (23, 128),
    (24, 48),
    (25, 52),
    (26, 60),
    (27, 132),
    (28, 164),
    (29, 168),
    (30, 172),
    (31, 176),
    (32, 180),
    (33, 440),
    (34, 4),
    (35, 56),
    (36, 64),
    (37, 80),
    (38, 120),
    (39, 124),
    (40, 140),
    (41, 144),
    (42, 160),
    (43, 92),
    (44, 96),
    (45, 100),
    (46, 448),
    (47, 444),
];
// Original msg.c entityStateFields order. The entity number is outside the
// delta field array and occupies the first public ABI cell.
const Q3_ENTITY_FIELDS: &[(usize, u64)] = &[
    (0, 16),
    (1, 24),
    (2, 28),
    (3, 36),
    (4, 40),
    (5, 32),
    (6, 64),
    (7, 44),
    (8, 60),
    (9, 180),
    (10, 132),
    (11, 4),
    (12, 200),
    (13, 184),
    (14, 196),
    (15, 148),
    (16, 12),
    (17, 8),
    (18, 140),
    (19, 192),
    (20, 168),
    (21, 120),
    (22, 20),
    (23, 48),
    (24, 92),
    (25, 96),
    (26, 100),
    (27, 176),
    (28, 188),
    (29, 160),
    (30, 144),
    (31, 156),
    (32, 204),
    (33, 112),
    (34, 104),
    (35, 108),
    (36, 164),
    (37, 116),
    (38, 84),
    (39, 52),
    (40, 56),
    (41, 68),
    (42, 72),
    (43, 76),
    (44, 80),
    (45, 88),
    (46, 124),
    (47, 128),
    (48, 136),
    (49, 152),
    (50, 172),
];

/// Collision and publication select offsets from the same public ABI table.
pub(crate) const fn q3_entity_offset(word: usize) -> usize {
    let mut field = 0;
    while field < Q3_ENTITY_FIELDS.len() {
        if Q3_ENTITY_FIELDS[field].0 == word {
            return Q3_ENTITY_FIELDS[field].1 as usize;
        }
        field += 1;
    }
    // Only compile-time, table-owned field ordinals call this helper.
    Q3_ENTITY_STATE_BYTES
}

fn array_fields(protocol: Protocol, arrays: [ArrayLayout; 4]) -> impl Iterator<Item = MemoryField> {
    qa_network::states::player_value_layout(protocol)
        .1
        .into_iter()
        .zip(arrays)
        .flat_map(|(span, array)| {
            span.range()
                .enumerate()
                .map(move |(ordinal, word)| MemoryField {
                    offset: (array.offset + ordinal * array.width.len()) as u64,
                    word,
                    width: array.width,
                })
        })
}

fn scalar_fields(fields: &[(usize, u64)]) -> impl Iterator<Item = MemoryField> + '_ {
    fields.iter().map(|&(word, offset)| MemoryField {
        offset,
        word,
        width: Width::Word,
    })
}

fn write_fields(output: &mut [u8], words: &[u32], fields: impl Iterator<Item = MemoryField>) {
    for field in fields {
        let start = field.offset as usize;
        let width = field.width.len();
        output[start..start + width].copy_from_slice(&words[field.word].to_le_bytes()[..width]);
    }
}

fn read_fields(
    memory: &ModuleMemory<'_>,
    base: u64,
    words: &mut [u32],
    fields: impl Iterator<Item = MemoryField>,
) -> Result<(), CallError> {
    for field in fields {
        let address = base.checked_add(field.offset).ok_or(CallError::Memory)?;
        let value = memory.read_field(address, field.width)?;
        words[field.word] = if matches!(field.width, Width::Short) {
            value as i16 as i32 as u32
        } else {
            value
        };
    }
    Ok(())
}

/// A parked native child publishes one checked public ES through the same
/// descriptor used by GetSnapshot. The words are invocation-local scratch.
pub(crate) fn read_q3_entity_state(
    memory: &ModuleMemory<'_>,
    base: u64,
) -> Result<[u32; qa_network::states::ENTITY_WORDS], CallError> {
    memory.read(base, Q3_ENTITY_STATE_BYTES)?;
    let mut words = [0; qa_network::states::ENTITY_WORDS];
    read_fields(memory, base, &mut words, scalar_fields(Q3_ENTITY_FIELDS))?;
    Ok(words)
}

/// Reconstructs the public pointer-free ABI from accepted Q3 snapshot words.
/// The five cells absent from MSG-null decoding remain zero; transmitted
/// cells retain their raw bits without another native or wire conversion.
pub fn write_q3_player_state(
    memory: &mut ModuleMemory<'_>,
    base: u64,
    words: &[u32],
) -> Result<(), CallError> {
    if words.len() < qa_network::states::PLAYER_WORDS {
        return Err(CallError::Capacity);
    }
    let output = memory.read_mut(base, Q3_PLAYER_STATE_BYTES)?;
    write_fields(
        output,
        words,
        array_fields(Protocol::Quake3_68, Q3_PLAYER_ARRAYS).chain(scalar_fields(Q3_PLAYER_FIELDS)),
    );
    for offset in [136, 452, 456, 460, 464] {
        output[offset..offset + 4].fill(0);
    }
    Ok(())
}

/// The native entity number is supplied by the accepted snapshot row, never
/// derived from a common entity slot. All public ES cells are transmitted.
pub fn write_q3_entity_state(
    memory: &mut ModuleMemory<'_>,
    base: u64,
    number: u32,
    words: &[u32],
) -> Result<(), CallError> {
    if words.len() < qa_network::states::ENTITY_WORDS {
        return Err(CallError::Capacity);
    }
    let output = memory.read_mut(base, Q3_ENTITY_STATE_BYTES)?;
    write_fields(output, words, scalar_fields(Q3_ENTITY_FIELDS));
    output[..4].copy_from_slice(&number.to_le_bytes());
    Ok(())
}
pub(crate) struct PlayerValues {
    address: PlayerAddress,
    protocol: Protocol,
    arrays: [ArrayLayout; 4],
    stat_layout: Q3StatLayout,
    /// Immutable numeric metadata; native values stay in the common ValueBank.
    bindings: [Box<[ValueBinding]>; 4],
    items: Option<ValueBinding>,
    inventory: Option<qa_gameplay::registry::NativePlayerBindings>,
    fields: Box<[MemoryField]>,
    projection: Option<PlayerProjection>,
    words: Box<[u32]>,
}
impl PlayerValues {
    pub fn load_q2(client: &'static crate::q2_abi::ClientLayout) -> Self {
        Self::load(
            PlayerAddress::EntityClient(client),
            client.stats.protocol,
            std::array::from_fn(|index| ArrayLayout {
                offset: if index == 0 { client.stats.offset } else { 0 },
                width: Width::Short,
            }),
            Q3StatLayout::BASE,
        )
    }
    pub fn load_q3(layout: Q3StatLayout) -> Self {
        // Original q_shared.h playerState_t: pointer-free signed32 arrays.
        let mut values = Self::load(
            PlayerAddress::Registered,
            Protocol::Quake3_68,
            Q3_PLAYER_ARRAYS,
            layout,
        );
        // Body/view/movement publication is useful even without HUD bank rows.
        values.bind_fields(PlayerValueBindings::EMPTY);
        values
    }
    fn load(
        address: PlayerAddress,
        protocol: Protocol,
        arrays: [ArrayLayout; 4],
        stat_layout: Q3StatLayout,
    ) -> Self {
        Self {
            address,
            protocol,
            arrays,
            stat_layout,
            bindings: std::array::from_fn(|_| Box::default()),
            items: None,
            inventory: None,
            fields: Box::default(),
            projection: None,
            words: Box::default(),
        }
    }
    pub fn stat_layout(&self) -> Q3StatLayout {
        self.stat_layout
    }

    fn pointer(
        &self,
        memory: &ModuleMemory<'_>,
        table: Entities,
        entities: &EntityProjection,
        module: ModuleId,
        common: &EntityTable,
        entity: EntityId,
    ) -> Result<Option<(u64, Option<u64>)>, CallError> {
        match self.address {
            PlayerAddress::EntityClient(layout) => {
                let Some(edict) = native_address(table, module, common, entity) else {
                    return Ok(None);
                };
                let pointer = memory.read_u64(edict + layout.stats.client as u64)?;
                Ok((pointer != 0).then_some((pointer, Some(edict))))
            }
            PlayerAddress::Registered => {
                let Some(players) = entities.registered_players() else {
                    return Ok(None);
                };
                let Some(slot) = common.resolve(entity) else {
                    return Ok(None);
                };
                let Some(native) =
                    common.columns.native_entity[slot].filter(|native| native.module == module)
                else {
                    return Ok(None);
                };
                if entities.binding(common, native)? != Some(entity) {
                    return Ok(None);
                }
                Ok(native_address(players, module, common, entity).map(|pointer| (pointer, None)))
            }
        }
    }

    pub fn write_position(
        &self,
        memory: &mut ModuleMemory<'_>,
        table: Entities,
        entities: &EntityProjection,
        context: crate::services::CallContext,
        server: &Server,
        native: NativeEntity,
        mut pmove: Option<&mut crate::pmove::Pmove>,
        position: Vec3,
    ) -> Result<(), CallError> {
        let Some(client) = server.client_for_native(native) else {
            return Ok(());
        };
        let entity = server.clients[client.0 as usize].entity;
        let Some((pointer, _)) = self.pointer(
            memory,
            table,
            entities,
            context.module,
            &server.entities,
            entity,
        )?
        else {
            return Ok(());
        };
        match self.address {
            PlayerAddress::EntityClient(_) => pmove
                .as_deref_mut()
                .ok_or(CallError::Memory)?
                .write_position(memory, pointer, position),
            PlayerAddress::Registered => {
                let projection = self.projection.as_ref().ok_or(CallError::Memory)?;
                for (word, value) in projection.position_words(position) {
                    let field = self
                        .fields
                        .iter()
                        .find(|field| field.word == word)
                        .ok_or(CallError::Memory)?;
                    memory.write_field(pointer + field.offset, field.width, value)?;
                }
                Ok(())
            }
        }
    }
    pub fn observe_clock(&self, clock: ClockObservation) -> Result<ClockObservation, CallError> {
        if !clock.is_valid() {
            return Err(CallError::Capacity);
        }
        match (self.address, clock.native) {
            (PlayerAddress::Registered, ThinkTime::Milliseconds(value)) => Ok(ClockObservation {
                native: ThinkTime::Milliseconds(i64::from(value as i32)),
                ..clock
            }),
            (PlayerAddress::Registered, _) => Err(CallError::Capacity),
            _ => Ok(clock),
        }
    }
    pub fn bind(
        &mut self,
        values: PlayerValueBindings<'_>,
        registry: &qa_gameplay::registry::Registry,
    ) -> Result<(), CallError> {
        let spans = qa_network::states::player_value_layout(self.protocol).1;
        if values.items.is_some()
            || spans
                .into_iter()
                .zip(values.arrays)
                .any(|(span, bindings)| bindings.len() != span.count as usize)
            || matches!(self.address, PlayerAddress::Registered)
                && !self.stat_layout.fits(values.arrays[0].len())
        {
            return Err(CallError::Capacity);
        }
        self.bind_fields(values);
        self.bindings = values.arrays.map(|array| array.into());
        self.items = values.items;
        self.inventory = if matches!(self.address, PlayerAddress::Registered) {
            qa_gameplay::registry::NativePlayerBindings::load(self.protocol.rule_set(), registry)
        } else {
            None
        };
        Ok(())
    }
    fn bindings(&self) -> PlayerValueBindings<'_> {
        PlayerValueBindings {
            arrays: std::array::from_fn(|index| self.bindings[index].as_ref()),
            items: self.items,
        }
    }
    fn bind_fields(&mut self, values: PlayerValueBindings<'_>) {
        let (projection, native_fields) = match self.address {
            PlayerAddress::Registered => (
                PlayerProjection::load_native_q3(values, self.stat_layout),
                Q3_PLAYER_FIELDS,
            ),
            PlayerAddress::EntityClient(_) => (
                PlayerProjection::load_stat_values(self.protocol, values, self.stat_layout),
                &[][..],
            ),
        };
        self.fields = array_fields(self.protocol, self.arrays)
            .chain(scalar_fields(native_fields))
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
        time: qa_core::sys_events::EventTime,
        clock: ClockObservation,
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
            let Some((pointer, edict)) = self.pointer(
                memory,
                table,
                entities,
                module,
                &server.entities,
                client.entity,
            )?
            else {
                continue;
            };
            if let Some(projection) = &self.projection {
                if self.inventory.as_ref().is_some_and(|inventory| {
                    !inventory.packet_fits(
                        self.bindings(),
                        self.stat_layout,
                        &client.player,
                        Some(clock),
                    )
                }) {
                    return Err(CallError::Capacity);
                }
                read_fields(
                    memory,
                    pointer,
                    &mut self.words,
                    self.fields.iter().copied(),
                )?;
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
                if let Some(inventory) = &self.inventory
                    && !inventory.import_packet(
                        self.bindings(),
                        self.stat_layout,
                        namespace,
                        &mut client.player,
                        time,
                        Some(clock),
                    )
                {
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
                client.player.native_clock = Some(clock);
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
            client.player.native_clock = Some(clock);
        }
        Ok(())
    }
}
