//! Mapped Team Arena ammo regeneration timers over the shared inventory.
//!
//! Port of Quake-Anthology-TS `src/app/bootstrap/simulation/arsenal/q3-ammo-regen.ts`
//! (`Q3MappedAmmoRegeneration`).

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use qa_content::contract::{ItemId, PickupOwner, PickupSupplyProfile};
use qa_content::q3::foundation::arsenal::Q3_WEAPON_ITEMS;
use qa_content::q3::team_arena::client_effects::{
    q3_ammo_regeneration_rule, Q3AmmoRegenerationRule, Q3MappedAmmoTimer,
};
use qa_core::identity::{ActorId, OwnedActor, SavedActorId};
use qa_world::inventory::InventoryTable;
use qa_world::registry::ActorRegistry;
use qa_world::save::records::{read_saved_actor, write_saved_actor};
use qa_world::save::value::{arr, int, namespaced, obj, str, SaveJson, SaveReader};
use qa_world::WorldError;

/// Errors in the mapped ammo regeneration owner.
#[derive(Debug, thiserror::Error)]
pub enum Q3AmmoRegenError {
    /// Periodic ammo pool has multiple owners.
    #[error("Periodic ammo pool has multiple owners")]
    DuplicatePool,
    /// Periodic ammo rule requires original source ammunition.
    #[error("Periodic ammo rule requires original source ammunition")]
    MissingSourceAmmo,
    /// Mapped ammo timer owner is closed.
    #[error("Mapped ammo timer owner is closed")]
    Closed,
    /// Mapped ammo pool is not admitted by the selected inventory.
    #[error("Mapped ammo pool is not admitted by the selected inventory")]
    NotAdmitted,
    /// Mapped ammo timer actor is retired.
    #[error("Mapped ammo timer actor is retired")]
    Retired,
    /// Wrapped world/save error.
    #[error(transparent)]
    World(#[from] WorldError),
}

/// One pool binding: destination item, original source ammo, regeneration rule.
#[derive(Debug, Clone)]
struct AmmoBinding {
    item: ItemId,
    source: ItemId,
    rule: Q3AmmoRegenerationRule,
}

/// Retained per-actor pool state. The content timer shape carries `Cell`
/// snapshots instead of the donor's live inventory accessors, so elapsed
/// counters live here and [`Q3MappedAmmoRegeneration::flush`] writes the
/// Team Arena call's result back after each call.
#[derive(Debug, Clone)]
struct PoolTimerState {
    item: ItemId,
    source: ItemId,
    rule: Q3AmmoRegenerationRule,
    elapsed_ms: i32,
}

/// Independent destination counters driven by the actual primary Team Arena
/// timer call.
pub struct Q3MappedAmmoRegeneration {
    profile: PickupSupplyProfile,
    actors: Rc<ActorRegistry>,
    inventory: Rc<RefCell<InventoryTable>>,
    legacy_owners: Vec<PickupOwner>,
    bindings: Vec<AmmoBinding>,
    entries: HashMap<ActorId, Vec<PoolTimerState>>,
    retired: HashMap<ActorId, Rc<Cell<bool>>>,
    closed: Rc<Cell<bool>>,
}

impl Q3MappedAmmoRegeneration {
    /// Create a mapped ammo regeneration owner.
    pub fn new(
        profile: PickupSupplyProfile,
        actors: Rc<ActorRegistry>,
        inventory: Rc<RefCell<InventoryTable>>,
        legacy_owners: Vec<PickupOwner>,
    ) -> Result<Self, Q3AmmoRegenError> {
        let mut items = HashSet::new();
        let mut bindings = Vec::with_capacity(profile.ammo_owners.len());
        for owner in &profile.ammo_owners {
            if !items.insert(owner.item.clone()) {
                return Err(Q3AmmoRegenError::DuplicatePool);
            }
            let source = Q3_WEAPON_ITEMS
                .iter()
                .find(|weapon| weapon.ammo.as_deref() == Some(owner.source.as_str()))
                .ok_or(Q3AmmoRegenError::MissingSourceAmmo)?;
            bindings.push(AmmoBinding {
                item: owner.item.clone(),
                source: owner.source.clone(),
                rule: q3_ammo_regeneration_rule(source.weapon as i32),
            });
        }
        Ok(Q3MappedAmmoRegeneration {
            profile,
            actors,
            inventory,
            legacy_owners,
            bindings,
            entries: HashMap::new(),
            retired: HashMap::new(),
            closed: Rc::new(Cell::new(false)),
        })
    }

    /// Selected supply profile.
    pub fn profile(&self) -> &PickupSupplyProfile {
        &self.profile
    }

