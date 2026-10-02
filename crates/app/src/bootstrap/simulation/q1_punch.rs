//! Original-style Q1 punch angles with a local fallback.
//!
//! Port of Quake-Anthology-TS `src/app/bootstrap/simulation/q1-punch.ts`
//! (`Q1PlayerPunch`).
//!
//! One original-style Q1 field per player. Bound fields remain saved by
//! their source; only the fallback map checkpoints here.

use std::collections::HashMap;

use qa_content::q1::foundation::entity_services::drop_q1_punch;
use qa_core::identity::{ActorId, OwnedActor, SavedActorId};
use qa_core::math::{vec3, Vec3};
use qa_core::numeric::NumericOps;
use qa_world::save::records::{read_saved_actor, write_saved_actor};
use qa_world::save::shared::{read_vector, write_vector};
use qa_world::save::value::{arr, obj, SaveJson, SaveReader};
use qa_world::WorldError;
use thiserror::Error;

use super::physics::PhysicsActors;

/// Source-owned punch field for one actor.
///
/// Seam over the donor `Q1PunchOwner` interface (`q1-punch.ts`).
pub trait Q1PunchOwner {
    /// Read the bound field.
    fn read(&self) -> Vec3;
    /// Write the bound field.
    fn write(&mut self, angles: Vec3);
}

/// Punch field source for live actors.
///
/// Seam over the donor `source` callback (`q1-punch.ts`).
pub trait Q1PunchSource {
    /// Bound field for an actor, when the source owns one.
    fn owner(&self, actor: &ActorId) -> Option<Box<dyn Q1PunchOwner>>;
}

/// Punch angle failures.
#[derive(Debug, Error)]
pub enum Q1PunchError {
    /// Punch references a retired actor.
    #[error("Q1 punch references a retired actor")]
    RetiredActor,
}

/// Captured fallback punch entry.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1PunchEntry {
    /// Owning actor.
    pub actor: SavedActorId,
    /// Pending angles.
    pub angles: Vec3,
}

/// One original-style Q1 punch field per player.
pub struct Q1PlayerPunch<A: PhysicsActors, S: Q1PunchSource> {
    actors: A,
    source: S,
    local: HashMap<ActorId, Vec3>,
}

impl<A: PhysicsActors, S: Q1PunchSource> Q1PlayerPunch<A, S> {
    /// Create punch state over an actor registry and field source.
    pub fn new(actors: A, source: S) -> Self {
        Self {
            actors,
            source,
            local: HashMap::new(),
        }
    }

    /// Drop fallback state after the engine releases an actor.
    ///
    /// The donor subscribes to registry releases; the engine calls this
    /// instead so the borrow stays on the host side.
    pub fn release_actor(&mut self, actor: &ActorId) {
        self.local.remove(actor);
    }

    fn owner(&mut self, actor: &ActorId) -> Result<(OwnedActor, Option<Box<dyn Q1PunchOwner>>), Q1PunchError> {
        let Some(current) = self.actors.resolve_owned(actor) else {
            return Err(Q1PunchError::RetiredActor);
        };
        let mut source = self.source.owner(current.id());
        if let Some(pending) = self.local.remove(current.id()) {
            if let Some(owner) = source.as_mut() {
                owner.write(pending);
            } else {
                self.local.insert(current.id().clone(), pending);
            }
        }
        Ok((current, source))
    }

    /// Read punch angles for an actor.
    pub fn read(&mut self, actor: &ActorId) -> Result<Vec3, Q1PunchError> {
        let (current, source) = self.owner(actor)?;
        if let Some(source) = source {
            return Ok(source.read());
        }
        Ok(self.local.get(current.id()).copied().unwrap_or(vec3(0.0, 0.0, 0.0)))
    }

    /// Write punch angles for an actor.
    pub fn write(&mut self, actor: &ActorId, value: Vec3) -> Result<(), Q1PunchError> {
        // Donor Math.fround calls are subsumed by binary32 storage.
        let (current, mut source) = self.owner(actor)?;
        if let Some(owner) = source.as_mut() {
            owner.write(value);
        } else {
            self.local.insert(current.id().clone(), value);
        }
        Ok(())
    }

