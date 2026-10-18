//! Connection-owned wire scratch and explicit entity-number projection.
use crate::output_namespace::NativeNumbers;
use qa_core::primitives::{
    Bounds, ClientId, EntityId, ModelRef, ModuleId, PlayerState, ResourceNamespace, SoundRef, Vec3,
};
use qa_network::{
    commands::packet::{Error, Protocol},
    projection::{EntityContext, EntityProjection, EntityRecord, PlayerProjection},
    snapshots::{Entity, Frame, ReceivedFrame, snapshot_limits},
    states,
};
use qa_world::{
    entities::{EntityTable, MAX_ENTITIES},
    leaves::LeafScratch,
    portals::AreaPortals,
    visibility::{PvsRows, VisibilityWorld},
};

/// SERVER inputs are borrowed only while reducing a frame. CLIENT entities
/// continue to belong to the receiving channel's native snapshot ring.
pub struct EntitySource<'a> {
    pub receiver: ClientId,
    pub entities: &'a EntityTable,
    pub players: &'a [qa_session::clients::Client],
    pub registry: &'a qa_gameplay::registry::Registry,
    pub native_module: Option<ModuleId>,
    pub self_entity: EntityId,
    pub visibility: Option<(&'a VisibilityWorld, &'a mut LeafScratch, &'a mut [u8])>,
    pub portals: Option<&'a AreaPortals>,
    pub resolve_model: &'a mut dyn FnMut(ModelRef, qa_core::primitives::ModelUse) -> Option<u32>,
    pub resolve_sound: &'a mut dyn FnMut(SoundRef) -> Option<u32>,
}

struct Rows<const N: usize> {
    rows: Box<[Entity<N>]>,
    sources: Box<[EntityId]>,
    native: Box<[Option<u32>]>,
    count: usize,
    limit: usize,
}

