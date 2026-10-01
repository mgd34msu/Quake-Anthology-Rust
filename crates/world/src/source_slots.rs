//! Source actor slots: allocation order from Quake `pr_edict.c`, Quake II
//! `g_utils.c`, Quake III `g_utils.c`.
//!
//! Donor provenance: `/home/buzzkill/Projects/quake-typescript/src/world/actors/source-slots.ts`.
//!
//! Raw pointers use [`SourceActorSlots::at`] on each access. Only built-in
//! host references carry generations.

use std::cell::RefCell;
use std::rc::Rc;

use qa_core::identity::{OwnedActor, ProviderId};
use qa_core::time::SourceTime;

use crate::registry::ActorRegistry;
use crate::WorldError;

/// Live view of one source slot.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SourceSlotState {
    /// Whether the slot is free.
    pub free: bool,
    /// Time the slot was freed.
    pub freed_at: SourceTime,
}

/// Slot reuse policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotReuse {
    /// Reuse immediately.
    Immediate,
    /// Reuse after the source cooldown.
    SourceCooldown,
}

/// Slot clearing policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotClear {
    /// Source edict clear (Q1/Q2).
    SourceEdict,
    /// Source gentity clear (Q3).
    SourceGentity,
}

/// Table exhaustion policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotExhaustion {
    /// Fail when the table is full.
    Fatal,
    /// QW unlinks and overwrites the final edict when full.
    QwLastSlot,
}

/// Source-owned slot table behind the registry.
pub trait SourceSlotStorage {
    /// High-water slot count.
    fn count(&self) -> u32;
    /// Set the high-water slot count.
    fn set_count(&mut self, count: u32);
    /// Read a slot state.
    fn read(&self, slot: u32) -> SourceSlotState;
    /// Initialize a slot for an actor.
    fn initialize(&mut self, slot: u32, actor: &OwnedActor);
    /// Mark a slot freed at `now`.
    fn clear_freed(&mut self, slot: u32, now: SourceTime);
    /// Whether a slot may be freed. Q2 reserved corpse/client slots and Q3
    /// never-free entities return false after unlinking.
    fn can_free(&self, slot: u32) -> bool;
}

/// Actor lifetime policy (donor `ActorLifetimePolicy`).
pub struct ActorLifetimePolicy {
    /// First dynamically allocated slot.
    pub first_dynamic_slot: u32,
    /// Reuse policy.
    pub reuse: SlotReuse,
    /// Cooldown check.
    pub reusable_after: Box<dyn Fn(SourceTime, SourceTime) -> Result<bool, WorldError>>,
    /// Clear policy.
    pub clear: SlotClear,
    /// Exhaustion policy.
    pub exhaustion: SlotExhaustion,
}

/// Q1/Q2 classic edict lifetime.
#[must_use]
pub fn quake_edict_lifetime(first_dynamic_slot: u32, exhaustion: SlotExhaustion) -> ActorLifetimePolicy {
    ActorLifetimePolicy {
        first_dynamic_slot,
        reuse: SlotReuse::SourceCooldown,
        reusable_after: Box::new(|freed_at, now| {
            let (SourceTime::Seconds(freed), SourceTime::Seconds(current)) = (freed_at, now) else {
                return Err(WorldError::BadSourceSlots(
                    "Q1/Q2 classic edict lifetime uses seconds".to_string(),
                ));
            };
            Ok(f64::from(freed) < 2.0 || f64::from(current) - f64::from(freed) > 0.5)
        }),
        clear: SlotClear::SourceEdict,
        exhaustion,
    }
}

/// Q3 entity lifetime.
#[must_use]
pub fn q3_entity_lifetime(first_dynamic_slot: u32, map_start_milliseconds: f64) -> ActorLifetimePolicy {
    ActorLifetimePolicy {
        first_dynamic_slot,
        reuse: SlotReuse::SourceCooldown,
        reusable_after: Box::new(move |freed_at, now| {
            let (SourceTime::Milliseconds(freed), SourceTime::Milliseconds(current)) = (freed_at, now) else {
                return Err(WorldError::BadSourceSlots(
                    "Q3 entity lifetime uses milliseconds".to_string(),
                ));
            };
            // Donor `| 0` truncates toward zero; values here are far from the
            // wrapping range so plain truncation matches.
            let threshold = (map_start_milliseconds + 2000.0).trunc() as i64;
            let held = f64::from(current - freed).trunc() as i64;
            Ok(!(i64::from(freed) > threshold && held < 1000))
        }),
        clear: SlotClear::SourceGentity,
        exhaustion: SlotExhaustion::Fatal,
    }
}

/// Source slot table options.
pub struct SourceActorSlotsOptions {
    /// Owning provider.
    pub provider: ProviderId,
    /// Table capacity.
    pub capacity: u32,
    /// Lifetime policy.
    pub lifetime: ActorLifetimePolicy,
    /// Slot storage.
    pub storage: Box<dyn SourceSlotStorage>,
    /// Clock.
    pub now: Box<dyn Fn() -> SourceTime>,
    /// Unlink hook.
    pub unlink: Box<dyn Fn(&OwnedActor)>,
    /// Exhaustion hook (runs before the fatal error).
    pub exhausted: Box<dyn Fn(u32)>,
}

