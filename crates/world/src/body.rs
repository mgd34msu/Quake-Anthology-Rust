//! Actor bodies ported from `src/world/actors/body.ts`. Bounds visible to
//! spatial queries remain those captured by the last source-defined link;
//! a source may link a snapped collision origin while movement keeps its
//! authoritative precision.

use std::collections::HashMap;

use qa_core::identity::{ActorId, OwnedActor};
use qa_core::math::{Bounds, Vec3};

use crate::registry::ActorRegistry;
use crate::WorldError;

/// Rigid-body state: origin, angles, velocity, local bounds, ground.
#[derive(Debug, Clone, PartialEq)]
pub struct BodyState {
    /// World origin.
    pub origin: Vec3,
    /// Euler angles in degrees.
    pub angles: Vec3,
    /// Velocity in units per second.
    pub velocity: Vec3,
    /// Local collision bounds.
    pub bounds: Bounds,
    /// Ground actor, if any.
    pub ground: Option<ActorId>,
}

/// How an attached body follows its anchor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BodyFollow {
    /// Anchor origin plus an offset.
    Translation {
        /// Local offset.
        offset: Vec3,
    },
    /// Anchor bounds center.
    Center,
    /// Anchor bounds minimum plus an offset.
    BoundsMin {
        /// Local offset.
        offset: Vec3,
    },
}

/// Attached bodies share their anchor's lifetime.
#[derive(Debug, Clone, PartialEq)]
pub struct BodyAttachment {
    /// Anchor actor.
    pub anchor: ActorId,
    /// Follow rule.
    pub follow: BodyFollow,
}

/// Last linked snapshot plus its absolute bounds and link count.
#[derive(Debug, Clone, PartialEq)]
pub struct LinkedBody {
    /// Linked actor.
    pub actor: ActorId,
    /// Linked state snapshot.
    pub state: BodyState,
    /// Absolute bounds at link time.
    pub absolute_bounds: Bounds,
    /// Link count.
    pub link_count: u64,
}

#[derive(Debug)]
struct Record {
    state: BodyState,
    linked: Option<LinkedBody>,
    link_count: u64,
}

/// Untranslated box policy: BSP rotation and family link padding are
/// supplied by the provider, exactly as in the donor.
#[must_use]
pub fn translated_body_bounds(state: &BodyState) -> Bounds {
    Bounds {
        min: Vec3 {
            x: state.origin.x + state.bounds.min.x,
            y: state.origin.y + state.bounds.min.y,
            z: state.origin.z + state.bounds.min.z,
        },
        max: Vec3 {
            x: state.origin.x + state.bounds.max.x,
            y: state.origin.y + state.bounds.max.y,
            z: state.origin.z + state.bounds.max.z,
        },
    }
}

/// Shared body table.
#[derive(Debug, Default)]
pub struct BodyTable {
    records: HashMap<ActorId, Record>,
    attachments: HashMap<ActorId, BodyAttachment>,
}

impl BodyTable {
    /// Empty table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a local body.
    pub fn create(
        &mut self,
        registry: &ActorRegistry,
        actor: &OwnedActor,
        initial: BodyState,
    ) -> Result<(), WorldError> {
        registry.assert_owned(actor)?;
        if self.records.contains_key(actor.id()) {
            return Err(WorldError::BodyExists);
        }
        self.records.insert(
            actor.id().clone(),
            Record {
                state: initial,
                linked: None,
                link_count: 0,
            },
        );
        Ok(())
    }

    /// Whether a live actor has a body, without cloning state.
    #[must_use]
    pub fn has_body(&self, registry: &ActorRegistry, actor: &ActorId) -> bool {
        if !registry.is_live(actor) {
            return false;
        }
        self.records.contains_key(actor)
    }

    /// Read a live body.
    #[must_use]
    pub fn read(&self, registry: &ActorRegistry, actor: &ActorId) -> Option<BodyState> {
        if !registry.is_live(actor) {
            return None;
        }
        self.records.get(actor).map(|record| record.state.clone())
    }

    /// Write a body, creating it when absent.
    pub fn write(&mut self, registry: &ActorRegistry, actor: &OwnedActor, state: BodyState) -> Result<(), WorldError> {
        registry.assert_owned(actor)?;
        match self.records.get_mut(actor.id()) {
            Some(record) => {
                record.state = state;
                Ok(())
            }
            None => self.create(registry, actor, state),
        }
    }