impl<const N: usize> Rows<N> {
    fn load(limit: usize) -> Self {
        Self {
            rows: vec![
                Entity {
                    number: 0,
                    words: [0; N]
                };
                MAX_ENTITIES
            ]
            .into_boxed_slice(),
            sources: vec![
                EntityId {
                    slot: 0,
                    generation: 0
                };
                MAX_ENTITIES
            ]
            .into_boxed_slice(),
            native: vec![None; MAX_ENTITIES].into_boxed_slice(),
            count: 0,
            limit,
        }
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "One bounded source iteration serves every native record layout"
    )]
    fn prepare(
        &mut self,
        projection: &EntityProjection,
        protocol: Protocol,
        source: &mut EntitySource<'_>,
        player: &PlayerState,
        self_number: u32,
        numbers: &mut NativeNumbers,
        visibility: &mut VisibilityScratch,
    ) -> Result<u64, Error> {
        self.count = 0;
        let mut rejected = 0;
        // sv_snapshot.c marks ps.clientNum already added before the PVS walk.
        // Cgame rebuilds this one row from PS, including spectator FOLLOW.
        let player_number = (protocol == Protocol::Quake3_68).then(|| {
            player
                .native_for(
                    protocol.rule_set(),
                    source.native_module.map(ResourceNamespace::Module),
                )
                .and_then(|native| native.client_number)
                .unwrap_or(self_number)
        });
        let origin = player.eye();
        visibility.begin(source, protocol, origin)?;
        for id in source.entities.active() {
            let slot = id.slot as usize;
            let columns = &source.entities.columns;
            let native = if id == source.self_entity {
                Some(self_number)
            } else {
                match columns.native_entity[slot]
                    .filter(|native| Some(native.module) == source.native_module)
                {
                    Some(_) => match numbers.project(source.entities, id) {
                        Some(number) => Some(number),
                        None => {
                            rejected += 1;
                            continue;
                        }
                    },
                    None => None,
                }
            };
            // World ordinals are protocol metadata, never packet entities.
            if native == Some(snapshot_limits(protocol).world_entity) {
                continue;
            }
            if native.is_some() && native == player_number {
                continue;
            }
            let appearance = columns.appearance[slot];
            if appearance.snapshot_excluded {
                continue;
            }
            // QW playerinfo is emitted independently of packetentities.
            if protocol == Protocol::QuakeWorld28
                && native.is_some_and(|n| n <= states::QW_PLAYER_SLOTS as u32)
            {
                continue;
            }
            if id != source.self_entity {
                if !qa_gameplay::rules::appearance::policy(appearance.source)
                    .snapshot_present(&appearance)
                {
                    continue;
                }
                let bounds = Bounds {
                    mins: columns.position[slot] + columns.mins[slot],
                    maxs: columns.position[slot] + columns.maxs[slot],
                };
                if !bounds.is_valid() || !visibility.visible(source, bounds) {
                    continue;
                }
            }
            if self.count == self.rows.len() {
                rejected += 1;
                continue;
            }
            self.sources[self.count] = id;
            self.native[self.count] = native;
            self.count += 1;
        }
        // Assign all visible lifetimes before reducing native references. A
        // secondary module's event target may appear after its event in SoA order.
        for index in 0..self.count {
            let id = self.sources[index];
            self.rows[index].number = if self.native[index].is_some() {
                numbers.number(id)
            } else {
                numbers.foreign(id)
            }
            .unwrap_or(u32::MAX);
        }
        let mut count = 0;
        for index in 0..self.count {
            let mut row = self.rows[index];
            if row.number == u32::MAX {
                rejected += 1;
                continue;
            }
            let slot = self.sources[index].slot as usize;
            let columns = &source.entities.columns;
            let appearance = columns.appearance[slot];
            let Some(tags) = source
                .entities
                .collision_tags(self.sources[index], columns.collision_tags[slot])
            else {
                rejected += 1;
                continue;
            };
            let context = EntityContext {
                tags,
                client: numbers.client_reference(
                    source.entities,
                    self.sources[index],
                    source.receiver,
                ),
            };
            let record = EntityRecord {
                position: columns.position[slot],
                velocity: columns.velocity[slot],
                angles: columns.angles[slot],
                appearance,
            };
            if !projection.reduce(
                &record,
                &context,
                &mut row.words,
                &mut *source.resolve_model,
                &mut *source.resolve_sound,
                |reference| numbers.reference(source.entities, source.receiver, reference),
            ) {
                rejected += 1;
                continue;
            }
            self.rows[count] = row;
            count += 1;
        }
        self.rows[..count].sort_unstable_by_key(|row| row.number);
        rejected += count.saturating_sub(self.limit) as u64;
        self.count = count.min(self.limit);
        Ok(rejected)
    }
}

enum NativeRows {
    NetQuake(Rows<{ states::NQ_ENTITY_WORDS }>),
    QuakeWorld(Rows<{ states::QW_ENTITY_WORDS }>),
    Quake2(Rows<{ states::Q2_ENTITY_WORDS }>),
    Quake2Repro(Rows<{ states::Q2_RERELEASE_ENTITY_WORDS }>),
    Quake3(Rows<{ states::ENTITY_WORDS }>),
}

struct VisibilityScratch {
    leaves: Box<[u32]>,
    pvs: Box<[u8]>,
    areas: [u8; 255],
    area_count: usize,
    viewer_area: Option<u32>,
    full_leaf_queries: u64,
}

impl VisibilityScratch {
    fn load() -> Self {
        Self {
            leaves: vec![0; MAX_ENTITIES].into_boxed_slice(),
            pvs: Box::default(),
            areas: [0; 255],
            area_count: 0,
            viewer_area: None,
            full_leaf_queries: 0,
        }
    }

    fn load_world(&mut self, world: &VisibilityWorld) {
        let leaves = world
            .leaf_count()
            .max(world.node_count().saturating_mul(2) + 1);
        self.leaves = vec![0; leaves].into_boxed_slice();
        self.pvs = vec![0; world.pvs().row_bytes()].into_boxed_slice();
    }

