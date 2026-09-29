//! Think dispatch ported from `src/world/scheduler.ts` (Q1/QW `sv_phys`,
//! Q2 `g_phys`, Q3 `g_main`): clear before calling. Deadlines select
//! eligibility; source traversal, rather than deadline sorting, selects
//! execution order. The table uses interior mutability so think callbacks
//! can reschedule reentrantly, exactly as in the donor.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use qa_core::identity::{ActorId, OwnedActor, ProviderId};
use qa_core::time::{ClockProfile, FrameContext, SourceTime};

use crate::clocks::{same_time_unit, validate_time};
use crate::registry::{provider_key, ActorRegistry};
use crate::WorldError;

/// Think execution boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThinkBoundary {
    /// Before physics.
    BeforePhysics,
    /// During physics.
    DuringPhysics,
    /// After physics.
    AfterPhysics,
}

/// Invocation order naming the owning actor and provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvocationOrder {
    /// Owning provider.
    pub provider: ProviderId,
    /// Owning actor.
    pub actor: ActorId,
    /// Invocation sequence.
    pub sequence: u64,
}

/// Think timing: due time, boundary, and order.
#[derive(Debug, Clone, PartialEq)]
pub struct ThinkTiming {
    /// Execution provider override.
    pub execution_provider: Option<ProviderId>,
    /// Due time.
    pub due: SourceTime,
    /// Execution boundary.
    pub boundary: ThinkBoundary,
    /// Invocation order.
    pub order: InvocationOrder,
}

/// Pending think.
#[derive(Debug, Clone)]
pub struct ScheduledThink {
    /// Actor.
    pub actor: ActorId,
    /// Callback identifier.
    pub callback: String,
    /// Timing.
    pub timing: ThinkTiming,
}

/// Frame ordering: native slot order or mixed provider groups.
#[derive(Debug, Clone, PartialEq)]
pub enum FrameOrdering {
    /// Single-clock native order.
    Native {
        /// Native clock.
        clock: ClockProfile,
    },
    /// Mixed provider order.
    Mixed {
        /// Providers in traversal order.
        providers: Vec<ProviderId>,
    },
}

/// Think callback. Receives the scheduler for reentrant scheduling.
pub type ThinkCallback = Rc<dyn Fn(&OwnedActor, FrameContext, &Scheduler)>;

/// Pending think identity key: (callback, sequence, provider, execution provider, actor).
type PendingThinkKey = (String, u32, ProviderId, ProviderId, ActorId);

/// Not-run reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotRunReason {
    /// Nothing scheduled.
    Unscheduled,
    /// Wrong boundary.
    Boundary,
    /// Not due yet.
    NotDue,
    /// Actor released.
    Stale,
}

/// Think run result. Preserves Q1 liveness and Q2 ran-think distinctions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThinkResult {
    /// Did not run.
    NotRun {
        /// Reason.
        reason: NotRunReason,
    },
    /// Ran with invocation count and liveness.
    Ran {
        /// Invocations.
        invocations: u32,
        /// Actor still live.
        alive: bool,
    },
}

/// Saved pending think.
#[derive(Debug, Clone, PartialEq)]
pub struct ThinkCheckpoint {
    /// Registry slot.
    pub slot: u32,
    /// Slot generation.
    pub generation: u32,
    /// Callback identifier.
    pub callback: String,
    /// Due time.
    pub due: SourceTime,
    /// Execution boundary.
    pub boundary: ThinkBoundary,
    /// Invocation sequence.
    pub sequence: u64,
    /// Execution provider override.
    pub execution_provider: Option<ProviderId>,
    /// Source slot.
    pub source_slot: u32,
}

/// Optional schedule overrides.
#[derive(Debug, Clone, Default)]
pub struct ScheduleOptions {
    /// Registered continuation provider; defaults to the actor owner.
    pub registered_execution: Option<ProviderId>,
    /// Source slot; defaults to the registry slot.
    pub source_slot: Option<u32>,
}

struct PendingThink {
    actor: ActorId,
    owned: OwnedActor,
    callback: String,
    source_slot: u32,
    timing: ThinkTiming,
}

