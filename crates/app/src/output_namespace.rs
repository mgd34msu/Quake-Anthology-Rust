//! One connection-owned entity-number projection shared by snapshots and cues.
use qa_core::{
    primitives::{
        ClientId, EntityId, ModuleId, NativeClientRef, NativeReference, ResourceNamespace,
    },
    stamps::StampSet,
};
use qa_network::{commands::packet::Protocol, snapshots::snapshot_limits, states};
use qa_world::entities::{EntityTable, MAX_ENTITIES};

/// Protocol client ordinals differ from entity ordinals by the native world slot.
pub fn native_client_entity(protocol: Protocol, client: u32) -> Option<u32> {
    client.checked_add(snapshot_limits(protocol).first_entity)
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum MappingRole {
    #[default]
    Entity,
    Player,
    InactivePlayer,
    RetiringPlayer,
}

#[derive(Clone, Copy, Default)]
struct Mapping {
    id: Option<EntityId>,
    number: Option<u32>,
    role: MappingRole,
}

/// Native ordinals and common lifetimes have separate namespaces. Foreign
/// lifetimes retain their number until release or a native ordinal claims it.
pub struct NativeNumbers {
    slots: Box<[Mapping]>,
    owners: Box<[Option<EntityId>]>,
    reserved: StampSet,
    module: Option<ModuleId>,
    native_clients: u32,
    source_entity_first: u32,
    self_claim: Option<(EntityId, u32)>,
    first: usize,
    player_end: usize,
    world_entity: u32,
    next_foreign: usize,
    end: usize,
    peer_seeded: bool,
}

impl NativeNumbers {
    pub fn load(protocol: Protocol) -> Self {
        let limits = snapshot_limits(protocol);
        let limit = snapshot_limits(Protocol::Quake2Repro1038).native_numbers;
        Self {
            slots: vec![Mapping::default(); MAX_ENTITIES].into_boxed_slice(),
            owners: vec![None; limit].into_boxed_slice(),
            reserved: StampSet::new(limit),
            module: None,
            native_clients: 1,
            source_entity_first: limits.first_entity,
            self_claim: None,
            first: limits.first_entity as usize,
            player_end: limits.first_entity as usize,
            world_entity: limits.world_entity,
            next_foreign: limits.first_entity as usize,
            end: limits.end_entity as usize,
            peer_seeded: false,
        }
    }

    pub(crate) fn peer_reset_count(&self) -> u32 {
        if self.peer_seeded {
            0
        } else {
            self.player_count()
        }
    }
    pub(crate) fn peers_seeded(&mut self) {
        self.peer_seeded = true;
    }

    pub fn native_clients(&self) -> u32 {
        self.native_clients
    }
    pub fn player_count(&self) -> u32 {
        (self.player_end - self.first) as u32
    }
    pub fn is_identity_namespace(&self, module: ModuleId) -> bool {
        self.module == Some(module) && self.source_entity_first == self.first as u32
    }
    pub fn set_native_clients(&mut self, clients: u32, source_entity_first: u32) {
        self.native_clients = clients;
        self.source_entity_first = source_entity_first;
    }
    pub fn set_self(&mut self, entity: EntityId, native_client: u32, protocol: Protocol) {
        self.self_claim =
            native_client_entity(protocol, native_client).map(|number| (entity, number));
    }
    pub fn reset(&mut self) {
        self.peer_seeded = false;
        self.slots.fill(Mapping::default());
        self.owners.fill(None);
        self.module = None;
        self.reserved.begin();
        self.next_foreign = self.first;
    }
    pub fn configure(&mut self, protocol: Protocol) {
        let limits = snapshot_limits(protocol);
        self.world_entity = limits.world_entity;
        if self.first != limits.first_entity as usize || self.end != limits.end_entity as usize {
            self.first = limits.first_entity as usize;
            self.end = limits.end_entity as usize;
            self.reset();
        }
    }
    /// Reserve every live native claim before assigning foreign emitters, just
    /// as snapshot preparation reserves hidden native entities before rows.
    pub fn refresh(
        &mut self,
        entities: &EntityTable,
        module: Option<ModuleId>,
        protocol: Protocol,
        players: impl IntoIterator<Item = EntityId>,
    ) {
        self.begin(entities, module, protocol);
        for slot in 0..entities.reserved_slots() {
            if let Some(id) = entities.id_at(slot)
                && entities.client_index(id).is_some()
            {
                let row = &mut self.slots[slot];
                if !matches!(row.role, MappingRole::RetiringPlayer) {
                    row.id = Some(id);
                    row.role = MappingRole::InactivePlayer;
                }
            }
        }
        for id in players {
            let Some(slot) = entities
                .resolve(id)
                .filter(|_| entities.client_index(id).is_some())
            else {
                continue;
            };
            if self.slots[slot].id != Some(id) {
                self.release(slot);
                self.slots[slot].id = Some(id);
            }
            self.slots[slot].role = MappingRole::Player;
        }
        for slot in 0..entities.reserved_slots() {
            if matches!(self.slots[slot].role, MappingRole::InactivePlayer) {
                if let Some(id) = self.slots[slot].id {
                    self.retain_player(id);
                }
            } else if matches!(self.slots[slot].role, MappingRole::Player)
                && (self
                    .self_claim
                    .is_some_and(|(id, _)| self.slots[slot].id == Some(id))
                    || entities.columns.native_entity[slot]
                        .is_some_and(|native| Some(native.module) == module))
            {
                self.unmap(slot);
            }
        }
        if let Some((id, number)) = self.self_claim
            && entities.resolve(id).is_some()
            && (entities.client_index(id).is_none()
                || matches!(self.slots[id.slot as usize].role, MappingRole::Player))
        {
            self.bind(id, number);
        }
        for slot in 0..entities.reserved_slots() {
            let row = self.slots[slot];
            if let Some(id) = row.id
                && matches!(row.role, MappingRole::Player)
                && !self.self_claim.is_some_and(|(owner, _)| owner == id)
                && let Some(native) = entities.columns.native_entity[slot]
                    .filter(|native| Some(native.module) == module)
                && let Some(number) = self.primary_player_number(native.slot)
            {
                self.bind(id, number);
            }
        }
        for id in entities.active() {
            if self.self_claim.is_some_and(|(owner, _)| owner == id)
                || entities.client_index(id).is_some()
            {
                continue;
            }
            if let Some(native) = entities.columns.native_entity[id.slot as usize]
                .filter(|native| Some(native.module) == module)
                && let Ok(number) = u32::try_from(native.slot)
                && (number != 0 || self.source_entity_first == 0)
            {
                if self.rebase_ordinary(number) {
                    if self.number(id) == Some(number) {
                        self.unmap(id.slot as usize);
                    }
                } else {
                    self.bind(id, number);
                }
            }
        }
        let player_end = self.player_end;
        // A larger native player range may cover yesterday's ordinary foreign
        // numbers. Those entities move through the same map on their next use;
        // exact primary claims and self keep priority over player allocation.
        for number in self.first..player_end {
            if let Some(id) = self.owners[number]
                && matches!(self.slots[id.slot as usize].role, MappingRole::Entity)
                && !self.self_claim.is_some_and(|(owner, _)| owner == id)
                && !entities.columns.native_entity[id.slot as usize].is_some_and(|native| {
                    Some(native.module) == module
                        && u32::try_from(native.slot).ok() == Some(number as u32)
                        && !self.rebase_ordinary(number as u32)
                })
            {
                self.unmap(id.slot as usize);
            }
        }
        for slot in 0..entities.reserved_slots() {
            let row = self.slots[slot];
            let Some(id) = row.id.filter(|_| matches!(row.role, MappingRole::Player)) else {
                continue;
            };
            // Exact primary and explicit self claims never fall back to another
            // ordinal, including when the declared player range cannot hold it.
            if self.self_claim.is_some_and(|(owner, _)| owner == id)
                || entities.columns.native_entity[slot]
                    .is_some_and(|native| Some(native.module) == module)
            {
                continue;
            }
            if row.number.is_some_and(|number| {
                (self.first..player_end).contains(&(number as usize))
                    && self.owners[number as usize] == Some(id)
            }) {
                continue;
            }
            self.unmap(slot);
            if let Some(number) = (self.first..player_end).find(|&n| self.owners[n].is_none()) {
                self.occupy(id, number as u32);
            }
        }
        // Relocated primary actors share the ordinary foreign allocator after
        // every exact primary claim and every admitted player is reserved.
        for id in entities.active() {
            if entities.client_index(id).is_none()
                && !self.self_claim.is_some_and(|(owner, _)| owner == id)
                && let Some(native) = entities.columns.native_entity[id.slot as usize]
                    .filter(|native| Some(native.module) == module)
                && let Ok(number) = u32::try_from(native.slot)
                && (number != 0 || self.source_entity_first == 0)
                && (self.first..self.end).contains(&(number as usize))
                && self.rebase_ordinary(number)
            {
                self.foreign(id);
            }
        }
    }

    fn primary_player_number(&self, slot: i32) -> Option<u32> {
        let client = u32::try_from(slot)
            .ok()?
            .checked_sub(self.source_entity_first)?;
        (client < self.native_clients && client < self.player_count())
            .then(|| self.first as u32 + client)
    }

    fn rebase_ordinary(&self, number: u32) -> bool {
        self.source_entity_first != self.first as u32
            && (self.first..self.player_end).contains(&(number as usize))
            || self
                .owners
                .get(number as usize)
                .copied()
                .flatten()
                .is_some_and(|id| matches!(self.slots[id.slot as usize].role, MappingRole::Player))
    }
    fn unmap(&mut self, slot: usize) {
        let previous = self.slots[slot];
        if let Some(number) = previous.number
            && self.owners[number as usize] == previous.id
        {
            self.owners[number as usize] = None;
        }
        self.slots[slot].number = None;
    }
    fn release(&mut self, slot: usize) {
        self.unmap(slot);
        self.slots[slot] = Mapping::default();
    }

    pub fn begin(&mut self, entities: &EntityTable, module: Option<ModuleId>, protocol: Protocol) {
        self.configure(protocol);
        if self.module != module {
            self.peer_seeded = false;
            self.slots.fill(Mapping::default());
            self.owners.fill(None);
            self.module = module;
        } else {
            for slot in 0..self.slots.len() {
                if !matches!(self.slots[slot].role, MappingRole::RetiringPlayer)
                    && self.slots[slot]
                        .id
                        .is_some_and(|id| entities.resolve(id).is_none())
                {
                    self.release(slot);
                }
            }
        }
        self.reserved.begin();
        self.next_foreign = self.first;
        // Vacant player ordinals remain reserved; a later connection must not
        // inherit a foreign entity's number. QW reserves all 32 player slots.
        let clients = if protocol == Protocol::QuakeWorld28 {
            states::QW_PLAYER_SLOTS
        } else {
            self.native_clients as usize
        };
        let end = (self.first + clients.min(snapshot_limits(protocol).client_capacity as usize))
            .min(self.end);
        self.player_end = end;
        for number in self.first..end {
            self.reserved.mark(number);
        }
        if let Some((id, number)) = self.self_claim
            && entities.resolve(id).is_some()
        {
            self.bind(id, number);
        }
    }

    fn occupy(&mut self, id: EntityId, number: u32) -> bool {
        let number_index = number as usize;
        if id.slot as usize >= self.slots.len()
            || number_index < self.first
            || number_index >= self.end
        {
            return false;
        }
        let role = if self.slots[id.slot as usize].id == Some(id) {
            self.slots[id.slot as usize].role
        } else {
            MappingRole::Entity
        };
        if let Some(previous) = self.owners[number_index]
            && previous != id
        {
            if matches!(
                self.slots[previous.slot as usize].role,
                MappingRole::RetiringPlayer
            ) {
                return false;
            }
            self.unmap(previous.slot as usize);
        }
        self.release(id.slot as usize);
        self.owners[number_index] = Some(id);
        self.slots[id.slot as usize] = Mapping {
            id: Some(id),
            number: Some(number),
            role,
        };
        true
    }

    pub fn bind(&mut self, id: EntityId, number: u32) -> bool {
        if id.slot as usize >= self.slots.len() {
            return false;
        }
        if self.slots.get(id.slot as usize).is_some_and(|row| {
            row.id == Some(id) && matches!(row.role, MappingRole::RetiringPlayer)
        }) {
            return false;
        }
        if self.slots.get(id.slot as usize).is_some_and(|row| {
            row.id == Some(id)
                && matches!(row.role, MappingRole::Player | MappingRole::InactivePlayer)
                && (!matches!(row.role, MappingRole::Player)
                    || !(self.first..self.player_end).contains(&(number as usize)))
        }) || self.self_claim.is_some_and(|(owner, own_number)| {
            owner != id
                && own_number == number
                && self.owners.get(number as usize) == Some(&Some(owner))
        }) || self
            .owners
            .get(number as usize)
            .copied()
            .flatten()
            .is_some_and(|owner| {
                owner != id
                    && matches!(self.slots[owner.slot as usize].role, MappingRole::Player)
                    && !matches!(self.slots[id.slot as usize].role, MappingRole::Player)
            })
        {
            self.unmap(id.slot as usize);
            return false;
        }
        if self.occupy(id, number) {
            self.reserved.mark(number as usize);
            true
        } else {
            false
        }
    }
    pub fn number(&self, id: EntityId) -> Option<u32> {
        let row = self.slots.get(id.slot as usize)?;
        (row.id == Some(id)).then_some(row.number).flatten()
    }

    /// One admitted player in this connection's native client namespace.
    pub fn client_reference(
        &self,
        entities: &EntityTable,
        id: EntityId,
        receiver: ClientId,
    ) -> Option<NativeClientRef> {
        entities.resolve(id)?;
        self.player_reference(id, receiver, MappingRole::Player)
    }

    pub(crate) fn peer_reference(
        &self,
        entities: &EntityTable,
        id: EntityId,
        receiver: ClientId,
    ) -> Option<NativeClientRef> {
        if self.slots.get(id.slot as usize).is_some_and(|row| {
            row.id == Some(id) && matches!(row.role, MappingRole::RetiringPlayer)
        }) {
            self.player_reference(id, receiver, MappingRole::RetiringPlayer)
        } else {
            self.client_reference(entities, id, receiver)
        }
    }

    fn player_reference(
        &self,
        id: EntityId,
        receiver: ClientId,
        role: MappingRole,
    ) -> Option<NativeClientRef> {
        let row = self.slots.get(id.slot as usize)?;
        if row.id != Some(id) || row.role != role {
            return None;
        }
        let number = row.number?;
        if self.owners.get(number as usize) != Some(&Some(id)) {
            return None;
        }
        let client = number.checked_sub(self.first as u32)?;
        (client < self.player_count())
            .then(|| NativeClientRef::client(ResourceNamespace::Client(receiver), client))
    }

    /// Keep a departing player's ordinal until its tombstone belongs to the
    /// output ring. Its entity generation may reset before the next output pass.
    pub fn retain_player(&mut self, id: EntityId) {
        if let Some(row) = self.slots.get_mut(id.slot as usize)
            && row.id == Some(id)
            && row.number.is_some()
            && matches!(row.role, MappingRole::Player | MappingRole::InactivePlayer)
        {
            row.role = MappingRole::RetiringPlayer;
        }
    }

    pub fn release_retiring_player(&mut self, id: EntityId) {
        if self.slots.get(id.slot as usize).is_some_and(|row| {
            row.id == Some(id) && matches!(row.role, MappingRole::RetiringPlayer)
        }) {
            self.release(id.slot as usize);
        }
    }

    pub fn foreign(&mut self, id: EntityId) -> Option<u32> {
        if let Some(row) = self.slots.get(id.slot as usize)
            && row.id == Some(id)
            && !matches!(row.role, MappingRole::Entity)
        {
            return matches!(row.role, MappingRole::Player)
                .then_some(row.number)
                .flatten();
        }
        if let Some(number) = self.number(id)
            && !self.reserved.contains(number as usize)
        {
            return Some(number);
        }
        while self.next_foreign < self.end
            && (self.owners[self.next_foreign].is_some()
                || self.reserved.contains(self.next_foreign))
        {
            self.next_foreign += 1;
        }
        if self.next_foreign == self.end {
            return None;
        }
        let number = self.next_foreign;
        self.next_foreign += 1;
        if self.occupy(id, number as u32) {
            // The binding is occupied, but foreign numbers are not native
            // reservations and may be retained on the following frame.
            Some(number as u32)
        } else {
            None
        }
    }

    pub fn project(&mut self, entities: &EntityTable, id: EntityId) -> Option<u32> {
        let slot = entities.resolve(id)?;
        if entities.client_index(id).is_some()
            && (self.slots[slot].id != Some(id)
                || !matches!(self.slots[slot].role, MappingRole::Player))
        {
            return None;
        }
        if let Some((owner, number)) = self.self_claim
            && owner == id
        {
            return self.bind(id, number).then_some(number);
        }
        if let Some(native) =
            entities.columns.native_entity[slot].filter(|native| Some(native.module) == self.module)
        {
            if entities.client_index(id).is_some() {
                let number = self.primary_player_number(native.slot)?;
                return self.bind(id, number).then_some(number);
            }
            let number = u32::try_from(native.slot).ok()?;
            if number == 0 && self.source_entity_first != 0 {
                return Some(self.world_entity);
            }
            if !(self.first..self.end).contains(&(number as usize)) {
                return None;
            }
            if self.rebase_ordinary(number) {
                return self.foreign(id);
            }
            return self.bind(id, number).then_some(number);
        }
        self.foreign(id)
    }
    pub fn native(&mut self, entities: &EntityTable, module: ModuleId, slot: i32) -> Option<u32> {
        if Some(module) == self.module {
            let number = u32::try_from(slot).ok()?;
            if number == 0 && self.source_entity_first != 0 {
                return Some(self.world_entity);
            }
        }
        let id = entities.find_native(qa_core::primitives::NativeEntity { module, slot }, None)?;
        self.project(entities, id)
    }
    /// Weak native references preserve their published ordinal in the primary
    /// namespace. Foreign entities need a real lifetime to rebase; player-config
    /// references need an actual client binding, not an entity-number mapping.
    pub fn reference(
        &mut self,
        entities: &EntityTable,
        receiver: ClientId,
        reference: NativeReference,
    ) -> Option<u32> {
        match reference.namespace() {
            ResourceNamespace::Client(client) => {
                (client == receiver).then_some(reference.ordinal())
            }
            ResourceNamespace::Module(module) if Some(module) == self.module => match reference {
                NativeReference::Client(_) => Some(reference.ordinal()),
                NativeReference::Entity(value) => {
                    if value.ordinal == 0 && self.source_entity_first != 0 {
                        return Some(self.world_entity);
                    }
                    let native = qa_core::primitives::NativeEntity {
                        module,
                        slot: i32::try_from(value.ordinal).ok()?,
                    };
                    if let Some(id) = entities.find_native(native, None) {
                        return self.project(entities, id);
                    }
                    if let Some(client) = value.ordinal.checked_sub(self.source_entity_first)
                        && client < self.native_clients
                    {
                        return (client < self.player_count()).then(|| self.first as u32 + client);
                    }
                    (!self.rebase_ordinary(value.ordinal)).then_some(value.ordinal)
                }
            },
            ResourceNamespace::Module(module) => match reference {
                NativeReference::Entity(value) => {
                    self.native(entities, module, i32::try_from(value.ordinal).ok()?)
                }
                NativeReference::Client(_) => None,
            },
        }
    }
    pub fn entity(&self, number: u32) -> Option<EntityId> {
        self.owners.get(number as usize).copied().flatten()
    }
}
