//! Generational actor registry ported from `src/world/actors/registry.ts`.
//! Host generations invalidate observations; a source slot resolves its
//! current occupant. Release callbacks are owned by the [`Simulation`]
//! (see `session.rs`), which releases bodies, inventories, and thinks in
//! source order instead of through reentrant registry hooks.
//!
//! [`Simulation`]: crate::session::Simulation

use std::collections::HashMap;

use qa_core::identity::{ActorId, IdentityOwner, OwnedActor, ProviderId, SavedActorId, SessionId};
use qa_core::time::SourceTime;

use crate::WorldError;

/// Stable map key for a provider (`namespace:name`).
#[must_use]
pub fn provider_key(provider: &ProviderId) -> String {
    format!("{}:{}", provider.namespace, provider.name)
}

struct Slot {
    generation: u32,
    live: Option<LiveSlot>,
}

struct LiveSlot {
    owner: ProviderId,
    definition: String,
    source: Option<(ProviderId, u32)>,
}

/// Live actor observation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActorObservation {
    /// Observed handle.
    pub id: ActorId,
    /// Owning provider.
    pub owner: ProviderId,
    /// `namespace:name` definition.
    pub definition: String,
}

/// Saved slot lifetime for checkpoints.
#[derive(Debug, Clone, PartialEq)]
pub enum SlotLifetime {
    /// Occupied slot.
    Active {
        /// Owning provider.
        owner: ProviderId,
        /// `namespace:name` definition.
        definition: String,
    },
    /// Free slot.
    Free {
        /// When the slot was freed, if retained from a save.
        freed_at: Option<SourceTime>,
    },
}

/// Saved actor slot.
#[derive(Debug, Clone, PartialEq)]
pub struct ActorSlotCheckpoint {
    /// Registry slot.
    pub slot: u32,
    /// Slot generation.
    pub generation: u32,
    /// Slot lifetime.
    pub lifetime: SlotLifetime,
}

/// Saved source-slot binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceActorCheckpoint {
    /// Source provider.
    pub provider: ProviderId,
    /// Source slot.
    pub source_slot: u32,
    /// Bound registry slot.
    pub slot: u32,
    /// Bound generation.
    pub generation: u32,
}

type ReleaseHook = dyn Fn(&OwnedActor);

/// Session-owned actor registry.
pub struct ActorRegistry {
    owner: IdentityOwner,
    slots: Vec<Slot>,
    sources: HashMap<String, HashMap<u32, OwnedActor>>,
    capacity: usize,
    revision: u64,
    closed: bool,
    release_hooks: Vec<Box<ReleaseHook>>,
}

impl ActorRegistry {
    /// Create a registry over a fresh identity authority.
    pub fn new(owner: IdentityOwner, capacity: usize) -> Result<Self, WorldError> {
        if capacity == 0 {
            return Err(WorldError::BadCapacity);
        }
        Ok(Self {
            owner,
            slots: Vec::new(),
            sources: HashMap::new(),
            capacity,
            revision: 0,
            closed: false,
            release_hooks: Vec::new(),
        })
    }

    /// Registry session.
    #[must_use]
    pub fn session(&self) -> &SessionId {
        self.owner.session()
    }

    /// Registry identity owner (for session-membership checks).
    #[must_use]
    pub fn owner(&self) -> &IdentityOwner {
        &self.owner
    }

    /// Ordering revision, bumped by every allocation and release.
    #[must_use]
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Maximum slot count.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    fn assert_open(&self) -> Result<(), WorldError> {
        if self.closed {
            return Err(WorldError::RegistryClosed);
        }
        Ok(())
    }

    /// Allocate an actor for an owning provider.
    pub fn allocate(&mut self, owner: ProviderId, definition: &str) -> Result<OwnedActor, WorldError> {
        self.assert_open()?;
        let index = match self.slots.iter().position(|slot| slot.live.is_none()) {
            Some(index) => index,
            None => {
                if self.slots.len() >= self.capacity {
                    return Err(WorldError::RegistryFull);
                }
                self.slots.push(Slot {
                    generation: 0,
                    live: None,
                });
                self.slots.len() - 1
            }
        };
        let slot = &mut self.slots[index];
        let id = self.owner.actor(index as u32, slot.generation);
        let actor = self
            .owner
            .owned_actor(&id, owner.clone())
            .map_err(|_| WorldError::ForeignActor)?;
        slot.live = Some(LiveSlot {
            owner,
            definition: definition.to_string(),
            source: None,
        });
        self.revision += 1;
        Ok(actor)
    }