    fn begin(
        &mut self,
        source: &mut EntitySource<'_>,
        protocol: Protocol,
        origin: Vec3,
    ) -> Result<(), Error> {
        self.area_count = 0;
        self.viewer_area = None;
        let Some((world, scratch, row)) = source.visibility.as_mut() else {
            return Ok(());
        };
        if self.pvs.len() != world.pvs().row_bytes() {
            return Err(Error::Count);
        }
        let viewer = world
            .point_in_leaf(
                origin,
                qa_gameplay::rules::point_leaf_rules(protocol.rule_set()),
            )
            .and_then(|leaf| world.leaf(leaf))
            .ok_or(Error::Context)?;
        self.viewer_area = viewer.area;
        self.pvs.fill(0);
        if protocol == Protocol::Quake3_68 {
            world
                .pvs()
                .read_into(viewer.selector, row)
                .map_err(|_| Error::Context)?;
            self.pvs.copy_from_slice(row);
        } else {
            let bounds = Bounds {
                mins: origin - Vec3([8.; 3]),
                maxs: origin + Vec3([8.; 3]),
            };
            let found = world
                .box_leaves(bounds, &mut self.leaves, scratch)
                .ok_or(Error::Count)?;
            if found.count == self.leaves.len() {
                self.pvs.fill(255);
                self.full_leaf_queries += 1;
            } else {
                for &leaf in &self.leaves[..found.count] {
                    let leaf = world.leaf(leaf).ok_or(Error::Context)?;
                    if leaf.solid {
                        continue;
                    }
                    world
                        .pvs()
                        .read_into(leaf.selector, row)
                        .map_err(|_| Error::Context)?;
                    for (combined, value) in self.pvs.iter_mut().zip(row.iter()) {
                        *combined |= value;
                    }
                }
            }
        }
        if let Some(portals) = source.portals {
            let limit = snapshot_limits(protocol).area_capacity;
            self.area_count = portals.count().div_ceil(8).min(limit);
            self.areas[..self.area_count].fill(0);
            for area in 0..portals.count().min(self.area_count * 8) {
                if self.viewer_area.is_none_or(|viewer| {
                    viewer == 0 || portals.connected(viewer, area as u32).unwrap_or(false)
                }) {
                    self.areas[area / 8] |= 1 << (area & 7);
                }
            }
            // Common frames store hidden areas; each native codec owns wire polarity.
            for byte in &mut self.areas[..self.area_count] {
                *byte = qa_network::snapshots::native_area_byte(*byte, true);
            }
        }
        Ok(())
    }

    fn visible(&mut self, source: &mut EntitySource<'_>, bounds: Bounds) -> bool {
        let Some((world, scratch, _)) = source.visibility.as_mut() else {
            return true;
        };
        let Some(found) = world.box_leaves(bounds, &mut self.leaves, scratch) else {
            return true;
        };
        if found.count == self.leaves.len() {
            self.full_leaf_queries += 1;
            return true;
        }
        self.leaves[..found.count].iter().any(|&index| {
            let Some(leaf) = world.leaf(index) else {
                return true;
            };
            if leaf.solid {
                return false;
            }
            if let (Some(viewer), Some(area), Some(portals)) =
                (self.viewer_area, leaf.area, source.portals)
                && viewer != 0
                && !portals.connected(viewer, area).unwrap_or(false)
            {
                return false;
            }
            leaf.selector
                .is_none_or(|selector| PvsRows::row_contains(&self.pvs, selector))
        })
    }
}

pub(super) struct EntityScratch {
    protocol: Protocol,
    projection: EntityProjection,
    rows: NativeRows,
    visibility: VisibilityScratch,
    qw_players: Box<[QwPlayer]>,
    qw_player_count: usize,
    pub rejected: u64,
}

