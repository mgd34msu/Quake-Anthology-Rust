//! Inventory counts ported from `src/world/gameplay/inventory.ts`.
//! Capacity and item selection belong to the chosen inventory provider;
//! counts have one write path. Source-item leases and mod operations live
//! with the compat phase; this table owns plain per-actor stores.

use std::collections::HashMap;

use qa_core::identity::{ActorId, OwnedActor};
use qa_core::numeric::float_to_wrapped_i32;

use crate::combat::ItemId;
use crate::registry::ActorRegistry;
use crate::WorldError;

/// Source-counter arithmetic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CountArithmetic {
    /// Binary32 rounding.
    Binary32,
    /// Binary64 intermediates.
    Binary64,
    /// 32-bit integer wrap.
    Int32,
}

/// Count policy of one entry. Absence on a new entry selects a nonnegative
/// stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CountPolicy {
    /// Nonnegative stack.
    Stack,
    /// Signed source counter.
    SourceCounter(CountArithmetic),
}

/// One inventory entry.
#[derive(Debug, Clone, PartialEq)]
pub struct InventoryEntry {
    /// Item identifier.
    pub item: ItemId,
    /// Current count.
    pub count: f64,
    /// Capacity.
    pub capacity: f64,
    /// Count policy.
    pub count_policy: Option<CountPolicy>,
}

fn quantity(value: f64) -> Result<f64, WorldError> {
    if !value.is_finite() || value < 0.0 {
        return Err(WorldError::BadQuantity);
    }
    Ok(value)
}

fn source_count(entry: &InventoryEntry, value: f64) -> Result<f64, WorldError> {
    if !value.is_finite() {
        return Err(WorldError::BadCounter);
    }
    match entry.count_policy {
        None | Some(CountPolicy::Stack) => quantity(value),
        Some(CountPolicy::SourceCounter(CountArithmetic::Binary32)) => {
            let rounded = value as f32;
            if !rounded.is_finite() {
                return Err(WorldError::CounterRange);
            }
            Ok(f64::from(rounded))
        }
        Some(CountPolicy::SourceCounter(CountArithmetic::Binary64)) => Ok(value),
        Some(CountPolicy::SourceCounter(CountArithmetic::Int32)) => Ok(f64::from(float_to_wrapped_i32(value))),
    }
}

fn copy_entry(entry: &InventoryEntry) -> Result<InventoryEntry, WorldError> {
    quantity(entry.capacity)?;
    Ok(InventoryEntry {
        item: entry.item.clone(),
        count: source_count(entry, entry.count)?,
        capacity: entry.capacity,
        count_policy: entry.count_policy,
    })
}

/// Give transition. Source rounding and writes hold even when the count
/// delta is zero.
#[derive(Debug, Clone, PartialEq)]
pub enum GiveTransition {
    /// Nothing given.
    Unchanged,
    /// Entry write with the count delta.
    Write {
        /// Updated entry.
        entry: InventoryEntry,
        /// Count delta.
        given: f64,
    },
}

/// Pure give transition for one entry.
pub fn inventory_give(entry: &InventoryEntry, count: f64) -> Result<GiveTransition, WorldError> {
    quantity(count)?;
    let given = count.min(0.0f64.max(entry.capacity - entry.count));
    if given == 0.0 {
        return Ok(GiveTransition::Unchanged);
    }
    let next = copy_entry(&InventoryEntry {
        count: entry.count + source_count(entry, given)?,
        ..entry.clone()
    })?;
    let delta = next.count - entry.count;
    Ok(GiveTransition::Write {
        entry: next,
        given: delta,
    })
}

/// Shared per-actor inventory table.
#[derive(Debug, Default)]
pub struct InventoryTable {
    stores: HashMap<ActorId, HashMap<ItemId, InventoryEntry>>,
}