    /// Allocate an actor bound to a source slot.
    pub fn allocate_at_source(
        &mut self,
        owner: ProviderId,
        source_slot: u32,
        definition: &str,
    ) -> Result<OwnedActor, WorldError> {
        let table = self.sources.entry(provider_key(&owner)).or_default();
        if table.contains_key(&source_slot) {
            return Err(WorldError::SourceBinding(format!(
                "Source slot {} is occupied",
                provider_key(&owner)
            )));
        }
        let actor = self.allocate(owner.clone(), definition)?;
        let slot = &mut self.slots[actor.id().slot() as usize];
        if let Some(live) = slot.live.as_mut() {
            live.source = Some((owner.clone(), source_slot));
        }
        self.sources
            .entry(provider_key(&owner))
            .or_default()
            .insert(source_slot, actor.clone());
        self.revision += 1;
        Ok(actor)
    }

    /// Current occupant of a source slot.
    #[must_use]
    pub fn at_source(&self, owner: &ProviderId, slot: u32) -> Option<OwnedActor> {
        self.sources.get(&provider_key(owner))?.get(&slot).cloned()
    }

    /// Source binding of a live actor.
    #[must_use]
    pub fn source_of(&self, actor: &ActorId) -> Option<(ProviderId, u32)> {
        if !self.is_live(actor) {
            return None;
        }
        self.slots.get(actor.slot() as usize)?.live.as_ref()?.source.clone()
    }

    /// Run `hook` after every successful release.
    pub fn on_release(&mut self, hook: Box<ReleaseHook>) {
        self.release_hooks.push(hook);
    }

    /// Resolve a saved slot/generation reference under this authority.
    #[must_use]
    pub fn resolve_saved(&self, saved: &SavedActorId) -> Option<OwnedActor> {
        let id = self.owner.actor(saved.slot, saved.generation);
        self.resolve_owned(&id)
    }

    /// Release an actor, invalidating its generation before tables drain.
    pub fn release(&mut self, actor: &OwnedActor) -> Result<(), WorldError> {
        let index = actor.id().slot() as usize;
        let slot = self.slots.get_mut(index).ok_or(WorldError::StaleActor)?;
        let live = slot.live.as_ref().ok_or(WorldError::StaleActor)?;
        if slot.generation != actor.id().generation() || &live.owner != actor.owner() {
            return Err(WorldError::StaleActor);
        }
        let source = live.source.clone();
        slot.live = None;
        slot.generation = slot.generation.checked_add(1).ok_or(WorldError::GenerationExhausted)?;
        self.revision += 1;
        if let Some((provider, source_slot)) = source {
            if let Some(table) = self.sources.get_mut(&provider_key(&provider)) {
                table.remove(&source_slot);
            }
        }
        for hook in &self.release_hooks {
            hook(actor);
        }
        Ok(())
    }

    /// Whether the handle names the live occupant of its slot.
    #[must_use]
    pub fn is_live(&self, actor: &ActorId) -> bool {
        if !self.owner.owns_actor(actor) {
            return false;
        }
        self.slots
            .get(actor.slot() as usize)
            .is_some_and(|slot| slot.live.is_some() && slot.generation == actor.generation())
    }

    /// Observe a live actor.
    #[must_use]
    pub fn observe(&self, actor: &ActorId) -> Option<ActorObservation> {
        if !self.is_live(actor) {
            return None;
        }
        let live = self.slots.get(actor.slot() as usize)?.live.as_ref()?;
        Some(ActorObservation {
            id: actor.clone(),
            owner: live.owner.clone(),
            definition: live.definition.clone(),
        })
    }

    /// Resolve a live handle back to its owned authority.
    #[must_use]
    pub fn resolve_owned(&self, actor: &ActorId) -> Option<OwnedActor> {
        if !self.is_live(actor) {
            return None;
        }
        let live = self.slots.get(actor.slot() as usize)?.live.as_ref()?;
        self.owner.owned_actor(actor, live.owner.clone()).ok()
    }

