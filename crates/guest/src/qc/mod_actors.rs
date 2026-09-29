//! QuakeC mod-owned actors: private edicts with scheduled thinks.
//!
//! Ported from `src/compat/qc/mod-actors.ts`.
//!
//! Local mirrors (owned by other workers' modules, noted here):
//! `QcActorMachine` mirrors the entity-word surface of `QcMachine` from
//! `src/compat/qc/machine.ts`; `ModActorScheduler` mirrors the `FrameScheduler`
//! usage from `src/world/scheduler.ts`; `QcPainReaction`/`QcDeathReaction`
//! mirror the reactions from `src/contracts/world.ts`; `QcModActorHost`
//! mirrors the `ModHostServices` actor/callback/body surface from
//! `src/world/session/mods.ts`.
//!
//! Adaptation: the donor threads a `think` closure through each physics step
//! so movement controls think timing; this port fires due thinks immediately
//! before the step instead, matching Quake think-before-movement order while
//! keeping the scheduler inside this module.

use std::collections::HashMap;

use qa_core::identity::{ActorId, OwnedActor, ProviderId};
use qa_core::time::{FrameContext, SourceTime};

use crate::error::GuestError;

/// Entity-word surface for think scheduling.
pub trait QcActorMachine {
    /// Read a slot integer.
    fn slot_int(&self, slot: u32, offset: i32) -> Result<i32, GuestError>;
    /// Read a slot float.
    fn slot_float(&self, slot: u32, offset: i32) -> Result<f32, GuestError>;
    /// Write a slot float.
    fn set_slot_float(&mut self, slot: u32, offset: i32, value: f32) -> Result<(), GuestError>;
}

/// Pain reaction delivered to the host.
#[derive(Debug, Clone)]
pub struct QcPainReaction {
    /// Reacting actor.
    pub target: OwnedActor,
    /// Attacker, if any.
    pub attacker: Option<ActorId>,
    /// Damage applied.
    pub damage: f64,
    /// Knockback kick.
    pub kick: f64,
}

/// Death reaction delivered to the host.
#[derive(Debug, Clone)]
pub struct QcDeathReaction {
    /// Reacting actor.
    pub target: OwnedActor,
    /// Attacker, if any.
    pub attacker: Option<ActorId>,
    /// Inflictor, if any.
    pub inflictor: Option<ActorId>,
    /// Damage applied.
    pub damage: f64,
    /// Knockback kick.
    pub kick: f64,
}

/// Host callbacks and services for mod-owned actors.
pub trait QcModActorHost {
    /// Current source time.
    fn now(&self) -> SourceTime;
    /// Bind a body for an admitted actor.
    fn bind_body(&mut self, actor: &OwnedActor, slot: u32);
    /// Unlink a released actor's body.
    fn unlink_body(&mut self, actor: &OwnedActor);
    /// Rebind a restored actor's body.
    fn rebind_body(&mut self, actor: &OwnedActor, slot: u32);
    /// Observe admission.
    fn admitted(&mut self, actor: &OwnedActor, slot: u32);
    /// Observe retirement.
    fn retired(&mut self, actor: &OwnedActor);
    /// Remember an actor/slot projection.
    fn remember(&mut self, actor: &ActorId, slot: u32);
    /// Step an actor for one frame.
    fn step(&mut self, actor: &OwnedActor, frame: &FrameContext);
    /// Dispatch touch.
    fn touch(&mut self, actor: &OwnedActor, other: &ActorId);
    /// Dispatch use.
    fn use_on(&mut self, actor: &OwnedActor, other: Option<&ActorId>, activator: Option<&ActorId>);
    /// Dispatch pain.
    fn pain(&mut self, reaction: &QcPainReaction);
    /// Dispatch death.
    fn die(&mut self, reaction: &QcDeathReaction);
    /// Invoke a source function for an actor.
    fn invoke(&mut self, actor: &OwnedActor, function_index: i32, frame: &FrameContext);
    /// Run a reserved client slot; true when consumed.
    fn client_frame(&mut self, _slot: u32, _frame: &FrameContext) -> bool {
        false
    }
}

/// Due-time scheduler for mod-actor thinks.
#[derive(Debug, Default)]
pub struct ModActorScheduler {
    due: HashMap<ActorId, f64>,
}

impl ModActorScheduler {
    /// Empty scheduler.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Schedule an actor think.
    pub fn schedule(&mut self, actor: &ActorId, due: f64) {
        self.due.insert(actor.clone(), due);
    }

