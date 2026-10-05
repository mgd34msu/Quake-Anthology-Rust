//! Trigger volumes ported from `src/world/actors/triggers.ts`
//! (`touchQ1Triggers`) with the touch canonical rules from
//! `src/world/actors/callbacks.ts`. Q1 visits live spatial links; nested
//! touches may remove or relink either actor, so every candidate is
//! re-validated and the visit stops when the mover is gone.

use qa_core::identity::ActorId;

use crate::body::BodyTable;
use crate::registry::ActorRegistry;
use crate::spatial::{bounds_intersect, CollisionRole, SpatialIndex, Visit};
use crate::WorldError;

/// Touch contact between a trigger and a mover.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TouchContact {
    /// Trigger actor.
    pub trigger: ActorId,
    /// Moving actor touched.
    pub other: ActorId,
}

/// Trigger membership table: which actors are trigger volumes.
#[derive(Debug, Clone, Default)]
pub struct TriggerTable {
    triggers: Vec<ActorId>,
}

impl TriggerTable {
    /// Empty table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Mark an actor as a trigger volume.
    pub fn mark(&mut self, registry: &ActorRegistry, actor: &ActorId) -> Result<(), WorldError> {
        if !registry.is_live(actor) {
            return Err(WorldError::StaleActor);
        }
        if !self.triggers.iter().any(|id| id == actor) {
            self.triggers.push(actor.clone());
        }
        Ok(())
    }

    /// Clear trigger membership.
    pub fn unmark(&mut self, actor: &ActorId) {
        self.triggers.retain(|id| id != actor);
    }

    /// Whether an actor is a marked trigger.
    #[must_use]
    pub fn is_trigger(&self, actor: &ActorId) -> bool {
        self.triggers.iter().any(|id| id == actor)
    }

    /// Marked triggers in mark order. The scene link step snapshots this
    /// once per tick so body classification stays linear.
    pub fn iter(&self) -> impl Iterator<Item = &ActorId> {
        self.triggers.iter()
    }

    /// Forget memberships for released actors.
    pub fn retain_live(&mut self, registry: &ActorRegistry) {
        self.triggers.retain(|id| registry.is_live(id));
    }

    /// Checkpoint marked triggers as slots.
    #[must_use]
    pub fn checkpoint(&self) -> Vec<(u32, u32)> {
        self.triggers.iter().map(|id| (id.slot(), id.generation())).collect()
    }

    /// Restore marked triggers against a fresh registry.
    pub fn restore(&mut self, registry: &ActorRegistry, saved: &[(u32, u32)]) -> Result<(), WorldError> {
        self.triggers.clear();
        for (slot, generation) in saved {
            let found = registry
                .live_id(*slot, *generation)
                .ok_or_else(|| WorldError::BadSave("Trigger names a missing actor".to_string()))?;
            self.triggers.push(found);
        }
        Ok(())
    }
}