#[derive(Clone, Copy)]
struct QwPlayer {
    number: u32,
    words: [u32; states::QW_PLAYER_WORDS],
    command: qa_network::commands::QwCmd,
    is_self: bool,
}

#[derive(Clone, Copy)]
pub(super) struct Selection {
    pub entities: usize,
    pub players: usize,
}

impl EntityScratch {
    pub fn load(protocol: Protocol) -> Self {
        let capacity = snapshot_limits(protocol).entity_capacity;
        Self {
            protocol,
            projection: EntityProjection::load(protocol),
            rows: match protocol {
                Protocol::NetQuake15 => NativeRows::NetQuake(Rows::load(capacity)),
                Protocol::QuakeWorld28 => NativeRows::QuakeWorld(Rows::load(capacity)),
                Protocol::Quake2_34 => NativeRows::Quake2(Rows::load(capacity)),
                Protocol::Quake2Repro1038 => NativeRows::Quake2Repro(Rows::load(capacity)),
                Protocol::Quake3_68 => NativeRows::Quake3(Rows::load(capacity)),
            },
            visibility: VisibilityScratch::load(),
            qw_players: vec![
                QwPlayer {
                    number: 0,
                    words: [0; states::QW_PLAYER_WORDS],
                    command: qa_network::commands::packet::ZERO_QW,
                    is_self: false,
                };
                states::QW_PLAYER_SLOTS
            ]
            .into_boxed_slice(),
            qw_player_count: 0,
            rejected: 0,
        }
    }

    pub fn load_visibility(&mut self, world: &VisibilityWorld) {
        self.visibility.load_world(world);
    }
    pub fn projection(&self) -> &EntityProjection {
        &self.projection
    }
    pub fn conservative_leaf_queries(&self) -> u64 {
        self.visibility.full_leaf_queries
    }

    pub fn prepare(
        &mut self,
        source: &mut EntitySource<'_>,
        player: &PlayerState,
        numbers: &mut NativeNumbers,
    ) -> Result<(), Error> {
        numbers.refresh(
            source.entities,
            source.native_module,
            self.protocol,
            source
                .players
                .iter()
                .filter(|client| client.is_server_active())
                .map(|client| client.entity),
        );
        let number = numbers.number(source.self_entity).ok_or(Error::Context)?;
        macro_rules! prepare {
            ($rows:expr) => {
                $rows.prepare(
                    &self.projection,
                    self.protocol,
                    source,
                    player,
                    number,
                    numbers,
                    &mut self.visibility,
                )
            };
        }
        self.rejected += match &mut self.rows {
            NativeRows::NetQuake(rows) => prepare!(rows)?,
            NativeRows::QuakeWorld(rows) => prepare!(rows)?,
            NativeRows::Quake2(rows) => prepare!(rows)?,
            NativeRows::Quake2Repro(rows) => prepare!(rows)?,
            NativeRows::Quake3(rows) => prepare!(rows)?,
        };
        Ok(())
    }

    pub fn prepare_qw_players(
        &mut self,
        source: &mut EntitySource<'_>,
        projection: &PlayerProjection,
        numbers: &NativeNumbers,
    ) -> Result<(), Error> {
        self.qw_player_count = 0;
        if self.protocol != Protocol::QuakeWorld28 {
            return Ok(());
        }
        for client in source.players {
            if !client.is_server_active() {
                continue;
            }
            let Some(slot) = source.entities.resolve(client.entity) else {
                continue;
            };
            let Some(number) = numbers.number(client.entity) else {
                continue;
            };
            if number == 0 || number > self.qw_players.len() as u32 {
                continue;
            }
            let is_self = client.entity == source.self_entity;
            let bounds = client.player.body.bounds();
            if !is_self && (!bounds.is_valid() || !self.visibility.visible(source, bounds)) {
                continue;
            }
            let appearance = source.entities.columns.appearance[slot];
            if appearance.snapshot_excluded {
                continue;
            }
            let context = super::player_context(
                &client.player,
                source,
                self.protocol,
                number - 1,
                source.entities.columns.angles[slot].0[1],
                Some(appearance),
                is_self,
            );
            let Some(row) = self.qw_players.get_mut(self.qw_player_count) else {
                self.rejected += 1;
                continue;
            };
            if !projection.reduce(
                &client.player,
                &context,
                &mut row.words,
                &mut *source.resolve_model,
                &mut *source.resolve_sound,
            ) {
                self.rejected += 1;
                continue;
            }
            row.number = number - 1;
            row.command = qa_network::commands::to_qw_usercmd(&client.command);
            row.is_self = is_self;
            self.qw_player_count += 1;
        }
        self.qw_players[..self.qw_player_count].sort_unstable_by_key(|row| row.number);
        Ok(())
    }

