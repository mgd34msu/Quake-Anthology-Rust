//! Original-allocation cargo for dropped pickups. Port of
//! `src/app/bootstrap/simulation/dropped-pickups.ts`.

use std::collections::{HashMap, HashSet};

use qa_core::identity::{ActorId, ProviderId, SavedActorId};
use qa_world::save::records::{read_saved_actor, write_saved_actor};
use qa_world::save::value::{SaveJson, SaveReader, arr, int, namespaced, obj, str as json_str};

/// Item identity belongs to the actual source allocation, not its reusable pickup alias.
pub trait PickupActorHost {
    fn is_live(&self, actor: &ActorId) -> bool;
    fn source_of(&self, actor: &ActorId) -> Option<(ProviderId, u32)>;
    fn at_source(&self, provider: &ProviderId, slot: u32) -> Option<ActorId>;
    fn resolve_saved(&self, saved: SavedActorId) -> Option<ActorId>;
}

/// Cargo carried by one dropped pickup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DroppedPickupCargo {
    pub item: String,
    pub count: i64,
}

/// Cargo retained for a level by source slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DroppedPickupSlot {
    pub item: String,
    pub count: i64,
    pub slot: u32,
}

/// Retained cargo per level map path.
pub type DroppedPickupLevels = HashMap<String, Vec<DroppedPickupSlot>>;

/// Dropped pickup cargo failure.
#[derive(Debug, thiserror::Error)]
pub enum DroppedPickupError {
    #[error(transparent)]
    World(#[from] qa_world::WorldError),
    #[error("Invalid original dropped pickup cargo")]
    BadCargo,
    #[error("Dropped pickup has no original level allocation")]
    MissingAllocation,
    #[error("Retained dropped pickup has no restored source allocation")]
    MissingRestoredAllocation,
}

/// Cargo keyed by live actor, retained per level by source slot.
#[derive(Debug)]
pub struct SourcePickupCargo<H> {
    host: H,
    provider: ProviderId,
    current: HashMap<ActorId, DroppedPickupCargo>,
    levels: DroppedPickupLevels,
}

impl<H: PickupActorHost> SourcePickupCargo<H> {
    #[must_use]
    pub fn new(host: H, provider: ProviderId) -> Self {
        Self { host, provider, current: HashMap::new(), levels: HashMap::new() }
    }

    /// Mirror of the donor `onRelease` subscription: the session calls this when
    /// an actor is released because `ActorRegistry` exposes no release hook.
    pub fn note_released(&mut self, actor: &ActorId) {
        self.current.remove(actor);
    }

    #[must_use]
    pub fn get(&self, actor: &ActorId) -> Option<&DroppedPickupCargo> {
        self.current.get(actor)
    }

    pub fn set(&mut self, actor: &ActorId, cargo: &DroppedPickupCargo) -> Result<(), DroppedPickupError> {
        if !self.host.is_live(actor) || cargo.count <= 0 {
            return Err(DroppedPickupError::BadCargo);
        }
        self.current.insert(actor.clone(), cargo.clone());
        Ok(())
    }

    pub fn delete(&mut self, actor: &ActorId) {
        self.current.remove(actor);
    }

    pub fn travel(&self, map: &str, new_unit: bool) -> Result<DroppedPickupLevels, DroppedPickupError> {
        if new_unit {
            return Ok(HashMap::new());
        }
        let mut levels = self.levels.clone();
        let mut items = Vec::with_capacity(self.current.len());
        for (actor, cargo) in &self.current {
            let source = self.host.source_of(actor).filter(|(owner, _)| *owner == self.provider);
            let Some((_, slot)) = source else {
                return Err(DroppedPickupError::MissingAllocation);
            };
            items.push(DroppedPickupSlot { item: cargo.item.clone(), count: cargo.count, slot });
        }
        items.sort_by_key(|item| item.slot);
        levels.insert(map.to_string(), items);
        Ok(levels)
    }