    /// Drop retained timers after an actor release. The Rust registry has no
    /// release subscription; owners call this explicitly.
    pub fn on_actor_released(&mut self, actor: &ActorId) {
        if let Some(retired) = self.retired.remove(actor) {
            retired.set(true);
        }
        self.entries.remove(actor);
    }

    fn actor_entry(&self, actor: &OwnedActor, item: &ItemId) -> Result<f64, Q3AmmoRegenError> {
        let inventory = self.inventory.borrow();
        inventory
            .entries(&self.actors, actor.id())
            .into_iter()
            .find(|entry| entry.item == *item)
            .map(|entry| entry.count)
            .ok_or(Q3AmmoRegenError::NotAdmitted)
    }

    fn require(&mut self, actor: &OwnedActor) -> Result<(), Q3AmmoRegenError> {
        if self.closed.get() {
            return Err(Q3AmmoRegenError::Closed);
        }
        self.actors.assert_owned(actor)?;
        if self.entries.contains_key(actor.id()) {
            return Ok(());
        }
        for binding in &self.bindings {
            self.actor_entry(actor, &binding.item)?;
        }
        let timers = self
            .bindings
            .iter()
            .map(|binding| PoolTimerState {
                item: binding.item.clone(),
                source: binding.source.clone(),
                rule: binding.rule,
                elapsed_ms: 0,
            })
            .collect();
        self.retired.insert(actor.id().clone(), Rc::new(Cell::new(false)));
        self.entries.insert(actor.id().clone(), timers);
        Ok(())
    }

    /// Build the mapped timers for an actor, synced from the shared
    /// inventory. The caller runs the Team Arena timer call over the result
    /// and then calls [`Q3MappedAmmoRegeneration::flush`] to write the
    /// mutated counters back; the content timer shape cannot write through
    /// to the inventory on its own.
    pub fn timers(&mut self, actor: &OwnedActor) -> Result<Vec<Q3MappedAmmoTimer>, Q3AmmoRegenError> {
        self.require(actor)?;
        let retired = self.retired.get(actor.id()).cloned().unwrap_or_else(|| {
            let retired = Rc::new(Cell::new(false));
            self.retired.insert(actor.id().clone(), retired.clone());
            retired
        });
        let closed = self.closed.clone();
        let mut built = Vec::with_capacity(self.bindings.len());
        for timer in self.entries.get(actor.id()).cloned().unwrap_or_default() {
            let count = self.actor_entry(actor, &timer.item)?;
            let current_closed = closed.clone();
            let current_retired = retired.clone();
            built.push(Q3MappedAmmoTimer {
                rule: timer.rule,
                current: Rc::new(move || !current_closed.get() && !current_retired.get()),
                count: Cell::new(count as i32),
                elapsed_ms: Cell::new(timer.elapsed_ms),
            });
        }
        Ok(built)
    }

    /// Write a Team Arena timer call's mutated counters back to the retained
    /// entries and the shared inventory.
    pub fn flush(&mut self, actor: &OwnedActor, timers: &[Q3MappedAmmoTimer]) -> Result<(), Q3AmmoRegenError> {
        if self.closed.get() {
            return Err(Q3AmmoRegenError::Closed);
        }
        self.actors.assert_owned(actor)?;
        let retained = self.entries.get_mut(actor.id()).ok_or(Q3AmmoRegenError::Retired)?;
        for timer in timers {
            for state in retained
                .iter_mut()
                .filter(|state| state.rule.weapon == timer.rule.weapon)
            {
                state.elapsed_ms = timer.elapsed_ms.get();
                let mut inventory = self.inventory.borrow_mut();
                let mut entry = inventory
                    .entries(&self.actors, actor.id())
                    .into_iter()
                    .find(|entry| entry.item == state.item)
                    .ok_or(Q3AmmoRegenError::NotAdmitted)?;
                entry.count = f64::from(timer.count.get());
                inventory.configure(&self.actors, actor, entry)?;
            }
        }
        Ok(())
    }

    /// Overwrite the elapsed counter for a weapon's timers.
    pub fn stored(&mut self, actor: &ActorId, weapon: i32, value: i32) {
        let Some(owner) = self.actors.resolve_owned(actor) else {
            return;
        };
        let Some(timers) = self.entries.get_mut(owner.id()) else {
            return;
        };
        for timer in timers.iter_mut().filter(|timer| timer.rule.weapon == weapon) {
            timer.elapsed_ms = value;
        }
    }

