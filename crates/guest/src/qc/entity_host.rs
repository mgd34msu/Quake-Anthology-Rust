//! `ED_ClearEdict` / `ED_Free` source-slot storage and the `spawn` / `remove`
//! actor builtins.
//!
//! Ported from donor `src/compat/qc/entity-host.ts`
//! (`createQcSourceSlotStorage`, `createQcActorBindings`).
//!
//! Local mirrors: [`SourceSlotStorage`] mirrors the donor
//! `SourceSlotStorage`/`SourceActorSlots` pair from
//! `src/world/actors/index.ts`; [`reference_for_slot`] and
//! [`slot_for_reference`] mirror the `QcEntityMemory`
//! reference/slot codec owned by the sibling `qc` memory worker. Entity
//! words live in [`FieldTable`](crate::fields::FieldTable).

use qa_core::identity::ActorId;
use qa_core::math::vec3;

use crate::error::GuestError;
use crate::fields::{FieldTable, FieldValue};

/// Fields zeroed by `ED_ClearEdict`.
const CLEARED_FIELDS: &[&str] = &["model", "takedamage", "modelindex", "colormap", "skin", "frame", "solid"];

/// Metadata prefix layout inside each source edict row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QcEdictMetadataLayout {
    /// Byte offset of the `free` flag.
    pub free_offset_bytes: usize,
    /// Byte offset of the free timestamp.
    pub free_time_offset_bytes: usize,
}

/// Validate that metadata offsets sit inside the source prefix, exactly
/// like the donor constructor. Returns the validated layout.
pub fn create_qc_source_slot_storage(
    metadata: QcEdictMetadataLayout,
    variables_offset_bytes: usize,
    provider: &str,
    capacity: usize,
) -> Result<SourceSlotStorage, GuestError> {
    for offset in [metadata.free_offset_bytes, metadata.free_time_offset_bytes] {
        if offset + 4 > variables_offset_bytes || offset % 4 != 0 {
            return Err(GuestError::invalid("QC edict metadata must reside in the source prefix"));
        }
    }
    Ok(SourceSlotStorage::with_capacity(provider, capacity))
}

/// Encode a slot as a QC entity reference.
#[must_use]
pub const fn reference_for_slot(slot: usize) -> i32 {
    slot as i32
}

/// Decode a QC entity reference back to a slot.
pub fn slot_for_reference(reference: i32, count: usize) -> Result<usize, GuestError> {
    if reference < 0 {
        return Err(GuestError::invalid(format!("QC entity reference {reference} is negative")));
    }
    let slot = reference as usize;
    if slot >= count {
        return Err(GuestError::invalid(format!("QC entity reference {reference} is out of range")));
    }
    Ok(slot)
}

/// Free-state snapshot for one slot.
#[derive(Debug, Clone, PartialEq)]
pub struct SlotState {
    /// Whether the slot is free.
    pub free: bool,
    /// Free timestamp in seconds.
    pub freed_at_seconds: f32,
}

#[derive(Debug, Clone, PartialEq)]
struct SlotEntry {
    occupant: Option<ActorId>,
    freed_at_seconds: f32,
}

/// Source-slot storage with `ED_ClearEdict` clearing semantics.
#[derive(Debug, Clone)]
pub struct SourceSlotStorage {
    provider: String,
    capacity: usize,
    slots: Vec<SlotEntry>,
}

impl SourceSlotStorage {
    /// Empty storage with a fixed capacity.
    #[must_use]
    pub fn with_capacity(provider: &str, capacity: usize) -> Self {
        Self { provider: provider.to_string(), capacity, slots: Vec::new() }
    }

    /// Provider name identifying QC-owned source rows.
    #[must_use]
    pub fn provider(&self) -> &str {
        &self.provider
    }

    /// Maximum slot count.
    #[must_use]
    pub const fn capacity(&self) -> usize {
        self.capacity
    }