    pub fn revisit(&mut self, levels: &DroppedPickupLevels, map: &str) -> Result<(), DroppedPickupError> {
        self.levels = levels.clone();
        for entry in levels.get(map).cloned().unwrap_or_default() {
            let actor = self.host.at_source(&self.provider, entry.slot);
            let Some(actor) = actor else {
                return Err(DroppedPickupError::MissingRestoredAllocation);
            };
            self.set(&actor, &DroppedPickupCargo { item: entry.item, count: entry.count })?;
        }
        Ok(())
    }

    #[must_use]
    pub fn capture(&self) -> SaveJson {
        let mut current: Vec<(&ActorId, &DroppedPickupCargo)> = self.current.iter().collect();
        current.sort_by_key(|(actor, _)| (actor.slot(), actor.generation()));
        let mut levels: Vec<(&String, &Vec<DroppedPickupSlot>)> = self.levels.iter().collect();
        levels.sort_by_key(|(map, _)| map.as_str());
        obj(vec![
            (
                "current",
                arr(current
                    .into_iter()
                    .map(|(actor, cargo)| {
                        obj(vec![
                            ("actor", write_saved_actor(SavedActorId::from(actor))),
                            ("item", json_str(&cargo.item)),
                            ("count", int(cargo.count)),
                        ])
                    })
                    .collect()),
            ),
            (
                "levels",
                arr(levels
                    .into_iter()
                    .map(|(map, items)| {
                        obj(vec![
                            ("map", json_str(map)),
                            (
                                "items",
                                arr(items
                                    .iter()
                                    .map(|item| {
                                        obj(vec![
                                            ("slot", int(i64::from(item.slot))),
                                            ("item", json_str(&item.item)),
                                            ("count", int(item.count)),
                                        ])
                                    })
                                    .collect()),
                            ),
                        ])
                    })
                    .collect()),
            ),
        ])
    }

    pub fn restore(&mut self, reader: SaveReader) -> Result<(), DroppedPickupError> {
        self.current.clear();
        self.levels.clear();
        if reader.is_missing() {
            return Ok(());
        }
        reader.field("current").list(|value| -> Result<(), DroppedPickupError> {
            let actor = match self.host.resolve_saved(read_saved_actor(value.field("actor"))?) {
                Some(actor) if !self.current.contains_key(&actor) => actor,
                _ => return Err(value.fail("Missing or duplicate dropped pickup actor").into()),
            };
            let cargo = Self::read_cargo(value)?;
            self.set(&actor, &cargo)?;
            Ok(())
        })?;
        reader.field("levels").list(|value| -> Result<(), DroppedPickupError> {
            let map = value.field("map").string()?;
            let items = value.field("items").list(|item| -> Result<DroppedPickupSlot, DroppedPickupError> {
                let slot = item.field("slot").integer(0)?;
                let slot = u32::try_from(slot)
                    .map_err(|_| item.field("slot").fail("expected an integer in range"))?;
                let cargo = Self::read_cargo(item)?;
                Ok(DroppedPickupSlot { item: cargo.item, count: cargo.count, slot })
            })?;
            let mut slots = HashSet::new();
            if self.levels.contains_key(&map) || items.iter().any(|item| !slots.insert(item.slot)) {
                return Err(value.fail("Duplicate dropped pickup level or source slot").into());
            }
            self.levels.insert(map, items);
            Ok(())
        })?;
        Ok(())
    }