    /// Capture the mapped timers for a save.
    pub fn capture(&mut self, actors: &[OwnedActor]) -> Result<SaveJson, Q3AmmoRegenError> {
        let mut captured = Vec::with_capacity(actors.len());
        for actor in actors {
            self.require(actor)?;
            let timers = self.entries.get(actor.id()).cloned().unwrap_or_default();
            captured.push(obj(vec![
                ("actor", write_saved_actor(SavedActorId::from(actor.id()))),
                (
                    "timers",
                    arr(timers
                        .into_iter()
                        .map(|timer| {
                            obj(vec![
                                ("item", str(&timer.item)),
                                ("source", str(&timer.source)),
                                ("elapsedMilliseconds", int(i64::from(timer.elapsed_ms))),
                            ])
                        })
                        .collect()),
                ),
            ]));
        }
        Ok(obj(vec![("profile", str(&self.profile.id)), ("actors", arr(captured))]))
    }

    /// Restore mapped timers from a save. A missing value imports legacy
    /// elapsed counters through `legacy`.
    pub fn restore(
        &mut self,
        reader: SaveReader,
        actors: &[OwnedActor],
        resolve: &dyn Fn(SavedActorId) -> Result<OwnedActor, WorldError>,
        legacy: &dyn Fn(&ActorId, i32) -> i64,
    ) -> Result<(), Q3AmmoRegenError> {
        if reader.is_missing() {
            for actor in actors {
                self.require(actor)?;
                let timers = self.entries.get_mut(actor.id()).ok_or(Q3AmmoRegenError::Retired)?;
                for timer in timers.iter_mut() {
                    let previous = self
                        .legacy_owners
                        .iter()
                        .any(|owner| owner.item == timer.item && owner.source == timer.source);
                    timer.elapsed_ms = if previous {
                        Self::elapsed(&reader, timer, legacy(actor.id(), timer.rule.weapon))?
                    } else {
                        0
                    };
                }
            }
            return Ok(());
        }
        let profile = namespaced(reader.field("profile"))
            .map_err(|_| reader.fail("Mapped ammo profile differs from selected supply"))?;
        if profile != self.profile.id {
            return Err(reader.fail("Mapped ammo profile differs from selected supply").into());
        }
        let mut remaining: HashSet<ActorId> = actors.iter().map(|actor| actor.id().clone()).collect();
        reader.field("actors").list(|value| -> Result<_, WorldError> {
            let actor = resolve(read_saved_actor(value.field("actor"))?)?;
            if !remaining.remove(actor.id()) {
                return Err(value.fail("Mapped ammo actor is missing or duplicated"));
            }
            self.require(&actor).map_err(|error| value.fail(&error.to_string()))?;
            let timers = self
                .entries
                .get_mut(actor.id())
                .ok_or_else(|| value.fail("Mapped ammo actor is missing or duplicated"))?;
            let mut unused: HashSet<usize> = (0..timers.len()).collect();
            value.field("timers").list(|timer| -> Result<_, WorldError> {
                let item = namespaced(timer.field("item"))?;
                let source = namespaced(timer.field("source"))?;
                let found = timers
                    .iter()
                    .position(|timer| timer.item == item && timer.source == source)
                    .filter(|index| unused.remove(index))
                    .ok_or_else(|| timer.fail("Mapped ammo pool or original rule differs from selected supply"))?;
                let value_ms = timer.field("elapsedMilliseconds").integer(0)?;
                let elapsed =
                    Self::elapsed(&timer, &timers[found], value_ms).map_err(|error| timer.fail(&error.to_string()))?;
                timers[found].elapsed_ms = elapsed;
                Ok(())
            })?;
            if !unused.is_empty() {
                return Err(value.fail("Saved mapped ammo pool is missing"));
            }
            Ok(())
        })?;
        if !remaining.is_empty() {
            return Err(reader.fail("Saved mapped ammo actor is missing").into());
        }
        Ok(())
    }

    fn elapsed(reader: &SaveReader, timer: &PoolTimerState, value: i64) -> Result<i32, WorldError> {
        if value < 0 || value >= i64::from(timer.rule.time) {
            return Err(reader.fail("Invalid original ammo regeneration counter"));
        }
        Ok(value as i32)
    }

    /// Close the owner, retiring every retained timer.
    pub fn close(&mut self) {
        if self.closed.get() {
            return;
        }
        self.closed.set(true);
        for retired in self.retired.values() {
            retired.set(true);
        }
        self.retired.clear();
        self.entries.clear();
    }
}

#[cfg(test)]
mod tests {
    use qa_content::contract::PickupOwner;
    use qa_core::identity::{IdentityOwner, ProviderId};
    use qa_world::inventory::{InventoryEntry, InventoryTable};
    use qa_world::registry::ActorRegistry;
    use qa_world::save::value::SaveJson;

    use super::*;