/// Compare invocation order under a frame ordering.
pub fn compare_invocation_order(
    ordering: &FrameOrdering,
    left: &InvocationOrder,
    right: &InvocationOrder,
    source_slot: &dyn Fn(&ActorId) -> u32,
) -> Result<i32, WorldError> {
    if let FrameOrdering::Mixed { providers } = ordering {
        let left_index = providers
            .iter()
            .position(|provider| provider == &left.provider)
            .ok_or(WorldError::ProviderAbsent)?;
        let right_index = providers
            .iter()
            .position(|provider| provider == &right.provider)
            .ok_or(WorldError::ProviderAbsent)?;
        if left_index != right_index {
            return Ok(left_index as i32 - right_index as i32);
        }
    }
    let slots = i64::from(source_slot(&left.actor)) - i64::from(source_slot(&right.actor));
    if slots != 0 {
        return Ok(slots.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32);
    }
    Ok((left.sequence as i64 - right.sequence as i64).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32)
}

fn is_seconds(profile: &ClockProfile) -> bool {
    matches!(
        profile,
        ClockProfile::Q1Netquake { .. } | ClockProfile::Q1Quakeworld { .. } | ClockProfile::Q2Classic
    )
}

/// Null means not due; the returned time is visible inside the callback.
pub fn think_callback_time(
    profile: &ClockProfile,
    due: SourceTime,
    frame: FrameContext,
) -> Result<Option<SourceTime>, WorldError> {
    validate_time(due)?;
    validate_time(frame.time)?;
    validate_time(frame.elapsed)?;
    same_time_unit(due, frame.time)?;
    same_time_unit(frame.time, frame.elapsed)?;
    let negative_elapsed = match frame.elapsed {
        SourceTime::Seconds(value) => value < 0.0,
        SourceTime::Milliseconds(value) => value < 0,
    };
    if negative_elapsed {
        return Err(WorldError::NegativeTime);
    }
    let seconds = is_seconds(profile);
    let due_is_seconds = matches!(due, SourceTime::Seconds(_));
    if due_is_seconds != seconds {
        return Err(WorldError::ThinkUnit);
    }
    let nonpositive = match due {
        SourceTime::Seconds(value) => value <= 0.0,
        SourceTime::Milliseconds(value) => value <= 0,
    };
    if nonpositive {
        return Ok(None);
    }
    match profile {
        ClockProfile::Q1Netquake { .. } | ClockProfile::Q1Quakeworld { .. } => {
            let (SourceTime::Seconds(due), SourceTime::Seconds(now), SourceTime::Seconds(elapsed)) =
                (due, frame.time, frame.elapsed)
            else {
                return Err(WorldError::ThinkUnit);
            };
            if due > now + elapsed {
                return Ok(None);
            }
            Ok(Some(SourceTime::Seconds(due.max(now))))
        }
        ClockProfile::Q2Classic => {
            let (SourceTime::Seconds(due), SourceTime::Seconds(now)) = (due, frame.time) else {
                return Err(WorldError::ThinkUnit);
            };
            if due > now + 0.001 {
                return Ok(None);
            }
            Ok(Some(frame.time))
        }
        ClockProfile::Q2Rerelease { .. } => {
            let (SourceTime::Milliseconds(due), SourceTime::Milliseconds(now)) = (due, frame.time) else {
                return Err(WorldError::ThinkUnit);
            };
            if due > now {
                return Ok(None);
            }
            Ok(Some(frame.time))
        }
        ClockProfile::Q3 { .. } => {
            let (SourceTime::Milliseconds(due), SourceTime::Milliseconds(now)) = (due, frame.time) else {
                return Err(WorldError::ThinkUnit);
            };
            if (due as f32) > now as f32 {
                return Ok(None);
            }
            Ok(Some(frame.time))
        }
    }
}

/// Frame scheduler.
pub struct Scheduler {
    scheduled: RefCell<HashMap<u32, PendingThink>>,
    profiles: HashMap<String, ClockProfile>,
    ordering: FrameOrdering,
    advancing: Cell<bool>,
    closed: Cell<bool>,
}