    /// Attach a body to an anchor body, rejecting cycles.
    pub fn attach(
        &mut self,
        registry: &ActorRegistry,
        actor: &OwnedActor,
        attachment: BodyAttachment,
    ) -> Result<(), WorldError> {
        registry.assert_owned(actor)?;
        if !self.records.contains_key(actor.id()) {
            return Err(WorldError::BodyMissing);
        }
        if !registry.is_live(&attachment.anchor) || !self.records.contains_key(&attachment.anchor) {
            return Err(WorldError::AnchorMissing);
        }
        let mut ancestor = Some(attachment.anchor.clone());
        while let Some(current) = ancestor {
            if current == *actor.id() {
                return Err(WorldError::AttachCycle);
            }
            ancestor = self.attachments.get(&current).map(|next| next.anchor.clone());
        }
        self.attachments.insert(actor.id().clone(), attachment);
        Ok(())
    }

    /// Detach a body.
    pub fn detach(&mut self, registry: &ActorRegistry, actor: &OwnedActor) -> Result<(), WorldError> {
        registry.assert_owned(actor)?;
        self.attachments.remove(actor.id());
        Ok(())
    }

    /// Current attachment of a live body.
    #[must_use]
    pub fn attachment(&self, registry: &ActorRegistry, actor: &ActorId) -> Option<BodyAttachment> {
        if !registry.is_live(actor) || !self.records.contains_key(actor) {
            return None;
        }
        self.attachments.get(actor).cloned()
    }

    /// Last linked snapshot of a live body.
    #[must_use]
    pub fn linked(&self, registry: &ActorRegistry, actor: &ActorId) -> Option<LinkedBody> {
        if !registry.is_live(actor) {
            return None;
        }
        self.records.get(actor)?.linked.clone()
    }

    /// Link a body, capturing its spatial snapshot.
    pub fn link(
        &mut self,
        registry: &ActorRegistry,
        actor: &OwnedActor,
        origin: Option<Vec3>,
    ) -> Result<(), WorldError> {
        registry.assert_owned(actor)?;
        let record = self.records.get_mut(actor.id()).ok_or(WorldError::BodyMissing)?;
        let mut state = record.state.clone();
        if let Some(origin) = origin {
            state.origin = origin;
        }
        let absolute_bounds = translated_body_bounds(&state);
        record.link_count += 1;
        let link_count = record.link_count;
        record.linked = Some(LinkedBody {
            actor: actor.id().clone(),
            state,
            absolute_bounds,
            link_count,
        });
        Ok(())
    }

    /// Unlink a body without touching its field state.
    pub fn unlink(&mut self, registry: &ActorRegistry, actor: &OwnedActor) -> Result<(), WorldError> {
        registry.assert_owned(actor)?;
        if let Some(record) = self.records.get_mut(actor.id()) {
            record.linked = None;
        }
        Ok(())
    }

    /// Install saved spatial state directly, without link callbacks.
    pub fn restore_link_state(
        &mut self,
        registry: &ActorRegistry,
        actor: &OwnedActor,
        link_count: u64,
        linked: Option<(BodyState, Bounds)>,
    ) -> Result<(), WorldError> {
        registry.assert_owned(actor)?;
        let record = self.records.get_mut(actor.id()).ok_or(WorldError::BodyMissing)?;
        if linked.is_some() && link_count == 0 {
            return Err(WorldError::BadLinkCount);
        }
        record.link_count = link_count;
        record.linked = linked.map(|(state, absolute_bounds)| LinkedBody {
            actor: actor.id().clone(),
            state,
            absolute_bounds,
            link_count,
        });
        Ok(())
    }

    /// Transport attachments at a committed execution boundary. Spatial
    /// publication never invokes source touches.
    pub fn transport_attachments(&mut self, registry: &ActorRegistry) {
        let actors: Vec<ActorId> = self.attachments.keys().cloned().collect();
        let mut transported: Vec<ActorId> = Vec::new();
        for actor in actors {
            self.transport_one(registry, &actor, &mut transported);
        }
    }

    fn transport_one(&mut self, registry: &ActorRegistry, actor: &ActorId, transported: &mut Vec<ActorId>) {
        if transported.contains(actor) {
            return;
        }
        transported.push(actor.clone());
        let attachment = match self.attachments.get(actor) {
            Some(attachment) => attachment.clone(),
            None => return,
        };
        if !self.records.contains_key(&attachment.anchor) {
            return;
        }
        self.transport_one(registry, &attachment.anchor, transported);
        let (anchor, body) = match (self.records.get(&attachment.anchor), self.records.get(actor)) {
            (Some(anchor), Some(body)) => (anchor.state.clone(), body.state.clone()),
            _ => return,
        };
        let offset = match attachment.follow {
            BodyFollow::Translation { offset } => offset,
            BodyFollow::Center => Vec3 {
                x: (anchor.bounds.min.x + anchor.bounds.max.x) * 0.5,
                y: (anchor.bounds.min.y + anchor.bounds.max.y) * 0.5,
                z: (anchor.bounds.min.z + anchor.bounds.max.z) * 0.5,
            },
            BodyFollow::BoundsMin { offset } => Vec3 {
                x: anchor.bounds.min.x + offset.x,
                y: anchor.bounds.min.y + offset.y,
                z: anchor.bounds.min.z + offset.z,
            },
        };
        let origin = Vec3 {
            x: anchor.origin.x + offset.x,
            y: anchor.origin.y + offset.y,
            z: anchor.origin.z + offset.z,
        };
        if origin == body.origin {
            return;
        }
        let Some(owned) = registry.resolve_owned(actor) else {
            return;
        };
        let mut next = body;
        next.origin = origin;
        if self.write(registry, &owned, next).is_ok() {
            let _ = self.link(registry, &owned, None);
        }
    }