    /// Current row count.
    #[must_use]
    pub fn count(&self) -> usize {
        self.slots.len()
    }

    /// Grow or shrink the row count. Shrinking drops only free tail rows.
    pub fn set_count(&mut self, count: usize) -> Result<(), GuestError> {
        if count > self.capacity {
            return Err(GuestError::invalid(format!("QC row count {count} exceeds capacity {}", self.capacity)));
        }
        if count < self.slots.len() && self.slots[count..].iter().any(|slot| slot.occupant.is_some()) {
            return Err(GuestError::invalid("QC row shrink would drop a live actor"));
        }
        self.slots.resize(
            count,
            SlotEntry { occupant: None, freed_at_seconds: 0.0 },
        );
        Ok(())
    }

    /// Read one slot's free state.
    pub fn read(&self, slot: usize) -> Result<SlotState, GuestError> {
        let entry = self.entry(slot)?;
        Ok(SlotState { free: entry.occupant.is_none(), freed_at_seconds: entry.freed_at_seconds })
    }

    /// Actor occupying a slot, if any.
    #[must_use]
    pub fn at(&self, slot: usize) -> Option<&ActorId> {
        self.slots.get(slot).and_then(|entry| entry.occupant.as_ref())
    }

    /// Whether a slot is free.
    #[must_use]
    pub fn is_free(&self, slot: usize) -> bool {
        self.slots.get(slot).is_none_or(|entry| entry.occupant.is_none())
    }

    /// Allocate the first free row, growing the table when needed.
    pub fn allocate(&mut self, actor: ActorId) -> Result<usize, GuestError> {
        if let Some(slot) = self.slots.iter().position(|entry| entry.occupant.is_none()) {
            self.slots[slot].occupant = Some(actor);
            return Ok(slot);
        }
        if self.slots.len() >= self.capacity {
            return Err(GuestError::invalid("No free QC row for a dynamic actor"));
        }
        self.slots.push(SlotEntry { occupant: Some(actor), freed_at_seconds: 0.0 });
        Ok(self.slots.len() - 1)
    }

    /// Zero a row's words and mark it occupied.
    pub fn initialize(&mut self, slot: usize, fields: &mut FieldTable, actor: &ActorId) -> Result<(), GuestError> {
        let entry = self.entry_mut(slot)?;
        entry.occupant = Some(actor.clone());
        clear_words(fields, actor)
    }

    /// Free a row with `ED_ClearEdict` clearing semantics.
    pub fn clear_freed(
        &mut self,
        slot: usize,
        fields: &mut FieldTable,
        actor: &ActorId,
        now_seconds: f32,
    ) -> Result<(), GuestError> {
        if !now_seconds.is_finite() {
            return Err(GuestError::invalid("QC source free time must use finite seconds"));
        }
        let entry = self.entry_mut(slot)?;
        entry.occupant = None;
        entry.freed_at_seconds = now_seconds;
        clear_words(fields, actor)
    }

    /// Slots always support freeing, matching the donor `canFree`.
    #[must_use]
    pub const fn can_free(&self) -> bool {
        true
    }

    fn entry(&self, slot: usize) -> Result<&SlotEntry, GuestError> {
        self.slots
            .get(slot)
            .ok_or_else(|| GuestError::invalid(format!("QC slot {slot} is out of range")))
    }

    fn entry_mut(&mut self, slot: usize) -> Result<&mut SlotEntry, GuestError> {
        if slot >= self.slots.len() {
            return Err(GuestError::invalid(format!("QC slot {slot} is out of range")));
        }
        Ok(&mut self.slots[slot])
    }
}