impl Scheduler {
    /// Create a scheduler with per-provider clocks.
    pub fn new(ordering: FrameOrdering, clocks: Vec<(ProviderId, ClockProfile)>) -> Result<Self, WorldError> {
        let mut profiles = HashMap::new();
        for (provider, profile) in clocks {
            if profiles.insert(provider_key(&provider), profile).is_some() {
                return Err(WorldError::DuplicateClock(provider_key(&provider)));
            }
        }
        if let FrameOrdering::Mixed { providers } = &ordering {
            let mut seen = std::collections::HashSet::new();
            for provider in providers {
                if !seen.insert(provider_key(provider)) {
                    return Err(WorldError::DuplicateProvider(provider_key(provider)));
                }
                if !profiles.contains_key(&provider_key(provider)) {
                    return Err(WorldError::MissingClock(provider_key(provider)));
                }
            }
        }
        Ok(Self {
            scheduled: RefCell::new(HashMap::new()),
            profiles,
            ordering,
            advancing: Cell::new(false),
            closed: Cell::new(false),
        })
    }

    fn assert_open(&self) -> Result<(), WorldError> {
        if self.closed.get() {
            return Err(WorldError::SchedulerClosed);
        }
        Ok(())
    }

    fn profile(&self, provider: &ProviderId) -> Result<ClockProfile, WorldError> {
        if let Some(profile) = self.profiles.get(&provider_key(provider)) {
            return Ok(*profile);
        }
        if let FrameOrdering::Native { clock } = &self.ordering {
            return Ok(*clock);
        }
        Err(WorldError::MissingClock(provider_key(provider)))
    }

    /// Schedule a think, replacing any pending think for the slot.
    pub fn schedule(
        &self,
        registry: &ActorRegistry,
        actor: &OwnedActor,
        callback: &str,
        timing: ThinkTiming,
        options: ScheduleOptions,
    ) -> Result<(), WorldError> {
        self.assert_open()?;
        registry.assert_owned(actor)?;
        let execution = timing
            .execution_provider
            .clone()
            .unwrap_or_else(|| actor.owner().clone());
        let registered = options.registered_execution.unwrap_or_else(|| actor.owner().clone());
        if execution != registered {
            return Err(WorldError::ExecutionProvider);
        }
        if execution != *actor.owner() && !self.profiles.contains_key(&provider_key(&execution)) {
            return Err(WorldError::MissingClock(provider_key(&execution)));
        }
        self.profile(&execution)?;
        validate_time(timing.due)?;
        if timing.order.actor != *actor.id() || timing.order.provider != *actor.owner() {
            return Err(WorldError::ThinkOrder);
        }
        if let FrameOrdering::Mixed { providers } = &self.ordering {
            if !providers.contains(actor.owner()) {
                return Err(WorldError::ProviderAbsent);
            }
        }
        let source_slot = options.source_slot.unwrap_or_else(|| actor.id().slot());
        self.scheduled.borrow_mut().insert(
            actor.id().slot(),
            PendingThink {
                actor: actor.id().clone(),
                owned: actor.clone(),
                callback: callback.to_string(),
                source_slot,
                timing,
            },
        );
        Ok(())
    }

    /// Cancel a pending think.
    pub fn cancel(&self, registry: &ActorRegistry, actor: &OwnedActor) -> Result<(), WorldError> {
        self.assert_open()?;
        let pending = self
            .scheduled
            .borrow()
            .get(&actor.id().slot())
            .map(|pending| (pending.actor.clone(), pending.owned.owner().clone()));
        if let Some((id, owner)) = pending {
            if id == *actor.id() {
                if &owner != actor.owner() {
                    return Err(WorldError::ThinkOwner);
                }
                self.scheduled.borrow_mut().remove(&actor.id().slot());
            }
        }
        let _ = registry;
        Ok(())
    }

