//! One connection-owned entity-number projection shared by snapshots and cues.
use qa_core::{
    primitives::{EntityId, ModuleId},
    stamps::StampSet,
};
use qa_network::{commands::packet::Protocol, snapshots::snapshot_limits, states};
use qa_world::entities::{EntityTable, MAX_ENTITIES};

/// Protocol client ordinals differ from entity ordinals by the native world slot.
pub fn native_client_entity(protocol: Protocol, client: u32) -> Option<u32> {
    client.checked_add(snapshot_limits(protocol).first_entity)
}

#[derive(Clone, Copy, Default)]
struct Mapping {
    id: Option<EntityId>,
    number: Option<u32>,
}

/// Native ordinals and common lifetimes have separate namespaces. Foreign
/// lifetimes retain their number until release or a native ordinal claims it.
pub struct NativeNumbers {
    slots: Box<[Mapping]>,
    owners: Box<[Option<EntityId>]>,
    reserved: StampSet,
    module: Option<ModuleId>,
    native_clients: u32,
    self_claim: Option<(EntityId, u32)>,
    first: usize,
    next_foreign: usize,
    end: usize,
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
            self_claim: None,
            first: limits.first_entity as usize,
            next_foreign: limits.first_entity as usize,
            end: limits.end_entity as usize,
        }
    }

    pub fn native_clients(&self) -> u32 {
        self.native_clients
    }
    pub fn is_native_module(&self, module: ModuleId) -> bool {
        self.module == Some(module)
    }
    pub fn set_native_clients(&mut self, clients: u32) {
        self.native_clients = clients;
    }
    pub fn set_self(&mut self, entity: EntityId, native_client: u32, protocol: Protocol) {
        self.self_claim =
            native_client_entity(protocol, native_client).map(|number| (entity, number));
    }
    pub fn reset(&mut self) {
        self.slots.fill(Mapping::default());
        self.owners.fill(None);
        self.module = None;
        self.reserved.begin();
        self.next_foreign = self.first;
    }
    pub fn configure(&mut self, protocol: Protocol) {
        let limits = snapshot_limits(protocol);
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
    ) {
        self.begin(entities, module, protocol);
        for id in entities.active() {
            if let Some(native) = entities.columns.native_entity[id.slot as usize]
                .filter(|native| Some(native.module) == module)
                && let Ok(number) = u32::try_from(native.slot)
                && (number != 0 || protocol == Protocol::Quake3_68)
            {
                self.bind(id, number);
            }
        }
    }
    fn release(&mut self, slot: usize) {
        let previous = self.slots[slot];
        if let Some(number) = previous.number
            && self.owners[number as usize] == previous.id
        {
            self.owners[number as usize] = None;
        }
        self.slots[slot] = Mapping::default();
    }

    pub fn begin(&mut self, entities: &EntityTable, module: Option<ModuleId>, protocol: Protocol) {
        self.configure(protocol);
        if self.module != module {
            self.slots.fill(Mapping::default());
            self.owners.fill(None);
            self.module = module;
        } else {
            for slot in 0..self.slots.len() {
                if self.slots[slot]
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
        let end = (self.first + clients).min(self.end);
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
        if let Some(previous) = self.owners[number_index]
            && previous != id
        {
            self.release(previous.slot as usize);
        }
        self.release(id.slot as usize);
        self.owners[number_index] = Some(id);
        self.slots[id.slot as usize] = Mapping {
            id: Some(id),
            number: Some(number),
        };
        true
    }

    pub fn bind(&mut self, id: EntityId, number: u32) -> bool {
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

    pub fn foreign(&mut self, id: EntityId) -> Option<u32> {
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
        if let Some((owner, number)) = self.self_claim
            && owner == id
        {
            return self.bind(id, number).then_some(number);
        }
        if let Some(native) =
            entities.columns.native_entity[slot].filter(|native| Some(native.module) == self.module)
        {
            let number = u32::try_from(native.slot).ok()?;
            if number == 0 && self.first != 0 {
                return Some(0);
            }
            return self.bind(id, number).then_some(number);
        }
        self.foreign(id)
    }
    pub fn native(&mut self, entities: &EntityTable, module: ModuleId, slot: i32) -> Option<u32> {
        if Some(module) == self.module {
            let number = u32::try_from(slot).ok()?;
            if number == 0 && self.first != 0 {
                return Some(0);
            }
        }
        let id = entities.find_native(qa_core::primitives::NativeEntity { module, slot }, None)?;
        self.project(entities, id)
    }
    pub fn entity(&self, number: u32) -> Option<EntityId> {
        self.owners.get(number as usize).copied().flatten()
    }
}