fn clear_words(fields: &mut FieldTable, actor: &ActorId) -> Result<(), GuestError> {
    for name in CLEARED_FIELDS {
        let cleared = match fields.get(actor, name)?.clone() {
            FieldValue::Text(_) => FieldValue::Text(String::new()),
            FieldValue::Vector(_) => FieldValue::Vector(vec3(0.0, 0.0, 0.0)),
            FieldValue::Entity(_) => FieldValue::Entity(None),
            FieldValue::Int(_) => FieldValue::Int(0),
            FieldValue::Float(_) => FieldValue::Float(0.0),
        };
        fields.set(actor, name, cleared)?;
    }
    fields.set(actor, "origin", FieldValue::Vector(vec3(0.0, 0.0, 0.0)))?;
    fields.set(actor, "angles", FieldValue::Vector(vec3(0.0, 0.0, 0.0)))?;
    fields.set(actor, "nextthink", FieldValue::Float(-1.0))?;
    Ok(())
}

/// `spawn` builtin: allocate a dynamic row for a freshly minted actor and
/// return its entity reference.
pub fn qc_spawn(
    storage: &mut SourceSlotStorage,
    fields: &mut FieldTable,
    actor: ActorId,
) -> Result<i32, GuestError> {
    let slot = storage.allocate(actor.clone())?;
    storage.initialize(slot, fields, &actor)?;
    Ok(reference_for_slot(slot))
}

/// `remove` builtin: free the row addressed by a reference. Rows without a
/// current occupant are still cleared, matching the donor fallback.
pub fn qc_remove(
    storage: &mut SourceSlotStorage,
    fields: &mut FieldTable,
    reference: i32,
    now_seconds: f32,
) -> Result<(), GuestError> {
    let slot = slot_for_reference(reference, storage.count())?;
    match storage.at(slot).cloned() {
        Some(actor) => storage.clear_freed(slot, fields, &actor, now_seconds),
        None => {
            // Donor clears the raw words even without an occupant; without an
            // actor key there are no words to clear, so only the free state
            // is stamped.
            let entry = storage.entry_mut(slot)?;
            entry.freed_at_seconds = now_seconds;
            Ok(())
        }
    }
}