    fn profile() -> PickupSupplyProfile {
        PickupSupplyProfile {
            id: "q3:supply/test".to_string(),
            ammo: Vec::new(),
            weapon_owners: Vec::new(),
            ammo_owners: vec![PickupOwner {
                item: "q3:ammo/test-cells".to_string(),
                source: "q3:ammo/plasmagun".to_string(),
            }],
            weapons: Vec::new(),
        }
    }

    fn setup() -> (
        Rc<ActorRegistry>,
        Rc<RefCell<InventoryTable>>,
        OwnedActor,
        Q3MappedAmmoRegeneration,
    ) {
        let owner = IdentityOwner::create("q3-ammo-regen-test").unwrap();
        let mut registry = ActorRegistry::new(owner, 8).unwrap();
        let actor = registry.allocate(ProviderId::new("sim", "test"), "q3:player").unwrap();
        let registry = Rc::new(registry);
        let inventory = Rc::new(RefCell::new(InventoryTable::new()));
        inventory.borrow_mut().create(&registry, &actor, &[]).unwrap();
        inventory
            .borrow_mut()
            .configure(
                &registry,
                &actor,
                InventoryEntry {
                    item: "q3:ammo/test-cells".to_string(),
                    count: 5.0,
                    capacity: 200.0,
                    count_policy: None,
                },
            )
            .unwrap();
        let regen = Q3MappedAmmoRegeneration::new(profile(), registry.clone(), inventory.clone(), Vec::new()).unwrap();
        (registry, inventory, actor, regen)
    }

    #[test]
    fn builds_and_flushes_timers() {
        let (registry, inventory, actor, mut regen) = setup();
        let timers = regen.timers(&actor).unwrap();
        assert_eq!(timers.len(), 1);
        assert_eq!(timers[0].count.get(), 5);
        assert_eq!(timers[0].elapsed_ms.get(), 0);
        assert!((timers[0].current)());
        timers[0].count.set(9);
        timers[0].elapsed_ms.set(120);
        regen.flush(&actor, &timers).unwrap();
        assert_eq!(
            inventory.borrow().count(&registry, actor.id(), "q3:ammo/test-cells"),
            9.0
        );
        let rebuilt = regen.timers(&actor).unwrap();
        assert_eq!(rebuilt[0].count.get(), 9);
        assert_eq!(rebuilt[0].elapsed_ms.get(), 120);
        regen.stored(actor.id(), timers[0].rule.weapon, 40);
        let stored = regen.timers(&actor).unwrap();
        assert_eq!(stored[0].elapsed_ms.get(), 40);
    }

    #[test]
    fn captures_and_restores() {
        let (_registry, _inventory, actor, mut regen) = setup();
        let timers = regen.timers(&actor).unwrap();
        timers[0].elapsed_ms.set(60);
        regen.flush(&actor, &timers).unwrap();
        let saved = regen.capture(std::slice::from_ref(&actor)).unwrap();

        let (_registry2, _inventory2, actor2, mut regen2) = setup();
        let id2 = actor2.id().clone();
        regen2
            .restore(
                SaveReader::new(&saved),
                std::slice::from_ref(&actor2),
                &|_| Ok(actor2.clone()),
                &|_, _| 0,
            )
            .unwrap();
        let restored = regen2.timers(&actor2).unwrap();
        assert_eq!(restored[0].elapsed_ms.get(), 60);
        assert_eq!(actor2.id(), &id2);

        let missing = SaveJson::Null;
        let present = SaveReader::new(&missing);
        assert!(regen2
            .restore(present, &[], &|_| Ok(actor2.clone()), &|_, _| 0)
            .is_err());
    }

    #[test]
    fn validates_and_retires() {
        let (registry, inventory, actor, mut regen) = setup();
        let mut duplicated = profile();
        duplicated.ammo_owners.push(PickupOwner {
            item: "q3:ammo/test-cells".to_string(),
            source: "q3:ammo/shotgun".to_string(),
        });
        assert!(matches!(
            Q3MappedAmmoRegeneration::new(duplicated, registry.clone(), inventory.clone(), Vec::new()),
            Err(Q3AmmoRegenError::DuplicatePool)
        ));
        let mut foreign = profile();
        foreign.ammo_owners[0].source = "q3:ammo/unknown".to_string();
        assert!(matches!(
            Q3MappedAmmoRegeneration::new(foreign, registry.clone(), inventory.clone(), Vec::new()),
            Err(Q3AmmoRegenError::MissingSourceAmmo)
        ));
        let timers = regen.timers(&actor).unwrap();
        assert!((timers[0].current)());
        regen.on_actor_released(actor.id());
        assert!(!(timers[0].current)());
        regen.close();
        assert!(matches!(regen.timers(&actor), Err(Q3AmmoRegenError::Closed)));
    }
}