    pub fn write_qw_players(
        &self,
        writer: &mut qa_network::message::Writer<'_>,
        count: usize,
    ) -> Result<(), Error> {
        let minimum = self.qw_players[..self.qw_player_count]
            .iter()
            .any(|row| row.is_self) as usize;
        let mut others = count.saturating_sub(minimum);
        for row in &self.qw_players[..self.qw_player_count] {
            if row.is_self || others != 0 {
                states::write_qw_player(writer, row.number, &row.words, row.command)?;
                if !row.is_self {
                    others -= 1;
                }
            }
        }
        Ok(())
    }

    pub fn selection(&self) -> Selection {
        let entities = match &self.rows {
            NativeRows::NetQuake(rows) => rows.count,
            NativeRows::QuakeWorld(rows) => rows.count,
            NativeRows::Quake2(rows) => rows.count,
            NativeRows::Quake2Repro(rows) => rows.count,
            NativeRows::Quake3(rows) => rows.count,
        };
        Selection {
            entities,
            players: self.qw_player_count,
        }
    }

    pub fn reduce_selection(&self, selection: &mut Selection) -> bool {
        if selection.entities != 0 {
            selection.entities /= 2;
            return true;
        }
        let minimum = self.qw_players[..self.qw_player_count]
            .iter()
            .any(|row| row.is_self) as usize;
        if selection.players > minimum {
            selection.players = (selection.players / 2).max(minimum);
            return true;
        }
        false
    }

    pub fn count_selection(&mut self, selection: Selection) {
        let full = self.selection();
        self.rejected += (full.entities.saturating_sub(selection.entities)
            + full.players.saturating_sub(selection.players)) as u64;
    }

    pub fn frame<'a>(
        &'a self,
        sequence: u32,
        time: qa_core::primitives::ThinkTime,
        flags: u8,
        words: &'a [u32; states::PLAYER_WORDS],
        count: usize,
    ) -> Result<ReceivedFrame<'a>, Error> {
        macro_rules! frame {
            ($variant:ident, $rows:expr, $n:expr) => {
                ReceivedFrame::$variant(Frame {
                    sequence,
                    time,
                    command: 0,
                    flags,
                    qw_peers: 0,
                    qw_dead: 0,
                    areas: &self.visibility.areas[..self.visibility.area_count],
                    player: words[..$n].try_into().map_err(|_| Error::Count)?,
                    entities: &$rows.rows[..count.min($rows.count)],
                })
            };
        }
        Ok(match &self.rows {
            NativeRows::NetQuake(rows) => frame!(NetQuake, rows, states::NQ_PLAYER_WORDS),
            NativeRows::QuakeWorld(rows) => frame!(QuakeWorld, rows, 0),
            NativeRows::Quake2(rows) => frame!(Quake2, rows, states::Q2_PLAYER_WORDS),
            NativeRows::Quake2Repro(rows) => {
                frame!(Quake2Repro, rows, states::Q2_REPRO_PLAYER_WORDS)
            }
            NativeRows::Quake3(rows) => frame!(Quake3, rows, states::PLAYER_WORDS),
        })
    }
}
