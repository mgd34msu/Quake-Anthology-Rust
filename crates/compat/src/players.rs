//! Native numeric arrays enter the same player/value projection as snapshots.
use crate::{
    entities::{Entities, EntityProjection, native_address},
    memory::{ModuleMemory, Width},
    services::CallError,
};
use qa_core::primitives::{
    ClientId, ClockObservation, ModuleId, NativeEntity, PlayerValueBindings, Q3StatLayout,
    ResourceNamespace, ThinkTime, ValueBinding,
};
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
    (13, 108),
    (16, 112),
    (18, 116),
    (19, 12),
    (20, 68),
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
    (34, 4),
    (35, 56),
    (36, 64),
    (38, 120),
    (39, 124),
    (40, 140),
    (41, 144),
    (42, 160),
    (47, 444),
];
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
            [184, 248, 376, 312].map(|offset| ArrayLayout {
                offset,
                width: Width::Word,
            }),
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