/// `isFreeEntity` predicate shared by the world and spatial hosts.
#[must_use]
pub fn is_free_entity(storage: &SourceSlotStorage, slot: usize) -> bool {
    storage.is_free(slot)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fields::FieldLayout;
    use qa_core::identity::IdentityOwner;

    const METADATA: QcEdictMetadataLayout = QcEdictMetadataLayout { free_offset_bytes: 0, free_time_offset_bytes: 4 };

    fn storage() -> SourceSlotStorage {
        create_qc_source_slot_storage(METADATA, 64, "quakec:edict", 8).unwrap()
    }

    #[test]
    fn metadata_must_live_in_source_prefix() {
        assert!(create_qc_source_slot_storage(METADATA, 64, "quakec:edict", 8).is_ok());
        let bad = QcEdictMetadataLayout { free_offset_bytes: 2, free_time_offset_bytes: 4 };
        assert!(create_qc_source_slot_storage(bad, 64, "quakec:edict", 8).is_err());
        let outside = QcEdictMetadataLayout { free_offset_bytes: 64, free_time_offset_bytes: 68 };
        assert!(create_qc_source_slot_storage(outside, 64, "quakec:edict", 8).is_err());
    }

    #[test]
    fn spawn_remove_round_trip_clears_like_ed_clear_edict() {
        let owner = IdentityOwner::create("entity-host").unwrap();
        let mut storage = storage();
        let mut fields = FieldTable::new();
        let actor = owner.actor(1, 1);
        fields.allocate(&actor, &FieldLayout::qc_entity()).unwrap();
        fields.set(&actor, "frame", FieldValue::Float(9.0)).unwrap();
        let reference = qc_spawn(&mut storage, &mut fields, actor.clone()).unwrap();
        assert_eq!(reference, 0);
        assert_eq!(storage.at(0), Some(&actor));
        assert!(!storage.read(0).unwrap().free);
        assert!(storage.can_free());
        assert_eq!(slot_for_reference(reference, storage.count()).unwrap(), 0);
        qc_remove(&mut storage, &mut fields, reference, 12.5).unwrap();
        assert!(is_free_entity(&storage, 0));
        let state = storage.read(0).unwrap();
        assert!(state.free);
        assert_eq!(state.freed_at_seconds, 12.5);
        assert_eq!(fields.get(&actor, "frame").unwrap(), &FieldValue::Float(0.0));
        assert_eq!(fields.get(&actor, "nextthink").unwrap(), &FieldValue::Float(-1.0));
        assert_eq!(
            fields.get(&actor, "origin").unwrap(),
            &FieldValue::Vector(vec3(0.0, 0.0, 0.0))
        );
    }

    #[test]
    fn remove_without_occupant_still_stamps_free_time() {
        let owner = IdentityOwner::create("entity-host").unwrap();
        let mut storage = storage();
        let mut fields = FieldTable::new();
        let actor = owner.actor(1, 1);
        fields.allocate(&actor, &FieldLayout::qc_entity()).unwrap();
        qc_spawn(&mut storage, &mut fields, actor.clone()).unwrap();
        qc_remove(&mut storage, &mut fields, 0, 3.0).unwrap();
        qc_remove(&mut storage, &mut fields, 0, 4.0).unwrap();
        assert_eq!(storage.read(0).unwrap().freed_at_seconds, 4.0);
    }

    #[test]
    fn capacity_and_reference_bounds_are_rejected() {
        let owner = IdentityOwner::create("entity-host").unwrap();
        let mut storage = create_qc_source_slot_storage(METADATA, 64, "quakec:edict", 1).unwrap();
        assert_eq!(storage.capacity(), 1);
        assert_eq!(storage.provider(), "quakec:edict");
        let mut fields = FieldTable::new();
        let first = owner.actor(1, 1);
        fields.allocate(&first, &FieldLayout::qc_entity()).unwrap();
        qc_spawn(&mut storage, &mut fields, first).unwrap();
        let second = owner.actor(2, 1);
        fields.allocate(&second, &FieldLayout::qc_entity()).unwrap();
        assert!(qc_spawn(&mut storage, &mut fields, second).is_err());
        assert!(slot_for_reference(-1, storage.count()).is_err());
        assert!(slot_for_reference(7, storage.count()).is_err());
        assert!(storage.set_count(2).is_err());
        assert!(storage.set_count(0).is_err());
    }

    #[test]
    fn freed_rows_are_reused_before_growing() {
        let owner = IdentityOwner::create("entity-host").unwrap();
        let mut storage = storage();
        let mut fields = FieldTable::new();
        let first = owner.actor(1, 1);
        let second = owner.actor(2, 1);
        fields.allocate(&first, &FieldLayout::qc_entity()).unwrap();
        fields.allocate(&second, &FieldLayout::qc_entity()).unwrap();
        qc_spawn(&mut storage, &mut fields, first.clone()).unwrap();
        qc_spawn(&mut storage, &mut fields, second.clone()).unwrap();
        assert_eq!(storage.count(), 2);
        qc_remove(&mut storage, &mut fields, 0, 1.0).unwrap();
        let third = owner.actor(3, 1);
        fields.allocate(&third, &FieldLayout::qc_entity()).unwrap();
        let reference = qc_spawn(&mut storage, &mut fields, third.clone()).unwrap();
        assert_eq!(reference, 0);
        assert_eq!(storage.count(), 2);
        assert_eq!(storage.at(0), Some(&third));
    }

    #[test]
    fn non_finite_free_time_is_rejected() {
        let owner = IdentityOwner::create("entity-host").unwrap();
        let mut storage = storage();
        let mut fields = FieldTable::new();
        let actor = owner.actor(1, 1);
        fields.allocate(&actor, &FieldLayout::qc_entity()).unwrap();
        qc_spawn(&mut storage, &mut fields, actor.clone()).unwrap();
        assert!(storage.clear_freed(0, &mut fields, &actor, f32::NAN).is_err());
    }
}