    /// Pending think for an actor, pruning stale entries.
    pub fn pending(&self, registry: &ActorRegistry, actor: &ActorId) -> Result<Option<ScheduledThink>, WorldError> {
        self.assert_open()?;
        let pending = self
            .scheduled
            .borrow()
            .get(&actor.slot())
            .map(|pending| (pending.actor.clone(), pending.callback.clone(), pending.timing.clone()));
        let Some((id, callback, timing)) = pending else {
            return Ok(None);
        };
        if id != *actor {
            return Ok(None);
        }
        if !registry.is_live(actor) {
            self.scheduled.borrow_mut().remove(&actor.slot());
            return Ok(None);
        }
        Ok(Some(ScheduledThink {
            actor: id,
            callback,
            timing,
        }))
    }

    /// Run the pending think at an actor's source-defined physics site.
    pub fn run(
        &self,
        registry: &ActorRegistry,
        resolver: &dyn Fn(&ProviderId, &str) -> Option<ThinkCallback>,
        actor: &ActorId,
        frame: FrameContext,
        boundary: ThinkBoundary,
    ) -> Result<ThinkResult, WorldError> {
        self.assert_open()?;
        let mut invocations: u32 = 0;
        loop {
            let pending = self.scheduled.borrow().get(&actor.slot()).map(|pending| {
                (
                    pending.actor.clone(),
                    pending.owned.clone(),
                    pending.callback.clone(),
                    pending.timing.clone(),
                )
            });
            let Some((id, owned, callback, timing)) = pending else {
                return Ok(Self::result(invocations, registry, actor, NotRunReason::Unscheduled));
            };
            if id != *actor {
                return Ok(Self::result(invocations, registry, actor, NotRunReason::Unscheduled));
            }
            if !registry.is_live(actor) {
                self.scheduled.borrow_mut().remove(&actor.slot());
                return Ok(Self::result(invocations, registry, actor, NotRunReason::Stale));
            }
            if timing.boundary != boundary {
                return Ok(Self::result(invocations, registry, actor, NotRunReason::Boundary));
            }
            let execution = timing
                .execution_provider
                .clone()
                .unwrap_or_else(|| owned.owner().clone());
            let profile = self.profile(&execution)?;
            let time = think_callback_time(&profile, timing.due, frame)?;
            if time.is_none() {
                return Ok(Self::result(invocations, registry, actor, NotRunReason::NotDue));
            }
            self.scheduled.borrow_mut().remove(&actor.slot());
            let callback_fn =
                resolver(&execution, &callback).ok_or_else(|| WorldError::UnknownCallback(callback.clone()))?;
            let mut invoked = frame;
            invoked.time = time.unwrap_or(frame.time);
            invoked.phase = qa_core::time::FramePhase::EntityThink;
            callback_fn(&owned, invoked, self);
            invocations += 1;
            if self.closed.get() || !registry.is_live(actor) || !matches!(profile, ClockProfile::Q1Quakeworld { .. }) {
                return Ok(ThinkResult::Ran {
                    invocations,
                    alive: registry.is_live(actor),
                });
            }
        }
    }

    /// Traverse due thinks in source order. Later-slot additions and
    /// cancellation are visible during the same traversal.
    pub fn advance(
        &self,
        registry: &ActorRegistry,
        resolver: &dyn Fn(&ProviderId, &str) -> Option<ThinkCallback>,
        frames: &[(ProviderId, FrameContext)],
        boundary: ThinkBoundary,
    ) -> Result<Vec<(ActorId, ThinkResult)>, WorldError> {
        self.assert_open()?;
        if self.advancing.get() {
            return Err(WorldError::SchedulerReentrant);
        }
        let mut contexts: HashMap<String, FrameContext> = HashMap::new();
        for (provider, frame) in frames {
            self.profile(provider)?;
            if contexts.insert(provider_key(provider), *frame).is_some() {
                return Err(WorldError::DuplicateProvider(provider_key(provider)));
            }
        }
        let mut results = Vec::new();
        let mut cursor: Option<(String, u32)> = None;
        self.advancing.set(true);
        let outcome = (|| -> Result<(), WorldError> {
            loop {
                if self.closed.get() {
                    break;
                }
                let next = self.next_pending(registry, cursor.as_ref())?;
                let Some((key, source_slot, provider, execution, actor)) = next else {
                    break;
                };
                cursor = Some((key, source_slot));
                let _ = provider;
                let frame = contexts
                    .get(&provider_key(&execution))
                    .copied()
                    .ok_or_else(|| WorldError::MissingFrame(provider_key(&execution)))?;
                let result = self.run(registry, resolver, &actor, frame, boundary)?;
                results.push((actor, result));
            }
            Ok(())
        })();
        self.advancing.set(false);
        outcome?;
        Ok(results)
    }