    /// Decay punch angles over an interval in seconds.
    pub fn advance(&mut self, actor: &ActorId, elapsed: f64, numeric: &NumericOps) -> Result<Vec3, Q1PunchError> {
        let current = self.read(actor)?;
        if current.x == 0.0 && current.y == 0.0 && current.z == 0.0 {
            return Ok(current);
        }
        let punch = drop_q1_punch(current, elapsed, numeric);
        self.write(actor, punch)?;
        Ok(punch)
    }

    /// Capture fallback entries as donor-shaped JSON.
    pub fn capture(&mut self) -> Result<SaveJson, Q1PunchError> {
        let ids: Vec<ActorId> = self.local.keys().cloned().collect();
        for id in ids {
            self.owner(&id)?;
        }
        let mut entries: Vec<(&ActorId, &Vec3)> = self.local.iter().collect();
        entries.sort_by_key(|(actor, _)| (actor.slot(), actor.generation()));
        Ok(arr(entries
            .iter()
            .map(|(actor, angles)| {
                obj(vec![
                    ("actor", write_saved_actor(SavedActorId::from(*actor))),
                    ("angles", write_vector(**angles)),
                ])
            })
            .collect()))
    }

    /// Restore fallback entries.
    pub fn restore(&mut self, reader: &SaveReader) -> Result<(), WorldError> {
        self.local.clear();
        if reader.value.is_none() {
            return Ok(());
        }
        let entries: Vec<(OwnedActor, Vec3)> = reader.list(|entry| {
            let saved = read_saved_actor(entry.field("actor"))?;
            let Some(actor) = self.actors.resolve_saved(saved) else {
                return Err(entry.fail("Saved Q1 punch has no unique fallback owner"));
            };
            if self.local.contains_key(actor.id()) || self.source.owner(actor.id()).is_some() {
                return Err(entry.fail("Saved Q1 punch has no unique fallback owner"));
            }
            Ok((actor, read_vector(entry.field("angles"))?))
        })?;
        for (actor, angles) in entries {
            if self.local.contains_key(actor.id()) || self.source.owner(actor.id()).is_some() {
                return Err(reader.fail("Saved Q1 punch has no unique fallback owner"));
            }
            self.local.insert(actor.id().clone(), angles);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use qa_core::numeric::Q2_DONOR_PROFILE;

    use super::*;
    use crate::bootstrap::simulation::physics::fakes::FakeActors;

    struct FakeOwner {
        angles: Rc<RefCell<Vec3>>,
    }

    impl Q1PunchOwner for FakeOwner {
        fn read(&self) -> Vec3 {
            *self.angles.borrow()
        }

        fn write(&mut self, angles: Vec3) {
            *self.angles.borrow_mut() = angles;
        }
    }

    struct FakeSource {
        bound: Option<ActorId>,
        angles: Rc<RefCell<Vec3>>,
    }

    impl Q1PunchSource for FakeSource {
        fn owner(&self, actor: &ActorId) -> Option<Box<dyn Q1PunchOwner>> {
            if self.bound.as_ref() == Some(actor) {
                Some(Box::new(FakeOwner {
                    angles: self.angles.clone(),
                }))
            } else {
                None
            }
        }
    }

    fn setup_punch(bound: bool) -> (OwnedActor, Rc<RefCell<Vec3>>, Q1PlayerPunch<FakeActors, FakeSource>) {
        let mut actors = FakeActors::new();
        let actor = actors.mint();
        let angles = Rc::new(RefCell::new(vec3(0.0, 0.0, 0.0)));
        let source = FakeSource {
            bound: bound.then(|| actor.id().clone()),
            angles: angles.clone(),
        };
        (actor, angles, Q1PlayerPunch::new(actors, source))
    }

    #[test]
    fn fallback_round_trips_locally() {
        let (actor, _, mut punch) = setup_punch(false);
        assert_eq!(punch.read(actor.id()).expect("read"), vec3(0.0, 0.0, 0.0));
        punch.write(actor.id(), vec3(1.0, 2.0, 3.0)).expect("write");
        assert_eq!(punch.read(actor.id()).expect("read"), vec3(1.0, 2.0, 3.0));
    }

    #[test]
    fn bound_source_flushes_pending() {
        let (actor, angles, mut punch) = setup_punch(false);
        punch.write(actor.id(), vec3(4.0, 5.0, 6.0)).expect("write");
        // Bind the source after the write; the next access flushes.
        punch.source.bound = Some(actor.id().clone());
        assert_eq!(punch.read(actor.id()).expect("read"), vec3(4.0, 5.0, 6.0));
        assert_eq!(*angles.borrow(), vec3(4.0, 5.0, 6.0));
        punch.write(actor.id(), vec3(7.0, 0.0, 0.0)).expect("write");
        assert_eq!(*angles.borrow(), vec3(7.0, 0.0, 0.0));
    }

    #[test]
    fn advance_decays_through_source_drop() {
        let (actor, _, mut punch) = setup_punch(false);
        punch.write(actor.id(), vec3(10.0, 0.0, 0.0)).expect("write");
        let numeric = NumericOps::select(Q2_DONOR_PROFILE).expect("numeric");
        let expected = drop_q1_punch(vec3(10.0, 0.0, 0.0), 0.1, &numeric);
        assert_eq!(punch.advance(actor.id(), 0.1, &numeric).expect("advance"), expected);
        assert_eq!(punch.read(actor.id()).expect("read"), expected);
        // Zero punch stays zero without touching the source.
        let (idle, _, mut punch) = setup_punch(false);
        assert_eq!(
            punch.advance(idle.id(), 0.1, &numeric).expect("advance"),
            vec3(0.0, 0.0, 0.0)
        );
    }

    #[test]
    fn capture_restore_round_trips_fallback() {
        let (actor, _, mut punch) = setup_punch(false);
        punch.write(actor.id(), vec3(1.0, 2.0, 3.0)).expect("write");
        let json = punch.capture().expect("capture");
        let (restored, _, mut punch) = setup_punch(false);
        assert_eq!(restored.id().slot(), actor.id().slot());
        punch.restore(&SaveReader::new(&json)).expect("restore");
        assert_eq!(punch.read(restored.id()).expect("read"), vec3(1.0, 2.0, 3.0));
    }

    #[test]
    fn restore_rejects_bound_and_duplicate_owners() {
        let (actor, _, mut punch) = setup_punch(true);
        let json = arr(vec![obj(vec![
            ("actor", write_saved_actor(SavedActorId::from(actor.id()))),
            ("angles", write_vector(vec3(1.0, 0.0, 0.0))),
        ])]);
        assert!(punch.restore(&SaveReader::new(&json)).is_err());
        let (actor, _, mut punch) = setup_punch(false);
        let json = arr(vec![
            obj(vec![
                ("actor", write_saved_actor(SavedActorId::from(actor.id()))),
                ("angles", write_vector(vec3(1.0, 0.0, 0.0))),
            ]),
            obj(vec![
                ("actor", write_saved_actor(SavedActorId::from(actor.id()))),
                ("angles", write_vector(vec3(2.0, 0.0, 0.0))),
            ]),
        ]);
        assert!(punch.restore(&SaveReader::new(&json)).is_err());
    }

    #[test]
    fn retired_actor_fails_and_release_clears() {
        let (actor, _, mut punch) = setup_punch(false);
        punch.write(actor.id(), vec3(1.0, 0.0, 0.0)).expect("write");
        punch.release_actor(actor.id());
        let json = punch.capture().expect("capture");
        assert_eq!(format!("{json:?}"), "Array([])");
        drop(punch);
        let (actor, _, mut punch) = setup_punch(false);
        punch.actors.kill(actor.id());
        assert!(matches!(punch.read(actor.id()), Err(Q1PunchError::RetiredActor)));
    }
}
