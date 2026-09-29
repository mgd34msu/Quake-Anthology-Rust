//! Source-slot world host: actor/reference conversion, spawn binding, and
//! `SV_LinkEdict` link calls.
//!
//! Ported from donor `src/compat/qc/world-host.ts` (`QcWorldHost`,
//! `qcLinkBounds`).
//!
//! Local mirrors: [`WorldSlots`] mirrors the `SessionActorRegistry` /
//! `SourceActorSlots` pair from `src/world/actors/index.ts`; [`WorldBodies`]
//! mirrors the `SharedBodyTable` link surface; both are intentionally narrow
//! so the session worker can back them without depending on this crate.
//! Body bindings reuse `super::actor_state`; slot references reuse
//! `super::entity_host`.

use qa_core::identity::{ActorId, SavedActorId};

use super::actor_state::{create_qc_body_binding, ActorResolver, BodyState, QcBodyBinding, FLAG_ITEM};
use super::entity_host::reference_for_slot;
use crate::error::GuestError;
use qa_core::math::{Bounds, Vec3};

/// Source-slot registry surface behind the world host.
pub trait WorldSlots {
    /// Whether an actor is live.
    fn is_live(&self, actor: &ActorId) -> bool;
    /// Whether a slot is free.
    fn is_free(&self, slot: usize) -> bool;
    /// Source slot owned by an actor, if the actor owns a QC row.
    fn source_slot(&self, actor: &ActorId) -> Option<usize>;
    /// Actor bound to a slot, if any.
    fn at(&self, slot: usize) -> Option<ActorId>;
    /// Bind an actor record to a pre-opened slot (map/client rows).
    fn bind_existing(&mut self, slot: usize) -> ActorId;
    /// Surrogate actor supplied by another module for a slot, if any.
    fn foreign_actor(&self, slot: usize) -> Option<ActorId> {
        let _ = slot;
        None
    }
    /// Encode an actor that owns no QC row.
    fn foreign_reference(&self, actor: &ActorId) -> i32;
}

/// Body-table surface behind the world host.
pub trait WorldBodies {
    /// Read a body snapshot, if bound.
    fn read(&self, actor: &ActorId) -> Option<BodyState>;
    /// Bind a QC body projection to an actor.
    fn bind(&mut self, actor: &ActorId, slot: usize, binding: QcBodyBinding);
    /// Link an actor into the collision world.
    fn link(&mut self, actor: &ActorId);
}

/// `SV_LinkEdict` bounds: items expand horizontally by 15, other edicts by
/// 1 on every axis.
#[must_use]
pub fn qc_link_bounds(origin: Vec3, bounds: Bounds, flags: i32) -> Bounds {
    let item = flags & FLAG_ITEM != 0;
    let (xy, z) = if item { (15.0, 0.0) } else { (1.0, 1.0) };
    Bounds {
        min: Vec3 {
            x: origin.x + bounds.min.x - xy,
            y: origin.y + bounds.min.y - xy,
            z: origin.z + bounds.min.z - z,
        },
        max: Vec3 {
            x: origin.x + bounds.max.x + xy,
            y: origin.y + bounds.max.y + xy,
            z: origin.z + bounds.max.z + z,
        },
    }
}

/// QC words remain authoritative; spatial snapshots change only at source
/// link calls.
pub struct QcWorldHost<S, B> {
    slots: S,
    bodies: B,
    resolve: ActorResolver,
    admit: Option<Box<dyn FnMut(&ActorId, usize)>>,
}

impl<S: WorldSlots, B: WorldBodies> QcWorldHost<S, B> {
    /// Build the host. `resolve` maps saved ground references back to live
    /// actors for body bindings.
    pub fn new(slots: S, bodies: B, resolve: ActorResolver) -> Self {
        Self {
            slots,
            bodies,
            resolve,
            admit: None,
        }
    }

    /// Install the admit hook run when a slot gains its first body binding.
    pub fn set_admit_hook(&mut self, hook: impl FnMut(&ActorId, usize) + 'static) {
        self.admit = Some(Box::new(hook));
    }

    /// Borrow the slot registry.
    #[must_use]
    pub fn slots(&self) -> &S {
        &self.slots
    }