    fn next_pending(
        &self,
        registry: &ActorRegistry,
        cursor: Option<&(String, u32)>,
    ) -> Result<Option<PendingThinkKey>, WorldError> {
        let mut stale = Vec::new();
        let mut best: Option<(String, u32, u64, ProviderId, ProviderId, ActorId)> = None;
        {
            let scheduled = self.scheduled.borrow();
            for pending in scheduled.values() {
                if !registry.is_live(&pending.actor) {
                    stale.push(pending.actor.slot());
                    continue;
                }
                let provider_rank = match &self.ordering {
                    FrameOrdering::Native { .. } => 0,
                    FrameOrdering::Mixed { providers } => providers
                        .iter()
                        .position(|provider| provider == pending.owned.owner())
                        .ok_or(WorldError::ProviderAbsent)?,
                };
                let key = format!("{provider_rank:08}:{:08}", pending.source_slot);
                if let Some((cursor_key, _)) = cursor {
                    if key.as_str() <= cursor_key.as_str() {
                        continue;
                    }
                }
                let candidate = (
                    key,
                    pending.source_slot,
                    pending.timing.order.sequence,
                    pending.owned.owner().clone(),
                    pending
                        .timing
                        .execution_provider
                        .clone()
                        .unwrap_or_else(|| pending.owned.owner().clone()),
                    pending.actor.clone(),
                );
                let better = match &best {
                    None => true,
                    Some(current) => (candidate.0.as_str(), candidate.2) < (current.0.as_str(), current.2),
                };
                if better {
                    best = Some(candidate);
                }
            }
        }
        if !stale.is_empty() {
            let mut scheduled = self.scheduled.borrow_mut();
            for slot in stale {
                scheduled.remove(&slot);
            }
        }
        Ok(
            best.map(|(key, source_slot, _, provider, execution, actor)| {
                (key, source_slot, provider, execution, actor)
            }),
        )
    }

    fn result(invocations: u32, registry: &ActorRegistry, actor: &ActorId, reason: NotRunReason) -> ThinkResult {
        if invocations == 0 {
            ThinkResult::NotRun { reason }
        } else {
            ThinkResult::Ran {
                invocations,
                alive: registry.is_live(actor),
            }
        }
    }

    /// Checkpoint pending thinks in slot order.
    pub fn think_checkpoint(&self) -> Result<Vec<ThinkCheckpoint>, WorldError> {
        self.assert_open()?;
        let mut checkpoints: Vec<ThinkCheckpoint> = self
            .scheduled
            .borrow()
            .values()
            .map(|pending| ThinkCheckpoint {
                slot: pending.actor.slot(),
                generation: pending.actor.generation(),
                callback: pending.callback.clone(),
                due: pending.timing.due,
                boundary: pending.timing.boundary,
                sequence: pending.timing.order.sequence,
                execution_provider: pending.timing.execution_provider.clone(),
                source_slot: pending.source_slot,
            })
            .collect();
        checkpoints.sort_by_key(|checkpoint| checkpoint.slot);
        Ok(checkpoints)
    }