/// Source-owned slot table over a session registry.
pub struct SourceActorSlots {
    actors: Rc<RefCell<ActorRegistry>>,
    options: SourceActorSlotsOptions,
}

impl SourceActorSlots {
    /// Open a table; validates the slot range and high-water count.
    pub fn new(actors: Rc<RefCell<ActorRegistry>>, options: SourceActorSlotsOptions) -> Result<Self, WorldError> {
        if options.capacity <= options.lifetime.first_dynamic_slot {
            return Err(WorldError::BadSourceSlots(
                "Invalid source actor slot range".to_string(),
            ));
        }
        let slots = Self { actors, options };
        slots.count()?;
        Ok(slots)
    }

    /// Table options.
    #[must_use]
    pub fn options(&self) -> &SourceActorSlotsOptions {
        &self.options
    }

    /// Actor bound at a source slot, if any.
    #[must_use]
    pub fn at(&self, slot: u32) -> Option<OwnedActor> {
        self.actors.borrow().at_source(&self.options.provider, slot)
    }

    /// Bind a map/world/client slot the source host already initialized.
    pub fn bind_existing(&self, slot: u32, definition: &str) -> Result<OwnedActor, WorldError> {
        if slot >= self.count()? {
            return Err(WorldError::BadSourceSlots(
                "Source slot is outside the opened table".to_string(),
            ));
        }
        if let Some(existing) = self.at(slot) {
            return Ok(existing);
        }
        self.actors
            .borrow_mut()
            .allocate_at_source(self.options.provider.clone(), slot, definition)
    }

    /// Allocate a dynamic slot, reusing cooled-down slots first.
    pub fn allocate(&mut self, definition: &str) -> Result<OwnedActor, WorldError> {
        let now = (self.options.now)();
        let count = self.count()?;
        let mut selected = count;
        for slot in self.options.lifetime.first_dynamic_slot..count {
            let state = self.options.storage.read(slot);
            if state.free
                && (self.options.lifetime.reuse == SlotReuse::Immediate
                    || (self.options.lifetime.reusable_after)(state.freed_at, now)?)
            {
                selected = slot;
                break;
            }
        }
        if selected == self.options.capacity {
            (self.options.exhausted)(selected);
            if self.options.lifetime.exhaustion == SlotExhaustion::Fatal {
                return Err(WorldError::BadSourceSlots(format!(
                    "{}:{}: no free source actors",
                    self.options.provider.namespace, self.options.provider.name
                )));
            }
            selected -= 1;
            // QW deliberately unlinks and overwrites the final edict when its table is full.
            if let Some(previous) = self.at(selected) {
                (self.options.unlink)(&previous);
                self.actors.borrow_mut().release(&previous)?;
            }
        } else if selected == count {
            self.options.storage.set_count(count + 1);
        }
        if let Some(previous) = self.at(selected) {
            self.actors.borrow_mut().release(&previous)?;
        }
        let actor = self
            .actors
            .borrow_mut()
            .allocate_at_source(self.options.provider.clone(), selected, definition)?;
        self.options.storage.initialize(selected, &actor);
        Ok(actor)
    }

    /// Unlink and free an actor; reserved slots unlink but stay bound.
    pub fn free(&mut self, actor: &OwnedActor) -> Result<bool, WorldError> {
        self.actors.borrow().assert_owned(actor)?;
        let source = self.actors.borrow().source_of(actor.id());
        let Some((provider, slot)) = source else {
            return Err(WorldError::BadSourceSlots(
                "Actor is outside this source table".to_string(),
            ));
        };
        if provider != self.options.provider {
            return Err(WorldError::BadSourceSlots(
                "Actor is outside this source table".to_string(),
            ));
        }
        (self.options.unlink)(actor);
        if !self.options.storage.can_free(slot) {
            return Ok(false);
        }
        let now = (self.options.now)();
        self.options.storage.clear_freed(slot, now);
        self.actors.borrow_mut().release(actor)?;
        Ok(true)
    }