    /// Cancel an actor think.
    pub fn cancel(&mut self, actor: &ActorId) {
        self.due.remove(actor);
    }

    /// Due time, if scheduled.
    #[must_use]
    pub fn due(&self, actor: &ActorId) -> Option<f64> {
        self.due.get(actor).copied()
    }

    /// Take a due think, if its time arrived.
    pub fn take_due(&mut self, actor: &ActorId, now: f64) -> bool {
        if self.due.get(actor).is_some_and(|due| *due <= now) {
            self.due.remove(actor);
            true
        } else {
            false
        }
    }
}

/// Mod-owned source actors with private edicts and deadlines.
pub struct QcModActors<M, H> {
    machine: M,
    host: H,
    provider: ProviderId,
    think_offset: Option<i32>,
    nextthink_offset: Option<i32>,
    first_dynamic_slot: u32,
    slots: Vec<Option<OwnedActor>>,
    owned: HashMap<ActorId, u32>,
    scheduler: ModActorScheduler,
    closed: bool,
}

impl<M: QcActorMachine, H: QcModActorHost> QcModActors<M, H> {
    /// Build over `capacity` entity slots (slot 0 is reserved for world).
    pub fn new(
        machine: M,
        host: H,
        provider: ProviderId,
        capacity: usize,
        first_dynamic_slot: u32,
        think_offset: Option<i32>,
        nextthink_offset: Option<i32>,
    ) -> Self {
        Self {
            machine,
            host,
            provider,
            think_offset,
            nextthink_offset,
            first_dynamic_slot: first_dynamic_slot.max(1),
            slots: vec![None; capacity.max(1)],
            owned: HashMap::new(),
            scheduler: ModActorScheduler::new(),
            closed: false,
        }
    }

    /// Borrow the host.
    #[must_use]
    pub fn host(&self) -> &H {
        &self.host
    }

    /// Borrow the host mutably.
    pub fn host_mut(&mut self) -> &mut H {
        &mut self.host
    }

    /// Borrow the scheduler.
    #[must_use]
    pub fn scheduler(&self) -> &ModActorScheduler {
        &self.scheduler
    }

    /// Owning provider.
    #[must_use]
    pub fn provider(&self) -> &ProviderId {
        &self.provider
    }

    /// Actor at a slot, if any.
    #[must_use]
    pub fn at(&self, slot: u32) -> Option<&OwnedActor> {
        self.slots.get(slot as usize).and_then(Option::as_ref)
    }

    /// Slot of an owned actor, if any.
    #[must_use]
    pub fn slot_of(&self, actor: &ActorId) -> Option<u32> {
        self.owned.get(actor).copied()
    }

    /// Spawn an owned actor into a free slot.
    pub fn spawn(&mut self, actor: OwnedActor) -> Result<u32, GuestError> {
        if self.closed {
            return Err(GuestError::invalid("Mod actors are closed"));
        }
        if actor.owner() != &self.provider {
            return Err(GuestError::invalid("Mod spawn requires its own provider actor"));
        }
        let slot = (self.first_dynamic_slot as usize..self.slots.len())
            .find(|slot| self.slots[*slot].is_none())
            .ok_or_else(|| GuestError::invalid("Gameplay mod source edicts exhausted"))?;
        let slot = slot as u32;
        self.slots[slot as usize] = Some(actor.clone());
        self.owned.insert(actor.id().clone(), slot);
        self.host.remember(actor.id(), slot);
        self.host.bind_body(&actor, slot);
        self.admit(actor, slot);
        Ok(slot)
    }

    /// Remove an owned actor.
    pub fn remove(&mut self, actor: &ActorId) -> Result<(), GuestError> {
        let slot = self
            .owned
            .get(actor)
            .copied()
            .filter(|slot| self.at(*slot).is_some_and(|current| current.id() == actor))
            .ok_or_else(|| {
                GuestError::invalid("Mod remove requires its own actor; foreign source removal needs its owner continuation")
            })?;
        let owned = self.slots[slot as usize].take().ok_or_else(|| GuestError::invalid("Mod actor slot is already free"))?;
        self.owned.remove(actor);
        self.scheduler.cancel(actor);
        self.host.unlink_body(&owned);
        self.host.retired(&owned);
        Ok(())
    }

    /// Admit an actor with bound callbacks.
    fn admit(&mut self, actor: OwnedActor, slot: u32) {
        self.host.admitted(&actor, slot);
    }