    fn read_cargo(value: SaveReader) -> Result<DroppedPickupCargo, DroppedPickupError> {
        Ok(DroppedPickupCargo {
            item: namespaced(value.field("item"))?,
            count: value.field("count").integer(1)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;

    struct MockHost {
        owner: IdentityOwner,
        live: HashSet<(u32, u32)>,
        provider: ProviderId,
    }

    impl MockHost {
        fn new() -> Self {
            Self {
                owner: IdentityOwner::create("test").unwrap(),
                live: HashSet::from([(1u32, 1u32), (2, 1)]),
                provider: ProviderId::new("q2", "game"),
            }
        }
    }

    impl PickupActorHost for MockHost {
        fn is_live(&self, actor: &ActorId) -> bool {
            self.owner.owns_actor(actor) && self.live.contains(&(actor.slot(), actor.generation()))
        }
        fn source_of(&self, actor: &ActorId) -> Option<(ProviderId, u32)> {
            self.is_live(actor).then(|| (self.provider.clone(), actor.slot() + 100))
        }
        fn at_source(&self, provider: &ProviderId, slot: u32) -> Option<ActorId> {
            (*provider == self.provider && slot >= 100)
                .then(|| self.owner.actor(slot - 100, 1))
                .filter(|actor| self.is_live(actor))
        }
        fn resolve_saved(&self, saved: SavedActorId) -> Option<ActorId> {
            let actor = self.owner.actor(saved.slot, saved.generation);
            self.is_live(&actor).then_some(actor)
        }
    }

    fn test_cargo() -> SourcePickupCargo<MockHost> {
        let provider = ProviderId::new("q2", "game");
        SourcePickupCargo::new(MockHost::new(), provider)
    }

    #[test]
    fn set_rejects_bad_cargo() {
        let mut cargo = test_cargo();
        let actor = cargo.host.owner.actor(9, 1);
        assert!(cargo.set(&actor, &DroppedPickupCargo { item: "q2:shells".to_string(), count: 1 }).is_err());
        let live = cargo.host.owner.actor(1, 1);
        let error = cargo
            .set(&live, &DroppedPickupCargo { item: "q2:shells".to_string(), count: 0 })
            .unwrap_err()
            .to_string();
        assert_eq!(error, "Invalid original dropped pickup cargo");
    }

    #[test]
    fn travel_and_revisit_round_trip() {
        let mut cargo = test_cargo();
        let actor = cargo.host.owner.actor(1, 1);
        cargo.set(&actor, &DroppedPickupCargo { item: "q2:shells".to_string(), count: 5 }).unwrap();
        let levels = cargo.travel("maps/base1.bsp", false).unwrap();
        assert_eq!(levels["maps/base1.bsp"], vec![DroppedPickupSlot { item: "q2:shells".to_string(), count: 5, slot: 101 }]);
        assert!(cargo.travel("maps/base1.bsp", true).unwrap().is_empty());
        let mut fresh = test_cargo();
        fresh.revisit(&levels, "maps/base1.bsp").unwrap();
        assert_eq!(fresh.get(&actor).unwrap().count, 5);
    }

    #[test]
    fn revisit_without_source_allocation_fails() {
        let mut cargo = test_cargo();
        let levels = HashMap::from([("maps/base1.bsp".to_string(), vec![DroppedPickupSlot {
            item: "q2:shells".to_string(),
            count: 1,
            slot: 999,
        }])]);
        let error = cargo.revisit(&levels, "maps/base1.bsp").unwrap_err().to_string();
        assert_eq!(error, "Retained dropped pickup has no restored source allocation");
    }

    #[test]
    fn capture_restore_round_trip() {
        let mut cargo = test_cargo();
        let actor = cargo.host.owner.actor(2, 1);
        cargo.set(&actor, &DroppedPickupCargo { item: "q2:cells".to_string(), count: 3 }).unwrap();
        let levels = cargo.travel("maps/base1.bsp", false).unwrap();
        let mut staged = test_cargo();
        staged.revisit(&levels, "maps/base1.bsp").unwrap();
        let value = staged.capture();
        let mut fresh = test_cargo();
        fresh.restore(SaveReader::at(&value, "pickups")).unwrap();
        assert_eq!(fresh.capture(), value);
    }

    #[test]
    fn restore_rejects_duplicate_actor() {
        let entry = obj(vec![
            ("actor", obj(vec![("slot", int(1)), ("generation", int(1))])),
            ("item", json_str("q2:shells")),
            ("count", int(1)),
        ]);
        let value = obj(vec![("current", arr(vec![entry.clone(), entry])), ("levels", arr(vec![]))]);
        let mut cargo = test_cargo();
        let error = cargo.restore(SaveReader::at(&value, "pickups")).unwrap_err().to_string();
        assert!(error.contains("Missing or duplicate dropped pickup actor"), "{error}");
    }
}