    /// Close the scheduler, dropping every pending think.
    pub fn close(&self) {
        self.closed.set(true);
        self.scheduled.borrow_mut().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;
    use qa_core::time::FramePhase;

    fn q1_profile() -> ClockProfile {
        ClockProfile::Q1Netquake {
            minimum_frame_seconds: 0.001,
            maximum_frame_seconds: 0.1,
            fixed_frame_seconds: None,
        }
    }

    fn frame(time: SourceTime, elapsed: SourceTime) -> FrameContext {
        FrameContext {
            frame: 1,
            time,
            elapsed,
            phase: FramePhase::FrameEntry,
        }
    }

    #[test]
    fn think_timing_follows_per_profile_rules() {
        let q1 = q1_profile();
        let current = frame(SourceTime::Seconds(10.0), SourceTime::Seconds(0.1));
        assert_eq!(
            think_callback_time(&q1, SourceTime::Seconds(10.05), current).unwrap(),
            Some(SourceTime::Seconds(10.05))
        );
        assert_eq!(
            think_callback_time(&q1, SourceTime::Seconds(9.0), current).unwrap(),
            Some(SourceTime::Seconds(10.0))
        );
        assert_eq!(
            think_callback_time(&q1, SourceTime::Seconds(11.0), current).unwrap(),
            None
        );
        assert_eq!(
            think_callback_time(&q1, SourceTime::Seconds(0.0), current).unwrap(),
            None
        );
        let classic = ClockProfile::Q2Classic;
        let q2 = frame(SourceTime::Seconds(10.0), SourceTime::Seconds(0.1));
        assert!(think_callback_time(&classic, SourceTime::Seconds(10.0005), q2)
            .unwrap()
            .is_some());
        assert!(think_callback_time(&classic, SourceTime::Seconds(10.01), q2)
            .unwrap()
            .is_none());
        let q3 = ClockProfile::Q3 {
            server_frame_milliseconds: 50.0,
            fixed_movement_milliseconds: None,
        };
        let q3frame = frame(SourceTime::Milliseconds(1000), SourceTime::Milliseconds(50));
        assert!(think_callback_time(&q3, SourceTime::Milliseconds(1000), q3frame)
            .unwrap()
            .is_some());
        assert!(think_callback_time(&q3, SourceTime::Milliseconds(1001), q3frame)
            .unwrap()
            .is_none());
    }

    #[test]
    fn run_clears_before_calling_and_reports_boundaries() {
        let owner = IdentityOwner::create("test").unwrap();
        let mut registry = ActorRegistry::new(owner, 8).unwrap();
        let provider = ProviderId::new("q1", "game");
        let actor = registry.allocate(provider.clone(), "q1:ogre").unwrap();
        let scheduler = Scheduler::new(
            FrameOrdering::Native { clock: q1_profile() },
            vec![(provider.clone(), q1_profile())],
        )
        .unwrap();
        let timing = ThinkTiming {
            execution_provider: None,
            due: SourceTime::Seconds(10.0),
            boundary: ThinkBoundary::DuringPhysics,
            order: InvocationOrder {
                provider: provider.clone(),
                actor: actor.id().clone(),
                sequence: 0,
            },
        };
        scheduler
            .schedule(&registry, &actor, "q1:think", timing, ScheduleOptions::default())
            .unwrap();
        let current = frame(SourceTime::Seconds(10.0), SourceTime::Seconds(0.1));
        let ran = scheduler
            .run(
                &registry,
                &|_, _| Some(Rc::new(|_, _, _| {})),
                actor.id(),
                current,
                ThinkBoundary::BeforePhysics,
            )
            .unwrap();
        assert_eq!(
            ran,
            ThinkResult::NotRun {
                reason: NotRunReason::Boundary
            }
        );
        let ran = scheduler
            .run(
                &registry,
                &|_, _| Some(Rc::new(|_, _, _| {})),
                actor.id(),
                current,
                ThinkBoundary::DuringPhysics,
            )
            .unwrap();
        assert_eq!(
            ran,
            ThinkResult::Ran {
                invocations: 1,
                alive: true
            }
        );
        assert!(scheduler.pending(&registry, actor.id()).unwrap().is_none());
    }

    #[test]
    fn quakeworld_reruns_thinks_scheduled_reentrantly() {
        let owner = IdentityOwner::create("test").unwrap();
        let mut registry = ActorRegistry::new(owner, 8).unwrap();
        let provider = ProviderId::new("qw", "game");
        let actor = registry.allocate(provider.clone(), "qw:player").unwrap();
        let profile = ClockProfile::Q1Quakeworld {
            maximum_command_milliseconds: 50.0,
        };
        let scheduler = Scheduler::new(
            FrameOrdering::Native { clock: profile },
            vec![(provider.clone(), profile)],
        )
        .unwrap();
        let schedule = |scheduler: &Scheduler, sequence: u64| {
            scheduler
                .schedule(
                    &registry,
                    &actor,
                    "qw:think",
                    ThinkTiming {
                        execution_provider: None,
                        due: SourceTime::Seconds(10.0),
                        boundary: ThinkBoundary::DuringPhysics,
                        order: InvocationOrder {
                            provider: provider.clone(),
                            actor: actor.id().clone(),
                            sequence,
                        },
                    },
                    ScheduleOptions::default(),
                )
                .unwrap();
        };
        schedule(&scheduler, 0);
        // Think callbacks are 'static; promote the registry to a leaked
        // shared reference (test-only) so the reentrant schedule call compiles.
        let registry: &'static ActorRegistry = Box::leak(Box::new(registry));
        let current = frame(SourceTime::Seconds(10.0), SourceTime::Seconds(0.05));
        let count = Rc::new(Cell::new(0));
        let again = count.clone();
        let resolver = move |_: &ProviderId, _: &str| {
            let again = again.clone();
            Some(
                Rc::new(move |owned: &OwnedActor, _frame: FrameContext, scheduler: &Scheduler| {
                    let seen = again.get();
                    again.set(seen + 1);
                    if seen == 0 {
                        scheduler
                            .schedule(
                                registry,
                                owned,
                                "qw:think",
                                ThinkTiming {
                                    execution_provider: None,
                                    due: SourceTime::Seconds(10.0),
                                    boundary: ThinkBoundary::DuringPhysics,
                                    order: InvocationOrder {
                                        provider: owned.owner().clone(),
                                        actor: owned.id().clone(),
                                        sequence: 1,
                                    },
                                },
                                ScheduleOptions::default(),
                            )
                            .unwrap();
                    }
                }) as ThinkCallback,
            )
        };
        let ran = scheduler
            .run(registry, &resolver, actor.id(), current, ThinkBoundary::DuringPhysics)
            .unwrap();
        assert_eq!(
            ran,
            ThinkResult::Ran {
                invocations: 2,
                alive: true
            }
        );
        assert_eq!(count.get(), 2);
    }

    #[test]
    fn advance_runs_slots_in_source_order() {
        let owner = IdentityOwner::create("test").unwrap();
        let mut registry = ActorRegistry::new(owner, 8).unwrap();
        let provider = ProviderId::new("q3", "game");
        let profile = ClockProfile::Q3 {
            server_frame_milliseconds: 50.0,
            fixed_movement_milliseconds: None,
        };
        let scheduler = Scheduler::new(
            FrameOrdering::Native { clock: profile },
            vec![(provider.clone(), profile)],
        )
        .unwrap();
        for _ in 0..3 {
            let actor = registry.allocate(provider.clone(), "q3:item").unwrap();
            scheduler
                .schedule(
                    &registry,
                    &actor,
                    "q3:think",
                    ThinkTiming {
                        execution_provider: None,
                        due: SourceTime::Milliseconds(1000),
                        boundary: ThinkBoundary::BeforePhysics,
                        order: InvocationOrder {
                            provider: provider.clone(),
                            actor: actor.id().clone(),
                            sequence: 0,
                        },
                    },
                    ScheduleOptions::default(),
                )
                .unwrap();
        }
        let current = frame(SourceTime::Milliseconds(1000), SourceTime::Milliseconds(50));
        let results = scheduler
            .advance(
                &registry,
                &|_, _| Some(Rc::new(|_, _, _| {})),
                &[(provider, current)],
                ThinkBoundary::BeforePhysics,
            )
            .unwrap();
        let slots: Vec<u32> = results.iter().map(|(actor, _)| actor.slot()).collect();
        assert_eq!(slots, vec![0, 1, 2]);
    }
}