    /// Schedule an actor think from its `nextthink` word.
    pub fn schedule(&mut self, actor: &ActorId) -> Result<(), GuestError> {
        let slot = self
            .owned
            .get(actor)
            .copied()
            .ok_or_else(|| GuestError::invalid("Mod scheduling requires its own source actor"))?;
        let nextthink = self
            .nextthink_offset
            .ok_or_else(|| GuestError::invalid("Mod scheduling requires a nextthink binding"))?;
        let due = f64::from(self.machine.slot_float(slot, nextthink)?);
        if due <= 0.0 {
            self.scheduler.cancel(actor);
        } else {
            self.scheduler.schedule(actor, due);
        }
        Ok(())
    }

    /// Fire one think dispatch.
    fn fire_think(&mut self, actor: &OwnedActor, frame: &FrameContext) -> Result<(), GuestError> {
        let (think, nextthink) = match (self.think_offset, self.nextthink_offset) {
            (Some(think), Some(nextthink)) => (think, nextthink),
            _ => return Err(GuestError::invalid("Mod think needs explicit think/nextthink field mappings")),
        };
        let slot = self
            .owned
            .get(actor.id())
            .copied()
            .ok_or_else(|| GuestError::invalid("Mod think requires its own source actor"))?;
        let function_index = self.machine.slot_int(slot, think)?;
        self.machine.set_slot_float(slot, nextthink, 0.0)?;
        if function_index != 0 {
            self.host.invoke(actor, function_index, frame);
        }
        Ok(())
    }

    /// Advance owned actors one frame.
    pub fn advance(&mut self, frame: &FrameContext) -> Result<(), GuestError> {
        let now = frame.time.as_seconds_f64() - frame.elapsed.as_seconds_f64();
        let source_frame = FrameContext {
            frame: frame.frame,
            time: SourceTime::Seconds(now as f32),
            elapsed: frame.elapsed,
            phase: frame.phase,
        };
        for slot in 1..self.slots.len() as u32 {
            if slot < self.first_dynamic_slot && self.host.client_frame(slot, &source_frame) {
                continue;
            }
            let actor = match self.at(slot) {
                Some(actor) => actor.clone(),
                None => continue,
            };
            if self.scheduler.take_due(actor.id(), now) {
                self.fire_think(&actor, &source_frame)?;
            }
            self.host.step(&actor, &source_frame);
        }
        Ok(())
    }

    /// Dispatch touch from an owned actor.
    pub fn touch(&mut self, actor: &ActorId, other: &ActorId) -> Result<(), GuestError> {
        let owned = self.require(actor)?;
        self.host.touch(&owned, other);
        Ok(())
    }

    /// Dispatch use from an owned actor.
    pub fn use_on(
        &mut self,
        actor: &ActorId,
        other: Option<&ActorId>,
        activator: Option<&ActorId>,
    ) -> Result<(), GuestError> {
        let owned = self.require(actor)?;
        self.host.use_on(&owned, other, activator);
        Ok(())
    }

    /// Dispatch a pain reaction.
    pub fn pain(&mut self, reaction: &QcPainReaction) -> Result<(), GuestError> {
        self.require(reaction.target.id())?;
        self.host.pain(reaction);
        Ok(())
    }

    /// Dispatch a death reaction.
    pub fn die(&mut self, reaction: &QcDeathReaction) -> Result<(), GuestError> {
        self.require(reaction.target.id())?;
        self.host.die(reaction);
        Ok(())
    }

    /// Re-admit actors after a save restore.
    pub fn restored(&mut self, actors: &[(OwnedActor, u32)]) -> Result<(), GuestError> {
        for (actor, slot) in actors {
            if actor.owner() != &self.provider || (*slot as usize) >= self.slots.len() {
                return Err(GuestError::invalid("Saved mod actor lost its source slot"));
            }
            self.slots[*slot as usize] = Some(actor.clone());
            self.owned.insert(actor.id().clone(), *slot);
            self.host.rebind_body(actor, *slot);
            let owned = actor.clone();
            self.admit(owned, *slot);
            if self.nextthink_offset.is_some() {
                self.schedule(actor.id())?;
            }
        }
        Ok(())
    }

    /// Require an owned actor.
    fn require(&self, actor: &ActorId) -> Result<OwnedActor, GuestError> {
        self.owned
            .get(actor)
            .and_then(|slot| self.at(*slot))
            .filter(|owned| owned.id() == actor)
            .cloned()
            .ok_or_else(|| GuestError::invalid("Mod callback requires its own source actor"))
    }