impl InventoryTable {
    /// Empty table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a store with initial entries.
    pub fn create(
        &mut self,
        registry: &ActorRegistry,
        actor: &OwnedActor,
        entries: &[InventoryEntry],
    ) -> Result<(), WorldError> {
        registry.assert_owned(actor)?;
        if self.stores.contains_key(actor.id()) {
            return Err(WorldError::InventoryMissing);
        }
        let mut store = HashMap::new();
        for entry in entries {
            if store.contains_key(&entry.item) {
                return Err(WorldError::DuplicateItem(entry.item.clone()));
            }
            store.insert(entry.item.clone(), copy_entry(entry)?);
        }
        self.stores.insert(actor.id().clone(), store);
        Ok(())
    }

    /// Whether a live actor has a bound inventory (donor `has`).
    #[must_use]
    pub fn has(&self, registry: &ActorRegistry, actor: &ActorId) -> bool {
        registry.is_live(actor) && self.stores.contains_key(actor)
    }

    /// Entries of a live actor.
    #[must_use]
    /// Whether the actor has an inventory store (C11).
    pub fn contains(&self, registry: &ActorRegistry, actor: &ActorId) -> bool {
        registry
            .resolve_owned(actor)
            .map(|owned| self.stores.contains_key(owned.id()))
            .unwrap_or(false)
    }

    pub fn entries(&self, registry: &ActorRegistry, actor: &ActorId) -> Vec<InventoryEntry> {
        if !registry.is_live(actor) {
            return Vec::new();
        }
        self.stores
            .get(actor)
            .map_or(Vec::new(), |store| store.values().cloned().collect())
    }

    /// Count of one item; missing actors and entries read zero.
    #[must_use]
    pub fn count(&self, registry: &ActorRegistry, actor: &ActorId, item: &str) -> f64 {
        if !registry.is_live(actor) {
            return 0.0;
        }
        self.stores
            .get(actor)
            .and_then(|store| store.get(item))
            .and_then(|entry| source_count(entry, entry.count).ok())
            .unwrap_or(0.0)
    }

    /// Consume a count, failing when unavailable.
    pub fn consume(
        &mut self,
        registry: &ActorRegistry,
        actor: &OwnedActor,
        item: &str,
        count: f64,
    ) -> Result<bool, WorldError> {
        registry.assert_owned(actor)?;
        quantity(count)?;
        let Some(store) = self.stores.get_mut(actor.id()) else {
            return Ok(count == 0.0);
        };
        let Some(entry) = store.get(item) else {
            return Ok(count == 0.0);
        };
        if entry.count < count {
            return Ok(false);
        }
        let next = copy_entry(&InventoryEntry {
            count: entry.count - source_count(entry, count)?,
            ..entry.clone()
        })?;
        store.insert(item.to_string(), next);
        Ok(true)
    }

    /// Give a count clamped to capacity.
    pub fn give(
        &mut self,
        registry: &ActorRegistry,
        actor: &OwnedActor,
        item: &str,
        count: f64,
    ) -> Result<f64, WorldError> {
        registry.assert_owned(actor)?;
        quantity(count)?;
        let Some(store) = self.stores.get_mut(actor.id()) else {
            return Ok(0.0);
        };
        let Some(entry) = store.get(item).cloned() else {
            return Ok(0.0);
        };
        match inventory_give(&entry, count)? {
            GiveTransition::Unchanged => Ok(0.0),
            GiveTransition::Write { entry, given } => {
                store.insert(item.to_string(), entry);
                Ok(given)
            }
        }
    }

    /// Configure an entry directly. Source pickups can change capacity or
    /// retain an over-cap count without a forced generic clamp.
    pub fn configure(
        &mut self,
        registry: &ActorRegistry,
        actor: &OwnedActor,
        entry: InventoryEntry,
    ) -> Result<(), WorldError> {
        registry.assert_owned(actor)?;
        let store = self.stores.get_mut(actor.id()).ok_or(WorldError::InventoryMissing)?;
        let policy = entry
            .count_policy
            .or_else(|| store.get(&entry.item).and_then(|previous| previous.count_policy));
        let next = copy_entry(&InventoryEntry {
            count_policy: policy,
            ..entry.clone()
        })?;
        store.insert(entry.item.clone(), next);
        Ok(())
    }