    /// Drop a released actor, returning attached children for the simulation
    /// to release from the registry.
    pub fn release_actor(&mut self, actor: &ActorId) -> Vec<ActorId> {
        self.attachments.remove(actor);
        let children: Vec<ActorId> = self
            .attachments
            .iter()
            .filter_map(|(child, attachment)| {
                if attachment.anchor == *actor {
                    Some(child.clone())
                } else {
                    None
                }
            })
            .collect();
        for child in &children {
            self.attachments.remove(child);
            self.records.remove(child);
        }
        self.records.remove(actor);
        children
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::{IdentityOwner, ProviderId};
    use qa_core::math::vec3;

    fn setup() -> (ActorRegistry, BodyTable, OwnedActor, OwnedActor) {
        let owner = IdentityOwner::create("test").unwrap();
        let mut registry = ActorRegistry::new(owner, 8).unwrap();
        let provider = ProviderId::new("q3", "game");
        let anchor = registry.allocate(provider.clone(), "q3:anchor").unwrap();
        let child = registry.allocate(provider, "q3:child").unwrap();
        (registry, BodyTable::new(), anchor, child)
    }

    fn body_at(x: f32) -> BodyState {
        BodyState {
            origin: vec3(x, 0.0, 0.0),
            angles: vec3(0.0, 0.0, 0.0),
            velocity: vec3(0.0, 0.0, 0.0),
            bounds: qa_core::math::Bounds {
                min: vec3(-8.0, -8.0, -8.0),
                max: vec3(8.0, 8.0, 8.0),
            },
            ground: None,
        }
    }

    #[test]
    fn link_captures_absolute_bounds_and_stale_writes_stay_unlinked() {
        let (registry, mut bodies, anchor, _) = setup();
        bodies.create(&registry, &anchor, body_at(10.0)).unwrap();
        bodies.link(&registry, &anchor, None).unwrap();
        let linked = bodies.linked(&registry, anchor.id()).unwrap();
        assert_eq!(linked.link_count, 1);
        assert_eq!(linked.absolute_bounds.min.x, 2.0);
        assert_eq!(linked.absolute_bounds.max.x, 18.0);
        bodies.write(&registry, &anchor, body_at(100.0)).unwrap();
        let stale = bodies.linked(&registry, anchor.id()).unwrap();
        assert_eq!(stale.absolute_bounds.min.x, 2.0);
        bodies.unlink(&registry, &anchor).unwrap();
        assert!(bodies.linked(&registry, anchor.id()).is_none());
    }

    #[test]
    fn attachments_follow_and_reject_cycles() {
        let (registry, mut bodies, anchor, child) = setup();
        bodies.create(&registry, &anchor, body_at(10.0)).unwrap();
        bodies.create(&registry, &child, body_at(0.0)).unwrap();
        bodies
            .attach(
                &registry,
                &child,
                BodyAttachment {
                    anchor: anchor.id().clone(),
                    follow: BodyFollow::Translation {
                        offset: vec3(0.0, 5.0, 0.0),
                    },
                },
            )
            .unwrap();
        bodies.transport_attachments(&registry);
        let moved = bodies.read(&registry, child.id()).unwrap();
        assert_eq!(moved.origin, vec3(10.0, 5.0, 0.0));
        assert!(bodies.linked(&registry, child.id()).is_some());
        let cycle = bodies.attach(
            &registry,
            &anchor,
            BodyAttachment {
                anchor: child.id().clone(),
                follow: BodyFollow::Center,
            },
        );
        assert_eq!(cycle, Err(WorldError::AttachCycle));
    }

    #[test]
    fn center_follow_uses_anchor_bounds_midpoint() {
        let (registry, mut bodies, anchor, child) = setup();
        bodies.create(&registry, &anchor, body_at(0.0)).unwrap();
        bodies.create(&registry, &child, body_at(50.0)).unwrap();
        bodies
            .attach(
                &registry,
                &child,
                BodyAttachment {
                    anchor: anchor.id().clone(),
                    follow: BodyFollow::Center,
                },
            )
            .unwrap();
        bodies.transport_attachments(&registry);
        let moved = bodies.read(&registry, child.id()).unwrap();
        assert_eq!(moved.origin, vec3(0.0, 0.0, 0.0));
    }
}