    /// Borrow the body table.
    #[must_use]
    pub fn bodies(&self) -> &B {
        &self.bodies
    }

    /// Borrow the slot registry mutably.
    pub fn slots_mut(&mut self) -> &mut S {
        &mut self.slots
    }

    /// Borrow the body table mutably.
    pub fn bodies_mut(&mut self) -> &mut B {
        &mut self.bodies
    }

    /// Encode an actor as a QC entity reference.
    pub fn reference(&self, actor: &ActorId) -> Result<i32, GuestError> {
        if !self.slots.is_live(actor) {
            return Err(GuestError::invalid("cannot encode a stale actor as a QC entity"));
        }
        match self.slots.source_slot(actor) {
            Some(slot) => Ok(reference_for_slot(slot)),
            None => Ok(self.slots.foreign_reference(actor)),
        }
    }

    /// Resolve a slot to its actor, binding a body projection on first use.
    /// May be called by the application when it opens map/client slots
    /// before execution.
    pub fn actor(&mut self, slot: usize) -> Result<ActorId, GuestError> {
        if self.slots.is_free(slot) {
            return Err(GuestError::invalid("world builtin references a free source edict"));
        }
        if let Some(borrowed) = self.slots.foreign_actor(slot) {
            return Ok(borrowed);
        }
        let actor = match self.slots.at(slot) {
            Some(actor) => actor,
            None => self.slots.bind_existing(slot),
        };
        if self.bodies.read(&actor).is_none() {
            let binding = create_qc_body_binding(&actor, self.resolve.clone());
            self.bodies.bind(&actor, slot, binding);
            if let Some(admit) = self.admit.as_mut() {
                admit(&actor, slot);
            }
        }
        Ok(actor)
    }

    /// Link one slot into the collision world. Slot 0 and free slots are
    /// ignored, matching `SV_LinkEdict` guards.
    pub fn link(&mut self, slot: usize) -> Result<(), GuestError> {
        if slot == 0 || self.slots.is_free(slot) {
            return Ok(());
        }
        let actor = self.actor(slot)?;
        self.bodies.link(&actor);
        Ok(())
    }