/// Visit trigger volumes overlapped by `mover` and call `touch` for each
/// live trigger, mirroring `touchQ1Triggers`: the mover must be linked and
/// live, candidates must carry the trigger role, differ from the mover,
/// be live triggers, and still intersect after re-resolution.
pub fn touch_q1_triggers(
    registry: &ActorRegistry,
    bodies: &BodyTable,
    spatial: &SpatialIndex,
    triggers: &TriggerTable,
    mover: &ActorId,
    touch: &mut dyn FnMut(TouchContact),
) {
    let Some(linked) = bodies.linked(registry, mover) else {
        return;
    };
    if !registry.is_live(mover) {
        return;
    }
    let bounds = linked.absolute_bounds;
    let mut visitor = |candidate: &crate::spatial::SpatialActor| {
        if !registry.is_live(mover) {
            return Visit::Stop;
        }
        let id = &candidate.body.actor;
        if candidate.collision.role == CollisionRole::Trigger && id != mover {
            let trigger = registry.resolve_owned(id);
            let current = bodies.linked(registry, id);
            let moving = bodies.linked(registry, mover);
            if let (Some(trigger), Some(current), Some(moving)) = (trigger, current, moving) {
                if triggers.is_trigger(trigger.id())
                    && bounds_intersect(&current.absolute_bounds, &moving.absolute_bounds)
                {
                    touch(TouchContact {
                        trigger: trigger.id().clone(),
                        other: mover.clone(),
                    });
                }
            }
        }
        if registry.is_live(mover) {
            Visit::Continue
        } else {
            Visit::Stop
        }
    };
    spatial.visit(&bounds, &mut visitor);
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::{IdentityOwner, ProviderId};
    use qa_core::math::{vec3, Bounds};

    use crate::body::BodyState;
    use crate::spatial::{ActorCollision, CollisionFamily, CollisionShape};

    fn body_at(origin: qa_core::math::Vec3) -> BodyState {
        BodyState {
            origin,
            angles: vec3(0.0, 0.0, 0.0),
            velocity: vec3(0.0, 0.0, 0.0),
            bounds: Bounds {
                min: vec3(-8.0, -8.0, -8.0),
                max: vec3(8.0, 8.0, 8.0),
            },
            ground: None,
        }
    }

    fn collision(role: CollisionRole) -> ActorCollision {
        ActorCollision {
            family: CollisionFamily::Q1,
            shape: CollisionShape::Box,
            contents: 0,
            owner: None,
            role,
            monster: false,
            dead_monster: false,
            q1_corpse: false,
            q3_owner: None,
        }
    }

    fn world_bounds() -> Bounds {
        Bounds {
            min: vec3(-1024.0, -1024.0, -1024.0),
            max: vec3(1024.0, 1024.0, 1024.0),
        }
    }

    #[test]
    fn overlapping_trigger_fires_once() {
        let owner = IdentityOwner::create("test").unwrap();
        let mut registry = ActorRegistry::new(owner, 8).unwrap();
        let mut bodies = BodyTable::new();
        let mut spatial = SpatialIndex::new(&world_bounds());
        let mut triggers = TriggerTable::new();
        let provider = ProviderId::new("q1", "game");
        let mover = registry.allocate(provider.clone(), "q1:player").unwrap();
        let volume = registry.allocate(provider, "q1:trigger").unwrap();
        bodies.create(&registry, &mover, body_at(vec3(0.0, 0.0, 0.0))).unwrap();
        bodies.create(&registry, &volume, body_at(vec3(4.0, 0.0, 0.0))).unwrap();
        bodies.link(&registry, &mover, None).unwrap();
        bodies.link(&registry, &volume, None).unwrap();
        triggers.mark(&registry, volume.id()).unwrap();
        for (actor, role) in [
            (mover.id(), CollisionRole::Solid),
            (volume.id(), CollisionRole::Trigger),
        ] {
            let linked = bodies.linked(&registry, actor).unwrap();
            spatial.link(&linked, &collision(role));
        }
        let mut contacts = Vec::new();
        touch_q1_triggers(&registry, &bodies, &spatial, &triggers, mover.id(), &mut |contact| {
            contacts.push(contact);
        });
        assert_eq!(contacts.len(), 1);
        assert_eq!(contacts[0].trigger, *volume.id());
        assert_eq!(contacts[0].other, *mover.id());
    }

    #[test]
    fn unmarked_or_self_volumes_do_not_fire() {
        let owner = IdentityOwner::create("test").unwrap();
        let mut registry = ActorRegistry::new(owner, 8).unwrap();
        let mut bodies = BodyTable::new();
        let mut spatial = SpatialIndex::new(&world_bounds());
        let triggers = TriggerTable::new();
        let provider = ProviderId::new("q1", "game");
        let mover = registry.allocate(provider.clone(), "q1:player").unwrap();
        let volume = registry.allocate(provider, "q1:trigger").unwrap();
        bodies.create(&registry, &mover, body_at(vec3(0.0, 0.0, 0.0))).unwrap();
        bodies.create(&registry, &volume, body_at(vec3(4.0, 0.0, 0.0))).unwrap();
        bodies.link(&registry, &mover, None).unwrap();
        bodies.link(&registry, &volume, None).unwrap();
        for (actor, role) in [
            (mover.id(), CollisionRole::Trigger),
            (volume.id(), CollisionRole::Trigger),
        ] {
            let linked = bodies.linked(&registry, actor).unwrap();
            spatial.link(&linked, &collision(role));
        }
        let mut contacts = Vec::new();
        touch_q1_triggers(&registry, &bodies, &spatial, &triggers, mover.id(), &mut |contact| {
            contacts.push(contact);
        });
        assert!(contacts.is_empty());
    }

    #[test]
    fn trigger_checkpoint_round_trip() {
        let owner = IdentityOwner::create("test").unwrap();
        let mut registry = ActorRegistry::new(owner, 8).unwrap();
        let provider = ProviderId::new("q1", "game");
        let actor = registry.allocate(provider, "q1:trigger").unwrap();
        let mut triggers = TriggerTable::new();
        triggers.mark(&registry, actor.id()).unwrap();
        let saved = triggers.checkpoint();
        let mut restored = TriggerTable::new();
        restored.restore(&registry, &saved).unwrap();
        assert!(restored.is_trigger(actor.id()));
        assert!(restored.restore(&registry, &[(99, 1)]).is_err());
    }
}