    /// Release all owned actors.
    pub fn close(&mut self) -> Result<(), GuestError> {
        if self.closed {
            return Ok(());
        }
        self.closed = true;
        let mut errors = Vec::new();
        let owned: Vec<ActorId> = self.owned.keys().cloned().collect();
        for actor in owned {
            if let Err(error) = self.remove(&actor) {
                errors.push(error.to_string());
            }
        }
        self.owned.clear();
        if errors.is_empty() {
            Ok(())
        } else {
            Err(GuestError::Callback(format!("Mod actor release failed: {}", errors.join("; "))))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;
    use qa_core::time::FramePhase;

    struct FakeMachine {
        slots: HashMap<(u32, i32), f32>,
        ints: HashMap<(u32, i32), i32>,
    }

    impl QcActorMachine for FakeMachine {
        fn slot_int(&self, slot: u32, offset: i32) -> Result<i32, GuestError> {
            Ok(self.ints.get(&(slot, offset)).copied().unwrap_or(0))
        }

        fn slot_float(&self, slot: u32, offset: i32) -> Result<f32, GuestError> {
            Ok(self.slots.get(&(slot, offset)).copied().unwrap_or(0.0))
        }

        fn set_slot_float(&mut self, slot: u32, offset: i32, value: f32) -> Result<(), GuestError> {
            self.slots.insert((slot, offset), value);
            Ok(())
        }
    }

    #[derive(Default)]
    struct FakeHost {
        events: Vec<String>,
        client_slots: Vec<u32>,
    }

    impl QcModActorHost for FakeHost {
        fn now(&self) -> SourceTime {
            SourceTime::Seconds(10.0)
        }

        fn bind_body(&mut self, actor: &OwnedActor, slot: u32) {
            self.events.push(format!("bind {}@{slot}", actor.id().slot()));
        }

        fn unlink_body(&mut self, actor: &OwnedActor) {
            self.events.push(format!("unlink {}", actor.id().slot()));
        }

        fn rebind_body(&mut self, actor: &OwnedActor, slot: u32) {
            self.events.push(format!("rebind {}@{slot}", actor.id().slot()));
        }

        fn admitted(&mut self, actor: &OwnedActor, slot: u32) {
            self.events.push(format!("admitted {}@{slot}", actor.id().slot()));
        }

        fn retired(&mut self, actor: &OwnedActor) {
            self.events.push(format!("retired {}", actor.id().slot()));
        }

        fn remember(&mut self, actor: &ActorId, slot: u32) {
            self.events.push(format!("remember {}@{slot}", actor.slot()));
        }

        fn step(&mut self, actor: &OwnedActor, _frame: &FrameContext) {
            self.events.push(format!("step {}", actor.id().slot()));
        }

        fn touch(&mut self, actor: &OwnedActor, other: &ActorId) {
            self.events.push(format!("touch {} {}", actor.id().slot(), other.slot()));
        }

        fn use_on(&mut self, actor: &OwnedActor, _other: Option<&ActorId>, _activator: Option<&ActorId>) {
            self.events.push(format!("use {}", actor.id().slot()));
        }

        fn pain(&mut self, reaction: &QcPainReaction) {
            self.events.push(format!("pain {}", reaction.target.id().slot()));
        }

        fn die(&mut self, reaction: &QcDeathReaction) {
            self.events.push(format!("die {}", reaction.target.id().slot()));
        }

        fn invoke(&mut self, actor: &OwnedActor, function_index: i32, _frame: &FrameContext) {
            self.events.push(format!("invoke {}#{function_index}", actor.id().slot()));
        }

        fn client_frame(&mut self, slot: u32, _frame: &FrameContext) -> bool {
            self.client_slots.push(slot);
            true
        }
    }

    fn fixture() -> (IdentityOwner, QcModActors<FakeMachine, FakeHost>) {
        let owner = IdentityOwner::create("actors").unwrap();
        let actors = QcModActors::new(
            FakeMachine { slots: HashMap::new(), ints: HashMap::new() },
            FakeHost::default(),
            ProviderId::new("mod", "test"),
            8,
            2,
            Some(10),
            Some(11),
        );
        (owner, actors)
    }

    fn frame() -> FrameContext {
        FrameContext {
            frame: 1,
            time: SourceTime::Seconds(10.0),
            elapsed: SourceTime::Seconds(0.5),
            phase: FramePhase::EntityThink,
        }
    }

    #[test]
    fn spawn_admits_and_remove_retires() {
        let (owner, mut actors) = fixture();
        let provider = ProviderId::new("mod", "test");
        let actor = owner.actor(3, 1);
        let owned = owner.owned_actor(&actor, provider).unwrap();
        let slot = actors.spawn(owned).unwrap();
        assert_eq!(slot, 2);
        assert_eq!(actors.slot_of(&actor), Some(2));
        actors.remove(&actor).unwrap();
        assert_eq!(actors.slot_of(&actor), None);
        assert!(actors.host().events.iter().any(|event| event == "admitted 3@2"));
        assert!(actors.host().events.iter().any(|event| event == "retired 3"));
    }

    #[test]
    fn remove_rejects_foreign_actors() {
        let (owner, mut actors) = fixture();
        let foreign = owner.actor(5, 1);
        assert!(actors.remove(&foreign).is_err());
        let wrong = owner.owned_actor(&owner.actor(6, 1), ProviderId::new("mod", "other")).unwrap();
        assert!(actors.spawn(wrong).is_err());
    }

    #[test]
    fn schedule_and_advance_fire_due_think() {
        let (owner, mut actors) = fixture();
        let provider = ProviderId::new("mod", "test");
        let actor = owner.actor(3, 1);
        let first = actors.spawn(owner.owned_actor(&actor, provider).unwrap()).unwrap();
        assert_eq!(first, 2);
        // Slot 1 is reserved for clients; client_frame consumes it.
        let actor2 = owner.actor(4, 1);
        let slot = actors.spawn(owner.owned_actor(&actor2, ProviderId::new("mod", "test")).unwrap()).unwrap();
        assert_eq!(slot, 3);
        actors.machine.slots.insert((slot, 11), 9.0);
        actors.machine.ints.insert((slot, 10), 42);
        actors.schedule(&actor2).unwrap();
        assert_eq!(actors.scheduler().due(&actor2), Some(9.0));
        actors.advance(&frame()).unwrap();
        assert_eq!(actors.host().client_slots, vec![1]);
        assert!(actors.host().events.iter().any(|event| event == "invoke 4#42"));
        assert!(actors.host().events.iter().any(|event| event == "step 4"));
        assert_eq!(actors.machine.slots.get(&(slot, 11)), Some(&0.0));
        assert_eq!(actors.scheduler().due(&actor2), None);
    }

    #[test]
    fn zero_nextthink_cancels_schedule() {
        let (owner, mut actors) = fixture();
        let actor = owner.actor(3, 1);
        actors.spawn(owner.owned_actor(&actor, ProviderId::new("mod", "test")).unwrap()).unwrap();
        actors.schedule(&actor).unwrap();
        assert_eq!(actors.scheduler().due(&actor), None);
    }

    #[test]
    fn callbacks_require_owned_actors() {
        let (owner, mut actors) = fixture();
        let actor = owner.actor(3, 1);
        let owned = owner.owned_actor(&actor, ProviderId::new("mod", "test")).unwrap();
        actors.spawn(owned.clone()).unwrap();
        let other = owner.actor(7, 1);
        actors.touch(&actor, &other).unwrap();
        actors.use_on(&actor, Some(&other), None).unwrap();
        actors.pain(&QcPainReaction { target: owned.clone(), attacker: None, damage: 5.0, kick: 0.0 }).unwrap();
        actors
            .die(&QcDeathReaction { target: owned, attacker: None, inflictor: None, damage: 50.0, kick: 0.0 })
            .unwrap();
        assert!(actors.touch(&other, &actor).is_err());
    }

    #[test]
    fn restored_rebinds_and_schedules() {
        let (owner, mut actors) = fixture();
        let actor = owner.actor(3, 1);
        let owned = owner.owned_actor(&actor, ProviderId::new("mod", "test")).unwrap();
        actors.machine.slots.insert((2, 11), 4.0);
        actors.restored(&[(owned, 2)]).unwrap();
        assert_eq!(actors.slot_of(&actor), Some(2));
        assert_eq!(actors.scheduler().due(&actor), Some(4.0));
        assert!(actors.host().events.iter().any(|event| event == "rebind 3@2"));
    }

    #[test]
    fn close_releases_all() {
        let (owner, mut actors) = fixture();
        let provider = ProviderId::new("mod", "test");
        for slot in [3, 4] {
            let actor = owner.actor(slot, 1);
            actors.spawn(owner.owned_actor(&actor, provider.clone()).unwrap()).unwrap();
        }
        actors.close().unwrap();
        assert!(actors.slot_of(&owner.actor(3, 1)).is_none());
        actors.close().unwrap();
    }
}