    /// Resolve a saved ground reference through this host's resolver.
    #[must_use]
    pub fn resolve_ground(&self, saved: &SavedActorId) -> Option<ActorId> {
        (self.resolve)(saved)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;
    use qa_core::math::vec3;
    use std::collections::HashMap;

    struct FakeSlots {
        owner_session: u64,
        live: Vec<ActorId>,
        slots: Vec<Option<ActorId>>,
        foreign: HashMap<usize, ActorId>,
    }

    impl WorldSlots for FakeSlots {
        fn is_live(&self, actor: &ActorId) -> bool {
            let _ = self.owner_session;
            self.live.contains(actor)
        }

        fn is_free(&self, slot: usize) -> bool {
            self.slots.get(slot).is_none_or(Option::is_none)
        }

        fn source_slot(&self, actor: &ActorId) -> Option<usize> {
            self.slots.iter().position(|entry| entry.as_ref() == Some(actor))
        }

        fn at(&self, slot: usize) -> Option<ActorId> {
            self.slots.get(slot).and_then(Clone::clone)
        }

        fn bind_existing(&mut self, slot: usize) -> ActorId {
            let bound = self.live[slot % self.live.len()].clone();
            if slot >= self.slots.len() {
                self.slots.resize(slot + 1, None);
            }
            self.slots[slot] = Some(bound.clone());
            bound
        }

        fn foreign_actor(&self, slot: usize) -> Option<ActorId> {
            self.foreign.get(&slot).cloned()
        }

        fn foreign_reference(&self, actor: &ActorId) -> i32 {
            10_000 + actor.slot() as i32
        }
    }

    struct FakeBodies {
        bodies: HashMap<ActorId, BodyState>,
        bindings: Vec<(ActorId, usize)>,
        links: Vec<ActorId>,
    }

    impl WorldBodies for FakeBodies {
        fn read(&self, actor: &ActorId) -> Option<BodyState> {
            self.bodies.get(actor).cloned()
        }

        fn bind(&mut self, actor: &ActorId, slot: usize, _binding: QcBodyBinding) {
            self.bindings.push((actor.clone(), slot));
            self.bodies.insert(
                actor.clone(),
                BodyState {
                    origin: vec3(0.0, 0.0, 0.0),
                    angles: vec3(0.0, 0.0, 0.0),
                    velocity: vec3(0.0, 0.0, 0.0),
                    bounds: Bounds {
                        min: vec3(-1.0, -1.0, -1.0),
                        max: vec3(1.0, 1.0, 1.0),
                    },
                    ground: None,
                },
            );
        }

        fn link(&mut self, actor: &ActorId) {
            self.links.push(actor.clone());
        }
    }

    fn host() -> (IdentityOwner, QcWorldHost<FakeSlots, FakeBodies>) {
        let owner = IdentityOwner::create("world-host").unwrap();
        let world = owner.actor(0, 1);
        let player = owner.actor(1, 1);
        let slots = FakeSlots {
            owner_session: 0,
            live: vec![world.clone(), player.clone()],
            slots: vec![Some(world), Some(player)],
            foreign: HashMap::new(),
        };
        let bodies = FakeBodies {
            bodies: HashMap::new(),
            bindings: Vec::new(),
            links: Vec::new(),
        };
        let live = owner.actor(1, 1);
        let resolve: ActorResolver =
            std::rc::Rc::new(move |saved: &SavedActorId| (*saved == SavedActorId::from(&live)).then(|| live.clone()));
        (owner, QcWorldHost::new(slots, bodies, resolve))
    }

    #[test]
    fn link_bounds_expand_items_horizontally() {
        let origin = vec3(10.0, 20.0, 30.0);
        let bounds = Bounds {
            min: vec3(-8.0, -8.0, -8.0),
            max: vec3(8.0, 8.0, 8.0),
        };
        let item = qc_link_bounds(origin, bounds, FLAG_ITEM);
        assert_eq!(item.min, vec3(-13.0, -3.0, 21.0));
        assert_eq!(item.max, vec3(33.0, 43.0, 39.0));
        let other = qc_link_bounds(origin, bounds, 0);
        assert_eq!(other.min, vec3(1.0, 11.0, 21.0));
        assert_eq!(other.max, vec3(19.0, 29.0, 39.0));
    }

    #[test]
    fn reference_encodes_source_and_foreign_actors() {
        let (owner, host) = host();
        assert_eq!(host.reference(&owner.actor(1, 1)).unwrap(), 1);
        assert!(host.reference(&owner.actor(7, 1)).is_err());
    }

    #[test]
    fn actor_binds_bodies_once_and_admits() {
        let (owner, mut host) = host();
        let admitted = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let sink = admitted.clone();
        host.set_admit_hook(move |actor: &ActorId, slot: usize| {
            sink.borrow_mut().push((actor.clone(), slot));
        });
        let player = host.actor(1).unwrap();
        assert_eq!(player, owner.actor(1, 1));
        assert_eq!(host.bodies().bindings.len(), 1);
        let again = host.actor(1).unwrap();
        assert_eq!(again, player);
        assert_eq!(host.bodies().bindings.len(), 1);
        assert_eq!(*admitted.borrow(), vec![(player, 1)]);
        assert!(host.actor(9).is_err());
    }

    #[test]
    fn foreign_slots_return_surrogates() {
        let (owner, mut host) = host();
        let surrogate = owner.actor(2, 1);
        host.slots_mut().foreign.insert(1, surrogate.clone());
        host.slots_mut().live.push(surrogate.clone());
        assert_eq!(host.actor(1).unwrap(), surrogate);
        assert!(host.bodies().bindings.is_empty());
    }

    #[test]
    fn link_skips_world_and_free_slots() {
        let (_owner, mut host) = host();
        host.link(0).unwrap();
        host.link(5).unwrap();
        assert!(host.bodies().links.is_empty());
        host.link(1).unwrap();
        assert_eq!(host.bodies().links.len(), 1);
        assert_eq!(
            host.resolve_ground(&SavedActorId { slot: 1, generation: 1 })
                .unwrap()
                .slot(),
            1
        );
        assert!(host.resolve_ground(&SavedActorId { slot: 8, generation: 8 }).is_none());
    }
}