    /// Live actors owned by one provider, in slot order.
    #[must_use]
    pub fn owned_by(&self, owner: &ProviderId) -> Vec<OwnedActor> {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(index, slot)| {
                let live = slot.live.as_ref()?;
                if &live.owner != owner {
                    return None;
                }
                let id = self.owner.actor(index as u32, slot.generation);
                self.owner.owned_actor(&id, owner.clone()).ok()
            })
            .collect()
    }

    /// Number of live actors, without allocating an observation vector.
    #[must_use]
    pub fn live_count(&self) -> usize {
        self.slots.iter().filter(|slot| slot.live.is_some()).count()
    }

    /// Visit every live actor in slot order without allocating.
    ///
    /// The visitor receives the live handle, owning provider, and definition.
    /// This is the allocation-free counterpart of [`observations`](Self::observations)
    /// for per-frame passes.
    pub fn for_each_live(&self, mut visit: impl FnMut(ActorId, &ProviderId, &str)) {
        for (index, slot) in self.slots.iter().enumerate() {
            let Some(live) = slot.live.as_ref() else {
                continue;
            };
            visit(
                self.owner.actor(index as u32, slot.generation),
                &live.owner,
                live.definition.as_str(),
            );
        }
    }

    /// Live handle for an exact slot/generation pair, without scanning.
    #[must_use]
    pub fn live_id(&self, slot: u32, generation: u32) -> Option<ActorId> {
        let found = self.slots.get(slot as usize)?;
        if found.live.is_none() || found.generation != generation {
            return None;
        }
        Some(self.owner.actor(slot, generation))
    }

    /// Live handle for a slot's current occupant, without scanning.
    #[must_use]
    pub fn live_id_in_slot(&self, slot: u32) -> Option<ActorId> {
        let found = self.slots.get(slot as usize)?;
        found.live.as_ref()?;
        Some(self.owner.actor(slot, found.generation))
    }

    /// All live observations, in slot order.
    #[must_use]
    pub fn observations(&self) -> Vec<ActorObservation> {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(index, slot)| {
                let live = slot.live.as_ref()?;
                let id = self.owner.actor(index as u32, slot.generation);
                Some(ActorObservation {
                    id,
                    owner: live.owner.clone(),
                    definition: live.definition.clone(),
                })
            })
            .collect()
    }

    /// Assert owned authority over a live actor.
    pub fn assert_owned(&self, actor: &OwnedActor) -> Result<(), WorldError> {
        if !self.owner.owns_owned(actor) {
            return Err(WorldError::ForeignActor);
        }
        if self.resolve_owned(actor.id()).as_ref() != Some(actor) {
            return Err(WorldError::StaleActor);
        }
        Ok(())
    }

    /// Checkpoint every slot.
    #[must_use]
    pub fn checkpoint(&self) -> Vec<ActorSlotCheckpoint> {
        self.slots
            .iter()
            .enumerate()
            .map(|(index, slot)| ActorSlotCheckpoint {
                slot: index as u32,
                generation: slot.generation,
                lifetime: slot
                    .live
                    .as_ref()
                    .map_or(SlotLifetime::Free { freed_at: None }, |live| SlotLifetime::Active {
                        owner: live.owner.clone(),
                        definition: live.definition.clone(),
                    }),
            })
            .collect()
    }

    /// Checkpoint source bindings.
    #[must_use]
    pub fn source_checkpoint(&self) -> Vec<SourceActorCheckpoint> {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(index, slot)| {
                let live = slot.live.as_ref()?;
                let (provider, source_slot) = live.source.clone()?;
                Some(SourceActorCheckpoint {
                    provider,
                    source_slot,
                    slot: index as u32,
                    generation: slot.generation,
                })
            })
            .collect()
    }

    /// Restore a registry from checkpoints under a fresh authority.
    pub fn restore(
        owner: IdentityOwner,
        checkpoints: &[ActorSlotCheckpoint],
        sources: &[SourceActorCheckpoint],
        capacity: usize,
    ) -> Result<Self, WorldError> {
        if capacity == 0 || checkpoints.len() > capacity {
            return Err(WorldError::BadCapacity);
        }
        let mut registry = Self::new(owner, capacity)?;
        for (index, checkpoint) in checkpoints.iter().enumerate() {
            if checkpoint.slot != index as u32 {
                return Err(WorldError::BadSave("Invalid actor slot checkpoint".to_string()));
            }
            let live = match &checkpoint.lifetime {
                SlotLifetime::Free { .. } => None,
                SlotLifetime::Active { owner, definition } => Some(LiveSlot {
                    owner: owner.clone(),
                    definition: definition.clone(),
                    source: None,
                }),
            };
            registry.slots.push(Slot {
                generation: checkpoint.generation,
                live,
            });
            if registry.slots[index].live.is_some() {
                registry.revision += 1;
            }
        }
        for source in sources {
            let slot = registry
                .slots
                .get_mut(source.slot as usize)
                .ok_or_else(|| WorldError::BadSave("Invalid source actor checkpoint".to_string()))?;
            let live = slot
                .live
                .as_mut()
                .ok_or_else(|| WorldError::BadSave("Invalid source actor checkpoint".to_string()))?;
            if slot.generation != source.generation || live.owner != source.provider {
                return Err(WorldError::BadSave("Invalid source actor checkpoint".to_string()));
            }
            if live.source.is_some() {
                return Err(WorldError::BadSave("Duplicate source actor binding".to_string()));
            }
            live.source = Some((source.provider.clone(), source.source_slot));
            let id = registry.owner.actor(source.slot, source.generation);
            let owned = registry
                .owner
                .owned_actor(&id, source.provider.clone())
                .map_err(|_| WorldError::ForeignActor)?;
            let table = registry.sources.entry(provider_key(&source.provider)).or_default();
            if table.insert(source.source_slot, owned).is_some() {
                return Err(WorldError::BadSave("Duplicate source actor binding".to_string()));
            }
            registry.revision += 1;
        }
        Ok(registry)
    }

    /// Close the registry, releasing every live actor.
    pub fn close(&mut self) {
        self.closed = true;
        self.slots.clear();
        self.sources.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owner(name: &str) -> IdentityOwner {
        IdentityOwner::create(name).unwrap()
    }

    fn provider() -> ProviderId {
        ProviderId::new("q3", "game")
    }

    #[test]
    fn allocate_release_reuses_slots_with_new_generations() {
        let mut registry = ActorRegistry::new(owner("test"), 4).unwrap();
        let first = registry.allocate(provider(), "q3:soldier").unwrap();
        assert!(registry.is_live(first.id()));
        assert_eq!(registry.observe(first.id()).unwrap().definition, "q3:soldier");
        registry.release(&first).unwrap();
        assert!(!registry.is_live(first.id()));
        assert!(registry.observe(first.id()).is_none());
        let second = registry.allocate(provider(), "q3:scout").unwrap();
        assert_eq!(second.id().slot(), first.id().slot());
        assert_eq!(second.id().generation(), first.id().generation() + 1);
        assert!(registry.is_live(second.id()));
        assert_eq!(registry.owned_by(&provider()).len(), 1);
    }

    #[test]
    fn foreign_actors_never_go_live() {
        let registry = ActorRegistry::new(owner("a"), 4).unwrap();
        let foreign = owner("b");
        let id = foreign.actor(0, 0);
        assert!(!registry.is_live(&id));
        assert!(registry.observe(&id).is_none());
        assert!(registry.resolve_owned(&id).is_none());
    }

    #[test]
    fn source_slots_track_their_occupant() {
        let mut registry = ActorRegistry::new(owner("test"), 4).unwrap();
        let actor = registry.allocate_at_source(provider(), 7, "q2:soldier").unwrap();
        assert_eq!(registry.at_source(&provider(), 7).as_ref(), Some(&actor));
        assert_eq!(registry.source_of(actor.id()), Some((provider(), 7)));
        assert!(registry.allocate_at_source(provider(), 7, "q2:other").is_err());
        registry.release(&actor).unwrap();
        assert!(registry.at_source(&provider(), 7).is_none());
    }

    #[test]
    fn registry_enforces_capacity_and_checkpoint_round_trip() {
        let mut registry = ActorRegistry::new(owner("test"), 1).unwrap();
        let actor = registry.allocate(provider(), "q1:ogre").unwrap();
        assert_eq!(registry.allocate(provider(), "q1:fiend"), Err(WorldError::RegistryFull));
        let checkpoints = registry.checkpoint();
        let sources = registry.source_checkpoint();
        let restored = ActorRegistry::restore(owner("restored"), &checkpoints, &sources, 1).unwrap();
        let id = restored.observations().into_iter().next().expect("restored actor").id;
        assert_eq!(id.slot(), actor.id().slot());
        assert_eq!(id.generation(), actor.id().generation());
    }

    #[test]
    fn live_helpers_match_observations() {
        let mut registry = ActorRegistry::new(owner("test"), 8).unwrap();
        let first = registry.allocate(provider(), "q3:soldier").unwrap();
        let second = registry.allocate(provider(), "q3:scout").unwrap();
        let third = registry.allocate(provider(), "q3:medic").unwrap();
        registry.release(&second).unwrap();
        let fresh = registry.allocate(provider(), "q3:engineer").unwrap();

        let observed = registry.observations();
        assert_eq!(registry.live_count(), observed.len());

        let mut visited = Vec::new();
        registry.for_each_live(|id, owner, definition| {
            visited.push(ActorObservation {
                id,
                owner: owner.clone(),
                definition: definition.to_owned(),
            });
        });
        assert_eq!(visited, observed);

        assert_eq!(
            registry.live_id(first.id().slot(), first.id().generation()).as_ref(),
            Some(first.id())
        );
        assert_eq!(registry.live_id(second.id().slot(), second.id().generation()), None);
        assert_eq!(
            registry.live_id(third.id().slot(), third.id().generation()).as_ref(),
            Some(third.id())
        );
        assert_eq!(registry.live_id(7, 0), None);
        assert_eq!(registry.live_id_in_slot(second.id().slot()).as_ref(), Some(fresh.id()));
        assert_eq!(registry.live_id_in_slot(7), None);
    }
}