    /// Adjust a signed source counter. Fixed bursts may decrement past zero;
    /// ordinary stack consumption keeps its availability check.
    pub fn adjust_source_counter(
        &mut self,
        registry: &ActorRegistry,
        actor: &OwnedActor,
        item: &str,
        delta: f64,
    ) -> Result<f64, WorldError> {
        registry.assert_owned(actor)?;
        let store = self.stores.get_mut(actor.id()).ok_or(WorldError::InventoryMissing)?;
        let entry = store.get(item).cloned().ok_or(WorldError::NotSourceCounter)?;
        if !matches!(entry.count_policy, Some(CountPolicy::SourceCounter(_))) {
            return Err(WorldError::NotSourceCounter);
        }
        let next = copy_entry(&InventoryEntry {
            count: source_count(&entry, entry.count)? + source_count(&entry, delta)?,
            ..entry
        })?;
        let count = next.count;
        store.insert(item.to_string(), next);
        Ok(count)
    }

    /// Drop a released actor.
    pub fn release_actor(&mut self, actor: &ActorId) {
        self.stores.remove(actor);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::{IdentityOwner, ProviderId};

    use crate::combat::item_id;

    fn setup() -> (ActorRegistry, InventoryTable, OwnedActor) {
        let owner = IdentityOwner::create("test").unwrap();
        let mut registry = ActorRegistry::new(owner, 8).unwrap();
        let actor = registry.allocate(ProviderId::new("q3", "game"), "q3:player").unwrap();
        (registry, InventoryTable::new(), actor)
    }

    fn entry(item: &str, count: f64, capacity: f64) -> InventoryEntry {
        InventoryEntry {
            item: item.to_string(),
            count,
            capacity,
            count_policy: None,
        }
    }

    #[test]
    fn give_clamps_to_capacity_and_consume_checks_availability() {
        let (registry, mut table, actor) = setup();
        table
            .create(&registry, &actor, &[entry("q3:shells", 5.0, 10.0)])
            .unwrap();
        assert_eq!(table.give(&registry, &actor, "q3:shells", 8.0).unwrap(), 5.0);
        assert_eq!(table.count(&registry, actor.id(), "q3:shells"), 10.0);
        assert!(!table.consume(&registry, &actor, "q3:shells", 11.0).unwrap());
        assert!(table.consume(&registry, &actor, "q3:shells", 4.0).unwrap());
        assert_eq!(table.count(&registry, actor.id(), "q3:shells"), 6.0);
    }

    #[test]
    fn source_counters_round_by_policy() {
        let (registry, mut table, actor) = setup();
        table
            .create(
                &registry,
                &actor,
                &[InventoryEntry {
                    item: item_id("q2", "bullets"),
                    count: 0.0,
                    capacity: 300.0,
                    count_policy: Some(CountPolicy::SourceCounter(CountArithmetic::Int32)),
                }],
            )
            .unwrap();
        assert_eq!(
            table
                .adjust_source_counter(&registry, &actor, "q2:bullets", 5.0)
                .unwrap(),
            5.0
        );
        assert_eq!(
            table
                .adjust_source_counter(&registry, &actor, "q2:bullets", -8.0)
                .unwrap(),
            -3.0
        );
    }

    #[test]
    fn configure_retains_over_cap_counts() {
        let (registry, mut table, actor) = setup();
        table
            .create(&registry, &actor, &[entry("q1:nails", 0.0, 200.0)])
            .unwrap();
        table
            .configure(&registry, &actor, entry("q1:nails", 250.0, 200.0))
            .unwrap();
        assert_eq!(table.count(&registry, actor.id(), "q1:nails"), 250.0);
    }
}