    fn count(&self) -> Result<u32, WorldError> {
        let count = self.options.storage.count();
        if count < self.options.lifetime.first_dynamic_slot || count > self.options.capacity {
            return Err(WorldError::BadSourceSlots(
                "Invalid source actor high-water count".to_string(),
            ));
        }
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;

    struct Table {
        count: u32,
        free: Vec<bool>,
        freed_at: Vec<SourceTime>,
        initialized: Vec<u32>,
        reserved: Vec<u32>,
    }

    impl SourceSlotStorage for Table {
        fn count(&self) -> u32 {
            self.count
        }
        fn set_count(&mut self, count: u32) {
            self.count = count;
        }
        fn read(&self, slot: u32) -> SourceSlotState {
            SourceSlotState {
                free: self.free.get(slot as usize).copied().unwrap_or(true),
                freed_at: self
                    .freed_at
                    .get(slot as usize)
                    .copied()
                    .unwrap_or(SourceTime::Seconds(0.0)),
            }
        }
        fn initialize(&mut self, slot: u32, _actor: &OwnedActor) {
            while self.free.len() <= slot as usize {
                self.free.push(true);
                self.freed_at.push(SourceTime::Seconds(0.0));
            }
            self.free[slot as usize] = false;
            self.initialized.push(slot);
        }
        fn clear_freed(&mut self, slot: u32, now: SourceTime) {
            while self.free.len() <= slot as usize {
                self.free.push(true);
                self.freed_at.push(SourceTime::Seconds(0.0));
            }
            self.free[slot as usize] = true;
            self.freed_at[slot as usize] = now;
        }
        fn can_free(&self, slot: u32) -> bool {
            !self.reserved.contains(&slot)
        }
    }

    fn slots(
        lifetime: ActorLifetimePolicy,
        capacity: u32,
        now: SourceTime,
    ) -> (SourceActorSlots, Rc<RefCell<Vec<u32>>>) {
        let actors = Rc::new(RefCell::new(
            ActorRegistry::new(IdentityOwner::create("slots-test").unwrap(), 16).unwrap(),
        ));
        let exhausted = Rc::new(RefCell::new(Vec::new()));
        let exhausted_hook = Rc::clone(&exhausted);
        let table = SourceActorSlots::new(
            actors,
            SourceActorSlotsOptions {
                provider: ProviderId::new("q1", "game"),
                capacity,
                lifetime,
                storage: Box::new(Table {
                    count: 1,
                    free: vec![false],
                    freed_at: vec![SourceTime::Seconds(0.0)],
                    initialized: Vec::new(),
                    reserved: Vec::new(),
                }),
                now: Box::new(move || now),
                unlink: Box::new(|_| {}),
                exhausted: Box::new(move |slot| exhausted_hook.borrow_mut().push(slot)),
            },
        )
        .unwrap();
        (table, exhausted)
    }

    #[test]
    fn allocate_grows_and_reuses_slots() {
        let (mut table, _) = slots(
            quake_edict_lifetime(1, SlotExhaustion::Fatal),
            4,
            SourceTime::Seconds(10.0),
        );
        let first = table.bind_existing(0, "q1:world").unwrap();
        assert_eq!(first.id().slot(), 0);
        let a = table.allocate("q1:ogre").unwrap();
        let b = table.allocate("q1:fiend").unwrap();
        assert_ne!(a.id().slot(), b.id().slot());
        assert!(table.free(&a).unwrap());
        // The 0.5s cooldown keeps the just-freed slot closed, so allocation
        // grows the table instead of reusing it.
        let _c = table.allocate("q1:knight").unwrap();
        // Source slot 1 stays vacant: the table grew instead of reusing it.
        assert!(table.at(1).is_none());
    }

    #[test]
    fn fatal_exhaustion_runs_the_hook() {
        let (mut table, exhausted) = slots(
            quake_edict_lifetime(1, SlotExhaustion::Fatal),
            2,
            SourceTime::Seconds(10.0),
        );
        table.allocate("q1:a").unwrap();
        assert!(table.allocate("q1:b").is_err());
        assert_eq!(*exhausted.borrow(), vec![2]);
    }

    #[test]
    fn qw_last_slot_overwrites_final_edict() {
        let (mut table, _) = slots(
            quake_edict_lifetime(1, SlotExhaustion::QwLastSlot),
            2,
            SourceTime::Seconds(10.0),
        );
        let first = table.allocate("q1:a").unwrap();
        let second = table.allocate("q1:b").unwrap();
        assert_eq!(first.id().slot(), second.id().slot());
        assert_ne!(first.id().generation(), second.id().generation());
    }

    #[test]
    fn foreign_actors_stay_outside_the_table() {
        let (mut table, _) = slots(
            quake_edict_lifetime(1, SlotExhaustion::Fatal),
            4,
            SourceTime::Seconds(10.0),
        );
        let foreign_owner = IdentityOwner::create("foreign").unwrap();
        let foreign_id = foreign_owner.actor(0, 0);
        let foreign = foreign_owner
            .owned_actor(&foreign_id, ProviderId::new("q1", "other"))
            .unwrap();
        assert!(table.free(&foreign).is_err());
        assert!(table.bind_existing(9, "q1:x").is_err());
    }

    #[test]
    fn lifetimes_enforce_their_units() {
        let quake = quake_edict_lifetime(1, SlotExhaustion::Fatal);
        assert!((quake.reusable_after)(SourceTime::Seconds(0.0), SourceTime::Seconds(10.0)).unwrap());
        assert!((quake.reusable_after)(SourceTime::Milliseconds(0), SourceTime::Seconds(10.0)).is_err());
        let q3 = q3_entity_lifetime(1, 1000.0);
        assert!((q3.reusable_after)(SourceTime::Milliseconds(0), SourceTime::Milliseconds(5000)).unwrap());
        assert!(!(q3.reusable_after)(SourceTime::Milliseconds(3500), SourceTime::Milliseconds(3600)).unwrap());
        assert!((q3.reusable_after)(SourceTime::Seconds(0.0), SourceTime::Milliseconds(0)).is_err());
    }
}
